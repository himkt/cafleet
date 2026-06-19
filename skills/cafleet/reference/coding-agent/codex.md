# Overlay: codex

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a plain operator message relayed by the Director (no in-pane prompt; the question to the Director must be a concrete, answerable ask, not free-form prose) |
| `{monitor_model}` | `gpt-5.4-mini` |
| `{member_model}` | `gpt-5.5` |
| `{permission_flags}` | `--ask-for-approval never --sandbox workspace-write` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing that background process |
| `{task_coord}` | cafleet messages (no harness task list) |
| `{pane_title}` | no `--name` pane-title analog (locate panes via `cafleet member list`) |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides (cannot load skills) |
