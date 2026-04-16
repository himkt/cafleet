"""Tests for ``cafleet member send-input`` CLI subcommand.

Design doc 0000027 (Step 5) — executable specification for the restricted
tmux-send-keys wrapper. Tests pin the following contracts:

  - Flag validation: exactly one of ``--choice`` / ``--freetext``; digit in
    {1,2,3}; free text rejects newlines
  - Authorization boundary: missing agent / no placement / cross-Director /
    pending pane all exit 1 with the exact error wording from the design
  - Dispatch: ``--choice N`` calls ``tmux.send_choice_key`` with the digit
    and the resolved pane; ``--freetext TEXT`` calls
    ``tmux.send_freetext_and_submit`` with the exact bytes (shell meta,
    multi-byte, empty string) — no shell expansion
  - Output: text = ``Sent choice <N> to member <name> (<pane>).`` /
    ``Sent free text to member <name> (<pane>).``; JSON carries the four
    documented keys (``member_agent_id``, ``pane_id``, ``action``, ``value``)

Every test uses ``CliRunner`` with ``--session-id`` threaded through the
root group, and monkeypatches ``broker.get_agent`` + the two ``tmux``
helpers so no real tmux subprocess is ever invoked. Tests MUST fail until
the Programmer implements ``tmux.send_choice_key``,
``tmux.send_freetext_and_submit``, and ``@member.command("send-input")``.
"""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli


# ---------------------------------------------------------------------------
# Shared fixtures
# ---------------------------------------------------------------------------


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
    """Build a fake broker.get_agent result.

    ``placement`` uses the ``_UNSET`` sentinel so the default (no kwarg) path
    installs a valid placement while an explicit ``placement=None`` from a
    caller passes through verbatim — the latter is required to exercise the
    "no placement row" branch of the CLI handler.
    """
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


# ===========================================================================
# TestFlagValidation
# ===========================================================================


class TestFlagValidation:
    """Design doc 0000027 Specification § "Validation rules"."""

    def test_neither_choice_nor_freetext_exits_two(
        self, runner, session_id, happy_path_agent
    ):
        result = _invoke(runner, session_id)
        assert result.exit_code == 2, (
            f"neither flag → exit 2 (click UsageError). "
            f"exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert "Must supply exactly one of --choice or --freetext" in (
            result.output or ""
        ), f"error must mention the exactly-one rule. got: {result.output!r}"

    def test_both_choice_and_freetext_exits_two(
        self, runner, session_id, happy_path_agent
    ):
        result = _invoke(
            runner,
            session_id,
            "--choice",
            "1",
            "--freetext",
            "hello",
        )
        assert result.exit_code == 2, (
            f"both flags → exit 2 (click UsageError). "
            f"exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert "Must supply exactly one of --choice or --freetext" in (
            result.output or ""
        ), f"error must mention the exactly-one rule. got: {result.output!r}"

    @pytest.mark.parametrize("bad_digit", ["0", "4", "5", "-1", "a"])
    def test_choice_out_of_range_exits_two(
        self, runner, session_id, happy_path_agent, bad_digit
    ):
        """``click.IntRange(1, 3)`` rejects anything outside {1,2,3}."""
        result = _invoke(runner, session_id, "--choice", bad_digit)
        assert result.exit_code == 2, (
            f"--choice {bad_digit!r} must exit 2 via click's built-in "
            f"IntRange validator. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )

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
    def test_freetext_with_newlines_exits_two(
        self, runner, session_id, happy_path_agent, bad_text
    ):
        result = _invoke(runner, session_id, "--freetext", bad_text)
        assert result.exit_code == 2, (
            f"--freetext with newline → exit 2. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        assert "free text may not contain newlines" in (result.output or ""), (
            f"error must mention the newline rule. got: {result.output!r}"
        )

    def test_freetext_empty_string_is_accepted(
        self, runner, session_id, happy_path_agent, freetext_recorder
    ):
        """Empty string is valid — submits an empty answer to the prompt."""
        result = _invoke(runner, session_id, "--freetext", "")
        assert result.exit_code == 0, (
            f'--freetext "" must be accepted. exit_code={result.exit_code}, '
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert len(freetext_recorder) == 1, (
            f"send_freetext_and_submit must be called exactly once. "
            f"got {len(freetext_recorder)} calls: {freetext_recorder!r}"
        )
        assert freetext_recorder[0]["text"] == "", (
            f"empty free text must pass through as empty string. "
            f"got: {freetext_recorder[0]!r}"
        )


# ===========================================================================
# TestAuthorizationBoundary
# ===========================================================================


class TestAuthorizationBoundary:
    """Design doc 0000027 Specification § "Authorization boundary"."""

    def test_missing_agent_exits_one(self, runner, session_id, monkeypatch):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
        result = _invoke(runner, session_id, "--choice", "1")
        assert result.exit_code == 1, (
            f"missing agent must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        assert MEMBER_ID in (result.output or ""), (
            f"error must mention the missing member_id {MEMBER_ID!r}. "
            f"got: {result.output!r}"
        )
        assert "not found" in (result.output or "").lower(), (
            f"error must say 'not found'. got: {result.output!r}"
        )

    def test_placement_none_exits_one_with_exact_message(
        self, runner, session_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=None),
        )
        result = _invoke(runner, session_id, "--choice", "1")
        assert result.exit_code == 1, (
            f"placement None must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out, (
            f"error must reference member_id. got: {out!r}"
        )
        assert "has no placement row" in out, (
            f"error must say 'has no placement row'. got: {out!r}"
        )
        assert "cafleet member create" in out, (
            f"error must hint at 'cafleet member create'. got: {out!r}"
        )

    def test_cross_director_exits_one_with_exact_message(
        self, runner, session_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )
        result = _invoke(runner, session_id, "--choice", "1")
        assert result.exit_code == 1, (
            f"cross-Director must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out, (
            f"error must reference member_id. got: {out!r}"
        )
        assert "is not a member of your team" in out, (
            f"error must mirror 'member capture' wording "
            f"('is not a member of your team'). got: {out!r}"
        )
        assert OTHER_DIRECTOR_ID in out, (
            f"error must disclose the actual director_agent_id "
            f"{OTHER_DIRECTOR_ID!r}. got: {out!r}"
        )

    def test_pending_pane_exits_one_with_exact_message(
        self, runner, session_id, monkeypatch
    ):
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )
        result = _invoke(runner, session_id, "--choice", "1")
        assert result.exit_code == 1, (
            f"pending placement must exit 1. exit_code={result.exit_code}, "
            f"output: {result.output!r}"
        )
        out = result.output or ""
        assert f"member {MEMBER_ID}" in out, (
            f"error must reference member_id. got: {out!r}"
        )
        assert "has no pane yet" in out, (
            f"error must say 'has no pane yet'. got: {out!r}"
        )
        assert "pending placement" in out, (
            f"error must say 'pending placement'. got: {out!r}"
        )


