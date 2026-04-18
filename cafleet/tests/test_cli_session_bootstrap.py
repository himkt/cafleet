"""CLI-level tests for the session-bootstrap surface (design 0000026)."""

import json
import sqlite3
import uuid

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.db import engine as engine_mod
from cafleet.tmux import DirectorContext, TmuxError

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture(autouse=True)
def _reset_engine_singletons():
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None
    yield
    engine_mod._sync_engine = None
    engine_mod._sync_sessionmaker = None


@pytest.fixture
def db_file(tmp_path, monkeypatch):
    path = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{path}",
    )
    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, init.output
    return path


@pytest.fixture
def mock_tmux_ok(monkeypatch):
    from cafleet import cli as cli_mod

    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _FAKE_DIRECTOR_CTX)
    if hasattr(cli_mod, "ensure_tmux_available"):
        monkeypatch.setattr(cli_mod, "ensure_tmux_available", lambda: None)
    if hasattr(cli_mod, "director_context"):
        monkeypatch.setattr(cli_mod, "director_context", lambda: _FAKE_DIRECTOR_CTX)


@pytest.fixture
def mock_tmux_unavailable(monkeypatch):
    from cafleet import cli as cli_mod

    def _raise():
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", _raise)
    if hasattr(cli_mod, "ensure_tmux_available"):
        monkeypatch.setattr(cli_mod, "ensure_tmux_available", _raise)


def _session_rows(db_path) -> list[tuple]:
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT session_id, label, created_at, deleted_at, director_agent_id "
            "FROM sessions"
        ).fetchall()
    finally:
        conn.close()


def _agent_rows(db_path) -> list[tuple]:
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT agent_id, session_id, name, status FROM agents"
        ).fetchall()
    finally:
        conn.close()


