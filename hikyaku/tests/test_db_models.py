"""Tests for db/models.py — schema, columns, indexes, FK declarations and enforcement.

Verifies that the SQLAlchemy declarative models match the schema in the
design doc (design-docs/0000015-remove-auth0-local-session-model/design-doc.md
Specification → Data Model):

  * Tables: sessions, agents, tasks, agent_placements
  * Primary keys: sessions.session_id, agents.agent_id, tasks.task_id
  * Foreign keys:
      - agents.session_id  -> sessions.session_id    (ON DELETE RESTRICT)
      - tasks.context_id   -> agents.agent_id         (ON DELETE RESTRICT)
      - tasks.from_agent_id is intentionally NOT a FK
  * Indexes:
      - idx_agents_session_status      (session_id, status)
      - idx_tasks_context_status_ts    (context_id, status_timestamp DESC)
      - idx_tasks_from_agent_status_ts (from_agent_id, status_timestamp DESC)
  * deregistered_at is NULLABLE; label is NULLABLE; every other column is NOT NULL.
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
import hikyaku.db.engine  # noqa: F401
from hikyaku.db.models import Agent, AgentPlacement, Base, Session, Task


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


def _make_session(session_id: str = "session-1", label: str | None = None) -> Session:
    return Session(
        session_id=session_id,
        label=label,
        created_at=_now(),
    )


def _make_agent(
    *,
    agent_id: str = "agent-1",
    session_id: str = "session-1",
) -> Agent:
    return Agent(
        agent_id=agent_id,
        session_id=session_id,
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
    coding_agent: str = "claude",
) -> AgentPlacement:
    return AgentPlacement(
        agent_id=agent_id,
        director_agent_id=director_agent_id,
        tmux_session=tmux_session,
        tmux_window_id=tmux_window_id,
        tmux_pane_id=tmux_pane_id,
        coding_agent=coding_agent,
        created_at=_now(),
    )


# ---------------------------------------------------------------------------
# Table existence
# ---------------------------------------------------------------------------


class TestTablesExist:
    """``Base.metadata.create_all`` produces the expected tables."""

    @pytest.mark.asyncio
    async def test_sessions_table_exists(self, engine):
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "sessions" in tables

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

    @pytest.mark.asyncio
    async def test_api_keys_table_does_not_exist(self, engine):
        """api_keys table must be removed — replaced by sessions."""
        async with engine.connect() as conn:
            tables = await conn.run_sync(lambda c: inspect(c).get_table_names())
        assert "api_keys" not in tables


# ---------------------------------------------------------------------------
# sessions table — columns + primary key
# ---------------------------------------------------------------------------


class TestSessionsSchema:
    """Schema tests for the ``sessions`` table (replaces ``api_keys``).

    Design doc 0000015 (Specification §1.1):

      sessions
        session_id   TEXT PRIMARY KEY   -- opaque string (UUID or legacy hash)
        label        TEXT               -- NULLABLE, free-form human label
        created_at   TEXT NOT NULL      -- ISO 8601 UTC
    """

    @pytest.mark.asyncio
    async def test_has_expected_columns(self, engine):
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"] for col in inspect(c).get_columns("sessions")}
            )
        expected = {"session_id", "label", "created_at"}
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_session_id_is_primary_key(self, engine):
        async with engine.connect() as conn:
            pk = await conn.run_sync(
                lambda c: inspect(c).get_pk_constraint("sessions")[
                    "constrained_columns"
                ]
            )
        assert pk == ["session_id"]

    @pytest.mark.asyncio
    async def test_label_is_nullable(self, engine):
        """label is optional free-form text — NULLABLE."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col for col in inspect(c).get_columns("sessions")
                }
            )
        assert cols["label"]["nullable"] is True

    @pytest.mark.asyncio
    async def test_required_columns_are_not_null(self, engine):
        """session_id and created_at are NOT NULL."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col for col in inspect(c).get_columns("sessions")
                }
            )
        for name in ("session_id", "created_at"):
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
            "session_id",
            "name",
            "description",
            "status",
            "registered_at",
            "deregistered_at",
            "agent_card_json",
        }
        assert expected <= cols, f"missing columns: {expected - cols}"

    @pytest.mark.asyncio
    async def test_no_tenant_id_column(self, engine):
        """agents.tenant_id must be renamed to session_id — no tenant_id column."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {col["name"] for col in inspect(c).get_columns("agents")}
            )
        assert "tenant_id" not in cols, (
            "agents table still has tenant_id column — it must be renamed to session_id"
        )

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
            "session_id",
            "name",
            "description",
            "status",
            "registered_at",
            "agent_card_json",
        ):
            assert cols[name]["nullable"] is False, f"{name} should be NOT NULL"

    @pytest.mark.asyncio
    async def test_session_id_foreign_key_to_sessions(self, engine):
        """agents.session_id REFERENCES sessions(session_id)."""
        async with engine.connect() as conn:
            fks = await conn.run_sync(lambda c: inspect(c).get_foreign_keys("agents"))
        match = [
            fk
            for fk in fks
            if fk["constrained_columns"] == ["session_id"]
            and fk["referred_table"] == "sessions"
            and fk["referred_columns"] == ["session_id"]
        ]
        assert len(match) == 1, f"expected one session_id FK to sessions, got: {fks}"


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
        """``tasks.origin_task_id`` is a nullable TEXT column."""
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
        deregistration of the original sender."""
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
            "coding_agent",
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
            "coding_agent",
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
        assert len(match) == 1, f"expected one agent_id FK to agents, got: {fks}"

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
            f"expected idx_placements_director, got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"] == ["director_agent_id"]

    @pytest.mark.asyncio
    async def test_coding_agent_column_not_nullable(self, engine):
        """``coding_agent`` column is NOT NULL."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col
                    for col in inspect(c).get_columns("agent_placements")
                }
            )
        assert cols["coding_agent"]["nullable"] is False

    @pytest.mark.asyncio
    async def test_coding_agent_server_default_is_claude(self, engine):
        """``coding_agent`` has server_default='claude' for backward compatibility."""
        async with engine.connect() as conn:
            cols = await conn.run_sync(
                lambda c: {
                    col["name"]: col
                    for col in inspect(c).get_columns("agent_placements")
                }
            )
        default = cols["coding_agent"].get("default")
        assert default is not None, "coding_agent should have a server default"
        assert "claude" in default, (
            f"server default should be 'claude', got: {default}"
        )

    @pytest.mark.asyncio
    async def test_coding_agent_defaults_to_claude_on_insert(self, session):
        """Inserting a placement without explicit coding_agent gets 'claude'."""
        session.add(_make_session(session_id="ca-def-sess"))
        session.add(_make_agent(agent_id="ca-def-dir", session_id="ca-def-sess"))
        session.add(_make_agent(agent_id="ca-def-mem", session_id="ca-def-sess"))
        await session.flush()

        # Insert placement without setting coding_agent explicitly
        placement = AgentPlacement(
            agent_id="ca-def-mem",
            director_agent_id="ca-def-dir",
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id="%7",
            created_at=_now(),
        )
        session.add(placement)
        await session.flush()

        result = await session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == "ca-def-mem")
        )
        row = result.scalar_one()
        assert row.coding_agent == "claude"

    @pytest.mark.asyncio
    async def test_coding_agent_stores_codex(self, session):
        """Can store 'codex' as the coding_agent value."""
        session.add(_make_session(session_id="ca-codex-sess"))
        session.add(_make_agent(agent_id="ca-codex-dir", session_id="ca-codex-sess"))
        session.add(_make_agent(agent_id="ca-codex-mem", session_id="ca-codex-sess"))
        await session.flush()

        session.add(
            _make_placement(
                agent_id="ca-codex-mem",
                director_agent_id="ca-codex-dir",
                coding_agent="codex",
            )
        )
        await session.flush()

        result = await session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == "ca-codex-mem")
        )
        row = result.scalar_one()
        assert row.coding_agent == "codex"

    @pytest.mark.asyncio
    async def test_coding_agent_stores_claude_explicitly(self, session):
        """Explicitly setting coding_agent='claude' is stored correctly."""
        session.add(_make_session(session_id="ca-claude-sess"))
        session.add(
            _make_agent(agent_id="ca-claude-dir", session_id="ca-claude-sess")
        )
        session.add(
            _make_agent(agent_id="ca-claude-mem", session_id="ca-claude-sess")
        )
        await session.flush()

        session.add(
            _make_placement(
                agent_id="ca-claude-mem",
                director_agent_id="ca-claude-dir",
                coding_agent="claude",
            )
        )
        await session.flush()

        result = await session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == "ca-claude-mem")
        )
        row = result.scalar_one()
        assert row.coding_agent == "claude"

    @pytest.mark.asyncio
    async def test_coding_agent_attribute_exists_on_model(self, engine):
        """AgentPlacement model has a ``coding_agent`` attribute."""
        assert hasattr(AgentPlacement, "coding_agent")


