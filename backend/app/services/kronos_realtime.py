"""
Kronos Real-time 1-minute predictor.

Fetches the latest 1-minute candle data, runs it through the Kronos
foundation model to predict the next candle, then applies a pattern-based
adjustment from the PatternScorer.
"""

import sys
import os
import numpy as np
import pandas as pd
import logging
from datetime import datetime, timedelta

logger = logging.getLogger(__name__)

KRONOS_REPO_PATH = "/app/kronos_repo"
if os.path.exists(KRONOS_REPO_PATH) and KRONOS_REPO_PATH not in sys.path:
    sys.path.insert(0, KRONOS_REPO_PATH)


def kronos_predict_1min(df: pd.DataFrame, steps: int = 1) -> dict:
    """
    Run Kronos on 1-minute candle data to predict the next `steps` candles.

    Args:
        df: DataFrame with Date, Open, High, Low, Close, Volume (1-min bars)
        steps: how many 1-min candles to predict (1-5)

    Returns:
        dict with raw Kronos predictions (before pattern adjustment)
    """
    from app.services.kronos_predictor import _load_kronos, _predictor, _model_source, MAX_CONTEXT

    if not _load_kronos():
        from app.services.kronos_predictor import _load_error
        return {"error": f"Kronos not available: {_load_error or 'loading'}"}

    if len(df) < 30:
        return {"error": "Need at least 30 one-minute candles"}

    try:
        kronos_df = pd.DataFrame()
        kronos_df["open"] = df["Open"].astype(float)
        kronos_df["high"] = df["High"].astype(float)
        kronos_df["low"] = df["Low"].astype(float)
        kronos_df["close"] = df["Close"].astype(float)
        kronos_df["volume"] = df["Volume"].astype(float) if "Volume" in df.columns else 0.0
        kronos_df["amount"] = kronos_df["volume"] * kronos_df[["open", "high", "low", "close"]].mean(axis=1)

        # Timestamps
        date_series = pd.to_datetime(df["Date"])
        x_timestamp = pd.Series(date_series.values)

        # Future timestamps — 1-min intervals
        last_ts = date_series.iloc[-1]
        future_ts = [last_ts + timedelta(minutes=i + 1) for i in range(steps)]
        y_timestamp = pd.Series(pd.DatetimeIndex(future_ts))

        # Limit lookback
        lookback = min(len(kronos_df), MAX_CONTEXT)
        kronos_df = kronos_df.iloc[-lookback:].reset_index(drop=True)
        x_timestamp = x_timestamp.iloc[-lookback:].reset_index(drop=True)

        pred_df = _predictor.predict(
            df=kronos_df,
            x_timestamp=x_timestamp,
            y_timestamp=y_timestamp,
            pred_len=steps,
            T=0.6,         # lower temp for tight 1-min forecasts
            top_p=0.85,
            top_k=0,
            sample_count=5,  # more samples for stability on noisy 1-min
            verbose=False,
        )

        current_close = float(df["Close"].iloc[-1])
        predictions = []

        for i in range(len(pred_df)):
            row = pred_df.iloc[i]
            pred_close = float(row["close"])
            prev_close = current_close if i == 0 else float(pred_df.iloc[i - 1]["close"])
            change_pct = (pred_close - prev_close) / prev_close * 100

            predictions.append({
                "step": i + 1,
                "timestamp": future_ts[i].isoformat(),
                "predicted_open": round(float(row["open"]), 2),
                "predicted_high": round(float(row["high"]), 2),
                "predicted_low": round(float(row["low"]), 2),
                "predicted_close": round(pred_close, 2),
                "change_percent": round(change_pct, 4),
                "direction": "bullish" if pred_close > prev_close else "bearish",
            })

        final_close = float(pred_df.iloc[-1]["close"])
        total_change = (final_close - current_close) / current_close * 100

        return {
            "current_close": round(current_close, 2),
            "final_predicted_close": round(final_close, 2),
            "total_change_percent": round(total_change, 4),
            "direction": "bullish" if final_close > current_close else "bearish",
            "predictions": predictions,
            "model_source": _model_source,
            "lookback_bars": lookback,
            "parameters": {"temperature": 0.6, "top_p": 0.85, "sample_count": 5},
        }

    except Exception as e:
        logger.error("Kronos 1-min prediction failed: %s", e, exc_info=True)
        return {"error": f"Prediction failed: {str(e)}"}


def kronos_predict_with_patterns(df: pd.DataFrame, steps: int = 1) -> dict:
    """
    Predict next 1-min candle(s) using Kronos + pattern-based adjustment.

    1. Run raw Kronos prediction
    2. Compute pattern score from recent candles
    3. Blend the two: adjust Kronos close by pattern-bias * ATR
    """
    from app.services.pattern_scorer import compute_pattern_score

    # Raw Kronos
    raw = kronos_predict_1min(df, steps=steps)
    if "error" in raw:
        return raw

    # Pattern analysis
    pattern_info = compute_pattern_score(df)

    # ATR for scaling the adjustment
    highs = df["High"].values.astype(float)
    lows = df["Low"].values.astype(float)
    n = min(14, len(highs))
    atr = float(np.mean(np.maximum(highs[-n:] - lows[-n:], 0.01)))

    # Apply pattern adjustment to each predicted candle
    score = pattern_info["score"]
    adjusted_predictions = []
    for p in raw["predictions"]:
        # Shift predicted close by score * fraction_of_ATR
        adjustment = score * atr * 0.3  # 30% of ATR scaled by score
        adj_close = round(p["predicted_close"] + adjustment, 2)
        adj_high = round(max(p["predicted_high"], adj_close + atr * 0.1), 2)
        adj_low = round(min(p["predicted_low"], adj_close - atr * 0.1), 2)

        prev = raw["current_close"] if p["step"] == 1 else adjusted_predictions[-1]["predicted_close"]
        change_pct = round((adj_close - prev) / prev * 100, 4)

        adjusted_predictions.append({
            **p,
            "raw_close": p["predicted_close"],
            "predicted_close": adj_close,
            "predicted_high": adj_high,
            "predicted_low": adj_low,
            "change_percent": change_pct,
            "direction": "bullish" if adj_close > prev else "bearish",
            "pattern_adjustment": round(adjustment, 4),
        })

    final = adjusted_predictions[-1]["predicted_close"]
    total_change = round((final - raw["current_close"]) / raw["current_close"] * 100, 4)

    return {
        **raw,
        "predictions": adjusted_predictions,
        "final_predicted_close": final,
        "total_change_percent": total_change,
        "direction": "bullish" if final > raw["current_close"] else "bearish",
        "pattern_analysis": pattern_info,
        "atr_1min": round(atr, 4),
    }
