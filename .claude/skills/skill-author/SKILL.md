---
name: skill-author
description: >
  Integrate a CAFleet-orchestrated Director/member team into a new skill, with
  role files, broker coordination and supervision. Use when authoring a skill
  that spawns cafleet members or adding a CAFleet team to an existing skill.
  Project-local integration guide; load the linked CAFleet prerequisites at
  their phase triggers.
---

# Authoring a CAFleet team skill

Use a CAFleet team when the work needs separate persistent coding-agent processes, specialized roles or parallel slices coordinated through the broker. Use a single-session skill or an in-process subagent for a small transformation that needs none of those properties. A team consists of the Director, its monitor, and ordinary members; the Director coordinates the work and owns user communication.

Write the new skill's workflow and role responsibilities locally. Reuse the shared protocols through required reads at the phases below, so their exact mechanics have one owner.

## Phase prerequisites

| Before | Read and apply | Local skill responsibility |
|---|---|---|
| Designing the workflow | [CAFleet core](../../../skills/cafleet/SKILL.md), [backend-neutrality rule](../../rules/coding-agent-overlay.md) and the executing backend's [runtime bindings](../../../skills/cafleet/reference/coding-agents.md) | State the outcome, role boundaries, disjoint write ownership, artifact locations and approval gates; resolve concrete local tools. |
| Resolving output paths or writing | [BASE contract](../../../skills/cafleet/reference/base-dir.md) | Declare a task-folder convention and normalize the input to that folder before resolution. Members consume the supplied BASE. |
| Bootstrapping or spawning | [Director role](../../../skills/cafleet/roles/director.md) and [supervision](../../../skills/cafleet/reference/supervision.md#spawn-protocol) | Gate on doctor and monitor live; select models under the Director policy and inspect failed compensation before retrying. |
| Rendering a prompt | [Canonical frame](../../../skills/cafleet/roles/director.md#canonical-spawn-prompt-skeleton), [audit protocol](../../../skills/cafleet/roles/director.md#member-create--scratch-and-audit-files), selected backend's Model catalog/Role defaults | Supply role path, identity block, assignment, all IMPORTANT obligations, ready and start cue. |
| Coordinating work | [Coordination](../../../skills/cafleet-design-doc/reference/coordination.md), including payload exemptions and marker pairing | Declare artifact pointers and any local role extension; make substance readable at the routed pointer. |
| Recovering or finishing | [Recovery](../../../skills/cafleet/reference/supervision.md#recovery) and [Shutdown](../../../skills/cafleet/reference/supervision.md#shutdown) | Keep authorized work moving, then delete monitor first, remaining members, confirm cleanup and delete the fleet. |

Load a shared prerequisite once and return to its relevant section when its trigger occurs. The new member's role file requires its own backend resolution, CAFleet member startup and local workflow reads. Use an available text reader; when shell is the only reader, prerequisite reads may precede ready. Ready remains the first operational broker command. Supply equivalent session instructions for absent optional host-rule files and route an essential unknown prerequisite before dependent work.

## Integration decisions

Choose `researches/<topic-slug>` for research-shaped tasks or `design-docs/<NNNNNNN>-<slug>` for design documents. Strip a supplied deliverable filename before resolving the task folder; relative arguments use the skill's bucket, absolute arguments keep their location. The BASE owner defines no-repository errors, `<unset>`, guarded writes and the missing-BASE member status. Agent-only artifacts use hidden directories such as `.prompts/`; user deliverables use visible paths. A caller's explicit output path follows the BASE contract, and a failed resolution receives its specified error rather than an ad-hoc temporary fallback.

Render each prompt to a unique `.prompts/<lowercase-member-name>-<UTC-compact>.md`, retaining that pre-spawn file as the immutable audit input. Resolve every `[INSERT …]` marker and model/runtime token before writing; leave only `{fleet_id}`, `{member_id}`, `{director_member_id}` and `{coding_agent}` for the CLI formatter. Double all other literal braces. Both creation commands format identities after allocating them. Take returned IDs as literal integers in subsequent broker calls.

Reference role files by absolute path. `--file` provides a durable input artifact and avoids an oversized caller shell command; the resolved prompt still reaches the backend spawn argv and remains subject to transport limits. Keep the frame compact and move role instructions into their owner file. Follow the BASE owner's guarded `<unset>` branch: omit `BASE:`, skip the audit write, use the supported inline ordinary-member or stdin monitor form, and report the anchorless audit-disabled status. An unsupported or failed transport is an error to surface.

Broker sends persist messages and attempt automatic previews. A missed preview leaves the message pending until poll/ACK; members end idle when no work remains and wake on broker events. `member prompt` is the separate Director-controlled keystroke operation, with its shell follow-up governed by prompt-routing. A successful placement establishes pane existence; ready establishes that the member booted, and monitor live gates the first ordinary spawn.

Keep base skills and roles backend-neutral. Resolve runtime bindings for the executing agent, models and effort for the selected spawn backend, and pane cues for the observed member. Monitor bootstrap and recovery inherit the Director backend. The [unified reference](../../../skills/cafleet/reference/coding-agents.md) owns each backend's six ordered sections: Runtime bindings, Role defaults, Model catalog, Note → applies at, Pane-state capture cues, Worked resolution. Use its Template when adding a backend; the model-refresh skill owns catalog/default/freshness data while runtime maintenance owns bindings, notes and cues. Agent and public operator documentation remain independent homes per the overlay rule.

## Worked example: summarize-pr

This illustrative skill accepts a PR number, gives one Summarizer a read-only diff, and produces a 200-word summary with three risk areas. The paths, timestamp and returned IDs below are example values; resolve real paths, current model defaults and returned IDs for an actual run. Each shell command is a separate invocation.

### Resolve and bootstrap

For `/summarize-pr 1234` in `/repo`, normalize `researches/pr-1234` and resolve BASE to `/repo/researches/pr-1234`. Fetch the PR diff through the host's approved GitHub workflow and write this agent-only input with the file writer to `.inputs/diff.patch` under BASE. Supply a `roles/summarizer.md` defining read-only input, summary-only output, revision handling, broker reporting and immediate escalation of blockers; it loads CAFleet member startup and coordination.

Run `cafleet doctor`. On success, resolve the Director backend and its monitor default. For this example the resolved pair is `claude` and `haiku`. Render the canonical monitor frame referencing `/repo/skills/cafleet/roles/monitor.md` to `/repo/researches/pr-1234/.prompts/monitor-20260911T100000Z.md`, with BASE, all identity lines, resolved startup prerequisites and the monitor start cue. Bootstrap:

```bash
cafleet fleet create --name summarize-pr-1234 --coding-agent claude --monitor-file /repo/researches/pr-1234/.prompts/monitor-20260911T100000Z.md --monitor-model haiku --json
```

Suppose the result provides fleet 7, Director 8 and monitor 9. Carry those literal IDs thereafter. Read and ACK the monitor's ready and monitor live messages; the monitor owns launching and confirming its loop. Spawn the Summarizer only after that gate.

### Render and spawn

Write the following complete prompt to `/repo/researches/pr-1234/.prompts/summarizer-20260911T100100Z.md`. This is the pre-substitution audit input:

```text
You are the Summarizer in a summarize-pr team (CAFleet-native).

ROLE DEFINITION: Open /repo/skills/summarize-pr/roles/summarizer.md with an available text reader BEFORE any other action. That file is your authoritative role definition. Re-read it whenever unsure of protocol.

Load the cafleet skill at startup for broker and member protocols, resolving the backend named below. Read the summarize-pr workflow and CAFleet design-doc coordination before substantive work.

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /repo/researches/pr-1234
CODING AGENT: {coding_agent}

INPUT FILE: /repo/researches/pr-1234/.inputs/diff.patch
OUTPUT FILE: /repo/researches/pr-1234/summary.md

IMPORTANT: Keep the input and repository source unchanged; write only the summary and task-local audit artifacts.
IMPORTANT: The Director owns all Git operations and user communication.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Follow CAFleet member Bash and prompt-routing protocols plus the supplied host Bash rules: one command per call, literal arguments and a file writer for output.

When you see cafleet message poll output from the Director, read and ACK each message, then act on its instructions. Retrieve full payloads with JSON when needed.

Use an available non-shell text reader for prerequisites; shell file reads may precede ready when shell is the only reader.

On spawn, as your first operational broker shell command, send the ready signal: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"

Read INPUT FILE, write a 200-word summary highlighting three risk areas to OUTPUT FILE, then send complete (doc) to the Director. Handle revisions at standing COMMENT(director) markers and reply addressed (doc).
```

```bash
cafleet member create --fleet-id 7 --name summarizer --description "Summarizes a PR diff and three risks" --file /repo/researches/pr-1234/.prompts/summarizer-20260911T100100Z.md --json
```

Suppose the returned member ID is 11. The CLI renders that member's own identity; the Director uses 11 for later calls and follows the placement/ready checks in supervision.

### Review and revisions

After writing the summary, member 11 sends:

```bash
cafleet message send --from-member-id 11 --to-member-id 8 "complete (doc) — summary and three risk areas ready"
```

The Director polls and ACKs that message, reads `summary.md` and presents the result to the user. A requested revision becomes a `COMMENT(director)` at the top of that document, paired with:

```bash
cafleet message send --from-member-id 8 --to-member-id 11 "ready (doc)"
```

The Summarizer reads the marker, revises the summary, removes the addressed marker and sends `addressed (doc)`. The Director reviews the revised artifact. Full clarification payloads follow coordination's exemptions; routing summaries stay short and substance stays at its pointer.

### Teardown

Once the authorized work is complete, run each command separately:

```bash
cafleet member delete 9
```

```bash
cafleet member delete 11
```

```bash
cafleet member list 7
```

After confirming that only the root Director remains in the registry, delete the fleet:

```bash
cafleet fleet delete 7
```

Confirm closure with:

```bash
cafleet fleet list
```

Confirm fleet 7 is absent from the active fleet list before reporting teardown complete.

Member deletion kills its pane immediately; deleting the monitor first ends the wake source. Inspect any cleanup error before claiming teardown complete. Fleet deletion handles registry/runtime cleanup and preserves messages; member deletion handles panes. The summary and immutable prompt inputs remain under the task folder.
