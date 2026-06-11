"""
Agent Prediction Server — serves ML agent predictions to the Rust backend.

Runs on the same container as kronos_server.py, sharing the GPU.
The Rust backend calls POST /predict/agents with current market features,
and gets back individual agent scores + meta-ensemble decision.

Also logs feature snapshots at trade entry time for future training.

Endpoints:
  POST /predict/agents  — get agent scores for a symbol
  POST /log/trade       — log trade outcome for training data
  POST /train           — trigger incremental training
  GET  /agents/health   — check if agents are loaded
"""

import os
import sys
import json
import logging
import numpy as np
from datetime import datetime
from pathlib import Path
from fastapi import FastAPI
from pydantic import BaseModel
from typing import List, Optional, Dict

sys.path.insert(0, "/app/kronos_repo")

from agent_models import (
    AgentEnsemble, MomentumAgent, PatternAgent,
    FlowAgent, LevelAgent, SentimentAgent, MetaAgent,
    MODEL_DIR,
)

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
logger = logging.getLogger("agent-server")

app = FastAPI(title="Agent Prediction Server")

TRAINING_DATA_DIR = "/app/model_cache/training_data"

# Global ensemble
_ensemble: Optional[AgentEnsemble] = None


class AgentPredictRequest(BaseModel):
    symbol: str
    # Kronos data (passed through from existing Kronos predictions)
    kronos_score: float = 0.0
    kronos_confidence: float = 0.5
    # Kalman features
    kalman_momentum: float = 0.0
    kalman_trend_strength: float = 0.0
    kalman_confidence: float = 0.5
    kalman_direction: str = "neutral"
    kalman_momentum_building: bool = False
    kalman_momentum_fading: bool = False
    # Pattern features
    pattern_signal: float = 0.0
    pattern_confidence: float = 0.0
    signal_history: List[float] = []
    # CVD features
    cvd_signal: float = 0.0
    cvd_buy_sell_ratio: float = 1.0
    prev_cvd_signal: float = 0.0
    # VP features
    vp_signal: float = 0.0
    vp_position: str = "unknown"
    price: float = 0.0
    poc_price: float = 0.0
    # GEX + COT features
    gex_signal: float = 0.0
    gex_regime: str = "neutral"
    cot_signal: float = 0.0


class TradeLogRequest(BaseModel):
    symbol: str
    action: str  # "BUY" or "SELL"
    price: float
    pnl: Optional[float] = None
    pnl_pct: Optional[float] = None
    hold_seconds: Optional[int] = None
    reason: str = ""
    # Feature snapshot at trade time
    features: Dict = {}


@app.on_event("startup")
async def startup():
    global _ensemble
    import torch
    device = "cuda:0" if torch.cuda.is_available() else "cpu"
    logger.info(f"Initializing agent ensemble on {device}...")

    _ensemble = AgentEnsemble(device=device)
    if _ensemble.load():
        logger.info("Loaded trained agent models")
    else:
        logger.info("No trained models found — agents will output random until first training")

    # Ensure training data directory exists
    os.makedirs(TRAINING_DATA_DIR, exist_ok=True)
    logger.info("Agent server ready")


@app.get("/agents/health")
def agents_health():
    return {
        "status": "ready" if _ensemble else "not_loaded",
        "trained": _ensemble.is_trained if _ensemble else False,
        "model_dir": MODEL_DIR,
        "agents": list(_ensemble.agents.keys()) if _ensemble else [],
    }


@app.post("/predict/agents")
def predict_agents(req: AgentPredictRequest):
    if not _ensemble:
        return {"error": "Agent ensemble not loaded"}

    try:
        # Encode features for each agent
        features = {
            "momentum": MomentumAgent.encode_features(
                kalman_momentum=req.kalman_momentum,
                kalman_trend_strength=req.kalman_trend_strength,
                kalman_confidence=req.kalman_confidence,
                kalman_direction=req.kalman_direction,
                momentum_building=req.kalman_momentum_building,
                momentum_fading=req.kalman_momentum_fading,
            ),
            "pattern": PatternAgent.encode_features(
                pattern_signal=req.pattern_signal,
                pattern_confidence=req.pattern_confidence,
                signal_history=req.signal_history,
            ),
            "flow": FlowAgent.encode_features(
                cvd_signal=req.cvd_signal,
                buy_sell_ratio=req.cvd_buy_sell_ratio,
                prev_cvd_signal=req.prev_cvd_signal,
            ),
            "level": LevelAgent.encode_features(
                vp_signal=req.vp_signal,
                vp_position=req.vp_position,
                price=req.price,
                poc_price=req.poc_price,
            ),
            "sentiment": SentimentAgent.encode_features(
                gex_signal=req.gex_signal,
                gex_regime=req.gex_regime,
                cot_signal=req.cot_signal,
            ),
        }

        result = _ensemble.predict(
            features=features,
            kronos_score=req.kronos_score,
            kronos_confidence=req.kronos_confidence,
        )

        result["symbol"] = req.symbol
        return result

    except Exception as e:
        logger.error(f"Agent prediction failed: {e}", exc_info=True)
        return {"error": str(e)}


@app.post("/log/trade")
def log_trade(req: TradeLogRequest):
    """
    Log a trade with its feature snapshot for future training.
    Called by the Rust backend after each BUY/SELL.
    """
    try:
        log_path = os.path.join(TRAINING_DATA_DIR, "trade_features.jsonl")

        entry = {
            "timestamp": datetime.now().isoformat(),
            "symbol": req.symbol,
            "action": req.action,
            "price": req.price,
            "pnl": req.pnl,
            "pnl_pct": req.pnl_pct,
            "hold_seconds": req.hold_seconds,
            "reason": req.reason,
            "features": req.features,
            "label": None,
        }

        # For SELL trades, compute the training label from P&L
        if req.action == "SELL" and req.pnl is not None:
            entry["label"] = float(np.clip(req.pnl * 100.0, -1.0, 1.0))

        with open(log_path, "a") as f:
            f.write(json.dumps(entry) + "\n")

        return {"status": "logged", "label": entry.get("label")}

    except Exception as e:
        logger.error(f"Failed to log trade: {e}")
        return {"error": str(e)}


@app.post("/train")
async def trigger_training():
    """Trigger incremental training from accumulated trade data."""
    global _ensemble

    try:
        from train_agents import train_all
        import torch

        device = "cuda:0" if torch.cuda.is_available() else "cpu"
        result = train_all(epochs=30, device=device)

        if result is not None:
            _ensemble = result
            return {
                "status": "training_complete",
                "trained": _ensemble.is_trained,
            }
        else:
            return {"status": "training_failed", "reason": "not enough data"}

    except Exception as e:
        logger.error(f"Training failed: {e}", exc_info=True)
        return {"error": str(e)}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8002)
