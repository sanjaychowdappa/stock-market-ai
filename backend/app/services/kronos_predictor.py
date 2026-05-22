"""
Kronos Foundation Model integration for stock prediction.
Uses the Kronos pre-trained model (decoder-only transformer) for
financial candlestick forecasting with hierarchical tokenization.
"""

import sys
import os
import numpy as np
import pandas as pd
import logging
from datetime import datetime, timedelta

logger = logging.getLogger(__name__)

# Add Kronos repo to path (cloned during Docker build)
KRONOS_REPO_PATH = "/app/kronos_repo"
if os.path.exists(KRONOS_REPO_PATH):
    sys.path.insert(0, KRONOS_REPO_PATH)

# Lazy-loaded globals
_tokenizer = None
_model = None
_predictor = None
_load_error = None
_loading = False

# Model configuration
KRONOS_TOKENIZER_ID = "NeoQuasar/Kronos-Tokenizer-base"
KRONOS_MODEL_ID = "NeoQuasar/Kronos-small"  # 24.7M params, good for CPU
MAX_CONTEXT = 512


def _load_kronos():
    """Lazy-load Kronos model on first prediction request."""
    global _tokenizer, _model, _predictor, _load_error, _loading

    if _predictor is not None:
        return True
    if _load_error is not None:
        return False
    if _loading:
        return False

    _loading = True
    try:
        import torch
        from model import KronosTokenizer, Kronos, KronosPredictor

        logger.info("Loading Kronos tokenizer from %s...", KRONOS_TOKENIZER_ID)
        _tokenizer = KronosTokenizer.from_pretrained(KRONOS_TOKENIZER_ID)

        logger.info("Loading Kronos model from %s...", KRONOS_MODEL_ID)
        _model = Kronos.from_pretrained(KRONOS_MODEL_ID)

        _predictor = KronosPredictor(_model, _tokenizer, max_context=MAX_CONTEXT)
        logger.info("Kronos model loaded successfully (device: %s)", _predictor.device)
        _loading = False
        return True
    except Exception as e:
        _load_error = str(e)
        _loading = False
        logger.error("Failed to load Kronos model: %s", e)
        return False


def _generate_business_days(start_date: datetime, num_days: int) -> pd.DatetimeIndex:
    """Generate future business day timestamps."""
    dates = []
    current = start_date
    while len(dates) < num_days:
        current += timedelta(days=1)
        if current.weekday() < 5:  # Mon-Fri
            dates.append(current)
    return pd.DatetimeIndex(dates)


def kronos_predict(df: pd.DataFrame, steps: int = 7) -> dict:
    """
    Run Kronos foundation model prediction on stock data.

    Args:
        df: DataFrame with columns Date, Open, High, Low, Close, Volume
        steps: Number of future candles to predict (default 7)

    Returns:
        dict with predictions list and metadata
    """
    if not _load_kronos():
        return {"error": f"Kronos model not available: {_load_error or 'still loading'}"}

    if len(df) < 30:
        return {"error": "Not enough data for Kronos prediction (need ≥30 rows)"}

    try:
        # Prepare data in Kronos format (lowercase columns)
        kronos_df = pd.DataFrame()
        kronos_df["open"] = df["Open"].astype(float)
        kronos_df["high"] = df["High"].astype(float)
        kronos_df["low"] = df["Low"].astype(float)
        kronos_df["close"] = df["Close"].astype(float)

        if "Volume" in df.columns:
            kronos_df["volume"] = df["Volume"].astype(float)
        else:
            kronos_df["volume"] = 0.0
        # Amount = price * volume approximation
        kronos_df["amount"] = kronos_df["volume"] * kronos_df[["open", "high", "low", "close"]].mean(axis=1)

        # Parse timestamps — Kronos calc_time_stamps expects .dt accessor, so use Series not DatetimeIndex
        date_series = pd.to_datetime(df["Date"])
        x_timestamp = pd.Series(date_series.values)

        # Generate future business day timestamps
        last_date = date_series.iloc[-1]
        y_dates = _generate_business_days(last_date, steps)
        y_timestamp = pd.Series(y_dates)

        # Limit lookback to max_context
        lookback = min(len(kronos_df), MAX_CONTEXT)
        kronos_df = kronos_df.iloc[-lookback:].reset_index(drop=True)
        x_timestamp = x_timestamp.iloc[-lookback:].reset_index(drop=True)

        # Run prediction
        pred_df = _predictor.predict(
            df=kronos_df,
            x_timestamp=x_timestamp,
            y_timestamp=y_timestamp,
            pred_len=steps,
            T=0.8,        # slightly lower temperature for more focused predictions
            top_p=0.9,     # nucleus sampling
            top_k=0,
            sample_count=3,  # average 3 samples for stability
            verbose=False,
        )

        # Build response
        current_close = float(df["Close"].iloc[-1])
        predictions = []

        for i in range(len(pred_df)):
            row = pred_df.iloc[i]
            pred_close = float(row["close"])
            prev_close = current_close if i == 0 else float(pred_df.iloc[i - 1]["close"])
            change_pct = round((pred_close - prev_close) / prev_close * 100, 2)

            predictions.append({
                "step": i + 1,
                "date": str(pred_df.index[i].date()) if hasattr(pred_df.index[i], "date") else str(pred_df.index[i]),
                "predicted_open": round(float(row["open"]), 2),
                "predicted_high": round(float(row["high"]), 2),
                "predicted_low": round(float(row["low"]), 2),
                "predicted_close": round(pred_close, 2),
                "change_percent": change_pct,
                "direction": "bullish" if pred_close > prev_close else "bearish",
            })

        # Overall summary
        final_close = float(pred_df.iloc[-1]["close"])
        total_change = round((final_close - current_close) / current_close * 100, 2)

        return {
            "model": "Kronos-small (24.7M params)",
            "model_type": "Foundation Model — Decoder-only Transformer",
            "pre_training": "12B K-lines from 45+ global exchanges",
            "current_close": round(current_close, 2),
            "final_predicted_close": round(final_close, 2),
            "total_change_percent": total_change,
            "direction": "bullish" if final_close > current_close else "bearish",
            "predictions": predictions,
            "parameters": {
                "temperature": 0.8,
                "top_p": 0.9,
                "sample_count": 3,
                "lookback": lookback,
            },
        }

    except Exception as e:
        logger.error("Kronos prediction failed: %s", e, exc_info=True)
        return {"error": f"Kronos prediction failed: {str(e)}"}


def kronos_status() -> dict:
    """Check Kronos model loading status."""
    if _predictor is not None:
        return {"status": "ready", "model": KRONOS_MODEL_ID, "device": str(_predictor.device)}
    if _loading:
        return {"status": "loading", "model": KRONOS_MODEL_ID}
    if _load_error:
        return {"status": "error", "error": _load_error}
    return {"status": "not_loaded", "model": KRONOS_MODEL_ID}
