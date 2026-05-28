//! Daily prediction tracker + auto fine-tune trigger.
//!
//! Collects every prediction and actual price throughout the day,
//! saves data periodically, and at EOD sends candle data to the
//! Python fine-tune sidecar if the day was profitable.

use crate::config::*;
use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tracing::{error, info, warn};

const LOG_DIR: &str = "/app/model_cache/daily_logs";
const FINETUNE_URL: &str = "http://finetune-sidecar:8001/finetune";

#[derive(Debug, Clone, Serialize)]
struct TickEntry {
    ts: f64,
    price: f64,
}

#[derive(Debug, Clone, Serialize)]
struct PredEntry {
    ts: f64,
    current_price: f64,
    pred_5s: f64,
    pred_10s: f64,
    pred_30s: f64,
    pattern_signal: f64,
}

#[derive(Debug)]
struct DailyRecord {
    symbol: String,
    ticks: Vec<TickEntry>,
    predictions: Vec<PredEntry>,
    trades: Vec<serde_json::Value>,
}

impl DailyRecord {
    fn new(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_string(),
            ticks: Vec::with_capacity(10000),
            predictions: Vec::with_capacity(10000),
            trades: Vec::new(),
        }
    }

    fn add_tick(&mut self, ts: f64, price: f64) {
        self.ticks.push(TickEntry { ts, price });
    }

    fn add_prediction(&mut self, data: &serde_json::Value) {
        let ts = data["timestamp"].as_f64().unwrap_or(0.0);
        let current = data["current_price"].as_f64().unwrap_or(0.0);
        let mut pred_5s = 0.0;
        let mut pred_10s = 0.0;
        let mut pred_30s = 0.0;
        if let Some(preds) = data["predictions"].as_array() {
            for p in preds {
                match p["seconds_ahead"].as_u64() {
                    Some(5) => pred_5s = p["predicted_price"].as_f64().unwrap_or(0.0),
                    Some(10) => pred_10s = p["predicted_price"].as_f64().unwrap_or(0.0),
                    Some(30) => pred_30s = p["predicted_price"].as_f64().unwrap_or(0.0),
                    _ => {}
                }
            }
        }
        let signal = data["pattern"]["signal"].as_f64().unwrap_or(0.0);
        self.predictions.push(PredEntry {
            ts,
            current_price: current,
            pred_5s,
            pred_10s,
            pred_30s,
            pattern_signal: signal,
        });
    }

    /// Build 1-minute candles from ticks.
    fn build_1min_candles(&self) -> Vec<serde_json::Value> {
        if self.ticks.len() < 60 {
            return Vec::new();
        }

        let mut candles = Vec::new();
        let mut open = 0.0;
        let mut high = f64::NEG_INFINITY;
        let mut low = f64::INFINITY;
        let mut close = 0.0;
        let mut minute_start = 0.0;
        let mut count = 0u32;

        for tick in &self.ticks {
            let minute = (tick.ts / 60.0).floor() * 60.0;
            if count == 0 {
                minute_start = minute;
                open = tick.price;
                high = tick.price;
                low = tick.price;
                close = tick.price;
                count = 1;
                continue;
            }
            if minute != minute_start {
                candles.push(json!({
                    "Open": open, "High": high, "Low": low, "Close": close,
                    "Volume": count, "timestamp": minute_start,
                }));
                minute_start = minute;
                open = tick.price;
                high = tick.price;
                low = tick.price;
                close = tick.price;
                count = 1;
            } else {
                high = high.max(tick.price);
                low = low.min(tick.price);
                close = tick.price;
                count += 1;
            }
        }
        if count > 0 {
            candles.push(json!({
                "Open": open, "High": high, "Low": low, "Close": close,
                "Volume": count, "timestamp": minute_start,
            }));
        }
        candles
    }

    fn compute_accuracy(&self) -> serde_json::Value {
        if self.predictions.len() < 30 || self.ticks.len() < 30 {
            return json!({"error": "insufficient data"});
        }

        // Build price lookup
        let prices: HashMap<i64, f64> = self
            .ticks
            .iter()
            .map(|t| (t.ts as i64, t.price))
            .collect();

        let mut correct_5s = 0u32;
        let mut total_5s = 0u32;
        let mut correct_10s = 0u32;
        let mut total_10s = 0u32;

        for pred in &self.predictions {
            if pred.current_price <= 0.0 {
                continue;
            }
            for (secs, predicted) in [(5, pred.pred_5s), (10, pred.pred_10s)] {
                if predicted <= 0.0 {
                    continue;
                }
                let target_ts = pred.ts as i64 + secs;
                // Find closest tick within 1.5s
                let actual = (-1..=1)
                    .filter_map(|d| prices.get(&(target_ts + d)).copied())
                    .next();

                if let Some(actual) = actual {
                    let pred_dir = predicted > pred.current_price;
                    let actual_dir = actual > pred.current_price;
                    if secs == 5 {
                        total_5s += 1;
                        if pred_dir == actual_dir { correct_5s += 1; }
                    } else {
                        total_10s += 1;
                        if pred_dir == actual_dir { correct_10s += 1; }
                    }
                }
            }
        }

        json!({
            "total_ticks": self.ticks.len(),
            "total_predictions": self.predictions.len(),
            "direction_accuracy_5s": if total_5s > 0 { correct_5s as f64 / total_5s as f64 * 100.0 } else { 0.0 },
            "direction_accuracy_10s": if total_10s > 0 { correct_10s as f64 / total_10s as f64 * 100.0 } else { 0.0 },
            "samples_5s": total_5s,
            "samples_10s": total_10s,
        })
    }
}

