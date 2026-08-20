//! agentic_test — agentic workflow module: autonomous OPERATIONS, not
//! autonomous strategy.
//!
//! This module watches the system the way a careful operator would: it checks
//! that data is flowing, that capital is actually deployed, that the books add
//! up, and that running experiments get their pre-committed verdicts on time.
//! It writes plain-English findings to reports/agent_log.jsonl and serves them
//! at /api/agent.
//!
//! DELIBERATE BOUNDARY — it never changes strategy.
//! It cannot alter weights, thresholds, sizing or entry/exit rules. That is not
//! a missing feature; it is the point. This project's entire value rests on
//! being able to say "this configuration produced these results". An agent that
//! silently retunes itself would produce automated overfitting and destroy every
//! A/B comparison. So the supervisor OBSERVES and RECOMMENDS; a human decides.

use crate::config::*;
use crate::state::AppState;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

const AGENT_LOG: &str = "/app/reports/agent_log.jsonl";
const LEDGER_PATH: &str = "/app/reports/performance_ledger.json";

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub enum Severity { Info, Warn, Critical }

impl Severity {
    fn as_str(&self) -> &'static str {
        match self { Severity::Info => "info", Severity::Warn => "warn", Severity::Critical => "critical" }
    }
}

/// A single thing the supervisor noticed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub check: String,
    pub severity: String,
    pub message: String,
    /// What a human should consider doing. Never auto-applied.
    pub recommendation: Option<String>,
}

impl Finding {
    fn new(check: &str, sev: Severity, msg: String, rec: Option<String>) -> Self {
        Self { check: check.into(), severity: sev.as_str().into(), message: msg, recommendation: rec }
    }
}

#[derive(Default)]
pub struct AgentState {
    pub last_run: String,
    pub runs: u32,
    pub findings: Vec<Finding>,
    pub summary: String,
}

impl AgentState {
    pub fn to_json(&self) -> Value {
        let crit = self.findings.iter().filter(|f| f.severity == "critical").count();
        let warn = self.findings.iter().filter(|f| f.severity == "warn").count();
        json!({
            "role": "operational supervisor — observes and recommends; never changes strategy",
            "boundary": "cannot alter weights, thresholds, sizing, or entry/exit rules (protects measurement integrity)",
            "version": MODEL_VERSION,
            "last_run": self.last_run,
            "runs": self.runs,
            "health": if crit > 0 { "critical" } else if warn > 0 { "degraded" } else { "healthy" },
            "critical_count": crit,
            "warn_count": warn,
            "summary": self.summary,
            "findings": self.findings,
        })
    }
}

pub type SharedAgent = Arc<Mutex<AgentState>>;

pub fn create_shared() -> SharedAgent {
    Arc::new(Mutex::new(AgentState::default()))
}

/// ET wall-clock minutes since midnight, plus whether the market is open.
fn et_now() -> (u32, bool) {
    use chrono::{Datelike, Timelike, Weekday};
    let utc = chrono::Utc::now();
    let off = if utc.month() >= 3 && utc.month() <= 10 { 4 } else { 5 };
    let et = utc - chrono::Duration::hours(off);
    let mins = et.hour() * 60 + et.minute();
    let weekday = !matches!(et.weekday(), Weekday::Sat | Weekday::Sun);
    let open = weekday && mins >= 9 * 60 + 30 && mins < 16 * 60;
    (mins, open)
}

