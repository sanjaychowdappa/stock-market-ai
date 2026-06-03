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

pub async fn performance(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let ledger = crate::services::performance_ledger::PerformanceLedger::load().await;
    let comparison = ledger.compare_days();
    Json(comparison)
}
