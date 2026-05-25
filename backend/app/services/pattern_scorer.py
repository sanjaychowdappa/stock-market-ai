"""
Pattern Scorer — analyses recent 1-min candles for repeating chart patterns,
scores their directional bias, and produces an adjustment signal that the
Kronos 1-min predictor uses to bias its next-candle forecast.

Pattern categories scored:
 1. Classic candlestick patterns (doji, hammer, engulfing, etc.)
 2. Micro-structure repeats  — sliding-window DTW on last N candles
 3. Support / Resistance proximity
 4. Momentum regime (RSI / MACD micro)
"""

import numpy as np
import pandas as pd
import logging
from typing import Optional

logger = logging.getLogger(__name__)


# ── Micro-pattern repeat detector ────────────────────────────────
def _normalise_window(closes: np.ndarray) -> np.ndarray:
    """Zero-mean, unit-variance normalisation of a close series."""
    std = np.std(closes)
    if std < 1e-9:
        return np.zeros_like(closes)
    return (closes - np.mean(closes)) / std


def find_repeating_patterns(
    closes: np.ndarray,
    window: int = 10,
    search_depth: int = 120,
    top_k: int = 3,
    similarity_threshold: float = 0.80,
) -> list[dict]:
    """
    Slide a window of `window` bars back through the last `search_depth` bars
    and find the top-k most similar subsequences to the most-recent window.
    For each match, record what happened in the bar immediately *after* the
    pattern completed — that's our signal for the next candle.
    """
    if len(closes) < window + 2:
        return []

    query = _normalise_window(closes[-window:])
    search_start = max(0, len(closes) - search_depth - window)
    search_end = len(closes) - window - 1  # leave room for "what-came-next"

    matches = []
    for i in range(search_start, search_end):
        candidate = _normalise_window(closes[i : i + window])
        # Pearson correlation as similarity
        corr = float(np.corrcoef(query, candidate)[0, 1])
        if corr >= similarity_threshold:
            next_idx = i + window
            if next_idx < len(closes):
                prev_close = closes[i + window - 1]
                next_close = closes[next_idx]
                pct_change = (next_close - prev_close) / prev_close * 100
                matches.append({
                    "offset": i,
                    "similarity": round(corr, 4),
                    "next_change_pct": round(pct_change, 4),
                })

    # Sort by similarity desc, take top k
    matches.sort(key=lambda m: m["similarity"], reverse=True)
    return matches[:top_k]


# ── Support / Resistance micro-levels ────────────────────────────
def detect_micro_sr(
    highs: np.ndarray, lows: np.ndarray, closes: np.ndarray, lookback: int = 60
) -> dict:
    """Detect the nearest support and resistance from recent pivot points."""
    h = highs[-lookback:]
    l = lows[-lookback:]
    current = closes[-1]

    # Simple pivot: local max/min over 5-bar windows
    resistances = []
    supports = []
    for i in range(2, len(h) - 2):
        if h[i] > h[i - 1] and h[i] > h[i - 2] and h[i] > h[i + 1] and h[i] > h[i + 2]:
            resistances.append(float(h[i]))
        if l[i] < l[i - 1] and l[i] < l[i - 2] and l[i] < l[i + 1] and l[i] < l[i + 2]:
            supports.append(float(l[i]))

    nearest_resistance = min(resistances, key=lambda r: r - current, default=None) if resistances else None
    nearest_support = max(
        [s for s in supports if s < current], key=lambda s: s, default=None
    ) if supports else None

    return {
        "current": float(current),
        "nearest_resistance": nearest_resistance,
        "nearest_support": nearest_support,
        "resistance_distance_pct": round((nearest_resistance - current) / current * 100, 3) if nearest_resistance else None,
        "support_distance_pct": round((current - nearest_support) / current * 100, 3) if nearest_support else None,
    }


