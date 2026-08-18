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
        .route("/api/momentum", get(handlers::health::momentum))
        .route("/api/sector-leaders", get(handlers::health::sector_leaders))
        .route("/api/sector-leaders/scan", get(handlers::health::sector_leaders_scan))
        .route("/api/profit", get(handlers::health::profit))
        .route("/api/experiments", get(handlers::health::experiments))
        .route("/api/broker", get(handlers::health::broker))
        .route("/api/broker/sync", get(handlers::health::broker_sync))
        .route("/api/broker/backfill", get(handlers::health::broker_backfill))
        .route("/api/damage-control", get(handlers::health::damage_control))
        .route("/api/benchmark", get(handlers::health::benchmark))
        .route("/api/agentic", get(handlers::health::agentic))
        .route("/api/agentic/run", get(handlers::health::agentic_run))
        .route("/api/layer-monitor", get(handlers::health::layer_monitor))
        .route("/api/stocks/{symbol}", get(handlers::stocks::get_stock))
        .route("/api/bars/{symbol}", get(handlers::stocks::get_bars))
        .route("/api/signals/{symbol}", get(handlers::stocks::get_signals))
        .route("/api/patterns/{symbol}", get(handlers::stocks::get_patterns))
        .route("/api/realtime/{symbol}/predict", get(handlers::realtime::predict))
        // WebSocket endpoints
        .route("/ws/live/{symbol}", get(websocket::ws_live))
        .route("/ws/predict/{symbol}", get(websocket::ws_predict))
        .route("/ws/paper-trade", get(websocket::ws_paper_trade))
        .with_state(state)
}
