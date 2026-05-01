# Port `design-doc-interview` Skill to CAFleet

**Status**: Approved
**Progress**: 8/14 tasks complete
**Last Updated**: 2026-05-01

## Overview

Port the global `design-doc-interview` skill into this repository as `skills/design-doc-interview/` so it becomes the cafleet-namespaced `/cafleet:design-doc-interview` slash command, completing the cafleet design-doc trio (`create` / `interview` / `execute`). Adapt the orchestration from the global skill's in-process `Agent` subagent pattern to the CAFleet-native pattern (Director + member spawned via `cafleet member create`, message broker for coordination) used by the existing cafleet design-doc skills.

## Success Criteria

- [ ] `skills/design-doc-interview/SKILL.md` exists and is loaded automatically as `cafleet:design-doc-interview` (visible in the system-reminder skill list when the cafleet plugin is active)
- [ ] Invoking `/cafleet:design-doc-interview <doc-path-or-slug>` on a fresh design document drives a multi-round Q&A and writes a `question.md` file alongside the doc
- [ ] Discrepancies surface as `COMMENT(claude): ...` annotations inline in the design document, identical in shape to the global skill's output (so `cafleet:design-doc-create` resume mode keeps working unchanged)
- [ ] Resume invocation on a doc with an existing `question.md` skips already-reviewed sections via the `<!-- interview-progress: [...] -->` marker
- [ ] All inter-agent communication is auditable in the admin WebUI message timeline (no hidden in-process Agent subagent calls)
- [ ] CLAUDE.md in both `/` and `.claude/` lists the new skill in the project skills section

---

## Background

### What the global skill does

The global `~/.claude/skills/design-doc-interview/SKILL.md` validates an existing design document through structured Q&A:

1. **Analyzer** (a one-shot `Agent` subagent of `Explore` type) reads the document and returns a flat numbered list of fine-grained questions (intent alignment, ambiguity, missing requirements, implicit assumptions, design decisions, internal consistency, implementation actionability), with 2–4 answer options each, up to ~100 questions total across multiple invocations.
2. **Interviewer** (main Claude) batches the questions 4-at-a-time into `AskUserQuestion`, records answers under `### Round X` headings in `{dir}/question.md`, and tracks progress via a `<!-- interview-progress: [...] -->` HTML comment.
3. After all rounds, discrepancies become inline `COMMENT(claude): ...` annotations in the design document (immediately above the affected section), and the progress marker is updated (or removed if all sections are now covered).

### Why CAFleet currently lacks it

`design-doc-interview` is the only member of the design-doc family that has not been ported. The cafleet variants of `design-doc-create` and `design-doc-execute` already exist as `skills/design-doc-{create,execute}/`. The interview step sits between the two — `cafleet:design-doc-create` even has a built-in resume mode that ingests the COMMENT markers this skill produces — so the gap forces users to fall back to the global skill, which uses in-process `Agent` subagents that are invisible to the CAFleet message timeline.

### Other global-only skills considered

The system-reminder skill list at session start enumerates every available skill. Cross-referencing that list against `git ls-files skills/`:

| Global skill | Already in cafleet? | Recommend porting? | Reason |
|---|---|---|---|
| `design-doc-interview` | No | **Yes (this design doc)** | Completes the design-doc trio; produces COMMENT markers consumed by `cafleet:design-doc-create` resume mode |
| `design-doc` | Yes (`cafleet:design-doc`) | — | Already ported |
| `design-doc-create` | Yes (`cafleet:design-doc-create`) | — | Already ported |
| `design-doc-execute` | Yes (`cafleet:design-doc-execute`) | — | Already ported |
| `agent-team-supervision` | Yes (`cafleet:cafleet`) | — | CAFleet-native equivalent |
| `agent-team-monitoring` | Yes (`cafleet:cafleet-monitoring`) | — | CAFleet-native equivalent |
| `base-dir` | No | **No (out of scope here)** | Pure path-resolution helper with no CAFleet-specific behavior; the global version is invoked from `cafleet:design-doc-create` and works as-is. Re-evaluate only if the cafleet plugin should become standalone-installable without the global skills present |
| `loop`, `schedule`, `update-config`, `keybindings-help`, `simplify`, `fewer-permission-prompts` | No | No | Harness-level utilities, not project-specific |
| `claude-api`, `github-cli`, `nixos-boot-troubleshoot`, `pathfinder-explain`, `bilingual-explain`, `english-review`, `create-figure`, `my-slidev`, `research-report`, `research-presentation` | No | No | General-purpose utility skills with no cafleet coupling; staying global is correct |
| `init`, `review`, `security-review` | No (project-level slash commands only) | No | Already accessible per-project |

