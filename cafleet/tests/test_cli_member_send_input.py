"""CLI tests for ``cafleet member send-input`` (design doc 0000027).

Per principle (iii) of design 0000061: per-flag and per-payload fragmentation
collapses to one parametrized test per behavioural group.
"""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli
from tests._member_cli_helpers import (
    DIRECTOR_ID,
    MEMBER_ID,
    MEMBER_NAME,
    OTHER_DIRECTOR_ID,
    PANE_ID,
    _agent,
    _placement,
)


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_agent(monkeypatch):
    monkeypatch.setattr(broker, "get_agent", lambda *_args, **_kw: _agent())


@pytest.fixture
def choice_recorder(monkeypatch):
    calls: list[dict] = []
    monkeypatch.setattr(tmux, "send_choice_key", lambda **kw: calls.append(kw), raising=False)
    return calls


@pytest.fixture
def freetext_recorder(monkeypatch):
    calls: list[dict] = []
    monkeypatch.setattr(
        tmux, "send_freetext_and_submit", lambda **kw: calls.append(kw), raising=False,
    )
    return calls


def _invoke(runner, session_id, *extra_args, json_output=False):
    args = ["--session-id", session_id]
    if json_output:
        args.append("--json")
    args.extend([
        "member", "send-input",
        "--agent-id", DIRECTOR_ID,
        "--member-id", MEMBER_ID,
        *extra_args,
    ])
    return runner.invoke(cli, args)


@pytest.mark.parametrize(
    ("scenario", "extra_args", "expected_substring"),
    [
        ("no_flag_supplied", [], "--choice and --freetext are mutually exclusive"),
        (
            "choice_and_freetext_combo",
            ["--choice", "1", "--freetext", "hello"],
            "mutually exclusive",
        ),
        ("choice_out_of_range_zero", ["--choice", "0"], None),
        ("choice_out_of_range_four", ["--choice", "4"], None),
        ("choice_out_of_range_negative", ["--choice", "-1"], None),
        ("choice_non_integer", ["--choice", "a"], None),
        (
            "freetext_with_newline",
            ["--freetext", "line1\nline2"],
            "free text may not contain newlines",
        ),
        (
            "freetext_with_trailing_newline",
            ["--freetext", "trailing\n"],
            "free text may not contain newlines",
        ),
        (
            "freetext_with_cr",
            ["--freetext", "carriage\rreturn"],
            "free text may not contain newlines",
        ),
    ],
)
def test_flag_validation(
    runner, session_id, happy_path_agent, scenario, extra_args, expected_substring
):
    result = _invoke(runner, session_id, *extra_args)
    assert result.exit_code == 2, result.output
    if expected_substring is not None:
        assert expected_substring in (result.output or "")


