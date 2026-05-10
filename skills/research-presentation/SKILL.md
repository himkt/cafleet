---
name: research-presentation
description: Create a Slidev presentation and reading transcript from an existing research report folder. Reads report.md and researcher files for context, creates slides using /my-slidev skill and a reading transcript. Takes folder path as argument (e.g., topic-name). Do NOT use for research — use /research-report for that.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, TaskCreate, TaskUpdate, TaskList, TaskGet
---

# Research Presentation

Create a Slidev presentation and reading transcript from an existing research report folder using a four-role CAFleet-orchestrated team: Director (orchestrator), Presentation (slides), Transcript (narration), and per-batch Visual Reviewer (screenshot-based QA). The team iterates through content revision and visual review before presenting to the user.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Bootstrap CAFleet session, spawn members, review all deliverables, demand revisions, run Slidev server lifecycle and `agent-browser close --all` safety net | Create slides/transcript, conduct research, modify report, run agent-browser browser-operation commands (except close --all) | [roles/director.md](roles/director.md) |
| **Presentation** | claude pane (loads Skill(my-slidev) + Skill(create-figure)) | Create Slidev presentation from report using `/my-slidev` | Invent data, modify report, conduct research | [roles/presentation.md](roles/presentation.md) |
| **Transcript** | claude pane | Create reading transcript with 1:1 slide correspondence | Invent data, modify report, conduct research | [roles/transcript.md](roles/transcript.md) |
| **Visual Reviewer** | claude pane — one per batch | Capture screenshots/snapshots of assigned slides using the agent-browser CLI (`bun run agent-browser`) with a per-batch named session (`--session vr-batch-<start>`), identify visual issues including aesthetic quality, report findings to Director | Edit slide.md, modify report, fix issues directly | [roles/visual-reviewer.md](roles/visual-reviewer.md) |

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` and embeds them into every member's spawn prompt.

## Architecture

The Director is the root agent of a CAFleet session — bootstrapped automatically by `cafleet session create` — and spawns every member via `cafleet --session-id [session-id] member create --agent-id [director-agent-id]`. All inter-agent coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered tmux push notifications).

```text
User
 +-- Director (main Claude — runs cafleet session create, cafleet member create, runs Slidev background server)
      +-- presentation (claude pane — authors slide.md; loads Skill(my-slidev), Skill(create-figure))
      +-- transcript   (claude pane — authors transcript.md)
      +-- vr-batch-<start> (claude pane — captures + reports on one slide batch; per-batch spawn/delete)
