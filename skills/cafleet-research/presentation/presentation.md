# Research Presentation

Create a Slidev presentation and reading transcript from an existing research report folder using a four-role CAFleet-orchestrated team: Director (orchestrator), Presentation (slides), Transcript (narration), and per-batch Visual Reviewer (screenshot-based QA). The team iterates through content revision and visual review before presenting to the user.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`../../cafleet/reference/coding-agent-overlays.md#<name>`](../../cafleet/reference/coding-agent-overlays.md) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — the failure modes *Resolve your overlay* closes, e.g. a literal `{bg_run}` emitted unresolved |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root the spawn-prompt audit files or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (the monitor-first spawn, the `monitor live` gate, Authorization-Scope Guard, Stall Response) — you spawn an unsupervised team |

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main agent | Bootstrap CAFleet fleet, spawn members, review all deliverables, demand revisions, run Slidev server lifecycle and `agent-browser close --all` safety net | Create slides/transcript, conduct research, modify report, run agent-browser browser-operation commands (except close --all) | [roles/director.md](roles/director.md) |
| **Presentation** | member pane (reads the `../reference/slidev.md` and `../reference/visualization.md` pages) | Create Slidev presentation from report using the `../reference/slidev.md` page | Invent data, modify report, conduct research | [roles/presentation.md](roles/presentation.md) |
| **Transcript** | member pane | Create reading transcript with 1:1 slide correspondence | Invent data, modify report, conduct research | [roles/transcript.md](roles/transcript.md) |
| **Visual Reviewer** | member pane — one per batch | Capture screenshots/snapshots of assigned slides using the agent-browser CLI (`pnpm exec agent-browser ...`) with a per-batch named session (`--session vr-batch-<start>`), identify visual issues including aesthetic quality, report findings to Director | Edit slide.md, modify report, fix issues directly | [roles/visual-reviewer.md](roles/visual-reviewer.md) |

## Prerequisites

The cafleet binary must be installed and on `PATH` (verify with `cafleet doctor`); everything else is gated in Steps 0–1.

For autonomous Slidev generation, see `../reference/slidev.md` § Autonomous slide generation.

## Architecture

The Director is the root member of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` — and spawns every member via `cafleet member create --fleet-id [fleet-id]` (the Director is auto-resolved from the fleet row). All inter-member coordination flows through the CAFleet message broker (`cafleet message send` + auto-delivered push notifications).

```text
User
 +-- Director (main agent — runs cafleet fleet create, cafleet member create, runs Slidev background server)
      +-- presentation (member pane — authors slide.md; reads the slidev.md + visualization.md reference pages)
      +-- transcript   (member pane — authors transcript.md)
      +-- vr-batch-<start> (member pane — captures + reports on one slide batch; per-batch spawn/delete)
```

Members cannot talk to the user directly — the Director always relays.

> **Literal-integer-id rule** — every `cafleet ...` call carries the literal integer ids as positional subjects or id flags; the convention is canonical in the `cafleet` skill § *Required ids*.

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

   - If `$ARGUMENTS` is a relative folder name (e.g. `my-topic`), **canonicalize first** to `researches/<topic-slug>` per the `cafleet` skill's `reference/base-dir.md` § *Consumer contract*, then run the skill's **Step 0 (task-scope resolution)** with that relpath. Store the bare slug in `$CANONICAL_SLUG`.

   - If `$ARGUMENTS` is an absolute path (e.g. `/abs/path/to/researches/my-topic`), canonicalize per the same § *Consumer contract* row, which leaves the absolute folder path verbatim. Store it in `$CANONICAL_ABS` and run Step 0 with that absolute path.

     Step 0 treats the canonicalized absolute path as the task folder verbatim if it is strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

   Branch on Step 0's outcome: when it **resolves**, set both `${FOLDER}` and `${BASE}` to the resolved task folder (the task folder IS the report folder; no further `${BASE}/researches/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root), set `${FOLDER}` to the **canonicalized** absolute path (the same trailing-`/report.md`-stripped path you passed to Step 0) so the report-folder check in step 4 still runs against a folder rather than a file, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.
