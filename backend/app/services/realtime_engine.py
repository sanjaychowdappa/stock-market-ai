"""
Real-time per-second prediction engine — ZERO DELAY + LIVE SIMULATION.

Always produces moving prices:
  - Market open: real yfinance ticks + micro-noise between 1-min updates
  - Market closed: realistic Brownian simulation anchored to last close + ATR
  - Kronos GPU predictions run every 8s in background
  - Pattern signal computed pure-numpy every tick
"""

import asyncio
import time
import math
import random
import numpy as np
import pandas as pd
import logging
from datetime import datetime, timedelta
from collections import deque
from typing import Optional

logger = logging.getLogger(__name__)


# ── Micro-candle buffer ─────────────────────────────────────────
class MicroCandleBuffer:
    def __init__(self, max_candles: int = 600):
        self.max_candles = max_candles
        self._candles: deque = deque(maxlen=max_candles)
        self._current_second: int = 0
        self._current_candle: Optional[dict] = None

    def add_tick(self, price: float, volume: int = 0):
        now = int(time.time())
        if now != self._current_second:
            if self._current_candle is not None:
                self._candles.append(self._current_candle)
            self._current_second = now
            self._current_candle = {
                "timestamp": now, "Open": price, "High": price,
                "Low": price, "Close": price, "Volume": volume,
            }
        else:
            if self._current_candle is not None:
                self._current_candle["High"] = max(self._current_candle["High"], price)
                self._current_candle["Low"] = min(self._current_candle["Low"], price)
                self._current_candle["Close"] = price
                self._current_candle["Volume"] += volume

    def get_candles(self, n: Optional[int] = None) -> list[dict]:
        result = list(self._candles)
        if self._current_candle is not None:
            result.append(self._current_candle)
        return result[-n:] if n else result

    def to_dataframe(self, n: Optional[int] = None) -> pd.DataFrame:
        candles = self.get_candles(n)
        if not candles:
            return pd.DataFrame()
        df = pd.DataFrame(candles)
        df["Date"] = pd.to_datetime(df["timestamp"], unit="s")
        return df

    @property
    def count(self) -> int:
        return len(self._candles) + (1 if self._current_candle else 0)

    @property
    def latest_price(self) -> Optional[float]:
        if self._current_candle:
            return self._current_candle["Close"]
        if self._candles:
            return self._candles[-1]["Close"]
        return None


# ── Micro-tick simulator ────────────────────────────────────────
class MicroTickSimulator:
    """
    Generates realistic per-second price movements.
    Uses Ornstein-Uhlenbeck process (mean-reverting random walk)
    anchored to the base price with volatility scaled from ATR.
    """

    def __init__(self, base_price: float, atr: float):
        self.base_price = base_price
        self.price = base_price
        self.atr = max(atr, base_price * 0.001)  # floor at 0.1% of price
        # OU process parameters
        self.theta = 0.15       # mean-reversion speed
        self.sigma = self.atr / 60  # per-second vol ≈ ATR / sqrt(3600) simplified
        self.mu = base_price    # mean to revert to (shifts with Kronos target)
        self._trend = 0.0       # small directional drift

    def set_target(self, target_price: float):
        """Shift the mean-reversion anchor toward the Kronos predicted price."""
        self.mu = target_price

    def set_trend(self, trend: float):
        """Set a micro-trend from pattern signal (-1 to +1)."""
        self._trend = trend * self.sigma * 0.5

    def tick(self) -> float:
        """Generate next price tick using OU process."""
        dt = 1.0  # 1 second
        noise = random.gauss(0, 1)
        # Ornstein-Uhlenbeck: dx = theta*(mu - x)*dt + sigma*sqrt(dt)*dW + trend*dt
        dx = (self.theta * (self.mu - self.price) * dt +
              self.sigma * math.sqrt(dt) * noise +
              self._trend * dt)
        self.price += dx
        # Keep price positive and within reasonable bounds (±3% from base)
        max_dev = self.base_price * 0.03
        self.price = max(self.base_price - max_dev, min(self.base_price + max_dev, self.price))
        return round(self.price, 2)

    def update_base(self, new_base: float, new_atr: float):
        """Update when real yfinance data comes in."""
        self.base_price = new_base
        self.price = new_base  # snap to real price
        self.atr = max(new_atr, new_base * 0.001)
        self.sigma = self.atr / 60
        self.mu = new_base


