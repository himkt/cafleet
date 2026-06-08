# Documentation Simplification (docs/ + README.md)

**Status**: Approved
**Progress**: 0/24 tasks complete
**Last Updated**: 2026-06-08

## Overview

The `docs/` tree (~3095 lines across 24 files) and the duplicated parts of `README.md` are far too wordy: the same facts are re-stated across multiple pages, internal implementation detail (private symbol names, statement-by-statement SQL, test-harness asides) leaks into user-facing docs, and `data-model.md` still describes an OLD async/class-based architecture that no longer exists in source. This design doc specifies, **per file**, exactly what to cut, what to keep, and the single canonical home for each currently-duplicated topic — concrete enough to apply mechanically. Documentation-only: changes are confined to `docs/`, `README.md`, and `zensical.toml`; no source-code behavior changes.

## Success Criteria

- [ ] Every necessary fact (install, configure, CLI surface, data model, message envelope, WebUI API contracts, architecture concepts) survives, stated **once** in one canonical place; other mentions become a one-line cross-reference or are deleted.
- [ ] A strict **necessity test** is applied to every fact: if a user or operator does not need it to install, configure, use, or understand cafleet, it is **deleted** (not relocated).
- [ ] All **bare code symbol names** are removed from user docs — including public `broker.*` / `MULTIPLEXERS.*` / `CODING_AGENTS.*` dotted references — and replaced with plain-prose behavior descriptions. (Bare module/file names like `broker.py`, `cli.py` may remain only where they orient the reader.)
- [ ] All **stale content** describing the old async architecture is deleted (verified against `cafleet/src/cafleet/`): `RegistryStore`, `TaskStore`, `BrokerExecutor`, `async_sessionmaker`/`AsyncSession`, the `task_json` column, the `PATCH /api/v1/agents/<id>/placement` endpoint, and `metadata["recipientIds"]`.
- [ ] No reference material is lost: the CLI surface, all flags, the Error Messages table, the data-model column tables, the message-envelope schema, and the WebUI API request/response contracts remain intact.
- [ ] `zensical.toml` nav still resolves: no page is removed, so the nav is unchanged. Anchors that survive cross-references (`#full-semantics`, `#message-body-truncation`) are preserved.
- [ ] After the edit, the repository reads as if the removed/duplicated content never existed — no "see design NNNN for the old behavior" deprecation notices (per `config/claude/rules/removal.md`).

---

## Background

Source verification was performed against `cafleet/src/cafleet/` before specifying any deletion. The current broker is **synchronous and function-based** (`db/engine.py` uses `sessionmaker`, not `async_sessionmaker`); the `tasks` table uses **typed columns** with no `task_json` blob; `webui_api.py` defines no `PATCH .../placement` route; and there is no `metadata`/`recipientIds` field on tasks. The data-model column tables (fleets / agents / tasks / agent_placements) and the fleet-create bootstrap **are** accurate and must be kept. Everything flagged "stale" below is confirmed absent from current source.

---

## Specification

### Guiding principles (apply to every file)

1. **Necessity test first.** For each fact ask: "does a user or operator need this to install, configure, use, or understand cafleet?" If no → delete. When torn between dedupe and delete, delete.
2. **Single source of truth (SSOT).** Each duplicated topic has exactly one canonical home (table below). Every other occurrence is reduced to a one-line cross-reference or deleted.
3. **Prose, not symbols.** Strip every dotted code-symbol reference. Describe the observable behavior. Keep observable contract strings (error messages, CLI output shapes, flag names, env-var names, column names).
4. **No historical notices.** Removed content leaves no trace in user docs (no "previously…", no "deprecated…", no design-doc back-pointers). Git history and this design doc are the record.
5. **Keep reference material.** Column tables, message-envelope schema, WebUI API contracts, CLI flags, and the Error Messages table are necessary and stay (trimmed of symbols, not gutted).

### SSOT canonical-home map

