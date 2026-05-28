//! Aggressive momentum scalping paper trader — $100 → $150 challenge.
//!
//! Trades fractional shares across TOP_SYMBOLS using Kronos predictions
//! + pattern signals. Tight trailing stops, concentrated positions.

use crate::config::*;
use crate::models::{Position, Trade};
use crate::services::realtime_engine::RealtimeEngine;
use chrono::Local;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{debug, info};

pub struct PaperTrader {
    cash: f64,
    positions: HashMap<String, Position>,
    trades: VecDeque<Trade>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    start_time: Instant,
    cooldowns: HashMap<String, Instant>,
    // Latest market data per symbol
    market_data: HashMap<String, MarketSnapshot>,
    // Broadcast
    pub tx: broadcast::Sender<serde_json::Value>,
    pub last_payload: Option<serde_json::Value>,
}

struct MarketSnapshot {
    price: f64,
    pred_5s: f64,
    pred_30s: f64,
    pattern_signal: f64,
    micro_momentum: f64,
    trend: f64,
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
            start_time: Instant::now(),
            cooldowns: HashMap::new(),
            market_data: HashMap::new(),
            tx,
            last_payload: None,
        }
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

        // Extract prediction data
        let pattern_signal = data["pattern"]["signal"].as_f64().unwrap_or(0.0);
        let predictions = data["predictions"].as_array();

        let mut pred_5s = price;
        let mut pred_30s = price;
        if let Some(preds) = predictions {
            for p in preds {
                match p["seconds_ahead"].as_u64() {
                    Some(5) => pred_5s = p["predicted_price"].as_f64().unwrap_or(price),
                    Some(30) => pred_30s = p["predicted_price"].as_f64().unwrap_or(price),
                    _ => {}
                }
            }
        }

        let micro_mom = data["pattern"]["confidence"].as_f64().unwrap_or(0.0)
            * if data["pattern"]["direction"].as_str() == Some("bullish") { 1.0 } else { -1.0 };

        let trend = (pred_30s - price) / price;

        self.market_data.insert(
            symbol.to_string(),
            MarketSnapshot {
                price,
                pred_5s,
                pred_30s,
                pattern_signal,
                micro_momentum: micro_mom,
                trend,
            },
        );

        // Update position if held
        if let Some(pos) = self.positions.get_mut(symbol) {
            pos.update(price);
        }

        // Strategy
        self.manage_position(symbol, price);
        self.find_best_entry();
    }

    fn manage_position(&mut self, symbol: &str, price: f64) {
        let should_sell = {
            if let Some(pos) = self.positions.get(symbol) {
                let pnl_pct = pos.unrealized_pnl_pct();
                let drawdown = pos.trailing_drawdown_pct();

                // Hard stop
                if pnl_pct <= HARD_STOP_PCT {
                    Some("HARD_STOP".to_string())
                }
                // Trailing stop
                else if drawdown <= -TRAILING_STOP_PCT && pos.hold_seconds > 5 {
                    Some("TRAIL_STOP".to_string())
                }
                // Take profit
                else if pnl_pct >= TAKE_PROFIT_PCT {
                    Some("TAKE_PROFIT".to_string())
                }
                // Flat exit
                else if pos.hold_seconds >= FLAT_EXIT_SECS {
                    Some("FLAT_EXIT".to_string())
                }
                // Bearish flip
                else if self.market_data.get(symbol).map(|d| d.pattern_signal).unwrap_or(0.0) < SELL_SIGNAL_THRESHOLD {
                    Some("SIGNAL_FLIP".to_string())
                } else {
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
        if self.positions.len() >= 5 {
            return; // Max 5 concurrent
        }

        let total_value = self.total_value();
        let available = total_value * MAX_POSITION_PCT - self.invested_value();
        if available < 1.0 {
            return;
        }

        // Score each symbol
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

            // Composite buy score (same weights as Python)
            let score = 0.30 * data.pattern_signal
                + 0.15 * ((data.pred_5s - data.price) / data.price * 100.0)
                + 0.15 * ((data.pred_30s - data.price) / data.price * 100.0)
                + 0.25 * data.micro_momentum
                + 0.15 * data.trend * 100.0;

            if score > MIN_BUY_SIGNAL {
                candidates.push((sym.clone(), score));
            }
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        if let Some((best_sym, score)) = candidates.first() {
            let price = self.market_data[best_sym].price;
            let alloc = if *score > STRONG_BUY_SIGNAL {
                available
            } else {
                available * 0.5
            };
            let shares = alloc / price;

            if shares * price >= 0.50 {
                self.buy(best_sym, shares, price, *score);
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
            "PAPER: BUY {:.4} {} @ ${:.2} = ${:.2} (score={:.2})",
            shares, symbol, price, value, score
        );

        let pos = Position::new(symbol.to_string(), shares, price, time.clone());
        self.positions.insert(symbol.to_string(), pos);

        let trade = Trade {
            symbol: symbol.to_string(),
            action: "BUY".into(),
            shares,
            price,
            value,
            pnl: None,
            pnl_pct: None,
            reason: format!("MOMENTUM(s={:.2})", score),
            time,
            hold_seconds: None,
        };
        self.total_trades += 1;
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

        info!(
            "PAPER: SELL {:.4} {} @ ${:.2} PnL: ${:.4} ({}) held {}s",
            pos.shares, symbol, pos.current_price, pnl, reason, pos.hold_seconds
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

        // Drawdown: how far below peak
        let drawdown_pct = if total < INITIAL_CASH {
            (total - INITIAL_CASH) / INITIAL_CASH * 100.0
        } else {
            0.0
        };

        // Average hold time from recent sell trades
        let sell_trades: Vec<&Trade> = self.trades.iter().filter(|t| t.action == "SELL").collect();
        let avg_hold = if !sell_trades.is_empty() {
            let total_hold: u64 = sell_trades.iter()
                .filter_map(|t| t.hold_seconds)
                .sum();
            total_hold / sell_trades.len() as u64
        } else {
            0
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

        // Symbol grid: per-symbol status with market data
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

        // Value history (last 60 entries from trades)
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
            // Keep last 60
            if hist.len() > 60 { hist.split_off(hist.len() - 60) } else { hist }
        };

        json!({
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
                "open_positions": self.positions.len(),
                "uptime_seconds": self.start_time.elapsed().as_secs(),
            },
            "symbols": symbols,
            "value_history": value_history,
        })
    }
}
