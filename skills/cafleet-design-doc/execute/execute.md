# Design Doc Execute (CAFleet Edition)

Implement features based on a design document using up to four roles orchestrated via the CAFleet message broker: Director (orchestrator), Programmer (implements), Tester (writes tests), and Verifier (E2E/integration testing). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The Director judges which members to spawn based on the nature of the implementation tasks. For each step, the Tester writes unit tests first, the Director reviews and approves them, then the Programmer implements code to pass the tests. The Director also reviews the Programmer's implementation for code quality and design doc compliance before committing. After all TDD steps, the Verifier performs E2E/integration verification (Phase D) if spawned. After user approval, the Director runs the full publication flow: Step 6 pushes the feature branch and opens a PR with `@copilot` requested, Step 7 runs a Copilot review loop — driven by the monitoring member's idle-nudges on the `cafleet monitor` heartbeat — that routes inline comments to the still-live Programmer / Tester and ends only when the user instructs termination or Copilot reports no remaining concerns, and Step 8 finalizes, commits the completion marker, pushes it (when the branch is tracked on origin), and tears the team down.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../../cafleet/reference/coding-agent/<name>.md`](../../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet, spawn members via `cafleet member create`, validate doc, assign steps, review tests against design doc, review implementation code for quality and compliance, commit after each phase, escalation arbitration, orchestrate TDD cycle | Write code, write tests | [roles/director.md](roles/director.md) |
| **Programmer** | Member agent | Implement code to pass tests, run tests, report results via `cafleet message send`, escalate test defects to Director, update design doc checkboxes and Progress counter | Write or modify tests, commit code, communicate with user directly | [roles/programmer.md](roles/programmer.md) |
| **Tester** | Member agent | Read design doc, write unit tests per step, fix tests based on Director feedback, report to Director via `cafleet message send` | Write implementation code, commit code, communicate with user directly | [roles/tester.md](roles/tester.md) |
| **Verifier** | Member agent (optional) | E2E/integration testing, tool discovery, evidence collection (screenshots, logs, output), failure reporting with suggested fixes | Write code, write tests, commit, communicate with user directly | [roles/verifier.md](roles/verifier.md) |

## Additional resources

- For the document template, see: [../reference/template.md](../reference/template.md)
- For section guidelines and quality standards, see: [../reference/guidelines.md](../reference/guidelines.md)
- For the inter-agent coordination protocol (verb + pointer schema, `COMMENT(role)` markers), see: [../reference/coordination.md](../reference/coordination.md)

## Coordination Protocol

This skill's Director, Programmer, Tester, and Verifier coordinate via the verb + pointer schema and `COMMENT(role)` markers defined canonically in [../reference/coordination.md](../reference/coordination.md) — the single source of truth for the 6 verbs, the 3 pointer forms, the message format, the `COMMENT(role)` marker grammar, the issue/status split, Copilot routing, anchorless status, finalize-time cleanup, and Director per-file detail recovery.

Two skill-specific notes layer on top of that canonical protocol:

- **Roles in play**: this skill uses only the `director`, `programmer`, `tester`, `verifier`, `claude`, and `copilot` marker roles — never `drafter` or `reviewer` (those belong to the create workflow). Copilot review here is the full source-file / design-doc / PR-level routing; finalize happens at `Status: Complete` (Step 8).
- **Verifier Phase 1 exemption**: The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination, and rides as a free-form multi-line cafleet body (same precedent as the Analyzer's question list in the interview workflow). Phase 2 verification reports follow the schema.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` (no separate `cafleet agent register` call) — and spawns each needed member via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet fleet create, cafleet member create, orchestrates TDD cycle)
      +-- Programmer (member agent -- implements code to pass tests)
      +-- Tester (member agent -- writes unit tests per step)
      +-- Verifier (member agent, optional -- E2E/integration testing)
```

## Prerequisites

- The Director MUST be running inside a tmux session (required by `cafleet member create`). Verify by running `cafleet doctor` before spawning anyone — it reports the tmux session/window/pane identifiers and exits non-zero with a clear message when the environment is not ready. If `cafleet doctor` reports a problem, abort and surface its message to the user. Do NOT invoke `tmux display-message`, `printenv TMUX`, or any other raw tmux/env probe — `cafleet doctor` is the only supported environment check (see `skills/cafleet/SKILL.md` § *use cafleet primitives only*).
- `gh` must be authenticated for Steps 6 + 7. Lack of auth is NOT fatal — the Director checks `gh auth status` at Step 6a and falls back to Step 8 local-finalize, skipping the PR and Copilot review loop entirely. All other prerequisites (tmux, approved design doc, feature branch) remain unchanged.

## Process

**Run to completion.** Once the execute workflow is invoked, the fleet operates autonomously and collaboratively through every task in the design document. The Director keeps driving the team — dispatching the next step to each idle member the moment it is ready — until all Implementation tasks and Success Criteria are complete. The designed checkpoints stay in force: the Step 5 user-approval gate, the user's "stop means stop" halt during Step 7, and escalations that require a genuinely new user decision.

### Step 1: Resolve Design Document Path (Director)

Before validation, resolve `$ARGUMENTS` into a concrete `design-doc.md` path.

#### Phase 1: Base Directory Resolution

Read the `cafleet` skill's `reference/base-dir.md` for the no-bypass write protocol and `<unset>` sentinel contract. Then resolve BASE based on whether `$ARGUMENTS` was supplied:

- **`$ARGUMENTS` present** (the typical execute-a-specific-doc flow): canonicalize `$ARGUMENTS` and call the task-scope resolver positionally. `$ARGUMENTS` is normally a slug name (`0000060-skill-task-scoped-base-dir`) or a path containing such a slug.

  - **Relative input** — accept any of: `0000060-foo`, `0000060-foo/design-doc.md`, `design-docs/0000060-foo`, `design-docs/0000060-foo/design-doc.md`. Canonicalize to `design-docs/<slug>` by: (1) stripping the trailing `/design-doc.md` if present; (2) stripping the leading `design-docs/` if present; (3) prepending `design-docs/`. The skill's Step 0 does NOT perform this stripping (per the `cafleet` skill's `reference/base-dir.md` § *Consumer contract*) — canonicalize first, then run the skill's **Step 0 (task-scope resolution)** with the relpath `design-docs/<slug>`.

  - **Absolute path** (e.g. `/abs/path/to/design-docs/0000060-foo/design-doc.md`): Step 0 accepts only the task-folder path, not a child file. Strip the trailing `/design-doc.md` if present so the absolute path identifies the task folder, then run Step 0 with that absolute task-folder path. Step 0 accepts the absolute path if it lies strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

  Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder (the slug folder) and `${RESOLVED_ARGS} = ${BASE}/design-doc.md` (short-circuits at Tier 1 below). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${RESOLVED_ARGS}` to the literal `$ARGUMENTS` path so Tier 1 / Tier 2 still run against the user-supplied path, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.

