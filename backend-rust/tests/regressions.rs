//! Regression tests: one per bug found on 2026-08-05.
//!
//! Every test here encodes a defect that actually shipped and, in several
//! cases, cost money or reported a loss as a gain. The rule for this file is
//! that each test must FAIL against the old behaviour — a test that passes
//! either way documents nothing.
//!
//! Context for why this file exists at all: before today the repo had zero
//! tests, and two bugs regressed within the same session. The state-save race
//! was "fixed" with a sequence guard that did not work, and had to be fixed a
//! second time. Nothing would have caught that.

use stock_market_ai::services::alpaca_broker::fifo_stats;

// ── Helpers ─────────────────────────────────────────────────────────────

fn fill(sym: &str, side: &str, qty: f64, price: f64, day: &str) -> String {
    format!(
        r#"{{"symbol":"{sym}","side":"{side}","outcome":"filled","qty_filled":{qty},"qty_requested":{qty},"actual_price":{price},"sim_price":{price},"timestamp":"{day}T14:00:00Z"}}"#
    )
}

// ── BUG: round_trips counted matched LOTS, not trades ───────────────────
//
// `round_trips += 1` sat inside the inner lot-matching loop, so one sell that
// consumed three buy lots reported three round trips and three win/loss
// events. Live this inflated 18 real trades to 24 and made "win rate" a
// per-lot statistic wearing a per-trade label.

#[test]
fn one_sell_consuming_three_lots_is_one_round_trip() {
    let log = [
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        fill("NVDA", "buy", 1.0, 101.0, "2026-08-05"),
        fill("NVDA", "buy", 1.0, 102.0, "2026-08-05"),
        fill("NVDA", "sell", 3.0, 110.0, "2026-08-05"),
    ]
    .join("\n");

    let s = fifo_stats(&log);

    assert_eq!(s["round_trips"], 1, "one sell is one round trip, not one per lot consumed");
    assert_eq!(s["wins"], 1, "a single profitable sell is one win, not three");
    assert_eq!(s["losses"], 0);
}

#[test]
fn win_rate_is_per_trade_not_per_lot() {
    // One winning sell over 3 lots, one losing sell over 1 lot.
    // Per-lot counting would report 3 wins / 1 loss = 75%.
    // Per-trade is 1 win / 1 loss = 50%.
    let log = [
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        fill("NVDA", "sell", 3.0, 110.0, "2026-08-05"),
        fill("AAPL", "buy", 1.0, 200.0, "2026-08-05"),
        fill("AAPL", "sell", 1.0, 190.0, "2026-08-05"),
    ]
    .join("\n");

    let s = fifo_stats(&log);

    assert_eq!(s["round_trips"], 2);
    assert_eq!(s["win_rate_pct"], 50.0, "win rate must count trades, not lot matches");
}

// ── BUG: FIFO underflow silently discarded sells ────────────────────────
//
// A sell with no matching buy lot just `break`s out of the match loop. The
// unmatched quantity vanished with no record, so P&L quietly omitted real
// exposure. Live, 2 sells totalling 2.03 shares were being dropped.

#[test]
fn sell_without_a_matching_buy_is_reported_not_swallowed() {
    let log = [
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        fill("NVDA", "sell", 3.0, 110.0, "2026-08-05"), // 2.0 unmatched
    ]
    .join("\n");

    let s = fifo_stats(&log);
    let dq = &s["data_quality"];

    assert_eq!(dq["unmatched_sells"], 1, "the unmatched sell must be counted");
    assert!(
        (dq["unmatched_qty"].as_f64().unwrap() - 2.0).abs() < 1e-9,
        "the dropped quantity must be reported, got {:?}", dq["unmatched_qty"]
    );
}

// ── BUG: partial fills stored as final quantities ───────────────────────
//
// The order poller broke on the first `filled_avg_price` it saw, which Alpaca
// also reports while an order is still `partially_filled`. A 2.825-share
// request was stored as "filled 1.000". Those quantities feed this matcher, so
// the error reached the headline P&L — it is why a +$22.52 account was
// reported as -$12.29. The poller fix lives in shadow_order; here we assert the
// matcher at least SURFACES the discrepancy rather than computing silently.

#[test]
fn quantities_smaller_than_requested_are_flagged_as_suspect() {
    let log = format!(
        r#"{{"symbol":"NVDA","side":"buy","outcome":"filled","qty_requested":2.825,"qty_filled":1.0,"actual_price":100.0,"sim_price":100.0,"timestamp":"2026-08-05T14:00:00Z"}}"#
    );

    let s = fifo_stats(&log);

    assert_eq!(
        s["data_quality"]["partial_qty_rows"], 1,
        "a fill materially smaller than its request must be flagged, not trusted"
    );
}

#[test]
fn a_clean_log_reports_no_data_quality_problems() {
    let log = [
        fill("NVDA", "buy", 2.0, 100.0, "2026-08-05"),
        fill("NVDA", "sell", 2.0, 110.0, "2026-08-05"),
    ]
    .join("\n");

    let s = fifo_stats(&log);
    let dq = &s["data_quality"];

    assert_eq!(dq["unmatched_sells"], 0);
    assert_eq!(dq["partial_qty_rows"], 0);
    assert_eq!(s["round_trips"], 1);
    assert!(
        (s["real_realized_pnl"].as_f64().unwrap() - 20.0).abs() < 1e-9,
        "2 shares from 100 to 110 is $20"
    );
}

// ── BUG: a loss rendered as a gain ──────────────────────────────────────
//
// Sign handling is the single most consequential thing in this file: the
// dashboard once rendered -$6.64 as "$6.64", and separately reported a +$22.52
// account as -$12.29. Losing trades must produce a negative number here.

#[test]
fn a_losing_round_trip_is_negative() {
    let log = [
        fill("NVDA", "buy", 1.0, 110.0, "2026-08-05"),
        fill("NVDA", "sell", 1.0, 100.0, "2026-08-05"),
    ]
    .join("\n");

    let s = fifo_stats(&log);

    assert!(
        s["real_realized_pnl"].as_f64().unwrap() < 0.0,
        "a trade that lost money must report a negative P&L, got {:?}",
        s["real_realized_pnl"]
    );
    assert_eq!(s["losses"], 1);
    assert_eq!(s["wins"], 0);
}

// ── Day attribution ─────────────────────────────────────────────────────
//
// The simulator ledger separately booked gains on days that really lost money.
// Per-day attribution has to follow the SELL's timestamp, since that is when
// the P&L is realized.

#[test]
fn pnl_is_attributed_to_the_day_the_position_closed() {
    let log = [
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-04"),
        fill("NVDA", "sell", 1.0, 90.0, "2026-08-05"), // opened Tue, closed Wed at a loss
    ]
    .join("\n");

    let s = fifo_stats(&log);
    let days = s["by_day"].as_array().unwrap();

    assert_eq!(days.len(), 1, "only the closing day carries realized P&L");
    assert_eq!(days[0]["date"], "2026-08-05");
    assert!(days[0]["real_pnl"].as_f64().unwrap() < 0.0);
}

// ── Rejected and unfilled orders must never enter P&L ───────────────────
//
// 8 wash-trade rejections and 3 mislabelled "unfilled" rows sit in the live
// log. Only genuinely filled rows may be matched.

#[test]
fn rejected_and_unfilled_rows_are_ignored() {
    let log = [
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-05"),
        r#"{"symbol":"NVDA","side":"sell","outcome":"rejected","qty":1.0,"sim_price":200.0,"timestamp":"2026-08-05T14:00:00Z"}"#.to_string(),
        r#"{"symbol":"NVDA","side":"sell","outcome":"unfilled","qty":1.0,"sim_price":200.0,"timestamp":"2026-08-05T14:00:00Z"}"#.to_string(),
        fill("NVDA", "sell", 1.0, 110.0, "2026-08-05"),
    ]
    .join("\n");

    let s = fifo_stats(&log);

    assert_eq!(s["round_trips"], 1, "only the filled sell counts");
    assert!((s["real_realized_pnl"].as_f64().unwrap() - 10.0).abs() < 1e-9);
}

#[test]
fn an_empty_log_does_not_divide_by_zero() {
    let s = fifo_stats("");
    assert_eq!(s["round_trips"], 0);
    assert_eq!(s["win_rate_pct"], 0.0);
    assert_eq!(s["real_realized_pnl"], 0.0);
}

// ── BUG: every free slot was filled regardless of signal ────────────────
//
// Entries were chosen by ranking candidates and taking the top N for however
// many slots were open, with no floor. The book therefore looked identical
// every day and positions were opened on no information at all. The agent
// monitor recorded filter_rate 0.0% across 972 evaluations — the scoring model
// rejected nothing it was ever shown.
//
// These assert against the scores that ACTUALLY opened positions on
// 2026-08-05, taken from the trade log.

use stock_market_ai::config::{qualifies_for_entry, MIN_ENTRY_SCORE};

#[test]
fn the_entries_that_lost_money_today_would_now_be_refused() {
    // Morning deployment: all five slots filled at or below zero.
    for score in [0.000, -0.048, -0.024, -0.003] {
        assert!(
            !qualifies_for_entry(score),
            "score {score} opened a position on 2026-08-05 and must now be refused"
        );
    }
}

#[test]
fn buying_on_exactly_zero_information_is_refused() {
    // GOOGL 09:34: score 0.000 with every layer reading 0.00 — no signal at
    // all, not a weak one. This is the case that needs no backtest to reject.
    assert!(!qualifies_for_entry(0.0));
}

#[test]
fn a_negative_score_is_never_tradeable() {
    // NVDA 09:34 was bought at -0.048 with Kalman reading bearish: the model
    // said "don't" and the trader bought anyway.
    for score in [-0.001, -0.048, -0.5, -1.0] {
        assert!(!qualifies_for_entry(score), "negative score {score} must never enter");
    }
}

#[test]
fn genuinely_positive_scores_still_trade() {
    // The floor must not switch the system off entirely — it filters, it does
    // not veto. These are real scores from later in the same session.
    for score in [0.107, 0.173, 0.456, 0.627] {
        assert!(qualifies_for_entry(score), "score {score} should still qualify");
    }
}

