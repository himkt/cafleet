import os

from cafleet.config import settings
from cafleet.multiplexer.base import (
    AgentStateAware,
    Multiplexer,
    MultiplexerContext,
    MultiplexerError,
)
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

MULTIPLEXERS: dict[str, Multiplexer] = {
    "tmux": TmuxMultiplexer(),
}


def resolve_multiplexer() -> Multiplexer:
    """Resolve the active terminal-multiplexer backend.

    Precedence:

    1. Explicit ``CAFLEET_MULTIPLEXER`` override — must name a registered
       backend, else raise.
    2. Auto-detect from the environment: ``HERDR_ENV`` truthy → herdr,
       ``TMUX`` set → tmux.
    3. Both present (ambiguous) or neither present (empty) → raise.

    An unset override (auto-detect) is a legitimate default; the override is
    the deterministic escape hatch.

    Raises:
        MultiplexerError: On an unknown override, an ambiguous environment, or
            no detected backend.
    """
    override = settings.multiplexer
    if override is not None:
        if override not in MULTIPLEXERS:
            raise MultiplexerError(
                f"CAFLEET_MULTIPLEXER={override!r} is not a supported multiplexer "
                f"(expected one of: {', '.join(sorted(MULTIPLEXERS))})"
            )
        return MULTIPLEXERS[override]
    in_herdr = bool(os.environ.get("HERDR_ENV"))
    in_tmux = bool(os.environ.get("TMUX"))
    if in_herdr and in_tmux:
        raise MultiplexerError(
            "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; "
            "set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate"
        )
    if in_herdr:
        return MULTIPLEXERS["herdr"]
    if in_tmux:
        return MULTIPLEXERS["tmux"]
    raise MultiplexerError(
        "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; "
        "run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER"
    )


__all__ = [
    "MULTIPLEXERS",
    "AgentStateAware",
    "Multiplexer",
    "MultiplexerContext",
    "MultiplexerError",
    "TmuxError",
    "TmuxMultiplexer",
    "resolve_multiplexer",
]
