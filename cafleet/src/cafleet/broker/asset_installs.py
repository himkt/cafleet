"""Assets-install version recording; see ``docs/spec/data-model.md``."""

from sqlalchemy import inspect, select
from sqlalchemy.exc import OperationalError

from cafleet.broker._shared import now_iso, read_session, write_session
from cafleet.db.models import AssetInstall


def asset_installs_table_exists() -> bool:
    """Report whether the ``asset_installs`` table is reachable.

    An unopenable database (missing file or parent directory) means the
    schema was never created, so it reports ``False`` rather than raising.
    """
    try:
        with read_session() as session:
            return inspect(session.get_bind()).has_table("asset_installs")
    except OperationalError:
        return False


def list_asset_installs() -> list[dict]:
    with read_session() as session:
        rows = session.execute(
            select(
                AssetInstall.coding_agent,
                AssetInstall.cafleet_version,
                AssetInstall.installed_at,
            ).order_by(AssetInstall.coding_agent)
        ).all()
    return [
        {
            "coding_agent": row.coding_agent,
            "cafleet_version": row.cafleet_version,
            "installed_at": row.installed_at,
        }
        for row in rows
    ]


def record_asset_install(coding_agent: str, cafleet_version: str) -> None:
    with write_session() as session:
        session.merge(
            AssetInstall(
                coding_agent=coding_agent,
                cafleet_version=cafleet_version,
                installed_at=now_iso(),
            )
        )
