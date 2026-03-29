import hashlib
import json
import secrets
import uuid
from datetime import UTC, datetime
from typing import TypedDict

import redis.asyncio as aioredis


class CreateAgentResult(TypedDict):
    agent_id: str
    api_key: str
    name: str
    registered_at: str


class AgentRecord(TypedDict, total=False):
    agent_id: str
    name: str
    description: str
    agent_card_json: str
    status: str
    registered_at: str
    deregistered_at: str


class AgentListItem(TypedDict):
    agent_id: str
    name: str
    description: str
    registered_at: str
    agent_card_json: str


class RegistryStore:
    def __init__(self, redis: aioredis.Redis) -> None:
        self._redis = redis

    async def create_agent(
        self,
        name: str,
        description: str,
        skills: list[dict] | None = None,
        api_key: str | None = None,
    ) -> CreateAgentResult:
        agent_id = str(uuid.uuid4())
        if api_key is None:
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
        pipe.sadd("agents:active", agent_id)
        pipe.sadd(f"tenant:{api_key_hash}:agents", agent_id)
        await pipe.execute()

        return {
            "agent_id": agent_id,
            "api_key": api_key,
            "name": name,
            "registered_at": registered_at,
        }

    async def get_agent(self, agent_id: str) -> AgentRecord | None:
        record = await self._redis.hgetall(f"agent:{agent_id}")
        if not record:
            return None
        record.pop("api_key_hash", None)
        return record

    async def list_active_agents(
        self, tenant_id: str | None = None
    ) -> list[AgentListItem]:
        if tenant_id is not None:
            member_ids = await self._redis.smembers(
                f"tenant:{tenant_id}:agents"
            )
        else:
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
            pipe.srem(f"tenant:{api_key_hash}:agents", agent_id)
        await pipe.execute()

        return True

    async def verify_agent_tenant(
        self, agent_id: str, tenant_id: str
    ) -> bool:
        api_key_hash = await self._redis.hget(
            f"agent:{agent_id}", "api_key_hash"
        )
        if api_key_hash is None:
            return False
        return api_key_hash == tenant_id
