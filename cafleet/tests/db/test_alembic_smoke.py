"""Alembic smoke test — runs real migrations against a tempfile DB.

Other tests use Base.metadata.create_all and bypass Alembic entirely,
so this is the only place that catches migration-vs-model drift.
"""

import importlib.resources

import pytest
from alembic import command
from alembic.config import Config
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect, text


@pytest.fixture(scope="session")
def alembic_upgraded_db(tmp_path_factory):
    tmp_db_path = tmp_path_factory.mktemp("alembic_smoke") / "smoke.db"

    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{tmp_db_path}")
        command.upgrade(cfg, "head")

    return tmp_db_path


def test_alembic_upgrade_head_creates_expected_tables(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        tables = set(insp.get_table_names())

        expected = {
            "fleets",
            "agents",
            "tasks",
            "agent_placements",
            "monitor_config",
            "monitor_runtime",
            "skill_installs",
            "alembic_version",
        }
        missing = expected - tables
        assert not missing
        assert "api_keys" not in tables
    finally:
        engine.dispose()


def test_alembic_version_table_records_head_0001(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert rows == [("0001",)]
    finally:
        engine.dispose()


def test_two_migration_revisions_exist():
    """The migration history is a linear 2-revision chain: 0002 (head) on top
    of the initial 0001."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        script = ScriptDirectory.from_config(cfg)
        revisions = list(script.walk_revisions())

    assert len(revisions) == 2
    assert revisions[0].revision == "0002"
    assert revisions[0].down_revision == "0001"
    assert revisions[1].revision == "0001"
    assert revisions[1].down_revision is None
    assert script.get_current_head() == "0002"


def test_minted_id_tables_declare_autoincrement(alembic_upgraded_db):
    """``fleets``/``agents``/``tasks`` mint ids via AUTOINCREMENT; placements do not."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            # SQLite creates the sqlite_sequence table whenever an
            # AUTOINCREMENT table exists (guaranteeing ids are never reused).
            seq = conn.execute(
                text(
                    "SELECT name FROM sqlite_master "
                    "WHERE type='table' AND name='sqlite_sequence'"
                )
            ).fetchall()
            assert seq, "sqlite_sequence must exist for AUTOINCREMENT tables"

            for table in ("fleets", "agents", "tasks"):
                ddl = conn.execute(
                    text(
                        "SELECT sql FROM sqlite_master WHERE type='table' AND name=:t"
                    ),
                    {"t": table},
                ).scalar()
                assert "AUTOINCREMENT" in ddl.upper()

            # agent_placements.agent_id is the agents FK reused as a 1:1 PK —
            # explicitly NOT AUTOINCREMENT.
            placements_ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='agent_placements'"
                )
            ).scalar()
            assert "AUTOINCREMENT" not in placements_ddl.upper()
    finally:
        engine.dispose()


@pytest.mark.parametrize(
    ("table", "pk_column"),
    [
        ("fleets", "fleet_id"),
        ("agents", "agent_id"),
        ("tasks", "task_id"),
        ("agent_placements", "agent_id"),
        ("monitor_config", "agent_id"),
        ("monitor_runtime", "fleet_id"),
    ],
)
def test_primary_key_columns_are_integer(alembic_upgraded_db, table, pk_column):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns(table)}
        assert "INT" in str(cols[pk_column]["type"]).upper()
    finally:
        engine.dispose()


