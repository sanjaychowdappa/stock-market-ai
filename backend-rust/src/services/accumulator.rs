//! Long-term accumulation: a fixed dollar amount into a broad index, daily,
//! held and never sold.
//!
//! This is the only strategy in the project that tested profitable. Backtested
//! over 1,160 trading days (2022-01-03 -> 2026-08-18), $15,080 contributed
//! became $22,938 — +52.1%.
//!
//! DELIBERATELY HAS NO STOCK PICKER
//! A 6/12-month momentum screen with a 200-day SMA filter and one name per
//! sector was tested head to head against a plain index and lost from all four
//! start dates, by $784 to $2,848. Every selection layer this project has
//! measured — intraday signals, pyramiding into winners, the momentum screen —
//! has subtracted value. So the money goes into the index and the screen runs
//! in shadow, logged but not funded, until it earns its place out of sample.
//!
//! SEPARATION FROM THE INTRADAY BOOK
//! This is a different animal from the $3,000 day book and must not touch it:
//!
//!   * Its own order path, so accumulator buys never enter broker_fills.jsonl
//!     and cannot be counted as intraday entries by fifo_stats or the change
//!     monitor.
//!   * Its own log, reports/accumulator.jsonl, which is the record of what was
//!     contributed and when.
//!   * reconcile() must skip these symbols entirely. That loop unions the
//!     simulator's book with LIVE Alpaca positions, so a holding the simulator
//!     knows nothing about reads as want=0/have=N — a sell. Unguarded, it would
//!     liquidate the whole accumulation every cycle. `owns()` exists for that
//!     guard and nothing else.
//!   * It NEVER sells. There is no exit path in this module, by construction.

use serde_json::{json, Value};
use tracing::{info, warn};

const LOG: &str = "/app/reports/accumulator.jsonl";

/// Is this symbol owned by the accumulator, and therefore off-limits to any
/// process that reconciles or flattens?
///
/// Called by alpaca_broker::reconcile. Keep it cheap and total: a wrong answer
/// here sells a long-term holding.
pub fn owns(symbol: &str) -> bool {
    crate::config::ACCUMULATOR_ENABLED && symbol == crate::config::ACCUMULATOR_SYMBOL
}

fn rows() -> Vec<Value> {
    std::fs::read_to_string(LOG)
        .map(|c| c.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// Has today's contribution already been made?
///
/// Idempotency is the whole safety story here. The loop wakes repeatedly and
/// the process restarts often; without this a single day could be funded many
/// times over.
fn already_contributed(date: &str) -> bool {
    rows().iter().any(|r| {
        r["date"].as_str() == Some(date) && r["outcome"].as_str() == Some("filled")
    })
}

fn append(row: Value) {
    let mut line = serde_json::to_string(&row).unwrap_or_default();
    line.push('\n');
    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(LOG) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()) {
                warn!("[ACCUM] write failed: {}", e);
            }
        }
        Err(e) => warn!("[ACCUM] open failed: {}", e),
    }
}

/// Buy a fixed dollar amount using Alpaca's notional order support, so the
/// contribution is exactly $N regardless of share price.
async fn buy_notional(symbol: &str, usd: f64) -> Option<Value> {
    let key = std::env::var("APCA_API_KEY_ID").ok()?;
    let secret = std::env::var("APCA_API_SECRET_KEY").ok()?;
    let base = std::env::var("APCA_API_BASE_URL")
        .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string());
    // Same hard rail as the intraday broker: paper endpoints only. This module
    // buys and never sells, so a misconfigured endpoint would accumulate a real
    // position with real money.
    if !base.contains("paper-api") {
        warn!("[ACCUM] endpoint is not paper-api — refusing to place an order");
        return None;
    }

    let body = json!({
        "symbol": symbol,
        "notional": format!("{:.2}", usd),
        "side": "buy",
        "type": "market",
        "time_in_force": "day",
    });
    let resp = reqwest::Client::new()
        .post(format!("{}/v2/orders", base))
        .header("APCA-API-KEY-ID", &key)
        .header("APCA-API-SECRET-KEY", &secret)
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        warn!("[ACCUM] order rejected {}: {}", status, text.chars().take(200).collect::<String>());
        append(json!({
            "date": chrono::Local::now().format("%Y-%m-%d").to_string(),
            "symbol": symbol, "usd": usd, "outcome": "rejected",
            "detail": text.chars().take(300).collect::<String>(),
            "timestamp": chrono::Local::now().to_rfc3339(),
        }));
        return None;
    }
    resp.json::<Value>().await.ok()
}

