# Design Doc as Communication Medium

**Status**: Approved
**Progress**: 23/23 tasks complete
**Last Updated**: 2026-05-08

## Overview

Switch CAFleet design-doc skills (`/design-doc-create`, `/design-doc-execute`, with cosmetic alignment for `/design-doc-interview`) so every team member uses the design document itself as the substantive communication medium and `cafleet message send` carries only a verb + pointer poke. Issue and feedback content moves into inline `COMMENT(role)` markers in the design doc; status updates stay as one-line cafleet messages.

## Success Criteria

- [x] A new `skills/design-doc/coordination.md` reference defines the pointer + verb + COMMENT(role) protocol and is linked from `skills/design-doc/SKILL.md`, `skills/design-doc-create/SKILL.md`, `skills/design-doc-execute/SKILL.md`, and `skills/design-doc-interview/SKILL.md`.
- [x] Every `cafleet message send --text` example in the touched SKILL.md / role files conforms to the verb + pointer schema (verb chosen from the canonical 6, pointer drawn from the canonical 3 forms), is single-line, and respects the ~80-codepoint summary cap.
- [x] Reviewer feedback, test reports, implementation reports, escalation reasons, code-review notes, and Copilot review routing in the role files are expressed as `COMMENT(role)` markers in the design doc (or, for source-anchored Copilot, in the source file) instead of long cafleet message bodies.
- [x] No file under `cafleet/` source, no Alembic migration, and no schema/wire-format change is touched by this design doc.
- [x] `rg "Reviewer feedback: " skills/`, `rg "Tests at: " skills/`, `rg "Test results \(all passing\)" skills/`, and `rg "What tests you wrote" skills/` each return 0 hits after the changes land.

---

## Background

Today, every coordination event between Director and members is encoded in the body of a `cafleet message send`:

- The Reviewer ships a multi-paragraph review (`[COMPLIANCE] ... [GAP] ... [IMPROVEMENT] ...`) over the wire.
- The Director relays Reviewer feedback verbatim to the Drafter (`Reviewer feedback: ... Please address`).
- The Programmer reports test results, file lists, and design-doc compliance notes in the message body.
- The Director routes Copilot inline review comments by quoting `<file>:<line> — <body>` into a fresh cafleet message per item.

This makes message bodies long, duplicates content the design doc already (or could) hold, and forces every reader of the admin WebUI timeline to reconstruct meaning from scrollback. The design document — the one persistent artefact the whole team is working on — is the natural place for substantive content. `cafleet message send` should be a poke, not a payload.

`/design-doc-interview` already follows this shape: it writes `COMMENT(claude)` annotations inline and treats the document as the medium. This change extends the same shape to `/design-doc-create` and `/design-doc-execute`, generalises the role tag, and codifies the protocol in one shared reference.

This design doc is the **rationale-of-record** for the protocol. The new `skills/design-doc/coordination.md` carries the protocol mechanics (verb table, pointer table, marker format, rules); reasons and tradeoffs live here.

---

## Specification

### Core Principle

The design document is the substantive communication medium. `cafleet message send --text` carries only a single-line verb + pointer; substantive content (feedback, reports, escalation reasons, review items) is written into the design doc as inline `COMMENT(role)` markers — except for source-anchored Copilot inline review, which is annotated in the source file at `<file>:<line>` because that is where the comment lives.

### Verb Vocabulary

The canonical set is exactly 6. Members and the Director MUST pick from this list and MUST NOT invent new verbs.

| Verb | Sender direction | Meaning | Used for |
|:--|:--|:--|:--|
| `ready` | Director → member, or member → Director | "The pointer target is ready for you to read / act on." | Fresh assignments, member-to-Director "I have something for you to look at," and Director stall-nudges (the recipient interprets contextually — see below). |
| `complete` | Member → Director | "I finished a fresh deliverable at the pointer target." | Initial drafts, freshly written tests, freshly written implementation, FIXME-resolution sweeps. |
| `addressed` | Member → Director, or Director → Director (self-note) | "I resolved a pre-existing marker (a `COMMENT(role)` marker, a Copilot inline comment, or a Director-arbitration note) at the pointer." | Round-2+ work on items already flagged in the doc or in source. |
| `blocked` | Member → Director | "I cannot proceed at the pointer; the blocker rationale is in a `COMMENT(role)` marker at the same pointer." | Spec ambiguity, missing deps, environmental issues. |
| `escalating` | Member → Director | "I am escalating an issue (e.g. a suspected test defect) at the pointer; the rationale is in a `COMMENT(role)` marker at the same pointer." | Test-defect arbitration, multi-round disagreements. |
| `approved` | Reviewer → Director, or Director → user-result | "All quality criteria are met at the pointer (typically `doc`)." | Reviewer approval signal in `/design-doc-create`. |

**Verb choice for `complete` vs `addressed`**: `complete` signals a fresh deliverable (work that did not previously have a marker waiting). `addressed` signals resolution of a pre-existing marker (any `COMMENT(role)`, any Copilot line, any Director arbitration). When in doubt, ask: "did a marker exist before I started this turn?" — if yes, use `addressed`; if no, use `complete`.

