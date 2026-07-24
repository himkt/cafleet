# `cafleet member prompt` and Removal of `member exec` / `message cancel`

**Status**: Complete
**Progress**: 28/28 tasks complete
**Last Updated**: 2026-07-24

## Overview

Introduce a Director-only CLI subcommand `cafleet member prompt` that keystrokes text directly into a member's pane as a user turn, with a `--shell` flag that keystrokes `! <text>` instead — reproducing `cafleet member exec` exactly. `cafleet member exec` is removed entirely, and `cafleet message cancel` plus the `canceled` message status value are removed, with a data migration folding legacy `canceled` rows into `completed`.

## Success Criteria

- [x] `cafleet member prompt` dispatches both forms with the specified keystroke mechanics (plain: Esc-safeguarded user turn; `--shell`: un-escaped `! <text>`, byte-identical to former `member exec` delivery) on both multiplexer backends.
- [x] `cafleet member exec` and `cafleet message cancel` no longer parse (Click's default `No such command` error, exit 2).
- [x] The `canceled` status value is absent from source, tests, docs, skills, presets, and the admin WebUI; the message lifecycle is exactly `input_required` → `completed` via `message ack`.
- [x] Alembic migration `0004` folds legacy `status_state = 'canceled'` rows into `completed`; the chain-guard tests assert the four-revision chain with head `0004`.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //admin:lint` pass; a repo-wide `git grep` (tracked files only — gitignored trees like `site/` and `researches/` are excluded by construction) for `member exec`, `member_exec`, `send_bash_command`, `message cancel`, `cancel_message`, and `canceled` returns no hits outside `design-docs/` — for the `canceled` term only, additionally excluding the `0004` migration file (`cafleet/src/cafleet/db/alembic/versions/0004_*.py`) and `SPEC.md`'s migration-chain description, whose contract text necessarily carries the literal legacy value being folded.

---

## Background

`cafleet member exec` (`cli/member.py`) is the Director's shell-dispatch primitive: it keystrokes `! <command>` + `Enter` into a member pane via `Multiplexer.send_bash_command` (implemented in `tmux.py` and `herdr.py`, declared in `base.py`), deliberately omitting the leading-`Esc` permission-prompt safeguard that `member ping` and inline previews carry. There is no primitive for injecting a plain user turn — text that must arrive as a direct prompt (e.g. a slash command) cannot be delivered; a broker `message send` arrives as message content, not as a typed turn.

`cafleet message cancel` (`cli/message.py` → `broker.cancel_message`) lets a sender transition its own un-acked message from `input_required` to `canceled`. The status value leaks into the poll/ack machinery docs, SPEC.md, the skills, and the admin WebUI (`types.ts` status union, timeline and badge rendering), while carrying no workflow value — no skill or protocol ever instructs a member to cancel.

---

## Specification

### CLI: `cafleet member prompt`

Director-only by convention (no caller-auth mechanism, same as `member exec` today). Fleet-gated: `--fleet-id` required after the subcommand name.

```bash
cafleet member prompt --fleet-id <fleet-id> --member-id <member-id> [--shell] "TEXT"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID. |
| `--shell` | no | Boolean flag, default off. Dispatch `! TEXT` (shell form) instead of `TEXT` (plain form). |
| *(positional `TEXT`)* | yes | Single line of text; leading/trailing whitespace stripped before dispatch. Pipes, `&&`, `;`, `$(...)`, and backticks are forwarded opaquely. |
| `--json` | no | JSON output. |

**Validation** (both forms, mirroring `member exec` exactly, in this CLI-layer precedence): **first**, the original text containing `\n` or `\r` → `Error: text may not contain newlines.` (exit 2); **then**, empty after strip → `Error: text may not be empty.` (exit 2) — so a `"\n"`-only input raises the newline error, as `member exec` does today. Missing positional → Click's `Error: Missing argument 'TEXT'.` (exit 2). Target-resolution errors reuse the shared `member capture` / `member ping` shapes with action name `prompt` (e.g. `nothing to prompt` for pending placement). (The mux-layer `send_prompt` checks empty-first, matching today's `send_bash_command` — see § Multiplexer contract.)

**Keystroke behavior** — the `--shell` flag controls both the payload prefix and the Esc safeguard:

| Form | Keystroke sequence | Rationale | Follow-up |
|---|---|---|---|
| Plain (no `--shell`) | `Esc` → settle → literal `TEXT` → `Enter` | The trailing `Enter` submits a real user turn; the leading `Esc` (as in `member ping` / inline previews) keeps it from blindly confirming a pending permission prompt. | None — the submitted turn opens the member's turn directly. |
| `--shell` | literal `! TEXT` → `Enter` (no `Esc`) | Byte-identical to former `member exec` delivery; an `Esc` before `! <cmd>` would mis-fire (existing safeguard doctrine in `tmux.py`). | `cafleet member ping` required — the bang output only stages in the pane; the ping advances the member's turn to consume it. |

The flag performs no content inspection: plain-form `TEXT` beginning with `!` is delivered verbatim without the shell mechanics.

**Output.** Plain text: `Sent prompt '<text>' to member <name> (<pane_id>).`; with `--shell`: `Sent shell prompt '<text>' to member <name> (<pane_id>).`. JSON: `{member_id, pane_id, text, shell}`.

**Permission gates** — both forms sit under per-call confirmation; the whole subcommand is one gate:

| Backend | Gate |
|---|---|
| claude | `permissions.ask` (operator config convention); `docs/spec/cli-options.md` § *permissions.allow coverage* excludes `member prompt` (replacing the `member exec` exclusion). |
| codex | `presets/codex/cafleet.rules`: `prefix_rule(pattern = ["cafleet", "member", "prompt"], decision = "prompt", justification = "cafleet member prompt keystrokes arbitrary text or shell commands into a member pane")` replaces the `member exec` rule. |
| opencode | `presets/opencode/cafleet.md` allowlist prose: `cafleet` (except `cafleet member prompt`). |

### Plain-form usage guidance (docs + skills)

The plain form exists for text that only takes effect when it arrives as a **direct user turn** in the member's pane — slash commands, skill invocations, and other magic commands (e.g. `/goal`-style commands) that a broker message body cannot trigger, because `message send` delivers an inline preview the agent reads as content, not as a typed command. Broker messaging remains the canonical coordination channel; the plain form is not a substitute for `message send` and is not documented as a stall-recovery or urgent-redirect primitive.

### Multiplexer contract: `send_prompt` replaces `send_bash_command`

`Multiplexer.send_bash_command(*, target_pane_id, command)` is renamed and generalized in `base.py`, `tmux.py`, and `herdr.py`:

```python
def send_prompt(self, *, target_pane_id: str, text: str, shell: bool = False) -> None
```

- **Fail-fast validation** (each backend, mirroring today's `send_bash_command`): empty after strip → `send_prompt: text may not be empty`; original text with `\n`/`\r` → `send_prompt: text may not contain newlines` (backend-native error types `TmuxError` / `HerdrError`).
- **tmux**: payload = `f"! {stripped}"` when `shell` else `stripped`; delivered via `_send_literal_then_enter(esc_first=not shell)`. The Esc-safeguard doctrine comment in `_send_literal_then_enter` is updated to name `send_prompt`'s shell form as the deliberate omission site.
- **herdr**: shell form = `herdr pane run <pane> "! <stripped>"` (no esc, exactly today's `send_bash_command`); plain form = `_send_esc(pane)` then `herdr pane run <pane> <stripped>` (mirroring `send_poll_trigger`'s esc-then-run shape).
- `cli/member.py`'s `member prompt` calls `mux.send_prompt(target_pane_id=pane_id, text=text, shell=shell)`; a `MultiplexerError` maps to `Error: send failed: <exc>` (exit 1), as with `member exec` today.

### Routing protocol rewrite: `exec-routing.md` → `prompt-routing.md`

`skills/cafleet/reference/exec-routing.md` is renamed to `skills/cafleet/reference/prompt-routing.md` (hard-break rename; every cross-reference updated in the same change) and rewritten:

- The two-primitives table becomes `cafleet member prompt` (`permissions.ask` — operator-controlled body) and `cafleet member ping` (`permissions.allow` — fixed action).
- Member-side "reconsider, then route" flow is unchanged in substance; the Director dispatch sequence becomes:

  ```bash
  cafleet member prompt --fleet-id <fleet-id> --member-id <member-id> --shell "<command>"
  cafleet member ping --fleet-id <fleet-id> --member-id <member-id>
  cafleet message ack --fleet-id <fleet-id> --member-id <director-member-id> --message-id <message-id>
  ```

- Serialization rule restated as `prompt --shell → ping → ack → next`; never two concurrent prompt dispatches against the same pane.
- The rewritten page also documents the plain form's no-ping semantics (a real user turn) so the split is stated once, canonically.

### Removal: `cafleet message cancel` and the `canceled` status

- **Broker** (`broker/messaging.py`): delete `cancel_message` and drop its `broker/__init__.py` export. The module-docstring summary line drops the `/cancel` token (becoming `Message send/broadcast/poll/ack and inline-preview notification`), and the `poll` docstring drops its `canceled` mention. With `ack_message` as the sole remaining caller, `_transition_message_state` is inlined into `ack_message` — the generic 6-parameter shape reads as residue of the removed second caller — preserving the exact behavior and error strings (`Message {id} not found`, `Only the recipient can ACK a message`, `Cannot ACK message in state {state}`).
- **CLI** (`cli/message.py`): delete the `message cancel` command. The status domain is `input_required` | `completed`; the only transition is `input_required` → `completed` via `message ack` (recipient-only). `message ack` and its semantics are unchanged.
- **Admin WebUI** (`admin/src/`): `types.ts` status union becomes `"input_required" | "completed"`; `TimelineMessage.tsx` drops `isCanceled` and the canceled rendering branch; `MemberDetail.tsx` drops the `canceled` badge entry. `mise //admin:build` regenerates `cafleet/src/cafleet/webui/dist`.
- **Docs**: `docs/spec/cli-options.md` (subcommand table row, `message cancel` section, error-table rows, `--member-id` semantics list), `docs/spec/webui-api.md` (status values), `docs/spec/data-model.md`, `docs/api/broker.md`, `SPEC.md` (MessageStatus domain, transition table, `message cancel` CLI section, reimplementation checklist), plus any `canceled`/`cancel` mentions in `docs/concepts/` pages. Per the removal rule, no surface documents the removed value.
- **Skills**: `skills/cafleet/SKILL.md` (On-demand row text, `--member-id` semantics), `skills/cafleet/reference/cli.md` (`message cancel` section), `roles/*.md`, `reference/supervision.md`, and any `cafleet-design-doc` workflow/role mentions.

### Data migration `0004`: fold legacy `canceled` rows

The migration chain currently stands at three revisions (`0001` → `0002` → `0003`, head `0003`); `makemigration` mints the next sequential id, so this migration lands as `0004` with `down_revision = "0003"`. `messages.status_state` is an unconstrained string column — no schema change, a data-only migration:

- **Upgrade**: `op.execute("UPDATE messages SET status_state = 'completed' WHERE status_state = 'canceled'")`. `status_timestamp` is left untouched — it records when the row reached its terminal state, which remains true after the fold. Folding (rather than deleting) keeps every message row and its content — the data-preserving choice.
- **Downgrade**: no-op (`pass`). The fold is not invertible — which rows were previously `canceled` is unrecoverable; the migration docstring states this.
- **Workflow** (per `.claude/rules/database-migrations.md`): bring the DB to head with `cafleet setup --skip claude --skip codex --skip opencode`, then `mise //cafleet:makemigration "fold legacy canceled message status into completed"`. Autogenerate emits empty ops (no schema diff) — hand-edit in the `UPDATE` and the no-op `downgrade()`. The file lands as `0004_<slug>.py` with `down_revision = "0003"`.
- **Chain guards**: update `tests/db/test_alembic_smoke.py` — rename `test_three_revision_migration_chain_exists` to its four-revision counterpart asserting count 4 and the chain `0004` → `0003` → `0002` → `0001` → `None`, and rename `test_alembic_version_table_records_head_0003` to `test_alembic_version_table_records_head_0004` asserting `[("0004",)]`.
- **Stale rule correction**: `.claude/rules/database-migrations.md` still describes the chain guard as `test_single_initial_migration_revision_exists` asserting a single `0001` revision — stale by two revisions. Update its chain-guard description to the post-change state (the four-revision guard names above).

### Surfaces inventory

| Surface | Files | Action |
|---|---|---|
| CLI | `cafleet/src/cafleet/cli/member.py`, `cafleet/src/cafleet/cli/message.py` | Add `member prompt`; delete `member_exec`, `message_cancel`. |
| Multiplexer | `multiplexer/base.py`, `multiplexer/tmux.py`, `multiplexer/herdr.py` | `send_bash_command` → `send_prompt(text, shell=False)`; update the tmux Esc-doctrine comment. |
| Broker | `broker/messaging.py`, `broker/__init__.py` | Delete `cancel_message` + export; scrub docstrings. |
| Migration | `db/alembic/versions/0004_*.py`, `tests/db/test_alembic_smoke.py` | New data migration + chain-guard and head-version test updates. |
| Admin WebUI | `admin/src/types.ts`, `admin/src/components/TimelineMessage.tsx`, `admin/src/components/MemberDetail.tsx`, `cafleet/src/cafleet/webui/dist/` | Drop `canceled`; rebuild dist. |
| Presets | `presets/codex/cafleet.rules`, `presets/opencode/cafleet.md` | Gate `member prompt` instead of `member exec`. |
| Docs | `SPEC.md`, `docs/spec/cli-options.md`, `docs/spec/multiplexer-backends.md`, `docs/spec/webui-api.md`, `docs/spec/data-model.md`, `docs/spec/coding-agent-backends.md`, `docs/api/broker.md`, `docs/concepts/coding-agents.md`, `docs/concepts/member-lifecycle.md`, `docs/concepts/overview.md`, `docs/concepts/monitoring.md`, `docs/quickstart.md` | Replace exec with prompt; remove cancel/`canceled`. |
| Skills | `skills/cafleet/reference/exec-routing.md` → `prompt-routing.md`, `skills/cafleet/SKILL.md`, `reference/cli.md`, `reference/director.md`, `reference/supervision.md`, `reference/recovery.md`, `roles/member.md`, `roles/director.md`, `roles/monitor.md`, `skills/cafleet-design-doc/create/roles/director.md`, `skills/cafleet-design-doc/execute/roles/director.md`, `skills/cafleet-design-doc/execute/execute.md` | Rename + rewrite routing page; update every mention and cross-reference. |
| Rules + repo-local skills (`.claude/`) | `.claude/rules/bash-tool.md`, `.claude/rules/database-migrations.md`, `.claude/skills/clean-docs/residue/reference/rubric.md` | Rewrite bash-tool.md's Director-side dispatch sections around `member prompt --shell`; correct database-migrations.md's stale chain-guard description; replace rubric.md's "a canceled message no longer appears" illustrative example with one not referencing the removed status (e.g. "a deleted member no longer appears"). |
| Tests | `tests/cli/test_member_exec.py` → `tests/cli/test_member_prompt.py`, `tests/multiplexer/test_tmux.py`, `tests/multiplexer/test_herdr.py`, `tests/broker/test_messaging.py`, `tests/broker/test_typed_columns.py`, `tests/broker/test_inline_preview.py`, `tests/broker/test_asset_installs.py`, `tests/cli/test_message.py`, `tests/cli/test_help_budget.py` | Rewrite/delete per behavior above; no removal-sentinel tests beyond the absence regression guards. |

`README.md` is expected unaffected (thin surface) — verified, not edited, in Step 1.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (docs/ + SPEC.md first, per documentation-maintenance.md)

- [x] Update `docs/spec/cli-options.md`: replace the `member exec` row/section/error rows with `member prompt` (flags, both forms, error strings, permissions.allow-coverage exclusion), and remove the `message cancel` row/section and `--member-id` cancel mention <!-- completed: 2026-07-24T09:50 -->
- [x] Update `docs/spec/multiplexer-backends.md`: `send_prompt` contract, Esc-safeguard section listing the plain form as Esc-carrying and the shell form as the deliberate omission <!-- completed: 2026-07-24T09:52 -->
- [x] Update `docs/spec/webui-api.md` and `docs/spec/data-model.md`: status domain `input_required` | `completed` <!-- completed: 2026-07-24T09:53 -->
- [x] Update `docs/api/broker.md`, `docs/concepts/` pages (`coding-agents.md`, `member-lifecycle.md`, `overview.md`, `monitoring.md`), `docs/quickstart.md`, `docs/spec/coding-agent-backends.md` <!-- completed: 2026-07-24T09:57 -->
- [x] Update `SPEC.md`: `member prompt` section, `send_prompt` multiplexer entries and Esc table, MessageStatus domain and transitions, preset excerpts, reimplementation checklist <!-- completed: 2026-07-24T10:08 -->
- [x] Verify `README.md` thin surface is unaffected <!-- completed: 2026-07-24T10:08 -->

### Step 2: Skills, rules, presets

- [x] Rename `skills/cafleet/reference/exec-routing.md` → `prompt-routing.md`; rewrite per the routing-protocol spec (two-primitives table, `prompt --shell → ping → ack` serialization, plain-form no-ping semantics) <!-- completed: 2026-07-24T10:05 -->
- [x] Update every cross-reference to `exec-routing.md` and every `member exec` mention across `skills/cafleet/` (SKILL.md, `reference/cli.md`, `reference/director.md`, `reference/supervision.md`, `reference/recovery.md`, `roles/member.md`, `roles/director.md`, `roles/monitor.md`) <!-- completed: 2026-07-24T10:05 -->
- [x] Remove `message cancel` / `canceled` mentions from `skills/cafleet/` (SKILL.md On-demand row, `--member-id` semantics, `reference/cli.md`) and `skills/cafleet-design-doc/` role/workflow files <!-- completed: 2026-07-24T10:05 -->
- [x] Add plain-form usage guidance (direct-user-turn / slash-command use case) to the rewritten routing page and the Director-facing skill pages <!-- completed: 2026-07-24T10:05 -->
- [x] Rewrite `.claude/rules/bash-tool.md` Director-side sections around `member prompt --shell` <!-- completed: 2026-07-24T10:09 -->
- [x] Replace the "a canceled message no longer appears" illustrative example in `.claude/skills/clean-docs/residue/reference/rubric.md` with one not referencing the removed status <!-- completed: 2026-07-24T10:09 -->
- [x] Update `presets/codex/cafleet.rules` (`prefix_rule` for `member prompt`) and `presets/opencode/cafleet.md` (allowlist exception) <!-- completed: 2026-07-24T10:05 -->

### Step 3: CLI + multiplexer code

- [x] Replace `send_bash_command` with `send_prompt(*, target_pane_id, text, shell=False)` in `multiplexer/base.py`, `tmux.py` (`esc_first=not shell`), `herdr.py` (esc-then-run plain form); update the tmux Esc-doctrine comment <!-- completed: 2026-07-24T10:15 -->
- [x] Add `member prompt` to `cli/member.py` (flags, validation, output shapes per spec); delete `member_exec` <!-- completed: 2026-07-24T10:15 -->

### Step 4: Cancel removal code

- [x] Delete `cancel_message` from `broker/messaging.py` and its `broker/__init__.py` export; inline `_transition_message_state` into `ack_message` preserving exact error strings; drop `/cancel` from the module-docstring summary and `canceled` from the poll docstring <!-- completed: 2026-07-24T10:20 -->
- [x] Delete the `message cancel` command from `cli/message.py` <!-- completed: 2026-07-24T10:20 -->

### Step 5: Admin WebUI

- [x] Update `admin/src/types.ts` status union, `TimelineMessage.tsx`, `MemberDetail.tsx`; run `mise //admin:lint` <!-- completed: 2026-07-24T10:24 -->
- [x] Run `mise //admin:build` to regenerate `cafleet/src/cafleet/webui/dist` <!-- completed: 2026-07-24T10:24 -->

### Step 6: Data migration

- [x] Bring the DB to head (`cafleet setup --skip claude --skip codex --skip opencode`), run `mise //cafleet:makemigration "fold legacy canceled message status into completed"` (lands as `0004_<slug>.py`, `down_revision = "0003"`), hand-edit the `UPDATE` upgrade and no-op `downgrade()` with the irreversibility docstring <!-- completed: 2026-07-24T10:22 -->
- [x] Update `tests/db/test_alembic_smoke.py`: rename `test_three_revision_migration_chain_exists` to the four-revision counterpart (count 4, chain `0004` → `0003` → `0002` → `0001` → `None`) and `test_alembic_version_table_records_head_0003` to `…_head_0004` asserting `[("0004",)]` <!-- completed: 2026-07-24T10:22 -->
- [x] Correct `.claude/rules/database-migrations.md`'s stale chain-guard description to the four-revision guard names <!-- completed: 2026-07-24T10:22 -->

### Step 7: Tests + verification

- [x] Rewrite `tests/cli/test_member_exec.py` as `tests/cli/test_member_prompt.py`: both forms' dispatch, validation errors, output shapes, target-resolution errors, absence guard for `member exec` (Click `No such command` error) <!-- completed: 2026-07-24T10:30 -->
- [x] Update `tests/multiplexer/test_tmux.py` / `test_herdr.py`: `send_prompt` both forms, Esc-first assertions (plain: esc; shell: no esc), fail-fast validation <!-- completed: 2026-07-24T10:30 -->
- [x] Remove cancel tests from `tests/broker/test_messaging.py` and `tests/cli/test_message.py`; add the `message cancel` absence guard <!-- completed: 2026-07-24T10:30 -->
- [x] Update `tests/broker/test_typed_columns.py`, `tests/broker/test_inline_preview.py`, `tests/broker/test_asset_installs.py` (codex rules content), `tests/cli/test_help_budget.py` (subcommand lists) <!-- completed: 2026-07-24T10:30 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` <!-- completed: 2026-07-24T10:30 -->
- [x] Repo-wide `git grep` sweep (tracked files only) for `member exec`, `member_exec`, `send_bash_command`, `message cancel`, `cancel_message`, `canceled` — no hits outside `design-docs/`; for the `canceled` term only, additionally exclude `cafleet/src/cafleet/db/alembic/versions/0004_*.py` and `SPEC.md`'s migration-chain description (their contract text necessarily carries the literal legacy value being folded) <!-- completed: 2026-07-24T10:28 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-23 | Initial draft |
| 2026-07-24 | Review round 1: CLI validation precedence pinned newline-first; migration renumbered `0002` → `0004` (four-revision chain guards, head-version test); `_transition_message_state` inlined into `ack_message`; module-docstring `/cancel` drop made explicit; sweep switched to `git grep`; clean-docs rubric example and stale `database-migrations.md` chain-guard description added to scope |
| 2026-07-24 | User approved — Status: Approved |
| 2026-07-24 | Director arbitration (Programmer escalation): the `canceled` sweep term in Step 7 and Success Criterion 5 now excludes the `0004` migration file and `SPEC.md`'s migration-chain description — their contract text necessarily carries the literal legacy value being folded |
| 2026-07-24 | Implementation complete: 28/28 tasks, all Success Criteria verified (Phase D E2E + Reviewer round-1 approval), PR #220 opened — Status: Complete |