@pytest.mark.parametrize(
    ("scenario", "agent_return", "expected_substrings"),
    [
        ("missing_agent", None, [MEMBER_ID, "not found"]),
        (
            "placement_none",
            "placement_none_sentinel",
            [f"agent {MEMBER_ID}", "has no placement row", "cafleet member create"],
        ),
        (
            "cross_director",
            "cross_director_sentinel",
            [f"agent {MEMBER_ID}", "is not a member of your team", OTHER_DIRECTOR_ID],
        ),
        (
            "pending_pane",
            "pending_pane_sentinel",
            [f"member {MEMBER_ID}", "has no pane yet", "pending placement"],
        ),
    ],
)
def test_authorization_boundary(
    runner, session_id, monkeypatch, scenario, agent_return, expected_substrings
):
    if agent_return is None:
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    elif agent_return == "placement_none_sentinel":
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))
    elif agent_return == "cross_director_sentinel":
        monkeypatch.setattr(
            broker, "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )
    else:  # pending_pane
        monkeypatch.setattr(
            broker, "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 1, result.output
    out = (result.output or "").lower() if scenario == "missing_agent" else (result.output or "")
    for needle in expected_substrings:
        haystack = out
        needle_check = needle.lower() if scenario == "missing_agent" else needle
        assert needle_check in haystack


@pytest.mark.parametrize("digit", [1, 2, 3])
def test_choice_dispatch__matching_digit_and_pane(
    runner, session_id, happy_path_agent, choice_recorder, freetext_recorder, digit
):
    result = _invoke(runner, session_id, "--choice", str(digit))
    assert result.exit_code == 0, result.output
    assert len(choice_recorder) == 1
    assert choice_recorder[0]["digit"] == digit
    assert choice_recorder[0]["target_pane_id"] == PANE_ID
    assert freetext_recorder == []


@pytest.mark.parametrize(
    ("scenario", "payload"),
    [
        ("plain_ascii", "hello"),
        ("shell_meta_literal", "$(echo pwn) `backticks` $VAR ;&&|"),
        ("multibyte_literal", "日本語 !@# テスト ✓"),
        ("key_name_lookalike", "Enter C-c Esc"),
        ("empty_string", ""),
    ],
)
def test_freetext_dispatch__literal_passthrough(
    runner, session_id, happy_path_agent, freetext_recorder, choice_recorder,
    scenario, payload,
):
    result = _invoke(runner, session_id, "--freetext", payload)
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == payload
    assert freetext_recorder[0]["target_pane_id"] == PANE_ID
    assert choice_recorder == []


@pytest.mark.parametrize(
    ("action", "args_extra", "expected_substring", "recorder_fixture"),
    [
        ("choice_1", ["--choice", "1"], "Sent choice 1 to member ", "choice_recorder"),
        ("choice_2", ["--choice", "2"], "Sent choice 2 to member ", "choice_recorder"),
        ("choice_3", ["--choice", "3"], "Sent choice 3 to member ", "choice_recorder"),
        (
            "freetext",
            ["--freetext", "hello"],
            f"Sent free text to member {MEMBER_NAME} ({PANE_ID}).",
            "freetext_recorder",
        ),
    ],
)
def test_output_format__text(
    runner, session_id, happy_path_agent, choice_recorder, freetext_recorder,
    action, args_extra, expected_substring, recorder_fixture,
):
    result = _invoke(runner, session_id, *args_extra)
    assert result.exit_code == 0, result.output
    assert expected_substring in (result.output or "")


@pytest.mark.parametrize(
    ("scenario", "args_extra", "expected_action", "expected_value"),
    [
        ("choice", ["--choice", "2"], "choice", "2"),
        ("freetext", ["--freetext", "hello world"], "freetext", "hello world"),
    ],
)
def test_output_format__json_envelope(
    runner, session_id, happy_path_agent, choice_recorder, freetext_recorder,
    scenario, args_extra, expected_action, expected_value,
):
    result = _invoke(runner, session_id, *args_extra, json_output=True)
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {"member_agent_id", "pane_id", "action", "value"}
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID
    assert data["action"] == expected_action
    assert data["value"] == expected_value


def test_bash_flag_removed__old_bash_flag_form_errors_with_no_such_option(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--bash", "x")
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "No such option" in out
    assert "--bash" in out


@pytest.mark.parametrize(
    ("scenario", "payload", "expect_exit", "expect_in", "expect_not_in"),
    [
        ("leading_bang_rejected", "!ls", 2, ["--freetext may not start with"], []),
        ("whitespace_then_bang_rejected", "  !ls", 2, ["--freetext may not start with"], []),
        ("lone_bang_rejected", "!", 2, ["--freetext may not start with"], []),
        (
            "error_wording_backend_neutral",
            "! pwd",
            2,
            ["the coding agent's shell-execution shortcut", "cafleet member exec"],
            ["Claude Code's shell-execution shortcut"],
        ),
        ("bang_not_in_leading_position_accepted", "hi !", 0, [], []),
        ("empty_accepted", "", 0, [], []),
        ("whitespace_only_accepted", "   ", 0, [], []),
    ],
)
def test_freetext_bang_rejection(
    runner, session_id, happy_path_agent, freetext_recorder,
    scenario, payload, expect_exit, expect_in, expect_not_in,
):
    result = _invoke(runner, session_id, "--freetext", payload)
    assert result.exit_code == expect_exit, result.output
    out = result.output or ""
    for needle in expect_in:
        assert needle in out
    for needle in expect_not_in:
        assert needle not in out
    if expect_exit == 0:
        assert len(freetext_recorder) == 1
        assert freetext_recorder[0]["text"] == payload
