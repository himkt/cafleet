"""herdr terminal-multiplexer backend.

Mirrors :mod:`cafleet.multiplexer.tmux`: a ``_run`` subprocess dispatcher (argv
list, no shell) over the stable ``herdr`` ``pane`` / ``wait`` command set, and
:class:`HerdrMultiplexer` realizing every :class:`~cafleet.multiplexer.base.Multiplexer`
method plus the optional :class:`~cafleet.multiplexer.base.AgentStateAware`
capability. Pane ids are opaque strings (``wG:p1``), passed verbatim.

``herdr pane run`` submits text and Enter atomically, so there is no
literal-then-Enter submit delay; the ``esc_first`` safeguard maps to a discrete
``pane send-keys <id> esc``; the 2-line inline preview uses ``pane send-text``
(raw, no Enter) then one ``pane send-keys enter``.

**herdr CLI JSON envelope.** Every command prints a JSON envelope. Success →
exit 0, stdout ``{"id":.., "result":{...}, "type":".."}``. Error → non-zero
exit, stderr ``{"error":{"code":"..", "message":".."}, "id":".."}``. Reads go
through ``_run_json`` (returns the ``result`` object); the missing-pane case is
detected by the error ``code == "pane_not_found"``. A ``result.pane`` object
carries ``pane_id`` (``wG:p1``), ``tab_id`` (``wG:t1``), ``workspace_id``
(``wG``), and ``agent_status`` (``idle``/``working``/``blocked``/``done``/
``unknown``). ``MultiplexerContext`` maps ``session ← workspace_id``,
``window_id ← tab_id``, ``pane_id ← pane_id``.
"""

import json
import os
import shlex
import shutil
import subprocess
import time
from collections.abc import Callable

from cafleet.multiplexer.base import (
    MultiplexerContext,
    MultiplexerError,
    poll_until_pane_gone,
)

_PANE_NOT_FOUND = "pane_not_found"
_ESC_SETTLE_DELAY = 0.1


class HerdrError(MultiplexerError):
    """Raised when a herdr subprocess fails or herdr is not reachable.

    ``code`` carries the herdr error envelope's ``error.code`` when the failure
    was a non-zero exit with a JSON error body (else ``None``).
    """

    def __init__(self, message: str, *, code: str | None = None) -> None:
        super().__init__(message)
        self.code = code


def _error_code(stderr: str) -> str | None:
    try:
        payload = json.loads(stderr)
    except (ValueError, TypeError):
        return None
    error = payload.get("error") if isinstance(payload, dict) else None
    return error.get("code") if isinstance(error, dict) else None


def _run(args: list[str], *, timeout: float | None = None) -> str:
    try:
        result = subprocess.run(
            args, capture_output=True, text=True, check=True, timeout=timeout
        )
    except FileNotFoundError as exc:
        raise HerdrError(f"herdr binary not found: {exc}") from exc
    except subprocess.TimeoutExpired as exc:
        raise HerdrError(
            f"herdr command timed out after {exc.timeout}s: {' '.join(args)}"
        ) from exc
    except subprocess.CalledProcessError as exc:
        raise HerdrError(
            f"herdr command failed: {' '.join(args)}\nstderr: {exc.stderr.strip()}",
            code=_error_code(exc.stderr),
        ) from exc
    return result.stdout


def _run_json(args: list[str], *, timeout: float | None = None) -> dict:
    out = _run(args, timeout=timeout)
    try:
        payload = json.loads(out)
    except ValueError as exc:
        raise HerdrError(f"herdr returned non-JSON output: {out!r}") from exc
    result = payload.get("result") if isinstance(payload, dict) else None
    if not isinstance(result, dict):
        raise HerdrError(f"herdr output missing 'result' object: {out!r}")
    return result


def _run_tolerating_missing(
    args: list[str], *, ignore_missing: bool, timeout: float | None = None
) -> None:
    try:
        _run(args, timeout=timeout)
    except HerdrError as exc:
        if ignore_missing and exc.code == _PANE_NOT_FOUND:
            return
        raise


def _best_effort(steps: Callable[[], None]) -> bool:
    """Run a keystroke sequence, mapping herdr-absent or any ``HerdrError`` to False."""
    if shutil.which("herdr") is None:
        return False
    try:
        steps()
    except HerdrError:
        return False
    return True


def _sanitize_wake_name(name: str) -> str:
    """Neutralize CR/LF/tab/backtick/``$(`` in a name so the wake payload stays
    single-line with no shell metacharacters (mirrors the tmux payload contract)."""
    return (
        name.replace("\r\n", "⏎")
        .replace("\n", "⏎")
        .replace("\r", "⏎")
        .replace("\t", "⏎")
        .replace("`", "ˋ")
        .replace("$(", "$﹙")
    )


