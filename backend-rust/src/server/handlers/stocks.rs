use axum::extract::Path;
use axum::Json;
use crate::services::stock_data;
use crate::services::signal_generator;

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
