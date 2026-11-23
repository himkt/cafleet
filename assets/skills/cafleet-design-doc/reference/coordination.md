# Coordination Protocol

A `cafleet message send` body carries only a single-line **verb + pointer** poke. Substantive content (feedback, reports, escalation reasons, review items) lives in `COMMENT(role)` markers in the design doc, or in the source file at `<file>:<line>` for a source-anchored review finding.

## Payload exemptions

These exchanges send free-form complete payloads instead of verb + pointer:

| Exchange | Producer and receiver | Boundary |
|---|---|---|
| Create pre-draft clarification | Drafter questions and Director user-answer relay | Before design-document content exists; normal coordination resumes with the first draft. |
| Execute Verifier tool discovery | Verifier's initial tool-and-MCP inventory to Director | First substantive payload after ready. |

Send and read them complete under the cafleet skill's [Send and Poll rules](../../cafleet/SKILL.md#send-unicast) (`--json` reads, `--file` for long bodies). A fresh Drafter may write clarification payloads as hidden BASE-rooted artifacts; it receives the answers before writing the design document.

## Verb Vocabulary

Use exactly these 6 verbs.

| Verb | Direction | Meaning |
|---|---|---|
| `ready` | Director ↔ member | The pointer target is ready to read or act on: a fresh assignment, something to look at, or a stall-nudge. |
| `complete` | Member → Director | A fresh deliverable (draft, tests, implementation, FIXME sweep) is at the pointer. |
| `addressed` | Member → Director | A marker that existed before your turn is resolved at the pointer, or the finalize assignment is done. |
| `blocked` | Member → Director | Cannot proceed at the pointer (spec ambiguity, missing deps, environment); the rationale is a `COMMENT(role)` marker there. |
| `escalating` | Member → Director | An issue such as a suspected test defect or a multi-round disagreement; the rationale is a `COMMENT(role)` marker there. |
| `approved` | Reviewer → Director | All quality criteria are met at the pointer, typically `doc`. |

Use `addressed` when a marker existed before your turn began or you are completing a finalize assignment; otherwise use `complete`.

A Director stall-nudge re-sends the same `ready (...)` target and expected action, not a new assignment. On any `ready (...)`, act on the target's `COMMENT(role)` markers addressed to your role or relevant to your phase.

## Pointer Forms

Use the tightest form that locates the target. A paired `COMMENT(role)` marker (for `blocked`, `escalating`, Director arbitration and source-anchored `COMMENT(reviewer)` findings) MUST live at the same pointer as the message.

| Form | Target | Marker placement |
|---|---|---|
| `paragraph-<HeadingPath>`, e.g. `paragraph-Implementation > Step 2` | A design-doc heading | Inline within that heading's section. |
| `<file>:<line>` or `<file>:<start>-<end>`, e.g. `src/cli/member.rs:142` | A line or range in a source, test or design-doc file | Above or on that line, in the file's comment syntax. |
| `doc` | The whole design document | Doc-top: directly under the metadata block, before the first heading. |

A `paragraph-` pointer uses the heading text verbatim, punctuation included, nested with ` > `.

## Message Format

Outside [payload exemptions](#payload-exemptions) and [anchorless statuses](#anchorless-status), every body is `<verb> (<pointer>)`, optionally followed by ` — <one-line summary>`. The body is a single line. The summary should fit one terminal line (aim for ≤ 80 codepoints) and names at most 3 items; longer enumerations, reasoning and reports belong in a marker at the pointer.

Examples: `ready (paragraph-Implementation > Step 1)`, `complete (paragraph-Implementation > Step 1) — 12 tests pass`, `addressed (src/cli/member.rs:142)`, `blocked (paragraph-Specification > Retry Strategy)`, `approved (doc)`.

## COMMENT(role) Marker

```
COMMENT(<role>): <substantive content>
```

| Role | Writer | Content |
|---|---|---|
| `user-relay` | Director as user-mediator | User clarifications and revision requests, preserving the user's meaning |
| `director` | Director | Spec resolutions, arbitration, execute Step 4 code-review feedback |
| `drafter` | Drafter | Spec ambiguities needing Director input |
| `reviewer` | Reviewer | Design-doc and code-review findings, tagged `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` |
| `programmer` | Programmer | Implementation notes, escalation rationales, blocking spec gaps |
| `tester` | Tester | Test-spec gaps, escalation rationales, evaluation of routed test-defect reports |
| `verifier` | Verifier | E2E findings, evidence pointers (`see file:line`), fix category (impl bug / test gap / spec issue) |

Write one marker per logical issue, stating the issue and what should change: `COMMENT(reviewer): [GAP] Step 4 lacks a verification entry.` Markers carry issues only; status lives in message text. The resolver removes the marker as part of the fix, and the doc reaches `Status: Approved` / `Status: Complete` only when no `COMMENT(` marker remains ([Completeness Check](guidelines.md#completeness-check)). Each source-file `COMMENT(reviewer)` marker is removed in its fix commit, so finalize confirms only that the design doc is marker-free.

The Director records ordinary-language user feedback as `COMMENT(user-relay)` at the affected paragraph, `doc`, or file/line pointer, and routes it to the owner the workflow selects (the Programmer when no Tester exists), asking only about ambiguous meaning or scope. Relayed and user-written markers follow the same rules as every other marker.

## Anchorless Status

A status with no target — `restarted`, `still working`, `ack`, `noted` — is a short parens-free phrase that adds no marker and informs the Director. Frequent anchorless updates are a stall signal under the [stall-response ladder](../../cafleet/roles/director.md#stall-response).

The spawn handshake `ready` (optionally `ready: <recap>`) is an anchorless status: the member's startup gate, separate from every role's first substantive message.

## Director Per-File Detail Recovery

Message bodies carry no file lists; for commit-message detail on every execute-workflow commit, the Director uses `git status`, `git diff --stat <base>..HEAD`, `git log <base>..HEAD --name-only` and `git diff <base>..HEAD -- <pattern>`.
