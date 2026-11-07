# Design Doc Create (CAFleet Edition)

Create a reviewed design document with a Director, Drafter and independent Reviewer. The Director owns user decisions, Git and lifecycle; the Drafter owns document edits; the Reviewer assesses the draft.

## Required reading

Read and resolve your [backend](../../cafleet/reference/coding-agents.md), then the [Director role](../../cafleet/roles/director.md), [file layout](../reference/guidelines.md#file-layout), [BASE write protocol](../../cafleet/roles/director.md#base-directory-resolution), [supervision](../../cafleet/roles/director.md#supervision), [coordination](../reference/coordination.md) and [guidelines](../reference/guidelines.md), which own model selection, paths, reception, messages and document format.

Preserve BASE/output paths and role boundaries.

## Director responsibilities

Preserve the user's scope, answers and existing annotations. Assign explicit owned paths, and report evidence instead of inferring completion from idle time. Read full durable messages, ACK consumption, and keep reception and supervision under the shared [Director procedure](../../cafleet/roles/director.md#supervision).

### Step 0: Path Resolution & Resume Detection (Director)

Normalize the argument per the guidelines and resolve the selected folder through BASE before writing. Preserve explicit targets; a newly chosen folder takes the next available seven-digit prefix. Set the document's absolute output path and guard audit writes through BASE.

An existing document keeps its content: existing user-relay markers enter resume mode; otherwise resolve whether the user wants quality review or a fresh draft before replacing content.

### Step 1: Register & Spawn Members (Director)

1. Diagnose actual prerequisites with `cafleet doctor`, register the calling Director and retain returned literal IDs.
2. Follow [Director wake](../../cafleet/roles/director.md#director-wake).
3. Spawn the [Drafter](roles/drafter.md) and [Reviewer](roles/reviewer.md) with their role paths, required skills, concrete document/BASE paths and scoped assignments.
4. Receive and ACK ready before depending on a member. Dispatch each assignment when its inputs exist; inspect actual placement when launch evidence is uncertain.

Render each member prompt from the [canonical spawn skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with ROLE TITLE `the <Role>`, TEAM `design document creation`, role-file `roles/<role>.md`, CAFleet load purpose `for communication with the Director`, EXTRA SKILL LOADS `cafleet-design-doc` and this per-role delta.

| Slot | Drafter (fresh draft) | Drafter (resume or review-only) | Reviewer |
|---|---|---|---|
| ROLE-DEF SUFFIX | — | `Follow the Resume Mode section in particular.` (resume) or `Follow the Review-only Mode section in particular.` (review-only) | — |
| CONTEXT LINES | `OUTPUT PATH: [INSERT absolute design-doc path]`, `USER REQUEST: [INSERT the user's request]` | `OUTPUT PATH: [INSERT absolute design-doc path]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]` |
| START CUE (verbatim) | `Read the target codebase for context, then send your clarifying questions to the Director.` | `Read the design document, then wait for the Director's revision assignment.` | `Wait for the Director to route the draft with ready (doc).` |

Render the IMPORTANT / ROLE-CONSTRAINT lines verbatim, one prompt line each, in this order: for the fresh Drafter, `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.`, `Send your questions to the Director who will relay them to the user.` and `Do NOT create any design document file until you have received answers.`; for the Reviewer, `IMPORTANT: Your only edits are COMMENT(reviewer) markers in the design document.`; then, for every role, `When you see cafleet message poll output with a message from the Director, act on those instructions.`

### Step 2: Clarification Phase (Director)

For fresh work, the Drafter inspects relevant source and sends one clarification round before drafting: questions on consequential unanswered requirements and confirmation of the requirements it understood. Relay it through the executing backend's user-decision surface, preserve the answers and route them back faithfully. Resume and review-only work start from existing content and annotations.

### Step 3: Internal Quality Loop (Director)

1. Assign the draft against the guidelines: intended behavior, scope, compatibility, failure handling, measurable success criteria and implementable tasks.
2. Route the complete revision to the Reviewer. Findings use coordination pointers/markers and explain the concrete problem.
3. Send actionable findings to the Drafter, arbitrate specification decisions and obtain another review of the revised document.
4. Continue until the Reviewer approves. Report blockers immediately; preserve explicit halt/abort requests and pending decisions.

### Step 4: Present to User (Director)

Present the reviewed document, its location and the decisions needing approval, collecting the user's reaction through the executing backend's decision surface. A question requests an answer; approval is its own decision.

### Step 5: User Feedback Loop (Director)

Route ordinary-language feedback and existing user markers to the Drafter without losing their meaning. Ask only about consequential ambiguity. After revisions, repeat independent review before presenting the new revision. Retain previous answers and publication authority.

### Step 6: Finalize & Clean Up (Director)

After user approval, send the Drafter exactly `ready (doc) — user approved; finalize`: incorporate accepted decisions, resolve coordination markers, set Status to Approved and keep implementation progress accurate. Verify the final artifact and apply host Git/publication rules. Honor abort without marking the design approved.

Then end the team under the Director [Shutdown](../../cafleet/roles/director.md#shutdown), preserving resources outside this team's ownership.
