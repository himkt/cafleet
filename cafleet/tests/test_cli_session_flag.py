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
    assert init.exit_code == 0, init.output
    return runner


class TestMissingSessionIdFailsClientSubcommands:
    def test_register_without_session_id_exits_one(self, db_runner):
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        assert result.exit_code == 1, result.output

    def test_register_without_session_id_shows_new_error_message(self, db_runner):
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        out = result.output or ""
        assert "--session-id" in out
        assert "is required" in out
        assert "cafleet session create" in out
        assert "CAFLEET_SESSION_ID" not in out
        assert "environment variable" not in out.lower()

    def test_send_without_session_id_exits_one(self, db_runner):
        aid = str(uuid.uuid4())
        bid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            ["send", "--agent-id", aid, "--to", bid, "--text", "hi"],
        )
        assert result.exit_code == 1, result.output

    def test_poll_without_session_id_exits_one(self, db_runner):
        aid = str(uuid.uuid4())
        result = db_runner.invoke(cli, ["poll", "--agent-id", aid])
        assert result.exit_code == 1, result.output


class TestSessionIdFlagFlowsIntoBroker:
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

        assert result.exit_code == 0, result.output
        all_values = list(captured["args"]) + list(captured["kwargs"].values())
        assert sid in all_values

    def test_send_passes_session_id_to_broker(self, db_runner, monkeypatch):
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

        assert result.exit_code == 0, result.output
        all_values = list(captured["args"]) + list(captured["kwargs"].values())
        assert sid in all_values

    def test_session_id_not_read_from_environment(self, db_runner, monkeypatch):
        """The env var CAFLEET_SESSION_ID was removed; only the CLI flag works."""
        monkeypatch.setenv("CAFLEET_SESSION_ID", str(uuid.uuid4()))
        result = db_runner.invoke(
            cli,
            ["register", "--name", "A", "--description", "a"],
        )
        assert result.exit_code == 1, result.output


class TestSubcommandsThatDoNotRequireSessionId:
    def test_db_init_without_session_id(self, tmp_path, monkeypatch):
        db_file = tmp_path / "registry.db"
        monkeypatch.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )
        runner = CliRunner()
        result = runner.invoke(cli, ["db", "init"])
        assert result.exit_code == 0, result.output

    def test_session_create_without_session_id(self, db_runner):
        """session create mints a session, so it cannot itself require one."""
        result = db_runner.invoke(cli, ["session", "create", "--label", "smoke"])
        assert result.exit_code == 0, result.output

    def test_session_list_without_session_id(self, db_runner):
        result = db_runner.invoke(cli, ["session", "list"])
        assert result.exit_code == 0, result.output


class TestCafleetEnvSubcommandRemoved:
    def test_env_subcommand_is_gone(self, db_runner):
        result = db_runner.invoke(cli, ["env"])
        assert result.exit_code == 2, result.output

    def test_env_subcommand_reports_no_such_command(self, db_runner):
        result = db_runner.invoke(cli, ["env"])
        out = result.output or ""
        assert "no such command" in out.lower()

    def test_help_no_longer_lists_env(self, db_runner):
        result = db_runner.invoke(cli, ["--help"])
        assert result.exit_code == 0
        for line in result.output.splitlines():
            stripped = line.strip()
            assert not stripped.startswith("env ")
            assert stripped != "env"


class TestSessionIdSilentlyAcceptedWhereNotRequired:
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
        assert result.exit_code == 0, result.output
        combined = (result.output or "").lower()
        assert "unused" not in combined
        assert "unexpected" not in combined

    def test_session_create_accepts_session_id_silently(self, db_runner):
        sid = str(uuid.uuid4())
        result = db_runner.invoke(
            cli,
            ["--session-id", sid, "session", "create", "--label", "x"],
        )
        assert result.exit_code == 0, result.output


def _create_session_via_cli(runner: CliRunner) -> tuple[str, str]:
    """Run ``session create --json`` and return (session_id, administrator_agent_id)."""
    result = runner.invoke(cli, ["session", "create", "--json"])
    assert result.exit_code == 0, result.output
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
        assert result.exit_code == 1, result.output

    def test_cli_deregister_admin_message_is_user_friendly(self, tmp_path, monkeypatch):
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
        assert "Administrator cannot be deregistered" in out
        assert "Traceback" not in out

    def test_cli_deregister_unknown_agent_exits_nonzero(self, tmp_path, monkeypatch):
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
        assert result.exit_code == 1, result.output
        assert "not found or already deregistered" in (result.output or "")

    def test_cli_deregister_admin_leaves_row_active(self, tmp_path, monkeypatch):
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
        assert status == "active"
        assert deregistered_at is None
