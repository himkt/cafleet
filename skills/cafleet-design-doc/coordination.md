Rationale-of-record: design-docs/0000050-design-doc-as-medium/design-doc.md.

# Coordination Protocol

Mechanics for inter-agent coordination. The design document is the substantive communication medium; `cafleet message send --text` carries only a single-line **verb + pointer** poke. Substantive content (feedback, reports, escalation reasons, review items) lives in inline `COMMENT(role)` markers in the design doc — except for source-anchored Copilot inline review, which is annotated in the source file at `<file>:<line>` because that is where the comment lives.

**Scope.** The verb + pointer schema applies to the `cafleet-design-doc-create` and `cafleet-design-doc-execute` skills. The `cafleet-design-doc-interview` skill shares only the inline `COMMENT(role)` marker convention (specifically `COMMENT(claude)`) — its Director-Analyzer cafleet messages are explicitly exempt because the Analyzer's question-list deliverable is a multi-line payload.

## Core Principle

`cafleet message send` is a poke, not a payload. Status hops travel as compact `<verb> (<pointer>)` lines on the broker; reasoning, findings, and item-by-item routing live as `COMMENT(role)` markers inside the design document (or, for source-anchored Copilot, the source file). Anchorless meta-events (member crashed/restarted, generic ping ack, "still working") do NOT use a verb from the canonical list and do NOT touch the design doc.

## Verb Vocabulary

The canonical set is exactly 6. Members and the Director MUST pick from this list and MUST NOT invent new verbs.

| Verb | Sender direction | Meaning | Used for |
|:--|:--|:--|:--|
| `ready` | Director → member, or member → Director | "The pointer target is ready for you to read / act on." | Fresh assignments, member-to-Director "I have something for you to look at," and Director stall-nudges (the recipient interprets contextually — see below). |
| `complete` | Member → Director | "I finished a fresh deliverable at the pointer target." | Initial drafts, freshly written tests, freshly written implementation, FIXME-resolution sweeps. |
| `addressed` | Member → Director, or Director → Director (self-note) | "I resolved a pre-existing marker (a `COMMENT(role)` marker, a Copilot inline comment, or a Director-arbitration note) at the pointer." | Round-2+ work on items already flagged in the doc or in source. |
| `blocked` | Member → Director | "I cannot proceed at the pointer; the blocker rationale is in a `COMMENT(role)` marker at the same pointer." | Spec ambiguity, missing deps, environmental issues. |
| `escalating` | Member → Director | "I am escalating an issue (e.g. a suspected test defect) at the pointer; the rationale is in a `COMMENT(role)` marker at the same pointer." | Test-defect arbitration, multi-round disagreements. |
| `approved` | Reviewer → Director, or Director → user-result | "All quality criteria are met at the pointer (typically `doc`)." | Reviewer approval signal in the `cafleet-design-doc-create` skill. |

**Verb choice for `complete` vs `addressed`**: `complete` signals a fresh deliverable (work that did not previously have a marker waiting). `addressed` signals resolution of a pre-existing marker (any `COMMENT(role)`, any Copilot line, any Director arbitration). When in doubt, ask: "did a marker exist before I started this turn?" — if yes, use `addressed`; if no, use `complete`.

**Director stall-nudges** reuse `ready (doc)` or `ready (paragraph-...)` — they are `ready` from the Director's perspective ("the pointer target is ready for you, please act"). The recipient interprets contextually: on receiving `ready (...)`, scan the pointer target for any `COMMENT(role)` markers addressed to your role or relevant to your current phase, and act accordingly. Re-sent `ready (...)` after a member's idle window is a nudge, not a new assignment — same target, same expected action.

## Pointer Forms

Exactly 3 canonical forms. Use the tightest one that locates the target.

| Form | Example | When to use |
|:--|:--|:--|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. Use the literal heading text. Nest with the three-character separator ` > ` (space, greater-than, space). Heading text is preserved verbatim — slashes, colons, hyphens, and other punctuation inside a heading remain literal. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `cafleet/src/cafleet/cli/main.py:142` | The target is a specific line (or range) in a source file, test file, or the design doc itself. Used for source-anchored Copilot inline review and for source-file `COMMENT` markers added during code review. |
| `doc` | `doc` | The target is the design document as a whole (e.g., Reviewer signalling overall approval, Drafter signalling the full draft is ready). |

**Pointer-marker pairing rule.** When a verb's spec requires a paired `COMMENT(role)` marker (`blocked` / `escalating`, also Director arbitration replies and `COMMENT(copilot)` placements), the marker MUST live at the SAME pointer as the cafleet body:

