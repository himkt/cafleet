# Design Doc Interview (CAFleet Edition)

Validate an existing design document through structured, fine-grained Q&A across multiple sessions. The Director (main agent) drives the conversation and writes annotations; an Analyzer member spawned via `cafleet member create` reads the document and returns the question list, then is torn down before the interview rounds begin. Discrepancies surface as inline `COMMENT(user-relay)` annotations in the design document. Multi-session splitting via `question.md` prevents context compaction for large interviews.

## Required reading

Before any orchestration action — fleet create, spawn, or message — Read every file in the **Load-bearing** table below, in order. Each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; as main session, use your own identity.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay section [`../../cafleet/reference/coding-agent-overlays.md#<name>`](../../cafleet/reference/coding-agent-overlays.md) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — the failure modes *Resolve your overlay* closes, e.g. a literal `{bg_run}` emitted unresolved |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../cafleet/reference/base-dir.md) | the no-bypass write protocol + `<unset>` contract — you mis-root the spawn-prompt audit file and `question.md` or fall back to `/tmp` |
| 3 | the `cafleet` skill's [`reference/supervision.md`](../../cafleet/reference/supervision.md) | the governance + heartbeat (the Director-hosted monitor launch, the startup-line gate, Authorization-Scope Guard, Stall Response) — you spawn an unsupervised Analyzer |
| 4 | [`../reference/coordination.md`](../reference/coordination.md) | the `COMMENT(user-relay)` marker grammar and anchorless-status rules — your inline annotations are malformed |

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director (Interviewer)** | Main agent | Resolve doc path, parse `question.md` progress, spawn Analyzer, drive decision-surface Q&A rounds, write answers + COMMENT annotations + progress marker | Read the document for question generation (delegated to Analyzer); conduct the Q&A rounds off {decision_surface} | (inline in this workflow body) |
| **Analyzer** | CAFleet member spawned via `cafleet member create` | Read the design doc, return a flat numbered question list covering uncovered sections, then idle pending shutdown | Talk to the user; edit any file; persist state across spawns | [roles/analyzer.md](roles/analyzer.md) |

## Coordination Protocol

This skill writes only `COMMENT(user-relay)` markers in the design document; the Director-Analyzer cafleet messages are exempt from the verb + pointer schema (the Analyzer's question list is a one-time multi-line payload, and the Director's user relay goes through {decision_surface}, not cafleet). The `COMMENT(role)` marker format, the `user-relay` role (the Director as user-mediator, carrying user-derived clarifications), and the one-per-issue / actionable rules are canonical in [../reference/coordination.md](../reference/coordination.md) § *COMMENT(role) Marker*.

Interview-specific: place each `COMMENT(user-relay)` marker on its own line immediately before the section it refers to (e.g. above `### Retry Strategy`); markers persist until the create workflow's resume mode resolves them (reads each marker, applies the fix, removes it).

## Prerequisites

The Director MUST be running inside a tmux or herdr session and pass the gating `cafleet doctor` env-check before spawning the Analyzer, per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol*.

## Context Management Strategy

Two mechanisms prevent context compaction:

1. **Member offloading**: The Analyzer member performs the heavy document analysis (reading, reasoning, question generation) and returns only a compact numbered question list. The Director never reads the entire design document for question generation — it only reads it for resume-mode progress detection (Step 1) and for inserting COMMENT annotations (Step 4).
2. **Multi-session splitting**: Each invocation covers a batch of sections. The Director tracks progress via `question.md` in the design document's directory, so subsequent invocations skip already-reviewed sections.

## Process

### Step 0: Path Resolution & Doc Validation (Director)

