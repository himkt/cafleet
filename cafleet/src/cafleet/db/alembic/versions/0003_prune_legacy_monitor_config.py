"""prune legacy non-Director monitor_config rows

Revision ID: 0003
Revises: 0002
Create Date: 2026-06-14

"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0003"
down_revision: str | None = "0002"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # design 0000090 §8: enrollment is restricted to the root Director and the
    # dedicated monitoring member. Pre-upgrade there are no monitoring members,
    # so pruning every non-Director row leaves exactly the root-Director rows
    # enrolled; new monitoring members enroll going forward via register_agent.
    op.execute(
        """
        DELETE FROM monitor_config
        WHERE agent_id NOT IN (
            SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL
        )
        """
    )


def downgrade() -> None:
    # No-op: re-enrolling every pane-bound agent is neither possible nor
    # desirable, and the pruned rows are gone for good.
    pass