```

- **Director ↔ User**: `AskUserQuestion` (approval, revision collection)
- **Director ↔ presentation**: `cafleet message send` (tagged feedback, realignment, revision loop)
- **Director ↔ transcript**: `cafleet message send` (finalized slide structure, tagged feedback)
- **Director ↔ vr-batch-\***: `cafleet message send` (batch assignment, re-check requests, close instruction)

Members cannot talk to the user directly — the Director always relays.

> **Literal-UUID flag rule** — every `cafleet ...` invocation carries the literal `session_id` and `agent_id` UUIDs as flags (not shell variables). `--session-id` is global (before the subcommand); `--agent-id` is a per-subcommand option (after the subcommand name). See `Skill(cafleet)` for the full convention.

> Never store IDs in shell variables — substitute the UUIDs printed by `cafleet session create` and `cafleet member create` directly into every `cafleet ...` call.

## Director Process

The Director's pipeline runs autonomously from Step 0 through Step 3, converges on a single user approval gate at Step 4, then cleans up at Step 5. Read the User Interaction Contract below before entering the steps — it defines the only two points at which the Director is permitted to originate `AskUserQuestion` calls.

### User Interaction Contract

The Director originates `AskUserQuestion` at exactly two kinds of points:

1. **Step 4 — single post-pipeline approval gate.** After all pipeline deliverables exist — slides, transcript, AND visual review — the Director presents them to the user and collects approval or revision requests.
2. **Member-escalated user delegation.** When a member sends a `cafleet message send` that genuinely requires a user decision, the Director relays it via `AskUserQuestion` and passes the answer back verbatim.

The Director does NOT use `AskUserQuestion` to:

- Ask whether to run, skip, or shorten any pipeline step. Steps 0–3 are obligatory and non-negotiable. Visual review in Step 3 is not an optional polish — it is a quality gate.
- Offer "faster," "lighter," or "partial" variants of the pipeline.
- Confirm the Director's own design choices (spawn counts, batch boundaries, tag usage, layout decisions).

If a pipeline step fails for a technical reason the Director cannot resolve (e.g. the Slidev dev server refuses to start after the fallback chain), *then* escalate to the user via `AskUserQuestion` with concrete options — but escalation is a response to failure, not a planning shortcut.

Step 5 (cleanup) is autonomous — no user prompt.

### Step 0: Validate Input (Director)

1. If `$ARGUMENTS` is absent → error: "Usage: `/research-presentation <folder-name>`. Specify the folder containing report.md."
2. Load `Skill(base-dir)` and follow its procedure with `$ARGUMENTS` as the argument.
   - If skipped (absolute path): set `${FOLDER} = $ARGUMENTS`.
   - If base resolved: set `${FOLDER} = ${BASE}/researches/$ARGUMENTS`. Resolve to absolute path.
3. Check that `${FOLDER}/report.md` exists. If not, error: "No report.md found in `${FOLDER}`. Run `/research-report` first to generate a report."
4. Pass `${FOLDER}` as the resolved absolute path to all members in spawn prompts.

### Step 1: Bootstrap CAFleet Session, Start Monitor & Spawn Presentation + Transcript (Director)

Load `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` — this skill spawns parallel members (Presentation + Transcript, and later VR batches), so the `/loop` monitor is mandatory.

#### 1a. Environment precheck and session bootstrap

```bash
cafleet doctor
cafleet --json session create --label "present-[topic-slug]"
```

`cafleet doctor` confirms the Director is inside a tmux session (a hard requirement of `cafleet member create`). On non-zero exit, abort and surface the error to the user — do NOT attempt raw `tmux` probes as a workaround.

`cafleet session create` atomically creates the session, registers a root Director bound to the current tmux pane, and seeds the built-in Administrator. Capture `session_id` and `director.agent_id` from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

#### 1b. Start the `/loop` monitor BEFORE the first `cafleet member create` call

Per `Skill(cafleet:agent-team-monitoring)`, start a 1-minute interval monitor before spawning so the first tick fires while spawning completes. Use the agent-team-monitoring template with literal `[session-id]` and `[director-agent-id]` substituted in. Expected deliverables: `${FOLDER}/slide.md`, `${FOLDER}/transcript.md`. Active members will include `presentation`, `transcript`, and later `vr-batch-*`.

#### 1c. Read role definitions

Read the role files that will be embedded verbatim in spawn prompts (paths are relative to this skill's directory):

- `roles/presentation.md`
- `roles/transcript.md`
- `roles/visual-reviewer.md`

> **Template safety**: cafleet `member create` runs `str.format()` on the entire spawn prompt with `session_id` / `agent_id` / `director_name` / `director_agent_id` as kwargs. The role docs in this skill use `<...>` / `[...]` notation everywhere placeholders appear, so the embedded role content contains no literal `{` or `}` and no escaping is needed. The only single-brace tokens in the spawn prompt are the four kwargs cafleet itself substitutes: `{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}`. If you ever add a `{` or `}` to a role doc (a JSON example, a format-string example), double it to `{{` / `}}` before embedding — `str.format()` raises `KeyError` at spawn time otherwise. Do NOT shell out to `sed` / `awk` — those are blocked by the harness Bash validator.

#### 1d. Spawn Presentation + Transcript in parallel

Both work from `report.md` independently. After the slide deck is finalized (Step 3), the Director sends the final slide structure to the Transcript member for realignment.

**Presentation spawn prompt:**

```
You are the Presentation Specialist in a research presentation team (CAFleet-native).

