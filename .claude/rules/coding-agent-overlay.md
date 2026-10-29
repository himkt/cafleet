# Coding-agent overlay

Write CAFleet base instructions so they read the same on every backend: family `SKILL.md` files, workflow bodies, roles, and shared reference pages prescribe neutral behavior and point to the relevant runtime overlay.

## Where backend specifics live

`skills/cafleet/reference/coding-agents.md` is the canonical backend reference. Each self-contained backend section contains these four subsections in order:

1. **Runtime bindings**: the five runtime placeholders for decision surface, permission flags, pane title, skill loader, and effort levels.
2. **Role defaults**: the concrete model and effort assignments for reviewers and other members; models come from the backend catalog and effort matches its runtime capability.
3. **Model catalog**: exact spawn tokens, aliases, reviewed capability classes/order, prices, context notes, and official provenance.
4. **Note → applies at**: constraints bound to their tokens and consuming instructions.

Backend-specific `####` subsections, such as a native reception recipe, may follow Note → applies at. Add a backend by copying `## Template` and supplying all four subsections. Keep each section complete for its own backend, with shared policy and supervision linked to their authoritative homes. Model selection belongs to `skills/cafleet/roles/director.md` § Model selection. The `cafleet-model-list-refresh` skill owns catalogs, provenance/context notes, Role defaults, and shared freshness metadata; runtime documentation maintenance owns the remaining subsections.

## How the base and overlay connect

Each reader entry point places its runtime overlay first in its Required-reading block — as the first table row or the first prose reference — gated before action. Link `coding-agents.md#<name>`, substituting the reader's concrete backend anchor when used. The reader resolves the overlay with the procedure in `skills/cafleet/SKILL.md` § Resolve your overlay, which owns the subject selection, materialization, note application, emission self-check and documented neutral defaults. Report a missing required supported-backend section, malformed table, or broken reference as a documentation defect.

## Three homes

| Home | Audience | Content |
|---|---|---|
| `SPEC.md` §6.7 Coding-agent backends | Everything (contract) | Backend argv, model/effort validation, permission posture, presets and host evidence |
| `skills/cafleet/reference/coding-agents.md` | Agents | The runtime overlay above |
| `docs/docs/concepts/coding-agents.md` | Users | How backends differ and how to choose and configure them |

State shared operational facts in each home for its audience. Skills never link `docs/` or `SPEC.md`, and `SPEC.md` never links `docs/`.
