# Architecture

How a price becomes an order, what runs when, and where each guard sits.

Constants cited here are read from `backend-rust/src/config.rs`; timings from
`backend-rust/src/state.rs`. Where this document and the code disagree, the code
is right — check before trusting a number.

---

## 1. Universe selection — resolved once, at boot

A sector agent ranks 54 names across the 11 GICS sectors by blended 1/3/6-month
momentum (21/63/126-day windows) and writes its picks to disk. At startup
`TOP_SYMBOLS` reads that file and is then **frozen for the process lifetime**.

```
sector agent ──writes──► sector_leaders_state.json ──read at boot──► TOP_SYMBOLS
   54 names                                                          11 tickers
   11 sectors
```

The agent rescans about two minutes after boot, so **today trades yesterday's
picks**. That is deliberate, not an oversight:

> Each symbol spawns an engine with its own model and agent loops, and the
> Alpaca websocket subscribes once when it connects. Rotating the universe
> mid-session means tearing down engines and resubscribing the socket — a
> lifecycle refactor, not a config change.

Since momentum is measured over 21–126 days, one day of staleness is immaterial.
The machine restarts daily, so rotation happens for free.

`TOP_SYMBOLS` is unioned with any currently-held position. Without that, a
universe change orphans open positions: their prices stop updating, every
price-based exit goes dead, and the capital floor is left blind. That happened
once, in a mid-session deploy.

---

## 2. The live loop

```
Alpaca IEX websocket ──ticks──► 11 engines ──score──► entry gate ──order──► broker
+ REST historical bars          Kronos · VP           >= 0.05              paper acct
                                Kalman · Pattern      max 11 open          $3,000/day
                                CVD                   20 min cooldown
```

Sizing is `per_slot = cash / qualified.len()`, so the full budget deploys across
whatever qualifies. Capital not deployed is a symptom of the entry floor, not of
the sizing.

**Historical bars matter more than they look.** Alpaca truncates from the
*start* of the window, so requesting a bar limit without a `start` date returns
only the current session — which starved the volume-profile layer and prevented
any trading until roughly 10:01 each morning. The daily-bar fetch had the same
bug in a worse form: it fed the regime filter a five-week-old price.

---

## 3. Guards

Each can stop the path above independently.

| Guard | Cadence | Behaviour |
|---|---|---|
| Regime filter | hourly | QQQ vs its 50-day SMA; risk-off flattens |
| Exit ladder | per tick | hard stop (ATR-scaled), trailing stop (fixed 0.75%), flat-exit backstop |
| Damage control | per tick | floor at −1% of book; halts **real orders only** |
| Reconcile | 120 s | corrects book drift; defers symbols with orders in flight |

### Damage control halts the broker, not the simulator

Below the floor the book is flattened and real orders stop, but the simulator
keeps trading. Simulated trades cost nothing and are the only way to learn
whether the model has recovered. Each recovery trade is charged a modelled
round-trip cost first, so the gate measures the model rather than the
simulator's optimism. Once the day peaks above +1%, the floor ratchets up behind
it so a winning day cannot become a losing one.

This bounds a loss; it cannot eliminate one. A stop fills below its trigger and
every exit pays a round trip.

### The trailing stop is deliberately not volatility-scaled

It was `(1.5 × entry_atr).clamp(0.5, 3.0)`, where `entry_atr` was a one-minute
reading captured at entry and frozen. Stop width therefore depended on *what
time the position opened* — a sixfold range set by intraday noise rather than by
risk. One symbol, one session:

```
10:18 entry  ->  TRAIL_STOP at -1.69% from peak
13:53 entry  ->  TRAIL_STOP at -0.50% from peak
```

Both failure modes followed. Morning entries could not be stopped at all (one
position sat 118 minutes past its threshold and rode to the close); afternoon
entries were cut on ordinary noise and immediately re-entered.

`trail_stop_pct()` now takes the entry ATR and deliberately ignores it, so the
call site reads as a decision rather than an omission.

---

## 4. End of day

```
15:55  flatten everything  ──►  ledger  ──►  reset to $3,000  ──►  16:10 compose down
```

