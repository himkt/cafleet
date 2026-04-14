---
name: cafleet-design-doc-execute
description: Implement features based on a design document using CAFleet-native orchestration with TDD cycle. Use when the user asks to implement or execute a design document. Takes document path as argument. Do NOT implement a design document by reading it and coding manually — always invoke this skill instead.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---

# Design Doc Execute (CAFleet Edition)

Implement features based on a design document using up to four roles orchestrated via the CAFleet message broker: Director (orchestrator), Programmer (implements), Tester (writes tests), and Verifier (E2E/integration testing). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The Director judges which members to spawn based on the nature of the implementation tasks. For each step, the Tester writes unit tests first, the Director reviews and approves them, then the Programmer implements code to pass the tests. The Director also reviews the Programmer's implementation for code quality and design doc compliance before committing. After all TDD steps, the Verifier performs E2E/integration verification (Phase D) if spawned.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet, spawn members via `cafleet member create`, validate doc, assign steps, review tests against design doc, review implementation code for quality and compliance, commit after each phase, escalation arbitration, orchestrate TDD cycle | Write code, write tests | [roles/director.md](roles/director.md) |
| **Programmer** | Member agent | Implement code to pass tests, run tests, report results via `cafleet send`, escalate test defects to Director, update design doc checkboxes and Progress counter | Write or modify tests, commit code, communicate with user directly | [roles/programmer.md](roles/programmer.md) |
| **Tester** | Member agent | Read design doc, write unit tests per step, fix tests based on Director feedback, report to Director via `cafleet send` | Write implementation code, commit code, communicate with user directly | [roles/tester.md](roles/tester.md) |
| **Verifier** | Member agent (optional) | E2E/integration testing, tool discovery, evidence collection (screenshots, logs, output), failure reporting with suggested fixes | Write code, write tests, commit, communicate with user directly | [roles/verifier.md](roles/verifier.md) |

## Additional resources

- For the document template, see: [../cafleet-design-doc/template.md](../cafleet-design-doc/template.md)
- For section guidelines and quality standards, see: [../cafleet-design-doc/guidelines.md](../cafleet-design-doc/guidelines.md)

## Architecture

The Director registers with a CAFleet session and spawns each needed member via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet register, cafleet member create, orchestrates TDD cycle)
      +-- Programmer (member agent -- implements code to pass tests)
      +-- Tester (member agent -- writes unit tests per step)
      +-- Verifier (member agent, optional -- E2E/integration testing)
```

- **Director ↔ Programmer**: `cafleet send` (step assignments, test results, code review feedback, escalation)
- **Director ↔ Tester**: `cafleet send` (step assignments, test review feedback, test defect reports)
- **Director ↔ Verifier**: `cafleet send` (verification assignments, results, failure routing)
- **Director**: git operations (commit after each phase — tests and implementation separately)
- Members receive messages via push notification: the broker injects `cafleet --session-id <session-id> poll --agent-id <recipient-agent-id>` into the member's pane via `tmux send-keys`. The literal `<session-id>` and `<recipient-agent-id>` UUIDs are the session and target member UUIDs the broker has in scope, baked into the injected command string. `--session-id` is a global flag (placed **before** the subcommand); `--agent-id` is a per-subcommand option (placed **after** the subcommand name).

## Prerequisites

The Director MUST be running inside a tmux session (required by `cafleet member create`). If `TMUX` is not set, abort with an explanatory message to the user before spawning anyone.

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="execute-{slug}")` | CAFleet session (pre-existing or `cafleet session create`) + `cafleet --session-id <session-id> register` (Director) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name "..." --description "..." -- "<prompt>"` |
| `SendMessage(to="Programmer")` | `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <programmer-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from member) | `cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `agent-team-supervision` `/loop` | `Skill(cafleet-monitoring)` `/loop` |
| `TeamDelete` | `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <member-agent-id>` for each member + `cafleet --session-id <session-id> deregister --agent-id <director-agent-id>` |
| Auto message delivery | Push notification injects `cafleet --session-id <session-id> poll --agent-id <recipient-agent-id>` into member's tmux pane |

## Process

### Step 1: Resolve Design Document Path (Director)

Before validation, resolve `$ARGUMENTS` into a concrete `design-doc.md` path.

#### Phase 1: Base Directory Resolution

Load `Skill(base-dir)` and follow its procedure with `$ARGUMENTS` as the argument.
- If skipped (absolute path): set `${RESOLVED_ARGS} = $ARGUMENTS`.
- If base resolved: set `${RESOLVED_ARGS} = ${BASE}/design-docs/$ARGUMENTS`. Resolve to absolute path.

#### Phase 2: Three-Tier Detection

Using `${RESOLVED_ARGS}`, apply a three-tier detection strategy, evaluated in order:

| Tier | Condition | Action |
|:--|:--|:--|
| 1 — Direct file path | `${RESOLVED_ARGS}` ends with `design-doc.md` | Use as-is |
| 2 — Slug directory | `${RESOLVED_ARGS}` is a directory that contains `design-doc.md` directly | Append `/design-doc.md` |
| 3 — Base directory | `${RESOLVED_ARGS}` is a directory containing `**/design-doc.md` (one level deep) | Enter discovery flow |

Tier evaluation is sequential and short-circuits.

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
| 2–4 | Present options via `AskUserQuestion` (see Selection UI below) |
| 5+ | Present options via paginated `AskUserQuestion` (see Pagination below) |

#### Selection UI (2–4 Approved Docs)

Use `AskUserQuestion` with one question. Each option label is the slug name (directory name) of the design doc. The built-in "Other" option is always available for the user to type a direct path or cancel.

Example with 3 approved docs:

```
Question: "Which design document would you like to implement?"
Options:
  1: "feature-auth"
  2: "refactor-db-layer"
  3: "add-cli-export"
  (Other is added automatically)
