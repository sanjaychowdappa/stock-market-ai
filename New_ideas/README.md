# New_ideas

A sandbox for testing trading ideas against real fills, isolated from the live
system. Nothing here runs in production or places an order.

```
py New_ideas\run.py
```

## The rule of this directory

An idea is never evaluated by its own score. The harness automatically scores
every idea against two baselines and re-checks it with a day withheld, and an
idea has to clear all three to survive:

| Check | Question it answers |
|---|---|
| **do nothing** | Does acting beat not acting? |
| **random timing** | Same number of actions at random moments — does the *trigger* carry information, or does the result just come from acting more? |
| **leave-one-day-out** | Does the edge survive dropping any single day, or is it a memory of one session? |

This is structural rather than a discipline you have to remember, because
remembering did not work. A flat 0.50% trailing stop once scored `+$24.74`
overall and `−$5.50` with one day removed — it had memorised 2026-08-05 and
would have shipped without the fold check.

The random-timing baseline is the sharp one. It has the same exposure as the
idea and differs only in *when* it acts, so it isolates the trigger itself.

## First result: pyramiding into winners

**Proposal:** when a stock shows a profit during the day, put more money into
it; concentrate into whatever is working.

Tested against 112 real closed positions across 10 days (2026-08-05 → 08-18).

```
idea                                          total    per action   win rate
ANTI-PYRAMID  add 1x at -0.50% (average down)  -$4.89      -$0.10       43%
PYRAMID       add 1x at +1.00%                 -$8.44      -$0.56       33%
CONTROL       add 1x at 30 min, price ignored -$14.65      -$0.16       37%
PYRAMID       add 1x at +0.50%                -$36.29      -$0.84       37%
PYRAMID       ladder, 3 adds every +0.50%     -$45.92      -$1.07       28%
PYRAMID       add 1x at +0.25%                -$54.46      -$0.76       31%
PYRAMID       add 2x at +0.50%                -$72.57      -$1.69       37%
```

**Rejected**, and the way it fails is more informative than the fact that it
does.

**The trigger is worse than no trigger.** Pyramiding at +0.50% scored −$36.29,
while acting at *random moments in the same windows* scored −$28.39, and 29 of
40 random runs beat the real rule. Adding at a fixed 30 minutes with price
ignored entirely scored −$14.65, better than every price-triggered variant.
Choosing to add *because a position is winning* performs worse than choosing at
random or not choosing at all.

**The opposite trade is better.** Averaging down at −0.50% lost $4.89 against
pyramiding's $36.29, roughly seven times better, and only 5 of 40 random runs
beat it. Buying weakness beats buying strength here by a wide margin.

Read together, those two facts say the intraday moves in this universe **mean
revert**. Pyramiding depends on trend: it assumes a move already underway tends
to continue. When prices revert instead, the moment a position looks strongest
is the moment it sits nearest a local high, so every add buys the top. Making
the rule more aggressive makes it worse, monotonically — the max-concentration
variant is the worst line in the table.

**Nothing here is profitable in absolute terms**, including averaging down,
because the underlying trades have negative expectancy (−$0.36 per round trip
over 123 real round trips). Adding exposure to a negative-edge process adds
loss. Position sizing changes the size of the number, never its sign.

## Adding an idea

Write a function in `strategies.py`:

```python
def my_idea(threshold):
    def strat(pos, bars):
        # return (marginal P&L of the EXTRA action, whether it acted)
        return 0.0, False
    return strat
```

Return the P&L of the extra action only, not of the position. If an idea has an
edge, the actions it adds must be profitable on their own; modelling where the
capital came from can only obscure that.

Then add one line to `run.py`:

```python
results.append(lab.evaluate("MY IDEA", strategies.my_idea(0.5), positions))
```

The three checks run automatically. There is no way to skip them, which is the
point.

## Files

| File | Contents |
|---|---|
| `lab.py` | Data loading, the baselines, leave-one-out, the verdict logic |
| `strategies.py` | Ideas under test — add new ones here |
| `run.py` | Runs everything and prints a summary |

Positions are FIFO-matched from the production fill log, restricted to
2026-08-05 onward because earlier fills predate the partial-fill parse fix and
carry known-bad quantities. Minute bars come from Alpaca and are cached in
`.bars_cache/` after the first fetch.

---

## Second result: $13/day long-term accumulation

**Proposal:** put a fixed $13 into the best stocks every day and hold for the
long term, with Kronos involved in selection.

`py New_ideas\dca.py`