/// Make today's contribution, once, if the market is open.
pub async fn contribute() -> Value {
    if !crate::config::ACCUMULATOR_ENABLED {
        return json!({"skipped": "disabled"});
    }
    let symbol = crate::config::ACCUMULATOR_SYMBOL;
    let usd = crate::config::ACCUMULATOR_DAILY_USD;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    if already_contributed(&today) {
        return json!({"skipped": "already contributed today", "date": today});
    }

    let order = match buy_notional(symbol, usd).await {
        Some(o) => o,
        None => return json!({"error": "order not placed", "date": today}),
    };
    let order_id = order["id"].as_str().unwrap_or("").to_string();

    // Poll briefly for the fill. Unlike the intraday path there is no urgency
    // and nothing is blocked by waiting, but a fixed dollar order on a liquid
    // ETF fills fast, so a short window is plenty.
    let (key, secret, base) = (
        std::env::var("APCA_API_KEY_ID").unwrap_or_default(),
        std::env::var("APCA_API_SECRET_KEY").unwrap_or_default(),
        std::env::var("APCA_API_BASE_URL")
            .unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string()),
    );
    let client = reqwest::Client::new();
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let o = client
            .get(format!("{}/v2/orders/{}", base, order_id))
            .header("APCA-API-KEY-ID", &key)
            .header("APCA-API-SECRET-KEY", &secret)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        let o: Value = match o {
            Ok(r) if r.status().is_success() => match r.json().await { Ok(v) => v, Err(_) => continue },
            _ => continue,
        };
        if o["status"].as_str() != Some("filled") {
            continue;
        }
        let px = o["filled_avg_price"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let qty = o["filled_qty"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        info!("[ACCUM] contributed ${:.2} -> {:.6} {} @ ${:.2}", usd, qty, symbol, px);
        let row = json!({
            "date": today, "symbol": symbol, "usd": usd,
            "shares": qty, "price": px, "outcome": "filled",
            "order_id": order_id, "timestamp": chrono::Local::now().to_rfc3339(),
        });
        append(row.clone());
        return row;
    }

    // Timed out. Record it as pending rather than assuming either outcome; the
    // next cycle re-checks and the day stays unfunded until a fill is seen.
    append(json!({
        "date": today, "symbol": symbol, "usd": usd, "outcome": "pending",
        "order_id": order_id, "timestamp": chrono::Local::now().to_rfc3339(),
        "note": "poll window elapsed; not counted as contributed",
    }));
    json!({"pending": true, "order_id": order_id, "date": today})
}

/// Current state of the accumulation, priced at the broker.
pub async fn status() -> Value {
    let filled: Vec<Value> = rows().into_iter()
        .filter(|r| r["outcome"].as_str() == Some("filled"))
        .collect();
    let contributed: f64 = filled.iter().filter_map(|r| r["usd"].as_f64()).sum();
    let shares: f64 = filled.iter().filter_map(|r| r["shares"].as_f64()).sum();

    let symbol = crate::config::ACCUMULATOR_SYMBOL;
    let mut price = 0.0;
    if let Some(pos) = crate::services::alpaca_broker::positions().await {
        // Prefer the broker's own mark for the held position.
        if let Some(_q) = pos.get(symbol) {
            for p in crate::services::alpaca_broker::positions_detail().await {
                if p["symbol"].as_str() == Some(symbol) {
                    price = p["current_price"].as_f64().unwrap_or(0.0);
                }
            }
        }
    }
    let value = shares * price;

    json!({
        "enabled": crate::config::ACCUMULATOR_ENABLED,
        "symbol": symbol,
        "daily_usd": crate::config::ACCUMULATOR_DAILY_USD,
        "contributions": filled.len(),
        "contributed": (contributed * 100.0).round() / 100.0,
        "shares": (shares * 1e6).round() / 1e6,
        "price": price,
        "value": (value * 100.0).round() / 100.0,
        "profit": ((value - contributed) * 100.0).round() / 100.0,
        "return_pct": if contributed > 0.0 {
            ((value / contributed - 1.0) * 10000.0).round() / 100.0
        } else { 0.0 },
        "note": "Buys only. This book is never sold and is excluded from reconcile.",
    })
}

/// Once a day, shortly after the open, make the contribution.
pub fn spawn() {
    if !crate::config::ACCUMULATOR_ENABLED {
        return;
    }
    tokio::spawn(async move {
        // Let the stack settle before the first attempt.
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            // Only when the market is actually open; a notional market order
            // outside hours is rejected rather than queued.
            let open = crate::services::alpaca_broker::account().await
                .map(|_| market_open_now())
                .unwrap_or(false);
            if open {
                let _ = contribute().await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        }
    });
}

/// 09:30-16:00 ET on a weekday. Deliberately simple: the contribution is
/// idempotent per day and retried every 10 minutes, so a holiday just means the
/// order is rejected and the day records nothing.
fn market_open_now() -> bool {
    use chrono::{Datelike, Timelike};
    let utc = chrono::Utc::now();
    let month = utc.month();
    let offset = if (3..=10).contains(&month) { 4 } else { 5 };
    let et = utc - chrono::Duration::hours(offset);
    let dow = et.weekday().num_days_from_monday();
    if dow > 4 {
        return false;
    }
    let mins = et.hour() * 60 + et.minute();
    (9 * 60 + 30..16 * 60).contains(&mins)
}