```

#### Pagination (5+ Approved Docs)

When there are more than 4 approved docs, `AskUserQuestion`'s option limit (max 4) is exceeded. Use pagination with all options sorted alphabetically by slug:

- **Non-last page**: Show 3 options + a 4th option labeled `"More..."`.
- **Last page rule**: If remaining items after the current page would be ≤ 4, show all remaining items directly (no `"More..."` needed). This avoids a last page with only 1 option, which would violate `AskUserQuestion`'s minimum of 2 options per question.
- Continue until the user selects a document or uses "Other".

Example with 7 approved docs: page 1 shows 3 + "More..." (4 remain), page 2 shows all 4. Example with 5: page 1 shows 3 + "More..." (2 remain), page 2 shows both 2.

#### Error: Zero Approved Docs

When design docs exist but none have `Status: Approved`, display a message listing all found docs with their current statuses so the user understands why none qualified. Format:

```
No approved design documents found in <base-directory>.

Found documents:
  - <slug-1>/design-doc.md — Status: Draft
  - <slug-2>/design-doc.md — Status: In Progress
  - <slug-3>/design-doc.md — Status: Complete

Only documents with Status: Approved can be executed. Update the status or specify a direct path.
```

Then abort (do not proceed to team creation or execution).

#### Error: Invalid Path

When `${RESOLVED_ARGS}` does not match any of the three tiers (not a file path ending in `design-doc.md`, not a directory containing `design-doc.md`, and no `**/design-doc.md` found underneath), display:

```
Invalid argument: `${RESOLVED_ARGS}`
Expected one of:
  - Path to a design-doc.md file (e.g., my-feature/design-doc.md)
  - Slug directory containing design-doc.md (e.g., my-feature/)
  - No argument (discovers all design docs in ${BASE}/design-docs/)
