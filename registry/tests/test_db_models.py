"""Tests for db/models.py — schema, columns, indexes, FK declarations and enforcement.

Verifies that the SQLAlchemy declarative models match the schema in the
design doc (Specification → Data Model → Schema):

  * Tables: api_keys, agents, tasks
  * Primary keys: api_keys.api_key_hash, agents.agent_id, tasks.task_id
  * Foreign keys:
      - agents.tenant_id  -> api_keys.api_key_hash  (ON DELETE RESTRICT)
      - tasks.context_id  -> agents.agent_id        (ON DELETE RESTRICT)
      - tasks.from_agent_id is intentionally NOT a FK
  * Indexes:
      - idx_api_keys_owner            (owner_sub)
      - idx_agents_tenant_status      (tenant_id, status)
      - idx_tasks_context_status_ts   (context_id, status_timestamp DESC)
      - idx_tasks_from_agent_status_ts (from_agent_id, status_timestamp DESC)
  * deregistered_at is NULLABLE; every other column is NOT NULL.
  * FK enforcement at runtime: inserting an orphan agent / orphan task raises
    IntegrityError. This depends on BOTH the FK declarations in models.py AND
    the PRAGMA foreign_keys=ON listener in db/engine.py being correctly wired.
"""

from datetime import datetime, timezone

import pytest
from sqlalchemy import delete, inspect, select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

# Importing the engine module registers the FK PRAGMA listener globally.
import hikyaku_registry.db.engine  # noqa: F401
from hikyaku_registry.db.models import Agent, ApiKey, Base, Task


# ---------------------------------------------------------------------------
# Local fixtures (do not depend on the new conftest fixture stack)
# ---------------------------------------------------------------------------


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


@pytest.fixture
async def engine() -> AsyncEngine:
    """Fresh in-memory engine with the schema created via Base.metadata.create_all."""
    eng = create_async_engine("sqlite+aiosqlite:///:memory:")
    async with eng.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    yield eng
    await eng.dispose()


@pytest.fixture
async def session(engine: AsyncEngine) -> AsyncSession:
    sm = async_sessionmaker(engine, expire_on_commit=False)
    async with sm() as s:
        yield s


def _make_api_key(api_key_hash: str = "tenant-hash-1") -> ApiKey:
    return ApiKey(
        api_key_hash=api_key_hash,
        owner_sub="auth0|user1",
        key_prefix="hky_aaaa",
        status="active",
        created_at=_now(),
    )


def _make_agent(
    *,
    agent_id: str = "agent-1",
    tenant_id: str = "tenant-hash-1",
) -> Agent:
    return Agent(
        agent_id=agent_id,
        tenant_id=tenant_id,
        name="Test Agent",
        description="A test agent",
        status="active",
        registered_at=_now(),
        agent_card_json="{}",
    )


def _make_task(
    *,
    task_id: str = "task-1",
    context_id: str = "agent-1",
    from_agent_id: str = "sender-1",
    to_agent_id: str = "agent-1",
    type_: str = "unicast",
) -> Task:
    return Task(
        task_id=task_id,
        context_id=context_id,
        from_agent_id=from_agent_id,
        to_agent_id=to_agent_id,
        type=type_,
        created_at=_now(),
        status_state="submitted",
        status_timestamp=_now(),
        task_json="{}",
    )


# ---------------------------------------------------------------------------
# Table existence
# ---------------------------------------------------------------------------


class TestTablesExist:
    """``Base.metadata.create_all`` produces the three expected tables."""

    @pytest.mark.asyncio
    async def test_api_keys_table_exists(self, engine):
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "api_keys" in tables

    @pytest.mark.asyncio
    async def test_agents_table_exists(self, engine):
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "agents" in tables

    @pytest.mark.asyncio
    async def test_tasks_table_exists(self, engine):
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "tasks" in tables


# ---------------------------------------------------------------------------
# api_keys table — columns + primary key
# ---------------------------------------------------------------------------


