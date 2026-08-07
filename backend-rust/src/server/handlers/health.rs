use axum::extract::State;
use axum::Json;
use crate::state::AppState;
use std::sync::Arc;
use chrono::Datelike;

pub async fn health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "engine": "rust",
        "version": crate::config::MODEL_VERSION,
        "config_frozen_until": crate::config::CONFIG_FREEZE_UNTIL,
        "symbols": crate::config::TOP_SYMBOLS,
    }))
}

pub async fn daily_status(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tracker = state.tracker.lock();
    Json(tracker.status())
}

pub async fn eod_report(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let report = {
        let trader = state.trader.lock();
        let tracker = state.tracker.lock();
        crate::services::eod_publisher::generate_report(&trader, &tracker)
    };
    let text = crate::services::eod_publisher::generate_text_summary(&report);

    // Save to disk
    let save_result = crate::services::eod_publisher::save_report(&report, &text).await;
    let saved_to = match save_result {
        Ok(path) => format!("{}", path.display()),
        Err(e) => format!("error: {}", e),
    };

    Json(serde_json::json!({
        "report": report,
        "text_summary": text,
        "saved_to": saved_to,
    }))
}

pub async fn institutional(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cot = state.institutional.cot.lock().clone();
    let mut symbols = serde_json::Map::new();
    for sym in crate::config::TOP_SYMBOLS {
        let gex = state.institutional.gex.get(*sym);
        let vp = state.institutional.volume_profiles.get(*sym);
        let engine = state.get_engine(sym);
        let last_payload = engine.get_last_payload();
        let cvd_json = last_payload.as_ref()
            .and_then(|p| p["cvd"].as_object().cloned())
            .unwrap_or_default();
        let kalman_json = last_payload.as_ref()
            .and_then(|p| p["kalman"].as_object().cloned())
            .unwrap_or_default();

        symbols.insert(sym.to_string(), serde_json::json!({
            "gex": gex.as_ref().map(|g| serde_json::json!({
                "level": g.gex_level, "regime": g.regime,
                "flip_price": g.flip_price, "signal": g.signal,
                "vix": g.vix_level, "put_call_ratio": g.put_call_ratio,
            })),
            "volume_profile": vp.as_ref().map(|v| serde_json::json!({
                "poc": v.poc_price, "va_high": v.va_high, "va_low": v.va_low,
                "position": v.position, "signal": v.signal,
            })),
            "cvd": cvd_json,
            "kalman": kalman_json,
        }));
    }
    Json(serde_json::json!({
        "cot": { "report_date": cot.report_date, "commercial_net": cot.commercial_net,
                 "speculator_net": cot.speculator_net, "signal": cot.signal, "extreme": cot.extreme },
        "symbols": symbols,
        "focus": state.daily_focus.lock().to_json(),
    }))
}


pub async fn momentum(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(state.momentum.lock().to_json())
}

pub async fn experiments(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let trader = state.trader.lock();
    Json(trader.experiments_json())
}

pub async fn exp1(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let trader = state.trader.lock();
    Json(trader.exp1_json())
}

/// Alpaca paper account snapshot + simulator-vs-reality fill comparison.
pub async fn broker(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let acct = crate::services::alpaca_broker::account().await;
    let fills = crate::services::alpaca_broker::fills_summary().await;
    let real = crate::services::alpaca_broker::real_pnl().await;
    let equity = crate::services::alpaca_broker::equity_pnl().await;
    // Positions straight from the broker. Never the simulator's book — the two
    // diverge on rejections, partial fills and halts, and on 2026-08-05 the
    // simulator showed five holdings while the real account was flat.
    let positions = crate::services::alpaca_broker::positions_detail().await;
    Json(serde_json::json!({
        "mode": "Alpaca PAPER is the REAL scoreboard — real fills, real slippage, real rejections. The simulator is only the decision engine.",
        // Authoritative: Alpaca's own equity curve.
        "equity_pnl": equity,
        "positions": positions,
        // Diagnostic only: re-derived by FIFO-matching our fill log, so it can
        // inherit our own recording bugs. Useful for attributing cost between
        // simulator-assumed and real prices; NOT the result.
        "real_pnl": real,
        "connected": acct.is_some(),
        "account": acct.map(|a| serde_json::json!({
            "status": a["status"],
            "cash": a["cash"],
            "equity": a["equity"],
            "buying_power": a["buying_power"],
            "daytrade_count": a["daytrade_count"],
            "pattern_day_trader": a["pattern_day_trader"],
        })),
        "fills": fills,
    }))
}

/// Force an immediate reconcile so Alpaca matches the simulator's book.
pub async fn broker_sync(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let (qty, px) = { state.trader.lock().book_snapshot() };
    Json(crate::services::alpaca_broker::reconcile(qty, px).await)
}

/// Damage-control state: the floor, current headroom, and halt status.
pub async fn damage_control(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let trader = state.trader.lock();
    Json(trader.build_payload()["damage_control"].clone())
}

