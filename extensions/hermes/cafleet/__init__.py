"""CAFleet Director reception: push each `cafleet message listen` record into the owning session."""

from __future__ import annotations

import asyncio
import json
import logging
import os
import queue
import subprocess
import threading
import time
from pathlib import Path

logger = logging.getLogger("hermes_plugins.cafleet")

RECORD_PREFIX = "[cafleet listen record] "
CLOSED_PREFIX = "[cafleet listen closed] "
SETUP_FIX = "run 'cafleet setup --coding-agent hermes'"
WATCHDOG_INTERVAL_S = 5.0
DETACH_GRACE_S = 30.0
EXIT_CONFIRM_S = 5.0
INJECT_DEADLINE_S = 10.0
VERIFY_DEADLINE_S = 10.0
REFUSAL_RETRY_S = 1.0
REFUSAL_LIMIT_S = 600.0
INJECTION_SEAM = "gateway runner._dispatch_plugin_message_injection"
CONSENT_ERROR = f"plugins.entries.cafleet.allow_gateway_injection is not true; {SETUP_FIX}"
MEMBER_REFUSAL = "cafleet_listen runs only in a Director session"

_ctx = None
_receivers: dict[str, "Receiver"] = {}
_receivers_lock = threading.Lock()


def _result(**fields) -> str:
    return json.dumps(fields)


def _session_key() -> str:
    from hermes_cli.plugin_session import session_key
    return session_key()


def _gateway_runner():
    injector = _ctx._manager._gateway_message_injector
    if injector is None:
        raise RuntimeError("no live gateway serves this plugin")
    return injector[0]


def _injection_target():
    runner = _gateway_runner()
    loop = runner._gateway_loop
    if loop is None or loop.is_closed():
        raise RuntimeError("the gateway loop is not running")
    return runner._dispatch_plugin_message_injection, loop


def _authority():
    from gateway.session_authorities import authority_for_home
    home = _ctx._manager.home_path
    authority = authority_for_home(_gateway_runner(), home)
    if authority is None:
        raise RuntimeError(f"no session authority serves {home}")
    return authority


def _session_attached(session_key: str) -> bool:
    return any(live.route == session_key and live.subscribers for live in list(_authority().sessions.values()))


def _recorded_executable() -> Path:
    record = Path(__file__).with_name("executable")
    try:
        target = Path(record.read_text(encoding="utf-8").strip())
    except OSError as exc:
        raise ValueError(f"executable record {record}: {exc}; {SETUP_FIX}") from None
    if not (target.is_file() and os.access(target, os.X_OK)):
        raise ValueError(f"executable record {record}: {target} is not an executable file; {SETUP_FIX}")
    return target


def _database_args(database_url: str | None) -> list[str]:
    return ["--database-url", database_url] if database_url else []


def _cafleet_env() -> dict[str, str]:
    return {name: value for name, value in os.environ.items() if not name.startswith("CAFLEET_")}


