"""Alembic migration tests for the Surface-14 typed-column upgrade (design 0000049 Step 2).

The migration must:

1. Add a ``Task.text`` column.
2. Drop the legacy ``Task.task_json`` column in the **same** revision.
3. Backfill ``Task.text`` from ``task_json.artifacts[0].parts[0].text``.
4. Pre-flight check: refuse to upgrade if any existing row has a NULL body
   at ``task_json.artifacts[0].parts[0].text``.

Tests build a DB at the prior head (revision ``0008``), seed legacy rows,
then run ``alembic upgrade head`` and assert the resulting schema and data.
"""

import importlib.resources
import json
import uuid

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, text


def _make_alembic_cfg(db_path) -> Config:
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _seed_session_and_two_agents(
    engine,
    *,
    session_id: str,
    director_id: str,
    user_id: str,
    created_at: str,
) -> None:
    director_card = json.dumps(
        {"name": "director", "description": "Root Director", "skills": []}
    )
    user_card = json.dumps(
        {"name": "user-agent", "description": "user agent", "skills": []}
    )
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO sessions "
                "(session_id, label, created_at, deleted_at, director_agent_id) "
                "VALUES (:sid, NULL, :at, NULL, NULL)"
            ),
            {"sid": session_id, "at": created_at},
        )
        for aid, name, card in (
            (director_id, "director", director_card),
            (user_id, "user-agent", user_card),
        ):
            conn.execute(
                text(
                    "INSERT INTO agents "
                    "(agent_id, session_id, name, description, status, "
                    " registered_at, deregistered_at, agent_card_json) "
                    "VALUES (:aid, :sid, :name, 'desc', 'active', "
                    " :at, NULL, :card)"
                ),
                {
                    "aid": aid,
                    "sid": session_id,
                    "name": name,
                    "at": created_at,
                    "card": card,
                },
            )
        conn.execute(
            text(
                "UPDATE sessions SET director_agent_id = :aid "
                "WHERE session_id = :sid"
            ),
            {"aid": director_id, "sid": session_id},
        )


def _legacy_unicast_task_json(
    *,
    task_id: str,
    sender_id: str,
    recipient_id: str,
    body: str | None,
    timestamp: str,
    state: str = "input_required",
) -> str:
    """Serialize a pre-Surface-14 task envelope (with metadata wrapping)."""
    return json.dumps(
        {
            "id": task_id,
            "contextId": recipient_id,
            "status": {"state": state, "timestamp": timestamp},
            "artifacts": [
                {
                    "artifactId": str(uuid.uuid4()),
                    "parts": [{"kind": "text", "text": body}],
                }
            ],
            "metadata": {
                "fromAgentId": sender_id,
                "toAgentId": recipient_id,
                "type": "unicast",
            },
            "history": [],
        }
    )


def _legacy_broadcast_summary_task_json(
    *,
    task_id: str,
    sender_id: str,
    body: str,
    timestamp: str,
    recipient_count: int,
    recipient_ids: list[str],
) -> str:
    """Serialize a pre-Surface-14 broadcast summary envelope."""
    return json.dumps(
        {
            "id": task_id,
            "contextId": sender_id,
            "status": {"state": "completed", "timestamp": timestamp},
            "artifacts": [
                {
                    "artifactId": str(uuid.uuid4()),
                    "parts": [{"kind": "text", "text": body}],
                }
            ],
            "metadata": {
                "fromAgentId": sender_id,
                "type": "broadcast_summary",
                "recipientCount": recipient_count,
                "recipientIds": recipient_ids,
                "originTaskId": task_id,
            },
            "history": [],
        }
    )


def _insert_legacy_task_row(
    engine,
    *,
    task_id: str,
    context_id: str,
    from_agent_id: str,
    to_agent_id: str,
    type_: str,
    status_state: str,
    timestamp: str,
    task_json_body: str,
    origin_task_id: str | None = None,
) -> None:
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO tasks "
                "(task_id, context_id, from_agent_id, to_agent_id, type, "
                " created_at, status_state, status_timestamp, origin_task_id, "
                " task_json) "
                "VALUES (:tid, :ctx, :fa, :ta, :ty, :at, :ss, :st, :ot, :tj)"
            ),
            {
                "tid": task_id,
                "ctx": context_id,
                "fa": from_agent_id,
                "ta": to_agent_id,
                "ty": type_,
                "at": timestamp,
                "ss": status_state,
                "st": timestamp,
                "ot": origin_task_id,
                "tj": task_json_body,
            },
        )


