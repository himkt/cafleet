"""SQLAlchemy declarative models for the hikyaku registry SQLite store.

Schema mirrors `docs/spec/data-model.md` exactly. The hybrid model promotes
indexed/queried fields to columns and stores opaque A2A payloads
(`AgentCard`, `Task`) as JSON `TEXT` blobs that the application layer
serializes and deserializes via Pydantic.
"""

from sqlalchemy import ForeignKey, Index, String
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


class Base(DeclarativeBase):
    pass


class ApiKey(Base):
    __tablename__ = "api_keys"

    api_key_hash: Mapped[str] = mapped_column(String, primary_key=True)
    owner_sub: Mapped[str] = mapped_column(String, nullable=False)
    key_prefix: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    created_at: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_api_keys_owner", "owner_sub"),)


class Agent(Base):
    __tablename__ = "agents"

    agent_id: Mapped[str] = mapped_column(String, primary_key=True)
    tenant_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("api_keys.api_key_hash", ondelete="RESTRICT"),
        nullable=False,
    )
    name: Mapped[str] = mapped_column(String, nullable=False)
    description: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    registered_at: Mapped[str] = mapped_column(String, nullable=False)
    deregistered_at: Mapped[str | None] = mapped_column(String, nullable=True)
    agent_card_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_agents_tenant_status", "tenant_id", "status"),)


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
