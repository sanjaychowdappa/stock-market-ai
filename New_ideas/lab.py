"""Research harness for testing trading ideas against real fills.

The rule of this directory: an idea is not evaluated by its own score. It is
evaluated against a null hypothesis, and re-checked with a day withheld. Both
are done here, by the harness, so no idea can skip them.

That rule exists because this project has twice produced a parameter that
looked excellent and was memorising a single day. A flat 0.50% trailing stop
scored +$24.74 overall and -$5.50 with one day removed. The harness that let
that happen was one where validation was a thing you remembered to do.

Two baselines run automatically for every idea:

  NULL       take no action at all. If the idea cannot beat doing nothing, it
             is not an idea.
  RANDOM     take the same NUMBER of actions, at random moments inside the same
             holding windows. This is the sharp one: it isolates whether the
             TRIGGER carries information, or whether the result comes merely
             from having acted more often. An idea that ties random has a
             mechanism that does nothing.
"""
import json
import os
import random
import urllib.parse
import urllib.request
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.path.join(os.path.dirname(os.path.abspath(__file__)), ".bars_cache")
os.makedirs(CACHE, exist_ok=True)

# Fills before this date predate the partial-fill parse fix and carry
# known-bad quantities. The production ledger excludes them; so does this.
RELIABLE_FROM = "2026-08-05"


def _creds():
    env = {}
    with open(os.path.join(ROOT, ".env"), encoding="utf-8", errors="ignore") as fh:
        for line in fh:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    return {
        "APCA-API-KEY-ID": env["APCA_API_KEY_ID"],
        "APCA-API-SECRET-KEY": env["APCA_API_SECRET_KEY"],
    }


def bars(symbol, day):
    """One-minute bars for a symbol on a day, cached to disk after first fetch."""
    path = os.path.join(CACHE, f"{symbol}_{day}.json")
    if os.path.exists(path):
        with open(path) as fh:
            return json.load(fh)
    out, token = [], None
    while True:
        q = {
            "timeframe": "1Min",
            "start": f"{day}T00:00:00Z",
            "end": f"{day}T23:59:00Z",
            "feed": "iex",
            "limit": "10000",
        }
        if token:
            q["page_token"] = token
        url = "https://data.alpaca.markets/v2/stocks/%s/bars?%s" % (
            symbol, urllib.parse.urlencode(q))
        req = urllib.request.Request(url, headers=_creds())
        with urllib.request.urlopen(req, timeout=45) as r:
            j = json.loads(r.read().decode())
        out.extend(j.get("bars") or [])
        token = j.get("next_page_token")
        if not token:
            break
    with open(path, "w") as fh:
        json.dump(out, fh)
    return out


