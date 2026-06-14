---
name: cafleet-design-doc-create
description: "Create a new design document using CAFleet-native orchestration (Director/Drafter/Reviewer team). Use when the user asks to create a design doc, design document, specification, or technical spec. Do NOT write the doc directly with Write or use EnterPlanMode — always invoke this skill."
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---

# Design Doc Create (CAFleet Edition)

Create high-quality design documents using a three-role team orchestrated via the CAFleet message broker: Director (orchestrator), Drafter (writes the document), and Reviewer (critically reviews drafts). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The team iterates through an internal quality loop before presenting a polished draft to the user.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet fleet, spawn members via `cafleet member create`, relay user answers, enforce clarification gate, orchestrate internal quality loop, present polished draft to user | Write the document, review it in detail | [roles/director.md](roles/director.md) |
| **Drafter** | Member agent (claude) | Ask clarifying questions (via Director relay), read target codebase, write and revise the design document | Communicate with user directly (goes through Director), review own work | [roles/drafter.md](roles/drafter.md) |
| **Reviewer** | Member agent (claude) | Critically review drafts for rule compliance, readability, completeness, correctness | Write the document, communicate with user | [roles/reviewer.md](roles/reviewer.md) |

## Additional resources

- For the document template, see: [../cafleet-design-doc/template.md](../cafleet-design-doc/template.md)
- For section guidelines and quality standards, see: [../cafleet-design-doc/guidelines.md](../cafleet-design-doc/guidelines.md)
- For the inter-agent coordination protocol (verb + pointer schema, `COMMENT(role)` markers), see: [../cafleet-design-doc/coordination.md](../cafleet-design-doc/coordination.md)

## Coordination Protocol

This skill's Director, Drafter, and Reviewer coordinate via the verb + pointer schema and `COMMENT(role)` markers defined canonically in [../cafleet-design-doc/coordination.md](../cafleet-design-doc/coordination.md) — the single source of truth for the 6 verbs, the 3 pointer forms, the message format, the `COMMENT(role)` marker grammar, the issue/status split, Copilot routing, anchorless status, finalize-time cleanup, and Director per-file detail recovery.

Two skill-specific notes layer on top of that canonical protocol:

- **Roles in play**: this skill uses only the `director`, `drafter`, `reviewer`, `claude`, and `copilot` marker roles — never `programmer`, `tester`, or `verifier` (those belong to the `cafleet-design-doc-execute` skill). Copilot review in this skill is design-doc-anchored only, routed through `COMMENT(director)`; finalize happens at `Status: Approved` (Step 6).
- **Clarification Exemption**: Director-to-Drafter messages during the **Step 2 clarification phase** are exempt from the verb + pointer schema. At clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying), so the Director's "User answers: ..." relay rides as a free-form multi-line cafleet body. From Step 3 onward (once the initial draft exists) every message falls back under the schema.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` (no separate `cafleet agent register` call) — and spawns both the Drafter and Reviewer via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet fleet create, cafleet member create, orchestrates cycle)
      +-- Drafter (member agent -- spawned in tmux pane; writes the design document)
      +-- Reviewer (member agent -- spawned in tmux pane; critically reviews the draft)
