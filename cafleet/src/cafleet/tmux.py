import os
import shutil
import subprocess
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
    """Best-effort ``cafleet poll`` trigger for the recipient's pane.

    The command string is sent literally so the recipient's
    ``permissions.allow`` can match it. Returns False on any tmux failure
    or when the binary is missing, never raising.
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
                f"cafleet --session-id {session_id} poll --agent-id {agent_id}",
                "Enter",
            ],
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


def capture_pane(*, target_pane_id: str, lines: int = 80) -> str:
    """Return the last ``lines`` lines of the pane's terminal buffer."""
    if lines <= 0:
        raise TmuxError(f"capture_pane: lines must be positive, got {lines}")
    return _run(["tmux", "capture-pane", "-p", "-t", target_pane_id, "-S", f"-{lines}"])


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
