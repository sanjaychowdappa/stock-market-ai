use axum::extract::State;
use axum::Json;
use crate::state::AppState;
use std::sync::Arc;

pub async fn health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "engine": "rust",
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

pub async fn sp500_scan(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(state.sp500_scan.lock().to_json())
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
