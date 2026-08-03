/// Top 5 symbols we track and trade.
pub const TOP_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

// ══════════════════════════════════════════════════════════════
//  VERSION: claude_1  (2026-07-21)
//  1. Shadow/experiment trades now charge a modeled round-trip cost
//     (spread+slippage) so the A/B scoreboard can't flatter itself.
//  2. exp1 has a PRE-COMMITTED kill criterion (below) — decided before
//     results accumulated, so the goalposts can't move.
//  3. Config freeze: no parameter changes until CONFIG_FREEZE_UNTIL so
//     a clean, comparable week of data can accumulate.
// ══════════════════════════════════════════════════════════════
pub const MODEL_VERSION: &str = "claude_1";

/// Modeled round-trip trading cost for SHADOW/experiment trades, as % of
/// trade value, charged at exit (spread + slippage on liquid megacaps).
/// Without this, fast traders like exp1 look better than reality.
pub const SHADOW_COST_PCT: f64 = 0.04;

/// exp1 kill criterion, pre-committed 2026-07-21: after EXP1_KILL_DAYS days
/// or EXP1_KILL_TRADES closed trades (whichever first), exp1's expectancy per
/// trade AFTER costs must be positive AND beat the random baseline — or exp1
/// is declared dead. No retuning before the deadline.
pub const EXP1_KILL_DAYS: i64 = 14;
pub const EXP1_KILL_TRADES: u32 = 200;

/// VERDICT DELIVERED 2026-07-29: exp1 FAILED its pre-committed criterion.
/// At 325 closed trades (threshold was 200) its expectancy was -$0.256/trade
/// (total -$83.18) versus the random baseline's +$0.65/trade — failing both
/// required conditions (positive expectancy AND beating random).
/// Per the rule agreed BEFORE any results were seen, exp1 is retired rather
/// than retuned. It stops opening new positions; open ones exit normally and
/// its history is preserved for the record.
pub const EXP1_RETIRED: bool = true;

/// Mirror every simulated trade as a real order on the Alpaca PAPER account
/// (fake money, real execution). Purely observational — the internal simulator
/// stays the source of truth for P&L. This measures how optimistic the
/// simulator's assumed fills are, which is the last unmeasured gap in the
/// numbers. Disabled by a hard rail if the endpoint is not paper-api.
pub const ALPACA_SHADOW_ORDERS: bool = true;

/// No config/parameter changes before this date — clean-data window.
pub const CONFIG_FREEZE_UNTIL: &str = "2026-07-28";

// ══════════════════════════════════════════════════════════════
//  PAPER TRADING — v7 SWING MODE (multi-day holds)
//
//  Budget: $500. Goal: profit over ~1 week, not intraday scalping.
//  Why swing: at $500 intraday fees ate 137% of gross. Holding 2-5
//  days cuts trade count ~20x so fees become negligible, and avoids
//  the PDT rule entirely (overnight holds aren't day trades).
//  Few, high-conviction trades. Wide targets. Hold through noise.
// ══════════════════════════════════════════════════════════════

/// Fixed working capital for the day trader. Each trading day starts flat at
/// this amount; at 3:55pm ET everything is liquidated, the day's profit/loss is
/// banked to the profit ledger (reports/daily_profit.jsonl), and capital resets
/// to this amount for the next day. Profit is never compounded — position size
/// stays constant, and losses can't snowball into the next day's risk.
pub const INITIAL_CASH: f64 = 3000.0;

/// The 5-megacap signal trader. Kept running alongside the ETF momentum
/// strategy. Its biggest weakness (long-only losing in down/choppy markets) is
/// now addressed by a market-regime filter (see MARKET_REGIME in state.rs).
pub const SIGNAL_TRADER_ENABLED: bool = true;

/// MAX-EXPOSURE MODE: deploy as much cash as possible instead of leaving it
/// idle. When the market regime is risk-on, fill EVERY position slot at full
/// size, skipping the selectivity gates (per-symbol trend veto, minimum-score
/// threshold, and confidence-based size discount). The evidence pointed here:
/// time-in-market, not clever entry timing, was what actually made money.
/// Risk is contained by the market-regime filter (retreat to all-cash when the
/// broad market turns risk-off), the ATR stops, and the hard stop. This is a
/// higher risk/reward setting than the selective default.
pub const MAX_EXPOSURE_MODE: bool = true;

pub const MAX_POSITION_PCT: f64 = 0.25;

// More selective entries — swing wants conviction, not volume.
pub const MIN_BUY_SIGNAL: f64 = 0.20;
pub const STRONG_BUY_SIGNAL: f64 = 0.35;
pub const SELL_SIGNAL_THRESHOLD: f64 = -0.10;

// Wide, multi-day risk bands. Swings need room to breathe.
// These now act as CLAMP BOUNDS for the ATR-based exits below (fallbacks when
// ATR is unavailable), not fixed thresholds.
pub const TRAILING_STOP_PCT: f64 = 1.5;

pub const HARD_STOP_PCT: f64 = -3.0;

pub const TAKE_PROFIT_PCT: f64 = 4.0;

// ── ATR-based dynamic exits ──────────────────────────────────
// Stops/targets scale with each name's own volatility (Average True Range)
// instead of one-size-fits-all percentages: a volatile stock gets wider stops,
// a calm one tighter. Expressed as multiples of ATR%, then clamped to a sane
// band so a bad ATR reading can't produce an absurd stop.
pub const HARD_STOP_ATR_MULT: f64 = 2.0;    // stop ~2x ATR below entry
pub const TAKE_PROFIT_ATR_MULT: f64 = 3.0;  // target ~3x ATR
pub const TRAIL_ATR_MULT: f64 = 1.5;        // trail ~1.5x ATR off the peak
pub const PARTIAL_ATR_MULT: f64 = 1.5;      // book half at ~1.5x ATR
pub const ATR_PCT_FLOOR: f64 = 0.3;         // treat ATR as >= 0.3% of price

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
