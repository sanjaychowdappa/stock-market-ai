/// Top 5 symbols we track and trade.
pub const TOP_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

// ══════════════════════════════════════════════════════════════
//  PAPER TRADING — v6 PREDICTION-DRIVEN MULTI-STOCK
//
//  Budget: $500. Target: $600 in 1-2 days.
//  No artificial trade cap — model decides based on signals.
//  Up to 5 concurrent positions (one per symbol), ~$95 each.
//  If all 5 stocks are bullish, hold all 5 simultaneously.
//  Trades happen when signals say go, stop when signals say no.
// ══════════════════════════════════════════════════════════════

pub const INITIAL_CASH: f64 = 500.0;

pub const MAX_POSITION_PCT: f64 = 0.25;

pub const MIN_BUY_SIGNAL: f64 = 0.15;
pub const STRONG_BUY_SIGNAL: f64 = 0.25;
pub const SELL_SIGNAL_THRESHOLD: f64 = -0.10;

pub const TRAILING_STOP_PCT: f64 = 0.50;

pub const HARD_STOP_PCT: f64 = -1.0;

pub const TAKE_PROFIT_PCT: f64 = 2.0;

pub const TRADE_COOLDOWN_SECS: u64 = 120;

/// Daily circuit breaker: stop opening new positions once the day's loss
/// hits this percentage of the day's starting value. Existing positions
/// still run their normal exits. (Prop-desk standard risk rule.)
pub const DAILY_LOSS_LIMIT_PCT: f64 = -2.0;

/// Partial profit booking: sell half the position at this gain and let
/// the remainder run behind the trailing stop.
pub const PARTIAL_PROFIT_PCT: f64 = 0.5;

pub const MAX_CONCURRENT_POSITIONS: usize = 5;

pub const MOMENTUM_WINDOW: usize = 5;

pub const FLAT_EXIT_SECS: u64 = 900;

pub const MAX_DAILY_TRADES: u32 = 999;

pub const MIN_PREDICTED_MOVE_PCT: f64 = 0.012;

pub const MIN_HOLD_SECS: u64 = 200;

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
