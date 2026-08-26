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
## Fourth result: the profit lock was armed far too early

**Question asked:** how much of a winning day should the profit lock let you
give back — 0.15% or 0.50%?

`py New_ideas\giveback.py`

The floor and the ratchet act on the **intraday equity path**, not on the
closing number, so answering from `daily_profit.jsonl` would answer a
different question. This reconstructs the path minute by minute from 146 real
closed positions across 14 sessions and replays the actual `DAMAGE_CONTROL`
rule over it.

**The answer turned out not to be about the giveback at all.**

```
setting                                total  worst fold  best day  worst day  halts
A  current       trig 0.30 / gb 0.15  -35.84      -42.79      6.95     -11.93     12
B  chosen        trig 0.50 / gb 0.15  -11.74      -26.56     14.82     -11.93     10
C  wide giveback trig 0.30 / gb 0.50  -40.16      -47.10      6.95     -11.93     10
D  lock off                           -36.46      -56.34     19.88     -11.93      8
E  no floor either (raw)              -73.25      -93.13     19.88     -36.83      0
```

At the 0.30% trigger (+$9 on a $3,000 book) the lock armed on almost any
decent morning, and a $4.50 wiggle then halted the session. Best day +$6.95
against a worst day of −$11.93: **capped upside against uncapped-by-comparison
downside.**

```
setting              win/loss   avg win   avg loss   break-even   achieved
trig 0.30 / gb 0.15     7 / 7     $4.71     -$9.83        67.6%      50.0%
trig 0.50 / gb 0.15     7 / 7     $8.16     -$9.83        54.7%      50.0%
lock off                6 / 8     $6.95     -$9.77        58.4%      42.9%
```

Raising the trigger does not touch the losses — average loss is −$9.83 either
way — it stops truncating the wins. That alone moves break-even from needing
two rights in three to needing a shade better than a coin flip.

### Plateau, not a spike

A single standout cell is usually overfitting. This one is not:

```
trigger   0.30    0.35    0.40    0.45    0.50    0.55    0.60    0.65    0.70
total   -35.84  -24.31  -27.07  -11.74  -11.74  -12.98  -12.01  -42.09  -43.06
```

0.45 through 0.60 are one flat region and 0.50 sits inside it; outside, the
result degrades sharply in **both** directions. That two-sided degradation is
the degenerate-case check passing — a monotonic "always tighter is better"
would have meant the replay was measuring exposure. And 0.50% beats 0.30% on
**all 14 of 14 leave-one-out folds**, worst fold −$26.56 against −$42.79.

### The floor is still the thing that prevents losses

`-11.93` is the worst day in every locked row. `CAPITAL_FLOOR_PCT` does all
the loss prevention, taking the worst day from −$36.83 to −$11.93. The profit
lock has never contributed to that and still does not. What it buys, at the
right trigger, is keeping winning days alive.

### The bug that produced the opposite answer first

The first version of this replay keyed positions by their entry day, so the
eight positions held overnight were invisible on every later session — their
marks could not move the equity path, and the floor could neither fire nor be
avoided because of them.

Cross-checking against the broker's own `by_day` figures is what exposed it:
the days that disagreed were exactly the days with carry. Five of fourteen.

With that fixed, **the conclusion inverted.** 0.30/0.15 had scored best and
0.50/0.15 had failed the fold check; corrected, 0.50/0.15 wins on every fold.
The committed recommendation was already wrong when it was written.

A carried position's basis is now the day's opening mark rather than its entry
price — using the entry price folds prior sessions' P&L into today's path, and
the floor is a rule about today.

### Caveats

The replay treats a halt as ending the day; production allows one resume
through the recovery gate, so halted days are a bound rather than a
prediction. And all 14 sessions were traded by the signal stack that the five
rule books replaced on 2026-08-25, so these are parameters fitted to a
strategy that no longer runs — the plateau is the reason to expect them to
transfer, not a guarantee that they will.

---

## Fifth result: the low win rate is manufactured, and it is not the problem

**Question asked:** why is the win rate so low — 19% on 2026-08-26, 33% across
the window?

`py New_ideas\winrate.py`

159 real closed positions, 2026-08-05 → 08-26, scored by the reason each was
closed.

```
exit reason               n  wins   win%     total   avg win  avg loss
TRAIL_STOP               74    17    23%   -114.42      1.35     -2.41
HARD_STOP                 8     0     0%    -32.62      0.00     -4.08
RECONCILE                 5     0     0%    -24.90      0.00     -4.98
DAMAGE_CONTROL           15     7    47%      3.88      2.43     -1.64
REGIME_EXIT              16     6    38%      4.02      2.63     -1.17
EOD_DAILY_SKIM           41    23    56%     83.71      4.80     -1.49
ALL                     159    53    33%
```

**47% of all exits are stop-outs, and a stop-out is a loss by construction.**
The trail stop fires precisely on positions that turned down, so 23% is the
mechanism working, not failing. Positions that survive to the closing skim win
56% of the time.

### Every exit is worth money except one

The same positions, held to the close instead of being exited:

```
exit reason               n    ACTUAL  HELD to close   the exit was worth
DAMAGE_CONTROL           15      3.88         -31.39               +35.27
TRAIL_STOP               74   -114.42        -145.69               +31.27
REGIME_EXIT              16      4.02         -20.16               +24.18
EOD_DAILY_SKIM           41     83.71          63.44               +20.26
HARD_STOP                 8    -32.62         -41.23                +8.61
RECONCILE                 5    -24.90         -14.18               -10.72
```

Raising the win rate is trivial — remove the stops — and it costs about $109.
**Win rate is the wrong number to optimise here.** Expectancy is the number,
and the stops improve it while lowering the win rate.

`RECONCILE` is the exception and deserves attention: five positions, zero wins,
2.4-minute median hold, −$24.90 where holding would have lost $14.18. That is
bookkeeping churn losing real money, not a trading decision.

### Where the losses actually come from

```
                                     total    win%
signalled entries + full exit ladder  -80.33   33.3%
same names, same days, bought at the
  open and held to the close         -431.41   38.4%
```

The signalled entries beat buying the same names at the open on **10 of 15
days**, by $351 in total. Note this compares entry *and* exit machinery against
buy-and-hold-the-session, so it is a verdict on the system as a whole, not on
timing alone.

The reason both lose is upstream of all of it:

```
cumulative intraday move, 15 sessions
  traded universe   -3.65%
  SPY               -2.58%
  selection cost    -1.07%
```

**The tape fell and the book is long-only.** The universe dropped 3.65% across
these sessions, underperforming SPY by 1.07%. A long-only intraday system in a
falling market loses; the machinery clawed back $351 of a $431 hole and still
finished down $80.

So the honest decomposition of the loss is: market direction first, stock
selection second, and the exit ladder actually working against both.

### One table in this file is biased — deliberately left in

`winrate.py` also re-runs alternative trail widths over the positions the
0.75% trail closed. That subset is *selected by the rule being varied*: every
position in it hit a 0.75% drawdown from peak. Testing 0.50% on that set asks
whether a tighter stop would have done better on trades already known to have
fallen far enough to trigger the looser one, which it will, for free. The table
prints because deleting it invites someone to reconstruct it later; it should
not be read as evidence about trail width. The unbiased version of that
question is in `lab.py`, over all positions, with the fold check.