- **`$ARGUMENTS` absent** (the discover-all-approved-docs flow): the no-argument form scans `<repo-root>/design-docs/`, so the Director MUST invoke from the repo root. Verify with `git rev-parse --show-toplevel` and abort with a clear "invoke from the repo root" error if `cwd` differs. Then run the skill's **Step 1 (shared-root resolution)**:

  Step 1 resolves `${BASE}` to the CWD (the verified repo root). In the rare edge case where the repo root is itself `$HOME` or under `~/.claude`, Step 1 reaches **Step 2** of base-dir resolution (its decision-surface prompt); there, explicitly choose the `${CWD}` candidate so `${BASE}` stays the verified repo root — do NOT pick `/tmp/claude-code`, which would make `${RESOLVED_ARGS} = /tmp/claude-code/design-docs/` and point the discovery scan at the wrong directory. With `${BASE}` resolved to the repo root, set `${RESOLVED_ARGS} = ${BASE}/design-docs/` — this matches Tier 3 below and engages the discovery flow that scans every approved slug under `<repo>/design-docs/`.

#### Phase 2: Three-Tier Detection

Using `${RESOLVED_ARGS}`, apply a three-tier detection strategy, evaluated in order:

| Tier | Condition | Action |
|:--|:--|:--|
| 1 — Direct file path | `${RESOLVED_ARGS}` ends with `design-doc.md` | Use as-is |
| 2 — Slug directory | `${RESOLVED_ARGS}` is a directory that contains `design-doc.md` directly | Append `/design-doc.md` |
| 3 — Base directory | `${RESOLVED_ARGS}` is a directory containing `**/design-doc.md` (one level deep) | Enter discovery flow |

Tier evaluation is sequential and short-circuits.

> **Tier 3 with task-scope BASE**: When Phase 1's present-argument branch fires, `${BASE}` is one slug folder and `${RESOLVED_ARGS}` is set to `${BASE}/design-doc.md` — Tier 1 short-circuits before Tier 3 is reached, so the task-scoped BASE never exercises the discovery flow. Tier 3 is preserved for the no-argument branch, where `${BASE}` is the repo root and the discovery flow scans every approved slug under `<repo>/design-docs/`.

#### Discovery Flow (Tier 3)

When the base directory tier matches:

1. **Discover**: Use Glob to find all `**/design-doc.md` files under the base directory, then filter results to keep only those exactly one level deep (i.e., `<base>/<slug>/design-doc.md`). Discard any deeper matches.
2. **Read Status**: For each discovered file, read the `**Status**:` field from the document header.
3. **Filter**: Keep only documents with `Status: Approved`. Documents with any other status (`Draft`, `In Progress`, `Complete`) are excluded.
4. **Branch by count**:

| Count | Behavior |
|:--|:--|
| 0 | Error and abort (see Error: Zero Approved below) |
| 1 | Auto-select: proceed with this document directly |
| 2+ | Present the approved docs as choices through {decision_surface} (see Selection below); if {decision_surface} caps how many options it shows at once and the count exceeds that cap, paginate (see Pagination below) |

#### Selection (2+ approved docs)

Present the approved docs as labeled choices through {decision_surface} — one option per doc, labeled with its slug (directory name). A free-text fallback is always available for the user to type a direct path or cancel.

#### Pagination (only if {decision_surface} caps the option count)

If {decision_surface} caps how many options it shows at once (your overlay states the cap) and the approved-doc count exceeds it, paginate with all options sorted alphabetically by slug:

- **Non-last page**: fill the prompt up to the cap, reserving the last slot for a `"More..."` option that advances to the next page.
- **Last page rule**: when the remaining items fit within the cap, show them all directly (no `"More..."` needed) — this avoids a last page falling below the surface's minimum option count.
- Continue until the user selects a document or supplies free-form text.

If {decision_surface} has no such cap (e.g. a plain message that lists all choices at once), present all approved docs in one prompt — no pagination needed.

#### Error: Zero Approved Docs

When design docs exist but none have `Status: Approved`, display a message listing every found doc with its current status (so the user sees why none qualified), noting that only `Status: Approved` docs can be executed, then abort (do not proceed to team creation or execution).

