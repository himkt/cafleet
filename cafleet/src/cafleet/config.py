"""Project-wide settings sourced from the ``CAFLEET_*`` environment block.

The :class:`Settings` singleton (``settings``) is imported by the broker,
CLI, server, and multiplexer modules. Every field reads from a single
explicit ``CAFLEET_*`` env var via ``validation_alias`` — no implicit
prefix-based binding — and falls back to a hard-coded default suitable for
local single-user use.
"""

from pathlib import Path

from pydantic import Field
from pydantic_settings import BaseSettings


def _default_database_url() -> str:
    """Return the default SQLite URL under ``~/.local/share/cafleet/``."""
    db_path = Path("~/.local/share/cafleet/cafleet_v5.db").expanduser()
    return f"sqlite:///{db_path}"


class Settings(BaseSettings):
    """Runtime configuration for the CAFleet broker, CLI, and server.

    Every attribute is read from an explicit ``CAFLEET_*`` environment
    variable and defaults to a value suitable for local single-user use.
    Modify via the env block; do NOT mutate the ``settings`` singleton at
    runtime.

    Attributes:
        database_url: SQLAlchemy URL for the SQLite registry. Sourced from
            ``CAFLEET_DATABASE_URL``; defaults to
            ``sqlite:///~/.local/share/cafleet/cafleet_v5.db`` (the home
            directory is expanded at import time). Use an absolute path —
            SQLAlchemy does not expand ``~`` in SQLite URLs.
        broker_host: Bind host for ``cafleet server``. Sourced from
            ``CAFLEET_BROKER_HOST``; defaults to ``"127.0.0.1"``.
        broker_port: Bind port for ``cafleet server``. Sourced from
            ``CAFLEET_BROKER_PORT``; defaults to ``8000``.
        max_text_len: Codepoint truncation limit applied when rendering
            message bodies (CLI echo and broker inline-preview keystroke).
            Sourced from ``CAFLEET_MAX_TEXT_LEN``; defaults to ``200``. The
            persisted ``Message.text`` column is never truncated, and the
            WebUI API returns raw broker dicts that do not apply this
            limit.
        multiplexer: Explicit terminal-multiplexer backend override consumed
            by ``resolve_multiplexer``. Sourced from ``CAFLEET_MULTIPLEXER``;
            ``None`` (the default) means auto-detect from ``HERDR_ENV`` /
            ``TMUX`` — a valid, well-defined state, not a fallback. A set value
            must name a registered backend (``tmux``/``herdr``).
        monitor_stall_interval: Per-member stall-check cadence in seconds,
            driven by the monitor loop independently of the
            ``monitor_config.interval_seconds`` ping intervals. Sourced from
            ``CAFLEET_MONITOR_STALL_INTERVAL``; defaults to ``240``. A watched
            member is stall-check due every ``monitor_stall_interval`` seconds;
            ``0`` disables stall detection entirely (no ``stall-check`` wake
            reason is emitted). A successful watcher wake persists its dispatch
            time in ``monitor_config.last_stall_check_at``, so a restarted
            monitor honors the remaining cadence.
    """

    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="CAFLEET_DATABASE_URL",
    )
    broker_host: str = Field(
        default="127.0.0.1",
        validation_alias="CAFLEET_BROKER_HOST",
    )
    broker_port: int = Field(
        default=8000,
        validation_alias="CAFLEET_BROKER_PORT",
    )
    max_text_len: int = Field(
        default=200,
        validation_alias="CAFLEET_MAX_TEXT_LEN",
    )
    multiplexer: str | None = Field(
        default=None,
        validation_alias="CAFLEET_MULTIPLEXER",
    )
    monitor_stall_interval: int = Field(
        default=240,
        validation_alias="CAFLEET_MONITOR_STALL_INTERVAL",
    )


settings = Settings()
