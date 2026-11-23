# Skills and Schema Simplification

**Status**: Complete
**Progress**: 34/34 tasks complete
**Last Updated**: 2026-11-23

## Overview

Tighten the prose of every bundled skill under `assets/skills/` by applying the reviewed edits of the clean-docs simplification run, and simplify the database schema layer by replacing `assets/migrations/V1__baseline.sql` with `assets/schema.sql` and removing the vestigial `refinery_schema_history` ledger. The two parts share one document but have separate implementation steps and commits.

## Success Criteria

- [x] Every edit row in § Part 1: Per-file skill edits is applied, and no row from § Excluded rows is applied.
- [x] No heading text under `assets/skills/` changes, and every relative link and `#anchor` in the edited skill files resolves to an existing file and heading, both established by § Verifying the skill edits.
- [x] `assets/schema.sql` is the only embedded schema file; `assets/migrations/` no longer exists.
- [x] Neither `assets/schema.sql`, `SPEC.md` §8 nor any source file declares or reads `refinery_schema_history`; `LedgerState` and `ledger_state` no longer exist.
- [x] `initialize` on a non-empty file succeeds only when the classifier returns current, and refuses a file of another recorded version or with a damaged required shape with the incompatible-format error, leaving its bytes unchanged (asserted by new `OTHER_FILE_CASES` rows).
- [x] `git grep` finds no `migrations`, `refinery`, `V1__`, `LedgerState`, `ledger_state`, `BASELINE_SQL` or `migrated_conn` in the tracked tree outside `design-docs/`, and no schema-ledger or schema-baseline wording in `SPEC.md`, `docs/`, `.claude/rules/`, `CLAUDE.md` or `src/`.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Two maintainer-local research artifacts feed this design; their content needed for implementation is copied into this document, so neither file is needed to implement it.

| Input | Content |
|---|---|
| clean-docs simplification run, merged `findings.md` | 165 rows over every file under `assets/skills/`, each annotated by the run's reviewer: 156 APPROVE, 3 REVISE, 3 REJECT and 3 superseded inside their slice; plus per-slice Observations |
| `db-proposal.md` | Analysis of `assets/migrations/`, the ledger and the compatibility classifier in `src/db.rs` |

The schema layer predates the per-version database file model. Commit `67ea6551` replaced the `refinery` crate with a hand-written ledger that kept refinery's table name and its `V1__` file convention; commit `760d03e9` introduced per-version `cafleet_<VERSION>.db` files and the `cafleet_database` version row, which supersede the ledger. No migration runner exists or can exist under the per-version model, nothing reads the ledger's `applied_on` or `checksum`, and `Cargo.toml` has no `refinery` dependency.

---

## Specification

### Part 1: Skill prose simplification

#### Scope

- Files: every file under `assets/skills/`. Skills elsewhere (such as `.claude/skills/`) are out of scope.
- Edits: exactly the APPROVE and REVISE rows of the run, plus the three adopted Observation rows (`OBS-1`..`OBS-3`). A REVISE row uses the reviewer's revised text, which the tables already carry.
- Preserved: every heading and anchor, every contract fact (exact flags, error strings, output fields, command examples, IMPORTANT lines, spawn-skeleton items, start cues), and every `### Model catalog` and `### Role defaults` subsection in `coding-agents.md`, which the `cafleet-model-list-refresh` skill owns.
- `SPEC.md`, `docs/` and `README.md` receive no Part 1 edits, and Part 1 adds no tests: no test asserts skill text, and the embedded skill bytes follow the edited files automatically.

#### Applying the edit tables

| Table element | Meaning |
|---|---|
| Row | `S<slice>-<id>` is the run's row ID; `OBS-<n>` is an adopted Observation |
| Line | Line number in the file before any Part 1 edit; locate each edit by its Current text, because earlier edits shift line numbers |
| Current | Locates the span to edit (rules below) |
| Replacement | The new text for the whole span, or a deletion instruction naming exactly what to delete |
| `*(…)*` prefix | Scope qualifier from the run (for example "sentence 3 only"); it is an instruction, not inserted text |
| `<br>` | A line break inside the quoted text (list items, table rows, separate lines) |
| `\|` | A literal `\|` that belongs to the quoted text (most often a skill table row) |

Every row edits one span, determined in this order:

1. A `*(…)*` qualifier on Current or Replacement, when present, defines the span (for example "sentence 3", "first sentence only", "whole line").
2. Otherwise the span runs from Current's first visible fragment to its last; an interior `…` stands for the original text between two fragments, a leading `…` extends the span back to the start of its paragraph, list item or table row, and a trailing `…` extends it to that unit's end.
3. The Replacement replaces the entire span, elided text included. It keeps original text only where it carries its own `…` or a "(… unchanged)" note: a leading `…` keeps the span's original text before the Replacement's first fragment, a trailing `…` keeps the original text after its last fragment.
4. Text outside the span stays unchanged.

#### Verifying the skill edits

No task or test checks skill links (`mise //docs:build` covers only the documentation site), so Part 1 is verified with two explicit checks after every table is applied:

1. **Headings unchanged.** `git diff -U0 -- assets/skills | grep -E '^[-+]#'` prints nothing.
2. **Links resolve.** For every edited file, list each Markdown link target `](…)`. A relative path must name an existing file, resolved from the linking file's directory. Each `#anchor`, in-file or after a path, must equal the GitHub slug of a heading in its target file: lowercase the heading text, drop backticks and every punctuation character except hyphens and spaces, then replace each space with a hyphen (`` ### `member show` `` → `member-show`, `## COMMENT(role) Marker` → `commentrole-marker`).

Rows `S6-R7` and `S6-R8` appear under both `programmer.md` and `tester.md`; apply the deletion in each file, and for `S6-R8` apply the half that names that file. Where one row lists two lines (for example `S4-R2-3`, `S7-F7-R2-11`, `S2-R2-26`), apply every listed part.

#### Excluded rows

| Row | File | Disposition |
|---|---|---|
| `S2-R2-12` | `coding-agents.md:327` | REJECT: inside `#### OpenCode Go`, owned by `cafleet-model-list-refresh` |
| `S6-F4` | `tester.md:60,64` | REJECT: the two bullets state distinct rules |
| `S7-F7-R2-8` | `coordination.md:31` | REJECT: the line defines "pre-existing" and the `complete` default |
| `S3-F3-6` | `director.md` | Superseded by `S3-R2-21` |
| `S4-F4` | `member.md` | Superseded by `S4-R2-5` |
| `S4-F5` | `member.md` | Superseded by `S4-R2-6` |

#### Observation dispositions

Observations are adopted when they remove a pure redundancy behind an existing or one-clause link, or fix drift where `cli.md` is authoritative. Contract and policy tensions stay out of scope.

| Observation | Disposition |
|---|---|
| `member.md:69` guarded subset lists "run recovery" (drift from `cli.md`) | Covered by `S4-R2-6`, which removes the enumeration |
| `member.md:69` `permission inspect` duplicate | Covered by `S4-R2-6` |
| `SKILL.md:62` / `:94` restate `cli.md` | Covered by `S4-R2-2` and `S4-R2-4` |
| `SKILL.md:64` (`CAFLEET_*`, `--database-url`) / `:82` (body input, 200-codepoint truncation) restate `cli.md` | Kept: required core facts members act on before reading `cli.md`; `:82` also carries the out-of-scope long-body reading |
| Hermes and pi argv notes, repeated pending-notice handling in `coding-agents.md` | Covered by `S2-R2-7`, `-9`, `-15`, `-18`, `-21`, `-23`, `-24` |
| `cli.md:218` OpenCode trade-off duplicate | Covered by `S1-R11` |
| `execute.md:40-43` restates Spawn Protocol | Covered by `S6-R3` |
| `verifier.md:22` doubled qualifier | Covered by `S7-F7-R2-4` |
| `director.md:45` pane placement drift | Covered by `S3-R2-4`, which drops the phrase |
| `director.md:110` placeholder restatement | Covered by `S3-F3-3`; the remaining `member create` link is the owner pointer |
| `director.md:17` model-selection trigger list restates `model-selection.md` | Adopted as `OBS-1` |
| `director.md:365` "without a probe" contradicts `cli.md` § `member show` | Adopted as `OBS-2` |
| `coding-agents.md:404`, `:409` plain-text "§ Hermes native Director reception" | Adopted as `OBS-3` (link like every other backend) |
| `coordination.md:81` review-tag list | Kept: the only shared tag list across both reviewer roles; `S7-F7-R2-11` already tightens the row |
| `Error: ` prefix and `X` placeholder inconsistency (cli.md, SPEC) | Out of scope: contract change |
| Backend specifics in `director.md` and `verifier.md`; test-evidence sentence at `coding-agents.md:354`; internal mechanics in `cli.md` | Out of scope: audience/overlay placement decisions |
| `director.md:60` vs `:129` positional `PROMPT`; `SKILL.md:72` vs `:82` long bodies; `create.md:40` "one round" vs `drafter.md`; `create.md:61` vs `director.md` § Shutdown; `guidelines.md:78` ordering; verifier "delegate to a teammate"; `director.md:223` vs `:248`; `director.md:254` unpaired prohibition | Out of scope: each needs a decision on which reading wins |

### Part 1: Per-file skill edits

#### `assets/skills/cafleet-design-doc/SKILL.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S5-R2-1 | 17 | `This skill is the umbrella for the design-document lifecycle: the standardized **format spec** plus the CAFleet-native **orchestration workflows** in Dispatch. Consult a reference page for the format, or route a main-session user request through Dispatch to run a team. **Assigned teammates** continue their existing workflow ([Member startup](#member-startup)); loading this umbrella as a member creates no second team.` | `This skill is the umbrella for the design-document lifecycle: the standardized **format spec** plus the CAFleet-native **orchestration workflows** in Dispatch. **Assigned teammates** continue their existing workflow ([Member startup](#member-startup)) without starting a second team.` |
| S5-R2-2 | 36 | `**Routing into a workflow means executing its entire orchestration.** The Director registers its current session with `cafleet fleet create`, spawns the role team, receives each ready and drives the workflow’s review/revision rounds through approval, keeping reception under [Director wake](../cafleet/roles/director.md#director-wake) until fleet deletion; the linked workflow body owns its phase gates.` | `**Routing into a workflow means executing its entire orchestration.** The Director registers its current session with `cafleet fleet create`, spawns the role team and drives review/revision rounds through approval, keeping [Director wake](../cafleet/roles/director.md#director-wake) reception until fleet deletion; the linked workflow body owns its phase gates.` |
| S5-R2-3 | 55 | `Read [command routing](../cafleet/roles/member.md#reconsider-then-route) before routing a command your harness denied. Apply supplied host-rule equivalents where optional files are absent; route an essential unknown prerequisite to the Director before dependent work.` | delete (the line and its preceding blank line; § Member startup ends with the table). |

