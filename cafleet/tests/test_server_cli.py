"""Tests for the ``cafleet server`` CLI subcommand (design doc 0000024).

The subcommand launches the admin WebUI FastAPI app via uvicorn.  Design doc
Specification, Tests subsection defines these five behavioural contracts:

  1. ``cafleet server --help`` exits 0 and shows both ``--host`` / ``--port``.
  2. With ``uvicorn.run`` monkey-patched, flag parsing threads defaults from
     ``settings.broker_host`` / ``settings.broker_port`` and explicit flag
     values override them.
  3. ``cafleet server`` does NOT require ``--session-id``.  Supplying one is
     silently accepted (matches the ``db init`` / ``session *`` pattern).
  4. ``create_app()`` emits the missing-WebUI-dist warning to stderr when the
     default path does not exist; an explicit ``webui_dist_dir`` override
     suppresses the warning (proving the ``webui_dist_dir is None`` gate).
  5. ``Settings().broker_host`` default is ``"127.0.0.1"`` (flipped from
     ``"0.0.0.0"`` to match CAFleet's local-only stance) and the
     ``CAFLEET_BROKER_HOST`` / ``CAFLEET_BROKER_PORT`` aliases are wired.

These tests rely on changes delivered in Steps 4-6 (``config.py``,
``server.py``, ``cli.py``) and WILL fail until those steps are implemented.
That is expected — this file is written first in the TDD cycle.
"""

import uuid

import uvicorn
from click.testing import CliRunner

from cafleet import server as server_mod
from cafleet.cli import cli
from cafleet.config import Settings


# ---------------------------------------------------------------------------
# (1) --help
# ---------------------------------------------------------------------------


