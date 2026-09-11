# Simplifying Code and Documentation and Improving Failure Consistency

**Status**: Complete
**Progress**: 36/36 tasks complete
**Last Updated**: 2026-09-06

**Minimal-change override (user)**: In response to feedback about the volume of added tests, keep changes to the minimum necessary. Do not add tests for behavior already covered by existing tests. Limit new tests to representative cases needed for public behavior changes, actual defects, or prevention of serious data-loss regressions. Do not exhaustively cover internal events or implementation order, expand combinations merely to increase case counts, or add production hooks/APIs solely for that purpose. Each TDD phase need not add new tests; proceed with a rationale when existing tests suffice. This instruction overrides the older exhaustive requirements below. Previously added duplicate tests, tests coupled to internal structure, and verification-only mechanisms may also be reduced, but do not delete assertions or combine tests into giant tests merely to reduce their number. Start with a bounded cleanup of the Step 10 additions; do not introduce a large verification framework or redesign the whole system. Finish the bounded fixes and checks already in progress and avoid unnecessary reruns.

**Post-completion translation (user)**: Translate this design doc into English after all Implementation tasks and required checks/reviews are complete. Do not translate in parallel with implementation. Do not replace or translate the whole document at once. Work by chapter or section, splitting long sections further, and inspect each diff for preservation of meaning, specification values, code/commands, links/anchors, tables, checkboxes, and completion timestamps before proceeding. Record completed and remaining portions, and do not declare the request complete until consistency has been checked across the whole document. The earlier instruction excluding the design doc from commits remains in effect.

**Execution scope override (user)**: The user has stated that additional E2E/browser verification is unnecessary. Omit Step 4 UI verification, later ad hoc E2E checks, final Phase D, and Step 12 browser QA; required verification is limited to automated tests, lint, builds, and code review. This is an omission at the user's request, not an E2E success. The design doc and evidence also remain excluded from commits.

## Overview

Organize the 20 findings from the code and documentation review into incremental improvements, prioritizing four reproduced defects. Make external contracts explicit, then simplify process, DB, pane, and installation failure handling, Rust internal boundaries, WebUI fetching, and documentation references. This document was originally a Japanese design proposal; it does not itself authorize implementation, commits, or release.

**Execution-history status**: The Execution history below preserves the resolved implementation process. Intermediate red/staged/arbitration-pending states were resolved by each step's completion record or superseded by a user override. They are not current unresolved requests. See Step 12 for final verification. A fresh Reviewer's independent review completed with approved (doc) at 2026-09-06T04:13:52Z (CAFleet message 1421, Reviewer 225).

## Success Criteria

- [x] F01–F04 each have a regression test that detects the pre-fix failure and passes after the fix.
- [x] Two active monitors cannot be registered in the same fleet; records and panes are preserved when migration fails on existing duplicates.
- [x] Cleanup is verified on important representative pane, runtime, and assets failure paths; cleanup errors do not replace the primary error.
- [x] CLI/HTTP shapes, ordering, nulls, exit codes, and error strings outside the intentional-change list pass contract tests.
- [x] Each WebUI member-history request retrieves at most 201 rows, distinguishes initial errors from empty history, and prevents responses from a previous fleet from updating the new screen.
- [x] Required local references in installed skills resolve without a checkout in both Markdown links and inline code, and bootstrap examples contain four identity labels and references to real roles.
- [x] SPEC retains its existing structure and the contract details needed for reimplementation; the identified specification drift and supervision/recovery contradictions are resolved.
- [x] All work and verification in the F01–F20 mapping are complete, and Rust tests/lint, admin tests/lint/build, and the docs build pass.

---

## Background

The review targets Rust CAFleet 0.24.4, DB schema 7, the React admin, `docs/`, `SPEC.md`, `skills/`, and project rules. The 480 passing Rust tests and successful admin build/lint at review time are the existing baseline, not verification of this document's future implementation. F01–F03 were reproduced with temporary Rust probes; F04 was reproduced with a probe extracting the actual TS grouping function. F04 has not been reproduced in a browser.

Detailed review input is in [review-brief](.notes/review-brief.md). The specification and verification conditions below are sufficient for implementation without opening that file. The Director's answers to pre-drafting questions (CAFleet message 894) are adopted as design decisions for this proposal, not additional user approval.

| Existing design doc | Boundary with this document |
|---|---|
| `0000170-docs-skills-affirmative-simplification` (Complete) | Preserve documentation ownership, affirmative wording, self-contained overlays, and Required-reading for distinct audiences. Add the newly identified defects, broken installed links, and ordinary user-feedback handling. |
| `0000174-fix-codex-monitor-lifecycle` (Complete) | Preserve the contract that Codex retains a managed execution session and checks startup logs before sending `monitor live`. Releasing resources after Rust runtime failures is a separate fix. |
| `0000175-notification-failure-reporting-and-command-isolation` (Complete) | Preserve notification failures after message persistence, the no-resend rule, isolated one-shot CLI calls, and asynchronous handoff. Moving the notifier must not turn these failures into success. |
| `0000175-reliable-pane-message-notifications` | The directory was inspected, but it contains only `.prompts/` and no `design-doc.md`. Do not treat an unestablished specification as an implemented contract. |

---

## Specification

### 1. Scope and Change Control

For each step, update the relevant documents before changing regression tests and code. Document order is `docs/`, any required `README.md`, `SPEC.md`, relevant `skills/`, then rules. `docs/` is the primary explanatory source; `SPEC.md` is the authoritative contract for reimplementation and must not become a link-only summary. Modify the repository's `skills/`, not installation destinations such as `~/.codex/skills`.

| ID | Problem and main evidence | Step |
|---|---|---|
| F01 High, reproduced | `cli/system.rs::SystemRunner::run` waits for child exit before reading pipes | 1 |
| F02 High, reproduced | Monitor registration races between the precheck in `cli/member.rs` and `broker/members.rs::register_member` | 2 |
| F03 High, reproduced | A run failure in `multiplexer/herdr.rs::split_window` loses the created pane; CLI compensation also uses send_exit | 3 |
| F04 Medium, algorithm reproduced | `broker/queries.rs::list_timeline` returns summaries, and `Timeline.tsx` counts fictitious recipients/ACKs | 4 |
| F05 | `get_member` converts a 13-element tuple to JSON and reparses it; consumers use expect/clone | 5 |
| F06 | `webui/mod.rs` depends on `cli::helpers::CliNotifier` | 5 |
| F07 | helpers/doctor/setup duplicate schema/assets diagnosis and connections | 6 |
| F08 | Unused correlated aggregates in `roster_rows`; per-ID SQL in `get_member_names` | 6 |
| F09 | MemberDetail fetches all history before truncating to 201 rows | 7 |
| F10 | Long create argument lists and manual rollback, long write transactions during bootstrap, early return after monitor claim | 3, 8 |
| F11 | capture/monitor scan duplicate ANSI handling, timestamps, and SHA256 generation | 8 |
| F12 | Global fleetId, URL, and App state overlap; multiple load owners | 9 |
| F13 | Timeline/MemberDetail catch paths display initial failures as empty results | 9 |
| F14 | `assets.rs` deletes the existing tree before copying; a healthy record for the same version survives failure | 10 |
| F15 | Duplication across SPEC/cli-options and drift in ordering, activity, joins, distribution, and formatter descriptions | 6, 11 |
| F16 | Recovery's idle/unread heuristic contradicts supervision's capture-based decision | 11 |
| F17 | 24 links valid in a checkout and four overlay inline-code references escape the installed skills tree | 11 |
| F18 | A 243-line quickstart and an incomplete monitor prompt | 11 |
| F19 | Repeated coordination protocol and requiring users to write COMMENT syntax | 11 |
| F20 | Prose-pinning tests, insufficient frontend tests, duplicate CI work, and incorrect mise argument examples | 4, 9, 12 |

Changes to retention, history deletion, automatic notification retries, new management CLIs, wholesale module reorganization, a large fetch library, and general CLI panic/BrokenPipe handling are out of scope. Broadcast groups truncated by timeline row limits are separate from F04; API pagination semantics do not change.

| Intentional external behavior change | Change |
|---|---|
| F01 | Valid output exceeding pipe capacity no longer causes a false timeout. |
| F02 | The DB rejects a racing second monitor, and migration stops with diagnostics on existing duplicates. |
| F03/F10 | Creation failure kills owned panes and reports cleanup failures. Bootstrap subprocesses have deadlines, and runtime initialization failure releases the claim. |
| F04 | Timeline excludes summaries and displays/aggregates deliveries only. |
| F09 | inbox/sent HTTP accepts an optional `limit`. Omission and CLI history retain existing behavior. |
| F12/F13 | Prevent duplicate fetching and stale responses; show communication failures explicitly. |
| F14 | An incomplete assets replacement is not treated as healthy and can be recovered by setup. |
| F15 boundary correction | Clamp only negative `idle` caused by future activity timestamps to 0. This is an additional correction separate from the four reproduced bugs. |
| F19 | The Director converts ordinary user feedback into internal COMMENT markers. Users need not enter the syntax. |

### 2. Process Output, Deadlines, and Cleanup (F01)

Preserve `CommandRunner::run(argv, timeout_secs) -> Result<String, RunError>`. With `Some(timeout)`, drain stdout/stderr concurrently immediately after spawn, checking child liveness and the deadline in the same loop. Use safe nonblocking FD/poll APIs from the existing `nix` dependency, adding the required `fs`/`poll` features. Do not use unbounded joins on reader threads; manage the owned pipes' completion within the call.

1. Own the directly spawned child and both pipes, and set both FDs to nonblocking. Kill/reap the child even if that setup fails.
2. On each iteration, check the deadline and `try_wait`, then read both streams fairly. Limit each stream to 64KiB per iteration so continuous output on one side cannot starve the other or the deadline. Poll for the shorter of 20ms and the remaining deadline.
3. Complete when the child has exited and both streams reach EOF. On success, return stdout as lossy UTF-8 as before; on nonzero exit, pass stderr as lossy UTF-8 to `Failed`. Do not introduce output truncation.
4. At the deadline, kill and wait for the direct child, close both read FDs, and return `Timeout`. Apply the same rule if descendants retain the pipes after the child exits and EOF has not arrived by the deadline. Stopping all descendants is not guaranteed.
5. On read/poll/try_wait errors, also kill/reap and release FDs. Preserve the primary cause and attach kill/reap failures as secondary diagnostics. Retry signal interruptions after recalculating the deadline. `None` may use the existing unlimited `wait_with_output` path.

The deadline bounds observation and the start of termination, not a strict wall-clock return time when the OS is unresponsive. Add an integration test in which a normal child's 1-second timeout returns within 5 seconds. Verify successful 1MiB stdout, 1MiB stderr, and simultaneous output with generous deadlines, checking for missing output, zombies, and surviving readers. Also verify actual sleep timeouts, nonzero exit, empty output, FD setup/read failures, and a fixture retaining pipes after child exit.

### 3. Active Monitor Uniqueness and Migration (F02)

Add the following in the migration after schema 7 (currently `V8__unique_active_monitor.sql`). If the head has advanced when work begins, use the next sequence number.

```sql
CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';
```

Match the existing `active_monitor_member_id` predicate. Ordinary members and deregistered monitors are exempt; fleets are independent. Enforce the constraint on status/card/fleet_id updates as well as INSERT. Keep root Director cards free of the monitor marker, and preserve Director precedence in display-kind resolution.

Retain the CLI precheck for error ordering and early diagnostics, but rely on the DB for correctness. Use an `IMMEDIATE` registration transaction to serialize writers, checking for an existing ID inside the transaction for monitors too. Convert only this unique-constraint violation to `ActiveMonitorExists { fleet_id, member_id }`; preserve the CLI string `fleet {fleet_id} already has an active monitor member (member {existing})` and exit 1. Do not disguise other SQL errors as this conflict. The losing registration must not proceed to registration, placement, or pane creation.

Run pre-migration diagnosis in the DB half of `setup`, after rejecting unversioned/too-new schemas and before migration. If members already exist, use the same predicate to retrieve duplicate fleets and member IDs in ascending order. On duplicates, display `active monitor duplicates prevent migration: fleet <id>: members <ids>; ...`, fail the DB half, and leave records, panes, and schema history unchanged. A fresh DB has nothing to diagnose and migrates normally.

Constraint creation also rejects races between diagnosis and DDL. Apply pending migrations in a refinery grouped transaction; guarantee and test restoration of the old schema/history on failure. If duplicates can be retrieved after constraint creation fails, show the same diagnostic; otherwise preserve the original migration error. Do not rewrite existing migration bodies or choose a survivor automatically.

Recover existing duplicates as follows.

1. Stop new registrations connecting to the affected DB, and have the operator choose the monitor to retain from the displayed IDs.
2. Run the immediately preceding release that supports the old schema from another path, using the same `CAFLEET_DATABASE_URL` and configuration directory. Do not use the new CLI's `member delete`, which the behind-schema guard blocks.
3. Because setup preserves independent DB/assets execution, the failed new setup may already have updated assets alone. If so, use the old binary's `setup` to restore the same target backends' old assets, then call the old binary's `member delete <surplus-id>` one call at a time. This path works because the DB remains at the old head.
4. After resolving duplicates, rerun the new binary's `setup`. Do not automatically downgrade after schema migration succeeds.

Test a deterministic interleaving: both connections observe None in their precheck, A registers and commits, then B registers. Test migration on an empty DB, a populated DB without duplicates, a duplicate DB, an already-applied rerun, and a race after diagnosis. Assert post-failure schema/history/row counts and zero pane operations.

### 4. Pane Ownership and Creation Compensation (F03 and Part of F10)

The backend owns a new pane until `split_window` returns successfully, then transfers ownership to the CLI creation flow. Herdr creates a guard immediately after obtaining the ID from the split response and calls `kill_pane(id, true)` if the subsequent run fails. If closing the pane fails, return the ID and primary run error as well. If it is impossible to confirm whether an external operation created a pane, such as a malformed split response before the ID is known, diagnose an unknown ID and unconfirmed compensation; never guess and close another pane.

| Failure point | Guard owner and compensation order | Final DB action |
|---|---|---|
| Member registration fails | Broker rolls back registration transaction; no pane guard | Add no rows |
| Member placeholder expansion fails | CLI registration guard only; no pane created | Deregister member and remove placement under existing rules |
| Member Herdr run fails | Backend pane guard attempts kill → returns error to CLI → CLI registration guard deregisters | Deregister/remove placement |
| Member placement update returns Err/None | After handling update SQL failure, CLI pane guard kills → registration guard deregisters | Deregister/remove placement |
| Fleet callback fails before creating a pane | No pane guard → broker rolls back bootstrap transaction | Undo fleet, Director, monitor, and placement additions |
| Herdr run fails/times out inside fleet callback (known ID) | Backend pane guard attempts kill → callback returns Err → broker rolls back bootstrap transaction | Same as above |
| Placement INSERT/commit fails after fleet callback succeeds | CLI retains pane guard received on callback success → broker rolls back transaction and returns Err → CLI kills | Same as above |
| Fleet callback fails/times out before obtaining an ID | Backend returns unconfirmed compensation → broker rolls back bootstrap transaction; do not guess a pane to kill | Same as above, retaining the unconfirmed pane diagnostic |

Return backend-attempted compensation to the CLI as `PaneCleanup::Attempted { pane_id, error: Option<_> }`, and unknown IDs as `PaneCleanup::Unknown`. For Attempted, the CLI must not kill again; it performs the remaining DB compensation. Immediately after `split_window` succeeds, the callback transfers ownership to the CLI guard and returns the ID, with no intervening fallible operation. On post-callback fleet failures, close the broker transaction before pane compensation; do not introduce a new broker API requiring pane-first cleanup.

Follow the table's compensation order, attempting the remaining cleanup even when an earlier action fails. Remove `send_exit` from creation rollback because it does not guarantee shell/pane termination. Append cleanup failures after the primary error as `cleanup failed for pane <id>: <detail>`, `cleanup failed for member <id>: <detail>`, or `cleanup failed for fleet <id> transaction: <detail>`; do not falsely report success or complete rollback. Explicitly diagnose transaction rollback failure too, without claiming an uncertain DB state was fully undone. Disarm guards through explicit `finish`/`rollback` to prevent duplicate kill/deregister; Drop is only the last defense for unfinished work. Once member placement is finalized or the fleet commit succeeds, disarm all creation guards before calling the existing `emit`. Preserve existing text/JSON output boundaries; do not introduce new exit-code or diagnostic contracts for stdout failures.

Use FakeRunner/FakeMux and transaction-boundary event fixtures to verify every sequence in the table, list → split (ID obtained) → run failure → close, close/rollback failure diagnostics, and no kill on success. Do not mix changes to normal `member delete` or notification keystrokes into this work.

### 5. Timeline and History HTTP (F04 and F09)

Add `g.type='unicast'` to the timeline SQL WHERE clause, preserving fleet scope through the owner member and `ORDER BY status_timestamp DESC, message_id DESC LIMIT 200`. Retain summaries in the DB, message show, and broadcast results. Fix only the error of treating summaries as deliveries; distinguish the API's delivery ordering from the UI's existing created_at display order.

Declare the actual wire `type` in TS, expressing `FormattedMessage` as a union of `type: 'unicast'` (non-null recipient ID/name) and `type: 'broadcast_summary'` (both null). Narrow timeline, inbox, and sent to the delivery type for rendering. Move `groupMessages` to `timeline.ts` as a pure function, explicitly testing non-null `origin_message_id`. Defensively exclude any summaries mixed into its input from aggregation.

With two pending deliveries and one completed summary, assert two recipients and zero ACKs separately in SQL, HTTP, and TS. ACKing one delivery gives two recipients/one ACK; ACKing all gives two ACKs. Also verify ordinary unicast and empty arrays. When a row limit cuts off a broadcast, aggregates describe the fetched deliveries, not a claimed completion rate for all deliveries. Explain this limitation in ReactionBar; do not change the limit to operate on groups.

| HTTP item | Contract |
|---|---|
| Targets | `GET /api/members/{member_id}/inbox`, `/sent` |
| New query | Optional `limit`, a decimal integer from 1–1000. Empty, 0, negative, fractional, nonnumeric, overflow, or duplicate values return 422 `{"detail":"limit must be an integer between 1 and 1000"}`. Unknown queries remain ignored. |
| Omitted | Existing behavior returns all rows. Equivalent CLI inbox/sent retrieval is unchanged. |
| Ordering | `status_timestamp DESC, message_id DESC`, deterministic even for equal timestamps. “Latest” here means status-update order. |
| Envelope | Preserve `{"messages":[...]}` and each row's key order, names, and nulls. Do not add cursor/has_more/total. |
| Validation order | Preserve existing Path extraction, fleet header (400), fleet existence (404), and member membership (404), then validate limit. SQL errors retain the existing 500 detail format. |
| Broker | Pass `HistoryOptions { limit: Option<usize> }` to inbox/sent retrieval. Apply a SQL-bound LIMIT only when specified; do not substitute post-fetch truncation. |
| WebUI | Use `?limit=201` on both endpoints. Display each endpoint's latest 200 rows and the existing omission notice if a 201st row exists. No omission notice for 200 or fewer rows. |

This design bounds rows fetched by explicit-limit HTTP and the WebUI. It does not claim to bound the unlimited HTTP retained for compatibility or storage volume. Verify 0/200/201/many rows, equal timestamps, other fleets, deregistered members, every invalid limit, and compatibility when omission returns more than 201 rows. Check the query plan with existing indexes; do not add unnecessary indexes.

### 6. Rust Internal Types and Dependency Direction (F05–F08 and F15)

Convert SQL results once into small internal types, then to wire output in CLI/HTTP presenters. Migrate callers in order: member → placement → message → monitor. Short-lived compatibility adapters are allowed but must be removed after all consumers migrate. Do not build a generic repository framework.

| Internal type | Main fields and constraints |
|---|---|
| `MemberRecord` | `member_id: i64`, `fleet_id: i64`, `name/description/registered_at: String`, `status: MemberStatus`, `kind: MemberKind`, `skills: Vec<Value>`, `placement: Option<Placement>`. Allow Value only for free-form skill elements. |
| `Placement` | `backend`, `mux_session`, `mux_window_id`, `coding_agent`, `created_at`: String; `mux_pane_id: Option<String>`. Distinguish missing placement from an unfinalized pane. |
| `MessageRecord` | IDs: i64; `to_member_id/origin_message_id: Option<i64>`, `kind: MessageKind`, `status: MessageStatus`, `created_at/status_timestamp/text: String`. Preserve null summary recipients. |
| `MonitorRuntime` | `fleet_id: i64`, `pid: Option<i64>`, `started_at: Option<String>`, `last_tick_at: Option<String>`, `last_wake_at: Option<String>`, `wake_requested_at: Option<String>`, `tick_seconds: i64`, `wake_interval_seconds: Option<i64>`. Field meanings are below. |
| `CaptureSnapshot` | `content: String`, `captured_at: String`, `content_sha256: String`. The caller adds member/pane metadata. |
| `Diagnosis` | `SchemaState` (Missing/Unversioned/Behind/Head/Ahead/Unreachable) and `AssetState` per backend/path. Contains no display wording. |

| MonitorRuntime field | Meaning of NULL/0 and lifecycle |
|---|---|
| `fleet_id` | Non-null fleet key. Represent an absent row as `Option<MonitorRuntime>::None`, never substitute 0. |
| `pid` | NULL means no claim is held. Record the direct loop PID on claim; restore NULL on normal clear. Do not newly interpret 0 as unclaimed; preserve the existing process-probe decision. |
| `started_at` | NULL means no claim start time. Update on claim/reclaim; NULL on normal clear. |
| `last_tick_at` | NULL or unparseable means the heartbeat is not fresh. Update on claim/tick; NULL on normal clear. |
| `last_wake_at` | NULL means no successful wake is recorded. Update only on successful scheduled/forced delivery; preserve across clear/reclaim. If NULL, cadence falls back to started_at. |
| `wake_requested_at` | NULL means no forced-wake request. Coalesce by overwriting the request; set NULL on successful delivery or reclaim. Normal clear alone does not erase it. |
| `tick_seconds` | Non-null, DB default 5, and positive for valid CLI input. 0 is invalid, not disablement, and is rejected by existing CLI validation. Preserve on normal clear. |
| `wake_interval_seconds` | NULL means a row predating V5 has not been reclaimed and has no recorded value; do not convert it to 0. 0 disables scheduled wakes (forced wakes remain possible); positive values are intervals in seconds. Record on claim/reclaim/PATCH; preserve on normal clear. |

Preserve the invariant that wake_interval_seconds is Some after the loop claims. Do not confuse nullable DB fields with stopped-state HTTP projection; the existing presenter contract handles hidden timestamps and retained intervals. Migration regressions must verify V5-origin NULL, 0, positive values, clear/reclaim, and forced wakes.

Internal enums convert invalid DB values to `InvalidStoredValue`, without leaking panics to the boundary. Preserve the existing card_skills fallback from malformed JSON/missing skills to an empty array. Introducing types must not change timestamp parsing/formatting. Existing JSON key ordering depends on `serde_json` preserve_order; presenters must construct keys in explicit order.

Move `SystemRunner`, `SystemProbe`, and the real-process notifier adapter below CLI into `runtime/`. Dependencies flow CLI/HTTP → runtime adapter → broker notification traits and multiplexer/coding_agent. The broker knows neither process spawning nor HTTP, and webui must not import cli. Preserve `NotificationAttempt`, persisted message IDs, transport diagnostics, and the CLI partial-failure output from design 0000175.

Introduce only domain-error variants needed for actual branching, such as `ActiveMonitorExists`, `MemberNotFound`, `FleetNotFound`, and `InvalidStoredValue`. Map them to existing CLI Usage/App/Value exit codes/strings and existing HTTP status/detail. Replace panics on missing sender/name with integrity errors returning 500; do not fabricate names.

Shared diagnosis receives the Connection used by the invocation and preserves schema → assets guard order. Doctor treats connection failure as a report value and displays all sections; ordinary CLI stops at the required guard. Setup retains its contract to attempt the assets half even after DB failure and rediagnoses on the same connection after DB creation/migration. Do not insist on “one connection” to the point of forbidding reconnection after an initial connection failure. HTTP continues connecting per blocking handler.

Make `list_roster` a lean query without message activity calculation, preserving the EXISTS condition including message holders, kind, placement, and member_id order. `list_members` uses the query with activity. `get_member_names` deduplicates IDs and fetches them with bound IN queries in batches of at most 500 IDs. Empty input executes zero SQL statements; unknown IDs are absent from the map; deregistered members are included; return a BTreeMap. SQL count is at most `ceil(unique_ids/500)`; concatenate only placeholders.

Adopt the following resolutions of specification drift, updating behavior-pinning tests and SPEC together.

| Item | Adopted contract |
|---|---|
| Fleet order | Current Rust behavior: `created_at DESC, fleet_id DESC`. Correct SPEC's ASC tie-breaker. |
| `last_sent` | `MAX(created_at)` across all messages with the matching sender, including summaries. Do not switch to status-update time. |
| `last_recv` | `MAX(created_at)` for matching-owner unicast messages. |
| `last_ack` | `MAX(status_timestamp)` for matching-owner, unicast, completed messages. |
| `idle` | Select the maximum of the three timestamps and apply existing parsing rules; all-null/unparseable yields null. Correct only the calculated result to `max(0, seconds(now-latest))`. |
| Timeline scope | Join through the owner member. Correct the WebUI spec's sender-join description. |
| Assets distribution | Unpack skills/presets bundled in the Rust binary offline. Update the obsolete release-archive fetching description. |
| Spawn formatter | Explain the Rust mini formatter's four placeholders and literal-brace escaping. Do not promise all Python `str.format` features. |

