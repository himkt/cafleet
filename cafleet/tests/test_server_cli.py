"""Tests for the ``cafleet server`` CLI subcommand (design doc 0000028)."""

import uuid

import uvicorn
from click.testing import CliRunner

from cafleet import server as server_mod
from cafleet.cli import cli
from cafleet.config import Settings, settings


class TestServerCommandHelp:
    def test_server_help_exits_zero(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert result.exit_code == 0, result.output

    def test_server_help_mentions_host_flag(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert "--host" in result.output

    def test_server_help_mentions_port_flag(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert "--port" in result.output


class TestServerCommandFlagParsing:
    def test_default_flags_pass_settings_defaults_to_uvicorn(self, monkeypatch):
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])

        assert result.exit_code == 0, result.output
        assert captured
        kwargs = captured["kwargs"]
        assert kwargs["host"] == settings.broker_host
        assert kwargs["port"] == settings.broker_port

    def test_explicit_flags_override_defaults(self, monkeypatch):
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(
            cli,
            ["server", "--host", "0.0.0.0", "--port", "9000"],
        )

        assert result.exit_code == 0, result.output
        kwargs = captured["kwargs"]
        assert kwargs["host"] == "0.0.0.0"
        assert kwargs["port"] == 9000

    def test_app_import_string_passed_as_first_positional(self, monkeypatch):
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])
        assert result.exit_code == 0, result.output
        args = captured["args"]
        assert args
        assert args[0] == "cafleet.server:app"

    def test_port_string_rejected_by_click_type_int(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--port", "not-a-port"])
        assert result.exit_code == 2, result.output


class TestServerDoesNotRequireSessionId:
    """Uses ``--help`` so ``uvicorn.run`` is never called and tests do not
    need to monkey-patch it.
    """

    def test_server_help_without_session_id_succeeds(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert result.exit_code == 0, result.output
        out = result.output or ""
        assert "is required" not in out

    def test_server_help_with_session_id_silently_accepted(self):
        runner = CliRunner()
        sid = str(uuid.uuid4())
        result = runner.invoke(cli, ["--session-id", sid, "server", "--help"])
        assert result.exit_code == 0, result.output
        combined = (result.output or "").lower()
        assert "unused" not in combined
        assert "unexpected" not in combined
        assert "no such option" not in combined

    def test_server_invocation_without_session_id_runs_handler(self, monkeypatch):
        """Regression guard: the handler must NOT call _require_session_id,
        so invoking without --session-id must reach uvicorn.run (patched out).
        """
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["called"] = True
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])
        assert result.exit_code == 0, result.output
        assert captured["called"] is True


class TestWebUIDistWarning:
    """``create_app()`` emits the missing-WebUI-dist warning only when the
    default path is used; explicit overrides must stay quiet.
    """

    _WARNING_PREFIX = "warning: admin WebUI is not built"

    def test_warning_emitted_when_default_dir_missing(
        self, tmp_path, monkeypatch, capsys
    ):
        nonexistent = tmp_path / "never_built"
        assert not nonexistent.exists()
        monkeypatch.setattr(server_mod, "default_webui_dist_dir", lambda: nonexistent)
        server_mod.create_app()
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX in captured.err
        assert "mise //admin:build" in captured.err
        assert "/ui/" in captured.err

    def test_warning_suppressed_on_explicit_override(
        self, tmp_path, monkeypatch, capsys
    ):
        """Proves the ``emit_warning_if_missing = webui_dist_dir is None`` gate:
        explicit overrides suppress the warning even when the path is missing.
        """
        nonexistent = tmp_path / "never_built"
        assert not nonexistent.exists()
        monkeypatch.setattr(
            server_mod,
            "default_webui_dist_dir",
            lambda: tmp_path / "default_also_missing",
        )
        server_mod.create_app(webui_dist_dir=str(nonexistent))
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX not in captured.err

    def test_no_warning_when_default_dir_exists(self, tmp_path, monkeypatch, capsys):
        built = tmp_path / "dist"
        built.mkdir()
        monkeypatch.setattr(server_mod, "default_webui_dist_dir", lambda: built)
        server_mod.create_app()
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX not in captured.err


class TestBrokerHostDefault:
    def test_broker_host_default_is_loopback(self, monkeypatch):
        monkeypatch.delenv("BROKER_HOST", raising=False)
        monkeypatch.delenv("CAFLEET_BROKER_HOST", raising=False)
        s = Settings()
        assert s.broker_host == "127.0.0.1"

    def test_broker_port_default_is_8000(self, monkeypatch):
        """Asserted alongside broker_host so any accidental regression in
        the Field() rewrite is caught.
        """
        monkeypatch.delenv("BROKER_PORT", raising=False)
        monkeypatch.delenv("CAFLEET_BROKER_PORT", raising=False)
        s = Settings()
        assert s.broker_port == 8000

    def test_cafleet_broker_host_env_var_is_read(self, monkeypatch):
        monkeypatch.delenv("BROKER_HOST", raising=False)
        monkeypatch.setenv("CAFLEET_BROKER_HOST", "10.20.30.40")
        s = Settings()
        assert s.broker_host == "10.20.30.40"

    def test_cafleet_broker_port_env_var_is_read(self, monkeypatch):
        monkeypatch.delenv("BROKER_PORT", raising=False)
        monkeypatch.setenv("CAFLEET_BROKER_PORT", "9876")
        s = Settings()
        assert s.broker_port == 9876
