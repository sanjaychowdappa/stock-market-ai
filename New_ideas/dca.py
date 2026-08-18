"""A $13/day long-term accumulation model, and an honest backtest of it.

THE DESIGN

Every trading day, put a fixed $13 into the best one-to-three names for a
LONG hold. Never flatten. This is a different animal from the intraday book:
no daily reset, no stops, no churn — the whole point is time in the market.

Selection runs in two stages, deliberately in this order:

  STAGE 1 — the long-horizon screen (testable, and tested below)
    A name qualifies only if all three hold:
      * 12-month momentum positive   — it is in a real uptrend, not a bounce
      * 6-month momentum positive    — the trend is still current
      * price above its 200-day SMA  — the classic long-term regime filter
    Survivors are ranked by blended 6/12-month momentum, and at most one name
    per sector is taken so a single sector cannot absorb every contribution.

  STAGE 2 — Kronos confirmation (NOT tested below; see the warning)
    Kronos is consulted on the stage-1 picks and may DEMOTE a name, never
    promote one. If it is bearish on the top pick, the next qualifying name is
    taken instead.

WHY KRONOS ONLY GETS A VETO, AND ONLY SECOND

Kronos predicts the NEXT BAR. In daily_stock_picker it emits values like
"TMO: bearish (-0.443%)" — a one-day-ahead move. A one-day predictor has
almost nothing to say about a holding you intend to keep for years, and this
is the same predictor that failed its pre-committed kill criterion at -$0.36
per round trip over 123 real round trips.

Giving it the power to *choose* long-term holdings would be putting a
falsified, wrong-horizon model in charge of the decision that matters most.
Giving it a veto on an already-qualified name is the strongest role the
evidence supports.

Its contribution is therefore NOT baked in as an assumption. The live model
logs both the stage-1 pick and the post-Kronos pick every day, so after a few
months you can measure exactly what the veto was worth — positive, negative or
nothing — instead of believing it.

WHAT THE BACKTEST BELOW DOES AND DOES NOT COVER

It tests stage 1 only, because Kronos cannot be replayed historically without
re-running the model over every past day. Faking that would produce a number
that looks like evidence and is not. Stage 1 is the part that carries the
decision; stage 2 is the part that gets measured going forward.

The benchmark is the only one that matters for a plan like this: the same $13
a day into SPY. If the screen cannot beat that, the screen is costing you
money for the privilege of being complicated.
"""
import json
import os
import sys
import urllib.parse
import urllib.request
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab  # noqa: E402  (shared credential + cache helpers)

DAILY_CONTRIBUTION = 13.0
MAX_NAMES_PER_DAY = 3
CACHE = os.path.join(os.path.dirname(os.path.abspath(__file__)), ".daily_cache")
os.makedirs(CACHE, exist_ok=True)

# Sector map so one sector cannot take every contribution. Same universe the
# production sector agent screens, minus SPY which is the benchmark.
SECTORS = {
    "tech": ["AAPL", "MSFT", "NVDA", "AVGO", "AMD", "CRM", "ORCL", "V", "MA"],
    "comm": ["GOOGL", "META", "NFLX", "DIS", "CMCSA"],
    "discretionary": ["AMZN", "TSLA", "HD", "MCD", "NKE", "LOW"],
    "staples": ["COST", "WMT", "PG", "KO", "PEP"],
    "health": ["LLY", "UNH", "JNJ", "MRK", "ABBV", "TMO"],
    "financials": ["JPM", "BAC", "WFC", "GS"],
    "industrials": ["GE", "CAT", "RTX", "HON", "UNP", "BA"],
    "energy": ["XOM", "CVX", "COP", "SLB"],
    "materials": ["LIN", "SHW", "FCX"],
    "utilities": ["NEE", "SO", "DUK"],
    "realestate": ["AMT", "PLD", "EQIX"],
}
SECTOR_OF = {s: sec for sec, names in SECTORS.items() for s in names}
UNIVERSE = sorted(SECTOR_OF)


def daily_bars(symbol, start="2020-06-01"):
    path = os.path.join(CACHE, f"{symbol}.json")
    if os.path.exists(path):
        with open(path) as fh:
            return json.load(fh)
    q = {"timeframe": "1Day", "start": start, "feed": "iex", "limit": "10000"}
    url = "https://data.alpaca.markets/v2/stocks/%s/bars?%s" % (
        symbol, urllib.parse.urlencode(q))
    req = urllib.request.Request(url, headers=lab._creds())
    with urllib.request.urlopen(req, timeout=60) as r:
        bars = json.loads(r.read().decode()).get("bars") or []
    out = [{"d": b["t"][:10], "c": float(b["c"])} for b in bars]
    with open(path, "w") as fh:
        json.dump(out, fh)
    return out


def build_series():
    series, missing = {}, []
    for sym in UNIVERSE + ["SPY"]:
        try:
            bs = daily_bars(sym)
        except Exception:
            missing.append(sym)
            continue
        if len(bs) < 300:
            missing.append(sym)
            continue
        series[sym] = {b["d"]: b["c"] for b in bs}
    return series, missing


