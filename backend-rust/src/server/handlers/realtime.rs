use axum::extract::{Path, State};
use axum::Json;
use crate::state::AppState;
use std::sync::Arc;

pub async fn predict(
    State(state): State<Arc<AppState>>,
    Path(symbol): Path<String>,
) -> Json<serde_json::Value> {
    let symbol = symbol.to_uppercase();
    let engine = state.get_engine(&symbol);
    let price = engine.current_price();

    Json(serde_json::json!({
        "symbol": symbol,
        "current_price": price,
        "engine": "rust-ort",
    }))
}
