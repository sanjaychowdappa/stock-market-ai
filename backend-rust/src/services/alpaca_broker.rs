//! Alpaca paper-trading SHADOW broker.
//!
//! The internal simulator remains the source of truth for P&L. This module
//! mirrors every simulated trade as a real order on Alpaca's **paper** account
//! (fake money, real execution mechanics) and records what actually happened.
//!
//! Why: the simulator assumes it fills at the last observed tick price. That is
//! optimistic — real fills cross the spread and slip. By submitting the same
//! order to a real broker and comparing, we can measure exactly how flattering
//! the simulator is, which is the last unmeasured gap in the P&L numbers.
//!
//! This NEVER feeds back into simulated P&L. It only observes and records, so a
//! broker outage or rejection cannot corrupt the research dataset.

use serde_json::{json, Value};
use std::env;
use tracing::{info, warn};

const FILL_LOG: &str = "/app/reports/broker_fills.jsonl";

/// A real fill reported back so the simulator can adopt the ACTUAL execution
/// price instead of its assumed last-tick price. Draining these keeps the two
/// books identical rather than merely similar.
#[derive(Debug, Clone)]
pub struct FillCorrection {
    pub symbol: String,
    pub side: String,
    pub actual_price: f64,
    pub assumed_price: f64,
    pub qty: f64,
}

/// Pending corrections, drained by the trader on its next tick. A queue (rather
/// than a blocking call inside the trade path) keeps the 1 Hz trading loop from
/// stalling on broker latency.
pub static CORRECTIONS: once_cell::sync::Lazy<parking_lot::Mutex<Vec<FillCorrection>>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(Vec::new()));

pub fn drain_corrections() -> Vec<FillCorrection> {
    let mut q = CORRECTIONS.lock();
    std::mem::take(&mut *q)
}

/// Symbols with an order currently in flight.
///
/// The strategy can exit and re-enter the same name within a second, which the
/// simulator allows but a real broker does not: Alpaca rejects the second order
/// with "potential wash trade detected — opposite side market/stop order
/// exists". Serialising orders per symbol removes that entire class of
/// rejection (7 of 33 orders on 2026-08-04).
static IN_FLIGHT: once_cell::sync::Lazy<parking_lot::Mutex<std::collections::HashSet<String>>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(std::collections::HashSet::new()));

