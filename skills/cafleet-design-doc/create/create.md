# Design Doc Create (CAFleet Edition)

Create a reviewed design document with a Director, Drafter and independent Reviewer. The Director owns user decisions, Git and lifecycle; the Drafter owns document edits; the Reviewer assesses the draft.

## Required reading

Read and resolve your [backend](../../cafleet/reference/coding-agents.md), then the [Director role](../../cafleet/roles/director.md), [file layout](../reference/guidelines.md#file-layout), [BASE resolver](../../cafleet/reference/base-dir.md), [supervision](../../cafleet/reference/supervision.md), [coordination](../reference/coordination.md) and [guidelines](../reference/guidelines.md). These references own model selection, paths, reception, messages and document format.

Use absolute role/core/backend paths and the canonical CAFleet spawn frame with the Step 1 delta table. Preserve literal identity placeholders, BASE/output paths and role boundaries. Assigned members continue this team. Read recovery or shutdown immediately before those actions.

## Director responsibilities

Preserve the user's scope, answers and existing annotations. Assign explicit owned paths and report evidence rather than inferring completion from idle time. Read full durable messages and ACK consumption. Maintain native reception and supervise outstanding work under the shared Director procedure.

### Step 0: Path Resolution & Resume Detection (Director)

Normalize the argument using the guidelines and resolve the selected folder through BASE before writing. Preserve explicit targets; a newly chosen folder uses the next available seven-digit prefix. Set the document's absolute output path and guard audit writes through BASE.

For an existing document, retain its content and answered questions. Existing user-relay markers enter resume mode. Otherwise resolve whether the user wants quality review or a fresh draft before replacing content. Reuse prior answers and skip already completed clarification.

### Step 1: Register & Spawn Members (Director)

1. Diagnose actual prerequisites with `cafleet doctor`, register the calling Director and retain returned literal IDs.
2. Follow [Director wake](../../cafleet/reference/supervision.md#director-wake).
3. Spawn the [Drafter](roles/drafter.md) and [Reviewer](roles/reviewer.md) with their role paths, required skills, concrete document/BASE paths and scoped assignments.
4. Receive and ACK ready before depending on a member. Dispatch each assignment when its inputs exist; inspect actual placement when launch evidence is uncertain.

Render each member prompt from the [canonical spawn skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with the CAFleet load purpose `for communication with the Director` and this per-role delta. Each line in the IMPORTANT row is a separate prompt line, rendered verbatim.

| Slot | Drafter (fresh draft) | Drafter (resume or review-only) | Reviewer |
|---|---|---|---|
| ROLE TITLE / TEAM | `the Drafter` / `design document creation` | `the Drafter` / `design document creation` | `the Reviewer` / `design document creation` |
| role-file + ROLE-DEF SUFFIX | `roles/drafter.md` | `roles/drafter.md` + `Follow the Resume Mode section in particular.` (resume) or `Follow the Review-only Mode section in particular.` (review-only) | `roles/reviewer.md` |
| EXTRA SKILL LOADS | `cafleet-design-doc` | `cafleet-design-doc` | `cafleet-design-doc` |
| CONTEXT LINES | `OUTPUT PATH: [INSERT absolute design-doc path]`, `USER REQUEST: [INSERT the user's request]` | `OUTPUT PATH: [INSERT absolute design-doc path]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]` |
| IMPORTANT / ROLE-CONSTRAINT LINES (verbatim) | `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.`, `Send your questions to the Director who will relay them to the user.`, `Do NOT create any design document file until you have received answers.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `IMPORTANT: Your only edits are COMMENT(reviewer) markers in the design document.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| start cue (verbatim) | `Read the target codebase for context, then send your clarifying questions to the Director.` | `Read the design document, then wait for the Director's revision assignment.` | `Wait for the Director to route the draft with ready (doc).` |

### Step 2: Clarification Phase (Director)

For fresh work, have the Drafter inspect relevant source and identify consequential unanswered requirements before drafting. Relay focused questions through the executing backend's user-decision surface, preserve answers and route them back faithfully. Resume and review-only work start from existing content and annotations.

### Step 3: Internal Quality Loop (Director)

1. Assign the draft against the guidelines: intended behavior, scope, compatibility, failure handling, measurable success criteria and implementable tasks.
2. Route the complete revision to the Reviewer. Findings use coordination pointers/markers and explain the concrete problem.
3. Send actionable findings to the Drafter, arbitrate specification decisions and obtain another review of the revised document.
4. Continue until the Reviewer approves. Report blockers immediately; preserve explicit halt/abort requests and pending decisions.

### Step 4: Present to User (Director)

Present the reviewed document, its location and the decisions that need approval. Use the executing backend's decision surface for the user's reaction. A question requests an answer; approval is its own decision.

### Step 5: User Feedback Loop (Director)

Route ordinary-language feedback and existing user markers to the Drafter without losing their meaning. Ask only about consequential ambiguity. After revisions, repeat independent review before presenting the new revision. Retain previous answers and publication authority.

### Step 6: Finalize & Clean Up (Director)

After user approval, send the Drafter an explicit finalization assignment: incorporate accepted decisions, resolve coordination markers, set Status to Approved and keep implementation progress accurate. Verify the final artifact and apply host Git/publication rules. Honor abort without marking the design approved.

Collect receiver closure and observed process exit, delete owned ordinary members, verify the root-only roster, then delete the owned fleet and confirm closure. Preserve resources outside this team's ownership.
