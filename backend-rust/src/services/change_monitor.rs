//! Detects when a parameter change alters behaviour, and says so.
//!
//! WHY THIS EXISTS
//! On 2026-08-18 `MAX_DAILY_ENTRIES` (12) sat one above
//! `MAX_CONCURRENT_POSITIONS` (11), so filling the book at the open consumed
//! the whole daily budget. The cap bound at 10:45 and for the next five hours
//! the system could only sell: capital drained to idle and two leftover
//! positions could not be rotated out. $2,120 of $3,000 sat uninvested.
//!
//! The existing agent DID notice — it logged "only 29% of capital is
//! deployed". Detection was never the problem. Nobody saw it, and nothing
//! connected the behaviour to the parameter that caused it.
//!
//! So this module does two things the health checks do not:
//!
//!   1. Fingerprints the operative strategy parameters. When the fingerprint
//!      changes, a new epoch is recorded with the date it began, so behaviour
//!      is always attributable to a specific configuration.
//!   2. Compares the current epoch's behaviour against the previous one and
//!      reports what moved. A parameter change that shifts nothing is worth
//!      knowing; so is one that halves the fill rate.
//!
//! The comparison is a pure function over recorded stats, so the rules can be
//! tested without a broker, a clock, or a market. That matters here: an alarm
//! that only fires in production is an alarm nobody can trust.

use serde_json::{json, Value};
use std::collections::BTreeMap;

const EPOCH_LOG: &str = "/app/reports/config_epochs.jsonl";

/// The parameters whose change makes past behaviour non-comparable.
///
/// Deliberately explicit rather than derived: adding a constant here is a
/// decision that it affects behaviour, and the kill criterion holds that a
/// change to any entry rule invalidates the trial. Anything omitted is a claim
/// that it does not matter, which should be a conscious claim.
pub fn config_fingerprint() -> String {
    use crate::config::*;
    format!(
        "entries={} slots={} floor={} cooldown={} trail={} hardmult={} atrfloor={} \
         cash={} maxexp={} floorpct={} locktrig={} lockgive={}",
        MAX_DAILY_ENTRIES,
        MAX_CONCURRENT_POSITIONS,
        MIN_ENTRY_SCORE,
        ENTRY_COOLDOWN_SECS,
        TRAIL_STOP_FIXED_PCT,
        HARD_STOP_ATR_MULT,
        ATR_PCT_FLOOR,
        INITIAL_CASH,
        MAX_EXPOSURE_MODE,
        CAPITAL_FLOOR_PCT,
        PROFIT_LOCK_TRIGGER_PCT,
        PROFIT_LOCK_GIVEBACK_PCT,
    )
}

