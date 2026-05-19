import asyncio
import logging
from datetime import datetime
from app.services.stock_data import fetch_stock_data, get_stock_info
from app.services.predictor import predict_next_candle
from app.services.signal_generator import generate_signals, generate_stop_loss_report
from app.services.pattern_recognition import detect_all_patterns
from app.services.cache import redis_client

logger = logging.getLogger(__name__)

WATCHLIST = ["AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "NVDA", "META", "SPY", "QQQ", "AMD"]


class MarketAgent:
    def __init__(self):
        self.watchlist = WATCHLIST.copy()
        self.analysis_results = {}

    async def run_analysis_cycle(self):
        logger.info(f"Starting analysis cycle at {datetime.now()}")
        for symbol in self.watchlist:
            try:
                result = await self.analyze_stock(symbol)
                self.analysis_results[symbol] = result
                await redis_client.set_json(f"analysis:{symbol}", result, ttl=600)
            except Exception as e:
                logger.error(f"Error analyzing {symbol}: {e}")
            await asyncio.sleep(1)

        rankings = self._rank_opportunities()
        await redis_client.set_json("rankings:latest", rankings, ttl=600)
        logger.info(f"Analysis cycle complete. Top pick: {rankings[0]['symbol'] if rankings else 'None'}")

    async def analyze_stock(self, symbol: str) -> dict:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"symbol": symbol, "error": "No data available", "agent_recommendation": {"action": "HOLD", "score": 0, "summary": f"{symbol}: No data available."}}

        info = await get_stock_info(symbol)

        prediction = predict_next_candle(df)
        signals = generate_signals(df)
        patterns = detect_all_patterns(df)
        stop_loss = generate_stop_loss_report(df)

        recent_patterns = [p for p in patterns if p["index"] >= len(df) - 5]

        bullish_patterns = sum(1 for p in recent_patterns if p["type"] == "bullish")
        bearish_patterns = sum(1 for p in recent_patterns if p["type"] == "bearish")

        agent_score = self._compute_composite_score(prediction, signals, bullish_patterns, bearish_patterns)

        if agent_score > 0.5:
            agent_action = "STRONG BUY"
        elif agent_score > 0.2:
            agent_action = "BUY"
        elif agent_score < -0.5:
            agent_action = "STRONG SELL"
        elif agent_score < -0.2:
            agent_action = "SELL"
        else:
            agent_action = "HOLD"

        return {
            "symbol": symbol,
            "info": info,
            "prediction": prediction,
            "signals": signals,
            "patterns": recent_patterns,
            "stop_loss": stop_loss,
            "agent_recommendation": {
                "action": agent_action,
                "score": round(agent_score, 3),
                "summary": self._generate_summary(symbol, agent_action, prediction, signals, recent_patterns),
            },
            "timestamp": datetime.now().isoformat(),
        }

    def _compute_composite_score(self, prediction, signals, bullish_count, bearish_count) -> float:
        pred_score = 0
        if "error" not in prediction:
            change = prediction.get("change_percent", 0)
            pred_score = max(-1, min(1, change / 3)) * prediction.get("confidence", 0.5)

        signal_score = signals.get("score", 0)

        pattern_score = (bullish_count - bearish_count) * 0.15

        return pred_score * 0.35 + signal_score * 0.45 + pattern_score * 0.20

    def _generate_summary(self, symbol, action, prediction, signals, patterns) -> str:
        parts = [f"{symbol}: {action}."]

        if "error" not in prediction:
            direction = prediction.get("direction", "neutral")
            change = prediction.get("change_percent", 0)
            parts.append(f"ML models predict {direction} move ({change:+.2f}%).")

        if signals.get("reasons"):
            top_reasons = signals["reasons"][:3]
            parts.append("Key signals: " + "; ".join(top_reasons) + ".")

        if patterns:
            pattern_names = [p["name"] for p in patterns[:3]]
            parts.append(f"Recent patterns: {', '.join(pattern_names)}.")

        sl = signals.get("stop_loss", {})
        if sl.get("recommended"):
            parts.append(f"Stop loss: ${sl['recommended']} ({sl['risk_percent']:.1f}% risk).")

        return " ".join(parts)

    def _rank_opportunities(self) -> list[dict]:
        rankings = []
        for symbol, result in self.analysis_results.items():
            rec = result.get("agent_recommendation", {})
            rankings.append({
                "symbol": symbol,
                "action": rec.get("action", "HOLD"),
                "score": rec.get("score", 0),
                "summary": rec.get("summary", ""),
                "current_price": result.get("signals", {}).get("current_price", 0),
            })

        rankings.sort(key=lambda x: abs(x["score"]), reverse=True)
        return rankings

    async def add_to_watchlist(self, symbol: str):
        symbol = symbol.upper()
        if symbol not in self.watchlist:
            self.watchlist.append(symbol)
            await self.analyze_stock(symbol)

    async def remove_from_watchlist(self, symbol: str):
        symbol = symbol.upper()
        if symbol in self.watchlist:
            self.watchlist.remove(symbol)
            self.analysis_results.pop(symbol, None)