# ── Kronos interpolator ─────────────────────────────────────────
def interpolate_kronos_to_seconds(kronos_result: dict, seconds_ahead: int = 60) -> list[dict]:
    if not kronos_result or "predictions" not in kronos_result:
        return []
    preds = kronos_result["predictions"]
    current = kronos_result["current_close"]
    keypoints = [(0, current)]
    for p in preds:
        keypoints.append((p["step"] * 60, p["predicted_close"]))
    if len(keypoints) < 2:
        return []
    times = [k[0] for k in keypoints]
    prices = [k[1] for k in keypoints]
    result = []
    for s in range(1, seconds_ahead + 1):
        price = prices[-1]
        for i in range(len(times) - 1):
            if times[i] <= s <= times[i + 1]:
                frac = (s - times[i]) / (times[i + 1] - times[i])
                price = prices[i] + frac * (prices[i + 1] - prices[i])
                break
        result.append({"seconds_ahead": s, "kronos_price": round(price, 4)})
    return result


# ── Fast pattern signal (pure numpy) ────────────────────────────
def fast_pattern_signal(closes: np.ndarray) -> dict:
    n = len(closes)
    if n < 10:
        return {"signal": 0.0, "direction": "neutral", "confidence": 0.0,
                "momentum": 0.0, "trend": 0.0, "reversion": 0.0}

    recent = np.mean(closes[-5:])
    older = np.mean(closes[-10:-5])
    mom = (recent - older) / (older + 1e-9)

    w = min(20, n)
    if w >= 5:
        slope = np.polyfit(np.arange(w, dtype=np.float64), closes[-w:], 1)[0]
        trend = slope / (closes[-1] + 1e-9) * 100
    else:
        trend = 0.0

    if n >= 30:
        rm = np.mean(closes[-30:])
        dev = (closes[-1] - rm) / (rm + 1e-9) * 100
        reversion = float(-np.clip(dev / 0.5, -1, 1) * 0.3)
    else:
        reversion = 0.0

    if n >= 20:
        vol = float(np.std(np.diff(closes[-20:])) / (np.mean(closes[-20:]) + 1e-9))
        confidence = min(1.0, vol * 500)
    else:
        confidence = 0.3

    signal = float(np.clip(
        0.4 * np.clip(mom * 50, -1, 1) +
        0.4 * np.clip(trend * 10, -1, 1) +
        0.2 * reversion,
        -1, 1
    ))
    direction = "bullish" if signal > 0.02 else "bearish" if signal < -0.02 else "neutral"

    return {
        "signal": round(signal, 4), "direction": direction,
        "confidence": round(confidence, 4),
        "momentum": round(float(mom * 100), 4),
        "trend": round(float(trend), 4),
        "reversion": round(float(reversion), 4),
    }


def blend_prediction(kronos_price: float, pattern_signal: float,
                     current_price: float, atr: float) -> float:
    return round(kronos_price + pattern_signal * atr * 0.25, 4)


