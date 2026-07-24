"""fold legacy canceled message status into completed

Revision ID: 0004
Revises: 0003
Create Date: 2026-07-24 19:18:28.042001

"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0004"
down_revision: str | None = "0003"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # Data-only fold: keep every message row and its content; status_timestamp
    # stays untouched — it records when the row reached its terminal state,
    # which remains true after the fold.
    op.execute(
        "UPDATE messages SET status_state = 'completed' "
        "WHERE status_state = 'canceled'"
    )


def downgrade() -> None:
    # Irreversible: which rows were previously 'canceled' is unrecoverable
    # after the fold, so the downgrade is a no-op.
    pass
