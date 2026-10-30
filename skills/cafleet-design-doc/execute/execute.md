# Design Doc Execute (CAFleet Edition)

Implement an approved design through scoped CAFleet members, suitable verification and fresh independent review. The Director owns specification decisions, Git, user communication and lifecycle; members own their assigned source/test regions.

## Required reading

Read and resolve your [backend](../../cafleet/reference/coding-agents.md), then the [Director role](../../cafleet/roles/director.md), [file layout](../reference/guidelines.md#file-layout), [BASE resolver](../../cafleet/reference/base-dir.md), [supervision](../../cafleet/reference/supervision.md), [coordination](../reference/coordination.md) and [guidelines](../reference/guidelines.md). Use their model selection, spawn frame, path guards, native reception and message rules without duplicating those registries here.

Read recovery or shutdown immediately before those actions. Preserve host permissions, user scope, prior decisions and stop requests throughout execution.

## Director responsibilities

Own correctness against the approved specification. Assign bounded paths, shared interfaces, checks and evidence locations. Retrieve complete durable messages, ACK consumption and facilitate ready work. Inspect broker, pane and artifact evidence before treating a member as stalled; disclose a replacement and preserve its handoff.

### Step 1: Resolve Design Document Path (Director)

Accept an explicit document, a task folder containing `design-doc.md`, or discovery under the repository's design-doc root. Normalize and resolve BASE before task/audit writes. Preserve an explicit target.

Discovery selects documents whose Status is Approved: select the only match, ask the user when several match, and report an actionable error when none match. For resumed work, verify checked tasks and progress metadata before continuing from the next incomplete task.

### Step 2: Validate Design Document & Create Branch (Director)

Read the complete design and its existing annotations. Resolve clear requests and route consequential ambiguity before dependent work. Identify implementation order, protected behavior and existing `FIXME(agent)` findings.

Inspect the default/current branch. On the default branch, propose a feature branch and obtain the user's approval before creating it. Preserve an existing feature branch. Host rules govern eligible files and commits.

### Team composition

| Work | Composition |
|---|---|
| Code implementation | Programmer and Tester with separate source/test ownership. |
| Documentation/configuration only | Programmer with suitable checks and Director review. |
| External behavior or explicit live acceptance | Add Verifier for the required observations. |
| Completed implementation | Add a fresh Reviewer after implementation and verification gates pass. |

Read the assigned roles before spawning: [Programmer](roles/programmer.md), [Tester](roles/tester.md), [Verifier](roles/verifier.md) and [Reviewer](roles/reviewer.md). Existing roles retain their responsibilities; a Programmer-only team receives the documentation/configuration composition explicitly.

### Step 3: Register & Spawn Members (Director)

1. Diagnose prerequisites, register the calling Director atomically and retain literal fleet/member IDs.
2. Follow [Director wake](../../cafleet/reference/supervision.md#director-wake).
3. Render compact role-by-path prompts with the delta table below: absolute role/core/backend paths, literal identity placeholders, BASE, owned paths and checks. Members load both skills and continue this team.
4. Receive and ACK ready, then dispatch each member when inputs exist. Resolve uncertain launch/input by inspecting actual evidence before another submission.

Render each member prompt from the [canonical spawn skeleton](../../cafleet/roles/director.md#canonical-spawn-prompt-skeleton) with the CAFleet load purpose `for communication with the Director` and this per-role delta. Each line in the IMPORTANT row is a separate prompt line, rendered verbatim. Spawn the Reviewer only at Step 5.

| Slot | Programmer | Tester | Verifier | Reviewer |
|---|---|---|---|---|
| ROLE TITLE / TEAM | `the Programmer` / `design document execution` | `the Tester` / `design document execution` | `the Verifier` / `design document execution` | `the Reviewer` / `design document execution` |
| role-file | `roles/programmer.md` | `roles/tester.md` | `roles/verifier.md` | `roles/reviewer.md` |
| EXTRA SKILL LOADS | `cafleet-design-doc` | `cafleet-design-doc` | `cafleet-design-doc` | `cafleet-design-doc` |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT absolute design-doc path]`, `TEAM COMPOSITION: [INSERT composition and owned paths]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]`, `TEAM COMPOSITION: [INSERT composition and owned paths]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]` | `DESIGN DOCUMENT: [INSERT absolute design-doc path]`, `BASE BRANCH: [INSERT base branch name]` |
| IMPORTANT / ROLE-CONSTRAINT LINES (verbatim) | `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.`, `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`, `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `IMPORTANT: Write only test code, including inline test regions; do NOT edit implementation code.`, `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.`, `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`, `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `IMPORTANT: Do NOT commit, and do NOT modify implementation or test files; your only edits are COMMENT(verifier) markers.`, `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`, `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `IMPORTANT: Your only edits are COMMENT(reviewer) markers; do NOT edit code or commit.`, `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/prompt-routing.md), which you load at startup.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| start cue (verbatim) | `Read the design document and wait for the Director to assign your first step.` | `Read the design document and wait for the Director to assign your first step.` | `Read the design document, perform tool discovery and send your tool inventory to the Director.` | `Wait for the Director's ready (doc) to start the review.` |

### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For a code step, the Tester writes behavior-focused tests; the Director reviews them against the design and commits eligible tests. The Programmer implements within its source ownership and runs the relevant checks. Suspected test defects return to the Director and Tester for arbitration. In a Programmer-only team, route findings directly to that owner.

Review the final change for correctness, scope and maintainability; request fixes and affected checks until the step passes. Keep tests and implementation commits separate in TDD teams, applying host rules to mixed files. Update each completed checkbox and timestamp together, and keep any existing Progress counter accurate. Preserve completed work when resuming.

### Phase D: Verification (Director) — conditional

Have the Verifier discover available tools and perform applicable real-product acceptance with isolated, owned resources. Require exact commands/results and concrete observations for changed safety boundaries. Record actual limitations; missing required evidence remains incomplete. Resolve failures with the owning member and repeat affected verification.

### Step 5: Reviewer Review Loop (Director)

Verify every implementation task and Success Criterion, including required checks and manual acceptance, before the fresh Reviewer starts. Give it the complete design, full branch diff and actual evidence. Its work is independent read-only implementation review, not self-review by an implementing member.

Route actionable findings to their source/test owners, arbitrate disputes against evidence and rerun affected checks. Repeat fresh review until approval; the loop has no round cap. Honor a user halt immediately and retain the pending work. User revisions return through this review before another approval presentation.

### Step 6: User Approval (Director)

Present the reviewed result with the diff command, behavior changes, checks and actual limitations. Request approval through the executing backend's decision surface. Interpret feedback by meaning; preserve questions as questions and route revisions through implementation, checks and review.

Record publication scope explicitly: local approval finalizes locally; remote approval authorizes push and PR creation. Carry earlier answers forward and resolve only a genuinely new decision.

### Step 7: Push & Create PR (Director)

For remote approval, verify authentication, the feature branch and commits beyond the base. Apply host Git rules before staging or publishing. Push without force and create the PR from a prepared description file under the applicable GitHub workflow.

Authentication/precondition failure leaves the result local. Report exact push/PR failures; preserve a successful push if PR creation fails and retain its manual recovery information. Tracking alone supplies no publication authority.

### Step 8: Finalize & Clean Up (Director)

Verify final progress and marker resolution, set Status to Complete only when the required work and user gate are satisfied, and preserve existing historical artifacts. Commit only files allowed by host rules. Apply the recorded publication scope to any eligible final push.

Drain/cancel native reception and observe its source-process exit before retirement. Delete the fleet, which ends every owned ordinary member and reports each pane outcome; resolve and rerun when it reports a member it could not end, then verify the fleet's absence. Limit cleanup to owned resources.
