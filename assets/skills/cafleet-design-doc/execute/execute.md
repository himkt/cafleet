# Design Doc Execute (CAFleet Edition)

Implement an approved design through scoped CAFleet members, verification and fresh review.

## Required reading

Resolve your [backend](../../cafleet/reference/coding-agents.md), then read the [Director role](../../cafleet/roles/director.md), [coordination](../reference/coordination.md) and [guidelines](../reference/guidelines.md).

## Director responsibilities

Own correctness against the specification, specification decisions, Git, user communication and lifecycle, preserving host permissions, user scope, prior decisions and stop requests; members own their assigned source/test regions. Assign bounded paths, shared interfaces, checks and evidence locations, and disclose a member replacement with its handoff.

### Step 1: Resolve the design document

Use an explicit document, a task folder containing `design-doc.md`, or the only Approved document under the design-doc root; ask the user when several match and report an error when none do. Resolve BASE before writes. To resume, verify checked tasks and progress metadata and continue from the next incomplete task.

### Step 2: Validate and branch

Read the complete design, its annotations and `FIXME(agent)` findings; identify implementation order and protected behavior, and route consequential ambiguity first. On the default branch, propose a feature branch and create it once the user approves; keep an existing feature branch.

### Team composition

Code implementation uses a Programmer and Tester with separate source/test ownership; documentation/configuration-only work uses a Programmer under suitable checks and Director review, stated in its prompt. Add a Verifier for external behavior or explicit live acceptance, and a fresh Reviewer once implementation and verification gates pass. Read each assigned [Programmer](roles/programmer.md), [Tester](roles/tester.md), [Verifier](roles/verifier.md) and [Reviewer](roles/reviewer.md) role file before spawning.

### Step 3: Spawn members

Follow the Director [Spawn Protocol](../../cafleet/roles/director.md#spawn-protocol) and [Director wake](../../cafleet/roles/director.md#director-wake). Render each prompt from the [canonical spawn skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with ROLE TITLE `the <Role>` (the column heading), TEAM `design document execution`, role-file `roles/<role>.md`, CAFleet load purpose `for communication with the Director`, EXTRA SKILL LOADS `cafleet-design-doc`, BASE, owned paths, checks and this delta. Every role's CONTEXT LINES start with `DESIGN DOCUMENT: [INSERT absolute design-doc path]`.

| Slot | Programmer | Tester | Verifier | Reviewer |
|---|---|---|---|---|
| Further CONTEXT LINES | `TEAM COMPOSITION: [INSERT composition and owned paths]` | `TEAM COMPOSITION: [INSERT composition and owned paths]` | — | `BASE BRANCH: [INSERT base branch name]` |
| Role constraint lines | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` | `IMPORTANT: Write only test code, including inline test regions; do NOT edit implementation code.`, `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.` | `IMPORTANT: Do NOT commit, and do NOT modify implementation or test files; your only edits are COMMENT(verifier) markers.` | `IMPORTANT: Your only edits are COMMENT(reviewer) markers; do NOT edit code or commit.` |
| Blocked line | yes | yes | yes | no |
| START CUE (verbatim) | `Read the design document and wait for the Director to assign your first step.` | `Read the design document and wait for the Director to assign your first step.` | `Read the design document, perform tool discovery and send your tool inventory to the Director.` | `Wait for the Director's ready (doc) to start the review.` |

Render these lines verbatim, one prompt line each, in order: the role constraint lines; `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` where Blocked line is `yes`; `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md, including Reconsider, then route), which you load at startup.`; `When you see cafleet message poll output with a message from the Director, act on those instructions.`

### Step 4: Per-step TDD cycle

Review the Tester's behavior-focused tests against the design and commit eligible ones; the Programmer then implements and runs checks. Arbitrate suspected test defects; a Programmer-only team routes findings to the Programmer. Review each change for correctness, scope and maintainability until the step passes with its checkbox, timestamp and Progress updated; commit tests and implementation separately, applying host rules to mixed files.

### Verification — conditional

The Verifier performs applicable real-product acceptance on isolated, owned resources, reporting exact commands and observations, including changed safety boundaries; work lacking required evidence stays incomplete. Resolve failures with the owner and re-verify.

### Step 5: Review loop

Once every task, Success Criterion, required check and manual acceptance passes, give the fresh Reviewer the design, full branch diff and evidence. Route findings to their owners, arbitrate disputes against evidence, rerun affected checks and repeat fresh review until approval, with no round cap. Honor a user halt immediately, retaining pending work.

### Step 6: User approval

Present the diff command, behavior changes, checks and limitations through {decision_surface}; answer questions and route revisions through implementation, checks and review. Local approval finalizes locally; remote approval authorizes push and PR creation.

### Step 7: Push and PR

For remote approval, verify authentication, the feature branch and commits beyond the base, push without force and create the PR from a prepared description file under host Git rules. A failed precondition leaves the result local; report exact push/PR failures, keeping a successful push when PR creation fails and reporting the manual PR-creation details. Tracking alone supplies no publication authority.

### Step 8: Finalize

Verify progress and marker resolution, set Status to Complete once the work and user gate are satisfied, preserve existing historical artifacts, apply the approved publication scope to any final push, and end the team under the Director [Shutdown](../../cafleet/roles/director.md#shutdown), cleaning up only owned resources.
