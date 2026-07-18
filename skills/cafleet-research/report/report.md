# Research Report

Generate comprehensive research reports using a multi-layer CAFleet-orchestrated team: Director → Manager → Scouts/Researchers. Every member carries serious accountability for the quality of the final deliverable, and the team iterates relentlessly until the report meets the highest standard. After the report is approved, the Director offers to chain into the presentation workflow (`../presentation/presentation.md`) for slides and transcript.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../cafleet/reference/coding-agent/<name>-overlay.md`](../../cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — you emit a literal `{monitor_model}` / `{task_coord}` / `{decision_surface}` (spawn the monitor with `--model {monitor_model}`), **or** guess a wrong/default value, **or** ignore a backend note (codex has no harness task list) |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root the spawn-prompt audit files and `${OUTPUT_DIR}` or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (monitor-first spawn, the `ready: monitor live` gate, Authorization-Scope Guard, Stall Response) — you spawn an unsupervised team |

Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main agent | Bootstrap CAFleet fleet, spawn all members, relay Manager requests, review all deliverables, present to user | Write the report, decompose topics, conduct research | [roles/director.md](roles/director.md) |
| **Manager** | member pane (member) | Run orientation searches for landscape understanding and topic decomposition, request Scout/Researcher spawning from the Director, aggregate Scout and Researcher findings, compile report, revise | Conduct deep investigation — all substantive research MUST be delegated to Researchers | [roles/manager.md](roles/manager.md) |
| **Scout** | member pane (member) | Landscape mapping — broad discovery to expand knowledge before decomposition | Collect facts for the report, write report sections | [roles/scout.md](roles/scout.md) |
| **Researcher** | member pane (member) | Search exhaustively, collect facts with sources, filter misinformation, write findings to assigned file | Synthesize or write report sections | [roles/researcher.md](roles/researcher.md) |

## Additional resources

- For the report format specification, see [template.md](template.md)

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads the `cafleet` skill (reading its `reference/supervision.md`) and embeds it into every member's spawn prompt. The fleet runs a dedicated monitoring member (the first `member create`, `--role monitor --model {monitor_model}`) that owns the heartbeat and re-engages the Director on demand — see Step 1.

## Output

The skill writes its working folder to `<CWD>/researches/<topic-slug>/` (one folder per research run, containing `report.md` and per-researcher files). Callers MUST add `researches/` to their per-project `.gitignore` before invoking this skill — the skill does not create or modify `.gitignore` itself, and the working folder is meant to stay out of version control.

## Architecture

The Director is the root member of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` — and spawns every member via `cafleet member create --fleet-id [fleet-id]` (the Director is auto-resolved from the fleet row). All inter-member coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered push notifications) and {task_coord}.

```text
User
 +-- Director (main agent — runs cafleet fleet create, cafleet member create, drives the loop)
      +-- manager (member pane — compiles report, decomposes topic)
      +-- scout-<NN> (member pane — landscape mapping)
      +-- researcher-NN (member pane — deep investigation)
```

Members cannot talk to the user directly — the Director always relays. Members cannot talk to each other directly either — Manager requests are always mediated by the Director (Manager → Director → Scout/Researcher, and Scout/Researcher → Director → Manager).

> **Literal-integer-id flag rule** — substitute the integer ids printed by `cafleet fleet create` and `cafleet member create` directly into every `cafleet ...` call (the harness matches Bash invocations as literal command strings). Never store IDs in shell variables. `--fleet-id` and the member-identity options (`--member-id`, `--from-member-id`, `--to-member-id`) are per-subcommand options (placed AFTER the subcommand name). See the `cafleet` skill for the full convention.

## Process

### Step 0: Base Directory Selection (Director)

Before creating the team, resolve the task-scoped base directory for this run.

1. Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above).
2. Resolve the task-scoped BASE by calling the resolver positionally with the topic relpath:

   Run the skill's **Step 0 (task-scope resolution)** with the relpath `researches/[topic-slug]`.

   Step 0 infers the repo root via `git rev-parse --show-toplevel`, joins `researches/[topic-slug]` under it, and resolves `${BASE}` to that absolute task folder. The resolver writes nothing at resolution time; `${BASE}` is created by the first write under it — the Director's spawn-prompt audit write to `${BASE}/.prompts/<role>-<UTC-compact>.md`, which lands before any member spawns, so the output directory already exists by the time Researchers write to it (matching `roles/researcher.md` § File Output). Use that `${BASE}` for the rest of this run.
3. `${OUTPUT_DIR} = ${BASE}` — the task folder IS the output directory. There is no further concatenation.
4. Pass `${OUTPUT_DIR}` (i.e., `${BASE}`) as the resolved absolute path to the Manager and all Researchers/Scouts in their spawn prompts. The audit-file path `${BASE}/.prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<topic-folder>/.prompts/`, not under the repo root.

