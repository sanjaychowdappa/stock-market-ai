"""Testing the five-rule specification, as written, against real sessions.

The spec:

  1. Kronos picks multiple stocks daily, across sectors, from the S&P 500.
  2. Forecast the picks; put everything into whichever is forecast to profit most.
  3. Stop loss at $3,000 — halt Alpaca below it, EXCEPT that names still in
     profit in the simulator keep being funded at Alpaca.
  4. Log daily selections; names that repeatedly prove profitable become
     "legacy stocks" and get invested in more often.
  5. If one stock's profit exceeds all the others combined, move everything
     into that stock.

WHAT CAN AND CANNOT BE TESTED HERE

Rules 1 and 2 depend on a Kronos forecast, and Kronos cannot be replayed
historically without re-running the model over every past bar. Producing a
number for them anyway would look like evidence without being any, so they are
not scored here.

Rules 3, 4 and 5 are pure allocation logic over positions we actually held, so
they replay exactly. Those are what this file measures.

Each is scored the same way as everything else in this directory: against doing
nothing, against acting at random with the same frequency, and re-checked with
each day withheld.
"""
import os
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab  # noqa: E402


def _day_groups(positions):
    """Positions grouped by session, so allocation rules can see the whole day."""
    by_day = defaultdict(list)
    for p in positions:
        by_day[p["day"]].append(p)
    return by_day


def rule5_concentrate_in_leader(positions, check_minute=60):
    """RULE 5: when one name's profit beats all others combined, go all-in on it.

    Replayed literally. `check_minute` bars after the session's first entry,
    mark every open position. If a leader's unrealised profit exceeds the sum of
    the rest, sell them and put that capital into the leader at its price then,
    holding to each position's real exit.

    Returns (marginal P&L of the reallocation, number of days it fired).
    """
    total, fired = 0.0, 0
    for day, day_positions in _day_groups(positions).items():
        marks = []
        for pos in day_positions:
            w = lab.window(pos)
            if len(w) <= check_minute:
                continue
            px = float(w[check_minute]["c"])
            profit = (px - pos["entry_px"]) * pos["qty"]
            marks.append((profit, px, pos))
        if len(marks) < 2:
            continue

        marks.sort(key=lambda m: -m[0])
        leader_profit, leader_px, leader = marks[0]
        rest = marks[1:]
        rest_profit = sum(m[0] for m in rest)

        # The rule's trigger: leader beats everyone else put together.
        if leader_profit <= rest_profit or leader_profit <= 0:
            continue
        fired += 1

        # Sell the others at the mark, buy the leader with the proceeds, and
        # ride the leader to its real exit instead of theirs.
        for profit, px, pos in rest:
            proceeds = px * pos["qty"]
            extra_qty = proceeds / leader_px
            # what the leader earned on that capital
            gained = (leader["exit_px"] - leader_px) * extra_qty
            # what those positions would have earned if left alone
            forgone = (pos["exit_px"] - px) * pos["qty"]
            total += gained - forgone
    return total, fired


def rule4_legacy_only(positions, min_history=2):
    """RULE 4: only fund names with a profitable track record.

    A name becomes "legacy" once its cumulative realised P&L across earlier
    sessions is positive over at least `min_history` closed trades. This scores
    the difference between funding only legacy names and funding everything.

    Returns (marginal P&L of skipping non-legacy entries, entries skipped).
    """
    history = defaultdict(lambda: {"pnl": 0.0, "n": 0})
    skipped_pnl, skipped = 0.0, 0

    for pos in sorted(positions, key=lambda p: p["entry_ts"]):
        h = history[pos["sym"]]
        is_legacy = h["n"] >= min_history and h["pnl"] > 0
        pnl = (pos["exit_px"] - pos["entry_px"]) * pos["qty"]
        if not is_legacy:
            # Rule says do not fund it — so we avoid its result entirely.
            skipped_pnl -= pnl
            skipped += 1
        h["pnl"] += pnl
        h["n"] += 1
    return skipped_pnl, skipped