class TestApiKeysSchema:
    @pytest.mark.asyncio
    async def test_has_expected_columns(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"] for col in inspect(c).get_columns("api_keys")}
            )
        expected = {
            "api_key_hash",
            "owner_sub",
            "key_prefix",
            "status",
            "created_at",
        }
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_api_key_hash_is_primary_key(self, engine):
        async with engine.connect() as conn:
            pk = await conn.run_sync(
                lambda c: inspect(c).get_pk_constraint("api_keys")[
                    "constrained_columns"
                ]
            )
        assert pk == ["api_key_hash"]

    @pytest.mark.asyncio
    async def test_required_columns_are_not_null(self, engine):
        """All api_keys columns are NOT NULL per the design doc schema."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col for col in inspect(c).get_columns("api_keys")
                }
            )
        for name in ("api_key_hash", "owner_sub", "key_prefix", "status", "created_at"):
            assert cols[name]["nullable"] is False, f"{name} should be NOT NULL"


# ---------------------------------------------------------------------------
# agents table — columns + PK + FK + nullability
# ---------------------------------------------------------------------------


class TestAgentsSchema:
    @pytest.mark.asyncio
    async def test_has_expected_columns(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"] for col in inspect(c).get_columns("agents")}
            )
        expected = {
            "agent_id",
            "tenant_id",
            "name",
            "description",
            "status",
            "registered_at",
            "deregistered_at",
            "agent_card_json",
        }
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_agent_id_is_primary_key(self, engine):
        async with engine.connect() as conn:
            pk = await conn.run_sync(
                lambda c: inspect(c).get_pk_constraint("agents")["constrained_columns"]
            )
        assert pk == ["agent_id"]

    @pytest.mark.asyncio
    async def test_deregistered_at_is_nullable(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"]: col for col in inspect(c).get_columns("agents")}
            )
        assert cols["deregistered_at"]["nullable"] is True

    @pytest.mark.asyncio
    async def test_other_columns_not_null(self, engine):
        """Every agents column except deregistered_at is NOT NULL."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"]: col for col in inspect(c).get_columns("agents")}
            )
        for name in (
            "agent_id",
            "tenant_id",
            "name",
            "description",
            "status",
            "registered_at",
            "agent_card_json",
        ):
            assert cols[name]["nullable"] is False, f"{name} should be NOT NULL"

    @pytest.mark.asyncio
    async def test_tenant_id_foreign_key_to_api_keys(self, engine):
        """agents.tenant_id REFERENCES api_keys(api_key_hash)."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(lambda c: inspect(c).get_foreign_keys("agents"))
        match = [
            fk
            for fk in fks
            if fk["constrained_columns"] == ["tenant_id"]
            and fk["referred_table"] == "api_keys"
            and fk["referred_columns"] == ["api_key_hash"]
        ]
        assert len(match) == 1, f"expected one tenant_id FK to api_keys, got: {fks}"


# ---------------------------------------------------------------------------
# tasks table — columns + PK + FK + intentional non-FK
# ---------------------------------------------------------------------------


class TestTasksSchema:
    @pytest.mark.asyncio
    async def test_has_expected_columns(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"] for col in inspect(c).get_columns("tasks")}
            )
        expected = {
            "task_id",
            "context_id",
            "from_agent_id",
            "to_agent_id",
            "type",
            "created_at",
            "status_state",
            "status_timestamp",
            "task_json",
        }
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_task_id_is_primary_key(self, engine):
        async with engine.connect() as conn:
            pk = await conn.run_sync(
                lambda c: inspect(c).get_pk_constraint("tasks")["constrained_columns"]
            )
        assert pk == ["task_id"]

    @pytest.mark.asyncio
    async def test_all_columns_not_null(self, engine):
        """Every task column is NOT NULL (no nullable columns on tasks)."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"]: col for col in inspect(c).get_columns("tasks")}
            )
        for name in (
            "task_id",
            "context_id",
            "from_agent_id",
            "to_agent_id",
            "type",
            "created_at",
            "status_state",
            "status_timestamp",
            "task_json",
        ):
            assert cols[name]["nullable"] is False, f"{name} should be NOT NULL"

    @pytest.mark.asyncio
    async def test_context_id_foreign_key_to_agents(self, engine):
        """tasks.context_id REFERENCES agents(agent_id)."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(lambda c: inspect(c).get_foreign_keys("tasks"))
        match = [
            fk
            for fk in fks
            if fk["constrained_columns"] == ["context_id"]
            and fk["referred_table"] == "agents"
            and fk["referred_columns"] == ["agent_id"]
        ]
        assert len(match) == 1, f"expected one context_id FK to agents, got: {fks}"

    @pytest.mark.asyncio
    async def test_from_agent_id_is_not_foreign_key(self, engine):
        """from_agent_id intentionally has NO FK so historical tasks survive
        deregistration of the original sender (design doc Data Model section)."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(lambda c: inspect(c).get_foreign_keys("tasks"))
        from_agent_fks = [
            fk for fk in fks if "from_agent_id" in fk["constrained_columns"]
        ]
        assert from_agent_fks == [], (
            f"from_agent_id should not be a foreign key, but found: {from_agent_fks}"
        )

    @pytest.mark.asyncio
    async def test_to_agent_id_is_not_foreign_key(self, engine):
        """to_agent_id is also not a FK (it can be '' for broadcast_summary)."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(lambda c: inspect(c).get_foreign_keys("tasks"))
        to_agent_fks = [fk for fk in fks if "to_agent_id" in fk["constrained_columns"]]
        assert to_agent_fks == []


# ---------------------------------------------------------------------------
# Indexes — names and column composition match the design doc
# ---------------------------------------------------------------------------


class TestIndexes:
    @pytest.mark.asyncio
    async def test_idx_api_keys_owner(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("api_keys"))
        match = [idx for idx in indexes if idx["name"] == "idx_api_keys_owner"]
        assert len(match) == 1, (
            f"expected idx_api_keys_owner, got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"] == ["owner_sub"]

    @pytest.mark.asyncio
    async def test_idx_agents_tenant_status(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("agents"))
        match = [idx for idx in indexes if idx["name"] == "idx_agents_tenant_status"]
        assert len(match) == 1, (
            f"expected idx_agents_tenant_status, got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"] == ["tenant_id", "status"]

    @pytest.mark.asyncio
    async def test_idx_tasks_context_status_ts(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("tasks"))
        match = [idx for idx in indexes if idx["name"] == "idx_tasks_context_status_ts"]
        assert len(match) == 1, (
            f"expected idx_tasks_context_status_ts, got: {[i['name'] for i in indexes]}"
        )
        # Order matters for the index — context_id is the leading column.
        assert match[0]["column_names"][0] == "context_id"
        assert "status_timestamp" in match[0]["column_names"]

    @pytest.mark.asyncio
    async def test_idx_tasks_from_agent_status_ts(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("tasks"))
        match = [
            idx for idx in indexes if idx["name"] == "idx_tasks_from_agent_status_ts"
        ]
        assert len(match) == 1, (
            f"expected idx_tasks_from_agent_status_ts, got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"][0] == "from_agent_id"
        assert "status_timestamp" in match[0]["column_names"]


# ---------------------------------------------------------------------------
# FK enforcement at runtime
#
# These tests will silently pass (false positive) unless BOTH:
#   (a) the FK declarations in models.py exist, AND
#   (b) the PRAGMA foreign_keys=ON listener in db/engine.py is registered.
# That's exactly the dual-failure case the design doc warns about.
# ---------------------------------------------------------------------------


class TestForeignKeyEnforcement:
    @pytest.mark.asyncio
    async def test_inserting_agent_with_unknown_tenant_raises(self, session):
        """An orphan agent (tenant_id not in api_keys) cannot be inserted."""
        agent = _make_agent(tenant_id="nonexistent-tenant")
        session.add(agent)
        with pytest.raises(IntegrityError):
            await session.commit()

    @pytest.mark.asyncio
    async def test_inserting_agent_with_existing_tenant_succeeds(self, session):
        """An agent referencing an existing api_key is accepted."""
        session.add(_make_api_key(api_key_hash="tenant-ok"))
        await session.flush()

        session.add(_make_agent(agent_id="agent-ok", tenant_id="tenant-ok"))
        await session.commit()  # must not raise

        result = await session.execute(
            select(Agent).where(Agent.agent_id == "agent-ok")
        )
        row = result.scalar_one()
        assert row.tenant_id == "tenant-ok"

    @pytest.mark.asyncio
    async def test_inserting_task_with_unknown_context_raises(self, session):
        """An orphan task (context_id not in agents) cannot be inserted."""
        task = _make_task(context_id="nonexistent-agent")
        session.add(task)
        with pytest.raises(IntegrityError):
            await session.commit()

    @pytest.mark.asyncio
    async def test_inserting_task_with_existing_context_succeeds(self, session):
        """A task whose context_id refers to a real agent is accepted."""
        session.add(_make_api_key(api_key_hash="tenant-x"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-x", tenant_id="tenant-x"))
        await session.flush()

        session.add(
            _make_task(
                task_id="task-ok",
                context_id="agent-x",
                from_agent_id="sender-anything",
                to_agent_id="agent-x",
            )
        )
        await session.commit()  # must not raise

        result = await session.execute(select(Task).where(Task.task_id == "task-ok"))
        row = result.scalar_one()
        assert row.context_id == "agent-x"

    @pytest.mark.asyncio
    async def test_task_from_agent_id_unconstrained(self, session):
        """from_agent_id is intentionally not a FK; arbitrary values are allowed.

        A task may be inserted with a from_agent_id that does NOT correspond to
        any row in the agents table — historical tasks must survive sender
        deregistration.
        """
        session.add(_make_api_key(api_key_hash="tenant-y"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-y", tenant_id="tenant-y"))
        await session.flush()

        session.add(
            _make_task(
                task_id="task-history",
                context_id="agent-y",
                from_agent_id="ghost-sender-no-such-agent",
                to_agent_id="agent-y",
            )
        )
        await session.commit()  # must not raise — from_agent_id is unconstrained

    @pytest.mark.asyncio
    async def test_deleting_api_key_with_referencing_agent_restricted(self, session):
        """ON DELETE RESTRICT: cannot delete an api_key that has agents pointing at it.

        ``session.execute(delete(...))`` issues the DELETE statement
        immediately (it is not deferred to commit-time flush like
        ``session.delete(instance)`` would be), so the FK violation surfaces
        from ``execute()`` itself — that is what ``pytest.raises`` must wrap.
        """
        session.add(_make_api_key(api_key_hash="tenant-r"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-r", tenant_id="tenant-r"))
        await session.commit()

        with pytest.raises(IntegrityError):
            await session.execute(
                delete(ApiKey).where(ApiKey.api_key_hash == "tenant-r")
            )

    @pytest.mark.asyncio
    async def test_deleting_agent_with_referencing_task_restricted(self, session):
        """ON DELETE RESTRICT: cannot delete an agent whose agent_id is a task context_id.

        ``session.execute(delete(...))`` issues the DELETE statement
        immediately (Core-level execution, not deferred to commit-time flush),
        so the FK violation surfaces from ``execute()`` itself — that is
        what ``pytest.raises`` must wrap.
        """
        session.add(_make_api_key(api_key_hash="tenant-s"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-s", tenant_id="tenant-s"))
        await session.flush()
        session.add(
            _make_task(
                task_id="task-s",
                context_id="agent-s",
                from_agent_id="any-sender",
                to_agent_id="agent-s",
            )
        )
        await session.commit()

        with pytest.raises(IntegrityError):
            await session.execute(delete(Agent).where(Agent.agent_id == "agent-s"))


# ---------------------------------------------------------------------------
# Insert/select round-trip — sanity check that the JSON blob columns work.
# ---------------------------------------------------------------------------


class TestRoundtrip:
    @pytest.mark.asyncio
    async def test_api_key_roundtrip(self, session):
        session.add(_make_api_key(api_key_hash="rt-1"))
        await session.commit()

        result = await session.execute(
            select(ApiKey).where(ApiKey.api_key_hash == "rt-1")
        )
        row = result.scalar_one()
        assert row.owner_sub == "auth0|user1"
        assert row.status == "active"

    @pytest.mark.asyncio
    async def test_agent_roundtrip_preserves_json_blob(self, session):
        import json

        card = {"name": "Test", "skills": [{"id": "s1"}]}
        session.add(_make_api_key(api_key_hash="rt-2"))
        await session.flush()
        agent = _make_agent(agent_id="rt-agent", tenant_id="rt-2")
        agent.agent_card_json = json.dumps(card)
        session.add(agent)
        await session.commit()

        result = await session.execute(
            select(Agent).where(Agent.agent_id == "rt-agent")
        )
        row = result.scalar_one()
        assert json.loads(row.agent_card_json) == card

    @pytest.mark.asyncio
    async def test_task_roundtrip_preserves_json_blob(self, session):
        import json

        payload = {"id": "t-1", "status": {"state": "submitted"}}
        session.add(_make_api_key(api_key_hash="rt-3"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-3", tenant_id="rt-3"))
        await session.flush()

        task = _make_task(
            task_id="rt-task",
            context_id="rt-agent-3",
            from_agent_id="rt-agent-3",
            to_agent_id="rt-agent-3",
        )
        task.task_json = json.dumps(payload)
        session.add(task)
        await session.commit()

        result = await session.execute(select(Task).where(Task.task_id == "rt-task"))
        row = result.scalar_one()
        assert json.loads(row.task_json) == payload
