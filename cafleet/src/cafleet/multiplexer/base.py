import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, runtime_checkable


@dataclass(frozen=True)
class MultiplexerContext:
    """Resolved pane identity, returned by ``Multiplexer.context_discovery()``."""

    session: str
    window_id: str
    pane_id: str


@runtime_checkable
class Multiplexer(Protocol):
    """Terminal multiplexer that hosts coding-agent panes."""

    @property
    def name(self) -> str:
        """Registry key (e.g. ``"tmux"``)."""
        ...

    def ensure_available(self) -> None: ...

    def context_discovery(self) -> MultiplexerContext: ...

    def split_window(
        self,
        *,
        target_window_id: str,
        env: dict[str, str],
        command: list[str],
    ) -> str: ...

    def kill_pane(
        self, *, target_pane_id: str, ignore_missing: bool = False
    ) -> None: ...

    def pane_exists(self, *, target_pane_id: str) -> bool: ...

    def wait_for_pane_gone(
        self,
        *,
        target_pane_id: str,
        timeout: float = 15.0,
        interval: float = 0.5,
    ) -> bool: ...

    def send_exit(
        self, *, target_pane_id: str, ignore_missing: bool = False
    ) -> None: ...

    def send_poll_trigger(
        self, *, target_pane_id: str, session_id: str, agent_id: str
    ) -> bool: ...

    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        task_id_8: str,
        sender_8: str,
        ts: str,
        text: str,
    ) -> bool: ...

    def send_choice_key(self, *, target_pane_id: str, digit: int) -> None: ...

    def send_freetext_and_submit(self, *, target_pane_id: str, text: str) -> None: ...

    def send_bash_command(self, *, target_pane_id: str, command: str) -> None: ...

    def capture_pane(self, *, target_pane_id: str, lines: int = 30) -> str: ...


def poll_until_pane_gone(
    pane_exists_fn: Callable[[], bool],
    *,
    timeout: float,
    interval: float,
) -> bool:
    """Generic poll-until-False helper for any Multiplexer's ``wait_for_pane_gone``."""
    deadline = time.monotonic() + timeout
    while True:
        if not pane_exists_fn():
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
