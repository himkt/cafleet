"""capitalize root Director name from "director" to "Director"

Revision ID: 0008
Revises: 0007
Create Date: 2026-04-18

Data-only migration. Brings every existing session's root Director into
line with the Administrator's Title-case casing by renaming the stored
``agents.name`` (and the mirror ``agent_card_json.name`` field) from
``"director"`` to ``"Director"`` for the agent referenced by
``sessions.director_agent_id``. New sessions created after this
migration already bootstrap with ``"Director"`` (see
``broker._DIRECTOR_NAME``).

Idempotency:
    The upgrade only rewrites rows whose current ``name`` is exactly
    ``"director"`` and that are the root Director of their session, so
    re-running it against an already-migrated DB is a no-op. User-
    supplied agents named ``"director-1"`` / ``"lonely-director"`` are
    left alone because they are not referenced by
    ``sessions.director_agent_id`` (those placements point at the
    bootstrap director, not user members).
"""

import json
from collections.abc import Sequence

from alembic import op
from sqlalchemy import text

# revision identifiers, used by Alembic.
revision: str = "0008"
down_revision: str | None = "0007"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    bind = op.get_bind()

    rows = bind.execute(
        text(
            "SELECT a.agent_id, a.agent_card_json "
            "FROM agents a "
            "JOIN sessions s ON s.director_agent_id = a.agent_id "
            "WHERE a.name = 'director'"
        )
    ).fetchall()

    for agent_id, card_json in rows:
        try:
            card = json.loads(card_json)
        except (ValueError, TypeError):
            card = None
        if isinstance(card, dict) and card.get("name") == "director":
            card["name"] = "Director"
            new_card = json.dumps(card)
        else:
            new_card = card_json

        bind.execute(
            text(
                "UPDATE agents SET name = 'Director', agent_card_json = :card "
                "WHERE agent_id = :aid"
            ),
            {"card": new_card, "aid": agent_id},
        )


def downgrade() -> None:
    bind = op.get_bind()

    rows = bind.execute(
        text(
            "SELECT a.agent_id, a.agent_card_json "
            "FROM agents a "
            "JOIN sessions s ON s.director_agent_id = a.agent_id "
            "WHERE a.name = 'Director'"
        )
    ).fetchall()

    for agent_id, card_json in rows:
        try:
            card = json.loads(card_json)
        except (ValueError, TypeError):
            card = None
        if isinstance(card, dict) and card.get("name") == "Director":
            card["name"] = "director"
            new_card = json.dumps(card)
        else:
            new_card = card_json

        bind.execute(
            text(
                "UPDATE agents SET name = 'director', agent_card_json = :card "
                "WHERE agent_id = :aid"
            ),
            {"card": new_card, "aid": agent_id},
        )
