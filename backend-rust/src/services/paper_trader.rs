//! Smart momentum paper trader — $100 → $150 challenge.
//!
//! Optimized for real-world profitability:
//! - Fewer, higher-conviction trades (30-80/day vs 2000+)
//! - Longer hold times (60-300s vs 3s) to capture real moves
//! - Wider stops to survive noise and let winners run
//! - Spread-aware: only enters when expected move > spread cost
//! - Daily trade cap to limit transaction costs

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
    // Signal history for confirmation (multiple ticks agreeing)
    signal_history: HashMap<String, VecDeque<f64>>,
    // Latest market data per symbol
    market_data: HashMap<String, MarketSnapshot>,
    /// Whether the market is currently open (updated each tick)
    pub market_open: bool,
    // Broadcast
    pub tx: broadcast::Sender<serde_json::Value>,
    pub last_payload: Option<serde_json::Value>,
}

struct MarketSnapshot {
    price: f64,
    pred_5s: f64,
    pred_30s: f64,
    pred_60s: f64,
    pattern_signal: f64,
    micro_momentum: f64,
    trend: f64,
    /// True if Kronos AI is providing real predictions
    kronos_active: bool,
    /// Kronos predicted direction for 60s horizon
    kronos_direction: f64,
    /// Consecutive bearish Kronos readings (for sustained signal detection)
    kronos_bearish_streak: u32,
    /// Kronos conviction at time of entry (snapshot for comparison)
    kronos_entry_direction: f64,
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

        if weekday >= 5 {
            return false;
        }

        let minutes_since_midnight = et_hour * 60 + et_minute;
        let market_open = 9 * 60 + 30;
        let market_close = 16 * 60;