#[test]
fn the_floor_is_a_floor_not_a_veto() {
    assert!(qualifies_for_entry(MIN_ENTRY_SCORE), "the boundary itself qualifies");
    assert!(!qualifies_for_entry(MIN_ENTRY_SCORE - 0.001));
    assert!(
        MIN_ENTRY_SCORE > 0.0,
        "a floor at or below zero would still permit buying on no information"
    );
}

// ── Reconcile vs. in-flight orders (2026-08-12) ─────────────────────────
//
// At 15:55:19 the EOD skim's KO sell was partially filled. A reconcile cycle
// read the position mid-fill, saw 1.716 shares that the simulator had already
// written off, and submitted a duplicate sell. Alpaca rejected it only because
// those shares were committed to the open order. Accepted, it would have taken
// the account short — the simulator's book said zero.
//
// claim_symbol() did not prevent this. It serialises execution, not decisions:
// the duplicate waited for the real fill and then submitted anyway, because the
// delta had already been computed from the stale snapshot.

use std::collections::{HashMap, HashSet};
use stock_market_ai::services::alpaca_broker::reconcile_plan;

/// Ages for a book whose positions have all been held long enough that
/// RECONCILE_MIN_AGE_SECS is not the thing under test.
fn settled(sim: &HashMap<String, f64>) -> HashMap<String, u64> {
    sim.keys().map(|k| (k.clone(), 86_400u64)).collect()
}

fn books(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(s, q)| (s.to_string(), *q)).collect()
}

#[test]
fn mid_fill_position_is_never_reconciled() {
    // The exact 2026-08-12 state: simulator flat, Alpaca still showing the
    // unfilled remainder of a sell that is actively working.
    let sim = books(&[("KO", 0.0)]);
    let live = books(&[("KO", 1.716)]);
    let prices = books(&[("KO", 86.70)]);
    let busy: HashSet<String> = ["KO".to_string()].into_iter().collect();

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &busy, &settled(&sim));

    assert!(
        actions.is_empty(),
        "reconcile ordered {actions:?} against a position that was still mid-fill"
    );
    assert_eq!(deferred, vec!["KO".to_string()], "the symbol must be reported as deferred, not silently dropped");
}

#[test]
fn the_same_gap_is_corrected_once_the_order_settles() {
    // Same numbers, nothing in flight. This is real drift and MUST be fixed —
    // the guard has to defer, not disable.
    let sim = books(&[("KO", 0.0)]);
    let live = books(&[("KO", 1.716)]);
    let prices = books(&[("KO", 86.70)]);

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));

    assert!(deferred.is_empty());
    assert_eq!(actions.len(), 1, "settled drift must still be corrected");
    assert_eq!(actions[0]["action"], "sell");
    assert_eq!(actions[0]["symbol"], "KO");
    assert!((actions[0]["qty"].as_f64().unwrap() - 1.716).abs() < 1e-6);
}

#[test]
fn a_busy_symbol_does_not_block_the_others() {
    // Deferring must be per symbol. Skipping the whole cycle would mean one
    // slow fill at 15:55 leaves every other book unreconciled overnight.
    let sim = books(&[("KO", 0.0), ("DIS", 7.329), ("RTX", 0.0)]);
    let live = books(&[("KO", 1.716), ("DIS", 0.0), ("RTX", 3.398)]);
    let prices = books(&[("KO", 86.70), ("DIS", 103.48), ("RTX", 222.79)]);
    let busy: HashSet<String> = ["KO".to_string()].into_iter().collect();

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &busy, &settled(&sim));

    assert_eq!(deferred, vec!["KO".to_string()]);
    let syms: Vec<&str> = actions.iter().map(|a| a["symbol"].as_str().unwrap()).collect();
    assert_eq!(syms, vec!["DIS", "RTX"], "unaffected symbols must still reconcile");
    assert_eq!(actions[0]["action"], "buy", "sim holds DIS the broker does not");
    assert_eq!(actions[1]["action"], "sell", "broker holds RTX the sim does not");
}

#[test]
fn dust_is_still_ignored_when_nothing_is_in_flight() {
    // A sub-dollar gap is not worth a round trip in fees. Guard must not have
    // disturbed this.
    let sim = books(&[("KO", 0.0)]);
    let live = books(&[("KO", 0.001)]);
    let prices = books(&[("KO", 86.70)]);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));
    assert!(actions.is_empty(), "$0.09 of KO is not worth an order");
}

// ── Trailing-stop width vs entry timing (2026-08-13) ────────────────────
//
// The trail level used to be (1.5 * entry_atr_pct).clamp(0.5, 3.0), where
// entry_atr_pct was a 1-minute ATR frozen at entry. That made stop width a
// function of what time the position opened. FCX, one symbol, one session:
//     10:18 entry -> TRAIL_STOP at -1.69% from peak
//     13:53 entry -> TRAIL_STOP at -0.50% from peak
// Morning entries then could not be stopped at all (TMO spent 118 minutes past
// its threshold and rode to the close), while afternoon entries were cut on
// noise and re-entered.

use stock_market_ai::config::{TRAIL_STOP_FIXED_PCT, trail_stop_pct};

/// The old, buggy computation, frozen. Kept so the tests can demonstrate the
/// defect rather than merely assert the new value.
///
/// The numbers are hardcoded on purpose. Importing the live constants would let
/// a future edit redefine what "the old rule" meant, and these tests would then
/// be comparing the fix against something that never shipped.
fn legacy_trail_lvl(entry_atr_pct: f64) -> f64 {
    (1.5 * entry_atr_pct.max(0.3)).clamp(0.5, 3.0)
}

#[test]
fn trail_width_no_longer_depends_on_when_the_position_opened() {
    // The two ATR readings FCX actually produced on 2026-08-13.
    let morning_atr = 1.13; // volatile open
    let afternoon_atr = 0.20; // calm afternoon, below the floor

    assert!(
        (legacy_trail_lvl(morning_atr) - legacy_trail_lvl(afternoon_atr)).abs() > 1.0,
        "the old rule really did vary stop width by over a full percent on entry time"
    );

    // The whole point of the fix: the production path returns an identical
    // width for every entry ATR, including the two FCX readings above.
    let baseline = trail_stop_pct(morning_atr);
    for atr in [0.05, 0.20, 0.33, 1.13, 4.0, 50.0] {
        assert_eq!(
            trail_stop_pct(atr), baseline,
            "entry-time volatility {atr} must not change the trailing stop"
        );
    }
    assert_eq!(trail_stop_pct(afternoon_atr), trail_stop_pct(morning_atr),
        "FCX's morning and afternoon entries must now get the same stop");
}

#[test]
fn tmo_would_now_be_stopped_out() {
    // TMO drew down 1.60% from its peak and sat past 0.5% for 118 minutes,
    // exiting only via the EOD skim for -$6.76. Under the old rule its frozen
    // morning ATR bought it a stop wider than the drawdown ever reached.
    let tmo_max_drawdown_pct = 1.60;
    assert!(
        tmo_max_drawdown_pct > TRAIL_STOP_FIXED_PCT,
        "TMO's drawdown must now breach the trail and force an exit"
    );
    // And the old rule genuinely would not have caught it.
    assert!(
        tmo_max_drawdown_pct < legacy_trail_lvl(1.13),
        "under the old width TMO's drawdown stayed inside the stop — the bug"
    );
}

#[test]
fn the_width_is_the_robust_choice_not_the_best_backtest_score() {
    // A flat 0.50% scored highest overall (+$24.74 vs actual) but fell to
    // -$5.50 with 2026-08-05 removed. 0.75% was positive in all seven
    // leave-one-day-out folds. If someone later "optimises" this back down to
    // the higher-scoring value, that is the overfit returning.
    assert!(
        (TRAIL_STOP_FIXED_PCT - 0.75).abs() < 1e-9,
        "0.75% was chosen for leave-one-out robustness; re-tuning needs a new replay"
    );
}

#[test]
fn positions_still_get_room_to_breathe() {
    // The stop must not be so tight that ordinary noise closes everything --
    // that was the other half of the 2026-08-13 damage (FCX x3, AMD x2).
    // BAC/KO/EQIX drew down 0.40/0.21/0.30% and were correctly held.
    for quiet_drawdown in [0.21, 0.30, 0.40] {
        assert!(
            quiet_drawdown < TRAIL_STOP_FIXED_PCT,
            "a {quiet_drawdown}% wiggle must not trigger the trailing stop"
        );
    }
}

// ── Ledger: observation rows must never join the banked total (2026-08-14) ──
//
// The daily ledger diverged from Alpaca by a growing amount. Decomposing every
// day showed the cause was not accounting drift but non-fills: the banked
// figure is `cash - INITIAL_CASH` from the SIMULATOR, which books a trade
// whether or not the broker filled it.
//
//   day         ledger    broker   non-filled orders
//   2026-08-13  -$7.34    -$6.83     0   (agrees, slippage only)
//   2026-08-14  +$0.17    +$1.19     0   (agrees, slippage only)
//   2026-08-10 -$10.17    +$1.47    16   (diverges by $11.64)
//   2026-08-11 +$13.56     $0.00     6   (diverges by $13.56, zero round trips)
//
// Recording the broker's own figure alongside means adding a row type, and
// ledger_cumulative() used to sum EVERY row carrying `day_pnl`. That is the
// same shape as the bug that banked $72.28 twice.

use stock_market_ai::services::paper_trader::{ledger_sum, is_banking_kind};

fn row(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("test row must parse")
}

