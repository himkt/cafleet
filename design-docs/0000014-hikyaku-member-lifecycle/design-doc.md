# Hikyaku Member Lifecycle

**Status**: Approved
**Progress**: 6/44 tasks complete
**Last Updated**: 2026-04-12

## Overview

Introduce a `hikyaku member` CLI subcommand group that wraps the two-step "register an agent + spawn a tmux pane" recipe behind a single command, and persists the `agent_id ↔ tmux pane` mapping in the registry SQLite store. Director sessions no longer shell out to `tmux` directly, so five `Bash(tmux …)` allow entries can be removed from `.claude/settings.json`. `member create` also auto-targets the Director's own tmux window via `TMUX_PANE` + `tmux display-message`, fixing the bug where panes are created in whichever window the user happens to be focused on.

## Success Criteria

- [ ] `.claude/settings.json` contains zero `Bash(tmux *)` allow entries and zero `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` entry
- [ ] `hikyaku member create --agent-id <director> --name <n> --description <d> -- "<prompt>"` atomically (a) registers a new agent, (b) writes an `agent_placements` row, and (c) spawns a `claude` pane in the Director's own tmux window
- [ ] If step (c) fails, step (a) is rolled back via `DELETE /api/v1/agents/{new_id}` before the CLI exits non-zero
- [ ] The spawned pane is created in the Director's window regardless of which window the user is currently focused on, by resolving `tmux display-message -p -t "$TMUX_PANE" '#{window_id}'` at create time
- [ ] `hikyaku member delete --agent-id <director> --member-id <target>` reads the placement row, deregisters the agent (which cascade-deletes the placement row), **then** sends `/exit` to the pane — in that order, so a deregister failure leaves both agent and pane intact for safe retry
- [ ] `hikyaku member list --agent-id <director>` returns all agents currently bound to a placement row whose `director_agent_id` equals `<director>`, tenant-scoped
- [ ] A new `agent_placements` table exists in the registry schema, declared in `db/models.py` and created by an Alembic migration (`0002_add_agent_placements.py`) that is idempotent under `db init`
- [ ] The `DELETE /api/v1/agents/{agent_id}` endpoint accepts the request when `X-Agent-Id` equals either the target `agent_id` OR the `director_agent_id` recorded on the target's placement row
- [ ] Every affected documentation file is updated *before* any code is written: `ARCHITECTURE.md`, `docs/`, `README.md`, `.claude/skills/hikyaku/SKILL.md`, `plugins/*/skills/hikyaku/SKILL.md`, and this design doc itself
- [ ] `hikyaku member list` output includes `status`, `session`, `window_id`, `pane_id` columns (text) and matching fields (JSON), sourced from `agent_placements` joined into `agents`
- [ ] `hikyaku member capture --agent-id <director> --member-id <target>` returns the last N lines of the target pane via the same `tmux.py` subprocess helper module, with cross-Director capture rejected at the CLI layer (defense-in-depth check against `placement.director_agent_id`; server-side enforcement is limited to tenant scoping by design)
- [ ] Raw `tmux capture-pane` and `tmux list-panes` invocations are no longer documented in `.claude/skills/hikyaku/SKILL.md` — the skill points users at `hikyaku member list` / `hikyaku member capture` instead
- [ ] `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //registry:test`, and `mise //client:test` all pass

## Non-goals

- **Non-tmux environments** (screen, plain shell, tmate, VS Code integrated terminal). `hikyaku member create` errors out immediately when `TMUX` is unset. No graceful degradation.
- **Cross-Director member handover.** `placement.director_agent_id` is set at create time and is immutable. No "adopt this member into a new team" command.
- **Automatic dead-pane detection / janitor process.** The CLI is the only cleanup path. If a user kills a pane with `tmux kill-pane` directly, the placement row stays until the next `hikyaku member delete` resolves it.
- **`hikyaku team …` CLI noun.** Rejected for v1 per the Conceptual Model section. May be revisited if team-level data (shared inbox, team name, shared config) emerges.
- **`--prompt` / `--skills` flags on `member create`.** Deferred by user request. Prompt customization uses a trailing positional after `--`; skills cannot be set on member-class agents in v1.
- **Alembic `db revision` / `db downgrade` / `db current` commands.** Inherited non-goal from 0000010-sqlite-store-migration. The new migration is applied via the existing `hikyaku-registry db init` path only.
- **Cross-Director pane capture or list.** A Director can only `member capture` / `member list` agents whose `placement.director_agent_id` matches its own `agent_id`. Seeing into another Director's team — even within the same tenant — is deliberately out of scope. Enforced at the API layer by the same caller-check that gates `member delete`.
- **Continuous pane streaming.** `hikyaku member capture` is strictly one-shot: it returns the last N lines and exits. There is no `tmux pipe-pane` equivalent, no follow mode, no server-side buffering. A Director that wants to watch a pane live must re-invoke `member capture` in a loop externally.

---

## Background

### Current pain points

1. **Raw tmux in the permission allow list.** `.claude/settings.json:15-19` contains five allow entries that only exist so the Director can execute the multi-step spawn/shutdown recipe documented in `.claude/skills/hikyaku/SKILL.md` (Multi-Session Coordination). Every allow entry is a miniature trust surface and the user explicitly wants them gone.

2. **Literal env paste.** The recipe forces the Director to run `printenv HIKYAKU_URL HIKYAKU_API_KEY`, read the output, and paste the literal strings back into `tmux split-window -e`. Shell variable expansion inside the `Bash` tool fails silently or is blocked by the validator in some configurations, so the documented workaround is "copy-paste the concrete values". This is error-prone and annoying.

3. **Wrong-window pane creation.** `tmux split-window` without a `-t <target>` creates the new pane in whichever window the user is currently focused on, not the Director's window. If the user is looking at a different window when the Director fires the command, the pane lands in the wrong place and the `main-vertical` rebalance ruins a layout the user cares about.

4. **Two-step registration.** In the current recipe, the spawned Claude is expected to run `hikyaku register` itself inside the new pane. If that `register` call fails (broker down, bad API key, etc.), the pane is alive but unregistered, and the Director has no feedback channel — it already released control of the pane. There is no atomicity.

5. **No server-side mapping from agent_id to pane.** The Director has no durable record of which pane corresponds to which member agent. Today that mapping only exists in the Director's short-term memory (the chat transcript). If the Director session crashes or a new Director takes over, the mapping is lost, and the only way to clean up is to `tmux kill-session` or manually match pane contents to broker registrations.

### Fixing (3) requires `TMUX_PANE`

Claude Code sessions run inside a tmux pane, so the `TMUX_PANE` environment variable is always set in the Director's process (e.g. `%0`). From `TMUX_PANE` the wrapper can resolve the Director's own window id without caring about user focus:

```bash
tmux display-message -p -t "$TMUX_PANE" '#{window_id}'   # → e.g. '@3'
```

The wrapper then passes `-t @3` to `tmux split-window`, which targets the Director's window specifically.

---

## Specification

### Conceptual model

The user explicitly asked for a conceptual model exercise, not just two commands. This section enumerates the nouns that were considered and records which ones became CLI surface.

#### Nouns considered

| Noun | Definition | First-class CLI noun? |
|---|---|---|
| **agent** | Any registered entity in the registry. Pre-existing concept. | Yes (existing: `hikyaku register`, `hikyaku deregister`, `hikyaku agents`) |
| **member** | An agent that was spawned by a Director via `hikyaku member create`, and therefore has an associated placement row (tmux pane, window, session, director back-reference). "Member" is a **role an agent plays**, not a distinct DB entity. | Yes — `hikyaku member create/delete/list` |
| **placement** | The metadata row that binds a member agent to a specific tmux pane. Internal concept. | No — not a CLI noun, never surfaced directly |
| **team** | The collection of a Director and the members it has spawned into the same tmux window. Fully derivable as `SELECT * FROM agent_placements WHERE director_agent_id = <director>`. | **No — rejected for v1** |
| **session** / **window** | tmux concepts. Stored on the placement row so `member list` can show them, but never exposed as a subcommand. | No |

#### Why `team` is NOT a CLI noun in v1

The user mentioned "team とか member とかいくつか重要な概念がある", and we seriously considered adding `hikyaku team` commands. We rejected it for three reasons:

