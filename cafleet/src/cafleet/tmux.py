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
    """Split the target window with ``command`` and return the new pane id."""
    args = ["tmux", "split-window", "-t", target_window_id, "-P", "-F", "#{pane_id}"]
    for k, v in env.items():
        args += ["-e", f"{k}={v}"]
    args += command
    return _run(args).strip()


def select_layout(*, target_window_id: str, layout: str = "main-vertical") -> None:
    _run(["tmux", "select-layout", "-t", target_window_id, layout])


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


def send_exit(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Send ``/exit`` + Enter, swallowing pane-gone errors when requested."""
    try:
        _run(["tmux", "send-keys", "-t", target_pane_id, "/exit", "Enter"])
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


def send_poll_trigger(*, target_pane_id: str, session_id: str, agent_id: str) -> bool:
    """Best-effort ``cafleet ... message poll`` trigger for the recipient's pane.

    The keystroke is sent literally so the recipient's Bash tool runs it
    directly — members are spawned with ``--permission-mode dontAsk`` so
    the Bash tool is enabled and permission prompts auto-resolve. Returns
    False on any tmux failure or when the binary is missing, never raising.

    Split into two ``send-keys`` calls for the same reason as
    ``send_freetext_and_submit``: ``-l`` is per-invocation, so mixing
    literal text with the ``Enter`` key name in one call means tmux
    does not interpret ``Enter`` as the Enter key. It is sent literally
    instead of producing the submit keypress that prompts such as the
    Claude Code input box expect.
    """
    if shutil.which("tmux") is None:
        return False
    try:
        _run(
            [
                "tmux",
                "send-keys",
                "-t",
                target_pane_id,
                "-l",
                f"cafleet --session-id {session_id} message poll --agent-id {agent_id}",
            ],
            timeout=5,
        )
        _run(
            ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
            timeout=5,
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
    """Send ``4`` + literal ``text`` + Enter as three separate send-keys calls.

    tmux's ``-l`` (literal) flag is per-invocation, so a single call cannot
    mix literal characters with the ``Enter`` key name. Splitting also means
    embedded ``Enter`` / ``C-c`` / ``Esc`` in ``text`` land as plain chars.
    """
    if "\n" in text or "\r" in text:
        raise TmuxError("send_freetext_and_submit: text may not contain newlines")
    _run(["tmux", "send-keys", "-t", target_pane_id, "4"])
    _run(["tmux", "send-keys", "-t", target_pane_id, "-l", text])
    _run(["tmux", "send-keys", "-t", target_pane_id, "Enter"])


def send_bash_command(*, target_pane_id: str, command: str) -> None:
    """Send ``! <command>`` + Enter as two separate send-keys calls.

    Used by ``cafleet member send-input --bash`` to route shell commands
    via Claude Code's ``!`` shortcut. Unlike ``send_freetext_and_submit``,
    there is NO leading ``4`` keystroke (no AskUserQuestion gate).
    """
    normalized_command = command.strip()
    if not normalized_command:
        raise TmuxError("send_bash_command: command may not be empty")
    if "\n" in command or "\r" in command:
        raise TmuxError("send_bash_command: command may not contain newlines")
    _run(["tmux", "send-keys", "-t", target_pane_id, "-l", f"! {normalized_command}"])
    _run(["tmux", "send-keys", "-t", target_pane_id, "Enter"])


def capture_pane(*, target_pane_id: str, lines: int = 80) -> str:
    """Return the last ``lines`` lines of the pane's terminal buffer."""
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
    try:
        _run(["tmux", "kill-pane", "-t", target_pane_id])
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


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
