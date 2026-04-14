# Migrate session-id and agent-id from env vars / shell expansion to CLI flags

**Status**: Complete
**Progress**: 33/33 tasks complete
**Last Updated**: 2026-04-14

## Overview

Replace the `CAFLEET_SESSION_ID` and `CAFLEET_AGENT_ID` environment variables with explicit `--session-id` (global) and `--agent-id` (per-subcommand, already exists) CLI flags, then rewrite every recommendation that uses `export VAR=...` or `$VAR` shell expansion to pass literal UUIDs on the command line. This unblocks Claude Code's `permissions.allow` literal-string matching, eliminating the per-invocation permission prompt that interrupts agent work. The change also cleans up adjacent drift: the `cafleet env` subcommand (whose sole purpose was dumping env vars) is deleted, residual `CAFLEET_URL` references in `mise.toml`, `docs/spec/cli-options.md`, and `cafleet/tests/test_tmux.py` are removed, and the `## Plugin Skills` section in `CLAUDE.md` pointing at the non-existent `plugins/` directory is deleted.

## Success Criteria

- [x] `cafleet --session-id <uuid> ...` global flag is the only supported way to pass session-id; `CAFLEET_SESSION_ID` env var is removed from the codebase
- [x] `CAFLEET_AGENT_ID` env var is removed; the spawned member's coding-agent prompt receives a literal `agent_id` UUID instead of `$CAFLEET_AGENT_ID`
- [x] `cafleet env` subcommand is removed (its purpose disappears with env vars)
- [x] `broker._try_notify_recipient` injects `cafleet --session-id <uuid> poll --agent-id <uuid>` into the recipient pane (currently `cafleet poll --agent-id <uuid>`)
- [x] `cafleet member create`'s tmux `-e` env injection no longer carries `CAFLEET_SESSION_ID` / `CAFLEET_AGENT_ID`; only `CAFLEET_DATABASE_URL` remains forwarded when set
- [x] `coding_agent.py` prompt templates use literal substitution placeholders (`{session_id}`, `{agent_id}`) instead of shell vars `$CAFLEET_AGENT_ID`
- [x] `README.md`, `ARCHITECTURE.md`, `docs/spec/cli-options.md`, all `.claude/skills/*/SKILL.md`, all `.claude/skills/cafleet-design-doc-*/roles/*.md` show `cafleet --session-id <uuid> <subcmd> --agent-id <uuid> ...` (with `--session-id` as a global flag before the subcommand and `--agent-id` as a per-subcommand option after the subcommand name) and contain zero `export CAFLEET_*` and zero `$CAFLEET_*` / `$DIRECTOR_ID` / `$MY_ID` / `$MEMBER_ID` / `$PROGRAMMER_ID` / `$TESTER_ID` / `$VERIFIER_ID` / `$DRAFTER_ID` / `$REVIEWER_ID` references
- [x] `CAFLEET_URL` is removed from `mise.toml`, `docs/spec/cli-options.md`, and `cafleet/tests/test_tmux.py` (dead reference; no longer read anywhere)
- [x] **Residual-grep zero-hits**: `grep -rn "CAFLEET_SESSION_ID\|CAFLEET_AGENT_ID\|\$DIRECTOR_ID\|\$MY_ID\|\$MEMBER_ID\|\$PROGRAMMER_ID\|\$TESTER_ID\|\$VERIFIER_ID\|\$DRAFTER_ID\|\$REVIEWER_ID\|export CAFLEET_" README.md ARCHITECTURE.md docs/ .claude/skills/ cafleet/src/ cafleet/tests/ CLAUDE.md .claude/CLAUDE.md` returns zero hits **in production docs and source** (remaining hits are load-bearing negative assertions in `cafleet/tests/test_cli_session_flag.py`, `test_tmux.py`, `test_coding_agent.py` that prove the migration worked, plus a "Removed Surface" historical line in `docs/spec/cli-options.md:52` explaining what was removed — all legitimate)
- [x] `CLAUDE.md` (root) no longer contains the `## Plugin Skills` section referencing non-existent `plugins/cafleet/skills/...` paths
- [x] `mise //cafleet:test`, `mise //:lint`, `mise //:format`, `mise //:typecheck` all pass after the migration
- [x] Manual smoke test: a fresh shell with `CAFLEET_SESSION_ID` unset can run `cafleet db init`, `cafleet session create --label test`, then `cafleet --session-id <uuid> register --name A --description a` followed by `cafleet --session-id <uuid> --json poll --agent-id <returned-id>` end-to-end with no env vars touched

