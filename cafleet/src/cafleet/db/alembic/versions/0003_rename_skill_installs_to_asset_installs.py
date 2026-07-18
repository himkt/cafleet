"""rename skill_installs to asset_installs

Revision ID: 0003
Revises: 0002
Create Date: 2026-07-18 09:46:45.531334

"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0003"
down_revision: str | None = "0002"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "asset_installs",
        sa.Column("coding_agent", sa.String(), primary_key=True),
        sa.Column("cafleet_version", sa.String(), nullable=False),
        sa.Column("installed_at", sa.String(), nullable=False),
    )
    op.execute(
        "INSERT INTO asset_installs (coding_agent, cafleet_version, installed_at) "
        "SELECT coding_agent, cafleet_version, installed_at FROM skill_installs"
    )
    op.drop_table("skill_installs")


def downgrade() -> None:
    op.create_table(
        "skill_installs",
        sa.Column("coding_agent", sa.String(), primary_key=True),
        sa.Column("cafleet_version", sa.String(), nullable=False),
        sa.Column("installed_at", sa.String(), nullable=False),
    )
    op.execute(
        "INSERT INTO skill_installs (coding_agent, cafleet_version, installed_at) "
        "SELECT coding_agent, cafleet_version, installed_at FROM asset_installs"
    )
    op.drop_table("asset_installs")