| Topic | Canonical home (the ONE full statement) | Everywhere else |
|---|---|---|
| Fleet-create bootstrap (observable facts only) | `spec/cli-options.md` → `fleet create` | `concepts/fleet-isolation.md`, `spec/data-model.md`: drop the 5-step INSERT narrative; data-model keeps a single sentence "bootstrap is one all-or-nothing transaction" |
| Fleet soft-delete (observable behavior) | `spec/cli-options.md` → `fleet delete` | `spec/data-model.md`: keep only `deleted_at` column semantics; `concepts/fleet-isolation.md`: drop the soft-delete section |
| Built-in Administrator agent | `spec/data-model.md` → `agents` table | `concepts/overview.md`, `concepts/fleet-isolation.md`, `spec/webui-api.md`: minimal mention + link |
| `--full` + `CAFLEET_MAX_TEXT_LEN` truncation rules | `spec/cli-options.md` | `spec/message-envelope.md`, `concepts/tmux-push.md`: reference, do not re-derive |
| Bash-routing protocol | `concepts/bash-routing.md` | `concepts/coding-agents.md`, `spec/cli-options.md` (`member exec`), `reference/coding-agents/codex.md`, `…/opencode.md`: reference, do not re-explain |
| Pane-title asymmetry (only `claude --name` sets the pane title) | `concepts/coding-agents.md` → "Known asymmetries" | `concepts/member-lifecycle.md`, `reference/coding-agents/codex.md`, `…/opencode.md`: one-line link |
| Member-resolution + key-delivery rationale (shared by `send-input`/`exec`/`ping`) | `spec/cli-options.md` → one shared subsection | the three member subcommands reference it instead of repeating it |
| Integer-PK upgrade warning | `get-started/install.md` | `concepts/storage.md`: drop the duplicate |
| Task lifecycle / state values | `spec/message-envelope.md` | `concepts/storage.md`: drop the duplicate table |
| SQL schema (column/index/FK tables, ER diagram) | `spec/data-model.md` | `concepts/storage.md`: drop the duplicate ER diagram + column/index tables |
| CLI option-source matrix | `spec/cli-options.md` | `concepts/overview.md`: drop the duplicate |

### Stale fragments to DELETE outright (verified absent from source)

All in `spec/data-model.md` unless noted:

- The entire **"Operation mapping"** section (the `RegistryStore` and `TaskStore` method→SQL tables, including `task_json`, `INSERT … ON CONFLICT … task_json=excluded.task_json`, and `get_endpoints`/`_handle_get_task`).
- The entire **"Session ownership"** subsection (`async_sessionmaker[AsyncSession]`, `async with self._sessionmaker()`, `async with session.begin()`, "Route handlers and the `BrokerExecutor`").
- The phrase **"Set via `PATCH /api/v1/agents/{id}/placement`"** in the `agent_placements.tmux_pane_id` row → replace with plain prose ("set after the pane is spawned").
- The sentence about **`metadata["recipientIds"]`** in "Broadcast Grouping" (the summary task has no metadata column).
- All **`BrokerExecutor._handle_*`** references (`_handle_broadcast`, `_handle_ack`, `_handle_unicast`) → rewrite as plain prose or delete.
- The line **"Deregistration is handled in `RegistryStore.deregister_agent`."**
- Every reference to **`AdministratorProtectedError`** (a class that does **not** exist in current source — confirmed via grep). The real guard is the module-level `_is_administrator` check (discriminator `ADMINISTRATOR_KIND = "builtin-administrator"`), which raises `click.ClickException` with the literal strings `Administrator cannot be deregistered` and `Administrator cannot be a director`. Strip the non-existent class name; keep only those literal error strings (and the root-Director protection message) as observable contract.
- In `spec/webui-api.md`: the parenthetical "The summary row's metadata (`recipientIds`) is not needed…" → reword to drop `recipientIds` (keep the grouping point).

---

### Per-file directives

#### `README.md`

