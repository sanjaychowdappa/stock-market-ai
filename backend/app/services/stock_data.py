import yfinance as yf
import pandas as pd
import ta
from app.services.cache import redis_client

CACHE_TTL = {
    "1m": 15, "5m": 30, "15m": 60,
    "1h": 120, "1d": 300, "1wk": 600,
}


async def fetch_stock_data(symbol: str, period: str = "6mo", interval: str = "1d") -> pd.DataFrame:
    cache_key = f"stock:{symbol}:{period}:{interval}"
    ttl = CACHE_TTL.get(interval, 120)
    cached = await redis_client.get_json(cache_key)
    if cached:
        return pd.DataFrame(cached)

    ticker = yf.Ticker(symbol)
    df = ticker.history(period=period, interval=interval)

    if df.empty:
        return df

    df.reset_index(inplace=True)

    date_col = "Date" if "Date" in df.columns else "Datetime" if "Datetime" in df.columns else None
    if date_col:
        if date_col != "Date":
            df.rename(columns={date_col: "Date"}, inplace=True)
        if interval in ("1m", "5m", "15m", "1h"):
            df["Date"] = pd.to_datetime(df["Date"], utc=True).dt.strftime("%Y-%m-%dT%H:%M")
        else:
            df["Date"] = pd.to_datetime(df["Date"], utc=True).dt.strftime("%Y-%m-%d")

    df = add_technical_indicators(df)

    await redis_client.set_json(cache_key, df.to_dict(orient="records"), ttl=ttl)
    return df


async def fetch_live_price(symbol: str) -> dict:
    cache_key = f"live:{symbol}"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    ticker = yf.Ticker(symbol)
    df = ticker.history(period="1d", interval="1m")
    if df.empty:
        return {"error": "No live data"}

    last = df.iloc[-1]
    today_open = float(df.iloc[0]["Open"])
    current = float(last["Close"])
    day_high = float(df["High"].max())
    day_low = float(df["Low"].min())
    total_volume = int(df["Volume"].sum())
    change = current - today_open
    change_pct = (change / today_open) * 100 if today_open else 0

    result = {
        "symbol": symbol,
        "price": round(current, 2),
        "open": round(today_open, 2),
        "high": round(day_high, 2),
        "low": round(day_low, 2),
        "volume": total_volume,
        "change": round(change, 2),
        "change_percent": round(change_pct, 2),
        "timestamp": pd.to_datetime(last.name, utc=True).strftime("%Y-%m-%d %H:%M:%S") if hasattr(last, 'name') else df["Date"].iloc[-1] if "Date" in df.columns else "",
        "candles_today": len(df),
    }

    await redis_client.set_json(cache_key, result, ttl=15)
    return result


def add_technical_indicators(df: pd.DataFrame) -> pd.DataFrame:
    n = len(df)
    if n < 20:
        return df

    df["SMA_20"] = ta.trend.sma_indicator(df["Close"], window=20)
    df["EMA_12"] = ta.trend.ema_indicator(df["Close"], window=12)
    df["EMA_26"] = ta.trend.ema_indicator(df["Close"], window=26)
    df["RSI"] = ta.momentum.rsi(df["Close"], window=14)
    macd = ta.trend.MACD(df["Close"])
    df["MACD"] = macd.macd()
    df["MACD_Signal"] = macd.macd_signal()
    df["MACD_Hist"] = macd.macd_diff()
    bb = ta.volatility.BollingerBands(df["Close"], window=20)
    df["BB_Upper"] = bb.bollinger_hband()
    df["BB_Lower"] = bb.bollinger_lband()
    df["BB_Mid"] = bb.bollinger_mavg()
    df["ATR"] = ta.volatility.average_true_range(df["High"], df["Low"], df["Close"], window=14)
    df["VWAP"] = (df["Volume"] * (df["High"] + df["Low"] + df["Close"]) / 3).cumsum() / df["Volume"].cumsum()
    df["OBV"] = ta.volume.on_balance_volume(df["Close"], df["Volume"])

    if n >= 30:
        stoch = ta.momentum.StochasticOscillator(df["High"], df["Low"], df["Close"])
        df["Stoch_K"] = stoch.stoch()
        df["Stoch_D"] = stoch.stoch_signal()
    else:
        df["Stoch_K"] = 0
        df["Stoch_D"] = 0

    if n >= 50:
        df["SMA_50"] = ta.trend.sma_indicator(df["Close"], window=50)
        df["ADX"] = ta.trend.adx(df["High"], df["Low"], df["Close"], window=14)
    else:
        df["SMA_50"] = 0
        df["ADX"] = 0

    df.fillna(0, inplace=True)
    return df


async def get_stock_info(symbol: str) -> dict:
    cache_key = f"info:{symbol}"
    cached = await redis_client.get_json(cache_key)
    if cached:
        return cached

    ticker = yf.Ticker(symbol)
    try:
        info = ticker.info
    except Exception:
        info = {}
    result = {
        "symbol": symbol,
        "name": info.get("longName", symbol),
        "sector": info.get("sector", "N/A"),
        "market_cap": info.get("marketCap", 0),
        "pe_ratio": info.get("trailingPE", 0),
        "dividend_yield": info.get("dividendYield", 0),
        "fifty_two_week_high": info.get("fiftyTwoWeekHigh", 0),
        "fifty_two_week_low": info.get("fiftyTwoWeekLow", 0),
        "avg_volume": info.get("averageVolume", 0),
    }
    await redis_client.set_json(cache_key, result, ttl=600)
    return result
