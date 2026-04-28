"""Tests for ``cafleet.tmux`` (all subprocesses mocked)."""

import pytest

from cafleet import tmux


class TestEnsureTmuxAvailable:
    def test_errors_when_tmux_missing(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: None)
        monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
        with pytest.raises(tmux.TmuxError, match="tmux binary not found on PATH"):
            tmux.ensure_tmux_available()

    def test_errors_when_tmux_env_unset(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
        monkeypatch.delenv("TMUX", raising=False)
        with pytest.raises(tmux.TmuxError, match="must be run inside a tmux session"):
            tmux.ensure_tmux_available()


class TestDirectorContext:
    def test_parses_display_message(self, monkeypatch):
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

    def test_errors_when_tmux_pane_unset(self, monkeypatch):
        monkeypatch.delenv("TMUX_PANE", raising=False)
        with pytest.raises(tmux.TmuxError, match="TMUX_PANE is not set"):
            tmux.director_context()


class TestSplitWindow:
    def test_returns_captured_pane_id(self, monkeypatch):
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

    def test_command_appended_directly_to_args(self, monkeypatch):
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

    def test_claude_style_command(self, monkeypatch):
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
                    "Load Skill(cafleet). Your agent_id is "
                    "7ba91234-5678-90ab-cdef-112233445566."
                ),
            ],
        )
        assert captured_args[-2] == "claude"
        assert "Load Skill(cafleet)" in captured_args[-1]

    def test_env_vars_forwarded_as_flags(self, monkeypatch):
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

    def test_empty_env_emits_no_e_flags(self, monkeypatch):
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


class TestSendExit:
    def test_raises_tmuxerror_on_failure(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
            tmux.send_exit(target_pane_id="%7")

    def test_ignore_missing_swallows_pane_gone(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux send-keys -t %99 /exit Enter\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        tmux.send_exit(target_pane_id="%99", ignore_missing=True)

    def test_ignore_missing_does_not_swallow_other_errors(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError("tmux command failed: server crashed")

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="server crashed"):
            tmux.send_exit(target_pane_id="%7", ignore_missing=True)


class TestCapturePane:
    def test_invokes_correct_args(self, monkeypatch):
        captured_args = []

        def mock_run(args):
            captured_args.extend(args)
            return "line 1\nline 2\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.capture_pane(target_pane_id="%7", lines=80)
        assert captured_args == ["tmux", "capture-pane", "-p", "-t", "%7", "-S", "-80"]
        assert result == "line 1\nline 2\n"

    def test_raises_on_missing_pane(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux capture-pane -p -t %99 -S -80\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="can't find pane"):
            tmux.capture_pane(target_pane_id="%99", lines=80)

    def test_rejects_non_positive_lines(self, monkeypatch):
        with pytest.raises(tmux.TmuxError, match="lines must be positive, got 0"):
            tmux.capture_pane(target_pane_id="%7", lines=0)
        with pytest.raises(tmux.TmuxError, match="lines must be positive, got -1"):
            tmux.capture_pane(target_pane_id="%7", lines=-1)


class TestSendPollTrigger:
    def test_success_returns_true(self, monkeypatch):
        """``--session-id`` is a root-group global option and MUST come
        before the subcommand; ``--agent-id`` is a per-subcommand option
        and MUST come after ``message poll``. This ordering is what click's
        parser actually accepts and is the literal string
        ``permissions.allow`` entries need to match.
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

    def test_pane_not_found_returns_false(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

        def mock_run(args, **kwargs):
            raise tmux.TmuxError(
                "tmux command failed: tmux send-keys -t %99\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.send_poll_trigger(
            target_pane_id="%99",
            session_id="sess-001",
            agent_id="agent-001",
        )
        assert result is False

    def test_tmux_binary_missing_returns_false(self, monkeypatch):
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

    def test_never_raises_on_tmux_error(self, monkeypatch):
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


class TestSendPollTriggerKeystroke:
    """Regression guard against any future revert of the round-6 keystroke
    rename (design 0000034 §14). The pushed keystroke MUST contain the
    literal ``message poll`` — bare ``poll`` no longer parses, so a stale
    keystroke would land in the member's pane and error out with
    ``Error: No such command 'poll'``.
    """

    def test_keystroke_contains_message_poll(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
        captured_args = []

        def mock_run(args, **kwargs):
            captured_args.extend(args)
            return ""

        monkeypatch.setattr(tmux, "_run", mock_run)
        tmux.send_poll_trigger(
            target_pane_id="%0",
            session_id="550e8400-e29b-41d4-a716-446655440000",
            agent_id="7ba91234-5678-90ab-cdef-112233445566",
        )
        # The keystroke literal is the 6th element of the captured argv:
        # ["tmux", "send-keys", "-t", <pane>, "-l", <keystroke>, ...].
        keystroke = captured_args[5]
        assert "message poll" in keystroke, (
            f"keystroke must contain 'message poll', got: {keystroke!r}"
        )
        # Belt-and-suspenders: a bare ' poll ' with no preceding 'message'
        # would slip through ``"message poll" in keystroke`` only if the
        # literal contained both, so this is just a sanity check.
        assert " poll --agent-id" in keystroke


class TestPaneExists:
    def test_returns_true_when_pane_present_in_list(self, monkeypatch):
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

    def test_returns_false_when_pane_absent(self, monkeypatch):
        def mock_run(args):
            return "%0\n%3\n%9\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        assert tmux.pane_exists(target_pane_id="%7") is False

    def test_propagates_unrelated_tmux_error(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux list-panes -a -F #{pane_id}\n"
                "stderr: no server running on /tmp/tmux-1000/default"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="no server running"):
            tmux.pane_exists(target_pane_id="%7")


class TestKillPane:
    def test_invokes_correct_args(self, monkeypatch):
        captured_args = []

        def mock_run(args):
            captured_args.extend(args)
            return ""

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.kill_pane(target_pane_id="%7")
        assert result is None
        assert captured_args == ["tmux", "kill-pane", "-t", "%7"]

    def test_ignore_missing_swallows_pane_gone(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux kill-pane -t %99\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        tmux.kill_pane(target_pane_id="%99", ignore_missing=True)

    def test_ignore_missing_does_not_swallow_other_errors(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux kill-pane -t %7\n"
                "stderr: server exited unexpectedly"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
            tmux.kill_pane(target_pane_id="%7", ignore_missing=True)

    def test_default_raises_even_for_pane_gone(self, monkeypatch):
        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux kill-pane -t %99\n"
                "stderr: can't find pane: %99"
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


class TestWaitForPaneGone:
    def test_returns_true_immediately_when_pane_already_gone(self, monkeypatch):
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

    def test_returns_true_when_pane_disappears_mid_wait(self, monkeypatch):
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

    def test_returns_false_after_timeout(self, monkeypatch):
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

    def test_propagates_tmux_error_from_pane_exists(self, monkeypatch):
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