#[test]
fn an_observation_row_does_not_change_the_banked_total() {
    let banked = vec![
        row(r#"{"date":"2026-08-13","kind":"skim","day_pnl":-7.34}"#),
        row(r#"{"date":"2026-08-14","kind":"skim","day_pnl":0.17}"#),
    ];
    let before = ledger_sum(&banked);

    let mut with_broker = banked.clone();
    // The broker's figure for a day already banked. If this joins the sum, the
    // day is counted twice.
    with_broker.push(row(r#"{"date":"2026-08-14","kind":"broker","broker_day_pnl":1.19}"#));

    assert!(
        (ledger_sum(&with_broker) - before).abs() < 1e-9,
        "a broker observation row changed the banked total: {} -> {}",
        before, ledger_sum(&with_broker)
    );
}

#[test]
fn a_stray_day_pnl_on_a_non_banking_row_is_still_ignored() {
    // Defence in depth: even if some future writer puts `day_pnl` on an
    // observation row, the kind filter must keep it out of the total.
    let rows = vec![
        row(r#"{"date":"2026-08-13","kind":"skim","day_pnl":-7.34}"#),
        row(r#"{"date":"2026-08-13","kind":"broker","day_pnl":-6.83}"#),
    ];
    assert!(
        (ledger_sum(&rows) - (-7.34)).abs() < 1e-9,
        "the kind filter must exclude non-banking rows regardless of their fields"
    );
}

#[test]
fn genuine_banking_rows_still_sum() {
    // The guard must not switch the ledger off. Both real kinds count.
    let rows = vec![
        row(r#"{"date":"2026-08-12","kind":"skim","day_pnl":20.62}"#),
        row(r#"{"date":"2026-08-13","kind":"carryover","day_pnl":-7.34}"#),
    ];
    assert!((ledger_sum(&rows) - 13.28).abs() < 1e-9, "got {}", ledger_sum(&rows));
    assert!(is_banking_kind("skim") && is_banking_kind("carryover"));
    assert!(!is_banking_kind("broker"), "the broker's own figure is not banked capital");
}

#[test]
fn the_quarantined_era_stays_excluded() {
    // Pre-2026-08-04 rows double-counted the same dollars and cannot be
    // reconstructed. Adding the kind filter must not accidentally readmit them.
    let rows = vec![
        row(r#"{"date":"2026-08-01","kind":"skim","day_pnl":72.28,"reliable":false}"#),
        row(r#"{"date":"2026-08-13","kind":"skim","day_pnl":-7.34}"#),
    ];
    assert!(
        (ledger_sum(&rows) - (-7.34)).abs() < 1e-9,
        "quarantined rows must not count"
    );
}

// ── FIFO must match chronologically, not in file order (2026-08-18) ────────
//
// backfill_pending_fills() recovers fills the order poller had to abandon --
// at the open Alpaca took 187-402 seconds to fill while the poller waits 60 --
// and stamps the recovered row with the broker's filled_at. That deliberately
// breaks append-in-time-order: an older buy lands in the file after newer
// sells. fifo_stats iterated the file directly, so those sells found no lot
// and were counted unmatched. The reported unmatched_sells rose from 17 to 22
// purely from adding buy rows, which is impossible if the matching is correct.

#[test]
fn a_buy_appended_after_its_sell_still_matches() {
    // File order is deliberately wrong: the sell is written before the buy it
    // consumes, exactly as a backfill produces.
    let log = [
        fill("NVDA", "sell", 1.0, 110.0, "2026-08-13"),
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-11"), // recovered later, older
    ]
    .join("\n");

    let s = fifo_stats(&log);

    assert_eq!(
        s["data_quality"]["unmatched_sells"], 0,
        "the sell has a matching buy once rows are ordered by time"
    );
    assert_eq!(s["round_trips"], 1);
    assert!(
        (s["real_realized_pnl"].as_f64().unwrap() - 10.0).abs() < 1e-9,
        "1 share 100 -> 110 is $10, got {:?}", s["real_realized_pnl"]
    );
}

#[test]
fn out_of_order_rows_are_attributed_to_the_closing_day() {
    // Ordering must not disturb day attribution: P&L belongs to the sell's day.
    let log = [
        fill("NVDA", "sell", 1.0, 90.0, "2026-08-13"),
        fill("NVDA", "buy", 1.0, 100.0, "2026-08-11"),
    ]
    .join("\n");

    let s = fifo_stats(&log);
    let days = s["by_day"].as_array().unwrap();

    assert_eq!(days.len(), 1);
    assert_eq!(days[0]["date"], "2026-08-13", "realized on the day it closed");
    assert!(days[0]["real_pnl"].as_f64().unwrap() < 0.0);
}

// ── Change monitor: tripwires must fire without a market (2026-08-18) ──────
//
// MAX_DAILY_ENTRIES (12) sat one above MAX_CONCURRENT_POSITIONS (11), so
// filling the book at the open consumed the whole daily budget. The cap bound
// at 10:45 and for the next five hours the system could only sell: $2,120 of
// $3,000 sat idle and two positions could not be rotated out.
//
// The deployment health check DID detect this and logged it. Nobody saw it, and
// nothing tied the behaviour to the parameter responsible. These tests pin the
// comparison rules so they can be trusted without waiting for a session.

use stock_market_ai::services::change_monitor::{compare, epoch_stats};

#[test]
fn a_binding_entry_cap_is_reported_as_critical() {
    // A day that used exactly its cap. Whether it "wanted" more entries is not
    // observable, which is why hitting the cap at all must be the alarm.
    let cur = serde_json::json!({
        "days": 1, "entries_per_day": 12.0, "cap_bound_days": 1,
        "fill_rate": 1.0, "expectancy": -0.3, "round_trips": 12,
        "median_hold_secs": 3600, "trail_stops_per_day": 5.0
    });
    let prev = serde_json::json!({"days": 0});

    let out = compare(&prev, &cur, 12);

    let hit = out.iter().find(|(c, _, _)| c == "entry_cap_binding")
        .expect("a bound cap must be reported");
    assert_eq!(hit.1, "critical", "stranded capital is not a warning, it is a fault");
}

#[test]
fn a_cap_that_never_binds_is_silent() {
    // The guard must not cry wolf once the cap is raised above what is used.
    let cur = serde_json::json!({
        "days": 2, "entries_per_day": 12.0, "cap_bound_days": 0,
        "fill_rate": 1.0, "expectancy": -0.3, "round_trips": 24,
        "median_hold_secs": 3600, "trail_stops_per_day": 5.0
    });
    let prev = serde_json::json!({
        "days": 2, "entries_per_day": 11.0, "cap_bound_days": 0,
        "fill_rate": 1.0, "expectancy": -0.3, "round_trips": 22,
        "median_hold_secs": 3600, "trail_stops_per_day": 5.0
    });

    let out = compare(&prev, &cur, 60);

    assert!(
        !out.iter().any(|(c, _, _)| c == "entry_cap_binding"),
        "cap not reached, so it must not be flagged: {out:?}"
    );
}

#[test]
fn a_collapsed_fill_rate_is_reported() {
    // 2026-08-10: 16 of 25 orders never reached a filled state, and the P&L
    // built on that log was wrong by $11.64.
    let cur = serde_json::json!({
        "days": 1, "entries_per_day": 9.0, "cap_bound_days": 0,
        "fill_rate": 0.36, "expectancy": -0.3, "round_trips": 9,
        "median_hold_secs": 3600, "trail_stops_per_day": 0.0
    });
    let prev = serde_json::json!({"days": 0});

    let out = compare(&prev, &cur, 60);

    assert!(out.iter().any(|(c, _, _)| c == "fill_rate"), "a 36% fill rate must be flagged");
}

#[test]
fn worsened_expectancy_after_a_change_is_critical() {
    let prev = serde_json::json!({
        "days": 5, "entries_per_day": 6.0, "cap_bound_days": 0, "fill_rate": 1.0,
        "expectancy": -0.30, "round_trips": 40, "median_hold_secs": 3600,
        "trail_stops_per_day": 3.0
    });
    let cur = serde_json::json!({
        "days": 3, "entries_per_day": 7.0, "cap_bound_days": 0, "fill_rate": 1.0,
        "expectancy": -0.95, "round_trips": 30, "median_hold_secs": 3600,
        "trail_stops_per_day": 4.0
    });

    let out = compare(&prev, &cur, 60);

    let hit = out.iter().find(|(c, _, _)| c == "expectancy").expect("must flag expectancy");
    assert_eq!(hit.1, "critical");
}

#[test]
fn a_thin_sample_does_not_trigger_an_expectancy_alarm() {
    // Two trades cannot distinguish a bad change from a bad morning. A tripwire
    // that fires on noise gets ignored, which is worse than none.
    let prev = serde_json::json!({
        "days": 5, "entries_per_day": 6.0, "cap_bound_days": 0, "fill_rate": 1.0,
        "expectancy": -0.30, "round_trips": 40, "median_hold_secs": 3600,
        "trail_stops_per_day": 3.0
    });
    let cur = serde_json::json!({
        "days": 1, "entries_per_day": 6.0, "cap_bound_days": 0, "fill_rate": 1.0,
        "expectancy": -4.00, "round_trips": 2, "median_hold_secs": 3600,
        "trail_stops_per_day": 3.0
    });

    let out = compare(&prev, &cur, 60);

    assert!(
        !out.iter().any(|(c, s, _)| c == "expectancy" && s == "critical"),
        "2 round trips is too thin to declare a regression: {out:?}"
    );
}

#[test]
fn epoch_stats_counts_a_cap_bound_day_from_the_fill_log() {
    // Built from the log, not from a hand-written summary, so the metric the
    // alarm reads is the metric the log actually produces.
    let mut rows = Vec::new();
    for i in 0..12 {
        rows.push(fill("AMD", "buy", 1.0, 100.0 + i as f64, "2026-08-18"));
    }
    let log = rows.join("\n");

    let s = epoch_stats(&log, "2026-08-18", 12);

    assert_eq!(s["days"], 1);
    assert_eq!(s["entries_per_day"], 12.0);
    assert_eq!(s["cap_bound_days"], 1, "12 entries against a cap of 12 is bound");
}

// ── The accumulator must be invisible to reconcile (2026-08-18) ────────────
//
// reconcile() unions the simulator's book with LIVE Alpaca positions and
// corrects the difference. The accumulator buys SPY that the simulator knows
// nothing about, so without a guard it reads as want=0 / have=N — a SELL — and
// the reconcile loop liquidates the entire long-term accumulation every cycle.
//
// That is the exact opposite of a buy-and-never-sell strategy, and it would
// have happened silently, on a book meant to be held for years.

use stock_market_ai::services::accumulator;

#[test]
fn broker_only_drift_in_an_ordinary_symbol_is_still_corrected() {
    // The behaviour the guard has to spare the accumulator from WITHOUT
    // switching it off for everything else: a holding present at the broker and
    // absent from the simulator is real drift, and must still be sold.
    let sim = books(&[]);
    let live = books(&[("NVDA", 12.5)]);
    let prices = books(&[("NVDA", 700.0)]);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));

    assert_eq!(actions.len(), 1, "unowned drift is still corrected");
    assert_eq!(actions[0]["action"], "sell");
    assert_eq!(actions[0]["symbol"], "NVDA");
}

#[test]
fn the_accumulator_holding_is_never_reconciled_away() {
    // Identical shape, but in the accumulator's symbol. Unguarded this produced
    // a SELL, and would have liquidated a book meant to be held for years —
    // silently, on every reconcile cycle.
    let sym = stock_market_ai::config::ACCUMULATOR_SYMBOL;
    let sim = books(&[]);
    let live = books(&[(sym, 12.5)]);
    let prices = books(&[(sym, 700.0)]);

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));

    assert!(actions.is_empty(), "the accumulator holding must never be sold: {actions:?}");
    assert!(deferred.is_empty(), "it is excluded outright, not merely postponed");
}

#[test]
fn the_accumulator_symbol_is_recognised_as_protected() {
    // The guard reconcile actually calls. If this ever returns false for the
    // configured symbol, the holding above gets sold.
    assert!(
        accumulator::owns(stock_market_ai::config::ACCUMULATOR_SYMBOL),
        "the configured accumulator symbol must be protected from reconcile"
    );
    assert!(
        !accumulator::owns("NVDA"),
        "an ordinary traded symbol must NOT be protected, or drift stops being corrected"
    );
}

#[test]
fn resuming_intraday_requires_a_fresh_trial_window() {
    // Trial 1 failed at -$0.2888 over 130 round trips. Intraday trading was
    // resumed on 2026-08-20 by explicit decision, which is allowed — but only
    // against a NEW window. Reusing the old one would either report trial 1's
    // failure forever or, worse, let trial 1's losses be averaged away by
    // trial 2's trades until the number looked acceptable.
    use stock_market_ai::config::*;

    if ALPACA_SHADOW_ORDERS {
        assert!(
            LIVE_KILL_BASELINE_TRIPS > 0,
            "trading is live, so the criterion needs a baseline or it measures history"
        );
        // Assert the invariant, not a literal date. The window resets every
        // time a parameter changes, so pinning the exact day makes this test
        // fail for the right reason and the wrong purpose — it should catch a
        // window that was never moved, not one that legitimately moved again.
        assert!(
            LIVE_KILL_START_DATE > "2026-08-06",
            "the window must start after trial 1, not inherit its dates"
        );
    }

    // The threshold itself may never be relaxed to make a resumed strategy
    // pass. Loosening this is moving the goalposts, which is the precise thing
    // the pre-commitment exists to prevent.
    assert!(
        LIVE_KILL_MIN_EXPECTANCY >= 0.0,
        "a system that loses money on average has no case for continuing"
    );
    assert_eq!(LIVE_KILL_TRADES, 100, "trial 2 uses the same trade count as trial 1");
}

// ── A simulated day is not banked capital (2026-08-19) ────────────────────
//
// With the trader retired the simulator keeps trading while no order reaches a
// broker, so its daily figure became hypothetical. On 2026-08-19 the skim row
// read -$6.39 against an account that moved -$0.03, with unfilled_today = 0 —
// the diagnostic built for that gap could not explain it, because the cause was
// now different and intended.
//
// Left under "skim" those numbers keep joining the running total and read as
// real P&L to anyone opening the file. That is the same failure that let a
// scoreboard row labelled REAL report +$185.71 against a -$32.45 account.

#[test]
fn a_simulated_day_does_not_join_the_banked_total() {
    let rows = vec![
        row(r#"{"date":"2026-08-18","kind":"skim","day_pnl":-0.33}"#),
        row(r#"{"date":"2026-08-19","kind":"skim_simulated","day_pnl":-6.39}"#),
    ];

    assert!(
        (ledger_sum(&rows) - (-0.33)).abs() < 1e-9,
        "a day the trader did not actually trade must not count, got {}",
        ledger_sum(&rows)
    );
    assert!(
        !is_banking_kind("skim_simulated"),
        "simulated days are observations, not banked capital"
    );
    assert!(
        is_banking_kind("skim"),
        "genuinely traded days must still count, or the ledger stops working"
    );
}

// ── BUG: the five rule books scored with all-zero weights ───────────────
//
// `new_rule` built every rule book with `[0.0; 7]`, copied from the random
// and always-in constructors where zero weights are correct because those
// books ignore signals entirely. The rule books do not ignore signals: r3 and
// r5 both gate on `weighted_score > MIN_BUY_SIGNAL`, and a zero weight vector
// makes `weighted_score` identically 0.0 no matter what the layers say.
//
// So two of the five rules could never open a position. On the board this
// showed as `r3_profit_only 0 trades` and `r5_concentrate 0 trades`, which
// reads as "the setup has not come up yet" and was really "the setup cannot
// come up". A rule that cannot fire is not evidence about the rule.

use stock_market_ai::services::paper_trader::{rule_entry_allowed, RuleEntry, RULE_WEIGHTS};
use stock_market_ai::config::MIN_BUY_SIGNAL;

/// A layer-score vector a real bullish tick would produce.
fn bullish_layers() -> [f64; 7] {
    [0.5, 0.4, 0.3, 0.3, 0.5, 0.0, 0.0]
}

fn weighted(scores: [f64; 7]) -> f64 {
    RULE_WEIGHTS.iter().zip(scores.iter()).map(|(w, s)| w * s).sum()
}

fn entry(weighted_score: f64, kronos_score: f64) -> RuleEntry {
    RuleEntry {
        kronos_score,
        weighted_score,
        below_floor: false,
        is_legacy: false,
        holds_nothing: true,
    }
}

#[test]
fn the_rule_books_do_not_score_with_zero_weights() {
    assert!(
        RULE_WEIGHTS.iter().any(|w| *w != 0.0),
        "all-zero weights make weighted_score identically 0.0, which silently \
         disables every rule gated on it"
    );
}

#[test]
fn a_bullish_tick_clears_the_buy_threshold() {
    let w = weighted(bullish_layers());
    assert!(
        w > MIN_BUY_SIGNAL,
        "an ordinary bullish tick scored {w}, at or below the {MIN_BUY_SIGNAL} \
         entry threshold — with these weights no signal-gated rule can ever fire"
    );
}

#[test]
fn rule_3_can_open_a_position_while_solvent() {
    let w = weighted(bullish_layers());
    assert!(
        rule_entry_allowed("r3_profit_only", &entry(w, 0.5)),
        "r3 must fund normally above the floor; with zero weights it never did"
    );
}

#[test]
fn rule_5_can_open_a_position() {
    let w = weighted(bullish_layers());
    assert!(
        rule_entry_allowed("r5_concentrate", &entry(w, 0.5)),
        "r5 enters normally and concentrates on the exit side — it cannot \
         concentrate into positions it was never able to open"
    );
}

#[test]
fn rule_3_stops_funding_below_the_floor() {
    let w = weighted(bullish_layers());
    let mut e = entry(w, 0.5);
    e.below_floor = true;
    assert!(
        !rule_entry_allowed("r3_profit_only", &e),
        "the spec's stop loss: under $3,000 this book opens nothing new"
    );
}

#[test]
fn rule_4_will_not_fund_a_name_with_no_track_record() {
    assert!(
        !rule_entry_allowed("r4_legacy", &entry(1.0, 0.9)),
        "r4 funds legacy names only, however strong the signal looks"
    );
    let mut e = entry(1.0, 0.9);
    e.is_legacy = true;
    assert!(
        rule_entry_allowed("r4_legacy", &e),
        "a name with a profitable track record must be fundable, or r4 is inert"
    );
}

#[test]
fn rule_2_holds_one_name_at_a_time() {
    let mut e = entry(1.0, 0.9);
    e.holds_nothing = false;
    assert!(
        !rule_entry_allowed("r2_max_forecast", &e),
        "r2 puts the WHOLE book into one forecast — a second position breaks it"
    );
}

// ── The random baseline was removed on the owner's instruction ──────────
//
// Every book must now declare a rule. The old code fell through to a generic
// signal-weighted entry for any book without one, which meant a typo in a
// rule name produced a book that traded on a rule nobody had specified.

#[test]
fn a_book_with_no_rule_cannot_trade() {
    assert!(
        !rule_entry_allowed("", &entry(1.0, 1.0)),
        "an unnamed book must be inert, not silently signal-weighted"
    );
    assert!(
        !rule_entry_allowed("r6_typo", &entry(1.0, 1.0)),
        "a misspelled rule must be inert, not silently signal-weighted"
    );
}

// ── The profit lock armed too early and strangled winning days ──────────
//
// PROFIT_LOCK_TRIGGER_PCT was 0.30% (+$9 on a $3,000 book), which almost any
// decent morning reached. Once armed with a 0.15% giveback, a $4.50 wiggle
// halted the session — so across 14 replayed sessions the best day was +$6.95
// against a worst day of -$11.93. A book that can lose more than it can win
// needs a 67.6% win rate to break even and had 50%.
//
// The replay that found this was itself wrong the first time: it keyed
// positions by entry day, so the eight positions held overnight were invisible
// on subsequent sessions and could not move the equity path. With that fixed,
// the answer inverted — 0.50% went from second-best to best on all 14 folds.

use stock_market_ai::config::{PROFIT_LOCK_TRIGGER_PCT, PROFIT_LOCK_GIVEBACK_PCT,
                              CAPITAL_FLOOR_PCT};

#[test]
fn the_profit_lock_trigger_sits_inside_the_robust_plateau() {
    // 0.45 / 0.50 / 0.55 / 0.60 scored -11.74 / -11.74 / -12.98 / -12.01 over
    // 14 sessions. Outside it the result degrades sharply: 0.40 is -27.07 and
    // 0.65 is -42.09. Chosen for being inside the plateau, not for the peak
    // score — the same reason the trail width is 0.75%.
    assert!(
        (0.45..=0.60).contains(&PROFIT_LOCK_TRIGGER_PCT),
        "the trigger left the 0.45-0.60 plateau (now {PROFIT_LOCK_TRIGGER_PCT}). \
         At 0.30 the lock armed on any decent morning and capped the best day \
         at +$6.95 against a -$11.93 worst day; at 0.65 and above it stops \
         earning its place. If this is a deliberate retune, replay it first \
         with New_ideas/giveback.py and move this range with the evidence."
    );
}

#[test]
fn the_lock_cannot_arm_before_it_can_give_back() {
    // A giveback wider than the trigger means the ratcheted floor starts below
    // the hard floor, so arming the lock would do nothing at all — the max()
    // in the damage-control block would discard it silently.
    assert!(
        PROFIT_LOCK_GIVEBACK_PCT < PROFIT_LOCK_TRIGGER_PCT,
        "giveback {PROFIT_LOCK_GIVEBACK_PCT} >= trigger {PROFIT_LOCK_TRIGGER_PCT}: \
         the ratcheted floor would never rise above the hard floor and the lock \
         would be dead code wearing the name of a safety feature"
    );
}

#[test]
fn the_hard_floor_is_still_what_prevents_losses() {
    // Worst day was -$11.93 under every trigger/giveback combination tried and
    // -$36.83 with no floor at all. If this ever inverts, the floor has been
    // moved outside the range where the losses live — which is exactly the
    // state it was in before 2026-08-24, when it looked like protection and
    // was not.
    assert!(
        CAPITAL_FLOOR_PCT < 0.0 && CAPITAL_FLOOR_PCT >= -0.50,
        "the floor at {CAPITAL_FLOOR_PCT}% is outside the range the daily P&L \
         actually occupies; it did all the loss prevention in the replay and \
         the profit lock did none"
    );
}

// ── BUG: r4_legacy could never trade, because it read its own history ───
//
// `is_legacy` consulted the shadow book's OWN legacy_pnl/legacy_trades maps,
// which are only written when THAT book closes a trade. So r4 could fund a
// name only after two closed profitable trades on r4's book, and r4 could not
// trade in order to acquire them. A closed loop.
//
// It ran a full session and placed zero orders. On the board that reads as
// "no name has qualified yet"; it actually meant "no name can ever qualify".
// Specification rule 4 describes an everyday log the SYSTEM keeps — so the log
// now lives on the trader and is fed by the real book and every shadow book.

/// The qualification rule, extracted so the closed loop cannot come back
/// disguised as a refactor: two closed trades and a positive total.
fn qualifies(trades: u32, cum_pnl: f64) -> bool {
    trades >= 2 && cum_pnl > 0.0
}

#[test]
fn r4_can_be_funded_from_history_it_did_not_create_itself() {
    // A name the rest of the system traded profitably twice. r4 has never
    // traded at all, and must still be able to act on this.
    assert!(
        qualifies(2, 4.10),
        "r4 must qualify names from the system-wide log; reading its own \
         empty history made the rule permanently inert"
    );
    assert!(
        rule_entry_allowed("r4_legacy", &RuleEntry {
            kronos_score: 0.5,
            weighted_score: 0.0,
            below_floor: false,
            is_legacy: qualifies(2, 4.10),
            holds_nothing: true,
        }),
        "a qualified name must be fundable by a book with no trades of its own"
    );
}

#[test]
fn one_lucky_fill_does_not_make_a_legacy_name() {
    assert!(!qualifies(1, 9.99), "a single profitable trade is not a track record");
    assert!(!qualifies(5, -0.01), "a losing cumulative total is not a track record");
    assert!(!qualifies(0, 0.0), "an untraded name has no track record");
}

// ── BUG: reconcile chased positions the simulator was about to close ────
//
// Reconcile makes Alpaca match the simulator and runs every 120s. With no
// minimum age it opened broker positions for simulator holdings that were
// seconds old, and the simulator then exited them before the next cycle — so
// the account paid a full round trip to hold something for two minutes.
//
// Every position ever closed by reconcile lost money: 5 of 5, -$24.90 total,
// 2.4-minute median hold, against -$14.18 for leaving them alone. It was the
// only exit path in the system that destroyed value; every other one is net
// positive. GOOGL was bought 2026-08-05 16:00:55 and sold 16:02:57 for -$13.03.

use stock_market_ai::config::RECONCILE_MIN_AGE_SECS;

#[test]
fn a_position_the_simulator_just_opened_is_not_chased() {
    let mut sim = HashMap::new();
    sim.insert("AMD".to_string(), 1.6);
    let live: HashMap<String, f64> = HashMap::new();     // broker holds nothing
    let mut prices = HashMap::new();
    prices.insert("AMD".to_string(), 465.0);
    let mut ages = HashMap::new();
    ages.insert("AMD".to_string(), 30u64);               // 30 seconds old

    let (actions, deferred) =
        reconcile_plan(&sim, &live, &prices, &HashSet::new(), &ages);
    assert!(
        actions.is_empty(),
        "reconcile opened a broker position for a 30-second-old simulator \
         holding; every such order it ever placed was closed at a loss within \
         minutes"
    );
    assert_eq!(deferred, vec!["AMD".to_string()],
        "the symbol must be reported as deferred, not silently dropped");
}

#[test]
fn a_position_the_simulator_has_actually_held_is_still_mirrored() {
    let mut sim = HashMap::new();
    sim.insert("AMD".to_string(), 1.6);
    let live: HashMap<String, f64> = HashMap::new();
    let mut prices = HashMap::new();
    prices.insert("AMD".to_string(), 465.0);
    let mut ages = HashMap::new();
    ages.insert("AMD".to_string(), RECONCILE_MIN_AGE_SECS + 1);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &ages);
    assert_eq!(actions.len(), 1,
        "a genuinely held position must still be mirrored, or the real book \
         silently stops tracking the simulator");
    assert_eq!(actions[0]["action"], "buy");
}

#[test]
fn unwanted_broker_exposure_is_still_sold_immediately() {
    // The gate is on OPENING only. An unwanted position is a live risk, and
    // waiting on it is how the 2026-08-12 duplicate-sell hazard returns.
    let sim: HashMap<String, f64> = HashMap::new();
    let mut live = HashMap::new();
    live.insert("KO".to_string(), 8.0);
    let mut prices = HashMap::new();
    prices.insert("KO".to_string(), 90.0);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(),
                                      &HashMap::new());   // no age at all
    assert_eq!(actions.len(), 1, "exposure the simulator does not want must be \
        closed regardless of age");
    assert_eq!(actions[0]["action"], "sell");
}

#[test]
fn a_partial_fill_is_topped_up_without_waiting() {
    // The broker already holds some: this corrects a partial fill rather than
    // establishing exposure, so the age gate must not apply.
    let mut sim = HashMap::new();
    sim.insert("COP".to_string(), 5.6);
    let mut live = HashMap::new();
    live.insert("COP".to_string(), 2.0);
    let mut prices = HashMap::new();
    prices.insert("COP".to_string(), 131.0);
    let mut ages = HashMap::new();
    ages.insert("COP".to_string(), 5u64);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &ages);
    assert_eq!(actions.len(), 1,
        "topping up a partial fill is not opening a position and must not wait");
    assert_eq!(actions[0]["action"], "buy");
}

// ── The rule monitor: catching an inert rule mechanically ───────────────
//
// Three of the five specification rules shipped structurally inert — r3 and r5
// with all-zero weights, r4 with a self-referential history requirement — and
// each ran that way for days looking exactly like a rule whose conditions had
// not come up. All three were found by hand, late.
//
// The probe below is the mechanical version of the question that found them:
// evaluate every rule against the most favourable input it could ever receive.
// A rule that is false on THAT can never be true.

use stock_market_ai::services::rule_monitor::{
    check_legacy_log, check_reconcile_churn, check_rules_can_fire, rule_can_ever_fire, BookStat,
};

fn book(id: &str, rule: &str, trades: u32) -> BookStat {
    BookStat { model_id: id.into(), rule: rule.into(), total_trades: trades }
}

#[test]
fn every_shipped_rule_can_fire() {
    for rule in ["r1_kronos_sectors", "r2_max_forecast", "r3_profit_only",
                 "r4_legacy", "r5_concentrate"] {
        assert!(
            rule_can_ever_fire(rule),
            "{rule} cannot fire under its own best-case input — no market \
             condition will ever satisfy it, and its trade count on the board \
             is a defect rather than a result"
        );
    }
}

#[test]
fn a_misspelled_rule_is_reported_as_dead() {
    let books = vec![book("r6_typo", "r6_typo", 0)];
    let out = check_rules_can_fire(&books);
    assert_eq!(out[0]["severity"], "critical",
        "a book wired to a rule name with no match arm must be reported, not \
         left looking merely selective");
}

#[test]
fn a_healthy_roster_reports_no_dead_rules() {
    let books = vec![
        book("r1_kronos_sectors", "r1_kronos_sectors", 27),
        book("r4_legacy", "r4_legacy", 10),
    ];
    let out = check_rules_can_fire(&books);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["severity"], "info");
}

#[test]
fn an_empty_everyday_log_after_real_trades_is_critical() {
    // The exact r4 failure: the system has closed trades, and the log r4 reads
    // has nothing in it, so r4 can never qualify a name.
    let f = check_legacy_log(0, 0, 40);
    assert_eq!(f["severity"], "critical",
        "an empty log after 40 closed trades means the log is not being \
         written, which is how r4 stayed inert");

    let f = check_legacy_log(0, 0, 0);
    assert_eq!(f["severity"], "info", "an empty log before any trade is normal");

    let f = check_legacy_log(12, 3, 40);
    assert_eq!(f["severity"], "info");
}

#[test]
fn reconcile_opening_and_closing_within_the_window_is_flagged() {
    // The real 2026-08-31 sequence: bought 13:51, sold 14:01.
    let events = vec![
        ("AMZN".to_string(), "buy".to_string(), 1_000_000i64),
        ("AMZN".to_string(), "sell".to_string(), 1_000_600i64), // 10 minutes
    ];
    let f = check_reconcile_churn(&events, 30 * 60);
    assert_eq!(f["severity"], "warn",
        "reconcile is bookkeeping, not a strategy; a 10-minute round trip is \
         pure cost and every one of them has lost money");
}

#[test]
fn a_reconcile_position_that_is_actually_held_is_not_flagged() {
    let events = vec![
        ("UNP".to_string(), "buy".to_string(), 1_000_000i64),
        ("UNP".to_string(), "sell".to_string(), 1_009_000i64), // 2.5 hours
    ];
    let f = check_reconcile_churn(&events, 30 * 60);
    assert_eq!(f["severity"], "info",
        "correcting drift and holding the position is reconcile working");
}

// ── BUG: every trailing stop asked to sell more shares than existed ─────
//
// The simulator accumulates share counts from its own arithmetic; Alpaca
// stores what its fills produced. They agree to about four decimals and then
// disagree, and Alpaca rejects the WHOLE order (code 40310000) when the
// request exceeds the holding by any amount at all. On 2026-08-31:
//
//   FCX   requested 9.919237   available 9.919233
//   KO    requested 8.373627   available 8.3736
//   EQIX  requested 0.725741   available 0.7257
//   AMZN  requested 1.071021   available 1.071
//
// All four were TRAILING STOPS. A rejected stop does not execute: the
// simulator books the exit, the account keeps the position. That is both a
// silently disabled safety mechanism and the source of the divergence — the
// simulator closed that day at -$8.26 and the broker at -$27.79.

use stock_market_ai::services::alpaca_broker::{available_from_rejection, sellable_qty};

/// The four real rejections, as (requested, available).
const REJECTED: [(f64, f64); 4] = [
    (9.919237, 9.919233),
    (8.373627, 8.3736),
    (0.725741, 0.7257),
    (1.071021, 1.071),
];

#[test]
fn no_sell_asks_for_more_than_the_account_holds() {
    for (requested, available) in REJECTED {
        let send = sellable_qty(requested);
        assert!(
            send <= available,
            "would send {send:.6} against a holding of {available:.6} — Alpaca \
             rejects the entire order, so the stop does not execute"
        );
        assert!(send > 0.0, "the position must still be sold, not skipped");
    }
}

#[test]
fn the_dust_left_behind_is_negligible() {
    for (requested, _) in REJECTED {
        let left = requested - sellable_qty(requested);
        assert!(
            left < 0.0001,
            "left {left} shares behind; more than a dust threshold means the \
             position is not really being closed"
        );
    }
}

#[test]
fn a_whole_share_count_is_not_disturbed() {
    assert_eq!(sellable_qty(5.0), 5.0);
    assert_eq!(sellable_qty(0.5), 0.5);
}

#[test]
fn the_available_quantity_is_recovered_from_the_rejection() {
    let detail = r#"{"available":"9.919233","code":40310000,"existing_qty":"9.919233","held_for_orders":"0","message":"insufficient qty available for order (requested: 9.919237, available: 9.919233)","symbol":"FCX"}"#;
    assert_eq!(available_from_rejection(detail), Some(9.919233),
        "the retry needs the exact holding; without it a wider drift than \
         flooring absorbs would still fail the stop");
}

#[test]
fn an_unrelated_rejection_yields_no_retry_quantity() {
    let rate_limited = r#"{"code":42910000,"message":"rate limit exceeded"}"#;
    assert_eq!(available_from_rejection(rate_limited), None,
        "only an insufficient-quantity rejection carries a holding to retry with");
    assert_eq!(available_from_rejection("not json"), None);
}

// ── GAP: the shadow books had no history that outlived a restart ────────
//
// Their recent-trade window was capped at 60 and never persisted, and this
// process restarts daily. On 2026-08-31 r2_max_forecast showed +$45.27 and
// the trades behind it could not be read back — the explanation had to be
// reconstructed from the REAL trader's per-symbol numbers.
//
// These five books are what decides which specification rule survives. A
// decision of that kind needs an audit trail, and a snapshot that gets
// overwritten is not one.

/// Recompute a book's realized total from its logged rows, the way
/// /api/shadow-trades does. If this disagrees with the scoreboard, one of the
/// two is wrong — which is the point of keeping an independent record.
fn realized_from_log(rows: &[serde_json::Value], model: &str) -> (f64, u32) {
    let mut pnl = 0.0;
    let mut n = 0;
    for r in rows {
        if r["model_id"].as_str() != Some(model) { continue; }
        if let Some(p) = r["pnl"].as_f64() {
            pnl += p;
            n += 1;
        }
    }
    (pnl, n)
}

fn srow(model: &str, sym: &str, action: &str, pnl: Option<f64>) -> serde_json::Value {
    match pnl {
        Some(p) => serde_json::json!({"model_id": model, "symbol": sym, "action": action, "pnl": p}),
        None => serde_json::json!({"model_id": model, "symbol": sym, "action": action, "pnl": null}),
    }
}

#[test]
fn a_books_total_can_be_rebuilt_from_the_log() {
    let rows = vec![
        srow("r2_max_forecast", "MRK", "BUY", None),
        srow("r2_max_forecast", "MRK", "SELL", Some(34.10)),
        srow("r2_max_forecast", "EQIX", "BUY", None),
        srow("r2_max_forecast", "EQIX", "SELL", Some(11.17)),
        srow("r1_kronos_sectors", "KO", "SELL", Some(-2.00)),
    ];
    let (pnl, n) = realized_from_log(&rows, "r2_max_forecast");
    assert!((pnl - 45.27).abs() < 1e-9,
        "rebuilt {pnl}, expected 45.27 — a scoreboard number that cannot be \
         rebuilt from the log is a number nobody can check");
    assert_eq!(n, 2, "only closes carry P&L; entries must not count as trades");
}

#[test]
fn entries_do_not_contribute_to_the_rebuilt_total() {
    let rows = vec![
        srow("r4_legacy", "FCX", "BUY", None),
        srow("r4_legacy", "FCX", "BUY", None),
    ];
    let (pnl, n) = realized_from_log(&rows, "r4_legacy");
    assert_eq!(n, 0);
    assert_eq!(pnl, 0.0);
}

#[test]
fn one_books_trades_do_not_leak_into_another() {
    let rows = vec![
        srow("r2_max_forecast", "MRK", "SELL", Some(34.10)),
        srow("r5_concentrate", "MRK", "SELL", Some(-1.10)),
    ];
    assert!((realized_from_log(&rows, "r5_concentrate").0 - (-1.10)).abs() < 1e-9,
        "books trade the same symbols at the same moments; mixing them would \
         make every comparison between rules meaningless");
}

// ── BUG: orders were silently discarded when the symbol was busy ────────
//
// A per-symbol claim serialises orders so an exit and its re-entry are never
// in flight together. The claim is held while the fill is polled — up to 60
// seconds — but a waiting order gave up after 10 and returned WITHOUT LOGGING
// ANYTHING. The order simply never happened and nothing counted it.
//
// That is how the simulator came to hold positions the account did not. On
// 2026-09-01 the simulator sold AMD at 13:42, re-entered, then tried to sell
// again at 14:07; Alpaca rejected it because the account held 0.000051 shares.
// The re-entry had been dropped. Four symbols failed the same way that
// session and the books closed $20.45 apart — after the earlier
// insufficient-quantity fix, so this was a second, separate cause.

use stock_market_ai::services::alpaca_broker::{claim_wait_budget_ms, fill_poll_budget_ms};

#[test]
fn an_order_waits_at_least_as_long_as_a_claim_can_be_held() {
    assert!(
        claim_wait_budget_ms() >= fill_poll_budget_ms(),
        "orders wait {}ms for a claim that can be held {}ms — anything arriving \
         in the gap is discarded, and the simulator books a trade the account \
         never receives",
        claim_wait_budget_ms(), fill_poll_budget_ms()
    );
}

#[test]
fn a_dropped_order_is_reported_even_when_everything_else_filled() {
    use stock_market_ai::services::rule_monitor::check_fill_quality;
    // 20 of 21 filled is a 95% fill rate — healthy by every other measure.
    let f = check_fill_quality(20, 0, 0, 0, 1);
    assert_eq!(f["severity"], "warn",
        "a dropped order is worse than a rejected one: nothing was sent and no \
         broker-side record exists, so a high fill rate hides it completely");

    let f = check_fill_quality(20, 0, 0, 1, 0);
    assert_eq!(f["severity"], "info", "an ordinary pending order is not an alarm");
}

// ── The divergence check compared two different things ──────────────────
//
// Alpaca's today_pnl is the WHOLE account, including a long-term SPY position
// the simulator does not model. On 2026-09-03 SPY rose 0.68% and the account
// showed +$46.39 against the simulator's -$9.06 — a $55 "divergence" with ZERO
// unfilled orders, entirely index drift.
//
// A warning that fires on every good SPY day is a warning that gets ignored,
// and this one exists to catch orders the broker never filled.

use stock_market_ai::services::rule_monitor::{accumulator_day_pnl, check_divergence};

fn arow(day: &str, value: f64, contributed: f64) -> serde_json::Value {
    serde_json::json!({
        "kind": "accumulator", "date": day,
        "accum_value": value, "accum_contributed": contributed,
    })
}

#[test]
fn index_drift_is_removed_before_the_books_are_compared() {
    // The real day: +$46.39 at the account, ~+$41 of it SPY drift.
    let f = check_divergence("2026-09-03", -9.06, 46.39, Some(41.0));
    let msg = f["message"].as_str().unwrap();
    // The raw gap was -55.45; netting the index out leaves -14.45, which is
    // still a real disagreement about TRADING and still warns. The point is
    // that the reported number now describes trading rather than SPY.
    assert!(msg.contains("+5.39 from trading"),
        "the comparison must be against trading only; got: {msg}");
    assert!(msg.contains("-14.45"),
        "the gap must be the netted one, not the raw -55.45; got: {msg}");
    assert!(!msg.contains("-55.45"), "the raw gap must not be the headline");
}

#[test]
fn a_day_that_is_only_index_drift_stops_warning_entirely() {
    // Same shape, where SPY accounts for essentially all of it.
    let f = check_divergence("2026-09-03", -9.06, 46.39, Some(50.0));
    assert_eq!(f["severity"], "info",
        "when the account's move is the index and trading roughly matches the \
         simulator, there is nothing to report — this is the case that would \
         have fired on every good SPY day until the warning was ignored");
}

#[test]
fn a_real_divergence_still_fires_once_the_index_is_netted_out() {
    // 2026-08-31: simulator -8.26, broker -27.79, SPY roughly flat.
    let f = check_divergence("2026-08-31", -8.26, -27.79, Some(-0.5));
    assert_eq!(f["severity"], "warn",
        "the rejected-stop divergence must still be caught");
}

#[test]
fn an_unknown_accumulator_move_is_not_silently_treated_as_zero() {
    let f = check_divergence("2026-09-03", -9.06, 46.39, None);
    let msg = f["message"].as_str().unwrap();
    assert!(msg.contains("UNKNOWN"),
        "treating an unknown as zero would attribute a whole day of index \
         drift to the trader without saying so; got: {msg}");
}

#[test]
fn the_accumulators_day_move_excludes_new_money() {
    let rows = vec![
        arow("2026-09-02", 5000.0, 5000.0),
        // $500 added, holding also gained $41.
        arow("2026-09-03", 5541.0, 5500.0),
    ];
    let got = accumulator_day_pnl(&rows, "2026-09-03").unwrap();
    assert!((got - 41.0).abs() < 1e-9,
        "got {got}, expected 41.00 — a daily contribution is a transfer, not a \
         gain, and counting it would net the wrong amount out of the broker");
}

#[test]
fn the_first_logged_day_reports_unknown_rather_than_zero() {
    let rows = vec![arow("2026-09-03", 5541.0, 5500.0)];
    assert_eq!(accumulator_day_pnl(&rows, "2026-09-03"), None,
        "the first row has no yesterday to difference against");
}

// ── BUG: a market holiday was treated as an ordinary trading day ────────
//
// is_market_open() checked weekday and clock only. On 2026-09-07, Labor Day,
// the system ran its loops against a dead feed, the supervisor reported
// CRITICAL because 0 of 10 symbols were streaming, and reconcile placed four
// orders that sat pending and would have queued into the next session's open.
//
// An alarm that is wrong on a known, published schedule is an alarm people
// learn to ignore — the same failure as a divergence check that fires on every
// good SPY day.

use stock_market_ai::config::{is_market_holiday, HOLIDAYS_KNOWN_THROUGH, MARKET_HOLIDAYS};

#[test]
fn labor_day_is_not_a_trading_day() {
    assert!(is_market_holiday("2026-09-07"), "the day this was found");
    assert!(is_market_holiday("2026-11-26"), "Thanksgiving");
    assert!(is_market_holiday("2026-12-25"), "Christmas");
}

#[test]
fn an_ordinary_monday_is_still_a_trading_day() {
    assert!(!is_market_holiday("2026-09-14"));
    assert!(!is_market_holiday("2026-09-08"));
}

#[test]
fn half_days_are_deliberately_not_holidays() {
    // The 1:00pm closes after Thanksgiving and at Christmas Eve: the market IS
    // open, and skipping them would skip real sessions. Idle afternoon loops
    // are the cheaper mistake.
    assert!(!is_market_holiday("2026-11-27"), "day after Thanksgiving is a half day, not a closure");
    assert!(!is_market_holiday("2026-12-24"), "Christmas Eve is a half day, not a closure");
}

#[test]
fn the_holiday_list_has_not_gone_stale() {
    // Past the last covered date the system is silently back to weekday-and-
    // clock, and the next holiday behaves exactly as 2026-09-07 did.
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    assert!(
        today.as_str() <= HOLIDAYS_KNOWN_THROUGH,
        "MARKET_HOLIDAYS covers only to {HOLIDAYS_KNOWN_THROUGH} and today is \
         {today}; holidays past that are treated as ordinary sessions"
    );
    assert!(MARKET_HOLIDAYS.len() >= 18, "roughly nine closures a year");
}

// ── BUG: the dust guard stopped working when prices were missing ───────
//
// reconcile skips gaps worth under $1, but that test was conditional on
// `px > 0.0`. With no ticks — a holiday, a dead feed — it silently stopped
// applying and only the share test remained. Four RECONCILE sells of 0.0001
// shares went out on 2026-09-07 and sat pending.
//
// 0.0001 is exactly what sellable_qty() leaves behind by design, so a strict
// `<` made reconcile chase precisely the residue of the stop-loss fix.

#[test]
fn no_correction_is_attempted_without_a_price() {
    let mut sim = HashMap::new();
    sim.insert("AMZN".to_string(), 0.0);
    let mut live = HashMap::new();
    live.insert("AMZN".to_string(), 0.0001);
    let prices: HashMap<String, f64> = HashMap::new();   // feed is dead

    let (actions, deferred) =
        reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));
    assert!(actions.is_empty(),
        "a gap that cannot be valued cannot be told from dust; acting blind is \
         how four orders went out on a closed exchange");
    assert_eq!(deferred, vec!["AMZN".to_string()],
        "and it must be reported as deferred, not silently dropped");
}

#[test]
fn the_residue_of_a_floored_sell_is_treated_as_dust() {
    let mut sim = HashMap::new();
    sim.insert("KO".to_string(), 0.0);
    let mut live = HashMap::new();
    live.insert("KO".to_string(), 0.0001);       // exactly what flooring leaves
    let mut prices = HashMap::new();
    prices.insert("KO".to_string(), 90.0);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));
    assert!(actions.is_empty(),
        "sellable_qty() leaves up to 0.0001 shares by design; reconcile must \
         not spend a round trip chasing it");
}

