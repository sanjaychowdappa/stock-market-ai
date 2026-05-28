//! Candlestick pattern detection — 15 classic patterns.
//!
//! Each detector returns a list of (index, pattern_name, direction, confidence).

use crate::models::Candle;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PatternResult {
    pub index: usize,
    pub name: String,
    pub direction: PatternDir,
    pub confidence: f64,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PatternDir {
    Bullish,
    Bearish,
    Neutral,
}

impl PatternDir {
    pub fn signal(&self) -> f64 {
        match self {
            Self::Bullish => 1.0,
            Self::Bearish => -1.0,
            Self::Neutral => 0.0,
        }
    }
}

/// Detect all patterns on the given candle series.
pub fn detect_all(candles: &[Candle]) -> Vec<PatternResult> {
    let mut results = Vec::new();
    let fns: Vec<fn(&[Candle]) -> Vec<PatternResult>> = vec![
        detect_doji,
        detect_hammer,
        detect_inverted_hammer,
        detect_engulfing,
        detect_morning_star,
        detect_evening_star,
        detect_three_white_soldiers,
        detect_three_black_crows,
        detect_harami,
        detect_piercing_line,
        detect_dark_cloud,
        detect_spinning_top,
        detect_marubozu,
        detect_tweezer,
        detect_hanging_man,
    ];

    for f in fns {
        results.extend(f(candles));
    }
    results.sort_by(|a, b| b.index.cmp(&a.index));
    results
}

fn avg_body(candles: &[Candle]) -> f64 {
    if candles.is_empty() { return 0.001; }
    let sum: f64 = candles.iter().map(|c| c.body()).sum();
    (sum / candles.len() as f64).max(0.001)
}

fn detect_doji(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 2 { return r; }
    let ab = avg_body(c);
    for i in 0..c.len() {
        if c[i].body() < ab * 0.1 && c[i].range() > ab * 0.5 {
            r.push(PatternResult {
                index: i,
                name: "Doji".into(),
                direction: PatternDir::Neutral,
                confidence: 0.6,
                description: "Indecision — open equals close".into(),
            });
        }
    }
    r
}

fn detect_hammer(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    let ab = avg_body(c);
    for i in 0..c.len() {
        let body = c[i].body();
        let ls = c[i].lower_shadow();
        let us = c[i].upper_shadow();
        if body > 0.0 && ls >= body * 2.0 && us < body * 0.5 && body < ab * 1.5 {
            r.push(PatternResult {
                index: i,
                name: "Hammer".into(),
                direction: PatternDir::Bullish,
                confidence: 0.7,
                description: "Bullish reversal — long lower shadow, small body".into(),
            });
        }
    }
    r
}

fn detect_inverted_hammer(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    let ab = avg_body(c);
    for i in 0..c.len() {
        let body = c[i].body();
        let ls = c[i].lower_shadow();
        let us = c[i].upper_shadow();
        if body > 0.0 && us >= body * 2.0 && ls < body * 0.5 && body < ab * 1.5 {
            r.push(PatternResult {
                index: i,
                name: "Inverted Hammer".into(),
                direction: PatternDir::Bullish,
                confidence: 0.6,
                description: "Potential bullish reversal — long upper shadow".into(),
            });
        }
    }
    r
}

fn detect_engulfing(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    for i in 1..c.len() {
        let prev = &c[i - 1];
        let curr = &c[i];
        if prev.is_bearish()
            && curr.is_bullish()
            && curr.open <= prev.close
            && curr.close >= prev.open
        {
            r.push(PatternResult {
                index: i,
                name: "Bullish Engulfing".into(),
                direction: PatternDir::Bullish,
                confidence: 0.8,
                description: "Strong bullish reversal — current candle engulfs previous".into(),
            });
        }
        if prev.is_bullish()
            && curr.is_bearish()
            && curr.open >= prev.close
            && curr.close <= prev.open
        {
            r.push(PatternResult {
                index: i,
                name: "Bearish Engulfing".into(),
                direction: PatternDir::Bearish,
                confidence: 0.8,
                description: "Strong bearish reversal — current candle engulfs previous".into(),
            });
        }
    }
    r
}

fn detect_morning_star(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 3 { return r; }
    let ab = avg_body(c);
    for i in 2..c.len() {
        if c[i - 2].is_bearish()
            && c[i - 2].body() > ab
            && c[i - 1].body() < ab * 0.3
            && c[i].is_bullish()
            && c[i].body() > ab
            && c[i].close > c[i - 2].midpoint()
        {
            r.push(PatternResult {
                index: i,
                name: "Morning Star".into(),
                direction: PatternDir::Bullish,
                confidence: 0.85,
                description: "Strong bullish reversal — three-candle pattern".into(),
            });
        }
    }
    r
}

fn detect_evening_star(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 3 { return r; }
    let ab = avg_body(c);
    for i in 2..c.len() {
        if c[i - 2].is_bullish()
            && c[i - 2].body() > ab
            && c[i - 1].body() < ab * 0.3
            && c[i].is_bearish()
            && c[i].body() > ab
            && c[i].close < c[i - 2].midpoint()
        {
            r.push(PatternResult {
                index: i,
                name: "Evening Star".into(),
                direction: PatternDir::Bearish,
                confidence: 0.85,
                description: "Strong bearish reversal — three-candle pattern".into(),
            });
        }
    }
    r
}

fn detect_three_white_soldiers(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 3 { return r; }
    for i in 2..c.len() {
        if c[i - 2].is_bullish()
            && c[i - 1].is_bullish()
            && c[i].is_bullish()
            && c[i - 1].close > c[i - 2].close
            && c[i].close > c[i - 1].close
            && c[i - 1].open > c[i - 2].open
            && c[i].open > c[i - 1].open
        {
            r.push(PatternResult {
                index: i,
                name: "Three White Soldiers".into(),
                direction: PatternDir::Bullish,
                confidence: 0.9,
                description: "Very bullish — three consecutive higher closes".into(),
            });
        }
    }
    r
}

fn detect_three_black_crows(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 3 { return r; }
    for i in 2..c.len() {
        if c[i - 2].is_bearish()
            && c[i - 1].is_bearish()
            && c[i].is_bearish()
            && c[i - 1].close < c[i - 2].close
            && c[i].close < c[i - 1].close
        {
            r.push(PatternResult {
                index: i,
                name: "Three Black Crows".into(),
                direction: PatternDir::Bearish,
                confidence: 0.9,
                description: "Very bearish — three consecutive lower closes".into(),
            });
        }
    }
    r
}

fn detect_harami(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    for i in 1..c.len() {
        let prev = &c[i - 1];
        let curr = &c[i];
        if prev.is_bearish()
            && curr.is_bullish()
            && curr.body() < prev.body()
            && curr.open > prev.close
            && curr.close < prev.open
        {
            r.push(PatternResult {
                index: i,
                name: "Bullish Harami".into(),
                direction: PatternDir::Bullish,
                confidence: 0.65,
                description: "Potential bullish reversal — small body inside previous".into(),
            });
        }
        if prev.is_bullish()
            && curr.is_bearish()
            && curr.body() < prev.body()
            && curr.open < prev.close
            && curr.close > prev.open
        {
            r.push(PatternResult {
                index: i,
                name: "Bearish Harami".into(),
                direction: PatternDir::Bearish,
                confidence: 0.65,
                description: "Potential bearish reversal — small body inside previous".into(),
            });
        }
    }
    r
}

fn detect_piercing_line(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    for i in 1..c.len() {
        if c[i - 1].is_bearish()
            && c[i].is_bullish()
            && c[i].open < c[i - 1].low
            && c[i].close > c[i - 1].midpoint()
            && c[i].close < c[i - 1].open
        {
            r.push(PatternResult {
                index: i,
                name: "Piercing Line".into(),
                direction: PatternDir::Bullish,
                confidence: 0.7,
                description: "Bullish reversal — opens below low, closes above midpoint".into(),
            });
        }
    }
    r
}

fn detect_dark_cloud(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    for i in 1..c.len() {
        if c[i - 1].is_bullish()
            && c[i].is_bearish()
            && c[i].open > c[i - 1].high
            && c[i].close < c[i - 1].midpoint()
            && c[i].close > c[i - 1].open
        {
            r.push(PatternResult {
                index: i,
                name: "Dark Cloud Cover".into(),
                direction: PatternDir::Bearish,
                confidence: 0.7,
                description: "Bearish reversal — opens above high, closes below midpoint".into(),
            });
        }
    }
    r
}

fn detect_spinning_top(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    let ab = avg_body(c);
    for i in 0..c.len() {
        let body = c[i].body();
        let us = c[i].upper_shadow();
        let ls = c[i].lower_shadow();
        if body < ab * 0.3 && us > body && ls > body && c[i].range() > ab * 0.5 {
            r.push(PatternResult {
                index: i,
                name: "Spinning Top".into(),
                direction: PatternDir::Neutral,
                confidence: 0.5,
                description: "Indecision — small body with shadows on both sides".into(),
            });
        }
    }
    r
}

fn detect_marubozu(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    let ab = avg_body(c);
    for i in 0..c.len() {
        let body = c[i].body();
        let us = c[i].upper_shadow();
        let ls = c[i].lower_shadow();
        let shadow_ratio = if body > 0.0 { (us + ls) / body } else { 99.0 };
        if body > ab * 1.5 && shadow_ratio < 0.1 {
            let dir = if c[i].is_bullish() {
                PatternDir::Bullish
            } else {
                PatternDir::Bearish
            };
            r.push(PatternResult {
                index: i,
                name: "Marubozu".into(),
                direction: dir,
                confidence: 0.75,
                description: "Strong conviction — no shadows".into(),
            });
        }
    }
    r
}

fn detect_tweezer(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    if c.len() < 2 { return r; }
    let ab = avg_body(c);
    let tolerance = ab * 0.1;
    for i in 1..c.len() {
        // Tweezer bottom
        if c[i - 1].is_bearish()
            && c[i].is_bullish()
            && (c[i].low - c[i - 1].low).abs() < tolerance
        {
            r.push(PatternResult {
                index: i,
                name: "Tweezer Bottom".into(),
                direction: PatternDir::Bullish,
                confidence: 0.7,
                description: "Bullish reversal — matching lows".into(),
            });
        }
        // Tweezer top
        if c[i - 1].is_bullish()
            && c[i].is_bearish()
            && (c[i].high - c[i - 1].high).abs() < tolerance
        {
            r.push(PatternResult {
                index: i,
                name: "Tweezer Top".into(),
                direction: PatternDir::Bearish,
                confidence: 0.7,
                description: "Bearish reversal — matching highs".into(),
            });
        }
    }
    r
}

fn detect_hanging_man(c: &[Candle]) -> Vec<PatternResult> {
    let mut r = Vec::new();
    let ab = avg_body(c);
    for i in 0..c.len() {
        let body = c[i].body();
        let ls = c[i].lower_shadow();
        let us = c[i].upper_shadow();
        if c[i].is_bearish() && body > 0.0 && ls >= body * 2.0 && us < body * 0.3 && body < ab * 1.5 {
            r.push(PatternResult {
                index: i,
                name: "Hanging Man".into(),
                direction: PatternDir::Bearish,
                confidence: 0.65,
                description: "Bearish reversal — long lower shadow after uptrend".into(),
            });
        }
    }
    r
}
