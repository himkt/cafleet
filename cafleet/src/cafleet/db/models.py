"""SQLAlchemy declarative models; see ``docs/spec/data-model.md``."""

from sqlalchemy import CheckConstraint, ForeignKey, Index, Integer, String, text
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class Fleet(Base):
    __tablename__ = "fleets"

    fleet_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    name: Mapped[str | None] = mapped_column(String, nullable=True)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    deleted_at: Mapped[str | None] = mapped_column(String, nullable=True)
    director_member_id: Mapped[int | None] = mapped_column(
        Integer, ForeignKey("members.member_id", ondelete="RESTRICT"), nullable=True
    )

    __table_args__ = ({"sqlite_autoincrement": True},)


class Member(Base):
    __tablename__ = "members"

    member_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    fleet_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("fleets.fleet_id", ondelete="RESTRICT"), nullable=False
    )
    name: Mapped[str] = mapped_column(String, nullable=False)
    description: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    registered_at: Mapped[str] = mapped_column(String, nullable=False)
    deregistered_at: Mapped[str | None] = mapped_column(String, nullable=True)
    member_card_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        Index("idx_members_fleet_status", "fleet_id", "status"),
        {"sqlite_autoincrement": True},
    )


class MemberPlacement(Base):
    __tablename__ = "member_placements"

    member_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("members.member_id", ondelete="CASCADE"), primary_key=True
    )
    mux_session: Mapped[str] = mapped_column(String, nullable=False)
    mux_window_id: Mapped[str] = mapped_column(String, nullable=False)
    mux_pane_id: Mapped[str | None] = mapped_column(String, nullable=True)
    backend: Mapped[str] = mapped_column(String, nullable=False, server_default="tmux")
    coding_agent: Mapped[str] = mapped_column(String, nullable=False)
    created_at: Mapped[str] = mapped_column(String, nullable=False)


class Message(Base):
    __tablename__ = "messages"

    message_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    owner_member_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("members.member_id", ondelete="RESTRICT"), nullable=False
    )
    from_member_id: Mapped[int] = mapped_column(Integer, nullable=False)
    to_member_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    type: Mapped[str] = mapped_column(String, nullable=False)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    status_state: Mapped[str] = mapped_column(String, nullable=False)
    status_timestamp: Mapped[str] = mapped_column(String, nullable=False)
    origin_message_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    text: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        Index(
            "idx_messages_owner_member_status_ts",
            "owner_member_id",
            "status_timestamp",
        ),
        Index(
            "idx_messages_from_member_status_ts",
            "from_member_id",
            "status_timestamp",
        ),
        {"sqlite_autoincrement": True},
    )


class MonitorConfig(Base):
    __tablename__ = "monitor_config"

    member_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("members.member_id", ondelete="CASCADE"), primary_key=True
    )
    interval_seconds: Mapped[int] = mapped_column(
        Integer, nullable=False, server_default="60"
    )
    last_ping_at: Mapped[str | None] = mapped_column(String, nullable=True)
    enabled: Mapped[int] = mapped_column(Integer, nullable=False, server_default="1")
    last_stall_check_at: Mapped[str | None] = mapped_column(String, nullable=True)
    last_stall_candidate_at: Mapped[str | None] = mapped_column(String, nullable=True)
    last_stall_capture_sha256: Mapped[str | None] = mapped_column(String, nullable=True)
    stall_episode_state: Mapped[str] = mapped_column(
        String, nullable=False, server_default="clear"
    )
    stall_escalation_reason: Mapped[str | None] = mapped_column(String, nullable=True)

    __table_args__ = (
        CheckConstraint(
            "stall_episode_state IN "
            "('clear', 'nudge_claimed', 'nudged', "
            "'escalation_pending', 'escalated')",
            name="ck_monitor_config_stall_episode_state",
        ),
        CheckConstraint(
            "stall_escalation_reason IS NULL OR "
            "stall_escalation_reason IN "
            "('ping_failed', 'ping_interrupted', 'unchanged_after_nudge')",
            name="ck_monitor_config_stall_escalation_reason",
        ),
        CheckConstraint(
            "(last_stall_candidate_at IS NULL AND "
            "last_stall_capture_sha256 IS NULL) OR "
            "(last_stall_candidate_at IS NOT NULL AND "
            "last_stall_capture_sha256 IS NOT NULL)",
            name="ck_monitor_config_stall_candidate_pair",
        ),
        CheckConstraint(
            "stall_episode_state = 'clear' OR "
            "(last_stall_candidate_at IS NOT NULL AND "
            "last_stall_capture_sha256 IS NOT NULL)",
            name="ck_monitor_config_nonclear_has_candidate",
        ),
        CheckConstraint(
            "(stall_episode_state IN ('clear', 'nudge_claimed', 'nudged') "
            "AND stall_escalation_reason IS NULL) OR "
            "(stall_episode_state IN ('escalation_pending', 'escalated') "
            "AND stall_escalation_reason IS NOT NULL)",
            name="ck_monitor_config_state_reason_pair",
        ),
        CheckConstraint(
            "last_stall_capture_sha256 IS NULL OR "
            "(length(last_stall_capture_sha256) = 64 AND "
            "last_stall_capture_sha256 = lower(last_stall_capture_sha256) AND "
            "last_stall_capture_sha256 NOT GLOB '*[^0-9a-f]*')",
            name="ck_monitor_config_capture_sha256",
        ),
    )