### 7. Shared Lifecycle and Capture Processing (F10 and F11)

Collect current arguments into `MemberCreateOptions` and `FleetCreateOptions`. Follow each CLI's existing validation order; do not force together fleet/member preconditions that look alike but differ. Share spawn preparation, pane ownership guards, and construction of compensation results.

Fleet bootstrap retains atomic commit of fleet/Director/monitor. Do not move external side effects outside the transaction and expose unfinished fleets. Before opening the DB transaction, complete prompt reading, backend validation, and as much cwd/env/argv preparation as possible. Leave only ID-dependent expansion and pane spawning in the callback, passing one monotonic 30-second deadline across all callback subprocesses. Bound each Herdr list/split/run/resize and tmux split/layout call by the remaining time rather than resetting 30 seconds each time. Compensation kill has a separate 5-second budget; retain the existing 5-second DB busy timeout. Do not guarantee DB lock waits or an unresponsive OS finish within 30 seconds. Use §4's failure-path table for timeout guard ownership and compensation order. In particular, a run failure inside the callback attempts backend kill before DB rollback; placement/commit failure after callback success rolls back the DB before CLI kill.

The monitor loop owns a `MonitorLease` immediately after a successful claim. Retain each successfully registered signal handle and unregister them on every path: subsequent signal registration failure, startup write/flush failure, tick failure, normal stop, and ownership loss. Preserve the conditional `clear_monitor_runtime(fleet_id, pid)` update so a new owner with another PID is not cleared. If both the main operation and clear fail, retain the first and append the second; if only clear fails after main success, exit with the clear failure. Existing stale reclaim is the recovery path after SIGKILL/crash.

Centralize capture in `CaptureSnapshot::from_raw(raw, ansi, now)`. With ansi=false, use content after existing strip_ansi processing, including CR normalization; with true, use the received raw content. Convert the final content's UTF-8 bytes to lowercase SHA256 hex. Preserve timestamp format, lines input, nulls on scan errors, and text output. Build scan text headings only in the text presenter, not in the JSON path.

Inject signal registration/Write/flush/clear to verify every early return. For capture, verify matching content and hashes between capture/scan for ANSI, CRLF, standalone CR, Unicode, and empty input; eliminate timestamp differences with a fixed clock.

### 8. WebUI Fleet and Asynchronous State (F12 and F13)

Make the URL route the sole source of the selected fleet, encapsulating its ID in `createFleetClient(fleetId)`. `listFleets` uses an unscoped client; all other operations receive an explicit fleet client through props/context. Remove global `setFleetId`; the fleet-selection handler only navigates.

App owns the route and fleet-existence check; Dashboard owns member roster, monitor, and refreshKey; Timeline/MemberDetail each own their history fetches. Remove App's initialMembers prefetch so route effects and Dashboard mount do not both fetch the same roster. Key Dashboard by fleet ID and the member panel by member ID. Give each resource one initial-fetch path (aborted attempts in development StrictMode are allowed).

Fetching uses an AbortController and generation ID, aborting on unmount and fleet/member changes. Only the current generation may apply responses or release inFlight in finally. If refreshKey increases during an update, coalesce it into one pending latest refresh and refetch after completion. Failure/abort must not leave a guard that blocks polling. Preserve 5-second polling independent of visibility, manual Refresh, and refresh after successful send.

| Resource state | Display/refetch |
|---|---|
| Initial loading | Skeleton |
| Success, empty | No messages |
| Initial error | Error text and Retry button; do not show an empty result. |
| Refresh in progress | Retain existing data for the same fleet/member. |
| Refresh error | Existing data, update-failure notice, and Retry. |
| Abort/old generation | Do not update the display. |

Preserve existing navigation to the fleet list for invalid/nonexistent/deleted fleets, deep links, and Back/Forward. Distinguish communication failure from nonexistence; retain the route and show a retryable screen. Verify that old fleet names/recipients do not appear in a new fleet's send form.

At the user's request, frontend verification uses only the existing Vitest Node environment, with no DOM dependency. Test the real FleetClient, shared route/ID parser, history, and asynchronous state handling in the production resource object subscribed to by real useResource. Use controlled promises to verify generation/stop/restart, reverse-order resolve/reject/finally that ignores abort, initial error → retry, data retention on refresh error, independent resources, one pending coalesced refresh, and snapshot/subscribe/unsubscribe. Do not execute React render/click/unmount/navigation. Key/cleanup wiring, one initial-roster fetch path, 5-second hidden-tab polling, manual/post-send refresh, error/Retry/empty display, deep links/BackForward, fleet/member selection, and isolation of old mutation callbacks/recipients/drafts are checked only through code review, type checking, lint, and build. Do not claim Node tests provide equivalent DOM verification. UI implementation requirements remain; do not migrate fetch libraries or introduce a test-only controller/React imitation.

### 9. Assets Staging, Swapping, and Recovery (F14)

Replace assets per backend while preserving the successful installation and its record. Manage both skills, the preset, and removal of legacy cafleet-research in one install plan. Do not roll back successful updates to other backends. Do not describe this as a single atomic commit spanning multiple directories and SQLite.

Create stage/backup entries with unique hidden names in each target's parent directory. Do not rename the whole parent containing unrelated skills. If a target is a symlink, back up the entry itself without following and deleting its destination. Serialize concurrent setup through OS advisory locks acquired in normalized target-path order, including when different configuration paths point to the same physical skills tree. Locks release on process exit; do not rely solely on PID files.

1. Write all files to staging and validate the embedded manifest and each skill entrypoint. Step 11's development generation/check verifies the required runtime-reference closure in a real installed tree, ensuring the distributed embedded bytes are correct. Do not add a Markdown parser to the normal installer; retain its manifest/file-set/hash validation. Copy/validation failure removes only staging, leaving current files and asset_installs unchanged.
2. Under the assets identity path, record transaction ID, phase, each target/stage/backup path and previous existence state, the previous asset_installs row (null if absent), and the new version/manifest in `.cafleet-install-journal.json`. Update the journal through temp → rename, flushing/syncing before changes.
3. Rename each target to backup, then stage to target. Journal the state before and after every operation. Treat cafleet-research removal as a move to backup too. On intermediate failure, restore old entries in reverse order.
4. After all swaps succeed, commit `record_asset_install` in a transaction. On failure, restore files and the old record. On success, durably mark the journal `committed`.
5. After committed, delete backup/stage and delete the journal last. Diagnose cleanup failure separately from installation success and allow the next setup to retry it.

| Interruption/failure point | Diagnosis and recovery |
|---|---|
| Before journal | Current installation unchanged. Clean unreferenced staging at the next setup. |
| Journal exists, not committed | Guard/doctor diagnose an incomplete install. After acquiring locks, setup checks the record against actual target/backup existence and rolls back to old entries and the old DB record. |
| After DB commit, before journal committed | Use the same uncommitted recovery to restore old files and the old DB record. Do not infer success from matching versions alone. |
| Committed, cleanup in progress | Validate the new manifest and DB record and continue cleanup; do not roll back. |
| Restore failure/corrupt journal | Preserve journal and backups, report incomplete state, and stop. Do not falsely report a healthy old version. |

Guard/doctor inspect the resolved installation's journal and, if unfinished, report `incomplete assets install at <path>; run 'cafleet setup' to recover`. Preserve existing healthy-path output. Make each restore/delete idempotent so another interruption during recovery can resume from the same journal. Do not delete backups before restoration or confirmation of committed state. State that power-loss durability depends on filesystem sync/rename guarantees.

Use fixtures to verify stage writes, first/second skill replacement, preset replacement, record failure, rollback failure, restart at each journal phase, concurrent setup, presets on another filesystem, symlinks, and legacy research removal, including same-version reinstalls. Assert file digests, records, journals, and remaining backups for each case; never modify real user configuration during tests.

### 10. Documentation Ownership and Self-Contained Installation (F15–F19)

Implementation boundary under the minimal-change policy: Markdown parsing, reference closure, and manifest classification checks belong in development docs-generate/docs-check, with the parser only in dev-dependencies. The check uses the real installer in isolation to validate files/anchors and bootstrap; Step 12 connects it to CI. Normal binary StageValidate retains existing embedded manifest/file-set/hash validation; normal setup does not gain a contract to reparse documentation links. This boundary supersedes earlier proposals below and in Step 10 to add Markdown-reference checking to a private stage validator. To ensure distribution of verified bytes, docs-check is a required pre-release gate.

| Content | Authoritative source and treatment elsewhere |
|---|---|
| Mechanical CLI argument tables | Generate argument names/types/defaults/requiredness from clap definitions into bounded blocks within existing cli-options and SPEC sections. Keep explanations, error precedence, and text layout as handwritten contracts. |
| Mechanical schema tables | Generate columns/null/default/index metadata from SQLite metadata in a fixture DB with all migrations applied. Do not hand-edit migration history. |
| Human usage explanations | `docs/`. Preserve README as the entry point, SPEC as detailed contracts, and skills as required runtime procedures. |
| Shared member/Director procedures | Centralize broker operations in existing `cafleet/SKILL.md`, write rules in `base-dir.md`, backend differences in `coding-agent-overlays.md`, and Director decisions in `supervision.md`. Roles retain required reading order and their differences. |
| Required runtime references | Of the 29 references below, bundle required L01–L28 (24 links + four inline-code references) under `skills/cafleet/reference/runtime/`. L29 is an optional public SPEC link. `docs/docs/` remains authoritative; runtime is explicitly generated and must not be hand-edited. |

Fix replacements for all 29 references as follows. L01–L24 are the 24 Markdown links in the initial review, L25–L28 the four overlay inline-code references, and L29 the generated data-model's SPEC reference. For L01–L28, sources are relative to `skills/cafleet/`, old targets to `docs/docs/`, and replacements to installed `cafleet/reference/`. Only L29 lists its source/old target relative to the repository. Emit actual Markdown hrefs as relative paths from source to replacement. Line numbers are for matching as of 2026-09-05; track the same references if they move. L01–L28 are 28 required references used for CLI arguments, output, notifications, and lifecycle decisions; L29 is one optional reference for reimplementers.

| ID | Source | Old target | Classification | Installed-tree replacement |
|---|---|---|---|---|
| L01 | `SKILL.md:43` | `spec/cli-options.md` | Required | `runtime/spec/cli-options.md` |
| L02 | `SKILL.md:81` | `spec/cli-options.md#positional-subject-ids` | Required | `runtime/spec/cli-options.md#positional-subject-ids` |
| L03 | `SKILL.md:81` | `spec/cli-options.md#permissionsallow-coverage` | Required | `runtime/spec/cli-options.md#permissionsallow-coverage` |
| L04 | `SKILL.md:137` | `spec/multiplexer-backends.md#push-notifications` | Required | `runtime/spec/multiplexer-backends.md#push-notifications` |
| L05 | `reference/cli.md:3` | `spec/cli-options.md` | Required | `runtime/spec/cli-options.md` |
| L06 | `reference/cli.md:22` | `spec/cli-options.md#message-body-truncation` | Required | `runtime/spec/cli-options.md#message-body-truncation` |
| L07 | `reference/cli.md:28` | `spec/cli-options.md#output-shapes` | Required | `runtime/spec/cli-options.md#output-shapes` |
| L08 | `reference/cli.md:69` | `spec/data-model.md#broadcast-grouping` | Required | `runtime/spec/data-model.md#broadcast-grouping` |
| L09 | `reference/cli.md:69` | `spec/message-envelope.md` | Required | `runtime/spec/message-envelope.md` |
| L10 | `reference/cli.md:116` | `spec/cli-options.md#error-messages` | Required | `runtime/spec/cli-options.md#error-messages` |
| L11 | `reference/cli.md:125` | `spec/cli-options.md#fleet-delete` | Required | `runtime/spec/cli-options.md#fleet-delete` |
| L12 | `reference/cli.md:148` | `spec/cli-options.md#error-messages` | Required | `runtime/spec/cli-options.md#error-messages` |
| L13 | `reference/director.md:26` | `spec/cli-options.md#member-create` | Required | `runtime/spec/cli-options.md#member-create` |
| L14 | `reference/director.md:30` | `spec/cli-options.md#error-messages` | Required | `runtime/spec/cli-options.md#error-messages` |
| L15 | `reference/director.md:32` | `spec/cli-options.md#error-messages` | Required | `runtime/spec/cli-options.md#error-messages` |
| L16 | `reference/director.md:34` | `spec/cli-options.md#member-create` | Required | `runtime/spec/cli-options.md#member-create` |
| L17 | `reference/director.md:118` | `concepts/member-lifecycle.md` | Required | `runtime/concepts/member-lifecycle.md` |
| L18 | `reference/director.md:146` | `spec/cli-options.md#member-delete` | Required | `runtime/spec/cli-options.md#member-delete` |
| L19 | `reference/director.md:179` | `spec/cli-options.md#member-prompt` | Required | `runtime/spec/cli-options.md#member-prompt` |
| L20 | `reference/director.md:192` | `spec/multiplexer-backends.md#esc-safeguard` | Required | `runtime/spec/multiplexer-backends.md#esc-safeguard` |
| L21 | `reference/prompt-routing.md:48` | `spec/cli-options.md#member-prompt` | Required | `runtime/spec/cli-options.md#member-prompt` |
| L22 | `reference/recovery.md:38` | `concepts/monitoring.md` | Required | `runtime/concepts/monitoring.md` |
| L23 | `reference/supervision.md:15` | `spec/multiplexer-backends.md#push-notifications` | Required | `runtime/spec/multiplexer-backends.md#push-notifications` |
| L24 | `reference/supervision.md:122` | `spec/cli-options.md#member-create` | Required | `runtime/spec/cli-options.md#member-create` |
| L25 | `reference/coding-agent-overlays.md:28` (claude Note) | `concepts/monitoring.md` (inline code) | Required | `runtime/concepts/monitoring.md` (convert to Markdown link) |
| L26 | `reference/coding-agent-overlays.md:73` (codex Note) | `concepts/monitoring.md` (inline code) | Required | `runtime/concepts/monitoring.md` (convert to Markdown link) |
| L27 | `reference/coding-agent-overlays.md:117` (opencode Note) | `concepts/monitoring.md` (inline code) | Required | `runtime/concepts/monitoring.md` (convert to Markdown link) |
| L28 | `reference/coding-agent-overlays.md:163` (Template Note) | `concepts/monitoring.md` (inline code) | Required | `runtime/concepts/monitoring.md` (convert to Markdown link) |
| L29 | `docs/docs/spec/data-model.md:9` | `SPEC.md` (inline code) | Optional; human reimplementation specification | `https://github.com/himkt/cafleet/blob/main/SPEC.md` (public Markdown link) |

Record the developer reference added in Step 10 separately in the manifest as additional optional reference R01: convert `../contributing.md#assets-installer-tests` in `docs/docs/spec/cli-options.md` to the public link `https://github.com/himkt/cafleet/blob/main/docs/docs/contributing.md`. It supplements installer-fixture authoring and verification limits; operational setup/recovery contracts remain in cli-options, so do not bundle all of contributing. Retain original L01–L29 as the tracked set of 28 required references plus one optional SPEC reference, and do not leave this additional optional link unclassified. Match the URL and optional-reference explanation in the authoritative docs and generated copy; no network fetch is required. Keep the bundle at 12 pages. Record R02 (data-model → contributing Rust-boundary explanation) and R03 (webui-api → use-the-webui usage examples) in the manifest under the same optional-supplement rule. Their public URLs are `https://github.com/himkt/cafleet/blob/main/docs/docs/contributing.md` and `https://github.com/himkt/cafleet/blob/main/docs/docs/how-to/use-the-webui.md`, respectively; retain required typed/HTTP/retry contracts in the original bundled pages.

Starting from six direct targets (`spec/cli-options.md`, `spec/multiplexer-backends.md`, `spec/data-model.md`, `spec/message-envelope.md`, `concepts/member-lifecycle.md`, `concepts/monitoring.md`), transitively bundle local Markdown link targets. Generate whole pages preserving body, headings, and anchors so offline references also work beyond the first linked page. The current closure contains the following 12 files, each generated from `docs/docs/<path>` to `skills/cafleet/reference/runtime/<path>`.

| Bundled path | Bundled path |
|---|---|
| `spec/cli-options.md` | `spec/multiplexer-backends.md` |
| `spec/data-model.md` | `spec/message-envelope.md` |
| `spec/coding-agent-backends.md` | `spec/webui-api.md` |
| `concepts/member-lifecycle.md` | `concepts/monitoring.md` |
| `concepts/coding-agents.md` | `concepts/storage.md` |
| `how-to/mixed-backend-team.md` | `quickstart.md` |

Update in this order: docs → required SPEC synchronization → runtime generation → referring skills. Runtime generation preserves the relative directory structure within pages, includes local dependencies such as Markdown/images, and checks existence and anchors. Store generated files in the repository and include them in existing embedded skills. Do not maintain handwritten checkout copies or separate contract descriptions. References remain on demand; do not add all 12 pages to each role's startup Required-reading.

Place the generation/check manifest at `cafleet/tests/fixtures/runtime-reference-manifest.json`. Record L01–L29 source root (skills/docs), path, heading path, original target/anchor, original notation (link/inline-code), occurrence order for identical references, classification, replacement, and the set of 12 generated paths above. Do not use line numbers as identity. A Markdown parser detects ordinary/reference links and local images outside code fences; after migration, match these replacements against the manifest. Fail check on new unclassified local references outside skills, disappearance/changed duplicate counts of known references, additions/removals in the generated closure, or content drift in generated files. Compare sets, classifications, and occurrence counts rather than asserting only the total of 29 (28 required, one optional). Match L29's expected URL in both authoritative docs and generated runtime/spec/data-model.md; do not assign its generated copy another ID and count it twice. If documentation cleanup removes links, update and review the manifest and design mapping in the same change, rather than silently dropping classifications. Apply the same bundling rule to new required references; changes to optional references require a reason that they are human-facing supplements and a concrete public URL in the reviewed manifest. Existing out-of-docs reference L29 requires the public-link conversion below. After known replacements, fail generation if any unclassified local reference outside docs remains; never guess a checkout fallback.

For L25–L28, replace inline code `docs/docs/concepts/monitoring.md` in each backend's and the Template's load-bearing Note with a real Markdown link labeled “Monitoring” and href `runtime/concepts/monitoring.md`. Retain a link in all four Notes to preserve self-contained overlays. The monitoring page is already in the bundled closure, so the generated-page count stays 12.

Besides Markdown links, extract static document paths from inline code in handwritten skills, the 12 authoritative docs pages, and generated runtime text. Checkout-only references such as paths beginning `docs/docs/` or repository `SPEC.md` fail check as missed link conversions. A path instructing readers to open a local document must resolve inside the installed tree or become a manifest-declared optional public link. Distinguish user output destinations in code examples and task paths containing placeholders from references, recording exclusion reasons in detection-rule fixtures. Mutate L25–L28 back to their original inline code separately at all four locations and assert failure; after replacement, assert the bundled page opens from all four headings (three backends and Template). This includes required references not written as Markdown links in acceptance criteria.

