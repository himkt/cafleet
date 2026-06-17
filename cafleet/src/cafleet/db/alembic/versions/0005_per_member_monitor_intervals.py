"""per-member monitor intervals: prune the monitoring member, backfill Director + members

Revision ID: 0005
Revises: 0004
Create Date: 2026-06-17

"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0005"
down_revision: str | None = "0004"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # design 0000096 §9: invert enrollment. 0004 left only monitoring-member rows;
    # this step clears them and enrolls the watched set fresh — the root Director
    # @180 and every active, pane-bound ordinary member @720. INSERT OR IGNORE
    # makes the backfill idempotent.
    # 1. Drop the monitoring member's monitor_config rows (its interval is removed).
    op.execute(
        """
        DELETE FROM monitor_config WHERE agent_id IN (
            SELECT agent_id FROM agents
            WHERE json_extract(agent_card_json, '$.cafleet.kind') = 'monitoring-member'
        )
        """
    )
    # 2. Backfill existing active root Directors @180.
    op.execute(
        """
        INSERT OR IGNORE INTO monitor_config (agent_id, interval_seconds, enabled)
        SELECT f.director_agent_id, 180, 1 FROM fleets f
        WHERE f.director_agent_id IS NOT NULL AND f.deleted_at IS NULL
        """
    )
    # 3. Backfill existing active, pane-bound ordinary members @720 (skip the
    #    Director, the monitoring member, and the Administrator).
    op.execute(
        """
        INSERT OR IGNORE INTO monitor_config (agent_id, interval_seconds, enabled)
        SELECT a.agent_id, 720, 1 FROM agents a
        JOIN agent_placements p ON p.agent_id = a.agent_id
        WHERE a.status = 'active'
          AND a.agent_id NOT IN
              (SELECT director_agent_id FROM fleets WHERE director_agent_id IS NOT NULL)
          AND json_extract(a.agent_card_json, '$.cafleet.kind') IS NOT 'monitoring-member'
          AND json_extract(a.agent_card_json, '$.cafleet.kind') IS NOT 'builtin-administrator'
        """
    )


def downgrade() -> None:
    # No-op: re-deriving the pre-inversion enrollment is neither possible nor desirable.
    pass