def test_agent_placements_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "agent_placements" in tables

        cols = {col["name"]: col for col in insp.get_columns("agent_placements")}
        expected_cols = {
            "agent_id",
            "director_agent_id",
            "backend",
            "mux_session",
            "mux_window_id",
            "mux_pane_id",
            "created_at",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL = pending placement before the pane is spawned
        assert cols["mux_pane_id"]["nullable"] is True

        # NULL marks the root Director's own placement (no parent)
        assert cols["director_agent_id"]["nullable"] is True

        for name in (
            "agent_id",
            "backend",
            "mux_session",
            "mux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False

        # backend backfills existing rows to 'tmux' (matching their provenance)
        assert "tmux" in str(cols["backend"]["default"])

        indexes = insp.get_indexes("agent_placements")
        idx_names = {idx["name"] for idx in indexes}
        assert "idx_placements_director" in idx_names
    finally:
        engine.dispose()


def test_tasks_table_has_origin_task_id_column(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("tasks")}

        assert "origin_task_id" in cols

        # Must be nullable because unicast + historical rows store NULL
        assert cols["origin_task_id"]["nullable"] is True
    finally:
        engine.dispose()


def test_tasks_to_agent_id_is_nullable_after_migration(alembic_upgraded_db):
    """``tasks.to_agent_id`` is nullable so broadcast-summary rows persist NULL
    instead of the ``0`` sentinel."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("tasks")}

        assert cols["to_agent_id"]["nullable"] is True
    finally:
        engine.dispose()


def _default_int(cols, name):
    """Normalize a SQLite column default (PRAGMA returns text, maybe quoted)."""
    raw = cols[name]["default"]
    assert raw is not None, f"{name} must declare a default"
    return int(str(raw).strip("'\""))


def test_monitor_config_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "monitor_config" in tables

        cols = {col["name"]: col for col in insp.get_columns("monitor_config")}
        expected_cols = {
            "agent_id",
            "interval_seconds",
            "last_ping_at",
            "enabled",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL last_ping_at = never pinged ⇒ due immediately
        assert cols["last_ping_at"]["nullable"] is True

        # schedule columns are NOT NULL
        for name in ("agent_id", "interval_seconds", "enabled"):
            assert cols[name]["nullable"] is False

        # defaults per §2: interval_seconds 60, enabled 1
        assert _default_int(cols, "interval_seconds") == 60
        assert _default_int(cols, "enabled") == 1

        # agent_id is the agents FK reused as a 1:1 PK
        fks = insp.get_foreign_keys("monitor_config")
        assert any(fk["referred_table"] == "agents" for fk in fks)
    finally:
        engine.dispose()


def test_monitor_runtime_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "monitor_runtime" in tables

        cols = {col["name"]: col for col in insp.get_columns("monitor_runtime")}
        expected_cols = {
            "fleet_id",
            "pid",
            "started_at",
            "last_tick_at",
            "tick_seconds",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL after a clean stop / before a tick
        for name in ("pid", "started_at", "last_tick_at"):
            assert cols[name]["nullable"] is True

        # PK + scan cadence are NOT NULL
        for name in ("fleet_id", "tick_seconds"):
            assert cols[name]["nullable"] is False

        # default per §2: tick_seconds 5
        assert _default_int(cols, "tick_seconds") == 5

        # fleet_id reuses the fleets PK 1:1
        fks = insp.get_foreign_keys("monitor_runtime")
        assert any(fk["referred_table"] == "fleets" for fk in fks)
    finally:
        engine.dispose()


def test_monitor_tables_do_not_declare_autoincrement(alembic_upgraded_db):
    """monitor_config (agent_id) and monitor_runtime (fleet_id) reuse a parent id 1:1 — no AUTOINCREMENT."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            for table in ("monitor_config", "monitor_runtime"):
                ddl = conn.execute(
                    text(
                        "SELECT sql FROM sqlite_master WHERE type='table' AND name=:t"
                    ),
                    {"t": table},
                ).scalar()
                assert ddl is not None
                assert "AUTOINCREMENT" not in ddl.upper()
    finally:
        engine.dispose()


def test_skill_installs_table_created_by_migration(alembic_upgraded_db):
    """The initial schema creates ``skill_installs``: three NOT NULL string
    columns with ``coding_agent`` (a known home key, not a minted id) as the PK."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "skill_installs" in tables

        cols = {col["name"]: col for col in insp.get_columns("skill_installs")}
        assert set(cols) == {"coding_agent", "cafleet_version", "installed_at"}
        for name in ("coding_agent", "cafleet_version", "installed_at"):
            assert cols[name]["nullable"] is False

        pk = insp.get_pk_constraint("skill_installs")
        assert pk["constrained_columns"] == ["coding_agent"]

        with engine.connect() as conn:
            ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='skill_installs'"
                )
            ).scalar()
        assert ddl is not None
        assert "AUTOINCREMENT" not in ddl.upper()
    finally:
        engine.dispose()


def _populate_0001(db_path):
    """Insert rows into every table migration 0002 renames or touches: one
    fleet, a root Director + an ordinary member with their placements
    (director's placement carries NULL director_agent_id, the member's carries
    the Director's id), unicast tasks both ways plus a broadcast summary with a
    NULL recipient, a monitor schedule, a runtime row, and a skill install."""
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        with engine.begin() as conn:
            conn.execute(
                text(
                    "INSERT INTO fleets "
                    "(fleet_id, name, created_at, deleted_at, director_agent_id) "
                    "VALUES (1, 'fleet-one', '2026-07-11T00:00:00+00:00', NULL, NULL)"
                )
            )
            conn.execute(
                text(
                    "INSERT INTO agents (agent_id, fleet_id, name, description, "
                    "status, registered_at, deregistered_at, agent_card_json) VALUES "
                    "(1, 1, 'Director', 'root director', 'active', "
                    "'2026-07-11T00:00:01+00:00', NULL, '{\"name\": \"Director\"}'), "
                    "(2, 1, 'Worker', 'ordinary member', 'active', "
                    "'2026-07-11T00:00:02+00:00', NULL, '{\"name\": \"Worker\"}')"
                )
            )
            conn.execute(
                text("UPDATE fleets SET director_agent_id = 1 WHERE fleet_id = 1")
            )
            conn.execute(
                text(
                    "INSERT INTO agent_placements (agent_id, director_agent_id, "
                    "mux_session, mux_window_id, mux_pane_id, backend, coding_agent, "
                    "created_at) VALUES "
                    "(1, NULL, 'cafleet-1', '@1', '%1', 'tmux', 'claude', "
                    "'2026-07-11T00:00:01+00:00'), "
                    "(2, 1, 'cafleet-1', '@1', '%2', 'tmux', 'claude', "
                    "'2026-07-11T00:00:02+00:00')"
                )
            )
            conn.execute(
                text(
                    "INSERT INTO tasks (task_id, context_id, from_agent_id, "
                    "to_agent_id, type, created_at, status_state, status_timestamp, "
                    "origin_task_id, text) VALUES "
                    "(1, 2, 1, 2, 'message', '2026-07-11T00:01:00+00:00', "
                    "'input_required', '2026-07-11T00:01:00+00:00', NULL, "
                    "'hello worker'), "
                    "(2, 1, 2, 1, 'message', '2026-07-11T00:02:00+00:00', "
                    "'completed', '2026-07-11T00:02:30+00:00', NULL, "
                    "'hello director'), "
                    "(3, 1, 1, NULL, 'message', '2026-07-11T00:03:00+00:00', "
                    "'completed', '2026-07-11T00:03:00+00:00', NULL, "
                    "'broadcast summary')"
                )
            )
            conn.execute(
                text(
                    "INSERT INTO monitor_config "
                    "(agent_id, interval_seconds, last_ping_at, enabled) "
                    "VALUES (2, 90, NULL, 1)"
                )
            )
            conn.execute(
                text(
                    "INSERT INTO monitor_runtime "
                    "(fleet_id, pid, started_at, last_tick_at, tick_seconds) "
                    "VALUES (1, 4242, '2026-07-11T00:00:03+00:00', NULL, 5)"
                )
            )
            conn.execute(
                text(
                    "INSERT INTO skill_installs "
                    "(coding_agent, cafleet_version, installed_at) "
                    "VALUES ('claude', '0.1.0', '2026-07-11T00:00:00+00:00')"
                )
            )
    finally:
        engine.dispose()


@pytest.fixture(scope="module")
def upgraded_populated_db(tmp_path_factory):
    """A DB populated at revision 0001, then upgraded to head (0002)."""
    db_path = tmp_path_factory.mktemp("alembic_populated") / "populated.db"

    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        command.upgrade(cfg, "0001")
        _populate_0001(db_path)
        command.upgrade(cfg, "head")

    return db_path


def test_upgrade_renames_registry_tables(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        tables = set(inspect(engine).get_table_names())
        assert "members" in tables
        assert "member_placements" in tables
        assert "agents" not in tables
        assert "agent_placements" not in tables
    finally:
        engine.dispose()


def test_upgrade_records_head_0002(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("SELECT version_num FROM alembic_version"))
            assert rows.fetchall() == [("0002",)]
    finally:
        engine.dispose()


def test_upgrade_preserves_member_rows(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT member_id, fleet_id, name, description, status, "
                    "registered_at, deregistered_at, member_card_json "
                    "FROM members ORDER BY member_id"
                )
            ).fetchall()
        assert rows == [
            (
                1,
                1,
                "Director",
                "root director",
                "active",
                "2026-07-11T00:00:01+00:00",
                None,
                '{"name": "Director"}',
            ),
            (
                2,
                1,
                "Worker",
                "ordinary member",
                "active",
                "2026-07-11T00:00:02+00:00",
                None,
                '{"name": "Worker"}',
            ),
        ]
    finally:
        engine.dispose()


def test_upgrade_drops_placement_director_column(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        cols = {c["name"] for c in inspect(engine).get_columns("member_placements")}
        assert cols == {
            "member_id",
            "mux_session",
            "mux_window_id",
            "mux_pane_id",
            "backend",
            "coding_agent",
            "created_at",
        }
    finally:
        engine.dispose()


def test_upgrade_preserves_placement_rows(upgraded_populated_db):
    """Both placement rows survive the director-column drop — including the
    root Director's own (formerly NULL-director) row."""
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT member_id, mux_session, mux_window_id, mux_pane_id, "
                    "backend, coding_agent, created_at "
                    "FROM member_placements ORDER BY member_id"
                )
            ).fetchall()
        assert rows == [
            (1, "cafleet-1", "@1", "%1", "tmux", "claude", "2026-07-11T00:00:01+00:00"),
            (2, "cafleet-1", "@1", "%2", "tmux", "claude", "2026-07-11T00:00:02+00:00"),
        ]
    finally:
        engine.dispose()


def test_upgrade_renames_fleet_director_column_preserving_reference(
    upgraded_populated_db,
):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        cols = {c["name"] for c in inspect(engine).get_columns("fleets")}
        assert "director_member_id" in cols
        assert "director_agent_id" not in cols

        with engine.connect() as conn:
            director = conn.execute(
                text("SELECT director_member_id FROM fleets WHERE fleet_id = 1")
            ).scalar()
        assert director == 1
    finally:
        engine.dispose()


def test_upgrade_preserves_task_rows(upgraded_populated_db):
    """Task party columns are renamed with values intact — including the NULL
    to_member_id on the broadcast-summary row."""
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        cols = {c["name"] for c in inspect(engine).get_columns("tasks")}
        assert {"from_member_id", "to_member_id"} <= cols
        assert "from_agent_id" not in cols
        assert "to_agent_id" not in cols

        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT task_id, context_id, from_member_id, to_member_id, text "
                    "FROM tasks ORDER BY task_id"
                )
            ).fetchall()
        assert rows == [
            (1, 2, 1, 2, "hello worker"),
            (2, 1, 2, 1, "hello director"),
            (3, 1, 1, None, "broadcast summary"),
        ]
    finally:
        engine.dispose()


