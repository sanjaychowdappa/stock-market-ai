//! rule_monitor — does each specification rule still WORK, and do the
//! simulator and the broker still agree?
//!
//! WHY THIS EXISTS
//! Three of the five specification rules shipped structurally inert and ran
//! that way for days, and all three looked identical on the board to a rule
//! whose conditions had simply not come up:
//!
//!   r3_profit_only  gated on `weighted_score > MIN_BUY_SIGNAL` while built
//!                   with `[0.0; 7]` weights, so the score was always 0.0
//!   r5_concentrate  same cause
//!   r4_legacy       qualified names from its OWN closed trades, and could not
//!                   trade to acquire any — a closed loop
//!
//! Each was found by hand, days late, by asking "can this fire AT ALL?" rather
//! than "why has it not fired?". That question is mechanical, so it belongs in
//! code and not in whoever happens to look.
//!
//! The probe is the useful part: every rule is evaluated against the most
//! favourable input it could ever receive. If a rule returns false on THAT, no
//! market condition can make it true, and the zero on the board is a defect
//! rather than a result.
//!
//! DELIBERATE BOUNDARY — same as agentic_test: this observes and reports. It
//! never edits a rule, a weight or a threshold.

use serde_json::{json, Value};

use crate::config::MIN_BUY_SIGNAL;
use crate::services::paper_trader::{rule_entry_allowed, RuleEntry, RULE_WEIGHTS};

const FILL_LOG: &str = "/app/reports/broker_fills.jsonl";
const PROFIT_LOG: &str = "/app/reports/daily_profit.jsonl";

/// One shadow book, as the monitor sees it.
#[derive(Debug, Clone)]
pub struct BookStat {
    pub model_id: String,
    pub rule: String,
    pub total_trades: u32,
}

/// The most favourable input a rule could ever be handed.
///
/// Built from RULE_WEIGHTS rather than from a hard-coded number, so a change to
/// the weights is reflected here automatically — that is exactly the change
/// that broke r3 and r5.
fn best_case() -> RuleEntry {
    RuleEntry {
        kronos_score: 1.0,
        weighted_score: RULE_WEIGHTS.iter().sum::<f64>(),
        below_floor: false,
        is_legacy: true,
        holds_nothing: true,
    }
}

/// Can this rule fire at all?
///
/// False means no market condition can ever satisfy it — the rule is dead code
/// wearing the name of a strategy.
pub fn rule_can_ever_fire(rule: &str) -> bool {
    rule_entry_allowed(rule, &best_case())
}

/// Severity levels, kept as strings so this module does not depend on
/// agentic_test's enum and can be tested on its own.
pub const CRITICAL: &str = "critical";
pub const WARN: &str = "warn";
pub const INFO: &str = "info";

fn finding(check: &str, sev: &str, msg: String) -> Value {
    json!({"check": check, "severity": sev, "message": msg})
}

/// CHECK: every rule book must be capable of firing.
pub fn check_rules_can_fire(books: &[BookStat]) -> Vec<Value> {
    let mut out = Vec::new();
    let dead: Vec<&BookStat> = books.iter()
        .filter(|b| !rule_can_ever_fire(&b.rule))
        .collect();

    if dead.is_empty() {
        out.push(finding("rule_can_fire", INFO,
            format!("All {} rule books can fire: each returns true on its \
                     best-case input.", books.len())));
        return out;
    }

    for b in dead {
        out.push(finding("rule_can_fire", CRITICAL, format!(
            "{} (rule '{}') CANNOT FIRE under any market condition. Its \
             best-case input — kronos 1.0, weighted score {:.2}, legacy true, \
             book empty, above the floor — still evaluates to false. Its trade \
             count is a defect, not a result. Check that RULE_WEIGHTS are \
             non-zero (they sum to {:.2}, and the buy threshold is {:.2}) and \
             that the rule name matches the match arm exactly.",
            b.model_id, b.rule,
            RULE_WEIGHTS.iter().sum::<f64>(),
            RULE_WEIGHTS.iter().sum::<f64>(), MIN_BUY_SIGNAL)));
    }
    out
}

