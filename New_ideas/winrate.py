r"""Why is the win rate low?

Win rate is a property of the EXIT rule at least as much as the entry. A tight
trailing stop manufactures many small losses; a forced flatten closes whatever
is open at the worst moment of the day by construction. This separates the two:
it scores every real position by the reason it was closed, and then asks what
the same entries would have returned under one alternative exit — hold to the
close — which uses no information the trader did not have at entry.

  py New_ideas\winrate.py
"""
import sys, os, json
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab

ROOT = lab.ROOT


def positions_with_reasons(cutoff=lab.RELIABLE_FROM):
    """FIFO-matched positions carrying the exit reason of the sell that closed
    them — load_positions() drops it, and it is the whole question here."""
    fills = []
    with open(os.path.join(ROOT, "reports", "broker_fills.jsonl"), encoding="utf-8") as fh:
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
            fills.append({"sym": f["symbol"], "side": f["side"], "qty": float(qty),
                          "px": float(f["actual_price"]), "ts": f["timestamp"],
                          "reason": f.get("reason", "")})
    fills.sort(key=lambda x: x["ts"])

    lots, out = defaultdict(list), []
    for f in fills:
        if f["side"] == "buy":
            lots[f["sym"]].append(f)
            continue
        rem = f["qty"]
        while rem > 1e-9 and lots[f["sym"]]:
            lot = lots[f["sym"]][0]
            take = min(rem, lot["qty"])
            out.append({"sym": f["sym"], "qty": take,
                        "entry_ts": lot["ts"], "entry_px": lot["px"],
                        "exit_ts": f["ts"], "exit_px": f["px"],
                        "reason": f["reason"], "day": lot["ts"][:10]})
            lot["qty"] -= take
            rem -= take
            if lot["qty"] <= 1e-9:
                lots[f["sym"]].pop(0)
    return [p for p in out if p["day"] >= cutoff]


def family(reason):
    for k in ("DAMAGE_CONTROL", "EOD_DAILY_SKIM", "TRAIL_STOP", "REGIME_EXIT",
              "HARD_STOP", "TAKE_PROFIT", "BEARISH", "FLAT_EXIT", "RECONCILE"):
        if reason.startswith(k):
            return k
    return reason.split("(")[0] or "UNKNOWN"


def close_price(sym, day):
    """Last regular-session bar of the day."""
    bs = [b for b in lab.bars(sym, day) if "13:30" <= b["t"][11:16] <= "20:00"]
    return bs[-1]["c"] if bs else None


def main():
    pos = positions_with_reasons()
    print(f"{len(pos)} real closed positions, {pos[0]['day']} -> {pos[-1]['day']}\n")

    print("=== WIN RATE BY EXIT REASON ===")
    print(f"{'exit reason':22} {'n':>4} {'wins':>5} {'win%':>6} {'total':>9} "
          f"{'avg':>8} {'avg win':>8} {'avg loss':>9}")
    groups = defaultdict(list)
    for p in pos:
        groups[family(p["reason"])].append(p["qty"] * (p["exit_px"] - p["entry_px"]))
    rows = sorted(groups.items(), key=lambda kv: sum(kv[1]))
    tot_n = tot_w = 0
    for name, pnls in rows:
        w = [x for x in pnls if x > 0]
        l = [x for x in pnls if x <= 0]
        tot_n += len(pnls); tot_w += len(w)
        print(f"{name:22} {len(pnls):>4} {len(w):>5} {100*len(w)/len(pnls):>5.0f}% "
              f"{sum(pnls):>9.2f} {sum(pnls)/len(pnls):>8.2f} "
              f"{(sum(w)/len(w) if w else 0):>8.2f} {(sum(l)/len(l) if l else 0):>9.2f}")
    print(f"{'ALL':22} {tot_n:>4} {tot_w:>5} {100*tot_w/tot_n:>5.0f}%")

    print("\n=== SAME ENTRIES, ONE ALTERNATIVE EXIT: hold to the close ===")
    print("(uses no information the trader lacked at entry — it is a policy, "
          "not a prediction)\n")
    act_p, hold_p, skipped = [], [], 0
    for p in pos:
        c = close_price(p["sym"], p["day"])
        if c is None:
            skipped += 1
            continue
        act_p.append(p["qty"] * (p["exit_px"] - p["entry_px"]))
        hold_p.append(p["qty"] * (c - p["entry_px"]))

    def line(tag, pnls):
        w = [x for x in pnls if x > 0]
        l = [x for x in pnls if x <= 0]
        print(f"{tag:24} {len(pnls):>4} trades  {100*len(w)/len(pnls):>5.1f}% win  "
              f"total {sum(pnls):>8.2f}  avg win {(sum(w)/len(w) if w else 0):>6.2f}  "
              f"avg loss {(sum(l)/len(l) if l else 0):>6.2f}")
    line("ACTUAL exits", act_p)
    line("HOLD to close", hold_p)
    if skipped:
        print(f"({skipped} positions skipped: no bars)")

    print("\n=== HOW LONG POSITIONS LIVED ===")
    import datetime as dt
    def secs(p):
        f = "%Y-%m-%dT%H:%M:%S"
        a = dt.datetime.strptime(p["entry_ts"][:19], f)
        b = dt.datetime.strptime(p["exit_ts"][:19], f)
        return (b - a).total_seconds()
    for name, _ in rows:
        hs = [secs(p) for p in pos if family(p["reason"]) == name]
        print(f"{name:22} median {sorted(hs)[len(hs)//2]/60:>7.1f} min")


