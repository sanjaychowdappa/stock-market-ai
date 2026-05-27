"""
Daily Prediction Tracker + Auto Fine-Tuner.

Collects every prediction and actual price throughout the day,
computes prediction accuracy at EOD, and triggers Kronos fine-tuning
if the day was profitable.

Data saved to: /app/model_cache/daily_logs/<date>/
"""

import os
import json
import time
import asyncio
import logging
import numpy as np
import pandas as pd
from datetime import datetime, date, timedelta
from collections import deque
from typing import Optional

logger = logging.getLogger(__name__)

LOG_DIR = "/app/model_cache/daily_logs"
FINETUNE_DIR = "/app/model_cache/finetuned_nvda"
TOP_SYMBOLS = ["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"]


class DailyRecord:
    """Stores all predictions and actuals for one symbol for one day."""

    def __init__(self, symbol: str):
        self.symbol = symbol
        self.ticks: list[dict] = []             # {ts, price}
        self.predictions: list[dict] = []       # {ts, current, pred_5s, pred_10s, pred_30s, signal, ...}
        self.trades: list[dict] = []            # from paper trader
        self.candles_1min: list[dict] = []      # aggregated 1-min candles for fine-tuning

    def add_tick(self, price: float):
        self.ticks.append({"ts": time.time(), "price": price})

    def add_prediction(self, data: dict):
        entry = {
            "ts": time.time(),
            "current_price": data.get("current_price", 0),
            "pattern_signal": data.get("pattern", {}).get("signal", 0),
            "pattern_direction": data.get("pattern", {}).get("direction", "neutral"),
            "kronos_age": data.get("kronos_age_seconds", 0),
            "atr": data.get("atr", 0),
        }
        # Save key prediction horizons
        for p in data.get("predictions", []):
            sec = p.get("seconds_ahead", 0)
            if sec in (5, 10, 30):
                entry[f"pred_{sec}s"] = p.get("predicted_price", 0)
                entry[f"pred_{sec}s_kronos"] = p.get("kronos_price", 0)
        self.predictions.append(entry)

    def add_trade(self, trade: dict):
        self.trades.append(trade)

    def compute_accuracy(self) -> dict:
        """Compute prediction accuracy by comparing predictions with actual outcomes."""
        if len(self.predictions) < 30:
            return {"error": "Not enough predictions", "count": len(self.predictions)}

        correct_5s = 0
        correct_10s = 0
        total_5s = 0
        total_10s = 0
        errors_5s = []
        errors_10s = []

        # Build a price lookup by timestamp
        prices = {t["ts"]: t["price"] for t in self.ticks}
        ts_list = sorted(prices.keys())

        for pred in self.predictions:
            ts = pred["ts"]
            current = pred["current_price"]
            if current <= 0:
                continue

            # Find actual price 5s and 10s later
            for sec, key in [(5, "pred_5s"), (10, "pred_10s")]:
                predicted = pred.get(key, 0)
                if predicted <= 0:
                    continue

                target_ts = ts + sec
                # Find closest actual tick
                actual = None
                for t in ts_list:
                    if t >= target_ts - 1.5 and t <= target_ts + 1.5:
                        actual = prices[t]
                        break

                if actual is None:
                    continue

                # Direction accuracy
                pred_dir = "up" if predicted > current else "down"
                actual_dir = "up" if actual > current else "down"
                is_correct = pred_dir == actual_dir

                error_pct = abs(predicted - actual) / actual * 100

                if sec == 5:
                    total_5s += 1
                    if is_correct:
                        correct_5s += 1
                    errors_5s.append(error_pct)
                elif sec == 10:
                    total_10s += 1
                    if is_correct:
                        correct_10s += 1
                    errors_10s.append(error_pct)

        return {
            "total_ticks": len(self.ticks),
            "total_predictions": len(self.predictions),
            "direction_accuracy_5s": round(correct_5s / total_5s * 100, 2) if total_5s > 0 else 0,
            "direction_accuracy_10s": round(correct_10s / total_10s * 100, 2) if total_10s > 0 else 0,
            "mean_error_5s": round(float(np.mean(errors_5s)), 4) if errors_5s else 0,
            "mean_error_10s": round(float(np.mean(errors_10s)), 4) if errors_10s else 0,
            "sample_count_5s": total_5s,
            "sample_count_10s": total_10s,
        }

    def build_1min_candles(self) -> pd.DataFrame:
        """Aggregate ticks into 1-minute candles for fine-tuning."""
        if len(self.ticks) < 60:
            return pd.DataFrame()

        df = pd.DataFrame(self.ticks)
        df["dt"] = pd.to_datetime(df["ts"], unit="s")
        df.set_index("dt", inplace=True)

        candles = df["price"].resample("1min").ohlc()
        candles.columns = ["Open", "High", "Low", "Close"]
        candles["Volume"] = df["price"].resample("1min").count()
        candles = candles.dropna()
        candles["Date"] = candles.index
        candles = candles.reset_index(drop=True)
        return candles

    def to_dict(self) -> dict:
        return {
            "symbol": self.symbol,
            "ticks_count": len(self.ticks),
            "predictions_count": len(self.predictions),
            "trades_count": len(self.trades),
            "accuracy": self.compute_accuracy(),
        }


