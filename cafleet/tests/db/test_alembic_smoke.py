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
            "monitor_report_delivery",
            "monitor_director_gate",
            "asset_installs",
            "alembic_version",
        }
        assert tables == expected
    finally:
        engine.dispose()


def test_alembic_version_table_records_head_0005(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert rows == [("0005",)]
    finally:
        engine.dispose()


def test_five_revision_migration_chain_exists():
    """The migration history is linear through the monitor episode revision."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        script = ScriptDirectory.from_config(cfg)
        revisions = list(script.walk_revisions())

    assert len(revisions) == 5
    assert revisions[0].revision == "0005"
    assert revisions[0].down_revision == "0004"
    assert revisions[1].revision == "0004"
    assert revisions[1].down_revision == "0003"
    assert revisions[2].revision == "0003"
    assert revisions[2].down_revision == "0002"
    assert revisions[3].revision == "0002"
    assert revisions[3].down_revision == "0001"
    assert revisions[4].revision == "0001"
    assert revisions[4].down_revision is None
    assert script.get_current_head() == "0005"


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
        ("monitor_report_delivery", "message_id"),
        ("monitor_director_gate", "fleet_id"),
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

        # coding_agent declares no column default — the backend is always an
        # explicit, operator-declared or inherited value
        assert cols["coding_agent"]["default"] is None

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
            "last_stall_check_at",
            "last_stall_candidate_at",
            "last_stall_capture_sha256",
            "stall_episode_state",
            "stall_escalation_reason",
        }
        assert set(cols) == expected_cols

        for name in (
            "last_ping_at",
            "last_stall_check_at",
            "last_stall_candidate_at",
            "last_stall_capture_sha256",
            "stall_escalation_reason",
        ):
            assert cols[name]["nullable"] is True

        # schedule columns are NOT NULL
        for name in (
            "member_id",
            "interval_seconds",
            "enabled",
            "stall_episode_state",
        ):
            assert cols[name]["nullable"] is False

        # Existing schedule defaults remain; episode state backfills to clear.
        assert _default_int(cols, "interval_seconds") == 60
        assert _default_int(cols, "enabled") == 1
        assert "clear" in str(cols["stall_episode_state"]["default"])

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
    """Monitor table keys reuse parent ids; none mint an independent id."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            for table in (
                "monitor_config",
                "monitor_runtime",
                "monitor_report_delivery",
                "monitor_director_gate",
            ):
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


def test_monitor_report_delivery_schema_constraints_and_indexes(
    alembic_upgraded_db,
):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("monitor_report_delivery")}
        assert set(cols) == {
            "message_id",
            "fleet_id",
            "preview_state",
            "attempt_count",
            "last_attempt_at",
            "delivered_at",
        }
        for name in ("message_id", "fleet_id", "preview_state", "attempt_count"):
            assert cols[name]["nullable"] is False
        assert cols["last_attempt_at"]["nullable"] is True
        assert cols["delivered_at"]["nullable"] is True
        assert "pending" in str(cols["preview_state"]["default"])
        assert _default_int(cols, "attempt_count") == 0

        fks = {
            tuple(fk["constrained_columns"]): fk
            for fk in insp.get_foreign_keys("monitor_report_delivery")
        }
        assert fks[("message_id",)]["referred_table"] == "messages"
        assert fks[("fleet_id",)]["referred_table"] == "fleets"

        indexes = insp.get_indexes("monitor_report_delivery")
        assert any(
            idx["unique"] and idx["column_names"] == ["fleet_id"] for idx in indexes
        )
        assert any(
            idx["column_names"] == ["fleet_id", "preview_state", "message_id"]
            for idx in indexes
        )

        with engine.connect() as conn:
            ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='monitor_report_delivery'"
                )
            ).scalar_one()
        for term in (
            "pending",
            "awaiting_ack",
            "delivered",
            "attempt_count >= 0",
            "last_attempt_at",
            "delivered_at",
        ):
            assert term.upper() in ddl.upper()
    finally:
        engine.dispose()


def test_monitor_director_gate_schema_constraints(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("monitor_director_gate")}
        assert set(cols) == {
            "fleet_id",
            "director_member_id",
            "token_sha256",
            "classification",
            "issued_at",
            "expires_at",
        }
        assert all(not col["nullable"] for col in cols.values())

        fks = {
            tuple(fk["constrained_columns"]): fk
            for fk in insp.get_foreign_keys("monitor_director_gate")
        }
        assert fks[("fleet_id",)]["referred_table"] == "fleets"
        assert fks[("director_member_id",)]["referred_table"] == "members"

        with engine.connect() as conn:
            ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='monitor_director_gate'"
                )
            ).scalar_one()
        for term in ("finished", "stalled", "token_sha256", "expires_at"):
            assert term.upper() in ddl.upper()
    finally:
        engine.dispose()


