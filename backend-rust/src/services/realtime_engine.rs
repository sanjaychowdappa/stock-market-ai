//! Real-time prediction engine — uses Alpaca real-time data,
//! pattern detection, and Kronos inference for per-second predictions.

use crate::config::*;
use crate::models::Candle;
use crate::services::{alpaca_stream, candle_buffer::CandleBuffer, institutional_signals, kalman_filter::PriceKalmanFilter, kronos_onnx, pattern_scorer};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

pub struct RealtimeEngine {
    pub symbol: String,
    inner: Mutex<EngineInner>,
    pub pred_tx: broadcast::Sender<serde_json::Value>,
    pub tick_tx: broadcast::Sender<serde_json::Value>,
    kronos: kronos_onnx::SharedKronos,
}

struct EngineInner {
    buffer: CandleBuffer,
    kalman: PriceKalmanFilter,
    /// Bar-based Kalman: fed one close per minute instead of every tick.
    /// Tick-level velocity is dominated by microstructure noise — this is
    /// the filter trading decisions read from.
    kalman_1m: PriceKalmanFilter,
    /// Completed 1-minute candles (seeded from Alpaca history at startup).
    /// Pattern scoring runs on these, not on 1-second micro-candles.
    minute_candles: Vec<Candle>,
    cur_min: u64,
    min_open: f64,
    min_high: f64,
    min_low: f64,
    min_close: f64,
    min_vol: f64,
    cvd: institutional_signals::CvdTracker,
    // Live market data from Alpaca
    live_price: f64,
    live_atr: f64,
    live_volume: f64,
    prev_close: f64,
    // Kronos predictions cache
    kronos_predictions: Option<Vec<Candle>>,
    kronos_timestamp: f64,
    last_payload: Option<serde_json::Value>,
    // ML Agent scores cache (from agent sidecar on port 8002)
    agent_scores: Option<serde_json::Value>,
    agent_timestamp: f64,
}

