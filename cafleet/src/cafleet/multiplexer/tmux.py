import contextlib
import os
import shutil
import subprocess
import time

from cafleet.multiplexer.base import (
    MultiplexerContext,
    MultiplexerError,
    _monitor_wake_payload,
)


class TmuxError(MultiplexerError):
    """Raised when a tmux subprocess fails or tmux is not reachable."""


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


# Sleep between literal-text ``send-keys -l`` and following ``send-keys Enter``
# so the payload submits instead of the Enter being absorbed: codex classifies
# the fast-typed payload as a paste and suppresses an Enter arriving within its
# ~120ms post-paste window, and opencode's slash-command autocomplete popup
# needs to settle before submit. 1.0s clears the codex window with margin for a
# busy event loop reading the burst late (applied unconditionally to keep the
# helpers backend-agnostic).
_SUBMIT_DELAY = 1.0

# Sleep after a leading ``send-keys Escape`` (the opt-in ``esc_first`` safeguard)
# so the agent dismisses a pending permission-approval prompt and the pane
# settles before the literal text is typed.
_ESC_SETTLE_DELAY = 0.1


def _run(args: list[str], *, timeout: float | None = None) -> str:
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, check=True, timeout=timeout
        )
    except FileNotFoundError as exc:
        raise TmuxError(f"tmux binary not found: {exc}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TmuxError(
            f"tmux command timed out after {exc.timeout}s: {' '.join(args)}"
        ) from exc
    except subprocess.CalledProcessError as exc:
        raise TmuxError(
            f"tmux command failed: {' '.join(args)}\nstderr: {exc.stderr.strip()}"
        ) from exc
    return result.stdout


def _run_tolerating_pane_gone(
    args: list[str], *, ignore_missing: bool, timeout: float | None = None
) -> None:
    try:
        _run(args, timeout=timeout)
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


def _send_literal_then_enter(
    *,
    target_pane_id: str,
    payload: str,
    timeout: float | None = None,
    ignore_missing: bool = False,
    esc_first: bool = False,
) -> None:
    if esc_first:
        # Permission-prompt safeguard: a leading ``Escape`` dismisses a pending
        # permission-approval prompt so the trailing ``Enter`` below cannot
        # blindly confirm it. Applied wherever the target pane MAY be parked on
        # such a prompt: ``send_poll_trigger`` (``cafleet member ping`` — a
        # member that might be on a prompt), ``send_inline_preview`` (every
        # inbound ``message send`` / ``broadcast`` — any recipient, the Director
        # included), and ``send_prompt``'s plain form (a submitted user turn).
        # It is NOT applied where a leading ``Esc`` would mis-fire —
        # ``send_exit`` (``Esc`` before ``/exit``) and ``send_prompt``'s shell
        # form (``Esc`` before ``! <cmd>``) — nor where the pane is never on a
        # prompt: ``send_wake_trigger`` (the monitoring member's own read-only
        # pane).
        _run_tolerating_pane_gone(
            ["tmux", "send-keys", "-t", target_pane_id, "Escape"],
            ignore_missing=ignore_missing,
            timeout=timeout,
        )
        time.sleep(_ESC_SETTLE_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "-l", payload],
        ignore_missing=ignore_missing,
        timeout=timeout,
    )
    time.sleep(_SUBMIT_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
        ignore_missing=ignore_missing,
        timeout=timeout,
    )


def _best_effort_send(
    *, target_pane_id: str, payload: str, esc_first: bool = False
) -> bool:
    """Best-effort keystroke: skip when tmux is absent, map ``TmuxError`` to ``False``.

    Shared by ``send_poll_trigger`` / ``send_wake_trigger`` / ``send_inline_preview``;
    each supplies its own ``payload`` and ``esc_first`` and keeps the ``timeout=5``.
    """
    if shutil.which("tmux") is None:
        return False
    try:
        _send_literal_then_enter(
            target_pane_id=target_pane_id,
            payload=payload,
            timeout=5,
            esc_first=esc_first,
        )
    except TmuxError:
        return False
    return True


