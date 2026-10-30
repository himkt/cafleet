# CAFleet CLI reference

Exact command facts the CAFleet skills tell an agent to run, parse or act on.

## Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize/migrate the current database and install assets | — | no |
| `doctor` | Diagnose multiplexer, database, agent assets and member runtimes | — | yes |
| `fleet create` | Register the calling Director atomically | — | yes |
| `fleet list` / `fleet show` | Read fleet registry | `FLEET_ID` for show | yes |
| `fleet delete` | End a fleet's ordinary members, then deregister the fleet | `FLEET_ID` | yes |
| `member create` | Register and spawn an independent agent | `--fleet-id` relationship | yes |
| `member delete` | Kill its owned pane and deregister | `MEMBER_ID` | yes |
| `member show` / `member list` | Read detail / active roster | `MEMBER_ID` / `FLEET_ID` | yes |
| `member prompt` / `member exec` | Submit authorized native input | `MEMBER_ID` | yes |
| `member press` | Request explicit ordered keys | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `member recover` | Reconcile the recorded run without restarting an agent | `MEMBER_ID` | yes |
| `event show` | Read a durable runtime event and current state | `EVENT_ID` | yes |
| `input show` / `input reconcile` | Read / settle a durable input receipt | `INPUT_ID` | yes |
| `permission inspect` / `permission respond` | Inspect / answer the current native permission request | `INTERACTION_ID` | yes |
| `message send` | Persist a point-to-point message and preview it for an ordinary recipient | Sender/recipient flags | yes |
| `message poll` / `message wait` | Read incoming deliveries / block until deliveries arrive | `MEMBER_ID` | yes |
| `message listen` | Stream pending inbox records through stdout | `MEMBER_ID` | JSONL only |
| `message ack` / `message show` | Consume / read one message | `MESSAGE_ID` | yes |

`cafleet --version` prints `cafleet <version>` and exits 0.

## Positional subject ids

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

`message send` requires integer `--from-member-id` and `--to-member-id` flags. Both must be active in the same live fleet; the sender row determines the fleet.

## Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `CAFLEET_DATABASE_URL` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v8.db` | Selected SQLite file path |
| `CAFLEET_MULTIPLEXER` | Auto-detect | Explicit supported multiplexer override (`tmux` or `herdr`) |
| `CAFLEET_MAX_TEXT_LEN` | `200` | CLI text-output body codepoint limit |

An explicit URL has the form `sqlite:///<path>`; a relative file path resolves against the invoking working directory. A literal tilde in an override is not expanded. Count values accept nonnegative integers and invalid values fail with `<NAME> must be a non-negative integer (got '<value>')`.

The backend config-location variables `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `OPENCODE_CONFIG_DIR` relocate a backend's config directory. A set variable must hold an absolute path; any other value fails with `Error: <VAR> must be an absolute path (got '<value>')`, exit 1. `member create` forwards the selected backend's variable, when set, into the member's environment together with `CAFLEET_DATABASE_URL`.

## Output shapes

| Command | Text | JSON |
|---|---|---|
| `fleet create` | `<fleet-id> director=<member-id>` | Ordered `fleet_id, name, created_at, director`, with Director identity/description/registration/placement |
| `fleet list` | `FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT` columns | Fleet records |
| `fleet show` | Fleet row with `deleted_at:` | Fleet including nullable `deleted_at` |
| `fleet delete` | `Deleted fleet ID. Deregistered N members.` plus one line per ordinary member | `{fleet_id, deregistered_count, members}` |
| `message send` | `Message sent.` then envelope, delivery line and any input line | `{message, notification_sent, input_id, delivery}` |
| `message poll` | Envelopes, or `No messages found.` | Full record array |
| `message ack` | `Message acknowledged.` then envelope | `{message}` |
| `message show` | Envelope | `{message}` |
| `message wait` | Availability/timeout line and any messages | `{member_id, outcome, messages, notification_errors}` |
| `message listen` | — | Tagged `ready`, `message`, `closed` JSONL records; no `--json` flag |
| `member create` | `ID NAME backend=AGENT pane=PANE` plus runtime line | Member with placement and runtime |
| `member delete` | `Member deleted.` with member/pane lines | `{member_id, pane_status}` |
| `member show` | `ID NAME STATUS` plus wrapped runtime line | Detail including kind, placement and runtime |
| `member recover` | Recovery counts/state and cleanup lines | Ordered recovery result |
| `event show` | Event, runtime and optional interaction lines | Event and current state |
| `input show` / `input reconcile` | Input receipt and optional decision lines | `{input}` |
| `permission inspect` | Request identity/capability lines | `{request}` |
| `permission respond` | Answer submission status/stage line | `{response}` |
| `member list` | Roster rows or `0 members.` | Roster array |
| `member prompt` | `Sent prompt TEXT_REPR to member NAME (PANE).` or the queued line, plus delivery and input lines | `{member_id, pane_id, text, input_id, delivery}` |
| `member exec` | `Dispatched native command to member ID (PANE). Inspect the pane for its result.` or the queued line, plus delivery and input lines | `{member_id, pane_id, coding_agent, command, dispatched, input_id, delivery}` |
| `member press` | `Acknowledged keys KEYS_JSON for member ID (PANE).` | Ordered `member_id, pane_id, keys, status` |
| `member capture` | Exact selected content | `{member_id, pane_id, lines, content, captured_at, content_sha256}` |
| `doctor` | One `STATUS NAME: DETAIL` line per check | `{checks}` |

The ordered public message record is `message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text`. `to_member_id` is always an integer. ACK changes `status_state` from `input_required` to `completed` and stamps `status_timestamp`; completion means the delivery was consumed, independently of the assignment's progress. The text envelope is `[MESSAGE_ID | from:SENDER_ID | STATUS_TIMESTAMP]` with the body on the next line when nonempty. Text bodies use `CAFLEET_MAX_TEXT_LEN` codepoints plus `…`; JSON preserves the complete exact text.

### Member list output

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane. Text idle values render as `Ns` / `Nm` / `Nh`.

## JSON output

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text, with the structured `permission respond` exception; parse errors exit 2. Commands marked no in the subcommand summary reject the flag, including the root group.

## `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Verify native Monitor authorization separately: a Bash-rule check establishes no Monitor permission. Literal IDs and trailing JSON flags preserve prefix matching.

