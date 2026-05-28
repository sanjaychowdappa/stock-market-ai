//! Stock Market AI — Rust backend.
//!
//! High-performance real-time trading engine with:
//! - ONNX Runtime GPU inference (Kronos transformer)
//! - Ornstein-Uhlenbeck micro-tick simulation
//! - 15 candlestick pattern detectors
//! - Momentum scalping paper trader ($100 → $150 challenge)
//! - Daily prediction tracking + auto fine-tune trigger
//!
//! Replaces the Python FastAPI backend with ~10x lower latency.

mod config;
mod models;
mod server;
mod services;
mod state;

use crate::config::TOP_SYMBOLS;
use crate::state::AppState;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, error};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "stock_market_ai=info,tower_http=info".into()),
        )
        .init();

    info!("Stock Market AI — Rust Engine starting...");
    info!("Symbols: {:?}", TOP_SYMBOLS);

    // Build shared state (creates engines, loads ONNX models)
    let state = AppState::new();
    info!("AppState initialized — {} engines running", TOP_SYMBOLS.len());

    // Spawn the orchestration loop (paper trading + daily tracking)
    spawn_orchestrator(state.clone());

    // Build router
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = server::routes::build(state).layer(cors);

    // Start server
    let addr = "0.0.0.0:8000";
    info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

/// Background orchestrator: feeds engine data into paper trader and daily tracker.
fn spawn_orchestrator(state: Arc<AppState>) {
    // Paper trading + daily tracker loop (1 Hz)
    tokio::spawn(async move {
        // Wait for engines to warm up
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Subscribe to all engine prediction streams
        let mut receivers: Vec<(String, tokio::sync::broadcast::Receiver<serde_json::Value>)> = Vec::new();
        for &sym in TOP_SYMBOLS {
            let engine = state.get_engine(sym);
            let rx = engine.subscribe_predictions();
            receivers.push((sym.to_string(), rx));
        }

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        let mut eod_counter = 0u32;
        let mut save_counter = 0u32;

        loop {
            interval.tick().await;

            // Drain prediction queues and feed to trader + tracker
            for (sym, rx) in &mut receivers {
                while let Ok(data) = rx.try_recv() {
                    // Feed paper trader
                    {
                        let mut trader = state.trader.lock();
                        trader.tick(sym, &data);
                    }
                    // Feed daily tracker
                    {
                        let mut tracker = state.tracker.lock();
                        tracker.feed_prediction(sym, &data);
                    }
                }
            }

            // Broadcast paper trader state
            let payload = {
                let trader = state.trader.lock();
                trader.build_payload()
            };
            {
                let trader = state.trader.lock();
                let _ = trader.tx.send(payload.clone());
            }
            {
                let mut trader = state.trader.lock();
                trader.last_payload = Some(payload.clone());
            }

            // Feed tracker with portfolio data
            {
                let mut tracker = state.tracker.lock();
                tracker.feed_portfolio(&payload);
            }

            // EOD check every 60 seconds
            eod_counter += 1;
            if eod_counter >= 60 {
                eod_counter = 0;
                let should_finetune = {
                    let mut tracker = state.tracker.lock();
                    tracker.check_eod()
                };
                if should_finetune {
                    // Save + evaluate outside the lock (extract data first)
                    let mut tracker = state.tracker.lock();
                    // save() and evaluate use tokio::fs and reqwest internally.
                    // Since parking_lot MutexGuard isn't Send, we call them
                    // in a sync-compatible way via block_in_place.
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            tracker.save().await;
                            tracker.evaluate_and_finetune().await;
                        });
                    });
                }
            }

            // Periodic save every 5 minutes
            save_counter += 1;
            if save_counter >= 300 {
                save_counter = 0;
                let mut tracker = state.tracker.lock();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(tracker.save());
                });
            }
        }
    });
}