# ── Momentum micro-regime ────────────────────────────────────────
def micro_momentum(closes: np.ndarray, n_rsi: int = 14) -> dict:
    """Compute micro RSI and MACD on the 1-min series."""
    if len(closes) < n_rsi + 1:
        return {"rsi": 50.0, "macd_hist": 0.0, "regime": "neutral"}

    deltas = np.diff(closes)
    gains = np.where(deltas > 0, deltas, 0.0)
    losses = np.where(deltas < 0, -deltas, 0.0)

    avg_gain = np.mean(gains[-n_rsi:])
    avg_loss = np.mean(losses[-n_rsi:])
    rs = avg_gain / (avg_loss + 1e-9)
    rsi = 100 - 100 / (1 + rs)

    # Simple MACD: 12-bar EMA - 26-bar EMA
    if len(closes) >= 26:
        ema12 = pd.Series(closes).ewm(span=12).mean().iloc[-1]
        ema26 = pd.Series(closes).ewm(span=26).mean().iloc[-1]
        macd = ema12 - ema26
        signal = pd.Series(closes).ewm(span=12).mean().ewm(span=9).mean().iloc[-1]
        hist = macd - signal
    else:
        hist = 0.0

    if rsi > 70:
        regime = "overbought"
    elif rsi < 30:
        regime = "oversold"
    elif hist > 0:
        regime = "bullish"
    elif hist < 0:
        regime = "bearish"
    else:
        regime = "neutral"

    return {"rsi": round(float(rsi), 2), "macd_hist": round(float(hist), 4), "regime": regime}


# ── Aggregate score ──────────────────────────────────────────────
def compute_pattern_score(df: pd.DataFrame) -> dict:
    """
    Given a DataFrame of 1-min OHLCV candles, compute a combined pattern
    score in [-1.0, +1.0] where +1 is strongly bullish, -1 strongly bearish.
    Also returns component details for the UI.
    """
    closes = df["Close"].values.astype(float)
    highs = df["High"].values.astype(float)
    lows = df["Low"].values.astype(float)

    # 1) Repeating pattern matches
    repeats = find_repeating_patterns(closes, window=10, search_depth=120, top_k=5)
    if repeats:
        avg_next_change = np.mean([m["next_change_pct"] for m in repeats])
        repeat_signal = np.clip(avg_next_change / 0.5, -1, 1)  # normalize
    else:
        avg_next_change = 0.0
        repeat_signal = 0.0

    # 2) Support / Resistance proximity
    sr = detect_micro_sr(highs, lows, closes)
    sr_signal = 0.0
    if sr["resistance_distance_pct"] is not None and sr["resistance_distance_pct"] < 0.1:
        sr_signal = -0.3  # near resistance, bearish bias
    if sr["support_distance_pct"] is not None and sr["support_distance_pct"] < 0.1:
        sr_signal = 0.3  # near support, bullish bias

    # 3) Momentum regime
    mom = micro_momentum(closes)
    mom_signal = 0.0
    if mom["regime"] == "overbought":
        mom_signal = -0.3
    elif mom["regime"] == "oversold":
        mom_signal = 0.3
    elif mom["regime"] == "bullish":
        mom_signal = 0.2
    elif mom["regime"] == "bearish":
        mom_signal = -0.2

    # 4) Recent candle patterns (last 5 bars micro-patterns)
    from app.services.pattern_recognition import detect_all_patterns
    micro_df = df.tail(20).reset_index(drop=True)
    candle_patterns = detect_all_patterns(micro_df)
    candle_signal = 0.0
    if candle_patterns:
        # Weight recent patterns more
        for p in candle_patterns[-5:]:
            if p["type"] == "bullish":
                candle_signal += 0.1 * p.get("confidence", 0.5)
            elif p["type"] == "bearish":
                candle_signal -= 0.1 * p.get("confidence", 0.5)
        candle_signal = np.clip(candle_signal, -1, 1)

    # Weighted combination
    combined = (
        0.40 * repeat_signal +
        0.20 * sr_signal +
        0.20 * mom_signal +
        0.20 * candle_signal
    )
    combined = float(np.clip(combined, -1, 1))

    return {
        "score": round(combined, 4),
        "direction": "bullish" if combined > 0.05 else "bearish" if combined < -0.05 else "neutral",
        "components": {
            "repeat_pattern": {
                "signal": round(float(repeat_signal), 4),
                "matches_found": len(repeats),
                "avg_next_change_pct": round(avg_next_change, 4),
                "matches": repeats,
            },
            "support_resistance": {
                "signal": round(sr_signal, 4),
                **sr,
            },
            "momentum": {
                "signal": round(mom_signal, 4),
                **mom,
            },
            "candle_patterns": {
                "signal": round(float(candle_signal), 4),
                "patterns_detected": [p["name"] for p in candle_patterns[-5:]] if candle_patterns else [],
            },
        },
    }