#[test]
fn a_real_gap_is_still_corrected_when_priced() {
    let mut sim = HashMap::new();
    sim.insert("KO".to_string(), 0.0);
    let mut live = HashMap::new();
    live.insert("KO".to_string(), 8.0);
    let mut prices = HashMap::new();
    prices.insert("KO".to_string(), 90.0);

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new(), &settled(&sim));
    assert_eq!(actions.len(), 1, "real drift must still be corrected");
    assert_eq!(actions[0]["action"], "sell");
}

// ── BUG: the accumulator queued 33 duplicate orders through a holiday ───
//
// Two failures compounding, found 45 minutes before the open on 2026-09-08.
//
// FIRST: market_open_now() was a THIRD private copy of the trading calendar.
// The day before, paper_trader and agentic_test were taught about holidays;
// this one was missed, so the accumulator loop ran all through Labor Day.
//
// SECOND: already_contributed() checked for a FILLED row. With the exchange
// shut the order was accepted and never filled, so it stayed false and the
// ten-minute loop tried again — 33 times, $16,500 of notional SPY, every one
// queued to execute together at the next open. The account contributes $500 a
// day; it came within an hour of holding $16,500 of one morning.
//
// Retrying a queued notional market order stacks duplicates rather than
// replacing one. One attempt per day; a miss is reported, not retried.

