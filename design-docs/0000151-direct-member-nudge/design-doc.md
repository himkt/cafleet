# Direct Fixed-Action Nudge from the Monitoring Member

**Status**: Approved
**Progress**: 28/31 tasks complete
**Last Updated**: 2026-07-29

## Overview

Allow the dedicated monitoring member to invoke the existing fixed-action `cafleet member ping` directly when an ordinary member is confidently stalled, avoiding an extra Director round trip while preserving synchronized batched checks. Un-acked delivery state remains visible only as context, and all task dispatch and finished-work judgment remain with the Director (GitHub issue #228).

## Success Criteria

- [ ] The monitor loop continues to run one fixed-cadence `scan → one batched watcher wake → sleep` cycle with no jitter, per-member timer, or direct scheduler keystroke into a watched pane.
- [ ] A stale un-acked delivery can annotate an already-due member with `unacked`, but can never independently add a member to the due set or wake the monitoring member.
- [ ] On both tmux and herdr, the monitoring member directly invokes the existing `cafleet member ping` only for an ordinary member classified `stalled` from two consecutive byte-identical `stall_candidate` captures separated by a full stall interval.
- [ ] The direct nudge remains fixed action: it injects only `Esc` plus the target's `cafleet message poll` command and cannot carry a task body or arbitrary instruction.
- [ ] Each stall episode permits at most one successful direct nudge; an unchanged next synchronized capture escalates to the Director exactly once, while changed or non-stalled capture state resets the episode.
- [ ] A failed direct nudge is durably queued as `escalation_pending/ping_failed` in the same monitoring turn and is not retried during that stall episode.
- [ ] Capture fingerprint, episode state, and deferred-escalation reason are durable in SQLite, so restarting the monitoring member cannot repeat a claimed/successful nudge or lose a pending/already-reported escalation.
- [ ] Separate durable dispatch and candidate-observation timestamps make the monitor loop honor the remaining stall cadence after restart and make the broker reject hash promotion until two actual candidate captures are a full `CAFLEET_MONITOR_STALL_INTERVAL` apart.
- [ ] The monitoring member never directly nudges the Director, itself, an `awaiting_user`/`unknown`/`working`/`finished` ordinary member, or a member based only on `unacked`.
- [ ] If no normal wake can safely reach the Director, escalation stays sticky without creating a new wake; Director re-enable/live rebind or another normal due-member event is the explicit delivery precondition, and the next safe synchronized check reports it once.
- [ ] One synchronized wake emits at most one Director-targeting inline preview; when ACK reconciliation leaves no older open aggregate, all then-pending stall escalations and that wake's finished observations are committed as one fixed aggregate after ordinary-member actions complete.
- [ ] Aggregate preview delivery is durable and bounded: each fleet has at most one open aggregate, retries reuse its message ID, new escalations remain pending and repeated finished observations are not inserted while it is open, and only Director ACK completes delivery.
- [ ] After an older open aggregate is ACKed, every durable stall escalation that accumulated behind it remains queued and is included in the next aggregate formed on a Director-safe normal synchronized wake; ephemeral finished observations are included only when observed again on such a wake.
- [ ] The Director treats every aggregate preview as a notification, retrieves that message ID with `message show --full` before acting, processes the untruncated body, and ACKs the aggregate once.
- [ ] The final Director gate permits one aggregate preview only when the Director is `finished` or resolved `stalled` from two full-spacing unchanged Director observations; a fresh one-use broker token makes that gate enforceable by `report-batch`, and Director observation can never claim or run an ordinary-member ping.
- [ ] `finished` remains a report-only classification: the Director alone decides whether assigned work remains and whether to dispatch another task.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

The monitor loop currently builds a due set from four reasons: `interval`, `stall-check`, herdr's `status:done`, and `unacked`. It wakes the dedicated monitoring member once with the whole due batch. The monitoring member captures every named pane plus the Director, but its role is limited to `member capture` and `message send`: it reports stalled or finished members to the Director, and only the Director may run `member ping`.

That indirection is useful when judgment is required, but not for the narrow recovery action a confidently stalled member needs. `cafleet member ping` accepts no operator-controlled body; it can only inject an `Esc`-safeguarded `cafleet message poll --fleet-id … --member-id …` command. Requiring the watcher to report a deterministic two-capture stall and then waiting for the Director to repeat the capture before invoking that fixed action adds a full supervision turn without adding task-level judgment.

The independent `unacked` trigger also overstates what broker state proves. An un-acked message can coexist with legitimate in-progress work, so it is evidence worth showing during a scheduled check, not sufficient reason to schedule another check or nudge a pane.

---

## Specification

### Locked decisions

| Area | Decision |
|---|---|
| Scheduling | Preserve the existing fixed tick and synchronized batched wake. Add no jitter, per-member timer, delayed callback, pending-only trigger, or independent direct-nudge cadence. |
| Direct action | Reuse `cafleet member ping`; add no arbitrary-body monitor command and no new task-dispatch path. |
| Eligible target | An active ordinary member already named in the synchronized due batch and confidently classified `stalled`. The root Director and dedicated monitoring member are never direct-ping targets. |
| Stall confidence | Two consecutive byte-identical `--lines 120 --no-ansi` captures explicitly classified quiet `stall_candidate`, separated by the configured stall-check interval. A capture with any active-work cue is `working` and never hash-promoted. |
| `unacked` | Annotation on an already-due member only. It never creates a due member, directly causes a ping, or makes `working` reportable by itself. |
| Episode state | The last stall-check dispatch, last accepted capture timestamp/fingerprint, nudge/escalation state, and escalation reason are persisted on the watched member's `monitor_config` row. Monitoring-member and monitor-loop restarts preserve cadence and resume the same episode without repeating a nudge or escalation. |
| Retry/escalation | First confident stall: one direct ping. Same capture at the next synchronized check: queue one Director escalation. Ping failure: queue the escalation immediately and never retry in that episode. If no older aggregate survives ACK reconciliation, a safe end-of-wake batch includes it; otherwise it remains durable until a later safe wake after the open aggregate is ACKed. |
| Completion | A `finished` pane is reported to the Director. Only the Director knows the assignment ledger and judges whether more work is owed. |
| Director delivery | Defer every Director-targeting effect until the end of the wake, then re-capture the Director. Only `finished` or broker-resolved `stalled` issues a fresh one-use Director-gate token accepted by one immediate fixed aggregate report/preview; `awaiting_user`, `working`, unresolved `stall_candidate`, and `unknown` invalidate any prior token and defer delivery without suppressing eligible ordinary-member pings. |
| Preview recovery | Keep at most one open aggregate per fleet. Preview success becomes `awaiting_ack`, ACK alone completes delivery, and a stale un-ACKed/failed preview retries the same message ID only on a later normal wake with a new safe Director-gate token. |
| Compatibility | No existing command arguments, HTTP API, cadence setting, or monitor-status/WebUI shape changes. Additive `captured_at`/`content_sha256` fields on `member capture --json`, internal `coding_agent` scan/wake metadata, narrow `monitor stall` and `monitor report-batch` state surfaces, five `monitor_config` columns, `monitor_report_delivery` and `monitor_director_gate` tables, and one Alembic migration are added. |

### 1. Synchronized monitor scheduling

The monitor loop retains its current responsibility boundary:

```text
fixed tick
  → scan every watched member once
  → reconcile disabled / placement-pending / dead episode rows
  → union interval / stall-check / status:done triggers
  → annotate already-due rows with stale unacked context
  → send one wake containing the complete due batch to the monitoring member
  → commit cadence bookkeeping only if that one wake succeeds
  → sleep for the configured tick
```

The loop never calls `send_poll_trigger` and never directly keystrokes the Director or an ordinary member. The direct poll nudge is an agent action performed during the monitoring member's handling of that synchronized wake, after capture-based classification. Consequently:

- multiple due members still produce one watcher wake;
- the follow-up boundary is the member's next normal synchronized due wake; the direct nudge does not start a separate timer;
- `last_ping_at`, `monitor_config.interval_seconds`, `CAFLEET_MONITOR_STALL_INTERVAL`, and `tick_seconds` retain their existing meanings; the stall-check timestamp moves from `_last_stall_check_at` process memory into `monitor_config.last_stall_check_at`;
- `CAFLEET_MONITOR_STALL_INTERVAL=0` disables stall classification and therefore disables monitor-direct nudges.

### 2. `unacked` becomes annotation-only

Replace `_flag_unacked_due(targets, due, now)` with an annotation helper that iterates only the members already present in `due`. A member receives the `unacked` annotation when its existing `oldest_pending_ts` is non-null and at least its own `interval_seconds` old. The staleness threshold remains unchanged so ordinary send/ack latency does not pollute wake context.

Trigger construction order becomes:

1. reconcile disabled, placement-pending, and dead rows so any in-flight claim becomes discoverable pending state;
2. add interval-due members with `interval`;
3. union stall-check-due members with `stall-check`;
4. on an `AgentStateAware` backend, add native `done` transitions with `status:done`;
5. append `unacked` to any already-due row whose oldest delivery is stale.

This ordering ensures `unacked` describes every kind of scheduled due row, including a herdr `status:done` row, while remaining incapable of adding a row. The reason is always appended last; representative reason lists are `[interval,stall-check,unacked]` and `[status:done,unacked]`.

The process-local `_last_unacked_wake_at` map, its per-run clear, its successful-wake commit, and its re-fire policy are removed. They are unnecessary once `unacked` cannot schedule a wake: the annotation simply appears on each existing synchronized check while the stale delivery remains and disappears after ACK. `pending_count`, `oldest_pending_ts`, `oldest_pending_age_seconds`, monitor-status output, and WebUI exposure remain unchanged.

Normative cases:

| Member state at a tick | Due? | `unacked` annotation? |
|---|---:|---:|
| No interval/stall/native trigger; stale un-acked delivery | No | No wake row exists to annotate |
| Interval due; stale un-acked delivery | Yes | Yes |
| Stall-check due; delivery younger than one member interval | Yes | No |
| Native `done` transition; stale un-acked delivery | Yes | Yes |
| Disabled, placement-pending, or dead pane | No | No |

The annotation is only context for interpreting the capture. It does not alter the five-state classification or action table: in particular, `working + unacked` is still `working` and causes no direct nudge or report solely because of the delivery.

### 3. Durable episode state and capture identity

The existing `monitor_config` row is the minimal durable home because every eligible ordinary member already has exactly one row, the monitoring member has none, and soft deregistration already explicitly deletes the row in the same transaction as placement cleanup. Add five columns:

| Column | Type/default | Contract |
|---|---|---|
| `last_stall_check_at` | nullable `TEXT` | UTC ISO-8601 timestamp of the last successfully dispatched stall-check wake. It is the durable cadence gate across loop restart. |
| `last_stall_candidate_at` | nullable `TEXT` | Validated UTC ISO-8601 `captured_at` of the last accepted stall-check candidate used as the confidence baseline. It is distinct from dispatch time. |
| `last_stall_capture_sha256` | nullable `TEXT` | Lowercase 64-character SHA-256 of the last `--lines 120 --no-ansi` stall-check capture's emitted `content`. `NULL` means no usable baseline. |
| `stall_episode_state` | non-null `TEXT`, default `"clear"` | One of `clear`, `nudge_claimed`, `nudged`, `escalation_pending`, `escalated`. |
| `stall_escalation_reason` | nullable `TEXT` | `ping_failed`, `ping_interrupted`, or `unchanged_after_nudge`; non-null only for `escalation_pending` / `escalated`. |

Database checks enforce the enumerations and cross-field invariants:

- `clear`, `nudge_claimed`, and `nudged` require a null escalation reason;
- `escalation_pending` and `escalated` require a non-null escalation reason;
- candidate time and capture fingerprint are either both null or both non-null;
- every non-`clear` state requires a non-null candidate time and capture fingerprint.

Alembic revision `0005_add_monitor_stall_episode_state.py` adds the columns with server defaults that backfill every existing row to `(NULL, NULL, NULL, "clear", NULL)`, then retains the `"clear"` server default for newly enrolled members. Episode writes address the existing `monitor_config.member_id` primary key.

The same migration creates `monitor_report_delivery`, the durable preview queue:

| Column | Type/default | Contract |
|---|---|---|
| `message_id` | `INTEGER` primary key, FK `messages.message_id` | Exactly one delivery row per aggregate broker message; retries reuse this ID. |
| `fleet_id` | non-null `INTEGER`, FK `fleets.fleet_id` | Fleet scope for ordered pending lookup and cleanup. |
| `preview_state` | non-null `TEXT`, default `"pending"` | `pending`, `awaiting_ack`, or `delivered`. |
| `attempt_count` | non-null `INTEGER`, default `0` | Incremented only when a preview call returns a known success/failure. |
| `last_attempt_at` | nullable `TEXT` | Broker-clock UTC timestamp of the last recorded attempt. |
| `delivered_at` | nullable `TEXT` | Set only when reconciliation observes the aggregate is ACKed/non-`input_required`. |

Checks enforce the preview-state enumeration, non-negative attempts, `(attempt_count = 0) == (last_attempt_at IS NULL)`, `delivered_at IS NOT NULL` exactly for `delivered`, and at least one recorded attempt for `awaiting_ack`. A partial unique index on `fleet_id WHERE preview_state IN ('pending','awaiting_ack')` enforces one open aggregate per fleet; an ordered `(fleet_id, preview_state, message_id)` index supports reconciliation.

The migration also creates `monitor_director_gate`, the fleet-scoped consumable proof required by `report-batch`:

| Column | Type/default | Contract |
|---|---|---|
| `fleet_id` | `INTEGER` primary key, FK `fleets.fleet_id` | At most one unconsumed gate exists per fleet. |
| `director_member_id` | non-null `INTEGER`, FK `members.member_id` | The active root Director whose final capture produced the gate. |
| `token_sha256` | non-null `TEXT` | SHA-256 of the random 32-byte token returned once to the monitor; the raw token is never stored. |
| `classification` | non-null `TEXT` | Safe result `finished` or resolved `stalled`. |
| `issued_at` | non-null `TEXT` | Broker-clock UTC issue time. |
| `expires_at` | non-null `TEXT` | Exactly 30 seconds after issue; `report-batch` rejects a token at or after this time. |

Checks enforce a lowercase 64-hex token digest, the two safe classifications, and `expires_at > issued_at`. `delete_fleet_monitor_rows` explicitly deletes delivery and gate rows during fleet teardown; message deletion also cascades delivery rows by FK. Director deregistration/replacement deletes its gate row. Downgrade drops both tables before the five columns.

`cafleet member capture --json` gains additive fields `captured_at` and `content_sha256`. `captured_at` is stamped from the local UTC clock at the capture read boundary. `content_sha256` is always computed from the exact UTF-8 bytes of the emitted `content` string in that same JSON object:

- `--no-ansi --json` (default): hash the ANSI-stripped, carriage-return-defragmented emitted content;
- `--ansi --json`: hash the ANSI-preserving emitted content.

No hidden normalization is applied after the selected capture mode. The monitoring routine always uses `--no-ansi` at the normative depth:

```bash
cafleet member capture --fleet-id <fleet-id> \
  --member-id <member-id> --lines 120 --no-ansi --json
```

The hash lets the broker apply byte-identity transitions without storing terminal content in SQLite. Capture text can contain prompts, source, or secrets and remains confined to the existing capture response; only its one-way fingerprint is persisted.

The capture JSON key order remains `member_id`, `pane_id`, `lines`, `content`, then additive `captured_at`, `content_sha256`.

The broker owns atomic state transitions through three stall functions plus aggregate Director-report creation/delivery functions:

| Broker operation | CLI form | Result |
|---|---|---|
| `observe_stall_episode(...)` | `monitor stall observe --member-id <id> [--captured-at <UTC-ISO>] [--capture-sha256 <hex>] --classification <state> [--stall-check\|--director-gate] --json` | Resolves an explicit `stall_candidate`, persists/reset baseline and episode state, and returns the resolved classification plus `action = none`, `ping`, or `escalate`; Director-gate mode can resolve `stalled` but always returns `none` and issues a short-lived token only for `finished`/`stalled`. |
| `record_stall_ping_result(...)` | `monitor stall ping-result --member-id <id> --success\|--failure --json` | Transitions a claimed nudge to `nudged` or `escalation_pending/ping_failed`. |
| `list_pending_stall_escalations(...)` | `monitor stall pending --json` | Lists durable pending reports for all enrolled ordinary members, including disabled or dead-pane targets omitted from the due set. |
| `report_monitor_batch(...)` | `monitor report-batch --director-gate-token <64-hex-token> [--finished-member-id <id>]... --json` | Atomically validates/consumes a fresh safe Director gate, reconciles ACK, applies one-open-batch backpressure, creates a new aggregate only when none is open, and returns that open message only when its retry policy permits at most one preview. |
| `record_monitor_report_preview(...)` | Internal final phase of `monitor report-batch` | Records the one attempted preview outcome: first success moves `pending → awaiting_ack`; failure keeps the current open state; neither completes delivery without ACK. |

Exact command result fields are:

| Command | JSON keys in order | Text output |
|---|---|---|
| `observe` | `member_id`, `classification`, `action`, `episode_state`, `escalation_reason`, `director_gate_token` | `member <id>: <classification>, action <action>, episode <state>, reason <reason-or->, director gate <token-or->` |
| `ping-result` | `member_id`, `episode_state`, `escalation_reason` | `member <id>: episode <state>, reason <reason-or->` |
| `pending` | top-level `members`; each row `member_id`, `name`, `escalation_reason` | One `member <id> (<name>): <reason>` line per row; `(no pending stall escalations)` when empty |
| `report-batch` | `created_message_id`, `open_message_id`, `preview_message_id`, `escalated_member_ids`, `finished_member_ids`, `created`, `preview_outcome` | `monitor report batch: created <id-or->, open <id-or->, preview <id-or-> <awaiting_ack|failed|none>, <n> escalated, <n> finished` |

Ordinary-mode readable `observe` classifications verify that the target is an active, enabled, enrolled ordinary member with a currently live placement. The ordinary loss-tolerant input is `--classification unknown` with both capture fields omitted: it validates only the retained active/enrolled ordinary member and `monitor_config` row, then applies the same reconciliation as lifecycle cleanup even if the pane disappeared or monitoring was disabled after the scheduler scan. It converts `nudge_claimed` to sticky `escalation_pending/ping_interrupted`, preserves existing pending state, and otherwise clears the episode/candidate baseline. The result is resolved `unknown` with `action = escalate` when state is pending and `action = none` otherwise. This lets a scan-live → capture-fails race reset or preserve state immediately instead of waiting for the next scheduler tick.

`--director-gate` is mutually exclusive with `--stall-check` and accepted only for the fleet's active, enrolled root Director. The monitor uses it for the final synchronized Director capture on every wake. At the start of every Director-gate transaction, the broker deletes any prior unconsumed gate for the fleet, so a later observation always invalidates an earlier result. It applies the same validated timestamp/full-interval candidate algorithm but never writes `nudge_claimed`, `nudged`, or escalation state and always returns `action = none`: first/too-early candidate → `unknown`, full-spacing changed candidate → `working`, full-spacing identical candidate → resolved `stalled`. Explicit `finished`, `working`, or `awaiting_user` clears the Director candidate baseline as appropriate. A no-capture `unknown` is loss-tolerant for the retained Director row and clears its candidate baseline.

Only resolved `finished` or `stalled` creates a new random 32-byte gate value, returns its lowercase-hex encoding as `director_gate_token`, and stores SHA-256 of the decoded 32 bytes with the Director identity and broker-clock 30-second expiry in `monitor_director_gate`; the raw value is never stored. Every other result returns null. `report-batch` requires that raw hex token, decodes and hashes it, and in its transaction verifies the one row belongs to the current active root Director and has not expired; it then deletes the row before ACK reconciliation and message selection. The deletion, validation, reconciliation, and any aggregate insertion/state advance commit atomically, so a later validation/insertion error rolls consumption back. Missing/malformed token is usage error (exit 2); absent, mismatched, expired, wrong-Director, or already-consumed token is an application error (exit 1) with no preview or state mutation. An expired row remains unusable until the next accepted Director-gate observation replaces/deletes it or lifecycle cleanup removes it.

The token is single-use under concurrency: only the first serialized `report-batch` transaction can consume it, and every concurrent/replayed call fails without a second preview. It survives a process restart only as an unusable digest and expires after 30 seconds; because the raw value exists only in the interrupted process response, a restarted monitoring member must perform a new final Director capture/gate. Director disablement, deregistration/replacement, placement loss, and fleet teardown delete any outstanding gate. Thus two actual full-spacing unchanged Director captures or one explicit finished capture can authorize exactly one immediate report command, but no Director observation can authorize `member ping`.

`ping-result` deliberately does not recheck enablement, placement, or pane liveness: it validates fleet membership, ordinary-member enrollment, the retained `monitor_config` row, and the durable claim/result state so the fixed action's actual result remains recordable after the target disappears. It always rejects the Director. `report-batch` can drain pending rows for disabled or currently dead ordinary panes; its Director must still be active and pane-bound, and the supplied fresh gate must identify that current Director.

The dedicated monitoring member is unenrolled and rejected by every state path. SHA-256 input must be exactly 64 lowercase hexadecimal characters. A readable classification requires both capture fields; loss-tolerant `unknown` omits both. `captured_at` must parse as timezone-aware UTC, must not be later than the broker transaction clock, and must be strictly newer than the stored candidate time before it can replace or promote a baseline. `--success` and `--failure` are mutually exclusive and one is required.

Lifecycle cleanup and `ping-result` serialize as SQLite writes. Failure has the more specific result: from `nudge_claimed` it records `escalation_pending/ping_failed`, and from `escalation_pending/ping_interrupted` it atomically upgrades the reason to `ping_failed`. Therefore cleanup-before-failure and failure-before-cleanup converge on the same sticky state; the monitor then reports it in that safe wake. Success from `nudge_claimed` records `nudged`; a late success when cleanup has already converted the claim to `escalation_pending/ping_interrupted` is an idempotent no-op that cannot erase the safer escalation. Cleanup preserves any pending reason.

Wrong-fleet, ineligible, mode/role-mismatched, monitoring-member, and soft-deregistered targets fail without mutation. Exact success replay in `nudged`, exact failure replay in `escalation_pending/ping_failed`, and late success in `escalation_pending/ping_interrupted` are no-ops; contradictory outcomes otherwise fail. With a valid consumed gate, `report-batch` with an open delivery creates no message and leaves current escalation rows/finished IDs unconsumed; without an open delivery or new entry it returns `created = false`, null message IDs, and `preview_outcome = none`. Repeated finished IDs are deduplicated and output IDs are ascending. Invalid target/state/token conditions are application errors (exit 1); malformed hashes, gate tokens, classifications, mutually exclusive/missing flags/modes, or malformed finished-ID input are usage errors (exit 2).

The monitoring member classifies current content as `awaiting_user`, `unknown`, `finished`, `working`, or `stall_candidate`; it does not remember the previous hash. `stall_candidate` means quiet non-finished content with no prompt, no active tool/streaming/working indicator, and no other active-work cue. Any ambiguity between candidate and active work is `working`. It passes the `captured_at` and hash returned by that same capture immediately to `observe`; both are omitted only when `unknown` means dead/unreadable.

Only `stall_candidate` can be hash-promoted. Given `--classification stall_candidate --stall-check` in `clear`, the broker validates the capture timestamp against its transaction clock and resolves: no baseline → `unknown` and seed hash/time; a duplicate/out-of-order timestamp or elapsed time below `settings.monitor_stall_interval` → `unknown` with no baseline/time mutation; a full interval with changed hash → `working` and replace hash/time; a full interval with identical hash → `stalled`, update candidate time, and claim the episode. `--stall-check` declares the scheduler reason but cannot bypass this broker check, and a disabled (`0`) interval cannot promote. Explicit `working` is always non-actionable and resets any non-pending episode even when its hash is identical. A non-stall-check candidate cannot start an episode or replace the baseline. Once state is `nudge_claimed` or `nudged`, the next synchronized candidate with a strictly newer validated capture timestamp is the follow-up: equality queues escalation, while inequality resets; duplicate/out-of-order candidates are no-ops, and `working` still resets.

`observe` transactionally claims a first nudge by writing `nudge_claimed` before returning `action = ping`. This closes the restart race: a restart after the claim, whether before or after the keystroke, must not repeat it. On the next strictly newer synchronized observation, a still-`nudge_claimed` episode whose capture remains unchanged becomes `escalation_pending/ping_interrupted`; the monitor escalates the unknown outcome instead of risking a duplicate ping. A changed or higher-precedence non-stalled observation resets the episode instead.

The scheduler reads `last_stall_check_at` from each scan row instead of a process-local map. A member is stall-check dispatch-due only when this durable timestamp is null or the full configured interval has elapsed. A successful batched watcher wake records `now` for every row tagged `stall-check`; a failed wake records nothing. Separately, `observe` compares the validated capture timestamp with `last_stall_candidate_at` and the same configured interval. Monitor-loop restart therefore honors the remaining dispatch interval, delayed wake handling cannot make closely spaced captures confident, and duplicate/direct `observe` calls cannot hash-promote or push the accepted baseline forward.

### 4. Capture classification and stall episodes

The existing precedence remains safety-critical:

| Classification | Evidence | Monitor action for an ordinary member |
|---|---|---|
| `awaiting_user` | Unanswered question or permission prompt | None; reset any stall episode |
| `unknown` | Dead/unreadable pane, first stall-check candidate with no durable baseline, or a candidate too early to compare safely | None; preserve pending state, otherwise reset as specified below and retain only a newly accepted baseline |
| `finished` | Completed turn at an empty input prompt | Report to the Director; reset any stall episode |
| `working` | Any affirmative or ambiguous in-flight cue, including an active tool/stream/working indicator | None; always non-actionable and resets any non-pending episode |
| `stall_candidate` (typed input only) | Quiet non-finished content with no prompt and no active-work cue | Submit to `observe`; only the broker may resolve it to `unknown`, `working`, or `stalled` |
| `stalled` (resolved only) | A stall-check `stall_candidate` fingerprint equals the durable prior candidate fingerprint after a full interval, with no higher-precedence state | Run the episode action below |

The ambiguity tie-breaks remain unchanged: ambiguity between `awaiting_user` and `finished` resolves to `awaiting_user`; ambiguity between a true stall candidate and active work is submitted as `working`. Hash equality never overrides `working`. Captures taken only for `interval` or `status:done` remain observational and do not replace the stall baseline unless that member is also tagged `stall-check`.

Backend-specific affirmative-work and quiet-candidate cues are normative in `skills/cafleet/reference/coding-agent/{claude,codex,opencode}-overlay.md`, with the required note/application mapping defined in `_template.md`. `list_monitor_targets` selects internal `MemberPlacement.coding_agent` metadata without adding it to monitor-status/WebUI output. `send_wake_trigger` receives a Director descriptor (`member_id`, `coding_agent`) plus due rows that each carry `coding_agent`; the rendered entry is `<role> <id> (<sanitized-name>; coding_agent=<sanitized-agent>) [<reasons>]`, and the standing Director clause includes the same coding-agent field when the Director is not due. Both values pass through the single-line wake sanitizer, and an absent/unregistered value aborts the wake without cadence commit.

The monitor applies the overlay named on the **target member's** rendered entry, not its own. Each overlay names visible streaming/generation/tool indicators that force `working`, the at-rest quiet form eligible for `stall_candidate`, and ambiguous/truncated cases that force `working`; role contract tests cover mixed claude/codex/opencode due rows and an independently identified Director backend.

For each ordinary-member pane, SQLite records:

```text
last_stall_check_at: UTC ISO-8601 timestamp | null
last_stall_candidate_at: UTC ISO-8601 timestamp | null
last_stall_capture_sha256: 64 lowercase hex characters | null
stall_episode_state: clear | nudge_claimed | nudged | escalation_pending | escalated
stall_escalation_reason: null | ping_failed | ping_interrupted | unchanged_after_nudge
```

An episode begins when two full-interval-separated stall-check `stall_candidate` fingerprints are identical and the second resolves `stalled`. For `clear`, `nudge_claimed`, `nudged`, or `escalated`, a later changed, explicit `working`, `finished`, `awaiting_user`, or unreadable observation ends the episode: `observe` returns the row to `clear` and clears the reason. A full-spacing changed stall-check candidate becomes the new candidate time/fingerprint; explicit non-candidate or unreadable content clears both so a later candidate must seed again. A duplicate, out-of-order, or too-early candidate is a no-op and cannot end, advance, or promote an episode.

Lifecycle cleanup also has a non-agent owner because disabled/dead rows are omitted from the wake payload:

| Event | Durable cleanup |
|---|---|
| `monitor config --disable` | In the same transaction, always clear `last_stall_check_at`. For `nudge_claimed`, preserve candidate time/fingerprint and convert to sticky `escalation_pending/ping_interrupted`; for existing `escalation_pending`, preserve candidate fields/state/reason; otherwise clear candidate fields and reset the episode to `clear`. Disabling the Director also deletes its outstanding gate token. |
| Scheduler sees placement-pending or dead pane | Before due filtering, apply the same cleanup in one batched broker update; when the row is the Director, also delete its outstanding gate token. |
| Member deregistration | Preserve `deregister_member`'s explicit `delete_member_monitor_row` call in the same transaction as soft-deregistering the member and deleting its placement; teardown is the terminal exception to pending-escalation retention. |
| Re-enable or live rebind after cleanup | Null dispatch timestamp makes the first live tick stall-check due; null candidate time/baseline makes the first candidate resolve `unknown` and seed only, never ping. |

Captures on interval/status-only wakes do not establish initial stall confidence or replace the stall-check baseline. After a nudge is claimed/successful, however, the very next synchronized observation is the follow-up regardless of wake reason: an unchanged `stall_candidate` escalates; changed or explicit non-stalled content resets unless escalation is already pending. Only a broker-accepted, full-spacing stall-check candidate replaces the baseline after comparison. A monitoring-member restart reads the durable row through the next `observe`; a monitor-loop restart reads the durable cadence timestamp.

### 5. One fixed-action direct nudge per stall episode

The episode transition is deterministic:

| Durable state | Synchronized observation | Atomic/action sequence | Next durable state |
|---|---|---|---|
| `clear` | First confident `stalled` classification | `observe` claims `nudge_claimed` and returns `ping`; invoke `cafleet member ping` once | `nudged` after successful `ping-result` |
| `nudge_claimed` | `member ping` reports failure | `ping-result --failure` records `escalation_pending/ping_failed`; queue Director report | `escalation_pending` |
| `nudge_claimed` | Monitoring member restarts before recording a result | Next synchronized unchanged `stall_candidate` records `escalation_pending/ping_interrupted`; never retry | `escalation_pending` |
| `nudged` | Next synchronized capture is the unchanged `stall_candidate` | `observe` records `escalation_pending/unchanged_after_nudge`; queue Director report | `escalation_pending` |
| `nudged` | Capture changes or is no longer `stalled` | `observe` closes episode | `clear` |
| `escalation_pending` | Director is safe and no older aggregate survives ACK reconciliation | Consume the gate and include the row in `report-batch`; persist its fixed aggregate entry and final state in one transaction | `escalated` |
| `escalation_pending` | An older aggregate remains open, the Director gate is unsafe, or any member observation/lifecycle change occurs before report | Preserve state, reason, and episode fingerprint; return `action = escalate` on observation and defer any new baseline | `escalation_pending` |
| `escalated` | Capture remains unchanged and `stalled` | No further ping or duplicate escalation | `escalated` |
| `clear`, `nudge_claimed`, `nudged`, or `escalated` | Episode-ending condition | Atomically clear episode/reason; retain/update baseline per §4 | `clear` |

`escalation_pending` is sticky until a `report-batch` transaction includes it. Member progress, finish, disable, unreadability, or pane death cannot erase the queued reason; observations while pending do not replace its fingerprint. After the aggregate report is durably inserted and state becomes `escalated`, the next episode-ending observation or lifecycle cleanup may clear the row and seed a later baseline normally.

The direct command is exactly:

```bash
cafleet member ping --fleet-id <fleet-id> --member-id <ordinary-member-id>
```

It runs only after `stall observe` has durably returned `action = ping`. Immediately after the CLI result, the monitoring member records it with exactly one of:

```bash
cafleet monitor stall ping-result --fleet-id <fleet-id> \
  --member-id <ordinary-member-id> --success

cafleet monitor stall ping-result --fleet-id <fleet-id> \
  --member-id <ordinary-member-id> --failure
```

The claim-before-keystroke ordering makes the safety guarantee at-most-once across crashes. A failed/ambiguous attempt may cause earlier escalation, but never a second direct ping in the same unchanged episode.

No message body is accepted. The existing implementation resolves the member's pane and dispatches only:

```text
Escape
cafleet message poll --fleet-id <fleet-id> --member-id <ordinary-member-id>
Enter
```

The monitor must not substitute `cafleet message send`, `cafleet message broadcast`, or `cafleet member prompt` for this action and must never address an ordinary member with task text. Ordinary members remain prohibited from calling Director primitives; `skills/cafleet/roles/director.md` documents the monitoring member's single `member ping` exception without broadening any other member authority.

### 6. Aggregate reporting and Director ownership

All synchronized-wake reporting uses one typed command:

```bash
cafleet monitor report-batch --fleet-id <fleet-id> \
  --director-gate-token <64-lowercase-hex-token> \
  [--finished-member-id <ordinary-member-id>]...
```

The command accepts no text. In one SQLite transaction it:

1. resolves the fleet's active monitoring member and active pane-bound root Director;
2. validates the supplied token against the current unexpired `monitor_director_gate` row for that Director and consumes the row;
3. reconciles the fleet's open delivery to `delivered` only if its broker message is ACKed/non-`input_required`;
4. if an open `pending`/`awaiting_ack` delivery remains, applies backpressure: selects no new escalation rows, ignores the ephemeral finished-ID set, and inserts no message;
5. otherwise selects every `escalation_pending` ordinary-member row in ascending `member_id` order, including disabled/dead targets, and validates/deduplicates the finished IDs as active, enrolled ordinary members in the same fleet;
6. when either new entry set is non-empty, inserts one direct `input_required` message plus its pending delivery row and transitions only those selected escalation rows to `escalated`; and
7. returns the single open delivery for preview only when it is `pending`, or when it is `awaiting_ack` and its last attempt is at least the Director's `monitor_config.interval_seconds` old.

The message uses `from_member_id` = active monitoring member and `to_member_id` = Director. Its exact fixed shape is:

```text
monitor report batch:
- <entry 1>
- <entry 2>
```

Escalation entries precede finished entries, each group sorted by member ID. Member names pass through the same single-line control/metacharacter sanitizer as wake entries before interpolation, so a name cannot add an aggregate line or instruction. Entry templates are:

| Entry | Exact line after `- ` |
|---|---|
| `ping_failed` | `monitor escalation: member <id> (<name>) direct inbox-poll nudge failed` |
| `ping_interrupted` | `monitor escalation: member <id> (<name>) direct inbox-poll nudge outcome unknown before its result was recorded` |
| `unchanged_after_nudge` | `monitor escalation: member <id> (<name>) unchanged at next synchronized check after direct inbox-poll nudge` |
| `finished` | `monitor finished: member <id> (<name>) is at an empty input prompt; Director must decide whether assigned work remains` |

With neither an open delivery nor new entries, the transaction inserts nothing. If new-entry validation or insertion fails, no episode advances. Every inserted aggregate gets the fleet's sole `monitor_report_delivery(pending)` row in the same transaction. While it remains open, new stall escalations stay sticky in `escalation_pending`, and repeated still-finished observations remain ephemeral; neither can create duplicate logical entries or grow a message queue. After ACK completes the open row, a later safe wake can form the next aggregate from then-current pending/finished inputs.

After commit, the CLI invokes `send_inline_preview` at most once for the eligible open message and records the known outcome. The first success moves `pending → awaiting_ack`, increments `attempt_count`, and stamps `last_attempt_at`; it does **not** mark delivery complete. A known failure increments/stamps the attempt and retains `pending` (or retains `awaiting_ack` if an earlier attempt succeeded). If outcome recording itself fails, the prior open state remains and the command must not make a second attempt in that wake.

An `awaiting_ack` row whose message remains `input_required` is eligible for same-ID re-preview only after one full Director monitor interval has elapsed since `last_attempt_at`, checked during a later normal synchronized wake. A `pending` row from a known/ambiguous failure is eligible on the next safe normal wake. Neither state creates a wake: `unacked` may annotate an already-due Director row but never schedules recovery. Only observing the message ACK/non-`input_required` in a later `report-batch` transaction sets `delivered_at` and terminal `delivered`.

If the process crashes after the keystroke but before outcome recording, the delivery remains in its prior open state. A Director ACK observed at the next transaction completes it without another preview; otherwise the same message ID follows the pending/awaiting-ACK retry rule. Thus the broker report is exactly-once and the open-row count is bounded at one; preview delivery is at-least-once only until ACK, with interval throttling after a known success and message ID as the Director's deduplication key. If no active monitoring member/Director exists, the operation fails and all episode/delivery rows remain retryable.

The ordinary inline preview still applies `CAFLEET_MAX_TEXT_LEN` and may truncate an aggregate after a few entries. Its envelope always carries the durable `message_id`; preview text is notification, not the action source. On every `monitor report batch` preview, the Director must first retrieve:

```bash
cafleet message show --fleet-id <fleet-id> \
  --member-id <director-member-id> --message-id <message-id> --full
```

The Director processes every entry from that untruncated body, uses the message ID to avoid replaying already-handled work after an ambiguous re-preview, and only then ACKs the one aggregate message. It must never act or ACK from the truncated inline preview alone. `message poll --full` is allowed when consuming the whole inbox, but the ID-specific `message show --full` form is the normative retry-safe path.

```bash
cafleet message ack --fleet-id <fleet-id> \
  --member-id <director-member-id> --message-id <message-id>
```

A ping failure becomes durable pending state in the same monitoring turn. It is included in that wake's end-of-wake aggregate only when the Director is safely `finished` or broker-resolved `stalled` **and** ACK reconciliation leaves no older open aggregate. If an older aggregate survives, backpressure leaves the failure pending until a later Director-safe normal wake after that message is ACKed; finished IDs from the blocked wake remain ephemeral and must be re-observed to enter a later aggregate. Any unsafe Director classification suppresses only the aggregate Director-targeting effect; it does **not** suppress an otherwise eligible fixed `member ping` to an ordinary member. The monitor therefore never claims a nudge it intentionally withholds because of Director state.

At the start of every watcher wake, `monitor stall pending` surfaces queued reports even when their members are disabled or dead and absent from the named due set. Unless the final Director gate resolves `finished` or `stalled`, the monitor does not call `report-batch`: pending state remains untouched, finished IDs are not sent, and no Director preview fires. Pending escalation deliberately does not create an independent wake. If the Director is disabled, placement-pending, or dead and no other row is due, Director re-enable/live rebind or another ordinary member becoming normally due is the external event required to resume delivery. The next normal synchronized wake re-captures the Director and drains queued work only when it is finished or confidently stalled; a disabled but live/safe Director may therefore receive the durable report during an ordinary member's normal wake, while a dead/unreadable Director must first recover. Member progress/disappearance while waiting cannot clear pending rows.

The complete per-wake order is:

1. From the wake entries, select each target's coding-agent overlay. Capture every named live due member and the Director read-only; classify a capture failure as loss-tolerant `unknown` and the Director as unavailable.
2. Query `monitor stall pending` for durable context, but emit nothing yet.
3. For each named ordinary member, classify content and call `stall observe`; collect finished member IDs without sending them.
4. For `action = ping`, invoke `member ping` immediately regardless of Director classification, then record success/failure. Do not preview a newly pending failure yet.
5. For `action = escalate`, leave the row pending for the aggregate gate.
6. After every ordinary-member action is complete, re-capture/reclassify the Director and immediately submit that capture to `stall observe --director-gate`. This is the final read-only/state-observation command and supersedes the initial Director classification. Save its raw token only when the broker resolves `finished` or `stalled`.
7. If the resolved Director gate is `finished` or `stalled`, immediately call `monitor report-batch --director-gate-token <returned-token>` exactly once with the collected finished IDs, even when there is no known new entry, so an open delivery can recover. No tool call may intervene; the report command atomically consumes the gate, and its aggregate preview is the only possible following action. For `awaiting_user`, `working`, unresolved/too-early `stall_candidate` (`unknown`), or capture failure, no token exists: discard/defer the ephemeral finished-ID collection, leave durable escalation/delivery rows untouched, and make no Director-targeting call.
8. End the wake. `message send`, per-member stall reports, and any second Director preview are forbidden.

This ordering makes the freshness premise broker-enforceable against direct, stale, replayed, restarted, or concurrent `report-batch` calls: the authoritative Director capture issues a 30-second single-use token immediately before the only command that may preview, with no intervening tool call. Multiple pre-existing pending rows, a new same-wake ping failure, multiple finished members, and an older failed delivery still produce at most one `send_inline_preview` invocation on tmux/herdr in that wake.

The Director then runs the normal facilitation and recovery protocol. It alone may:

- decide whether a `finished` member has outstanding assigned work;
- send or resend task instructions;
- choose recovery, replacement, or user escalation;
- interpret an un-acked delivery in light of the assignment ledger.

The existing Director pre-nudge capture gate continues to apply to any later Director-initiated member action. The monitor's just-taken capture is authoritative for its one fixed-action ping, but does not waive the Director's fresh-capture requirement for a later message, broadcast, or ping.

### 7. Monitoring-member command boundary

The monitoring member's on-wake command set expands from two command families to four:

| Command | Allowed use |
|---|---|
| `cafleet member capture` | Read each named due pane plus the Director at `--lines 120 --json`, including the runtime-computed capture timestamp/fingerprint. |
| `cafleet monitor stall` | Observe/reset durable ordinary episodes, resolve the no-ping Director gate, list pending reports, and record ordinary ping outcome. It never inserts a message or keystrokes a pane. |
| `cafleet monitor report-batch` | Once at the end of every safe wake and only with the just-issued consumable Director-gate token, reconcile the one open delivery by ACK, apply backpressure or create one aggregate, enforce pending/awaiting-ACK retry timing, and record at most one same-ID preview outcome. |
| `cafleet member ping` | Once per stall episode, only for a confidently stalled ordinary member, using the fixed inbox-poll action. |

`member ping` itself is reused unchanged: it has no text parameter, is Esc-safeguarded on both multiplexer backends, and is covered by the coding-agent permission presets. The new monitor commands accept only typed state-machine/member-ID inputs and cannot carry task text. During a synchronized wake, the monitor must not call `message send`, `message broadcast`, or `member prompt`; `report-batch` is its sole Director-delivery path. No caller-identity flag or new authorization schema is introduced; the ordinary-member target guard plus the monitoring role protocol form the authority boundary, consistent with the existing Director/member command model.

### 8. Wake-nudge contract

The byte-identical `send_wake_trigger` payload in `tmux.py` and `herdr.py` must encode the full action policy because it reopens the monitoring member's turn. Its normative content must:

- name the synchronized due batch and the Director, including a sanitized `coding_agent` for every due entry and the standing Director descriptor;
- state that `unacked` is context on an already-due member and never an action trigger;
- preserve capture-only classification, precedence, backend cue use, the `awaiting_user` safety bar, and the distinct quiet `stall_candidate` vs affirmative/ambiguous `working` inputs;
- require one direct `member ping` for the first confident stalled classification of an ordinary member;
- require target-overlay selection from each rendered coding-agent field, then initial Director capture and pending-list processing before ordinary observations; any final Director classification other than `finished` or resolved `stalled` suppresses only the aggregate report, not an eligible ordinary-member ping;
- require the durable `observe → claim → ping → ping-result` ordering, durable stall cadence, and recovery of `nudge_claimed` / sticky `escalation_pending` state after restart;
- require validated capture-time spacing independently of scheduler dispatch spacing, so delayed wake processing and duplicate `observe` calls cannot create confidence;
- require one Director escalation when the next synchronized capture is unchanged, or immediately when the ping fails;
- require lifecycle cleanup outside the due payload, a loss-tolerant capture-failed `unknown` path, loss-tolerant ping-result ordering, and explicit soft-deregistration row deletion, while preserving pending reports until committed;
- state that pending escalation creates no wake while the Director is unavailable, remains sticky, and resumes after the explicit re-enable/live-rebind precondition on a normal synchronized wake;
- require non-pending episode reset on changed/non-stalled/unreadable/member-exit state and forbid repeat ping/escalation within the episode;
- require collection of every pending/new escalation and finished observation, an authoritative Director recapture after all ordinary actions, and exactly one safe end-of-wake `report-batch` using its fresh one-use token even when no new entry is known, with no intervening command or second Director preview; state that a surviving open aggregate leaves escalations pending and finished IDs ephemeral;
- require same-message-ID preview recovery and require the Director to use `message show --full` before processing/ACKing an aggregate that may exceed the preview cap;
- prohibit task text and every direct ordinary-member action other than the fixed poll ping;
- keep `finished` judgment with the Director.

The example in `skills/cafleet/roles/monitor.md` and the exact payload tests are updated together. tmux and herdr payloads remain byte-identical.

### 9. Unchanged surfaces

- `cafleet member ping` CLI arguments, output, errors, and JSON shape.
- The `send_poll_trigger` Esc-first keystroke sequence.
- Broker message status and monitor-status/WebUI response fields.
- `monitor_runtime` schema and every cadence/default interval.
- The monitoring member's first-in/first-out lifecycle.
- README thin-surface content.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first ordering is required by `.claude/rules/documentation-maintenance.md`.

### Step 1: Documentation and role contracts

- [x] Update `docs/concepts/monitoring.md`: preserve synchronized batches; redefine `unacked` as annotation-only; document durable cadence/episodes, candidate-vs-working safety, fixed direct ping, sticky pending reports, lifecycle reset, consumable Director gate, and completion ownership <!-- completed: 2026-07-29T01:47 -->
- [x] Update `SPEC.md` data model and migration contract with five `monitor_config` columns, the one-open-per-fleet `monitor_report_delivery` invariants, consumable `monitor_director_gate` proof, explicit teardown/soft-deregistration cleanup, and revision `0005_add_monitor_stall_episode_state.py` <!-- completed: 2026-07-29T01:47 -->
- [x] Update `SPEC.md` broker/CLI contracts for mode-exact capture timestamps/fingerprints, ordinary/director-gate/loss-tolerant paths, `monitor stall observe`/`ping-result`/`pending`, and token-gated atomic fixed `monitor report-batch`, including backpressure, ACK-only completion, retry thresholds, exact payloads/outputs, validations, idempotency, and errors <!-- completed: 2026-07-29T01:47 -->
- [x] Update `SPEC.md` monitor-loop/multiplexer contracts: durable stall cadence, lifecycle cleanup, annotation-only unacked ordering, internal coding-agent metadata, removal of both process maps, unchanged batch commit gate, and exact wake payload <!-- completed: 2026-07-29T01:47 -->
- [x] Update `docs/spec/data-model.md` and `docs/spec/cli-options.md` for the five episode fields, delivery queue, consumable Director-gate row/token, timestamped ANSI/no-ANSI capture identity, complete `monitor stall` group, token-required report-batch recovery/output, and aggregate full-body consumption <!-- completed: 2026-07-29T01:47 -->
- [x] Update `docs/spec/multiplexer-backends.md`: distinguish trigger reasons from the `unacked` hint; document sanitized target coding-agent entries, reuse of the unchanged Esc-first poll primitive, one aggregate Director preview per wake, and retry of the same message ID <!-- completed: 2026-07-29T01:47 -->
- [x] Update `skills/cafleet/roles/monitor.md`: per-target overlay selection, JSON capture/fingerprint and loss-tolerant unknown flow, `stall_candidate`/`working` split, pending-list-first collection, direct ping/result ordering, final Director `finished|stalled` gate observation, immediate one-use-token aggregate/retry command, restart recovery, lifecycle/sticky-report rules, and finished behavior <!-- completed: 2026-07-29T01:47 -->
- [x] Update `skills/cafleet/reference/coding-agent/_template.md` and the claude/codex/opencode overlays with target-backend affirmative `working`, quiet `stall_candidate`, and ambiguity cues; bind the note to monitor classification and add per-backend role-contract fixtures <!-- completed: 2026-07-29T01:47 -->
- [x] Update `skills/cafleet/reference/supervision.md`: make monitor-direct fixed poll the first confident-candidate stall action; route durable pending escalation to the Director; define the Director gate as target-specific; retain the Director's gate for every later Director action <!-- completed: 2026-07-29T01:47 -->
- [x] Update `skills/cafleet/roles/director.md`, `skills/cafleet/roles/member.md`, `skills/cafleet/SKILL.md`, and `.claude/rules/bash-tool.md` for the monitoring member's sole fixed-ping exception, Director `message show --full`/message-ID dedup/ACK protocol for aggregate previews, and preserved arbitrary-instruction prohibition/completion ownership <!-- completed: 2026-07-29T01:47 -->
- [x] Verify `README.md`, broker/API reference pages unrelated to the new broker functions, and WebUI response contracts require no change because the thin surface, HTTP shapes, and monitor-status member shape are unchanged <!-- completed: 2026-07-29T01:47 -->

### Step 2: Durable schema and broker state machine

- [x] Add dispatch time, candidate-observation time, fingerprint, episode state, and reason with check constraints to `MonitorConfig`, including `"clear"` defaults and paired-null candidate fields <!-- completed: 2026-07-29T07:51 -->
- [x] Add Alembic revision `0005_add_monitor_stall_episode_state.py` with episode backfill/constraints plus `monitor_report_delivery` and `monitor_director_gate`, their checks/indexes/FKs, teardown and Director-lifecycle cleanup, and reversible downgrade; update canonical DDL assertions <!-- completed: 2026-07-29T07:51 -->
- [x] Add capture-boundary UTC `captured_at` plus `content_sha256 = sha256(emitted_content.encode("utf-8"))` for both ANSI modes without storing capture content <!-- completed: 2026-07-29T07:51 -->
- [x] Implement atomic `observe_stall_episode`: live guard for readable states, loss-tolerant capture-failed `unknown`, ordinary vs Director-gate modes, validated capture-time full-interval enforcement, Director `stalled` with permanent `action=none`, issue/invalidate 30-second one-use gate tokens, unconditional non-actionable `working`, ordinary baseline/claim/restart behavior, and exact results <!-- completed: 2026-07-29T07:51 -->
- [x] Implement loss-tolerant/idempotent ping-result, pending-list, token-validating/consuming transactional `report_monitor_batch`, one-open backpressure, ACK reconciliation, pending/awaiting-ACK retry eligibility, and preview-outcome recording; retries always reuse the open message ID <!-- completed: 2026-07-29T07:51 -->
- [x] Implement shared lifecycle cleanup: disable/dead/pending-pane reset, `nudge_claimed`→sticky interrupted escalation, pending preservation, explicit soft-deregistration deletion, and recovery reseed <!-- completed: 2026-07-29T07:51 -->

### Step 3: Monitor-loop trigger policy

- [x] In `monitor/loop.py`, remove `_last_unacked_wake_at` and `_last_stall_check_at` plus their per-run clear/re-fire/commit paths <!-- completed: 2026-07-29T08:10 -->
- [x] Replace `_flag_unacked_due` with an annotation-only helper over existing due rows, retain the one-member-interval staleness threshold, run native detection first, and append `unacked` last without changing batched cadence commits <!-- completed: 2026-07-29T08:10 -->
- [x] Read durable `last_stall_check_at`, select/validate `MemberPlacement.coding_agent` for every target and the Director descriptor, batch-clear non-live lifecycle state before due filtering, keep pending escalation non-triggering, and persist dispatch cadences only after the single watcher wake succeeds <!-- completed: 2026-07-29T08:10 -->
- [x] Rewrite loop tests for annotation-only unacked, durable interval across immediate restart, coding-agent propagation/invalid-value fail-closed, success/failure cadence commits, no pending-only wake while the Director is disabled/dead, later-recovery delivery, non-live cleanup/pending preservation, recovery reseed, and one synchronized wake <!-- completed: 2026-07-29T08:10 -->

### Step 4: CLI state surface

- [x] Add `captured_at`/`content_sha256` to capture JSON in documented key order, stamp/hash at the capture boundary under both `--ansi` and `--no-ansi`, and leave text output byte-identical <!-- completed: 2026-07-29T08:26 -->
- [x] Add `monitor stall observe` with typed classification (`awaiting_user|unknown|finished|working|stall_candidate`), mutually exclusive ordinary stall-check/Director-gate modes, paired timestamp/hash validation, exact resolved/token output, live readable targeting, and loss-tolerant no-capture `unknown` targeting <!-- completed: 2026-07-29T08:26 -->
- [x] Add `cafleet monitor stall ping-result --success|--failure` and token-required `cafleet monitor report-batch [--finished-member-id]...`; neither accepts arbitrary text, report-batch consumes the fresh safe gate, enforces backpressure/ACK-only completion, and records at most one same-ID aggregate preview outcome <!-- completed: 2026-07-29T08:26 -->
- [x] Add `cafleet monitor stall pending` with stable text/JSON ordering so queued reports remain discoverable when their members are absent from the due batch <!-- completed: 2026-07-29T08:26 -->
- [x] Add CLI tests for classifications, hash modes, mode/argument exclusivity, ordinary/Director target guards and no-ping Director results, safe/unsafe gate-token output, missing/malformed/expired/replayed token rejection, pending visibility, batch sorting/deduplication/empty behavior, aggregate-name sanitization, one-open backpressure, pending vs awaiting-ACK retry timing, success-without-ACK/ACK recovery, state conflicts, retry idempotency, and no-arbitrary-body rejection <!-- completed: 2026-07-29T08:26 -->

### Step 5: Wake contract

- [x] Update the multiplexer protocol and both `send_wake_trigger` payloads byte-identically with sanitized per-due/Director coding-agent fields, pending-list-first order, initial/final Director captures, finished-or-confidently-stalled token gate, target-overlay cues, candidate-vs-working rules, broker-enforced observation spacing, durable observe/ping/result/backpressured-batch-retry flow, lifecycle ownership, full-body Director consumption, and completion ownership <!-- completed: 2026-07-29T08:41 -->
- [x] Update exact-payload/cross-backend tests to pin coding-agent/name sanitization and parity, mixed-backend cue selection, active-working non-action, one ordinary candidate ping, Director-awaiting ordinary ping, Director becoming working/awaiting during processing, full-spacing Director stalled gate with no ping claim, fresh-token pass-through with no intervening command, disabled/dead-Director deferral, at-most-one aggregate preview invocation, restart/cadence rules, and the arbitrary-instruction prohibition <!-- completed: 2026-07-29T08:41 -->

### Step 6: End-to-end verification

- [ ] Add model/migration/broker tests for backfill/explicit cleanup, delivery state/one-open constraints, gate issuance/invalidation/expiry/atomic consumption and concurrent replay, capture-timestamp validation, ordinary/Director full-interval observations, explicit-working non-action, no Director ping claim, scan-live→capture-fails, claim→death→ping-failure ordering, sticky pending, backpressured aggregate creation, same-ID preview retry/ACK reconciliation, and non-pending reset <!-- completed: -->
- [ ] Add integration scenarios for immediate loop restart before interval, disable/dead→recover reseed, scan-live→capture-fails, ordinary ping while Director awaits, two full-spacing unchanged Director captures→fresh-token one aggregate preview, active/ambiguous Director→no token/preview, stale/direct/restarted/concurrent report-batch rejection, repeated known preview failures with the same finished member→one open row/no duplicate entry, queued ping failure behind open message→later aggregate after ACK, successful preview→no ACK→later finished/unacked Director→interval-stale same-ID recovery, over-cap full-body consumption before ACK, and preview-failure same-ID recovery <!-- completed: -->
- [ ] Run unchanged `member ping` safety tests plus full test/lint/typecheck; verify no independent `unacked`, process-local cadence/episode state, working-hash promotion, erasable pending escalation, or blanket “members never ping” residue <!-- completed: -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-07-28 | Initial draft |
| 2026-07-28 | User clarification: made stall-episode state durable across monitoring-member restart with `monitor_config` fields, an Alembic migration, atomic broker transitions, and a narrow `monitor stall` CLI surface |
| 2026-07-28 | Reviewer round 2: defined ANSI-mode hashing; split `stall_candidate` from `working`; persisted stall cadence; assigned disabled/dead cleanup; made pending escalation sticky/discoverable; and made Director prompt suppression target-specific |
| 2026-07-28 | Reviewer round 3: made ping results survive liveness loss; separated validated capture time from dispatch cadence; specified per-backend work cues; corrected soft-deregistration cleanup; and defined Director recovery as the precondition for deferred-report delivery |
| 2026-07-28 | Reviewer round 4: added loss-tolerant capture-failure reconciliation, carried sanitized target coding-agent metadata in wake entries, and batched every Director report into one final atomic message/preview per wake |
| 2026-07-28 | Reviewer round 5: added durable same-message preview recovery, required full aggregate retrieval by message ID before Director action/ACK, and made the final pre-preview Director recapture authoritative |
| 2026-07-28 | Reviewer round 6: applied one-open-batch backpressure, made ACK the sole delivery completion signal with interval-throttled re-preview, and added a durable no-ping Director stall gate |
| 2026-07-28 | Reviewer round 7: distinguished same-turn durable escalation queuing from backpressured aggregate delivery, guaranteed later inclusion after ACK on a safe normal wake, and made the final Director result enforceable through a fresh consumable broker token |
