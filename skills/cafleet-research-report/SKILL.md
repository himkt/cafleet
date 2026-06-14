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

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads the `cafleet` skill and the `cafleet-agent-team-monitoring` skill and embeds them into every member's spawn prompt.

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

- **Director ↔ User**: `AskUserQuestion` (final report presentation, feedback collection, language disambiguation when escalated by a member)
- **Director ↔ manager**: `cafleet message send` (spawn requests, review feedback)
- **Director ↔ scout-* / researcher-***: `cafleet message send` (assignment relays, findings reports, revision requests)
- **Manager → Director**: spawn requests via `cafleet message send` (Director executes `cafleet member create` on receipt)

Members cannot talk to the user directly — the Director always relays. Members cannot talk to each other directly either — Manager requests are always mediated by the Director (Manager → Director → Scout/Researcher, and Scout/Researcher → Director → Manager).

> **Literal-UUID flag rule** — substitute the UUIDs printed by `cafleet fleet create` and `cafleet member create` directly into every `cafleet ...` call (the harness matches Bash invocations as literal command strings). Never store IDs in shell variables. `--fleet-id` and `--agent-id` are per-subcommand options (placed AFTER the subcommand name). See the `cafleet` skill for the full convention.

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

`cafleet fleet create` atomically creates the fleet, registers a root Director bound to the current tmux pane, and seeds the built-in Administrator. Capture both UUIDs from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

```bash
cafleet --json fleet create --label "research-[topic-slug]"
```

Capture `fleet_id` and `director.agent_id` from the response. Treat `fleet_id` as `[fleet-id]` and `director.agent_id` as `[director-agent-id]` for the rest of this skill.

### Step 1: Start Progress Monitor (Director — MANDATORY)

Load the `cafleet` skill and the `cafleet-agent-team-monitoring` skill. Run the monitor as a background task with `cafleet monitor start --fleet-id [fleet-id]` BEFORE the first `cafleet member create` call so the heartbeat is running while the Manager is spawning (confirm with `cafleet monitor status --fleet-id [fleet-id]`).

The loop must check `${OUTPUT_DIR}` for these expected deliverables:

- `report.md` — required final compiled report from the Manager
- `00-scout-*.md` — Scout landscape/discovery notes (one or more files may exist)
- `NN-research-*.md` — Researcher findings files for delegated sub-topics (`NN` is the assigned number; one or more files may exist)

Readiness/stall rules (apply per the `cafleet-agent-team-monitoring` skill):

- After Scouts/Researchers have been spawned and tasks have been assigned, expect at least one `00-scout-*.md` or `NN-research-*.md` file to appear within a couple of ticks.
- Do not consider the workflow ready for Step 5 until `report.md` exists.
- If a member owns an `in_progress` task but their deliverable file is missing past the expected milestone, run the 2-stage health-check from the `cafleet-agent-team-monitoring` skill: `cafleet message poll` → `cafleet member capture --lines 200` → directed `cafleet message send` nudge → user escalation.
- Keep the monitor running until Step 8.

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

> **Why path-by-reference (and not inline-verbatim)**: cafleet `member create` passes the prompt to `tmux split-window` as a single positional argument. tmux fails with `command too long` once the shell-quoted prompt grows past a few KB, and cafleet rolls back the agent registration. A role file is typically large enough (5–15 KB) that inlining it exceeds the limit. The member loads the role file via `Read` on its first turn; the file lives in the skill directory and is stable, so this is safe. See the `cafleet` skill's `reference/director.md` reference file § *Spawn prompt size limit* for the canonical write-up.
>
> **Template safety (str.format placeholders)**: cafleet `member create` runs `str.format()` over the entire spawn prompt (whether it arrived via `--prompt-file` or inline `prompt_argv`) with `fleet_id` / `agent_id` / `director_agent_id` as kwargs. Leave those three single-braced. Double any other literal `{` or `}` in the prompt body (a JSON example, a `${{VAR}}` reference) to `{{` / `}}`. The prompt body rarely needs `{` or `}` at all, since role content is loaded via `Read` rather than embedded in the prompt.
>
> **Spawn-prompt audit file**: every spawn in this skill writes the rendered prompt to `${BASE}/prompts/<role>-<UTC-compact>.md` BEFORE invoking `cafleet member create --prompt-file <abs path>` (see the per-role flow below). The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet-base-dir` skill § *No-bypass write protocol* and the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

#### 2c. Spawn the Manager

**Manager spawn prompt:**

```
You are the Manager in a research report team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/manager.md] with the Read tool BEFORE any other action. That file is your authoritative role definition — accountability, communication protocol, task discipline, file-aggregation rules, pre-compilation verification, revision loop, and shutdown. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives, literal-UUID flag convention, and bash-via-Director routing

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
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- You do NOT talk to Scouts or Researchers directly. The Director spawns them and relays their findings.
- The team shares a harness task list (TaskList / TaskGet / TaskUpdate). Use it to track sub-topic assignments.