# ===========================================================================
# TestChoiceDispatch
# ===========================================================================


class TestChoiceDispatch:
    """``--choice N`` dispatches to ``tmux.send_choice_key`` once with the
    resolved pane + the matching digit, and does NOT touch
    ``send_freetext_and_submit``.
    """

    @pytest.mark.parametrize("digit", [1, 2, 3])
    def test_choice_dispatches_with_matching_digit_and_pane(
        self,
        runner,
        session_id,
        happy_path_agent,
        choice_recorder,
        freetext_recorder,
        digit,
    ):
        result = _invoke(runner, session_id, "--choice", str(digit))
        assert result.exit_code == 0, (
            f"--choice {digit} must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert len(choice_recorder) == 1, (
            f"send_choice_key must be called exactly once. "
            f"got {len(choice_recorder)} calls: {choice_recorder!r}"
        )
        call = choice_recorder[0]
        assert call.get("digit") == digit, (
            f"send_choice_key must receive digit={digit}. got: {call!r}"
        )
        assert call.get("target_pane_id") == PANE_ID, (
            f"send_choice_key must receive target_pane_id={PANE_ID!r}. got: {call!r}"
        )
        assert len(freetext_recorder) == 0, (
            f"--choice must NOT call send_freetext_and_submit. "
            f"got: {freetext_recorder!r}"
        )


# ===========================================================================
# TestFreetextDispatch
# ===========================================================================


