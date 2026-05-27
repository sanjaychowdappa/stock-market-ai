"""
Aggressive Paper Trading Engine — $100 capital targeting 50%+ returns.

Strategy: Momentum scalping on top 5 stocks using Kronos AI predictions
+ pattern signals. Fractional shares supported. Concentrates capital into
the strongest signal at any moment and rides momentum with trailing stops.
"""

import asyncio
import time
import math
import logging
from datetime import datetime
from typing import Optional
from collections import deque

logger = logging.getLogger(__name__)

TOP_SYMBOLS = ["NVDA", "AAPL", "MSFT", "GOOGL", "AMZN"]
INITIAL_CASH = 100.00

# ── Aggressive momentum scalping thresholds ─────────────────────
# We concentrate capital into the single best signal and ride it
MAX_POSITION_PCT = 0.90           # go up to 90% in one stock (concentrated)
MIN_BUY_SIGNAL = 0.01             # very low bar — we want to be in the market
STRONG_BUY_SIGNAL = 0.05          # strong conviction → full size
SELL_SIGNAL_THRESHOLD = -0.02     # flip bearish → sell
TRAILING_STOP_PCT = 0.15          # trail 0.15% from high (tight for scalping)
HARD_STOP_PCT = -0.3              # hard stop at -0.3%
TAKE_PROFIT_PCT = 0.5             # take profit at +0.5% (then re-evaluate)
TRADE_COOLDOWN = 8                # 8 seconds between trades on same symbol
MOMENTUM_WINDOW = 5               # seconds of momentum to track


class Position:
    """Tracks a single stock position with fractional shares."""

    def __init__(self, symbol: str, shares: float, entry_price: float, entry_time: str):
        self.symbol = symbol
        self.shares = round(shares, 6)  # fractional shares
        self.entry_price = entry_price
        self.entry_time = entry_time
        self.current_price = entry_price
        self.high_price = entry_price
        self.hold_seconds = 0

    @property
    def market_value(self) -> float:
        return self.shares * self.current_price

    @property
    def cost_basis(self) -> float:
        return self.shares * self.entry_price

    @property
    def unrealized_pnl(self) -> float:
        return self.market_value - self.cost_basis

    @property
    def unrealized_pnl_pct(self) -> float:
        return (self.unrealized_pnl / self.cost_basis) * 100 if self.cost_basis else 0

    @property
    def trailing_drawdown_pct(self) -> float:
        """How far price has fallen from the position's high."""
        if self.high_price <= 0:
            return 0
        return ((self.current_price - self.high_price) / self.high_price) * 100

    def to_dict(self) -> dict:
        return {
            "symbol": self.symbol,
            "shares": round(self.shares, 4),
            "entry_price": round(self.entry_price, 2),
            "current_price": round(self.current_price, 2),
            "market_value": round(self.market_value, 2),
            "unrealized_pnl": round(self.unrealized_pnl, 4),
            "unrealized_pnl_pct": round(self.unrealized_pnl_pct, 4),
            "trailing_drawdown_pct": round(self.trailing_drawdown_pct, 4),
            "hold_seconds": self.hold_seconds,
            "entry_time": self.entry_time,
        }