4. Check that `${FOLDER}/report.md` exists. If not, error: "No report.md found in `${FOLDER}`. Run the report workflow (`../report/report.md`) first to generate a report."
5. Pass `${FOLDER}` as the resolved absolute path to all members in spawn prompts. The audit-file path `${BASE}/.prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<topic-folder>/.prompts/`, not under the repo root.

### Step 1: Bootstrap CAFleet Fleet & Spawn Presentation + Transcript (Director)

Load the `cafleet` skill; its `reference/supervision.md` policy (heartbeat, facilitation, Stall Response) is § Required reading above. Gate the Presentation/Transcript spawns on the monitor member's `monitor live` signal — see 1b.

#### 1a. Environment precheck and fleet bootstrap (monitor included)

Bootstrap the fleet per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Fleet bootstrap (monitor included)* — the gating `cafleet doctor` env-check first, then the fleet create with the monitor member's spawn prompt written first and passed via `--monitor-file`:

```bash
cafleet doctor
cafleet fleet create --name "present-[topic-slug]" --coding-agent <backend> --monitor-file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md> --monitor-model {monitor_model} --json
```

Capture `fleet_id` and `director.member_id` from the JSON response and substitute them into every subsequent `cafleet ...` call.

#### 1b. Wait for the monitor gate (before any ordinary member)

Wait for the monitor member's `ready` then `monitor live` signals per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Wait for the monitor gate* — `monitor live` gates the Presentation/Transcript spawns (1d). The monitor member is deleted first (first-out) in the Step 5 teardown. Expected member-produced deliverables: `${FOLDER}/slide.md`, `${FOLDER}/transcript.md`. Active members will include `monitor`, `presentation`, `transcript`, and later `vr-batch-*`.

#### 1c. Read role definitions

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file via `Read` on its first turn — do NOT inline the content; paths below are relative to this skill's directory):

- `roles/presentation.md`
- `roles/transcript.md`
- `roles/visual-reviewer.md`

> **Spawn mechanics**: render each spawn prompt to `${BASE}/.prompts/<role>-<UTC-compact>.md` and spawn from that file by path — the two-step, the path-by-reference requirement, and the brace-doubling rule are canonical in the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol*. Members are spawned on demand and each gets its first task on its own ready signal, never held for other members' readiness (dispatch-on-ready, canonical in the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) § *Spawn Protocol*).

#### 1d. Spawn Presentation + Transcript in parallel

**Gate**: do not spawn Presentation or Transcript until the monitor member's `monitor live` signal (1b) has arrived.

Both work from `report.md` independently. After the slide deck is finalized (Step 3), the Director sends the final slide structure to the Transcript member for realignment.

**Presentation spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Presentation delta below (two-stage rendering + brace rules at the skeleton):

| Slot | Presentation Specialist |
|---|---|
| ROLE TITLE / TEAM | `the Presentation Specialist` / `research presentation` |
| role-file | `roles/presentation.md` |
| cafleet-load purpose + EXTRA SKILL LOADS | `for the broker primitives and bash-via-Director routing`; additionally Read the `../reference/slidev.md` page (Slidev authoring layouts + rules) + `../reference/visualization.md` (if the report includes data that would render better as a chart) |
| CONTEXT LINES | `TASK: Create a Slidev presentation from the approved research report.` / `REPORT: [INSERT <folder>/report.md]` / `RESEARCHER FILES: [INSERT <folder>/[0-9][0-9]-research-*.md]` / `LANGUAGE: [INSERT language detected from report.md]` / `FIGURE BASE: [INSERT <folder>]` (substitute literally for the FIGURE_BASE / BASE placeholders in `../reference/visualization.md`) / `OUTPUT: [INSERT <folder>/slide.md]` |
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form (capture the `id:` integer as `<message-id>` and `cafleet message ack <message-id>`, then act) |
| start cue (verbatim) | `When complete, send the file path to the Director via cafleet message send.` |

