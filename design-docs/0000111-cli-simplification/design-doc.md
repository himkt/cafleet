# CLI Simplification: Redesign the cafleet Command Surface and Rewrite SPEC.md

**Status**: Approved
**Progress**: 20/28 tasks complete
**Last Updated**: 2026-06-25

## Overview

The `cafleet` CLI has grown to 29 commands — 5 command groups plus 4 top-level
commands — with accumulated inconsistencies (positional vs. flag fleet ids, two
"absent" glyphs, split exit codes for one error, hidden flags, a `member`/`agent`
conceptual overlap). This
design **deliberately breaks the behavior-preserving contract** that `SPEC.md`
currently enforces, redesigns the CLI for ergonomics (25 commands, 5 groups, a
flatter single-baseline schema), and rewrites `SPEC.md` to describe the new
surface. It also enumerates every downstream `docs/`, `README.md`, `.claude/`,
and `SKILL.md` edit the change forces, since the project's
documentation-maintenance rule makes those first-class targets.

## Success Criteria

- [ ] `SPEC.md` describes the new CLI surface end-to-end (command tree, options,
      `CAFLEET_*` config, single-baseline schema, HTTP API, error/exit model,
      output rules) with no residue of the removed surface.
- [ ] The new command tree is fully specified: every command, option name, type,
      default, required-ness, documented-vs-hidden status, and exit code.
- [ ] Every old→new command/flag mapping is captured so reimplementers and
      doc-editors can mechanically translate.
- [ ] The schema simplifications (single baseline `CREATE`, nullable
      `to_agent_id`) are specified at contract level.
- [ ] The coding-agent simplification (env-var context injection replacing the
      prompt mini-language) is specified.
- [ ] A complete, file-by-file follow-up checklist enumerates every `docs/`,
      `README.md`, `.claude/rules/`, and `skills/**` edit required to keep
      documentation in sync (the code refactor itself is out of scope).

---

## Background

`SPEC.md` is today a **behavior-preserving reimplementation specification**: §1
declares the goal as exposing "the same interface as the reference
implementation," and its non-goals forbid new/changed commands, flags,
endpoints, and any schema redesign (so databases interoperate across
implementations). The user has **explicitly chosen to lift those non-goals**.
The DB need not interoperate, no production database must survive, and the CLI
may be reshaped for simplicity.

The current surface (per `SPEC.md` §10) is 29 commands:

- top-level: `db init`, `setup`, `server`, `doctor`
- `fleet`: `create`, `list`, `show`, `delete`
- `agent`: `register`, `list`, `show`, `deregister`
- `message`: `send`, `broadcast`, `poll`, `ack`, `cancel`, `show`
- `member`: `create`, `delete`, `list`, `capture`, `send-input`, `exec`, `ping`, `nudge`
- `monitor`: `start`, `status`, `config`

`SPEC.md` itself flags the accidental quirks this design fixes: the
optional-at-parser `--fleet-id` with a hand-written exit-1 callback (§6.3), the
positional-vs-flag fleet id split (§6.3), the dual absent glyphs (§6.4), the
root-Director guard that raises exit 2 from the broker but exit 1 from `member
delete` for the identical string (§6.2/§7.2), the `to_agent_id = 0` sentinel
(§5.5), and the prompt brace mini-language (§6.7).

### Decisions taken (user-approved)

| # | Decision |
|---|----------|
| Q1 Scope | `SPEC.md` rewrite **plus** an enumerated follow-up checklist of every `docs/`/`SKILL.md` edit (as Implementation steps). The code refactor is **out of scope** for this doc. |
| Q2 Renames | **`member` splits into `agent` + `pane`; `fleet`/`message`/`monitor` keep their names.** `member` is replaced by `agent` (registry + lifecycle CRUD) and a new `pane` group (pane-interaction ops). The `fleet`, `message`, and `monitor` group names are retained as-is. |
| Q3 Migration | **Hard cut / greenfield-only.** One baseline `CREATE` schema; drop the 5-migration chain and the `db init` upgrade guards. Existing DBs are not migrated. |
| Q4 Schema bundle | **Three of four.** (i) nullable `to_agent_id`; (ii) drop the prompt mini-language, inject context as env vars; (iii) single absent glyph. The opencode-dir unification was **dropped** — the two opencode paths serve distinct purposes (§9). |
| Q5 Consistency | **Adopt three, exclude one.** Adopt: standardize `--fleet-id`; one error/exit model; hidden-flag cleanup. **Exclude:** do *not* unify the `--json` emit path — keep the current `--json` behavior, including the `fleet`-group local `--json`. |
| Q6 Commands | **Both.** Remove standalone `db init` (fold into `setup`); collapse `member ping` + `member nudge` into one `pane wake` with `--poll-only` vs `--message`. |
| Keep-as-is | Preserve as load-bearing (not quirks): the Esc-first keystroke matrix, the monitor claim/heartbeat/clear protocol, the best-effort notification booleans, the fail-fast PRAGMAs, and `ack`/`cancel` as two distinct verbs (different authz). |