Tested over 1,160 trading days, 2022-01-03 → 2026-08-18, $15,080 contributed.

```
                             value      profit    return
MODEL (momentum screen)  20,334.62   +5,254.62     34.8%
BENCHMARK  all SPY       22,938.39   +7,858.39     52.1%
                                     edge  -2,603.78
```

Walk-forward, four different start dates:

```
start          model         SPY        edge
2022-01-03 20,334.62   22,938.39   -2,603.78
2023-01-03 13,930.54   16,778.71   -2,848.17
2024-01-02  8,913.07   10,920.65   -2,007.58
2025-01-02  5,467.64    6,251.36     -783.72
```

**The plan works. The stock picking subtracts from it.**

Both halves matter. Committing $13 a day and holding turns $15,080 into
$20,335 even with a mediocre screen, and into $22,938 with no screen at all.
That is a genuinely sound strategy, and it is the first thing tested in this
repository that makes money in absolute terms.

But the selection layer loses to a plain index fund from every start date
tested, by $784 to $2,848. The screen is well-constructed — 6- and 12-month
momentum both positive, price above the 200-day SMA, one name per sector — and
it still costs money relative to owning everything. The most-picked names (LLY,
META, NVDA, CAT, NFLX) were reasonable choices; concentrating into them simply
did worse than not concentrating.

### Where Kronos sits, and why it is not in these numbers

Kronos predicts the NEXT BAR. In `daily_stock_picker` it emits values like
`TMO: bearish (-0.443%)` — one day ahead. A one-day predictor has close to
nothing to say about a position intended to be held for years, and it is the
same model that failed its pre-committed kill criterion at −$0.36 per round
trip over 123 real round trips.

So in the design it gets a **veto, not a vote**: it may demote an
already-qualified name, never promote one. And its contribution is deliberately
absent from the backtest above, because Kronos cannot be replayed historically
without re-running it over every past day — a number produced that way would
look like evidence without being any.

The live model instead logs both the stage-1 pick and the post-veto pick every
day, so what the veto is actually worth becomes measurable after a few months
rather than assumed now.

### Honest recommendation

Do the $13/day. Put it in a broad index. Run the screen in shadow alongside it,
logging what it would have bought, and revisit in six months with real
out-of-sample evidence — including whether the Kronos veto added anything.

That gets you the part that demonstrably works today, and turns the part that
does not into a measurement instead of a bet.

---

## Third result: the five-rule specification

**Proposal:** (1) Kronos picks multiple S&P names across sectors daily;
(2) forecast them and put everything into the one predicted to profit most;
(3) stop loss at $3,000, but keep funding names still in profit even when the
book is under it; (4) log daily picks and treat repeatedly profitable names as
"legacy stocks" to invest in more often; (5) if one stock's profit exceeds all
others combined, move everything into it.

`py New_ideas\rules.py` · `py New_ideas\validate_rules.py` · `py New_ideas\degenerate.py`

Rules 1 and 2 are **not scored**. They need a Kronos forecast, and Kronos cannot
be replayed historically without re-running it over every past bar. A number
produced any other way would look like evidence without being any.

Rules 3, 4 and 5 are pure allocation logic over 142 real positions across 13
sessions, so they replay exactly. All three fail.

### Rule 3 — keep funding winners below the floor

```
                  first version    corrected
floor $3,000           +$125.35      -$18.39
floor $2,990            +$90.79       +$3.10
floor $2,970            +$10.16       +$4.10
```

Conceptually the best idea in the spec: today's damage control halts the entire
book, discarding winners with losers, and this keeps the winners funded.

The first version was **look-ahead biased** — it selected positions whose FINAL
P&L was positive, which at the moment the floor breaks is unknowable. Marking
positions at the breach instant instead reverses the result. A position in
profit right then usually gives it back; the same mean reversion that defeats
pyramiding defeats this.

### Rule 4 — legacy stocks

```
actual P&L of all 142 trades      -$63.69
DEGENERATE: skip every entry      +$63.69   <- the bar to beat
RULE 4 legacy (1 prior win)       +$38.77   -$24.91 vs not trading
RULE 4 legacy (2 prior wins)      +$34.55   -$29.13
RULE 4 legacy (3 prior wins)      +$38.65   -$25.04
```

It skips 95–110 of 142 entries, so in a negative-expectancy system it scores
well for a trivial reason. Compared against skipping *everything*, it is $25–29
**worse**: the "legacy" screen actively selects the wrong survivors.

An earlier fold reported +$69.35 for this. That was also wrong — the per-day
grouping reset each symbol's history every morning, so "legacy" meant nothing.

