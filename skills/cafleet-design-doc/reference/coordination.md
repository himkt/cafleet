# Coordination Protocol

Mechanics for inter-member coordination. The design document is the substantive communication medium; `cafleet message send --text` carries only a single-line **verb + pointer** poke. Substantive content (feedback, reports, escalation reasons, review items) lives in inline `COMMENT(role)` markers in the design doc — except for source-anchored review findings, which are annotated in the source file at `<file>:<line>` because that is where the finding lives.

**Scope.** The verb + pointer schema applies to the create and execute workflows. Two exchanges are **exempt** and ride as free-form multi-line bodies: the create workflow's **Step-2 clarification exchange** (the Drafter's clarifying questions + the Director's user-answers relay, before the design doc exists), and the interview workflow's Director-Analyzer messages (which share only the inline `COMMENT(user-relay)` marker convention — the Analyzer's question-list deliverable is a multi-line payload).

## Core Principle

`cafleet message send` is a poke, not a payload. Status hops travel as compact `<verb> (<pointer>)` lines on the broker; reasoning, findings, and item-by-item routing live as `COMMENT(role)` markers inside the design document (or, for source-anchored review findings, the source file). Anchorless meta-events (member crashed/restarted, generic ping ack, "still working") do NOT use a verb from the canonical list and do NOT touch the design doc.

## Verb Vocabulary

The canonical set is exactly 6. Members and the Director MUST pick from this list and MUST NOT invent new verbs.

| Verb | Sender direction | Meaning | Used for |
|:--|:--|:--|:--|
| `ready` | Director → member, or member → Director | "The pointer target is ready for you to read / act on." | Fresh assignments, member-to-Director "I have something for you to look at," and Director stall-nudges (the recipient interprets contextually — see below). |
| `complete` | Member → Director | "I finished a fresh deliverable at the pointer target." | Initial drafts, freshly written tests, freshly written implementation, FIXME-resolution sweeps. |
| `addressed` | Member → Director, or Director → Director (self-note) | "I resolved a pre-existing marker (a `COMMENT(role)` marker or a Director-arbitration note) at the pointer." | Round-2+ work on items already flagged in the doc or in source. |
| `blocked` | Member → Director | "I cannot proceed at the pointer; the blocker rationale is in a `COMMENT(role)` marker at the same pointer." | Spec ambiguity, missing deps, environmental issues. |
| `escalating` | Member → Director | "I am escalating an issue (e.g. a suspected test defect) at the pointer; the rationale is in a `COMMENT(role)` marker at the same pointer." | Test-defect arbitration, multi-round disagreements. |
| `approved` | Reviewer → Director, or Director → user-result | "All quality criteria are met at the pointer (typically `doc`)." | Reviewer approval signal in the create and execute workflows. |

**Verb choice for `complete` vs `addressed`**: `complete` signals a fresh deliverable (work that did not previously have a marker waiting). `addressed` signals resolution of a pre-existing marker (any `COMMENT(role)` marker, any Director arbitration). When in doubt, ask: "did a marker exist before I started this turn?" — if yes, use `addressed`; if no, use `complete`.

**Director stall-nudges** reuse `ready (doc)` or `ready (paragraph-...)` — they are `ready` from the Director's perspective ("the pointer target is ready for you, please act"). The recipient interprets contextually: on receiving `ready (...)`, scan the pointer target for any `COMMENT(role)` markers addressed to your role or relevant to your current phase, and act accordingly. Re-sent `ready (...)` after a member's idle window is a nudge, not a new assignment — same target, same expected action.

## Pointer Forms

Exactly 3 canonical forms. Use the tightest one that locates the target.

| Form | Example | When to use |
|:--|:--|:--|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. Use the literal heading text. Nest with the three-character separator ` > ` (space, greater-than, space). Heading text is preserved verbatim — slashes, colons, hyphens, and other punctuation inside a heading remain literal. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `cafleet/src/cli/member.rs:142` | The target is a specific line (or range) in a source file, test file, or the design doc itself. Used for source-anchored review findings and for source-file `COMMENT` markers added during code review. |
| `doc` | `doc` | The target is the design document as a whole (e.g., Reviewer signalling overall approval, Drafter signalling the full draft is ready). |

**Pointer-marker pairing rule.** When a verb's spec requires a paired `COMMENT(role)` marker (`blocked` / `escalating`, also Director arbitration replies and source-anchored `COMMENT(reviewer)` placements), the marker MUST live at the SAME pointer as the cafleet body:

| Pointer | Canonical marker placement |
|:--|:--|
| `paragraph-<HeadingPath>` | Inline within that heading's section. |
| `<file>:<line>` | At that exact line in the file (immediately above or on `<line>` per the file's native comment syntax). |
| `doc` | Doc-top — directly under the metadata block (`Status:` / `Progress:` / `Last Updated:`), before the first heading. |

## Message Format

Every `cafleet message send --text` body, when used to coordinate within a create or execute workflow team, MUST match:

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
| No payloads | The summary is for human readability in the broker timeline; substantive content (reasoning, file lists beyond 3, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. |

Examples:

- `ready (paragraph-Implementation > Step 1)`
- `complete (paragraph-Implementation > Step 1) — 12 tests pass`
- `addressed (cafleet/src/cli/member.rs:142)`
- `blocked (paragraph-Specification > Retry Strategy)`
- `escalating (paragraph-Implementation > Step 3)`
- `approved (doc)`

## COMMENT(role) Marker

Inline marker placed in the design document (or, for source-anchored review findings, in the source file at `file:line`).

```
COMMENT(<role>): <substantive content>
```

Roles:

| Role | Who writes it | When |
|:--|:--|:--|
| `user-relay` | The Director acting as user-mediator (existing convention from the interview workflow) | Carries user-derived clarifications. Existing usage unchanged. |
| `director` | The Director | Spec resolution notes, Director judgments, ambiguity arbitration, Phase C code-review feedback in the execute workflow. |
| `drafter` | The Drafter | Spec ambiguities the Drafter cannot resolve and needs Director input on. |
| `reviewer` | The Reviewer | Design-doc review findings in the create workflow; code-review findings at `<file>:<line>` in the execute workflow — both tagged with the existing `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` taxonomy inside the marker body. |
| `programmer` | The Programmer | Implementation-side notes, escalation rationales, observations of spec gaps that block implementation. |
| `tester` | The Tester | Test-spec gaps (Phase 2 and Phase 1 framework-selection), escalation rationales, evaluation of Programmer-routed test-defect reports. |
| `verifier` | The Verifier | E2E findings, evidence pointers (`see file:line`), suggested-fix categorisation (impl bug / test gap / spec issue). |

Rules:

- One marker per logical issue. Do not bundle.
- Body must be actionable — state the issue and what should change.
- Reviewer markers carry one of the 5 review tags inside the body: `COMMENT(reviewer): [GAP] Step 4 lacks a Phase D entry.`

## Issue Markers vs Status Markers (split)

`COMMENT(role)` markers carry **issue and feedback content only**. Status updates ("ready", "complete") stay as pure cafleet messages with verb + pointer; they do NOT add a marker to the design doc.

| Class | Example | Lifecycle |
|:--|:--|:--|
| Issue | `COMMENT(reviewer): [GAP] Specification omits the retry budget.` | Persists in the doc until resolved. The resolver removes the marker as part of the fix. Per `skills/cafleet-design-doc/reference/guidelines.md` § *Completeness Check*, the doc cannot reach `Status: Approved` / `Status: Complete` while any `COMMENT(` marker remains. |
| Status | (none — never enters the doc) | Lives only in `cafleet message send` text. |

## Anchorless Status

A member may need to communicate something that does not point at any heading, file, or doc — e.g. "I crashed and restarted," "still working, no progress yet," a generic ping ack.

- These ride as a pure cafleet message with a freeform short phrase. The set is **NOT a fixed canonical list** — members may use whatever short phrase fits ("restarted", "still working", "ack", "noted", etc.).
- They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses) and the Director MUST treat them as informational, not as work signals.
- They do NOT add markers to the design doc.

If a member finds themselves needing to send anchorless status updates frequently, that is a stall signal — the Director's stall-response ladder (per `skills/cafleet/SKILL.md` and the existing role files) still applies.

## Finalize-Time Cleanup

When the design doc moves to `Status: Approved` (the create workflow Step 6) or `Status: Complete` (the execute workflow Step 8):

1. Issue markers (`COMMENT(role)` for `reviewer`, `director`, `drafter`, `programmer`, `tester`, `verifier`, `user-relay`) MUST already be resolved per `skills/cafleet-design-doc/reference/guidelines.md` § *Completeness Check* — the existing rule "No `COMMENT(` markers remain" stays.
2. Status markers do not exist in the design doc by construction (split), so there is nothing to strip.
3. `COMMENT(reviewer)` markers in source files are removed by the routed member as part of each fix commit; finalize-time validation only needs to confirm the design doc is marker-free.

## Director Per-File Detail Recovery

Members do not ship file lists in cafleet bodies (the verb + pointer poke carries none); the Director recovers per-file detail directly via git when a commit message needs it — `git status`, `git diff --stat <base>..HEAD`, `git log <base>..HEAD --name-only`, `git diff <base>..HEAD -- <pattern>`. This applies to every commit the Director makes in the execute workflow (test, impl, Reviewer-fix, and finalize commits).