/// ── CHECK 1: is market data actually flowing? ─────────────────────
fn check_data_flow(state: &AppState, market_open: bool) -> Finding {
    let mut live = 0;
    let mut stale = Vec::new();
    for sym in TOP_SYMBOLS.iter() {
        let engine = state.get_engine(sym);
        match engine.get_last_payload() {
            Some(p) => {
                let price = p["current_price"].as_f64().unwrap_or(0.0);
                if price > 0.0 { live += 1; } else { stale.push(sym.clone()); }
            }
            None => stale.push(sym.clone()),
        }
    }
    if live == TOP_SYMBOLS.len() {
        Finding::new("data_flow", Severity::Info,
            format!("All {} symbols streaming live prices.", live), None)
    } else if market_open {
        Finding::new("data_flow", Severity::Critical,
            format!("Only {}/{} symbols have live prices; stale: {:?}. Trading decisions may be blind.", live, TOP_SYMBOLS.len(), stale),
            Some("Check the Alpaca stream and restart the backend if it does not recover.".into()))
    } else {
        Finding::new("data_flow", Severity::Info,
            format!("{}/{} symbols warm (market closed).", live, TOP_SYMBOLS.len()), None)
    }
}

/// ── CHECK 2: is the budget actually deployed? ─────────────────────
/// This is the check that would have caught the "90% of capital sitting idle"
/// bug within one cycle instead of days later.
fn check_deployment(state: &AppState, market_open: bool) -> Finding {
    let trader = state.trader.lock();
    let snap = trader.portfolio_snapshot();
    let cash = snap.0;
    let invested = snap.1;
    let total = cash + invested;
    let pct = if total > 0.0 { invested / total * 100.0 } else { 0.0 };
    let risk_on = trader.is_risk_on();
    drop(trader);

    if !market_open {
        return Finding::new("capital_deployment", Severity::Info,
            format!("Market closed. {:.0}% deployed (${:.2} invested / ${:.2} cash).", pct, invested, cash), None);
    }
    if !risk_on {
        return Finding::new("capital_deployment", Severity::Info,
            format!("Regime risk-OFF, so holding {:.0}% deployed by design (cash is the safe posture).", pct), None);
    }
    if MAX_EXPOSURE_MODE && pct < 50.0 {
        Finding::new("capital_deployment", Severity::Warn,
            format!("Max-exposure mode is ON and the market is risk-on, but only {:.0}% of capital is deployed (${:.2} idle).", pct, cash),
            Some("Expected ~100%. Check for an entry gate (trade cap, cooldown, veto) blocking redeployment.".into()))
    } else {
        Finding::new("capital_deployment", Severity::Info,
            format!("{:.0}% deployed (${:.2} invested, ${:.2} cash) — as expected.", pct, invested, cash), None)
    }
}

/// ── CHECK 3: do the books add up? ─────────────────────────────────
/// Validates that recorded daily P&L increments reconcile with the running
/// cumulative total — the exact class of bug that inflated profit reports 5x.
async fn check_ledger_integrity() -> Finding {
    let content = match tokio::fs::read_to_string(LEDGER_PATH).await {
        Ok(c) => c,
        Err(_) => return Finding::new("ledger_integrity", Severity::Info,
            "No performance ledger yet.".into(), None),
    };
    let v: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => return Finding::new("ledger_integrity", Severity::Critical,
            format!("Performance ledger is unparseable: {}", e),
            Some("Inspect reports/performance_ledger.json — reporting is unreliable until fixed.".into())),
    };
    let days = v["days"].as_array().cloned().unwrap_or_default();
    let reliable: Vec<&Value> = days.iter().filter(|d| d["daily_reliable"].as_bool().unwrap_or(false)).collect();
    let unreliable = days.len() - reliable.len();

    // Reconcile: sum of daily increments should equal the latest cumulative.
    let mut mismatch = None;
    if reliable.len() >= 2 {
        let sum: f64 = reliable.iter().map(|d| d["gross_pnl"].as_f64().unwrap_or(0.0)).sum();
        let first_prev = reliable.first().map(|d| {
            d["cumulative_pnl"].as_f64().unwrap_or(0.0) - d["gross_pnl"].as_f64().unwrap_or(0.0)
        }).unwrap_or(0.0);
        let latest = reliable.last().map(|d| d["cumulative_pnl"].as_f64().unwrap_or(0.0)).unwrap_or(0.0);
        let expected = latest - first_prev;
        if (sum - expected).abs() > 0.05 {
            mismatch = Some(format!("daily increments sum to ${:.2} but cumulative implies ${:.2}", sum, expected));
        }
    }
    match mismatch {
        Some(m) => Finding::new("ledger_integrity", Severity::Critical,
            format!("Books do not reconcile: {}.", m),
            Some("Daily P&L recording may be double-counting again. Do not trust total-profit figures until resolved.".into())),
        None => Finding::new("ledger_integrity", Severity::Info,
            format!("Ledger reconciles. {} reliable day(s), {} legacy row(s) excluded.", reliable.len(), unreliable), None),
    }
}

