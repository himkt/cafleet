# Overlay: <backend name>

Substitute these into the base `{…}` placeholders. Each value must be a short noun phrase that reads correctly when substituted inline into a base sentence; push any constraint or caveat into the *Note → applies at* table below (a required section), where each note names the base token/instruction it qualifies.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | <this backend's recorded-user-reaction surface as a short noun phrase: its interactive prompt tool, or "a Director-relayed operator message" — a fleet member always routes its question to the Director> |
| `{monitor_model}` | <this backend's monitor default from the model list's *Monitor and reviewer defaults* table> |
| `{reviewer_model}` | <this backend's reviewer default from the same table> |
| `{permission_flags}` | <the exact spawn flags for workspace-scoped auto-approval> |
| `{bg_run}` | <this backend's primitive for running long-lived background work, as a noun phrase> |
| `{bg_stop}` | <the matching stop primitive, as a noun phrase> |
| `{task_coord}` | <this backend's task-list primitive, or "cafleet messages"> |
| `{pane_title}` | <any `--name`-style pane-title analog, or "no `--name` analog"> |
| `{skill_loader}` | <the skill-loader, or the read-by-absolute-path fallback, as a noun phrase> |
| `{effort_levels}` | <this backend's accepted reasoning-effort levels and spawn flag for `cafleet member create --effort`, or "unsupported — omit `--effort`"> |

## Note → applies at

Required section. Convert every note (a constraint/caveat the inline value shouldn't carry — e.g. the decision surface's question-shape taxonomy, or "no harness task list" for task coordination) into a row of this table. **Every note names the base token/instruction it qualifies**: the *Applies at* cell leads with the `{token}` the note binds to, followed by the base section(s) where it takes effect (`<skill>/<file>` § <heading>). A floating note with no bound anchor is not allowed.

| Note | Applies at |
|------|-----------|
| <the caveat, one row each> | `{token}` — `<skill>/<file>` § <base heading> |
| *Pane-state capture cues* (below) — this backend's `awaiting_user` vs `finished` discriminators. | classification rubric rule 1 (`awaiting_user`) — `cafleet/roles/monitor.md` § On each wake; the pane-state taxonomy in `docs/concepts/monitoring.md`; the Director's pre-nudge capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

Required section. Give this backend's concrete capture-content discriminators for the two pane states a single capture most easily confuses — `awaiting_user` (classification rubric rule 1, the destructive-if-missed class) vs `finished` (rule 3) — so both consumers can tell them apart from pane text alone, never from native `agent_status`: the monitoring member's classification rubric and the Director's pre-nudge capture gate (`cafleet/reference/supervision.md` § Idle Semantics). Register a row for this table in the *Note → applies at* table above, bound to both consumers — the rubric's rule 1 and the Director's pre-nudge gate.

| State | <backend> capture cue |
|---|---|
| `awaiting_user` | <the concrete frame this backend shows when a pane is waiting on a user answer or approval — the box/prompt shape, awaiting a keypress> |
| `finished` | <the concrete frame this backend shows at a completed turn — the empty composer/prompt at rest, no pending box, no active-turn indicator> |

State the ambiguity tie-break: when a capture cannot cleanly separate the two, classify `awaiting_user`.

## Worked resolution

Required section. Give the canonical monitor-spawn command fully resolved for this backend — every `{placeholder}` replaced by its concrete value — so the reader has a concrete string to match rather than a transformation to invent:

`cafleet member create --role monitor --model <this backend's monitor model> --text-file <rendered monitor prompt>` (members spawned `<this backend's permission flags>`).
