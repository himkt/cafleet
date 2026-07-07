"""rename fleet label to name

Revision ID: 0009
Revises: 0008
Create Date: 2026-07-07 00:00:00.000000

Rename the ``fleets.label`` column to ``fleets.name``. Data-preserving: the
existing values carry over unchanged. The column stays nullable.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0009"
down_revision: str | None = "0008"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table(
        "fleets", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "label", new_column_name="name", existing_type=sa.String()
        )


def downgrade() -> None:
    with op.batch_alter_table(
        "fleets", schema=None, table_kwargs={"sqlite_autoincrement": True}
    ) as batch_op:
        batch_op.alter_column(
            "name", new_column_name="label", existing_type=sa.String()
        )
