import os
import shutil
import subprocess
import time
from dataclasses import dataclass


class TmuxError(Exception):
    """Raised when a tmux subprocess fails or tmux is not reachable."""


@dataclass(frozen=True)
class DirectorContext:
    session: str
    window_id: str
    pane_id: str


def ensure_tmux_available() -> None:
    if shutil.which("tmux") is None:
        raise TmuxError("tmux binary not found on PATH")
    if not os.environ.get("TMUX"):
        raise TmuxError("cafleet member commands must be run inside a tmux session")


def director_context() -> DirectorContext:
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
    return DirectorContext(session=session, window_id=window_id, pane_id=pane_id)


def split_window(
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
    return _run(args).strip()


def select_layout(*, target_window_id: str, layout: str = "main-vertical") -> None:
    _run(["tmux", "select-layout", "-t", target_window_id, layout])


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


def _run_tolerating_pane_gone(args: list[str], *, ignore_missing: bool) -> None:
    try:
        _run(args)
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


# Sleep between literal-text ``send-keys -l`` and following ``send-keys Enter``
# so the codex TUI's bracketed-paste finalises before submit (applied
# unconditionally to keep the helpers backend-agnostic).
_SUBMIT_DELAY = 0.12


def _send_literal_then_enter(
    *, target_pane_id: str, payload: str, timeout: float | None = None
) -> None:
    _run(
        ["tmux", "send-keys", "-t", target_pane_id, "-l", payload],
        timeout=timeout,
    )
    time.sleep(_SUBMIT_DELAY)
    _run(
        ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
        timeout=timeout,
    )


def send_exit(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Send ``/exit`` + Enter, swallowing pane-gone errors when requested."""
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "/exit", "Enter"],
        ignore_missing=ignore_missing,
    )


def send_poll_trigger(*, target_pane_id: str, session_id: str, agent_id: str) -> bool:
    """Best-effort ``cafleet ... message poll`` keystroke for the recipient's pane."""
    if shutil.which("tmux") is None:
        return False
    payload = f"cafleet --session-id {session_id} message poll --agent-id {agent_id}"
    try:
        _send_literal_then_enter(
            target_pane_id=target_pane_id, payload=payload, timeout=5
        )
    except TmuxError:
        return False
    return True


def send_inline_preview(
    *,
    target_pane_id: str,
    task_id_8: str,
    sender_8: str,
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
    payload = f"[cafleet msg {task_id_8} from {sender_8} {ts}]\n{sanitized_text}"
    try:
        _send_literal_then_enter(
            target_pane_id=target_pane_id, payload=payload, timeout=5
        )
    except TmuxError:
        return False
    return True


def send_choice_key(*, target_pane_id: str, digit: int) -> None:
    """Send a single digit key in {1, 2, 3} to the pane (no Enter)."""
    if digit not in (1, 2, 3):
        raise TmuxError(f"send_choice_key: digit must be 1, 2, or 3 (got {digit})")
    _run(["tmux", "send-keys", "-t", target_pane_id, str(digit)])


def send_freetext_and_submit(*, target_pane_id: str, text: str) -> None:
    """Send ``4`` + literal ``text`` + Enter as three separate send-keys calls."""
    if "\n" in text or "\r" in text:
        raise TmuxError("send_freetext_and_submit: text may not contain newlines")
    _run(["tmux", "send-keys", "-t", target_pane_id, "4"])
    _send_literal_then_enter(target_pane_id=target_pane_id, payload=text)


def send_bash_command(*, target_pane_id: str, command: str) -> None:
    """Send ``! <command>`` + Enter, routing shell via the coding agent's ``!`` shortcut."""
    normalized_command = command.strip()
    if not normalized_command:
        raise TmuxError("send_bash_command: command may not be empty")
    if "\n" in command or "\r" in command:
        raise TmuxError("send_bash_command: command may not contain newlines")
    _send_literal_then_enter(
        target_pane_id=target_pane_id, payload=f"! {normalized_command}"
    )


def capture_pane(*, target_pane_id: str, lines: int = 30) -> str:
    """Return the last ``lines`` lines of the pane's terminal buffer.

    Default is 30 to keep per-tick Director monitoring captures from
    dominating token cost. Explicit ``lines`` overrides apply unchanged.
    """
    if lines <= 0:
        raise TmuxError(f"capture_pane: lines must be positive, got {lines}")
    return _run(["tmux", "capture-pane", "-p", "-t", target_pane_id, "-S", f"-{lines}"])


def pane_exists(*, target_pane_id: str) -> bool:
    """Return True iff target_pane_id currently appears in the tmux server's pane list.

    Uses ``tmux list-panes -a`` (all sessions on the server) so the check stays
    correct even if the pane somehow migrated to a different window.
    """
    out = _run(["tmux", "list-panes", "-a", "-F", "#{pane_id}"])
    return target_pane_id in out.split()


def kill_pane(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Unconditionally kill the target pane. Swallows pane-gone errors when ignore_missing=True."""
    _run_tolerating_pane_gone(
        ["tmux", "kill-pane", "-t", target_pane_id],
        ignore_missing=ignore_missing,
    )


def wait_for_pane_gone(
    *, target_pane_id: str, timeout: float = 15.0, interval: float = 0.5
) -> bool:
    """Poll ``pane_exists`` until the pane is absent or the timeout elapses.

    Returns True if the pane disappeared, False on timeout. Errors from
    ``pane_exists`` propagate as TmuxError (caller decides).
    """
    deadline = time.monotonic() + timeout
    while True:
        if not pane_exists(target_pane_id=target_pane_id):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)


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
