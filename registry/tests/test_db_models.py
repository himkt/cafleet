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
from hikyaku_registry.db.models import Agent, AgentPlacement, ApiKey, Base, Task


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
    origin_task_id: str | None = None,
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
        origin_task_id=origin_task_id,
    )


def _make_placement(
    *,
    agent_id: str = "member-1",
    director_agent_id: str = "director-1",
    tmux_session: str = "main",
    tmux_window_id: str = "@3",
    tmux_pane_id: str | None = "%7",
) -> AgentPlacement:
    return AgentPlacement(
        agent_id=agent_id,
        director_agent_id=director_agent_id,
        tmux_session=tmux_session,
        tmux_window_id=tmux_window_id,
        tmux_pane_id=tmux_pane_id,
        created_at=_now(),
    )


# ---------------------------------------------------------------------------
# Table existence
# ---------------------------------------------------------------------------


class TestTablesExist:
    """``Base.metadata.create_all`` produces the four expected tables."""

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

    @pytest.mark.asyncio
    async def test_agent_placements_table_exists(self, engine):
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "agent_placements" in tables


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
            "origin_task_id",
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
    async def test_required_columns_not_null(self, engine):
        """Every non-nullable task column is NOT NULL.

        ``origin_task_id`` is intentionally excluded — it is the single
        nullable column on ``tasks``, populated only on broadcast rows
        (delivery + summary) and left NULL for unicast and historical rows.
        See design doc 0000013 Data model change.
        """
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
    async def test_origin_task_id_is_nullable(self, engine):
        """``tasks.origin_task_id`` is a nullable TEXT column.

        Unicast deliveries and historical rows from before migration 0002
        write NULL here; broadcast delivery rows AND the broadcast summary
        row itself share a single self-referencing UUID (see design doc
        0000013 Specification → Data model change).
        """
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"]: col for col in inspect(c).get_columns("tasks")}
            )
        assert "origin_task_id" in cols, (
            "origin_task_id column missing from tasks table; "
            "Task model in db/models.py has not been updated"
        )
        assert cols["origin_task_id"]["nullable"] is True, (
            "origin_task_id must be nullable — unicast + historical rows "
            "store NULL and the broadcast-grouping predicate is "
            "`origin_task_id IS NOT NULL`"
        )

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
# agent_placements table — columns + PK + FK + nullability
# ---------------------------------------------------------------------------


