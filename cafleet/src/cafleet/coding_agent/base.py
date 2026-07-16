"""Registry contract for coding-agent backends (claude, codex, opencode).

The :class:`CodingAgent` Protocol defines the surface every backend must
implement; the concrete impls live alongside this module
(``cafleet.coding_agent.claude``, ``...codex``, ``...opencode``) and are
selected via the ``placement.coding_agent`` column. The Director picks a
backend per member at create time and the multiplexer spawns the resulting
argv inside a fresh pane.
"""

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
        """Raise if any spawn precondition is unmet.

        Preconditions covered by impls include: the backend binary is on
        ``PATH``, and (for backends with bundled presets) the preset file
        exists on disk.

        Raises:
            RuntimeError: If any precondition is unmet.
        """
        ...

    def validate_model(self, model: str | None) -> None:
        """Raise ValueError if ``model`` is not acceptable to this backend.

        ``None`` (flag omitted) is always valid. Called by ``member create``
        before any registration or tmux side effect.

        Raises:
            ValueError: If ``model`` violates the backend's format rules.
        """
        ...

    def build_spawn_argv(
        self, prompt: str, *, display_name: str, model: str | None = None
    ) -> list[str]:
        """Return the argv list passed to the multiplexer's ``split_window``.

        Args:
            prompt: The initial prompt text the backend should be launched
                with (e.g. the member's role-defining spawn prompt).
            display_name: Pane title. Honored by backends that support a
                pane-title flag (currently only ``claude`` via ``--name``)
                and silently ignored by every other backend (``codex``,
                ``opencode``) because neither has a pane-title-equivalent
                flag.
            model: Model forwarded to the binary via its ``--model`` flag,
                emitted immediately before the prompt tokens. ``None`` (flag
                omitted) emits no model tokens, keeping the argv byte-identical
                to the no-model form so the binary uses its own default model.

        Returns:
            argv list ready to hand to ``Multiplexer.split_window``.
        """
        ...


def ensure_binary_on_path(binary_name: str) -> None:
    """Shared availability check used by every CodingAgent impl.

    Args:
        binary_name: Executable name to resolve via ``shutil.which``.

    Raises:
        RuntimeError: If ``binary_name`` is not on ``PATH``.
    """
    if shutil.which(binary_name) is None:
        raise RuntimeError(f"binary {binary_name} not found on PATH")
