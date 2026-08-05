//! Institutional-grade market analysis signals.
//!
//! 1. Gamma Exposure (GEX) — options dealer hedging pressure
//! 2. COT Report — CFTC Commitment of Traders positioning
//! 3. Volume Profile — price levels with highest traded volume
//! 4. Cumulative Volume Delta (CVD) — aggressive buy vs sell pressure
//!
//! These signals reveal what institutional money is doing,
//! giving an edge over pure price-based pattern detection.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{info, warn};

// ══════════════════════════════════════════════════════════════
//  1. GAMMA EXPOSURE (GEX) — Options Market Maker Hedging
// ══════════════════════════════════════════════════════════════
//
// When dealers are LONG gamma → they buy dips, sell rallies → low volatility
// When dealers are SHORT gamma → they chase moves → high volatility
// GEX flip point = price where dealers switch from long to short gamma

/// Gamma exposure tracker — estimates dealer positioning from market data.
#[derive(Debug, Clone, Serialize)]
pub struct GammaExposure {
    /// Estimated GEX level: positive = dealers long gamma (stable),
    /// negative = dealers short gamma (volatile)
    pub gex_level: f64,
    /// GEX regime: "long_gamma" (mean-reverting), "short_gamma" (trending),
    /// or "neutral"
    pub regime: String,
    /// Estimated gamma flip price (above = long gamma, below = short gamma)
    pub flip_price: f64,
    /// Put/Call ratio (>1 = bearish hedging, <1 = bullish)
    pub put_call_ratio: f64,
    /// VIX-based volatility regime
    pub vix_level: f64,
    /// Trading implication
    pub signal: f64, // -1 to +1
}

impl GammaExposure {
    pub fn new() -> Self {
        Self {
            gex_level: 0.0,
            regime: "neutral".to_string(),
            flip_price: 0.0,
            put_call_ratio: 1.0,
            vix_level: 20.0,
            signal: 0.0,
        }
    }
}

/// Estimate GEX from VIX and price action (proxy when options data unavailable).
/// Uses realized volatility ratio + price distance from key levels.
pub fn estimate_gex(
    current_price: f64,
    prices_1d: &[f64],     // Recent daily closes
    atr: f64,
) -> GammaExposure {
    if prices_1d.len() < 10 {
        return GammaExposure::new();
    }

    let n = prices_1d.len();

    // Realized volatility (20-day)
    let returns: Vec<f64> = prices_1d.windows(2)
        .map(|w| (w[1] / w[0]).ln())
        .collect();
    let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance: f64 = returns.iter()
        .map(|r| (r - mean_ret).powi(2))
        .sum::<f64>() / returns.len() as f64;
    let realized_vol = variance.sqrt() * (252.0_f64).sqrt() * 100.0; // Annualized %

    // VIX proxy from realized vol
    let vix_proxy = realized_vol;

    // Estimate gamma regime from volatility clustering
    // Low vol + price near round numbers = dealers long gamma
    // High vol + price moving fast = dealers short gamma
    let recent_vol = if returns.len() >= 5 {
        let recent: f64 = returns.iter().rev().take(5)
            .map(|r| (r - mean_ret).powi(2))
            .sum::<f64>() / 5.0;
        recent.sqrt() * (252.0_f64).sqrt() * 100.0
    } else {
        realized_vol
    };

    let vol_ratio = recent_vol / (realized_vol + 0.01);

    // GEX level estimate
    let gex = if vol_ratio < 0.8 {
        // Low recent vol vs historical = dealers absorbing, long gamma
        (0.8 - vol_ratio) * 2.0
    } else if vol_ratio > 1.3 {
        // High recent vol = dealers chasing, short gamma
        -(vol_ratio - 1.3) * 2.0
    } else {
        0.0
    };

    // Gamma flip price estimate (round number nearest to current)
    let round_interval = if current_price > 200.0 { 10.0 }
        else if current_price > 50.0 { 5.0 }
        else { 1.0 };
    let flip_price = (current_price / round_interval).round() * round_interval;

    // Put/Call ratio proxy from skew
    // If price dropping + vol rising = puts being bought (bearish hedging)
    let price_change_5d = if n >= 5 {
        (prices_1d[n-1] - prices_1d[n-5]) / prices_1d[n-5] * 100.0
    } else { 0.0 };
    let pcr = if price_change_5d < -1.0 && recent_vol > realized_vol {
        1.2 + (-price_change_5d * 0.1) // Rising put buying
    } else if price_change_5d > 1.0 {
        0.8 - (price_change_5d * 0.05) // Call buying
    } else {
        1.0
    };

    let regime = if gex > 0.3 { "long_gamma" }
        else if gex < -0.3 { "short_gamma" }
        else { "neutral" };

    // Trading signal:
    // Long gamma + price near flip = mean revert (sell rallies, buy dips)
    // Short gamma = ride the trend
    let signal = if regime == "long_gamma" {
        // Mean reversion environment — contrarian
        -price_change_5d.signum() * 0.3
    } else if regime == "short_gamma" {
        // Trending environment — follow momentum
        price_change_5d.signum() * 0.5
    } else {
        0.0
    };

    GammaExposure {
        gex_level: gex,
        regime: regime.to_string(),
        flip_price,
        put_call_ratio: pcr.clamp(0.5, 2.0),
        vix_level: vix_proxy,
        signal: signal.clamp(-1.0, 1.0),
    }
}