---

## Background

### Why the env-var pattern broke

Claude Code matches Bash invocations against `permissions.allow` patterns as **literal command strings** (see `.claude/rules/bash-command.md`). Two patterns trip the matcher and force a per-invocation permission prompt that breaks autonomous agent loops:

| Pattern | Reason it breaks allow-matching |
|---|---|
| `export CAFLEET_SESSION_ID=...` | Sets shell state — there is no allow-list entry for "set env var", and subsequent commands depend on opaque shell state the matcher cannot inspect |
| `cafleet send --agent-id $DIRECTOR_ID --to $MEMBER_ID --text ...` | The expanded form differs every run; literal `$DIRECTOR_ID` does not match a literal `<uuid>` allow pattern |

A literal invocation like `cafleet --session-id 550e8400-... poll --agent-id 7ba91234-...` matches a `permissions.allow` entry of the same shape exactly once and stays matched across the session. (`--session-id` is a global flag before the subcommand; `--agent-id` is a per-subcommand option after the subcommand name.)

### Current state (Status: Complete designs that established the env-var convention)

| Design doc | Established |
|---|---|
| `0000010-sqlite-store-migration` | `CAFLEET_DATABASE_URL` env var (kept — not in scope) |
| `0000015-remove-auth0-local-session-model` | `CAFLEET_SESSION_ID` as the sole namespace input (now being replaced) |
| `0000017-client-registry-integration` | `CAFLEET_AGENT_ID` env injection on member panes (now being replaced) |
| `0000020-tmux-push-notification` | Broker injects `cafleet poll --agent-id <id>` into recipient pane (now needs `--session-id`) |
| `0000021-direct-sqlite-cli` | Dropped `CAFLEET_URL`; left `CAFLEET_SESSION_ID` |

`CAFLEET_API_KEY` was already removed in 0000015. `CAFLEET_URL` is a dead reference that still appears in three places and is removed here as cleanup:

| Location | Purpose at write time | Current status |
|---|---|---|
| `mise.toml:25` | legacy env default for HTTP broker | dead — no code reads it |
| `docs/spec/cli-options.md:12` | Option Source Matrix entry | dead — documented-only |
| `cafleet/tests/test_tmux.py` | mocked env in test fixtures | dead — fixture-only |

---

## Specification

### CLI surface

```
cafleet [--json] [--session-id <uuid>] <subcommand> [--agent-id <uuid>] [opts...]
```

| Flag | Scope | Required | Notes |
|---|---|---|---|
| `--json` | global | no | unchanged |
| `--session-id <uuid>` | global | yes for client + member subcommands; no for `db init`, `session *` | NEW; replaces `CAFLEET_SESSION_ID` env var |
| `--agent-id <uuid>` | per-subcommand | yes for `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member *` | unchanged; `register` does not require it |

`--session-id` is **global** (placed before the subcommand) so a single `permissions.allow` pattern of the form `cafleet --session-id <literal-uuid> *` matches every subcommand for that session.

### Validation

| Subcommand class | `--session-id` requirement | Error on missing |
|---|---|---|
| `db init`, `db *` | not required | n/a |
| `session create`, `session list`, `session show`, `session delete` | not required | n/a |
| `register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member *` | required | `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.` exit 1 |

**Provided but not required**: when `--session-id` is supplied to a subcommand that does not use it (e.g. `cafleet --session-id <uuid> db init`), it is **silently accepted** and ignored. `_require_session_id()` is the only gate; no "unused option" rejection is implemented. This intentionally keeps a single `cafleet --session-id <literal-uuid> *` allow pattern usable for every subcommand without needing per-command exceptions.

`_require_session_id()` keeps the same name but now reads `ctx.obj["session_id"]` populated from the click flag, not from `os.environ`.

### Removed surface

| Item | Reason |
|---|---|
| `cafleet env` subcommand | Sole purpose was printing env vars; with no env-driven session-id it loses meaning |
| `os.environ.get("CAFLEET_SESSION_ID")` lookup in `cli()` group | Replaced by the click flag |
| `CAFLEET_SESSION_ID` / `CAFLEET_AGENT_ID` keys in `member create`'s `fwd_env` dict (`cli.py:567-573`) | Replaced by literal substitution in the spawn prompt |
| `$CAFLEET_AGENT_ID` shell-expansion placeholders in `coding_agent.py` prompt templates | Replaced by `{session_id}` / `{agent_id}` Python `.format()` placeholders, resolved at spawn time |

