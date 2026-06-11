"""
ML Agent Models — replaces hardcoded if/else scoring with trained neural networks.

Each agent is a small 3-layer MLP that takes feature vectors and outputs a score
from -1.0 (strong bearish) to +1.0 (strong bullish).

Architecture:
  - MomentumAgent:  Kalman state (momentum, trend_strength, confidence, direction) → score
  - PatternAgent:   Pattern signal history (last 10 signals + confidence) → score
  - FlowAgent:      CVD features (signal, buy_sell_ratio, trend) → score
  - LevelAgent:     VP features (signal, position encoding, price relative to POC) → score
  - SentimentAgent: GEX + COT features (regime, signal, vix proxy) → score
  - MetaAgent:      All 5 agent outputs + Kronos score → final BUY/SELL/HOLD decision

All models are intentionally small (< 10K params each) to:
  1. Train fast on limited data (days, not months)
  2. Avoid overfitting on small $100 portfolio
  3. Run inference in < 1ms per symbol
"""

import os
import json
import torch
import torch.nn as nn
import torch.nn.functional as F
import numpy as np
from typing import Dict, List, Optional, Tuple

MODEL_DIR = "/app/model_cache/agents"


class AgentMLP(nn.Module):
    """Base MLP for all agents. Small and fast."""

    def __init__(self, input_dim: int, hidden_dim: int = 32, name: str = "agent"):
        super().__init__()
        self.name = name
        self.net = nn.Sequential(
            nn.Linear(input_dim, hidden_dim),
            nn.LayerNorm(hidden_dim),
            nn.GELU(),
            nn.Dropout(0.1),
            nn.Linear(hidden_dim, hidden_dim // 2),
            nn.GELU(),
            nn.Linear(hidden_dim // 2, 1),
            nn.Tanh(),  # Output in [-1, 1]
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.net(x).squeeze(-1)


class MomentumAgent(AgentMLP):
    """
    Replaces hardcoded Kalman scoring.
    Input: [momentum, trend_strength, confidence, direction_encoded,
            momentum_building, momentum_fading]
    """
    INPUT_DIM = 6

    def __init__(self):
        super().__init__(input_dim=self.INPUT_DIM, hidden_dim=32, name="momentum")

    @staticmethod
    def encode_features(kalman_momentum: float, kalman_trend_strength: float,
                        kalman_confidence: float, kalman_direction: str,
                        momentum_building: bool, momentum_fading: bool) -> np.ndarray:
        dir_map = {"bullish": 1.0, "neutral": 0.0, "bearish": -1.0}
        return np.array([
            np.clip(kalman_momentum, -2.0, 2.0),
            np.clip(kalman_trend_strength, 0.0, 2.0),
            np.clip(kalman_confidence, 0.0, 1.0),
            dir_map.get(kalman_direction, 0.0),
            float(momentum_building),
            float(momentum_fading),
        ], dtype=np.float32)


class PatternAgent(AgentMLP):
    """
    Replaces hardcoded pattern scoring.
    Input: [current_signal, confidence, last 8 signal history values]
    Total: 10 features
    """
    INPUT_DIM = 10

    def __init__(self):
        super().__init__(input_dim=self.INPUT_DIM, hidden_dim=32, name="pattern")

    @staticmethod
    def encode_features(pattern_signal: float, pattern_confidence: float,
                        signal_history: List[float]) -> np.ndarray:
        # Pad/truncate history to 8 values
        hist = list(signal_history)[-8:]
        while len(hist) < 8:
            hist.insert(0, 0.0)
        features = [
            np.clip(pattern_signal, -1.0, 1.0),
            np.clip(pattern_confidence, 0.0, 1.0),
        ] + [np.clip(s, -1.0, 1.0) for s in hist]
        return np.array(features, dtype=np.float32)


class FlowAgent(AgentMLP):
    """
    Replaces hardcoded CVD scoring.
    Input: [cvd_signal, buy_sell_ratio, cvd_signal_delta (momentum of flow)]
    """
    INPUT_DIM = 3

    def __init__(self):
        super().__init__(input_dim=self.INPUT_DIM, hidden_dim=24, name="flow")

    @staticmethod
    def encode_features(cvd_signal: float, buy_sell_ratio: float,
                        prev_cvd_signal: float = 0.0) -> np.ndarray:
        return np.array([
            np.clip(cvd_signal, -1.0, 1.0),
            np.clip(buy_sell_ratio - 1.0, -1.0, 1.0),  # Center around 0
            np.clip(cvd_signal - prev_cvd_signal, -0.5, 0.5),  # Delta
        ], dtype=np.float32)


class LevelAgent(AgentMLP):
    """
    Replaces hardcoded VP scoring.
    Input: [vp_signal, position_encoded, price_vs_poc_pct]
    """
    INPUT_DIM = 3

    def __init__(self):
        super().__init__(input_dim=self.INPUT_DIM, hidden_dim=24, name="level")

    @staticmethod
    def encode_features(vp_signal: float, vp_position: str,
                        price: float = 0.0, poc_price: float = 0.0) -> np.ndarray:
        pos_map = {
            "above_value": 1.0,
            "in_value": 0.0,
            "at_poc": 0.0,
            "below_value": -1.0,
        }
        price_vs_poc = 0.0
        if poc_price > 0:
            price_vs_poc = np.clip((price - poc_price) / poc_price * 100.0, -2.0, 2.0)

        return np.array([
            np.clip(vp_signal, -1.0, 1.0),
            pos_map.get(vp_position, 0.0),
            price_vs_poc,
        ], dtype=np.float32)


class SentimentAgent(AgentMLP):
    """
    Replaces hardcoded GEX + COT scoring.
    Input: [gex_signal, gex_regime_encoded, cot_signal]
    """
    INPUT_DIM = 3

    def __init__(self):
        super().__init__(input_dim=self.INPUT_DIM, hidden_dim=24, name="sentiment")

    @staticmethod
    def encode_features(gex_signal: float, gex_regime: str,
                        cot_signal: float) -> np.ndarray:
        regime_map = {
            "short_gamma": 1.0,   # High vol, momentum-friendly
            "neutral": 0.0,
            "long_gamma": -0.5,   # Pinned, mean-reverting
        }
        return np.array([
            np.clip(gex_signal, -1.0, 1.0),
            regime_map.get(gex_regime, 0.0),
            np.clip(cot_signal, -1.0, 1.0),
        ], dtype=np.float32)


class MetaAgent(nn.Module):
    """
    Combines all agent outputs into final decision.
    Input: [kronos_score, momentum_score, pattern_score, flow_score,
            level_score, sentiment_score, kronos_confidence]
    Output: score in [-1, 1]

    This learns optimal agent weighting — replaces the fixed 25/25/15/15/8/6/6 split.
    """
    INPUT_DIM = 7

    def __init__(self):
        super().__init__()
        self.name = "meta"
        # Attention-style weighting: learn which agents matter most
        self.agent_attention = nn.Sequential(
            nn.Linear(self.INPUT_DIM, 16),
            nn.GELU(),
            nn.Linear(16, 6),  # 6 agents (Kronos + 5 ML agents)
            nn.Softmax(dim=-1),
        )
        # Combine weighted scores with context
        self.decision = nn.Sequential(
            nn.Linear(self.INPUT_DIM, 16),
            nn.GELU(),
            nn.Linear(16, 1),
            nn.Tanh(),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        # x: [batch, 7] = [kronos, momentum, pattern, flow, level, sentiment, kronos_conf]
        agent_scores = x[:, :6]  # First 6 are agent scores
        weights = self.agent_attention(x)  # Learn dynamic weights
        weighted = (agent_scores * weights).sum(dim=-1, keepdim=True)

        # Also run through decision network for non-linear combinations
        decision = self.decision(x)

        # Blend attention-weighted and decision network
        final = 0.5 * weighted + 0.5 * decision.squeeze(-1)
        return torch.tanh(final).squeeze(-1)

    @staticmethod
    def encode_features(kronos_score: float, momentum_score: float,
                        pattern_score: float, flow_score: float,
                        level_score: float, sentiment_score: float,
                        kronos_confidence: float = 0.5) -> np.ndarray:
        return np.array([
            kronos_score, momentum_score, pattern_score,
            flow_score, level_score, sentiment_score,
            np.clip(kronos_confidence, 0.0, 1.0),
        ], dtype=np.float32)


class AgentEnsemble:
    """Manages all agent models — load, save, inference."""

    def __init__(self, device: str = "cpu"):
        self.device = device
        self.momentum = MomentumAgent().to(device)
        self.pattern = PatternAgent().to(device)
        self.flow = FlowAgent().to(device)
        self.level = LevelAgent().to(device)
        self.sentiment = SentimentAgent().to(device)
        self.meta = MetaAgent().to(device)

        self.agents = {
            "momentum": self.momentum,
            "pattern": self.pattern,
            "flow": self.flow,
            "level": self.level,
            "sentiment": self.sentiment,
            "meta": self.meta,
        }
        self._trained = False

    def save(self, path: str = MODEL_DIR):
        os.makedirs(path, exist_ok=True)
        for name, model in self.agents.items():
            torch.save(model.state_dict(), os.path.join(path, f"{name}.pt"))

        meta = {
            "trained": self._trained,
            "agent_names": list(self.agents.keys()),
            "param_counts": {n: sum(p.numel() for p in m.parameters())
                             for n, m in self.agents.items()},
        }
        with open(os.path.join(path, "ensemble_meta.json"), "w") as f:
            json.dump(meta, f, indent=2)

    def load(self, path: str = MODEL_DIR) -> bool:
        if not os.path.exists(path):
            return False
        try:
            for name, model in self.agents.items():
                fpath = os.path.join(path, f"{name}.pt")
                if os.path.exists(fpath):
                    model.load_state_dict(torch.load(fpath, map_location=self.device, weights_only=True))
            self._trained = True
            return True
        except Exception as e:
            print(f"Failed to load agents: {e}")
            return False

    @property
    def is_trained(self) -> bool:
        return self._trained

    @torch.no_grad()
    def predict(self, features: Dict[str, np.ndarray], kronos_score: float = 0.0,
                kronos_confidence: float = 0.5) -> Dict:
        """
        Run all agents and return scores.

        features: dict with keys matching agent names, values are encoded feature arrays
        kronos_score: from the existing Kronos transformer (passed through, not replaced)
        """
        # Set all to eval mode
        for m in self.agents.values():
            m.eval()

        results = {}

        # Run individual agents
        for agent_name in ["momentum", "pattern", "flow", "level", "sentiment"]:
            if agent_name in features:
                x = torch.from_numpy(features[agent_name]).unsqueeze(0).to(self.device)
                score = self.agents[agent_name](x).item()
                results[agent_name] = round(score, 4)
            else:
                results[agent_name] = 0.0

        # Run meta agent
        meta_features = MetaAgent.encode_features(
            kronos_score=kronos_score,
            momentum_score=results.get("momentum", 0.0),
            pattern_score=results.get("pattern", 0.0),
            flow_score=results.get("flow", 0.0),
            level_score=results.get("level", 0.0),
            sentiment_score=results.get("sentiment", 0.0),
            kronos_confidence=kronos_confidence,
        )
        x_meta = torch.from_numpy(meta_features).unsqueeze(0).to(self.device)
        final_score = self.meta(x_meta).item()

        results["meta_score"] = round(final_score, 4)
        results["kronos_score"] = round(kronos_score, 4)
        results["is_trained"] = self._trained

        # Decision
        if final_score > 0.1:
            results["action"] = "BUY"
        elif final_score < -0.1:
            results["action"] = "SELL"
        else:
            results["action"] = "HOLD"

        # Count bullish/bearish agents
        all_scores = {
            "Kronos": kronos_score,
            "Momentum": results.get("momentum", 0.0),
            "Pattern": results.get("pattern", 0.0),
            "Flow": results.get("flow", 0.0),
            "Level": results.get("level", 0.0),
            "Sentiment": results.get("sentiment", 0.0),
        }
        results["bullish_agents"] = [n for n, s in all_scores.items() if s > 0.1]
        results["bearish_agents"] = [n for n, s in all_scores.items() if s < -0.1]

        return results
