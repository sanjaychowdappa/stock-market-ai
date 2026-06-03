"""
Kronos Prediction Sidecar — runs real Kronos transformer predictions.
The Rust backend calls this over HTTP to get actual AI predictions
instead of pattern-only fallback.

Endpoints:
  POST /predict  — predict next candles from historical data
  GET  /health   — check if model is loaded
"""

import os
import sys
import logging
import numpy as np
import pandas as pd
from fastapi import FastAPI
from pydantic import BaseModel
from typing import List, Optional

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("kronos-sidecar")

# Add Kronos repo to path
sys.path.insert(0, "/app/kronos_repo")

app = FastAPI(title="Kronos Prediction Sidecar")

# Global model state
_tokenizer = None
_model = None
_predictor = None
_model_source = "none"


class CandleInput(BaseModel):
    open: float
    high: float
    low: float
    close: float
    volume: float
    time: float


class PredictRequest(BaseModel):
    symbol: str
    candles: List[CandleInput]
    steps: int = 5


def load_model():
    global _tokenizer, _model, _predictor, _model_source
    try:
        import torch
        from model import KronosTokenizer, Kronos, KronosPredictor

        device = "cuda:0" if torch.cuda.is_available() else "cpu"
        logger.info(f"Loading Kronos on {device}...")

        ft_tok = "/app/model_cache/finetuned_nvda/tokenizer"
        ft_pred = "/app/model_cache/finetuned_nvda/predictor"

        if os.path.exists(ft_tok) and os.path.exists(ft_pred):
            _tokenizer = KronosTokenizer.from_pretrained(ft_tok).to(device)
            _model = Kronos.from_pretrained(ft_pred).to(device)
            _model_source = "finetuned-nvda"
            logger.info("Loaded fine-tuned Kronos model")
        else:
            _tokenizer = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base").to(device)
            _model = Kronos.from_pretrained("NeoQuasar/Kronos-small").to(device)
            _model_source = "base"
            logger.info("Loaded base Kronos model (24.7M params)")

        _predictor = KronosPredictor(_model, _tokenizer, max_context=512)
        logger.info(f"Kronos ready — source={_model_source}, device={device}")
        return True
    except Exception as e:
        logger.error(f"Failed to load Kronos: {e}")
        return False


@app.on_event("startup")
async def startup():
    load_model()


@app.get("/health")
def health():
    return {
        "status": "ready" if _predictor else "not_loaded",
        "model_source": _model_source,
    }


@app.post("/predict")
def predict(req: PredictRequest):
    if not _predictor:
        return {"error": "Model not loaded"}

    if len(req.candles) < 30:
        return {"error": f"Need ≥30 candles, got {len(req.candles)}"}

    try:
        # Build DataFrame
        df = pd.DataFrame([{
            "open": c.open, "high": c.high, "low": c.low,
            "close": c.close, "volume": c.volume,
            "amount": c.volume * (c.open + c.high + c.low + c.close) / 4.0,
        } for c in req.candles])

        # Timestamps — use candle times as seconds, convert to datetime
        times = [c.time for c in req.candles]
        x_timestamp = pd.Series(pd.to_datetime(times, unit='s'))

        # Future timestamps — spaced 60s apart (1-min candles)
        last_ts = times[-1]
        y_times = [last_ts + 60 * (i + 1) for i in range(req.steps)]
        y_timestamp = pd.Series(pd.to_datetime(y_times, unit='s'))

        # Limit lookback
        lookback = min(len(df), 512)
        df = df.iloc[-lookback:].reset_index(drop=True)
        x_timestamp = x_timestamp.iloc[-lookback:].reset_index(drop=True)

        # Run prediction
        pred_df = _predictor.predict(
            df=df,
            x_timestamp=x_timestamp,
            y_timestamp=y_timestamp,
            pred_len=req.steps,
            T=0.8,
            top_p=0.9,
            top_k=0,
            sample_count=3,
            verbose=False,
        )

        current_close = float(req.candles[-1].close)
        predictions = []
        for i in range(len(pred_df)):
            row = pred_df.iloc[i]
            pred_close = float(row["close"])
            prev = current_close if i == 0 else float(pred_df.iloc[i-1]["close"])
            change_pct = (pred_close - prev) / prev * 100.0

            predictions.append({
                "step": i + 1,
                "open": round(float(row["open"]), 4),
                "high": round(float(row["high"]), 4),
                "low": round(float(row["low"]), 4),
                "close": round(pred_close, 4),
                "change_pct": round(change_pct, 4),
                "direction": "bullish" if pred_close > prev else "bearish",
            })

        final = float(pred_df.iloc[-1]["close"])
        return {
            "symbol": req.symbol,
            "model_source": _model_source,
            "current_close": round(current_close, 4),
            "final_predicted_close": round(final, 4),
            "total_change_pct": round((final - current_close) / current_close * 100.0, 4),
            "direction": "bullish" if final > current_close else "bearish",
            "predictions": predictions,
        }
    except Exception as e:
        logger.error(f"Prediction failed: {e}", exc_info=True)
        return {"error": str(e)}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8001)