class TestSessionCreateTextOutput:
    def test_line_1_is_session_id_and_line_2_is_director_agent_id(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 0, result.output

        lines = [ln for ln in result.output.splitlines() if ln.strip() != ""]
        assert len(lines) >= 2

        sid_line = lines[0].strip()
        uuid.UUID(sid_line)

        director_line = lines[1].strip()
        uuid.UUID(director_line)
        assert director_line != sid_line

        rows = _session_rows(db_file)
        assert any(r[0] == sid_line for r in rows)

    def test_contains_label_director_name_pane_and_administrator_labels(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--label", "bootstrap-check"])
        assert result.exit_code == 0

        text = result.output
        assert "label:" in text
        assert "director_name:" in text
        assert "pane:" in text
        assert "administrator:" in text

        assert "director" in text
        assert "bootstrap-check" in text

        assert (
            f"{_FAKE_DIRECTOR_CTX.session}:{_FAKE_DIRECTOR_CTX.window_id}:"
            f"{_FAKE_DIRECTOR_CTX.pane_id}" in text
        )

    def test_administrator_line_references_the_seeded_administrator(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 0

        admin_line = next(
            (
                ln.strip()
                for ln in result.output.splitlines()
                if ln.strip().lower().startswith("administrator")
            ),
            None,
        )
        assert admin_line is not None
        admin_id = None
        for token in admin_line.replace(":", " ").split():
            try:
                uuid.UUID(token)
                admin_id = token
                break
            except ValueError:
                continue
        assert admin_id is not None

        rows = _agent_rows(db_file)
        matches = [r for r in rows if r[0] == admin_id]
        assert len(matches) == 1
        assert matches[0][2] == "Administrator"


class TestSessionCreateJsonOutput:
    def test_top_level_keys(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0, result.output

        data = json.loads(result.output)
        for key in (
            "session_id",
            "label",
            "created_at",
            "administrator_agent_id",
            "director",
        ):
            assert key in data

        uuid.UUID(data["administrator_agent_id"])

    def test_director_sub_dict(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        director = data["director"]
        for key in ("agent_id", "name", "description", "registered_at", "placement"):
            assert key in director
        assert director["name"] == "Director"
        assert director["description"] == "Root Director for this session"
        uuid.UUID(director["agent_id"])

    def test_placement_sub_dict_matches_spec(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        placement = data["director"]["placement"]

        assert placement["director_agent_id"] is None
        assert placement["coding_agent"] == "unknown"
        assert placement["tmux_session"] == _FAKE_DIRECTOR_CTX.session
        assert placement["tmux_window_id"] == _FAKE_DIRECTOR_CTX.window_id
        assert placement["tmux_pane_id"] == _FAKE_DIRECTOR_CTX.pane_id
        assert "created_at" in placement

    def test_label_propagates_to_json(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(
            cli, ["session", "create", "--label", "json-label", "--json"]
        )
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data["label"] == "json-label"

    def test_administrator_and_director_are_distinct_uuids(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data["administrator_agent_id"] != data["director"]["agent_id"]


class TestSessionCreateOutsideTmux:
    def test_fails_with_specific_error_and_exit_1(self, db_file, mock_tmux_unavailable):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 1, result.output
        combined = (result.output or "") + (result.stderr or "")
        assert "cafleet session create must be run inside a tmux session" in combined

    def test_no_session_row_is_written_when_tmux_is_missing(
        self, db_file, mock_tmux_unavailable
    ):
        runner = CliRunner()
        before = _session_rows(db_file)
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 1, result.output
        after = _session_rows(db_file)
        assert before == after


class TestSessionListHidesSoftDeleted:
    def test_deleted_session_is_hidden_from_text_list(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        r1 = runner.invoke(cli, ["session", "create", "--label", "keep", "--json"])
        r2 = runner.invoke(cli, ["session", "create", "--label", "drop", "--json"])
        assert r1.exit_code == 0
        assert r2.exit_code == 0
        keep_sid = json.loads(r1.output)["session_id"]
        drop_sid = json.loads(r2.output)["session_id"]

        del_result = runner.invoke(cli, ["session", "delete", drop_sid])
        assert del_result.exit_code == 0, del_result.output

        list_result = runner.invoke(cli, ["session", "list"])
        assert list_result.exit_code == 0
        assert keep_sid in list_result.output
        assert drop_sid not in list_result.output

    def test_deleted_session_is_hidden_from_json_list(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        r1 = runner.invoke(cli, ["session", "create", "--json"])
        r2 = runner.invoke(cli, ["session", "create", "--json"])
        keep_sid = json.loads(r1.output)["session_id"]
        drop_sid = json.loads(r2.output)["session_id"]
        runner.invoke(cli, ["session", "delete", drop_sid])

        list_result = runner.invoke(cli, ["session", "list", "--json"])
        assert list_result.exit_code == 0
        data = json.loads(list_result.output)
        ids = {s["session_id"] for s in data}
        assert keep_sid in ids
        assert drop_sid not in ids

    def test_list_has_no_all_flag(self, db_file):
        """Design 0000026 explicitly removes/omits an ``--all`` flag."""
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "list", "--all"])
        assert result.exit_code == 2, result.output


class TestSessionDeleteUnknownAndIdempotent:
    def test_unknown_session_id_exits_1_with_not_found(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        fake = str(uuid.uuid4())
        result = runner.invoke(cli, ["session", "delete", fake])
        assert result.exit_code == 1, result.output
        combined = ((result.output or "") + (result.stderr or "")).lower()
        assert "not found" in combined
        assert fake in (result.output or "") + (result.stderr or "")

    def test_second_delete_is_idempotent_and_reports_zero(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        r = runner.invoke(cli, ["session", "create", "--json"])
        assert r.exit_code == 0
        sid = json.loads(r.output)["session_id"]

        first = runner.invoke(cli, ["session", "delete", sid])
        second = runner.invoke(cli, ["session", "delete", sid])

        assert first.exit_code == 0, first.output
        assert second.exit_code == 0, second.output
        assert "0 agents" in second.output
