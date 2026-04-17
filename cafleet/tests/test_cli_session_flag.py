"""Tests for the ``cafleet --session-id <uuid>`` global CLI flag (design 0000023)."""

import json
import sqlite3
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, config
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
    """Let CliRunner-driven ``session create`` succeed without a real tmux pane."""
    ctx = DirectorContext(session="main", window_id="@3", pane_id="%0")
    monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", lambda: None)
    monkeypatch.setattr("cafleet.tmux.director_context", lambda: ctx)


@pytest.fixture
def db_runner(tmp_path, monkeypatch):
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )
    runner = CliRunner()
    init = runner.invoke(cli, ["db", "init"])
    assert init.exit_code == 0, (
        f"db init failed during test setup.\n"
        f"output: {init.output}\nexception: {init.exception}"
    )
    return runner


class TestMissingSessionIdFailsClientSubcommands:
    """Design doc Step 6(a): client subcommands without --session-id exit 1."""

    def test_register_without_session_id_exits_one(self, db_runner):
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        assert result.exit_code == 1, (
            f"register without --session-id should exit 1. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )

    def test_register_without_session_id_shows_new_error_message(self, db_runner):
        """Error message must match the new wording exactly (design doc Validation section)."""
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        # CliRunner defaults to mix_stderr=True, so both streams land in .output.
        out = result.output or ""
        assert "--session-id" in out, (
            f"error must mention the new --session-id flag. got: {out!r}"
        )
        assert "is required" in out, (
            f"error must state the flag is required. got: {out!r}"
        )
        assert "cafleet session create" in out, (
            f"error must suggest running 'cafleet session create'. got: {out!r}"
        )
        # The old env-var message must be gone.
        assert "CAFLEET_SESSION_ID" not in out, (
            f"error must not reference the removed env var. got: {out!r}"
        )
        assert "environment variable" not in out.lower(), (
            f"error must not mention env vars at all. got: {out!r}"
        )

    def test_send_without_session_id_exits_one(self, db_runner):
        """send is also a client subcommand that requires --session-id."""
        aid = str(uuid.uuid4())
        bid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            ["send", "--agent-id", aid, "--to", bid, "--text", "hi"],
        )
        assert result.exit_code == 1, (
            f"send without --session-id should exit 1. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )

    def test_poll_without_session_id_exits_one(self, db_runner):
        aid = str(uuid.uuid4())
        result = db_runner.invoke(cli, ["poll", "--agent-id", aid])
        assert result.exit_code == 1, (
            f"poll without --session-id should exit 1. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )


class TestSessionIdFlagFlowsIntoBroker:
    """Design doc Step 6(b): the flag value reaches broker.register_agent."""

    def test_register_passes_session_id_to_broker(self, db_runner, monkeypatch):
        captured: dict = {}

        def fake_register_agent(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs
            return {
                "agent_id": "cccccccc-cccc-cccc-cccc-cccccccccccc",
                "name": "A",
                "registered_at": "2026-01-01T00:00:00+00:00",
            }

        monkeypatch.setattr(broker, "register_agent", fake_register_agent)

        sid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            ["--session-id", sid, "register", "--name", "A", "--description", "a"],
        )

        assert result.exit_code == 0, (
            f"register with --session-id should succeed when broker is mocked. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        all_values = list(captured["args"]) + list(captured["kwargs"].values())
        assert sid in all_values

    def test_send_passes_session_id_to_broker(self, db_runner, monkeypatch):
        """send --text also threads session_id through to broker.send_message."""
        captured: dict = {}

        def fake_send_message(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs
            sender = args[1] if len(args) > 1 else kwargs.get("agent_id")
            recipient = args[2] if len(args) > 2 else kwargs.get("to")
            return {
                "task": {
                    "id": "tttttttt-tttt-tttt-tttt-tttttttttttt",
                    "contextId": recipient,
                    "status": {
                        "state": "input_required",
                        "timestamp": "2026-01-01T00:00:00+00:00",
                    },
                    "artifacts": [],
                    "metadata": {
                        "fromAgentId": sender,
                        "toAgentId": recipient,
                        "type": "unicast",
                    },
                }
            }

        monkeypatch.setattr(broker, "send_message", fake_send_message)

        sid = str(uuid.uuid4())
        aid = str(uuid.uuid4())
        bid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            [
                "--session-id",
                sid,
                "send",
                "--agent-id",
                aid,
                "--to",
                bid,
                "--text",
                "hi",
            ],
        )

        assert result.exit_code == 0, (
            f"send with --session-id should succeed when broker is mocked. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        all_values = list(captured["args"]) + list(captured["kwargs"].values())
        assert sid in all_values

    def test_session_id_not_read_from_environment(self, db_runner, monkeypatch):
        """Setting CAFLEET_SESSION_ID env var alone must no longer satisfy the gate.

        Design doc Success Criteria: ``CAFLEET_SESSION_ID`` env var is removed
        from the codebase. A bare invocation with only the env var set must
        still exit 1, proving the CLI no longer reads from os.environ.
        """
        monkeypatch.setenv("CAFLEET_SESSION_ID", str(uuid.uuid4()))
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        assert result.exit_code == 1, (
            f"env var alone must not satisfy the session-id requirement. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )


class TestSubcommandsThatDoNotRequireSessionId:
    """Design doc Step 6(c): ``db init`` and ``session create`` never gate."""

    def test_db_init_without_session_id(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["db", "init"])
        assert result.exit_code == 0, (
            f"db init without --session-id must succeed. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )

    def test_session_create_without_session_id(self, db_runner):
        """session create is the very command that mints a session; it cannot
        itself require one as input.
        """
        result = db_runner.invoke(cli, ["session", "create", "--label", "smoke"])
        assert result.exit_code == 0, (
            f"session create without --session-id must succeed. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )

    def test_session_list_without_session_id(self, db_runner):
        """session list is also a management command and does not need --session-id."""
        result = db_runner.invoke(cli, ["session", "list"])
        assert result.exit_code == 0, (
            f"session list without --session-id must succeed. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )


class TestCafleetEnvSubcommandRemoved:
    """Design doc Step 6(d): ``cafleet env`` is deleted as part of this cycle."""

    def test_env_subcommand_is_gone(self, db_runner):
        result = db_runner.invoke(cli, ["env"])
        assert result.exit_code == 2, (
            f"'cafleet env' must be rejected by Click with exit 2 (unknown "
            f"command is a UsageError). exit_code={result.exit_code}, "
            f"output: {result.output}"
        )

    def test_env_subcommand_reports_no_such_command(self, db_runner):
        result = db_runner.invoke(cli, ["env"])
        out = result.output or ""
        assert "no such command" in out.lower(), (
            f"error must indicate 'No such command'. got: {out!r}"
        )

    def test_help_no_longer_lists_env(self, db_runner):
        result = db_runner.invoke(cli, ["--help"])
        assert result.exit_code == 0
        # Help text lists subcommands; 'env' must not appear as a row.
        # Use a loose heuristic: no line starting with 'env ' in the commands list.
        for line in result.output.splitlines():
            stripped = line.strip()
            assert not stripped.startswith("env "), (
                f"help output still lists 'env' as a subcommand: {line!r}"
            )
            assert stripped != "env", (
                f"help output still lists 'env' as a subcommand: {line!r}"
            )


class TestSessionIdSilentlyAcceptedWhereNotRequired:
    """Design doc Spec "Provided but not required": no rejection."""

    def test_db_init_accepts_session_id_silently(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        sid = str(uuid.uuid4())
        result = runner.invoke(cli, ["--session-id", sid, "db", "init"])
        assert result.exit_code == 0, (
            f"db init with --session-id must be silently accepted. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        # Must not complain about the flag being unused.
        combined = (result.output or "").lower()
        assert "unused" not in combined, (
            f"must not warn about unused option. got: {result.output!r}"
        )
        assert "unexpected" not in combined, (
            f"must not warn about unexpected option. got: {result.output!r}"
        )

    def test_session_create_accepts_session_id_silently(self, db_runner):
        sid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            ["--session-id", sid, "session", "create", "--label", "x"],
        )
        assert result.exit_code == 0, (
            f"session create with --session-id must be silently accepted. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )


def _create_session_via_cli(runner: CliRunner) -> tuple[str, str]:
    """Run ``session create --json`` and return (session_id, administrator_agent_id)."""
    result = runner.invoke(cli, ["session", "create", "--json"])
    assert result.exit_code == 0, (
        f"session create --json failed. output: {result.output}, "
        f"exception: {result.exception}"
    )
    data = json.loads(result.output)
    return data["session_id"], data["administrator_agent_id"]


def _fetch_agent_status(db_file, agent_id: str) -> tuple[str, str | None]:
    """Return (status, deregistered_at) for a given agent_id via raw SQLite."""
    conn = sqlite3.connect(str(db_file))
    try:
        row = conn.execute(
            "SELECT status, deregistered_at FROM agents WHERE agent_id = ?",
            (agent_id,),
        ).fetchone()
    finally:
        conn.close()
    assert row is not None, f"agent {agent_id} not found"
    return row[0], row[1]


class TestDeregisterAdministratorCliGuard:
    """``cafleet deregister --agent-id <admin_id>`` must fail with a clear error.

    The broker raises ``AdministratorProtectedError``; the CLI catches it,
    prints ``Error: ...`` to stderr, and exits non-zero via ``ctx.exit(1)``.
    Under CliRunner's default ``mix_stderr=True``, that stderr output is
    folded into ``result.output``.
    """

    def test_cli_deregister_admin_exits_nonzero(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        init = runner.invoke(cli, ["db", "init"])
        assert init.exit_code == 0

        session_id, admin_id = _create_session_via_cli(runner)

        result = runner.invoke(
            cli,
            ["--session-id", session_id, "deregister", "--agent-id", admin_id],
        )
        assert result.exit_code == 1, (
            f"cafleet deregister on an Administrator must exit 1 "
            f"(AdministratorProtectedError → ctx.exit(1), per docs/spec/"
            f"cli-options.md error table). exit_code={result.exit_code}, "
            f"output: {result.output}"
        )

    def test_cli_deregister_admin_message_is_user_friendly(self, tmp_path, monkeypatch):
        """The error output must mention the guard text, not a raw traceback."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        init = runner.invoke(cli, ["db", "init"])
        assert init.exit_code == 0

        session_id, admin_id = _create_session_via_cli(runner)

        result = runner.invoke(
            cli,
            ["--session-id", session_id, "deregister", "--agent-id", admin_id],
        )
        out = result.output or ""
        assert "Administrator cannot be deregistered" in out, (
            f"error output must mention 'Administrator cannot be deregistered'. "
            f"got: {out!r}"
        )
        assert "Traceback" not in out, (
            f"error output must be a friendly message, not a raw traceback. "
            f"got: {out!r}"
        )

    def test_cli_deregister_unknown_agent_exits_nonzero(self, tmp_path, monkeypatch):
        """``broker.deregister_agent`` returns ``False`` when the agent
        does not exist or is already deregistered. The CLI must surface
        that as a non-zero exit so callers do not see a misleading
        "Agent deregistered successfully" line.
        """
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        init = runner.invoke(cli, ["db", "init"])
        assert init.exit_code == 0

        session_id, _admin_id = _create_session_via_cli(runner)
        bogus_agent_id = str(uuid.uuid4())

        result = runner.invoke(
            cli,
            ["--session-id", session_id, "deregister", "--agent-id", bogus_agent_id],
        )
        assert result.exit_code == 1, (
            f"deregister of an unknown agent must exit 1 (the CLI falls "
            f"through to 'not deregistered' → ctx.exit(1), not a Click "
            f"usage error). exit_code={result.exit_code}, "
            f"output: {result.output}"
        )
        assert "not found or already deregistered" in (result.output or ""), (
            f"error output must mention the missing/deregistered state. "
            f"got: {result.output!r}"
        )

    def test_cli_deregister_admin_leaves_row_active(self, tmp_path, monkeypatch):
        """The administrators row must still be active after the failed CLI call."""
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        init = runner.invoke(cli, ["db", "init"])
        assert init.exit_code == 0

        session_id, admin_id = _create_session_via_cli(runner)

        runner.invoke(
            cli,
            ["--session-id", session_id, "deregister", "--agent-id", admin_id],
        )
        status, deregistered_at = _fetch_agent_status(db_file, admin_id)
        assert status == "active", (
            f"Administrator row must still be active after failed CLI deregister, "
            f"got status={status!r}"
        )
        assert deregistered_at is None, (
            f"Administrator must not have a deregistered_at timestamp after "
            f"failed CLI deregister, got {deregistered_at!r}"
        )
