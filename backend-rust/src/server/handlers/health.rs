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
