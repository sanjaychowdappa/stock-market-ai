"""
Real-time prediction routes — 1-minute Kronos + pattern scoring.

Endpoints:
  GET  /api/realtime/{symbol}/predict   — next 1-5 minute predictions
  GET  /api/realtime/{symbol}/patterns  — current pattern analysis
  GET  /api/realtime/{symbol}/candles   — raw 1-min candle history
"""

from fastapi import APIRouter, Query
from app.services.stock_data import fetch_stock_data
from app.services.kronos_realtime import kronos_predict_with_patterns
from app.services.pattern_scorer import compute_pattern_score
from app.services.cache import redis_client

router = APIRouter()


@router.get("/{symbol}/predict")
async def realtime_predict(symbol: str, steps: int = Query(1, ge=1, le=5)):
    """
    Predict the next `steps` 1-minute candles using Kronos + pattern scoring.
    """
    symbol = symbol.upper()

    # Short cache — 10 seconds for 1-min predictions
    cache_key = f"rt_pred:{symbol}:{steps}"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    try:
        df = await fetch_stock_data(symbol, period="1d", interval="1m")
        if df.empty or len(df) < 30:
            return {"error": "Not enough 1-min data (need market hours)", "symbol": symbol}

        result = kronos_predict_with_patterns(df, steps=steps)
        result["symbol"] = symbol
        result["timeframe"] = "1min"

        if "error" not in result:
            await redis_client.set_json(cache_key, result, ttl=10)

        return result
    except Exception as e:
        return {"error": str(e), "symbol": symbol}


@router.get("/{symbol}/patterns")
async def realtime_patterns(symbol: str):
    """
    Get current pattern analysis for 1-min candle data.
    Includes repeating pattern matches, S/R levels, momentum regime.
    """
    symbol = symbol.upper()

    cache_key = f"rt_patterns:{symbol}"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    try:
        df = await fetch_stock_data(symbol, period="1d", interval="1m")
        if df.empty or len(df) < 30:
            return {"error": "Not enough 1-min data", "symbol": symbol}

        result = compute_pattern_score(df)
        result["symbol"] = symbol
        result["candles_analysed"] = len(df)

        await redis_client.set_json(cache_key, result, ttl=10)
        return result
    except Exception as e:
        return {"error": str(e), "symbol": symbol}


@router.get("/{symbol}/candles")
async def realtime_candles(symbol: str, count: int = Query(120, ge=30, le=390)):
    """
    Get the latest 1-minute candles for the live chart.
    """
    symbol = symbol.upper()
    try:
        df = await fetch_stock_data(symbol, period="1d", interval="1m")
        if df.empty:
            return {"symbol": symbol, "candles": [], "count": 0}

        data = df.tail(count)
        candles = data.to_dict(orient="records")
        return {
            "symbol": symbol,
            "candles": candles,
            "count": len(candles),
        }
    except Exception as e:
        return {"symbol": symbol, "candles": [], "count": 0, "error": str(e)}
