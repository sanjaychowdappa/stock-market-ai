import pandas as pd
import numpy as np
from dataclasses import dataclass


@dataclass
class PatternResult:
    name: str
    type: str  # bullish, bearish, neutral
    confidence: float
    index: int
    description: str


def detect_all_patterns(df: pd.DataFrame) -> list[dict]:
    if len(df) < 10:
        return []

    detectors = [
        detect_doji,
        detect_hammer,
        detect_engulfing,
        detect_morning_star,
        detect_evening_star,
        detect_three_white_soldiers,
        detect_three_black_crows,
        detect_harami,
        detect_shooting_star,
        detect_spinning_top,
        detect_marubozu,
        detect_tweezer,
        detect_head_and_shoulders,
        detect_double_top_bottom,
        detect_ascending_triangle,
    ]

    all_patterns = []
    for detector in detectors:
        patterns = detector(df)
        all_patterns.extend(patterns)

    all_patterns.sort(key=lambda p: p["index"], reverse=True)
    return all_patterns


def _body(o, c):
    return abs(c - o)


def _upper_shadow(h, o, c):
    return h - max(o, c)


def _lower_shadow(l, o, c):
    return min(o, c) - l


def _avg_body(df, end, n=10):
    start = max(0, end - n)
    bodies = [_body(df["Open"].iloc[i], df["Close"].iloc[i]) for i in range(start, end)]
    return np.mean(bodies) if bodies else 0.0001


