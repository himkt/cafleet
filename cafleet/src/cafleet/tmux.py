import os
import shutil
import subprocess
from dataclasses import dataclass


class TmuxError(Exception):
    """Raised when a tmux subprocess fails or tmux is not reachable."""


@dataclass(frozen=True)
class DirectorContext:
    session: str  # e.g. 'main'
    window_id: str  # e.g. '@3'
    pane_id: str  # e.g. '%0' — the Director's own pane


def ensure_tmux_available() -> None:
    """Raise TmuxError if the `tmux` binary is not on PATH or TMUX is unset."""
    if shutil.which("tmux") is None:
        raise TmuxError("tmux binary not found on PATH")
    if not os.environ.get("TMUX"):
        raise TmuxError("cafleet member commands must be run inside a tmux session")


def director_context() -> DirectorContext:
    """Resolve the Director's own tmux session, window_id, and pane_id.

    Uses the TMUX_PANE env var as the anchor and queries tmux for the
    containing window. Works regardless of which window the user is
    currently focused on.
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
    """Spawn a coding agent in a new pane in `target_window_id`.

    Returns the new pane_id (e.g. '%7'). Forwards env as `-e KEY=VAL` flags.
    """
    args = ["tmux", "split-window", "-t", target_window_id, "-P", "-F", "#{pane_id}"]
    for k, v in env.items():
        args += ["-e", f"{k}={v}"]
    args += command
    return _run(args).strip()


def select_layout(*, target_window_id: str, layout: str = "main-vertical") -> None:
    _run(["tmux", "select-layout", "-t", target_window_id, layout])


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


def send_exit(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Send '/exit' + Enter to the given pane.

    If `ignore_missing=True` and tmux reports the pane no longer exists
    (matched against _PANE_GONE_MARKERS), return silently instead of raising.
    Any other tmux failure raises `TmuxError`.
    """
    try:
        _run(["tmux", "send-keys", "-t", target_pane_id, "/exit", "Enter"])
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


def send_poll_trigger(*, target_pane_id: str, agent_id: str) -> bool:
    """Send a cafleet poll trigger to the given tmux pane.

    Returns True on success, False if tmux is unavailable or the pane
    no longer exists. Never raises — internally calls _run() and catches
    TmuxError, returning False on any failure.
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
                f"cafleet poll --agent-id {agent_id}",
                "Enter",
            ],
            timeout=5,
        )
    except TmuxError:
        return False
    return True


def capture_pane(*, target_pane_id: str, lines: int = 80) -> str:
    """Capture the last `lines` lines of the target pane's terminal buffer.

    Invokes `tmux capture-pane -p -t <pane_id> -S -<lines>`. Returns the
    raw captured string as tmux emitted it (bytes decoded via text=True).
    Raises `TmuxError` on failure — the caller should surface "can't find
    pane" errors to the user rather than swallowing them, since the whole
    point of capture is to inspect a live pane.
    """
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
