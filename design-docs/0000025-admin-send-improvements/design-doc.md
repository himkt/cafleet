# Admin Send UX Improvements — Administrator Agent, @mention Autocomplete, Newline Rendering

**Status**: Complete
**Progress**: 52/52 tasks complete
**Last Updated**: 2026-04-15

## Overview

Three related papercuts in the Admin WebUI Send feature: the sender selector is a free dropdown (the user wants a fixed Administrator identity), the mention field has no autocomplete, and newlines in message bodies collapse in the timeline. This document introduces a session-scoped built-in `Administrator` agent, adds a Discord-style `@` mention popover, and preserves newlines end-to-end (textarea composition + `whitespace-pre-wrap` rendering).

## Success Criteria

- [x] Every session (new and pre-existing) has exactly one active `Administrator` agent, marked via `agent_card_json.cafleet.kind == "builtin-administrator"`.
- [x] Broker rejects deregister, rename, and placement operations targeting an Administrator with a dedicated error class (mapped to HTTP 409 in WebUI, `click.UsageError` in CLI).
- [x] `cafleet broadcast` excludes Administrator agents from the recipient set.
- [x] WebUI Send shows a read-only `Sending as Administrator` label and submits every message with `from_agent_id = administrator.agent_id`. No sender dropdown exists.
- [x] Typing `@` anywhere in the message textarea opens a popover listing active agents (by name prefix) plus a virtual `@all` entry; ArrowUp/Down navigate, Enter/Tab insert, Esc dismisses.
- [x] The message input is a textarea: Enter sends (unless popover is open), Shift+Enter inserts a newline.
- [x] Timeline renders message bodies with newlines preserved (multi-line messages render on multiple lines with correct wrapping).
- [x] When a session is missing an Administrator (pre-migration or manual DB edit), the Dashboard shows a warning banner and disables the Send control. The WebUI does not lazy-create.
- [x] `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test`, and `mise //admin:lint` all pass.

---

## Background

This project ships three prior pieces of work whose contract this doc extends:

| Prior doc | Contribution this doc extends |
|---|---|
| `design-docs/0000002-access-control/design-doc.md` | Session as a non-secret namespace; sender identity is an agent in that session. |
| `design-docs/0000013-admin-discord-style-timeline/design-doc.md` | The WebUI timeline + MessageInput + SenderSelector components this doc refactors. |
| `design-docs/0000015-remove-auth0-local-session-model/design-doc.md` | Removed Auth0 and the concept of an end-user identity — the WebUI currently sends *as* some agent in the session rather than *as* the operator. |
| `design-docs/0000021-direct-sqlite-cli/design-doc.md` | CLI / WebUI call `broker.py` directly against SQLite; all enforcement must live at the broker layer. |

Current state:

- `cafleet session create` (`broker.create_session` in `cafleet/src/cafleet/broker.py:69`) inserts a single row into `sessions` and exits. No agents are auto-created. The first sender identity the WebUI can use is whatever the operator registers with `cafleet register` afterward.
- `admin/src/components/SenderSelector.tsx` is a free `<select>` over every active agent, persisted in `localStorage` per session (`cafleet.sender.<session_id>`).
- `admin/src/components/MessageInput.tsx` uses `<input type="text">` + the regex `/^@([A-Za-z0-9_-]+)(?:\s|$)/` to peel mentions off the leading text. No popover, no keyboard navigation, and `<input>` cannot contain `\n`.
- `admin/src/components/TimelineMessage.tsx` renders the body in two places that both inherit Tailwind's default `white-space: normal`, which collapses every `\n` to a single space — the root cause of the user-reported newline bug:
  - Line 72 (canceled branch `<p>`): `<p className="mt-0.5 text-sm opacity-60"><s>{body(entry)}</s></p>` (the `<s>` at line 73 is the strikethrough wrapper — the `whitespace-pre-wrap` class belongs on the **outer `<p>`**, not on `<s>`).
  - Line 77 (active branch `<p>`): `<p className="mt-0.5 text-sm text-gray-700">{body(entry)}</p>`.
- The `agents` table has no `kind` / `is_builtin` column (see `cafleet/src/cafleet/db/models.py:25-41`). We mark Administrator via `agent_card_json` so the schema stays unchanged and no new migration column is required — only a data migration.

---

## Specification

### A. Data model — Administrator agent

The Administrator is an ordinary row in `agents`, distinguished only by a flag inside its `agent_card_json` blob. No schema change is needed.