def detect_doji(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(1, len(df)):
        o, h, l, c = df["Open"].iloc[i], df["High"].iloc[i], df["Low"].iloc[i], df["Close"].iloc[i]
        body = _body(o, c)
        total_range = h - l
        if total_range > 0 and body / total_range < 0.1:
            results.append({
                "name": "Doji",
                "type": "neutral",
                "confidence": round(1 - (body / total_range), 2),
                "index": i,
                "description": "Indecision candle - open and close are nearly equal. Watch for trend reversal.",
            })
    return results


def detect_hammer(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(5, len(df)):
        o, h, l, c = df["Open"].iloc[i], df["High"].iloc[i], df["Low"].iloc[i], df["Close"].iloc[i]
        body = _body(o, c)
        lower = _lower_shadow(l, o, c)
        upper = _upper_shadow(h, o, c)
        avg = _avg_body(df, i)

        if body > 0 and lower >= 2 * body and upper <= body * 0.3:
            is_downtrend = df["Close"].iloc[i - 5] > df["Close"].iloc[i - 1]
            if is_downtrend:
                results.append({
                    "name": "Hammer",
                    "type": "bullish",
                    "confidence": round(min(lower / (2 * body), 1.0), 2),
                    "index": i,
                    "description": "Bullish reversal pattern after downtrend. Long lower shadow shows buying pressure.",
                })
    return results


def detect_engulfing(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(1, len(df)):
        o1, c1 = df["Open"].iloc[i - 1], df["Close"].iloc[i - 1]
        o2, c2 = df["Open"].iloc[i], df["Close"].iloc[i]

        if c1 < o1 and c2 > o2 and o2 <= c1 and c2 >= o1:
            results.append({
                "name": "Bullish Engulfing",
                "type": "bullish",
                "confidence": round(min(_body(o2, c2) / max(_body(o1, c1), 0.001), 1.0), 2),
                "index": i,
                "description": "Strong bullish reversal. Current candle completely engulfs previous bearish candle.",
            })
        elif c1 > o1 and c2 < o2 and o2 >= c1 and c2 <= o1:
            results.append({
                "name": "Bearish Engulfing",
                "type": "bearish",
                "confidence": round(min(_body(o2, c2) / max(_body(o1, c1), 0.001), 1.0), 2),
                "index": i,
                "description": "Strong bearish reversal. Current candle completely engulfs previous bullish candle.",
            })
    return results


def detect_morning_star(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(2, len(df)):
        o1, c1 = df["Open"].iloc[i - 2], df["Close"].iloc[i - 2]
        o2, c2 = df["Open"].iloc[i - 1], df["Close"].iloc[i - 1]
        o3, c3 = df["Open"].iloc[i], df["Close"].iloc[i]

        if (c1 < o1 and _body(o2, c2) < _body(o1, c1) * 0.3
                and c3 > o3 and c3 > (o1 + c1) / 2):
            results.append({
                "name": "Morning Star",
                "type": "bullish",
                "confidence": 0.85,
                "index": i,
                "description": "Three-candle bullish reversal. Gap down, small body, then strong bullish close.",
            })
    return results


def detect_evening_star(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(2, len(df)):
        o1, c1 = df["Open"].iloc[i - 2], df["Close"].iloc[i - 2]
        o2, c2 = df["Open"].iloc[i - 1], df["Close"].iloc[i - 1]
        o3, c3 = df["Open"].iloc[i], df["Close"].iloc[i]

        if (c1 > o1 and _body(o2, c2) < _body(o1, c1) * 0.3
                and c3 < o3 and c3 < (o1 + c1) / 2):
            results.append({
                "name": "Evening Star",
                "type": "bearish",
                "confidence": 0.85,
                "index": i,
                "description": "Three-candle bearish reversal. Gap up, small body, then strong bearish close.",
            })
    return results


def detect_three_white_soldiers(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(2, len(df)):
        candles = [(df["Open"].iloc[i - j], df["Close"].iloc[i - j]) for j in range(2, -1, -1)]
        if all(c > o for o, c in candles):
            if candles[1][1] > candles[0][1] and candles[2][1] > candles[1][1]:
                if candles[1][0] > candles[0][0] and candles[2][0] > candles[1][0]:
                    results.append({
                        "name": "Three White Soldiers",
                        "type": "bullish",
                        "confidence": 0.9,
                        "index": i,
                        "description": "Three consecutive bullish candles with higher opens and closes. Strong uptrend signal.",
                    })
    return results


def detect_three_black_crows(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(2, len(df)):
        candles = [(df["Open"].iloc[i - j], df["Close"].iloc[i - j]) for j in range(2, -1, -1)]
        if all(c < o for o, c in candles):
            if candles[1][1] < candles[0][1] and candles[2][1] < candles[1][1]:
                if candles[1][0] < candles[0][0] and candles[2][0] < candles[1][0]:
                    results.append({
                        "name": "Three Black Crows",
                        "type": "bearish",
                        "confidence": 0.9,
                        "index": i,
                        "description": "Three consecutive bearish candles with lower opens and closes. Strong downtrend signal.",
                    })
    return results


def detect_harami(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(1, len(df)):
        o1, c1 = df["Open"].iloc[i - 1], df["Close"].iloc[i - 1]
        o2, c2 = df["Open"].iloc[i], df["Close"].iloc[i]

        if _body(o1, c1) > _body(o2, c2) * 2:
            if min(o2, c2) > min(o1, c1) and max(o2, c2) < max(o1, c1):
                pattern_type = "bullish" if c1 < o1 else "bearish"
                results.append({
                    "name": f"{pattern_type.title()} Harami",
                    "type": pattern_type,
                    "confidence": 0.7,
                    "index": i,
                    "description": f"{pattern_type.title()} reversal - small candle contained within previous large candle.",
                })
    return results


def detect_shooting_star(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(5, len(df)):
        o, h, l, c = df["Open"].iloc[i], df["High"].iloc[i], df["Low"].iloc[i], df["Close"].iloc[i]
        body = _body(o, c)
        upper = _upper_shadow(h, o, c)
        lower = _lower_shadow(l, o, c)

        if body > 0 and upper >= 2 * body and lower <= body * 0.3:
            is_uptrend = df["Close"].iloc[i - 1] > df["Close"].iloc[i - 5]
            if is_uptrend:
                results.append({
                    "name": "Shooting Star",
                    "type": "bearish",
                    "confidence": 0.75,
                    "index": i,
                    "description": "Bearish reversal at top of uptrend. Long upper shadow shows selling pressure.",
                })
    return results


def detect_spinning_top(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(1, len(df)):
        o, h, l, c = df["Open"].iloc[i], df["High"].iloc[i], df["Low"].iloc[i], df["Close"].iloc[i]
        body = _body(o, c)
        upper = _upper_shadow(h, o, c)
        lower = _lower_shadow(l, o, c)
        avg = _avg_body(df, i)

        if body < avg * 0.5 and upper > body and lower > body:
            results.append({
                "name": "Spinning Top",
                "type": "neutral",
                "confidence": 0.6,
                "index": i,
                "description": "Small body with long shadows. Market indecision - potential trend change.",
            })
    return results


def detect_marubozu(df: pd.DataFrame) -> list[dict]:
    results = []
    for i in range(len(df)):
        o, h, l, c = df["Open"].iloc[i], df["High"].iloc[i], df["Low"].iloc[i], df["Close"].iloc[i]
        body = _body(o, c)
        total = h - l
        if total > 0 and body / total > 0.9:
            pattern_type = "bullish" if c > o else "bearish"
            results.append({
                "name": f"{pattern_type.title()} Marubozu",
                "type": pattern_type,
                "confidence": round(body / total, 2),
                "index": i,
                "description": f"Strong {pattern_type} candle with no shadows. High conviction move.",
            })
    return results


def detect_tweezer(df: pd.DataFrame) -> list[dict]:
    results = []
    tolerance = 0.001
    for i in range(1, len(df)):
        h1, l1 = df["High"].iloc[i - 1], df["Low"].iloc[i - 1]
        h2, l2 = df["High"].iloc[i], df["Low"].iloc[i]
        o1, c1 = df["Open"].iloc[i - 1], df["Close"].iloc[i - 1]
        o2, c2 = df["Open"].iloc[i], df["Close"].iloc[i]

        if abs(h1 - h2) / max(h1, 0.01) < tolerance and c1 > o1 and c2 < o2:
            results.append({
                "name": "Tweezer Top",
                "type": "bearish",
                "confidence": 0.75,
                "index": i,
                "description": "Equal highs signal resistance. Bearish reversal pattern.",
            })
        if abs(l1 - l2) / max(l1, 0.01) < tolerance and c1 < o1 and c2 > o2:
            results.append({
                "name": "Tweezer Bottom",
                "type": "bullish",
                "confidence": 0.75,
                "index": i,
                "description": "Equal lows signal support. Bullish reversal pattern.",
            })
    return results


def detect_head_and_shoulders(df: pd.DataFrame) -> list[dict]:
    results = []
    if len(df) < 30:
        return results

    highs = df["High"].values
    for i in range(20, len(df)):
        window = highs[i - 20:i + 1]
        peaks = []
        for j in range(2, len(window) - 2):
            if window[j] > window[j - 1] and window[j] > window[j - 2] and window[j] > window[j + 1] and window[j] > window[j + 2]:
                peaks.append((j, window[j]))

        if len(peaks) >= 3:
            for k in range(len(peaks) - 2):
                left, head, right = peaks[k], peaks[k + 1], peaks[k + 2]
                if head[1] > left[1] and head[1] > right[1]:
                    if abs(left[1] - right[1]) / head[1] < 0.03:
                        results.append({
                            "name": "Head and Shoulders",
                            "type": "bearish",
                            "confidence": 0.8,
                            "index": i,
                            "description": "Classic reversal pattern. Head higher than both shoulders signals trend reversal.",
                        })
                        break
    return results


def detect_double_top_bottom(df: pd.DataFrame) -> list[dict]:
    results = []
    if len(df) < 20:
        return results

    highs = df["High"].values
    lows = df["Low"].values

    for i in range(15, len(df)):
        h_window = highs[i - 15:i + 1]
        l_window = lows[i - 15:i + 1]

        h_peaks = []
        l_troughs = []
        for j in range(2, len(h_window) - 2):
            if h_window[j] > h_window[j - 1] and h_window[j] > h_window[j + 1]:
                h_peaks.append((j, h_window[j]))
            if l_window[j] < l_window[j - 1] and l_window[j] < l_window[j + 1]:
                l_troughs.append((j, l_window[j]))

        if len(h_peaks) >= 2:
            p1, p2 = h_peaks[-2], h_peaks[-1]
            if abs(p1[1] - p2[1]) / max(p1[1], 0.01) < 0.02 and abs(p1[0] - p2[0]) >= 5:
                results.append({
                    "name": "Double Top",
                    "type": "bearish",
                    "confidence": 0.8,
                    "index": i,
                    "description": "Two peaks at similar levels. Resistance confirmed - bearish reversal likely.",
                })

        if len(l_troughs) >= 2:
            t1, t2 = l_troughs[-2], l_troughs[-1]
            if abs(t1[1] - t2[1]) / max(t1[1], 0.01) < 0.02 and abs(t1[0] - t2[0]) >= 5:
                results.append({
                    "name": "Double Bottom",
                    "type": "bullish",
                    "confidence": 0.8,
                    "index": i,
                    "description": "Two troughs at similar levels. Support confirmed - bullish reversal likely.",
                })
    return results


def detect_ascending_triangle(df: pd.DataFrame) -> list[dict]:
    results = []
    if len(df) < 20:
        return results

    for i in range(20, len(df)):
        window_h = df["High"].iloc[i - 15:i + 1].values
        window_l = df["Low"].iloc[i - 15:i + 1].values

        high_std = np.std(window_h[-5:])
        low_slope = np.polyfit(range(len(window_l)), window_l, 1)[0]

        if high_std < np.mean(window_h) * 0.01 and low_slope > 0:
            results.append({
                "name": "Ascending Triangle",
                "type": "bullish",
                "confidence": 0.75,
                "index": i,
                "description": "Flat resistance with rising lows. Bullish breakout pattern.",
            })
    return results