class PaperTradingEngine:
    """
    Aggressive paper trader: $100 capital, fractional shares,
    momentum scalping across NVDA/AAPL/MSFT/GOOGL/AMZN.
    """

    def __init__(self):
        self.cash = INITIAL_CASH
        self.initial_cash = INITIAL_CASH
        self.positions: dict[str, Position] = {}
        self.trade_history: deque = deque(maxlen=500)
        self.realized_pnl = 0.0
        self.total_trades = 0
        self.winning_trades = 0
        self.losing_trades = 0
        self._subscribers: set = set()
        self._running = False
        self._task: Optional[asyncio.Task] = None
        self._last_trade_time: dict[str, float] = {}
        self._price_cache: dict[str, float] = {}
        self._price_history: dict[str, deque] = {s: deque(maxlen=60) for s in TOP_SYMBOLS}
        self._signal_cache: dict[str, dict] = {}
        self._prediction_cache: dict[str, dict] = {}
        self._last_payload: Optional[dict] = None
        self._start_time: Optional[str] = None
        self._peak_value: float = INITIAL_CASH
        self._value_history: deque = deque(maxlen=3600)  # 1 hour of 1s snapshots

    @property
    def portfolio_value(self) -> float:
        pos_value = sum(p.market_value for p in self.positions.values())
        return self.cash + pos_value

    @property
    def total_pnl(self) -> float:
        return self.portfolio_value - self.initial_cash

    @property
    def total_pnl_pct(self) -> float:
        return (self.total_pnl / self.initial_cash) * 100

    @property
    def win_rate(self) -> float:
        total = self.winning_trades + self.losing_trades
        return (self.winning_trades / total * 100) if total > 0 else 0

    def subscribe(self) -> asyncio.Queue:
        q: asyncio.Queue = asyncio.Queue(maxsize=50)
        self._subscribers.add(q)
        if self._last_payload:
            try:
                q.put_nowait(self._last_payload)
            except asyncio.QueueFull:
                pass
        if not self._running:
            self._running = True
            self._start_time = datetime.now().isoformat()
            self._task = asyncio.create_task(self._trading_loop())
        return q

    def unsubscribe(self, q: asyncio.Queue):
        self._subscribers.discard(q)
        if not self._subscribers:
            self._running = False
            if self._task:
                self._task.cancel()

    def _broadcast(self, payload: dict):
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

    # ── Core trading loop ───────────────────────────────────────────
    async def _trading_loop(self):
        from app.services.realtime_engine import get_engine

        # Start engines for all symbols
        engines = {}
        pred_queues = {}
        for sym in TOP_SYMBOLS:
            engine = get_engine(sym)
            q = engine.subscribe()
            engines[sym] = engine
            pred_queues[sym] = q

        try:
            while self._running:
                # Drain prediction data from all engines
                for sym, q in pred_queues.items():
                    try:
                        while not q.empty():
                            data = q.get_nowait()
                            self._process_prediction(sym, data)
                    except Exception:
                        pass

                # Update position prices + hold time
                for sym, pos in list(self.positions.items()):
                    if sym in self._price_cache:
                        pos.current_price = self._price_cache[sym]
                        pos.high_price = max(pos.high_price, pos.current_price)
                        pos.hold_seconds += 1

                # Run strategy
                self._run_strategy()

                # Track portfolio value
                val = self.portfolio_value
                self._peak_value = max(self._peak_value, val)
                self._value_history.append(round(val, 2))

                # Broadcast
                self._broadcast(self._build_payload())

                await asyncio.sleep(1)

        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error(f"Paper trading loop error: {e}", exc_info=True)
        finally:
            for sym, q in pred_queues.items():
                engines[sym].unsubscribe(q)

    def _process_prediction(self, symbol: str, data: dict):
        if "current_price" in data:
            price = data["current_price"]
            self._price_cache[symbol] = price
            self._price_history[symbol].append(price)
        if "pattern" in data:
            self._signal_cache[symbol] = data["pattern"]
        if "predictions" in data:
            self._prediction_cache[symbol] = data

    # ── Strategy: Momentum Scalping ─────────────────────────────────
    def _run_strategy(self):
        now = time.time()

        # STEP 1: Manage existing positions (stop-loss, trailing stop, take-profit)
        for sym in list(self.positions.keys()):
            self._manage_position(sym, now)

        # STEP 2: If we have cash, find the best buy opportunity
        if self.cash > 1.0:  # need at least $1 to trade
            self._find_best_entry(now)

    def _manage_position(self, symbol: str, now: float):
        pos = self.positions.get(symbol)
        if not pos:
            return

        pnl_pct = pos.unrealized_pnl_pct
        trailing = pos.trailing_drawdown_pct
        signal = self._signal_cache.get(symbol, {})
        pattern_signal = signal.get("signal", 0.0)

        # Hard stop loss
        if pnl_pct <= HARD_STOP_PCT:
            self._execute_sell(symbol, "HARD_STOP", pos.current_price)
            return

        # Trailing stop: only after we've been in profit
        if pos.high_price > pos.entry_price and trailing <= -TRAILING_STOP_PCT:
            self._execute_sell(symbol, "TRAIL_STOP", pos.current_price)
            return

        # Take profit + signal weakening → partial/full exit
        if pnl_pct >= TAKE_PROFIT_PCT:
            # If signal is turning, take all profit
            if pattern_signal < 0:
                self._execute_sell(symbol, "TAKE_PROFIT", pos.current_price)
                return
            # If signal still strong, let it ride (but tighten trailing stop mentally)

        # Signal reversal sell
        if pattern_signal < SELL_SIGNAL_THRESHOLD and pos.hold_seconds > 5:
            self._execute_sell(symbol, "SIGNAL_SELL", pos.current_price)
            return

        # Momentum died — been flat too long
        if pos.hold_seconds > 30 and abs(pnl_pct) < 0.02:
            self._execute_sell(symbol, "FLAT_EXIT", pos.current_price)
            return

    def _find_best_entry(self, now: float):
        """Rank all symbols by signal strength and buy the best one."""
        candidates = []

        for sym in TOP_SYMBOLS:
            # Skip if in cooldown or already have position
            if now - self._last_trade_time.get(sym, 0) < TRADE_COOLDOWN:
                continue
            if sym in self.positions:
                continue

            price = self._price_cache.get(sym, 0)
            if price <= 0:
                continue

            signal = self._signal_cache.get(sym, {})
            pred_data = self._prediction_cache.get(sym, {})
            pattern_signal = signal.get("signal", 0.0)
            confidence = signal.get("confidence", 0.0)
            momentum = signal.get("momentum", 0.0)
            trend = signal.get("trend", 0.0)

            # Get predicted changes at different horizons
            pred_5s = 0.0
            pred_10s = 0.0
            pred_30s = 0.0
            for p in pred_data.get("predictions", []):
                if p.get("seconds_ahead") == 5:
                    pred_5s = p.get("change_percent", 0.0)
                elif p.get("seconds_ahead") == 10:
                    pred_10s = p.get("change_percent", 0.0)
                elif p.get("seconds_ahead") == 30:
                    pred_30s = p.get("change_percent", 0.0)

            # Compute micro-momentum from recent price history
            prices = list(self._price_history.get(sym, []))
            micro_mom = 0.0
            if len(prices) >= MOMENTUM_WINDOW:
                recent = prices[-MOMENTUM_WINDOW:]
                micro_mom = (recent[-1] - recent[0]) / (recent[0] + 1e-9) * 100

            # ── Composite score ──────────────────────────────────────
            # Weight: pattern signal (30%), predictions (30%), micro-momentum (25%), trend (15%)
            score = (
                0.30 * min(max(pattern_signal * 10, -1), 1) +
                0.15 * min(max(pred_5s * 100, -1), 1) +
                0.15 * min(max(pred_30s * 50, -1), 1) +
                0.25 * min(max(micro_mom * 20, -1), 1) +
                0.15 * min(max(trend * 5, -1), 1)
            )

            if score > MIN_BUY_SIGNAL:
                candidates.append({
                    "symbol": sym,
                    "score": score,
                    "price": price,
                    "signal": pattern_signal,
                    "pred_30s": pred_30s,
                    "micro_mom": micro_mom,
                })

        if not candidates:
            return

        # Pick the best candidate
        candidates.sort(key=lambda c: c["score"], reverse=True)
        best = candidates[0]

        # Size: concentrate more capital on stronger signals
        strength = min(best["score"] / 0.5, 1.0)  # normalize to 0-1
        invest_pct = 0.3 + strength * 0.6  # 30% to 90% of available cash
        invest_amount = self.cash * invest_pct

        # Ensure we don't exceed max position size
        max_invest = self.portfolio_value * MAX_POSITION_PCT
        invest_amount = min(invest_amount, max_invest)

        if invest_amount >= 0.50:  # at least 50 cents
            reason = f"MOMENTUM(s={best['score']:.2f})"
            self._execute_buy(best["symbol"], reason, best["price"], invest_amount)

    # ── Execution ───────────────────────────────────────────────────
    def _execute_buy(self, symbol: str, reason: str, price: float, amount: float):
        # Fractional shares
        shares = amount / price
        if shares < 0.0001:
            return
        cost = shares * price
        if cost > self.cash:
            shares = self.cash / price
            cost = shares * price
        if shares < 0.0001:
            return

        self.cash -= cost
        self.positions[symbol] = Position(
            symbol=symbol, shares=shares,
            entry_price=price, entry_time=datetime.now().isoformat()
        )
        self.total_trades += 1
        self._last_trade_time[symbol] = time.time()

        trade = {
            "time": datetime.now().isoformat(),
            "symbol": symbol,
            "action": "BUY",
            "shares": round(shares, 4),
            "price": round(price, 2),
            "total": round(cost, 2),
            "reason": reason,
            "portfolio_value": round(self.portfolio_value, 2),
        }
        self.trade_history.appendleft(trade)
        logger.info(f"PAPER: BUY {shares:.4f} {symbol} @ ${price:.2f} = ${cost:.2f} ({reason})")

    def _execute_sell(self, symbol: str, reason: str, price: float):
        if symbol not in self.positions:
            return
        pos = self.positions[symbol]
        revenue = pos.shares * price
        pnl = revenue - pos.cost_basis
        pnl_pct = (pnl / pos.cost_basis) * 100 if pos.cost_basis else 0

        self.cash += revenue
        self.realized_pnl += pnl
        self.total_trades += 1
        if pnl >= 0:
            self.winning_trades += 1
        else:
            self.losing_trades += 1
        self._last_trade_time[symbol] = time.time()

        trade = {
            "time": datetime.now().isoformat(),
            "symbol": symbol,
            "action": "SELL",
            "shares": round(pos.shares, 4),
            "price": round(price, 2),
            "total": round(revenue, 2),
            "pnl": round(pnl, 4),
            "pnl_pct": round(pnl_pct, 4),
            "reason": reason,
            "hold_seconds": pos.hold_seconds,
            "portfolio_value": round(self.portfolio_value, 2),
        }
        self.trade_history.appendleft(trade)
        del self.positions[symbol]
        logger.info(f"PAPER: SELL {trade['shares']} {symbol} @ ${price:.2f} PnL: ${pnl:.4f} ({reason}) held {pos.hold_seconds}s")

    # ── Payload builder ─────────────────────────────────────────────
    def _build_payload(self) -> dict:
        positions_list = [p.to_dict() for p in self.positions.values()]
        trades_list = list(self.trade_history)[:30]

        symbol_status = []
        for sym in TOP_SYMBOLS:
            price = self._price_cache.get(sym, 0)
            signal = self._signal_cache.get(sym, {})
            in_position = sym in self.positions
            pos = self.positions.get(sym)

            # Micro-momentum
            prices = list(self._price_history.get(sym, []))
            micro_mom = 0.0
            if len(prices) >= 3:
                micro_mom = (prices[-1] - prices[-3]) / (prices[-3] + 1e-9) * 100

            symbol_status.append({
                "symbol": sym,
                "price": round(price, 2) if price else 0,
                "signal": round(signal.get("signal", 0), 4),
                "direction": signal.get("direction", "neutral"),
                "confidence": round(signal.get("confidence", 0), 2),
                "momentum": round(signal.get("momentum", 0), 4),
                "micro_momentum": round(micro_mom, 4),
                "in_position": in_position,
                "position_pnl": round(pos.unrealized_pnl, 4) if pos else 0,
                "position_pnl_pct": round(pos.unrealized_pnl_pct, 4) if pos else 0,
            })

        drawdown = 0
        if self._peak_value > 0:
            drawdown = ((self.portfolio_value - self._peak_value) / self._peak_value) * 100

        # Win/loss streak
        streak = 0
        streak_type = ""
        for t in self.trade_history:
            if t["action"] != "SELL":
                continue
            if not streak_type:
                streak_type = "W" if t.get("pnl", 0) >= 0 else "L"
            current = "W" if t.get("pnl", 0) >= 0 else "L"
            if current == streak_type:
                streak += 1
            else:
                break

        return {
            "timestamp": datetime.now().isoformat(),
            "portfolio": {
                "cash": round(self.cash, 2),
                "positions_value": round(sum(p.market_value for p in self.positions.values()), 2),
                "total_value": round(self.portfolio_value, 2),
                "initial_capital": self.initial_cash,
                "total_pnl": round(self.total_pnl, 4),
                "total_pnl_pct": round(self.total_pnl_pct, 4),
                "realized_pnl": round(self.realized_pnl, 4),
                "unrealized_pnl": round(sum(p.unrealized_pnl for p in self.positions.values()), 4),
                "peak_value": round(self._peak_value, 2),
                "drawdown_pct": round(drawdown, 4),
                "target_pct": 50.0,
                "target_value": round(self.initial_cash * 1.50, 2),
                "progress_pct": round(max(0, self.total_pnl_pct / 50.0 * 100), 2),
            },
            "stats": {
                "total_trades": self.total_trades,
                "winning_trades": self.winning_trades,
                "losing_trades": self.losing_trades,
                "win_rate": round(self.win_rate, 1),
                "open_positions": len(self.positions),
                "start_time": self._start_time,
                "streak": streak,
                "streak_type": streak_type,
                "avg_hold_seconds": self._avg_hold_time(),
            },
            "positions": positions_list,
            "symbols": symbol_status,
            "recent_trades": trades_list,
            "value_history": list(self._value_history)[-120:],  # last 2 min
        }

    def _avg_hold_time(self) -> float:
        sells = [t for t in self.trade_history if t["action"] == "SELL" and "hold_seconds" in t]
        if not sells:
            return 0
        return round(sum(t["hold_seconds"] for t in sells) / len(sells), 1)


# ── Singleton ───────────────────────────────────────────────────────
_trader: Optional[PaperTradingEngine] = None


def get_trader() -> PaperTradingEngine:
    global _trader
    if _trader is None:
        _trader = PaperTradingEngine()
    return _trader
