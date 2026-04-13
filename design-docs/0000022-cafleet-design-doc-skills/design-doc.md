# CAFleet-Native Design Document Orchestration Skills

**Status**: Approved
**Progress**: 10/19 tasks complete
**Last Updated**: 2026-04-14

## Overview

Two new skills (`/cafleet-design-doc-create` and `/cafleet-design-doc-execute`) that replicate the existing Agent Teams-based design document workflows using CAFleet message broker primitives. The Director spawns member agents via `cafleet member create`, communicates via `cafleet send` with tmux push notifications, and monitors via `Skill(cafleet-monitoring)`. Plugin infrastructure updates expose these skills to other projects via the existing `cafleet` plugin.

## Success Criteria

- [ ] `/cafleet-design-doc-create` produces design documents through the same clarification → drafting → review → approval process as `/design-doc-create`
- [ ] `/cafleet-design-doc-execute` implements features through the same TDD cycle as `/design-doc-execute`
- [ ] Both skills use CAFleet primitives exclusively (no `TeamCreate`, `Agent(team_name=...)`, or `SendMessage`)
- [ ] Skills are accessible in other projects via plugin as `/cafleet:cafleet-design-doc-create` and `/cafleet:cafleet-design-doc-execute`
- [ ] Existing `/design-doc-create` and `/design-doc-execute` Agent Teams skills remain functional (coexistence)
- [ ] No changes to CAFleet Python codebase or database schema

---

## Background

The existing `/design-doc-create` and `/design-doc-execute` skills use Claude Code's Agent Teams primitives (`TeamCreate`, `Agent(team_name=...)`, `SendMessage`, `agent-team-supervision`) for multi-agent orchestration. CAFleet provides equivalent primitives — `cafleet member create` for spawning, `cafleet send` for messaging, `cafleet member capture` for inspection — with the benefit that all coordination goes through a persistent message queue visible in the admin WebUI.

The new skills replicate the same processes but use CAFleet primitives exclusively. Every message between Director and members is persisted, auditable, and visible in the admin timeline. The two skill sets coexist naturally: Agent Teams versions at `~/.claude/skills/design-doc-{create,execute}/` and CAFleet versions at `.claude/skills/cafleet-design-doc-{create,execute}/` in the CAFleet project.

---

## Specification

### Primitive Mapping

| Agent Teams Primitive | CAFleet Equivalent |
|---|---|
| `TeamCreate(name="create-{slug}")` | CAFleet session (pre-existing; Director registers with `cafleet register`) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet member create --agent-id $DIR --name "..." --description "..." -- "prompt"` |
| `SendMessage(to="Drafter")` | `cafleet send --agent-id $DIR --to $MEMBER_ID --text "..."` (push notification auto-triggers poll in member pane) |
| `SendMessage(to="Director")` (from member) | `cafleet send --agent-id $CAFLEET_AGENT_ID --to $DIRECTOR_ID --text "..."` |
| `agent-team-supervision` `/loop` | `Skill(cafleet-monitoring)` `/loop` |
| Teammate shutdown | `cafleet member delete --agent-id $DIR --member-id $MEMBER_ID` |
| `TeamDelete` | `cafleet deregister --agent-id $DIR` |
| Message auto-delivery to teammate | Push notification sends `cafleet poll --agent-id $CAFLEET_AGENT_ID` to member's tmux pane |

### Communication Flow

```
Director                                    Member
   |                                           |
   |-- cafleet member create (with prompt) --> |  (spawned with initial task)
   |                                           |
   |                                           |-- does assigned work
   |                                           |
   |  <-- cafleet send (report) --------------|  (reports completion)
   |                                           |
   |-- cafleet send (next task) ------------> |  (push notification triggers poll)
   |                                           |
   |                                           |-- sees task via cafleet poll
   |                                           |-- does next task
   |                                           |
   |  <-- cafleet send (report) --------------|
   |                                           |
   |-- cafleet member delete --------------> |  (shutdown)
