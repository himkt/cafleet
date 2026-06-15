"""Tests for ``cafleet.multiplexer.tmux.TmuxMultiplexer`` (all subprocesses mocked)."""

import pytest

from cafleet.multiplexer import tmux as multiplexer_tmux
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

# Stateless class — one shared instance for all tests is safe.
_tmux = TmuxMultiplexer()


@pytest.fixture
def run_recorder(monkeypatch):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    return captured


@pytest.mark.parametrize(
    ("which_return", "tmux_env_set", "expected_match"),
    [
        (None, True, "tmux binary not found on PATH"),
        ("/usr/bin/tmux", False, "must be run inside a tmux session"),
    ],
)
def test_ensure_available__detects_binary_and_session_env(
    monkeypatch, which_return, tmux_env_set, expected_match
):
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    if tmux_env_set:
        monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    else:
        monkeypatch.delenv("TMUX", raising=False)
    with pytest.raises(TmuxError, match=expected_match):
        _tmux.ensure_available()


def test_context_discovery__parses_display_message_and_requires_tmux_pane(monkeypatch):
    # Happy path: argv matches and the parsed context fields are extracted.
    monkeypatch.setenv("TMUX_PANE", "%0")

    def mock_run(args, **_kwargs):
        assert args == [
            "tmux",
            "display-message",
            "-p",
            "-t",
            "%0",
            "#{session_name}|#{window_id}|#{pane_id}",
        ]
        return "main|@3|%0\n"

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    ctx = _tmux.context_discovery()
    assert ctx.session == "main"
    assert ctx.window_id == "@3"
    assert ctx.pane_id == "%0"

    # Missing TMUX_PANE → TmuxError.
    monkeypatch.delenv("TMUX_PANE", raising=False)
    with pytest.raises(TmuxError, match="TMUX_PANE is not set"):
        _tmux.context_discovery()


def test_split_window__argv_construction(monkeypatch, run_recorder):
    # 1. Returns captured pane id and includes -P/-F/-d/-e flags.
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: run_recorder.append(list(args)) or "%7\n",
    )
    pane_id = _tmux.split_window(
        target_window_id="@3",
        env={"CAFLEET_DATABASE_URL": "sqlite+aiosqlite:////tmp/cafleet.db"},
        command=["claude", "Hello world"],
    )
    assert pane_id == "%7"
    argv = run_recorder[0]
    assert argv[0:3] == ["tmux", "split-window", "-t"]
    assert "@3" in argv
    assert "-P" in argv
    assert "-F" in argv
    assert "#{pane_id}" in argv

    # `-d` is always present so the new pane is not made active and the
    # caller's active window stays put. Pin the position: after the
    # `-F #{pane_id}` block, before the `-e` env block, and before the
    # spawn command.
    assert "-d" in argv
    assert argv.index("-d") > argv.index("#{pane_id}")
    assert argv.index("-d") < argv.index("-e")
    assert argv.index("-d") < argv.index("claude")

    # 2. env vars forwarded as `-e KEY=VAL` flags.
    flag_idx = argv.index("-e")
    assert (
        argv[flag_idx + 1] == "CAFLEET_DATABASE_URL=sqlite+aiosqlite:////tmp/cafleet.db"
    )

    # 3. Command is appended directly after env flags.
    run_recorder.clear()
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: run_recorder.append(list(args)) or "%8\n",
    )
    _tmux.split_window(
        target_window_id="@5",
        env={},
        command=["my-binary", "--flag", "value", "prompt text"],
    )
    appended = run_recorder[0]
    assert appended[-4:] == ["my-binary", "--flag", "value", "prompt text"]
    # Empty env → no -e flags.
    assert "-e" not in appended
    # `-d` is still present and precedes the spawn command even when env is
    # empty (the helper always appends `-d` unconditionally).
    assert "-d" in appended
    assert appended.index("-d") < appended.index("my-binary")


_PANE_GONE_FAILURE = (
    "tmux command failed: tmux send-keys -t %99 Enter\nstderr: can't find pane: %99"
)
_NON_PANE_GONE_FAILURE = "tmux command failed: server exited unexpectedly"


