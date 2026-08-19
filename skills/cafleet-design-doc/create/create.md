# Design Doc Create (CAFleet Edition)

Create high-quality design documents using a three-role team orchestrated via the CAFleet message broker: Director (orchestrator), Drafter (writes the document), and Reviewer (critically reviews drafts). Every inter-member message is persisted in SQLite and auditable. The team iterates through an internal quality loop before presenting a polished draft to the user.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`../../cafleet/reference/coding-agent-overlays.md#<name>`](../../cafleet/reference/coding-agent-overlays.md) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — the failure modes *Resolve your overlay* closes, e.g. a literal `{bg_run}` emitted unresolved |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root every spawn-prompt audit file or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (the monitor-first spawn, the `monitor live` gate, Authorization-Scope Guard, the 5-step facilitation loop) — you spawn an unsupervised team |
| 4 | [`../reference/coordination.md`](../reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema (and the Step-2 clarification exemption) — you coordinate in free-form bodies and findings get lost / mis-routed |

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main agent | Register with CAFleet fleet, spawn members via `cafleet member create`, relay user answers, enforce clarification gate, orchestrate internal quality loop, present polished draft to user | Write the document, review it in detail | [roles/director.md](roles/director.md) |
| **Drafter** | Member | Ask clarifying questions (via Director relay), read target codebase, write and revise the design document | Communicate with user directly (goes through Director), review own work | [roles/drafter.md](roles/drafter.md) |
| **Reviewer** | Member | Critically review drafts for rule compliance, readability, completeness, correctness | Write the document, communicate with user | [roles/reviewer.md](roles/reviewer.md) |

## Coordination Protocol

This skill's Director, Drafter, and Reviewer coordinate via the verb + pointer schema and `COMMENT(role)` markers defined canonically in [../reference/coordination.md](../reference/coordination.md) — the single source of truth for the 6 verbs, the 3 pointer forms, the message format, the `COMMENT(role)` marker grammar, the issue/status split, anchorless status, finalize-time cleanup, and Director per-file detail recovery.

Two skill-specific notes layer on top of that canonical protocol:

- **Roles in play**: this skill uses only the `director`, `drafter`, `reviewer`, and `user-relay` marker roles — never `programmer`, `tester`, or `verifier` (those belong to the execute workflow). Finalize happens at `Status: Approved` (Step 6).
- **Clarification Exemption**: Director-to-Drafter messages during the **Step 2 clarification phase** are exempt from the verb + pointer schema. At clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying), so the Director's "User answers: ..." relay rides as a free-form multi-line cafleet body. From Step 3 onward (once the initial draft exists) every message falls back under the schema.

## Prerequisites

The Director MUST be running inside a tmux or herdr session and pass the gating `cafleet doctor` env-check before spawning anyone, per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol*.

## Process

### Step 0: Path Resolution & Resume Detection (Director)

**Path resolution** (before resume detection):

Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above). Then canonicalize `$ARGUMENTS` and resolve the task-scoped BASE:

