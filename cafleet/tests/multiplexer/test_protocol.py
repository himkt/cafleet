"""Contract tests for the ``Multiplexer`` Protocol and registered impls.

The parametrized ``test_impl_satisfies_protocol`` catches signature drift the
moment a second multiplexer (cmux) lands. The tmux-specific smoke stays
single-impl because the monkeypatch target (``_run``) is impl-private —
when cmux lands, it adds its own parallel smoke test rather than
parametrizing over both impls.
"""

import pytest

from cafleet.multiplexer import (
    MULTIPLEXERS,
    AgentStateAware,
    Multiplexer,
    MultiplexerContext,
)
from cafleet.multiplexer.herdr import HerdrMultiplexer
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


def test_protocol_declares_list_pane_ids():
    """The per-tick pane-liveness query is part of the Multiplexer contract."""
    assert hasattr(Multiplexer, "list_pane_ids")


def test_protocol_declares_send_wake_trigger():
    """The monitoring-member wake keystroke is part of the Multiplexer contract."""
    assert hasattr(Multiplexer, "send_wake_trigger")


def test_herdr_satisfies_multiplexer_and_agent_state_aware():
    """herdr implements the base Multiplexer Protocol AND the optional
    AgentStateAware capability (native lifecycle-state reads)."""
    herdr = HerdrMultiplexer()
    assert isinstance(herdr, Multiplexer)
    assert isinstance(herdr, AgentStateAware)
    assert herdr.name == "herdr"


def test_tmux_satisfies_multiplexer_but_not_agent_state_aware():
    """tmux has no native agent state, so it satisfies the base Protocol but NOT
    AgentStateAware — the `isinstance(mux, AgentStateAware)` guard is false and
    the monitor's native-status branch never runs on the tmux path."""
    tmux = TmuxMultiplexer()
    assert isinstance(tmux, Multiplexer)
    assert not isinstance(tmux, AgentStateAware)


def test_registered_multiplexers_include_tmux_and_herdr():
    """The registry exposes both backends under their canonical keys."""
    assert set(MULTIPLEXERS) == {"tmux", "herdr"}
    assert isinstance(MULTIPLEXERS["herdr"], HerdrMultiplexer)


def test_tmux_list_pane_ids_returns_set_of_pane_ids(monkeypatch):
    """``list_pane_ids`` splits ``tmux list-panes -a`` output into a set."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux._run",
        lambda *a, **k: "%0\n%1\n%2\n",
    )
    assert TmuxMultiplexer().list_pane_ids() == {"%0", "%1", "%2"}