#### Error: Invalid Path

When `${RESOLVED_ARGS}` does not match any of the three tiers (not a file path ending in `design-doc.md`, not a directory containing `design-doc.md`, and no `**/design-doc.md` underneath), display an invalid-argument error naming `${RESOLVED_ARGS}` and the three accepted forms (direct `design-doc.md` path, slug directory, or no argument to discover all under `${BASE}/design-docs/`), then abort.

After resolution, the resolved path is used as the design document path for all subsequent steps.

### Step 2: Validate Design Document & Create Branch (Director)

Before registering with CAFleet:

1. Read the design document completely.
2. Check for `COMMENT(` markers using Grep. If found, resolve them directly: apply the requested changes and remove the markers. Verify with Grep that no `COMMENT(` markers remain before proceeding.
3. Check for `FIXME(claude)` markers in the codebase using Grep. If found, note them for the Programmer to resolve first.
4. Determine the step order and total number of steps.
5. **Create a feature branch if on the default branch.** Get the default branch with `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` and the current branch with `git branch --show-current`. If they match, use {decision_surface} to propose the branch name `feat/<design-doc-slug>` and ask the user to approve before creating it. The user will create the branch themselves or approve the proposed name. If already on a non-default branch, skip this step.

### Step 3: Register & Spawn Members (Director)

Load the `cafleet` skill and Read its `reference/supervision.md`.

#### 3a. Establish a CAFleet fleet and capture the root Director's `agent_id`

`cafleet fleet create` (which must be run inside a tmux session) atomically creates the fleet and registers a root Director bound to the current tmux pane — there is no separate `cafleet agent register` step for the Director. Use `--json` so both IDs are machine-parseable:

```bash
cafleet fleet create --label "design-doc-execute-{slug}" --json
# → { "fleet_id": <int>, "administrator_agent_id": <int>, "director": { "agent_id": <int>, "name": "Director", "placement": {...} } }
```

Capture `fleet_id` and `director.agent_id` from the JSON response. Substitute them for `<fleet-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal ids. Remember: `--fleet-id` and `--agent-id` are per-subcommand options that go **after** the subcommand name.

If you already have a running fleet (e.g. an outer orchestration), reuse its `fleet_id` and its root Director's `agent_id` instead of creating a new fleet. Do **not** attempt to register a second Director with `cafleet agent register --name Director` — the root Director from `fleet create` is the team lead; a second registration would just create an unrelated agent with no placement row.

#### 3b. Spawn the monitoring member (first-in)

This team **keeps an active heartbeat** (Step 7's Copilot loop needs a turn source — see Step 7), so it adopts the monitoring-member model: the Director does **not** run `cafleet monitor start` itself. The **first** `cafleet member create` in the fleet is the dedicated monitoring member, spawned with `--role monitor --model {monitor_model}`; it launches `cafleet monitor start --fleet-id <fleet-id>` as a background task in its own pane, confirms with `cafleet monitor status`, and reports `ready: monitor live` to the Director. **Receipt of that handshake gates the first ordinary `member create`** (first-in). The heartbeat runs **unchanged** through Steps 3–8; its `monitor start` background task is stopped in Step 8's cleanup (first-out). See the `cafleet` skill's `roles/monitor.md` for the canonical spawn prompt and lifecycle, and its `reference/supervision.md` for supervision obligations (Authorization-Scope Guard, idle semantics).

**Spawn-prompt delta (execute only).** Execute's monitoring member runs an **extended** routine versus the canonical `cafleet` skill's `roles/monitor.md` prompt: when it finds the Director **idle**, it nudges **unconditionally** — it does **not** gate the nudge on naming un-acked inbox items or stalled members. The unconditional idle-nudge is what grants the Director a re-poll turn during a quiet Copilot wait (Step 7), so `silence_ticks` can advance even when the inbox is empty and members have already reported their fixes. State this delta in execute's monitoring-member spawn prompt; the canonical `cafleet` skill's `roles/monitor.md` routine keeps its conditional nudge. No Step-7 enter/exit handshake is needed — the monitoring member is PR-agnostic and the Director's Step-7 per-turn checklist consumes the granted turn (harmless outside Step 7: the Director re-polls, finds nothing new, idles again).

#### 3c. Analyze implementation tasks to decide team composition

Based on the design document steps (see [roles/director.md](roles/director.md) for the full decision matrix):

| Task nature | Team composition |
|---|---|
| Code implementation | Programmer + Tester |
| Config/documentation only | Programmer only |
| E2E verification needed (user-visible changes, CLI/UI/API) | + Verifier |

#### 3d. Read role files

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file via `Read` on its first turn — do NOT inline the content):

- `skills/cafleet-design-doc/execute/roles/programmer.md`
- `skills/cafleet-design-doc/execute/roles/tester.md` (if Tester needed)
- `skills/cafleet-design-doc/execute/roles/verifier.md` (if Verifier needed)

#### 3e. Spawn each member via `cafleet member create`

Each member is spawned from the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the per-role delta below. `{fleet_id}` / `{agent_id}` / `{director_agent_id}` are filled by `member create`'s `str.format()`; the `[INSERT …]` markers (`[INSERT DESIGN DOC PATH]`, `[INSERT abs path to roles/<role>.md]`) are shell-substituted by the Director first (double any literal `{`/`}` per the Template-safety note in `cafleet/reference/director.md`). All three roles load `cafleet` + `cafleet-design-doc` and take `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` as their only context line; each delta below gives the role's title, role-file, IMPORTANT lines (verbatim), and start cue.

> **Spawn-prompt audit file (two-step pattern)**: every spawn in this skill follows the same two steps — (1) **render** the prompt (substitute the `[INSERT …]` markers; leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass); (2) **write** it to `${BASE}/prompts/<role>-<UTC-compact>.md` (`<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`; create `${BASE}/prompts/` on first write; same-second collision → append `_2`, `_3`, … — never overwrite), then invoke `cafleet member create --prompt-file <abs path>` (see the per-role spawn templates and commands below). The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol* and its `reference/director.md` § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

**Programmer spawn prompt** (skeleton + delta):

| Slot | Programmer |
|---|---|
| ROLE TITLE | `the Programmer` |
| role-file | `roles/programmer.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` |
| start cue | `Start by reading the design document. Then wait for the Director to assign your first step.` |

Render the prompt to `${BASE}/prompts/programmer-<UTC-compact>.md` per the 3e two-step audit-file pattern (leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --name "Programmer" \
     --description "Implements code to pass tests per step" \
     --prompt-file ${BASE}/prompts/programmer-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<programmer-agent-id>` in every subsequent command.

