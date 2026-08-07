/// Fallback universe, used only when the sector-leader agent has never run.
const DEFAULT_SYMBOLS: &[&str] = &["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"];

/// The symbols this process tracks and trades, resolved ONCE at startup.
///
/// Previously a hard-coded list of five names chosen once and never revisited.
/// It is now whatever the sector-leader agent last selected: the best stock in
/// each of the strongest sectors, at most one per sector.
///
/// WHY STARTUP AND NOT CONTINUOUSLY
/// Each symbol spawns a RealtimeEngine with its own Kronos and agent loops
/// against the GPU sidecar, and the Alpaca WebSocket subscribes once when it
/// connects. Rotating the universe mid-session therefore means tearing down
/// engines and resubscribing the socket — a lifecycle refactor, not a config
/// change, and not something to deploy into a live session. Resolving at
/// startup gives daily rotation for free, because this machine restarts daily
/// anyway.
///
/// The first run has no picks and falls back to DEFAULT_SYMBOLS; every run
/// after the agent's first scan uses its selection.
pub static TOP_SYMBOLS: once_cell::sync::Lazy<Vec<String>> = once_cell::sync::Lazy::new(|| {
    let picks = std::fs::read_to_string("/app/reports/sector_leaders_state.json")
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v["holdings"].as_array().map(|a| {
            a.iter().filter_map(|s| s.as_str().map(String::from)).collect::<Vec<_>>()
        }))
        .unwrap_or_default();

    let mut universe = if picks.is_empty() {
        tracing::warn!("[UNIVERSE] sector-leader agent has no picks yet — using default {:?}",
            DEFAULT_SYMBOLS);
        DEFAULT_SYMBOLS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    } else {
        tracing::info!("[UNIVERSE] sector-leader selection: {:?}", picks);
        picks
    };

    // ALWAYS include anything currently held, even if the agent has since
    // dropped that sector.
    //
    // Without this a universe change orphans open positions: no engine is
    // created for them, so no ticks arrive, so their prices freeze at whatever
    // they were at restart. Every price-based exit — trailing stop, hard stop,
    // take-profit — silently stops working, and the damage-control floor goes
    // blind too because total_value() sums stale marks.
    //
    // Observed for real on 2026-08-07: rotating the universe mid-session left
    // AMZN, AAPL, GOOGL and MSFT held with frozen prices and no stop
    // protection. A position you hold must be a position you can see.
    let held: Vec<String> = std::fs::read_to_string("/app/reports/trader_state.json")
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v["positions"].as_object().map(|m| m.keys().cloned().collect()))
        .unwrap_or_default();

    let mut added = Vec::new();
    for h in held {
        if !universe.contains(&h) {
            added.push(h.clone());
            universe.push(h);
        }
    }
    if !added.is_empty() {
        tracing::warn!("[UNIVERSE] also streaming {:?} — held from a previous universe and \
                        they need live prices for their stops to work", added);
    }
    tracing::info!("[UNIVERSE] tracking {} symbols: {:?}", universe.len(), universe);
    universe
});

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
/// Without this, fast traders look better than reality.
pub const SHADOW_COST_PCT: f64 = 0.04;


// ── LIVE SIGNAL TRADER KILL CRITERION ────────────────────────────────────
//
// Pre-committed 2026-08-06, written while the strategy stood at -$44.66 —
// deliberately BEFORE more data arrived, because the pull to move the
// goalposts is strongest exactly when the number is bad. exp1 was retired on
// a rule agreed in advance rather than retuned, and that is the precedent.
//
// Judged on REAL Alpaca round trips, not simulator trades. The first three
// days of trustworthy measurement gave 33.3% win rate on both full sessions,
// with expectancy -$0.68 then -$1.43 per trade while slippage fell to 0.002%
// — so the losses are the trades themselves, not friction.
//
// THE RULE: at LIVE_KILL_TRADES completed real round trips, or after
// LIVE_KILL_DAYS trading days, whichever comes first — if expectancy per trade
// is negative, the intraday signal trader is retired. Not retuned. Not given a
// new threshold. Retired, exactly as exp1 was.
//
// Nothing about the entry rules, the score floor, position sizing, or the exit
// ladder may be changed while this trial runs. A parameter changed mid-trial
// resets the count to zero, because the data before the change describes a
// different system.
pub const LIVE_KILL_ENABLED: bool = true;
pub const LIVE_KILL_TRADES: u32 = 100;
pub const LIVE_KILL_DAYS: i64 = 20;

