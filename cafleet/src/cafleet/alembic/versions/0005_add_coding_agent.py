"""add coding_agent column to agent_placements

Revision ID: 0005
Revises: 0004
Create Date: 2026-04-12
"""

from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = "0005"
down_revision: Union[str, None] = "0004"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "agent_placements",
        sa.Column("coding_agent", sa.String(), nullable=False, server_default="claude"),
    )


def downgrade() -> None:
    op.drop_column("agent_placements", "coding_agent")
