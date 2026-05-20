import yfinance as yf
import logging
from datetime import datetime
from app.services.cache import redis_client

logger = logging.getLogger(__name__)


async def fetch_stock_news(symbol: str) -> list[dict]:
    cache_key = f"news:{symbol}"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    try:
        ticker = yf.Ticker(symbol)
        raw_news = ticker.news
    except Exception as e:
        logger.error(f"Failed to fetch news for {symbol}: {e}")
        return []

    if not raw_news or not isinstance(raw_news, list):
        return []

    articles = []
    for item in raw_news:
        content = item.get("content", item)
        if isinstance(content, dict):
            title = content.get("title", "")
            summary = content.get("summary", content.get("description", ""))
            pub_date = content.get("pubDate", "")
            provider = content.get("provider", {})
            provider_name = provider.get("displayName", "Unknown") if isinstance(provider, dict) else "Unknown"
            canonical = content.get("canonicalUrl", {})
            url = canonical.get("url", "") if isinstance(canonical, dict) else ""
            content_type = content.get("contentType", "STORY")
        else:
            title = item.get("title", "")
            summary = item.get("summary", "")
            pub_date = item.get("pubDate", "")
            provider_name = item.get("publisher", "Unknown")
            url = item.get("link", "")
            content_type = "STORY"

        if summary:
            import re
            summary = re.sub(r'<[^>]+>', '', summary).strip()

        if title:
            articles.append({
                "title": title,
                "summary": summary,
                "url": url,
                "provider": provider_name,
                "published": pub_date,
                "type": content_type,
            })

    await redis_client.set_json(cache_key, articles, ttl=300)
    return articles


async def fetch_market_news() -> list[dict]:
    cache_key = "news:market"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    market_symbols = ["SPY", "QQQ", "DIA"]
    all_news = []
    seen_titles = set()

    for sym in market_symbols:
        articles = await fetch_stock_news(sym)
        for a in articles:
            if a["title"] not in seen_titles:
                seen_titles.add(a["title"])
                a["related_symbol"] = sym
                all_news.append(a)

    all_news.sort(key=lambda x: x.get("published", ""), reverse=True)
    await redis_client.set_json(cache_key, all_news, ttl=300)
    return all_news