/// Expectancy per real round trip below which the trader is retired.
/// Zero, not a negative tolerance: a system that loses money on average has
/// no case for continuing, whatever its win rate looks like.
pub const LIVE_KILL_MIN_EXPECTANCY: f64 = 0.0;

/// The date the criterion was fixed. Trading days are counted from here.
pub const LIVE_KILL_START_DATE: &str = "2026-08-06";


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

// DAILY_LOSS_LIMIT_PCT (-4%) removed. Damage control stops real orders at
// -1%, which is strictly tighter, so the breaker could never fire first on
// real money — but it could fire on the simulator, which keeps trading below
// the floor to earn its way back, and would then block the very entries the
// recovery gate needs to observe.


/// One slot per sector the selector covers.
///
/// Was 5, which had nothing to do with the strategy — the sector agent ranks
/// eleven sectors and five of its picks were simply discarded. Raised so the
/// MOMENTUM FILTER decides how many names are held rather than an arbitrary
/// cap: every sector whose leader clears MIN_ABSOLUTE_MOMENTUM gets a slot, so
/// a strong tape holds eleven and a weak one holds two.
///
/// The GPU load this implies was measured before changing it, not assumed. Each
/// symbol costs a Kronos inference every 8s plus an agent call every 3s (~27
/// sidecar calls/min); eleven symbols is ~300/min against five's ~138. With the
/// sidecar sitting at 1.4% CPU and 2.2GB of 15GB, that is not close to a limit.
///
/// Sizing still self-limits: the budget splits across QUALIFYING names, and
/// MAX_POSITION_PCT caps any single one at 25%. Eleven names is ~$272 each and
/// deploys fully; two names is $750 each and deliberately leaves cash idle
/// rather than concentrate.
pub const MAX_CONCURRENT_POSITIONS: usize = 11;


/// Max hold backstop ≈ 5 trading days of market-hour ticks (~1 week).
/// If no price-based exit fires within a week, flatten the position.
pub const FLAT_EXIT_SECS: u64 = 117000;

/// Few trades per day — swing is about quality, not frequency.
pub const MAX_DAILY_TRADES: u32 = 5;

/// Entry cap while MAX_EXPOSURE_MODE is on.
///
/// Max exposure fills every slot at the open, so a cap of 5 would forbid any
/// re-entry for the rest of the session. This allows the initial deployment
/// (one per slot) plus limited redeployment after exits, then stops — a hard
/// backstop against the runaway churn seen on 2026-08-05 (25 orders, 11 of them
/// the same symbol) rather than a tuned parameter.
pub const MAX_DAILY_ENTRIES: u32 = 12;

// ── DAMAGE CONTROL ───────────────────────────────────────────────────────
//
// A hard floor on the day, plus a ratchet so a winning day cannot become a
// losing one. This CANNOT guarantee a non-negative day: a stop fills below the
// price that triggered it, and every exit pays a round trip. It bounds the loss;
// it does not eliminate it.
//
// The floor is deliberately not 0%. A fully invested book is below its starting
// capital the moment it buys — by the spread alone — so a 0% floor would halt
// before any position had a chance to work.
pub const DAMAGE_CONTROL_ENABLED: bool = true;

/// Day P&L (% of the day's starting capital) at which everything is flattened
/// and new entries stop. -1.0% of $3000 = -$30, i.e. a floor at $2970.
pub const CAPITAL_FLOOR_PCT: f64 = -1.0;

/// Once the day's peak P&L reaches this, the floor starts ratcheting upward.
pub const PROFIT_LOCK_TRIGGER_PCT: f64 = 1.0;

/// Most of the peak that may be given back once the lock is armed. With a 1.0%
/// trigger and 0.5% giveback, a day that reaches +1.2% cannot close below +0.7%.
pub const PROFIT_LOCK_GIVEBACK_PCT: f64 = 0.5;

// HALT_RESUME_SECS removed: it belonged to the timed-resume design that the
// recovery gate replaced. Re-engagement is now decided by closed trades and
// cost-adjusted P&L, not by a clock.

/// Resumptions allowed per day. More than one lets a bad session repeat itself.
pub const MAX_RESUMES_PER_DAY: u32 = 1;

