# Overlay: opencode

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{monitor_model}` | `anthropic/claude-haiku-4-5` |
| `{reviewer_model}` | `opencode/gpt-5.5-pro` |
| `{permission_flags}` | `--agent cafleet` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides |

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. The `--agent cafleet` safety floor shows no popup; if a popup ever appears it is a regression to escalate, not a decision point. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| No harness task list — track sub-topic registrations, claims, and completions as cafleet messages. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |
| *Pane-state capture cues* (below) — the concrete opencode-pane discriminators for `awaiting_user` vs `finished`. | classification rubric rule 1 (`awaiting_user`) — `cafleet/roles/monitor.md` § On each wake (step 2); the pane-state taxonomy in `docs/concepts/monitoring.md`; the Director's pre-nudge capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The monitoring member classifies each captured pane from its **content only** (never native `agent_status`). These are the opencode-backend discriminators for `awaiting_user` (rule 1, the destructive-if-missed class) vs `finished` (rule 3):

| State | opencode capture cue |
|---|---|
| `awaiting_user` | An opencode permission/selection popup awaiting a choice. The `--agent cafleet` floor suppresses permission popups, so a visible popup is both the `awaiting_user` signal **and** a regression to escalate (see the decision-surface note above). |
| `finished` | The empty opencode prompt at rest — no streaming response above it and no active generation indicator. |

When a capture cannot cleanly separate the two, apply the ambiguity tie-break and classify `awaiting_user`.

## Worked resolution

The canonical monitor-spawn command, fully resolved for this backend:

`cafleet member create --role monitor --model anthropic/claude-haiku-4-5 --text-file <rendered monitor prompt>` (members spawned `--agent cafleet`).