class DailyTracker:
    """
    Singleton that tracks all symbols throughout the day,
    saves results at EOD, and triggers fine-tuning if profitable.
    """

    def __init__(self):
        self.today: str = date.today().isoformat()
        self.records: dict[str, DailyRecord] = {s: DailyRecord(s) for s in TOP_SYMBOLS}
        self.portfolio_snapshots: list[dict] = []
        self._running = False
        self._collection_task: Optional[asyncio.Task] = None
        self._eod_task: Optional[asyncio.Task] = None
        self._day_start_value: float = 0
        self._day_end_value: float = 0
        self._day_pnl: float = 0
        self._finetune_triggered: bool = False
        self._last_save: float = 0

    def start(self):
        """Start daily collection and EOD scheduling."""
        if not self._running:
            self._running = True
            self._collection_task = asyncio.create_task(self._collect_loop())
            self._eod_task = asyncio.create_task(self._eod_check_loop())
            logger.info("DailyTracker started for %s", self.today)

    def stop(self):
        self._running = False
        if self._collection_task:
            self._collection_task.cancel()
        if self._eod_task:
            self._eod_task.cancel()

    def _reset_day(self):
        """Reset for a new trading day."""
        self.today = date.today().isoformat()
        self.records = {s: DailyRecord(s) for s in TOP_SYMBOLS}
        self.portfolio_snapshots = []
        self._day_start_value = 0
        self._day_end_value = 0
        self._day_pnl = 0
        self._finetune_triggered = False
        self._last_save = 0
        logger.info("DailyTracker reset for new day: %s", self.today)

    async def _collect_loop(self):
        """Collect prediction data from all engines every second."""
        from app.services.realtime_engine import get_engine
        from app.services.paper_trader import get_trader

        # Wait for engines to warm up
        await asyncio.sleep(10)

        engines = {}
        queues = {}
        for sym in TOP_SYMBOLS:
            engine = get_engine(sym)
            q = engine.subscribe()
            engines[sym] = engine
            queues[sym] = q

        trader = get_trader()
        # Subscribe to paper trader so its trading loop starts
        trader_q = trader.subscribe()

        try:
            while self._running:
                # Check for day rollover
                today = date.today().isoformat()
                if today != self.today:
                    # Save yesterday's data before resetting
                    await self._save_day()
                    self._reset_day()

                # Drain paper trader queue to keep it flowing
                try:
                    while not trader_q.empty():
                        trader_q.get_nowait()
                except Exception:
                    pass

                # Drain prediction queues
                for sym, q in queues.items():
                    try:
                        while not q.empty():
                            data = q.get_nowait()
                            self.records[sym].add_tick(data.get("current_price", 0))
                            self.records[sym].add_prediction(data)
                    except Exception:
                        pass

                # Capture portfolio snapshot
                if trader._last_payload:
                    portfolio = trader._last_payload.get("portfolio", {})
                    self.portfolio_snapshots.append({
                        "ts": time.time(),
                        "value": portfolio.get("total_value", 0),
                        "cash": portfolio.get("cash", 0),
                        "pnl": portfolio.get("total_pnl", 0),
                        "pnl_pct": portfolio.get("total_pnl_pct", 0),
                        "trades": trader._last_payload.get("stats", {}).get("total_trades", 0),
                    })

                    if self._day_start_value == 0:
                        self._day_start_value = portfolio.get("total_value", 100)
                    self._day_end_value = portfolio.get("total_value", 0)
                    self._day_pnl = portfolio.get("total_pnl", 0)

                    # Capture trades
                    for t in trader._last_payload.get("recent_trades", []):
                        for sym in TOP_SYMBOLS:
                            if t.get("symbol") == sym:
                                # Avoid duplicates by checking time
                                existing_times = {tr.get("time") for tr in self.records[sym].trades}
                                if t.get("time") not in existing_times:
                                    self.records[sym].add_trade(t)

                # Periodic save (every 5 minutes)
                now = time.time()
                if now - self._last_save > 300:
                    await self._save_day()
                    self._last_save = now

                await asyncio.sleep(1)

        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error("DailyTracker collect error: %s", e, exc_info=True)
        finally:
            for sym, q in queues.items():
                engines[sym].unsubscribe(q)
            trader.unsubscribe(trader_q)

    async def _eod_check_loop(self):
        """Check every 60 seconds if it's end of day to trigger fine-tuning."""
        while self._running:
            try:
                await asyncio.sleep(60)

                now = datetime.now()
                # Market closes at 4pm ET (20:00 UTC)
                # Run EOD analysis at 4:05pm or later
                # For after-hours / weekend testing, also trigger after enough data
                should_trigger = False

                # After market close
                if now.hour >= 16 and now.minute >= 5 and not self._finetune_triggered:
                    should_trigger = True

                # For testing: trigger after 5000+ ticks collected (~17min with 5 symbols)
                total_ticks = sum(len(r.ticks) for r in self.records.values())
                if total_ticks >= 5000 and not self._finetune_triggered:
                    should_trigger = True

                if should_trigger:
                    self._finetune_triggered = True
                    logger.info("EOD triggered — saving data and checking for fine-tune")
                    await self._save_day()
                    await self._evaluate_and_finetune()

            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.error("EOD check error: %s", e, exc_info=True)

    async def _save_day(self):
        """Save all daily data to disk."""
        day_dir = os.path.join(LOG_DIR, self.today)
        os.makedirs(day_dir, exist_ok=True)

        # Summary
        summary = {
            "date": self.today,
            "saved_at": datetime.now().isoformat(),
            "portfolio": {
                "start_value": round(self._day_start_value, 2),
                "end_value": round(self._day_end_value, 2),
                "pnl": round(self._day_pnl, 4),
                "pnl_pct": round(self._day_pnl / self._day_start_value * 100, 4) if self._day_start_value > 0 else 0,
                "profitable": self._day_pnl > 0,
            },
            "symbols": {},
        }

        for sym, record in self.records.items():
            summary["symbols"][sym] = record.to_dict()

            # Save ticks
            if record.ticks:
                ticks_path = os.path.join(day_dir, f"{sym}_ticks.json")
                with open(ticks_path, "w") as f:
                    json.dump(record.ticks[-10000:], f)  # last 10k ticks

            # Save predictions
            if record.predictions:
                preds_path = os.path.join(day_dir, f"{sym}_predictions.json")
                with open(preds_path, "w") as f:
                    json.dump(record.predictions[-10000:], f)

            # Save trades
            if record.trades:
                trades_path = os.path.join(day_dir, f"{sym}_trades.json")
                with open(trades_path, "w") as f:
                    json.dump(record.trades, f)

            # Save 1-min candles
            candle_df = record.build_1min_candles()
            if not candle_df.empty:
                candle_path = os.path.join(day_dir, f"{sym}_candles_1min.csv")
                candle_df.to_csv(candle_path, index=False)

        # Save portfolio snapshots
        if self.portfolio_snapshots:
            snap_path = os.path.join(day_dir, "portfolio_snapshots.json")
            with open(snap_path, "w") as f:
                json.dump(self.portfolio_snapshots[-3600:], f)

        # Save summary
        summary_path = os.path.join(day_dir, "summary.json")
        with open(summary_path, "w") as f:
            json.dump(summary, f, indent=2)

        logger.info("Daily data saved to %s (%d ticks total)",
                     day_dir, sum(len(r.ticks) for r in self.records.values()))

    async def _evaluate_and_finetune(self):
        """
        At EOD: evaluate performance. If profitable, fine-tune Kronos
        on today's data to reinforce the winning patterns.
        """
        logger.info("=" * 60)
        logger.info("END OF DAY EVALUATION — %s", self.today)
        logger.info("=" * 60)

        # Check profitability
        if self._day_pnl <= 0:
            logger.info("Day was NOT profitable (PnL: $%.4f). Skipping fine-tune.", self._day_pnl)
            return

        logger.info("Day WAS PROFITABLE! PnL: $%.4f. Triggering fine-tune.", self._day_pnl)

        # Collect all 1-min candle data from today for fine-tuning
        all_candles = []
        for sym, record in self.records.items():
            candle_df = record.build_1min_candles()
            if not candle_df.empty and len(candle_df) >= 30:
                candle_df["Symbol"] = sym
                all_candles.append(candle_df)
                logger.info("  %s: %d 1-min candles for training", sym, len(candle_df))

        if not all_candles:
            logger.warning("Not enough candle data for fine-tuning.")
            return

        # Run fine-tuning in background thread (GPU intensive)
        loop = asyncio.get_event_loop()
        try:
            result = await loop.run_in_executor(None, self._run_finetune, all_candles)
            logger.info("Fine-tuning result: %s", result)

            # Reload the model with new weights
            if result.get("success"):
                from app.services.kronos_predictor import reload_kronos
                reload_kronos()
                logger.info("Kronos model reloaded with updated weights")

                # Save fine-tune record
                day_dir = os.path.join(LOG_DIR, self.today)
                ft_path = os.path.join(day_dir, "finetune_result.json")
                with open(ft_path, "w") as f:
                    json.dump(result, f, indent=2)

        except Exception as e:
            logger.error("Fine-tuning failed: %s", e, exc_info=True)

    def _run_finetune(self, candle_dfs: list[pd.DataFrame]) -> dict:
        """
        Fine-tune Kronos on today's profitable trading data.
        Uses a lightweight 3-epoch pass to reinforce patterns that made money.
        Runs synchronously on GPU.
        """
        import sys
        sys.path.insert(0, "/app/kronos_repo")

        import torch
        import torch.nn.functional as F
        from model import KronosTokenizer, Kronos

        device = "cuda:0" if torch.cuda.is_available() else "cpu"
        CLIP = 5.0
        LOOKBACK = 60
        PRED_LEN = 5
        EPOCHS = 3
        BATCH_SIZE = 4
        LR = 5e-6  # very small LR for incremental update

        # Load current model
        ft_tok_path = os.path.join(FINETUNE_DIR, "tokenizer")
        ft_pred_path = os.path.join(FINETUNE_DIR, "predictor")

        if os.path.exists(ft_tok_path) and os.path.exists(ft_pred_path):
            tokenizer = KronosTokenizer.from_pretrained(ft_tok_path).to(device)
            model = Kronos.from_pretrained(ft_pred_path).to(device)
            logger.info("Loaded existing fine-tuned model for incremental update")
        else:
            tokenizer = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base").to(device)
            model = Kronos.from_pretrained("NeoQuasar/Kronos-small").to(device)
            logger.info("Loaded base model for first fine-tune")

        # Prepare training samples from all symbols' 1-min candles
        X_all, Y_all, Xs_all, Ys_all = [], [], [], []

        for df in candle_dfs:
            if len(df) < LOOKBACK + PRED_LEN:
                continue

            features = df[["Open", "High", "Low", "Close"]].values.astype(np.float32)
            # Add Volume and Amount columns
            vol = df["Volume"].values.astype(np.float32) if "Volume" in df.columns else np.ones(len(df), dtype=np.float32)
            amount = vol * features.mean(axis=1)
            features_full = np.column_stack([features, vol, amount])

            # Timestamps
            if "Date" in df.columns:
                ts = pd.to_datetime(df["Date"])
            else:
                ts = pd.date_range(end=datetime.now(), periods=len(df), freq="1min")

            stamps = np.column_stack([
                ts.minute if hasattr(ts, 'minute') else pd.DatetimeIndex(ts).minute,
                ts.hour if hasattr(ts, 'hour') else pd.DatetimeIndex(ts).hour,
                ts.weekday if hasattr(ts, 'weekday') else pd.DatetimeIndex(ts).weekday,
                ts.day if hasattr(ts, 'day') else pd.DatetimeIndex(ts).day,
                ts.month if hasattr(ts, 'month') else pd.DatetimeIndex(ts).month,
            ]).astype(np.float32)

            # Create sliding window samples
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
            return {"success": False, "error": "Not enough training samples", "samples": len(X_all)}

        X = np.array(X_all, dtype=np.float32)
        Y = np.array(Y_all, dtype=np.float32)
        Xs = np.array(Xs_all, dtype=np.float32)
        Ys = np.array(Ys_all, dtype=np.float32)

        logger.info("Fine-tuning on %d samples from %d symbols", len(X), len(candle_dfs))

        # Stage 1: Quick tokenizer update (1 epoch)
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
        logger.info("Tokenizer update: loss=%.4f", avg_tok_loss)

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

        # Save updated model
        os.makedirs(os.path.join(FINETUNE_DIR, "tokenizer"), exist_ok=True)
        os.makedirs(os.path.join(FINETUNE_DIR, "predictor"), exist_ok=True)
        tokenizer.save_pretrained(os.path.join(FINETUNE_DIR, "tokenizer"))
        model.save_pretrained(os.path.join(FINETUNE_DIR, "predictor"))

        # Save metadata
        meta = {
            "date": self.today,
            "day_pnl": round(self._day_pnl, 4),
            "training_samples": len(X),
            "symbols": len(candle_dfs),
            "epochs": EPOCHS,
            "learning_rate": LR,
            "tokenizer_loss": avg_tok_loss,
            "predictor_losses": pred_losses,
            "finetuned_at": datetime.now().isoformat(),
            "device": device,
        }
        with open(os.path.join(FINETUNE_DIR, "metadata.json"), "w") as f:
            json.dump(meta, f, indent=2)

        logger.info("Fine-tuned model saved to %s", FINETUNE_DIR)
        return {"success": True, **meta}

    def get_status(self) -> dict:
        """Return current daily tracking status."""
        return {
            "date": self.today,
            "running": self._running,
            "portfolio": {
                "start_value": round(self._day_start_value, 2),
                "current_value": round(self._day_end_value, 2),
                "pnl": round(self._day_pnl, 4),
                "profitable": self._day_pnl > 0,
            },
            "data_collected": {
                sym: {
                    "ticks": len(r.ticks),
                    "predictions": len(r.predictions),
                    "trades": len(r.trades),
                }
                for sym, r in self.records.items()
            },
            "finetune_triggered": self._finetune_triggered,
            "snapshots_count": len(self.portfolio_snapshots),
        }


# ── Singleton ───────────────────────────────────────────────────────
_tracker: Optional[DailyTracker] = None


def get_tracker() -> DailyTracker:
    global _tracker
    if _tracker is None:
        _tracker = DailyTracker()
    return _tracker
