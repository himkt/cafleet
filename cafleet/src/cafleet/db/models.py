"""SQLAlchemy declarative models; see ``docs/spec/data-model.md``."""

from sqlalchemy import ForeignKey, Index, Integer, String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class Fleet(Base):
    __tablename__ = "fleets"

    fleet_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    label: Mapped[str | None] = mapped_column(String, nullable=True)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    deleted_at: Mapped[str | None] = mapped_column(String, nullable=True)
    director_agent_id: Mapped[int | None] = mapped_column(
        Integer, ForeignKey("agents.agent_id", ondelete="RESTRICT"), nullable=True
    )

    __table_args__ = ({"sqlite_autoincrement": True},)


class Agent(Base):
    __tablename__ = "agents"

    agent_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    fleet_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("fleets.fleet_id", ondelete="RESTRICT"), nullable=False
    )
    name: Mapped[str] = mapped_column(String, nullable=False)
    description: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    registered_at: Mapped[str] = mapped_column(String, nullable=False)
    deregistered_at: Mapped[str | None] = mapped_column(String, nullable=True)
    agent_card_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        Index("idx_agents_fleet_status", "fleet_id", "status"),
        {"sqlite_autoincrement": True},
    )


class AgentPlacement(Base):
    __tablename__ = "agent_placements"

    agent_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("agents.agent_id", ondelete="CASCADE"), primary_key=True
    )
    director_agent_id: Mapped[int | None] = mapped_column(
        Integer, ForeignKey("agents.agent_id", ondelete="RESTRICT"), nullable=True
    )
    tmux_session: Mapped[str] = mapped_column(String, nullable=False)
    tmux_window_id: Mapped[str] = mapped_column(String, nullable=False)
    tmux_pane_id: Mapped[str | None] = mapped_column(String, nullable=True)
    coding_agent: Mapped[str] = mapped_column(
        String, nullable=False, server_default="claude"
    )
    created_at: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_placements_director", "director_agent_id"),)


class Task(Base):
    __tablename__ = "tasks"

    task_id: Mapped[int] = mapped_column(Integer, primary_key=True)
    context_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("agents.agent_id", ondelete="RESTRICT"), nullable=False
    )
    from_agent_id: Mapped[int] = mapped_column(Integer, nullable=False)
    to_agent_id: Mapped[int] = mapped_column(Integer, nullable=False)
    type: Mapped[str] = mapped_column(String, nullable=False)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    status_state: Mapped[str] = mapped_column(String, nullable=False)
    status_timestamp: Mapped[str] = mapped_column(String, nullable=False)
    origin_task_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    text: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        Index("idx_tasks_context_status_ts", "context_id", "status_timestamp"),
        Index("idx_tasks_from_agent_status_ts", "from_agent_id", "status_timestamp"),
        {"sqlite_autoincrement": True},
    )


class MonitorConfig(Base):
    __tablename__ = "monitor_config"

    agent_id: Mapped[int] = mapped_column(
        Integer, ForeignKey("agents.agent_id", ondelete="CASCADE"), primary_key=True
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


class SkillInstall(Base):
    __tablename__ = "skill_installs"

    coding_agent: Mapped[str] = mapped_column(String, primary_key=True)
    cafleet_version: Mapped[str] = mapped_column(String, nullable=False)
    installed_at: Mapped[str] = mapped_column(String, nullable=False)
