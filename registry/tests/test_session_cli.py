"""Tests for ``hikyaku-registry session`` CLI subcommands.

Design doc 0000015 Step 7 adds a ``session`` click group as a sibling of
the existing ``db`` group, with four commands:

  - ``session create [--label TEXT] [--json]``
  - ``session list [--json]``
  - ``session show <session_id> [--json]``
  - ``session delete <session_id>``

All commands use sync SQLAlchemy (``create_engine(_sync_db_url())``) to
talk directly to the SQLite file, the same pattern ``db init`` uses.
The broker server does NOT need to be running.

Test isolation strategy (mirrors ``test_db_init.py``):

  Each test seeds a fresh ``tmp_path`` DB via ``db init``, then exercises
  session subcommands against that file.  ``config.settings.database_url``
  is monkeypatched to the temp path so the CLI's ``_sync_db_url()`` resolves
  to the right file.
"""

import json
import sqlite3
import uuid

from click.testing import CliRunner

from hikyaku_registry import config


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _init_db(runner: CliRunner, main) -> None:
    """Run ``db init`` to set up the schema in the temp DB."""
    result = runner.invoke(main, ["db", "init"])
    assert result.exit_code == 0, (
        f"db init failed during test setup.\n"
        f"output: {result.output}\nexception: {result.exception}"
    )


def _seed_session(db_path, session_id: str, label: str | None = None) -> None:
    """Insert a session row directly for test setup (bypasses CLI)."""
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT INTO sessions (session_id, label, created_at) VALUES (?, ?, ?)",
            (session_id, label, "2026-01-01T00:00:00+00:00"),
        )
        conn.commit()
    finally:
        conn.close()


def _seed_agent(db_path, agent_id: str, session_id: str, *, status: str = "active") -> None:
    """Insert an agent row directly for test setup (bypasses CLI)."""
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
    """Read all session rows from the DB."""
    conn = sqlite3.connect(str(db_path))
    try:
        return conn.execute(
            "SELECT session_id, label, created_at FROM sessions"
        ).fetchall()
    finally:
        conn.close()


# ===========================================================================
# session create
# ===========================================================================


class TestSessionCreate:
    """``hikyaku-registry session create`` mints a UUID session."""

    def test_creates_session_with_uuid(self, tmp_path, monkeypatch):
        """Creates a session and prints a valid UUID to stdout."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        result = runner.invoke(main, ["session", "create"])

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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        result = runner.invoke(main, ["session", "create", "--label", "PR-42 review"])

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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        result = runner.invoke(main, ["session", "create"])
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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        result = runner.invoke(main, ["session", "create", "--label", "test", "--json"])

        assert result.exit_code == 0, (
            f"session create --json failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        data = json.loads(result.output)
        assert "session_id" in data, "JSON output should contain 'session_id'"
        # Validate it's a UUID
        uuid.UUID(data["session_id"])
        assert data.get("label") == "test", (
            f"JSON output label should be 'test', got {data.get('label')!r}"
        )

    def test_each_create_mints_unique_id(self, tmp_path, monkeypatch):
        """Each invocation mints a fresh UUID — no idempotency."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        r1 = runner.invoke(main, ["session", "create", "--json"])
        r2 = runner.invoke(main, ["session", "create", "--json"])

        assert r1.exit_code == 0
        assert r2.exit_code == 0

        id1 = json.loads(r1.output)["session_id"]
        id2 = json.loads(r2.output)["session_id"]
        assert id1 != id2, "each session create should mint a unique UUID"

        rows = _session_rows(db_file)
        assert len(rows) == 2, "two creates should produce two session rows"


# ===========================================================================
# session list
# ===========================================================================


