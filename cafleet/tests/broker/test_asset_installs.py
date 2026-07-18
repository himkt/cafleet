"""Tests for ``cafleet.broker.asset_installs`` (version-recording helpers)."""

from datetime import UTC, datetime

from sqlalchemy import create_engine, text
from sqlalchemy.orm import sessionmaker

from cafleet.broker import _shared
from cafleet.db.models import AssetInstall, Base


def test_table_exists_true():
    from cafleet.broker.asset_installs import asset_installs_table_exists

    assert asset_installs_table_exists() is True


def test_table_exists_false_when_table_missing(monkeypatch):
    """A pre-``asset_installs`` schema (or an empty DB) reports False."""
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    with engine.begin() as conn:
        conn.execute(text("DROP TABLE IF EXISTS asset_installs"))
    monkeypatch.setattr(_shared, "get_sync_sessionmaker", lambda: sessionmaker(engine))

    from cafleet.broker.asset_installs import asset_installs_table_exists

    assert asset_installs_table_exists() is False


def test_list_asset_installs_empty(broker_session):
    from cafleet.broker.asset_installs import list_asset_installs

    # The shared fixture seeds one row (guard contract); build an empty state.
    with broker_session() as session, session.begin():
        session.query(AssetInstall).delete()

    assert list_asset_installs() == []


def test_record_asset_install_row_shape():
    from cafleet.broker.asset_installs import (
        list_asset_installs,
        record_asset_install,
    )

    record_asset_install("claude", "0.6.0")

    rows = list_asset_installs()
    assert len(rows) == 1
    row = rows[0]
    assert set(row.keys()) == {"coding_agent", "cafleet_version", "installed_at"}
    assert row["coding_agent"] == "claude"
    assert row["cafleet_version"] == "0.6.0"

    # installed_at follows the now_iso() convention: UTC ISO-8601.
    parsed = datetime.fromisoformat(row["installed_at"])
    assert parsed.tzinfo is not None
    assert parsed.utcoffset() == UTC.utcoffset(None)


def test_list_asset_installs_ordered_by_coding_agent():
    from cafleet.broker.asset_installs import (
        list_asset_installs,
        record_asset_install,
    )

    record_asset_install("codex", "0.6.0")
    record_asset_install("opencode", "0.6.0")
    record_asset_install("claude", "0.6.0")

    rows = list_asset_installs()
    assert [row["coding_agent"] for row in rows] == ["claude", "codex", "opencode"]


def test_record_asset_install_upserts_per_home():
    """Re-installing replaces the row for that home instead of adding one."""
    from cafleet.broker.asset_installs import (
        list_asset_installs,
        record_asset_install,
    )

    record_asset_install("claude", "0.5.0")
    first = list_asset_installs()[0]

    record_asset_install("claude", "0.6.0")

    rows = list_asset_installs()
    assert len(rows) == 1
    assert rows[0]["cafleet_version"] == "0.6.0"
    assert rows[0]["installed_at"] >= first["installed_at"]