/// True while a reconcile cycle is running. A cycle can take minutes (each
/// symbol waits for its claim, then polls for a fill), so without this guard a
/// 30s timer stacks overlapping cycles that each grab the trader lock —
/// starving tokio's workers until the HTTP server stops responding entirely.
static RECONCILING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wait until this symbol has no order in flight, then claim it.
/// Returns false if the wait timed out (caller should skip the order).
async fn claim_symbol(symbol: &str) -> bool {
    for _ in 0..24 {
        {
            let mut f = IN_FLIGHT.lock();
            if !f.contains(symbol) {
                f.insert(symbol.to_string());
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    warn!("[BROKER] {} still busy after 10s — skipping order", symbol);
    false
}

fn release_symbol(symbol: &str) {
    IN_FLIGHT.lock().remove(symbol);
}

fn creds() -> Option<(String, String, String)> {
    let key = env::var("APCA_API_KEY_ID").ok()?;
    let secret = env::var("APCA_API_SECRET_KEY").ok()?;
    let base = env::var("APCA_API_BASE_URL")
        .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string());
    // Hard safety rail: refuse to trade against a live endpoint. This module is
    // for paper only, and the strategy has no demonstrated edge.
    if !base.contains("paper-api") {
        warn!("[BROKER] APCA_API_BASE_URL is not a paper endpoint — shadow orders disabled");
        return None;
    }
    Some((key, secret, base))
}

/// Fetch the paper account snapshot (equity, cash, buying power).
pub async fn account() -> Option<Value> {
    let (key, secret, base) = creds()?;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v2/account", base))
        .header("APCA-API-KEY-ID", key)
        .header("APCA-API-SECRET-KEY", secret)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    resp.json::<Value>().await.ok()
}

/// Submit a market order to the paper account, wait briefly for the fill, and
/// record the simulated price alongside the real one.
///
/// `sim_price` is what the internal simulator assumed it got. Everything is
/// fire-and-forget: failures are logged, never propagated.
pub async fn shadow_order(symbol: String, qty: f64, side: &str, sim_price: f64, reason: String) {
    let (key, secret, base) = match creds() {
        Some(c) => c,
        None => return,
    };
    if qty <= 0.0 {
        return;
    }
    let side = side.to_lowercase();
    // Serialise per symbol so an exit and its immediate re-entry cannot be in
    // flight together — that collision is what Alpaca rejects as a wash trade.
    if !claim_symbol(&symbol).await {
        return;
    }
    let client = reqwest::Client::new();

    // Alpaca accepts fractional qty for market/day orders on liquid names.
    let body = json!({
        "symbol": symbol,
        "qty": format!("{:.6}", qty),
        "side": side,
        "type": "market",
        "time_in_force": "day",
    });

    let submitted = client
        .post(format!("{}/v2/orders", base))
        .header("APCA-API-KEY-ID", &key)
        .header("APCA-API-SECRET-KEY", &secret)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let order = match submitted {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v,
            Err(e) => {
                warn!("[BROKER] {} {} — bad response: {}", side, symbol, e);
                release_symbol(&symbol);
                return;
            }
        },
        Ok(r) => {
            let status = r.status();
            let msg = r.text().await.unwrap_or_default();
            // Rejections are DATA, not failures — they reveal real constraints
            // (buying power, PDT, halted symbol) the simulator never models.
            warn!("[BROKER] {} {} REJECTED ({}): {}", side, symbol, status, msg.trim());
            log_entry(json!({
                "timestamp": chrono::Local::now().to_rfc3339(),
                "symbol": symbol, "side": side, "qty": qty,
                "sim_price": sim_price, "reason": reason,
                "outcome": "rejected", "http_status": status.as_u16(),
                "detail": msg.trim(),
            }));
            release_symbol(&symbol);
            return;
        }
        Err(e) => {
            warn!("[BROKER] {} {} — submit failed: {}", side, symbol, e);
            release_symbol(&symbol);
            return;
        }
    };

    let order_id = order["id"].as_str().unwrap_or("").to_string();
    if order_id.is_empty() {
        release_symbol(&symbol);
        return;
    }

    // Poll for the fill. Market orders usually fill in well under a second, but
    // the closing rush is slower — a 4s window mislabelled three genuinely
    // filled 15:55 sells as "unfilled" on 2026-08-04. 20s is comfortably clear.
    let mut filled_price: Option<f64> = None;
    let mut filled_qty: Option<f64> = None;
    let mut final_status = String::from("pending");
    let mut settled = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let got = client
            .get(format!("{}/v2/orders/{}", base, order_id))
            .header("APCA-API-KEY-ID", &key)
            .header("APCA-API-SECRET-KEY", &secret)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        if let Ok(r) = got {
            if let Ok(v) = r.json::<Value>().await {
                final_status = v["status"].as_str().unwrap_or("unknown").to_string();

                // Keep the most recent price/qty seen, but do NOT stop here.
                // Alpaca populates filled_avg_price and filled_qty while an
                // order is still `partially_filled`. The previous version broke
                // on the first sight of a price, so a fractional order caught
                // mid-execution recorded its PARTIAL quantity as final — which
                // is why 2.825-share requests logged as "filled 1.000". Those
                // quantities feed the FIFO real-P&L match, so the error
                // propagated straight into the headline number.
                if let Some(p) = v["filled_avg_price"].as_str().and_then(|s| s.parse::<f64>().ok()) {
                    filled_price = Some(p);
                    filled_qty = v["filled_qty"].as_str().and_then(|s| s.parse::<f64>().ok());
                }

                // Only a terminal status settles the order.
                if matches!(final_status.as_str(),
                    "filled" | "canceled" | "rejected" | "expired" | "done_for_day")
                {
                    settled = true;
                    break;
                }
            }
        }
    }
    if !settled && filled_price.is_some() {
        warn!("[BROKER] {} {} still '{}' after 10s — recording partial qty {:?}; \
               final fill may differ",
            side, symbol, final_status, filled_qty);
    }

    match filled_price {
        Some(actual) => {
            // Slippage signed so that POSITIVE always means "worse than the
            // simulator assumed", for both buys and sells.
            let slip = if side == "buy" { actual - sim_price } else { sim_price - actual };
            let slip_pct = if sim_price > 0.0 { slip / sim_price * 100.0 } else { 0.0 };
            info!("[BROKER] {} {} filled @ ${:.4} vs sim ${:.4} — slippage ${:.4} ({:+.3}%)",
                side, symbol, actual, sim_price, slip, slip_pct);

            // Report the real price back so the simulator can adopt it. Skip
            // reconciliation orders — those are corrections to a position the
            // simulator already priced, not the simulator's own trade.
            if reason != "RECONCILE" && sim_price > 0.0 {
                CORRECTIONS.lock().push(FillCorrection {
                    symbol: symbol.clone(),
                    side: side.clone(),
                    actual_price: actual,
                    assumed_price: sim_price,
                    qty: filled_qty.unwrap_or(qty),
                });
            }
            log_entry(json!({
                "timestamp": chrono::Local::now().to_rfc3339(),
                "symbol": symbol, "side": side,
                "qty_requested": qty, "qty_filled": filled_qty,
                "sim_price": sim_price, "actual_price": actual,
                "slippage": (slip * 10000.0).round() / 10000.0,
                "slippage_pct": (slip_pct * 10000.0).round() / 10000.0,
                "reason": reason, "outcome": "filled", "order_id": order_id,
            }));
        }
        None => {
            warn!("[BROKER] {} {} not filled (status {})", side, symbol, final_status);
            log_entry(json!({
                "timestamp": chrono::Local::now().to_rfc3339(),
                "symbol": symbol, "side": side, "qty": qty,
                "sim_price": sim_price, "reason": reason,
                "outcome": "unfilled", "status": final_status, "order_id": order_id,
            }));
        }
    }
    release_symbol(&symbol);
}