def load_positions(cutoff=RELIABLE_FROM):
    """Real closed positions, FIFO-matched from the production fill log.

    These are trades that actually happened at a real broker: real fill prices,
    real slippage. An idea tested against these is being asked what it would
    have added to something that genuinely occurred.
    """
    fills = []
    log = os.path.join(ROOT, "reports", "broker_fills.jsonl")
    with open(log, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                f = json.loads(line)
            except Exception:
                continue
            if f.get("outcome") != "filled":
                continue
            qty = f.get("qty_filled") or f.get("qty_requested") or 0
            if not qty or not f.get("actual_price"):
                continue
            fills.append({
                "sym": f["symbol"], "side": f["side"], "qty": float(qty),
                "px": float(f["actual_price"]), "ts": f["timestamp"],
            })
    # Chronological: the log is no longer strictly append-in-time-order once
    # abandoned fills are recovered from the broker and stamped with filled_at.
    fills.sort(key=lambda x: x["ts"])

    lots, positions = defaultdict(list), []
    for f in fills:
        if f["side"] == "buy":
            lots[f["sym"]].append(f)
            continue
        rem = f["qty"]
        while rem > 1e-9 and lots[f["sym"]]:
            lot = lots[f["sym"]][0]
            take = min(rem, lot["qty"])
            positions.append({
                "sym": f["sym"], "qty": take,
                "entry_ts": lot["ts"], "entry_px": lot["px"],
                "exit_ts": f["ts"], "exit_px": f["px"],
                "day": lot["ts"][:10],
            })
            lot["qty"] -= take
            rem -= take
            if lot["qty"] <= 1e-9:
                lots[f["sym"]].pop(0)

    return [p for p in positions if p["day"] >= cutoff]


def window(pos):
    """The bars between a position's entry and its exit."""
    try:
        bs = bars(pos["sym"], pos["day"])
    except Exception:
        return []
    return [b for b in bs if pos["entry_ts"] < b["t"] <= pos["exit_ts"]]


def _run(strategy, positions):
    """Apply a strategy to every position; return (total, per-day, actions)."""
    total, actions = 0.0, 0
    by_day, wins = defaultdict(float), 0
    for pos in positions:
        w = window(pos)
        if not w:
            continue
        pnl, acted = strategy(pos, w)
        if not acted:
            continue
        total += pnl
        by_day[pos["day"]] += pnl
        actions += 1
        if pnl > 0:
            wins += 1
    return total, dict(by_day), actions, wins


def _random_baseline(action_rate, seed):
    """Act at a random moment, at the same rate as the idea being tested."""
    rng = random.Random(seed)

    def strat(pos, w):
        if rng.random() > action_rate:
            return 0.0, False
        b = w[rng.randrange(len(w))]
        px = float(b["c"])
        notional = pos["qty"] * pos["entry_px"]
        return (pos["exit_px"] - px) * (notional / px), True

    return strat


def evaluate(name, strategy, positions, trials=40, verbose=True):
    """Score an idea against doing nothing and against acting at random.

    Returns a dict; also prints a verdict unless silenced.
    """
    total, by_day, actions, wins = _run(strategy, positions)
    n = len(positions)
    rate = actions / n if n else 0.0

    # RANDOM baseline, averaged over many seeds so a lucky draw cannot decide
    # the outcome. Same action rate, so only the timing differs.
    rand_totals = []
    for seed in range(trials):
        rt, _, ra, _ = _run(_random_baseline(rate, seed), positions)
        if ra:
            rand_totals.append(rt)
    rand_avg = sum(rand_totals) / len(rand_totals) if rand_totals else 0.0
    rand_beat = sum(1 for t in rand_totals if t >= total)

    # LEAVE-ONE-DAY-OUT. A total carried by one day is a memory, not an edge.
    worst_fold, worst_day = None, None
    for day in sorted(by_day):
        without = total - by_day[day]
        if worst_fold is None or without < worst_fold:
            worst_fold, worst_day = without, day

    result = {
        "name": name, "total": total, "actions": actions,
        "per_action": total / actions if actions else 0.0,
        "win_rate": wins / actions if actions else 0.0,
        "null": 0.0,
        "random_avg": rand_avg,
        "random_beat_count": rand_beat, "random_trials": len(rand_totals),
        "worst_fold": worst_fold, "worst_fold_day": worst_day,
    }
    result["verdict"] = _verdict(result)
    if verbose:
        _report(result)
    return result


def _verdict(r):
    if r["actions"] < 10:
        return "INCONCLUSIVE — fewer than 10 actions, nothing can be concluded"
    if r["total"] <= 0:
        return "REJECTED — loses money outright; it does not clear doing nothing"
    if r["worst_fold"] is not None and r["worst_fold"] <= 0:
        return "REJECTED — the entire edge disappears when one day is withheld"
    if r["random_trials"] and r["random_beat_count"] / r["random_trials"] > 0.10:
        return "REJECTED — random timing matches it; the trigger carries no information"
    return "SURVIVES — beats doing nothing, beats random timing, survives leave-one-out"


def _report(r):
    print(f"\n{'=' * 66}")
    print(f"  {r['name']}")
    print(f"{'=' * 66}")
    print(f"  actions            {r['actions']}")
    print(f"  total P&L          ${r['total']:+.2f}")
    print(f"  per action         ${r['per_action']:+.2f}")
    print(f"  win rate           {r['win_rate'] * 100:.0f}%")
    print(f"  --- baselines it must beat ---")
    print(f"  do nothing         ${r['null']:+.2f}")
    print(f"  random timing      ${r['random_avg']:+.2f}   "
          f"({r['random_beat_count']}/{r['random_trials']} random runs matched or beat it)")
    if r["worst_fold"] is not None:
        print(f"  worst fold         ${r['worst_fold']:+.2f}  (dropping {r['worst_fold_day']})")
    print(f"\n  VERDICT: {r['verdict']}")
