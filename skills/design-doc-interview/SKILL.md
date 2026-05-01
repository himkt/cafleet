---
name: design-doc-interview
description: Validate an existing design document through fine-grained multi-round Q&A using CAFleet-native orchestration. Spawns a short-lived Analyzer member that reads the document and returns a numbered question list; the Director then drives AskUserQuestion rounds and writes COMMENT(claude) annotations inline. Supports multi-session splitting via question.md progress tracking. Use after /cafleet:design-doc-create and before /cafleet:design-doc-execute. Takes document path as argument. Do NOT use this to create or execute design documents — use the dedicated skills instead.
allowed-tools: Read, Write, Edit, Grep, AskUserQuestion, Bash
---

# Design Doc Interview (CAFleet Edition)

Validate an existing design document through structured, fine-grained Q&A across multiple sessions. The Director (main Claude) drives the conversation and writes annotations; an Analyzer member spawned via `cafleet member create` reads the document and returns the question list, then is torn down before the interview rounds begin. Discrepancies surface as inline `COMMENT(claude)` annotations in the design document. Multi-session splitting via `question.md` prevents context compaction for large interviews.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director (Interviewer)** | Main Claude | Resolve doc path, parse `question.md` progress, spawn Analyzer, drive `AskUserQuestion` rounds, write answers + COMMENT annotations + progress marker | Read the document for question generation (delegated to Analyzer); conduct the Q&A rounds outside `AskUserQuestion` | (inline in this SKILL.md) |
| **Analyzer** | CAFleet member spawned via `cafleet member create` | Read the design doc, return a flat numbered question list covering uncovered sections, then idle pending shutdown | Talk to the user; edit any file; persist state across spawns | [roles/analyzer.md](roles/analyzer.md) |

## Additional resources

- For the document template, see: [../design-doc/template.md](../design-doc/template.md)
- For section guidelines and quality standards, see: [../design-doc/guidelines.md](../design-doc/guidelines.md)
- Output of `/cafleet:design-doc-create` is the input to this skill; this skill's `COMMENT(claude)` markers are consumed by `/cafleet:design-doc-create` resume mode.

## Architecture

The Director is the root agent of a CAFleet session — bootstrapped automatically by `cafleet session create` (no separate `cafleet agent register` call) — and spawns one short-lived Analyzer via `cafleet member create`. The Analyzer is torn down BEFORE the interview rounds begin; the Director then runs the rounds (and writes annotations) on its own. All Analyzer coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet session create, cafleet member create, drives Q&A, writes annotations)
      +-- Analyzer (member agent -- spawned in tmux pane; returns question list; terminated)
