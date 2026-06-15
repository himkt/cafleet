---
name: cafleet-research-report
description: Create a comprehensive research report with folder-based output. Researchers write findings to individual files, the Manager compiles report.md, and the Director reviews. Output goes to researches/[topic-slug]/. After report approval, offers to chain into the cafleet-research-presentation skill for slides and transcript. Members must always load skills using the Skill tool, not by reading skill files directly. Do NOT do a quick web search and summarize — invoke this skill for thorough, multi-source research.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch, Agent, TaskCreate, TaskUpdate, TaskList, TaskGet
---

# Research Report

Generate comprehensive research reports using a multi-layer CAFleet-orchestrated team: Director → Manager → Scouts/Researchers. Every member carries serious accountability for the quality of the final deliverable, and the team iterates relentlessly until the report meets the highest standard. After the report is approved, the Director offers to chain into the `cafleet-research-presentation` skill for slides and transcript.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Bootstrap CAFleet fleet, spawn all members, relay Manager requests, review all deliverables, present to user | Write the report, decompose topics, conduct research | [roles/director.md](roles/director.md) |
| **Manager** | claude pane (member) | Run orientation searches for landscape understanding and topic decomposition, request Scout/Researcher spawning from the Director, aggregate Scout and Researcher findings, compile report, revise | Conduct deep investigation — all substantive research MUST be delegated to Researchers | [roles/manager.md](roles/manager.md) |
| **Scout** | claude pane (member) | Landscape mapping — broad discovery to expand knowledge before decomposition | Collect facts for the report, write report sections | [roles/scout.md](roles/scout.md) |
| **Researcher** | claude pane (member) | Search exhaustively, collect facts with sources, filter misinformation, write findings to assigned file | Synthesize or write report sections | [roles/researcher.md](roles/researcher.md) |

## Additional resources

- For the report format specification, see [template.md](template.md)

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads the `cafleet` skill and the `cafleet-agent-team-monitoring` skill and embeds them into every member's spawn prompt. The fleet runs a dedicated monitoring member (the first `member create`, `--role monitor --model sonnet`) that owns the heartbeat and re-engages the idle Director — see Step 1.

## Output

The skill writes its working folder to `<CWD>/researches/<topic-slug>/` (one folder per research run, containing `report.md` and per-researcher files). Callers MUST add `researches/` to their per-project `.gitignore` before invoking this skill — the skill does not create or modify `.gitignore` itself, and the working folder is meant to stay out of version control.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` — and spawns every member via `cafleet member create --fleet-id [fleet-id] --agent-id [director-agent-id]`. All inter-agent coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered tmux push notifications) and a shared task list.

```text
User
 +-- Director (main Claude — runs cafleet fleet create, cafleet member create, drives the loop)
      +-- manager (claude pane — compiles report, decomposes topic)
      +-- scout-<NN> (claude pane — landscape mapping)
      +-- researcher-NN (claude pane — deep investigation)
