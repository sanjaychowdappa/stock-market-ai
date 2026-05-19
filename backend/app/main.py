from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from contextlib import asynccontextmanager
from apscheduler.schedulers.asyncio import AsyncIOScheduler

from app.routes import stocks, predictions, signals, patterns
from app.services.cache import redis_client
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


@app.get("/api/health")
async def health():
    return {"status": "ok", "agent_active": scheduler.running}
