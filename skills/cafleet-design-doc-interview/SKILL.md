---
name: cafleet-design-doc-interview
description: "Validate an existing design document through fine-grained multi-round Q&A using CAFleet-native orchestration. Spawns a short-lived Analyzer member that reads the document and returns a numbered question list; the Director then drives AskUserQuestion rounds and writes COMMENT(claude) annotations inline. Supports multi-session splitting via question.md progress tracking. Use after the cafleet-design-doc-create skill and before the cafleet-design-doc-execute skill. Takes document path as argument. Do NOT use this to create or execute design documents — use the dedicated skills instead."
allowed-tools: Read, Write, Edit, Glob, Grep, AskUserQuestion, Bash
---

# Design Doc Interview (CAFleet Edition)

Validate an existing design document through structured, fine-grained Q&A across multiple sessions. The Director (main Claude) drives the conversation and writes annotations; an Analyzer member spawned via `cafleet member create` reads the document and returns the question list, then is torn down before the interview rounds begin. Discrepancies surface as inline `COMMENT(claude)` annotations in the design document. Multi-session splitting via `question.md` prevents context compaction for large interviews.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director (Interviewer)** | Main Claude | Resolve doc path, parse `question.md` progress, spawn Analyzer, drive `AskUserQuestion` rounds, write answers + COMMENT annotations + progress marker | Read the document for question generation (delegated to Analyzer); conduct the Q&A rounds outside `AskUserQuestion` | (inline in this SKILL.md) |
| **Analyzer** | CAFleet member spawned via `cafleet member create` | Read the design doc, return a flat numbered question list covering uncovered sections, then idle pending shutdown | Talk to the user; edit any file; persist state across spawns | [roles/analyzer.md](roles/analyzer.md) |

## Additional resources

- For the document template, see: [../cafleet-design-doc/template.md](../cafleet-design-doc/template.md)
- For section guidelines and quality standards, see: [../cafleet-design-doc/guidelines.md](../cafleet-design-doc/guidelines.md)
- Output of the `cafleet-design-doc-create` skill is the input to this skill; this skill's `COMMENT(claude)` markers are consumed by the `cafleet-design-doc-create` skill's resume mode.

## Coordination Protocol

This skill writes only `COMMENT(claude)` markers in the design document, and the Director-Analyzer cafleet messages are exempt from the verb + pointer + `COMMENT(role)` schema used by the `cafleet-design-doc-create` and `cafleet-design-doc-execute` skills. The Analyzer's question list is a one-time payload deliverable (a numbered list, not iterative coordination), and the Director's user-facing relay goes through `AskUserQuestion`, not cafleet. Only the inline `COMMENT(claude)` annotation rules below are in scope.

> **Maintainer source-of-truth.** The `COMMENT(role)` marker convention is mirrored from `skills/cafleet-design-doc/coordination.md` (the canonical reference; updates to one require updates to the other). The convention is fully inlined below so this skill stands alone when packaged as an independent plugin — no cross-skill markdown link is added, by design.

### COMMENT(claude) Marker

Inline marker placed in the design document.

```
COMMENT(claude): <description of discrepancy and what needs to change>
```

Placed on its own line, immediately before the content it refers to — e.g. a `COMMENT(claude): User confirmed the retry limit should be 5, not 3. ...` line inserted directly above the `### Retry Strategy` section it refers to.

Roles relevant in this skill:

| Role | Who writes it | When |
|:--|:--|:--|
| `claude` | The Director acting as user-mediator | Carries user-derived clarifications produced by the interview. The marker is placed inline immediately before the relevant content; one marker per discrepancy. |

Rules:

- One marker per logical discrepancy. Do not bundle unrelated issues.
- Body must be actionable — state what is wrong AND what the correct behavior should be.
- Markers persist in the document until resolved by the `cafleet-design-doc-create` skill's resume mode, which reads each `COMMENT(claude)` marker, applies the fix, and removes the marker as part of the fix.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` (no separate `cafleet agent register` call) — and spawns one short-lived Analyzer via `cafleet member create`. The Analyzer is torn down BEFORE the interview rounds begin; the Director then runs the rounds (and writes annotations) on its own. All Analyzer coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet fleet create, cafleet member create, drives Q&A, writes annotations)
      +-- Analyzer (member agent -- spawned in tmux pane; returns question list; terminated)
