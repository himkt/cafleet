"""Rename the ``sessions`` table to ``fleets`` (and ``session_id`` to ``fleet_id``).

Revision ID: 0011
Revises: 0010
Create Date: 2026-06-06

Forward rename of the CAFleet entity from "session" to "fleet": the
``sessions`` table becomes ``fleets``, its ``session_id`` PK becomes
``fleet_id``, the ``agents.session_id`` FK column becomes ``fleet_id``,
and the ``idx_agents_session_status`` index becomes
``idx_agents_fleet_status``.

FK enforcement is ON during migrations (``db/engine.py`` registers a global
``PRAGMA foreign_keys=ON`` listener) and the ``sessions``/``agents`` FK pair
is circular, so a parent-table rebuild is awkward. SQLite's native
``ALTER TABLE ... RENAME`` (on SQLite >= 3.25, with ``legacy_alter_table``
OFF — the default) auto-propagates the rename into dependent FK definitions
and index column references; Python 3.12's bundled ``sqlite3`` ships SQLite
well above 3.25. Index *names* are not auto-renamed, so the index is dropped
and recreated explicitly. The downgrade is a full inverse restoring the
exact pre-0011 schema.
"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0011"
down_revision: str | None = "0010"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.execute("ALTER TABLE sessions RENAME TO fleets")
    op.execute("ALTER TABLE fleets RENAME COLUMN session_id TO fleet_id")
    op.execute("ALTER TABLE agents RENAME COLUMN session_id TO fleet_id")
    op.drop_index("idx_agents_session_status", table_name="agents")
    op.create_index("idx_agents_fleet_status", "agents", ["fleet_id", "status"])


def downgrade() -> None:
    op.drop_index("idx_agents_fleet_status", table_name="agents")
    op.execute("ALTER TABLE agents RENAME COLUMN fleet_id TO session_id")
    op.execute("ALTER TABLE fleets RENAME COLUMN fleet_id TO session_id")
    op.execute("ALTER TABLE fleets RENAME TO sessions")
    op.create_index("idx_agents_session_status", "agents", ["session_id", "status"])
