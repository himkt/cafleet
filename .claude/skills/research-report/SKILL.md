---
name: research-report
description: Create a comprehensive research report with folder-based output. Researchers write findings to individual files, the Manager compiles report.md, and the Director reviews. Output goes to researches/[topic-slug]/. After report approval, offers to chain into /research-presentation for slides and transcript. Members must always load skills using the Skill tool, not by reading skill files directly. Do NOT do a quick web search and summarize — invoke this skill for thorough, multi-source research.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, TaskCreate, TaskUpdate, TaskList, TaskGet
---

# Research Report

Generate comprehensive research reports using a multi-layer CAFleet-orchestrated team: Director → Manager → Scouts/Researchers. Every member carries serious accountability for the quality of the final deliverable, and the team iterates relentlessly until the report meets the highest standard. After the report is approved, the Director offers to chain into `/research-presentation` for slides and transcript.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Bootstrap CAFleet session, spawn all members, relay Manager requests, review all deliverables, present to user | Write the report, decompose topics, conduct research | [roles/director.md](roles/director.md) |
| **Manager** | claude pane (member) | Run orientation searches for landscape understanding and topic decomposition, request Scout/Researcher spawning from the Director, aggregate Scout and Researcher findings, compile report, revise | Conduct deep investigation — all substantive research MUST be delegated to Researchers | [roles/manager.md](roles/manager.md) |
| **Scout** | claude pane (member) | Landscape mapping — broad discovery to expand knowledge before decomposition | Collect facts for the report, write report sections | [roles/scout.md](roles/scout.md) |
| **Researcher** | claude pane (member) | Search exhaustively, collect facts with sources, filter misinformation, write findings to assigned file | Synthesize or write report sections | [roles/researcher.md](roles/researcher.md) |

## Additional resources

- For the report format specification, see [template.md](template.md)

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` and embeds them into every member's spawn prompt.

## Architecture

The Director is the root agent of a CAFleet session — bootstrapped automatically by `cafleet session create` — and spawns every member via `cafleet --session-id [session-id] member create --agent-id [director-agent-id]`. All inter-agent coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered tmux push notifications) and a shared task list.

```text
User
 +-- Director (main Claude — runs cafleet session create, cafleet member create, drives the loop)
      +-- manager (claude pane — compiles report, decomposes topic)
      +-- scout-<NN> (claude pane — landscape mapping)
      +-- researcher-NN (claude pane — deep investigation)
