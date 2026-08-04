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
use chrono::{Datelike, Timelike};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, error};

/// Get current US Eastern time components from UTC.
fn et_now() -> (u32, u32, u32) {
    let utc = chrono::Utc::now();
    let month = utc.month();
    let offset: i64 = if month >= 3 && month <= 10 { 4 } else { 5 };
    let hour = (utc.hour() as i64 - offset).rem_euclid(24) as u32;
    let minute = utc.minute();
    let weekday = utc.weekday().num_days_from_monday();
    (hour, minute, weekday)
}

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
        let mut focus_sync_counter = 0u32;
        let mut eod_report_published = false;
        let mut saw_market_open = false;
        let mut market_ticks = 0u32;

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

            // Sync daily focus + institutional signals into paper trader (every 30s)
            focus_sync_counter += 1;
            if focus_sync_counter >= 30 {
                focus_sync_counter = 0;

                // Sync Kronos daily bias
                let focus = state.daily_focus.lock();
                let mut trader = state.trader.lock();
                for r in &focus.rankings {
                    trader.set_kronos_bias(&r.symbol, r.predicted_change_pct);
                }
                drop(focus);

                // Sync GEX, Volume Profile, COT into paper trader
                let cot_signal = state.institutional.cot.lock().signal;
                for sym in crate::config::TOP_SYMBOLS {
                    let gex = state.institutional.gex.get(*sym);
                    let vp = state.institutional.volume_profiles.get(*sym);

                    let (gex_sig, gex_reg) = gex.as_ref()
                        .map(|g| (g.signal, g.regime.clone()))
                        .unwrap_or((0.0, "neutral".to_string()));
                    let (vp_sig, vp_pos) = vp.as_ref()
                        .map(|v| (v.signal, v.position.clone()))
                        .unwrap_or((0.0, "unknown".to_string()));

                    trader.set_institutional_signals(
                        sym, gex_sig, &gex_reg, vp_sig, &vp_pos, cot_signal,
                    );
                }
            }

            // Track if we've seen market hours (needed to gate EOD)
            {
                let (et_h, et_m, wd) = et_now();
                let mins = et_h * 60 + et_m;
                let is_open = wd < 5 && mins >= 9 * 60 + 30 && mins < 16 * 60;
                if is_open {
                    saw_market_open = true;
                    market_ticks += 1;
                }
            }

            // EOD check every 60 seconds
            eod_counter += 1;
            if eod_counter >= 60 {
                eod_counter = 0;

                // Auto-publish EOD report at 4:05 PM ET
                // GUARD: Only fire if we actually traded during market hours this session.
                // This prevents immediate firing when restarting after 4:05 PM.
                if !eod_report_published && saw_market_open && market_ticks > 60 {
                    let (et_h, et_m, _wd) = et_now();
                    if (et_h == 16 && et_m >= 5) || et_h > 16 {
                        eod_report_published = true;
                        info!("=== EOD PIPELINE STARTING ===");

                        // Step 1: Generate and save EOD report
                        info!("[EOD 1/3] Publishing EOD report...");
                        let report = {
                            let trader = state.trader.lock();
                            let tracker = state.tracker.lock();
                            crate::services::eod_publisher::generate_report(&trader, &tracker)
                        };
                        let text = crate::services::eod_publisher::generate_text_summary(&report);
                        match crate::services::eod_publisher::save_report(&report, &text).await {
                            Ok(path) => info!("  EOD report saved to {:?}", path),
                            Err(e) => error!("  Failed to save EOD report: {}", e),
                        }

                        // Step 2: Record day in performance ledger + calculate net profit
                        info!("[EOD 2/3] Recording performance & calculating net profit...");
                        let mut ledger = crate::services::performance_ledger::PerformanceLedger::load().await;
                        let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

                        // AUDIT FIX (2026-07-29): the 3:55pm daily skim flattens
                        // and resets capital BEFORE this 4:05pm report runs, so
                        // report.portfolio.total_pnl is always $0 — the ledger was
                        // recording zero every day and was blind to performance.
                        // daily_profit.jsonl (written by the skim) is the
                        // authoritative record, so read the running total from it
                        // and fall back to the report only if it is unavailable.
                        // SUM the banked days rather than trusting the last row's
                        // stored cumulative. Two writers append to this ledger, so
                        // "last row" could be stale or out of order.
                        let cumulative_pnl = tokio::fs::read_to_string("/app/reports/daily_profit.jsonl")
                            .await.ok()
                            .map(|c| c.lines().filter_map(|l|
                                serde_json::from_str::<serde_json::Value>(l).ok())
                                .filter_map(|v| v["day_pnl"].as_f64())
                                .sum::<f64>())
                            .unwrap_or_else(|| report["portfolio"]["total_pnl"].as_f64().unwrap_or(0.0));
                        let total_trades = report["trading_stats"]["total_trades"].as_u64().unwrap_or(0) as u32;
                        let winning_trades = report["trading_stats"]["winning_trades"].as_u64().unwrap_or(0) as u32;
                        let avg_hold = report["trading_stats"]["avg_hold_seconds"].as_u64().unwrap_or(0);
                        let avg_pnl = report["trading_stats"]["avg_pnl_per_trade"].as_f64().unwrap_or(0.0);
                        let portfolio_value = report["portfolio"]["total_value"].as_f64().unwrap_or(100.0);

                        // Share count and sell value used to be INVENTED here:
                        //     let avg_trade_value = portfolio_value * 0.8;
                        //     let avg_price = 300.0;
                        // Those two guesses fed the spread/SEC/TAF/PFOF model, which
                        // then printed a "NET P&L" and an efficiency score to four
                        // decimal places — fabricated precision built on a made-up
                        // $300 average price. Passing zero disables the modeled-cost
                        // path instead of dressing up guesses as measurements. Real
                        // execution cost is measured from actual fills at /api/broker.
                        let day_record = ledger.record_day(
                            &today_str, cumulative_pnl, total_trades, winning_trades,
                            avg_hold, avg_pnl, 0.0, 0.0, portfolio_value,
                        );

                        info!("  Gross P&L (simulator, uncosted): ${:.2}", day_record.gross_pnl);
                        info!("  NOTE: fee/tax/efficiency modeling disabled — its share-count inputs");
                        info!("        were fabricated. Real costs: GET /api/broker (Alpaca fills).");

                        // Day comparison
                        let comparison = ledger.compare_days();
                        let improved = comparison["comparison"]["improved"].as_bool().unwrap_or(true);
                        if let Some(verdict) = comparison["comparison"]["verdict"].as_str() {
                            info!("  Verdict: {}", verdict);
                        }

                        // Step 3: Auto-tune if not improved
                        info!("[EOD 3/3] Strategy analysis...");
                        if !improved {
                            info!("  Performance DEGRADED — running auto-tuner...");
                            let (tuned, tune_report) = crate::services::strategy_tuner::analyze_and_tune(&ledger);

                            if let Some(diags) = tune_report["diagnosis"].as_array() {
                                for d in diags {
                                    info!("  Diagnosis: {}", d.as_str().unwrap_or("?"));
                                }
                            }
                            if let Some(acts) = tune_report["actions"].as_array() {
                                for a in acts {
                                    info!("  Action: {}", a.as_str().unwrap_or("?"));
                                }
                            }

                            crate::services::strategy_tuner::save_tuned_params(&tuned).await;
                            info!("  Tuned params saved — will take effect on next restart");
                        } else {
                            info!("  Performance OK — keeping current strategy");
                        }

                        // Save ledger
                        ledger.save().await;
                        info!("  Performance ledger saved ({} days)", ledger.days.len());

                        // Save comparison report
                        let perf_report = serde_json::to_string_pretty(&comparison).unwrap_or_default();
                        let perf_path = format!("/app/reports/performance_{}.json", today_str);
                        let _ = tokio::fs::write(&perf_path, perf_report).await;

                        info!("=== EOD PIPELINE COMPLETE ===");
                    }
                }

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

            // Periodic save + layer monitor dump every 5 minutes
            save_counter += 1;
            if save_counter >= 300 {
                save_counter = 0;
                // Dump layer block stats
                {
                    let trader = state.trader.lock();
                    if let Some(payload) = &trader.last_payload {
                        if let Some(monitor) = payload.get("agent_monitor") {
                            let passed = monitor["total_passed"].as_u64().unwrap_or(0);
                            let evaluated = monitor["total_evaluated"].as_u64().unwrap_or(0);
                            let rate = monitor["filter_rate_pct"].as_f64().unwrap_or(0.0);
                            let vetoed = monitor["vetoed"].as_u64().unwrap_or(0);
                            let weak = monitor["score_too_low"].as_u64().unwrap_or(0);
                            info!("=== AGENT MONITOR (5min) === passed={} evaluated={} filter_rate={:.1}% vetoed={} weak_score={}",
                                passed, evaluated, rate, vetoed, weak);
                        }
                    }
                }
                let mut tracker = state.tracker.lock();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(tracker.save());
                });
            }
        }
    });
}