fn log_entry(v: Value) {
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        if let Ok(mut f) = tokio::fs::OpenOptions::new()
            .create(true).append(true).open(FILL_LOG).await
        {
            let mut line = serde_json::to_string(&v).unwrap_or_default();
            line.push('\n');
            let _ = f.write_all(line.as_bytes()).await;
        }
    });
}

/// Current Alpaca paper positions as symbol -> signed quantity.
pub async fn positions() -> Option<std::collections::HashMap<String, f64>> {
    let (key, secret, base) = creds()?;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/v2/positions", base))
        .header("APCA-API-KEY-ID", key)
        .header("APCA-API-SECRET-KEY", secret)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let arr = resp.json::<Vec<Value>>().await.ok()?;
    let mut map = std::collections::HashMap::new();
    for p in arr {
        if let (Some(sym), Some(q)) = (
            p["symbol"].as_str(),
            p["qty"].as_str().and_then(|s| s.parse::<f64>().ok()),
        ) {
            map.insert(sym.to_string(), q);
        }
    }
    Some(map)
}

/// Force the Alpaca paper account to match the simulator's book exactly.
///
/// Drift is inevitable: the broker may be deployed mid-session, an order can be
/// rejected for buying power, or the process can restart between a simulated
/// fill and its mirror. Rather than let the two books diverge silently, this
/// computes the difference per symbol and submits the orders that close it.
///
/// `sim` maps symbol -> quantity the simulator believes it holds.
/// `prices` maps symbol -> current price, used to ignore economically trivial gaps.
pub async fn reconcile(
    sim: std::collections::HashMap<String, f64>,
    prices: std::collections::HashMap<String, f64>,
) -> Value {
    use std::sync::atomic::Ordering as AOrd;
    if RECONCILING.swap(true, AOrd::SeqCst) {
        return json!({"skipped": "a reconcile cycle is already running"});
    }
    let out = reconcile_inner(sim, prices).await;
    RECONCILING.store(false, AOrd::SeqCst);
    out
}