@pytest.mark.parametrize(
    (
        "scenario",
        "error_by_call",
        "ignore_missing",
        "expect_raise_match",
        "expected_call_count",
    ),
    [
        ("success_two_invocations", {}, False, None, 2),
        (
            "ignore_missing_swallows_pane_gone_on_every_invocation",
            {0: _PANE_GONE_FAILURE, 1: _PANE_GONE_FAILURE},
            True,
            None,
            2,
        ),
        (
            "pane_dies_after_literal_send",
            {1: _PANE_GONE_FAILURE},
            True,
            None,
            2,
        ),
        (
            "non_pane_gone_error_propagates_despite_ignore_missing",
            {0: _NON_PANE_GONE_FAILURE},
            True,
            "server exited unexpectedly",
            1,
        ),
        (
            "pane_gone_on_first_invocation_raises_without_ignore_missing",
            {0: _PANE_GONE_FAILURE},
            False,
            "can't find pane",
            1,
        ),
        (
            "pane_gone_mid_sequence_raises_without_ignore_missing",
            {1: _PANE_GONE_FAILURE},
            False,
            "can't find pane",
            2,
        ),
    ],
)
def test_send_exit__success_and_ignore_missing_semantics(
    monkeypatch,
    scenario,
    error_by_call,
    ignore_missing,
    expect_raise_match,
    expected_call_count,
):
    captured: list[list[str]] = []
    sleeps: list[float] = []

    def mock_run(args, **_kwargs):
        call_index = len(captured)
        captured.append(list(args))
        if call_index in error_by_call:
            raise TmuxError(error_by_call[call_index])
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    monkeypatch.setattr("time.sleep", lambda secs: sleeps.append(secs))

    if expect_raise_match is not None:
        with pytest.raises(TmuxError, match=expect_raise_match):
            _tmux.send_exit(target_pane_id="%7", ignore_missing=ignore_missing)
    else:
        _tmux.send_exit(target_pane_id="%7", ignore_missing=ignore_missing)
        assert captured == [
            ["tmux", "send-keys", "-t", "%7", "-l", "/exit"],
            ["tmux", "send-keys", "-t", "%7", "Enter"],
        ]
        assert sleeps == [multiplexer_tmux._SUBMIT_DELAY]
    assert len(captured) == expected_call_count


@pytest.mark.parametrize(
    (
        "scenario",
        "error_by_call",
        "ignore_missing_kwargs",
        "expect_raise_match",
        "expected_call_count",
    ),
    [
        (
            "default_false_raises_on_pane_gone_literal_send",
            {0: _PANE_GONE_FAILURE},
            {},
            "can't find pane",
            1,
        ),
        (
            "default_false_raises_on_pane_gone_enter",
            {1: _PANE_GONE_FAILURE},
            {},
            "can't find pane",
            2,
        ),
        (
            "explicit_false_raises_on_pane_gone",
            {0: _PANE_GONE_FAILURE},
            {"ignore_missing": False},
            "can't find pane",
            1,
        ),
        (
            "true_swallows_pane_gone_on_literal_send",
            {0: _PANE_GONE_FAILURE},
            {"ignore_missing": True},
            None,
            2,
        ),
        (
            "true_swallows_pane_gone_on_enter",
            {1: _PANE_GONE_FAILURE},
            {"ignore_missing": True},
            None,
            2,
        ),
        (
            "true_does_not_swallow_other_errors",
            {0: _NON_PANE_GONE_FAILURE},
            {"ignore_missing": True},
            "server exited unexpectedly",
            1,
        ),
    ],
)
def test_send_literal_then_enter__ignore_missing_semantics(
    monkeypatch,
    scenario,
    error_by_call,
    ignore_missing_kwargs,
    expect_raise_match,
    expected_call_count,
):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        call_index = len(captured)
        captured.append(list(args))
        if call_index in error_by_call:
            raise TmuxError(error_by_call[call_index])
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    monkeypatch.setattr("time.sleep", lambda _secs: None)

    if expect_raise_match is not None:
        with pytest.raises(TmuxError, match=expect_raise_match):
            multiplexer_tmux._send_literal_then_enter(
                target_pane_id="%7", payload="hello", **ignore_missing_kwargs
            )
    else:
        multiplexer_tmux._send_literal_then_enter(
            target_pane_id="%7", payload="hello", **ignore_missing_kwargs
        )
        assert captured == [
            ["tmux", "send-keys", "-t", "%7", "-l", "hello"],
            ["tmux", "send-keys", "-t", "%7", "Enter"],
        ]
    assert len(captured) == expected_call_count


