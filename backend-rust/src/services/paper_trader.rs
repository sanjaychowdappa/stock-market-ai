//! Pattern-driven momentum trader — $100 → $150 challenge.
//!
//! Strategy v3: Kronos sets daily bias, patterns drive trade timing.
//!
//! ENTRY: Pattern signal strong + momentum confirmed + Kronos not opposing
//! EXIT:  Hard stop | Take profit | Momentum exhaustion | Flat timeout
//!
//! Key insight: Kronos predicts daily direction well but oscillates
//! at sub-minute timeframes. Patterns detect real momentum shifts.

use crate::config::*;
use crate::models::{Position, Trade};
use chrono::{Datelike, Local, Timelike};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::info;

pub struct PaperTrader {
    cash: f64,
    positions: HashMap<String, Position>,
    trades: VecDeque<Trade>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    daily_trades: u32,
    start_time: Instant,
    cooldowns: HashMap<String, Instant>,
    signal_history: HashMap<String, VecDeque<f64>>,
    market_data: HashMap<String, MarketSnapshot>,
    /// Kronos daily bias per symbol: >0 = bullish day, <0 = bearish day
    /// Updated every Kronos cycle (~8s) but only used as a filter, not trigger
    kronos_daily_bias: HashMap<String, f64>,
    /// Running average of Kronos direction (smoothed, not raw oscillation)
    kronos_bias_ema: HashMap<String, f64>,
    pub market_open: bool,
    pub tx: broadcast::Sender<serde_json::Value>,
    pub last_payload: Option<serde_json::Value>,
    /// Per-layer block counters for monitoring which layers filter most
    layer_blocks: LayerBlockCounters,
}

/// Tracks how many times each layer blocked an entry — for efficiency analysis.
#[derive(Default, Clone)]
struct LayerBlockCounters {
    kronos_bias: u32,
    kalman_direction: u32,
    pattern_signal: u32,
    kalman_momentum: u32,
    pattern_history: u32,
    cvd_pressure: u32,
    vp_resistance: u32,
    consensus: u32,
    score_too_low: u32,
    total_passed: u32,
}

/// Cached ML agent scores from the Python sidecar (port 8002).
/// When available, these replace the hardcoded if/else scoring.
#[derive(Default, Clone)]
struct CachedAgentScores {
    momentum: f64,
    pattern: f64,
    flow: f64,
    level: f64,
    sentiment: f64,
    meta_score: f64,
    is_trained: bool,
    age_seconds: f64,
}

struct MarketSnapshot {
    price: f64,
    pattern_signal: f64,
    pattern_confidence: f64,
    micro_momentum: f64,
    trend: f64,
    kronos_active: bool,
    kronos_direction: f64,
    // Kalman filter signals
    kalman_momentum: f64,
    kalman_trend_strength: f64,
    kalman_confidence: f64,
    kalman_momentum_building: bool,
    kalman_momentum_fading: bool,
    kalman_direction: String,
    // CVD signals
    cvd_signal: f64,
    cvd_buy_sell_ratio: f64,
    // Institutional signals (updated every 30s from orchestrator)
    gex_signal: f64,
    gex_regime: String,    // "long_gamma", "short_gamma", "neutral"
    vp_signal: f64,
    vp_position: String,   // "above_value", "below_value", "at_poc", "in_value"
    cot_signal: f64,
    session_high: f64,
    session_low: f64,
    // ML agent scores from sidecar (None = use hardcoded fallback)
    ml_agent_scores: Option<CachedAgentScores>,
}