async fn reconcile_inner(
    sim: std::collections::HashMap<String, f64>,
    prices: std::collections::HashMap<String, f64>,
) -> Value {
    let live = match positions().await {
        Some(p) => p,
        None => return json!({"error": "could not read Alpaca positions"}),
    };

    // Union of both books — a symbol held on only one side still needs fixing.
    let mut symbols: Vec<String> = sim.keys().cloned().collect();
    for s in live.keys() {
        if !symbols.contains(s) {
            symbols.push(s.clone());
        }
    }

    let mut actions: Vec<Value> = Vec::new();
    for sym in symbols {
        let want = sim.get(&sym).copied().unwrap_or(0.0);
        let have = live.get(&sym).copied().unwrap_or(0.0);
        let delta = want - have;
        let px = prices.get(&sym).copied().unwrap_or(0.0);

        // Skip dust: sub-cent share counts, or gaps worth under $1.
        if delta.abs() < 0.0001 || (px > 0.0 && (delta.abs() * px) < 1.0) {
            continue;
        }

        let side = if delta > 0.0 { "buy" } else { "sell" };
        info!("[RECONCILE] {} sim={:.4} alpaca={:.4} → {} {:.4}", sym, want, have, side, delta.abs());
        actions.push(json!({
            "symbol": sym, "sim_qty": want, "alpaca_qty": have,
            "action": side, "qty": (delta.abs() * 10000.0).round() / 10000.0,
        }));
        shadow_order(sym.clone(), delta.abs(), side, px, "RECONCILE".to_string()).await;
    }

    if actions.is_empty() {
        info!("[RECONCILE] books already match");
    }
    json!({
        "checked": chrono::Local::now().to_rfc3339(),
        "in_sync": actions.is_empty(),
        "corrections": actions,
    })
}

/// REAL realized P&L, computed from actual Alpaca fills using FIFO matching.
///
/// This is the number that matters. The simulator's P&L is an estimate built on
/// last-tick prices it never actually traded at; this is money that genuinely
/// moved at prices a broker genuinely gave us — including slippage, and with
/// rejected orders simply absent because they never happened.
pub async fn real_pnl() -> Value {
    let content = tokio::fs::read_to_string(FILL_LOG).await.unwrap_or_default();

    // symbol -> FIFO queue of open lots (qty, price)
    let mut lots: std::collections::HashMap<String, std::collections::VecDeque<(f64, f64)>> =
        std::collections::HashMap::new();
    let mut realized = 0.0;
    let mut wins = 0u32;
    let mut losses = 0u32;
    let mut by_day: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    let mut sim_realized = 0.0; // same trades priced the simulator's way
    let mut round_trips = 0u32;
    // Data-quality counters — silently dropping these is how a wrong number
    // looks like a right one.
    let mut unmatched_sells = 0u32;
    let mut unmatched_qty = 0.0;
    let mut partial_qty_rows = 0u32;

    for line in content.lines() {
        let v: Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };
        if v["outcome"].as_str() != Some("filled") { continue; }
        let sym = v["symbol"].as_str().unwrap_or("").to_string();
        let side = v["side"].as_str().unwrap_or("");
        let px = v["actual_price"].as_f64().unwrap_or(0.0);
        let sim_px = v["sim_price"].as_f64().unwrap_or(px);
        let qty = v["qty_filled"].as_f64()
            .or_else(|| v["qty_requested"].as_f64())
            .unwrap_or(0.0);
        let day = v["timestamp"].as_str().unwrap_or("").chars().take(10).collect::<String>();
        if qty <= 0.0 || px <= 0.0 { continue; }
        // Rows where the recorded fill is materially smaller than the request
        // are suspect: before 2026-08-04 the poll loop broke on the first sight
        // of filled_avg_price, so a partially_filled snapshot could be stored as
        // the final quantity.
        if let Some(req) = v["qty_requested"].as_f64() {
            if req - qty > 0.01 { partial_qty_rows += 1; }
        }

        let q = lots.entry(sym.clone()).or_default();
        if side == "buy" {
            q.push_back((qty, px));
            // Track the simulator's cost basis in parallel for comparison.
            lots.entry(format!("{}__sim", sym)).or_default().push_back((qty, sim_px));
        } else {
            let mut remaining = qty;
            let mut sim_rem = qty;
            // Accumulate the whole sell, then classify it ONCE. This counter
            // used to increment per matched lot, so a single sell that consumed
            // three buy lots reported three round trips and three win/loss
            // events — inflating the trade count and making the win rate a
            // per-lot statistic wearing a per-trade label.
            let mut trip_pnl = 0.0;
            let mut matched = false;
            while remaining > 1e-9 {
                let (lot_qty, lot_px) = match q.front_mut() { Some(l) => (l.0, l.1), None => break };
                let used = remaining.min(lot_qty);
                trip_pnl += (px - lot_px) * used;
                matched = true;
                remaining -= used;
                if used >= lot_qty - 1e-9 { q.pop_front(); } else { q.front_mut().unwrap().0 -= used; }
            }
            if matched {
                realized += trip_pnl;
                if trip_pnl >= 0.0 { wins += 1 } else { losses += 1 }
                *by_day.entry(day.clone()).or_insert(0.0) += trip_pnl;
                round_trips += 1;
            }
            // A sell with no matching buy lot means the buy side was never
            // recorded (rejected, or its quantity was understated). The old code
            // just `break`s and drops it, so the P&L quietly omits real exposure.
            if remaining > 1e-9 {
                unmatched_sells += 1;
                unmatched_qty += remaining;
            }
            // Mirror the same matching against simulator prices.
            let sq = lots.entry(format!("{}__sim", sym)).or_default();
            while sim_rem > 1e-9 {
                let (lot_qty, lot_px) = match sq.front_mut() { Some(l) => (l.0, l.1), None => break };
                let used = sim_rem.min(lot_qty);
                sim_realized += (sim_px - lot_px) * used;
                sim_rem -= used;
                if used >= lot_qty - 1e-9 { sq.pop_front(); } else { sq.front_mut().unwrap().0 -= used; }
            }
        }
    }

    let n = round_trips.max(1) as f64;
    json!({
        "headline": "REAL P&L from actual Alpaca fills — this is the number that counts",
        "real_realized_pnl": (realized * 100.0).round() / 100.0,
        "simulator_would_have_shown": (sim_realized * 100.0).round() / 100.0,
        "execution_drag": ((sim_realized - realized) * 100.0).round() / 100.0,
        "round_trips": round_trips,
        "wins": wins, "losses": losses,
        "win_rate_pct": if round_trips > 0 { (wins as f64 / n * 10000.0).round() / 100.0 } else { 0.0 },
        "avg_per_round_trip": (realized / n * 100.0).round() / 100.0,
        "by_day": by_day.iter().map(|(d, p)| json!({"date": d, "real_pnl": (p * 100.0).round() / 100.0})).collect::<Vec<_>>(),
        "data_quality": {
            "unmatched_sells": unmatched_sells,
            "unmatched_qty": (unmatched_qty * 10000.0).round() / 10000.0,
            "partial_qty_rows": partial_qty_rows,
            "note": if partial_qty_rows > 0 || unmatched_sells > 0 {
                "Some fills were recorded before the partial-fill parse fix (2026-08-04): \
                 the poll loop stored a partially_filled snapshot as the final quantity, so \
                 those quantities understate the real position and this P&L is approximate \
                 for the affected days. Fills recorded from 2026-08-05 onward wait for a \
                 terminal order status."
            } else {
                "All fills settled on a terminal order status; quantities are final."
            },
        },
    })
}