def test_send_literal_then_enter__forwards_timeout_to_run(monkeypatch):
    captured_timeouts: list[float | None] = []

    def mock_run(args, *, timeout=None, **_kwargs):
        captured_timeouts.append(timeout)
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    multiplexer_tmux._send_literal_then_enter(
        target_pane_id="%7", payload="hello", timeout=5
    )
    assert captured_timeouts == [5, 5]


@pytest.mark.parametrize(
    ("scenario", "lines", "mock_error", "expect_raise_match", "expect_argv_suffix"),
    [
        ("default_argv_lines_80", 80, None, None, ["-t", "%7", "-S", "-80"]),
        (
            "missing_pane_raises",
            80,
            TmuxError(
                "tmux command failed: tmux capture-pane -p -t %99 -S -80\n"
                "stderr: can't find pane: %99"
            ),
            "can't find pane",
            None,
        ),
        ("rejects_zero_lines", 0, None, "lines must be positive, got 0", None),
        ("rejects_negative_lines", -1, None, "lines must be positive, got -1", None),
    ],
)
def test_capture_pane__argv_and_validation(
    monkeypatch, scenario, lines, mock_error, expect_raise_match, expect_argv_suffix
):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        if mock_error is not None:
            raise mock_error
        return "line 1\nline 2\n"

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    if expect_raise_match is not None:
        with pytest.raises(TmuxError, match=expect_raise_match):
            _tmux.capture_pane(target_pane_id="%7", lines=lines)
    else:
        result = _tmux.capture_pane(target_pane_id="%7", lines=lines)
        assert result == "line 1\nline 2\n"
        assert captured[0] == ["tmux", "capture-pane", "-p", *expect_argv_suffix]


@pytest.mark.parametrize(
    ("scenario", "which_return", "mock_error", "expected_result", "expect_run_called"),
    [
        ("success_returns_true", "/usr/bin/tmux", None, True, True),
        (
            "pane_not_found_returns_false",
            "/usr/bin/tmux",
            TmuxError(
                "tmux command failed: tmux send-keys -t %99\n"
                "stderr: can't find pane: %99"
            ),
            False,
            True,
        ),
        ("tmux_binary_missing_returns_false", None, None, False, False),
        (
            "never_raises_on_tmux_error",
            "/usr/bin/tmux",
            TmuxError("tmux command failed: server exited unexpectedly"),
            False,
            True,
        ),
    ],
)
def test_send_poll_trigger__return_branches_and_argv(
    monkeypatch,
    scenario,
    which_return,
    mock_error,
    expected_result,
    expect_run_called,
):
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        if mock_error is not None:
            raise mock_error
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    result = _tmux.send_poll_trigger(
        target_pane_id="%7",
        fleet_id="sess-001",
        agent_id="agent-001",
    )
    assert result is expected_result
    if expect_run_called and scenario == "success_returns_true":
        # `send_poll_trigger` passes `esc_first=True`: the keystroke leads with
        # `Escape` (permission-prompt safeguard) before the unchanged poll
        # command + Enter (spec §1, §2).
        assert captured == [
            ["tmux", "send-keys", "-t", "%7", "Escape"],
            [
                "tmux",
                "send-keys",
                "-t",
                "%7",
                "-l",
                "cafleet message poll --fleet-id sess-001 --agent-id agent-001",
            ],
            ["tmux", "send-keys", "-t", "%7", "Enter"],
        ]
    elif not expect_run_called:
        assert captured == []


@pytest.mark.parametrize(
    ("scenario", "list_panes_return", "mock_error", "expected_result", "raise_match"),
    [
        ("pane_present", "%0\n%3\n%7\n%9\n", None, True, None),
        ("pane_absent", "%0\n%3\n%9\n", None, False, None),
        (
            "propagates_unrelated_error",
            None,
            TmuxError(
                "tmux command failed: tmux list-panes -a -F #{pane_id}\n"
                "stderr: no server running on /tmp/tmux-1000/default"
            ),
            None,
            "no server running",
        ),
    ],
)
def test_pane_exists__return_branches_and_argv(
    monkeypatch,
    scenario,
    list_panes_return,
    mock_error,
    expected_result,
    raise_match,
):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        if mock_error is not None:
            raise mock_error
        return list_panes_return

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    if raise_match is not None:
        with pytest.raises(TmuxError, match=raise_match):
            _tmux.pane_exists(target_pane_id="%7")
    else:
        assert _tmux.pane_exists(target_pane_id="%7") is expected_result
        assert captured[0] == ["tmux", "list-panes", "-a", "-F", "#{pane_id}"]


