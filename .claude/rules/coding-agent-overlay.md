# Coding-agent overlay

cafleet skill instructions are backend-neutral by default. Write the base — every cafleet-family `SKILL.md`, every per-workflow body (`<workflow>/<workflow>.md`), every `roles/*.md`, and every `reference/*.md` page (outside `skills/cafleet/reference/coding-agent/`) — so it reads the same for an agent on any coding-agent backend.

## Where backend specifics live

Backend-specific deltas live in `skills/cafleet/reference/coding-agent/<name>.md`, one overlay per backend (`claude`, `codex`, `opencode`), with the canonical skeleton in `_template.md`. Each overlay carries that backend's concrete realization of the six deltas: the decision surface, the per-role model pins (monitor + reviewer), the auto-approval / permission flags, the background-task + task-list primitives, pane discovery / pane title, and the skill-loading recipe.

## How the base and overlay connect

Every base instruction that varies by backend states the neutral behavior and points the agent at its overlay. The overlay is not an in-stream aside to consult lazily: it is **row #1 of each reader entry point's Required-reading block** — the first load-bearing read, gated before any other action. That row is **read-and-resolve**, not read-only. An agent identifies its coding agent — the spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity — reads `reference/coding-agent/<name>.md` first, then **resolves** it before its first action:

1. **Materialize values.** For every `{placeholder}` token it will use, take the concrete value from the overlay's table and use that literal value — never the brace token.
2. **Apply notes.** At each base instruction named in the overlay's *Note → applies at* table, follow that note's caveat there.
3. **Self-check at emission.** A literal `{token}` in any command run, any message sent, or anything shown to the user is a defect — resolve it before emitting.

Skip resolution and the agent emits a literal `{monitor_model}` / `{permission_flags}`, guesses a wrong or default value, or ignores a backend note — the three application-failure modes the resolve step closes. The canonical procedure, with the resolution order, lives in `skills/cafleet/SKILL.md` § *Resolve your overlay*.

**Documented defaults.** When an overlay omits a token, or an agent cannot identify its backend, each token has a documented backend-neutral default: the lowest-common-denominator form that functions on every backend (message-only coordination, POSIX backgrounding, a neutral mode description). This is a legitimate default per `affirmative-writing.md` — absence of an overlay value is an expected, valid state with a well-defined correct behavior — not an error-swallowing fallback. The default table is canonical in `skills/cafleet/SKILL.md` § *Resolve your overlay*.

Authors write the base neutrally, put every backend specific in the overlay, and list the overlay as the first load-bearing read wherever an entry point uses `{placeholder}` tokens.

## Two independent homes

The agent-facing overlay home (`skills/cafleet/reference/coding-agent/`) and the human-facing operator docs (`docs/reference/coding-agents/`) serve different audiences and stay independent: they never cross-link in either direction. Restating the same operational fact in both homes is fine; linking between them is not.
