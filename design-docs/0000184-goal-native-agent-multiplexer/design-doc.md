# CAFleet as a Coding-Agent Multiplexer

**Status**: Approved
**Progress**: 0/28 tasks complete
**Last Updated**: 2026-10-22

## Overview

CAFleet proactively supplies independent coding-agent sessions wherever the host would otherwise delegate to subagents or an agent team. The host owns continuation and the Director supervises delegated work through a small CLI for panes, durable messages, bounded receiving and visible native command dispatch. Reduce the product to CLI and skills by removing the monitor member, detached scheduler, tracked execution helper, WebUI and HTTP server.

## Success Criteria

- [ ] Discovery descriptions select CAFleet proactively for delegation; ordinary workflows describe host-owned continuation without activation commands or validation gates.
- [ ] Fleet creation registers the current Director atomically and spawns no agent or background process; ordinary member creation works immediately afterward.
- [ ] A Director receives replies through broker reads, and a finite receive operation retries pending ordinary-member notifications while it waits.
- [ ] Messaging is point-to-point; the Director performs fan-out as individual sends, with no broadcast summaries or grouping-only fields.
- [ ] A scripted delegation with host continuation disabled receives the member reply through repeated finite `message wait` calls owned by the Director's active tool session; the Director retains that session until the reply or explicit user handoff.
- [ ] Every CAFleet pane writer holds one process-shared input lock through readiness revalidation and the complete submission sequence.
- [ ] Public `member exec` visibly submits the actual native shell command and returns dispatch metadata; native host output owns execution status.
- [ ] Busy, approval, draft, unknown and pending-placement exec targets receive no keystrokes, and uncertain dispatch never causes automatic command resubmission.
- [ ] The fresh default database has an explicit format identity and no execution/supervision state; selecting an incompatible existing file preserves its bytes and rejects writes.
- [ ] OpenCode members retain model selection, assignments, editing, permitted commands, broker communication and routed native execution.
- [ ] Current code, docs, skills, presets, rules, tasks, dependencies and tests contain a consistent reduced product surface with all removed-concept references deleted.
- [ ] Rust builds and release packaging work without the frontend; documentation-site tooling remains available.
- [ ] Every row of the §11 verification table has a passing automated check or recorded smoke result, and the implementation report records net reductions in production code and living documentation.

---

## Background

The investigation covered the Rust architecture and tests, frontend/server, skills, documentation, build tasks and release workflows. The Director's unmodified baseline run passed 544 tests in 340.74 seconds. Source references below use the investigated CAFleet 0.27.1 checkout; the hidden scan reports are optional local supporting evidence. In this investigation/design phase, the design and its hidden artifacts stay uncommitted per the session's instruction.

| Existing dependency | Evidence | Required change |
|---|---|---|
| Fleet creation owns a monitor agent and daemon | `cafleet/src/cli/fleet.rs:124`, `broker/fleets.rs:49`, `runtime/mod.rs:80` | Register the current Director without spawning either |
| Ordinary spawn requires a monitor | `cafleet/src/cli/member.rs:333` | Keep placement/rollback and remove the prerequisite |
| Message retries depend on a permanent loop | `cli/message.rs:128`, `monitor/mod.rs:152` | Give retries a finite caller-owned receive path |
| Command execution is a persisted lifecycle | `cli/member.rs:552`, `broker/execs.rs`, `delivery.rs:125` | Submit the real command through the host and inspect native output |
| Pane serialization is only a timestamp interval | `broker/panes.rs:83` | Hold exclusive ownership over the complete input operation |
| Rust builds require frontend output | `cafleet/build.rs:28`, `cafleet/mise.toml` | Retain skill/preset embedding and remove SPA embedding |
| Database compatibility uses version alone | `diagnosis.rs:90`, `cli/setup.rs:107` | Recognize a distinct format before either setup half writes |

Actual Codex member 23 demonstrated a tool-bearing turn automatically continuing into a second turn, then model-driven completion. Native `! printf` displayed the actual command, exit code, duration and output. Actual Claude member 24 demonstrated its session condition and separate transcript evaluator, with native background work deferring evaluation; it also observed command/stdout/stderr without an exit status, followed by an assistant turn whose trigger the probe did not establish. Neither host registers external CAFleet panes as native child work. A Claude observation of a slash prompt merging with a concurrent broker preview establishes that complete pane-input serialization is required.

