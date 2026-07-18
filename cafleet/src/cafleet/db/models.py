"""SQLAlchemy declarative models; see ``docs/spec/data-model.md``."""

from sqlalchemy import ForeignKey, Index, Integer, String
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


class AssetInstall(Base):
    __tablename__ = "asset_installs"

    coding_agent: Mapped[str] = mapped_column(String, primary_key=True)
    cafleet_version: Mapped[str] = mapped_column(String, nullable=False)
    installed_at: Mapped[str] = mapped_column(String, nullable=False)