### Member-spawn handoff redesign

`cafleet member create` currently relies on `tmux split-window -e CAFLEET_SESSION_ID=... -e CAFLEET_AGENT_ID=...` so the spawned coding agent inherits ids via env. The new flow:

1. `member create` resolves `session_id` and the newly-allocated `new_agent_id` as Python strings.
2. The prompt template (in `coding_agent.py`) is expanded via `str.format(session_id=..., agent_id=..., director_name=..., director_agent_id=...)` so the literal UUIDs are baked into the prompt text.
3. tmux `-e` only forwards `CAFLEET_DATABASE_URL` (when set), since SQLite path is the one env input we keep.
4. The spawned `claude` (or `codex`) sees its agent_id and session_id verbatim in the initial prompt text, and from then on every `cafleet ...` it issues uses literal `--session-id <uuid> --agent-id <uuid>` flags.

### Push-notification command shape

`tmux.send_poll_trigger` currently injects:

```
cafleet poll --agent-id <recipient-uuid>
```

into the recipient's pane. After this change the broker passes both ids through:

```
cafleet --session-id <session-uuid> poll --agent-id <recipient-uuid>
```

`--session-id` is a global flag (placed **before** the subcommand); `--agent-id` is a per-subcommand option (placed **after** the subcommand name).

`broker._try_notify_recipient` already has the session in scope (it's the message's session); thread it down through `send_poll_trigger(target_pane_id=..., session_id=..., agent_id=...)`.

### Documentation rewrite rules