---

## Specification

### 1. Target command tree (25 commands, 5 groups + 3 top-level)

```
cafleet setup     # schema create + skills install (absorbs the old `db init`)
cafleet doctor
cafleet server

cafleet fleet     create | list | show | delete
cafleet agent     register | list | show | deregister | spawn
cafleet pane      capture | input | exec | wake
cafleet message   send | broadcast | poll | ack | cancel | show
cafleet monitor   start | status | config
```

`agent` is now the single mental model for **registry + lifecycle** (a "member"
is just an agent with a placement); `pane` is the single home for **keystroke
interaction** with a pane-bound agent.

**Accepted residue — `--agent-id` polarity.** `--agent-id` denotes the **calling
agent (requester)** in `agent show` and every `message *` command, but the **target
agent** in `agent deregister` and every `pane *` command. This polarity is
inherited from the current surface and is retained deliberately rather than
renamed (a rename would break every existing call site beyond the scope of this
doc). Each command's §5/§6 entry states its role explicitly, and the SPEC §6.3
rewrite fixes the role per command. The env-var default in §3/§9 binds
`--fleet-id` **only**, precisely to avoid auto-defaulting an overloaded
`--agent-id` into the wrong polarity.

### 2. Old → new mapping (complete)

| Old command | New command | Notes |
|---|---|---|
| `db init` | *(removed)* | Folded into `setup`. |
| `setup` | `setup` | Now also creates the schema to head. `--agent` (multiple) unchanged. |
| `server` | `server` | Unchanged. |
| `doctor` | `doctor` | Unchanged (global `--json` only). |
| `fleet create` | `fleet create` | Options unchanged; local `--json` kept (Q5 exclude). `--full` promoted to documented. |
| `fleet list` | `fleet list` | Unchanged; local `--json` kept. |
| `fleet show <fleet_id>` | `fleet show --fleet-id <id>` | Positional → `--fleet-id`. |
| `fleet delete <fleet_id>` | `fleet delete --fleet-id <id>` | Positional → `--fleet-id`. Idempotent. |
| `agent register` | `agent register` | Unchanged. |
| `agent list` | `agent list` | Absorbs `member list`; gains a placement/pane column when present; `--activity` folded in here; `--full` documented. |
| `agent show` | `agent show` | `--agent-id` (fleet-gated requester), `--id` (target), `--full` documented. |
| `agent deregister` | `agent deregister` | Absorbs `member delete`; gains `--force`. Tears down the pane if one exists. |
| `member create` | `agent spawn` | The one genuinely distinct op: register **and** spawn a pane. |
| `member delete` | `agent deregister --force` | Merged (see §6). |
| `member list` | `agent list` | Merged. |
| `member capture` | `pane capture` | `--member-id`→`--agent-id`; `--lines` kept, `--tail` alias dropped. |
| `member send-input` | `pane input` | `--member-id`→`--agent-id`. |
| `member exec` | `pane exec` | `--member-id`→`--agent-id`; positional `command`. |
| `member ping` | `pane wake --poll-only` | Merged into `pane wake`. `--quiet` dropped. |
| `member nudge --agent-id <sender> --member-id <target>` | `pane wake --agent-id <target> --message --from <sender>` | Merged into `pane wake`. **Flag-role swap:** old `--agent-id` (sender) → `--from`; old `--member-id` (target) → `--agent-id`. A mechanical translation that keeps the same flag names would swap sender and target. |
| `message send` | `message send` | `--quiet` dropped. |
| `message broadcast` | `message broadcast` | Count reporting fixed (see §8). |
| `message poll` | `message poll` | — |
| `message ack` | `message ack` | `--quiet` dropped. |
| `message cancel` | `message cancel` | — |
| `message show` | `message show` | — |
| `monitor start` | `monitor start` | Unchanged (`--fleet-id`, `--tick`≥1=5). |
| `monitor status` | `monitor status` | Unchanged. |
| `monitor config` | `monitor config` | Unchanged. |

