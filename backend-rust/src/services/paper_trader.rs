//! Pattern-driven momentum trader — $100 → $150 challenge.
//!
//! Strategy v3: Kronos sets daily bias, patterns drive trade timing.
//!
//! ENTRY: Pattern signal strong + momentum confirmed + Kronos not opposing
//! EXIT:  Hard stop | Take profit | Momentum exhaustion | Flat timeout
//!
//! Key insight: Kronos predicts daily direction well but oscillates
//! at sub-minute timeframes. Patterns detect real momentum shifts.

use crate::config::*;
use crate::models::position::EntryPrediction;
use crate::models::{Position, Trade};
use chrono::{Datelike, Local, Timelike};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Where the trader persists its state so multi-day swing positions survive
/// restarts (the machine is powered off overnight). Lives in the mounted
/// reports/ volume so it persists on the host.
const STATE_FILE: &str = "/app/reports/trader_state.json";

/// Monotonic stamp for state snapshots. Every save claims the next value; an
/// async save that finds a newer value pending declines to write. This stops a
/// slow pre-skim write from landing after the skim's synchronous write and
/// resurrecting already-banked cash.
static SAVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedShadow {
    model_id: String,
    cash: f64,
    positions: HashMap<String, Position>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    #[serde(default)]
    started_date: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedState {
    cash: f64,
    positions: HashMap<String, Position>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    daily_trades: u32,
    last_trading_date: String,
    day_start_value: f64,
    circuit_breaker_tripped: bool,
    #[serde(default)]
    did_daily_skim: bool,
    shadows: Vec<PersistedShadow>,
    saved_at: String,
}

pub struct PaperTrader {
    cash: f64,
    positions: HashMap<String, Position>,
    trades: VecDeque<Trade>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    daily_trades: u32,
    start_time: Instant,
    cooldowns: HashMap<String, Instant>,
    signal_history: HashMap<String, VecDeque<f64>>,
    market_data: HashMap<String, MarketSnapshot>,
    /// Kronos daily bias per symbol: >0 = bullish day, <0 = bearish day
    /// Updated every Kronos cycle (~8s) but only used as a filter, not trigger
    kronos_daily_bias: HashMap<String, f64>,
    /// Running average of Kronos direction (smoothed, not raw oscillation)
    kronos_bias_ema: HashMap<String, f64>,
    pub market_open: bool,
    pub tx: broadcast::Sender<serde_json::Value>,
    pub last_payload: Option<serde_json::Value>,
    /// Per-layer block counters for monitoring which layers filter most
    layer_blocks: LayerBlockCounters,
    /// Track vetoed entries to check if they were missed opportunities
    veto_log: VecDeque<VetoEntry>,
    last_veto_check: Instant,
    last_save: Instant,
    /// Market-regime flag: true = broad market risk-on (QQQ above its 50-day
    /// average). When false, the signal trader stops opening new longs — its
    /// biggest failure mode was buying into down/choppy markets. Updated by a
    /// background task in state.rs.
    market_risk_on: Arc<AtomicBool>,
    /// Daily circuit breaker state: portfolio value at day start and
    /// whether the -2% loss limit has tripped (blocks new entries only).
    day_start_value: f64,
    last_trading_date: String,
    circuit_breaker_tripped: bool,
    /// True once today's 3:55pm profit-skim + reset has run, so it happens
    /// exactly once per day and no further trading occurs afterwards.
    did_daily_skim: bool,
    /// Market intraday momentum (Gao, Han, Li & Zhou 2018): each symbol's
    /// 9:30 open price and its first-half-hour (9:30–10:00 ET) return %.
    /// The first-half-hour return predicts the last-half-hour return, so we
    /// tilt late-day long entries by it. Empty until 10:00 each day.
    day_open_price: HashMap<String, f64>,
    first_hh_return: HashMap<String, f64>,
    /// Shadow models for A/B testing different layer weights
    shadow_traders: Vec<ShadowTrader>,
}

#[derive(Clone)]
struct VetoEntry {
    symbol: String,
    price_at_veto: f64,
    veto_reason: String,
    score: f64,
    timestamp: Instant,
}

/// Shadow trader: runs a second model with different layer weights on the same
/// market data. Doesn't execute real trades — just tracks what it *would* do
/// and logs results to prediction_accuracy.jsonl with model_id for A/B comparison.
struct ShadowTrader {
    model_id: String,
    cash: f64,
    positions: HashMap<String, Position>,
    realized_pnl: f64,
    total_trades: u32,
    winning_trades: u32,
    daily_trades: u32,
    cooldowns: HashMap<String, Instant>,
    /// Layer weights: [kronos, kalman, pattern, cvd, vp, gex, cot]
    weights: [f64; 7],
    /// Random-entry baseline: ignores all signals, enters by coin flip.
    /// If the signal models can't beat this trader, the layers have no edge.
    is_random: bool,
    /// Max-exposure model: always fully invested — enters any free slot
    /// immediately (no signals, no coin flip, no cooldown). Tests the
    /// hypothesis the weekly data keeps pointing at: time-in-market with
    /// the swing exit ladder is the driver, and every entry gate hurts.
    is_always_in: bool,
    /// exp1: short-horizon prediction trader. Buys when the engine's
    /// next-minute forecast predicts an up-move big enough to clear the
    /// spread; holds ~5 minutes; exits on target/stop/time/prediction-flip.
    is_exp1: bool,
    /// Recent trade log (kept for the dashboard's live exp1 panel).
    trades: VecDeque<Trade>,
    /// Date this model started trading (kill-criterion clock for exp1).
    started_date: String,
    /// Trend filter mode for A/B testing: "fullday", "short", or "off".
    trend_mode: String,
}

impl ShadowTrader {
    fn new(model_id: &str, weights: [f64; 7], trend_mode: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            cash: INITIAL_CASH,
            positions: HashMap::new(),
            realized_pnl: 0.0,
            total_trades: 0,
            winning_trades: 0,
            daily_trades: 0,
            cooldowns: HashMap::new(),
            weights,
            is_random: false,
            is_always_in: false,
            is_exp1: false,
            trades: VecDeque::with_capacity(60),
            started_date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            trend_mode: trend_mode.to_string(),
        }
    }

    fn record_trade(&mut self, t: Trade) {
        if self.trades.len() >= 60 { self.trades.pop_front(); }
        self.trades.push_back(t);
    }

    fn new_random(model_id: &str) -> Self {
        let mut s = Self::new(model_id, [0.0; 7], "off");
        s.is_random = true;
        s
    }

    fn new_always_in(model_id: &str) -> Self {
        let mut s = Self::new(model_id, [0.0; 7], "off");
        s.is_always_in = true;
        s
    }

    fn new_exp1(model_id: &str) -> Self {
        let mut s = Self::new(model_id, [0.0; 7], "off");
        s.is_exp1 = true;
        s
    }

    fn total_value(&self) -> f64 {
        self.cash + self.positions.values().map(|p| p.market_value()).sum::<f64>()
    }
}

/// Tracks how many times each layer blocked an entry — for efficiency analysis.
#[derive(Default, Clone)]
struct LayerBlockCounters {
    kronos_bias: u32,
    kalman_direction: u32,
    pattern_signal: u32,
    kalman_momentum: u32,
    pattern_history: u32,
    cvd_pressure: u32,
    vp_resistance: u32,
    consensus: u32,
    score_too_low: u32,
    total_passed: u32,
}

/// Cached ML agent scores from the Python sidecar (port 8002).
/// When available, these replace the hardcoded if/else scoring.
#[derive(Default, Clone)]
struct CachedAgentScores {
    momentum: f64,
    pattern: f64,
    flow: f64,
    level: f64,
    sentiment: f64,
    meta_score: f64,
    is_trained: bool,
    age_seconds: f64,
}

struct MarketSnapshot {
    price: f64,
    pattern_signal: f64,
    pattern_confidence: f64,
    micro_momentum: f64,
    trend: f64,
    kronos_active: bool,
    kronos_direction: f64,
    // Kalman filter signals
    kalman_momentum: f64,
    kalman_trend_strength: f64,
    kalman_confidence: f64,
    kalman_momentum_building: bool,
    kalman_momentum_fading: bool,
    kalman_direction: String,
    // CVD signals
    cvd_signal: f64,
    cvd_buy_sell_ratio: f64,
    // Institutional signals (updated every 30s from orchestrator)
    gex_signal: f64,
    gex_regime: String,    // "long_gamma", "short_gamma", "neutral"
    vp_signal: f64,
    vp_position: String,   // "above_value", "below_value", "at_poc", "in_value"
    cot_signal: f64,
    session_high: f64,
    session_low: f64,
    // Trend regime filter: price relative to its intraday average.
    // Don't fight the trend — long-only should not buy into downtrends.
    uptrend: bool,        // full-day average reference
    uptrend_short: bool,  // 30-min average reference (for A/B test)
    atr_pct: f64,         // Average True Range as % of price (for ATR exits)
    // ML agent scores from sidecar (None = use hardcoded fallback)
    ml_agent_scores: Option<CachedAgentScores>,
}

