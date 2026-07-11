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


class MultiplexerError(Exception):
    """Base for terminal-multiplexer backend failures.

    Backend-specific subclasses (:class:`~cafleet.multiplexer.tmux.TmuxError`,
    :class:`~cafleet.multiplexer.herdr.HerdrError`) let CLI boundaries catch a
    single ``MultiplexerError`` while each backend keeps its own message text.
    """


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
        reference: MultiplexerContext,
        env: dict[str, str],
        command: list[str],
    ) -> str:
        """Spawn a new pane from ``reference`` running ``command``.

        Takes the full reference context because backends split different
        primitives: tmux splits ``reference.window_id`` (a window); herdr splits
        ``reference.pane_id`` (a pane).

        Args:
            reference: Resolved pane identity to split from.
            env: Extra environment variables exported into the new pane.
            command: argv list executed in the new pane.

        Returns:
            The new pane's id (e.g. tmux ``%N``, herdr ``w1:p1``).
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

    def list_pane_ids(self) -> set[str]:
        """Return the set of every live pane id across the multiplexer server.

        The per-tick liveness query for ``cafleet monitor``: one call resolves
        pane liveness for every member in a tick (e.g. ``tmux list-panes -a``).

        Returns:
            A set of pane ids (e.g. ``{"%0", "%7"}``).
        """
        ...

    def wait_for_pane_gone(
        self,
        *,
        target_pane_id: str,
        timeout: float = 15.0,
        interval: float = 0.5,
    ) -> bool:
        """Block until ``target_pane_id`` disappears or ``timeout`` elapses.

        A backend whose pane outlives its coding agent may realize this as
        "wait for the agent to exit, then reap the pane" rather than a pure
        read-only poll.

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
        self, *, target_pane_id: str, fleet_id: int, member_id: int
    ) -> bool:
        """Keystroke a ``cafleet message poll`` shortcut into the pane.

        Args:
            target_pane_id: Pane id of the member to nudge.
            fleet_id: Fleet id embedded in the keystroked command.
            member_id: Recipient member id embedded in the keystroked
                command.

        Returns:
            ``True`` if the keystroke landed; ``False`` if the pane is dead
            or the multiplexer is unreachable.
        """
        ...

    def send_wake_trigger(
        self, *, target_pane_id: str, due_members: list[dict], director_member_id: int
    ) -> bool:
        """Keystroke a single-line *wake nudge* into the monitoring member's pane.

        Unlike :meth:`send_poll_trigger` (a bare ``cafleet … message poll`` the
        Director receives), this carries an instruction to run the monitoring
        member's capture-classify-reengage routine. The nudge **names** each
        freshly-due member (``<role> <id> (<name>)``) and the Director id as the
        standing inspect-and-re-engage target, so the monitoring member inspects
        exactly those panes plus the Director. The wake nudge does **not** lead
        with an ``Esc`` safeguard — only :meth:`send_poll_trigger` (via
        ``cafleet member ping``) and :meth:`send_inline_preview` do, because their
        targets may be parked on a permission-approval prompt; the monitoring
        member's own pane never is. The payload carries no backtick and no
        ``$(…)`` command substitution, so it is safe in the monitoring member's
        coding-agent input box.

        Args:
            target_pane_id: Pane id of the monitoring member to nudge.
            due_members: The freshly-due watched members to name, each a target
                dict carrying ``member_id``, ``name``, and ``is_director``.
            director_member_id: The Director's member id, named in every payload
                as the standing inspect-and-re-engage target.

        Returns:
            ``True`` if the keystroke landed; ``False`` if the pane is dead or
            the multiplexer is unreachable.
        """
        ...

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        message_id: int,
        sender_id: int,
        ts: str,
        text: str,
    ) -> bool:
        """Keystroke a 2-line message preview into the recipient's pane.

        The first line carries an ``[cafleet msg <message_id> from <sender_id>
        <ts>]`` header; the second line carries the (possibly truncated)
        body. The recipient's coding agent processes the keystrokes as a
        fresh user-turn input.

        Args:
            target_pane_id: Recipient pane id.
            message_id: Message id of the delivered message.
            sender_id: Sender's member id.
            ts: Status timestamp string included in the header.
            text: Message body (caller is responsible for truncation).

        Returns:
            ``True`` if the keystroke landed; ``False`` otherwise.
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

    def capture_pane(self, *, target_pane_id: str, lines: int = 20) -> str:
        """Return the last ``lines`` of the pane's visible buffer as text.

        Args:
            target_pane_id: Pane id to capture from.
            lines: Number of trailing lines to capture (default 20).

        Returns:
            Captured pane text, newline-joined.
        """
        ...


@runtime_checkable
class AgentStateAware(Protocol):
    """Optional capability for backends that track native agent lifecycle state.

    Kept **off** the base :class:`Multiplexer` Protocol so a backend without
    native state (tmux) implements nothing new. Only a backend whose
    multiplexer natively detects an agent's state (herdr) implements this;
    consumers gate on ``isinstance(mux, AgentStateAware)``.
    """

    def agent_status(self, *, target_pane_id: str) -> str | None:
        """Return the pane's current native agent state.

        Args:
            target_pane_id: Pane id to read.

        Returns:
            One of ``working``/``blocked``/``done``/``idle``/``unknown``, or
            ``None`` when no agent is detected in the pane.
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