**Director stall-nudges** (e.g. the existing milestone-table prods like "Please send your clarifying questions") reuse `ready (doc)` or `ready (paragraph-...)` — they are `ready` from the Director's perspective ("the pointer target is ready for you, please act"). The recipient interprets contextually: on receiving `ready (...)`, scan the pointer target for any `COMMENT(role)` markers addressed to your role or relevant to your current phase, and act accordingly. Re-sent `ready (...)` after a member's idle window is a nudge, not a new assignment — the same target, the same expected action.

Anchorless meta-events (a member crashed and restarted, a generic ping ack, a "still working") do NOT use a verb from this list and do NOT touch the design doc — see *Anchorless Status* below.

### Pointer Forms

Exactly 3 canonical forms. Use the tightest one that locates the target.

| Form | Example | When to use |
|:--|:--|:--|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. Use the literal heading text. Nest with the three-character separator ` > ` (space, greater-than, space). Heading text is preserved verbatim — slashes, colons, hyphens, and other punctuation inside a heading remain literal. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `cafleet/src/cafleet/cli/main.py:142` | The target is a specific line (or range) in a source file, test file, or the design doc itself. Used for source-anchored Copilot inline review and for source-file `COMMENT` markers added during code review. |
| `doc` | `doc` | The target is the design document as a whole (e.g., Reviewer signalling overall approval, Drafter signalling the full draft is ready). |

Rationale for the ` > ` separator: heading text in real-world design docs frequently contains `/` (e.g. `Step 2: Update /design-doc-create`); using `/` as a nesting separator collides with literal slashes and forces an escape rule. ` > ` is unambiguous, ASCII-safe, shell-safe inside double-quoted `--text` arguments, and human-readable.

### Message Format

Every `cafleet message send --text` body, when used to coordinate within a `/design-doc-create` or `/design-doc-execute` team, MUST match:

```
<verb> (<pointer>)
```

Optional one-line summary may follow, separated by ` — ` (space, em-dash, space):

```
<verb> (<pointer>) — <one-line summary>
```

Constraints:

| Constraint | Rule |
|:--|:--|
| Single line | The body MUST be a single line. No literal newlines. |
| Summary cap | The optional summary SHOULD fit on one terminal line; aim for ≤ 80 codepoints. |
| Enumeration cap | The summary MUST NOT enumerate more than 3 items. Longer enumerations belong in a `COMMENT(role)` marker at the pointer. |
| No payloads | The summary is for human readability in the admin WebUI timeline; substantive content (reasoning, file lists beyond 3, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. |

Examples:

- `ready (paragraph-Implementation > Step 1)`
- `complete (paragraph-Implementation > Step 1) — 12 tests pass`
- `addressed (cafleet/src/cafleet/cli/main.py:142)`
- `blocked (paragraph-Specification > Retry Strategy)`
- `escalating (paragraph-Implementation > Step 3)`
- `approved (doc)`

### COMMENT(role) Marker

Inline marker placed in the design document (or, for source-anchored Copilot inline review, in the source file at `file:line`).

```
COMMENT(<role>): <substantive content>
```

Roles:

| Role | Who writes it | When |
|:--|:--|:--|
| `claude` | The Director acting as user-mediator (existing convention from `/design-doc-interview`) | Carries user-derived clarifications. Existing usage unchanged. |
| `director` | The Director | Spec resolution notes, Director judgments, ambiguity arbitration, design-doc-anchored Copilot review (see *Copilot Routing* below), Phase C code-review feedback in `/design-doc-execute`. |
| `drafter` | The Drafter | Spec ambiguities the Drafter cannot resolve and needs Director input on. Example: while drafting, the Drafter notices two sections imply contradictory retry counts → `COMMENT(drafter): retry count contradicts paragraph-Specification > Retry Strategy; need user arbitration` placed at the affected paragraph + `blocked (paragraph-Specification > Retry Strategy)`. |
| `reviewer` | The Reviewer | Review findings, tagged with the existing `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` taxonomy inside the marker body. |
| `programmer` | The Programmer | Implementation-side notes, escalation rationales, observations of spec gaps that block implementation. |
| `tester` | The Tester | Test-spec gaps (Phase 2 and Phase 1 framework-selection), escalation rationales, evaluation of Programmer-routed test-defect reports. |
| `verifier` | The Verifier | E2E findings, evidence pointers (`see file:line`), suggested-fix categorisation (impl bug / test gap / spec issue). |
| `copilot` | The Director on Copilot's behalf (Copilot does not edit files) | One marker per source-anchored inline review item, written into the **source file** at `<file>:<line>`. The Director copies the Copilot body into the marker. (Design-doc-anchored Copilot lines route through `COMMENT(director)` instead — see *Copilot Routing*.) |

Rules:

- One marker per logical issue. Do not bundle.
- Body must be actionable — state the issue and what should change.
- Reviewer markers carry one of the 5 review tags inside the body: `COMMENT(reviewer): [GAP] Step 4 lacks a Phase D entry.`
- Markers are split into two classes — *issue* and *status* — only the *issue* class enters the doc (see below).

### Issue Markers vs Status Markers (Option II — split)

`COMMENT(role)` markers carry **issue and feedback content only**. Status updates ("ready", "complete") stay as pure cafleet messages with verb + pointer; they do NOT add a marker to the design doc.

| Class | Example | Lifecycle |
|:--|:--|:--|
| Issue | `COMMENT(reviewer): [GAP] Specification omits the retry budget.` | Persists in the doc until resolved. The resolver removes the marker as part of the fix. Per `skills/design-doc/guidelines.md` § *Completeness Check*, the doc cannot reach `Status: Approved` / `Status: Complete` while any `COMMENT(` marker remains. |
| Status | (none — never enters the doc) | Lives only in `cafleet message send` text. |

This keeps the design doc clean: at any moment, the markers in the doc reflect *outstanding work*, never historical chatter.

### Coordination Reference Doc

A new file `skills/design-doc/coordination.md` carries the protocol **mechanics only** — verb table, pointer table, message format, marker format, role table, issue-vs-status rule, Copilot routing edge cases, anchorless-status rule, finalize cleanup rule. Rationale and tradeoffs do NOT live there; design doc 0000050 (this file) is the rationale-of-record. All four design-doc skills (`design-doc`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) link to `coordination.md` from a top-level "Coordination Protocol" reference. Per-skill SKILL.md and role files reference the protocol by link rather than restating it; small skill-specific tables map *which* verb / pointer / role each phase uses.

### Per-Skill Application

#### /design-doc-create

| Phase | Old cafleet body (excerpt) | New cafleet body | Where the substantive content moves |
|:--|:--|:--|:--|
| Director relays user clarification answers to Drafter | `User answers: ...` | (exempt — see Clarification Exemption below) | The Drafter has not yet written the design doc, so there is no in-doc target for `COMMENT(claude)` markers. The user answers ride as a one-shot multi-line cafleet body the same way they do today. |
| Drafter signals draft ready | `Draft complete at <path>...` | `complete (doc)` | (no marker — the design doc itself is the deliverable) |
| Director routes draft to Reviewer | `Please review the draft at ${DOC_PATH}. Provide feedback or signal APPROVED.` | `ready (doc)` | (no marker — the doc is the input) |
| Reviewer reports findings (round 1, fresh) | Long body with `[COMPLIANCE] ... [GAP] ...` bullets | `complete (doc) — N issues` | Each finding is a `COMMENT(reviewer): [TAG] <body>` marker placed inline at the offending section. |
| Reviewer signals approval | `APPROVED - Ready for user review.` | `approved (doc)` | (no marker) |
| Director routes Reviewer feedback to Drafter | `Reviewer feedback: ... Please address.` | `ready (doc)` | (no marker — the Drafter reads the existing `COMMENT(reviewer)` markers in the doc) |
| Drafter signals revision complete (round 2+) | `Revisions applied: ...` | `addressed (doc)` | The Drafter resolves each `COMMENT(reviewer)` marker by editing the section and removing the marker. (`addressed` because the markers pre-existed; the Drafter is not producing a fresh deliverable.) |

**Clarification Exemption**. Director-to-Drafter messages during the Step 2 clarification phase are exempt from the verb + pointer schema for one reason: at clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying). The Director's "User answers: ..." relay rides as a free-form multi-line cafleet body, identical to today. Once the Drafter has written the initial draft (Step 2 → Step 3 transition), every subsequent message in the team falls back under the schema.

**Step 0 resume-detection nuance**. With the role taxonomy expanded beyond `claude`, a half-finished doc that legitimately contains stale `COMMENT(reviewer)` / `COMMENT(director)` / `COMMENT(programmer)` markers must NOT be misclassified as "interview-resume mode." `skills/design-doc-create/SKILL.md` Step 0 currently greps for the bare prefix `COMMENT(` to set `SKIP_CLARIFICATION=true`. The grep MUST be tightened to `COMMENT(claude)` specifically, because only the `claude` role marker is produced by `/design-doc-interview` and is the only signal that resume-mode is appropriate.

#### /design-doc-execute

