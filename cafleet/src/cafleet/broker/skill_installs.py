"""Skills-install version recording; see ``docs/spec/data-model.md``."""

from sqlalchemy import inspect, select
from sqlalchemy.exc import OperationalError

from cafleet.broker._shared import now_iso, read_session, write_session
from cafleet.db.models import SkillInstall


def skill_installs_table_exists() -> bool:
    """Report whether the ``skill_installs`` table is reachable.

    An unopenable database (missing file or parent directory) means the
    schema was never created, so it reports ``False`` rather than raising.
    """
    try:
        with read_session() as session:
            return inspect(session.get_bind()).has_table("skill_installs")
    except OperationalError:
        return False


def list_skill_installs() -> list[dict]:
    with read_session() as session:
        rows = session.execute(
            select(
                SkillInstall.coding_agent,
                SkillInstall.cafleet_version,
                SkillInstall.installed_at,
            ).order_by(SkillInstall.coding_agent)
        ).all()
    return [
        {
            "coding_agent": row.coding_agent,
            "cafleet_version": row.cafleet_version,
            "installed_at": row.installed_at,
        }
        for row in rows
    ]


def record_skill_install(coding_agent: str, cafleet_version: str) -> None:
    with write_session() as session:
        session.merge(
            SkillInstall(
                coding_agent=coding_agent,
                cafleet_version=cafleet_version,
                installed_at=now_iso(),
            )
        )
