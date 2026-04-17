import pytest

from cafleet import tmux


@pytest.fixture(autouse=True)
def _silence_real_tmux_subprocess(monkeypatch):
    """Stub tmux._run so tests never send-keys into a real pane."""
    monkeypatch.setattr(tmux, "_run", lambda *args, **kwargs: "")
