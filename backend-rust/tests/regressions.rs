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

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &busy);

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

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &HashSet::new());

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

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &busy);

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

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new());
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

    let (actions, _) = reconcile_plan(&sim, &live, &prices, &HashSet::new());

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

    let (actions, deferred) = reconcile_plan(&sim, &live, &prices, &HashSet::new());

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
fn the_intraday_trader_no_longer_places_real_orders() {
    // It failed its pre-committed criterion at -$0.2888 over 130 round trips.
    // The rule said retired, not retuned. The simulator keeps running; the
    // broker link is off. If someone flips this back on, that is a decision
    // that should have to be made deliberately.
    assert!(
        !stock_market_ai::config::ALPACA_SHADOW_ORDERS,
        "the intraday trader is retired; re-enabling it needs a new criterion"
    );
}