1. **No independent team state.** A team has no name, no shared inbox, no config, and no lifecycle separate from its constituent agents. Every queryable fact about a team is a fact about its members. Adding a `teams` table would be storing nothing.
2. **`team create` is redundant with `register`.** A team exists the moment a Director registers and starts calling `member create`. There is no explicit creation point.
3. **`team destroy` is a thin convenience.** The only operation that would genuinely need a `team` noun is "shut down all my members at once". This is trivially scriptable as `hikyaku --json member list --agent-id $D | jq -r '.[].agent_id' | xargs -I {} hikyaku member delete --agent-id $D --member-id {}`, and the extra surface area is not justified until usage shows the one-liner is clumsy.

`team` remains available as a future CLI noun if real team-level data emerges (shared broadcast inbox, team name, team-wide config, Director handover). The current design does not foreclose that extension — adding a `teams` table later is cheap because `agent_placements` already carries the per-member data.

#### Why `member` IS a CLI noun

- It corresponds to a concrete lifecycle (`create` → pane runs → `delete`) that is currently a 5-step manual recipe in SKILL.md.
- It has distinct input/output shapes from `register` (create needs `--director` context + tmux targeting; register does not).
- It maps cleanly onto the new `agent_placements` table: a "member" is exactly "an agent with a placement row".

#### Director self-registration stays on `hikyaku register`

The Director is **not** a member of its own team. Directors register themselves with plain `hikyaku register --name <n> --description <d>` and do not create an `agent_placements` row for themselves. There is no `hikyaku member create --self` form. Rationale: the Director already knows its own `TMUX_PANE`, so there is nothing to store, and introducing a self-registration variant forces every other command to handle "is this call from a member or a director?" which adds branching for no user-visible win.

### Command surface

```
hikyaku register                --name ... --description ... [--skills ...]      # unchanged
hikyaku deregister              --agent-id ...                                   # unchanged
hikyaku agents                  --agent-id ... [--id ...]                        # unchanged
hikyaku send | broadcast | ...  ...                                              # unchanged

hikyaku member create           --agent-id <director-id>
                                --name <member-name>
                                --description <member-description>
                                [-- <prompt>]                                    # NEW
hikyaku member delete           --agent-id <director-id>
                                --member-id <target-agent-id>                    # NEW
hikyaku member list             --agent-id <director-id>                         # NEW
hikyaku member capture          --agent-id <director-id>
                                --member-id <target-agent-id>
                                [--lines <n>]                                    # NEW
```

#### `hikyaku member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **Director's** agent_id, sent as `X-Agent-Id` on the HTTP call. Becomes `director_agent_id` on the new placement row. |
| `--name` | yes | Display name of the new member agent (e.g. `Claude-B`). Passed verbatim to `POST /api/v1/agents`. |
| `--description` | yes | One-sentence purpose of the new member. |
| *(positional, after `--`)* | no | Prompt text handed to the spawned `claude` process as its trailing argument. If omitted, a default prompt is used (see below). No `--prompt` flag — positional-after-`--` is the intentional shape. |

**Prompt handling.** The user explicitly did not want a `--prompt` flag ("`--prompt --skills` とかはまだいらなくない"). We chose **trailing positional argument after `--`** as the mechanism because it is structurally not a flag (the user's stated objection), it is a well-understood CLI convention (`docker run image -- arg`, `xargs -- cmd`), and it keeps the command self-contained without stdin/tempfile plumbing. If the positional is omitted, the CLI uses a default prompt:

```
Load Skill(hikyaku). Your agent_id is $HIKYAKU_AGENT_ID. You are a member of the team led by <director-name> (<director-agent-id>). Wait for instructions from the director via `hikyaku poll --agent-id $HIKYAKU_AGENT_ID`.
```

Where `<director-name>` / `<director-agent-id>` are filled in from a `GET /api/v1/agents/<director-id>` lookup the Director's own CLI issues before calling `tmux split-window`.

**Example.** The two invocations below are equivalent except for the prompt:

```bash
hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
  --description "Reviewer bot for PR #42"

hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
  --description "Reviewer bot for PR #42" \
  -- "Review PR #42, post feedback via send, and deregister on completion."
```

**Output (non-JSON).**

```
Member registered and spawned.
  agent_id:  <new-uuid>
  name:      Claude-B
  pane_id:   %7
  window_id: @3
```

**Output (`--json`).**

```json
{
  "agent_id": "<uuid>",
  "name": "Claude-B",
  "registered_at": "2026-04-12T10:15:00Z",
  "placement": {
    "director_agent_id": "<director-uuid>",
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": "%7",
    "created_at": "2026-04-12T10:15:00Z"
  }
}
```

The `api_key` field is NOT included in the output — just as `hikyaku register` already strips it in the `--json` path (cli.py:77-79).

#### `hikyaku member delete`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **Director's** agent_id. Sent as `X-Agent-Id`. |
| `--member-id` | yes | The target member's agent_id. This is what gets deregistered. |

No `--name` lookup. The user chose agent_id for simplicity: "delete は agent_id がいいかな". Members whose `agent_id` the Director has forgotten can be recovered via `hikyaku member list --agent-id <director>`.

**Output (non-JSON).**

```
Member deleted.
  agent_id:  <target-uuid>
  pane_id:   %7 (closed)
```

If the pane is already gone when delete runs, the line reads `%7 (already closed)` instead. See Error Handling below.

#### `hikyaku member list`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **Director's** agent_id. Only placement rows whose `director_agent_id` equals this are returned, tenant-scoped by the authenticated API key. |

This is the DB-backed substitute for `tmux list-panes -a` — it replaces the raw tmux invocation for discovering active teammate pane ids. Columns are sourced from `agents` joined with `agent_placements`:

| Column | Source |
|---|---|
| `agent_id` | `agents.agent_id` |
| `name` | `agents.name` |
| `status` | `agents.status` (always `active` in v1 — deregistered members are not listed) |
| `session` | `agent_placements.tmux_session` |
| `window_id` | `agent_placements.tmux_window_id` |
| `pane_id` | `agent_placements.tmux_pane_id` — or `(pending)` if `NULL` (see Pending placement state) |
| `created_at` | `agents.registered_at` |

**Output (non-JSON).**

```
2 members:
  agent_id        name      status  session  window_id  pane_id  created_at
  --------------  --------  ------  -------  ---------  -------  --------------------
  ffa1c0b2-…      Claude-B  active  main     @3         %7       2026-04-12T10:15:00Z
  0a2e8b7d-…      Claude-C  active  main     @3         %9       2026-04-12T10:22:13Z
```

**Output (`--json`).** Array of objects with the shape:

```json
[
  {
    "agent_id": "<uuid>",
    "name": "Claude-B",
    "status": "active",
    "registered_at": "2026-04-12T10:15:00Z",
    "placement": {
      "director_agent_id": "<director-uuid>",
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%7",
      "created_at": "2026-04-12T10:15:00Z"
    }
  }
]
```

#### `hikyaku member capture`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **Director's** agent_id. Sent as `X-Agent-Id`. Used both to authenticate and to enforce the cross-Director boundary. |
| `--member-id` | yes | The target member agent_id whose pane should be captured. |
| `--lines` | no | Number of trailing terminal lines to capture. Default: `80`. Passed to `tmux capture-pane -S -<lines>`. |

This is the DB-backed substitute for the raw `tmux capture-pane -p -t <pane> -S -<n>` invocation currently documented in the external `agent-team-supervision` skill's "Stall Response" section. It is the canonical way for a Director to inspect a stalled teammate inside this project.

**Behavior.**

1. Validate `HIKYAKU_API_KEY` (same guard as every authenticated command).
2. Validate `TMUX` / `TMUX_PANE` via `tmux.ensure_tmux_available()` — capture without tmux makes no sense.
3. Fetch the target's placement: `GET /api/v1/agents/<member-id>`. The existing handler already enforces tenant scope. Additionally, the CLI verifies that the returned `placement.director_agent_id` equals `--agent-id`. If it does not, exit 1 with `"agent {id} is not a member of your team"`. This is the cross-Director boundary — enforced client-side as a defense-in-depth check, even though the server would also deny a delete/patch attempt.
4. If `placement.tmux_pane_id` is `NULL` (pending row), exit 1 with `"member has no pane yet (pending placement) — nothing to capture"`. No rollback is attempted.
5. Call `tmux.capture_pane(pane_id=..., lines=...)` from the tmux helper module.
6. Print the captured content.

**Output (non-JSON).** The raw captured terminal buffer — bytes as tmux emitted them, printed to stdout. No framing, no prefix, no trailing newline beyond what tmux produced. This matches the shape the user gets today from `tmux capture-pane -p` directly, so existing eyeballing habits transfer.

**Output (`--json`).** Single object:

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "lines": 80,
  "content": "...<raw buffer>..."
}
```

**Example.**

```bash
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID --lines 200
hikyaku --json member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID | jq -r .content
```

### Data model

A single new table, `agent_placements`, stores the placement metadata. The table is **additive** — no existing column is touched.

```
agent_placements
  agent_id            TEXT PRIMARY KEY
                        REFERENCES agents(agent_id)
                        ON DELETE CASCADE
  director_agent_id   TEXT NOT NULL
                        REFERENCES agents(agent_id)
                        ON DELETE RESTRICT
  tmux_session        TEXT NOT NULL         -- e.g. 'main', from `tmux display-message '#{session_name}'`
  tmux_window_id      TEXT NOT NULL         -- e.g. '@3', from `#{window_id}`
  tmux_pane_id        TEXT                  -- e.g. '%7'; NULLABLE. NULL = pending (row inserted during register, pane not yet captured). Set by the PATCH call after split-window succeeds.
  created_at          TEXT NOT NULL         -- ISO-8601
  INDEX idx_placements_director (director_agent_id)