Optional local-only supporting evidence and limits are in [.scan/rust-architecture.md](.scan/rust-architecture.md), [.scan/codex-goal.md](.scan/codex-goal.md), [.scan/claude-goal.md](.scan/claude-goal.md), [.scan/claude-audit.md](.scan/claude-audit.md), and [.scan/director-answers.md](.scan/director-answers.md). The official [Codex continuation guide](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), [Claude continuation guide](https://code.claude.com/docs/en/goal), and [OpenCode TUI reference](https://docs.opencode.ai/docs/tui/) support the backend distinctions; local probes establish only the behavior they exercised.

---

## Specification

### 1. Responsibility and product boundary

| Owner | Responsibility | Evidence it consumes |
|---|---|---|
| Host | Continue its own task and own native shell/background work under its permission controls | Its conversation and tool/task results |
| Director | Select CAFleet for delegation, assign scope, collect results, inspect panes, resolve blockers and finish the workflow | Broker records, artifacts, native task handles and pane captures |
| CAFleet CLI | Register sessions, manage panes, persist messages, receive for a bounded time and submit authorized native input | Explicit IDs, database rows and fresh transport-readiness captures |
| Multiplexer adapter | Discover/split/read/type/close panes and preserve layout/cleanup ownership | Native tmux/herdr results |
| Member | Perform its assignment, report through CAFleet and route required unavailable operations to the Director | Spawn identity, role, assignment and its own inbox |

The shipped product consists of CLI, embedded skills/presets and the documentation site. It has no HTTP listener, UI package, permanent scheduler, synthetic timer prompts or model-based monitor. Continuation and supervision remain host/Director behavior. The CLI's finite notification retries inspect transport readiness only; they infer no task progress, stall or completion.

Claude and Codex provide the verified normal Director context. Existing explicit backend selection remains registry-driven. OpenCode is a full member backend: it accepts assignments, selects provider/model pairs, reads/edits allowed workspace files, runs permitted tools, exchanges broker messages, and receives Director-authorized commands through native shell mode. The design adds no host-condition setting, validation or mutation to fleet creation or workflow startup.

All members of a fleet use the same database URL. CLI IDs remain coordination identifiers within the current local trust model; this redesign adds no remote authentication or new permission bypass. A routed command still requires the Director's authorization under its host rules.

### 2. Normal delegation and receive procedure

1. When independent investigation, review or implementation work would otherwise use a subagent or agent team, load CAFleet and select the necessary member roles/backends. A direct local edit that needs no delegation proceeds directly.
2. Register the current Director with `fleet create`; write scoped spawn prompts and create the selected members. Preserve literal identity substitution, supplied host rules, BASE inheritance and the ready handshake.
3. Read complete broker payloads, record outstanding actions or deferred user decisions in host-owned task state with their message IDs/artifact pointers, then ACK each read delivery immediately and act on its evidence. Broker messages carry coordination; `member prompt` supplies explicitly authorized native composer input and `member exec` supplies a native shell command.
4. While replies are outstanding, run `message wait <director-id> --timeout 300 --json` as a host-native background task. Use an explicit timeout of at most 30 seconds with a foreground mechanism that cannot return a live task handle. Retain its live task/session handle and collect its terminal result. A timeout causes a roster/pane/artifact check, followed by another finite receive if useful work remains outstanding.
5. On a reply, perform the requested workflow action and begin another bounded receive when necessary. An existing user decision or review gate remains authoritative. Finish by deleting ordinary members, then deregistering the fleet.

The Director maintains the receive/check procedure during ordinary delegation as well. When no host automatic continuation is available, it keeps its active tool/background receive handle until the delegated work resolves, or explicitly hands the remaining work back to the user. Ending a Director turn with outstanding work and no host-owned receive/continuation is an incomplete handoff.

A member polls its inbox immediately after ready, before declaring an assignment complete, and on each subsequent wake. At the end of a report turn it handles new queued assignments before becoming idle. If a message arrives after the final poll while the member is busy, the Director's next finite receive retries its pending notification. This closes the poll/idle race without a permanent process.

The Director chooses its response from evidence: affirmative active work permits another receive; a pending human decision is relayed; a missing/dead pane or concrete failure calls for recovery; quiet or ambiguous captures call for further inspection. For a retained user draft, relay the capture and ask the user how to preserve or submit it. A persistently unusable unknown pane follows the existing authorized member-replacement procedure, preserving assignment, messages and artifact context in its recovery prompt. Keep its input untouched while inspecting or relaying the issue. A timeout alone proves no member failure. Expected ready/reply deadlines belong to the assignment, and the Director checks overdue expectations itself.

### 3. Fleet and member lifecycle

```text
cafleet fleet create --name NAME --coding-agent AGENT [--json]
cafleet fleet capture FLEET_ID [--lines N] [--ansi] [--json]
```

`fleet create` resolves a valid tmux/herdr context, validates the selected backend token, and writes fleet, current Director member and Director placement in one transaction. It backfills `director_member_id` before commit. It performs no pane split, member readiness wait or process launch. Text output is `<fleet-id> director=<member-id>`. JSON retains the existing ordered `fleet_id`, `name`, `created_at`, `director` structure; `director` retains its current identity/description/registration/placement fields.

`fleet capture` is the one-shot diagnostic rehomed from the current batch capture. It uses the current active roster, emits the Director first and other placement-owning members by ascending ID, preserves `--lines`/`--ansi` and timestamp/fingerprint behavior, and annotates per-pane failures without aborting the batch. It writes no broker state and runs no subsequent check. JSON retains the existing scan-entry shape with `kind` restricted to `director` or `member`.

`member create` retains the current prompt/file contract, backend inheritance/override, model/effort validation, identity substitution, pending placement, pane split, placement patch and compensation ladder. Its role is its supplied assignment; no registry role flag is needed. Spawn argv contains no monitor argument or special monitor permission branch. A registration/placeholder/split/patch failure preserves the primary error and any cleanup failure; cleanup targets only a pane actually owned by that operation.

`member delete` retains root-Director protection and kills/deregisters the selected ordinary member. `fleet delete` retains registry soft deletion and deregistration without closing panes; the Director deletes ordinary members first when it owns their teardown. Both serialize placement mutation with input operations as specified below. List/show/activity outputs retain their current fields except the removed member-card `skills` field, and `kind` derives solely from `fleets.director_member_id`.

### 4. Durable messages and synchronous notification

The broker stores point-to-point messages with fleet isolation, full text, explicit ACK and subject-ID behavior. The Director performs fan-out with an individual send to each selected recipient; every resulting message has its own identity and acknowledgement. A committed message is the source of truth; a pane notification is a convenience that may repeat. Poll/show/wait JSON remains untruncated; text and pane previews obey `CAFLEET_MAX_TEXT_LEN`.

The complete ordered message record is:

```json
{
  "message_id": 17,
  "from_member_id": 41,
  "to_member_id": 42,
  "created_at": "2026-10-22T13:00:00.000000+00:00",
  "status_state": "input_required",
  "status_timestamp": "2026-10-22T13:00:00.000000+00:00",
  "text": "ready"
}
```

`to_member_id` is always an integer and defines the inbox owner. `status_state` is `input_required` until ACK changes it to `completed`; `status_timestamp` records that transition. Completed means the delivery was read/consumed, independently of whether the assignment or a deferred user decision has finished. Send returns `{message, notification_sent}`, poll returns an array of these records, and show/ACK return `{message}`. Remove the broadcast command, summary rows, `type`, `origin_message_id`, duplicate `owner_member_id`, grouping records/formatters and their specialized errors/tests. There is no shared transaction or summary identity across individually sent fan-out messages; report any failed recipient explicitly and retain IDs for already committed sends.

| Recipient | Synchronous send behavior | Retry owner |
|---|---|---|
| Fleet Director | Persist only; `notification_sent=false` | Director's broker read/wait |
| Sender itself | Persist with notification already settled | Sender's broker read |
| Ordinary member with an idle empty composer | Acquire input lock, revalidate, submit its oldest pending preview, stamp success | Sending invocation; subsequent finite receives for anything still pending |
| Working/approval/pending or input-lock contention | Persist and leave notification pending as an ordinary hold; send no keystroke | Next finite receive in the fleet or a later send to that member |
| Draft/unknown or unreachable member pane | Persist and leave notification pending; report the readiness state or transport error without typing | Director inspection and the next finite receive or later send |

Send attempts at most one preview for its selected recipient in the synchronous phase. The oldest pending unacknowledged message by `(created_at, message_id)` wins. `notified_at` is stamped only after the complete submission succeeds, conditional on the message still being pending. An ACK removes a message from notification eligibility. Revalidate recipient activity, fleet, placement, message status and stamp while holding the input lock, immediately before submission.

The preview is one literal line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] TEXT
```

Fold CRLF/LF/CR in preview text to `⏎`, display tabs as `␉`, and escape other C0/DEL controls as visible hexadecimal escapes. Stored text remains exact. The fixed prefix keeps message content distinct from native composer commands. Only an actual persisted pending message justifies a notification; waiting creates no synthetic poll, wake, resume or health-check text.

Send JSON uses `{message, notification_sent}` with the reduced message record above. The boolean refers to the newly committed message, so delivering an older pending preview leaves it false. A held or unavailable notification exits successfully with `notification_sent=false`. On the first attempted preview classified `unknown` or `draft`, emit `Message NEW_ID was persisted. member ID is STATE; preview PREVIEW_ID was not sent.` on stderr, naming both the newly stored message and the attempted preview when they differ. No daemon startup can turn a committed send into a failure. A storage failure before commit exits 1 with its database cause; another notification/stamp problem after commit is a stderr diagnostic naming the persisted message ID, while the successful stdout record remains available and is never resubmitted as a new message.

Pane notifications are best effort, with pending state enabling retries. A process can exit after native submission but before its stamp, so a preview may repeat. Members use the message ID, current poll result and ACK state to decide whether work remains actionable. The broker does not promise exactly-once task execution. Full delivery to the Director depends on its receive procedure, independent of preview success.

### 5. Finite `message wait`

```text
cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]
```

`--timeout` is an integer in `1..=1800`, default 300. The default suits a host-native background receive: notification retries continue every second while the host avoids a new Director/evaluator turn on each short timeout. A foreground-only mechanism without a live task handle uses an explicit timeout of at most 30 seconds. Validate the active subject and live fleet before waiting. The operation returns existing unacknowledged incoming messages immediately, otherwise checks at one-second monotonic intervals until its deadline. It never ACKs messages or starts a detached process. Its own process is the host-owned finite receive task; transient multiplexer subprocesses remain bounded and owned by the invocation.

Between incoming-message checks, retry pending notifications for active ordinary members in the subject's fleet. Exclude the Director, self-notification and acknowledged/already-notified rows. Keep a round-robin member cursor in memory, attempt at most one member per interval, select its oldest pending preview, and recheck the inbox before starting another attempt. This gives held assignments a retry owner even while the Director is waiting. Multiple waiters remain safe through the shared pane lock and under-lock database revalidation.

The receive loop starts no new I/O after its deadline. A notification attempt has a total five-second budget covering capture, typing, settle and stamp; cap each subprocess/database busy wait to the remaining attempt budget. Stop between stages when the budget expires, preserving pending state and reporting uncertainty if bytes may already have been typed. Consequently the command returns by its deadline plus at most one five-second in-flight attempt/cleanup allowance; injected-clock and subprocess fixtures verify this bound. Normal receiving never blocks indefinitely on a pane or database lock.

JSON is one ordered object:

```json
{
  "member_id": 41,
  "outcome": "timeout",
  "messages": [],
  "notification_errors": []
}
```

`outcome` is `messages` with a nonempty array of the same full records returned by poll, or `timeout` with an empty array. Messages retain poll's descending status-timestamp/message-ID order. Each notification error is `{member_id, message_id, delivery, error}`, where delivery is `not_sent` or `uncertain`; keep only the latest error per member in this invocation. The first attempted preview classified `unknown` or `draft` records `delivery: "not_sent"` and `error: "member ID is STATE; preview MESSAGE_ID was not sent."`, making an unsupported composer or retained draft visible without a hold timer. Working, approval, pending placement and input-lock contention remain ordinary holds without errors.

Revalidate diagnostics at each receive check and before returning. Remove a member's diagnostic when its referenced message is ACKed, that member leaves the active fleet, or that preview is successfully notified by any invocation. A later error for a different pending message replaces the previous diagnostic. This is finite in-memory reporting, with no persistent hold history or forced typing.

Text prints `Messages available for member ID.` plus the normal indexed messages, or `Timed out waiting for member ID.`; notification errors are stderr diagnostics. Both normal outcomes exit 0. A vanished/inactive subject, deleted fleet or unreadable/corrupt broker state exits 1 with its actual cause. Ctrl+C terminates this finite task under normal process behavior; it owns no daemon, persistent lease or task lifecycle to recover.

Before calling wait again, the Director ACKs every delivery it has read, including questions awaiting the user and approvals whose subsequent workflow action is deferred. Deferred work lives in the Director's host-owned task state; every consumed delivery is ACKed before the next wait. This keeps the next wait eligible to block for a new reply without a cursor flag.

The waiter never injects into the Director or its own active pane. Native shell/background results are the Director's receive surface, preventing broker previews from mixing with native input while it waits. A timeout requires the Director to re-read relevant state rather than infer completion or failure from elapsed time.

### 6. Exclusive pane input and readiness

Use `std::fs::File::try_lock`, available before the pinned Rust toolchain, with an owned guard. The [Rust locking contract](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock) supplies process-shared exclusive acquisition, WouldBlock and release when all owning handles close. No new dependency or SQLite execution/lease table is required.

| Property | Contract |
|---|---|
| Lock directory | `<canonical-database-file>.pane-locks/`, adjacent to the selected database |
| Lock key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id`; duplicate registry entries for the same pane share ownership |
| File | `<key>.lock`, opened read/write with create, no truncation, owner-only creation permissions |
| Acquisition | Nonblocking exclusive lock; unexpected filesystem/locking errors propagate with path/member context |
| Lifetime | Acquire before database/placement revalidation and capture; hold through every optional focus key, literal payload, settle, Enter and notification stamp |
| Release | Close the owned non-cloned handle on every path; process exit releases it |
| Stable identity | Keep lockfiles while CAFleet processes may use the database; runtime code never unlinks or replaces them |

