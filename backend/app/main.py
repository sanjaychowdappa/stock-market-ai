import asyncio
import logging
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from contextlib import asynccontextmanager
from apscheduler.schedulers.asyncio import AsyncIOScheduler

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")

from app.routes import stocks, predictions, signals, patterns, news, realtime
from app.services.cache import redis_client
from app.services.realtime_engine import get_engine
from app.services.paper_trader import get_trader
from app.services.daily_tracker import get_tracker
from app.agents.market_agent import MarketAgent

scheduler = AsyncIOScheduler()
market_agent = MarketAgent()


@asynccontextmanager
async def lifespan(app: FastAPI):
    await redis_client.connect()
    scheduler.add_job(market_agent.run_analysis_cycle, "interval", minutes=5, id="market_analysis")
    scheduler.start()
    # Start daily prediction tracker + auto fine-tuner
    tracker = get_tracker()
    tracker.start()
    yield
    tracker.stop()
    scheduler.shutdown()
    await redis_client.disconnect()


app = FastAPI(title="Stock Market AI Agent", version="1.0.0", lifespan=lifespan)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(stocks.router, prefix="/api/stocks", tags=["stocks"])
app.include_router(predictions.router, prefix="/api/predictions", tags=["predictions"])
app.include_router(signals.router, prefix="/api/signals", tags=["signals"])
app.include_router(patterns.router, prefix="/api/patterns", tags=["patterns"])
app.include_router(news.router, prefix="/api/news", tags=["news"])
app.include_router(realtime.router, prefix="/api/realtime", tags=["realtime"])


@app.get("/api/health")
async def health():
    return {"status": "ok", "agent_active": scheduler.running}


@app.get("/api/daily/status")
async def daily_status():
    """Current daily tracking status — ticks, predictions, trades, fine-tune state."""
    tracker = get_tracker()
    return tracker.get_status()


@app.websocket("/ws/live/{symbol}")
async def websocket_live(websocket: WebSocket, symbol: str):
    """
    Live tick stream — uses the engine's simulated ticks so prices
    move even when the market is closed.
    """
    await websocket.accept()
    symbol = symbol.upper()
    engine = get_engine(symbol)
    q = engine.subscribe_ticks()
    try:
        while True:
            tick = await asyncio.wait_for(q.get(), timeout=5)
            await websocket.send_json(tick)
    except (WebSocketDisconnect, asyncio.TimeoutError, Exception):
        pass
    finally:
        engine.unsubscribe_ticks(q)


@app.websocket("/ws/paper-trade")
async def websocket_paper_trade(websocket: WebSocket):
    """
    Live paper trading stream — portfolio state updated every second.
    Trades top 5 stocks based on Kronos predictions + pattern signals.
    """
    await websocket.accept()
    trader = get_trader()
    q = trader.subscribe()
    try:
        while True:
            data = await asyncio.wait_for(q.get(), timeout=5)
            await websocket.send_json(data)
    except (WebSocketDisconnect, asyncio.TimeoutError, Exception):
        pass
    finally:
        trader.unsubscribe(q)


@app.websocket("/ws/predict/{symbol}")
async def websocket_predict(websocket: WebSocket, symbol: str):
    """
    Per-second prediction stream — zero delay.
    Instant snapshot on connect, then event-driven updates.
    """
    await websocket.accept()
    symbol = symbol.upper()
    engine = get_engine(symbol)
    q = engine.subscribe()
    # subscribe() already pushes last_payload into q for instant first frame
    try:
        while True:
            data = await asyncio.wait_for(q.get(), timeout=5)
            await websocket.send_json(data)
    except (WebSocketDisconnect, asyncio.TimeoutError, Exception):
        pass
    finally:
        engine.unsubscribe(q)