**Tester spawn prompt** (skeleton + delta; if needed):

| Slot | Tester |
|---|---|
| ROLE TITLE | `the Tester` |
| role-file | `roles/tester.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` / `IMPORTANT: Do NOT write implementation code — only test code.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` |
| start cue | `Start by reading the design document. Then wait for the Director to assign your first step.` |

Render the prompt to `${BASE}/prompts/tester-<UTC-compact>.md` per the 3e two-step audit-file pattern, then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --name "Tester" \
     --description "Writes unit tests per step" \
     --prompt-file ${BASE}/prompts/tester-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<tester-agent-id>` in every subsequent command.

**Verifier spawn prompt (if needed):**

> **Phase 1 exemption**: The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination, and rides as a free-form multi-line cafleet body (same precedent as the Analyzer's question list in the interview workflow). Phase 2 verification reports follow the verb + pointer + `COMMENT(verifier)` schema documented in [the Coordination Protocol section above](#coordination-protocol).

| Slot | Verifier |
|---|---|
| ROLE TITLE | `the Verifier` |
| role-file | `roles/verifier.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code or modify implementation/test files.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` |
| start cue | `Start by reading the design document and discovering available tools. Then wait for the Director to assign your first verification task.` |

Render the prompt to `${BASE}/prompts/verifier-<UTC-compact>.md` per the 3e two-step audit-file pattern, then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --name "Verifier" \
     --description "E2E/integration testing and evidence collection" \
     --prompt-file ${BASE}/prompts/verifier-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<verifier-agent-id>` in every subsequent command.

#### 3f. Verify members are live

```bash
cafleet member list --fleet-id <fleet-id>
```

All spawned members must show `status: active` with a non-null `pane_id`. If any is missing or pending, retry the spawn before proceeding.

See [roles/director.md](roles/director.md) for commit message conventions.

### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For each step in the design document:

#### Phase A: Test Writing

**Skip this phase entirely when the Tester was not spawned** (Programmer-only team composition for config/documentation-only steps). Proceed directly to Phase B and assign the step to the Programmer without a separate test-writing commit.

1. **Assign**: Send the Tester a verb + pointer poke. The Tester reads the step description and specification directly from the design document at the pointer.
   ```bash
   cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --to <tester-agent-id> --text "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Tester's `complete (paragraph-Implementation > Step N) — <count> tests` (or `blocked (paragraph-Implementation > Step N)` if the spec is unclear)** via `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>`. On `blocked`, read the Tester's `COMMENT(tester)` marker at the same pointer (per the pointer-marker pairing rule in the Coordination Protocol section above); if the test framework is ambiguous (per the Tester's `Phase 1` selection step, which uses `blocked (doc)` with the marker at doc-top), ask the user via {decision_surface}, write the answer back as `COMMENT(claude): <choice>` at the same doc-top location, and reply with `ready (doc)` so the Tester resumes.
3. **Review tests** against the design doc. If issues are found, write `COMMENT(director): <issue>` markers at `paragraph-Implementation > Step N` (matching the cafleet pointer per the pointer-marker pairing rule in the Coordination Protocol section above) and reply `ready (paragraph-Implementation > Step N)`; the Tester resolves the markers and replies `addressed (paragraph-Implementation > Step N)`. Repeat until satisfied.
4. **Commit tests** (separate commands, do NOT chain with `&&`). Recover the per-test file list directly via git (`git status` / `git diff --stat` / `git log --name-only`) — the Tester does not embed file lists in cafleet bodies under the verb + pointer schema.
   - `git add <test-files>`
   - `git commit -m "test: add tests for [feature description]"`

#### Phase B: Implementation

1. **Assign**: Send the Programmer a verb + pointer poke. The Programmer reads the step spec at the pointer and locates the Tester's freshly-committed test files via git (`git log <base>..HEAD --name-only -- '**/test_*' '**/tests/**'`); the prior Tester `complete (...) — N tests` summary went Tester → Director, not Tester → Programmer.
   ```bash
   cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --to <programmer-agent-id> --text "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Programmer's `complete (paragraph-Implementation > Step N)`** via `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>`. On `escalating (paragraph-Implementation > Step N)` (suspected test defect), see the **Escalation Protocol (Test Defect)** at the end of Step 4; the rationale lives in a `COMMENT(programmer)` marker at the pointer, not in the cafleet body.
3. **Programmer updates design doc**: Checkboxes, timestamps, and Progress counter.

#### Phase C: Code Review (Director)

