from fastapi import APIRouter
from app.services.news_fetcher import fetch_stock_news, fetch_market_news
from app.services.sentiment_analyzer import analyze_articles
from app.agents.news_agent import NewsAgent

router = APIRouter()
news_agent = NewsAgent()


@router.get("/watchlist/report")
async def get_watchlist_report():
    try:
        return await news_agent.generate_watchlist_report()
    except Exception as e:
        return {"error": str(e)}


@router.get("/market/news")
async def get_market_news():
    try:
        articles = await fetch_market_news()
        return {"articles": articles, "count": len(articles)}
    except Exception as e:
        return {"articles": [], "count": 0, "error": str(e)}


@router.get("/{symbol}")
async def get_news(symbol: str):
    symbol = symbol.upper()
    try:
        articles = await fetch_stock_news(symbol)
        return {"symbol": symbol, "articles": articles, "count": len(articles)}
    except Exception as e:
        return {"symbol": symbol, "articles": [], "count": 0, "error": str(e)}


@router.get("/{symbol}/sentiment")
async def get_sentiment(symbol: str):
    symbol = symbol.upper()
    try:
        articles = await fetch_stock_news(symbol)
        sentiment = analyze_articles(articles)
        return {"symbol": symbol, **sentiment}
    except Exception as e:
        return {"symbol": symbol, "overall_sentiment": "neutral", "error": str(e)}


@router.get("/{symbol}/report")
async def get_report(symbol: str):
    symbol = symbol.upper()
    try:
        return await news_agent.generate_daily_report(symbol)
    except Exception as e:
        return {"symbol": symbol, "error": str(e)}
