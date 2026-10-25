---
name: cafleet-design-doc
description: Create, refine or implement design documents through scoped CAFleet members and independent review.
---

# Design documents

Use the [CAFleet core](../cafleet/SKILL.md), your [backend](../cafleet/reference/coding-agents.md) and the [Director procedure](../cafleet/roles/director.md) for coordination. An assigned member continues its existing team and assignment. Read the compact [template and guidelines](reference/guidelines.md) before document edits.

The Director supplies explicit absolute output/scratch paths, assigns scoped work, receives ready and direct evidence messages, owns Git/user decisions and closes owned members. Use the selected backend's configured model/effort defaults unless the user supplied explicit values. Review substantial work independently and validate routine edits proportionately.

## Choose the work

| Request | Procedure |
|---|---|
| Create a design/specification | Clarify consequential gaps, draft an implementable document, obtain independent review and resolve findings. |
| Interview/refine an existing design | Read existing content/answers, ask focused unresolved questions, preserve answers and revise affected decisions. |
| Implement/execute a design | Confirm scope, assign bounded implementation and suitable checks, review evidence and resolve findings. |

Keep the document readable as the current intended specification. Existing annotations and answered questions remain meaningful inputs; preserve their content until incorporated or explicitly resolved.

## Create

1. Read repository/host rules, existing designs and relevant implementation. Resolve consequential unknown requirements before dependent drafting; use ordinary reasonable choices for routine details.
2. Select the next seven-digit directory under `design-docs/` or preserve the explicit target. Supply absolute document/scratch paths to members.
3. Assign a member to write the design: clear behavior, boundaries, compatibility, failure handling, measurable success criteria and executable tasks.
4. Assign independent review of substantial specifications. Send findings directly with document paths/sections and rationale; revise until the design is coherent and implementable.
5. Present the concrete result through the host's user-decision surface when approval is required. Carry prior authorization and explicit gates forward.
6. Mark the design approved when applicable and close the owned team through the Director procedure.

## Refine

1. Read the selected document, related source and existing answers/annotations. Preserve numbering and substantive prior answers.
2. Identify decisions whose uncertainty affects behavior or implementation. Ask concise, focused questions in ordinary language; independent analysis may help substantial designs.
3. Record questions/answers beside the document when requested, using the explicit absolute artifact path. A response answers that question; approvals remain their own decision.
4. Incorporate clarified decisions, reconcile annotations and update affected tasks/success criteria. Review substantial revisions independently.
5. Honor stop requests and pending decisions. Continue only independent authorized work while a needed answer is pending.

## Implement

1. Read the approved specification and task checkboxes, then inspect source and existing checks. Verify completed work before resuming from the next unfinished task.
2. Assign members by bounded ownership, including shared interfaces, expected behavior, checks and evidence paths. Each implementing member owns suitable tests/checks for its change.
3. Update affected operator docs and agent procedure with the behavior they own. Preserve historical records while fully removing maintained references to removed features.
4. Implement and run checks appropriate to the affected boundary. Report exact commands/results, limitations and artifacts directly; escalate consequential spec gaps before dependent work.
5. Check each completed task and its completion timestamp in the same edit. Task checkboxes are progress truth; preserve any existing progress metadata accurately.
6. Review substantial implementation independently against the design and diff. Resolve findings, rerun affected checks and verify success criteria.
7. Complete applicable user gates, finalize Git/publication under existing authorization, report evidence and remaining gaps, and tear down owned resources.

## Communication

Assignments and reports carry their substantive content in durable messages. Use artifact paths for long reports, retain IDs/evidence in host-owned state and ACK consumption after full reads. Report a blocker immediately with the observed cause and needed decision. The Director resolves ownership conflicts and user decisions.

Use the general [member procedure](../cafleet/roles/member.md). The Director's receiver remains owned across delegated work and follows observed-exit recovery; broker events preserve host goals, budgets and pending user decisions.
