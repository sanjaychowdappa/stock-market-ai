#!/usr/bin/env python3
"""
Backtest: Before Layers vs After Layers

Fetches real Alpaca historical data and simulates two strategies:
  A) PATTERN-ONLY (old system — no institutional signals)
  B) PATTERN + KALMAN + CVD + GEX + VP + COT (new full-layer system)

Compares prediction accuracy, win rate, P&L, and efficiency.
"""

import os
import sys
import json
import math
import numpy as np
import pandas as pd
from datetime import datetime, timedelta
from collections import deque

# ── Fetch real data from Alpaca ────────────────────────────
def fetch_alpaca_bars(symbol, timeframe="1Min", days=5):
    """Fetch intraday bars from Alpaca."""
    import httpx
    api_key = os.environ.get("APCA_API_KEY_ID", "")
    api_secret = os.environ.get("APCA_API_SECRET_KEY", "")
    if not api_key:
        raise ValueError("Set APCA_API_KEY_ID and APCA_API_SECRET_KEY")

    end = datetime.now()
    start = end - timedelta(days=days)
    all_bars = []
    page_token = None

    while True:
        params = {
            "timeframe": timeframe,
            "start": start.strftime("%Y-%m-%dT00:00:00Z"),
            "end": end.strftime("%Y-%m-%dT23:59:59Z"),
            "limit": 10000, "feed": "iex", "sort": "asc",
        }
        if page_token:
            params["page_token"] = page_token

        resp = httpx.get(
            f"https://data.alpaca.markets/v2/stocks/{symbol}/bars",
            params=params,
            headers={"APCA-API-KEY-ID": api_key, "APCA-API-SECRET-KEY": api_secret},
            timeout=30,
        )
        data = resp.json()
        bars = data.get("bars", [])
        if not bars:
            break
        all_bars.extend(bars)
        page_token = data.get("next_page_token")
        if not page_token:
            break

    return all_bars


# ── Kalman Filter (Python port) ────────────────────────────
class KalmanFilter:
    def __init__(self, price, dt=1.0, process_noise=0.01, meas_noise=0.05):
        self.x = np.array([price, 0.0, 0.0])  # [price, velocity, accel]
        self.P = np.diag([meas_noise**2, meas_noise*10, meas_noise*100])
        self.dt = dt
        self.Q = np.diag([process_noise*0.1, process_noise*1.0, process_noise*5.0])
        self.R = meas_noise**2
        self.F = np.array([
            [1, dt, dt**2/2],
            [0, 1, dt],
            [0, 0, 0.95],
        ])

    def update(self, z):
        # Predict
        self.x = self.F @ self.x
        self.P = self.F @ self.P @ self.F.T + self.Q
        # Update
        H = np.array([[1, 0, 0]])
        y = z - H @ self.x
        S = H @ self.P @ H.T + self.R
        K = self.P @ H.T / S[0, 0]
        self.x = self.x + K.flatten() * y[0]
        self.P = (np.eye(3) - K @ H) @ self.P
        return self.x[0], self.x[1], self.x[2]  # price, velocity, accel


# ── Pattern Detection (simplified) ────────────────────────
def detect_pattern(prices, n=10):
    """Detect bullish/bearish patterns from recent prices."""
    if len(prices) < n:
        return 0.0, "neutral"
    recent = prices[-n:]
    # Simple momentum: weighted slope
    weights = np.arange(1, n+1, dtype=float)
    weighted_change = sum((recent[i] - recent[i-1]) / recent[i-1] * weights[i] for i in range(1, n))
    signal = weighted_change * 100
    # Engulfing check
    if len(recent) >= 3:
        if recent[-1] > recent[-2] > recent[-3]:  # Three rising
            signal += 0.1
        elif recent[-1] < recent[-2] < recent[-3]:  # Three falling
            signal -= 0.1
    direction = "bullish" if signal > 0.02 else ("bearish" if signal < -0.02 else "neutral")
    return signal, direction


# ── CVD Tracker ────────────────────────────────────────────
class CVDTracker:
    def __init__(self):
        self.cvd = 0.0
        self.last_price = 0.0
        self.buy_vol = 0.0
        self.sell_vol = 0.0

    def process(self, price, volume=1.0):
        if self.last_price > 0:
            if price > self.last_price:
                self.cvd += volume
                self.buy_vol += volume
            elif price < self.last_price:
                self.cvd -= volume
                self.sell_vol += volume
        self.last_price = price

    @property
    def signal(self):
        total = self.buy_vol + self.sell_vol
        if total == 0:
            return 0.0
        ratio = self.buy_vol / max(self.sell_vol, 0.001)
        return min(max((ratio - 1.0) * 2.0, -1.0), 1.0)


