"""Tests for ``cafleet.tmux`` (all subprocesses mocked)."""

import pytest

from cafleet import tmux


def test_ensure_tmux_available__errors_when_tmux_missing(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: None)
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    with pytest.raises(tmux.TmuxError, match="tmux binary not found on PATH"):
        tmux.ensure_tmux_available()


def test_ensure_tmux_available__errors_when_tmux_env_unset(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.delenv("TMUX", raising=False)
    with pytest.raises(tmux.TmuxError, match="must be run inside a tmux session"):
        tmux.ensure_tmux_available()


def test_director_context__parses_display_message(monkeypatch):
    monkeypatch.setenv("TMUX_PANE", "%0")

    def mock_run(args):
        assert args == [
            "tmux",
            "display-message",
            "-p",
            "-t",
            "%0",
            "#{session_name}|#{window_id}|#{pane_id}",
        ]
        return "main|@3|%0\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    ctx = tmux.director_context()
    assert ctx.session == "main"
    assert ctx.window_id == "@3"
    assert ctx.pane_id == "%0"


def test_director_context__errors_when_tmux_pane_unset(monkeypatch):
    monkeypatch.delenv("TMUX_PANE", raising=False)
    with pytest.raises(tmux.TmuxError, match="TMUX_PANE is not set"):
        tmux.director_context()


def test_split_window__returns_captured_pane_id(monkeypatch):
    def mock_run(args):
        assert args[0:3] == ["tmux", "split-window", "-t"]
        assert "@3" in args
        assert "-P" in args
        assert "-F" in args
        assert "#{pane_id}" in args
        assert "-e" in args
        return "%7\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    pane_id = tmux.split_window(
        target_window_id="@3",
        env={
            "CAFLEET_DATABASE_URL": "sqlite+aiosqlite:////tmp/registry.db",
        },
        command=["claude", "Hello world"],
    )
    assert pane_id == "%7"


def test_split_window__command_appended_directly_to_args(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "%8\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.split_window(
        target_window_id="@5",
        env={},
        command=["my-binary", "--flag", "value", "prompt text"],
    )
    assert captured_args[-4:] == ["my-binary", "--flag", "value", "prompt text"]


def test_split_window__claude_style_command(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "%9\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.split_window(
        target_window_id="@3",
        env={},
        command=[
            "claude",
            (
                "Load the 'cafleet' skill. Your agent_id is "
                "7ba91234-5678-90ab-cdef-112233445566."
            ),
        ],
    )
    assert captured_args[-2] == "claude"
    assert "cafleet" in captured_args[-1]
    assert "7ba91234" in captured_args[-1]


def test_split_window__env_vars_forwarded_as_flags(monkeypatch):
    """Only CAFLEET_DATABASE_URL is forwarded as -e KEY=VAL when set.

    Design doc 0000023: session_id and agent_id are now passed as literal
    CLI flags via the prompt text, not via tmux env inheritance. The only
    env var that member-create forwards is CAFLEET_DATABASE_URL so the
    spawned agent can reach the same SQLite file.
    """
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "%11\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.split_window(
        target_window_id="@3",
        env={
            "CAFLEET_DATABASE_URL": "sqlite+aiosqlite:////tmp/registry.db",
        },
        command=["claude", "prompt"],
    )
    env_pairs = []
    for i, a in enumerate(captured_args):
        if a == "-e" and i + 1 < len(captured_args):
            env_pairs.append(captured_args[i + 1])
    assert "CAFLEET_DATABASE_URL=sqlite+aiosqlite:////tmp/registry.db" in env_pairs
    for pair in env_pairs:
        assert not pair.startswith("CAFLEET_SESSION_ID=")
        assert not pair.startswith("CAFLEET_AGENT_ID=")
        assert not pair.startswith("CAFLEET_URL=")


def test_split_window__empty_env_emits_no_e_flags(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "%12\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.split_window(
        target_window_id="@3",
        env={},
        command=["claude", "prompt"],
    )
    assert "-e" not in captured_args


