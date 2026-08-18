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