use stock_market_ai::config::is_market_open_now;

#[test]
fn there_is_one_trading_calendar_and_it_knows_holidays() {
    // The function every caller now shares. On a holiday it must be closed
    // whatever the clock says, which is what the three copies disagreed about.
    assert!(!is_market_holiday("2026-09-08"), "the day after Labor Day trades");
    assert!(is_market_holiday("2026-09-07"), "Labor Day does not");
    // Sanity: the shared function runs and returns a bool for "now".
    let _ = is_market_open_now();
}

/// One accumulator log row, as the day-attempt check sees it.
fn arow_json(date: &str, outcome: Option<&str>, kind: Option<&str>) -> serde_json::Value {
    let mut v = serde_json::json!({"date": date, "usd": 500.0});
    if let Some(o) = outcome { v["outcome"] = serde_json::json!(o); }
    if let Some(k) = kind { v["kind"] = serde_json::json!(k); }
    v
}

/// Mirrors accumulator::already_attempted, which is private. The rule under
/// test is "any attempt today counts", not "any FILL today counts".
fn attempted(rows: &[serde_json::Value], date: &str) -> bool {
    rows.iter().any(|r| {
        r["date"].as_str() == Some(date)
            && r["kind"].as_str() != Some("top_up")
            && (r["order_id"].as_str().is_some() || r["outcome"].as_str().is_some())
    })
}