# ── GEX Estimator ─────────────────────────────────────────
def estimate_gex(prices):
    if len(prices) < 20:
        return 0.0, "neutral"
    returns = [math.log(prices[i]/prices[i-1]) for i in range(1, len(prices))]
    mean_r = np.mean(returns)
    vol_full = np.std(returns) * math.sqrt(252)
    vol_recent = np.std(returns[-5:]) * math.sqrt(252) if len(returns) >= 5 else vol_full
    ratio = vol_recent / max(vol_full, 0.001)
    if ratio < 0.8:
        return 0.3, "long_gamma"
    elif ratio > 1.3:
        return -0.3, "short_gamma"
    return 0.0, "neutral"


# ── Volume Profile ─────────────────────────────────────────
def compute_vp(prices, volumes, current_price, levels=20):
    if len(prices) < 10:
        return 0.0, "unknown"
    min_p, max_p = min(prices), max(prices)
    if max_p <= min_p:
        return 0.0, "unknown"
    level_size = (max_p - min_p) / levels
    vol_at = [0.0] * levels
    for p, v in zip(prices, volumes):
        idx = min(int((p - min_p) / level_size), levels - 1)
        vol_at[idx] += v

    poc_idx = np.argmax(vol_at)
    poc = min_p + (poc_idx + 0.5) * level_size

    # Value area (70%)
    total = sum(vol_at)
    va_target = total * 0.7
    lo, hi = poc_idx, poc_idx
    va_vol = vol_at[poc_idx]
    while va_vol < va_target:
        up = vol_at[hi+1] if hi+1 < levels else 0
        dn = vol_at[lo-1] if lo > 0 else 0
        if up >= dn and hi+1 < levels:
            hi += 1; va_vol += up
        elif lo > 0:
            lo -= 1; va_vol += dn
        else:
            break

    va_high = min_p + (hi + 1) * level_size
    va_low = min_p + lo * level_size

    if current_price > va_high:
        return -0.3, "above_value"
    elif current_price < va_low:
        return 0.5, "below_value"
    else:
        return 0.0, "in_value"


# ── Simulate Strategy A: Pattern Only ──────────────────────
def simulate_pattern_only(bars):
    """Old system: just pattern detection + momentum."""
    prices = [b["c"] for b in bars]
    trades = []
    position = None
    signal_hist = deque(maxlen=15)
    cooldown = 0

    for i in range(30, len(prices)):
        price = prices[i]
        cooldown = max(0, cooldown - 1)

        signal, direction = detect_pattern(prices[max(0,i-15):i+1])
        signal_hist.append(signal)

        if position:
            position["hold"] += 1
            pnl_pct = (price - position["entry"]) / position["entry"] * 100
            # Exits
            if pnl_pct <= -0.15:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "HARD_STOP"})
                position = None; cooldown = 20
            elif pnl_pct >= 0.12:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "TAKE_PROFIT"})
                position = None; cooldown = 20
            elif position["hold"] >= 180:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "FLAT_EXIT"})
                position = None; cooldown = 20
            elif len(signal_hist) >= 5 and all(s < -0.05 for s in list(signal_hist)[-5:]):
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "MOM_DEAD"})
                position = None; cooldown = 20
        else:
            if cooldown > 0:
                continue
            # Entry: pattern signal positive + confirmed
            if signal > 0.02 and direction == "bullish":
                pos_count = sum(1 for s in list(signal_hist)[-5:] if s > 0.005)
                if pos_count >= 3:
                    position = {"entry": price, "hold": 0}

    return trades