### 3. Global options and the `--fleet-id` standardization (Q5 adopt)

- Global, before any subcommand: `--json` (boolean, default false) and
  `--version` (prints `cafleet <version>`, exit 0, bypasses `--fleet-id`).
- **`--fleet-id` is a plain required option** (integer) on every subcommand that
  operates within a fleet. There is no longer an optional-at-parser flag with a
  hand-written exit-1 callback: a missing `--fleet-id` is a parser-native
  missing-required-option error (**exit 2**). The custom hint message and the
  whole guard mechanism (old §6.3) are deleted.
- **`CAFLEET_FLEET_ID` env default.** When set, it supplies the default for
  `--fleet-id` so it need not be retyped each call. An explicit `--fleet-id`
  overrides it. A non-integer value fails loudly at parse time (exit 2).
- **`--fleet-id` everywhere it applies**, including `fleet show` and `fleet
  delete` (positional `fleet_id` removed). Commands that do not operate within a
  single existing fleet — `setup`, `doctor`, `server`, `fleet create`, `fleet
  list` — do not take `--fleet-id` and reject it with the parser's
  unknown-option error (exit 2).

Subcommands taking `--fleet-id`: all of `agent *`, `pane *`, `message *`, `monitor
*`, plus `fleet show` and `fleet delete`.

### 4. Error and exit-code model (Q5 adopt)

One unified model replaces the per-path split:

- **Exit 2** — argument/parse/usage errors: missing required option (now
  including `--fleet-id`), unknown option, invalid integer, integer-range
  violation, mutually-exclusive-option violations.
- **Exit 1** — application/runtime errors: runtime conflicts (one-monitor rule,
  Administrator immutability, not-enrolled, not-found-on-delete), broker
  value/permission errors surfaced through the CLI, the spawn rollback ladder,
  and the pane-teardown timeout.

The faithful exit-code quirk is **removed**: the root-Director-deregistration
guard now raises a single application error (**exit 1**) on both the broker side
and the CLI side, for the same message. The old `member delete` explicit exit-2
timeout becomes a runtime error (**exit 1**), consistent with "runtime → exit 1."

All errors print `Error: <message>` to stderr; usage errors additionally print a
usage line. The WebUI maps the same broker errors to HTTP status + `{"detail":
<string>}` (unchanged).

### 5. `agent` group (registry + lifecycle)

- **`agent register`** — `--name` (required), `--description` (required),
  `--skills` (optional JSON). Unchanged from today.
- **`agent list`** — `--full` (documented), `--activity` (documented; folds in
  the old `member list --activity`). Lists active agents; when an agent has a
  placement, the row shows a placement/pane column. Empty case `No agents
  found.`.
- **`agent show`** — `--agent-id` (required, fleet-gated requester), `--id`
  (required, target), `--full` (documented). Target not found → application
  error `Agent <id> not found`.
- **`agent deregister`** — `--agent-id` (required, fleet-gated target),
  `--force`/`-f`. Behavior:
  - If the target has **no pane**: registry soft-delete (the old `agent
    deregister`). Nothing deregistered → application error `agent <agent_id> not
    found or already deregistered.`. Success text `Agent deregistered
    successfully.`.
  - If the target **has a pane**: tear the pane down (send the backend's exit
    keystroke, wait up to 15 s for graceful close), then soft-delete. `--force`
    skips the graceful wait and force-closes the pane immediately. A pane that
    fails to close within 15 s without `--force` → application error (exit 1).
  - Root-Director target → application error (exit 1) `cannot deregister the root
    Director; use 'cafleet fleet delete' instead`. Administrator target →
    application error `Administrator cannot be deregistered`.