if __name__ == "__main__":
    main()


def bucket_counterfactual():
    """For each exit family: what did it actually return, and what would the
    SAME positions have returned held to the close instead? This is the test
    the aggregate hides — the trail stop can be saving money on the names it
    cuts while still producing most of the losing trades."""
    pos = positions_with_reasons()
    from collections import defaultdict
    g = defaultdict(list)
    for p in pos:
        c = close_price(p["sym"], p["day"])
        if c is None:
            continue
        g[family(p["reason"])].append((
            p["qty"] * (p["exit_px"] - p["entry_px"]),
            p["qty"] * (c - p["entry_px"]),
        ))

    print(f"\n{'exit reason':22} {'n':>4} {'ACTUAL':>9} {'win%':>6} "
          f"{'HELD to close':>14} {'win%':>6} {'exit worth':>11}")
    for name, rows in sorted(g.items(), key=lambda kv: sum(a for a, _ in kv[1])):
        a = [x for x, _ in rows]; h = [y for _, y in rows]
        wa = 100 * sum(1 for x in a if x > 0) / len(a)
        wh = 100 * sum(1 for x in h if x > 0) / len(h)
        print(f"{name:22} {len(rows):>4} {sum(a):>9.2f} {wa:>5.0f}% "
              f"{sum(h):>14.2f} {wh:>5.0f}% {sum(a)-sum(h):>11.2f}")

    print("\n=== WHAT A WIDER TRAIL WOULD HAVE DONE ===")
    print("Only the positions the trail actually closed, re-run at other widths.")
    trail = [p for p in pos if family(p["reason"]) == "TRAIL_STOP"]
    for width in [0.50, 0.75, 1.00, 1.50, 2.00, 3.00]:
        tot = n = wins = 0
        for p in trail:
            bs = lab.window(p)
            if not bs:
                continue
            peak = p["entry_px"]
            out = None
            for b in bs:
                peak = max(peak, b["h"])
                if b["l"] <= peak * (1 - width / 100.0):
                    out = peak * (1 - width / 100.0)
                    break
            if out is None:
                out = bs[-1]["c"]
            pnl = p["qty"] * (out - p["entry_px"])
            tot += pnl; n += 1; wins += 1 if pnl > 0 else 0
        print(f"  trail {width:.2f}%   {n:>3} trades   {100*wins/n:>5.1f}% win   "
              f"total {tot:>8.2f}")


def timing_test():
    """Is it the STOCKS or the TIMING?

    Same symbols, same days, same position sizes — but bought at the first bar
    of the session and sold at the last. If that makes money while the real
    entries lose it, the selection is fine and the entry timing is the problem.
    Nothing here uses information unavailable at the open.
    """
    pos = positions_with_reasons()
    real = timed = 0.0
    rw = tw = n = 0
    per_day = {}
    for p in pos:
        bs = [b for b in lab.bars(p["sym"], p["day"])
              if "13:30" <= b["t"][11:16] <= "20:00"]
        if len(bs) < 2:
            continue
        open_px, close_px = bs[0]["o"], bs[-1]["c"]
        r = p["qty"] * (p["exit_px"] - p["entry_px"])
        # Same dollars deployed, bought at the open instead of when signalled.
        shares = (p["qty"] * p["entry_px"]) / open_px
        t = shares * (close_px - open_px)
        real += r; timed += t; n += 1
        rw += 1 if r > 0 else 0
        tw += 1 if t > 0 else 0
        d = per_day.setdefault(p["day"], [0.0, 0.0])
        d[0] += r; d[1] += t

    print(f"\n=== SAME STOCKS, SAME DAYS, SAME SIZE — {n} positions ===")
    print(f"  signalled entries   {real:>9.2f}   {100*rw/n:>5.1f}% win")
    print(f"  bought at the open  {timed:>9.2f}   {100*tw/n:>5.1f}% win")
    print(f"  cost of timing      {real-timed:>9.2f}")
    print(f"\n{'day':>12} {'signalled':>11} {'open->close':>12} {'timing cost':>12}")
    for d in sorted(per_day):
        a, b = per_day[d]
        print(f"{d:>12} {a:>11.2f} {b:>12.2f} {a-b:>12.2f}")
    beat = sum(1 for d in per_day if per_day[d][0] > per_day[d][1])
    print(f"\nsignalled entries beat buy-the-open on {beat} of {len(per_day)} days")
