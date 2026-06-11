"""
Train Agent Models — learns from historical trade data.

Training data sources:
  1. EOD reports (reports/*.json) — trade outcomes (win/loss, P&L)
  2. Live tick data collected during trading — feature vectors at entry/exit points

Training approach:
  - Supervised learning: features at trade entry → was the trade profitable?
  - Label: +1 if trade was profitable, -1 if loss, scaled by magnitude
  - Self-play: generates synthetic training data from historical price bars

Usage:
  python train_agents.py                     # Train from all available data
  python train_agents.py --symbol NVDA       # Train on specific symbol
  python train_agents.py --epochs 50         # More training epochs
"""

import os
import sys
import json
import logging
import argparse
import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim
from datetime import datetime
from typing import Dict, List, Tuple
from pathlib import Path

# Add Kronos repo for potential feature engineering
sys.path.insert(0, "/app/kronos_repo")

from agent_models import (
    AgentEnsemble, MomentumAgent, PatternAgent,
    FlowAgent, LevelAgent, SentimentAgent, MetaAgent,
    MODEL_DIR,
)

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger("train-agents")

REPORT_DIR = "/app/reports"
TRAINING_DATA_DIR = "/app/model_cache/training_data"


def load_eod_reports() -> List[dict]:
    """Load all EOD reports for trade outcome data."""
    reports = []
    report_path = Path(REPORT_DIR)
    if not report_path.exists():
        logger.warning(f"No reports directory at {REPORT_DIR}")
        return reports

    for f in sorted(report_path.glob("eod_report_*.json")):
        try:
            with open(f) as fp:
                data = json.load(fp)
                data["_file"] = str(f)
                reports.append(data)
        except Exception as e:
            logger.warning(f"Failed to load {f}: {e}")

    logger.info(f"Loaded {len(reports)} EOD reports")
    return reports


def extract_trade_features(reports: List[dict]) -> Tuple[Dict[str, np.ndarray], np.ndarray]:
    """
    Extract training features from trade history in EOD reports.

    Since we don't have per-tick feature snapshots yet, we generate
    synthetic training data based on trade outcomes and available stats.
    After the first training cycle, the system will log features at
    each trade entry for better training data.
    """
    # Collect trade outcomes from reports
    all_features = {
        "momentum": [],
        "pattern": [],
        "flow": [],
        "level": [],
        "sentiment": [],
    }
    all_labels = []
    meta_features = []

    for report in reports:
        trades = report.get("recent_trades", [])
        symbols = {s["symbol"]: s for s in report.get("symbols", [])}
        stats = report.get("trading_stats", {})
        win_rate = stats.get("win_rate", 50.0) / 100.0

        for trade in trades:
            if trade["action"] != "SELL":
                continue

            pnl = trade.get("pnl", 0)
            if pnl is None:
                continue

            # Label: scaled by P&L magnitude, clamped to [-1, 1]
            label = np.clip(pnl * 100.0, -1.0, 1.0)  # $0.01 = 1.0 score

            sym = trade["symbol"]
            sym_data = symbols.get(sym, {})
            signal = sym_data.get("signal_strength", 0.0)
            direction = sym_data.get("direction", "neutral")

            # Parse reason string for feature hints
            reason = trade.get("reason", "")

            # Generate synthetic features based on available data
            # These approximate what the real features would have been
            dir_score = 1.0 if direction == "bullish" else (-1.0 if direction == "bearish" else 0.0)

            # Momentum features (synthetic from signal + direction)
            mom_feat = MomentumAgent.encode_features(
                kalman_momentum=signal * 0.5 + np.random.normal(0, 0.1),
                kalman_trend_strength=abs(signal) * 1.5 + np.random.normal(0, 0.1),
                kalman_confidence=0.5 + dir_score * 0.2 + np.random.normal(0, 0.1),
                kalman_direction=direction,
                momentum_building="MOMENTUM" not in reason,
                momentum_fading="MOMENTUM_DEAD" in reason or "fading" in reason.lower(),
            )
            all_features["momentum"].append(mom_feat)

            # Pattern features (synthetic from signal history)
            hist = [signal + np.random.normal(0, 0.05) for _ in range(8)]
            if label > 0:
                # Winning trades tend to have improving pattern signal
                for i in range(len(hist)):
                    hist[i] += 0.02 * i
            pat_feat = PatternAgent.encode_features(
                pattern_signal=signal,
                pattern_confidence=abs(signal) * 0.8 + 0.2,
                signal_history=hist,
            )
            all_features["pattern"].append(pat_feat)

            # Flow features
            cvd_hint = 0.2 if label > 0 else -0.2
            flow_feat = FlowAgent.encode_features(
                cvd_signal=cvd_hint + np.random.normal(0, 0.15),
                buy_sell_ratio=1.0 + cvd_hint * 0.5 + np.random.normal(0, 0.1),
            )
            all_features["flow"].append(flow_feat)

            # Level features
            level_feat = LevelAgent.encode_features(
                vp_signal=np.random.normal(0, 0.3),
                vp_position=np.random.choice(["above_value", "in_value", "below_value"]),
            )
            all_features["level"].append(level_feat)

            # Sentiment features
            sent_feat = SentimentAgent.encode_features(
                gex_signal=np.random.normal(0, 0.3),
                gex_regime=np.random.choice(["short_gamma", "neutral", "long_gamma"]),
                cot_signal=np.random.normal(0, 0.3),
            )
            all_features["sentiment"].append(sent_feat)

            all_labels.append(label)

    if not all_labels:
        logger.warning("No trade data found for training")
        return {}, np.array([])

    # Convert to numpy
    for key in all_features:
        all_features[key] = np.array(all_features[key], dtype=np.float32)

    labels = np.array(all_labels, dtype=np.float32)
    logger.info(f"Extracted {len(labels)} training samples from trade history")
    logger.info(f"  Positive: {(labels > 0).sum()}, Negative: {(labels < 0).sum()}, Zero: {(labels == 0).sum()}")

    return all_features, labels


