import contextlib
import os
import shutil
import subprocess
import time

from cafleet.multiplexer.base import MultiplexerContext, poll_until_pane_gone


class TmuxError(Exception):
    """Raised when a tmux subprocess fails or tmux is not reachable."""


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


# Sleep between literal-text ``send-keys -l`` and following ``send-keys Enter``
# so the codex TUI's bracketed-paste finalises and opencode's slash-command
# autocomplete popup settles before submit (applied unconditionally to keep
# the helpers backend-agnostic).
_SUBMIT_DELAY = 0.12

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
        # blindly confirm it. Opt-in (ping helpers only) — an ``Esc`` before
        # ``/exit``, an inline preview, or ``! <cmd>`` would mis-fire.
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
        target_window_id: str,
        env: dict[str, str],
        command: list[str],
    ) -> str:
        """Split the target window with ``command`` and return the new pane id.

        Always invoked with ``-d`` so the new pane is not made active and the
        calling client's active window is not switched. This behavior is
        unconditional — there is no opt-out parameter.
        """
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
        self, *, target_pane_id: str, fleet_id: int, agent_id: int
    ) -> bool:
        """Best-effort Esc-safeguarded ``cafleet ... message poll`` keystroke.

        ``esc_first=True`` leads with ``Escape`` so a pane on a pending
        permission-approval prompt dismisses it rather than confirming it with
        the trailing ``Enter``. The Director's manual ``cafleet member ping``
        reuses this helper, inheriting the safeguard for free.
        """
        if shutil.which("tmux") is None:
            return False
        payload = f"cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}"
        try:
            _send_literal_then_enter(
                target_pane_id=target_pane_id,
                payload=payload,
                timeout=5,
                esc_first=True,
            )
        except TmuxError:
            return False
        return True

    def send_wake_trigger(
        self, *, target_pane_id: str, fleet_id: int, agent_id: int
    ) -> bool:
        """Best-effort Esc-safeguarded wake nudge for the monitoring member's pane.

        Carries a single-line instruction to run the monitoring member's
        capture-classify-reengage routine — distinct from the Director's poll
        command. No shell-special characters, so the keystroke is sane whether
        it lands in the coding agent's input or at a shell prompt. ``fleet_id`` /
        ``agent_id`` keep the keystroke-helper signature uniform with
        ``send_poll_trigger`` (the loop calls both identically); the routine
        itself runs in the monitoring member's own pane, so they are not echoed
        into the nudge.
        """
        if shutil.which("tmux") is None:
            return False
        payload = (
            "[monitor] wake: run your monitoring routine now — capture the "
            "Director pane, judge it active vs idle, and if idle assess the "
            "inbox and members and re-engage the Director with an "
            "Esc-safeguarded nudge."
        )
        try:
            _send_literal_then_enter(
                target_pane_id=target_pane_id,
                payload=payload,
                timeout=5,
                esc_first=True,
            )
        except TmuxError:
            return False
        return True

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        task_id: int,
        sender_id: int,
        ts: str,
        text: str,
    ) -> bool:
        """Best-effort 2-line inline preview keystroke for the recipient's pane."""
        if shutil.which("tmux") is None:
            return False
        # Sanitize newlines in the user-supplied body — under tmux ``-l`` a raw
        # newline submits as Enter, corrupting the 2-line shape. The single ``\n``
        # in the f-string below is the contract between envelope and body and is
        # intentionally not sanitized.
        sanitized_text = text.replace("\r\n", "⏎").replace("\n", "⏎").replace("\r", "⏎")
        payload = f"[cafleet msg {task_id} from {sender_id} {ts}]\n{sanitized_text}"
        try:
            _send_literal_then_enter(
                target_pane_id=target_pane_id, payload=payload, timeout=5
            )
        except TmuxError:
            return False
        return True

    def send_choice_key(self, *, target_pane_id: str, digit: int) -> None:
        """Send a single digit key in {1, 2, 3} to the pane (no Enter)."""
        if digit not in (1, 2, 3):
            raise TmuxError(f"send_choice_key: digit must be 1, 2, or 3 (got {digit})")
        _run(["tmux", "send-keys", "-t", target_pane_id, str(digit)])

    def send_freetext_and_submit(self, *, target_pane_id: str, text: str) -> None:
        """Send ``4`` + literal ``text`` + Enter as three separate send-keys calls."""
        if "\n" in text or "\r" in text:
            raise TmuxError("send_freetext_and_submit: text may not contain newlines")
        _run(["tmux", "send-keys", "-t", target_pane_id, "4"])
        _send_literal_then_enter(target_pane_id=target_pane_id, payload=text)

    def send_bash_command(self, *, target_pane_id: str, command: str) -> None:
        """Send ``! <command>`` + Enter, routing shell via the coding agent's ``!`` shortcut."""
        normalized_command = command.strip()
        if not normalized_command:
            raise TmuxError("send_bash_command: command may not be empty")
        if "\n" in command or "\r" in command:
            raise TmuxError("send_bash_command: command may not contain newlines")
        _send_literal_then_enter(
            target_pane_id=target_pane_id, payload=f"! {normalized_command}"
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
        (all sessions), so one call resolves pane liveness for every agent in a
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

    def wait_for_pane_gone(
        self,
        *,
        target_pane_id: str,
        timeout: float = 15.0,
        interval: float = 0.5,
    ) -> bool:
        """Poll ``pane_exists`` until the pane is absent or the timeout elapses.

        Returns True if the pane disappeared, False on timeout. Errors from
        ``pane_exists`` propagate as TmuxError (caller decides).
        """
        return poll_until_pane_gone(
            lambda: self.pane_exists(target_pane_id=target_pane_id),
            timeout=timeout,
            interval=interval,
        )