Audit file: `${BASE}/.prompts/presentation-<UTC-compact>.md`:

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "presentation" \
     --description "Authors slide.md" \
     --file ${BASE}/.prompts/presentation-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` and substitute it for `[presentation-member-id]` in subsequent `cafleet message send` calls.

**Transcript spawn prompt:**

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Transcript delta:

| Slot | Transcript Specialist |
|---|---|
| ROLE TITLE / TEAM | `the Transcript Specialist` / `research presentation` |
| role-file | `roles/transcript.md` |
| cafleet-load purpose | `for the broker primitives and bash-via-Director routing` (no extra skills) |
| CONTEXT LINES | `TASK: Create a reading transcript from the approved research report.` / `REPORT: [INSERT <folder>/report.md]` / `LANGUAGE: [INSERT language detected from report.md]` / `OUTPUT: [INSERT <folder>/transcript.md]` |
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form |
| start cue (verbatim) | `When complete, send the file path to the Director via cafleet message send.` |

Audit file: `${BASE}/.prompts/transcript-<UTC-compact>.md`:

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "transcript" \
     --description "Authors transcript.md" \
     --file ${BASE}/.prompts/transcript-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` and substitute it for `[transcript-member-id]` in subsequent `cafleet message send` calls.

### Step 2: Content Review & Revision Loop (Director)

Read the output files (`${FOLDER}/slide.md`, `${FOLDER}/transcript.md`) and review using the tag criteria in [roles/director.md](roles/director.md). Send tagged feedback via `cafleet message send`; members revise and reply. See [roles/director.md](roles/director.md) for revision approach and iteration limits.

```bash
cafleet message send --from-member-id [director-member-id] \
  --to-member-id [presentation-member-id] \
  "slide revisions: [SLIDE STRUCTURE] ... / [VISUAL] ... / ..."

cafleet message send --from-member-id [director-member-id] \
  --to-member-id [transcript-member-id] \
  "transcript revisions: [FLOW] ... / [TIMING] ... / ..."
```

Each polled inbound message MUST be `ack`ed via `cafleet message ack <message-id>` after acting on it.

Once the slide deck is finalized, send the finalized slide structure to the Transcript member for 1:1 realignment.

### Step 3: Visual Review & Fix (Director)

Once Step 2 converges on an approved slide deck and transcript, the Director runs the batched visual-review loop defined below. Per the User Interaction Contract, this step is a pipeline stage, not a decision — the Director does not call {decision_surface} to decide whether to run it, skip it, or shorten it.

**Server Startup (once):**

**Calling-pane working directory: a directory that contains the Slidev `package.json` (typically the host project root).** pnpm resolves `node_modules/` and `package.json` from the calling directory directly — no `--cwd` plumbing or sidecar directory. Project-specific task wrappers (e.g., `mise` tasks) that capture invariants like `--frozen-lockfile` belong in the host project's agent rules directory (`.claude/rules/` in this repo), not in this skill body.

1. Install pnpm dependencies — refer to your host project's agent rules directory (`.claude/rules/` in this repo) for the canonical command (it typically wraps `pnpm install --frozen-lockfile`).
2. Start the Slidev dev server **as a backgrounded process** — refer to your host project's agent rules directory (`.claude/rules/` in this repo) for the canonical launcher. The underlying invocation is `pnpm exec slidev --open false <folder>/slide.md` (the `--open false` flag is required for headless review) and the launcher MUST PTY-wrap stdout (e.g., via `script -qfc`) so Slidev does not exit on detecting a non-TTY. **Record the background process handle** your coding agent returns when it backgrounds the process — the overall Step 5 of this skill (*Finalize & Clean Up*, further down the page) needs it to stop the server cleanly without falling back on `pkill`. Run the backgrounded process via {bg_run}; stop it at teardown via {bg_stop}. On start failure: retry the start command once, then escalate to the user via {decision_surface} (options: Retry again / I started it manually — continue / Abort).
3. Set `<server_url>` to the Slidev dev server URL (default: `http://localhost:3030`). Use this value when spawning Visual Reviewers.
4. Create the persistent screenshots directory: write `<folder>/.screenshots/.keep` (empty file) using the Write tool. This is a one-time operation per presentation-workflow run; do NOT delete or wipe it on subsequent batches. agent-browser does not auto-create parent directories when given an explicit `screenshot <path>`, so this step is required for VR's per-slide capture to succeed.
5. The Director runs no agent-browser browser-operation commands — that restriction and its two narrow exceptions are canonical in [roles/director.md](roles/director.md) § *Your Accountability*.

