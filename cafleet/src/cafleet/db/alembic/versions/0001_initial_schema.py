"""initial schema

Revision ID: 0001
Revises:
Create Date: 2026-07-11 12:11:23.590758

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
    # ``members`` is created first because ``fleets``, ``tasks``, ``monitor_config``,
    # and ``member_placements`` all FK into it. ``members.fleet_id`` forward-references
    # ``fleets`` — SQLite tolerates a FK to a not-yet-created table at CREATE TABLE
    # time, and no rows are inserted during the migration. Do NOT reorder these two
    # tables (autogenerate cannot sort the members<->fleets cycle; this order is the fix).
    op.create_table(
        "members",
        sa.Column("member_id", sa.Integer(), nullable=False),
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("name", sa.String(), nullable=False),
        sa.Column("description", sa.String(), nullable=False),
        sa.Column("status", sa.String(), nullable=False),
        sa.Column("registered_at", sa.String(), nullable=False),
        sa.Column("deregistered_at", sa.String(), nullable=True),
        sa.Column("member_card_json", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.PrimaryKeyConstraint("member_id"),
        sqlite_autoincrement=True,
    )
    with op.batch_alter_table("members", schema=None) as batch_op:
        batch_op.create_index(
            "idx_members_fleet_status", ["fleet_id", "status"], unique=False
        )

    op.create_table(
        "fleets",
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("name", sa.String(), nullable=True),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.Column("deleted_at", sa.String(), nullable=True),
        sa.Column("director_member_id", sa.Integer(), nullable=True),
        sa.ForeignKeyConstraint(
            ["director_member_id"], ["members.member_id"], ondelete="RESTRICT"
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
        "member_placements",
        sa.Column("member_id", sa.Integer(), nullable=False),
        sa.Column("mux_session", sa.String(), nullable=False),
        sa.Column("mux_window_id", sa.String(), nullable=False),
        sa.Column("mux_pane_id", sa.String(), nullable=True),
        sa.Column("backend", sa.String(), server_default="tmux", nullable=False),
        sa.Column("coding_agent", sa.String(), server_default="claude", nullable=False),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(
            ["member_id"], ["members.member_id"], ondelete="CASCADE"
        ),
        sa.PrimaryKeyConstraint("member_id"),
    )
    op.create_table(
        "monitor_config",
        sa.Column("member_id", sa.Integer(), nullable=False),
        sa.Column(
            "interval_seconds", sa.Integer(), server_default="60", nullable=False
        ),
        sa.Column("last_ping_at", sa.String(), nullable=True),
        sa.Column("enabled", sa.Integer(), server_default="1", nullable=False),
        sa.ForeignKeyConstraint(
            ["member_id"], ["members.member_id"], ondelete="CASCADE"
        ),
        sa.PrimaryKeyConstraint("member_id"),
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
        sa.Column("from_member_id", sa.Integer(), nullable=False),
        sa.Column("to_member_id", sa.Integer(), nullable=True),
        sa.Column("type", sa.String(), nullable=False),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.Column("status_state", sa.String(), nullable=False),
        sa.Column("status_timestamp", sa.String(), nullable=False),
        sa.Column("origin_task_id", sa.Integer(), nullable=True),
        sa.Column("text", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(
            ["context_id"], ["members.member_id"], ondelete="RESTRICT"
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
            "idx_tasks_from_member_status_ts",
            ["from_member_id", "status_timestamp"],
            unique=False,
        )


def downgrade() -> None:
    with op.batch_alter_table("tasks", schema=None) as batch_op:
        batch_op.drop_index("idx_tasks_from_member_status_ts")
        batch_op.drop_index("idx_tasks_context_status_ts")

    op.drop_table("tasks")
    op.drop_table("monitor_runtime")
    op.drop_table("monitor_config")
    op.drop_table("member_placements")
    op.drop_table("skill_installs")
    op.drop_table("fleets")
    with op.batch_alter_table("members", schema=None) as batch_op:
        batch_op.drop_index("idx_members_fleet_status")

    op.drop_table("members")
