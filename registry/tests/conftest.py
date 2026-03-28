"""Shared test fixtures for hikyaku-registry tests."""

import pytest
import fakeredis.aioredis

from hikyaku_registry.registry_store import RegistryStore


@pytest.fixture
async def redis_client():
    """Provide a fake async Redis client for testing."""
    client = fakeredis.aioredis.FakeRedis(decode_responses=True)
    yield client
    await client.aclose()


@pytest.fixture
async def store(redis_client):
    """Provide a RegistryStore instance with fake Redis."""
    return RegistryStore(redis_client)
