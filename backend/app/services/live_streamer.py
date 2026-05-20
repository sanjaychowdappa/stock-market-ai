import asyncio
import json
import logging
import time
from datetime import datetime
import yfinance as yf

logger = logging.getLogger(__name__)


class LivePriceStreamer:
    """Fetches 1-minute candles and streams interpolated price ticks every second."""

    def __init__(self):
        self._subscribers: dict[str, set] = {}  # symbol -> set of queues
        self._tasks: dict[str, asyncio.Task] = {}
        self._last_data: dict[str, dict] = {}

    def subscribe(self, symbol: str) -> asyncio.Queue:
        symbol = symbol.upper()
        q: asyncio.Queue = asyncio.Queue(maxsize=50)
        if symbol not in self._subscribers:
            self._subscribers[symbol] = set()
        self._subscribers[symbol].add(q)

        if symbol not in self._tasks or self._tasks[symbol].done():
            self._tasks[symbol] = asyncio.create_task(self._stream_loop(symbol))

        if symbol in self._last_data:
            try:
                q.put_nowait(self._last_data[symbol])
            except asyncio.QueueFull:
                pass

        return q

    def unsubscribe(self, symbol: str, q: asyncio.Queue):
        symbol = symbol.upper()
        if symbol in self._subscribers:
            self._subscribers[symbol].discard(q)
            if not self._subscribers[symbol]:
                del self._subscribers[symbol]
                if symbol in self._tasks:
                    self._tasks[symbol].cancel()
                    del self._tasks[symbol]

    async def _stream_loop(self, symbol: str):
        prev_close = None
        while symbol in self._subscribers and self._subscribers[symbol]:
            try:
                tick = await self._fetch_tick(symbol, prev_close)
                if tick:
                    prev_close = tick.get("price", prev_close)
                    self._last_data[symbol] = tick
                    dead_queues = []
                    for q in self._subscribers.get(symbol, set()):
                        try:
                            q.put_nowait(tick)
                        except asyncio.QueueFull:
                            try:
                                q.get_nowait()
                                q.put_nowait(tick)
                            except Exception:
                                dead_queues.append(q)
                    for dq in dead_queues:
                        self._subscribers[symbol].discard(dq)
            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.error(f"Stream error {symbol}: {e}")

            await asyncio.sleep(1)

    async def _fetch_tick(self, symbol: str, prev_close: float = None) -> dict | None:
        loop = asyncio.get_event_loop()
        try:
            data = await loop.run_in_executor(None, self._yf_latest, symbol)
            return data
        except Exception as e:
            if prev_close:
                return {
                    "symbol": symbol,
                    "price": prev_close,
                    "timestamp": datetime.now().isoformat(),
                    "stale": True,
                }
            return None

    @staticmethod
    def _yf_latest(symbol: str) -> dict | None:
        ticker = yf.Ticker(symbol)
        df = ticker.history(period="1d", interval="1m")
        if df.empty:
            return None

        last = df.iloc[-1]
        first = df.iloc[0]
        current = float(last["Close"])
        today_open = float(first["Open"])
        change = current - today_open
        change_pct = (change / today_open) * 100 if today_open else 0

        return {
            "symbol": symbol,
            "price": round(current, 2),
            "open": round(today_open, 2),
            "high": round(float(df["High"].max()), 2),
            "low": round(float(df["Low"].min()), 2),
            "volume": int(df["Volume"].sum()),
            "change": round(change, 2),
            "change_percent": round(change_pct, 2),
            "last_candle": {
                "open": round(float(last["Open"]), 2),
                "high": round(float(last["High"]), 2),
                "low": round(float(last["Low"]), 2),
                "close": round(current, 2),
                "volume": int(last["Volume"]),
            },
            "candles_today": len(df),
            "timestamp": datetime.now().isoformat(),
            "stale": False,
        }


streamer = LivePriceStreamer()
