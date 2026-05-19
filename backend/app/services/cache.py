import json
import os
import redis.asyncio as redis


class RedisClient:
    def __init__(self):
        self.client = None

    async def connect(self):
        url = os.getenv("REDIS_URL", "redis://localhost:6379")
        self.client = redis.from_url(url, decode_responses=True)

    async def disconnect(self):
        if self.client:
            await self.client.close()

    async def get_json(self, key: str):
        val = await self.client.get(key)
        return json.loads(val) if val else None

    async def set_json(self, key: str, data, ttl: int = 300):
        await self.client.set(key, json.dumps(data, default=str), ex=ttl)


redis_client = RedisClient()
