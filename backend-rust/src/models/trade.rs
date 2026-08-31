use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: String,
    pub action: String,   // "BUY" or "SELL"
    pub shares: f64,
    pub price: f64,
    pub value: f64,
    pub pnl: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub reason: String,
    pub time: String,
    pub hold_seconds: Option<u64>,
}