```

Members do NOT run `/loop`. They do work, report via `cafleet send`, and receive next tasks via push notification.

### File Inventory

#### New skill files

| Path | Purpose |
|---|---|
| `.claude/skills/cafleet-design-doc/SKILL.md` | Skill definition — design doc template and guidelines (plugin-local copy) |
| `.claude/skills/cafleet-design-doc/template.md` | Design document template (identical to `~/.claude/skills/design-doc/template.md`) |
| `.claude/skills/cafleet-design-doc/guidelines.md` | Quality standards and formatting rules (identical to `~/.claude/skills/design-doc/guidelines.md`) |
| `.claude/skills/cafleet-design-doc-create/SKILL.md` | Skill definition — CAFleet-native design doc creation |
| `.claude/skills/cafleet-design-doc-create/roles/director.md` | Director role — orchestration via CAFleet |
| `.claude/skills/cafleet-design-doc-create/roles/drafter.md` | Drafter role — document authoring |
| `.claude/skills/cafleet-design-doc-create/roles/reviewer.md` | Reviewer role — quality review |
| `.claude/skills/cafleet-design-doc-execute/SKILL.md` | Skill definition — CAFleet-native design doc execution |
| `.claude/skills/cafleet-design-doc-execute/roles/director.md` | Director role — TDD orchestration via CAFleet |
| `.claude/skills/cafleet-design-doc-execute/roles/programmer.md` | Programmer role — implementation |
| `.claude/skills/cafleet-design-doc-execute/roles/tester.md` | Tester role — test writing |
| `.claude/skills/cafleet-design-doc-execute/roles/verifier.md` | Verifier role — E2E verification |

#### Modified files

| Path | Change |
|---|---|
| `.claude/skills/cafleet-monitoring/SKILL.md` | Replace `SendMessage` references with `cafleet send` equivalents |
| `.claude-plugin/plugin.json` | Add new skill paths to `skills` array |
| `CLAUDE.md` | Add skill entries (via `/sync-skills`) |
| `.claude/CLAUDE.md` | Add project skill entries (via `/sync-skills`) |

### cafleet-design-doc (Template Skill)

A plugin-local copy of the global `/design-doc` skill (`~/.claude/skills/design-doc/`). This makes the plugin fully self-contained — other projects using the `cafleet` plugin do not need the global `/design-doc` skill installed.

| File | Content |
|---|---|
| `SKILL.md` | Identical to global `/design-doc` SKILL.md with `name: cafleet-design-doc` |
| `template.md` | Identical copy of global `/design-doc` template.md |
| `guidelines.md` | Identical copy of global `/design-doc` guidelines.md |

All spawn prompts instruct members to load `Skill(cafleet-design-doc)` instead of `Skill(design-doc)`.

### Plugin Infrastructure

Update `.claude-plugin/plugin.json` to expose the new skills:

```json
{
  "name": "cafleet",
  "version": "0.2.0",
  "description": "A2A-native message broker CLI and design document orchestration skills for coding agents.",
  "author": { "name": "himkt" },
  "repository": "https://github.com/himkt/hikyaku",
  "license": "MIT",
  "keywords": ["a2a", "messaging", "broker", "agents", "cli", "design-doc"],
  "skills": [
    "./.claude/skills/cafleet",
    "./.claude/skills/cafleet-design-doc",
    "./.claude/skills/cafleet-design-doc-create",
    "./.claude/skills/cafleet-design-doc-execute"
  ]
}
```

Other projects consume the plugin by referencing the CAFleet repo path. Once installed, skills are available as `/cafleet:cafleet-design-doc-create` and `/cafleet:cafleet-design-doc-execute`.

### Director Registration Protocol

Both skills share the same Director setup sequence at the start of each invocation:

1. Load `Skill(cafleet)` and `Skill(cafleet-monitoring)`.
2. Create or reuse a CAFleet session:
   ```bash
   cafleet session create --label "design-doc-{slug}"
   export CAFLEET_SESSION_ID=<session_id>
   ```
3. Register as Director:
   ```bash
   cafleet --json register --name "Director" --description "Design doc orchestration director"
   ```
   Parse `agent_id` from the JSON response and store as `$DIRECTOR_ID`.
4. Start the monitoring `/loop` per `Skill(cafleet-monitoring)` BEFORE spawning any member.

### Member Spawn Protocol

The Director spawns members via `cafleet member create`. The spawn prompt includes the role definition content (read from `roles/<role>.md` and embedded verbatim), instructions to load skills, the Director's agent ID, and the initial task.

```bash
cafleet --json member create --agent-id $DIRECTOR_ID \
  --name "<Role>" \
  --description "<one-sentence purpose>" \
  -- "<spawn prompt with embedded role definition>"