def _director_refusal(executable: Path, director_id: int, database_url: str | None) -> str | None:
    """None when `director_id` is a root Director, otherwise the member refusal with the CLI's stderr."""
    try:
        shown = subprocess.run(
            [str(executable), *_database_args(database_url), "member", "show", str(director_id), "--json"],
            stdin=subprocess.DEVNULL, capture_output=True, encoding="utf-8", errors="replace",
            env=_cafleet_env(), timeout=VERIFY_DEADLINE_S,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return f"{MEMBER_REFUSAL}: {exc}"
    if shown.returncode == 0:
        try:
            if json.loads(shown.stdout).get("kind") == "director":
                return None
        except ValueError:
            pass
    return f"{MEMBER_REFUSAL}: {shown.stderr.strip()}"


class Receiver:
    def __init__(self, session_key: str, session_id: str, proc: subprocess.Popen):
        self.session_key = session_key
        self.session_id = session_id
        self.proc = proc
        self.stopping = threading.Event()
        self.saw_closed = False
        self.notice_attempted = False
        self.last_stderr = ""
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.stderr_thread: threading.Thread | None = None
        self.deliver_thread: threading.Thread | None = None

    def alive(self) -> bool:
        return self.proc.poll() is None

    def start(self) -> None:
        self.stderr_thread = self._spawn_thread(self._pump_stderr, "stderr")
        self.deliver_thread = self._spawn_thread(self._deliver, "deliver")
        for target, name in ((self._read_stdout, "stdout"), (self._watch, "watchdog")):
            self._spawn_thread(target, name)

    def _spawn_thread(self, target, name: str) -> threading.Thread:
        thread = threading.Thread(target=target, daemon=True, name=f"cafleet-listen-{name}-{self.proc.pid}")
        thread.start()
        return thread

    def stop(self, reason: str) -> None:
        if self.stopping.is_set():
            return
        self.stopping.set()
        logger.info("cafleet_listen: ending receiver pid=%s session=%s: %s", self.proc.pid, self.session_key, reason)
        if self.alive():
            self.proc.kill()
        try:
            self.proc.wait(timeout=EXIT_CONFIRM_S)
        except subprocess.TimeoutExpired:
            logger.error("cafleet_listen: receiver pid=%s did not exit within %ss", self.proc.pid, EXIT_CONFIRM_S)

    def _inject(self, text: str) -> bool:
        """Commit one turn into the session's durable FIFO and wait for the commit.

        `ctx.inject_message` returns once its dispatch coroutine is scheduled, so consecutive calls race
        through their awaited session lookups and can commit out of order."""
        dispatch, loop = _injection_target()
        coro = dispatch(session_key=self.session_key, content=text, plugin_id=_ctx.plugin_id,
                        plugin_home=_ctx._manager.home_path)
        try:
            future = asyncio.run_coroutine_threadsafe(coro, loop)
        except BaseException:
            coro.close()
            raise
        try:
            return bool(future.result(timeout=INJECT_DEADLINE_S))
        except TimeoutError:
            future.cancel()
            raise TimeoutError(f"the injection did not finish within {INJECT_DEADLINE_S:.0f} s") from None

    def _read_stdout(self) -> None:
        try:
            for raw in self.proc.stdout:
                line = raw.rstrip("\n")
                if line:
                    self.lines.put(line)
        finally:
            self.lines.put(None)

    def _commit(self, text: str) -> bool:
        """Inject one record, retrying each refusal (a full admission queue, a draining gateway, a detached
        client); False once the receiver has ended.

        A detached session keeps retrying because the watchdog's detach grace decides when it is gone."""
        refused_since = None
        while not self._inject(text):
            refused_since = refused_since or time.monotonic()
            if time.monotonic() - refused_since >= REFUSAL_LIMIT_S:
                self.stop(f"the attached session refused one record for {REFUSAL_LIMIT_S:.0f} s")
                self._inject_closed_notice()
                return False
            if self.stopping.wait(REFUSAL_RETRY_S):
                return False
        return True

    def _deliver(self) -> None:
        while (line := self.lines.get()) is not None:
            self.saw_closed = self.saw_closed or line.startswith('{"event":"closed"')
            try:
                committed = self._commit(RECORD_PREFIX + line)
            except Exception as exc:
                logger.error("cafleet_listen: delivery to session=%s failed: %r", self.session_key, exc)
                self.stop(f"the delivery failed: {exc!r}")
                self._inject_closed_notice()
                break
            if not committed:
                break
        else:
            self.proc.wait()
            if not self.saw_closed and not self.stopping.is_set():
                self._inject_closed_notice()
        self.stopping.set()
        with _receivers_lock:
            if _receivers.get(self.session_key) is self:
                del _receivers[self.session_key]

    def _inject_closed_notice(self, reason: str = "") -> None:
        if self.notice_attempted:
            return
        self.notice_attempted = True
        self.stderr_thread.join(timeout=EXIT_CONFIRM_S)
        cause = f"{reason}: " if reason else ""
        try:
            self._inject(f"{CLOSED_PREFIX}{cause}exit {self.proc.poll()}: {self.last_stderr}")
        except Exception as exc:
            logger.error("cafleet_listen: closed notice for session=%s failed: %r", self.session_key, exc)

    def unload(self) -> None:
        """Stop for a plugin unload, then tell a still-attached session on a separate thread.

        Unload callbacks run under the plugin manager's discovery lock, so the notice waits outside them."""
        self.stop("the plugin unloaded")
        self._spawn_thread(self._notify_unloaded, "unload")

    def _notify_unloaded(self) -> None:
        self.deliver_thread.join()
        try:
            attached = _session_attached(self.session_key)
        except Exception as exc:
            logger.error("cafleet_listen: gateway.session_authorities.authority_for_home: %s", exc)
            return
        if attached:
            self._inject_closed_notice("the plugin unloaded")

    def _pump_stderr(self) -> None:
        for raw in self.proc.stderr:
            line = raw.strip()
            if line:
                self.last_stderr = line

    def _watch(self) -> None:
        detached_since = None
        while not self.stopping.wait(WATCHDOG_INTERVAL_S):
            try:
                attached = _session_attached(self.session_key)
            except Exception as exc:
                logger.error("cafleet_listen: gateway.session_authorities.authority_for_home: %s", exc)
                self.stop(f"client attachment unreadable: {exc}")
                return
            if attached:
                detached_since = None
                continue
            detached_since = detached_since or time.monotonic()
            if time.monotonic() - detached_since >= DETACH_GRACE_S:
                self.stop(f"no client attached for {DETACH_GRACE_S:.0f} s")
                return


def _listen(args: dict, **kwargs) -> str:
    director_id = int(args["director_id"])
    database_url = args.get("database_url")
    try:
        session_key = _session_key()
    except Exception as exc:
        return _result(ok=False, error=f"hermes_cli.plugin_session.session_key: {exc}")
    if not session_key:
        return _result(ok=False, error="cafleet_listen must be called from a session turn")
    try:
        _session_attached(session_key)
    except Exception as exc:
        return _result(ok=False, error=f"gateway.session_authorities.authority_for_home: {exc}")
    try:
        _injection_target()
    except Exception as exc:
        return _result(ok=False, error=f"{INJECTION_SEAM}: {exc}")
    with _receivers_lock:
        existing = _receivers.get(session_key)
        if existing is not None and existing.alive():
            return _result(ok=True, pid=existing.proc.pid, session_key=session_key, existing=True)
        try:
            executable = _recorded_executable()
        except ValueError as exc:
            return _result(ok=False, error=str(exc))
        if not _ctx._gateway_injection_allowed():
            return _result(ok=False, error=CONSENT_ERROR)
        refusal = _director_refusal(executable, director_id, database_url)
        if refusal is not None:
            return _result(ok=False, error=refusal)
        proc = subprocess.Popen(
            [str(executable), *_database_args(database_url), "message", "listen", str(director_id)],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            encoding="utf-8", errors="replace", bufsize=1, env=_cafleet_env(),
        )
        receiver = Receiver(session_key, str(kwargs.get("session_id") or ""), proc)
        _receivers[session_key] = receiver
    receiver.start()
    return _result(ok=True, pid=proc.pid, session_key=session_key)


def _finalized_route(session_id: str) -> str:
    """The route of a finalized session, which survives the session-id rotation of a compression."""
    if not session_id:
        return ""
    try:
        live = _authority().sessions.get(session_id)
    except Exception as exc:
        logger.error("cafleet_listen: gateway.session_authorities.authority_for_home: %s", exc)
        return ""
    return "" if live is None else live.route


def _on_session_finalize(**kwargs) -> None:
    session_id = str(kwargs.get("session_id") or "")
    try:
        session_key = _session_key()
    except Exception:
        session_key = ""
    keys = {key for key in (_finalized_route(session_id), session_key) if key}
    with _receivers_lock:
        owned = [r for r in _receivers.values()
                 if r.session_key in keys or (session_id and r.session_id == session_id)]
    for receiver in owned:
        receiver.stop("the session finalized")


def _on_unload() -> None:
    with _receivers_lock:
        receivers = list(_receivers.values())
        _receivers.clear()
    for receiver in receivers:
        receiver.unload()


_SCHEMA = {
    "name": "cafleet_listen",
    "description": (
        "Start CAFleet Director reception for this session: every broker record addressed to the "
        "Director arrives as its own turn beginning '[cafleet listen record]'."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "director_id": {"type": "integer", "description": "The Director's literal member ID."},
            "database_url": {
                "type": "string",
                "description": "The fleet's database URL when the Director's DATABASE URL line names one.",
            },
        },
        "required": ["director_id"],
        "additionalProperties": False,
    },
}


def register(ctx) -> None:
    global _ctx
    _ctx = ctx
    ctx.register_tool(name="cafleet_listen", toolset="cafleet", schema=_SCHEMA, handler=_listen)
    ctx.register_hook("on_session_finalize", _on_session_finalize)
    ctx.on_unload(_on_unload)