```

Members cannot talk to the user directly — the Director always relays. Members cannot talk to each other directly either — Manager requests are always mediated by the Director (Manager → Director → Scout/Researcher, and Scout/Researcher → Director → Manager).

> **Literal-integer-id flag rule** — substitute the integer ids printed by `cafleet fleet create` and `cafleet member create` directly into every `cafleet ...` call (the harness matches Bash invocations as literal command strings). Never store IDs in shell variables. `--fleet-id` and `--agent-id` are per-subcommand options (placed AFTER the subcommand name). See the `cafleet` skill for the full convention.

## Process

### Step 0: Base Directory Selection (Director)

Before creating the team, resolve the task-scoped base directory for this run.

1. Load the `cafleet-base-dir` skill for the no-bypass write protocol and `<unset>` sentinel contract.
2. Resolve the task-scoped BASE by calling the resolver positionally with the topic relpath:

   Run the skill's **Step 0 (task-scope resolution)** with the relpath `researches/[topic-slug]`.

   Step 0 infers the repo root via `git rev-parse --show-toplevel`, joins `researches/[topic-slug]` under it, and resolves `${BASE}` to that absolute task folder. The resolver writes nothing at resolution time; `${BASE}` is created by the first write under it — the Director's spawn-prompt audit write to `${BASE}/prompts/<role>-<UTC-compact>.md`, which lands before any member spawns, so the output directory already exists by the time Researchers write to it (matching `roles/researcher.md` § File Output). Use that `${BASE}` for the rest of this run.
3. `${OUTPUT_DIR} = ${BASE}` — the task folder IS the output directory. There is no further concatenation.
4. Pass `${OUTPUT_DIR}` (i.e., `${BASE}`) as the resolved absolute path to the Manager and all Researchers/Scouts in their spawn prompts. The audit-file path `${BASE}/prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<topic-folder>/prompts/`, not under the repo root.

### Step 0a: Environment Precheck (Director — MANDATORY)

Run `cafleet doctor` to confirm the Director is inside a tmux session with valid pane identifiers (a hard requirement of `cafleet member create`). On non-zero exit, abort and surface the error to the user — do NOT attempt raw `tmux` probes as a workaround.

### Step 0b: Bootstrap CAFleet Fleet (Director — MANDATORY)

`cafleet fleet create` atomically creates the fleet, registers a root Director bound to the current tmux pane, and seeds the built-in Administrator. Capture both integer ids from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

```bash
cafleet --json fleet create --label "research-[topic-slug]"
```

Capture `fleet_id` and `director.agent_id` from the response. Treat `fleet_id` as `[fleet-id]` and `director.agent_id` as `[director-agent-id]` for the rest of this skill.

### Step 1: Supervision Model (Director — spawn the monitoring member first)

Load the `cafleet` skill and the `cafleet-agent-team-monitoring` skill for their heartbeat, facilitation, and Stall Response policy. The **first** `cafleet member create` in the fleet is the dedicated monitoring member, spawned with `--role monitor --model sonnet`. It launches `cafleet monitor start --fleet-id [fleet-id]` as a background task in its own pane, confirms with `cafleet monitor status`, and reports `ready: monitor live` to the Director. **Receipt of that handshake gates the Manager / Scout / Researcher spawns** — do not spawn an ordinary member until `ready: monitor live` has arrived (first-in). The Director does **not** run `cafleet monitor start` itself.

Render the canonical monitoring-member spawn prompt (the **conditional** idle-nudge routine — re-engage the Director via `cafleet member nudge` only when un-acked inbox items or stalled members can be named) to a `--prompt-file` per the audit-file pattern this skill uses for every spawn, then spawn:

```bash
cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
  --name "monitor" \
  --description "Monitoring member — runs the heartbeat and re-engages the idle Director" \
  --role monitor --model sonnet \
  --prompt-file ${BASE}/prompts/monitor-<UTC-compact>.md