| Phase | Old cafleet body (excerpt) | New cafleet body | Where the substantive content moves |
|:--|:--|:--|:--|
| Director assigns Tester (Phase A) | `Step N: <description>. Spec: <…>. Write unit tests and report file paths when done.` | `ready (paragraph-Implementation > Step N)` | (no marker — the Tester reads the spec at the pointer in the doc) |
| Tester reports tests written | `What tests you wrote (test names and descriptions); Which files you created or modified; What requirements the tests cover` | `complete (paragraph-Implementation > Step N) — <count> tests` | (no marker for status. If the spec is unclear, the Tester adds `COMMENT(tester): <gap>` at the offending paragraph and sends `blocked (paragraph-Implementation > Step N)`.) |
| Director assigns Programmer (Phase B) | `Step N: <description>. Tests at: <paths>. Implement to pass all tests, update design doc checkboxes and Progress counter, then report.` | `ready (paragraph-Implementation > Step N)` | (no marker — the Programmer locates test files via `git log <base>..HEAD --name-only -- '**/test_*' '**/tests/**'`; the prior `complete` summary went Tester→Director, not Tester→Programmer.) |
| Programmer reports impl complete | `What you implemented; Which files were changed; Test results (all passing)` | `complete (paragraph-Implementation > Step N)` | (no marker for status. If an issue surfaces, `COMMENT(programmer): <note>` at the offending paragraph + `blocked` or `escalating`.) |
| Programmer escalates suspected test defect | Multi-paragraph body with test name, design-doc citation, and expected/actual diff | `escalating (paragraph-Implementation > Step N)` | The Programmer writes `COMMENT(programmer): test <test-name> expects X but design doc says Y (cited via paragraph-Specification > <…>); please arbitrate` **at `paragraph-Implementation > Step N`** — the marker location matches the `escalating(...)` pointer per the protocol invariant; the spec citation lives inside the marker body, not in the marker placement. |
| Director arbitration after escalation | (informal) | `ready (paragraph-Implementation > Step N)` | The Director writes `COMMENT(director): <decision> — <rationale, ≤2 sentences>` at the same paragraph. The receiving member acts and replies `addressed (paragraph-Implementation > Step N)`. |
| Director Phase C code-review feedback | Long quality + compliance body | `ready (paragraph-Implementation > Step N)` | The Director writes `COMMENT(director): <issue>` markers at the offending step. The Programmer resolves and replies `addressed (paragraph-Implementation > Step N)`. |
| Verifier reports findings | Pass/fail body with evidence paths | **Success**: one `complete (doc)` overall (E2E commonly spans multiple steps, so a single overall pass is reported once). **Failure**: one `escalating (paragraph-Implementation > Step N)` per affected step. | Each fail / suggested-fix is a `COMMENT(verifier): <category> <body>` marker (category = impl bug / test gap / spec issue) **at `paragraph-Implementation > Step N`** — the marker location matches the `escalating(...)` pointer per the protocol invariant. Spec-issue cross-references (e.g. `cf. paragraph-Specification > <…>`) live inside the marker body, not in the marker placement. |
| Step 5 user-feedback revision loop (COMMENT routing) | `Reviewer feedback: ...` style routing per role | `ready (<file>:<line>)` to Programmer/Tester for source/test markers; design-doc markers handled by Director directly with no member route | The classification rules in `skills/design-doc-execute/SKILL.md` Step 5's *Revision Loop* are unchanged; only the cafleet body shrinks. |
| Step 5 Abort Flow signal to members | (was an instruction in the body) | `ready (doc)` after the Director writes `COMMENT(director): aborting — finalize and stand by` near the top of the doc body (no heading anchor needed — `Status:` and the other metadata lines are bold-prefixed metadata, not headings, so they are not valid `paragraph-` targets). | (the Abort Flow's git commit + `cafleet member delete` mechanics are unchanged) |
| Director routes source-anchored Copilot inline review (Step 7c) | `Copilot review: <file>:<line> — <body>. Please address.` | `ready (<file>:<line>)` | The Director writes `COMMENT(copilot): <body>` **in the source file** at `<file>:<line>`. The routed member responds with `addressed (<file>:<line>)` after fixing the source and removing the marker. (The design doc is NOT touched for source-anchored Copilot routing — the issue lives where the line lives.) |
| Director handles design-doc-anchored Copilot review (Step 7c) | (would have been a route to Director-direct via the existing `Director resolves directly` rule) | (no cafleet message — Director resolves silently via git commit + marker removal) | The Director writes `COMMENT(director): <body>` inline at the affected paragraph in the design doc, applies the spec change, and removes the marker as part of the fix. PR-level (non-line-anchored) Copilot reviews follow Director judgment as today. |

**Phase 1.5 (FIXME resolution)** is brought into the schema. The Programmer reads/fixes FIXMEs as today (replacing each `FIXME(claude)` with `DONE(claude)`), then sends `complete (doc)` once all FIXMEs are converted. After Director confirmation (`ready (doc)`), the Programmer strips `DONE(claude)` comments and sends `complete (doc)` again. No `COMMENT(role)` markers are added — the FIXME → DONE → removed sweep is its own self-contained inline trail.

**Phase 1 (Tester Test-Framework Selection)** is brought into the schema. If the framework is deterministically detected, the Tester proceeds silently to Phase 2 — no cafleet message. If the framework is ambiguous, the Tester writes `COMMENT(tester): framework selection ambiguous — found <evidence>; need user arbitration` **at the doc-top anchor** (directly under the metadata block, before the first heading — see coordination.md's canonical placement for the `doc` pointer) and sends `blocked (doc)` — the cafleet pointer (`doc`) matches the marker location (doc-top) per the pointer-marker pairing rule. The Director relays via `AskUserQuestion`, writes the answer back as `COMMENT(claude): <choice>` at the same doc-top anchor, sends `ready (doc)`, and the Tester resumes.

**Phase 1 (Verifier Tool Discovery)** is **exempt** from the schema. The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination. The same precedent applies as the Analyzer's question list in `/design-doc-interview`: a one-shot deliverable rides as a multi-line cafleet body. Subsequent Verifier messages (Phase 2 verification reports) follow the schema.

#### /design-doc-interview (cosmetic alignment only)

The interview skill already uses inline `COMMENT(claude)` markers as the medium and already keeps cafleet messages compact. The Director-Analyzer messages are **explicitly exempt** from the verb + pointer schema: the Analyzer's reply is a numbered question list (a one-time payload deliverable, not iterative coordination), and the Director's relay back to the user goes through `AskUserQuestion`, not cafleet at all. The only doc change is the SKILL.md `## Additional resources` block: add a link to `skills/design-doc/coordination.md` and state the exemption explicitly.

### Edge Cases

#### Copilot Routing

Copilot reviews split into two line-anchored classes plus a PR-level fallback:

| Anchor | Where the marker lives | cafleet message | Resolver |
|:--|:--|:--|:--|
| Source file (e.g. `cafleet/.../foo.py:42`) | `COMMENT(copilot): <body>` in the source file at `<file>:<line>` | `ready (<file>:<line>)` to Programmer or Tester per the existing path-pattern routing | Routed member fixes source, removes marker, replies `addressed (<file>:<line>)` |
| Design doc (e.g. `design-docs/foo/design-doc.md:42`) | `COMMENT(director): <body>` inline at the affected paragraph in the design doc | (no cafleet route — Director resolves directly per the existing rule) | Director applies the spec change and removes the marker. **No self-note cafleet message is sent** — the git commit and marker removal are sufficient audit trail. |
| PR-level (non-line-anchored) | Director judgment per existing classification (spec → `COMMENT(director)` in design doc; impl → `COMMENT(copilot)` at a representative file:line; test → `COMMENT(copilot)` at a representative test file:line) | `ready (...)` per anchor | per anchor |

The Director's commit message follows the existing convention (`fix: address Copilot review - <short summary>`); the message text contains the summary, not the `COMMENT(copilot)` body (which would be gone from source by then).

#### Finalize-Time Cleanup

When the design doc moves to `Status: Approved` (`/design-doc-create` Step 6) or `Status: Complete` (`/design-doc-execute` Step 8):

1. Issue markers (`COMMENT(role)` for `reviewer`, `director`, `drafter`, `programmer`, `tester`, `verifier`, `claude`) MUST already be resolved per `skills/design-doc/guidelines.md` § *Completeness Check* — the existing rule "No `COMMENT(` markers remain" stays.
2. Status markers do not exist in the design doc by construction (Option II split), so there is nothing to strip.
3. `COMMENT(copilot)` markers in source files are removed by the routed member as part of each fix commit; finalize-time validation only needs to confirm the design doc is marker-free.

The audit trail of every status hop lives in the cafleet message log (admin WebUI timeline) and in git history. The design document itself reads as the current state, not an archaeological dig.

#### Anchorless Status

A member may need to communicate something that does not point at any heading, file, or doc — e.g. "I crashed and restarted," "still working, no progress yet," a generic ping ack. Per Q8 (Option P):

- These ride as a pure cafleet message with a freeform short phrase. The set is **NOT a fixed canonical list** — members may use whatever short phrase fits ("restarted", "still working", "ack", "noted", etc.).
- They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses) and the Director MUST treat them as informational, not as work signals.
- They do NOT add markers to the design doc.

If a member finds themselves needing to send anchorless status updates frequently, that is a stall signal — the Director's stall-response ladder (per `skills/cafleet/SKILL.md` and the existing role files) still applies.

#### Director's Per-File Detail Recovery

Today's commit messages sometimes need per-file or per-test detail (e.g. "feat: add CLI flag --foo to bar.py and bar_test.py"). Under the new scheme, members no longer ship file lists in cafleet bodies. The Director recovers per-file detail directly via git: `git status` for unstaged/staged file lists, `git diff --stat <base>..HEAD` for cumulative scope, `git log <base>..HEAD --name-only` for file-touch history, `git diff <base>..HEAD -- <pattern>` for content. This applies in `/design-doc-execute` Phase A (test commits), Phase B/C (impl commits), Phase 7d (Copilot fix commits), and Step 8 (finalize commit).

### Out of Scope

- No changes to `cafleet/` source code, the `cafleet` CLI, the message-broker schema, or the agent-team-monitoring/-supervision skills.
- No changes to `ARCHITECTURE.md`, `README.md`, or `docs/` — none of those reference the cafleet message body conventions today (verified by grep before implementation; if a reference is found, scope is extended in Step 5 below).
- No changes to the existing `permissions.allow` posture: cafleet message bodies are free-form text and the new format is a convention, not enforced by the broker.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-05-08T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Write the shared coordination reference

- [x] Create `skills/design-doc/coordination.md` containing **mechanics only**: Core Principle, Verb Vocabulary table (6 rows + the `complete` vs `addressed` rule + Director-stall-nudge rule), Pointer Forms table (3 rows + ` > ` separator rule), Message Format with examples + the four constraints (single-line, ~80-codepoint cap, ≤3-item enumeration, no payloads), COMMENT(role) Marker section (8-row role table + rules + issue-vs-status split), Copilot Routing edge case (3-row table for source / design-doc / PR-level), Anchorless Status edge case (freeform short phrase, not a fixed list), Finalize-Time Cleanup edge case, Director Per-File Detail Recovery edge case. Rationale and tradeoffs do NOT live in this file — add a leading line "Rationale-of-record: design-docs/0000050-design-doc-as-medium/design-doc.md." Mirror the wording in *Specification* above. <!-- completed: 2026-05-08T13:30 -->
- [x] Update `skills/design-doc/SKILL.md` `## Additional resources` block to add a third bullet linking to `coordination.md` ("For the inter-agent coordination protocol used by `/design-doc-create`, `/design-doc-execute`, and `/design-doc-interview`, see [coordination.md](coordination.md)"). <!-- completed: 2026-05-08T13:30 -->

### Step 2: Update /design-doc-create

- [x] Update `skills/design-doc-create/SKILL.md`: (a) tighten Step 0 resume-detection grep from `COMMENT(` to `COMMENT(claude)` and add a one-sentence inline note explaining why; (b) replace every long-form `--text "..."` example in Steps 3–6 with the verb + pointer schema; (c) leave Step 2's clarification-relay body free-form (per Clarification Exemption) and add a one-sentence inline note explaining why; (d) add a `## Coordination Protocol` subsection (3–5 lines) pointing to `../design-doc/coordination.md`; (e) update Step 3 "Route to Reviewer" / "Route to Drafter" / "Route Drafter feedback" cafleet examples to use `ready (doc)` / `complete (doc)` / `addressed (doc)` / `approved (doc)` per the per-skill table above. <!-- completed: 2026-05-08T13:40 -->
- [x] Update `skills/design-doc-create/roles/director.md`: rewrite the Skill-specific milestones table's Director-action column to use the new schema (each nudge becomes `ready (doc)` or `ready (paragraph-...)`); rewrite the COMMENT Marker Handling section to mention the role taxonomy (claude / director / reviewer / drafter); add a one-sentence note that Step 2 clarification messages remain free-form per the Clarification Exemption; add a "Coordination Protocol" link to `../../design-doc/coordination.md` near the top of the Communication Protocol section. <!-- completed: 2026-05-08T13:40 -->
- [x] Update `skills/design-doc-create/roles/drafter.md`: in the Workflow section, replace "Report completion to the Director via `cafleet message send`" wording with "Send `complete (doc)` for fresh drafts, `addressed (doc)` for revision rounds (resolving `COMMENT(reviewer)` markers), and `blocked (doc)` with a `COMMENT(drafter)` marker if the spec is ambiguous"; in the COMMENT Processing section, **remove** the existing "Summarize what was changed in your `cafleet message send` report to the Director" bullet (the per-section diff is recoverable from `git diff`; the cafleet body is just `addressed (doc)`); expand the multi-role marker list (`reviewer`, `director`, `claude`); add a link to `../../design-doc/coordination.md`. <!-- completed: 2026-05-08T13:40 -->
- [x] Update `skills/design-doc-create/roles/reviewer.md`: rewrite the Review Process section to direct the Reviewer to write each finding as a `COMMENT(reviewer): [TAG] <body>` marker inline at the offending section, and to send only `complete (doc) — N issues` (or `approved (doc)`) over cafleet; remove the directive to format multi-paragraph bullet feedback in the message body; preserve the 5-tag taxonomy (`[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]`) — it lives inside the marker body now; add a link to `../../design-doc/coordination.md`. <!-- completed: 2026-05-08T13:40 -->

### Step 3: Update /design-doc-execute

- [x] Update `skills/design-doc-execute/SKILL.md`: replace every long-form `--text "..."` example in Steps 3–7 with the verb + pointer schema; cover Phase A / B / C / D assignments, escalation lines, **Step 5 (User Approval revision loop) COMMENT routing**, **Step 5 Abort Flow signal**, **Step 7c source-anchored Copilot routing**, **Step 7c design-doc-anchored Copilot routing** (Director resolves directly via `COMMENT(director)` marker), and **Director arbitration after Programmer escalation** (`ready (paragraph-...)` + `COMMENT(director)` decision); add a `## Coordination Protocol` subsection pointing to `../design-doc/coordination.md`; add the Verifier Phase 1 exemption note in the Verifier spawn-prompt section. <!-- completed: 2026-05-08T13:55 -->
- [x] Update `skills/design-doc-execute/roles/director.md`: rewrite the Skill-specific milestones table's Director-action column to use the new schema (each nudge becomes `ready (paragraph-...)`); in the User Interaction Rules section, expand COMMENT Classification to enumerate the role taxonomy (programmer / tester / verifier / director / copilot / claude / drafter is N/A here) and where each marker class lives (design doc vs source file); document the *Director's Per-File Detail Recovery* mechanism (git status / git diff --stat / git log --name-only) for commit-message construction; add a link to `../../design-doc/coordination.md`. <!-- completed: 2026-05-08T13:55 -->
- [x] Update `skills/design-doc-execute/roles/programmer.md`: rewrite Phase 2 step 8 ("Message the Director via `cafleet message send` when the step is complete, including: What you implemented; Which files were changed; Test results (all passing)") to "Send `complete (paragraph-Implementation > Step N)` (optional summary: ` — <count> tests pass`, ≤80 codepoints, ≤3-item enumeration). Per-file or per-test detail is NOT in the body — the Director recovers it via `git status` / `git diff --stat`."; rewrite the Escalation block to direct the Programmer to add a `COMMENT(programmer)` marker at the relevant paragraph then send `escalating (paragraph-Implementation > Step N)`; add explicit guidance that the Programmer locates Phase B test files via `git log <base>..HEAD --name-only -- '**/test_*' '**/tests/**'` (the prior Tester `complete` summary went Tester → Director, not Tester → Programmer); add a `## Coordination Protocol` link to `../../design-doc/coordination.md`; bring Phase 1.5 reporting into the schema — replace "Message the Director via `cafleet message send` with a summary of all changes made; List the DONE(claude) comments and their locations" with `complete (doc)` (DONE(claude) comments are the inline trail). <!-- completed: 2026-05-08T13:55 -->
- [x] Update `skills/design-doc-execute/roles/tester.md`: rewrite Phase 2 step 3 (the multi-bullet "Report to the Director" content) to "Send `complete (paragraph-Implementation > Step N) — <count> tests`; record gaps as `COMMENT(tester)` markers at the affected paragraph if the spec is unclear, then send `blocked (paragraph-Implementation > Step N)` instead"; rewrite Phase 3 to use `addressed` / `escalating` verbs with paragraph pointers; bring Phase 1 framework selection into the schema — deterministic detection produces no message; ambiguous detection produces `COMMENT(tester): framework selection ambiguous — found <evidence>; need user arbitration` at doc-top + `blocked (doc)` (per coordination.md's `doc` pointer canonical placement — the pairing rule rejects placing the marker under `paragraph-Implementation` while sending `blocked (doc)`); add a link to `../../design-doc/coordination.md`. <!-- completed: 2026-05-08T13:55 -->
- [x] Update `skills/design-doc-execute/roles/verifier.md`: rewrite Phase 2 step 4's reporting block to direct the Verifier to write each fail / suggested-fix as a `COMMENT(verifier): <category> <body>` marker (category = impl bug / test gap / spec issue) at the relevant paragraph; on overall success send a single `complete (doc)`; on failures send one `escalating (paragraph-Implementation > Step N)` per affected step (E2E commonly spans multiple steps, so success is reported once at doc-level and failures route per offending step); **explicitly exempt** Phase 1 tool-discovery from the schema with a one-sentence rationale (one-time discovery payload, same precedent as the Analyzer's question list); add a link to `../../design-doc/coordination.md`. <!-- completed: 2026-05-08T13:55 -->

### Step 4: Cosmetic alignment for /design-doc-interview

- [x] Update `skills/design-doc-interview/SKILL.md` `## Additional resources` block to add a bullet linking to `../design-doc/coordination.md` ("For the verb + pointer + COMMENT(role) protocol shared with `/design-doc-create` and `/design-doc-execute`, see [../design-doc/coordination.md](../design-doc/coordination.md)") AND state explicitly: "Director-Analyzer cafleet messages in this skill are exempt from the verb + pointer schema. The Analyzer's question list is a one-time payload deliverable; the Director's user-facing relay goes through `AskUserQuestion`, not cafleet." No other changes. <!-- completed: 2026-05-08T13:58 -->
- [x] Verify `skills/design-doc-interview/roles/analyzer.md` requires no edits (the Analyzer's deliverable is the question list itself, which is a payload by design and outside the verb + pointer schema, per the explicit exemption above). Document the verification in the commit message; do not edit the file. <!-- completed: 2026-05-08T13:58 -->

### Step 5: Verification

- [x] Run the negative-grep verification table below. Each row's command MUST return 0 hits. Any non-zero hit indicates a stale long-form payload still in the role files; resolve by editing the offending file before continuing.

  | Check | Command | On-fail action |
  |:--|:--|:--|
  | Reviewer feedback prelude gone | `rg "Reviewer feedback: " skills/` | Edit the offending role/SKILL file to use `ready (doc)`. |
  | Tester `Tests at:` payload gone | `rg "Tests at: " skills/` | Edit Phase B assignment to use `ready (paragraph-Implementation > Step N)` and direct Programmer to git log. |
  | Programmer "all passing" payload gone | `rg "Test results \(all passing\)" skills/` | Edit Phase 2 step 8 to use `complete (paragraph-Implementation > Step N)`. |
  | Tester multi-bullet summary gone | `rg "What tests you wrote" skills/` | Edit Phase 2 step 3 to use `complete (paragraph-Implementation > Step N) — <count> tests`. |
  <!-- completed: 2026-05-08T14:00 -->
- [x] Run the top-level docs scope check. Both commands MUST return 0 hits; if either has hits, scope is extended to update those files within Step 5. The `docs/` glob is constrained to top-level only — do **NOT** descend into `**/skills/*/docs/` because skill-internal docs are part of the skill change surface and are already covered by Steps 1–4.

  | Check | Command | On-fail action |
  |:--|:--|:--|
  | Skills don't reference top-level docs surfaces | `rg -n "ARCHITECTURE\.md|README\.md|^docs/" skills/design-doc skills/design-doc-create skills/design-doc-execute skills/design-doc-interview` | Update the offending top-level surface. |
  | Top-level docs don't carry cafleet body conventions | `rg -n "cafleet message send" ARCHITECTURE.md README.md docs/` (constrained to direct top-level `docs/` only — do NOT include `**/skills/**/docs/`) | Update the offending top-level surface. |
  <!-- completed: 2026-05-08T14:00 -->
- [x] Stage every changed file under `skills/design-doc*/` plus `design-docs/0000050-design-doc-as-medium/design-doc.md`, and commit per the project's `.claude/rules/git-workflow.md` (single-line message, `docs:` prefix, no HEREDOC, no `git -C`). The commit message body is not used; the design doc is the historical record. <!-- completed: 2026-05-08T14:00 -->

### Step 6: Post-Approval Consolidation (User Feedback)

- [x] Update `skills/design-doc-create/roles/drafter.md`: replace the `## COMMENT Processing` section with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Remove the duplicated bullet list. Keep `## Resume Mode` (skill-specific). <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-create/roles/reviewer.md`: replace the bare role table / marker rules with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Keep the `[COMPLIANCE]/[GAP]/[UNCLEAR]/[INCORRECT]/[IMPROVEMENT]` 5-tag table (skill-specific) and the `complete (doc) — N issues` / `approved (doc)` send pattern. <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-create/roles/director.md`: replace the COMMENT-Marker-Handling / role-taxonomy paragraphs with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Keep the milestones table (skill-specific) and the Clarification Exemption note. <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-execute/roles/programmer.md`: replace any duplicated COMMENT-routing / marker-format prose with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Keep Phase 1.5 (FIXME → DONE → cleanup sweep, skill-specific) and the Phase 2 git-log test-file-locator note. <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-execute/roles/tester.md`: replace duplicated COMMENT-routing / gap-marker prose with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Keep Phase 1 framework-selection ambiguity → `COMMENT(tester)` + `blocked (doc)` (skill-specific) and Phase 3 defect-resolution flow. <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-execute/roles/verifier.md`: replace the duplicated `COMMENT(verifier): <category> <body>` rules with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker*. Keep the Phase 1 tool-discovery exemption (skill-specific) and the success-vs-failure reporting policy (`complete (doc)` overall vs per-step `escalating`). <!-- completed: 2026-05-08T14:05 -->
- [x] Update `skills/design-doc-execute/roles/director.md`: replace the COMMENT-Classification / role-taxonomy / marker-location paragraphs with a one-line link to `../../design-doc/coordination.md` § *COMMENT(role) Marker* + § *Copilot Routing* + § *Director Per-File Detail Recovery*. Keep the milestones table (skill-specific) and the Per-File Detail Recovery cite to git plumbing. <!-- completed: 2026-05-08T14:05 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-08 | Initial draft. |
| 2026-05-08 | Round-1 Reviewer revision: addressed all 25 items (4 INCORRECT, 10 GAP, 7 UNCLEAR, 4 IMPROVEMENT). Key changes: Progress 0/14 → 0/16; clarification flow exempted from schema; Step 0 resume-detection grep tightened to `COMMENT(claude)`; Tester completion summary standardised on `— <count> tests`; nesting separator changed from `/` to ` > ` to avoid heading-text collision; `complete` (fresh deliverables) vs `addressed` (marker resolution) policy added; cafleet-body single-line + ~80-codepoint + ≤3-item-enumeration constraints added; Director stall-nudge mapping added (`ready (...)` interpreted contextually); Step 5 user-feedback revision loop and Abort Flow added to /design-doc-execute table; Director arbitration row added after Programmer escalation; Programmer Phase 1.5, Tester Phase 1, and Verifier Phase 1 reporting addressed (1.5 + Tester P1 in-schema, Verifier P1 exempted with rationale); design-doc-anchored Copilot routing added (`COMMENT(director)` direct, no member route); drafter.md "summarize what was changed" bullet flagged for removal; Programmer test-file location via `git log` documented; Director per-file detail recovery via `git status` / `git diff --stat` documented; coordination.md scope clarified (mechanics-only; design doc 0000050 is rationale-of-record); anchorless status set declared freeform (not fixed); /design-doc-interview Director-Analyzer exemption made explicit; Step 5 task 2 restructured into table format with constrained `docs/` scope. |
| 2026-05-08 | Round-2 Reviewer revision: addressed all 5 minor items. R2.1 — Step 5 ripgrep alternation pattern fixed (dropped backslash before `|`; ripgrep's default Rust regex treats `\|` as a literal pipe). R2.2 — `paragraph-Status` pointer dropped (`Status:` is bold metadata, not a heading); Abort Flow now uses `doc` pointer with the `COMMENT(director)` marker placed near the top of the doc body. R2.3 — Verifier policy disambiguated: success reported as a single overall `complete (doc)`; failures routed per-affected-step via `escalating (paragraph-Implementation > Step N)`. R2.4 — Summary cap rephrased as a soft target: SHOULD fit on one terminal line; aim for ≤ 80 codepoints. R2.5 — Director self-note for design-doc-anchored Copilot resolution dropped (git commit + marker removal is sufficient audit trail). |
