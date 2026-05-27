"""Multiplexer Protocol contract for terminal-pane hosting backends.

The :class:`Multiplexer` Protocol defines the surface CAFleet uses to spawn
coding-agent panes, push keystrokes for message delivery, and reap dead
panes. Only the tmux impl is currently shipped
(``cafleet.multiplexer.tmux``), but the Protocol exists so alternative
backends (e.g. a screen-based or in-process fake) can be substituted under
test or in future host environments.
"""

import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, runtime_checkable


@dataclass(frozen=True)
class MultiplexerContext:
    """Resolved pane identity, returned by ``Multiplexer.context_discovery()``.

    Attributes:
        session: Multiplexer session name (e.g. tmux ``session_name``).
        window_id: Multiplexer window id (e.g. tmux ``@N``).
        pane_id: Multiplexer pane id (e.g. tmux ``%N``).
    """

    session: str
    window_id: str
    pane_id: str


@runtime_checkable
class Multiplexer(Protocol):
    """Terminal multiplexer that hosts coding-agent panes."""

    @property
    def name(self) -> str:
        """Registry key (e.g. ``"tmux"``)."""
        ...

    def ensure_available(self) -> None:
        """Raise if the multiplexer binary is missing or the runtime context
        is not a live multiplexer session.

        Raises:
            Exception: A backend-specific exception when the precondition
                fails (e.g. :class:`TmuxError` from
                :class:`~cafleet.multiplexer.tmux.TmuxMultiplexer`).
        """
        ...

    def context_discovery(self) -> MultiplexerContext:
        """Resolve the calling shell's pane identity.

        Returns:
            :class:`MultiplexerContext` carrying ``session``, ``window_id``,
            and ``pane_id`` for the pane the caller is running in.
        """
        ...

    def split_window(
        self,
        *,
        target_window_id: str,
        env: dict[str, str],
        command: list[str],
    ) -> str:
        """Spawn a new pane inside ``target_window_id`` running ``command``.

        Args:
            target_window_id: Multiplexer window id to split.
            env: Extra environment variables exported into the new pane.
            command: argv list executed in the new pane.

        Returns:
            The new pane's id (e.g. tmux ``%N``).
        """
        ...

    def kill_pane(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        """Forcibly close ``target_pane_id``.

        Args:
            target_pane_id: Pane id to kill.
            ignore_missing: If True, do not raise when the pane is already
                gone — useful for idempotent teardown paths.
        """
        ...

    def pane_exists(self, *, target_pane_id: str) -> bool:
        """Return True iff ``target_pane_id`` is currently alive."""
        ...

    def wait_for_pane_gone(
        self,
        *,
        target_pane_id: str,
        timeout: float = 15.0,
        interval: float = 0.5,
    ) -> bool:
        """Block until ``target_pane_id`` disappears or ``timeout`` elapses.

        Args:
            target_pane_id: Pane id to watch.
            timeout: Maximum seconds to wait.
            interval: Poll interval in seconds.

        Returns:
            ``True`` if the pane disappeared before ``timeout``, ``False``
            on timeout.
        """
        ...

    def send_exit(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        """Push ``/exit`` + Enter into the pane to gracefully terminate the
        coding-agent process.

        Args:
            target_pane_id: Pane id to signal.
            ignore_missing: If True, do not raise when the pane is already
                gone.
        """
        ...

    def send_poll_trigger(
        self, *, target_pane_id: str, session_id: str, agent_id: str
    ) -> bool:
        """Keystroke a ``cafleet message poll`` shortcut into the pane.

        Args:
            target_pane_id: Pane id of the agent to nudge.
            session_id: Session UUID embedded in the keystroked command.
            agent_id: Recipient agent UUID embedded in the keystroked
                command.

        Returns:
            ``True`` if the keystroke landed; ``False`` if the pane is dead
            or the multiplexer is unreachable.
        """
        ...

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        task_id_8: str,
        sender_8: str,
        ts: str,
        text: str,
    ) -> bool:
        """Keystroke a 2-line message preview into the recipient's pane.

        The first line carries an ``[cafleet msg <task8> from <sender8>
        <ts>]`` header; the second line carries the (possibly truncated)
        body. The recipient's coding agent processes the keystrokes as a
        fresh user-turn input.

        Args:
            target_pane_id: Recipient pane id.
            task_id_8: First 8 chars of the task UUID.
            sender_8: First 8 chars of the sender's agent UUID.
            ts: Status timestamp string included in the header.
            text: Message body (caller is responsible for truncation).

        Returns:
            ``True`` if the keystroke landed; ``False`` otherwise.
        """
        ...

    def send_choice_key(self, *, target_pane_id: str, digit: int) -> None:
        """Keystroke a single digit (no Enter) into the pane.

        Used by Director-driven AskUserQuestion answers to select one of the
        first three options. The tmux backend rejects digits outside
        ``{1, 2, 3}``. Enter is not sent because the AskUserQuestion frame
        commits the selection on digit press.

        Args:
            target_pane_id: Recipient pane id.
            digit: Decimal digit ``1``, ``2``, or ``3``.
        """
        ...

    def send_freetext_and_submit(self, *, target_pane_id: str, text: str) -> None:
        """Keystroke ``4`` + literal ``text`` + Enter into the pane.

        Used by Director-driven AskUserQuestion freetext answers. The leading
        ``4`` selects the "Type something" option in the AskUserQuestion
        frame; the literal text is then typed in and Enter submits it. The
        tmux backend rejects newlines in ``text`` so each call is exactly
        one prompt submission.

        Args:
            target_pane_id: Recipient pane id.
            text: Literal text to type into the pane (no newlines).
        """
        ...

    def send_bash_command(self, *, target_pane_id: str, command: str) -> None:
        """Keystroke ``! <command>`` + Enter into the pane.

        Uses the leading-``!`` shell-shortcut convention honored by every
        supported coding-agent backend (``claude``, ``codex``, ``opencode``),
        so a single keystroke path serves all backends.

        Args:
            target_pane_id: Recipient pane id.
            command: Shell command to dispatch on the member's behalf.
        """
        ...

    def capture_pane(self, *, target_pane_id: str, lines: int = 30) -> str:
        """Return the last ``lines`` of the pane's visible buffer as text.

        Args:
            target_pane_id: Pane id to capture from.
            lines: Number of trailing lines to capture (default 30).

        Returns:
            Captured pane text, newline-joined.
        """
        ...


def poll_until_pane_gone(
    pane_exists_fn: Callable[[], bool],
    *,
    timeout: float,
    interval: float,
) -> bool:
    """Generic poll-until-False helper for any Multiplexer's ``wait_for_pane_gone``.

    Args:
        pane_exists_fn: Zero-arg callable returning whether the pane is
            still alive.
        timeout: Maximum seconds to wait.
        interval: Poll interval in seconds.

    Returns:
        ``True`` if ``pane_exists_fn`` returned False before ``timeout``,
        ``False`` on timeout.
    """
    deadline = time.monotonic() + timeout
    while True:
        if not pane_exists_fn():
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