```

- **Director ↔ User**: `AskUserQuestion` (clarification relay, draft presentation, feedback collection)
- **Director ↔ Drafter**: `cafleet message send` (questions relay, user answers, reviewer feedback, drafting instructions)
- **Director ↔ Reviewer**: `cafleet message send` (draft review requests, review feedback)
- Members receive messages via a push notification: the broker keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into the member's pane via `tmux.send_inline_preview` whenever a `cafleet message send` is persisted. The recipient processes the preview as a fresh user-turn input — no `cafleet message poll` invocation is in the auto-fire path; to fetch the full body, the recipient calls `cafleet message poll` themselves. `--fleet-id` is global (before the subcommand); `--agent-id` is per-subcommand (after the subcommand name).

## Prerequisites

The Director MUST be running inside a tmux session (required by `cafleet member create`). Verify by running `cafleet doctor` before spawning anyone — it reports the tmux session/window/pane identifiers and exits non-zero with a clear message when the environment is not ready. If `cafleet doctor` reports a problem, abort and surface its message to the user. Do NOT invoke `tmux display-message`, `printenv TMUX`, or any other raw tmux/env probe — `cafleet doctor` is the only supported environment check (see `skills/cafleet/SKILL.md` § *use cafleet primitives only*).

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="create-{slug}")` | CAFleet fleet created via `cafleet fleet create` — it bootstraps the fleet + root Director + placement + Administrator in one transaction (no separate `cafleet agent register` call needed for the Director) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> --name "..." --description "..." -- "<prompt>"` |
| `SendMessage(to="Drafter")` | `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <drafter-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from member) | `cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `cafleet-agent-team-supervision` supervision tick | Load the `cafleet-agent-team-monitoring` skill (heartbeat + facilitation) and the `cafleet-agent-team-supervision` skill (governance), then start the heartbeat via `cafleet --fleet-id <fleet-id> monitor start` |
| `TeamDelete` | `cafleet --fleet-id <fleet-id> member delete --member-id <member-agent-id>` for each member, then `cafleet fleet delete <fleet-id>` (soft-deletes the fleet, deregisters the root Director + Administrator + any surviving members in one transaction). The root Director cannot be deregistered via `cafleet agent deregister` — `fleet delete` is the only supported teardown. |
| Auto message delivery | Push notification keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into member's tmux pane via `tmux.send_inline_preview` |

## Process

### Step 0: Path Resolution & Resume Detection (Director)

**Path resolution** (before resume detection):

Load the `cafleet-base-dir` skill for the no-bypass write protocol and `<unset>` sentinel contract. Then canonicalize `$ARGUMENTS` and resolve the task-scoped BASE:

- **Relative input** — accept any of: `0000060-foo`, `0000060-foo/design-doc.md`, `design-docs/0000060-foo`, `design-docs/0000060-foo/design-doc.md`. Canonicalize to `design-docs/<slug>` by: (1) stripping the trailing `/design-doc.md` if present; (2) stripping the leading `design-docs/` if present; (3) prepending `design-docs/`. The skill's Step 0 does NOT perform this stripping (per the `cafleet-base-dir` skill § *Consumer contract*) — canonicalize first, then run the skill's **Step 0 (task-scope resolution)** with the relpath `design-docs/<slug>`.

- **Absolute path** (e.g. `/abs/path/to/design-docs/0000060-skill-task-scoped-base-dir/design-doc.md`): Step 0 accepts only the task-folder path, not a child file. Strip the trailing `/design-doc.md` if present so the absolute path identifies the task folder, then run Step 0 with that absolute task-folder path. Step 0 accepts the absolute path if it lies strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder and `${DOC_PATH} = ${BASE}/design-doc.md` (the task folder IS the design-doc directory; no further `${BASE}/design-docs/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${DOC_PATH}` to the **canonicalized** absolute task-folder path with `/design-doc.md` appended (unless `$ARGUMENTS` already names `design-doc.md`, in which case use it verbatim) so the Drafter receives a writable doc file path rather than a directory, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet-base-dir` skill § *The `<unset>` sentinel*.

Pass `${DOC_PATH}` to the Drafter as OUTPUT PATH in the spawn prompt. The audit-file path `${BASE}/prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<task-folder>/prompts/`, not under the repo root.

**Resume detection** (using resolved `${DOC_PATH}`):

1. **File does not exist** → Fresh creation (proceed to Step 1 as normal).
2. **File exists** → Check for `COMMENT(claude)` markers:
   - Use Grep to search for `COMMENT(claude)` in the file. The grep is tightened to the `claude` role because user-derived clarifications are the only marker class that warrants resume-mode. Stale `COMMENT(reviewer)` / `COMMENT(director)` / `COMMENT(programmer)` markers from other workflows MUST NOT be misclassified as interview-resume. Note: the `cafleet-design-doc-execute` skill also writes transient `COMMENT(claude): <choice>` markers for user arbitration (e.g., test-framework selection in `Phase 1`), but those are short-lived and removed by the routed member as part of the fix; under normal flow no `COMMENT(claude)` survives an in-progress execute run. If the user invokes the `cafleet-design-doc-create` skill against a half-finished execute doc that happens to carry a transient `COMMENT(claude)`, treating it as resume-mode is acceptable — the Drafter will read and resolve the marker the same way it handles interview clarifications.

   - **`COMMENT(claude)` markers found** → This is **resume mode**. Proceed to Step 1 with the resume-specific Drafter spawn prompt. Set an internal flag `SKIP_CLARIFICATION=true` so Step 2 (clarification) is skipped.
   - **No `COMMENT(claude)` markers found** → Inform the user: "No `COMMENT(claude)` markers found in the existing document." Use `AskUserQuestion` with two options:
     - **"Run quality review"**: Set internal flags `SKIP_CLARIFICATION=true` and `QUALITY_REVIEW_ONLY=true`. Skip Step 2 entirely and enter Step 3 by immediately routing the existing `${DOC_PATH}` to the Reviewer via `cafleet message send` (no new draft is produced; the Drafter is only involved later if the Reviewer requests revisions).
     - **"Start fresh"**: Treat as new creation, ignoring the existing file. Ensure `SKIP_CLARIFICATION` and `QUALITY_REVIEW_ONLY` are unset, then proceed to Step 1 as normal.