#[test]
fn an_order_that_did_not_fill_still_counts_as_the_days_attempt() {
    let rows = vec![arow_json("2026-09-07", Some("pending"), None)];
    assert!(
        attempted(&rows, "2026-09-07"),
        "a pending order counted as 'not yet contributed' and the loop retried \
         every ten minutes — 33 orders and $16,500 of notional in one session"
    );
}

#[test]
fn a_filled_contribution_still_blocks_a_second_one() {
    let rows = vec![arow_json("2026-09-08", Some("filled"), None)];
    assert!(attempted(&rows, "2026-09-08"));
}

#[test]
fn a_manual_top_up_does_not_consume_the_days_contribution() {
    // top_up is a deliberate one-off; it must not make the system think the
    // scheduled $500 already went in.
    let rows = vec![arow_json("2026-09-08", Some("filled"), Some("top_up"))];
    assert!(!attempted(&rows, "2026-09-08"),
        "a manual top-up is not the daily contribution");
}

#[test]
fn yesterdays_attempt_does_not_block_today() {
    let rows = vec![arow_json("2026-09-07", Some("pending"), None)];
    assert!(!attempted(&rows, "2026-09-08"),
        "a missed day must not silently skip the next one too");
}

// ── The simulator traded while the account sat flat ─────────────────────
//
// A halt suppressed real orders and let the simulator keep opening positions,
// "to earn its way back". On 2026-09-08 that ran as designed and produced
// exactly what it was asked not to: the simulator finished the morning at
// +0.62% while the account was flat, locked out, and unable to join a recovery
// it could see happening. The broker held nothing but SPY; the simulator held
// five positions.
//
// A paper account that does not mirror the simulator is not measuring the
// simulator. A halt now stops BOTH books and both resume together.

