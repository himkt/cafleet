# Design Doc Execute (CAFleet Edition)

Implement features based on a design document using up to five roles orchestrated via the CAFleet message broker: Director (orchestrator), Programmer (implements), Tester (writes tests), Verifier (E2E/integration testing), and Reviewer (fresh post-implementation review). Every inter-member message is persisted in SQLite and auditable. The Director judges which members to spawn from the nature of the implementation tasks and drives a per-step TDD cycle (Tester writes unit tests → Director reviews → Programmer implements → Director reviews and commits). After all TDD steps: Phase D E2E verification (if the Verifier was spawned), the Step 5 fresh-Reviewer loop until approval, Step 6 user approval, Step 7 push + PR, Step 8 finalize and teardown.

Resolve runtime tools from the executing agent's backend. For spawns, read the selected member backend's Model catalog and Role defaults under the CAFleet Director's model-selection policy; validate its effort and launch capabilities there. Monitor bootstrap and recovery inherit the Director's backend. Interpret captured panes using the observed member's backend cues.

## Required reading

Read these prerequisites in order before orchestration. Your `CODING AGENT:` identity selects your executing backend; a main session uses its own identity.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../../cafleet/reference/coding-agents.md) | Resolve local Runtime bindings and bound notes before acting. |
| 2 | [Generic Director role](../../cafleet/roles/director.md) | Before setup; obey its selected-backend, spawn skeleton, audit, size and action-triggered reads. |
| 3 | [Guidelines File Layout](../reference/guidelines.md#file-layout), then [BASE](../../cafleet/reference/base-dir.md) | Before argument normalization and output-root resolution. |
| 4 | [Supervision](../../cafleet/reference/supervision.md) | Before orchestration; monitor bootstrap/live, capture, dispatch and authorization gates. |
| 5 | [Coordination](../reference/coordination.md) | Before messages, payload retrieval or markers. |
| 6 | [Guidelines](../reference/guidelines.md) | Before document-format or phase-completion actions. |

Read [Recovery](../../cafleet/reference/supervision.md#recovery) immediately before recovery and [Shutdown](../../cafleet/reference/supervision.md#shutdown) immediately before teardown. The executing backend owns local tools; selected member backend owns spawn model/capabilities; observed backend owns pane cues.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main agent | Register with CAFleet, spawn members via `cafleet member create`, validate doc, assign steps, review tests against design doc, review implementation code for quality and compliance, commit after each phase, escalation arbitration, orchestrate TDD cycle | Write code, write tests | [Director responsibilities](#director-responsibilities) |
| **Programmer** | Member | Implement code to pass tests, run tests, report results via `cafleet message send`, escalate test defects to Director, update design doc checkboxes and Progress counter | Write or modify tests, commit code, communicate with user directly | [roles/programmer.md](roles/programmer.md) |
| **Tester** | Member | Read design doc, write unit tests per step, fix tests based on Director feedback, report to Director via `cafleet message send` | Write implementation code, commit code, communicate with user directly | [roles/tester.md](roles/tester.md) |
| **Verifier** | Member (optional) | E2E/integration testing, tool discovery, evidence collection (screenshots, logs, output), failure reporting with suggested fixes | Write code, write tests, commit, communicate with user directly | [roles/verifier.md](roles/verifier.md) |
| **Reviewer** | Member (spawned at Step 5 only) | Fresh post-implementation review: read the design doc and the full branch diff, run read-only mise checks to verify claims, write `COMMENT(reviewer): [TAG]` markers, signal `complete (doc) — N issues` / `approved (doc)` | Write or modify implementation or test code, commit, communicate with user directly | [roles/reviewer.md](roles/reviewer.md) |

## Director responsibilities

Own a correct implementation and eligible phase commits. Validate the document and arbitrate unclear/conflicting markers before spawning. Choose only needed roles, drive each step through test review and implementation review, and keep tests and implementation in separate commits. Members own their assigned code/test regions; the Director owns orchestration, specification decisions, reviews and all Git operations.

Drive ready work through completion while preserving user halt/abort signals and new-decision escalations. Run applicable Phase D verification and independently check Success Criteria before spawning the fresh Step 5 Reviewer. Its uncapped review loop precedes user approval. Preserve the user's publication scope through Steps 7–8, then finalize and shut down the monitor first.

Store literal fleet/member IDs from CLI JSON. Poll complete messages with `--json`, ACK consumed deliveries, and route verb/pointer messages with paired issue markers. Coordination owns marker grammar and per-file Git recovery for test, implementation, review-fix and finalization commits.

### Diagnostics and progress

Inbound member replies and monitor events/pings resume the supervision loop. Use `cafleet member capture <member-id> --lines 200` for stalled-member diagnostics. Disclose deletion and re-spawn to the user; never silently replace a member. Apply [generic replacement](../../cafleet/roles/director.md#model-replacement) with this disclosure constraint. Route test-defect arbitration through Step 4 and commit test corrections separately.

### Free-form user replies

Interpret halt, abort, revision, question, approve-local and remote approval by meaning. Step 6 owns feedback and approval routing. Questions receive answers and do not imply approval; revisions return through Step 5 review before presentation.

### Skill-specific milestones

Every Director action below is a re-sent stall-nudge — the recipient interprets it contextually per [coordination](../reference/coordination.md): same target, same expected action.

| Phase | Expected event | Stall indicator | Director action |
|:--|:--|:--|:--|
| Test writing (Phase A) | Tester writes tests for current step | Tester goes idle without reporting test completion | `cafleet message send --from-member-id <director-member-id> --to-member-id <tester-member-id> "ready (paragraph-Implementation > Step N)"` |
| Implementation (Phase B) | Programmer implements code and runs tests | Programmer goes idle without reporting implementation result | `cafleet message send --from-member-id <director-member-id> --to-member-id <programmer-member-id> "ready (paragraph-Implementation > Step N)"` |
| Verification (Phase D) | Verifier performs E2E testing | Verifier goes idle without reporting verification result | `cafleet message send --from-member-id <director-member-id> --to-member-id <verifier-member-id> "ready (doc)"` (the Verifier reads the design doc and the standing `COMMENT(verifier)` markers) |
| Reviewer Review (Step 5) | Reviewer reports `complete (doc) — N issues` or `approved (doc)` | Reviewer goes idle without a report | `cafleet message send --from-member-id <director-member-id> --to-member-id <reviewer-member-id> "ready (doc)"` |
| Escalation | Member responds to escalation | Escalation recipient goes idle without responding | `cafleet message send --from-member-id <director-member-id> --to-member-id <member-id> "ready (paragraph-Implementation > Step N)"` (the standing `COMMENT(director)` arbitration marker carries the issue) |


## Coordination Protocol

This skill's Director, Programmer, Tester, Verifier, and Reviewer coordinate via the verb + pointer schema and `COMMENT(role)` markers defined canonically in [../reference/coordination.md](../reference/coordination.md) — the single source of truth for the 6 verbs, the 3 pointer forms, the message format, the `COMMENT(role)` marker grammar, the issue/status split, anchorless status, finalize-time cleanup, and Director per-file detail recovery.

Use `director`, `programmer`, `tester`, `verifier`, `reviewer` and `user-relay` markers; finalization sets `Status: Complete`. Read [Payload exemptions](../reference/coordination.md#payload-exemptions) before Verifier tool discovery. Receive that first substantive post-ready inventory with `cafleet message poll <director-member-id> --json`, then ACK the complete payload; Phase 2 reports use paired markers.

## Prerequisites

- The Director MUST be running inside a tmux or herdr session and pass the gating `cafleet doctor` env-check before spawning anyone, per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol*.
- `gh` must be authenticated for the Step 7 push / PR creation. Lack of auth is NOT fatal — the Director checks `gh auth status` at Step 7a and falls back to Step 8 local-finalize, skipping the PR. All other prerequisites (tmux, approved design doc, feature branch) remain unchanged.

## Shared spawn deltas

Render ordinary-member prompts from the required [shared frame](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton), with the per-role tables below. Supply absolute installed role and skill paths, the CAFleet load purpose `for communication with the Director`, and the workflow's team name. Every role and mode carries this poll-handling line verbatim:

```text
When you see cafleet message poll output with a message from the Director, act on those instructions.
```

The shared frame supplies identity placeholders, reader/startup ordering, complete backend-supported skill loading and the supplied host-rule source. Preserve each table's hard lines and start cue verbatim. The role opens first, sends operational ready and loads its prerequisites before substantive work. Selected roles load both `cafleet` and `cafleet-design-doc`; members continue this assigned workflow. The monitor retains its own startup delta.

## Process

**Run to completion.** Once the execute workflow is invoked, the fleet operates autonomously and collaboratively through every task in the design document. The Director keeps driving the team — dispatching the next step to each idle member the moment it is ready — until all Implementation tasks and Success Criteria are complete. The designed checkpoints stay in force: the Step 6 user-approval gate, the user's "stop means stop" halt during the Step 5 Reviewer review loop, and escalations that require a genuinely new user decision.

### Step 1: Resolve Design Document Path (Director)

Before validation, resolve `$ARGUMENTS` into a concrete `design-doc.md` path.

#### Phase 1: Base Directory Resolution

Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above). Then resolve BASE based on whether `$ARGUMENTS` was supplied:

- **`$ARGUMENTS` present** (the typical execute-a-specific-doc flow): canonicalize `$ARGUMENTS` and call the task-scope resolver positionally. `$ARGUMENTS` is normally a slug name (`0000060-skill-task-scoped-base-dir`) or a path containing such a slug.

  Read and apply [Guidelines File Layout](../reference/guidelines.md#file-layout) to normalize the task-folder path, then run [BASE Step 0](../../cafleet/reference/base-dir.md#step-0-task-scope-resolution) on that folder.

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
2. Scan for `COMMENT(` markers. Resolve clear requests directly and remove their markers; when a request is ambiguous, conflicts with the design or requires a product decision, ask the user through {decision_surface} before resolving it. Confirm marker absence before proceeding.
3. Check for `FIXME(agent)` markers in the codebase using Grep. If found, note them for the Programmer to resolve first.
4. Determine the step order and total number of steps.
5. **Create a feature branch if on the default branch.** Get the default branch with `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` and the current branch with `git branch --show-current`. If they match, use {decision_surface} to propose the branch name `feat/<design-doc-slug>` and ask the user to approve before creating it. The user will create the branch themselves or approve the proposed name. If already on a non-default branch, skip this step.

### Team composition

| Task profile | Initial composition |
|---|---|
| Code implementation | Programmer + Tester, using per-step TDD. |
| Configuration/documentation only | Programmer only, with Director review. Skip Phase A; route test findings to Programmer when no Tester exists. |
| UI/CLI/API behavior, external integration or explicit E2E criteria | Add Verifier for tool discovery and applicable verification. |
| Internal refactoring, library code or work fully covered by unit tests | Omit Verifier unless another requirement calls for it. |

Members report when they have no work and may request shutdown if unnecessary. The fresh Reviewer joins at Step 5 only, after all Implementation tasks, applicable verification and Success Criteria gates.

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

Decide the team composition from the design document steps per the [Team composition](#team-composition).

The Reviewer is **never** part of the initial team composition — it is spawned fresh at Step 5 only, after every Implementation task and Success Criterion is complete.

#### 3d. Read role files

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file through an available text reader on its first turn — do NOT inline the content):

- `skills/cafleet-design-doc/execute/roles/programmer.md`
- `skills/cafleet-design-doc/execute/roles/tester.md` (if Tester needed)
- `skills/cafleet-design-doc/execute/roles/verifier.md` (if Verifier needed)

#### 3e. Spawn each member via `cafleet member create`

Each member is spawned from the canonical [spawn-prompt skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with the per-role delta below (two-stage rendering + brace rules at the skeleton). Use TEAM `design document execution` for all execute roles. All three initial roles load `cafleet` + `cafleet-design-doc` and take `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` as their only context line; each delta below gives the role's title, role-file, IMPORTANT lines (verbatim), and start cue.

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

> Verifier tool discovery follows [Payload exemptions](../reference/coordination.md#payload-exemptions): the inventory is its first substantive message after ready; retrieve complete JSON and ACK it before routing verification work.

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

Use [Commit protocol](#commit-protocol) for phase commits.

### Commit protocol

The Director stages and commits only files eligible under the user's and host's instructions. Keep tests and implementation in separate phase commits. If design-doc or audit files are excluded by those instructions, update them on disk and leave them uncommitted; skip an otherwise empty metadata-only commit. Each `git`/`gh` command runs separately, using a single-line commit message and allowed project prefixes.

| Event | Commit Message Format |
|:--|:--|
| Tests approved | `test: add tests for [feature description]` |
| Implementation passes tests | `feat: [description of what was implemented]` |
| Test fix after escalation | `fix: correct tests for [description]` |
| Post-approval fix | `fix: address review feedback - [description]` |
| Fix routed to Programmer (Reviewer review) | `fix: address Reviewer feedback - <short summary>` |
| Fix routed to Tester (Reviewer review) | `fix: address Reviewer test feedback - <short summary>` |
| Design-doc fix by Director (Reviewer review) | `docs: address Reviewer feedback - <short summary>` |
| Aborted by user | `docs: mark design doc as aborted` |
| All steps complete | `docs: mark design doc as complete` |

No co-author signature (disabled via `attribution.commit` in settings.json).

**Separate-commands rule**: Run every git / gh command as its own Bash call — never chain with `&&`.


### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For each step in the design document:

#### Phase A: Test Writing

**Skip this phase entirely when the Tester was not spawned** (Programmer-only team composition for config/documentation-only steps). Proceed directly to Phase B and assign the step to the Programmer without a separate test-writing commit.

1. **Assign**: Send the Tester a verb + pointer poke. Fire this first dispatch on the Tester's own ready signal — its input, the approved design doc, already exists — never on full-team placement (dispatch-on-ready, § 3f). The Tester reads the step description and specification directly from the design document at the pointer.
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <tester-member-id> "ready (paragraph-Implementation > Step N)"
   ```
2. **End or yield the turn — the assignment is an asynchronous handoff, and the Tester's `complete (paragraph-Implementation > Step N) — <count> tests` (or `blocked (paragraph-Implementation > Step N)` if the spec is unclear) re-opens a later turn**, where you poll and ACK it. On `blocked`, read the Tester's `COMMENT(tester)` marker at the same pointer (pairing rule, coordination.md); if the test framework is ambiguous (per the Tester's `Phase 1` selection step, which uses `blocked (doc)` with the marker at doc-top), ask the user via {decision_surface}, write the answer back as `COMMENT(user-relay): <choice>` at the same doc-top location, and reply with `ready (doc)` so the Tester resumes.
3. **Review tests** against the design doc. If issues are found, write `COMMENT(director): <issue>` markers at `paragraph-Implementation > Step N` (pairing rule) and reply `ready (paragraph-Implementation > Step N)`; the Tester resolves the markers and replies `addressed (paragraph-Implementation > Step N)`. Repeat until satisfied.
4. **Commit tests** per the [Commit protocol](#commit-protocol). Recover the per-test file list directly via git (`git status` / `git diff --stat` / `git log --name-only`) — the Tester does not embed file lists in cafleet bodies under the verb + pointer schema.
   - `git add <test-files>`
   - `git commit -m "test: add tests for [feature description]"`

#### Phase B: Implementation

1. **Assign**: Send the Programmer a verb + pointer poke. The Programmer reads the step spec at the pointer and locates the Tester's freshly-committed test files via git, per its role file.
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <programmer-member-id> "ready (paragraph-Implementation > Step N)"
   ```
2. **End or yield the turn — the Programmer's `complete (paragraph-Implementation > Step N)` re-opens a later turn**, where you poll and ACK it. On `escalating (paragraph-Implementation > Step N)` (suspected test defect), see the **Escalation Protocol (Test Defect)** at the end of Step 4; the rationale lives in a `COMMENT(programmer)` marker at the pointer, not in the cafleet body.
3. **Programmer updates design doc**: Checkboxes, timestamps, and Progress counter.

#### Phase C: Code Review (Director)

1. **Review**: Verify code matches design doc, quality is acceptable, no unnecessary changes.
2. **Feedback loop**: If issues are found, write a `COMMENT(director): <issue>` marker — for design-doc-anchored issues, place it at `paragraph-Implementation > Step N` and send `ready (paragraph-Implementation > Step N)`; for source-anchored issues, place it at `<file>:<line>` and send `ready (<file>:<line>)` (pairing rule, coordination.md). The Programmer resolves the markers, re-runs tests, and replies `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`). Repeat until satisfied.
3. **Commit implementation** per the [Commit protocol](#commit-protocol). Recover the per-file list via git (`git status` / `git diff --stat <base>..HEAD`):
   - `git add <eligible-implementation-files>`
   - `git commit -m "feat: [description of what was implemented]"`

Repeat from Phase A when a Tester exists, otherwise from Phase B. Update the design document for each completed task; stage it only when user/host rules make it eligible under [Commit protocol](#commit-protocol).

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
3. **Route failures** by reading the standing `COMMENT(verifier)` markers and dispatching with `ready (paragraph-Implementation > Step N)`: impl-bug markers → Programmer, test-gap markers → Tester, or Programmer when no Tester was spawned, spec-issue markers → Director resolves directly via `COMMENT(director)` arbitration (or escalates to the user via {decision_surface} if a product decision is needed).
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

**Reviewer spawn prompt** (built from the canonical [spawn-prompt skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton), like the 3e roles):

| Slot | Reviewer |
|---|---|
| ROLE TITLE | `the Reviewer` |
| role-file | `roles/reviewer.md` |
| skill loads | the `cafleet` skill — for communication with the Director; the `cafleet-design-doc` skill — for the coordination protocol and the design-doc format (same pair as the other execute roles per §3e) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` / `BASE BRANCH: [INSERT default branch name from Step 2]` |
| poll-handling line (verbatim) | `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| IMPORTANT (verbatim) | `IMPORTANT: You are a fresh reviewer with no implementation context — judge only what you can verify from the design document, the diff, and the checks you run.` / `IMPORTANT: Do NOT write or modify implementation or test code. Your only edits are COMMENT(reviewer) markers.` / `IMPORTANT: Do NOT commit. The Director handles all git operations.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.` |
| start cue | `Read the design document and the branch diff. Then act on the Director's ready (doc) assignment.` |
| `--name` / `--description` / `--model` | `Reviewer` / `Fresh post-implementation review` / `{reviewer_model}` (the selected member backend's Role defaults value) |

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

Intent judgment and abort detection for free-form replies: [Free-form user replies](#free-form-user-replies).

#### Revision Loop (COMMENT Marker-Based Feedback)

Process a selected marker scan immediately. For ordinary-language revision feedback, the Director records one actionable `COMMENT(user-relay)` at each affected pointer, preserving the user's meaning under [coordination](../reference/coordination.md#commentrole-marker). Ask only about ambiguous meaning/scope. A question receives an answer and supplies no approval; users may also write their own markers.

Route either source of feedback by content location:

| Location | Owner and route |
|---|---|
| Design document | Director resolves the specification and removes the marker; escalate a new product decision. |
| Test files or inline test regions such as `#[cfg(test)]` | Tester; when no Tester was spawned, Programmer. Send `ready (<file>:<line>)` to that owner. |
| Implementation outside test regions | Programmer; send `ready (<file>:<line>)`. |

The assigned owner reads and fixes the marker, removes it, runs appropriate checks and replies `addressed (<file>:<line>)`. With no markers, report that result, show the diff command and invite ordinary-language feedback; marker syntax is optional.

After every accepted revision, route `ready (doc)` to the Reviewer through Step 5. Re-present to the user only after fresh `approved (doc)`; revision feedback never bypasses review. The loop has no round cap and ends on explicit approval or abort.

Record the user's approval scope: **approve-local** means local finalization without push or PR, regardless of upstream tracking. **Remote approval** authorizes Step 7 push/PR and eligible finalization push. Preserve that scope throughout Steps 7–8; tracking configuration cannot grant publication authority.

#### Abort Flow

1. Update design document Status to "Aborted", add Changelog entry. Place a `COMMENT(director): aborting — finalize and stand by` marker near the top of the doc body (above the Overview section — `Status:` is bold metadata, not a heading, so it is not a valid `paragraph-` target). Notify any still-live members with a single `cafleet message send ... "ready (doc)"` per member so they read the marker and stand by.
2. Commit eligible changes under [Commit protocol](#commit-protocol), using `docs: mark design doc as aborted` when the document is eligible; otherwise retain the metadata update uncommitted.
3. Resolve the abort-action marker after members stand by, then follow Shutdown (Step 8: the canonical teardown after reading [Shutdown](../../cafleet/reference/supervision.md#shutdown) immediately before teardown).

### Step 7: Push & Create PR (Director)

After Step 6 Approve, the Director pushes the feature branch and opens a PR BEFORE marking the design doc complete. Nothing waits on the PR after creation. Every command runs per the [Commit protocol](#commit-protocol) separate-command rule.

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
| `gh pr create` fails after push succeeded | stderr of `gh pr create` | Report the successful push and failed PR separately; proceed to Step 8 with remote authorization retained. |
| User provides free-form text in Step 6 with abort-intent | Existing LLM intent judgment | Abort Flow (unchanged — no push) |
| User provides free-form text in Step 6 with approve-local intent | Existing LLM intent judgment, extended | Skip Step 7; go to Step 8 local-finalize |

### Step 8: Finalize & Clean Up (Director)

Runs after Step 7 completes, or directly after Step 6 when Step 7 was skipped (gh not authenticated / default branch / no commits / approve-local intent).

1. Confirm the design document is marker-free, then set Status to "Complete" and add the final Changelog entry.
2. Stage and commit eligible finalization changes under [Commit protocol](#commit-protocol), using `docs: mark design doc as complete` when applicable. Keep excluded design/audit files uncommitted and skip an empty metadata-only commit.
3. **Apply the recorded approval scope before considering upstream tracking.** On approve-local, skip every push and PR action regardless of an existing upstream. For remote-authorized work, inspect whether Step 7's push succeeded: a failed push or precondition failure stays local; report that outcome instead of retrying from finalization. If the authorized push succeeded (including push-success/PR-failure), check `git rev-parse --abbrev-ref <branch-name>@{upstream}` and push eligible finalization commits only when tracked. Report exact push failures; never force-push. With no upstream, leave the finalization commit local.
4. Read [Shutdown](../../cafleet/reference/supervision.md#shutdown) immediately before teardown. Delete the monitor first, then Programmer, Tester, Verifier and Reviewer if spawned; confirm the root-only registry, delete the fleet and confirm closure. Teardown follows local finalization and any authorized final push attempt.
5. Report the PR URL if created or reused, Reviewer rounds/outcome, publication scope, successful push versus failed/missing PR, and every skipped-step reason.