Conclusion: only `design-doc-interview` warrants porting in this design doc. `base-dir` is noted as a future consideration but is not in scope.

---

## Specification

### Roles

The cafleet-native interview maps the global skill's Interviewer/Analyzer split onto the Director/member pattern used by the other cafleet design-doc skills.

| Role | Identity | Does | Does NOT |
|---|---|---|---|
| **Director (Interviewer)** | Main Claude | Resolve doc path, parse `question.md` progress, spawn Analyzer, drive `AskUserQuestion` rounds, write answers + COMMENT annotations + progress marker | Read the document for question generation (delegated to Analyzer); communicate with the user except via `AskUserQuestion` |
| **Analyzer** | CAFleet member spawned via `cafleet member create` | Read the design doc, return a flat numbered question list covering uncovered sections, then idle pending shutdown | Talk to the user; edit any file; persist state across spawns |

The Analyzer is a short-lived member: spawned once per skill invocation (or per question-generation batch in resume mode when a new batch is needed), produces its question list via `cafleet message send` to the Director, and is torn down before the interview rounds begin.

### Architecture

```
User
 +-- Director (main Claude — runs cafleet session create, orchestrates the interview)
      +-- Analyzer (member agent — reads the design doc, returns question list, terminates)
```

- **Director ↔ User**: `AskUserQuestion` (4 questions per call, mirroring the global skill)
- **Director ↔ Analyzer**: `cafleet message send` (assignment with doc path + already-reviewed sections; reply with numbered question list)
- Analyzer push-notification: the broker injects `cafleet --session-id <s> message poll --agent-id <m>` into the Analyzer's pane via `tmux send-keys` whenever the Director sends a message — same primitive the other cafleet skills rely on.

### Inputs and outputs

| Aspect | Detail |
|---|---|
| **Input** | Design document path or slug (via `$ARGUMENTS`), resolved through `Skill(base-dir)` |
| **Created** | `{dir_path}/question.md` (Questions section + Answers section + `<!-- interview-progress: [...] -->` HTML comment marker) |
| **Edited** | Inline `# COMMENT(claude): ...` annotations in the design document, immediately before the affected section |
| **Allowed tools (Director)** | `Read`, `Write`, `Edit`, `Grep`, `AskUserQuestion`, `Bash` (for `cafleet` CLI) |
| **Allowed tools (Analyzer)** | `Read`, `Bash` (for `cafleet message send`/`poll`/`ack`) |

### Process

#### Step 0: Path resolution & doc validation (Director)

1. Load `Skill(base-dir)` with `$ARGUMENTS`. If `$ARGUMENTS` is an absolute path, set `doc_path = $ARGUMENTS`. Otherwise set `doc_path = ${BASE}/design-docs/$ARGUMENTS`.
2. If `doc_path` does not end with `design-doc.md`, append `/design-doc.md`.
3. Set `dir_path = dirname(doc_path)`.
4. Read the document at `doc_path`. If it is missing or empty, report the error and stop. If `cafleet doctor` reports a problem (no tmux), report and stop.

#### Step 1: Progress check (Director)

| `{dir_path}/question.md` state | Action |
|---|---|
| Does not exist | Fresh start. Proceed to Step 2 to spawn the Analyzer. |
| Exists, no `<!-- interview-progress: [...] -->` marker | Interview already complete. Report completion and stop. |
| Exists with marker, all sections covered | Report completion. Stop. |
| Exists with marker, unanswered questions remain in Questions section | Skip Step 2. Proceed to Step 3 with those remaining questions. |
| Exists with marker, all current questions answered but uncovered sections remain | Proceed to Step 2 to generate a new batch of questions for the next uncovered sections. |

#### Step 2: Spawn Analyzer & collect question list (Director)

1. Run `cafleet session create --label "design-doc-interview-{slug}" --json`. Capture `session_id` and root Director `agent_id`.
2. Start the `Skill(cafleet-monitoring)` `/loop` at the 1-minute interval with literal UUIDs substituted.
3. Read `skills/design-doc-interview/roles/analyzer.md` and embed it verbatim in the spawn prompt.
4. Run:
   ```bash
   cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
     --name "Analyzer" \
     --description "Reads the design doc and generates a numbered question list" \
     -- "<Analyzer spawn prompt>"
   ```
   Spawn prompt fields:
   - SESSION ID, DIRECTOR AGENT ID, YOUR AGENT ID (substituted by `member create`)
   - DOC PATH = `doc_path`
   - ALREADY-REVIEWED SECTIONS = the JSON array parsed from the existing `interview-progress` marker (or `none`)
   - Instruction to send the question list back via `cafleet message send` and then idle.
