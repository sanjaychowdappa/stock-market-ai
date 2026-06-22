//! Shared application state — engines, trader, tracker, all signals.

use crate::config::TOP_SYMBOLS;
use crate::services::{
    alpaca_stream,
    daily_stock_picker::{self, SharedDailyFocus},
    daily_tracker::DailyTracker,
    institutional_signals::{self, CotData, GammaExposure, VolumeProfile},
    kronos_onnx::{self, SharedKronos},
    paper_trader::PaperTrader,
    realtime_engine::RealtimeEngine,
};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;

/// Shared institutional signals state (updated at startup + periodically).
pub struct InstitutionalState {
    pub gex: DashMap<String, GammaExposure>,
    pub volume_profiles: DashMap<String, VolumeProfile>,
    pub cot: Mutex<CotData>,
}

pub struct AppState {
    pub engines: DashMap<String, Arc<RealtimeEngine>>,
    pub kronos: SharedKronos,
    pub trader: Mutex<PaperTrader>,
    pub tracker: Mutex<DailyTracker>,
    pub daily_focus: SharedDailyFocus,
    pub sp500_scan: daily_stock_picker::SharedSp500Scan,
    pub institutional: Arc<InstitutionalState>,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let kronos = kronos_onnx::create_shared();

        if let Err(e) = kronos_onnx::load_models(&kronos) {
            tracing::warn!("Kronos ONNX not loaded: {}", e);
        }

        let daily_focus = daily_stock_picker::create_shared();
        let sp500_scan = daily_stock_picker::create_scan_shared();
        let institutional = Arc::new(InstitutionalState {
            gex: DashMap::new(),
            volume_profiles: DashMap::new(),
            cot: Mutex::new(CotData::new()),
        });

        let state = Arc::new(Self {
            engines: DashMap::new(),
            kronos: kronos.clone(),
            trader: Mutex::new(PaperTrader::new()),
            tracker: Mutex::new(DailyTracker::new()),
            daily_focus: daily_focus.clone(),
            sp500_scan: sp500_scan.clone(),
            institutional: institutional.clone(),
        });

        // ── Spawn S&P 500 daily scanner (Phase 1) ──
        let scan2 = sp500_scan.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(45)).await;
            daily_stock_picker::run_sp500_scan(&scan2).await;
            // Re-scan once per day
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400));
            interval.tick().await; // consume immediate first tick
            loop {
                interval.tick().await;
                daily_stock_picker::run_sp500_scan(&scan2).await;
            }
        });

        // ── Spawn Kronos daily ranking ──
        let focus2 = daily_focus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;
            daily_stock_picker::run_kronos_ranking(&focus2).await;
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(7200));
            loop {
                interval.tick().await;
                tracing::info!("Running midday Kronos re-ranking...");
                daily_stock_picker::run_kronos_ranking(&focus2).await;
            }
        });

        // ── Spawn institutional signals computation ──
        let inst2 = institutional.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            compute_institutional_signals(&inst2).await;

            // Refresh every 30 minutes
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1800));
            loop {
                interval.tick().await;
                tracing::info!("Refreshing institutional signals...");
                compute_institutional_signals(&inst2).await;
            }
        });

        // Create engines
        for &sym in TOP_SYMBOLS {
            let engine = RealtimeEngine::new(sym, kronos.clone());
            state.engines.insert(sym.to_string(), engine);
        }

        // ── Spawn Alpaca real-time stream ──
        let state2 = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            let (tick_tx, bar_tx) = alpaca_stream::spawn_stream(TOP_SYMBOLS);
            let mut tick_rx = tick_tx.subscribe();
            let mut bar_rx = bar_tx.subscribe();
            tracing::info!("Alpaca stream dispatcher started");
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

/// Compute GEX, Volume Profile, and COT for all symbols.
async fn compute_institutional_signals(inst: &InstitutionalState) {
    tracing::info!("=== COMPUTING INSTITUTIONAL SIGNALS ===");

    // 1. Fetch COT data (market-wide, not per-symbol)
    let cot = institutional_signals::fetch_cot_data().await;
    tracing::info!("  COT: commercial_net={:.0} signal={:.2} date={}",
        cot.commercial_net, cot.signal, cot.report_date);
    *inst.cot.lock() = cot;

    // 2. Per-symbol: GEX (uses DAILY bars) + Volume Profile (uses 1-min bars)
    for &sym in TOP_SYMBOLS {
        // GEX needs daily bars for realized volatility (20+ trading days)
        match alpaca_stream::fetch_daily_bars(sym, 60).await {
            Ok(daily_bars) if daily_bars.len() >= 20 => {
                let daily_prices: Vec<f64> = daily_bars.iter()
                    .filter_map(|b| b["close"].as_f64())
                    .collect();
                let current_price = *daily_prices.last().unwrap_or(&0.0);
                let atr_vals: Vec<f64> = daily_bars.iter()
                    .filter_map(|b| {
                        let h = b["high"].as_f64()?;
                        let l = b["low"].as_f64()?;
                        Some(h - l)
                    }).collect();
                let atr = if !atr_vals.is_empty() {
                    atr_vals.iter().rev().take(14).sum::<f64>() / 14.0_f64.min(atr_vals.len() as f64)
                } else { current_price * 0.01 };

                let gex = institutional_signals::estimate_gex(current_price, &daily_prices, atr);
                tracing::info!("  {}: GEX={:.2} regime={} signal={:.2} (from {} daily bars)",
                    sym, gex.gex_level, gex.regime, gex.signal, daily_bars.len());
                inst.gex.insert(sym.to_string(), gex);
            }
            Ok(bars) => {
                tracing::warn!("  {}: Only {} daily bars for GEX, need 20+", sym, bars.len());
            }
            Err(e) => {
                tracing::warn!("  {}: Failed to fetch daily bars for GEX: {}", sym, e);
            }
        }

        // Volume Profile uses intraday 1-min bars (where volume actually traded today)
        match alpaca_stream::fetch_historical_bars(sym, 500).await {
            Ok(bars) if bars.len() >= 20 => {
                let current_price = bars.last()
                    .and_then(|b| b["close"].as_f64())
                    .unwrap_or(0.0);
                let vp = institutional_signals::compute_volume_profile(&bars, current_price, 50);
                tracing::info!("  {}: VP POC=${:.2} VA=[${:.2}-${:.2}] pos={} signal={:.2}",
                    sym, vp.poc_price, vp.va_low, vp.va_high, vp.position, vp.signal);
                inst.volume_profiles.insert(sym.to_string(), vp);
            }
            Ok(bars) => {
                tracing::warn!("  {}: Only {} intraday bars for VP, need 20+", sym, bars.len());
            }
            Err(e) => {
                tracing::warn!("  {}: Failed to fetch intraday bars for VP: {}", sym, e);
            }
        }
    }

    tracing::info!("=== INSTITUTIONAL SIGNALS COMPLETE ===");
}