- **Cut**: Section 2 "Recommended settings" in full — both the §2.1 Claude permissions JSON block (byte-identical to `configure.md`) and the §2.2 Codex `config.toml` + rules block (a subset of `configure.md`'s fuller version).
- **Replace** Section 2 with a 1–2 line pointer: per-coding-agent config (Claude `permissions.allow`, Codex `config.toml` + rules, opencode) lives on the Configure page → `https://himkt.github.io/cafleet/get-started/configure/`.
- **Keep**: the video embed, §1 Install (both 1.1 skills and 1.2 CLI), §3 Examples, §4 Architecture blurb, §5 Contributing pointer. Renumber remaining sections so they stay sequential (Install=1, Examples=2, Architecture=3, Contributing=4, with the Configure pointer folded into Install or as a short standalone line).

#### `zensical.toml`

- **No change.** No page is removed or merged, so the nav stays valid. This task is a **verification only**: confirm no edits are needed and that the surviving cross-reference anchors are intact.

#### `docs/index.md`

- **Cut**: the `mermaid` component-overview diagram (a minimal variant of the fuller diagram on `concepts/overview.md`; the two are not byte-identical, so do not gate the cut on a verbatim match).
- **Keep**: the title, the one-paragraph intro, and the "Browse the docs" link list. The "Get started" button stays.
- Point architecture-curious readers to `concepts/overview.md` via the existing browse link (no new prose needed).

#### `docs/get-started/index.md`

- **No change** (already a minimal nav landing page).

#### `docs/get-started/install.md`

- **Keep** as the canonical home for install + the integer-PK upgrade warning (the `!!! warning "Upgrading across the integer-PK rearchitecture"` block stays here).
- No other change.

#### `docs/get-started/configure.md`

- **Keep** as the canonical config home — both code blocks (Claude `permissions.allow` JSON, Codex `config.toml` + rules) and the opencode section stay; README now points here.
- **Strip symbols** in the opencode section: replace `OpencodeAgent.ensure_available()` and the `CAFLEET_AGENT` preset references with plain prose ("on first opencode member spawn, cafleet writes the `cafleet` agent definition to `~/.opencode/agents/cafleet.md` if it does not already exist"). Keep the file path, the refresh instruction, and the MCP MUST-NOT pointer.
- **Keep** the "Building docs locally" section.

#### `docs/get-started/quickstart.md`

- **Keep** as the canonical walkthrough. Light touch only.
- **Strip symbols** if any remain (e.g. inline-preview prose); keep the raw CLI command sequence intact.

#### `docs/get-started/contributing.md`

- **Keep** as the canonical home for project structure + dev loop. No content cut.

#### `docs/concepts/overview.md`

This page is the worst offender for internal/duplicated detail. Reduce it to a true conceptual overview.

- **Keep**: the title, the opening conceptual paragraph (broker module is the single data-access layer for both CLI and WebUI; single-file SQLite; fleets partition agents; no HTTP server needed for agent ops), and the architecture `mermaid` diagram.
- **Cut**: the **"Component layout"** per-file table (internal file map — belongs to contributing/source, fails the necessity test).
- **Cut**: the **"Operation mapping"** CLI→broker table (internal SQL mapping; users use the CLI, the surface lives in `cli-options.md`).
- **Cut**: the **"CLI option sources"** table + the two paragraphs after it (duplicates `cli-options.md` Option Source Matrix and `server` config) → at most a one-line pointer to `spec/cli-options.md`.
- **Trim** the **"WebUI"** section to one short conceptual paragraph: a browser dashboard at `/`, no login, fleet picker → unified per-fleet timeline, Administrator is the implicit sender. Drop the implementation detail (5 s auto-refresh, CSS hover, React/Tailwind versions, `StaticFiles` mount path, `create_app()` warning). Point to `spec/webui-api.md` for contracts.
- **Cut**: **"Package structure"** (duplicates `contributing.md`) and **"Plugin packaging"** (contributor-internal, fails necessity test).
- **Trim** **"Design document orchestration skills"** to 2–3 sentences (the SDD skills exist; persisted, auditable message trail vs. ephemeral Agent Teams) → point to `get-started/quickstart.md` / `contributing.md`. Drop the role-file / communication-pattern / coexistence subsections and the skill table.
- **Strip** all `broker.*`, `server.py`, `webui_api.py` dotted symbols from surviving prose.

#### `docs/concepts/fleet-isolation.md`

- **Keep**: the fleet-isolation concept — `fleet_id` is the boundary, non-secret, a tidiness partition not a security boundary, no authentication, cross-fleet requests return "not found".
- **Cut**: the **"Fleet bootstrap (transactional)"** section in full (the sequence `mermaid` + the 5 ordered operations + the post-bootstrap invariant prose) → replace with a single sentence: fleet create must run inside a tmux session and atomically creates the fleet, its root Director, and the Administrator; see `spec/cli-options.md` `fleet create`.
- **Cut**: **"Fleet soft-delete"** and **"Soft-delete visibility"** sections in full → one-line pointer to `spec/cli-options.md` `fleet delete`.
- **Trim** **"Root Director protection"** to one observable sentence: the root Director cannot be deregistered; use `fleet delete` to tear down a fleet.
- **Cut**: **"Built-in Administrator agent"** section → one-line link to `spec/data-model.md` (Administrator canonical home).
- **Strip** all `broker.*` symbols from surviving prose.

#### `docs/concepts/storage.md`

- **Keep**: the storage concept — single SQLite file, SQLAlchemy 2.x sync `pysqlite` driver, Alembic via `cafleet db init`, `busy_timeout` concurrency note, "No physical cleanup" section, and the "contextId convention" paragraph.
- **Cut**: the **ER `mermaid` diagram** and the **column/index tables** (the "Predominantly relational model" table block) — duplicates `spec/data-model.md`. Replace with a one-line pointer to `spec/data-model.md` for the schema.
- **Cut**: the **"Upgrading across the integer-PK rearchitecture"** section (canonical in `get-started/install.md`).
- **Cut**: the **"Task lifecycle mapping"** table (canonical in `spec/message-envelope.md`).
- **Trim** **"Schema management"** to: a single initial Alembic migration; `cafleet db init` is idempotent (re-run after upgrades), refuses to auto-downgrade, and refuses an unknown schema. Drop the 6-state idempotency table and the `0001_initial_schema.py` / `down_revision` internals.
- **Cut**: the **"Session ownership"** paragraph (internal, symbol-laden).
- **Strip** all `broker.*` / `_try_notify_recipient` symbols; keep PRAGMA names and index names only where they are still referenced as observable facts (prefer dropping them with the duplicate tables).

#### `docs/concepts/member-lifecycle.md`

- **Keep**: the member concept (terminology: a member is a Director-spawned agent with a placement row; single-Director invariant) and the lifecycle `stateDiagram` (it is unique and orienting).
- **Trim** **"Atomic create flow"** to the observable three steps in prose (register with a pending placement → spawn the pane in the Director's window → record the real pane id; rollback on failure), stripped of `split_window` / `update_placement_pane_id` / `deregister_agent` symbols and the `-d`-flag focus digression (the focus point is already covered; keep one sentence).
- **Cut**: **"Spawn-prompt input modes"** (duplicates `cli-options.md` `member create`) → one-line pointer.
- **Cut**: **"Pane display-name propagation (claude backend only)"** → one-line link to `concepts/coding-agents.md` (pane-title canonical).
- **Cut**: **"Write-path member resolution"** (duplicates `cli-options.md` `send-input`/`exec`) → one-line pointer.
- **Trim** **"Commands"** to a bare list of the member subcommands → point to `cli-options.md` for flags.
- **Cut**: **"Operator diagnostics"** (`doctor`, duplicates `cli-options.md`), **"Base-dir resolution"** (owned by the `cafleet-base-dir` skill, tangential), and **"Supervision skills"** (duplicates the supervision/monitoring skills) → at most a one-line pointer each, or delete.
- **Strip** all dotted symbols throughout.

#### `docs/concepts/coding-agents.md`

- **Keep** as the canonical home for backend selection + the pane-title asymmetry. Keep the backend table, the spawn commands (observable), the mixed-backend note, and the **"Known asymmetries"** section (canonical pane-title statement).
- **Trim** the "Bash-disable parity" asymmetry bullet to reference `concepts/bash-routing.md` rather than re-explaining the protocol.
- **Strip** `CODING_AGENTS`, `build_spawn_argv`, `ensure_available`, `OpencodeAgent` symbols → plain prose; keep the literal spawn command strings and the `agent_placements.coding_agent` column name.

#### `docs/concepts/bash-routing.md`

- **Keep** as the canonical bash-routing home. Keep the two-paragraph behavior description (members auto-approve workspace-scoped; harness deny-list is the fallback trigger; member routes to Director who dispatches via the `!` shortcut; member reconsiders first).
- **Strip** `MULTIPLEXERS["tmux"].send_bash_command`, `CAFLEET_AGENT` symbols → prose. Keep the pointer to `skills/cafleet/reference/exec-routing.md` for the full convention.

#### `docs/concepts/tmux-push.md`

- **Keep**: the push-notification concept, the send-sequence `mermaid` diagram, and the "Design principles" list (best-effort, self-send skip, silent failure, no `TMUX` env var required).
- **Cut**: the **"CLI message body truncation"** section in full (lines covering `CAFLEET_MAX_TEXT_LEN`, `--full`, `truncate_text`, broadcast summary truncation) → one-line pointer to `spec/cli-options.md` (truncation canonical).
- **Cut**: the implementation note contrasting `send_inline_preview` vs `send_freetext_and_submit` (internal), and the "imported and instantiated per-call so per-test `monkeypatch.setattr` … is honored" test aside.
- **Trim** "Response annotations" and "Manual entry-point" to the observable facts (unicast responses carry a `notification_sent` boolean; the count is not persisted) stripped of symbol names.
- **Strip** `_try_notify_recipient`, `send_inline_preview`, `send_poll_trigger`, `TmuxMultiplexer` symbols throughout → prose.

#### `docs/concepts/token-reduction.md`

- The current page is a technique table, but each row is annotated with a symbol (`Compact rendered envelope (output.render_task)`, `_MEMBER_PROMPT_TEMPLATE … _resolve_prompt`, the six-caller list `_save_task`/`_read_task`/`_unicast_task_dict`/…, `broker._try_notify_recipient` / `TmuxMultiplexer.send_inline_preview` / `send_poll_trigger`, `output.render_agent`). **Strip the symbol from each row, keep the technique.** Do not gut the technique coverage — the per-technique rows are the useful content; only the parenthetical/inline symbol references are removed.
- **Keep** a short user-relevant summary of what keeps cafleet's output small: compact 2-line default message envelope (6 lines under `--full`), slim member spawn prompt, the core skill split with reference files loaded on demand, `member capture` defaulting to 30 lines, and the flat typed-column task shape (no JSON blob). Keep the page in nav.

#### `docs/spec/data-model.md`

- **Keep** (necessary reference): the four column tables (`fleets`, `agents`, `tasks`, `agent_placements`) with their constraints/notes, the index tables, and the "Foreign key enforcement" section (trimmed).
- **Cut wholesale** (stale): the entire **"Operation mapping"** section and the **"Session ownership"** section (both describe the absent async/class-based architecture).
- **Cut** (duplicate, not stale): the **"Fleet Lifecycle"** section — but **verify-before-cut**. This is the one whole-section deletion justified by "duplicate" rather than "absent from source", so before deleting it confirm every observable fact it states already lives at its canonical destination: the `fleet delete` directive in `cli-options.md` (the idempotent re-run message `Deleted fleet <id>. Deregistered 0 agents.`, "member panes are not auto-closed → run `cafleet member delete` first", and the `Deleted fleet <id>. Deregistered N agents.` output) plus the `deleted_at` column semantics retained here in the `fleets` table. If any fact in Fleet Lifecycle is found NOT covered by those destinations, fold it into the destination first, then delete the section. Do not delete on the "duplicates" label alone.
- **Reduce** the **"Root Director bootstrap"** subsection (under `fleets`) to a single sentence: the fleet row, root Director (+ placement), the `director_agent_id` back-reference, and the Administrator are written in one all-or-nothing transaction. Drop the 5 numbered INSERT/UPDATE steps.
- **Reduce** the `fleets` soft-delete prose to the `deleted_at` column semantics only (nullable; `NULL` = active, non-NULL = soft-deleted). Drop the transaction narrative and the `WHERE deleted_at IS NULL` short-circuit explanation.
- **Keep** the **"Built-in Administrator agent"** subsection as the Administrator canonical home, but trim to essentials: each fleet has exactly one Administrator, distinguished by the `agent_card_json.cafleet.kind == "builtin-administrator"` flag (no separate table); it is a write-only identity used as the WebUI's implicit sender; it cannot be deregistered and cannot be a Director; it is excluded from broadcast recipients. Drop the JSON-card example’s surrounding `json_extract` / `_is_administrator` symbol detail and the protection table — state the guards as plain behavior. **Strip** the stale `AdministratorProtectedError` reference entirely (the class does not exist in source — see the Stale-fragments list). Keep only the literal error strings that the guards actually raise: `Administrator cannot be deregistered` and `Administrator cannot be a director`.
- **Fix** the `agent_placements.tmux_pane_id` row: remove "Set via `PATCH /api/v1/agents/{id}/placement`" → "set after the pane is spawned".
- **Fix** "Broadcast Grouping": keep the `origin_task_id` grouping concept (referenced by `message-envelope.md` and `webui-api.md`) but delete the `metadata["recipientIds"]` sentence and the `BrokerExecutor._handle_*` symbols. **Cut** the "Known design debt — ACK timestamp inference" subsection (internal design rationale → belongs in this design doc / git, fails necessity test) or reduce to one sentence.
- **Rename** the **"Task Visibility Rules"** table verbs from the old A2A protocol names (`ListTasks` / `GetTask` / `SendMessage` / `CancelTask`) to the current CLI verbs (`message poll` / `message show` / `message send` / `message cancel`). Keep the inbox-privacy concept.
- **Cut** the "Deregistered Agents" section's speculative `cafleet db purge --older-than 30d` example (keep the one-line "no physical cleanup" fact; it is also in `storage.md`, so a single sentence + no example).
- **Strip** all remaining `broker.*` dotted symbols from surviving prose.

#### `docs/spec/message-envelope.md`

- **Keep** (necessary reference): the persisted-shape typed-column table, the rendered-shape field-decision table, the JSON/text-mode examples.
- **Cut**: the `render_task` Python code block (lines defining the function) — the field-decision table already conveys the compact shape; strip the `output.render_task` / `format_task` / `broker.broadcast_message` symbols from surrounding prose.
- **Trim** the truncation re-derivation: the `--full` / `CAFLEET_MAX_TEXT_LEN` behavior is canonical in `cli-options.md`; keep the existing "Flag cross-reference" section (it already points to `cli-options.md#full-semantics` / `#message-body-truncation`) and remove any in-page re-statement of the truncation rules beyond a one-line mention.
- **Keep** the link to `data-model.md` for the full SQL schema.

#### `docs/spec/cli-options.md`

This is the canonical CLI reference and the new canonical home for fleet bootstrap (observable), soft-delete (observable), `--full`/truncation, and the shared member-resolution rationale. Keep the reference value; remove internal narrative and repetition.

- **Keep** (necessary): the Option Source Matrix, Global Options, the `--full` semantics table, the Message Body Truncation table + `CAFLEET_MAX_TEXT_LEN` row, every per-subcommand flag table, all output shapes (text + JSON), and the **Error Messages** table.
- **`fleet create`**: keep the flag table, the "must run inside a tmux session" fact, and both output blocks (non-JSON + JSON). **Cut** the 5 numbered INSERT/UPDATE steps (lines listing `INSERT INTO fleets … director_agent_id=NULL`, etc.) → replace with one sentence: creates the fleet, its root Director (+ placement), and the built-in Administrator atomically (all-or-nothing). Strip `broker.*` symbols and the stale `AdministratorProtectedError` reference (non-existent class — see Stale-fragments list); keep the literal error strings (`cannot deregister the root Director; use 'cafleet fleet delete' instead`, `Administrator cannot be deregistered`).
- **`fleet delete`**: this becomes the canonical soft-delete home. Keep the observable behavior: soft-deletes the fleet; hidden from `fleet list`; message history preserved; idempotent (re-run prints `Deleted fleet X. Deregistered 0 agents.`); member panes are NOT auto-closed, so run `cafleet member delete` per member first. **Cut** the 3-step `UPDATE/UPDATE/DELETE` SQL listing and the "`WHERE deleted_at IS NULL` short-circuit" deep explanation. Keep the `Deleted fleet <id>. Deregistered N agents.` output line.
- **Member subcommands repetition** (point 2 of the request): the three subcommands `member send-input`, `member exec`, and `member ping` repeat verbatim (a) the 3-step member-resolution block, (b) the "two separate tmux invocations because tmux's `-l` flag is per-invocation" rationale, and (c) near-identical exit-code tables. **Factor these into ONE shared subsection** (e.g. "### Member targeting and key delivery") placed once before the three subcommands, covering: fleet-scoped `--member-id` resolution (cross-fleet → "not found"; missing placement row → error; pending placement → error), the per-invocation `-l` literal-flag rationale, and the common exit-code rows (tmux unavailable → 1, agent not found → 1, missing/pending placement → 1, send-keys failure → 1). Each of the three subcommands then keeps ONLY its unique surface (its specific flags, its key-sequence row, its unique validation rows, its output shape, and its own pending-placement message wording) and references the shared subsection for the rest.
- **Strip symbols**: remove `_load_authorized_member`, `MULTIPLEXERS.tmux.*`, `broker.get_agent`, `send_bash_command`, `send_poll_trigger`, `_require_fleet_id` dotted references throughout → plain prose. Keep CLI flag names, env-var names, and the literal error/`Error:` strings (observable contract).

#### `docs/spec/webui-api.md`

- **Keep** (necessary reference): all endpoint request/response contracts, the `X-Fleet-Id` header convention, the `kind` discriminator table, the broadcast-grouping table, and the error format.
- **Fix**: in the timeline section, reword "The summary row's metadata (`recipientIds`) is not needed…" to drop `recipientIds` while keeping the point that the frontend reconstructs broadcasts from delivery rows alone.
- **Trim** the Administrator mention to a brief note + link to `data-model.md` (Administrator canonical home); keep the `kind` discriminator (it is the API contract).
- **Strip** `broker.list_fleet_agents` / `_is_administrator` / `json_extract` symbols from the `kind`-derivation prose → "derived at read time from the stored agent card; there is no dedicated column."

#### `docs/reference/coding-agents/codex.md`

- **Keep** (operational essentials): the spawn flags + behavior, the `sandbox_workspace_write.writable_roots` caveat (with the `config.toml` snippet), the validated `codex-cli 0.128.0` version pin, the binary-not-found error, and the verification recipe.
- **Cut/trim**: collapse the `!` shell-shortcut section to a one-line reference to `concepts/bash-routing.md`; collapse "Pane-title asymmetry" to a one-line link to `concepts/coding-agents.md`. Drop rationale that outweighs operational fact.
- **Strip** any dotted symbols → prose.

#### `docs/reference/coding-agents/opencode.md`

- **Keep** (operational essentials): spawn flags, the validated `opencode 1.15.5` version pin, the install/binary-not-found pointer, the MCP-server MUST-NOT caveat, the safety-floor summary (catch-all-allow + specific-deny, deny-list only, no kernel sandbox), the preset file path + refresh recipe, the unwritable-`$HOME` failure note, and the verification recipe.
- **Cut**: the multi-paragraph **"Why we don't pass `--dangerously-skip-permissions`"** section → at most one sentence ("the bare-`opencode` TUI takes no skip-permissions flag; the safety floor pre-empts the `ask` state instead").
- **Cut**: the `permission/evaluate.ts` `findLast` evaluator internals and the `opencode_preset.py` / `PermissionRuleset` / `OpencodeAgentDefinition` / `CAFLEET_AGENT` dataclass-name detail → describe the agent definition's behavior in prose.
- **Trim** the `## Safety floor caveats` heading's exhaustive wrapper enumeration (the bulleted list of `bash -c` / `sh -c` / `fish -c` / `tclsh` / etc.) to the operational point: the deny-list cannot cover MCP tools, un-enumerated shell wrappers, in-language eval, or side-channel egress; use `codex` for kernel-enforced isolation. Keep the MCP MUST-NOT bullet.
- **Cut/trim** the `## Permission-popup recovery posture` and `## CAFleet writes one file under \`$HOME\`` headings to their operational kernel (a popup is a regression → escalate + capture; cafleet writes exactly one file at `~/.opencode/agents/cafleet.md`). (Both are real `##` headings in the current file — verified.)
- Collapse the `## The \`!\` shell-shortcut convention` and `## Pane-title asymmetry` headings to one-line links (bash-routing / coding-agents), as for codex.

#### `docs/api/*.md`

- **No change.** The four 3-line mkdocstrings stubs (`broker.md`, `config.md`, `coding-agent.md`, `multiplexer.md`) are kept as-is.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> After all edits, build the docs locally (`mise //:docs-build`) to confirm nav + cross-references resolve.

### Step 1: Spec pages (largest cuts + canonical homes)

- [ ] `docs/spec/data-model.md` — delete stale Operation mapping + Session ownership sections; cut Fleet Lifecycle (verify-before-cut: confirm coverage at `cli-options.md` `fleet delete` first); reduce bootstrap + soft-delete to observable minimums; trim Administrator to essentials (canonical home) and strip the stale `AdministratorProtectedError` reference; fix the `PATCH …/placement` phrase; fix Broadcast Grouping (drop `recipientIds`, drop `BrokerExecutor`); rename Task Visibility verbs to CLI verbs; cut ACK-timestamp-debt + `db purge` example; strip all dotted symbols. <!-- completed: -->
- [ ] `docs/spec/cli-options.md` — make `fleet create` (observable bootstrap) and `fleet delete` (observable soft-delete) the canonical homes; cut their SQL-step narratives; factor the shared "Member targeting and key delivery" subsection out of `send-input`/`exec`/`ping`; strip all dotted symbols; keep all flag tables, output shapes, and the Error Messages table. <!-- completed: -->
- [ ] `docs/spec/message-envelope.md` — cut the `render_task` code block; defer `--full`/truncation to `cli-options.md` (keep the Flag cross-reference); strip symbols; keep the schema + examples. <!-- completed: -->
- [ ] `docs/spec/webui-api.md` — drop `recipientIds` wording; trim Administrator to a link; strip the `kind`-derivation symbols; keep all endpoint contracts. <!-- completed: -->

### Step 2: Concepts pages (necessity-test trims + SSOT cross-refs)

- [ ] `docs/concepts/overview.md` — cut Component layout, Operation mapping, CLI option sources, Package structure, Plugin packaging; trim WebUI + orchestration-skills to short concept paragraphs; keep the architecture diagram; strip symbols. <!-- completed: -->
- [ ] `docs/concepts/fleet-isolation.md` — cut the bootstrap + soft-delete + Administrator sections (→ links to canonical homes); keep the isolation concept; trim Root Director protection to one sentence; strip symbols. <!-- completed: -->
- [ ] `docs/concepts/storage.md` — cut the duplicate ER diagram + column/index tables (→ `data-model.md`), the integer-PK upgrade section (→ `install.md`), and the Task lifecycle table (→ `message-envelope.md`); trim Schema management; cut Session ownership; strip symbols. <!-- completed: -->
- [ ] `docs/concepts/member-lifecycle.md` — keep the concept + state diagram; trim Atomic create flow to prose; cut Spawn-prompt modes / Pane display-name / Write-path resolution / Operator diagnostics / Base-dir / Supervision to one-line pointers; strip symbols. <!-- completed: -->
- [ ] `docs/concepts/coding-agents.md` — keep as pane-title canonical; trim the bash-disable bullet to a bash-routing link; strip symbols. <!-- completed: -->
- [ ] `docs/concepts/bash-routing.md` — keep as canonical; strip symbols; keep the exec-routing skill pointer. <!-- completed: -->
- [ ] `docs/concepts/tmux-push.md` — cut the CLI truncation section (→ `cli-options.md`) and the implementation/test asides; trim Response annotations; strip symbols; keep the concept + diagram + design principles. <!-- completed: -->
- [ ] `docs/concepts/token-reduction.md` — strip the per-row symbol from each technique row (keep the technique coverage, do not gut); keep in nav. <!-- completed: -->

### Step 3: Reference (per-backend) pages

- [ ] `docs/reference/coding-agents/codex.md` — keep operational essentials + verification recipe; collapse `!`-shortcut and pane-title to links; strip symbols. <!-- completed: -->
- [ ] `docs/reference/coding-agents/opencode.md` — cut the `--dangerously-skip-permissions` rationale + `findLast`/preset-dataclass internals; trim safety-floor caveats to the operational point; keep MCP MUST-NOT, version pin, refresh recipe, verification recipe; collapse `!`-shortcut and pane-title to links. <!-- completed: -->

### Step 4: Get-started + landing + README

- [ ] `docs/get-started/configure.md` — strip the opencode symbol references to prose; keep all config blocks (canonical config home). <!-- completed: -->
- [ ] `docs/get-started/install.md` — confirm it remains the canonical integer-PK upgrade-warning home (no cut). <!-- completed: -->
- [ ] `docs/get-started/quickstart.md` — light symbol strip only; keep the walkthrough. <!-- completed: -->
- [ ] `docs/get-started/contributing.md` — keep as the canonical project-structure / dev-loop home (no cut). <!-- completed: -->
- [ ] `docs/get-started/index.md` — confirm no change needed. <!-- completed: -->
- [ ] `docs/index.md` — cut the duplicate component-overview mermaid; keep intro + browse links. <!-- completed: -->
- [ ] `README.md` — collapse Section 2 to a Configure-page pointer; keep Install, Examples, Architecture blurb, Contributing pointer, video; renumber sections. <!-- completed: -->

### Step 5: Nav verification + build

- [ ] `docs/api/*.md` — confirm the four mkdocstrings stubs (`broker.md`, `config.md`, `coding-agent.md`, `multiplexer.md`) need no change and are left as-is. <!-- completed: -->
- [ ] `zensical.toml` — verify no nav edit is required (no page removed) and that surviving anchors (`#full-semantics`, `#message-body-truncation`) are intact. <!-- completed: -->
- [ ] Run `mise //:docs-build` and confirm a clean build with all internal cross-references resolving; spot-check that no stale symbol / stale-architecture phrase and no deprecation notice remains (grep for `RegistryStore`, `TaskStore`, `BrokerExecutor`, `async_sessionmaker`, `task_json`, `PATCH /api/v1/agents`, `recipientIds`). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-08 | Initial draft |
