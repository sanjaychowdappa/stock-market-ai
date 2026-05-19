from fastapi import APIRouter, Query
from app.services.stock_data import fetch_stock_data
from app.services.signal_generator import generate_signals, generate_stop_loss_report
from app.services.cache import redis_client
from app.agents.market_agent import MarketAgent

router = APIRouter()
agent = MarketAgent()


@router.get("/{symbol}")
async def get_signals(symbol: str):
    symbol = symbol.upper()
    try:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"signal": "HOLD", "score": 0, "confidence": 0, "reasons": ["No data available"], "individual_signals": [], "stop_loss": {}, "take_profit": {"targets": []}, "current_price": 0}
        return generate_signals(df)
    except Exception as e:
        return {"signal": "HOLD", "score": 0, "confidence": 0, "reasons": [str(e)], "individual_signals": [], "stop_loss": {}, "take_profit": {"targets": []}, "current_price": 0}


@router.get("/{symbol}/stop-loss")
async def get_stop_loss(
    symbol: str,
    entry_price: float = Query(None),
    position_type: str = Query("long", regex="^(long|short)$"),
):
    symbol = symbol.upper()
    try:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"error": "No data available"}
        return generate_stop_loss_report(df, entry_price, position_type)
    except Exception as e:
        return {"error": str(e)}


@router.get("/{symbol}/full-analysis")
async def get_full_analysis(symbol: str):
    symbol = symbol.upper()
    cached = await redis_client.get_json(f"analysis:{symbol}")
    if cached:
        return cached

    try:
        result = await agent.analyze_stock(symbol)
        await redis_client.set_json(f"analysis:{symbol}", result, ttl=300)
        return result
    except Exception as e:
        return {"symbol": symbol, "error": str(e)}


@router.get("/rankings/top")
async def get_rankings():
    cached = await redis_client.get_json("rankings:latest")
    if cached:
        return {"rankings": cached}
    return {"rankings": [], "message": "No rankings yet. Agent cycle will run shortly."}


@router.post("/watchlist/{symbol}")
async def add_watchlist(symbol: str):
    await agent.add_to_watchlist(symbol.upper())
    return {"message": f"{symbol.upper()} added to watchlist"}


@router.delete("/watchlist/{symbol}")
async def remove_watchlist(symbol: str):
    await agent.remove_from_watchlist(symbol.upper())
    return {"message": f"{symbol.upper()} removed from watchlist"}