Keep commands that send explicit native pane input under `permissions.ask`, including the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes. Ask rules outrank allow rules. Authorize `cafleet permission respond` under the Director host’s effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses.

A claude member receives `Bash(cafleet message *)` as a spawn-time allow rule. The codex rules file allows every `cafleet` command and prompts for `member prompt`, `member exec`, `member press`, `permission respond`, `input reconcile` and `member recover`. Both OpenCode presets allow `cafleet *` and deny those same Director-owned commands.

## Text bodies

`message send` and `member create` require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the diagnostics in § Error messages. `member prompt` takes one positional line.

## Fleet commands

`cafleet fleet create --name NAME --coding-agent AGENT [--json]` requires a valid tmux/herdr context; outside it, it fails with `cafleet fleet create must be run inside a tmux or herdr session`. The calling pane becomes the root Director's placement; creation splits no pane and launches no process, and an ordinary member can be created immediately. Root-Director messages remain in the durable inbox for native or manual reads; automatic pane previews target ordinary members.

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. An unknown ID fails with `fleet 'X' not found.`, exit 1.

`cafleet fleet delete FLEET_ID [--json]` ends every active ordinary member that owns a concrete pane through the `member delete` procedure, then deregisters the remaining members and the fleet. It retains messages, leaves the root Director's pane open and is idempotent. One five-second deadline bounds the command.

| Outcome | Result |
|---|---|
| Success | `Deleted fleet ID. Deregistered N members.` followed by one `  member MEMBER_ID: PANE_STATUS` line per ordinary member in ascending id; `PANE_STATUS` is `PANE (killed)`, `(pending — no pane)` or `(no placement)` |
| Lock contention until the deadline | Exit 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`; every pane and registry row is unchanged |
| A member cannot be ended | Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered, and a rerun resumes |
| Placement set changed after cleanup began | Exit 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` |

## Messages

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

The send persists one point-to-point delivery, then evaluates delivery for it. A self-send (equal sender and recipient IDs) is evaluated first and persists only in the broker, even for an ordinary member with a healthy wrapped pane. Other root-Director messages use native or manual broker reads with no pane-delivery diagnostic and no multiplexer requirement. Otherwise the send drains an ordinary recipient's pending messages oldest-first into its concrete pane within a five-second budget, the new message included. A committed send remains successful with exit 0; queued, held or unavailable ordinary-member notifications preserve the stdout record and `notification_sent=false`.

`notification_sent` is `true` exactly when this invocation submitted the newly committed message, including when it was drained after an older one. When another invocation delivered the new message, `notification_sent` stays `false` and stderr carries no diagnostic. Messages left pending when the budget no longer covers another submission stay pending with no diagnostic; the next send to that recipient or a `message wait` rotation retries them. A preview the recipient's runner queued is retried by the runner itself after the blocking native interaction closes.

JSON is ordered `message,notification_sent,input_id,delivery`. `input_id` is the nullable durable input ID of the new message's preview receipt, and `delivery` is the [delivery projection](#delivery-reporting) for the newly sent message, even when the same invocation also retries an older one:

| Recipient | `delivery` | `input_id` |
|---|---|---|
| The sender itself | `{"status":"not_applicable","method":null,"reason":"self_send"}` | `null` |
| Another sender's root Director | `{"status":"not_applicable","method":null,"reason":"director_stdout"}` | `null` |
| An ordinary member | The projection of this invocation's evaluation of the new message | The receipt's ID when allocated |

Text adds `Delivery: status=STATUS method=METHOD reason=REASON` after the envelope, using `-` for a missing method or reason, then `Input: INPUT_ID` when an ID is allocated.

### Delivery outcome

A notification failure prints `Message NEW_ID was persisted. ` followed by its diagnostic on stderr, without suppressing successful stdout. Every per-attempt failure uses this diagnostic:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| The pane input lock stays busy through the lock wait | `pane input remained busy` |
| Lock acquisition error | `pane input: <lock error>` |
| Multiplexer resolution or availability error | The multiplexer error text |
| Budget cannot cover literal input, settle and Enter before the first line | `insufficient pane input budget for literal input, settle and Enter` |
| Transport error before or during typing | The transport error text |
| Database busy or deadline exceeded during the under-lock snapshot or notification stamp | The storage error text; names the current candidate, or the candidate from placement revalidation when selection has not completed |
| `message wait` attempt fails with database busy or deadline exceeded before selecting a candidate | The storage error text; names the selected rotation entry |

A retirement failure after a wrapped delivery settled reports `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE` and keeps `notification_sent=true`. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Existing unread deliveries return immediately; otherwise the command checks at one-second intervals. It is a blocking receive for scripts and manual use and never ACKs messages. Between inbox checks, it retries pending deliveries for at most one ordinary member per interval, excluding the root Director and its own subject; one invocation attempts each pending message at most once.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic has ordered keys `{member_id, message_id, error}`, one per not-notified message, ordered by message ID. Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause. A receive returns by its deadline plus at most one six-second in-flight attempt/finalization allowance.

### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS]`

Streams any active member's pending inbox through UTF-8 JSONL. It requires a live root Director with a concrete registered placement and resolves no multiplexer. Unknown/inactive startup subjects use `Member ID not found`, exit 1.

