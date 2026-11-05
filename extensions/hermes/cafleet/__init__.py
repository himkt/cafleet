"""CAFleet Director reception: push each `cafleet message listen` record into the owning session."""

from __future__ import annotations

import json
import logging
import os
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

_ctx = None
_receivers: dict[str, "Receiver"] = {}
_receivers_lock = threading.Lock()


def _result(**fields) -> str:
    return json.dumps(fields)


def _session_key() -> str:
    from hermes_cli.plugin_session import session_key
    return session_key()


def _session_env(name: str) -> str:
    from gateway.session_context import get_session_env
    return get_session_env(name, "") or ""


def _session_attached(session_key: str) -> bool:
    from gateway.session_authorities import authority_for_home
    manager = _ctx._manager
    injector = manager._gateway_message_injector
    if injector is None:
        raise RuntimeError("no live gateway serves this plugin")
    authority = authority_for_home(injector[0], manager.home_path)
    if authority is None:
        raise RuntimeError(f"no session authority serves {manager.home_path}")
    return any(live.route == session_key and live.subscribers for live in list(authority.sessions.values()))


def _recorded_executable() -> Path:
    record = Path(__file__).with_name("executable")
    try:
        target = Path(record.read_text(encoding="utf-8").strip())
    except OSError as exc:
        raise ValueError(f"executable record {record}: {exc}; {SETUP_FIX}") from None
    if not (target.is_file() and os.access(target, os.X_OK)):
        raise ValueError(f"executable record {record}: {target} is not an executable file; {SETUP_FIX}")
    return target


class Receiver:
    def __init__(self, session_key: str, session_id: str, proc: subprocess.Popen):
        self.session_key = session_key
        self.session_id = session_id
        self.proc = proc
        self.stopping = threading.Event()
        self.saw_closed = False
        self.last_stderr = ""

    def alive(self) -> bool:
        return self.proc.poll() is None

    def start(self) -> None:
        for target, name in ((self._pump_stdout, "stdout"), (self._pump_stderr, "stderr"), (self._watch, "watchdog")):
            threading.Thread(target=target, daemon=True, name=f"cafleet-listen-{name}-{self.proc.pid}").start()

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
        return bool(_ctx.inject_message(text, session_key=self.session_key))

    def _pump_stdout(self) -> None:
        for raw in self.proc.stdout:
            line = raw.rstrip("\n")
            if not line:
                continue
            self.saw_closed = self.saw_closed or line.startswith('{"event":"closed"')
            if not self._inject(RECORD_PREFIX + line):
                self.stop("the session refused the injection")
                break
        code = self.proc.wait()
        if not self.saw_closed and not self.stopping.is_set():
            self._inject(f"{CLOSED_PREFIX}exit {code}: {self.last_stderr}")
        self.stopping.set()
        with _receivers_lock:
            if _receivers.get(self.session_key) is self:
                del _receivers[self.session_key]

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
    try:
        member_id = _session_env("CAFLEET_MEMBER_ID")
    except Exception as exc:
        return _result(ok=False, error=f"gateway.session_context.get_session_env: {exc}")
    if member_id:
        return _result(ok=False, error="cafleet_listen runs only in a Director session")
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
    with _receivers_lock:
        existing = _receivers.get(session_key)
        if existing is not None and existing.alive():
            return _result(ok=True, pid=existing.proc.pid, session_key=session_key, existing=True)
        try:
            executable = _recorded_executable()
        except ValueError as exc:
            return _result(ok=False, error=str(exc))
        env = dict(os.environ)
        database_url = _session_env("CAFLEET_DATABASE_URL")
        if database_url:
            env["CAFLEET_DATABASE_URL"] = database_url
        proc = subprocess.Popen(
            [str(executable), "message", "listen", str(int(args["director_id"]))],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, bufsize=1, env=env,
        )
        receiver = Receiver(session_key, str(kwargs.get("session_id") or ""), proc)
        _receivers[session_key] = receiver
    receiver.start()
    return _result(ok=True, pid=proc.pid, session_key=session_key)


def _on_session_finalize(**kwargs) -> None:
    session_id = str(kwargs.get("session_id") or "")
    try:
        session_key = _session_key()
    except Exception:
        session_key = ""
    with _receivers_lock:
        owned = [r for r in _receivers.values()
                 if (session_id and r.session_id == session_id) or (session_key and r.session_key == session_key)]
    for receiver in owned:
        receiver.stop("the session finalized")


_SCHEMA = {
    "name": "cafleet_listen",
    "description": (
        "Start CAFleet Director reception for this session: every broker record addressed to the "
        "Director arrives as its own turn beginning '[cafleet listen record]'."
    ),
    "parameters": {
        "type": "object",
        "properties": {"director_id": {"type": "integer", "description": "The Director's literal member ID."}},
        "required": ["director_id"],
        "additionalProperties": False,
    },
}


def register(ctx) -> None:
    global _ctx
    _ctx = ctx
    ctx.register_tool(name="cafleet_listen", toolset="cafleet", schema=_SCHEMA, handler=_listen)
    ctx.register_hook("on_session_finalize", _on_session_finalize)
