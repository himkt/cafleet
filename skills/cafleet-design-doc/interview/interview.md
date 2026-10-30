# Design Doc Interview (CAFleet Edition)

Validate an existing design document through structured, fine-grained Q&A across multiple sessions. The Director (main agent) drives the conversation and writes annotations; an Analyzer member spawned via `cafleet member create` reads the document and returns the question list, then is torn down before the interview rounds begin. Discrepancies surface as inline `COMMENT(user-relay)` annotations in the design document. Multi-session splitting via `question.md` prevents context compaction for large interviews.

## Required reading

Read these prerequisites in order before orchestration. Your `CODING AGENT:` identity selects your executing backend; a main session uses its own identity.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../../cafleet/reference/coding-agents.md) | Resolve local Runtime bindings and bound notes before acting. |
| 2 | [Generic Director role](../../cafleet/roles/director.md) | Before setup; obey its selected-backend, spawn skeleton, audit, size and action-triggered reads. |
| 3 | [Guidelines File Layout](../reference/guidelines.md#file-layout), then [BASE](../../cafleet/reference/base-dir.md) | Before argument normalization and output-root resolution. |
| 4 | [Supervision](../../cafleet/reference/supervision.md) | Before orchestration; atomic registration, Director wake, dispatch and authorization scope. |
| 5 | [Coordination](../reference/coordination.md) | Before messages, payload retrieval or markers. |
| 6 | [Guidelines](../reference/guidelines.md) | Before document-format or phase-completion actions. |

Read [Recovery](../../cafleet/reference/supervision.md#recovery) immediately before recovery and [Shutdown](../../cafleet/reference/supervision.md#shutdown) immediately before teardown. The executing backend owns local tools; selected member backend owns spawn model/capabilities.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director (Interviewer)** | Main agent | Resolve doc path, parse `question.md` progress, spawn Analyzer, drive decision-surface Q&A rounds, write answers + COMMENT annotations + progress marker | Read the document for question generation (delegated to Analyzer); conduct the Q&A rounds off {decision_surface} | (inline in this workflow body) |
| **Analyzer** | CAFleet member spawned via `cafleet member create` | Read the design doc, return a flat numbered question list covering uncovered sections, then idle pending shutdown | Talk to the user; edit any file; persist state across spawns | [roles/analyzer.md](roles/analyzer.md) |

## Coordination Protocol

This workflow writes only `COMMENT(user-relay)` markers. Read [Payload exemptions](../reference/coordination.md#payload-exemptions) before the Director/Analyzer exchange; preserve Analyzer no-file-edit scope with tool-provided stdin for long payloads and retrieve complete JSON at the receiver. Coordination owns marker grammar and one actionable issue per marker.

Interview-specific: place each `COMMENT(user-relay)` marker on its own line immediately before the section it refers to (e.g. above `### Retry Strategy`); markers persist until the create workflow's resume mode resolves them (reads each marker, applies the fix, removes it).

## Context Management Strategy

Two mechanisms prevent context compaction:

1. **Member offloading**: The Analyzer member performs the heavy document analysis (reading, reasoning, question generation) and returns only a compact numbered question list. The Director never reads the entire design document for question generation — it only reads it for resume-mode progress detection (Step 1) and for inserting COMMENT annotations (Step 4).
2. **Multi-session splitting**: Each invocation covers a batch of sections. The Director tracks progress via `question.md` in the design document's directory, so subsequent invocations skip already-reviewed sections.

## Analyzer spawn delta

Render the Analyzer prompt from the [shared frame](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with the delta table in Step 2, the absolute installed role and skill paths and the CAFleet load purpose `for communication with the Director`. The Analyzer loads both `cafleet` and `cafleet-design-doc` and continues this assigned workflow.

## Process

### Step 0: Path Resolution & Doc Validation (Director)

1. Normalize `$ARGUMENTS` per [Guidelines File Layout](../reference/guidelines.md#file-layout) and resolve BASE with [BASE Step 0](../../cafleet/reference/base-dir.md#step-0-task-scope-resolution). A resolved BASE is the task folder: `dir_path = ${BASE}` and `doc_path = ${BASE}/design-doc.md`. For `<unset>`, use the canonicalized absolute task folder as `dir_path` (or the supplied `design-doc.md` path as `doc_path`) and guard-skip audit writes.
2. Read the design document at `doc_path`. If missing or empty, report the error and stop.
3. Confirm the Director runs inside a tmux or herdr session, then run `cafleet doctor`. If either reports a problem, surface it and stop.

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

#### 2a. Register and arm reception

Follow the [Spawn Protocol](../../cafleet/reference/supervision.md#spawn-protocol) to register the calling session with `cafleet fleet create --name "design-doc-interview-<slug>" --coding-agent <backend> --json`, then follow [Director wake](../../cafleet/reference/supervision.md#director-wake).

#### 2b. Spawn the Analyzer

Render the Analyzer prompt by path reference to the absolute `<this skill>/roles/analyzer.md` with this delta:

| Slot | Analyzer |
|---|---|
| ROLE TITLE / TEAM | `the Analyzer` / `design document interview` |
| role-file | `roles/analyzer.md` |
| EXTRA SKILL LOADS | `cafleet-design-doc` (assigned role, format and coordination references) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT doc_path]` + `ALREADY-REVIEWED SECTIONS: [INSERT JSON array from interview-progress, or "none" on fresh start]` |
| IMPORTANT / ROLE-CONSTRAINT LINES (verbatim) | `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| start cue (verbatim) | `Read the design document, generate a numbered question list per the role definition, send it to the Director via cafleet message send, then idle pending shutdown.` |

Write the rendered prompt to the audit file `${BASE}/.prompts/analyzer-<UTC-compact>.md`, then spawn with `--file`:

   ```bash
   cafleet member create --fleet-id <fleet-id> \
     --name "Analyzer" \
     --description "Reads the design doc and generates a numbered question list" \
     --file ${BASE}/.prompts/analyzer-<UTC-compact>.md \
     --json
   ```

   Parse `member_id` from the JSON response and substitute it for `<analyzer-member-id>` in every subsequent command.

#### 2c. Wait for the Analyzer's question list

Receive the Analyzer reply, then read it complete with `cafleet message poll <director-member-id> --json` and acknowledge it with `cafleet message ack <message-id>`.

The Analyzer follows the read-only stdin branch in [Payload exemptions](../reference/coordination.md#payload-exemptions); a concrete unavailable transport is routed to the Director, and no delivery is inferred. The reply must be a flat numbered list following the format specified in [roles/analyzer.md](roles/analyzer.md), terminated by a `Total: N questions` line. If the Analyzer returns a malformed list, send a single corrective `cafleet message send` requesting the canonical format and read the corrected reply the same way. After 2 corrective rounds, escalate to the user via {decision_surface} (options: retry the Analyzer once more / abort the interview / proceed with the partial list).

#### 2d. Tear down the Analyzer

After reading and ACKing its complete question list, read [Shutdown](../../cafleet/reference/supervision.md#shutdown), delete the Analyzer, verify the root-only roster and deregister the fleet. The Director performs the remaining interview itself.

#### 2e. Persist the question list to `question.md`

- **Fresh start** (file does not exist): write `{dir_path}/question.md` per the § *`question.md` Format* below — `interview-progress` starts as an empty array `[]`, the Questions section is a verbatim copy of the Analyzer's full numbered list, and the Answers section is initially empty.

- **Resume mode** (file already exists, Step 1 sent us here for a new batch): do NOT overwrite the file. Append the new questions to the end of the existing Questions list, continuing the numbering from the last existing question (e.g., if the last question is `#20`, start the new batch at `#21`). This preserves prior Answers, progress, and stable question numbers for Step 3.

### Step 3: Interview Loop (Director)

After persisting the question list (Step 2e) — or directly when `SKIP_ANALYZER=true` was set in Step 1 — the Director runs a deterministic round-counter loop.

**Pre-loop setup:**

1. Parse the numbered question list from `question.md`'s Questions section.
2. Filter out any already-answered question numbers (the Answers section's `### Round X (Questions Y-Z)` headings carry the question-number range).
3. Count remaining questions: `N`.
4. Calculate total rounds: `total_rounds = ceil(N / 4)`.
5. Log: `"Starting interview: N questions, total_rounds rounds"`.

**Loop: for `round = 1` to `total_rounds`:**

1. Take the next batch of up to 4 questions.
2. Present the batch via {decision_surface}, grouped by related topic when possible. Each question carries 2–4 options as supplied by the Analyzer. A free-text fallback is always available.
3. After the user responds:
   - Append the round's answers to `question.md`'s Answers section under a `### Round X (Questions Y-Z)` heading.
   - Record any discrepancies (target section, current text, what needs to change) for Step 4.
4. Log: `"Completed round X of total_rounds (Y of N questions asked)"`.
5. Continue to the next round.

**Mandatory completion rule (NON-NEGOTIABLE):**

> The Director MUST complete all rounds in the current invocation. Stopping before all questions are asked is FORBIDDEN. The only exception is the user explicitly providing free-form text via {decision_surface} to request early termination — in that case proceed directly to Step 4 with the answers collected so far.

**There is no "End interview" option.** The user's escape hatch is the free-text option on any question.

**Session termination:**

| Condition | Next action |
|:--|:--|
| All rounds completed (`round = total_rounds`) | Proceed to Step 4 |
| User requests early exit via the free-text option | Proceed to Step 4 with answers collected so far |

### Step 4: Annotate & Update Progress (Director)

1. **Annotate discrepancies**: For each discrepancy found, add a `COMMENT(user-relay): ...` annotation inline in the design document, immediately before the relevant content (per § Coordination Protocol). Edit the document to insert each annotation.
2. **Update progress in `question.md`**: Append the section headings reviewed in this invocation to the JSON array inside `<!-- interview-progress: [...] -->` in `question.md` (NOT in the design document).
3. **If final session** (every section in the design document is now in the progress array): remove the `<!-- interview-progress: [...] -->` line from `question.md` entirely.
4. **Verify**: Search the design document to confirm all intended COMMENT annotations were written.

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
| Sections remain (with or without COMMENT markers) | Re-run the interview workflow with `<doc-path>` for the next session |
| All sections covered, COMMENT markers present in document | Run the create workflow with `<doc-path>` to fix annotations (resume mode auto-detects markers and routes to the Drafter), then the execute workflow |
| All sections covered, no COMMENT markers in document | Run the execute workflow with `<doc-path>` to implement |

## `question.md` Format

Progress is tracked via `question.md` in the design document's directory (e.g., `design-docs/xxx/question.md`). The `<!-- interview-progress: [...] -->` HTML comment at its top holds a JSON array of the section headings already reviewed (whether clean or with issues). The marker lives in `question.md`, NOT in the design document; if `question.md` exists but the marker is absent, the interview is complete.

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
