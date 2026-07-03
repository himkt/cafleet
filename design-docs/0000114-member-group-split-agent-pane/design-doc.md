# Split the `member` CLI group into `agent` + `pane`

**Status**: Aborted
**Progress**: 0/27 tasks complete
**Last Updated**: 2026-07-03

## Overview

Migrate the CAFleet CLI — and its docs, `SPEC.md`, `README.md`, and skills — from the `cafleet member *` command group to the `agent` (`spawn` / `deregister` / `list`) and `pane` (`capture` / `exec` / `wake`) surface. This builds directly on design 0000113, which first reconciles the **entire** corpus onto one consistent `member` / `--member-id` surface (removing the earlier `agent`/`pane` documentation drift and the env-var-injection fiction). From that clean baseline, this design performs the structural split to `agent` + `pane` across code and documentation **together**, so the two never diverge again. Runtime behavior is unchanged — the same broker calls, tmux helpers, and `str.format` spawn-prompt substitution are preserved; only command names, module layout, and the drift-guard direction change.

## Success Criteria

- [ ] The `member` command group no longer exists; `cafleet member …` exits with Click's "No such command 'member'".
- [ ] `cafleet agent` exposes `register | list | show | deregister | spawn`, and `cafleet pane` exposes `capture | exec | wake`.
- [ ] Every pane-interaction command and `agent deregister` identify their target by `--agent-id`; `pane wake` carries the sender as `--from`; no `--member-id` **flag** exists anywhere.
- [ ] `docs/`, `SPEC.md`, `README.md`, `skills/`, and `.claude/` describe the `agent`/`pane` surface exclusively; no `cafleet member ` invocation or `--member-id` flag remains outside `design-docs/`.
- [ ] The spawn-prompt mechanism is unchanged from the 0000113 baseline: `agent spawn` runs `str.format` placeholder substitution and forwards only `CAFLEET_DATABASE_URL` into the pane. No env-var-injection mechanism is (re)introduced.
- [ ] The 0000113 guard test (which forbids `agent spawn` / `cafleet pane` / `$CAFLEET_*`) is replaced by the inverse guard — forbidding the `member` group and the `--member-id` flag — running inside `mise //cafleet:test`.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:lint-overlay` pass.

---

## Background

### The baseline this design starts from

This design assumes 0000112 and 0000113 are **Complete**. The resulting baseline:

- **CLI**: `cafleet member create | delete | list | capture | exec | ping | nudge`, all targeting via `--member-id`. `member send-input` is gone (removed by 0000112), and `member create` / `member nudge` carry the `--text` / `--text-file` pair backed by `_text_input.py` (`read_text_input` + `substitute_spawn_placeholders`).
- **Whole corpus consistent on `member`**: `docs/`, `SPEC.md`, `README.md`, every skill, and `.claude/` describe the shipped `member` surface. No `cafleet agent spawn` or `cafleet pane` reference remains anywhere — 0000113 removed them and added a static guard **forbidding** their reintroduction.
- **Identity mechanism is settled and documented**: spawn-prompt identity reaches an agent through `str.format` substitution of `{fleet_id}` / `{agent_id}` / `{director_agent_id}` / `{coding_agent}` (with literal braces doubled). Only `CAFLEET_DATABASE_URL` is forwarded into a pane (`cafleet/src/cafleet/cli/member.py:260-261`). The env-var-injection model ("`$CAFLEET_*` injected", "delivered verbatim, no substitution") no longer appears in any doc — 0000113 deleted it.

### Why split into `agent` + `pane`

The split gives one mental model per concern: `agent` is the registry + lifecycle home (a "member" is just an agent with a placement), and `pane` is the home for keystroke interaction with a pane-bound agent. Design 0000111 first proposed this split but landed it **docs-only**; 0000113 later folded the corpus back onto the shipped `member` names to kill the resulting drift. This design realizes the split properly — code and documentation migrated in one cycle — so there is no docs-vs-code gap to guard against in the `agent`/`pane` direction ever again.

### Relationship to adjacent designs

| Design | Status (assumed at 0114 start) | Relationship |
|--------|--------|--------------|
| 0000112 `text-file-input-and-send-input-removal` | **Complete** (land first) | Removes `member send-input` (so `pane` has no `input`) and gives `member create` / `member nudge` the `--text` / `--text-file` pair via `_text_input.py`. This design inherits that state: `agent spawn` carries `--text` / `--text-file`; the `pane` group is `capture` / `exec` / `wake`. |
| 0000113 `spawn-prompt-simplification` | **Complete** (land first) | The direct predecessor. It reconciles the whole corpus onto `member`, removes the env-var-injection fiction, makes `str.format` the canonical documented mechanism, and installs a guard forbidding `agent`/`pane`. This design is 0000113's **forward successor**, not a supersession: it starts from 0000113's clean, consistent baseline and migrates it onward to `agent`/`pane`, flipping the guard direction. |
| 0000111 `cli-simplification` | Complete (historical) | First proposed the `agent`/`pane` split, docs-only; those docs were folded back to `member` by 0000113. This design re-establishes the split across code **and** docs. |

---

## Specification

Because 0000113 leaves the whole corpus on `member`, this design **defines** the `agent`/`pane` contract (it is not "match the existing docs" — the docs are migrated here). The contract below restores the 0000111 `agent`/`pane` surface, expressed over the **`str.format` mechanism the 0000113 baseline made canonical** (never the env-var-injection prose).

### Command mapping

| Current (`member`) | Target | Change kind |
|---|---|---|
| `member create` | `agent spawn` | Move into `agent`; keep all flags and the `str.format` substitution + `CAFLEET_DATABASE_URL` forwarding verbatim. |
| `member delete` | `agent deregister` | Merge into the existing `agent deregister` (today a registry-only soft-delete): it gains pane teardown (backend exit keystroke → 15.0 s graceful poll → deregister) and the `--force` / `-f` immediate kill-pane path. `--member-id` → `--agent-id`. |
| `member list` | `agent list` | Reconcile into the existing `agent list` (see Step 4 — not additive). |
| `member capture` | `pane capture` | Move into new `pane`; `--member-id` → `--agent-id`. Flags unchanged (`--lines` default 20, `--ansi`/`--no-ansi`). |
| `member exec` | `pane exec` | Move into new `pane`; `--member-id` → `--agent-id`. Positional `COMMAND` unchanged. |
| `member ping` | `pane wake --poll-only` | Merge into new `pane wake`; `--member-id` → `--agent-id`. Drop `--quiet`. |
| `member nudge` | `pane wake --message` | Merge into new `pane wake` with the flag-polarity swap below. |

`agent register` and `agent show` are unchanged.

### Flag polarity: `pane wake`

`member nudge` names the **sender** `--agent-id` and the **target** `--member-id`. The target surface swaps both:

| `member nudge` (current) | `pane wake --message` (target) | Role |
|---|---|---|
| `--agent-id <sender>` | `--from <sender>` | Sender (persisted as the task's `from_agent_id`) |
| `--member-id <target>` | `--agent-id <target>` | Target (the agent being woken) |

A mechanical rename keeping `--agent-id` on the sender would invert sender and target — the swap is required. Exactly one of `--poll-only` / `--message` is required; `--message` requires `--from` and exactly one of `--text` / `--text-file` (from 0000112). Both/neither mode, or `--message` without `--from` or a body, is exit 2.

### JSON key rename (observable-contract change)

The current `member` pane commands emit `{"member_agent_id": …}` (`cafleet/src/cafleet/cli/member.py:521/595/640/688/741` — capture/send-input/exec/ping/nudge). Every ported `pane` command MUST emit **`agent_id`** instead: `pane capture` → `{agent_id, pane_id, lines, content}`; `pane exec` → `{agent_id, pane_id, command}`; `pane wake` → `{agent_id, pane_id}` (`--poll-only`) / `{agent_id, pane_id, task_id, notification_sent}` (`--message`). This is a real output-contract change, not just a flag rename.

### Target surface (the contract this design establishes)

- **`agent spawn`** — flags `--agent-id` (Director) / `--name` / `--description` / `--coding-agent` / `--model` / `--role` / `--full` / `--text` / `--text-file`. Keeps `str.format` substitution and `CAFLEET_DATABASE_URL`-only forwarding. Output line `<agent_id> <name> backend=<coding_agent> pane=<pane_id>`; `--full` the 6-line block; JSON carries `placement`. One-monitor-per-fleet guard preserved. **Docs describe the `str.format` mechanism, not env-var injection.**
- **`agent deregister`** — one unified error model, every failure exits 1. Default path: exit keystroke → poll every 500 ms up to 15.0 s → deregister; on timeout, print the pane tail (last 80 lines) + recovery hint on stderr and exit **1** (the current `member delete` exits 2 on timeout — change to 1). `--force`: kill-pane immediately. Success headers `Agent deregistered successfully.` (no-pane + graceful-close) / `Agent deregistered (--force).` (replacing `Member deleted.` / `Member deleted (--force).`, `member.py:343/362/393`); timeout stderr `…did not close within 15.0s after the exit keystroke.` (replacing `…after /exit.`, `member.py:407`). Root-Director guard fires **before any pane mutation** (`Error: cannot deregister the root Director; use 'cafleet fleet delete' instead`); Administrator guard `Error: Administrator cannot be deregistered`. JSON `{agent_id, pane_status}` with `pane_status` ∈ `(pending — no pane)` / `<pane> (closed)` / `<pane> (killed)` / `<pane> (timeout)`.
- **`pane` resolution + exit codes** (shared): cross-fleet / unknown / inactive `--agent-id` → exit 1 `Error: Agent <id> not found`; no placement row → exit 1 ``…not spawned via `cafleet agent spawn``; pending pane → exit 1 with each subcommand's "nothing to …" wording. Common exit 2 for per-subcommand arg validation.
- **`pane capture`** — `--agent-id`, `--lines` (default 20, no `--tail` alias), `--ansi`/`--no-ansi`.
- **`pane exec`** — `--agent-id`, positional `COMMAND` (strip whitespace; reject empty/newline exit 2). Text `Sent bash command '<cmd>' to agent <name> (<pane>).`.
- **`pane wake`** — see [polarity](#flag-polarity-pane-wake). `--poll-only` text `Woke agent <name> (<pane>) — poll keystroke dispatched.`; `--message` text `Woke <name> (<pane>) — task <id> queued, Esc-safeguarded preview dispatched.` (+ `no pane` / `inline preview not delivered` variants).

### Module layout

| File | Action |
|---|---|
| `cafleet/src/cafleet/cli/pane.py` | **New.** The `pane` group: `capture`, `exec`, `wake` (ported from `member_capture` / `member_exec` / merged `member_ping` + `member_nudge`). |
| `cafleet/src/cafleet/cli/agent.py` | Add `spawn` (from `member_create`); replace `deregister` with the merged pane-teardown + `--force` logic (from `member_delete`); reconcile `list` with the placement column + `--activity` (from `member_list`). |
| `cafleet/src/cafleet/cli/member.py` | **Delete.** |
| `cafleet/src/cafleet/cli/_helpers.py` | Replace `director_member_options` (defines `--member-id`) with an `--agent-id` target-option decorator. Host the shared resolution helpers (`_load_authorized_member`, `_require_member_pane`) moved out of `member.py`, `member_id` params renamed to `agent_id`, placement-missing templates pointing at `cafleet agent spawn`. |
| `cafleet/src/cafleet/cli/__init__.py` | Drop the `member` import + `cli.add_command(member)`; add the `pane` import + `cli.add_command(pane)`. |

**Import direction (avoid cycles):** `agent.py` and `pane.py` import shared helpers from `_helpers.py`; `_helpers.py` MUST NOT import from `agent.py` / `pane.py`. `_resolve_coding_agent` has a single caller (`agent spawn`) — keep it in `agent.py` rather than `_helpers.py`.

The broker layer is unchanged: `register_agent`, `deregister_agent`, `list_members`, `list_members_with_activity`, `get_agent`, `update_placement_pane_id`, and `send_message` keep their names and signatures (they name the domain concept "member" = an agent with a placement). Output formatters (`format_member`, `format_member_list`, `format_member_list_activity`) keep their names — internal CLI decoration, no external contract.

### Embedded command strings and docstrings

Every user-visible string and docstring that names a `member` subcommand is updated to its `agent`/`pane` equivalent (per `.claude/rules/removal.md` — no deprecation notices):

| File | Current reference | Target |
|---|---|---|
| `cafleet/src/cafleet/cli/agent.py` (ported spawn/deregister) | placement-missing / recovery hints naming `member create` / `member delete` / `member capture` | `agent spawn` / `agent deregister` / `pane capture` |
| `cafleet/src/cafleet/broker/agents.py` | ``'cafleet member create --role monitor'`` (registration validation error) | `cafleet agent spawn --role monitor` |
| `cafleet/src/cafleet/cli/monitor.py` | ``'cafleet member create --role monitor'`` (monitor-start warning) | `cafleet agent spawn --role monitor` |
| `cafleet/src/cafleet/multiplexer/tmux.py` | docstrings/messages naming `cafleet member ping` / `member nudge` / `member commands` (incl. the user-facing wake-nudge string ~line 260) | `pane wake --poll-only` / `pane wake --message` / `pane` |
| `cafleet/src/cafleet/multiplexer/base.py` (line ~165) | docstring `` ``cafleet member ping`` `` | `pane wake --poll-only` |
| `cafleet/src/cafleet/coding_agent/base.py` | docstring "Called by `member create`" | `agent spawn` |
| `cafleet/src/cafleet/coding_agent/overlay_coverage.py` (line ~50) | comment ``filled by `member create` str.format()`` | `agent spawn` |

Verify with a repo-wide grep of `cafleet/src/` for `member create` / `member delete` / `member list` / `member capture` / `member exec` / `member ping` / `member nudge` after the sweep — zero matches expected.

### Guard test (flip 0000113's guard)

0000113 installs a static test that fails if any skill/doc reintroduces `cafleet agent spawn`, `cafleet pane`, or a `$CAFLEET_*` reference. This design **removes that guard and installs its inverse**:

- **Source guard**: assert `cafleet/src/cafleet/cli/` registers no `member` Click group and no `--member-id` **CLI option flag** (the literal `"--member-id"` in a `click.option(...)`). Scope to the flag string, **not** the Python identifier `member_id` — this design deliberately retains `member_id` / `list_members` / `format_member_list` as internal names (see [Non-goals](#non-goals)), so an identifier grep would false-positive.
- **Docs/skills guard**: assert no `cafleet member ` invocation and no `--member-id` flag remain under `docs/`, `SPEC.md`, `README.md`, `skills/`, and `.claude/` (excluding `design-docs/`, the historical record). The `$CAFLEET_*` assertion from 0000113 is retained if still desired — this design does not reintroduce env-var identity references, so it stays green.

### Non-goals

| Item | Reason |
|---|---|
| Identity-injection mechanism | Settled by 0000113 (`str.format` canonical; only `CAFLEET_DATABASE_URL` forwarded). This design preserves it verbatim and does **not** reintroduce env-var injection. |
| Text-input flags / `send-input` removal | Owned by 0000112 (dependency). This design assumes the post-0000112 state. |
| Broker function / output-formatter renames | Internal names with no CLI contract; renaming is churn without user-visible benefit. |
| The `member`-as-domain-concept ("a member is an agent with a placement") in prose, `agent_card_json`, or schema | Only the CLI *command* surface changes; the conceptual model stays. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> Documentation-first per `.claude/rules/documentation-maintenance.md`: because the 0000113 baseline leaves every doc/skill on `member`, the doc/skill migration (Steps 1–2) is substantial and runs **before** code.
>
> **Precondition — do not begin until 0000112 AND 0000113 are `Complete`.** The doc/skill baseline (Steps 1–2) assumes 0000113 has consolidated the corpus on `member`; the code steps assume 0000112's `_text_input.py` (`read_text_input` + `substitute_spawn_placeholders`) exists and `send-input` is gone.

### Step 1: Docs, SPEC & README migration (`member` → `agent`/`pane`)

- [ ] `docs/spec/cli-options.md` — rewrite the `member` surface to `agent` + `pane`: subcommand summary rows; the `agent` group gaining `spawn` and the merged `deregister`; a new `cafleet pane` section (`capture`/`exec`/`wake` with the `--poll-only`/`--message` modes and `--from`); `--member-id` → `--agent-id`; the error-message table. Describe the `str.format` spawn mechanism (not env-var injection). <!-- completed: -->
- [ ] `docs/concepts/*.md` — migrate `member`→`agent`/`pane` in `member-lifecycle.md`, `tmux-push.md`, `bash-routing.md`, `monitoring.md`, `token-reduction.md`, `coding-agents.md`, and `overview.md` (command-group overview). <!-- completed: -->
- [ ] `docs/how-to/monitor-and-recover.md` and `docs/reference/coding-agents/{claude,codex,opencode}.md` — migrate CLI examples to `agent spawn` / `pane *`. <!-- completed: -->
- [ ] `README.md` — reflect the `agent`/`pane` command groups (use the `/update-readme` skill if the surface change is large). <!-- completed: -->
- [ ] `SPEC.md` — migrate the CLI, spawn-prompt, and pane-interaction contract surfaces to `agent`/`pane`, preserving the `str.format` substitution note. <!-- completed: -->

### Step 2: Skills migration (`member` → `agent`/`pane`)

- [ ] `skills/cafleet/SKILL.md`, `skills/cafleet/roles/{director,member,monitor}.md`, and `skills/cafleet/reference/*.md` — migrate every `cafleet member *` / `--member-id` reference to `agent spawn` / `agent deregister` / `pane capture|exec|wake` / `--agent-id` / `--from`. <!-- completed: -->
- [ ] Rename the spawn-prompt skeleton and every per-role delta from `member create` to `agent spawn` while **keeping** 0000113's `str.format`-literal identity approach (CLI-substituted `{fleet_id}`/`{agent_id}`/`{director_agent_id}`/`{coding_agent}` literals; no `$CAFLEET_*` env-var references, no re-added translation notes). <!-- completed: -->
- [ ] `skills/cafleet-design-doc/**` and `skills/cafleet-research/**` — migrate spawn recipes and Director role docs to `cafleet agent spawn` / `pane *`. <!-- completed: -->
- [ ] `.claude/rules/bash-tool.md`, `.claude/rules/coding-agent-overlay.md`, and `.claude/skills/skill-author/SKILL.md` — migrate `cafleet member *` primitives (Director-side `member ping`/`member exec`/`member nudge`) to `pane wake --poll-only` / `pane exec` / `pane wake --message`. <!-- completed: -->

### Step 3: Shared helpers (code)

- [ ] In `_helpers.py`, replace `director_member_options` (`--member-id`) with an `--agent-id` target-option decorator (help "Target agent's ID"). <!-- completed: -->
- [ ] Move `_load_authorized_member` / `_require_member_pane` out of `member.py` into `_helpers.py`; rename `member_id` params to `agent_id`; update placement-missing / rollback templates to name `cafleet agent spawn` / `cafleet agent deregister`. Keep `_resolve_coding_agent` in `agent.py` (single caller). <!-- completed: -->

### Step 4: `agent` group (code)

- [ ] Add `agent spawn` (port `member_create`): flags per [target surface](#target-surface-the-contract-this-design-establishes); keep `str.format` substitution (`substitute_spawn_placeholders`) and `CAFLEET_DATABASE_URL`-only forwarding verbatim. <!-- completed: -->
- [ ] Replace `agent deregister` with the merged logic (port `member_delete`): pane teardown, `--force`/`-f`, root-Director guard **before** pane mutation, Administrator guard, no-pane soft-delete, timeout tail + recovery hint + **exit 1** (was 2), the exact success/timeout strings in the [target surface](#target-surface-the-contract-this-design-establishes), `{agent_id, pane_status}` JSON. `--agent-id` target. <!-- completed: -->
- [ ] Reconcile `agent list` (**not** additive): current `agent list` (`agent.py:39–51`) uses `broker.list_agents` + `output.format_agent` via `client_command(renders_agent_card=True)`; `member list` (`member.py:448–475`) uses `broker.list_members` / `list_members_with_activity` + `format_member_list` / `format_member_list_activity`, bypasses `client_command`, and **excludes the root Director**. Specify which broker call + formatter the merged command uses for the default vs `--activity` projections and resolve the root-Director-inclusion difference to match the spec's placement-column output. <!-- completed: -->

### Step 5: `pane` group (code, new module)

- [ ] Create `pane.py` with the `pane` group and `pane capture` (port `member_capture`). Emit `agent_id` (not `member_agent_id`) in JSON — applies to all three `pane` subcommands. <!-- completed: -->
- [ ] Add `pane exec` (port `member_exec`: `--agent-id`, positional `COMMAND`, strip/reject-empty/reject-newline exit 2, `Sent bash command …` text + `{agent_id, pane_id, command}` JSON). <!-- completed: -->
- [ ] Add `pane wake` merging `member_ping` + `member_nudge`: `--agent-id` target, `--poll-only` xor `--message`, `--message` requires `--from` + `--text`/`--text-file`; drop `--quiet`; `Woke …` text + JSON shapes; enforce the sender/target polarity swap. <!-- completed: -->

### Step 6: Wire and delete

- [ ] `__init__.py`: remove the `member` import + `cli.add_command(member)`; add the `pane` import + `cli.add_command(pane)`. <!-- completed: -->
- [ ] Delete `cafleet/src/cafleet/cli/member.py`. <!-- completed: -->

### Step 7: Embedded strings & docstrings

- [ ] Update `broker/agents.py` and `cli/monitor.py` (`cafleet member create --role monitor` → `cafleet agent spawn --role monitor`). <!-- completed: -->
- [ ] Update `multiplexer/tmux.py`, `multiplexer/base.py:165`, `coding_agent/base.py`, and `coding_agent/overlay_coverage.py:50` per the [embedded-strings table](#embedded-command-strings-and-docstrings); grep `cafleet/src/` to confirm zero residual `member <subcommand>` strings. <!-- completed: -->

### Step 8: Guard test

- [ ] Remove 0000113's anti-`agent`/`pane` guard and install the inverse (source + docs/skills) per [Guard test](#guard-test-flip-0000113s-guard); run inside `mise //cafleet:test`. <!-- completed: -->

### Step 9: Tests

- [ ] Rename the CLI test modules and helper (`test_member*.py` → `test_agent_*` / `test_pane_*`; `_member_helpers.py` → `_agent_helpers.py`), updating every `CliRunner` invocation from `member …` / `--member-id` to `agent` / `pane` / `--agent-id`. <!-- completed: -->
- [ ] Update assertions changed by the merge: `agent deregister` timeout exits 1 (was 2), success headers `Agent deregistered successfully.` / `(--force).`, no `--quiet` on wake, `Woke …` wording, JSON key `agent_id` (was `member_agent_id`), and placement-missing / recovery-hint strings naming `agent spawn` / `agent deregister` / `pane capture`. <!-- completed: -->
- [ ] Add coverage for merged behavior: `agent deregister` default-graceful vs `--force` vs timeout; `pane wake --poll-only` and `pane wake --message --from <s> --agent-id <t>` asserting the sender/target polarity; `agent list --activity`. <!-- completed: -->
- [ ] Update `test_help_budget.py` expectations for the reorganized `agent` / `pane` help; update `test_monitor.py` / `test_fleet_bootstrap.py` embedded spawn-command strings. <!-- completed: -->

### Step 10: Verify

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:lint-overlay`; fix any fallout. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-02 | Initial draft. |
| 2026-07-03 | Interview round 4: user rejected the design premise — the shipped `member` surface stays; no `agent spawn` / `pane` group will be created. Status → Aborted. Rounds 1–3 answers preserved in `question.md`. |
