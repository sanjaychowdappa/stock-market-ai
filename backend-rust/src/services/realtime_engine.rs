//! Real-time prediction engine — uses Alpaca real-time data,
//! pattern detection, and Kronos inference for per-second predictions.

use crate::config::*;
use crate::models::Candle;
use crate::services::{alpaca_stream, candle_buffer::CandleBuffer, kronos_onnx, pattern_scorer};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

pub struct RealtimeEngine {
    pub symbol: String,
    inner: Mutex<EngineInner>,
    pub pred_tx: broadcast::Sender<serde_json::Value>,
    pub tick_tx: broadcast::Sender<serde_json::Value>,
    kronos: kronos_onnx::SharedKronos,
}

struct EngineInner {
    buffer: CandleBuffer,
    // Live market data from Alpaca
    live_price: f64,
    live_atr: f64,
    live_volume: f64,
    prev_close: f64,
    // Kronos predictions cache
    kronos_predictions: Option<Vec<Candle>>,
    kronos_timestamp: f64,
    last_payload: Option<serde_json::Value>,
}

impl RealtimeEngine {
    pub fn new(symbol: &str, kronos: kronos_onnx::SharedKronos) -> Arc<Self> {
        let (pred_tx, _) = broadcast::channel(64);
        let (tick_tx, _) = broadcast::channel(64);

        let engine = Arc::new(Self {
            symbol: symbol.to_string(),
            inner: Mutex::new(EngineInner {
                buffer: CandleBuffer::new(),
                live_price: 0.0,
                live_atr: 0.0,
                live_volume: 0.0,
                prev_close: 0.0,
                kronos_predictions: None,
                kronos_timestamp: 0.0,
                last_payload: None,
            }),
            pred_tx,
            tick_tx,
            kronos,
        });

        // Fetch initial snapshot from Alpaca
        let e = engine.clone();
        tokio::spawn(async move { e.init_snapshot().await });

        // Spawn Kronos prediction loop
        let e = engine.clone();
        tokio::spawn(async move { e.kronos_loop().await });

        // Spawn prediction builder loop (1 Hz)
        let e = engine.clone();
        tokio::spawn(async move { e.prediction_loop().await });

        engine
    }

    /// Feed a real-time trade tick from Alpaca stream.
    pub fn feed_tick(&self, price: f64, size: f64, timestamp: f64) {
        let mut inner = self.inner.lock();

        if inner.live_price == 0.0 {
            inner.prev_close = price;
        }
        inner.live_price = price;

        // Feed into candle buffer
        let vol = inner.live_volume;
        inner.buffer.feed(price, vol + size, timestamp);
        inner.live_volume = vol + size;

        // Broadcast raw tick
        let change = price - inner.prev_close;
        let change_pct = if inner.prev_close > 0.0 { change / inner.prev_close * 100.0 } else { 0.0 };
        let tick = json!({
            "symbol": self.symbol,
            "price": price,
            "size": size,
            "timestamp": timestamp,
            "change": change,
            "change_pct": change_pct,
            "change_percent": change_pct,
            "volume": inner.live_volume,
            "high": price + inner.live_atr,
            "low": price - inner.live_atr,
            "atr": inner.live_atr,
            "source": "alpaca-realtime",
        });
        let _ = self.tick_tx.send(tick);
    }

    /// Feed a 1-minute bar from Alpaca stream.
    pub fn feed_bar(&self, open: f64, high: f64, low: f64, close: f64, volume: f64, _ts: f64) {
        let mut inner = self.inner.lock();
        // Update ATR from bar range
        let bar_range = high - low;
        inner.live_atr = inner.live_atr * 0.9 + bar_range * 0.1; // EMA of ranges
        inner.live_price = close;
        inner.live_volume = volume;
    }

    pub fn subscribe_predictions(&self) -> broadcast::Receiver<serde_json::Value> {
        let rx = self.pred_tx.subscribe();
        if let Some(payload) = self.inner.lock().last_payload.clone() {
            let _ = self.pred_tx.send(payload);
        }
        rx
    }

    pub fn subscribe_ticks(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tick_tx.subscribe()
    }

    pub fn current_price(&self) -> f64 {
        self.inner.lock().live_price
    }

    // ── Background tasks ──────────────────────────────────────

    /// Fetch initial price snapshot from Alpaca REST.
    async fn init_snapshot(self: Arc<Self>) {
        match alpaca_stream::fetch_snapshot(&self.symbol).await {
            Ok((price, atr, volume)) => {
                let mut inner = self.inner.lock();
                inner.live_price = price;
                inner.live_atr = atr;
                inner.live_volume = volume;
                inner.prev_close = price;
                info!("{}: Alpaca snapshot ${:.2} ATR={:.4}", self.symbol, price, atr);
            }
            Err(e) => {
                warn!("{}: Alpaca snapshot failed: {} — will init from first tick", self.symbol, e);
            }
        }
    }