### Step 1: Register & Spawn Members (Director)

Load the `cafleet` skill, the `cafleet-agent-team-monitoring` skill, and the `cafleet-agent-team-supervision` skill (in that order — monitoring is the foundation layer, supervision the governance layer that depends on it).

#### 1a. Establish a CAFleet fleet and capture the root Director's `agent_id`

`cafleet fleet create` (which must be run inside a tmux session) atomically creates the fleet and registers a root Director bound to the current tmux pane — there is no separate `cafleet agent register` step for the Director. Use `--json` so both IDs are machine-parseable:

```bash
cafleet fleet create --label "design-doc-create-{slug}" --json
# → {
#     "fleet_id": "550e8400-e29b-41d4-a716-446655440000",
#     "label": "design-doc-create-{slug}",
#     "created_at": "…",
#     "administrator_agent_id": "…",
#     "director": {
#       "agent_id": "7ba91234-…",
#       "name": "Director",
#       "description": "Root Director for this fleet",
#       "registered_at": "…",
#       "placement": { "director_agent_id": null, "tmux_session": "…", "tmux_window_id": "…", "tmux_pane_id": "…", "coding_agent": "unknown", "created_at": "…" }
#     }
#   }
```

Capture `fleet_id` and `director.agent_id` from the JSON response. Substitute them for `<fleet-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal ids.

If you already have a running fleet (e.g. an outer orchestration), reuse its `fleet_id` and its root Director's `agent_id` instead of creating a new fleet. Do **not** attempt to register a second Director with `cafleet agent register --name Director` — the root Director from `fleet create` is the team lead; a second registration would just create an unrelated agent with no placement row.

#### 1b. Start the monitor

BEFORE spawning any member, run the supervision heartbeat as a **background task** with `cafleet --fleet-id <fleet-id> monitor start` (the loop runs in-process and blocks the task; confirm with `cafleet --fleet-id <fleet-id> monitor status`). The monitor must stay running from the first `member create` until Step 6's shutdown cleanup. Supervision obligations (Authorization-Scope Guard, idle semantics, etc.) come from the `cafleet-agent-team-supervision` skill, which loads the `cafleet-agent-team-monitoring` skill as a hard prerequisite.

#### 1c. Locate role definitions (path-by-reference)

The Director references each role definition by **absolute path** in the spawn prompt — the spawned member opens its role doc with `Read` at startup. Do NOT inline the role content. Resolve the absolute paths for:

- `<abs path to this skill>/roles/drafter.md`
- `<abs path to this skill>/roles/reviewer.md`

Substitute these absolute paths into the spawn prompts below.

> **Why path-by-reference (and not inline-verbatim)**: cafleet `member create` passes the prompt to `tmux split-window` as a single positional argument. The cumulative caller-shell + cafleet-argv + tmux-argv budget exhausts well below `ARG_MAX` and surfaces as `command too long` once the shell-quoted prompt grows past a few KB. The role file is typically large enough that inlining it exceeds the limit. The member loads the role file via `Read` on its first turn. See the `cafleet` skill's `reference/director.md` reference file § *Spawn prompt size limit* for the canonical write-up.
>
> **Spawn-prompt audit file (two-step pattern)**: every spawn in this skill follows the same two steps — (1) **render** the prompt (substitute the `[INSERT …]` markers; leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass); (2) **write** it to `${BASE}/prompts/<role>-<UTC-compact>.md` (`<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`; create `${BASE}/prompts/` on first write; same-second collision → append `_2`, `_3`, … — never overwrite), then invoke `cafleet member create --prompt-file <abs path>` (see Step 1d / 1e for the per-role spawn templates and commands). The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet-base-dir` skill § *No-bypass write protocol* and the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

#### 1d. Spawn the Drafter

**Drafter spawn prompt (normal mode):**