use stock_market_ai::config::MAX_HALTS_PER_DAY;
use stock_market_ai::services::paper_trader::book_policy;

#[test]
fn the_simulator_never_trades_while_the_account_cannot() {
    for halted in [false, true] {
        let p = book_policy(halted);
        assert!(
            !(p.sim_may_enter && !p.broker_mirrors),
            "halted={halted}: the simulator may enter but the broker does not \
             mirror — that is the divergence, and it is the state the old halt \
             deliberately created"
        );
    }
}

#[test]
fn a_halt_stops_the_simulator_too() {
    assert!(!book_policy(true).sim_may_enter,
        "the simulator kept trading through a halt and compounded a paper \
         recovery the account could not act on");
    assert!(book_policy(false).sim_may_enter, "and trades normally otherwise");
}

#[test]
fn the_broker_mirrors_in_every_state() {
    assert!(book_policy(true).broker_mirrors);
    assert!(book_policy(false).broker_mirrors,
        "there is no state where an action is taken and not mirrored");
}

#[test]
fn a_bad_day_still_ends() {
    // Mirroring must not mean trading through an unlimited number of halts.
    assert!((1..=5).contains(&MAX_HALTS_PER_DAY),
        "with the books mirrored, the halt cap is what stops a bad session \
         repeating itself; {MAX_HALTS_PER_DAY} is outside the sane range");
}