    /// Build and broadcast prediction payload at 1 Hz.
    async fn prediction_loop(self: Arc<Self>) {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

        loop {
            interval.tick().await;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();

            let maybe_payload = {
                let inner = self.inner.lock();
                let price = inner.live_price;
                if price <= 0.0 {
                    None
                } else {
                    let candles: Vec<Candle> = inner.buffer.candles().iter().copied().collect();
                    if candles.len() < 5 {
                        None
                    } else {
                        let pattern = pattern_scorer::compute(&candles);
                        let atr = inner.live_atr.max(price * 0.001);

                        let predictions = build_predictions(
                            price, atr, &pattern,
                            &inner.kronos_predictions,
                            now - inner.kronos_timestamp,
                        );

                        let change = price - inner.prev_close;
                        let change_pct = if inner.prev_close > 0.0 {
                            change / inner.prev_close * 100.0
                        } else { 0.0 };

                        Some(json!({
                            "symbol": self.symbol,
                            "current_price": price,
                            "timestamp": now,
                            "atr": atr,
                            "change_pct": change_pct,
                            "change_percent": change_pct,
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
                            "source": "alpaca-realtime",
                        }))
                    }
                }
            };

            if let Some(payload) = maybe_payload {
                let _ = self.pred_tx.send(payload.clone());
                self.inner.lock().last_payload = Some(payload);
            }
        }
    }

    /// Fetch Alpaca historical bars and send to Kronos sidecar.
    async fn kronos_loop(self: Arc<Self>) {
        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();

        let sidecar_url = "http://finetune-sidecar:8001";

        match client.get(format!("{}/health", sidecar_url)).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!("{}: Kronos sidecar connected!", self.symbol);
            }
            _ => {
                warn!("{}: Kronos sidecar not available", self.symbol);
            }
        }

        loop {
            // Fetch real 1-min bars from Alpaca (last 500 bars)
            match alpaca_stream::fetch_historical_bars(&self.symbol, 500).await {
                Ok(candle_data) if candle_data.len() >= 30 => {
                    let symbol = self.symbol.clone();
                    let body = json!({
                        "symbol": symbol,
                        "candles": candle_data,
                        "steps": 5,
                    });

                    match client.post(format!("{}/predict", sidecar_url))
                        .json(&body)
                        .send()
                        .await
                    {
                        Ok(resp) if resp.status().is_success() => {
                            if let Ok(data) = resp.json::<serde_json::Value>().await {
                                if data.get("error").is_none() {
                                    if let Some(preds) = data["predictions"].as_array() {
                                        let predicted: Vec<Candle> = preds.iter().filter_map(|p| {
                                            Some(Candle {
                                                time: 0.0,
                                                open: p["open"].as_f64()?,
                                                high: p["high"].as_f64()?,
                                                low: p["low"].as_f64()?,
                                                close: p["close"].as_f64()?,
                                                volume: 0.0,
                                            })
                                        }).collect();

                                        if !predicted.is_empty() {
                                            let now = SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .unwrap()
                                                .as_secs_f64();
                                            let mut inner = self.inner.lock();
                                            inner.kronos_predictions = Some(predicted);
                                            inner.kronos_timestamp = now;
                                            let src = data["model_source"].as_str().unwrap_or("?");
                                            let dir = data["direction"].as_str().unwrap_or("?");
                                            let change = data["total_change_pct"].as_f64().unwrap_or(0.0);
                                            info!("{}: Kronos {} ({}) change={:.3}% [{}bars]",
                                                symbol, dir, src, change, candle_data.len());
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(candles) => {
                    debug!("{}: Only {} bars from Alpaca, need 30+", self.symbol, candles.len());
                }
                Err(e) => {
                    debug!("{}: Alpaca bars fetch failed: {}", self.symbol, e);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(KRONOS_INTERVAL_SECS)).await;
        }
    }
}

/// Build blended predictions from pattern + Kronos signals.
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
        let pattern_move = pattern.signal * atr * 0.1 * (secs / 10.0);

        let kronos_price = if kronos_age < KRONOS_MAX_AGE_SECS {
            kronos.as_ref().and_then(|preds| {
                let idx = ((secs / 60.0).ceil() as usize).saturating_sub(1);
                preds.get(idx).map(|c| c.close)
            })
        } else {
            None
        };

        let predicted_price = if let Some(kp) = kronos_price {
            0.65 * kp + 0.35 * (current_price + pattern_move)
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
