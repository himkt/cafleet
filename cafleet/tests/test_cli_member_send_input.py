"""CLI tests for ``cafleet member send-input`` (design doc 0000027)."""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli

DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
OTHER_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"


# Sentinel for ``_agent(placement=...)`` so callers can pass explicit None
# (meaning "no placement row, exercise the missing-placement branch") without
# being silently coerced back to a default valid placement.
_UNSET: object = object()


def _placement(
    *,
    director_agent_id: str = DIRECTOR_ID,
    tmux_pane_id: str | None = PANE_ID,
    coding_agent: str = "claude",
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": tmux_pane_id,
        "coding_agent": coding_agent,
        "created_at": "2026-04-16T08:00:00+00:00",
    }


def _agent(
    *,
    agent_id: str = MEMBER_ID,
    name: str = MEMBER_NAME,
    placement: dict | None | object = _UNSET,
) -> dict:
    resolved_placement = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved_placement,
    }


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    """``ensure_tmux_available`` is a no-op for every test in this module."""
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_agent(monkeypatch):
    """``broker.get_agent`` returns a well-formed target for the Director."""
    monkeypatch.setattr(broker, "get_agent", lambda *_args, **_kw: _agent())


@pytest.fixture
def choice_recorder(monkeypatch):
    """Record every call into ``tmux.send_choice_key``.

    Uses ``raising=False`` so the fixture also works before the Programmer
    adds the helper to ``cafleet.tmux`` — a clean FAIL beats a setup ERROR
    during TDD.
    """
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_choice_key", fake, raising=False)
    return calls


@pytest.fixture
def freetext_recorder(monkeypatch):
    """Record every call into ``tmux.send_freetext_and_submit``.

    Uses ``raising=False`` so the fixture also works before the Programmer
    adds the helper to ``cafleet.tmux``.
    """
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_freetext_and_submit", fake, raising=False)
    return calls


def _invoke(runner, session_id, *extra_args, **invoke_kwargs):
    """Helper: call ``cafleet --session-id <sid> member send-input ...``."""
    return runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "send-input",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            *extra_args,
        ],
        **invoke_kwargs,
    )


def test_flag_validation__no_flag_supplied_exits_two(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id)
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "--choice and --freetext are mutually exclusive" in out


def test_flag_validation__choice_and_freetext_combo_exits_two(
    runner, session_id, happy_path_agent
):
    result = _invoke(
        runner,
        session_id,
        "--choice",
        "1",
        "--freetext",
        "hello",
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "--choice" in out
    assert "--freetext" in out
    assert "mutually exclusive" in out


@pytest.mark.parametrize("bad_digit", ["0", "4", "5", "-1", "a"])
def test_flag_validation__choice_out_of_range_exits_two(
    runner, session_id, happy_path_agent, bad_digit
):
    """``click.IntRange(1, 3)`` rejects anything outside {1,2,3}."""
    result = _invoke(runner, session_id, "--choice", bad_digit)
    assert result.exit_code == 2, result.output


@pytest.mark.parametrize(
    "bad_text",
    [
        "line1\nline2",
        "trailing\n",
        "\nleading",
        "carriage\rreturn",
        "mixed\r\nCRLF",
    ],
)
def test_flag_validation__freetext_with_newlines_exits_two(
    runner, session_id, happy_path_agent, bad_text
):
    result = _invoke(runner, session_id, "--freetext", bad_text)
    assert result.exit_code == 2, result.output
    assert "free text may not contain newlines" in (result.output or "")


def test_flag_validation__freetext_empty_string_is_accepted(
    runner, session_id, happy_path_agent, freetext_recorder
):
    """Empty string is valid -- submits an empty answer to the prompt."""
    result = _invoke(runner, session_id, "--freetext", "")
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == ""


def test_authorization_boundary__missing_agent_exits_one(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 1, result.output
    assert MEMBER_ID in (result.output or "")
    assert "not found" in (result.output or "").lower()


def test_authorization_boundary__placement_none_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=None),
    )
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "has no placement row" in out
    assert "cafleet member create" in out


def test_authorization_boundary__cross_director_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(
            placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
        ),
    )
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "is not a member of your team" in out
    assert OTHER_DIRECTOR_ID in out


def test_authorization_boundary__pending_pane_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
    )
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"member {MEMBER_ID}" in out
    assert "has no pane yet" in out
    assert "pending placement" in out


@pytest.mark.parametrize("digit", [1, 2, 3])
def test_choice_dispatch__choice_dispatches_with_matching_digit_and_pane(
    runner,
    session_id,
    happy_path_agent,
    choice_recorder,
    freetext_recorder,
    digit,
):
    result = _invoke(runner, session_id, "--choice", str(digit))
    assert result.exit_code == 0, result.output
    assert len(choice_recorder) == 1
    call = choice_recorder[0]
    assert call["digit"] == digit
    assert call["target_pane_id"] == PANE_ID
    assert len(freetext_recorder) == 0


def test_freetext_dispatch__freetext_plain_ascii_dispatches_exactly(
    runner,
    session_id,
    happy_path_agent,
    freetext_recorder,
    choice_recorder,
):
    result = _invoke(runner, session_id, "--freetext", "hello")
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == "hello"
    assert freetext_recorder[0]["target_pane_id"] == PANE_ID
    assert len(choice_recorder) == 0


