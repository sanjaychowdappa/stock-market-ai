r"""How much of a winning day should the profit lock let you give back?

The floor and the ratchet act on the INTRADAY equity path, not on the day's
closing number, so a decision made from daily_profit.jsonl would be answering
a different question than the one the code asks. This reconstructs the path
minute by minute from real fills and real minute bars, then replays the actual
damage-control rule over it at a range of giveback widths.

  py New_ideas\giveback.py
"""
import sys, os, json
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab

CAPITAL = 3000.0
FLOOR_PCT = -0.30      # config::CAPITAL_FLOOR_PCT
TRIGGER_PCT = 0.30     # config::PROFIT_LOCK_TRIGGER_PCT


def px_by_minute(sym, day):
    """{minute_iso: close} for one symbol on one day."""
    out = {}
    for b in lab.bars(sym, day):
        out[b["t"][:16]] = b["c"]
    return out


def equity_paths(positions):
    """Per day: [(minute, day_pnl_dollars)] marked to market, minute by minute."""
    by_day = defaultdict(list)
    for p in positions:
        by_day[p["day"]].append(p)

    paths = {}
    for day, poss in sorted(by_day.items()):
        prices, minutes = {}, set()
        for sym in {p["sym"] for p in poss}:
            prices[sym] = px_by_minute(sym, day)
            minutes |= set(prices[sym])
        if not minutes:
            continue
        # Regular session only — the floor is a trading-day rule.
        mins = sorted(m for m in minutes if "13:30" <= m[11:] <= "20:00")
        if not mins:
            continue

        path = []
        for m in mins:
            pnl = 0.0
            for p in poss:
                entry, exit_ = p["entry_ts"][:16], p["exit_ts"][:16]
                if m < entry:
                    continue
                if m >= exit_:
                    pnl += p["qty"] * (p["exit_px"] - p["entry_px"])   # realised
                else:
                    mark = prices[p["sym"]].get(m)
                    if mark is None:
                        continue
                    pnl += p["qty"] * (mark - p["entry_px"])           # open
            path.append((m, pnl))
        paths[day] = path
    return paths


def replay(path, giveback, floor_pct=FLOOR_PCT, trigger_pct=TRIGGER_PCT):
    """Day-end P&L under the damage-control rule. Returns (pnl, halted, minute)."""
    peak = 0.0
    for minute, pnl in path:
        pct = pnl / CAPITAL * 100.0
        peak = max(peak, pct)
        floor = max(floor_pct, peak - giveback) if peak >= trigger_pct else floor_pct
        if pct <= floor:
            # Halt flattens everything at this minute and stops real orders.
            return pnl, True, minute
    return path[-1][1], False, None


def summarise(paths, giveback):
    total, halts = 0.0, 0
    per_day = {}
    for day, path in paths.items():
        pnl, halted, _ = replay(path, giveback)
        total += pnl
        halts += 1 if halted else 0
        per_day[day] = pnl
    return total, halts, per_day


def main():
    positions = lab.load_positions()
    print(f"{len(positions)} real closed positions")
    paths = equity_paths(positions)
    print(f"{len(paths)} days reconstructed: {min(paths)} -> {max(paths)}\n")

    # What actually happened, with no floor and no ratchet at all.
    raw = {d: p[-1][1] for d, p in paths.items()}
    raw_total = sum(raw.values())

    widths = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.50, 0.75, 1.00]
    print(f"{'giveback':>9} {'total':>9} {'halts':>6} {'best day':>9} {'worst day':>10}")
    print(f"{'NO LOCK':>9} {raw_total:>9.2f} {'-':>6} "
          f"{max(raw.values()):>9.2f} {min(raw.values()):>10.2f}")
    results = {}
    for w in widths:
        total, halts, per_day = summarise(paths, w)
        results[w] = per_day
        print(f"{w:>9.2f} {total:>9.2f} {halts:>6} "
              f"{max(per_day.values()):>9.2f} {min(per_day.values()):>10.2f}")

    # LEAVE-ONE-DAY-OUT: does the winner survive dropping any single day?
    print("\nleave-one-day-out (worst fold per width — the number that matters)")
    print(f"{'giveback':>9} {'all days':>9} {'worst fold':>11} {'losing folds':>13}")
    days = sorted(paths)
    for w in widths:
        per_day = results[w]
        full = sum(per_day.values())
        folds = [full - per_day[d] for d in days]
        losing = sum(1 for f in folds if f < 0)
        print(f"{w:>9.2f} {full:>9.2f} {min(folds):>11.2f} {losing:>9}/{len(folds)}")

    # Per-day detail for the two candidates under discussion.
    print(f"\n{'day':>12} {'no lock':>9} {'gb 0.15':>9} {'gb 0.50':>9}")
    for d in days:
        print(f"{d:>12} {raw[d]:>9.2f} {results[0.15][d]:>9.2f} {results[0.50][d]:>9.2f}")