Capital resets daily; profit does not compound. The invariant is re-asserted on
the next new-day tick in case the skim never ran, because a missed skim would
silently carry yesterday's positions and profit into today's position sizing.

### The ledger records two numbers, not one

The banked figure is `cash − INITIAL_CASH` from the **simulator**, which books a
trade whether or not the broker filled it. Decomposing every day showed the
divergence is entirely explained by non-fills:

```
day         ledger    broker   non-filled
2026-08-13  -$7.34    -$6.83       0        agrees, slippage only
2026-08-14  +$0.17    +$1.19       0        agrees, slippage only
2026-08-10 -$10.17    +$1.47      16        diverges by $11.64
2026-08-11 +$13.56     $0.00       6        diverges by $13.56, zero round trips
```

Each skim row now carries `unfilled_today`, so a row states its own reliability,
and a separate `broker` row records Alpaca's own figure.

Adding a row type was the dangerous part: `ledger_cumulative()` summed *every*
row carrying a `day_pnl` field, the same shape as an earlier bug that banked the
same dollars three times. The sum now filters on row kind, and non-banking rows
are not given a `day_pnl` field at all.

---

## 5. What runs when

| Clock | Component | Cadence |
|---|---|---|
| 09:25 | `auto_start.bat` | weekdays |
| boot +3 s | websocket subscribe | once, never resubscribes |
| boot +20 s | reconcile loop | 120 s, skipped when both books are empty |
| boot +120 s | sector agent | 24 h — writes picks for the *next* boot |
| continuous | regime filter | 1 h |
| continuous | model re-ranking | 2 h |
| 15:55 | EOD skim | daily, no weekday special case |
| 16:10 | `auto_stop.bat` | weekdays |

The 16:10 stop removes the containers, and **backend stdout goes with them**.
`reports/*.jsonl` survives; diagnostic logs do not. Post-mortems must be built
from the report files.

---

## 6. Measurement

The broker is the scoreboard. The simulator is the decision engine.

| Source | Trust | Why |
|---|---|---|
| Alpaca `/v2/account` | authoritative | the broker's own equity curve |
| FIFO over our fill log | diagnostic | inherits our recording bugs |
| Simulator P&L | decision engine only | books trades that never filled |

`portfolio/history` day buckets are settlement-lagged, so per-day comparisons
between simulator and broker are misleading; only cumulative comparisons are
fair.

Three separate P&L reports were wrong in the flattering direction before this
separation was enforced — twice with the wrong **sign**, and once by $218 in a
scoreboard row labelled "REAL" that was reading simulator P&L. The current rule
is that an unreachable broker marks a figure unverified rather than silently
substituting the simulator's.

---

## 7. Experiment design

Six models run in parallel on identical prices with identical accounting,
differing only in their entry rule. Two are null hypotheses:

- `random_baseline` — coin-flip entries. Any model that cannot beat this has no
  demonstrated skill.
- `always_in_max_exposure` — always fully invested, no gates. Tests whether
  time-in-market, rather than selection, is the real driver.

Shadow trades are charged a modelled round-trip cost at exit, so the comparison
is not flattered by free execution.

The result is in [`README.md`](README.md): the ordering is monotonic in how much
the signals are allowed to filter, and both null hypotheses beat every signal
variant.

---

## 8. Development environment notes

- Tests run in Docker (`rust:1.95-bookworm`); the development machine has Rust
  but no MSVC linker, so local `cargo test` fails at link time.
- Use `127.0.0.1`, never `localhost` — the latter resolves to IPv6 `::1`, where
  a stale relay swallows port 8000. This cost hours of misdiagnosis.
- `frontend/src` is bind-mounted, but the dev server's hot-reload socket does
  not survive the Docker-on-Windows file-watching path. Frontend edits need
  `docker compose restart frontend`.
- `scripts/auto_start.bat` must stay pure ASCII with CRLF endings and must not
  use `timeout`, which requires a console that Task Scheduler does not provide.
  Each of those three constraints was learned by the script failing silently.