- **`agent spawn`** (was `member create`) — `--agent-id` (required, the
  Director), `--name` (required), `--description` (required), `--coding-agent`
  (choice, optional — resolved when absent), `--model` (optional), `--role`
  (choice `member`/`monitor`, default `member`), `--prompt-file` (optional),
  `--full` (documented), and a **positional variadic** `prompt_argv`. Spawn
  sequence, model-validation-before-side-effects, the rollback ladder, and the
  one-monitor-per-fleet guard are preserved from old `member create`. **The
  prompt mini-language is removed** (see §9): `--prompt-file` and the positional
  prompt are passed **verbatim**; fleet/agent/director context is injected as
  environment variables into the spawned pane.

### 6. `pane` group (keystroke interaction)

All `pane` subcommands target a pane-bound agent via **`--agent-id`** (the old
`--member-id` is renamed for one consistent rule). The shared resolution helpers
(require-pane, load-authorized-member, the no-pane error surfaces) are preserved
with `agent`/`pane` wording in place of `member`.

Documented-vs-hidden status: the hidden-flag cleanup (Q5) promotes the
carried-forward interaction flags to **documented** (`pane capture
--lines`/`--ansi`/`--no-ansi`, `pane input --choice`/`--freetext`); no `pane`
flag remains hidden. The single authoritative per-option documented-vs-hidden
enumeration for the **whole** command tree is fixed in the SPEC §6.3 rewrite
(§14); the §5/§6 entries here annotate the notable cases only.

- **`pane capture`** — `--agent-id` (required), `--lines` (integer, default
  **20**), `--ansi`/`--no-ansi` (documented). The `--tail` alias is dropped;
  `--lines` is the single spelling.
- **`pane input`** (was `member send-input`) — `--agent-id` (required),
  `--choice` (1..3, documented), `--freetext` (string, documented). Keystroke
  mechanics unchanged.
- **`pane exec`** (was `member exec`) — `--agent-id` (required), positional
  `command`. Keystroke mechanics unchanged.
- **`pane wake`** (collapses `member ping` + `member nudge`) — `--agent-id`
  (required, target). Mode is selected by mutually-exclusive flags:
  - `--poll-only` (was `member ping`): inject the Esc-first inbox-poll trigger
    into the target pane. No sender, no text.
  - `--message` (was `member nudge`): inject the task + preview. Requires
    `--from <sender-agent-id>` (the dispatching Director) and `--text <text>`.
  - Exactly one of `--poll-only` / `--message` is required; supplying both, or
    `--message` without `--from`/`--text`, is a usage error (exit 2). `--quiet`
    is dropped.

### 7. `message` group

All six subcommands route through the existing `client_command` wrapper with the
**global** `--json` (Q5 exclude leaves this path unchanged). `--full` is
promoted to documented; `--quiet` is dropped from `send` and `ack`.

- **`message send`** — `--agent-id`, `--to`, `--text`, `--full`. Fleet-gated;
  truncates task echo. Output `Message sent.` + the formatted task.
- **`message broadcast`** — `--agent-id`, `--text`, `--full`. Count reporting fixed
  (§8).
- **`message poll`** — `--agent-id`, `--full`. Empty `No messages found.`.
- **`message ack`** — `--agent-id`, `--task-id`, `--full`. Prefix `Message
  acknowledged.`.
- **`message cancel`** — `--agent-id`, `--task-id`, `--full`. Prefix `Task
  canceled.`.
- **`message show`** — `--agent-id`, `--task-id`, `--full`.

### 8. `message broadcast` count fix (Q4 follow-on)

Today the text overloads `recipients=<count>` with the *notification-success*
count (`notifications_sent_count`), which diverges from the real recipient count
`N` in the summary text `Broadcast sent to {N} recipients`. The new surface
reports both as separate fields:

- Text: `broadcast id=<task_id> recipients=<N> delivered=<k>`, where `N` is the
  real recipient count and `k` is the count of best-effort inline previews that
  landed.
- JSON: the result object carries both `recipients` (`N`) and `delivered` (`k`).

The broker's `broadcast_message` already computes both values; this change stops
conflating them at the CLI boundary.

### 9. Coding-agent simplifications (Q4)