@pytest.mark.parametrize(
    ("scenario", "mock_error", "ignore_missing", "expect_raise_match"),
    [
        ("success_argv", None, False, None),
        (
            "ignore_missing_swallows_pane_gone",
            TmuxError(
                "tmux command failed: tmux kill-pane -t %99\n"
                "stderr: can't find pane: %99"
            ),
            True,
            None,
        ),
        (
            "ignore_missing_does_not_swallow_other_errors",
            TmuxError(
                "tmux command failed: tmux kill-pane -t %7\n"
                "stderr: server exited unexpectedly"
            ),
            True,
            "server exited unexpectedly",
        ),
        (
            "default_raises_even_for_pane_gone",
            TmuxError(
                "tmux command failed: tmux kill-pane -t %99\n"
                "stderr: can't find pane: %99"
            ),
            False,
            "can't find pane",
        ),
    ],
)
def test_kill_pane__argv_and_ignore_missing_semantics(
    monkeypatch, scenario, mock_error, ignore_missing, expect_raise_match
):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        if mock_error is not None:
            raise mock_error
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    if expect_raise_match is not None:
        with pytest.raises(TmuxError, match=expect_raise_match):
            _tmux.kill_pane(target_pane_id="%7", ignore_missing=ignore_missing)
    else:
        result = _tmux.kill_pane(target_pane_id="%7", ignore_missing=ignore_missing)
        assert result is None
        if scenario == "success_argv":
            assert captured[0] == ["tmux", "kill-pane", "-t", "%7"]


class _FakeClock:
    def __init__(self):
        self.now = 0.0
        self.sleep_calls: list[float] = []

    def monotonic(self) -> float:
        return self.now

    def sleep(self, secs: float) -> None:
        self.sleep_calls.append(secs)
        self.now += secs


@pytest.mark.parametrize(
    ("scenario", "pane_exists_seq", "expected_result", "expected_poll_count"),
    [
        ("already_gone_returns_true_immediately", [False], True, 1),
        ("disappears_mid_wait_returns_true", [True, True, False], True, 3),
        ("times_out_returns_false", "always_true", False, 5),
        ("propagates_tmux_error_from_pane_exists", "raise_on_second", None, None),
    ],
)
def test_wait_for_pane_gone__polling_branches(
    monkeypatch, scenario, pane_exists_seq, expected_result, expected_poll_count
):
    clock = _FakeClock()
    monkeypatch.setattr("time.monotonic", clock.monotonic)
    monkeypatch.setattr("time.sleep", clock.sleep)

    poll_calls: list[str] = []
    call_count = {"n": 0}

    if pane_exists_seq == "always_true":

        def fake_pane_exists(self, *, target_pane_id):
            poll_calls.append(target_pane_id)
            return True
    elif pane_exists_seq == "raise_on_second":

        def fake_pane_exists(self, *, target_pane_id):
            call_count["n"] += 1
            if call_count["n"] >= 2:
                raise TmuxError("tmux command failed: server exited unexpectedly")
            return True
    else:
        results = iter(pane_exists_seq)

        def fake_pane_exists(self, *, target_pane_id):
            poll_calls.append(target_pane_id)
            return next(results)

    monkeypatch.setattr(TmuxMultiplexer, "pane_exists", fake_pane_exists)

    if scenario == "propagates_tmux_error_from_pane_exists":
        with pytest.raises(TmuxError, match="server exited unexpectedly"):
            _tmux.wait_for_pane_gone(target_pane_id="%7", timeout=2.0, interval=0.5)
        assert call_count["n"] == 2
    else:
        result = _tmux.wait_for_pane_gone(
            target_pane_id="%7", timeout=2.0, interval=0.5
        )
        assert result is expected_result
        assert len(poll_calls) == expected_poll_count


