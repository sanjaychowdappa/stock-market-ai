# Project Timeline & P&L History

**2026-05-19 → 2026-07-31 · 2.5 months · 54+ commits**
Generated from git history and `reports/` data. Last updated 2026-07-31.

---

## Phase 1 — Build-out (May 19 – Jun 12)

| Date | Milestone |
|---|---|
| 2026-05-19 | Project born — full-stack app (Rust/Axum backend + React frontend) |
| 2026-05-20 | WebSocket live streaming, dual-pane charts, RAG news agent |
| 2026-05-22 | **Kronos** transformer integrated for price prediction |
| 2026-05-25 | GPU-accelerated per-second Kronos predictions |
| 2026-06-05 | All **7 signal layers** wired end-to-end + per-layer decision monitoring |
| 2026-06-10 | v5 swing trading, $500 budget, old Python backend removed |
| 2026-06-11 | v6 prediction-driven multi-stock + **shadow A/B testing** framework |
| 2026-06-12 | **Random-entry baseline** added; signals moved from ticks → 1-minute bars |
| 2026-06-12 | Daily circuit breaker (−2%), partial profit booking |

## Phase 2 — The honest test (Jun 22 – Jun 26)

| Date | Milestone |
|---|---|
| 2026-06-22 | **v7 SWING MODE** — intraday fees were eating **137% of gross profit** |
| 2026-06-22 | Trend-filter A/B/C experiment; standalone `analyze.ps1` + dated archives |
| 2026-06-22 | S&P 500 Kronos scanner (Phase 1); fixed daily-bar fetch bug |
| 2026-06-24 | **State persistence** — swing positions survive overnight restarts |
| **2026-06-26** | **VERDICT: the 7-layer signal stack has NO EDGE.** The random baseline was the week's best performer. Filters were removing profitable exposure, not adding skill. |

## Phase 3 — The pivot (Jun 26 – Jul 20)

| Date | Milestone |
|---|---|
| 2026-06-26 | Cross-sectional **momentum portfolio** added, with a QQQ kill-date |
| 2026-06-29 | Momentum state persisted so the benchmark experiment survives restarts |
| 2026-07-01 | `always_in_max_exposure` model added (tests the exposure hypothesis) |
| 2026-07-20 | Momentum redesigned → **monthly ETF rotation** (tradeable mutual-fund proxy) |
| 2026-07-20 | ETF momentum becomes the primary dashboard; megacap trader demoted to "legacy" |

## Phase 4 — Aggressive scaling (Jul 20 – Jul 21)

| Date | Milestone |
|---|---|
| 2026-07-20 | Day trader re-enabled + **market-regime filter** (QQQ vs its 50-day SMA) |
| 2026-07-20 | **ATR-based dynamic exits** — stops/targets scale to each symbol's volatility |
| 2026-07-20 | **MAX_EXPOSURE_MODE** — deploy ~100% of capital when the market is risk-on |
| 2026-07-21 | Budget → **$3,000**, daily profit skim + reset, weekly reporting |
| 2026-07-21 | **exp1** launched (next-minute prediction) + Experiments dashboard tab |
| 2026-07-21 | **`claude_1`** — cost modeling, exp1 kill criterion, config freeze, automation |

## Phase 5 — The audit (Jul 28 – Jul 31)

| Date | Milestone |
|---|---|
| 2026-07-28 | Fixed cumulative-vs-daily bug that produced a fake "$60 profit" |
| 2026-07-28 | **`agentic_test`** module — autonomous operations supervisor + UI tab |
| 2026-07-29 | Made the fixed-capital invariant durable (sync writes + enforced day-start reset) |
| **2026-07-29** | **exp1 RETIRED** — failed its pre-committed criterion |
| 2026-07-29 | Workflow audit — 4 anomalies fixed, unnecessary background work removed |
| 2026-07-30 | Capital-reset fix passed its first live test; Docker cleanup (~8.9 GB reclaimed) |
| 2026-07-31 | Ledger repaired — recovered $69.90 of discarded profit; fixed a BOM parsing bug |

---

## Daily P&L — every trading day

### Era 1 · $100 capital — total **−$0.08**

| Date | Day P&L |
|---|---|
| 2026-05-29 | $0.00 (start) |
| 2026-06-01 | $0.00 |
| 2026-06-02 | −$0.36 |
| 2026-06-03 | +$0.19 |
| 2026-06-04 | +$1.61 |
| 2026-06-05 | −$0.95 |
| 2026-06-08 | −$1.39 |
| 2026-06-09 | −$0.29 |
| 2026-06-10 | +$1.11 |

> ⚠️ **2026-06-11:** portfolio jumped $99.92 → $508.07. That is a **capital
> increase** ($100 → $500 budget), **not** a $408 profit. Excluded from all totals.

### Era 2 · $500 swing mode — total **+$11.08** (21 days, ~0.1%/day)

| Date | Day P&L | | Date | Day P&L |
|---|---|---|---|---|
| 2026-06-12 | −$7.40 | | 2026-07-03 | +$0.24 |
| 2026-06-22 | −$0.51 | | 2026-07-06 | −$0.47 |
| 2026-06-23 | −$3.75 | | 2026-07-07 | −$0.17 |
| 2026-06-24 | +$2.72 | | 2026-07-08 | −$1.23 |
| 2026-06-25 | −$2.82 | | 2026-07-09 | +$1.50 |
| 2026-06-26 | −$1.02 | | 2026-07-10 | +$0.62 |
| 2026-06-29 | +$0.93 | | 2026-07-13 | −$0.89 |
| 2026-06-30 | +$4.58 | | 2026-07-14 | +$1.10 |
| 2026-07-01 | +$1.64 | | 2026-07-15 | **+$7.73** (best) |
| 2026-07-02 | +$4.09 | | 2026-07-16 | −$1.29 |
| | | | 2026-07-17 | −$0.65 |
| | | | 2026-07-20 | −$1.27 |

### Era 3 · $3,000 max-exposure — total **+$198.50** (5 days)

| Date | Day P&L | Cumulative |
|---|---|---|
| 2026-07-21 | −$6.64 | −$6.64 |
| 2026-07-28 | +$19.78 | +$13.14 |
| 2026-07-29 | +$22.96 | +$36.10 |
| 2026-07-30 | +$20.22 | +$56.32 |
| 2026-07-31 | **+$72.28** (best day) | +$128.60 |
| 2026-07-31 | +$69.90 *(carryover recovery)* | **+$198.50** |

> The carryover entry is real money, but it was earned over **several days** by
> stale positions the missed skims left open (e.g. AMZN entered $233.98, exited
> $262.82). It is flagged separately so it is never mistaken for one day's trading.

---

## The honest reading of these numbers

| Era | Return | Per day |
|---|---|---|
| $500 swing (21 days) | +2.2% | **~0.1%/day** |
| $3,000 max-exposure (5 days) | +6.6% | **~1.3%/day** |

**95% of all profit came from 5 days (Jul 28–31).** That window coincided with a
megacap earnings rally while the system sat at 100% exposure. The zero-signal
`always_in_max_exposure` model profited too, and so did the random baseline.

The $500 era's ~0.1%/day is plausibly sustainable. The $3,000 era's ~1.3%/day is
several times what the best quant funds achieve — a clear sign it is **regime
luck, not skill**. A down week at full exposure will subtract just as quickly.

**Total banked across all eras: ~$209.50** — real, honestly accounted for, and
earned mostly by being fully invested during a rally.
