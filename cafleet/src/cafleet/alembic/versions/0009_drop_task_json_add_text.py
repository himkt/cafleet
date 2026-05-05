"""drop tasks.task_json blob, add tasks.text typed column

Revision ID: 0009
Revises: 0008
Create Date: 2026-05-05

Surface 14 of design 0000049: replace the redundant ``Task.task_json`` JSON
blob with a typed ``Task.text`` column. The other typed columns
(``task_id``, ``context_id``, ``from_agent_id``, ``to_agent_id``, ``type``,
``created_at``, ``status_state``, ``status_timestamp``, ``origin_task_id``)
were already promoted in earlier revisions; this migration removes the last
JSON blob so every read path is a direct typed-column SELECT.

Migration shape (single revision, no in-between binary state):

1. Pre-flight check — assert every existing row has a non-NULL body at
   ``json_extract(task_json, '$.artifacts[0].parts[0].text')``. Abort with
   ``RuntimeError`` if any row violates; the operator must repair or remove
   the bad row before re-running ``cafleet db init``.
2. Add ``tasks.text`` column nullable (so the backfill UPDATE can populate
   it before NOT NULL is enforced).
3. Backfill ``tasks.text`` from the JSON blob via ``json_extract``.
4. Tighten ``tasks.text`` to ``NOT NULL`` and drop ``tasks.task_json`` in a
   single ``batch_alter_table`` so SQLite's table rebuild happens once.

Migration risk: irreversible without backup. Operators MUST take a backup
before running ``cafleet db init`` against a populated database — see
``docs/spec/data-model.md`` § ``tasks.task_json`` removal for the recipe.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op
from sqlalchemy import text

# revision identifiers, used by Alembic.
revision: str = "0009"
down_revision: str | None = "0008"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    bind = op.get_bind()

    bad_rows = bind.execute(
        text(
            "SELECT task_id FROM tasks "
            "WHERE json_extract(task_json, '$.artifacts[0].parts[0].text') IS NULL"
        )
    ).fetchall()
    if bad_rows:
        bad_ids = ", ".join(r[0] for r in bad_rows[:5])
        suffix = f" (and {len(bad_rows) - 5} more)" if len(bad_rows) > 5 else ""
        raise RuntimeError(
            "Pre-flight check failed for design 0000049 Surface 14 migration: "
            f"{len(bad_rows)} task row(s) have a NULL body at "
            "json_extract(task_json, '$.artifacts[0].parts[0].text'). "
            f"Offending task_id(s): {bad_ids}{suffix}. "
            "Restore from the pre-0049 backup, repair or remove the offending "
            "rows, and re-run 'cafleet db init'."
        )

    with op.batch_alter_table("tasks") as batch_op:
        batch_op.add_column(sa.Column("text", sa.String(), nullable=True))

    bind.execute(
        text(
            "UPDATE tasks "
            "SET text = json_extract(task_json, '$.artifacts[0].parts[0].text')"
        )
    )

    with op.batch_alter_table("tasks") as batch_op:
        batch_op.alter_column("text", existing_type=sa.String(), nullable=False)
        batch_op.drop_column("task_json")


def downgrade() -> None:
    raise NotImplementedError(
        "Surface 14 migration is forward-only. The task_json blob carried "
        "the legacy nested envelope shape (artifacts/parts/metadata/contextId/"
        "history) which the typed columns + text column do not preserve. "
        "Restore from the pre-0049 backup if you need to revert: "
        "cp ~/.local/share/cafleet/registry.db.pre-0049.bak "
        "~/.local/share/cafleet/registry.db"
    )
