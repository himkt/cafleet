"""backend-neutral placement columns

Revision ID: 0008
Revises: 0007
Create Date: 2026-07-06 11:10:00.000000

Rename the three ``tmux_*`` ``agent_placements`` columns to ``mux_*`` and add a
``backend`` column recording which multiplexer produced the ids. The
``DEFAULT 'tmux'`` backfills every pre-existing row to its real provenance.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0008"
down_revision: str | None = "0007"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table("agent_placements", schema=None) as batch_op:
        batch_op.alter_column(
            "tmux_session", new_column_name="mux_session", existing_type=sa.String()
        )
        batch_op.alter_column(
            "tmux_window_id", new_column_name="mux_window_id", existing_type=sa.String()
        )
        batch_op.alter_column(
            "tmux_pane_id", new_column_name="mux_pane_id", existing_type=sa.String()
        )
        batch_op.add_column(
            sa.Column("backend", sa.String(), nullable=False, server_default="tmux")
        )


def downgrade() -> None:
    with op.batch_alter_table("agent_placements", schema=None) as batch_op:
        batch_op.drop_column("backend")
        batch_op.alter_column(
            "mux_session", new_column_name="tmux_session", existing_type=sa.String()
        )
        batch_op.alter_column(
            "mux_window_id", new_column_name="tmux_window_id", existing_type=sa.String()
        )
        batch_op.alter_column(
            "mux_pane_id", new_column_name="tmux_pane_id", existing_type=sa.String()
        )