| Surface | Contract |
|---|---|
| Lease | Optional integer `1..=1800`; absence runs until closure, cancellation or failure |
| Expiry | One monotonic lease from startup; activity leaves expiry unchanged |
| Output | Compact tagged JSON object plus LF, flushed per record; JSON strings escape controls/newlines |
| Startup | Validate/pin identity, then emit and flush `ready` before replay |
| Idle | Check immediately, then wait one second after a snapshot emits zero new IDs, shortened by the lease |
| Completion | Flush `closed`, then exit 0 for expiry or normal subject/fleet closure |
| Failure | Diagnostic on stderr and exit 1; parser usage errors retain exit 2 |

The command always emits JSONL; text mode and `--json` are absent. Stdout contains protocol records only:

```json
{"event":"ready","member_id":1,"fleet_id":1}
{"event":"message","message":{"message_id":12,"from_member_id":2,"to_member_id":1,"created_at":"2026-10-25T00:50:00.000000+00:00","status_state":"input_required","status_timestamp":"2026-10-25T00:50:00.000000+00:00","text":"complete (doc)\nFull original body"}}
{"event":"closed","member_id":1,"reason":"timeout"}
```

The inner message is exactly the full message record, preserving original text beyond CLI/pane limits. `closed.reason` is `timeout`, `member_inactive` or `fleet_deleted`; fleet deletion takes precedence when both lifecycle closures are observed. Records are ordered by `(created_at, message_id)`, and a process-local ID set suppresses already-emitted pending IDs.

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch. Concurrent listeners are independent read-only observers; restart replays all pending IDs. ACK racing with buffered output may leave a stale pending envelope. Reconcile known/replayed IDs with `poll --json` or `show --json` before acting, record deferred actions and explicitly ACK consumption. Completed records need no second action or ACK. Broken pipe, write-zero, flush failure or output deadline exhaustion terminates with exit 1; host cancellation may end the stream without `closed`.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. ACK signifies consumption.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record.

## Push notifications

Active ordinary recipients with concrete panes receive previews through their recorded transport. The root Director receives through `message listen` or manual broker reads, with zero automatic pane input. Self-sends persist without pane input.

A runner-managed (wrapped) member receives this exact line through profile delivery:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

A wrapped member receiving through delivery fallback, and an explicitly unwrapped member, receive this exact line, because their submission leads with Escape:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where N counts stored text. This limit is independent of `CAFLEET_MAX_TEXT_LEN`. The runner chooses the wrapped form from the selected delivery method before the first byte. Delivery leaves `status_state` unchanged: a previewed message remains `input_required` until its recipient consumes and ACKs the message ID.

### Wrapped delivery methods

The runner selects each wrapped preview, `member prompt` and `member exec` delivery by these ordered rules:

1. A failed identity, reachability, ownership, storage or receipt-history check, or an uncertain prior attempt, is an explicit failure and sends no fallback bytes. Unknown detection coverage alone is not such a failure.
2. A candidate or open detected interaction (`permission_request`, `question`, `trust`, `update` or `unknown_interaction`, including while obscured), or earlier queued input, queues the request with zero bytes, including zero Escape bytes.
3. A verified empty composer with the operation's measured submission plan uses profile delivery: the literal stage, a 500 ms settle, then the measured submit key, without Escape.
4. With a measured plan and a composer that is not verified empty (a draft, or a screen the observer cannot classify), the request queues with zero bytes under `composer_busy` and drains by profile delivery once the composer is verified empty. A working member whose composer row is empty receives profile delivery, which the native CLI queues behind its current turn. After 3 minutes since its `deferred_at` without a verified empty composer, it is delivered by fallback with reason `idle_timeout`. Interrupt a working member explicitly with `cafleet member press MEMBER_ID Escape`.
5. Otherwise the runner uses delivery fallback: Escape, 500 ms settle, the literal (with `! ` for exec), 500 ms settle, Enter. This covers a missing plan, profile or version probe.

| Selection | `delivery.method` | Reason |
|---|---|---|
| Queued | `null` until an attempt selects one | `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input` |
| Profile delivery | `profile` | `null` |
| Fallback without an executable plan, profile or version probe | `fallback` | `submission_plan_unavailable` |
| Fallback after 3 minutes queued behind a composer that is not verified empty | `fallback` | `idle_timeout` |

Fallback runs through the runner's own child PTY and receipts and rechecks for a newly detected interaction before Escape and before every later stage. It can interrupt work, mix text with a draft, or dismiss an undetected permission request or question on an unknown-coverage run. Permission answers never use fallback.

### Runner queue

The runner owns a FIFO of ordinary input, including previews, bounded with active work by the 4096 unresolved-input limit; a full queue returns an explicit zero-byte capacity error. Queue acceptance commits a durable `ready` receipt with `deferred_at` and `defer_reason`; the literal stays in runner memory only. One request writes at a time in enqueue order. After an `interaction_closed` transition commits, a candidate clears or the active input completes, the runner rechecks the current state and drains the head; no further `message send`, `message wait` or Director wake is needed. A superseding interaction keeps the queue blocked, and closure from exit or monitor loss dispatches nothing. A queued preview whose message was ACKed or whose target left is settled not submitted rather than typed.

A queued request outlives its CLI invocation; a client timeout after acceptance reports the known input ID for inspection with `cafleet input show INPUT_ID --json`. Each drain attempt has a five-second budget and retries a busy pane lock or temporary storage/observer unavailability on a 100 ms timer. The first deferred submitted, not-submitted or uncertain result is published to the Director as one `input_delivery_result` runtime event; a zero-byte expiry settles not submitted with reason `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`, and a consumed or changed target uses `message_consumed`, `target_inactive` or `target_changed`. Shutdown settles queued requests not submitted with `runner_stopped`; recovery after runner loss uses `runner_lost`. Payloads are never restored from disk.

### Held delivery

Wrapped delivery remains held, with no queue acceptance or write, for a degraded/recovering/lost observer, unavailable ownership or storage, or an uncertain receipt/reachability. Queued and held delivery leave `notified_at` unset, create no new permission event and grant no native answer.