1. **Review**: Verify code matches design doc, quality is acceptable, no unnecessary changes.
2. **Feedback loop**: If issues are found, write a `COMMENT(director): <issue>` marker — for design-doc-anchored issues, place it at `paragraph-Implementation > Step N` and send `ready (paragraph-Implementation > Step N)`; for source-anchored issues, place it at `<file>:<line>` and send `ready (<file>:<line>)`. The marker location MUST match the cafleet pointer (per the pointer-marker pairing rule in the Coordination Protocol section above). The Programmer resolves the markers, re-runs tests, and replies `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`). Repeat until satisfied.
3. **Commit implementation** (separate commands, do NOT chain with `&&`). Recover the per-file list via git (`git status` / `git diff --stat <base>..HEAD`):
   - `git add <files> <design-doc>`
   - `git commit -m "feat: [description of what was implemented]"`

Repeat from Phase A for the next step. Always include the design document in the implementation commit.

**Escalation Protocol (Test Defect):** When the Programmer sends `escalating (paragraph-Implementation > Step N)`, the Director reads the design doc paragraph, the Programmer's `COMMENT(programmer)` rationale at that pointer (the marker MUST live at `paragraph-Implementation > Step N` per the pointer-marker pairing rule in the Coordination Protocol section above), and the failing test. The Director then writes a `COMMENT(director): <decision> — <rationale, ≤2 sentences>` marker at the same `paragraph-Implementation > Step N` stating the arbitration outcome, and sends `ready (paragraph-Implementation > Step N)` to whichever member needs to act (Tester to fix the test, or Programmer to adjust the implementation). The recipient acts on the standing markers and replies `addressed (paragraph-Implementation > Step N)`. 3-round limit before escalating to the user.

**On-Demand Verification**: Any member can request verification mid-task via `cafleet message send` to the Director. The Director decides whether to route immediately or defer:

| Route immediately | Defer to Phase D |
|:--|:--|
| User-visible behavior change (UI, CLI output, API response) | Internal refactoring or data model change |
| Integration with external system | Adequately covered by unit tests |
| Behavior difficult to catch with unit tests alone | Verification requires setup from a later step |

### Phase D: Verification (Director) — conditional

**Skip this phase entirely if the Verifier was not spawned.** Proceed directly to Step 5 (User Approval).

If the Verifier was spawned, assign verification:

1. Send the Verifier a verb + pointer poke — the Verifier reads the design document and the completed Implementation paragraphs directly at the pointer:
   ```bash
   cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --to <verifier-agent-id> --text "ready (doc)"
   ```
2. The Verifier discovers tools, executes E2E verification, captures evidence, and writes each fail / suggested-fix as a `COMMENT(verifier): <category> <body>` marker (category = impl bug / test gap / spec issue). Marker location MUST match the cafleet pointer used to report the failure — for per-step `escalating (paragraph-Implementation > Step N)` reports, the paired `COMMENT(verifier)` marker lives at the SAME `paragraph-Implementation > Step N` (per the pointer-marker pairing rule in the Coordination Protocol section above). On overall success the Verifier sends a single `complete (doc)`; on failures the Verifier sends one `escalating (paragraph-Implementation > Step N)` per affected step.
3. **Route failures** by reading the standing `COMMENT(verifier)` markers and dispatching with `ready (paragraph-Implementation > Step N)`: impl-bug markers → Programmer, test-gap markers → Tester, spec-issue markers → Director resolves directly via `COMMENT(director)` arbitration (or escalates to the user via {decision_surface} if a product decision is needed).
4. Re-verify after fixes. Proceed to User Approval when all verifiable criteria pass.

### Step 5: User Approval (Director)

After all TDD steps complete but before finalization, present the implementation to the user for approval.

#### Success Criteria Verification

**Before presenting to the user**, verify the design document's Success Criteria section:

1. Read the `## Success Criteria` section from the design document.
2. For each criterion, verify it is satisfied by inspecting the implementation (grep, read files, run tests as needed).
3. Check off all satisfied criteria in the design document (`- [ ]` → `- [x]`).
4. If any criterion is NOT satisfied, resolve it before proceeding to user approval — route to Programmer or Tester as needed via `cafleet message send`.

This step is **mandatory** and must not be skipped.

#### Change Presentation

1. **Git diff command** for the user to inspect (e.g., `git diff main...HEAD`).
2. **Step-by-step change summary** — concise prose of what changed per step (files modified, key behaviors).

#### Approval Interaction

Use {decision_surface}:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with push, PR creation, Copilot review loop, then finalize | Steps 6 → 7 → 8 |
| 2 | **Scan for COMMENT markers** | Add `COMMENT(name): feedback` markers to the changed source files, then select this option to process them | Scan and process markers (see Revision Loop below) |
| 3 | *(Other — built-in)* | *(Free text input, e.g. "approve but skip PR")* | Interpret user intent (see Revision Loop below). Intent judgment recognises an **approve-local** variant that skips Steps 6 + 7 and jumps straight to Step 8 (local finalize only, no push/PR). Abort intent triggers the Abort Flow. |

See [roles/director.md](roles/director.md) for user interaction rules (COMMENT handling, classification, intent judgment, abort detection).

#### Revision Loop (COMMENT Marker-Based Feedback)

