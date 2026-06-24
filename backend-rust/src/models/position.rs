use serde::{Serialize, Deserialize};

/// Layer-by-layer prediction snapshot captured at entry time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EntryPrediction {
    pub overall_score: f64,
    pub predicted_direction: String, // "bullish" or "bearish"
    pub kronos_score: f64,
    pub kalman_score: f64,
    pub pattern_score: f64,
    pub cvd_score: f64,
    pub vp_score: f64,
    pub gex_score: f64,
    pub cot_score: f64,
    pub scoring_source: String, // "ML" or "RULES"
    pub bullish_layers: Vec<String>,
    pub bearish_layers: Vec<String>,
}

/// A single stock position with fractional shares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub shares: f64,
    pub entry_price: f64,
    pub entry_time: String,
    pub current_price: f64,
    pub high_price: f64,
    pub hold_seconds: u64,
    /// True once half the position was sold at PARTIAL_PROFIT_PCT —
    /// the remainder rides with trailing-stop protection.
    pub partial_taken: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_prediction: Option<EntryPrediction>,
}

impl Position {
    pub fn new(symbol: String, shares: f64, entry_price: f64, entry_time: String) -> Self {
        Self {
            symbol,
            shares: (shares * 1e6).round() / 1e6,
            entry_price,
            entry_time,
            current_price: entry_price,
            high_price: entry_price,
            hold_seconds: 0,
            partial_taken: false,
            entry_prediction: None,
        }
    }

    #[inline]
    pub fn market_value(&self) -> f64 {
        self.shares * self.current_price
    }

    #[inline]
    pub fn cost_basis(&self) -> f64 {
        self.shares * self.entry_price
    }

    #[inline]
    pub fn unrealized_pnl(&self) -> f64 {
        self.market_value() - self.cost_basis()
    }

    #[inline]
    pub fn unrealized_pnl_pct(&self) -> f64 {
        if self.cost_basis() == 0.0 {
            return 0.0;
        }
        self.unrealized_pnl() / self.cost_basis() * 100.0
    }

    #[inline]
    pub fn trailing_drawdown_pct(&self) -> f64 {
        if self.high_price == 0.0 {
            return 0.0;
        }
        (self.current_price - self.high_price) / self.high_price * 100.0
    }

    pub fn update(&mut self, price: f64) {
        self.current_price = price;
        if price > self.high_price {
            self.high_price = price;
        }
        self.hold_seconds += 1;
    }
}