@pytest.mark.parametrize(
    ("scenario", "command", "expect_raise_match", "expected_literal"),
    [
        ("canonical_argv", "git log -1 --oneline", None, "! git log -1 --oneline"),
        ("strips_surrounding_whitespace", "  git status  ", None, "! git status"),
        ("rejects_newline_in_body", "line1\nline2", "(?i)newline", None),
        ("rejects_trailing_newline", "trailing\n", "(?i)newline", None),
        ("rejects_leading_newline", "\nleading", "(?i)newline", None),
        ("rejects_carriage_return", "carriage\rreturn", "(?i)newline", None),
        ("rejects_crlf", "mixed\r\nCRLF", "(?i)newline", None),
        ("rejects_empty", "", "send_bash_command: command may not be empty", None),
        (
            "rejects_blank_space",
            "   ",
            "send_bash_command: command may not be empty",
            None,
        ),
        ("rejects_tab", "\t", "send_bash_command: command may not be empty", None),
    ],
)
def test_send_bash_command__argv_and_validation(
    monkeypatch, scenario, command, expect_raise_match, expected_literal
):
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    if expect_raise_match is not None:
        with pytest.raises(TmuxError, match=expect_raise_match):
            _tmux.send_bash_command(target_pane_id="%5", command=command)
        assert captured == []
    else:
        _tmux.send_bash_command(target_pane_id="%5", command=command)
        assert len(captured) == 2
        assert captured[0] == [
            "tmux",
            "send-keys",
            "-t",
            "%5",
            "-l",
            expected_literal,
        ]
        assert captured[1] == ["tmux", "send-keys", "-t", "%5", "Enter"]


# --- Esc keystroke safeguard (design 0000090 §1, §2; inverted by 0000092) ------


def test_send_literal_then_enter__esc_first_prepends_escape_and_settle(monkeypatch):
    """esc_first=True prepends an `Escape` keystroke + settle sleep before the
    literal-text/Enter pair — the permission-prompt safeguard (spec §1).

    Full sequence: Escape → _ESC_SETTLE_DELAY → send-keys -l <text> →
    _SUBMIT_DELAY → Enter.
    """
    captured: list[list[str]] = []
    sleeps: list[float] = []

    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )
    monkeypatch.setattr("time.sleep", lambda secs: sleeps.append(secs))

    multiplexer_tmux._send_literal_then_enter(
        target_pane_id="%7", payload="hello", esc_first=True
    )

    assert captured == [
        ["tmux", "send-keys", "-t", "%7", "Escape"],
        ["tmux", "send-keys", "-t", "%7", "-l", "hello"],
        ["tmux", "send-keys", "-t", "%7", "Enter"],
    ]
    # Escape settles first, then the literal/Enter submit delay.
    assert sleeps == [
        multiplexer_tmux._ESC_SETTLE_DELAY,
        multiplexer_tmux._SUBMIT_DELAY,
    ]


def test_send_literal_then_enter__esc_first_default_sends_no_escape(monkeypatch):
    """esc_first defaults to False: no `Escape` keystroke, single submit-delay sleep."""
    captured: list[list[str]] = []
    sleeps: list[float] = []

    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )
    monkeypatch.setattr("time.sleep", lambda secs: sleeps.append(secs))

    multiplexer_tmux._send_literal_then_enter(target_pane_id="%7", payload="hello")

    assert captured == [
        ["tmux", "send-keys", "-t", "%7", "-l", "hello"],
        ["tmux", "send-keys", "-t", "%7", "Enter"],
    ]
    assert all("Escape" not in call for call in captured)
    assert sleeps == [multiplexer_tmux._SUBMIT_DELAY]


def test_send_literal_then_enter__esc_first_forwards_timeout_to_all_calls(monkeypatch):
    """The Escape send-keys call forwards the same timeout as the literal/Enter
    pair (three calls, three timeouts)."""
    captured_timeouts: list[float | None] = []

    def mock_run(args, *, timeout=None, **_kwargs):
        captured_timeouts.append(timeout)
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    monkeypatch.setattr("time.sleep", lambda _secs: None)

    multiplexer_tmux._send_literal_then_enter(
        target_pane_id="%7", payload="hello", timeout=5, esc_first=True
    )
    assert captured_timeouts == [5, 5, 5]