def test_send_exit__raises_tmuxerror_on_failure(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
        tmux.send_exit(target_pane_id="%7")


def test_send_exit__ignore_missing_swallows_pane_gone(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux send-keys -t %99 /exit Enter\n"
            "stderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.send_exit(target_pane_id="%99", ignore_missing=True)


def test_send_exit__ignore_missing_does_not_swallow_other_errors(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError("tmux command failed: server crashed")

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="server crashed"):
        tmux.send_exit(target_pane_id="%7", ignore_missing=True)


def test_capture_pane__invokes_correct_args(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "line 1\nline 2\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.capture_pane(target_pane_id="%7", lines=80)
    assert captured_args == ["tmux", "capture-pane", "-p", "-t", "%7", "-S", "-80"]
    assert result == "line 1\nline 2\n"


def test_capture_pane__raises_on_missing_pane(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux capture-pane -p -t %99 -S -80\n"
            "stderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="can't find pane"):
        tmux.capture_pane(target_pane_id="%99", lines=80)


def test_capture_pane__rejects_non_positive_lines(monkeypatch):
    with pytest.raises(tmux.TmuxError, match="lines must be positive, got 0"):
        tmux.capture_pane(target_pane_id="%7", lines=0)
    with pytest.raises(tmux.TmuxError, match="lines must be positive, got -1"):
        tmux.capture_pane(target_pane_id="%7", lines=-1)


def test_send_poll_trigger__success_returns_true(monkeypatch):
    """``--session-id`` is a root-group global option and MUST come
    before the subcommand; ``--agent-id`` is a per-subcommand option
    and MUST come after ``message poll``. This ordering is what click's
    parser actually accepts and is the literal string the recipient's
    Bash tool receives.
    """
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    captured_args = []

    def mock_run(args, **kwargs):
        captured_args.extend(args)
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_poll_trigger(
        target_pane_id="%7",
        session_id="sess-001",
        agent_id="agent-001",
    )
    assert result is True
    assert captured_args == [
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "-l",
        "cafleet --session-id sess-001 message poll --agent-id agent-001",
        "tmux",
        "send-keys",
        "-t",
        "%7",
        "Enter",
    ]


def test_send_poll_trigger__pane_not_found_returns_false(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

    def mock_run(args, **kwargs):
        raise tmux.TmuxError(
            "tmux command failed: tmux send-keys -t %99\nstderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_poll_trigger(
        target_pane_id="%99",
        session_id="sess-001",
        agent_id="agent-001",
    )
    assert result is False


def test_send_poll_trigger__tmux_binary_missing_returns_false(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: None)
    run_called = False

    def mock_run(args):
        nonlocal run_called
        run_called = True
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_poll_trigger(
        target_pane_id="%7",
        session_id="sess-001",
        agent_id="agent-001",
    )
    assert result is False
    assert not run_called


def test_send_poll_trigger__never_raises_on_tmux_error(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

    def mock_run(args, **kwargs):
        raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.send_poll_trigger(
        target_pane_id="%7",
        session_id="sess-001",
        agent_id="agent-001",
    )
    assert result is False


# --- send_poll_trigger_keystroke: regression guard for the literal keystroke
# shape pushed by ``send_poll_trigger``. The keystroke is the bare cafleet
# command and MUST carry ``message poll --agent-id <a>``. The recipient's Bash
# tool is enabled under ``--permission-mode dontAsk``, so the harness runs the
# keystroke as a normal Bash invocation.
#
# The two ``send-keys`` calls are inspected separately:
# - call 0: ``["tmux", "send-keys", "-t", <pane>, "-l", <keystroke>]``
# - call 1: ``["tmux", "send-keys", "-t", <pane>, "Enter"]``
# ---


def test_send_poll_trigger_keystroke__keystroke_starts_with_bare_cafleet(monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    ok = tmux.send_poll_trigger(
        target_pane_id="%5",
        session_id="550e8400-e29b-41d4-a716-446655440000",
        agent_id="7ba91234-5678-90ab-cdef-112233445566",
    )
    assert ok is True
    assert len(captured) == 2

    keystroke = captured[0][-1]
    assert keystroke.startswith("cafleet --session-id "), (
        "send_poll_trigger keystroke must start with `cafleet --session-id `; "
        f"got: {keystroke!r}"
    )
    assert "message poll --agent-id" in keystroke, (
        f"keystroke must carry `message poll --agent-id`; got: {keystroke!r}"
    )

    assert captured[1] == ["tmux", "send-keys", "-t", "%5", "Enter"]


def test_pane_exists__returns_true_when_pane_present_in_list(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return "%0\n%3\n%7\n%9\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    assert tmux.pane_exists(target_pane_id="%7") is True
    assert captured_args == [
        "tmux",
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}",
    ]


def test_pane_exists__returns_false_when_pane_absent(monkeypatch):
    def mock_run(args):
        return "%0\n%3\n%9\n"

    monkeypatch.setattr(tmux, "_run", mock_run)
    assert tmux.pane_exists(target_pane_id="%7") is False


def test_pane_exists__propagates_unrelated_tmux_error(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux list-panes -a -F #{pane_id}\n"
            "stderr: no server running on /tmp/tmux-1000/default"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="no server running"):
        tmux.pane_exists(target_pane_id="%7")


def test_kill_pane__invokes_correct_args(monkeypatch):
    captured_args = []

    def mock_run(args):
        captured_args.extend(args)
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    result = tmux.kill_pane(target_pane_id="%7")
    assert result is None
    assert captured_args == ["tmux", "kill-pane", "-t", "%7"]


def test_kill_pane__ignore_missing_swallows_pane_gone(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux kill-pane -t %99\nstderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.kill_pane(target_pane_id="%99", ignore_missing=True)


def test_kill_pane__ignore_missing_does_not_swallow_other_errors(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux kill-pane -t %7\n"
            "stderr: server exited unexpectedly"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
        tmux.kill_pane(target_pane_id="%7", ignore_missing=True)


def test_kill_pane__default_raises_even_for_pane_gone(monkeypatch):
    def mock_run(args):
        raise tmux.TmuxError(
            "tmux command failed: tmux kill-pane -t %99\nstderr: can't find pane: %99"
        )

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="can't find pane"):
        tmux.kill_pane(target_pane_id="%99")


class _FakeClock:
    """Deterministic stand-in for time.monotonic / time.sleep.

    monotonic() returns the accumulated virtual time; sleep(secs) advances it.
    """

    def __init__(self):
        self.now = 0.0
        self.sleep_calls: list[float] = []

    def monotonic(self) -> float:
        return self.now

    def sleep(self, secs: float) -> None:
        self.sleep_calls.append(secs)
        self.now += secs


def test_wait_for_pane_gone__returns_true_immediately_when_pane_already_gone(
    monkeypatch,
):
    clock = _FakeClock()
    monkeypatch.setattr("time.monotonic", clock.monotonic)
    monkeypatch.setattr("time.sleep", clock.sleep)

    poll_calls: list[str] = []

    def fake_pane_exists(*, target_pane_id):
        poll_calls.append(target_pane_id)
        return False

    monkeypatch.setattr(tmux, "pane_exists", fake_pane_exists)

    result = tmux.wait_for_pane_gone(target_pane_id="%7", timeout=2.0, interval=0.5)
    assert result is True
    assert poll_calls == ["%7"]
    assert clock.sleep_calls == []


def test_wait_for_pane_gone__returns_true_when_pane_disappears_mid_wait(monkeypatch):
    clock = _FakeClock()
    monkeypatch.setattr("time.monotonic", clock.monotonic)
    monkeypatch.setattr("time.sleep", clock.sleep)

    results = iter([True, True, False])
    poll_calls: list[str] = []

    def fake_pane_exists(*, target_pane_id):
        poll_calls.append(target_pane_id)
        return next(results)

    monkeypatch.setattr(tmux, "pane_exists", fake_pane_exists)

    result = tmux.wait_for_pane_gone(target_pane_id="%7", timeout=2.0, interval=0.5)
    assert result is True
    assert poll_calls == ["%7", "%7", "%7"]
    assert clock.sleep_calls == [0.5, 0.5]
    assert clock.now < 1.0 + 1e-9


def test_wait_for_pane_gone__returns_false_after_timeout(monkeypatch):
    clock = _FakeClock()
    monkeypatch.setattr("time.monotonic", clock.monotonic)
    monkeypatch.setattr("time.sleep", clock.sleep)

    poll_calls: list[str] = []

    def fake_pane_exists(*, target_pane_id):
        poll_calls.append(target_pane_id)
        return True

    monkeypatch.setattr(tmux, "pane_exists", fake_pane_exists)

    result = tmux.wait_for_pane_gone(target_pane_id="%7", timeout=2.0, interval=0.5)
    assert result is False
    assert len(poll_calls) == 5
    assert clock.sleep_calls == [0.5, 0.5, 0.5, 0.5]


def test_wait_for_pane_gone__propagates_tmux_error_from_pane_exists(monkeypatch):
    clock = _FakeClock()
    monkeypatch.setattr("time.monotonic", clock.monotonic)
    monkeypatch.setattr("time.sleep", clock.sleep)

    call_count = {"n": 0}

    def fake_pane_exists(*, target_pane_id):
        call_count["n"] += 1
        if call_count["n"] >= 2:
            raise tmux.TmuxError("tmux command failed: server exited unexpectedly")
        return True

    monkeypatch.setattr(tmux, "pane_exists", fake_pane_exists)

    with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
        tmux.wait_for_pane_gone(target_pane_id="%7", timeout=2.0, interval=0.5)
    assert call_count["n"] == 2


# --- send_bash_command: round-8 ``tmux.send_bash_command`` helper.
#
# Bash routing uses Claude Code's ``!`` keystroke convention: the keystroke
# ``! <command>`` followed by ``Enter`` enters Bash-input mode in the
# member's pane and runs ``<command>``. Unlike ``send_freetext_and_submit``,
# there is NO leading ``4`` keystroke (no AskUserQuestion gate), so the
# helper issues exactly two ``send-keys`` invocations.
# ---


def test_send_bash_command__sends_keystrokes_in_two_calls(monkeypatch):
    captured_calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured_calls.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.send_bash_command(target_pane_id="%5", command="git log -1 --oneline")
    assert len(captured_calls) == 2
    assert captured_calls[0] == [
        "tmux",
        "send-keys",
        "-t",
        "%5",
        "-l",
        "! git log -1 --oneline",
    ]
    assert captured_calls[1] == [
        "tmux",
        "send-keys",
        "-t",
        "%5",
        "Enter",
    ]


@pytest.mark.parametrize(
    "bad_command",
    [
        "line1\nline2",
        "trailing\n",
        "\nleading",
        "carriage\rreturn",
        "mixed\r\nCRLF",
    ],
)
def test_send_bash_command__rejects_newlines(monkeypatch, bad_command):
    run_calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        run_calls.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(tmux.TmuxError, match="(?i)newline"):
        tmux.send_bash_command(target_pane_id="%5", command=bad_command)
    assert run_calls == []


def test_send_bash_command__rejects_empty_command(monkeypatch):
    run_calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        run_calls.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(
        tmux.TmuxError,
        match="send_bash_command: command may not be empty",
    ):
        tmux.send_bash_command(target_pane_id="%5", command="")
    assert run_calls == []


@pytest.mark.parametrize(
    "whitespace_command",
    [" ", "   ", "\t", " \t ", "\t\t"],
)
def test_send_bash_command__rejects_whitespace_only_command(
    monkeypatch, whitespace_command
):
    run_calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        run_calls.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    with pytest.raises(
        tmux.TmuxError,
        match="send_bash_command: command may not be empty",
    ):
        tmux.send_bash_command(target_pane_id="%5", command=whitespace_command)
    assert run_calls == []


def test_send_bash_command__strips_surrounding_whitespace_from_command(monkeypatch):
    captured_calls: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured_calls.append(list(args))
        return ""

    monkeypatch.setattr(tmux, "_run", mock_run)
    tmux.send_bash_command(target_pane_id="%5", command="  git status  ")
    assert captured_calls[0] == [
        "tmux",
        "send-keys",
        "-t",
        "%5",
        "-l",
        "! git status",
    ]
