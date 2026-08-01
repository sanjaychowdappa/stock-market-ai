# RESUME HERE

**Read this first after a system reset, a new machine, or a long break.**
Last updated: 2026-07-31 · Version: `claude_1` · Git tag: `claude_1`

---

## 1. What this project is

An automated **paper-trading research system** (no real money). Two strategies run
side by side, plus a set of control experiments whose only job is to tell us
honestly whether any of it actually works.

| | What it does | Status |
|---|---|---|
| **Day trader** | $3,000 fixed capital, fully deployed when the market is risk-on, flat every night, profit banked daily | Running |
| **Momentum ETF portfolio** | Monthly rotation into the top-5 momentum ETFs, benchmarked vs SPY | Running, trailing SPY |
| **Shadow experiments** | Same live data, different rules — including a **random baseline** that everything must beat | Running |
| **agentic_test** | Autonomous supervisor: watches health, integrity, verdicts. Never changes strategy | Running |

**The single most important number: banked P&L = `+$198.50`** (since the $3,000
model began 2026-07-21). See `TIMELINE.md` for the full day-by-day history.

---

## 2. Restart from scratch

```bash
git clone https://github.com/sanjaychowdappa/stock-market-ai.git
cd stock-market-ai
```

**Create `.env`** from the template (it is gitignored — you must recreate it):

```bash
cp .env.example .env          # then fill in your Alpaca PAPER keys
```

**Enable the secret guard** (once per clone — protects a public repo from
credential leaks):

```bash
git config core.hooksPath .githooks
```

Then:

```bash
docker compose up -d          # starts backend, frontend, redis, GPU sidecar
```

First build takes several minutes (Rust compile). After that it is fast.

**Dashboard:** http://localhost:3000
**Backend API:** http://localhost:8000

---

## 3. Daily routine (what you actually do)

There is deliberately very little to do — that is the design.

1. **Keep the machine awake during market hours** (9:30am–4:00pm ET). This is the
   one real requirement. State persists across restarts, so nothing is *lost* if
   it sleeps, but no trading happens while it is off.
2. **Check the dashboard** occasionally, or run the analysis:
   ```powershell
   .\analyze.ps1            # today + last 7 days + the edge test
   ```
   A Windows scheduled task (`StockAI Daily Analysis`) already runs this at
   16:10 daily and archives a dated report to `reports/analysis/`.

---

## 4. Key endpoints

| Endpoint | Shows |
|---|---|
| `/api/health` | version, config-freeze date, symbols |
| `/api/profit` | **daily + weekly banked profit** (the real scoreboard) |
| `/api/daily/status` | live portfolio value, per-symbol tick counts |
| `/api/experiments` | A/B scoreboard + exp1 kill-criterion status |
| `/api/exp1` | exp1 detail (retired) |
| `/api/agentic` | supervisor health + findings (`/api/agentic/run` forces a pass) |
| `/api/momentum` | ETF portfolio vs SPY |
| `/api/scan` | S&P 500 Kronos scan (manual only — auto-scan disabled) |

---

## 5. Where the important data lives

All under `reports/` (mounted into the container, so it survives restarts):

| File | What |
|---|---|
| `daily_profit.jsonl` | **Authoritative** banked P&L per day. This is the scoreboard. |
| `trader_state.json` | Live trader state (positions, cash). Restored on startup. |
| `momentum_state.json` | ETF portfolio state + SPY benchmark |
| `prediction_accuracy.jsonl` | Every trade with all layer scores — the research dataset |
| `agent_log.jsonl` | agentic_test supervisor findings |
| `analysis/` | Dated `analyze.ps1` reports |

---

## 6. Read before changing anything

- **`DECISIONS.md`** — the verdicts already reached and *why*. Several ideas are
  settled; re-litigating them wastes weeks.
- **`TIMELINE.md`** — dated history and per-day P&L.

**The house rule that has served this project best:** decide the success
criterion *before* looking at results, then honour it. Two models have already
been killed by rules written in advance. Do not retune a strategy because it had
a bad week — that is how you fit to noise.

---

## 7. Secret hygiene (this repo is public)

Audited 2026-07-31: **no credential has ever been committed** — `.env` is
untracked, absent from all history, and its values appear in no commit.

Three layers keep it that way:

1. **`.gitignore`** blocks `.env`, `.env.*`, `*.pem`, `*.key`, `secrets.*`,
   `credentials.*`, and raw `*transcript*.jsonl` (transcripts capture file
   contents, including `.env` reads — the sneakiest leak path).
2. **Pre-commit hook** (`.githooks/pre-commit`) inspects *staged content* and
   refuses commits containing live-looking credentials — Alpaca, AWS, OpenAI,
   GitHub tokens, private keys. Verified to block a real test secret.
   Enable with `git config core.hooksPath .githooks`.
3. **`.env.example`** documents required variables without any real values.

**Rules:**
- Use Alpaca **paper** keys only. This project trades on paper; live keys would
  put real money behind a strategy with no demonstrated edge.
- **Redact before committing any transcript.** `docs/session_transcript_redacted.jsonl`
  had 20 credential occurrences stripped and was verified clean first.
- If a key is ever exposed, **rotate it in the Alpaca dashboard immediately** —
  scrubbing git history is unreliable once something is pushed.

## 8. Known gotchas

- **Docker dies when the machine sleeps/shuts down.** A Startup-folder shortcut
  auto-launches Docker Desktop on login; containers use `restart: unless-stopped`.
- **Fixed-capital invariant:** the day trader must start every day at exactly
  $3,000. If a 3:55pm skim is missed, `NEW_DAY` flattens carryover, banks it, and
  hard-resets. Look for `[CAPITAL_RESET]` in the logs.
- **Windows/PowerShell writes UTF-8 BOMs** which break JSONL parsing. If a ledger
  suddenly looks short by one entry, check for a BOM on line 1.
- **`analyze.ps1` trade totals ≠ banked profit.** It sums per-trade P&L, which
  includes multi-day carryover gains. `/api/profit` is authoritative.
