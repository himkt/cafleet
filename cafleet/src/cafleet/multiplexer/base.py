"""Multiplexer Protocol contract for terminal-pane hosting backends.

The :class:`Multiplexer` Protocol defines the surface CAFleet uses to spawn
coding-agent panes, push keystrokes for message delivery, and reap dead
panes. Only the tmux impl is currently shipped
(``cafleet.multiplexer.tmux``), but the Protocol exists so alternative
backends (e.g. a screen-based or in-process fake) can be substituted under
test or in future host environments.
"""

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from cafleet.coding_agent import CODING_AGENTS


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


def _sanitize_wake_name(name: str) -> str:
    """Keep a user-controlled member name inside the one-line wake envelope."""
    return (
        name.replace("\r\n", "⏎")
        .replace("\n", "⏎")
        .replace("\r", "⏎")
        .replace("\t", "⏎")
        .replace("`", "ˋ")
        .replace("$(", "$﹙")
        .replace("|", "│")
    )


def _monitor_wake_payload(due_members: list[dict], director: dict) -> str:
    """Build the byte-identical tmux/herdr monitoring-member wake contract."""
    for target in due_members:
        coding_agent = target.get("coding_agent")
        if coding_agent not in CODING_AGENTS:
            raise ValueError(
                f"member {target.get('member_id')} has invalid "
                f"coding_agent {coding_agent!r}"
            )
    director_agent = director.get("coding_agent")
    if director_agent not in CODING_AGENTS:
        raise ValueError(
            f"Director {director.get('member_id')} has invalid "
            f"coding_agent {director_agent!r}"
        )

    noun = "member" if len(due_members) == 1 else "members"
    due_list = ", ".join(
        f"{'director' if target['is_director'] else 'member'} "
        f"{target['member_id']} "
        f"({_sanitize_wake_name(target['name'])}; "
        f"coding_agent={target['coding_agent']}) "
        f"[{','.join(target['wake_reasons'])}]"
        for target in due_members
    )
    director_id = director["member_id"]
    return (
        f"[monitor] wake: {len(due_members)} {noun} due — {due_list}. "
        f"Capture every named pane and the initial Director {director_id} "
        f"(coding_agent={director_agent}) at --lines 120 --no-ansi --json; "
        "apply each target's coding_agent overlay. Treat unacked only as "
        "context on an already-due member; it never authorizes an action. "
        "Classify capture content only in this precedence: awaiting_user, "
        "unknown, finished, working, stall_candidate. Backend-overlay active "
        "tool, stream, generation, working, ambiguous, or truncated cues force "
        "working; only quiet non-finished content with no prompt or active-work "
        "cue is a stall_candidate. Never classify stalled yourself or remember "
        "hashes in process. Query monitor stall pending before ordinary "
        "observations, including durable disabled or dead reports absent from "
        "this batch. Submit every named ordinary observation through monitor "
        "stall observe with the captured_at and content_sha256 from that same "
        "capture; add --stall-check only for that reason, and use loss-tolerant "
        "unknown without capture fields when unreadable. working is always "
        "non-actionable, including when tagged unacked or byte-identical. Run "
        "cafleet member ping only when observe atomically returns action=ping, "
        "then immediately record ping-result --success or --failure; never retry "
        "a claimed, nudged, or pending episode. A failed ping queues ping_failed "
        "immediately; an unchanged next synchronized capture after a successful "
        "nudge queues unchanged_after_nudge exactly once. Restart from durable "
        "broker state; lifecycle cleanup preserves sticky escalation_pending "
        "and resets non-pending disabled, dead, or placement-pending episodes. "
        "The Director being awaiting_user, working, unknown, disabled, dead, or "
        "unreadable suppresses only the final aggregate, never an eligible "
        "ordinary-member ping. After all ordinary actions, recapture Director "
        f"{director_id} (coding_agent={director_agent}) and submit "
        "--director-gate; only finished or broker-resolved stalled after two "
        "byte-identical captures separated by a full stall interval returns a "
        "token, and Director observation never authorizes ping. With that fresh "
        "token, immediately call monitor report-batch exactly once with "
        "collected finished IDs and no intervening command, even when no new "
        "entry is known; without a token make no Director-targeting call. "
        "report-batch is the sole Director-delivery path; it collects every "
        "durable pending or newly queued escalation plus this wake's finished "
        "IDs, applies one-open backpressure, and retries the same message ID at "
        "most once this wake; a surviving open aggregate leaves new escalations "
        "pending and finished IDs ephemeral. The Director must retrieve an "
        "aggregate with message show --full before acting and ACK it once. Never "
        "call message send, message broadcast, or member prompt this wake; "
        "attach no task text or arbitrary instruction, and take no ordinary "
        "action except the fixed member ping. finished is report-only; the "
        "Director alone judges whether assigned work remains."
    )


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
        self,
        *,
        target_pane_id: str,
        due_members: list[dict],
        director: dict,
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
            director: The Director descriptor carrying ``member_id`` and
                ``coding_agent``.
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

    def send_prompt(
        self, *, target_pane_id: str, text: str, shell: bool = False
    ) -> None:
        """Keystroke ``text`` + Enter into the pane.

        The shell form (``shell=True``) keystrokes ``! <text>`` un-escaped,
        using the leading-``!`` shell-shortcut convention honored by every
        supported coding-agent backend (``claude``, ``codex``, ``opencode``),
        so a single keystroke path serves all backends. The plain form
        (``shell=False``) leads with the ``Esc`` permission-prompt safeguard
        and submits ``text`` as a real user turn.

        Args:
            target_pane_id: Recipient pane id.
            text: Single line of text to dispatch.
            shell: Dispatch ``! <text>`` (shell form) instead of ``text``.
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
