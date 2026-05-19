from fastapi import APIRouter, Query
from app.services.stock_data import fetch_stock_data
from app.services.predictor import predict_next_candle, predict_multi_step
from app.services.cache import redis_client

router = APIRouter()


@router.get("/{symbol}")
async def get_prediction(symbol: str):
    symbol = symbol.upper()
    cached = await redis_client.get_json(f"prediction:{symbol}")
    if cached:
        return cached

    try:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"error": "No data available", "symbol": symbol}
        result = predict_next_candle(df)
        result["symbol"] = symbol
        if "error" not in result:
            await redis_client.set_json(f"prediction:{symbol}", result, ttl=300)
        return result
    except Exception as e:
        return {"error": str(e), "symbol": symbol}


@router.get("/{symbol}/multi")
async def get_multi_prediction(symbol: str, steps: int = Query(5, ge=1, le=10)):
    symbol = symbol.upper()
    try:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"symbol": symbol, "predictions": []}
        results = predict_multi_step(df, steps)
        return {"symbol": symbol, "predictions": results}
    except Exception as e:
        return {"symbol": symbol, "predictions": [], "error": str(e)}
