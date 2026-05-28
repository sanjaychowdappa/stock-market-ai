//! Composite signal generator — RSI, MACD, Bollinger, SMA crossover scoring.

use crate::models::Candle;
use crate::services::technical;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub action: String,     // "BUY", "SELL", "HOLD"
    pub score: f64,         // -100 to 100
    pub confidence: f64,    // 0 to 1
    pub reasons: Vec<String>,
}

pub fn generate(candles: &[Candle]) -> Signal {
    if candles.len() < 30 {
        return Signal {
            action: "HOLD".into(),
            score: 0.0,
            confidence: 0.0,
            reasons: vec!["Insufficient data".into()],
        };
    }

    let mut score: f64 = 0.0;
    let mut reasons = Vec::new();

    // RSI
    let rsi_vals = technical::rsi(candles, 14);
    if let Some(&rsi) = rsi_vals.last() {
        if rsi.is_finite() {
            if rsi < 30.0 {
                score += 25.0;
                reasons.push(format!("RSI oversold ({:.1})", rsi));
            } else if rsi > 70.0 {
                score -= 25.0;
                reasons.push(format!("RSI overbought ({:.1})", rsi));
            }
        }
    }

    // MACD
    let (macd_line, signal_line, hist) = technical::macd(candles);
    if let (Some(&m), Some(&s), Some(&h)) = (macd_line.last(), signal_line.last(), hist.last()) {
        if m.is_finite() && s.is_finite() {
            if h > 0.0 && m > s {
                score += 20.0;
                reasons.push("MACD bullish crossover".into());
            } else if h < 0.0 && m < s {
                score -= 20.0;
                reasons.push("MACD bearish crossover".into());
            }
        }
    }

    // Bollinger Bands
    let (upper, _mid, lower) = technical::bollinger(candles, 20, 2.0);
    let current_price = candles.last().unwrap().close;
    if let (Some(&u), Some(&l)) = (upper.last(), lower.last()) {
        if u.is_finite() && l.is_finite() {
            if current_price <= l {
                score += 20.0;
                reasons.push("Price at lower Bollinger Band".into());
            } else if current_price >= u {
                score -= 20.0;
                reasons.push("Price at upper Bollinger Band".into());
            }
        }
    }

    // SMA crossover (20/50)
    let sma20 = technical::sma(candles, 20);
    let sma50 = technical::sma(candles, 50);
    if let (Some(&s20), Some(&s50)) = (sma20.last(), sma50.last()) {
        if s20.is_finite() && s50.is_finite() {
            if s20 > s50 {
                score += 15.0;
                reasons.push("SMA 20 > SMA 50 (bullish trend)".into());
            } else {
                score -= 15.0;
                reasons.push("SMA 20 < SMA 50 (bearish trend)".into());
            }
        }
    }

    // Volume trend
    if candles.len() >= 20 {
        let recent_vol: f64 = candles[candles.len() - 5..].iter().map(|c| c.volume).sum::<f64>() / 5.0;
        let avg_vol: f64 = candles[candles.len() - 20..].iter().map(|c| c.volume).sum::<f64>() / 20.0;
        if avg_vol > 0.0 && recent_vol > avg_vol * 1.5 {
            let vol_boost = 10.0 * score.signum();
            score += vol_boost;
            reasons.push("High volume confirmation".into());
        }
    }

    let action = if score > 20.0 {
        "BUY"
    } else if score < -20.0 {
        "SELL"
    } else {
        "HOLD"
    }
    .into();

    let confidence = (score.abs() / 100.0).min(1.0);

    Signal {
        action,
        score: score.clamp(-100.0, 100.0),
        confidence,
        reasons,
    }
}
