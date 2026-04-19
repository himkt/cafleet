---
name: design-doc-create
description: Create a new design document using CAFleet-native orchestration. Use when user wants to create a specification or technical document with CAFleet message broker coordination. Do NOT use EnterPlanMode — always invoke this skill instead.
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---

# Design Doc Create (CAFleet Edition)

Create high-quality design documents using a three-role team orchestrated via the CAFleet message broker: Director (orchestrator), Drafter (writes the document), and Reviewer (critically reviews drafts). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The team iterates through an internal quality loop before presenting a polished draft to the user.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet session, spawn members via `cafleet member create`, relay user answers, enforce clarification gate, orchestrate internal quality loop, present polished draft to user | Write the document, review it in detail | [roles/director.md](roles/director.md) |
| **Drafter** | Member agent (claude) | Ask clarifying questions (via Director relay), read target codebase, write and revise the design document | Communicate with user directly (goes through Director), review own work | [roles/drafter.md](roles/drafter.md) |
| **Reviewer** | Member agent (claude) | Critically review drafts for rule compliance, readability, completeness, correctness | Write the document, communicate with user | [roles/reviewer.md](roles/reviewer.md) |

## Additional resources

- For the document template, see: [../design-doc/template.md](../design-doc/template.md)
- For section guidelines and quality standards, see: [../design-doc/guidelines.md](../design-doc/guidelines.md)

## Architecture

The Director registers with a CAFleet session and spawns both the Drafter and Reviewer via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet register, cafleet member create, orchestrates cycle)
      +-- Drafter (member agent -- spawned in tmux pane; writes the design document)
      +-- Reviewer (member agent -- spawned in tmux pane; critically reviews the draft)