The spawn prompt's identity block uses `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders — cafleet `member create` runs `str.format()` over the entire prompt and substitutes these from the new member's allocated `agent_id`, the fleet ID, and the spawning Director's `agent_id`. The `[INSERT …]` markers in the prompt (e.g. `[INSERT DOC PATH]`, `[INSERT USER'S ORIGINAL REQUEST]`, `[INSERT abs path to roles/drafter.md]`) are NOT format placeholders — the Director substitutes them in shell before calling `member create`. See the Template safety note under `Member Create` in `skills/cafleet/reference/director.md` — any literal `{` / `}` you embed in a custom prompt must be doubled (`{{` / `}}`), and prompts MUST stay under ~2 KB (see Step 1c above on path-by-reference).

```
You are the Drafter in a design document creation team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/drafter.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
OUTPUT PATH: [INSERT DOC PATH]

The user's request: [INSERT USER'S ORIGINAL REQUEST]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.
Send your questions to the Director who will relay them to the user.
Start by reading the target codebase for context, then send your clarifying questions.
Do NOT create any design document file until you have received answers.
```

**Drafter spawn prompt (resume mode):**

Use this instead when Step 0 detected resume mode:

```
You are the Drafter in a design document creation team (CAFleet-native, RESUME MODE).

ROLE DEFINITION: Open [INSERT abs path to roles/drafter.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Follow the Resume Mode section in particular. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

This is a RESUME run. The document contains COMMENT markers from a previous
interview. Follow the Resume Mode instructions in your role definition.
Do NOT ask clarifying questions — the COMMENTs contain the needed information.
Start by reading the design document.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact — the CLI's `str.format()` pass resolves them at member-create time using the newly-allocated `agent_id`.
2. **Write the rendered text** to `${BASE}/prompts/drafter-<UTC-compact>.md` per the two-step audit-file procedure in Step 1c (both normal and resume modes — the resume-mode rendered body indicates the COMMENT-resolution flow).
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Drafter" \
     --description "Writes and revises the design document" \
     --prompt-file ${BASE}/prompts/drafter-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<drafter-agent-id>` in every subsequent command.

#### 1e. Spawn the Reviewer

**Reviewer spawn prompt:**

```
You are the Reviewer in a design document creation team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/reviewer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

Wait for the Director to assign a document for review (cafleet body: `ready (doc)`). When you receive that message, the `doc` pointer refers to the DESIGN DOCUMENT path above — read that file and provide specific, actionable feedback per the role definition.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/reviewer-<UTC-compact>.md` per the two-step audit-file procedure in Step 1c.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Reviewer" \
     --description "Critically reviews drafts for rule compliance and quality" \
     --prompt-file ${BASE}/prompts/reviewer-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<reviewer-agent-id>` in every subsequent command.

#### 1f. Verify members are live

```bash
cafleet --fleet-id <fleet-id> member list
```

Both members must show `status: active` with a non-null `pane_id`. If either is missing or pending, retry the spawn before proceeding.

### Step 2: Clarification Phase (Director)

**Skip this step entirely when `SKIP_CLARIFICATION=true`** (set by Step 0 in resume mode or quality-review-only mode). Resume mode: the COMMENT markers serve as the clarification and the Drafter already has all the information needed. Quality-review-only mode: the Drafter is not producing a new draft at all — proceed directly to Step 3 by routing the existing `${DOC_PATH}` to the Reviewer.

> **Clarification Exemption**: Director-to-Drafter messages during this step are exempt from the verb + pointer schema documented in [the Coordination Protocol section above](#coordination-protocol). At clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying), so there is no in-doc target for `COMMENT(claude)` markers — the user-answer relay rides as a free-form multi-line cafleet body. From Step 3 onward (once the initial draft exists) every message falls back under the schema.

1. Wait for the Drafter's clarifying questions. The monitor wake and periodic `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>` will surface the Drafter's message once it arrives.
2. `cafleet --fleet-id <fleet-id> message ack --agent-id <director-agent-id> --task-id <task-id>` each received message after reading it.
3. Relay the questions to the user via `AskUserQuestion`. If the number of questions exceeds the per-call limit of `AskUserQuestion`, split them into multiple sequential calls to relay all questions without omission.
4. Relay the user's answers back to the Drafter (free-form, per the Clarification Exemption above):
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "User answers: ..."
   ```
5. **Gate check**: If the Drafter produces a draft without prior questions, reject it and instruct them to ask first (also free-form, per the Clarification Exemption):
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "Stop — you must send clarifying questions before drafting. Discard the draft and send questions first."
   ```
   A focused confirmation round counts as valid clarification.

### Step 3: Internal Quality Loop (Director)

Enter this step after the Drafter reports `complete (doc)`, **or immediately** when `QUALITY_REVIEW_ONLY=true` (the existing `${DOC_PATH}` is treated as the "completed draft" — no waiting for a Drafter report):

1. **Route to Reviewer**. The Reviewer reads `${DOC_PATH}` directly; no path needs to be embedded in the cafleet body.
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <reviewer-agent-id> --text "ready (doc)"
   ```
2. **Wait** for the Reviewer's response via `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>`. Round-1 fresh review arrives as `complete (doc) — N issues`; approval arrives as `approved (doc)`. Each finding is recorded as a `COMMENT(reviewer): [TAG] <body>` marker inline in the design doc — the Director does NOT relay the finding text in cafleet.
3. **On feedback**: Route the Drafter to address the markers in-doc:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "ready (doc)"
   ```
4. Wait for the Drafter's `addressed (doc)` reply (revisions resolve the `COMMENT(reviewer)` markers), then loop back to step 1 (re-route to Reviewer with `ready (doc)`).
5. Repeat until the Reviewer explicitly signals `approved (doc)`.
6. **Iteration limit**: Aim for 2–3 rounds. If not converging, escalate to the user: summarize the remaining issues at a high level (read directly from the surviving `COMMENT(reviewer)` markers in the doc) and use `AskUserQuestion` to ask whether to continue iterating or abort. Do not proceed to Step 4 until the Reviewer has approved.

### Step 4: Present to User (Director)

Only after the Reviewer explicitly approves, present a summary (including file path) and use `AskUserQuestion`:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with the current result | Proceed to finalization (Step 6) |
| 2 | **Scan for COMMENT markers** | Immediately scan the document for `COMMENT(name): feedback` markers and process them | Scan immediately and process markers (see Step 5) |
| 3 | *(Other — built-in)* | *(Free text input)* | Interpret user intent (see Step 5) |

See [roles/director.md](roles/director.md) for user interaction rules (COMMENT handling, intent judgment, abort detection).

### Step 5: User Feedback Loop (Director)

Process the user's selection:

- **"Scan for COMMENT markers"**:
  1. **Immediately** scan the document with Grep for `COMMENT(` markers — do NOT wait for the user to confirm they are done editing. The selection itself is the signal to scan now.
  2. **If markers are found**: Route the Drafter to read and resolve the markers in-doc with `ready (doc)` — no marker content is quoted in the cafleet body:
     ```bash
     cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
       --to <drafter-agent-id> --text "ready (doc)"
     ```
     After the Drafter replies `addressed (doc)` and removes the markers, verify with Grep that no `COMMENT(` markers remain. Then re-enter the quality loop (Step 3) and re-present (Step 4).
  3. **If no markers are found**: Explain the COMMENT marker convention to the user — markers follow the pattern `# COMMENT(username): feedback` placed directly in the design document file. Show the file path so the user can edit it. Then re-prompt with the same three-option pattern (Approve / Scan for COMMENT markers / Other).

- **"Other" (free text)**: Use LLM reasoning — not keyword matching — to distinguish between:
  - **Abort intent** (user wants to stop or cancel the process): Trigger the Abort Flow — follow the Shutdown Protocol (Step 6) without Drafter finalization.
  - **Non-abort intent** (user providing verbal feedback or asking a question): Explain that feedback should be provided via COMMENT markers in the design document, then re-prompt with the same three-option pattern.

No round limit — loop continues until approved or aborted.

### Step 6: Finalize & Clean Up (Director)

1. Instruct the Drafter to finalize. The Drafter's role definition spells out the finalize checklist (set Status to Approved, refresh Last Updated, bump the Progress header field if present, verify implementation steps are actionable); the cafleet body is just the verb + pointer poke:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "ready (doc)"
   ```
   Wait for the Drafter's `addressed (doc)` confirmation.

2. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol*:
   1. Stop the monitor's background task (started at Step 1b — there is no `monitor stop` command; stop the background task running `cafleet monitor start`).
   2. `cafleet member delete` for each member (Drafter, then Reviewer). Each call blocks until the pane is gone (15 s timeout); on exit 2 follow the `member capture` + `send-input` recovery in the canonical protocol, or rerun with `--force`.
   3. `cafleet member list` — the team's roster MUST be empty before continuing.
   4. `cafleet fleet delete <fleet-id>` (positional, no `--fleet-id` flag).
   5. `cafleet fleet list` — the fleet MUST not appear (soft-deleted fleets are hidden).

The fleet row is soft-deleted and `tasks` are preserved so the message trail remains inspectable in the admin WebUI.
