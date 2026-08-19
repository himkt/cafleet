# Design Doc Execute (CAFleet Edition)

Implement features based on a design document using up to five roles orchestrated via the CAFleet message broker: Director (orchestrator), Programmer (implements), Tester (writes tests), Verifier (E2E/integration testing), and Reviewer (fresh post-implementation review). Every inter-member message is persisted in SQLite and auditable. The Director judges which members to spawn from the nature of the implementation tasks and drives a per-step TDD cycle (Tester writes unit tests → Director reviews → Programmer implements → Director reviews and commits). After all TDD steps: Phase D E2E verification (if the Verifier was spawned), the Step 5 fresh-Reviewer loop until approval, Step 6 user approval, Step 7 push + PR, Step 8 finalize and teardown.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`../../cafleet/reference/coding-agent-overlays.md#<name>`](../../cafleet/reference/coding-agent-overlays.md) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — the failure modes *Resolve your overlay* closes, e.g. a literal `{bg_run}` emitted unresolved |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root every spawn-prompt audit file or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (the monitor-first spawn, the `monitor live` gate, Authorization-Scope Guard, the facilitation loop) — you spawn an unsupervised team |
| 4 | [`../reference/coordination.md`](../reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema — you coordinate in free-form bodies and findings get mis-routed |

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main agent | Register with CAFleet, spawn members via `cafleet member create`, validate doc, assign steps, review tests against design doc, review implementation code for quality and compliance, commit after each phase, escalation arbitration, orchestrate TDD cycle | Write code, write tests | [roles/director.md](roles/director.md) |
| **Programmer** | Member | Implement code to pass tests, run tests, report results via `cafleet message send`, escalate test defects to Director, update design doc checkboxes and Progress counter | Write or modify tests, commit code, communicate with user directly | [roles/programmer.md](roles/programmer.md) |
| **Tester** | Member | Read design doc, write unit tests per step, fix tests based on Director feedback, report to Director via `cafleet message send` | Write implementation code, commit code, communicate with user directly | [roles/tester.md](roles/tester.md) |
| **Verifier** | Member (optional) | E2E/integration testing, tool discovery, evidence collection (screenshots, logs, output), failure reporting with suggested fixes | Write code, write tests, commit, communicate with user directly | [roles/verifier.md](roles/verifier.md) |
| **Reviewer** | Member (spawned at Step 5 only) | Fresh post-implementation review: read the design doc and the full branch diff, run read-only mise checks to verify claims, write `COMMENT(reviewer): [TAG]` markers, signal `complete (doc) — N issues` / `approved (doc)` | Write or modify implementation or test code, commit, communicate with user directly | [roles/reviewer.md](roles/reviewer.md) |

## Coordination Protocol

This skill's Director, Programmer, Tester, Verifier, and Reviewer coordinate via the verb + pointer schema and `COMMENT(role)` markers defined canonically in [../reference/coordination.md](../reference/coordination.md) — the single source of truth for the 6 verbs, the 3 pointer forms, the message format, the `COMMENT(role)` marker grammar, the issue/status split, anchorless status, finalize-time cleanup, and Director per-file detail recovery.

Two skill-specific notes layer on top of that canonical protocol:

- **Roles in play**: this skill uses only the `director`, `programmer`, `tester`, `verifier`, `reviewer`, and `user-relay` marker roles — never `drafter` (that belongs to the create workflow). Finalize happens at `Status: Complete` (Step 8).
- **Verifier Phase 1 exemption**: The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination, and rides as a free-form multi-line cafleet body (same precedent as the Analyzer's question list in the interview workflow). Phase 2 verification reports follow the schema.

## Prerequisites

- The Director MUST be running inside a tmux or herdr session and pass the gating `cafleet doctor` env-check before spawning anyone, per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol*.
- `gh` must be authenticated for the Step 7 push / PR creation. Lack of auth is NOT fatal — the Director checks `gh auth status` at Step 7a and falls back to Step 8 local-finalize, skipping the PR. All other prerequisites (tmux, approved design doc, feature branch) remain unchanged.

## Process

**Run to completion.** Once the execute workflow is invoked, the fleet operates autonomously and collaboratively through every task in the design document. The Director keeps driving the team — dispatching the next step to each idle member the moment it is ready — until all Implementation tasks and Success Criteria are complete. The designed checkpoints stay in force: the Step 6 user-approval gate, the user's "stop means stop" halt during the Step 5 Reviewer review loop, and escalations that require a genuinely new user decision.

### Step 1: Resolve Design Document Path (Director)

Before validation, resolve `$ARGUMENTS` into a concrete `design-doc.md` path.

#### Phase 1: Base Directory Resolution

Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above). Then resolve BASE based on whether `$ARGUMENTS` was supplied:

- **`$ARGUMENTS` present** (the typical execute-a-specific-doc flow): canonicalize `$ARGUMENTS` and call the task-scope resolver positionally. `$ARGUMENTS` is normally a slug name (`0000060-skill-task-scoped-base-dir`) or a path containing such a slug.

  Canonicalize `$ARGUMENTS` per the `cafleet` skill's `reference/base-dir.md` § *Consumer contract* row for this skill (relative forms get `design-docs/` prepended and a trailing `/design-doc.md` stripped; absolute paths are used verbatim after the filename strip), then run its **Step 0 (task-scope resolution)** with the result.

  Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder (the slug folder) and `${RESOLVED_ARGS} = ${BASE}/design-doc.md` (short-circuits at Tier 1 below). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${RESOLVED_ARGS}` to the literal `$ARGUMENTS` path so Tier 1 / Tier 2 still run against the user-supplied path, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.

- **`$ARGUMENTS` absent** (the discover-all-approved-docs flow): the no-argument form scans `<repo-root>/design-docs/`, so the Director MUST invoke from the repo root. Verify with `git rev-parse --show-toplevel` and abort with a clear "invoke from the repo root" error if `cwd` differs. Then run the skill's **Step 1 (shared-root resolution)**:

  Step 1 resolves `${BASE}` to the CWD (the verified repo root). In the rare edge case where the repo root is itself `$HOME` or under a coding agent's user-level config directory (per base-dir.md's table), Step 1 reaches **Step 2** of base-dir resolution (its decision-surface prompt); there, explicitly choose the `${CWD}` candidate so `${BASE}` stays the verified repo root — do NOT pick `/tmp/cafleet`, which would make `${RESOLVED_ARGS} = /tmp/cafleet/design-docs/` and point the discovery scan at the wrong directory. With `${BASE}` resolved to the repo root, set `${RESOLVED_ARGS} = ${BASE}/design-docs/` — this matches Tier 3 below and engages the discovery flow that scans every approved slug under `<repo>/design-docs/`.

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
3. Check for `FIXME(agent)` markers in the codebase using Grep. If found, note them for the Programmer to resolve first.
4. Determine the step order and total number of steps.
5. **Create a feature branch if on the default branch.** Get the default branch with `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` and the current branch with `git branch --show-current`. If they match, use {decision_surface} to propose the branch name `feat/<design-doc-slug>` and ask the user to approve before creating it. The user will create the branch themselves or approve the proposed name. If already on a non-default branch, skip this step.

### Step 3: Register & Spawn Members (Director)

Load the `cafleet` skill; its `reference/supervision.md` governance is § Required reading above.

#### 3a. Establish a CAFleet fleet (monitor included) and capture the ids

Bootstrap the fleet per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Fleet bootstrap (monitor included)* (write the monitor's spawn prompt first and pass it via `--monitor-file`; the reuse-a-running-fleet rule is there too). Use `--json` so the IDs are machine-parseable:

```bash
cafleet fleet create --name "design-doc-execute-{slug}" --coding-agent <backend> --monitor-file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md> --monitor-model {monitor_model} --json
# → { "fleet_id": <int>, "director": { "member_id": <int>, ... }, "monitor": { "member_id": <int>, ... } }
```

Capture `fleet_id` and `director.member_id` from the JSON response and substitute them for `<fleet-id>` and `<director-member-id>` in every subsequent command.

#### 3b. Wait for the monitor gate (before any ordinary member)

Wait for the monitor member's `ready` then `monitor live` signals per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Wait for the monitor gate* — `monitor live` gates the first ordinary `member create`. The monitor member runs unchanged through Steps 3–8 and is deleted first (first-out) in Step 8's cleanup.

#### 3c. Analyze implementation tasks to decide team composition

Decide the team composition from the design document steps per the full decision matrix in [roles/director.md](roles/director.md).

The Reviewer is **never** part of the initial team composition — it is spawned fresh at Step 5 only, after every Implementation task and Success Criterion is complete.

#### 3d. Read role files

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file via `Read` on its first turn — do NOT inline the content):

- `skills/cafleet-design-doc/execute/roles/programmer.md`
- `skills/cafleet-design-doc/execute/roles/tester.md` (if Tester needed)
- `skills/cafleet-design-doc/execute/roles/verifier.md` (if Verifier needed)

#### 3e. Spawn each member via `cafleet member create`

Each member is spawned from the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the per-role delta below (two-stage rendering + brace rules at the skeleton). All three roles load `cafleet` + `cafleet-design-doc` and take `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` as their only context line; each delta below gives the role's title, role-file, IMPORTANT lines (verbatim), and start cue.

> **Spawn frame (two-step pattern)**: render each spawn prompt to `${BASE}/.prompts/<role>-<UTC-compact>.md` per the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol*, spawn with `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --file <abs path> --json`, and parse `member_id` from the JSON response, substituting it for that role's `<x-member-id>` in every subsequent command.

**Programmer spawn prompt** (skeleton + delta):

| Slot | Programmer |
|---|---|
| ROLE TITLE | `the Programmer` |
| role-file | `roles/programmer.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.` |
| start cue | `Read the design document and wait for the Director to assign your first step.` |

Spawn per the 3e spawn frame. Worked example — the one full command block of this file; the Tester, Verifier, and Step-5 Reviewer spawns reuse the frame with their own literals:

   ```bash
   cafleet member create --fleet-id <fleet-id> \
     --name "Programmer" \
     --description "Implements code to pass tests per step" \
     --file ${BASE}/.prompts/programmer-<UTC-compact>.md \
     --json
   ```

**Tester spawn prompt** (skeleton + delta; if needed):

| Slot | Tester |
|---|---|
| ROLE TITLE | `the Tester` |
| role-file | `roles/tester.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` / `IMPORTANT: Do NOT write implementation code — only test code.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.` |
| start cue | `Read the design document and wait for the Director to assign your first step.` |
| `--name` / `--description` | `Tester` / `Writes unit tests per step` |

Spawn per the 3e spawn frame (audit file `${BASE}/.prompts/tester-<UTC-compact>.md`).

**Verifier spawn prompt (if needed):**

> **Phase 1 exemption** ([Coordination Protocol above](#coordination-protocol)): the Verifier's first message — the tool-and-MCP inventory — rides as a free-form multi-line cafleet body; Phase 2 verification reports follow the schema.

| Slot | Verifier |
|---|---|
| ROLE TITLE | `the Verifier` |
| role-file | `roles/verifier.md` |
| IMPORTANT (verbatim) | `IMPORTANT: Do NOT commit code or modify implementation/test files.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.` |
| start cue | `Read the design document and discover available tools. Wait for the Director to assign your first verification task.` |
| `--name` / `--description` | `Verifier` / `E2E/integration testing and evidence collection` |

Spawn per the 3e spawn frame (audit file `${BASE}/.prompts/verifier-<UTC-compact>.md`).

#### 3f. Spawn-health placement audit (non-gating)

```bash
cafleet member list <fleet-id>
```

Placement-audit semantics — non-gating, retry a missing or pending row, dispatch rides each member's ready signal — per [`supervision.md`](../../cafleet/reference/supervision.md) § *Spawn Protocol*.

See [roles/director.md](roles/director.md) for commit message conventions.

### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For each step in the design document:

#### Phase A: Test Writing

**Skip this phase entirely when the Tester was not spawned** (Programmer-only team composition for config/documentation-only steps). Proceed directly to Phase B and assign the step to the Programmer without a separate test-writing commit.

1. **Assign**: Send the Tester a verb + pointer poke. Fire this first dispatch on the Tester's own ready signal — its input, the approved design doc, already exists — never on full-team placement (dispatch-on-ready, § 3f). The Tester reads the step description and specification directly from the design document at the pointer.
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <tester-member-id> "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Tester's `complete (paragraph-Implementation > Step N) — <count> tests` (or `blocked (paragraph-Implementation > Step N)` if the spec is unclear)** via `cafleet message poll <director-member-id>`. On `blocked`, read the Tester's `COMMENT(tester)` marker at the same pointer (pairing rule, coordination.md); if the test framework is ambiguous (per the Tester's `Phase 1` selection step, which uses `blocked (doc)` with the marker at doc-top), ask the user via {decision_surface}, write the answer back as `COMMENT(user-relay): <choice>` at the same doc-top location, and reply with `ready (doc)` so the Tester resumes.
3. **Review tests** against the design doc. If issues are found, write `COMMENT(director): <issue>` markers at `paragraph-Implementation > Step N` (pairing rule) and reply `ready (paragraph-Implementation > Step N)`; the Tester resolves the markers and replies `addressed (paragraph-Implementation > Step N)`. Repeat until satisfied.
4. **Commit tests** per the Commit Protocol ([roles/director.md](roles/director.md)). Recover the per-test file list directly via git (`git status` / `git diff --stat` / `git log --name-only`) — the Tester does not embed file lists in cafleet bodies under the verb + pointer schema.
   - `git add <test-files>`
   - `git commit -m "test: add tests for [feature description]"`

#### Phase B: Implementation

1. **Assign**: Send the Programmer a verb + pointer poke. The Programmer reads the step spec at the pointer and locates the Tester's freshly-committed test files via git, per its role file.
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <programmer-member-id> "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Programmer's `complete (paragraph-Implementation > Step N)`** via `cafleet message poll <director-member-id>`. On `escalating (paragraph-Implementation > Step N)` (suspected test defect), see the **Escalation Protocol (Test Defect)** at the end of Step 4; the rationale lives in a `COMMENT(programmer)` marker at the pointer, not in the cafleet body.
3. **Programmer updates design doc**: Checkboxes, timestamps, and Progress counter.

#### Phase C: Code Review (Director)

1. **Review**: Verify code matches design doc, quality is acceptable, no unnecessary changes.
2. **Feedback loop**: If issues are found, write a `COMMENT(director): <issue>` marker — for design-doc-anchored issues, place it at `paragraph-Implementation > Step N` and send `ready (paragraph-Implementation > Step N)`; for source-anchored issues, place it at `<file>:<line>` and send `ready (<file>:<line>)` (pairing rule, coordination.md). The Programmer resolves the markers, re-runs tests, and replies `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`). Repeat until satisfied.
3. **Commit implementation** per the Commit Protocol ([roles/director.md](roles/director.md)). Recover the per-file list via git (`git status` / `git diff --stat <base>..HEAD`):
   - `git add <files> <design-doc>`
   - `git commit -m "feat: [description of what was implemented]"`

Repeat from Phase A for the next step. Always include the design document in the implementation commit.

**Escalation Protocol (Test Defect):** When the Programmer sends `escalating (paragraph-Implementation > Step N)`, the Director reads the design doc paragraph, the Programmer's `COMMENT(programmer)` rationale at that pointer (pairing rule, coordination.md), and the failing test. The Director then writes a `COMMENT(director): <decision> — <rationale, ≤2 sentences>` marker at the same `paragraph-Implementation > Step N` stating the arbitration outcome, and sends `ready (paragraph-Implementation > Step N)` to whichever member needs to act (Tester to fix the test, or Programmer to adjust the implementation). The recipient acts on the standing markers and replies `addressed (paragraph-Implementation > Step N)`. 3-round limit before escalating to the user.

**On-Demand Verification**: Any member can request verification mid-task via `cafleet message send` to the Director. The Director decides whether to route immediately or defer:

| Route immediately | Defer to Phase D |
|:--|:--|
| User-visible behavior change (UI, CLI output, API response) | Internal refactoring or data model change |
| Integration with external system | Adequately covered by unit tests |
| Behavior difficult to catch with unit tests alone | Verification requires setup from a later step |

### Phase D: Verification (Director) — conditional

**Skip this phase entirely if the Verifier was not spawned.** Proceed directly to Step 5 (Reviewer Review Loop).

If the Verifier was spawned, assign verification:

1. Send the Verifier a verb + pointer poke — the Verifier reads the design document and the completed Implementation paragraphs directly at the pointer:
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <verifier-member-id> "ready (doc)"
   ```
2. The Verifier discovers tools, executes E2E verification, captures evidence, and writes each fail / suggested-fix as a `COMMENT(verifier): <category> <body>` marker (category = impl bug / test gap / spec issue) at the pointer used to report the failure (pairing rule, coordination.md). On overall success the Verifier sends a single `complete (doc)`; on failures the Verifier sends one `escalating (paragraph-Implementation > Step N)` per affected step.
3. **Route failures** by reading the standing `COMMENT(verifier)` markers and dispatching with `ready (paragraph-Implementation > Step N)`: impl-bug markers → Programmer, test-gap markers → Tester, spec-issue markers → Director resolves directly via `COMMENT(director)` arbitration (or escalates to the user via {decision_surface} if a product decision is needed).
4. Re-verify after fixes. Proceed to Step 5 (Reviewer Review Loop) when all verifiable criteria pass.

### Step 5: Reviewer Review Loop (Director)

After all TDD steps (and Phase D, if run) complete, the Director runs a fresh-context review loop before anything is presented to the user. **Run to completion / stop means stop**: the loop is uncapped and ends only on Reviewer approval or an explicit user halt/abort. When the user signals halt (explicit "stop", "wait", "pause", profanity / frustration, or repeated rejection of tool calls), the Director halts dispatch immediately, treats monitor pings and event messages as notification-only, and waits for explicit re-authorization; an explicit abort triggers the Abort Flow (Step 6).

#### Success Criteria Verification (gate)

The Reviewer is spawned only when every Implementation task is checked (`- [x]`) and Phase D (if run) passed. Before spawning, verify the design document's Success Criteria section:

1. Read the `## Success Criteria` section from the design document.
2. For each criterion, verify it is satisfied by inspecting the implementation (grep, read files, run tests as needed).
3. Check off all satisfied criteria in the design document (`- [ ]` → `- [x]`).
4. If any criterion is NOT satisfied, resolve it before proceeding — route to Programmer or Tester as needed via `cafleet message send`.

This verification is **mandatory** and must not be skipped. Only when every criterion is checked does the Director spawn the Reviewer — this is the "all design-doc tasks finished" trigger.

#### Spawn the fresh Reviewer

This is the first and only time the Reviewer exists in the fleet (never in the Step 3 initial composition), so its context holds no memory of the implementation's compromises.

**Reviewer spawn prompt** (built from the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton), like the 3e roles):