### Rule 5 — concentrate into the leader

```
checked  30 min   +$79.62   but 74% of it from one day (2026-08-17)
checked  60 min   +$71.56   44% from one day
checked 120 min    -$3.98   sign flips
```

Structurally better than the pyramid tested earlier: it *reallocates* from
losers into the leader rather than adding exposure, which is why it does not
collapse the same way. But a rule whose sign depends on checking at 60 versus
120 minutes is not a rule yet.

### What all five have in common

Every rule is a variation on *select better* — better stocks, better timing,
better concentration, better memory. The shadow board answers that directly, and
the ordering is monotonic in how much selection is applied:

```
random_baseline    +$30.30    0 rules
always_in          +$26.95    1 rule
trend_off           +$1.45
trend_30min         -$0.71
trend_fullday      -$31.83
REAL_TRADER        -$67.03   13 gates
```

Nothing tested in this repository has beaten *not choosing*. The one strategy
that makes money — $13/day into an index — works because it does not choose.

### Two errors caught here, both mine

The look-ahead in Rule 3 and the history reset in Rule 4's fold. Both inflated
results in the flattering direction, and both were caught by checks that exist
because this has happened before. Worth recording: the harness is not protection
against other people's mistakes, it is protection against your own.

---

## Fourth result: the profit lock does not do what it was built to do

**Question asked:** how much of a winning day should the profit lock let you
give back — 0.15% (losses near zero, days end on small dips) or 0.50%
(winners run, bigger losses)?

`py New_ideas\giveback.py`

The floor and the ratchet act on the **intraday equity path**, not on the
closing number, so answering from `daily_profit.jsonl` would answer a
different question. This reconstructs the path minute by minute from 146 real
closed positions and real minute bars across 14 sessions, then replays the
actual `DAMAGE_CONTROL` rule over it.

```
setting                                   total  worst fold  best day  worst day  halts
A  current       trig 0.30 / gb 0.15     -35.04      -42.18      7.15     -11.93     12
B  proposed      trig 0.50 / gb 0.15     -36.61      -51.43     14.82     -11.93     10
C  wide giveback trig 0.30 / gb 0.50     -46.34      -53.28      6.95     -11.93     10
D  lock off                              -51.70      -71.58     19.88     -11.93      9
E  no floor either (raw)                 -76.87      -96.75     19.88     -36.83      0
```

**Answer: 0.15%, which is where it already is.** It has both the best total
and the best leave-one-out fold. But the ranking is the least interesting
thing in the table.

### The worst day is identical in every configuration

`-11.93` appears in all four locked rows. **The profit lock contributes
nothing to loss prevention.** All of it comes from `CAPITAL_FLOOR_PCT`, which
takes the worst day from `-$36.83` to `-$11.93` and the total from `-$76.87`
to `-$51.70`. That is the mechanism that works.

The lock's only measurable effect is on winning days, where it cuts the best
session from `$19.88` to `$7.15`. Its apparent contribution to total P&L comes
from halting 12 of 14 days — and in a negative-expectancy system, halting is
always "better". That is the exposure artifact this directory exists to catch.

### The setting that looked structurally right fails the fold check

B raises the trigger so the lock arms only on a genuinely good day, keeping
`$14.82` of upside instead of `$7.15` for one extra halt. That is the shape
you want. It is also fitted to two sessions: B beats A on 2 days, loses on 3,
and its worst leave-one-out fold is `-$51.43` against A's `-$42.18`. Rejected
on the repo's own rule, and worth stating plainly — B was the answer until the
fold check ran.

### The number that actually decides this

At the current settings, across 14 sessions:

```
winning days   7          average win    $4.80
losing days    7          average loss  -$9.81

win rate needed to break even   67.1%
win rate achieved               50.0%
```

**The payoff is asymmetric in the wrong direction.** The floor caps losses at
about $10 and the lock caps wins at about $7, so the system must be right two
times in three merely to break even, and it is right one time in two. No
setting of the giveback dial changes that; both mechanisms are downstream of
it.

Tuning either one is optimising the brakes on a car with negative horsepower.
The giveback stays at 0.15% because it is the best-supported value, not
because moving it would have mattered.

### Caveats

The replay treats a halt as ending the day. Production allows one resume
through the recovery gate, so real halted days could finish above or below
these numbers — this is a bound, not a prediction. And every session here was
traded by the signal stack that the five rule books replaced on 2026-08-25, so
these are parameters fitted to a strategy that no longer runs.