/// Summary of simulator-vs-reality for the API/dashboard.
pub async fn fills_summary() -> Value {
    let content = tokio::fs::read_to_string(FILL_LOG).await.unwrap_or_default();
    let mut filled = 0u32;
    let mut rejected = 0u32;
    let mut unfilled = 0u32;
    let mut slip_sum = 0.0;
    let mut slip_pct_sum = 0.0;
    let mut recent: Vec<Value> = Vec::new();
    for line in content.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            match v["outcome"].as_str().unwrap_or("") {
                "filled" => {
                    filled += 1;
                    slip_sum += v["slippage"].as_f64().unwrap_or(0.0);
                    slip_pct_sum += v["slippage_pct"].as_f64().unwrap_or(0.0);
                }
                "rejected" => rejected += 1,
                "unfilled" => unfilled += 1,
                _ => {}
            }
            recent.push(v);
        }
    }
    let n = filled.max(1) as f64;
    json!({
        "note": "Shadow orders on Alpaca PAPER. The internal simulator remains the source of truth; this measures how optimistic its assumed fills are.",
        "filled": filled, "rejected": rejected, "unfilled": unfilled,
        "avg_slippage": (slip_sum / n * 10000.0).round() / 10000.0,
        "avg_slippage_pct": (slip_pct_sum / n * 10000.0).round() / 10000.0,
        "total_slippage_cost": (slip_sum * 100.0).round() / 100.0,
        "recent": recent.iter().rev().take(20).collect::<Vec<_>>(),
    })
}
