from cafleet.multiplexer.base import (
    Multiplexer,
    MultiplexerContext,
    poll_until_pane_gone,
)
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

MULTIPLEXERS: dict[str, Multiplexer] = {
    "tmux": TmuxMultiplexer(),
}

__all__ = [
    "MULTIPLEXERS",
    "Multiplexer",
    "MultiplexerContext",
    "TmuxError",
    "TmuxMultiplexer",
    "poll_until_pane_gone",
]
