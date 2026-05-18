"""Tests for ``cafleet session *`` CLI verbs."""

import json
import sqlite3
import uuid

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext as DirectorContext


@pytest.fixture(autouse=True)
def _autouse_reset_engine(_reset_engine_singletons):
    pass


@pytest.fixture(autouse=True)
def _mock_tmux_for_session_create(monkeypatch):
    ctx = DirectorContext(session="main", window_id="@3", pane_id="%0")
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available",
        lambda self: None,
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery",
        lambda self: ctx,
    )


def _init_db(runner: CliRunner) -> None:
    result = runner.invoke(cli, ["db", "init"])
    assert result.exit_code == 0, result.output


def _seed_session(db_path, session_id: str, label: str | None = None) -> None:
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT INTO sessions (session_id, label, created_at) VALUES (?, ?, ?)",
            (session_id, label, "2026-01-01T00:00:00+00:00"),
        )
        conn.commit()
    finally:
        conn.close()


def _seed_agent(
    db_path, agent_id: str, session_id: str, *, status: str = "active"
) -> None:
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT INTO agents "
            "(agent_id, session_id, name, description, status, "
            "registered_at, deregistered_at, agent_card_json) "
            "VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            (
                agent_id,
                session_id,
                f"agent-{agent_id[:8]}",
                "test agent",
                status,
                "2026-01-01T00:00:00+00:00",
                "{}",
            ),
        )
        conn.commit()
    finally:
        conn.close()


def _session_rows(db_path):
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT session_id, label, created_at FROM sessions"
        ).fetchall()
    finally:
        conn.close()


def _session_deleted_at(db_path, session_id: str) -> str | None:
    conn = sqlite3.connect(str(db_path))
    try:
        row = conn.execute(
            "SELECT deleted_at FROM sessions WHERE session_id = ?", (session_id,)
        ).fetchone()
    finally:
        conn.close()
    return None if row is None else row[0]


@pytest.fixture
def fresh_db(tmp_path, monkeypatch):
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    runner = CliRunner()
    _init_db(runner)
    return db_file, runner


@pytest.mark.parametrize("output_mode", ["text", "json"])
def test_session_create__happy_path(fresh_db, output_mode):
    db_file, runner = fresh_db
    args = ["session", "create"]
    if output_mode == "json":
        args.append("--json")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0, result.output
    if output_mode == "json":
        data = json.loads(result.output)
        assert "session_id" in data
        uuid.UUID(data["session_id"])
    else:
        # Find a UUID in the text output.
        found = next(
            (w for w in result.output.split() if _is_uuid(w)),
            None,
        )
        assert found is not None
    rows = _session_rows(db_file)
    assert len(rows) == 1


def _is_uuid(word: str) -> bool:
    try:
        uuid.UUID(word)
        return True
    except ValueError:
        return False


def test_session_create__label_round_trip_and_default_none(fresh_db):
    db_file, runner = fresh_db
    result = runner.invoke(cli, ["session", "create", "--label", "PR-42 review"])
    assert result.exit_code == 0, result.output
    assert _session_rows(db_file)[0][1] == "PR-42 review"

    # Default label is None.
    result2 = runner.invoke(cli, ["session", "create"])
    assert result2.exit_code == 0
    labels = sorted([r[1] or "" for r in _session_rows(db_file)])
    assert "" in labels
    assert "PR-42 review" in labels


def test_session_create__each_create_mints_unique_id(fresh_db):
    db_file, runner = fresh_db
    r1 = runner.invoke(cli, ["session", "create", "--json"])
    r2 = runner.invoke(cli, ["session", "create", "--json"])
    assert json.loads(r1.output)["session_id"] != json.loads(r2.output)["session_id"]
    assert len(_session_rows(db_file)) == 2


