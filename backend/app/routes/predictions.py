from fastapi import APIRouter, Query
from app.services.stock_data import fetch_stock_data
from app.services.predictor import predict_next_candle, predict_multi_step
from app.services.kronos_predictor import kronos_predict, kronos_status, reload_kronos
from app.services.cache import redis_client

router = APIRouter()


@router.get("/kronos/status")
async def get_kronos_status():
    """Check if the Kronos foundation model is loaded and ready."""
    return kronos_status()


@router.post("/kronos/reload")
async def reload_kronos_model():
    """Reload Kronos model (picks up fine-tuned weights if available)."""
    success = reload_kronos()
    status = kronos_status()
    status["reloaded"] = success
    return status


@router.get("/{symbol}/kronos")
async def get_kronos_prediction(symbol: str, steps: int = Query(7, ge=1, le=14)):
    """Get Kronos foundation model prediction for a symbol."""
    symbol = symbol.upper()
    cached = await redis_client.get_json(f"kronos:{symbol}:{steps}")
    if cached:
        return cached

    try:
        df = await fetch_stock_data(symbol, period="1y", interval="1d")
        if df.empty:
            return {"error": "No data available", "symbol": symbol}

        result = kronos_predict(df, steps=steps)
        result["symbol"] = symbol

        if "error" not in result:
            await redis_client.set_json(f"kronos:{symbol}:{steps}", result, ttl=300)

        return result
    except Exception as e:
        return {"error": str(e), "symbol": symbol}


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
