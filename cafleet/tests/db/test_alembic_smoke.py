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
            "members",
            "messages",
            "member_placements",
            "monitor_config",
            "monitor_runtime",
            "skill_installs",
            "alembic_version",
        }
        assert tables == expected
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


def test_single_initial_migration_revision_exists():
    """The migration history is a single fresh initial revision (0001) with no
    predecessor, which is the head."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        script = ScriptDirectory.from_config(cfg)
        revisions = list(script.walk_revisions())

    assert len(revisions) == 1
    assert revisions[0].revision == "0001"
    assert revisions[0].down_revision is None
    assert script.get_current_head() == "0001"


def test_minted_id_tables_declare_autoincrement(alembic_upgraded_db):
    """``fleets``/``members``/``messages`` mint ids via AUTOINCREMENT; placements do not."""
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

            for table in ("fleets", "members", "messages"):
                ddl = conn.execute(
                    text(
                        "SELECT sql FROM sqlite_master WHERE type='table' AND name=:t"
                    ),
                    {"t": table},
                ).scalar()
                assert "AUTOINCREMENT" in ddl.upper()

            # member_placements.member_id is the members FK reused as a 1:1 PK —
            # explicitly NOT AUTOINCREMENT.
            placements_ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='member_placements'"
                )
            ).scalar()
            assert "AUTOINCREMENT" not in placements_ddl.upper()
    finally:
        engine.dispose()


@pytest.mark.parametrize(
    ("table", "pk_column"),
    [
        ("fleets", "fleet_id"),
        ("members", "member_id"),
        ("messages", "message_id"),
        ("member_placements", "member_id"),
        ("monitor_config", "member_id"),
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


def test_member_placements_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "member_placements" in tables

        # exact column set — member-ness derives from the fleets join, so the
        # placement table carries no director column
        cols = {col["name"]: col for col in insp.get_columns("member_placements")}
        assert set(cols) == {
            "member_id",
            "backend",
            "coding_agent",
            "mux_session",
            "mux_window_id",
            "mux_pane_id",
            "created_at",
        }

        # NULL = pending placement before the pane is spawned
        assert cols["mux_pane_id"]["nullable"] is True

        for name in (
            "member_id",
            "backend",
            "mux_session",
            "mux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False

        # backend defaults to 'tmux'
        assert "tmux" in str(cols["backend"]["default"])

        assert insp.get_indexes("member_placements") == []
    finally:
        engine.dispose()


def test_messages_table_columns(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("messages")}

        assert set(cols) == {
            "message_id",
            "owner_member_id",
            "from_member_id",
            "to_member_id",
            "type",
            "created_at",
            "status_state",
            "status_timestamp",
            "origin_message_id",
            "text",
        }
    finally:
        engine.dispose()


def test_messages_indexes_renamed(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        indexes = {idx["name"]: idx for idx in insp.get_indexes("messages")}

        assert set(indexes) == {
            "idx_messages_owner_member_status_ts",
            "idx_messages_from_member_status_ts",
        }
        assert indexes["idx_messages_owner_member_status_ts"]["column_names"] == [
            "owner_member_id",
            "status_timestamp",
        ]
        assert indexes["idx_messages_from_member_status_ts"]["column_names"] == [
            "from_member_id",
            "status_timestamp",
        ]
    finally:
        engine.dispose()


def test_messages_owner_member_fk_restrict(alembic_upgraded_db):
    """``messages.owner_member_id`` keeps the members FK with ON DELETE RESTRICT."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        fks = insp.get_foreign_keys("messages")
        assert len(fks) == 1
        assert fks[0]["constrained_columns"] == ["owner_member_id"]
        assert fks[0]["referred_table"] == "members"
        assert fks[0]["referred_columns"] == ["member_id"]

        with engine.connect() as conn:
            ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='messages'"
                )
            ).scalar()
        assert ddl is not None
        assert "ON DELETE RESTRICT" in ddl.upper()
    finally:
        engine.dispose()


def test_messages_table_has_origin_message_id_column(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("messages")}

        assert "origin_message_id" in cols

        # Nullable: a unicast message references no origin, so it stores NULL
        assert cols["origin_message_id"]["nullable"] is True
    finally:
        engine.dispose()


def test_messages_to_member_id_is_nullable_after_migration(alembic_upgraded_db):
    """``messages.to_member_id`` is nullable so broadcast-summary rows persist NULL."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("messages")}

        assert cols["to_member_id"]["nullable"] is True
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
            "member_id",
            "interval_seconds",
            "last_ping_at",
            "enabled",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL last_ping_at = never pinged ⇒ due immediately
        assert cols["last_ping_at"]["nullable"] is True

        # schedule columns are NOT NULL
        for name in ("member_id", "interval_seconds", "enabled"):
            assert cols[name]["nullable"] is False

        # defaults per §2: interval_seconds 60, enabled 1
        assert _default_int(cols, "interval_seconds") == 60
        assert _default_int(cols, "enabled") == 1

        # member_id is the members FK reused as a 1:1 PK
        fks = insp.get_foreign_keys("monitor_config")
        assert any(fk["referred_table"] == "members" for fk in fks)
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
    """monitor_config (member_id) and monitor_runtime (fleet_id) reuse a parent id 1:1 — no AUTOINCREMENT."""
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
