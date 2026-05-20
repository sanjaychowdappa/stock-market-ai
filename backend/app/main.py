import asyncio
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from contextlib import asynccontextmanager
from apscheduler.schedulers.asyncio import AsyncIOScheduler

from app.routes import stocks, predictions, signals, patterns, news
from app.services.cache import redis_client
from app.services.live_streamer import streamer
from app.agents.market_agent import MarketAgent

scheduler = AsyncIOScheduler()
market_agent = MarketAgent()


@asynccontextmanager
async def lifespan(app: FastAPI):
    await redis_client.connect()
    scheduler.add_job(market_agent.run_analysis_cycle, "interval", minutes=5, id="market_analysis")
    scheduler.start()
    yield
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


@app.get("/api/health")
async def health():
    return {"status": "ok", "agent_active": scheduler.running}


@app.websocket("/ws/live/{symbol}")
async def websocket_live(websocket: WebSocket, symbol: str):
    await websocket.accept()
    symbol = symbol.upper()
    q = streamer.subscribe(symbol)
    try:
        while True:
            tick = await asyncio.wait_for(q.get(), timeout=10)
            await websocket.send_json(tick)
    except (WebSocketDisconnect, asyncio.TimeoutError, Exception):
        pass
    finally:
        streamer.unsubscribe(symbol, q)
