import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.db.models import Base
from cafleet.multiplexer import tmux as multiplexer_tmux


@pytest.fixture(autouse=True)
def _silence_real_tmux_subprocess(monkeypatch):
    """Stub ``cafleet.multiplexer.tmux._run`` so tests never send-keys into a real pane."""
    monkeypatch.setattr(multiplexer_tmux, "_run", lambda *args, **kwargs: "")


@pytest.fixture
def sync_sessionmaker():
    """Per-test in-memory SQLite + sessionmaker with the full broker schema."""
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    """Redirect ``broker.get_sync_sessionmaker`` at the in-memory sessionmaker."""
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture
def broker_session(sync_sessionmaker, _patch_broker):
    """Composite fixture: patched sessionmaker, ready for broker.* calls."""
    return sync_sessionmaker


@pytest.fixture
def _reset_engine_singletons():
    """Reset the cafleet.db.engine module-level singletons around each test."""
    cafleet.db.engine._sync_engine = None
    cafleet.db.engine._sync_sessionmaker = None
    yield
    cafleet.db.engine._sync_engine = None
    cafleet.db.engine._sync_sessionmaker = None
