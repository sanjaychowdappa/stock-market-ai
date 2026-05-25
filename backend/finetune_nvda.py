"""
Fine-tune Kronos Foundation Model on NVIDIA (NVDA) historical data.
Uses 2 years of daily OHLCV data to specialize the model for NVDA prediction.
Two-stage: (1) fine-tune tokenizer, (2) fine-tune predictor.
"""
import sys
import os
sys.path.insert(0, "/app/kronos_repo")

import torch
import torch.nn as nn
import torch.nn.functional as F
import numpy as np
import pandas as pd
import yfinance as yf
from datetime import datetime, timedelta
import json
import logging

logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger(__name__)

# ── Configuration ────────────────────────────────────────────────
SYMBOL = "NVDA"
LOOKBACK = 90          # historical window per sample
PRED_LEN = 5           # predict next 5 candles
CLIP = 5.0
EPOCHS_TOKENIZER = 10
EPOCHS_PREDICTOR = 15
BATCH_SIZE = 8
LR_TOKENIZER = 1e-4
LR_PREDICTOR = 2e-5
SAVE_DIR = "/app/model_cache/finetuned_nvda"

PRETRAINED_TOKENIZER = "NeoQuasar/Kronos-Tokenizer-base"
PRETRAINED_MODEL = "NeoQuasar/Kronos-small"

device = "cpu"
if torch.cuda.is_available():
    device = "cuda:0"
elif hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
    device = "mps"

logger.info(f"Device: {device}")


# ── 1. Download NVDA Data ────────────────────────────────────────
logger.info(f"Downloading {SYMBOL} historical data...")
ticker = yf.Ticker(SYMBOL)
df = ticker.history(period="2y", interval="1d")

if df.empty:
    logger.error("No data available!")
    sys.exit(1)

df = df.reset_index()
date_col = "Date" if "Date" in df.columns else "Datetime"
df.rename(columns={date_col: "date"}, inplace=True)

logger.info(f"Downloaded {len(df)} daily candles from {df['date'].iloc[0]} to {df['date'].iloc[-1]}")

# Prepare OHLCV
price_cols = ["Open", "High", "Low", "Close"]
df["Volume"] = df["Volume"].astype(float)
df["Amount"] = df["Volume"] * df[price_cols].mean(axis=1)

features = df[["Open", "High", "Low", "Close", "Volume", "Amount"]].values.astype(np.float32)
timestamps = pd.to_datetime(df["date"])


def calc_stamps(ts):
    """Convert timestamps to temporal feature array [minute, hour, weekday, day, month]."""
    ts = pd.DatetimeIndex(ts)
    return np.column_stack([
        ts.minute,
        ts.hour,
        ts.weekday,
        ts.day,
        ts.month,
    ]).astype(np.float32)


all_stamps = calc_stamps(timestamps)


# ── 2. Create Training Samples ──────────────────────────────────
def create_samples(features, stamps, lookback, pred_len, stride=1):
    """Create sliding window samples for training."""
    X, Y, X_stamp, Y_stamp = [], [], [], []
    total = len(features)

    for i in range(0, total - lookback - pred_len, stride):
        x = features[i:i + lookback]
        y = features[i + lookback:i + lookback + pred_len]

        # Normalize per-sample
        x_mean = x.mean(axis=0)
        x_std = x.std(axis=0) + 1e-5
        x_norm = np.clip((x - x_mean) / x_std, -CLIP, CLIP)
        y_norm = np.clip((y - x_mean) / x_std, -CLIP, CLIP)

        X.append(x_norm)
        Y.append(y_norm)
        X_stamp.append(stamps[i:i + lookback])
        Y_stamp.append(stamps[i + lookback:i + lookback + pred_len])

    return (
        np.array(X, dtype=np.float32),
        np.array(Y, dtype=np.float32),
        np.array(X_stamp, dtype=np.float32),
        np.array(Y_stamp, dtype=np.float32),
    )


logger.info("Creating training samples...")
X, Y, X_stamp, Y_stamp = create_samples(features, all_stamps, LOOKBACK, PRED_LEN, stride=1)

