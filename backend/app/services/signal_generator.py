import pandas as pd
import numpy as np


def generate_signals(df: pd.DataFrame) -> dict:
    if len(df) < 50:
        return {"signal": "hold", "confidence": 0, "reasons": ["Insufficient data"]}

    signals = []
    weights = []

    rsi_signal, rsi_weight = _rsi_signal(df)
    signals.append(rsi_signal)
    weights.append(rsi_weight)

    macd_signal, macd_weight = _macd_signal(df)
    signals.append(macd_signal)
    weights.append(macd_weight)

    bb_signal, bb_weight = _bollinger_signal(df)
    signals.append(bb_signal)
    weights.append(bb_weight)

    sma_signal, sma_weight = _sma_crossover_signal(df)
    signals.append(sma_signal)
    weights.append(sma_weight)

    stoch_signal, stoch_weight = _stochastic_signal(df)
    signals.append(stoch_signal)
    weights.append(stoch_weight)

    vol_signal, vol_weight = _volume_signal(df)
    signals.append(vol_signal)
    weights.append(vol_weight)

    adx_signal, adx_weight = _adx_signal(df)
    signals.append(adx_signal)
    weights.append(adx_weight)

    weighted_score = sum(s["score"] * w for s, w in zip(signals, weights)) / sum(weights)

    reasons = [s["reason"] for s in signals if s["reason"]]

    if weighted_score > 0.3:
        action = "BUY"
    elif weighted_score < -0.3:
        action = "SELL"
    else:
        action = "HOLD"

    stop_loss = _calculate_stop_loss(df, action)
    take_profit = _calculate_take_profit(df, action)

    return {
        "signal": action,
        "score": round(weighted_score, 3),
        "confidence": round(abs(weighted_score), 2),
        "reasons": reasons,
        "individual_signals": [
            {**s, "weight": w} for s, w in zip(signals, weights)
        ],
        "stop_loss": stop_loss,
        "take_profit": take_profit,
        "current_price": round(float(df["Close"].iloc[-1]), 2),
    }


def _rsi_signal(df: pd.DataFrame) -> tuple:
    rsi = df["RSI"].iloc[-1]
    if rsi < 30:
        return {"name": "RSI", "score": 0.8, "reason": f"RSI oversold at {rsi:.1f} - bullish"}, 1.2
    elif rsi > 70:
        return {"name": "RSI", "score": -0.8, "reason": f"RSI overbought at {rsi:.1f} - bearish"}, 1.2
    elif rsi < 40:
        return {"name": "RSI", "score": 0.3, "reason": f"RSI approaching oversold at {rsi:.1f}"}, 0.8
    elif rsi > 60:
        return {"name": "RSI", "score": -0.3, "reason": f"RSI approaching overbought at {rsi:.1f}"}, 0.8
    return {"name": "RSI", "score": 0, "reason": ""}, 0.5


def _macd_signal(df: pd.DataFrame) -> tuple:
    macd = df["MACD"].iloc[-1]
    signal = df["MACD_Signal"].iloc[-1]
    hist = df["MACD_Hist"].iloc[-1]
    prev_hist = df["MACD_Hist"].iloc[-2]

    if hist > 0 and prev_hist <= 0:
        return {"name": "MACD", "score": 0.9, "reason": "MACD bullish crossover"}, 1.5
    elif hist < 0 and prev_hist >= 0:
        return {"name": "MACD", "score": -0.9, "reason": "MACD bearish crossover"}, 1.5
    elif hist > 0:
        return {"name": "MACD", "score": 0.4, "reason": "MACD histogram positive"}, 1.0
    elif hist < 0:
        return {"name": "MACD", "score": -0.4, "reason": "MACD histogram negative"}, 1.0
    return {"name": "MACD", "score": 0, "reason": ""}, 0.5


def _bollinger_signal(df: pd.DataFrame) -> tuple:
    close = df["Close"].iloc[-1]
    upper = df["BB_Upper"].iloc[-1]
    lower = df["BB_Lower"].iloc[-1]
    mid = df["BB_Mid"].iloc[-1]

    bb_width = (upper - lower) / mid if mid else 0

    if close <= lower:
        return {"name": "Bollinger", "score": 0.7, "reason": "Price at lower Bollinger Band - potential bounce"}, 1.0
    elif close >= upper:
        return {"name": "Bollinger", "score": -0.7, "reason": "Price at upper Bollinger Band - potential pullback"}, 1.0
    elif bb_width < 0.04:
        return {"name": "Bollinger", "score": 0, "reason": "Bollinger squeeze - breakout imminent"}, 0.5
    return {"name": "Bollinger", "score": 0, "reason": ""}, 0.5


