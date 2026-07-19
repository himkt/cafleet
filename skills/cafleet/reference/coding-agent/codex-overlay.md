# Overlay: codex

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{monitor_model}` | `gpt-5.4-mini` |
| `{reviewer_model}` | `gpt-5.6-sol` |
| `{permission_flags}` | `--ask-for-approval never --sandbox workspace-write` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`, forwarded as `--config=model_reasoning_effort=<level>`) |

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. Ask a concrete, answerable question, not free-form prose. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| No harness task list — track sub-topic registrations, claims, and completions as cafleet messages. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |
| *Pane-state capture cues* (below) — the concrete codex-pane discriminators for `awaiting_user` vs `finished`. | classification rubric rule 1 (`awaiting_user`) — `cafleet/roles/monitor.md` § On each wake (step 2); the pane-state taxonomy in `docs/concepts/monitoring.md`; the Director's pre-nudge capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The monitoring member classifies each captured pane from its **content only** (never native `agent_status`). These are the codex-backend discriminators for `awaiting_user` (rule 1, the destructive-if-missed class) vs `finished` (rule 3):

| State | codex capture cue |
|---|---|
| `awaiting_user` | A codex confirmation/approval prompt awaiting a keypress — a `[y/n]`-style command-approval or an out-of-sandbox escalation request. Under `--ask-for-approval never --sandbox workspace-write` codex auto-approves in-workspace work, so a visible approval prompt means codex hit something the policy could not clear and is genuinely waiting. |
| `finished` | The empty codex composer at rest — the input prompt with no streaming output above it and no active-turn / "working" indicator. |

When a capture cannot cleanly separate the two, apply the ambiguity tie-break and classify `awaiting_user`.

## Worked resolution

The canonical monitor-spawn command, fully resolved for this backend:

`cafleet member create --role monitor --model gpt-5.4-mini --text-file <rendered monitor prompt>` (members spawned `--ask-for-approval never --sandbox workspace-write`).
