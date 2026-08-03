# Overlay: codex

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
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
| *Pane-state capture cues* (below) — the concrete codex-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the Director's on-tick health check and pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in `docs/docs/concepts/monitoring.md` (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The Director classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | codex capture cue |
|---|---|
| `awaiting_user` | A codex confirmation/approval prompt awaiting a keypress — a `[y/n]`-style command-approval or an out-of-sandbox escalation request. Under `--ask-for-approval never --sandbox workspace-write` codex auto-approves in-workspace work, so a visible approval prompt means codex hit something the policy could not clear and is genuinely waiting. |
| `finished` | The empty codex composer at rest — the input prompt with no streaming output above it and no active-turn / "working" indicator. |
| `working` | Affirmative active-work evidence: the active-turn or `working` indicator, streaming model output, a running tool, or generation in progress. A truncated or ambiguous capture that may still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no confirmation prompt, no empty at-rest composer, and no active-turn, tool, streaming, generation, or other work cue. |

When a capture cannot separate `awaiting_user` from `finished`, classify `awaiting_user`. When it cannot separate active work from a quiet candidate, classify `working`. `stall_candidate` and `finished` are the two quiet families: the Director confirms a member quiet when its captures on two consecutive wakes are byte-identical.

## Worked resolution

The canonical Director-side monitor launch, fully resolved for this backend:

Launch `cafleet monitor start --fleet-id <fleet-id> &` as a backgrounded `!` shell command, and confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in its output before the first `member create` (members spawned `--ask-for-approval never --sandbox workspace-write`).
