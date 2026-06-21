# Research Presentation

Create a Slidev presentation and reading transcript from an existing research report folder using a four-role CAFleet-orchestrated team: Director (orchestrator), Presentation (slides), Transcript (narration), and per-batch Visual Reviewer (screenshot-based QA). The team iterates through content revision and visual review before presenting to the user.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../cafleet/reference/coding-agent/<name>.md`](../../cafleet/reference/coding-agent/) | every `{placeholder}` (`{monitor_model}`, `{decision_surface}`, `{bg_run}`, `{bg_stop}`) stays unresolved — you spawn the monitor with a literal `--model {monitor_model}` or can't background the Slidev server |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root the spawn-prompt audit files or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (monitor-first spawn, the `ready: monitor live` gate, Authorization-Scope Guard, Stall Response) — you spawn an unsupervised team |

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Bootstrap CAFleet fleet, spawn members, review all deliverables, demand revisions, run Slidev server lifecycle and `agent-browser close --all` safety net | Create slides/transcript, conduct research, modify report, run agent-browser browser-operation commands (except close --all) | [roles/director.md](roles/director.md) |
| **Presentation** | claude pane (reads the `../reference/slidev.md` and `../reference/visualization.md` pages) | Create Slidev presentation from report using the `../reference/slidev.md` page | Invent data, modify report, conduct research | [roles/presentation.md](roles/presentation.md) |
| **Transcript** | claude pane | Create reading transcript with 1:1 slide correspondence | Invent data, modify report, conduct research | [roles/transcript.md](roles/transcript.md) |
| **Visual Reviewer** | claude pane — one per batch | Capture screenshots/snapshots of assigned slides using the agent-browser CLI (`bun run agent-browser ...`) with a per-batch named session (`--session vr-batch-<start>`), identify visual issues including aesthetic quality, report findings to Director | Edit slide.md, modify report, fix issues directly | [roles/visual-reviewer.md](roles/visual-reviewer.md) |

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`). The Director loads the `cafleet` skill (reading its `reference/supervision.md`) and embeds it into every member's spawn prompt. The fleet runs a dedicated monitoring member (the first `member create`, `--role monitor --model {monitor_model}`) that owns the heartbeat and re-engages the idle Director — see Step 1.

For autonomous Slidev generation, see `../reference/slidev.md` § Autonomous slide generation.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` — and spawns every member via `cafleet member create --fleet-id [fleet-id] --agent-id [director-agent-id]`. All inter-agent coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered tmux push notifications).

```text
User
 +-- Director (main Claude — runs cafleet fleet create, cafleet member create, runs Slidev background server)
      +-- presentation (claude pane — authors slide.md; reads the slidev.md + visualization.md reference pages)
      +-- transcript   (claude pane — authors transcript.md)
      +-- vr-batch-<start> (claude pane — captures + reports on one slide batch; per-batch spawn/delete)