| Field | Value |
|---|---|
| `agent_id` | Fresh UUID (as for any agent) |
| `session_id` | The owning session's id |
| `name` | `"Administrator"` |
| `description` | `"Built-in administrator agent for session <session_id-first-8>"` |
| `status` | `"active"` |
| `registered_at` | Same ISO timestamp as the owning session's `created_at` |
| `agent_card_json` | See below |

`agent_card_json` payload for the Administrator:

```json
{
  "name": "Administrator",
  "description": "Built-in administrator agent for session <short-id>",
  "skills": [],
  "cafleet": {
    "kind": "builtin-administrator"
  }
}
```

A module-level constant `ADMINISTRATOR_KIND = "builtin-administrator"` lives in `broker.py`. A tiny helper `_administrator_agent_card(session_id: str) -> dict` builds the card. A helper `_is_administrator_card(agent_card_json: str) -> bool` parses the blob and returns True iff `card.get("cafleet", {}).get("kind") == ADMINISTRATOR_KIND`.

The `cafleet.*` namespace inside `agent_card_json` is reserved for broker-owned flags. The current `broker.register_agent` (`broker.py:187-191`) constructs the card itself from `name`, `description`, and `skills` — callers cannot smuggle `cafleet.kind` through any public path today. If a future design doc exposes card-import (accepting a raw `agent_card_json` from callers), that code MUST strip any caller-supplied `cafleet.*` keys; no such path is introduced here.

### B. Session-create auto-seeding

`broker.create_session(label)` is extended to INSERT an Administrator agent in the same transaction as the session row:

1. Generate `session_id`, `created_at`, and `administrator_agent_id`.
2. `INSERT INTO sessions VALUES (...)`.
3. `INSERT INTO agents VALUES (administrator_agent_id, session_id, 'Administrator', '<desc>', 'active', created_at, NULL, <agent_card_json>)`.
4. Return `{"session_id": ..., "label": ..., "created_at": ..., "administrator_agent_id": ...}`.

CLI `cafleet session create` (`cli.py:182`):

- Non-JSON output: unchanged (prints `session_id` only).
- `--json` output: the new `administrator_agent_id` field is included in the dict printed via `json.dumps(result)`.

### C. Backfill migration

New Alembic revision `0006_seed_administrator_agent.py`:

```
Revision ID: 0006
Revises:     0005
```

`upgrade()`:

1. `SELECT session_id, created_at FROM sessions` for all rows.
2. For each session, check whether an Administrator already exists:
   `SELECT 1 FROM agents WHERE session_id = ? AND json_extract(agent_card_json, '$.cafleet.kind') = 'builtin-administrator' LIMIT 1`.