def test_migration_0005_backfills_existing_monitor_rows_and_downgrades(tmp_path):
    db_path = tmp_path / "monitor-episode.db"

    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        command.upgrade(cfg, "0004")

        engine = create_engine(f"sqlite:///{db_path}")
        try:
            with engine.begin() as conn:
                conn.execute(
                    text(
                        "INSERT INTO fleets"
                        " (fleet_id, name, created_at, deleted_at, director_member_id)"
                        " VALUES (1, 'fleet', :now, NULL, NULL)"
                    ),
                    {"now": "2026-07-28T00:00:00+00:00"},
                )
                conn.execute(
                    text(
                        "INSERT INTO members"
                        " (member_id, fleet_id, name, description, status,"
                        " registered_at, deregistered_at, member_card_json)"
                        " VALUES (1, 1, 'director', 'root', 'active',"
                        " :now, NULL, '{}')"
                    ),
                    {"now": "2026-07-28T00:00:00+00:00"},
                )
                conn.execute(
                    text("UPDATE fleets SET director_member_id = 1 WHERE fleet_id = 1")
                )
                conn.execute(
                    text(
                        "INSERT INTO monitor_config"
                        " (member_id, interval_seconds, last_ping_at, enabled)"
                        " VALUES (1, 180, NULL, 1)"
                    )
                )
        finally:
            engine.dispose()

        command.upgrade(cfg, "0005")
        engine = create_engine(f"sqlite:///{db_path}")
        try:
            with engine.connect() as conn:
                row = conn.execute(
                    text(
                        "SELECT last_stall_check_at, last_stall_candidate_at,"
                        " last_stall_capture_sha256, stall_episode_state,"
                        " stall_escalation_reason FROM monitor_config"
                        " WHERE member_id = 1"
                    )
                ).one()
            assert tuple(row) == (None, None, None, "clear", None)
        finally:
            engine.dispose()

        command.downgrade(cfg, "0004")
        engine = create_engine(f"sqlite:///{db_path}")
        try:
            insp = inspect(engine)
            assert "monitor_report_delivery" not in insp.get_table_names()
            assert "monitor_director_gate" not in insp.get_table_names()
            cols = {col["name"] for col in insp.get_columns("monitor_config")}
            assert "last_stall_check_at" not in cols
            assert "last_stall_candidate_at" not in cols
            assert "last_stall_capture_sha256" not in cols
            assert "stall_episode_state" not in cols
            assert "stall_escalation_reason" not in cols
        finally:
            engine.dispose()


def test_asset_installs_table_created_by_migration(alembic_upgraded_db):
    """The migrated head schema carries ``asset_installs``: three NOT NULL string
    columns with ``coding_agent`` (a known home key, not a minted id) as the PK."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "asset_installs" in tables

        cols = {col["name"]: col for col in insp.get_columns("asset_installs")}
        assert set(cols) == {"coding_agent", "cafleet_version", "installed_at"}
        for name in ("coding_agent", "cafleet_version", "installed_at"):
            assert cols[name]["nullable"] is False

        pk = insp.get_pk_constraint("asset_installs")
        assert pk["constrained_columns"] == ["coding_agent"]

        with engine.connect() as conn:
            ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='asset_installs'"
                )
            ).scalar()
        assert ddl is not None
        assert "AUTOINCREMENT" not in ddl.upper()
    finally:
        engine.dispose()


def test_migration_0003_moves_rows_both_directions(tmp_path):
    """Upgrading ``0002`` → ``0003`` copies every ``skill_installs`` row into
    ``asset_installs`` and drops the old table; downgrading mirrors it back."""
    db_path = tmp_path / "preserve.db"
    row = ("claude", "0.6.0", "2026-07-18T00:00:00+00:00")

    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic" / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")

        command.upgrade(cfg, "0002")
        engine = create_engine(f"sqlite:///{db_path}")
        try:
            with engine.begin() as conn:
                conn.execute(
                    text(
                        "INSERT INTO skill_installs"
                        " (coding_agent, cafleet_version, installed_at)"
                        " VALUES (:a, :v, :t)"
                    ),
                    {"a": row[0], "v": row[1], "t": row[2]},
                )
        finally:
            engine.dispose()

        command.upgrade(cfg, "0003")
        engine = create_engine(f"sqlite:///{db_path}")
        try:
            insp = inspect(engine)
            assert "skill_installs" not in insp.get_table_names()
            with engine.connect() as conn:
                rows = conn.execute(
                    text(
                        "SELECT coding_agent, cafleet_version, installed_at"
                        " FROM asset_installs"
                    )
                ).fetchall()
            assert rows == [row]
        finally:
            engine.dispose()

        command.downgrade(cfg, "0002")
        engine = create_engine(f"sqlite:///{db_path}")
        try:
            insp = inspect(engine)
            assert "asset_installs" not in insp.get_table_names()
            with engine.connect() as conn:
                rows = conn.execute(
                    text(
                        "SELECT coding_agent, cafleet_version, installed_at"
                        " FROM skill_installs"
                    )
                ).fetchall()
            assert rows == [row]
        finally:
            engine.dispose()
