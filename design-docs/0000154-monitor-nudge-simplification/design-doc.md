# Monitor Ping Simplification

**Status**: Approved
**Progress**: 24/38 tasks complete
**Last Updated**: 2026-07-29

## Overview

Reduce monitoring to its minimal shape — `monitor start` + capture + ping, plus plain broker messages for escalation. This design compiles the approved-but-unimplemented design 0000153 (pending-placement ping tolerance; deletion of the durable stall-episode machine; `cafleet monitor` = `start` + `capture` with the `member capture` → `monitor capture` hard rename) into one authoritative superset and adds two changes on top: the wake payload becomes a pure trigger (due list + Director descriptor + one pointer sentence, deleting the ~2.4 KB inline protocol restatement), and an ordinary member idle at an empty composer (`finished`) becomes directly ping-eligible through the same two-wake in-context judgment as a `stall_candidate`.

## Success Criteria

- [ ] `cafleet member ping` against a pending-placement member exits 0 in all three output modes (text, `--json`, `--quiet`) with the skip contract below (stable `skipped` JSON key on both success paths); `member prompt` keeps its pending-placement hard error; `message send` is unchanged.
- [ ] `cafleet monitor --help` lists exactly two subcommands: `start` and `capture`; `cafleet member --help` no longer lists `capture`.
- [ ] The tmux/herdr wake payload is the pure-trigger form specified below, byte-identical across both backends, containing no protocol clauses; `skills/cafleet/roles/monitor.md` is the sole normative protocol carrier.
- [ ] An ordinary member whose capture classifies `finished` (e.g. a claude member at the empty at-rest composer) receives the same two-wake treatment as a `stall_candidate`: byte-identical captures across two consecutive stall-check wakes yield at most one fixed `cafleet member ping`; a member still unchanged at the next stall-check wake, or a failed ping, is reported to the Director by a plain `cafleet message send`.
- [ ] The broker exposes no stall-episode API (`observe_stall_episode`, `record_stall_ping_result`, `list_pending_stall_escalations`, the report-batch/gate path are gone) and no durable episode state exists anywhere; no durable ping/episode log is added.
- [ ] Migration `0006` drops the four episode columns from `monitor_config` and the `monitor_director_gate` / `monitor_report_delivery` tables, preserving the five kept columns, with a working `downgrade()`; the chain-guard tests in `tests/db/test_alembic_smoke.py` assert the six-revision chain and head `0006`.
- [ ] A repo-wide sweep for the removed vocabulary (`monitor stall`, `monitor status`, `monitor config`, `report-batch`, `ping_failed`, `ping_interrupted`, `unchanged_after_nudge`, `nudge_claimed`, `escalation_pending`, `stall_episode_state`, `director_gate`, `member capture`, the deleted broker function names, `format_monitor_status`, `format_monitor_config`, and — for the fixed-ping/wake-trigger senses — `nudge`) returns zero hits outside `design-docs/` and git history. Carve-outs for the `nudge` term: the Director's message-level stall-nudge concept (the cafleet-design-doc skill's coordination protocol and workflow role files) is a different concept and stays, as do the kept test module names `test_direct_member_nudge.py` / `test_direct_member_nudge_docs.py`.
- [ ] The five-state classification taxonomy, the scheduler loop's triggers/cadence/annotation-only `unacked`, `monitor_runtime`, and the fixed `member ping` action are unchanged; the WebUI `monitor` object simply loses the four episode fields.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //admin:lint` pass.

---

## Background

