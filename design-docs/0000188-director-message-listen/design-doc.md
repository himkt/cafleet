# Persistent Native Message Reception for the Director

**Status**: Complete
**Progress**: 15/15 tasks complete
**Last Updated**: 2026-10-25

## Overview

Receive Director messages through one persistent JSONL process consumed by the coding agent's native tools, preserving the host's existing continuation state. Codex repeatedly reads a retained execution session, and Claude Code receives individual stdout records through Monitor. Restore the Director exclusion from automatic pane input while retaining PR #486's ordinary-member delivery improvements.

## Success Criteria

- [x] A Director receives and explicitly ACKs distinct messages through the same live receiver process, including a receive–act–receive cycle.
- [x] Every automatic message-preview entry point excludes the fleet's root Director before locking a pane, claiming a preview or resolving a multiplexer.
- [x] Listening emits full pending message envelopes, replays unacknowledged messages after restart and changes no message consumption or preview fields.
- [x] Codex reception uses the same native session between events; Claude Monitor starts separate turns for separate events and renews successfully after a finite receiver lifetime.
- [x] Acceptance runs establish preservation of Codex active/paused/absent and Claude active/absent host continuation state, including during an open user decision. Claude's current paused state remains explicitly unverified and outside this completion gate.
- [x] Ordinary-member delivery, finite `message wait`, explicit member commands and their existing uncertainty guarantees continue to work.
- [x] Living documentation and task-scoped installed skills consistently prescribe native Director reception and distinguish it from ordinary-member pane delivery.

---

## Background

