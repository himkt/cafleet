"""add durable monitor stall episode and report delivery state

Revision ID: 0005
Revises: 0004
Create Date: 2026-07-29 07:40:00.000000

"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0005"
down_revision: str | None = "0004"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table("monitor_config") as batch_op:
        batch_op.add_column(
            sa.Column("last_stall_check_at", sa.String(), nullable=True)
        )
        batch_op.add_column(
            sa.Column("last_stall_candidate_at", sa.String(), nullable=True)
        )
        batch_op.add_column(
            sa.Column("last_stall_capture_sha256", sa.String(), nullable=True)
        )
        batch_op.add_column(
            sa.Column(
                "stall_episode_state",
                sa.String(),
                server_default="clear",
                nullable=False,
            )
        )
        batch_op.add_column(
            sa.Column("stall_escalation_reason", sa.String(), nullable=True)
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_stall_episode_state",
            "stall_episode_state IN "
            "('clear', 'nudge_claimed', 'nudged', "
            "'escalation_pending', 'escalated')",
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_stall_escalation_reason",
            "stall_escalation_reason IS NULL OR "
            "stall_escalation_reason IN "
            "('ping_failed', 'ping_interrupted', 'unchanged_after_nudge')",
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_stall_candidate_pair",
            "(last_stall_candidate_at IS NULL AND "
            "last_stall_capture_sha256 IS NULL) OR "
            "(last_stall_candidate_at IS NOT NULL AND "
            "last_stall_capture_sha256 IS NOT NULL)",
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_nonclear_has_candidate",
            "stall_episode_state = 'clear' OR "
            "(last_stall_candidate_at IS NOT NULL AND "
            "last_stall_capture_sha256 IS NOT NULL)",
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_state_reason_pair",
            "(stall_episode_state IN ('clear', 'nudge_claimed', 'nudged') "
            "AND stall_escalation_reason IS NULL) OR "
            "(stall_episode_state IN ('escalation_pending', 'escalated') "
            "AND stall_escalation_reason IS NOT NULL)",
        )
        batch_op.create_check_constraint(
            "ck_monitor_config_capture_sha256",
            "last_stall_capture_sha256 IS NULL OR "
            "(length(last_stall_capture_sha256) = 64 AND "
            "last_stall_capture_sha256 = lower(last_stall_capture_sha256) AND "
            "last_stall_capture_sha256 NOT GLOB '*[^0-9a-f]*')",
        )

    op.create_table(
        "monitor_report_delivery",
        sa.Column("message_id", sa.Integer(), nullable=False),
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column(
            "preview_state",
            sa.String(),
            server_default="pending",
            nullable=False,
        ),
        sa.Column("attempt_count", sa.Integer(), server_default="0", nullable=False),
        sa.Column("last_attempt_at", sa.String(), nullable=True),
        sa.Column("delivered_at", sa.String(), nullable=True),
        sa.CheckConstraint(
            "preview_state IN ('pending', 'awaiting_ack', 'delivered')",
            name="ck_monitor_report_delivery_preview_state",
        ),
        sa.CheckConstraint(
            "attempt_count >= 0",
            name="ck_monitor_report_delivery_attempt_count",
        ),
        sa.CheckConstraint(
            "(attempt_count = 0 AND last_attempt_at IS NULL) OR "
            "(attempt_count > 0 AND last_attempt_at IS NOT NULL)",
            name="ck_monitor_report_delivery_attempt_timestamp",
        ),
        sa.CheckConstraint(
            "(preview_state = 'delivered' AND delivered_at IS NOT NULL) OR "
            "(preview_state != 'delivered' AND delivered_at IS NULL)",
            name="ck_monitor_report_delivery_delivered_timestamp",
        ),
        sa.CheckConstraint(
            "preview_state != 'awaiting_ack' OR attempt_count > 0",
            name="ck_monitor_report_delivery_awaiting_ack_attempted",
        ),
        sa.ForeignKeyConstraint(
            ["message_id"], ["messages.message_id"], ondelete="CASCADE"
        ),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.PrimaryKeyConstraint("message_id"),
    )
    op.create_index(
        "uq_monitor_report_delivery_one_open_per_fleet",
        "monitor_report_delivery",
        ["fleet_id"],
        unique=True,
        sqlite_where=sa.text("preview_state IN ('pending', 'awaiting_ack')"),
    )
    op.create_index(
        "idx_monitor_report_delivery_fleet_state_message",
        "monitor_report_delivery",
        ["fleet_id", "preview_state", "message_id"],
        unique=False,
    )

    op.create_table(
        "monitor_director_gate",
        sa.Column("fleet_id", sa.Integer(), nullable=False),
        sa.Column("director_member_id", sa.Integer(), nullable=False),
        sa.Column("token_sha256", sa.String(), nullable=False),
        sa.Column("classification", sa.String(), nullable=False),
        sa.Column("issued_at", sa.String(), nullable=False),
        sa.Column("expires_at", sa.String(), nullable=False),
        sa.CheckConstraint(
            "length(token_sha256) = 64 AND "
            "token_sha256 = lower(token_sha256) AND "
            "token_sha256 NOT GLOB '*[^0-9a-f]*'",
            name="ck_monitor_director_gate_token_sha256",
        ),
        sa.CheckConstraint(
            "classification IN ('finished', 'stalled')",
            name="ck_monitor_director_gate_classification",
        ),
        sa.CheckConstraint(
            "expires_at > issued_at",
            name="ck_monitor_director_gate_expiry",
        ),
        sa.ForeignKeyConstraint(["fleet_id"], ["fleets.fleet_id"], ondelete="RESTRICT"),
        sa.ForeignKeyConstraint(
            ["director_member_id"], ["members.member_id"], ondelete="RESTRICT"
        ),
        sa.PrimaryKeyConstraint("fleet_id"),
    )


def downgrade() -> None:
    op.drop_table("monitor_director_gate")
    op.drop_index(
        "idx_monitor_report_delivery_fleet_state_message",
        table_name="monitor_report_delivery",
    )
    op.drop_index(
        "uq_monitor_report_delivery_one_open_per_fleet",
        table_name="monitor_report_delivery",
    )
    op.drop_table("monitor_report_delivery")

    with op.batch_alter_table("monitor_config") as batch_op:
        batch_op.drop_constraint("ck_monitor_config_capture_sha256", type_="check")
        batch_op.drop_constraint("ck_monitor_config_state_reason_pair", type_="check")
        batch_op.drop_constraint(
            "ck_monitor_config_nonclear_has_candidate", type_="check"
        )
        batch_op.drop_constraint(
            "ck_monitor_config_stall_candidate_pair", type_="check"
        )
        batch_op.drop_constraint(
            "ck_monitor_config_stall_escalation_reason", type_="check"
        )
        batch_op.drop_constraint("ck_monitor_config_stall_episode_state", type_="check")
        batch_op.drop_column("stall_escalation_reason")
        batch_op.drop_column("stall_episode_state")
        batch_op.drop_column("last_stall_capture_sha256")
        batch_op.drop_column("last_stall_candidate_at")
        batch_op.drop_column("last_stall_check_at")