```

- **Director ↔ User**: `AskUserQuestion` (4 questions per call, batching the Analyzer's numbered list)
- **Director ↔ Analyzer**: `cafleet message send` (assignment with doc path + already-reviewed sections; reply with numbered question list)
- Members receive messages via push notification: the broker injects `cafleet --session-id <session-id> message poll --agent-id <recipient-agent-id>` into the member's pane via `tmux send-keys`. The literal `<session-id>` and `<recipient-agent-id>` UUIDs are baked into the injected command string. `--session-id` is a global flag (placed **before** the subcommand); `--agent-id` is a per-subcommand option (placed **after** the subcommand name).

## Prerequisites

The Director MUST be running inside a tmux session (required by `cafleet member create`). Verify by running `cafleet doctor` before spawning the Analyzer — it reports the tmux session/window/pane identifiers and exits non-zero with a clear message when the environment is not ready. If `cafleet doctor` reports a problem, abort and surface its message to the user. Do NOT invoke `tmux display-message`, `printenv TMUX`, or any other raw tmux/env probe — `cafleet doctor` is the only supported environment check (see `skills/cafleet/SKILL.md` § *use cafleet primitives only*).

## Context Management Strategy

Two mechanisms prevent context compaction:

1. **Member offloading**: The Analyzer member performs the heavy document analysis (reading, reasoning, question generation) and returns only a compact numbered question list. The Director never reads the entire design document for question generation — it only reads it for resume-mode progress detection (Step 1) and for inserting COMMENT annotations (Step 4).
2. **Multi-session splitting**: Each invocation covers a batch of sections. The Director tracks progress via `question.md` in the design document's directory, so subsequent invocations skip already-reviewed sections.

## Interview Progress Tracking

Progress is tracked via `question.md` in the design document's directory (e.g., `design-docs/xxx/question.md`):

```html
<!-- interview-progress: ["Overview", "Success Criteria", "Specification/Retry Strategy"] -->
```

- The `<!-- interview-progress: [...] -->` HTML comment is at the top of `question.md` (NOT in the design document).
- Contains a JSON array of section headings that have been reviewed (whether clean or with issues).
- Created when `question.md` is first written (after the Analyzer returns questions).
- Appended to on subsequent invocations.
- Removed when all sections are covered (final invocation). If `question.md` exists but the marker is absent, the interview is considered complete.
- The Director reads this to determine resume state.

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="interview-{slug}")` | CAFleet session created via `cafleet session create` |
| `Agent(subagent_type="Explore", prompt=...)` (Analyzer) | `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name "Analyzer" --description "..." -- "<prompt>"` |
| `SendMessage(to="Analyzer")` | `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <analyzer-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from Analyzer) | `cafleet --session-id <session-id> message send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `agent-team-supervision` `/loop` | `Skill(cafleet-monitoring)` `/loop` |
| `TeamDelete` | `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <analyzer-agent-id>`, then `cafleet session delete <session-id>` |
| Auto message delivery | Push notification injects `cafleet --session-id <session-id> message poll --agent-id <recipient-agent-id>` into member's tmux pane |

## Process

### Step 0: Path Resolution & Doc Validation (Director)

1. Load `Skill(base-dir)` and follow its procedure with `$ARGUMENTS` as the argument.
   - If skipped (absolute path): set `doc_path = $ARGUMENTS`.
   - If base resolved: set `doc_path = ${BASE}/design-docs/$ARGUMENTS`. Resolve to absolute path.
   - If `doc_path` does not end with `design-doc.md`, append `/design-doc.md`.
2. Set `dir_path = dirname(doc_path)`.
3. Read the design document at `doc_path`. If missing or empty, report the error and stop.
4. Run `cafleet doctor`. If it reports a problem, surface its message and stop.

### Step 1: Progress Check (Director)

| `{dir_path}/question.md` state | Action |
|---|---|
| Does not exist | Fresh start. Proceed to Step 2 to spawn the Analyzer. Set `SKIP_ANALYZER=false`. |
| Exists, no `<!-- interview-progress: [...] -->` marker | Interview already complete. Report completion and stop. |
| Exists with marker, all sections covered | Report completion. Stop. |
| Exists with marker, unanswered questions remain in the Questions section | Set `SKIP_ANALYZER=true`. Skip Step 2 and proceed to Step 3 with the unanswered questions parsed from `question.md`. |
| Exists with marker, all current questions answered but uncovered sections remain | Proceed to Step 2 to generate a new batch of questions for the next uncovered sections. Set `SKIP_ANALYZER=false`. |

In resume mode where Step 2 IS run, parse the JSON array from the existing `interview-progress` marker and pass it to the Analyzer as the list of already-reviewed sections.

### Step 2: Spawn Analyzer & Collect Question List (Director)

**Skip this step entirely when `SKIP_ANALYZER=true`** (Step 1 found unanswered questions still in `question.md` from a prior invocation — the Director already has the question list).

#### 2a. Establish a CAFleet session

```bash
cafleet session create --label "design-doc-interview-{slug}" --json
```

Capture `session_id` and `director.agent_id` from the JSON response. Substitute them for `<session-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal UUIDs.

#### 2b. Start the monitoring `/loop`

BEFORE spawning the Analyzer, follow `Skill(cafleet-monitoring)`'s Monitoring Mandate and start a `/loop` monitor at the 1-minute interval using the literal `<session-id>` and `<director-agent-id>` UUIDs. **Record the cron job ID returned by `/loop` (and by any `CronCreate` it issues underneath) — Step 2f references this exact ID when tearing the loop down via `CronDelete`.** The loop stays active until the Analyzer is torn down at the end of this step.

#### 2c. Read the Analyzer role file

Read `skills/design-doc-interview/roles/analyzer.md` — its content will be embedded verbatim in the spawn prompt.

#### 2d. Spawn the Analyzer

```
You are the Analyzer in a design document interview team (CAFleet-native).