Before generating runtime, update the opening of `docs/docs/spec/data-model.md` for L29 to a public Markdown link with this meaning: “The complete column-level DDL contract is documented in [Repository specification](https://github.com/himkt/cafleet/blob/main/SPEC.md), an optional reference for reimplementers.” SPEC remains the authoritative reimplementation contract. Fleet operations, communication, and monitoring handled by installed skills can use the CLI and bundled contract pages; complete DDL for reimplementing CAFleet or manually manipulating DB schema is not required runtime reading, so do not bundle all of SPEC offline. These operational procedures remain usable even if the public URL is unavailable. Fixtures match the optional link's expected URL, optional labeling, and location without requiring network access during offline verification.

L29's change leaves the generated closure at 12 pages. Fixtures reinserting inline-code `SPEC.md` separately in authoritative docs and generated output must fail as unconverted references; the public-link form must pass. Verify this alongside the four L25–L28 mutations. Inline-code diagnostics identify the original page and generated destination; do not exclude all generated files. Distinguish examples that do not instruct reading a document, such as preset installation/source paths, with reasoned fixtures; do not apply the same exclusion to authoritative reference L29.

Run generation explicitly and review its output; normal builds must not rewrite files. CI check mode detects differences from generated output. If generated blocks appear in two places, use the same input. Replace tests of exact full prose/headings with generated-block, CLI-parser, wire-output, schema, and link-resolution tests. Do not erase verification of important existing obligations or error ordering by deletion.

Make supervision's capture-to-action decision the Director-side authority, removing recovery prose that treats idle >5 minutes plus unread messages as evidence of a stall. Recovery retains only its specific procedures for checking dead panes/disconnections, explicitly requested shell dispatch, respawn, and monitor-first shutdown. Capture failure alone does not prove pane disappearance. Use doctor for current connection state and member list/show to compare registered backend/pane information, not as proof of a physical pane's existence. If existing CLI commands cannot establish the fact, retain unknown and do not ping/delete/respawn on that evidence alone. Only when the unconfirmed fact blocks continued work does the Director ask the user to check the target pane's state. Do not add a pane-list API or raw mux operations for this documentation cleanup.

| State | Director | Monitor member |
|---|---|---|
| working | Defer send/ping | Treat as working |
| awaiting_user | Do not guess an answer to the pane's prompt; defer this round's send. Explicit questions sent by the member follow ordinary Director relay. | Treat as waiting for user |
| finished | Resume through a fresh capture gate if assigned work remains; otherwise normal idle | Follow the existing role's quiet criteria |
| stall_candidate | Resume after confirming quiet from identical captures across consecutive facilitation turns | Confirm quiet from identical SHA across consecutive wakes; ping an ordinary member once per quiet period |
| unknown/dead pane | Investigate cause and recover, without pinging | Report event to Director |

Preserve the distinction that a monitor pinging the Director requires unacked >0 in addition to quiet confirmation. For the Director, unread counts and monitor events are advisory and do not independently authorize ping. Sending keystrokes makes a capture stale; the next send requires a fresh capture. Preserve existing exceptions for immediate replies to reply-soliciting messages and shell dispatch explicitly requested by a member. Also preserve backend-specific cues, Codex managed-session startup confirmation/restart, isolated one-shot calls, and no resend after notification failure following persistence.

Limit the quickstart's main path to one short example: setup → environment check → fleet bootstrap → member creation/communication → shutdown. Link to backend configuration details. Remove claims such as “one-screen” that do not match its actual length. Generate the published monitor prompt from the same content as the executable canonical fixture, without omitting the following.

```text
ROLE DEFINITION: Open <そのbackendのインストール済みcafleet skill絶対path>/roles/monitor.md BEFORE any other action.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
CODING AGENT: {coding_agent}
```

This shows required fields, not a finished copyable example. Actual examples fill absolute paths using backend configuration-resolution rules and include the role-required `BASE` and load-bearing reading instructions. Substitute only the four identity placeholders at spawn time; escape other literal braces. Preserve `fleet create --monitor-file` → ready/loop startup → startup-log confirmation → `monitor live` → ordinary-member authorization in both explanation and fixture. Tests expand the fixture using the three backends' actual staged-install paths and verify role existence and all four identity values.

Skills link verification starts from both skills and the preset unpacked by the real installer into an isolated HOME equivalent. Follow the manifest's 28 required references, including four inline-code-to-link conversions, and their generated closure's files/anchors; match optional L29's public URL to verify all 29 references. Also detect unconverted static inline-code references in authoritative docs and generated output. Do not search through absolute checkout paths or fallbacks; required outbound links fail. After sharing procedures, retain every role's required-reading table and each overlay section's self-contained instructions, avoiding link cycles that obscure reading responsibilities.

The Director records ordinary user change requests without changing their meaning in a user-relay COMMENT marker, then passes them to the Drafter through existing `ready (doc)`. Ask specific questions only about ambiguities. Continue accepting markers written directly by users. Preserve the inter-member verb/pointer format, marker resolution, and the rule forbidding unresolved markers in Approved documents.

### 11. Verification and CI (F20)

Run the relevant regression/contract tests at each step and one final overall gate. Do not record implementation tests as complete during drafting of this design. Use small exact-byte snapshots for representative CLI JSON, wire shape/null/order and status assertions for HTTP, and semantic assertions for internal pure functions; do not add long full-prose snapshots.

| Gate | Completion condition |
|---|---|
| Rust | `env CI=true mise //cafleet:test` and `mise //cafleet:lint` pass. Add necessary regressions to the existing 480 tests; during cleanup, map old assertions to replacement checks to prevent coverage loss. |
| Admin | New `mise //admin:test`, existing `mise //admin:lint`, and `mise //admin:build` pass. Use fake timers/controlled promises for reproducibility. |
| Docs | `mise //:docs-build`, generated-block checks, and staged-skills link/anchor/bootstrap fixture verification pass. |
| Query | Use query plans/traces to verify bounded-history row counts, name-lookup counts, and absence of activity aggregates in lean roster. Do not impose fixed millisecond performance thresholds. |
| Actual UI | In an isolated DB, browser-check broadcast ACK display, initial failure/recovery, fleet switching, and the 200-row omission notice. Record this as implementation-completion evidence separate from F04's algorithm probe. |

Add admin lint to the CI lint job. Preserve Rust tasks' `//admin:build` dependency to ensure embedded dist generation in a fresh clone, removing the duplicate explicit admin build in the same job. After confirming clippy checks the same features/target, remove the additional cargo-check equivalent in CI while retaining the manual typecheck task. Do not remove necessary builds across separate jobs as duplicates.

Verify argv forwarding on the current toolchain before updating the mise `--nocapture` example. Do not republish current `mise //cafleet:test my_test_name -- --nocapture` as a working example. During implementation, define explicit task-level test/harness argument forwarding or adopt measured separator placement. Publish only a command whose one-test filter and nocapture have both been verified with a test fixture in `.claude/rules/commands.md`. Verification operations use full-path mise tasks.

---

## Implementation

> Record the completion time in the same edit that checks a task: `- [x] 作業 <!-- completed: 2026-09-05T14:30 -->`. Track the 36 tasks below. Each step proceeds through document updates, targeted regressions, implementation, and targeted verification.

### Step 1: Fix Pipe Blocking and Timeouts

- [x] Update process-contract documentation first under §2, then add regression fixtures for large stdout/stderr, both streams, real timeouts, and nonzero exit. <!-- completed: 2026-09-05T16:31 -->
- [x] Implement nonblocking draining, deadline checks, and direct-child reaping; verify cleanup including FD setup/IO failure and descendant-held pipes. <!-- completed: 2026-09-05T16:43 -->

### Step 2: Add the Monitor Registration DB Constraint

Verification evidence: [Isolated DB/assets recovery with the old CLI and verified scope](.verification/step2-20260905T0835/report.md).

- [x] Reflect §3's schema/errors/old-version recovery procedure in docs and SPEC; add the unique-index migration and preflight diagnosis. <!-- completed: 2026-09-05T17:25 -->
- [x] Introduce in-transaction duplicate detection and a typed conflict error; verify zero losing-side effects with a deterministic two-connection interleaving. <!-- completed: 2026-09-05T17:25 -->
- [x] Verify grouped migration rollback, existing duplicates, reruns, and recovery fixtures accounting for behind-schema guards and the assets half; update the chain guard. <!-- completed: 2026-09-05T17:46 -->

### Step 3: Unify Pane Creation Ownership and Compensation

Execution history (tester): Additional verification completed. Added 25 tests (13 real CLI creation paths, six shared guards, five backend guard/metadata, one null-name fleet wire test) and strengthened assertions in eight existing tests. `CI=true mise //cafleet:test` passed 591/0 failed; `CI=true mise //cafleet:lint` passed both clippy all-targets -D warnings and fmt check. Evidence is in BASE-local `.cafleet-evidence/tester-step3/full-tests.log` and `lint.log` (local only). No implementation or git operations.

Execution history (tester): Verified reachable §4 failure paths using real CLI helpers, the real Herdr backend, and a shared event sink in a test-only CommandRunner. Asserted complete order for backend close → deregister/rollback, CLI kill → deregister on placement Err/None, and rollback → CLI kill after post-callback placement INSERT/real deferred COMMIT failure. Verified subsequent DB compensation after close failure, primary diagnostics/exit classification/diagnostic order with multiple cleanup failures, and no guessed kill for Unknown. A two-connection BEGIN IMMEDIATE probe confirmed that the write lock cannot be acquired during backend close inside the callback but can during post-callback CLI close. Also verified SQLite automatic rollback through RAISE(ROLLBACK). No duplicate kill/deregister across finish/explicit rollback/Drop, disarm before successful Value return, and typed matching for None/Attempted success/failure/Unknown all passed.

Execution history (tester): Pinned all text/JSON keys, order, and newlines for successful member/fleet CLI output, plus complete broker JSON for null-name fleets and placement=null members. Existing notification-failure exit/persisted-ID/no-resend/broadcast aggregation contracts passed in the full suite. The initial two SQL diagnostic-prefix expectations were corrected as test defects, not implementation defects.

Execution history (tester): Verification limits remain explicit. after_rollback confirms real rollback success, autocommit, actual DB-row removal, and write-lock release before returning a simulated diagnostic; it verifies diagnostic retention and subsequent pane compensation, not a real SQLite/OS rollback failure. Per Director arbitration, removed the unreachable fixture returning RunError::Timeout with timeout=None. Deadline-backed run/split timeout tests, primary timeout retention, and cleanup order remained mandatory carryover in Step 8; Step 3 completion is not evidence that timeouts were verified.

- [x] Update §4's error/cleanup contracts first; add fixtures for Herdr run failure, placement Err/None, and commit failure. <!-- completed: 2026-09-05T18:02 -->
- [x] Implement backend/CLI pane guards and registration compensation; replace send_exit with kill and verify primary errors and cleanup diagnostics. <!-- completed: 2026-09-05T18:09 -->
- [x] Verify normal fleet/member text/JSON compatibility, notification-failure contracts, §4's failure-specific compensation order and ownership transfer, absence of duplicate cleanup, and limits for unknown-ID failures. <!-- completed: 2026-09-05T19:11 -->

### Step 4: Fix Fictitious Timeline ACKs

UI/E2E verification was canceled at the user's request and not performed. [Verifier cancellation record and unverified scope](.verification/step4-20260905T1032/report.md) records that ACK transitions, partial-group explanation, the 200-row cap, and summary/ordinary delivery/empty displays were not demonstrated. No running server/browser session was obtained.

Execution history (director): Canceled UI/E2E verification at the user's request. The Verifier was to remain idle without starting a server/browser or requesting approval. All Director startup requests were canceled; no running session was obtained. Step 4 verification consists of Rust 601/TS 11, type checking, build, and lint success, with no claim of E2E success.

Execution history (programmer): Step 4 Phase B completed. SQL preserves owner-fleet join, descending status_timestamp/message_id order, and the limit, filtering `g.type='unicast'` before the limit. TS makes `FormattedMessage` a type-discriminated union of delivery (`TimelineMessage`) with non-null recipient ID/name and summary with nulls; pure `groupMessages(readonly FormattedMessage[])` moved to timeline.ts. It preserves defensive summary exclusion, explicit origin-null checking, and existing display order by the minimum created_at of fetched deliveries. MemberDetail also narrows to deliveries before applying the existing display cap. For broadcasts even with ACK0, ReactionBar displays “Recipients and ACKs shown may cover only part of this broadcast.” and does not claim an overall completion rate. Using Director-installed Vitest 5.0.0 and its lockfile, `mise //admin:test` runs `tsc -b && vitest run`, discovering admin/tests/**/*.test.ts in Node. Tests/config are included in app/node tsconfig for type checking too. Test contents were unchanged; no React DOM test infrastructure or Step 7 query/201-row retrieval was introduced.

Verification: `CI=true mise //admin:test` 11 passed (including type checking), `CI=true mise //admin:lint` passed, `CI=true mise //cafleet:test` 601 passed/0 failed, and `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt check. Rust tasks' explicit `//admin:build` prerequisite also passed tsc/Vite production build. This includes all six added SQL, four HTTP, and 11 TS cases; regression tests also confirmed preservation of existing summary DB/show/broadcast output. The UI explanation was implemented and build-checked, without any claim of new DOM/browser verification. Evidence is in BASE-local `.cafleet-evidence/programmer-step4/{admin-test,admin-lint,rust-tests,rust-lint}.log` (uncommitted).

Execution history (tester): Phase B required timeline summary exclusion. Ran the ten added Rust tests (six broker SQL, four HTTP): four passed/six were expected red. The six reds reproduced, in both SQL and HTTP, two deliveries plus a completed summary (three rows violating the two-row contract), summary-only input (violating the empty contract), and the 200-row cap before filtering (a summary consumes one slot). Owner-fleet scope was checked with a read fixture deliberately separating sender/recipient from owner; deregistered owner, descending IDs at equal status_timestamp, and difference from created_at order passed. The post-filter cap pins 200 rows from 201 deliveries with only one side of a broadcast remaining. The same fixture checks ACK0 → 1 → 2, summary retention in DB/get_message, and completed summary output from HTTP broadcast. An assertion matching the full summary/null recipient in message show was added to the existing CLI broadcast JSON test and passed. Step 7 history queries/201-row retrieval were unchanged.

Execution history (tester): Awaited the TS Phase B small cycle. Created 11 Vitest cases in `admin/tests/timeline.test.ts`, outside src so the current admin build remained unblocked. They use the pure groupMessages export from `../src/timeline` and the FormattedMessage discriminated union from `../src/types` (no test-side implementation stubs/config changes). They pin mixed-summary and summary-only input, ACK0/1/2, standalone null-origin deliveries, origin=0, separation of distinct origins, ascending order by minimum created_at of fetched deliveries, fetched recipient/ACK counts for partial groups, and input nonmutation. The Programmer was to introduce the minimal Vitest dependency/test task and export/union implementation, activate the 11 cases, and hand them to subsequent Tester verification. TS execution was not claimed at that point.

Execution history (tester): Evidence in BASE-local `.cafleet-evidence/tester-step4/`: `rust-red.log` (four passed/three failed, including one existing test), `http-red.log` (two passed/three failed, including one existing test), and `summary-wire.log` (one strengthened existing test passed). `CI=true mise //cafleet:lint` also passed clippy all-targets -D warnings/fmt check and was saved in `lint.log`. Added 21 tests total (Rust ten/TS 11); no implementation, package/mise configuration, or git operations.

- [x] Reflect §5's delivery-only API and row-cap explanation in docs/SPEC, and add SQL/HTTP summary-exclusion regressions. <!-- completed: 2026-09-05T19:22 -->
- [x] Implement timeline filtering, the TS wire union and pure groupMessages, and a minimal admin test task; verify two deliveries/summary/ACK transitions and partial-group display. <!-- completed: 2026-09-05T19:29 -->

### Step 5: Refine Broker Types and Adapter Boundaries

Execution history (director): Step 5 completed. Committed typed tests separately as 08a9ae4b and unreferenced wrapper/alias removal as b97725e4. Reviewed the seven added cases and existing wire/notification/runtime regressions, confirming 620 passing tests and post-removal all-target lint/fmt/admin build success from evidence. The 620 tests ran from the pre-removal compile; afterward, confirmed that only unreferenced code was removed and all targets compiled. Design doc/evidence were not committed; no E2E/server execution.

Execution history (tester): Typed migration completed. Migrated approximately 190 existing broker calls to finalized record APIs plus explicit presentation at wire assertions, and moved to RuntimeNotifier imports, the skills extractor, HTTP private formatter, and borrowed WakeEntry payload/backend/FakeMux APIs. Preserved existing full JSON/text/notification diagnostics and keystroke-order assertions. Added seven direct tests (six records, one monitor descriptor): all enum values and InvalidStoredValue field/value, member role/free-form skills/placement None versus Some(pane None)/activity/history, persisted MessageRecord and ACK lifecycle for Failed notifications, typed broadcast-summary/delivery metadata, wake/monitor pending counts and identity, and forwarding agent/name/count through the borrowed descriptor. Existing notification tests directly match Skipped/Sent/Failed; runtime tests directly assert Option/null/0/raw timestamps and stopped views; invalid message kind/status checks now assert the domain-error variant.

Execution history (tester): The selected Rust suite passed 620/0 failed (407 unit + 213 selected integration). All-target `--no-run` compilation also passed; after subsequent Programmer cleanup, `CI=true mise //cafleet:lint` passed clippy --all-targets -D warnings and fmt check. The dependent admin tsc/Vite build passed too. Only the ten specified suite targets ran; e2e/cli_server were compile-only, with no browser/server startup. Initial missing arguments in new tests and the expectation that broadcast-summary origin=None were corrected as Tester defects (the existing contract retains the summary's own ID), not production defects.

Execution history (tester): Earlier cleanup resolved compile blockers and unused warnings. Final src/tests searches found zero references to old broker APIs, SendMessageOutcome, CliNotifier, card_skills, formatted_messages, old wake APIs/from_legacy/legacy_value. The Programmer removed production code; the Tester performed no implementation/git operations. Evidence is in BASE-local `.cafleet-evidence/tester-step5/{typed-compile,typed-tests,typed-lint,typed-format}.log` and `old-api-audit.txt` (uncommitted). The 620-case suite used the pre-wrapper-removal compile; all-target lint reconfirmed compile compatibility afterward, so no post-removal full-suite rerun is claimed. Step 5 completion judgment/commit returned to the Director.

Execution history (programmer): Completed removal of all temporary wrappers. Removed old broker APIs/SendMessageOutcome/card_skills, HTTP formatted_messages, WakeEntry::{from_legacy,legacy_value}, old wake payload/backend wrappers, and associated imports/production descriptions. Old API declarations/calls/imports in cafleet/src and cafleet/tests were zero; old names remained only in three member-test assertion diagnostic strings. Member/messaging/query/monitor production code also had zero presentation references. New typed APIs and wire presenters remained, with no production dependency on the deleted wrappers. SHA-256 values of test sections in all eight changed files matched before/after, preserving Tester edits (no whole-tree formatting/git operations). Read all temporary wrappers in the table and historical markers below as subsequently removed.

`CI=true mise //cafleet:lint` exited 0; clippy all-targets -D warnings, fmt check, and the dependent admin build passed. Optional mise cache-write warnings appeared, but the task succeeded with zero Rust warnings. Evidence: `.cafleet-evidence/programmer-step5/{cleanup-lint,cleanup-old-api-audit}.log`. This change only removed unreferenced wrappers; per Director instruction, the Tester's running selected suite was not duplicated. Task 1 was completed and Progress updated to 12/36; task 3 then awaited final Tester/Director suite confirmation. No E2E/cli_server execution or browser/server startup.

Execution history (programmer): Applied a local fix to unblock the Tester. Removed MonitorMux's required old send_wake_trigger method/default bridge and the blanket implementation's old method, making send_wake_entries the required method in every build. Also removed the now-unused monitor cfg(test) Value import and cli/helpers CliNotifier alias. Edited production regions only through apply_patch, with no test-region edits, whole-tree formatting, or git operations. All-target compile-only `CI=true mise //cafleet:test --no-run` exited 0, resolving E0046 (no E2E/server execution). Evidence: `.cafleet-evidence/programmer-step5/bridge-compile.log`. Four migration warnings remained: an unused json import in tmux tests and old card_skills/WakeEntry::legacy_value/formatted_messages. The Tester owned the test import; as directed, the Programmer would remove old wrappers together after Tester completion. No lint success was claimed at this stage.

Execution history (programmer): Wake review fix — monitor_tick borrows broker `WakeTarget` through private `wake_descriptor(&WakeTarget) -> multiplexer::WakeEntry<'_>`, passing it through MonitorMux → Multiplexer → tmux/herdr without JSON conversion. Multiplexer does not import broker and stringifies typed fields only while building the payload. The shared formatter retains coding_agent registration validation, sanitization, empty/singular/plural grammar, the trailing Director segment, and two fixed sentences. tmux best_effort_send(Esc-first=true), herdr Esc → pane run, timeouts/delays/false on failure, and ledger updates only on success are unchanged.

Finalized wake APIs and additional Tester migration targets:

- `multiplexer::WakeEntry<'a> { pub member_id: i64, pub name: &'a str, pub coding_agent: &'a str, pub pending_count: i64 }` (Debug/Clone/Copy/PartialEq/Eq). Broker-to-transport conversion belongs in monitor.
- `multiplexer::build_wake_payload_from_entries(fleet_id: i64, members: &[WakeEntry<'_>], director: &WakeEntry<'_>) -> Result<String, MultiplexerError>`. Old `build_wake_payload` remained only as a cfg(test) wrapper. Payload tests in multiplexer/mod.rs were to migrate to this typed function and typed fixtures.
- `MonitorMux::send_wake_entries`, `Multiplexer::send_wake_entries`, `TmuxMultiplexer::send_wake_entries`, and `HerdrMultiplexer::send_wake_entries` share the signature `(&self, target_pane_id: &str, fleet_id: i64, members: &[WakeEntry<'_>], director: &WakeEntry<'_>) -> Result<bool, MultiplexerError>`. AnyMultiplexer performs typed dispatch only. tmux/herdr tests were to migrate from old send_wake_trigger to the new function, preserving existing full payload/keystroke/failure assertions.
- For the old FakeMux in monitor/mod.rs, a cfg(test)-only `MonitorMux::send_wake_entries` default → old send_wake_trigger(Value) bridge was retained. The real backend's blanket implementation overrides the typed method even in test builds. The Tester was to move FakeMux to send_wake_entries and make recorded values/direct field assertions typed (fixtures may copy borrowed arguments into owned tuples if they must retain them). The Programmer would then delete the cfg(test) old trait method/default bridge, WakeEntry::{from_legacy,legacy_value}, and old payload/backend wrappers, making the typed trait method required in every build. The Tester would also migrate API descriptions in multiplexer/test_support.rs. The Programmer had not edited test bodies. Other broker/presenter/runtime migration targets are in the table and descriptions below.

Execution history (programmer): The selected Rust suite after the wake fix also passed 613/0 failed (including 400 unit tests). Preserved all existing complete wake payload strings, sanitization, empty/singular/plural cases, no send on invalid agents, tmux/herdr keystroke order and best-effort failure, and monitor ledger conditions. `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt check and its dependent admin build. Latest evidence: `.cafleet-evidence/programmer-step5/{wake-tests,wake-lint,wake-format}.log` (uncommitted). No e2e/cli_server execution or browser/server startup. Remaining work then was Tester migration to direct typed assertions → Programmer temporary-wrapper removal; Progress remained 11/36 with Step 5 tasks 1/3 incomplete.

Execution history (programmer): Completed Phase B production typed migration/runtime relocation, ready for the Tester's small cycle. Placed the records/enums below in `broker::records`, validating MemberStatus/MessageKind/MessageStatus at SQL columns. Added `CafleetError::InvalidStoredValue { field: String, value: String }` and preserved that error inside FromSqlConversionFailure through db_err. Required HTTP names use checked lookup returning the same integrity error; implementation does not use catch_unwind. Existing ActiveMonitorExists plus new InvalidStoredValue cover necessary branches, so no unused MemberNotFound/FleetNotFound variants were added. Migrated member/placement → message → monitor production consumers to typed APIs, explicitly invoking `presentation` at CLI/HTTP boundaries. Fleet branches also directly use existing typed FleetRow. Did not change Step 6 aggregate/idle/lookup plans or Step 8 deadlines (roster SQL retained the same aggregates, adding only the card column for MemberRecord skills).

- Finalized types: `broker::records::{MemberRecord, Placement, RegisteredMember, MemberActivity, MessageRecord, MonitorRuntime, MonitorRuntimeView, MonitorMember, WakeTarget, SendOutcome, BroadcastOutcome}`. MemberActivity contains `MemberActivity.member: MemberRecord` and last_sent/last_recv/last_ack/idle. `MemberStatus::{Active,Deregistered}`, `MemberKind::{Director,Monitor,Member}`, `MessageKind::{Unicast,BroadcastSummary}`, and `MessageStatus::{InputRequired,Completed}` implement Copy/PartialEq/Eq and `as_str()`; `TryFrom<&str>` returns InvalidStoredValue. Record fields are public; all records implement Debug/Clone/PartialEq (also Eq where no skills field exists).
- `NotificationAttempt::{Skipped, Sent, Failed { error: String }}`. `SendOutcome { message: MessageRecord, notification: NotificationAttempt }`, `BroadcastOutcome { message: MessageRecord, recipients: usize, delivered: i64 }`. Transport diagnostics stay out of wire output; CLI accesses message_id directly and preserves existing no-resend partial-failure wording.
- Moved real adapters to `runtime::system::{SystemRunner, SystemProbe, find_on_path, read_stdin}` and `runtime::{RuntimeNotifier, resolve_mux}`. Moved old cli/system.rs to runtime/system.rs with colocated tests and unchanged contents; zero webui → cli imports. Only a cfg(test) `CliNotifier` alias in cli/helpers remained for existing unit tests, to be removed after Tester migration.

| Temporary old wrapper | Typed API (existing argument order preserved) | Return value / presenter |
|---|---|---|
| register_member | register_member_record | Result<RegisteredMember>; presentation::registered_member(&record) |
| get_member | get_member_record | Result<Option<MemberRecord>>; presentation::member(&record) |
| update_placement_pane_id | update_placement_record | Result<Option<Placement>>; presentation::placement(&record) |
| list_members | list_member_records | Result<Vec<MemberActivity>>; presentation::member_activity(&record) |
| list_roster | list_roster_records | Result<Vec<MemberRecord>>; presentation::roster_member(&record) |
| send_message | send_message_record | Result<SendOutcome>; presentation::send_outcome(&record) |
| broadcast_message | broadcast_message_record | Result<BroadcastOutcome>; presentation::broadcast_outcome(&record); caller wraps the old wire's one-element array |
| poll_messages | poll_message_records | Result<Vec<MessageRecord>>; presentation::message(&record) |
| ack_message / get_message | ack_message_record / get_message_record | Result<MessageRecord>; presentation::message_envelope(&record) |
| list_inbox / list_sent / list_timeline | list_inbox_records / list_sent_records / list_timeline_records | Result<Vec<MessageRecord>>; presentation::message(&record) |
| read_monitor_runtime | read_monitor_runtime_record | Result<Option<MonitorRuntime>>; presentation::monitor_runtime(&record) |
| monitor_runtime_payload | monitor_runtime_view | Result<MonitorRuntimeView>; presentation::monitor_runtime_view(&record) |
| monitor_members_payload | monitor_member_records | Result<Vec<MonitorMember>>; presentation::monitor_member(&record) |
| list_fleet_wake_targets / fleet_wake_director | list_fleet_wake_target_records / fleet_wake_director_record | Result<Vec<WakeTarget>> / Result<WakeTarget>; presentation::wake_target(&record) |

All Results use CafleetError; presentation functions return Value. HTTP private `formatted_message_records(&Connection, &[MessageRecord]) -> Result<Vec<Value>, CafleetError>` owns name resolution and existing wire-key order. Only `formatted_messages(&Connection, &[Value])` was temporarily retained under cfg(test) for existing direct tests; the Tester was to construct typed rows and migrate to the new function. Typed records do not implement JSON Index. Before deleting the old functions above, old SendMessageOutcome, cfg(test) card_skills/old notifier alias/HTTP old presenter wrapper, the Tester needed to migrate test calls to typed APIs plus explicit presenters where necessary and add direct type-match checks. Wire helpers inside test fixtures could remain as needed, but the Programmer would delete production wrappers after this small cycle.

Verification: Specified `CI=true mise //cafleet:test --lib --test cli_compensation --test cli_fleet --test cli_global --test cli_member --test cli_message --test cli_setup_doctor --test docs_sync --test monitor_uniqueness --test webui_routes` passed 613/0 failed (400 unit, including 15 added cases). `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt check. Both tasks' admin builds (tsc/Vite) passed too. No extra admin test/lint rerun because admin source was unchanged. No e2e/cli_server execution or browser/server startup. Evidence is in BASE-local `.cafleet-evidence/programmer-step5/{selected-tests,lint,format,members-tests,typecheck}.log` (uncommitted). Temporary-wrapper removal and direct typed verification remained, so Step 5 overall and tasks 1/3 were incomplete; task 2's runtime relocation/required error mapping was marked complete with its timestamp.

Execution history (director): Phase B — reviewed 15 tests (seven passed/eight expected red) and committed 40b4d002. The Programmer was to introduce §6's small typed records and necessary domain errors, migrate production consumers in member → placement → message → monitor order, move SystemRunner/SystemProbe/real notifier into runtime, and remove webui → cli dependencies. Put JSON presenters at boundaries; preserve existing wire/text/exit/guard order and notification partial failures. Convert invalid enums/missing names to explicit integrity errors rather than catching panics. Do not mix in Step 6 queries/diagnosis or Step 8 deadlines.

Execution history (director): Migration small cycle — the Programmer must not edit tests. Where return-type changes require migrating existing tests, first move production consumers to typed APIs and keep tests working through temporary compatibility wrappers, then return migration targets and finalized type/presenter signatures to the Tester (do not substitute JSON Index implementations on typed records). Step 5 remains incomplete until Tester migration/direct type matching and subsequent Programmer deletion of old wrappers. Verification uses `CI=true mise //cafleet:test --lib --test cli_compensation --test cli_fleet --test cli_global --test cli_member --test cli_message --test cli_setup_doctor --test docs_sync --test monitor_uniqueness --test webui_routes`, Rust lint, and required admin tests/lint/build. At user direction, omit e2e/cli_server execution and browser/server startup, and do not commit the design doc/evidence.

Execution history (tester): Phase B required conversion to integrity errors. Confirmed seven passed/eight expected red among 15 added tests. Reds were two invalid-kind/status broker get_message cases; two HTTP missing-sender/recipient-name cases (then leaking a blocking-task panic as 500); two HTTP invalid-message/member-status cases (then returning successful 200 rows); one CLI message show invalid-kind/status case (requiring exit 1/no stdout in both text/JSON but then succeeding); and one direct presenter catch_unwind case where a missing name unwound instead of returning Err. A fix merely catching HTTP panics and changing diagnostics must not pass. Reproduced by removing CHECK/FK constraints only in isolated DBs, without modifying implementation/schema or real DBs. Exact new integrity diagnostic wording was unspecified and not pinned; required nonempty detail/existing Error prefix, exit/status/envelope, and no panic. The Programmer was to map InvalidStoredValue and missing-name errors to existing CLI/HTTP boundaries without returning successful rows or panics.

Execution history (tester): Seven compatibility regressions passed. Used the existing extractor to pin malformed/missing/non-array skills JSON → [], and real broker → member display to pin free-form nested/null/bool/number/list elements and order. Distinguished absent placement from an object with pending pane=null, pinning all key ordering. Monitor checks covered absent raw rows, legacy interval NULL, 0/positive values, pid=0 not converted to null, raw timestamps not reformatted, request/last_wake retention after clear, request-only removal after reclaim, and request removal on successful delivery. Pinned raw and stopped projections separately as complete JSON; HTTP absent-row/NULL/0/positive cases were also checked for all keys and scalar/list order. Strengthened existing member show to complete JSON plus newline assertions and passed. Reused existing member/message/monitor wire/role/notification/guard tests, without introducing API strings or internal enum names as new wire output.

Execution history (tester): Verification logs are in BASE-local `.cafleet-evidence/tester-step5/`. `CI=true mise //cafleet:test --lib` had 397 passed/three expected red; `--test webui_routes` 15 passed/four expected red; `--test cli_message` 21 passed/one expected red; `--test cli_member member_show_takes_the_positional_subject` one passed. HTTP used existing tower oneshot only; no E2E/browser/server startup, and no E2E/server suite selected. Existing persisted-ID/raw transport diagnostic/no-resend notification failures and broadcast success passed in the message/unit suites above. `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt check (`lint.log`). Removed the initial lint's test-only unused import. After Phase B, a small cycle still needed direct matches on finalized typed records/presenters/InvalidStoredValue variants and imports after runtime relocation. Diagnosis/activity/query/capture/deadlines remained assigned to later steps.

- [x] Document §6's dependency direction and wire preservation; migrate member/placement/message/monitor sequentially to typed rows and remove JSON-reparsing consumers. <!-- completed: 2026-09-05T20:51 -->
- [x] Move real process/notifier adapters into runtime, remove webui → cli dependencies, and add only necessary domain errors mapped to CLI/HTTP. <!-- completed: 2026-09-05T20:15 -->
- [x] Contract-test JSON key order/null/text/exit codes/guard order, the meaning of each nullable MonitorRuntime field and 0, missing-name 500s, and notification failures after persistence. <!-- completed: 2026-09-05T21:16 -->

### Step 6: Simplify diagnosis and queries

Execution history (director): Step 6 is complete. Test corrections and direct owner-slot verification were committed separately as 9fc4cc8c, and production/docs as 4124bdc3. Reviewed shared facts across diagnosis/guard/doctor/setup, propagation of the same connection, closing the owner slot on failure, observation of real Statements, batching of 500 IDs, the lean roster, and idle clamping. After the 650-test suite passed, the 26 added/modified tests and all-target lint/fmt/admin build passed. The current total is 651, but this is not a report of rerunning all 651 tests. The Director’s final docs build also exited 0. The design doc/evidence are uncommitted; E2E/server checks were not run.

Execution history (tester): Phase C test corrections are complete. Replaced two comparisons in step6_contract_tests.rs with borrowed comparisons using Path::new and removed the unused PathBuf import (no lint suppression or production type changes). Updated the header of the connected tests to reflect their current status. Connected four existing fleet regressions—success, callback run failure, real placement INSERT failure, and real deferred COMMIT failure—directly to the create_with_connection owner slot, adding assertions for Some/TEMP sentinel retention on success and None on broker failure. Preserved the real second-connection probes and complete event ordering for write-lock:false→backend close→rollback inside the callback and rollback→write-lock:true→CLI close after the callback, plus primary errors, cleanup diagnostics, and no double cleanup. One new case checks retention of Some/TEMP sentinel/autocommit and absence of DB side effects when context discovery is rejected before the broker.

Execution history (tester): Targeted verification: fleet creation regressions 8 passed/0 failed; connected Step 6 contract tests 18 passed/0 failed. `CI=true mise //cafleet:lint` passed clippy --all-targets -D warnings/fmt check, as did its dependent admin tsc/vite build. Evidence is under BASE in `.cafleet-evidence/tester-step6/{phase-c-fleet,phase-c-contracts,phase-c-lint,phase-c-format}.log` (uncommitted). Following the Programmer’s existing 650-test success, only the necessary 26 tests were rerun here; no successful rerun of all 651 is claimed. The existing synthetic rollback diagnostic remains an injected diagnostic after a successful real rollback, not a reproduction of a real rollback failure. Changes are limited to tests/coordination markers; no production/git/E2E/server work.



The owner-slot decision is implemented. The private shared core in `cli/fleet.rs` has signature `fn create_with_connection<M: Multiplexer>(slot: &mut Option<Connection>, name: &str, agent_name: &str, monitor_file: &str, monitor_model: Option<&str>, resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>, probe: &dyn SpawnProbe, hooks: &dyn CreationHooks) -> Result<Value, CafleetError>`. Phase C can call it from colocated tests in the same file to verify retention of Some on success, None on real broker failure, and lock ordering for pane compensation after closing. Preconditions fail before the broker and retain the slot; only broker failures take/drop it. The test-only create_with_dependencies creates an owner slot and delegates to the same core, with no separate compensation implementation. The member test-only dependency wrapper likewise delegates to the same borrowed-Connection core. Production bodies contain zero direct db::connect calls; connections come only from the per-call factories in shared dispatch/doctor/setup. After closing, Finished.conn=None; there is no reopen for observation. The ownership exception is also reflected in the public SPEC/cli-options/contributing docs.

Execution history (director): Connection ownership decision — Adopt Option<Connection>. The invocation owns the connection and lends the same connection to ordinary bodies through as_mut(); only fleet create receives the owner slot. On broker failure, end the borrow and close the real handle with take/drop before CLI pane compensation, preserving Step 3’s backend close→rollback inside the callback and rollback→DB close→CLI close after it. Do not reopen a connection closed on failure for the observer; pass Finished.conn=None. Success retains the slot and opens once, preserving the existing same-connection ACK/schema/assets tests. The existing test-only create_with_dependencies wrapper may remain while referenced, but must construct an owner slot and delegate to the production core rather than implement compensation separately. Resume implementation and return the finalized owner-slot core signature. Enable the Tester to verify Some retention on success/None on failure and the existing lock/cleanup ordering directly in Phase C. Sharing in this design means preserving the required lifetime without reopening; it does not prohibit mandatory closing on failure. Briefly document this exception in public docs too.

Execution history (programmer): Phase B needs a Connection ownership decision. The current fleet::create_with_dependencies drops its owned Connection before calling PaneGuard::rollback on broker failure, closing the DB handle and proceeding to pane compensation even if explicit ROLLBACK fails (drop(conn) and its preceding comment in fleet.rs; Step 3’s close/rollback guarantee). Converting every body to a plain &mut Connection as approved would make drop(conn) drop only a reference, losing this failure order and lock-release guarantee. Proposal: the invocation owns Option<Connection>; ordinary bodies borrow through as_mut(), while only fleet create receives the owner slot and takes/drops it on broker failure before pane compensation. Success keeps the same connection and opens once; Finished.conn is None if failure closed it. The old create_with_dependencies test seam delegates to the same real operation by constructing a test-only owner slot, preserving the Tester’s existing test bodies (no unreferenced wrappers retained). CLI body/creation changes are paused pending approval of this ownership clarification. Query/idle, diagnosis/presenter/guard, and dispatch wiring are in progress. Staged tests are connected to lib, but migration to the new body signatures is pending, so compilation/success is not claimed.

Execution history (director): Reviewed the 30 Phase A tests and committed them as 0be7f66c. Of the 12 additions using existing APIs, 11 pass and 1 is expected red for future idle timestamps. The designs of the 18 unconnected tests against approved APIs have been reviewed; execution remains for Phase B. The Programmer must use the finalized signatures below (including InvocationHooks.asset_env) to implement Diagnosis and boundary presenters/guards, propagation of the same Connection to all CLI bodies, doctor/setup contracts, observers after real operations finish, lean roster/activity separation, unique 500-bind lookups, and final idle clamping at 0. Connect cfg(test) mod step6_contract_tests from lib.rs so all 18 can run, without changing existing test bodies. Report test defects found during type wiring to the Director with evidence. Update planned/pending wording in docs only for implemented areas. Run and report the designated Rust suite (lib and 9 integration targets)/lint and docs build. Preserve existing wire/text/exit/guard behavior, independence of the two setup halves, duplicate-monitor callbacks, and transaction ordering; retain no unreferenced compatibility wrappers. No E2E/cli_server execution, browser/server startup, or git operations.

Execution history (tester): Created 30 Phase A tests. The 12 executable cases comprise 8 in members (fixed-now future clamping/null/no fallback for an invalid maximum/selection of the maximum raw string before offset parsing/fraction truncation; name counts 0/1/500/501/1001 plus duplicates/unknown/deregistered; owner-only roster; ACK time; summary sends), 1 fleet tie-order test, and 3 doctor tests (Missing/Unversioned with an empty ledger; all sections and issues despite simultaneous open/path failures; no read of a malformed asset table for a non-head schema). Activity fixtures deliberately separate owner from to_member_id to detect use of the recipient column by mistake. Reused equivalent existing coverage for role/placement/wire, owner timeline, all guard groups, doctor’s six states and assets/superseded entries, assets after setup failure, selectors, and the duplicate-monitor race.

Execution history (tester): The remaining 18 tests for approved but unimplemented APIs are isolated in `cafleet/src/step6_contract_tests.rs`, not yet connected to lib.rs. Four schema tests require six states, empty ledgers, SQLite bookkeeping, SQL failure causes, distinct guard/doctor wording from the same facts, and JSON ordering. Five asset tests require Current/Stale/NotInstalled/PathError, raw installation timestamps, superseded ordering, one lookup each, Guard path-error precedence, and real SQL errors for invalid tables. Three query tests require real Statement parameter_count/expanded_sql, unique-500 chunk counts, zero SQL for empty input/no completion on failure, lean roster versus three activity aggregates, and fixed now. Six invocation tests require actual factory call counts and TEMP sentinels, SELECT after an in-memory message ACK completes, unchanged real DB messages after rejection, and same-connection setup reclassification to Head after migration/Head no-op/assets after DB refusal/assets retry after the initial open fails. Setup tests use asset_env to resolve only the claude path under BASE and do not evaluate invalid paths of unselected agents. No tests mutate process-global environment or HOME/CODEX_HOME.

Execution history (tester): In Phase B, the Programmer must implement the approved diagnosis/guard/presentation APIs, InvocationHooks (including asset_env), and three observed query entry points, then connect the 18 tests with the test-only `mod step6_contract_tests;` in lib.rs. The Tester added no production stubs/module exports/Cargo settings. The staged file was only parsed/formatted through the full-path mise format task; type checking or execution is not claimed. Observers must execute and observe SQL through real Connections/Statements, not substitute invented events or planned SQL strings.

Execution history (tester): Results: 11 pass/1 expected red among the 12 added tests. For a future timestamp, the current idle_seconds returns Some(-60), violating the Some(0) contract. The first designated full suite (with 11 additions, --no-fail-fast) produced 630 pass/1 expected red. After strengthening the owner-column check and adding one doctor case, `--lib --test cli_setup_doctor step6_` confirmed unit 8 pass/1 expected red plus CLI 3 pass. `CI=true mise //cafleet:lint` passed clippy --all-targets -D warnings/fmt check and its dependent admin tsc/vite build. Evidence is under BASE in `.cafleet-evidence/tester-step6/{behavior-red,selected-red,step6-red,lint,format,staged-format}.log` (uncommitted). No E2E/cli_server execution, browser/server startup, implementation edits, or git operations. Type checking and execution observation for the 18 staged tests remain required after Phase B.

Execution history (director): Adopt the API proposal below as the finalized Phase A/B signatures, adding asset_env: config_dir::EnvLookup<'a> to InvocationHooks; the standard path passes std::env lookup. Pass this lookup only to diagnosis and setup’s agent_paths, preserving HOME reads, guard ordering, and lazy resolution of selected agents only. Tests must direct CLAUDE_CONFIG_DIR and other lookup results to fixtures under BASE and use only setup --coding-agent claude to avoid writes to real assets. Do not mutate the process environment or overwrite HOME/CODEX_HOME. Adopt the factory’s in-memory DB plus TEMP sentinel and SELECT after a real message ACK, and observation of real Statement parameter_count/expanded_sql; do not make these contracts tests of invented events or strings alone.

Execution history (director): Begin Phase A, Tester. Run behavioral regressions using existing APIs first, and also create direct type/execution-observation tests for the finalized diagnosis/InvocationHooks/query observer signatures. Explicitly identify checks that cannot compile because APIs are unimplemented; the Tester must not add production stubs or report them as executed. If needed, place new test modules in separate files for Phase B connection without preventing the existing suite from running. Cover the six schema states (including an empty ledger), current/stale/absent/invalid-path/superseded assets, all doctor sections, schema→assets→body and one successful open, setup rediagnosis/retry after initial open failure/assets after DB failure, unique name counts 0/1/500/501/1001 and bind counts, lean roster/activity, and idle/ACK/summary/tie ordering. Reuse equivalent existing tests rather than duplicate them. No production edits, git operations, E2E, or server work. Record and return results and remaining unimplemented APIs.

Execution history (director): Reviewed the docs and committed them as c286830d. Clarified the Missing/Unversioned table as “no recorded version,” since recorded_version returns None for both a missing and an empty ledger. Docs build/148 links passed; checked the existing owner-only roster and activity selection against the code. For Phase A, the Tester should first add existing-API regressions for name lookup results/boundaries, lean roster inclusion and role/placement/order, fixed-clock idle future/null/unparseable/offset/fraction cases, ACK/summary activity, fleet tie ordering, owner timeline, and existing guard/doctor/setup output and ordering. Tests needing new types or query/connection observation APIs must await the next finalized signatures; do not invent unapproved APIs. Later seams must verify actual propagation of the shared connection to bodies and query counts, rather than infer them from results alone. Only the designated Rust suite/lint; no E2E/server, production edits, or git operations.

Execution history (programmer): Finalized API proposal for Phase A. The signatures below await Director review; no production/test/Cargo changes have been made. Query observation uses a single per-call observer mechanism. No SQLite trace feature, process-global hook/cache, or general repository/context framework will be added.

**Placement and diagnostic facts**: Expose `cafleet/src/diagnosis.rs` from `lib.rs` as `pub(crate) mod diagnosis`. The following types are `pub(crate)`, and listed struct fields are also `pub(crate)`; all types implement Debug (AssetInstallRecord also needs Clone). Reuse existing `config_dir::{ResolvedDir, DirSource}` and `CafleetError`; do not implement Display/JSON Index on diagnostic types.

```rust
enum SchemaState {
    Missing,
    Unversioned,
    Behind { recorded: u32, head: u32 },
    Head { version: u32 },
    Ahead { recorded: u32, head: u32 },
    Unreachable { cause: CafleetError },
}
struct AssetInstallRecord {
    coding_agent: String,
    path: String,
    cafleet_version: String,
    installed_at: String,
}
enum AssetState {
    Current { identity: ResolvedDir, install: AssetInstallRecord },
    Stale { identity: ResolvedDir, install: AssetInstallRecord },
    NotInstalled { identity: ResolvedDir },
    PathError { variable: &'static str, raw_value: String, cause: CafleetError },
}
struct AgentAsset { coding_agent: &'static str, state: AssetState }
struct AssetReport {
    agents: Vec<AgentAsset>,
    superseded: Vec<AssetInstallRecord>,
}
struct Diagnosis {
    head_version: u32,
    schema: SchemaState,
    assets: Option<Result<AssetReport, CafleetError>>,
}
enum AssetMode { Guard, Report }

fn classify_schema(conn: &Connection, head: u32) -> SchemaState;
fn asset_table_exists(conn: &Connection) -> Result<bool, CafleetError>;
fn diagnose_assets(
    conn: Option<&Connection>,
    env: config_dir::EnvLookup<'_>,
    home: &Path,
    cli_version: &str,
    mode: AssetMode,
) -> Result<AssetReport, CafleetError>;
```

The listed functions are also `pub(crate)`. `classify_schema` moves and executes the real SQL from existing recorded_version/has_foreign_tables in the diagnosis module, using an already-open Connection. No recorded version includes both missing and empty ledgers: Missing if there are no tables other than the ledger/SQLite internal tables, otherwise Unversioned. SQL failures become the raw cause in Unreachable. If opening itself fails, the caller constructs Unreachable without reopening a Connection to call the classifier. Diagnosis also holds `head_version` to preserve the head in doctor JSON for Missing/Unreachable.

`diagnose_assets` resolves paths once each, in fixed claude/codex/opencode order, using the existing resolver. Guard mode returns the first path error as the existing CafleetError without executing install SQL (the function is not called at all if the schema is rejected). Report mode retains PathError as the agent’s state and continues with other agents. conn=None does not read records; even with Some, asset_table_exists=false is classified as empty records. Decode record queries directly into typed AssetInstallRecord, without adding Value→reparse conversions. Compare versions as strings; superseded contains only other paths of successfully resolved agents, sorted by (coding_agent,path). Preserve table/record SQL errors as Result::Err, distinct from ordinary Missing/NotInstalled states. There is no need to remove the existing public asset_installs wire API as an unrelated Step 6 change.

Diagnosis.assets=None means unevaluated (a schema-stopped guard or setup’s schema-only stage); Some(Ok/Err) contains an actual asset diagnosis result. Doctor calls Report with conn=Some only for schema Head and None for other states. Setup shares the schema classifier and asset_table_exists, without resolving every agent through Report before installation. Preserve lazy path resolution in existing selector order and installations completed before an intermediate failure.

**Presenter/guard signatures**: Place the following `pub(crate)` functions at the boundaries. Replace the existing settings-taking schema_guard/stale_assets_guard with these fact-taking versions. Guards must not connect or execute SQL. Keep the existing private renderer for doctor’s complete text table, constructing its input AgentRow from AssetReport.

```rust
// cli/helpers.rs
fn schema_guard(schema: &SchemaState) -> Result<(), CafleetError>;
fn stale_assets_guard(assets: &AssetReport, cli_version: &str) -> Result<(), CafleetError>;
// presentation.rs — existing field order/null/error detail, without Error: prefix
fn doctor_database(schema: &SchemaState, head: u32) -> serde_json::Value;
fn doctor_database_detail(schema: &SchemaState) -> String;
fn doctor_assets(assets: &AssetReport, cli_version: &str) -> serde_json::Value;
```

doctor_database returns the entire existing database object; doctor_assets returns the entire existing coding_agents object (preserving ok/cli_version/agents/superseded order). Keep the existing differing text/detail and guard strings at their respective boundaries. The Unreachable guard returns cause.clone() to preserve error classification; doctor displays cause.message() as detail. Keep the existing CafleetError path for asset SQL errors without inventing a new wire state. The all-section regressions in this step cover the existing open/schema/path failure contracts.

**Invocation observation**: In `cli/mod.rs`, place the following `pub(crate)` types/functions, using the same private dispatch as existing `run()` after parsing. These exports are not stable public CLI APIs. Crate unit tests can call them; Hooks fields are also pub(crate). The standard path uses the existing process environment; diagnosis unit tests and invocation tests inject EnvLookup for asset path resolution.

```rust
enum SchemaPoint { Guard, Doctor, SetupBefore, SetupAfter }
enum InvocationPhase { CommandBody, SetupDatabase, SetupAssets }
enum InvocationEvent<'a> {
    SchemaInspected {
        point: SchemaPoint,
        conn: &'a Connection,
        state: &'a SchemaState,
    },
    AssetsInspected {
        conn: Option<&'a Connection>,
        result: &'a Result<AssetReport, CafleetError>,
    },
    Finished {
        phase: InvocationPhase,
        conn: Option<&'a Connection>,
        result: &'a Result<(), CafleetError>,
    },
}
struct InvocationHooks<'a> {
    connect: &'a dyn Fn(&str) -> Result<Connection, CafleetError>,
    observe: &'a dyn for<'event> Fn(InvocationEvent<'event>),
    asset_env: config_dir::EnvLookup<'a>,
}
fn run_with_hooks(
    settings: &Settings,
    argv: &[&str], // includes argv[0] = "cafleet"
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError>;
```

Existing run() retains clap parsing/exit behavior, then uses shared dispatch with `connect=crate::db::connect` and observe=no-op. run_with_hooks is an entry point for real-command regressions with valid argv; try_parse_from failures become Usage(error.to_string()) (do not use this helper’s parse errors to judge compatibility of the existing CLI parser). All fleet/member/message/monitor bodies receive the same `&mut Connection` opened by dispatch, removing body-local connects. Only server schema startup uses this factory; HTTP blocking-handler connections retain separate lifetimes. Setup preserves URL validation/parent-directory creation before calling the factory. Once opening succeeds, both DB and asset halves retain that Connection. If the initial open fails, the assets half may retry the factory.

SchemaInspected fires only immediately after the real classifier returns, and AssetsInspected only after real path/record diagnosis returns. Finished passes the real result and connection **after** the real command body/DB half/assets half returns (no invented success events or substitute body closures). Ordinary commands rejected by schema/assets guards produce no CommandBody Finished. Count factory calls inside the injected closure, not through a separate Open event. After a successful migration, setup reruns the classifier on the same connection and emits SetupAfter, then SetupDatabase Finished; it emits SetupAssets Finished after assets execute. Do not fabricate a successful-migration SetupAfter for head no-op/rejection/failure. Proceed to SetupAssets Finished even if the DB half fails. Keep the existing after_diagnosis callback in its position before the migration transaction.

The Tester creates real fixtures and a TEMP sentinel in an in-memory DB through the factory, then SELECTs the sentinel through the conn supplied to each observer. Also run a real `message ack` or similar command through run_with_hooks and SELECT through CommandBody Finished’s conn to confirm that the same in-memory message changed to completed. This prevents claiming connection propagation to the body merely because an event has the same ID. On guard rejection, verify no actual body side effects; in setup, check head/version and actual asset records through SetupAfter. Callbacks have no return value and inject neither retries nor success; RefCell and similar state stay within per-call test closures.

**Query observation**: In `broker/members.rs`, add the following `pub(crate)` entry points; existing public functions must delegate to them with observer=no-op. Existing list_member_records obtains now_utc() once and passes it in. Do not create separately implemented test stubs.

```rust
fn get_member_names_observed(
    conn: &Connection, member_ids: &[i64],
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<BTreeMap<i64, String>, CafleetError>;
fn list_roster_records_observed(
    conn: &Connection, fleet_id: i64, include_message_holders: bool,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MemberRecord>, CafleetError>;
fn list_member_records_observed(
    conn: &Connection, fleet_id: i64, now: chrono::DateTime<chrono::Utc>,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MemberActivity>, CafleetError>;
```

Call each observer once after fully consuming the query results from the actual prepared/bound Statement and releasing the Rows borrow. A zero-row result still represents an executed query and triggers one call; empty ID input creates no Statement and triggers zero calls. Prepare/query failures produce no completion callback (this step measures query bounds on successful SELECT paths). The Tester uses the supplied Statement.parameter_count() for bind counts and expanded_sql() to inspect owner EXISTS/aggregate presence and bound fixture IDs in real SQL. One event per successful chunk allows observation of ceil(unique/500). Do not pass synthetic SQL strings or events stating that a query is planned. Local dependency source confirmed that both methods in rusqlite 0.39.0 are available with the current bundled feature, so no Cargo.toml/Cargo.lock feature/dependency changes are needed.

Only this design doc changed in this update. Implementation/tests against these signatures remain for Phase A/B after Director approval.

Execution history (programmer): Prepared Step 6 contract docs (no production/test changes). Added Shared diagnosis and connection reuse to SPEC §6.3, defining the six SchemaState states, separation of AssetState facts and presentation, schema→assets→body ordering, Connection reuse within an invocation, all doctor sections and install-record reads only at head with the table present, and setup’s continuation to assets after DB failure, rediagnosis on the same connection after DB changes, and permitted reconnect after initial open failure. Preserve path resolution only for selected setup agents, existing text/JSON/issues/exit behavior, and duplicate-monitor callback ordering. Contributing and cli-options describe the same boundaries with explicit planned status.

SPEC §6.2, data-model’s Query and activity contracts, and cli-options’ member list now define last_sent as the maximum created_at for all sender message kinds (including summary), last_recv as the maximum created_at for owner unicasts, and last_ack as the maximum status_timestamp for owner completed unicasts. Select the lexicographic maximum of the three raw strings, parse it once with the existing RFC3339 reader, and return null if all are null or the selected value cannot parse (no fallback to an older valid value). Preserve existing num_seconds truncation and clamp at 0 only at the end. Align descending fleet ID tie-breaking across SPEC/CLI/WebUI. The lean roster preserves current Rust owner-only message-holder EXISTS, kind, placement, and member_id ordering (correcting SPEC’s owner OR sender wording; deregistered members referenced only as senders are excluded). Timeline’s owner join was already aligned in Step 4; reconfirmed it and linked it from data-model. Defined name lookups with at most 500 unique binds, query-count bounds, zero queries for empty input, omission of unknown IDs, inclusion of deregistered members, and BTreeMap results. Query separation/batching/clamping and shared diagnosis are explicitly marked unimplemented.

Candidate observation points for the Tester (Director finalizes signatures before Phase A; no seams implemented in this update):

- Match pure diagnostic classification directly through SchemaState/AssetState; use existing guard/doctor/setup fixtures to assert that the same state produces the respective existing text/JSON. A candidate per-invocation connection factory/call observer records one open on success, schema→assets→body, and setup rediagnosis after migration. Distinguish retry after initial open failure from attempting assets after DB-half failure; verify schema rejection precedes invalid asset paths and doctor does not suppress other sections. Do not add process-global hooks.
- Consider a per-query observer or connection-local SQLite trace to observe name lookup maps and SQL counts/binds for 0/1/500/501/1001 unique IDs, many duplicates, and unknown/deregistered IDs. Do not pin whitespace in complete SQL strings. Observe that roster has owner EXISTS only and no message aggregates, while activity has the three required aggregates; verify returned kind/placement/member_id order and exclusion of sender-only holders with a separate query.
- Existing private idle_seconds accepts MemberActivity and fixed now, so future/all-null/unparseable maximum string (with another valid value)/offset/fraction cases can be tested directly. Use SQL fixtures to check that ACK advances only last_ack without changing created_at, summaries contribute to last_sent, simultaneous fleets sort by descending ID, and timeline scope holds when owner and sender fleets are deliberately separated.

`CI=true mise //docs:build` exited 0. Checked 148 local links (including 134 fragments) across four changed public pages against actual generated HTML IDs, with zero failures. Evidence: `.cafleet-evidence/programmer-step6/{docs-build,anchors}.log` (uncommitted). Also confirmed removal of the old fleet ASC/activity formula/owner OR sender wording from the affected SPEC sections. Step 6 implementation/tests have not started, so checkboxes/Progress remain unchanged. No Step 7/8 work, E2E/cli_server execution, browser/server startup, or git operations.

- [x] Finalize §6 diagnosis, activity selection, fleet ordering, and owner joins in docs/SPEC; apply typed diagnosis to guard/doctor/setup. <!-- completed: 2026-09-05T23:00 -->
- [x] Separate lean roster and activity queries; change name lookups to 500-ID batches and verify empty, duplicate, unknown, and deregistered inputs and query counts. <!-- completed: 2026-09-05T22:12 -->
- [x] Add only idle clamping at 0; verify future timestamps, all-null/unparseable values, ACK updates, and summary sends with a fixed clock. <!-- completed: 2026-09-05T22:12 -->

### Step 7: Fetch only the member history needed by the WebUI

Execution history (director): Phase C is complete. Reviewed SQL delivery filter→status/ID ordering→bound LIMIT, form decoding/limit validation after scope checks, permanent unbounded entry points, and selectHistory wiring in both tabs; committed as 9b2b5808. Confirmed byte-identical staged TS relocation and, after Rust module connection, logs showing the designated 669-test suite, 22 admin tests, lint/build/docs links all passing. All 29 added tests have run; approve 18/36. The design doc is excluded from commits; E2E was not performed.

Execution history (tester): Created 29 Phase A tests: 12 executable Rust cases (11 HTTP plus 1 permanent unbounded API) and 4 TS cases, with 6 Rust plus 7 TS cases awaiting new APIs. Added executable cases produced 5 pass/11 expected red; the 13 staged cases remain unexecuted. HTTP tests use the existing tower router and pin both endpoints’ limits 1/200/201/1000; delivery counts 0/200/201/1205; exclusion of newer summaries before limiting; 1205 rows with limit omitted and ignoring unknown queries; ASCII digits/form decoding/leading zeros/signs/whitespace/Unicode digits/decimal/exponent/overflow/duplicate decoded keys; Path→header→fleet→member→limit ordering; deregistered/other-fleet cases; status_timestamp precedence/descending ID ties; complete row wire/null/envelope; and SQL 500 versus limit 422 ordering. Corrected the initial fixture’s attempt to deregister the root Director to use an ordinary historical sender; all final 9 red cases are contract violations from the unimplemented limit.

Execution history (tester): The 6 cases in `cafleet/src/step7_contract_tests.rs` await approved HistoryOptions/with_options/observed APIs and are not connected to lib.rs. Connect test-only `mod step7_contract_tests;` in Phase B. They verify one bind/no LIMIT for None, two binds/real SQL LIMIT for Some, returned IDs, existing owner/status and sender/status index use through EXPLAIN QUERY PLAN on actual expanded SQL, one observer call after all rows succeed/one for zero rows/zero on prepare or decode failure, Value errors for 0/1001/usize::MAX, equivalence with permanent unbounded APIs, and scope/ACK/ordering. In particular, an invalid-enum row is placed last and outside the limit: bounded SELECT must succeed while unbounded returns InvalidStoredValue, detecting implementations that decode all rows and then truncate. No full-SQL/full-plan goldens or invented SQL events. The staged file was parsed/formatted through mise format only, without type checking or execution.

Execution history (tester): The two real fetchInbox/fetchSent URL/header/response cases in `admin/tests/history-api.test.ts` are expected red because ?limit=201 is absent. The two error-detail/unselected-fleet-header cases pass; global fetch stubs and setFleetId are restored after each test. The 7 cases in `admin/staged-tests/history.test.ts` await unimplemented src/history.ts and are outside the tsc/Vitest includes. In Phase B, move them to `admin/tests/history.test.ts`, connect selectHistory/HISTORY_ROW_CAP/HISTORY_FETCH_LIMIT/HistoryWindow, and run the 0/200/201/1205, truncation only at the 201st delivery after summary exclusion, order/field preservation, and frozen-input nonmutation cases. Actual wiring to both MemberDetail tabs is for Programmer implementation/review. No package/tsconfig/Vitest configuration or production helper changes.

Execution history (tester): Final targeted Rust command `CI=true mise //cafleet:test --no-fail-fast --lib --test webui_routes history_` yielded unit 2 pass (1 new/1 existing) plus HTTP 2 pass/9 expected red. `CI=true mise //admin:test` yielded 13 pass/2 expected red, including the existing 11 timeline cases, with tsc passing. `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt check and the dependent admin tsc/vite build; `CI=true mise //admin:lint` also passed (linting staged TS is not type checking). The initial unused Rust test import was removed. Evidence is under BASE in `.cafleet-evidence/tester-step7/{http-red,history-red,admin-red,rust-lint,admin-lint,format,staged-format}.log` (uncommitted). The 13 unconnected cases are excluded from pass counts. No implementation/Cargo/package configuration/git/E2E/browser/server work.

**Implemented APIs and compatibility**: Retain the existing two-argument unbounded APIs as permanent entry points, delegating to the options entry points with `HistoryOptions::default()`. HTTP uses the options entry points. Implemented the following types/functions and connected the 6 staged Rust tests through a cfg(test) module in lib.rs. No JSON wrappers or unreferenced adapters were added.

```rust
// cafleet/src/broker/queries.rs; pub items are also re-exported by broker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryOptions {
    pub limit: Option<usize>,
}
pub fn list_inbox_records(
    conn: &Connection, member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError>;
pub fn list_sent_records(
    conn: &Connection, member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError>;
pub fn list_inbox_records_with_options(
    conn: &Connection, member_id: i64, options: HistoryOptions,
) -> Result<Vec<MessageRecord>, CafleetError>;
pub fn list_sent_records_with_options(
    conn: &Connection, member_id: i64, options: HistoryOptions,
) -> Result<Vec<MessageRecord>, CafleetError>;

// Both production options APIs delegate to the corresponding observed core
// with a no-op observer; crate-local tests can call these directly.
pub(crate) fn list_inbox_records_observed(
    conn: &Connection, member_id: i64, options: HistoryOptions,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError>;
pub(crate) fn list_sent_records_observed(
    conn: &Connection, member_id: i64, options: HistoryOptions,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError>;
```

The observer receives the real Statement exactly once after all history SELECT rows decode successfully (once for zero-row success, zero on prepare/query/decode failure). Other SELECTs such as name lookups are excluded. `None` means no LIMIT clause and one member_id bind; `Some(n)` means `LIMIT ?2` with two binds for member_id/limit. Rust truncation after fetching is prohibited. The Tester can assert `parameter_count()`, `expanded_sql()`, and returned IDs/order together. Verify existing owner/status and sender/status index use with `EXPLAIN QUERY PLAN` on expanded SQL from the real statement, without goldens for full plans or SQL whitespace. No new indexes/migrations.

**Limit lexical/API boundaries**: The HTTP extractor obtains only side-effect-free RawQuery; decode/validate it after existing Path→fleet header→fleet existence→member membership checks succeed. Detect duplicate limit keys in the key/value sequence before collapsing it into a map. After URL form decoding, accept only nonempty ASCII digits; allow leading zeros (`001`) as 1, but reject signs (`+1`/`-1`), whitespace, Unicode digits, decimals, and exponent notation. Count keys that decode to limit as duplicates too. Ignore unknown keys. Checked parsing and the 1..=1000 range check produce the same 422 detail for overflow/0/1001. Direct options inputs `Some(0)` / `Some(n > 1000)` return `CafleetError::Value` with the same wording; only HTTP maps this to 422 (SQL failures remain 500). Implemented the Director-approved lexical clarification and direct-input range guard in shared validation. URL form decoding uses form_urlencoded 1.2.2, already in the lockfile, as a direct dependency rather than adding a custom decoder. HTTP tests invoke existing `webui::create_app(&str) -> Result<axum::Router, CafleetError>` through tower oneshot; parser-only tests do not substitute for validation-order checks.

**TS fetch/display entry points**: Wire the following pure helper in new `admin/src/history.ts` into both MemberDetail tabs. Preserve input order, defensively exclude summaries, and return up to 200 deliveries plus whether a 201st exists. Store the helper result in tab state; MessageList renders only visible, and truncated alone controls the existing footer. Preserve existing load/refresh/error/Promise.all behavior; do not include Step 9 fetch restructuring.

```typescript
import type { FormattedMessage, TimelineMessage } from "./types";
export const HISTORY_ROW_CAP = 200;
export const HISTORY_FETCH_LIMIT = HISTORY_ROW_CAP + 1;
export interface HistoryWindow {
  visible: TimelineMessage[];
  truncated: boolean;
}
export function selectHistory(
  rows: readonly FormattedMessage[],
): HistoryWindow;
// Existing admin/src/api.ts public entry points keep their signatures:
export function fetchInbox(memberId: number): Promise<TimelineResponse>;
export function fetchSent(memberId: number): Promise<TimelineResponse>;
```

Existing fetchInbox/fetchSent in api.ts use shared HISTORY_FETCH_LIMIT to fetch `/api/members/{id}/inbox?limit=201` and `/api/members/{id}/sent?limit=201`. Add no URL-only helper. Stub global fetch in Vitest’s Node environment to verify the real functions’ URLs, X-Fleet-Id, and existing responses; restore the stub and setFleetId after tests. Test selectHistory directly with 0/200/201/many rows, mixed summaries, preserved order, and nonmutation of input. Moved staged helper tests unchanged to admin/tests/history.test.ts and ran them with the existing fetch API tests. No React DOM/browser infrastructure is needed.

- [x] Add §5 limits, validation order, and compatibility when omitted to docs/SPEC; implement broker options and HTTP→SQL limits. <!-- completed: 2026-09-06T06:45 -->
- [x] Change WebUI to fetch 201 rows and display 200; verify boundaries/invalid input/scope/order and existing unbounded HTTP/CLI behavior. <!-- completed: 2026-09-06T06:45 -->

**Verification results**: The designated Rust suite had 669 passed/0 failed (442 unit tests and 9 integration targets); admin tests had 22 passed/0 failed. All 29 Step 7 additions (18 Rust plus 11 TS), including the 13 staged cases, were connected and run. Real Statement checks for one/two binds, SQL LIMIT, query plans using the two existing indexes, and invalid enums outside the limit being excluded from bounded decoding also passed. Rust lint (all-target clippy -D warnings/fmt), admin lint, dependent admin build (tsc/vite), and docs build exited 0; 21 local links/18 fragments across two changed public pages had zero failures. Code review confirmed that both tabs actually render selectHistory’s visible/truncated and fetchInbox/Sent use the shared 201 constant. Evidence: `.cafleet-evidence/programmer-step7/{selected-tests,admin-tests,rust-lint,admin-lint,docs-build-final,anchors-final}.log` (uncommitted). No test-expectation changes, new indexes, Step 9 work, E2E/cli_server execution, browser/server startup, or git operations.

### Step 8: Introduce shared lifecycle and capture processing

Execution history (director): Phase C is complete; implementation committed as fe6d8575. Removed old create wrappers/CreationTransport/the trivial create_core; both create_with_options entry points connect directly to the same SpawnExecution/SpawnMultiplexer. final-tests.log records 711 passing tests. Subsequent changes were only unused-import removal and formatting, followed by successful all-target clippy/fmt/admin build in final-lint-3.log. Docs build and 143 local links/131 fragments also passed. Reviewed compensation ordering/DB locks/owner slots/remaining Duration/independent 5-second close in the 40 new and 14 migrated cases, plus monitor SQL/handles and capture bytes/hashes. Formally close the Step 3 timeout carryover and approve 21/36. Design doc/evidence are uncommitted; E2E was not performed.

Final implementation and verification are complete. MemberCreateOptions/FleetCreateOptions and PreparedSpawn share preparation before registration/transactions and delegate directly from create_with_options to SpawnMultiplexer/SpawnExecution. Removed unreferenced create_with_connection/create_with_dependencies, all of CreationTransport, and the trivial create_core; zero references to old entry points remain in Rust sources. Retained permanent legacy multiplexer/runner entry points and the PaneGuard unit-test entry point.

The designated Rust suite after migration/removal had 711 passed / 0 failed (lib 482, integration 229); all 40 new-API cases passed. Real SQLite/events/write locks verify member/fleet known run timeout→backend close→DB compensation, unknown split timeout→no guessed kill, and SQL failure→rollback/real DB close→CLI kill. Exercised fractional remaining budgets from 30 seconds, independent 5-second direct close, remaining compensation after failure, primary diagnostics, and no double cleanup. The two real-process SystemRunner Duration cases are drain/exit/timeout/reap regressions, not proof of a precise wall-clock bound. Return this evidence for final Director review of the Step 3 carryover.

`CI=true mise //cafleet:lint` passed all-targets clippy -D warnings, fmt check, and dependent admin build. `CI=true mise //docs:build` also passed; checked 143 local links / 131 fragments across three changed public pages against generated HTML, with zero failures. Updated planned wording in public SPEC/docs to implemented status. Evidence: `.cafleet-evidence/programmer-step8/{final-tests,final-lint-3,final-docs-build,final-anchors,removed-api-audit}.log`. Changes after the final tests started were limited to unused-import removal/rustfmt and documentation; lint passed on the final source including those changes. No E2E/cli_server, browser/server, or git operations. All three Step 8 tasks complete; Progress 21/36.

The API proposals, adoption decisions, and Phase A descriptions below are design history. Refer to the final results above for current implementation and verification status.

Execution history (tester): Before final verification, the Programmer must remove unreferenced old create wrappers/CreationTransport::legacy. Migrated 14 existing creation/owner-slot cases to create_with_options plus SpawnPreparation/SpawnExecution; tests contain zero references to old entry points. Real Herdr plus a per-call fake clock/Duration runner advances in 125ms increments and asserts the remaining 30-second budget/independent 5-second close, rejecting creation calls that leak to the old runner. The only expectation changes for the adopted direct close remove optional get:error at two member and three fleet locations; primary-error/DB-row/real-lock/compensation-order/disarm/owner-slot/TEMP assertions remain. Converted a nested if to a let-chain without changing assertions. All 60 targeted cases passed: 14 creation, 6 existing guard, and 40 Step 8 contracts, including real-process Duration tests and the Step 3 timeout carryover. Evidence and remaining wrapper references are under BASE in .cafleet-evidence/tester-step8/migration.md and associated logs. The five unused compilation warnings refer only to wrappers/legacy transport awaiting deletion. The Programmer will run the final designated suite/lint after deletion, preserving permanent non-create APIs and standalone guard entry points.

Execution history (director): Reviewed Phase A’s 5 files/42 tests and committed them as 94f802aa. Confirmed the designated 671-test Rust suite, including two existing-API additions, and lint/build success; approved the 40 new-API cases as not yet compiled/executed. Begin Phase B and implement spawn preparation/shared Duration deadline, MonitorLease, and typed capture/presenters according to the adopted APIs/contracts below. The Programmer may connect step8_spawn_tests/step8_monitor_tests/step8_capture_tests through root cfg(test) modules and include! step8_duration_tests in a child module under runtime::system::tests to share existing ProcessFixture/mutex (test bodies must not change). Do not weaken ContextOnly’s prohibition of legacy calls or real DB/event/lock assertions. The two real-process Duration cases verify drain/exit/timeout/reap, not precise wall-clock measurements. Run all 40 and return evidence that resolves the Step 3 timeout carryover. Existing create wrappers may only temporarily delegate to the shared core; if tests need SpawnMultiplexer migration, report concrete entry points for a Tester correction cycle. Return test defects through the Director too; do not alter expectations simply to pass. Verify the designated Rust suite (--lib --test cli_compensation --test cli_fleet --test cli_global --test cli_member --test cli_message --test cli_setup_doctor --test docs_sync --test monitor_uniqueness --test webui_routes) and Rust lint/build through mise, and update planned statements in public docs. No E2E/cli_server execution, browser/server startup, or git operations. Design doc/evidence remain uncommitted.

Execution history (director): API decision — Adopt the Duration-based TimedCommandRunner/Deadline/SpawnExecution/SpawnMultiplexer, create options/PreparedSpawn, MonitorLoopHooks/driver, CaptureSnapshot, and typed presenter proposals below. Preserve existing run’s integer-second API and connect it to the same SystemRunner process core. Ordinary layout/resize Failed remains best-effort, but Timeout/budget exhaustion fails creation; deadline checking before transfer is mandatory. Approve assigning the independent 5-second compensation budget to direct close and skipping pre-get/rebalance (normal delete and missing tolerance remain). Format seconds as the actual Duration’s integer or fractional value without unnecessary trailing zeros. Adopt the budget-exhaustion diagnostic below, now added to public SPEC/backend docs. Preserve existing validation order during preparation, resolve cwd only for Herdr, snapshot environment per call, and retain placeholder handling’s existing compensation position. Also adopt MonitorLease and real-completion events, primary error plus clear suffix, the new startup-flush failure diagnostic, one now call immediately after each successful capture, and no text construction on JSON paths. Old create wrappers remain only during test migration; remove unnecessary ones at the end. Existing non-create Multiplexer/CommandRunner APIs are permanent compatibility entry points.

Execution history (director): Phase A — Tester: write tests against the adopted APIs using small per-call fake clocks/timed runners/registration handles/failing writers and real SQLite fixtures. Without waiting 30 real seconds, verify decreasing fractional Durations for each Herdr/tmux subprocess, no runner call when zero remains before execution, expiry before transfer, ordinary layout Failed versus Timeout, 5-second close, and DB compensation after failure. Also check that bounded paths do not leak into legacy unbounded runners, and connect the Step 3 carryover to real DB/event/lock probes for both member and fleet. Reuse or add short real-process regressions to show SystemRunner’s new Duration entry point uses the existing process core, without overly strict wall-clock bounds. Verify preparation’s autocommit/no rows created, byte-identical argv for all three coding backends, no brace expansion in name/model and similar fields, and existing validation ordering. Use the same monitor driver for claim rejection, each signal-registration failure, partial registration, startup write/flush/tick/clear failure, normal stop, and owner replacement; pin one unregister per handle, primary-error retention, clear-only errors, and preservation of the next owner/wake ledger. No global signal mutation. Capture tests cover independent expected hashes, ANSI/CR/Unicode/empty input and exact timestamps, both CLI guard orders, scan continuation/error-null/order/text bytes, and no text-closure calls for JSON. Reuse equivalent existing tests; stage tests for unimplemented APIs in modules and report connection points/unexecuted counts. Run existing-API regressions first. Edit test bodies only; no production/Cargo configuration/git operations. No E2E/cli_server or browser/server work is needed.

Execution history (director): Docs first — Programmer: reflect §7 in public docs/SPEC and propose finalized APIs/observation points for Phase A under this heading. Specify MemberCreateOptions/FleetCreateOptions, preparation of prompt/backend/cwd/env/argv before DB transactions and separation of ID-dependent callbacks, a monotonic 30-second deadline for the entire callback (pass remaining time to each Herdr list/split/run/resize and tmux split/layout; never reset it), and a separate 5-second compensation-kill budget. Preserve existing 5-second DB busy timeout, atomic commit/validation order/owner-slot close order; do not claim DB waits or OS suspension fit within 30 seconds. Show per-call clock/runner seams that demonstrate the Step 3 timeout carryover below, connected to real-completion events in existing CreationHooks. Define MonitorLease ownership immediately after claim success, retention/unregistration of each successful signal handle, all later registration/startup write/flush/tick/clear failures, normal stops and owner changes, primary-error retention with clear suffix; propose observation entry points avoiding process-global signal/clock mutation in tests. CaptureSnapshot::from_raw(raw,ansi,now) and text/JSON presenters preserve ANSI/CR/Unicode/empty input, final-content UTF-8 hash, existing timestamp/lines/error-null/text, and avoid constructing text headings for JSON. This update is contract docs and concrete API proposals only; no production/test changes or git operations. Check docs build/links and report. No E2E/cli_server execution or browser/server startup is needed.

Execution history (director): Mandatory Step 3 carryover — After introducing the shared deadline, test backend kill→broker rollback for Herdr run timeouts with a known ID, and Unknown diagnostic→rollback without guessed kills for split timeouts before an ID is obtained. Assert primary-timeout diagnostic retention, remaining compensation after close failure, and no double cleanup on both member/fleet paths. Verify measurement of the shared 30-second deadline and 5-second compensation budget and propagation of remaining time to each call. Do not substitute out-of-contract RunError::Timeout injection with timeout=None for demonstrating deadline behavior.

Execution history (programmer): Spawn API proposal — This is an unimplemented, docs-only proposal. Types/functions are pub(crate) unless stated otherwise; make cli::creation/member/fleet/monitor visible within the crate so staged tests can reach them. Preserve the public CommandRunner::run(argv, Option<u64>) and existing Multiplexer methods. Put the new Duration path in a separate trait so existing fakes need no additional methods. Standard SystemRunner and real tmux/herdr/AnyMultiplexer implement both paths; CLI create always calls the bounded path.

```rust
// multiplexer/spawn.rs (new); std::time::{Duration, Instant}
trait MonotonicClock { fn now(&self) -> Instant; }
trait TimedCommandRunner {
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError>;
}
struct Deadline { /* private: absolute monotonic end */ }
impl Deadline {
    fn after(clock: &dyn MonotonicClock, budget: Duration) -> Self;
    fn remaining(&self, clock: &dyn MonotonicClock) -> Result<Duration, RunError>;
}
struct SpawnExecution<'a> {
    pub clock: &'a dyn MonotonicClock,
    pub runner: &'a dyn TimedCommandRunner,
}
struct PaneSpawnRequest<'a> {
    pub reference: &'a MultiplexerContext,
    pub env: &'a [(String, String)],
    pub command: &'a [String],
    pub cwd: Option<&'a std::path::Path>, // Some for herdr; tmux ignores it
}
trait SpawnMultiplexer: Multiplexer {
    fn split_prepared(&self, request: &PaneSpawnRequest<'_>, deadline: &Deadline,
        execution: &SpawnExecution<'_>) -> Result<String, MultiplexerError>;
    fn kill_pane_with_deadline(&self, pane_id: &str, ignore_missing: bool,
        deadline: &Deadline, execution: &SpawnExecution<'_>) -> Result<(), MultiplexerError>;
}

// cli/creation.rs; borrowed values are owned by the parsed CLI invocation.
struct MemberCreateOptions<'a> {
    pub fleet_id: i64, pub name: &'a str, pub description: &'a str,
    pub explicit_agent: Option<&'a str>, pub model: Option<&'a str>,
    pub effort: Option<&'a str>, pub monitor: bool,
    pub prompt: Option<&'a str>, pub file: Option<&'a str>,
}
struct FleetCreateOptions<'a> {
    pub name: &'a str, pub agent_name: &'a str,
    pub monitor_file: &'a str, pub monitor_model: Option<&'a str>,
}
struct SpawnPreparation<'a> {
    pub cwd: &'a dyn Fn() -> std::io::Result<std::path::PathBuf>,
    pub env: crate::config_dir::EnvLookup<'a>,
}
struct PreparedSpawn {
    pub prompt_template: String, pub argv_prefix: Vec<String>,
    pub coding_agent: String, pub env: Vec<(String, String)>,
    pub cwd: Option<std::path::PathBuf>,
}
impl PreparedSpawn {
    fn render(&self, fleet_id: i64, member_id: i64, director_id: i64)
        -> Result<Vec<String>, CafleetError>;
}
// Separate existing precondition ladders delegate only the already-validated
// prompt/backend argv/env/cwd preparation to a shared private helper.
// cli/member.rs
fn create_with_options<M: SpawnMultiplexer>(
    conn: &mut Connection, options: &MemberCreateOptions<'_>,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe, preparation: &SpawnPreparation<'_>,
    execution: &SpawnExecution<'_>, hooks: &dyn CreationHooks,
) -> Result<serde_json::Value, CafleetError>;
// cli/fleet.rs
fn create_with_options<M: SpawnMultiplexer>(
    slot: &mut Option<Connection>, options: &FleetCreateOptions<'_>,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe, preparation: &SpawnPreparation<'_>,
    execution: &SpawnExecution<'_>, hooks: &dyn CreationHooks,
) -> Result<serde_json::Value, CafleetError>;
```

PreparedSpawn relies on all three current backends’ argv contract placing the prompt last: build with an empty prompt and separate only that final slot to create argv_prefix. Expand placeholders only in the prompt, without interpreting braces in name/model/effort. Verify final argv is byte-identical to the existing builder. Read cwd only for Herdr, adding no new cwd failure condition to tmux. Snapshot only CAFLEET_DATABASE_URL once per create through env lookup; read cwd/env and construct argv prefixes before transactions/registration, while leaving placeholder syntax errors in their existing post-ID compensation path. Propose adding default-noop `fn prepared(&self, conn: &Connection, plan: &PreparedSpawn) {}` to existing CreationHooks, allowing observation of autocommit/row counts in a real callback after preparation but before registration/BEGIN. Existing BootstrapEvent::Begun/CommitFinished/RollbackFinished and CleanupEvent::PaneKillFinished/DeregisterFinished/GuardDisarmed retain their real-operation completion positions.

Execution history (programmer): Deadline decision request — Rounding remaining time to the existing run API’s u64 seconds would exceed 30 seconds or time out early, so propose the independent TimedCommandRunner Duration argument above. SystemRunner delegates both existing run and run_for to the shared process core, preserving integer-second APIs/existing wire diagnostics. Create one 30-second deadline immediately after the fleet callback begins or member registration succeeds, before placeholder expansion. Obtain remaining time before every subprocess; if zero or less, return Timeout without spawning, with no fractional-second rounding. A fake clock uses fixed Instant plus Cell<Duration>; the fake timed runner records argv/actual Duration and advances elapsed time. If required time is at least the remaining budget, it advances only that budget and returns Timeout. Do not inject invented Timeout for None. Arm the guard immediately after obtaining the ID from a successful response, and check the deadline again before transfer. Request approval to continue ignoring ordinary Failed from best-effort layout/resize, but treat deadline exhaustion/Timeout as creation failure and compensate the known pane (tmux layout timeout is also not success). New Duration timeout diagnostics retain `<backend> command timed out after <seconds>s: <argv>`, with seconds expressing the Duration actually passed, using decimal seconds (existing integer notation for integers, no unnecessary trailing zeros). Propose `<backend> spawn deadline exceeded: <argv>` for exhaustion before a call and `<backend> spawn deadline exceeded` before transfer, preserving the existing CLI tmux split-window failed prefix/exit.

Execution history (programmer): 5-second compensation budget decision request — Current Herdr kill_pane performs optional pane get→close→optional layout repair; if get consumes the budget, the essential close cannot start. For new bounded compensation kills only, propose prioritizing direct `herdr pane close <id>` / `tmux kill-pane -t <id>`, assigning the entire 5 seconds to forced close with no cosmetic relayout. Preserve existing kill_pane get/close/rebalance behavior for normal delete and similar operations. Create one independent 5-second Deadline when backend or CLI guard cleanup actually starts, and disarm even on failure to prevent double kills. Preserve existing PaneCleanup Attempted/Unknown and primary-error-plus-cleanup suffix. Verify member known run timeout→backend close→deregister, fleet known run timeout→backend close→broker rollback, unknown split ID timeout→Unknown→DB compensation without kill, and SQL failure after callback success→rollback→owner slot take/drop→CLI close through actual clock/runner/CreationHooks/DB lock acquisition. Separately assert that remaining deregister/rollback work executes even if close returns Timeout/Failed. Where the API addition requires new SpawnMultiplexer in existing CLI creation fixtures, the Tester migrates them in Phase B (no production fallback with fake trait defaults that ignore deadlines). Existing backend non-create tests can continue using old methods. Old create_with_connection/create_with_dependencies delegate to the shared core only as temporary wrappers while referenced tests migrate; retain no separate compensation implementation.

Execution history (programmer): MonitorLease/test entry-point proposal — Claims use real broker SQL and clear failures use real DB triggers; only signals/clock/sleep are replaced per call. The following types/fields are pub(crate). Own each SignalHandle in a Vec immediately after successful registration, unregistering once during finish/drop. The lease holds &mut Connection and lends that same connection to ordinary ticks. Explicit finish unregisters all handles then conditionally clears; Drop makes a final attempt at the same cleanup only if not finished. Registration rejection creates no lease/handles/clear.

```rust
// monitor/mod.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorSignal { Terminate, Interrupt }
trait MonitorSignalHandle {
    fn unregister(self: Box<Self>) -> bool;
}
struct MonitorLoopHooks<'a> {
    pub pid: i64,
    pub stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub now: &'a dyn Fn() -> chrono::DateTime<chrono::Utc>,
    pub register: &'a dyn Fn(MonitorSignal, std::sync::Arc<std::sync::atomic::AtomicBool>)
        -> std::io::Result<Box<dyn MonitorSignalHandle>>,
    pub sleep: &'a dyn Fn(u64, &std::sync::atomic::AtomicBool),
    pub observe: &'a dyn for<'event> Fn(MonitorEvent<'event>),
}
enum MonitorEvent<'a> {
    Claimed { conn: &'a Connection },
    SignalRegistered { signal: MonitorSignal },
    SignalUnregistered { signal: MonitorSignal, removed: bool },
    StartupWriteFinished { result: &'a std::io::Result<()> },
    StartupFlushFinished { result: &'a std::io::Result<()> },
    TickFinished { conn: &'a Connection, result: &'a Result<TickResult, CafleetError> },
    ClearFinished { conn: &'a Connection, result: &'a Result<(), CafleetError> },
}
fn run_monitor_loop_with_hooks(
    conn: &mut Connection, mux: &dyn MonitorMux, out: &mut dyn std::io::Write,
    fleet_id: i64, tick_seconds: i64, wake_interval: i64,
    hooks: &MonitorLoopHooks<'_>,
) -> Result<(), CafleetError>;
// MonitorLease implementation remains private; all paths exercise it through
// this driver. Existing run_monitor_loop signature delegates with real hooks.
```

All events occur after real operations return. Claimed exposes the retained conn so tests can install clear-specific UPDATE triggers or tick-specific triggers, rather than substitute an invented clear callback for SQL. For replacement owners, update pid through TickFinished’s real conn and verify Stop on the next tick and no-op clear. Registration closures can return independent fake handles and set the stop flag, so tests do not manipulate SIGTERM/SIGINT or real signal_hook. Standard hooks connect to OS pid/UTC/current interruptible_sleep/real signal_hook registration and unregistration. Propose `stdout flush failed: <io error>` for startup flush failure; when both the main operation and clear fail, append `cleanup failed for monitor runtime (fleet <id>, pid <pid>): <clear error>` to the primary error. Success plus clear failure returns the original clear error. Do not expand existing best-effort flush after tick into another contract in this step. Verify all paths—after registration but before write/flush, after ticks, normal stop, and owner change—through the same driver, also checking that repeated invocation does not accumulate handles.

Execution history (programmer): CaptureSnapshot/presenter entry-point proposal — Share a small typed snapshot from a new capture module between the two CLIs. Keep existing capture windowing and lines validation at the same backend/CLI locations. Types/fields/functions are pub(crate); ScanEntry.kind uses existing MemberKind and only error annotations use String. Do not read results back from JSON.

```rust
// capture.rs (new)
#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSnapshot {
    pub content: String, pub captured_at: String, pub content_sha256: String,
}
impl CaptureSnapshot {
    fn from_raw(raw: &str, ansi: bool, now: chrono::DateTime<chrono::Utc>) -> Self;
}
struct MemberCapture {
    pub member_id: i64, pub pane_id: String, pub lines: i64,
    pub snapshot: CaptureSnapshot,
}
struct ScanEntry {
    pub member_id: i64, pub name: String, pub kind: crate::broker::records::MemberKind,
    pub coding_agent: String, pub pane_id: Option<String>, pub lines: i64,
    pub outcome: Result<CaptureSnapshot, String>,
}
// cli/member.rs; executes existing mux-first targeting/capture guards.
fn capture_with_dependencies<M: Multiplexer>(
    conn: &Connection, member_id: i64, lines: i64, ansi: bool,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    now: &dyn Fn() -> chrono::DateTime<chrono::Utc>,
) -> Result<MemberCapture, CafleetError>;
// cli/monitor.rs; executes live-fleet-first guards and real roster selection.
fn scan_with_dependencies<M: Multiplexer>(
    conn: &Connection, fleet_id: i64, lines: i64, ansi: bool,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    now: &dyn Fn() -> chrono::DateTime<chrono::Utc>,
) -> Result<Vec<ScanEntry>, CafleetError>;
// presentation.rs
fn member_capture(capture: &MemberCapture) -> serde_json::Value;
fn scan_json(entries: &[ScanEntry]) -> serde_json::Value;
fn scan_text(entries: &[ScanEntry]) -> String;
// capture.rs; CLI uses these dispatchers with stdout and scan_text.
fn write_member_capture(out: &mut dyn std::io::Write,
    capture: &MemberCapture, json: bool) -> Result<(), CafleetError>;
fn write_scan(out: &mut dyn std::io::Write, entries: &[ScanEntry], json: bool,
    text: &dyn Fn(&[ScanEntry]) -> String) -> Result<(), CafleetError>;
```

Call now exactly once immediately after each successful read, never for pending/capture errors; those create no snapshot. Assert strip_ansi CSI/CR processing, raw ANSI, Unicode, empty strings, and independently computed SHA256/exact fixed-UTC values, reusing existing CLI JSON field order/null and text-byte contracts. write_scan’s standard text argument is real scan_text; JSON tests pass a panicking text closure to prove it is not called (do not add separate heading construction on the JSON path). out can be an injected Vec<u8>/failing writer. Preserve Director-first/other active-placement ascending order in scans, continued capture after one pane fails, exit 0 even if all captures fail, and echoed lines. Stage new-API tests in separate modules for Phase B connection. Run executable existing CLI integration regressions first in Phase A, excluding unexecuted new-API cases from pass counts.

- [x] Document §7 and introduce create options/shared spawn preparation; implement and verify the shared 30-second bootstrap deadline and 5-second compensation budget. <!-- completed: 2026-09-06T07:47 -->
- [x] Implement MonitorLease and signal-handle management; verify injected registration/startup write/flush/tick/clear failures and owner replacement. <!-- completed: 2026-09-06T07:34 -->
- [x] Share CaptureSnapshot and text/JSON presenters; verify content/time/hash contracts for ANSI/CR/Unicode/empty input. <!-- completed: 2026-09-06T07:34 -->

### Step 9: Simplify fleet selection and asynchronous fetching

Execution history (director): Step 9 is complete — Confirmed location.replace for automatic fallback and hash navigation for explicit selection/Back/Close. Reconfirmed 75 passing Node tests/tsc, lint, admin/docs builds, and zero failures across 29 links/23 fragments. Production/public docs committed as 092ff923; test placement/config as a3b25eeb. No new dependencies. Verification remains Node execution for resource/client/route and review/typecheck/build for React wiring. Design doc/evidence are uncommitted; browser/E2E/server checks were not run. Progress 24/36.

Completed navigation correction 1219. Converted App’s invalid-hash normalization and missing-fleet fallback after successful lists, Dashboard’s invalid/missing member and resource missing-fleet handling, Timeline/MemberDetail missing-fleet handling, and MessageInput/AppHeader’s current-mutation missing-fleet handling to location.replace with a fragment, replacing the current history entry while preserving hashchange notification. App’s explicit Back/fleet selection and Dashboard’s member selection/explicit Close retain existing hash assignment. Added the distinction between automatic replacement and explicit navigation to SPEC and WebUI API. All 75 Node tests/tsc, admin lint (zero warnings), admin build, and docs build passed. The 29 local links / 23 fragments had zero failures. Evidence: .cafleet-evidence/programmer-step9/navigation-{tests,lint,build,docs-build,anchors,audit}.log. All five activated test hashes match; no test-body changes. No execution checks of DOM/Back interactions; navigation wiring was checked only through code review/type checking. Progress 24/36.

Final implementation and Node verification are complete. Implemented explicit FleetClient and whole-route/shared ID parsing, a single generation/AbortController/request token/pending/snapshot implementation in resource.ts, useSyncExternalStore wiring and refresh timers, migration of every UI consumer, and per-resource loading/error/Retry/stale data. Removed global selection/old unscoped APIs/initialMembers prefetch and old usePolling/useRefreshKeyLoad; old-reference audit found zero. Updated planned wording in SPEC and three public pages to implemented status.

`CI=true mise //admin:test` passed tsc -b and **6 files / 75 tests pass** (18 existing plus 57 revised Node staged cases; zero failures). `//admin:lint` had zero warnings/errors, `//admin:build` passed tsc/Vite, and `//docs:build` passed. The 29 local links / 23 fragments across three related public pages had zero failures. Moved five test files byte-identically to admin/tests according to the new placement table, replacing only the four history cases and leaving no old staged copies. All SHA rechecks matched. Evidence: `.cafleet-evidence/programmer-step9/{final-tests-2,final-lint-2,final-build,final-docs-build,final-anchors,old-api-audit}.log`, `activation.json`, and `final-test-hashes.json`.

The only configuration change adds already-installed node to types in tsconfig.app.json. New Node tests import node:timers/promises, causing TS2591; including existing @types/node in the type scope resolved it. No new dependencies, package/lock/Vitest configuration changes, or test-body corrections. Implementation changes cover api.ts/App.tsx, resource.ts/route.ts, hooks/useResource/useRefreshKey, components Dashboard/FleetPicker/Timeline/MemberDetail/MessageInput/AppHeader and shared ResourceNotice, deletion of the old hooks above, and public docs/SPEC.

Checked the following React wiring through code review and type checking/build. No DOM/render/click/actual navigation execution is claimed.

| Wiring | Production locations and evidence reviewed |
|---|---|
| Excluding old data on new key/load | useMemo at hooks/useResource.ts:12 creates a new resource synchronously; useSyncExternalStore at :13 returns its loading snapshot. No old-data reset effect. |
| Cleanup/StrictMode/stable refresh | start/stop at :16 in the same hook and resource/previous refreshKey comparison at :24 avoid extra pending work on initial/replay runs; the stable callback delegates to the current resource retained by the effect. Resource stop/restart/obsolete finally also passed Node tests. |
| URL authority/single roster owner | Only App.tsx’s hash listener and FleetView’s unscoped check resolve the route; selection handlers only navigate. FleetView/Dashboard are keyed by fleet ID; only Dashboard calls getMembers. Member-route-only changes do not recreate fleet checks/rosters. |
| Member/sender | Dashboard matches members through parsePositiveId plus a successful roster. Before success, it shows no missing-member redirect, empty state, claim that no Director exists, or send form. MemberDetail is keyed by member ID. |
| Missing-fleet detection | Navigate to the picker only when a successful fleet list lacks the target, or the current resource/mutation returns ApiError 404 with exact message Fleet not found. Other 404/500/network errors in roster/monitor/timeline/history/mutations retain individual errors. Resource generation/lifetime abort guards prevent old promises from changing the new route. |
| Independent fetching/5 seconds/Retry | Roster and monitor, inbox and sent use independent useResource instances. Only Dashboard/picker useRefreshKey owns a 5000ms timer, without stopping for visibility. Manual refresh/send/save/wake success increments the shared key; each resource’s Retry calls only its own refresh. History adds no timer. |
| Mutations/suppression of old drafts | MessageInput/AppHeader layout cleanup aborts the captured controller; post-await/catch/finally checks the signal to suppress old state/onSent/onSaved. Fleet keys discard form/monitor state; clients retain their captured creation IDs. Failed sends are not automatically retried, and abort does not cancel server commits. |
| Display contracts | ResourceNotice displays loading/initial errors/refresh errors and identifiable Retry controls; empty states appear only after success. Preserve Timeline grouping/scroll, MemberDetail’s independent history fetch of 201/display of 200/overflow-only notice, existing send parsing/ESC, and other behavior. These DOM displays/interactions were not executed. |

All three Step 9 tasks complete; Progress 24/36. Rust source was unchanged and the existing 711-test Rust suite was not rerun. No browser/E2E/server or git operations. The old API proposals/DOM counts below are design history; current verification is limited to the Node execution and React wiring review above.

Execution history (director): Reviewed the public-doc update for Node verification and committed it as 0ed80be7. Confirmed 29 local links/23 fragments/zero failures in node-docs-build/node-anchors. Only contributing.md changed; design doc/evidence are uncommitted. The Tester is working on revised Phase A; additional Programmer instructions follow its review.

Completed docs-only update 1194. Removed assumptions about DOM dependency installation/tsx activation from contributing’s Frontend resource tests, replacing them with Vitest Node tests of the real production resource and explicit limits of React wiring type checking/lint/build/review. Updated §8’s verification paragraph and task 3 to the same scope while preserving UX implementation requirements. `CI=true mise //docs:build` exited 0; 29 local links / 23 fragments across three related pages had zero failures (`.cafleet-evidence/programmer-step9/{node-docs-build,node-anchors}.log`). No production/test/package/lock/config changes or staged relocations. Awaiting revised Phase A; Progress 21/36. The old DOM infrastructure proposal/89 and 107 counts below are design history, superseded by the latest Node decision.

Execution history (director): Node API decision — Adopt the createResource/Resource/ResourceState proposal below. One implementation in resource.ts owns fetching, generations, request tokens, pending work, and snapshots; useResource wires its subscription/lifecycle. Construction/subscribe do not fetch; start while active is a no-op; stop invalidates→aborts and discards pending work; refresh while stopped is a no-op; restart fetches afresh from loading. getSnapshot preserves reference identity while state is unchanged; prevent notifications after unsubscribe and from obsolete promises. Handle synchronous throws through the same path as rejections. An initial AbortError keeps loading/no error and releases the fetch guard, allowing manual/next-refresh retry. AbortError with data retains data/refreshing=false/no error. Also adopt parsePositiveId(token: string): number | null, sharing ASCII/positive/safe-integer validation between parseHashRoute and member roster matching. From the render with a new key/load, the hook returns the new resource snapshot; StrictMode replay/initial refreshKey must create no extra pending fetch, and stable refresh callbacks delegate to the current resource. Explicitly document untested DOM/real React behavior as in the table below.


Execution history (programmer): Minimal production resource proposal for Vitest alone (response to 1189; unimplemented) — Cancel DOM dependency installation and do not retry it. Preserve existing FleetClient/RequestOptions/ApiError, strict route ID validation, the external useResource contract, and component props. Add only a small React-independent resource object whose snapshots/subscriptions/operations the real hook uses directly. Do not create a test-only controller, DOM imitation, or separate parallel async implementation inside the hook. This update edits only this design doc; no production/test/package/lock/config changes or staged moves.

```ts
// admin/src/resource.ts (new, production)
// Previously approved ResourceState<T> union moves here unchanged.
export type ResourceState<T> =
  | { status: "loading"; data: null; error: null; refreshing: false }
  | { status: "error"; data: null; error: Error; refreshing: false }
  | { status: "success"; data: T; error: Error | null; refreshing: boolean };

export interface Resource<T> {
  getSnapshot(): ResourceState<T>;
  subscribe(listener: () => void): () => void;
  start(): void;
  refresh(): void;
  stop(): void;
}
export function createResource<T>(
  load: (signal: AbortSignal) => Promise<T>,
): Resource<T>;

// admin/src/hooks/useResource.ts keeps its approved external signature.
export type { ResourceState } from "../resource";
export interface ResourceOptions<T> {
  key: string;
  load: (signal: AbortSignal) => Promise<T>;
  refreshKey?: number;
}
export function useResource<T>(options: ResourceOptions<T>): {
  state: ResourceState<T>;
  refresh: () => void;
};
```

createResource creates a loading snapshot without side effects. subscribe only adds/removes listeners and never starts fetching; getSnapshot returns the same object while state is unchanged. start begins one fetch only from the stopped state; duplicate start while active is a no-op. stop first invalidates the generation, aborts the active controller, and discards pending work, preventing old promises from affecting listeners/state/guards even if they ignore the signal and settle. stop itself sends no snapshot notification; refresh while stopped is a no-op. start after stop creates a new generation and fetches from loading, allowing reuse of the same resource for StrictMode setup→cleanup→setup. Resolve/reject/finally from an old start generation cannot interfere with the new one.

Refresh during a fetch only sets the pending boolean to true. When the current generation’s request settles with success/error/AbortError, release only that request; if pending, clear it and start one latest fetch. Do not queue each refresh or hook refreshKey change separately. Check both generation and request token so old finally handlers cannot release the current in-flight request or replay old pending work. Route synchronous load throws and rejections through the same failure path. Initial errors set status=error; refresh after success retains data with refreshing=true/error=null; failure retains the same data with error/refreshing=false. A current AbortError publishes no new error, retains existing data, and releases the guard (returning refreshing=false when successful data exists). Subsequent poll/Retry/queued refresh can run. A stopped resource does not copy data to a new resource. Roster/monitor/timeline/inbox/sent each have independent resource instances, so one’s delays/failures do not propagate to another.

Real hook wiring — Create a side-effect-free resource per identity with `useMemo(() => createResource(load), [key, load])` and subscribe directly with `useSyncExternalStore(resource.subscribe, resource.getSnapshot, resource.getSnapshot)`. Return the new resource’s loading state from the first render with new key/load, avoiding a flash of old data while waiting for an effect reset. The lifecycle effect only calls resource.start and calls the same resource.stop in cleanup. The refreshKey effect calls resource.refresh only when the value changes from its predecessor within the same resource, without treating initial/new-resource/StrictMode replay as extra refreshes. The external refresh function is a stable callback delegating only to the current resource retained by the effect. Implement dependencies and reference updates using ordinary React hook rules; do not disable lint to permit mutable-ref manipulation during render. Only resource.ts owns generations/AbortController/pending/state transitions.

Verification boundaries follow. Do not claim Node tests substitute for executing React.

| Contract | Direct Vitest Node verification | Limits of component-wiring checks |
|---|---|---|
| Header/signal/body/error and concurrent A/B clients | Existing 19 staged FleetClient cases and 4 history replacement cases | Type checking/reference audit/review verifies that all consumers call their props client |
| Whole-route and positive safe integers | Existing 4 staged route cases. If needed, export the same pure ID parser used for member matching from route.ts | hashchange, Back/Forward, and real App redirects receive review only |
| Generation/abort/late resolve-reject-finally | Real createResource start→refresh→stop→start, independent A/B instances, promises ignoring abort | Hook key/load→new-resource wiring and unmount→stop receive type checking/review only |
| Coalescing/initial error/refresh error/retry/independence | Real snapshots/subscribers, controlled promises, two concurrent resources | Actual DOM alert/Retry/empty/skeleton display and clicks are untested |
| Single initial fetch | Zero calls on createResource construction/subscribe, one on start, duplicate start no-op, one on restart | App prefetch removal and one roster fetch per Dashboard mount receive code review only |
| 5-second hidden-tab/manual/post-send refresh | Through resource.refresh processing and coalescing only | useRefreshKey timer/visibility independence and props callback wiring receive type checking/review only. Do not report successful 5-second polling from Node tests |
| Suppression of old mutation callbacks/recipients/drafts | Through client-captured fleet/header/body and AbortSignal forwarding only | Keyed component cleanup, mutation-completion guards, and absence of mixed names/recipients/drafts receive review only. Do not claim execution of DOM cancellation/interactions |

Only if Phase A needs pure member-ID validation, propose exporting `route.ts`’s `export function parsePositiveId(token: string): number | null` and sharing it between that module’s parseHashRoute and Dashboard roster matching. This centralizes the agreed nonempty ASCII digits/positive/safe-integer/leading-zero allowance contract; no separate route coordinator is needed. Review strict fleet-not-found handling as ApiError status/message matching in consumers, without adding a navigation controller that imitates App solely for tests. Add no mutation coordinator or timer abstraction in this change.

Public-doc follow-up proposal — Rewrite contributing’s Frontend resource tests around existing Vitest Node and tests of real resource processing, removing jsdom/RTL/tsx activation/dependency-resolution wording. Split the design’s §8 test-environment paragraph and Step 9 task 3 between Node execution in the table above and React wiring review/tsc/build. Preserve WebUI API/how-to/SPEC UX requirements (omitting DOM execution does not waive implementation requirements). After Director arbitration, update public docs and run build/link checks. The planned 89/107 counts are withdrawn; recount from the Tester’s Node revision and actual results. The Tester owns reuse of the 27 existing Node staged cases, removal of DOM files/helpers, and addition of Node resource tests; move nothing until receiving the new placement table. No DOM changes to package/lock/Vitest configuration are needed. Final gates: `CI=true mise //admin:test`, `//admin:lint`, `//admin:build`, plus `//docs:build` and link checks for public-doc changes. No Rust/E2E/browser/server work.

Execution history (director): Change the test policy in response to the user’s request for Vitest-only verification. This overrides the jsdom/RTL dependency introduction and byte-identical DOM staged-test activation approval below. Cancel jsdom/@testing-library/react/@testing-library/dom additions; package/lock are currently unchanged. Verify API client/route/history and real production async state handling in existing Vitest’s Node environment. Do not test React DOM rendering/clicks/actual App navigation or claim equivalent verification from Node tests. Withdraw previous planned counts of 89/107; exclude the 62 DOM-dependent cases while still unexecuted. The Tester changes/removes staged tests; the Programmer must not rewrite assertions. E2E/browser/server work remains excluded.

Execution history (director): Programmer: remove the dependency-installation blocker and first return a minimal API proposal for Node verification under this heading. Existing FleetClient/route APIs and the external useResource contract may remain. If consolidating generation/AbortController/refresh coalescing/stale resolve-reject-finally suppression into small resource processing used by the real production hook is natural, provide concrete signatures and hook wiring. Do not add an independent test-only controller, homemade React/DOM imitation, or excessive abstractions solely for tests. Distinguish Node-verifiable contracts from those limited to component-wiring code review/type checking/build. Also record a proposal to remove DOM-infrastructure assumptions from public docs. This update is API/scope proposals only, with no test changes/moves or production implementation yet. After Director arbitration, the Tester updates Phase A for Node, then Phase B resumes.

Execution history (director): Begin Phase B — Reviewed Phase A’s 8 files/89 cases, confirmed all placement SHA256 values match and existing 22 tests/tsc/admin lint pass, and committed as 6fd39724. The 89 cases have not compiled/run; the expected discovery count after replacing four existing history cases is 107. Programmer: implement production according to the adopted APIs/§8 below. Add jsdom, @testing-library/react, explicit peer @testing-library/dom, and the minimum Vitest .tsx include. Move the eight admin/staged-tests files byte-identically to admin/tests according to .cafleet-evidence/tester-step9/placement.json (only history-api.test.ts replaces an existing file; leave no staged copies). If test contents need correction, return concrete compilation/fixture defects through the Director for the Tester to fix; do not weaken assertions.

Execution history (director): Implementation priorities are immutable explicit FleetClient, routes with URL hash as the sole selection state, independent fetching per resource, exclusion of old data from the generation-change render, suppression of old promises’ resolve/reject/finally and mutation callbacks even when abort is ignored, and coalescing in-flight refreshes into one latest request. Detect missing fleets only through absence in a successful list or the current generation’s exact Fleet not found 404. Validate member/sender after roster success. Preserve 5-second polling, manual/send/monitor-success refresh, initial-error versus empty distinction, per-resource Retry, and existing data after refresh failure. After all consumers migrate, remove global setFleetId/old unscoped APIs/initialMembers prefetched state/unneeded old hooks; retain no compatibility fallback. Verify with CI=true mise //admin:test, //admin:lint, //admin:build, plus //docs:build and local links when docs change. No need to rerun 711 Rust tests if Rust code is unchanged; if Rust changes become necessary, report why and run relevant regressions. No browser/E2E/server startup or git operations; design doc/evidence stay uncommitted. On completion, record executed counts, changed files, old-API reference audit, test-placement hashes, and needed decisions under this heading.

Execution history (tester): Revised Node Phase A is complete. Organized admin/staged-tests into 57 cases: 19 real-client, 4 history replacements, 7 route (adding 3 direct shared parsePositiveId cases), and 27 real createResource cases; 53 new plus 4 replacements, with one deferred helper file. Removed the old 62 DOM cases and step9-dom.ts without running them; withdrew planned 89/107 counts. All 57 new-API cases remain uncompiled/unexecuted; existing 22 tests/tsc and admin lint including staged files pass (under BASE: .cafleet-evidence/tester-step9/{node-baseline-test.log,node-admin-lint.log}). Placement/SHA/static counts/verification limits are in that directory’s placement.json and README.md. Move five files byte-identically to admin/tests in Phase B, replacing only history. Existing 18＋57＝75 is an expected discovery count, not an execution result. No production/package/lock/config/git changes. Resource tests assert real production API construction/subscription with zero fetches, start/restart, snapshots/unsubscribe, independence, throw/unknown/error/AbortError, coalescing, stop/old resolve-reject-finally/pending discard. React/StrictMode/actual navigation/DOM display/clicks/5-second hidden-tab timers/mutation callback and recipient/draft suppression are not executed; hook/component wiring is limited to type checking and review. No test-only controller/React imitation or browser/E2E/server startup. Return compilation/fixture defects to the Tester through the Director.

Execution history (director): Phase A/API decision — Adopt the FleetClient/RequestOptions/ApiError, useResource discriminated state/useRefreshKey, parseHashRoute, and component Props proposals below. Client fleet IDs must be positive safe integers; route fleet/member IDs must validate from nonempty ASCII digits to positive safe integers, allowing leading zeros. Member routes may retain raw strings, but roster matching uses the validated number; Number coercion must not permit exponents/signs/whitespace/overflow. Invalid members return to the fleet dashboard; invalid fleets/unknown paths return to the picker. Determine missing fleets only from absence in a successful fleet list or current-generation ApiError(404, 'Fleet not found'), not network/parse/server failures or monitor/member 404s. Return no old data from the first new-generation render; suppress old promises’ resolve/reject/finally and mutation callbacks too. Do not treat send abort as cancellation of server commits or automatically resend. Adopt independent resources, one coalesced refresh, 5-second hidden-tab polling, and resource-identifiable Retry controls.

Execution history (director): Tester: begin Phase A. Cover all real-client methods/headers/bodies/signals/errors, concurrent A/B client immutability, real-hook generation changes/ignored abort/reversed resolve and reject/finally/retry/stale refresh errors/coalescing, and real component/App single roster fetch/deep links/BackForward/initial error versus empty/member guards/prevention of mixed recipients/drafts/suppression of old send/monitor mutation callbacks/5-second/manual/post-send refresh. Also verify resource independence with delayed promises. Put new-API tests in committable `admin/staged-tests/` and move unchanged to `admin/tests/` in Phase B (.cafleet-evidence holds only logs/placement tables, not canonical tests). Prepare the Step 7 history-api.test.ts replacement for the new client in the same staged directory, retaining limit201/header/error checks and explicitly documenting that only the omitted-header check moves to listFleets. Keep the existing 22 TS tests executable during Phase A. Approve minimal infrastructure with jsdom/RTL/explicit dom peer, test.tsx includes, and jsdom environment only for DOM files; installation is the Programmer’s Phase B responsibility. No production/package/lock/git work now. Report uncompiled/unexecuted counts and connection points separately from regressions run against existing APIs. Do not start browser/E2E/server work.

Completed Step 9 docs-only preparation. Reflected planned contracts in SPEC §6.8, webui-api’s frontend-resource-lifecycle, WebUI how-to’s loading-and-retrying, and contributing’s Frontend resource tests. `CI=true mise //docs:build` exited 0; checked 29 local links / 23 fragments across three changed public pages against generated HTML IDs, with zero failures (`.cafleet-evidence/programmer-step9/{docs-build,anchors}.log`). The concrete API/route interpretation/mutation-completion suppression/test-migration proposals below await Director adoption. No production/test/package/lock changes, dependency installation, git operations, or E2E/browser/server startup. Step 9 is unimplemented, so all tasks remain unchecked and Progress stays 21/36.

Execution history (programmer): Phase A API proposal (docs-only, unimplemented) — Remove current App roster prefetch, global fleetId, and refreshes lost through shared inFlight in usePolling/useRefreshKeyLoad. Request adoption of the following proposal. Add only an explicit client and small real React resource hook/refresh timer as production abstractions; no separate test-only controller or fetch library.

```ts
// admin/src/api.ts; existing response types remain in types.ts.
export interface RequestOptions { signal?: AbortSignal }
export class ApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string);
}
export interface FleetClient {
  readonly fleetId: number;
  getMembers(options?: RequestOptions): Promise<MembersResponse>;
  fetchTimeline(options?: RequestOptions): Promise<TimelineResponse>;
  fetchInbox(memberId: number, options?: RequestOptions): Promise<TimelineResponse>;
  fetchSent(memberId: number, options?: RequestOptions): Promise<TimelineResponse>;
  getMonitor(options?: RequestOptions): Promise<MonitorRuntime>;
  sendMessage(fromMemberId: number, toMemberId: number | "*", text: string,
    options?: RequestOptions): Promise<void>;
  patchMonitor(wakeIntervalSeconds: number, options?: RequestOptions): Promise<void>;
  postMonitorWake(options?: RequestOptions): Promise<{ wake_requested_at: string }>;
}
export function createFleetClient(fleetId: number): FleetClient;
export function listFleets(options?: RequestOptions): Promise<FleetListItem[]>;
// No exported setFleetId or unscoped versions of scoped methods in final code.

// admin/src/hooks/useResource.ts
export type ResourceState<T> =
  | { status: "loading"; data: null; error: null; refreshing: false }
  | { status: "error"; data: null; error: Error; refreshing: false }
  | { status: "success"; data: T; error: Error | null; refreshing: boolean };
export interface ResourceOptions<T> {
  key: string;
  load: (signal: AbortSignal) => Promise<T>;
  refreshKey?: number; // default 0
}
export function useResource<T>(options: ResourceOptions<T>): {
  state: ResourceState<T>;
  refresh: () => void;
};

// admin/src/hooks/useRefreshKey.ts; replaces legacy polling trigger guards.
export const POLL_INTERVAL_MS = 5000;
export function useRefreshKey(intervalMs?: number): {
  refreshKey: number;
  refresh: () => void;
};
// Default interval: POLL_INTERVAL_MS. No fetch or initial increment here.

// admin/src/route.ts; used by real App hashchange handling.
export type Route = { kind: "fleets" } | {
  kind: "dashboard"; fleetId: number; memberId?: string;
};
export function parseHashRoute(hash: string): Route;
```

Propose that clients capture their positive safe-integer creation ID in a closure and reject invalid arguments with RangeError("Invalid fleet ID"). Routes accept the current hash/optional leading slash, validate ASCII fleet digits→positive safe integer (leading zeros allowed), and match the entire dashboard path. Normalize invalid fleets/unknown paths to fleets. Retain member IDs as raw strings and match strictly as numbers against member_id in a successful roster (invalid members also return to that fleet’s dashboard). App replaces invalid routes with #/fleets and also returns to the list when a successful fleet list lacks the target. Keep the route on network/JSON parse/HTTP errors. listFleets has no fleet header; all scoped methods attach X-Fleet-Id with the captured ID and pass each options.signal unchanged to fetch. Preserve history limit=201, POST/PATCH bodies, and void/wake return values. ApiError retains HTTP status and current error→detail→HTTP status fallback messages; network/AbortError pass through unchanged. Do not treat every 404 as fleet disappearance. Distinguish scoped ApiError(404,"Fleet not found") as fleet disappearance from absent monitor runtime or members, which remain resource errors.

The hook treats key or load identity changes as new generations; callers stabilize load with useCallback. A new generation returns loading rather than old data from its first render. Each generation owns one active request/AbortController and one pending boolean. Mount starts once; refreshKey changes or refresh() start a request when idle and only set pending=true while in flight. When the current request settles, apply success/error and release its guard, then clear pending and start the latest load once if needed. New-generation/cleanup handling invalidates first, aborts, and discards pending work. Generation checks ignore even promises that resolve/reject/finally despite abort, without releasing/restarting another current request. A current AbortError only releases the guard and processes pending work without changing the display, leaving the next poll/Retry possible. Refresh starts with success.data retained and refreshing=true/error=null; failure retains success.data plus error with refreshing=false; only initial failure sets status=error. Always normalize errors to Error. Keep refresh() identity stable.

useRefreshKey is a synchronous trigger starting at 0 and incrementing functionally on each 5-second interval or refresh(); it owns no inFlight state and fetches nothing. Dashboard passes this key to roster/monitor/Timeline/MemberDetail; the picker uses an independent instance of the same hook. Since only each useResource’s initial effect starts fetching, the timer does not increment on mount. Add no visibility listener and clear the interval on unmount. Roster/monitor and inbox/sent use separate useResource instances; delays/failures in one do not affect another’s state/polling. App checks fleet existence through a useResource per route fleet ID (no periodic timer). After confirmation, mount keyed Dashboard; there is no duplicate fetch beyond its initial roster. Retry refreshes the relevant resource; overall Refresh/send success/monitor save or wake success increments Dashboard’s refreshKey.

Execution history (programmer): Real component test entry-point proposal — Preserve existing default exports with the props below. No additional App dependency injection is needed: `render(<App />)` plus real window.location.hash/hashchange and vi.stubGlobal("fetch", fake) exercises the real client/route/component/hook. Dashboard and descendants can also render directly, passing createFleetClient(id) or a per-test fake satisfying FleetClient through props.client.

```ts
// Existing default exports; exported Props types for fixtures.
export interface DashboardProps {
  client: FleetClient; fleetName: string | null; memberId?: string;
  onBack: () => void;
}
export interface TimelineProps {
  client: FleetClient; members: Member[]; refreshKey: number;
}
export interface MemberDetailProps {
  client: FleetClient; member: Member; refreshKey: number; onClose: () => void;
}
export interface MessageInputProps {
  client: FleetClient; senderId: number | null; members: Member[]; onSent: () => void;
}
// AppHeaderProps gains client?: FleetClient; absent only for unscoped picker.
// MonitorIndicator receives that explicit client whenever monitor controls render.
// App has no props. FleetPicker keeps its existing onSelect callback; it only navigates.
```

App does not remount Dashboard when only the member route changes within a fleet; key=client.fleetId changes only on fleet switches. Pass the verified fleet name with the same-ID client. The member panel inside that Dashboard uses key=member.member_id. During initial roster loading/error, show no missing-member redirect, assertion that the fleet is empty, or old-recipient form; only successful rosters determine member selection and active Director send eligibility. Resource errors have role=alert and identifiable Retry buttons (for example aria-label="Retry timeline" / "Retry inbox" / "Retry sent" / "Retry members" / "Retry monitor" / "Retry fleets"); initial errors must not show a successful empty state. Refresh errors display previous data and indicate update failure. Loading regions identify resources through aria-label; tests assert visible text/roles/buttons and API calls rather than CSS classes. Preserve successful empty displays, timeline grouping, member history capped at 200 with notices only on overflow, scroll/ESC/send parsing, and other existing features.

Mutations also receive their fleet client through props; switching A→B during a send does not change A’s header/recipient to B’s. Keyed component cleanup aborts its own signal and ignores later results/notification callbacks (abort does not cancel an existing server-side send commit and must not trigger resending). Call onSent/onSaved only for ordinary success in the same fleet. New fleets retain no old drafts/candidates/monitor editing state.

Execution history (programmer): Minimal test infrastructure/migration proposal — Keep admin’s existing Vitest 5/React 19/Vite 8 setup; in Phase B, add only jsdom and @testing-library/react as devDependencies (explicitly adding its @testing-library/dom peer). Do not require user-event/jest-dom; use RTL fireEvent and standard Vitest matchers. Verify exact compatible versions and lock resolution during installation; make no package/lock changes or dependency installs during this docs-only stage. Expand vitest.config.ts include to tests/**/*.test.{ts,tsx}; keep node as default and specify `// @vitest-environment jsdom` only atop DOM files. tsconfig.app.json already includes tests and react-jsx, so should generally need no change. Explicitly provide RTL cleanup, restoration of fake timers/global mocks, and minimal fixtures for missing DOM scrollIntoView/matchMedia in tests. Use real useResource/renderHook rather than testing a separately implemented controller.

For Phase A, propose staging new-API hook/component/client tests in `.cafleet-evidence/tester-step9/staged/`, recording uncompiled/unexecuted counts and connection points. Keep existing 22 TS tests executable; do not require production stubs or package edits from the Tester. After Director approval, the Programmer handles dependencies/config and unchanged staged-file placement in Phase B, implementing the new APIs. Provide a Tester correction cycle to migrate the four Step 7 history-api.test.ts cases to createFleetClient(7).fetchInbox/Sent. Since no scoped call will have an unset-header case, retain error wording through an explicit-client 422 and move the omitted-header assertion to unscoped listFleets (explicitly documenting the contract change). To remove the old global API finally, recommend preparing migration in advance as staged-test replacements that can be placed alongside the new implementation; leave no global fallback in production.

Phase A coverage includes headers/signals/bodies/errors for all eight real-client methods plus unscoped listFleets, concurrent A/B client immutability, hook reversed resolve/reject/finally/unmount/abort/initial Retry/stale refresh/error/one coalesced consecutive refresh, 5-second/hidden-tab/manual/post-send refresh, App selection→one roster fetch (excluding aborted attempts in StrictMode), deep links/BackForward/fleet absence versus network failure/member guards, and no mixed fleet names/recipients/drafts after switching. Include delayed promises that ignore signals. Inject failure into each resource and verify through real components that empty states are not shown incorrectly. After introducing the infrastructure, verify with `CI=true mise //admin:test` (currently tsc -b plus vitest run), `//admin:lint`, and `//admin:build`; no E2E/browser/server.

Execution history (director): Docs first — Programmer: reflect §8’s URL route authority, createFleetClient(fleetId) and unscoped listFleets, fetching responsibilities for App/fleet existence, Dashboard/roster plus monitor, Timeline/MemberDetail/each history, and initial/refreshing/refresh-failure/abort UX in public docs/SPEC. Specify migration away from global setFleetId and initialMembers prefetch, fleet/member keys, AbortController plus generation IDs to ignore stale response/finally and coalesce refreshes into one latest request, preserved 5-second polling and manual/post-send refresh, distinction between 404 and transport failures, retries, and prevention of mixed old fleet names/recipients. Propose explicit-client method/signal signatures, resource state/refresh coordinator (pure helper if needed), real React component/hook test entry points, minimal Vitest/jsdom/React Testing Library introduction, and migration order for existing Step 7 fetch tests under this heading. Verify §8 through unit/component tests without browser/E2E/server startup. Existing Vitest tests include only .test.ts, so propose only necessary .tsx/DOM additions; do not add test-only stub implementations or replace the fetch library. This update is contract docs/API/test-infrastructure proposals only, with no production/test/package/lock changes or git operations. Report after docs build/link checks. The design doc remains uncommitted.

- [x] Reflect §8 fetching responsibilities and error UX in docs; migrate to explicit fleet clients, URL authority, and one roster-load owner. <!-- completed: 2026-09-06T08:46 -->
- [x] Introduce AbortController/generations/refresh coalescing and loading/success/error states; implement retries and refresh-failure display. <!-- completed: 2026-09-06T08:46 -->
- [x] Verify real client/route/history/resource generations, reversed responses, stop/restart, coalescing, errors/retries with Vitest Node. Check React key/unmount/poll/refresh, deep links, invalid/deleted fleets, and suppression of old mutations/recipients/drafts through type checking/lint/build and code review. Explicitly document unexecuted DOM coverage. <!-- completed: 2026-09-06T08:54 -->

### Step 10: Make asset replacement recoverable

Director review completed: committed the test reduction as `f801ff2b` and production/public documentation changes as `11c3dc13`. The two test files were reduced by a net 417 lines; 40 representative tests and the subsequent lint run passed. The design document and evidence were excluded from commits. The earlier full gate and the limited gates after the fixes/reduction are treated separately; no additional full rerun was performed.

Execution history (tester): reduction requested in 1334 completed. The current Step 10 tests total assets37 + setup3 = 40 (46 before reduction). Removed two full checkpoint-matrix tests and Cut/interruption_cuts, one full event-order test, one artificial record-boundary test, and two redundant setup tests. Reinstallation coverage was reduced to one interruption at Active After ordinal2, one ordinary Fail after Prepared, and two repeated-interruption cases during normalization from Prepared/after rollback. Function names, comments, count output, and unused imports were cleaned up. Retained the remaining tests' file/DB-row/recovery-result assertions and representative regressions for symlinks, contention, real SQL failures, the Committed boundary, and the actual reinstallation defect.

Ran the specified `CI=true mise //cafleet:test --lib step10_contract_tests -- -- --test-threads=1` once: 40 passed/0 failed/485 filtered, 108.91 seconds. After it completed, ran `CI=true mise //cafleet:lint` sequentially; both clippy -D warnings and fmt passed. Evidence is under BASE at .cafleet-evidence/tester-step10/{reduction-tests.log,reduction-lint.log,reduction-format.log}. Updated the current counts/SHA in the existing placement.json. No implementation changes outside the two test files, new tests, production API changes, full-suite/CLI integration/docs-build reruns, or git operations. No production API was identified as unnecessary, and no new verification infrastructure was added.

**Regarding history before the reduction:** The 760-test gate below, the old assets41/setup5 SHAs, the initial-install 81 cuts/rollback 48 cuts, and earlier combination counts describe the state before reduction. Those matrices have been removed; old counts in matrix-counts.log or README are not current coverage. Refer to the 40 tests above and reduction logs/placement.json for current results.


Limited Phase C fix: restored `assets_half` to table preflight → HOME resolution → backend loop, restoring schema-error precedence for a missing/outdated table. Production and `assets_half_with_options` share private `assets_preflight` and `install_selected_assets`, with one table query per call. Seam signatures and existing test bodies were preserved. After the fix, `CI=true mise //cafleet:test --lib setup -- -- --test-threads=1` passed 18 tests, followed by `CI=true mise //cafleet:test --test cli_setup_doctor -- -- --test-threads=1` with 40 passed; after completion, `CI=true mise //cafleet:lint` passed clippy/fmt. Evidence: `.cafleet-evidence/programmer-step10/preflight-{unit,integration,lint,audit}.log`. As directed, neither all 760 tests nor the docs build was rerun after this limited fix.

Full gate before the preflight-order fix above: `CI=true mise //cafleet:test --lib --test cli_compensation --test cli_fleet --test cli_global --test cli_member --test cli_message --test cli_setup_doctor --test docs_sync --test monitor_uniqueness --test webui_routes -- -- --test-threads=1` completed with 760 passed/0 failed/0 ignored across 10 binaries: library531, compensation16, fleet15, global21, member35, message22, setup/doctor40, docs32, uniqueness18, HTTP routes30. The actual cargo argv received `-- '--test-threads=1'`; total duration was 643.96 seconds. The sequential `CI=true mise //cafleet:lint` run after completion also passed clippy all-targets -D warnings/fmt. Docs build passed; no missing targets among 130 local links/121 fragments. Evidence: `.cafleet-evidence/programmer-step10/{selected-serial,lint-final,docs-final,final-audit}.log` and `selected-counts.json`.

Pre-reduction placement SHA verification: assets41 tests had SHA `09ed6738439ae60a511c350799a48e4cb9469ca26cf72aecce3e6f796d2441e8`; setup5 tests had SHA `8d51101a8b2ac2f2852f742fa060f10e0447ab7cc382d392337d6f5e02fde57d`. These matched the Tester's placement table at that time, and both cfg(test) connections were checked. The Programmer did not change test bodies. Production/docs SHAs at the full gate were retained in `production-sha-before-preflight.json`; SHAs after the limited fix were updated in `production-sha.json`. The only production difference was `cli/setup.rs`; `preflight-audit.log` also confirmed that both staged tests and the setup unit-test bodies in that file were unchanged.

Replaced direct asset deletion/copying with per-backend stage/journal/backup replacement. `assets.rs` coordinates the real embedded plan and installation/recovery; `assets/{types,files,journal,locks,driver}.rs` provide typed boundaries, real filesystem operations that do not follow symlinks, validated journals, physical locking/intent discovery, and the shared installation/rollback/cleanup driver. `cli/setup.rs` shares the real writer adapter and fixed-order/selection loop; `diagnosis.rs`, `cli/helpers.rs`, `cli/doctor.rs`, and `presentation.rs` expose Incomplete facts while preserving existing healthy output. No dependencies, InvocationHooks extensions, or Step 11 runtime generation were added.

Evidence for production guard ordering: schema guard at `cli/mod.rs:149`, asset diagnosis on the same connection at 155, assets guard at 173, and command bodies from 176 onward. `diagnose_assets` validates all backend paths first, then checks readable DB rows against read-only journal/intent evidence. `stale_assets_guard` orders checks as all path errors → Incomplete → no-assets/stale. Doctor inspects filesystem evidence even without a connection/ledger/table and continues the other sections.

Verification history is consolidated in `.cafleet-evidence/programmer-step10/README.md`. Initial compile problems were resolved by the Tester/Programmer's SHA-representation corrections and the Tester's EnvLookup type annotation. The selected-1 library result of 518 passed/4 failed was resolved by moving only four existing setup factories to file-backed DBs, retaining their assertions. The selected-2 library result of 520 passed/2 failed involved missing existing runtime PID files; a standalone serial run of all 38 runtime tests passed, but this is not asserted to resolve the cause. Integration tests did not run in either failed run. Original logs and Director decisions are retained in `.cafleet-evidence/programmer-step10/coordination-before-final.md`.

Before reduction, the Tester owned the existing 37 tests and nine additional reinstallation tests, connected as listed in the placement table. The initial-install matrix measured 81 install cuts/48 rollback cuts; these 129 cases are not added to the Rust test count. A limited serial run of the nine added tests passed all nine (56.19 seconds). Completion output reported four Active boundaries, five same-call rollbacks, eight normalization retries, and ten repeated interruptions. Separate assertions also passed for standalone Prepared, two RollingBack interruptions, shared identity, and 16 negative variants plus an unrelated lock key. Initial-matrix results are not extrapolated to all reinstallation behavior.

Unverified scope: stage/backup placement under different parents on the same device was checked, but no actual separate device was provided. Installer process kills, power loss, and real fsync failures were not induced. Per-call Interrupt plus RAII release and recovery with a fresh connection are not equivalent to actual process-crash durability. Real SQLite trigger failures are verified separately from injected failures. No E2E/cli_server/browser/server, mounts, real user-configuration changes, HOME/CODEX_HOME overrides, or Programmer git operations were performed.

#### Implemented installer API and recovery contract

The following implementation contract incorporates the Director’s decisions. The public contract is reflected in [SPEC](../../SPEC.md) and [CLI recovery](../../docs/docs/spec/cli-options.md#assets-install-recovery); fixture boundaries are documented in [contributing](../../docs/docs/contributing.md#assets-installer-tests).

**Entry points and ownership.** In `assets.rs`, share the existing `agent_paths(env: EnvLookup, home: &Path, agent: &str)` and build the plan from the actual embedded SKILLS/PRESETS. Make the existing `AgentPaths` fields `pub(crate)` so fixtures can construct explicit paths. Borrow the caller-owned DB connection; the installer does not open another connection. Everything below is `pub(crate)` (including the listed fields of the types); each call borrows its hooks and maintains no global state.

```rust
fn prepare_install(paths: &AgentPaths, agent: &str, version: &str)
    -> Result<InstallPlan, CafleetError>;
fn execute_install(conn: &mut Connection, plan: &InstallPlan, hooks: &InstallHooks<'_>)
    -> Result<InstallOutcome, InstallFailure>;
fn recover_install(conn: &mut Connection, paths: &AgentPaths, hooks: &InstallHooks<'_>)
    -> Result<RecoveryOutcome, InstallFailure>;
fn inspect_install(paths: &AgentPaths) -> Result<Option<IncompleteInstall>, CafleetError>;
fn read_journal(path: &Path) -> Result<InstallJournal, CafleetError>;

struct InstallPlan {
    coding_agent: String, identity: PathBuf, version: String,
    entries: Vec<PlannedEntry>, // skills 0/1, preset if any, research last
}
struct PlannedEntry {
    kind: EntryKind, target: PathBuf, manifest: Vec<ManifestFile>,
    // embedded replacement bytes are private; no injected fake payload source
}
enum EntryKind { Skill(&'static str), Preset, ObsoleteResearch }
struct ManifestFile { relative_path: PathBuf, size: u64, sha256: String }
// Preset is one file with empty relative_path; research has no manifest/stage.

struct InstallHooks<'a> {
    lock_mode: LockMode,
    checkpoint: &'a dyn Fn(&InstallEvent) -> Result<(), InstallFault>,
}
enum LockMode { Wait, Try }
enum InstallFault { Fail(String), Interrupt }
struct InstallEvent {
    operation: InstallOperation, edge: Edge, entry: Option<usize>,
    path: Option<PathBuf>, journal: PathBuf, phase: Option<InstallPhase>,
}
enum Edge { Before, After }
enum InstallOperation {
    LockAcquire, StageWrite, StageValidate, JournalPersist, IntentPersist,
    BackupRename, InstallRename, RecordCommit, RemoveNew, RestoreBackup,
    RestoreRecord, CleanupStage, CleanupBackup, JournalRemove,
}
enum InstallFailure {
    Failed(CafleetError), Interrupted(InstallEvent), Busy(PathBuf),
}
struct InstallOutcome {
    installed_record: AssetInstallRecord,
    cleanup_pending: Option<IncompleteInstall>, recovered_only: bool,
}
enum RecoveryOutcome {
    None, RolledBack,
    Committed { installed_record: AssetInstallRecord, cleanup_pending: Option<IncompleteInstall> },
}
struct IncompleteInstall { identity: PathBuf, journal: PathBuf, cause: String }
```

`prepare_install` performs no I/O mutations and fixes the source/target mapping and manifest. Every `execute_install` call follows lock → recovery: after `RolledBack`, it proceeds with the new plan; for `Committed` with the same coding_agent/identity, that backend returns after cleanup only (validation uses the journal’s manifest/row even if the version differs from the new binary; the next explicit setup performs the new installation). Recovery of another identity discovered through shared targets completes first; its row is not treated as success for the current identity, and execution proceeds with the new plan. If cleanup for another identity remains incomplete, stop with an incomplete-install diagnosis without starting a new plan. `recover_install` uses the same internal driver/lock acquisition but performs recovery only. Preserve the existing `install_agent` signature as a thin adapter that calls the same `execute_install` with a no-op checkpoint/Wait and passes success output and cleanup warnings to the CLI. Tests call `execute_install`/`recover_install` directly rather than implementing a separate installer.

**Concrete journal format.** Format version `1`. JSON can be read/written through explicit conversion to/from the existing serde_json Value; no additional dependency such as serde is needed. Reuse the existing diagnosis type `AssetInstallRecord` (holding all four columns as String). Reading validates the format, phase, required fields, duplicate targets, and path layout; unrecognized formats or inconsistencies are not repaired automatically.

```rust
struct InstallJournal {
    format_version: u32, transaction_id: String, phase: InstallPhase,
    coding_agent: String, identity: PathBuf, database_path: PathBuf,
    previous_record: Option<AssetInstallRecord>, new_record: AssetInstallRecord,
    entries: Vec<JournalEntry>, pending: Option<JournalOperation>,
}
enum InstallPhase { Prepared, Swapping, Recording, RollingBack, Committed }
struct InstallIntent { transaction_id: String, journal: PathBuf, state: IntentState }
enum IntentState { Active, Finished }
struct JournalEntry {
    target: PathBuf, stage: Option<PathBuf>, backup: PathBuf,
    previous: Option<EntryFingerprint>, manifest: Vec<ManifestFile>,
    state: EntryState,
}
enum EntryState { Original, BackedUp, Installed, Restored }
struct EntryFingerprint { sha256: String } // type + relative names + bytes/link text
struct JournalOperation { operation: InstallOperation, entry: Option<usize> }
```

`previous: None` means the old entry did not exist. The fingerprint is computed from the entry type reported by symlink_metadata, regular-file bytes, the tree’s sorted relative entry names/types/bytes, and symlink read_link text; symlink targets are not read. Renames preserve existing entries, and fingerprints determine whether restoration has already occurred after a restart. Old/new rows retain all four columns, including timestamps rather than just versions; the new timestamp is created only once. Write all four new-record columns in an explicit transaction. Restoration likewise uses a transaction to restore all four old columns, or DELETE the row by its composite key if it did not previously exist. Do not use an existing record helper that regenerates timestamps for restoration.

The journal first durably records `pending`, then performs the real operation → syncs the relevant parent directories → durably records the state update/clears pending. The journal’s and intent sidecars’ temp → rename operations are this persistence primitive and are not recursively journaled. Stage file syncs and directory syncs also complete before swapping. Recovery checks real entries as well as records:

| State | Idempotent operation and determination after another interruption |
|---|---|
| `BackupRename` intent | If the backup exists, it has been moved aside. If no backup exists and the old target fingerprint matches, the operation has not occurred. Preserve evidence and stop if both are inconsistent or the old entry is lost. |
| `InstallRename` intent | If the stage is absent and the target matches the new manifest, installation has occurred. If the stage exists, roll back as a pre-install state. Do not overwrite the backup with the new target. |
| Backup exists during rollback | Save a `RemoveNew` intent and delete the new target as an entry (absence counts as success), then save a `RestoreBackup` intent and rename backup → target. Process entries in reverse order. |
| Interrupted after `RestoreBackup` but before the completion journal | Even if the backup is absent, a matching old-target fingerprint establishes that restoration is complete. If this cannot be determined, do not treat a missing old backup as success. |
| Old entry did not exist | Delete only a target that was installed or has an installation intent; absence on retry counts as success. |
| Restore old DB record | Repeating the exact UPSERT/DELETE yields the same result. Preserve the journal on failure. Only after all entries and the row are restored, remove remaining stages, mark intents Finished, and remove the journal. |
| Committed cleanup | Verify the new manifest and exact new row every time; absent backups/stages count as success. Durably mark all intents Finished and remove the journal last. On an intermediate failure, return `cleanup_pending: Some` without rollback. |

A stage failure before journal creation cleans up stages without touching old entries/rows. Cleanup of unreferenced stages on the next setup is limited, under lock, to stages matching the reserved naming rules for its own targets and referenced by no journal. Do not automatically delete unreferenced backups. Before using journal paths, validate the relationship between normalized targets, transaction-specific stages/backups under the same parent, and the identity journal. A corrupt journal must not cause restoration/deletion at arbitrary paths.

**Locks and discovery through another identity.** Use the existing nix `Flock<File>` for locking (safe API, fs feature enabled). Create parents as needed and canonicalize them; use parent + basename as the lock key without following the final target entry. Add a key constructed the same way for the identity journal, then sort/deduplicate all keys before acquiring locks. Lock files are `.cafleet-install-lock-<sha256(key)>` under each physical parent. Do not replace or delete their inodes; guard Drop releases only the OS lock. Failure partway through acquisition releases guards already acquired. Try returns the path where the real nonblocking lock encountered contention as `Busy`.

To discover another identity’s journal through shared physical targets, place a durable `.intent` sidecar next to each lock file. Order operations as Prepared journal → all Active intents → swap. If an additional journal requires more locks, release all locks, acquire the union in order, and reread. Active with a missing/corrupt journal stops; Finished with a journal triggers recovery; Finished without a journal is normal. After all cleanup/rollback, mark all intents Finished and delete the journal last. Retain lock inodes and Finished intents. If the journal’s physical SQLite main path differs from the caller’s connection, require setup using the original DB configuration; do not automatically open the recorded DB.

A txid mismatch between old Finished intents from a successful installation and a new Prepared journal is accepted as a discovery candidate only when there is no pending operation, all entries are Original, and the valid path/target set includes that key. During recovery, check all locks, DB identity, old row, old fingerprints, and absence of backups; while still Prepared, normalize all intents to Active for the new transaction before moving to RollingBack. Fresh recovery and same-call Fail use the same driver. Another interruption during normalization also resumes from this Prepared state. Reject other tx/state/key mismatches and preserve evidence.

**Per-call injection boundaries.** Before runs immediately before the actual operation; After runs immediately after the operation and required syncs succeed (before the subsequent journal completion record). StageWrite applies to each file with its path; StageValidate performs real validation of all stages; JournalPersist covers every durable update; RecordCommit/RestoreRecord bracket real SQLite transaction commits. LockAcquire brackets acquisition of all locks, retaining locks during After so barriers/independent connections can observe contention. Rename/delete/intent updates/journal removal also pass through the same hook before and after the enum operations above. `Fail` invokes ordinary failure handling; a Fail during rollback preserves the initial cause and appends the recovery failure. `Interrupt` is a typed interruption that exits the driver immediately without rollback/cleanup, releasing only RAII locks and borrows. The next fresh call recovers solely from files/DB/journal. Hooks remain active during rollback; the Tester’s closure injects a fault once based on the event/count. Do not use global environment variables, fake filesystems, fake DBs, or panic-based behavior. Real SQL error cases also include a failure trigger in the fixture that makes record writes fail; injected failures alone must not be described as actual DB failures.

**Diagnosis integration.** Preserve the env snapshot/all-path-validation precedence in `diagnose_assets`. After successful path resolution, use `inspect_install` to inspect the identity’s own journal/shared intents and produce the following before version checks. Inspection does not create files/directories, perform cleanup, or write locks. Journal/intent read/parse failures become incomplete facts with a cause.

```rust
AssetState::Incomplete {
    identity: ResolvedDir, install: Option<AssetInstallRecord>,
    recovery: IncompleteInstall,
}
```

Completion of durable Committed journal persistence is the boundary beyond which rollback is prohibited. Even a Fail at its After checkpoint means installation success plus cleanup_pending; a Fail at Before restores old entries/rows even after DB commit. If a real sync failure leaves durability uncertain, retain the backups and preserve the incomplete state.

Committed cleanup pending means the installation itself succeeded: setup emits a warning and continues with subsequent backends, but guard/doctor report Incomplete while the journal remains. Preserve precedence as all path validation → Incomplete → no-assets/stale/current. Doctor adds state `incomplete` and the canonical error while preserving existing keys/order/nulls; even with conn=None, it examines filesystem evidence without suppressing other sections. `install_agent_with_hooks` is the sole writer adapter translating the real Outcome into the existing installed line and `warning: assets installed at <path>; cleanup pending: <cause>; run 'cafleet setup' to recover`. Setup’s `assets_half_with_options` shares table preflight, fixed order/selection, agent_paths, and the loop using that same adapter; it does not extend InvocationHooks. Cleanup-only output reports the journal’s actual version.

**Verification targets and the Step 11 boundary.** Fixtures use different existing bytes at the same version, old timestamps, and absent rows. Direct entry-point checks cover before/after swaps for each skill and preset; old research presence/type/symlinks; unrelated skills; stage writes/validation; real record errors; the point after DB commit; rollback rename/delete/record; committed cleanup; every journal phase and pending operation; and repeated interruption during recovery. Assert entry fingerprints/digests, exact rows, remaining journal/stage/backup/intent files, and Active/Finished transitions in every case. Also cover real Wait/Try locking, alias identities, shared OpenCode skills with separate presets, and different-DB mismatches. A separate filesystem is demonstrated only by the ID of an available distinct device; distinguish this from a different-directory fixture/injection and report it as unverified if no device is provided. The current manifest verifies sorted relative paths/sizes/SHA-256 of every embedded file and the presence of each skill’s SKILL.md/preset source. The same private stage validator is the integration boundary for Step 11 runtime generation and L01–L28 reference-closure checks; Step 10 does not implement generated artifacts or link rewrites. Actual OS process death/power loss is outside these per-call interruption fixtures.

Documentation verification: `CI=true mise //docs:build` passed, along with checks of 130 local links and 121 fragments in generated HTML. Final evidence: `.cafleet-evidence/programmer-step10/{docs-final,final-audit}.log`.

- [x] Reflect the §9 install journal, locking, and failure/interruption recovery contracts in docs/SPEC, and add filesystem/record-failure fixtures. <!-- completed: 2026-09-06T09:37 -->
- [x] Implement same-filesystem stage/backup replacement, journaling, DB recording, and rollback/recovery; add incomplete-state detection to guard/doctor. <!-- completed: 2026-09-06T11:26 -->
- [x] Verify the same version, each swap/DB/rollback failure, restart, concurrent setup, symlinks, and a preset on another filesystem in an isolated area. <!-- completed: 2026-09-06T11:38 -->

### Step 11: Organize authoritative documentation and skill references

Limited review fix (1401): added the four existing placeholder labels FLEET ID/DIRECTOR MEMBER ID/YOUR MEMBER ID/CODING AGENT, member-role reading instructions, and the ready instruction to the quickstart’s ordinary-member prompt. Also explained waiting for ready and checking the fresh capture gate before the first work assignment. Changed only the authoritative page and runtime copy: `CI=true mise //cafleet:docs-generate` passed in 12.27 seconds → `CI=true mise //cafleet:docs-check` passed in 11.85 seconds. As directed, docs_sync/build/lint were not rerun and no new tests were added. This scope is distinct from the earlier 32-test/docs-build/lint results.

Tasks 3/4 completed (the dev-only boundary in 1363): connected `cli::runtime_docs` to the existing maintenance generate/check commands. Use pulldown-cmark 0.13.4 (dev dependency with default-features=false; only that crate and unicase added to the lockfile) to parse ordinary/reference links, images, headings/explicit IDs, and inline code. Distinguish fenced examples and link labels from static reading instructions; fail on unresolved files/anchors, symbolic-link targets, required paths outside the root, and old docs/docs/SPEC inline references. Production assets/Driver/public APIs are unchanged; setup uses its existing embedded file-set/manifest/hash validation.

Generated 12 documentation pages into runtime with relative structure/content preserved; every page is byte-identical to its authoritative source. The manifest checks the 28 required references and one optional SPEC reference in L01–L29 by source/heading/occurrence/target. Additional optional references R01–R03 form a separate set, with reasons/original targets/URLs/optional labels checked in both authoritative docs and copies. Validate all generation targets before writing; check/build do not write documentation. Only two developer-oriented contributing links and one human-oriented UI-operation example were changed to public repository links, with operational contracts retained in the bundled content (decisions 1375/1384). Preserve existing role/overlay Required-reading and on-demand references; change the four overlay references to Monitoring links.

Shortened quickstart from 243 to 134 lines, moved redundant configuration details to the existing coding-agents page, and fixed two anchors at the destination. Generated the main Claude example and complete Codex/OpenCode examples from one monitor-bootstrap template. Show illustrative absolute HOME/BASE paths and how to substitute actual configuration; preserve ready → loop-start confirmation → monitor live gate → ordinary spawn, plus monitor-first shutdown. Using existing agent_paths/config-dir resolution and the spawn formatter, docs-check performs real installations for all three backends under BASE and checks the two-skill closure, preset presence, four identity labels, and actual absolute paths to the role/overlay/skill/base-dir files. No process-env/HOME/CODEX_HOME changes, monitor execution, E2E/server/browser work.

Validation: final `CI=true mise //cafleet:docs-generate` passed in 10.45 seconds; `CI=true mise //cafleet:docs-check` passed in 12.62 seconds (real installations for three backends plus representative parser fixtures and negative cases for unresolved anchors/old inline SPEC references); existing `CI=true mise //cafleet:test --test docs_sync` passed all 32 tests (16.33 seconds); docs build passed. Changes after docs_sync clarified the OpenCode configuration explanation and added symbolic-path rejection/if formatting in maintenance code. The first two were rechecked by final generate/check/build, and the last was recompiled by lint. Final `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt in 11.26 seconds. No new standalone test groups or full mutation matrix were added; checks were consolidated in the existing check command. The initial dependency-fetch 403 was resolved by Director 1369; decisions for R01–R03 found during pre-generation validation were incorporated; two moved anchors and one manifest heading were corrected; the one collapsible_if lint issue was fixed by formatting. No git operations.

Task 1 completed (limited scope in 1347): added private `cli::doc_contract` under cfg(test). It generates three tables for monitor loop/scan and member capture from actual CliArgs clap metadata, and two tables for monitor_runtime/asset_installs plus four application indexes from SQLite metadata after all migrations have been applied to a temporary file-backed DB under BASE. SPEC has six blocks; cli-options has the same three CLI blocks. Ordinary check does not write documentation; only explicit generate writes. Validate the count/uniqueness/order/nesting of all markers in both target files, then construct all output before writing. No public API/CLI flags/dependencies, migration changes, or separate test suite. The check/generate entry points form the Director-approved maintenance harness.

Review summary of changes outside markers: replaced the three old CLI tables with generated tables, preserving Notes as bullet lists immediately below. Likewise retained Notes for the two old SPEC schema tables; FKs, runtime descriptions, text layout, error ordering, and migration history remain handwritten. Outside generated content, added only corrections to Homebrew/bottle distribution and the Rust spawn mini-formatter, plus maintenance-command guidance. Preserved the existing strformat heading anchor. Verified representative output values: tick parser default=5, capture/scan lines=20, no interval parser default (env → 600 remains handwritten), nullable wake columns and PK order 1/2, and the active-monitor index’s unique/partial properties and `status = 'active' AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor'`. This does not mean FKs, the entire schema, or the entire CLI were generated.

Validation: `CI=true mise //cafleet:docs-generate` passed (10.75 seconds), followed by `CI=true mise //cafleet:docs-check` (8.96 seconds), existing `CI=true mise //cafleet:test --test docs_sync` with 32 passed (13.47 seconds), and `CI=true mise //docs:build`; finally, `CI=true mise //cafleet:lint` passed clippy all-targets -D warnings/fmt (12.07 seconds). No new individual argument tests or mutation suite were added/run. CI integration belongs to Step 12; tasks 3/4, which had not yet started at that point, were completed as described above. No git operations/full suite/E2E/server/browser work.

Tasks 2/5 completed: removed recovery’s idle + unread criterion and independent respawn thresholds, consolidating references on supervision’s fresh capture → action rules. Aligned finished waiting without an assignment, consecutive quiet confirmation for stall_candidate, deferred sends for working/awaiting_user, and exceptions for explicit questions and requested shell dispatch. If even doctor + registry cannot prove that a real pane is gone, retain unknown; ask the user only when missing information blocks work (decision 1343). Preserved the monitor role, backend overlays, Required-reading tables, monitor-first shutdown, and the session/notification contracts from 174/175. For ordinary prose feedback, the Director identifies the pointer and converts it into a user-relay marker, returning through Drafter resolution → rereview → user approval. Direct user-authored markers remain accepted; Approved with unresolved markers remains prohibited.

Verification: existing `CI=true mise //cafleet:test --test docs_sync` passed all 32 tests/0 failed (14.86 seconds); `CI=true mise //docs:build` passed. Matched the four types of added skill anchors against existing headings. No new tests, test-body changes, Rust/TS/package/API/generator additions, git operations, full suite, or lint runs. Changes cover six files: monitoring docs, cafleet recovery/supervision, and design-doc coordination/create workflow/Director role. Tasks 1/3/4, which remained at that point, were completed above; English translation follows all Implementation work.

- [x] Implement generation/checking of mechanical CLI/schema blocks under §10; preserve SPEC structure/details and fix drift including distribution and formatter descriptions. <!-- completed: 2026-09-06T12:14 -->
- [x] Align supervision/recovery state → action rules and reduce repetition while retaining role differences, Required-reading, self-contained overlays, and the contracts from 174/175. <!-- completed: 2026-09-06T12:05 -->
- [x] Fix the §10 set of 29 references (28 required, one optional) and the 12-page closure in a manifest and explicit generation/checking; convert four overlay references into local links and the data-model SPEC reference into an optional public link. Bundle them with skills; verify staged-install files/anchors/public URLs, unconverted inline-code references in authoritative docs and generated output, and detection of unknown/missing references. <!-- completed: 2026-09-06T12:38 -->
- [x] Edit quickstart into a short main workflow; verify complete bootstrap fixtures for three backends and ready/live/identity/role references. <!-- completed: 2026-09-06T12:38 -->
- [x] Update workflows so the Director converts ordinary prose feedback into COMMENT markers; verify role obligations, marker resolution, and pre-approval conditions. <!-- completed: 2026-09-06T12:05 -->


### Step 12: Refine tests and CI, and verify the full implementation

Tester task 1 completed (b134f6e1). Removed only the simple `--interval` presence assertion from cli_options_defines_the_ping_skip_and_moved_capture in docs_sync.rs. Its replacement is the exact comparison of the same cli-options.md `cli-monitor` generated block with the actual Clap definitions by cli::doc_contract::check/update(false), including type, count, action, and default. Retained the role/monitor-gate/overlay, placeholder, notification, session, and ping contracts in the remaining 32 tests. The installed-tree check verifies consistency of distributed files/links/anchors/bootstrap, but does not replace those semantic obligations or inline-path/token validation across all repository skills, so those checks are retained. The data-model.md prose also remains because it is a separate public surface from the generated SPEC.md target. No helper/import became unnecessary; no production/CI changes, new tests/fixtures, or execution. The final gate confirmed that the existing 32 tests and generated-block/installed-tree checks passed.

Final implementation and verification (2026-09-06T12:58): added admin lint to the CI lint job and admin test to the test job. Used Rust tasks’ admin-build dependency to remove explicit builds from both jobs and the extra typecheck from the lint job, retaining the manual typecheck task. Admin test runs only Vitest; build handles TypeScript checking. Avoided duplicating the docs check already in the ordinary Rust suite, and replaced the release workflow’s explicit admin build with docs-check. Corrected the full-path forwarding examples and execution scope in the commands rules. Preserved the existing CI Rust test scope; added no new tests, dependencies, or tasks.

Measured results: `CI=true mise //cafleet:test --lib cli::doc_contract::check -- -- --nocapture --exact` passed one test and displayed installed-closure/bootstrap verification output for three backends (15.15 seconds). Ran the specified selected Rust suite once with `--test-threads=1`: 755 passed / 0 failed / 1 ignored (537.87 seconds; library 526 + integration 229). The only ignored test was explicit-write `cli::doc_contract::generate`. After completion, `CI=true mise //cafleet:lint` passed (clippy all-targets + fmt check). `CI=true mise //admin:test` passed 75 tests across six files; `CI=true mise //admin:lint` and `CI=true mise //docs:build` each passed once. Admin build/TypeScript success comes from the Rust prerequisite; neither was rerun independently. Logs are under BASE at `.cafleet-evidence/programmer-step12/`.

The mapping for all 20 findings reuses existing steps’ contracts and focused verification: F01 → Step 1 pipe/timeout; F02 → 2 monitor uniqueness; F03 → 3 pane ownership/compensation; F04 → 4 delivery aggregation; F05/F06 → 5 typed broker/runtime boundaries; F07/F08 → 6 diagnosis/query; F09 → 7 bounded history; F10 → 3/8 lifecycle/rollback; F11 → 8 shared capture; F12/F13 → 9 fleet/asynchronous state; F14 → 10 recoverable assets; F15 → 6/11 authoritative documentation/generation; F16–F19 → 11 supervision/bundled references/quickstart/ordinary prose feedback; F20 → 4/9/12 frontend tests/deduplication/CI/forwarding. Existing automated verification checked CLI/HTTP wire contracts, preservation of primary errors, DB/pane compensation, repeated asset recovery, and suppression of responses after fleet switches.

Unverified scope: under the user override, E2E, cli_server, and browser/server startup were not performed; task 3’s former browser requirement was omitted under the same override. The actual CI/release workflows themselves were not run locally. Mise emitted external-cache write warnings, but every gate exited 0. The Director completed task 4 checks of implementation differences, documentation drift, historical markers, and completion timestamps. Independent approval by a fresh Reviewer was completed in CAFleet message 1421, and English translation begins.

- [x] Map each important docs_sync assertion to executable contracts/generated blocks/installed-tree verification, and reduce duplication that only pins prose. <!-- completed: 2026-09-06T12:58 -->
- [x] Measure full-path mise test/harness argument forwarding, correct the commands rules, and reflect admin lint/test and build/typecheck deduplication in CI. <!-- completed: 2026-09-06T12:58 -->
- [x] Run the §11 full gates within the user override’s scope, and record evidence for all 20 findings, preserved contracts, and error paths. <!-- completed: 2026-09-06T12:58 -->
- [x] Perform final checks for drift in first-class documentation, unresolved markers, and task completion timestamps; complete the Director’s review of the implementation (fresh Reviewer approval is a subsequent separate gate). <!-- completed: 2026-09-06T13:07 -->

Director final verification: checked F01–F04 regressions, monitor uniqueness, major compensation paths and primary errors, and CLI/HTTP compatibility contracts through each step’s implementation review and the final 755 passing tests. WebUI verification used real HTTP/Node resource checks, React wiring review, and typecheck/build; it does not include DOM/browser execution. Three-backend installed-tree verification, the existing 32 docs_sync tests, generated-block checks, and docs build established reference/authoritative-source consistency. All 36 tasks have completion timestamps; implementation markers were moved into resolved history. git diff --check main...HEAD passed. These checks do not substitute for independent Reviewer approval.

## Translation progress

Completed on 2026-09-06: all sections, including execution history, translated into English. Remaining: none. At the user's request, three members translated separate parts concurrently, each using small subsection edits and incremental diff checks. The Director integrated the verified parts in source order after all three completed.

Final checks preserve fenced code, inline-code content and occurrence counts, link destinations, explicit anchors, heading structure, table structure, checkbox states, and completion timestamps. Part 1 reorders some unchanged inline-code spans to fit English sentence structure. Two Japanese strings remain only inside preserved literal examples: the monitor role-path placeholder and the checkbox task-label example. No Japanese prose remains outside code. Original and incremental snapshots and per-part reports are retained under BASE/.cafleet-evidence/translation/.

Implementation review: approved by Reviewer 225 in CAFleet message 1421. Translation completion: messages 1428, 1429, and 1430. All 36 implementation tasks and eight success criteria are checked. The user approved publication; the branch was pushed and PR #362 was created. This design document and translation evidence remain excluded from commits.

## Changelog

- 2026-09-06: Completed all 36 implementation tasks, required checks, and independent review (approved on the first pass). Completed the English translation in small verified edits across three parallel parts. User approved publication; pushed the implementation and opened [PR #362](https://github.com/himkt/cafleet/pull/362). This design document and its evidence remain local and uncommitted at the user’s request.