# ── Simulate Strategy B: Full Layers ──────────────────────
def simulate_full_layers(bars):
    """New system: Kalman + CVD + GEX + VP + Pattern combined."""
    prices = [b["c"] for b in bars]
    volumes = [b["v"] for b in bars]
    trades = []
    position = None
    signal_hist = deque(maxlen=15)
    cooldown = 0

    # Initialize layers
    kf = KalmanFilter(prices[0], process_noise=prices[0]*0.0001, meas_noise=prices[0]*0.0005)
    cvd = CVDTracker()

    # Pre-compute daily-level signals
    daily_prices = prices[::60] if len(prices) > 60 else prices  # Downsample to ~daily
    gex_signal, gex_regime = estimate_gex(daily_prices)
    vp_signal, vp_pos = compute_vp(prices[:len(prices)//2], volumes[:len(volumes)//2], prices[len(prices)//2])

    for i in range(30, len(prices)):
        price = prices[i]
        vol = volumes[i] if i < len(volumes) else 1.0
        cooldown = max(0, cooldown - 1)

        # Update all layers
        kf_price, kf_vel, kf_accel = kf.update(price)
        cvd.process(price, vol)
        signal, direction = detect_pattern(prices[max(0,i-15):i+1])
        signal_hist.append(signal)

        # Kalman-derived signals
        kf_momentum = kf_vel * 100
        kf_building = (kf_vel > 0 and kf_accel > 0) or (kf_vel < 0 and kf_accel < 0)
        kf_fading = (kf_vel > 0 and kf_accel < 0) or (kf_vel < 0 and kf_accel > 0)
        kf_bullish = kf_vel > 0

        if position:
            position["hold"] += 1
            pnl_pct = (price - position["entry"]) / position["entry"] * 100

            # Enhanced exits using Kalman
            mom_dead = (kf_fading and abs(kf_momentum) < 0.01) or \
                       (len(signal_hist) >= 5 and all(s < -0.05 for s in list(signal_hist)[-5:]))

            if pnl_pct <= -0.15:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "HARD_STOP"})
                position = None; cooldown = 20
            elif pnl_pct >= 0.12:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "TAKE_PROFIT"})
                position = None; cooldown = 20
            elif mom_dead and position["hold"] > 30:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "MOM_DEAD"})
                position = None; cooldown = 20
            elif position["hold"] >= 180:
                trades.append({"pnl": pnl_pct, "hold": position["hold"], "exit": "FLAT_EXIT"})
                position = None; cooldown = 20
        else:
            if cooldown > 0:
                continue

            # Full-layer entry filters
            if not kf_bullish:
                continue  # Kalman not bullish
            if kf_fading:
                continue  # Momentum fading
            if cvd.signal < -0.3:
                continue  # Sell pressure
            if vp_pos == "above_value" and vp_signal < -0.2:
                continue  # At resistance
            if signal < 0.01:
                continue  # Pattern not positive

            # Pattern confirmation
            pos_count = sum(1 for s in list(signal_hist)[-5:] if s > 0.005)
            if pos_count < 3:
                continue

            # GEX regime adjustment
            gex_boost = 0.05 if gex_regime == "short_gamma" else (-0.02 if gex_regime == "long_gamma" else 0)

            # Composite score
            score = (0.20 * max(kf_momentum, 0)
                   + 0.20 * signal
                   + 0.12 * min(abs(kf_vel) / max(abs(kf_price * 0.0005), 1e-10), 3.0) / 3.0
                   + 0.12 * max(cvd.signal, 0)
                   + 0.08 * max(vp_signal, 0)
                   + 0.06 * max(gex_signal, 0)
                   + gex_boost)

            if score > 0.04:
                position = {"entry": price, "hold": 0}

    return trades


# ── Analysis ───────────────────────────────────────────────
def analyze_trades(trades, label):
    if not trades:
        return {"label": label, "trades": 0, "win_rate": 0, "avg_pnl": 0, "total_pnl": 0}

    wins = [t for t in trades if t["pnl"] > 0]
    losses = [t for t in trades if t["pnl"] <= 0]
    total_pnl = sum(t["pnl"] for t in trades)
    avg_pnl = total_pnl / len(trades)
    win_rate = len(wins) / len(trades) * 100
    avg_hold = sum(t["hold"] for t in trades) / len(trades)
    avg_win = sum(t["pnl"] for t in wins) / len(wins) if wins else 0
    avg_loss = sum(t["pnl"] for t in losses) / len(losses) if losses else 0
    profit_factor = abs(sum(t["pnl"] for t in wins)) / abs(sum(t["pnl"] for t in losses)) if losses else 999

    exits = {}
    for t in trades:
        exits[t["exit"]] = exits.get(t["exit"], 0) + 1

    return {
        "label": label,
        "trades": len(trades),
        "wins": len(wins),
        "losses": len(losses),
        "win_rate": round(win_rate, 1),
        "avg_pnl_pct": round(avg_pnl, 4),
        "total_pnl_pct": round(total_pnl, 4),
        "avg_win": round(avg_win, 4),
        "avg_loss": round(avg_loss, 4),
        "profit_factor": round(profit_factor, 2),
        "avg_hold": round(avg_hold, 1),
        "exits": exits,
    }


