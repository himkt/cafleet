"""session bootstrap Director: add sessions.deleted_at + sessions.director_agent_id, relax agent_placements.director_agent_id nullability

Revision ID: 0007
Revises: 0006
Create Date: 2026-04-16

Design doc 0000026. Structural migration — no data backfill.

The 5-step transactional bootstrap in ``broker.create_session`` writes the
``sessions`` row first (with ``director_agent_id=NULL``) and back-fills the
FK later, so the column is DB-nullable. The post-bootstrap invariant
(every non-deleted session has a non-NULL ``director_agent_id``) is
enforced by the broker, not the schema.

``agent_placements.director_agent_id`` is relaxed to nullable because the
root Director's placement stores ``NULL`` there to indicate "no parent".

Implementation note: The two new columns on ``sessions`` are added via
raw ``ALTER TABLE ... ADD COLUMN`` statements. SQLite's SQLAlchemy
dialect refuses ``op.add_column`` with an inline ``ForeignKey`` (it
raises ``NotImplementedError: No support for ALTER of constraints in
SQLite dialect``), and ``batch_alter_table("sessions")`` cannot be used
on a non-empty database because it DROPs and recreates the ``sessions``
table — tripping the ``agents.session_id -> sessions.session_id``
``ON DELETE RESTRICT`` FK. SQLite natively supports inline FK clauses
in ``ALTER TABLE ADD COLUMN``, which gives the desired schema without a
table rebuild. ``agent_placements`` is not referenced by any other
table, so batch-altering it to relax ``director_agent_id`` nullability
is safe.
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
    op.execute("ALTER TABLE sessions ADD COLUMN deleted_at VARCHAR")
    op.execute(
        "ALTER TABLE sessions ADD COLUMN director_agent_id VARCHAR "
        "REFERENCES agents(agent_id) ON DELETE RESTRICT"
    )

    with op.batch_alter_table("agent_placements") as batch_op:
        batch_op.alter_column(
            "director_agent_id", existing_type=sa.String(), nullable=True
        )


def downgrade() -> None:
    with op.batch_alter_table("agent_placements") as batch_op:
        batch_op.alter_column(
            "director_agent_id", existing_type=sa.String(), nullable=False
        )

    # Use raw ``ALTER TABLE DROP COLUMN`` (SQLite 3.35+) for the same reason
    # the upgrade uses raw ``ADD COLUMN``: ``batch_alter_table("sessions")``
    # DROPs and recreates the table, which trips the
    # ``agents.session_id -> sessions.session_id`` FK on non-empty DBs.
    op.execute("ALTER TABLE sessions DROP COLUMN director_agent_id")
    op.execute("ALTER TABLE sessions DROP COLUMN deleted_at")
