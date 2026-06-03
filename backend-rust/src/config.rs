/// Top 5 symbols we track and trade.
pub const TOP_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

// ══════════════════════════════════════════════════════════════
//  PAPER TRADING — Optimized for real-world profitability
//
//  Previous config: 2143 trades/day, 3s avg hold, $0.003/trade
//  → Spread cost alone: ~$6-20 → net negative.
//
//  New philosophy: FEWER trades, BIGGER moves, LONGER holds.
//  Target: 30-80 trades/day, 60-300s hold, $0.05+ avg profit.
//  At ~$0.05/trade × 50 trades = $2.50/day gross.
//  Spread cost: 50 × $0.01 = $0.50 → net ~$2.00/day.
// ══════════════════════════════════════════════════════════════

pub const INITIAL_CASH: f64 = 100.0;

/// Max % of portfolio in positions at once (keep some cash reserve)
pub const MAX_POSITION_PCT: f64 = 0.80;

/// Only enter when composite score > this.
/// Calibrated to OU sim signal range (~0.02-0.10 typical).
/// Old: 0.01 (too low → 2000 trades). Then 0.15 (too high → 0 trades).
pub const MIN_BUY_SIGNAL: f64 = 0.04;

/// Full-size position above this score
pub const STRONG_BUY_SIGNAL: f64 = 0.08;

/// Exit on bearish flip only if signal drops below this
/// More negative = stays in trade longer through noise
pub const SELL_SIGNAL_THRESHOLD: f64 = -0.06;

/// Trailing stop: exit if price drops this % from peak
/// Wider stop lets winners run through normal volatility
pub const TRAILING_STOP_PCT: f64 = 0.06;

/// Hard stop loss: absolute max loss before forced exit
pub const HARD_STOP_PCT: f64 = -0.15;

/// Take profit: lock in gains at this %
/// Must be large enough to overcome spread (~$0.01/share)
pub const TAKE_PROFIT_PCT: f64 = 0.12;

/// Cooldown between trades on same symbol
/// 20s — Kronos updates every 8s, so 2-3 fresh readings before re-entry
pub const TRADE_COOLDOWN_SECS: u64 = 20;

/// Max concurrent positions — trade up to 3 stocks at once
pub const MAX_CONCURRENT_POSITIONS: usize = 3;

/// Momentum confirmation window (seconds of consistent signal)
pub const MOMENTUM_WINDOW: usize = 5;

/// Time limit: close position if flat after this long
/// 3 minutes gives the trade time to develop
pub const FLAT_EXIT_SECS: u64 = 180;

/// No daily trade limit — keep trading until $150 goal
pub const MAX_DAILY_TRADES: u32 = u32::MAX;

/// Minimum predicted price move % to justify entry
/// Calibrated: OU sim typically predicts 0.002-0.01% moves
pub const MIN_PREDICTED_MOVE_PCT: f64 = 0.003;

/// OU simulator parameters
pub const OU_THETA: f64 = 0.15;
pub const OU_DT: f64 = 1.0;

/// Micro-candle buffer capacity (10 minutes of 1-second candles)
pub const CANDLE_BUFFER_CAP: usize = 600;

/// Kronos inference interval (seconds)
pub const KRONOS_INTERVAL_SECS: u64 = 8;
/// Kronos max age before considered stale
pub const KRONOS_MAX_AGE_SECS: f64 = 30.0;

/// Yahoo Finance data refresh interval (seconds)
pub const YF_REFRESH_SECS: u64 = 5;

/// Daily tracker
pub const EOD_TICK_THRESHOLD: usize = 5000;
pub const SAVE_INTERVAL_SECS: u64 = 300;

/// ONNX model paths
pub const ONNX_DIR: &str = "/app/model_cache/onnx";
pub const ONNX_TOKENIZER_ENCODE: &str = "kronos_tokenizer_encode.onnx";
pub const ONNX_DECODER: &str = "kronos_decoder.onnx";
pub const ONNX_TOKENIZER_DECODE: &str = "kronos_tokenizer_decode.onnx";

/// Redis
pub const REDIS_URL: &str = "redis://redis:6379";