Pending placement and a placement change under the lock are holds. A message remains pending while the lock stays busy, the pane is not yet placed, the budget runs out or an attempt fails. The next send to that ordinary recipient, or a `message wait` rotation, retries it. Explicitly unwrapped delivery is at-least-once: a failed stamp leaves the message pending, so a later send/wait may repeat partial or complete input.

### Accepted trade-offs

The following trade-offs apply to explicitly unwrapped input and to wrapped delivery fallback, both of which lead with Escape. What a single Escape does depends on the backend, and a backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both trailing clauses are conditional, so they stay accurate either way.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the host can report a user rejection before the input arrives; Claude Code reports `The user doesn't want to proceed with this tool use.` The preview's dismissed-prompt clause marks that rejection as delivery-caused, and the member re-issues the required tool call or re-asks the question. `member prompt` and `member exec` input carries no clause; the Director checks the pane for an open prompt before that dispatch.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- Interrupting a host turn can also end its in-flight shell tool; the CLI defers termination signals across its pane-input spans.
- Opencode mini captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; use broker poll and ACK to establish message consumption.

## Member commands

Pane operations require an active member and placement; pane IDs are opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Prompt, exec and press reject the root Director before pane I/O, and individual member deletion protects it.

### `member create`

`member create` registers a member **and** spawns its coding-agent pane. The acting Director is auto-resolved from the fleet's root Director, so no identity flag exists.

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins |
| `--name` | yes | Display name; only claude panes display it as the pane title |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | `claude`, `codex` or `opencode`; when omitted, the member inherits the spawning Director's placement backend |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag; opencode requires `<provider-id>/<model-id>` |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend's policy catalog; pass it only for a mode other than the backend default |
| `--event-evidence POLICY` | no | `metadata` by default, or explicit `redacted` |
| `--event-redaction-file PATH` | for redacted | Absolute readable UTF-8 redaction configuration; invalid combinations/configuration fail with usage exit 2 before registration |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect |
| positional `PROMPT` | one of | Inline spawn prompt; exactly one of `PROMPT` / `--file` |
| `--file PATH` | one of | UTF-8 file whose contents are the spawn prompt (`-` = stdin) |
| `--json` | no | Output as JSON |

`--permission-mode` choices per backend are `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet)`. A value outside the backend's catalog exits 2 with `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`.

| Backend | Accepted `--effort` levels | Rejected with (exit 2) |
|---|---|---|
| `claude` | `low`, `medium`, `high`, `xhigh`, `max` | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| `codex` | `minimal`, `low`, `medium`, `high`, `xhigh` | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| `opencode` | none | `Error: opencode does not support reasoning effort.` |

An opencode `--model` outside the `<provider-id>/<model-id>` form exits 2 with `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`

The spawn prompt substitutes exactly four placeholders, which the member sees as literal lines:

| Placeholder | Substituted value | How the spawned member sees it |
|---|---|---|
| `{fleet_id}` | The member's fleet id | `FLEET ID: <fleet_id>` |
| `{member_id}` | The member's own newly-allocated id | `YOUR MEMBER ID: <member_id>` |
| `{director_member_id}` | The fleet's root Director id | `DIRECTOR MEMBER ID: <director_member_id>` |
| `{coding_agent}` | The resolved backend name | `CODING AGENT: <coding_agent>` |

Any other literal brace is doubled (`{{` / `}}`). An unknown placeholder or malformed brace expression exits 2 before registration. In the default output, `pane` renders `(pending)` until the pane id is patched onto the placement.

Creation probes the native executable's `--version` and selects the bundled runner profile with the greatest `min_version` at or below that version, using SemVer precedence (prerelease below its release, build metadata ignored). A version newer than every boundary reuses the newest profile with no warning. A known version below the backend's first boundary exits 1 before registration or pane creation with `unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION`. A failed native version probe, including a first line with zero or several SemVer tokens, adds the stderr line `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and creation continues with unknown coverage, whose ordinary input uses delivery fallback. Creation returns only after the runner reports that the native child started.

A failed creation compensates in order: shut down an owned runner and child, close the created pane, remove the pending descriptor, then mark the run `failed` and deregister the member, so the member ends deregistered with its placement removed. It keeps the primary cause and exit category and appends each cleanup diagnostic on its own stderr line: `cleanup failed for pane <id>: <detail>`, `owned child cleanup remains unconfirmed`, the cause of a failed descriptor removal, run update or deregistration, or `pane ID unknown; pane cleanup unconfirmed` when a failed split left no confirmed pane id. A member deregistered by compensation needs no further cleanup; an unconfirmed pane or child needs inspection.

### `member delete`

`cafleet member delete MEMBER_ID [--json]` shuts down a wrapped member's runner, locks its concrete pane, closes the pane and deregisters the member. Lock contention for five seconds exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. After a successful close, a database failure reports that the pane was closed and preserves the database cause. A failed or expired layout restoration emits a stderr diagnostic while preserving successful deregistration. The root Director fails with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead`.

### `member list`

`cafleet member list FLEET_ID [--json]` lists every active registry entry of the fleet — the root Director, ordinary members and placementless rows — as described in § Member list output. An empty roster prints `0 members.`.

### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active registry entry, root Director included, with no multiplexer requirement. Text is the compact one-line row; JSON is the detailed view including `kind`, placement and the appended runtime summary (§ Member runtime).

### `member prompt`

`cafleet member prompt MEMBER_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane` before any pane I/O. A wrapped member's runner delivers by profile, by delivery fallback or by queueing (§ Wrapped delivery methods); an explicitly unwrapped member receives Escape, settle, literal text, settle and Enter, which can interrupt work or dismiss a prompt. Exit 0 means submission or queue acceptance only, with ordered `{member_id, pane_id, text, input_id, delivery}`; unwrapped input has a null ID and method `unwrapped`. A queued prompt prints `Queued prompt for member ID (PANE).` instead of the sent line.

