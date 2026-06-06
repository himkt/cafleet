# Integer Primary-Key Rearchitecture + `cafleet.db` Rename + Migration Collapse

**Status**: Approved
**Progress**: 0/54 tasks complete
**Last Updated**: 2026-06-06

## Overview

Rearchitect the CAFleet SQLite persistence layer in one breaking "reset the persistence layer" cycle: replace every String-UUID primary/foreign key with a global `INTEGER PRIMARY KEY AUTOINCREMENT`, rename the default DB file `registry.db → cafleet.db`, and collapse all existing Alembic migrations into one fresh schema-only initial migration. The motivation is token efficiency — a UUID is 36 chars (~13-20 tokens) and agents pass 2-4 ids per CLI call across hundreds of calls per fleet, so short integer ids cut a recurring cost. This is a hard break: no data migration, no backward compatibility, no deprecation shims.

## Relationship to 0000074 (hard prerequisite — lands first)

**0000074 (`session`→`fleet` Tier-C rename + base-dir skill self-containment) is a hard prerequisite and lands before 0000075.** 0000075 assumes the codebase is **already fully fleet-named** and does **not** perform any part of that rename. Concretely, when 0000075 begins:

| Surface | Post-0074 state 0000075 builds on |
|---|---|
| Table / PK | `fleets` table, PK column `fleet_id`; `agents.fleet_id` FK → `fleets.fleet_id`; index `idx_agents_fleet_status` |
| ORM model | `Fleet` (in `db/models.py`) |
| Broker fns | `create_fleet` / `get_fleet` / `list_fleets` / `delete_fleet` / `verify_agent_fleet` / `list_fleet_agents` / `_agent_is_active_in_fleet` |
| CLI | global `--fleet-id`; `cafleet fleet create`; `_require_fleet_id`; `_client_command(requires_agent_fleet=...)` |
| WebUI | `X-Fleet-Id` header; `/fleets` route; `get_webui_fleet`; `fleet_id` route params |
| Admin | `FleetPicker.tsx`; `FleetListItem`; `fleetId` / `setFleetId` |
| Docs | `docs/concepts/fleet-isolation.md` (renamed from `session-isolation.md`) |
| Migrations | head is `0011` (0074 added `0011_rename_sessions_to_fleets`) |

**Migration interaction.** 0074 ends with 11 migrations (`0001`–`0011`), head `0011`, cumulative schema = fleet-named TEXT-keyed tables. 0000075 **deletes all eleven** and replaces them with a single fresh `0001_initial_schema.py` (`down_revision=None`) whose target is that same post-0074 cumulative schema **re-expressed with integer PKs**. The new head is `0001`.

The `resolve_agent_ref` / `resolve_task_ref` broker functions are **not** renamed by 0074 (they are agent/task-scoped, fleet-scoped internally); 0000075 deletes them outright.

## Success Criteria

- [ ] All four tables (`fleets`, `agents`, `tasks`, `agent_placements`) key on `INTEGER` columns; minted-id tables (`fleets`, `agents`, `tasks`) use `AUTOINCREMENT` (a `sqlite_sequence` row guarantees ids are never reused).
- [ ] Default DB path is `~/.local/share/cafleet/cafleet.db`; no source, doc, skill, or test references `registry.db`.
- [ ] Exactly one Alembic migration file exists (`0001_initial_schema.py`, `down_revision=None`), and `cafleet db init` on an empty DB produces the current cumulative (fleet-named) schema with integer keys.
- [ ] The entire id-prefix-resolution feature is gone: `broker._resolve_id_prefix` / `resolve_agent_ref` / `resolve_task_ref`, every **id** `[:8]` slice in `output.py` / `cli.py` / `broker.py`, and the `send_inline_preview` `task_id_8` / `sender_8` params no longer exist.
- [ ] CLI id options (`--fleet-id`, `--agent-id`, `--to`, `--id`, `--member-id`, `--task-id`) are typed `int`; passing a non-integer fails with Click's standard "not a valid integer" error.
- [ ] Ids cross the JSON/CLI/HTTP wire as native JSON integers; the admin frontend types them as `number`.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint`, and `mise //admin:build` all pass.
- [ ] `test_cli_prefix_resolution.py` and `test_broker_resolve_ref.py` are deleted; no remaining test asserts UUID-shaped ids, 8-char prefixes, or prefix resolution.