class MonitorRuntime(Base):
    __tablename__ = "monitor_runtime"

    fleet_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("fleets.fleet_id", ondelete="RESTRICT"), primary_key=True
    )
    pid: Mapped[int | None] = mapped_column(Integer, nullable=True)
    started_at: Mapped[str | None] = mapped_column(String, nullable=True)
    last_tick_at: Mapped[str | None] = mapped_column(String, nullable=True)
    tick_seconds: Mapped[int] = mapped_column(
        Integer, nullable=False, server_default="5"
    )


class MonitorReportDelivery(Base):
    __tablename__ = "monitor_report_delivery"

    message_id: Mapped[int] = mapped_column(
        Integer,
        ForeignKey("messages.message_id", ondelete="CASCADE"),
        primary_key=True,
    )
    fleet_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("fleets.fleet_id", ondelete="RESTRICT"), nullable=False
    )
    preview_state: Mapped[str] = mapped_column(
        String, nullable=False, server_default="pending"
    )
    attempt_count: Mapped[int] = mapped_column(
        Integer, nullable=False, server_default="0"
    )
    last_attempt_at: Mapped[str | None] = mapped_column(String, nullable=True)
    delivered_at: Mapped[str | None] = mapped_column(String, nullable=True)

    __table_args__ = (
        CheckConstraint(
            "preview_state IN ('pending', 'awaiting_ack', 'delivered')",
            name="ck_monitor_report_delivery_preview_state",
        ),
        CheckConstraint(
            "attempt_count >= 0",
            name="ck_monitor_report_delivery_attempt_count",
        ),
        CheckConstraint(
            "(attempt_count = 0 AND last_attempt_at IS NULL) OR "
            "(attempt_count > 0 AND last_attempt_at IS NOT NULL)",
            name="ck_monitor_report_delivery_attempt_timestamp",
        ),
        CheckConstraint(
            "(preview_state = 'delivered' AND delivered_at IS NOT NULL) OR "
            "(preview_state != 'delivered' AND delivered_at IS NULL)",
            name="ck_monitor_report_delivery_delivered_timestamp",
        ),
        CheckConstraint(
            "preview_state != 'awaiting_ack' OR attempt_count > 0",
            name="ck_monitor_report_delivery_awaiting_ack_attempted",
        ),
        Index(
            "uq_monitor_report_delivery_one_open_per_fleet",
            "fleet_id",
            unique=True,
            sqlite_where=text("preview_state IN ('pending', 'awaiting_ack')"),
        ),
        Index(
            "idx_monitor_report_delivery_fleet_state_message",
            "fleet_id",
            "preview_state",
            "message_id",
        ),
    )


class MonitorDirectorGate(Base):
    __tablename__ = "monitor_director_gate"

    fleet_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("fleets.fleet_id", ondelete="RESTRICT"), primary_key=True
    )
    director_member_id: Mapped[int] = mapped_column(
        Integer,
        ForeignKey("members.member_id", ondelete="RESTRICT"),
        nullable=False,
    )
    token_sha256: Mapped[str] = mapped_column(String, nullable=False)
    classification: Mapped[str] = mapped_column(String, nullable=False)
    issued_at: Mapped[str] = mapped_column(String, nullable=False)
    expires_at: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        CheckConstraint(
            "length(token_sha256) = 64 AND "
            "token_sha256 = lower(token_sha256) AND "
            "token_sha256 NOT GLOB '*[^0-9a-f]*'",
            name="ck_monitor_director_gate_token_sha256",
        ),
        CheckConstraint(
            "classification IN ('finished', 'stalled')",
            name="ck_monitor_director_gate_classification",
        ),
        CheckConstraint(
            "expires_at > issued_at",
            name="ck_monitor_director_gate_expiry",
        ),
    )


class AssetInstall(Base):
    __tablename__ = "asset_installs"

    coding_agent: Mapped[str] = mapped_column(String, primary_key=True)
    cafleet_version: Mapped[str] = mapped_column(String, nullable=False)
    installed_at: Mapped[str] = mapped_column(String, nullable=False)