Every notification, prompt, ping and exec uses this same guard. Member deletion acquires its placement lock before kill/deregistration. Fleet deregistration acquires active placement locks in sorted key order before its transaction and revalidates that placement set inside the transaction. Deletion retries WouldBlock every 50 milliseconds against one operation-wide five-second monotonic deadline, then releases all acquired guards without changing registry state if any lock remains unavailable.

The exact contention errors (exit 1, no kill or registry change) are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.` A member without a concrete pane needs no lock. A placement-set change during fleet acquisition releases the guards and restarts within the same deadline; database/lock errors preserve their cause.

Read-only pane capture needs no input lock. New-member delivery remains unavailable until the owned pane ID has been patched into placement.

WouldBlock yields a normal held notification or skipped ping, and an explicit prompt/exec error. Readiness is checked after locking, using the recorded target backend and a fresh capture. Replace the permissive finished-composer regex with a transport-readiness result:

| Readiness | Evidence | Input behavior |
|---|---|---|
| `idle` | Affirmative empty composer or verified empty placeholder; no active work, approval or draft evidence | Submit |
| `working` | Streaming, active turn, tool/shell execution or interrupt indicator | Hold/reject |
| `awaiting_user` | Question, confirmation or approval UI | Hold/reject |
| `draft` | Composer contains unsent user text | Hold/reject |
| `unknown` | Missing, truncated, unrecognized or ambiguous UI | Hold/reject |

Approval/work evidence outranks idle cues. Readiness consumes a styled capture before ANSI stripping: tmux capture adds `-e`, and herdr reads `--format ansi` (supported by the installed `herdr pane read --help`). Plain/ANSI public capture output still follows its requested presentation mode. Parse SGR spans within the detected composer and compare their decoration/input roles against captured backend fixtures; a demonstrated dim/grey host suggestion or placeholder is empty input regardless of its changing text. The same visible text in the user-input style is a draft. Grey text outside the recognized composer decoration is not an idle cue.

Claude member argv additionally includes `--prompt-suggestions false`, supported by the installed 2.1.296 CLI, to avoid generating predicted next prompts for unattended members. The styled recognizer still handles a suggestion if the operator enables it during a session, and handles rotating placeholder hints without a fixed text allowlist. Preserve explicit user text and configuration outside the member process. When style or composer evidence is absent/ambiguous, classify unknown and inspect it instead of guessing; default supported themes have captured fixtures proving dynamic empty hints remain deliverable.

Successful input starts from an empty composer. Direct exec types native shell input without a preceding interruption key; ordinary prompts/previews also avoid Escape by default. Any backend-specific idle-focus normalization remains inside the guard and never serves to convert an active/approval pane into idle. Every stage uses a bounded transport deadline.

The lock serializes CAFleet writers, not the user's keyboard or autonomous host UI changes. A readiness capture is point-in-time evidence, so partial or ambiguous transport failure remains uncertain. A timestamp interval alone is insufficient to establish exclusive ownership.

#### Explicit prompt and ping

`member prompt MEMBER_ID TEXT [--json]` requires an active concrete placement and one nonblank input line without CR/LF or terminal control characters. Acquire the shared guard and require `idle`, then send literal TEXT, the backend settle interval and Enter, with no Escape. Preserve successful JSON `{member_id, pane_id, text}` and text `Sent prompt TEXT_REPR to member NAME (PANE).`; exit 0 means submission only. Missing/pending placement keeps the existing contextual error, lock contention uses the §7 held-lock error, and a readiness rejection is `member ID is STATE; prompt was not dispatched.` (exit 1, no typing). Pre-write transport errors use `prompt was not dispatched to member ID: CAUSE`; possible partial submission uses `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` Both exit 1 without a success record. Invalid text exits 2 with `text must be one non-empty line without control characters.`

`member ping MEMBER_ID [--json]` remains an explicit Director action. It acquires the same guard, requires idle, and submits exactly `cafleet message poll ID --json` as literal text plus settle/Enter, with no Escape or resume clause. Finite receiving never generates pings. Successful JSON is `{member_id, pane_id, skipped:false, reason:null}`; the existing successful text is `Pinged member NAME (PANE) — poll keystroke dispatched.` Pending placement, lock contention and readiness states return exit 0 with `{member_id, pane_id, skipped:true, reason:REASON}`; reason is `pending_placement`, `busy`, `working`, `awaiting_user`, `draft` or `unknown`, and pane_id is null only for pending placement. Pending-placement text is `Member NAME has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` Other skips use `Member NAME (PANE) is REASON — ping skipped.` Capture/transport errors exit 1 with their cause and no success record; a possible partial write uses `poll-trigger delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.`

These explicit actions share exec's stage-aware uncertainty and five-second transport budget. Unknown panes are held for ping and rejected for prompt. Composer-native commands must begin the submitted input; the lock and empty-input check prevent a preview from joining that payload.

### 7. Visible direct `member exec`

```text
cafleet member exec MEMBER_ID COMMAND [--json]
cafleet member exec MEMBER_ID --file PATH [--json]
cafleet member exec MEMBER_ID --file - [--json]
```

Exactly one source is required. File/stdin is UTF-8; remove at most one terminal LF or CRLF before validation. Inline text is exact. Require a nonblank command with no CR/LF, NUL or other C0/DEL control character, including tabs. Preserve the validated command's bytes; reject invalid input before multiplexer calls. A user needing multiline work supplies an explicit script invocation as the one command line.

Resolve an active ordinary member and concrete placement, reject the root Director, acquire the shared input lock, reload the target and require `idle`. Build the backend's native shell input as `! ` followed by the actual command, and submit it once. The native backend owns working directory, environment, permission behavior, output and exit rendering. CAFleet writes no script, wrapper, exec identifier, process record or execution-resume message, and waits only for input submission.

Successful JSON is ordered as follows:

```json
{
  "member_id": 42,
  "pane_id": "w1:p2",
  "coding_agent": "codex",
  "command": "printf example",
  "dispatched": true
}
```

Text is `Dispatched native command to member ID (PANE). Inspect the pane for its result.` Exit 0 means the complete input submission succeeded; it says nothing about command success or completion. The host's visible transcript/pane supplies those facts. Capture first after a failed or uncertain dispatch; an operator-authorized retry is a new decision.

| Condition | Exact application-error body | Result |
|---|---|---|
| Invalid command line | `command must be one non-empty line without control characters.` | Exit 2; no pane I/O |
| Director target | `cannot exec in the Director's own pane` | Exit 1; no typing |
| Pending placement | Existing `member ID has no pane yet (pending placement) — nothing to exec.` | Exit 1; no typing |
| Held input lock | `member ID's pane is receiving another input; retry after it is idle.` | Exit 1; no typing |
| Known non-idle/unknown capture | `member ID is STATE; native command was not dispatched.` | Exit 1; no typing |
| Capture/transport failure before any typing | `native command was not dispatched to member ID: CAUSE` | Exit 1; preserve cause |
| Failure after typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` | Exit 1; no automatic retry or success record |

Existing unknown/inactive-member, absent-placement and malformed-file errors remain at their current validation boundary. Classify transport uncertainty by stage, conservatively treating an error from the first text-writing call as possibly typed. A failure after Enter can still mean the command ran. There is no dispatch queue and no `--wait` option.

The Director checks the native command's visible terminal state, then sends an explicit ordinary `member prompt` telling the member to use the output and continue when the host does not resume it. The prompt is sent only after idle evidence; a still-running shell is not ready. Codex's local probe exposed exit code/duration/output to the model, while Claude's exposed command/stdout/stderr without an exit status and an assistant turn followed; the probe did not identify that turn's trigger. CAFleet therefore promises visibility through the host, not uniform exit metadata or automatic resumption.

OpenCode documents native `!` command input and conversation-visible output. Its preset keeps permitted workspace work and broker operations available, with Director-mediated native dispatch for additional commands. Preserve that capability positively without claiming it has a continuation mechanism that was not established by the investigation.

The Director's authorization of `member exec` is the command authorization boundary. CAFleet makes no claim that a member's own model-tool permission rules re-check native shell input.

| Backend | Established provenance | Not established by this investigation |
|---|---|---|
| Codex | Local `! printf` displayed command/exit/duration/output; installed protocol describes unsandboxed dispatch, while the public CLI reference describes current approval/sandbox settings | Effective TUI sandbox boundary and whether the TUI uses that protocol method |
| Claude | Local `! printf` displayed command/stdout/stderr, followed by a model turn | Whether native bash mode re-checks the member's `dontAsk` allow/deny rules; the cause of the following turn |
| OpenCode | Official TUI reference documents `!` and conversation-visible output | Whether native shell input re-checks the member preset's model-tool bash allowlist; no local OpenCode permission probe was performed |

Carry this backend-specific provenance into the backend reference. A harmless command verifies visibility, not a permission boundary; preserve host confirmations and existing user authorization without inventing an additional CAFleet approval bypass.

### 8. Fresh storage format and configuration

Use `sqlite:///$HOME/.local/share/cafleet/cafleet_v8.db` as the default URL. Preserve the existing `cafleet_v7.db` and its sidecars; startup/setup neither migrates nor renames nor deletes them. `CAFLEET_DATABASE_URL` selects an explicit compatible or empty target and is inherited by spawned panes as today.

