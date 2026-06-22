//! Route registration.

use axum::{routing::get, Router};
use crate::state::AppState;
use crate::server::{handlers, websocket};
use std::sync::Arc;

pub fn build(state: Arc<AppState>) -> Router {
    Router::new()
        // REST endpoints
        .route("/api/health", get(handlers::health::health))
        .route("/api/daily/status", get(handlers::health::daily_status))
        .route("/api/eod/report", get(handlers::health::eod_report))
        .route("/api/performance", get(handlers::health::performance))
        .route("/api/institutional", get(handlers::health::institutional))
        .route("/api/scan", get(handlers::health::sp500_scan))
        .route("/api/layer-monitor", get(handlers::health::layer_monitor))
        .route("/api/stocks/{symbol}", get(handlers::stocks::get_stock))
        .route("/api/signals/{symbol}", get(handlers::stocks::get_signals))
        .route("/api/patterns/{symbol}", get(handlers::stocks::get_patterns))
        .route("/api/realtime/{symbol}/predict", get(handlers::realtime::predict))
        // WebSocket endpoints
        .route("/ws/live/{symbol}", get(websocket::ws_live))
        .route("/ws/predict/{symbol}", get(websocket::ws_predict))
        .route("/ws/paper-trade", get(websocket::ws_paper_trade))
        .with_state(state)
}