def test_freetext_dispatch__freetext_shell_meta_delivered_as_literal_no_expansion(
    runner, session_id, happy_path_agent, freetext_recorder
):
    """Shell meta characters must reach the helper unchanged; they are
    delivered via ``subprocess.run([...], shell=False)`` so no shell
    ever sees them.
    """
    payload = "$(echo pwn) `backticks` $VAR ;&&|"
    result = _invoke(runner, session_id, "--freetext", payload)
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == payload


def test_freetext_dispatch__freetext_multibyte_delivered_as_literal(
    runner, session_id, happy_path_agent, freetext_recorder
):
    payload = "日本語 !@# テスト ✓"
    result = _invoke(runner, session_id, "--freetext", payload)
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == payload


def test_freetext_dispatch__freetext_key_name_lookalike_delivered_as_literal(
    runner, session_id, happy_path_agent, freetext_recorder
):
    """Key-name lookalikes (Enter, C-c, Esc) must be delivered as literal
    characters because the wrapper always uses ``-l`` for the free-text
    step, per the design doc's key-sequence table.
    """
    payload = "Enter C-c Esc"
    result = _invoke(runner, session_id, "--freetext", payload)
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == payload


def test_output_format__text_output_choice(
    runner, session_id, happy_path_agent, choice_recorder
):
    result = _invoke(runner, session_id, "--choice", "1")
    assert result.exit_code == 0, result.output
    assert f"Sent choice 1 to member {MEMBER_NAME} ({PANE_ID})." in (
        result.output or ""
    )


@pytest.mark.parametrize("digit", [1, 2, 3])
def test_output_format__text_output_choice_varies_by_digit(
    runner,
    session_id,
    happy_path_agent,
    choice_recorder,
    digit,
):
    result = _invoke(runner, session_id, "--choice", str(digit))
    assert result.exit_code == 0, result.output
    assert f"Sent choice {digit} to member " in (result.output or "")


def test_output_format__text_output_freetext(
    runner, session_id, happy_path_agent, freetext_recorder
):
    result = _invoke(runner, session_id, "--freetext", "hello")
    assert result.exit_code == 0, result.output
    assert f"Sent free text to member {MEMBER_NAME} ({PANE_ID})." in (
        result.output or ""
    )


def test_output_format__json_output_choice_has_four_keys(
    runner, session_id, happy_path_agent, choice_recorder
):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "member",
            "send-input",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            "--choice",
            "2",
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {
        "member_agent_id",
        "pane_id",
        "action",
        "value",
    }
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID
    assert data["action"] == "choice"
    assert data["value"] == "2"


def test_output_format__json_output_freetext_has_four_keys(
    runner, session_id, happy_path_agent, freetext_recorder
):
    payload = "hello world"
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "member",
            "send-input",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            "--freetext",
            payload,
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {
        "member_agent_id",
        "pane_id",
        "action",
        "value",
    }
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID
    assert data["action"] == "freetext"
    assert data["value"] == payload


# --- bash_flag_removed: regression guard: the old ``--bash`` flag no longer
# exists on ``cafleet member send-input``. Per .claude/rules/removal.md, the
# absence-as-test guards against re-introduction. ---


def test_bash_flag_removed__old_bash_flag_form_errors_with_no_such_option(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--bash", "x")
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "No such option" in out
    assert "--bash" in out


# --- freetext_bang_rejection: bang-prefix guardrail on ``--freetext``. Any
# value whose first non-whitespace character (per ``str.lstrip()``) is ``!`` is
# rejected with exit 2, because Claude Code's ``!`` shortcut would otherwise
# smuggle a shell command through the AskUserQuestion path and bypass the new
# ``cafleet member exec`` boundary. ---


def test_freetext_bang_rejection__freetext_leading_bang_rejected(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--freetext", "!ls")
    assert result.exit_code == 2, result.output
    assert "--freetext may not start with" in (result.output or "")


# --- freetext_bang_rejection_message_is_backend_neutral: design 0000046 §5/§10.
# The error wording must reference "the coding agent's shell-execution shortcut"
# (not "Claude Code's") so the message is accurate for both backends. The
# guidance to use ``cafleet member exec`` for shell dispatch is preserved. ---


def test_freetext_bang_rejection__error_wording_is_backend_neutral(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--freetext", "! pwd")
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "the coding agent's shell-execution shortcut" in out
    # The guidance to use ``cafleet member exec`` for shell dispatch is preserved.
    assert "cafleet member exec" in out
    # The old, claude-specific phrasing must not survive the softening.
    assert "Claude Code's shell-execution shortcut" not in out


def test_freetext_bang_rejection__freetext_whitespace_then_bang_rejected(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--freetext", "  !ls")
    assert result.exit_code == 2, result.output
    assert "--freetext may not start with" in (result.output or "")


def test_freetext_bang_rejection__freetext_lone_bang_rejected(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "--freetext", "!")
    assert result.exit_code == 2, result.output
    assert "--freetext may not start with" in (result.output or "")


def test_freetext_bang_rejection__freetext_bang_not_in_leading_position_accepted(
    runner, session_id, happy_path_agent, freetext_recorder
):
    result = _invoke(runner, session_id, "--freetext", "hi !")
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == "hi !"


def test_freetext_bang_rejection__freetext_empty_still_accepted(
    runner, session_id, happy_path_agent, freetext_recorder
):
    result = _invoke(runner, session_id, "--freetext", "")
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == ""


def test_freetext_bang_rejection__freetext_whitespace_only_accepted(
    runner, session_id, happy_path_agent, freetext_recorder
):
    result = _invoke(runner, session_id, "--freetext", "   ")
    assert result.exit_code == 0, result.output
    assert len(freetext_recorder) == 1
    assert freetext_recorder[0]["text"] == "   "