```

Then abort.

After resolution, the resolved path is used as the design document path for all subsequent steps.

### Step 2: Validate Design Document & Create Branch (Director)

Before registering with CAFleet:

1. Read the design document completely.
2. Check for `COMMENT(` markers using Grep. If found, resolve them directly: apply the requested changes and remove the markers. Verify with Grep that no `COMMENT(` markers remain before proceeding.
3. Check for `FIXME(claude)` markers in the codebase using Grep. If found, note them for the Programmer to resolve first.
4. Determine the step order and total number of steps.
5. **Create a feature branch if on the default branch.** Get the default branch with `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` and the current branch with `git branch --show-current`. If they match, use `AskUserQuestion` to propose the branch name `feat/<design-doc-slug>` and ask the user to approve before creating it. The user will create the branch themselves or approve the proposed name. If already on a non-default branch, skip this step.

### Step 3: Register & Spawn Members (Director)

Load `Skill(cafleet)` and `Skill(cafleet-monitoring)`.

#### 3a. Establish a CAFleet session

If you do not already have a session UUID for this run, create one:

```bash
cafleet session create --label "design-doc-execute-{slug}"
# → prints <session-id>, e.g. 550e8400-e29b-41d4-a716-446655440000
```

Capture the printed UUID and substitute it for `<session-id>` in every subsequent command. **Do not store it in a shell variable** — `permissions.allow` matches command strings literally, so every command must carry the literal UUID. Reuse the same UUID across the entire run. Remember: `--session-id` is a global flag that goes **before** the subcommand; `--agent-id` is a per-subcommand option that goes **after** the subcommand name.

#### 3b. Register the Director

```bash
cafleet --session-id <session-id> --json register \
  --name "Director" \
  --description "Design doc execute orchestration director"
```

Parse `agent_id` from the JSON response and substitute it for `<director-agent-id>` in every subsequent command for the remainder of the session.

#### 3c. Start the monitoring `/loop`

BEFORE spawning any member, follow `Skill(cafleet-monitoring)`'s Monitoring Mandate and start a `/loop` monitor at the 1-minute interval using the literal `<session-id>` and `<director-agent-id>` UUIDs. The loop must stay active from the first `member create` until Step 6's shutdown cleanup.

#### 3d. Analyze implementation tasks to decide team composition

Based on the design document steps (see [roles/director.md](roles/director.md) for the full decision matrix):

| Task nature | Team composition |
|---|---|
| Code implementation | Programmer + Tester |
| Config/documentation only | Programmer only |
| E2E verification needed (user-visible changes, CLI/UI/API) | + Verifier |

#### 3e. Read role files

Read the role files that will be embedded verbatim in spawn prompts:

- `.claude/skills/cafleet-design-doc-execute/roles/programmer.md`
- `.claude/skills/cafleet-design-doc-execute/roles/tester.md` (if Tester needed)
- `.claude/skills/cafleet-design-doc-execute/roles/verifier.md` (if Verifier needed)

#### 3f. Spawn each member via `cafleet member create`

**Programmer spawn prompt:**

```
You are the Programmer in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/programmer.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>     (will be filled in literally by member create)
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Programmer" \
  --description "Implements code to pass tests per step" \
  -- "<Programmer spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<programmer-agent-id>` in every subsequent command.

**Tester spawn prompt (if needed):**

```
You are the Tester in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/tester.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>     (will be filled in literally by member create)
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: Do NOT write implementation code — only test code.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Tester" \
  --description "Writes unit tests per step" \
  -- "<Tester spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<tester-agent-id>` in every subsequent command.

**Verifier spawn prompt (if needed):**

```
You are the Verifier in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/verifier.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>     (will be filled in literally by member create)
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code or modify implementation/test files.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document and discovering available tools.
Then wait for the Director to assign your first verification task.
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Verifier" \
  --description "E2E/integration testing and evidence collection" \
  -- "<Verifier spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<verifier-agent-id>` in every subsequent command.

#### 3g. Verify members are live

```bash
cafleet --session-id <session-id> member list --agent-id <director-agent-id>
```

All spawned members must show `status: active` with a non-null `pane_id`. If any is missing or pending, retry the spawn before proceeding.

See [roles/director.md](roles/director.md) for commit message conventions.

### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For each step in the design document:

#### Phase A: Test Writing

**Skip this phase entirely when the Tester was not spawned** (Programmer-only team composition for config/documentation-only steps). Proceed directly to Phase B and assign the step to the Programmer without a separate test-writing commit.

1. **Assign**: Send the Tester the step number, description, and specification:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <tester-agent-id> --text "Step N: <description>. Spec: <…>. Write unit tests and report file paths when done."
   ```
2. **Wait for Tester report via `cafleet --session-id <session-id> poll --agent-id <director-agent-id>`**. If the test framework is ambiguous, ask the user via `AskUserQuestion` and relay the answer via `cafleet send`.
3. **Review tests** against the design doc. Send feedback via `cafleet send` if issues found. Repeat until satisfied.
4. **Commit tests** (separate commands, do NOT chain with `&&`):
   - `git add <test-files>`
   - `git commit -m "test: add tests for [feature description]"`

#### Phase B: Implementation

1. **Assign**: Send the Programmer the step number, description, and test file paths:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <programmer-agent-id> --text "Step N: <description>. Tests at: <paths>. Implement to pass all tests, update design doc checkboxes and Progress counter, then report."
   ```
2. **Wait for Programmer report via `cafleet --session-id <session-id> poll --agent-id <director-agent-id>`**. On suspected test defect, see [roles/director.md](roles/director.md) for the escalation protocol.
3. **Programmer updates design doc**: Checkboxes, timestamps, and Progress counter.

#### Phase C: Code Review (Director)

1. **Review**: Verify code matches design doc, quality is acceptable, no unnecessary changes.
2. **Feedback loop**: Send feedback via `cafleet send` if issues found. Programmer fixes, re-runs tests, re-reports via `cafleet send`. Repeat until satisfied.
3. **Commit implementation** (separate commands, do NOT chain with `&&`):
   - `git add <files> <design-doc>`
   - `git commit -m "feat: [description of what was implemented]"`

Repeat from Phase A for the next step. Always include the design document in the implementation commit.

**Escalation Protocol (Test Defect):** If the Programmer reports a suspected test defect (implementation matches design doc but tests expect something different), the Director reads the design doc and test, then directs either the Tester to fix the test or the Programmer to adjust the implementation via `cafleet send`. 3-round limit before escalating to the user.

**On-Demand Verification**: Any member can request verification mid-task via `cafleet send` to the Director. The Director decides whether to route immediately or defer:

| Route immediately | Defer to Phase D |
|:--|:--|
| User-visible behavior change (UI, CLI output, API response) | Internal refactoring or data model change |
| Integration with external system | Adequately covered by unit tests |
| Behavior difficult to catch with unit tests alone | Verification requires setup from a later step |

### Phase D: Verification (Director) — conditional

**Skip this phase entirely if the Verifier was not spawned.** Proceed directly to Step 5 (User Approval).

If the Verifier was spawned, assign verification:

1. Send the Verifier the design document, completed steps, and relevant files via `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <verifier-agent-id> --text "..."`.
2. Verifier discovers tools, executes E2E verification, captures evidence, reports results via `cafleet send`.
3. **Route failures**: Implementation bugs → Programmer via `cafleet send`, test gaps → Tester via `cafleet send`, spec issues → user.
4. Re-verify after fixes. Proceed to User Approval when all verifiable criteria pass.

### Step 5: User Approval (Director)

After all TDD steps complete but before finalization, present the implementation to the user for approval.

#### Success Criteria Verification

**Before presenting to the user**, verify the design document's Success Criteria section:

1. Read the `## Success Criteria` section from the design document.
2. For each criterion, verify it is satisfied by inspecting the implementation (grep, read files, run tests as needed).
3. Check off all satisfied criteria in the design document (`- [ ]` → `- [x]`).
4. If any criterion is NOT satisfied, resolve it before proceeding to user approval — route to Programmer or Tester as needed via `cafleet send`.

This step is **mandatory** and must not be skipped.

#### Change Presentation

1. **Git diff command** for the user to inspect (e.g., `git diff main...HEAD`).
2. **Step-by-step change summary** — concise prose of what changed per step (files modified, key behaviors).

#### Approval Interaction

Use `AskUserQuestion`:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with the current result | Proceed to finalization (Step 6) |
| 2 | **Scan for COMMENT markers** | Add `COMMENT(name): feedback` markers to the changed source files, then select this option to process them | Scan and process markers (see Revision Loop below) |
| 3 | *(Other — built-in)* | *(Free text input)* | Interpret user intent (see Revision Loop below) |

See [roles/director.md](roles/director.md) for user interaction rules (COMMENT handling, classification, intent judgment, abort detection).

#### Revision Loop (COMMENT Marker-Based Feedback)

When the user selects "Scan for COMMENT markers": scan changed files for `COMMENT(` markers. Classify by file location (see [roles/director.md](roles/director.md)) and route via `cafleet send`:
- Design-doc COMMENTs → Director resolves directly (no routing).
- Source-file COMMENTs → `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <programmer-agent-id> --text "..."`.
- Test-file COMMENTs → `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <tester-agent-id> --text "..."`.

After all COMMENTs are resolved and verified, re-present to user.

When the user selects "Other": interpret intent per [roles/director.md](roles/director.md) rules.

No round limit — the loop continues until the user approves or aborts.

#### Abort Flow

1. Update design document Status to "Aborted", add Changelog entry.
2. Commit (separate commands): `git add <design-doc>` then `git commit -m "docs: mark design doc as aborted"`
3. Follow Shutdown Protocol (Step 6: cancel /loop, delete members, deregister Director).

### Step 6: Finalize & Clean Up (Director)

1. Update design document Status to "Complete" and add final Changelog entry.
2. Commit (separate commands): `git add <design-doc>` then `git commit -m "docs: mark design doc as complete"`
3. Cancel the `/loop` monitor (`CronDelete` on the cron ID recorded when the loop was created).
4. Delete each spawned member:
   ```bash
   cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <programmer-agent-id>
   cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <tester-agent-id>      # if spawned
   cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <verifier-agent-id>    # if spawned
   ```
5. Deregister the Director:
   ```bash
   cafleet --session-id <session-id> deregister --agent-id <director-agent-id>
   ```

No `TeamDelete` equivalent is needed — the CAFleet session persists for audit purposes so the message history remains inspectable in the admin WebUI.