def _sma_crossover_signal(df: pd.DataFrame) -> tuple:
    sma20 = df["SMA_20"].iloc[-1]
    sma50 = df["SMA_50"].iloc[-1]
    prev_sma20 = df["SMA_20"].iloc[-2]
    prev_sma50 = df["SMA_50"].iloc[-2]

    if prev_sma20 <= prev_sma50 and sma20 > sma50:
        return {"name": "SMA Crossover", "score": 0.85, "reason": "Golden cross: SMA20 crossed above SMA50"}, 1.3
    elif prev_sma20 >= prev_sma50 and sma20 < sma50:
        return {"name": "SMA Crossover", "score": -0.85, "reason": "Death cross: SMA20 crossed below SMA50"}, 1.3
    elif sma20 > sma50:
        return {"name": "SMA Crossover", "score": 0.3, "reason": "SMA20 above SMA50 - uptrend"}, 0.8
    else:
        return {"name": "SMA Crossover", "score": -0.3, "reason": "SMA20 below SMA50 - downtrend"}, 0.8


def _stochastic_signal(df: pd.DataFrame) -> tuple:
    k = df["Stoch_K"].iloc[-1]
    d = df["Stoch_D"].iloc[-1]

    if k < 20 and d < 20:
        return {"name": "Stochastic", "score": 0.7, "reason": f"Stochastic oversold (K={k:.1f}, D={d:.1f})"}, 1.0
    elif k > 80 and d > 80:
        return {"name": "Stochastic", "score": -0.7, "reason": f"Stochastic overbought (K={k:.1f}, D={d:.1f})"}, 1.0
    return {"name": "Stochastic", "score": 0, "reason": ""}, 0.5


def _volume_signal(df: pd.DataFrame) -> tuple:
    avg_vol = df["Volume"].iloc[-20:].mean()
    curr_vol = df["Volume"].iloc[-1]
    price_change = df["Close"].iloc[-1] - df["Close"].iloc[-2]

    if curr_vol > avg_vol * 1.5 and price_change > 0:
        return {"name": "Volume", "score": 0.6, "reason": "High volume on price increase - bullish"}, 0.8
    elif curr_vol > avg_vol * 1.5 and price_change < 0:
        return {"name": "Volume", "score": -0.6, "reason": "High volume on price decrease - bearish"}, 0.8
    return {"name": "Volume", "score": 0, "reason": ""}, 0.3


def _adx_signal(df: pd.DataFrame) -> tuple:
    adx = df["ADX"].iloc[-1]
    if adx > 25:
        return {"name": "ADX", "score": 0, "reason": f"Strong trend (ADX={adx:.1f})"}, 1.0
    return {"name": "ADX", "score": 0, "reason": f"Weak trend (ADX={adx:.1f})"}, 0.5


def _calculate_stop_loss(df: pd.DataFrame, action: str) -> dict:
    close = float(df["Close"].iloc[-1])
    atr = float(df["ATR"].iloc[-1]) if "ATR" in df.columns else close * 0.02

    swing_low = float(df["Low"].iloc[-10:].min())
    swing_high = float(df["High"].iloc[-10:].max())

    if action == "BUY":
        atr_stop = close - 2 * atr
        swing_stop = swing_low
        bb_stop = float(df["BB_Lower"].iloc[-1]) if "BB_Lower" in df.columns else close * 0.97
        recommended = max(atr_stop, swing_stop, bb_stop)
        risk_percent = (close - recommended) / close * 100
    elif action == "SELL":
        atr_stop = close + 2 * atr
        swing_stop = swing_high
        bb_stop = float(df["BB_Upper"].iloc[-1]) if "BB_Upper" in df.columns else close * 1.03
        recommended = min(atr_stop, swing_stop, bb_stop)
        risk_percent = (recommended - close) / close * 100
    else:
        return {"recommended": None, "risk_percent": 0, "methods": {}}

    return {
        "recommended": round(recommended, 2),
        "risk_percent": round(risk_percent, 2),
        "methods": {
            "atr_based": round(atr_stop, 2),
            "swing_based": round(swing_stop if action == "BUY" else swing_high, 2),
            "bollinger_based": round(bb_stop, 2),
        },
    }


