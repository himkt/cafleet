"""CLI tests for ``cafleet member prompt``."""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer
from tests.cli._member_helpers import (
    MEMBER_ID,
    MEMBER_NAME,
    PANE_ID,
    _member,
    _placement,
)


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    """``ensure_available`` is a no-op for every test in this module."""
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_member(monkeypatch):
    """``broker.get_member`` returns a well-formed target for the Director."""
    monkeypatch.setattr(broker, "get_member", lambda *_args, **_kw: _member())


@pytest.fixture
def prompt_recorder(monkeypatch):
    """Record every call into ``TmuxMultiplexer.send_prompt``.

    Uses ``raising=False`` so the fixture works before the Programmer adds
    the ``member prompt`` subcommand and the ``send_prompt`` multiplexer
    method — clean FAIL beats setup ERROR.
    """
    calls: list[dict] = []

    def fake(self, **kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(TmuxMultiplexer, "send_prompt", fake, raising=False)
    return calls


def _invoke(runner, fleet_id, *extra_args, **invoke_kwargs):
    """Helper: call ``cafleet member prompt --fleet-id <sid> ...``."""
    return runner.invoke(
        cli,
        [
            "member",
            "prompt",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            *extra_args,
        ],
        **invoke_kwargs,
    )


def test_plain_dispatch__text_pane_and_shell_false_forwarded(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    result = _invoke(runner, fleet_id, "please run /compact")
    assert result.exit_code == 0, result.output
    assert len(prompt_recorder) == 1
    call = prompt_recorder[0]
    assert call["target_pane_id"] == PANE_ID
    assert call["text"] == "please run /compact"
    assert call["shell"] is False


def test_shell_dispatch__shell_true_forwarded_without_bang_prefixing(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    """``--shell`` forwards ``shell=True``; the ``! `` prefix is applied at the
    multiplexer layer, not in the CLI."""
    result = _invoke(runner, fleet_id, "--shell", "git log -1 --oneline")
    assert result.exit_code == 0, result.output
    assert len(prompt_recorder) == 1
    call = prompt_recorder[0]
    assert call["target_pane_id"] == PANE_ID
    assert call["text"] == "git log -1 --oneline"
    assert call["shell"] is True


@pytest.mark.parametrize("shell_args", [(), ("--shell",)])
def test_dispatch__surrounding_whitespace_stripped_before_dispatch(
    runner, fleet_id, happy_path_member, prompt_recorder, shell_args
):
    result = _invoke(runner, fleet_id, *shell_args, "  git status  ")
    assert result.exit_code == 0, result.output
    assert prompt_recorder[0]["text"] == "git status"


def test_plain_dispatch__bang_leading_text_delivered_verbatim(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    """The flag performs no content inspection: plain-form TEXT beginning with
    ``!`` is delivered verbatim without the shell mechanics."""
    result = _invoke(runner, fleet_id, "! git status")
    assert result.exit_code == 0, result.output
    call = prompt_recorder[0]
    assert call["text"] == "! git status"
    assert call["shell"] is False


def test_plain_dispatch__text_output(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    result = _invoke(runner, fleet_id, "please run /compact")
    assert result.exit_code == 0, result.output
    assert (
        f"Sent prompt 'please run /compact' to member {MEMBER_NAME} ({PANE_ID})."
        in (result.output or "")
    )


def test_shell_dispatch__text_output(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    result = _invoke(runner, fleet_id, "--shell", "git status")
    assert result.exit_code == 0, result.output
    assert f"Sent shell prompt 'git status' to member {MEMBER_NAME} ({PANE_ID})." in (
        result.output or ""
    )


@pytest.mark.parametrize(
    ("shell_args", "expected_shell"),
    [((), False), (("--shell",), True)],
)
def test_json_output__four_keys_with_shell_flag(
    runner, fleet_id, happy_path_member, prompt_recorder, shell_args, expected_shell
):
    payload = "git log -1 --oneline"
    result = runner.invoke(
        cli,
        [
            "member",
            "prompt",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            "--json",
            *shell_args,
            payload,
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {"member_id", "pane_id", "text", "shell"}
    assert data["member_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID
    assert data["text"] == payload
    assert data["shell"] is expected_shell


def test_input_validation__missing_positional_exits_two(
    runner, fleet_id, happy_path_member
):
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 2, result.output
    assert "Missing argument" in (result.output or "")


@pytest.mark.parametrize("shell_args", [(), ("--shell",)])
@pytest.mark.parametrize("empty_text", ["", "   ", "\t"])
def test_input_validation__empty_text_exits_two(
    runner, fleet_id, happy_path_member, prompt_recorder, shell_args, empty_text
):
    result = _invoke(runner, fleet_id, *shell_args, empty_text)
    assert result.exit_code == 2, result.output
    assert "text may not be empty." in (result.output or "")
    assert prompt_recorder == []


@pytest.mark.parametrize("shell_args", [(), ("--shell",)])
@pytest.mark.parametrize(
    "bad_text",
    [
        "\n",
        "\r",
        "\r\n",
        "\nls",
        "ls\n",
        "line1\nline2",
        "carriage\rreturn",
    ],
)
def test_input_validation__text_with_newline_exits_two(
    runner, fleet_id, happy_path_member, prompt_recorder, shell_args, bad_text
):
    result = _invoke(runner, fleet_id, *shell_args, bad_text)
    assert result.exit_code == 2, result.output
    assert "text may not contain newlines." in (result.output or "")
    assert prompt_recorder == []


def test_input_validation__newline_check_precedes_empty_check(
    runner, fleet_id, happy_path_member, prompt_recorder
):
    """CLI-layer precedence is newline-first: a ``"\\n"``-only input (empty
    after strip) raises the newline error, not the empty error."""
    result = _invoke(runner, fleet_id, "\n")
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "text may not contain newlines." in out
    assert "text may not be empty." not in out


def test_authorization_boundary__missing_member_exits_one(
    runner, fleet_id, monkeypatch
):
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: None)
    result = _invoke(runner, fleet_id, "git log -1")
    assert result.exit_code == 1, result.output
    assert str(MEMBER_ID) in (result.output or "")
    assert "not found" in (result.output or "").lower()


def test_authorization_boundary__placement_none_exits_one_with_exact_message(
    runner, fleet_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_member",
        lambda *_a, **_kw: _member(placement=None),
    )
    result = _invoke(runner, fleet_id, "git log -1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"member {MEMBER_ID}" in out
    assert "has no placement row" in out
    assert "cafleet member create" in out


def test_authorization_boundary__pending_pane_exits_one_with_nothing_to_prompt(
    runner, fleet_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_member",
        lambda *_a, **_kw: _member(placement=_placement(mux_pane_id=None)),
    )
    result = _invoke(runner, fleet_id, "git log -1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"member {MEMBER_ID}" in out
    assert "has no pane yet" in out
    assert "pending placement" in out
    assert "nothing to prompt" in out


def test_tmux_unavailable__tmux_not_available_exits_one(
    runner, fleet_id, happy_path_member, monkeypatch
):
    def raise_unavailable(self):
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", raise_unavailable)
    result = _invoke(runner, fleet_id, "git log -1")
    assert result.exit_code == 1, result.output
    assert "cafleet member commands must be run inside a tmux session" in (
        result.output or ""
    )


def test_send_failure__multiplexer_error_maps_to_send_failed_exit_one(
    runner, fleet_id, happy_path_member, monkeypatch
):
    def raise_send_failure(self, **_kwargs):
        raise TmuxError("tmux command failed: server exited unexpectedly")

    monkeypatch.setattr(
        TmuxMultiplexer, "send_prompt", raise_send_failure, raising=False
    )
    result = _invoke(runner, fleet_id, "git log -1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "send failed:" in out
    assert "server exited unexpectedly" in out


def test_member_exec__no_longer_parses(runner, fleet_id):
    """``cafleet member exec`` is removed: Click's default unknown-subcommand
    error (exit 2)."""
    result = runner.invoke(
        cli,
        [
            "member",
            "exec",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            "git status",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "No such command" in (result.output or "")