```

Parse `agent_id` from the JSON response. The Director stores this for subsequent communication.

### Member Communication Protocol

Members use the `CAFLEET_AGENT_ID` environment variable (auto-injected by `cafleet member create`) and the Director's agent ID (provided in their spawn prompt).

**Sending a report to Director:**

```bash
cafleet send --agent-id $CAFLEET_AGENT_ID --to <director-id> --text "<report>"
```

**Receiving a task from Director:**

When the Director sends a message, the push notification mechanism injects `cafleet poll --agent-id $CAFLEET_AGENT_ID` into the member's tmux pane. The member sees the poll output and acts on the Director's instructions.

**Acknowledging a received message:**

```bash
cafleet ack --agent-id $CAFLEET_AGENT_ID --task-id <task-id>
```

### Shutdown Protocol

1. Cancel the `/loop` monitor (`CronDelete`).
2. Delete each member: `cafleet member delete --agent-id $DIRECTOR_ID --member-id <member-id>`
3. Deregister Director: `cafleet deregister --agent-id $DIRECTOR_ID`

No `TeamDelete` equivalent needed — the CAFleet session persists for audit purposes.

### cafleet-design-doc-create

#### SKILL.md

```yaml
---
name: cafleet-design-doc-create
description: Create a new design document using CAFleet-native orchestration. Use when user wants to create a specification or technical document with CAFleet message broker coordination. Do NOT use EnterPlanMode — always invoke this skill instead.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---
```

The process mirrors `/design-doc-create` exactly with CAFleet primitives replacing Agent Teams.

> **Implementer note**: Steps marked "Identical to `/design-doc-create`" or "Identical to `/design-doc-execute`" must be copied into the SKILL.md verbatim — each skill file must be self-contained. Read the referenced step from the original skill, reproduce the full logic, and replace only the communication primitives (`TeamCreate` → registration, `Agent(team_name=...)` → `cafleet member create`, `SendMessage` → `cafleet send`, `agent-team-supervision` → `cafleet-monitoring`). All process logic, option tables, error handling, and user interaction flows remain identical.

| Role | Name | Description |
|---|---|---|
| **Director** | Main Claude | Register with CAFleet, spawn members, relay user answers, enforce clarification gate, orchestrate quality loop, present draft to user |
| **Drafter** | Member agent | Ask clarifying questions (via Director relay), read codebase, write and revise the design document |
| **Reviewer** | Member agent | Critically review drafts for rule compliance, readability, completeness, correctness |

**Step 0: Path Resolution & Resume Detection**

Identical to `/design-doc-create`. Load `Skill(base-dir)`, resolve `$ARGUMENTS` to `${DOC_PATH}`. Resume detection checks for COMMENT markers in existing file.

**Step 1: Register & Spawn Members**

1. Follow Director Registration Protocol.
2. Read `roles/drafter.md` and `roles/reviewer.md` content.
3. Spawn Drafter via `cafleet member create` with role content embedded (see spawn prompts below).
4. Spawn Reviewer via `cafleet member create` with role content embedded.
5. Verify both members active via `cafleet member list`.

**Step 2: Clarification Phase**

Skip in resume mode. Otherwise:

1. Wait for Drafter's clarifying questions via `cafleet poll` (monitoring loop detects incoming messages).
2. Relay questions to user via `AskUserQuestion`, relay answers back via `cafleet send --to $DRAFTER_ID`.
3. Gate check: if Drafter produces a draft without prior questions, send rejection via `cafleet send` and instruct to ask first.

**Step 3: Internal Quality Loop**

1. After Drafter produces draft, send document path to Reviewer via `cafleet send --to $REVIEWER_ID`.
2. Reviewer reviews, reports feedback via `cafleet send` to Director.
3. If feedback, Director routes to Drafter via `cafleet send --to $DRAFTER_ID`.
4. Repeat until Reviewer signals "APPROVED". Aim for 2–3 rounds.

**Step 4: Present to User**

Identical to `/design-doc-create`. `AskUserQuestion` with Approve / Scan for COMMENT markers / Other.

**Step 5: User Feedback Loop**

Identical to `/design-doc-create`. COMMENT marker handling, LLM intent judgment, abort detection. When COMMENTs found, route to Drafter via `cafleet send`, then re-enter quality loop.

**Step 6: Finalize & Clean Up**

1. Send finalization instructions to Drafter via `cafleet send`. Wait for confirmation.
2. Follow Shutdown Protocol (cancel `/loop` monitor, delete members, deregister Director).

#### Spawn Prompts

**Drafter (normal mode):**

```
You are the Drafter in a design document creation team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/drafter.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]
OUTPUT PATH: [INSERT ${DOC_PATH}]

