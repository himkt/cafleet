# Pending-Placement Ping Tolerance and Monitor Simplification

**Status**: Aborted
**Progress**: 0/37 tasks complete
**Last Updated**: 2026-07-29

## Overview

`cafleet member ping` against a member whose placement is still pending exits 1, and the durable stall-episode machine amplifies that benign spawn-time race into a fleet-halting `ping_failed` escalation (GitHub issue #232). This design makes the ping a documented no-op success on pending placement and, per the user's direction, radically simplifies monitoring: the durable stall-episode machine, its five CLI endpoints, and its two delivery/gate tables are deleted, shrinking `cafleet monitor` to exactly `start` + `capture` (the latter moved from `member capture`). Stall handling becomes the monitoring member's own judgment: capture, classify, confirm across two wakes, at most one fixed ping, and report to the Director by plain `cafleet message send`.

## Success Criteria

- [ ] `cafleet member ping` against a pending-placement member exits 0 in all three output modes (text, `--json`, `--quiet`) with the skip contract below; `member prompt` keeps its pending-placement hard error; `message send` is unchanged.
- [ ] `cafleet monitor --help` lists exactly two subcommands: `start` and `capture`; `cafleet member --help` no longer lists `capture`; the removed subcommands (`monitor status`, `monitor config`, `monitor stall observe|ping-result|pending`, `monitor report-batch`, `member capture`) fail with Click's default no-such-command error.
- [ ] Migration `0006` drops the four episode columns from `monitor_config` and the `monitor_director_gate` / `monitor_report_delivery` tables; the chain-guard tests in `tests/db/test_alembic_smoke.py` assert the six-revision chain and head `0006`.
- [ ] The broker exposes no stall-episode API: `observe_stall_episode`, `record_stall_ping_result`, `list_pending_stall_escalations`, and the report-batch/gate path are gone.
- [ ] A repo-wide sweep for the removed vocabulary (`monitor stall`, `monitor status`, `monitor config`, `report-batch`, `ping_failed`, `ping_interrupted`, `unchanged_after_nudge`, `nudge_claimed`, `escalation_pending`, `member capture`, the deleted broker function names, `format_monitor_status`, `format_monitor_config`) returns zero hits outside `design-docs/` and git history.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

`cafleet member create` registers the member and its placement row first and binds the tmux pane afterwards, so a freshly spawned member has a window where `member_placements.mux_pane_id` is NULL ("pending placement"). Three amplifiers turn that benign window into a fleet halt:

1. **CLI**: `member ping` routes through `_require_member_pane` and exits 1 — `Error: member <id> has no pane yet (pending placement) — nothing to ping.` A Director agent reads the non-zero exit as a fleet problem and stops everything (the observed failure in issue #232).
2. **Broker**: the stall-claim path (`_stall_target` with `require_live=True`) raises `member <id> does not have a live placement` for a pending target, and the episode machine converts ping failures into sticky `escalation_pending/ping_failed` rows.
3. **Protocol weight**: the durable episode machine (5 states, 3 escalation reasons, 2 delivery/gate tables, a ~40-sentence wake-prompt protocol, 7 monitor CLI endpoints) exists to make an LLM's stall handling transactional — and each of its failure paths is a new way for a benign race to become a durable escalation.

The user confirmed the pending-skip fix, then steered twice toward simplification ("now monitoring is getting unnecessarily complicated. It should be simple as much as possible"; "cafleet monitor does not have to have so much endpoints, just having 'start' would be enough???") and asked for `member capture` to move into the `monitor` group. The decisions below were confirmed through the Director relay.

### Confirmed decisions

| # | Decision | Resolution |
|---|---|---|
| 1 | Simplification depth | Option A — delete the durable stall-episode machine entirely |
| 2 | Premature-ping regression | Accepted, mitigated by the two-wake confirmation instruction |
| 3 | `monitor status` | Deleted — handshake via the loop's stdout; WebUI keeps the liveness view |
| 4 | `monitor config` | Deleted — defaults + `CAFLEET_MONITOR_STALL_INTERVAL` + WebUI remain |
| 5 | Ping skip JSON | Stable `skipped` key on both success paths |
| 6 | Director anomalies | Stay loud failures; the pending skip applies to ordinary-member pings |
| 7 | `member capture` | Hard-break rename to `monitor capture`, total mention cleanup |

---

## Specification

### 1. `member ping` — pending placement is a no-op success

`member ping` stops calling `_require_member_pane`. After resolving the member (fleet-scoped, placement row required as today), a NULL `mux_pane_id` takes the **skip path**: no keystroke is sent, and the command succeeds. Rationale: the pending member's inbox is intact and it polls it on spawn, so there is nothing a ping would add.

| Mode | Normal success (unchanged except JSON key) | Pending-placement skip (new) |
|---|---|---|
| text | `Pinged member <name> (<pane_id>) — poll keystroke dispatched.` | `Member <name> has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` |
| `--json` | `{"member_id": <id>, "pane_id": "<pane_id>", "skipped": false}` | `{"member_id": <id>, "pane_id": null, "skipped": true}` |
| `--quiet` | bare `<member_id>` | bare `<member_id>` |

Exit code 0 on both paths in every mode. The `skipped` key is present on **both** paths (stable schema).

Unchanged failure modes (all exit 1 unless noted):

| Case | Error | Notes |
|---|---|---|
| Member not found / cross-fleet | `Member <id> not found` | unchanged |
| No placement row | ``member <id> has no placement row; it was not spawned via `cafleet member create`.`` | unchanged — a placementless row was never spawned; loud failure per `affirmative-writing.md` |
| tmux delivery failure | `send failed: …` | unchanged |
| `member prompt` on pending placement | `member <id> has no pane yet (pending placement) — nothing to prompt.` | unchanged — prompt's contract requires the pane |
| `monitor capture` on pending placement | `member <id> has no pane yet (pending placement) — nothing to capture.` | unchanged string; command respelled per §4 |

`_require_member_pane`'s action set narrows to `capture`/`prompt`. `message send` is placement-free and unchanged.

### 2. Delete the durable stall-episode machine

Everything in the following inventory is removed. Per `removal.md`, every mention across code, docs, skills, and tests goes in the same change; this design doc is the historical record.

| Layer | Removed |
|---|---|
| CLI | `monitor stall observe`, `monitor stall ping-result`, `monitor stall pending`, `monitor report-batch`, `monitor status`, `monitor config` |
| Broker | `observe_stall_episode`, `record_stall_ping_result`, `list_pending_stall_escalations`, the report-batch + Director-gate path (`_issue_director_gate`, `_validate_director_gate_token`, report/preview delivery), `_observe_ordinary_candidate`, `_observe_director_candidate`, `_stall_target`, `_apply_nonlive_episode_cleanup`, `_clear_stall_episode`, `_parse_capture_identity` (`update_monitor_config` stays — the WebUI PATCH endpoint keeps it, §6) |
| Episode vocabulary | states `nudge_claimed` / `nudged` / `escalation_pending` / `escalated`; reasons `ping_failed` / `ping_interrupted` / `unchanged_after_nudge` |
| `monitor_config` columns | `stall_episode_state`, `stall_escalation_reason`, `last_stall_candidate_at`, `last_stall_capture_sha256`, and every CHECK constraint referencing them |
| Tables | `monitor_director_gate`, `monitor_report_delivery` |
| Output | `format_monitor_status` and `format_monitor_config` in `output/formatters.py` (plus their `output/__init__.py` exports) and the private `_format_ping_age` helper used only by `format_monitor_status` — the WebUI renders from `monitor_runtime_payload` / `monitor_members_payload` directly and does not use them |
| Wake prompt | the observe/claim/ping-result/report-batch protocol in `multiplexer/base.py` |

The scan-to-claim race of issue #232 ceases to exist structurally: with no claim, there is no `ping_failed`, no `ping_interrupted`, and no sticky `escalation_pending`.

### 3. Replacement: the monitoring member's own judgment

The heartbeat loop is untouched in its role: it scans the watched set, unions the `interval` / `stall-check` / `status:done` / `unacked` wake reasons exactly as today, and keystrokes one wake into the monitoring member's pane. What changes is what the monitoring member does on wake — LLM judgment with in-context memory replaces broker transactions:

1. **Capture** every named pane and the Director with `cafleet monitor capture --fleet-id <f> --member-id <m> --lines 120 --no-ansi --json` (the JSON carries `captured_at` and `content_sha256`).
2. **Classify** each capture on the existing five-state rubric (`awaiting_user`, `unknown`, `finished`, `working`, `stall_candidate`) using the target's backend overlay cues; any affirmative or ambiguous active-work cue is `working`.
3. **Confirm before acting**: a `stall_candidate` is confirmed stalled only when its `content_sha256` is byte-identical to the sha the monitoring member recorded for that member on the **previous** stall-check wake (its own conversation notes are the record). A first candidate only seeds the baseline. After a monitoring-member restart the notes are gone; the first post-restart wake re-seeds and never pings.
4. **At most one fixed ping** per confirmed stall: `cafleet member ping` (now no-op-safe on pending placement). Never ping the Director or itself; never `member prompt`, never `message broadcast`, no other pane action.
5. **Report by plain messaging**: when a wake produced anything the Director must know (a confirmed stall, a ping delivery failure, a `finished` member), send exactly **one** `cafleet message send` to the Director summarizing all of it; otherwise send nothing. `finished` stays report-only — the Director alone judges whether assigned work remains.

The `ready: monitor live` handshake replaces `monitor status`: `run_monitor_loop` emits a startup line to stdout immediately after claiming the runtime row — `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` — and the monitoring member, having launched `cafleet monitor start` as a background task, confirms that line in the task output before sending `ready: monitor live`. A task that exits instead (runtime-claim conflict, dead fleet) is reported to the Director as a failed start.

Accepted regressions (user-confirmed):

| Regression | Bound | Mitigation |
|---|---|---|
| Premature ping — the two-identical-captures rule is instruction, not broker-enforced | One Esc + poll keystroke into a working pane; may cancel that member's in-flight turn | The two-wake confirmation instruction in the wake prompt; ping payload stays fixed and benign |
| Escalations do not survive a monitoring-member restart | Latency — a persisting stall is re-detected on a later wake | Baseline re-seeding on the first post-restart wake |
| At-most-once ping is not transactional | A restart mid-wake may re-ping once (same bound as the first row) | Same fixed payload bound |

### 4. `cafleet monitor` = `start` + `capture`; `member capture` moves

| Endpoint (before) | After | Replacement |
|---|---|---|
| `monitor start` | kept | unchanged flags (`--fleet-id`, `--tick`); adds the startup line of §3 |
| `member capture` | moved → `monitor capture` | identical flags (`--fleet-id`, `--member-id`, `--lines` default 20, `--ansi/--no-ansi`, `--json`), identical output and error contract, including the pending-placement hard error — the §3 wake protocol passes `--lines 120` explicitly |
| `monitor status` | deleted | loop stdout backs the handshake; the WebUI (`GET /api/monitor`) keeps the liveness + schedule view |
| `monitor config` | deleted | enrollment defaults (`DIRECTOR_PING_INTERVAL_SECONDS = 180`, `MEMBER_PING_INTERVAL_SECONDS = 720`) + `CAFLEET_MONITOR_STALL_INTERVAL`; per-member editing stays in the WebUI (`PATCH /api/members/{id}/monitor`) |
| `monitor stall observe` / `ping-result` / `pending` | deleted | §3 judgment protocol |
| `monitor report-batch` | deleted | one plain `cafleet message send` per wake — the same Esc-safeguarded preview path every fleet message already uses |

The rename is a hard break: no alias, no deprecation note; every mention in docs, skills, rules, and permission-pattern documentation is respelled in the same change. `member prompt` and `member ping` stay under `member` — they are Director write/drive primitives and their permission tiers live there. The monitor group becomes exactly the monitoring toolkit: the loop and its read primitive.

### 5. Data model — migration `0006`

Generated via `mise //cafleet:makemigration "drop monitor stall episode state"` (DB at head first via the schema-only `cafleet setup` invocation), then hand-reviewed per `database-migrations.md`:

- `monitor_config`: drop `stall_episode_state`, `stall_escalation_reason`, `last_stall_candidate_at`, `last_stall_capture_sha256` and all CHECK constraints referencing them. SQLite cannot drop table-level CHECKs in place, so this is a batch recreate of `monitor_config` — safe under FK enforcement because `monitor_config` is a child table (FK to `members`), not a parent. The recreate **preserves** `member_id`, `interval_seconds`, `enabled`, `last_ping_at`, `last_stall_check_at` for every row.
- Drop `monitor_director_gate` and `monitor_report_delivery` (their contents are runtime-ephemeral delivery/gate state; nothing durable is lost).
- `downgrade()`: re-add the four columns (`stall_episode_state` defaulting to `'clear'`, the rest NULL) with their constraints, and recreate the two tables empty — the pre-0006 schema, with episode state legitimately reset because it was runtime-ephemeral.
- Chain guard: in `tests/db/test_alembic_smoke.py`, rename `test_five_revision_migration_chain_exists` → `test_six_revision_migration_chain_exists` (chain `0006` → `0005` → … → `0001` → `None`) and `test_alembic_version_table_records_head_0005` → `…_0006`; delete the `monitor_report_delivery` / `monitor_director_gate` schema tests; update the `monitor_config` column assertions; keep the 0005 up/down round-trip test (it exercises the historical chain).

### 6. What stays

| Kept | Notes |
|---|---|
| The `scan → wake → sleep` loop, `should_ping`, wake-reason union, per-due-member stdout lines | unchanged |
| `monitor_runtime` single-instance claim + ownership-checked per-tick heartbeat | internal, not an endpoint |
| Enrollment at registration (`enroll_member`, the two interval constants) and `monitor_config` scheduling columns (`interval_seconds`, `enabled`, `last_ping_at`, `last_stall_check_at`) | the stall-check cadence survives restarts |
| The `stall-check` wake reason and `CAFLEET_MONITOR_STALL_INTERVAL` (`0` disables stall-check wakes and therefore pings) | the loop still tells the monitor *when*; the monitor decides *whether* |
| WebUI: `GET /api/monitor`, `GET`/`PATCH /api/members/{id}/monitor`, `broker.monitor_runtime_payload` / `monitor_members_payload` / `get_monitor_config` / `list_monitor_configs` / `update_monitor_config` | the config dict (`_config_dict` / `_CONFIG_COLS`) loses the four episode fields, so the WebUI `monitor` shape shrinks accordingly; the admin frontend has no stall-episode rendering to remove |
| `delete_fleet_monitor_rows` / `delete_member_monitor_row` | the fleet-level helper drops its two deleted-table statements |
| The five-state classification rubric and the backend-overlay capture cues | overlay wording changes only where it names broker resolution |
| The Director's facilitation layer, including the pre-nudge capture gate | the gate's read is respelled `cafleet monitor capture` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (docs-first — before any code)

- [ ] Rewrite `docs/concepts/monitoring.md`: keep the heartbeat/facilitation split and the watched-set/cadence/single-instance/lifecycle sections; replace the "Fixed stall recovery" layer and the on-wake numbered protocol with the §3 judgment protocol; replace the `monitor status` handshake with the startup-line handshake; remove episode machine, report-batch, gate, and `monitor config` mentions (intervals: defaults + WebUI) <!-- completed: -->
- [ ] Update `docs/concepts/member-lifecycle.md`: pending placement is ping-tolerant (skip semantics), one-line note in the lifecycle narrative <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md`: subcommand summary table; `member ping` section (skip contract of §1, `skipped` JSON key); delete the `member capture` section and add `monitor capture` under the monitor group; delete `monitor status` / `monitor config` / `monitor stall *` / `monitor report-batch` sections; update the error-message table (pending-placement row now names `capture`/`prompt` via `monitor capture` / `member prompt`); update `permissions.allow` coverage for the respelled capture <!-- completed: -->
- [ ] Update `docs/spec/data-model.md`: `monitor_config` column list, drop the two tables, remove episode-state vocabulary <!-- completed: -->
- [ ] Update `docs/spec/multiplexer-backends.md`: wake-prompt contract per §3 <!-- completed: -->
- [ ] Update `docs/api/*` pages that reference deleted broker functions or removed CLI endpoints <!-- completed: -->
- [ ] Update `SPEC.md`: §5.3 enums (episode states/reasons gone), §6.2 broker (stall machine removal, messaging-based reporting), §6.3 CLI (require-pane helper narrowed to capture/prompt; `member ping` skip contract; monitor group = `start` + `capture`), §6.5/§6.6 (wake prompt, startup line), §6.8 (WebUI monitor shape), §8 schema, §10 checklist <!-- completed: -->
- [ ] Update `skills/cafleet/SKILL.md`: Team supervision section and the fixed monitoring-member ping exception paragraph (two-wake self-confirmation, plain message-send reporting, no broker claim/aggregate) <!-- completed: -->
- [ ] Update `skills/cafleet/reference/supervision.md`: stall-response and reporting flow, `monitor capture` respelling, handshake wording <!-- completed: -->
- [ ] Update `skills/cafleet/reference/director.md`: pre-nudge capture gate → `monitor capture`; remove any stall-episode/aggregate retrieval instructions (Director now receives plain messages) <!-- completed: -->
- [ ] Update `skills/cafleet/reference/cli.md`: command catalog (monitor group, capture move, ping skip) <!-- completed: -->
- [ ] Rewrite `skills/cafleet/roles/monitor.md`: startup (launch task, confirm startup line, `ready: monitor live`), on-wake routine per §3, teardown unchanged <!-- completed: -->
- [ ] Update `skills/cafleet/reference/coding-agent/` overlays + `_template.md`: replace "only the broker may promote … to `stalled`" with the two-wake self-confirmation rule; respell capture commands <!-- completed: -->
- [ ] Update `.claude/rules/bash-tool.md`: the monitoring-member ping exception paragraph (no broker claim / `action = ping`) <!-- completed: -->
- [ ] Verify `README.md`'s thin surface (pitch, install, docs links) is unaffected; sync via `/update-readme` only if drift exists <!-- completed: -->

### Step 2: Schema

- [ ] Update `db/models.py`: remove the four `MonitorConfig` episode columns and their CHECK constraints; delete the `MonitorDirectorGate` and `MonitorReportDelivery` models <!-- completed: -->
- [ ] Generate migration 0006 via `mise //cafleet:makemigration "drop monitor stall episode state"`; hand-review to the §5 shape (batch recreate of `monitor_config` preserving the five kept columns; table drops; full `downgrade()`) <!-- completed: -->
- [ ] Update the chain-guard and schema tests in `tests/db/test_alembic_smoke.py` per §5 <!-- completed: -->

### Step 3: Broker

- [ ] Delete the stall-episode API and helpers listed in §2 from `broker/monitor.py` (and their `broker/__init__.py` exports); trim `_config_dict` / `_CONFIG_COLS` to the five kept fields <!-- completed: -->
- [ ] Simplify lifecycle reconciliation: disable/dead/pending cleanup now only clears `last_stall_check_at` (no episode transitions) <!-- completed: -->
- [ ] Trim `delete_fleet_monitor_rows` to `monitor_config` + `monitor_runtime` <!-- completed: -->

### Step 4: CLI

- [ ] `cli/member.py`: implement the §1 ping skip (drop `_require_member_pane` from `member ping`; add the `skipped` key to both JSON paths; skip text line); narrow `_require_member_pane` docs/action set to capture/prompt; delete the `member capture` command <!-- completed: -->
- [ ] `cli/monitor.py`: delete `status`, `config`, the `stall` group, and `report-batch`; add `monitor capture` (the moved implementation, contract unchanged) <!-- completed: -->
- [ ] Update any internal callers/help text referencing the removed or moved commands (e.g. `doctor`, `member create` spawn scaffolding, `fleet delete` messages) found by the Step 7 sweep <!-- completed: -->

### Step 5: Monitor loop and wake prompt

- [ ] `monitor/loop.py`: emit the startup line `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` after a successful runtime claim, before the first tick <!-- completed: -->
- [ ] `multiplexer/base.py`: rewrite the wake-prompt builder to the §3 protocol (capture via `monitor capture`, classify, two-wake confirmation from own notes, at most one fixed ping, one plain message-send report, no other pane action) <!-- completed: -->

### Step 6: WebUI

- [ ] `webui/api.py`: confirm `GET /api/monitor` and `GET`/`PATCH /api/members/{id}/monitor` still function on the trimmed config dict; update the `monitor` response shape and any typed models <!-- completed: -->
- [ ] Sweep `admin/src` for episode-field usage (none expected) and rebuild via `mise //admin:build` if the API types are generated <!-- completed: -->

### Step 7: Tests and verification

- [ ] Delete `tests/broker/test_monitor_stall_state.py`, `tests/cli/test_monitor_stall.py`, and — with the output formatters — `tests/output/test_monitor_status_table.py` and `tests/output/test_ping_age_ascii.py` <!-- completed: -->
- [ ] Rewrite `tests/multiplexer/test_monitor_wake_contract.py` against the new wake prompt <!-- completed: -->
- [ ] Update `tests/cli/test_monitor.py`: delete the `monitor status` / `monitor config` sections; add the §3 startup-line assertion to the `monitor start` tests <!-- completed: -->
- [ ] Update `tests/monitor/test_loop.py` (startup line; unchanged scan/wake behavior) and `tests/integration/test_direct_member_nudge.py` + `tests/docs/test_direct_member_nudge_docs.py` for the ping skip and respelled capture <!-- completed: -->
- [ ] Update `tests/cli/test_member_ping.py` (existing ping tests take the `skipped` JSON key), `tests/cli/test_member_capture_defaults.py` (respelled to `monitor capture`), and `tests/cli/test_help_budget.py` (the `("member", "capture")` budget entry moves to the monitor group) <!-- completed: -->
- [ ] Update `tests/webui/test_monitor_api.py` (trimmed `monitor` shape) and `tests/broker/test_monitor.py` (config-dict assertions lose the episode columns) <!-- completed: -->
- [ ] Add CLI tests: ping skip in text/`--json`/`--quiet` (exit 0, `skipped` key both paths), placementless ping still errors, `monitor capture` contract (including the pending-placement hard error), and absence guards for every removed command via Click's default no-such-command error <!-- completed: -->
- [ ] Repo-wide vocabulary sweep per Success Criteria (rg over source, docs, skills, rules, tests) <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-29 | Initial draft |
| 2026-07-29 | Review round: Output row added to the removal inventory; capture `--lines` default corrected to 20; sweep vocabulary and Step 7 test coverage extended. Approved. |
| 2026-07-29 | Compiled into design 0000154 (monitor-nudge-simplification) and aborted before implementation. |
