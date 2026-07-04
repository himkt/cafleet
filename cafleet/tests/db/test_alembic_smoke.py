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
        importlib.resources.files("cafleet.db") / "alembic.ini"
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


def test_alembic_version_table_records_head_0007(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert rows == [("0007",)]
    finally:
        engine.dispose()


def test_seven_migration_revisions_exist():
    """The migration history is seven revisions: 0001 (base) → 0002 (monitor
    tables) → 0003 (prune non-Director monitor_config rows) → 0004 (prune the
    root-Director monitor_config rows) → 0005 (per-member intervals: prune the
    monitoring member, backfill the root Director @180 + ordinary members @720)
    → 0006 (skill_installs: the per-home record of the installing CLI version)
    → 0007 (tasks.to_agent_id → nullable so broadcast-summary rows persist
    NULL)."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        script = ScriptDirectory.from_config(cfg)
        revisions = list(script.walk_revisions())

    assert len(revisions) == 7
    by_revision = {rev.revision: rev for rev in revisions}
    assert set(by_revision) == {
        "0001",
        "0002",
        "0003",
        "0004",
        "0005",
        "0006",
        "0007",
    }
    assert by_revision["0001"].down_revision is None
    assert by_revision["0002"].down_revision == "0001"
    assert by_revision["0003"].down_revision == "0002"
    assert by_revision["0004"].down_revision == "0003"
    assert by_revision["0005"].down_revision == "0004"
    assert by_revision["0006"].down_revision == "0005"
    assert by_revision["0007"].down_revision == "0006"
    assert script.get_current_head() == "0007"


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
            "tmux_session",
            "tmux_window_id",
            "tmux_pane_id",
            "created_at",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL = pending placement before the pane is spawned
        assert cols["tmux_pane_id"]["nullable"] is True

        # NULL marks the root Director's own placement (no parent)
        assert cols["director_agent_id"]["nullable"] is True

        for name in (
            "agent_id",
            "tmux_session",
            "tmux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False

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
    """0007 alters ``tasks.to_agent_id`` to nullable so broadcast-summary rows
    persist NULL instead of the ``0`` sentinel (design 0000118, item 1.1)."""
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
    """0006 creates ``skill_installs``: three NOT NULL string columns with
    ``coding_agent`` (a known home key, not a minted id) as the PK."""
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


# 0005 data-migration fixture ids — one of each enrollment role (§9).
_FLEET_ID = 1
_DIRECTOR_ID = 10
_MEMBER_ID = 20
_MONITORING_MEMBER_ID = 30
_ADMINISTRATOR_ID = 40


def _seed_pre_0005_fleet(engine):
    """Seed a fleet at revision 0004: one of each role, every agent pane-bound,
    with the only enrolled monitor_config row being the monitoring member's
    (the post-0004 invariant). The Administrator is given a placement too, so the
    only thing that can keep it unenrolled after 0005 is the kind guard.

    Inserts are ordered to dodge the fleets↔agents circular FK: the fleet lands
    first with a NULL director, agents/placements next, then the director link is
    closed.
    """
    ts = "2026-06-17T00:00:00+00:00"
    agents = [
        (_DIRECTOR_ID, "director", '{"cafleet": {"kind": "director"}}'),
        (_MEMBER_ID, "member", '{"cafleet": {"kind": "member"}}'),
        (
            _MONITORING_MEMBER_ID,
            "monitor",
            '{"cafleet": {"kind": "monitoring-member"}}',
        ),
        (
            _ADMINISTRATOR_ID,
            "Administrator",
            '{"cafleet": {"kind": "builtin-administrator"}}',
        ),
    ]
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO fleets (fleet_id, label, created_at, deleted_at, "
                "director_agent_id) VALUES (:fid, 'mig', :ts, NULL, NULL)"
            ),
            {"fid": _FLEET_ID, "ts": ts},
        )
        for agent_id, name, card in agents:
            conn.execute(
                text(
                    "INSERT INTO agents (agent_id, fleet_id, name, description, "
                    "status, registered_at, agent_card_json) "
                    "VALUES (:aid, :fid, :name, 'seed', 'active', :ts, :card)"
                ),
                {
                    "aid": agent_id,
                    "fid": _FLEET_ID,
                    "name": name,
                    "ts": ts,
                    "card": card,
                },
            )
            conn.execute(
                text(
                    "INSERT INTO agent_placements (agent_id, tmux_session, "
                    "tmux_window_id, tmux_pane_id, created_at) "
                    "VALUES (:aid, 'sess', '@1', :pane, :ts)"
                ),
                {"aid": agent_id, "pane": f"%{agent_id}", "ts": ts},
            )
        conn.execute(
            text("UPDATE fleets SET director_agent_id = :did WHERE fleet_id = :fid"),
            {"did": _DIRECTOR_ID, "fid": _FLEET_ID},
        )
        conn.execute(
            text(
                "INSERT INTO monitor_config (agent_id, interval_seconds, enabled) "
                "VALUES (:aid, 60, 1)"
            ),
            {"aid": _MONITORING_MEMBER_ID},
        )


def test_0005_prunes_monitoring_member_and_backfills_director_and_member(tmp_path):
    """0005 deletes the monitoring member's monitor_config row, backfills an
    active root Director @180 and an active ordinary member @720, and leaves the
    Administrator unenrolled (§9). The DB is staged at 0004 (monitoring-member-
    only enrollment), seeded with one of each role, then upgraded to 0005."""
    db_path = tmp_path / "migration_0005.db"
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        command.upgrade(cfg, "0004")

        engine = create_engine(f"sqlite:///{db_path}")
        try:
            _seed_pre_0005_fleet(engine)
            command.upgrade(cfg, "0005")
            with engine.connect() as conn:
                configs = {
                    row.agent_id: row
                    for row in conn.execute(
                        text(
                            "SELECT agent_id, interval_seconds, enabled "
                            "FROM monitor_config"
                        )
                    )
                }
        finally:
            engine.dispose()

    # step 1: the monitoring member's pre-existing row is pruned.
    assert _MONITORING_MEMBER_ID not in configs
    # step 2: the active root Director is backfilled @180, enabled.
    assert _DIRECTOR_ID in configs
    assert configs[_DIRECTOR_ID].interval_seconds == 180
    assert configs[_DIRECTOR_ID].enabled == 1
    # step 3: the active ordinary member is backfilled @720, enabled.
    assert _MEMBER_ID in configs
    assert configs[_MEMBER_ID].interval_seconds == 720
    assert configs[_MEMBER_ID].enabled == 1
    # the Administrator stays unenrolled despite its placement (kind guard).
    assert _ADMINISTRATOR_ID not in configs
