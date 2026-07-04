"""to_agent_id nullable

Revision ID: 0007
Revises: 0006
Create Date: 2026-07-04 07:30:00.000000

"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0007"
down_revision: str | None = "0006"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # ``tasks`` mints ids via AUTOINCREMENT; the batch recreate must carry the
    # ``sqlite_autoincrement`` table kwarg or it silently drops the clause.
    with op.batch_alter_table(
        "tasks", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "to_agent_id",
            existing_type=sa.Integer(),
            nullable=True,
        )


def downgrade() -> None:
    with op.batch_alter_table(
        "tasks", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "to_agent_id",
            existing_type=sa.Integer(),
            nullable=False,
        )