def _calculate_take_profit(df: pd.DataFrame, action: str) -> dict:
    close = float(df["Close"].iloc[-1])
    atr = float(df["ATR"].iloc[-1]) if "ATR" in df.columns else close * 0.02

    if action == "BUY":
        tp1 = close + 1.5 * atr
        tp2 = close + 2.5 * atr
        tp3 = close + 4 * atr
    elif action == "SELL":
        tp1 = close - 1.5 * atr
        tp2 = close - 2.5 * atr
        tp3 = close - 4 * atr
    else:
        return {"targets": []}

    return {
        "targets": [
            {"level": 1, "price": round(tp1, 2), "risk_reward": "1:1.5"},
            {"level": 2, "price": round(tp2, 2), "risk_reward": "1:2.5"},
            {"level": 3, "price": round(tp3, 2), "risk_reward": "1:4"},
        ]
    }


def generate_stop_loss_report(df: pd.DataFrame, entry_price: float = None, position_type: str = "long") -> dict:
    if entry_price is None:
        entry_price = float(df["Close"].iloc[-1])

    close = float(df["Close"].iloc[-1])
    atr = float(df["ATR"].iloc[-1]) if "ATR" in df.columns else close * 0.02

    support_levels = _find_support_resistance(df, "support")
    resistance_levels = _find_support_resistance(df, "resistance")

    if position_type == "long":
        stops = {
            "tight": round(entry_price - 1 * atr, 2),
            "normal": round(entry_price - 2 * atr, 2),
            "wide": round(entry_price - 3 * atr, 2),
            "swing_low": round(float(df["Low"].iloc[-10:].min()), 2),
            "percent_2": round(entry_price * 0.98, 2),
            "percent_5": round(entry_price * 0.95, 2),
        }
        trailing_stop = round(close - 2 * atr, 2)
    else:
        stops = {
            "tight": round(entry_price + 1 * atr, 2),
            "normal": round(entry_price + 2 * atr, 2),
            "wide": round(entry_price + 3 * atr, 2),
            "swing_high": round(float(df["High"].iloc[-10:].max()), 2),
            "percent_2": round(entry_price * 1.02, 2),
            "percent_5": round(entry_price * 1.05, 2),
        }
        trailing_stop = round(close + 2 * atr, 2)

    return {
        "entry_price": round(entry_price, 2),
        "current_price": round(close, 2),
        "position_type": position_type,
        "atr": round(atr, 2),
        "stop_levels": stops,
        "trailing_stop": trailing_stop,
        "support_levels": support_levels[:3],
        "resistance_levels": resistance_levels[:3],
        "risk_per_share": {
            level: round(abs(entry_price - price), 2) for level, price in stops.items()
        },
        "recommendation": _recommend_stop(stops, entry_price, atr, position_type),
    }


def _find_support_resistance(df: pd.DataFrame, level_type: str) -> list[float]:
    prices = df["Low"].values if level_type == "support" else df["High"].values
    levels = []

    for i in range(2, len(prices) - 2):
        if level_type == "support":
            if prices[i] < prices[i - 1] and prices[i] < prices[i + 1]:
                levels.append(round(float(prices[i]), 2))
        else:
            if prices[i] > prices[i - 1] and prices[i] > prices[i + 1]:
                levels.append(round(float(prices[i]), 2))

    levels = list(set(levels))
    levels.sort(reverse=(level_type == "resistance"))
    return levels


def _recommend_stop(stops: dict, entry: float, atr: float, position_type: str) -> str:
    risk_ratio = atr / entry
    if risk_ratio < 0.01:
        return f"Low volatility stock. Use tight stop at {stops['tight']} (1x ATR) for day trades, normal stop at {stops['normal']} (2x ATR) for swing trades."
    elif risk_ratio < 0.03:
        return f"Moderate volatility. Recommended stop at {stops['normal']} (2x ATR). Swing low/high stop at {stops.get('swing_low', stops.get('swing_high'))} provides structural support."
    else:
        return f"High volatility stock. Use wide stop at {stops['wide']} (3x ATR) to avoid noise. Consider reducing position size."