The user's request: [INSERT USER'S ORIGINAL REQUEST]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.
Send your questions to the Director who will relay them to the user.
Start by reading the target codebase for context, then send your clarifying questions.
Do NOT create any design document file until you have received answers.
```

**Drafter (resume mode):**

```
You are the Drafter in a design document creation team (CAFleet-native, RESUME MODE).

<ROLE DEFINITION>
[Content of roles/drafter.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]
DESIGN DOCUMENT: [INSERT ${DOC_PATH}]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

This is a RESUME session. The document contains COMMENT markers from a previous
interview. Follow the Resume Mode instructions in your role definition.
Do NOT ask clarifying questions — the COMMENTs contain the needed information.
Start by reading the design document.
```

**Reviewer:**

```
You are the Reviewer in a design document creation team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/reviewer.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

Wait for the Director to assign a document for review.
```

#### Role File Specifications

Each role file is based on its corresponding Agent Teams role with communication adapted for CAFleet. The files are self-contained documents embedded in spawn prompts.

**roles/director.md** — Based on `/design-doc-create` `roles/director.md`:

Responsibilities (unchanged):
- Enforce clarification gate
- Relay communication faithfully
- Orchestrate internal quality loop
- Present polished draft to user
- Drive user feedback iterations

Changes from Agent Teams version:
- **Registration**: Load `Skill(cafleet)` and `Skill(cafleet-monitoring)`. Register with CAFleet session, start monitoring `/loop`.
- **Communication**: Replace all `SendMessage(to=name)` references with `cafleet send --agent-id $DIRECTOR_ID --to $MEMBER_ID --text "..."`. Replace message checking with `cafleet poll --agent-id $DIRECTOR_ID`.
- **Health checks**: Replace `agent-team-supervision` idle detection with `cafleet-monitoring` 2-stage health check (poll → member capture).
- **Cleanup**: Replace `TeamDelete` with `cafleet member delete` for each member + `cafleet deregister` for Director.

User Interaction Rules (unchanged): COMMENT marker handling, LLM intent judgment, abort detection — identical to Agent Teams version.

Progress Monitoring milestones table — same phases (Clarification, Drafting, Review, Revision) with same stall indicators. Director action changes from `SendMessage` to `cafleet send`.

**roles/drafter.md** — Based on `/design-doc-create` `roles/drafter.md`:

Responsibilities (unchanged):
- Ask clarifying questions before drafting
- Write document using design-doc skill template
- Revise based on Reviewer feedback
- Process COMMENT markers
- Resume mode behavior

Changes from Agent Teams version:
- **Communication**: Replace "send questions to the Director" with `cafleet send --agent-id $CAFLEET_AGENT_ID --to $DIRECTOR_ID --text "questions"`. Replace "Director relay" with "Director receives via `cafleet poll` and relays to user".
- **Receiving instructions**: Add section explaining that incoming tasks arrive via push notification (`cafleet poll` output appears in terminal). Member reads the message content and acts on it.

Structured Question Framework, COMMENT Processing, Resume Mode — unchanged from Agent Teams version.

**roles/reviewer.md** — Based on `/design-doc-create` `roles/reviewer.md`:

Responsibilities (unchanged):
- Rule compliance, readability, completeness, correctness, actionability
- Categorized feedback with tags ([COMPLIANCE], [GAP], [UNCLEAR], [INCORRECT], [IMPROVEMENT])
- APPROVED signal format

Changes from Agent Teams version:
- **Communication**: Replace "provide feedback" with `cafleet send --agent-id $CAFLEET_AGENT_ID --to $DIRECTOR_ID --text "feedback"`.
- **Receiving assignments**: Via push notification (`cafleet poll` output).

### cafleet-design-doc-execute

#### SKILL.md

```yaml
---
name: cafleet-design-doc-execute
description: Implement features based on a design document using CAFleet-native orchestration with TDD cycle. Use when the user asks to implement or execute a design document. Takes document path as argument. Do NOT implement a design document by reading it and coding manually — always invoke this skill instead.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---
```

| Role | Name | Description |
|---|---|---|
| **Director** | Main Claude | Validate doc, spawn members, assign steps, review tests & code, commit, orchestrate TDD cycle |
| **Programmer** | Member agent | Implement code to pass tests, run tests, update design doc checkboxes |
| **Tester** | Member agent | Write unit tests per step, fix tests based on Director feedback |
| **Verifier** | Member agent (optional) | E2E/integration testing, evidence collection |

**Step 1: Resolve Design Document Path**

Identical to `/design-doc-execute`. Three-tier detection: direct file path → slug directory → base directory discovery. Same `Skill(base-dir)` integration, same Selection UI, same error handling.

**Step 2: Validate Design Document & Create Branch**

Identical to `/design-doc-execute`. Read doc, resolve COMMENT markers, note FIXME(claude) markers, create feature branch if on default branch.

**Step 3: Register & Spawn Members**

1. Follow Director Registration Protocol.
2. Read role files for each needed member.
3. Analyze implementation tasks to decide team composition (same rules as `/design-doc-execute`):
   - Code implementation → Programmer + Tester
   - Config/documentation only → Programmer only
   - E2E verification needed → + Verifier
4. Spawn members via `cafleet member create` with role content embedded (see spawn prompts below).
5. Verify all members active via `cafleet member list`.

**Step 4: Execute Steps with Per-Step TDD Cycle**

Same TDD cycle as `/design-doc-execute`, using CAFleet messaging:

**Phase A — Test Writing:**
1. Send test assignment to Tester via `cafleet send --to $TESTER_ID`.
2. Wait for Tester report via `cafleet poll` (monitoring loop).
3. Review tests against design doc. Send feedback via `cafleet send` if needed.
4. Commit tests: `git add <test-files>` then `git commit -m "test: add tests for [description]"`.

**Phase B — Implementation:**
1. Send implementation assignment to Programmer via `cafleet send --to $PROGRAMMER_ID` (include test file paths).
2. Wait for Programmer report via `cafleet poll`.
3. Programmer updates design doc checkboxes and Progress counter.

**Phase C — Code Review:**
1. Review code for quality and design doc compliance.
2. Send feedback via `cafleet send` if needed. Programmer fixes and re-reports.
3. Commit implementation: `git add <files> <design-doc>` then `git commit -m "feat: [description]"`.

**Phase D — Verification (conditional):**
Skip if Verifier was not spawned. Otherwise same as `/design-doc-execute`: assign verification, route failures, re-verify after fixes.

**Escalation Protocol (Test Defect):**
Same as `/design-doc-execute`. Programmer reports via `cafleet send`, Director reads doc and test, directs Tester or Programmer via `cafleet send`. 3-round limit before user escalation.

**On-Demand Verification:**
Same routing rules as `/design-doc-execute` (route immediately for user-visible changes, defer for internal refactoring).

**Step 5: User Approval**

Identical to `/design-doc-execute`. Success criteria verification (mandatory), change presentation with `git diff`, `AskUserQuestion` with Approve / Scan for COMMENT markers / Other. COMMENT classification by file location (design doc → Director, source → Programmer, test → Tester).

**Step 6: Finalize & Clean Up**

1. Update design doc Status to "Complete", add Changelog entry.
2. Commit: `git add <design-doc>` then `git commit -m "docs: mark design doc as complete"`.
3. Follow Shutdown Protocol (cancel `/loop` monitor, delete members, deregister Director).

#### Spawn Prompts

**Programmer:**

```
You are the Programmer in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/programmer.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