/// ── CHECK: live signal trader kill criterion ──────────────────────
///
/// Judged on REAL Alpaca round trips. The simulator's own count is not used:
/// it disagreed with the broker in sign as recently as 2026-08-05, and a
/// retirement decision must not rest on a number we derive ourselves.
async fn check_live_kill_criterion() -> Finding {
    if !LIVE_KILL_ENABLED {
        return Finding::new("live_kill_criterion", Severity::Info,
            "Live kill criterion disabled.".into(), None);
    }
    // Both numbers come from ALPACA, not from our fill log.
    //
    // This read real_pnl(), which FIFO-matches reports/broker_fills.jsonl. On
    // 2026-08-10 that log held 9 filled and 15 "unfilled" while Alpaca had
    // filled all 24 — the poller's 10s window was tuned on megacaps and the new
    // sector leaders settle slower. The criterion was therefore judging the
    // strategy on 11 unmatched sells and 16 suspect rows. Reconstructing from
    // our own records is precisely the mistake this project keeps repeating.
    let eq = crate::services::alpaca_broker::equity_pnl().await;
    let account_net = eq["net_pnl"].as_f64().unwrap_or(0.0);
    let all_trips = crate::services::alpaca_broker::round_trips_from_broker().await;

    // Measure THIS trial, not the account's whole history.
    //
    // Both readings are account-wide and all-time. Without the baseline the
    // criterion would keep reporting trial 1's failure forever and never say
    // anything about trial 2.
    //
    // The accumulator is subtracted for the same reason: it holds a long-term
    // SPY position fed $13 a day, in the same equity curve, and it is not the
    // intraday trader's doing. Leaving it in would credit or blame the trader
    // for SPY's drift — small now, compounding every session. It never sells,
    // so it adds no round trips and only the P&L needs removing.
    let acc = crate::services::accumulator::status().await;
    let acc_profit = acc["profit"].as_f64().unwrap_or(0.0);

    let net = account_net - LIVE_KILL_BASELINE_NET - acc_profit;
    let trades = all_trips.saturating_sub(LIVE_KILL_BASELINE_TRIPS);
    let exp = if trades > 0 { net / trades as f64 } else { 0.0 };

    let days = chrono::NaiveDate::parse_from_str(LIVE_KILL_START_DATE, "%Y-%m-%d")
        .map(|s| (chrono::Local::now().date_naive() - s).num_days())
        .unwrap_or(0);
    let due = trades >= LIVE_KILL_TRADES || days >= LIVE_KILL_DAYS;

    if !due {
        Finding::new("live_kill_criterion", Severity::Info,
            format!("Trial running: {}/{} real round trips, day {}/{}. \
                     Expectancy ${:.4}/trade (retire below ${:.2}).",
                trades, LIVE_KILL_TRADES, days, LIVE_KILL_DAYS, exp, LIVE_KILL_MIN_EXPECTANCY),
            Some("No entry rule, score floor, sizing or exit parameter may change \
                  while this runs — a mid-trial change resets the count.".into()))
    } else if exp > LIVE_KILL_MIN_EXPECTANCY {
        Finding::new("live_kill_criterion", Severity::Info,
            format!("PASSED: expectancy ${:.4}/trade over {} real round trips.", exp, trades),
            Some("Criterion met. Confirm over a second window before trusting it.".into()))
    } else {
        Finding::new("live_kill_criterion", Severity::Critical,
            format!("FAILED its pre-committed criterion: expectancy ${:.4}/trade over {} real \
                     round trips, against a ${:.2} floor.", exp, trades, LIVE_KILL_MIN_EXPECTANCY),
            Some("Per the rule fixed on 2026-08-06, the intraday signal trader should be \
                  RETIRED, not retuned. exp1 was retired on the same basis.".into()))
    }
}