if __name__ == "__main__":
    main()


def sweep():
    """Trigger x giveback. The giveback cannot be chosen without the trigger:
    an early-arming lock halts on the first ordinary wiggle no matter how wide
    the giveback is."""
    positions = lab.load_positions()
    paths = equity_paths(positions)
    days = sorted(paths)
    raw = {d: p[-1][1] for d, p in paths.items()}

    triggers = [0.30, 0.50, 0.75, 1.00, 1.50]
    widths = [0.15, 0.25, 0.50, 0.75, 1.00]

    print("TOTAL P&L over %d days (no lock at all: %.2f)\n" % (len(days), sum(raw.values())))
    print("           " + "".join(f"{'gb '+format(w,'.2f'):>10}" for w in widths))
    for t in triggers:
        row = f"trig {t:>5.2f} "
        for w in widths:
            tot = sum(replay(paths[d], w, trigger_pct=t)[0] for d in days)
            row += f"{tot:>10.2f}"
        print(row)

    print("\nBEST DAY (no lock: %.2f) — how much upside survives\n" % max(raw.values()))
    print("           " + "".join(f"{'gb '+format(w,'.2f'):>10}" for w in widths))
    for t in triggers:
        row = f"trig {t:>5.2f} "
        for w in widths:
            best = max(replay(paths[d], w, trigger_pct=t)[0] for d in days)
            row += f"{best:>10.2f}"
        print(row)

    print("\nWORST DAY (no lock: %.2f) — how much downside is cut\n" % min(raw.values()))
    print("           " + "".join(f"{'gb '+format(w,'.2f'):>10}" for w in widths))
    for t in triggers:
        row = f"trig {t:>5.2f} "
        for w in widths:
            worst = min(replay(paths[d], w, trigger_pct=t)[0] for d in days)
            row += f"{worst:>10.2f}"
        print(row)

    print("\nHALTED DAYS out of %d\n" % len(days))
    print("           " + "".join(f"{'gb '+format(w,'.2f'):>10}" for w in widths))
    for t in triggers:
        row = f"trig {t:>5.2f} "
        for w in widths:
            n = sum(1 for d in days if replay(paths[d], w, trigger_pct=t)[1])
            row += f"{n:>10}"
        print(row)


def candidates():
    """Leave-one-day-out on the settings actually under consideration.

    The repo rule: never accept a parameter on its full-sample score. A width
    that memorises one session shows up here and nowhere else.
    """
    positions = lab.load_positions()
    paths = equity_paths(positions)
    days = sorted(paths)
    raw = {d: p[-1][1] for d, p in paths.items()}

    cands = [
        ("A  current      trig 0.30 / gb 0.15", 0.30, 0.15),
        ("B  proposed     trig 0.50 / gb 0.15", 0.50, 0.15),
        ("C  wide giveback trig 0.30 / gb 0.50", 0.30, 0.50),
        ("D  lock off     trig 9.99 / gb 9.99", 9.99, 9.99),
        ("E  no floor either (raw)",            None, None),
    ]

    print(f"{'setting':38} {'total':>8} {'worst fold':>11} {'best day':>9} "
          f"{'worst day':>10} {'halts':>6}")
    per = {}
    for name, t, w in cands:
        if t is None:
            pd = dict(raw)
            halts = 0
        else:
            pd = {d: replay(paths[d], w, trigger_pct=t)[0] for d in days}
            halts = sum(1 for d in days if replay(paths[d], w, trigger_pct=t)[1])
        per[name] = pd
        total = sum(pd.values())
        folds = [total - pd[d] for d in days]
        print(f"{name:38} {total:>8.2f} {min(folds):>11.2f} "
              f"{max(pd.values()):>9.2f} {min(pd.values()):>10.2f} {halts:>6}")

    a, b = per[cands[0][0]], per[cands[1][0]]
    wins = sum(1 for d in days if b[d] > a[d] + 1e-9)
    ties = sum(1 for d in days if abs(b[d] - a[d]) <= 1e-9)
    print(f"\nB vs A, day by day: B better on {wins}, tied on {ties}, "
          f"worse on {len(days)-wins-ties} of {len(days)}")
    print(f"{'day':>12} {'A 0.30/0.15':>12} {'B 0.50/0.15':>12} {'diff':>8}")
    for d in days:
        print(f"{d:>12} {a[d]:>12.2f} {b[d]:>12.2f} {b[d]-a[d]:>8.2f}")