class HerdrMultiplexer:
    name = "herdr"

    def ensure_available(self) -> None:
        if shutil.which("herdr") is None:
            raise HerdrError("herdr binary not found on PATH")
        if not os.environ.get("HERDR_ENV"):
            raise HerdrError(
                "cafleet member commands must be run inside a herdr session"
            )

    def context_discovery(self) -> MultiplexerContext:
        result = _run_json(["herdr", "pane", "current"])
        try:
            pane = result["pane"]
            return MultiplexerContext(
                session=pane["workspace_id"],
                window_id=pane["tab_id"],
                pane_id=pane["pane_id"],
            )
        except KeyError as exc:
            raise HerdrError(f"herdr pane current missing {exc} field") from exc

    def split_window(
        self,
        *,
        reference: MultiplexerContext,
        env: dict[str, str],
        command: list[str],
    ) -> str:
        split_args = [
            "herdr",
            "pane",
            "split",
            reference.pane_id,
            "--direction",
            "down",
            "--no-focus",
        ]
        for k, v in env.items():
            split_args += ["--env", f"{k}={v}"]
        result = _run_json(split_args)
        try:
            new_pane_id = result["pane"]["pane_id"]
        except KeyError as exc:
            raise HerdrError(f"herdr pane split missing {exc} field") from exc
        # pane run feeds one shell line, so the argv is quoted to preserve boundaries.
        _run(["herdr", "pane", "run", new_pane_id, shlex.join(command)])
        return new_pane_id

    def send_exit(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        _run_tolerating_missing(
            ["herdr", "pane", "run", target_pane_id, "/exit"],
            ignore_missing=ignore_missing,
        )

    def send_poll_trigger(
        self, *, target_pane_id: str, fleet_id: int, agent_id: int
    ) -> bool:
        payload = f"cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}"

        def steps() -> None:
            self._send_esc(target_pane_id)
            _run(["herdr", "pane", "run", target_pane_id, payload], timeout=5)

        return _best_effort(steps)

    def send_wake_trigger(
        self, *, target_pane_id: str, due_agents: list[dict], director_agent_id: int
    ) -> bool:
        noun = "agent" if len(due_agents) == 1 else "agents"
        due_list = ", ".join(
            f"{'director' if t['is_director'] else 'member'} {t['agent_id']} "
            f"({_sanitize_wake_name(t['name'])})"
            for t in due_agents
        )
        payload = (
            f"[monitor] wake: {len(due_agents)} {noun} due — {due_list}. "
            f"Capture each named pane read-only, with the Director pane "
            f"({director_agent_id}) always inspected; judge each active/idle and "
            "progressing/stalled; re-engage the Director via cafleet member nudge "
            "when it is idle with un-acked work or any due agent looks stalled."
        )

        # No esc: the monitoring member's own pane is never on a permission prompt.
        def steps() -> None:
            _run(["herdr", "pane", "run", target_pane_id, payload], timeout=5)

        return _best_effort(steps)

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        task_id: int,
        sender_id: int,
        ts: str,
        text: str,
    ) -> bool:
        sanitized_text = text.replace("\r\n", "⏎").replace("\n", "⏎").replace("\r", "⏎")
        payload = f"[cafleet msg {task_id} from {sender_id} {ts}]\n{sanitized_text}"

        # send-text delivers the raw 2-line payload without submitting; the single
        # trailing enter submits the whole payload as one recipient turn.
        def steps() -> None:
            self._send_esc(target_pane_id)
            _run(["herdr", "pane", "send-text", target_pane_id, payload], timeout=5)
            _run(["herdr", "pane", "send-keys", target_pane_id, "enter"], timeout=5)

        return _best_effort(steps)

    def send_bash_command(self, *, target_pane_id: str, command: str) -> None:
        normalized_command = command.strip()
        if not normalized_command:
            raise HerdrError("send_bash_command: command may not be empty")
        if "\n" in command or "\r" in command:
            raise HerdrError("send_bash_command: command may not contain newlines")
        _run(["herdr", "pane", "run", target_pane_id, f"! {normalized_command}"])

    def capture_pane(self, *, target_pane_id: str, lines: int = 20) -> str:
        if lines <= 0:
            raise HerdrError(f"capture_pane: lines must be positive, got {lines}")
        result = _run_json(
            [
                "herdr",
                "pane",
                "read",
                target_pane_id,
                "--source",
                "recent-unwrapped",
                "--lines",
                str(lines),
            ]
        )
        # The exact pane-read output key is pending operator validation; herdr
        # docs suggest `output` / `content`.
        text = result.get("output")
        if text is None:
            text = result.get("content", "")
        return text

    def pane_exists(self, *, target_pane_id: str) -> bool:
        try:
            _run(["herdr", "pane", "get", target_pane_id])
        except HerdrError as exc:
            if exc.code == _PANE_NOT_FOUND:
                return False
            raise
        return True

    def list_pane_ids(self) -> set[str]:
        result = _run_json(["herdr", "pane", "list"], timeout=5)
        try:
            return {p["pane_id"] for p in result["panes"]}
        except KeyError as exc:
            raise HerdrError(f"herdr pane list missing {exc} field") from exc

    def kill_pane(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
        _run_tolerating_missing(
            ["herdr", "pane", "close", target_pane_id],
            ignore_missing=ignore_missing,
        )

    def wait_for_pane_gone(
        self,
        *,
        target_pane_id: str,
        timeout: float = 15.0,
        interval: float = 0.5,
    ) -> bool:
        return poll_until_pane_gone(
            lambda: self.pane_exists(target_pane_id=target_pane_id),
            timeout=timeout,
            interval=interval,
        )

    def agent_status(self, *, target_pane_id: str) -> str | None:
        result = _run_json(["herdr", "pane", "get", target_pane_id])
        try:
            pane = result["pane"]
        except KeyError as exc:
            raise HerdrError(f"herdr pane get missing {exc} field") from exc
        return pane.get("agent_status") or None

    def wait_agent_status(
        self, *, target_pane_id: str, status: str, timeout_ms: int
    ) -> bool:
        try:
            _run(
                [
                    "herdr",
                    "wait",
                    "agent-status",
                    target_pane_id,
                    "--status",
                    status,
                    "--timeout",
                    str(timeout_ms),
                ],
                timeout=timeout_ms / 1000 + 5,
            )
        except HerdrError:
            return False
        return True

    def _send_esc(self, target_pane_id: str) -> None:
        _run(["herdr", "pane", "send-keys", target_pane_id, "esc"], timeout=5)
        time.sleep(_ESC_SETTLE_DELAY)
