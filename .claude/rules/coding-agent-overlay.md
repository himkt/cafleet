# Coding-agent overlay

Write CAFleet base instructions so they read the same on every backend: family `SKILL.md` files, workflow bodies, roles, and shared reference pages prescribe neutral behavior and point to the relevant runtime overlay.

## Where backend specifics live

`skills/cafleet/reference/coding-agents.md` is the canonical backend reference. Each self-contained `## claude`, `## codex`, or `## opencode` section contains these six subsections in order:

1. **Runtime bindings**: the seven runtime placeholders for decision surface, permission flags, background run/stop, pane title, skill loader, and effort levels.
2. **Role defaults**: the sole concrete assignments of `{monitor_model}` and `{reviewer_model}` for that backend, drawn from its catalog.
3. **Model catalog**: exact spawn tokens, aliases, reviewed capability classes/order, prices, context notes, and official provenance.
4. **Note → applies at**: constraints bound to their tokens and consuming instructions.
5. **Pane-state capture cues**: the four states and the supervision tie-break pointer.
6. **Worked resolution**: the fully resolved monitor-loop launch.

Add a backend by copying `## Template` and supplying all six subsections. Keep each section complete for its own backend, with shared policy and supervision linked to their authoritative homes. Model selection belongs to `skills/cafleet/roles/director.md` § Model selection. The `cafleet-model-list-refresh` skill owns catalogs, provenance/context notes, Role defaults, and shared freshness metadata; runtime documentation maintenance owns the remaining subsections.

## Reader contract

Resolve by the subject of the action:

| Purpose | Backend selector | Values |
|---|---|---|
| Execute current instructions | Executing agent's `CODING AGENT:` identity, or its own identity when standalone | Its Runtime bindings and bound notes, including local execution and decision tools. |
| Select/configure a spawned member | Backend selected under Director policy | Its Model catalog and Role defaults; validate its effort and launch capabilities against Runtime bindings. |
| Interpret a captured pane | Observed member's recorded backend | Its capture cues, retaining the observer's own tools and decision surface. |

For example, a Codex Director selecting an OpenCode reviewer resolves the OpenCode reviewer default and effort capability while keeping Codex decision and execution tools. A Claude monitor observing Codex applies Codex capture cues while its own loop uses Claude execution. Monitor bootstrap and recovery inherit the Director's backend. Ordinary members resolve their own runtime section without acquiring model-selection duties.

## How the base and overlay connect

Each reader entry point places its runtime overlay at **row #1 of the Required-reading block**, gated before action. Link `coding-agents.md#<name>`, substituting the reader's concrete backend anchor when used. The row requires **read-and-resolve**: identify the executing backend, read its Runtime bindings, and apply the core skill's resolution procedure before acting. Directors additionally read the selected spawn backend when selecting a member.

1. **Materialize values.** Use the seven runtime bindings for the relevant operation and the selected spawn backend's two Role defaults.
2. **Apply notes.** Follow each caveat at the instruction named in Note → applies at.
3. **Self-check at emission.** Emit concrete command and message values; resolve any remaining literal `{token}` first.

The canonical procedure and documented neutral defaults live in `skills/cafleet/SKILL.md` § Resolve your overlay. Use those defaults only for the explicitly allowed missing/unknown-backend cases. Report a missing required supported-backend section, malformed table, or broken reference as a documentation defect.

## Two independent homes

Maintain the agent-facing reference (`skills/cafleet/reference/coding-agents.md`) and human-facing operator documentation (`docs/docs/spec/coding-agent-backends.md`) independently. State shared operational facts in each home for its audience; never cross-link these homes in either direction.