### Step 0a: Environment Precheck (Director — MANDATORY)

Run `cafleet doctor` to confirm the Director is inside a tmux or herdr session with valid pane identifiers (a hard requirement of `cafleet member create`). On non-zero exit, abort and surface the error to the user — do NOT attempt raw `tmux` probes as a workaround.

### Step 0b: Bootstrap CAFleet Fleet (Director — MANDATORY)

`cafleet fleet create` atomically creates the fleet and registers a root Director bound to the current tmux pane. Capture the `fleet_id` and `director.member_id` integer ids from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

```bash
cafleet fleet create --name "research-[topic-slug]" --coding-agent <backend> --json
```

`--coding-agent <backend>` — substitute the coding agent you are actually running on: your spawn prompt's `CODING AGENT:` line names it; a standalone Director uses its own identity (e.g. Claude Code → `claude`).

Capture `fleet_id` and `director.member_id` from the response. Treat `fleet_id` as `[fleet-id]` and `director.member_id` as `[director-member-id]` for the rest of this skill.

### Step 1: Supervision Model (Director — spawn the monitoring member first)

Load the `cafleet` skill; its `reference/supervision.md` policy (heartbeat, facilitation, Stall Response) is § Required reading above. The **first** `cafleet member create` in the fleet is the dedicated monitoring member, spawned with `--role monitor --model {monitor_model}`. It launches `cafleet monitor start --fleet-id [fleet-id]` as a background task in its own pane, confirms with `cafleet monitor status`, and reports `ready: monitor live` to the Director. **Receipt of that handshake gates the Manager / Scout / Researcher spawns** — do not spawn an ordinary member until `ready: monitor live` has arrived (first-in). The Director does **not** run `cafleet monitor start` itself.

See the `cafleet` skill's `roles/monitor.md` for the canonical monitoring-member spawn prompt (including the conditional idle-nudge routine) and lifecycle. The monitoring member is deleted first in the Step 8 teardown (first-out).

On each active turn, check `${OUTPUT_DIR}` for these expected deliverables:

- `report.md` — required final compiled report from the Manager
- `00-scout-*.md` — Scout landscape/discovery notes (one or more files may exist)
- `NN-research-*.md` — Researcher findings files for delegated sub-topics (`NN` is the assigned number; one or more files may exist)