```

- **Director ↔ User**: `AskUserQuestion` (final report presentation, feedback collection, language disambiguation when escalated by a member)
- **Director ↔ manager**: `cafleet message send` (spawn requests, review feedback)
- **Director ↔ scout-* / researcher-***: `cafleet message send` (assignment relays, findings reports, revision requests)
- **Manager → Director**: spawn requests via `cafleet message send` (Director executes `cafleet member create` on receipt)

Members cannot talk to the user directly — the Director always relays. Members cannot talk to each other directly either — Manager requests are always mediated by the Director (Manager → Director → Scout/Researcher, and Scout/Researcher → Director → Manager).

> **Literal-UUID flag rule** — substitute the UUIDs printed by `cafleet session create` and `cafleet member create` directly into every `cafleet ...` call (the harness matches Bash invocations as literal command strings). Never store IDs in shell variables. `--session-id` is a global flag (placed BEFORE the subcommand); `--agent-id` is a per-subcommand option (placed AFTER the subcommand name). See `Skill(cafleet)` for the full convention.

## Process

### Step 0: Base Directory Selection (Director)

Before creating the team, determine where output files will be saved.

1. Load `Skill(base-dir)` and follow its procedure. (The topic argument is not a path, so the absolute-path skip rule does not apply.)
2. Compute: `${OUTPUT_DIR} = ${BASE}/researches/[topic-slug]/`
3. Create the output directory.
4. Pass `${OUTPUT_DIR}` as the resolved absolute path to the Manager and all Researchers/Scouts in their spawn prompts.

### Step 0a: Environment Precheck (Director — MANDATORY)

Run `cafleet doctor` to confirm the Director is inside a tmux session with valid pane identifiers (a hard requirement of `cafleet member create`). On non-zero exit, abort and surface the error to the user — do NOT attempt raw `tmux` probes as a workaround.

### Step 0b: Bootstrap CAFleet Session (Director — MANDATORY)

`cafleet session create` atomically creates the session, registers a root Director bound to the current tmux pane, and seeds the built-in Administrator. Capture both UUIDs from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

```bash
cafleet --json session create --label "research-[topic-slug]"
```

Capture `session_id` and `director.agent_id` from the response. Treat `session_id` as `[session-id]` and `director.agent_id` as `[director-agent-id]` for the rest of this skill.

### Step 1: Start Progress Monitor (Director — MANDATORY)

Load `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)`. Start a `/loop` monitor at a 1-minute interval BEFORE the first `cafleet member create` call so the first tick fires while the Manager is spawning. Use the agent-team-monitoring template with the literal `[session-id]` and `[director-agent-id]` UUIDs substituted in.

The loop must check `${OUTPUT_DIR}` for these expected deliverables:

- `report.md` — required final compiled report from the Manager
- `00-scout-*.md` — Scout landscape/discovery notes (one or more files may exist)
- `NN-research-*.md` — Researcher findings files for delegated sub-topics (`NN` is the assigned number; one or more files may exist)

Readiness/stall rules (apply per `Skill(cafleet:agent-team-monitoring)`):

- After Scouts/Researchers have been spawned and tasks have been assigned, expect at least one `00-scout-*.md` or `NN-research-*.md` file to appear within a couple of ticks.
- Do not consider the workflow ready for Step 5 until `report.md` exists.
- If a member owns an `in_progress` task but their deliverable file is missing past the expected milestone, run the 2-stage health-check from `Skill(cafleet:agent-team-monitoring)`: `cafleet message poll` → `cafleet member capture --lines 200` → directed `cafleet message send` nudge → user escalation.
- Keep the monitor running until Step 8.

### Step 2: Spawn Manager (Director)

Load `Skill(cafleet)` and follow its spawn protocol.

#### 2a. Shared task list

The harness task tools (`TaskCreate / TaskUpdate / TaskList / TaskGet`) are the work-coordination substrate. The on-disk task store is created on the first `TaskCreate` call (typically by the Manager when decomposing the topic). No explicit team-bootstrap step is required.

#### 2b. Read role definitions

Read the role files that will be embedded verbatim in spawn prompts (paths are relative to this skill's directory):

- `roles/manager.md`
- `roles/scout.md`
- `roles/researcher.md`

> **Template safety**: cafleet `member create` runs `str.format()` on the entire spawn prompt with `session_id` / `agent_id` / `director_name` / `director_agent_id` as kwargs. The role docs in this skill use `<...>` / `[...]` notation everywhere placeholders appear, so the embedded role content contains no literal `{` or `}` and no escaping is needed. The only single-brace tokens in the spawn prompt are the four kwargs cafleet itself substitutes: `{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}`. If you ever add a `{` or `}` to a role doc (a JSON example, a format-string example), double it to `{{` / `}}` before embedding — `str.format()` raises `KeyError` at spawn time otherwise. Do NOT shell out to `sed` / `awk` — those are blocked by the harness Bash validator.

#### 2c. Spawn the Manager

**Manager spawn prompt:**

```
You are the Manager in a research report team (CAFleet-native).