| Condition | Error body (exit 1) |
|---|---|
| Input lock still busy when the wait ends | `member ID's pane input remained busy; prompt was not dispatched.` |
| Pre-write error, including an Escape failure or a held runner reply | `prompt was not dispatched to member ID: CAUSE` |
| Partial uncertainty | `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

### `member exec`

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin is UTF-8 and strips at most one terminal LF or CRLF. Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs; invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

A wrapped member's runner submits literal `! ` plus the command by its native-exec plan, by delivery fallback or after queueing (§ Wrapped delivery methods); an explicitly unwrapped member receives Escape, settle, literal native shell input, settle and Enter. The backend owns working directory, environment, permissions, output and exit rendering. Successful exit 0 establishes complete submission/dispatch or queue acceptance only. Ordered JSON is:

```json
{"member_id":42,"pane_id":"w1:p2","coding_agent":"codex","command":"printf example","dispatched":true,"input_id":17,"delivery":{"status":"submitted","method":"profile","reason":null}}
```

`dispatched` is `false` for a queued command, whose text prints `Queued native command for member ID (PANE).`

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Pending placement | `member ID has no pane yet (pending placement) — nothing to exec.` |
| Input lock still busy when the wait ends | `member ID's pane input remained busy; native command was not dispatched.` |
| Escape or transport failure before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

The first text-writing call can partially type even if it fails. Failure after Enter may mean execution began. Inspect native pane output before any newly authorized retry. A wrapped operation durably records its operation key, payload digest and submission state; the receipt contains no command text. After native completion, inspect the pane and send an explicit prompt to use the output and continue when the host has not resumed the member.

### `member press`

`cafleet member press MEMBER_ID KEY... [--json]` requests only the specified ordered keys: one through 64 keys, each at most 32 UTF-8 bytes, from the key grammar in § Pane input and previews. It supplies no Escape prelude, Enter suffix, prompt text or shell-command submission. Use the ordinary `--` option separator before literal punctuation that could be read as an option, and put `--json` before that separator: `cafleet member press 42 -- '-' '+' ';'` requests three keys. Wrapped members receive the keys through their outer pane as manual native input. Success means backend acknowledgment of this ordered request; actual PTY bytes, native approval and completion require their own observation.

Exit 0 prints compact ordered JSON when `--json` is selected:

```json
{"member_id":42,"pane_id":"%5","keys":["Ctrl+a",";"],"status":"acknowledged"}
```

Otherwise it prints `Acknowledged keys KEYS_JSON for member ID (PANE).`, where `KEYS_JSON` is the compact JSON array of canonical keys.

| Condition | Error body (exit 2) |
|---|---|
| Key count outside the domain | `press requires between 1 and 64 keys` |
| A key exceeds its UTF-8 bound | `press key at position N exceeds 32 UTF-8 bytes` |
| A key is outside the common grammar | `invalid press key at position N: JSON_QUOTED_TOKEN` |

Operational failures exit 1 with empty stdout, including under `--json`. The primary error is followed by an effect diagnostic on its own stderr line:

| Observed boundary | Effect diagnostic |
|---|---|
| Before any writing process starts, including a failed metadata probe | `keys were not dispatched to member ID.` |
| A writing process starts and fails, times out or returns an unreadable acknowledgment | `key delivery to member ID is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.` |

| Local refusal | Primary error body |
|---|---|
| Root Director target | `cannot press keys in the Director's own pane` |
| Input lock remains busy | `member ID's pane input remained busy` |
| Revalidated target changes | `member ID placement changed before key dispatch` |
| Selected adapter differs from placement | `press backend does not match the recorded placement` |
| Selected live context differs from placement | `press target context does not match the recorded placement` |

A pending pane uses `member ID has no pane yet (pending placement) — nothing to press.` Every started failure, including a pane-gone response, retains possible partial delivery; perform no automatic retry. Request-bound permission decisions use `permission respond`, not `member press`.

### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` reads a pane tail. `--lines` is the number of trailing lines (default **20**); `--ansi` preserves ANSI escapes, and the default strips them and cleans carriage-return redraws. JSON stamps `captured_at` at the capture read and sets `content_sha256` to the lowercase SHA-256 hex of the exact emitted `content`. Text prints content without adding a newline. Read-only capture needs no input lock; a pending placement is an error.

## Pane input and previews

Every operation that writes pane input uses one process-shared OS file lock per pane, shared by duplicate placements of that pane. Read-only capture needs no input lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `message send` / `message wait` delivery | `message MESSAGE_ID was not notified to member MEMBER_ID: pane input remained busy; the next send to member MEMBER_ID or the next message wait retries it.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |
| `member press` | Exit 1 with the primary and zero-dispatch diagnostics (§ `member press`) |
| `member delete` / `fleet delete` | `member ID's pane input remained busy for 5 seconds; member was not deleted.` / `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.` |

Unwrapped prompt, exec and preview submission waits at most 1.5 seconds for the lock within a five-second submission budget. A wrapped member's runner owns the lock for its writes: the client holds no lock while awaiting the runner, and the runner retries a busy lock every 100 ms within its five-second attempt budget, holds the lock across an active staged write and releases it while a request is merely queued. The lock excludes CAFleet writers only; the user's keyboard and host UI can change at any time. Successful dispatch establishes complete submission only.

`member press` key grammar: single printable characters preserve case; named keys and modifiers match case-insensitively, and control letters normalize to lowercase.

