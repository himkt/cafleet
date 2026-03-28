import hashlib
import json
import secrets
import uuid
from datetime import UTC, datetime

import redis.asyncio as aioredis


class RegistryStore:
    def __init__(self, redis: aioredis.Redis) -> None:
        self._redis = redis

    async def create_agent(
        self,
        name: str,
        description: str,
        skills: list[dict] | None = None,
    ) -> dict:
        agent_id = str(uuid.uuid4())
        api_key = "hky_" + secrets.token_hex(16)
        api_key_hash = hashlib.sha256(api_key.encode()).hexdigest()
        registered_at = datetime.now(UTC).isoformat()

        agent_card = {
            "name": name,
            "description": description,
            "skills": skills or [],
        }

        record = {
            "agent_id": agent_id,
            "api_key_hash": api_key_hash,
            "name": name,
            "description": description,
            "agent_card_json": json.dumps(agent_card),
            "status": "active",
            "registered_at": registered_at,
        }

        pipe = self._redis.pipeline()
        pipe.hset(f"agent:{agent_id}", mapping=record)
        pipe.set(f"apikey:{api_key_hash}", agent_id)
        pipe.sadd("agents:active", agent_id)
        await pipe.execute()

        return {
            "agent_id": agent_id,
            "api_key": api_key,
            "name": name,
            "registered_at": registered_at,
        }

    async def get_agent(self, agent_id: str) -> dict | None:
        record = await self._redis.hgetall(f"agent:{agent_id}")
        if not record:
            return None
        record.pop("api_key_hash", None)
        return record

    async def list_active_agents(self) -> list[dict]:
        member_ids = await self._redis.smembers("agents:active")
        if not member_ids:
            return []

        agents = []
        for agent_id in member_ids:
            record = await self._redis.hgetall(f"agent:{agent_id}")
            if record and record.get("status") == "active":
                agents.append({
                    "agent_id": record["agent_id"],
                    "name": record["name"],
                    "description": record["description"],
                    "registered_at": record["registered_at"],
                    "agent_card_json": record.get("agent_card_json", "{}"),
                })
        return agents

    async def deregister_agent(self, agent_id: str) -> bool:
        exists = await self._redis.exists(f"agent:{agent_id}")
        if not exists:
            return False

        api_key_hash = await self._redis.hget(f"agent:{agent_id}", "api_key_hash")
        deregistered_at = datetime.now(UTC).isoformat()

        pipe = self._redis.pipeline()
        pipe.hset(f"agent:{agent_id}", "status", "deregistered")
        pipe.hset(f"agent:{agent_id}", "deregistered_at", deregistered_at)
        pipe.srem("agents:active", agent_id)
        if api_key_hash:
            pipe.delete(f"apikey:{api_key_hash}")
        await pipe.execute()

        return True

    async def lookup_by_api_key(self, api_key: str) -> str | None:
        api_key_hash = hashlib.sha256(api_key.encode()).hexdigest()
        return await self._redis.get(f"apikey:{api_key_hash}")
