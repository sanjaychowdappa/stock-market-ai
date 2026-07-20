/// Top 5 symbols we track and trade.
pub const TOP_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

// ══════════════════════════════════════════════════════════════
//  PAPER TRADING — v7 SWING MODE (multi-day holds)
//
//  Budget: $500. Goal: profit over ~1 week, not intraday scalping.
//  Why swing: at $500 intraday fees ate 137% of gross. Holding 2-5
//  days cuts trade count ~20x so fees become negligible, and avoids
//  the PDT rule entirely (overnight holds aren't day trades).
//  Few, high-conviction trades. Wide targets. Hold through noise.
// ══════════════════════════════════════════════════════════════

pub const INITIAL_CASH: f64 = 500.0;

/// The 5-megacap signal trader. Kept running alongside the ETF momentum
/// strategy. Its biggest weakness (long-only losing in down/choppy markets) is
/// now addressed by a market-regime filter (see MARKET_REGIME in state.rs).
pub const SIGNAL_TRADER_ENABLED: bool = true;

pub const MAX_POSITION_PCT: f64 = 0.25;

// More selective entries — swing wants conviction, not volume.
pub const MIN_BUY_SIGNAL: f64 = 0.20;
pub const STRONG_BUY_SIGNAL: f64 = 0.35;
pub const SELL_SIGNAL_THRESHOLD: f64 = -0.10;

// Wide, multi-day risk bands. Swings need room to breathe.
pub const TRAILING_STOP_PCT: f64 = 1.5;

pub const HARD_STOP_PCT: f64 = -3.0;

pub const TAKE_PROFIT_PCT: f64 = 4.0;

// Cooldown is wall-clock: ~3h before re-entering the same symbol.
pub const TRADE_COOLDOWN_SECS: u64 = 10800;

/// Daily circuit breaker — widened for swing: overnight gaps can move a
/// held book >2% legitimately. Stops new entries once the day is down 4%.
pub const DAILY_LOSS_LIMIT_PCT: f64 = -4.0;

/// Partial profit booking: sell half at +2% on a swing, let the rest run.
pub const PARTIAL_PROFIT_PCT: f64 = 2.0;

pub const MAX_CONCURRENT_POSITIONS: usize = 5;

pub const MOMENTUM_WINDOW: usize = 5;

/// Max hold backstop ≈ 5 trading days of market-hour ticks (~1 week).
/// If no price-based exit fires within a week, flatten the position.
pub const FLAT_EXIT_SECS: u64 = 117000;

/// Few trades per day — swing is about quality, not frequency.
pub const MAX_DAILY_TRADES: u32 = 5;

pub const MIN_PREDICTED_MOVE_PCT: f64 = 0.012;

/// Hold at least ~30 min of market time before any non-stop exit, so
/// intraday noise doesn't shake us out of a multi-day thesis.
pub const MIN_HOLD_SECS: u64 = 1800;

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
