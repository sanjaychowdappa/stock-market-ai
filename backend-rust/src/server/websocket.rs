//! WebSocket upgrade handlers for live streaming.

use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Path, State},
    response::IntoResponse,
};
use crate::state::AppState;
use std::sync::Arc;
use tracing::debug;

/// /ws/live/{symbol} — raw tick stream for live chart
pub async fn ws_live(
    ws: WebSocketUpgrade,
    Path(symbol): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let symbol = symbol.to_uppercase();
    ws.on_upgrade(move |socket| handle_live(socket, symbol, state))
}

async fn handle_live(mut socket: WebSocket, symbol: String, state: Arc<AppState>) {
    let engine = state.get_engine(&symbol);
    let mut rx = engine.subscribe_ticks();

    loop {
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(tick)) => {
                let msg = Message::Text(tick.to_string().into());
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            Ok(Err(_)) => break,   // Channel closed
            Err(_) => continue,     // Timeout — keep alive
        }
    }
    debug!("ws_live disconnected: {}", symbol);
}

/// /ws/predict/{symbol} — per-second prediction stream
pub async fn ws_predict(
    ws: WebSocketUpgrade,
    Path(symbol): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let symbol = symbol.to_uppercase();
    ws.on_upgrade(move |socket| handle_predict(socket, symbol, state))
}

async fn handle_predict(mut socket: WebSocket, symbol: String, state: Arc<AppState>) {
    let engine = state.get_engine(&symbol);
    let mut rx = engine.subscribe_predictions();

    loop {
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(data)) => {
                let msg = Message::Text(data.to_string().into());
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }
    debug!("ws_predict disconnected: {}", symbol);
}

/// /ws/paper-trade — live paper trading portfolio stream
pub async fn ws_paper_trade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_paper_trade(socket, state))
}

async fn handle_paper_trade(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = {
        let trader = state.trader.lock();
        trader.subscribe()
    };

    loop {
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(data)) => {
                let msg = Message::Text(data.to_string().into());
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }
    debug!("ws_paper_trade disconnected");
}
