//! Real-time prediction engine — orchestrates OU simulation,
//! pattern detection, Kronos inference, and per-second prediction blending.

use crate::config::*;
use crate::models::Candle;
use crate::services::{candle_buffer::CandleBuffer, kronos_onnx, ou_simulator::OuSimulator, pattern_scorer};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Shared engine state behind Arc for thread safety.
pub struct RealtimeEngine {
    pub symbol: String,
    inner: Mutex<EngineInner>,
    /// Prediction updates (subscribers get latest per-second payload).
    pub pred_tx: broadcast::Sender<serde_json::Value>,
    /// Raw tick stream for live chart.
    pub tick_tx: broadcast::Sender<serde_json::Value>,
    /// Kronos ONNX session (shared across all engines).
    kronos: kronos_onnx::SharedKronos,
}

struct EngineInner {
    sim: OuSimulator,
    buffer: CandleBuffer,
    // Yahoo Finance cache
    yf_price: f64,
    yf_atr: f64,
    yf_change: f64,
    yf_change_pct: f64,
    yf_volume: f64,
    // Kronos predictions cache
    kronos_predictions: Option<Vec<Candle>>,
    kronos_timestamp: f64,
    // Last broadcast payload (for instant snapshot on subscribe)
    last_payload: Option<serde_json::Value>,
    running: bool,
}

impl RealtimeEngine {
    pub fn new(symbol: &str, kronos: kronos_onnx::SharedKronos) -> Arc<Self> {
        let (pred_tx, _) = broadcast::channel(64);
        let (tick_tx, _) = broadcast::channel(64);

        let engine = Arc::new(Self {
            symbol: symbol.to_string(),
            inner: Mutex::new(EngineInner {
                sim: OuSimulator::new(100.0, 1.0), // Placeholder until YF data arrives
                buffer: CandleBuffer::new(),
                yf_price: 0.0,
                yf_atr: 0.0,
                yf_change: 0.0,
                yf_change_pct: 0.0,
                yf_volume: 0.0,
                kronos_predictions: None,
                kronos_timestamp: 0.0,
                last_payload: None,
                running: true,
            }),
            pred_tx,
            tick_tx,
            kronos,
        });

        // Spawn background tasks
        let e = engine.clone();
        tokio::spawn(async move { e.yf_loop().await });
        let e = engine.clone();
        tokio::spawn(async move { e.sim_loop().await });
        let e = engine.clone();
        tokio::spawn(async move { e.kronos_loop().await });

        engine
    }

    /// Subscribe to prediction stream. Returns a receiver.
    pub fn subscribe_predictions(&self) -> broadcast::Receiver<serde_json::Value> {
        let rx = self.pred_tx.subscribe();
        // Push last payload immediately for instant first frame
        if let Some(payload) = self.inner.lock().last_payload.clone() {
            let _ = self.pred_tx.send(payload);
        }
        rx
    }

    /// Subscribe to raw tick stream.
    pub fn subscribe_ticks(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tick_tx.subscribe()
    }

    /// Get last known price.
    pub fn current_price(&self) -> f64 {
        self.inner.lock().sim.current_price()
    }

    // ── Background loops ──────────────────────────────────────────

