"""Run the ideas in strategies.py against real fills.

    py New_ideas\\run.py

Nothing here touches the live system. It reads the production fill log and
Alpaca's historical bars, and writes only to a local bar cache.
"""
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import lab
import strategies


def main():
    positions = lab.load_positions()
    days = sorted({p["day"] for p in positions})
    print(f"{len(positions)} real closed positions across {len(days)} days "
          f"({days[0]} .. {days[-1]})")
    print("Every idea is scored against doing nothing, against random timing,\n"
          "and re-checked with each day withheld.")

    results = []

    # The idea as proposed: put more into whatever is showing a profit.
    for trigger in (0.25, 0.50, 1.00):
        results.append(lab.evaluate(
            f"PYRAMID  add 1x at +{trigger:.2f}%",
            strategies.pyramid(trigger), positions))

    # Concentrate harder into the winner, which is the strongest form of the
    # proposal: "invest max in the stock that shows profit".
    results.append(lab.evaluate(
        "PYRAMID  add 2x at +0.50% (max concentration)",
        strategies.pyramid(0.50, add_multiple=2.0), positions))

    # Ladder in as it keeps running.
    results.append(lab.evaluate(
        "PYRAMID  ladder, 3 adds of 1x every +0.50%",
        strategies.pyramid(0.50, max_adds=3), positions))

    # The opposite trade. If this wins where pyramiding loses, the market is
    # mean-reverting and the direction of the rule is what matters.
    results.append(lab.evaluate(
        "ANTI-PYRAMID  add 1x at -0.50% (average down)",
        strategies.anti_pyramid(0.50), positions))

    # Same exposure, no trigger at all.
    results.append(lab.evaluate(
        "CONTROL  add 1x 30 minutes after entry, price ignored",
        strategies.add_at_fixed_time(30), positions))

    print(f"\n{'=' * 66}")
    print("  SUMMARY")
    print(f"{'=' * 66}")
    print(f"  {'idea':<44}{'total':>10}  verdict")
    print(f"  {'-' * 62}")
    for r in sorted(results, key=lambda x: -x["total"]):
        print(f"  {r['name']:<44}{r['total']:>+10.2f}  {r['verdict'].split(' — ')[0]}")

    survivors = [r for r in results if r["verdict"].startswith("SURVIVES")]
    print()
    if survivors:
        print(f"  {len(survivors)} idea(s) survived all three checks:")
        for r in survivors:
            print(f"    - {r['name']}")
        print("  Worth a forward test on paper before anything else.")
    else:
        print("  Nothing survived. No idea here beats doing nothing, beating")
        print("  random timing, and holding up with a day withheld.")


if __name__ == "__main__":
    main()
