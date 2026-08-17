# stock-market-ai

An algorithmic trading research system in Rust: a live market-data pipeline, a
signal engine, and a paper-broker integration — built to test whether a
multi-layer signal stack could predict intraday price direction.

**It could not, and this repository is the record of establishing that.**

The system trades a $3,000/day book against Alpaca's paper API with real
execution mechanics: real fills, real slippage, real partial fills, real
rejections. It ran a pre-committed kill criterion and a set of shadow models
including a random-entry baseline. The random baseline beat the signal stack.

```
model                     realized  trades  source
always_in_max_exposure     +$56.04    189   simulated
random_baseline            +$30.26    135   simulated      <-- the bar
trend_off                   +$9.31    118   simulated
trend_30min                 +$1.37    109   simulated
trend_fullday              -$30.64    103   simulated      <-- production rule
REAL_TRADER                -$32.45     87   alpaca /v2/account
```

All six models run on identical prices with identical accounting. The ordering
is monotonic: **the more the signals are allowed to filter, the worse the
result.** Over the same window, buy-and-hold on QQQ returned +$127.91.

That is the finding. The rest of this document is how it was built and how it
was made trustworthy enough to believe.

---

## Why a negative result is the deliverable

Any trading system can be made to look profitable by measuring it carelessly.
This one was wrong in the flattering direction three separate times, and each
error was caught only because something independent disagreed:

| Reported | Reality | Cause |
|---|---|---|
| −$12.29 | +$22.52 | FIFO reconstruction inherited our own logging bug |
| +$22.52 | −$27.29 | P&L read stale daily bars instead of the live account |
| +$185.71 | −$32.45 | Scoreboard row labelled "REAL" was reading simulator P&L |

The last one persisted for weeks in the most prominent number in the UI. The
fix was to take P&L from the broker's own equity curve and refuse to fall back
to the simulator silently — an unreachable broker now marks the row unverified
rather than quietly substituting a friendlier number.

**Design rule that came out of this:** a number rebuilt from our own records
can inherit our own bugs; the broker's equity curve cannot. Anywhere the two
disagree, the broker wins and the divergence is displayed rather than resolved.

---

## Architecture

```
                        resolved once at boot
  sector agent  ──────►  state file  ──────►  TOP_SYMBOLS (11 tickers)
  54 names                                          │
  11 GICS sectors                                   ▼
                                          ┌──────────────────┐
  Alpaca IEX websocket  ───── ticks ─────►│  11 engines      │
  + REST historical bars                  │  Kronos · VP     │
                                          │  Kalman · CVD    │
                                          └────────┬─────────┘
                                                   │ score
                                                   ▼
                                          ┌──────────────────┐
                                          │   entry gate     │
                                          │   score >= 0.05  │
                                          └────────┬─────────┘
                                                   │ order
                                                   ▼
                                          ┌──────────────────┐
       guards ───────────────────────────►│  Alpaca broker   │
       regime filter · exit ladder        │  paper account   │
       damage control · reconcile         └──────────────────┘
```

**Backend** — Rust, Axum, Tokio. Per-symbol engines run concurrently against a
Python GPU sidecar; a websocket dispatcher fans ticks out to them; several
independent background loops handle regime detection, sector ranking, and
broker reconciliation on separate cadences.

**Frontend** — React, with candles aggregated client-side from trade ticks.

**Infrastructure** — Docker Compose, scheduled start/stop around market hours.