```

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

## Process

### Step 0: Path Resolution & Doc Validation (Director)

1. Load the `cafleet-base-dir` skill for the no-bypass write protocol and `<unset>` sentinel contract. Then canonicalize `$ARGUMENTS` and resolve the task-scoped BASE:

   - **Relative input** — accept any of: `0000060-foo`, `0000060-foo/design-doc.md`, `design-docs/0000060-foo`, `design-docs/0000060-foo/design-doc.md`. Canonicalize to `design-docs/<slug>` by: (1) stripping the trailing `/design-doc.md` if present; (2) stripping the leading `design-docs/` if present; (3) prepending `design-docs/`. The skill's Step 0 does NOT perform this stripping (per the `cafleet-base-dir` skill § *Consumer contract*) — canonicalize first, then run the skill's **Step 0 (task-scope resolution)** with the relpath `design-docs/<slug>`.

   - **Absolute path** (e.g. `/abs/path/to/design-docs/0000060-foo/design-doc.md`): Step 0 accepts only the task-folder path, not a child file. Strip the trailing `/design-doc.md` if present so the absolute path identifies the task folder, then run Step 0 with that absolute task-folder path. Step 0 accepts the absolute path if it lies strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

   Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder, `dir_path = ${BASE}`, and `doc_path = ${BASE}/design-doc.md` (the task folder IS the design-doc directory; no further `${BASE}/design-docs/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `dir_path` to the **canonicalized** absolute task-folder path and `doc_path = dir_path / "design-doc.md"` (unless `$ARGUMENTS` already names `design-doc.md`, in which case use it verbatim and derive `dir_path = dirname(doc_path)`), and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet-base-dir` skill § *The `<unset>` sentinel*.
2. Read the design document at `doc_path`. If missing or empty, report the error and stop.
3. Run `cafleet doctor`. If it reports a problem, surface its message and stop.

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

#### 2a. Establish a CAFleet fleet

```bash
cafleet fleet create --label "design-doc-interview-{slug}" --json
```

Capture `fleet_id` and `director.agent_id` from the JSON response. Substitute them for `<fleet-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal ids.

#### 2b. Spawn the monitoring member (first-in)

BEFORE spawning the Analyzer, load both the `cafleet-agent-team-monitoring` skill and the `cafleet-agent-team-supervision` skill (in that order) for their heartbeat, Authorization-Scope Guard, idle semantics, and Stall Response policy. Then spawn the dedicated monitoring member as the **first** `cafleet member create` in the fleet, with `--role monitor --model sonnet`. It launches `cafleet monitor start --fleet-id <fleet-id>` as a background task in its own pane, confirms with `cafleet monitor status`, and reports `ready: monitor live` to the Director. **Receipt of that handshake gates the Analyzer spawn** (2d) — do not spawn the Analyzer until it has arrived (first-in). The Director does **not** run `cafleet monitor start` itself.

Render the canonical monitoring-member spawn prompt (the **conditional** idle-nudge routine — re-engage the Director via `cafleet member nudge` only when un-acked inbox items or stalled members can be named) to a `--prompt-file` per the audit-file pattern in 2c, then spawn:

```bash
cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name "monitor" \
  --description "Monitoring member — runs the heartbeat and re-engages the idle Director" \
  --role monitor --model sonnet \
  --prompt-file ${BASE}/prompts/monitor-<UTC-compact>.md
```

See the `cafleet-agent-team-monitoring` skill § The monitoring member for the canonical spawn prompt and lifecycle. The monitoring member is stopped and deleted first in the 2f teardown (first-out).

#### 2c. Locate the Analyzer role file (path-by-reference)

Resolve the absolute path of `<this skill>/roles/analyzer.md`. The spawn prompt below references it by **absolute path**; the spawned Analyzer opens it with `Read` on its first turn. Do NOT inline the role content — cafleet `member create` fails with `tmux command failed: command too long` once the shell-quoted prompt grows past a few KB. See `skills/cafleet/reference/director.md` § *Spawn prompt size limit* for the canonical write-up.