[ROLE DEFINITION]
[Content of roles/presentation.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives and bash-via-Director routing
- Skill(my-slidev) — for Slidev authoring layouts and rules
- Skill(create-figure) — if the report includes data that would render better as a chart

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

TASK: Create a Slidev presentation from the approved research report.
REPORT:           [INSERT <folder>/report.md]
RESEARCHER FILES: [INSERT <folder>/[0-9][0-9]-research-*.md]
LANGUAGE:         [INSERT language detected from report.md]
FIGURE BASE:      [INSERT <folder>]    (substitute literally for the FIGURE_BASE / BASE placeholders in create-figure)
OUTPUT:           [INSERT <folder>/slide.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `<task-id>` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id <task-id>, then act on the instructions.

When complete, send the file path to the Director via cafleet message send.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "presentation" \
  --description "Authors slide.md" \
  -- "[Presentation spawn prompt]"
```

Capture the printed `agent_id` and substitute it for `[presentation-agent-id]` in subsequent `cafleet message send` calls.

**Transcript spawn prompt:**

```
You are the Transcript Specialist in a research presentation team (CAFleet-native).

[ROLE DEFINITION]
[Content of ~/.claude/skills/research-presentation/roles/transcript.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives and bash-via-Director routing

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

TASK: Create a reading transcript from the approved research report.
REPORT:   [INSERT <folder>/report.md]
LANGUAGE: [INSERT language detected from report.md]
OUTPUT:   [INSERT <folder>/transcript.md]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `<task-id>` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id <task-id>, then act on the instructions.

When complete, send the file path to the Director via cafleet message send.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "transcript" \
  --description "Authors transcript.md" \
  -- "[Transcript spawn prompt]"
```

### Step 2: Content Review & Revision Loop (Director)

Read the output files (`${FOLDER}/slide.md`, `${FOLDER}/transcript.md`) and review using the tag criteria in [roles/director.md](roles/director.md). Send tagged feedback via `cafleet message send`; members revise and reply. See [roles/director.md](roles/director.md) for revision approach and iteration limits.

```bash
cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
  --to [presentation-agent-id] \
  --text "slide revisions: [SLIDE STRUCTURE] ... / [VISUAL] ... / ..."

cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
  --to [transcript-agent-id] \
  --text "transcript revisions: [FLOW] ... / [TIMING] ... / ..."
```

Each polled inbound message MUST be `ack`ed via `cafleet --session-id [session-id] message ack --agent-id [director-agent-id] --task-id <task-id>` after acting on it.

Once the slide deck is finalized, send the finalized slide structure to the Transcript member for 1:1 realignment.

### Step 3: Visual Review & Fix (Director)

Once Step 2 converges on an approved slide deck and transcript, the Director runs the batched visual-review loop defined below. Per the User Interaction Contract, this step is a pipeline stage, not a decision — the Director does not call `AskUserQuestion` to decide whether to run it, skip it, or shorten it.

**Server Startup (once):**

**Working directory: project root** (the directory containing `node_modules/` and `skills/`). Do NOT cd to dependency source directories — they are source repos, not runnable installations.

1. From the project root, run `bun install --frozen-lockfile` to ensure dependencies.
2. Start the Slidev dev server (`run_in_background: true`): `mise run slidev <folder>/slide.md`
3. Set `<server_url>` to the Slidev dev server URL (default: `http://localhost:3030`). Use this value when spawning Visual Reviewers.
4. Create the persistent screenshots directory: write `<folder>/screenshots/.keep` (empty file) using the Write tool. This is a one-time operation per `/research-presentation` invocation; do NOT delete or wipe it on subsequent batches. agent-browser does not auto-create parent directories when given an explicit `screenshot <path>`, so this step is required for VR's per-slide capture to succeed.
5. The Director MUST NOT run `bun run agent-browser --session vr-batch-* open|snapshot|screenshot|wait|close` directly — agent-browser's browser-operation commands are exclusively for Visual Reviewers (the only exception is the `close --all` safety net in Step 5). The Director MAY run `console` and `errors` for diagnostics if needed (e.g., investigating a stuck VR), but should prefer letting the VR do it.

**Batched Review Loop** (batch_size=10, fresh Visual Reviewer per batch to avoid context overflow):

Run the loop serially: spawn one VR member via `cafleet member create`, wait for its report, run the fix-and-recheck sub-loop, then run `cafleet member delete` to close the pane (sends `/exit`, waits up to 15 s). Do not spawn multiple VRs in parallel — fixes from one batch can affect later batches, and parallel agent-browser sessions race on the same Slidev dev server.

> **Per-batch teardown overhead.** `cafleet member delete` blocks ≈15 s per batch waiting for `/exit` to close the pane, plus a tmux layout rebalance. For a 60-slide deck (six batches), the cumulative teardown wall-clock is roughly 90 s plus visible pane churn in the operator's tmux window. This is the documented trade-off for context isolation; `--force` is available as an escape hatch for stuck panes but is NOT the default.

```
total_slides = count slides in slide.md
start = 1

while start <= total_slides:
    end = min(start + 9, total_slides)

    vr_round = 1                               # current VR round number; bumped on each re-check
    spawn VR member via cafleet member create (name="vr-batch-<start>") with slides [start..end], ROUND=vr_round
    # spawn prompt MUST include `RESEARCH FOLDER: <folder>` and `ROUND: 1` lines so the VR
    # can build screenshot/report paths
    # capture the printed agent_id as [vr-batch-agent-id] for subsequent message send / member delete

    while True:                                # initial review (r1) + up to 2 re-checks (r2, r3)
        wait for report from VR for round <vr_round> via cafleet message poll arrival
        if no issues: break
        if vr_round >= 3: break                # max 2 re-check rounds reached; remaining issues escalate to user in Step 4
        cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
            --to [presentation-agent-id] --text "<tagged issues>"   # fix
        vr_round += 1
        cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
            --to [vr-batch-agent-id] --text "ROUND: <vr_round>\nRe-check slides: <list>"
        # VR writes the next capture to `vr<start>-r<vr_round>-p<slide_number>.png` and
        # the next persisted report to `vr<start>-r<vr_round>.md`, preserving prior rounds

    # Explicit close handshake before delete: the VR cannot reliably run extra commands after /exit.
    cafleet --session-id [session-id] message send --agent-id [director-agent-id] \
        --to [vr-batch-agent-id] --text "CLOSE: run `bun run agent-browser --session vr-batch-<start> close`, then reply 'closed'."
    wait for the VR's "closed" confirmation via cafleet message poll
    cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [vr-batch-agent-id]
    start = end + 1
```

**Visual Reviewer spawn prompt** (per batch):

```
You are the Visual Reviewer in a research presentation team (CAFleet-native).

[ROLE DEFINITION]
[Content of ~/.claude/skills/research-presentation/roles/visual-reviewer.md injected verbatim. Cafleet substitutes only the four format kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` — leave those single-braced. Any other literal `{` or `}` characters that appear inside the role doc itself must be doubled to `{{` / `}}` before embedding (per Template safety)]
[/ROLE DEFINITION]

Load these skills at startup:
- Skill(cafleet) — for the broker primitives and bash-via-Director routing

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
DIRECTOR NAME: {director_name}
YOUR AGENT ID: {agent_id}

TASK: Visually verify the rendered Slidev presentation.
SLIDE FILE:      [INSERT <folder>/slide.md]
RESEARCH FOLDER: [INSERT <folder>]
SERVER URL:      [INSERT <server_url>]
SESSION NAME:    [INSERT vr-batch-<start>]
CHECK SLIDES:    [INSERT <start> to <end>]
ROUND:           [INSERT <round>]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `<task-id>` and ack it via cafleet --session-id {session_id} message ack --agent-id {agent_id} --task-id <task-id>, then act on the instructions.

When complete, persist the report to <folder>/screenshots/vr<start>-r<round>.md and send it to the Director via cafleet message send.
```

Spawn with:

```bash
cafleet --session-id [session-id] --json member create --agent-id [director-agent-id] \
  --name "vr-batch-<start>" \
  --description "Visual Reviewer for slides <start>..<end>" \
  -- "[Visual Reviewer spawn prompt]"
```

### Step 4: User Approval & Revision Loop (Director)

This is the single post-pipeline approval gate defined in the User Interaction Contract. Only enter Step 4 after Step 3's visual-review loop has completed (all batches reviewed, fixes applied, re-check rounds exhausted or passing).

Present deliverables (slides, transcript, preview URL) and request approval via `AskUserQuestion`. Report any known residual visual issues surfaced by Step 3 so the user can weigh them.

If the user requests revisions:

1. Triage feedback — slides → `presentation`, transcript → `transcript`.
2. Route feedback via `cafleet message send` using tag-based format; members revise.
3. If slides changed, spawn a fresh Visual Reviewer for affected slides only (same serial pattern as Step 3).
4. Re-present and request approval again.

No round limit — loop until approved.

### Step 5: Finalize & Clean Up (Director)

**Only enter after the user approves in Step 4.**

Follow the Shutdown Protocol in `Skill(cafleet)` § *Shutdown Protocol*. Order matters — every step before `cafleet session delete` must complete first.

1. **Cancel the `/loop` monitor** with `CronDelete <job-id>`. The cron must stop firing BEFORE any member is deleted; a cron that keeps polling a tearing-down session spams `Error: session is deleted`.
2. **Delete every member** — Presentation, Transcript, and any active VR batch. For any active VR batch, run the explicit close handshake first (Director sends `CLOSE:` via `cafleet message send`, VR runs `bun run agent-browser --session vr-batch-<start> close` and replies `closed`), THEN run `cafleet member delete`. Once all VR browser sessions are closed:
   ```bash
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [presentation-agent-id]
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [transcript-agent-id]
   cafleet --session-id [session-id] member delete --agent-id [director-agent-id] --member-id [vr-batch-agent-id]   # if still alive — only after the close handshake
   ```
   Each call sends `/exit` and waits up to 15 s for the pane's `claude` process to exit. Do not rely on `/exit` to trigger any post-shutdown action — additional commands are not guaranteed to run after `/exit` arrives.
3. **Verify the roster is empty**: `cafleet --session-id [session-id] member list --agent-id [director-agent-id]` must return zero members.
4. **Run the agent-browser safety net** to close any orphan browser sessions left behind:
   ```bash
   bun run agent-browser close --all
   ```
5. **Kill the Slidev dev server** if still running (stop the background Bash task started at Step 3).
6. **Delete the session**: `cafleet session delete [session-id]` (positional, no `--session-id` flag). Soft-deletes the session and deregisters the root Director and Administrator atomically.
7. **Confirm**: `cafleet session list` — the current session must not appear (soft-deleted sessions are hidden).

Do NOT use raw `tmux kill-pane` or `tmux send-keys` at any point — `cafleet member delete` is the only supported teardown primitive.

$ARGUMENTS
