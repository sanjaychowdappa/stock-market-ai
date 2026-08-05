//! Aggregate pattern scoring: repeating-pattern detection, support/resistance,
//! micro-momentum, and candlestick pattern signals.

use crate::models::Candle;
use crate::services::pattern_recognition::{self, PatternResult};
use crate::services::technical;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PatternScore {
    pub signal: f64,          // -1.0 to 1.0
    pub direction: String,    // "bullish", "bearish", "neutral"
    pub confidence: f64,      // 0.0 to 1.0
    pub repeat_score: f64,
    pub sr_score: f64,
    pub momentum_score: f64,
    pub candle_score: f64,
    pub patterns_found: Vec<PatternResult>,
}

/// Compute the aggregate pattern score for a candle series.
pub fn compute(candles: &[Candle]) -> PatternScore {
    if candles.len() < 10 {
        return PatternScore {
            signal: 0.0,
            direction: "neutral".into(),
            confidence: 0.0,
            repeat_score: 0.0,
            sr_score: 0.0,
            momentum_score: 0.0,
            candle_score: 0.0,
            patterns_found: Vec::new(),
        };
    }

    // 1. Candlestick patterns — focus on recent (last 20 candles)
    let recent_start = candles.len().saturating_sub(20);
    let recent = &candles[recent_start..];
    let patterns = pattern_recognition::detect_all(recent);

    let candle_score = if patterns.is_empty() {
        0.0
    } else {
        let total: f64 = patterns
            .iter()
            .map(|p| p.direction.signal() * p.confidence)
            .sum();
        (total / patterns.len() as f64).clamp(-1.0, 1.0)
    };

    // 2. Repeating pattern detection (Pearson correlation on sliding windows)
    let repeat_score = find_repeating_patterns(candles);

    // 3. Support/Resistance proximity
    let sr_score = detect_sr_signal(candles);

    // 4. Micro-momentum
    let momentum_score = technical::micro_momentum(candles, 5);

    // Weighted combination — require more candle data to reduce noise
    let min_candles_bonus = if candles.len() >= 120 { 1.0 }
        else if candles.len() >= 60 { 0.7 }
        else { 0.4 };

    let signal = (0.40 * repeat_score
        + 0.20 * sr_score
        + 0.20 * momentum_score
        + 0.20 * candle_score)
        .clamp(-1.0, 1.0) * min_candles_bonus;

    // Higher threshold to avoid noise: was 0.1, now 0.2
    let direction = if signal > 0.2 {
        "bullish"
    } else if signal < -0.2 {
        "bearish"
    } else {
        "neutral"
    }
    .into();

    let confidence = signal.abs().min(1.0);

    PatternScore {
        signal,
        direction,
        confidence,
        repeat_score,
        sr_score,
        momentum_score,
        candle_score,
        patterns_found: patterns,
    }
}

/// Sliding-window Pearson correlation to detect repeating price patterns.
fn find_repeating_patterns(candles: &[Candle]) -> f64 {
    if candles.len() < 60 {
        return 0.0;
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let n = closes.len();
    let window = 20.min(n / 3);
    let target = &closes[n - window..];
    let target_mean: f64 = target.iter().sum::<f64>() / window as f64;

    let mut best_corr: f64 = 0.0;
    let mut best_next_dir: f64 = 0.0;

    // Scan for matching windows
    let scan_end = n - window - 1;
    let scan_start = scan_end.saturating_sub(200);

    for start in scan_start..scan_end {
        let candidate = &closes[start..start + window];
        let cand_mean: f64 = candidate.iter().sum::<f64>() / window as f64;

        // Pearson correlation
        let mut num = 0.0;
        let mut denom_a = 0.0;
        let mut denom_b = 0.0;
        for j in 0..window {
            let a = target[j] - target_mean;
            let b = candidate[j] - cand_mean;
            num += a * b;
            denom_a += a * a;
            denom_b += b * b;
        }
        let denom = (denom_a * denom_b).sqrt();
        if denom < 1e-10 {
            continue;
        }
        let corr = num / denom;

        if corr > best_corr && start + window < n {
            best_corr = corr;
            // What happened AFTER this matching pattern?
            let after_price = closes[(start + window).min(n - 1)];
            let pattern_end = closes[start + window - 1];
            best_next_dir = if after_price > pattern_end {
                1.0
            } else {
                -1.0
            };
        }
    }

    if best_corr > 0.7 {
        best_next_dir * (best_corr - 0.7) / 0.3 // Scale 0.7-1.0 → 0.0-1.0
    } else {
        0.0
    }
}

/// Detect support/resistance levels and return signal based on proximity.
fn detect_sr_signal(candles: &[Candle]) -> f64 {
    if candles.len() < 20 {
        return 0.0;
    }

    let n = candles.len();
    let current = candles[n - 1].close;
    let window = 5;

    // Find pivot highs and lows
    let mut supports = Vec::new();
    let mut resistances = Vec::new();

    for i in window..(n - window) {
        let is_pivot_high = (0..window).all(|j| {
            candles[i].high >= candles[i - j - 1].high && candles[i].high >= candles[i + j + 1].high
        });
        let is_pivot_low = (0..window).all(|j| {
            candles[i].low <= candles[i - j - 1].low && candles[i].low <= candles[i + j + 1].low
        });

        if is_pivot_high {
            resistances.push(candles[i].high);
        }
        if is_pivot_low {
            supports.push(candles[i].low);
        }
    }

    if supports.is_empty() && resistances.is_empty() {
        return 0.0;
    }

    let atr = candles[n - 1].range(); // Approximate

    // Near support → bullish, near resistance → bearish
    let near_support = supports.iter().any(|s| (current - s).abs() < atr * 0.5);
    let near_resistance = resistances.iter().any(|r| (current - r).abs() < atr * 0.5);

    match (near_support, near_resistance) {
        (true, false) => 0.5,
        (false, true) => -0.5,
        _ => 0.0,
    }
}