# Train/val split (last 20% for validation)
split = int(len(X) * 0.8)
X_train, X_val = X[:split], X[split:]
Y_train, Y_val = Y[:split], Y[split:]
Xs_train, Xs_val = X_stamp[:split], X_stamp[split:]
Ys_train, Ys_val = Y_stamp[:split], Y_stamp[split:]

logger.info(f"Training samples: {len(X_train)}, Validation: {len(X_val)}")


# ── 3. Load Pre-trained Models ──────────────────────────────────
from model import KronosTokenizer, Kronos

logger.info("Loading pre-trained Kronos tokenizer...")
tokenizer = KronosTokenizer.from_pretrained(PRETRAINED_TOKENIZER).to(device)

logger.info("Loading pre-trained Kronos model...")
model = Kronos.from_pretrained(PRETRAINED_MODEL).to(device)

logger.info(f"Tokenizer params: {sum(p.numel() for p in tokenizer.parameters()):,}")
logger.info(f"Model params: {sum(p.numel() for p in model.parameters()):,}")


# ── 4. Stage 1: Fine-tune Tokenizer ─────────────────────────────
logger.info("=" * 60)
logger.info("STAGE 1: Fine-tuning Tokenizer on NVDA data")
logger.info("=" * 60)

tokenizer.train()
optimizer_tok = torch.optim.AdamW(
    tokenizer.parameters(),
    lr=LR_TOKENIZER,
    betas=(0.9, 0.95),
    weight_decay=0.1,
)
scheduler_tok = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer_tok, T_max=EPOCHS_TOKENIZER)

best_tok_loss = float("inf")

for epoch in range(EPOCHS_TOKENIZER):
    tokenizer.train()
    epoch_losses = []

    # Shuffle training data
    indices = np.random.permutation(len(X_train))

    for batch_start in range(0, len(indices), BATCH_SIZE):
        batch_idx = indices[batch_start:batch_start + BATCH_SIZE]
        if len(batch_idx) < 2:
            continue

        x_batch = torch.from_numpy(X_train[batch_idx]).to(device)

        # Forward through tokenizer
        (z_pre, z_full), bsq_loss, quantized, z_indices = tokenizer(x_batch)

        # Reconstruction loss (both paths)
        recon_loss_full = F.mse_loss(z_full, x_batch)
        recon_loss_pre = F.mse_loss(z_pre, x_batch)
        recon_loss = 0.5 * recon_loss_full + 0.5 * recon_loss_pre

        loss = recon_loss + bsq_loss

        optimizer_tok.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(tokenizer.parameters(), 1.0)
        optimizer_tok.step()

        epoch_losses.append(loss.item())

    scheduler_tok.step()

    # Validation
    tokenizer.eval()
    val_losses = []
    with torch.no_grad():
        for batch_start in range(0, len(X_val), BATCH_SIZE):
            x_batch = torch.from_numpy(X_val[batch_start:batch_start + BATCH_SIZE]).to(device)
            (z_pre, z_full), bsq_loss, quantized, z_indices = tokenizer(x_batch)
            recon_loss = 0.5 * F.mse_loss(z_full, x_batch) + 0.5 * F.mse_loss(z_pre, x_batch)
            val_losses.append((recon_loss + bsq_loss).item())

    train_loss = np.mean(epoch_losses)
    val_loss = np.mean(val_losses)
    lr = scheduler_tok.get_last_lr()[0]

    logger.info(
        f"  Epoch {epoch + 1}/{EPOCHS_TOKENIZER}  "
        f"Train: {train_loss:.4f}  Val: {val_loss:.4f}  LR: {lr:.2e}"
    )

    if val_loss < best_tok_loss:
        best_tok_loss = val_loss
        tok_state = {k: v.cpu().clone() for k, v in tokenizer.state_dict().items()}

# Restore best tokenizer
tokenizer.load_state_dict(tok_state)
logger.info(f"Tokenizer fine-tuned. Best val loss: {best_tok_loss:.4f}")