        minutes_since_midnight >= market_open && minutes_since_midnight < market_close
    }

    pub fn subscribe(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tx.subscribe()
    }

    /// Main trading loop — call every second with latest engine data.
    pub fn tick(&mut self, symbol: &str, data: &serde_json::Value) {
        let price = data["current_price"].as_f64().unwrap_or(0.0);
        if price <= 0.0 {
            return;
        }

        self.market_open = Self::is_market_open();

        // Extract prediction data
        let pattern_signal = data["pattern"]["signal"].as_f64().unwrap_or(0.0);
        let predictions = data["predictions"].as_array();

        let mut pred_5s = price;
        let mut pred_30s = price;
        let mut pred_60s = price;
        let mut kronos_active = false;
        if let Some(preds) = predictions {
            for p in preds {
                // Check if Kronos is providing real predictions
                if p["kronos_price"].as_f64().is_some() {
                    kronos_active = true;
                }
                match p["seconds_ahead"].as_u64() {
                    Some(5) => pred_5s = p["predicted_price"].as_f64().unwrap_or(price),
                    Some(30) => pred_30s = p["predicted_price"].as_f64().unwrap_or(price),
                    Some(60) => pred_60s = p["predicted_price"].as_f64().unwrap_or(price),
                    _ => {}
                }
            }
        }

        let micro_mom = data["pattern"]["confidence"].as_f64().unwrap_or(0.0)
            * if data["pattern"]["direction"].as_str() == Some("bullish") { 1.0 } else { -1.0 };

        let trend = (pred_30s - price) / price;
        let kronos_direction = (pred_60s - price) / price * 100.0;

        // Track consecutive bearish Kronos readings
        let prev_streak = self.market_data.get(symbol)
            .map(|d| d.kronos_bearish_streak).unwrap_or(0);
        let prev_entry_dir = self.market_data.get(symbol)
            .map(|d| d.kronos_entry_direction).unwrap_or(0.0);
        let bearish_streak = if kronos_active && kronos_direction < -0.01 {
            prev_streak + 1
        } else {
            0
        };

        // Always update market data (for display even when market closed)
        self.market_data.insert(
            symbol.to_string(),
            MarketSnapshot {
                price,
                pred_5s,
                pred_30s,
                pred_60s,
                pattern_signal,
                micro_momentum: micro_mom,
                trend,
                kronos_active,
                kronos_direction,
                kronos_bearish_streak: bearish_streak,
                kronos_entry_direction: prev_entry_dir,
            },
        );

        // Track signal history for confirmation
        let hist = self.signal_history
            .entry(symbol.to_string())
            .or_insert_with(|| VecDeque::with_capacity(MOMENTUM_WINDOW + 1));
        hist.push_back(pattern_signal);
        if hist.len() > MOMENTUM_WINDOW {
            hist.pop_front();
        }

        // Update position prices if held
        if let Some(pos) = self.positions.get_mut(symbol) {
            pos.update(price);
        }

        // Only execute trading strategy during market hours
        if !self.market_open {
            return;
        }

        // Strategy
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
                let kronos_dir = data.map(|d| d.kronos_direction).unwrap_or(0.0);
                let kronos_on = data.map(|d| d.kronos_active).unwrap_or(false);
                let bearish_streak = data.map(|d| d.kronos_bearish_streak).unwrap_or(0);
                let entry_dir = data.map(|d| d.kronos_entry_direction).unwrap_or(0.0);

                // 1. Hard stop — protect capital (immediate, no waiting)
                if pnl_pct <= HARD_STOP_PCT {
                    Some("HARD_STOP".to_string())
                }
                // 2. Take profit — lock in gains
                else if pnl_pct >= TAKE_PROFIT_PCT {
                    Some(format!("TAKE_PROFIT({:.3}%)", pnl_pct))
                }
                // 3. Kronos SUSTAINED bearish — AI consistently says sell
                //    Need 3+ consecutive bearish readings (24+ seconds at 8s interval)
                //    AND the direction must have fully reversed from entry
                else if kronos_on && bearish_streak >= 3
                    && kronos_dir < -0.03
                    && pos.hold_seconds > 30 {
                    Some(format!("KRONOS_EXIT(dir={:.3}%,streak={})", kronos_dir, bearish_streak))
                }
                // 4. Trailing stop — only after 60s+ hold time
                else if drawdown <= -TRAILING_STOP_PCT && pos.hold_seconds > 60 {
                    Some(format!("TRAIL_STOP({:.3}%)", drawdown))
                }
                // 5. Flat exit — position going nowhere after time limit
                else if pos.hold_seconds >= FLAT_EXIT_SECS {
                    Some(format!("FLAT_EXIT({}s,{:.3}%)", pos.hold_seconds, pnl_pct))
                }
                // 6. Deep pattern bearish — only if very strong reversal AND held 60s+
                else if signal < -0.15 && pos.hold_seconds > 60 {
                    Some(format!("PATTERN_REVERSAL(sig={:.3})", signal))
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
        // Respect daily trade cap
        if self.daily_trades >= MAX_DAILY_TRADES {
            return;
        }

        // Max concurrent positions
        if self.positions.len() >= MAX_CONCURRENT_POSITIONS {
            return;
        }

        let total_value = self.total_value();
        let available = total_value * MAX_POSITION_PCT - self.invested_value();
        if available < 2.0 {
            return;
        }

        // Score each symbol — Kronos-driven when available, pattern fallback otherwise
        let mut candidates: Vec<(String, f64)> = Vec::new();

        for (sym, data) in &self.market_data {
            if self.positions.contains_key(sym) {
                continue;
            }

            // Cooldown check
            if let Some(cd) = self.cooldowns.get(sym) {
                if cd.elapsed().as_secs() < TRADE_COOLDOWN_SECS {
                    continue;
                }
            }

            let score;

            if data.kronos_active {
                // ── KRONOS MODE: AI-driven entry ──
                // Only enter when Kronos is clearly bullish (not marginal)
                if data.kronos_direction < 0.01 {
                    continue; // Kronos says bearish/flat — don't fight the AI
                }

                // Must not be in a bearish streak
                if data.kronos_bearish_streak > 0 {
                    continue; // Wait for clean bullish signal
                }

                // Kronos conviction is the dominant factor
                score = 0.50 * data.kronos_direction.min(0.5)         // Kronos conviction (capped)
                    + 0.20 * data.pattern_signal.max(0.0)             // Pattern confirmation (only positive)
                    + 0.15 * ((data.pred_60s - data.price) / data.price * 100.0).max(0.0) // 60s move
                    + 0.15 * data.micro_momentum.max(0.0);            // Momentum (only positive)
            } else {
                // ── PATTERN-ONLY FALLBACK ──
                // Signal confirmation: recent signal trend must be positive
                let confirmed = self.signal_history.get(sym).map_or(false, |hist| {
                    if hist.len() < 3 { return false; }
                    let positive_count = hist.iter().filter(|&&s| s > 0.005).count();
                    positive_count >= 2
                });
                if !confirmed {
                    continue;
                }

                score = 0.35 * data.pattern_signal
                    + 0.10 * ((data.pred_5s - data.price) / data.price * 100.0)
                    + 0.20 * ((data.pred_30s - data.price) / data.price * 100.0)
                    + 0.20 * data.micro_momentum
                    + 0.15 * data.trend * 100.0;
            }

            if score > MIN_BUY_SIGNAL {
                candidates.push((sym.clone(), score));
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        if let Some((best_sym, score)) = candidates.first() {
            let price = self.market_data[best_sym].price;

            // Position sizing: scale with conviction
            let alloc = if *score > STRONG_BUY_SIGNAL {
                available  // Full allocation for strong signals
            } else {
                available * 0.6  // Partial for moderate signals
            };
            let shares = alloc / price;

            if shares * price >= 1.0 {
                self.buy(&best_sym.clone(), shares, price, *score);
            }
        }
    }

    fn buy(&mut self, symbol: &str, shares: f64, price: f64, score: f64) {
        let value = shares * price;
        if value > self.cash {
            return;
        }

        self.cash -= value;
        let time = Local::now().format("%H:%M:%S").to_string();

        info!(
            "PAPER: BUY {:.4} {} @ ${:.2} = ${:.2} (score={:.3})",
            shares, symbol, price, value, score
        );

        let pos = Position::new(symbol.to_string(), shares, price, time.clone());
        self.positions.insert(symbol.to_string(), pos);

        // Snapshot Kronos conviction at entry for later comparison
        if let Some(snap) = self.market_data.get_mut(symbol) {
            snap.kronos_entry_direction = snap.kronos_direction;
            snap.kronos_bearish_streak = 0; // Reset streak on new entry
        }

        let trade = Trade {
            symbol: symbol.to_string(),
            action: "BUY".into(),
            shares,
            price,
            value,
            pnl: None,
            pnl_pct: None,
            reason: format!("MOMENTUM(s={:.3})", score),
            time,
            hold_seconds: None,
        };
        self.total_trades += 1;
        self.daily_trades += 1;
        if self.trades.len() >= 500 {
            self.trades.pop_front();
        }
        self.trades.push_back(trade);
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
        if pnl > 0.0 {
            self.winning_trades += 1;
        }

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
        if self.trades.len() >= 500 {
            self.trades.pop_front();
        }
        self.trades.push_back(trade);
        self.cooldowns.insert(symbol.to_string(), Instant::now());
    }

    fn total_value(&self) -> f64 {
        self.cash + self.invested_value()
    }

    fn invested_value(&self) -> f64 {
        self.positions.values().map(|p| p.market_value()).sum()
    }

    /// Build the full payload for WebSocket broadcast.
    pub fn build_payload(&self) -> serde_json::Value {
        let total = self.total_value();
        let total_pnl = total - INITIAL_CASH;
        let total_pnl_pct = total_pnl / INITIAL_CASH * 100.0;
        let target_value = INITIAL_CASH * 1.5;
        let progress = ((total - INITIAL_CASH) / (target_value - INITIAL_CASH) * 100.0).clamp(0.0, 100.0);

        let win_rate = if self.total_trades > 0 {
            self.winning_trades as f64 / self.total_trades as f64 * 100.0
        } else {
            0.0
        };

        let drawdown_pct = if total < INITIAL_CASH {
            (total - INITIAL_CASH) / INITIAL_CASH * 100.0
        } else {
            0.0
        };

        let sell_trades: Vec<&Trade> = self.trades.iter().filter(|t| t.action == "SELL").collect();
        let avg_hold = if !sell_trades.is_empty() {
            let total_hold: u64 = sell_trades.iter()
                .filter_map(|t| t.hold_seconds)
                .sum();
            total_hold / sell_trades.len() as u64
        } else {
            0
        };

        // Compute avg profit per trade
        let avg_pnl = if !sell_trades.is_empty() {
            let total_pnl_trades: f64 = sell_trades.iter()
                .filter_map(|t| t.pnl)
                .sum();
            total_pnl_trades / sell_trades.len() as f64
        } else {
            0.0
        };

        let positions: Vec<serde_json::Value> = self
            .positions
            .values()
            .map(|p| {
                json!({
                    "symbol": p.symbol,
                    "shares": p.shares,
                    "entry_price": p.entry_price,
                    "current_price": p.current_price,
                    "market_value": (p.market_value() * 100.0).round() / 100.0,
                    "unrealized_pnl": (p.unrealized_pnl() * 10000.0).round() / 10000.0,
                    "unrealized_pnl_pct": (p.unrealized_pnl_pct() * 100.0).round() / 100.0,
                    "hold_seconds": p.hold_seconds,
                })
            })
            .collect();

        let recent_trades: Vec<serde_json::Value> = self
            .trades
            .iter()
            .rev()
            .take(20)
            .map(|t| {
                json!({
                    "symbol": t.symbol,
                    "action": t.action,
                    "shares": t.shares,
                    "price": t.price,
                    "total": (t.value * 100.0).round() / 100.0,
                    "pnl": t.pnl.map(|v| (v * 10000.0).round() / 10000.0),
                    "pnl_pct": t.pnl_pct.map(|v| (v * 100.0).round() / 100.0),
                    "reason": t.reason,
                    "time": t.time,
                })
            })
            .collect();

        let symbols: Vec<serde_json::Value> = TOP_SYMBOLS
            .iter()
            .map(|&sym| {
                let data = self.market_data.get(sym);
                let in_position = self.positions.contains_key(sym);
                let position_pnl = self.positions.get(sym)
                    .map(|p| p.unrealized_pnl())
                    .unwrap_or(0.0);
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
                    "in_position": in_position,
                    "position_pnl": (position_pnl * 10000.0).round() / 10000.0,
                })
            })
            .collect();

        let value_history: Vec<f64> = {
            let mut hist = vec![INITIAL_CASH];
            let mut running = INITIAL_CASH;
            for t in self.trades.iter() {
                if let Some(pnl) = t.pnl {
                    running += pnl;
                    hist.push(running);
                }
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
                "target_pct": 50.0,
                "target_value": target_value,
                "progress_pct": (progress * 100.0).round() / 100.0,
                "drawdown_pct": (drawdown_pct * 100.0).round() / 100.0,
            },
            "positions": positions,
            "recent_trades": recent_trades,
            "stats": {
                "total_trades": self.total_trades,
                "winning_trades": self.winning_trades,
                "win_rate": (win_rate * 100.0).round() / 100.0,
                "avg_hold_seconds": avg_hold,
                "avg_pnl_per_trade": (avg_pnl * 10000.0).round() / 10000.0,
                "daily_trades": self.daily_trades,
                "daily_trade_limit": MAX_DAILY_TRADES,
                "open_positions": self.positions.len(),
                "uptime_seconds": self.start_time.elapsed().as_secs(),
            },
            "symbols": symbols,
            "value_history": value_history,
        })
    }
}