#### `assets/skills/cafleet-design-doc/create/create.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S5-R2-4 | 13 | *(sentence 3)*<br>`Read full durable messages, record each one as its poll consumes it, and keep reception and supervision under the shared [Director procedure](../../cafleet/roles/director.md#supervision).` | *(sentence 3 only; sentences 1–2 unchanged)*<br>`Keep reception, message recording and supervision under the shared [Director procedure](../../cafleet/roles/director.md#supervision).` |
| S5-R2-5 | 17 | `Normalize the argument per the guidelines and resolve the selected folder through BASE before writing. Preserve explicit targets; a newly chosen folder takes the next available seven-digit prefix. Set the document's absolute output path and guard audit writes through BASE.` | `Resolve the document folder and its absolute output path through [File Layout](../reference/guidelines.md#file-layout) and [Numbering](../reference/guidelines.md#numbering) before writing.` |
| S5-R2-6 | 25-26 | `3. Spawn the [Drafter](roles/drafter.md) and [Reviewer](roles/reviewer.md) with their role paths, required skills, concrete document/BASE paths and scoped assignments.` / `4. Receive and record ready before depending on a member. Dispatch each assignment when its inputs exist; inspect actual placement when launch evidence is uncertain.` | `3. Spawn the [Drafter](roles/drafter.md) and [Reviewer](roles/reviewer.md) from the prompts below.` / `4. Receive each ready and dispatch assignments per [Spawn Protocol](../../cafleet/roles/director.md#spawn-protocol) step 6; inspect actual placement when launch evidence is uncertain.` |
| S5-R2-7 | 40 | `For fresh work, the Drafter inspects relevant source and sends one clarification round before drafting: questions on consequential unanswered requirements and confirmation of the requirements it understood. Relay it through the executing backend's user-decision surface, preserve the answers and route them back faithfully. Resume and review-only work start from existing content and annotations.` | `For fresh work, relay each Drafter clarification round through the executing backend's user-decision surface and route the answers back faithfully. Resume and review-only work start from existing content and annotations.` |
| S5-R2-8 | 45 | `2. Route the complete revision to the Reviewer. Findings use coordination pointers/markers and explain the concrete problem.` | `2. Route the complete revision to the Reviewer.` |
| S5-R2-9 | 55 | `Route ordinary-language feedback and existing user markers to the Drafter without losing their meaning. Ask only about consequential ambiguity. After revisions, repeat independent review before presenting the new revision. Retain previous answers and publication authority.` | `Route ordinary-language feedback and existing user markers to the Drafter through [COMMENT(role) markers](../reference/coordination.md#commentrole-marker). After revisions, repeat independent review before presenting the new revision. Retain previous answers and publication authority.` |

#### `assets/skills/cafleet-design-doc/create/roles/drafter.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S5-F5-1 | 38 | `Ask only about points the request and earlier answers leave unsettled. After receiving answers, send at most one focused follow-up round, only when critical ambiguities remain.` | `After receiving answers, send at most one focused follow-up round, only for critical ambiguities the request and earlier answers leave unsettled.` |
| S5-F5-2 | 42 | `1. **Clarify**: Read the target codebase for context, then send clarifying questions to the Director as a free-form body; hidden BASE-rooted question-payload files are permitted ([Payload exemptions](../../reference/coordination.md#payload-exemptions)). Receive answers before creating or writing design-document content.` | `1. **Clarify**: Read the target codebase for context, then send your clarification round to the Director ([Payload exemptions](../../reference/coordination.md#payload-exemptions)) and receive the answers before creating or writing design-document content.` |
| S5-R2-10 | 3 | `You are the **Drafter** in a CAFleet design document creation team. You bear **sole responsibility for producing a high-quality design document that accurately captures the user's requirements**: you gather requirements through clarifying questions (relayed by the Director), write the document and revise it on Reviewer feedback.` | `You are the **Drafter** in a CAFleet design document creation team. You bear **sole responsibility for producing a high-quality design document that accurately captures the user's requirements**.` |
| S5-R2-11 | 17 | `**Coordination Protocol**: Clarification questions and answers are free-form ([Payload exemptions](../../reference/coordination.md#payload-exemptions)); send long payloads through `--file` and retrieve answers with `--json`. From your first draft onward, every message follows the **verb + pointer + `COMMENT(role)`** schema in [coordination](../../reference/coordination.md).` | `**Coordination Protocol**: Clarification questions and answers are free-form ([Payload exemptions](../../reference/coordination.md#payload-exemptions)). From your first draft onward, every message follows the **verb + pointer + `COMMENT(role)`** schema in [coordination](../../reference/coordination.md).` |
| S5-R2-12 | 21 | `This framework is MANDATORY, not advisory. Send all relevant questions to the Director in one message, grouped by category, with a brief context line per category explaining why you ask; the Director batches them into decision-surface relays for the user.` | `This framework is MANDATORY. Send all relevant questions to the Director in one message, grouped by category, with a brief context line per category explaining why you ask.` |
| S5-R2-13 | 23 | `Cover at least 3 categories from the framework below, or every relevant category when fewer than 3 apply. In each covered category, … Skip a category only when the request makes it entirely irrelevant (e.g., UI/UX for a backend-only feature).` | `In every category below that the request does not make entirely irrelevant (e.g., UI/UX for a backend-only feature), ask each consequential question the request leaves unanswered; where the request already answers it, state your understanding for confirmation instead (e.g., "I want to confirm my understanding: [summary]. Is this correct? Any adjustments?").` |
| S5-R2-14 | 43 | `2. **Draft**: Create the document at the OUTPUT PATH you were given, following the template rule in § Your Accountability, and send `complete (doc)`.` | `2. **Draft**: Create the document at your OUTPUT PATH and send `complete (doc)`.` |

#### `assets/skills/cafleet-design-doc/create/roles/reviewer.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S5-F5-3 | 3 | `You are the **Reviewer** in a CAFleet design document creation team. You bear **critical responsibility for ensuring every design document meets quality standards before it reaches the user**, critically reviewing drafts and giving specific, actionable feedback as inline `COMMENT(reviewer)` markers that drive the document toward excellence.` | `You are the **Reviewer** in a CAFleet design document creation team. You bear **critical responsibility for ensuring every design document meets quality standards before it reaches the user**.` |
| S5-F5-4 | 43 | `Front-load your effort: read the entire document before writing any feedback, so you catch systemic issues, not just local ones. A review that catches all issues in the first pass is far more valuable than one that trickles feedback over multiple rounds.` | `Front-load your effort: read the entire document before writing any feedback, and catch every issue, systemic and local, in the first pass rather than trickling it over multiple rounds.` |
| S5-R2-15 | 12 | `- **Ensure readability.** The document must be well-structured, scannable and free of filler, with sections that flow logically and are easy to navigate.` | `- **Ensure readability.** The document must be well-structured, scannable and free of filler, with logically ordered sections.` |
| S5-R2-16 | 37 | `If the draft meets every quality standard in § Your Accountability, send the Director `approved (doc)`. Approve only when no substantive issues remain; minor style preferences alone are not grounds for blocking approval.` | `Send the Director `approved (doc)` once no substantive issue against § Your Accountability remains; minor style preferences alone are not grounds for blocking approval.` |
| S5-R2-17 | 41 | `The Director routes the Drafter to your standing `COMMENT(reviewer)` markers with `ready (doc)`; after the Drafter revises and resubmits, the Director re-routes the updated draft to you. Repeat until you are satisfied.` | `After the Drafter revises your standing `COMMENT(reviewer)` markers, the Director re-routes the updated draft to you; review it again until you approve.` |

#### `assets/skills/cafleet-design-doc/execute/execute.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S6-R1 | 13 | `Own correctness against the approved specification. Assign bounded paths, shared interfaces, checks and evidence locations. Retrieve complete durable messages, record each one as its poll consumes it and facilitate ready work. Inspect broker, pane and artifact evidence before treating a member as stalled; disclose a replacement and preserve its handoff.` | `Own correctness against the approved specification. Assign bounded paths, shared interfaces, checks and evidence locations, and dispatch ready work. Disclose a member replacement and preserve its handoff.` |
| S6-R2 | 36 | `Read the assigned [Programmer](roles/programmer.md), [Tester](roles/tester.md), [Verifier](roles/verifier.md) and [Reviewer](roles/reviewer.md) role files before spawning; each owns its role's responsibilities. A Programmer-only team receives the documentation/configuration composition explicitly.` | `Read the assigned [Programmer](roles/programmer.md), [Tester](roles/tester.md), [Verifier](roles/verifier.md) and [Reviewer](roles/reviewer.md) role files before spawning. A Programmer-only team receives the documentation/configuration composition explicitly.` |
| S6-R3 | 40-43 | 1. Diagnose prerequisites, register the calling Director atomically and retain literal fleet/member IDs.<br>2. Follow [Director wake](../../cafleet/roles/director.md#director-wake).<br>3. Render compact role-by-path prompts with the delta table below, supplying BASE, owned paths and checks.<br>4. Receive and record ready, then dispatch each member when inputs exist. Resolve uncertain launch/input by inspecting actual evidence before another submission. | 1. Follow the Director [Spawn Protocol](../../cafleet/roles/director.md#spawn-protocol) and [Director wake](../../cafleet/roles/director.md#director-wake).<br>2. Render compact role-by-path prompts with the delta table below, supplying BASE, owned paths and checks.<br>3. Dispatch each member once its ready is recorded and its inputs exist. Resolve uncertain launch/input by inspecting actual evidence before another submission. |
| S6-R4 | 60 | last sentence `Preserve completed work when resuming.` | delete the sentence. The rest of the paragraph is unchanged. |
| S6-R5 | 70 | last sentence `User revisions return through this review before another approval presentation.` | delete the sentence. |
| S6-R6 | 86 | middle sentence `Commit only files allowed by host rules.` | delete the sentence. |

#### `assets/skills/cafleet-design-doc/execute/roles/programmer.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S6-F1 | 41 | `If resuming a partially-complete document:` | delete line 41 only; the blank line 40 then separates the heading from the numbered list. |
| S6-F2 | 61-62 | - **Tests fail (implementation bug)**: Fix your implementation and re-run tests. Repeat until all tests pass.<br>- **Tests fail (suspected test defect)**: If your implementation matches the design doc but tests expect something different, escalate to the Director per § Escalation below. | - **Tests fail (implementation bug)**: Fix your implementation and re-run tests until all pass.<br>- **Tests fail (suspected test defect)**: Escalate to the Director per § Escalation below. |
| S6-F3 | 65 | `8. **Handle Director feedback**: The Director will review your code for quality and design doc compliance. If feedback arrives as … reply `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`).` | 8. **Handle Director feedback**: On `ready (paragraph-Implementation > Step N)` (or `ready (<file>:<line>)`), read the standing `COMMENT(director)` markers at the pointer, fix the issues, re-run tests to ensure they still pass, remove the markers as part of the fix, and reply `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`). |
| S6-R7 | 18, 18 | *(both files)*<br>`**Coordination Protocol**: Follow [coordination](../../reference/coordination.md) for the verb + pointer schema, the `COMMENT(role)` role taxonomy and marker rules.` | delete the line and the blank line after it in both files, so `## Communication Protocol` opens with the **Role boundaries** paragraph. |
| S6-R8 | 20, 20 | programmer `the Director owns all git operations and every commit, all user communication and every specification decision (spec fixes need its approval).`; tester `the Director owns all git operations and every commit, and all user communication;` | programmer `the Director owns all git operations, all user communication and every specification decision (spec fixes need its approval).`; tester `the Director owns all git operations and all user communication;` |
| S6-R9 | 51 | `1. **Read the step spec**: Read the step description and checkbox items in the design document at the pointer.` | `1. **Read the step spec**: the step description and checkbox items at the pointer.` |
| S6-R10 | 57 | `3. **Implement code**: Write implementation code to make ALL tests for the step pass.` | `3. **Implement code**: Make ALL tests for the step pass.` |
| S6-R11 | 71-74 | step `1. Follow § Composition-specific scope for who changes test files and inline test regions.`, followed by steps 2–4. | delete step 1 and renumber steps 2–4 as 1–3, with their text unchanged. |

