# Coding-agent overlay

cafleet skill instructions are backend-neutral by default. Write the base — every cafleet-family `SKILL.md`, every per-workflow body (`<workflow>/<workflow>.md`), every `roles/*.md`, and every `reference/*.md` page (outside `skills/cafleet/reference/coding-agent/`) — so it reads the same for an agent on any coding-agent backend.

## Where backend specifics live

Backend-specific deltas live in `skills/cafleet/reference/coding-agent/<name>.md`, one overlay per backend (`claude`, `codex`, `opencode`), with the canonical skeleton in `_template.md`. Each overlay carries that backend's concrete realization of the six deltas: the decision surface, the monitor model, the auto-approval / permission flags, the background-task + task-list primitives, pane discovery / pane title, and the skill-loading recipe.

## How the base and overlay connect

Every base instruction that varies by backend states the neutral behavior and points the agent at its overlay. An agent reads the base, identifies its coding agent — the spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity — reads `reference/coding-agent/<name>.md`, and applies the overlay's deltas on top of every base instruction. Authors write the base neutrally and put every backend specific in the overlay.

## Two independent homes

The agent-facing overlay home (`skills/cafleet/reference/coding-agent/`) and the human-facing operator docs (`docs/reference/coding-agents/`) serve different audiences and stay independent: they never cross-link in either direction. Restating the same operational fact in both homes is fine; linking between them is not.