/// CHECK: a book silent while its peers trade.
///
/// Weaker than the probe — a rule can be legitimately quiet — so this is a
/// warning, and it names the probe result so the two are not confused.
pub fn check_silent_books(books: &[BookStat]) -> Vec<Value> {
    let active: u32 = books.iter().map(|b| b.total_trades).sum();
    if active == 0 {
        return vec![finding("rule_activity", INFO,
            "No rule book has traded yet; nothing to compare.".into())];
    }
    let mut out = Vec::new();
    for b in books.iter().filter(|b| b.total_trades == 0) {
        out.push(finding("rule_activity", WARN, format!(
            "{} has never traded while its peers have {} trades between them. \
             It CAN fire (best-case probe passes), so this is a live \
             precondition that has not been met — check the data feeding that \
             precondition before concluding the rule is merely selective.",
            b.model_id, active)));
    }
    if out.is_empty() {
        out.push(finding("rule_activity", INFO,
            format!("All {} rule books have traded.", books.len())));
    }
    out
}

/// CHECK: r4's everyday log is being written.
///
/// r4 reads a system-wide log of per-symbol results. If that log is empty while
/// the system has closed trades, the log is not being fed and r4 is inert for a
/// reason no probe can see.
pub fn check_legacy_log(symbols: usize, qualified: usize, closed_trades: u32) -> Value {
    if closed_trades == 0 {
        return finding("legacy_log", INFO,
            "No closed trades yet; the everyday log is empty as expected.".into());
    }
    if symbols == 0 {
        return finding("legacy_log", CRITICAL, format!(
            "The everyday log is EMPTY after {} closed trades. r4_legacy funds \
             names from this log, so it cannot trade and never will. The log is \
             not being written — check that closes call record_legacy().",
            closed_trades));
    }
    finding("legacy_log", INFO, format!(
        "Everyday log holds {} symbol(s), {} qualified for r4 (2+ closed trades \
         and positive cumulative P&L).", symbols, qualified))
}

/// CHECK: the simulator and the broker should end the day at similar numbers.
///
/// They will never match exactly — the simulator books no spread, no slippage
/// and no rejections. A LARGE gap is the interesting case: it has twice meant
/// the simulator was booking trades the broker never filled.
pub fn check_divergence(day: &str, sim: f64, broker: f64) -> Value {
    let gap = sim - broker;
    let sev = if gap.abs() >= 10.0 { WARN } else { INFO };
    finding("sim_vs_broker", sev, format!(
        "{}: simulator {:+.2}, broker {:+.2}, gap {:+.2}. {}",
        day, sim, broker, gap,
        if gap.abs() >= 10.0 {
            "The simulator and the real account disagree by more than $10. The \
             usual cause is orders the simulator counted and the broker never \
             filled — check the rejected and unfilled counts for the same day."
        } else {
            "Within the range spread and slippage explain."
        }))
}

/// CHECK: order fill quality. A collapsed fill rate means the simulator's book
/// and the real one are drifting apart faster than reconcile can close them.
pub fn check_fill_quality(filled: u32, rejected: u32, unfilled: u32, pending: u32) -> Value {
    let total = filled + rejected + unfilled + pending;
    if total == 0 {
        return finding("fill_quality", INFO, "No orders placed today.".into());
    }
    let rate = 100.0 * filled as f64 / total as f64;
    let sev = if rate < 70.0 { WARN } else { INFO };
    finding("fill_quality", sev, format!(
        "{} of {} orders filled ({:.0}%); {} rejected, {} unfilled, {} pending.{}",
        filled, total, rate, rejected, unfilled, pending,
        if rate < 70.0 {
            " Below 70%: every unfilled order is a position the simulator holds \
              and the account does not."
        } else { "" }))
}

