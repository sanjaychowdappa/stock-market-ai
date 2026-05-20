import re
import logging

logger = logging.getLogger(__name__)

BULLISH_WORDS = {
    "surge", "surges", "surging", "soar", "soars", "soaring", "rally", "rallies",
    "rallying", "jump", "jumps", "jumping", "gain", "gains", "gaining", "rise",
    "rises", "rising", "climb", "climbs", "climbing", "boost", "boosts", "boosting",
    "upgrade", "upgrades", "upgraded", "outperform", "buy", "bullish", "breakout",
    "upbeat", "optimistic", "strong", "strength", "growth", "growing", "beat",
    "beats", "beating", "exceeded", "exceeds", "record", "high", "highs",
    "positive", "profit", "profits", "profitable", "revenue", "expand",
    "expansion", "innovation", "opportunity", "momentum", "recovery", "recover",
    "rebound", "rebounds", "rebounding", "accelerate", "accelerating",
    "dividend", "buyback", "outpace", "overweight", "upside", "breakthrough",
    "approve", "approved", "launch", "launches", "partnership", "deal",
}

BEARISH_WORDS = {
    "fall", "falls", "falling", "drop", "drops", "dropping", "plunge", "plunges",
    "plunging", "decline", "declines", "declining", "sink", "sinks", "sinking",
    "crash", "crashes", "crashing", "tumble", "tumbles", "tumbling", "slide",
    "slides", "sliding", "slump", "slumps", "slumping", "downgrade", "downgrades",
    "downgraded", "underperform", "sell", "bearish", "breakdown", "weak",
    "weakness", "loss", "losses", "losing", "miss", "misses", "missed",
    "warning", "warns", "risk", "risks", "risky", "fear", "fears", "concern",
    "concerns", "worried", "trouble", "troubled", "struggling", "struggle",
    "recession", "inflation", "layoff", "layoffs", "cut", "cuts", "cutting",
    "debt", "lawsuit", "fine", "fined", "penalty", "investigation", "probe",
    "recall", "recalls", "shortage", "shortages", "tariff", "tariffs",
    "underweight", "downside", "overvalued", "bubble", "default",
}

INTENSITY_WORDS = {
    "massive", "huge", "significant", "dramatically", "sharply", "substantially",
    "extremely", "remarkably", "unprecedented", "historic", "major",
}


def analyze_sentiment(text: str) -> dict:
    if not text:
        return {"score": 0, "label": "neutral", "confidence": 0}

    text_lower = text.lower()
    words = set(re.findall(r'\b[a-z]+\b', text_lower))

    bull_matches = words & BULLISH_WORDS
    bear_matches = words & BEARISH_WORDS
    intensity_matches = words & INTENSITY_WORDS

    bull_count = len(bull_matches)
    bear_count = len(bear_matches)
    intensity = 1 + len(intensity_matches) * 0.3

    total = bull_count + bear_count
    if total == 0:
        return {"score": 0, "label": "neutral", "confidence": 0.2, "bull_words": [], "bear_words": []}

    raw_score = (bull_count - bear_count) / total
    score = max(-1, min(1, raw_score * intensity))

    if score > 0.2:
        label = "bullish"
    elif score < -0.2:
        label = "bearish"
    else:
        label = "neutral"

    confidence = min(1.0, (total / 5) * 0.5 + abs(score) * 0.5)

    return {
        "score": round(score, 3),
        "label": label,
        "confidence": round(confidence, 2),
        "bull_words": sorted(bull_matches),
        "bear_words": sorted(bear_matches),
    }


def analyze_articles(articles: list[dict]) -> dict:
    if not articles:
        return {
            "overall_sentiment": "neutral",
            "overall_score": 0,
            "confidence": 0,
            "article_sentiments": [],
            "summary": "No news articles available for analysis.",
        }

    sentiments = []
    for article in articles:
        text = f"{article.get('title', '')} {article.get('summary', '')}"
        sentiment = analyze_sentiment(text)
        sentiments.append({
            "title": article.get("title", ""),
            "provider": article.get("provider", ""),
            "published": article.get("published", ""),
            "url": article.get("url", ""),
            **sentiment,
        })

    scores = [s["score"] for s in sentiments]
    confidences = [s["confidence"] for s in sentiments]

    weighted_score = sum(s * c for s, c in zip(scores, confidences)) / max(sum(confidences), 0.01)

    if weighted_score > 0.15:
        overall = "bullish"
    elif weighted_score < -0.15:
        overall = "bearish"
    else:
        overall = "neutral"

    bull_count = sum(1 for s in sentiments if s["label"] == "bullish")
    bear_count = sum(1 for s in sentiments if s["label"] == "bearish")
    neutral_count = sum(1 for s in sentiments if s["label"] == "neutral")

    return {
        "overall_sentiment": overall,
        "overall_score": round(weighted_score, 3),
        "confidence": round(sum(confidences) / len(confidences), 2),
        "breakdown": {
            "bullish": bull_count,
            "bearish": bear_count,
            "neutral": neutral_count,
            "total": len(sentiments),
        },
        "article_sentiments": sentiments,
    }
