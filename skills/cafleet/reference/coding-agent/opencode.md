# Overlay: opencode

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a plain operator message relayed by the Director (no in-pane prompt; the `--agent cafleet` safety floor shows no popup, so a popup is a regression to escalate) |
| `{monitor_model}` | `anthropic/claude-haiku-4-5` |
| `{member_model}` | `anthropic/claude-sonnet-4-6` |
| `{permission_flags}` | `--agent cafleet` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing that background process |
| `{task_coord}` | cafleet messages (no harness task list) |
| `{pane_title}` | no `--name` pane-title analog (locate panes via `cafleet member list`) |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides (cannot load skills) |