[ROLE DEFINITION]
[Content of roles/manager.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives, literal-UUID flag convention, and bash-via-Director routing

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

CURRENT DATE: [INSERT today's date]
USER REQUEST: [INSERT user's original request in full]
OUTPUT DIRECTORY: [INSERT ${OUTPUT_DIR}]
LANGUAGE: [INSERT user's language preference if specified]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- You do NOT talk to Scouts or Researchers directly. The Director spawns them and relays their findings.
- The team shares a harness task list (TaskList / TaskGet / TaskUpdate). Use it to track sub-topic assignments.

To request Scouts or Researchers, send the Director a cafleet message specifying: role (Scout or Researcher), scope, search angles, and output file path. The Director will spawn them via `cafleet member create` and relay their completion reports back to you.

Your first compiled report will be reviewed critically by the Director. Aim for highest quality on the first attempt.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "manager" \
  --description "Compiles the research report" \
  -- "[Manager spawn prompt with embedded role content (literal braces doubled)]"
```

Capture the printed `agent_id` and substitute it for `[manager-agent-id]` in every subsequent `cafleet` call that targets the Manager.

### Step 3: Knowledge Bootstrapping — Scout Phase (Director, on Manager's request)

After assessing the topic, the Manager may send the Director one or more Scout spawn requests via `cafleet message send`. For each request, the Director spawns a Scout.

**Scout spawn prompt:**

```
You are a Scout Researcher in a research team (CAFleet-native).

[ROLE DEFINITION]
[Content of roles/scout.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives and bash-via-Director routing

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

CURRENT DATE: [INSERT today's date]
YOUR ASSIGNMENT: [landscape scope and what areas to map]
OUTPUT FILE: [INSERT <resolved-path>/00-scout-<topic>.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id [task-id], then act on the instructions.

Write findings to the output file, then send the Director a completion summary. The Director will relay your findings to the Manager.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "scout-<NN>" \
  --description "Landscape scout" \
  -- "[Scout spawn prompt]"
```

(Use `scout` if only one; `scout-1`, `scout-2`, ... for multiple.) Capture the printed `agent_id` for each Scout and substitute it into subsequent `cafleet message send` calls targeting that Scout.

**Scout-Manager loop (relayed through Director):**

1. Manager sends Director a Scout spawn request via `cafleet message send`.
2. Director spawns the Scout via `cafleet member create`.
3. Scout investigates, writes findings to the output file, sends the Director a completion report via `cafleet message send`.
4. Director relays the Scout's output file path (and any summary text) to the Manager via `cafleet message send`.
5. Manager reads the Scout file and may send a follow-up request (either targeted re-scouting, a new Scout, or proceed to decomposition).

**Safety cap**: Maximum 3 Scout-Manager iterations (request → investigate → review = one iteration). After 3 iterations, the Manager must proceed to topic decomposition with the knowledge gathered so far.

### Step 4: Spawn Researchers (Director, on Manager's request)

After decomposing the topic, the Manager sends the Director one or more Researcher spawn requests via `cafleet message send`.

#### 4a. Create tasks for each sub-topic (Manager, before spawn requests)

With multiple Researchers running in parallel, coordination goes through the **shared harness task list** — not just through spawn prompts. The Manager MUST create one task per sub-topic BEFORE asking the Director to spawn the Researcher for it.

- The Manager calls `TaskCreate` for each sub-topic. Task content describes the sub-topic, scope, and the expected output file path (e.g., `<resolved-path>/01-research-<subtopic>.md`).
- Tasks start unowned. When a Researcher is spawned and given their assignment, they claim their assigned task by calling `TaskUpdate(taskId, owner: "researcher-NN")` and marking it `in_progress`.
- Researchers mark their task `completed` when their output file is written and the completion report has been sent.
- The Manager blocks on all research tasks being `completed` before starting compilation. Use `TaskList` to check progress.

The Manager's `TaskCreate` calls also serve as the authoritative list of sub-topic scopes — if the Director sees a discrepancy between a spawn request's scope and the corresponding task, treat the task description as canonical and ask the Manager to reconcile.

#### 4b. Spawn each Researcher (Director)

**Researcher spawn prompt:**

```
You are a Research Specialist in a research team (CAFleet-native).

[ROLE DEFINITION]
[Content of roles/researcher.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives and bash-via-Director routing

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

CURRENT DATE: [INSERT today's date]
YOUR NAME: researcher-NN
YOUR ASSIGNMENT: [specific sub-topic and what to investigate]
YOUR TASK ID: [INSERT the taskId the Manager created for this sub-topic]
OUTPUT FILE: [INSERT <resolved-path>/NN-research-<subtopic>.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- On start, claim your task: TaskUpdate(taskId: YOUR TASK ID, owner: "researcher-NN", status: "in_progress").
- On completion, mark your task completed: TaskUpdate(taskId: YOUR TASK ID, status: "completed").

Write findings to the output file, then send the Director a completion summary. The Director will relay findings and any follow-up questions between you and the Manager.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "researcher-NN" \
  --description "Researcher for sub-topic <slug>" \
  -- "[Researcher spawn prompt]"
```

The Director repeats this step whenever the Manager requests additional Researchers (for coverage gaps, failed investigations, or revision-driven re-research). Any new Researcher must first have a task created by the Manager; the Director includes the `taskId` in the spawn prompt.

### Step 5: Review & Revision Loop (Director ↔ Manager, via `cafleet message send`)

When the Manager delivers the compiled `report.md`:

1. The Director reads `${OUTPUT_DIR}/report.md` and reviews it critically against the checklist in [roles/director.md](roles/director.md).
2. The Director sends tagged feedback to the Manager:
   ```bash
   cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
     --to [manager-agent-id] \
     --text "review feedback round <N>: [FACTUAL ERROR] ... / [GAP] ... / ..."
   ```
3. The Manager revises the report (requesting additional Researchers from the Director as needed) and sends a completion message back via `cafleet message send`.
4. Each polled inbound message MUST be `ack`ed via `cafleet --session-id [session-id] message ack --agent-id [director-agent-id] --task-id [task-id]` after acting on it. Un-acked messages stay in `INPUT_REQUIRED` and re-surface on every subsequent `message poll` cycle.
5. Repeat until the Director judges quality is sufficient. Aim for 2–3 rounds maximum.

If the Manager asks the Director a question that is really a user decision (e.g. language choice, scope trade-off), the Director MUST relay via `AskUserQuestion` and pass the user's verbatim answer back via `cafleet message send`. Never decide on the user's behalf.

### Step 6: Present to User (Director)

Present the approved report to the user via `AskUserQuestion` with: a summary of findings (2–3 sentences), file paths (report, scout files, researcher files), known limitations, and a request for feedback. If the user provides feedback, route it to the Manager via `cafleet message send`, re-review, and re-present. Repeat until the user approves.

### Step 7: Offer Presentation Chaining (Director)

After user approval, offer to create a presentation via `AskUserQuestion` (adapt to user's language). If yes, proceed to Step 8, then invoke `/research-presentation ${OUTPUT_DIR}`. If no, proceed directly to Step 8.

### Step 8: Finalize & Clean Up (Director)

Follow the Shutdown Protocol in `Skill(cafleet)` § *Shutdown Protocol*. Order matters — every step before `cafleet session delete` must complete first, otherwise crons fire against dead members or orphan `claude` processes linger.

1. **Cancel the `/loop` monitor** with `CronDelete <job-id>`. The cron must stop firing BEFORE any member is deleted; a cron that keeps polling a tearing-down session spams `Error: session is deleted` and races with member-delete.
2. **Delete every member** in dependency order — Researchers first, then any active Scout, then the Manager:
   ```bash
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [researcher-agent-id]
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [scout-agent-id]
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [manager-agent-id]
   ```
   Each call sends `/exit` to the pane and waits up to 15 s for it to close. On exit 2 (timeout), the pane buffer tail is printed on stderr — inspect with `cafleet member capture`, answer any prompt with `cafleet member send-input`, then re-run. As a last resort, rerun with `--force` to skip the wait and kill-pane immediately.
3. **Verify the roster is empty**:
   ```bash
   cafleet --session-id [session-id] member list --agent-id [director-agent-id]
   ```
   If anyone remains, repeat step 2 for that member.
4. **Delete the session**:
   ```bash
   cafleet session delete [session-id]
   ```
   This soft-deletes the session and deregisters the root Director, Administrator, and any surviving members in one transaction.
5. **Confirm**:
   ```bash
   cafleet session list
   ```
   The current session must not appear (soft-deleted sessions are hidden).

Do NOT use raw `tmux kill-pane` or `tmux send-keys` at any point — `cafleet member delete` and `cafleet member capture` / `cafleet member send-input` are the only supported teardown and recovery primitives.

$ARGUMENTS