# ---------------------------------------------------------------------------
# Indexes — names and column composition match the design doc
# ---------------------------------------------------------------------------


class TestIndexes:
    @pytest.mark.asyncio
    async def test_idx_agents_session_status(self, engine):
        """Index renamed from idx_agents_tenant_status to idx_agents_session_status."""
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("agents"))
        match = [idx for idx in indexes if idx["name"] == "idx_agents_session_status"]
        assert len(match) == 1, (
            f"expected idx_agents_session_status, got: {[i['name'] for i in indexes]}"
        )
        assert match[0]["column_names"] == ["session_id", "status"]

    @pytest.mark.asyncio
    async def test_no_idx_agents_tenant_status(self, engine):
        """Old index idx_agents_tenant_status must not exist."""
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("agents"))
        old_match = [
            idx for idx in indexes if idx["name"] == "idx_agents_tenant_status"
        ]
        assert len(old_match) == 0, (
            "idx_agents_tenant_status still exists — must be renamed to "
            "idx_agents_session_status"
        )

    @pytest.mark.asyncio
    async def test_idx_tasks_context_status_ts(self, engine):
        async with engine.connect() as conn:
            indexes = await conn.run_sync(lambda c: inspect(c).get_indexes("tasks"))
        match = [idx for idx in indexes if idx["name"] == "idx_tasks_context_status_ts"]
        assert len(match) == 1, (
            f"expected idx_tasks_context_status_ts, got: {[i['name'] for i in indexes]}"
        )
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
    async def test_inserting_agent_with_unknown_session_raises(self, session):
        """An orphan agent (session_id not in sessions) cannot be inserted."""
        agent = _make_agent(session_id="nonexistent-session")
        session.add(agent)
        with pytest.raises(IntegrityError):
            await session.commit()

    @pytest.mark.asyncio
    async def test_inserting_agent_with_existing_session_succeeds(self, session):
        """An agent referencing an existing session is accepted."""
        session.add(_make_session(session_id="session-ok"))
        await session.flush()

        session.add(_make_agent(agent_id="agent-ok", session_id="session-ok"))
        await session.commit()  # must not raise

        result = await session.execute(
            select(Agent).where(Agent.agent_id == "agent-ok")
        )
        row = result.scalar_one()
        assert row.session_id == "session-ok"

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
        session.add(_make_session(session_id="session-x"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-x", session_id="session-x"))
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
        """from_agent_id is intentionally not a FK; arbitrary values are allowed."""
        session.add(_make_session(session_id="session-y"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-y", session_id="session-y"))
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
    async def test_deleting_session_with_referencing_agent_restricted(self, session):
        """ON DELETE RESTRICT: cannot delete a session that has agents pointing at it."""
        session.add(_make_session(session_id="session-r"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-r", session_id="session-r"))
        await session.commit()

        with pytest.raises(IntegrityError):
            await session.execute(
                delete(Session).where(Session.session_id == "session-r")
            )

    @pytest.mark.asyncio
    async def test_deleting_agent_with_referencing_task_restricted(self, session):
        """ON DELETE RESTRICT: cannot delete an agent whose agent_id is a task context_id."""
        session.add(_make_session(session_id="session-s"))
        await session.flush()
        session.add(_make_agent(agent_id="agent-s", session_id="session-s"))
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
        """ON DELETE CASCADE: hard-deleting an agent cascades to its placement row."""
        session.add(_make_session(session_id="session-cascade"))
        await session.flush()
        session.add(_make_agent(agent_id="director-c", session_id="session-cascade"))
        session.add(_make_agent(agent_id="member-c", session_id="session-cascade"))
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
        await session.execute(delete(Agent).where(Agent.agent_id == "member-c"))
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
        placement rows referencing its ``agent_id`` via ``director_agent_id``."""
        session.add(_make_session(session_id="session-restrict"))
        await session.flush()
        session.add(_make_agent(agent_id="director-r2", session_id="session-restrict"))
        session.add(_make_agent(agent_id="member-r2", session_id="session-restrict"))
        await session.flush()
        session.add(
            _make_placement(
                agent_id="member-r2",
                director_agent_id="director-r2",
            )
        )
        await session.commit()

        with pytest.raises(IntegrityError):
            await session.execute(delete(Agent).where(Agent.agent_id == "director-r2"))

    @pytest.mark.asyncio
    async def test_inserting_placement_with_unknown_agent_raises(self, session):
        """An orphan placement (agent_id not in agents) cannot be inserted."""
        session.add(_make_session(session_id="session-orphan-p"))
        await session.flush()
        session.add(
            _make_agent(agent_id="director-orphan", session_id="session-orphan-p")
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
        session.add(_make_session(session_id="session-orphan-d"))
        await session.flush()
        session.add(
            _make_agent(agent_id="member-orphan", session_id="session-orphan-d")
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
    async def test_session_roundtrip(self, session):
        """Session insert + select round-trip preserves all fields."""
        session.add(_make_session(session_id="rt-sess", label="PR-42 review"))
        await session.commit()

        result = await session.execute(
            select(Session).where(Session.session_id == "rt-sess")
        )
        row = result.scalar_one()
        assert row.label == "PR-42 review"
        assert row.created_at is not None

    @pytest.mark.asyncio
    async def test_session_roundtrip_null_label(self, session):
        """Session with label=None round-trips correctly."""
        session.add(_make_session(session_id="rt-sess-null", label=None))
        await session.commit()

        result = await session.execute(
            select(Session).where(Session.session_id == "rt-sess-null")
        )
        row = result.scalar_one()
        assert row.label is None

    @pytest.mark.asyncio
    async def test_agent_roundtrip_preserves_json_blob(self, session):
        import json

        card = {"name": "Test", "skills": [{"id": "s1"}]}
        session.add(_make_session(session_id="rt-2"))
        await session.flush()
        agent = _make_agent(agent_id="rt-agent", session_id="rt-2")
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
        session.add(_make_session(session_id="rt-3"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-3", session_id="rt-3"))
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
    async def test_task_roundtrip_origin_task_id_null(self, session):
        """Saving a task without origin_task_id reads back as None."""
        session.add(_make_session(session_id="rt-null"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-null", session_id="rt-null"))
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
    async def test_task_roundtrip_origin_task_id_populated(self, session):
        """Saving a task with a concrete origin_task_id reads back unchanged."""
        session.add(_make_session(session_id="rt-origin"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-agent-origin", session_id="rt-origin"))
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
        """Saving a placement with ``tmux_pane_id=None`` reads back as None."""
        session.add(_make_session(session_id="rt-pend"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-dir-pend", session_id="rt-pend"))
        session.add(_make_agent(agent_id="rt-mem-pend", session_id="rt-pend"))
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
            select(AgentPlacement).where(AgentPlacement.agent_id == "rt-mem-pend")
        )
        row = result.scalar_one()
        assert row.tmux_pane_id is None
        assert row.tmux_session == "main"
        assert row.tmux_window_id == "@3"
        assert row.director_agent_id == "rt-dir-pend"

    @pytest.mark.asyncio
    async def test_placement_roundtrip_with_pane_id(self, session):
        """Saving a placement with a concrete ``tmux_pane_id`` reads back unchanged."""
        session.add(_make_session(session_id="rt-pane"))
        await session.flush()
        session.add(_make_agent(agent_id="rt-dir-pane", session_id="rt-pane"))
        session.add(_make_agent(agent_id="rt-mem-pane", session_id="rt-pane"))
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
            select(AgentPlacement).where(AgentPlacement.agent_id == "rt-mem-pane")
        )
        row = result.scalar_one()
        assert row.tmux_pane_id == "%42"
