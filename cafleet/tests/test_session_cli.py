import json
import sqlite3
import uuid

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.db import engine as engine_mod
from cafleet.tmux import DirectorContext


@pytest.fixture(autouse=True)
def _reset_engine_singletons():
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None
    yield
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None


@pytest.fixture(autouse=True)
def _mock_tmux_for_session_create(monkeypatch):
    """Let ``session create`` succeed without a real tmux pane.

    Tests in this file predate the 0000026 director-context dependency,
    so a blanket stub is applied here; the outside-tmux failure path is
    covered explicitly in ``test_cli_session_bootstrap.py``.
    """
    ctx = DirectorContext(session="main", window_id="@3", pane_id="%0")
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: ctx)


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


def _session_rows(db_path) -> list[tuple]:
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


class TestSessionCreate:
    def test_creates_session_with_uuid(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create"])

        assert result.exit_code == 0, result.output
        output = result.output.strip()
        found_uuid = None
        for word in output.split():
            try:
                uuid.UUID(word)
                found_uuid = word
                break
            except ValueError:
                continue
        assert found_uuid is not None

        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert found_uuid in session_ids

    def test_creates_session_with_label(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--label", "PR-42 review"])

        assert result.exit_code == 0, result.output

        rows = _session_rows(db_file)
        assert len(rows) == 1
        assert rows[0][1] == "PR-42 review"

    def test_creates_session_without_label(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 0

        rows = _session_rows(db_file)
        assert len(rows) == 1
        assert rows[0][1] is None

    def test_creates_session_json_output(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--label", "test", "--json"])

        assert result.exit_code == 0, result.output
        data = json.loads(result.output)
        assert "session_id" in data
        uuid.UUID(data["session_id"])
        assert data["label"] == "test"

    def test_each_create_mints_unique_id(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        r1 = runner.invoke(cli, ["session", "create", "--json"])
        r2 = runner.invoke(cli, ["session", "create", "--json"])

        assert r1.exit_code == 0
        assert r2.exit_code == 0

        id1 = json.loads(r1.output)["session_id"]
        id2 = json.loads(r2.output)["session_id"]
        assert id1 != id2

        rows = _session_rows(db_file)
        assert len(rows) == 2

    def test_json_output_includes_administrator_agent_id(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--json"])

        assert result.exit_code == 0, result.output
        data = json.loads(result.output)
        assert "administrator_agent_id" in data
        uuid.UUID(data["administrator_agent_id"])

    def test_json_administrator_agent_id_matches_db_row(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        sid = data["session_id"]
        admin_id = data["administrator_agent_id"]

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
        _row_agent_id, row_session_id, row_name, row_status, row_card_json = rows[0]
        assert row_session_id == sid
        assert row_name == "Administrator"
        assert row_status == "active"
        card = json.loads(row_card_json)
        assert card["cafleet"]["kind"] == "builtin-administrator"

    # NOTE: the former ``test_non_json_output_unchanged_single_uuid_line``
    # (design 0000025 §B guard that the text path prints exactly one line)
    # is deliberately removed here. Design 0000026 §CLI-surface supersedes
    # that contract with a 7-line text shape (session_id, director
    # agent_id, label, created_at, director_name, pane, administrator).
    # The equivalent assertions for the NEW shape live in
    # ``test_cli_session_bootstrap.py::TestSessionCreateTextOutput``.


class TestSessionList:
    def test_lists_empty(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "list"])

        assert result.exit_code == 0, result.output

    def test_lists_sessions_with_agent_count(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="test-session")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(cli, ["session", "list"])

        assert result.exit_code == 0
        assert sid in result.output
        assert "test-session" in result.output

    def test_lists_json_output(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="json-test")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

        result = runner.invoke(cli, ["session", "list", "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert isinstance(data, list)
        assert len(data) == 1
        assert data[0]["session_id"] == sid
        assert data[0]["label"] == "json-test"
        assert data[0]["agent_count"] == 1

    def test_lists_multiple_sessions(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        _seed_session(db_file, sid_a, label="session-a")
        _seed_session(db_file, sid_b, label="session-b")

        result = runner.invoke(cli, ["session", "list", "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert len(data) == 2
        ids = {d["session_id"] for d in data}
        assert sid_a in ids
        assert sid_b in ids

    def test_agent_count_only_active(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(cli, ["session", "list", "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data[0]["agent_count"] == 1


class TestSessionShow:
    def test_shows_existing_session(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="show-test")

        result = runner.invoke(cli, ["session", "show", sid])

        assert result.exit_code == 0, result.output
        assert sid in result.output
        assert "show-test" in result.output

    def test_shows_json_output(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="json-show")

        result = runner.invoke(cli, ["session", "show", sid, "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data["session_id"] == sid
        assert data["label"] == "json-show"
        assert "created_at" in data

    def test_missing_session_exits_nonzero(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        fake_id = str(uuid.uuid4())
        result = runner.invoke(cli, ["session", "show", fake_id])

        assert result.exit_code == 1, result.output
        output_lower = result.output.lower()
        assert "not found" in output_lower

    def test_soft_deleted_session_surfaces_deleted_at_line(self, tmp_path, monkeypatch):
        """Design 0000026: ``get_session`` intentionally returns soft-deleted
        rows (exposing ``deleted_at`` for audit), but pre-fix the text output
        dropped that field, so a soft-deleted session was visually identical
        to an active one. This regression guard pins the new behavior: when
        ``deleted_at`` is non-NULL, text output includes a ``deleted_at:``
        line so users can distinguish it without parsing JSON.
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="audit-me")
        # Flip deleted_at directly; bypass the cascade so the rest of the DB
        # stays untouched and we only pin the show-output behavior.
        conn = sqlite3.connect(str(db_file))
        try:
            conn.execute(
                "UPDATE sessions SET deleted_at = ? WHERE session_id = ?",
                ("2026-04-16T10:00:00+00:00", sid),
            )
            conn.commit()
        finally:
            conn.close()

        result = runner.invoke(cli, ["session", "show", sid])

        assert result.exit_code == 0, result.output
        assert "deleted_at:" in result.output
        assert "2026-04-16T10:00:00+00:00" in result.output

    def test_active_session_does_not_print_deleted_at_line(self, tmp_path, monkeypatch):
        """Symmetric check: an active (``deleted_at IS NULL``) session must
        NOT show a ``deleted_at:`` line — otherwise users would see a blank
        or misleading field on every healthy session.
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="live-session")

        result = runner.invoke(cli, ["session", "show", sid])

        assert result.exit_code == 0, result.output
        assert "deleted_at" not in result.output


class TestSessionDelete:
    def test_deletes_session(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)

        result = runner.invoke(cli, ["session", "delete", sid])

        assert result.exit_code == 0, result.output
        # Soft delete: row stays but deleted_at is set, list_sessions hides it.
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert sid in session_ids
        assert _session_deleted_at(db_file, sid) is not None
        assert "deleted" in result.output.lower()

    def test_delete_nonexistent_session(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        fake_id = str(uuid.uuid4())
        result = runner.invoke(cli, ["session", "delete", fake_id])

        assert result.exception is None or isinstance(result.exception, SystemExit)

    def test_delete_session_with_active_agents_succeeds(self, tmp_path, monkeypatch):
        """Session with active agents soft-deletes successfully and flips them
        to ``deregistered``.

        Design 0000026: ``session delete`` is a soft delete that deregisters
        every active agent in the session (including the root Director and
        the Administrator) in the same transaction. The session row stays
        but gains a ``deleted_at`` timestamp.
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

        result = runner.invoke(cli, ["session", "delete", sid])

        assert result.exit_code == 0, result.output
        assert _session_deleted_at(db_file, sid) is not None

    def test_delete_session_with_deregistered_agents_succeeds(
        self, tmp_path, monkeypatch
    ):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(cli, ["session", "delete", sid])

        assert result.exit_code == 0, result.output
        assert _session_deleted_at(db_file, sid) is not None


class TestDbInitNoAutoSession:
    def test_db_init_creates_no_sessions(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        rows = _session_rows(db_file)
        assert len(rows) == 0


class TestSessionGroupStructure:
    def test_session_group_exists(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "--help"])

        assert result.exit_code == 0, result.output
        output_lower = result.output.lower()
        assert "create" in output_lower
        assert "list" in output_lower
        assert "show" in output_lower
        assert "delete" in output_lower

    def test_session_is_not_under_db(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["db", "session", "create"])

        assert result.exit_code == 2, result.output
