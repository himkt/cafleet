# Coordination Protocol

Mechanics for inter-member coordination. The `cafleet message send` body carries only a single-line **verb + pointer** poke. Substantive content (feedback, reports, escalation reasons, review items) lives in inline `COMMENT(role)` markers in the design doc, or in the source file at `<file>:<line>` for a source-anchored review finding.

## Payload exemptions

Create and execute coordination uses the verb/pointer schema, except that these exchanges use free-form complete payloads:

| Exchange | Producer and receiver | Boundary |
|---|---|---|
| Create pre-draft clarification | Drafter questions and Director user-answer relay | Before design-document content exists; normal coordination resumes with the first draft. |
| Execute Verifier tool discovery | Verifier's initial tool-and-MCP inventory to Director | First substantive payload after ready; later verification reports use ordinary paired markers. |

Retrieve and send these payloads complete under the cafleet skill's [Send and Poll rules](../../cafleet/SKILL.md#send-unicast) (`--json` reads, `--file` for long bodies), then ACK the consumed delivery.

Before drafting, fresh-Drafter clarification may write hidden BASE-rooted broker-payload artifacts that hold questions, not design-document content, under the BASE write guards and explicit-output-path rules. The clarification gate stands: receive answers before creating or writing the design document.

## Verb Vocabulary

Members and the Director MUST use exactly these 6 verbs and invent no others.

| Verb | Direction | Meaning | Used for |
|---|---|---|---|
| `ready` | Director ↔ member | The pointer target is ready to read or act on. | Fresh assignments, a member's "something to look at", Director stall-nudges |
| `complete` | Member → Director | A fresh deliverable is at the pointer. | Initial drafts, fresh tests or implementation, FIXME-resolution sweeps |
| `addressed` | Member → Director | A pre-existing marker (`COMMENT(role)` or Director arbitration note) at the pointer is resolved, or the finalize assignment is done. | Round-2+ work on items already flagged in the doc or source |
| `blocked` | Member → Director | Cannot proceed at the pointer; the rationale is a `COMMENT(role)` marker there. | Spec ambiguity, missing deps, environmental issues |
| `escalating` | Member → Director | Escalating an issue (e.g. a suspected test defect) at the pointer; the rationale is a `COMMENT(role)` marker there. | Test-defect arbitration, multi-round disagreements |
| `approved` | Reviewer → Director | All quality criteria are met at the pointer (typically `doc`). | Reviewer approval in both workflows |

Use `addressed` when a marker existed before your turn began or you are completing a finalize assignment; otherwise use `complete`.

**Director stall-nudges** reuse `ready (doc)` or `ready (paragraph-...)`: a `ready (...)` re-sent after an idle window carries the same target and expected action, not a new assignment. On any `ready (...)`, act on the pointer target's `COMMENT(role)` markers addressed to your role or relevant to your current phase.

## Pointer Forms

Use the tightest of exactly 3 canonical forms that locates the target.