class TmuxMultiplexer:
    name = "tmux"

    def ensure_available(self) -> None:
        if shutil.which("tmux") is None:
            raise TmuxError("tmux binary not found on PATH")
        if not os.environ.get("TMUX"):
            raise TmuxError("cafleet member commands must be run inside a tmux session")

    def context_discovery(self) -> MultiplexerContext:
        """Resolve the tmux session/window/pane of the calling pane.

        Anchored on ``$TMUX_PANE`` so it works regardless of which window the
        user is currently focused on.
        """
        tmux_pane = os.environ.get("TMUX_PANE")
        if not tmux_pane:
            raise TmuxError("TMUX_PANE is not set; not running inside a tmux pane")
        out = _run(
            [
                "tmux",
                "display-message",
                "-p",
                "-t",
                tmux_pane,
                "#{session_name}|#{window_id}|#{pane_id}",
            ]
        )
        try:
            session, window_id, pane_id = out.strip().split("|", 2)
        except ValueError as exc:
            raise TmuxError(f"unexpected tmux display-message output: {out!r}") from exc
        return MultiplexerContext(session=session, window_id=window_id, pane_id=pane_id)

    def split_window(
        self,
        *,
        reference: MultiplexerContext,
        env: dict[str, str],
        command: list[str],
    ) -> str:
        """Split ``reference.window_id`` with ``command`` and return the new pane id.

        Always invoked with ``-d`` so the new pane is not made active and the
        calling client's active window is not switched. This behavior is
        unconditional — there is no opt-out parameter.
        """
        target_window_id = reference.window_id
        args = [
            "tmux",
            "split-window",
            "-t",
            target_window_id,
            "-P",
            "-F",
            "#{pane_id}",
            "-d",
        ]
        for k, v in env.items():
            args += ["-e", f"{k}={v}"]
        args += command
        pane_id = _run(args).strip()
        # Rebalance the window layout post-split. Wrapped in suppress(TmuxError)
        # so a layout failure does not break the spawn — the new pane is already
        # live and tmux auto-fits remaining panes if the explicit rebalance fails.
        with contextlib.suppress(TmuxError):
            self.select_layout(target_window_id=target_window_id)
        return pane_id

    def select_layout(
        self, *, target_window_id: str, layout: str = "main-vertical"
    ) -> None:
        _run(["tmux", "select-layout", "-t", target_window_id, layout])

    def send_exit(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        """Send ``/exit`` + Enter, swallowing pane-gone errors when requested."""
        _send_literal_then_enter(
            target_pane_id=target_pane_id,
            payload="/exit",
            ignore_missing=ignore_missing,
        )

    def send_poll_trigger(
        self, *, target_pane_id: str, fleet_id: int, member_id: int
    ) -> bool:
        """Best-effort Esc-safeguarded ``cafleet ... message poll`` keystroke.

        ``esc_first=True`` leads with ``Escape`` so a pane on a pending
        permission-approval prompt dismisses it rather than confirming it with
        the trailing ``Enter``. The Director's manual ``cafleet member ping``
        reuses this helper, inheriting the safeguard for free.
        """
        payload = f"cafleet message poll --fleet-id {fleet_id} --member-id {member_id}"
        return _best_effort_send(
            target_pane_id=target_pane_id, payload=payload, esc_first=True
        )

    def send_wake_trigger(
        self,
        *,
        target_pane_id: str,
        due_members: list[dict],
        director: dict,
    ) -> bool:
        """Best-effort wake nudge for the monitoring member's pane.

        Carries a single-line instruction that **names** each due member
        (``<role> <id> (<name>) [<reasons>]``) and the Director id, directing the
        monitoring member to classify each pane on the five-state taxonomy and
        re-engage the Director — distinct from the poll command
        ``send_poll_trigger`` carries (now used only by ``cafleet member ping``).
        This is the sole keystroke the loop fires — it wakes only the monitoring
        member. No leading ``Escape``: the target is the monitoring member's own
        pane, which runs a read-only routine under ``dontAsk`` and is never
        parked on a permission-approval prompt, so an ``Esc`` would merely
        self-interrupt an in-progress routine. User-controlled member names are
        sanitized (CR/LF/tab → U+23CE) to preserve the single-line shape, and the
        payload carries no backtick, no ``$(…)`` command substitution, and no
        pipe, so it is safe in the monitoring member's coding-agent input box. The
        payload is byte-identical to the herdr backend's.
        """
        payload = _monitor_wake_payload(due_members, director)
        return _best_effort_send(target_pane_id=target_pane_id, payload=payload)

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        message_id: int,
        sender_id: int,
        ts: str,
        text: str,
    ) -> bool:
        """Best-effort 2-line inline preview keystroke for the recipient's pane."""
        if shutil.which("tmux") is None:
            return False
        # The single ``\n`` in the f-string below is the envelope/body separator
        # and is intentionally kept: under tmux ``send-keys -l`` an embedded
        # newline is delivered as a soft line break inside one keystroke
        # sequence — it does NOT fragment delivery into a second submit, so the
        # whole 2-line payload arrives as a single recipient turn submitted by
        # the one trailing ``Enter`` below. (Asserted by
        # ``test_tmux_send_inline_preview.py::test_send_inline_preview__newline_soft_insert_single_submit``,
        # which pins the one-``-l``/one-``Enter`` 2-line contract.) The leading
        # ``Escape`` (``esc_first``) carries the permission-prompt safety
        # guarantee independently. The body sanitization to U+23CE keeps a
        # multi-line user body from visually breaking the 2-line framing; it is
        # cosmetic, not submit-safety.
        sanitized_text = text.replace("\r\n", "⏎").replace("\n", "⏎").replace("\r", "⏎")
        payload = f"[cafleet msg {message_id} from {sender_id} {ts}]\n{sanitized_text}"
        return _best_effort_send(
            target_pane_id=target_pane_id, payload=payload, esc_first=True
        )

    def send_prompt(
        self, *, target_pane_id: str, text: str, shell: bool = False
    ) -> None:
        """Send ``! <text>`` + Enter via the coding agent's ``!`` shortcut when
        ``shell``; else lead with the ``Esc`` safeguard and submit ``text`` as a
        real user turn."""
        stripped = text.strip()
        if not stripped:
            raise TmuxError("send_prompt: text may not be empty")
        if "\n" in text or "\r" in text:
            raise TmuxError("send_prompt: text may not contain newlines")
        payload = f"! {stripped}" if shell else stripped
        _send_literal_then_enter(
            target_pane_id=target_pane_id, payload=payload, esc_first=not shell
        )

    def capture_pane(self, *, target_pane_id: str, lines: int = 20) -> str:
        """Return the last ``lines`` lines of the pane's terminal buffer.

        Default is 20 to keep per-tick Director monitoring captures from
        dominating token cost. Explicit ``lines`` overrides apply unchanged.
        """
        if lines <= 0:
            raise TmuxError(f"capture_pane: lines must be positive, got {lines}")
        raw = _run(
            ["tmux", "capture-pane", "-p", "-t", target_pane_id, "-S", f"-{lines}"]
        )
        # -S gives tmux a start hint but can return more lines than requested
        # (e.g. wrapped lines, partial scrollback); enforce the limit in Python.
        # Split on \n only — splitlines() would also split on \r and break the
        # CR-defragmentation step in the CLI layer.
        # tmux always terminates output with \n, so split produces a trailing "".
        # Taking -(lines+1) elements keeps that "" so the join restores the newline.
        return "\n".join(raw.split("\n")[-(lines + 1) :])

    def pane_exists(self, *, target_pane_id: str) -> bool:
        """Return True iff target_pane_id currently appears in the tmux server's pane list.

        Uses ``tmux list-panes -a`` (all sessions on the server) so the check stays
        correct even if the pane somehow migrated to a different window.
        """
        return target_pane_id in self.list_pane_ids()

    def list_pane_ids(self) -> set[str]:
        """Return the set of all live pane ids across the tmux server.

        ``tmux list-panes -a -F '#{pane_id}'`` lists every pane on the server
        (all sessions), so one call resolves pane liveness for every member in a
        monitor tick. The 5 s timeout (consistent with the other tmux helpers)
        keeps a hung tmux from blocking the monitor loop indefinitely.
        """
        return set(
            _run(["tmux", "list-panes", "-a", "-F", "#{pane_id}"], timeout=5).split()
        )

    def kill_pane(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        """Unconditionally kill the target pane. Swallows pane-gone errors when ignore_missing=True."""
        _run_tolerating_pane_gone(
            ["tmux", "kill-pane", "-t", target_pane_id],
            ignore_missing=ignore_missing,
        )
