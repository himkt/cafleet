# Overlay: opencode

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{monitor_model}` | `opencode/deepseek-v4-flash-free` |
| `{reviewer_model}` | `opencode/glm-5.2` |
| `{permission_flags}` | `--agent cafleet` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides |
| `{effort_levels}` | unsupported — omit `--effort` |

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. The `--agent cafleet` safety floor shows no popup; if a popup ever appears it is a regression to escalate, not a decision point. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| No harness task list — track sub-topic registrations, claims, and completions as cafleet messages. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |
| *Pane-state capture cues* (below) — the concrete opencode-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitoring member's target-specific classification rubric — `cafleet/roles/monitor.md` § On each wake; the pane-state taxonomy in `docs/docs/concepts/monitoring.md`; the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (both consumers apply the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The monitoring member classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | opencode capture cue |
|---|---|
| `awaiting_user` | An opencode permission/selection popup awaiting a choice. The `--agent cafleet` floor suppresses permission popups, so a visible popup is both the `awaiting_user` signal **and** a regression to escalate (see the decision-surface note above). |
| `finished` | The empty opencode prompt at rest — no streaming response above it and no active generation indicator. |
| `working` | Affirmative active-work evidence: streaming response text, an active generation indicator, a running tool, or any other visible in-progress state. A truncated or ambiguous capture that might still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no popup, no empty at-rest prompt, and no streaming, tool, generation, or other active-work cue. |

When a capture cannot separate `awaiting_user` from `finished`, classify `awaiting_user`. When it cannot separate active work from a quiet candidate, classify `working`. `stall_candidate` and `finished` are the two quiet families: the monitoring member itself confirms a member quiet when its captures on two consecutive stall-check wakes are byte-identical.

## Worked resolution

The canonical monitor-spawn command, fully resolved for this backend:

`cafleet member create --role monitor --model opencode/deepseek-v4-flash-free --text-file <rendered monitor prompt>` (members spawned `--agent cafleet`).
