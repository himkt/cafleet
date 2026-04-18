"""SQLAlchemy declarative models; see ``docs/spec/data-model.md``."""

from sqlalchemy import ForeignKey, Index, String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class Session(Base):
    __tablename__ = "sessions"

    session_id: Mapped[str] = mapped_column(String, primary_key=True)
    label: Mapped[str | None] = mapped_column(String, nullable=True)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    deleted_at: Mapped[str | None] = mapped_column(String, nullable=True)
    director_agent_id: Mapped[str | None] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="RESTRICT"),
        nullable=True,
    )


class Agent(Base):
    __tablename__ = "agents"

    agent_id: Mapped[str] = mapped_column(String, primary_key=True)
    session_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("sessions.session_id", ondelete="RESTRICT"),
        nullable=False,
    )
    name: Mapped[str] = mapped_column(String, nullable=False)
    description: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    registered_at: Mapped[str] = mapped_column(String, nullable=False)
    deregistered_at: Mapped[str | None] = mapped_column(String, nullable=True)
    agent_card_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_agents_session_status", "session_id", "status"),)


class AgentPlacement(Base):
    __tablename__ = "agent_placements"

    agent_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="CASCADE"),
        primary_key=True,
    )
    director_agent_id: Mapped[str | None] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="RESTRICT"),
        nullable=True,
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

    task_id: Mapped[str] = mapped_column(String, primary_key=True)
    context_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="RESTRICT"),
        nullable=False,
    )
    from_agent_id: Mapped[str] = mapped_column(String, nullable=False)
    to_agent_id: Mapped[str] = mapped_column(String, nullable=False)
    type: Mapped[str] = mapped_column(String, nullable=False)
    created_at: Mapped[str] = mapped_column(String, nullable=False)
    status_state: Mapped[str] = mapped_column(String, nullable=False)
    status_timestamp: Mapped[str] = mapped_column(String, nullable=False)
    origin_task_id: Mapped[str | None] = mapped_column(String, nullable=True)
    task_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (
        Index("idx_tasks_context_status_ts", "context_id", "status_timestamp"),
        Index("idx_tasks_from_agent_status_ts", "from_agent_id", "status_timestamp"),
    )
