"""Tests for ``cafleet.multiplexer.herdr.HerdrMultiplexer`` (all subprocesses mocked).

The herdr backend shells out through two dispatchers: ``_run`` (returns raw
stdout) and ``_run_json`` (parses the ``{"result": {...}}`` envelope and returns
the ``result`` object). ``_run_json`` calls the module-level ``_run``, so every
test monkeypatches ``herdr._run`` alone — capturing the exact argv and feeding a
per-call return value (a JSON envelope for the read paths, ``""`` for the
keystroke/write paths, or an exception to model a failure). This mirrors
``test_tmux.py``'s single-``_run`` mock and pins the herdr argv for each
``Multiplexer`` method plus the ``AgentStateAware`` capability.
"""

import json

import pytest

from cafleet.multiplexer import herdr as multiplexer_herdr
from cafleet.multiplexer.herdr import HerdrError, HerdrMultiplexer

# Stateless class — one shared instance for all tests is safe.
_herdr = HerdrMultiplexer()


def _envelope(result) -> str:
    """A herdr success envelope carrying ``result`` (a dict, or None for a
    no-neighbor ``pane``)."""
    return json.dumps({"id": 1, "result": result, "type": "response"})


@pytest.fixture
def herdr_run(monkeypatch):
    """Patch ``herdr._run`` to capture argv and feed per-call return values.

    Returns ``(captured, set_returns)``: ``captured`` is the list of argv lists
    in call order; ``set_returns(*vals)`` primes the per-call return sequence.
    A primed value that is an exception is raised (modelling a subprocess
    failure); once the sequence is exhausted every further call returns ``""``
    (the keystroke/write paths ignore stdout).
    """
    captured: list[list[str]] = []
    state: dict = {"returns": iter(())}

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        try:
            val = next(state["returns"])
        except StopIteration:
            return ""
        if isinstance(val, BaseException):
            raise val
        return val

    monkeypatch.setattr(multiplexer_herdr, "_run", mock_run)

    def set_returns(*vals) -> None:
        state["returns"] = iter(vals)

    return captured, set_returns


# --- ensure_available ------------------------------------------------------


@pytest.mark.parametrize(
    ("which_return", "herdr_env_set", "expected_match"),
    [
        (None, True, "herdr binary not found on PATH"),
        ("/usr/bin/herdr", False, "must be run inside a herdr session"),
    ],
)
def test_ensure_available__detects_binary_and_session_env(
    monkeypatch, which_return, herdr_env_set, expected_match
):
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    if herdr_env_set:
        monkeypatch.setenv("HERDR_ENV", "1")
    else:
        monkeypatch.delenv("HERDR_ENV", raising=False)
    with pytest.raises(HerdrError, match=expected_match):
        _herdr.ensure_available()


def test_ensure_available__succeeds_when_binary_and_env_present(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setenv("HERDR_ENV", "1")
    assert _herdr.ensure_available() is None


# --- context_discovery -----------------------------------------------------


def test_context_discovery__parses_pane_current_json(herdr_run):
    captured, set_returns = herdr_run
    set_returns(
        _envelope(
            {"pane": {"workspace_id": "wG", "tab_id": "wG:t1", "pane_id": "wG:p1"}}
        )
    )
    ctx = _herdr.context_discovery()
    assert captured == [["herdr", "pane", "current"]]
    # session ← workspace_id, window_id ← tab_id, pane_id ← pane_id.
    assert ctx.session == "wG"
    assert ctx.window_id == "wG:t1"
    assert ctx.pane_id == "wG:p1"


def test_context_discovery__missing_field_raises(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"workspace_id": "wG", "tab_id": "wG:t1"}}))
    with pytest.raises(HerdrError, match="missing"):
        _herdr.context_discovery()


# NOTE: split_window layout coverage (the pane-neighbor walk, the first-member
# --direction right vs subsequent --direction down branch, _neighbor's
# has-/no-neighbor cases, and the _equalize_column resize sequence) is added once
# the herdr layout impl stabilizes — the operator's live validation is still
# settling the real `pane neighbor` JSON shape.


# --- list_pane_ids ---------------------------------------------------------


def test_list_pane_ids__collects_pane_id_set(herdr_run):
    captured, set_returns = herdr_run
    set_returns(_envelope({"panes": [{"pane_id": "wG:p1"}, {"pane_id": "wG:p2"}]}))
    assert _herdr.list_pane_ids() == {"wG:p1", "wG:p2"}
    assert captured == [["herdr", "pane", "list"]]


# --- pane_exists / wait_for_pane_gone --------------------------------------


