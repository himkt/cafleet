"""monitor tables

Revision ID: 0002
Revises: 0001
Create Date: 2026-06-13

"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0002"
down_revision: str | None = "0001"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # ``monitor_config.agent_id`` reuses the parent ``agents.agent_id`` value as a
    # 1:1 PK (mirrors ``agent_placements``), so it is NOT an AUTOINCREMENT table.
    # CASCADE off ``agents`` matches ``agent_placements``.
    op.create_table(
        "monitor_config",
        sa.Column("agent_id", sa.Integer(), nullable=False),
        sa.Column(
            "interval_seconds", sa.Integer(), nullable=False, server_default="60"
        ),
        sa.Column("last_ping_at", sa.String(), nullable=True),
        sa.Column("enabled", sa.Integer(), nullable=False, server_default="1"),
        sa.ForeignKeyConstraint(["agent_id"], ["agents.agent_id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("agent_id"),
    )

    # ``monitor_runtime.fleet_id`` reuses the parent ``fleets.fleet_id`` value as a
    # 1:1 PK, so it is NOT an AUTOINCREMENT table. RESTRICT off ``fleets`` matches
    # the other fleet FKs.
    op.create_table(
        "monitor_runtime",
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("pid", sa.Integer(), nullable=True),
        sa.Column("started_at", sa.String(), nullable=True),
        sa.Column("last_tick_at", sa.String(), nullable=True),
        sa.Column("tick_seconds", sa.Integer(), nullable=False, server_default="5"),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.PrimaryKeyConstraint("fleet_id"),
    )


def downgrade() -> None:
    op.drop_table("monitor_runtime")
    op.drop_table("monitor_config")
