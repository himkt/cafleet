"""prune the root-Director monitor_config rows

Revision ID: 0004
Revises: 0003
Create Date: 2026-06-14

"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0004"
down_revision: str | None = "0003"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # design 0000091 §5: drop the root Director from the heartbeat entirely. 0003
    # kept the root-Director rows; this step prunes them, so monitor_config holds
    # only monitoring-member rows. The Director is re-engaged on demand, never by
    # the loop.
    op.execute(
        """
        DELETE FROM monitor_config
        WHERE agent_id IN (
            SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL
        )
        """
    )


def downgrade() -> None:
    # No-op: re-enrolling the Director is neither possible nor desirable.
    pass
