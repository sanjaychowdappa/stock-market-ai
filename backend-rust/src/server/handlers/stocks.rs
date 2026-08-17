use axum::extract::Path;
use axum::Json;
use crate::services::stock_data;
use crate::services::signal_generator;

/// Recent 1-minute bars, for seeding the chart.
///
/// The chart previously had no history source at all: it opened an empty
/// series and drew forward from the websocket, so every page load started
/// blank and a reload discarded everything already drawn. `/api/stocks/{sym}`
/// could not fill that gap — it returns `candles_count` and `latest` but never
/// the candles themselves, and it fetches DAILY bars, which are the wrong
/// granularity for a 1-minute chart.
///
/// Bars come back oldest-first as `{open, high, low, close, volume, time}`,
/// with `time` in unix seconds, which is the shape lightweight-charts'
/// `setData` expects with no transformation.
pub async fn get_bars(Path(symbol): Path<String>) -> Json<serde_json::Value> {
    let symbol = symbol.to_uppercase();
    match crate::services::alpaca_stream::fetch_historical_bars(&symbol, 240).await {
        Ok(bars) => Json(serde_json::json!({ "symbol": symbol, "bars": bars })),
        // Report the failure rather than an empty array: an empty `bars` is
        // indistinguishable from "this symbol simply had no trades", and the
        // chart would silently show nothing while looking healthy.
        Err(e) => Json(serde_json::json!({ "symbol": symbol, "bars": [], "error": e })),
    }
}

pub async fn get_stock(Path(symbol): Path<String>) -> Json<serde_json::Value> {
    let symbol = symbol.to_uppercase();
    match stock_data::fetch_candles(&symbol, "1d", "6mo").await {
        Ok(candles) => {
            let signal = signal_generator::generate(&candles);
            let last = candles.last().map(|c| serde_json::json!({
                "open": c.open, "high": c.high, "low": c.low,
                "close": c.close, "volume": c.volume, "time": c.time,
            }));
            Json(serde_json::json!({
                "symbol": symbol,
                "candles_count": candles.len(),
                "latest": last,
                "signal": signal,
            }))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

pub async fn get_signals(Path(symbol): Path<String>) -> Json<serde_json::Value> {
    let symbol = symbol.to_uppercase();
    match stock_data::fetch_candles(&symbol, "1d", "3mo").await {
        Ok(candles) => {
            let signal = signal_generator::generate(&candles);
            let price = candles.last().map(|c| c.close).unwrap_or(0.0);
            Json(serde_json::json!({
                "symbol": symbol,
                "signal": signal.action,
                "score": signal.score,
                "confidence": signal.confidence,
                "reasons": signal.reasons,
                "current_price": price,
            }))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}

pub async fn get_patterns(Path(symbol): Path<String>) -> Json<serde_json::Value> {
    let symbol = symbol.to_uppercase();
    match stock_data::fetch_candles(&symbol, "1d", "3mo").await {
        Ok(candles) => {
            let score = crate::services::pattern_scorer::compute(&candles);
            Json(serde_json::json!({
                "symbol": symbol,
                "pattern_score": score,
            }))
        }
        Err(e) => Json(serde_json::json!({"error": e})),
    }
}