pub struct DailyTracker {
    today: String,
    records: HashMap<String, DailyRecord>,
    portfolio_snapshots: Vec<serde_json::Value>,
    day_start_value: f64,
    day_end_value: f64,
    day_pnl: f64,
    finetune_triggered: bool,
    last_save_ts: f64,
}

impl DailyTracker {
    pub fn new() -> Self {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let records = TOP_SYMBOLS
            .iter()
            .map(|s| (s.to_string(), DailyRecord::new(s)))
            .collect();
        Self {
            today,
            records,
            portfolio_snapshots: Vec::new(),
            day_start_value: 0.0,
            day_end_value: 0.0,
            day_pnl: 0.0,
            finetune_triggered: false,
            last_save_ts: 0.0,
        }
    }

    /// Feed a prediction payload from an engine.
    pub fn feed_prediction(&mut self, symbol: &str, data: &serde_json::Value) {
        if let Some(record) = self.records.get_mut(symbol) {
            let price = data["current_price"].as_f64().unwrap_or(0.0);
            let ts = data["timestamp"].as_f64().unwrap_or(0.0);
            if price > 0.0 {
                record.add_tick(ts, price);
                record.add_prediction(data);
            }
        }
    }

    /// Feed portfolio snapshot from paper trader.
    pub fn feed_portfolio(&mut self, payload: &serde_json::Value) {
        let total = payload["portfolio"]["total_value"].as_f64().unwrap_or(0.0);
        let pnl = payload["portfolio"]["total_pnl"].as_f64().unwrap_or(0.0);

        if self.day_start_value == 0.0 && total > 0.0 {
            self.day_start_value = total;
        }
        self.day_end_value = total;
        self.day_pnl = pnl;

        self.portfolio_snapshots.push(json!({
            "ts": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            "value": total,
            "pnl": pnl,
        }));

        // Capture trades
        if let Some(trades) = payload["recent_trades"].as_array() {
            for t in trades {
                if let Some(sym) = t["symbol"].as_str() {
                    if let Some(record) = self.records.get_mut(sym) {
                        let time_key = t["time"].as_str().unwrap_or("");
                        let already = record.trades.iter().any(|existing| {
                            existing["time"].as_str() == Some(time_key)
                        });
                        if !already {
                            record.trades.push(t.clone());
                        }
                    }
                }
            }
        }
    }

    /// Check if EOD should trigger. Returns true if fine-tune was triggered.
    pub fn check_eod(&mut self) -> bool {
        if self.finetune_triggered {
            return false;
        }

        let now = Local::now();
        let total_ticks: usize = self.records.values().map(|r| r.ticks.len()).sum();

        let should_trigger = (now.hour() >= 16 && now.minute() >= 5) || total_ticks >= EOD_TICK_THRESHOLD;

        if should_trigger {
            self.finetune_triggered = true;
            info!("EOD triggered — {} total ticks, PnL: ${:.4}", total_ticks, self.day_pnl);
            true
        } else {
            false
        }
    }