```

- **Director ↔ User**: `AskUserQuestion` (clarification relay, draft presentation, feedback collection)
- **Director ↔ Drafter**: `cafleet send` (questions relay, user answers, reviewer feedback, drafting instructions)
- **Director ↔ Reviewer**: `cafleet send` (draft review requests, review feedback)
- Members receive messages via a push notification: the broker injects `cafleet --session-id <session-id> poll --agent-id <recipient-agent-id>` into the member's pane via `tmux send-keys` whenever a `cafleet send` is persisted. The literal `<session-id>` and `<recipient-agent-id>` UUIDs are the session and target member UUIDs the broker has in scope, baked into the injected command string. `--session-id` is global (before the subcommand); `--agent-id` is per-subcommand (after the subcommand name).

## Prerequisites

The Director MUST be running inside a tmux session (required by `cafleet member create`). If `TMUX` is not set, abort with an explanatory message to the user before spawning anyone.

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="create-{slug}")` | CAFleet session created via `cafleet session create` — it bootstraps the session + root Director + placement + Administrator in one transaction (no separate `cafleet register` call needed for the Director) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name "..." --description "..." -- "<prompt>"` |
| `SendMessage(to="Drafter")` | `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <drafter-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from member) | `cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `agent-team-supervision` `/loop` | `Skill(cafleet-monitoring)` `/loop` |
| `TeamDelete` | `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <member-agent-id>` for each member, then `cafleet session delete <session-id>` (soft-deletes the session, deregisters the root Director + Administrator + any surviving members in one transaction). The root Director cannot be deregistered via `cafleet deregister` — `session delete` is the only supported teardown. |
| Auto message delivery | Push notification injects `cafleet --session-id <session-id> poll --agent-id <recipient-agent-id>` into member's tmux pane |

## Process

### Step 0: Path Resolution & Resume Detection (Director)

**Path resolution** (before resume detection):

Load `Skill(base-dir)` and follow its procedure with `$ARGUMENTS` as the argument.
- If skipped (absolute path): set `${DOC_PATH} = $ARGUMENTS`.
- If base resolved: set `${DOC_PATH} = ${BASE}/design-docs/$ARGUMENTS`. Resolve to absolute path.

Pass `${DOC_PATH}` to the Drafter as OUTPUT PATH in the spawn prompt.

**Resume detection** (using resolved `${DOC_PATH}`):

1. **File does not exist** → Fresh creation (proceed to Step 1 as normal).
2. **File exists** → Check for COMMENT markers:
   - Use Grep to search for `COMMENT(` in the file.
   - **COMMENT markers found** → This is **resume mode**. Proceed to Step 1 with the resume-specific Drafter spawn prompt. Set an internal flag `SKIP_CLARIFICATION=true` so Step 2 (clarification) is skipped.
   - **No COMMENT markers found** → Inform the user: "No COMMENT markers found in the existing document." Use `AskUserQuestion` with two options:
     - **"Run quality review"**: Set internal flags `SKIP_CLARIFICATION=true` and `QUALITY_REVIEW_ONLY=true`. Skip Step 2 entirely and enter Step 3 by immediately routing the existing `${DOC_PATH}` to the Reviewer via `cafleet send` (no new draft is produced; the Drafter is only involved later if the Reviewer requests revisions).
     - **"Start fresh"**: Treat as new creation, ignoring the existing file. Ensure `SKIP_CLARIFICATION` and `QUALITY_REVIEW_ONLY` are unset, then proceed to Step 1 as normal.

### Step 1: Register & Spawn Members (Director)

Load `Skill(cafleet)` and `Skill(cafleet-monitoring)`.

#### 1a. Establish a CAFleet session and capture the root Director's `agent_id`

`cafleet session create` (which must be run inside a tmux session) atomically creates the session and registers a root Director bound to the current tmux pane — there is no separate `cafleet register` step for the Director. Use `--json` so both IDs are machine-parseable:

```bash
cafleet session create --label "design-doc-create-{slug}" --json
# → {
#     "session_id": "550e8400-e29b-41d4-a716-446655440000",
#     "label": "design-doc-create-{slug}",
#     "created_at": "…",
#     "administrator_agent_id": "…",
#     "director": {
#       "agent_id": "7ba91234-…",
#       "name": "Director",
#       "description": "Root Director for this session",
#       "registered_at": "…",
#       "placement": { "director_agent_id": null, "tmux_session": "…", "tmux_window_id": "…", "tmux_pane_id": "…", "coding_agent": "unknown", "created_at": "…" }
#     }
#   }
```

Capture `session_id` and `director.agent_id` from the JSON response. Substitute them for `<session-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal UUIDs.

If you already have a running session (e.g. an outer orchestration), reuse its `session_id` and its root Director's `agent_id` instead of creating a new session. Do **not** attempt to register a second Director with `cafleet register --name Director` — the root Director from `session create` is the team lead; a second registration would just create an unrelated agent with no placement row.

#### 1c. Start the monitoring `/loop`

BEFORE spawning any member, follow `Skill(cafleet-monitoring)`'s Monitoring Mandate and start a `/loop` monitor at the 1-minute interval using the literal `<session-id>` and `<director-agent-id>` UUIDs. The loop must stay active from the first `member create` until Step 6's shutdown cleanup.

#### 1d. Read role definitions

Read the role files that will be embedded verbatim in spawn prompts:

- `.claude/skills/design-doc-create/roles/drafter.md`
- `.claude/skills/design-doc-create/roles/reviewer.md`

#### 1e. Spawn the Drafter

**Drafter spawn prompt (normal mode):**

When constructing the prompt, substitute the literal `<session-id>` and `<director-agent-id>` UUIDs for the placeholders below. The new member's own `<my-agent-id>` will be allocated by `member create` and baked into the spawn prompt automatically by the `coding_agent.py` template — `_resolve_prompt` now runs `str.format()` on BOTH the default template AND any user-supplied custom prompt, so `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` placeholders are substituted either way. See the Template safety note under `Member Create` in `.claude/skills/cafleet/SKILL.md` — any literal `{` / `}` in a custom prompt must be doubled (`{{` / `}}`), because the prompt is passed through `.format()` even when it contains no placeholders. Pre-substituting the dynamic placeholder values in shell is separate and does NOT remove the need to escape literal braces.

```
You are the Drafter in a design document creation team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/drafter.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>     (will be filled in literally by member create)
OUTPUT PATH: [INSERT ${DOC_PATH}]

The user's request: [INSERT USER'S ORIGINAL REQUEST]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.
Send your questions to the Director who will relay them to the user.
Start by reading the target codebase for context, then send your clarifying questions.
Do NOT create any design document file until you have received answers.
```

**Drafter spawn prompt (resume mode):**

Use this instead when Step 0 detected resume mode:

```
You are the Drafter in a design document creation team (CAFleet-native, RESUME MODE).

<ROLE DEFINITION>
[Content of roles/drafter.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>
DESIGN DOCUMENT: [INSERT ${DOC_PATH}]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

This is a RESUME session. The document contains COMMENT markers from a previous
interview. Follow the Resume Mode instructions in your role definition.
Do NOT ask clarifying questions — the COMMENTs contain the needed information.
Start by reading the design document.
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Drafter" \
  --description "Writes and revises the design document" \
  -- "<Drafter spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<drafter-agent-id>` in every subsequent command.

#### 1f. Spawn the Reviewer

**Reviewer spawn prompt:**

```
You are the Reviewer in a design document creation team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/reviewer.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director
- Skill(design-doc) — for template and guidelines

SESSION ID: <session-id>
DIRECTOR AGENT ID: <director-agent-id>
YOUR AGENT ID: <my-agent-id>

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id <session-id> send --agent-id <my-agent-id> --to <director-agent-id> --text "your report"
- When you see cafleet poll output with a message from the Director, act on those instructions.

Wait for the Director to assign a document for review. Read the document file and
provide specific, actionable feedback. If the draft meets all quality standards,
signal: "APPROVED - Ready for user review."
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Reviewer" \
  --description "Critically reviews drafts for rule compliance and quality" \
  -- "<Reviewer spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<reviewer-agent-id>` in every subsequent command.

#### 1g. Verify members are live

```bash
cafleet --session-id <session-id> member list --agent-id <director-agent-id>
```

Both members must show `status: active` with a non-null `pane_id`. If either is missing or pending, retry the spawn before proceeding.

### Step 2: Clarification Phase (Director)

**Skip this step entirely when `SKIP_CLARIFICATION=true`** (set by Step 0 in resume mode or quality-review-only mode). Resume mode: the COMMENT markers serve as the clarification and the Drafter already has all the information needed. Quality-review-only mode: the Drafter is not producing a new draft at all — proceed directly to Step 3 by routing the existing `${DOC_PATH}` to the Reviewer.

1. Wait for the Drafter's clarifying questions. The monitoring `/loop` and periodic `cafleet --session-id <session-id> poll --agent-id <director-agent-id>` will surface the Drafter's message once it arrives.
2. `cafleet --session-id <session-id> ack --agent-id <director-agent-id> --task-id <task-id>` each received message after reading it.
3. Relay the questions to the user via `AskUserQuestion`. If the number of questions exceeds the per-call limit of `AskUserQuestion`, split them into multiple sequential calls to relay all questions without omission.
4. Relay the user's answers back to the Drafter:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "User answers: ..."
   ```
5. **Gate check**: If the Drafter produces a draft without prior questions, reject it and instruct them to ask first:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "Stop — you must send clarifying questions before drafting. Discard the draft and send questions first."
   ```
   A focused confirmation round counts as valid clarification.

### Step 3: Internal Quality Loop (Director)

Enter this step after the Drafter reports a completed draft, **or immediately** when `QUALITY_REVIEW_ONLY=true` (the existing `${DOC_PATH}` is treated as the "completed draft" — no waiting for a Drafter report):

1. **Route to Reviewer** with the document path:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <reviewer-agent-id> --text "Please review the draft at ${DOC_PATH}. Provide feedback or signal APPROVED."
   ```
2. **Wait** for the Reviewer's feedback via `cafleet --session-id <session-id> poll --agent-id <director-agent-id>`.
3. **On feedback**: Route to Drafter for revision:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "Reviewer feedback: ... Please address and reply when done."
   ```
4. Wait for the Drafter's revision report, then loop back to step 1 (re-route to Reviewer).
5. Repeat until the Reviewer explicitly signals `APPROVED - Ready for user review.`
6. **Iteration limit**: Aim for 2–3 rounds. If not converging, escalate to the user: summarize the remaining issues at a high level and use `AskUserQuestion` to ask whether to continue iterating or abort. Do not proceed to Step 4 until the Reviewer has approved.

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
  2. **If markers are found**: Route COMMENT content and fix instructions to the Drafter via `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <drafter-agent-id> --text "..."`. After the Drafter revises and removes markers, verify with Grep that no `COMMENT(` markers remain. Then re-enter the quality loop (Step 3) and re-present (Step 4).
  3. **If no markers are found**: Explain the COMMENT marker convention to the user — markers follow the pattern `# COMMENT(username): feedback` placed directly in the design document file. Show the file path so the user can edit it. Then re-prompt with the same three-option pattern (Approve / Scan for COMMENT markers / Other).

- **"Other" (free text)**: Use LLM reasoning — not keyword matching — to distinguish between:
  - **Abort intent** (user wants to stop or cancel the process): Trigger the Abort Flow — follow the Shutdown Protocol (Step 6) without Drafter finalization.
  - **Non-abort intent** (user providing verbal feedback or asking a question): Explain that feedback should be provided via COMMENT markers in the design document, then re-prompt with the same three-option pattern.

No round limit — loop continues until approved or aborted.

### Step 6: Finalize & Clean Up (Director)

1. Instruct the Drafter to finalize:
   ```bash
   cafleet --session-id <session-id> send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "User approved. Please finalize: set Status to Approved, refresh Last Updated, bump the Progress header field if present in the template, verify implementation steps are actionable, then report done."
   ```
   Wait for the Drafter's confirmation.

2. Cancel the `/loop` monitor (`CronDelete` on the cron ID returned when the loop was created).

3. Shut down members:
   ```bash
   cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <drafter-agent-id>
   cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <reviewer-agent-id>
   ```

   Each `member delete` now blocks until the pane is actually gone (15 s default timeout). On exit 2 (stuck prompt), inspect with `cafleet member capture` and answer with `cafleet member send-input`, then retry — or rerun with `--force` to skip `/exit` and kill-pane immediately.

4. Tear down the session (this also deregisters the root Director and the Administrator — `cafleet deregister --agent-id <director-agent-id>` is rejected with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.`):
   ```bash
   cafleet session delete <session-id>
   # → Deleted session <session-id>. Deregistered N agents.
   ```

`session delete` soft-deletes the `sessions` row and physically deletes every associated `agent_placements` row while preserving all `tasks` rows for audit — the message history remains inspectable in the admin WebUI (subject to the WebUI's soft-delete filtering behavior).
