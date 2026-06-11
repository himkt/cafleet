"""Tests for the ``cafleet server`` CLI subcommand."""

import pytest
import uvicorn
from click.testing import CliRunner

from cafleet.cli import cli
from cafleet.config import Settings, settings
from cafleet.webui import app as webui_app


@pytest.fixture
def uvicorn_recorder(monkeypatch):
    captured: dict = {}

    def fake_run(*args, **kwargs):
        captured["args"] = args
        captured["kwargs"] = kwargs

    monkeypatch.setattr(uvicorn, "run", fake_run)
    return captured


def test_server_help__shows_host_port_and_fleet_id_accepted_silently():
    runner = CliRunner()
    no_fleet_result = runner.invoke(cli, ["server", "--help"])
    assert no_fleet_result.exit_code == 0
    assert "--host" in no_fleet_result.output
    assert "--port" in no_fleet_result.output
    assert "is required" not in no_fleet_result.output

    # --fleet-id is silently accepted and ignored on `server`.
    sid = "100"
    with_fleet = runner.invoke(cli, ["--fleet-id", sid, "server", "--help"])
    assert with_fleet.exit_code == 0
    out_lower = with_fleet.output.lower()
    for forbidden in ("unused", "unexpected", "no such option"):
        assert forbidden not in out_lower


@pytest.mark.parametrize(
    ("scenario", "argv", "expected_exit", "expected_host", "expected_port"),
    [
        (
            "default_flags_match_settings",
            [],
            0,
            settings.broker_host,
            settings.broker_port,
        ),
        (
            "explicit_flags_override_defaults",
            ["--host", "0.0.0.0", "--port", "9000"],
            0,
            "0.0.0.0",
            9000,
        ),
        ("non_integer_port_rejected", ["--port", "not-a-port"], 2, None, None),
    ],
)
def test_server_command__flag_parsing(
    uvicorn_recorder, scenario, argv, expected_exit, expected_host, expected_port
):
    runner = CliRunner()
    result = runner.invoke(cli, ["server", *argv])
    assert result.exit_code == expected_exit, result.output
    if expected_exit == 0:
        kwargs = uvicorn_recorder["kwargs"]
        assert kwargs["host"] == expected_host
        assert kwargs["port"] == expected_port


def test_server_command__app_string_first_positional_and_handler_reached(
    uvicorn_recorder,
):
    runner = CliRunner()
    result = runner.invoke(cli, ["server"])
    assert result.exit_code == 0, result.output
    args = uvicorn_recorder["args"]
    assert args
    assert args[0] == "cafleet.webui.app:app"


@pytest.mark.parametrize(
    ("scenario", "default_dir_exists", "explicit_override", "expect_warning"),
    [
        ("default_dir_missing_warns", False, None, True),
        ("explicit_override_suppresses_warning", False, "explicit_path", False),
        ("default_dir_exists_no_warning", True, None, False),
    ],
)
def test_webui_dist_warning__matrix(
    tmp_path,
    monkeypatch,
    capsys,
    scenario,
    default_dir_exists,
    explicit_override,
    expect_warning,
):
    warning_prefix = "warning: admin WebUI is not built"
    if default_dir_exists:
        built = tmp_path / "dist"
        built.mkdir()
        monkeypatch.setattr(webui_app, "default_webui_dist_dir", lambda: built)
        webui_app.create_app()
    elif explicit_override:
        explicit_path = tmp_path / "never_built"
        monkeypatch.setattr(
            webui_app,
            "default_webui_dist_dir",
            lambda: tmp_path / "default_also_missing",
        )
        webui_app.create_app(webui_dist_dir=str(explicit_path))
    else:
        nonexistent = tmp_path / "never_built"
        monkeypatch.setattr(webui_app, "default_webui_dist_dir", lambda: nonexistent)
        webui_app.create_app()

    captured = capsys.readouterr()
    if expect_warning:
        assert warning_prefix in captured.err
        assert "mise //admin:build" in captured.err
        assert "warning: admin WebUI is not built. / will return 404." in captured.err
    else:
        assert warning_prefix not in captured.err


@pytest.mark.parametrize(
    ("scenario", "env_var", "env_value", "expected_attr", "expected_value"),
    [
        ("broker_host_default_loopback", None, None, "broker_host", "127.0.0.1"),
        (
            "broker_host_env_override",
            "CAFLEET_BROKER_HOST",
            "10.20.30.40",
            "broker_host",
            "10.20.30.40",
        ),
        ("broker_port_default_8000", None, None, "broker_port", 8000),
        (
            "broker_port_env_override",
            "CAFLEET_BROKER_PORT",
            "9876",
            "broker_port",
            9876,
        ),
    ],
)
def test_settings__broker_host_port_defaults_and_env(
    monkeypatch, scenario, env_var, env_value, expected_attr, expected_value
):
    # Clear both prefixed and unprefixed forms.
    for var in (
        "BROKER_HOST",
        "BROKER_PORT",
        "CAFLEET_BROKER_HOST",
        "CAFLEET_BROKER_PORT",
    ):
        monkeypatch.delenv(var, raising=False)
    if env_var is not None:
        monkeypatch.setenv(env_var, env_value)
    s = Settings()
    assert getattr(s, expected_attr) == expected_value
