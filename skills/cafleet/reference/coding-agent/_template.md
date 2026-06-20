# Overlay: <backend name>

Substitute these into the base `{…}` placeholders. Each value must be a short noun phrase that reads correctly when substituted inline into a base sentence; push any constraint or caveat to a note line below the table.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | <this backend's recorded-user-reaction surface as a short noun phrase: its interactive prompt tool, or "a Director-relayed operator message" — a fleet member always routes its question to the Director> |
| `{monitor_model}` | <cheapest capable model for the monitor on this backend> |
| `{permission_flags}` | <the exact spawn flags for workspace-scoped auto-approval> |
| `{bg_run}` | <this backend's primitive for running long-lived background work, as a noun phrase> |
| `{bg_stop}` | <the matching stop primitive, as a noun phrase> |
| `{task_coord}` | <this backend's task-list primitive, or "cafleet messages"> |
| `{pane_title}` | <any `--name`-style pane-title analog, or "no `--name` analog"> |
| `{skill_loader}` | <the skill-loader, or the read-by-absolute-path fallback, as a noun phrase> |

Notes (one line each, only where a value needs a constraint/caveat the inline value shouldn't carry — e.g. the decision surface's question-shape taxonomy, or "no harness task list" for task coordination).
