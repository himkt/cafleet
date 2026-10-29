---
name: skill-author
description: >-
  Integrate a CAFleet Director/member team into a skill, with scoped roles,
  durable broker coordination, Director wake and explicit review gates.
---

# Authoring a CAFleet team skill

Use CAFleet when independent investigation, implementation or review calls for subagents or agent teams. A direct local transformation can stay in the current session. A team has a Director and the members selected for its assignment; the host owns continuation.

## Phase prerequisites

| Before | Read | Local responsibility |
|---|---|---|
| Designing the workflow | [Core](../../../skills/cafleet/SKILL.md) and [backend bindings](../../../skills/cafleet/reference/coding-agents.md) | Define outcome, role authority, file ownership, artifacts and user gates |
| Resolving outputs | [BASE](../../../skills/cafleet/reference/base-dir.md) | Normalize the task folder and inherit supplied BASE in members |
| Spawning | [Spawn Protocol](../../../skills/cafleet/reference/supervision.md#spawn-protocol) and [Director policy](../../../skills/cafleet/roles/director.md) | Diagnose environment, register the Director and select members/models |
| Rendering prompts | [Canonical skeleton](../../../skills/cafleet/roles/director.md#canonical-spawn-prompt-skeleton) and [audit convention](../../../skills/cafleet/roles/director.md#member-create--scratch-and-audit-files) | Supply absolute role/core/backend paths, identity, BASE, host prerequisites and lossless role constraints |
| Coordination | [Coordination](../../../skills/cafleet-design-doc/reference/coordination.md) | Use artifact pointers and paired issue markers |
| Receiving | [Director wake](../../../skills/cafleet/reference/supervision.md#director-wake) and [backend recipes](../../../skills/cafleet/reference/coding-agents.md) | Arm native reception before ready replies and retain one owned receiver until resolution or explicit handoff |
| Recovery / finish | [Recovery](../../../skills/cafleet/reference/supervision.md#recovery) / [Shutdown](../../../skills/cafleet/reference/supervision.md#shutdown) | Preserve context and tear down owned members before the fleet |

Members open their authoritative role first, load their own backend and required skills through its supported loader, then use supplied host-rule equivalents for absent optional files. Ready is the first operational broker command; prerequisite shell reads may precede it only when shell is the available reader.

## Integration decisions

Normalize research/design inputs to their task folder before resolving BASE. Agent-only artifacts use hidden folders; deliverables use visible paths. Inherit literal BASE or its documented disabled-audit state.

Render a compact role-by-path prompt to a unique `.prompts/<member>-<UTC-compact>.md` and preserve that pre-spawn audit input. Leave only `{fleet_id}`, `{member_id}`, `{director_member_id}` and `{coding_agent}` for CLI identity substitution; double other literal braces. A missing BASE follows its guarded inline transport branch.

Send individually to selected recipients. Members poll after ready, before reporting and on each wake. Ordinary members receive pane previews; the Director follows [Director wake](../../../skills/cafleet/reference/supervision.md#director-wake). Native shell dispatch returns submission metadata; inspect output and explicitly continue after completion evidence if the host has not resumed the member.

## Worked example: summarize-pr

A Summarizer receives a read-only PR diff and produces a 200-word summary with three risk areas. Resolve actual paths/models/IDs for each run; examples use concrete values for readability.

### Resolve and bootstrap

Normalize the task folder, resolve BASE and fetch the diff through the host's authorized GitHub workflow. Define a read-only Summarizer role and write its compact prompt under BASE.

Run `cafleet doctor`, then register the calling Director:

```bash
cafleet fleet create --name summarize-pr-1234 --coding-agent claude --json
```

Suppose this returns fleet 7 and Director 8. Arm native reception through the shared backend recipe and require its ready record before depending on member replies.

### Render and spawn

The prompt references the authoritative role, installed CAFleet core/backend/BASE references, host-rule source, assignment, literal identity placeholders and startup ready command. Its output scope names the summary artifact; Git and user communication remain Director-owned.

```bash
cafleet member create --fleet-id 7 --name summarizer --description "Summarizes a PR diff and three risks" --file /repo/researches/pr-1234/.prompts/summarizer-20260911T100100Z.md --json
```

Receive that member's ready and ACK it. Dispatch its scope when inputs exist.

### Review and revisions

Receive the Summarizer's report under [Director wake](../../../skills/cafleet/reference/supervision.md#director-wake).

Read and ACK the completed report, inspect the summary and present it. Record revisions as actionable markers at the artifact pointer and send `ready (doc)`. The member revises, removes the marker and sends `addressed (doc)`; keep the owned receiver through the review loop.

### Teardown

After the authorized outcome, consume the final inbox, delete fleet 7 and confirm it is absent from the active list. Collect the receiver's fleet-deleted closure and native task exit. Fleet deletion ends each ordinary member as member deletion does, closing its pane, then deregisters the registry while keeping the Director's pane and retaining history. Preserve artifacts and immutable prompt inputs.
