"""Contract tests for the ``Multiplexer`` Protocol and registered impls.

The parametrized ``test_impl_satisfies_protocol`` catches signature drift the
moment a second multiplexer (cmux) lands. The tmux-specific smoke stays
single-impl because the monkeypatch target (``_run``) is impl-private —
when cmux lands, it adds its own parallel smoke test rather than
parametrizing over both impls.
"""

import pytest

from cafleet.multiplexer import MULTIPLEXERS, Multiplexer, MultiplexerContext
from cafleet.multiplexer.tmux import TmuxMultiplexer


@pytest.mark.parametrize(("name", "impl"), list(MULTIPLEXERS.items()))
def test_impl_satisfies_protocol(name, impl):
    """Cross-impl Protocol check — runs against every registered Multiplexer."""
    assert isinstance(impl, Multiplexer)
    assert impl.name == name


def test_tmux_context_discovery_returns_multiplexer_context(monkeypatch):
    """tmux-specific smoke — stays single-impl because the monkeypatch target is tmux-private."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux._run",
        lambda *a, **k: "fake-session|@1|%1",
    )
    monkeypatch.setenv("TMUX", "fake")
    monkeypatch.setenv("TMUX_PANE", "%1")
    ctx = TmuxMultiplexer().context_discovery()
    assert isinstance(ctx, MultiplexerContext)
    assert ctx.session == "fake-session"
    assert ctx.window_id == "@1"
    assert ctx.pane_id == "%1"