Canonicalize `$ARGUMENTS` per the `cafleet` skill's `reference/base-dir.md` § *Consumer contract* row for this skill (relative forms get `design-docs/` prepended and a trailing `/design-doc.md` stripped; absolute paths are used verbatim after the filename strip), then run its **Step 0 (task-scope resolution)** with the result.

Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder and `${DOC_PATH} = ${BASE}/design-doc.md` (the task folder IS the design-doc directory; no further `${BASE}/design-docs/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${DOC_PATH}` to the **canonicalized** absolute task-folder path with `/design-doc.md` appended (unless `$ARGUMENTS` already names `design-doc.md`, in which case use it verbatim) so the Drafter receives a writable doc file path rather than a directory, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.

Pass `${DOC_PATH}` to the Drafter as OUTPUT PATH in the spawn prompt. The audit-file path `${BASE}/.prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<task-folder>/.prompts/`, not under the repo root.

**Resume detection** (using resolved `${DOC_PATH}`):

1. **File does not exist** → Fresh creation (proceed to Step 1 as normal).
2. **File exists** → Check for `COMMENT(user-relay)` markers:
   - Use Grep to search for `COMMENT(user-relay)` in the file. The grep is tightened to the `user-relay` role because user-derived clarifications are the only marker class that warrants resume-mode. Stale `COMMENT(reviewer)` / `COMMENT(director)` / `COMMENT(programmer)` markers from other workflows MUST NOT be misclassified as interview-resume (a stray execute-workflow `COMMENT(user-relay)` marker is safely treated as resume-mode — the Drafter resolves it like an interview clarification).

   - **`COMMENT(user-relay)` markers found** → This is **resume mode**. Proceed to Step 1 with the resume-specific Drafter spawn prompt. Set an internal flag `SKIP_CLARIFICATION=true` so Step 2 (clarification) is skipped.
   - **No `COMMENT(user-relay)` markers found** → Inform the user: "No `COMMENT(user-relay)` markers found in the existing document." Present two options through {decision_surface}:
     - **"Run quality review"**: Set internal flags `SKIP_CLARIFICATION=true` and `QUALITY_REVIEW_ONLY=true`. Skip Step 2 entirely and enter Step 3 by immediately routing the existing `${DOC_PATH}` to the Reviewer via `cafleet message send` (no new draft is produced; the Drafter is only involved later if the Reviewer requests revisions).
     - **"Start fresh"**: Treat as new creation, ignoring the existing file. Ensure `SKIP_CLARIFICATION` and `QUALITY_REVIEW_ONLY` are unset, then proceed to Step 1 as normal.

### Step 1: Register & Spawn Members (Director)

Load the `cafleet` skill; its `reference/supervision.md` governance is § Required reading above.

#### 1a. Establish a CAFleet fleet (monitor included) and capture the ids

Bootstrap the fleet per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Fleet bootstrap (monitor included)* (write the monitor's spawn prompt first and pass it via `--monitor-file`; the reuse-a-running-fleet rule is there too). Use `--json` so the IDs are machine-parseable:

```bash
cafleet fleet create --name "design-doc-create-{slug}" --coding-agent <backend> --monitor-file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md> --monitor-model {monitor_model} --json
# → { "fleet_id": <int>, "director": { "member_id": <int>, ... }, "monitor": { "member_id": <int>, ... } }
```

Capture `fleet_id` and `director.member_id` from the JSON response and substitute them for `<fleet-id>` and `<director-member-id>` in every subsequent command.

#### 1b. Wait for the monitor gate (before any ordinary member)

Wait for the monitor member's `ready` then `monitor live` signals per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Wait for the monitor gate* — `monitor live` gates the Drafter and Reviewer spawns (1d/1e). The monitor member runs unchanged through the quality loop and is deleted first (first-out) in Step 6's teardown.

#### 1c. Locate role definitions (path-by-reference)

The Director references each role definition by **absolute path** in the spawn prompt — the spawned member opens its role doc with `Read` at startup. Do NOT inline the role content. Resolve the absolute paths for:

- `<abs path to this skill>/roles/drafter.md`
- `<abs path to this skill>/roles/reviewer.md`

Substitute these absolute paths into the spawn prompts below.

> **Spawn frame (two-step pattern)**: render each spawn prompt to `${BASE}/.prompts/<role>-<UTC-compact>.md` per the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol*, spawn with `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --file <abs path> --json`, and parse `member_id` from the JSON response, substituting it for that role's `<x-member-id>` in every subsequent command.

#### 1d. Spawn the Drafter

**Gate**: do not spawn the Drafter until the monitor member's `monitor live` signal (1b) has arrived.

**Drafter spawn prompt** — render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the per-role delta below (two-stage rendering + brace rules at the skeleton). Keep the prompt under ~2 KB (path-by-reference). Use the normal-mode column by default; the resume-mode column when Step 0 detected resume mode.

| Slot | Drafter (normal mode) | Drafter (resume mode) |
|---|---|---|
| ROLE TITLE / TEAM | `the Drafter` / `design document creation` | `the Drafter` / `design document creation`, with `(CAFleet-native, RESUME MODE)` in the identity line |
| role-file + ROLE-DEF suffix | `roles/drafter.md` | `roles/drafter.md`; add suffix `Follow the Resume Mode section in particular.` |
| EXTRA SKILL LOADS | `cafleet-design-doc` (template + guidelines) | same |
| CONTEXT LINES | `OUTPUT PATH: [INSERT DOC PATH]` + a blank line + `The user's request: [INSERT USER'S ORIGINAL REQUEST]` | `DESIGN DOCUMENT: [INSERT DOC PATH]` |
| IMPORTANT / start cue (verbatim) | `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.` / `Send your questions to the Director who will relay them to the user.` / `Start by reading the target codebase for context, then send your clarifying questions.` / `Do NOT create any design document file until you have received answers.` | `This is a RESUME run. The document contains COMMENT markers from a previous interview. Follow the Resume Mode instructions in your role definition.` / `Do NOT ask clarifying questions — the COMMENTs contain the needed information.` / `Start by reading the design document.` |

Spawn per the Step 1c spawn frame (both normal and resume modes). Worked example — the one full command block of this file; the Reviewer spawn reuses the frame with its own literals:

   ```bash
   cafleet member create --fleet-id <fleet-id> \
     --name "Drafter" \
     --description "Writes and revises the design document" \
     --file ${BASE}/.prompts/drafter-<UTC-compact>.md \
     --json
   ```

#### 1e. Spawn the Reviewer

**Reviewer spawn prompt** — the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with this delta:

| Slot | Reviewer |
|---|---|
| ROLE TITLE / TEAM | `the Reviewer` / `design document creation` |
| role-file | `roles/reviewer.md` |
| EXTRA SKILL LOADS | `cafleet-design-doc` (template + guidelines) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT DOC PATH]` |
| start cue (verbatim) | `Wait for the Director to assign a document for review (cafleet body: ready (doc)). When you receive that message, the doc pointer refers to the DESIGN DOCUMENT path above — read that file and provide specific, actionable feedback per the role definition.` |
| `--name` / `--description` | `Reviewer` / `Critically reviews drafts for rule compliance and quality` |

Spawn per the Step 1c spawn frame (audit file `${BASE}/.prompts/reviewer-<UTC-compact>.md`).

#### 1f. Spawn-health placement audit (non-gating)

```bash
cafleet member list <fleet-id>
```

Placement-audit semantics — non-gating, retry a missing or pending row, dispatch rides each member's ready signal — per [`supervision.md`](../../cafleet/reference/supervision.md) § *Spawn Protocol*. The Drafter's first task is embedded in its spawn prompt and needs no separate dispatch, so Step 2 proceeds on the Drafter's ready signal regardless of the Reviewer's state.

### Step 2: Clarification Phase (Director)

**Skip this step entirely when `SKIP_CLARIFICATION=true`** (set by Step 0 in resume mode or quality-review-only mode). Resume mode: the COMMENT markers serve as the clarification and the Drafter already has all the information needed. Quality-review-only mode: the Drafter is not producing a new draft at all — proceed directly to Step 3 by routing the existing `${DOC_PATH}` to the Reviewer.

> **Clarification Exemption** ([Coordination Protocol above](#coordination-protocol)): Director-to-Drafter messages in this step ride as free-form multi-line cafleet bodies — the design doc does not yet exist. From Step 3 onward every message falls back under the schema.

1. Wait for the Drafter's clarifying questions. The broker's inline-preview keystroke on the Drafter's `message send`, and your own periodic `cafleet message poll <director-member-id>`, will surface the Drafter's message once it arrives.
2. `cafleet message ack <message-id>` each received message after reading it.
3. Relay the questions to the user via {decision_surface}. If {decision_surface} caps how many questions it shows at once (your overlay states the cap) and the number exceeds it, split them into multiple sequential calls to relay all questions without omission.
4. Relay the user's answers back to the Drafter (free-form, per the Clarification Exemption above):
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <drafter-member-id> "User answers: ..."
   ```
5. **Gate check**: If the Drafter produces a draft without prior questions, reject it and instruct them to ask first (also free-form, per the Clarification Exemption):
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <drafter-member-id> "Stop — you must send clarifying questions before drafting. Discard the draft and send questions first."
   ```
   A focused confirmation round counts as valid clarification.

### Step 3: Internal Quality Loop (Director)

Enter this step after the Drafter reports `complete (doc)`, **or immediately** when `QUALITY_REVIEW_ONLY=true` (the existing `${DOC_PATH}` is treated as the "completed draft" — no waiting for a Drafter report):

1. **Route to Reviewer**. The Reviewer reads `${DOC_PATH}` directly; no path needs to be embedded in the cafleet body.
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <reviewer-member-id> "ready (doc)"
   ```
2. **Wait** for the Reviewer's response via `cafleet message poll <director-member-id>`. Round-1 fresh review arrives as `complete (doc) — N issues`; approval arrives as `approved (doc)`. Each finding is recorded as a `COMMENT(reviewer): [TAG] <body>` marker inline in the design doc — the Director does NOT relay the finding text in cafleet.
3. **On feedback**: Route the Drafter to address the markers in-doc:
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <drafter-member-id> "ready (doc)"
   ```
4. Wait for the Drafter's `addressed (doc)` reply (revisions resolve the `COMMENT(reviewer)` markers), then loop back to step 1 (re-route to Reviewer with `ready (doc)`).
5. Repeat until the Reviewer explicitly signals `approved (doc)`.
6. **Iteration limit**: Aim for 2–3 rounds. If not converging, escalate to the user: summarize the remaining issues at a high level (read directly from the surviving `COMMENT(reviewer)` markers in the doc) and use {decision_surface} to ask whether to continue iterating or abort. Do not proceed to Step 4 until the Reviewer has approved.

### Step 4: Present to User (Director)

Only after the Reviewer explicitly approves, present a summary (including file path) and use {decision_surface}:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with the current result | Proceed to finalization (Step 6) |
| 2 | **Scan for COMMENT markers** | Immediately scan the document for `COMMENT(name): feedback` markers and process them | Scan immediately and process markers (see Step 5) |
| 3 | *(Other — built-in)* | *(Free text input)* | Interpret user intent (see Step 5) |

Intent judgment and abort detection for free-text replies: [roles/director.md](roles/director.md) § *Free-form user replies*.

### Step 5: User Feedback Loop (Director)

This step owns the user-feedback COMMENT-scan procedure. Process the user's selection:

- **"Scan for COMMENT markers"**: scan immediately with Grep — the selection itself is the signal, do NOT wait for the user to confirm they are done editing. If markers are found, route the Drafter with `ready (doc)`:
  ```bash
  cafleet message send --from-member-id <director-member-id> \
    --to-member-id <drafter-member-id> "ready (doc)"
  ```
  After the Drafter replies `addressed (doc)` and removes the markers, verify with Grep that no `COMMENT(` markers remain, then re-enter the quality loop (Step 3) and re-present (Step 4). If no markers are found, explain the marker convention to the user (`# COMMENT(username): feedback` placed directly in the design document), show the file path so the user can edit it, then re-prompt with the same three-option pattern.
- **Free-text response**: judge abort vs non-abort intent per [roles/director.md](roles/director.md) § *Free-form user replies* (LLM reasoning, not keyword matching). Abort intent → the Abort Flow (Shutdown Protocol, Step 6, without Drafter finalization); non-abort → explain the COMMENT-marker channel and re-prompt.

No round limit — loop continues until approved or aborted.

### Step 6: Finalize & Clean Up (Director)

1. Instruct the Drafter to finalize. The Drafter's role definition spells out the finalize checklist (set Status to Approved, refresh Last Updated, bump the Progress header field if present, verify implementation steps are actionable); the cafleet body is just the verb + pointer poke:
   ```bash
   cafleet message send --from-member-id <director-member-id> \
     --to-member-id <drafter-member-id> "ready (doc)"
   ```
   Wait for the Drafter's `addressed (doc)` confirmation.

2. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol* (the monitor member goes first, first-out). Workflow delta: then delete the Drafter and Reviewer.

The fleet row is soft-deleted and `messages` rows are preserved so the message trail remains inspectable in the broker database.
