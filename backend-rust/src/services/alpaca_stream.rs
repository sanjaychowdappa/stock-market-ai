//! Alpaca real-time market data streaming.
//!
//! Connects to Alpaca's WebSocket for real-time trades/quotes/bars.
//! Also provides REST endpoints for historical 1-min bars (for Kronos).
//! Replaces Yahoo Finance delayed data + OU simulator.

use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::env;
use tokio::sync::broadcast;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

const ALPACA_STREAM_URL: &str = "wss://stream.data.alpaca.markets/v2/iex";
const ALPACA_SNAPSHOT_URL: &str = "https://data.alpaca.markets/v2/stocks";

/// Real-time tick from Alpaca.
#[derive(Debug, Clone)]
pub struct AlpacaTick {
    pub symbol: String,
    pub price: f64,
    pub size: f64,
    pub timestamp: f64,
}

/// Real-time bar (1-min) from Alpaca.
#[derive(Debug, Clone)]
pub struct AlpacaBar {
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp: f64,
}

/// Spawn the Alpaca WebSocket stream for given symbols.
/// Returns a broadcast receiver for real-time ticks and bars.
pub fn spawn_stream(
    symbols: &[&str],
) -> (broadcast::Sender<AlpacaTick>, broadcast::Sender<AlpacaBar>) {
    let (tick_tx, _) = broadcast::channel(1024);
    let (bar_tx, _) = broadcast::channel(256);

    let tick_tx2 = tick_tx.clone();
    let bar_tx2 = bar_tx.clone();
    let syms: Vec<String> = symbols.iter().map(|s| s.to_string()).collect();

    tokio::spawn(async move {
        loop {
            match run_stream(&syms, &tick_tx2, &bar_tx2).await {
                Ok(_) => info!("Alpaca stream ended normally"),
                Err(e) => error!("Alpaca stream error: {} — reconnecting in 5s", e),
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    (tick_tx, bar_tx)
}

async fn run_stream(
    symbols: &[String],
    tick_tx: &broadcast::Sender<AlpacaTick>,
    bar_tx: &broadcast::Sender<AlpacaBar>,
) -> Result<(), String> {
    let api_key = env::var("APCA_API_KEY_ID")
        .map_err(|_| "APCA_API_KEY_ID not set".to_string())?;
    let api_secret = env::var("APCA_API_SECRET_KEY")
        .map_err(|_| "APCA_API_SECRET_KEY not set".to_string())?;

    info!("Connecting to Alpaca real-time stream...");

    let (ws, _) = connect_async(ALPACA_STREAM_URL).await
        .map_err(|e| format!("WebSocket connect failed: {}", e))?;

    let (mut write, mut read) = ws.split();

    // Wait for welcome message
    if let Some(Ok(msg)) = read.next().await {
        let data: serde_json::Value = serde_json::from_str(&msg.to_string()).unwrap_or_default();
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.first() {
                let msg_type = first["T"].as_str().unwrap_or("");
                if msg_type == "success" {
                    info!("Alpaca stream: connected");
                }
            }
        }
    }

    // Authenticate
    let auth = json!({
        "action": "auth",
        "key": api_key,
        "secret": api_secret,
    });
    write.send(Message::Text(auth.to_string())).await
        .map_err(|e| format!("Auth send failed: {}", e))?;

    // Wait for auth response
    if let Some(Ok(msg)) = read.next().await {
        let data: serde_json::Value = serde_json::from_str(&msg.to_string()).unwrap_or_default();
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.first() {
                let msg_type = first["T"].as_str().unwrap_or("");
                if msg_type == "success" && first["msg"].as_str() == Some("authenticated") {
                    info!("Alpaca stream: authenticated");
                } else {
                    return Err(format!("Auth failed: {:?}", first));
                }
            }
        }
    }

    // Subscribe to trades and 1-min bars for all symbols
    let subscribe = json!({
        "action": "subscribe",
        "trades": symbols,
        "bars": symbols,
    });
    write.send(Message::Text(subscribe.to_string())).await
        .map_err(|e| format!("Subscribe failed: {}", e))?;

    // Wait for subscription confirmation
    if let Some(Ok(msg)) = read.next().await {
        let data: serde_json::Value = serde_json::from_str(&msg.to_string()).unwrap_or_default();
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.first() {
                if first["T"].as_str() == Some("subscription") {
                    info!("Alpaca stream: subscribed to {} symbols (trades + bars)",
                        symbols.len());
                }
            }
        }
    }

    // Process messages
    while let Some(msg_result) = read.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!("Alpaca stream read error: {}", e);
                break;
            }
        };

        if let Message::Text(text) = msg {
            let data: serde_json::Value = match serde_json::from_str(&text) {
                Ok(d) => d,
                Err(_) => continue,
            };

            if let Some(arr) = data.as_array() {
                for item in arr {
                    let msg_type = item["T"].as_str().unwrap_or("");

                    match msg_type {
                        "t" => {
                            // Trade
                            if let (Some(sym), Some(price), Some(size)) = (
                                item["S"].as_str(),
                                item["p"].as_f64(),
                                item["s"].as_f64(),
                            ) {
                                let ts = parse_alpaca_timestamp(
                                    item["t"].as_str().unwrap_or(""),
                                );
                                let _ = tick_tx.send(AlpacaTick {
                                    symbol: sym.to_string(),
                                    price,
                                    size,
                                    timestamp: ts,
                                });
                            }
                        }
                        "b" => {
                            // 1-minute bar
                            if let (Some(sym), Some(o), Some(h), Some(l), Some(c)) = (
                                item["S"].as_str(),
                                item["o"].as_f64(),
                                item["h"].as_f64(),
                                item["l"].as_f64(),
                                item["c"].as_f64(),
                            ) {
                                let vol = item["v"].as_f64().unwrap_or(0.0);
                                let ts = parse_alpaca_timestamp(
                                    item["t"].as_str().unwrap_or(""),
                                );
                                let _ = bar_tx.send(AlpacaBar {
                                    symbol: sym.to_string(),
                                    open: o,
                                    high: h,
                                    low: l,
                                    close: c,
                                    volume: vol,
                                    timestamp: ts,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

/// Fetch historical 1-min bars from Alpaca REST API for Kronos.
pub async fn fetch_historical_bars(
    symbol: &str,
    limit: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let api_key = env::var("APCA_API_KEY_ID")
        .map_err(|_| "APCA_API_KEY_ID not set".to_string())?;
    let api_secret = env::var("APCA_API_SECRET_KEY")
        .map_err(|_| "APCA_API_SECRET_KEY not set".to_string())?;

    let url = format!(
        "{}/v2/stocks/{}/bars?timeframe=1Min&limit={}&feed=iex",
        env::var("APCA_DATA_URL").unwrap_or_else(|_| "https://data.alpaca.markets".to_string()),
        symbol,
        limit,
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("APCA-API-KEY-ID", &api_key)
        .header("APCA-API-SECRET-KEY", &api_secret)
        .send()
        .await
        .map_err(|e| format!("Alpaca bars fetch: {}", e))?;

    let data: serde_json::Value = resp.json().await
        .map_err(|e| format!("Alpaca bars JSON: {}", e))?;

    let bars = data["bars"].as_array()
        .ok_or("No bars in response")?;

    let candles: Vec<serde_json::Value> = bars.iter().filter_map(|b| {
        let ts = parse_alpaca_timestamp(b["t"].as_str()?);
        Some(json!({
            "open": b["o"].as_f64()?,
            "high": b["h"].as_f64()?,
            "low": b["l"].as_f64()?,
            "close": b["c"].as_f64()?,
            "volume": b["v"].as_f64().unwrap_or(0.0),
            "time": ts,
        }))
    }).collect();

    Ok(candles)
}

/// Fetch latest snapshot (price, volume) from Alpaca.
pub async fn fetch_snapshot(symbol: &str) -> Result<(f64, f64, f64), String> {
    let api_key = env::var("APCA_API_KEY_ID")
        .map_err(|_| "APCA_API_KEY_ID not set".to_string())?;
    let api_secret = env::var("APCA_API_SECRET_KEY")
        .map_err(|_| "APCA_API_SECRET_KEY not set".to_string())?;

    let url = format!(
        "{}/{}/snapshot?feed=iex",
        ALPACA_SNAPSHOT_URL, symbol,
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("APCA-API-KEY-ID", &api_key)
        .header("APCA-API-SECRET-KEY", &api_secret)
        .send()
        .await
        .map_err(|e| format!("Alpaca snapshot: {}", e))?;

    let data: serde_json::Value = resp.json().await
        .map_err(|e| format!("Alpaca snapshot JSON: {}", e))?;

    let price = data["latestTrade"]["p"].as_f64()
        .or_else(|| data["minuteBar"]["c"].as_f64())
        .ok_or("No price in snapshot")?;
    let volume = data["minuteBar"]["v"].as_f64().unwrap_or(0.0);

    // Compute ATR from daily bar
    let high = data["dailyBar"]["h"].as_f64().unwrap_or(price);
    let low = data["dailyBar"]["l"].as_f64().unwrap_or(price);
    let atr = (high - low).max(price * 0.003); // Fallback 0.3%

    Ok((price, atr, volume))
}

/// Fetch daily bars from Alpaca (for GEX realized volatility calculation).
pub async fn fetch_daily_bars(
    symbol: &str,
    limit: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let api_key = env::var("APCA_API_KEY_ID")
        .map_err(|_| "APCA_API_KEY_ID not set".to_string())?;
    let api_secret = env::var("APCA_API_SECRET_KEY")
        .map_err(|_| "APCA_API_SECRET_KEY not set".to_string())?;

    // Without an explicit start, Alpaca's daily-bar endpoint returns only the
    // single most recent bar. Span enough calendar days to cover `limit`
    // trading days (~1.5x for weekends/holidays) so we get real history.
    let lookback_days = (limit as i64 * 7 / 5) + 40;
    let start = (chrono::Utc::now() - chrono::Duration::days(lookback_days))
        .format("%Y-%m-%d").to_string();
    // NO `limit` in the request. Alpaca's limit truncates from the START of the
    // window, so `&start=<124 days ago>&limit=60` returned 2026-04-06..06-30 —
    // the OLDEST 60 bars, ending five weeks before today. Callers take
    // bars.last() as "current", so the market-regime filter spent weeks
    // comparing a stale price to a stale SMA: it logged "QQQ $736.07 vs 50d SMA
    // $706.13 -> RISK-ON" on 2026-08-06, where $736.07 is the June 30 close.
    // Fetch the whole window and keep the most recent `limit` bars instead.
    let url = format!(
        "{}/v2/stocks/{}/bars?timeframe=1Day&start={}&feed=iex",
        env::var("APCA_DATA_URL").unwrap_or_else(|_| "https://data.alpaca.markets".to_string()),
        symbol,
        start,
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("APCA-API-KEY-ID", &api_key)
        .header("APCA-API-SECRET-KEY", &api_secret)
        .send()
        .await
        .map_err(|e| format!("Alpaca daily bars fetch: {}", e))?;

    let data: serde_json::Value = resp.json().await
        .map_err(|e| format!("Alpaca daily bars JSON: {}", e))?;

    let bars = data["bars"].as_array()
        .ok_or("No bars in daily response")?;

    let mut candles: Vec<serde_json::Value> = bars.iter().filter_map(|b| {
        Some(json!({
            "open": b["o"].as_f64()?,
            "high": b["h"].as_f64()?,
            "low": b["l"].as_f64()?,
            "close": b["c"].as_f64()?,
            "volume": b["v"].as_f64().unwrap_or(0.0),
            // Kronos /predict requires a timestamp per candle (422 without it)
            "time": b["t"].as_str().map(parse_alpaca_timestamp).unwrap_or(0.0),
        }))
    }).collect();

    // Keep the most recent `limit` bars. Every caller treats bars.last() as the
    // current price, so the tail is what matters — trimming from the front is
    // what made the regime filter read a five-week-old close as "now".
    if candles.len() > limit {
        candles.drain(..candles.len() - limit);
    }

    Ok(candles)
}

fn parse_alpaca_timestamp(ts: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.timestamp() as f64 + dt.timestamp_subsec_millis() as f64 / 1000.0)
        .unwrap_or(0.0)
}
