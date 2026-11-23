# Design Doc Create (CAFleet Edition)

The Director owns user decisions, Git and lifecycle, the Drafter owns document edits and an independent Reviewer assesses the draft.

## Required reading

Resolve your [backend](../../cafleet/reference/coding-agents.md), then read the [Director role](../../cafleet/roles/director.md) (including [BASE](../../cafleet/roles/director.md#base-directory-resolution) and [supervision](../../cafleet/roles/director.md#supervision)), [coordination](../reference/coordination.md) and [guidelines](../reference/guidelines.md).

## Director responsibilities

Preserve the user's scope, answers and annotations, and judge completion by reported evidence, not idle time.

### Step 0: Path Resolution & Resume Detection (Director)

Resolve the absolute output path per [File Layout](../reference/guidelines.md#file-layout) and [Numbering](../reference/guidelines.md#numbering). An existing document with user-relay markers enters resume mode; for any other, ask whether the user wants review or a fresh draft.

### Step 1: Register & Spawn Members (Director)

Run `cafleet doctor`, register the Director, follow [Director wake](../../cafleet/roles/director.md#director-wake), spawn the [Drafter](roles/drafter.md) and [Reviewer](roles/reviewer.md) and dispatch per [Spawn Protocol](../../cafleet/roles/director.md#spawn-protocol) step 6.

Render each prompt from the [canonical spawn skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with ROLE TITLE `the <Role>`, TEAM `design document creation`, role-file `roles/<role>.md`, CAFleet load purpose `for communication with the Director`, EXTRA SKILL LOADS `cafleet-design-doc` and this delta:

| Slot | Drafter (fresh draft) | Drafter (resume or review-only) | Reviewer |
|---|---|---|---|
| ROLE-DEF SUFFIX | — | `Follow the Resume Mode section in particular.` (resume) or `Follow the Review-only Mode section in particular.` (review-only) | — |
| CONTEXT LINES | `OUTPUT PATH: [INSERT absolute design-doc path]`, `USER REQUEST: [INSERT the user's request]` | `OUTPUT PATH: [INSERT absolute design-doc path]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]` |
| START CUE (verbatim) | `Read the target codebase for context, then send your clarifying questions to the Director.` | `Read the design document, then wait for the Director's revision assignment.` | `Wait for the Director to route the draft with ready (doc).` |

Render these lines verbatim, one prompt line each, in order: for the fresh Drafter, `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.`, `Send your questions to the Director who will relay them to the user.` and `Do NOT create any design document file until you have received answers.`; for the Reviewer, `IMPORTANT: Your only edits are COMMENT(reviewer) markers in the design document.`; then, for every role, `When you see cafleet message poll output with a message from the Director, act on those instructions.`

### Step 2: Clarification Phase (Director)

Relay each fresh Drafter clarification round through the decision surface and route the answers back faithfully.

### Step 3: Internal Quality Loop (Director)

Assign the draft against the guidelines (intended behavior, scope, compatibility, failure handling, measurable success criteria, implementable tasks); route each complete revision to the Reviewer and its findings to the Drafter, arbitrating specification decisions, until the Reviewer approves. Report blockers immediately.

### Step 4: Present to User (Director)

Present the reviewed document, its location and pending decisions through the decision surface; approval is its own decision.

### Step 5: User Feedback Loop (Director)

Route user feedback to the Drafter as [COMMENT(role) markers](../reference/coordination.md#commentrole-marker) and obtain Reviewer approval before presenting each revision.

### Step 6: Finalize & Clean Up (Director)

After user approval, send the Drafter exactly `ready (doc) — user approved; finalize`, verify the result and apply host Git/publication rules; an abort leaves the design unapproved. Then delete owned members, verify the root-only roster and delete the fleet per [Shutdown](../../cafleet/roles/director.md#shutdown).
