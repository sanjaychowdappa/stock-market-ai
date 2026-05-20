import logging
from datetime import datetime
from app.services.news_fetcher import fetch_stock_news, fetch_market_news
from app.services.sentiment_analyzer import analyze_articles
from app.services.stock_data import fetch_stock_data, fetch_live_price
from app.services.signal_generator import generate_signals
from app.services.predictor import predict_next_candle
from app.services.cache import redis_client

logger = logging.getLogger(__name__)

WATCHLIST = ["AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "NVDA", "META", "SPY", "QQQ", "AMD"]


class NewsAgent:
    def __init__(self):
        self.watchlist = WATCHLIST.copy()

    async def generate_daily_report(self, symbol: str) -> dict:
        cache_key = f"report:{symbol}"
        cached = await redis_client.get_json(cache_key)
        if cached:
            return cached

        news = await fetch_stock_news(symbol)
        sentiment = analyze_articles(news)

        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        signals = generate_signals(df) if not df.empty else {}
        prediction = predict_next_candle(df) if not df.empty else {}

        live = await fetch_live_price(symbol)

        recommendation = self._build_recommendation(
            symbol, sentiment, signals, prediction, live
        )

        report = {
            "symbol": symbol,
            "generated_at": datetime.now().isoformat(),
            "live_price": live,
            "news_sentiment": sentiment,
            "technical_signal": {
                "action": signals.get("signal", "N/A"),
                "score": signals.get("score", 0),
                "confidence": signals.get("confidence", 0),
                "key_reasons": signals.get("reasons", [])[:5],
            },
            "ml_prediction": {
                "direction": prediction.get("direction", "N/A"),
                "change_percent": prediction.get("change_percent", 0),
                "confidence": prediction.get("confidence", 0),
                "predicted_close": prediction.get("predicted_close", 0),
            } if "error" not in prediction else {"error": prediction.get("error")},
            "recommendation": recommendation,
            "news_articles": news[:10],
        }

        await redis_client.set_json(cache_key, report, ttl=600)
        return report

    def _build_recommendation(self, symbol, sentiment, signals, prediction, live):
        scores = {
            "news": sentiment.get("overall_score", 0),
            "technical": signals.get("score", 0),
            "prediction": 0,
        }
        weights = {
            "news": 0.30,
            "technical": 0.45,
            "prediction": 0.25,
        }

        if "error" not in prediction and prediction.get("change_percent"):
            change = prediction["change_percent"]
            scores["prediction"] = max(-1, min(1, change / 3)) * prediction.get("confidence", 0.5)

        composite = sum(scores[k] * weights[k] for k in scores)

        if composite > 0.4:
            action = "STRONG BUY"
            risk = "low"
        elif composite > 0.15:
            action = "BUY"
            risk = "moderate"
        elif composite < -0.4:
            action = "STRONG SELL"
            risk = "low"
        elif composite < -0.15:
            action = "SELL"
            risk = "moderate"
        else:
            action = "HOLD"
            risk = "high" if abs(composite) < 0.05 else "moderate"

        news_label = sentiment.get("overall_sentiment", "neutral")
        tech_signal = signals.get("signal", "N/A")
        pred_dir = prediction.get("direction", "N/A") if "error" not in prediction else "N/A"

        alignment = _check_alignment(news_label, tech_signal, pred_dir)

        reasons = []

        if news_label == "bullish":
            bull_count = sentiment.get("breakdown", {}).get("bullish", 0)
            total = sentiment.get("breakdown", {}).get("total", 0)
            reasons.append(f"News sentiment is bullish ({bull_count}/{total} articles positive)")
        elif news_label == "bearish":
            bear_count = sentiment.get("breakdown", {}).get("bearish", 0)
            total = sentiment.get("breakdown", {}).get("total", 0)
            reasons.append(f"News sentiment is bearish ({bear_count}/{total} articles negative)")
        else:
            reasons.append("News sentiment is neutral/mixed")

        if tech_signal in ("BUY", "SELL"):
            tech_reasons = signals.get("reasons", [])[:2]
            reasons.append(f"Technical analysis says {tech_signal}: {'; '.join(tech_reasons)}")

        if "error" not in prediction:
            reasons.append(f"ML models predict {pred_dir} move ({prediction.get('change_percent', 0):+.2f}%)")

        if alignment == "aligned":
            reasons.append("All signals aligned — higher confidence")
        elif alignment == "conflicting":
            reasons.append("Signals are conflicting — exercise caution")

        stop_loss_info = signals.get("stop_loss", {})
        take_profit_info = signals.get("take_profit", {})

        return {
            "action": action,
            "composite_score": round(composite, 3),
            "risk_level": risk,
            "signal_alignment": alignment,
            "component_scores": {k: round(v, 3) for k, v in scores.items()},
            "component_weights": weights,
            "reasons": reasons,
            "stop_loss": stop_loss_info.get("recommended"),
            "take_profit_targets": [t["price"] for t in take_profit_info.get("targets", [])],
            "current_price": live.get("price", 0) if isinstance(live, dict) else 0,
        }

    async def generate_watchlist_report(self) -> dict:
        cache_key = "report:watchlist"
        cached = await redis_client.get_json(cache_key)
        if cached:
            return cached

        reports = []
        for symbol in self.watchlist:
            try:
                report = await self.generate_daily_report(symbol)
                rec = report.get("recommendation", {})
                reports.append({
                    "symbol": symbol,
                    "action": rec.get("action", "HOLD"),
                    "composite_score": rec.get("composite_score", 0),
                    "risk_level": rec.get("risk_level", "high"),
                    "alignment": rec.get("signal_alignment", "unknown"),
                    "news_sentiment": report.get("news_sentiment", {}).get("overall_sentiment", "neutral"),
                    "tech_signal": report.get("technical_signal", {}).get("action", "N/A"),
                    "price": rec.get("current_price", 0),
                    "reasons": rec.get("reasons", [])[:2],
                })
            except Exception as e:
                logger.error(f"Failed report for {symbol}: {e}")
                reports.append({"symbol": symbol, "action": "HOLD", "error": str(e)})

        buy_picks = [r for r in reports if "BUY" in r.get("action", "")]
        sell_picks = [r for r in reports if "SELL" in r.get("action", "")]

        buy_picks.sort(key=lambda x: x.get("composite_score", 0), reverse=True)
        sell_picks.sort(key=lambda x: x.get("composite_score", 0))

        result = {
            "generated_at": datetime.now().isoformat(),
            "total_analyzed": len(reports),
            "top_buys": buy_picks[:5],
            "top_sells": sell_picks[:5],
            "all_stocks": sorted(reports, key=lambda x: abs(x.get("composite_score", 0)), reverse=True),
            "market_summary": _generate_market_summary(reports),
        }

        await redis_client.set_json(cache_key, result, ttl=600)
        return result


def _check_alignment(news, technical, prediction):
    signals = []
    if news == "bullish":
        signals.append(1)
    elif news == "bearish":
        signals.append(-1)
    else:
        signals.append(0)

    if technical in ("BUY",):
        signals.append(1)
    elif technical in ("SELL",):
        signals.append(-1)
    else:
        signals.append(0)

    if prediction == "bullish":
        signals.append(1)
    elif prediction == "bearish":
        signals.append(-1)
    else:
        signals.append(0)

    nonzero = [s for s in signals if s != 0]
    if len(nonzero) >= 2 and all(s == nonzero[0] for s in nonzero):
        return "aligned"
    elif len(nonzero) >= 2 and any(s != nonzero[0] for s in nonzero):
        return "conflicting"
    return "neutral"


def _generate_market_summary(reports):
    buy_count = sum(1 for r in reports if "BUY" in r.get("action", ""))
    sell_count = sum(1 for r in reports if "SELL" in r.get("action", ""))
    hold_count = sum(1 for r in reports if r.get("action") == "HOLD")
    bullish_news = sum(1 for r in reports if r.get("news_sentiment") == "bullish")
    bearish_news = sum(1 for r in reports if r.get("news_sentiment") == "bearish")

    if buy_count > sell_count * 2:
        mood = "Strongly bullish"
    elif buy_count > sell_count:
        mood = "Moderately bullish"
    elif sell_count > buy_count * 2:
        mood = "Strongly bearish"
    elif sell_count > buy_count:
        mood = "Moderately bearish"
    else:
        mood = "Mixed/Neutral"

    return {
        "mood": mood,
        "buys": buy_count,
        "sells": sell_count,
        "holds": hold_count,
        "bullish_news_count": bullish_news,
        "bearish_news_count": bearish_news,
    }
