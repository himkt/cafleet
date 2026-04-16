"""Shared pytest fixtures.

Autouse fixtures defined here apply to every test in ``cafleet/tests/``.
Individual tests can still override the monkeypatched attribute by calling
``monkeypatch.setattr`` themselves — function-scoped monkeypatch ordering
makes the later call win.
"""

import pytest

from cafleet import tmux


@pytest.fixture(autouse=True)
def _silence_real_tmux_subprocess(monkeypatch):
    """Replace ``tmux._run`` with a no-op for every test by default.

    Without this, any test that calls ``broker.send_message`` (or a CLI path
    that broadcasts) triggers ``_try_notify_recipient`` → ``send_poll_trigger``
    → the real ``subprocess.run(["tmux", "send-keys", ...])``. When the
    broker tests reuse pane IDs like ``%0`` / ``%2`` / ``%3`` (from the
    shared autouse fixtures that mock ``director_context``), those sends
    hit whichever real tmux pane currently owns that ID — spraying ``cafleet
    poll ...`` invocations into a developer's interactive session.

    Tests that actually exercise ``_run`` (``test_tmux.py``,
    ``test_tmux_send_helpers.py``) already call ``monkeypatch.setattr(tmux,
    "_run", mock_run)`` themselves; that later setattr replaces this
    autouse stub for the duration of the test, so those assertions keep
    working.
    """
    monkeypatch.setattr(tmux, "_run", lambda *args, **kwargs: "")