```

See the `cafleet-agent-team-monitoring` skill § The monitoring member for the canonical spawn prompt and lifecycle. The monitoring member is stopped and deleted first in the Step 8 teardown (first-out).

On each active turn, check `${OUTPUT_DIR}` for these expected deliverables:

- `report.md` — required final compiled report from the Manager
- `00-scout-*.md` — Scout landscape/discovery notes (one or more files may exist)
- `NN-research-*.md` — Researcher findings files for delegated sub-topics (`NN` is the assigned number; one or more files may exist)

Readiness/stall rules (apply per the `cafleet-agent-team-monitoring` skill):

- After Scouts/Researchers have been spawned and tasks have been assigned, expect at least one `00-scout-*.md` or `NN-research-*.md` file to appear after the first round of replies.
- Do not consider the workflow ready for Step 5 until `report.md` exists.
- If a member owns an `in_progress` task but their deliverable file is missing past the expected milestone, run the 2-stage health-check from the `cafleet-agent-team-monitoring` skill: `cafleet message poll` → `cafleet member capture --lines 200` → directed `cafleet message send` nudge → user escalation.

### Step 2: Spawn Manager (Director)

Load the `cafleet` skill and follow its spawn protocol.

#### 2a. Shared task list

The harness task tools (`TaskCreate / TaskUpdate / TaskList / TaskGet`) are the work-coordination substrate. The on-disk task store is created on the first `TaskCreate` call (typically by the Manager when decomposing the topic). No explicit team-bootstrap step is required.

#### 2b. Locate role definitions (path-by-reference)

The Director references each role definition by its **absolute path** in the spawn prompt — the spawned member opens its role doc with `Read` at startup. Do NOT inline the role content into the prompt. Resolve the absolute path for each role file once (the role files live in this skill's `roles/` directory):

- `<abs path to this skill>/roles/manager.md`
- `<abs path to this skill>/roles/scout.md`
- `<abs path to this skill>/roles/researcher.md`

Substitute these absolute paths into the spawn prompts below.

> **Spawn mechanics**: path-by-reference is required because cafleet `member create` passes the prompt to `tmux split-window` as one positional arg and fails with `command too long` past a few KB (see the `cafleet` skill's `reference/director.md` § *Spawn prompt size limit*). `str.format()` runs over the prompt with `fleet_id` / `agent_id` / `director_agent_id` as kwargs — leave those single-braced, double any other literal `{` / `}`. **Two-step audit file**: write the rendered prompt to `${BASE}/prompts/<role>-<UTC-compact>.md` BEFORE `cafleet member create --prompt-file <abs path>` (the pre-spawn file IS both the CLI input and the permanent audit artifact); see the `cafleet-base-dir` skill § *No-bypass write protocol* and `reference/director.md` § *Member Create — Scratch and audit files* for the contract incl. the `${BASE} == <unset>` guarded-skip + inline fallback.

#### 2c. Spawn the Manager

**Manager spawn prompt:**

```
You are the Manager in a research report team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/manager.md] with the Read tool BEFORE any other action. That file is your authoritative role definition — accountability, communication protocol, task discipline, file-aggregation rules, pre-compilation verification, revision loop, and shutdown. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives, literal-integer-id flag convention, and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]