Readiness/stall rules (apply per the `cafleet` skill's `reference/supervision.md`):

- After Scouts/Researchers have been spawned and tasks have been assigned, expect at least one `00-scout-*.md` or `NN-research-*.md` file to appear after the first round of replies.
- Do not consider the workflow ready for Step 5 until `report.md` exists.
- If a member has an assignment still in progress but their deliverable file is missing past the expected milestone, run the 2-stage health-check from the `cafleet` skill's `reference/supervision.md`: `cafleet message poll` → `cafleet member capture --lines 200` → directed `cafleet message send` nudge → user escalation.

### Step 2: Spawn Manager (Director)

Load the `cafleet` skill and follow its spawn protocol.

#### 2a. Shared task coordination

Work-coordination substrate: {task_coord}. On a harness task list, the on-disk store is created on the first task creation (typically by the Manager when decomposing the topic) and each sub-topic is one task (`owner: "researcher-NN"`, `status: "in_progress"` → `completed`); on a message-only backend, registrations, claims, and completions ride as cafleet messages with no store to create. Either way, no explicit team-bootstrap step is required.

#### 2b. Locate role definitions (path-by-reference)

The Director references each role definition by its **absolute path** in the spawn prompt — the spawned member opens its role doc with `Read` at startup. Do NOT inline the role content into the prompt. Resolve the absolute path for each role file once (the role files live in this skill's `roles/` directory):

- `<abs path to this skill>/roles/manager.md`
- `<abs path to this skill>/roles/scout.md`
- `<abs path to this skill>/roles/researcher.md`

Substitute these absolute paths into the spawn prompts below.

> **Spawn mechanics**: path-by-reference is required because `cafleet member create` passes the prompt to `tmux split-window` as one positional arg and fails with `command too long` past a few KB (see the `cafleet` skill's `reference/director.md` § *Spawn prompt size limit*). The CLI runs `str.format` over the prompt, rendering the four `{fleet_id}` / `{director_member_id}` / `{member_id}` / `{coding_agent}` identity placeholders to literals at spawn — double any literal brace as `{{` / `}}` and leave no other stray single braces. **Two-step audit file**: write the rendered prompt to `${BASE}/.prompts/<role>-<UTC-compact>.md` before `cafleet member create --text-file <abs path>` — the pre-spawn file is both the CLI input and the permanent audit artifact. The placeholders-pre-substitution note and the `${BASE} == <unset>` guarded-skip + inline fallback are canonical in the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol* and its `reference/director.md` § *Member Create — Scratch and audit files*.

#### 2c. Spawn the Manager

**Manager spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Manager delta below (the skeleton's identity lines carry the CLI's four `{...}` placeholders, rendered to literals by `cafleet member create` at spawn; `[INSERT …]` markers rendered by the Director first):

| Slot | Manager |
|---|---|
| ROLE TITLE / TEAM | `the Manager` / `research report` |
| role-file + ROLE-DEF suffix | `roles/manager.md`; suffix `— accountability, communication protocol, task discipline, file-aggregation rules, pre-compilation verification, revision loop, and shutdown.` |
| cafleet-load purpose | `for the broker primitives, literal-integer-id flag convention, and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `CURRENT DATE: [INSERT today's date]` / `USER REQUEST: [INSERT user's original request in full]` / `OUTPUT DIRECTORY: [INSERT OUTPUT DIRECTORY]` / `LANGUAGE: [INSERT user's language preference if specified]` |
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form (capture the `id:` integer as `[message-id]` and `cafleet message ack … --message-id [message-id]`, then act); plus `You do NOT talk to Scouts or Researchers directly. The Director spawns them and relays their findings.` and `The team coordinates sub-topic assignments via {task_coord}.` |
| start cue (verbatim) | `To request Scouts or Researchers, send the Director a cafleet message specifying: role (Scout or Researcher), scope, search angles, and output file path. The Director will spawn them via cafleet member create and relay their completion reports back to you.` + `Your first compiled report will be reviewed critically by the Director. Aim for highest quality on the first attempt.` |

Render the prompt to `${BASE}/.prompts/manager-<UTC-compact>.md` per the 2b two-step audit-file pattern (the four identity placeholders are rendered by the CLI at spawn), then spawn with `--text-file`:

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "manager" \
     --description "Compiles the research report" \
     --text-file ${BASE}/.prompts/manager-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` and substitute it for `[manager-member-id]` in every subsequent `cafleet` call that targets the Manager.

### Step 3: Knowledge Bootstrapping — Scout Phase (Director, on Manager's request)

After assessing the topic, the Manager may send the Director one or more Scout spawn requests via `cafleet message send`. For each request, the Director spawns a Scout.

**Scout spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Scout delta:

| Slot | Scout |
|---|---|
| ROLE TITLE / TEAM | `a Scout Researcher` / `research` |
| role-file + ROLE-DEF suffix | `roles/scout.md`; suffix `— landscape-mapping focus, communication protocol, output format, and shutdown.` |
| cafleet-load purpose | `for the broker primitives and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `CURRENT DATE: [INSERT today's date]` / `YOUR ASSIGNMENT: [landscape scope and what areas to map]` / `OUTPUT FILE: [INSERT <resolved-path>/00-scout-<topic>.md]` |
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form (capture the `id:` integer as `[message-id]` and `cafleet message ack … --message-id [message-id]`, then act) |
| start cue (verbatim) | `Write findings to the output file, then send the Director a completion summary. The Director will relay your findings to the Manager.` |

Render the prompt to `${BASE}/.prompts/<scout-name>-<UTC-compact>.md` per the 2b two-step audit-file pattern (use `scout` for a single Scout, `scout-1`/`scout-2`/… for multiple; `<scout-name>` is the lowercased `--name`), then spawn with `--text-file`:

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "scout-<NN>" \
     --description "Landscape scout" \
     --text-file ${BASE}/.prompts/scout-<NN>-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` for each Scout and substitute it into subsequent `cafleet message send` calls targeting that Scout.

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

With multiple Researchers running in parallel, coordination goes through {task_coord} — not just through spawn prompts. The Manager MUST create one task per sub-topic BEFORE asking the Director to spawn the Researcher for it.

- The Manager registers each sub-topic with {task_coord} before its Researcher is spawned. The registration records the sub-topic, scope, and expected output file path (e.g., `<resolved-path>/01-research-<subtopic>.md`).
- Each Researcher claims its assignment via {task_coord} on spawn.
- Researchers report their assignment complete via {task_coord} when their output file is written and the completion report has been sent.
- The Manager blocks until every assignment is reported complete before starting compilation. Check {task_coord} for progress.

The Manager's task-creation calls also serve as the authoritative list of sub-topic scopes — if the Director sees a discrepancy between a spawn request's scope and the corresponding task, treat the task description as canonical and ask the Manager to reconcile.

#### 4b. Spawn each Researcher (Director)

**Researcher spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Researcher delta:

| Slot | Researcher |
|---|---|
| ROLE TITLE / TEAM | `a Research Specialist` / `research` |
| role-file + ROLE-DEF suffix | `roles/researcher.md`; suffix `— accountability, Discovery Phase, fact verification protocol, output format, and shutdown.` |
| cafleet-load purpose | `for the broker primitives and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `CURRENT DATE: [INSERT today's date]` / `YOUR NAME: researcher-NN` / `YOUR ASSIGNMENT: [specific sub-topic and what to investigate]` / `YOUR TASK ID: [INSERT the task id the Manager created for this sub-topic]` / `OUTPUT FILE: [INSERT <resolved-path>/NN-research-<subtopic>.md]` |
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form; plus `On start, claim your assignment via {task_coord}.` and `Report completion via {task_coord} when done.` |
| start cue (verbatim) | `Write findings to the output file, then send the Director a completion summary. The Director will relay findings and any follow-up questions between you and the Manager.` |

Render the prompt to `${BASE}/.prompts/researcher-<NN>-<UTC-compact>.md` per the 2b two-step audit-file pattern, then spawn with `--text-file`:

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "researcher-NN" \
     --description "Researcher for sub-topic <slug>" \
     --text-file ${BASE}/.prompts/researcher-NN-<UTC-compact>.md \
     --json
   ```

   The Director repeats this step whenever the Manager requests additional Researchers (coverage gaps, failed investigations, revision-driven re-research). Any new Researcher must first have a task created by the Manager; the Director includes the `taskId` in the spawn prompt.

### Step 5: Review & Revision Loop (Director ↔ Manager, via `cafleet message send`)

When the Manager delivers the compiled `report.md`:

1. The Director reads `${OUTPUT_DIR}/report.md` and reviews it critically against the checklist in [roles/director.md](roles/director.md).
2. The Director sends tagged feedback to the Manager:
   ```bash
   cafleet message send --fleet-id [fleet-id] --from-member-id [director-member-id] \
     --to-member-id [manager-member-id] \
     --text "review feedback round <N>: [FACTUAL ERROR] ... / [GAP] ... / ..."
   ```
3. The Manager revises the report (requesting additional Researchers from the Director as needed) and sends a completion message back via `cafleet message send`.
4. Each polled inbound message MUST be `ack`ed via `cafleet message ack --fleet-id [fleet-id] --member-id [director-member-id] --message-id [message-id]` after acting on it. Un-acked messages stay in `INPUT_REQUIRED` and re-surface on every subsequent `message poll` cycle.
5. Repeat until the Director judges quality is sufficient. Aim for 2–3 rounds maximum.

If the Manager asks the Director a question that is really a user decision (e.g. language choice, scope trade-off), the Director MUST relay via {decision_surface} and pass the user's verbatim answer back via `cafleet message send`. Never decide on the user's behalf.

### Step 6: Present to User (Director)

Present the approved report to the user via {decision_surface} (options: **Approve** / **Request changes**; a free-text fallback captures other feedback) with a summary of findings (2–3 sentences), file paths (report, scout files, researcher files), and known limitations. If the user selects **Request changes** or provides free-form feedback, route it to the Manager via `cafleet message send`, re-review, and re-present. Repeat until the user approves.

### Step 7: Offer Presentation Chaining (Director)

After user approval, offer to create a presentation via {decision_surface} (adapt to user's language). If yes, proceed to Step 8, then run the presentation workflow (`../presentation/presentation.md`) with `${OUTPUT_DIR}`. If no, proceed directly to Step 8.

### Step 8: Finalize & Clean Up (Director)

Follow the Shutdown Protocol in the `cafleet` skill § *Shutdown Protocol* (first-out): `cafleet member delete` the monitoring member first (the pane kill terminates its `monitor start` loop), then Researchers, any active Scout, and the Manager (each kills the pane immediately); `cafleet member list` to verify the roster is empty; `cafleet fleet delete --fleet-id [fleet-id]`; `cafleet fleet list` to confirm. Never use raw `tmux kill-pane` / `tmux send-keys`.

## Spawnable Agents

### web-researcher

This skill ships an embedded agent spec for parallel web research that returns structured summaries with sources. The canonical spec lives in [`roles/web-researcher.md`](roles/web-researcher.md). Read that file and paste its spec body (the prose under the frontmatter) verbatim into the dispatch recipe below.

#### Dispatching this agent

Read the spec body (the prose under the frontmatter) from [`roles/web-researcher.md`](roles/web-researcher.md) and use it verbatim as the dispatched agent's prompt, then append the per-call inputs — the research topic(s) and the context for why the information is needed:

```
<paste the web-researcher spec body verbatim>

Research: <topic>
Context: <why this information is needed>
```

Dispatch this prompt via your backend's sub-agent primitive if it has one (see your overlay). If your backend has no sub-agent primitive, either **inline-follow** the spec (read [`roles/web-researcher.md`](roles/web-researcher.md) and follow it in your own turn — no new agent spawned) or **member-spawn** a dedicated member via `cafleet member create` with the spec body pasted into its spawn prompt.
