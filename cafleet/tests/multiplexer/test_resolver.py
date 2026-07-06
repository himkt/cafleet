"""Tests for ``resolve_multiplexer`` — backend selection precedence (§3).

Precedence: (1) an explicit ``CAFLEET_MULTIPLEXER`` override (must be a registry
key), then (2) auto-detect from ``HERDR_ENV`` / ``TMUX``, with both-set and
neither-set failing loudly. The autouse ``_pin_multiplexer_tmux`` fixture pins
the override to ``tmux`` for the rest of the suite; every test here sets
``settings.multiplexer`` explicitly (``None`` to exercise auto-detect) so it is
independent of that default and of the host environment.
"""

import pytest

from cafleet.config import settings
from cafleet.multiplexer import (
    MULTIPLEXERS,
    HerdrMultiplexer,
    MultiplexerError,
    TmuxMultiplexer,
    resolve_multiplexer,
)


@pytest.fixture
def clear_mux_env(monkeypatch):
    """Start every resolver test from a known-empty detection environment."""
    monkeypatch.delenv("HERDR_ENV", raising=False)
    monkeypatch.delenv("TMUX", raising=False)


@pytest.mark.parametrize(
    ("override", "expected_type"),
    [("tmux", TmuxMultiplexer), ("herdr", HerdrMultiplexer)],
)
def test_resolve__valid_override_wins_over_environment(
    monkeypatch, clear_mux_env, override, expected_type
):
    monkeypatch.setattr(settings, "multiplexer", override)
    # Even a conflicting environment is ignored when the override is explicit.
    monkeypatch.setenv("HERDR_ENV", "1")
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,1,0")
    mux = resolve_multiplexer()
    assert mux is MULTIPLEXERS[override]
    assert isinstance(mux, expected_type)


def test_resolve__invalid_override_raises(monkeypatch, clear_mux_env):
    monkeypatch.setattr(settings, "multiplexer", "screen")
    with pytest.raises(MultiplexerError) as exc_info:
        resolve_multiplexer()
    message = str(exc_info.value)
    assert "CAFLEET_MULTIPLEXER='screen' is not a supported multiplexer" in message
    # The error lists the registry keys, sorted.
    assert "herdr, tmux" in message


def test_resolve__autodetect_herdr(monkeypatch, clear_mux_env):
    monkeypatch.setattr(settings, "multiplexer", None)
    monkeypatch.setenv("HERDR_ENV", "1")
    assert resolve_multiplexer() is MULTIPLEXERS["herdr"]


def test_resolve__autodetect_tmux(monkeypatch, clear_mux_env):
    monkeypatch.setattr(settings, "multiplexer", None)
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,1,0")
    assert resolve_multiplexer() is MULTIPLEXERS["tmux"]


def test_resolve__both_present_is_ambiguous_error(monkeypatch, clear_mux_env):
    monkeypatch.setattr(settings, "multiplexer", None)
    monkeypatch.setenv("HERDR_ENV", "1")
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,1,0")
    with pytest.raises(
        MultiplexerError,
        match="ambiguous multiplexer environment: both HERDR_ENV and TMUX are set",
    ):
        resolve_multiplexer()


def test_resolve__neither_present_is_empty_error(monkeypatch, clear_mux_env):
    monkeypatch.setattr(settings, "multiplexer", None)
    with pytest.raises(
        MultiplexerError,
        match="no supported multiplexer detected: neither HERDR_ENV nor TMUX is set",
    ):
        resolve_multiplexer()
