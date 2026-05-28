/// Top 5 symbols we track and trade.
pub const TOP_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

/// Paper trading
pub const INITIAL_CASH: f64 = 100.0;
pub const MAX_POSITION_PCT: f64 = 0.90;
pub const MIN_BUY_SIGNAL: f64 = 0.01;
pub const STRONG_BUY_SIGNAL: f64 = 0.05;
pub const SELL_SIGNAL_THRESHOLD: f64 = -0.02;
pub const TRAILING_STOP_PCT: f64 = 0.15;
pub const HARD_STOP_PCT: f64 = -0.3;
pub const TAKE_PROFIT_PCT: f64 = 0.5;
pub const TRADE_COOLDOWN_SECS: u64 = 8;
pub const MOMENTUM_WINDOW: usize = 5;
pub const FLAT_EXIT_SECS: u64 = 30;

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