#### `assets/skills/cafleet-design-doc/execute/roles/reviewer.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S7-F7-1 | 3 | "You are the **Reviewer** … spawned fresh only after every Implementation task and Success Criterion is complete. You bear **sole responsibility for the post-implementation review**: judging the full branch diff against the design document with no memory of the implementation's compromises, and approving only when no substantive issues remain. Your approval gates the user (admin) approval: the admin sees the change only after you approve it." | *(whole line)*<br>You are the **Reviewer** in a CAFleet design document execution team, spawned fresh only after every Implementation task and Success Criterion is complete. You bear **sole responsibility for the post-implementation review**; the user (admin) sees the change for approval only after you approve it. |
| S7-F7-2 | 13 | "… to verify what the diff and design doc claim, testing hypotheses by running checks, never by editing code." | *(whole line)*<br>- **Verify claims by running checks (read-execute scope).** You may run the host project's validation tasks and other read-only checks to verify what the diff and design doc claim; never edit code. |
| S7-F7-3 | 25 | "Front-load your effort: read the design document and the **entire branch diff** before writing any feedback." | delete (the line and its following blank line) |
| S7-F7-R2-1 | 19 | "**Coordination Protocol**: Inter-member cafleet messages follow the **verb + pointer + `COMMENT(role)`** schema in [coordination](../../reference/coordination.md): a single-line `<verb> (<pointer>)` body, with findings in markers, never in the cafleet body." | **Coordination Protocol**: Inter-member cafleet messages follow the **verb + pointer + `COMMENT(role)`** schema in [coordination](../../reference/coordination.md). |

#### `assets/skills/cafleet-design-doc/execute/roles/tester.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S6-R7 | 18, 18 | *(both files)*<br>`**Coordination Protocol**: Follow [coordination](../../reference/coordination.md) for the verb + pointer schema, the `COMMENT(role)` role taxonomy and marker rules.` | delete the line and the blank line after it in both files, so `## Communication Protocol` opens with the **Role boundaries** paragraph. |
| S6-R8 | 20, 20 | programmer `the Director owns all git operations and every commit, all user communication and every specification decision (spec fixes need its approval).`; tester `the Director owns all git operations and every commit, and all user communication;` | programmer `the Director owns all git operations, all user communication and every specification decision (spec fixes need its approval).`; tester `the Director owns all git operations and all user communication;` |
| S6-R12 | 3 | `…verify the design document specification before implementation begins**; your tests define the contract the Programmer must satisfy. You work with…` | `…verify the design document specification before implementation begins**. You work with…` |
| S6-R13 | 11 | `- **Write comprehensive unit tests before implementation.** For each step, write tests that verify the design document's requirements BEFORE the Programmer implements — this is TDD.` | `- **Write comprehensive unit tests before implementation.** For each step, test the design document's requirements BEFORE the Programmer implements.` |
| S6-R14 | 14 | `- **Use the project's existing test patterns.** Match the file naming, directory structure, and assertion style already established in the project.` | `- **Use the project's existing test patterns.** Match its file naming, directory structure and assertion style.` |
| S6-R15 | 31 | `4. **If deterministic** → use the detected framework and proceed silently to Phase 2, sending no cafleet message.` | `4. **If deterministic** → use the detected framework and proceed to Phase 2, sending no cafleet message.` |
| S6-R16 | 50 | `When the Director sends `ready (paragraph-Implementation > Step N)` after a Programmer escalation, the design doc paragraph contains a standing `COMMENT(programmer)` rationale and a `COMMENT(director)` arbitration decision.` | `When the Director sends `ready (paragraph-Implementation > Step N)` after a Programmer escalation, act on that paragraph's standing markers:` |
| S6-R17 | 54-55 | line 54 `- **If valid** (the Director's decision says your test expectation was wrong per the design doc): …`; line 55 `- **If you disagree** (your test is correct per the design doc and the Director's arbitration is wrong): …` | line 54 `- **If valid** (the Director ruled your test expectation wrong per the design doc): …`; line 55 `- **If you disagree** (your test is correct per the design doc): …`. Everything after each colon is unchanged. |
| S6-R18 | 56 | `3. **Wait for the Director's next decision**: the Director arbitrates again; read the updated `COMMENT(director)` marker and act accordingly.` | `3. **Wait for the Director's next decision**, then act on the updated `COMMENT(director)` marker.` |

#### `assets/skills/cafleet-design-doc/execute/roles/verifier.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S7-F7-4 | 3 | "You are the **Verifier** … with **sole responsibility for E2E and integration testing of implemented features**: you dynamically discover available tools (MCP servers, CLI tools, skills) and use them to verify that the implementation meets the design document's success criteria. You work with … a Tester (writes unit tests)." | *(whole line)*<br>You are the **Verifier** in a CAFleet design document execution team, with **sole responsibility for E2E and integration testing of implemented features** against the design document's success criteria. You work with the Director (orchestrates, reviews, commits), the Programmer (implements code) and, in a TDD team, a Tester (writes unit tests). |
| S7-F7-5 | 20 | "… before the initial tool-discovery inventory, your first substantive payload after ready; use complete JSON retrieval …" | *(whole line)*<br>**Coordination Protocol**: The initial tool-discovery inventory follows coordination's [Payload exemptions](../../reference/coordination.md#payload-exemptions); Phase 2 reports follow the ordinary [coordination](../../reference/coordination.md) schema. |
| S7-F7-6 | 53-54 | "Then report to the Director per the Verifier-specific reporting policy:" / "- **Overall success** (all verifiable criteria pass): send a single `complete (doc)`; E2E commonly spans multiple steps, so success is reported once at doc level." | *(both lines, indentation preserved)*<br>Then report to the Director:<br>- **Overall success** (all verifiable criteria pass): send a single `complete (doc)`, since E2E commonly spans multiple steps. |
| S7-F7-R2-2 | 11 | "- **Verify implementations against success criteria.** Use E2E and integration testing to confirm the implementation works as specified in the design document, beyond what unit tests cover." | - **Verify implementations against success criteria** with E2E and integration testing, beyond what unit tests cover. |
| S7-F7-R2-3 | 12 | "- **Discover and use the best available tools.** At startup, inventory all available tools (MCP servers, CLI tools, skills) and select the most appropriate ones for each verification task." | - **Discover and use the best available tools.** Inventory them at startup (§ Phase 1: Tool Discovery) and select the most appropriate ones for each verification task. |
| S7-F7-R2-4 | 22 | "**Role boundaries:** the Director owns all git operations and every commit, and all user communication; …" | *(only the opening clause changes; the rest of the line stays verbatim)*<br>**Role boundaries:** the Director owns all git operations and user communication; the Programmer and Tester own the implementation and test files — you modify neither; blockers route to the Director. You run no subagents and launch no other coding-agent session (no backend binary and no `cafleet member create`); `cafleet` broker commands remain part of your protocol. |
| S7-F7-R2-5 | 30 | "1. Inventory available tools and MCP servers (browser automation, HTTP clients, etc.)" | 1. Inventory available tools and MCP servers |
| S7-F7-R2-6 | 55 | "   - **Failures**: send one `escalating (paragraph-Implementation > Step N)` per affected step, with its paired `COMMENT(verifier)` marker at that same pointer." | *(indentation preserved: three leading spaces)*<br>- **Failures**: send one `escalating (paragraph-Implementation > Step N)` per affected step. |

