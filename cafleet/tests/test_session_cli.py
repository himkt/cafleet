"""Tests for the ``cafleet session`` CLI subcommands."""

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
    assert result.exit_code == 0, (
        f"db init failed during test setup.\n"
        f"output: {result.output}\nexception: {result.exception}"
    )


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
    """``cafleet-registry session create`` mints a UUID session."""

    def test_creates_session_with_uuid(self, tmp_path, monkeypatch):
        """Creates a session and prints a valid UUID to stdout."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create"])

        assert result.exit_code == 0, (
            f"session create failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        # The output should contain a valid UUID
        output = result.output.strip()
        # Extract UUID from output (may have surrounding text)
        found_uuid = None
        for word in output.split():
            try:
                uuid.UUID(word)
                found_uuid = word
                break
            except ValueError:
                continue
        assert found_uuid is not None, (
            f"session create should print a valid UUID. got: {output!r}"
        )

        # Verify the session was actually inserted into the DB
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert found_uuid in session_ids, (
            f"session create should insert a row into the sessions table. "
            f"expected {found_uuid} in {session_ids}"
        )

    def test_creates_session_with_label(self, tmp_path, monkeypatch):
        """``--label`` stores the label in the sessions row."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--label", "PR-42 review"])

        assert result.exit_code == 0, (
            f"session create --label failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )

        rows = _session_rows(db_file)
        assert len(rows) == 1, f"expected 1 session row, got {len(rows)}"
        assert rows[0][1] == "PR-42 review", (
            f"label should be 'PR-42 review', got {rows[0][1]!r}"
        )

    def test_creates_session_without_label(self, tmp_path, monkeypatch):
        """Without ``--label``, label is NULL."""
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
        assert rows[0][1] is None, (
            f"label should be None when --label is not provided, got {rows[0][1]!r}"
        )

    def test_creates_session_json_output(self, tmp_path, monkeypatch):
        """``--json`` flag produces machine-parseable JSON output."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--label", "test", "--json"])

        assert result.exit_code == 0, (
            f"session create --json failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        data = json.loads(result.output)
        assert "session_id" in data, "JSON output should contain 'session_id'"
        # Validate it's a UUID
        uuid.UUID(data["session_id"])
        assert data["label"] == "test"

    def test_each_create_mints_unique_id(self, tmp_path, monkeypatch):
        """Each invocation mints a fresh UUID — no idempotency."""
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
        assert id1 != id2, "each session create should mint a unique UUID"

        rows = _session_rows(db_file)
        assert len(rows) == 2, "two creates should produce two session rows"

    def test_json_output_includes_administrator_agent_id(self, tmp_path, monkeypatch):
        """--json output now contains ``administrator_agent_id`` (design 0000025 §B)."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "create", "--json"])

        assert result.exit_code == 0, (
            f"session create --json failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        data = json.loads(result.output)
        assert "administrator_agent_id" in data, (
            "JSON output should contain 'administrator_agent_id'"
        )
        # Validate it's a UUID-shaped string.
        uuid.UUID(data["administrator_agent_id"])

    def test_json_administrator_agent_id_matches_db_row(self, tmp_path, monkeypatch):
        """The returned administrator_agent_id must correspond to a real agents row
        in the same session marked as an administrator.
        """
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

        assert len(rows) == 1, (
            f"expected one agents row matching administrator_agent_id {admin_id}"
        )
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
    """``cafleet session list`` shows all sessions."""

    def test_lists_empty(self, tmp_path, monkeypatch):
        """No sessions: table output with no data rows."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        _init_db(runner)

        result = runner.invoke(cli, ["session", "list"])

        assert result.exit_code == 0, (
            f"session list failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )

    def test_lists_sessions_with_agent_count(self, tmp_path, monkeypatch):
        """Lists sessions with their active agent counts."""
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
        # Deregistered agent should NOT be counted
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(cli, ["session", "list"])

        assert result.exit_code == 0
        # The output should contain the session_id and the label
        assert sid in result.output, (
            f"session list output should contain session_id {sid}"
        )
        assert "test-session" in result.output, (
            "session list output should contain the label"
        )

    def test_lists_json_output(self, tmp_path, monkeypatch):
        """``--json`` flag produces machine-parseable JSON array."""
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
        assert isinstance(data, list), "JSON output should be a list"
        assert len(data) == 1
        assert data[0]["session_id"] == sid
        assert data[0]["label"] == "json-test"
        assert data[0]["agent_count"] == 1

    def test_lists_multiple_sessions(self, tmp_path, monkeypatch):
        """Multiple sessions are all listed."""
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
        """Agent count in list uses LEFT JOIN WHERE status='active'."""
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
    """``cafleet session show <id>`` displays a single session."""

    def test_shows_existing_session(self, tmp_path, monkeypatch):
        """Shows the session row when it exists."""
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

        assert result.exit_code == 0, (
            f"session show failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        assert sid in result.output, (
            f"session show output should contain the session_id {sid}"
        )
        assert "show-test" in result.output, (
            "session show output should contain the label"
        )

    def test_shows_json_output(self, tmp_path, monkeypatch):
        """``--json`` flag produces machine-parseable JSON object."""
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
        """Non-existent session_id exits with code 1 and error message."""
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

        assert result.exit_code == 1, (
            f"session show must exit 1 for missing session (ctx.exit(1) in "
            f"cli.session_show). got exit_code={result.exit_code}, "
            f"output: {result.output}"
        )
        output_lower = result.output.lower()
        assert "not found" in output_lower, (
            f"error message must mention 'not found'. got: {result.output!r}"
        )

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

        assert result.exit_code == 0, (
            f"session show on a soft-deleted row must still succeed "
            f"(audit semantics). got exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        assert "deleted_at:" in result.output, (
            f"text output must include a 'deleted_at:' line when the session "
            f"row has a non-NULL deleted_at. got: {result.output!r}"
        )
        assert "2026-04-16T10:00:00+00:00" in result.output, (
            f"text output must show the actual deleted_at timestamp. "
            f"got: {result.output!r}"
        )

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

        assert result.exit_code == 0, (
            f"session show on an active row must succeed. "
            f"got exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert "deleted_at" not in result.output, (
            f"active session text output must NOT include a 'deleted_at' "
            f"line (that line is reserved for soft-deleted rows). "
            f"got: {result.output!r}"
        )


class TestSessionDelete:
    """``cafleet session delete <id>`` removes a session."""

    def test_deletes_session(self, tmp_path, monkeypatch):
        """Deletes an empty session and prints success message."""
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

        assert result.exit_code == 0, (
            f"session delete failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        # Design 0000026: delete_session is a SOFT delete. The row stays but
        # its ``deleted_at`` is set, and ``list_sessions`` hides it.
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert sid in session_ids, (
            f"session {sid} should remain in the DB after soft delete, "
            f"got session_ids={session_ids!r}"
        )
        assert _session_deleted_at(db_file, sid) is not None, (
            f"session {sid}.deleted_at should be set after soft delete"
        )
        # Output should mention "Deleted"
        assert "deleted" in result.output.lower(), (
            f"delete success output should mention 'Deleted'. got: {result.output!r}"
        )

    def test_delete_nonexistent_session(self, tmp_path, monkeypatch):
        """Deleting a non-existent session exits non-zero or with error."""
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

        # Either exit non-zero or print an error — design doc says
        # "Deleted session <id>" on success, implying no output on
        # non-existent. Either way it should not crash.
        # The implementation may silently succeed (DELETE WHERE ...
        # affects 0 rows) or error. We just verify it doesn't crash
        # and ideally signals the missing session.
        assert result.exception is None or isinstance(result.exception, SystemExit), (
            f"session delete should not raise unexpected exceptions. "
            f"exception: {result.exception}"
        )

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

        assert result.exit_code == 0, (
            f"session delete should succeed for a task-free session. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        assert _session_deleted_at(db_file, sid) is not None, (
            "session.deleted_at should be set after soft delete"
        )

    def test_delete_session_with_deregistered_agents_succeeds(
        self, tmp_path, monkeypatch
    ):
        """Deregistered agents are a no-op for the cascade; soft delete still works."""
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

        assert result.exit_code == 0, (
            f"session delete should succeed even with deregistered agents. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        assert _session_deleted_at(db_file, sid) is not None, (
            "session.deleted_at should be set after soft delete"
        )


class TestDbInitNoAutoSession:
    """``db init`` must NOT auto-create a default session."""

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
        assert len(rows) == 0, (
            f"db init should not auto-create any session rows. found: {rows}"
        )


class TestSessionGroupStructure:
    """Verify the session group is a sibling of db, not a child."""

    def test_session_group_exists(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["session", "--help"])

        assert result.exit_code == 0, (
            f"session group should exist.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        # Help text should show subcommands
        output_lower = result.output.lower()
        assert "create" in output_lower, "session --help should list 'create'"
        assert "list" in output_lower, "session --help should list 'list'"
        assert "show" in output_lower, "session --help should list 'show'"
        assert "delete" in output_lower, "session --help should list 'delete'"

    def test_session_is_not_under_db(self, tmp_path, monkeypatch):
        """``cafleet-registry db session`` should NOT work."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["db", "session", "create"])

        # Should fail with exit 2 — "session" is not a subcommand of "db",
        # so Click emits a UsageError (exit 2), not a runtime error (exit 1).
        assert result.exit_code == 2, (
            f"`db session` must be rejected by Click with exit 2 (unknown "
            f"subcommand is a UsageError). exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