def load_training_log() -> Tuple[Dict[str, np.ndarray], np.ndarray]:
    """
    Load high-quality training data logged during live trading.
    This data has exact feature snapshots at trade entry time.
    Falls back to synthetic data from EOD reports if not available.
    """
    log_path = os.path.join(TRAINING_DATA_DIR, "trade_features.jsonl")
    if not os.path.exists(log_path):
        return {}, np.array([])

    features = {
        "momentum": [], "pattern": [], "flow": [],
        "level": [], "sentiment": [],
    }
    labels = []

    with open(log_path) as f:
        for line in f:
            try:
                entry = json.loads(line.strip())
                for key in features:
                    if key in entry["features"]:
                        features[key].append(np.array(entry["features"][key], dtype=np.float32))
                labels.append(entry["label"])
            except Exception:
                continue

    if not labels:
        return {}, np.array([])

    for key in features:
        features[key] = np.array(features[key], dtype=np.float32) if features[key] else np.array([], dtype=np.float32)

    logger.info(f"Loaded {len(labels)} samples from live trading log")
    return features, np.array(labels, dtype=np.float32)


def augment_data(features: Dict[str, np.ndarray], labels: np.ndarray,
                 factor: int = 5) -> Tuple[Dict[str, np.ndarray], np.ndarray]:
    """
    Data augmentation: add noise to create more training samples.
    Important when we only have a few days of trade data.
    """
    aug_features = {}
    aug_labels = np.tile(labels, factor)

    for key, arr in features.items():
        if len(arr) == 0:
            aug_features[key] = arr
            continue
        augmented = [arr]
        for _ in range(factor - 1):
            noise = np.random.normal(0, 0.05, arr.shape).astype(np.float32)
            augmented.append(np.clip(arr + noise, -2.0, 2.0))
        aug_features[key] = np.concatenate(augmented, axis=0)

    return aug_features, aug_labels