The fresh baseline sets `PRAGMA application_id = 1128351288` (`0x43414638`, ASCII `CAF8`). Refinery's baseline remains version 1 within this distinct format family. Compatibility checks precede every broker write and both setup halves, including asset-install record writes. A current-format identity alone is insufficient: require the ledger version and expected current table/column shapes as well.

| Selected file | Behavior |
|---|---|
| Empty SQLite file with application_id=0 and no non-internal tables | Setup applies the fresh baseline transactionally, then installs assets |
| Current identity and valid current schema | Normal commands and idempotent setup proceed |
| Current identity with behind/ahead schema | Apply existing version classification within this family; setup upgrades known versions and refuses downgrade |
| Any other populated/identified format or malformed current schema | Exit 1 before content writes or asset installation; preserve file contents |

The incompatible-format error body is `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` It describes the current requirement without advertising a removed feature. Database read/open failures preserve their actual cause. The schema classifier is shared by invocation guard, setup and doctor; doctor reports the format problem while completing its other read-only diagnostics.

The new `V1__baseline.sql` contains:

```sql
PRAGMA application_id = 1128351288;

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    deregistered_at TEXT
);
CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT
);

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT NOT NULL,
    mux_pane_id TEXT,
    backend TEXT NOT NULL DEFAULT 'tmux',
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    from_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    to_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    text TEXT NOT NULL,
    notified_at TEXT
);
CREATE INDEX idx_messages_to_member_status_ts ON messages (to_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_status_ts ON messages (from_member_id, status_timestamp);
```