def test_pane_exists__present_returns_true(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")  # pane get succeeds
    assert _herdr.pane_exists(target_pane_id="wG:p1") is True
    assert captured == [["herdr", "pane", "get", "wG:p1"]]


def test_pane_exists__pane_not_found_returns_false(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    assert _herdr.pane_exists(target_pane_id="wG:p1") is False


def test_pane_exists__other_error_propagates(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    with pytest.raises(HerdrError, match="server unreachable"):
        _herdr.pane_exists(target_pane_id="wG:p1")


def test_wait_for_pane_gone__returns_true_when_pane_disappears(monkeypatch, herdr_run):
    _captured, set_returns = herdr_run
    monkeypatch.setattr("time.sleep", lambda _s: None)
    # present, present, then pane_not_found → gone.
    set_returns(
        "",
        "",
        HerdrError("herdr command failed", code="pane_not_found"),
    )
    assert (
        _herdr.wait_for_pane_gone(target_pane_id="wG:p1", timeout=5.0, interval=0.1)
        is True
    )


# --- kill_pane (not_found-tolerant teardown) -------------------------------


def test_kill_pane__argv_and_success(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    assert _herdr.kill_pane(target_pane_id="wG:p1") is None
    assert captured == [["herdr", "pane", "close", "wG:p1"]]


def test_kill_pane__ignore_missing_swallows_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    # ignore_missing=True swallows the pane-not-found teardown race.
    assert _herdr.kill_pane(target_pane_id="wG:p1", ignore_missing=True) is None


def test_kill_pane__ignore_missing_does_not_swallow_other_errors(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed: server unreachable"))
    with pytest.raises(HerdrError, match="server unreachable"):
        _herdr.kill_pane(target_pane_id="wG:p1", ignore_missing=True)


def test_kill_pane__default_raises_on_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    with pytest.raises(HerdrError):
        _herdr.kill_pane(target_pane_id="wG:p1")


# --- send_exit -------------------------------------------------------------


def test_send_exit__argv_and_ignore_missing(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    _herdr.send_exit(target_pane_id="wG:p1")
    assert captured == [["herdr", "pane", "run", "wG:p1", "/exit"]]


def test_send_exit__ignore_missing_swallows_pane_not_found(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command failed", code="pane_not_found"))
    assert _herdr.send_exit(target_pane_id="wG:p1", ignore_missing=True) is None


# --- send_poll_trigger (esc-safeguarded) -----------------------------------


def test_send_poll_trigger__esc_then_poll_command(monkeypatch, herdr_run):
    captured, set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    result = _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
    assert result is True
    # Leading esc safeguard, then the poll command via pane run (atomic submit).
    assert captured == [
        ["herdr", "pane", "send-keys", "wG:p1", "esc"],
        [
            "herdr",
            "pane",
            "run",
            "wG:p1",
            "cafleet message poll --fleet-id 24 --agent-id 88",
        ],
    ]


def test_send_poll_trigger__herdr_missing_returns_false(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: None)
    assert (
        _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
        is False
    )
    assert captured == []


def test_send_poll_trigger__herdr_error_returns_false(monkeypatch, herdr_run):
    _captured, set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    set_returns(HerdrError("herdr command failed: server unreachable"))
    assert (
        _herdr.send_poll_trigger(target_pane_id="wG:p1", fleet_id=24, agent_id=88)
        is False
    )


# --- send_wake_trigger (no esc) --------------------------------------------


def test_send_wake_trigger__no_esc_single_pane_run(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    result = _herdr.send_wake_trigger(
        target_pane_id="wG:p1",
        due_agents=[{"agent_id": 332, "name": "Director", "is_director": True}],
        director_agent_id=332,
    )
    assert result is True
    # No leading esc — the monitoring member's own pane is never on a prompt.
    assert len(captured) == 1
    argv = captured[0]
    assert argv[:4] == ["herdr", "pane", "run", "wG:p1"]
    assert "esc" not in argv
    payload = argv[4]
    assert payload.startswith("[monitor] wake: 1 agent due")
    assert "director 332 (Director)" in payload
    assert "(332)" in payload


def test_send_wake_trigger__sanitizes_name_metacharacters(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    _herdr.send_wake_trigger(
        target_pane_id="wG:p1",
        due_agents=[
            {"agent_id": 332, "name": "Director", "is_director": True},
            {"agent_id": 336, "name": "evil\r\nname\there`$(id)", "is_director": False},
        ],
        director_agent_id=332,
    )
    payload = captured[0][4]
    assert payload.startswith("[monitor] wake: 2 agents due")
    # Single-line + no shell metacharacters survive the sanitizer.
    assert "\n" not in payload
    assert "\r" not in payload
    assert "\t" not in payload
    assert "`" not in payload
    assert "$(" not in payload
    assert "⏎" in payload


# --- send_inline_preview (esc, send-text, enter) ---------------------------


def test_send_inline_preview__esc_send_text_enter(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    result = _herdr.send_inline_preview(
        target_pane_id="wG:p1",
        task_id=7,
        sender_id=42,
        ts="2026-07-06T00:00:00+00:00",
        text="hello there",
    )
    assert result is True
    assert captured == [
        ["herdr", "pane", "send-keys", "wG:p1", "esc"],
        [
            "herdr",
            "pane",
            "send-text",
            "wG:p1",
            "[cafleet msg 7 from 42 2026-07-06T00:00:00+00:00]\nhello there",
        ],
        ["herdr", "pane", "send-keys", "wG:p1", "enter"],
    ]


def test_send_inline_preview__sanitizes_body_newlines(monkeypatch, herdr_run):
    captured, _set_returns = herdr_run
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/herdr")
    monkeypatch.setattr("time.sleep", lambda _s: None)
    _herdr.send_inline_preview(
        target_pane_id="wG:p1",
        task_id=7,
        sender_id=42,
        ts="2026-07-06T00:00:00+00:00",
        text="line1\nline2\r\nline3",
    )
    # The single envelope/body separator newline is kept; body newlines fold to ⏎.
    payload = captured[1][4]
    header, body = payload.split("\n", 1)
    assert header == "[cafleet msg 7 from 42 2026-07-06T00:00:00+00:00]"
    assert "\n" not in body
    assert "\r" not in body
    assert body == "line1⏎line2⏎line3"


# --- send_bash_command -----------------------------------------------------


def test_send_bash_command__argv_and_strip(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    _herdr.send_bash_command(target_pane_id="wG:p1", command="  git status  ")
    assert captured == [["herdr", "pane", "run", "wG:p1", "! git status"]]


@pytest.mark.parametrize(
    ("command", "expected_match"),
    [
        ("", "may not be empty"),
        ("   ", "may not be empty"),
        ("line1\nline2", "may not contain newlines"),
        ("carriage\rreturn", "may not contain newlines"),
    ],
)
def test_send_bash_command__validation(herdr_run, command, expected_match):
    captured, _set_returns = herdr_run
    with pytest.raises(HerdrError, match=expected_match):
        _herdr.send_bash_command(target_pane_id="wG:p1", command=command)
    assert captured == []


# --- capture_pane ----------------------------------------------------------


def test_capture_pane__argv_and_output_key(herdr_run):
    captured, set_returns = herdr_run
    set_returns(_envelope({"output": "line 1\nline 2"}))
    assert _herdr.capture_pane(target_pane_id="wG:p1", lines=20) == "line 1\nline 2"
    assert captured == [
        [
            "herdr",
            "pane",
            "read",
            "wG:p1",
            "--source",
            "recent-unwrapped",
            "--lines",
            "20",
        ]
    ]


def test_capture_pane__falls_back_to_content_key(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"content": "fallback text"}))
    assert _herdr.capture_pane(target_pane_id="wG:p1") == "fallback text"


@pytest.mark.parametrize("lines", [0, -1])
def test_capture_pane__rejects_non_positive_lines(herdr_run, lines):
    _captured, _set_returns = herdr_run
    with pytest.raises(HerdrError, match="lines must be positive"):
        _herdr.capture_pane(target_pane_id="wG:p1", lines=lines)


# --- AgentStateAware: agent_status -----------------------------------------


def test_agent_status__reads_native_state(herdr_run):
    captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"pane_id": "wG:p1", "agent_status": "blocked"}}))
    assert _herdr.agent_status(target_pane_id="wG:p1") == "blocked"
    assert captured == [["herdr", "pane", "get", "wG:p1"]]


def test_agent_status__empty_or_absent_status_is_none(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"pane": {"pane_id": "wG:p1", "agent_status": ""}}))
    assert _herdr.agent_status(target_pane_id="wG:p1") is None


def test_agent_status__missing_pane_raises(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(_envelope({"not_pane": {}}))
    with pytest.raises(HerdrError, match="missing"):
        _herdr.agent_status(target_pane_id="wG:p1")


# --- AgentStateAware: wait_agent_status ------------------------------------


def test_wait_agent_status__argv_and_true_on_success(herdr_run):
    captured, set_returns = herdr_run
    set_returns("")
    result = _herdr.wait_agent_status(
        target_pane_id="wG:p1", status="done", timeout_ms=5000
    )
    assert result is True
    assert captured == [
        [
            "herdr",
            "wait",
            "agent-status",
            "wG:p1",
            "--status",
            "done",
            "--timeout",
            "5000",
        ]
    ]


def test_wait_agent_status__false_on_error(herdr_run):
    _captured, set_returns = herdr_run
    set_returns(HerdrError("herdr command timed out"))
    assert (
        _herdr.wait_agent_status(target_pane_id="wG:p1", status="done", timeout_ms=1000)
        is False
    )
