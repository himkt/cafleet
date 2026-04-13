"""Tests for cafleet.tmux module.

All tmux interaction is mocked via monkeypatch.setattr — no test requires
a real tmux server.
"""

import pytest

from cafleet import tmux


# ---------------------------------------------------------------------------
# ensure_tmux_available
# ---------------------------------------------------------------------------


class TestEnsureTmuxAvailable:
    def test_errors_when_tmux_missing(self, monkeypatch):
        """Raises TmuxError when tmux binary is not on PATH."""
        monkeypatch.setattr("shutil.which", lambda _: None)
        monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
        with pytest.raises(tmux.TmuxError, match="tmux binary not found on PATH"):
            tmux.ensure_tmux_available()

    def test_errors_when_tmux_env_unset(self, monkeypatch):
        """Raises TmuxError when TMUX env var is not set."""
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
        monkeypatch.delenv("TMUX", raising=False)
        with pytest.raises(tmux.TmuxError, match="must be run inside a tmux session"):
            tmux.ensure_tmux_available()


# ---------------------------------------------------------------------------
# director_context
# ---------------------------------------------------------------------------


class TestDirectorContext:
    def test_parses_display_message(self, monkeypatch):
        """Parses session|window_id|pane_id from tmux display-message output."""
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
        """Raises TmuxError when TMUX_PANE is not set."""
        monkeypatch.delenv("TMUX_PANE", raising=False)
        with pytest.raises(tmux.TmuxError, match="TMUX_PANE is not set"):
            tmux.director_context()


# ---------------------------------------------------------------------------
# split_window
# ---------------------------------------------------------------------------


class TestSplitWindow:
    def test_returns_captured_pane_id(self, monkeypatch):
        """Returns the pane_id captured from tmux split-window -P -F output."""

        def mock_run(args):
            assert args[0:3] == ["tmux", "split-window", "-t"]
            assert "@3" in args
            assert "-P" in args
            assert "-F" in args
            assert "#{pane_id}" in args
            # Verify env flags are present
            assert "-e" in args
            return "%7\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        pane_id = tmux.split_window(
            target_window_id="@3",
            env={
                "CAFLEET_URL": "http://localhost:8000",
                "CAFLEET_SESSION_ID": "key123",
            },
            command=["claude", "Hello world"],
        )
        assert pane_id == "%7"

    def test_command_appended_directly_to_args(self, monkeypatch):
        """The command list is appended directly to tmux args — no wrapping."""
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
        # The command elements should appear at the end of the args list
        assert captured_args[-4:] == ["my-binary", "--flag", "value", "prompt text"]

    def test_claude_style_command(self, monkeypatch):
        """Claude-style command: [\"claude\", \"prompt\"] — backward compatible."""
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
                "Load Skill(cafleet). Your agent_id is $CAFLEET_AGENT_ID.",
            ],
        )
        assert captured_args[-2] == "claude"
        assert "Load Skill(cafleet)" in captured_args[-1]

    def test_codex_style_command(self, monkeypatch):
        """Codex-style command: [\"codex\", \"--approval-mode\", \"auto-edit\", \"prompt\"]."""
        captured_args = []

        def mock_run(args):
            captured_args.extend(args)
            return "%10\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        tmux.split_window(
            target_window_id="@3",
            env={},
            command=["codex", "--approval-mode", "auto-edit", "Do the task"],
        )
        assert captured_args[-4:] == [
            "codex",
            "--approval-mode",
            "auto-edit",
            "Do the task",
        ]

    def test_env_vars_forwarded_as_flags(self, monkeypatch):
        """Environment variables are forwarded as -e KEY=VAL flags before the command."""
        captured_args = []

        def mock_run(args):
            captured_args.extend(args)
            return "%11\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        tmux.split_window(
            target_window_id="@3",
            env={
                "CAFLEET_URL": "http://localhost:8000",
                "CAFLEET_SESSION_ID": "sess-001",
                "CAFLEET_AGENT_ID": "agent-001",
            },
            command=["claude", "prompt"],
        )
        # Each env var should appear as -e KEY=VAL
        assert "-e" in captured_args
        env_pairs = []
        for i, a in enumerate(captured_args):
            if a == "-e" and i + 1 < len(captured_args):
                env_pairs.append(captured_args[i + 1])
        assert "CAFLEET_URL=http://localhost:8000" in env_pairs
        assert "CAFLEET_SESSION_ID=sess-001" in env_pairs
        assert "CAFLEET_AGENT_ID=agent-001" in env_pairs


