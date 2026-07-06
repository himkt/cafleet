"""herdr terminal-multiplexer backend.

Mirrors :mod:`cafleet.multiplexer.tmux`: a ``_run`` subprocess dispatcher (argv
list, no shell) over the stable ``herdr`` ``pane`` / ``wait`` command set, and
:class:`HerdrMultiplexer` realizing every :class:`~cafleet.multiplexer.base.Multiplexer`
method plus the optional :class:`~cafleet.multiplexer.base.AgentStateAware`
capability. Pane ids are opaque strings (``w1:p1``), passed verbatim.

``herdr pane run`` submits text and Enter atomically, so there is no
literal-then-Enter submit delay; the ``esc_first`` safeguard maps to a discrete
``pane send-keys <id> esc``; the 2-line inline preview uses ``pane send-text``
(raw, no Enter) then one ``pane send-keys enter``.

The herdr *argv* is the stable contract; its stdout *formats* are not pinned by
design 0000121, so the parsers here assume: ``pane current`` prints the pane id;
``pane get`` prints ``key: value`` lines with ``session`` / ``tab`` (and
``agent_status`` when an agent is detected); ``pane list`` prints one pane id per
token; ``wait agent-status`` exits 0 when reached, non-zero on timeout. Validate
against a real herdr binary.
"""

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


class HerdrError(MultiplexerError):
    """Raised when a herdr subprocess fails or herdr is not reachable."""


_PANE_GONE_MARKERS = ("not found", "no such pane", "does not exist")
_ESC_SETTLE_DELAY = 0.1


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
            f"herdr command failed: {' '.join(args)}\nstderr: {exc.stderr.strip()}"
        ) from exc
    return result.stdout


def _run_tolerating_missing(
    args: list[str], *, ignore_missing: bool, timeout: float | None = None
) -> None:
    try:
        _run(args, timeout=timeout)
    except HerdrError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
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


def _parse_pane_info(output: str) -> dict[str, str]:
    info: dict[str, str] = {}
    for line in output.splitlines():
        if ":" in line:
            key, _, value = line.partition(":")
            info[key.strip()] = value.strip()
    return info


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
        pane_id = _run(["herdr", "pane", "current"]).strip()
        if not pane_id:
            raise HerdrError(
                "herdr pane current returned no pane; not inside a herdr pane"
            )
        info = _parse_pane_info(_run(["herdr", "pane", "get", pane_id]))
        try:
            session = info["session"]
            window_id = info["tab"]
        except KeyError as exc:
            raise HerdrError(
                f"herdr pane get missing {exc} field for pane {pane_id!r}"
            ) from exc
        return MultiplexerContext(session=session, window_id=window_id, pane_id=pane_id)

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
        new_pane_id = _run(split_args).strip()
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
        return _run(
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

    def pane_exists(self, *, target_pane_id: str) -> bool:
        try:
            _run(["herdr", "pane", "get", target_pane_id])
        except HerdrError as exc:
            if any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
                return False
            raise
        return True

    def list_pane_ids(self) -> set[str]:
        return set(_run(["herdr", "pane", "list"], timeout=5).split())

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
        info = _parse_pane_info(_run(["herdr", "pane", "get", target_pane_id]))
        return info.get("agent_status") or None

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