| Form | Example | When to use |
|---|---|---|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `src/cli/member.rs:142` | The target is a line (or range) in a source file, test file or the design doc itself, as for source-anchored review findings and code-review source-file `COMMENT` markers. |
| `doc` | `doc` | The target is the whole design document (e.g. Reviewer approval, or a Drafter's full draft being ready). |

A `paragraph-` pointer uses the literal heading text, preserved verbatim (slashes, colons, hyphens and other punctuation stay literal), and nests with the three-character separator ` > ` (space, greater-than, space).

**Pointer-marker pairing rule.** A paired `COMMENT(role)` marker — for `blocked` / `escalating`, Director arbitration replies and source-anchored `COMMENT(reviewer)` placements — MUST live at the SAME pointer as the cafleet body:

| Pointer | Canonical marker placement |
|---|---|
| `paragraph-<HeadingPath>` | Inline within that heading's section. |
| `<file>:<line>` | At that exact line in the file (immediately above or on `<line>` per the file's native comment syntax). |
| `doc` | Doc-top — directly under the metadata block (`Status:` / `Progress:` / `Last Updated:`), before the first heading. |

## Message Format

Outside [payload exemptions](#payload-exemptions) and anchorless statuses, every create/execute coordination body matches `<verb> (<pointer>)`, optionally followed by a one-line summary after ` — ` (space, em-dash, space): `<verb> (<pointer>) — <one-line summary>`.

| Constraint | Rule |
|---|---|
| Single line | The body MUST be a single line, with no literal newlines. |
| Summary cap | The optional summary SHOULD fit on one terminal line; aim for ≤ 80 codepoints. |
| Enumeration cap | The summary MUST NOT enumerate more than 3 items; longer enumerations belong in a `COMMENT(role)` marker at the pointer. |
| No payloads | The summary is for human readability in the broker timeline; substantive content (reasoning, file lists beyond 3, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. |

Examples: `ready (paragraph-Implementation > Step 1)`, `complete (paragraph-Implementation > Step 1) — 12 tests pass`, `addressed (src/cli/member.rs:142)`, `blocked (paragraph-Specification > Retry Strategy)`, `escalating (paragraph-Implementation > Step 3)`, `approved (doc)`.

## COMMENT(role) Marker

An inline marker in the design document, or in the source file at `file:line` for source-anchored review findings:

```
COMMENT(<role>): <substantive content>
```

| Role | Writer | Content |
|---|---|---|
| `user-relay` | Director as user-mediator | User-derived clarifications and ordinary-language revision requests, preserving the user's meaning |
| `director` | Director | Spec resolution notes, judgments, ambiguity arbitration, execute Step 4 code-review feedback |
| `drafter` | Drafter | Spec ambiguities it cannot resolve without Director input |
| `reviewer` | Reviewer | Create design-doc findings; execute code-review findings at `<file>:<line>` — both tagged `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` in the body |
| `programmer` | Programmer | Implementation notes, escalation rationales, spec gaps that block implementation |
| `tester` | Tester | Test-spec gaps (Phase 2 and Phase 1 framework selection), escalation rationales, evaluation of Programmer-routed test-defect reports |
| `verifier` | Verifier | E2E findings, evidence pointers (`see file:line`), suggested-fix category (impl bug / test gap / spec issue) |

Write one marker per logical issue, with an actionable body stating the issue and what should change. A reviewer body carries one of the 5 review tags: `COMMENT(reviewer): [GAP] Step 4 lacks a verification entry.`

Users may give ordinary-language feedback without markers or pointers, or write markers themselves. The Director records each request as `COMMENT(user-relay)` at the affected paragraph (or `doc` for a document-wide issue), or selects the file/line pointer for source/test feedback, and routes it through the workflow, which chooses its owner (the Programmer when no Tester exists). Ask only about ambiguous meaning or scope. Relayed and user-written feedback follows the same member role obligations, verb/pointer pairing, marker removal and approval preconditions as every other marker.

## Issue Markers vs Status Markers (split)

`COMMENT(role)` markers carry **issue and feedback content only**, such as `COMMENT(reviewer): [GAP] Specification omits the retry budget.` An issue marker persists in the doc until resolved, and the resolver removes it as part of the fix; per [Completeness Check](guidelines.md#completeness-check), the doc cannot reach `Status: Approved` / `Status: Complete` while any `COMMENT(` marker remains. Status updates ("ready", "complete") live only in `cafleet message send` verb + pointer text and never enter the design doc.

## Anchorless Status

A status that points at no heading, file or doc — a restart, "still working, no progress yet", a generic acknowledgement — rides as a pure cafleet message with whatever short phrase fits (`restarted`, `still working`, `ack`, `noted`, etc.); the set is **not a fixed canonical list**. It MUST NOT match the `<verb> (<pointer>)` schema (no parentheses), adds no marker to the design doc, and is informational for the Director, except for the spawn handshake.

The spawn handshake `ready` (optionally `ready: <recap>`) is an anchorless status, distinct from the `ready (<pointer>)` verb: the member's startup gate under the Director's Spawn Protocol, not a deliverable signal. It never carries or replaces a role's first substantive message — the Drafter still sends its clarification round as a separate message and drafts nothing before the answers arrive.

Frequent anchorless updates from a member are a stall signal under the Director's [stall-response ladder](../../cafleet/roles/director.md#stall-response).

## Finalize-Time Cleanup

The routed member removes source-file `COMMENT(reviewer)` markers as part of each fix commit, so finalize-time validation only confirms the design doc is marker-free.

## Director Per-File Detail Recovery

Cafleet bodies carry no file lists; when a commit message needs per-file detail, the Director recovers it via git — `git status`, `git diff --stat <base>..HEAD`, `git log <base>..HEAD --name-only`, `git diff <base>..HEAD -- <pattern>` — for every execute-workflow commit (test, impl, Reviewer-fix and finalize).