class TestServerCommandHelp:
    """Design doc Success Criteria: ``cafleet server --help`` exits 0 and lists both flags."""

    def test_server_help_exits_zero(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert result.exit_code == 0, (
            f"'cafleet server --help' must exit 0. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )

    def test_server_help_mentions_host_flag(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert "--host" in result.output, (
            f"help must mention --host flag. got: {result.output!r}"
        )

    def test_server_help_mentions_port_flag(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert "--port" in result.output, (
            f"help must mention --port flag. got: {result.output!r}"
        )


# ---------------------------------------------------------------------------
# (2) flag parsing threads into uvicorn.run
# ---------------------------------------------------------------------------


class TestServerCommandFlagParsing:
    """Design doc Command behaviour: flags and settings defaults reach uvicorn.run."""

    def test_default_flags_pass_settings_defaults_to_uvicorn(self, monkeypatch):
        """No flags → uvicorn.run receives settings.broker_host / broker_port."""
        from cafleet.config import settings

        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])

        assert result.exit_code == 0, (
            f"'cafleet server' with patched uvicorn must exit 0. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        assert captured, "uvicorn.run was never called"
        kwargs = captured["kwargs"]
        assert kwargs.get("host") == settings.broker_host, (
            f"default host must equal settings.broker_host "
            f"({settings.broker_host!r}). got: {kwargs.get('host')!r}"
        )
        assert kwargs.get("port") == settings.broker_port, (
            f"default port must equal settings.broker_port "
            f"({settings.broker_port!r}). got: {kwargs.get('port')!r}"
        )

    def test_explicit_flags_override_defaults(self, monkeypatch):
        """--host 0.0.0.0 --port 9000 → uvicorn.run receives exactly those."""
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

        assert result.exit_code == 0, (
            f"'cafleet server --host 0.0.0.0 --port 9000' must exit 0. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        kwargs = captured.get("kwargs", {})
        assert kwargs.get("host") == "0.0.0.0", (
            f"--host 0.0.0.0 must pass host='0.0.0.0'. "
            f"got: {kwargs.get('host')!r}"
        )
        assert kwargs.get("port") == 9000, (
            f"--port 9000 must pass port=9000. got: {kwargs.get('port')!r}"
        )

    def test_app_import_string_passed_as_first_positional(self, monkeypatch):
        """uvicorn.run must receive 'cafleet.server:app' as the app spec."""
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["args"] = args
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])
        assert result.exit_code == 0, (
            f"'cafleet server' must exit 0 with patched uvicorn. "
            f"output: {result.output}, exception: {result.exception}"
        )
        args = captured.get("args", ())
        assert args and args[0] == "cafleet.server:app", (
            f"uvicorn.run must receive 'cafleet.server:app' as first arg. "
            f"got args: {args!r}, kwargs: {captured.get('kwargs')!r}"
        )

    def test_port_string_rejected_by_click_type_int(self):
        """--port foo → click's built-in int validation exits 2."""
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--port", "not-a-port"])
        assert result.exit_code != 0, (
            f"non-integer --port must be rejected by click. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )


# ---------------------------------------------------------------------------
# (3) session-id is NOT required (and silently accepted if supplied)
# ---------------------------------------------------------------------------


class TestServerDoesNotRequireSessionId:
    """Design doc Session-id gating: ``server`` joins the 'does NOT require
    --session-id' list alongside ``db init`` and ``session *``.

    Uses ``--help`` so ``uvicorn.run`` is never called and tests do not
    need to monkey-patch it.
    """

    def test_server_help_without_session_id_succeeds(self):
        runner = CliRunner()
        result = runner.invoke(cli, ["server", "--help"])
        assert result.exit_code == 0, (
            f"'cafleet server --help' without --session-id must exit 0. "
            f"exit_code={result.exit_code}, output: {result.output}"
        )
        out = result.output or ""
        assert "is required" not in out, (
            f"server --help must not complain about missing --session-id. "
            f"got: {out!r}"
        )

    def test_server_help_with_session_id_silently_accepted(self):
        runner = CliRunner()
        sid = str(uuid.uuid4())
        result = runner.invoke(cli, ["--session-id", sid, "server", "--help"])
        assert result.exit_code == 0, (
            f"'cafleet --session-id <uuid> server --help' must be silently "
            f"accepted. exit_code={result.exit_code}, output: {result.output}"
        )
        combined = (result.output or "").lower()
        assert "unused" not in combined, (
            f"must not warn about unused --session-id. got: {result.output!r}"
        )
        assert "unexpected" not in combined, (
            f"must not warn about unexpected --session-id. got: {result.output!r}"
        )
        assert "no such option" not in combined, (
            f"--session-id must be a valid global flag. got: {result.output!r}"
        )

    def test_server_invocation_without_session_id_runs_handler(self, monkeypatch):
        """Regression guard: the handler itself must NOT call
        ``_require_session_id`` — invoking without --session-id must reach
        uvicorn.run (patched out)."""
        captured: dict = {}

        def fake_run(*args, **kwargs):
            captured["called"] = True
            captured["kwargs"] = kwargs

        monkeypatch.setattr(uvicorn, "run", fake_run)

        runner = CliRunner()
        result = runner.invoke(cli, ["server"])
        assert result.exit_code == 0, (
            f"'cafleet server' (no --session-id) must exit 0. "
            f"exit_code={result.exit_code}, output: {result.output}, "
            f"exception: {result.exception}"
        )
        assert captured.get("called") is True, (
            f"server handler must reach uvicorn.run without --session-id. "
            f"captured={captured!r}, output={result.output!r}"
        )


# ---------------------------------------------------------------------------
# (4) missing WebUI dist warning fires from create_app()
# ---------------------------------------------------------------------------


class TestWebUIDistWarning:
    """Design doc ``server.py`` changes: ``create_app()`` emits the missing-
    WebUI-dist warning gated on ``webui_dist_dir is None`` so every real
    startup path (``cafleet server``, ``mise //cafleet:dev``, direct
    ``uv run uvicorn``) sees it while tests with explicit overrides stay
    quiet.
    """

    _WARNING_PREFIX = "warning: admin WebUI is not built"

    def test_warning_emitted_when_default_dir_missing(
        self, tmp_path, monkeypatch, capsys
    ):
        """Default path (webui_dist_dir=None) + non-existent dir → warning."""
        nonexistent = tmp_path / "never_built"
        assert not nonexistent.exists()
        monkeypatch.setattr(
            server_mod, "default_webui_dist_dir", lambda: nonexistent
        )
        server_mod.create_app()
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX in captured.err, (
            f"expected warning {self._WARNING_PREFIX!r} in stderr. "
            f"stderr={captured.err!r}, stdout={captured.out!r}"
        )
        assert "mise //admin:build" in captured.err, (
            f"warning must reference the 'mise //admin:build' remediation. "
            f"stderr={captured.err!r}"
        )
        assert "/ui/" in captured.err, (
            f"warning must mention /ui/ will 404. stderr={captured.err!r}"
        )

    def test_warning_suppressed_on_explicit_override(
        self, tmp_path, monkeypatch, capsys
    ):
        """Explicit webui_dist_dir override → no warning even if path missing.

        Proves the ``emit_warning_if_missing = webui_dist_dir is None`` gate:
        tests that pass explicit overrides must not be polluted by the
        warning even when their override path happens to be missing.
        """
        nonexistent = tmp_path / "never_built"
        assert not nonexistent.exists()
        # Also patch the default helper so the suppression proof is not
        # accidentally a side effect of the default path existing.
        monkeypatch.setattr(
            server_mod,
            "default_webui_dist_dir",
            lambda: tmp_path / "default_also_missing",
        )
        server_mod.create_app(webui_dist_dir=str(nonexistent))
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX not in captured.err, (
            f"warning must NOT fire when caller passes explicit "
            f"webui_dist_dir. stderr={captured.err!r}"
        )

    def test_no_warning_when_default_dir_exists(
        self, tmp_path, monkeypatch, capsys
    ):
        """Default path + existing dir → no warning."""
        built = tmp_path / "dist"
        built.mkdir()
        monkeypatch.setattr(server_mod, "default_webui_dist_dir", lambda: built)
        server_mod.create_app()
        captured = capsys.readouterr()
        assert self._WARNING_PREFIX not in captured.err, (
            f"warning must NOT fire when dist dir exists. "
            f"stderr={captured.err!r}"
        )


# ---------------------------------------------------------------------------
# (5) Settings.broker_host default + CAFLEET_* env var aliases
# ---------------------------------------------------------------------------


class TestBrokerHostDefault:
    """Design doc Success Criteria: ``Settings.broker_host`` default flipped to
    ``"127.0.0.1"`` (was ``"0.0.0.0"``) and ``broker_host`` / ``broker_port``
    gained ``validation_alias`` wiring for ``CAFLEET_BROKER_HOST`` /
    ``CAFLEET_BROKER_PORT``.
    """

    def test_broker_host_default_is_loopback(self, monkeypatch):
        """Fresh Settings() with no env override returns 127.0.0.1."""
        monkeypatch.delenv("BROKER_HOST", raising=False)
        monkeypatch.delenv("CAFLEET_BROKER_HOST", raising=False)
        s = Settings()
        assert s.broker_host == "127.0.0.1", (
            f"Settings().broker_host default must be '127.0.0.1'. "
            f"got: {s.broker_host!r}"
        )

    def test_broker_port_default_is_8000(self, monkeypatch):
        """Fresh Settings() with no env override returns 8000 for broker_port
        (unchanged by this cycle but asserted alongside broker_host so any
        accidental regression in the Field() rewrite is caught)."""
        monkeypatch.delenv("BROKER_PORT", raising=False)
        monkeypatch.delenv("CAFLEET_BROKER_PORT", raising=False)
        s = Settings()
        assert s.broker_port == 8000, (
            f"Settings().broker_port default must be 8000. "
            f"got: {s.broker_port!r}"
        )

    def test_cafleet_broker_host_env_var_is_read(self, monkeypatch):
        """validation_alias='CAFLEET_BROKER_HOST' → env var is honoured."""
        monkeypatch.delenv("BROKER_HOST", raising=False)
        monkeypatch.setenv("CAFLEET_BROKER_HOST", "10.20.30.40")
        s = Settings()
        assert s.broker_host == "10.20.30.40", (
            f"Settings() must read CAFLEET_BROKER_HOST from env. "
            f"got: {s.broker_host!r}"
        )

    def test_cafleet_broker_port_env_var_is_read(self, monkeypatch):
        """validation_alias='CAFLEET_BROKER_PORT' → env var is honoured."""
        monkeypatch.delenv("BROKER_PORT", raising=False)
        monkeypatch.setenv("CAFLEET_BROKER_PORT", "9876")
        s = Settings()
        assert s.broker_port == 9876, (
            f"Settings() must read CAFLEET_BROKER_PORT from env. "
            f"got: {s.broker_port!r}"
        )