// ── RECOVERY GATE ────────────────────────────────────────────────────────
//
// A halt stops REAL orders at Alpaca. The simulator keeps trading, because
// simulated trades cost nothing and are the only way to learn whether the model
// has recovered. Alpaca re-engages when the simulator has demonstrated it — not
// when a timer expires, and not when it has clawed back to any particular
// balance.
//
// The criterion is pre-committed for a reason: "resume when it looks better" is
// re-testing until noise obliges. Requiring several closed trades AND a positive
// cost-adjusted total makes a lucky tick insufficient.
pub const RECOVERY_GATE_ENABLED: bool = true;

/// Minimum observation window after a halt before re-engagement is considered.
/// Long enough that a single favourable tick cannot open the gate.
pub const RECOVERY_MIN_SECS: u64 = 900;

/// Closed round trips required after a halt before Alpaca may re-engage.
///
/// Kept at 0 deliberately. Requiring closed trades DEADLOCKED the gate: under
/// MAX_EXPOSURE_MODE the simulator buys its five slots and holds them to the
/// 3:55pm skim, so on 2026-08-05 it sat at 0/3 for the rest of the session and
/// real money was sidelined for the whole day with no way back in. Recovery is
/// now judged on the simulator's equity change since the halt, which a held
/// book still expresses. Raise this only if entries are also expected to close.
pub const RECOVERY_MIN_TRADES: u32 = 0;

/// Cost-adjusted simulated P&L required over those trades. The simulator books
/// no spread and no slippage, so each recovery trade is charged
/// SHADOW_COST_PCT before it counts — otherwise the gate would be measuring the
/// simulator's optimism rather than the model's recovery.
pub const RECOVERY_MIN_PNL: f64 = 0.0;

/// How long a symbol is ineligible for re-entry after a strategy exit.
///
/// On 2026-08-05 each stop-out was followed by an immediate re-buy of the same
/// symbol at the same price on the same tick, so the stop realized a loss and
/// then rebuilt the identical position. GOOGL stopped out at 12:00, 12:02,
/// 12:03 and 12:09 while falling 6.1%, for -$35.20. Twenty minutes blocks that
/// cascade while still allowing a genuine later re-entry.
pub const ENTRY_COOLDOWN_SECS: u64 = 1200;

pub const MIN_PREDICTED_MOVE_PCT: f64 = 0.012;

/// Minimum composite score required to open a position.
///
/// Entries used to be chosen by ranking candidates and taking the top N for
/// however many slots were free — with no floor at all. On 2026-08-05 that
/// bought NVDA at score -0.048 with Kalman reading bearish, and GOOGL at
/// exactly 0.000 with every layer at 0.00, i.e. on no information whatsoever.
/// The agent monitor recorded filter_rate 0.0% across 972 evaluations: the
/// scoring model rejected nothing it was ever shown.
///
/// This is a sanity floor, not a tuned parameter. Refusing to buy what the
/// model itself rates as neutral-or-worse needs no backtest to justify. If
/// nothing clears the bar, the book holds cash — which is a decision, and the
/// one the system was previously incapable of making.
pub const MIN_ENTRY_SCORE: f64 = 0.05;

/// Whether a candidate's composite score justifies opening a position.
///
/// A named predicate rather than an inline comparison so the rule can be tested
/// against the scores that actually shipped positions on 2026-08-05, instead of
/// being verified by reading the diff and hoping.
pub fn qualifies_for_entry(score: f64) -> bool {
    score >= MIN_ENTRY_SCORE
}

/// Hold at least ~30 min of market time before any non-stop exit, so
/// intraday noise doesn't shake us out of a multi-day thesis.
pub const MIN_HOLD_SECS: u64 = 1800;


/// Micro-candle buffer capacity (10 minutes of 1-second candles)
pub const CANDLE_BUFFER_CAP: usize = 600;

/// Kronos inference interval (seconds)
pub const KRONOS_INTERVAL_SECS: u64 = 8;
/// Kronos max age before considered stale
pub const KRONOS_MAX_AGE_SECS: f64 = 30.0;


/// Daily tracker
pub const EOD_TICK_THRESHOLD: usize = 5000;

/// ONNX model paths
pub const ONNX_DIR: &str = "/app/model_cache/onnx";
pub const ONNX_TOKENIZER_ENCODE: &str = "kronos_tokenizer_encode.onnx";
pub const ONNX_DECODER: &str = "kronos_decoder.onnx";
pub const ONNX_TOKENIZER_DECODE: &str = "kronos_tokenizer_decode.onnx";

// REDIS_URL removed along with the redis service: nothing ever connected to
// it. There is no redis crate in Cargo.toml and no code path that used it —
// the container was running purely to hold a port open.
