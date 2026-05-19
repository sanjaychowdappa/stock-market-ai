from fastapi import APIRouter
from app.services.stock_data import fetch_stock_data
from app.services.pattern_recognition import detect_all_patterns

router = APIRouter()


@router.get("/{symbol}")
async def get_patterns(symbol: str):
    symbol = symbol.upper()
    try:
        df = await fetch_stock_data(symbol, period="6mo", interval="1d")
        if df.empty:
            return {"symbol": symbol, "total_patterns": 0, "patterns": [], "summary": {"bullish": 0, "bearish": 0, "neutral": 0}}
        patterns = detect_all_patterns(df)
        return {
            "symbol": symbol,
            "total_patterns": len(patterns),
            "patterns": patterns,
            "summary": {
                "bullish": sum(1 for p in patterns if p["type"] == "bullish"),
                "bearish": sum(1 for p in patterns if p["type"] == "bearish"),
                "neutral": sum(1 for p in patterns if p["type"] == "neutral"),
            },
        }
    except Exception as e:
        return {"symbol": symbol, "total_patterns": 0, "patterns": [], "summary": {"bullish": 0, "bearish": 0, "neutral": 0}, "error": str(e)}