| Common key | tmux argument | herdr argument |
|---|---|---|
| Single printable ASCII character, U+0021–U+007E | `0xXX` for its ASCII codepoint | Same character, with `+` as `plus`, `-` as `minus`, and `;` as `semicolon` |
| `Space` | `Space` | `space` |
| `Escape` | `Escape` | `esc` |
| `Enter` | `Enter` | `enter` |
| `Tab` | `Tab` | `tab` |
| `Backspace` | `BSpace` | `backspace` |
| `Up`, `Down`, `Left`, `Right` | Canonical capitalized name | Lowercase name |
| `F1` through `F12` | `F1` through `F12` | `f1` through `f12` |
| `Ctrl+a` through `Ctrl+z` | `C-a` through `C-z` | `ctrl+a` through `ctrl+z` |
| `Shift+Tab` | `BTab` | `shift+tab` |

Explicitly unwrapped prompt, exec and preview input selects a lead:

| Lead | Input sequence | Used by |
|---|---|---|
| Escape | Escape, 0.5-second settle, literal line, settle, Enter | `member prompt`, `member exec` and the first notification of a drain |
| Direct | Literal line, settle, Enter | Each further notification drained in the same lock hold |

Runner-managed members receive input through their runner's own PTY under § Wrapped delivery methods: profile delivery writes the literal, waits 500 ms and writes the measured submit key without Escape; delivery fallback uses the Escape lead above; a candidate or open interaction queues the input with zero bytes. Native shell input starts with `! ` followed by the validated command.

## Member runtime

### Runtime presentation

Member create preserves ordered `member_id,name,registered_at,placement` and appends `runtime`; member show preserves ordered `member_id,name,description,status,registered_at,kind,placement` and appends `runtime`. An explicitly unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id,resolved_mode,status,reachability,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity,effective_policy`, preserving stored nullability. `effective_policy` is the stored effective policy record or null. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, plus queued `ready` inputs, in input-ID order; an accepted input needs no reconciliation and stays listed until a settlement records its disposition, a queued input stays listed while the runner owns it, and `in_progress` and `uncertain` inputs need Director reconciliation. These are registry reads and make no live-health claim.

Wrapped create/show preserves its existing text row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE review=STATE`, where `STATE` is the effective policy's `automatic_review` value, `-` when absent. `event show` uses the same runtime object and line. An uncertain error retains its existing exit category and includes known receipt identity in the cause as `input_id=INPUT_ID` together with the selected method; run/interaction context accompanies held/rejected input.

### Delivery reporting

Send, prompt and exec JSON append `delivery` with ordered `status,method,reason` after `input_id`; text appends `Delivery: status=STATUS method=METHOD reason=REASON` (`-` for a missing method or reason) and then `Input: INPUT_ID` when an ID exists. `method` is `profile`, `fallback`, `unwrapped` or null before selection. The submitted-success lines print only for actual submissions. A fallback submission always prints `method=fallback` with reason `submission_plan_unavailable` or `idle_timeout`. A queued result names `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input`, states that the runner will retry, and points to `cafleet input show INPUT_ID --json` for the eventual result.

| Status | Meaning | CLI behavior |
|---|---|---|
| `submitted` | Complete literal and submit stages were written | Exit 0; send has `notification_sent=true`, exec has `dispatched=true` |
| `queued` | The runner accepted durable zero-byte deferred work | Exit 0; send has `notification_sent=false`, exec has `dispatched=false`; the input ID and reason are returned |
| `held` | No queue acceptance or write, due to unavailable health, ownership or storage | Send keeps its committed message and prints the reason; prompt/exec exits 1 with the existing delivery context |
| `uncertain` | Some stage may have written; reconciliation is required | Existing error categories, input identity and stage diagnostics, including the selected method |
| `not_submitted` | Cancelled with established zero-byte evidence | Visible through `input show` and, for deferred work, its result event |
| `not_applicable` | A self-send or root-Director target uses broker consumption only | Send keeps `notification_sent=false`, null method and input ID, and no warning |

A queued preview stays unnotified until actual submission or ACK settlement. A deferred input's later result never rewrites the original command output: the receipt and `input show` carry its actual method, and the Director receives one `input_delivery_result` event.

The effective policy record has keys `schema_version` (`1`), `automatic_review` (`available` or `unavailable`), `observed_mode` (text or null), `rule_id`, `event_id` (integer or null) and `observed_at`. A null record means no tested observation has been made.

### Runtime events

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON; the observed member is its sender and the Director its recipient. Verify the durable event/message association through `event show` before acting. Runtime event kinds are `coverage_unknown`, `interaction_opened`, `interaction_obscured`, `interaction_closed`, `permission_denied_observed`, `automatic_refusal_observed`, `automatic_review_unavailable`, `input_delivery_result`, `runner_degraded`, `runner_lost` and `runner_exited`.

Version-2 event keys are ordered `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for lifecycle events, then, for `input_delivery_result` only, ordered `input_id,message_id,delivery,stage`, then required `director_action`. The action object has ordered `kind,instruction,inspect_argv`; its `kind` is `review_permission_request`, `review_automatic_refusal` or `review_automatic_review_unavailable`, and other events use a null action. The evidence object has ordered keys `backend,native_version,profile_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`. `operation_rows` is the positive number of displayed rows the operation was read from, null exactly when `operation` is null; when it is greater than 1 the instruction asks the Director to confirm the command against the excerpt. Metadata evidence leaves every screen-derived operation, choice, excerpt and native identifier null. A UTF-8 excerpt is limited to 8 KiB and the payload to 16 KiB.

| Field | `automatic_refusal_observed` | `automatic_review_unavailable` |
|---|---|---|
| `interaction_id`, `classification` | `null` | `null` |
| `source` | `runner_screen` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` | `null`, `"unknown"` |
| `evidence` | `rule_id` names the refusal rule; `operation`, `operation_rows` and `excerpt` follow the run's evidence policy | `rule_id` names the notice rule; `operation` and `operation_rows` are null |
| `director_action.kind` | `review_automatic_refusal` | `review_automatic_review_unavailable` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]` | `["cafleet","event","show","<EVENT_ID>","--json"]` |

Neither automatic review event creates an interaction or changes input eligibility, so `permission inspect` and `permission respond` naming one fail their event/interaction association check before any byte.

`input_delivery_result` reports a deferred input's first submitted, not-submitted or uncertain result. It has null `interaction_id`, `classification` and `director_action`, `source` `runner_lifecycle`, `decision_outcome` `"unknown"`, and a `cause` only when uncertain. `input_id` names the input, `message_id` is the preview's message or null for prompt/exec, `delivery` is the stored projection, and `stage` is `zero_bytes`, `partial`, `submitted` or `unknown`. It carries no literal payload and is published once per input.

### `event show`

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. The event is the complete versioned runtime payload, run is the current runtime summary, and nullable interaction contains the stored interaction fields in order `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. Unavailable optional text values render `-`.

