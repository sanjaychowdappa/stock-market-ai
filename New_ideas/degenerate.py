"""Is RULE 4 selecting good names, or just trading less?

In a negative-expectancy system any rule that skips entries scores well for a
trivial reason: the trades it avoids were losing on average. The only way to
tell selection from abstention is to compare against skipping EVERYTHING.

If "skip all" scores about the same as the rule, the rule's mechanism does
nothing and the result is pure exposure reduction.
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lab, rules

positions = lab.load_positions()

total_pnl = sum((p["exit_px"] - p["entry_px"]) * p["qty"] for p in positions)
skip_all = -total_pnl   # avoiding every trade avoids the whole loss

print(f"{len(positions)} positions")
print(f"  actual realised P&L of all trades : {total_pnl:+.2f}")
print(f"  DEGENERATE: skip every entry      : {skip_all:+.2f}   <- the bar to beat\n")

print(f"  {'rule':<44}{'scores':>9}{'vs skip-all':>13}{'entries skipped':>17}")
print("  " + "-" * 84)
for h in (1, 2, 3):
    score, skipped = rules.rule4_legacy_only(positions, h)
    edge = score - skip_all
    verdict = "adds selection" if edge > 0 else "NO better than not trading"
    print(f"  RULE 4  legacy after {h} trade(s){'':<15}{score:>+9.2f}{edge:>+13.2f}"
          f"{skipped:>10} / {len(positions)}   {verdict}")