def test_session_create__bootstraps_administrator_recorded_in_db(fresh_db):
    db_file, runner = fresh_db
    result = runner.invoke(cli, ["session", "create", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    sid = data["session_id"]
    admin_id = data["administrator_agent_id"]
    uuid.UUID(admin_id)

    conn = sqlite3.connect(str(db_file))
    try:
        rows = conn.execute(
            "SELECT agent_id, session_id, name, status, agent_card_json "
            "FROM agents WHERE agent_id = ?",
            (admin_id,),
        ).fetchall()
    finally:
        conn.close()
    assert len(rows) == 1
    _aid, row_sid, row_name, row_status, row_card = rows[0]
    assert row_sid == sid
    assert row_name == "Administrator"
    assert row_status == "active"
    card = json.loads(row_card)
    assert card["cafleet"]["kind"] == "builtin-administrator"


@pytest.mark.parametrize("output_mode", ["text", "json"])
def test_session_list__shape_and_agent_count(fresh_db, output_mode):
    db_file, runner = fresh_db
    sid = str(uuid.uuid4())
    _seed_session(db_file, sid, label="test-session")
    _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
    _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
    _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

    args = ["session", "list"]
    if output_mode == "json":
        args.append("--json")
    result = runner.invoke(cli, args)
    assert result.exit_code == 0
    if output_mode == "json":
        data = json.loads(result.output)
        assert len(data) == 1
        assert data[0]["session_id"] == sid
        assert data[0]["label"] == "test-session"
        assert data[0]["agent_count"] == 2  # active only
    else:
        assert sid in result.output
        assert "test-session" in result.output


@pytest.mark.parametrize(
    ("scenario", "expected_exit", "expected_in", "expected_not_in"),
    [
        ("existing_active", 0, ["show-test"], ["deleted_at"]),
        ("missing", 1, ["not found"], []),
        (
            "soft_deleted_surfaces_deleted_at",
            0,
            ["deleted_at:", "2026-04-16T10:00:00+00:00"],
            [],
        ),
    ],
)
def test_session_show__shape_and_branches(
    fresh_db, scenario, expected_exit, expected_in, expected_not_in
):
    db_file, runner = fresh_db
    if scenario == "existing_active":
        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="show-test")
        target = sid
    elif scenario == "missing":
        target = str(uuid.uuid4())
    else:
        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="audit-me")
        conn = sqlite3.connect(str(db_file))
        try:
            conn.execute(
                "UPDATE sessions SET deleted_at = ? WHERE session_id = ?",
                ("2026-04-16T10:00:00+00:00", sid),
            )
            conn.commit()
        finally:
            conn.close()
        target = sid

    result = runner.invoke(cli, ["session", "show", target])
    assert result.exit_code == expected_exit, result.output
    out = result.output.lower() if scenario == "missing" else result.output
    for needle in expected_in:
        check = needle.lower() if scenario == "missing" else needle
        assert check in out
    for needle in expected_not_in:
        assert needle not in result.output


def test_session_delete__soft_deletes_and_marks_row(fresh_db):
    db_file, runner = fresh_db
    sid = str(uuid.uuid4())
    _seed_session(db_file, sid)
    _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
    _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

    result = runner.invoke(cli, ["session", "delete", sid])
    assert result.exit_code == 0, result.output
    # Row persists, deleted_at set.
    assert sid in [r[0] for r in _session_rows(db_file)]
    assert _session_deleted_at(db_file, sid) is not None
    assert "deleted" in result.output.lower()


def test_session_delete__nonexistent_session_handles_gracefully(fresh_db):
    _db_file, runner = fresh_db
    fake_id = str(uuid.uuid4())
    result = runner.invoke(cli, ["session", "delete", fake_id])
    # Either exits non-zero or returns SystemExit — never crashes uncleanly.
    assert result.exception is None or isinstance(result.exception, SystemExit)


def test_session_group_structure__subcommands_under_session_not_db(fresh_db):
    _db_file, runner = fresh_db
    help_result = runner.invoke(cli, ["session", "--help"])
    assert help_result.exit_code == 0
    out = help_result.output.lower()
    for verb in ("create", "list", "show", "delete"):
        assert verb in out

    # session is NOT exposed under db.
    not_under_db = runner.invoke(cli, ["db", "session", "create"])
    assert not_under_db.exit_code == 2, not_under_db.output

    # db init creates no sessions (regression guard).
    rows = _session_rows(_db_file)
    assert len(rows) == 0