```

**SQLAlchemy declaration** (add to `registry/src/hikyaku_registry/db/models.py`):

```python
class AgentPlacement(Base):
    __tablename__ = "agent_placements"

    agent_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="CASCADE"),
        primary_key=True,
    )
    director_agent_id: Mapped[str] = mapped_column(
        String,
        ForeignKey("agents.agent_id", ondelete="RESTRICT"),
        nullable=False,
    )
    tmux_session: Mapped[str] = mapped_column(String, nullable=False)
    tmux_window_id: Mapped[str] = mapped_column(String, nullable=False)
    tmux_pane_id: Mapped[str | None] = mapped_column(String, nullable=True)   # NULL = pending; see "Pending placement state" below
    created_at: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_placements_director", "director_agent_id"),)
```

**Schema decision rationale.**

| Alternative | Why rejected |
|---|---|
| Add `tmux_pane_id` / `tmux_window_id` columns directly to `agents` | Pollutes the base `Agent` row with placement metadata that only applies to tmux-spawned members. Every `Agent` query has to remember "ignore those columns unless this is a member". |
| New `teams` table + `team_members` join table | Dead weight — teams have no independent data to store (see Conceptual Model). Can be added later without touching `agent_placements`. |
| Store placement in a local JSON file (`~/.local/state/hikyaku/members.json`) | User explicitly requested "DB にデータ持たせられない？". A local file is not visible to a replacement Director or to the admin WebUI, and drifts from the broker state if the broker is rebuilt. |

**Foreign key semantics.**

- `agent_id` uses `ON DELETE CASCADE`. The registry uses soft deletion via `status='deregistered'` for agents today, so this cascade is never triggered by the current soft-delete path, but it protects against accidental hard-delete paths added later.
- `director_agent_id` uses `ON DELETE RESTRICT`. A Director cannot be hard-deleted while it still owns live placement rows. This is a sanity guard — again, the current code doesn't hard-delete, but the constraint documents the invariant.
- Tenant scoping is **implicit**: both `agent_id` and `director_agent_id` FK into `agents.agent_id`, and every `agents` row has a `tenant_id`. Cross-tenant placement rows are structurally impossible because you can only write a placement row during a registration call that is already tenant-authenticated, and reads are joined through `agents` so the tenant filter flows through.

**Cleanup on deregister.** When an agent is deregistered through any path (`DELETE /api/v1/agents/{id}` via `hikyaku deregister` or via `hikyaku member delete`), the registry store also hard-deletes any `agent_placements` row where `agent_id == deregistered_agent_id`. This is a small addition to `RegistryStore.deregister_agent` in `registry_store.py`. Placement rows have no historical value and must not outlive the agent they describe.

**Stale placement on pane death.** If a user kills a pane manually (e.g. `tmux kill-pane %7`) without going through `hikyaku member delete`, the broker retains both the `agents` row (active) and the `agent_placements` row (pointing at a dead pane). This is an accepted staleness window. The next `hikyaku member delete` call handles it gracefully: deregister cascade-deletes the placement row first, then the `send_exit` step is a no-op because `tmux.send_exit(..., ignore_missing=True)` swallows the "can't find pane" error. We do not add a background job to detect dead panes in v1 — the CLI is the canonical cleanup path.

**Pending placement state (`tmux_pane_id IS NULL`).** The two-pass write flow (see Atomicity below) creates a placement row at register time before the tmux pane is actually spawned, so the row briefly has `tmux_pane_id = NULL`. In the happy path, the CLI patches the real pane_id via `PATCH /api/v1/agents/{id}/placement` within a few hundred milliseconds. If the Director crashes or loses the broker connection between the register and the PATCH, the row stays pending indefinitely. Consequences:

- `member list` renders a pending row with `(pending)` in place of the pane_id column (text output) or `"tmux_pane_id": null` in JSON. Users can spot these and clean them up.
- `member delete` on a pending row is safe: it skips the `send_exit` and `select-layout` steps (there is nothing to close or rebalance) and just calls `DELETE /api/v1/agents/{id}`, which cascade-deletes the placement row.
- Because NULL is a distinct value from an empty string, `SELECT ... WHERE tmux_pane_id IS NULL` is the exact query for "find all pending rows", with no risk of confusing the pending state with a freshly-written row.

### Alembic migration

New file: `registry/src/hikyaku_registry/alembic/versions/0002_add_agent_placements.py`

```python
"""add agent_placements table

Revision ID: 0002_add_agent_placements
Revises: 0001_initial_schema
Create Date: 2026-04-12
"""
from alembic import op
import sqlalchemy as sa

revision = "0002_add_agent_placements"
down_revision = "0001_initial_schema"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.create_table(
        "agent_placements",
        sa.Column("agent_id", sa.String(), primary_key=True),
        sa.Column("director_agent_id", sa.String(), nullable=False),
        sa.Column("tmux_session", sa.String(), nullable=False),
        sa.Column("tmux_window_id", sa.String(), nullable=False),
        sa.Column("tmux_pane_id", sa.String(), nullable=True),   # nullable: NULL = pending
        sa.Column("created_at", sa.String(), nullable=False),
        sa.ForeignKeyConstraint(
            ["agent_id"], ["agents.agent_id"], ondelete="CASCADE"
        ),
        sa.ForeignKeyConstraint(
            ["director_agent_id"], ["agents.agent_id"], ondelete="RESTRICT"
        ),
    )
    op.create_index(
        "idx_placements_director",
        "agent_placements",
        ["director_agent_id"],
    )


def downgrade() -> None:
    op.drop_index("idx_placements_director", table_name="agent_placements")
    op.drop_table("agent_placements")