    /// Fetch Yahoo Finance data every YF_REFRESH_SECS.
    async fn yf_loop(self: Arc<Self>) {
        loop {
            match fetch_yahoo_price(&self.symbol).await {
                Ok((price, atr, change, change_pct, volume)) => {
                    let mut inner = self.inner.lock();
                    if inner.yf_price == 0.0 {
                        // First fetch — initialize simulator
                        inner.sim = OuSimulator::new(price, atr);
                        info!("{}: initialized at ${:.2} ATR={:.4}", self.symbol, price, atr);
                    } else {
                        inner.sim.update_mu(price, atr);
                    }
                    inner.yf_price = price;
                    inner.yf_atr = atr;
                    inner.yf_change = change;
                    inner.yf_change_pct = change_pct;
                    inner.yf_volume = volume;
                }
                Err(e) => {
                    warn!("{}: Yahoo fetch failed: {}", self.symbol, e);
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(YF_REFRESH_SECS)).await;
        }
    }

    /// Generate ticks at 1 Hz, build candles, compute fast predictions.
    async fn sim_loop(self: Arc<Self>) {
        // Wait for YF data
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

        loop {
            interval.tick().await;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();

            let maybe_payload = {
                let mut inner = self.inner.lock();
                if inner.yf_price == 0.0 {
                    None // No data yet
                } else {
                    let price = inner.sim.tick();
                    let vol = inner.yf_volume;
                    inner.buffer.feed(price, vol, now);

                    // Broadcast tick
                    let tick = json!({
                        "symbol": self.symbol,
                        "price": price,
                        "timestamp": now,
                        "change": inner.yf_change,
                        "change_pct": inner.yf_change_pct,
                        "change_percent": inner.yf_change_pct,
                        "volume": inner.yf_volume,
                        "high": inner.yf_price + inner.yf_atr,
                        "low": inner.yf_price - inner.yf_atr,
                        "atr": inner.yf_atr,
                    });
                    let _ = self.tick_tx.send(tick);

                    // Build prediction payload
                    let candles: Vec<Candle> = inner.buffer.candles().iter().copied().collect();
                    if candles.len() < 10 {
                        None
                    } else {
                        // Fast pattern signal
                        let pattern = pattern_scorer::compute(&candles);
                        let atr = inner.buffer.atr(14);

                        // Blend Kronos + pattern into per-second predictions
                        let predictions = build_predictions(
                            price, atr, &pattern,
                            &inner.kronos_predictions,
                            now - inner.kronos_timestamp,
                        );

                        let payload = json!({
                            "symbol": self.symbol,
                            "current_price": price,
                            "timestamp": now,
                            "atr": atr,
                            "pattern": {
                                "signal": pattern.signal,
                                "direction": pattern.direction,
                                "confidence": pattern.confidence,
                                "momentum": pattern.momentum_score,
                                "trend": pattern.repeat_score,
                                "reversion": pattern.sr_score,
                            },
                            "predictions": predictions,
                            "micro_candles": candles.len(),
                            "kronos_age_seconds": now - inner.kronos_timestamp,
                        });

                        inner.last_payload = Some(payload.clone());
                        Some(payload)
                    }
                }
            };

            if let Some(payload) = maybe_payload {
                let _ = self.pred_tx.send(payload);
            }
        }
    }

    /// Run Kronos ONNX inference every KRONOS_INTERVAL_SECS.
    async fn kronos_loop(self: Arc<Self>) {
        // Wait for enough candle data
        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

        loop {
            let candles_1min = {
                let inner = self.inner.lock();
                inner.buffer.to_1min_candles()
            };

            if candles_1min.len() >= 60 {
                let kronos = self.kronos.clone();
                let symbol = self.symbol.clone();

                // Run on blocking thread (ONNX calls are synchronous)
                let result = tokio::task::spawn_blocking(move || {
                    kronos_onnx::predict(&kronos, &candles_1min, 5, 0.8, 0.9)
                })
                .await;

                match result {
                    Ok(Some(predicted)) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs_f64();
                        let mut inner = self.inner.lock();
                        inner.kronos_predictions = Some(predicted);
                        inner.kronos_timestamp = now;
                        debug!("{}: Kronos prediction updated", symbol);
                    }
                    Ok(None) => {
                        debug!("{}: Kronos returned no prediction", self.symbol);
                    }
                    Err(e) => {
                        error!("{}: Kronos inference error: {}", self.symbol, e);
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(KRONOS_INTERVAL_SECS)).await;
        }
    }
}

/// Build blended per-second predictions from pattern + Kronos signals.
fn build_predictions(
    current_price: f64,
    atr: f64,
    pattern: &pattern_scorer::PatternScore,
    kronos: &Option<Vec<Candle>>,
    kronos_age: f64,
) -> Vec<serde_json::Value> {
    let horizons = [5.0, 10.0, 30.0, 60.0];
    let mut predictions = Vec::new();

    for &secs in &horizons {
        // Pattern-based prediction
        let pattern_move = pattern.signal * atr * 0.1 * (secs / 10.0);

        // Kronos prediction (if available and fresh)
        let kronos_price = if kronos_age < KRONOS_MAX_AGE_SECS {
            kronos.as_ref().and_then(|preds| {
                // Map seconds to 1-min candle index
                let idx = ((secs / 60.0).ceil() as usize).saturating_sub(1);
                preds.get(idx).map(|c| c.close)
            })
        } else {
            None
        };

        // Blend
        let predicted_price = if let Some(kp) = kronos_price {
            let kronos_weight = 0.65;
            let pattern_weight = 0.35;
            kronos_weight * kp + pattern_weight * (current_price + pattern_move)
        } else {
            current_price + pattern_move
        };

        let change = predicted_price - current_price;
        let change_pct = change / current_price * 100.0;

        predictions.push(json!({
            "seconds_ahead": secs as u64,
            "predicted_price": (predicted_price * 100.0).round() / 100.0,
            "change": (change * 10000.0).round() / 10000.0,
            "change_pct": (change_pct * 10000.0).round() / 10000.0,
            "change_percent": (change_pct * 10000.0).round() / 10000.0,
            "kronos_price": kronos_price,
            "direction": if change > 0.0 { "bullish" } else { "bearish" },
        }));
    }

    predictions
}

/// Fetch current price, ATR, change from Yahoo Finance v8 API.
async fn fetch_yahoo_price(symbol: &str) -> Result<(f64, f64, f64, f64, f64), String> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=2d",
        symbol
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("JSON error: {e}"))?;

    let result = &data["chart"]["result"][0];
    let meta = &result["meta"];

    let price = meta["regularMarketPrice"]
        .as_f64()
        .ok_or("no price")?;
    let prev_close = meta["chartPreviousClose"]
        .as_f64()
        .unwrap_or(price);

    let change = price - prev_close;
    let change_pct = if prev_close > 0.0 { change / prev_close * 100.0 } else { 0.0 };

    // Compute ATR from recent candles
    let quotes = &result["indicators"]["quote"][0];
    let highs: Vec<f64> = quotes["high"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let lows: Vec<f64> = quotes["low"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let closes: Vec<f64> = quotes["close"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let volumes: Vec<f64> = quotes["volume"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    let volume = volumes.last().copied().unwrap_or(0.0);

    // Simple ATR from last 14 bars
    let n = highs.len().min(lows.len()).min(closes.len());
    let atr = if n >= 15 {
        let mut tr_sum = 0.0;
        for i in (n - 14)..n {
            let tr = (highs[i] - lows[i])
                .max((highs[i] - closes[i.saturating_sub(1)]).abs())
                .max((lows[i] - closes[i.saturating_sub(1)]).abs());
            tr_sum += tr;
        }
        tr_sum / 14.0
    } else {
        price * 0.005 // Fallback: 0.5% of price
    };

    Ok((price, atr, change, change_pct, volume))
}