**Batched Review Loop** (batch_size=10, fresh Visual Reviewer per batch to avoid context overflow):

Run the loop serially: spawn one VR member via `cafleet member create`, wait for its report, run the fix-and-recheck sub-loop, then run `cafleet member delete` to close the pane immediately. Do not spawn multiple VRs in parallel — fixes from one batch can affect later batches, and parallel agent-browser sessions race on the same Slidev dev server.

> **Per-batch teardown**: `cafleet member delete` kills the pane immediately (kill-pane + tmux layout rebalance) — clean context isolation per batch.

```
total_slides = count slides in slide.md
start = 1

while start <= total_slides:
    end = min(start + 9, total_slides)

    vr_round = 1                               # current VR round number; bumped on each re-check
    spawn VR member via cafleet member create (name="vr-batch-<start>") with slides [start..end], ROUND=vr_round
    # spawn prompt MUST include `RESEARCH FOLDER: <folder>` and `ROUND: 1` lines so the VR
    # can build screenshot/report paths
    # capture the printed member_id as [vr-batch-member-id] for subsequent message send / member delete

    while True:                                # initial review (r1) + up to 2 re-checks (r2, r3)
        wait for report from VR for round <vr_round> via cafleet message poll arrival
        if no issues: break
        if vr_round >= 3: break                # max 2 re-check rounds reached; remaining issues escalate to user in Step 4
        cafleet message send --from-member-id [director-member-id] \
            --to-member-id [presentation-member-id] "<tagged issues>"   # fix
        vr_round += 1
        cafleet message send --from-member-id [director-member-id] \
            --to-member-id [vr-batch-member-id] "ROUND: <vr_round>\nRe-check slides: <list>"
        # VR writes the next capture to `vr<start>-r<vr_round>-p<slide_number>.png` and
        # the next persisted report to `vr<start>-r<vr_round>.md`, preserving prior rounds

    # Explicit close handshake before deregister: the VR cannot reliably run extra commands after the exit keystroke.
    cafleet message send --from-member-id [director-member-id] \
        --to-member-id [vr-batch-member-id] "CLOSE: run `pnpm exec agent-browser --session vr-batch-<start> close`, then reply 'closed'."
    wait for the VR's "closed" confirmation via cafleet message poll
    cafleet member delete [vr-batch-member-id]
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
| IMPORTANT / coordination lines (verbatim) | **ack-inline** poll-handling form |
| start cue (verbatim) | `When complete, persist the report to <folder>/.screenshots/vr<start>-r<round>.md and send it to the Director via cafleet message send.` |

Audit file: `${BASE}/.prompts/vr-batch-<start>-<UTC-compact>.md` (`<start>` matches the batch's first-slide index used in `--name`; each VR batch gets its own timestamped file — no overwriting):

   ```bash
   cafleet member create --fleet-id [fleet-id] \
     --name "vr-batch-<start>" \
     --description "Visual Reviewer for slides <start>..<end>" \
     --file ${BASE}/.prompts/vr-batch-<start>-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` as `[vr-batch-member-id]` for subsequent `cafleet message send` / `member delete` calls.

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

Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol* (the monitor member goes first, first-out). Workflow delta: then delete Presentation, Transcript, and any active VR batch — an active VR batch gets the close handshake first (per the Step 3 loop).

Then the presentation-specific teardown:

1. **agent-browser safety net** — close any orphan browser sessions:
   ```bash
   pnpm exec agent-browser close --all
   ```
2. **Stop the Slidev dev server** via {bg_stop} (using the handle recorded in Step 3, Server Startup substep 2) — never the broad `pkill -f slidev`, which matches too widely and leaks stdout until the process is stopped through its recorded handle.
3. `cafleet fleet delete [fleet-id]`; `cafleet fleet list` to confirm. Never use raw `tmux kill-pane` / `tmux send-keys`.