# ---------------------------------------------------------------------------
# send_exit
# ---------------------------------------------------------------------------


class TestSendExit:
    def test_raises_tmuxerror_on_failure(self, monkeypatch):
        """Raises TmuxError when tmux send-keys fails (non-pane-gone error)."""

        def mock_run(args):
            raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="server exited unexpectedly"):
            tmux.send_exit(target_pane_id="%7")

    def test_ignore_missing_swallows_pane_gone(self, monkeypatch):
        """With ignore_missing=True, swallows 'can't find pane' errors."""

        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux send-keys -t %99 /exit Enter\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        # Should not raise
        tmux.send_exit(target_pane_id="%99", ignore_missing=True)

    def test_ignore_missing_does_not_swallow_other_errors(self, monkeypatch):
        """With ignore_missing=True, still raises non-pane-gone TmuxError."""

        def mock_run(args):
            raise tmux.TmuxError("tmux command failed: server crashed")

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="server crashed"):
            tmux.send_exit(target_pane_id="%7", ignore_missing=True)


# ---------------------------------------------------------------------------
# capture_pane
# ---------------------------------------------------------------------------


class TestCapturPane:
    def test_invokes_correct_args(self, monkeypatch):
        """Verifies argv is exactly the expected tmux capture-pane command."""
        captured_args = []

        def mock_run(args):
            captured_args.extend(args)
            return "line 1\nline 2\n"

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.capture_pane(target_pane_id="%7", lines=80)
        assert captured_args == ["tmux", "capture-pane", "-p", "-t", "%7", "-S", "-80"]
        assert result == "line 1\nline 2\n"

    def test_raises_on_missing_pane(self, monkeypatch):
        """Raises TmuxError when tmux reports 'can't find pane' — never swallowed."""

        def mock_run(args):
            raise tmux.TmuxError(
                "tmux command failed: tmux capture-pane -p -t %99 -S -80\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        with pytest.raises(tmux.TmuxError, match="can't find pane"):
            tmux.capture_pane(target_pane_id="%99", lines=80)

    def test_rejects_non_positive_lines(self, monkeypatch):
        """Guard fires for lines=0 and lines=-1."""
        with pytest.raises(tmux.TmuxError, match="lines must be positive, got 0"):
            tmux.capture_pane(target_pane_id="%7", lines=0)
        with pytest.raises(tmux.TmuxError, match="lines must be positive, got -1"):
            tmux.capture_pane(target_pane_id="%7", lines=-1)


# ---------------------------------------------------------------------------
# send_poll_trigger
# ---------------------------------------------------------------------------


class TestSendPollTrigger:
    def test_success_returns_true(self, monkeypatch):
        """Returns True when tmux send-keys succeeds."""
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")
        captured_args = []

        def mock_run(args, **kwargs):
            captured_args.extend(args)
            return ""

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.send_poll_trigger(target_pane_id="%7", agent_id="agent-001")
        assert result is True
        assert captured_args == [
            "tmux",
            "send-keys",
            "-t",
            "%7",
            "cafleet poll --agent-id agent-001",
            "Enter",
        ]

    def test_pane_not_found_returns_false(self, monkeypatch):
        """Returns False when tmux reports the pane no longer exists."""
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

        def mock_run(args, **kwargs):
            raise tmux.TmuxError(
                "tmux command failed: tmux send-keys -t %99\n"
                "stderr: can't find pane: %99"
            )

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.send_poll_trigger(target_pane_id="%99", agent_id="agent-001")
        assert result is False

    def test_tmux_binary_missing_returns_false(self, monkeypatch):
        """Returns False when tmux binary is not on PATH — never calls _run."""
        monkeypatch.setattr("shutil.which", lambda _: None)
        run_called = False

        def mock_run(args):
            nonlocal run_called
            run_called = True
            return ""

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.send_poll_trigger(target_pane_id="%7", agent_id="agent-001")
        assert result is False
        assert not run_called, "_run should not be called when tmux is not on PATH"

    def test_never_raises_on_tmux_error(self, monkeypatch):
        """Never raises — catches TmuxError internally and returns False."""
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/tmux")

        def mock_run(args, **kwargs):
            raise tmux.TmuxError("tmux command failed: server exited unexpectedly")

        monkeypatch.setattr(tmux, "_run", mock_run)
        result = tmux.send_poll_trigger(target_pane_id="%7", agent_id="agent-001")
        assert result is False