> **Spawn-prompt audit file**: the spawn below writes the rendered prompt to `${BASE}/prompts/analyzer-<UTC-compact>.md` BEFORE invoking `cafleet member create --prompt-file <abs path>`. The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet-base-dir` skill § *No-bypass write protocol* and the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

#### 2d. Spawn the Analyzer

**Gate**: do not spawn the Analyzer until the monitoring member's `ready: monitor live` handshake (2b) has arrived.

```
You are the Analyzer in a design document interview team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/analyzer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT doc_path]
ALREADY-REVIEWED SECTIONS: [INSERT JSON array from interview-progress, or "none" on fresh start]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "your numbered question list"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

Read the design document, generate a numbered question list per the role definition,
send it to the Director via cafleet message send, then idle pending shutdown.
```

Render the prompt to `${BASE}/prompts/analyzer-<UTC-compact>.md` per the 2c audit-file pattern (leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` intact for the CLI's `str.format()` pass), then spawn with `--prompt-file`:

   ```bash
   cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
     --name "Analyzer" \
     --description "Reads the design doc and generates a numbered question list" \
     --prompt-file ${BASE}/prompts/analyzer-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<analyzer-agent-id>` in every subsequent command.

#### 2e. Wait for the Analyzer's question list

Poll `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id> --full` until the Analyzer's reply arrives. **The `--full` flag is required**: `cafleet message poll` truncates each message body to 200 codepoints + `…` by default, which would silently mangle the Analyzer's numbered question list. Acknowledge with `cafleet message ack --fleet-id <fleet-id> --agent-id <director-agent-id> --task-id <task-id>`.

The reply must be a flat numbered list following the format specified in [roles/analyzer.md](roles/analyzer.md), terminated by a `Total: N questions` line. If the Analyzer returns a malformed list, send a single corrective `cafleet message send` requesting the canonical format and wait again with `cafleet message poll --full`. After 2 corrective rounds, escalate to the user.

#### 2f. Tear down the monitoring member and the Analyzer

The Analyzer is stateless and the heavy supervision work is done once its question list arrives — keeping either alive through the Q&A rounds wastes a pane. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol* immediately after the question list is received (first-out): stop the monitoring member's `monitor start` background task and wait for confirmation; `cafleet member delete` the monitoring member first, then the Analyzer (on exit 2 use `member capture` + `send-input` recovery or `--force`); `cafleet member list` to verify the roster is empty; `cafleet fleet delete <fleet-id>`; `cafleet fleet list` to confirm.

#### 2g. Persist the question list to `question.md`

- **Fresh start** (file does not exist): write `{dir_path}/question.md` per the § *`question.md` Format* below — `interview-progress` starts as an empty array `[]`, the Questions section is a verbatim copy of the Analyzer's full numbered list, and the Answers section is initially empty.

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

1. **Annotate discrepancies**: For each discrepancy found, add a `COMMENT(claude): ...` annotation inline in the design document, immediately before the relevant content (per § *COMMENT(claude) Marker*). Use `Edit` to insert each annotation.
2. **Update progress in `question.md`**: Append the section headings reviewed in this invocation to the JSON array inside `<!-- interview-progress: [...] -->` in `question.md` (NOT in the design document).
3. **If final session** (every section in the design document is now in the progress array): remove the `<!-- interview-progress: [...] -->` line from `question.md` entirely.
4. **Verify**: Use `Grep` on the design document to confirm all intended COMMENT annotations were written.

### Step 5: Session Report (Director)

Present a summary to the user:

| Field | Content |
|:--|:--|
| **Sections reviewed** | List of section headings covered in this invocation |
| **Discrepancies found** | Count and brief list of each COMMENT added |
| **Sections remaining** | List of section headings not yet reviewed |
| **Next step** | See decision table below |

**Next-step decision:**

| State | Suggested next step |
|:--|:--|
| Sections remain (with or without COMMENT markers) | Re-invoke the `cafleet-design-doc-interview` skill with `<doc-path>` for the next session |
| All sections covered, COMMENT markers present in document | Invoke the `cafleet-design-doc-create` skill with `<doc-path>` to fix annotations (resume mode auto-detects markers and routes to the Drafter), then the `cafleet-design-doc-execute` skill |
| All sections covered, no COMMENT markers in document | Invoke the `cafleet-design-doc-execute` skill with `<doc-path>` to implement |

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