Apply globally to README, ARCHITECTURE, docs/spec/, and every SKILL.md / roles/*.md:

| Old form | New form |
|---|---|
| `export CAFLEET_SESSION_ID=...` | (deleted entirely; replaced by inline literal flag) |
| `export CAFLEET_AGENT_ID=...` | (deleted entirely) |
| `$CAFLEET_SESSION_ID` | literal `<session-id>` placeholder, e.g. `550e8400-e29b-41d4-a716-446655440000` |
| `$CAFLEET_AGENT_ID` | literal `<agent-id>` placeholder |
| `cafleet poll --agent-id $MY_ID` | `cafleet --session-id <session-id> poll --agent-id <my-agent-id>` |
| `cafleet member create --agent-id $DIRECTOR_ID ...` | `cafleet --session-id <session-id> member create --agent-id <director-agent-id> ...` |
| `cafleet send --agent-id $CAFLEET_AGENT_ID --to $DIRECTOR_ID ...` | `cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> ...` |
| *(concrete final form, with example UUIDs)* | `cafleet --session-id 550e8400-e29b-41d4-a716-446655440000 poll --agent-id 7ba91234-5678-90ab-cdef-112233445566` |

**Placeholder convention** (applied uniformly across README, ARCHITECTURE, docs/spec, and every SKILL.md / roles/*.md):

- Use angle-bracket tokens for substitution points: `<session-id>`, `<my-agent-id>`, `<director-agent-id>`, `<member-agent-id>`, `<target-agent-id>`, `<task-id>`.
- Do **not** use shell-variable names (`$DIRECTOR_ID`, `$MY_ID`, `$CAFLEET_AGENT_ID`, etc.) anywhere — those are the exact form that breaks `permissions.allow` matching.
- A realistic UUID (e.g. `550e8400-e29b-41d4-a716-446655440000`) may appear once in Quick Start / Example sections to show the final literal form, but examples in command reference sections should use the angle-bracket tokens to keep them reusable.

Documentation should explain the convention with one sentence: "All `cafleet` invocations require `--session-id` and (for client subcommands) `--agent-id`. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet register` — do not use shell variables, since `permissions.allow` patterns match command strings literally."

### Tests

| Test file | Required change |
|---|---|
| `cafleet/tests/test_tmux.py` | Drop `CAFLEET_SESSION_ID` / `CAFLEET_URL` from the simulated env; assert `-e` flags carry only `CAFLEET_DATABASE_URL` when set; update prompt-template assertion to expect literal UUID instead of `$CAFLEET_AGENT_ID` |
| `cafleet/tests/test_session_cli.py`, `test_broker_messaging.py`, `test_broker_registry.py`, `test_broker_webui.py` | Where they invoke `cafleet` via `CliRunner`, switch from `env={"CAFLEET_SESSION_ID": ...}` to `cafleet ["--session-id", session_id, ...]` |
| New: `cafleet/tests/test_cli_session_flag.py` | Exercise: missing flag on client subcommand exits 1 with the new error message; flag value flows into `broker.*` calls correctly; `db init` / `session create` work without the flag |

### CLAUDE.md cleanup

The repository-root and `.claude/` `CLAUDE.md` both list `## Plugin Skills` referring to `plugins/cafleet/skills/...`. The `plugins/` directory does not exist in this repo. Delete the `## Plugin Skills` section from both files in this design doc — it is misleading drift left over from a future-plugin discussion.

---

## Implementation

> Documentation must be updated **before** any code change (per `.claude/rules/design-doc-numbering.md`).
> Task format: `- [x] Done task <!-- completed: 2026-04-14T14:30 -->`

### Step 1: Documentation — Top-level docs

- [x] Update `ARCHITECTURE.md` Option Source Matrix (lines 206-217): replace `Session ID — CAFLEET_SESSION_ID env var` with `Session ID — --session-id global flag`. Add a one-line note explaining the literal-flag rationale (permission allow-list). Remove the "Session ID uses an environment variable for convenience in tmux multi-pane workflows" sentence. <!-- completed: 2026-04-14T11:15 -->
- [x] Update `README.md`: delete the `### Set Session` block (lines 75-79); update Quick Start so each `cafleet` example shows `--session-id <session-id>` inline; update the env-var table (lines 115-118) to drop `CAFLEET_SESSION_ID`; update the agent-commands table to note that all client commands require `--session-id`; remove the `cafleet env` row. <!-- completed: 2026-04-14T11:15 -->
- [x] Update `docs/spec/cli-options.md`: rewrite the Option Source Matrix (lines 9-14); delete the "Environment Variable Setup" section (lines 16-23); add a "Global Options" section documenting `--session-id`; update the error-message table (line 141) with the new error string; remove the `CAFLEET_URL` row. <!-- completed: 2026-04-14T11:15 -->
- [x] Remove the dead `CAFLEET_URL = "http://localhost:8000"` entry from `mise.toml:25` (the entire `[env]` section if it becomes empty). <!-- completed: 2026-04-14T11:15 -->
- [x] Run `Skill(update-readme)` to keep README in sync with the rewritten ARCHITECTURE.md and docs/spec/cli-options.md. <!-- completed: 2026-04-14T11:15 -->

### Step 2: Documentation — Skill files

- [x] Update `.claude/skills/cafleet/SKILL.md`: rewrite "Environment Variables" (lines 22-28) to "Required Flags"; delete the "Env" subsection (lines 47-55); rewrite every `cafleet ...` example in the Command Reference, Multi-Session Coordination, and Typical Workflow sections to use literal `--session-id <session-id>` and `--agent-id <agent-id>` flags; remove the `export CAFLEET_SESSION_ID=...` lines; replace every `$DIRECTOR_ID`, `$MY_ID`, `$MEMBER_ID` reference with `<director-agent-id>`, `<my-agent-id>`, `<member-agent-id>` placeholders alongside an instruction to substitute the literal UUID returned by `cafleet register`. <!-- completed: 2026-04-14T12:30 -->
- [x] Update `.claude/skills/cafleet-monitoring/SKILL.md`: replace `$DIRECTOR_ID` and `$MEMBER_ID` placeholders in every command example with `<director-agent-id>` / `<member-agent-id>`; prepend `--session-id <session-id>` to every `cafleet` invocation; rewrite the `/loop` Prompt Template at the bottom to instruct the Director to substitute the literal session UUID and director UUID into every command, not to rely on shell vars. <!-- completed: 2026-04-14T12:30 -->
- [x] Update `.claude/skills/cafleet-design-doc-create/SKILL.md`: rewrite Section 1a (lines 80-89) — remove `export CAFLEET_SESSION_ID=...`; the session create step now stores the printed UUID and passes it as `--session-id <uuid>` on every subsequent command; rewrite all `cafleet --json register`, `cafleet --json member create`, `cafleet send`, `cafleet poll`, `cafleet member delete`, `cafleet deregister` examples in Steps 1b–6 to use the new flag form with literal placeholders. <!-- completed: 2026-04-14T12:30 -->
- [x] Update `.claude/skills/cafleet-design-doc-execute/SKILL.md`: same rewrite as the create skill (Section 3a lines 169-178 + every command example through Step 6). <!-- completed: 2026-04-14T12:30 -->
- [x] Update `.claude/skills/cafleet-design-doc-create/roles/director.md`, `roles/drafter.md`, `roles/reviewer.md`: rewrite every `cafleet send --agent-id $DIRECTOR_ID --to $DRAFTER_ID ...` example to literal-flag form. <!-- completed: 2026-04-14T12:30 -->
- [x] Update `.claude/skills/cafleet-design-doc-execute/roles/director.md`, `roles/programmer.md`, `roles/tester.md`, `roles/verifier.md`: same rewrite. <!-- completed: 2026-04-14T12:30 -->

### Step 3: Documentation — CLAUDE.md cleanup

- [x] Delete the `## Plugin Skills` section from `/home/himkt/work/himkt/cafleet/CLAUDE.md` (the one referencing `/cafleet:cafleet`, `/cafleet:cafleet-monitoring`, `/cafleet:cafleet-design-doc`, `/cafleet:cafleet-design-doc-create`, `/cafleet:cafleet-design-doc-execute`). <!-- completed: 2026-04-14T12:45 -->
- [x] Verify `.claude/CLAUDE.md` does not also need a similar deletion (it only lists `## Project Skills`, not `## Plugin Skills` — confirm with Grep before editing). <!-- completed: 2026-04-14T12:45 -->

### Step 4: Code — CLI flag implementation

- [x] Modify `cafleet/src/cafleet/cli.py` `cli()` group: add `@click.option("--session-id", default=None, help="Session ID (UUID); required for client subcommands")`; populate `ctx.obj["session_id"]` from the flag value; remove the `os.environ.get("CAFLEET_SESSION_ID")` line. <!-- completed: 2026-04-14T13:30 -->
- [x] Update `_require_session_id()` error message to: `"Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id."`. <!-- completed: 2026-04-14T13:30 -->
- [x] Delete the `cafleet env` subcommand definition (`cli.py:74-81`). Update any tests that exercised it. <!-- completed: 2026-04-14T13:30 -->
- [x] Modify `cafleet/src/cafleet/cli.py` `member create` (around `cli.py:567-573`): drop `CAFLEET_SESSION_ID` and `CAFLEET_AGENT_ID` from `fwd_env`; keep only `CAFLEET_DATABASE_URL` forwarding. <!-- completed: 2026-04-14T13:30 -->
- [x] Modify `cafleet/src/cafleet/coding_agent.py`: change `default_prompt_template` for both `CLAUDE` and `CODEX` to use `{session_id}` / `{agent_id}` placeholders instead of `$CAFLEET_AGENT_ID`; rewrite `cafleet poll --agent-id $CAFLEET_AGENT_ID` etc. to `cafleet --session-id {session_id} poll --agent-id {agent_id}` etc. (`--session-id` global before subcommand; `--agent-id` per-subcommand after subcommand name) <!-- completed: 2026-04-14T13:30 -->
- [x] Modify `_resolve_prompt()` in `cli.py` (around `cli.py:478-492`) to also pass `session_id=session_id, agent_id=new_agent_id` to `default_prompt_template.format(...)`. <!-- completed: 2026-04-14T13:30 -->

### Step 5: Code — Push-notification rewiring

- [x] Modify `cafleet/src/cafleet/tmux.py` `send_poll_trigger`: change signature to `(*, target_pane_id, session_id, agent_id)`; emit `f"cafleet --session-id {session_id} poll --agent-id {agent_id}"`. <!-- completed: 2026-04-14T13:30 -->
- [x] Modify `cafleet/src/cafleet/broker.py` `_try_notify_recipient`: accept `session_id` and pass it through to `send_poll_trigger`; update both call sites in `send_message` and `broadcast_message` to pass `session_id=session_id` (already in scope). <!-- completed: 2026-04-14T13:30 -->

### Step 6: Tests

- [x] Update `cafleet/tests/test_tmux.py`: drop `CAFLEET_SESSION_ID` / `CAFLEET_URL` from the env dicts; add session-id parameter to `send_poll_trigger` calls and assert the emitted command string contains `--session-id <uuid> poll --agent-id <uuid>`; assert tmux `-e` env list no longer carries the session/agent vars. <!-- completed: 2026-04-14T13:30 -->
- [x] Update `cafleet/tests/test_coding_agent.py`: flip `test_prompt_template_contains_agent_id_placeholder` (CLAUDE at line 209-213, CODEX at line 253-257) from asserting `"$CAFLEET_AGENT_ID" in template` to asserting `"{agent_id}" in template` and `"{session_id}" in template`; update `test_prompt_template_has_format_placeholders` to call `.format(...)` with the new `session_id=` and `agent_id=` keyword arguments; update the fixture string at line 100 (`"…Use $CAFLEET_AGENT_ID."`) to reflect the new convention or remove the example reference if it is decorative. <!-- completed: 2026-04-14T13:30 -->
- [x] Update CLI-runner-based tests (`test_session_cli.py`, `test_broker_messaging.py`, `test_broker_registry.py`, `test_broker_webui.py`) to invoke `cafleet` with `["--session-id", session_id, ...]` instead of injecting the env var. Where `CliRunner.invoke(env=...)` was used solely for `CAFLEET_SESSION_ID`, drop the env arg. <!-- completed: 2026-04-14T13:30 -->
- [x] Add `cafleet/tests/test_cli_session_flag.py` covering: (a) missing `--session-id` on `register` exits 1 with the new error string; (b) `--session-id <uuid>` flows into `broker.register_agent`; (c) `db init` and `session create` succeed without the flag; (d) `cafleet env` no longer exists (`Result.exit_code != 0` and stderr contains `No such command`); (e) `--session-id` supplied to `db init` is silently accepted (validates the "Provided but not required" rule). <!-- completed: 2026-04-14T13:30 -->
- [x] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: 2026-04-14T13:30 -->

### Step 7: Quality gates

- [x] Run `mise //:lint` — must pass. <!-- completed: 2026-04-14T14:15 -->
- [x] Run `mise //:format` — must pass. <!-- completed: 2026-04-14T14:15 -->
- [x] Run `mise //:typecheck` — must pass. <!-- completed: 2026-04-14T14:15 -->
- [x] Grep for residual `CAFLEET_SESSION_ID`, `CAFLEET_AGENT_ID`, `$DIRECTOR_ID`, `$MY_ID`, `$MEMBER_ID`, `$PROGRAMMER_ID`, `$TESTER_ID`, `$VERIFIER_ID`, `$DRAFTER_ID`, `$REVIEWER_ID`, `export CAFLEET_` across `README.md`, `ARCHITECTURE.md`, `docs/`, `.claude/skills/`, `cafleet/src/cafleet/`, `cafleet/tests/` — must return zero hits (excluding this design doc itself and `design-docs/0000015-*`, `0000017-*`, `0000020-*`, `0000021-*`, `0000022-*` which are historical records). <!-- completed: 2026-04-14T14:15 (production code/docs zero hits; test-file hits are load-bearing negative assertions) --> 
- [x] Manual smoke (Verifier-style): in a fresh shell with `unset CAFLEET_SESSION_ID CAFLEET_AGENT_ID`, run `cafleet db init`, `cafleet session create --label smoke`, capture the printed UUID, then `cafleet --session-id <uuid> register --name A --description a` and `cafleet --session-id <uuid> --json poll --agent-id <returned-id>` — verify both succeed and that no env var was needed. <!-- completed: 2026-04-14T14:15 -->

### Step 8: Finalize

- [x] Update Status to Complete and refresh Last Updated. <!-- completed: 2026-04-14T14:20 -->
- [x] Add a Changelog entry. <!-- completed: 2026-04-14T14:20 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-14 | Initial draft |
| 2026-04-14 | Reviewer revisions: added `mise.toml` and `test_coding_agent.py` to inventory; corrected `CAFLEET_URL` location list; added "provided but not required" silent-accept rule; added placeholder convention; expanded Overview to cover drift cleanup; added concrete-UUID example to rewrite table; promoted residual-grep to Success Criteria |
| 2026-04-14 | User approved — Status set to Approved |
| 2026-04-14 | Implementation complete: 267/267 tests pass, lint/format/typecheck green, residual-grep clean (production code/docs zero hits), smoke test verified end-to-end. Status: Complete. |
| 2026-04-14 | Copilot review fix: corrected `--agent-id` placement throughout implementation and documentation — `--session-id` is a global flag (before subcommand) but `--agent-id` is a per-subcommand option (after subcommand name). Updated `tmux.py`, `broker.py`, `coding_agent.py`, README, ARCHITECTURE, design-doc, and every SKILL.md / roles/*.md file accordingly. Matching tests updated. |
