import importlib.metadata

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet.broker import _shared
from cafleet.db.models import Base, SkillInstall
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
    sm = sessionmaker(engine, expire_on_commit=False)
    # Seed a current skills install so fleet-scoped CLI tests pass the guard.
    with sm() as session, session.begin():
        session.add(
            SkillInstall(
                coding_agent="claude",
                cafleet_version=importlib.metadata.version("cafleet"),
                installed_at=_shared.now_iso(),
            )
        )
    return sm


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    """Redirect ``cafleet.broker._shared.get_sync_sessionmaker`` at the in-memory sessionmaker."""
    monkeypatch.setattr(_shared, "get_sync_sessionmaker", lambda: sync_sessionmaker)


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
