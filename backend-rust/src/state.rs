//! Shared application state — engines, trader, tracker, Kronos, Alpaca stream.

use crate::config::TOP_SYMBOLS;
use crate::services::{
    alpaca_stream,
    daily_stock_picker::{self, SharedDailyFocus},
    daily_tracker::DailyTracker,
    kronos_onnx::{self, SharedKronos},
    paper_trader::PaperTrader,
    realtime_engine::RealtimeEngine,
};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub engines: DashMap<String, Arc<RealtimeEngine>>,
    pub kronos: SharedKronos,
    pub trader: Mutex<PaperTrader>,
    pub tracker: Mutex<DailyTracker>,
    pub daily_focus: SharedDailyFocus,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let kronos = kronos_onnx::create_shared();

        if let Err(e) = kronos_onnx::load_models(&kronos) {
            tracing::warn!("Kronos ONNX not loaded: {}", e);
        }

        let daily_focus = daily_stock_picker::create_shared();

        let state = Arc::new(Self {
            engines: DashMap::new(),
            kronos: kronos.clone(),
            trader: Mutex::new(PaperTrader::new()),
            tracker: Mutex::new(DailyTracker::new()),
            daily_focus: daily_focus.clone(),
        });

        // Spawn Kronos daily ranking (runs at startup + midday refresh)
        let focus2 = daily_focus.clone();
        tokio::spawn(async move {
            // Wait for sidecar to be ready
            tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;

            // Initial ranking
            daily_stock_picker::run_kronos_ranking(&focus2).await;

            // Midday refresh loop (every 2 hours)
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(7200));
            loop {
                interval.tick().await;
                tracing::info!("Running midday Kronos re-ranking...");
                daily_stock_picker::run_kronos_ranking(&focus2).await;
            }
        });

        // Create engines for all symbols
        for &sym in TOP_SYMBOLS {
            let engine = RealtimeEngine::new(sym, kronos.clone());
            state.engines.insert(sym.to_string(), engine);
        }

        // Spawn Alpaca real-time stream and feed ticks into engines
        let state2 = state.clone();
        tokio::spawn(async move {
            // Give engines a moment to init
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            let (tick_tx, bar_tx) = alpaca_stream::spawn_stream(TOP_SYMBOLS);
            let mut tick_rx = tick_tx.subscribe();
            let mut bar_rx = bar_tx.subscribe();

            tracing::info!("Alpaca stream dispatcher started — feeding real-time ticks to engines");

            loop {
                tokio::select! {
                    Ok(tick) = tick_rx.recv() => {
                        if let Some(engine) = state2.engines.get(&tick.symbol) {
                            engine.feed_tick(tick.price, tick.size, tick.timestamp);
                        }
                    }
                    Ok(bar) = bar_rx.recv() => {
                        if let Some(engine) = state2.engines.get(&bar.symbol) {
                            engine.feed_bar(bar.open, bar.high, bar.low, bar.close, bar.volume, bar.timestamp);
                        }
                    }
                    else => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        });

        state
    }

    pub fn get_engine(&self, symbol: &str) -> Arc<RealtimeEngine> {
        self.engines
            .entry(symbol.to_string())
            .or_insert_with(|| RealtimeEngine::new(symbol, self.kronos.clone()))
            .clone()
    }
}