`cafleet member create` registers the member and its placement row first and binds the pane afterwards, so a freshly spawned member has a window where `member_placements.mux_pane_id` is NULL ("pending placement"). `member ping` against that window exits 1, and the durable stall-episode machine amplifies the benign race into a fleet-halting `ping_failed` escalation (GitHub issue #232). Three further problems compound it:

1. **The wake payload restates the protocol.** `_monitor_wake_payload` emits a ~2.4 KB instruction block on every wake, duplicating `skills/cafleet/roles/monitor.md`, which the monitoring member already loads as required reading. Design 0000151 §8 locked that duplication in; this design deliberately reverses it.
2. **The direct ping almost never fires for idle members.** The claude overlay classifies an idle pane at the empty composer as `finished`, which is report-only: `finished` IDs flow only through the token-gated aggregate, suppressed whenever the final Director capture is unsafe — so the most common stall mode, "member went idle without reporting", is never pinged directly.
3. **Protocol weight.** The durable episode machine (5 states, 3 escalation reasons, 2 delivery/gate tables, 7 monitor CLI endpoints) exists to make an LLM's stall handling transactional — and each of its failure paths is a new way for a benign race to become a durable escalation.

The guiding principle (user directive): **the monitoring mechanism is `monitor start` + capture + ping, plus plain messages for escalation.** Design 0000153 specified the deletion and was approved but never implemented; the user directed that it be compiled into this design and aborted. Its confirmed decisions carry over unchanged.

### Confirmed decisions

| # | Decision | Resolution |
|---|---|---|
| 1 | Simplification depth | Delete the durable stall-episode machine entirely — no broker episode state, no durable ping log |
| 2 | Premature-ping regression | Accepted, mitigated by the two-wake confirmation instruction |
| 3 | `monitor status` | Deleted — handshake via the loop's stdout; WebUI keeps the liveness view |
| 4 | `monitor config` | Deleted — defaults + `CAFLEET_MONITOR_STALL_INTERVAL` + WebUI remain |
| 5 | Ping skip JSON | Stable `skipped` key on both success paths |
| 6 | Director anomalies | Stay loud failures; the pending skip applies to ordinary-member pings |
| 7 | `member capture` | Hard-break rename to `monitor capture`, total mention cleanup |
| 8 | Wake payload | Pure trigger — due list + Director descriptor + one pointer sentence, byte-identical on tmux/herdr; `roles/monitor.md` carries the full on-wake protocol (reverses 0000151 §8) |
| 9 | Idle members | `finished` ordinary members join the two-wake in-context judgment: at most one ping per quiet period, then a plain message to the Director if still unchanged |
| 10 | Escalation | Plain `cafleet message send` to the Director — no gate token, no batching, no backpressure, no deferral |

---

## Specification

### 1. `member ping` — pending placement is a no-op success

`member ping` stops calling `_require_member_pane`. After resolving the member (fleet-scoped, placement row required as today), a NULL `mux_pane_id` takes the **skip path**: no keystroke is sent, and the command succeeds. Rationale: the pending member's inbox is intact and it polls it on spawn, so there is nothing a ping would add — which is also why delivery verification stays live-only and no durable ping log exists.

| Mode | Normal success (unchanged except JSON key) | Pending-placement skip (new) |
|---|---|---|
| text | `Pinged member <name> (<pane_id>) — poll keystroke dispatched.` | `Member <name> has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` |
| `--json` | `{"member_id": <id>, "pane_id": "<pane_id>", "skipped": false}` | `{"member_id": <id>, "pane_id": null, "skipped": true}` |
| `--quiet` | bare `<member_id>` | bare `<member_id>` |

Exit code 0 on both paths in every mode. The `skipped` key is present on **both** paths (stable schema).

Unchanged failure modes (all exit 1):

| Case | Error | Notes |
|---|---|---|
| Member not found / cross-fleet | `Member <id> not found` | unchanged |
| No placement row | ``member <id> has no placement row; it was not spawned via `cafleet member create`.`` | unchanged — loud failure per `affirmative-writing.md` |
| tmux delivery failure | `send failed: …` | unchanged |
| `member prompt` on pending placement | `member <id> has no pane yet (pending placement) — nothing to prompt.` | unchanged — prompt's contract requires the pane |
| `monitor capture` on pending placement | `member <id> has no pane yet (pending placement) — nothing to capture.` | unchanged string; command respelled per §5 |

`_require_member_pane`'s action set narrows to `capture`/`prompt`. `message send` is placement-free and unchanged.

### 2. Delete the durable stall-episode machine

Everything in the following inventory is removed. Per `removal.md`, every mention across code, docs, skills, and tests goes in the same change; this design doc and 0000153 are the historical record.

| Layer | Removed |
|---|---|
| CLI | `monitor stall observe`, `monitor stall ping-result`, `monitor stall pending`, `monitor report-batch`, `monitor status`, `monitor config` |
| Broker | `observe_stall_episode`, `record_stall_ping_result`, `list_pending_stall_escalations`, the report-batch + Director-gate path (`_issue_director_gate`, `_validate_director_gate_token`, report/preview delivery), `_observe_ordinary_candidate`, `_observe_director_candidate`, `_stall_target`, `_apply_nonlive_episode_cleanup`, `_clear_stall_episode`, `_parse_capture_identity` (`update_monitor_config` stays — the WebUI PATCH endpoint keeps it, §7) |
| Episode vocabulary | states `nudge_claimed` / `nudged` / `escalation_pending` / `escalated`; reasons `ping_failed` / `ping_interrupted` / `unchanged_after_nudge` |
| `monitor_config` columns | `stall_episode_state`, `stall_escalation_reason`, `last_stall_candidate_at`, `last_stall_capture_sha256`, and every CHECK constraint referencing them |
| Tables | `monitor_director_gate`, `monitor_report_delivery` |
| Output | `format_monitor_status` and `format_monitor_config` in `output/formatters.py` (plus their `output/__init__.py` exports) and the private `_format_ping_age` helper — the WebUI renders from `monitor_runtime_payload` / `monitor_members_payload` directly |
| Wake payload | the entire observe/claim/ping-result/report-batch protocol body in `multiplexer/base.py` (replaced by the §3 pure trigger) |

The scan-to-claim race of issue #232 ceases to exist structurally: with no claim, there is no `ping_failed`, no `ping_interrupted`, and no sticky `escalation_pending`.

### 3. Pure-trigger wake payload

`_monitor_wake_payload` keeps its signature (due members + Director descriptor), entry rendering, name/agent sanitization (`_sanitize_wake_name`), and fail-closed `coding_agent` validation — and loses the protocol body:

```python
def _monitor_wake_payload(due_members: list[dict], director: dict) -> str:
    # per-entry and Director coding_agent validation unchanged
    noun = "member" if len(due_members) == 1 else "members"
    due_list = ", ".join(
        f"{'director' if target['is_director'] else 'member'} "
        f"{target['member_id']} "
        f"({_sanitize_wake_name(target['name'])}; "
        f"coding_agent={target['coding_agent']}) "
        f"[{','.join(target['wake_reasons'])}]"
        for target in due_members
    )
    return (
        f"[monitor] wake: {len(due_members)} {noun} due — {due_list}. "
        f"Director: {director['member_id']} "
        f"(coding_agent={director['coding_agent']}). "
        "Follow your monitor role protocol."
    )
```

Example rendered payload:

```text
[monitor] wake: 2 members due — director 332 (Director; coding_agent=codex) [interval], member 336 (alice; coding_agent=claude) [interval,stall-check]. Director: 332 (coding_agent=codex). Follow your monitor role protocol.
```

The payload names *who* is due and *who* the Director is — nothing else; the Director descriptor identifies the recipient of the monitoring member's §4 messages. The pointer sentence names the role, not a repo file path. `skills/cafleet/roles/monitor.md` is the sole normative carrier of the on-wake protocol; the exact-payload tests pin this short form byte-identically on tmux and herdr.

### 4. Replacement: the monitoring member's own judgment

The heartbeat loop is untouched in its role: it scans the watched set, unions the `interval` / `stall-check` / `status:done` / `unacked` wake reasons exactly as today, and keystrokes one wake into the monitoring member's pane. What changes is what the monitoring member does on wake — LLM judgment with in-context memory replaces broker transactions. The protocol, normative in `skills/cafleet/roles/monitor.md`:

1. **Capture** every named due **ordinary** pane with `cafleet monitor capture --fleet-id <f> --member-id <m> --lines 120 --no-ansi --json` (the JSON carries `captured_at` and `content_sha256`). A due `director` entry is not captured: the monitoring member takes no Director-directed action beyond the step-5 messages, and the payload's Director descriptor exists to identify their recipient.
2. **Classify** each capture on the existing five-state rubric (`awaiting_user`, `unknown`, `finished`, `working`, `stall_candidate`) using the target's backend overlay cues, selected from the wake entry's `coding_agent`. Any affirmative or ambiguous active-work cue is `working`; ambiguity between `awaiting_user` and `finished` resolves to `awaiting_user`. A failed or unreadable capture (dead pane, garbled output, or the pending-placement capture error of §1) is `unknown`: it never seeds, advances, or confirms the quiet baseline and is never pinged; the monitoring member clears its recorded baseline for that member and sends a step-5 message about the capture failure (once — repeated `unknown` on later wakes is not re-messaged).
3. **Confirm quiet members across two stall-check wakes.** `stall_candidate` and `finished` are both **quiet** observations for an ordinary member. Only a capture taken on a wake whose entry carries the `stall-check` reason may seed, advance, or confirm the quiet baseline; a capture from an `interval`- / `status:done`- / `unacked`-only entry is context and leaves the notes unchanged. A quiet member is confirmed only when its `content_sha256` is byte-identical to the sha the monitoring member recorded for that member on the **previous** stall-check wake (its own conversation notes are the record). A first quiet capture only seeds the baseline. After a monitoring-member restart the notes are gone; the first post-restart wake re-seeds and never pings.
4. **At most one fixed ping** per confirmed quiet member: `cafleet member ping` (no-op-safe on pending placement per §1). One ping per quiet period — a pane that changed only by reacting to the ping (poll output, an empty-inbox poll turn) is the same quiet period, not a new one. Observed `working` or `awaiting_user`, or materially changed quiet content (real work happened between wakes), ends the quiet period: the baseline re-seeds per step 3 and the member is re-armed for a future ping and message. Never ping the Director or itself; never `member prompt`, never `message broadcast`, no other pane action.
5. **Message the Director per event**: when an event needs Director attention — a member still unchanged at the next stall-check wake after its ping (stalled or idle; the Director alone judges whether assigned work remains), a ping delivery failure, or a capture failure per step 2 — the monitoring member simply sends a plain ordinary `cafleet message send` to the Director about it (the member named from its already-sanitized wake entry); with no such event, it sends nothing. There is no per-wake aggregation rule, no summary framing, and no one-message-per-wake requirement. Each send fires immediately regardless of the Director's pane state — the inline preview's existing `Esc` safeguard makes it safe on any pane, and it doubles as the Director's facilitation cue. Say each member's situation once per quiet period, not on every subsequent wake.

The `ready: monitor live` handshake replaces `monitor status`: `run_monitor_loop` emits a startup line to stdout immediately after claiming the runtime row — `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` — and the monitoring member, having launched `cafleet monitor start` as a background task, confirms that line in the task output before sending `ready: monitor live`. A task that exits instead (runtime-claim conflict, dead fleet) is reported to the Director as a failed start.

Accepted regressions (user-confirmed):

| Regression | Bound | Mitigation |
|---|---|---|
| Premature ping — the two-identical-captures rule is instruction, not broker-enforced | One Esc + poll keystroke into a working pane; may cancel that member's in-flight turn | The two-wake confirmation instruction; ping payload stays fixed and benign |
| Escalations do not survive a monitoring-member restart | Latency — a persisting stall is re-detected on a later wake | Baseline re-seeding on the first post-restart wake |
| At-most-once ping is not transactional | A restart mid-wake may re-ping once (same bound as the first row) | Same fixed payload bound |

### 5. `cafleet monitor` = `start` + `capture`; `member capture` moves

| Endpoint (before) | After | Replacement |
|---|---|---|
| `monitor start` | kept | unchanged flags (`--fleet-id`, `--tick`); adds the startup line of §4 |
| `member capture` | moved → `monitor capture` | identical flags (`--fleet-id`, `--member-id`, `--lines` default 20, `--ansi/--no-ansi`, `--json`), identical output and error contract, including the pending-placement hard error — the §4 wake protocol passes `--lines 120` explicitly |
| `monitor status` | deleted | loop stdout backs the handshake; the WebUI (`GET /api/monitor`) keeps the liveness + schedule view |
| `monitor config` | deleted | enrollment defaults (`DIRECTOR_PING_INTERVAL_SECONDS = 180`, `MEMBER_PING_INTERVAL_SECONDS = 720`) + `CAFLEET_MONITOR_STALL_INTERVAL`; per-member editing stays in the WebUI (`PATCH /api/members/{id}/monitor`) |
| `monitor stall observe` / `ping-result` / `pending` | deleted | §4 judgment protocol |
| `monitor report-batch` | deleted | a plain per-event `cafleet message send` — the same Esc-safeguarded preview path every fleet message already uses |

The rename is a hard break: no alias, no deprecation note; every mention in docs, skills, rules, and permission-pattern documentation is respelled in the same change. `member prompt` and `member ping` stay under `member` — they are Director write/drive primitives and their permission tiers live there. The monitor group becomes exactly the monitoring toolkit: the loop and its read primitive.

### 6. Data model — migration `0006`

Generated via `mise //cafleet:makemigration "drop monitor stall episode state"` (DB at head first via the schema-only `cafleet setup` invocation), then hand-reviewed per `database-migrations.md`:

- `monitor_config`: drop `stall_episode_state`, `stall_escalation_reason`, `last_stall_candidate_at`, `last_stall_capture_sha256` and all CHECK constraints referencing them. SQLite cannot drop table-level CHECKs in place, so this is a batch recreate of `monitor_config` — safe under FK enforcement because `monitor_config` is a child table (FK to `members`), not a parent. The recreate **preserves** `member_id`, `interval_seconds`, `enabled`, `last_ping_at`, `last_stall_check_at` for every row.
- Drop `monitor_director_gate` and `monitor_report_delivery` (their contents are runtime-ephemeral delivery/gate state; nothing durable is lost).
- `downgrade()`: re-add the four columns (`stall_episode_state` defaulting to `'clear'`, the rest NULL) with their constraints, and recreate the two tables empty — the pre-0006 schema, with episode state legitimately reset because it was runtime-ephemeral.
- Chain guard: in `tests/db/test_alembic_smoke.py`, rename `test_five_revision_migration_chain_exists` → `test_six_revision_migration_chain_exists` (chain `0006` → `0005` → … → `0001` → `None`) and `test_alembic_version_table_records_head_0005` → `…_0006`; delete the `monitor_report_delivery` / `monitor_director_gate` schema tests; update the `monitor_config` column assertions; keep the 0005 up/down round-trip test (it exercises the historical chain).

### 7. What stays

| Kept | Notes |
|---|---|
| The `scan → wake → sleep` loop, `should_ping`, wake-reason union, per-due-member stdout lines, annotation-only `unacked` | unchanged |
| `monitor_runtime` single-instance claim + ownership-checked per-tick heartbeat | internal, not an endpoint |
| Enrollment at registration (`enroll_member`, the two interval constants) and `monitor_config` scheduling columns (`interval_seconds`, `enabled`, `last_ping_at`, `last_stall_check_at`) | the stall-check cadence survives restarts |
| The `stall-check` wake reason and `CAFLEET_MONITOR_STALL_INTERVAL` (`0` disables stall-check wakes and therefore pings) | the loop still tells the monitor *when*; the monitor decides *whether* |
| Capture JSON identity (`captured_at`, `content_sha256`, key order, both ANSI modes) | consumed by the monitor's own notes instead of the broker |
| WebUI: `GET /api/monitor`, `GET`/`PATCH /api/members/{id}/monitor`, `broker.monitor_runtime_payload` / `monitor_members_payload` / `get_monitor_config` / `list_monitor_configs` / `update_monitor_config` | the config dict (`_config_dict` / `_CONFIG_COLS`) loses the four episode fields, so the WebUI `monitor` shape shrinks accordingly; the admin frontend has no stall-episode rendering to remove |
| `delete_fleet_monitor_rows` / `delete_member_monitor_row` | the fleet-level helper drops its two deleted-table statements |
| The five-state classification rubric and the backend-overlay capture cues | overlay wording changes only where it names broker resolution |
| The Director's facilitation layer, including the capture gate before re-engagement keystrokes | the gate is respelled "pre-ping capture gate" and its read `cafleet monitor capture` |
| `cafleet member ping`'s fixed Esc + poll action | unchanged apart from the §1 skip path; the never-ping-the-Director rule is instruction-level in `roles/monitor.md` — no CLI target guard exists or is added |
| README thin surface | unchanged |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first ordering per `.claude/rules/documentation-maintenance.md`.

### Step 1: Documentation (docs-first — before any code)

- [x] Rewrite `docs/concepts/monitoring.md`: keep the heartbeat/facilitation split and the watched-set/cadence/single-instance/lifecycle sections; replace the "Fixed stall recovery" layer and the on-wake numbered protocol with the §4 judgment protocol (quiet = `stall_candidate` + `finished`, two-wake confirmation, at most one ping, plain per-event Director messages); document the §3 pure-trigger wake; replace the `monitor status` handshake with the startup-line handshake; remove episode-machine, report-batch, gate, and `monitor config` mentions <!-- completed: 2026-07-30T07:16 -->
- [x] Update `docs/concepts/member-lifecycle.md`: pending placement is ping-tolerant (skip semantics), one-line note in the lifecycle narrative <!-- completed: 2026-07-30T07:17 -->
- [x] Update `docs/spec/cli-options.md`: subcommand summary table; `member ping` section (skip contract of §1, `skipped` JSON key); delete the `member capture` section and add `monitor capture` under the monitor group; delete `monitor status` / `monitor config` / `monitor stall *` / `monitor report-batch` sections; update the error-message table; update `permissions.allow` coverage for the respelled capture <!-- completed: 2026-07-30T07:24 -->
- [x] Update `docs/spec/data-model.md`: `monitor_config` column list, drop the two tables, remove episode-state vocabulary, document revision `0006` <!-- completed: 2026-07-30T07:27 -->
- [x] Update `docs/spec/multiplexer-backends.md`: the exact §3 pure-trigger wake payload and unchanged poll/preview primitives <!-- completed: 2026-07-30T07:31 -->
- [x] Update `docs/api/*` pages that reference deleted broker functions or removed CLI endpoints — audited all four pages; they name only kept functions, so no edit was needed <!-- completed: 2026-07-30T07:33 -->
- [x] Update `SPEC.md`: §5.3 enums (episode states/reasons gone), §6.2 broker (stall machine removal, plain per-event Director messaging), §6.3 CLI (require-pane helper narrowed to capture/prompt; `member ping` skip contract; monitor group = `start` + `capture`), §6.5/§6.6 (pure-trigger wake payload, startup line), §6.8 (WebUI monitor shape), §8 schema, §10 checklist <!-- completed: 2026-07-30T07:52 -->
- [x] Update `skills/cafleet/SKILL.md`: Team supervision section and the fixed monitoring-member ping exception paragraph (two-wake self-confirmation covering idle and stalled members, plain per-event message-send escalation, no broker claim/aggregate) <!-- completed: 2026-07-30T07:55 -->
- [x] Update `skills/cafleet/reference/supervision.md`: stall-response and escalation flow (plain per-event messages replace the aggregate/gate), Idle Semantics (`finished` members become ping-eligible after two quiet wakes), the "pre-nudge" → "pre-ping" gate respelling, `monitor capture` respelling, handshake wording <!-- completed: 2026-07-30T08:05 -->
- [x] Update `skills/cafleet/reference/director.md`: respell the capture gate "pre-nudge" → "pre-ping" and its read → `monitor capture`; rename the heading `## Member Ping (manual inbox-poll nudge)` → `## Member Ping (manual inbox-poll)`; remove stall-episode/aggregate retrieval instructions (the Director now receives plain messages and ACKs them normally) <!-- completed: 2026-07-30T08:09 -->
- [x] Respell the remaining fixed-ping/wake-trigger "nudge" wording: `skills/cafleet/reference/prompt-routing.md` ("Fixed-action inbox-poll nudge" and its link to the renamed `director.md` heading anchor), `skills/cafleet/reference/recovery.md` ("ping … to nudge"), and `docs/concepts/overview.md` (mermaid edge label "wake nudge" → "wake trigger") <!-- completed: 2026-07-30T08:12 -->
- [x] Update `skills/cafleet/reference/cli.md`: command catalog (monitor group, capture move, ping skip) <!-- completed: 2026-07-30T08:15 -->
- [x] Rewrite `skills/cafleet/roles/monitor.md`: startup (launch task, confirm startup line, `ready: monitor live`), the §4 on-wake protocol as the sole normative carrier, the short §3 wake-trigger example, teardown unchanged <!-- completed: 2026-07-30T08:19 -->
- [x] Update `skills/cafleet/reference/coding-agent/` overlays + `_template.md`: replace "only the broker may promote … to `stalled`" with the two-wake self-confirmation rule covering both quiet families; respell "pre-nudge capture gate" → "pre-ping capture gate" in every *Note → applies at* row; respell capture commands <!-- completed: 2026-07-30T08:23 -->
- [x] Update `.claude/rules/bash-tool.md`: the monitoring-member ping exception paragraph (no broker claim / `action = ping`) <!-- completed: 2026-07-30T08:52 -->

- [x] Verify `README.md`'s thin surface is unaffected and the `skills/cafleet-research` "aggregate" mentions are generic wording; sync via `/update-readme` only if drift exists — README unaffected; "aggregate" mentions are the generic compile-findings sense; fixed sweep-relevant drift found in `skills/cafleet-research`, `skills/cafleet-design-doc`, and `skills/cafleet/roles/director.md` (`monitor status` handshake, `member capture` respell, monitoring-sense "idle-nudge") <!-- completed: 2026-07-30T08:38 -->

### Step 2: Schema

- [x] Update `db/models.py`: remove the four `MonitorConfig` episode columns and their CHECK constraints; delete the `MonitorDirectorGate` and `MonitorReportDelivery` models <!-- completed: 2026-07-30T08:56 -->
- [x] Generate migration 0006 via `mise //cafleet:makemigration "drop monitor stall episode state"`; hand-review to the §6 shape (batch recreate of `monitor_config` preserving the five kept columns; table drops; full `downgrade()`) <!-- completed: 2026-07-30T09:35 -->
- [x] Update the chain-guard and schema tests in `tests/db/test_alembic_smoke.py` per §6 — done by the Tester (commit bda93e09) <!-- completed: 2026-07-30T08:56 -->

### Step 3: Broker

- [x] Delete the stall-episode API and helpers listed in §2 from `broker/monitor.py` (and their `broker/__init__.py` exports); trim `_config_dict` / `_CONFIG_COLS` to the five kept fields <!-- completed: 2026-07-30T09:35 -->
- [x] Simplify lifecycle reconciliation: disable/dead/pending cleanup now only clears `last_stall_check_at` (no episode transitions) <!-- completed: 2026-07-30T09:35 -->
- [x] Trim `delete_fleet_monitor_rows` to `monitor_config` + `monitor_runtime` <!-- completed: 2026-07-30T09:35 -->

### Step 4: CLI

- [x] `cli/member.py`: implement the §1 ping skip (drop `_require_member_pane` from `member ping`; add the `skipped` key to both JSON paths; skip text line); narrow `_require_member_pane` docs/action set to capture/prompt; delete the `member capture` command <!-- completed: 2026-07-30T09:55 -->
- [x] `cli/monitor.py`: delete `status`, `config`, the `stall` group, and `report-batch`; add `monitor capture` (the moved implementation, contract unchanged) <!-- completed: 2026-07-30T09:55 -->
- [x] Update any internal callers/help text referencing the removed or moved commands (e.g. `doctor`, `member create` spawn scaffolding, `fleet delete` messages) found by the Step 7 sweep — swept `src/`: the dead `format_monitor_status` / `format_monitor_config` / `_format_ping_age` formatters and their `output/__init__.py` exports removed; `monitor/loop.py` `should_ping` docstring respelled; the remaining `multiplexer/base.py` payload body is Step 5 scope <!-- completed: 2026-07-30T09:55 -->

### Step 5: Monitor loop and wake payload

- [ ] `monitor/loop.py`: emit the startup line `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` after a successful runtime claim, before the first tick <!-- completed: -->
- [ ] `multiplexer/base.py`: rewrite `_monitor_wake_payload` to the exact §3 pure-trigger form (unchanged signature, entry rendering, sanitization, and validation) and update the `send_wake_trigger` docstrings; respell fixed-ping/wake-trigger "nudge" wording in source docstrings repo-wide (`multiplexer/tmux.py`, `broker/members.py`, `monitor/loop.py`) <!-- completed: -->

### Step 6: WebUI

- [ ] `webui/api.py`: confirm `GET /api/monitor` and `GET`/`PATCH /api/members/{id}/monitor` still function on the trimmed config dict; update the `monitor` response shape and any typed models <!-- completed: -->
- [ ] Sweep `admin/src` for episode-field usage (none expected) and rebuild via `mise //admin:build` if the API types are generated <!-- completed: -->

### Step 7: Tests and verification

- [ ] Delete `tests/broker/test_monitor_stall_state.py`, `tests/cli/test_monitor_stall.py`, and — with the output formatters — `tests/output/test_monitor_status_table.py` and `tests/output/test_ping_age_ascii.py` <!-- completed: -->
- [ ] Rewrite `tests/multiplexer/test_monitor_wake_contract.py` against the §3 pure-trigger payload: exact-string pin, tmux/herdr byte parity, sanitizer coverage, invalid-`coding_agent` fail-closed <!-- completed: -->
- [ ] Update `tests/cli/test_monitor.py`: delete the `monitor status` / `monitor config` sections; add the startup-line assertion to the `monitor start` tests <!-- completed: -->
- [ ] Update `tests/monitor/test_loop.py` (startup line; unchanged scan/wake behavior) and `tests/integration/test_direct_member_nudge.py` + `tests/docs/test_direct_member_nudge_docs.py` for the ping skip, respelled capture, and removed broker machinery <!-- completed: -->
- [ ] Update `tests/cli/test_member_ping.py` (existing ping tests take the `skipped` JSON key), rename `tests/cli/test_member_capture_defaults.py` → `tests/cli/test_monitor_capture_defaults.py` with its invocations respelled to `monitor capture`, and update `tests/cli/test_help_budget.py` (the `("member", "capture")` budget entry moves to the monitor group) <!-- completed: -->
- [ ] Update `tests/webui/test_monitor_api.py` (trimmed `monitor` shape) and `tests/broker/test_monitor.py` (config-dict assertions lose the episode columns) <!-- completed: -->
- [ ] Add CLI tests: ping skip in text/`--json`/`--quiet` (exit 0, `skipped` key both paths), placementless ping still errors, and the `monitor capture` contract (including the pending-placement hard error) <!-- completed: -->
- [ ] Repo-wide vocabulary sweep per Success Criteria (rg over source, docs, skills, rules, tests) <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint` <!-- completed: -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-07-29 | Initial draft |
| 2026-07-29 | Compiled the approved-but-unimplemented design 0000153 into this design as the single authoritative superset: durable stall-episode machine deleted in favor of the monitoring member's two-wake in-context judgment (now also covering `finished` idle members), `member ping` pending-placement no-op success, monitor group = `start` + `capture` with the `member capture` hard rename, migration 0006. Wake payload finalized as a pure trigger. 0000153 aborted. |
| 2026-07-29 | Reviewer round: Director capture dropped from the wake routine (descriptor identifies the report recipient); loss-tolerant `unknown` rule added; quiet baseline anchored to stall-check wakes; materially changed quiet content re-arms a new quiet period; ping's Director-target prohibition stated as instruction-level; capture-defaults test module rename made explicit. |
| 2026-07-29 | User feedback: "nudge" vocabulary replaced with "ping" (the real primitive) throughout, including the title, the guiding principle, and the authored doc/skill text (capture gate respelled "pre-ping"); removed-vocabulary literals, test filenames, and slugs kept verbatim. |
| 2026-07-29 | User feedback: batch-report semantics dropped — the monitoring member sends a plain per-event `cafleet message send` for anything needing Director attention, with no per-wake aggregation, summary framing, or one-message-per-wake rule; the once-per-quiet-period non-repetition rule stays. |
| 2026-07-29 | Reviewer round: nudge→ping respell coverage completed — overlay gate rows, the `director.md` heading/anchor rename with `prompt-routing.md`, `recovery.md`, and the `overview.md` mermaid label added to the task list; source docstrings declared in scope; a `nudge` sweep term added with carve-outs for the message-level stall-nudge concept and the kept test filenames. |
| 2026-07-30 | User feedback: absence-guard tests for removed subcommands dropped from the Step 7 task and the Success Criteria — once a command is gone, its absence is the test (removal rule); the `--help` listing assertions stay. |