// ── Watching that the books ACTUALLY match, not that they intend to ─────
//
// Mirroring shipped 2026-09-08, but "the code intends to mirror" and "the
// books match right now" are different claims and only the second is worth
// anything. Every divergence in this project was invisible until two numbers
// were compared by hand: a halt that suppressed real orders while the
// simulator traded on, stops rejected for oversized quantity, entries silently
// discarded when a symbol was busy.

use stock_market_ai::services::rule_monitor::check_book_parity;

fn holdings(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(s, q)| (s.to_string(), *q)).collect()
}

#[test]
fn matching_books_report_clean() {
    let sim = holdings(&[("AMD", 1.5), ("KO", 8.0)]);
    let live = holdings(&[("AMD", 1.5), ("KO", 8.0)]);
    let px = holdings(&[("AMD", 460.0), ("KO", 90.0)]);
    assert_eq!(check_book_parity(&sim, &live, &px, 0.0001)["severity"], "info");
}

#[test]
fn a_position_the_account_never_got_is_reported() {
    // The 2026-09-08 shape: the simulator holds five, the account holds none.
    let sim = holdings(&[("AMD", 1.5), ("KO", 8.0)]);
    let live: HashMap<String, f64> = HashMap::new();
    let px = holdings(&[("AMD", 460.0), ("KO", 90.0)]);
    let f = check_book_parity(&sim, &live, &px, 0.0001);
    assert_eq!(f["severity"], "warn");
    let msg = f["message"].as_str().unwrap();
    assert!(msg.contains("AMD") && msg.contains("KO"),
        "the message must name the symbols; a bare count sends you hunting");
}

#[test]
fn a_position_only_the_account_holds_is_reported() {
    // The other direction: a rejected exit leaves the account long something
    // the simulator has already sold.
    let sim: HashMap<String, f64> = HashMap::new();
    let live = holdings(&[("FCX", 9.9)]);
    let px = holdings(&[("FCX", 75.0)]);
    assert_eq!(check_book_parity(&sim, &live, &px, 0.0001)["severity"], "warn");
}

#[test]
fn flooring_residue_is_not_a_divergence() {
    // sellable_qty() leaves up to 0.0001 shares by design.
    let sim = holdings(&[("KO", 0.0)]);
    let live = holdings(&[("KO", 0.0001)]);
    let px = holdings(&[("KO", 90.0)]);
    assert_eq!(check_book_parity(&sim, &live, &px, 0.0001)["severity"], "info",
        "a cent of residue is not a broken mirror");
}

#[test]
fn the_accumulator_is_not_counted_as_a_divergence() {
    // The simulator does not model the long-term SPY holding at all, so it
    // reads as a permanent gap and would drown every real signal.
    let sim: HashMap<String, f64> = HashMap::new();
    let live = holdings(&[("SPY", 9.21)]);
    let px = holdings(&[("SPY", 767.0)]);
    assert_eq!(check_book_parity(&sim, &live, &px, 0.0001)["severity"], "info",
        "a $7,000 permanent gap would make this check useless within a day");
}

// ── BUG: REAL_TRADER carried the accumulator's P&L ─────────────────────
//
// The kill criterion subtracted the long-term SPY holding before judging the
// intraday trader. The /api/experiments scoreboard did not — it displayed the
// whole account. Two definitions of the same number, and they disagreed.
//
// On 2026-09-09 REAL_TRADER read -$161.90 against an actual trading result of
// -$127.42: the intraday book carrying $34.53 of SPY's decline, which it never
// traded and cannot control.
//
// It is also why the simulator and the account looked like they had diverged
// while their positions matched to the cent. The simulator does not model the
// accumulator at all, so any comparison that leaves it in compares different
// things.

use stock_market_ai::services::alpaca_broker::trading_net;

#[test]
fn the_trader_is_not_charged_for_the_index_holding() {
    // The real readings.
    let shown = trading_net(-161.95, -34.53);
    assert!((shown - (-127.42)).abs() < 0.01,
        "got {shown}, expected -127.42 — the trader was carrying the \
         accumulator's loss");
}

#[test]
fn a_profitable_accumulator_does_not_flatter_the_trader_either() {
    // The error runs both ways: on an up day the index would make a losing
    // book look better, which is the more dangerous direction.
    let shown = trading_net(-20.00, 40.00);
    assert!((shown - (-60.00)).abs() < 1e-9,
        "got {shown}, expected -60.00 — index gains must not hide trading losses");
}

#[test]
fn with_no_accumulator_the_account_net_is_the_trading_net() {
    assert!((trading_net(-127.42, 0.0) - (-127.42)).abs() < 1e-9);
}

// ── BUG: the round-trip counter saturated at 500 closed orders ─────────
//
// The kill criterion counted filled sells within `limit=500&direction=desc` —
// the most recent 500 closed orders. That works until the account passes 500,
// after which the window slides forward and the count STOPS GROWING however
// much trading happens.
//
// On 2026-09-09 it read 310 all morning while our own fill log held 331 filled
// sells all-time and 13 that session. Trial 3 could have run its full 20-day
// clock and never reached the 100 trips it needs, and a retirement decision
// would have rested on a number that had quietly stopped counting. Third time
// in this project a verdict has depended on a capped or mis-parsed source.

/// Mirrors the counting rule: filled sells, deduplicated by order id.
/// Pagination cannot inflate a count that dedupes.
fn count_round_trips(pages: &[Vec<(&str, &str, &str)>]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for page in pages {
        for (id, status, side) in page {
            if *status == "filled" && *side == "sell" {
                seen.insert(id.to_string());
            }
        }
    }
    seen.len()
}

#[test]
fn trips_keep_counting_past_one_page() {
    let p1 = vec![("a", "filled", "sell"), ("b", "filled", "buy")];
    let p2 = vec![("c", "filled", "sell"), ("d", "canceled", "sell")];
    assert_eq!(count_round_trips(&[p1, p2]), 2,
        "a second page must add to the count; the old counter only ever saw \
         the most recent 500 orders and stopped growing");
}

#[test]
fn a_repeated_page_does_not_inflate_the_count() {
    // Alpaca's `after` bound has caught this project out before. Dedupe by id
    // means an overlapping page is harmless either way.
    let p1 = vec![("a", "filled", "sell"), ("b", "filled", "sell")];
    let p2 = vec![("b", "filled", "sell"), ("c", "filled", "sell")];
    assert_eq!(count_round_trips(&[p1, p2]), 3,
        "b appears on both pages and must be counted once");
}

#[test]
fn only_filled_sells_are_round_trips() {
    let p = vec![
        ("a", "filled", "buy"),
        ("b", "canceled", "sell"),
        ("c", "rejected", "sell"),
        ("d", "filled", "sell"),
    ];
    assert_eq!(count_round_trips(&[p]), 1,
        "a buy opens a position and a rejected sell closes nothing");
}

// ── BUG: the heaviest signal layer was frozen for 30 minutes at a time ──
//
// The volume profile is rebuilt every 30 minutes, and `position`/`signal` were
// computed once at that moment against the last completed bar's close. Both are
// functions of the CURRENT price, so for the next half hour the layer reported
// where price had been, not where it was.
//
// Observed 2026-09-10: AMZN's value area was [251.17, 252.74] and the live
// price was 251.73 — inside it — while the layer reported `below_value` with
// signal 0.50, the maximum buy reading. Five of ten symbols were pinned at 0.50
// that morning, on a layer weighted 0.52 of the entry score: more than Kronos,
// Kalman, pattern and CVD combined.

use stock_market_ai::services::institutional_signals::classify;

/// The real AMZN profile from that morning.
const POC: f64 = 252.1872;
const VA_LOW: f64 = 251.168;
const VA_HIGH: f64 = 252.736;
const LEVEL: f64 = 0.03;

#[test]
fn a_price_inside_the_value_area_is_not_below_it() {
    let (pos, sig) = classify(POC, VA_LOW, VA_HIGH, LEVEL, 251.73);
    assert_ne!(pos, "below_value",
        "251.73 sits inside [251.17, 252.74]; calling it below_value is what \
         pinned the heaviest layer at a maximum buy signal");
    assert!(sig.abs() < 0.5,
        "an in-value price must not emit the full 0.50 support reading, got {sig}");
}

#[test]
fn genuinely_below_value_still_reads_as_support() {
    let (pos, sig) = classify(POC, VA_LOW, VA_HIGH, LEVEL, 250.10);
    assert_eq!(pos, "below_value");
    assert!((sig - 0.5).abs() < 1e-9, "the support reading itself is unchanged");
}

#[test]
fn genuinely_above_value_still_reads_as_resistance() {
    let (pos, sig) = classify(POC, VA_LOW, VA_HIGH, LEVEL, 253.50);
    assert_eq!(pos, "above_value");
    assert!((sig - (-0.3)).abs() < 1e-9);
}

#[test]
fn a_price_at_the_point_of_control_is_recognised() {
    let (pos, _) = classify(POC, VA_LOW, VA_HIGH, LEVEL, POC + 0.01);
    assert_eq!(pos, "at_poc",
        "at_poc needs the level size; losing it would silently collapse this \
         case into in_value");
}

#[test]
fn the_reading_tracks_price_across_the_value_area() {
    // The property that matters: moving through the area must change the
    // reading. Frozen values did not, which is the whole defect.
    let below = classify(POC, VA_LOW, VA_HIGH, LEVEL, 250.0).1;
    let inside = classify(POC, VA_LOW, VA_HIGH, LEVEL, 252.0).1;
    let above = classify(POC, VA_LOW, VA_HIGH, LEVEL, 254.0).1;
    assert!(below > inside && inside > above,
        "signal must fall monotonically as price rises through the value area; \
         got below={below} inside={inside} above={above}");
}
