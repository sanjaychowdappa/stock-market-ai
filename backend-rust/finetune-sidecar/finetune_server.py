"""
Fine-tune sidecar — lightweight HTTP server that accepts candle data
from the Rust backend and runs Kronos fine-tuning on GPU.

Only wakes up at EOD when the Rust daily tracker decides to fine-tune.
After fine-tuning, re-exports the model to ONNX for Rust to reload.
"""

import os
import sys
import json
import logging
import numpy as np
import pandas as pd
from datetime import datetime
from fastapi import FastAPI
from pydantic import BaseModel

sys.path.insert(0, "/app/kronos_repo")

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger(__name__)

FINETUNE_DIR = "/app/model_cache/finetuned_nvda"
ONNX_DIR = "/app/model_cache/onnx"

app = FastAPI(title="Kronos Fine-Tune Sidecar")


class FinetuneRequest(BaseModel):
    date: str
    day_pnl: float
    symbols: dict  # { "NVDA": { "candles": [...] }, ... }


@app.get("/health")
def health():
    return {"status": "ok", "service": "finetune-sidecar"}


@app.post("/finetune")
async def finetune(req: FinetuneRequest):
    """
    Fine-tune Kronos on today's profitable candle data.
    Then re-export to ONNX for the Rust backend.
    """
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

    # Load current model
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

    # Prepare training samples from all symbols
    X_all, Y_all, Xs_all, Ys_all = [], [], [], []

    for sym, sym_data in req.symbols.items():
        candles = sym_data.get("candles", [])
        if len(candles) < LOOKBACK + PRED_LEN:
            continue

        df = pd.DataFrame(candles)
        features = df[["Open", "High", "Low", "Close"]].values.astype(np.float32)
        vol = df["Volume"].values.astype(np.float32) if "Volume" in df.columns else np.ones(len(df), dtype=np.float32)
        amount = vol * features.mean(axis=1)
        features_full = np.column_stack([features, vol, amount])

        # Timestamps
        if "timestamp" in df.columns:
            ts = pd.to_datetime(df["timestamp"], unit="s")
        else:
            ts = pd.date_range(end=datetime.now(), periods=len(df), freq="1min")

        stamps = np.column_stack([
            ts.minute,
            ts.hour,
            ts.weekday,
            ts.day,
            ts.month,
        ]).astype(np.float32)

        # Sliding window samples
        for i in range(0, len(features_full) - LOOKBACK - PRED_LEN, 2):
            x = features_full[i:i + LOOKBACK]
            y = features_full[i + LOOKBACK:i + LOOKBACK + PRED_LEN]

            x_mean = x.mean(axis=0)
            x_std = x.std(axis=0) + 1e-5
            x_norm = np.clip((x - x_mean) / x_std, -CLIP, CLIP)
            y_norm = np.clip((y - x_mean) / x_std, -CLIP, CLIP)

            X_all.append(x_norm)
            Y_all.append(y_norm)
            Xs_all.append(stamps[i:i + LOOKBACK])
            Ys_all.append(stamps[i + LOOKBACK:i + LOOKBACK + PRED_LEN])

    if len(X_all) < 5:
        return {"success": False, "error": f"Not enough samples ({len(X_all)})"}

    X = np.array(X_all, dtype=np.float32)
    Y = np.array(Y_all, dtype=np.float32)
    Xs = np.array(Xs_all, dtype=np.float32)
    Ys = np.array(Ys_all, dtype=np.float32)

    logger.info("Fine-tuning on %d samples from %d symbols", len(X), len(req.symbols))

    # Stage 1: Tokenizer update (1 epoch)
    tokenizer.train()
    opt_tok = torch.optim.AdamW(tokenizer.parameters(), lr=LR * 10, weight_decay=0.01)
    tok_losses = []

    indices = np.random.permutation(len(X))
    for batch_start in range(0, len(indices), BATCH_SIZE):
        batch_idx = indices[batch_start:batch_start + BATCH_SIZE]
        if len(batch_idx) < 2:
            continue
        x_batch = torch.from_numpy(X[batch_idx]).to(device)
        (z_pre, z_full), bsq_loss, _, _ = tokenizer(x_batch)
        loss = 0.5 * F.mse_loss(z_full, x_batch) + 0.5 * F.mse_loss(z_pre, x_batch) + bsq_loss
        opt_tok.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(tokenizer.parameters(), 1.0)
        opt_tok.step()
        tok_losses.append(loss.item())

    avg_tok_loss = float(np.mean(tok_losses)) if tok_losses else 0
    logger.info("Tokenizer loss: %.4f", avg_tok_loss)

    # Stage 2: Predictor fine-tune (3 epochs)
    model.train()
    tokenizer.eval()
    opt_pred = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=0.01)
    pred_losses = []

    for epoch in range(EPOCHS):
        epoch_losses = []
        indices = np.random.permutation(len(X))

        for batch_start in range(0, len(indices), BATCH_SIZE):
            batch_idx = indices[batch_start:batch_start + BATCH_SIZE]
            if len(batch_idx) < 2:
                continue

            x_b = torch.from_numpy(X[batch_idx]).to(device)
            y_b = torch.from_numpy(Y[batch_idx]).to(device)
            xs_b = torch.from_numpy(Xs[batch_idx]).to(device)
            ys_b = torch.from_numpy(Ys[batch_idx]).to(device)

            full_seq = torch.cat([x_b, y_b], dim=1)
            full_stamp = torch.cat([xs_b, ys_b], dim=1)

            with torch.no_grad():
                z_idx = tokenizer.encode(full_seq, half=True)
                s1, s2 = z_idx[0], z_idx[1]

            s1_logits, s2_logits = model(
                s1[:, :-1], s2[:, :-1],
                stamp=full_stamp[:, :-1],
                use_teacher_forcing=True,
                s1_targets=s1[:, 1:],
            )
            loss, _, _ = model.head.compute_loss(s1_logits, s2_logits, s1[:, 1:], s2[:, 1:])

            opt_pred.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt_pred.step()
            epoch_losses.append(loss.item())

        avg = float(np.mean(epoch_losses)) if epoch_losses else 0
        pred_losses.append(avg)
        logger.info("  Predictor epoch %d/%d: loss=%.4f", epoch + 1, EPOCHS, avg)

    # Save PyTorch weights
    os.makedirs(os.path.join(FINETUNE_DIR, "tokenizer"), exist_ok=True)
    os.makedirs(os.path.join(FINETUNE_DIR, "predictor"), exist_ok=True)
    tokenizer.save_pretrained(os.path.join(FINETUNE_DIR, "tokenizer"))
    model.save_pretrained(os.path.join(FINETUNE_DIR, "predictor"))
    logger.info("PyTorch weights saved")

    # Re-export to ONNX
    logger.info("Re-exporting to ONNX...")
    try:
        os.system("python /app/scripts/export_kronos_onnx.py")
        logger.info("ONNX export complete")
    except Exception as e:
        logger.error("ONNX export failed: %s", e)

    meta = {
        "date": req.date,
        "day_pnl": round(req.day_pnl, 4),
        "training_samples": len(X),
        "symbols": len(req.symbols),
        "epochs": EPOCHS,
        "learning_rate": LR,
        "tokenizer_loss": avg_tok_loss,
        "predictor_losses": pred_losses,
        "finetuned_at": datetime.now().isoformat(),
        "device": device,
        "success": True,
    }

    with open(os.path.join(FINETUNE_DIR, "metadata.json"), "w") as f:
        json.dump(meta, f, indent=2)

    logger.info("Fine-tune complete: %s", meta)
    return meta


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8001)
