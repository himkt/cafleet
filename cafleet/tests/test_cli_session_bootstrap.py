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
    assert init.exit_code == 0, (
        f"db init failed during test setup.\noutput: {init.output}\n"
        f"exception: {init.exception}"
    )
    return path


@pytest.fixture
def mock_tmux_ok(monkeypatch):
    """Stub tmux availability and hand back a fixed ``DirectorContext``."""
    from cafleet import cli as cli_mod

    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: _FAKE_DIRECTOR_CTX)
    # Cover callers that do ``from cafleet.tmux import …`` as well.
    if hasattr(cli_mod, "ensure_tmux_available"):
        monkeypatch.setattr(cli_mod, "ensure_tmux_available", lambda: None)
    if hasattr(cli_mod, "director_context"):
        monkeypatch.setattr(cli_mod, "director_context", lambda: _FAKE_DIRECTOR_CTX)


@pytest.fixture
def mock_tmux_unavailable(monkeypatch):
    """Make ``tmux.ensure_tmux_available`` raise the canonical TmuxError."""
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
    """``cafleet session create`` text output has the 6-line shape in §3."""

    def test_line_1_is_session_id_and_line_2_is_director_agent_id(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 0, (
            f"session create failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )

        lines = [ln for ln in result.output.splitlines() if ln.strip() != ""]
        assert len(lines) >= 2, (
            f"session create text output must have at least 2 lines "
            f"(session_id + director agent_id), got {lines!r}"
        )

        # Line 1: session_id (a UUID)
        sid_line = lines[0].strip()
        uuid.UUID(sid_line)  # raises if not a UUID

        # Line 2: director agent_id (a UUID, different from session_id)
        director_line = lines[1].strip()
        uuid.UUID(director_line)
        assert director_line != sid_line, (
            "line 2 must be the Director's agent_id, not a repeat of session_id"
        )

        # The session row matches line 1.
        rows = _session_rows(db_file)
        assert any(r[0] == sid_line for r in rows), (
            f"line 1 must match the created sessions row; "
            f"got line1={sid_line!r}, rows={[r[0] for r in rows]!r}"
        )

    def test_contains_label_director_name_pane_and_administrator_labels(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--label", "bootstrap-check"])
        assert result.exit_code == 0

        text = result.output
        # Design-specified labels (presence, not exact column widths).
        assert "label:" in text
        assert "director_name:" in text
        assert "pane:" in text
        assert "administrator:" in text

        # director_name value is hardcoded to "director"
        assert "director" in text

        # label is echoed
        assert "bootstrap-check" in text

        # pane value includes the fake tmux context components joined by ':'
        assert (
            f"{_FAKE_DIRECTOR_CTX.session}:{_FAKE_DIRECTOR_CTX.window_id}:"
            f"{_FAKE_DIRECTOR_CTX.pane_id}" in text
        ), (
            f"pane line must contain '<session>:<window_id>:<pane_id>' "
            f"matching the director_context, full output:\n{text!r}"
        )

    def test_administrator_line_references_the_seeded_administrator(
        self, db_file, mock_tmux_ok
    ):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 0

        # Verify the UUID that appears on the administrator: line is also in the
        # agents table as the built-in Administrator.
        admin_line = next(
            (
                ln.strip()
                for ln in result.output.splitlines()
                if ln.strip().lower().startswith("administrator")
            ),
            None,
        )
        assert admin_line is not None, (
            f"missing 'administrator:' line in output:\n{result.output!r}"
        )
        # Extract the UUID from that line.
        admin_id = None
        for token in admin_line.replace(":", " ").split():
            try:
                uuid.UUID(token)
                admin_id = token
                break
            except ValueError:
                continue
        assert admin_id is not None, (
            f"administrator: line must contain a UUID, got {admin_line!r}"
        )

        # Cross-check: a row in agents with this UUID must exist and be the
        # built-in Administrator (name='Administrator').
        rows = _agent_rows(db_file)
        matches = [r for r in rows if r[0] == admin_id]
        assert len(matches) == 1, (
            f"administrator: UUID must map to an agents row; got {matches!r}"
        )
        assert matches[0][2] == "Administrator", (
            f"administrator row should have name='Administrator', got {matches[0][2]!r}"
        )


class TestSessionCreateJsonOutput:
    """--json emits the nested {session, director{placement{...}}} shape."""

    def test_top_level_keys(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0, (
            f"session create --json failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )

        data = json.loads(result.output)
        for key in (
            "session_id",
            "label",
            "created_at",
            "administrator_agent_id",
            "director",
        ):
            assert key in data, (
                f"JSON output must contain top-level key {key!r}. got keys: "
                f"{sorted(data.keys())}"
            )

        # administrator_agent_id is a UUID.
        uuid.UUID(data["administrator_agent_id"])

    def test_director_sub_dict(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        director = data["director"]
        for key in ("agent_id", "name", "description", "registered_at", "placement"):
            assert key in director, (
                f"director sub-dict must contain {key!r}; got {sorted(director)}"
            )
        assert director["name"] == "director"
        assert director["description"] == "Root Director for this session"
        uuid.UUID(director["agent_id"])

    def test_placement_sub_dict_matches_spec(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create", "--json"])
        assert result.exit_code == 0
        data = json.loads(result.output)
        placement = data["director"]["placement"]

        assert placement["director_agent_id"] is None, (
            "root Director's placement.director_agent_id must be null in JSON"
        )
        assert placement["coding_agent"] == "unknown", (
            "root Director's placement.coding_agent must be 'unknown' "
            "(overrides the 'claude' server_default)"
        )
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
    """Without a usable tmux, session create aborts before touching the DB."""

    def test_fails_with_specific_error_and_exit_1(self, db_file, mock_tmux_unavailable):
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 1, (
            f"session create outside tmux must exit 1; got {result.exit_code}.\n"
            f"output: {result.output!r}\nexception: {result.exception}"
        )
        combined = (result.output or "") + (result.stderr or "")
        assert "cafleet session create must be run inside a tmux session" in combined, (
            f"expected specific tmux error, got: {combined!r}"
        )

    def test_no_session_row_is_written_when_tmux_is_missing(
        self, db_file, mock_tmux_unavailable
    ):
        runner = CliRunner()
        before = _session_rows(db_file)
        result = runner.invoke(cli, ["session", "create"])
        assert result.exit_code == 1, (
            f"session create outside tmux must exit 1 per design 0000026 "
            f"(Error Handling table); got exit_code={result.exit_code}, "
            f"output={result.output!r}"
        )
        after = _session_rows(db_file)
        assert before == after, (
            f"session create must not write any rows when tmux is unavailable; "
            f"before={before!r}, after={after!r}"
        )


class TestSessionListHidesSoftDeleted:
    """Soft-deleted sessions are excluded from ``session list``."""

    def test_deleted_session_is_hidden_from_text_list(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        r1 = runner.invoke(cli, ["session", "create", "--label", "keep", "--json"])
        r2 = runner.invoke(cli, ["session", "create", "--label", "drop", "--json"])
        assert r1.exit_code == 0
        assert r2.exit_code == 0
        keep_sid = json.loads(r1.output)["session_id"]
        drop_sid = json.loads(r2.output)["session_id"]

        del_result = runner.invoke(cli, ["session", "delete", drop_sid])
        assert del_result.exit_code == 0, (
            f"session delete failed: {del_result.output!r} {del_result.exception!r}"
        )

        list_result = runner.invoke(cli, ["session", "list"])
        assert list_result.exit_code == 0
        assert keep_sid in list_result.output, (
            f"active session must appear in list output; got:\n{list_result.output!r}"
        )
        assert drop_sid not in list_result.output, (
            f"soft-deleted session must be hidden from list output; got:\n"
            f"{list_result.output!r}"
        )

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
        # Click rejects unknown options with exit code 2 ("No such option").
        assert result.exit_code == 2, (
            "session list must reject --all via click's unknown-option handler "
            "(exit 2) in this revision; "
            f"got exit_code={result.exit_code}, output={result.output!r}"
        )


class TestSessionDeleteUnknownAndIdempotent:
    def test_unknown_session_id_exits_1_with_not_found(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        fake = str(uuid.uuid4())
        result = runner.invoke(cli, ["session", "delete", fake])
        assert result.exit_code == 1, (
            f"session delete on unknown id must exit 1 (design 0000026 pins "
            f"this to match session show); got exit_code={result.exit_code}, "
            f"output={result.output!r}"
        )
        combined = ((result.output or "") + (result.stderr or "")).lower()
        assert "not found" in combined, (
            f"error message must mention 'not found'; got {combined!r}"
        )
        assert fake in (result.output or "") + (result.stderr or ""), (
            "error message must include the unknown session_id"
        )

    def test_second_delete_is_idempotent_and_reports_zero(self, db_file, mock_tmux_ok):
        runner = CliRunner()
        r = runner.invoke(cli, ["session", "create", "--json"])
        assert r.exit_code == 0
        sid = json.loads(r.output)["session_id"]

        first = runner.invoke(cli, ["session", "delete", sid])
        second = runner.invoke(cli, ["session", "delete", sid])

        assert first.exit_code == 0, (
            f"first delete must succeed; got exit_code={first.exit_code}, "
            f"output={first.output!r}"
        )
        assert second.exit_code == 0, (
            f"second delete must be idempotent (exit 0); got exit_code="
            f"{second.exit_code}, output={second.output!r}"
        )
        # The second delete specifically reports 0 agents deregistered.
        assert "0 agents" in second.output, (
            f"second delete output must say 'Deregistered 0 agents.'; "
            f"got {second.output!r}"
        )