5. Wait for the Analyzer's `cafleet message send` containing the numbered list. Acknowledge with `cafleet message ack`.
6. **Tear down the Analyzer immediately** after receiving the list:
   - `CronDelete` the monitoring loop.
   - `cafleet member delete --member-id <analyzer-id>`.
   - `cafleet session delete <session-id>`.

   Rationale: the Analyzer is stateless and only needed to produce the list. Keeping it alive through the Q&A rounds wastes a pane and a monitor.

7. Write or update `{dir_path}/question.md`:
   - **Fresh start**: create the file with `<!-- interview-progress: [] -->`, a `## Questions` section containing the verbatim numbered list, and an empty `## Answers` section.
   - **Resume mode** (Step 1.f): append the new questions to the existing Questions section, continuing the numbering from the last question (e.g., last question was `#20` → new batch starts at `#21`). Do NOT overwrite prior Answers or progress.

#### Step 3: Interview loop (Director)

1. Parse the numbered question list from `question.md`'s Questions section. Filter out any already-answered question numbers (Answers section round headings carry the question-number range).
2. Set `N = len(remaining_questions)`, `total_rounds = ceil(N / 4)`.
3. For `round = 1` to `total_rounds`:
   - Take the next batch of up to 4 questions.
   - Call `AskUserQuestion` with the batch. Each question carries 2–4 options pre-supplied by the Analyzer.
   - Append answers to `question.md`'s Answers section under `### Round X (Questions Y-Z)`.
   - Record discrepancies (target section, current text, what needs to change) for Step 4.
4. **Mandatory completion rule**: complete every round in the current invocation. Stopping early is forbidden. The only exception is the user invoking `AskUserQuestion`'s built-in "Other" free-text option to request early termination — in that case proceed to Step 4 with the answers collected so far.

#### Step 4: Annotate & update progress (Director)

1. For each discrepancy, insert `COMMENT(claude): <description>` inline in the design document on its own line, immediately before the affected content. Use `Edit`.
2. Append the section headings reviewed in this invocation to the JSON array in `<!-- interview-progress: [...] -->` inside `question.md` (NOT the design document).
3. **If this was the final session** (every section in the design document is now in the progress array), remove the `<!-- interview-progress: [...] -->` line from `question.md` entirely.
4. Verify with `Grep` that all intended COMMENT annotations were written.

#### Step 5: Session report (Director)

Print a summary table to the user:

| Field | Content |
|---|---|
| Sections reviewed | Headings covered in this invocation |
| Discrepancies found | Count and brief list of each COMMENT added |
| Sections remaining | Headings not yet reviewed |
| Next step | See decision table below |

| State | Suggested next step |
|---|---|
| Sections remain (with or without COMMENT markers) | Re-invoke `/cafleet:design-doc-interview <doc-path>` for the next session |
| All sections covered, COMMENT markers present | Run `/cafleet:design-doc-create <doc-path>` (resume mode auto-detects markers and routes to the Drafter) |
| All sections covered, no COMMENT markers | Run `/cafleet:design-doc-execute <doc-path>` |

### COMMENT annotation format

Identical to the global skill so `cafleet:design-doc-create` resume mode keeps working without changes:

```markdown
## Specification

COMMENT(claude): User confirmed the retry limit should be 5, not 3. Update the retry count and the dependent timeout calculation.

### Retry Strategy

Maximum retries: 3 with exponential backoff...
```

Rules:
- Format: `COMMENT(claude): <description>` — actionable, states what is wrong AND what the correct behavior should be.
- Placement: on its own line, immediately before the affected content.
- One COMMENT per discrepancy.

### `question.md` format

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

### File layout

```
skills/design-doc-interview/
├── SKILL.md           # Skill description, allowed tools, full Process steps
└── roles/
    └── analyzer.md    # Analyzer role definition (embedded verbatim in spawn prompt)
```

The Director role is described inline in `SKILL.md` rather than a separate `roles/director.md`, mirroring the global skill which has no separate Interviewer role file. (`cafleet:design-doc-create` and `-execute` use `roles/director.md` because their Director orchestrates multiple long-lived members; the interview Director only orchestrates one short-lived Analyzer, so the inline form is sufficient.)

### CAFleet-specific differences from the global skill

