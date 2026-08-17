# Atomic Fleet + Monitor Bootstrap

**Status**: Approved
**Progress**: 14/14 tasks complete
**Last Updated**: 2026-08-17

## Overview

`cafleet fleet create` absorbs the monitor-member bootstrap: one command atomically creates the fleet, registers the root Director, registers the monitor member, and spawns the monitor's coding-agent pane. This simplifies the team-startup instruction surface and reduces the Director's startup CLI commands from three (`fleet create` → `member create --role monitor` → first ordinary `member create`) to two (GitHub issue #314).

## Success Criteria

- [x] `cafleet fleet create --name <n> --coding-agent <a> --monitor-file <path> [--monitor-model <m>]` creates the fleet, the Director, and the monitor member (registration + pane) in one invocation.
- [x] Any failure during the command leaves no fleet row, no Director row, no monitor row, and no placement rows behind; a spawned pane is killed on a post-spawn failure. The command is retryable as-is.
- [x] `cafleet member create --role monitor` still works as the mid-run recovery path for re-spawning a dead monitor; the one-per-fleet and monitor-first guards are unchanged.
- [x] The Director-facing skill instructions (supervision spawn protocol) describe the two-command startup; no skill or doc page still instructs spawning the monitor via `member create --role monitor` at bootstrap.
- [x] `mise //cafleet:test` and `mise //cafleet:lint` pass; SPEC.md, cli-options.md, and the pinned output-shape tests reflect the new contract.

---

## Background

Today the Director's bootstrap is: `cafleet doctor` → `cafleet fleet create` (atomic fleet + Director, pure DB) → write the monitor spawn prompt to `${BASE}/.prompts/monitor-<UTC-compact>.md` → `cafleet member create --fleet-id <id> --role monitor --model {monitor_model} --file <path>` → wait for `ready` then `monitor live` → first ordinary `member create`. The monitor-first sequencing is enforced twice: by skill instructions (belt) and by the CLI's monitor-first guard in `member create` (suspenders). The user wants the fleet and its monitor created atomically so the instruction surface shrinks and the Director issues fewer startup commands.

Decisions fixed by user interview (relayed 2026-08-16):

| Question | Decision |
|---|---|
| Scope | `fleet create` absorbs fleet + Director + monitor registration + monitor pane spawn. The monitor member (LLM) still launches the `cafleet monitor` wake loop in its own pane and still sends `monitor live`. |
| Opt-out | None. `fleet create` always spawns the monitor; no `--no-monitor` flag. |
| Prompt source | Director-authored, always. A required `--monitor-file <path>` flag carries it; the prompt text is not embedded in the binary. |
| Monitor model | No baked per-backend defaults in the binary. The model catalog lives in the skills (model-list.md / overlays); the Director passes the model explicitly via `--monitor-model`. |
| Failure handling | Literally atomic: on monitor spawn failure, roll back everything — no fleet, no Director row; exit with an error. |
| Recovery | `member create --role monitor` remains the mid-run re-spawn path; no new command. |

---

## Specification

### CLI surface — `cafleet fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Unchanged. |
| `--coding-agent` | yes | Unchanged: one of `claude` / `codex` / `opencode`, recorded on the Director's placement. The monitor member inherits this backend by construction — there is no `--monitor-coding-agent`. |
| `--monitor-file <PATH>` | yes | UTF-8 file whose contents are the monitor's spawn prompt; `-` reads stdin. Same body semantics as `member create --file` (empty/non-UTF-8 rejection); there is no inline positional form. |
| `--monitor-model <MODEL>` | no | Model passed to the monitor's backend binary; validated by the `--coding-agent` backend exactly as `member create --model`. When omitted the backend spawns on its own default model. |
| `--json` | no | Unchanged. |

There is no `--monitor-effort` flag and no monitor name/description flags: the monitor member is registered with the hardcoded identity `name="monitor"`, `description="Monitor member for this fleet"` (mirroring the Director's hardcoded `name="Director"`, `description="Root Director for this fleet"`).

### Execution order (single-transaction ladder)

All registration writes ride one SQLite transaction; the pane split happens inside the ladder, before commit. This is what makes the rollback total — contrast with `member create`, which commits the registration first and compensates with deletes.

1. **Multiplexer preconditions** (unchanged): resolve mux, `ensure_available`, `context_discovery`. Failure → existing `cafleet fleet create must be run inside a tmux or herdr session` error, exit 1, nothing written.
2. **Resolve the monitor prompt body** from `--monitor-file` (file, or stdin via `-`). Same rejection rules as `member create --file`; error strings name `--monitor-file` (see Error cases).
3. **Backend checks**, before any write: look up the `--coding-agent` backend, `validate_model(--monitor-model)`, `ensure_available` (PATH check for the backend binary). Same error strings as `member create`.
4. **Begin the transaction**: insert the fleet row; insert the Director member + placement; backfill `fleets.director_member_id`; insert the monitor member row with the monitor card marker (the same `kind=monitor` marker `member create --role monitor` writes). All ids (`fleet_id`, `director_member_id`, monitor `member_id`) are known inside the transaction.
5. **Substitute the four identity placeholders** into the prompt body (`{fleet_id}`, `{member_id}` = the monitor's own id, `{director_member_id}`, `{coding_agent}`). On error: roll back the transaction and re-raise the existing substitution error (exit 2, strings unchanged from `member create`).
6. **Spawn the pane**: build the spawn argv via the backend (`display_name="monitor"`, the `--monitor-model` value, no effort), forward `CAFLEET_DATABASE_URL` only, `split_window` detached. On failure: roll back the transaction (nothing persisted, no compensating deletes needed), exit 1.
7. **Insert the monitor placement row** with the returned pane id (same session/window context as the Director; `coding_agent` = the `--coding-agent` value), then **commit**. On a placement-insert or commit failure the transaction unwinds and the CLI error path kills the pane (pane-id plumbing below), exit 1.
8. **Emit** the result (JSON / text; shapes below).

Integration shape: `broker::create_fleet` grows into the atomic bootstrap — it additionally takes the monitor identity constants and a spawn callback. The broker owns the transaction and the row inserts; the CLI-supplied callback (invoked with the three allocated ids) performs substitution + `split_window` and returns the pane id. A callback error unwinds the transaction.

Pane-id plumbing for post-spawn failures: before returning the pane id to the broker, the callback also records it into a caller-held capture in `fleet.rs` (e.g. an `Option<String>` the closure writes). When `create_fleet` returns an error, the `fleet.rs` error path checks that capture and `send_exit`s the recorded pane — so the kill fires no matter where inside `create_fleet` the failure occurred (placement insert or commit), while a pre-spawn failure leaves the capture empty and skips the kill.

Timing note: the coding agent inside the pane boots asynchronously and issues its first DB read (the `ready` send) seconds after spawn, while the commit completes milliseconds after the split — so the spawned agent always observes committed rows. In the commit-failure corner the pane is killed within the same invocation before the agent can act. The flip side of the single transaction is the write-lock window: the connection holds SQLite's write lock across the `split_window` subprocess call, so any concurrent cafleet writer on the shared DB blocks for the duration of the tmux call, backstopped by the existing `busy_timeout=5000` connection PRAGMA — an accepted trade-off of the literally-atomic bootstrap.

### What does NOT change

| Surface | Status |
|---|---|
| Database schema | Unchanged — no migration. The monitor member reuses the existing member/placement rows and card marker. |
| `member create --role monitor` | Kept verbatim as the mid-run recovery path (dead-monitor re-spawn). One-per-fleet guard and its error string unchanged. |
| Monitor-first guard in `member create` | Kept verbatim (suspenders). After an atomic `fleet create` it is trivially satisfied; it fires only after a monitor death with no re-spawn. |
| Monitor member protocol (`roles/monitor.md` behavior) | Unchanged: on spawn it sends `ready`, launches `cafleet monitor <fleet-id>` as a background task in its own pane, confirms the startup line, sends `monitor live`. The `monitor live` gate before the first ordinary `member create` stays (belt). |
| Schema-version + stale-assets guards | Already wrap the whole `fleet` command group; no new wiring. |
| `fleet delete` / teardown ordering | Unchanged (monitor deleted first-out at teardown). |

### Output contract

JSON: the `fleet create` result gains a `monitor` object after `director`, with the identical member shape (key order pinned, updated in SPEC §6.4 and the pinned shape test):

```json
{
  "fleet_id": 1,
  "name": "alpha",
  "created_at": "<ts>",
  "director": {
    "member_id": 2, "name": "Director", "description": "Root Director for this fleet",
    "registered_at": "<ts>",
    "placement": { "backend": "tmux", "mux_session": "main", "mux_window_id": "@1",
                   "mux_pane_id": "%0", "coding_agent": "claude", "created_at": "<ts>" }
  },
  "monitor": {
    "member_id": 3, "name": "monitor", "description": "Monitor member for this fleet",
    "registered_at": "<ts>",
    "placement": { "backend": "tmux", "mux_session": "main", "mux_window_id": "@1",
                   "mux_pane_id": "%1", "coding_agent": "claude", "created_at": "<ts>" }
  }
}
```

Text form (SPEC §6.4): `<fleet_id> director=<id> monitor=<id>`.

### Error cases

| Condition | Behavior | Exit |
|---|---|---|
| Outside a tmux/herdr session | Existing `Error: cafleet fleet create must be run inside a tmux or herdr session`; nothing written | 1 |
| `--monitor-file` omitted | clap's native missing-required-argument error naming `--monitor-file` | 2 |
| Monitor file unreadable / empty / non-UTF-8 (or the stdin variants via `-`) | The `member create --file` rejection strings with the flag label `--monitor-file` — `resolve_body` is generalized to take the flag label so the message names the flag the user typed | mirrors `member create` |
| Invalid `--monitor-model` for the backend | Identical string and exit code to `member create --model` validation; nothing written | mirrors `member create` |
| Backend binary not on PATH | Identical string to `member create`'s availability check; nothing written | 1 |
| Unknown placeholder / malformed braces in the prompt | The two existing substitution error strings, verbatim; transaction rolled back — no fleet, no Director, no monitor | 2 |
| `split_window` failure | `Error: tmux split-window failed: <detail>. Rolled back fleet creation.`; transaction rolled back | 1 |
| Placement-insert or commit failure after a successful split | Transaction unwound; pane killed by the `fleet.rs` error path via the caller-held pane-id capture; no rows persisted | 1 |

The `--monitor-file` row above uses the parser's-native convention. The pre-existing `fleet create` missing-flag rows in cli-options.md's Error Messages table (`--name`, `--coding-agent`) are click-style strings the Rust binary never emits; the Step 1 cli-options.md task restates them in the same parser's-native convention while adding the `--monitor-file` row, so the table carries one style.

### Director startup protocol (instruction-surface change)

| Phase | Before | After |
|---|---|---|
| Env check | `cafleet doctor` | `cafleet doctor` (unchanged) |
| Monitor prompt | Write `${BASE}/.prompts/monitor-<UTC-compact>.md` | Same write, same audit convention — now consumed by `fleet create` |
| Fleet bootstrap | `cafleet fleet create --name … --coding-agent … --json` | `cafleet fleet create --name … --coding-agent … --monitor-file <abs path> --monitor-model {monitor_model} --json` |
| Monitor spawn | `cafleet member create --fleet-id … --role monitor --model {monitor_model} --file …` | Absorbed — command removed from the startup sequence |
| Gate | Wait for `ready` then `monitor live` | Unchanged |
| Team spawn | Ordinary `member create` per member | Unchanged |

`{monitor_model}` stays a skill-side overlay value mirroring the model list's *Monitor and reviewer defaults* table; the binary carries no model default. The pre-spawn prompt file remains the canonical audit artifact with the four identity placeholders pre-substitution. When `${BASE}` resolved to `<unset>`, the guarded-skip fallback for `fleet create` is `--monitor-file -` with the prompt on stdin (the flag has no inline positional form).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first (project rule), then code, then tests.

### Step 1: Documentation

- [x] Update `docs/docs/spec/cli-options.md`: the `fleet create` flag table (+ `--monitor-file`, `--monitor-model`), its section prose (atomic fleet + Director + monitor), the new/changed error-message rows — restating the pre-existing click-style `--name` / `--coding-agent` missing-flag rows in the parser's-native convention alongside the new `--monitor-file` row — the output-shapes section, and the monitor-first guard row's framing (bootstrap-satisfied; recovery-path trigger) <!-- completed: 2026-08-17T09:56 -->
- [x] Update `docs/docs/spec/data-model.md`: the bootstrap transaction description now includes the monitor member + placement, plus the write-lock window of holding the transaction across the pane spawn (backstopped by `busy_timeout=5000`) <!-- completed: 2026-08-17T09:58 -->
- [x] Update `SPEC.md`: §6.3 `fleet create` contract (flags, ladder, error strings), §6.4 output shapes (JSON key order + text form), the reimplementation checklist line, and the monitor-first framing <!-- completed: 2026-08-17T10:04 -->
- [x] Update `docs/docs/concepts/monitoring.md`, `docs/docs/concepts/overview.md`, and `docs/docs/how-to/mixed-backend-team.md`: bootstrap narrative (fleet create spawns the monitor; `member create --role monitor` is the recovery path) <!-- completed: 2026-08-17T10:08 -->
- [x] Update `skills/cafleet`: `SKILL.md` § Team supervision, `reference/supervision.md` (§ Spawn Protocol, § Monitor Lifecycle, § Quick Reference rows), `reference/director.md` (`--role` row → recovery-only framing), `reference/cli.md` (`fleet create` synopsis), `roles/monitor.md` (spawned by `fleet create --monitor-file`; recovery via `member create --role monitor`), `roles/director.md` where it names the old sequence <!-- completed: 2026-08-17T10:16 -->
- [x] Update consuming skills that restate the monitor-first spawn sequence: `skills/cafleet-design-doc/SKILL.md` + `create/create.md` + `interview/interview.md` + `execute/execute.md`; `skills/cafleet-research/SKILL.md` + `report/report.md` + `presentation/presentation.md` <!-- completed: 2026-08-17T10:24 -->
- [x] Update project-local skills `.claude/skills/skill-author/SKILL.md` and `.claude/skills/clean-docs/SKILL.md` (they teach/restate the monitor-first spawn) <!-- completed: 2026-08-17T10:14 -->

### Step 2: Code

- [x] Extend `broker::create_fleet` (`cafleet/src/broker/fleets.rs`) into the atomic bootstrap: monitor member + card marker + placement inside the single transaction, with a spawn callback invoked between monitor registration and placement insert; callback error unwinds the transaction <!-- completed: 2026-08-17T10:44 -->
- [x] Extend `cafleet/src/cli/fleet.rs`: `--monitor-file` / `--monitor-model` flags, prompt resolution, backend model validation + availability check, the substitution + `split_window` callback, the commit-failure pane kill, and the new error strings <!-- completed: 2026-08-17T10:44 -->
- [x] Generalize `resolve_body` (`cafleet/src/cli/helpers.rs`) to take the flag label so `fleet create` errors name `--monitor-file` while `member create` / `message send` keep their current strings <!-- completed: 2026-08-17T10:44 -->
- [x] Extend `format_fleet_create` (`cafleet/src/output/formatters.rs`) to `<fleet_id> director=<id> monitor=<id>` <!-- completed: 2026-08-17T10:44 -->

### Step 3: Tests and verification

- [x] Broker tests (`fleets.rs` colocated + `test_support`): bootstrap registers the monitor member with card marker and pane placement; the pinned JSON-shape and text-form tests updated; rollback tests for callback failure and substitution failure leave zero rows; update the shared `test_support::create_fleet` helper for the new signature and audit its call sites <!-- completed: 2026-08-17T10:41 -->
- [x] CLI parse tests (`cli/fleet.rs`): `--monitor-file` required, `--monitor-model` optional, `-` accepted as the file value <!-- completed: 2026-08-17T10:41 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format` <!-- completed: 2026-08-17T10:49 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-17 | Initial draft |
| 2026-08-17 | Review round 1: specified the caller-held pane-id capture for post-spawn failure kills; recorded the transaction write-lock window trade-off; directed the cli-options.md edit to unify the missing-flag error-row style |
