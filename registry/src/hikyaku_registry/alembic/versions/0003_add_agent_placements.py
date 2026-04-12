"""add agent_placements table

Revision ID: 0003
Revises: 0002
Create Date: 2026-04-12
"""

from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = "0003"
down_revision: Union[str, None] = "0002"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "agent_placements",
        sa.Column("agent_id", sa.String(), primary_key=True),
        sa.Column("director_agent_id", sa.String(), nullable=False),
        sa.Column("tmux_session", sa.String(), nullable=False),
        sa.Column("tmux_window_id", sa.String(), nullable=False),
        sa.Column("tmux_pane_id", sa.String(), nullable=True),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(["agent_id"], ["agents.agent_id"], ondelete="CASCADE"),
        sa.ForeignKeyConstraint(
            ["director_agent_id"], ["agents.agent_id"], ondelete="RESTRICT"
        ),
    )
    op.create_index(
        "idx_placements_director",
        "agent_placements",
        ["director_agent_id"],
    )


def downgrade() -> None:
    op.drop_index("idx_placements_director", table_name="agent_placements")
    op.drop_table("agent_placements")
