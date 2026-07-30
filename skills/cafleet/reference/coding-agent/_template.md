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
| *Pane-state capture cues* (below) — this backend's `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate` discriminators. | the monitoring member's target-specific classification rubric — `cafleet/roles/monitor.md` § On each wake; the pane-state taxonomy in `docs/concepts/monitoring.md`; the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (both consumers apply the cues of the **target member's** backend overlay). |

## Pane-state capture cues

Required section. Give concrete capture-content discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`, from pane text alone and never native `agent_status`. `working` includes visible streaming, generation, tool execution, or any ambiguous/truncated state that might still be active. `stall_candidate` is quiet, non-finished content with no prompt and no active-work cue; it and `finished` are the two quiet families the monitoring member confirms itself — two byte-identical captures across consecutive stall-check wakes, recorded in its own notes, confirm the member quiet. Register the table in *Note → applies at* and bind it to both the monitoring member's target-specific rubric and the Director's pre-ping gate.

| State | <backend> capture cue |
|---|---|
| `awaiting_user` | <the concrete frame this backend shows when a pane is waiting on a user answer or approval — the box/prompt shape, awaiting a keypress> |
| `finished` | <the concrete frame this backend shows at a completed turn — the empty composer/prompt at rest, no pending box, no active-turn indicator> |
| `working` | <affirmative streaming/generation/tool/active-turn indicators; include ambiguous or truncated content that may still be active> |
| `stall_candidate` | <quiet non-finished content with no prompt, no empty finished composer, and no affirmative active-work cue> |

State both ambiguity tie-breaks: `awaiting_user` wins over `finished`, and `working` wins over `stall_candidate`.

## Worked resolution

Required section. Give the canonical monitor-spawn command fully resolved for this backend — every `{placeholder}` replaced by its concrete value — so the reader has a concrete string to match rather than a transformation to invent:

`cafleet member create --role monitor --model <this backend's monitor model> --text-file <rendered monitor prompt>` (members spawned `<this backend's permission flags>`).