/// Behavioural summary of one configuration epoch, computed from the fill log.
///
/// Pure, so it can be tested. Every metric here is one a real bug actually
/// moved: `fill_rate` would have caught the 11 fills the poller abandoned,
/// `cap_bound_days` would have caught the entry cap, and `median_hold_secs`
/// would have caught positions held all afternoon because nothing could
/// replace them.
pub fn epoch_stats(fill_log: &str, since_date: &str, entry_cap: u32) -> Value {
    #[derive(Default)]
    struct Day {
        entries: u32,
        trail_stops: u32,
        filled: u32,
        not_filled: u32,
        realized: f64,
        round_trips: u32,
        holds: Vec<i64>,
    }
    let mut days: BTreeMap<String, Day> = BTreeMap::new();

    // Chronological. FIFO matching and hold times both depend on ordering, and
    // the log is no longer strictly append-in-time-order once fills are
    // backfilled from the broker.
    let mut rows: Vec<Value> = fill_log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    rows.sort_by(|a, b| {
        a["timestamp"].as_str().unwrap_or("").cmp(b["timestamp"].as_str().unwrap_or(""))
    });

    // symbol -> open lots of (qty, price, epoch_secs)
    let mut lots: BTreeMap<String, Vec<(f64, f64, i64)>> = BTreeMap::new();
    for v in &rows {
        let ts = v["timestamp"].as_str().unwrap_or("");
        let day: String = ts.chars().take(10).collect();
        if day.is_empty() || day.as_str() < since_date {
            continue;
        }
        let d = days.entry(day).or_default();
        if v["outcome"].as_str() == Some("filled") {
            d.filled += 1;
        } else {
            d.not_filled += 1;
            continue;
        }

        let sym = v["symbol"].as_str().unwrap_or("").to_string();
        let side = v["side"].as_str().unwrap_or("");
        let px = v["actual_price"].as_f64().unwrap_or(0.0);
        let qty = v["qty_filled"].as_f64().or_else(|| v["qty_requested"].as_f64()).unwrap_or(0.0);
        if px <= 0.0 || qty <= 0.0 {
            continue;
        }
        let secs = parse_epoch_secs(ts);

        if side == "buy" {
            d.entries += 1;
            lots.entry(sym).or_default().push((qty, px, secs));
        } else {
            let reason = v["reason"].as_str().unwrap_or("");
            if reason.contains("TRAIL_STOP") {
                d.trail_stops += 1;
            }
            d.round_trips += 1;
            let mut rem = qty;
            let book = lots.entry(sym).or_default();
            while rem > 1e-9 && !book.is_empty() {
                let (lq, lpx, lsecs) = book[0];
                let take = rem.min(lq);
                d.realized += (px - lpx) * take;
                if secs > 0 && lsecs > 0 {
                    d.holds.push(secs - lsecs);
                }
                rem -= take;
                if lq - take <= 1e-9 {
                    book.remove(0);
                } else {
                    book[0].0 = lq - take;
                }
            }
        }
    }

    let n = days.len().max(1) as f64;
    let tot_entries: u32 = days.values().map(|d| d.entries).sum();
    let tot_trail: u32 = days.values().map(|d| d.trail_stops).sum();
    let tot_filled: u32 = days.values().map(|d| d.filled).sum();
    let tot_notfilled: u32 = days.values().map(|d| d.not_filled).sum();
    let tot_real: f64 = days.values().map(|d| d.realized).sum();
    let tot_trips: u32 = days.values().map(|d| d.round_trips).sum();
    let cap_bound_days = days
        .values()
        .filter(|d| entry_cap > 0 && d.entries >= entry_cap)
        .count();
    let mut holds: Vec<i64> = days.values().flat_map(|d| d.holds.clone()).collect();
    holds.sort_unstable();
    let median_hold = if holds.is_empty() { 0 } else { holds[holds.len() / 2] };

    json!({
        "days": days.len(),
        "entries_per_day": round2(tot_entries as f64 / n),
        "trail_stops_per_day": round2(tot_trail as f64 / n),
        "fill_rate": round2(if tot_filled + tot_notfilled > 0 {
            tot_filled as f64 / (tot_filled + tot_notfilled) as f64
        } else { 1.0 }),
        "expectancy": round2(if tot_trips > 0 { tot_real / tot_trips as f64 } else { 0.0 }),
        "round_trips": tot_trips,
        "median_hold_secs": median_hold,
        "cap_bound_days": cap_bound_days,
        "realized": round2(tot_real),
    })
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn parse_epoch_secs(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts).map(|d| d.timestamp()).unwrap_or(0)
}

/// Compare two epochs and report what moved. Pure, and the whole point.
///
/// Returns (check, severity, message) triples. Thresholds are deliberately
/// coarse: this is a tripwire meant to make a human look, not a statistical
/// test. A tripwire that fires on noise gets ignored, which is worse than none.
pub fn compare(prev: &Value, cur: &Value, entry_cap: u32) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let g = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);

    if g(cur, "days") < 1.0 {
        out.push(("change_monitor".into(), "info".into(),
            "Current config has no completed trading day yet — nothing to compare.".into()));
        return out;
    }

    // The cap binding is not a subtle statistical signal, it is a hard stop:
    // once hit, the book cannot be redeployed for the rest of the session.
    let bound = g(cur, "cap_bound_days");
    if bound > 0.0 {
        out.push(("entry_cap_binding".into(), "critical".into(), format!(
            "The daily entry cap ({}) was reached on {:.0} of {:.0} day(s) in this config. \
             Once bound, the system can only sell for the rest of the session, so capital \
             drains to idle and open positions cannot be rotated. This is what stranded \
             $2,120 on 2026-08-18.",
            entry_cap, bound, g(cur, "days"))));
    }

    if g(cur, "fill_rate") < 0.9 {
        out.push(("fill_rate".into(), "warn".into(), format!(
            "Only {:.0}% of logged orders reached a filled state. Orders the poller \
             abandons are invisible to P&L until the backfill recovers them.",
            g(cur, "fill_rate") * 100.0)));
    }

    if g(prev, "days") < 1.0 {
        out.push(("change_monitor".into(), "info".into(), format!(
            "First epoch on record ({:.0} day(s)). Comparison begins at the next config change.",
            g(cur, "days"))));
        return out;
    }

    // Ratio comparison, guarded against a zero baseline.
    let moved = |now: f64, before: f64, factor: f64| -> bool {
        before > 0.0 && (now > before * factor || now < before / factor)
    };

    if moved(g(cur, "entries_per_day"), g(prev, "entries_per_day"), 2.0) {
        out.push(("entries_per_day".into(), "warn".into(), format!(
            "Entries per day moved from {:.1} to {:.1} — more than a 2x change. Either \
             the entry gate loosened or something is churning.",
            g(prev, "entries_per_day"), g(cur, "entries_per_day"))));
    }

    if moved(g(cur, "trail_stops_per_day"), g(prev, "trail_stops_per_day"), 2.0) {
        out.push(("churn".into(), "warn".into(), format!(
            "Trailing-stop exits per day moved from {:.1} to {:.1}. Each one is a realized \
             exit plus the cost of getting back in.",
            g(prev, "trail_stops_per_day"), g(cur, "trail_stops_per_day"))));
    }

    if moved(g(cur, "median_hold_secs"), g(prev, "median_hold_secs"), 2.5) {
        out.push(("hold_time".into(), "warn".into(), format!(
            "Median hold moved from {:.0}s to {:.0}s. A large jump can mean positions are \
             held because nothing can replace them, not because they are working.",
            g(prev, "median_hold_secs"), g(cur, "median_hold_secs"))));
    }

    // Expectancy is the one that matters. Flag a material move only once there
    // are enough trips for it to mean anything.
    let (pe, ce) = (g(prev, "expectancy"), g(cur, "expectancy"));
    if g(cur, "round_trips") >= 10.0 && ce < pe - 0.15 {
        out.push(("expectancy".into(), "critical".into(), format!(
            "Expectancy per round trip fell from ${:.2} to ${:.2} over {:.0} trips since the \
             config changed. The change made results worse, not merely different. \
             BASIS: this config epoch, FIFO-matched from our own fill log. The \
             live_kill_criterion finding measures the TRIAL window from Alpaca's \
             equity curve, so the two can point opposite ways — and where they \
             do, the broker's number is the one to act on.",
            pe, ce, g(cur, "round_trips"))));
    } else if g(cur, "round_trips") >= 10.0 && ce > pe + 0.15 {
        out.push(("expectancy".into(), "info".into(), format!(
            "Expectancy per round trip improved from ${:.2} to ${:.2} over {:.0} trips. \
             Provisional until the trip count is comparable to the baseline. \
             BASIS: this config epoch, FIFO-matched from our own fill log — a \
             DIFFERENT window and a different source from live_kill_criterion, \
             which reads Alpaca's equity curve over the trial window. An \
             improvement here alongside a worse figure there is not a \
             contradiction, and it is not grounds to prefer this one: a number \
             rebuilt from our own records can inherit our own bugs, and has.",
            pe, ce, g(cur, "round_trips"))));
    }

    if out.is_empty() {
        out.push(("change_monitor".into(), "info".into(), format!(
            "No behavioural change beyond the tripwires: {:.1} entries/day, {:.0}% fill rate, \
             ${:.2} per trip over {:.0} trips.",
            g(cur, "entries_per_day"), g(cur, "fill_rate") * 100.0, ce, g(cur, "round_trips"))));
    }
    out
}