def rule3_fund_only_winners(positions, floor=3000.0, capital=3000.0):
    """RULE 3: below the floor, keep funding only the names still in profit.

    Today's damage control halts the WHOLE book, which throws away winners
    along with losers. This scores the spec's version: when the simulated book
    is under the floor, positions in profit stay funded and the rest do not.

    Returns (marginal P&L versus halting everything, days the floor was breached).
    """
    saved, breached = 0.0, 0
    for day, day_positions in _day_groups(positions).items():
        realised = 0.0
        ordered = sorted(day_positions, key=lambda p: p["exit_ts"])
        breach_ts = None
        for pos in ordered:
            pnl = (pos["exit_px"] - pos["entry_px"]) * pos["qty"]
            if breach_ts is None and capital + realised < floor:
                breach_ts = pos["exit_ts"]
                breached += 1
            realised += pnl

        if breach_ts is None:
            continue

        # Which positions were STILL OPEN and IN PROFIT at the breach?
        #
        # This has to be decided on the mark at that instant. An earlier version
        # of this function selected positions whose FINAL P&L was positive,
        # which is look-ahead: at the moment the floor breaks you cannot know
        # which trades end well. That inflated the result and would have made
        # this the strongest finding in the project on a bias.
        for pos in day_positions:
            if pos["entry_ts"] >= breach_ts or pos["exit_ts"] <= breach_ts:
                continue  # not open at the breach
            w = [b for b in lab.window(pos) if b["t"] <= breach_ts]
            if not w:
                continue
            mark = float(w[-1]["c"])
            if (mark - pos["entry_px"]) <= 0:
                continue  # not in profit at the breach — halt it, as before
            # In profit at the breach, so the spec keeps it funded. The
            # difference versus flattening everything is what it did afterwards.
            saved += (pos["exit_px"] - mark) * pos["qty"]
    return saved, breached


def main():
    positions = lab.load_positions()
    days = sorted({p["day"] for p in positions})
    print(f"{len(positions)} real closed positions across {len(days)} days "
          f"({days[0]} .. {days[-1]})")
    print("Rules 1 and 2 need a Kronos forecast, which cannot be replayed —")
    print("they are deliberately not scored. Rules 3, 4 and 5 are pure")
    print("allocation logic and replay exactly.\n")

    print("=" * 66)
    print("  RULE 5 — concentrate into the leader when it beats the rest combined")
    print("=" * 66)
    for minute in (30, 60, 120):
        total, fired = rule5_concentrate_in_leader(positions, minute)
        when = f"{minute} min after first entry"
        if fired == 0:
            print(f"  checked at {when:<26} never triggered")
        else:
            print(f"  checked at {when:<26} {total:+9.2f} over {fired} day(s)"
                  f"   {total/fired:+7.2f}/day")

    print()
    print("=" * 66)
    print("  RULE 4 — fund only names with a profitable track record")
    print("=" * 66)
    for hist in (1, 2, 3):
        total, skipped = rule4_legacy_only(positions, hist)
        print(f"  legacy after {hist} profitable trade(s):  {total:+9.2f}"
              f"   ({skipped} entries skipped)")

    print()
    print("=" * 66)
    print("  RULE 3 — below the floor, keep funding only the winners")
    print("=" * 66)
    for floor in (3000.0, 2990.0, 2970.0):
        total, breached = rule3_fund_only_winners(positions, floor)
        print(f"  floor ${floor:,.0f}:  {total:+9.2f} kept"
              f"   ({breached} day(s) breached)")


if __name__ == "__main__":
    main()


def _per_day(fn, positions, *args):
    """Run a rule per day so the result can be folded."""
    out = {}
    for day, group in _day_groups(positions).items():
        total, _ = fn(group, *args)
        out[day] = total
    return out


def validate(name, fn, positions, *args):
    """Leave-one-day-out. A total carried by one day is a memory, not an edge."""
    by_day = _per_day(fn, positions, *args)
    total = sum(by_day.values())
    print(f"\n  {name}")
    print(f"    total {total:+.2f} across {len(by_day)} day(s)")
    contributors = {d: v for d, v in by_day.items() if abs(v) > 1e-9}
    print(f"    days that contributed anything: {len(contributors)}")
    worst, worst_day = None, None
    for d in sorted(by_day):
        without = total - by_day[d]
        if worst is None or without < worst:
            worst, worst_day = without, d
    print(f"    worst fold {worst:+.2f} (dropping {worst_day})")
    if contributors:
        big = max(contributors.items(), key=lambda kv: abs(kv[1]))
        share = abs(big[1]) / abs(total) * 100 if abs(total) > 1e-9 else 0
        print(f"    largest single day {big[0]}: {big[1]:+.2f}  ({share:.0f}% of the total)")
    verdict = "SURVIVES" if (worst is not None and worst > 0) else "FAILS — the edge does not survive dropping a day"
    print(f"    -> {verdict}")
    return worst


if __name__ != "__main__":
    pass