| Pointer | Canonical marker placement |
|:--|:--|
| `paragraph-<HeadingPath>` | Inline within that heading's section. |
| `<file>:<line>` | At that exact line in the file (immediately above or on `<line>` per the file's native comment syntax). |
| `doc` | Doc-top — directly under the metadata block (`Status:` / `Progress:` / `Last Updated:`), before the first heading. |

The ` > ` separator avoids the collision that would arise if `/` were used as a nesting separator (heading text in real-world design docs frequently contains `/`, e.g. `Step 2: Update docs/spec/cli-options.md`). ` > ` is unambiguous, ASCII-safe, and shell-safe inside double-quoted `--text` arguments.

## Message Format

Every `cafleet message send --text` body, when used to coordinate within a `cafleet-design-doc-create` or `cafleet-design-doc-execute` skill team, MUST match:

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

## COMMENT(role) Marker

Inline marker placed in the design document (or, for source-anchored Copilot inline review, in the source file at `file:line`).

```
COMMENT(<role>): <substantive content>
```

Roles:

| Role | Who writes it | When |
|:--|:--|:--|
| `claude` | The Director acting as user-mediator (existing convention from the `cafleet-design-doc-interview` skill) | Carries user-derived clarifications. Existing usage unchanged. |
| `director` | The Director | Spec resolution notes, Director judgments, ambiguity arbitration, design-doc-anchored Copilot review (see *Copilot Routing* below), Phase C code-review feedback in the `cafleet-design-doc-execute` skill. |
| `drafter` | The Drafter | Spec ambiguities the Drafter cannot resolve and needs Director input on. |
| `reviewer` | The Reviewer | Review findings, tagged with the existing `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` taxonomy inside the marker body. |
| `programmer` | The Programmer | Implementation-side notes, escalation rationales, observations of spec gaps that block implementation. |
| `tester` | The Tester | Test-spec gaps (Phase 2 and Phase 1 framework-selection), escalation rationales, evaluation of Programmer-routed test-defect reports. |
| `verifier` | The Verifier | E2E findings, evidence pointers (`see file:line`), suggested-fix categorisation (impl bug / test gap / spec issue). |
| `copilot` | The Director on Copilot's behalf (Copilot does not edit files) | One marker per source-anchored inline review item, written into the **source file** at `<file>:<line>`. Design-doc-anchored Copilot lines route through `COMMENT(director)` instead — see *Copilot Routing*. |

Rules:

- One marker per logical issue. Do not bundle.
- Body must be actionable — state the issue and what should change.
- Reviewer markers carry one of the 5 review tags inside the body: `COMMENT(reviewer): [GAP] Step 4 lacks a Phase D entry.`
- Markers are split into two classes — *issue* and *status*. Only the *issue* class enters the doc.

## Issue Markers vs Status Markers (split)

`COMMENT(role)` markers carry **issue and feedback content only**. Status updates ("ready", "complete") stay as pure cafleet messages with verb + pointer; they do NOT add a marker to the design doc.

| Class | Example | Lifecycle |
|:--|:--|:--|
| Issue | `COMMENT(reviewer): [GAP] Specification omits the retry budget.` | Persists in the doc until resolved. The resolver removes the marker as part of the fix. Per `skills/design-doc/guidelines.md` § *Completeness Check*, the doc cannot reach `Status: Approved` / `Status: Complete` while any `COMMENT(` marker remains. |
| Status | (none — never enters the doc) | Lives only in `cafleet message send` text. |

This keeps the design doc clean: at any moment, the markers in the doc reflect *outstanding work*, never historical chatter.

## Copilot Routing

Copilot reviews split into two line-anchored classes (source file, design doc) plus a PR-level catch-all:

| Anchor | Where the marker lives | cafleet message | Resolver |
|:--|:--|:--|:--|
| Source file (e.g. `cafleet/.../foo.py:42`) | `COMMENT(copilot): <body>` in the source file at `<file>:<line>` | `ready (<file>:<line>)` to Programmer or Tester per the existing path-pattern routing | Routed member fixes source, removes marker, replies `addressed (<file>:<line>)` |
| Design doc (e.g. `design-docs/foo/design-doc.md:42`) | `COMMENT(director): <body>` inline at the affected paragraph in the design doc | (no cafleet route — Director resolves directly per the existing rule) | Director applies the spec change and removes the marker. **No self-note cafleet message is sent** — the git commit and marker removal are sufficient audit trail. |
| PR-level (non-line-anchored) | Director judgment per existing classification (spec → `COMMENT(director)` in design doc; impl → `COMMENT(copilot)` at a representative file:line; test → `COMMENT(copilot)` at a representative test file:line) | `ready (...)` per anchor | per anchor |

The Director's commit message follows the existing convention (`fix: address Copilot review - <short summary>`); the message text contains the summary, not the `COMMENT(copilot)` body (which would be gone from source by then).

## Anchorless Status

A member may need to communicate something that does not point at any heading, file, or doc — e.g. "I crashed and restarted," "still working, no progress yet," a generic ping ack.

- These ride as a pure cafleet message with a freeform short phrase. The set is **NOT a fixed canonical list** — members may use whatever short phrase fits ("restarted", "still working", "ack", "noted", etc.).
- They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses) and the Director MUST treat them as informational, not as work signals.
- They do NOT add markers to the design doc.

If a member finds themselves needing to send anchorless status updates frequently, that is a stall signal — the Director's stall-response ladder (per `skills/cafleet/SKILL.md` and the existing role files) still applies.

## Finalize-Time Cleanup

When the design doc moves to `Status: Approved` (the `cafleet-design-doc-create` skill Step 6) or `Status: Complete` (the `cafleet-design-doc-execute` skill Step 8):

1. Issue markers (`COMMENT(role)` for `reviewer`, `director`, `drafter`, `programmer`, `tester`, `verifier`, `claude`) MUST already be resolved per `skills/cafleet-design-doc/guidelines.md` § *Completeness Check* — the existing rule "No `COMMENT(` markers remain" stays.
2. Status markers do not exist in the design doc by construction (split), so there is nothing to strip.
3. `COMMENT(copilot)` markers in source files are removed by the routed member as part of each fix commit; finalize-time validation only needs to confirm the design doc is marker-free.

The audit trail of every status hop lives in the cafleet message log (admin WebUI timeline) and in git history. The design document itself reads as the current state, not an archaeological dig.

## Director Per-File Detail Recovery

Members no longer ship file lists in cafleet bodies. The Director recovers per-file detail directly via git when a commit message needs it: `git status` for unstaged/staged file lists, `git diff --stat <base>..HEAD` for cumulative scope, `git log <base>..HEAD --name-only` for file-touch history, `git diff <base>..HEAD -- <pattern>` for content. This applies in the `cafleet-design-doc-execute` skill's Phase A (test commits), Phase B/C (impl commits), Phase 7d (Copilot fix commits), and Step 8 (finalize commit).