@pytest.fixture
def db_at_0008(tmp_path):
    """A DB upgraded to revision 0008 (the head before Surface 14 lands)."""
    db_path = tmp_path / "pre_surface_14.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0008")
    return db_path


# ---------------------------------------------------------------------------
# Schema-level invariants
# ---------------------------------------------------------------------------


def test_migration_typed_columns__upgrade_adds_text_column(db_at_0008):
    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert (
        "text" in columns
    ), f"text column missing post-upgrade; got {sorted(columns)}"


def test_migration_typed_columns__upgrade_drops_task_json_column(db_at_0008):
    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert (
        "task_json" not in columns
    ), f"task_json should be dropped; got {sorted(columns)}"


def test_migration_typed_columns__pre_existing_typed_columns_survive(db_at_0008):
    """The other typed columns on tasks must survive the migration unchanged."""
    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    expected = {
        "task_id",
        "context_id",
        "from_agent_id",
        "to_agent_id",
        "type",
        "created_at",
        "status_state",
        "status_timestamp",
        "origin_task_id",
    }
    missing = expected - columns
    assert not missing, f"typed columns dropped by migration: {sorted(missing)}"


# ---------------------------------------------------------------------------
# Backfill
# ---------------------------------------------------------------------------


def test_migration_typed_columns__backfills_unicast_text_from_artifacts(db_at_0008):
    sid = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    user_id = str(uuid.uuid4())
    task_id = str(uuid.uuid4())
    timestamp = "2026-05-01T00:00:00.000000+00:00"

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        _seed_session_and_two_agents(
            engine,
            session_id=sid,
            director_id=director_id,
            user_id=user_id,
            created_at=timestamp,
        )
        _insert_legacy_task_row(
            engine,
            task_id=task_id,
            context_id=user_id,
            from_agent_id=director_id,
            to_agent_id=user_id,
            type_="unicast",
            status_state="input_required",
            timestamp=timestamp,
            task_json_body=_legacy_unicast_task_json(
                task_id=task_id,
                sender_id=director_id,
                recipient_id=user_id,
                body="hello typed columns",
                timestamp=timestamp,
            ),
        )
    finally:
        engine.dispose()

    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            row = conn.execute(
                text("SELECT text FROM tasks WHERE task_id = :tid"),
                {"tid": task_id},
            ).fetchone()
    finally:
        engine.dispose()
    assert row is not None
    assert row[0] == "hello typed columns"


def test_migration_typed_columns__backfills_broadcast_summary_text(db_at_0008):
    sid = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    user_id = str(uuid.uuid4())
    summary_id = str(uuid.uuid4())
    timestamp = "2026-05-02T00:00:00.000000+00:00"

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        _seed_session_and_two_agents(
            engine,
            session_id=sid,
            director_id=director_id,
            user_id=user_id,
            created_at=timestamp,
        )
        _insert_legacy_task_row(
            engine,
            task_id=summary_id,
            context_id=director_id,
            from_agent_id=director_id,
            to_agent_id="",  # legacy broadcast_summary has empty recipient
            type_="broadcast_summary",
            status_state="completed",
            timestamp=timestamp,
            origin_task_id=summary_id,
            task_json_body=_legacy_broadcast_summary_task_json(
                task_id=summary_id,
                sender_id=director_id,
                body="Broadcast sent to 2 recipients",
                timestamp=timestamp,
                recipient_count=2,
                recipient_ids=[user_id, str(uuid.uuid4())],
            ),
        )
    finally:
        engine.dispose()

    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            row = conn.execute(
                text(
                    "SELECT text, to_agent_id FROM tasks WHERE task_id = :tid"
                ),
                {"tid": summary_id},
            ).fetchone()
    finally:
        engine.dispose()
    assert row is not None
    assert row[0] == "Broadcast sent to 2 recipients"
    # broadcast_summary's empty to_agent_id stays empty across the migration
    assert row[1] == ""