/// ── CHECK 5: stuck positions ──────────────────────────────────────
fn check_positions(state: &AppState) -> Finding {
    let trader = state.trader.lock();
    let stuck = trader.longest_hold_seconds();
    drop(trader);
    let limit = FLAT_EXIT_SECS;
    if stuck > limit {
        Finding::new("position_health", Severity::Warn,
            format!("A position has been held {}s, beyond the {}s max-hold backstop.", stuck, limit),
            Some("Exit logic may not be firing — check manage_position().".into()))
    } else {
        Finding::new("position_health", Severity::Info,
            format!("Positions within limits (longest hold {}s).", stuck), None)
    }
}

/// Run one full supervisory pass.
pub async fn run_cycle(state: &Arc<AppState>, agent: &SharedAgent) {
    let (_mins, market_open) = et_now();

    let mut findings = vec![
        check_data_flow(state, market_open),
        check_deployment(state, market_open),
        check_positions(state),
    ];
    findings.push(check_ledger_integrity().await);
    findings.push(check_live_kill_criterion().await);

    // Fold the config-epoch monitor in here rather than leaving it on its
    // own endpoint. On 2026-08-18 the deployment check DID detect the
    // stranded capital and logged it; nobody saw it. A finding that only
    // reaches a log file is a finding that does not exist.
    let mon = crate::services::change_monitor::run().await;
    if let Some(items) = mon["findings"].as_array() {
        for f in items {
            let sev = match f["severity"].as_str() {
                Some("critical") => Severity::Critical,
                Some("warn") => Severity::Warn,
                _ => Severity::Info,
            };
            findings.push(Finding::new(
                f["check"].as_str().unwrap_or("change_monitor"),
                sev,
                f["message"].as_str().unwrap_or("").to_string(),
                None,
            ));
        }
    }

    let crit = findings.iter().filter(|f| f.severity == "critical").count();
    let warn = findings.iter().filter(|f| f.severity == "warn").count();
    let health = if crit > 0 { "CRITICAL" } else if warn > 0 { "DEGRADED" } else { "HEALTHY" };

    // Plain-English summary a human can read in one line.
    let headline = findings.iter()
        .find(|f| f.severity == "critical")
        .or_else(|| findings.iter().find(|f| f.severity == "warn"))
        .map(|f| f.message.clone())
        .unwrap_or_else(|| "All checks passed; system operating normally.".into());
    let summary = format!("[{}] {}", health, headline);

    if crit > 0 { warn!("[AGENT] {}", summary); } else { info!("[AGENT] {}", summary); }
    for f in &findings {
        if f.severity != "info" {
            warn!("[AGENT:{}] {} — {}", f.check, f.message,
                f.recommendation.clone().unwrap_or_default());
        }
    }

    let now = chrono::Local::now().to_rfc3339();
    {
        let mut a = agent.lock();
        a.last_run = now.clone();
        a.runs += 1;
        a.summary = summary.clone();
        a.findings = findings.clone();
    }

    let entry = json!({
        "timestamp": now, "health": health, "summary": summary,
        "findings": findings, "version": MODEL_VERSION,
    });
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        if let Ok(mut f) = tokio::fs::OpenOptions::new().create(true).append(true).open(AGENT_LOG).await {
            let mut line = serde_json::to_string(&entry).unwrap_or_default();
            line.push('\n');
            let _ = f.write_all(line.as_bytes()).await;
        }
    });
}

/// Spawn the supervisor loop (every 15 minutes).
pub fn spawn(state: Arc<AppState>, agent: SharedAgent) {
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        loop {
            run_cycle(&state, &agent).await;
            tokio::time::sleep(tokio::time::Duration::from_secs(900)).await;
        }
    });
}