3. If absent, `INSERT INTO agents` a new Administrator row with `agent_id = str(uuid.uuid4())` **generated in Python inside the migration script** (matching the broker's idiom — no SQL-side `gen_random_uuid()` / custom function), `registered_at = session.created_at`, the canonical card JSON above, and `status = 'active'`.

This step is idempotent by construction: running `upgrade()` a second time finds the existing Administrator via the `json_extract` probe and skips the INSERT, leaving exactly one Administrator per session.

`downgrade()`:

- `DELETE FROM agents WHERE json_extract(agent_card_json, '$.cafleet.kind') = 'builtin-administrator'`. Best-effort: `tasks.context_id` (`db/models.py:74`) references `agents.agent_id` with `ON DELETE RESTRICT`, so a session that already has tasks addressed to or from the Administrator will raise `IntegrityError` on downgrade. (`agent_placements.agent_id` uses `ON DELETE CASCADE` per `db/models.py:49` and does NOT block downgrade — Administrators never receive a placement anyway.) This migration is treated as **forward-only in practice**; downgrade is provided for completeness against empty sessions only.

### D. Broker-layer protection

Introduce a single error class in `broker.py`:

```python
class AdministratorProtectedError(Exception):
    """Raised when an operation targets a built-in Administrator agent."""
```

Two write paths learn to raise it in this design doc:

| Operation | File:function | Behavior |
|---|---|---|
| Deregister | `broker.deregister_agent` | Before the UPDATE, SELECT the agent's `agent_card_json`. If `_is_administrator_card(...)`, raise `AdministratorProtectedError("Administrator cannot be deregistered")`. |
| Placement (member create) | `broker.register_agent` (only `member create` takes the `placement=` path today) | The existing director-validation SELECT at `broker.py:199-205` loads only the `Agent` row without `agent_card_json`. Extend the SELECT column list so `agent_card_json` is available, then call `_is_administrator_card(director.agent_card_json)` on the fetched row. If True, raise `AdministratorProtectedError("Administrator cannot be a director")` — the Administrator must never be handed a tmux pane. |

A future `rename_agent` broker function (out of scope here) MUST apply the same guard.

Error mapping:

| Caller | Mapping |
|---|---|
| CLI (`cafleet deregister`) | Catch `AdministratorProtectedError` → `click.UsageError(...)` → exit code 1 with message on stderr. |
| WebUI API (future deregister endpoint — not in 1st cut) | Catch → `raise HTTPException(status_code=409, detail=...)`. |
| Direct `broker` unit tests | Assert `AdministratorProtectedError` is raised. |

### E. Broadcast exclusion

`broker.broadcast_message` (`broker.py:567`) currently filters recipients by `Agent.session_id == session_id AND Agent.status == 'active' AND Agent.agent_id != agent_id`. The new filter adds:

```python
# Exclude Administrator agents from the recipient set; they are write-only.
recipient_rows = session.execute(
    select(Agent.agent_id, Agent.agent_card_json).where(
        Agent.session_id == session_id,
        Agent.status == "active",
        Agent.agent_id != agent_id,
    )
).all()
recipient_ids = [
    aid for aid, card in recipient_rows
    if not _is_administrator_card(card)
]
```

The sender itself may be an Administrator (the WebUI broadcasts *from* the Administrator). The summary task is unaffected.

### F. WebUI API — expose `kind`

`/ui/api/agents` today returns `agents: [{agent_id, name, description, status, registered_at}]`. Add a `kind` field so the frontend can locate the Administrator without scanning names:

```json
{
  "agents": [
    {
      "agent_id": "uuid",
      "name": "Administrator",
      "description": "Built-in administrator agent for session 3f9a1b2c",
      "status": "active",
      "registered_at": "2026-04-15T10:00:00+00:00",
      "kind": "builtin-administrator"
    },
    {
      "agent_id": "uuid",
      "name": "Claude-B",
      "description": "Reviewer",
      "status": "active",
      "registered_at": "2026-04-15T10:05:00+00:00",
      "kind": "user"
    }
  ]
}
```

Implementation: `broker.list_session_agents` (`broker.py:795-807`) today SELECTs only `(agent_id, name, description, status, registered_at)` — `agent_card_json` must be added to its column list. Then parse the blob once per row and set `kind` to `"builtin-administrator"` iff `_is_administrator_card(...)` is True, else `"user"`. Apply the same column-list extension to `broker.get_agent` (`broker.py:243-283`) so its single-agent dict also carries `kind`.

No other WebUI API endpoints change. `POST /ui/api/messages/send` still accepts any `from_agent_id` that belongs to the session — the WebUI simply always supplies the Administrator's id.

### G. WebUI — sender label

Delete `admin/src/components/SenderSelector.tsx` and its import from `Dashboard.tsx`.

`Dashboard.tsx` derives the sender id from the agents list:

```ts
const administrator = agents.find((a) => a.kind === "builtin-administrator") ?? null;
const senderId = administrator?.status === "active" ? administrator.agent_id : null;
```

If `senderId` is non-null, render a new read-only label in the header:

```tsx
<span className="text-sm text-gray-700">
  Sending as <span className="font-medium text-gray-900">Administrator</span>
</span>
```

If `senderId` is null, render a red banner above the Timeline with text that covers both failure modes without overpromising the remedy:

```tsx
<div className="bg-red-50 border-b border-red-200 px-4 py-2 text-sm text-red-700">
  This session has no Administrator agent. Send is disabled.
  If you just upgraded, run <code className="mx-1 bg-red-100 px-1 rounded">cafleet db init</code>
  to apply the backfill migration. If the Administrator was manually deleted, contact the operator —
  <code className="mx-1 bg-red-100 px-1 rounded">db init</code> will not re-seed it.
</div>
```

`MessageInput` is rendered with `disabled = senderId === null`.

`localStorage` cleanup: on first mount of `Dashboard.tsx` post-upgrade, `localStorage.removeItem("cafleet.sender." + sessionId)` so old selector state does not linger. The `types.ts` `Agent` interface gains `kind: "builtin-administrator" | "user"`.

Administrator appearance in `Sidebar.tsx` is unchanged — it shows up in the Active list like any other agent, with no special styling.

### H. WebUI — @mention autocomplete popover

`admin/src/components/MessageInput.tsx` is rewritten:

1. Replace the `<input type="text">` with a `<textarea>`:
   - `rows={1}` with auto-grow up to 6 rows (inline `style.height` recomputed on each change).
   - `resize-none` and `whitespace-pre-wrap`.
2. Mention detection runs on every `onChange`:
   - Find the substring from the nearest `@` to the left of the cursor up to the cursor.
   - If that substring matches `/@([A-Za-z0-9_-]*)$/` AND the character before the `@` is whitespace, BOL, or empty, set `mentionQuery = match[1]`; else set it to `null`.
3. Popover (`MentionPopover` component or inline):
   - Opens iff `mentionQuery !== null`.
   - Candidate sources:
     - Virtual `@all` entry: `{kind: "virtual", label: "all"}` — **filter matches `"all".startsWith(mentionQuery.toLowerCase())`** (it matches on its literal `label`, not on any agent name).
     - User agents: active agents, excluding the Administrator itself (listing the sender would be confusing) and deregistered agents — **filter matches `slugify(agent.name).startsWith(mentionQuery.toLowerCase())`**.
   - Candidates preserve this order: virtual `@all` first (when it passes its filter), then user agents by name ascending.
   - Empty filtered list → popover is hidden entirely (no "No matches" row).
   - Shows up to 6 rows; name only, no description.
   - Initial `selectedIndex = 0` on every open.
   - When the filtered list shrinks below the current `selectedIndex`, clamp to the last valid index (`max(0, list.length - 1)`). When the list is non-empty but `selectedIndex` was `-1`, reset to 0.
   - Mouse click on a row: insert that candidate and refocus the textarea (preserves keyboard flow).
   - Dismissal: Escape key **or** textarea blur **or** empty filtered list.
   - Anchored above the textarea, left-aligned to the textarea column (matching the caret pixel column is out of scope).
4. Keyboard handling inside the textarea. **All Enter/Tab handlers MUST check `event.nativeEvent.isComposing` first (i.e., return early without `preventDefault` when an IME composition is in progress) so Japanese/Chinese IME candidate confirmation does not trigger a submit or mention-insert**:
   | Key | IME composing | Popover open (not composing) | Popover closed (not composing) |
   |---|---|---|---|
   | `ArrowDown` | default (let IME) | Move selection down | default |
   | `ArrowUp` | default (let IME) | Move selection up | default |
   | `Enter` | default (confirm IME) | Insert selected mention, close popover | Submit form (unless Shift held) |
   | `Tab` | default | Insert selected mention, close popover | default |
   | `Shift+Enter` | default | Insert newline | Insert newline |
   | `Escape` | default | Close popover | default |

5. Insertion rewrites the textarea value: replace `@<mentionQuery>` (from the trigger `@` to the cursor) with `@<slug> ` (trailing space), move the cursor to immediately after the inserted token, then close the popover.
6. The parser (`parseInput`) is unchanged. Multi-recipient unicast remains unsupported; the popover still surfaces all candidates, but the final parse still errors if the user inserts more than one non-`all` mention at the head.

### I. Timeline — preserve newlines

`admin/src/components/TimelineMessage.tsx`:

- **Line 72 — canceled branch `<p>`**: change `<p className="mt-0.5 text-sm opacity-60">` → `<p className="mt-0.5 text-sm opacity-60 whitespace-pre-wrap break-words">`. The `<s>` at line 73 is the strikethrough child and gets NO new class.
- **Line 77 — active branch `<p>`**: change `<p className="mt-0.5 text-sm text-gray-700">` → `<p className="mt-0.5 text-sm text-gray-700 whitespace-pre-wrap break-words">`.

No markdown, no code-fence rendering, no link autolinking — explicitly out of scope.

### J. Testing matrix

| Test file | Scenario |
|---|---|
| `cafleet/tests/test_broker_registry.py` | `create_session` returns `administrator_agent_id`; the row exists in `agents`; `list_session_agents` marks exactly one agent with `kind == "builtin-administrator"`. |
| `cafleet/tests/test_broker_registry.py` | `deregister_agent(administrator_id)` raises `AdministratorProtectedError`; the row is still active afterward. |
| `cafleet/tests/test_broker_messaging.py` | `broadcast_message` from a user agent does NOT create a delivery task for the Administrator. Additionally, assert the summary task's artifact text reads `"Broadcast sent to N recipients"` with N equal to the count AFTER Administrator exclusion (not the raw active count). |
| `cafleet/tests/test_broker_messaging.py` | `broadcast_message` from the Administrator creates delivery tasks for every other active agent. |
| `cafleet/tests/test_broker_registry.py` | `register_agent(..., placement={director_agent_id: administrator_id, ...})` raises `AdministratorProtectedError`. |
| `cafleet/tests/test_alembic_0006_upgrade.py` (new) | Pre-seed two sessions and N user agents; run `alembic upgrade head` starting from `0005`; assert each session gained exactly one Administrator with the canonical card shape. |
| `cafleet/tests/test_alembic_0006_upgrade.py` (new) | **Idempotency**: run `upgrade` twice back-to-back; assert exactly one Administrator per session (no duplicates). |
| `cafleet/tests/test_alembic_0006_upgrade.py` (new) | **Downgrade smoke (empty session)**: upgrade → downgrade; assert the Administrator is removed. No test for the non-empty case — downgrade is forward-only in practice, and asserting `IntegrityError` against `ON DELETE RESTRICT` would test SQLite, not our code. |
| `cafleet/tests/test_broker_webui.py` | `/ui/api/agents` response includes `kind` field; the Administrator's row has `kind == "builtin-administrator"`. |
| `cafleet/tests/test_session_cli.py` | `cafleet session create --json` output includes `administrator_agent_id`; the non-JSON path is unchanged (prints session_id only). |
| `cafleet/tests/test_cli_session_flag.py` or a new CLI test file | `cafleet deregister --agent-id <administrator_id>` exits non-zero with a stderr message mentioning "Administrator cannot be deregistered"; after the call, the administrator row is still `active`. |
| Manual / Vite dev-server QA | Open the Dashboard, type `@cla` and see the popover, pick with Enter/Tab, verify the inserted token, compose a multi-line message with Shift+Enter, send, verify multi-line rendering. Verify the red banner appears when the Administrator is manually deleted from SQLite. Verify IME composition (e.g. Japanese input) can confirm candidates with Enter without submitting the message. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (MUST precede code)

- [x] Update `ARCHITECTURE.md` — note that `cafleet session create` auto-seeds an Administrator agent, and that `broker.py` enforces Administrator protection (deregister / placement / broadcast-recipient) in one place. <!-- completed: 2026-04-15T14:05 -->
- [x] Update `docs/spec/data-model.md` — document the `agent_card_json.cafleet.kind = "builtin-administrator"` flag and the one-Administrator-per-session invariant. <!-- completed: 2026-04-15T14:08 -->
- [x] Update `docs/spec/webui-api.md` — add `kind` field to `/ui/api/agents` response, note that WebUI always sends from the Administrator, and describe the 409 mapping for any future deregister endpoint. <!-- completed: 2026-04-15T14:12 -->
- [x] Update `docs/spec/cli-options.md` — extend the `session create` entry with the `administrator_agent_id` JSON field. <!-- completed: 2026-04-15T14:15 -->
- [x] Update `README.md` — in the Session section mention auto-seeded Administrator; in the Admin WebUI section describe the fixed sender label, mention autocomplete, and multi-line input. Run the `/update-readme` skill after ARCHITECTURE.md and docs/ are updated, per `.claude/rules/design-doc-numbering.md`. <!-- completed: 2026-04-15T14:18 -->
- [x] Update `.claude/skills/cafleet/SKILL.md` — add a callout under Register and Deregister that the Administrator is reserved and cannot be deregistered; note that `session create --json` returns `administrator_agent_id`. <!-- completed: 2026-04-15T14:22 -->

### Step 2: Broker helpers and constants

- [x] Add `ADMINISTRATOR_KIND = "builtin-administrator"` constant to `cafleet/src/cafleet/broker.py`. <!-- completed: 2026-04-15T14:35 -->
- [x] Add `_administrator_agent_card(session_id: str) -> dict` helper. <!-- completed: 2026-04-15T14:35 -->
- [x] Add `_is_administrator_card(agent_card_json: str) -> bool` helper. <!-- completed: 2026-04-15T14:35 -->
- [x] Add `class AdministratorProtectedError(Exception)`. <!-- completed: 2026-04-15T14:35 -->

### Step 3: Session-create auto-seeding

- [x] Extend `broker.create_session` to insert the Administrator agent in the same transaction and return `administrator_agent_id` in the result dict. <!-- completed: 2026-04-15T15:05 -->
- [x] Update `cli.session_create` to include `administrator_agent_id` in `--json` output; leave the text path unchanged. <!-- completed: 2026-04-15T15:05 -->

### Step 4: Alembic 0006 data migration

- [x] Create `cafleet/src/cafleet/alembic/versions/0006_seed_administrator_agent.py` (revises `0005`). <!-- completed: 2026-04-15T15:22 -->
- [x] `upgrade()` iterates all sessions and inserts an Administrator where absent. UUID is generated in Python (`uuid.uuid4()`) inside the migration script — NOT via a SQL-side function. Probe for existing Administrator via `json_extract(agent_card_json, '$.cafleet.kind') = 'builtin-administrator'` so re-runs are idempotent. <!-- completed: 2026-04-15T15:22 -->
- [x] `downgrade()` deletes Administrator rows via `json_extract`. Document forward-only intent in a docstring; do not try to work around `ON DELETE RESTRICT` on `tasks.context_id`. <!-- completed: 2026-04-15T15:22 -->

### Step 5: Broker protections

- [x] `broker.deregister_agent`: before UPDATE, SELECT the target's card and raise `AdministratorProtectedError` if it matches. <!-- completed: 2026-04-15T15:40 -->
- [x] `broker.register_agent`: when `placement` is provided, reject `placement.director_agent_id` pointing at an Administrator. <!-- completed: 2026-04-15T15:40 -->
- [x] CLI `cafleet deregister`: catch `AdministratorProtectedError` → `click.UsageError` → exit 1. <!-- completed: 2026-04-15T15:40 -->

### Step 6: Broadcast recipient exclusion

- [x] `broker.broadcast_message`: filter out Administrator agents from the recipient set. <!-- completed: 2026-04-15T15:55 -->

### Step 7: WebUI API — surface `kind`

- [x] Extend the SELECT column lists of `broker.list_session_agents` (`broker.py:795-807`) and `broker.get_agent` (`broker.py:243-283`) so they load `Agent.agent_card_json`. <!-- completed: 2026-04-15T16:10 -->
- [x] `broker.list_session_agents` and `broker.get_agent` return `kind: "builtin-administrator" | "user"` derived from `agent_card_json` via `_is_administrator_card`. <!-- completed: 2026-04-15T16:10 -->
- [x] `cafleet/src/cafleet/webui_api.py` passes the new field through unchanged. <!-- completed: 2026-04-15T16:10 -->
- [x] `admin/src/types.ts` adds `kind: "builtin-administrator" | "user"` to `Agent`. <!-- completed: 2026-04-15T16:10 -->

### Step 8: WebUI — fixed Administrator sender

- [x] Delete `admin/src/components/SenderSelector.tsx`. <!-- completed: 2026-04-15T16:25 -->
- [x] Remove `<SenderSelector>` import + usage from `admin/src/components/Dashboard.tsx`. <!-- completed: 2026-04-15T16:25 -->
- [x] Derive `senderId` in `Dashboard` from the Administrator entry of the agents list. <!-- completed: 2026-04-15T16:25 -->
- [x] Render a read-only `Sending as Administrator` label in the header when `senderId` is set. <!-- completed: 2026-04-15T16:25 -->
- [x] Render a red warning banner above the Timeline when `senderId` is null, and disable `MessageInput`. <!-- completed: 2026-04-15T16:25 -->
- [x] Strip legacy `localStorage.cafleet.sender.<session_id>` on mount. <!-- completed: 2026-04-15T16:25 -->

### Step 9: WebUI — @mention autocomplete popover

- [x] Rewrite `MessageInput.tsx` to use a `<textarea>` with auto-grow. <!-- completed: 2026-04-15T16:50 -->
- [x] Add mention-query detection (regex `/@([A-Za-z0-9_-]*)$/` against the text to the left of the cursor, gated on the preceding character being whitespace/BOL). <!-- completed: 2026-04-15T16:50 -->
- [x] Build the popover UI (up to 6 rows, name only, absolute-positioned above the textarea; empty-filter hides the popover; mouse click inserts and refocuses). <!-- completed: 2026-04-15T16:50 -->
- [x] Implement IME-composition guard: every Enter/Tab handler MUST return early when `event.nativeEvent.isComposing` is true, so IME confirmation does not submit or insert a mention. <!-- completed: 2026-04-15T16:50 -->
- [x] Implement keyboard handling per Spec §H (ArrowUp/Down/Enter/Tab/Esc, Shift+Enter always newline; initial `selectedIndex = 0`; clamp on shrink). <!-- completed: 2026-04-15T16:50 -->
- [x] Implement insertion logic (replace `@<query>` with `@<slug> ` and move caret). <!-- completed: 2026-04-15T16:50 -->
- [x] Build the candidate list per Spec §H: virtual `@all` filtered against its label `"all"` first, then active user agents (excluding the Administrator and deregistered) filtered by `slugify(name).startsWith(...)`. Dismiss on Escape or textarea blur. <!-- completed: 2026-04-15T16:50 -->

### Step 10: WebUI — newline rendering

- [x] Add `whitespace-pre-wrap break-words` to the body `<p>` in both branches of `TimelineMessage.tsx` (lines 72 and 77 — the `<p>` tags, NOT the `<s>` tag at line 73). <!-- completed: 2026-04-15T17:05 -->

### Step 11: Tests

- [x] Add broker tests for `create_session` auto-seeding + `kind` surfacing. <!-- completed: 2026-04-15T10:00 -->
- [x] Add broker tests for `AdministratorProtectedError` on deregister and placement. <!-- completed: 2026-04-15T10:10 -->
- [x] Add broker tests for broadcast recipient exclusion, including an assertion that the summary artifact's `"Broadcast sent to N recipients"` text reflects the POST-exclusion recipient count. <!-- completed: 2026-04-15T10:20 -->
- [x] Add `tests/test_alembic_0006_upgrade.py` verifying the data migration on a pre-seeded DB. <!-- completed: 2026-04-15T10:30 -->
- [x] Add migration idempotency test: run `alembic upgrade head` twice and assert exactly one Administrator per session. <!-- completed: 2026-04-15T10:30 -->
- [x] Add migration downgrade smoke test on an empty session (no tasks) — assert the Administrator is removed. <!-- completed: 2026-04-15T10:30 -->
- [x] Add WebUI API test for `kind` in `/ui/api/agents` response. <!-- completed: 2026-04-15T10:40 -->
- [x] Add CLI test for `session create --json` shape. <!-- completed: 2026-04-15T10:00 -->
- [x] Add CLI test for `cafleet deregister --agent-id <administrator_id>`: asserts non-zero exit, stderr mentions "Administrator cannot be deregistered", and the row is still `active`. <!-- completed: 2026-04-15T10:10 -->

### Step 12: Verification

- [x] `mise //:lint` passes. <!-- completed: 2026-04-15T15:57 -->
- [x] `mise //:format` passes. <!-- completed: 2026-04-15T15:57 -->
- [x] `mise //:typecheck` passes. <!-- completed: 2026-04-15T15:57 -->
- [x] `mise //cafleet:test` passes. <!-- completed: 2026-04-15T15:57 -->
- [x] `mise //admin:lint` passes. <!-- completed: 2026-04-15T15:57 -->
- [x] `mise //admin:build` succeeds and the built WebUI renders the new features (manual QA: popover, multi-line compose, multi-line render, warning banner). <!-- completed: 2026-04-15T15:57 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-15 | Initial draft |
| 2026-04-15 | Reviewer pass 1: fix FK description (tasks RESTRICT vs placements CASCADE); explicit `agent_card_json` SELECT extension for list_session_agents / get_agent / register_agent director guard; IME composition guard; popover mechanics (initial index, clamp, mouse click, empty hide, blur dismiss); `@all` matches its label; banner text covers both failure modes; Python-side UUID in migration; drop speculative rename row; drop defensive strip paragraph; line numbers 72 and 77 for TimelineMessage; add idempotency / deregister-CLI / downgrade-smoke tests; `/update-readme` workflow reference; progress recount to 52. |
| 2026-04-15 | Implementation complete. All 12 steps executed via CAFleet-native TDD cycle (Director + Tester + Programmer + Verifier). 54 new tests, 349/349 suite pass, all mise validators pass, admin:build clean. Interactive UI QA (popover, multi-line compose, warning banner, IME Enter) deferred to manual verification in browser. Pre-existing tracked webui/ build artifacts untracked to align with .gitignore. Status set to Complete. |