impl RealtimeEngine {
    pub fn new(symbol: &str, kronos: kronos_onnx::SharedKronos) -> Arc<Self> {
        let (pred_tx, _) = broadcast::channel(64);
        let (tick_tx, _) = broadcast::channel(64);

        let engine = Arc::new(Self {
            symbol: symbol.to_string(),
            inner: Mutex::new(EngineInner {
                buffer: CandleBuffer::new(),
                kalman: PriceKalmanFilter::new(1.0, 0.01, 0.05),
                kalman_1m: PriceKalmanFilter::new(1.0, 0.01, 0.05),
                minute_candles: Vec::with_capacity(400),
                cur_min: 0,
                min_open: 0.0,
                min_high: 0.0,
                min_low: 0.0,
                min_close: 0.0,
                min_vol: 0.0,
                cvd: institutional_signals::CvdTracker::new(),
                live_price: 0.0,
                live_atr: 0.0,
                live_volume: 0.0,
                prev_close: 0.0,
                kronos_predictions: None,
                kronos_timestamp: 0.0,
                last_payload: None,
                agent_scores: None,
                agent_timestamp: 0.0,
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

        // Spawn ML Agent prediction loop (every 3s)
        let e = engine.clone();
        tokio::spawn(async move { e.agent_loop().await });

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
            inner.kalman = PriceKalmanFilter::default_for_stock(price);
            // Don't reset kalman_1m here — it may already be seeded from history
            if inner.kalman_1m.state().tick_count == 0 {
                inner.kalman_1m = PriceKalmanFilter::default_for_stock(price);
            }
        }
        inner.live_price = price;

        // Feed tick Kalman (display only) and CVD tracker with every tick
        inner.kalman.process_tick(price);
        inner.cvd.process_tick(price, size);

        // Aggregate ticks into 1-minute candles — the signal layer's input
        let minute = (timestamp / 60.0) as u64;
        if inner.cur_min == 0 {
            inner.cur_min = minute;
            inner.min_open = price;
            inner.min_high = price;
            inner.min_low = price;
            inner.min_vol = 0.0;
        } else if minute > inner.cur_min {
            // Minute rolled over: commit the completed candle, advance the bar Kalman
            let completed = Candle {
                time: (inner.cur_min * 60) as f64,
                open: inner.min_open,
                high: inner.min_high,
                low: inner.min_low,
                close: inner.min_close,
                volume: inner.min_vol,
            };
            inner.minute_candles.push(completed);
            if inner.minute_candles.len() > 390 {
                inner.minute_candles.remove(0);
            }
            let close = inner.min_close;
            inner.kalman_1m.process_tick(close);
            inner.cur_min = minute;
            inner.min_open = price;
            inner.min_high = price;
            inner.min_low = price;
            inner.min_vol = 0.0;
        }
        inner.min_high = inner.min_high.max(price);
        inner.min_low = if inner.min_low == 0.0 { price } else { inner.min_low.min(price) };
        inner.min_close = price;
        inner.min_vol += size;

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
    pub fn feed_bar(&self, _open: f64, high: f64, low: f64, close: f64, volume: f64, _ts: f64) {
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

    pub fn get_last_payload(&self) -> Option<serde_json::Value> {
        self.inner.lock().last_payload.clone()
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

        // Seed 1-min candle history + bar Kalman so pattern/trend signals
        // are warm at startup instead of needing 2h of live data to fill.
        match alpaca_stream::fetch_historical_bars(&self.symbol, 500).await {
            Ok(bars) if bars.len() >= 10 => {
                let mut inner = self.inner.lock();
                for b in &bars {
                    if let (Some(o), Some(h), Some(l), Some(c)) = (
                        b["open"].as_f64(), b["high"].as_f64(),
                        b["low"].as_f64(), b["close"].as_f64(),
                    ) {
                        let v = b["volume"].as_f64().unwrap_or(0.0);
                        if inner.kalman_1m.state().tick_count == 0 {
                            inner.kalman_1m = PriceKalmanFilter::default_for_stock(c);
                        } else {
                            inner.kalman_1m.process_tick(c);
                        }
                        inner.minute_candles.push(Candle {
                            time: 0.0, open: o, high: h, low: l, close: c, volume: v,
                        });
                    }
                }
                let n = inner.minute_candles.len();
                if n > 390 {
                    inner.minute_candles.drain(0..n - 390);
                }
                info!("{}: seeded {} 1-min candles for bar-based signals",
                    self.symbol, inner.minute_candles.len());
            }
            Ok(bars) => {
                warn!("{}: only {} historical bars — signals will warm up live", self.symbol, bars.len());
            }
            Err(e) => {
                warn!("{}: historical bar seed failed: {}", self.symbol, e);
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
                    if candles.len() < 5 || inner.minute_candles.len() < 10 {
                        None
                    } else {
                        // Signals run on 1-minute bars: tick-level data is
                        // microstructure noise (the Jun 11/12 accuracy data
                        // showed every tick-fed layer at or below coin-flip).
                        let pattern = pattern_scorer::compute(&inner.minute_candles);
                        let atr = inner.live_atr.max(price * 0.001);
                        let ks = inner.kalman_1m.state();
                        // Linear extrapolation only — velocity is $/minute.
                        // The old quadratic term predicted ±30% moves in 60s.
                        let k_price_5s = ks.price + ks.velocity * (5.0 / 60.0);
                        let k_price_30s = ks.price + ks.velocity * 0.5;
                        let k_price_60s = ks.price + ks.velocity * 1.0;
                        let cvd_state = inner.cvd.state(price, k_price_30s);

                        // Trend regime filter: price vs its intraday SMA over
                        // the available 1-min bars (~full day). uptrend = above
                        // average → long-only entries are only allowed then.
                        let mc_n = inner.minute_candles.len();
                        let trend_ma = if mc_n >= 20 {
                            inner.minute_candles.iter().map(|c| c.close).sum::<f64>() / mc_n as f64
                        } else { price };
                        let uptrend = price >= trend_ma;
                        // Shorter 30-bar (~30 min) trend reference for A/B testing
                        // against the full-day filter — catches reversals faster.
                        let trend_ma_short = if mc_n >= 10 {
                            let take = mc_n.min(30);
                            inner.minute_candles.iter().rev().take(take).map(|c| c.close).sum::<f64>() / take as f64
                        } else { price };
                        let uptrend_short = price >= trend_ma_short;

                        let predictions = build_predictions(
                            price, atr, &pattern,
                            &inner.kronos_predictions,
                            now - inner.kronos_timestamp,
                        );

                        let change = price - inner.prev_close;
                        let change_pct = if inner.prev_close > 0.0 {
                            change / inner.prev_close * 100.0
                        } else { 0.0 };

                        // Count how many layers agree on direction
                        let bullish_layers = [
                            ks.direction == "bullish",
                            pattern.signal > 0.02,
                            cvd_state.signal > 0.1,
                        ];
                        let layer_agreement = bullish_layers.iter().filter(|&&b| b).count();

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
                            "kalman": {
                                "filtered_price": ks.price,
                                "velocity": ks.velocity,
                                "acceleration": ks.acceleration,
                                // Normalized to %-per-minute so downstream
                                // thresholds are price-independent
                                "momentum": ks.velocity / price * 100.0,
                                "momentum_change": ks.acceleration / price * 100.0,
                                "trend_strength": ks.trend_strength,
                                "direction": ks.direction,
                                "confidence": ks.confidence(),
                                "price_5s": k_price_5s,
                                "price_30s": k_price_30s,
                                "price_60s": k_price_60s,
                                "strong_trend": ks.has_strong_trend(),
                                "momentum_building": ks.momentum_building(),
                                "momentum_fading": ks.momentum_fading(),
                            },
                            "cvd": {
                                "delta": cvd_state.cvd,
                                "direction": cvd_state.direction,
                                "momentum": cvd_state.cvd_momentum,
                                "buy_sell_ratio": cvd_state.buy_sell_ratio,
                                "divergence": cvd_state.divergence,
                                "signal": cvd_state.signal,
                            },
                            "layer_agreement": {
                                "bullish_count": layer_agreement,
                                "total_realtime": 3,
                                "consensus": if layer_agreement >= 3 { "strong_buy" }
                                    else if layer_agreement == 2 { "buy" }
                                    else if layer_agreement == 1 { "mixed" }
                                    else { "bearish" },
                            },
                            "trend_filter": {
                                "trend_ma": trend_ma,
                                "uptrend": uptrend,
                                "trend_ma_short": trend_ma_short,
                                "uptrend_short": uptrend_short,
                                "bars": mc_n,
                            },
                            "predictions": predictions,
                            "micro_candles": candles.len(),
                            "bar_candles": inner.minute_candles.len(),
                            "kronos_age_seconds": now - inner.kronos_timestamp,
                            "agent_scores": inner.agent_scores.clone(),
                            "agent_age_seconds": now - inner.agent_timestamp,
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

    /// Fetch ML agent predictions from sidecar (port 8002).
    /// Runs every 3 seconds per symbol. Sends current features, gets back
    /// individual agent scores + meta-ensemble decision.
    async fn agent_loop(self: Arc<Self>) {
        tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();

        let agent_url = "http://finetune-sidecar:8002";

        // Check if agent sidecar is available
        match client.get(format!("{}/agents/health", agent_url)).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    let trained = data["trained"].as_bool().unwrap_or(false);
                    info!("{}: Agent sidecar connected (trained={})", self.symbol, trained);
                }
            }
            _ => {
                warn!("{}: Agent sidecar not available on port 8002 — using hardcoded scoring", self.symbol);
                return; // Don't loop if sidecar isn't up
            }
        }

        loop {
            // Collect current features from inner state
            let request_body = {
                let inner = self.inner.lock();
                let price = inner.live_price;
                if price <= 0.0 {
                    None
                } else if let Some(ref payload) = inner.last_payload {
                    let kalman = &payload["kalman"];
                    let cvd = &payload["cvd"];
                    let pattern = &payload["pattern"];
                    Some(json!({
                        "symbol": self.symbol,
                        "kronos_score": 0.0, // Will be set by paper_trader from bias
                        "kronos_confidence": 0.5,
                        "kalman_momentum": kalman["momentum"].as_f64().unwrap_or(0.0),
                        "kalman_trend_strength": kalman["trend_strength"].as_f64().unwrap_or(0.0),
                        "kalman_confidence": kalman["confidence"].as_f64().unwrap_or(0.5),
                        "kalman_direction": kalman["direction"].as_str().unwrap_or("neutral"),
                        "kalman_momentum_building": kalman["momentum_building"].as_bool().unwrap_or(false),
                        "kalman_momentum_fading": kalman["momentum_fading"].as_bool().unwrap_or(false),
                        "pattern_signal": pattern["signal"].as_f64().unwrap_or(0.0),
                        "pattern_confidence": pattern["confidence"].as_f64().unwrap_or(0.0),
                        "signal_history": [],
                        "cvd_signal": cvd["signal"].as_f64().unwrap_or(0.0),
                        "cvd_buy_sell_ratio": cvd["buy_sell_ratio"].as_f64().unwrap_or(1.0),
                        "prev_cvd_signal": 0.0,
                        "vp_signal": 0.0,
                        "vp_position": "unknown",
                        "price": price,
                        "poc_price": 0.0,
                        "gex_signal": 0.0,
                        "gex_regime": "neutral",
                        "cot_signal": 0.0,
                    }))
                } else {
                    None
                }
            };

            if let Some(body) = request_body {
                match client.post(format!("{}/predict/agents", agent_url))
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            if data.get("error").is_none() {
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs_f64();
                                let meta_score = data["meta_score"].as_f64().unwrap_or(0.0);
                                let action = data["action"].as_str().unwrap_or("HOLD");
                                let trained = data["is_trained"].as_bool().unwrap_or(false);
                                debug!("{}: Agents meta={:.3} action={} trained={}",
                                    self.symbol, meta_score, action, trained);

                                let mut inner = self.inner.lock();
                                inner.agent_scores = Some(data);
                                inner.agent_timestamp = now;
                            }
                        }
                    }
                    _ => {
                        debug!("{}: Agent sidecar request failed", self.symbol);
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
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