impl PaperTrader {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            cash: INITIAL_CASH,
            positions: HashMap::new(),
            trades: VecDeque::with_capacity(500),
            realized_pnl: 0.0,
            total_trades: 0,
            winning_trades: 0,
            daily_trades: 0,
            start_time: Instant::now(),
            cooldowns: HashMap::new(),
            signal_history: HashMap::new(),
            market_data: HashMap::new(),
            kronos_daily_bias: HashMap::new(),
            kronos_bias_ema: HashMap::new(),
            market_open: false,
            tx,
            last_payload: None,
            layer_blocks: LayerBlockCounters::default(),
        }
    }

    /// Check if US stock market is open (9:30 AM – 4:00 PM ET, Mon–Fri).
    fn is_market_open() -> bool {
        let utc_now = chrono::Utc::now();
        let month = utc_now.month();
        let offset_hours: i64 = if month >= 3 && month <= 10 { 4 } else { 5 };
        let et_hour = (utc_now.hour() as i64 - offset_hours).rem_euclid(24) as u32;
        let et_minute = utc_now.minute();
        let weekday = utc_now.weekday().num_days_from_monday();
        if weekday >= 5 { return false; }
        let mins = et_hour * 60 + et_minute;
        mins >= 9 * 60 + 30 && mins < 16 * 60
    }

    pub fn subscribe(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tx.subscribe()
    }

    /// Set the daily Kronos bias for a symbol (called by daily_stock_picker).
    pub fn set_kronos_bias(&mut self, symbol: &str, bias: f64) {
        self.kronos_daily_bias.insert(symbol.to_string(), bias);
        self.kronos_bias_ema.insert(symbol.to_string(), bias);
    }

    /// Set institutional signals for a symbol (GEX regime, VP position, COT).
    /// Called every 30s from the orchestrator.
    pub fn set_institutional_signals(
        &mut self, symbol: &str,
        gex_signal: f64, gex_regime: &str,
        vp_signal: f64, vp_position: &str,
        cot_signal: f64,
    ) {
        let snap = self.market_data.entry(symbol.to_string()).or_insert_with(|| MarketSnapshot {
            price: 0.0, pattern_signal: 0.0, pattern_confidence: 0.0,
            micro_momentum: 0.0, trend: 0.0, kronos_active: false,
            kronos_direction: 0.0, kalman_momentum: 0.0,
            kalman_trend_strength: 0.0, kalman_confidence: 0.0,
            kalman_momentum_building: false, kalman_momentum_fading: false,
            kalman_direction: "neutral".to_string(),
            cvd_signal: 0.0, cvd_buy_sell_ratio: 1.0,
            gex_signal: 0.0, gex_regime: "neutral".to_string(),
            vp_signal: 0.0, vp_position: "unknown".to_string(),
            cot_signal: 0.0, session_high: 0.0, session_low: 0.0,
            ml_agent_scores: None,
        });
        snap.gex_signal = gex_signal;
        snap.gex_regime = gex_regime.to_string();
        snap.vp_signal = vp_signal;
        snap.vp_position = vp_position.to_string();
        snap.cot_signal = cot_signal;
    }

    pub fn tick(&mut self, symbol: &str, data: &serde_json::Value) {
        let price = data["current_price"].as_f64().unwrap_or(0.0);
        if price <= 0.0 { return; }

        self.market_open = Self::is_market_open();

        // Extract data
        let pattern_signal = data["pattern"]["signal"].as_f64().unwrap_or(0.0);
        let pattern_confidence = data["pattern"]["confidence"].as_f64().unwrap_or(0.0);
        let pattern_direction = data["pattern"]["direction"].as_str().unwrap_or("neutral");
        let predictions = data["predictions"].as_array();

        let mut pred_30s = price;
        let mut pred_60s = price;
        let mut kronos_active = false;
        if let Some(preds) = predictions {
            for p in preds {
                if p["kronos_price"].as_f64().is_some() { kronos_active = true; }
                match p["seconds_ahead"].as_u64() {
                    Some(30) => pred_30s = p["predicted_price"].as_f64().unwrap_or(price),
                    Some(60) => pred_60s = p["predicted_price"].as_f64().unwrap_or(price),
                    _ => {}
                }
            }
        }

        let micro_mom = pattern_confidence
            * if pattern_direction == "bullish" { 1.0 } else { -1.0 };
        let trend = (pred_30s - price) / price;
        let kronos_direction = (pred_60s - price) / price * 100.0;

        // CVD (Cumulative Volume Delta) — buy vs sell pressure
        let cvd = &data["cvd"];
        let cvd_signal = cvd["signal"].as_f64().unwrap_or(0.0);
        let cvd_direction = cvd["direction"].as_str().unwrap_or("neutral");
        let cvd_buy_sell_ratio = cvd["buy_sell_ratio"].as_f64().unwrap_or(1.0);

        // Kalman filter signals (mathematically optimal state estimation)
        let kalman = &data["kalman"];
        let kalman_momentum = kalman["momentum"].as_f64().unwrap_or(0.0);
        let kalman_trend_strength = kalman["trend_strength"].as_f64().unwrap_or(0.0);
        let kalman_confidence = kalman["confidence"].as_f64().unwrap_or(0.0);
        let kalman_strong_trend = kalman["strong_trend"].as_bool().unwrap_or(false);
        let kalman_momentum_building = kalman["momentum_building"].as_bool().unwrap_or(false);
        let kalman_momentum_fading = kalman["momentum_fading"].as_bool().unwrap_or(false);
        let kalman_direction = kalman["direction"].as_str().unwrap_or("neutral");

        // Update Kronos daily bias with EMA smoothing (alpha=0.05)
        // This smooths out the 8-second oscillations into a stable daily view
        if kronos_active {
            let prev_ema = *self.kronos_bias_ema.get(symbol).unwrap_or(&0.0);
            let alpha = 0.05; // Very slow EMA — takes ~20 readings to shift
            let new_ema = alpha * kronos_direction + (1.0 - alpha) * prev_ema;
            self.kronos_bias_ema.insert(symbol.to_string(), new_ema);
            self.kronos_daily_bias.insert(symbol.to_string(), new_ema);
        }

        // Parse ML agent scores from sidecar (if available)
        let ml_agent_scores = data.get("agent_scores")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(|scores| {
                let age = data["agent_age_seconds"].as_f64().unwrap_or(999.0);
                if age > 10.0 { return None; } // Stale scores
                let trained = scores["is_trained"].as_bool().unwrap_or(false);
                Some(CachedAgentScores {
                    momentum: scores["momentum"].as_f64().unwrap_or(0.0),
                    pattern: scores["pattern"].as_f64().unwrap_or(0.0),
                    flow: scores["flow"].as_f64().unwrap_or(0.0),
                    level: scores["level"].as_f64().unwrap_or(0.0),
                    sentiment: scores["sentiment"].as_f64().unwrap_or(0.0),
                    meta_score: scores["meta_score"].as_f64().unwrap_or(0.0),
                    is_trained: trained,
                    age_seconds: age,
                })
            });

        // Track session high/low
        let prev_high = self.market_data.get(symbol).map(|d| d.session_high).unwrap_or(price);
        let prev_low = self.market_data.get(symbol).map(|d| d.session_low).unwrap_or(price);

        self.market_data.insert(symbol.to_string(), MarketSnapshot {
            price,
            pattern_signal,
            pattern_confidence,
            micro_momentum: micro_mom,
            trend,
            kronos_active,
            kronos_direction,
            kalman_momentum,
            kalman_trend_strength,
            kalman_confidence,
            kalman_momentum_building,
            kalman_momentum_fading,
            kalman_direction: kalman_direction.to_string(),
            cvd_signal,
            cvd_buy_sell_ratio,
            gex_signal: self.market_data.get(symbol).map(|d| d.gex_signal).unwrap_or(0.0),
            gex_regime: self.market_data.get(symbol).map(|d| d.gex_regime.clone()).unwrap_or("neutral".to_string()),
            vp_signal: self.market_data.get(symbol).map(|d| d.vp_signal).unwrap_or(0.0),
            vp_position: self.market_data.get(symbol).map(|d| d.vp_position.clone()).unwrap_or("unknown".to_string()),
            cot_signal: self.market_data.get(symbol).map(|d| d.cot_signal).unwrap_or(0.0),
            session_high: prev_high.max(price),
            session_low: if prev_low == 0.0 { price } else { prev_low.min(price) },
            ml_agent_scores,
        });

        // Signal history for momentum confirmation
        let hist = self.signal_history
            .entry(symbol.to_string())
            .or_insert_with(|| VecDeque::with_capacity(20));
        hist.push_back(pattern_signal);
        if hist.len() > 15 { hist.pop_front(); }

        // Update position
        if let Some(pos) = self.positions.get_mut(symbol) {
            pos.update(price);
        }

        if !self.market_open { return; }

        // EOD Liquidation: close all positions at 3:55 PM ET (5 min before close)
        // Prevents holding overnight risk with a $100 account
        let utc_now = chrono::Utc::now();
        let month = utc_now.month();
        let offset_hours: i64 = if month >= 3 && month <= 10 { 4 } else { 5 };
        let et_hour = (utc_now.hour() as i64 - offset_hours).rem_euclid(24) as u32;
        let et_minute = utc_now.minute();
        let et_mins = et_hour * 60 + et_minute;
        let eod_liquidation = et_mins >= 15 * 60 + 55; // 3:55 PM ET

        if eod_liquidation && self.positions.contains_key(symbol) {
            info!("[EOD_CLOSE] {} — liquidating before market close ({}:{})", symbol, et_hour, et_minute);
            self.sell(symbol, "EOD_LIQUIDATION(3:55pm)");
            return;
        }

        self.manage_position(symbol);

        // Don't open new positions in last 10 minutes
        if et_mins >= 15 * 60 + 50 { return; }
        self.find_best_entry();
    }

    fn manage_position(&mut self, symbol: &str) {
        let should_sell = {
            if let Some(pos) = self.positions.get(symbol) {
                let pnl_pct = pos.unrealized_pnl_pct();
                let drawdown = pos.trailing_drawdown_pct();
                let data = self.market_data.get(symbol);
                let signal = data.map(|d| d.pattern_signal).unwrap_or(0.0);
                let k_fading = data.map(|d| d.kalman_momentum_fading).unwrap_or(false);
                let k_momentum = data.map(|d| d.kalman_momentum).unwrap_or(0.0);
                let cvd_sig = data.map(|d| d.cvd_signal).unwrap_or(0.0);
                let cvd_ratio = data.map(|d| d.cvd_buy_sell_ratio).unwrap_or(1.0);
                let gex_regime = data.map(|d| d.gex_regime.clone()).unwrap_or("neutral".to_string());
                let vp_pos = data.map(|d| d.vp_position.clone()).unwrap_or("unknown".to_string());
                let vp_sig = data.map(|d| d.vp_signal).unwrap_or(0.0);

                // Kalman says momentum is dying + pattern confirms
                let momentum_dead = (k_fading && k_momentum.abs() < 0.01)
                    || self.signal_history.get(symbol).map_or(false, |hist| {
                        if hist.len() < 5 { return false; }
                        hist.iter().rev().take(5).all(|&s| s < -0.05)
                    });

                // CVD divergence: sellers dominating while we're long
                let cvd_bearish = cvd_sig < -0.4 && cvd_ratio < 0.7;

                // VP resistance: price pushed above value area (overbought)
                let at_resistance = vp_pos == "above_value" && vp_sig < -0.3;

                // GEX regime: in long_gamma, mean reversion kicks in faster
                let gex_mean_revert = gex_regime == "long_gamma" && pnl_pct > 0.03;

                // Minimum hold time gate: most exits require MIN_HOLD_SECS
                // to prevent micro-flips that generate fees without capturing moves.
                // Only HARD_STOP bypasses this — capital protection is always immediate.
                let held_long_enough = pos.hold_seconds >= MIN_HOLD_SECS;

                // === SWING TRADE EXIT LOGIC ===
                // Priority: protect capital → lock big wins → cut dead trades
                // Goal: 2:1 reward/risk, hold 5-30 min, catch 1-2% moves

                // 1. HARD STOP — protect capital, IMMEDIATE
                if pnl_pct <= HARD_STOP_PCT {
                    Some("HARD_STOP".to_string())
                }
                // 2. TAKE PROFIT — hit our 2% target
                else if held_long_enough && pnl_pct >= TAKE_PROFIT_PCT {
                    Some(format!("TAKE_PROFIT({:.2}%)", pnl_pct))
                }
                // 3. TRAILING STOP — only after 5+ min and we're losing
                else if drawdown <= -TRAILING_STOP_PCT && pos.hold_seconds > 300
                    && pnl_pct < 0.0 {
                    Some(format!("TRAIL_STOP({:.2}% from peak,pnl={:.2}%)", drawdown, pnl_pct))
                }
                // 4. MOMENTUM COLLAPSE — all signals dead + losing money
                // Don't exit winners on momentum death — they may resume
                else if momentum_dead && pos.hold_seconds > 300 && pnl_pct < -0.2 {
                    Some(format!("MOMENTUM_DEAD(sig={:.2},pnl={:.2}%)", signal, pnl_pct))
                }
                // 5. STRONG BEARISH REVERSAL — 3+ layers flipped against us hard
                else if pos.hold_seconds > 240 {
                    let bearish_count = [
                        k_fading,
                        signal < -0.15,
                        cvd_sig < -0.5,
                        vp_sig < -0.5,
                    ].iter().filter(|&&b| b).count();
                    if bearish_count >= 3 && pnl_pct < -0.3 {
                        Some(format!("REVERSAL_EXIT({}bearish,pnl={:.2}%)", bearish_count, pnl_pct))
                    } else { None }
                }
                // 6. FLAT EXIT — going nowhere after 30 min, take whatever we have
                else if pos.hold_seconds >= FLAT_EXIT_SECS {
                    Some(format!("FLAT_EXIT({}s,{:.2}%)", pos.hold_seconds, pnl_pct))
                }
                else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(reason) = should_sell {
            // Log all layer states at exit for monitoring
            if let Some(pos) = self.positions.get(symbol) {
                let data = self.market_data.get(symbol);
                info!("[EXIT_LAYERS] {} reason={} | pnl={:.4}% hold={}s | \
                    kalman(dir={},mom={:.4},fading={}) pat_sig={:.3} cvd={:.2}(ratio={:.2}) \
                    vp({},{:.2}) gex({},{:.2}) cot={:.2}",
                    symbol, reason,
                    pos.unrealized_pnl_pct(), pos.hold_seconds,
                    data.map(|d| d.kalman_direction.as_str()).unwrap_or("?"),
                    data.map(|d| d.kalman_momentum).unwrap_or(0.0),
                    data.map(|d| d.kalman_momentum_fading).unwrap_or(false),
                    data.map(|d| d.pattern_signal).unwrap_or(0.0),
                    data.map(|d| d.cvd_signal).unwrap_or(0.0),
                    data.map(|d| d.cvd_buy_sell_ratio).unwrap_or(1.0),
                    data.map(|d| d.vp_position.as_str()).unwrap_or("?"),
                    data.map(|d| d.vp_signal).unwrap_or(0.0),
                    data.map(|d| d.gex_regime.as_str()).unwrap_or("?"),
                    data.map(|d| d.gex_signal).unwrap_or(0.0),
                    data.map(|d| d.cot_signal).unwrap_or(0.0),
                );
            }
            self.sell(symbol, &reason);
        }
    }

    fn find_best_entry(&mut self) {
        if self.daily_trades >= MAX_DAILY_TRADES { return; }
        if self.positions.len() >= MAX_CONCURRENT_POSITIONS { return; }

        let total_value = self.total_value();
        let available = total_value * MAX_POSITION_PCT - self.invested_value();
        if available < 2.0 { return; }

        let mut candidates: Vec<(String, f64, String, usize)> = Vec::new();

        for (sym, data) in &self.market_data {
            if self.positions.contains_key(sym) { continue; }

            // Cooldown check
            if let Some(cd) = self.cooldowns.get(sym) {
                if cd.elapsed().as_secs() < TRADE_COOLDOWN_SECS { continue; }
            }

            // ══ AGENT-BASED SCORING ══
            // Uses ML sidecar scores when available, falls back to hardcoded rules
            let kronos_bias = *self.kronos_daily_bias.get(sym).unwrap_or(&0.0);

            // ── AGENT 1: Kronos Transformer (always hardcoded — it IS the ML model) ──
            let kronos_score = if !data.kronos_active {
                0.0
            } else if kronos_bias > 0.10 {
                1.0
            } else if kronos_bias > 0.03 {
                0.6
            } else if kronos_bias > -0.03 {
                0.1
            } else if kronos_bias > -0.10 {
                -0.4
            } else {
                -1.0
            };

            // Check if ML agent sidecar has fresh scores
            let use_ml = data.ml_agent_scores.as_ref()
                .map(|s| s.is_trained && s.age_seconds < 10.0)
                .unwrap_or(false);

            let (kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score, score, scoring_source);

            if use_ml {
                // ══ ML AGENT SCORING (trained models from sidecar) ══
                let ml = data.ml_agent_scores.as_ref().unwrap();
                kalman_score = ml.momentum;
                pattern_score = ml.pattern;
                cvd_score = ml.flow;
                vp_score = ml.level;
                // Split sentiment into gex + cot for compatibility
                gex_score = ml.sentiment * 0.6;
                cot_score = ml.sentiment * 0.4;
                // Meta agent already learned optimal weighting — use its score directly
                // but blend with kronos_score since meta doesn't have fresh kronos bias
                score = ml.meta_score * 0.75 + kronos_score * 0.25;
                scoring_source = "ML";
            } else {
                // ══ HARDCODED FALLBACK SCORING ══
                // ── Kalman Filter ──
                let kalman_dir_score = match data.kalman_direction.as_str() {
                    "bullish" => 0.5, "neutral" => 0.0, "bearish" => -0.5, _ => 0.0,
                };
                let kalman_mom_score = if data.kalman_momentum_fading {
                    -0.3
                } else {
                    (data.kalman_momentum.max(-1.0).min(1.0)) * 0.3
                };
                let kalman_conf_bonus = (data.kalman_confidence - 0.5).max(0.0) * 0.4;
                kalman_score = (kalman_dir_score + kalman_mom_score + kalman_conf_bonus)
                    .max(-1.0).min(1.0);

                // ── Pattern Recognition ──
                let hist_score = self.signal_history.get(sym).map_or(0.0, |hist| {
                    if hist.len() < 3 { return -0.2; }
                    let recent: Vec<f64> = hist.iter().rev().take(5).cloned().collect();
                    let pos = recent.iter().filter(|&&s| s > 0.005).count() as f64;
                    let neg = recent.iter().filter(|&&s| s < -0.005).count() as f64;
                    (pos - neg) / recent.len() as f64
                });
                pattern_score = ((data.pattern_signal * 3.0).max(-1.0).min(1.0) * 0.6
                    + hist_score * 0.4).max(-1.0).min(1.0);

                // ── CVD / Order Flow ──
                cvd_score = if data.cvd_signal > 0.5 { 1.0 }
                    else if data.cvd_signal > 0.1 { 0.5 }
                    else if data.cvd_signal > -0.1 { 0.0 }
                    else if data.cvd_signal > -0.3 { -0.4 }
                    else { -0.8 };

                // ── Volume Profile ──
                vp_score = if data.vp_position == "above_value" && data.vp_signal < -0.2 { -0.7 }
                    else if data.vp_position == "below_value" && data.vp_signal > 0.1 { 0.6 }
                    else { data.vp_signal.max(-1.0).min(1.0) * 0.3 };

                // ── GEX ──
                gex_score = if data.gex_regime == "short_gamma" { 0.5 }
                    else if data.gex_regime == "long_gamma" { -0.2 }
                    else { data.gex_signal.max(-1.0).min(1.0) * 0.3 };

                // ── COT ──
                cot_score = data.cot_signal.max(-1.0).min(1.0) * 0.5;

                // ── Weighted sum ──
                let agent_weights: [f64; 7] = [0.25, 0.25, 0.15, 0.15, 0.08, 0.06, 0.06];
                let agent_scores_arr = [kronos_score, kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score];
                score = agent_weights.iter().zip(agent_scores_arr.iter())
                    .map(|(w, s)| w * s).sum();
                scoring_source = "RULES";
            }

            // Agent scores array for reporting
            let agent_weights: [(&str, f64, f64); 7] = [
                ("Kronos",  0.25, kronos_score),
                ("Kalman",  0.25, kalman_score),
                ("Pattern", 0.15, pattern_score),
                ("CVD",     0.15, cvd_score),
                ("VP",      0.08, vp_score),
                ("GEX",     0.06, gex_score),
                ("COT",     0.06, cot_score),
            ];

            let bullish_agents: Vec<&str> = agent_weights.iter()
                .filter(|(_, _, s)| *s > 0.1).map(|(n, _, _)| *n).collect();
            let bearish_agents: Vec<&str> = agent_weights.iter()
                .filter(|(_, _, s)| *s < -0.1).map(|(n, _, _)| *n).collect();

            let agent_report = format!(
                "[{}] score={:.3} | K={:.2} Mo={:.2} Pa={:.2} Fl={:.2} Lv={:.2} Gx={:.2} Co={:.2} | \
                 bull=[{}] bear=[{}] | kbias={:.3}%",
                scoring_source, score,
                kronos_score, kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score,
                bullish_agents.join("+"), bearish_agents.join("+"),
                kronos_bias,
            );

            // ── HARD VETO 1: Kronos must not be bearish ──
            // Analysis showed system was entering with K- (Kronos bearish) and losing.
            // Kronos is the best predictor — never trade against it.
            if kronos_score < -0.1 {
                info!("[KRONOS_VETO] {} — kronos={:.2} bearish, refusing entry: {}", sym, kronos_score, agent_report);
                self.layer_blocks.kronos_bias += 1;
                continue;
            }

            // ── HARD VETO 2: majority bearish ──
            if bearish_agents.len() >= 4 {
                info!("[VETOED] {} — {}/7 agents bearish: {}", sym, bearish_agents.len(), agent_report);
                self.layer_blocks.consensus += 1;
                continue;
            }

            // ── HARD VETO 3: pattern strongly against ──
            if pattern_score < -0.25 {
                info!("[PATTERN_VETO] {} — pattern={:.2} too bearish: {}", sym, pattern_score, agent_report);
                self.layer_blocks.consensus += 1;
                continue;
            }

            // ── MINIMUM SCORE THRESHOLD ──
            if score <= MIN_BUY_SIGNAL {
                // Only log occasionally to reduce noise (was flooding logs)
                if self.layer_blocks.score_too_low % 50 == 0 {
                    info!("[WEAK] {} — {} (need > {:.2})", sym, agent_report, MIN_BUY_SIGNAL);
                }
                self.layer_blocks.score_too_low += 1;
                continue;
            }
            self.layer_blocks.total_passed += 1;

            let bullish_count = bullish_agents.len();
            let layer_report = agent_report;

            candidates.push((sym.clone(), score, layer_report, bullish_count));
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        if let Some((best_sym, score, layer_report, bullish_count)) = candidates.first() {
            let price = self.market_data[best_sym].price;
            let bias = *self.kronos_daily_bias.get(best_sym.as_str()).unwrap_or(&0.0);

            // Swing mode: always go all-in. With 1 max position and high entry
            // bar, every entry is high conviction — no reason to hold back.
            let alloc = available;

            let shares = alloc / price;
            if shares * price >= 1.0 {
                let bias_tag = if bias > 0.05 { "K+" } else if bias > -0.02 { "K~" } else { "K-" };
                info!(
                    "PAPER: BUY {:.4} {} @ ${:.2} = ${:.2} (score={:.3}, bias={:.3}% [{}])",
                    shares, best_sym, price, shares * price, score, bias, bias_tag
                );
                info!("  LAYERS: {}", layer_report);

                let pos = Position::new(best_sym.clone(), shares, price,
                    Local::now().format("%H:%M:%S").to_string());
                self.positions.insert(best_sym.clone(), pos);
                self.cash -= shares * price;

                let trade = Trade {
                    symbol: best_sym.clone(),
                    action: "BUY".into(),
                    shares,
                    price,
                    value: shares * price,
                    pnl: None,
                    pnl_pct: None,
                    reason: format!("ENTRY(s={:.3},{},{}of7)", score, bias_tag, bullish_count),
                    time: Local::now().format("%H:%M:%S").to_string(),
                    hold_seconds: None,
                };
                self.total_trades += 1;
                self.daily_trades += 1;
                if self.trades.len() >= 500 { self.trades.pop_front(); }
                self.trades.push_back(trade);
            }
        }
    }

    fn sell(&mut self, symbol: &str, reason: &str) {
        let pos = match self.positions.remove(symbol) {
            Some(p) => p,
            None => return,
        };

        let value = pos.market_value();
        let pnl = pos.unrealized_pnl();
        let pnl_pct = pos.unrealized_pnl_pct();

        self.cash += value;
        self.realized_pnl += pnl;
        if pnl > 0.0 { self.winning_trades += 1; }

        let pnl_tag = if pnl >= 0.0 { "WIN" } else { "LOSS" };
        info!(
            "PAPER: SELL {:.4} {} @ ${:.2} PnL: ${:.4} ({:.3}%) [{}] held {}s — {}",
            pos.shares, symbol, pos.current_price, pnl, pnl_pct,
            pnl_tag, pos.hold_seconds, reason
        );

        // Log trade to ML agent sidecar for training data collection
        if let Some(data) = self.market_data.get(symbol) {
            let features = json!({
                "momentum": [data.kalman_momentum, data.kalman_trend_strength,
                    data.kalman_confidence, data.micro_momentum,
                    if data.kalman_momentum_building { 1.0 } else { 0.0 },
                    if data.kalman_momentum_fading { 1.0 } else { 0.0 }],
                "pattern": [data.pattern_signal, data.pattern_confidence,
                    data.trend, data.kronos_direction,
                    data.session_high, data.session_low,
                    data.price, data.micro_momentum, 0.0, 0.0],
                "flow": [data.cvd_signal, data.cvd_buy_sell_ratio,
                    data.kalman_momentum],
                "level": [data.vp_signal, data.gex_signal, data.cot_signal],
                "sentiment": [data.gex_signal, data.cot_signal, data.vp_signal],
            });
            let log_body = json!({
                "symbol": symbol,
                "action": "SELL",
                "price": pos.current_price,
                "pnl": pnl,
                "pnl_pct": pnl_pct,
                "hold_seconds": pos.hold_seconds,
                "reason": reason,
                "features": features,
            });
            tokio::spawn(async move {
                let _ = reqwest::Client::new()
                    .post("http://finetune-sidecar:8002/log/trade")
                    .json(&log_body)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
            });
        }

        let time = Local::now().format("%H:%M:%S").to_string();
        let trade = Trade {
            symbol: symbol.to_string(),
            action: "SELL".into(),
            shares: pos.shares,
            price: pos.current_price,
            value,
            pnl: Some(pnl),
            pnl_pct: Some(pnl_pct),
            reason: reason.to_string(),
            time,
            hold_seconds: Some(pos.hold_seconds),
        };
        if self.trades.len() >= 500 { self.trades.pop_front(); }
        self.trades.push_back(trade);
        self.cooldowns.insert(symbol.to_string(), Instant::now());
    }

    fn total_value(&self) -> f64 {
        self.cash + self.invested_value()
    }

    fn invested_value(&self) -> f64 {
        self.positions.values().map(|p| p.market_value()).sum()
    }

    pub fn build_payload(&self) -> serde_json::Value {
        let total = self.total_value();
        let total_pnl = total - INITIAL_CASH;
        let total_pnl_pct = total_pnl / INITIAL_CASH * 100.0;
        let target_value = INITIAL_CASH + 100.0;
        let progress = ((total - INITIAL_CASH) / (target_value - INITIAL_CASH) * 100.0).clamp(0.0, 100.0);

        let win_rate = if self.total_trades > 0 {
            self.winning_trades as f64 / self.total_trades as f64 * 100.0
        } else { 0.0 };

        let drawdown_pct = if total < INITIAL_CASH {
            (total - INITIAL_CASH) / INITIAL_CASH * 100.0
        } else { 0.0 };

        let sell_trades: Vec<&Trade> = self.trades.iter().filter(|t| t.action == "SELL").collect();
        let avg_hold = if !sell_trades.is_empty() {
            sell_trades.iter().filter_map(|t| t.hold_seconds).sum::<u64>() / sell_trades.len() as u64
        } else { 0 };

        let avg_pnl = if !sell_trades.is_empty() {
            sell_trades.iter().filter_map(|t| t.pnl).sum::<f64>() / sell_trades.len() as f64
        } else { 0.0 };

        let positions: Vec<serde_json::Value> = self.positions.values().map(|p| json!({
            "symbol": p.symbol, "shares": p.shares, "entry_price": p.entry_price,
            "current_price": p.current_price,
            "market_value": (p.market_value() * 100.0).round() / 100.0,
            "unrealized_pnl": (p.unrealized_pnl() * 10000.0).round() / 10000.0,
            "unrealized_pnl_pct": (p.unrealized_pnl_pct() * 100.0).round() / 100.0,
            "hold_seconds": p.hold_seconds,
        })).collect();

        let recent_trades: Vec<serde_json::Value> = self.trades.iter().rev().take(20).map(|t| json!({
            "symbol": t.symbol, "action": t.action, "shares": t.shares, "price": t.price,
            "total": (t.value * 100.0).round() / 100.0,
            "pnl": t.pnl.map(|v| (v * 10000.0).round() / 10000.0),
            "pnl_pct": t.pnl_pct.map(|v| (v * 100.0).round() / 100.0),
            "reason": t.reason, "time": t.time,
        })).collect();

        let symbols: Vec<serde_json::Value> = TOP_SYMBOLS.iter().map(|&sym| {
            let data = self.market_data.get(sym);
            let bias = self.kronos_daily_bias.get(sym).unwrap_or(&0.0);
            let in_position = self.positions.contains_key(sym);
            let position_pnl = self.positions.get(sym).map(|p| p.unrealized_pnl()).unwrap_or(0.0);
            json!({
                "symbol": sym,
                "price": data.map(|d| d.price).unwrap_or(0.0),
                "direction": data.map(|d| {
                    if d.pattern_signal > 0.1 { "bullish" }
                    else if d.pattern_signal < -0.1 { "bearish" }
                    else { "neutral" }
                }).unwrap_or("neutral"),
                "signal": data.map(|d| d.pattern_signal).unwrap_or(0.0),
                "micro_momentum": data.map(|d| d.micro_momentum).unwrap_or(0.0),
                "kronos_bias": bias,
                "in_position": in_position,
                "position_pnl": (position_pnl * 10000.0).round() / 10000.0,
            })
        }).collect();

        let value_history: Vec<f64> = {
            let mut hist = vec![INITIAL_CASH];
            let mut running = INITIAL_CASH;
            for t in self.trades.iter() {
                if let Some(pnl) = t.pnl { running += pnl; hist.push(running); }
            }
            hist.push(total);
            if hist.len() > 60 { hist.split_off(hist.len() - 60) } else { hist }
        };

        let lb = &self.layer_blocks;
        let total_blocked = lb.consensus + lb.score_too_low;
        let total_evaluated = lb.total_passed + total_blocked;
        let filter_rate = if total_evaluated > 0 {
            total_blocked as f64 / total_evaluated as f64 * 100.0
        } else { 0.0 };

        json!({
            "market_open": self.market_open,
            "portfolio": {
                "cash": (self.cash * 100.0).round() / 100.0,
                "positions_value": (self.invested_value() * 100.0).round() / 100.0,
                "total_value": (total * 100.0).round() / 100.0,
                "total_pnl": (total_pnl * 10000.0).round() / 10000.0,
                "total_pnl_pct": (total_pnl_pct * 100.0).round() / 100.0,
                "realized_pnl": (self.realized_pnl * 10000.0).round() / 10000.0,
                "target_pct": 50.0, "target_value": target_value,
                "progress_pct": (progress * 100.0).round() / 100.0,
                "drawdown_pct": (drawdown_pct * 100.0).round() / 100.0,
            },
            "positions": positions, "recent_trades": recent_trades,
            "stats": {
                "total_trades": self.total_trades, "winning_trades": self.winning_trades,
                "win_rate": (win_rate * 100.0).round() / 100.0,
                "avg_hold_seconds": avg_hold,
                "avg_pnl_per_trade": (avg_pnl * 10000.0).round() / 10000.0,
                "daily_trades": self.daily_trades, "daily_trade_limit": MAX_DAILY_TRADES,
                "open_positions": self.positions.len(),
                "uptime_seconds": self.start_time.elapsed().as_secs(),
            },
            "symbols": symbols, "value_history": value_history,
            "agent_monitor": {
                "architecture": "ml_ensemble_with_fallback",
                "agents": ["Kronos(25%)", "Momentum(ML)", "Pattern(ML)", "Flow(ML)", "Level(ML)", "Sentiment(ML)", "Meta(ML)"],
                "ml_active": self.market_data.values().any(|d|
                    d.ml_agent_scores.as_ref().map(|s| s.is_trained).unwrap_or(false)
                ),
                "vetoed": lb.consensus,
                "score_too_low": lb.score_too_low,
                "total_passed": lb.total_passed,
                "total_evaluated": total_evaluated,
                "filter_rate_pct": (filter_rate * 10.0).round() / 10.0,
            },
        })
    }
}