```

`hikyaku-registry db init` runs Alembic upgrade to head, which picks this up automatically. No new CLI command is required.

### HTTP API changes

#### Extended — `POST /api/v1/agents`

Request body gains an optional `placement` object:

```json
{
  "name": "Claude-B",
  "description": "Reviewer bot",
  "skills": [],
  "placement": {
    "director_agent_id": "<director-uuid>",
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": "%7"
  }
}
```

- `placement` is optional. Omitting it preserves existing `hikyaku register` behavior exactly.
- When present, `placement.director_agent_id` MUST be a valid active agent in the caller's tenant. The server rejects cross-tenant placements with 403.
- When present, the server creates the agent row AND the placement row in the same transaction. Either both rows are written or neither is.
- The `created_at` timestamp on the placement row is set server-side to the same value as `agents.registered_at`.

Response (`RegisterAgentResponse`) gains an optional `placement` field:

```json
{
  "agent_id": "<uuid>",
  "api_key": "<tenant-key>",
  "name": "Claude-B",
  "registered_at": "2026-04-12T10:15:00Z",
  "placement": {
    "director_agent_id": "<director-uuid>",
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": "%7",
    "created_at": "2026-04-12T10:15:00Z"
  }
}
```

`placement` is null/absent when the request did not include one.

**Authentication note.** `POST /api/v1/agents` today uses `get_registration_tenant` (auth.py:56), which only verifies the API key and does NOT require `X-Agent-Id`. When `placement` is present, the server additionally requires `X-Agent-Id` to be set and to equal `placement.director_agent_id`. This is enforced in the handler, not the dependency, so the existing bare-registration flow is unaffected.

#### New — `GET /api/v1/agents/{agent_id}` returns `placement` field

Existing handler at `api/registry.py:51-83`. After the tenant check, join `agent_placements` and include the placement object in the response (or `null` when absent).

#### New — `GET /api/v1/agents?director_agent_id=<id>`

New query parameter on the existing list endpoint. When provided, the response is filtered to agents whose `agent_placements.director_agent_id` matches the value. Tenant scoping still applies (the caller's tenant is joined in). This is what `hikyaku member list` calls.

The response shape (`ListAgentsResponse`) is unchanged structurally; `agents[*].placement` is populated for the filtered rows.

#### Extended — `DELETE /api/v1/agents/{agent_id}` — caller check relaxed

Current handler at `api/registry.py:86-118` rejects with 403 if `caller_id != agent_id`. This is too strict for the "Director deletes member" flow. The new rule is:

```
allow DELETE if:
    caller_id == agent_id                                  -- self-deregister (existing)
  OR
    caller_id == placements[agent_id].director_agent_id    -- director-deletes-member (new)
```

The handler fetches the placement row (if any) and compares `director_agent_id` to `caller_id` as an alternative match. All other rules (tenant match, agent exists, not already deregistered) are unchanged.

On successful delete, the placement row is hard-deleted in the same transaction as the agent soft-delete (see Data Model → Cleanup on deregister). This keeps the agent_placements table tight.

#### New Pydantic models (in `registry/src/hikyaku_registry/models.py`)

```python
class PlacementCreate(BaseModel):
    director_agent_id: str
    tmux_session: str
    tmux_window_id: str
    tmux_pane_id: str | None = None   # null = pending; set later via PATCH


class PlacementView(BaseModel):
    director_agent_id: str
    tmux_session: str
    tmux_window_id: str
    tmux_pane_id: str | None          # null when placement row is still pending
    created_at: str


class RegisterAgentRequest(BaseModel):
    name: str
    description: str
    skills: list[dict] | None = None
    placement: PlacementCreate | None = None          # NEW


class RegisterAgentResponse(BaseModel):
    agent_id: str
    api_key: str
    name: str
    registered_at: str
    placement: PlacementView | None = None             # NEW


class AgentSummary(BaseModel):
    agent_id: str
    name: str
    description: str
    status: str                                        # NEW — surfaced so `member list` can render it
    registered_at: str
    placement: PlacementView | None = None             # NEW
```

### RegistryStore changes (`registry/src/hikyaku_registry/registry_store.py`)

New methods:

```python
async def create_agent_with_placement(
    self,
    *,
    name: str,
    description: str,
    skills: list[dict] | None,
    api_key: str,
    placement: PlacementCreate | None,
) -> CreateAgentResult: ...

async def list_placements_for_director(
    self, *, tenant_id: str, director_agent_id: str
) -> list[AgentWithPlacement]: ...

async def get_placement(self, *, agent_id: str) -> AgentPlacement | None: ...
```

`deregister_agent` is extended to also `DELETE FROM agent_placements WHERE agent_id = ?` inside the same session transaction. The existing signature stays unchanged.

`create_agent_with_placement` is a superset of the current `create_agent`. The old `create_agent` method is refactored to call `create_agent_with_placement(..., placement=None)` so there is one code path for both register flows.

### Client-side changes

#### `client/src/hikyaku_client/tmux.py` — NEW module

All tmux subprocess interaction is isolated in this module. The rest of the client never calls `subprocess` directly.

```python
import os
import shutil
import subprocess
from dataclasses import dataclass


class TmuxError(Exception):
    """Raised when a tmux subprocess fails or tmux is not reachable."""


@dataclass(frozen=True)
class DirectorContext:
    session: str      # e.g. 'main'
    window_id: str    # e.g. '@3'
    pane_id: str      # e.g. '%0' — the Director's own pane


def ensure_tmux_available() -> None:
    """Raise TmuxError if the `tmux` binary is not on PATH or TMUX is unset."""
    if shutil.which("tmux") is None:
        raise TmuxError("tmux binary not found on PATH")
    if not os.environ.get("TMUX"):
        raise TmuxError("hikyaku member commands must be run inside a tmux session")


def director_context() -> DirectorContext:
    """Resolve the Director's own tmux session, window_id, and pane_id.

    Uses the TMUX_PANE env var as the anchor and queries tmux for the
    containing window. Works regardless of which window the user is
    currently focused on.
    """
    tmux_pane = os.environ.get("TMUX_PANE")
    if not tmux_pane:
        raise TmuxError("TMUX_PANE is not set; not running inside a tmux pane")
    out = _run(
        ["tmux", "display-message", "-p", "-t", tmux_pane,
         "#{session_name}|#{window_id}|#{pane_id}"]
    )
    try:
        session, window_id, pane_id = out.strip().split("|", 2)
    except ValueError as exc:
        raise TmuxError(f"unexpected tmux display-message output: {out!r}") from exc
    return DirectorContext(session=session, window_id=window_id, pane_id=pane_id)


def split_window(
    *,
    target_window_id: str,
    env: dict[str, str],
    claude_prompt: str,
) -> str:
    """Spawn `claude <prompt>` in a new pane in `target_window_id`.

    Returns the new pane_id (e.g. '%7'). Forwards env as `-e KEY=VAL` flags.
    """
    args = ["tmux", "split-window", "-t", target_window_id, "-P", "-F", "#{pane_id}"]
    for k, v in env.items():
        args += ["-e", f"{k}={v}"]
    args += ["claude", claude_prompt]
    return _run(args).strip()


def select_layout(*, target_window_id: str, layout: str = "main-vertical") -> None:
    _run(["tmux", "select-layout", "-t", target_window_id, layout])


_PANE_GONE_MARKERS = ("can't find pane", "no such pane")


