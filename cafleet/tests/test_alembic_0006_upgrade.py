"""Tests for Alembic migration ``0006_seed_administrator_agent``.

Design doc 0000025 §C: the 0006 data migration backfills a built-in
Administrator agent into every pre-existing session.

The migration is:

- Idempotent: running ``upgrade`` a second time detects the existing
  Administrator via ``json_extract`` on ``agent_card_json`` and skips the
  INSERT, so there is always exactly one Administrator per session.
- Generates UUIDs in Python inside the script (``uuid.uuid4()``), matching
  the broker's idiom.
- Sets ``registered_at`` equal to the owning session's ``created_at``.
- ``downgrade`` deletes Administrator rows via ``json_extract``. Treated as
  forward-only in practice — the non-empty case (sessions with tasks
  addressed to or from the Administrator) would fail on SQLite's
  ``ON DELETE RESTRICT`` for ``tasks.context_id``. Only the empty-session
  downgrade smoke is exercised here.

Test isolation strategy:

  Each test creates its own temporary DB file and runs Alembic migrations
  via ``command.upgrade``. The DB is first brought to revision ``0005``
  (pre-seed state), test data is inserted via raw ``text()`` SQL, then
  ``upgrade`` to ``0006`` is applied and the resulting ``agents`` table
  is inspected.
"""

import importlib.resources
import json
import uuid
from datetime import UTC, datetime

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, text


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_alembic_cfg(db_path) -> Config:
    """Create an Alembic Config pointing at the given SQLite DB file."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _seed_session(engine, *, session_id: str, created_at: str, label: str | None = None):
    """INSERT a session row via raw SQL at the 0005 schema level."""
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO sessions (session_id, label, created_at) "
                "VALUES (:sid, :label, :created_at)"
            ),
            {"sid": session_id, "label": label, "created_at": created_at},
        )


def _seed_user_agent(
    engine,
    *,
    agent_id: str,
    session_id: str,
    name: str,
    registered_at: str,
    status: str = "active",
):
    """INSERT a regular (non-Administrator) agent row at the 0005 schema level."""
    card = json.dumps(
        {"name": name, "description": "test user agent", "skills": []}
    )
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, session_id, name, description, status, "
                " registered_at, deregistered_at, agent_card_json) "
                "VALUES (:aid, :sid, :name, :desc, :status, :at, NULL, :card)"
            ),
            {
                "aid": agent_id,
                "sid": session_id,
                "name": name,
                "desc": "test user agent",
                "status": status,
                "at": registered_at,
                "card": card,
            },
        )


def _fetch_administrator_rows(engine, session_id: str) -> list[dict]:
    """Return all administrator rows (per cafleet.kind) for a session as dicts."""
    with engine.connect() as conn:
        rows = conn.execute(
            text(
                "SELECT agent_id, session_id, name, description, status, "
                "       registered_at, agent_card_json "
                "FROM agents "
                "WHERE session_id = :sid "
                "  AND json_extract(agent_card_json, '$.cafleet.kind') "
                "      = 'builtin-administrator'"
            ),
            {"sid": session_id},
        ).fetchall()
    return [
        {
            "agent_id": r[0],
            "session_id": r[1],
            "name": r[2],
            "description": r[3],
            "status": r[4],
            "registered_at": r[5],
            "agent_card_json": r[6],
        }
        for r in rows
    ]


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def db_at_0005(tmp_path):
    """Create a DB at Alembic revision 0005 and return the path.

    Uses ``command.upgrade(cfg, "0005")`` so all pre-seed tables
    (``sessions``, ``agents``, ``agent_placements``, ``tasks``) and
    indexes are in place with the schema that exists just before 0006.
    """
    db_path = tmp_path / "upgrade_0006_test.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0005")
    return db_path


# ---------------------------------------------------------------------------
# Upgrade — seeding the Administrator into pre-existing sessions
# ---------------------------------------------------------------------------


class TestMigration0006UpgradeSeed:
    """Test 1 — upgrade seeds exactly one Administrator per pre-existing session."""

    def test_seeds_one_administrator_per_session(self, db_at_0005):
        """After upgrade, every pre-existing session has exactly one Administrator."""
        from cafleet.broker import ADMINISTRATOR_KIND

        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        created_at_a = "2026-01-01T00:00:00+00:00"
        created_at_b = "2026-02-02T12:34:56+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            # Two pre-seed sessions with a mix of user agents.
            _seed_session(engine, session_id=sid_a, created_at=created_at_a, label="a")
            _seed_session(engine, session_id=sid_b, created_at=created_at_b, label="b")
            _seed_user_agent(
                engine,
                agent_id=str(uuid.uuid4()),
                session_id=sid_a,
                name="user-1",
                registered_at=created_at_a,
            )
            _seed_user_agent(
                engine,
                agent_id=str(uuid.uuid4()),
                session_id=sid_a,
                name="user-2",
                registered_at=created_at_a,
            )
            # Session B deliberately has no user agents — still must gain Admin.
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins_a = _fetch_administrator_rows(engine, sid_a)
            admins_b = _fetch_administrator_rows(engine, sid_b)

            assert len(admins_a) == 1, (
                f"session A should gain exactly 1 Administrator, got {len(admins_a)}"
            )
            assert len(admins_b) == 1, (
                f"session B should gain exactly 1 Administrator, got {len(admins_b)}"
            )

            for admins, expected_created_at in (
                (admins_a, created_at_a),
                (admins_b, created_at_b),
            ):
                row = admins[0]
                assert row["name"] == "Administrator"
                assert row["status"] == "active"
                # registered_at must equal the session.created_at
                assert row["registered_at"] == expected_created_at, (
                    f"Administrator.registered_at should match session.created_at"
                    f" ({expected_created_at!r}), got {row['registered_at']!r}"
                )
                # agent_id must be a valid UUID
                uuid.UUID(row["agent_id"])
                # Card shape
                card = json.loads(row["agent_card_json"])
                assert card.get("cafleet", {}).get("kind") == ADMINISTRATOR_KIND
                assert card.get("name") == "Administrator"
                assert card.get("skills") == []
        finally:
            engine.dispose()

    def test_seed_preserves_existing_user_agents(self, db_at_0005):
        """Pre-existing user agents are not touched by the migration."""
        sid = str(uuid.uuid4())
        created_at = "2026-01-01T00:00:00+00:00"
        user_id = str(uuid.uuid4())

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
            _seed_user_agent(
                engine,
                agent_id=user_id,
                session_id=sid,
                name="survivor",
                registered_at=created_at,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            with engine.connect() as conn:
                row = conn.execute(
                    text(
                        "SELECT name, status FROM agents WHERE agent_id = :aid"
                    ),
                    {"aid": user_id},
                ).fetchone()
            assert row is not None
            assert row[0] == "survivor"
            assert row[1] == "active"
        finally:
            engine.dispose()

    def test_no_sessions_no_administrators(self, db_at_0005):
        """Upgrading a DB with zero sessions inserts zero Administrator rows."""
        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            with engine.connect() as conn:
                (count,) = conn.execute(
                    text(
                        "SELECT COUNT(*) FROM agents "
                        "WHERE json_extract(agent_card_json, '$.cafleet.kind') "
                        "      = 'builtin-administrator'"
                    )
                ).fetchone()
            assert count == 0
        finally:
            engine.dispose()


# ---------------------------------------------------------------------------
# Idempotency — re-running upgrade never duplicates
# ---------------------------------------------------------------------------


class TestMigration0006UpgradeIdempotent:
    """Test 2 — running upgrade twice back-to-back still yields one Admin per session."""

    def test_double_upgrade_is_idempotent(self, db_at_0005):
        """After two consecutive ``upgrade head`` calls, exactly one Admin per session."""
        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        created_at = "2026-03-03T03:03:03+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid_a, created_at=created_at)
            _seed_session(engine, session_id=sid_b, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        # First upgrade: seed administrators.
        command.upgrade(cfg, "head")
        # Second upgrade: must be a no-op for the seeded sessions
        # (migration probes via json_extract before INSERTing).
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins_a = _fetch_administrator_rows(engine, sid_a)
            admins_b = _fetch_administrator_rows(engine, sid_b)
            assert len(admins_a) == 1, (
                f"idempotency: session A should still have 1 Administrator, "
                f"got {len(admins_a)}"
            )
            assert len(admins_b) == 1, (
                f"idempotency: session B should still have 1 Administrator, "
                f"got {len(admins_b)}"
            )
        finally:
            engine.dispose()

    def test_idempotent_preserves_original_administrator_id(self, db_at_0005):
        """The Administrator's agent_id does not change across re-runs."""
        sid = str(uuid.uuid4())
        created_at = "2026-04-04T04:04:04+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            first_admins = _fetch_administrator_rows(engine, sid)
            assert len(first_admins) == 1
            first_id = first_admins[0]["agent_id"]
        finally:
            engine.dispose()

        # Re-run upgrade.
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            second_admins = _fetch_administrator_rows(engine, sid)
            assert len(second_admins) == 1
            assert second_admins[0]["agent_id"] == first_id, (
                "re-running upgrade must not regenerate the Administrator "
                f"agent_id (was {first_id}, now {second_admins[0]['agent_id']})"
            )
        finally:
            engine.dispose()