**Tester:**

```
You are the Tester in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/tester.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: Do NOT write implementation code — only test code.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

**Verifier:**

```
You are the Verifier in a design document execution team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/verifier.md injected here by Director]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(cafleet-design-doc) — for template and guidelines

DIRECTOR AGENT ID: [INSERT $DIRECTOR_ID]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet send --agent-id $CAFLEET_AGENT_ID --to [DIRECTOR_ID] --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code or modify implementation/test files.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.

Start by reading the design document and discovering available tools.
Then wait for the Director to assign your first verification task.
```

#### Role File Specifications

**roles/director.md** — Based on `/design-doc-execute` `roles/director.md`:

Responsibilities (unchanged):
- Validate design document, resolve COMMENT/FIXME markers
- Judge team composition and spawn needed members
- Orchestrate per-step TDD cycle
- Review tests against design doc (Phase A)
- Review implementation for quality and compliance (Phase C)
- Handle escalations (test defect protocol)
- Commit after each phase
- Run Phase D verification if Verifier spawned
- Verify Success Criteria before user approval
- Obtain user approval, process feedback

Changes from Agent Teams version:
- **Registration**: Load `Skill(cafleet)` and `Skill(cafleet-monitoring)`. Register, start monitoring `/loop`.
- **Communication**: All `SendMessage` → `cafleet send`. All message checking → `cafleet poll`.
- **Health checks**: `cafleet-monitoring` 2-stage protocol (poll → member capture).
- **Cleanup**: `cafleet member delete` for each member + `cafleet deregister`.

Commit Protocol Summary, User Interaction Rules (COMMENT handling, LLM intent judgment, abort detection), Escalation Protocol — unchanged from Agent Teams version. Git commands remain `git add` + `git commit` as separate Bash calls.

Progress Monitoring milestones — same phases (Test writing, Implementation, Verification, Escalation) with `cafleet send` replacing `SendMessage` in Director actions.

**roles/programmer.md** — Based on `/design-doc-execute` `roles/programmer.md`:

Responsibilities (unchanged):
- Implement code that passes all tests
- Keep design document in sync (checkboxes, timestamps, Progress counter)
- Escalate blockers immediately
- Maintain code quality

Changes:
- **Communication**: All `SendMessage(to="Director")` → `cafleet send --agent-id $CAFLEET_AGENT_ID --to $DIRECTOR_ID`.
- **Receiving assignments**: Via push notification (`cafleet poll` output).
- **Do NOT list**: Retains all prohibitions (no committing, no modifying tests, no user communication). Replaces "spawn subagents or run `claude` commands" with "spawn subagents".

FIXME Resolution, Resumption, Implementation (TDD), Escalation workflows — unchanged logic, communication method changes only.

**roles/tester.md** — Based on `/design-doc-execute` `roles/tester.md`:

Responsibilities (unchanged):
- Write comprehensive unit tests before implementation (TDD)
- Define correct contract
- Resolve test defects promptly
- Use project's existing test patterns

Changes:
- **Communication**: All `SendMessage(to="Director")` → `cafleet send`.
- **Receiving assignments**: Via push notification.
- **Do NOT list**: Same prohibitions, same communication method change.

Test Framework Selection, Test Writing (per step), Test Defect Resolution workflows — unchanged logic.

**roles/verifier.md** — Based on `/design-doc-execute` `roles/verifier.md`:

Responsibilities (unchanged):
- E2E/integration testing against success criteria
- Tool discovery and capability assessment
- Evidence collection (screenshots, logs, output)
- Graceful degradation when tools unavailable

Changes:
- **Communication**: All `SendMessage(to="Director")` → `cafleet send`.
- **Receiving assignments**: Via push notification.
- **Do NOT list**: Same prohibitions.

Tool Discovery, Verification, Graceful Degradation workflows — unchanged logic.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `ARCHITECTURE.md` with new orchestration skill descriptions <!-- completed: 2026-04-14T00:05 -->
- [x] Update `README.md` to include CAFleet-native design document orchestration skills <!-- completed: 2026-04-14T00:05 -->

### Step 2: cafleet-design-doc template skill

- [x] Create `.claude/skills/cafleet-design-doc/SKILL.md` (copy from `~/.claude/skills/design-doc/SKILL.md`, set `name: cafleet-design-doc`) <!-- completed: 2026-04-14T00:20 -->
- [x] Copy `template.md` and `guidelines.md` from `~/.claude/skills/design-doc/` to `.claude/skills/cafleet-design-doc/` <!-- completed: 2026-04-14T00:20 -->

### Step 3: Plugin infrastructure

- [x] Update `.claude-plugin/plugin.json` — add new skill paths (including `cafleet-design-doc`), bump version to 0.2.0 <!-- completed: 2026-04-14T00:35 -->
- [x] Update `.claude/skills/cafleet-monitoring/SKILL.md` — replace `SendMessage` references with `cafleet send` equivalents <!-- completed: 2026-04-14T00:35 -->
- [ ] Run `/sync-skills` to update `CLAUDE.md` and `.claude/CLAUDE.md` with new skill entries <!-- completed: -->

### Step 4: cafleet-design-doc-create SKILL.md

- [x] Create `.claude/skills/cafleet-design-doc-create/SKILL.md` with full process (Steps 0–6), spawn prompts, and role table <!-- completed: 2026-04-14T00:55 -->

### Step 5: cafleet-design-doc-create role files

- [x] Create `roles/director.md` — CAFleet-adapted Director role for create workflow <!-- completed: 2026-04-14T01:10 -->
- [x] Create `roles/drafter.md` — CAFleet-adapted Drafter role with communication protocol <!-- completed: 2026-04-14T01:10 -->
- [x] Create `roles/reviewer.md` — CAFleet-adapted Reviewer role with communication protocol <!-- completed: 2026-04-14T01:10 -->

### Step 6: cafleet-design-doc-execute SKILL.md

- [ ] Create `.claude/skills/cafleet-design-doc-execute/SKILL.md` with full process (Steps 1–6), spawn prompts, TDD cycle, and role table <!-- completed: -->

### Step 7: cafleet-design-doc-execute role files

- [ ] Create `roles/director.md` — CAFleet-adapted Director role for execute workflow <!-- completed: -->
- [ ] Create `roles/programmer.md` — CAFleet-adapted Programmer role with communication protocol <!-- completed: -->
- [ ] Create `roles/tester.md` — CAFleet-adapted Tester role with communication protocol <!-- completed: -->
- [ ] Create `roles/verifier.md` — CAFleet-adapted Verifier role with communication protocol <!-- completed: -->

### Step 8: Verification

- [ ] Verify all skill files load correctly via `Skill(cafleet-design-doc-create)` and `Skill(cafleet-design-doc-execute)` <!-- completed: -->
- [ ] Verify plugin exposes skills correctly (check `/sync-skills` output includes new plugin skills) <!-- completed: -->
- [ ] Verify existing `/design-doc-create` and `/design-doc-execute` skills still function (coexistence) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-13 | Initial draft |
| 2026-04-13 | Revision: add documentation step, fix task count, add `/loop` cancellation to shutdown, add cafleet-monitoring SKILL.md update, add implementer self-containment note, use `--json` for reliable parsing |
| 2026-04-13 | Revision: add `/cafleet-design-doc` template skill (plugin-local copy of `/design-doc`), update spawn prompts to use `Skill(cafleet-design-doc)`, add to plugin.json skills array |