#### `assets/skills/cafleet-design-doc/reference/coordination.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S7-F7-7 | 20 | "Members and the Director MUST use exactly these 6 verbs and invent no others." | Members and the Director MUST use exactly these 6 verbs. |
| S7-F7-8 | 64 | "\| No payloads \| … substantive content (reasoning, file lists beyond 3, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. \|" | \| No payloads \| The summary is for human readability in the broker timeline; substantive content (reasoning, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. \| |
| S7-F7-9 | 96 | "… rides as a pure cafleet message with whatever short phrase fits (`restarted`, `still working`, `ack`, `noted`, etc.); the set is **not a fixed canonical list**. It MUST NOT match …" | *(first sentence only; the sentence beginning "It MUST NOT match" stays verbatim)*<br>A status that points at no heading, file or doc — a restart, "still working, no progress yet", a generic acknowledgement — rides as a pure cafleet message with any short phrase that fits, such as `restarted`, `still working`, `ack` or `noted`. |
| S7-F7-R2-7 | 14 | "Retrieve and send these payloads complete under the cafleet skill's [Send and Poll rules](../../cafleet/SKILL.md#send-unicast) (`--json` reads, `--file` for long bodies), recording each delivery the poll consumes." | Retrieve and send these payloads complete under the cafleet skill's [Send and Poll rules](../../cafleet/SKILL.md#send-unicast) (`--json` reads, `--file` for long bodies). |
| S7-F7-R2-9 | 45 | "A `paragraph-` pointer uses the literal heading text, preserved verbatim (slashes, colons, hyphens and other punctuation stay literal), and nests with the three-character separator ` > ` (space, greater-than, space)." | A `paragraph-` pointer uses the heading text verbatim, punctuation included, and nests with the separator ` > ` (space, greater-than, space). |
| S7-F7-R2-10 | 61 | "\| Single line \| The body MUST be a single line, with no literal newlines. \|" | \| Single line \| The body MUST contain no literal newlines. \| |
| S7-F7-R2-11 | 81, 86 | *(81)*<br>"… execute code-review findings at `<file>:<line>` — both tagged `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` in the body \|"; (86): "Write one marker per logical issue, with an actionable body stating the issue and what should change. A reviewer body carries one of the 5 review tags: `COMMENT(reviewer): [GAP] Step 4 lacks a verification entry.`" | *(81, whole row)*<br>\| `reviewer` \| Reviewer \| Create design-doc findings; execute code-review findings at `<file>:<line>` — both tagged in the body with one of `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` \|<br>replacement (86, whole line):<br>Write one marker per logical issue, its body stating the issue and what should change: `COMMENT(reviewer): [GAP] Step 4 lacks a verification entry.` |
| S7-F7-R2-12 | 92 | "… Status updates ("ready", "complete") live only in `cafleet message send` verb + pointer text and never enter the design doc." | *(last sentence only; the rest of the line stays verbatim)*<br>Status updates ("ready", "complete") live only in `cafleet message send` verb + pointer text. |
| S7-F7-R2-13 | 98 | "… It never carries or replaces a role's first substantive message — the Drafter still sends its clarification round as a separate message and drafts nothing before the answers arrive." | *(second sentence only; the first sentence stays verbatim)*<br>It never carries or replaces a role's first substantive message, such as the Drafter's separate clarification round. |

#### `assets/skills/cafleet-design-doc/reference/guidelines.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S7-F7-R2-14 | 71 | "- **Background**: Only when the reader lacks context. Skip for obvious changes." | - **Background**: Only when the reader lacks context. |
| S7-F7-R2-15 | 87 | "2. Normalize the consumer's argument to its actual task folder — a relative or absolute folder, not a child filename — before containment resolution: design-doc workflows use the normalization above, and other consumers own their bucket conventions." | 2. Normalize the consumer's argument to its task folder (a relative or absolute folder, not a child filename): design-doc workflows use the normalization above, and other consumers own their bucket conventions. |

#### `assets/skills/cafleet/SKILL.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S4-F6 | 98 | "Use [cli.md](reference/cli.md) for invocation forms, flags, outputs and errors; its [subcommand summary](reference/cli.md#subcommand-summary) is the command registry, and per-command sections follow beside the shared output-shape, pane-input and setup/doctor sections." | "Use [cli.md](reference/cli.md) for invocation forms, flags, outputs and errors; its [subcommand summary](reference/cli.md#subcommand-summary) is the command registry." |
| S4-R2-1 | 32 | "3. **Apply notes.** Follow each Note → applies at caveat at its named consuming instruction. Ordinary members resolve their own runtime section and take on no model-selection duties." | "3. **Apply notes.** Follow each Note → applies at caveat at its named consuming instruction." |
| S4-R2-2 | 62 | "Members read their literal `FLEET ID:`, … spawn lines; Directors use IDs returned by creation. Spawn prompts contain exactly the CLI placeholders `{fleet_id}`, … `{database_url}`: the CLI fills the new member ID, … for a non-default database. Supply exactly one positional `PROMPT` or `--file PATH`." | "Members read their literal `FLEET ID:`, `DIRECTOR MEMBER ID:`, `YOUR MEMBER ID:` and `DATABASE URL:` spawn lines; Directors use IDs returned by creation and fill spawn prompts through the [`member create`](reference/cli.md#member-create) placeholders." |
| S4-R2-3 | 84, 86 | *(84)*<br>"A send stores the message and returns; an ordinary recipient's own runner types its preview later. Previews carry …"<br>*(86)*<br>"Retain the committed message ID. Send each body once; the recipient's runner delivers and retries the preview on its own. Recipients record deferred actions when their poll consumes the message. Fan out one send per recipient and report each failed recipient; [send outcomes](reference/cli.md#message-send) owns exact diagnostics." | *(84)*<br>"A send stores the message and returns; an ordinary recipient's own runner types its preview later and retries it on its own. Previews carry …" (rest of the paragraph unchanged)<br>*(86)*<br>"Retain the committed message ID and send each body once. Fan out one send per recipient and report each failed recipient; [send outcomes](reference/cli.md#message-send) owns exact diagnostics." |
| S4-R2-4 | 94 | "Poll returns every `input_required` delivery newest first, with complete bodies under `--json`, and consumes them in the same command: each returned record is `completed`, and no later poll returns it." | "Poll returns every `input_required` delivery newest first, with complete bodies under `--json`, and consumes them in the same command ([`message poll`](reference/cli.md#message-poll))." |

#### `assets/skills/cafleet/reference/cli.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S1-F1 | 242 | `Hermes forwards the level as `--reasoning`, and pi as `--thinking`.` | delete the line (and its following blank line) |
| S1-F2 | 267 | `The member environment forwards a set `HERMES_HOME`.` | delete the line (and its following blank line) |
| S1-F3 | 269 | `A pi spawn requires … `cafleet setup --coding-agent pi' first`, exit 1 before registration. The member environment forwards a set `PI_CODING_AGENT_DIR`. A gated member whose extension …` | delete only the middle sentence `The member environment forwards a set `PI_CODING_AGENT_DIR`.` so the paragraph reads:<br>`A pi spawn requires the CAFleet pi extension `<pi base>/extensions/cafleet.ts` in every launch mode: `Error: cannot read the CAFleet pi extension at <path>: <cause>; run 'cafleet setup --coding-agent pi' first`, exit 1 before registration. A gated member whose extension cannot register `--cafleet-gate` exits at startup with `Error: Unknown option: --cafleet-gate`, reported as `runner_exited`.` |
| S1-F4 | 328 | `Uncertain errors include `input_id=INPUT_ID`. The pre-write over-limit CAUSE strings are shared with [member prompt](#member-prompt); queued over-limit refusals settle `not_submitted/plain_limit_exceeded` and publish a result. An uncertain prompt/exec requires a deliberate retry.` | `Uncertain errors include `input_id=INPUT_ID`. The pre-write over-limit CAUSE strings are shared with [member prompt](#member-prompt); queued over-limit refusals settle `not_submitted/plain_limit_exceeded` and publish a result.` |
| S1-R1 | 38 | `Every other command, including `permission inspect`, `input show`, `event show`, `member list`, `member capture` and the lifecycle commands, carries no guard refusal; `permission inspect` still requires `--from-member-id` as its pinned Director (§ `permission inspect`).` | `Every other command carries no guard refusal; among them, `permission inspect` still requires `--from-member-id` as its pinned Director (§ `permission inspect`).` |
| S1-R2 | 58 | `Members using an existing window copy its recorded name, including names from existing registrations.` | `Members using an existing window copy its recorded name.` |
| S1-R3 | 75 | `… under ask to confirm each of the Director's explicit pane-input dispatches; ask outranks allow. Director permission responses remain subject to effective native/managed rules.` | delete the final sentence `Director permission responses remain subject to effective native/managed rules.`. The paragraph then ends at `ask outranks allow.` |
| S1-R4 | 77 | `… the Director-only commands refuse a member's own ID (§ Director-only command guard). Members retain broker reception/evidence-routing authority.` | delete the final sentence `Members retain broker reception/evidence-routing authority.` |
| S1-R5 | 85 | `… A failed `TMUX`/`HERDR_ENV` detection gives `cafleet fleet create must be run inside a tmux or herdr session`. Director messages use broker reception.` | delete the final sentence `Director messages use broker reception.` |
| S1-R6 | 107 | `… Existing fleets retain their recorded names; subsequent user renames leave snapshots unchanged. Ordinary pane titles/labels retain their current behavior.` | delete the final sentence `Ordinary pane titles/labels retain their current behavior.` |
| S1-R7 | 126 | `Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered, and a rerun resumes` | `Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered` |
| S1-R8 | 157 | `A turn with k mid-turn polls can queue at most k + 1 notices behind it; each announces only messages those polls already consumed, so its poll returns `[]` unless messages arrived since.` | `A turn with k mid-turn polls can queue at most k + 1 notices behind it.` |
| S1-R9 | 186 | `Ordinary concrete panes receive previews from their own runner's sweep; Directors and self-sends use broker reception. Profile previews are `[cafleet msg …` | delete the first sentence, so the paragraph begins `Profile previews are `[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.`` (rest unchanged) |
| S1-R10 | 216 | `… A host may need multiple Escapes; submission remains valid when the host queues instead of interrupting. Resume and dismissal clauses are conditional.` | delete the final sentence `Resume and dismissal clauses are conditional.` |
| S1-R11 | 218 | `… The CLI defers termination during pane input. OpenCode captures may show only a status bar; inspect retained sessions for omitted output and a message's `completed` state (`message show`) for its consumption.` | delete the final sentence `OpenCode captures may show only a status bar; inspect retained sessions for omitted output and a message's `completed` state (`message show`) for its consumption.` |
| S1-R12 | 246 | `A pi prompt body beginning with `@` exits 2 before registration with `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`; start the prompt with text.` | `A pi prompt body beginning with `@` exits 2 before registration with `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`` |
| S1-R13 | 255 | `Exit 1, nothing registered: `Error: window <window-name> … run 'cafleet member delete MEMBER_ID' for each, then retry.`; delete each listed member, then retry` | `Exit 1, nothing registered: `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.`` |
| S1-R14 | 330 | `… Inspect native output and reconcile receipts before a newly authorized retry. Receipts retain operation key/digest/state, without command text. After completion, …` | delete the sentence `Receipts retain operation key/digest/state, without command text.` |
| S1-R15 | 387 | `Uncertain prompt/exec errors exit 1 with known receipt/context, including `native command delivery to member 110 is uncertain: write_failed (method=profile)` and `input_id=INPUT_ID`.` | `Example uncertain exec error: `native command delivery to member 110 is uncertain: write_failed (method=profile)`, then `input_id=INPUT_ID`.` |
| S1-R16 | 491 | `… each quoted command appending ` --database-url URL` for a non-default database. An `error` member row makes doctor exit 1.` | delete the final sentence `An `error` member row makes doctor exit 1.` |
| S1-R17 | 503 | `The refusal body is the `fleet create` refusal without its `cafleet fleet create ` prefix, and a `warn` keeps the exit code at 0.` | `The refusal body is the `fleet create` refusal without its `cafleet fleet create ` prefix.` |

#### `assets/skills/cafleet/reference/coding-agents.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S2-F2-1 | 90 | "`timeout_ms` is required on every start and renewal. Monitor defaults it to 300,000 (5 minutes), shorter than the 1,770-second CLI lease, and caps it at 1,800,000 (30 minutes). With `1800000` the lease closes 30 seconds inside the host deadline, so the `closed` `timeout` record and stream completion precede Monitor expiry." | "With `1800000` the lease closes 30 seconds inside the host deadline, so the `closed` `timeout` record and stream completion precede Monitor expiry." |
| S2-F2-2 | 181 | "\| `{permission_flags}` \| `--agent cafleet-ask` (default mode `cafleet-ask`, class `manual`: the member broker commands run and every other shell command opens a native permission form); `--agent cafleet` (class `auto`: every other shell command is denied) when `--permission-mode cafleet` is selected \|" | "\| `{permission_flags}` \| `--agent cafleet-ask` (default mode `cafleet-ask`, class `manual`); `--agent cafleet` (class `auto`) when `--permission-mode cafleet` is selected \|" |
| S2-F2-3 | 447 | "\| `{permission_flags}` \| `--cafleet-gate cafleet-ask` (default mode `cafleet-ask`, class `manual`: the CAFleet member gate runs the member broker commands, opens its approval form for every other bash command and protects `.git`/`.pi`/`.agents`/`.claude`/`.codex`/`.opencode`); `--cafleet-gate cafleet` (class `auto`: every other bash command is blocked without a form) when `--permission-mode cafleet` is selected \|" | "\| `{permission_flags}` \| `--cafleet-gate cafleet-ask` (default mode `cafleet-ask`, class `manual`); `--cafleet-gate cafleet` (class `auto`) when `--permission-mode cafleet` is selected \|" |
| S2-F2-4 | 381 | "\| `{permission_flags}` \| no permission argv; the only recorded mode is `smart` (default, class `auto`), and the operator configuration's `approvals.mode` governs the session, with hardline floors, the sudo guard and `approvals.deny` enforced \|" | "\| `{permission_flags}` \| no permission argv; the only recorded mode is `smart` (default, class `auto`), and the operator configuration's `approvals.mode` governs the session \|" |
| S2-R2-1 | 3 | "Execute instructions with your own backend's Runtime bindings and bound notes. To spawn, select the backend under [Director model selection](../roles/director.md#model-selection), check its catalog, defaults, eligibility and runtime capabilities, and apply [Resolve your overlay](../SKILL.md#resolve-your-overlay)." | "Execute instructions with your own backend's Runtime bindings and bound notes; before a spawn, apply [Resolve your overlay](../SKILL.md#resolve-your-overlay) to the selected backend." |
| S2-R2-2 | 11 | "A role verdict is `yes` only when evidence establishes every Eligible-tier requirement; otherwise `no` with the missing evidence/failure. A Director needs two independent requirements, goal and listen; a `no` verdict names each unmet one. Evidence carries forward until re-check." | "A role verdict is `yes` only when evidence establishes every Eligible-tier requirement; otherwise `no`, naming each unmet requirement's missing evidence or failure. A Director needs two independent requirements, goal and listen. Evidence carries forward until re-check." |
| S2-R2-3 | 20 | "… and a classified default launch mode (`auto` — native automatic review or a fixed allow/deny policy — or `manual`, native interactive approval) whose bundled profile …" | "… and a default launch mode classified `auto` or `manual` ([mode classes](cli.md#member-create)) whose bundled profile …" (rest of the paragraph unchanged) |
| S2-R2-4 | 72 | "Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established. The Director authorizes routed native input under its host rules." | "Native shell probes displayed command/stdout/stderr followed by a model turn; its trigger and a second permission check were not established." |
| S2-R2-5 | 77 | "Several tool calls in one turn show one permission form at a time, each heading ending in `N of M`, and each form is its own permission request. An allow answers only the current form, and the next form arrives as a new request; a deny on any form interrupts the turn and refuses every call that has not completed." | "Several tool calls in one turn show one permission form at a time, each its own permission request with a heading ending in `N of M`. An allow answers only the current form; a deny on any form interrupts the turn and refuses every call that has not completed." |
| S2-R2-6 | 79 | "… arrives as a `question` `interaction_opened` with `review_question`; inspection lists no choice. Relay the question to the user, who answers in the member pane." | "… arrives as a `question` `interaction_opened` with `review_question`; inspection lists no choice." (form description unchanged) |
| S2-R2-7 | 92 | "Require `ready`, retain the task ID, pinned member/fleet IDs, receive status and unfinished actions, and keep Monitor active across other work, idle periods and turn endings while the fleet exists. Each `pending` stdout line … Events during a user question preserve that pending decision." | "Require `ready`, retain the Monitor task ID as the receiver handle and keep Monitor active across other work, idle periods and turn endings while the fleet exists. Handle each `pending` stdout line as a wake notice under [Director wake](../roles/director.md#director-wake) steps 1–4 and 6." |
| S2-R2-8 | 149 | "… so the effective TUI boundary and protocol path are not established. The Director authorizes routed native input under its host rules, where harmless command visibility establishes no second member-tool permission check, and inspects output before explicit continuation." | "… so the effective TUI boundary and protocol path are not established. Harmless command visibility establishes no second member-tool permission check." (first sentence unchanged) |
| S2-R2-9 | 162 | "4. Each `[cafleet listen record]` pending notice arrives as a queued user turn. Run the notice's `next` command once, record every returned message before any other command and act within authorization ([Director wake](../roles/director.md#director-wake) steps 3–4); an empty `[]` poll completes a stale notice. Notices interleave with `/goal` turns; reception preserves the host goal's state." | "4. Each `[cafleet listen record]` pending notice arrives as a queued user turn, interleaving with `/goal` turns; handle it under [Director wake](../roles/director.md#director-wake) steps 3–4." |
| S2-R2-10 | 163 | "5. On closure, follow [receiver recovery](../roles/director.md#receiver-recovery): read the retained handle once and verify exit before replacement. Fleet deletion ends reception; the poll that starts [Director wake](../roles/director.md#director-wake)'s overdue check covers missing closure notices." | "5. On closure, follow [receiver recovery](../roles/director.md#receiver-recovery); fleet deletion ends reception." |
| S2-R2-11 | 165 | "The forwarder's stdout, record forwarding and exit with its pinned Codex session follow [`message listen`](cli.md#message-listen). Preserve the exec `session_id` and unfinished actions across compaction. Reception makes no pane input." | "The forwarder's stdout, record forwarding and exit with its pinned Codex session follow [`message listen`](cli.md#message-listen)." |
| S2-R2-13 | 344 | "… every other `cafleet` command falls to the shell base, and Director-only commands refuse a member's own `--from-member-id` with an error telling the member to send the request to its Director." | "… every other `cafleet` command falls to the shell base." (preceding clauses unchanged) |
| S2-R2-14 | 352 | "On OpenCode a deny submits an empty rejection reason and ends the member's turn with `Step interrupted`; after verifying that nothing ran, continue the member with a Director message, which its runner delivers into the idle composer. The bundled OpenCode profile …" | "A deny submits an empty rejection reason and ends the member's turn with `Step interrupted`. The bundled OpenCode profile …" (rest unchanged) |
| S2-R2-15 | 360 | "3. Each `pending` notice arrives as its own turn beginning `[cafleet listen record]`, starting a turn when you are idle and waiting for the end of a running turn otherwise. Run the notice's `next` command once, record every returned message before any other command and act within authorization ([Director wake](../roles/director.md#director-wake) steps 3–4); an empty `[]` poll completes a stale notice. The poll releases the hold for the next notice." | "3. Each `pending` notice arrives as its own turn beginning `[cafleet listen record]`; handle it under [Director wake](../roles/director.md#director-wake) steps 3–4." |
| S2-R2-16 | 362 (`#### OpenCode native Director reception`) | "5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id`, and `database_url` when needed, on every goal tool. Reception itself calls no goal tool." | "5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`." |
| S2-R2-17 | 365 | "After a restart on the same session, re-arm with `cafleet_listen` on the first turn; that turn also restores an active goal. Exiting the client ends the receiver. Reception makes no pane input." | "After a restart on the same session, re-arm with `cafleet_listen` on the first turn; that turn also restores an active goal. Exiting the client ends the receiver." |
| S2-R2-18 | 408 | "Each pane runs `hermes chat --cli --source tool --in <cwd>` with optional `--model` and `--reasoning`, then `-q <prompt>` as a fresh conversation. `member create` forwards a set `HERMES_HOME`, so the member uses the configuration setup enabled the plugin in and the preflight resolved." | "Each pane runs the Hermes argv of [member create](cli.md#member-create) as a fresh conversation under the forwarded `HERMES_HOME`, so the member uses the configuration setup enabled the plugin in and the preflight resolved." |
| S2-R2-19 | 412 | "Leading-`!` input runs a native shell command in the classic CLI REPL and the TUI client, where it was verified. The Director authorizes routed native input under its host rules." | "Leading-`!` input runs a native shell command in the classic CLI REPL and the TUI client, where it was verified." |
| S2-R2-20 | 417 | "2. Register with `cafleet fleet create --name <name> --coding-agent hermes --json`. It refuses at the gateway, so follow [Director pane identification](../roles/director.md#director-pane-identification) and rerun with the user's reported `--pane <pane-id> --mux-server <server>`." | "2. Register with `cafleet fleet create --name <name> --coding-agent hermes --json`; it refuses at the gateway, so follow [Director pane identification](../roles/director.md#director-pane-identification)." |
| S2-R2-21 | 419 | "4. Each `pending` notice arrives as its own turn beginning `[cafleet listen record]`; a notice that arrives during a busy turn runs after that turn ends, without interrupting it. Run the notice's `next` command once, record every returned message before any other command and act within authorization ([Director wake](../roles/director.md#director-wake) steps 3–4); an empty `[]` poll completes a stale notice. The poll releases the hold for the next notice." | "4. Each `pending` notice arrives as its own turn beginning `[cafleet listen record]`; handle it under [Director wake](../roles/director.md#director-wake) steps 3–4." |
| S2-R2-22 | 431 | "Reception makes no pane input and leaves `/goal` authority with the host." | delete the line and its following blank line |
| S2-R2-23 | 475 | "A pane runs `pi --cafleet-gate <mode> --no-approve --tui-mode fullscreen --name "<member_id> - <name>"`; optional `--model` and `--thinking` follow, then `--` and the prompt, as a fresh session. `member create` forwards a set `PI_CODING_AGENT_DIR`, and a gated member whose extension cannot register `--cafleet-gate` exits at startup (`runner_exited`)." | "A pane runs the pi argv of [member create](cli.md#member-create) as a fresh session under the forwarded `PI_CODING_AGENT_DIR`; a gated member whose extension cannot register `--cafleet-gate` exits at startup (`runner_exited`)." |
| S2-R2-24 | 485 | "3. Each `pending` notice arrives as a `[cafleet listen record]` message: it starts a turn when you are idle and otherwise arrives at the end of the running turn. Run the notice's `next` command once, record every returned message before any other command and act within authorization ([Director wake](../roles/director.md#director-wake) steps 3–4); an empty `[]` poll completes a stale notice. The poll releases the hold for the next notice." | "3. Each `pending` notice arrives as a `[cafleet listen record]` message; handle it under [Director wake](../roles/director.md#director-wake) steps 3–4." |
| S2-R2-25 | 487 (`#### pi native Director reception`) | "5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id`, and `database_url` when needed, on every goal tool. Reception itself calls no goal tool." | "5. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id`, and `database_url` when needed, on every goal tool." |
| S2-R2-26 | 488-490 | "6. A `[cafleet listen closed]` message, including the runtime-replaced notice after `/reload`, `/new`, `/resume` or `/fork`, an `ok: false` result or a missing tool follows …" and "… Quitting pi ends the receiver. Reception makes no pane input." | line 488: "6. A `[cafleet listen closed]` message, including the runtime-replaced notice, an `ok: false` result or a missing tool follows …" (rest unchanged). Line 490: "After a restart on the same session (`pi --continue`), re-arm with `cafleet_listen` on the first turn; that turn also continues an active goal. Quitting pi ends the receiver." |
| OBS-3 | 404, 409 | "(§ Hermes native Director reception)" in the Hermes TUI-client row (404) and the Hermes gateway row (409) | "([native reception](#hermes-native-director-reception))" in both rows; the rest of each row unchanged |

#### `assets/skills/cafleet/reference/model-selection.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S4-R2-7 | 25 | "Apply these rows only when the operator names no backend. Inference never selects `hermes` or `pi`: a Hermes or pi member requires an operator-named backend or a fleet backend of that name ([backend-resolution](../roles/director.md#model-selection) steps 1 and 3), and a provider-prefixed model for a Hermes- or pi-named member follows that backend's explicit mapping." | "Apply these rows only when the operator names no backend. Inference never selects `hermes` or `pi`, which require an operator-named backend or a fleet backend of that name ([backend-resolution](../roles/director.md#model-selection) steps 1 and 3)." |
| S4-R2-8 | 27 | "Row 1 wins over row 2 for `anthropic/claude-sonnet-4-6`, which contains both `claude` and a `/`: a provider-prefixed token is itself an explicit OpenCode request. A bare name infers …" | "A bare name infers `opencode` only through row 4; Claude and GPT names select their native backends even when either OpenCode table lists the model." (delete the first sentence only) |
| S4-R2-9 | 69 | "The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against the OpenCode candidate IDs. It requires exactly one key-equal ID, so "sonnet 4.6 with OpenCode" resolves to the ID `claude-sonnet-4-6` before provider choice; normalization steps 2–4 apply only to native-backend resolution." | "The last row uses only Model-name normalization step 1, including the leading-`claude` strip, against the OpenCode candidate IDs and requires exactly one key-equal ID: "sonnet 4.6 with OpenCode" resolves to the ID `claude-sonnet-4-6` before provider choice." |
| S4-R2-10 | 99 | "Use this owner for provider choice and cost ranking. Complete provider-prefixed tokens from explicit references, …" | "Complete provider-prefixed tokens from explicit references, defaults or family-word mapping remain verbatim and skip provider choice. A bare resolved ID has `opencode-go/<id>` and/or `opencode/<id>` candidates only where its catalog tables list them." (delete the first sentence only) |

#### `assets/skills/cafleet/roles/director.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S3-F3-1 | 57-58 | - `` \| `--model` \| The selected backend's catalog token or Role default applies; a `—` default omits it \| ``<br>- `` \| `--effort` \| The selected backend's `{effort_levels}` accept the role's level; omit it for an unsupported or `—` level \| `` | - `` \| `--model` \| Per [Model selection](#model-selection) \| ``<br>- `` \| `--effort` \| Per [Model selection](#model-selection) \| `` |
| S3-F3-2 | 68 | "A backend's `{permission_flags}` note names any first-launch or nudge dialog … follows that section too." | "Every first-launch or nudge dialog, whether named in the backend's `{permission_flags}` note or not (such as a folder-trust prompt), follows [Consume observed interactions](#consume-observed-interactions), whose question rules cover listed and unlisted choices." |
| S3-F3-3 | 110 | "The CLI supplies the allocated IDs, backend and database (`default` or `sqlite:///<path>`); a member whose `DATABASE URL:` names a URL passes `--database-url <url>` on every command." | delete this sentence (keep the surrounding first and remaining sentences of the paragraph unchanged). |
| S3-F3-4 | 125 | "Guard `${BASE} != <unset>` before constructing audit paths; skip audit writes when disabled." | "Guard `${BASE} != <unset>` before constructing audit paths." |
| S3-F3-5 | 153 | "The Reviewer supplies evidence (an `[INCORRECT]` marker naming the suspected unmet capability) and stays independent of the execution." | "The Reviewer supplies evidence and stays independent of the execution." |
| S3-F3-7 | 341 | "A persisted-but-undelivered send needs no further message: the recipient's runner retries it, and Director wake's overdue check covers a silent recipient." | delete (the line and its preceding blank line). |
| S3-R2-1 | 3 | *(whole line)*<br>"You are a **Director** managing CAFleet members under their recorded launch policies. Members run host-permitted commands and route returned denials/automatic refusals to you. Verify current permission requests under [Runtime events](#runtime-events); separately authorized commands use `member exec`. Each backend's `{permission_flags}` note defines its host boundary."<br>Dropping the Runtime events pointer and the `member exec` sentence is intended: § Authority and refusal links dispatch, § Director-side dispatch owns `member exec`, and § Runtime events owns permission requests. | "You are a **Director** managing CAFleet members under their recorded launch policies, each bounded by its backend's `{permission_flags}` note. Members run host-permitted commands and route returned denials and automatic refusals to you." |
| S3-R2-2 | 27 | "Pass catalog tokens exactly with `--model` and a supported `--effort`; OpenCode takes no `--effort`, and a `—` Role default model or effort omits that flag so the backend's configured default applies." | "Pass catalog tokens exactly with `--model` and a supported `--effort`; a `—` Role default model or effort omits that flag so the backend's configured default applies." |
| S3-R2-3 | 41 | "Own member lifecycle, roster listing, captures and explicit pane input, and run your own host-authorized commands directly." | "Run your own host-authorized commands directly." |
| S3-R2-4 | 45 | "`cafleet member create` resolves the Director from the fleet row, registers the member with a pending placement, renders the spawn prompt, spawns its pane in your multiplexer window and records the real pane ID." | "`cafleet member create` resolves the Director from the fleet, registers the member, renders the spawn prompt and spawns its pane." |
| S3-R2-5 | 66 | "Spawn every member with the omitted `--permission-mode`, which selects the backend's default mode. Pass `--permission-mode` only when … their `auto`/`manual` classes." | "Omit `--permission-mode`, selecting the backend's default mode, unless the user explicitly requests a mode for that member; [member create](../reference/cli.md#member-create) and each backend's `{permission_flags}` note list the choices and their `auto`/`manual` classes." |
| S3-R2-6 | 78 | "Every CAFleet-native team skill spawns its ordinary members from this one shared frame; each skill supplies only a compact **per-role delta** (a table in that skill) for the parts that vary." | "Every CAFleet-native team skill spawns its ordinary members from this shared frame plus its own compact **per-role delta** table for the parts that vary." |
| S3-R2-7 | 127 | "Write each pre-substitution prompt before `member create --file`. Its canonical permanent audit path is … Preserve its five identity placeholders unchanged; that original file is the audit artifact." | "Before `member create --file`, write each pre-substitution prompt, with its five identity placeholders unchanged, to its permanent audit path `<BASE>/.prompts/<role>-<UTC-compact>.md`, using lowercase `--name` and `YYYYMMDDTHHMMSSZ`; create the folder lazily and use `_2`, `_3`, … for same-second collisions." |
| S3-R2-8 | 135 | "Resolve BASE once and pass members the resulting absolute path; members inherit it without re-resolution. Resolution writes nothing; the chosen folder is created lazily on first write." | "Resolve BASE once and pass members the resulting absolute path. Resolution writes nothing; the chosen folder is created on first write." |
| S3-R2-9 | 149 | "A selected temporary BASE is valid; use the selected root consistently." | "A selected temporary BASE is valid." |
| S3-R2-10 | 171 | "Delete a member before creating its replacement." | delete this sentence (rest of the paragraph unchanged). |
| S3-R2-11 | 198 | "`` \| `cafleet member prompt …` TEXT` \| Submit explicitly authorized composer input or post-command continuation; it rejects the Director's pane, and `delivery` reports submission/queue acceptance \| ``" | second cell → "Submit explicitly authorized composer input or post-command continuation" (first cell unchanged). |
| S3-R2-12 | 201 | "All writers share the bounded pane-input lock. Prompt/exec requires … [Member Press](#member-press) owns manual keys, their authorization and outcome evidence." | "Prompt/exec requires the pinned runner and durable receipt; [wrapped delivery methods](../reference/cli.md#wrapped-delivery-methods) owns profile submission, zero-byte queueing, fallback and delivery arbitration." |
| S3-R2-13 | 217 | "It holds the revalidated shared pane lock and sends one bounded backend request." | delete this sentence (rest of the paragraph unchanged). |
| S3-R2-14 | 227 | "The Director owns `permission inspect`/`respond` and `input reconcile`; each takes your literal ID as `--from-member-id`. The [CLI guard](…) refuses any other caller ID on `permission respond` and `input reconcile`; `permission inspect` carries no guard refusal but requires the pinned Director's ID." | "The Director owns `permission inspect`/`respond` and `input reconcile`; each takes your literal ID as `--from-member-id`, which the [CLI guard](../reference/cli.md#director-only-command-guard) enforces on `permission respond` and `input reconcile`." |
| S3-R2-15 | 233 | "1. Read complete inbox JSON through the `poll --json` a receiver notice prompts, or a manual `poll --json`; the poll consumes what it returns. Preserve host receive state and pending user decisions." | `1. Read complete inbox JSON per [Director wake](#director-wake) step 3 or a manual `poll --json`, preserving host receive state and pending user decisions.` |
| S3-R2-16 | 248 | "CLI decisions are semantic, separate from arbitrary keys." | delete this sentence (rest of the paragraph unchanged). |
| S3-R2-17 | 250 | "Explicit prompt/exec requires a separate deliberate retry after pane inspection." | delete this sentence (rest of the paragraph unchanged). |
| S3-R2-18 | 280 | "[Director wake](#director-wake) steps 3–4 own polling and recording deliveries; unfinished assignments and pending user decisions stay tracked in host-owned state, and stream/native delivery changes no consumption state." | "[Director wake](#director-wake) owns polling, recording deliveries and tracking unfinished assignments and pending user decisions." |
| S3-R2-19 | 285 | "Preserve the user's host continuation controls as supplied." | delete this sentence (rest of the step unchanged). |
| S3-R2-20 | 286 | "… and capture literal fleet and Director IDs; every later command targets the recorded placement." | "… and capture literal fleet and Director IDs." (rest of the step unchanged) |
| S3-R2-21 | 287 | "4. Arm one `message listen` receiver using the executing backend's [runtime recipe](…): a native tool, the `--codex-thread` forwarder or a CAFleet plugin or extension tool. Pass the recipe's complete invocation, … on this arm and on every renewal or replacement. Retain its handle and require `ready` … Every receiver failure or closure follows [receiver recovery](#receiver-recovery)." | "4. Arm one `message listen` receiver with the executing backend's [runtime recipe](../reference/coding-agents.md), passing its complete invocation, including every host tool deadline it sets (Claude Monitor `timeout_ms`). Require `ready` before depending on replies; initial replay recovers messages racing with startup. Every receiver failure or closure follows [receiver recovery](#receiver-recovery)." |
| S3-R2-22 | 308 | "A `pending` notice is a wake signal whose `next` field is the consuming command, never a payload." | delete this sentence (rest of the step unchanged). |
| S3-R2-23 | 314 | "Run one persistent receiver through your backend's reception recipe in [coding-agents.md](../reference/coding-agents.md); [message listen](../reference/cli.md#message-listen) owns the lease, record shapes, notice hold, replay, failures and closure." | "[message listen](../reference/cli.md#message-listen) owns the lease, record shapes, notice hold, replay, failures and closure." |
| S3-R2-24 | 353 | "A quiet or ambiguous pane calls for inspection rather than an inferred failure: timeouts, idle duration and unread counts alone establish no failure, so check the assignment's expected ready/reply point." | "Timeouts, idle duration, unread counts and a quiet or ambiguous pane alone establish no failure; check the assignment's expected ready/reply point." |
| OBS-1 | 17 | "Also read [model-selection procedures](../reference/model-selection.md) when the operator names a model, alias, family word or backend for a member, the originating request contains `cost efficiency mode`, the member backend is `opencode`, a [Model replacement](#model-replacement) re-selects, or a model is mismatched, unsuitable or missing." | "Before each spawn, also check the trigger table of [model-selection procedures](../reference/model-selection.md) and read the sections it names." (rest of the paragraph unchanged) |
| OBS-2 | 365 | "`member show --json` reads its stored runtime state without a probe." | "`member show --json` reads its stored runtime state and probes a `running` or `degraded` runner's health ([member show](../reference/cli.md#member-show))." (rest of the paragraph unchanged) |

#### `assets/skills/cafleet/roles/member.md`

| Row | Line | Current | Replacement |
|---|---|---|---|
| S4-F1 | 31 | "Poll after ready, on every wake (a preview arriving is a wake) and before completion reports. The poll consumes every message it returns, so record each message's actionable content and deferred work before any other work; `message show MESSAGE_ID --json` reads a consumed message again. Handle queued assignments before becoming idle; end the turn when none remain." | "Poll after ready, on every wake (a preview arriving is a wake) and before completion reports, recording each returned message under the core [Poll](../SKILL.md#poll) rule. Handle queued assignments before becoming idle; end the turn when none remain." |
| S4-F2 | 33 | "A `[cafleet msg ID from SENDER TIMESTAMP]` preview line carries the body inline or a pointer to your `poll --json`; still run `poll --json` on that wake, which consumes that message and any others in one command." | "A `[cafleet msg ID from SENDER TIMESTAMP]` preview carrying its body inline still needs the wake's `poll --json`, which consumes that message and any others in one command." |
| S4-F3 | 35 | "A send needs no follow-up: the recipient's runner delivers and retries the preview." | delete (the whole one-line paragraph, plus its blank line) |
| S4-R2-5 | 41 | "When `BASE:` is omitted, disable audits, send `audit-disabled no BASE in spawn prompt` once to the Director as a parens-free anchorless status, and use explicit consumer output targets when supplied. Preserve that disabled state and every explicit output target; anchorless status follows the shared [coordination convention](../../cafleet-design-doc/reference/coordination.md). Resolution and spawn-audit procedures belong to the Director." | "When `BASE:` is omitted, keep audits disabled for the whole run, send `audit-disabled no BASE in spawn prompt` once to the Director as an [anchorless status](../../cafleet-design-doc/reference/coordination.md#anchorless-status), and use every explicit consumer output target supplied. Spawn-audit procedures belong to the Director." |
| S4-R2-6 | 69 | "Use Send, Poll and `message show` on your own inbox; every other `cafleet` command is Director-owned. The [guarded subset](../reference/cli.md#director-only-command-guard) — explicit pane input, permission answers, input reconciliation and run recovery — requires the Director's `--from-member-id` and refuses your own ID with a `Director-only command` error. The rest carry no guard refusal and remain the Director's; `permission inspect` still requires the Director's `--from-member-id`." | "Use Send, Poll and `message show` on your own inbox; every other `cafleet` command is Director-owned, and the [guarded subset](../reference/cli.md#director-only-command-guard) refuses your own ID with a `Director-only command` error." |

### Part 2: Schema layer simplification

#### Retained and removed elements

| Element | Decision | Reason |
|---|---|---|
| `PRAGMA application_id` `1128351289` | Keep | File identity; the empty-file rule and the foreign-identity refusal depend on it |
| `cafleet_database` version row | Keep | The only cross-version guard for an explicit `--database-url` path |
| Required-shape check (`schema_matches`) | Keep | Catches development-time schema edits under an unchanged version and post-creation damage |
| Empty-file rule, `BEGIN IMMEDIATE` initialization | Keep | Serializes concurrent setup and refuses to stamp a schema onto a populated file |
| Incompatible-format refusal with byte preservation | Keep | Fail-loud protection of a wrong file |
| `refinery_schema_history` table, its row, `applied_on`, `checksum` | Remove | No migration runner exists; nothing reads the columns |
| `LedgerState`, `ledger_state` | Remove | Ledger reads only |
| `assets/migrations/` directory, `V1__` prefix | Remove | refinery's discovery convention |
| "baseline" terminology | Replace with "schema" | Nothing builds on a baseline; keep the `#### Schema-version guard` heading, whose anchor `docs/` and `cli.md` link |

Schema digests, `PRAGMA user_version` and dropping `application_id` stay out of scope.

#### Schema asset

`git mv assets/migrations/V1__baseline.sql assets/schema.sql`, delete its trailing `CREATE TABLE refinery_schema_history (...)` statement and the blank line before it, and remove the empty `assets/migrations/`. In `src/db.rs`:

```rust
const SCHEMA_SQL: &str = include_str!("../assets/schema.sql");
```

`SCHEMA_SQL` replaces every `BASELINE_SQL` use. No other code embeds or walks `assets/migrations/`.

#### Classifier

Delete `enum LedgerState` and `fn ledger_state`. `classify_schema` keeps its empty-file check, then tests identity, shape and version in that order:

```rust
fn classify_schema(conn: &Connection) -> Result<Classification, CafleetError> {
    if is_empty_file(conn)? {
        return Ok(Classification::Missing);
    }
    let identity: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if identity != crate::db::APPLICATION_ID
        || !schema_matches(conn)?
        || !records_running_version(conn)?
    {
        return Ok(Classification::Incompatible);
    }
    Ok(Classification::Current)
}
```

#### Initialization

`initialize` branches once on the classifier inside the same `IMMEDIATE` transaction:

```rust
pub(crate) fn initialize(conn: &mut Connection, database_url: &str) -> Result<(), CafleetError> {
    let failed =
        |e: rusqlite::Error| CafleetError::App(format!("database initialization failed: {e}"));
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(failed)?;
    match classify_schema(&tx)? {
        Classification::Current => return Ok(()),
        Classification::Incompatible => {
            return Err(incompatible_format(database_path(database_url)?));
        }
        Classification::Missing => {}
    }
    tx.execute_batch(SCHEMA_SQL).map_err(failed)?;
    tx.execute(
        "INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, ?1)",
        [crate::cli::VERSION],
    )
    .map_err(failed)?;
    tx.commit().map_err(failed)
}
```

`classify_schema` → `schema_matches` → `reference_schema` calls `initialize` on a fresh `:memory:` connection, whose classification returns `Missing` at the empty-file check before `schema_matches` runs, so the recursion is one level deep. `schema_matches` already skips its snapshot transaction when the connection is inside a transaction. `initialize` no longer calls `crate::digest::sha256_hex` or `crate::time::now_stamp`; both modules keep their other callers.

| File state | Condition | Result |
|---|---|---|
| Empty | Application identity zero and no user tables | Applies the schema, inserts the `cafleet_database` row for the running version, commits |
| Current | The classifier returns current | Writes nothing and succeeds |
| Other | Every other file, including a matching identity with another recorded version or a damaged required shape | Writes nothing and fails with the incompatible-format error naming the database path |
| Unreadable | A SQL error while classifying | Writes nothing and fails with that error |

Setup's observable behavior is unchanged, because setup classifies before it calls `initialize`. A current-version development file created before this change still contains `refinery_schema_history`; the shape check treats it as an auxiliary table, so the file stays current and needs no cleanup. This change does not bump the version; release CI owns versioning.

#### Documentation and rule edits

| Location | New text or action |
|---|---|
| `SPEC.md:20` module table | `\| Persistence \| Connection factory, format/shape diagnosis and the embedded schema \|` |
| `SPEC.md:83` | "Format validation, schema initialization, incompatible-file refusal and file preservation follow §6.3 Schema-version guard." (rest unchanged) |
| `SPEC.md:286` asset table | `` \| `assets/schema.sql` \| The complete database schema \| Schema initialization (§8 Database schema) \| `` |
| `SPEC.md:360` §6.3 Schema-version guard | "with the baseline atomically" → "with the embedded schema atomically" |
| `SPEC.md:3186` | "The baseline creates the runtime records" → "The schema creates the runtime records" |
| `SPEC.md:3778` §8 intro | Replace the paragraph with the §8 intro text below |
| `SPEC.md:3793` | Delete the `refinery_schema_history` table row |
| `SPEC.md:3795` | "Exact schema DDL:" |
| `SPEC.md:3965-3970` | Delete the `refinery_schema_history` statement and the blank line before it, mirroring `assets/schema.sql` |
| `SPEC.md:3973` | Delete the ledger-row paragraph |
| `SPEC.md:3975` | "Column semantics:" |
| `SPEC.md:3999-4005` | Replace the initialization table (header through last row) and the closing paragraph with the §8 initialization text below; the `SPEC.md:3997` sentence before the table stays |
| `docs/docs/concepts/storage.md:17` | "A compatible file has CAFleet's application identity, the cafleet version that initialized it equal to the running CLI's, and the expected table/column shapes; …" (rest unchanged) |
| `docs/docs/concepts/storage.md:21` | Result cell: "Apply the embedded schema transactionally" |
| `docs/docs/contributing.md:10` | "`assets/migrations/` the SQL schema baseline" → "`assets/schema.sql` the SQL schema" |
| `docs/docs/contributing.md:183` | "Fresh-baseline initialization" → "Fresh-schema initialization" |
| `.claude/rules/code-quality.md` § Database schema | Replace the paragraph with the rule text below |
| `.claude/rules/documentation-maintenance.md:36` | "the schema baseline" → "the embedded schema" |
| `CLAUDE.md:11` | "an embedded SQL schema baseline" → "an embedded SQL schema" |

`README.md` and `assets/skills/` mention none of these and need no Part 2 edit.

§8 intro text:

> The binary embeds one complete schema, `assets/schema.sql` (§6.3 Asset installation), whose application identity is `PRAGMA application_id` `1128351289` (`0x43414639`, ASCII `CAF9`). Each cafleet version owns its default database file, `cafleet_<VERSION>.db`, where `<VERSION>` is the release version (§7.6), so each version starts from a fresh file; schema initialization records that version in the `cafleet_database` row. A fresh initialization creates exactly the application tables declared in the schema. Compatibility diagnosis verifies, in order, the application identity, every required table’s column shape with required indexes/foreign keys, and a recorded version equal to the running binary's. Any other identified or populated file, including one recorded by another cafleet version, is an incompatible format whose bytes are preserved, and `cafleet doctor` reports the version's default file as absent until `cafleet setup` creates it. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

§8 initialization text (the sentence before the table stays):

> | File state | Condition | Result |
> |---|---|---|
> | Empty | Application identity zero and no user tables | Applies the schema and inserts the `cafleet_database` row for the running version, then commits |
> | Current | A file the compatibility diagnosis above classifies current | Writes nothing and succeeds |
> | Other | Every other file | Writes nothing and fails with the §6.3 incompatible-format error naming the database path |
>
> A read error during that classification fails initialization with its cause and writes nothing. A failed initialization leaves no identity, table or version row, and two concurrent initializations of one empty file both succeed with exactly one version row.

`code-quality.md` § Database schema text:

> `assets/schema.sql` is the complete schema; edit it in place for every schema change. Each cafleet version uses its own default database file, `cafleet_<VERSION>.db`, so every release starts from a fresh database and no data carries across versions. Schema initialization records the creating version in `cafleet_database`. Setup and the command guard accept only a file whose application identity, recorded version and required table/column shapes match the running binary, and refuse every other populated file as incompatible without writing to it.

#### Test changes

Every change stays in the in-file `#[cfg(test)] mod tests` block of the file it edits.

| Test | File | Change |
|---|---|---|
| `repeated_initialization_preserves_all_application_rows` | `src/db.rs` | Replace the ledger snapshot with a `SELECT * FROM cafleet_database` snapshot, asserted unchanged after each repeated `initialize` |
| `baseline_initialization_records_the_identity_ledger_version_and_shape` | `src/db.rs` | Rename to `initialization_records_the_identity_version_and_shape`; delete the ledger, `applied_on` and `checksum` assertions; keep identity, version row and shape assertions |
| `a_v8_family_file_is_incompatible_and_left_byte_identical` | `src/db.rs` | Make the `cafleet_v8.db` pair's damage `PRAGMA application_id=1128351288` (`0x43414638`), delete the `if name == "cafleet_v8.db"` `pragma_update` branch and the then-unused `V8_APPLICATION_ID` constant; both pairs then run only `conn.execute_batch(damage)` |
| `OTHER_FILE_CASES` / `initialization_refuses_every_other_file` | `src/db.rs` | Delete `ledger-row-beyond-baseline` and `ledger-row-deleted`; rename `unrelated-table-without-ledger` to `unrelated-table-only`; add `recorded-other-version` (`initialized: true`, `UPDATE cafleet_database SET cafleet_version='0.0.0'`) and `required-index-dropped` (`initialized: true`, `DROP INDEX idx_member_runner_diagnostics_recent`); the array stays at 4 cases |
| `concurrent_initialization_of_one_fresh_file_applies_the_baseline_once` | `src/db.rs` | Rename to `…_applies_the_schema_once`; drop the ledger count, keep the `cafleet_database` count and `require_current` |
| `a_ledger_row_beyond_the_baseline_is_incompatible` | `src/db.rs` | Delete |
| `failed_baseline_initialization_rolls_back_identity_tables_and_ledger` | `src/db.rs` | Rename to `failed_initialization_rolls_back_identity_and_tables`; body unchanged |
| `classification_preserves_missing_current_identity_and_sql_causes` | `src/db.rs` | Replace the ledger-column rename with a genuine SQL failure: a temp file holding non-SQLite bytes, opened with `Connection::open`, for which `classify_schema` returns `Err` (`file is not a database`) |
| `ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes` | `src/cli/mod.rs` | Held reader `BEGIN; SELECT * FROM cafleet_database` |
| `refused_wal_databases_and_sidecars_remain_byte_identical_with_a_held_reader` | `src/cli/setup.rs` | Held reader `BEGIN; SELECT * FROM cafleet_database` |

The remaining identity, shape, auxiliary-object, composer-receipt and refusal tests in `src/db.rs` stay unchanged.

#### Test helper rename

Rename `db::test_support::migrated_conn` to `initialized_file_conn` at its definition in `src/db.rs` and at every call site (74 occurrences across `src/broker/`, `src/cli/`, `src/pane_lock.rs`, `src/runtime/mod.rs` and `src/db.rs` at the time of writing). The rename is mechanical and compiler-checked.

### Commits

| Commit | Content |
|---|---|
| `docs: tighten bundled skill prose` | Step 4 |
| `refactor: drop the vestigial schema ledger and rename the schema asset` | Steps 1, 2, 5 and 6 |
| `refactor: rename the migrated_conn test helper` | Step 7 |
| `docs: add design doc 0000242-skills-and-schema-simplification` | This document's `design-doc.md` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Steps 1–5 complete the documentation before Step 6 changes code. Part 1 is Step 4; Part 2 is Steps 1, 2, 5, 6 and 7.

### Step 1: SPEC.md (Part 2)

- [x] Update `SPEC.md:20` and `SPEC.md:83` to the "embedded schema" / "schema initialization" wording <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the `assets/migrations/` asset-table row at `SPEC.md:286` with the `assets/schema.sql` row <!-- completed: 2026-11-23T02:56 -->
- [x] Update §6.3 Schema-version guard (`SPEC.md:360`) to "with the embedded schema atomically", keeping the heading <!-- completed: 2026-11-23T02:56 -->
- [x] Update `SPEC.md:3186` to "The schema creates the runtime records" <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the §8 intro paragraph with the §8 intro text <!-- completed: 2026-11-23T02:56 -->
- [x] Delete the `refinery_schema_history` table row, DDL statement and ledger-row paragraph; retitle "Exact schema DDL:" and "Column semantics:" <!-- completed: 2026-11-23T02:56 -->
- [x] Replace the §8 initialization table and closing paragraph (`SPEC.md:3999-4005`) with the §8 initialization text, keeping the `SPEC.md:3997` sentence <!-- completed: 2026-11-23T02:56 -->

### Step 2: docs/ (Part 2)

- [x] Update `docs/docs/concepts/storage.md:17` and `:21` <!-- completed: 2026-11-23T02:57 -->
- [x] Update `docs/docs/contributing.md:10` and `:183` <!-- completed: 2026-11-23T02:57 -->

### Step 3: README.md

- [x] Confirm `README.md` mentions neither the schema asset, the ledger nor any edited skill sentence; record "no change" <!-- completed: 2026-11-23T02:57 -->

### Step 4: assets/skills/ (Part 1)

- [x] Apply the edit tables for `assets/skills/cafleet/SKILL.md` and `assets/skills/cafleet/roles/member.md` <!-- completed: 2026-11-23T02:56 -->
- [x] Apply the edit table for `assets/skills/cafleet/reference/cli.md` <!-- completed: 2026-11-23T02:58 -->
- [x] Apply the edit table for `assets/skills/cafleet/reference/coding-agents.md`, including `OBS-3`, leaving every Model catalog and Role defaults subsection untouched <!-- completed: 2026-11-23T03:01 -->
- [x] Apply the edit table for `assets/skills/cafleet/reference/model-selection.md` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the edit table for `assets/skills/cafleet/roles/director.md`, including `OBS-1` and `OBS-2` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the edit tables for `assets/skills/cafleet-design-doc/SKILL.md`, `create/create.md`, `create/roles/drafter.md` and `create/roles/reviewer.md` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the edit tables for `assets/skills/cafleet-design-doc/execute/execute.md` and every file under `execute/roles/` <!-- completed: 2026-11-23T03:03 -->
- [x] Apply the edit tables for `assets/skills/cafleet-design-doc/reference/coordination.md` and `reference/guidelines.md` <!-- completed: 2026-11-23T03:03 -->
- [x] Run both checks in § Verifying the skill edits: the heading `git diff` check prints nothing, and every link target and `#anchor` in the edited files resolves <!-- completed: 2026-11-23T03:03 -->

### Step 5: Rules and project instructions (Part 2)

- [x] Replace `.claude/rules/code-quality.md` § Database schema with the rule text <!-- completed: 2026-11-23T02:57 -->
- [x] Update `.claude/rules/documentation-maintenance.md:36` to "the embedded schema" <!-- completed: 2026-11-23T02:57 -->
- [x] Update `CLAUDE.md:11` to "an embedded SQL schema" <!-- completed: 2026-11-23T02:57 -->

### Step 6: Schema code and tests (Part 2)

- [x] `git mv assets/migrations/V1__baseline.sql assets/schema.sql`, delete its `refinery_schema_history` statement and remove `assets/migrations/` <!-- completed: 2026-11-23T02:59 -->
- [x] Rename `BASELINE_SQL` to `SCHEMA_SQL` with the new `include_str!` path in `src/db.rs` <!-- completed: 2026-11-23T02:59 -->
- [x] Delete `LedgerState` and `ledger_state` and drop the ledger clause from `classify_schema` <!-- completed: 2026-11-23T02:59 -->
- [x] Rewrite `initialize` per § Initialization <!-- completed: 2026-11-23T02:59 -->
- [x] Apply the `src/db.rs` rows of § Test changes <!-- completed: 2026-11-23T03:05 -->
- [x] Apply the `src/cli/mod.rs` and `src/cli/setup.rs` rows of § Test changes <!-- completed: 2026-11-23T03:05 -->
- [x] Run `mise //:test` and `mise //:lint`; both pass <!-- completed: 2026-11-23T03:05 -->

### Step 7: Test helper rename (Part 2)

- [x] Rename `migrated_conn` to `initialized_file_conn` at its definition and every call site <!-- completed: 2026-11-23T03:10 -->
- [x] Run `mise //:test` and `mise //:lint`; both pass <!-- completed: 2026-11-23T03:10 -->

### Step 8: Validation

- [x] `git grep -n -E "migrations|refinery|V1__|LedgerState|ledger_state|BASELINE_SQL|migrated_conn" -- ":!design-docs"` returns nothing <!-- completed: 2026-11-23T03:05 -->
- [x] `git grep -n -i -E "ledger|schema baseline|baseline initialization|fresh-baseline|embedded baseline" -- SPEC.md docs .claude/rules CLAUDE.md src` returns nothing <!-- completed: 2026-11-23T03:05 -->
- [x] Run `mise //docs:build`, `mise //:test` and `mise //:lint` on the final tree; all pass <!-- completed: 2026-11-23T03:05 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-23 | Initial draft |
