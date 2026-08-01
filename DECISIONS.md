# Decisions & Verdicts

**Settled questions. Read this before changing strategy — several of these cost
weeks to establish, and re-litigating them wastes that work.**

Last updated: 2026-07-31

---

## The house rules

1. **Pre-commit the success criterion before looking at results.** Write down what
   would make an idea a failure, *then* run it. Two models have been killed by
   rules written in advance — that only works because the rule existed first.
2. **A small sample is not evidence.** 10–30 trades tells you nothing. Signals that
   looked brilliant at 12 trades reverted to noise by 25. Wait for hundreds.
3. **Everything must beat the random baseline.** A strategy that cannot beat
   coin-flip entries with the same exits has no demonstrated skill, no matter how
   sophisticated it looks.
4. **Never scale a negative edge.** More trades or bigger size on a losing system
   multiplies losses. Earn the right to scale by proving positive expectancy first.
5. **Model costs honestly.** Shadow trades charge a 0.04% round-trip cost. Without
   it, fast strategies look profitable when they are not — exp1's expectancy
   flipped from apparently positive to negative the moment costs applied.

---

## VERDICT 1 — The 7-layer signal stack has no edge
**Decided 2026-06-26 · one week of live data**

The random-entry baseline was the **best performer of the week**. Every
signal-weighted model underperformed it, and the more filtering a model applied,
the worse it did — a clean monotonic relationship:

```
random (no filters)   best
trend_30min           ↓
trend_off             ↓
trend_fullday         ↓
REAL trader (7 gates) worst
```

**Conclusion:** the filters were not adding skill — they were removing profitable
exposure. Stop refining per-second signals. This is settled.

---

## VERDICT 2 — exp1 (short-horizon prediction) is dead
**Criterion set 2026-07-21 · verdict delivered 2026-07-29**

Pre-committed rule, written before any results existed:

> By 14 days or 200 closed trades, exp1's expectancy after costs must be **positive
> AND beat the random baseline** — otherwise it is retired, not retuned.

**Result at 327 trades:** −$0.256/trade, total **−$83.18**, versus the random
baseline's +$0.65/trade. Failed both conditions decisively.

**Action taken:** retired (`EXP1_RETIRED = true`). It stops opening positions;
history preserved and the failure is displayed, not hidden.

---

## VERDICT 3 — Intraday scalping is impossible at small capital
**Decided 2026-06-22 · measured, not assumed**

At $500 with high trade frequency, **fees consumed 137% of gross profit** — the
strategy lost money even when the trades won. Compounding factors:

- Spread + slippage on every round trip
- The US **PDT rule**: under $25,000 you get 3 day-trades per 5 days

**Conclusion:** frequency is the enemy at small size. This drove the move to swing
mode, then to monthly ETF rotation.

---

## VERDICT 4 — Exposure is doing the work, not the signals
**Ongoing observation, strongest evidence 2026-07-28 → 07-31**

`always_in_max_exposure` — a model with **zero signals** that simply stays fully
invested — performs comparably to the "smart" trader. During the Jul 28–31 rally
both profited heavily, as did the random baseline.

**Conclusion:** when the real trader "beats random", check whether it was simply
more invested during an up move. Compare **% return on capital**, not $/trade,
across models with different position sizes.

---

## VERDICT 5 — The momentum ETF portfolio is trailing its benchmark
**Open, monitoring since 2026-07-20**

Latest: portfolio **−2.39%** vs SPY **−0.79%** → **−1.60% edge**. The concentrated
top-5 basket is higher-beta than the index: it falls harder in drawdowns. Its
absolute-momentum cash switch (rotate to BIL when momentum turns negative) has not
yet triggered.

**Not yet settled** — needs more time. But "just buy the index" is currently
winning.

---

## Decisions deliberately NOT taken

| Proposal | Why refused |
|---|---|
| **Increase trades/size to "maximize profits"** | Expectancy was negative. Scaling a losing edge multiplies losses — 5× trades on −$0.23/trade would have turned a −$14 week into ≈−$140. |
| **Let an AI agent modify strategy autonomously** | Would destroy measurement integrity. `agentic_test` observes and recommends but **cannot** alter weights, thresholds, sizing or entry/exit rules. Self-tuning produces automated overfitting. |
| **Integrate a self-replicating agent framework** | No functional overlap with trading; self-modifying code makes reproducible results impossible. |
| **Chase 10%/day** | Arithmetically impossible. $500 at 10%/day = $13 trillion in a year. The best fund ever recorded (Medallion) averages ~0.2%/day. Anyone advertising otherwise is running a scam. |

---

## Bugs found that changed reported results

These matter because each one made the system look **different from reality**:

| Date | Bug | Impact |
|---|---|---|
| 2026-07-28 | Ledger recorded the running total as each day's profit | Fake **$60.47** "total profit"; real figure was ≈break-even |
| 2026-07-28 | UI rendered losses without a minus sign | −$6.64 displayed as "$6.64" |
| 2026-07-29 | Daily capital reset was lost on shutdown (async write) | Profit silently **compounded** into next-day capital |
| 2026-07-29 | EOD ledger read a portfolio already zeroed by the 3:55pm skim | Recorded **$0.00 every day** |
| 2026-07-31 | Carryover profit discarded on capital reset | **$69.90** of real profit deleted |
| 2026-07-31 | UTF-8 BOM broke JSONL line 1 | An entire day silently missing from totals |

**Lesson:** the accounting is as likely to be wrong as the strategy. Verify that
totals reconcile before believing any performance claim — including a flattering one.

---

## What would actually change the picture

- A **positive-expectancy result over hundreds of trades** that beats the random
  baseline after costs.
- The momentum portfolio **beating SPY** over a meaningful period including a
  drawdown (so the cash switch is tested).
- Until then, the honest conclusion stands: **no demonstrated edge**, and the
  project's real value is as a rigorous engineering and experimental-design
  exercise — a system that refuses to fool its owner.
