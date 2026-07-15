"""drop coding_agent server default on member_placements

Revision ID: 0002
Revises: 0001
Create Date: 2026-07-14 23:08:36.615537

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
    with op.batch_alter_table("member_placements") as batch_op:
        batch_op.alter_column(
            "coding_agent",
            existing_type=sa.String(),
            existing_nullable=False,
            server_default=None,
        )


def downgrade() -> None:
    with op.batch_alter_table("member_placements") as batch_op:
        batch_op.alter_column(
            "coding_agent",
            existing_type=sa.String(),
            existing_nullable=False,
            server_default="claude",
        )
