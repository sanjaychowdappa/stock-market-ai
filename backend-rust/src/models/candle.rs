use serde::{Deserialize, Serialize};

/// OHLCV candle — core data type used everywhere.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Candle {
    pub time: f64, // unix timestamp (seconds)
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Candle {
    #[inline]
    pub fn body(&self) -> f64 {
        (self.close - self.open).abs()
    }

    #[inline]
    pub fn range(&self) -> f64 {
        self.high - self.low
    }

    #[inline]
    pub fn upper_shadow(&self) -> f64 {
        self.high - self.open.max(self.close)
    }

    #[inline]
    pub fn lower_shadow(&self) -> f64 {
        self.open.min(self.close) - self.low
    }

    #[inline]
    pub fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    #[inline]
    pub fn is_bearish(&self) -> bool {
        self.close < self.open
    }

    #[inline]
    pub fn midpoint(&self) -> f64 {
        (self.high + self.low) / 2.0
    }
}
