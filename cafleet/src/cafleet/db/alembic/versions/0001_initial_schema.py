"""initial schema

Revision ID: 0001
Revises:
Create Date: 2026-07-08 09:42:01.912428

"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0001"
down_revision: str | None = None
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    # ``agents`` is created first because ``fleets``, ``tasks``, ``monitor_config``,
    # and ``agent_placements`` all FK into it. ``agents.fleet_id`` forward-references
    # ``fleets`` — SQLite tolerates a FK to a not-yet-created table at CREATE TABLE
    # time, and no rows are inserted during the migration. Do NOT reorder these two
    # tables (autogenerate cannot sort the agents<->fleets cycle; this order is the fix).
    op.create_table(
        "agents",
        sa.Column("agent_id", sa.Integer(), nullable=False),
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("name", sa.String(), nullable=False),
        sa.Column("description", sa.String(), nullable=False),
        sa.Column("status", sa.String(), nullable=False),
        sa.Column("registered_at", sa.String(), nullable=False),
        sa.Column("deregistered_at", sa.String(), nullable=True),
        sa.Column("agent_card_json", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.PrimaryKeyConstraint("agent_id"),
        sqlite_autoincrement=True,
    )
    with op.batch_alter_table("agents", schema=None) as batch_op:
        batch_op.create_index(
            "idx_agents_fleet_status", ["fleet_id", "status"], unique=False
        )

    op.create_table(
        "fleets",
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("name", sa.String(), nullable=True),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.Column("deleted_at", sa.String(), nullable=True),
        sa.Column("director_agent_id", sa.Integer(), nullable=True),
        sa.ForeignKeyConstraint(
            ["director_agent_id"], ["agents.agent_id"], ondelete="RESTRICT"
        ),
        sa.PrimaryKeyConstraint("fleet_id"),
        sqlite_autoincrement=True,
    )
    op.create_table(
        "skill_installs",
        sa.Column("coding_agent", sa.String(), nullable=False),
        sa.Column("cafleet_version", sa.String(), nullable=False),
        sa.Column("installed_at", sa.String(), nullable=False),
        sa.PrimaryKeyConstraint("coding_agent"),
    )
    op.create_table(
        "agent_placements",
        sa.Column("agent_id", sa.Integer(), nullable=False),
        sa.Column("director_agent_id", sa.Integer(), nullable=True),
        sa.Column("mux_session", sa.String(), nullable=False),
        sa.Column("mux_window_id", sa.String(), nullable=False),
        sa.Column("mux_pane_id", sa.String(), nullable=True),
        sa.Column("backend", sa.String(), server_default="tmux", nullable=False),
        sa.Column("coding_agent", sa.String(), server_default="claude", nullable=False),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(["agent_id"], ["agents.agent_id"], ondelete="CASCADE"),
        sa.ForeignKeyConstraint(
            ["director_agent_id"], ["agents.agent_id"], ondelete="RESTRICT"
        ),
        sa.PrimaryKeyConstraint("agent_id"),
    )
    with op.batch_alter_table("agent_placements", schema=None) as batch_op:
        batch_op.create_index(
            "idx_placements_director", ["director_agent_id"], unique=False
        )

    op.create_table(
        "monitor_config",
        sa.Column("agent_id", sa.Integer(), nullable=False),
        sa.Column(
            "interval_seconds", sa.Integer(), server_default="60", nullable=False
        ),
        sa.Column("last_ping_at", sa.String(), nullable=True),
        sa.Column("enabled", sa.Integer(), server_default="1", nullable=False),
        sa.ForeignKeyConstraint(["agent_id"], ["agents.agent_id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("agent_id"),
    )
    op.create_table(
        "monitor_runtime",
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("pid", sa.Integer(), nullable=True),
        sa.Column("started_at", sa.String(), nullable=True),
        sa.Column("last_tick_at", sa.String(), nullable=True),
        sa.Column("tick_seconds", sa.Integer(), server_default="5", nullable=False),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.PrimaryKeyConstraint("fleet_id"),
    )
    op.create_table(
        "tasks",
        sa.Column("task_id", sa.Integer(), nullable=False),
        sa.Column("context_id", sa.Integer(), nullable=False),
        sa.Column("from_agent_id", sa.Integer(), nullable=False),
        sa.Column("to_agent_id", sa.Integer(), nullable=True),
        sa.Column("type", sa.String(), nullable=False),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.Column("status_state", sa.String(), nullable=False),
        sa.Column("status_timestamp", sa.String(), nullable=False),
        sa.Column("origin_task_id", sa.Integer(), nullable=True),
        sa.Column("text", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(
            ["context_id"], ["agents.agent_id"], ondelete="RESTRICT"
        ),
        sa.PrimaryKeyConstraint("task_id"),
        sqlite_autoincrement=True,
    )
    with op.batch_alter_table("tasks", schema=None) as batch_op:
        batch_op.create_index(
            "idx_tasks_context_status_ts",
            ["context_id", "status_timestamp"],
            unique=False,
        )
        batch_op.create_index(
            "idx_tasks_from_agent_status_ts",
            ["from_agent_id", "status_timestamp"],
            unique=False,
        )


def downgrade() -> None:
    with op.batch_alter_table("tasks", schema=None) as batch_op:
        batch_op.drop_index("idx_tasks_from_agent_status_ts")
        batch_op.drop_index("idx_tasks_context_status_ts")

    op.drop_table("tasks")
    op.drop_table("monitor_runtime")
    op.drop_table("monitor_config")
    with op.batch_alter_table("agent_placements", schema=None) as batch_op:
        batch_op.drop_index("idx_placements_director")

    op.drop_table("agent_placements")
    op.drop_table("skill_installs")
    op.drop_table("fleets")
    with op.batch_alter_table("agents", schema=None) as batch_op:
        batch_op.drop_index("idx_agents_fleet_status")

    op.drop_table("agents")