def qualifies(hist, dates, i):
    """Stage 1. Returns the blended momentum score, or None if it fails."""
    if i < 252:
        return None
    px = hist.get(dates[i])
    p6 = hist.get(dates[i - 126])
    p12 = hist.get(dates[i - 252])
    if not px or not p6 or not p12:
        return None
    m6 = (px - p6) / p6
    m12 = (px - p12) / p12
    if m6 <= 0 or m12 <= 0:
        return None
    window = [hist[d] for d in dates[i - 200:i] if d in hist]
    if len(window) < 150:
        return None
    if px <= sum(window) / len(window):   # 200-day SMA regime filter
        return None
    return (m6 + m12) / 2.0


def backtest(start_date="2022-01-03"):
    series, missing = build_series()
    if missing:
        print(f"  (no usable history for: {', '.join(missing)})")
    dates = sorted(series["SPY"])
    idx = [i for i, d in enumerate(dates) if d >= start_date]
    if not idx:
        print("no dates in range")
        return
    first = idx[0]

    model_shares = defaultdict(float)
    spy_shares = 0.0
    invested = 0.0
    picks_count = defaultdict(int)

    for i in range(first, len(dates)):
        d = dates[i]
        scored = []
        for sym in UNIVERSE:
            hist = series.get(sym)
            if not hist or d not in hist:
                continue
            s = qualifies(hist, dates, i)
            if s is not None:
                scored.append((s, sym))
        scored.sort(reverse=True)

        # One name per sector, best first.
        chosen, used = [], set()
        for s, sym in scored:
            sec = SECTOR_OF[sym]
            if sec in used:
                continue
            chosen.append(sym)
            used.add(sec)
            if len(chosen) == MAX_NAMES_PER_DAY:
                break

        invested += DAILY_CONTRIBUTION
        spy_shares += DAILY_CONTRIBUTION / series["SPY"][d]
        if not chosen:
            # Nothing qualifies: park the day's money in SPY rather than
            # sitting in cash. Being out of the market is itself a bet.
            spy_shares += 0.0
            model_shares["SPY"] += DAILY_CONTRIBUTION / series["SPY"][d]
            picks_count["(none - SPY)"] += 1
            continue
        each = DAILY_CONTRIBUTION / len(chosen)
        for sym in chosen:
            model_shares[sym] += each / series[sym][d]
            picks_count[sym] += 1

    last = dates[-1]
    model_val = sum(q * series[s][last] for s, q in model_shares.items() if last in series[s])
    spy_val = spy_shares * series["SPY"][last]

    print(f"\n{'=' * 62}")
    print(f"  $13/DAY, {dates[first]} -> {last}")
    print(f"{'=' * 62}")
    print(f"  trading days           {len(dates) - first}")
    print(f"  total contributed      ${invested:,.2f}")
    print()
    print(f"  {'':22}{'value':>14}{'profit':>12}{'return':>10}")
    print(f"  {'-' * 58}")
    for label, val in (("MODEL (screen)", model_val), ("BENCHMARK  all SPY", spy_val)):
        print(f"  {label:22}{val:>14,.2f}{val - invested:>+12,.2f}"
              f"{(val / invested - 1) * 100:>9.1f}%")
    edge = model_val - spy_val
    print(f"  {'-' * 58}")
    print(f"  {'edge vs just SPY':22}{'':14}{edge:>+12,.2f}")

    print(f"\n  most-picked names")
    for sym, n in sorted(picks_count.items(), key=lambda kv: -kv[1])[:10]:
        print(f"    {sym:<14}{n:>5} days")

    return {"invested": invested, "model": model_val, "spy": spy_val, "edge": edge}


def walk_forward(splits=("2022-01-03", "2023-01-03", "2024-01-02", "2025-01-02")):
    """Same screen, different start dates.

    A single backtest window is one sample. If the edge only exists from one
    starting point it is a property of that start, not of the screen.
    """
    print(f"\n{'=' * 62}")
    print("  WALK-FORWARD: does the edge survive a different start date?")
    print(f"{'=' * 62}")
    print(f"  {'start':<14}{'model':>12}{'SPY':>12}{'edge':>12}")
    print(f"  {'-' * 50}")
    edges = []
    for s in splits:
        r = backtest_quiet(s)
        if r:
            edges.append(r["edge"])
            print(f"  {s:<14}{r['model']:>12,.2f}{r['spy']:>12,.2f}{r['edge']:>+12,.2f}")
    if edges:
        print(f"  {'-' * 50}")
        print(f"  worst start: {min(edges):+,.2f}    best start: {max(edges):+,.2f}")
        if min(edges) <= 0:
            print("\n  The edge does NOT survive every start date. Treat it as absent.")
        else:
            print("\n  Positive from every start tested.")


def backtest_quiet(start_date):
    import io
    import contextlib
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        r = backtest(start_date)
    return r


if __name__ == "__main__":
    backtest()
    walk_forward()