impl PaperTrader {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);

        tokio::spawn(async {
            let path = "/app/reports/prediction_accuracy.jsonl";
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
                let marker = json!({
                    "type": "session_start",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "config": {
                        "initial_cash": INITIAL_CASH,
                        "max_concurrent_positions": MAX_CONCURRENT_POSITIONS,
                        "max_daily_trades": MAX_DAILY_TRADES,
                        "min_buy_signal": MIN_BUY_SIGNAL,
                        "take_profit_pct": TAKE_PROFIT_PCT,
                        "hard_stop_pct": HARD_STOP_PCT,
                        "trailing_stop_pct": TRAILING_STOP_PCT,
                    },
                    "experiment": "trend-filter A/B/C — identical weights, only trend gate differs",
                    "shadow_models": [
                        {"id": "trend_fullday", "trend": "fullday",
                         "description": "Live weights + full-day trend filter (current production rule)"},
                        {"id": "trend_30min", "trend": "short",
                         "description": "Live weights + 30-min trend filter — catches reversals faster"},
                        {"id": "trend_off", "trend": "off",
                         "description": "Live weights, NO trend filter — tests whether the filter helps at all"},
                        {"id": "random_baseline", "trend": "off",
                         "description": "Null hypothesis: random coin-flip entries — the bar every model must beat"},
                        {"id": "always_in_max_exposure", "trend": "off",
                         "description": "Max exposure: always fully invested, no entry gates — tests whether time-in-market is the real driver"},
                        {"id": "exp1", "trend": "off",
                         "description": "exp1: short-horizon prediction trader — buys when the next-minute forecast predicts an up-move that clears the spread, ~5-min holds"},
                    ]
                });
                let mut line = serde_json::to_string(&marker).unwrap_or_default();
                line.push('\n');
                let _ = f.write_all(line.as_bytes()).await;
            }
        });

        let mut trader = Self {
            cash: INITIAL_CASH,
            positions: HashMap::new(),
            trades: VecDeque::with_capacity(500),
            realized_pnl: 0.0,
            total_trades: 0,
            winning_trades: 0,
            daily_trades: 0,
            start_time: Instant::now(),
            cooldowns: HashMap::new(),
            signal_history: HashMap::new(),
            market_data: HashMap::new(),
            kronos_daily_bias: HashMap::new(),
            kronos_bias_ema: HashMap::new(),
            market_open: false,
            tx,
            last_payload: None,
            layer_blocks: LayerBlockCounters::default(),
            veto_log: VecDeque::with_capacity(100),
            last_veto_check: Instant::now(),
            last_save: Instant::now(),
            market_risk_on: Arc::new(AtomicBool::new(true)),
            day_start_value: INITIAL_CASH,
            last_trading_date: String::new(),
            circuit_breaker_tripped: false,
            did_daily_skim: false,
            day_open_price: HashMap::new(),
            first_hh_return: HashMap::new(),
            shadow_traders: {
                // TREND-FILTER A/B/C: all use the live production weights,
                // differing ONLY in the trend gate, so end-of-week expectancy
                // isolates the effect of the trend filter.
                let live_w = [0.24, 0.12, 0.06, 0.06, 0.52, 0.0, 0.0];
                vec![
                    ShadowTrader::new("trend_fullday", live_w, "fullday"),
                    ShadowTrader::new("trend_30min", live_w, "short"),
                    ShadowTrader::new("trend_off", live_w, "off"),
                    // Null hypothesis: random entries, no filter.
                    ShadowTrader::new_random("random_baseline"),
                    // Max-exposure hypothesis: always fully invested, exits
                    // do all the risk work. If this beats random, exposure is
                    // the driver and entry timing is irrelevant.
                    ShadowTrader::new_always_in("always_in_max_exposure"),
                    // exp1: short-horizon prediction trader — trades on the
                    // engine's next-minute forecast, ~5-minute holds.
                    ShadowTrader::new_exp1("exp1"),
                ]
            },
        };

        // Restore persisted state so multi-day swing positions survive a
        // restart (the machine is off overnight). The NEW_DAY check on the
        // next tick still resets daily counters if the calendar day changed.
        if let Some(ps) = Self::load_persisted() {
            trader.cash = ps.cash;
            trader.positions = ps.positions;
            trader.realized_pnl = ps.realized_pnl;
            trader.total_trades = ps.total_trades;
            trader.winning_trades = ps.winning_trades;
            trader.daily_trades = ps.daily_trades;
            trader.last_trading_date = ps.last_trading_date;
            trader.day_start_value = ps.day_start_value;
            trader.circuit_breaker_tripped = ps.circuit_breaker_tripped;
            trader.did_daily_skim = ps.did_daily_skim;
            for psh in ps.shadows {
                if let Some(sh) = trader.shadow_traders.iter_mut().find(|s| s.model_id == psh.model_id) {
                    sh.cash = psh.cash;
                    sh.positions = psh.positions;
                    sh.realized_pnl = psh.realized_pnl;
                    sh.total_trades = psh.total_trades;
                    sh.winning_trades = psh.winning_trades;
                    if !psh.started_date.is_empty() { sh.started_date = psh.started_date; }
                }
            }
            info!("[STATE_RESTORE] resumed: cash ${:.2}, {} open positions, realized ${:.2}",
                trader.cash, trader.positions.len(), trader.realized_pnl);
        } else {
            info!("[STATE_RESTORE] no saved state — fresh start at ${:.2}", INITIAL_CASH);
        }

        trader
    }

    /// Load persisted trader state from disk (startup only). None on first run.
    fn load_persisted() -> Option<PersistedState> {
        let content = std::fs::read_to_string(STATE_FILE).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Snapshot live state to disk (non-blocking). Called throttled from tick
    /// and after each realized trade so positions survive overnight restarts.
    fn save_state(&self) {
        let shadows: Vec<PersistedShadow> = self.shadow_traders.iter().map(|s| PersistedShadow {
            model_id: s.model_id.clone(),
            cash: s.cash,
            positions: s.positions.clone(),
            realized_pnl: s.realized_pnl,
            total_trades: s.total_trades,
            winning_trades: s.winning_trades,
            started_date: s.started_date.clone(),
        }).collect();
        let state = PersistedState {
            cash: self.cash,
            positions: self.positions.clone(),
            realized_pnl: self.realized_pnl,
            total_trades: self.total_trades,
            winning_trades: self.winning_trades,
            daily_trades: self.daily_trades,
            last_trading_date: self.last_trading_date.clone(),
            day_start_value: self.day_start_value,
            circuit_breaker_tripped: self.circuit_breaker_tripped,
            did_daily_skim: self.did_daily_skim,
            shadows,
            saved_at: Local::now().to_rfc3339(),
        };
        if let Ok(json_str) = serde_json::to_string(&state) {
            // Stamp this snapshot and refuse to write it if a newer save has
            // been issued in the meantime.
            //
            // Without this, an async save spawned just before the 3:55pm skim
            // can land AFTER the skim's synchronous save and overwrite the
            // clean post-skim state with pre-skim cash. On the next boot the
            // trader then sees drift it already banked and re-banks it — the
            // mechanism behind 08-03 and 08-04 each double-counting a day.
            let seq = SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            tokio::spawn(async move {
                if SAVE_SEQ.load(std::sync::atomic::Ordering::SeqCst) != seq {
                    return; // superseded by a newer save — writing would regress state
                }
                let _ = tokio::fs::write(STATE_FILE, json_str).await;
            });
        }
    }

    /// Persist state SYNCHRONOUSLY. Used at critical moments (the daily skim /
    /// capital reset) where a fire-and-forget async write can be lost if the
    /// process is killed immediately after — which is exactly how a completed
    /// skim once failed to reach disk and let profit compound into the next day.
    fn save_state_sync(&self) {
        let shadows: Vec<PersistedShadow> = self.shadow_traders.iter().map(|s| PersistedShadow {
            model_id: s.model_id.clone(),
            cash: s.cash,
            positions: s.positions.clone(),
            realized_pnl: s.realized_pnl,
            total_trades: s.total_trades,
            winning_trades: s.winning_trades,
            started_date: s.started_date.clone(),
        }).collect();
        let state = PersistedState {
            cash: self.cash,
            positions: self.positions.clone(),
            realized_pnl: self.realized_pnl,
            total_trades: self.total_trades,
            winning_trades: self.winning_trades,
            daily_trades: self.daily_trades,
            last_trading_date: self.last_trading_date.clone(),
            day_start_value: self.day_start_value,
            circuit_breaker_tripped: self.circuit_breaker_tripped,
            did_daily_skim: self.did_daily_skim,
            shadows,
            saved_at: Local::now().to_rfc3339(),
        };
        if let Ok(json_str) = serde_json::to_string(&state) {
            // Claim the newest sequence number so any async save still in flight
            // sees itself as superseded and declines to overwrite this write.
            SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Err(e) = std::fs::write(STATE_FILE, json_str) {
                error!("[STATE] synchronous save failed: {}", e);
            }
        }
    }

    /// Running total of BANKED day P&L from the ledger — the authoritative
    /// "how much has this system actually made" figure. Deliberately NOT the
    /// trader's lifetime realized_pnl, which counts every trade ever and does
    /// not reconcile with the sum of banked days.
    fn ledger_cumulative() -> f64 {
        // SUM the banked days. This previously read the LAST row's stored
        // `cumulative_pnl`, which is only correct if every row was appended in
        // order by a single writer. Two writers (the NEW_DAY carryover recovery
        // and the 3:55pm skim) append independently, and both writes used to be
        // fire-and-forget, so "last row" could be stale or out of order — which
        // is how $72.28 came to be banked twice. A sum cannot drift that way.
        // Rows explicitly marked `reliable: false` are excluded. The rows written
        // before 2026-08-04 double-counted the same dollars (see the quarantine
        // note in the ledger itself) and cannot be reconstructed — guessing which
        // of them were genuine would just be a different fabricated number. They
        // stay in the file for audit; they do not count toward any total.
        Self::ledger_rows().iter()
            .filter(|v| v["reliable"].as_bool() != Some(false))
            .filter_map(|v| v["day_pnl"].as_f64())
            .sum::<f64>()
    }

    /// Every parsed row of the daily profit ledger, in file order.
    fn ledger_rows() -> Vec<serde_json::Value> {
        std::fs::read_to_string("/app/reports/daily_profit.jsonl")
            .map(|c| c.lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .collect())
            .unwrap_or_default()
    }

    /// Append one banked-profit row, synchronously and at most once per
    /// (date, kind).
    ///
    /// Both callers previously did their own `tokio::spawn` append with no
    /// idempotency, so a single date could receive a carryover-recovery row in
    /// the morning AND a skim row at 3:55pm — which is exactly the duplication
    /// that made three dates in this ledger double-count. `kind` distinguishes
    /// the two legitimate entry types so a genuine carryover and a genuine skim
    /// on the same day still both record, but neither can ever record twice.
    fn bank_day(date: &str, kind: &str, day_pnl: f64, extra: serde_json::Value) {
        let already = Self::ledger_rows().iter().any(|v| {
            v["date"].as_str() == Some(date) && v["kind"].as_str() == Some(kind)
        });
        if already {
            warn!("[LEDGER] refusing duplicate {} row for {} — already banked", kind, date);
            return;
        }
        let cumulative = Self::ledger_cumulative() + day_pnl;
        let mut row = json!({
            "date": date,
            "kind": kind,
            "day_pnl": (day_pnl * 100.0).round() / 100.0,
            "cumulative_pnl": (cumulative * 100.0).round() / 100.0,
            "capital": INITIAL_CASH,
            "timestamp": Local::now().to_rfc3339(),
        });
        if let (Some(obj), Some(ex)) = (row.as_object_mut(), extra.as_object()) {
            for (k, v) in ex { obj.insert(k.clone(), v.clone()); }
        }
        // SYNCHRONOUS: the old fire-and-forget spawn could land after the next
        // ledger_cumulative() read, corrupting the running total.
        let mut line = serde_json::to_string(&row).unwrap_or_default();
        line.push('\n');
        use std::io::Write;
        match std::fs::OpenOptions::new().create(true).append(true)
            .open("/app/reports/daily_profit.jsonl")
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()) {
                    error!("[LEDGER] write failed for {} {}: {}", date, kind, e);
                }
            }
            Err(e) => error!("[LEDGER] open failed for {} {}: {}", date, kind, e),
        }
    }

    /// Has the daily profit ledger already recorded this date?
    /// Prevents double-banking when a skim's ledger write succeeded but its
    /// state write was lost.
    fn ledger_has_date(date: &str) -> bool {
        std::fs::read_to_string("/app/reports/daily_profit.jsonl")
            .map(|c| c.lines().any(|l| {
                serde_json::from_str::<serde_json::Value>(l)
                    .ok()
                    .and_then(|v| v["date"].as_str().map(|d| d == date))
                    .unwrap_or(false)
            }))
            .unwrap_or(false)
    }

    /// Check if US stock market is open (9:30 AM – 4:00 PM ET, Mon–Fri).
    fn is_market_open() -> bool {
        let utc_now = chrono::Utc::now();
        let month = utc_now.month();
        let offset_hours: i64 = if month >= 3 && month <= 10 { 4 } else { 5 };
        let et_hour = (utc_now.hour() as i64 - offset_hours).rem_euclid(24) as u32;
        let et_minute = utc_now.minute();
        let weekday = utc_now.weekday().num_days_from_monday();
        if weekday >= 5 { return false; }
        let mins = et_hour * 60 + et_minute;
        mins >= 9 * 60 + 30 && mins < 16 * 60
    }

    pub fn subscribe(&self) -> broadcast::Receiver<serde_json::Value> {
        self.tx.subscribe()
    }

    /// Set the daily Kronos bias for a symbol (called by daily_stock_picker).
    pub fn set_kronos_bias(&mut self, symbol: &str, bias: f64) {
        self.kronos_daily_bias.insert(symbol.to_string(), bias);
        self.kronos_bias_ema.insert(symbol.to_string(), bias);
    }

    /// Set institutional signals for a symbol (GEX regime, VP position, COT).
    /// Called every 30s from the orchestrator.
    pub fn set_institutional_signals(
        &mut self, symbol: &str,
        gex_signal: f64, gex_regime: &str,
        vp_signal: f64, vp_position: &str,
        cot_signal: f64,
    ) {
        let snap = self.market_data.entry(symbol.to_string()).or_insert_with(|| MarketSnapshot {
            price: 0.0, pattern_signal: 0.0, pattern_confidence: 0.0,
            micro_momentum: 0.0, trend: 0.0, kronos_active: false,
            kronos_direction: 0.0, kalman_momentum: 0.0,
            kalman_trend_strength: 0.0, kalman_confidence: 0.0,
            kalman_momentum_building: false, kalman_momentum_fading: false,
            kalman_direction: "neutral".to_string(),
            cvd_signal: 0.0, cvd_buy_sell_ratio: 1.0,
            gex_signal: 0.0, gex_regime: "neutral".to_string(),
            vp_signal: 0.0, vp_position: "unknown".to_string(),
            cot_signal: 0.0, session_high: 0.0, session_low: 0.0,
            uptrend: true, uptrend_short: true, atr_pct: 0.0,
            ml_agent_scores: None,
        });
        snap.gex_signal = gex_signal;
        snap.gex_regime = gex_regime.to_string();
        snap.vp_signal = vp_signal;
        snap.vp_position = vp_position.to_string();
        snap.cot_signal = cot_signal;
    }

    pub fn tick(&mut self, symbol: &str, data: &serde_json::Value) {
        let price = data["current_price"].as_f64().unwrap_or(0.0);
        if price <= 0.0 { return; }

        self.market_open = Self::is_market_open();

        // Extract data
        let pattern_signal = data["pattern"]["signal"].as_f64().unwrap_or(0.0);
        let pattern_confidence = data["pattern"]["confidence"].as_f64().unwrap_or(0.0);
        let pattern_direction = data["pattern"]["direction"].as_str().unwrap_or("neutral");
        let predictions = data["predictions"].as_array();

        let mut pred_30s = price;
        let mut pred_60s = price;
        let mut kronos_active = false;
        if let Some(preds) = predictions {
            for p in preds {
                if p["kronos_price"].as_f64().is_some() { kronos_active = true; }
                match p["seconds_ahead"].as_u64() {
                    Some(30) => pred_30s = p["predicted_price"].as_f64().unwrap_or(price),
                    Some(60) => pred_60s = p["predicted_price"].as_f64().unwrap_or(price),
                    _ => {}
                }
            }
        }

        let micro_mom = pattern_confidence
            * if pattern_direction == "bullish" { 1.0 } else { -1.0 };
        let trend = (pred_30s - price) / price;
        let kronos_direction = (pred_60s - price) / price * 100.0;

        // CVD (Cumulative Volume Delta) — buy vs sell pressure
        let cvd = &data["cvd"];
        let cvd_signal = cvd["signal"].as_f64().unwrap_or(0.0);
        let cvd_direction = cvd["direction"].as_str().unwrap_or("neutral");
        let cvd_buy_sell_ratio = cvd["buy_sell_ratio"].as_f64().unwrap_or(1.0);

        // Kalman filter signals (mathematically optimal state estimation)
        let kalman = &data["kalman"];
        let kalman_momentum = kalman["momentum"].as_f64().unwrap_or(0.0);
        let kalman_trend_strength = kalman["trend_strength"].as_f64().unwrap_or(0.0);
        let kalman_confidence = kalman["confidence"].as_f64().unwrap_or(0.0);
        let kalman_strong_trend = kalman["strong_trend"].as_bool().unwrap_or(false);
        let kalman_momentum_building = kalman["momentum_building"].as_bool().unwrap_or(false);
        let kalman_momentum_fading = kalman["momentum_fading"].as_bool().unwrap_or(false);
        let kalman_direction = kalman["direction"].as_str().unwrap_or("neutral");

        // Update Kronos daily bias with EMA smoothing (alpha=0.05)
        // This smooths out the 8-second oscillations into a stable daily view
        if kronos_active {
            let prev_ema = *self.kronos_bias_ema.get(symbol).unwrap_or(&0.0);
            let alpha = 0.05; // Very slow EMA — takes ~20 readings to shift
            let new_ema = alpha * kronos_direction + (1.0 - alpha) * prev_ema;
            self.kronos_bias_ema.insert(symbol.to_string(), new_ema);
            self.kronos_daily_bias.insert(symbol.to_string(), new_ema);
        }

        // Parse ML agent scores from sidecar (if available)
        let ml_agent_scores = data.get("agent_scores")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .and_then(|scores| {
                let age = data["agent_age_seconds"].as_f64().unwrap_or(999.0);
                if age > 10.0 { return None; } // Stale scores
                let trained = scores["is_trained"].as_bool().unwrap_or(false);
                Some(CachedAgentScores {
                    momentum: scores["momentum"].as_f64().unwrap_or(0.0),
                    pattern: scores["pattern"].as_f64().unwrap_or(0.0),
                    flow: scores["flow"].as_f64().unwrap_or(0.0),
                    level: scores["level"].as_f64().unwrap_or(0.0),
                    sentiment: scores["sentiment"].as_f64().unwrap_or(0.0),
                    meta_score: scores["meta_score"].as_f64().unwrap_or(0.0),
                    is_trained: trained,
                    age_seconds: age,
                })
            });

        // Track session high/low
        let prev_high = self.market_data.get(symbol).map(|d| d.session_high).unwrap_or(price);
        let prev_low = self.market_data.get(symbol).map(|d| d.session_low).unwrap_or(price);

        self.market_data.insert(symbol.to_string(), MarketSnapshot {
            price,
            pattern_signal,
            pattern_confidence,
            micro_momentum: micro_mom,
            trend,
            kronos_active,
            kronos_direction,
            kalman_momentum,
            kalman_trend_strength,
            kalman_confidence,
            kalman_momentum_building,
            kalman_momentum_fading,
            kalman_direction: kalman_direction.to_string(),
            cvd_signal,
            cvd_buy_sell_ratio,
            gex_signal: self.market_data.get(symbol).map(|d| d.gex_signal).unwrap_or(0.0),
            gex_regime: self.market_data.get(symbol).map(|d| d.gex_regime.clone()).unwrap_or("neutral".to_string()),
            vp_signal: self.market_data.get(symbol).map(|d| d.vp_signal).unwrap_or(0.0),
            vp_position: self.market_data.get(symbol).map(|d| d.vp_position.clone()).unwrap_or("unknown".to_string()),
            cot_signal: self.market_data.get(symbol).map(|d| d.cot_signal).unwrap_or(0.0),
            session_high: prev_high.max(price),
            session_low: if prev_low == 0.0 { price } else { prev_low.min(price) },
            // Trend regime: price above its intraday average = uptrend.
            // Fail-open (true) when the engine hasn't computed it yet.
            uptrend: data["trend_filter"]["uptrend"].as_bool().unwrap_or(true),
            uptrend_short: data["trend_filter"]["uptrend_short"].as_bool().unwrap_or(true),
            atr_pct: { let a = data["atr"].as_f64().unwrap_or(0.0); if price > 0.0 { a / price * 100.0 } else { 0.0 } },
            ml_agent_scores,
        });

        // Signal history for momentum confirmation
        let hist = self.signal_history
            .entry(symbol.to_string())
            .or_insert_with(|| VecDeque::with_capacity(20));
        hist.push_back(pattern_signal);
        if hist.len() > 15 { hist.pop_front(); }

        // Update position
        if let Some(pos) = self.positions.get_mut(symbol) {
            pos.update(price);
        }

        if !self.market_open { return; }

        // EOD Liquidation: close all positions at 3:55 PM ET (5 min before close)
        // Prevents holding overnight risk with a $100 account
        let utc_now = chrono::Utc::now();
        let month = utc_now.month();
        let offset_hours: i64 = if month >= 3 && month <= 10 { 4 } else { 5 };
        let et_hour = (utc_now.hour() as i64 - offset_hours).rem_euclid(24) as u32;
        let et_minute = utc_now.minute();
        let et_mins = et_hour * 60 + et_minute;
        // Daily model: flatten EVERY day at 3:55pm ET, bank the day's profit,
        // reset capital to the fixed budget for tomorrow.
        let eod_liquidation = et_mins >= 15 * 60 + 55; // 3:55 PM ET, any day

        // New trading day: reset circuit breaker, daily counters, and the skim flag
        let et_date = (utc_now - chrono::Duration::hours(offset_hours)).date_naive().to_string();
        if self.last_trading_date != et_date {
            let prev_date = self.last_trading_date.clone();
            self.last_trading_date = et_date.clone();

            // ── ENFORCE THE FIXED-CAPITAL INVARIANT ──────────────────────
            // Every day must begin with exactly INITIAL_CASH. The 3:55pm skim
            // normally guarantees that, but it only fires if the machine is
            // running then — and its state write can be lost on shutdown. When
            // that happens, yesterday's positions and profit silently carry
            // over and compound into today's position sizing, which this model
            // explicitly must not do. So re-assert the invariant here.
            let carried = self.total_value();
            let drift = carried - INITIAL_CASH;
            if !self.positions.is_empty() || drift.abs() > 0.01 {
                let syms: Vec<String> = self.positions.keys().cloned().collect();
                for s in &syms { self.sell(s, "CARRYOVER_FLATTEN(missed daily skim)"); }
                let recovered = self.cash - INITIAL_CASH;

                // Bank the drift ONLY when the previous day was never banked.
                //
                // The previous version banked it unconditionally, "attributing to
                // today" whenever the prior day was already in the ledger. That
                // re-banked money the 3:55pm skim had already recorded — and the
                // ledger proves it: 07-31 skim banked $72.28, then 08-03 banked
                // $72.28 again as a carryover; 08-03 skim banked $37.14, then
                // 08-04 banked $37.14 again. Three occurrences, same dollars twice.
                //
                // If the prior day already has a skim row, this drift is a
                // state-restore artifact (a stale async save landing after the
                // skim's synchronous one), not new profit. Reset capital and bank
                // nothing. Only a genuinely unbanked prior day gets a late entry.
                if recovered.abs() > 0.01 {
                    let missed_day = !prev_date.is_empty() && !Self::ledger_has_date(&prev_date);
                    if missed_day {
                        info!("[CAPITAL_RECOVERY] {} skim was missed — banking ${:.2} late", prev_date, recovered);
                        Self::bank_day(&prev_date, "carryover", recovered, json!({
                            "recovered_late": true,
                            "note": "late banking for a day whose 3:55pm skim never ran",
                        }));
                    } else {
                        warn!("[CARRYOVER_IGNORED] ${:.2} drift into {} — {} is already banked, \
                               so this is a state artifact, not new profit. Resetting without banking.",
                            recovered, et_date, prev_date);
                    }
                }
                info!("[CAPITAL_RESET] carried ${:.2} into {} — resetting to ${:.0}",
                    carried, et_date, INITIAL_CASH);
                self.cash = INITIAL_CASH;
            }

            self.day_start_value = INITIAL_CASH;
            self.circuit_breaker_tripped = false;
            self.did_daily_skim = false;
            self.daily_trades = 0;
            for shadow in &mut self.shadow_traders { shadow.daily_trades = 0; }
            self.day_open_price.clear();
            self.first_hh_return.clear();
            info!("[NEW_DAY] {} — capital reset to ${:.2}, circuit breaker reset", et_date, self.day_start_value);
            self.save_state_sync();
        }

        // Market intraday momentum capture (Gao et al. 2018): record the 9:30
        // open price, then lock in the first-half-hour return at 10:00 ET.
        // If the system starts after 10:00 the window is missed and the signal
        // simply stays inactive for the day — no fabricated data.
        if self.market_open && price > 0.0 {
            // Only capture the open *inside* the 9:30–10:00 window. A start
            // after 10:00 leaves day_open_price unset → signal stays inactive,
            // instead of grabbing a late price and fabricating a 0.000% return.
            if et_mins >= 9 * 60 + 30 && et_mins < 10 * 60 && !self.day_open_price.contains_key(symbol) {
                self.day_open_price.insert(symbol.to_string(), price);
            }
            if et_mins >= 10 * 60 && !self.first_hh_return.contains_key(symbol) {
                if let Some(&open) = self.day_open_price.get(symbol) {
                    if open > 0.0 {
                        let hh = (price - open) / open * 100.0;
                        self.first_hh_return.insert(symbol.to_string(), hh);
                        info!("[INTRADAY_MOM] {} first-half-hour return {:.3}% (open ${:.2} → ${:.2})",
                            symbol, hh, open, price);
                    }
                }
            }
        }

        // ── DAILY PROFIT SKIM ──────────────────────────────────────
        // At 3:55pm ET: flatten everything, bank the day's P&L to the profit
        // ledger, and reset working capital to the fixed budget. Runs once/day.
        if eod_liquidation && !self.did_daily_skim {
            // Flatten all real positions.
            let syms: Vec<String> = self.positions.keys().cloned().collect();
            for s in &syms { self.sell(s, "EOD_DAILY_SKIM"); }
            // Flatten shadow positions too (they keep their own running books).
            for shadow in &mut self.shadow_traders {
                let ss: Vec<String> = shadow.positions.keys().cloned().collect();
                for s in &ss {
                    if let Some(pos) = shadow.positions.remove(s) {
                        let cost = pos.market_value() * SHADOW_COST_PCT / 100.0;
                        let pnl = pos.unrealized_pnl() - cost;
                        shadow.cash += pos.market_value() - cost;
                        shadow.realized_pnl += pnl;
                        if pnl > 0.0 { shadow.winning_trades += 1; }
                    }
                }
            }
            // The day's P&L = whatever the flat cash is above/below the budget.
            let day_pnl = self.cash - INITIAL_CASH;
            // True running total of BANKED days. Previously this stored
            // self.realized_pnl (lifetime P&L of every trade), which does not
            // reconcile with the sum of day_pnl — the ledger read $174.11 when
            // the banked days summed to $128.60.
            info!("[DAILY_SKIM] {} — day P&L ${:.2} banked | cumulative ${:.2} | reset to ${:.0}",
                et_date, day_pnl, Self::ledger_cumulative() + day_pnl, INITIAL_CASH);
            Self::bank_day(&et_date, "skim", day_pnl, json!({}));
            // Reset working capital for tomorrow. Profit is "taken out".
            self.cash = INITIAL_CASH;
            self.did_daily_skim = true;
            self.day_start_value = INITIAL_CASH;
            // SYNCHRONOUS: an async write here can be lost if the process is
            // killed right after the skim, which would let the reset vanish and
            // yesterday's profit compound into tomorrow's capital.
            self.save_state_sync();
            return;
        }
        // After the skim, do nothing else for the rest of the day.
        if self.did_daily_skim { return; }

        self.manage_position(symbol);

        // Daily circuit breaker: once the day is down 2%, stop opening new
        // positions (exits keep running). Prevents grinding losses all day
        // when the market regime doesn't match the model.
        let day_pnl_pct = if self.day_start_value > 0.0 {
            (self.total_value() - self.day_start_value) / self.day_start_value * 100.0
        } else { 0.0 };
        if day_pnl_pct <= DAILY_LOSS_LIMIT_PCT && !self.circuit_breaker_tripped {
            self.circuit_breaker_tripped = true;
            info!("[CIRCUIT_BREAKER] Day P&L {:.2}% hit the {:.1}% limit — no new entries until tomorrow",
                day_pnl_pct, DAILY_LOSS_LIMIT_PCT);
        }

        // Don't open new positions in last 10 minutes
        if et_mins >= 15 * 60 + 50 { return; }
        // Signal trader retired (lost to random) — it no longer opens new
        // positions; existing ones exit normally. ETF momentum is primary.
        if SIGNAL_TRADER_ENABLED && !self.circuit_breaker_tripped {
            self.find_best_entry();
        }

        // Check for missed opportunities every 60s
        if self.last_veto_check.elapsed().as_secs() >= 60 {
            self.check_missed_opportunities();
            self.last_veto_check = Instant::now();
        }

        // ── Adopt REAL Alpaca fill prices ────────────────────────────
        // The simulator books a trade at the last observed tick price, which is
        // optimistic. When the mirrored order actually fills, restate the
        // position (or the realized cash) at the true execution price so the two
        // books hold identical cost bases and produce identical P&L, rather than
        // merely similar numbers.
        if ALPACA_SHADOW_ORDERS {
            for c in crate::services::alpaca_broker::drain_corrections() {
                let diff = c.actual_price - c.assumed_price;
                if diff.abs() < 1e-9 { continue; }
                if c.side == "buy" {
                    if let Some(pos) = self.positions.get_mut(&c.symbol) {
                        pos.entry_price = c.actual_price;
                        // Paying more (or less) than assumed changes cash too.
                        self.cash -= diff * c.qty;
                    }
                } else {
                    // Sold: proceeds differ from what was credited at close.
                    self.cash += diff * c.qty;
                    self.realized_pnl += diff * c.qty;
                }
                info!("[FILL_SYNC] {} {} restated ${:.4} -> ${:.4} (cash adj ${:+.4})",
                    c.side, c.symbol, c.assumed_price, c.actual_price,
                    if c.side == "buy" { -diff * c.qty } else { diff * c.qty });
            }
        }

        // Run shadow traders on same market data with different weights
        self.tick_shadow_traders(symbol);

        // Persist state every ~20s so positions/cash survive a restart.
        if self.last_save.elapsed().as_secs() >= 20 {
            self.save_state();
            self.last_save = Instant::now();
        }
    }

    fn manage_position(&mut self, symbol: &str) {
        // Partial profit booking: sell half at the ATR-scaled partial level,
        // let the rest run behind the trailing stop — this captures winners
        // that the full take-profit rarely reaches.
        let book_partial = !MAX_EXPOSURE_MODE && self.positions.get(symbol).map_or(false, |pos| {
            let atr = pos.entry_atr_pct.max(ATR_PCT_FLOOR);
            let partial_lvl = (PARTIAL_ATR_MULT * atr).clamp(1.0, 4.0);
            !pos.partial_taken
                && pos.unrealized_pnl_pct() >= partial_lvl
                && pos.hold_seconds >= MIN_HOLD_SECS
        });
        if book_partial {
            self.sell_partial(symbol, 0.5);
        }

        let should_sell = {
            if let Some(pos) = self.positions.get(symbol) {
                let pnl_pct = pos.unrealized_pnl_pct();
                let drawdown = pos.trailing_drawdown_pct();
                let partial_taken = pos.partial_taken;
                let data = self.market_data.get(symbol);
                let k_fading = data.map(|d| d.kalman_momentum_fading).unwrap_or(false);
                let k_momentum = data.map(|d| d.kalman_momentum).unwrap_or(0.0);
                let cvd_sig = data.map(|d| d.cvd_signal).unwrap_or(0.0);
                let cvd_ratio = data.map(|d| d.cvd_buy_sell_ratio).unwrap_or(1.0);
                let gex_regime = data.map(|d| d.gex_regime.clone()).unwrap_or("neutral".to_string());
                let vp_pos = data.map(|d| d.vp_position.clone()).unwrap_or("unknown".to_string());
                let vp_sig = data.map(|d| d.vp_signal).unwrap_or(0.0);

                // CVD divergence: sellers dominating while we're long
                let cvd_bearish = cvd_sig < -0.4 && cvd_ratio < 0.7;

                // VP resistance: price pushed above value area (overbought)
                let at_resistance = vp_pos == "above_value" && vp_sig < -0.3;

                // GEX regime: in long_gamma, mean reversion kicks in faster
                let gex_mean_revert = gex_regime == "long_gamma" && pnl_pct > 0.03;

                // Minimum hold time gate: most exits require MIN_HOLD_SECS
                // to prevent micro-flips that generate fees without capturing moves.
                // Only HARD_STOP bypasses this — capital protection is always immediate.
                let held_long_enough = pos.hold_seconds >= MIN_HOLD_SECS;

                // === ATR-SCALED EXIT THRESHOLDS ===
                // Each position's stops/targets are sized to its own volatility,
                // then clamped to a sane band. A calm name exits tight; a wild
                // one gets room to breathe instead of a one-size-fits-all stop.
                let atr = pos.entry_atr_pct.max(ATR_PCT_FLOOR);
                let hard_stop_lvl = -(HARD_STOP_ATR_MULT * atr).clamp(1.0, 5.0);
                let take_profit_lvl = (TAKE_PROFIT_ATR_MULT * atr).clamp(2.0, 8.0);
                let trail_lvl = (TRAIL_ATR_MULT * atr).clamp(0.5, 3.0);

                // === EXIT LOGIC — VP-ANCHORED (fixed based on accuracy data) ===
                // VP is 72.6% accurate. Kalman/Pattern fire false bearish constantly.
                // Require VP confirmation for bearish exits, not just noisy layers.

                let vp_bearish = vp_sig < -0.3 || at_resistance;
                let strong_bearish = [
                    vp_bearish,      // VP: proven reliable
                    k_fading && k_momentum.abs() > 0.1, // Kalman: only when clearly fading
                    cvd_bearish,     // CVD: sellers dominating
                ].iter().filter(|&&b| b).count();

                let regime_off = !self.market_risk_on.load(Ordering::Relaxed);

                // === ATR-SCALED EXIT LADDER ===
                // 1. HARD STOP — protect capital, IMMEDIATE (ATR-scaled)
                if pnl_pct <= hard_stop_lvl {
                    Some(format!("HARD_STOP({:.2}%,atr={:.2}%)", pnl_pct, atr))
                }
                // 1b. REGIME EXIT — the broad market turned risk-off. In an
                // aggressive/max-exposure posture, retreat to cash promptly
                // rather than ride a falling market down to the stops.
                else if regime_off && held_long_enough {
                    Some(format!("REGIME_EXIT(risk-off,pnl={:.2}%)", pnl_pct))
                }
                // 2. VP-CONFIRMED BEARISH — disabled in max-exposure mode (hold
                // through noise; only the hard stop / trailing / regime bail).
                else if !MAX_EXPOSURE_MODE && vp_bearish && strong_bearish >= 2 && held_long_enough && pnl_pct < -0.5 {
                    Some(format!("BEARISH_EXIT({}signals,vp_confirmed,pnl={:.2}%)", strong_bearish, pnl_pct))
                }
                // 3. TAKE PROFIT — disabled in max-exposure mode so winners run
                // (they are protected by the trailing stop instead of capped).
                else if !MAX_EXPOSURE_MODE && held_long_enough && pnl_pct >= take_profit_lvl {
                    Some(format!("TAKE_PROFIT({:.2}%,tgt={:.2}%)", pnl_pct, take_profit_lvl))
                }
                // 4. VP RESISTANCE — disabled in max-exposure mode.
                else if !MAX_EXPOSURE_MODE && at_resistance && pnl_pct > 1.0 && held_long_enough {
                    Some(format!("VP_PROFIT_LOCK(pnl={:.2}%)", pnl_pct))
                }
                // 5. TRAILING STOP — protect gains from peak (ATR-scaled). In
                // max-exposure mode it also fires while in profit, so a winner
                // that reverses is locked in (then the cash redeploys instantly).
                else if drawdown <= -trail_lvl && held_long_enough
                    && (pnl_pct < 0.0 || partial_taken || MAX_EXPOSURE_MODE) {
                    Some(format!("TRAIL_STOP({:.2}% from peak,pnl={:.2}%)", drawdown, pnl_pct))
                }
                // 6. FLAT EXIT — max-hold backstop (~1 trading week)
                else if pos.hold_seconds >= FLAT_EXIT_SECS {
                    Some(format!("FLAT_EXIT({}s,{:.2}%)", pos.hold_seconds, pnl_pct))
                }
                else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(reason) = should_sell {
            // Log all layer states at exit for monitoring
            if let Some(pos) = self.positions.get(symbol) {
                let data = self.market_data.get(symbol);
                info!("[EXIT_LAYERS] {} reason={} | pnl={:.4}% hold={}s | \
                    kalman(dir={},mom={:.4},fading={}) pat_sig={:.3} cvd={:.2}(ratio={:.2}) \
                    vp({},{:.2}) gex({},{:.2}) cot={:.2}",
                    symbol, reason,
                    pos.unrealized_pnl_pct(), pos.hold_seconds,
                    data.map(|d| d.kalman_direction.as_str()).unwrap_or("?"),
                    data.map(|d| d.kalman_momentum).unwrap_or(0.0),
                    data.map(|d| d.kalman_momentum_fading).unwrap_or(false),
                    data.map(|d| d.pattern_signal).unwrap_or(0.0),
                    data.map(|d| d.cvd_signal).unwrap_or(0.0),
                    data.map(|d| d.cvd_buy_sell_ratio).unwrap_or(1.0),
                    data.map(|d| d.vp_position.as_str()).unwrap_or("?"),
                    data.map(|d| d.vp_signal).unwrap_or(0.0),
                    data.map(|d| d.gex_regime.as_str()).unwrap_or("?"),
                    data.map(|d| d.gex_signal).unwrap_or(0.0),
                    data.map(|d| d.cot_signal).unwrap_or(0.0),
                );
            }
            self.sell(symbol, &reason);
        }
    }

    /// Share the market-regime flag so a background task can update it.
    pub fn regime_handle(&self) -> Arc<AtomicBool> {
        self.market_risk_on.clone()
    }

    /// The simulator's book as (symbol -> qty, symbol -> price), for mirroring
    /// onto the Alpaca paper account.
    pub fn book_snapshot(&self) -> (HashMap<String, f64>, HashMap<String, f64>) {
        let qty = self.positions.iter().map(|(s, p)| (s.clone(), p.shares)).collect();
        let px = self.market_data.iter().map(|(s, d)| (s.clone(), d.price)).collect();
        (qty, px)
    }

    /// (cash, invested) — used by the agentic_test supervisor.
    pub fn portfolio_snapshot(&self) -> (f64, f64) {
        (self.cash, self.positions.values().map(|p| p.market_value()).sum())
    }

    /// Current market-regime posture.
    pub fn is_risk_on(&self) -> bool {
        self.market_risk_on.load(Ordering::Relaxed)
    }

    /// Longest currently-open hold, in seconds (0 if flat).
    pub fn longest_hold_seconds(&self) -> u64 {
        self.positions.values().map(|p| p.hold_seconds).max().unwrap_or(0)
    }

    /// A/B experiment summary: the real trader plus every shadow model,
    /// with value, trades, win rate, and realized P&L — for /api/experiments.
    pub fn experiments_json(&self) -> serde_json::Value {
        let mut models: Vec<serde_json::Value> = Vec::new();
        models.push(json!({
            "model_id": "REAL_TRADER",
            "kind": "real",
            "description": "The live paper trader (max-exposure, $3000/day, daily profit skim)",
            "portfolio_value": (self.total_value() * 100.0).round() / 100.0,
            "cash": (self.cash * 100.0).round() / 100.0,
            "open_positions": self.positions.len(),
            "total_trades": self.total_trades,
            "win_rate_pct": if self.total_trades > 0 {
                ((self.winning_trades as f64 / self.total_trades as f64) * 10000.0).round() / 100.0
            } else { 0.0 },
            "realized_pnl": (self.realized_pnl * 100.0).round() / 100.0,
        }));
        for s in &self.shadow_traders {
            let desc = if s.is_exp1 {
                "exp1: buys on next-minute forecast (>0.08% predicted), ~5-min holds"
            } else if s.is_random {
                "Random coin-flip entries — the null-hypothesis bar to beat"
            } else if s.is_always_in {
                "Always fully invested, no entry gates"
            } else {
                "Signal-weighted entries with trend-filter variant"
            };
            models.push(json!({
                "model_id": s.model_id,
                "kind": if s.is_exp1 { "experiment" } else { "shadow" },
                "description": desc,
                "portfolio_value": (s.total_value() * 100.0).round() / 100.0,
                "cash": (s.cash * 100.0).round() / 100.0,
                "open_positions": s.positions.len(),
                "total_trades": s.total_trades,
                "win_rate_pct": if s.total_trades > 0 {
                    ((s.winning_trades as f64 / s.total_trades as f64) * 10000.0).round() / 100.0
                } else { 0.0 },
                "realized_pnl": (s.realized_pnl * 100.0).round() / 100.0,
            }));
        }
        // exp1 kill-criterion status (pre-committed 2026-07-21): after
        // EXP1_KILL_DAYS or EXP1_KILL_TRADES closed trades, expectancy after
        // costs must be > 0 AND beat the random baseline, or exp1 is dead.
        let exp1_status = {
            let exp1 = self.shadow_traders.iter().find(|s| s.is_exp1);
            let rand = self.shadow_traders.iter().find(|s| s.is_random);
            match exp1 {
                Some(e) => {
                    let exp = if e.total_trades > 0 { e.realized_pnl / e.total_trades as f64 } else { 0.0 };
                    let rand_exp = rand.map(|r| if r.total_trades > 0 { r.realized_pnl / r.total_trades as f64 } else { 0.0 }).unwrap_or(0.0);
                    let days = chrono::NaiveDate::parse_from_str(&e.started_date, "%Y-%m-%d").ok()
                        .map(|d| (chrono::Local::now().date_naive() - d).num_days()).unwrap_or(0);
                    let due = days >= EXP1_KILL_DAYS || e.total_trades >= EXP1_KILL_TRADES;
                    let verdict = if !due { "in trial" }
                        else if exp > 0.0 && exp > rand_exp { "PASSED — expectancy positive and beats random" }
                        else { "DEAD per pre-committed kill criterion" };
                    json!({
                        "criterion": format!("by {} days or {} trades: expectancy after costs > 0 AND > random baseline", EXP1_KILL_DAYS, EXP1_KILL_TRADES),
                        "started": e.started_date,
                        "days_elapsed": days,
                        "trades": e.total_trades,
                        "expectancy_per_trade": (exp * 10000.0).round() / 10000.0,
                        "random_expectancy": (rand_exp * 10000.0).round() / 10000.0,
                        "verdict": verdict,
                    })
                }
                None => json!(null),
            }
        };
        json!({
            "version": MODEL_VERSION,
            "config_frozen_until": CONFIG_FREEZE_UNTIL,
            "cost_model_pct_round_trip": SHADOW_COST_PCT,
            "note": "All models trade the same live market data in parallel (paper only). Shadow trades are charged a modeled round-trip cost at exit.",
            "exp1_kill_criterion": exp1_status,
            "models": models,
        })
    }

    /// Detailed live view of the exp1 experiment (positions + trade log),
    /// mirroring the legacy trader panel — for /api/exp1.
    pub fn exp1_json(&self) -> serde_json::Value {
        let s = match self.shadow_traders.iter().find(|s| s.is_exp1) {
            Some(s) => s,
            None => return json!({"error": "exp1 not running"}),
        };
        let positions: Vec<serde_json::Value> = s.positions.values().map(|p| json!({
            "symbol": p.symbol,
            "shares": (p.shares * 10000.0).round() / 10000.0,
            "entry_price": (p.entry_price * 100.0).round() / 100.0,
            "current_price": (p.current_price * 100.0).round() / 100.0,
            "value": (p.market_value() * 100.0).round() / 100.0,
            "pnl": (p.unrealized_pnl() * 100.0).round() / 100.0,
            "pnl_pct": (p.unrealized_pnl_pct() * 100.0).round() / 100.0,
            "hold_seconds": p.hold_seconds,
            "entry_time": p.entry_time,
        })).collect();
        let trades: Vec<serde_json::Value> = s.trades.iter().rev().take(30).map(|t| json!({
            "time": t.time, "action": t.action, "symbol": t.symbol,
            "shares": (t.shares * 10000.0).round() / 10000.0,
            "price": (t.price * 100.0).round() / 100.0,
            "value": (t.value * 100.0).round() / 100.0,
            "pnl": t.pnl, "pnl_pct": t.pnl_pct, "reason": t.reason,
            "hold_seconds": t.hold_seconds,
        })).collect();
        let invested: f64 = s.positions.values().map(|p| p.market_value()).sum();
        let days = chrono::NaiveDate::parse_from_str(&s.started_date, "%Y-%m-%d").ok()
            .map(|d| (chrono::Local::now().date_naive() - d).num_days()).unwrap_or(0);
        json!({
            "model_id": s.model_id,
            "version": MODEL_VERSION,
            "cost_model_pct_round_trip": SHADOW_COST_PCT,
            "retired": EXP1_RETIRED,
            "kill_criterion": if EXP1_RETIRED {
                format!("RETIRED 2026-07-29 — FAILED its pre-committed criterion at {} trades (threshold {}): expectancy was negative AND lost to the random baseline. Retired per the rule agreed in advance, not retuned.", s.total_trades, EXP1_KILL_TRADES)
            } else {
                format!("pre-committed: by day {} or trade {}, expectancy after costs must be > 0 and beat random ({}d elapsed, {} trades)", EXP1_KILL_DAYS, EXP1_KILL_TRADES, days, s.total_trades)
            },
            "strategy": "Buys when the next-minute forecast predicts an up-move >0.08% (30s trend agreeing). Exits: +0.4% target / -0.4% stop / prediction flip / 5-min time box. Round-trip cost charged at exit.",
            "portfolio_value": ((s.cash + invested) * 100.0).round() / 100.0,
            "cash": (s.cash * 100.0).round() / 100.0,
            "invested": (invested * 100.0).round() / 100.0,
            "realized_pnl": (s.realized_pnl * 100.0).round() / 100.0,
            "total_trades": s.total_trades,
            "winning_trades": s.winning_trades,
            "win_rate_pct": if s.total_trades > 0 {
                ((s.winning_trades as f64 / s.total_trades as f64) * 10000.0).round() / 100.0
            } else { 0.0 },
            "positions": positions,
            "recent_trades": trades,
        })
    }

    fn find_best_entry(&mut self) {
        // The cap now applies in max-exposure mode too. It was written as
        // `!MAX_EXPOSURE_MODE && ...`, and MAX_EXPOSURE_MODE is true, so the
        // condition was always false and the cap never once applied. That is how
        // 2026-08-05 ran 25 orders — 11 of them GOOGL — against a stated limit of
        // 5. Max exposure needs enough headroom to fill every slot and re-enter
        // legitimately, so the limit lives in MAX_DAILY_ENTRIES rather than
        // disabling the check.
        let cap = if MAX_EXPOSURE_MODE { MAX_DAILY_ENTRIES } else { MAX_DAILY_TRADES };
        if self.daily_trades >= cap {
            if self.daily_trades == cap {
                info!("[TRADE_CAP] {} entries today — no further entries (cap {})",
                    self.daily_trades, cap);
            }
            return;
        }
        if self.positions.len() >= MAX_CONCURRENT_POSITIONS { return; }
        // Market-regime filter: don't open new longs when the broad market is
        // risk-off (QQQ below its 50-day average). This is the single most
        // evidence-backed fix for a long-only intraday system.
        if !self.market_risk_on.load(Ordering::Relaxed) { return; }

        let open_slots_available = MAX_CONCURRENT_POSITIONS.saturating_sub(self.positions.len());
        // Divide available cash evenly across remaining slots so full budget gets deployed
        let per_slot = (self.cash / open_slots_available as f64).min(self.total_value() * MAX_POSITION_PCT);
        if self.cash < 2.0 { return; }

        let mut candidates: Vec<(String, f64, String, usize, EntryPrediction)> = Vec::new();
        let mut vetoes: Vec<(String, f64, f64, &str)> = Vec::new();

        // Late-day window for intraday-momentum tilt (after 2:30pm ET)
        let utc_n = chrono::Utc::now();
        let off_h: i64 = if utc_n.month() >= 3 && utc_n.month() <= 10 { 4 } else { 5 };
        let et_m_now = ((utc_n.hour() as i64 - off_h).rem_euclid(24) as u32) * 60 + utc_n.minute();
        let late_day = et_m_now >= 14 * 60 + 30;

        for (sym, data) in &self.market_data {
            if self.positions.contains_key(sym) { continue; }

            // Re-entry cooldown. This used to be skipped entirely under
            // MAX_EXPOSURE_MODE so a freed slot could refill instantly — but
            // "instantly" meant the same tick, at the same price, in the symbol
            // that had just stopped out. On 2026-08-05 GOOGL was re-bought after
            // every stop while it fell 6.1%, losing $35.20 over four round
            // trips; the stop realized each loss and immediately rebuilt the
            // position. The original 3h cooldown was too blunt for a fully
            // invested book (it would park cash for half the session), so max
            // exposure now uses a short cooldown instead of none.
            let cooldown_secs = if MAX_EXPOSURE_MODE {
                ENTRY_COOLDOWN_SECS
            } else {
                TRADE_COOLDOWN_SECS
            };
            if let Some(cd) = self.cooldowns.get(sym) {
                if cd.elapsed().as_secs() < cooldown_secs { continue; }
            }

            // ══ AGENT-BASED SCORING ══
            let kronos_bias = *self.kronos_daily_bias.get(sym).unwrap_or(&0.0);

            // Skip symbols Kronos daily ranking flags bearish (bias < -0.03%)
            if kronos_bias < -0.03 {
                continue;
            }

            // ── AGENT 1: Kronos Transformer (always hardcoded — it IS the ML model) ──
            let kronos_score = if !data.kronos_active {
                0.0
            } else if kronos_bias > 0.10 {
                1.0
            } else if kronos_bias > 0.03 {
                0.6
            } else if kronos_bias > -0.03 {
                0.1
            } else if kronos_bias > -0.10 {
                -0.4
            } else {
                -1.0
            };

            // Check if ML agent sidecar has fresh scores
            let use_ml = data.ml_agent_scores.as_ref()
                .map(|s| s.is_trained && s.age_seconds < 10.0)
                .unwrap_or(false);

            let (kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score, score, scoring_source);

            if use_ml {
                let ml = data.ml_agent_scores.as_ref().unwrap();
                kalman_score = ml.momentum;
                pattern_score = ml.pattern;
                cvd_score = ml.flow;
                vp_score = ml.level;
                gex_score = ml.sentiment * 0.6;
                cot_score = ml.sentiment * 0.4;
                score = ml.meta_score * 0.75 + kronos_score * 0.25;
                scoring_source = "ML";
            } else {
                // ══ FIXED SCORING — based on Jun 11 accuracy analysis ══

                // ── Kalman: require SUSTAINED momentum, not single-tick noise ──
                // Old: direction alone gave 0.5. Now: need strong trend + building momentum.
                let kalman_sustained = data.kalman_trend_strength > 1.5
                    && data.kalman_momentum_building
                    && !data.kalman_momentum_fading;
                kalman_score = if kalman_sustained && data.kalman_direction == "bullish" {
                    0.4
                } else if kalman_sustained && data.kalman_direction == "bearish" {
                    -0.4
                } else if data.kalman_momentum_fading {
                    -0.2
                } else {
                    0.0 // Neutral unless momentum is clearly sustained
                };

                // ── Pattern: require strong, consistent signals across history ──
                // Old: multiplied raw signal by 3x, catching noise. Now: need majority agreement.
                let hist_consensus = self.signal_history.get(sym).map_or(0.0, |hist| {
                    if hist.len() < 5 { return 0.0; }
                    let recent: Vec<f64> = hist.iter().rev().take(10).cloned().collect();
                    let pos = recent.iter().filter(|&&s| s > 0.02).count() as f64;
                    let neg = recent.iter().filter(|&&s| s < -0.02).count() as f64;
                    let total = recent.len() as f64;
                    // Need 70%+ agreement for a signal
                    if pos / total >= 0.7 { 0.5 }
                    else if neg / total >= 0.7 { -0.5 }
                    else { 0.0 }
                });
                let raw_pattern = data.pattern_signal;
                // Only trust pattern if both raw signal AND history agree
                pattern_score = if raw_pattern > 0.1 && hist_consensus > 0.0 {
                    (raw_pattern * 2.0).min(0.6)
                } else if raw_pattern < -0.1 && hist_consensus < 0.0 {
                    (raw_pattern * 2.0).max(-0.6)
                } else {
                    0.0 // Conflicting or weak — don't trust
                };

                // ── CVD: use momentum slope, not cumulative ratio ──
                // Old: buy_sell_ratio drifts bullish over time. Now: require clear momentum shift.
                let cvd_strong_buy = data.cvd_signal > 0.5 && data.cvd_buy_sell_ratio > 1.3;
                let cvd_strong_sell = data.cvd_signal < -0.3 && data.cvd_buy_sell_ratio < 0.7;
                cvd_score = if cvd_strong_buy { 0.5 }
                    else if cvd_strong_sell { -0.5 }
                    else { 0.0 }; // Neutral unless signal is extreme

                // ── Volume Profile: proven 72.6% accurate — trust it fully ──
                vp_score = if data.vp_position == "above_value" && data.vp_signal < -0.2 { -0.8 }
                    else if data.vp_position == "below_value" && data.vp_signal > 0.1 { 0.7 }
                    else if data.vp_position == "at_poc" { 0.3 }
                    else { data.vp_signal.max(-1.0).min(1.0) * 0.4 };

                // ── GEX ──
                gex_score = if data.gex_regime == "short_gamma" { 0.4 }
                    else if data.gex_regime == "long_gamma" { -0.3 }
                    else { 0.0 };

                // ── COT ── (weekly data, rarely useful intraday)
                cot_score = 0.0;

                // ── Weighted sum — dead layers removed (Jun 22 live diagnosis) ──
                // GEX returns null and COT returns 0 on every symbol, so their
                // 15% weight only diluted scores. Zeroed and proportionally
                // rescaled the live layers (×1/0.85), preserving relative balance:
                // VP 52%, Kronos 24%, Kalman 12%, Pattern 6%, CVD 6%, GEX 0%, COT 0%
                let agent_weights: [f64; 7] = [0.24, 0.12, 0.06, 0.06, 0.52, 0.0, 0.0];
                let agent_scores_arr = [kronos_score, kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score];
                score = agent_weights.iter().zip(agent_scores_arr.iter())
                    .map(|(w, s)| w * s).sum();
                scoring_source = "RULES";
            }

            let agent_weights: [(&str, f64, f64); 7] = [
                ("Kronos",  0.24, kronos_score),
                ("Kalman",  0.12, kalman_score),
                ("Pattern", 0.06, pattern_score),
                ("CVD",     0.06, cvd_score),
                ("VP",      0.52, vp_score),
                ("GEX",     0.0,  gex_score),
                ("COT",     0.0,  cot_score),
            ];

            let bullish_agents: Vec<&str> = agent_weights.iter()
                .filter(|(_, _, s)| *s > 0.1).map(|(n, _, _)| *n).collect();
            let bearish_agents: Vec<&str> = agent_weights.iter()
                .filter(|(_, _, s)| *s < -0.1).map(|(n, _, _)| *n).collect();

            let agent_report = format!(
                "[{}] score={:.3} | K={:.2} Mo={:.2} Pa={:.2} Fl={:.2} Lv={:.2} Gx={:.2} Co={:.2} | \
                 bull=[{}] bear=[{}] | kbias={:.3}%",
                scoring_source, score,
                kronos_score, kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score,
                bullish_agents.join("+"), bearish_agents.join("+"),
                kronos_bias,
            );

            // ── HARD VETO 0: don't fight the trend (regime filter) ──
            // Long-only must not buy into downtrends. Skipped in max-exposure
            // mode — the market-wide regime filter already confirms it's risk-on,
            // and intraday dips are acceptable when the goal is full deployment.
            if !MAX_EXPOSURE_MODE && !data.uptrend {
                if self.layer_blocks.consensus % 50 == 0 {
                    info!("[TREND_VETO] {} — price ${:.2} below intraday avg, standing aside (downtrend)", sym, data.price);
                }
                self.layer_blocks.consensus += 1;
                vetoes.push((sym.clone(), data.price, score, "TREND_VETO"));
                continue;
            }

            // ── HARD VETO 1: Kronos must not be bearish ──
            if kronos_score < -0.1 {
                info!("[KRONOS_VETO] {} — kronos={:.2} bearish, refusing entry: {}", sym, kronos_score, agent_report);
                self.layer_blocks.kronos_bias += 1;
                vetoes.push((sym.clone(), data.price, score, "KRONOS_VETO"));
                continue;
            }

            // ── HARD VETO 2: majority bearish ──
            if bearish_agents.len() >= 4 {
                info!("[VETOED] {} — {}/7 agents bearish: {}", sym, bearish_agents.len(), agent_report);
                self.layer_blocks.consensus += 1;
                vetoes.push((sym.clone(), data.price, score, "CONSENSUS_VETO"));
                continue;
            }

            // ── HARD VETO 3: VP strongly against (proven 72.6% accurate) ──
            if vp_score < -0.5 {
                info!("[VP_VETO] {} — vp={:.2} overbought: {}", sym, vp_score, agent_report);
                self.layer_blocks.consensus += 1;
                vetoes.push((sym.clone(), data.price, score, "VP_VETO"));
                continue;
            }

            // ── MARKET INTRADAY MOMENTUM TILT (Gao et al. 2018) ──
            // Late-day return follows the morning return. Long-only, so a
            // positive first-half-hour is a tailwind and a negative one is a
            // headwind we lean against. Only active after 2:30pm ET, and only
            // when the morning move was meaningful (>0.10%).
            let mom_tilt = if late_day {
                match self.first_hh_return.get(sym) {
                    Some(&hh) if hh > 0.10 => 0.10,
                    Some(&hh) if hh < -0.10 => -0.20,
                    _ => 0.0,
                }
            } else { 0.0 };
            let score = score + mom_tilt;

            // ── MINIMUM SCORE THRESHOLD ── (skipped in max-exposure mode so
            // weak-but-not-vetoed names still fill slots — deploy the cash)
            if !MAX_EXPOSURE_MODE && score <= MIN_BUY_SIGNAL {
                if self.layer_blocks.score_too_low % 50 == 0 {
                    info!("[WEAK] {} — {} mom_tilt={:.2} (need > {:.2})", sym, agent_report, mom_tilt, MIN_BUY_SIGNAL);
                }
                self.layer_blocks.score_too_low += 1;
                continue;
            }
            self.layer_blocks.total_passed += 1;

            let bullish_count = bullish_agents.len();
            let layer_report = agent_report;

            let prediction = EntryPrediction {
                overall_score: score,
                predicted_direction: "bullish".into(),
                kronos_score,
                kalman_score,
                pattern_score,
                cvd_score,
                vp_score,
                gex_score,
                cot_score,
                scoring_source: scoring_source.to_string(),
                bullish_layers: bullish_agents.iter().map(|s| s.to_string()).collect(),
                bearish_layers: bearish_agents.iter().map(|s| s.to_string()).collect(),
            };

            candidates.push((sym.clone(), score, layer_report, bullish_count, prediction));
        }

        // Log vetoes after the borrow of self.market_data ends
        for (sym, price, score, reason) in vetoes {
            self.log_veto(&sym, price, score, reason);
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        let open_slots = MAX_CONCURRENT_POSITIONS.saturating_sub(self.positions.len());
        for (best_sym, score, layer_report, bullish_count, prediction) in candidates.iter().take(open_slots) {
            if !MAX_EXPOSURE_MODE && self.daily_trades >= MAX_DAILY_TRADES { break; }
            let price = self.market_data[best_sym].price;
            let bias = *self.kronos_daily_bias.get(best_sym.as_str()).unwrap_or(&0.0);

            // Sizing: max-exposure mode always uses full slot size (deploy the
            // cash); otherwise scale the bet by signal confidence.
            let confidence_scale = if MAX_EXPOSURE_MODE { 1.0 }
                else if *score >= 0.40 { 1.0 }
                else if *score >= 0.25 { 0.75 }
                else { 0.50 };
            let alloc = (per_slot * confidence_scale).min(self.cash);

            let shares = alloc / price;
            if shares * price >= 1.0 {
                let bias_tag = if bias > 0.05 { "K+" } else if bias > -0.02 { "K~" } else { "K-" };
                info!(
                    "PAPER: BUY {:.4} {} @ ${:.2} = ${:.2} (score={:.3}, conf={:.0}%, bias={:.3}% [{}])",
                    shares, best_sym, price, shares * price, score, confidence_scale * 100.0, bias, bias_tag
                );
                info!("  LAYERS: {}", layer_report);

                let mut pos = Position::new(best_sym.clone(), shares, price,
                    Local::now().format("%H:%M:%S").to_string());
                pos.entry_prediction = Some(prediction.clone());
                pos.entry_atr_pct = self.market_data[best_sym].atr_pct;
                self.positions.insert(best_sym.clone(), pos);

                // Mirror to the Alpaca paper account (observational only).
                if ALPACA_SHADOW_ORDERS {
                    let (s, q, p) = (best_sym.clone(), shares, price);
                    tokio::spawn(async move {
                        crate::services::alpaca_broker::shadow_order(
                            s, q, "buy", p, "ENTRY".to_string()).await;
                    });
                }
                self.cash -= shares * price;

                let trade = Trade {
                    symbol: best_sym.clone(),
                    action: "BUY".into(),
                    shares,
                    price,
                    value: shares * price,
                    pnl: None,
                    pnl_pct: None,
                    reason: format!("ENTRY(s={:.3},{},{}of7)", score, bias_tag, bullish_count),
                    time: Local::now().format("%H:%M:%S").to_string(),
                    hold_seconds: None,
                };
                self.total_trades += 1;
                self.daily_trades += 1;
                if self.trades.len() >= 500 { self.trades.pop_front(); }
                self.trades.push_back(trade);
            }
        }
    }

    fn tick_shadow_traders(&mut self, symbol: &str) {
        let data = match self.market_data.get(symbol) {
            Some(d) => d,
            None => return,
        };
        let price = data.price;
        let kronos_bias = *self.kronos_daily_bias.get(symbol).unwrap_or(&0.0);

        // Compute raw layer scores (same fixed logic as main trader)
        let kronos_score = if !data.kronos_active { 0.0 }
            else if kronos_bias > 0.10 { 1.0 }
            else if kronos_bias > 0.03 { 0.6 }
            else if kronos_bias > -0.03 { 0.1 }
            else if kronos_bias > -0.10 { -0.4 }
            else { -1.0 };

        let kalman_sustained = data.kalman_trend_strength > 1.5
            && data.kalman_momentum_building
            && !data.kalman_momentum_fading;
        let kalman_score = if kalman_sustained && data.kalman_direction == "bullish" { 0.4 }
            else if kalman_sustained && data.kalman_direction == "bearish" { -0.4 }
            else if data.kalman_momentum_fading { -0.2 }
            else { 0.0 };

        let hist_consensus = self.signal_history.get(symbol).map_or(0.0, |hist| {
            if hist.len() < 5 { return 0.0; }
            let recent: Vec<f64> = hist.iter().rev().take(10).cloned().collect();
            let pos = recent.iter().filter(|&&s| s > 0.02).count() as f64;
            let neg = recent.iter().filter(|&&s| s < -0.02).count() as f64;
            let total = recent.len() as f64;
            if pos / total >= 0.7 { 0.5 }
            else if neg / total >= 0.7 { -0.5 }
            else { 0.0 }
        });
        let raw_pattern = data.pattern_signal;
        let pattern_score = if raw_pattern > 0.1 && hist_consensus > 0.0 {
            (raw_pattern * 2.0).min(0.6)
        } else if raw_pattern < -0.1 && hist_consensus < 0.0 {
            (raw_pattern * 2.0).max(-0.6)
        } else { 0.0 };

        let cvd_strong_buy = data.cvd_signal > 0.5 && data.cvd_buy_sell_ratio > 1.3;
        let cvd_strong_sell = data.cvd_signal < -0.3 && data.cvd_buy_sell_ratio < 0.7;
        let cvd_score = if cvd_strong_buy { 0.5 }
            else if cvd_strong_sell { -0.5 }
            else { 0.0 };

        let vp_score = if data.vp_position == "above_value" && data.vp_signal < -0.2 { -0.8 }
            else if data.vp_position == "below_value" && data.vp_signal > 0.1 { 0.7 }
            else if data.vp_position == "at_poc" { 0.3 }
            else { data.vp_signal.max(-1.0).min(1.0) * 0.4 };

        let gex_score = if data.gex_regime == "short_gamma" { 0.4 }
            else if data.gex_regime == "long_gamma" { -0.3 }
            else { 0.0 };

        let cot_score = 0.0;

        let raw_scores = [kronos_score, kalman_score, pattern_score, cvd_score, vp_score, gex_score, cot_score];

        let k_fading = data.kalman_momentum_fading;
        let signal = data.pattern_signal;
        let cvd_sig = data.cvd_signal;
        let cvd_bearish = cvd_sig < -0.4 && data.cvd_buy_sell_ratio < 0.7;
        let at_resistance = data.vp_position == "above_value" && data.vp_signal < -0.3;
        // Short-horizon forecast for exp1: predicted %-change over the next
        // ~60 seconds (Kronos + pattern blend from the engine).
        let pred_next_min = data.kronos_direction;

        for shadow in &mut self.shadow_traders {
            // Update existing positions
            if let Some(pos) = shadow.positions.get_mut(symbol) {
                pos.update(price);

                let pnl_pct = pos.unrealized_pnl_pct();
                let held_long_enough = pos.hold_seconds >= MIN_HOLD_SECS;

                let bearish_count = [k_fading, signal < -0.15, cvd_sig < -0.5,
                    vp_score < -0.5, cvd_bearish, at_resistance]
                    .iter().filter(|&&b| b).count();

                let exit_reason = if shadow.is_exp1 {
                    // exp1: minutes-scale exits — target, stop, prediction
                    // flip, or a 5-minute time box. No long-hold gates.
                    if pnl_pct >= 0.4 {
                        Some(format!("EXP1_TARGET({:.2}%)", pnl_pct))
                    } else if pnl_pct <= -0.4 {
                        Some(format!("EXP1_STOP({:.2}%)", pnl_pct))
                    } else if pred_next_min < -0.05 && pos.hold_seconds >= 60 {
                        Some(format!("EXP1_PRED_FLIP(pred={:.2}%,pnl={:.2}%)", pred_next_min, pnl_pct))
                    } else if pos.hold_seconds >= 300 {
                        Some(format!("EXP1_TIME_EXIT(300s,{:.2}%)", pnl_pct))
                    } else {
                        None
                    }
                } else if pnl_pct <= HARD_STOP_PCT {
                    Some("HARD_STOP".to_string())
                } else if bearish_count >= 2 && pnl_pct < 0.0 {
                    Some(format!("BEARISH_EXIT({}signals,pnl={:.2}%)", bearish_count, pnl_pct))
                } else if held_long_enough && pnl_pct >= TAKE_PROFIT_PCT {
                    Some(format!("TAKE_PROFIT({:.2}%)", pnl_pct))
                } else if bearish_count >= 3 && pnl_pct > 0.0 && held_long_enough {
                    Some(format!("BEARISH_PROFIT_LOCK({}signals)", bearish_count))
                } else if pos.hold_seconds >= FLAT_EXIT_SECS {
                    Some(format!("FLAT_EXIT({}s,{:.2}%)", pos.hold_seconds, pnl_pct))
                } else {
                    None
                };

                if let Some(reason) = exit_reason {
                    // Charge the modeled round-trip cost (spread + slippage)
                    // at exit so shadow results reflect tradable reality.
                    let cost = pos.market_value() * SHADOW_COST_PCT / 100.0;
                    let pnl = pos.unrealized_pnl() - cost;
                    let actual_dir = if pnl >= 0.0 { "bullish" } else { "bearish" };
                    shadow.cash += pos.market_value() - cost;
                    shadow.realized_pnl += pnl;
                    if pnl > 0.0 { shadow.winning_trades += 1; }
                    let sell_rec = Trade {
                        symbol: symbol.to_string(), action: "SELL".into(),
                        shares: pos.shares, price: pos.current_price,
                        value: pos.market_value(), pnl: Some(pnl),
                        pnl_pct: Some(pnl_pct), reason: format!("{} (cost ${:.2})", reason, cost),
                        time: Local::now().format("%H:%M:%S").to_string(),
                        hold_seconds: Some(pos.hold_seconds),
                    };
                    if shadow.trades.len() >= 60 { shadow.trades.pop_front(); }
                    shadow.trades.push_back(sell_rec);

                    let log_entry = json!({
                        "type": "shadow_trade",
                        "model_id": shadow.model_id,
                        "timestamp": Local::now().to_rfc3339(),
                        "action": "SELL",
                        "symbol": symbol,
                        "entry_price": pos.entry_price,
                        "exit_price": pos.current_price,
                        "pnl": pnl,
                        "pnl_pct": pnl_pct,
                        "hold_seconds": pos.hold_seconds,
                        "exit_reason": reason,
                        "actual_direction": actual_dir,
                        "portfolio_value": shadow.total_value(),
                        "realized_pnl": shadow.realized_pnl,
                        "total_trades": shadow.total_trades,
                        "win_rate": if shadow.total_trades > 0 {
                            shadow.winning_trades as f64 / shadow.total_trades as f64 * 100.0
                        } else { 0.0 },
                    });

                    tokio::spawn(async move {
                        let path = "/app/reports/prediction_accuracy.jsonl";
                        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
                            let mut line = serde_json::to_string(&log_entry).unwrap_or_default();
                            line.push('\n');
                            let _ = f.write_all(line.as_bytes()).await;
                        }
                    });

                    shadow.positions.remove(symbol);
                    shadow.cooldowns.insert(symbol.to_string(), Instant::now());
                    continue;
                }
            }

            // Try to enter
            if shadow.positions.contains_key(symbol) { continue; }
            if shadow.positions.len() >= MAX_CONCURRENT_POSITIONS { continue; }
            // exp1 is a fast trader — the 5/day cap would strangle it.
            if !shadow.is_exp1 && shadow.daily_trades >= MAX_DAILY_TRADES { continue; }
            // Cooldowns: always_in = none; exp1 = 90s breather; others = 3h.
            let cd_secs = if shadow.is_always_in { 0 }
                else if shadow.is_exp1 { 90 }
                else { TRADE_COOLDOWN_SECS };
            if cd_secs > 0 {
                if let Some(cd) = shadow.cooldowns.get(symbol) {
                    if cd.elapsed().as_secs() < cd_secs { continue; }
                }
            }

            let weighted_score: f64 = shadow.weights.iter()
                .zip(raw_scores.iter())
                .map(|(w, s)| w * s)
                .sum();

            // Trend gate per this shadow's mode — the only thing that differs
            // between the trend_fullday / trend_30min / trend_off models.
            let trend_ok = match shadow.trend_mode.as_str() {
                "fullday" => data.uptrend,
                "short" => data.uptrend_short,
                _ => true, // "off"
            };

            // Random baseline ignores every signal: ~0.1% chance per tick,
            // which lands near the real trader's daily trade count once
            // cooldowns and position limits apply. Always-in enters any free
            // slot immediately — maximum time-in-market by construction.
            let should_enter = if shadow.is_random {
                rand::random::<f64>() < 0.001
            } else if shadow.is_always_in {
                true
            } else if shadow.is_exp1 {
                // exp1 RETIRED 2026-07-29 — failed its pre-committed criterion
                // (325 trades, -$0.256/trade vs random +$0.65). No new entries;
                // open positions still exit normally through the exit ladder.
                if EXP1_RETIRED { false } else {
                    // Original rule: enter when the next-minute forecast predicts
                    // an up-move big enough to clear a typical spread (~0.08%),
                    // with the 30s trend agreeing.
                    pred_next_min > 0.08 && data.trend > 0.0
                }
            } else {
                weighted_score > MIN_BUY_SIGNAL && kronos_score >= -0.1 && trend_ok
            };

            if should_enter {
                let shadow_open = MAX_CONCURRENT_POSITIONS.saturating_sub(shadow.positions.len());
                let per_slot = (shadow.cash / shadow_open as f64).min(shadow.total_value() * MAX_POSITION_PCT);
                let shadow_conf = if shadow.is_always_in || shadow.is_exp1 { 1.0 } // full slot
                    else if weighted_score >= 0.40 { 1.0 }
                    else if weighted_score >= 0.25 { 0.75 }
                    else { 0.50 };
                let alloc = (per_slot * shadow_conf).min(shadow.cash);
                let shares = alloc / price;
                if shares * price >= 1.0 {
                    shadow.cash -= shares * price;
                    shadow.total_trades += 1;
                    shadow.daily_trades += 1;
                    let mut pos = Position::new(symbol.to_string(), shares, price,
                        Local::now().format("%H:%M:%S").to_string());
                    pos.entry_prediction = Some(EntryPrediction {
                        overall_score: weighted_score,
                        predicted_direction: "bullish".into(),
                        kronos_score, kalman_score, pattern_score, cvd_score,
                        vp_score, gex_score, cot_score,
                        scoring_source: "SHADOW".to_string(),
                        bullish_layers: vec![], bearish_layers: vec![],
                    });
                    shadow.positions.insert(symbol.to_string(), pos);

                    let model_id = shadow.model_id.clone();
                    let log_entry = json!({
                        "type": "shadow_trade",
                        "model_id": model_id,
                        "timestamp": Local::now().to_rfc3339(),
                        "action": "BUY",
                        "symbol": symbol,
                        "price": price,
                        "shares": shares,
                        "score": weighted_score,
                        "weights": shadow.weights,
                        "layer_scores": {
                            "kronos": kronos_score, "kalman": kalman_score,
                            "pattern": pattern_score, "cvd": cvd_score,
                            "vp": vp_score, "gex": gex_score, "cot": cot_score,
                        },
                    });
                    tokio::spawn(async move {
                        let path = "/app/reports/prediction_accuracy.jsonl";
                        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
                            let mut line = serde_json::to_string(&log_entry).unwrap_or_default();
                            line.push('\n');
                            let _ = f.write_all(line.as_bytes()).await;
                        }
                    });
                    let buy_rec = Trade {
                        symbol: symbol.to_string(), action: "BUY".into(),
                        shares, price, value: shares * price,
                        pnl: None, pnl_pct: None,
                        reason: if shadow.is_exp1 {
                            format!("EXP1_ENTRY(pred={:+.2}%)", pred_next_min)
                        } else { format!("ENTRY(s={:.3})", weighted_score) },
                        time: Local::now().format("%H:%M:%S").to_string(),
                        hold_seconds: None,
                    };
                    shadow.record_trade(buy_rec);
                }
            }
        }
    }

    /// Sell a fraction of the position at market, keep the rest running.
    /// Locks in profit without giving up the trade's remaining upside.
    fn sell_partial(&mut self, symbol: &str, fraction: f64) {
        let (sold_shares, price, sold_value, sold_pnl, pnl_pct) = {
            let pos = match self.positions.get_mut(symbol) {
                Some(p) => p,
                None => return,
            };
            let sold_shares = pos.shares * fraction;
            let sold_value = sold_shares * pos.current_price;
            let sold_pnl = (pos.current_price - pos.entry_price) * sold_shares;
            let pnl_pct = pos.unrealized_pnl_pct();
            pos.shares -= sold_shares;
            pos.partial_taken = true;
            (sold_shares, pos.current_price, sold_value, sold_pnl, pnl_pct)
        };

        self.cash += sold_value;
        self.realized_pnl += sold_pnl;
        // Not counted as a winning trade — the final close decides that.

        info!(
            "PAPER: PARTIAL SELL {:.4} {} @ ${:.2} PnL: ${:.4} ({:.3}%) — half booked, runner trails",
            sold_shares, symbol, price, sold_pnl, pnl_pct
        );

        let trade = Trade {
            symbol: symbol.to_string(),
            action: "SELL".into(),
            shares: sold_shares,
            price,
            value: sold_value,
            pnl: Some(sold_pnl),
            pnl_pct: Some(pnl_pct),
            reason: format!("PARTIAL_PROFIT(+{:.2}%,half)", pnl_pct),
            time: Local::now().format("%H:%M:%S").to_string(),
            hold_seconds: None,
        };
        if self.trades.len() >= 500 { self.trades.pop_front(); }
        self.trades.push_back(trade);
    }

    fn sell(&mut self, symbol: &str, reason: &str) {
        let pos = match self.positions.remove(symbol) {
            Some(p) => p,
            None => return,
        };

        let value = pos.market_value();
        let pnl = pos.unrealized_pnl();
        let pnl_pct = pos.unrealized_pnl_pct();

        self.cash += value;
        self.realized_pnl += pnl;
        if pnl > 0.0 { self.winning_trades += 1; }

        let pnl_tag = if pnl >= 0.0 { "WIN" } else { "LOSS" };
        info!(
            "PAPER: SELL {:.4} {} @ ${:.2} PnL: ${:.4} ({:.3}%) [{}] held {}s — {}",
            pos.shares, symbol, pos.current_price, pnl, pnl_pct,
            pnl_tag, pos.hold_seconds, reason
        );

        // === PREDICTION ACCURACY LOG ===
        if let Some(pred) = &pos.entry_prediction {
            let actual_direction = if pnl >= 0.0 { "bullish" } else { "bearish" };
            let correct = pred.predicted_direction == actual_direction;
            let accuracy_tag = if correct { "CORRECT" } else { "WRONG" };

            let mut wrong_layers = Vec::new();
            let mut right_layers = Vec::new();
            let layer_checks: &[(&str, f64)] = &[
                ("Kronos", pred.kronos_score),
                ("Kalman", pred.kalman_score),
                ("Pattern", pred.pattern_score),
                ("CVD", pred.cvd_score),
                ("VP", pred.vp_score),
                ("GEX", pred.gex_score),
                ("COT", pred.cot_score),
            ];
            for (name, score) in layer_checks {
                let layer_predicted_bull = *score > 0.05;
                let was_actually_bull = pnl >= 0.0;
                if *score > 0.05 || *score < -0.05 {
                    if layer_predicted_bull == was_actually_bull {
                        right_layers.push(format!("{}({:.2})", name, score));
                    } else {
                        wrong_layers.push(format!("{}({:.2})", name, score));
                    }
                }
            }

            info!(
                "PREDICTION: [{}] {} predicted={} actual={} | score={:.3} src={} | \
                 right=[{}] wrong=[{}] | entry=${:.2} exit=${:.2} pnl=${:.4} held={}s",
                accuracy_tag, symbol, pred.predicted_direction, actual_direction,
                pred.overall_score, pred.scoring_source,
                right_layers.join(","), wrong_layers.join(","),
                pos.entry_price, pos.current_price, pnl, pos.hold_seconds
            );

            let log_entry = json!({
                "timestamp": Local::now().to_rfc3339(),
                "symbol": symbol,
                "predicted_direction": pred.predicted_direction,
                "actual_direction": actual_direction,
                "correct": correct,
                "entry_score": pred.overall_score,
                "scoring_source": pred.scoring_source,
                "layer_scores": {
                    "kronos": pred.kronos_score,
                    "kalman": pred.kalman_score,
                    "pattern": pred.pattern_score,
                    "cvd": pred.cvd_score,
                    "vp": pred.vp_score,
                    "gex": pred.gex_score,
                    "cot": pred.cot_score,
                },
                "bullish_layers": pred.bullish_layers,
                "bearish_layers": pred.bearish_layers,
                "right_layers": right_layers,
                "wrong_layers": wrong_layers,
                "entry_price": pos.entry_price,
                "exit_price": pos.current_price,
                "pnl": pnl,
                "pnl_pct": pnl_pct,
                "hold_seconds": pos.hold_seconds,
                "exit_reason": reason,
            });

            tokio::spawn(async move {
                let path = "/app/reports/prediction_accuracy.jsonl";
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
                    let mut line = serde_json::to_string(&log_entry).unwrap_or_default();
                    line.push('\n');
                    let _ = f.write_all(line.as_bytes()).await;
                }
            });
        }

        // Log trade to ML agent sidecar for training data collection
        if let Some(data) = self.market_data.get(symbol) {
            let features = json!({
                "momentum": [data.kalman_momentum, data.kalman_trend_strength,
                    data.kalman_confidence, data.micro_momentum,
                    if data.kalman_momentum_building { 1.0 } else { 0.0 },
                    if data.kalman_momentum_fading { 1.0 } else { 0.0 }],
                "pattern": [data.pattern_signal, data.pattern_confidence,
                    data.trend, data.kronos_direction,
                    data.session_high, data.session_low,
                    data.price, data.micro_momentum, 0.0, 0.0],
                "flow": [data.cvd_signal, data.cvd_buy_sell_ratio,
                    data.kalman_momentum],
                "level": [data.vp_signal, data.gex_signal, data.cot_signal],
                "sentiment": [data.gex_signal, data.cot_signal, data.vp_signal],
            });
            let log_body = json!({
                "symbol": symbol,
                "action": "SELL",
                "price": pos.current_price,
                "pnl": pnl,
                "pnl_pct": pnl_pct,
                "hold_seconds": pos.hold_seconds,
                "reason": reason,
                "features": features,
            });
            tokio::spawn(async move {
                let _ = reqwest::Client::new()
                    .post("http://finetune-sidecar:8002/log/trade")
                    .json(&log_body)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;
            });
        }

        let time = Local::now().format("%H:%M:%S").to_string();
        let trade = Trade {
            symbol: symbol.to_string(),
            action: "SELL".into(),
            shares: pos.shares,
            price: pos.current_price,
            value,
            pnl: Some(pnl),
            pnl_pct: Some(pnl_pct),
            reason: reason.to_string(),
            time,
            hold_seconds: Some(pos.hold_seconds),
        };
        if self.trades.len() >= 500 { self.trades.pop_front(); }
        self.trades.push_back(trade);
        self.cooldowns.insert(symbol.to_string(), Instant::now());

        // Mirror the exit to the Alpaca paper account (observational only).
        if ALPACA_SHADOW_ORDERS {
            let (s, q, p, r) = (symbol.to_string(), pos.shares, pos.current_price, reason.to_string());
            tokio::spawn(async move {
                crate::services::alpaca_broker::shadow_order(s, q, "sell", p, r).await;
            });
        }

        // Persist immediately after a realized trade.
        self.save_state();
    }

    fn log_veto(&mut self, symbol: &str, price: f64, score: f64, reason: impl Into<String>) {
        if self.veto_log.len() >= 100 { self.veto_log.pop_front(); }
        self.veto_log.push_back(VetoEntry {
            symbol: symbol.to_string(),
            price_at_veto: price,
            veto_reason: reason.into(),
            score,
            timestamp: Instant::now(),
        });
    }

    pub fn check_missed_opportunities(&mut self) {
        let mut missed = Vec::new();
        self.veto_log.retain(|v| {
            let age = v.timestamp.elapsed().as_secs();
            if age < 300 { return true; } // check after 5 min
            if age > 900 { return false; } // expire after 15 min
            if let Some(data) = self.market_data.get(&v.symbol) {
                let price_change_pct = (data.price - v.price_at_veto) / v.price_at_veto * 100.0;
                if price_change_pct > 0.3 {
                    missed.push((v.clone(), data.price, price_change_pct));
                }
            }
            false
        });

        for (veto, current_price, change_pct) in &missed {
            info!(
                "MISSED_OPPORTUNITY: {} vetoed by {} at ${:.2} (score={:.3}), \
                 now ${:.2} (+{:.2}%) — would have profited",
                veto.symbol, veto.veto_reason, veto.price_at_veto,
                veto.score, current_price, change_pct
            );

            let log_entry = json!({
                "timestamp": Local::now().to_rfc3339(),
                "type": "missed_opportunity",
                "symbol": veto.symbol,
                "veto_reason": veto.veto_reason,
                "score_at_veto": veto.score,
                "price_at_veto": veto.price_at_veto,
                "price_after": current_price,
                "price_change_pct": change_pct,
                "seconds_later": veto.timestamp.elapsed().as_secs(),
            });

            tokio::spawn(async move {
                let path = "/app/reports/prediction_accuracy.jsonl";
                if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
                    let mut line = serde_json::to_string(&log_entry).unwrap_or_default();
                    line.push('\n');
                    let _ = f.write_all(line.as_bytes()).await;
                }
            });
        }
    }

    fn total_value(&self) -> f64 {
        self.cash + self.invested_value()
    }

    fn invested_value(&self) -> f64 {
        self.positions.values().map(|p| p.market_value()).sum()
    }

    pub fn build_payload(&self) -> serde_json::Value {
        let total = self.total_value();
        let total_pnl = total - INITIAL_CASH;
        let total_pnl_pct = total_pnl / INITIAL_CASH * 100.0;
        let target_value = INITIAL_CASH + 100.0;
        let progress = ((total - INITIAL_CASH) / (target_value - INITIAL_CASH) * 100.0).clamp(0.0, 100.0);

        let win_rate = if self.total_trades > 0 {
            self.winning_trades as f64 / self.total_trades as f64 * 100.0
        } else { 0.0 };

        let drawdown_pct = if total < INITIAL_CASH {
            (total - INITIAL_CASH) / INITIAL_CASH * 100.0
        } else { 0.0 };

        let sell_trades: Vec<&Trade> = self.trades.iter().filter(|t| t.action == "SELL").collect();
        let avg_hold = if !sell_trades.is_empty() {
            sell_trades.iter().filter_map(|t| t.hold_seconds).sum::<u64>() / sell_trades.len() as u64
        } else { 0 };

        let avg_pnl = if !sell_trades.is_empty() {
            sell_trades.iter().filter_map(|t| t.pnl).sum::<f64>() / sell_trades.len() as f64
        } else { 0.0 };

        let positions: Vec<serde_json::Value> = self.positions.values().map(|p| json!({
            "symbol": p.symbol, "shares": p.shares, "entry_price": p.entry_price,
            "current_price": p.current_price,
            "market_value": (p.market_value() * 100.0).round() / 100.0,
            "unrealized_pnl": (p.unrealized_pnl() * 10000.0).round() / 10000.0,
            "unrealized_pnl_pct": (p.unrealized_pnl_pct() * 100.0).round() / 100.0,
            "hold_seconds": p.hold_seconds,
        })).collect();

        let recent_trades: Vec<serde_json::Value> = self.trades.iter().rev().take(20).map(|t| json!({
            "symbol": t.symbol, "action": t.action, "shares": t.shares, "price": t.price,
            "total": (t.value * 100.0).round() / 100.0,
            "pnl": t.pnl.map(|v| (v * 10000.0).round() / 10000.0),
            "pnl_pct": t.pnl_pct.map(|v| (v * 100.0).round() / 100.0),
            "reason": t.reason, "time": t.time,
        })).collect();

        let symbols: Vec<serde_json::Value> = TOP_SYMBOLS.iter().map(|&sym| {
            let data = self.market_data.get(sym);
            let bias = self.kronos_daily_bias.get(sym).unwrap_or(&0.0);
            let in_position = self.positions.contains_key(sym);
            let position_pnl = self.positions.get(sym).map(|p| p.unrealized_pnl()).unwrap_or(0.0);
            json!({
                "symbol": sym,
                "price": data.map(|d| d.price).unwrap_or(0.0),
                "direction": data.map(|d| {
                    if d.pattern_signal > 0.1 { "bullish" }
                    else if d.pattern_signal < -0.1 { "bearish" }
                    else { "neutral" }
                }).unwrap_or("neutral"),
                "signal": data.map(|d| d.pattern_signal).unwrap_or(0.0),
                "micro_momentum": data.map(|d| d.micro_momentum).unwrap_or(0.0),
                "kronos_bias": bias,
                "in_position": in_position,
                "position_pnl": (position_pnl * 10000.0).round() / 10000.0,
            })
        }).collect();

        let value_history: Vec<f64> = {
            let mut hist = vec![INITIAL_CASH];
            let mut running = INITIAL_CASH;
            for t in self.trades.iter() {
                if let Some(pnl) = t.pnl { running += pnl; hist.push(running); }
            }
            hist.push(total);
            if hist.len() > 60 { hist.split_off(hist.len() - 60) } else { hist }
        };

        let lb = &self.layer_blocks;
        let total_blocked = lb.consensus + lb.score_too_low;
        let total_evaluated = lb.total_passed + total_blocked;
        let filter_rate = if total_evaluated > 0 {
            total_blocked as f64 / total_evaluated as f64 * 100.0
        } else { 0.0 };

        json!({
            "market_open": self.market_open,
            "portfolio": {
                "cash": (self.cash * 100.0).round() / 100.0,
                "positions_value": (self.invested_value() * 100.0).round() / 100.0,
                "total_value": (total * 100.0).round() / 100.0,
                "total_pnl": (total_pnl * 10000.0).round() / 10000.0,
                "total_pnl_pct": (total_pnl_pct * 100.0).round() / 100.0,
                "realized_pnl": (self.realized_pnl * 10000.0).round() / 10000.0,
                "target_pct": 50.0, "target_value": target_value,
                "progress_pct": (progress * 100.0).round() / 100.0,
                "drawdown_pct": (drawdown_pct * 100.0).round() / 100.0,
            },
            "positions": positions, "recent_trades": recent_trades,
            "stats": {
                "total_trades": self.total_trades, "winning_trades": self.winning_trades,
                "win_rate": (win_rate * 100.0).round() / 100.0,
                "avg_hold_seconds": avg_hold,
                "avg_pnl_per_trade": (avg_pnl * 10000.0).round() / 10000.0,
                "daily_trades": self.daily_trades, "daily_trade_limit": MAX_DAILY_TRADES,
                "open_positions": self.positions.len(),
                "uptime_seconds": self.start_time.elapsed().as_secs(),
            },
            "symbols": symbols, "value_history": value_history,
            "agent_monitor": {
                "architecture": "ml_ensemble_with_fallback",
                "agents": ["Kronos(25%)", "Momentum(ML)", "Pattern(ML)", "Flow(ML)", "Level(ML)", "Sentiment(ML)", "Meta(ML)"],
                "ml_active": self.market_data.values().any(|d|
                    d.ml_agent_scores.as_ref().map(|s| s.is_trained).unwrap_or(false)
                ),
                "vetoed": lb.consensus,
                "score_too_low": lb.score_too_low,
                "total_passed": lb.total_passed,
                "total_evaluated": total_evaluated,
                "filter_rate_pct": (filter_rate * 10.0).round() / 10.0,
            },
        })
    }
}
