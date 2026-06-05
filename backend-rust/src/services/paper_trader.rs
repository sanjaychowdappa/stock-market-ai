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
    session_high: f64,
    session_low: f64,
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
            session_high: prev_high.max(price),
            session_low: if prev_low == 0.0 { price } else { prev_low.min(price) },
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

        self.manage_position(symbol);
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
                let k_dir_str = data.map(|d| d.kalman_direction.clone()).unwrap_or("neutral".to_string());

                // Kalman says momentum is dying + pattern confirms
                let momentum_dead = (k_fading && k_momentum.abs() < 0.01)
                    || self.signal_history.get(symbol).map_or(false, |hist| {
                        if hist.len() < 5 { return false; }
                        hist.iter().rev().take(5).all(|&s| s < -0.05)
                    });

                // 1. HARD STOP — protect capital, immediate
                if pnl_pct <= HARD_STOP_PCT {
                    Some("HARD_STOP".to_string())
                }
                // 2. TAKE PROFIT — lock gains
                else if pnl_pct >= TAKE_PROFIT_PCT {
                    Some(format!("TAKE_PROFIT({:.2}%)", pnl_pct))
                }
                // 3. TRAILING STOP — price fell from peak, held long enough
                else if drawdown <= -TRAILING_STOP_PCT && pos.hold_seconds > 45 {
                    Some(format!("TRAIL_STOP({:.2}% from peak)", drawdown))
                }
                // 4. MOMENTUM EXHAUSTION — pattern signals all turned negative
                else if momentum_dead && pos.hold_seconds > 30 {
                    Some(format!("MOMENTUM_DEAD(sig={:.2},kmom={:.2})", signal, k_momentum))
                }
                // 5. PROFIT PROTECTION — was profitable, now giving it back
                else if pos.hold_seconds > 60 && pnl_pct < -0.02 && pos.high_price > pos.entry_price * 1.0005 {
                    Some(format!("PROFIT_PROTECT(was+,now{:.2}%)", pnl_pct))
                }
                // 6. FLAT EXIT — going nowhere
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
            self.sell(symbol, &reason);
        }
    }

    fn find_best_entry(&mut self) {
        if self.daily_trades >= MAX_DAILY_TRADES { return; }
        if self.positions.len() >= MAX_CONCURRENT_POSITIONS { return; }

        let total_value = self.total_value();
        let available = total_value * MAX_POSITION_PCT - self.invested_value();
        if available < 2.0 { return; }

        let mut candidates: Vec<(String, f64)> = Vec::new();

        for (sym, data) in &self.market_data {
            if self.positions.contains_key(sym) { continue; }

            // Cooldown check
            if let Some(cd) = self.cooldowns.get(sym) {
                if cd.elapsed().as_secs() < TRADE_COOLDOWN_SECS { continue; }
            }

            // ── KRONOS DAILY FOCUS FILTER ──
            // Skip stocks that Kronos ranked as bearish for today
            // The daily_stock_picker runs Kronos once at market open
            // and updates the kronos_daily_bias EMA
            if data.kronos_active {
                let bias = *self.kronos_daily_bias.get(sym).unwrap_or(&0.0);
                if bias < -0.02 {
                    continue; // Kronos says skip this stock today
                }
            }

            // ── KALMAN + PATTERN DRIVEN ENTRY ──

            // 1. Kalman filter must show bullish direction with confidence
            if data.kalman_direction != "bullish" || data.kalman_confidence < 0.3 { continue; }

            // 2. Pattern signal must be positive
            if data.pattern_signal < 0.01 { continue; }

            // 3. Momentum must be building (not fading)
            if data.kalman_momentum_fading { continue; }

            // 4. Pattern history confirmation
            let momentum_confirmed = self.signal_history.get(sym).map_or(false, |hist| {
                if hist.len() < 5 { return false; }
                let pos_count = hist.iter().rev().take(5).filter(|&&s| s > 0.005).count();
                pos_count >= 3
            });
            if !momentum_confirmed { continue; }

            // 5. Composite score — Kalman + pattern + momentum
            let score = 0.30 * data.kalman_momentum.max(0.0).min(1.0) // Kalman true momentum
                + 0.25 * data.pattern_signal                           // Pattern strength
                + 0.20 * data.kalman_trend_strength.min(3.0) / 3.0    // Signal-to-noise ratio
                + 0.15 * data.micro_momentum.max(0.0)                 // Pattern momentum
                + 0.10 * data.kalman_confidence;                       // Kalman confidence

            if score > MIN_BUY_SIGNAL {
                candidates.push((sym.clone(), score));
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        if let Some((best_sym, score)) = candidates.first() {
            let price = self.market_data[best_sym].price;
            let bias = *self.kronos_daily_bias.get(best_sym.as_str()).unwrap_or(&0.0);

            // Position sizing: bigger when Kronos bias supports the trade
            let alloc = if bias > 0.05 && *score > STRONG_BUY_SIGNAL {
                available  // Full size: strong pattern + Kronos bullish
            } else if *score > STRONG_BUY_SIGNAL {
                available * 0.85
            } else {
                available * 0.65
            };

            let shares = alloc / price;
            if shares * price >= 1.0 {
                let bias_tag = if bias > 0.05 { "K+" } else if bias > -0.02 { "K~" } else { "K-" };
                info!(
                    "PAPER: BUY {:.4} {} @ ${:.2} = ${:.2} (score={:.3}, bias={:.3}% [{}])",
                    shares, best_sym, price, shares * price, score, bias, bias_tag
                );

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
                    reason: format!("PATTERN(s={:.3},{})", score, bias_tag),
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
        let target_value = INITIAL_CASH * 1.5;
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
        })
    }
}