Retain per-connection foreign-key enforcement and the normal busy-timeout policy, capped by the remaining deadline for bounded receive I/O. Decode required enums/columns with typed errors. A live fleet requires a valid active Director and placement; missing required state fails at its read boundary. A pending `mux_pane_id` remains a valid state only during member creation and its documented diagnostic paths.

The member card is redundant: CLI registration already supplies name/description and has no way to populate member skill metadata. Remove its JSON column, monitor marker and unused skill field from decoded records/output rather than retain an empty duplicate registry. Installed skill assets remain tracked independently by `asset_installs`.

The settings structure retains database URL, max text length and multiplexer selection. Remove HTTP host/port, monitor interval and forced-delivery timeout settings with their environment/help/docs/tests. Wait timing is an explicit finite command argument. There is no stored wake schedule, execution ledger, keystroke timestamp or ready-watchdog stamp.

### 9. Runtime, dependency and artifact reduction

| Area | Final responsibility / deletion closure |
|---|---|
| Broker | Retain typed fleet/member/message/asset storage and pending-preview selection; remove broadcast/summary/grouping, exec/monitor modules, HTTP-only readers and their record types |
| Runtime | Retain multiplexer resolution and correct timed subprocess execution; add small synchronous notification, finite receive and input-guard components |
| Removed processes | Delete monitor loop, monitor CLI, launch/restart/log helpers, signal handlers, synthetic wake payloads, silence/lost-exec notices and their exports |
| Direct exec | Replace CLI implementation; delete ExecRun, queue/dispatch/start/finish/resume bookkeeping, grace clocks and completion notices |
| Multiplexer | Retain split/capture/input/list/kill/layout/cleanup; remove wake descriptors/sender and unused native-status/send-exit methods |
| Backend launch | Remove monitor arguments/defaults/allow rules; preserve ordinary member permissions, backend registry and model/effort validation |
| Server/UI | Delete `admin/`, `cafleet/src/webui/`, `cli/server.rs`, API/SPA tests, host/port config, SPA dist/MIME embedding and generated artifacts |
| Build | Keep generic SKILLS/PRESETS embedding; remove admin task prerequisites and server dev task from root/package task configuration, CI and release |
| Dependencies | Remove axum, Tokio, tower and signal-hook after the final consumer disappears; regenerate lockfile; retain nix for process I/O, regex for formatting/permissions and sha2 for capture/input keys |

Update `cafleet/src/member_permissions.rs` to probe exactly the ordinary-member broker surface, in this order:

```text
cafleet message send --from-member-id 1 --to-member-id 2 x
cafleet message poll 1
cafleet message wait 1 --timeout 300 --json
cafleet message ack 1
cafleet message show 1
```

Remove broadcast/monitor/ping probes: explicit pane control is Director authority, outside the ordinary-member broker diagnostic. Cover wait-specific deny/ask rules and managed-only allow coverage in doctor tests. Claude's existing spawn rule `Bash(cafleet message *)`, the Codex preset's `[cafleet]` allow prefix and OpenCode's `cafleet *` allow rule already cover finite receiving; retain the prompt/exec authorization exceptions. Update `docs/docs/spec/cli-options.md` permissions.allow coverage for `message wait` and the rehomed Director capture command, so their required fixed actions match configured host permissions without adding execution authority.

Preserve `runtime/system.rs` timed pipe draining and owned-child cleanup: those run multiplexer utilities and are independent of removed execution tracking. Retain member spawn compensation and backend-specific layout cleanup. Remove obsolete test fixtures at the same API boundary rather than leaving unused public functions hidden from dead-code analysis.

The documentation site continues to use its existing Node/pnpm toolchain. Remove the admin config root, all-install prerequisite, CI lint/test steps, release install/build steps and obsolete ignore/permission entries by their actual consumers. The concrete deletion targets are listed in the implementation steps below, so implementation does not depend on hidden local scan files. Living docs describe selecting properties or authoritative registries; this design names the exact point-in-time removal targets.

### 10. Skills, workflow and documentation contracts

Discovery leads with agent multiplexing and proactive substitution for subagents/agent teams. It names the delegation situations and the available independent sessions, without exposing backend continuation control commands. The executing host manages continuation as an existing context; workflows preserve the user's controls and introduce no activation or availability check.

| Surface | Intended content |
|---|---|
| `skills/cafleet/SKILL.md` | Short proactive trigger, broker/member identities, direct command index and responsibility split |
| Director role / supervision reference | Assignment/readiness, finite receive/check, visible native dispatch, evidence-based recovery and teardown |
| Member role / routing reference | Ready, startup/turn-boundary poll, precise reports, routed required commands, idle wait and explicit post-command continuation |
| Backend reference | Runtime/permissions/model defaults plus only host-specific receive/dispatch facts needed to execute correctly |
| Design-document core/workflow bodies | Existing create/interview/execute roles and review loops, using simplified fleet creation and finite Director receiving |
| Presets | Existing member broker capability and Director-only prompt/exec boundaries; actual permission registries remain authoritative |
| Human docs / SPEC | Current CLI, storage, pane/receive semantics and architecture, independently written for their audience |

Delete the monitor role and monitor model/effort placeholders. Remove native-subagent dispatch instructions from skill loading examples where they contradict CAFleet delegation. Keep reviewer/member backend selection, cost policy and model catalogs tied to their existing source of truth; model-catalog freshness is separate from this structural edit.

The backend reference preserves the observed Codex/Claude distinction: Codex has thread lifecycle/tools and tool-bearing idle continuation; Claude evaluates transcript conditions and can defer evaluation around native background work. Both need explicit CAFleet evidence and real receive-task ownership. OpenCode's member capabilities are described through its assignments, tools, provider/model choice, broker and native shell path. Research keeps detailed lifecycle tables, commands, untested stop conditions and the Codex permission discrepancy; ordinary workflow prose stays focused on the actions the operator/agent performs.

Preserve the existing five-subsection backend template. During implementation, bind observed host-receive and composer-input facts through `Note → applies at` rows pointing to their consuming Director/routing instructions, and state shared operational facts independently in `docs/docs/spec/coding-agent-backends.md`. Publish these evidence notes and the finite receive instructions together with the corresponding runtime change.

#### Discovery and backend-evidence implementation handoff

Use this proposed discovery description for `skills/cafleet/SKILL.md`:

```yaml
description: >-
  Multiplex coding agents in tmux or herdr and coordinate them through durable
  messages. Proactively use CAFleet when delegating work that calls for
  subagents or agent teams, including parallel research, implementation, and
  independent review. Also use for CAFleet member lifecycle, messaging, and
  pane interaction.
```

Add a command-index entry for host continuation, native waiting and evidence boundaries, pointing at the selected backend's bound notes in `reference/coding-agents.md`. Set Claude's `{skill_loader}` binding to `the Skill tool`; skill loading and delegation selection have separate responsibilities, so the loader binding contains only the loading mechanism.

Carry the following verified distinctions into the backend notes and the independently written human backend specification:

| Backend / evidence | Required fact and consuming instruction |
|---|---|
| Claude Code 2.1.296, checked 2026-10-22 | A separate evaluator checks the session condition against the transcript. Native background work defers evaluation and returns its result through the host; external CAFleet panes contribute evidence when the Director reads broker or pane state. Bind this to Director receiving and turn boundaries. |
| Claude continuation limit | An unmet condition can start another turn immediately when no native background work remains. Consecutive tool-free stops reach the configured block cap, eight by default. The finite receive task gives outstanding external work a native result handle. |
| Claude composer input | Native composer controls take effect at the beginning of submitted input. A local probe merged control text into a concurrent broker preview as prose. Bind input serialization and capture-based verification to the Director's pane-input instructions. |
| Codex CLI 0.162.1, checked 2026-10-22 | The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. A two-turn tool-bearing probe verified automatic continuation and model-reported completion. Preserve the user's objective, activation authority and budget, retain live tool handles, and collect explicit broker/pane evidence. |
| Native shell authorization | Treat routed native input as a Director-authorized host action. The harmless probes established command visibility rather than a second member-tool permission check. Installed Codex protocol metadata and public CLI guidance differ on sandbox behavior; retain that uncertainty rather than claim equivalent enforcement. |
| OpenCode member context | Members perform supplied assignments through their selected model, workspace tools, preset, broker and native shell interface. The Director coordinates their next steps and routed operations. An actual OpenCode permission probe was outside this investigation. |