class TestFreetextDispatch:
    """``--freetext TEXT`` dispatches to ``tmux.send_freetext_and_submit``
    with the exact byte-for-byte text — no shell expansion, no key-name
    interpretation, no multi-byte mangling.
    """

    def test_freetext_plain_ascii_dispatches_exactly(
        self,
        runner,
        session_id,
        happy_path_agent,
        freetext_recorder,
        choice_recorder,
    ):
        result = _invoke(runner, session_id, "--freetext", "hello")
        assert result.exit_code == 0, (
            f"--freetext 'hello' must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert len(freetext_recorder) == 1, (
            f"send_freetext_and_submit must be called exactly once. "
            f"got {len(freetext_recorder)} calls: {freetext_recorder!r}"
        )
        assert freetext_recorder[0].get("text") == "hello", (
            f"text must be 'hello' verbatim. got: {freetext_recorder[0]!r}"
        )
        assert freetext_recorder[0].get("target_pane_id") == PANE_ID, (
            f"pane_id must be {PANE_ID!r}. got: {freetext_recorder[0]!r}"
        )
        assert len(choice_recorder) == 0, (
            f"--freetext must NOT call send_choice_key. got: {choice_recorder!r}"
        )

    def test_freetext_shell_meta_delivered_as_literal_no_expansion(
        self, runner, session_id, happy_path_agent, freetext_recorder
    ):
        """Shell meta characters must reach the helper unchanged; they are
        delivered via ``subprocess.run([...], shell=False)`` so no shell
        ever sees them.
        """
        payload = "$(echo pwn) `backticks` $VAR ;&&|"
        result = _invoke(runner, session_id, "--freetext", payload)
        assert result.exit_code == 0, (
            f"shell-meta free text must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert len(freetext_recorder) == 1, (
            f"send_freetext_and_submit must be called exactly once. "
            f"got: {freetext_recorder!r}"
        )
        assert freetext_recorder[0].get("text") == payload, (
            f"shell meta must be delivered verbatim. "
            f"expected: {payload!r}, got: {freetext_recorder[0]!r}"
        )

    def test_freetext_multibyte_delivered_as_literal(
        self, runner, session_id, happy_path_agent, freetext_recorder
    ):
        payload = "日本語 !@# テスト ✓"
        result = _invoke(runner, session_id, "--freetext", payload)
        assert result.exit_code == 0, (
            f"multi-byte free text must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        assert len(freetext_recorder) == 1
        assert freetext_recorder[0].get("text") == payload, (
            f"multi-byte text must be delivered verbatim. "
            f"expected: {payload!r}, got: {freetext_recorder[0]!r}"
        )

    def test_freetext_key_name_lookalike_delivered_as_literal(
        self, runner, session_id, happy_path_agent, freetext_recorder
    ):
        """Text that *looks* like a key name (``Enter``, ``C-c``, ``Esc``)
        must be delivered as literal characters — the wrapper always uses
        ``-l`` for the free-text step, per the design doc's key-sequence
        table. (No special-case matching in the helper itself.)
        """
        payload = "Enter C-c Esc"
        result = _invoke(runner, session_id, "--freetext", payload)
        assert result.exit_code == 0, (
            f"key-name-lookalike free text must succeed. "
            f"exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert len(freetext_recorder) == 1
        assert freetext_recorder[0].get("text") == payload, (
            f"key-name-lookalike text must be delivered verbatim. "
            f"expected: {payload!r}, got: {freetext_recorder[0]!r}"
        )


# ===========================================================================
# TestOutputFormat
# ===========================================================================


class TestOutputFormat:
    """Text + JSON output contracts from the design doc."""

    def test_text_output_choice(
        self, runner, session_id, happy_path_agent, choice_recorder
    ):
        result = _invoke(runner, session_id, "--choice", "1")
        assert result.exit_code == 0, (
            f"expected success. exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert f"Sent choice 1 to member {MEMBER_NAME} ({PANE_ID})." in (
            result.output or ""
        ), (
            f"text output must match "
            f"'Sent choice 1 to member {MEMBER_NAME} ({PANE_ID}).'. "
            f"got: {result.output!r}"
        )

    @pytest.mark.parametrize("digit", [1, 2, 3])
    def test_text_output_choice_varies_by_digit(
        self,
        runner,
        session_id,
        happy_path_agent,
        choice_recorder,
        digit,
    ):
        result = _invoke(runner, session_id, "--choice", str(digit))
        assert result.exit_code == 0
        assert f"Sent choice {digit} to member " in (result.output or ""), (
            f"text output must embed the chosen digit. got: {result.output!r}"
        )

    def test_text_output_freetext(
        self, runner, session_id, happy_path_agent, freetext_recorder
    ):
        result = _invoke(runner, session_id, "--freetext", "hello")
        assert result.exit_code == 0, (
            f"expected success. exit_code={result.exit_code}, output: {result.output!r}"
        )
        assert f"Sent free text to member {MEMBER_NAME} ({PANE_ID})." in (
            result.output or ""
        ), (
            f"text output must match "
            f"'Sent free text to member {MEMBER_NAME} ({PANE_ID}).'. "
            f"got: {result.output!r}"
        )

    def test_json_output_choice_has_four_keys(
        self, runner, session_id, happy_path_agent, choice_recorder
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
        assert result.exit_code == 0, (
            f"--json --choice must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        data = json.loads(result.output)
        assert set(data.keys()) == {
            "member_agent_id",
            "pane_id",
            "action",
            "value",
        }, (
            f"JSON output must contain exactly the four documented keys. "
            f"got keys: {sorted(data.keys())!r}"
        )
        assert data["member_agent_id"] == MEMBER_ID
        assert data["pane_id"] == PANE_ID
        assert data["action"] == "choice"
        assert data["value"] == "2", (
            f"value must be the string '2' (stringified digit). got: {data!r}"
        )

    def test_json_output_freetext_has_four_keys(
        self, runner, session_id, happy_path_agent, freetext_recorder
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
        assert result.exit_code == 0, (
            f"--json --freetext must succeed. exit_code={result.exit_code}, "
            f"output: {result.output!r}, exception: {result.exception!r}"
        )
        data = json.loads(result.output)
        assert set(data.keys()) == {
            "member_agent_id",
            "pane_id",
            "action",
            "value",
        }, (
            f"JSON output must contain exactly the four documented keys. "
            f"got keys: {sorted(data.keys())!r}"
        )
        assert data["member_agent_id"] == MEMBER_ID
        assert data["pane_id"] == PANE_ID
        assert data["action"] == "freetext"
        assert data["value"] == payload, (
            f"value must be the text as sent. got: {data!r}"
        )