| Slot | Reviewer |
|---|---|
| ROLE TITLE | `the Reviewer` |
| role-file | `roles/reviewer.md` |
| skill loads | the `cafleet` skill — for communication with the Director; the `cafleet-design-doc` skill — for the coordination protocol and the design-doc format (same pair as the other execute roles per §3e) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` / `BASE BRANCH: [INSERT default branch name from Step 2]` |
| poll-handling line (verbatim) | `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| IMPORTANT (verbatim) | `IMPORTANT: You are a fresh reviewer with no implementation context — judge only what you can verify from the design document, the diff, and the checks you run.` / `IMPORTANT: Do NOT write or modify implementation or test code. Your only edits are COMMENT(reviewer) markers.` / `IMPORTANT: Do NOT commit. The Director handles all git operations.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.` |
| start cue | `Read the design document and the branch diff. Then act on the Director's ready (doc) assignment.` |
| `--name` / `--description` / `--model` | `Reviewer` / `Fresh post-implementation review` / `{reviewer_model}` (your overlay's resolved value) |

Spawn per the 3e spawn frame (audit file `${BASE}/.prompts/reviewer-<UTC-compact>.md`), adding `--model {reviewer_model}`. Verify `status: active` via `cafleet member list <fleet-id>` before assigning.

#### Review loop

1. **Assign**: `cafleet message send --from-member-id <director-member-id> --to-member-id <reviewer-member-id> "ready (doc)"`.
2. **Review pass** (Reviewer): reads the design doc, reads the full branch diff (`git diff <base-branch>...HEAD` — the base branch name is a spawn-prompt context line), and may run `mise //cafleet:test`, `mise //cafleet:lint`, and the other read-only mise tasks to verify claims (read-execute scope). Findings land as `COMMENT(reviewer): [TAG] <body>` markers — source-anchored findings in the source/test file at `<file>:<line>`, spec-level findings at the affected design-doc paragraph. The Reviewer then sends `complete (doc) — N issues`, or `approved (doc)` when no substantive issues remain.
3. **Route** (Director) — by marker location:

   | Marker location | Owner | Route |
   |:--|:--|:--|
   | Test content — an integration test under `cafleet/tests/`, or a finding inside a `#[cfg(test)]` module of a source file | Tester — or the Programmer when no Tester was spawned (Programmer-only composition) | `ready (<file>:<line>)` to `<tester-member-id>` (fallback: `<programmer-member-id>`) |
   | Any other source line — a source file outside its `#[cfg(test)]` module | Programmer | `ready (<file>:<line>)` to `<programmer-member-id>` |
   | Design doc | Director | Resolves directly: apply the spec change, remove the marker; no cafleet route (escalate to the user via {decision_surface} only when a product decision is needed) |

   The routed member fixes the target, removes the `COMMENT(reviewer)` marker as part of the fix, re-runs tests, and replies `addressed (<file>:<line>)`.
4. **Commit** (after all routed fixes report `addressed`; per the Commit Protocol):
   - Programmer fixes: `git commit -m "fix: address Reviewer feedback - <short summary>"`
   - Tester fixes: `git commit -m "fix: address Reviewer test feedback - <short summary>"`
   - Director doc fixes: `git commit -m "docs: address Reviewer feedback - <short summary>"`
5. **Re-review**: send `ready (doc)` to the Reviewer again. Loop 2→5 with **no round cap** until the Reviewer sends `approved (doc)`.

#### Dispute arbitration

When a routed member disputes a finding, it counter-escalates with `escalating (<file>:<line>)` plus a `COMMENT(programmer)` / `COMMENT(tester)` rationale at the same pointer. The Director arbitrates via a `COMMENT(director): <decision> — <rationale>` marker exactly like the Step 4 test-defect protocol, with the same 3-round limit before escalating to the user via {decision_surface}.

### Step 6: User Approval (Director)

After the Reviewer sends `approved (doc)`, present the implementation to the user for approval.

#### Change Presentation

1. **Git diff command** for the user to inspect (e.g., `git diff main...HEAD`).
2. **Step-by-step change summary** — concise prose of what changed per step (files modified, key behaviors).

#### Approval Interaction

Use {decision_surface}:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with push, PR creation, then finalize | Steps 7 → 8 |
| 2 | **Scan for COMMENT markers** | Add `COMMENT(name): feedback` markers to the changed source files, then select this option to process them | Scan and process markers (see Revision Loop below) |
| 3 | *(Other — built-in)* | *(Free text input, e.g. "approve but skip PR")* | Interpret user intent (see Revision Loop below). Intent judgment recognises an **approve-local** variant that skips Step 7 and jumps straight to Step 8 (local finalize only, no push/PR). Abort intent triggers the Abort Flow. |

Intent judgment and abort detection for free-form replies: [roles/director.md](roles/director.md) § *Free-form user replies*.

#### Revision Loop (COMMENT Marker-Based Feedback)

This loop owns the user-feedback COMMENT-scan procedure. When the user selects "Scan for COMMENT markers": scan changed files for `COMMENT(` markers **immediately** — the selection itself is the signal, do NOT wait for the user to confirm they are done editing. Classify by file location and route via the verb + pointer schema:
- Design-doc `COMMENT(...)` markers → Director resolves directly (apply spec change, remove marker; no cafleet route).
- Source-file `COMMENT(...)` markers → `cafleet message send --from-member-id <director-member-id> --to-member-id <programmer-member-id> "ready (<file>:<line>)"`. The Programmer reads the marker at the source pointer, fixes the source, removes the marker, and replies `addressed (<file>:<line>)`.
- Test-file `COMMENT(...)` markers → `cafleet message send --from-member-id <director-member-id> --to-member-id <tester-member-id> "ready (<file>:<line>)"`. The Tester reads, fixes, removes the marker, and replies `addressed (<file>:<line>)`.

If no markers are found: explain the COMMENT marker convention — add `COMMENT(username): feedback` to the relevant source or test files, using the file's native comment syntax as prefix (e.g., `# COMMENT(...)` for Python/Ruby/YAML, `// COMMENT(...)` for JS/TS/Go) — re-display the `git diff` command so the user can review the changes, then re-prompt with the same three-option pattern.

When the user provides free-form text: interpret intent per [roles/director.md](roles/director.md) § *Free-form user replies*.

**Re-review invariant**: after any post-feedback revision round (user `COMMENT(...)` markers or verbal feedback routed to members), the Director routes the revised change back through the Reviewer (`ready (doc)`; loop per Step 5's review loop) and re-presents to the user only after a fresh `approved (doc)`. The Reviewer approves before the admin sees it — always.

No round limit — the loop continues until the user approves or aborts.

#### Abort Flow

1. Update design document Status to "Aborted", add Changelog entry. Place a `COMMENT(director): aborting — finalize and stand by` marker near the top of the doc body (above the Overview section — `Status:` is bold metadata, not a heading, so it is not a valid `paragraph-` target). Notify any still-live members with a single `cafleet message send ... "ready (doc)"` per member so they read the marker and stand by.
2. Commit: `git add <design-doc>` then `git commit -m "docs: mark design doc as aborted"`
3. Follow Shutdown Protocol (Step 8: the canonical teardown per the `cafleet` skill § *Shutdown Protocol*).

### Step 7: Push & Create PR (Director)

After Step 6 Approve, the Director pushes the feature branch and opens a PR BEFORE marking the design doc complete. Nothing waits on the PR after creation. Every command runs per the Commit Protocol's separate-commands rule ([roles/director.md](roles/director.md)).

#### 7a. Preconditions (checked in order; first failure aborts to Step 8 local-finalize)

| Check | Command | Failure action |
|:--|:--|:--|
| `gh` authenticated | `gh auth status` | Report `gh not authenticated; skipping PR creation` → Step 8 local-finalize |
| Not on default branch | `git branch --show-current` vs `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` | Report `on default branch; cannot open PR` → Step 8 local-finalize |
| Branch has commits beyond base | `git log <base>..HEAD --oneline` | Report `no commits to push` → Step 8 local-finalize |

#### 7b. Procedure

1. **Resolve owner/repo**: `gh repo view --json nameWithOwner --jq '.nameWithOwner'`. Capture the literal `<owner>/<repo>` string (e.g. `himkt/cafleet`). Like the PR number, this is a literal string — NO shell variables.
2. **Push**: `git push -u origin <branch-name>`. If this fails (non-fast-forward, branch protection, etc.), report the exact stderr to the user and proceed to Step 8 local-finalize. NEVER force-push.
3. **Check for an existing PR on this branch**: `gh pr list --head <branch-name> --json number --jq '.[0].number // empty'`. If the result is non-empty, reuse that PR number. Otherwise, run `gh pr create --fill` and parse the printed URL's trailing number.
4. **Record PR number literally**: store the PR number (e.g. `42`) for the Step 8 report. DO NOT use a shell variable — `permissions.allow` matches literal command strings.

#### Error Handling (Step 7)

The three Step 7a precondition failures (`gh auth status` fails / on default branch / no commits beyond base) all skip the push + PR → Step 8 local-finalize (see 7a). The remaining cases:

| Case | Detection | Behavior |
|:--|:--|:--|
| `git push` rejected | stderr of `git push` | Report exact stderr to user, go to Step 8 local-finalize. NEVER force-push. |
| `gh pr create` fails | stderr of `gh pr create` | Report, go to Step 8 local-finalize |
| User provides free-form text in Step 6 with abort-intent | Existing LLM intent judgment | Abort Flow (unchanged — no push) |
| User provides free-form text in Step 6 with approve-local intent | Existing LLM intent judgment, extended | Skip Step 7; go to Step 8 local-finalize |

### Step 8: Finalize & Clean Up (Director)

Runs after Step 7 completes, or directly after Step 6 when Step 7 was skipped (gh not authenticated / default branch / no commits / approve-local intent).

1. Update design document Status to "Complete" and add a final Changelog entry.
2. `git add <design-doc>`.
3. `git commit -m "docs: mark design doc as complete"`.
4. **Push decision**: run `git rev-parse --abbrev-ref <branch-name>@{upstream}`.
   - Exit code 0 (branch is tracked on origin): `git push`. Covers both the "Step 7 fully succeeded" path and the "Step 7 partial-fail (push OK, PR create failed)" path.
   - Non-zero exit: skip the push. The docs commit stays local.
5. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol* (the monitor member goes first, first-out). Workflow delta: then delete the Programmer, Tester, Verifier, and Reviewer if spawned.
6. **Report to the user**: include the PR URL (if Step 7 created one), the Reviewer outcome (rounds to approval), and any skipped-step reasons.
