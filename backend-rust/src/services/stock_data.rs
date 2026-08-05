//! Yahoo Finance data fetcher — replaces the yfinance Python library.

use crate::models::Candle;
use serde_json::Value;

/// Fetch historical candles from Yahoo Finance v8 API.
pub async fn fetch_candles(symbol: &str, interval: &str, range: &str) -> Result<Vec<Candle>, String> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}",
        symbol, interval, range
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("HTTP: {e}"))?;

    let data: Value = resp.json().await.map_err(|e| format!("JSON: {e}"))?;

    let result = &data["chart"]["result"][0];
    let timestamps = result["timestamp"]
        .as_array()
        .ok_or("no timestamps")?;
    let quote = &result["indicators"]["quote"][0];

    let opens = quote["open"].as_array().ok_or("no opens")?;
    let highs = quote["high"].as_array().ok_or("no highs")?;
    let lows = quote["low"].as_array().ok_or("no lows")?;
    let closes = quote["close"].as_array().ok_or("no closes")?;
    let volumes = quote["volume"].as_array().ok_or("no volumes")?;

    let mut candles = Vec::with_capacity(timestamps.len());
    for i in 0..timestamps.len() {
        let t = timestamps[i].as_f64();
        let o = opens.get(i).and_then(|v| v.as_f64());
        let h = highs.get(i).and_then(|v| v.as_f64());
        let l = lows.get(i).and_then(|v| v.as_f64());
        let c = closes.get(i).and_then(|v| v.as_f64());
        let v = volumes.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);

        if let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (t, o, h, l, c) {
            candles.push(Candle {
                time: t,
                open: o,
                high: h,
                low: l,
                close: c,
                volume: v,
            });
        }
    }

    Ok(candles)
}