def send_exit(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Send '/exit' + Enter to the given pane.

    If `ignore_missing=True` and tmux reports the pane no longer exists
    (matched against _PANE_GONE_MARKERS), return silently instead of raising.
    Any other tmux failure raises `TmuxError`.
    """
    try:
        _run(["tmux", "send-keys", "-t", target_pane_id, "/exit", "Enter"])
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise


def capture_pane(*, target_pane_id: str, lines: int = 80) -> str:
    """Capture the last `lines` lines of the target pane's terminal buffer.

    Invokes `tmux capture-pane -p -t <pane_id> -S -<lines>`. Returns the
    raw captured string as tmux emitted it (bytes decoded via text=True).
    Raises `TmuxError` on failure — the caller should surface "can't find
    pane" errors to the user rather than swallowing them, since the whole
    point of capture is to inspect a live pane.
    """
    if lines <= 0:
        raise TmuxError(f"capture_pane: lines must be positive, got {lines}")
    return _run(
        ["tmux", "capture-pane", "-p", "-t", target_pane_id, "-S", f"-{lines}"]
    )


def _run(args: list[str]) -> str:
    try:
        result = subprocess.run(args, capture_output=True, text=True, check=True)
    except FileNotFoundError as exc:
        raise TmuxError(f"tmux binary not found: {exc}") from exc
    except subprocess.CalledProcessError as exc:
        raise TmuxError(
            f"tmux command failed: {' '.join(args)}\nstderr: {exc.stderr.strip()}"
        ) from exc
    return result.stdout
```

**No new dependency.** `subprocess` and `shutil` are stdlib. We explicitly rejected `libtmux` — the surface area is four commands, and adding a PyPI dependency for that is overkill.

#### `client/src/hikyaku_client/api.py` — extend `register_agent`

Add an optional `placement` keyword:

```python
async def register_agent(
    broker_url: str,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    *,
    api_key: str,
    placement: dict | None = None,
    director_agent_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"name": name, "description": description}
    if skills is not None:
        body["skills"] = skills
    if placement is not None:
        body["placement"] = placement
    headers = {"Authorization": f"Bearer {api_key}"}
    if director_agent_id is not None:
        headers["X-Agent-Id"] = director_agent_id
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/api/v1/agents", json=body, headers=headers
        )
        resp.raise_for_status()
        return resp.json()
```

New helper for the list flow:

```python
async def list_members(
    broker_url: str,
    api_key: str,
    director_agent_id: str,
) -> list[dict]:
    async with httpx.AsyncClient() as client:
        resp = await client.get(
            f"{broker_url}/api/v1/agents",
            params={"director_agent_id": director_agent_id},
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": director_agent_id,
            },
        )
        resp.raise_for_status()
        return resp.json().get("agents", [])
```

`deregister_agent` does not need to change — the endpoint's relaxed caller check handles Director-deletes-member transparently, and the CLI passes the Director's id in `X-Agent-Id` when it invokes delete.

#### `client/src/hikyaku_client/cli.py` — new `member` subgroup

Add a new click group `member` with three commands. The group mirrors the existing command style (error handling via `_require_api_key`, JSON/text output toggled by `--json`).

```python
@cli.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


@member.command("create")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(ctx, agent_id, name, description, prompt_argv):
    """Register a new member and spawn its claude pane in the Director's window."""
    from hikyaku_client import tmux

    _require_api_key(ctx)
    broker_url = ctx.obj["url"]
    api_key = ctx.obj["api_key"]

    # Pre-flight: must be running inside a tmux session.
    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    prompt = _resolve_prompt(ctx, agent_id, prompt_argv)

    # Step 1 — register member with pending placement (tmux_pane_id=null).
    try:
        result = _run(
            api.register_agent(
                broker_url,
                name,
                description,
                api_key=api_key,
                director_agent_id=agent_id,   # sent as X-Agent-Id
                placement={
                    "director_agent_id": agent_id,
                    "tmux_session": director_ctx.session,
                    "tmux_window_id": director_ctx.window_id,
                    "tmux_pane_id": None,
                },
            )
        )
    except Exception as exc:
        click.echo(f"Error: register failed: {exc}", err=True)
        ctx.exit(1)
        return
    new_agent_id = result["agent_id"]

    # Step 2 — split-window, forwarding env so the spawned claude can reach the broker.
    try:
        pane_id = tmux.split_window(
            target_window_id=director_ctx.window_id,
            env={
                "HIKYAKU_URL": broker_url,
                "HIKYAKU_API_KEY": api_key,
                "HIKYAKU_AGENT_ID": new_agent_id,
            },
            claude_prompt=prompt,
        )
    except tmux.TmuxError as exc:
        _rollback_register(
            broker_url, api_key, agent_id, new_agent_id,
            reason=f"tmux split-window failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 3 — PATCH the pending placement with the real pane_id.
    try:
        placement_view = _run(
            api.patch_placement(
                broker_url, api_key,
                director_agent_id=agent_id,
                member_agent_id=new_agent_id,
                pane_id=pane_id,
            )
        )
    except Exception as exc:
        # Placement patch failed: pane is alive but dangling. /exit it, then roll back.
        try:
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        except tmux.TmuxError:
            pass  # pane already closed — acceptable
        _rollback_register(
            broker_url, api_key, agent_id, new_agent_id,
            reason=f"placement PATCH failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 4 — rebalance layout (best-effort, non-fatal).
    try:
        tmux.select_layout(target_window_id=director_ctx.window_id)
    except tmux.TmuxError as exc:
        click.echo(f"Warning: select-layout failed: {exc}", err=True)

    result["placement"] = placement_view
    if ctx.obj["json_output"]:
        sanitized = {k: v for k, v in result.items() if k != "api_key"}
        click.echo(output.format_json(sanitized))
    else:
        click.echo(output.format_member(result))


def _rollback_register(broker_url, api_key, director_id, new_agent_id, *, reason):
    """Best-effort rollback: deregister the just-created agent as the Director."""
    click.echo(
        f"Error: {reason}. Rolling back registration of {new_agent_id}.",
        err=True,
    )
    try:
        _run(
            api.deregister_agent(
                broker_url, api_key, new_agent_id, caller_id=director_id
            )
        )
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — agent {new_agent_id} is "
            f"orphaned in the registry. Run `hikyaku deregister --agent-id "
            f"{new_agent_id}` manually to clean up. Cause: {drop_exc}",
            err=True,
        )
```

#### Default prompt resolution

Helper function `_resolve_prompt` (private to `cli.py`). No new API helper is added — it reuses the existing `api.list_agents(..., caller_id=..., agent_id=...)` path, which is already how `hikyaku agents --agent-id X --id X` fetches a single agent detail (cli.py:255-275).

```python
def _resolve_prompt(
    ctx, director_agent_id: str, prompt_argv: tuple[str, ...]
) -> str:
    if prompt_argv:
        return " ".join(prompt_argv)
    # Reuse the existing list_agents single-agent path. ctx.obj keys match
    # the current cli group callback: ctx.obj["url"] and ctx.obj["api_key"].
    director = _run(
        api.list_agents(
            ctx.obj["url"],
            ctx.obj["api_key"],
            caller_id=director_agent_id,
            agent_id=director_agent_id,
        )
    )
    return (
        f"Load Skill(hikyaku). Your agent_id is $HIKYAKU_AGENT_ID. "
        f"You are a member of the team led by {director['name']} "
        f"({director_agent_id}). Wait for instructions via "
        f"`hikyaku poll --agent-id $HIKYAKU_AGENT_ID`."
    )
```

#### `hikyaku member capture` implementation sketch

Parallel to `member_create`. All tmux interaction flows through the helper module, so the command itself is thin: fetch placement, guard, capture, print.

```python
@member.command("capture")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.option("--lines", type=int, default=80, show_default=True,
              help="Number of trailing terminal lines to capture")
@click.pass_context
def member_capture(ctx, agent_id, member_id, lines):
    """Capture the last N lines of a member pane's terminal buffer."""
    from hikyaku_client import tmux

    _require_api_key(ctx)
    broker_url = ctx.obj["url"]
    api_key = ctx.obj["api_key"]

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    # Fetch the target's agent + placement. Reuses the single-agent lookup
    # shape (caller_id + agent_id) already used elsewhere in cli.py.
    try:
        target = _run(
            api.list_agents(
                broker_url, api_key,
                caller_id=agent_id, agent_id=member_id,
            )
        )
    except Exception as exc:
        click.echo(f"Error: failed to fetch member: {exc}", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement row; it was not "
            f"spawned via `hikyaku member create`.",
            err=True,
        )
        ctx.exit(1)
        return
    if placement["director_agent_id"] != agent_id:
        click.echo(
            f"Error: agent {member_id} is not a member of your team "
            f"(director_agent_id={placement['director_agent_id']}).",
            err=True,
        )
        ctx.exit(1)
        return
    if placement["tmux_pane_id"] is None:
        click.echo(
            f"Error: member {member_id} has no pane yet (pending placement) "
            f"— nothing to capture.",
            err=True,
        )
        ctx.exit(1)
        return

    try:
        content = tmux.capture_pane(
            target_pane_id=placement["tmux_pane_id"], lines=lines
        )
    except tmux.TmuxError as exc:
        click.echo(f"Error: capture failed: {exc}", err=True)
        ctx.exit(1)
        return

    if ctx.obj["json_output"]:
        click.echo(output.format_json({
            "member_agent_id": member_id,
            "pane_id": placement["tmux_pane_id"],
            "lines": lines,
            "content": content,
        }))
    else:
        click.echo(content, nl=False)
```

Note that `member capture` has no rollback path — it is a pure read. The cross-Director check at the client is defense-in-depth; the server's `GET /api/v1/agents/{id}` already enforces tenant scope and the placement row's own constraints mean a cross-tenant id would simply return 404.

### Atomicity and error handling

`member create` has three server-visible steps:
1. **REGISTER** — `POST /api/v1/agents` with `placement` included. Server writes both `agents` and `agent_placements` rows in one transaction.
2. **SPLIT** — `tmux split-window … claude <prompt>`, capturing the new `pane_id`.
3. **PATCH** — update the placement row with the actual `pane_id` captured from step 2.

The problem: `tmux split-window` needs the new agent_id (it has to be passed to the spawned Claude via `-e HIKYAKU_AGENT_ID=…`), which only exists after step 1. But step 1 writes a placement row that needs the pane_id from step 2. So the two steps cannot both "know everything" without an update in between.

**Resolution.** We adopt a **two-pass placement write** and accept a tiny server-side transient:

1. `member create` calls `POST /api/v1/agents` with `placement.tmux_pane_id = null`. The server stores this as SQL `NULL` in the `agent_placements` row; `NULL` is the canonical "pending" marker (see Data Model → Pending placement state). It is distinct from any legal pane_id value so there is no ambiguity.
2. The CLI runs `tmux split-window -t <window-id> -P -F '#{pane_id}' -e HIKYAKU_URL=… -e HIKYAKU_API_KEY=… -e HIKYAKU_AGENT_ID=<new-id> claude <prompt>` and captures the real pane_id from stdout.
3. The CLI issues `PATCH /api/v1/agents/{new-id}/placement` with the real `tmux_pane_id`. (New endpoint — see below.)
4. The CLI runs `tmux select-layout -t <window-id> main-vertical`.

**New endpoint for step 3 — `PATCH /api/v1/agents/{agent_id}/placement`.**

```
PATCH /api/v1/agents/{agent_id}/placement
Authorization: Bearer <api-key>
X-Agent-Id: <director-id>     # must match the existing placement.director_agent_id

{"tmux_pane_id": "%7"}
```

Returns 200 with the updated `PlacementView`. Rejects with 403 if the caller is not the director of this placement, 404 if no placement exists.

**Rollback on SPLIT failure.**

```
try:
    result = POST /api/v1/agents (with placement, null pane_id)
except:
    # nothing created, nothing to roll back
    raise

try:
    pane_id = tmux.split_window(...)
except TmuxError:
    # Registered agent is orphaned. Roll back.
    DELETE /api/v1/agents/{result.agent_id}    # also deletes the placement row server-side
    raise

try:
    PATCH /api/v1/agents/{result.agent_id}/placement {tmux_pane_id}
except:
    # The pane is alive but has a null pane_id on the placement row.
    # /exit the pane, then delete the agent, to get back to a clean state.
    tmux.send_exit(target_pane_id=pane_id)
    DELETE /api/v1/agents/{result.agent_id}
    raise

tmux.select_layout(target_window_id=director_ctx.window_id)
```

If the rollback `DELETE` itself fails (network blip), the CLI prints a loud warning to stderr with the orphaned `agent_id` so the user can clean up manually. No silent drop.

**`member delete` ordering.** Steps 2 and 3 are deliberately in the order below — deregister **before** `send_exit`. The intuitive ordering (kill the visible pane first, then clean up the registry) is wrong because it violates the spec rule "if deregister fails, preserve the pane for retry": once `/exit` has been sent, the pane is gone and the work it held is unrecoverable.

```
1. GET /api/v1/agents/{member-id}   # fetch placement.tmux_pane_id (may be NULL)
   - 404 or agent deregistered    → exit 1, "no such member"
   - agent has no placement row   → exit 1, "agent has no placement; use `hikyaku deregister` instead"
2. DELETE /api/v1/agents/{member-id}   # X-Agent-Id = director's id
   - failure → exit 1 (agent AND pane both preserved, safe to retry)
3. tmux.send_exit(pane_id, ignore_missing=True)   # skipped entirely if pane_id was NULL (pending row)
   - success or "pane already gone" → continue
   - other TmuxError → warn to stderr with pane_id for manual cleanup, continue
4. tmux.select_layout(target_window_id, layout="main-vertical")   # skipped if pane_id was NULL
   - TmuxError → warn to stderr, exit 0
```

After step 3 the agent is deregistered on the broker AND the pane is closed (or was already gone). Exit 0.

**Tradeoff accepted.** A successful deregister followed by a `send_exit` failure for a non-"pane gone" reason (e.g. tmux server restarted) leaves the pane alive but detached from the registry. We accept this because (a) the pane is still a responsive `claude` session the user can drive manually, (b) the warning prints the pane_id so the user can `tmux kill-pane -t <id>` themselves, and (c) losing in-flight work inside the pane is strictly worse than losing the registry row.

**Error matrix.**

| Condition | Command behavior |
|---|---|
| `tmux` binary not found | `member *` commands exit 1 with "tmux binary not found on PATH" |
| `TMUX` env var unset (not inside a tmux session) | `member *` commands exit 1 with "hikyaku member commands must be run inside a tmux session" |
| `TMUX_PANE` unset (shouldn't happen if `TMUX` is set, but defensive) | exit 1 with "TMUX_PANE is not set; not running inside a tmux pane" |
| `HIKYAKU_API_KEY` unset | exit 1 with the existing "HIKYAKU_API_KEY environment variable is required" message |
| `member create`: `POST /agents` returns non-2xx | exit 1, stderr echoes broker error, no tmux side-effects |
| `member create`: `tmux split-window` fails | rollback via `DELETE /agents/{new-id}`, exit 1 |
| `member create`: `PATCH placement` fails | `send_exit` the pane, `DELETE /agents/{new-id}`, exit 1 |
| `member create`: `select-layout` fails | **ignored** — warning to stderr, exit 0. Layout cosmetics should not block a successful spawn. |
| `member delete`: `GET /agents/{id}` returns 404 | exit 1 with "no such member" |
| `member delete`: target has no placement row (registered via plain `hikyaku register`) | exit 1 with "agent has no placement; use `hikyaku deregister` instead" |
| `member delete`: `DELETE /agents/{id}` fails | exit 1, agent AND pane both preserved, safe to retry |
| `member delete`: target has pending placement (`tmux_pane_id IS NULL`) | skip `send_exit` and `select-layout`, deregister proceeds normally, exit 0 |
| `member delete`: `send_exit` fails because pane is already gone (`ignore_missing=True` handles it) | silently continue to `select-layout` |
| `member delete`: `send_exit` fails for non-"pane gone" reasons | warn to stderr with `pane_id` for manual cleanup, continue to `select-layout`, exit 0 |
| `member delete`: `select-layout` fails | warn to stderr, exit 0 |
| `member list`: no members | print "0 members." (exit 0) |
| `member capture`: `tmux` binary not found or `TMUX` unset | exit 1 via `ensure_tmux_available()` |
| `member capture`: `GET /agents/{id}` returns 404 | exit 1 with "failed to fetch member: ..." |
| `member capture`: target has no placement row | exit 1 with "agent {id} has no placement row; it was not spawned via `hikyaku member create`" |
| `member capture`: `placement.director_agent_id` does not match `--agent-id` (cross-Director) | exit 1 with "agent {id} is not a member of your team (director_agent_id=...)" |
| `member capture`: `placement.tmux_pane_id IS NULL` (pending row) | exit 1 with "member {id} has no pane yet (pending placement) — nothing to capture" |
| `member capture`: `tmux capture-pane` reports "can't find pane" (pane already dead) | exit 1 with "capture failed: ..." — deliberately NOT swallowed (capture is a read; the user needs to know the pane is gone) |
| `member capture`: `--lines <= 0` | exit 1 via `capture_pane`'s own guard, "capture_pane: lines must be positive" |
| `member capture`: happy path | print raw buffer to stdout (text mode) or `{member_agent_id, pane_id, lines, content}` JSON, exit 0 |

### Permissions cleanup

`.claude/settings.json` — before/after of the `permissions.allow` array (hikyaku/tmux-related entries only; unrelated `mise` entries elided).

**Before** — 6 entries related to this feature:

```json
"Bash(hikyaku *)",
"Bash(tmux split-window *)",
"Bash(tmux send-keys -t * '/exit' Enter)",
"Bash(tmux select-layout *)",
"Bash(tmux set-window-option *)",
"Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)"
```

**After** — 1 entry:

```json
"Bash(hikyaku *)"
```

**Removed:**
- `Bash(tmux split-window *)`
- `Bash(tmux send-keys -t * '/exit' Enter)`
- `Bash(tmux select-layout *)`
- `Bash(tmux set-window-option *)`
- `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)`

**Retained:** `Bash(hikyaku *)` — unchanged.

**Why `Bash(hikyaku *)` is sufficient.** The new `hikyaku member` commands call `tmux` via Python's `subprocess` module inside the `hikyaku` process. That is not a new `Bash` tool call at the Claude Code level, so it is not gated by the Bash allow list. The tmux-related allow entries previously existed only because the Director was shelling out to `tmux` directly from the `Bash` tool; once the CLI wrapper lands, no such invocations survive and the entries can be deleted.

**No new allow entries are added.** If any `hikyaku member` invocation requires a new permission, that is a bug in the wrapper — the whole point is to fit under the existing `Bash(hikyaku *)` entry.

### SKILL.md rewrite scope

`.claude/skills/hikyaku/SKILL.md` (and any `plugins/*/skills/hikyaku/SKILL.md` copy) changes:

1. **Command Reference section** — add four subsections: `Member Create`, `Member Delete`, `Member List`, `Member Capture`, following the style of the existing `Register`, `Deregister`, etc. entries. Each includes flag list, example, and the JSON response shape. `Member List` explicitly documents the `status` / `session` / `window_id` / `pane_id` columns and the `(pending)` rendering for `tmux_pane_id IS NULL`. `Member Capture` documents the `--lines` default (80) and the "raw buffer to stdout / structured JSON with `--json`" output shape.

2. **Multi-Session Coordination section** — complete rewrite:
   - Remove the `printenv HIKYAKU_URL HIKYAKU_API_KEY` literal-paste warning block (lines 200–208 of current SKILL.md).
   - Replace "Spawn a member" recipe (lines 198–229) with:
     ```bash
     hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
       --description "Reviewer for PR #42"
     ```
   - Replace "Shut down a member" recipe (lines 232–246) with:
     ```bash
     hikyaku member delete --agent-id $DIRECTOR_ID --member-id <member-id>
     ```
   - Replace any raw `tmux list-panes` / `tmux capture-pane` recipes with `hikyaku member list` and `hikyaku member capture --agent-id $DIRECTOR_ID --member-id <member-id> [--lines N]`. The skill must no longer document raw tmux as the canonical way to inspect a stalled member.
   - Remove the "`.claude/settings.local.json` must already allow `Bash(hikyaku:*)`" sub-rule — no longer needed since tmux access is internal to the `hikyaku` binary.
   - Keep the Monitoring mandate and Layout discipline subsections. Layout discipline is now enforced by `hikyaku member create` internally, so update its prose to reflect that. The Monitoring mandate's prose now points at `hikyaku member capture` as the canonical inspection tool.

3. **No other sections change.** Environment Variables, Agent ID, Global Options, Message Lifecycle, Error Handling stay as-is.

#### External skill — known limitation

The `agent-team-supervision` skill lives at `/home/himkt/.claude/skills/agent-team-supervision/SKILL.md` — a **user-level** skill outside this repo. Its "Stall Response" section currently instructs Directors to shell out to `tmux capture-pane -p -t <pane> -S -80` directly. We cannot edit that file from this design doc, so Directors that load `agent-team-supervision` will still see the old raw-tmux guidance.

Mitigation: the Hikyaku `SKILL.md` (which IS in this repo and IS rewritten here) will explicitly call out `hikyaku member capture` as the project-internal equivalent, with a note that it replaces the raw tmux recipe for any project using Hikyaku. Directors reading both skills should prefer the Hikyaku command because it enforces the cross-Director boundary. Bringing the external skill into alignment is tracked as out-of-scope follow-up work.

### Tests

Test plan lives entirely in `registry/tests/` and `client/tests/`:

#### `registry/tests/`
- `test_registry_store.py::test_create_agent_with_placement` — placement row written atomically
- `test_registry_store.py::test_deregister_cascades_placement` — placement row removed on soft-deregister
- `test_registry_store.py::test_list_placements_for_director_tenant_scoped` — cross-tenant isolation
- `test_registry_api.py::test_register_with_placement_sets_tmux_fields` — end-to-end HTTP round-trip
- `test_registry_api.py::test_register_with_placement_requires_x_agent_id` — 401 if header missing when placement present
- `test_registry_api.py::test_register_with_placement_cross_tenant_director_403` — director outside caller's tenant rejected
- `test_registry_api.py::test_delete_agent_as_director_allowed` — director can delete member
- `test_registry_api.py::test_delete_agent_as_unrelated_agent_403` — unrelated agent still rejected
- `test_registry_api.py::test_delete_agent_removes_placement` — row cleanup verified
- `test_registry_api.py::test_patch_placement_sets_pane_id` — the two-pass flow
- `test_registry_api.py::test_patch_placement_caller_must_be_director` — 403 otherwise
- `test_registry_api.py::test_list_agents_filter_by_director` — new query param
- `test_alembic_smoke.py::test_upgrade_to_0002_head` — Alembic smoke extended to the new revision

#### `client/tests/`
- `test_tmux.py::test_director_context_parses_display_message` — subprocess mocked
- `test_tmux.py::test_ensure_tmux_available_errors_when_tmux_missing`
- `test_tmux.py::test_ensure_tmux_available_errors_when_tmux_env_unset`
- `test_tmux.py::test_split_window_returns_captured_pane_id`
- `test_tmux.py::test_send_exit_raises_tmuxerror_on_failure`
- `test_tmux.py::test_capture_pane_invokes_correct_args` — verifies argv is exactly `["tmux", "capture-pane", "-p", "-t", "%7", "-S", "-80"]`
- `test_tmux.py::test_capture_pane_raises_on_missing_pane` — subprocess returns "can't find pane", helper raises `TmuxError` (no silent swallow — capture is a read)
- `test_tmux.py::test_capture_pane_rejects_non_positive_lines` — guard fires for `lines=0` and `lines=-1`
- `test_cli_member.py::test_member_create_happy_path` — monkeypatches `tmux` module + `httpx`
- `test_cli_member.py::test_member_create_rolls_back_on_split_failure`
- `test_cli_member.py::test_member_create_default_prompt_when_no_trailing_args`
- `test_cli_member.py::test_member_create_trailing_positional_becomes_prompt`
- `test_cli_member.py::test_member_delete_idempotent_on_dead_pane`
- `test_cli_member.py::test_member_delete_fail_fast_on_deregister_error`
- `test_cli_member.py::test_member_list_json_output_shape` — asserts each item has `agent_id`, `name`, `status`, `placement.{director_agent_id, tmux_session, tmux_window_id, tmux_pane_id, created_at}`
- `test_cli_member.py::test_member_list_renders_pending_pane_as_literal` — row with `tmux_pane_id=None` shows `(pending)` in text output and `null` in JSON
- `test_cli_member.py::test_member_capture_happy_path` — monkeypatches `tmux.capture_pane`, asserts raw content on stdout (text mode) and `lines` passed through unchanged
- `test_cli_member.py::test_member_capture_cross_director_rejected` — placement belongs to another Director, CLI exits 1 with "not a member of your team" before any tmux call
- `test_cli_member.py::test_member_capture_pending_pane_rejected` — placement has `tmux_pane_id=None`, CLI exits 1 with "pending placement" message before any tmux call
- `test_cli_member.py::test_member_capture_no_placement_rejected` — agent exists but has no placement row, CLI exits 1 with "no placement row"
- `test_cli_member.py::test_member_capture_json_shape` — `--json` output has exactly `{member_agent_id, pane_id, lines, content}` keys

All tmux interaction is mocked via `monkeypatch.setattr(hikyaku_client.tmux, "_run", ...)` — no test requires a real tmux server.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (MUST be first)

Per `.claude/rules/design-doc-numbering.md`, documentation is updated before any code. Every affected doc file must reflect the new CLI surface at the end of this step.

- [x] Update `ARCHITECTURE.md` — add an `agent_placements` table entry to the schema overview; add a "Member lifecycle" paragraph describing the atomic register + split-window + placement flow <!-- completed: 2026-04-12T12:00 -->
- [x] Update `docs/` — add or extend `docs/cli/hikyaku-member.md` with the full `create`/`delete`/`list`/`capture` reference, including the rollback semantics and the `TMUX_PANE` detection rule <!-- completed: 2026-04-12T12:00 -->
- [x] Update `README.md` via `/update-readme` skill to pick up the new CLI surface and the simplified `.claude/settings.json` footprint <!-- completed: 2026-04-12T12:00 -->
- [x] Rewrite `.claude/skills/hikyaku/SKILL.md` — add Member Create/Delete/List/Capture subsections under Command Reference, replace Multi-Session Coordination spawn/shutdown recipes (and any raw `tmux list-panes`/`tmux capture-pane` invocations) with the new CLI, remove the printenv literal-paste block, and add the external `agent-team-supervision` skill limitation note <!-- completed: 2026-04-12T12:00 -->
- [x] Mirror the SKILL.md changes into every `plugins/*/skills/hikyaku/SKILL.md` copy that exists in this repo <!-- completed: 2026-04-12T12:00 -->
- [x] Update `.claude/rules/commands.md` if any new mise task is introduced (expected: none) <!-- completed: 2026-04-12T12:00 -->

### Step 2: Database schema

- [ ] Add `AgentPlacement` model to `registry/src/hikyaku_registry/db/models.py` as specified in "Data model" above <!-- completed: -->
- [ ] Create Alembic migration `registry/src/hikyaku_registry/alembic/versions/0002_add_agent_placements.py` with the upgrade/downgrade pair from the Alembic migration subsection <!-- completed: -->
- [ ] Verify `hikyaku-registry db init` runs cleanly against a fresh file, an at-0001 file, and a file already at 0002 <!-- completed: -->
- [ ] Add a regression test ensuring `PRAGMA foreign_keys=ON` is active on the new connection path (verify the cascade FK behaves) <!-- completed: -->

### Step 3: RegistryStore

- [ ] Refactor `RegistryStore.create_agent` into `create_agent_with_placement(..., placement=None)` and have the old name delegate with `placement=None` <!-- completed: -->
- [ ] Extend `RegistryStore.deregister_agent` to delete the matching `agent_placements` row in the same session <!-- completed: -->
- [ ] Add `RegistryStore.get_placement(agent_id)` <!-- completed: -->
- [ ] Add `RegistryStore.update_placement_pane_id(agent_id, pane_id)` for the PATCH flow <!-- completed: -->
- [ ] Add `RegistryStore.list_placements_for_director(tenant_id, director_agent_id)` joining through `agents` for tenant scope <!-- completed: -->

### Step 4: HTTP API

- [ ] Update `registry/src/hikyaku_registry/models.py` Pydantic classes: add `PlacementCreate`, `PlacementView`, and the new optional fields on `RegisterAgentRequest`, `RegisterAgentResponse`, `AgentSummary` <!-- completed: -->
- [ ] Update `POST /api/v1/agents` handler in `api/registry.py` to (a) read `X-Agent-Id` when `placement` is present, (b) reject with 401/403 per the new auth rules, (c) call `create_agent_with_placement` <!-- completed: -->
- [ ] Update `GET /api/v1/agents` handler to support `?director_agent_id=<id>` and join `agent_placements` into the response <!-- completed: -->
- [ ] Update `GET /api/v1/agents/{agent_id}` handler to include `placement` (or null) in the response <!-- completed: -->
- [ ] Extend `DELETE /api/v1/agents/{agent_id}` handler to allow the caller whose id matches `placement.director_agent_id` as an alternative to the self-match rule <!-- completed: -->
- [ ] Add `PATCH /api/v1/agents/{agent_id}/placement` handler for the two-pass pane_id write. Caller must equal `placement.director_agent_id`. Body: `{tmux_pane_id: str}`. Returns updated `PlacementView`. <!-- completed: -->

### Step 5: Client — tmux helper

- [ ] Create `client/src/hikyaku_client/tmux.py` as specified in "Client-side changes → tmux.py" above, including `TmuxError`, `DirectorContext`, `ensure_tmux_available`, `director_context`, `split_window`, `select_layout`, `send_exit(target_pane_id, ignore_missing=False)`, `_PANE_GONE_MARKERS`, and `_run`. The `ignore_missing=True` branch matches the captured stderr against `_PANE_GONE_MARKERS` so the CLI can call `send_exit(..., ignore_missing=True)` during `member delete` without caring whether the user already closed the pane manually. <!-- completed: -->
- [ ] Add `capture_pane(target_pane_id, lines=80)` helper to `client/src/hikyaku_client/tmux.py` that invokes `tmux capture-pane -p -t <pane_id> -S -<lines>` and returns the raw stdout. Guard against `lines <= 0` with a `TmuxError`. Unlike `send_exit`, the helper never swallows "can't find pane" — capture is a read and the caller needs that signal. <!-- completed: -->

### Step 6: Client — api.py

- [ ] Extend `register_agent` in `client/src/hikyaku_client/api.py` to accept optional `placement` and `director_agent_id` kwargs as specified. When `director_agent_id` is provided, set the `X-Agent-Id` header. <!-- completed: -->
- [ ] Add `patch_placement(broker_url, api_key, director_agent_id, member_agent_id, pane_id)` helper that issues `PATCH /api/v1/agents/{member_agent_id}/placement` with `X-Agent-Id: <director>` and body `{"tmux_pane_id": pane_id}` <!-- completed: -->
- [ ] Add `list_members(broker_url, api_key, director_agent_id)` helper that issues `GET /api/v1/agents?director_agent_id=<id>` with `X-Agent-Id: <director>` and returns `response["agents"]` <!-- completed: -->
- [ ] Extend `deregister_agent` with an optional `caller_id` kwarg so the Director can deregister a member while sending its own id in `X-Agent-Id`. Default `caller_id=None` preserves the existing self-deregister behavior. Used by both `member delete` and the `member create` rollback path. <!-- completed: -->

### Step 7: Client — CLI member subgroup

- [ ] Add `@cli.group() def member()` in `client/src/hikyaku_client/cli.py` <!-- completed: -->
- [ ] Implement `member create` with the 4-step atomicity flow (register → split → patch → rebalance) and the rollback branches from "Atomicity and error handling" <!-- completed: -->
- [ ] Implement `_resolve_prompt` helper that either joins `prompt_argv` or synthesizes the default prompt via `api.list_agents(broker_url, api_key, caller_id=director_id, agent_id=director_id)` (routes to `GET /api/v1/agents/{director_id}` — no new helper required) <!-- completed: -->
- [ ] Implement `member delete` with the 4-step ordering (get → delete → /exit → rebalance) and the error matrix rules <!-- completed: -->
- [ ] Implement `member list` that calls `list_members` and formats output via a new `format_member_list` in `output.py`, rendering `tmux_pane_id IS NULL` as `(pending)` in text mode and `null` in JSON <!-- completed: -->
- [ ] Implement `member capture` that fetches the placement via `api.list_agents(..., caller_id, agent_id=member_id)`, verifies `placement.director_agent_id == --agent-id` (cross-Director guard), rejects pending placements, then calls `tmux.capture_pane(target_pane_id, lines)`. Text mode prints the raw buffer; `--json` emits `{member_agent_id, pane_id, lines, content}`. <!-- completed: -->
- [ ] Add `format_member` / `format_member_list` to `client/src/hikyaku_client/output.py` <!-- completed: -->

### Step 8: Tests

- [ ] Add all `registry/tests/` cases listed under "Tests → registry/tests/" <!-- completed: -->
- [ ] Add all `client/tests/` cases listed under "Tests → client/tests/" <!-- completed: -->
- [ ] Ensure `mise //registry:test` and `mise //client:test` pass <!-- completed: -->
- [ ] Ensure `mise //:lint`, `mise //:format`, `mise //:typecheck` pass <!-- completed: -->

### Step 9: Permissions cleanup

Do this only after Steps 1–8 are merged-complete locally, because removing the allow entries will cause earlier test commits to require user approval on rerun.

- [ ] Apply the `.claude/settings.json` diff from "Permissions cleanup" — remove all five tmux/printenv allow entries, keep `Bash(hikyaku *)` <!-- completed: -->
- [ ] Manual smoke test: run `hikyaku member create` from a fresh Director session, verify pane spawns in the correct window, verify `hikyaku member list` returns the new member, verify `hikyaku member delete` cleans up <!-- completed: -->
- [ ] Manual smoke test: user focused on a DIFFERENT window when `member create` runs — verify the pane lands in the Director's window, not the focused one <!-- completed: -->
- [ ] Manual smoke test: simulate a split failure by temporarily renaming `tmux` on PATH — verify the registered agent is rolled back (agent not listed in `hikyaku agents`) <!-- completed: -->

### Step 10: Wrap up

- [ ] Update the Status header of this design doc to `Complete` and set `Progress` to `44/44 tasks complete` <!-- completed: -->
- [ ] Commit the design doc on the implementation branch per `.claude/rules/git-workflow.md` project override (design docs are committed alongside code in this project) <!-- completed: -->