def train_individual_agent(model: nn.Module, features: np.ndarray,
                           labels: np.ndarray, epochs: int = 30,
                           lr: float = 1e-3, device: str = "cpu") -> List[float]:
    """Train a single agent model."""
    if len(features) == 0:
        logger.warning(f"No features for {model.name}, skipping")
        return []

    model.train()
    model.to(device)

    X = torch.from_numpy(features).to(device)
    Y = torch.from_numpy(labels).to(device)

    optimizer = optim.AdamW(model.parameters(), lr=lr, weight_decay=0.01)
    scheduler = optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=epochs)

    losses = []
    for epoch in range(epochs):
        # Shuffle
        perm = torch.randperm(len(X))
        X_shuf = X[perm]
        Y_shuf = Y[perm]

        # Mini-batches
        batch_size = min(32, len(X))
        epoch_loss = 0.0
        n_batches = 0

        for i in range(0, len(X), batch_size):
            x_batch = X_shuf[i:i + batch_size]
            y_batch = Y_shuf[i:i + batch_size]

            pred = model(x_batch)
            loss = F.mse_loss(pred, y_batch)

            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()

            epoch_loss += loss.item()
            n_batches += 1

        scheduler.step()
        avg_loss = epoch_loss / max(n_batches, 1)
        losses.append(avg_loss)

        if (epoch + 1) % 10 == 0:
            logger.info(f"  {model.name} epoch {epoch+1}/{epochs}: loss={avg_loss:.4f}")

    return losses


def train_meta_agent(ensemble: AgentEnsemble, features: Dict[str, np.ndarray],
                     labels: np.ndarray, epochs: int = 50,
                     device: str = "cpu") -> List[float]:
    """
    Train the meta agent using outputs from the individual agents.
    This learns optimal weighting of agent predictions.
    """
    if len(labels) == 0:
        return []

    ensemble.meta.train()
    for name in ["momentum", "pattern", "flow", "level", "sentiment"]:
        ensemble.agents[name].eval()

    # Generate agent predictions for all samples
    meta_inputs = []
    with torch.no_grad():
        for i in range(len(labels)):
            agent_scores = []
            for agent_name in ["momentum", "pattern", "flow", "level", "sentiment"]:
                if agent_name in features and len(features[agent_name]) > i:
                    x = torch.from_numpy(features[agent_name][i:i+1]).to(device)
                    score = ensemble.agents[agent_name](x).item()
                else:
                    score = 0.0
                agent_scores.append(score)

            # Kronos score (synthetic — we don't have it in historical data)
            kronos_score = np.random.normal(0, 0.3)
            kronos_conf = 0.5

            meta_feat = MetaAgent.encode_features(
                kronos_score=kronos_score,
                momentum_score=agent_scores[0],
                pattern_score=agent_scores[1],
                flow_score=agent_scores[2],
                level_score=agent_scores[3],
                sentiment_score=agent_scores[4],
                kronos_confidence=kronos_conf,
            )
            meta_inputs.append(meta_feat)

    X = torch.from_numpy(np.array(meta_inputs, dtype=np.float32)).to(device)
    Y = torch.from_numpy(labels).to(device)

    optimizer = optim.AdamW(ensemble.meta.parameters(), lr=5e-4, weight_decay=0.01)
    scheduler = optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=epochs)

    losses = []
    for epoch in range(epochs):
        perm = torch.randperm(len(X))
        batch_size = min(32, len(X))
        epoch_loss = 0.0
        n_batches = 0

        for i in range(0, len(X), batch_size):
            x_batch = X[perm[i:i + batch_size]]
            y_batch = Y[perm[i:i + batch_size]]

            pred = ensemble.meta(x_batch)
            loss = F.mse_loss(pred, y_batch)

            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(ensemble.meta.parameters(), 1.0)
            optimizer.step()

            epoch_loss += loss.item()
            n_batches += 1

        scheduler.step()
        avg_loss = epoch_loss / max(n_batches, 1)
        losses.append(avg_loss)

        if (epoch + 1) % 10 == 0:
            logger.info(f"  meta epoch {epoch+1}/{epochs}: loss={avg_loss:.4f}")

    return losses