    /// Save daily data to disk.
    pub async fn save(&mut self) {
        let dir = PathBuf::from(LOG_DIR).join(&self.today);
        if let Err(e) = fs::create_dir_all(&dir).await {
            error!("Failed to create daily dir: {}", e);
            return;
        }

        // Summary
        let summary = json!({
            "date": self.today,
            "saved_at": Local::now().to_rfc3339(),
            "portfolio": {
                "start_value": self.day_start_value,
                "end_value": self.day_end_value,
                "pnl": self.day_pnl,
                "profitable": self.day_pnl > 0.0,
            },
            "symbols": self.records.keys().map(|s| {
                let r = &self.records[s];
                (s.clone(), json!({
                    "ticks": r.ticks.len(),
                    "predictions": r.predictions.len(),
                    "trades": r.trades.len(),
                    "accuracy": r.compute_accuracy(),
                }))
            }).collect::<HashMap<_, _>>(),
        });

        let _ = fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap()).await;

        // Per-symbol data
        for (sym, record) in &self.records {
            if !record.ticks.is_empty() {
                let _ = fs::write(
                    dir.join(format!("{}_ticks.json", sym)),
                    serde_json::to_string(&record.ticks).unwrap(),
                ).await;
            }
            if !record.predictions.is_empty() {
                let _ = fs::write(
                    dir.join(format!("{}_predictions.json", sym)),
                    serde_json::to_string(&record.predictions).unwrap(),
                ).await;
            }
        }

        // Portfolio snapshots
        if !self.portfolio_snapshots.is_empty() {
            let _ = fs::write(
                dir.join("portfolio_snapshots.json"),
                serde_json::to_string(&self.portfolio_snapshots).unwrap(),
            ).await;
        }

        let total_ticks: usize = self.records.values().map(|r| r.ticks.len()).sum();
        info!("Daily data saved to {:?} ({} ticks)", dir, total_ticks);
    }

    /// Evaluate profitability and trigger fine-tune via HTTP to Python sidecar.
    pub async fn evaluate_and_finetune(&self) {
        if self.day_pnl <= 0.0 {
            info!("Day NOT profitable (PnL: ${:.4}). Skipping fine-tune.", self.day_pnl);
            return;
        }

        info!("Day WAS PROFITABLE! PnL: ${:.4}. Sending to fine-tune sidecar.", self.day_pnl);

        // Build candle data for each symbol
        let mut symbols_data = serde_json::Map::new();
        for (sym, record) in &self.records {
            let candles = record.build_1min_candles();
            if candles.len() >= 30 {
                symbols_data.insert(sym.clone(), json!({ "candles": candles }));
                info!("  {}: {} 1-min candles for training", sym, candles.len());
            }
        }

        if symbols_data.is_empty() {
            warn!("Not enough candle data for fine-tuning.");
            return;
        }

        let body = json!({
            "date": self.today,
            "day_pnl": self.day_pnl,
            "symbols": symbols_data,
        });

        // Send to fine-tune sidecar
        match reqwest::Client::new()
            .post(FINETUNE_URL)
            .json(&body)
            .timeout(std::time::Duration::from_secs(600))
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    info!("Fine-tune sidecar returned success");
                    // TODO: trigger ONNX model hot-reload
                } else {
                    error!("Fine-tune sidecar returned {}", resp.status());
                }
            }
            Err(e) => {
                error!("Fine-tune sidecar request failed: {}", e);
            }
        }
    }

    /// Get status for the API endpoint.
    pub fn status(&self) -> serde_json::Value {
        json!({
            "date": self.today,
            "running": true,
            "portfolio": {
                "start_value": self.day_start_value,
                "current_value": self.day_end_value,
                "pnl": self.day_pnl,
                "profitable": self.day_pnl > 0.0,
            },
            "data_collected": self.records.iter().map(|(sym, r)| {
                (sym.clone(), json!({
                    "ticks": r.ticks.len(),
                    "predictions": r.predictions.len(),
                    "trades": r.trades.len(),
                }))
            }).collect::<HashMap<_, _>>(),
            "finetune_triggered": self.finetune_triggered,
            "snapshots_count": self.portfolio_snapshots.len(),
        })
    }
}