Full mechanism, timings, and guard behaviour: [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Concurrency problems worth reading

The interesting engineering is in the places where a simulator and a real
broker disagree under concurrency.

**Reconcile vs. in-flight orders.** A loop compares the simulator's book to the
broker's and corrects drift. During an end-of-day liquidation one sell was
*partially filled* when the loop sampled positions; it read the unfilled
remainder as drift and issued a duplicate. Alpaca rejected it only because
those shares were already committed to the open order — accepted, it would have
taken the account short.

The subtle part is that a per-symbol lock already existed and did not help. It
serialised *execution*, not *decisions*: the duplicate waited politely for the
real fill to finish and then submitted anyway, because the delta had already
been computed from the pre-wait snapshot. The guard had to move to where the
delta is calculated, not where the order is sent.

**Frozen parameters.** Trailing-stop width was `(1.5 × entry_atr).clamp(0.5, 3.0)`
where `entry_atr` was a one-minute reading captured at entry and never updated.
That made stop width a function of *what time a position opened* — a sixfold
range driven by intraday noise. One symbol, one session:

```
10:18 entry  ->  stopped at -1.69% from peak
13:53 entry  ->  stopped at -0.50% from peak
```

**Write races.** State saves were async and fire-and-forget, so a stale write
could land last and corrupt the ledger; the first fix — a sequence guard —
checked before the await and did not work. Saves are now synchronous at
critical points. The profit ledger separately banked the same dollars three
times because its running total read the last row rather than summing.

---

## Testing

```
cargo test          # 26 regression tests
```

The rule for the regression suite is that **each test must fail against the old
behaviour** — a test that passes either way documents nothing. Every test
encodes a defect that actually shipped, with the conditions that produced it.

Some assert invariants that are easy to regress by "optimising":

- A trailing stop width chosen for leave-one-out robustness, not for the best
  backtest score. The naive winner scored `+$24.74` and collapsed to `−$5.50`
  when a single day was withheld; the shipped value was positive across all
  seven folds. A test asserts the robust value, with that reasoning in the
  failure message.
- Ledger observation rows must not join the banked total, because the sum
  originally counted any row carrying a `day_pnl` field.

The test harness itself was a bug: it ran `cargo test | tail -40`, making the
exit code `tail`'s, which is always `0`. It printed "All tests passed" for
builds that did not compile.

---

## Validation methodology

Parameter choices are replayed against real fills, with two checks that exist
to catch self-deception:

**Degenerate-case check.** In a negative-expectancy system, any rule that cuts
exposure looks better, so a monotonic "tighter is always better" result would
mean the replay measures exposure rather than the parameter. Widths below the
optimum score progressively *worse*, confirming an interior optimum.

**Leave-one-day-out.** The highest-scoring parameter lost its entire edge when
one day was withheld — it had memorised that day. The parameter that was
positive across every fold was chosen instead, and it was not the winner.

```
                     08-05   08-06   08-07   08-10   08-11   08-12   08-13   WORST
fixed trail 0.50%     -5.5   +19.2   +36.0   +24.7   +24.7   +39.1   +10.1    -5.5
fixed trail 0.75%     +7.0   +14.2   +12.9   +13.6   +13.6   +16.6    +3.8    +3.8
```

---

## Running it

Requires Docker and Alpaca **paper** API keys in `.env` (gitignored, never
committed).

```bash
docker compose up -d --build
```

Backend `:8000`, frontend `:3000`. Use `127.0.0.1`, not `localhost` — the
latter resolves to IPv6 `::1` on the development machine, where a stale relay
swallows port 8000.

`creds()` refuses any endpoint whose URL does not contain `paper-api`, so the
broker module cannot be pointed at live trading by configuration alone. This is
research code with no demonstrated edge; that rail is deliberate.

---

## Documentation

| File | Contents |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Full mechanism, timings, guard behaviour |
| [`DECISIONS.md`](DECISIONS.md) | Settled verdicts and the reasoning behind them |
| [`TIMELINE.md`](TIMELINE.md) | Dated history and per-day P&L across all eras |
| [`RESUME_HERE.md`](RESUME_HERE.md) | Restart procedure and known gotchas |

---

## Status

Paper trading only. No real capital, ever.

The signal stack is retired: it has been falsified twice, on independent data,
and a random baseline beats it. The pre-committed kill criterion is being run
to completion rather than abandoned at the point the answer became clear.
