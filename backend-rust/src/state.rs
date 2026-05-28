//! Shared application state — engines, trader, tracker, Kronos.

use crate::config::TOP_SYMBOLS;
use crate::services::{
    daily_tracker::DailyTracker,
    kronos_onnx::{self, SharedKronos},
    paper_trader::PaperTrader,
    realtime_engine::RealtimeEngine,
};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    /// Per-symbol real-time engines.
    engines: DashMap<String, Arc<RealtimeEngine>>,
    /// Shared Kronos ONNX session.
    pub kronos: SharedKronos,
    /// Paper trading engine.
    pub trader: Mutex<PaperTrader>,
    /// Daily prediction tracker.
    pub tracker: Mutex<DailyTracker>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let kronos = kronos_onnx::create_shared();

        // Try to load ONNX models (non-fatal if missing)
        if let Err(e) = kronos_onnx::load_models(&kronos) {
            tracing::warn!("Kronos ONNX not loaded: {} — running pattern-only mode", e);
        }

        let state = Arc::new(Self {
            engines: DashMap::new(),
            kronos: kronos.clone(),
            trader: Mutex::new(PaperTrader::new()),
            tracker: Mutex::new(DailyTracker::new()),
        });

        // Pre-create engines for all tracked symbols
        for &sym in TOP_SYMBOLS {
            let engine = RealtimeEngine::new(sym, kronos.clone());
            state.engines.insert(sym.to_string(), engine);
        }

        state
    }

    /// Get or create an engine for the given symbol.
    pub fn get_engine(&self, symbol: &str) -> Arc<RealtimeEngine> {
        self.engines
            .entry(symbol.to_string())
            .or_insert_with(|| RealtimeEngine::new(symbol, self.kronos.clone()))
            .clone()
    }
}