### `input show` and `input reconcile`

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response,delivery,deferred_at,defer_reason,payload_digest_version`. Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`, with `-` for absent optional fields.

Input kinds are `preview`, `prompt`, `exec` and `permission_response`; states are `ready`, `in_progress`, `accepted`, `uncertain` and `settled`. `ready` certifies a zero-byte attempt, and a settled disposition is `submitted`, `not_submitted` or `abandoned_uncertain`. A preview receipt uses the operation key `preview:<run_id>:<message_id>`. The receipt stores a payload digest and never the literal prompt or command. New preview receipts use `payload_digest_version` `2`, which binds the method-independent canonical preview so a later change between profile and fallback delivery reuses the same input ID; other receipts use `1`.

`delivery` projects the stored receipt: `ready` with `deferred_at`/`defer_reason` is `queued` with the original defer reason; `ready` without them is `held` with `zero_bytes_ready`; `in_progress`, `uncertain` and `settled/abandoned_uncertain` are `uncertain`; `accepted` and `settled/submitted` are `submitted`; `settled/not_submitted` is `not_submitted` with its zero-byte outcome reason. A receipt whose method was never recorded keeps a null method, with reason `method_unrecorded` outside `ready`. A permission response has `delivery: null`, null deferred fields and digest version `1`.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores. Require a nonempty evidence reference; the command synchronizes with the current owner before settlement and preserves uncertainty while an active writer cannot be ruled out. A settled ordinary receipt with the same disposition returns its original row and evidence reference; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One five-second budget covers the pane wait, current-owner synchronization, database work and socket exchange. If the writer is unreachable, settlement requires exclusive receipt-only ownership; unavailable proof retains uncertainty.

`input show` and `input reconcile` append nullable `permission_response` after `evidence_ref`. For a response kind it contains ordered `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at`; other input kinds return null. Text appends `Permission decision: interaction=INTERACTION_ID event=EVENT_ID decision=DECISION source=SOURCE mode=MODE director=DIRECTOR_ID epoch=EPOCH` for a response audit. Settlement neither grants a native answer nor releases a possibly transmitted episode claim.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly.

### `permission inspect`

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. An unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check. One five-second deadline covers the command. Inspection sends zero child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,profile_version,answer_profile_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence
```

`visibility` is `visible`, `obscured`, `not_visible` or `unknown`. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable and otherwise a bounded explanation. `supported_decisions` contains the currently tested subset of `allow,deny`, in that order; a blocked or unsupported request returns an empty array and null token. `request_token` is a nullable random lowercase 64-hex value bound to the request. `missing_evidence` lists unavailable public facts in order from `operation,arguments,cwd,environment,targets,effects,choice_scope`; operation is available only when its post-policy value is nonblank, unredacted and untruncated. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding.

Inspection text is:

```text
Permission: interaction=INTERACTION_ID event=EVENT_ID member=MEMBER_ID run=RUN_ID epoch=EPOCH visibility=VISIBILITY answers=DECISIONS
Request: token=TOKEN output_seq=SEQUENCE profile=ANSWER_PROFILE rule=ANSWER_RULE
```

Supported decisions are joined with commas, and unavailable values or an empty decision set render as `-`. `Capability: REASON` is appended when nonnull and `Missing evidence: FIELDS` when the array is nonempty.

### `permission respond`

`respond` requires the complete request snapshot and decision provenance:

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]
```

Require positive IDs/epoch, the token's exact hex form and a nonblank operation key. Evidence references contain 1–2048 UTF-8 bytes, include a non-whitespace character and contain no C0 or DEL control character; they identify host evidence without embedding commands, screen text or secrets. Source `director-auto` requires mode `auto`; source `user` records the user's decision in any mode. Enum/shape/source-mode errors exit 2 before lock or byte effects. Resolve Director mode from current host evidence, independently of member launch policy, and obtain user permission in other/unknown modes. Native and managed rules still authorize running the command. Record the operation key before dispatch.

A retained operation key is validated before any current-request check: a matching retained receipt returns its stored result, sending zero bytes, and conflicting reuse fails. Another key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim.

The response JSON is `{response}`, with ordered keys:

```text
status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,decision_source,stage,output_seq,reason
```

`status` is `accepted`, `held`, `rejected` or `uncertain`; `input_id` is nullable until a durable row exists. `stage` is `zero_bytes`, `partial`, `submitted` or `unknown`; accepted has submitted stage, held/rejected has zero-byte stage, and uncertainty reports the observed partial/submitted stage or unknown. `output_seq` is a nullable unsigned integer and `reason` a nullable bounded explanation.

