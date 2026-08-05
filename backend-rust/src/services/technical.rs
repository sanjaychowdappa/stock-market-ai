//! Technical analysis indicators — RSI, MACD, Bollinger, SMA, EMA, etc.
//! All implemented from scratch for zero-dependency speed.

use crate::models::Candle;

/// Simple Moving Average over `period` closing prices.
pub fn sma(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period {
        return vec![f64::NAN; candles.len()];
    }
    let mut result = vec![f64::NAN; period - 1];
    let mut sum: f64 = candles[..period].iter().map(|c| c.close).sum();
    result.push(sum / period as f64);
    for i in period..candles.len() {
        sum += candles[i].close - candles[i - period].close;
        result.push(sum / period as f64);
    }
    result
}

/// Exponential Moving Average.
pub fn ema(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.is_empty() || period == 0 {
        return Vec::new();
    }
    let mut result = vec![f64::NAN; prices.len()];
    let k = 2.0 / (period as f64 + 1.0);

    // Seed with SMA of first `period` values
    let first_valid = prices.iter().take(period).copied().filter(|v| v.is_finite()).collect::<Vec<_>>();
    if first_valid.len() < period {
        return result;
    }
    let seed: f64 = first_valid.iter().sum::<f64>() / period as f64;
    result[period - 1] = seed;

    let mut prev = seed;
    for i in period..prices.len() {
        let val = prices[i] * k + prev * (1.0 - k);
        result[i] = val;
        prev = val;
    }
    result
}

/// RSI (Relative Strength Index).
pub fn rsi(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    let mut result = vec![f64::NAN; n];
    if n <= period {
        return result;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    for i in 1..=period {
        let diff = candles[i].close - candles[i - 1].close;
        if diff > 0.0 {
            avg_gain += diff;
        } else {
            avg_loss += diff.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    if avg_loss == 0.0 {
        result[period] = 100.0;
    } else {
        let rs = avg_gain / avg_loss;
        result[period] = 100.0 - 100.0 / (1.0 + rs);
    }

    for i in (period + 1)..n {
        let diff = candles[i].close - candles[i - 1].close;
        let (gain, loss) = if diff > 0.0 { (diff, 0.0) } else { (0.0, diff.abs()) };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        if avg_loss == 0.0 {
            result[i] = 100.0;
        } else {
            let rs = avg_gain / avg_loss;
            result[i] = 100.0 - 100.0 / (1.0 + rs);
        }
    }
    result
}

/// MACD (12, 26, 9). Returns (macd_line, signal_line, histogram).
pub fn macd(candles: &[Candle]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let ema12 = ema(&closes, 12);
    let ema26 = ema(&closes, 26);

    let macd_line: Vec<f64> = ema12
        .iter()
        .zip(ema26.iter())
        .map(|(a, b)| a - b)
        .collect();

    let signal = ema(&macd_line, 9);
    let hist: Vec<f64> = macd_line
        .iter()
        .zip(signal.iter())
        .map(|(m, s)| m - s)
        .collect();

    (macd_line, signal, hist)
}

/// Bollinger Bands (20, 2). Returns (upper, middle, lower).
pub fn bollinger(candles: &[Candle], period: usize, num_std: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mid = sma(candles, period);
    let n = candles.len();
    let mut upper = vec![f64::NAN; n];
    let mut lower = vec![f64::NAN; n];

    for i in (period - 1)..n {
        let mean = mid[i];
        if mean.is_nan() {
            continue;
        }
        let variance: f64 = candles[(i + 1 - period)..=i]
            .iter()
            .map(|c| (c.close - mean).powi(2))
            .sum::<f64>()
            / period as f64;
        let std = variance.sqrt();
        upper[i] = mean + num_std * std;
        lower[i] = mean - num_std * std;
    }
    (upper, mid, lower)
}


/// Micro-momentum: quick RSI + price rate-of-change on last N candles.
pub fn micro_momentum(candles: &[Candle], window: usize) -> f64 {
    if candles.len() < window + 1 {
        return 0.0;
    }
    let n = candles.len();
    let recent = &candles[n - window..];
    let prev = candles[n - window - 1].close;

    // Price momentum
    let price_mom = (recent.last().unwrap().close - prev) / prev;

    // Quick RSI on window
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in 1..recent.len() {
        let diff = recent[i].close - recent[i - 1].close;
        if diff > 0.0 {
            gains += diff;
        } else {
            losses += diff.abs();
        }
    }
    let rsi_val = if losses == 0.0 {
        1.0
    } else {
        let rs = gains / losses;
        (100.0 - 100.0 / (1.0 + rs)) / 100.0
    };

    // Blend: 60% price momentum, 40% RSI centered at 0.5
    0.6 * price_mom + 0.4 * (rsi_val - 0.5)
}