# ---------------------------------------------------------------------------
# Downgrade smoke (empty session only)
# ---------------------------------------------------------------------------


class TestMigration0006DowngradeSmoke:
    """Test 3 — downgrade on a session with no tasks removes the Administrator.

    The non-empty case is deliberately out of scope: ``tasks.context_id``
    uses ``ON DELETE RESTRICT``, so any session with at least one task
    referencing the Administrator would fail on downgrade — testing that
    would be testing SQLite's FK enforcement, not our migration. The
    design doc treats downgrade as forward-only in practice.
    """

    def test_downgrade_removes_administrator_on_empty_session(self, db_at_0005):
        """Upgrade then downgrade on a task-free session leaves no Admin row."""
        sid = str(uuid.uuid4())
        created_at = "2026-05-05T05:05:05+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        # Confirm upgrade actually seeded the Administrator.
        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins = _fetch_administrator_rows(engine, sid)
            assert len(admins) == 1
        finally:
            engine.dispose()

        # Now downgrade one step (back to 0005).
        command.downgrade(cfg, "0005")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins = _fetch_administrator_rows(engine, sid)
            assert len(admins) == 0, (
                "downgrade must remove Administrator rows on empty sessions"
            )

            # Alembic version must be back at 0005.
            with engine.connect() as conn:
                (version,) = conn.execute(
                    text("SELECT version_num FROM alembic_version")
                ).fetchone()
            assert version == "0005"
        finally:
            engine.dispose()
