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


# ── Fine-tune endpoint (merged from finetune_server.py) ──────────

class FinetuneRequest(BaseModel):
    date: str
    day_pnl: float
    symbols: dict  # { "NVDA": { "candles": [...] }, ... }


@app.post("/finetune")
async def finetune(req: FinetuneRequest):
    """Fine-tune Kronos on today's profitable candle data, then re-export to ONNX."""
    global _tokenizer, _model, _predictor, _model_source

    logger.info("=" * 60)
    logger.info("FINE-TUNE REQUEST — date=%s PnL=$%.4f", req.date, req.day_pnl)
    logger.info("=" * 60)

    if req.day_pnl <= 0:
        return {"success": False, "error": "Day was not profitable"}

    import torch
    import torch.nn.functional as F
    from model import KronosTokenizer, Kronos

    device = "cuda:0" if torch.cuda.is_available() else "cpu"
    CLIP = 5.0
    LOOKBACK = 60
    PRED_LEN = 5
    EPOCHS = 3
    BATCH_SIZE = 4
    LR = 5e-6

    FINETUNE_DIR = "/app/model_cache/finetuned_nvda"
    ft_tok_path = os.path.join(FINETUNE_DIR, "tokenizer")
    ft_pred_path = os.path.join(FINETUNE_DIR, "predictor")

    if os.path.exists(ft_tok_path) and os.path.exists(ft_pred_path):
        tokenizer = KronosTokenizer.from_pretrained(ft_tok_path).to(device)
        model = Kronos.from_pretrained(ft_pred_path).to(device)
        logger.info("Loaded existing fine-tuned model")
    else:
        tokenizer = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base").to(device)
        model = Kronos.from_pretrained("NeoQuasar/Kronos-small").to(device)
        logger.info("Loaded base model for first fine-tune")

    X_all, Y_all, Xs_all, Ys_all = [], [], [], []
    from datetime import datetime as dt

    for sym, sym_data in req.symbols.items():
        candles = sym_data.get("candles", [])
        if len(candles) < LOOKBACK + PRED_LEN:
            continue

        df = pd.DataFrame(candles)
        features = df[["Open", "High", "Low", "Close"]].values.astype(np.float32)
        vol = df["Volume"].values.astype(np.float32) if "Volume" in df.columns else np.ones(len(df), dtype=np.float32)
        amount = vol * features.mean(axis=1)
        features_full = np.column_stack([features, vol, amount])

        if "timestamp" in df.columns:
            ts = pd.to_datetime(df["timestamp"], unit="s")
        else:
            ts = pd.date_range(end=dt.now(), periods=len(df), freq="1min")

        stamps = np.column_stack([ts.minute, ts.hour, ts.weekday, ts.day, ts.month]).astype(np.float32)

        for i in range(0, len(features_full) - LOOKBACK - PRED_LEN, 2):
            x = features_full[i:i + LOOKBACK]
            y = features_full[i + LOOKBACK:i + LOOKBACK + PRED_LEN]
            x_mean, x_std = x.mean(axis=0), x.std(axis=0) + 1e-5
            X_all.append(np.clip((x - x_mean) / x_std, -CLIP, CLIP))
            Y_all.append(np.clip((y - x_mean) / x_std, -CLIP, CLIP))
            Xs_all.append(stamps[i:i + LOOKBACK])
            Ys_all.append(stamps[i + LOOKBACK:i + LOOKBACK + PRED_LEN])

    if len(X_all) < 5:
        return {"success": False, "error": f"Not enough samples ({len(X_all)})"}

    X = np.array(X_all, dtype=np.float32)
    Y = np.array(Y_all, dtype=np.float32)
    Xs = np.array(Xs_all, dtype=np.float32)
    Ys = np.array(Ys_all, dtype=np.float32)

    logger.info("Fine-tuning on %d samples from %d symbols", len(X), len(req.symbols))

    # Stage 1: Tokenizer (1 epoch)
    tokenizer.train()
    opt_tok = torch.optim.AdamW(tokenizer.parameters(), lr=LR * 10, weight_decay=0.01)
    tok_losses = []
    indices = np.random.permutation(len(X))
    for bs in range(0, len(indices), BATCH_SIZE):
        bi = indices[bs:bs + BATCH_SIZE]
        if len(bi) < 2: continue
        x_b = torch.from_numpy(X[bi]).to(device)
        (z_pre, z_full), bsq_loss, _, _ = tokenizer(x_b)
        loss = 0.5 * F.mse_loss(z_full, x_b) + 0.5 * F.mse_loss(z_pre, x_b) + bsq_loss
        opt_tok.zero_grad(); loss.backward()
        torch.nn.utils.clip_grad_norm_(tokenizer.parameters(), 1.0); opt_tok.step()
        tok_losses.append(loss.item())

    avg_tok_loss = float(np.mean(tok_losses)) if tok_losses else 0

    # Stage 2: Predictor (3 epochs)
    model.train(); tokenizer.eval()
    opt_pred = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=0.01)
    pred_losses = []
    for epoch in range(EPOCHS):
        el = []; indices = np.random.permutation(len(X))
        for bs in range(0, len(indices), BATCH_SIZE):
            bi = indices[bs:bs + BATCH_SIZE]
            if len(bi) < 2: continue
            x_b = torch.from_numpy(X[bi]).to(device)
            y_b = torch.from_numpy(Y[bi]).to(device)
            xs_b = torch.from_numpy(Xs[bi]).to(device)
            ys_b = torch.from_numpy(Ys[bi]).to(device)
            full_seq = torch.cat([x_b, y_b], dim=1)
            full_stamp = torch.cat([xs_b, ys_b], dim=1)
            with torch.no_grad():
                z_idx = tokenizer.encode(full_seq, half=True)
                s1, s2 = z_idx[0], z_idx[1]
            s1_logits, s2_logits = model(s1[:, :-1], s2[:, :-1], stamp=full_stamp[:, :-1],
                use_teacher_forcing=True, s1_targets=s1[:, 1:])
            loss, _, _ = model.head.compute_loss(s1_logits, s2_logits, s1[:, 1:], s2[:, 1:])
            opt_pred.zero_grad(); loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0); opt_pred.step()
            el.append(loss.item())
        avg = float(np.mean(el)) if el else 0
        pred_losses.append(avg)
        logger.info("  Predictor epoch %d/%d: loss=%.4f", epoch + 1, EPOCHS, avg)

    # Save & re-export
    os.makedirs(os.path.join(FINETUNE_DIR, "tokenizer"), exist_ok=True)
    os.makedirs(os.path.join(FINETUNE_DIR, "predictor"), exist_ok=True)
    tokenizer.save_pretrained(os.path.join(FINETUNE_DIR, "tokenizer"))
    model.save_pretrained(os.path.join(FINETUNE_DIR, "predictor"))

    # Reload into inference
    _tokenizer = tokenizer; _model = model; _model_source = "finetuned-nvda"
    from model import KronosPredictor
    _predictor = KronosPredictor(_model, _tokenizer, max_context=512)

    try: os.system("python /app/scripts/export_kronos_onnx.py")
    except Exception as e: logger.error("ONNX export failed: %s", e)

    import json as json_mod
    meta = {"date": req.date, "day_pnl": round(req.day_pnl, 4), "training_samples": len(X),
            "tokenizer_loss": avg_tok_loss, "predictor_losses": pred_losses, "success": True}
    with open(os.path.join(FINETUNE_DIR, "metadata.json"), "w") as f:
        json_mod.dump(meta, f, indent=2)

    logger.info("Fine-tune complete: %s", meta)
    return meta


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8001)
