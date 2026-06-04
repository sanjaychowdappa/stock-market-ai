#!/usr/bin/env python3
"""
Fine-tune Kronos on 7 months of historical data from Alpaca.

This gives the model exposure to recent market patterns it has never seen,
dramatically improving prediction accuracy for current market conditions.

Usage (run inside the finetune-sidecar container):
    python finetune_historical.py

Data fetched: 7 months of 1-day candles for NVDA, AAPL, MSFT, GOOGL, AMZN
Training: tokenizer (1 epoch) + predictor (10 epochs) with validation
"""

import os
import sys
import json
import logging
import numpy as np
import pandas as pd
from datetime import datetime, timedelta

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger("finetune-historical")

sys.path.insert(0, "/app/kronos_repo")

SYMBOLS = ["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"]
FINETUNE_DIR = "/app/model_cache/finetuned_nvda"
LOOKBACK = 60       # Input sequence length (candles)
PRED_LEN = 5        # Predict next N candles
EPOCHS = 10         # More epochs for historical fine-tune
BATCH_SIZE = 8
LR = 3e-6           # Conservative LR for historical data
CLIP = 5.0
VAL_SPLIT = 0.15    # 15% validation


def fetch_alpaca_bars(symbol, timeframe="1Day", months=7):
    """Fetch historical bars from Alpaca REST API."""
    import httpx

    api_key = os.environ.get("APCA_API_KEY_ID", "")
    api_secret = os.environ.get("APCA_API_SECRET_KEY", "")

    if not api_key or not api_secret:
        raise ValueError("APCA_API_KEY_ID and APCA_API_SECRET_KEY must be set")

    end = datetime.now()
    start = end - timedelta(days=months * 30)

    all_bars = []
    page_token = None

    logger.info(f"Fetching {symbol} {timeframe} bars from {start.date()} to {end.date()}...")

    while True:
        params = {
            "timeframe": timeframe,
            "start": start.strftime("%Y-%m-%dT00:00:00Z"),
            "end": end.strftime("%Y-%m-%dT23:59:59Z"),
            "limit": 10000,
            "feed": "iex",
            "sort": "asc",
        }
        if page_token:
            params["page_token"] = page_token

        url = f"https://data.alpaca.markets/v2/stocks/{symbol}/bars"
        headers = {
            "APCA-API-KEY-ID": api_key,
            "APCA-API-SECRET-KEY": api_secret,
        }

        resp = httpx.get(url, params=params, headers=headers, timeout=30)
        data = resp.json()

        bars = data.get("bars", [])
        if not bars:
            break

        all_bars.extend(bars)
        page_token = data.get("next_page_token")
        if not page_token:
            break

    logger.info(f"  {symbol}: fetched {len(all_bars)} bars")
    return all_bars


def bars_to_dataframe(bars):
    """Convert Alpaca bars to DataFrame."""
    records = []
    for b in bars:
        ts = pd.to_datetime(b["t"])
        records.append({
            "timestamp": ts,
            "open": b["o"],
            "high": b["h"],
            "low": b["l"],
            "close": b["c"],
            "volume": b["v"],
        })
    df = pd.DataFrame(records)
    df.sort_values("timestamp", inplace=True)
    df.reset_index(drop=True, inplace=True)
    return df


def prepare_samples(df, lookback=LOOKBACK, pred_len=PRED_LEN, stride=1):
    """Create sliding window training samples."""
    features = df[["open", "high", "low", "close"]].values.astype(np.float32)
    vol = df["volume"].values.astype(np.float32)
    amount = vol * features.mean(axis=1)
    features_full = np.column_stack([features, vol, amount])  # [N, 6]

    ts = df["timestamp"]
    stamps = np.column_stack([
        ts.dt.minute.values,
        ts.dt.hour.values,
        ts.dt.weekday.values,
        ts.dt.day.values,
        ts.dt.month.values,
    ]).astype(np.float32)

    X, Y, Xs, Ys = [], [], [], []
    total_len = lookback + pred_len

    for i in range(0, len(features_full) - total_len, stride):
        x = features_full[i:i + lookback]
        y = features_full[i + lookback:i + total_len]

        # Normalize each window independently
        x_mean = x.mean(axis=0)
        x_std = x.std(axis=0) + 1e-5
        x_norm = np.clip((x - x_mean) / x_std, -CLIP, CLIP)
        y_norm = np.clip((y - x_mean) / x_std, -CLIP, CLIP)

        X.append(x_norm)
        Y.append(y_norm)
        Xs.append(stamps[i:i + lookback])
        Ys.append(stamps[i + lookback:i + total_len])

    return np.array(X), np.array(Y), np.array(Xs), np.array(Ys)