Primary provenance for these implementation notes: [Claude host continuation](https://code.claude.com/docs/en/goal), [Stop-hook block cap](https://code.claude.com/docs/en/hooks-guide#stop-hook-hits-the-block-cap), [Codex host continuation](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), [Codex command reference](https://learn.chatgpt.com/docs/developer-commands), and [OpenCode shell input](https://docs.opencode.ai/docs/tui/#shell-commands). Local probe results establish only the exercised behavior; detailed evidence remains linked from Background.

The create workflow retains the mandatory pre-draft clarification round, sole Drafter authorship, independent Reviewer iteration and explicit user approval. Interview keeps its read-only analysis and complete payload relay. Execute keeps TDD, phase gates, role boundaries, verifier tool discovery and review loops. Update their bootstrap/receive/recovery/teardown instructions to the same CLI contracts without changing their user gates or COMMENT coordination protocol.

Update docs first in the eventual implementation change, including every affected first-class documentation target: documentation-site content/navigation, README, SPEC, shipped skills and applicable rules. Rehome pane-readiness/notification concepts before removing monitoring pages. Delete obsolete WebUI/HTTP pages and inbound links. Skill runtime mirrors are symlinks into the docs tree; delete obsolete symlinks together with their targets so embedding follows no dangling path.

Update project helper skills and rules whose selecting content mentions removed commands, packages or roles, including command/task guidance, backend overlay structure, documentation maintenance and schema-chain guidance. The chain guard actually lives in `cafleet/src/db/mod.rs`; make its rule point at that source. Source, user docs, examples and tests describe the final state; design documents and git retain historical decisions.

### 11. Validation and implementation boundary

This phase produces investigation and this reviewed specification. Keep repository changes in the design document; the implementation handoff above preserves the proposed discovery wording and verified backend evidence. Apply skill, living-documentation, runtime, schema and build changes together in a subsequent authorized implementation phase. In this investigation/design phase, the design and its hidden artifacts stay uncommitted per the session's instruction; a later implementation session follows its own instructions and applicable repository rules.

| Verification | Required evidence |
|---|---|
| Fresh database | Current identity/table shapes, idempotent setup, FK behavior, byte-preserved predecessor, and explicit incompatible custom-path rejection before either setup write |
| Registration | Fleet creates no pane/process; immediate ordinary spawn succeeds; placeholder/split/placement/cleanup failures preserve ownership and causes; member/fleet delete contention either clears within five seconds or emits its exact error with no mutation |
| Notification | Director receives no injected preview; pending/active/ACK states, oldest-first selection, idle-only delivery and control rendering are truthful; unknown/draft preview attempts produce immediate diagnostics that clear on ACK/member departure/success, and retained drafts trigger capture/user relay or authorized context-preserving recovery |
| Finite receiving | Immediate pending reply, normal/background and bounded-foreground timeout, arrival near deadline, round-robin retries, busy-to-idle delivery, deleted subject/fleet, transport failures and bounded I/O; a read question is ACKed while its user decision remains pending, and a later reply blocks/returns normally without a busy loop |
| Input exclusion | Concurrent prompt/ping/preview/exec cannot interleave; prompt/ping idle/error/skip/output contracts pass; member/fleet teardown contention is bounded; process death releases the guard and stable lockfile identity is retained |
| Composer readiness | A changing Claude suggestion/placeholder in its host decoration style is idle, the same text typed by a user is draft, and disabled prompt suggestions appear in spawn argv; verify styled captures and supported backend themes |
| Direct execution | Source exclusivity/UTF-8/terminal newline/control validation, idle/draft/approval/busy/unknown rejection, literal visible command, no ledger/helper/wrapper and uncertain-stage output |
| Completion handoff | Harmless native command output is inspectable; a non-resuming member continues only after an explicit Director prompt |
| Backend/mux | Deterministic fixtures plus harmless installed-host smoke coverage for supported shell interfaces across retained mux adapters; OpenCode behavior is checked without weakening its member permissions |
| Artifact closure | Removed commands/fields/modules/dependencies/pages/symlinks are absent; links/embedding and reduced workflow checks pass; doctor diagnoses wait-specific deny/ask/managed-only rules and current broker probes match spawn/preset/docs permission coverage |
| Build/release | Repository Rust tests/lint/typecheck and docs checks pass; release builds require no admin output; record before/after production/living-doc counts |

Capture the implementation's source and documentation counts against the investigated baseline, excluding design/research evidence, generated/vendored content and lockfile churn. Report both the reductions and newly added receive/locking code so removal is measurable. The baseline test count is historical evidence, not a target to preserve after obsolete coverage is deleted.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update current contracts and instructions

The essential removal closure is self-contained below. These are the exact targets identified in this checkout; resolve additional current references through the same affected surface.

| Target | Required closure |
|---|---|
| `docs/docs/concepts/monitoring.md`, `docs/docs/how-to/use-the-webui.md`, `docs/docs/spec/webui-api.md` | Rehome retained pane/receive content, delete pages and owning `_meta.json` entries, and update inbound links; Rspress `checkDeadLinks` remains enabled |
| `skills/cafleet/reference/runtime/concepts/monitoring.md`, `skills/cafleet/roles/monitor.md` | Delete the mirror symlink and monitor role; remove role/anchor/template references |
| `docs/docs/spec/cli-options.md`, `data-model.md`, `multiplexer-backends.md`, `coding-agent-backends.md`, `message-envelope.md` | Replace affected CLI, point-to-point records, native-input/readiness and permission contracts; remove grouping fields and removed command/configuration descriptions |
| `docs/docs/concepts/overview.md`, `coding-agents.md`, `member-lifecycle.md`, `storage.md`; retained how-to/quickstart/contributor pages | Rewrite bootstrap, receive, routing, teardown, storage filename and package/build descriptions for their current audience |
| `cafleet/tests/docs_sync.rs` | Replace monitor role/page anchors, WebUI API, SPEC §6.6, monitor options/defaults and wake-payload pins in the same change as docs; cover the new receive/point-to-point/current links |
| `.claude/rules/database-migrations.md` | Point the chain-guard instruction at `cafleet/src/db/mod.rs` (investigated line 377), keeping the embedded migration registry authoritative |
| `.claude/rules/commands.md`, `bash-tool.md`, `coding-agent-overlay.md`, `documentation-maintenance.md`; `CLAUDE.md` | Rewrite obsolete server/admin/monitor/helper instructions and maintain backend subsection/note ownership |
| `.claude/skills/skill-author`, `clean-docs`, `residue`, `cafleet-model-list-refresh`, `update-readme` | Remove affected monitor/WebUI/broadcast examples and retain current helper responsibilities |

- [ ] Update documentation-site concepts/specifications/navigation for the reduced CLI, receive/notification behavior, direct execution and fresh storage format; remove obsolete pages and runtime mirror symlinks. <!-- completed: -->
- [ ] Update README and SPEC to the final responsibilities, affected outputs/errors, schema and build contracts, preserving their required detail and structure. <!-- completed: -->
- [ ] Rewrite CAFleet and design-document roles/workflows/references to proactive multiplexing and host-owned finite receiving; preserve existing clarification/review/user gates and positive OpenCode capabilities. <!-- completed: -->
- [ ] Update affected presets, project rules and helper skills; remove monitor placeholders, native-subagent dispatch examples, obsolete command/task references and duplicated registry descriptions. <!-- completed: -->

### Step 2: Establish the fresh database format

- [ ] Select `cafleet_v8.db`, implement shared format/shape diagnosis with application_id `1128351288`, and reject incompatible explicit paths before normal writes. <!-- completed: -->
- [ ] Replace the fresh baseline and decoded records with the schema in this document; remove execution/supervision/member-card state and simplify related queries/presenters. <!-- completed: -->
- [ ] Gate both setup halves and doctor/CLI behavior on current-format diagnosis; verify predecessor/custom-file preservation and transactional initialization. <!-- completed: -->
- [ ] Update schema/chain/asset fixtures and fail-fast enum/required-column checks for the new format. <!-- completed: -->

### Step 3: Simplify registry and lifecycle operations

- [ ] Replace fleet bootstrap with atomic current-Director registration and its reduced text/JSON output. <!-- completed: -->
- [ ] Remove member-role guards and monitor spawn parameters while retaining backend inheritance, identity substitution and owned-pane/registration compensation. <!-- completed: -->
- [ ] Rehome batch capture under `fleet capture`, narrow member kinds/output and preserve member/fleet deletion semantics with input ownership. <!-- completed: -->

### Step 4: Serialize native pane input

- [ ] Implement the stable-key OS file-lock guard, bounded acquisition/errors and shared use by every pane-writing/lifecycle mutation path. <!-- completed: -->
- [ ] Implement styled backend-specific readiness checks and Claude `--prompt-suggestions false`, distinguishing host decorations from identical user text, working/approval and unknown captures. <!-- completed: -->
- [ ] Add checked literal submission with stage-aware uncertainty and bounded transport deadlines; remove forced interruption behavior and timestamp claims. <!-- completed: -->
- [ ] Verify multi-process serialization, slow input, partial failures, duplicate placement keys and lock release on process exit. <!-- completed: -->

### Step 5: Implement finite receiving and notification retries

- [ ] Remove broadcast/summary/grouping code and simplify synchronous point-to-point notification, with Director exclusion and truthful committed-message output. <!-- completed: -->
- [ ] Implement `message wait` timeout validation, full result shape, inbox checks, 300-second background default/foreground guidance, monotonic deadline and capped per-operation I/O. <!-- completed: -->
- [ ] Implement in-memory round-robin ordinary-member preview retries, under-lock revalidation/stamping and bounded diagnostics without persistent runtime state. <!-- completed: -->
- [ ] Verify immediate replies, timeout/arrival races, busy-to-idle retry, ACK-on-read with deferred decisions and later replies, multiple waiters and the no-continuation ordinary delegation flow. <!-- completed: -->

### Step 6: Replace tracked execution with native dispatch

- [ ] Implement the one-line COMMAND/file/stdin contract, native backend shell-input framing and dispatch-only text/JSON/error output. <!-- completed: -->
- [ ] Delete ExecRun, `--wait`, queue/process/exit/resume code, notices, fixtures and execution-based delivery holds. <!-- completed: -->
- [ ] Verify literal command visibility, every readiness rejection and partial-send uncertainty; smoke-test native completion inspection and explicit continuation. <!-- completed: -->

### Step 7: Complete runtime, UI and build reduction

| Target | Required closure |
|---|---|
| `admin/`, `cafleet/src/webui/`, `cafleet/src/cli/server.rs` | Delete frontend/server, exports, HTTP config and SPA embedding |
| `cafleet/src/monitor/`, `cafleet/src/cli/monitor.rs`, `cafleet/src/broker/monitor.rs`, `cafleet/src/broker/execs.rs`, `cafleet/src/notices.rs` | Delete scheduler/exec lifecycle modules; rehome specified capture and synchronous notification behavior |
| `cafleet/tests/monitor_loop.rs`, `monitor_uniqueness.rs`, `webui_routes.rs` | Delete feature-specific suites; move required receive/locking/lifecycle cases to current suites and remove shared/inline obsolete fixtures |
| `cafleet/tests/cli_exec.rs`, `cli_fleet.rs`, `cli_member.rs`, `cli_global.rs`, `cli_message.rs`, `delivery.rs`, `e2e.rs`, `cli_setup_doctor.rs` | Replace tracked/broadcast/server/monitor assumptions with current point-to-point, native-input, schema and receive contracts |
| `.claude/settings.json` | Remove `Bash(mise //admin*)` (investigated line 5) with obsolete tasks |
| `.gitignore` | Remove `cafleet/webui-dist/` and `admin/node_modules/` entries |
| Root `mise.toml` | Remove admin from config roots and all-install (investigated lines 6,26); retain docs tooling |
| `cafleet/mise.toml` | Remove every `//admin:build` dependency and the server `dev` task |
| `.github/workflows/ci.yml`, `.github/workflows/release.yml` | Remove admin install/lint/test steps (CI lines 25-26,44-45) and install/build steps (release lines 30-31) |
| `cafleet/build.rs`, `embedded.rs`, `Cargo.toml`, `Cargo.lock` | Keep skills/presets embedding, remove SPA dist/MIME and unused dependency consumers, regenerate lockfile |

- [ ] Delete monitor/scheduler/HTTP/SPA code and frontend package; remove unused broker/multiplexer APIs, exports, configuration and generated-artifact references. <!-- completed: -->
- [ ] Reduce embedding to skills/presets and remove admin requirements from task roots, Rust tasks, CI/release, ignore rules and permission entries. <!-- completed: -->
- [ ] Remove dependencies with no surviving consumers, regenerate Cargo.lock and retain correct general process I/O/capture/permission support. <!-- completed: -->

### Step 8: Verify the complete reduced product

- [ ] Run meaningful retained/new Rust tests through `mise //cafleet:test`, plus the repository lint/typecheck tasks, and fix failures within the specified contracts. <!-- completed: -->
- [ ] Run documentation/link/embedding checks, verify removed-surface closure and backend/multiplexer smoke evidence, and record net production/documentation reductions. <!-- completed: -->
- [ ] Review final skills, presets, normal delegation and design-document create/interview/execute flows for consistency with the shipped CLI and host behavior. <!-- completed: -->