# ── 5. Stage 2: Fine-tune Predictor ─────────────────────────────
logger.info("=" * 60)
logger.info("STAGE 2: Fine-tuning Predictor on NVDA data")
logger.info("=" * 60)

model.train()
tokenizer.eval()  # Freeze tokenizer during predictor training

optimizer_pred = torch.optim.AdamW(
    model.parameters(),
    lr=LR_PREDICTOR,
    betas=(0.9, 0.95),
    weight_decay=0.1,
)
scheduler_pred = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer_pred, T_max=EPOCHS_PREDICTOR)

best_pred_loss = float("inf")

for epoch in range(EPOCHS_PREDICTOR):
    model.train()
    epoch_losses = []
    indices = np.random.permutation(len(X_train))

    for batch_start in range(0, len(indices), BATCH_SIZE):
        batch_idx = indices[batch_start:batch_start + BATCH_SIZE]
        if len(batch_idx) < 2:
            continue

        # Concatenate x + y for teacher-forced training
        x_batch = torch.from_numpy(X_train[batch_idx]).to(device)
        y_batch = torch.from_numpy(Y_train[batch_idx]).to(device)
        xs_batch = torch.from_numpy(Xs_train[batch_idx]).to(device)
        ys_batch = torch.from_numpy(Ys_train[batch_idx]).to(device)

        full_seq = torch.cat([x_batch, y_batch], dim=1)
        full_stamp = torch.cat([xs_batch, ys_batch], dim=1)

        # Tokenize the full sequence
        with torch.no_grad():
            z_indices = tokenizer.encode(full_seq, half=True)
            s1_ids, s2_ids = z_indices[0], z_indices[1]

        # Input = all tokens except last, target = all tokens except first
        input_s1 = s1_ids[:, :-1]
        input_s2 = s2_ids[:, :-1]
        target_s1 = s1_ids[:, 1:]
        target_s2 = s2_ids[:, 1:]
        input_stamp = full_stamp[:, :-1]

        # Forward with teacher forcing
        s1_logits, s2_logits = model(
            input_s1, input_s2,
            stamp=input_stamp,
            use_teacher_forcing=True,
            s1_targets=target_s1,
        )

        # Compute loss
        loss, ce_s1, ce_s2 = model.head.compute_loss(
            s1_logits, s2_logits, target_s1, target_s2,
        )

        optimizer_pred.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        optimizer_pred.step()

        epoch_losses.append(loss.item())

    scheduler_pred.step()

    # Validation
    model.eval()
    val_losses = []
    with torch.no_grad():
        for batch_start in range(0, len(X_val), BATCH_SIZE):
            end = min(batch_start + BATCH_SIZE, len(X_val))
            x_b = torch.from_numpy(X_val[batch_start:end]).to(device)
            y_b = torch.from_numpy(Y_val[batch_start:end]).to(device)
            xs_b = torch.from_numpy(Xs_val[batch_start:end]).to(device)
            ys_b = torch.from_numpy(Ys_val[batch_start:end]).to(device)

            full_seq = torch.cat([x_b, y_b], dim=1)
            full_stamp = torch.cat([xs_b, ys_b], dim=1)

            z_idx = tokenizer.encode(full_seq, half=True)
            s1, s2 = z_idx[0], z_idx[1]

            s1_logits, s2_logits = model(
                s1[:, :-1], s2[:, :-1],
                stamp=full_stamp[:, :-1],
                use_teacher_forcing=True,
                s1_targets=s1[:, 1:],
            )
            loss, _, _ = model.head.compute_loss(
                s1_logits, s2_logits, s1[:, 1:], s2[:, 1:],
            )
            val_losses.append(loss.item())

    train_loss = np.mean(epoch_losses)
    val_loss = np.mean(val_losses)
    lr = scheduler_pred.get_last_lr()[0]

    logger.info(
        f"  Epoch {epoch + 1}/{EPOCHS_PREDICTOR}  "
        f"Train: {train_loss:.4f}  Val: {val_loss:.4f}  LR: {lr:.2e}"
    )

    if val_loss < best_pred_loss:
        best_pred_loss = val_loss
        pred_state = {k: v.cpu().clone() for k, v in model.state_dict().items()}