def main():
    import torch
    import torch.nn.functional as F
    from model import KronosTokenizer, Kronos

    device = "cuda:0" if torch.cuda.is_available() else "cpu"
    logger.info(f"Device: {device}")
    logger.info(f"Fine-tuning Kronos on 7 months of data for {SYMBOLS}")
    logger.info("=" * 60)

    # ── Step 1: Fetch data from Alpaca ──────────────────────────
    logger.info("[1/5] Fetching 7 months of historical data from Alpaca...")

    all_X, all_Y, all_Xs, all_Ys = [], [], [], []

    for sym in SYMBOLS:
        try:
            # Fetch daily bars (7 months)
            daily_bars = fetch_alpaca_bars(sym, "1Day", months=7)
            if len(daily_bars) < LOOKBACK + PRED_LEN + 10:
                logger.warning(f"  {sym}: only {len(daily_bars)} daily bars, skipping")
                continue

            df = bars_to_dataframe(daily_bars)
            X, Y, Xs, Ys = prepare_samples(df, stride=1)
            logger.info(f"  {sym}: {len(X)} daily samples from {len(daily_bars)} bars")
            all_X.append(X)
            all_Y.append(Y)
            all_Xs.append(Xs)
            all_Ys.append(Ys)

            # Also fetch 1-hour bars for finer granularity (last 2 months)
            hourly_bars = fetch_alpaca_bars(sym, "1Hour", months=2)
            if len(hourly_bars) >= LOOKBACK + PRED_LEN + 10:
                df_h = bars_to_dataframe(hourly_bars)
                Xh, Yh, Xsh, Ysh = prepare_samples(df_h, stride=2)
                logger.info(f"  {sym}: {len(Xh)} hourly samples from {len(hourly_bars)} bars")
                all_X.append(Xh)
                all_Y.append(Yh)
                all_Xs.append(Xsh)
                all_Ys.append(Ysh)

        except Exception as e:
            logger.error(f"  {sym}: fetch failed: {e}")
            continue

    if not all_X:
        logger.error("No training data fetched!")
        return

    X = np.concatenate(all_X)
    Y = np.concatenate(all_Y)
    Xs = np.concatenate(all_Xs)
    Ys = np.concatenate(all_Ys)

    # Shuffle and split train/val
    n = len(X)
    indices = np.random.permutation(n)
    val_n = int(n * VAL_SPLIT)
    val_idx = indices[:val_n]
    train_idx = indices[val_n:]

    logger.info(f"Total samples: {n} (train: {len(train_idx)}, val: {len(val_idx)})")

    # ── Step 2: Load model ──────────────────────────────────────
    logger.info("[2/5] Loading Kronos model...")

    ft_tok = os.path.join(FINETUNE_DIR, "tokenizer")
    ft_pred = os.path.join(FINETUNE_DIR, "predictor")

    if os.path.exists(ft_tok) and os.path.exists(ft_pred):
        tokenizer = KronosTokenizer.from_pretrained(ft_tok).to(device)
        model = Kronos.from_pretrained(ft_pred).to(device)
        logger.info("  Loaded existing fine-tuned model (continuing)")
    else:
        tokenizer = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base").to(device)
        model = Kronos.from_pretrained("NeoQuasar/Kronos-small").to(device)
        logger.info("  Loaded base model (first fine-tune)")

    # ── Step 3: Fine-tune tokenizer ─────────────────────────────
    logger.info("[3/5] Fine-tuning tokenizer (1 epoch)...")

    tokenizer.train()
    opt_tok = torch.optim.AdamW(tokenizer.parameters(), lr=LR * 10, weight_decay=0.01)
    tok_losses = []

    shuffled = np.random.permutation(train_idx)
    for batch_start in range(0, len(shuffled), BATCH_SIZE):
        batch_idx = shuffled[batch_start:batch_start + BATCH_SIZE]
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

    avg_tok = float(np.mean(tok_losses)) if tok_losses else 0
    logger.info(f"  Tokenizer loss: {avg_tok:.4f}")

    # ── Step 4: Fine-tune predictor ─────────────────────────────
    logger.info(f"[4/5] Fine-tuning predictor ({EPOCHS} epochs)...")

    model.train()
    tokenizer.eval()
    opt_pred = torch.optim.AdamW(model.parameters(), lr=LR, weight_decay=0.01)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(opt_pred, T_max=EPOCHS, eta_min=LR * 0.1)

    best_val_loss = float("inf")
    train_losses = []
    val_losses = []

    for epoch in range(EPOCHS):
        # Train
        model.train()
        epoch_losses = []
        shuffled = np.random.permutation(train_idx)

        for batch_start in range(0, len(shuffled), BATCH_SIZE):
            batch_idx = shuffled[batch_start:batch_start + BATCH_SIZE]
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

        scheduler.step()
        avg_train = float(np.mean(epoch_losses)) if epoch_losses else 0
        train_losses.append(avg_train)

        # Validate
        model.eval()
        v_losses = []
        with torch.no_grad():
            for batch_start in range(0, len(val_idx), BATCH_SIZE):
                batch_idx = val_idx[batch_start:batch_start + BATCH_SIZE]
                if len(batch_idx) < 2:
                    continue

                x_b = torch.from_numpy(X[batch_idx]).to(device)
                y_b = torch.from_numpy(Y[batch_idx]).to(device)
                xs_b = torch.from_numpy(Xs[batch_idx]).to(device)
                ys_b = torch.from_numpy(Ys[batch_idx]).to(device)

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
                loss, _, _ = model.head.compute_loss(s1_logits, s2_logits, s1[:, 1:], s2[:, 1:])
                v_losses.append(loss.item())

        avg_val = float(np.mean(v_losses)) if v_losses else 0
        val_losses.append(avg_val)

        improved = "***" if avg_val < best_val_loss else ""
        if avg_val < best_val_loss:
            best_val_loss = avg_val

        logger.info(
            f"  Epoch {epoch+1}/{EPOCHS}: train={avg_train:.4f} val={avg_val:.4f} "
            f"lr={scheduler.get_last_lr()[0]:.2e} {improved}"
        )

    # ── Step 5: Save model ──────────────────────────────────────
    logger.info("[5/5] Saving fine-tuned model...")

    os.makedirs(os.path.join(FINETUNE_DIR, "tokenizer"), exist_ok=True)
    os.makedirs(os.path.join(FINETUNE_DIR, "predictor"), exist_ok=True)
    tokenizer.save_pretrained(os.path.join(FINETUNE_DIR, "tokenizer"))
    model.save_pretrained(os.path.join(FINETUNE_DIR, "predictor"))

    # Save training metadata
    meta = {
        "finetuned_at": datetime.now().isoformat(),
        "data_range": "7 months",
        "symbols": SYMBOLS,
        "total_samples": int(n),
        "train_samples": len(train_idx),
        "val_samples": len(val_idx),
        "epochs": EPOCHS,
        "batch_size": BATCH_SIZE,
        "learning_rate": LR,
        "best_val_loss": best_val_loss,
        "train_losses": train_losses,
        "val_losses": val_losses,
        "tokenizer_loss": avg_tok,
        "device": device,
    }

    with open(os.path.join(FINETUNE_DIR, "finetune_history.json"), "w") as f:
        json.dump(meta, f, indent=2)

    logger.info("=" * 60)
    logger.info("FINE-TUNING COMPLETE")
    logger.info(f"  Samples: {n} ({len(train_idx)} train / {len(val_idx)} val)")
    logger.info(f"  Best val loss: {best_val_loss:.4f}")
    logger.info(f"  Model saved to: {FINETUNE_DIR}")
    logger.info("=" * 60)


if __name__ == "__main__":
    main()