CURRENT DATE: [INSERT today's date]
USER REQUEST: [INSERT user's original request in full]
OUTPUT DIRECTORY: [INSERT OUTPUT DIRECTORY]
LANGUAGE: [INSERT user's language preference if specified]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` integer id from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- You do NOT talk to Scouts or Researchers directly. The Director spawns them and relays their findings.
- The team shares a harness task list (TaskList / TaskGet / TaskUpdate). Use it to track sub-topic assignments.

To request Scouts or Researchers, send the Director a cafleet message specifying: role (Scout or Researcher), scope, search angles, and output file path. The Director will spawn them via `cafleet member create` and relay their completion reports back to you.

Your first compiled report will be reviewed critically by the Director. Aim for highest quality on the first attempt.
```

Render the prompt to `${BASE}/prompts/manager-<UTC-compact>.md` per the 2b two-step audit-file pattern (leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "manager" \
     --description "Compiles the research report" \
     --prompt-file ${BASE}/prompts/manager-<UTC-compact>.md
   ```

   Capture the printed `agent_id` and substitute it for `[manager-agent-id]` in every subsequent `cafleet` call that targets the Manager.

### Step 3: Knowledge Bootstrapping — Scout Phase (Director, on Manager's request)

After assessing the topic, the Manager may send the Director one or more Scout spawn requests via `cafleet message send`. For each request, the Director spawns a Scout.

**Scout spawn prompt:**

```
You are a Scout Researcher in a research team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/scout.md] with the Read tool BEFORE any other action. That file is your authoritative role definition — landscape-mapping focus, communication protocol, output format, and shutdown. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]

CURRENT DATE: [INSERT today's date]
YOUR ASSIGNMENT: [landscape scope and what areas to map]
OUTPUT FILE: [INSERT <resolved-path>/00-scout-<topic>.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` integer id from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.

Write findings to the output file, then send the Director a completion summary. The Director will relay your findings to the Manager.
```

Render the prompt to `${BASE}/prompts/<scout-name>-<UTC-compact>.md` per the 2b two-step audit-file pattern (use `scout` for a single Scout, `scout-1`/`scout-2`/… for multiple; `<scout-name>` is the lowercased `--name`), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "scout-<NN>" \
     --description "Landscape scout" \
     --prompt-file ${BASE}/prompts/scout-<NN>-<UTC-compact>.md
   ```

   Capture the printed `agent_id` for each Scout and substitute it into subsequent `cafleet message send` calls targeting that Scout.

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

ROLE DEFINITION: Open [INSERT abs path to roles/researcher.md] with the Read tool BEFORE any other action. That file is your authoritative role definition — accountability, Discovery Phase, fact verification protocol, output format, and shutdown. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]

CURRENT DATE: [INSERT today's date]
YOUR NAME: researcher-NN
YOUR ASSIGNMENT: [specific sub-topic and what to investigate]
YOUR TASK ID: [INSERT the taskId the Manager created for this sub-topic]
OUTPUT FILE: [INSERT <resolved-path>/NN-research-<subtopic>.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` integer id from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- On start, claim your task: TaskUpdate(taskId: YOUR TASK ID, owner: "researcher-NN", status: "in_progress").
- On completion, mark your task completed: TaskUpdate(taskId: YOUR TASK ID, status: "completed").

Write findings to the output file, then send the Director a completion summary. The Director will relay findings and any follow-up questions between you and the Manager.
```

Render the prompt to `${BASE}/prompts/researcher-<NN>-<UTC-compact>.md` per the 2b two-step audit-file pattern, then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "researcher-NN" \
     --description "Researcher for sub-topic <slug>" \
     --prompt-file ${BASE}/prompts/researcher-NN-<UTC-compact>.md
   ```

   The Director repeats this step whenever the Manager requests additional Researchers (coverage gaps, failed investigations, revision-driven re-research). Any new Researcher must first have a task created by the Manager; the Director includes the `taskId` in the spawn prompt.

### Step 5: Review & Revision Loop (Director ↔ Manager, via `cafleet message send`)

When the Manager delivers the compiled `report.md`:

1. The Director reads `${OUTPUT_DIR}/report.md` and reviews it critically against the checklist in [roles/director.md](roles/director.md).
2. The Director sends tagged feedback to the Manager:
   ```bash
   cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --to [manager-agent-id] \
     --text "review feedback round <N>: [FACTUAL ERROR] ... / [GAP] ... / ..."
   ```
3. The Manager revises the report (requesting additional Researchers from the Director as needed) and sends a completion message back via `cafleet message send`.
4. Each polled inbound message MUST be `ack`ed via `cafleet message ack --fleet-id [fleet-id] --agent-id [director-agent-id] --task-id [task-id]` after acting on it. Un-acked messages stay in `INPUT_REQUIRED` and re-surface on every subsequent `message poll` cycle.
5. Repeat until the Director judges quality is sufficient. Aim for 2–3 rounds maximum.

If the Manager asks the Director a question that is really a user decision (e.g. language choice, scope trade-off), the Director MUST relay via `AskUserQuestion` and pass the user's verbatim answer back via `cafleet message send`. Never decide on the user's behalf.

### Step 6: Present to User (Director)

Present the approved report to the user via `AskUserQuestion` with: a summary of findings (2–3 sentences), file paths (report, scout files, researcher files), known limitations, and a request for feedback. If the user provides feedback, route it to the Manager via `cafleet message send`, re-review, and re-present. Repeat until the user approves.

### Step 7: Offer Presentation Chaining (Director)

After user approval, offer to create a presentation via `AskUserQuestion` (adapt to user's language). If yes, proceed to Step 8, then invoke the `cafleet-research-presentation` skill with `${OUTPUT_DIR}`. If no, proceed directly to Step 8.

### Step 8: Finalize & Clean Up (Director)

Follow the Shutdown Protocol in the `cafleet` skill § *Shutdown Protocol* (first-out): stop the monitoring member's `monitor start` background task and wait for confirmation; `cafleet member delete` the monitoring member first, then Researchers, any active Scout, and the Manager (each sends `/exit` and waits 15 s; on exit 2 use `cafleet member capture` + `cafleet member send-input` recovery, or `--force`); `cafleet member list` to verify the roster is empty; `cafleet fleet delete [fleet-id]`; `cafleet fleet list` to confirm. Never use raw `tmux kill-pane` / `tmux send-keys`.

## Spawnable Agents

### web-researcher

This skill ships an embedded agent spec for parallel web research that returns structured summaries with sources. The spec is reproduced verbatim below so it is reachable from both Claude Code (load the `cafleet-research-report` skill then dispatch via `Agent`) and codex (via plugin auto-discovery — see *Dispatching this agent (codex inline-follow)* and *Dispatching this agent (codex member-spawn)* below).

    ---
    name: web-researcher
    description: Use this agent to research topics on the web before specification development. Supports parallel research of multiple topics. Returns structured summaries with sources. Best used in combination with the cafleet-design-doc-create skill - run web-researcher first to gather context, then pass results to the cafleet-design-doc-create skill.
    model: sonnet
    color: blue
    ---

    You are a web research specialist focused on gathering accurate, up-to-date information to support specification development and technical decision-making.

    ## Your Core Mission

    Efficiently research topics on the web and provide structured, actionable summaries that can be used as input for specification documents.

    ---

    ## Input Format

    A single topic (`Research: <topic>` + `Context: <why this is needed>`) or multiple topics (a numbered list + a shared `Context:`) for parallel research.

    ---

    ## Research Process

    ### Step 0: Discovery Phase

    **Before topic-specific queries, run broad date-anchored searches to bridge your knowledge cutoff to the current date** — at least 3, e.g. `"{topic} {current_year}"`, `"{topic} latest news"`, `"{topic} announced {current_year}"`, `"{topic} {current_month} {current_year}"`. If nothing significant surfaces, try ≥2 alternative patterns before concluding. Document results in a **"Discovery Phase Findings"** section at the top of your output file (or state that none were found), and use them to inform query formulation.

    ### Steps 1–4: Research

    Formulate queries (key terms + alternative phrasings + Discovery findings); execute searches (**all WebSearch calls in parallel for multiple topics**; primary + follow-up per topic); prioritize sources by reliability (official docs → reputable publications → GitHub → community forums); synthesize per topic — key facts, technical specs, best practices, pitfalls, alternatives.

    ---

    ## Output Format

    Always return results in this structured format:

    ```markdown
    # Research Results

    ## Topic: <topic name>

    ### Summary
    <2-3 sentence overview>

    ### Key Findings
    - <finding 1>
    - <finding 2>
    - <finding 3>

    ### Technical Details
    <relevant specifications, APIs, configurations, etc.>

    ### Recommendations
    <actionable recommendations based on findings>

    ### Sources
    - [Source Title](URL)
    - [Source Title](URL)

    ---

    ## Topic: <next topic>
    ...
    ```

    ---

    ## Language Selection

    As a teammate, use the language specified by the Manager/Director (default English); standalone, ask the user via `AskUserQuestion` (English default / Japanese / Other). Write all output in the selected language; technical terms and source URLs stay as-is.

    ---

    ## Research Quality Guidelines

    Accuracy (cross-reference multiple sources), currency (prefer the last 1–2 years for fast-moving topics), relevance to the given context, completeness (benefits AND drawbacks/limitations), and actionability (specifics that inform decisions).

#### Dispatching this agent (Claude Code recipe)

On Claude Code, dispatch the embedded `web-researcher` spec via the `Agent` tool with `subagent_type="general-purpose"`. Paste the spec body verbatim into the `prompt` field, then append the per-call inputs (the research topic(s) + context):

```
Agent(
  subagent_type="general-purpose",
  description="Web research on <topic>",
  prompt="""<paste the web-researcher spec body verbatim>

Research: <topic>
Context: <why this information is needed>"""
)
```

This is the post-promotion equivalent of the named `Agent(subagent_type="web-researcher")` call that worked when `web-researcher` lived as a standalone `.claude/agents/web-researcher.md`. The structured `subagent_type` name is lost (Claude Code's plugin loader does not register skill-embedded agent specs as named subagents), but the behavior is identical because the spec body is the same.

#### Dispatching this agent (codex)

On codex (which reads SKILL.md directly — see `docs/reference/coding-agents/codex.md`), either **inline-follow** (the agent reads the embedded `## Spawnable Agents > web-researcher` block and follows the spec in its own turn, no new agent spawned) or **member-spawn** a dedicated codex member via `cafleet member create --coding-agent codex` with the spec body pasted into the positional prompt argument (positional `[PROMPT_ARGV]...`; there is no `--spawn-prompt-from-text` flag).

$ARGUMENTS
