# Overlay: <backend name>

Substitute these into the base `{…}` placeholders. Fill every row with this backend's concrete value, phrased as a noun phrase that reads correctly when substituted inline into a base sentence; keep values short.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | <this backend's recorded-user-reaction surface as a noun phrase: its interactive prompt tool, or "a plain operator message relayed by the Director" — a fleet member always routes its question to the Director> |
| `{monitor_model}` | <cheapest capable model for the monitor on this backend> |
| `{member_model}` | <general `--model` example value for an ordinary member on this backend> |
| `{permission_flags}` | <the exact spawn flags for workspace-scoped auto-approval> |
| `{bg_run}` | <this backend's primitive for running long-lived background work, as a noun phrase> |
| `{bg_stop}` | <the matching stop primitive, as a noun phrase> |
| `{task_coord}` | <this backend's task-list primitive, or "cafleet messages (no harness task list)"> |
| `{pane_title}` | <any `--name`-style pane-title analog, or "no `--name` pane-title analog (locate panes via `cafleet member list`)"> |
| `{skill_loader}` | <the skill-loader, or the read-by-absolute-path fallback, as a noun phrase> |
