from fastapi import APIRouter, Query
from fastapi.responses import JSONResponse
from app.services.stock_data import fetch_stock_data, get_stock_info, fetch_live_price

router = APIRouter()


@router.get("/{symbol}")
async def get_stock(
    symbol: str,
    period: str = Query("6mo", regex="^(1d|5d|1mo|3mo|6mo|1y|2y|5y)$"),
    interval: str = Query("1d", regex="^(1m|5m|15m|1h|1d|1wk)$"),
):
    try:
        df = await fetch_stock_data(symbol.upper(), period, interval)
        return {
            "symbol": symbol.upper(),
            "data": df.to_dict(orient="records") if not df.empty else [],
            "count": len(df),
        }
    except Exception as e:
        return JSONResponse(status_code=200, content={
            "symbol": symbol.upper(), "data": [], "count": 0, "error": str(e),
        })


@router.get("/{symbol}/live")
async def get_live(symbol: str):
    try:
        return await fetch_live_price(symbol.upper())
    except Exception as e:
        return {"symbol": symbol.upper(), "error": str(e)}


@router.get("/{symbol}/info")
async def get_info(symbol: str):
    try:
        return await get_stock_info(symbol.upper())
    except Exception as e:
        return {"symbol": symbol.upper(), "error": str(e)}
