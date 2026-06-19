# Overlay: opencode

Apply these deltas on top of the cafleet base. The base states each instruction in backend-neutral terms and points here; this overlay states how the opencode TUI realizes it.

## 1. Decision surface

opencode has no `AskUserQuestion` analog. In normal operation the TUI shows no permission popup — the `--agent cafleet` safety floor resolves every check to `allow` or `deny`, so there is no runtime decision point to relay. When a recorded user reaction is needed, a fleet **member** sends its question to the Director via `cafleet message send`; the **Director** answers as a plain operator message (the read-then-respond cadence). If a permission popup ever appears it is a regression escape from the safety floor, not a decision point: escalate to the user and capture pane state for diagnosis rather than answering it.

## 2. Monitor model

`--model anthropic/claude-haiku-4-5` — the cheapest capable model for the opencode backend, in opencode's required `<provider-id>/<model-id>` form. The Director substitutes this into the `cafleet member create … --role monitor` spawn command only when it spawns the monitor with `--coding-agent opencode`.

## 3. Auto-approval / permission mode

`--agent cafleet`. This binds the spawn to the `cafleet` agent definition, whose inline permission ruleset (catch-all allow first, then specific denies) resolves every permission check to `allow` or `deny` — nothing falls through to opencode's `ask` state. The Bash tool is enabled and routine commands run directly. This is the safety floor.

## 4. Background-task + task-list primitives

opencode has no harness task primitive. Run long-lived background work (e.g. the Slidev dev server) via the leading-`!` shell shortcut backgrounded, stop it at teardown, and coordinate parallel sub-work via cafleet messages.

## 5. Pane discovery / pane title

`cafleet member list` (the `pane_id` column) is ground truth for locating a member's pane. opencode has no `--name` pane-title analog, so locate opencode panes via `cafleet member list`.

## 6. Skill-loading recipe

opencode cannot load Claude Code skills. Read the cafleet `SKILL.md` core and this overlay (`reference/coding-agent/opencode.md`) by the absolute paths your spawn prompt provides.