To request Scouts or Researchers, send the Director a cafleet message specifying: role (Scout or Researcher), scope, search angles, and output file path. The Director will spawn them via `cafleet member create` and relay their completion reports back to you.

Your first compiled report will be reviewed critically by the Director. Aim for highest quality on the first attempt.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted and any literal `{` / `}` doubled. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact — the CLI's `str.format()` pass resolves them at member-create time using the newly-allocated `agent_id`.
2. **Write the rendered text** to `${BASE}/prompts/manager-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 0; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Create `${BASE}/prompts/` on first write (Python: `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "manager" \
     --description "Compiles the research report" \
     --prompt-file ${BASE}/prompts/manager-<UTC-compact>.md
   ```

   Capture the printed `agent_id` and substitute it for `[manager-agent-id]` in every subsequent `cafleet` call that targets the Manager. The pre-spawn file at `${BASE}/prompts/manager-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

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
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.

Write findings to the output file, then send the Director a completion summary. The Director will relay your findings to the Manager.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern. Use `scout` if only one Scout will be spawned this run; `scout-1`, `scout-2`, … for multiple — the `<role>` segment in the audit-file path matches the `--name` value (lowercased):

1. **Render the prompt locally** with all `[INSERT …]` markers substituted and any literal `{` / `}` doubled. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/<scout-name>-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 0; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`; `<scout-name>` matches the lowercased `--name` value, e.g., `scout-1`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "scout-<NN>" \
     --description "Landscape scout" \
     --prompt-file ${BASE}/prompts/scout-<NN>-<UTC-compact>.md
   ```

   Capture the printed `agent_id` for each Scout and substitute it into subsequent `cafleet message send` calls targeting that Scout. The pre-spawn file at `${BASE}/prompts/<scout-name>-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

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
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet message ack --fleet-id {fleet_id} --agent-id {agent_id} --task-id [task-id], then act on the instructions.
- On start, claim your task: TaskUpdate(taskId: YOUR TASK ID, owner: "researcher-NN", status: "in_progress").
- On completion, mark your task completed: TaskUpdate(taskId: YOUR TASK ID, status: "completed").

Write findings to the output file, then send the Director a completion summary. The Director will relay findings and any follow-up questions between you and the Manager.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern — the `<role>` segment in the audit-file path matches the `--name` value (lowercased), so each Researcher gets its own timestamped file:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted and any literal `{` / `}` doubled. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/researcher-<NN>-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 0; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "researcher-NN" \
     --description "Researcher for sub-topic <slug>" \
     --prompt-file ${BASE}/prompts/researcher-NN-<UTC-compact>.md
   ```

   The Director repeats this step whenever the Manager requests additional Researchers (for coverage gaps, failed investigations, or revision-driven re-research). Any new Researcher must first have a task created by the Manager; the Director includes the `taskId` in the spawn prompt. The pre-spawn file at `${BASE}/prompts/researcher-NN-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

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

Follow the Shutdown Protocol in the `cafleet` skill § *Shutdown Protocol*. Order matters — every step before `cafleet fleet delete` must complete first, otherwise the monitor keystrokes polls against dead members or orphan `claude` processes linger.

1. **Stop the monitor's background task** (there is no `monitor stop` command). The monitor must stop BEFORE any member is deleted; a monitor that keeps keystroking polls into a tearing-down fleet races with member-delete.
2. **Delete every member** in dependency order — Researchers first, then any active Scout, then the Manager:
   ```bash
   cafleet member delete --fleet-id [fleet-id] --member-id [researcher-agent-id]
   cafleet member delete --fleet-id [fleet-id] --member-id [scout-agent-id]
   cafleet member delete --fleet-id [fleet-id] --member-id [manager-agent-id]
   ```
   Each call sends `/exit` to the pane and waits up to 15 s for it to close. On exit 2 (timeout), the pane buffer tail is printed on stderr — inspect with `cafleet member capture`, answer any prompt with `cafleet member send-input`, then re-run. As a last resort, rerun with `--force` to skip the wait and kill-pane immediately.
3. **Verify the roster is empty**:
   ```bash
   cafleet member list --fleet-id [fleet-id]
   ```
   If anyone remains, repeat step 2 for that member.
4. **Delete the fleet**:
   ```bash
   cafleet fleet delete [fleet-id]
   ```
   This soft-deletes the fleet and deregisters the root Director, Administrator, and any surviving members in one transaction.
5. **Confirm**:
   ```bash
   cafleet fleet list
   ```
   The current fleet must not appear (soft-deleted fleets are hidden).

Do NOT use raw `tmux kill-pane` or `tmux send-keys` at any point — `cafleet member delete` and `cafleet member capture` / `cafleet member send-input` are the only supported teardown and recovery primitives.

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

    You will receive research requests in one of these formats:

    ### Single Topic
    ```
    Research: <topic>
    Context: <why this information is needed>
    ```

    ### Multiple Topics (for parallel research)
    ```
    Research the following topics:
    1. <topic 1>
    2. <topic 2>
    3. <topic 3>

    Context: <overall context>
    ```

    ---

    ## Research Process

    ### Step 0: Discovery Phase

    **Before formulating any topic-specific queries, execute broad searches to discover recent developments you may not know about.** Your training data has a knowledge cutoff — this phase bridges the gap between your knowledge and the current date.

    Execute at least 3 searches using date-anchored patterns:
    - `"{topic} {current_year}"` — events in the current year
    - `"{topic} latest news"` or `"{topic} latest developments"` — recent coverage
    - `"{topic} announced {current_year}"` or `"{topic} released {current_year}"` — launches and releases
    - `"{topic} {current_month} {current_year}"` — very recent events
    - `"{topic} update"` or `"{topic} new"` — catch remaining updates

    If these initial searches surface no significant new developments, try at least 2 additional searches with alternative query patterns (synonyms, related terms, different date granularity) before concluding.

    Document your discovery results in a **"Discovery Phase Findings"** section at the top of your output file — list what you found, or explicitly state that no recent developments were found after exhausting all search patterns. Use these discoveries to inform your query formulation in Step 1.

    ### Step 1: Query Formulation

    For each topic:
    - Identify key search terms
    - Consider alternative phrasings
    - **Incorporate Discovery Phase findings**: Add search queries specifically targeting events, papers, or releases discovered in Step 0

    ### Step 2: Parallel Search Execution

    **IMPORTANT: When researching multiple topics, execute all WebSearch calls in parallel.**

    For each topic, perform:
    1. Primary search with main keywords
    2. Follow-up search if initial results are insufficient

    ### Step 3: Source Evaluation

    Prioritize sources by reliability:
    1. Official documentation
    2. Reputable tech blogs and publications
    3. GitHub repositories and discussions
    4. Community forums (Stack Overflow, Reddit summaries)

    ### Step 4: Information Synthesis

    For each topic, extract:
    - Key facts and findings
    - Technical specifications or requirements
    - Best practices or recommendations
    - Potential pitfalls or considerations
    - Relevant alternatives or comparisons

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

    Determine the output language at the start of each research session:

    - **If running as a teammate**: Use the language specified by the Manager/Director. Default to English if not specified.
    - **If running standalone**: Ask the user via `AskUserQuestion` with options: English (default), Japanese, or Other.

    Write all research output (summaries, findings, recommendations) in the selected language. Technical terms and source URLs remain as-is regardless of language choice.

    ---

    ## Research Quality Guidelines

    1. **Accuracy**: Cross-reference information across multiple sources
    2. **Currency**: Prefer recent information (within the last 1-2 years) for rapidly evolving topics
    3. **Relevance**: Focus on information directly applicable to the context provided
    4. **Completeness**: Cover both benefits and drawbacks/limitations
    5. **Actionability**: Include specific details that can inform decisions

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

#### Dispatching this agent (codex inline-follow)

On codex, the simplest dispatch path is **inline-follow**: the codex agent reads the embedded `## Spawnable Agents > web-researcher` block in this SKILL.md (codex reads SKILL.md directly — see `cafleet/docs/reference/coding-agents/codex.md`) and follows the spec's instructions in its own turn, treating the spec body as additional instructions for the current task. No new agent is spawned; the calling agent absorbs the spec's role for one turn.

Use this when:
- A codex session needs ad-hoc research as part of a larger task.
- You do not need a separate pane or member for the work.

#### Dispatching this agent (codex member-spawn)

When you want a dedicated codex member running the spec (e.g., for parallel multi-topic research), spawn it via `cafleet member create` with `--coding-agent codex`. The spawn prompt is positional (`[PROMPT_ARGV]...`) — there is no `--spawn-prompt-from-text` flag. Paste the embedded spec body verbatim into the positional argument, then append the per-call inputs:

```bash
cafleet member create --fleet-id <fleet-id> \
  --agent-id <director-agent-id> \
  --name web-researcher-codex \
  --description "Web research on <topic>" \
  --coding-agent codex \
  "<paste the web-researcher spec body verbatim>

Research the following topics:
1. <topic 1>
2. <topic 2>

Context: <overall context>"
```

The new member opens its own tmux pane and works autonomously on the spec. Use this when the research benefits from a separate pane (parallel topic batches, longer-running sweeps, isolation from the director's context).

$ARGUMENTS