| Durable receipt | Retry status and stage |
|---|---|
| `accepted`, or `settled/submitted` | `accepted/submitted` |
| `in_progress` or `uncertain` | `uncertain/unknown`; retain original transmission uncertainty and claim |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected/zero_bytes` with a consumed-key reason; preserve the original disposition and claim |
| `ready` | The separate exact-metadata live-validation path |

For a consumed key, `zero_bytes` describes this retry only and supplies no zero-byte evidence about the original attempt.

Response text is:

```text
Permission response: interaction=INTERACTION_ID run=RUN_ID decision=DECISION source=SOURCE status=STATUS input=INPUT_ID stage=STAGE output_seq=SEQUENCE
```

Null values render as `-`, and `Reason: REASON` is appended when nonnull. Accepted means answer-key submission, independently of native approval and execution.

Exit 0 means accepted submission or its matching prior submitted receipt. A structured held/rejected/uncertain reply prints the response in the selected text/JSON form and exits 1, with stderr `Error: permission response STATUS for interaction INTERACTION_ID: REASON`; ` (input_id=INPUT_ID)` is appended when known. Pre-reply operational failures retain their ordinary error category and include known run/interaction/input context; usage failures exit 2.

A new attempt against a blocked current request fails before any receipt, audit, claim or byte with the primary stderr line `Error: runner control: permission request is blocked: REASON`, followed by the known run/interaction/operation-key context. `REASON` is the same explanation `permission inspect` returns as `capability_reason`:

| Reason | Blocking state |
|---|---|
| `permission input is fenced` | Programmatic input is fenced for shutdown, ownership loss or recovery |
| `manual input accounting failed` | The runner could not account for actual manual input |
| `observation degraded: CODE` | The observer recorded a gap or degradation with diagnostic `CODE` |
| `observation transitions pending` | Observed transitions await persistence |
| `observation evidence pending` | An interaction evidence refresh awaits persistence |
| `observation update pending` | A retained transition, draft or evidence update has not completed |
| `observation incomplete` | Child output was read but its observation has not completed |
| `observation health unavailable` | The observer reported no input health |
| `terminal input interrupted` | A terminal read was interrupted during the drain |
| `outer terminal closed` | The outer terminal reached end of input |
| `native output closed` | The native child's output reached end of file |
| `terminal output still arriving` | The drain's bounded rounds ended before human and child input were quiet |
| `permission request is not visible` | The request is obscured, closed or not observed as visible |
| `permission request is not the current interaction` | Input eligibility is not held for the current permission interaction |

A retained operation key is resolved before the admission drain, so its stored result returns without typing even when the current request is blocked.

### `member recover`

`cafleet member recover MEMBER_ID [--json]` is Director-owned and reconciles a recorded run of an active or deregistered member without starting an agent. A member without a run fails explicitly before mutation. One five-second deadline bounds each recovery pass.

The exceptional form is `cafleet member recover MEMBER_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. It requires all four options together, positive snapshot IDs/epoch taken from the inspected runtime record and a nonempty reference to the operator's recorded attestation that the owner and startup processes have terminated; incomplete or invalid input exits 2. A changed owner rejects the attestation, and positive live evidence preserves the live owner.

Ordered JSON keys are `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`. Counters measure work newly committed by this invocation. Text is `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC` in array order. Incomplete or unreachable outcomes remain distinct from confirmed recovery.

## `setup` and `doctor`

`cafleet setup [--coding-agent AGENT...]` prepares the database, then installs the embedded skills and presets offline for the selected backends (default: every backend), in registry order. After migrating it prints `Applied migrations to head ({head}) at {path}.`; a database already at head prints `Already at head ({head}).`. It then prints one success line per operation:

```
<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```

Installation stops at the first backend whose config-dir resolution or installation fails, and setup exits 1 with that cause: `failed to install skills into <skills_dir>: <error>`, `failed to install preset into <target>: <error>`, or the config-path validation error.

`cafleet doctor [--json]` reports every check without aborting. Each check has a `name`, a `status` of `ok`, `info` or `error`, and a one-line `detail`. Check names are `multiplexer`, `database`, `<agent>.configuration`, `<agent>.executable`, `opencode.preset`, `opencode.preset.cafleet-ask`, `<agent>.installation` and `member_runtime.<run_id>`. A recorded asset version other than the CLI version is `info` with `; run 'cafleet setup --coding-agent <agent>'`. Text mode prints one `{status} {name}: {detail}` line per check; `--json` prints `{"checks":[{"name":…,"status":…,"detail":…}]}`. When any check has status `error`, doctor exits 1 with `Error: doctor found errors` after printing every check.

## Error messages

Application errors print as stderr text even with `--json`, each prefixed `Error: `. Usage/parser/model/effort/line-validation errors exit 2; application, database and transport errors exit 1.

| Command | Situation | Error message | Exit |
|---|---|---|---|
| (any fleet/member/message command) | No database | `no cafleet database; run 'cafleet setup'` | 1 |
| (any fleet/member/message command) | Database behind the CLI | `database schema is outdated (schema M, head N); run 'cafleet setup'` | 1 |
| (any fleet/member/message command) | Database ahead of the CLI | `database schema M is newer than this cafleet (head N); upgrade cafleet` | 1 |
| (any fleet/member/message command) | Incompatible database file | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` | 1 |
| `member create` | The backend binary is missing | `Error: binary <name> not found on PATH` | 1 |
| `member create` | The selected opencode preset is missing or unreadable | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` | 1 |
| `member create` | Unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` | 2 |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 |
| `member create` | The fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 |
| `member create` | The fleet's root Director has no concrete placement | `Error: fleet <fleet-id>'s root Director (member <id>) has no concrete placement.` | 1 |
| `member create` | `--coding-agent` omitted and the spawning Director not found | `Error: Director (member <director-id>) not found.` | 1 |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: Director (member <director-id>) has no placement; supply --coding-agent.` | 1 |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` | 1 |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 |
| `member capture` / `member prompt` / `member exec` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&gt;.</code> | 1 |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 |
| `member prompt` / `exec` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 |
| `message send` / `member create` | `--file <path>` to an empty file | `Error: --file <path>: file is empty.` | 1 |
| `message send` / `member create` | `--file -` with empty stdin | `Error: --file -: stdin is empty.` | 1 |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 |
| `message send` / `member create` | `--file <path>` to invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` | 1 |
