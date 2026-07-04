from cafleet.multiplexer.base import Multiplexer, MultiplexerContext
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
]