/// The strategy against buy-and-hold — the benchmark that decides whether any
/// of this is worth running. Strategy P&L comes from Alpaca's equity curve.
pub async fn benchmark(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let eq = crate::services::alpaca_broker::equity_pnl().await;
    let strategy_pnl = eq["net_pnl"].as_f64().unwrap_or(0.0);

    // Both sides must cover the SAME window. Using the kill-criterion date
    // compared a zero-length hold against the strategy's whole life and
    // reported "buy-and-hold $0.00 vs strategy -$44.66" — which reads as the
    // benchmark making nothing rather than as no elapsed time at all.
    // strategy_pnl spans every real fill, so the benchmark starts at the first.
    let first_trade_day = tokio::fs::read_to_string("/app/reports/broker_fills.jsonl")
        .await.ok()
        .and_then(|c| c.lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter_map(|v| v["timestamp"].as_str().map(|t| t[..10].to_string()))
            .min())
        .unwrap_or_else(|| crate::config::LIVE_KILL_START_DATE.to_string());

    Json(crate::services::alpaca_broker::buy_and_hold_benchmark(
        &first_trade_day,
        crate::config::INITIAL_CASH,
        strategy_pnl,
    ).await)
}

/// agentic_test supervisor status + findings.
pub async fn agentic(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(state.agentic.lock().to_json())
}

/// Force an immediate supervisory pass (otherwise runs every 15 min).
pub async fn agentic_run(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    crate::services::agentic_test::run_cycle(&state, &state.agentic).await;
    Json(state.agentic.lock().to_json())
}

/// Daily profit ledger + weekly aggregation for the fixed-capital day trader.
pub async fn profit(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    use std::collections::BTreeMap;
    let content = tokio::fs::read_to_string("/app/reports/daily_profit.jsonl").await.unwrap_or_default();
    let mut days: Vec<serde_json::Value> = Vec::new();
    let mut weekly: BTreeMap<String, (f64, u32)> = BTreeMap::new();
    let mut total = 0.0;
    let mut quarantined = 0u32;
    let mut quarantined_sum = 0.0;
    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let date = v["date"].as_str().unwrap_or("").to_string();
            let pnl = v["day_pnl"].as_f64().unwrap_or(0.0);
            // Quarantined rows are kept for audit but excluded from every total.
            if v["reliable"].as_bool() == Some(false) {
                quarantined += 1;
                quarantined_sum += pnl;
                continue;
            }
            total += pnl;
            // ISO-ish week key: year + week number via naive date
            if let Ok(d) = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
                let iso = d.iso_week();
                let key = format!("{}-W{:02}", iso.year(), iso.week());
                let e = weekly.entry(key).or_insert((0.0, 0));
                e.0 += pnl; e.1 += 1;
            }
            days.push(serde_json::json!({"date": date, "day_pnl": pnl,
                "cumulative_pnl": v["cumulative_pnl"].as_f64().unwrap_or(0.0)}));
        }
    }
    let weeks: Vec<serde_json::Value> = weekly.iter().map(|(k, (p, n))| serde_json::json!({
        "week": k, "profit": (p*100.0).round()/100.0, "days": n,
    })).collect();
    let recent: Vec<&serde_json::Value> = days.iter().rev().take(15).collect();

    // Duplicate dates in the ledger mean `total` double-counts. Surface the
    // count so no consumer can mistake this for a clean figure.
    let mut seen = std::collections::HashSet::new();
    let dupes = days.iter()
        .filter(|d| !seen.insert(d["date"].as_str().unwrap_or("").to_string()))
        .count();

    Json(serde_json::json!({
        "is_scoreboard": false,
        "warning": "Simulator ledger — NOT a record of money made. It models no spread, \
                    no slippage and no rejections, and on days a real broker could check \
                    it, it reported gains on days that actually lost money. Real results \
                    come from /api/broker (Alpaca fills) and nowhere else.",
        "quarantined_rows": quarantined,
        "quarantined_sum_excluded": (quarantined_sum*100.0).round()/100.0,
        "quarantine_note": "Rows written before 2026-08-04 double-counted the same dollars: \
                            each morning's carryover re-banked the previous day's skim \
                            (07-31 $72.28 -> 08-03 $72.28; 08-03 $37.14 -> 08-04 $37.14). \
                            The true split cannot be reconstructed, so rather than guess a \
                            corrected figure they are excluded from every total.",
        "duplicate_date_rows": dupes,
        "model": "fixed-capital day trader: $3000/day, profit banked daily, reset each day",
        "capital_per_day": crate::config::INITIAL_CASH,
        "trustworthy_days_recorded": days.len(),
        "trustworthy_banked_profit": (total*100.0).round()/100.0,
        "weekly_profit": weeks,
        "recent_days": recent,
    }))
}

pub async fn layer_monitor(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let trader = state.trader.lock();
    if let Some(payload) = &trader.last_payload {
        if let Some(monitor) = payload.get("agent_monitor") {
            return Json(monitor.clone());
        }
    }
    Json(serde_json::json!({"status": "no data yet"}))
}

pub async fn performance(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let ledger = crate::services::performance_ledger::PerformanceLedger::load().await;
    let comparison = ledger.compare_days();
    Json(comparison)
}