# ── Engine ───────────────────────────────────────────────────────
class RealtimePredictionEngine:
    def __init__(self, symbol: str):
        self.symbol = symbol.upper()
        self.buffer = MicroCandleBuffer(max_candles=600)
        self._subscribers: set = set()          # prediction subscribers
        self._tick_subscribers: set = set()      # live-tick subscribers
        self._kronos_cache: Optional[dict] = None
        self._kronos_interp: list[dict] = []
        self._kronos_timestamp: float = 0
        self._running = False
        self._task: Optional[asyncio.Task] = None
        self._tick_task: Optional[asyncio.Task] = None
        self._sim_task: Optional[asyncio.Task] = None
        self._kronos_task: Optional[asyncio.Task] = None
        self._1min_df: Optional[pd.DataFrame] = None
        self._atr: float = 0.1
        self._tick_event: asyncio.Event = asyncio.Event()
        self._last_payload: Optional[dict] = None
        self._yf_cache: Optional[dict] = None
        self._yf_cache_time: float = 0
        self._simulator: Optional[MicroTickSimulator] = None
        self._last_real_price: Optional[float] = None

    def _ensure_running(self):
        """Start all background loops if not already running."""
        if not self._running:
            self._running = True
            self._tick_event = asyncio.Event()
            self._tick_task = asyncio.create_task(self._tick_loop())
            self._sim_task = asyncio.create_task(self._sim_loop())
            self._task = asyncio.create_task(self._prediction_loop())
            self._kronos_task = asyncio.create_task(self._kronos_loop())

    def _check_stop(self):
        """Stop loops if no subscribers of any kind remain."""
        if not self._subscribers and not self._tick_subscribers:
            self._running = False
            for t in (self._task, self._tick_task, self._sim_task, self._kronos_task):
                if t:
                    t.cancel()

    def subscribe(self) -> asyncio.Queue:
        """Subscribe to prediction payloads (right pane)."""
        q: asyncio.Queue = asyncio.Queue(maxsize=100)
        self._subscribers.add(q)
        if self._last_payload:
            try:
                q.put_nowait(self._last_payload)
            except asyncio.QueueFull:
                pass
        self._ensure_running()
        return q

    def unsubscribe(self, q: asyncio.Queue):
        self._subscribers.discard(q)
        self._check_stop()

    def subscribe_ticks(self) -> asyncio.Queue:
        """Subscribe to live tick data (left pane — simulated when market closed)."""
        q: asyncio.Queue = asyncio.Queue(maxsize=100)
        self._tick_subscribers.add(q)
        if self._yf_cache:
            try:
                q.put_nowait(self._yf_cache)
            except asyncio.QueueFull:
                pass
        self._ensure_running()
        return q

    def unsubscribe_ticks(self, q: asyncio.Queue):
        self._tick_subscribers.discard(q)
        self._check_stop()

    # ── yfinance fetcher ─────────────────────────────────────────
    def _fetch_yf(self) -> Optional[dict]:
        now = time.time()
        if self._yf_cache and (now - self._yf_cache_time) < 5.0:
            return self._yf_cache

        import yfinance as yf
        ticker = yf.Ticker(self.symbol)
        df = ticker.history(period="1d", interval="1m")
        if df.empty:
            return self._yf_cache

        last = df.iloc[-1]
        first = df.iloc[0]
        current = float(last["Close"])
        today_open = float(first["Open"])

        self._1min_df = df.reset_index()
        n = min(14, len(df))
        self._atr = float(np.mean(df["High"].values[-n:] - df["Low"].values[-n:]))

        self._yf_cache = {
            "symbol": self.symbol,
            "price": round(current, 2),
            "open": round(today_open, 2),
            "high": round(float(df["High"].max()), 2),
            "low": round(float(df["Low"].min()), 2),
            "volume": int(df["Volume"].sum()),
            "change": round(current - today_open, 2),
            "change_percent": round((current - today_open) / (today_open + 1e-9) * 100, 2),
            "last_candle": {
                "open": round(float(last["Open"]), 2),
                "high": round(float(last["High"]), 2),
                "low": round(float(last["Low"]), 2),
                "close": round(current, 2),
                "volume": int(last["Volume"]),
            },
            "candles_today": len(df),
            "timestamp": datetime.now().isoformat(),
        }
        self._yf_cache_time = now

        # Initialize or update simulator with real data
        if self._simulator is None:
            self._simulator = MicroTickSimulator(current, self._atr)
        else:
            self._simulator.update_base(current, self._atr)
        self._last_real_price = current

        return self._yf_cache

    # ── Tick loop: fetch real data periodically ──────────────────
    async def _tick_loop(self):
        loop = asyncio.get_event_loop()
        while self._running:
            try:
                data = await loop.run_in_executor(None, self._fetch_yf)
                if data and self._simulator is None:
                    # First fetch — bootstrap simulator
                    self._simulator = MicroTickSimulator(data["price"], self._atr)
            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.debug(f"Tick error {self.symbol}: {e}")
            await asyncio.sleep(5)  # real yfinance every 5s is enough

    # ── Simulation loop: generates a tick EVERY SECOND ───────────
    async def _sim_loop(self):
        """
        Core tick generator — runs every second no matter what.
        Uses real yfinance price when fresh, otherwise simulates.
        """
        # Wait for initial yfinance fetch
        for _ in range(20):
            if self._simulator is not None:
                break
            await asyncio.sleep(0.25)

        if self._simulator is None:
            # Fallback: bootstrap with a default price
            logger.warning(f"No yfinance data for {self.symbol}, starting sim at 100.0")
            self._simulator = MicroTickSimulator(100.0, 0.5)

        while self._running:
            try:
                # Feed Kronos target into simulator for realistic drift
                if self._kronos_interp:
                    # Target = predicted price 30s ahead
                    target = self._kronos_interp[min(29, len(self._kronos_interp) - 1)]["kronos_price"]
                    self._simulator.set_target(target)

                # Feed pattern signal for micro-trend
                candles = self.buffer.get_candles(30)
                if len(candles) >= 10:
                    closes = np.array([c["Close"] for c in candles], dtype=np.float64)
                    pat = fast_pattern_signal(closes)
                    self._simulator.set_trend(pat["signal"])

                # Generate simulated tick
                sim_price = self._simulator.tick()

                # Add to buffer + signal prediction loop
                self.buffer.add_tick(sim_price, random.randint(100, 5000))
                self._tick_event.set()

                # Update yf_cache with simulated price and broadcast to tick subscribers
                if self._yf_cache:
                    self._yf_cache = {
                        **self._yf_cache,
                        "price": sim_price,
                        "last_candle": {
                            **self._yf_cache.get("last_candle", {}),
                            "close": sim_price,
                            "high": max(self._yf_cache.get("last_candle", {}).get("high", sim_price), sim_price),
                            "low": min(self._yf_cache.get("last_candle", {}).get("low", sim_price), sim_price),
                        },
                        "timestamp": datetime.now().isoformat(),
                    }
                else:
                    # No yfinance data yet — create a minimal tick payload
                    self._yf_cache = {
                        "symbol": self.symbol,
                        "price": sim_price,
                        "open": sim_price,
                        "high": sim_price,
                        "low": sim_price,
                        "volume": 0,
                        "change": 0.0,
                        "change_percent": 0.0,
                        "last_candle": {
                            "open": sim_price, "high": sim_price,
                            "low": sim_price, "close": sim_price, "volume": 0,
                        },
                        "candles_today": 0,
                        "timestamp": datetime.now().isoformat(),
                    }

                # Broadcast to live-tick subscribers
                dead_ticks = []
                for tq in self._tick_subscribers:
                    try:
                        if tq.full():
                            try:
                                tq.get_nowait()
                            except asyncio.QueueEmpty:
                                pass
                        tq.put_nowait(self._yf_cache)
                    except Exception:
                        dead_ticks.append(tq)
                for dt in dead_ticks:
                    self._tick_subscribers.discard(dt)

            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.error(f"Sim loop error {self.symbol}: {e}")

            await asyncio.sleep(1)  # exactly 1 tick per second

    # ── Kronos loop ──────────────────────────────────────────────
    async def _kronos_loop(self):
        while self._running:
            try:
                if self._1min_df is not None and len(self._1min_df) >= 30:
                    await self._run_kronos()
            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.error(f"Kronos loop error {self.symbol}: {e}")
            await asyncio.sleep(8)

    async def _run_kronos(self):
        loop = asyncio.get_event_loop()
        df = self._1min_df.copy()
        if "Datetime" in df.columns:
            df.rename(columns={"Datetime": "Date"}, inplace=True)
        if "Date" not in df.columns:
            if df.index.name:
                df = df.reset_index()
            if "Datetime" in df.columns:
                df.rename(columns={"Datetime": "Date"}, inplace=True)
        if "Date" not in df.columns:
            df["Date"] = pd.date_range(end=datetime.now(), periods=len(df), freq="1min")

        from app.services.kronos_realtime import kronos_predict_1min
        result = await loop.run_in_executor(None, kronos_predict_1min, df, 3)
        if "error" not in result:
            self._kronos_cache = result
            self._kronos_interp = interpolate_kronos_to_seconds(result, seconds_ahead=60)
            self._kronos_timestamp = time.time()

    # ── Prediction loop: event-driven ────────────────────────────
    async def _prediction_loop(self):
        while self._running:
            try:
                try:
                    await asyncio.wait_for(self._tick_event.wait(), timeout=1.0)
                except asyncio.TimeoutError:
                    pass
                self._tick_event.clear()

                current_price = self.buffer.latest_price
                if current_price is None:
                    continue

                now = time.time()

                # Pattern signal
                candles = self.buffer.get_candles(120)
                if len(candles) >= 10:
                    closes = np.array([c["Close"] for c in candles], dtype=np.float64)
                    pattern = fast_pattern_signal(closes)
                else:
                    pattern = {"signal": 0.0, "direction": "neutral", "confidence": 0.0,
                               "momentum": 0.0, "trend": 0.0, "reversion": 0.0}

                elapsed = now - self._kronos_timestamp if self._kronos_timestamp > 0 else 0

                # 30-second predictions
                predictions = []
                for s in range(1, 31):
                    target_sec = int(elapsed) + s
                    kronos_price = current_price
                    for ip in self._kronos_interp:
                        if ip["seconds_ahead"] >= target_sec:
                            kronos_price = ip["kronos_price"]
                            break
                    final_price = blend_prediction(
                        kronos_price, pattern["signal"], current_price, self._atr
                    )
                    change_pct = (final_price - current_price) / (current_price + 1e-9) * 100
                    predictions.append({
                        "seconds_ahead": s,
                        "predicted_price": final_price,
                        "kronos_price": round(kronos_price, 4),
                        "change_percent": round(change_pct, 4),
                        "direction": "bullish" if final_price > current_price else "bearish",
                    })

                payload = {
                    "symbol": self.symbol,
                    "timestamp": datetime.now().isoformat(),
                    "current_price": current_price,
                    "predictions": predictions,
                    "pattern": pattern,
                    "atr": round(self._atr, 4),
                    "kronos_age_seconds": round(elapsed, 1),
                    "micro_candles": self.buffer.count,
                }
                self._last_payload = payload

                dead = []
                for q in self._subscribers:
                    try:
                        if q.full():
                            try:
                                q.get_nowait()
                            except asyncio.QueueEmpty:
                                pass
                        q.put_nowait(payload)
                    except Exception:
                        dead.append(q)
                for d in dead:
                    self._subscribers.discard(d)

            except asyncio.CancelledError:
                return
            except Exception as e:
                logger.error(f"Prediction loop error {self.symbol}: {e}")


# ── Global registry ──────────────────────────────────────────────
_engines: dict[str, RealtimePredictionEngine] = {}


def get_engine(symbol: str) -> RealtimePredictionEngine:
    symbol = symbol.upper()
    if symbol not in _engines:
        _engines[symbol] = RealtimePredictionEngine(symbol)
    return _engines[symbol]
