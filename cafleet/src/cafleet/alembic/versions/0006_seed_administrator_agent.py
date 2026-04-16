"""seed built-in Administrator agent into every pre-existing session

Revision ID: 0006
Revises: 0005
Create Date: 2026-04-15

Design doc 0000025 §C.

Data-only migration: each pre-existing session in the ``sessions`` table
gains a built-in Administrator agent whose ``agent_card_json.cafleet.kind``
is ``"builtin-administrator"``. The ``agent_id`` is generated in Python via
``uuid.uuid4()`` (matching the broker's idiom) and ``registered_at`` is set
equal to the owning session's ``created_at``.

Idempotency:
    The upgrade probes each session for an existing Administrator via
    ``json_extract(agent_card_json, '$.cafleet.kind') = 'builtin-administrator'``
    before inserting. Re-running ``upgrade`` leaves the single Administrator
    untouched — there are never duplicates.

Forward-only in practice:
    ``downgrade()`` is provided for completeness on empty sessions and
    deletes Administrator rows via the same ``json_extract`` probe. However,
    ``tasks.context_id`` (see ``db/models.py``) references ``agents.agent_id``
    with ``ON DELETE RESTRICT``, so any session that already has tasks
    addressed to or from the Administrator will fail the downgrade with an
    SQLite ``IntegrityError``. We do NOT try to work around RESTRICT — the
    correct recovery from the non-empty case is a DB backup, not this
    migration. Treat 0006 as one-way in production.
"""

import json
import uuid
from collections.abc import Sequence

from alembic import op
from sqlalchemy import text


# revision identifiers, used by Alembic.
revision: str = "0006"
down_revision: str | None = "0005"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


ADMINISTRATOR_KIND = "builtin-administrator"


def _administrator_card(session_id: str) -> str:
    """Serialized canonical Administrator card for the given session."""
    return json.dumps(
        {
            "name": "Administrator",
            "description": f"Built-in administrator agent for session {session_id[:8]}",
            "skills": [],
            "cafleet": {"kind": ADMINISTRATOR_KIND},
        }
    )


def upgrade() -> None:
    bind = op.get_bind()

    sessions = bind.execute(
        text("SELECT session_id, created_at FROM sessions")
    ).fetchall()

    for session_id, created_at in sessions:
        existing = bind.execute(
            text(
                "SELECT 1 FROM agents "
                "WHERE session_id = :sid "
                "  AND json_extract(agent_card_json, '$.cafleet.kind') "
                "      = 'builtin-administrator' "
                "LIMIT 1"
            ),
            {"sid": session_id},
        ).fetchone()
        if existing is not None:
            continue

        bind.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, session_id, name, description, status, "
                " registered_at, deregistered_at, agent_card_json) "
                "VALUES (:aid, :sid, :name, :desc, :status, "
                "        :registered_at, NULL, :card)"
            ),
            {
                "aid": str(uuid.uuid4()),
                "sid": session_id,
                "name": "Administrator",
                "desc": f"Built-in administrator agent for session {session_id[:8]}",
                "status": "active",
                "registered_at": created_at,
                "card": _administrator_card(session_id),
            },
        )


def downgrade() -> None:
    bind = op.get_bind()
    bind.execute(
        text(
            "DELETE FROM agents "
            "WHERE json_extract(agent_card_json, '$.cafleet.kind') "
            "      = 'builtin-administrator'"
        )
    )