- **opencode base dirs unchanged (distinct by purpose).** cafleet keeps two
  opencode paths because they serve **different purposes** and are not
  interchangeable: the agent **preset** lives at `~/.opencode/agents/cafleet.md`,
  opencode's mandated `--agent cafleet` discovery path (verified against opencode
  1.15.5); `~/.config/opencode/` is cafleet's own skills-install / home-detection
  target. The "refuse to overwrite a non-regular-file target" fail-fast on the
  preset write is preserved.
- **Drop the prompt mini-language.** `agent spawn` no longer performs
  `{placeholder}` brace substitution; the unknown-placeholder and malformed-brace
  error surfaces are removed. `--prompt-file` content and the positional prompt
  are delivered **verbatim**. The spawn instead forwards three identity values as
  **environment variables** into the new pane via the multiplexer's `split_window
  env=...` (alongside the already-forwarded `CAFLEET_DATABASE_URL`):
  `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID` (the spawned agent's own id), and
  `CAFLEET_DIRECTOR_AGENT_ID` (its Director's id).

  **Identity-delivery mechanism (full).** Only `CAFLEET_FLEET_ID` is auto-wired as
  a flag default — it defaults `--fleet-id` everywhere (§3), because a spawned
  agent's fleet is unambiguous. `CAFLEET_AGENT_ID` and `CAFLEET_DIRECTOR_AGENT_ID`
  are **not** auto-bound to any flag default, precisely because `--agent-id` is
  polarity-overloaded (requester in `message *`/`agent show`, target in `pane
  *`/`agent deregister`; §1) — auto-defaulting it would attribute target-role
  commands to the wrong agent. They are instead **machine-readable identity the
  agent passes explicitly**: a self-attributed call is `cafleet message send
  --agent-id $CAFLEET_AGENT_ID --to $CAFLEET_DIRECTOR_AGENT_ID --text ...`; a poll
  is `cafleet message poll --agent-id $CAFLEET_AGENT_ID`. The cafleet skill (loaded by
  every spawned agent) documents reading `$CAFLEET_AGENT_ID` /
  `$CAFLEET_DIRECTOR_AGENT_ID` and passing them explicitly; a Director may also
  embed the literal ids in its verbatim prompt. This wholly replaces the old
  `{agent_id}`/`{director_agent_id}` brace substitution. The SPEC rewrite fixes
  the exact env-var names and this read-then-pass convention as the contract.

### 10. Schema simplifications (Q3 + Q4)

- **Single baseline schema.** The six tables (§5.2 of the current SPEC: `fleets`,
  `agents`, `agent_placements`, `tasks`, `monitor_config`, `monitor_runtime`)
  are created by **one baseline `CREATE`** that yields the final schema directly.
  The 5-migration chain (`0001`–`0005`) and the `db init` upgrade machinery —
  Guard A (existing-tables-no-version), Guard B (unknown-revision), the
  already-current / fresh / upgraded outcome strings, and the version table — are
  **removed**. `setup` creates the schema fresh; there is no in-place upgrade
  path and no reference-era DB migration. Indexes (the four non-unique indexes)
  and the per-connection PRAGMAs (`foreign_keys=ON`, `busy_timeout=5000`) are
  preserved, as is the create-order forward-reference (or it is rendered moot by
  a single ordered baseline — the SPEC rewrite states the final ordering).
- **Nullable `to_agent_id`.** `tasks.to_agent_id` becomes a nullable integer
  column; `broadcast_message` writes **`NULL`** on the summary row instead of the
  `0` sentinel. `get_task` and `format_task` test `to_agent_id IS NULL` / `is
  None` instead of the `if to_id:` truthiness dance. The §5.5 sentinel section is
  rewritten to describe the nullable column.

### 11. Output: single absent glyph (Q4)

The two "absent/empty" glyphs (ASCII `-` and EM DASH `—`) collapse to **one
glyph, the ASCII hyphen-minus `-`**, used by every formatter for an absent or
empty cell. Rationale: portable, no Unicode dependency, already the dominant
form. The golden-output tests assert the single glyph.

### 12. `--json` behavior retained (Q5 exclude)

The `--json` emit path is **not** unified. The `agent`/`message` groups emit JSON via
the global `--json` through `client_command`; the `fleet` group keeps its local
`--json` flag OR-ed with the global one, with its own emit path, exactly as
today. Both `--json` and `--full` are promoted to **documented** flags, but the
dual-path structure is preserved. The WebUI continues to bypass truncation and
preserve key order + raw UTF-8.

### 13. Preserved behavior (keep-as-is)

These are load-bearing and unchanged: the Esc-first keystroke matrix (the
`-l`/Enter ordering, the two sleeps, the sanitizer substitutions), the monitor
claim/heartbeat/clear single-instance protocol and liveness probe, the
best-effort notification booleans (`notification_sent` /
`notifications_sent_count`; never roll back the insert), the fail-fast
per-connection PRAGMAs and "exactly one row" broker invariants, and `ack`/`cancel`
as two distinct verbs with different authorization (recipient-acks /
sender-cancels).

### 14. SPEC.md rewrite plan

The rewrite edits these sections of `SPEC.md` (the file stays the single
authoritative reimplementation spec; only the contract surfaces change):

- **§1 Overview & goals / non-goals** — replace the behavior-preserving /
  interoperability framing with the new-surface framing: the spec now describes
  the *redesigned* CLI; DB interoperability and reference-parity are no longer
  goals.
- **§3/§4 module layout & graph** — drop the `member` CLI grouping; reflect
  `agent`/`pane`/`message` groups. No module-dependency edges change (the split is a
  CLI-command-tree change, not a module change).
- **§5.5** — rewrite the `to_agent_id = 0` sentinel section as the nullable
  column.
- **§6.1 / §8** — replace the 5-migration chain + `db init` driver with the
  single-baseline `CREATE`; remove the upgrade guards and outcome strings.
- **§6.3 CLI** — rewrite the command tree: remove the `--fleet-id` optional+
  callback guard (now plain required + `CAFLEET_FLEET_ID`); convert `fleet
  show/delete` to `--fleet-id`; fold `member` into `agent`/`pane`; document
  `--full`/`--json`; remove `--quiet`/`--tail`; add
  `pane wake` modes; absorb `db init` into `setup`.
- **§6.4 Output** — single absent glyph; documented flags; `--json` behavior note
  (dual path retained).
- **§6.7 Coding agents** — remove the prompt mini-language; env-var injection.
  The opencode base dirs are unchanged (the two paths differ by purpose);
  rewrite §6.7's "two dirs coexist" wording to state the purpose distinction
  affirmatively rather than as a faithful-to-reference quirk.
- **§6.2** — broadcast count fields; remove the root-Director exit-code split
  note.
- **§7.2 exit-code policy** — one model; remove the per-path split and the
  explicit `member delete` exit-2 timeout note.
- **§10 CLI parity checklist** — replace with the new command checklist (rename
  it away from "parity," since parity is no longer the goal).
- **§11** — drop the now-resolved clarifications (sentinel, migration choice,
  exit-code quirk) and record the new decisions.

---

## Implementation

> Scope per Q1: this doc delivers the `SPEC.md` rewrite **and** the enumerated
> downstream-doc follow-up checklist. The **code refactor is out of scope** —
> it belongs to a later execute-workflow design doc. Documentation-first order
> (the project rule) means the SPEC and docs are updated as the authoritative
> target, then code follows in that separate cycle.

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Rewrite SPEC.md to the new surface

- [x] Apply the §14 rewrite plan to `SPEC.md` section by section. <!-- completed: 2026-06-25T08:35 -->
- [x] Verify no residue of removed surface remains (no `member` group, no `db
      init`, no positional `fleet_id`, no `to_agent_id = 0`, no migration chain,
      no `--quiet`/`--tail`, no prompt mini-language, no dual absent glyph, no
      exit-code split). <!-- completed: 2026-06-25T08:35 -->
- [x] Confirm the new command checklist lists all 25 commands with exact options,
      types, defaults, hidden-vs-documented, and exit codes. <!-- completed: 2026-06-25T08:35 -->

### Step 2: README.md

- [x] Update `README.md` to the new command tree, groups, and any CLI examples
      (architecture/CLI-surface section). <!-- completed: 2026-06-25T08:48 -->

### Step 3: docs/ — concepts and spec pages (contract-facing)

- [x] `docs/spec/cli-options.md` — rewrite the full command/option reference to
      the new surface (the largest single edit). <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/spec/data-model.md` — nullable `to_agent_id`; single-baseline schema
      (drop migration-chain language). <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/spec/message-envelope.md` — `to_agent_id` nullable; broadcast
      `recipients`/`delivered` split. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/spec/webui-api.md` — confirm/adjust any references to schema or
      command names. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/overview.md` — command-group overview (`agent`/`pane`/`message`). <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/member-lifecycle.md` — reframe around `agent spawn` / `agent
      deregister --force`; rename references. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/tmux-push.md` — `pane` group ops; `pane wake` modes. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/bash-routing.md` — `pane exec`, `pane wake --poll-only`. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/monitoring.md` — monitor + `pane wake` wording. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/storage.md` — single-baseline schema; remove `db init`
      upgrade language. <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/coding-agents.md` — env-var context injection (no prompt
      mini-language). <!-- completed: 2026-06-25T09:05 -->
- [x] `docs/concepts/token-reduction.md` — update any command references. <!-- completed: 2026-06-25T09:05 -->

### Step 4: docs/ — get-started, how-to, reference, api, index

- [x] `docs/index.md`, `docs/get-started/index.md`,
      `docs/get-started/install.md`, `docs/get-started/quickstart.md`,
      `docs/get-started/configure.md` (add `CAFLEET_FLEET_ID`),
      `docs/get-started/contributing.md` — update CLI examples and `setup`
      (no `db init`). <!-- completed: 2026-06-25T09:30 -->
- [x] `docs/how-to/mixed-backend-team.md`, `docs/how-to/monitor-and-recover.md`,
      `docs/how-to/design-doc-development.md` — rename old commands. <!-- completed: 2026-06-25T09:30 -->
- [x] `docs/reference/coding-agents/claude.md`, `.../codex.md`,
      `.../opencode.md` — update CLI examples (`agent spawn`, `pane *`); the two
      opencode base dirs are unchanged. <!-- completed: 2026-06-25T09:30 -->
- [x] `docs/api/coding-agent.md` — env injection / no mini-language. <!-- completed: 2026-06-25T09:30 -->

### Step 5: .claude/rules and project skill

- [ ] `.claude/rules/bash-tool.md` — `cafleet member ping`→`pane wake
      --poll-only`; `cafleet member exec`→`pane exec`; Director-side
      `member ping`/`member exec` primitives renamed. <!-- completed: -->
- [ ] `.claude/rules/coding-agent-overlay.md` — overlay command references. <!-- completed: -->
- [ ] `.claude/skills/skill-author/SKILL.md` — `cafleet member create` examples. <!-- completed: -->

### Step 6: skills/** (SKILL.md, workflow bodies, roles, reference)

- [ ] `skills/cafleet/SKILL.md` and `skills/cafleet/reference/*` — `cli.md`,
      `director.md`, `recovery.md`, `supervision.md`, `broadcast.md`,
      `output-flags.md`, `exec-routing.md`, and the `coding-agent/` overlays
      (`_template.md`, `claude.md`, `codex.md`, `opencode.md`); plus
      `skills/cafleet/roles/{director,member,monitor}.md`. <!-- completed: -->
- [ ] `skills/cafleet-design-doc/**` — `SKILL.md`, `reference/coordination.md`,
      and every `create/`, `interview/`, `execute/` workflow body + `roles/*.md`
      that embeds old `cafleet member`/`message` commands. <!-- completed: -->
- [ ] `skills/cafleet-research/**` — `SKILL.md` and every `report/`,
      `presentation/` workflow body + `roles/*.md` with old commands. <!-- completed: -->
- [ ] **Additive edit (not a rename):** document the env-var identity convention
      in the cafleet skill — teach reading `$CAFLEET_AGENT_ID` /
      `$CAFLEET_DIRECTOR_AGENT_ID` and passing them explicitly (the read-then-pass
      convention that replaces the dropped `{agent_id}`/`{director_agent_id}` brace
      mini-language, per §9), in `skills/cafleet/SKILL.md`, the relevant
      `reference/*` pages, and the `roles/*.md` spawn-prompt guidance. <!-- completed: -->
- [ ] Run `mise //cafleet:lint-overlay` to confirm the coding-agent overlay token
      set stays coherent after the renames. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-24 | Initial draft |
| 2026-06-24 | Reviewer-approved after 3 rounds; finalized (Status: Approved). |
| 2026-06-24 | Reverted the `message`→`msg` rename; the `message` group name is retained. |