@pytest.mark.parametrize(
    ("scenario", "which_return", "mock_error", "expected_result", "expect_run_called"),
    [
        ("success_returns_true", "/usr/bin/tmux", None, True, True),
        (
            "pane_not_found_returns_false",
            "/usr/bin/tmux",
            TmuxError(
                "tmux command failed: tmux send-keys -t %99\n"
                "stderr: can't find pane: %99"
            ),
            False,
            True,
        ),
        ("tmux_binary_missing_returns_false", None, None, False, False),
        (
            "never_raises_on_tmux_error",
            "/usr/bin/tmux",
            TmuxError("tmux command failed: server exited unexpectedly"),
            False,
            True,
        ),
    ],
)
def test_send_wake_trigger__return_branches_and_argv(
    monkeypatch,
    scenario,
    which_return,
    mock_error,
    expected_result,
    expect_run_called,
):
    """`send_wake_trigger` mirrors `send_poll_trigger`'s return branches but,
    after design 0000092 §2, emits NO leading `Escape`: the monitoring member's
    own pane (the wake target) is never parked on a permission-approval prompt,
    so the safeguard would be a pointless self-interrupt. The success keystroke
    is the plain `-l <payload>` → `Enter` pair."""
    monkeypatch.setattr("shutil.which", lambda _: which_return)
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []

    def mock_run(args, **_kwargs):
        captured.append(list(args))
        if mock_error is not None:
            raise mock_error
        return ""

    monkeypatch.setattr(multiplexer_tmux, "_run", mock_run)
    result = _tmux.send_wake_trigger(
        target_pane_id="%7",
        fleet_id=59,
        agent_id=247,
    )
    assert result is expected_result
    if scenario == "success_returns_true":
        assert len(captured) == 2
        assert captured[0][:5] == ["tmux", "send-keys", "-t", "%7", "-l"]
        assert captured[1] == ["tmux", "send-keys", "-t", "%7", "Enter"]
        assert all("Escape" not in call for call in captured)
    elif not expect_run_called:
        assert captured == []


def test_send_wake_trigger__payload_is_single_line_monitor_nudge(monkeypatch):
    """The wake nudge is a single-line monitoring instruction, NOT the poll
    command keystroke (spec §2 — distinct payload for the monitoring member)."""
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )

    result = _tmux.send_wake_trigger(target_pane_id="%7", fleet_id=59, agent_id=247)
    assert result is True

    # After 0000092 §2 the wake keystroke leads with the literal payload — no
    # leading `Escape` — so the literal call is the first captured send-keys.
    assert all("Escape" not in call for call in captured)
    literal_call = captured[0]
    assert literal_call[:5] == ["tmux", "send-keys", "-t", "%7", "-l"]
    payload = literal_call[5]
    # Single line by design — the wake nudge is a one-line instruction with no
    # embedded newline (per design 0000092 §3 an embedded ``\n`` under ``-l``
    # soft-inserts, so single-line is a shape choice, not a submit-safety guard).
    assert "\n" not in payload
    assert "\r" not in payload
    # A monitoring-member wake nudge (``[monitor]`` provenance tag), not the
    # ``cafleet ... message poll`` command the Director receives.
    assert payload.startswith("[monitor]")
    assert not payload.startswith("cafleet")


@pytest.mark.parametrize(
    ("helper_name", "invoke"),
    [
        ("send_exit", lambda t: t.send_exit(target_pane_id="%7")),
        (
            "send_bash_command",
            lambda t: t.send_bash_command(target_pane_id="%7", command="git status"),
        ),
        (
            "send_freetext_and_submit",
            lambda t: t.send_freetext_and_submit(
                target_pane_id="%7", text="free text answer"
            ),
        ),
    ],
)
def test_esc_first_false_helpers__never_send_escape(monkeypatch, helper_name, invoke):
    """The three opt-out helpers — `send_exit`, `send_bash_command`, and
    `send_freetext_and_submit` — must NOT send an `Esc` first. An `Escape`
    before `/exit` or `! <cmd>` would strip a literal the agent must keep, and
    the freetext helper deliberately ANSWERS a live AskUserQuestion prompt that
    an `Esc` would dismiss (design 0000092 §1 keystroke-helper inventory).

    `send_inline_preview` is deliberately ABSENT here — after 0000092 §1 it now
    leads with `Esc` (asserted in test_tmux_send_inline_preview.py)."""
    monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )

    invoke(_tmux)

    assert captured, "expected the helper to issue send-keys calls"
    assert all("Escape" not in call for call in captured)


def test_send_resume_trigger__removed():
    """`send_resume_trigger` (the blind ordinary-member resume nudge) is removed
    entirely — no caller remains (spec §2, Success Criteria)."""
    assert not hasattr(TmuxMultiplexer, "send_resume_trigger")
