"""Leave-one-day-out on the three testable rules.

A total carried by one day is a memory of that day, not an edge. This project
has twice shipped a parameter that scored well overall and collapsed when a
single session was withheld, so nothing here is reported without this check.
"""
import os
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab
import rules


def fold(name, fn, positions, *args):
    groups = rules._day_groups(positions)
    by_day = {}
    for day, group in groups.items():
        total, _ = fn(group, *args)
        by_day[day] = total
    total = sum(by_day.values())
    active = {d: v for d, v in by_day.items() if abs(v) > 1e-9}

    worst, worst_day = None, None
    for d in by_day:
        without = total - by_day[d]
        if worst is None or without < worst:
            worst, worst_day = without, d

    biggest = max(active.items(), key=lambda kv: abs(kv[1])) if active else ("-", 0.0)
    share = (abs(biggest[1]) / abs(total) * 100) if abs(total) > 1e-9 else 0.0

    verdict = "SURVIVES" if (worst is not None and worst > 0) else "FAILS"
    print(f"  {name:<44}{total:>+9.2f}{worst:>+11.2f}   {biggest[0]} = {share:>3.0f}%   {verdict}")
    return worst


def main():
    positions = lab.load_positions()
    print(f"{len(positions)} positions, {len({p['day'] for p in positions})} days\n")
    print(f"  {'rule':<44}{'total':>9}{'worst fold':>11}   biggest day   verdict")
    print("  " + "-" * 88)

    for m in (30, 60, 120):
        fold(f"RULE 5  concentrate in leader @ {m}min",
             rules.rule5_concentrate_in_leader, positions, m)
    print()
    for h in (1, 2, 3):
        fold(f"RULE 4  legacy after {h} profitable trade(s)",
             rules.rule4_legacy_only, positions, h)
    print()
    for f in (3000.0, 2990.0, 2970.0):
        fold(f"RULE 3  fund only winners below ${f:,.0f}",
             rules.rule3_fund_only_winners, positions, f)


if __name__ == "__main__":
    main()
