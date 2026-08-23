# Coding-agent overlay

cafleet skill instructions are backend-neutral by default. Write the base — every cafleet-family `SKILL.md`, every per-workflow body (`<workflow>/<workflow>.md`), every `roles/*.md`, and every `reference/*.md` page (other than the per-backend sections of `skills/cafleet/reference/coding-agent-overlays.md`) — so it reads the same for an agent on any coding-agent backend.

## Where backend specifics live

Backend-specific deltas live in the single file `skills/cafleet/reference/coding-agent-overlays.md`, one self-contained top-level section per backend (`## claude`, `## codex`, `## opencode`), with the canonical skeleton in the `## Template` section — adding a backend means copying the Template section into a new `## <name>` section of the same file. Each backend section carries that backend's concrete realization of the backend deltas: the decision surface, the auto-approval / permission flags, the long-lived-execution primitives, pane discovery / pane title, the effort levels, and the skill-loading recipe. Model policy stays with the Director and the model list — availability, capability classes, and prices live in `skills/cafleet/reference/model-list.md`, and every spawn's backend/model pair is the Director's pre-spawn choice from that list; each backend section's `{monitor_model}` and `{reviewer_model}` values mirror the list's *Monitor and reviewer defaults* table and are refreshed with it.

## Reader contract

Every backend section is fully self-contained: its own placeholder table, its own *Note → applies at* table, its own pane-state capture cues (with the tie-break pointer sentence), and its own worked resolution. Cross-section references ("same as claude") are forbidden — self-containment is what keeps single-section reading sufficient and resolution deterministic.

A reader resolves **only its own backend's section**, identified from the spawn prompt's `CODING AGENT:` line (a standalone agent uses its own identity). Values in other sections are never applicable: a value taken from another backend's section is a resolution defect, the same class as emitting a literal `{token}`. The cross-section readers are the Director and the monitor member — each applies the **target member's** backend section for pane-state cues.

This sectioning is the accepted substitute for context isolation: a Read of the merged file pulls every backend's values into the reader's context, and the self-contained sections plus the resolve-only-your-section rule are what keep overlay resolution deterministic regardless.

## How the base and overlay connect

Every base instruction that varies by backend states the neutral behavior and points the agent at its overlay section. The overlay is not an in-stream aside to consult lazily: it is **row #1 of each reader entry point's Required-reading block** — the first load-bearing read, gated before any other action, linked as the merged file with the reader's per-backend anchor (`coding-agent-overlays.md#<name>`). That row is **read-and-resolve**, not read-only. An agent identifies its coding agent — the spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity — reads `reference/coding-agent-overlays.md` first, then **resolves** its own backend's section before its first action:

1. **Materialize values.** For every `{placeholder}` token it will use, take the concrete value from its backend section's table and use that literal value — never the brace token.
2. **Apply notes.** At each base instruction named in its backend section's *Note → applies at* table, follow that note's caveat there.
3. **Self-check at emission.** A literal `{token}` in any command run, any message sent, or anything shown to the user is a defect — resolve it before emitting.

Skip resolution and the agent emits a literal `{skill_loader}` / `{permission_flags}`, guesses a wrong or default value, or ignores a backend note — the three application-failure modes the resolve step closes. The canonical procedure, with the resolution order, lives in `skills/cafleet/SKILL.md` § *Resolve your overlay*.

**Documented defaults.** When a backend section omits a token, or an agent cannot identify its backend, each token has a documented backend-neutral default: the lowest-common-denominator form that functions on every backend (message-only coordination, POSIX backgrounding, a neutral mode description). This is a legitimate default per `affirmative-writing.md` — absence of a section value is an expected, valid state with a well-defined correct behavior — not an error-swallowing fallback. The default table is canonical in `skills/cafleet/SKILL.md` § *Resolve your overlay*.

Authors write the base neutrally, put every backend specific in that backend's section of the merged file, and list the overlay section as the first load-bearing read wherever an entry point uses `{placeholder}` tokens.

## Two independent homes

The agent-facing overlay home (`skills/cafleet/reference/coding-agent-overlays.md`) and the human-facing operator docs (`docs/docs/spec/coding-agent-backends.md`) serve different audiences and stay independent: they never cross-link in either direction. Restating the same operational fact in both homes is fine; linking between them is not.