def main():
    symbols = ["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"]

    print("=" * 70)
    print("  BACKTEST: Pattern-Only vs Full 7-Layer System")
    print("  Using real Alpaca 1-minute data (last 5 trading days)")
    print("=" * 70)

    all_pattern = []
    all_layers = []

    for sym in symbols:
        print(f"\n  Fetching {sym}...", end=" ")
        try:
            raw_bars = fetch_alpaca_bars(sym, "1Min", days=5)
            bars = [{"c": b["c"], "o": b["o"], "h": b["h"], "l": b["l"], "v": b["v"]}
                    for b in raw_bars if b.get("c")]
            print(f"{len(bars)} bars")

            if len(bars) < 100:
                print(f"    Skipping — not enough data")
                continue

            # Run both strategies
            pattern_trades = simulate_pattern_only(bars)
            layer_trades = simulate_full_layers(bars)

            all_pattern.extend(pattern_trades)
            all_layers.extend(layer_trades)

            pa = analyze_trades(pattern_trades, f"{sym} Pattern-Only")
            la = analyze_trades(layer_trades, f"{sym} Full-Layers")

            print(f"    Pattern-Only: {pa['trades']} trades, {pa['win_rate']}% WR, "
                  f"PF={pa['profit_factor']}, PnL={pa['total_pnl_pct']:.3f}%")
            print(f"    Full-Layers:  {la['trades']} trades, {la['win_rate']}% WR, "
                  f"PF={la['profit_factor']}, PnL={la['total_pnl_pct']:.3f}%")

            better = "LAYERS" if la.get("total_pnl_pct", 0) > pa.get("total_pnl_pct", 0) else "PATTERN"
            print(f"    Winner: {better}")

        except Exception as e:
            print(f"ERROR: {e}")

    # Overall comparison
    print("\n" + "=" * 70)
    print("  OVERALL COMPARISON")
    print("=" * 70)

    pa = analyze_trades(all_pattern, "ALL Pattern-Only")
    la = analyze_trades(all_layers, "ALL Full-Layers")

    print(f"""
  ┌────────────────────┬──────────────────┬──────────────────┐
  │                    │  PATTERN-ONLY    │  FULL 7-LAYER    │
  ├────────────────────┼──────────────────┼──────────────────┤
  │ Total Trades       │  {pa['trades']:<16} │  {la['trades']:<16} │
  │ Wins               │  {pa['wins']:<16} │  {la['wins']:<16} │
  │ Losses             │  {pa['losses']:<16} │  {la['losses']:<16} │
  │ Win Rate           │  {pa['win_rate']:<15}% │  {la['win_rate']:<15}% │
  │ Avg P&L/trade      │  {pa['avg_pnl_pct']:<15}% │  {la['avg_pnl_pct']:<15}% │
  │ Total P&L          │  {pa['total_pnl_pct']:<15}% │  {la['total_pnl_pct']:<15}% │
  │ Avg Win            │  {pa['avg_win']:<15}% │  {la['avg_win']:<15}% │
  │ Avg Loss           │  {pa['avg_loss']:<15}% │  {la['avg_loss']:<15}% │
  │ Profit Factor      │  {pa['profit_factor']:<16} │  {la['profit_factor']:<16} │
  │ Avg Hold (bars)    │  {pa['avg_hold']:<16} │  {la['avg_hold']:<16} │
  └────────────────────┴──────────────────┴──────────────────┘
""")

    # Verdict
    pattern_pnl = pa.get("total_pnl_pct", 0)
    layer_pnl = la.get("total_pnl_pct", 0)
    improvement = layer_pnl - pattern_pnl

    print(f"  P&L Improvement:  {improvement:+.4f}%")
    print(f"  Win Rate Change:  {la['win_rate'] - pa['win_rate']:+.1f}%")
    print(f"  Profit Factor:    {pa['profit_factor']} → {la['profit_factor']}")
    print()

    if improvement > 0:
        print("  ✅ FULL 7-LAYER SYSTEM IS BETTER")
        print(f"     Layers improved P&L by {improvement:.4f}% over pattern-only")
    elif improvement == 0:
        print("  ⚠️  NO SIGNIFICANT DIFFERENCE")
    else:
        print("  ❌ PATTERN-ONLY WAS BETTER")
        print(f"     Layers reduced P&L by {abs(improvement):.4f}%")

    print()
    print("  Exit Breakdown:")
    print(f"    Pattern-Only: {pa.get('exits', {})}")
    print(f"    Full-Layers:  {la.get('exits', {})}")

    # Save results
    results = {
        "backtest_date": datetime.now().isoformat(),
        "data_range": "5 trading days of 1-min bars",
        "symbols": symbols,
        "pattern_only": pa,
        "full_layers": la,
        "improvement_pct": round(improvement, 4),
        "verdict": "LAYERS_BETTER" if improvement > 0 else "PATTERN_BETTER",
    }
    out_path = os.path.join(os.path.dirname(__file__), "..", "reports", "backtest_layers.json")
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\n  Results saved to: {out_path}")


if __name__ == "__main__":
    main()
