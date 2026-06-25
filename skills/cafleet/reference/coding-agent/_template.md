# Overlay: <backend name>

Substitute these into the base `{…}` placeholders. Each value must be a short noun phrase that reads correctly when substituted inline into a base sentence; push any constraint or caveat into the *Note → applies at* table below (a required section), where each note names the base token/instruction it qualifies.

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

## Note → applies at

Required section. Convert every note (a constraint/caveat the inline value shouldn't carry — e.g. the decision surface's question-shape taxonomy, or "no harness task list" for task coordination) into a row of this table. **Every note names the base token/instruction it qualifies**: the *Applies at* cell leads with the `{token}` the note binds to, followed by the base section(s) where it takes effect (`<skill>/<file>` § <heading>). A floating note with no bound anchor is not allowed.

| Note | Applies at |
|------|-----------|
| <the caveat, one row each> | `{token}` — `<skill>/<file>` § <base heading> |

## Worked resolution

Required section. Give the canonical monitor-spawn command fully resolved for this backend — every `{placeholder}` replaced by its concrete value — so the reader has a concrete string to match rather than a transformation to invent:

`cafleet agent spawn --role monitor --model <this backend's monitor model>` (members spawned `<this backend's permission flags>`).
