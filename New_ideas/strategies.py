"""Ideas under test. Add new ones here.

A strategy is a function `(position, bars) -> (marginal_pnl, acted)`.

It returns the P&L of whatever EXTRA action the idea takes, not the P&L of the
position itself. That keeps the measurement honest: if adding to winners has an
edge, the added units must be profitable on their own. Modelling where the
capital came from can only obscure that.
"""


def pyramid(trigger_pct, add_multiple=1.0, max_adds=1):
    """Add to a position once it is winning by `trigger_pct`.

    The proposal being tested: a stock showing a profit during the day deserves
    more money, so concentrate into it.

    The mechanism it depends on is TREND — that a move already underway tends to
    continue. Its failure mode is MEAN REVERSION, where the moment a position
    looks strongest is the moment it is nearest a local high, so every add buys
    the top.

    `add_multiple` scales the added unit against the original position's
    notional, so 1.0 doubles the exposure and 2.0 triples it. `max_adds` allows
    laddering in as the position keeps running.
    """
    def strat(pos, bars):
        entry = pos["entry_px"]
        notional = pos["qty"] * entry
        pnl, adds, next_trigger = 0.0, 0, trigger_pct
        for b in bars:
            if adds >= max_adds:
                break
            px = float(b["c"])
            if (px - entry) / entry * 100.0 < next_trigger:
                continue
            qty = (notional * add_multiple) / px
            pnl += (pos["exit_px"] - px) * qty
            adds += 1
            # Ladder: the next add needs another trigger_pct of progress, so a
            # position wobbling around one threshold cannot be bought twice.
            next_trigger += trigger_pct
        return pnl, adds > 0

    return strat


def anti_pyramid(trigger_pct, add_multiple=1.0):
    """Add to a position once it is DOWN by `trigger_pct` — averaging down.

    Included because it is the exact opposite trade, and testing both is how you
    tell "my trigger works" apart from "this market mean-reverts". If pyramiding
    loses and averaging down wins, the market is reverting and the direction of
    the rule is what matters, not the rule.
    """
    def strat(pos, bars):
        entry = pos["entry_px"]
        notional = pos["qty"] * entry
        for b in bars:
            px = float(b["c"])
            if (px - entry) / entry * 100.0 <= -trigger_pct:
                qty = (notional * add_multiple) / px
                return (pos["exit_px"] - px) * qty, True
        return 0.0, False

    return strat


def add_at_fixed_time(minutes_after_entry, add_multiple=1.0):
    """Add a unit a fixed time after entry, ignoring price entirely.

    A second null hypothesis, and a pointed one: it has the same exposure
    profile as pyramiding but no trigger at all. If this matches the pyramid,
    then any result came from being more invested, not from choosing when.
    """
    def strat(pos, bars):
        if len(bars) <= minutes_after_entry:
            return 0.0, False
        b = bars[minutes_after_entry]
        px = float(b["c"])
        notional = pos["qty"] * pos["entry_px"]
        return (pos["exit_px"] - px) * (notional * add_multiple / px), True

    return strat
