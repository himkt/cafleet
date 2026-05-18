"""Backfill agent_placements.coding_agent = 'unknown' rows to 'claude'.

Revision ID: 0010
Revises: 0009
Create Date: 2026-05-18

Legacy root-Director rows in ``agent_placements`` carry
``coding_agent = "unknown"``. ``CODING_AGENTS["unknown"]`` raises
``KeyError``, so the column value must be a valid registry key. Backfill
those rows to ``'claude'`` — the only coding-agent backend available when
those rows were originally inserted. Rows already at ``'claude'`` /
``'codex'`` are untouched; the migration is idempotent.
"""

from collections.abc import Sequence

from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0010"
down_revision: str | None = "0009"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.execute(
        "UPDATE agent_placements "
        "SET coding_agent = 'claude' "
        "WHERE coding_agent = 'unknown'"
    )


def downgrade() -> None:
    # No-op: data-only backfill. The original 'unknown' value is not
    # recoverable from 'claude' (it would require knowing which rows were
    # originally backfilled), and 'unknown' is treated as a label-not-truth
    # value that 'claude' supersedes.
    pass