PR [#486](https://github.com/himkt/cafleet/pull/486), merged as `6801b276c6b396529df5aad78a38bf3fefc38876`, addressed stalls caused by pane-state classification. It also made automatic Director reception use Escape-first pane input. The user observed that Escape pauses a Codex goal; resuming prose after the input cannot establish preservation of that host state.

The inspected baseline is `31aaa0e5`. The current `cafleet/src/delivery.rs`, `cafleet/src/receive.rs` and `cafleet/src/cli/message.rs` have no post-merge changes relative to #486. The implementation must apply the behavioral changes below to the current tree rather than revert the merge or restore an old file wholesale.

The experiment at `.prompts/stream-probe-20261025T0025/` used Codex CLI 0.162.1 and Claude Code 2.1.301. `README.md`, `final-summary.json`, the native transcripts and `review.md` establish the following boundaries:

| Mechanism | Established result | Design consequence |
|---|---|---|
| Codex retained execution session with repeated `write_stdin` | One process/session received two events, with action between reads and before process exit | Use explicit native session reads while the Director's working loop remains active |
| Claude Monitor | One process produced two separately delivered native events, each starting a model turn before exit | Use Monitor for per-record notification and retain its task handle |
| Ordinary background stdout | Neither runtime acted during the finite passive observation; ordinary Claude Bash completion later woke Claude | A background process alone supplies no supported per-record wake contract |
| `codex queue` | Woke an idle experimental session without pane input; successful submission required elevated execution in that run | Keep an external queue bridge outside this change |
| Goal preservation and Monitor expiry | Initial goal state was not independently measured; Monitor's finite lifetime was advertised, but expiry renewal was not exercised | Verify these acceptance requirements during implementation |

The user approved this direction through the Director's pre-draft clarification. Fully idle Codex wake, a queue bridge, a general coding-agent event daemon and a new durable consumption model are outside this design.

During execution on 2026-10-25, the user explicitly excluded independent observation of Claude Code 2.1.301's current paused state from this change's completion criteria. Available goal displays and records expose goal presence without the current pause bookkeeping. Preserve the intended pause/authorization contract in the adapter and report this one verification item as unverified; the remaining native reception, continuation-state and decision gates retain their requirements.

---

## Specification

### Transport ownership and scope

The broker remains the source of truth. Sending commits one message; reception exposes that committed record; explicit `message ack MESSAGE_ID` records consumption. Native tool handles belong to the Director's host task state rather than SQLite.

| Recipient or operation | Transport |
|---|---|
| Root Director, determined by `fleets.director_member_id` | Native `message listen`, or explicit manual `poll`/finite `wait` when a verified native adapter is unavailable |
| Active ordinary member with a concrete placement | Existing Escape-first notification drain, independent of pane-state classification |
| Self-send | Existing durable insertion with no pane input; visible to the recipient's broker reads |
| Explicit Director-authorized `member prompt` / `member exec` | Existing explicit dispatch contracts and authorization checks |

Director automatic reception uses broker reads and stdout only. It sends no Escape, text, Enter or queued composer input to the Director, including on startup failure, receiver loss or a missing native tool. The same invariant applies regardless of the Director's recorded coding-agent backend. Explicit member-command behavior remains separate; `member prompt` continues to reject the Director's own pane.

The listener is an inbox reader. It performs no ordinary-member preview retries. The existing `send` drain, sender follow-up and finite `wait` retry rotation remain the mechanisms for ordinary-member notification recovery. A Director explicitly invokes finite `wait` when that recovery is needed; routine stream reads keep using the existing receiver process.

### CLI and JSONL protocol

Add this command:

```text
cafleet message listen MEMBER_ID [--timeout SECONDS]
```

`MEMBER_ID` is the positional active recipient, following the existing subject-ID convention. The command can read any active member's inbox; the supplied native adapters use the Director's ID. It derives and pins the fleet at startup and uses the configured database and ordinary schema/assets guards. Retain the existing `broker::fleets::require_live_fleet` invariant: the root Director must be active and have a concrete registered placement. Listening uses that registry invariant without resolving a multiplexer or checking whether its executable or physical pane is available. An ordinary recipient's own placement may be absent.

| Surface | Contract |
|---|---|
| Output | UTF-8 JSONL, one compact JSON object followed by LF; JSON strings escape embedded newlines and controls |
| `--timeout` | Optional positive integer in `1..=1800`; absence means run until closure, cancellation or failure |
| Timeout meaning | One monotonic lease from listener startup; receiving a message leaves its expiry unchanged |
| Formatting | Always JSONL; `--json` and text mode are absent from this subcommand |
| Idle behavior | Wait for the next database check; emit no repeated empty arrays or heartbeat lines |
| Polling | Check immediately at startup. After filtering already-emitted IDs, a check that emits zero new records waits one second before the next check, shortened by the remaining lease. A check that emits new records may recheck immediately after draining them. |
| Startup | Validate the active subject and live fleet, pin their identity, then emit and flush `ready` before replay |
| Completion | Emit and flush `closed` for normal lease expiry or observed subject/fleet closure, then exit 0 |
| Failure | Report the cause on stderr and exit 1; clap usage failures retain exit 2 |

Use these tagged envelopes, with the shown field names:

```json
{"event":"ready","member_id":1,"fleet_id":1}
{"event":"message","message":{"message_id":12,"from_member_id":2,"to_member_id":1,"created_at":"2026-10-25T00:50:00.000000+00:00","status_state":"input_required","status_timestamp":"2026-10-25T00:50:00.000000+00:00","text":"complete (doc)\nFull original body"}}
{"event":"closed","member_id":1,"reason":"timeout"}
```

`message` has exactly the existing `presentation::message` record shape: integer IDs, UTC timestamp strings, `status_state` and original `text`. The listener bypasses text truncation, pane folding and the pane inline-body limit. Its stdout carries protocol records only; stderr carries diagnostics. An error after some complete records preserves those records and the nonzero exit status; an incomplete final line is unconsumed data.

`closed.reason` is `timeout`, `member_inactive` or `fleet_deleted`. A subject that is unknown/inactive at startup is an error, using the existing member-not-found behavior. After successful startup, normal deregistration or fleet deletion produces the corresponding closure; deletion takes precedence when both occur in the same lifecycle transaction. Check these normal closure conditions before calling `require_live_fleet` again. For a continuing live subject, reapply that validator in the same read transaction as message selection. A missing persisted row, changed fleet identity, invalid stored enum, invalid root-Director relationship or malformed live-fleet invariant, including a missing root placement, is a fatal error, rather than a normal empty inbox. A process terminated by a host signal need not emit `closed`; the host observes its termination.

### Ordering, replay and consumption

Each successful database snapshot selects all `input_required` messages for the pinned subject, ordered by `(created_at ASC, message_id ASC)`. Initial replay and each later snapshot use this same ordering; messages already emitted by this process are skipped. Ordering applies to the records visible in each snapshot: a later commit with an earlier timestamp is emitted when first observed, rather than retroactively reordered.

Maintain a process-local set of emitted message IDs. Insert an ID only after its complete record and newline are successfully flushed. Retain set entries while their IDs remain pending in successful snapshots; completed messages cannot become pending again through the broker API. Pruning completed entries therefore preserves once-per-process emission while bounding retained IDs by outstanding messages. A failed snapshot leaves the set unchanged. Use the set rather than a timestamp cursor, which could skip equal or backdated timestamps.

Determine idleness from the number of newly emitted records after this filtering. A pending ID already in the emitted set supplies no reason to check again immediately. For example, after emitting A and rechecking once, a snapshot containing only the still-unACKed A waits one second; a later snapshot containing A and new B emits B and leaves A suppressed.

The query ignores `notified_at` and `preview_started_at`: preview state is independent of inbox consumption. A second listener is another read-only observer and can emit the same ID. Operational ownership permits one receiver per Director; concurrent readers supply no consumer lock or exactly-once processing guarantee.

After a listener restart, its emitted-ID set is empty. All still-pending messages replay. An ACK committed before a snapshot removes the message from that snapshot. An ACK racing with output can leave an already-snapshotted pending envelope in the native buffer. Before acting on a replay or an ID already known to the Director, reconcile through `poll --json` or `show --json`; an already completed record calls for no second action or ACK. Existing `ack` remains a strict pending-to-completed transition and reports an already-completed state normally.

For a new pending record, the Director reads the full body, records unfinished actions and their message/artifact pointers in host-owned state, then ACKs consumption. Recording a pending user decision is consumption, not permission to resolve it. Listener output, successful flush and native event delivery are never automatic ACKs.

### Read loop, resource boundaries and failure handling

Implement the loop in a dedicated `cafleet/src/listen.rs`, with broker snapshot selection in `cafleet/src/broker/messaging.rs`. Keep the existing finite receiver's scheduling and retry semantics in `receive.rs`. Share typed message mapping, clock abstractions and subject validation where their contracts agree; the listener never calls `ReceiveIo::attempt`, `runtime::deliver_pending`, pane locks or multiplexer resolution.

Each snapshot owns a short read transaction containing subject/fleet validation and message selection. Commit and release it before any stdout operation or sleep. A fresh monotonic database deadline is at most five seconds away; a finite lease shortens it. Use `input::with_database_budget` for lock waiting and a scoped SQLite progress handler for executing SQL. Enable the existing `rusqlite` dependency's `hooks` feature for normal builds in `cafleet/Cargo.toml`; it is currently enabled only by the dev dependency.

The listener owns its connection's progress handler. Before preparing snapshot statements, install `Connection::progress_handler` with an interval of 1,000 VM instructions and a callback that returns true once the snapshot deadline expires. Check the same deadline before and after preparing/executing statements, before and after decoding each row, and before commit. These checks cover Rust-side collection as well as SQLite execution/sorting; the busy handler alone covers neither. Deadline enforcement is cooperative at those boundaries, rather than a hard real-time preemption guarantee for one allocation or SQLite operation.

On deadline interruption, discard the partial snapshot and return `CafleetError::DeadlineExceeded`; preserve other SQLite errors and their causes. Only translate an interrupted SQL error when the owning deadline has actually expired. Release statements and roll back an incomplete transaction, then clear only the listener-owned progress handler with `progress_handler(0, None::<fn() -> bool>)` on every return path. Check handler installation/removal results and retain cleanup failures alongside the original cause. Preserve the existing scoped busy-handler restoration. A completed snapshot disables its progress handler before any stdout or sleep.

Retry transient SQLite busy conditions only inside the snapshot budget. Exhaustion during a snapshot or other database errors terminates the stream visibly with exit 1; normal lease expiry observed between snapshots/records follows the `closed` contract. Receiver restart begins with a new validated snapshot. A healthy idle stream retains its connection and PID across messages.

Serialize and write one record at a time, checking every write and flush. Bound output progress by a five-second deadline, shortened by a finite remaining lease. Use the existing `nix` filesystem/poll facilities for interruptible nonblocking output, retaining/restoring descriptor flags on normal return. A partial write continues from its byte offset within that budget. Broken pipe, write-zero, flush failure or exhausted output budget ends with an explicit stderr diagnostic and exit 1; this never becomes a healthy timeout. Normal lease expiry between records emits `closed` with a separate bounded final-write budget. A failed partial record is recovered through broker replay, not by continuing a damaged stream.

Only the current snapshot, one serialized record and pending emitted IDs occupy listener memory. Apply stdout backpressure before selecting more messages; no reader transaction or broker write lock spans backpressure. Large bodies keep their exact bytes after JSON decoding. Repeated idle checks consume no native notification budget because they emit nothing.

The listener retains ordinary process signal handling, so host cancellation terminates it without waiting for a pane-input critical section. PR #486's `DeferTermination` remains confined to the operations it protects; an indefinite listener never holds that guard. The implementation adds no schema migration, external service, broker daemon or package dependency.

| Condition | Owner response |
|---|---|
| Healthy native read returns no output with a live handle | Keep the same handle and read again when appropriate |
| Clean `closed` with `timeout` and authorized work remains | Confirm native task exit, then start the next lease and replay pending records |
| `member_inactive` / `fleet_deleted` | Retire the handle; resume only after an explicitly established new task/identity |
| Unexpected EOF, failed tool call or receiver exit | Inspect host exit/output and reconcile the inbox; validate broker health before restarting |
| Fatal database/schema/assets error | Surface the actual cause and use the existing repair/authorization procedure; start a receiver after repair |
| Host output truncation or incomplete JSONL | Preserve complete records, leave the incomplete message unacknowledged and retrieve the full native output or reconcile/replay via broker commands |
| Old receiver liveness cannot be established | Preserve its handle and diagnose or use explicit manual reads; establish termination before replacing the owned receiver |
| Native stream tool unavailable or denied | State the limitation and use the documented manual receive path within existing host permissions |

Restarts are lifecycle/error recovery, not a step after each message. An unexpectedly failing receiver must not generate a tight automatic restart loop. Perform a fresh successful broker health/inbox check before a replacement attempt; repeated failures remain visible for diagnosis.

### Codex Director adapter

Keep this integration in skills/backend instructions; the Rust CLI does not invoke the Codex app server or manage Codex threads.

1. After `fleet create`, start `cafleet message listen <director-id> --timeout 300` once through native `exec_command`, with a short initial yield such as 1,000 ms. Retain the returned live `session_id` and any stdout from that initial result. Require the `ready` record before relying on reception.
2. While delegated work remains, call `write_stdin` on that same session with empty `chars` and a bounded yield such as 10,000 ms. A tool-level read/yield timeout leaves the receiver running. It is distinct from the CLI's 300-second lease.
3. Accumulate incomplete JSONL across tool-result chunks. Consume every complete record, reconcile pending IDs as needed, ACK consumption and dispatch work whose inputs are ready. Then read from the same session again.
4. Keep independent authorized work and user-facing progress updates between bounded reads. Native output reads are explicit tool calls; a background session alone supplies no unsolicited per-line wake after the model turn ends.
5. At `closed`/exit, renew only while permitted delegated work remains, using the recovery rules above. Store the session ID, pinned member/fleet IDs, receive status and unfinished actions in host-owned task state across context compaction.

The Director keeps its tool-bearing receive loop active while delegated work remains, independently of whether a host goal was supplied. It preserves that goal's activation, pause, completion and budget authority as received: reception never calls a goal lifecycle tool, starts a goal, resumes a paused goal or changes its budget. A required user decision gates dependent work even if messages continue arriving. Where the host requires ending the turn or blocks tool use at that decision, a fully idle Codex session waits for the next user/native activation and reconciles its durable inbox then; this design claims no automatic idle wake.

For normal receiver retirement without a verified session-cancel primitive, stop renewing and drain the finite lease to its observed exit. A host-supported native cancellation may retire it sooner; confirm termination and discard any partial final line. Sending control characters into the Director's pane is outside the adapter.

### Claude Code Director adapter

Discover Monitor through the installed host's tool-discovery mechanism. The tested invocation shape was:

```json
{"command":"cafleet message listen 1 --timeout 210","description":"CAFleet Director inbox","timeout_ms":240000}
```

Substitute the actual literal Director ID. Retain the returned task ID and consume its `ready` record. The 210-second CLI lease normally ends before the tested 240-second Monitor lifetime, providing a controlled renewal point while retaining one process between messages.

Each Monitor stdout event carries one protocol record and may start a new model turn. On a `message` event, consume/reconcile the record and the durable inbox, record deferred work, ACK consumption and act within existing authorization. Return to other work or end the turn while Monitor stays active. An event arriving during a user question is a broker event, not the user's answer.

On `closed` or the native stream-ended/completion notice, establish the old task's exit. If authorized work remains, start one new Monitor with the same command and reconcile replay. Deduplicate `closed` and completion notifications by the owned task ID so they cause one replacement, and ignore stale task-completion notices after ownership has moved. A host timeout before CLI closure follows the unexpected-termination recovery path and requires establishing that its source process ended. Verify real Monitor expiry, cancellation and source-child cleanup during acceptance instead of assuming the earlier experiment established them.

Ordinary `Bash(run_in_background=true)` is documented as a different capability: the observed completion notice did not provide per-line reception while the process lived. Where Monitor is unavailable, manual finite reception preserves the explicit limitation. Reception and renewal preserve the supplied Claude `/goal` state and user decision gates.

### Director supervision and shutdown

Arm native reception after Director registration and before depending on member ready replies. A reply racing with startup is recovered by initial replay. On every received batch/wake, reconcile through `message poll <director-id> --json`, consume and ACK pending records, then inspect roster, artifacts and assignments past their expected reply point. Apply existing capture and ordinary-member `unconfirmed_previews` recovery within authorization. A Director's own claimed pending IDs are consumed through broker reads, rather than a prompt into its pane.

Keep the live native handle while assignments or an authorized user-feedback phase remain outstanding. Before teardown, consume the final inbox and complete the workflow's approval gates. Delete owned members and the fleet through the existing lifecycle commands; the listener observes `fleet_deleted`, ends and is collected through its native handle. If reception is retired while the fleet stays live, use native cancellation with observed termination, or the finite lease. Preserve pending messages for later reads. Fleet deletion continues to deregister registry state without killing the Director's pane.

### Exact partial revert and preservation boundaries

Restore recipient eligibility and the Director supervision contract from before #486; retain its ordinary-member transport implementation. These are targeted edits against the current tree:

| Current target | Required change |
|---|---|
| `cafleet/src/delivery.rs::preview_snapshot` | After typed member lookup, return `None` unless `member.kind == MemberKind::Member`, before selecting a preview or acquiring a pane lock. This protects `send` and every caller of `deliver_pending`. |
| `cafleet/src/delivery.rs::claim_current_preview` | Revalidate ordinary-member eligibility with the current typed member inside the claim transaction. End an ineligible attempt as `NoPending`; use a private skip-variant name covering inactive/ineligible membership. Preserve placement and oldest-unclaimed revalidation. |
| `cafleet/src/broker/panes.rs::pending_previews` | Restore the join to `fleets` and the predicate `m.member_id != f.director_member_id`. Keep the wait-subject exclusion, concrete-placement condition and ordering. An ordinary subject's `wait` must never retry the Director. |
| `cafleet/src/cli/message.rs::Send` | Keep post-commit delivery through the common guarded path. Director sends succeed with `notification_sent: false`, no pane-delivery diagnostic and no multiplexer requirement. This boolean still describes pane submission by this invocation, not native consumption. |
| `cafleet/src/cli/message.rs` and `cafleet/src/lib.rs` | Add the `Listen` command and dedicated module; preserve `poll`, `show`, `ack` and finite `wait` response shapes and timeout contracts. |
| `cafleet/src/presentation.rs` and broker read helpers | Reuse the existing full message record inside tagged stream envelopes and add the snapshot API. Keep preview and consumption mutations out of that API. |
| `cafleet/src/member_permissions.rs::PROBE_COMMANDS` | Include a representative `message listen` invocation in broker-command diagnostics. Verify native Monitor permission separately; a Bash-rule check alone proves no Monitor authorization. |
| Tests currently expecting Director pane input | Replace them with positive broker/stream reception and zero-input assertions; retain ordinary-member transport coverage. Include retry rotation from an ordinary subject, not only a Director subject. |
| Skills and living runtime documentation | Replace universal pane-delivery and end-turn wake instructions with recipient-specific reception and backend-native lifecycles, using the authoritative ownership described below. |

Retain the `Lead::Escape`/`Lead::Direct` interface, first-Escape/remaining-direct drain, full inline-or-pointer pane formatting, lock reservations, durable uncertain claims, `unconfirmed_previews`, termination deferral around pane/lifecycle operations and all explicit member-dispatch guards. Keep the classifier and `member ping` absent. The stream change needs none of the deleted classifier modules or fixtures.

Keep the existing schema and roster field meanings. New non-self sends to the Director receive no pane claim or notification stamp; they remain `input_required` until explicitly ACKed. Preserve self-send insertion semantics: a Director self-send sets `notified_at` at insertion, produces no pane input and remains pending for listener reception and explicit ACK. Existing pending Director records, including records with a notification stamp or an uncertain claim, are eligible for stream replay. Their consumption uses the same ACK transition as every pending record. This requires no migration or historical callouts in living schema documentation.

`message wait` retains full inbox results, its finite timeout and ordinary-member retry fairness; `pending_previews` narrows its recipient set. It remains a valid manual receiver and explicit ordinary-member retry mechanism. The new listener is not implemented as repeated child invocations of `message wait`.

### Documentation ownership and complete cleanup

`skills/cafleet/reference/supervision.md` owns the generic Director lifecycle; the runtime sections of `skills/cafleet/reference/coding-agents.md` own concrete native tool recipes and their evidence limits. `skills/cafleet/SKILL.md` owns broker command usage and ordinary-member communication. Consumers link these owners rather than duplicate a runtime recipe.

Update `SPEC.md` and the relevant pages under `docs/docs/` for the command/JSONL contract, recipient eligibility, send outcome, read-only data semantics and the manual lifecycle. Update README reception prose and every skill/workflow/rule that prescribes automatic Director pane delivery, unconditional end-turn wake or the former per-turn 30-second Director wait. Preserve finite `wait` examples where they serve manual reception or ordinary-member retry recovery. Existing pane interruption clauses remain accurate for ordinary-member transport and explicit dispatch.

Find affected surfaces by the behavior they describe, including the design-doc workflows and broker-command permission guidance. Rewrite them as the current positive contract. Remove obsolete Director Escape/re-ask recovery instructions and any assertion that a persistent Codex process supplies unsolicited idle wake. Historical rationale remains in this design and git; living instructions describe the resulting system. Leave unrelated design documents unchanged.

### Validation gates

| Gate | Required evidence |
|---|---|
| Stream unit/CLI behavior | Ready-before-replay, oldest-first snapshot order including equal/backdated timestamps, multiple senders, exact Unicode/multiline bodies exceeding the pane limit, once-per-process emission, ACK-before-read, ACK/output race and restart replay. A clock-driven case keeps an emitted A unACKed across several checks, verifies one-second waits after zero new emissions, then inserts and receives B without repeating A. |
| Read-only and isolation | Database state unchanged by listening; a stamped pending Director self-send is emitted with zero pane input; another fleet's messages are absent; an ordinary recipient may lack its own placement while the root has a concrete registered placement; unavailable multiplexer executables cause no stream failure or invocation. |
| Lifecycle and errors | Empty inbox stays live, lease expiry after activity, member/fleet closure, startup invalid subject, changed/corrupt identity, missing root placement at startup and mid-stream, database contention/failure, partial writes, broken pipe, output timeout, cancellation and no leaked receiver process. A long-running SELECT/sort with no lock contention is interrupted by the scoped execution deadline; row-collection checks discard an expired partial snapshot; a later query succeeds after handler cleanup. |
| Partial revert | A member-to-Director send persists with `notification_sent: false` and zero pane calls; a wait owned by another ordinary member excludes the Director; direct delivery entry points reject Director eligibility before resolve/lock/claim |
| Ordinary-member preservation | Existing drains, self-send exclusion, locks, uncertainty reconstruction, retries, explicit dispatch, termination spans and roster guarantees still pass |
| Native Codex | One PID/session receives event A, action/ACK completes, event B arrives, and a later read on the same session processes it before lease exit; a new lease replays an intentionally unACKed record |
| Native Claude | Distinct events start separate Monitor turns before process exit, including a quiet interval after a completed turn; controlled lease renewal and actual Monitor expiry leave one owned source process and lose no pending messages |
| Host continuation and decisions | Independently observe Codex active/paused/absent and Claude active/absent states before and after receive, ACK, renewal and an open decision. Demonstrate authorized active work, preserved Codex pause and absent-goal state; record Claude current paused-state preservation as unverified and excluded from this completion gate |

Use isolated test fleets/databases and recorded process/handle identities for native acceptance. Send unpredictable real broker messages after arming; prove their bodies came from native results/notifications and acted before receiver exit. Capture host goal state with the installed runtime's read-only inspection surface and retain that evidence. A transcript lacking goal lifecycle calls alone does not prove state preservation. If a required state cannot be observed or a native-tool permission is unavailable, report that validation gap explicitly and keep the native integration unverified until it is resolved.

The user-approved exception is Claude's current paused-state observation: leave that item unverified and identify it in the final report, while completing all other applicable gates. This verification exception grants no authority to resume or change a paused goal.

Exercise an approval/question prompt with incoming messages: its pending decision remains pending, its composer/draft remains intact, and broker events supply no user answer. Native tests use no automatic Director pane injection or queue bridge. Measure PID continuity and action chronology rather than treating the earlier two-event delays as latency guarantees.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Restore Director recipient eligibility

- [x] Add ordinary-member eligibility guards to preview snapshots and claim revalidation, and restore the root-Director exclusion in `pending_previews` against the current implementation. <!-- completed: 2026-10-25T01:12 -->
- [x] Replace Director pane-notification expectations with durable-inbox and zero-input assertions across send, direct delivery and ordinary-subject wait retry tests; retain ordinary-member behavior tests. <!-- completed: 2026-10-25T01:16 -->

### Step 2: Implement the persistent stream

- [x] Add a short-transaction typed inbox snapshot API with pinned subject/fleet validation, the existing root-placement invariant, oldest-first selection and normal lifecycle-closure detection; enable production `rusqlite` hooks and enforce scoped execution and row-collection deadlines. <!-- completed: 2026-10-25T01:33 -->
- [x] Implement `listen.rs` with injected clock/output boundaries, process-local pending emission tracking, one-second waits after zero new emissions, bounded database/output work and explicit lifecycle/error outcomes. <!-- completed: 2026-10-25T01:39 -->
- [x] Wire `message listen MEMBER_ID [--timeout SECONDS]`, tagged JSONL presentation, checked stdout handling and broker-command permission diagnostics without changing existing command responses or the schema. <!-- completed: 2026-10-25T01:43 -->
- [x] Add meaningful unit and subprocess tests for stream ordering/replay, exact bodies, isolation, read-only/self-send state, ACK races, suppressed-message idle scheduling, missing root placement, long-running SQL/collection deadlines and handler cleanup, partial/broken output and process termination. <!-- completed: 2026-10-25T01:50 -->

### Step 3: Integrate native Director reception

- [x] Rewrite the supervision owner around arming, receiving, consumption, recovery and shutdown; retain ordinary-member notification recovery and user authorization gates. <!-- completed: 2026-10-25T01:18 -->
- [x] Add the Codex retained-session recipe and Claude Monitor recipe in their runtime sections, including handle retention, bounded reads/leases, Monitor renewal and the fully idle Codex limitation. <!-- completed: 2026-10-25T01:18 -->
- [x] Reconcile living documentation, consuming workflows, CLI/spec pages and permission guidance by behavior; remove obsolete Director pane-wake instructions and link the authoritative recipes. <!-- completed: 2026-10-25T01:31 -->

### Step 4: Verify runtime behavior and preservation

Independent pre-install verification: `.prompts/execute-20261025T010615Z/verifier-implementation-review.md` maps C/R/Z/V gates to reviewed source and original passing results; `verifier-implementation-audit.json` retains the exact test names, native command results and source fingerprints. The fresh documentation-only checks also pass. `.acceptance-claude/verifier-review.md` and `verifier-original-audit.json` independently establish per-record turns, clean renewal, actual Monitor expiry, TaskStop/source cleanup, absent/active goal preservation and the genuine open-question/composer window. The passive native Goal achieved capture follows cleanup. H-CL-P remains explicitly unverified under the user-approved exception. Director-owned old-runtime fleets are closed. CLI/assets installation and a fresh installed-runtime lifecycle use task-scoped locations while unrelated teams retain the shared runtime.




Programmer audit evidence: [semantic documentation audit and deferred installation plan](.prompts/execute-20261025T010615Z/programmer-living-doc-audit.md), with [source fingerprints and resolved runtime links](.prompts/execute-20261025T010615Z/programmer-living-doc-audit-manifest.json). One stale create-workflow clause was corrected; fresh docs-sync tests and the documentation build pass. Native acceptance and final task completion remain with the Director.

Codex acceptance evidence: `.prompts/execute-20261025T010615Z/.acceptance-codex-active/verifier-review.md` and `verifier-post-answer-draft-audit.json` establish active-goal preservation and the actual open-question/composer gate. The paused and absent reviews independently establish complete native cycles and emitted-message replay. Original timing limits remain explicit in each review. The temporary test objective is complete after its exact FINALIZE control, genuine Keep waiting answer and observed source cleanup; the separate paused root remains identical.


- [x] Run isolated real-broker Codex receive–act–receive and restart/replay acceptance with recorded PID, session, original bodies and action/ACK chronology. <!-- completed: 2026-10-25T02:52 -->
- [x] Run isolated Claude Monitor per-record wake, CLI lease renewal, actual Monitor expiry and child-process cleanup acceptance with recorded task/PID ownership. <!-- completed: 2026-10-25T02:52 -->
- [x] Verify independently observed Codex active/paused/absent and Claude active/absent continuation states and open user decisions, retaining state and zero-Director-input evidence; report Claude current paused-state preservation as unverified under the user-approved scope exception. <!-- completed: 2026-10-25T02:52 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`; resolve failures and verify that no living surface prescribes automatic Director pane input. <!-- completed: 2026-10-25T02:52 -->

### Step 5: Deliver the verified runtime

- [x] Prepare a reviewable change showing targeted #486 reversals, native reception evidence and preservation of ordinary-member behavior, with design files remaining outside the implementation commit. <!-- completed: 2026-10-25T02:52 -->
- [x] After closing this task's old-runtime fleets, use the installation operation from `mise tasks` to install the CLI and embedded skills into task-scoped locations, run `cafleet doctor --json`, and verify a fresh native Director lifecycle and ordinary-member delivery. Preserve the shared runtime while unrelated teams use it. <!-- completed: 2026-10-25T03:10 -->


Installed-runtime evidence: `.prompts/execute-20261025T010615Z/installed-runtime-acceptance.json` and `installed-doctor.json` record the registry-selected installation, exact installed/source skill comparison, doctor with zero issues, native session 91100 / PID 47827 receiving and explicitly ACKing two distinct messages, ordinary-member pane notification, root-only teardown, `closed: fleet_deleted`, exit 0 and no remaining database owner. Shared installation remains deferred.

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-25 | Final verification uses a task-scoped CLI and embedded-skill installation; shared deployment is deferred while unrelated teams use the runtime. The user authorized completion and PR creation after successful final review. |
| 2026-10-25 | Completed after independent final review approval; published as PR #491. Shared-runtime deployment remains deferred. |