# Restore best model
model.load_state_dict(pred_state)
logger.info(f"Predictor fine-tuned. Best val loss: {best_pred_loss:.4f}")


# ── 6. Save Fine-tuned Models ───────────────────────────────────
logger.info("=" * 60)
logger.info("Saving fine-tuned models...")
logger.info("=" * 60)

os.makedirs(SAVE_DIR, exist_ok=True)

# Save tokenizer
tok_path = os.path.join(SAVE_DIR, "tokenizer")
os.makedirs(tok_path, exist_ok=True)
tokenizer.save_pretrained(tok_path)
logger.info(f"  Tokenizer saved to {tok_path}")

# Save predictor
pred_path = os.path.join(SAVE_DIR, "predictor")
os.makedirs(pred_path, exist_ok=True)
model.save_pretrained(pred_path)
logger.info(f"  Predictor saved to {pred_path}")


# ── 7. Evaluate Fine-tuned vs Base Model ────────────────────────
logger.info("=" * 60)
logger.info("EVALUATION: Fine-tuned vs Base Model")
logger.info("=" * 60)

from model import KronosPredictor

# Fine-tuned predictor
ft_predictor = KronosPredictor(model, tokenizer, device=device, max_context=512)

# Load base model for comparison
base_tokenizer = KronosTokenizer.from_pretrained(PRETRAINED_TOKENIZER).to(device)
base_model = Kronos.from_pretrained(PRETRAINED_MODEL).to(device)
base_predictor = KronosPredictor(base_model, base_tokenizer, device=device, max_context=512)

# Prepare test data (last 30 days, predict 5 from various cutoffs)
df_test = df.copy()
df_test["date_str"] = pd.to_datetime(df_test["date"]).dt.strftime("%Y-%m-%d")

test_cutoffs = [5, 10, 15]
base_errors = {"open": [], "high": [], "low": [], "close": []}
ft_errors = {"open": [], "high": [], "low": [], "close": []}

for cutoff in test_cutoffs:
    if cutoff + PRED_LEN > len(df_test):
        continue

    train_part = df_test.iloc[:-cutoff]
    actual_part = df_test.iloc[-cutoff:-cutoff + PRED_LEN] if cutoff > PRED_LEN else df_test.iloc[-cutoff:]
    steps = min(PRED_LEN, len(actual_part))

    # Prepare input for both models
    feat = train_part[["Open", "High", "Low", "Close", "Volume", "Amount"]].values.astype(np.float32)
    ts = pd.to_datetime(train_part["date"])
    stamps_x = calc_stamps(ts)

    # Use last 252 candles max
    lookback = min(len(feat), 252)
    feat = feat[-lookback:]
    stamps_x = stamps_x[-lookback:]
    ts = ts.iloc[-lookback:]

    x_mean = feat.mean(axis=0)
    x_std = feat.std(axis=0) + 1e-5
    x_norm = np.clip((feat - x_mean) / x_std, -CLIP, CLIP)

    # Future timestamps
    last_date = ts.iloc[-1]
    future_dates = []
    d = last_date
    while len(future_dates) < steps:
        d += timedelta(days=1)
        if d.weekday() < 5:
            future_dates.append(d)
    future_ts = pd.Series(future_dates)
    stamps_y = calc_stamps(pd.DatetimeIndex(future_dates))

    x_tensor = torch.from_numpy(x_norm[np.newaxis]).to(device)
    xs_tensor = torch.from_numpy(stamps_x[np.newaxis]).to(device)
    ys_tensor = torch.from_numpy(stamps_y[np.newaxis]).to(device)

    from model.kronos import auto_regressive_inference

    # Base model prediction
    base_model.eval()
    base_tokenizer.eval()
    with torch.no_grad():
        base_preds = auto_regressive_inference(
            base_tokenizer, base_model, x_tensor, xs_tensor, ys_tensor,
            max_context=512, pred_len=steps, clip=CLIP, T=0.8, top_k=0, top_p=0.9, sample_count=3,
        )
    base_preds = base_preds[0, -steps:] * (x_std + 1e-5) + x_mean

    # Fine-tuned prediction
    model.eval()
    tokenizer.eval()
    with torch.no_grad():
        ft_preds = auto_regressive_inference(
            tokenizer, model, x_tensor, xs_tensor, ys_tensor,
            max_context=512, pred_len=steps, clip=CLIP, T=0.8, top_k=0, top_p=0.9, sample_count=3,
        )
    ft_preds = ft_preds[0, -steps:] * (x_std + 1e-5) + x_mean

    print(f"\n[Cutoff {cutoff}d] Predicting {steps} days from {train_part['date_str'].iloc[-1]}")
    print(f"  {'Day':<5} {'Metric':<6} {'Base Pred':>10} {'FT Pred':>10} {'Actual':>10} {'Base Err%':>10} {'FT Err%':>10} {'Improved':>10}")
    print(f"  {'-' * 75}")

    for i in range(steps):
        a = actual_part.iloc[i]
        for j, (metric, col) in enumerate([("Open", "Open"), ("High", "High"), ("Low", "Low"), ("Close", "Close")]):
            act = float(a[col])
            bp = float(base_preds[i, j])
            fp = float(ft_preds[i, j])
            be = abs((bp - act) / act) * 100
            fe = abs((fp - act) / act) * 100
            improved = "YES" if fe < be else "no"
            base_errors[metric.lower()].append(be)
            ft_errors[metric.lower()].append(fe)
            print(f"  D{i+1:<4} {metric:<6} ${bp:>9.2f} ${fp:>9.2f} ${act:>9.2f} {be:>9.2f}% {fe:>9.2f}% {improved:>10}")


