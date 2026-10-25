# Coding-Agent Requirements by Role

**Status**: Approved
**Progress**: 0/6 tasks complete
**Last Updated**: 2026-10-26

## Overview

Define the coding-agent capabilities required for the Director and Member roles so maintainers can evaluate additional backends. Make the smallest documentation-only change in the existing skill and operator documentation, preserving their independent structure.

## Success Criteria

- [ ] Both backend references state that a Director requires a goal command or native equivalent and background execution, and a Member requires auto or dontAsk mode or native equivalent.
- [ ] Each CAFleet role definition links to the requirements within the skill documentation.
- [ ] The requirements and existing manual-reception alternatives distinguish Director prerequisites from reception methods, preserving host permissions and continuation authority.
- [ ] The production diff is confined to the Markdown files specified below, and documentation validation passes.

---

## Background

The repository backend references already describe permission mechanisms and Director reception: Claude uses native auto review, Codex uses `--approve-for-me` with `workspace-write`, and OpenCode uses the `cafleet` permission preset. These operational details currently lack a concise statement of the capabilities required for each role. The repository rule in `.claude/rules/coding-agent-overlay.md` keeps skill and operator documentation independent.

---

## Specification

### Role requirements

Add a short requirements section to each backend reference using this contract:

| Role | Required coding-agent capabilities |
|---|---|
| Director | A goal command, such as `/goal`, or equivalent native goal lifecycle controls; and the ability to run background processes while continuing supervision. Both capabilities are required. |
| Member | `auto` or `dontAsk` mode, or equivalent native automatic-review or fixed allow/deny behavior for task execution, under the host's existing permissions. |

Follow the table with these shared clarifications:

- Evaluate suitability for each role separately. Native mechanisms with different names qualify when they provide the required capability.
- Treat goal support as a host capability. Preserve the supplied goal's activation, pause, completion and budget authority.
- Preserve native guards, explicit denies, managed policies and sandbox boundaries. Apply the existing denial-routing and unresolved-approval procedures.
- Evaluate a verified native receive adapter separately from Director suitability. On a host satisfying both Director prerequisites, explicit manual `poll` or finite `wait` remains a valid reception method when a verified native adapter is unavailable. Background execution alone establishes no unsolicited wake or per-record reception guarantee.

The existing backend notes remain the home for concrete mechanisms and acceptance evidence. This change specifies the role contract through the shared section, without introducing a backend support matrix or new acceptance probes.

### Placement and scope

| File | Change |
|---|---|
| `skills/cafleet/reference/coding-agents.md` | Insert `## Role requirements` immediately before the first backend section, containing the contract and clarifications above. Qualify the Claude and OpenCode manual-reception alternatives with Director eligibility, linking to `#role-requirements`. |
| `skills/cafleet/roles/director.md` | Add one sentence to the introduction requiring a backend that satisfies the Director requirements, linking to `../reference/coding-agents.md#role-requirements`. |
| `skills/cafleet/roles/member.md` | Add the corresponding Member sentence and same relative target to the introduction. |
| `skills/cafleet/reference/supervision.md` | Qualify the Spawn Protocol's manual-reception alternative and the Receiver recovery row for an unavailable or denied native stream tool with Director eligibility, linking to `coding-agents.md#role-requirements`. |
| `docs/docs/spec/coding-agent-backends.md` | Insert `## Role requirements {#role-requirements}` immediately before `## Spawn argv`, independently stating the same contract and clarifications for operators. Qualify the Claude manual-reception alternative, linking to `#role-requirements`. |
| `docs/docs/concepts/coding-agents.md` | Qualify the manual-reception alternative in Director wake with Director eligibility, linking to `../spec/coding-agent-backends.md#role-requirements`. |
| `docs/docs/how-to/mixed-backend-team.md` | Qualify the manual-reception alternative in Receive results with Director eligibility and the same operator-reference link. |

Keep links within their documentation home: role definitions and supervision point to the skill reference, and operator pages point to the operator reference. The skill core's existing required-reading links already lead to the backend reference. Qualify existing manual-reception alternatives as methods for hosts satisfying both Director prerequisites, preserving their receive behavior and backend evidence. The production change consists of the short requirements sections, role links and these local consistency edits.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: State requirements in the skill

- [ ] Add the shared role requirements and clarifications to `skills/cafleet/reference/coding-agents.md` at the specified location. <!-- completed: -->
- [ ] Add the Director and Member introduction sentences linking to the skill requirements. <!-- completed: -->
- [ ] Qualify the manual-reception alternatives in the skill backend reference and supervision entry points listed above, linking to the skill requirements and preserving receive behavior and evidence. <!-- completed: -->

### Step 2: State requirements for operators

- [ ] Add the corresponding self-contained requirements section to `docs/docs/spec/coding-agent-backends.md`. <!-- completed: -->
- [ ] Qualify the manual-reception alternatives in the operator backend reference, coding-agent concept and fleet how-to entry points listed above, linking to the operator requirements and preserving receive behavior and evidence. <!-- completed: -->

### Step 3: Validate the documentation change

- [ ] Review the diff for consistent role semantics, working local anchors, independent documentation homes and the specified Markdown-only scope; run `mise //docs:build`. <!-- completed: -->