<ROLE DEFINITION>
[Content of roles/analyzer.md injected here verbatim]
</ROLE DEFINITION>

Load these skills at startup:
- Skill(cafleet) — for communication with the Director

SESSION ID: {session_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
DESIGN DOCUMENT: [INSERT doc_path]
ALREADY-REVIEWED SECTIONS: [INSERT JSON array from interview-progress, or "none" on fresh start]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --session-id {session_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your numbered question list"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

Read the design document, generate a numbered question list per the role definition,
send it to the Director via cafleet message send, then idle pending shutdown.
```

Spawn with:

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Analyzer" \
  --description "Reads the design doc and generates a numbered question list" \
  -- "<Analyzer spawn prompt (embedded role content)>"
```

Parse `agent_id` from the JSON response and substitute it for `<analyzer-agent-id>` in every subsequent command.

#### 2e. Wait for the Analyzer's question list

Poll `cafleet --session-id <session-id> message poll --agent-id <director-agent-id> --full` until the Analyzer's reply arrives. **The `--full` flag is required**: `cafleet message poll` truncates each message body to 10 codepoints + `...` by default, which would silently mangle the Analyzer's numbered question list. Acknowledge with `cafleet --session-id <session-id> message ack --agent-id <director-agent-id> --task-id <task-id>`.

The reply must be a flat numbered list following the format specified in [roles/analyzer.md](roles/analyzer.md), terminated by a `Total: N questions` line. If the Analyzer returns a malformed list, send a single corrective `cafleet message send` requesting the canonical format and wait again with `cafleet message poll --full`. After 2 corrective rounds, escalate to the user.

#### 2f. Tear down the Analyzer

The Analyzer is stateless — keeping it alive through the Q&A rounds wastes a pane and a monitor. Run the canonical teardown per `Skill(cafleet)` § *Shutdown Protocol* immediately after the question list is received:

1. `CronDelete` the `/loop` monitor (cron ID recorded at Step 2b).
2. `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <analyzer-agent-id>`. The call blocks until the pane is gone (15 s timeout); on exit 2, follow the `member capture` + `send-input` recovery in the canonical protocol, or rerun with `--force`.
3. `cafleet --session-id <session-id> member list --agent-id <director-agent-id>` — the team's roster MUST be empty.
4. `cafleet session delete <session-id>` (positional, no `--session-id` flag).
5. `cafleet session list` — the session MUST not appear.

#### 2g. Persist the question list to `question.md`

- **Fresh start** (file does not exist): write `{dir_path}/question.md` with:
  ```markdown
  <!-- interview-progress: [] -->

  ## Questions

  1. [Section: ...] Question text | Options: A) ... B) ...
  2. ...

  ## Answers
  ```
  The `interview-progress` starts as an empty array. The Questions section is a verbatim copy of the Analyzer's full numbered list. The Answers section is initially empty.

- **Resume mode** (file already exists, Step 1 sent us here for a new batch): do NOT overwrite the file. Append the new questions to the end of the existing Questions list, continuing the numbering from the last existing question (e.g., if the last question is `#20`, start the new batch at `#21`). This preserves prior Answers, progress, and stable question numbers for Step 3.

### Step 3: Interview Loop (Director)

After persisting the question list (Step 2g) — or directly when `SKIP_ANALYZER=true` was set in Step 1 — the Director runs a deterministic round-counter loop.

**Pre-loop setup:**

1. Parse the numbered question list from `question.md`'s Questions section.
2. Filter out any already-answered question numbers (the Answers section's `### Round X (Questions Y-Z)` headings carry the question-number range).
3. Count remaining questions: `N`.
4. Calculate total rounds: `total_rounds = ceil(N / 4)`.
5. Log: `"Starting interview: N questions, total_rounds rounds"`.

**Loop: for `round = 1` to `total_rounds`:**

1. Take the next batch of up to 4 questions.
2. Call `AskUserQuestion` with the batch, grouped by related topic when possible. Each question carries 2–4 options as supplied by the Analyzer. AskUserQuestion's built-in "Other" free-text option is always available.
3. After the user responds:
   - Append the round's answers to `question.md`'s Answers section under a `### Round X (Questions Y-Z)` heading.
   - Record any discrepancies (target section, current text, what needs to change) for Step 4.
4. Log: `"Completed round X of total_rounds (Y of N questions asked)"`.
5. Continue to the next round.

**Mandatory completion rule (NON-NEGOTIABLE):**

> The Director MUST complete all rounds in the current invocation. Stopping before all questions are asked is FORBIDDEN. The only exception is the user explicitly using `AskUserQuestion`'s built-in "Other" option to request early termination — in that case proceed directly to Step 4 with the answers collected so far.

**There is no "End interview" option.** The user's escape hatch is the built-in "Other" free-text on any question.

**Session termination:**

| Condition | Next action |
|:--|:--|
| All rounds completed (`round = total_rounds`) | Proceed to Step 4 |
| User requests early exit via "Other" | Proceed to Step 4 with answers collected so far |

### Step 4: Annotate & Update Progress (Director)

1. **Annotate discrepancies**: For each discrepancy found, add a `COMMENT(claude): ...` annotation inline in the design document, immediately before the relevant content. Use `Edit` to insert each annotation.
2. **Update progress in `question.md`**: Append the section headings reviewed in this invocation to the JSON array inside `<!-- interview-progress: [...] -->` in `question.md` (NOT in the design document).
3. **If final session** (every section in the design document is now in the progress array): remove the `<!-- interview-progress: [...] -->` line from `question.md` entirely.
4. **Verify**: Use `Grep` on the design document to confirm all intended COMMENT annotations were written.

### Step 5: Session Report (Director)

Present a session summary to the user:

| Field | Content |
|:--|:--|
| **Sections reviewed** | List of section headings covered in this invocation |
| **Discrepancies found** | Count and brief list of each COMMENT added |
| **Sections remaining** | List of section headings not yet reviewed |
| **Next step** | See decision table below |

**Next-step decision:**

| State | Suggested next step |
|:--|:--|
| Sections remain (with or without COMMENT markers) | Re-invoke `/cafleet:design-doc-interview <doc-path>` for the next session |
| All sections covered, COMMENT markers present in document | Run `/cafleet:design-doc-create <doc-path>` to fix annotations (resume mode auto-detects markers and routes to the Drafter), then `/cafleet:design-doc-execute` |
| All sections covered, no COMMENT markers in document | Run `/cafleet:design-doc-execute <doc-path>` to implement |

## COMMENT Annotation Format

Annotations are plain text inserted inline in the markdown document, immediately before the content they refer to:

```markdown
## Specification

COMMENT(claude): User confirmed the retry limit should be 5, not 3. Update the retry count and adjust the timeout calculation that depends on it.

### Retry Strategy

Maximum retries: 3 with exponential backoff...
```

Rules:

- Format: `COMMENT(claude): <description of discrepancy and what needs to change>`
- Placement: immediately before the relevant content, on its own line
- One COMMENT per discrepancy (do not combine unrelated issues)
- Description must be actionable — state what is wrong AND what the correct behavior should be

The format matches `/cafleet:design-doc-create` resume-mode expectations exactly, so a follow-up `/cafleet:design-doc-create <doc-path>` invocation auto-detects the markers and routes them to the Drafter.

## `question.md` Format

```markdown
<!-- interview-progress: ["Overview", "Specification/Retry Strategy"] -->

## Questions

1. [Section: Specification/Retry Strategy] Should the maximum retries be 3 or 5? | Options: A) 3 (current) B) 5 C) Configurable
2. [Section: Specification/Logging] Should failed requests be logged for debugging? | Options: A) Log all failures B) Log only final failure
...

## Answers

### Round 1 (Questions 1-4)

1. B) 5 — confirmed by user
2. A) Log all failures
...
```

$ARGUMENTS