# ── 8. Summary ───────────────────────────────────────────────────
print(f"\n{'=' * 70}")
print("FINE-TUNING RESULTS SUMMARY")
print(f"{'=' * 70}")
print(f"\n{'Metric':<8} {'Base Mean%':>12} {'FT Mean%':>12} {'Improvement':>14}")
print("-" * 50)

total_improvement = 0
for metric in ["open", "high", "low", "close"]:
    be = base_errors[metric]
    fe = ft_errors[metric]
    if be and fe:
        bm = np.mean(be)
        fm = np.mean(fe)
        imp = ((bm - fm) / bm) * 100
        total_improvement += imp
        print(f"{metric.upper():<8} {bm:>11.2f}% {fm:>11.2f}% {imp:>+13.1f}%")

all_base = sum(base_errors.values(), [])
all_ft = sum(ft_errors.values(), [])
if all_base and all_ft:
    bm_all = np.mean(all_base)
    fm_all = np.mean(all_ft)
    imp_all = ((bm_all - fm_all) / bm_all) * 100
    print("-" * 50)
    print(f"{'OVERALL':<8} {bm_all:>11.2f}% {fm_all:>11.2f}% {imp_all:>+13.1f}%")

# Save metadata
meta = {
    "symbol": SYMBOL,
    "training_samples": len(X_train),
    "validation_samples": len(X_val),
    "lookback": LOOKBACK,
    "pred_len": PRED_LEN,
    "epochs_tokenizer": EPOCHS_TOKENIZER,
    "epochs_predictor": EPOCHS_PREDICTOR,
    "best_tokenizer_loss": float(best_tok_loss),
    "best_predictor_loss": float(best_pred_loss),
    "base_mean_error": float(np.mean(all_base)) if all_base else None,
    "finetuned_mean_error": float(np.mean(all_ft)) if all_ft else None,
    "improvement_pct": float(imp_all) if all_base and all_ft else None,
    "finetuned_at": datetime.now().isoformat(),
    "device": device,
}
with open(os.path.join(SAVE_DIR, "metadata.json"), "w") as f:
    json.dump(meta, f, indent=2)

print(f"\nModels saved to: {SAVE_DIR}")
print(f"Metadata saved to: {SAVE_DIR}/metadata.json")
print(f"\n{'=' * 70}")
print("FINE-TUNING COMPLETE")
print(f"{'=' * 70}")
