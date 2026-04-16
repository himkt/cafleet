"""Shared pytest fixtures for ``cafleet/tests/``."""

import pytest

from cafleet import tmux


@pytest.fixture(autouse=True)
def _silence_real_tmux_subprocess(monkeypatch):
    """Stub ``tmux._run`` so tests never spray send-keys into a real pane.

    Broker tests reuse pane IDs like ``%0`` / ``%2`` (from the shared
    ``director_context`` mocks); without this stub any call that ends up
    in ``_try_notify_recipient`` → ``send_poll_trigger`` would fire at
    whichever real pane currently owns that id. Tests that want to observe
    ``_run`` replace this stub with their own ``monkeypatch.setattr``.
    """
    monkeypatch.setattr(tmux, "_run", lambda *args, **kwargs: "")
