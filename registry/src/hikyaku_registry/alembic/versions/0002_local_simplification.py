"""local simplification: drop api_keys+owner_sub, add sessions, rename agents.tenant_id to session_id

Revision ID: 0002_local_simplification
Revises: 0001
Create Date: 2026-04-12
"""

from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


revision: str = "0002_local_simplification"
down_revision: Union[str, None] = "0001"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # 1. Create sessions table.
    op.create_table(
        "sessions",
        sa.Column("session_id", sa.String(), nullable=False),
        sa.Column("label", sa.String(), nullable=True),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.PrimaryKeyConstraint("session_id"),
    )

    # 2. Seed one session per api_keys row (active AND revoked), using
    #    the api_key_hash as the session_id. This preserves existing
    #    agents.tenant_id values byte-for-byte so step 3 becomes a pure rename.
    #    Including revoked keys prevents FK violations for agents whose
    #    tenant_id references a revoked key row.
    op.execute(
        """
        INSERT INTO sessions (session_id, label, created_at)
        SELECT api_key_hash, 'legacy-' || key_prefix, created_at
        FROM api_keys
        """
    )

    # 3. Recreate agents table with tenant_id renamed to session_id and
    #    FK retargeted from api_keys to sessions.
    #
    #    Raw SQL avoids an Alembic batch-mode bug where column rename +
    #    target_metadata index reconciliation produces a KeyError on the
    #    new column name.
    op.execute(
        """
        CREATE TABLE _agents_new (
            agent_id VARCHAR NOT NULL,
            session_id VARCHAR NOT NULL,
            name VARCHAR NOT NULL,
            description VARCHAR NOT NULL,
            status VARCHAR NOT NULL,
            registered_at VARCHAR NOT NULL,
            deregistered_at VARCHAR,
            agent_card_json VARCHAR NOT NULL,
            PRIMARY KEY (agent_id),
            FOREIGN KEY(session_id) REFERENCES sessions(session_id) ON DELETE RESTRICT
        )
        """
    )
    op.execute(
        """
        INSERT INTO _agents_new
            (agent_id, session_id, name, description, status,
             registered_at, deregistered_at, agent_card_json)
        SELECT agent_id, tenant_id, name, description, status,
               registered_at, deregistered_at, agent_card_json
        FROM agents
        """
    )
    op.execute("DROP TABLE agents")
    op.execute("ALTER TABLE _agents_new RENAME TO agents")
    op.create_index("idx_agents_session_status", "agents", ["session_id", "status"])

    # 4. Drop api_keys entirely.
    op.drop_index("idx_api_keys_owner", table_name="api_keys")
    op.drop_table("api_keys")


def downgrade() -> None:
    raise NotImplementedError(
        "0002_local_simplification is a one-way migration. "
        "Auth0 re-introduction is out of scope; restore from a backup instead."
    )
