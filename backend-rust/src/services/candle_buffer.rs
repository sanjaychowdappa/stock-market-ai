//! Rolling 1-second micro-candle buffer with 1-minute aggregation.

use crate::config::CANDLE_BUFFER_CAP;
use crate::models::Candle;
use std::collections::VecDeque;

/// Accumulates ticks within the current second, then emits a completed candle.
pub struct CandleBuffer {
    candles: VecDeque<Candle>,
    // Current second accumulator
    cur_open: f64,
    cur_high: f64,
    cur_low: f64,
    cur_close: f64,
    cur_volume: f64,
    cur_time: f64,
    tick_count: u32,
}

impl CandleBuffer {
    pub fn new() -> Self {
        Self {
            candles: VecDeque::with_capacity(CANDLE_BUFFER_CAP),
            cur_open: 0.0,
            cur_high: f64::NEG_INFINITY,
            cur_low: f64::INFINITY,
            cur_close: 0.0,
            cur_volume: 0.0,
            cur_time: 0.0,
            tick_count: 0,
        }
    }

    /// Feed a tick. Returns `Some(candle)` if a candle was completed.
    pub fn feed(&mut self, price: f64, volume: f64, time: f64) -> Option<Candle> {
        let time_sec = time.floor();
        if self.tick_count == 0 {
            // First tick ever
            self.cur_open = price;
            self.cur_high = price;
            self.cur_low = price;
            self.cur_close = price;
            self.cur_volume = volume;
            self.cur_time = time_sec;
            self.tick_count = 1;
            return None;
        }

        if time_sec != self.cur_time {
            // New second → close previous candle
            let completed = Candle {
                time: self.cur_time,
                open: self.cur_open,
                high: self.cur_high,
                low: self.cur_low,
                close: self.cur_close,
                volume: self.cur_volume,
            };
            if self.candles.len() >= CANDLE_BUFFER_CAP {
                self.candles.pop_front();
            }
            self.candles.push_back(completed);

            // Reset for new second
            self.cur_open = price;
            self.cur_high = price;
            self.cur_low = price;
            self.cur_close = price;
            self.cur_volume = volume;
            self.cur_time = time_sec;
            self.tick_count = 1;
            return Some(completed);
        }

        // Same second — accumulate
        self.cur_high = self.cur_high.max(price);
        self.cur_low = self.cur_low.min(price);
        self.cur_close = price;
        self.cur_volume += volume;
        self.tick_count += 1;
        None
    }

    /// All completed candles (most recent last).
    pub fn candles(&self) -> &VecDeque<Candle> {
        &self.candles
    }

    /// Last N candles as a slice-like iterator.
    pub fn last_n(&self, n: usize) -> impl Iterator<Item = &Candle> {
        let skip = self.candles.len().saturating_sub(n);
        self.candles.iter().skip(skip)
    }

    pub fn len(&self) -> usize {
        self.candles.len()
    }

    /// Build 1-minute candles from the buffer for Kronos inference.
    pub fn to_1min_candles(&self) -> Vec<Candle> {
        if self.candles.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut open = 0.0;
        let mut high = f64::NEG_INFINITY;
        let mut low = f64::INFINITY;
        let mut close = 0.0;
        let mut vol = 0.0;
        let mut minute_start = 0.0;
        let mut count = 0u32;

        for c in &self.candles {
            let minute = (c.time / 60.0).floor() * 60.0;
            if count == 0 {
                minute_start = minute;
                open = c.open;
                high = c.high;
                low = c.low;
                close = c.close;
                vol = c.volume;
                count = 1;
                continue;
            }
            if minute != minute_start {
                // Emit previous minute
                result.push(Candle {
                    time: minute_start,
                    open,
                    high,
                    low,
                    close,
                    volume: vol,
                });
                minute_start = minute;
                open = c.open;
                high = c.high;
                low = c.low;
                close = c.close;
                vol = c.volume;
                count = 1;
            } else {
                high = high.max(c.high);
                low = low.min(c.low);
                close = c.close;
                vol += c.volume;
                count += 1;
            }
        }

        // Final partial minute
        if count > 0 {
            result.push(Candle {
                time: minute_start,
                open,
                high,
                low,
                close,
                volume: vol,
            });
        }

        result
    }

    /// Compute ATR from last N candles.
    pub fn atr(&self, period: usize) -> f64 {
        let n = self.candles.len();
        if n < 2 {
            return 0.0;
        }
        let start = n.saturating_sub(period + 1);
        let slice: Vec<&Candle> = self.candles.iter().skip(start).collect();

        let mut tr_sum = 0.0;
        let mut count = 0;
        for i in 1..slice.len() {
            let prev_close = slice[i - 1].close;
            let tr = (slice[i].high - slice[i].low)
                .max((slice[i].high - prev_close).abs())
                .max((slice[i].low - prev_close).abs());
            tr_sum += tr;
            count += 1;
        }
        if count == 0 {
            0.0
        } else {
            tr_sum / count as f64
        }
    }
}