def test_migration_typed_columns__backfills_multiple_rows(db_at_0008):
    sid = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    user_id = str(uuid.uuid4())
    timestamp = "2026-05-03T00:00:00.000000+00:00"

    bodies = ["alpha", "beta", "gamma"]
    task_ids = [str(uuid.uuid4()) for _ in bodies]

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        _seed_session_and_two_agents(
            engine,
            session_id=sid,
            director_id=director_id,
            user_id=user_id,
            created_at=timestamp,
        )
        for tid, body in zip(task_ids, bodies, strict=True):
            _insert_legacy_task_row(
                engine,
                task_id=tid,
                context_id=user_id,
                from_agent_id=director_id,
                to_agent_id=user_id,
                type_="unicast",
                status_state="input_required",
                timestamp=timestamp,
                task_json_body=_legacy_unicast_task_json(
                    task_id=tid,
                    sender_id=director_id,
                    recipient_id=user_id,
                    body=body,
                    timestamp=timestamp,
                ),
            )
    finally:
        engine.dispose()

    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT task_id, text FROM tasks "
                    "WHERE task_id IN (:a, :b, :c)"
                ),
                {"a": task_ids[0], "b": task_ids[1], "c": task_ids[2]},
            ).fetchall()
    finally:
        engine.dispose()
    by_id = {r[0]: r[1] for r in rows}
    assert by_id[task_ids[0]] == "alpha"
    assert by_id[task_ids[1]] == "beta"
    assert by_id[task_ids[2]] == "gamma"


# ---------------------------------------------------------------------------
# Pre-flight check
# ---------------------------------------------------------------------------


def test_migration_typed_columns__preflight_raises_on_null_body(db_at_0008):
    """Upgrade must abort when a row's artifacts[0].parts[0].text is None."""
    sid = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    user_id = str(uuid.uuid4())
    bad_task_id = str(uuid.uuid4())
    timestamp = "2026-05-04T00:00:00.000000+00:00"

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        _seed_session_and_two_agents(
            engine,
            session_id=sid,
            director_id=director_id,
            user_id=user_id,
            created_at=timestamp,
        )
        _insert_legacy_task_row(
            engine,
            task_id=bad_task_id,
            context_id=user_id,
            from_agent_id=director_id,
            to_agent_id=user_id,
            type_="unicast",
            status_state="input_required",
            timestamp=timestamp,
            task_json_body=_legacy_unicast_task_json(
                task_id=bad_task_id,
                sender_id=director_id,
                recipient_id=user_id,
                body=None,  # the offending NULL body
                timestamp=timestamp,
            ),
        )
    finally:
        engine.dispose()

    cfg = _make_alembic_cfg(db_at_0008)
    with pytest.raises(Exception):
        command.upgrade(cfg, "head")


def test_migration_typed_columns__preflight_raises_when_artifacts_missing(
    db_at_0008,
):
    """A row whose task_json has no artifacts at all must also abort the upgrade."""
    sid = str(uuid.uuid4())
    director_id = str(uuid.uuid4())
    user_id = str(uuid.uuid4())
    bad_task_id = str(uuid.uuid4())
    timestamp = "2026-05-05T00:00:00.000000+00:00"

    bogus = json.dumps(
        {
            "id": bad_task_id,
            "contextId": user_id,
            "status": {"state": "input_required", "timestamp": timestamp},
            "metadata": {
                "fromAgentId": director_id,
                "toAgentId": user_id,
                "type": "unicast",
            },
            "history": [],
        }
    )

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        _seed_session_and_two_agents(
            engine,
            session_id=sid,
            director_id=director_id,
            user_id=user_id,
            created_at=timestamp,
        )
        _insert_legacy_task_row(
            engine,
            task_id=bad_task_id,
            context_id=user_id,
            from_agent_id=director_id,
            to_agent_id=user_id,
            type_="unicast",
            status_state="input_required",
            timestamp=timestamp,
            task_json_body=bogus,
        )
    finally:
        engine.dispose()

    cfg = _make_alembic_cfg(db_at_0008)
    with pytest.raises(Exception):
        command.upgrade(cfg, "head")


def test_migration_typed_columns__upgrade_succeeds_on_empty_tasks_table(
    db_at_0008,
):
    """An empty tasks table is a no-op for backfill; the schema change still applies."""
    cfg = _make_alembic_cfg(db_at_0008)
    command.upgrade(cfg, "head")  # must not raise

    engine = create_engine(f"sqlite:///{db_at_0008}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert "text" in columns
    assert "task_json" not in columns
