# Overlay: codex

Apply these deltas on top of the cafleet base. The base states each instruction in backend-neutral terms and points here; this overlay states how the OpenAI Codex CLI realizes it.

## 1. Decision surface

codex has no interactive in-pane prompt for soliciting a user decision, so the relay through the Director IS the decision surface. When a recorded user reaction is needed (approve / choose / confirm / continue-or-abort), a fleet **member** sends its question to the Director via `cafleet message send`, and the **Director** answers as a plain operator message (read-then-respond cadence). The base's no-free-form-prose rule still holds for the member→Director hop: the question must be a concrete, answerable ask — a specific choice or yes/no — not "let me know what you think".

## 2. Monitor model

`--model gpt-5.4-mini` — the cheapest capable model for the codex backend (cheaper than `gpt-5.5`). The Director substitutes this into the `cafleet member create … --role monitor` spawn command only when it spawns the monitor with `--coding-agent codex`.

## 3. Auto-approval / permission mode

`--ask-for-approval never --sandbox workspace-write`. Together these are the codex equivalent of workspace-scoped auto-approval: interactive approval prompts are disabled, the Bash tool is enabled, and the member runs cafleet (and other shell commands) directly, confined to writing within the workspace.

## 4. Background-task + task-list primitives

codex has no harness task primitive. Run long-lived background work (e.g. the Slidev dev server) via the leading-`!` shell shortcut backgrounded, stop it at teardown, and coordinate parallel sub-work via cafleet messages.

## 5. Pane discovery / pane title

`cafleet member list` (the `pane_id` column) is ground truth for locating a member's pane. codex has no `--name` pane-title analog, so locate codex panes via `cafleet member list`.

## 6. Skill-loading recipe

codex cannot load Claude Code skills. Read the cafleet `SKILL.md` core and this overlay (`reference/coding-agent/codex.md`) by the absolute paths your spawn prompt provides.