1. Apply the no-bypass write protocol and `<unset>` sentinel contract from the `cafleet` skill's `reference/base-dir.md` (§ Required reading above). Then canonicalize `$ARGUMENTS` and resolve the task-scoped BASE:

   Canonicalize `$ARGUMENTS` per the `cafleet` skill's `reference/base-dir.md` § *Consumer contract* row for this skill (relative forms get `design-docs/` prepended and a trailing `/design-doc.md` stripped; absolute paths are used verbatim after the filename strip), then run its **Step 0 (task-scope resolution)** with the result.

   Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder, `dir_path = ${BASE}`, and `doc_path = ${BASE}/design-doc.md` (the task folder IS the design-doc directory; no further `${BASE}/design-docs/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `dir_path` to the **canonicalized** absolute task-folder path and `doc_path = dir_path / "design-doc.md"` (unless `$ARGUMENTS` already names `design-doc.md`, in which case use it verbatim and derive `dir_path = dirname(doc_path)`), and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet` skill's `reference/base-dir.md` § *The `<unset>` sentinel*.
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
cafleet fleet create --name "design-doc-interview-{slug}" --coding-agent <backend> --json
```

Capture `fleet_id` and `director.member_id` from the JSON response and substitute them for `<fleet-id>` and `<director-member-id>` in every subsequent command, per the `cafleet` skill's `reference/supervision.md` § *Spawn Protocol* → *Fleet bootstrap*.

#### 2b. Launch the monitor loop (before the Analyzer)

BEFORE spawning the Analyzer, apply the `cafleet` skill's `reference/supervision.md` policy (§ Required reading above): heartbeat, Authorization-Scope Guard, idle semantics, Stall Response. Launch the monitor heartbeat per its § *Spawn Protocol*. **The startup-line confirmation gates the Analyzer spawn** (2d) — do not spawn the Analyzer until it has arrived. The background task is stopped first in the 2f teardown.

#### 2c. Locate the Analyzer role file (path-by-reference)

Resolve the absolute path of `<this skill>/roles/analyzer.md`. The spawn prompt below references it by **absolute path**; the spawned Analyzer opens it with `Read` on its first turn. Do NOT inline the role content — `cafleet member create` fails with `tmux command failed: command too long` once the shell-quoted prompt grows past a few KB. See `skills/cafleet/reference/director.md` § *Spawn prompt size limit* for the canonical write-up.

> **Spawn-prompt audit file**: the spawn below renders the prompt to `${BASE}/.prompts/analyzer-<UTC-compact>.md` and spawns from that file, per the `cafleet` skill's `reference/base-dir.md` § *No-bypass write protocol*.

#### 2d. Spawn the Analyzer

**Gate**: do not spawn the Analyzer until the monitor loop's startup line (2b) has been confirmed.

Render the canonical [spawn-prompt skeleton](../../cafleet/reference/director.md#canonical-spawn-prompt-skeleton) with the Analyzer delta below (two-stage rendering + brace rules at the skeleton):

| Slot | Analyzer |
|---|---|
| ROLE TITLE / TEAM | `the Analyzer` / `design document interview` |
| role-file | `roles/analyzer.md` |
| EXTRA SKILL LOADS | none (the `cafleet` skill only) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT doc_path]` + `ALREADY-REVIEWED SECTIONS: [INSERT JSON array from interview-progress, or "none" on fresh start]` |
| start cue (verbatim) | `Read the design document, generate a numbered question list per the role definition, send it to the Director via cafleet message send, then idle pending shutdown.` |

Render the prompt to `${BASE}/.prompts/analyzer-<UTC-compact>.md` per the 2c audit-file pattern (the four identity placeholders are rendered by the CLI at spawn), then spawn with `--file`:

   ```bash
   cafleet member create --fleet-id <fleet-id> \
     --name "Analyzer" \
     --description "Reads the design doc and generates a numbered question list" \
     --file ${BASE}/.prompts/analyzer-<UTC-compact>.md \
     --json
   ```

   Parse `member_id` from the JSON response and substitute it for `<analyzer-member-id>` in every subsequent command.

#### 2e. Wait for the Analyzer's question list

Poll `cafleet message poll <director-member-id> --json` until the Analyzer's reply arrives. **The `--json` flag is required**: text-mode `cafleet message poll` truncates each message body to 200 codepoints + `…`, which would silently mangle the Analyzer's numbered question list — `--json` carries the complete body. Acknowledge with `cafleet message ack <message-id>`.

The reply must be a flat numbered list following the format specified in [roles/analyzer.md](roles/analyzer.md), terminated by a `Total: N questions` line. If the Analyzer returns a malformed list, send a single corrective `cafleet message send` requesting the canonical format and wait again with `cafleet message poll <director-member-id> --json`. After 2 corrective rounds, escalate to the user via {decision_surface} (options: retry the Analyzer once more / abort the interview / proceed with the partial list).

#### 2f. Tear down the monitor loop and the Analyzer

The Analyzer is stateless and the heavy supervision work is done once its question list arrives — keeping it alive through the Q&A rounds wastes a pane. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol*. Workflow delta: teardown fires immediately after the question list is acked; delete the Analyzer.

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

1. **Annotate discrepancies**: For each discrepancy found, add a `COMMENT(user-relay): ...` annotation inline in the design document, immediately before the relevant content (per § *COMMENT(user-relay) Marker*). Use `Edit` to insert each annotation.
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