def test_upgrade_renames_monitor_config_pk_preserving_schedule(
    upgraded_populated_db,
):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        cols = {c["name"] for c in inspect(engine).get_columns("monitor_config")}
        assert "member_id" in cols
        assert "agent_id" not in cols

        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT member_id, interval_seconds, last_ping_at, enabled "
                    "FROM monitor_config"
                )
            ).fetchall()
        assert rows == [(2, 90, None, 1)]
    finally:
        engine.dispose()


def test_upgrade_preserves_untouched_tables(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        with engine.connect() as conn:
            runtime = conn.execute(
                text(
                    "SELECT fleet_id, pid, started_at, last_tick_at, tick_seconds "
                    "FROM monitor_runtime"
                )
            ).fetchall()
            installs = conn.execute(
                text(
                    "SELECT coding_agent, cafleet_version, installed_at "
                    "FROM skill_installs"
                )
            ).fetchall()
        assert runtime == [(1, 4242, "2026-07-11T00:00:03+00:00", None, 5)]
        assert installs == [("claude", "0.1.0", "2026-07-11T00:00:00+00:00")]
    finally:
        engine.dispose()


def test_upgrade_recreates_renamed_indexes(upgraded_populated_db):
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        insp = inspect(engine)
        member_idx = {i["name"] for i in insp.get_indexes("members")}
        assert "idx_members_fleet_status" in member_idx

        task_idx = {i["name"] for i in insp.get_indexes("tasks")}
        assert "idx_tasks_from_member_status_ts" in task_idx
        assert "idx_tasks_context_status_ts" in task_idx

        placement_idx = {i["name"] for i in insp.get_indexes("member_placements")}
        assert placement_idx == set()

        with engine.connect() as conn:
            old_names = conn.execute(
                text(
                    "SELECT name FROM sqlite_master WHERE type='index' AND name IN "
                    "('idx_agents_fleet_status', 'idx_placements_director', "
                    "'idx_tasks_from_agent_status_ts')"
                )
            ).fetchall()
        assert old_names == []
    finally:
        engine.dispose()


def test_upgrade_rewrites_fk_references_and_breaks_nothing(upgraded_populated_db):
    """SQLite's RENAME auto-rewrites FK definitions in referencing tables, and
    the placement batch recreate must not leave dangling references."""
    engine = create_engine(f"sqlite:///{upgraded_populated_db}")
    try:
        insp = inspect(engine)
        for table in ("fleets", "tasks", "monitor_config", "member_placements"):
            referred = {fk["referred_table"] for fk in insp.get_foreign_keys(table)}
            assert "agents" not in referred, table
            assert "agent_placements" not in referred, table

        member_referrers = ("fleets", "tasks", "monitor_config", "member_placements")
        for table in member_referrers:
            referred = {fk["referred_table"] for fk in insp.get_foreign_keys(table)}
            assert "members" in referred, table

        with engine.connect() as conn:
            violations = conn.execute(text("PRAGMA foreign_key_check")).fetchall()
        assert violations == []
    finally:
        engine.dispose()