// ══════════════════════════════════════════════════════════════
//  2. COT REPORT — Commitment of Traders Positioning
// ══════════════════════════════════════════════════════════════
//
// CFTC publishes weekly. Shows:
// - Commercial hedgers (smart money): if they're net long = bullish
// - Large speculators (funds): trend followers
// - Small speculators (retail): usually wrong at extremes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotData {
    pub report_date: String,
    /// Commercial net position (positive = hedgers bullish)
    pub commercial_net: f64,
    /// Large speculator net position (positive = funds bullish)
    pub speculator_net: f64,
    /// Small trader net position
    pub small_trader_net: f64,
    /// Commercial net as % of open interest
    pub commercial_pct: f64,
    /// Signal: positive = follow smart money (bullish), negative = bearish
    pub signal: f64,
    /// Extreme reading flag
    pub extreme: bool,
}

impl CotData {
    pub fn new() -> Self {
        Self {
            report_date: "unknown".to_string(),
            commercial_net: 0.0,
            speculator_net: 0.0,
            small_trader_net: 0.0,
            commercial_pct: 0.0,
            signal: 0.0,
            extreme: false,
        }
    }
}

/// Fetch COT data from CFTC for equity index futures.
pub async fn fetch_cot_data() -> CotData {
    // CFTC WASDE/COT API for S&P 500 E-mini futures
    let url = "https://publicreporting.cftc.gov/resource/6dca-aqc2.json?\
        $where=market_and_exchange_names like '%E-MINI S&P%'&\
        $order=report_date_as_yyyy_mm_dd DESC&\
        $limit=1";

    match reqwest::Client::new()
        .get(url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(data) = resp.json::<Vec<serde_json::Value>>().await {
                if let Some(report) = data.first() {
                    let comm_long = report["comm_positions_long_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let comm_short = report["comm_positions_short_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let spec_long = report["noncomm_positions_long_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let spec_short = report["noncomm_positions_short_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let small_long = report["nonrept_positions_long_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let small_short = report["nonrept_positions_short_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                    let oi = report["open_interest_all"]
                        .as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0);

                    let comm_net = comm_long - comm_short;
                    let spec_net = spec_long - spec_short;
                    let small_net = small_long - small_short;
                    let comm_pct = comm_net / oi * 100.0;

                    let date = report["report_date_as_yyyy_mm_dd"]
                        .as_str().unwrap_or("unknown").to_string();

                    // Signal: follow commercial hedgers (smart money)
                    let signal = (comm_pct / 10.0).clamp(-1.0, 1.0);
                    let extreme = comm_pct.abs() > 15.0;

                    info!("COT: comm_net={:.0} spec_net={:.0} signal={:.2} date={}",
                        comm_net, spec_net, signal, date);

                    return CotData {
                        report_date: date,
                        commercial_net: comm_net,
                        speculator_net: spec_net,
                        small_trader_net: small_net,
                        commercial_pct: comm_pct,
                        signal,
                        extreme,
                    };
                }
            }
        }
        Err(e) => {
            warn!("COT fetch failed: {}", e);
        }
    }

    CotData::new()
}


// ══════════════════════════════════════════════════════════════
//  3. VOLUME PROFILE — Price Level Analysis
// ══════════════════════════════════════════════════════════════
//
// Shows WHERE volume was traded, not just how much.
// POC (Point of Control) = price with most volume = strongest S/R
// Value Area = where 70% of volume traded = fair value range

#[derive(Debug, Clone, Serialize)]
pub struct VolumeProfile {
    /// Point of Control — price with highest volume
    pub poc_price: f64,
    /// Value Area High (70% of volume above this)
    pub va_high: f64,
    /// Value Area Low (70% of volume below this)
    pub va_low: f64,
    /// Is current price above or below POC?
    pub position: String, // "above_poc", "below_poc", "at_poc"
    /// Signal: +1 if at support (buy), -1 if at resistance (sell)
    pub signal: f64,
    /// Volume at each price level
    pub levels: Vec<(f64, f64)>, // (price, volume)
}

impl VolumeProfile {
    pub fn new() -> Self {
        Self {
            poc_price: 0.0,
            va_high: 0.0,
            va_low: 0.0,
            position: "unknown".to_string(),
            signal: 0.0,
            levels: Vec::new(),
        }
    }
}

/// Compute volume profile from OHLCV bar data.
pub fn compute_volume_profile(
    bars: &[serde_json::Value],
    current_price: f64,
    num_levels: usize,
) -> VolumeProfile {
    if bars.is_empty() {
        return VolumeProfile::new();
    }

    // Find price range
    let mut min_price = f64::MAX;
    let mut max_price = f64::MIN;
    for bar in bars {
        let low = bar["low"].as_f64().unwrap_or(f64::MAX);
        let high = bar["high"].as_f64().unwrap_or(f64::MIN);
        min_price = min_price.min(low);
        max_price = max_price.max(high);
    }

    if max_price <= min_price { return VolumeProfile::new(); }

    let level_size = (max_price - min_price) / num_levels as f64;
    let mut volume_at_level: Vec<f64> = vec![0.0; num_levels];
    let mut price_at_level: Vec<f64> = (0..num_levels)
        .map(|i| min_price + (i as f64 + 0.5) * level_size)
        .collect();

    // Distribute volume across price levels
    for bar in bars {
        let high = bar["high"].as_f64().unwrap_or(0.0);
        let low = bar["low"].as_f64().unwrap_or(0.0);
        let close = bar["close"].as_f64().unwrap_or(0.0);
        let volume = bar["volume"].as_f64().unwrap_or(0.0);

        if volume <= 0.0 || high <= low { continue; }

        // Distribute bar's volume across price levels it spans
        let bar_range = high - low;
        for i in 0..num_levels {
            let level_low = min_price + i as f64 * level_size;
            let level_high = level_low + level_size;

            // Overlap between bar range and this level
            let overlap_low = level_low.max(low);
            let overlap_high = level_high.min(high);
            if overlap_high > overlap_low {
                let overlap_pct = (overlap_high - overlap_low) / bar_range;
                // Extra weight for close price level (where most volume actually traded)
                let close_bonus = if close >= level_low && close < level_high { 1.5 } else { 1.0 };
                volume_at_level[i] += volume * overlap_pct * close_bonus;
            }
        }
    }

    // Find POC (level with highest volume)
    let mut poc_idx = 0;
    let mut max_vol = 0.0;
    for (i, &vol) in volume_at_level.iter().enumerate() {
        if vol > max_vol {
            max_vol = vol;
            poc_idx = i;
        }
    }

    let poc_price = price_at_level[poc_idx];

    // Compute Value Area (70% of total volume)
    let total_vol: f64 = volume_at_level.iter().sum();
    let va_target = total_vol * 0.70;

    let mut va_vol = volume_at_level[poc_idx];
    let mut va_low_idx = poc_idx;
    let mut va_high_idx = poc_idx;

    while va_vol < va_target {
        let expand_up = if va_high_idx + 1 < num_levels {
            volume_at_level[va_high_idx + 1]
        } else { 0.0 };
        let expand_down = if va_low_idx > 0 {
            volume_at_level[va_low_idx - 1]
        } else { 0.0 };

        if expand_up >= expand_down && va_high_idx + 1 < num_levels {
            va_high_idx += 1;
            va_vol += expand_up;
        } else if va_low_idx > 0 {
            va_low_idx -= 1;
            va_vol += expand_down;
        } else {
            break;
        }
    }

    let va_high = price_at_level[va_high_idx] + level_size / 2.0;
    let va_low = price_at_level[va_low_idx] - level_size / 2.0;

    // Position and signal
    let position = if current_price > va_high { "above_value" }
        else if current_price < va_low { "below_value" }
        else if (current_price - poc_price).abs() < level_size { "at_poc" }
        else { "in_value" };

    // Signal: price at VA low = support (buy), at VA high = resistance (sell)
    let signal = if current_price < va_low {
        0.5  // Below value = potential buy (oversold)
    } else if current_price > va_high {
        -0.3 // Above value = potential sell (overbought)
    } else {
        // Within value area — closer to POC = neutral
        let dist_from_poc = (current_price - poc_price) / (va_high - va_low + 0.01);
        -dist_from_poc * 0.3
    };

    let levels: Vec<(f64, f64)> = price_at_level.iter().zip(volume_at_level.iter())
        .map(|(&p, &v)| (p, v))
        .collect();

    VolumeProfile {
        poc_price,
        va_high,
        va_low,
        position: position.to_string(),
        signal: signal.clamp(-1.0, 1.0),
        levels,
    }
}


// ══════════════════════════════════════════════════════════════
//  4. CUMULATIVE VOLUME DELTA (CVD) — Buy vs Sell Pressure
// ══════════════════════════════════════════════════════════════
//
// Tracks aggressive buyers vs sellers in real-time.
// Rising CVD = buyers dominating (bullish pressure)
// Falling CVD = sellers dominating (bearish pressure)
// CVD divergence from price = reversal warning

#[derive(Debug, Clone)]
pub struct CvdTracker {
    /// Running cumulative delta
    cvd: f64,
    /// CVD history for trend detection
    cvd_history: VecDeque<f64>,
    /// Last trade price (to classify uptick/downtick)
    last_price: f64,
    /// Buy volume accumulated
    buy_volume: f64,
    /// Sell volume accumulated
    sell_volume: f64,
    /// Tick count
    ticks: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CvdState {
    /// Current cumulative volume delta
    pub cvd: f64,
    /// CVD direction: "accumulating" (buying) or "distributing" (selling)
    pub direction: String,
    /// CVD momentum (rate of change)
    pub cvd_momentum: f64,
    /// Buy/sell ratio
    pub buy_sell_ratio: f64,
    /// Divergence from price (CVD rising but price falling = hidden buying)
    pub divergence: f64,
    /// Signal: +1 strong buy pressure, -1 strong sell pressure
    pub signal: f64,
}

impl CvdTracker {
    pub fn new() -> Self {
        Self {
            cvd: 0.0,
            cvd_history: VecDeque::with_capacity(300),
            last_price: 0.0,
            buy_volume: 0.0,
            sell_volume: 0.0,
            ticks: 0,
        }
    }

    /// Process a trade tick: classify as buy (uptick) or sell (downtick).
    pub fn process_tick(&mut self, price: f64, volume: f64) {
        if self.last_price == 0.0 {
            self.last_price = price;
            return;
        }

        // Tick rule: uptick = buy, downtick = sell, same = previous direction
        let delta = if price > self.last_price {
            volume  // Uptick → aggressive buy
        } else if price < self.last_price {
            -volume // Downtick → aggressive sell
        } else {
            0.0 // No change
        };

        self.cvd += delta;
        if delta > 0.0 { self.buy_volume += volume; }
        else if delta < 0.0 { self.sell_volume += volume; }

        self.last_price = price;
        self.ticks += 1;

        // Store CVD history (every 10 ticks to save memory)
        if self.ticks % 10 == 0 {
            self.cvd_history.push_back(self.cvd);
            if self.cvd_history.len() > 300 {
                self.cvd_history.pop_front();
            }
        }
    }

    /// Get current CVD state with analysis.
    pub fn state(&self, current_price: f64, price_30s_ago: f64) -> CvdState {
        let total_vol = self.buy_volume + self.sell_volume;
        let buy_sell_ratio = if self.sell_volume > 0.0 {
            self.buy_volume / self.sell_volume
        } else { 1.0 };

        // CVD momentum (slope of recent CVD)
        let cvd_momentum = if self.cvd_history.len() >= 10 {
            let recent: f64 = self.cvd_history.iter().rev().take(5).sum::<f64>() / 5.0;
            let older: f64 = self.cvd_history.iter().rev().skip(5).take(5).sum::<f64>() / 5.0;
            recent - older
        } else {
            0.0
        };

        let direction = if cvd_momentum > 0.0 { "accumulating" } else { "distributing" };

        // Divergence: CVD going up but price going down = hidden buying (bullish)
        let price_change = if price_30s_ago > 0.0 {
            (current_price - price_30s_ago) / price_30s_ago
        } else { 0.0 };
        let cvd_change = if total_vol > 0.0 { self.cvd / total_vol } else { 0.0 };

        let divergence = if cvd_change > 0.1 && price_change < -0.001 {
            0.5  // Hidden buying — bullish divergence
        } else if cvd_change < -0.1 && price_change > 0.001 {
            -0.5 // Hidden selling — bearish divergence
        } else {
            0.0
        };

        // Signal based on CVD momentum slope (not cumulative ratio which drifts bullish)
        let momentum_signal = if self.cvd_history.len() >= 20 {
            let recent_avg: f64 = self.cvd_history.iter().rev().take(10).sum::<f64>() / 10.0;
            let older_avg: f64 = self.cvd_history.iter().rev().skip(10).take(10).sum::<f64>() / 10.0;
            let slope = recent_avg - older_avg;
            if total_vol > 0.0 { (slope / total_vol * 10.0).clamp(-1.0, 1.0) } else { 0.0 }
        } else { 0.0 };
        let signal = momentum_signal * 0.7 + divergence * 0.3;

        CvdState {
            cvd: self.cvd,
            direction: direction.to_string(),
            cvd_momentum,
            buy_sell_ratio,
            divergence,
            signal: signal.clamp(-1.0, 1.0),
        }
    }

    /// Reset for new trading day.
    pub fn reset(&mut self) {
        self.cvd = 0.0;
        self.cvd_history.clear();
        self.buy_volume = 0.0;
        self.sell_volume = 0.0;
        self.ticks = 0;
    }
}