/// CHECK: reconcile opening and closing the same name within `window_secs`.
///
/// Reconcile exists to correct drift, not to trade. A position it opens and
/// closes inside a few minutes is pure cost — this was worth -$24.90 across 5
/// occurrences before RECONCILE_MIN_AGE_SECS was introduced, and the gate does
/// not cover the case where the original entry was REJECTED and reconcile
/// restores the position just before the simulator exits it.
pub fn check_reconcile_churn(events: &[(String, String, i64)], window_secs: i64) -> Value {
    // events: (symbol, side, unix_seconds), reconcile-tagged, chronological.
    let mut churn: Vec<String> = Vec::new();
    for (i, (sym, side, ts)) in events.iter().enumerate() {
        if side != "buy" { continue; }
        for (sym2, side2, ts2) in events.iter().skip(i + 1) {
            if sym2 == sym && side2 == "sell" {
                if ts2 - ts <= window_secs {
                    churn.push(format!("{} held {}m", sym, (ts2 - ts) / 60));
                }
                break;
            }
        }
    }
    if churn.is_empty() {
        return finding("reconcile_churn", INFO,
            "No reconcile position was opened and closed within the window.".into());
    }
    finding("reconcile_churn", WARN, format!(
        "Reconcile opened and closed {} position(s) inside {} minutes: {}. \
         Reconcile is bookkeeping, not a strategy; every such round trip is \
         pure cost. Every position ever closed by reconcile has lost money.",
        churn.len(), window_secs / 60, churn.join(", ")))
}

// ── Wiring: read the logs and run the file-backed checks ────────────────

fn read_lines(path: &str) -> Vec<Value> {
    std::fs::read_to_string(path)
        .map(|s| s.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect())
        .unwrap_or_default()
}

fn today_et() -> String {
    let utc = chrono::Utc::now();
    let off = if chrono::Datelike::month(&utc) >= 3 && chrono::Datelike::month(&utc) <= 10 { 4 } else { 5 };
    (utc - chrono::Duration::hours(off)).format("%Y-%m-%d").to_string()
}

/// Everything this module can check from disk. The in-process checks
/// (rules, legacy log) are called separately by agentic_test, which holds the
/// trader lock.
pub fn run_from_logs() -> Vec<Value> {
    let mut out = Vec::new();
    let today = today_et();

    // Fill quality and reconcile churn, today only.
    let fills = read_lines(FILL_LOG);
    let (mut f, mut r, mut u, mut p) = (0u32, 0u32, 0u32, 0u32);
    let mut recon: Vec<(String, String, i64)> = Vec::new();
    for row in &fills {
        let ts = row["timestamp"].as_str().unwrap_or("");
        if !ts.starts_with(&today) { continue; }
        match row["outcome"].as_str() {
            Some("filled") => f += 1,
            Some("rejected") => r += 1,
            Some("unfilled") => u += 1,
            Some("pending") => p += 1,
            _ => {}
        }
        if row["reason"].as_str().unwrap_or("").starts_with("RECONCILE")
            && row["outcome"].as_str() == Some("filled") {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) {
                recon.push((
                    row["symbol"].as_str().unwrap_or("").to_string(),
                    row["side"].as_str().unwrap_or("").to_string(),
                    t.timestamp(),
                ));
            }
        }
    }
    recon.sort_by_key(|e| e.2);
    out.push(check_fill_quality(f, r, u, p));
    out.push(check_reconcile_churn(&recon, 30 * 60));

    // Simulator vs broker, for the most recent day that has both rows.
    let profit = read_lines(PROFIT_LOG);
    let mut sim: Option<(String, f64)> = None;
    let mut brk: Option<(String, f64)> = None;
    for row in profit.iter().rev() {
        let d = row["date"].as_str().unwrap_or("").to_string();
        match row["kind"].as_str() {
            Some("skim") | Some("skim_simulated") if sim.is_none() => {
                if let Some(v) = row["day_pnl"].as_f64() { sim = Some((d, v)); }
            }
            Some("broker") if brk.is_none() => {
                if let Some(v) = row["broker_day_pnl"].as_f64() { brk = Some((d, v)); }
            }
            _ => {}
        }
        if sim.is_some() && brk.is_some() { break; }
    }
    if let (Some((ds, s)), Some((db, b))) = (sim, brk) {
        if ds == db {
            out.push(check_divergence(&ds, s, b));
        } else {
            out.push(finding("sim_vs_broker", INFO, format!(
                "No matching pair yet: latest simulator row is {}, latest broker \
                 row is {}.", ds, db)));
        }
    }
    out
}