class TestSessionList:
    """``hikyaku-registry session list`` shows all sessions."""

    def test_lists_empty(self, tmp_path, monkeypatch):
        """No sessions: table output with no data rows."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        result = runner.invoke(main, ["session", "list"])

        assert result.exit_code == 0, (
            f"session list failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )

    def test_lists_sessions_with_agent_count(self, tmp_path, monkeypatch):
        """Lists sessions with their active agent counts."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="test-session")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        # Deregistered agent should NOT be counted
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(main, ["session", "list"])

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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="json-test")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

        result = runner.invoke(main, ["session", "list", "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert isinstance(data, list), "JSON output should be a list"
        assert len(data) == 1
        assert data[0]["session_id"] == sid
        assert data[0]["label"] == "json-test"
        assert data[0]["agent_count"] == 1, (
            f"agent_count should be 1 (only active agents), "
            f"got {data[0].get('agent_count')}"
        )

    def test_lists_multiple_sessions(self, tmp_path, monkeypatch):
        """Multiple sessions are all listed."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        _seed_session(db_file, sid_a, label="session-a")
        _seed_session(db_file, sid_b, label="session-b")

        result = runner.invoke(main, ["session", "list", "--json"])

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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(main, ["session", "list", "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data[0]["agent_count"] == 1, (
            f"only active agents should be counted; "
            f"got {data[0].get('agent_count')}"
        )


# ===========================================================================
# session show
# ===========================================================================


class TestSessionShow:
    """``hikyaku-registry session show <id>`` displays a single session."""

    def test_shows_existing_session(self, tmp_path, monkeypatch):
        """Shows the session row when it exists."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="show-test")

        result = runner.invoke(main, ["session", "show", sid])

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
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid, label="json-show")

        result = runner.invoke(main, ["session", "show", sid, "--json"])

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data["session_id"] == sid
        assert data["label"] == "json-show"
        assert "created_at" in data

    def test_missing_session_exits_nonzero(self, tmp_path, monkeypatch):
        """Non-existent session_id exits with code 1 and error message."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        fake_id = str(uuid.uuid4())
        result = runner.invoke(main, ["session", "show", fake_id])

        assert result.exit_code != 0, (
            f"session show should exit non-zero for missing session. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        output_lower = result.output.lower()
        assert "not found" in output_lower or "error" in output_lower, (
            f"error message should mention 'not found' or 'error'. "
            f"got: {result.output!r}"
        )


# ===========================================================================
# session delete
# ===========================================================================


class TestSessionDelete:
    """``hikyaku-registry session delete <id>`` removes a session."""

    def test_deletes_session(self, tmp_path, monkeypatch):
        """Deletes an empty session and prints success message."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)

        result = runner.invoke(main, ["session", "delete", sid])

        assert result.exit_code == 0, (
            f"session delete failed.\noutput: {result.output}\n"
            f"exception: {result.exception}"
        )
        # Verify the session was removed
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert sid not in session_ids, (
            f"session {sid} should have been deleted from the DB"
        )
        # Output should mention "Deleted"
        assert "deleted" in result.output.lower(), (
            f"delete success output should mention 'Deleted'. "
            f"got: {result.output!r}"
        )

    def test_delete_nonexistent_session(self, tmp_path, monkeypatch):
        """Deleting a non-existent session exits non-zero or with error."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        fake_id = str(uuid.uuid4())
        result = runner.invoke(main, ["session", "delete", fake_id])

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

    def test_delete_session_with_active_agents_fails(self, tmp_path, monkeypatch):
        """Cannot delete a session that still has agents (FK RESTRICT).

        Design doc: catches IntegrityError, queries agent count,
        raises click.UsageError with "session <id> has N active agents".
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

        result = runner.invoke(main, ["session", "delete", sid])

        assert result.exit_code != 0, (
            f"session delete should fail when agents reference the session. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        # Verify the session still exists (delete was rolled back)
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert sid in session_ids, (
            "session should still exist after failed delete"
        )

    def test_delete_session_with_deregistered_agents_fails(self, tmp_path, monkeypatch):
        """Deregistered agents still reference the session via FK.

        Design doc edge case: "hikyaku-registry session delete on a
        session with deregistered (but not purged) agents rows hits
        the ondelete='RESTRICT' — friendly error needed."
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="deregistered")

        result = runner.invoke(main, ["session", "delete", sid])

        assert result.exit_code != 0, (
            f"session delete should fail even with deregistered agents "
            f"(FK RESTRICT still applies). "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        # Session must still be present
        rows = _session_rows(db_file)
        session_ids = [r[0] for r in rows]
        assert sid in session_ids

    def test_delete_friendly_error_message(self, tmp_path, monkeypatch):
        """FK violation produces a friendly error, not a raw IntegrityError."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        sid = str(uuid.uuid4())
        _seed_session(db_file, sid)
        _seed_agent(db_file, str(uuid.uuid4()), sid, status="active")

        result = runner.invoke(main, ["session", "delete", sid])

        assert result.exit_code != 0
        # The error message should be user-friendly, not a raw traceback
        assert "IntegrityError" not in result.output, (
            "error should be a friendly message, not a raw IntegrityError"
        )
        # Should mention agents or the session
        output_lower = result.output.lower()
        assert "agent" in output_lower or sid in result.output, (
            f"friendly error should mention 'agent' count or the session id. "
            f"got: {result.output!r}"
        )


# ===========================================================================
# db init does NOT auto-create a session
# ===========================================================================


class TestDbInitNoAutoSession:
    """``db init`` must NOT auto-create a default session.

    Design doc: "hikyaku-registry db init remains schema-only.
    It does NOT auto-create a default session."
    """

    def test_db_init_creates_no_sessions(self, tmp_path, monkeypatch):
        """After db init, the sessions table exists but is empty."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        _init_db(runner, main)

        rows = _session_rows(db_file)
        assert len(rows) == 0, (
            f"db init should not auto-create any session rows. "
            f"found: {rows}"
        )


# ===========================================================================
# session group exists as a sibling of db
# ===========================================================================


class TestSessionGroupStructure:
    """Verify the session group is a sibling of db, not a child."""

    def test_session_group_exists(self, tmp_path, monkeypatch):
        """``hikyaku-registry session`` is a recognized command group."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["session", "--help"])

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
        """``hikyaku-registry db session`` should NOT work."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings, "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        from hikyaku_registry.cli import main

        runner = CliRunner()
        result = runner.invoke(main, ["db", "session", "create"])

        # Should fail — "session" is not a subcommand of "db"
        assert result.exit_code != 0, (
            "session should be a sibling of db, not a child"
        )