class TestAgentPlacementsSchema:
    """Schema tests for the ``agent_placements`` table.

    Design doc 0000014 (Data Model):

      agent_placements
        agent_id            TEXT PRIMARY KEY  REFERENCES agents(agent_id) ON DELETE CASCADE
        director_agent_id   TEXT NOT NULL      REFERENCES agents(agent_id) ON DELETE RESTRICT
        tmux_session        TEXT NOT NULL
        tmux_window_id      TEXT NOT NULL
        tmux_pane_id        TEXT              -- NULLABLE. NULL = pending
        created_at          TEXT NOT NULL
        INDEX idx_placements_director (director_agent_id)
    """

    @pytest.mark.asyncio
    async def test_has_expected_columns(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"] for col in inspect(c).get_columns("agent_placements")
                }
            )
        expected = {
            "agent_id",
            "director_agent_id",
            "tmux_session",
            "tmux_window_id",
            "tmux_pane_id",
            "created_at",
        }
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_agent_id_is_primary_key(self, engine):
        async with engine.connect() as conn:
            pk = await conn.run_sync(
                lambda c: inspect(c).get_pk_constraint("agent_placements")[
                    "constrained_columns"
                ]
            )
        assert pk == ["agent_id"]

    @pytest.mark.asyncio
    async def test_tmux_pane_id_is_nullable(self, engine):
        """``tmux_pane_id`` is nullable — NULL signals a pending placement
        before the pane is spawned (two-pass write flow)."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col
                    for col in inspect(c).get_columns("agent_placements")
                }
            )
        assert cols["tmux_pane_id"]["nullable"] is True

    @pytest.mark.asyncio
    async def test_required_columns_not_null(self, engine):
        """Every column except ``tmux_pane_id`` is NOT NULL."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col
                    for col in inspect(c).get_columns("agent_placements")
                }
            )
        for name in (
            "agent_id",
            "director_agent_id",
            "tmux_session",
            "tmux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False, f"{name} should be NOT NULL"

    @pytest.mark.asyncio
    async def test_agent_id_fk_to_agents(self, engine):
        """``agent_placements.agent_id`` REFERENCES ``agents(agent_id)``."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(
                lambda c: inspect(c).get_foreign_keys("agent_placements")
            )
        match = [
            fk
            for fk in fks
            if fk["constrained_columns"] == ["agent_id"]
            and fk["referred_table"] == "agents"
            and fk["referred_columns"] == ["agent_id"]
        ]
        assert len(match) == 1, (
            f"expected one agent_id FK to agents, got: {fks}"
        )

    @pytest.mark.asyncio
    async def test_director_agent_id_fk_to_agents(self, engine):
        """``agent_placements.director_agent_id`` REFERENCES ``agents(agent_id)``."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(
                lambda c: inspect(c).get_foreign_keys("agent_placements")
            )
        match = [
            fk
            for fk in fks
            if fk["constrained_columns"] == ["director_agent_id"]
            and fk["referred_table"] == "agents"
            and fk["referred_columns"] == ["agent_id"]
        ]
        assert len(match) == 1, (
            f"expected one director_agent_id FK to agents, got: {fks}"
        )

    @pytest.mark.asyncio
    async def test_idx_placements_director_exists(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(
                lambda c: inspect(c).get_indexes("agent_placements")
            )
        match = [idx for idx in indexes if idx["name"] == "idx_placements_director"]
        assert len(match) == 1, (
            f"expected idx_placements_director, "
            f"got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"] == ["director_agent_id"]


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

    @pytest.mark.asyncio
    async def test_cascade_delete_agent_removes_placement(self, session):
        """ON DELETE CASCADE: hard-deleting an agent cascades to its placement row.

        FK regression test: verifies ``PRAGMA foreign_keys=ON`` is active and
        the CASCADE FK on ``agent_placements.agent_id`` fires correctly. Without
        the PRAGMA, the DELETE silently leaves an orphan placement row.

        The member agent is hard-deleted (not soft-deleted via status update) to
        exercise the FK cascade. The soft-delete path in ``deregister_agent``
        never triggers this cascade — it does an explicit placement DELETE
        instead — but the CASCADE declaration protects against accidental
        hard-delete paths added later.
        """
        session.add(_make_api_key(api_key_hash="tenant-cascade"))
        await session.flush()
        session.add(_make_agent(agent_id="director-c", tenant_id="tenant-cascade"))
        session.add(_make_agent(agent_id="member-c", tenant_id="tenant-cascade"))
        await session.flush()
        session.add(
            _make_placement(
                agent_id="member-c",
                director_agent_id="director-c",
            )
        )
        await session.commit()

        # Pre-condition: placement exists
        result = await session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == "member-c")
        )
        assert result.scalar_one_or_none() is not None

        # Hard-delete the member agent — CASCADE should remove the placement
        await session.execute(
            delete(Agent).where(Agent.agent_id == "member-c")
        )
        await session.commit()

        # Placement must be gone via CASCADE
        result = await session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == "member-c")
        )
        assert result.scalar_one_or_none() is None, (
            "CASCADE FK on agent_placements.agent_id did not fire — "
            "the placement row survives after the agent was hard-deleted. "
            "Check that PRAGMA foreign_keys=ON is active."
        )

    @pytest.mark.asyncio
    async def test_restrict_delete_director_with_live_placements(self, session):
        """ON DELETE RESTRICT: cannot hard-delete a director that still has
        placement rows referencing its ``agent_id`` via ``director_agent_id``.

        This is a sanity guard — the current code uses soft-delete for
        deregistration, but the constraint documents the invariant that a
        Director cannot be removed while it still owns live placement rows.
        """
        session.add(_make_api_key(api_key_hash="tenant-restrict"))
        await session.flush()
        session.add(
            _make_agent(agent_id="director-r2", tenant_id="tenant-restrict")
        )
        session.add(
            _make_agent(agent_id="member-r2", tenant_id="tenant-restrict")
        )
        await session.flush()
        session.add(
            _make_placement(
                agent_id="member-r2",
                director_agent_id="director-r2",
            )
        )
        await session.commit()

        # Attempt to hard-delete the director — RESTRICT should block it
        with pytest.raises(IntegrityError):
            await session.execute(
                delete(Agent).where(Agent.agent_id == "director-r2")
            )

    @pytest.mark.asyncio
    async def test_inserting_placement_with_unknown_agent_raises(self, session):
        """An orphan placement (agent_id not in agents) cannot be inserted."""
        session.add(_make_api_key(api_key_hash="tenant-orphan-p"))
        await session.flush()
        session.add(
            _make_agent(agent_id="director-orphan", tenant_id="tenant-orphan-p")
        )
        await session.flush()

        session.add(
            _make_placement(
                agent_id="nonexistent-member",
                director_agent_id="director-orphan",
            )
        )
        with pytest.raises(IntegrityError):
            await session.commit()

    @pytest.mark.asyncio
    async def test_inserting_placement_with_unknown_director_raises(self, session):
        """An orphan placement (director_agent_id not in agents) cannot be inserted."""
        session.add(_make_api_key(api_key_hash="tenant-orphan-d"))
        await session.flush()
        session.add(
            _make_agent(agent_id="member-orphan", tenant_id="tenant-orphan-d")
        )
        await session.flush()

        session.add(
            _make_placement(
                agent_id="member-orphan",
                director_agent_id="nonexistent-director",
            )
        )
        with pytest.raises(IntegrityError):
            await session.commit()


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

    @pytest.mark.asyncio
    async def test_task_roundtrip_origin_task_id_null(self, session):  # noqa: D401
        """Saving a task without origin_task_id reads back as None."""
        session.add(_make_api_key(api_key_hash="rt-null"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-null", tenant_id="rt-null"))
        await session.flush()

        session.add(
            _make_task(
                task_id="rt-task-null",
                context_id="rt-agent-null",
                from_agent_id="rt-agent-null",
                to_agent_id="rt-agent-null",
                origin_task_id=None,
            )
        )
        await session.commit()

        result = await session.execute(
            select(Task).where(Task.task_id == "rt-task-null")
        )
        row = result.scalar_one()
        assert row.origin_task_id is None

    @pytest.mark.asyncio
    async def test_task_roundtrip_origin_task_id_populated(self, session):  # noqa: D401
        """Saving a task with a concrete origin_task_id reads back unchanged."""
        session.add(_make_api_key(api_key_hash="rt-origin"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-origin", tenant_id="rt-origin"))
        await session.flush()

        origin = "11111111-2222-4333-8444-555555555555"
        session.add(
            _make_task(
                task_id="rt-task-origin",
                context_id="rt-agent-origin",
                from_agent_id="rt-agent-origin",
                to_agent_id="rt-agent-origin",
                origin_task_id=origin,
            )
        )
        await session.commit()

        result = await session.execute(
            select(Task).where(Task.task_id == "rt-task-origin")
        )
        row = result.scalar_one()
        assert row.origin_task_id == origin

    @pytest.mark.asyncio
    async def test_placement_roundtrip_null_pane_id(self, session):
        """Saving a placement with ``tmux_pane_id=None`` (pending) reads back as None."""
        session.add(_make_api_key(api_key_hash="rt-pend"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-dir-pend", tenant_id="rt-pend"))
        session.add(_make_agent(agent_id="rt-mem-pend", tenant_id="rt-pend"))
        await session.flush()

        session.add(
            _make_placement(
                agent_id="rt-mem-pend",
                director_agent_id="rt-dir-pend",
                tmux_pane_id=None,
            )
        )
        await session.commit()

        result = await session.execute(
            select(AgentPlacement).where(
                AgentPlacement.agent_id == "rt-mem-pend"
            )
        )
        row = result.scalar_one()
        assert row.tmux_pane_id is None
        assert row.tmux_session == "main"
        assert row.tmux_window_id == "@3"
        assert row.director_agent_id == "rt-dir-pend"

    @pytest.mark.asyncio
    async def test_placement_roundtrip_with_pane_id(self, session):
        """Saving a placement with a concrete ``tmux_pane_id`` reads back unchanged."""
        session.add(_make_api_key(api_key_hash="rt-pane"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-dir-pane", tenant_id="rt-pane"))
        session.add(_make_agent(agent_id="rt-mem-pane", tenant_id="rt-pane"))
        await session.flush()

        session.add(
            _make_placement(
                agent_id="rt-mem-pane",
                director_agent_id="rt-dir-pane",
                tmux_pane_id="%42",
            )
        )
        await session.commit()

        result = await session.execute(
            select(AgentPlacement).where(
                AgentPlacement.agent_id == "rt-mem-pane"
            )
        )
        row = result.scalar_one()
        assert row.tmux_pane_id == "%42"