---

## Background

After 0000074, every identifier in CAFleet is still a 36-char UUIDv4 String, generated in Python via `str(uuid.uuid4())` and stored in `TEXT` primary/foreign keys. To make the long ids tolerable in agent output, two compensating features exist:

- **Prefix resolution** (`broker._resolve_id_prefix` and its `resolve_agent_ref` / `resolve_task_ref` wrappers) lets a user paste an 8-char prefix into `--to` / `--id` / `--member-id` / `--task-id`; the broker scans the fleet for a unique match.
- **8-char rendering** (`[:8]` slicing throughout `output.py`, `cli.py` quiet branches, and `broker._try_notify_recipient` → `tmux.send_inline_preview`) shortens displayed ids.

Both exist solely because UUIDs are long. Global integer ids (typically 1-4 digits in practice) are short enough to display and paste in full, so both features become dead weight and are removed entirely. Because ids are globally unique per table (not per-fleet), a single-column FK needs no per-fleet composite and a pasted `--to <id>` resolves to exactly one agent. (Global uniqueness makes the id *lookup* unambiguous; it does **not** permit cross-fleet delivery — `send_message`'s same-fleet guard still rejects a send whose destination lives in another fleet with `Destination agent not in fleet`.)

The pre-collapse migrations include one-time legacy backfills (the original `0002` local-simplification dropping `api_keys`/`tenant_id`, `0006` Administrator seed, `0008` Director-name capitalization, `0010` coding_agent backfill) plus 0074's `0011` rename, whose cumulative effect is exactly the current fleet-named `db/models.py`. Since `broker.create_fleet` seeds the Administrator and Director at runtime, no migration-time seed data is needed; the migrations collapse to a single schema-only `CREATE TABLE` set.

---

## Specification

### 1. ID semantics

| Decision | Resolution |
|---|---|
| PK flavor | `INTEGER PRIMARY KEY AUTOINCREMENT` on `fleets`, `agents`, `tasks`. Emitted by SQLAlchemy via `__table_args__ = (..., {"sqlite_autoincrement": True})`. Creates a `sqlite_sequence` row per table; **ids are never reused**, even after the highest row is (hypothetically) deleted. |
| `agent_placements.agent_id` | `INTEGER PRIMARY KEY` **without** `AUTOINCREMENT`. It is the `agents.agent_id` FK reused as a 1:1 PK — its value is the parent agent's id supplied at insert, not a minted sequence. Applying `AUTOINCREMENT` here is semantically wrong and would break the 1:1, so it is excluded. (This refines the "all four tables" instruction: AUTOINCREMENT applies only to the three minted-id tables.) |
| Wire format | Native JSON integers everywhere — CLI `--json`, WebUI API responses, and inline previews emit real integers. The admin frontend retypes ids `string → number`. |
| CLI typing | Plain `type=int` on all id options including the global `--fleet-id`. Click rejects non-integers with its standard `Error: Invalid value for '...': '<x>' is not a valid integer.` (exit 2). `IntRange` is **not** used — real ids are `>= 1` and `0` is an internal sentinel never passed on the CLI, so no `IntRange` floor is needed. |
| Broker validation | `send_message` replaces `uuid.UUID(to)` with light integer coercion (`int(to)`), raising `ValueError(f"Invalid destination format: {to}")` on failure — preserved for non-CLI callers (WebUI, tests) that may pass a string. |
| Real-id floor | The first `AUTOINCREMENT` value is `1`; real agent ids are always `>= 1`. `0` is therefore free as a sentinel (see §2). |

### 2. Schema (target = post-0074 `db/models.py`, retyped)

`db/models.py` (already `Fleet`-named after 0074) is rewritten as below. `String` stays imported for the text columns (`label`, `created_at`, `name`, `description`, `status`, timestamps, `agent_card_json`, tmux fields, `type`, `text`); `Integer` is added.

```python
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
```

Schema notes:

- When `__table_args__` mixes `Index` objects with the SQLite kwargs dict, **the dict must be last** in the tuple — this is a SQLAlchemy requirement.
- `tasks.to_agent_id` stays `NOT NULL`. The `broadcast_summary` "no single recipient" case stores **`0`** (the user-chosen sentinel; real ids are `>= 1` so `0` never collides). The empty-string `""` used today is invalid under `Integer`.
- `tasks.from_agent_id` / `to_agent_id` remain non-FK (historical tasks outlive their sender). `tasks.origin_task_id` stays a nullable, non-FK `Integer` self-link.

### 3. Single collapsed migration

Delete all of `alembic/versions/0001_*.py` … `0011_*.py` (eleven files post-0074) and replace with one `alembic/versions/0001_initial_schema.py`:

- `revision = "0001"`, `down_revision = None`.
- `upgrade()` issues `op.create_table(...)` for `agents`, `fleets`, `tasks`, `agent_placements` (order chosen so FK targets exist) with `sqlite_autoincrement=True` on the three minted-id tables, plus the four indexes (`idx_agents_fleet_status`, `idx_tasks_context_status_ts`, `idx_tasks_from_agent_status_ts`, `idx_placements_director`). Schema-only — **no seed INSERTs** (the Administrator and Director are created at runtime by `broker.create_fleet`).
- `downgrade()` drops indexes and tables in reverse order.
- `alembic/env.py` is unchanged (it reflects `Base.metadata`).

Because `tasks.context_id` and `fleets.director_agent_id` both FK into `agents`, and `agent_placements` FKs into `agents`, create `agents` before the tables that reference it. `agents.fleet_id` FKs into `fleets`; SQLite tolerates this ordering inside a single migration because FK enforcement is per-connection and the tables are all created before any row is inserted. Mirror the existing migrations' `op.batch_alter_table(...).create_index(...)` idiom.

### 4. Broker — DB-assigned ids (`broker.py`)

With `AUTOINCREMENT`, the DB mints ids on INSERT; the broker no longer calls `uuid.uuid4()` for fleet/agent/task ids. Remove the now-unused `import uuid`.

| Function | Change |
|---|---|
| `create_fleet` | Stop pre-generating `fleet_id` / `director_agent_id` / `administrator_agent_id`. INSERT the `Fleet` row, `flush()`, read the assigned `fleet.fleet_id`; INSERT the Director `Agent`, `flush()`, read `director_agent_id`; INSERT the Director placement; `UPDATE fleets SET director_agent_id`; INSERT the Administrator `Agent`, read `administrator_agent_id`. Build the Administrator card **after** `fleet_id` is known (the description `f"...fleet {fleet_id}"` drops the old `[:8]`). Return the same dict shape with integer ids. |
| `register_agent` | INSERT the `Agent` (and optional placement), read back the assigned `agent_id` (e.g. via `flush()` + attribute, or `.returning(Agent.agent_id)`). Return integer `agent_id`. |
| `send_message` | Replace `uuid.UUID(to)` with `int(to)` coercion (raise `ValueError(f"Invalid destination format: {to}")` on failure). INSERT the unicast task without a `task_id`, read the assigned id, then notify. |
| `broadcast_message` | **Reorder** (ids are DB-assigned, so the summary id is unknown up front): INSERT the summary row first (`to_agent_id=0`, `origin_task_id` temporarily `NULL`), read its `task_id`, `UPDATE` its `origin_task_id` to itself (self-reference), then INSERT each delivery with `origin_task_id = summary_task_id`. Then fire notifications. |
| `_unicast_task_dict` | No longer sets `task_id` (assigned by DB). Callers obtain the id from the INSERT result and merge it into the returned dict. |
| `_save_task` | Split the insert-vs-transition concern: **new tasks** INSERT without `task_id` and return the assigned id; **state transitions** (`ack_task` / `cancel_task`) keep the `task_id`-keyed `ON CONFLICT DO UPDATE` (or plain UPDATE) path. |
| `_try_notify_recipient` | Drop `task_dict["task_id"][:8]` / `sender_id[:8]`; pass full integer ids to `send_inline_preview` (renamed params, §6). |
| `_resolve_id_prefix`, `resolve_agent_ref`, `resolve_task_ref` | **Delete entirely.** |

The `get_task` visibility guard `if to_id:` is **0-correct as-is**: a `broadcast_summary` row's `to_agent_id=0` is falsy (correctly "no recipient endpoint"), and real recipients (`>= 1`) are truthy and pass. `get_agent_names` is only ever called with real recipient ids on the WebUI paths (summary rows are excluded by `_NOT_BROADCAST_SUMMARY` in `list_inbox` / `list_sent` / `list_timeline`), so `0` never reaches the `agent_id IN (...)` lookup.

### 5. Output layer — remove **id** `[:8]` (`output.py`)

Every id is rendered in full. Compact key names are unchanged (`id`, `from`, `origin`) — only the slicing is dropped, and values become integers.

| Site | Change |
|---|---|
| `render_task` | `id`/`from`/`origin` = full integer ids (no `[:8]`). |
| `render_agent` | `id` = full `agent_id`. |
| `format_task` | compact line uses full ids. |
| `format_agent` | compact line uses full `agent_id`. |
| `format_fleet_create` | `director=`/`admin=` use full ids. |
| `format_member` | leading id is full. |
| `_agent_id_for_column` / `format_member_list*` | integer ids are short; render the id directly (drop the 14-char width truncation, or keep a numeric-width column). String-format integers (e.g. `str(m["agent_id"])`) for the `{:<width}` padding. |

**Preserve the NON-id `[:8]`.** `_format_iso_hms` (≈ `output.py:337`, `iso_ts.split("T")[1][:8]`) slices a **time** string to `HH:MM:SS` for `format_member_list_activity` — it is **not** an id slice and **STAYS** (analogous to §10 keeping `MessageInput.tsx`'s text `.slice` calls). The grep-and-delete sweeps in Step 1 / Step 9 must not remove it, or the member-activity table's time rendering breaks.

### 6. Multiplexer inline preview (`multiplexer/tmux.py`, `multiplexer/base.py`)

Rename `send_inline_preview` params `task_id_8 → task_id`, `sender_8 → sender_id` (typed `int`), update the abstract method in `base.py` and its docstring, and update the f-string payload `[cafleet msg {task_id} from {sender_id} {ts}]` to interpolate the integers. No truncation of ids.

### 7. CLI (`cli.py`)

- Add `type=int` to: global `--fleet-id` (on the root `cli` group), `--agent-id` (every subcommand that declares it), `--to`, `--id` (`agent show`), `--member-id`, `--task-id`.
- Fix the global `--fleet-id` help string: drop the `(UUID)` wording (currently `"Fleet ID (UUID); required for client subcommands."`) → integer wording (e.g. `"Fleet ID (integer); required for client subcommands."`).
- Remove the `broker.resolve_agent_ref` / `resolve_task_ref` calls in `message_send`, `message_ack`, `message_cancel`, `message_show`, `agent_show`, and `_load_authorized_member`. The raw integer flows straight to the broker.
- `_load_authorized_member` no longer resolves a prefix; it loads the member by exact integer id. Its "resolved id" comments and the `resolve_agent_ref` `ValueError` re-raise branch are removed.
- `--quiet` branches in `message_send` / `message_ack` print the full integer `task_id` (drop `[:8]`); `member_ping --quiet` prints the full `member_id` (drop `[:8]`).
- `_MEMBER_PROMPT_TEMPLATE` substitution is unaffected (`.format()` handles ints), but the substituted `{fleet_id}` / `{agent_id}` / `{director_agent_id}` values are now integers.

### 8. WebUI API (`webui_api.py`)

- `get_webui_fleet`: the `X-Fleet-Id` header is a string over HTTP; coerce with `int(...)` and raise `HTTPException(400, "X-Fleet-Id must be an integer")` on `ValueError` before calling `broker.get_fleet`. **Retype the return annotation `-> int`** (was `-> str`).
- **Retype the five Depends-injected route params** `fleet_id: int = Depends(get_webui_fleet)` (was `... : str`) on `/agents`, `/agents/{agent_id}/inbox`, `/agents/{agent_id}/sent`, `/timeline`, `/messages/send` — leaving them `str` fails `mise //cafleet:typecheck`.
- Path params: type as `int` (`get_inbox(agent_id: int, ...)`, `get_sent(agent_id: int, ...)`) so FastAPI returns 422 on non-integer paths.
- `SendMessageRequest`: `from_agent_id: int`; `to_agent_id` accepts `int | Literal["*"]` to keep the broadcast sentinel (the `== "*"` branch is unchanged; the unicast branch passes the int through).
- `_format_messages` is structurally unchanged — it only ever sees real recipient ids (summary rows are filtered upstream), so `agent_names[row["to_agent_id"]]` stays safe.

### 9. Config rename (`config.py`)

`_default_database_url` builds `~/.local/share/cafleet/cafleet.db` (was `registry.db`). Update the `Settings.database_url` docstring and the module docstring's example URL. No change to `db/engine.py` (it derives the sync URL from `settings.database_url`).

### 10. Admin frontend (`admin/src/`)

| File | Change |
|---|---|
| `types.ts` | Retype id fields `string → number`: `Agent.agent_id`; `TimelineMessage.task_id` / `from_agent_id` / `to_agent_id` / `origin_task_id` (`number \| null`); `FleetListItem.fleet_id`. Name fields stay `string`. |
| `api.ts` | `fleetId` / `setFleetId` accept a `number`; stringify at the `X-Fleet-Id` header boundary (headers are strings). `sendMessage(fromAgentId: number, toAgentId: number \| "*", ...)`. |
| `Dashboard.tsx` | `{fleetId.slice(0, 8)}` → `{fleetId}`. |
| `FleetPicker.tsx` | `{f.fleet_id.slice(0, 8)}` → `{f.fleet_id}` (renamed from `SessionPicker.tsx` by 0074). |
| `ReactionBar.tsx` | `agentId.slice(0, 8)` → `String(agentId)`. |
| `MessageInput.tsx` | Its `.slice(...)` calls are text/cursor/list operations (not id display) and **stay**. Adjust only where it consumes/inserts an `agent_id` value (mentions) if the `number` retype requires it. |

### 11. Hard-break upgrade note (docs)

There is **no data migration and no backward compatibility**. An existing `registry.db` is incompatible and is simply orphaned by the rename (a fresh `cafleet.db` is created by `cafleet db init`). Document, in `docs/get-started/install.md` and `docs/concepts/storage.md`:

> **Upgrading across the integer-PK rearchitecture (0000075):** delete any pre-existing database. The default file moved from `~/.local/share/cafleet/registry.db` to `~/.local/share/cafleet/cafleet.db`, so the old file is left untouched and ignored — remove it manually. If you set `CAFLEET_DATABASE_URL` to a custom path holding an old (UUID-era) schema, `cafleet db init` refuses to run against its unknown Alembic revision; delete that file and re-run `cafleet db init`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> Per `.claude/rules/design-doc-numbering.md`, **documentation is updated before any code.**
> All names below are the post-0074 fleet names (see "Relationship to 0000074").

### Step 1: Documentation (docs / README / skills) — FIRST

- [ ] `docs/spec/data-model.md`: retype every PK/FK row to `INTEGER` `AUTOINCREMENT`; document the `sqlite_sequence` table and "ids never reused"; document `tasks.to_agent_id=0` broadcast_summary sentinel (and that real ids are `>= 1`); note `agent_placements.agent_id` is the agents FK reused as PK (no AUTOINCREMENT); replace "UUID v4" notes; update the Administrator-seed paragraph (now seeded only at runtime by `create_fleet`, not by a migration); reflect the single-migration collapse; `registry.db → cafleet.db`. Keep the `### Session ownership` (ORM) + "tmux session" wording untouched. <!-- completed: -->
- [ ] `docs/spec/message-envelope.md`: id fields are integers; remove any 8-char/prefix language. <!-- completed: -->
- [ ] `docs/spec/cli-options.md`: document `type=int` id options; remove every prefix-resolution / "unique prefix" / "8-char" row, the resolver error-message rows, and the "Human-facing … truncate IDs to an 8-char prefix" paragraph; `--fleet-id` help wording `(UUID)` → integer; `registry.db → cafleet.db`. <!-- completed: -->
- [ ] `docs/spec/webui-api.md`: retype the API id fields (`fleet_id`, `agent_id`, `task_id`, `from_agent_id`, `to_agent_id`, `origin_task_id`) from `"uuid"` to integers in every example; keep the `to_agent_id="*"` broadcast sentinel; the `X-Fleet-Id` header value is the integer fleet id. <!-- completed: -->
- [ ] `docs/concepts/overview.md`: `registry.db → cafleet.db`; integer-id model. <!-- completed: -->
- [ ] `docs/concepts/storage.md`: `registry.db → cafleet.db`; single-migration / schema-collapse description; add the §11 upgrade note. <!-- completed: -->
- [ ] `docs/concepts/token-reduction.md`: rewrite the compact-envelope row — replace "8-char IDs … pasteable back … via prefix resolution on `--to`/`--id`/`--member-id`/`--task-id`" with "full integer ids (short by construction; no prefix resolution)". <!-- completed: -->
- [ ] `docs/concepts/fleet-isolation.md`: integer-id references; confirm no prefix language. <!-- completed: -->
- [ ] `docs/get-started/install.md`: `registry.db → cafleet.db`; add the §11 "delete old DB" upgrade note. <!-- completed: -->
- [ ] `docs/get-started/configure.md`: `CAFLEET_DATABASE_URL` default → `cafleet.db`. <!-- completed: -->
- [ ] `docs/reference/coding-agents/codex.md`: `registry.db → cafleet.db`. <!-- completed: -->
- [ ] `README.md`: `registry.db → cafleet.db`; integer-id model; remove prefix-resolution mentions (use `/update-readme` if the surface is large). <!-- completed: -->
- [ ] `skills/cafleet/SKILL.md`: `registry.db → cafleet.db`; remove the reserved-prefix / "8-char prefix" language (e.g. the `--quiet` rows and inline-preview header description); integer-id examples; confirm `--fleet-id`/`--agent-id`/`--to`/`--task-id` examples read as integers. <!-- completed: -->
- [ ] `skills/cafleet/reference/director.md`: integer-id examples; remove prefix mentions. <!-- completed: -->
- [ ] `skills/cafleet/reference/broadcast.md`: integer-id / `origin_task_id` examples; remove prefix mentions. <!-- completed: -->
- [ ] `skills/cafleet/reference/output-flags.md`: remove `--quiet` "8-char prefix" and any prefix-resolution language. <!-- completed: -->
- [ ] Grep the repo for residual `registry.db`, id-`[:8]`, "prefix", "8-char", "uuid" in docs/skills to confirm no stragglers — but do NOT flag the time-slice `[:8]` in `_format_iso_hms` (it is not an id slice; see §5). <!-- completed: -->

### Step 2: Schema, migration, config

- [ ] Rewrite `db/models.py` per §2 (add `Integer` import; retype all PKs/FKs; add `sqlite_autoincrement` to `fleets`/`agents`/`tasks`; keep `agent_placements` PK plain). <!-- completed: -->
- [ ] Delete `alembic/versions/0001_*.py` … `0011_*.py`. <!-- completed: -->
- [ ] Add `alembic/versions/0001_initial_schema.py` (`down_revision=None`, schema-only, `sqlite_autoincrement=True` on the three minted-id tables, all indexes, FK-safe create order). <!-- completed: -->
- [ ] `config.py`: `_default_database_url` → `cafleet.db`; update docstrings. <!-- completed: -->

### Step 3: Broker

- [ ] Remove `import uuid`; stop generating ids in `create_fleet` / `register_agent` / `send_message` / `broadcast_message`; read DB-assigned ids via `flush()`/`returning`. <!-- completed: -->
- [ ] `create_fleet`: reorder so the Administrator card description is built after `fleet_id` is assigned (drop `[:8]`). <!-- completed: -->
- [ ] `broadcast_message`: reorder to insert-summary-first → self-reference `origin_task_id` → deliveries; `to_agent_id=0` on the summary. <!-- completed: -->
- [ ] `send_message`: replace `uuid.UUID(to)` with `int(to)` coercion + preserved error message. <!-- completed: -->
- [ ] Split `_save_task` / adjust `_unicast_task_dict` for insert-without-id vs transition-with-id. <!-- completed: -->
- [ ] `_try_notify_recipient`: pass full integer `task_id` / `sender_id` (drop `[:8]`). <!-- completed: -->
- [ ] Delete `_resolve_id_prefix`, `resolve_agent_ref`, `resolve_task_ref`. <!-- completed: -->

### Step 4: Output

- [ ] Remove every **id** `[:8]` in `render_task`, `render_agent`, `format_task`, `format_agent`, `format_fleet_create`, `format_member`, `_agent_id_for_column`; string-format integer ids for column padding. **Leave `_format_iso_hms`'s time `[:8]` intact** (§5). <!-- completed: -->

### Step 5: CLI

- [ ] Add `type=int` to `--fleet-id` (root group), `--agent-id`, `--to`, `--id`, `--member-id`, `--task-id`; fix the `--fleet-id` help string `(UUID)` → integer. <!-- completed: -->
- [ ] Remove `resolve_agent_ref` / `resolve_task_ref` calls in `message_send`/`ack`/`cancel`/`show`, `agent_show`, `_load_authorized_member`. <!-- completed: -->
- [ ] Drop id `[:8]` in the `--quiet` branches of `message_send` / `message_ack` and in `member_ping --quiet`. <!-- completed: -->

### Step 6: Multiplexer

- [ ] Rename `send_inline_preview` params (`task_id_8 → task_id`, `sender_8 → sender_id`, typed `int`) in `multiplexer/tmux.py` and `multiplexer/base.py` (+ docstrings); update the payload f-string. <!-- completed: -->

### Step 7: WebUI API

- [ ] `get_webui_fleet`: int-coerce `X-Fleet-Id` (400 on non-int); retype return `-> int`. <!-- completed: -->
- [ ] Retype the five `fleet_id: int = Depends(get_webui_fleet)` route params (was `str`). <!-- completed: -->
- [ ] Type path params `agent_id: int` on inbox/sent routes. <!-- completed: -->
- [ ] `SendMessageRequest.from_agent_id: int`; `to_agent_id: int | Literal["*"]`. <!-- completed: -->

### Step 8: Admin frontend

- [ ] `types.ts`: retype id fields `string → number` (`origin_task_id: number | null`; `FleetListItem.fleet_id: number`). <!-- completed: -->
- [ ] `api.ts`: `setFleetId`/`fleetId` as `number`; stringify at the `X-Fleet-Id` header; `sendMessage` param types. <!-- completed: -->
- [ ] `Dashboard.tsx`, `FleetPicker.tsx`, `ReactionBar.tsx`: remove id `.slice(0, 8)` display logic. <!-- completed: -->
- [ ] `MessageInput.tsx`: adjust agent-id consumption (mentions) for the `number` retype; leave text `.slice` calls intact. <!-- completed: -->

### Step 9: Tests

- [ ] Delete `tests/test_cli_prefix_resolution.py` and `tests/test_broker_resolve_ref.py`. <!-- completed: -->
- [ ] Update shared fixtures/helpers (`conftest.py`, `_helpers.py`, `_broker_helpers.py` incl. `_create_fleet`, `_member_cli_helpers.py`) to use DB-assigned integer ids (stop fabricating `str(uuid.uuid4())` ids; capture ids returned by `create_fleet` / `register_agent`). <!-- completed: -->
- [ ] Update `test_alembic_smoke.py` for the single migration (one revision, head == `0001`). <!-- completed: -->
- [ ] Update inline-preview tests (`test_broker_inline_preview.py`, `test_multiplexer_tmux_send_inline_preview.py`) for the renamed params / integer ids. <!-- completed: -->
- [ ] Update output tests (`test_output_render_task.py`, `test_output_render_agent.py`, `test_output_compact_formatters.py`, `test_output_render_broadcast_summary.py`, `test_output.py`) for full integer ids; confirm the `_format_iso_hms` time render still asserts `HH:MM:SS`. <!-- completed: -->
- [ ] Update CLI tests (`test_cli_message.py`, `test_cli_agent.py`, `test_cli_member*.py`, `test_cli_compact_echo.py`, `test_cli_message_truncation.py`, `test_cli_fleet_flag.py`, `test_cli_fleet_bootstrap.py`, `test_cli_help_budget.py`) for `type=int` options, integer ids, removed prefix help, and `registry.db → cafleet.db`. <!-- completed: -->
- [ ] Update broker/webui tests (`test_broker_messaging.py`, `test_broker_registry.py`, `test_broker_administrator.py`, `test_broker_typed_columns.py`, `test_broker_webui.py`, `test_broker_member_activity.py`, `test_webui_api_format.py`, `test_server_routing.py`) for integer ids and the `to_agent_id=0` sentinel. <!-- completed: -->
- [ ] Update `test_db_init.py` and fleet tests (`test_fleet_cli.py`, `test_fleet_bootstrap.py`, `test_fleet_list_director.py`) for the new default path and integer ids. <!-- completed: -->
- [ ] Grep the test tree for residual `uuid`, id-`[:8]`, `registry.db`, and prefix assertions. <!-- completed: -->

### Step 10: Verification

- [ ] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck` pass. <!-- completed: -->
- [ ] `mise //cafleet:test` passes. <!-- completed: -->
- [ ] `mise //admin:lint` and `mise //admin:build` pass. <!-- completed: -->
- [ ] Smoke: a fresh `cafleet db init` creates `cafleet.db` at head `0001`; `sqlite_sequence` exists for `fleets`/`agents`/`tasks`; `fleet create` → `member create` → `message send` → `poll` → `ack` round-trips with integer ids. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-06 | Initial draft |
| 2026-06-06 | Rewrote entirely in post-0074 `fleet` naming (0000074 is now a hard prerequisite that lands first); added "Relationship to 0000074" section; resolved 6 Reviewer markers — cross-fleet wording, plain `type=int` (no `IntRange`) + `--fleet-id` help fix, preserve `_format_iso_hms` time-slice, retype `get_webui_fleet`/route params to `int`, add `docs/spec/webui-api.md` to the doc-update list |
| 2026-06-06 | Resolved 2 Reviewer nits — Progress recounted to the implementation-task total (0/54); §3 "three indexes" → "four indexes" |
| 2026-06-06 | User approved; Status → Approved. Spec frozen; ready for implementation (0/54 tasks) |