When the user selects "Scan for COMMENT markers": scan changed files for `COMMENT(` markers. Classify by file location (see [roles/director.md](roles/director.md)) and route via the verb + pointer schema:
- Design-doc `COMMENT(...)` markers → Director resolves directly (apply spec change, remove marker; no cafleet route).
- Source-file `COMMENT(...)` markers → `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <programmer-agent-id> --text "ready (<file>:<line>)"`. The Programmer reads the marker at the source pointer, fixes the source, removes the marker, and replies `addressed (<file>:<line>)`.
- Test-file `COMMENT(...)` markers → `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <tester-agent-id> --text "ready (<file>:<line>)"`. The Tester reads, fixes, removes the marker, and replies `addressed (<file>:<line>)`.

After all `COMMENT(...)` markers are resolved and verified, re-present to user.

When the user provides free-form text: interpret intent per [roles/director.md](roles/director.md) rules.

No round limit — the loop continues until the user approves or aborts.

#### Abort Flow

1. Update design document Status to "Aborted", add Changelog entry. Place a `COMMENT(director): aborting — finalize and stand by` marker near the top of the doc body (above the Overview section — `Status:` is bold metadata, not a heading, so it is not a valid `paragraph-` target). Notify any still-live members with a single `cafleet message send --fleet-id <fleet-id> ... --text "ready (doc)"` per member so they read the marker and stand by.
2. Commit (separate commands): `git add <design-doc>` then `git commit -m "docs: mark design doc as aborted"`
3. Follow Shutdown Protocol (Step 8: stop the monitoring member's `monitor start` background task, then delete the monitoring member first and the remaining members, and run `cafleet fleet delete <fleet-id>` to tear down the fleet and sweep the root Director + Administrator).

### Step 6: Push & Create PR (Director)

After Step 5 Approve, the Director pushes the feature branch, opens a PR, and requests a Copilot review BEFORE marking the design doc complete. Every command is run as a separate Bash call — do NOT chain with `&&`.

#### 6a. Preconditions (checked in order; first failure aborts to Step 8 local-finalize)

| Check | Command | Failure action |
|:--|:--|:--|
| `gh` authenticated | `gh auth status` | Report `gh not authenticated; skipping PR creation` → Step 8 local-finalize |
| Not on default branch | `git branch --show-current` vs `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` | Report `on default branch; cannot open PR` → Step 8 local-finalize |
| Branch has commits beyond base | `git log <base>..HEAD --oneline` | Report `no commits to push` → Step 8 local-finalize |

#### 6b. Procedure

1. **Resolve owner/repo**: `gh repo view --json nameWithOwner --jq '.nameWithOwner'`. Capture the literal `<owner>/<repo>` string (e.g. `himkt/cafleet`) and substitute it into every `gh api repos/<owner>/<repo>/...` call below. Like the PR number, this is a literal string — NO shell variables.
2. **Initial push**: `git push -u origin <branch-name>`. If this fails (non-fast-forward, branch protection, etc.), report the exact stderr to the user and proceed to Step 8 local-finalize. NEVER force-push.
3. **Check for an existing PR on this branch**: `gh pr list --head <branch-name> --json number --jq '.[0].number // empty'`. If the result is non-empty, reuse that PR number. Otherwise, run `gh pr create --fill` and parse the printed URL's trailing number.
4. **Record PR number literally**: store the PR number (e.g. `42`) and substitute it into `<pr-number>` in every subsequent command. DO NOT use a shell variable — `permissions.allow` matches literal command strings.
5. **Request Copilot review**: `gh pr edit <pr-number> --add-reviewer @copilot`.
6. **Verify the review request**: `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers` should list Copilot. If Copilot is absent from the response AND no Copilot review already exists (`gh pr view <pr-number> --json reviews`), report `Copilot reviewer unavailable for this PR` and proceed to Step 8 local-finalize.
7. **Capture `last_push_ts`**: record the ISO 8601 timestamp of the push completion (the Director's wall-clock time captured immediately after step 2 returned, or `date -u +%Y-%m-%dT%H:%M:%SZ`). This initialises the in-context loop state described in the "PR Review Loop State" subsection below.

### Step 7: Copilot Review Loop (Director)

Once the PR exists and Copilot has been invited, the Director runs a Copilot review loop. The monitoring member runs **unchanged** — there is no scheduler swap. While Step 7 is active, the Director simply **adds the PR-review poll to what it does on each idle-nudge-driven turn**, on top of its normal team-health facilitation. The "loop" here is the logical poll → route → fix → push → re-poll cycle the Director drives; the turn that drives each pass is the monitoring member's periodic idle-nudge (the monitoring member finds the Director idle-while-awaiting-Copilot and nudges it, granting a re-poll turn). Copilot is an *external* reviewer that never fires a broker inline-preview into the Director's pane, so this idle-nudge is the loop's turn source.

#### Termination authority

Once the loop is active (the PR exists and Copilot has been invited), authority to end it rests solely with the Administrator (the user). The loop ends on exactly two conditions: (1) the user instructs termination (§ User Interjection During Step 7), or (2) a post-push Copilot no-concerns signal arrives — a `reviews` entry with `state == "APPROVED"`, or a Copilot review/comment whose body indicates no remaining concerns even when `state == "COMMENTED"`. In every other state the Director keeps the loop turning: it waits while a Copilot review is pending, and it autonomously re-requests the review (7e) when a prior request failed to land. The Step 6a preconditions and the initial push / PR-create failures are pre-loop fallbacks that skip Step 7 entirely — they are distinct from ending an active loop.

#### PR Review Loop State

The Director holds two **PR-review-specific** in-context variables across idle-nudge-driven turns (separate from the team-health inbox poll the monitoring loop runs via `cafleet message poll` (per the `cafleet` skill's `reference/supervision.md`), which returns only un-acked deliveries and tracks no timestamp). They are NOT persisted to disk — the Director carries them in its own working memory.

| Variable | Meaning | Update rule |
|:--|:--|:--|
| `last_push_ts` | ISO 8601 timestamp of the most recent push to the PR branch | Reset on every `git push` from 6b-step 2 or 7d-step 3 |
| `silence_ticks` | Consecutive Director turns (driven by the monitoring member's idle nudge) with 0 new Copilot items since the last activity | Increment each turn with 0 new items; reset to 0 when new Copilot items arrive, after a fix-push from 7d, OR after the 7e autonomous re-request |

#### 7a. Add PR-review polling to each idle-nudge-driven turn

The monitoring member's `cafleet monitor` (started in Step 3b) runs unchanged on entry and exit; the Director simply adds the **7b per-turn procedure** (team health + PR-review poll) to each idle-nudge-driven turn, and drops it after exit (Step 8's shutdown stops the monitor).

#### 7b. Per-turn procedure

On each idle-nudge-driven turn (and in any active turn while Step 7 is in progress), the Director runs — in order:

1. **Team health** (unchanged from the `cafleet` skill's `reference/supervision.md`): `member list` → `poll` → `member capture` fallback → nudge stalled members.
2. **Fetch new PR reviews**: `gh pr view <pr-number> --json reviews` (GraphQL-shaped; fields are `author.login`, `state`, `submittedAt`, `body`) AND `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments` (REST-shaped; fields are `user.login`, `body`, `path`, `line`, `created_at`).
3. **Filter Copilot-authored entries**: keep items where the login field (`author.login` for `gh pr view` reviews, `user.login` for `gh api` inline comments) matches the regex `^copilot` (case-insensitive). Copilot reviews currently post under a login that begins with `copilot` — the exact slug varies by account plan, so a prefix match is the safe filter.
4. **New-since-push filter**: keep items whose timestamp (`submittedAt` for reviews, `created_at` for inline comments) is strictly later than `last_push_ts`.
5. **Branch on the filter result**:

Evaluate top-down; the first matching row wins (a post-push no-concerns signal matches row 1 before the general new-items row):

| Result | Action |
|:--|:--|
| A **post-push** Copilot no-concerns signal — a `reviews` entry with `state == "APPROVED"`, OR a Copilot review/comment whose body indicates no remaining concerns (even when `state == "COMMENTED"`) | Exit loop (success) → Step 8 |
| ≥ 1 new Copilot items | Reset `silence_ticks = 0`, go to 7c |
| 0 new Copilot items AND `silence_ticks < 30` | Increment `silence_ticks`, keep waiting |
| 0 new Copilot items AND `silence_ticks >= 30` | Run the 7e autonomous re-request check, reset `silence_ticks = 0`, keep waiting |

The no-concerns exit MUST be qualified by the post-push filter (`submittedAt > last_push_ts` for reviews, `created_at > last_push_ts` for comments): only a Copilot signal newer than the most recent fix-push clears the current HEAD. An older approval or no-concerns note reflects a previous revision and leaves the loop running.

**Silence keeps the loop turning.** A silent Copilot is a pending review, not completion. On prolonged silence the Director autonomously re-requests the review (7e) and continues; the loop ends only on the two termination conditions above — the user instructs termination, or a post-push Copilot no-concerns signal arrives.

**Read `reviews`, not `reviewDecision`**: `reviewDecision` only reflects required reviewers (CODEOWNERS); Copilot usually is not one, so its approve leaves `reviewDecision` null — the Copilot-specific entry in the `reviews` array is the reliable signal.

#### 7c. Classify and route

For each new inline comment, pick the owner by file-path pattern. **Source-anchored** Copilot lines route via the verb + pointer schema; the Director writes a `COMMENT(copilot): <body>` marker at the source pointer (because that is where the comment lives) and pokes the routed member with `ready (<file>:<line>)`. **Design-doc-anchored** Copilot lines do NOT route to a member — the Director writes a `COMMENT(director): <body>` marker at the affected paragraph, applies the spec change, and removes the marker as part of the fix; no cafleet message is sent (the git commit + marker removal is sufficient audit trail).

| Path pattern | Owner | Marker location | Route |
|:--|:--|:--|:--|
| Design doc (`design-docs/**/design-doc.md`) | Director | `COMMENT(director): <body>` at the affected paragraph in the design doc | (no cafleet route — Director resolves silently) |
| Test file (e.g. `**/test_*.py`, `**/*_test.py`, `**/tests/**`) | Tester | `COMMENT(copilot): <body>` in the test file at `<file>:<line>` | `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <tester-agent-id> --text "ready (<file>:<line>)"` |
| Any other source file | Programmer | `COMMENT(copilot): <body>` in the source file at `<file>:<line>` | `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <programmer-agent-id> --text "ready (<file>:<line>)"` |

The routed member fixes the source, removes the `COMMENT(copilot)` marker as part of the fix, and replies `addressed (<file>:<line>)`.

For review-level comments (body text not attached to a specific line), route by Director judgment: spec-level → `COMMENT(director)` in design doc, Director resolves directly; implementation-level → `COMMENT(copilot)` at a representative source `<file>:<line>` + `ready (<file>:<line>)` to the Programmer; test-level → `COMMENT(copilot)` at a representative test `<file>:<line>` + `ready (<file>:<line>)` to the Tester.

#### 7d. Fix, commit, push, re-request

1. Wait for each routed member to report completion via `cafleet message poll`. Members do NOT commit — the Director commits after each report.
2. Commit fixes per scope (each `git add` / `git commit` is its own Bash call, no `&&`):
   - Programmer fixes: `git commit -m "fix: address Copilot review - <short summary>"`
   - Tester fixes: `git commit -m "fix: address Copilot test review - <short summary>"`
   - Director doc fixes: `git commit -m "docs: address Copilot review - <short summary>"`
3. `git push` (no flags — the branch already tracks origin from Step 6).
4. Update `last_push_ts` to the post-push wall-clock timestamp and reset `silence_ticks = 0` (the new push restarts the review window).
5. Re-request Copilot review: `gh pr edit <pr-number> --add-reviewer @copilot`. Re-adding the same reviewer triggers a fresh Copilot pass.
6. Continue the loop.

#### 7e. Silence handling — autonomous re-request

When `silence_ticks >= 30` (≈ 90 min since the last Copilot activity AND no new items this turn), the Director re-requests the review on its own — no user prompt. Authority to end the loop stays with the Administrator (§ Termination authority); silence is a pending review, so the Director keeps it turning:

1. **Detect the request state** via `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers`. Reaching 7e means 0 new post-push Copilot items this turn, so Copilot's absence here means the request failed to land:
   - **Copilot present** → the request landed and the review is pending; reset `silence_ticks = 0` and keep waiting.
   - **Copilot absent** → the request failed to land; re-request with `gh pr edit <pr-number> --add-reviewer @copilot`, confirm Copilot now appears in `requested_reviewers`, reset `silence_ticks = 0`, and keep waiting.
2. The user may terminate at any time via the "stop means stop" halt (§ User Interjection During Step 7) — that is the one path that ends the loop short of a Copilot no-concerns signal.

The 30-tick patience window keeps the Director from re-requesting every tick; Copilot's first review after a `--add-reviewer` typically lands within 3–5 minutes.

#### Error Handling (Steps 6–7)

The three Step 6a precondition failures (`gh auth status` fails / on default branch / no commits beyond base) all skip Steps 6 + 7 → Step 8 local-finalize (see 6a). The remaining cases:

| Case | Detection | Behavior |
|:--|:--|:--|
| `git push` rejected | stderr of `git push` | Report exact stderr to user, skip Step 7, go to Step 8 local-finalize. NEVER force-push. |
| `gh pr create` fails | stderr of `gh pr create` | Report, skip Step 7, go to Step 8 local-finalize |
| `@copilot` reviewer unavailable | `gh api .../requested_reviewers` shows no Copilot AND no prior Copilot review | Report `Copilot reviewer unavailable for this PR`; skip Step 7; go to Step 8 |
| Fix-push fails mid-loop (any subsequent push after the initial one) | stderr of `git push` | Escalate to user (decision surface: retry / finalize now / abort) |
| User provides free-form text in Step 5 with abort-intent | Existing LLM intent judgment | Abort Flow (unchanged — no push) |
| User provides free-form text in Step 5 with approve-local intent | Existing LLM intent judgment, extended | Skip Steps 6 + 7; go to Step 8 local-finalize |

#### User Interjection During Step 7

The monitoring member's idle-nudges keep arriving while the user is speaking to the Director. **Stop means stop**: when the user signals halt (explicit "stop", "wait", "pause", profanity / frustration, or repeated rejection of tool calls), the Director MUST halt dispatch immediately and wait for explicit re-authorization — the monitoring member's idle-nudges and idle notifications during the halted state are NOT instructions and must be skipped silently. Concretely, the Director:

1. Stops dispatching new `cafleet message send` / `git commit` / `git push` / `gh` actions immediately.
2. Acknowledges the user briefly and waits for explicit instructions.
3. Treats subsequent idle-nudge-driven turns as notification-only — runs the PR review poll for situational awareness but does NOT route comments, commit, or push until the user re-engages with a specific instruction.
4. Does NOT silently tear the team down — the state stays paused so the user can resume or explicitly abort.

If the user explicitly aborts, follow the Abort Flow (update doc Status → "Aborted", commit, run Shutdown Protocol). Step 7's cleanup is identical to Step 8's cleanup — stop the monitoring member's `monitor start` background task, delete members (monitoring member first), run `cafleet fleet delete`.

### Step 8: Finalize & Clean Up (Director)

Runs after Step 7 exits, or directly after Step 5 when Step 6 was skipped (gh not authenticated / default branch / no commits / approve-local intent).

1. Update design document Status to "Complete" and add a final Changelog entry.
2. `git add <design-doc>` (separate Bash call).
3. `git commit -m "docs: mark design doc as complete"` (separate Bash call).
4. **Push decision** (separate Bash call): run `git rev-parse --abbrev-ref <branch-name>@{upstream}`.
   - Exit code 0 (branch is tracked on origin): `git push`. Covers both the "Step 6 fully succeeded" path and the "Step 6 partial-fail (push OK, PR create failed)" path.
   - Non-zero exit: skip the push. The docs commit stays local.
   - The Director does NOT re-request Copilot review on this final docs commit.
5. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol* (first-out): stop the monitoring member's `monitor start` background task (launched in Step 3b, ran unchanged through Step 7) and wait for confirmation; `cafleet member delete` the monitoring member first, then Programmer, Tester, and Verifier if spawned (on exit 2 use `member capture` + your overlay's decision-prompt recovery or `--force`); `cafleet member list` to verify the roster is empty; `cafleet fleet delete <fleet-id>`; `cafleet fleet list` to confirm.
6. **Report to the user**: include the PR URL (if Step 6 created one), the Copilot loop exit reason (no-concerns / user-terminated / skipped / aborted), and any skipped-step reasons.