| Aspect | Global skill | This port |
|---|---|---|
| Analyzer invocation | `Agent(subagent_type="Explore", prompt=...)` returning question list as the agent's final message | `cafleet member create` + wait for `cafleet message send` reply |
| Analyzer lifecycle | One-shot, in-process, no observability | Member spawned in tmux pane; messages persisted in SQLite; visible in admin WebUI |
| Required setup | None beyond the Agent tool | `cafleet doctor` (tmux check), `cafleet session create`, `Skill(cafleet-monitoring)` `/loop` |
| Teardown | Implicit (subagent returns) | Explicit per `Skill(cafleet)` Shutdown Protocol: `CronDelete` → `member delete` → `session delete` |
| Allowed tools | `Read, Write, Edit, Glob, Grep, AskUserQuestion, Agent` | `Read, Write, Edit, Grep, AskUserQuestion, Bash` (Bash for `cafleet` CLI; no `Agent` tool needed) |
| Coordination cost | Free | One short-lived tmux pane + one cron-driven monitor per invocation |

The user-visible Q&A flow (4-questions-per-`AskUserQuestion`, round counter, mandatory completion, COMMENT annotation format, `question.md` schema, next-step decision table) is identical to the global skill. Only the Analyzer plumbing changes.

### Open question (deferred)

`base-dir` is currently used by `cafleet:design-doc-create` (it is loaded as `Skill(base-dir)` from the global skill set). The interview port will do the same. Whether to fork `base-dir` into the cafleet plugin so the cafleet skills become standalone-installable is out of scope for this design doc — file a follow-up if it becomes an actual problem (e.g., the cafleet plugin is distributed to users without the global skill set).

---

## Implementation

### Step 1: Documentation prerequisites

- [x] Update `CLAUDE.md` (root) — add `/cafleet:design-doc-interview` to the Project Skills list with a one-line description. <!-- completed: 2026-05-01T12:30 -->
- [x] Update `.claude/CLAUDE.md` — same addition, mirrored. <!-- completed: 2026-05-01T12:30 -->
- [x] Update `cafleet:design-doc-create` SKILL.md if it currently mentions interview as global-only — confirm it now references the cafleet variant. <!-- completed: 2026-05-01T12:30 -->

### Step 2: Skill scaffolding

- [x] Create directory `skills/design-doc-interview/`. <!-- completed: 2026-05-01T12:37 -->
- [x] Create `skills/design-doc-interview/SKILL.md` with the full Process section from this design doc, the YAML front-matter (`name: design-doc-interview`, `description: ...`, `allowed-tools: Read, Write, Edit, Grep, AskUserQuestion, Bash`), and the `$ARGUMENTS` footer. <!-- completed: 2026-05-01T12:37 -->
- [x] Create `skills/design-doc-interview/roles/analyzer.md` with the role definition (read doc, return numbered list, idle pending shutdown). <!-- completed: 2026-05-01T12:37 -->

### Step 3: Analyzer prompt template

- [x] Lock the Analyzer's question-generation prompt (categories, priority order, output format spec, `Total: N questions` footer) verbatim from the global skill — the categories and format must be byte-identical so the question list parses the same way. <!-- completed: 2026-05-01T12:43 -->
- [x] Verify any literal `{` or `}` characters in the spawn prompt are doubled per the `cafleet member create` template-safety rule. <!-- completed: 2026-05-01T12:43 -->

### Step 4: Director-side helpers

- [ ] Document in SKILL.md the exact `cafleet` invocations for spawn / poll / ack / member delete / session delete with literal placeholder UUIDs. <!-- completed: -->
- [ ] Document the `question.md` schema (interview-progress marker placement, Questions and Answers sections, round heading format) so the same parser logic the global skill uses works unchanged. <!-- completed: -->

### Step 5: Resume-mode parity

- [ ] Verify the COMMENT annotation format matches what `cafleet:design-doc-create` resume mode greps for (`COMMENT(`). The format is already specified in this design doc; no change to `cafleet:design-doc-create` should be needed. <!-- completed: -->

### Step 6: Smoke test

- [ ] Pick one existing design doc (e.g., `design-docs/0000044-pytest-functional-style/design-doc.md`). Invoke `/cafleet:design-doc-interview 0000044-pytest-functional-style` from a tmux session. <!-- completed: -->
- [ ] Confirm: Analyzer pane appears, returns a question list, is torn down; `question.md` is created with the progress marker; at least one round of `AskUserQuestion` runs end-to-end; a `COMMENT(claude):` annotation is written; the progress marker is updated. <!-- completed: -->
- [ ] Confirm resume invocation on the same doc skips already-reviewed sections. <!-- completed: -->

### Step 7: Commit

- [ ] Stage `design-docs/0000045-cafleet-design-doc-interview/design-doc.md` plus `skills/design-doc-interview/` and the CLAUDE.md updates. Commit with `feat: port design-doc-interview skill to cafleet`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-01 | Initial draft |