/// Record the current epoch if the fingerprint changed, then compare.
pub async fn run() -> Value {
    let fp = config_fingerprint();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let existing = tokio::fs::read_to_string(EPOCH_LOG).await.unwrap_or_default();
    let mut epochs: Vec<Value> = existing
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();

    let changed = epochs
        .last()
        .map(|e| e["fingerprint"].as_str() != Some(fp.as_str()))
        .unwrap_or(true);
    if changed {
        let row = json!({
            "fingerprint": fp,
            "started": today,
            "recorded_at": chrono::Local::now().to_rfc3339(),
        });
        let mut line = serde_json::to_string(&row).unwrap_or_default();
        line.push('\n');
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(EPOCH_LOG) {
            let _ = f.write_all(line.as_bytes());
        }
        tracing::warn!("[MONITOR] config changed — new epoch recorded; behaviour before this \
                        point describes a different system");
        epochs.push(row);
    }

    let cap = crate::config::MAX_DAILY_ENTRIES;
    let log = tokio::fs::read_to_string(crate::services::alpaca_broker::FILL_LOG)
        .await
        .unwrap_or_default();

    let cur_start = epochs
        .last()
        .and_then(|e| e["started"].as_str())
        .unwrap_or(&today)
        .to_string();
    let prev_start = if epochs.len() >= 2 {
        epochs[epochs.len() - 2]["started"].as_str().unwrap_or("1970-01-01").to_string()
    } else {
        "1970-01-01".to_string()
    };

    let cur = epoch_stats(&log, &cur_start, cap);
    // The previous epoch ends where the current one begins. epoch_stats takes a
    // start bound only, so trim to rows before the current epoch first.
    let prev_log: String = log
        .lines()
        .filter(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .and_then(|v| v["timestamp"].as_str().map(|t| t.chars().take(10).collect::<String>()))
                .map(|d| d < cur_start)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prev = if epochs.len() >= 2 {
        epoch_stats(&prev_log, &prev_start, cap)
    } else {
        json!({"days": 0})
    };

    let findings: Vec<Value> = compare(&prev, &cur, cap)
        .into_iter()
        .map(|(c, s, m)| json!({"check": c, "severity": s, "message": m}))
        .collect();

    json!({
        "epochs_recorded": epochs.len(),
        "config_changed_this_cycle": changed,
        "current_epoch": { "fingerprint": fp, "started": cur_start, "stats": cur },
        "previous_epoch": { "started": prev_start, "stats": prev },
        "findings": findings,
    })
}
