"""Tests for ``cafleet.broker.skill_installs`` (version-recording helpers)."""

from datetime import UTC, datetime

import pytest
from sqlalchemy import create_engine, text
from sqlalchemy.orm import sessionmaker

from cafleet.broker import _shared
from cafleet.db.models import Base


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def test_table_exists_true():
    from cafleet.broker.skill_installs import skill_installs_table_exists

    assert skill_installs_table_exists() is True


def test_table_exists_false_when_table_missing(monkeypatch):
    """A pre-``skill_installs`` schema (or an empty DB) reports False."""
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    with engine.begin() as conn:
        conn.execute(text("DROP TABLE IF EXISTS skill_installs"))
    monkeypatch.setattr(_shared, "get_sync_sessionmaker", lambda: sessionmaker(engine))

    from cafleet.broker.skill_installs import skill_installs_table_exists

    assert skill_installs_table_exists() is False


def test_list_skill_installs_empty():
    from cafleet.broker.skill_installs import list_skill_installs

    assert list_skill_installs() == []


def test_record_skill_install_row_shape():
    from cafleet.broker.skill_installs import (
        list_skill_installs,
        record_skill_install,
    )

    record_skill_install("claude", "0.6.0")

    rows = list_skill_installs()
    assert len(rows) == 1
    row = rows[0]
    assert set(row.keys()) == {"coding_agent", "cafleet_version", "installed_at"}
    assert row["coding_agent"] == "claude"
    assert row["cafleet_version"] == "0.6.0"

    # installed_at follows the now_iso() convention: UTC ISO-8601.
    parsed = datetime.fromisoformat(row["installed_at"])
    assert parsed.tzinfo is not None
    assert parsed.utcoffset() == UTC.utcoffset(None)


def test_list_skill_installs_ordered_by_coding_agent():
    from cafleet.broker.skill_installs import (
        list_skill_installs,
        record_skill_install,
    )

    record_skill_install("codex", "0.6.0")
    record_skill_install("opencode", "0.6.0")
    record_skill_install("claude", "0.6.0")

    rows = list_skill_installs()
    assert [row["coding_agent"] for row in rows] == ["claude", "codex", "opencode"]


def test_record_skill_install_upserts_per_home():
    """Re-installing replaces the row for that home instead of adding one."""
    from cafleet.broker.skill_installs import (
        list_skill_installs,
        record_skill_install,
    )

    record_skill_install("claude", "0.5.0")
    first = list_skill_installs()[0]

    record_skill_install("claude", "0.6.0")

    rows = list_skill_installs()
    assert len(rows) == 1
    assert rows[0]["cafleet_version"] == "0.6.0"
    assert rows[0]["installed_at"] >= first["installed_at"]