```

Members cannot talk to the user directly — the Director always relays.

> **Literal-integer-id flag rule** — every `cafleet ...` invocation carries the literal `fleet_id` / `agent_id` integer ids as flags (per-subcommand, after the subcommand name), never shell variables; substitute the integer ids printed by `cafleet fleet create` / `cafleet member create` directly. See the `cafleet` skill for the full convention.

## Director Process

The Director's pipeline runs autonomously from Step 0 through Step 3, converges on a single user approval gate at Step 4, then cleans up at Step 5. Read the User Interaction Contract below before entering the steps — it defines the only two points at which the Director is permitted to originate a decision-surface prompt.

### User Interaction Contract

The Director originates a decision-surface prompt at exactly two kinds of points:

1. **Step 4 — single post-pipeline approval gate.** After all pipeline deliverables exist — slides, transcript, AND visual review — the Director presents them to the user and collects approval or revision requests.
2. **Member-escalated user delegation.** When a member sends a `cafleet message send` that genuinely requires a user decision, the Director relays it via {decision_surface} and passes the answer back verbatim.

The Director does NOT use {decision_surface} to:

- Ask whether to run, skip, or shorten any pipeline step. Steps 0–3 are obligatory and non-negotiable. Visual review in Step 3 is not an optional polish — it is a quality gate.
- Offer "faster," "lighter," or "partial" variants of the pipeline.
- Confirm the Director's own design choices (spawn counts, batch boundaries, tag usage, layout decisions).

If a pipeline step fails for a technical reason the Director cannot resolve (e.g. the Slidev dev server refuses to start after the fallback chain), *then* escalate to the user via {decision_surface} with concrete options — but escalation is a response to failure, not a planning shortcut.

Step 5 (cleanup) is autonomous — no user prompt.

### Step 0: Validate Input (Director)

1. If `$ARGUMENTS` is absent → error: "Usage: run the presentation workflow with `<folder-name>`. Specify the folder containing report.md."
2. Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above).
3. Resolve the task-scoped BASE by calling the resolver positionally with the topic relpath:

   - If `$ARGUMENTS` is a relative folder name (e.g. `my-topic`), **canonicalize first** to the bare topic slug `$CANONICAL_SLUG`: strip a trailing `/report.md` if present, then strip a leading `researches/` prefix if present — so `$CANONICAL_SLUG` carries no `researches/`. Then run the skill's **Step 0 (task-scope resolution)** with the relpath `researches/$CANONICAL_SLUG`; the single `researches/` prepend happens only here, at the call site. Stripping the leading `researches/` during canonicalization is what prevents a doubled `researches/researches/my-topic` when `$ARGUMENTS` already begins with `researches/`.

   - If `$ARGUMENTS` is an absolute path (e.g. `/abs/path/to/researches/my-topic`), **canonicalize first**: strip a trailing `/report.md` if present. Store the canonicalized absolute folder path in `$CANONICAL_ABS` and run Step 0 with that absolute path.

     Step 0 treats the canonicalized absolute path as the task folder verbatim if it is strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel. Step 0 does no filename folding, so a raw child path like `/abs/path/to/researches/my-topic/report.md` is taken verbatim as the task folder — a `report.md`-named folder rather than the topic folder, with directory creation deferred to the first consumer write.

   Branch on Step 0's outcome: when it **resolves**, set both `${FOLDER}` and `${BASE}` to the resolved task folder (the task folder IS the report folder; no further `${BASE}/researches/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root), set `${FOLDER}` to the **canonicalized** absolute path (the same trailing-`/report.md`-stripped path you passed to Step 0) so the report-folder check in step 4 still runs against a folder rather than a file, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.
4. Check that `${FOLDER}/report.md` exists. If not, error: "No report.md found in `${FOLDER}`. Run the report workflow (`../report/report.md`) first to generate a report."
5. Pass `${FOLDER}` as the resolved absolute path to all members in spawn prompts. The audit-file path `${BASE}/prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<topic-folder>/prompts/`, not under the repo root.

### Step 1: Bootstrap CAFleet Fleet & Spawn Presentation + Transcript (Director)

Load the `cafleet` skill; its `reference/supervision.md` policy (heartbeat, facilitation, Stall Response) is § Required reading above. The fleet runs a dedicated monitoring member (the **first** `cafleet member create`, `--role monitor --model {monitor_model}`) that owns the heartbeat and re-engages the idle Director via `cafleet member nudge`; the Director does **not** run `cafleet monitor` itself. Gate the Presentation/Transcript spawns on the monitoring member's `ready: monitor live` handshake (first-in) — see 1b.

#### 1a. Environment precheck and fleet bootstrap

```bash
cafleet doctor
cafleet --json fleet create --label "present-[topic-slug]"
```

`cafleet doctor` confirms the Director is inside a tmux session (a hard requirement of `cafleet member create`). On non-zero exit, abort and surface the error to the user — do NOT attempt raw `tmux` probes as a workaround.

`cafleet fleet create` atomically creates the fleet, registers a root Director bound to the current tmux pane, and seeds the built-in Administrator. Capture `fleet_id` and `director.agent_id` from the JSON response and substitute them as literal strings into every subsequent `cafleet ...` call (never shell variables — the harness matches Bash invocations as literal command strings).

#### 1b. Spawn the monitoring member (first-in)

The **first** `cafleet member create` in the fleet is the dedicated monitoring member, spawned with `--role monitor --model {monitor_model}`. It launches `cafleet monitor start --fleet-id [fleet-id]` as a background task in its own pane, confirms with `cafleet monitor status`, and reports `ready: monitor live` to the Director. **Receipt of that handshake gates the Presentation/Transcript spawns** (1d) — do not spawn an ordinary member until it has arrived (first-in). The Director does **not** run `cafleet monitor start` itself.

Render the canonical monitoring-member spawn prompt (the **conditional** idle-nudge routine — re-engage the Director via `cafleet member nudge` only when un-acked inbox items or stalled members can be named) to a `--prompt-file` per the audit-file pattern in 1c, then spawn:

```bash
cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
  --name "monitor" \
  --description "Monitoring member — runs the heartbeat and re-engages the idle Director" \
  --role monitor --model {monitor_model} \
  --prompt-file ${BASE}/prompts/monitor-<UTC-compact>.md
```

See the `cafleet` skill's `roles/monitor.md` for the canonical spawn prompt and lifecycle. The monitoring member is stopped and deleted first in the Step 5 teardown (first-out). Expected member-produced deliverables: `${FOLDER}/slide.md`, `${FOLDER}/transcript.md`. Active members will include `monitor`, `presentation`, `transcript`, and later `vr-batch-*`.

#### 1c. Read role definitions

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file via `Read` on its first turn — do NOT inline the content; paths below are relative to this skill's directory):

- `roles/presentation.md`
- `roles/transcript.md`
- `roles/visual-reviewer.md`

> **Spawn mechanics**: path-by-reference is required because cafleet `member create` passes the prompt to `tmux split-window` as one positional arg and fails with `command too long` past a few KB (see the `cafleet` skill's `reference/director.md` § *Spawn prompt size limit*). `str.format()` runs over the prompt with `fleet_id` / `agent_id` / `director_agent_id` as kwargs — leave those single-braced, double any other literal `{` / `}`. **Two-step audit file**: write the rendered prompt to `${BASE}/prompts/<role>-<UTC-compact>.md` BEFORE `cafleet member create --prompt-file <abs path>` (the pre-spawn file IS both the CLI input and the permanent audit artifact); see the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol* and its `reference/director.md` § *Member Create — Scratch and audit files* for the contract incl. the `${BASE} == <unset>` guarded-skip + inline fallback.

#### 1d. Spawn Presentation + Transcript in parallel

**Gate**: do not spawn Presentation or Transcript until the monitoring member's `ready: monitor live` handshake (1b) has arrived.

Both work from `report.md` independently. After the slide deck is finalized (Step 3), the Director sends the final slide structure to the Transcript member for realignment.

**Presentation spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Presentation delta below (`{fleet_id}` / `{agent_id}` / `{director_agent_id}` filled by `member create`; `[INSERT …]` markers shell-substituted by the Director first):

| Slot | Presentation Specialist |
|---|---|
| ROLE TITLE / TEAM | `the Presentation Specialist` / `research presentation` |
| role-file | `roles/presentation.md` |
| cafleet-load purpose + EXTRA SKILL LOADS | `for the broker primitives and bash-via-Director routing`; additionally Read the `../reference/slidev.md` page (Slidev authoring layouts + rules) + `../reference/visualization.md` (if the report includes data that would render better as a chart) |
| CONTEXT LINES | `TASK: Create a Slidev presentation from the approved research report.` / `REPORT: [INSERT <folder>/report.md]` / `RESEARCHER FILES: [INSERT <folder>/[0-9][0-9]-research-*.md]` / `LANGUAGE: [INSERT language detected from report.md]` / `FIGURE BASE: [INSERT <folder>]` (substitute literally for the FIGURE_BASE / BASE placeholders in `../reference/visualization.md`) / `OUTPUT: [INSERT <folder>/slide.md]` |
| POLL-HANDLING | **ack-inline** form (capture the `id:` integer as `<task-id>` and `cafleet message ack … --task-id <task-id>`, then act) |
| start cue (verbatim) | `When complete, send the file path to the Director via cafleet message send.` |

Render the prompt to `${BASE}/prompts/presentation-<UTC-compact>.md` per the 1c two-step audit-file pattern (leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "presentation" \
     --description "Authors slide.md" \
     --prompt-file ${BASE}/prompts/presentation-<UTC-compact>.md
   ```

   Capture the printed `agent_id` and substitute it for `[presentation-agent-id]` in subsequent `cafleet message send` calls.

**Transcript spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Transcript delta:

| Slot | Transcript Specialist |
|---|---|
| ROLE TITLE / TEAM | `the Transcript Specialist` / `research presentation` |
| role-file | `roles/transcript.md` |
| cafleet-load purpose | `for the broker primitives and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `TASK: Create a reading transcript from the approved research report.` / `REPORT: [INSERT <folder>/report.md]` / `LANGUAGE: [INSERT language detected from report.md]` / `OUTPUT: [INSERT <folder>/transcript.md]` |
| POLL-HANDLING | **ack-inline** form |
| start cue (verbatim) | `When complete, send the file path to the Director via cafleet message send.` |

Render the prompt to `${BASE}/prompts/transcript-<UTC-compact>.md` per the 1c two-step audit-file pattern, then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "transcript" \
     --description "Authors transcript.md" \
     --prompt-file ${BASE}/prompts/transcript-<UTC-compact>.md
   ```

   Capture the printed `agent_id` and substitute it for `[transcript-agent-id]` in subsequent `cafleet message send` calls.

### Step 2: Content Review & Revision Loop (Director)

Read the output files (`${FOLDER}/slide.md`, `${FOLDER}/transcript.md`) and review using the tag criteria in [roles/director.md](roles/director.md). Send tagged feedback via `cafleet message send`; members revise and reply. See [roles/director.md](roles/director.md) for revision approach and iteration limits.

```bash
cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
  --to [presentation-agent-id] \
  --text "slide revisions: [SLIDE STRUCTURE] ... / [VISUAL] ... / ..."

cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
  --to [transcript-agent-id] \
  --text "transcript revisions: [FLOW] ... / [TIMING] ... / ..."
```

Each polled inbound message MUST be `ack`ed via `cafleet message ack --fleet-id [fleet-id] --agent-id [director-agent-id] --task-id <task-id>` after acting on it.

Once the slide deck is finalized, send the finalized slide structure to the Transcript member for 1:1 realignment.

### Step 3: Visual Review & Fix (Director)

Once Step 2 converges on an approved slide deck and transcript, the Director runs the batched visual-review loop defined below. Per the User Interaction Contract, this step is a pipeline stage, not a decision — the Director does not call {decision_surface} to decide whether to run it, skip it, or shorten it.

**Server Startup (once):**

**Calling-pane working directory: a directory that contains the Slidev `package.json` (typically the host project root).** Bun resolves `node_modules/` and `package.json` from the calling directory directly — no `--cwd` plumbing or sidecar directory. Project-specific task wrappers (e.g., `mise` tasks) that capture invariants like `--frozen-lockfile` belong in the host project's `.claude/rules/`, not in this skill body.

1. Install bun dependencies — refer to your host project's `.claude/rules/` for the canonical command (it typically wraps `bun install --frozen-lockfile`).
2. Start the Slidev dev server **as a backgrounded process** — refer to your host project's `.claude/rules/` for the canonical launcher. The underlying invocation is `bun run slidev --open false <folder>/slide.md` (the `--open false` flag is required for headless review) and the launcher MUST PTY-wrap stdout (e.g., via `script -qfc`) so Slidev does not exit on detecting a non-TTY. **Record the background process handle** your coding agent returns when it backgrounds the process — the overall Step 5 of this skill (*Finalize & Clean Up*, further down the page) needs it to stop the server cleanly without falling back on `pkill`. Run the backgrounded process via {bg_run}; stop it at teardown via {bg_stop}.
3. Set `<server_url>` to the Slidev dev server URL (default: `http://localhost:3030`). Use this value when spawning Visual Reviewers.
4. Create the persistent screenshots directory: write `<folder>/screenshots/.keep` (empty file) using the Write tool. This is a one-time operation per presentation-workflow run; do NOT delete or wipe it on subsequent batches. agent-browser does not auto-create parent directories when given an explicit `screenshot <path>`, so this step is required for VR's per-slide capture to succeed.
5. The Director MUST NOT run `bun run agent-browser --session vr-batch-* open|snapshot|screenshot|wait|close` (or the equivalent `bun run agent-browser ...`) directly — agent-browser's browser-operation commands are exclusively for Visual Reviewers (the only exception is the `close --all` safety net in Step 5 *Finalize & Clean Up*). The Director MAY run `console` and `errors` for diagnostics if needed (e.g., investigating a stuck VR), but should prefer letting the VR do it.

**Batched Review Loop** (batch_size=10, fresh Visual Reviewer per batch to avoid context overflow):

Run the loop serially: spawn one VR member via `cafleet member create`, wait for its report, run the fix-and-recheck sub-loop, then run `cafleet member delete` to close the pane (sends `/exit`, waits up to 15 s). Do not spawn multiple VRs in parallel — fixes from one batch can affect later batches, and parallel agent-browser sessions race on the same Slidev dev server.

> **Per-batch teardown**: `cafleet member delete` blocks ≈15 s per batch (`/exit` + tmux layout rebalance) — the documented trade-off for context isolation. `--force` is an escape hatch for stuck panes, not the default.

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
        cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
            --to [presentation-agent-id] --text "<tagged issues>"   # fix
        vr_round += 1
        cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
            --to [vr-batch-agent-id] --text "ROUND: <vr_round>\nRe-check slides: <list>"
        # VR writes the next capture to `vr<start>-r<vr_round>-p<slide_number>.png` and
        # the next persisted report to `vr<start>-r<vr_round>.md`, preserving prior rounds

    # Explicit close handshake before delete: the VR cannot reliably run extra commands after /exit.
    cafleet message send --fleet-id [fleet-id] --agent-id [director-agent-id] \
        --to [vr-batch-agent-id] --text "CLOSE: run `bun run agent-browser --session vr-batch-<start> close`, then reply 'closed'."
    wait for the VR's "closed" confirmation via cafleet message poll
    cafleet member delete --fleet-id [fleet-id] --member-id [vr-batch-agent-id]
    start = end + 1
```

**Visual Reviewer spawn prompt** (per batch):

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Visual Reviewer delta:

| Slot | Visual Reviewer |
|---|---|
| ROLE TITLE / TEAM | `the Visual Reviewer` / `research presentation` |
| role-file | `roles/visual-reviewer.md` |
| cafleet-load purpose | `for the broker primitives and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `TASK: Visually verify the rendered Slidev presentation.` / `SLIDE FILE: [INSERT <folder>/slide.md]` / `RESEARCH FOLDER: [INSERT <folder>]` / `SERVER URL: [INSERT <server_url>]` / `SESSION NAME: [INSERT vr-batch-<start>]` / `CHECK SLIDES: [INSERT <start> to <end>]` / `ROUND: [INSERT <round>]` |
| POLL-HANDLING | **ack-inline** form |
| start cue (verbatim) | `When complete, persist the report to <folder>/screenshots/vr<start>-r<round>.md and send it to the Director via cafleet message send.` |

Render the prompt to `${BASE}/prompts/vr-batch-<start>-<UTC-compact>.md` per the 1c two-step audit-file pattern (`<start>` matches the batch's first-slide index used in `--name`; each VR batch gets its own timestamped file — no overwriting), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id [fleet-id] --agent-id [director-agent-id] \
     --name "vr-batch-<start>" \
     --description "Visual Reviewer for slides <start>..<end>" \
     --prompt-file ${BASE}/prompts/vr-batch-<start>-<UTC-compact>.md
   ```

   Capture the printed `agent_id` as `[vr-batch-agent-id]` for subsequent `cafleet message send` / `member delete` calls.

### Step 4: User Approval & Revision Loop (Director)

This is the single post-pipeline approval gate defined in the User Interaction Contract. Only enter Step 4 after Step 3's visual-review loop has completed (all batches reviewed, fixes applied, re-check rounds exhausted or passing).

Present deliverables (slides, transcript, preview URL) and request approval via {decision_surface}. Report any known residual visual issues surfaced by Step 3 so the user can weigh them.

If the user requests revisions:

1. Triage feedback — slides → `presentation`, transcript → `transcript`.
2. Route feedback via `cafleet message send` using tag-based format; members revise.
3. If slides changed, spawn a fresh Visual Reviewer for affected slides only (same serial pattern as Step 3).
4. Re-present and request approval again.

No round limit — loop until approved.

### Step 5: Finalize & Clean Up (Director)

**Only enter after the user approves in Step 4.**

Follow the Shutdown Protocol in the `cafleet` skill § *Shutdown Protocol* (first-out): stop the monitoring member's `monitor start` background task and wait for confirmation, then `cafleet member delete` the monitoring member first, then Presentation, Transcript, and any active VR batch — but **for an active VR batch, run the close handshake first**: the Director sends `CLOSE:` via `cafleet message send`, the VR runs `bun run agent-browser --session vr-batch-<start> close` and replies `closed`, THEN delete it. Verify the roster is empty with `cafleet member list`.

Then the presentation-specific teardown:

1. **agent-browser safety net** — close any orphan browser sessions:
   ```bash
   bun run agent-browser close --all
   ```
2. **Stop the Slidev dev server** via {bg_stop} (using the handle recorded in Step 3, Server Startup substep 2) — never the broad `pkill -f slidev`, which matches too widely and leaks stdout until the process is stopped through its recorded handle.
3. `cafleet fleet delete [fleet-id]` (positional); `cafleet fleet list` to confirm. Never use raw `tmux kill-pane` / `tmux send-keys`.

$ARGUMENTS
