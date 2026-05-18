import shutil
from typing import Protocol, runtime_checkable


@runtime_checkable
class CodingAgent(Protocol):
    """Coding-agent binary that runs inside a multiplexer pane."""

    @property
    def name(self) -> str:
        """Registry key — matches the ``placement.coding_agent`` column value."""
        ...

    @property
    def binary_name(self) -> str:
        """Executable name resolved via ``shutil.which``."""
        ...

    def ensure_available(self) -> None:
        """Raise RuntimeError if ``binary_name`` is not on PATH."""
        ...

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        """Return the argv list passed to the multiplexer's ``split_window``.

        ``display_name`` is honored by backends that support a pane-title flag
        (claude) and silently ignored by those that do not (codex).
        """
        ...


def ensure_binary_on_path(binary_name: str) -> None:
    """Shared availability check used by every CodingAgent impl."""
    if shutil.which(binary_name) is None:
        raise RuntimeError(f"binary {binary_name} not found on PATH")