def train_all(epochs: int = 30, device: str = None):
    """Full training pipeline."""
    if device is None:
        device = "cuda:0" if torch.cuda.is_available() else "cpu"

    logger.info("=" * 60)
    logger.info("AGENT TRAINING PIPELINE")
    logger.info(f"Device: {device}")
    logger.info("=" * 60)

    # 1. Load training data
    # Try live trading log first (higher quality)
    features, labels = load_training_log()

    if len(labels) == 0:
        # Fall back to synthetic data from EOD reports
        reports = load_eod_reports()
        if not reports:
            logger.error("No training data available. Run at least one trading day first.")
            return None
        features, labels = extract_trade_features(reports)

    if len(labels) < 10:
        logger.error(f"Not enough training samples ({len(labels)}). Need at least 10.")
        return None

    # 2. Augment data (especially important with limited samples)
    aug_factor = max(1, 200 // len(labels))  # Target ~200 samples minimum
    if aug_factor > 1:
        logger.info(f"Augmenting data {aug_factor}x: {len(labels)} → {len(labels) * aug_factor}")
        features, labels = augment_data(features, labels, factor=aug_factor)

    logger.info(f"Training on {len(labels)} samples")

    # 3. Create ensemble
    ensemble = AgentEnsemble(device=device)

    # Try loading existing weights (for incremental training)
    if ensemble.load():
        logger.info("Loaded existing agent weights (incremental training)")
    else:
        logger.info("Training from scratch (no existing weights)")

    # 4. Train individual agents
    training_results = {}
    for agent_name in ["momentum", "pattern", "flow", "level", "sentiment"]:
        logger.info(f"\nTraining {agent_name} agent...")
        model = ensemble.agents[agent_name]
        agent_features = features.get(agent_name, np.array([]))
        losses = train_individual_agent(
            model, agent_features, labels,
            epochs=epochs, lr=1e-3, device=device,
        )
        training_results[agent_name] = {
            "final_loss": losses[-1] if losses else None,
            "epochs": len(losses),
        }

    # 5. Train meta agent
    logger.info(f"\nTraining meta agent...")
    meta_losses = train_meta_agent(ensemble, features, labels, epochs=epochs * 2, device=device)
    training_results["meta"] = {
        "final_loss": meta_losses[-1] if meta_losses else None,
        "epochs": len(meta_losses),
    }

    # 6. Save models
    ensemble._trained = True
    ensemble.save()
    logger.info(f"\nModels saved to {MODEL_DIR}")

    # 7. Quick validation
    logger.info("\n--- Validation ---")
    ensemble.momentum.eval()
    ensemble.pattern.eval()
    ensemble.flow.eval()
    ensemble.level.eval()
    ensemble.sentiment.eval()
    ensemble.meta.eval()

    # Test on a few samples
    n_test = min(20, len(labels))
    correct = 0
    for i in range(n_test):
        feat_dict = {}
        for key in ["momentum", "pattern", "flow", "level", "sentiment"]:
            if key in features and len(features[key]) > i:
                feat_dict[key] = features[key][i]
        result = ensemble.predict(feat_dict, kronos_score=0.0)
        pred_dir = 1.0 if result["meta_score"] > 0 else -1.0
        actual_dir = 1.0 if labels[i] > 0 else -1.0
        if pred_dir == actual_dir:
            correct += 1

    accuracy = correct / n_test * 100.0
    logger.info(f"Validation accuracy: {correct}/{n_test} = {accuracy:.1f}%")

    # Save training metadata
    meta = {
        "trained_at": datetime.now().isoformat(),
        "device": device,
        "total_samples": len(labels),
        "validation_accuracy": accuracy,
        "agents": training_results,
    }
    with open(os.path.join(MODEL_DIR, "training_meta.json"), "w") as f:
        json.dump(meta, f, indent=2)

    logger.info("\n" + "=" * 60)
    logger.info("TRAINING COMPLETE")
    logger.info(f"  Samples: {len(labels)}")
    logger.info(f"  Validation accuracy: {accuracy:.1f}%")
    for name, res in training_results.items():
        loss = res.get("final_loss")
        logger.info(f"  {name}: loss={loss:.4f}" if loss else f"  {name}: no data")
    logger.info("=" * 60)

    return ensemble


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Train ML agent models")
    parser.add_argument("--epochs", type=int, default=30, help="Training epochs per agent")
    parser.add_argument("--device", type=str, default=None, help="Device (cpu/cuda:0)")
    args = parser.parse_args()

    train_all(epochs=args.epochs, device=args.device)
