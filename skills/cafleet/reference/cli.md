# CAFleet CLI reference

Exact command facts for running CAFleet and interpreting its results.

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

Place the complete DB-assigned integer subject immediately after its subcommand. Related fleet/recipient state comes from that row; missing or malformed IDs use native parser errors, exit 2. Members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` spawn lines. `message send` requires active integer `--from-member-id` and `--to-member-id` in the same live fleet; the sender determines the fleet.

## Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `CAFLEET_DATABASE_URL` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v9.db` | SQLite path |
| `CAFLEET_MULTIPLEXER` | Auto-detect | `tmux` or `herdr` override |
| `CAFLEET_MAX_TEXT_LEN` | `200` | Text-body codepoint limit |

Explicit `sqlite:///<path>` resolves relative paths against cwd and leaves literal tildes unexpanded. Counts require nonnegative integers: `<NAME> must be a non-negative integer (got '<value>')`.

Config-location overrides `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR` and `HERMES_HOME` (default `~/.hermes`) require absolute paths: `Error: <VAR> must be an absolute path (got '<value>')`, exit 1. Creation forwards the selected backend's set override and `CAFLEET_DATABASE_URL`.

## Output shapes

Use JSON for complete untruncated text. Message records contain `message_id`, `from_member_id`, integer `to_member_id`, `created_at`, `status_state`, `status_timestamp` and `text`. ACK changes `input_required` to `completed` and stamps consumption independently of assignment progress.

Fleet creation returns `fleet_id`, `name`, `created_at` and `director` identity/description/registration/placement; lists return records and show includes nullable `deleted_at`. Deletion returns `fleet_id`, `deregistered_count` and `members`. Send returns `message`, `notification_sent`, `input_id`, `delivery`; poll returns an array; ACK/show return `message`. Member deletion returns `member_id`, `pane_status`; capture returns `member_id`, `pane_id`, `lines`, `content`, `captured_at`, `content_sha256`. Runtime commands define their fields below.

### Member list output

Roster records contain `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack`, `idle`. Fleet-reference matches have kind `director`; others `member`. Missing activity/placement may be null and pane placement pending.

## JSON output

Place `--json` before any `--` literal-key separator. JSON is compact UTF-8; application errors remain stderr text except structured permission responses. Usage errors exit 2. Commands marked no in the summary, including the root group, reject `--json`.

## `permissions.allow` coverage

Broker prefix rules such as `Bash(cafleet message *)` cover finite wait/listen, including `cafleet message listen ID --timeout 300`; authorize Monitor separately. Literal IDs preserve matching. Keep `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` under ask; ask outranks allow. Director permission responses remain subject to effective native/managed rules.

Claude receives `Bash(cafleet message *)` at spawn. Codex allows CAFleet commands but prompts for prompt/exec/press, permission respond, input reconcile and member recover; OpenCode presets allow `cafleet *` and deny those Director-owned commands. Members retain broker reception/evidence-routing authority.

## Text bodies

Send/create require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve nonblank text exactly. Missing/both sources or blank positional text exit 2; blank/unreadable/invalid file or stdin exits 1 with § Error messages. Prompt accepts one positional line.

## Fleet commands

`cafleet fleet create --name NAME --coding-agent AGENT [--json]` registers the current tmux/herdr pane as root Director without spawning. Outside a session: `cafleet fleet create must be run inside a tmux or herdr session`. Director messages use broker reception.

`cafleet fleet list [--json]` reads the registry. `cafleet fleet show FLEET_ID [--json]` includes deleted fleets; unknown IDs give `fleet 'X' not found.`, exit 1.

`cafleet fleet delete FLEET_ID [--json]` deletes active ordinary concrete panes through member deletion, then deregisters remaining members/fleet, retaining messages and the Director pane. It is idempotent; partial cleanup resumes on rerun.

| Outcome | Result |
|---|---|
| Lock contention until the deadline | Exit 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`; every pane and registry row is unchanged |
| A member cannot be ended | Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered, and a rerun resumes |
| Placement set changed after cleanup began | Exit 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` |

## Messages

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Persist once, then deliver ordinary-recipient previews oldest-first. Self-sends return `delivery.status=not_applicable`, null method/input and reason `self_send`; other root-Director sends use reason `director_stdout`, without multiplexer or pane diagnostics.

Commit succeeds with exit 0 even when notification is queued, held or unavailable. `notification_sent=true` means this invocation submitted the new message; another invocation's submission leaves it false without a diagnostic. `input_id` identifies the new message's nullable preview receipt; `delivery` describes that message even when older ones are retried. Pending previews retry on the next send/wait; runner-queued previews retry automatically.

### Delivery outcome

Failure preserves successful stdout and prints `Message NEW_ID was persisted. ` followed by:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

A busy lock reports `message MESSAGE_ID was not notified to member MEMBER_ID: pane input remained busy; the next send to member MEMBER_ID or the next message wait retries it.` Other causes are runner lock/transport/submission or storage errors, with known run/input context. Storage failures identify the current candidate or selected wait-rotation entry. An exhausted attempt retains its sent prefix and reports failure with current receipt context.

Receipt retirement failure reports `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE`, retaining `notification_sent=true`. Pre-commit storage failure exits 1. Consume each committed ID once.

For an ordinary recipient without a recorded run, `CAUSE` is `member MEMBER_ID has no recorded run`. The committed message stays pending, failed delivery has null method, stdout retains its successful record and exit 0, and later wait attempts report the same cause.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns incoming `input_required` records descending by status timestamp/message ID. Unknown/inactive member: `Member ID not found`, exit 1.

### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is integer `1..=1800`, default `300`. Pending messages return immediately; otherwise poll each second, retrying at most one other ordinary recipient per interval and each pending message once per invocation. It never ACKs.

Result fields are `member_id`, `outcome`, `messages`, `notification_errors`; outcome is `messages` with full records or `timeout` with an empty array. Diagnostics contain `member_id`, `message_id`, `error`, ordered by message ID. Both outcomes exit 0; deleted subject/fleet or corrupt/unreadable state exits 1. Deadline allows one in-flight attempt/finalization of at most six seconds.

### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS]`

Streams an active member's pending inbox, requiring a live root Director with concrete placement and no multiplexer. Unknown/inactive startup: `Member ID not found`, exit 1. Optional integer `1..=1800` lease is monotonic from startup; absence runs until closure/cancellation/failure.

Flush UTF-8 JSONL records: `ready` with `member_id,fleet_id` before replay; `message` containing the complete message record; `closed` with `member_id,reason`. There is no text mode or `--json`. Idle checks emit nothing. Closure reasons are `timeout`, `member_inactive`, `fleet_deleted`; deletion wins simultaneous lifecycle closure. Normal closure exits 0, operational/output failures exit 1, usage errors exit 2; cancellation may omit `closed`.

Order records by creation/message ID; suppress already-emitted IDs per process. Listening changes no consumption/notification state and retries no pane delivery. Restart replays pending IDs; concurrent listeners are independent. Reconcile stale/replayed envelopes through poll/show JSON, record deferred actions and ACK consumption once. Preserve incomplete JSONL across native reads.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` stamps an unread delivery completed; ACK means consumption.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns its full record.

## Push notifications

Ordinary concrete panes receive previews; Directors and self-sends use broker reception. Profile previews are:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Delivery fallback leads with Escape and uses:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text stays exact. Previews fold newlines to `⏎`, tabs to `␉`, other C0/DEL controls to `\xNN`. At most 4000 Unicode scalars appear inline; longer text becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.` independently of `CAFLEET_MAX_TEXT_LEN`. Delivery method selects the clause before writing. Preview leaves consumption `input_required`.

### Wrapped delivery methods

Validate identity, ownership, reachability, storage and receipt history first; failure or uncertain prior input sends no fallback bytes. Unknown detection alone permits fallback.

An open/candidate interaction (`permission_request`, `question`, `trust`, `update`, `unknown_interaction`, including obscured) or earlier queued input defers with zero bytes, including Escape. A verified empty composer with a measured plan receives literal input, 500 ms settle and the measured submit key without Escape. Terminal replies leave submission valid; human input stops it with `uncertain_cause=manual_input`.

A measured plan with an unverified/nonempty composer queues under `composer_busy`, then drains when empty; three minutes waiting solely on that condition permits fallback `idle_timeout`. Interaction/earlier-input time is excluded. Working members with empty composers receive profile input queued natively behind their turn. Explicit interruption uses `cafleet member press MEMBER_ID Escape`.

Absent plan/profile/version probe uses fallback `submission_plan_unavailable`: Escape, settle, literal (`! ` prefix for exec), settle, Enter. Recheck interactions before each stage; permission answers require their tested plan.

### Runner queue

Accepted deferred input returns durable `ready`, `deferred_at`, `defer_reason` and its input ID; it outlives the invocation and drains automatically. Inspect `cafleet input show INPUT_ID --json`. The Director receives one `input_delivery_result` for its first submitted/not-submitted/uncertain outcome.

Zero-byte reasons are `pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted`, `message_consumed`, `target_inactive`, `target_changed`, `runner_stopped` or `runner_lost`.

### Held delivery

Degraded/recovering/lost observation, unavailable ownership/storage or uncertain receipts/reachability hold input without queue acceptance or writing. Pending/changed placements also hold. Held/queued previews leave notification unset and grant no permission. Pending sends retry on subsequent send/wait.

### Accepted trade-offs

Delivery fallback can interrupt work, dismiss undetected prompts or combine unsent drafts. A host may need multiple Escapes; submission remains valid when the host queues instead of interrupting. Resume and dismissal clauses are conditional.

Claude may report `The user doesn't want to proceed with this tool use.`; only an Escape-dismissal preview establishes delivery-caused rejection. Prompt/exec carry no clause, so inspect open prompts first. Repeated previews are consumed by ID; concurrent sends may interrupt earlier turns, and interruption can end shell tools. The CLI defers termination during pane input. OpenCode captures may show only a status bar; inspect retained sessions for omitted output and broker poll/ACK for consumption.

## Member commands

Pane operations require active placement; opaque pane IDs may be pending. Prompt/exec/press reject the Director before I/O; individual deletion protects it.

### `member create`

`cafleet member create --fleet-id ID --name NAME --description DESCRIPTION [--coding-agent AGENT] [--model MODEL] [--permission-mode MODE] [--event-evidence POLICY] [--event-redaction-file PATH] [--effort LEVEL] (PROMPT | --file PATH) [--json]`

Registers and spawns, resolving the acting Director from the fleet. Omitted backend inherits its placement backend. Description is one sentence; only Claude displays the name as pane title. Model forwards to the native CLI; OpenCode requires `<provider-id>/<model-id>`, and Hermes passes any value through. Hermes argv: `hermes chat --cli --source tool --in <cwd>`, then optional `--model MODEL` and `--reasoning LEVEL`, then `-q PROMPT`; it never carries `--yolo`, `--provider`, `--resume`, `--continue` or `--create-if-missing`.

Evidence defaults to `metadata`; `redacted` requires an absolute readable UTF-8 redaction file. Invalid policy/configuration combinations exit 2 before registration. `--file -` reads stdin.

Permission choices: `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet); hermes: smart (default smart)`. Invalid selection: `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, exit 2. Mode classes (documentation only; argv and run records are unchanged): `auto` (native automatic review or a fixed allow/deny policy) is claude `auto`, `dontAsk`, `bypassPermissions`; codex `approve-for-me`, `never`; opencode `cafleet`; hermes `smart`, which emits no permission argv (`permission_argv_json` `[]`) because the effective Hermes `approvals.mode` is the policy. `manual` (native interactive approval) is claude `default`, `manual`, `acceptEdits`, `plan`; codex `on-request`; opencode `cafleet-ask`. Every default is `auto`.

Effort validation precedes side effects:

| Backend | Levels | Error (exit 2) |
|---|---|---|
| claude | low, medium, high, xhigh, max | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| codex | minimal, low, medium, high, xhigh | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| opencode | — | `Error: opencode does not support reasoning effort.` |
| hermes | none, minimal, low, medium, high, xhigh, max, ultra (forwarded as `--reasoning`) | `Error: --effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got '<value>').` |

Invalid OpenCode model: `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`, exit 2.

Hermes availability preflight, after the config-dir and PATH checks: one bounded `hermes config get --json approvals` under the member's `HERMES_HOME`. Each failure exits 1 before registration:

| Condition | Error |
|---|---|
| Command fails, times out or prints no JSON object | `Error: cannot read hermes configuration through 'hermes config get': <cause>` |
| `mode` is not `smart` | `Error: hermes approvals.mode is '<value>'; CAFleet members require 'smart'` |
| `deny` lacks a Director-only glob | `Error: hermes approvals.deny lacks CAFleet's Director-only rules: <glob>, …; run 'cafleet setup --coding-agent hermes'` |

The member environment forwards a set `HERMES_HOME` and removes `HERMES_YOLO_MODE`.

Substitute exactly `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}` into literal identity lines. Double literal braces `{{`/`}}`; malformed/unknown placeholders exit 2 before registration (§ Error messages).

Probe native `--version`; select the greatest bundled `min_version` not exceeding it by SemVer precedence (ignore build metadata). Newer versions reuse the newest profile. Below-minimum versions fail before registration/pane creation: `unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION`, exit 1. Hermes takes its version from a first line matching `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$` (`Hermes Agent vunknown (2026.9.24)` records `2026.9.24`). Failed probes, including a first line with zero or several SemVer tokens or a non-matching Hermes line, warn `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and continue with unknown coverage/fallback. Creation waits for native-child startup.

Failure compensation shuts down owned runner/child, closes pane, removes descriptor, marks run `failed` and deregisters/removes placement, retaining the primary exit category. Cleanup diagnostics include `cleanup failed for pane <id>: <detail>`, `owned child cleanup remains unconfirmed`, descriptor/run/deregistration errors and `pane ID unknown; pane cleanup unconfirmed`. Inspect unconfirmed resources.

### `member delete`

`cafleet member delete MEMBER_ID [--json]` shuts down a recorded runner, closes the locked pane and deregisters. Run-less teardown skips runner shutdown while retaining pane close/deregistration. Busy: `member ID's pane input remained busy for 5 seconds; member was not deleted.`, exit 1 unchanged. Post-close database errors retain the cause and closed-pane evidence. Layout-restoration diagnostics preserve successful deregistration. Director protection uses § Error messages. A `hermes` run also stops its Hermes session work and registered background processes after runner shutdown; an unverified stop exits 1 with `Error: hermes work for member ID could not be verified as stopped: CAUSE; member was not deleted.` and keeps the member, and a retry re-runs the settlement.

### `member list`

`cafleet member list FLEET_ID [--json]` lists active Director, ordinary and placementless records (§ Member list output).

### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active member, including Director, without a multiplexer; includes `kind`, placement and runtime.

### `member prompt`

`cafleet member prompt MEMBER_ID TEXT [--json]` requires a nonblank control-free line: `text must be one non-empty line without control characters.`, exit 2.

Director target: `cannot prompt the Director's own pane`, exit 1. An ordinary member requires a recorded run; absence fails before pane I/O with `member <ID> has no recorded run`, exit 1. Runner delivery uses profile/fallback/queue, and exit 0 establishes submission/queue acceptance. Result fields: `member_id,pane_id,text,input_id,delivery`.

| Condition | Error body (exit 1) |
|---|---|
| Pre-write error, including a held runner reply | `prompt was not dispatched to member ID: CAUSE` |
| Partial uncertainty | `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

### `member exec`

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads UTF-8 stdin, stripping at most one final LF/CRLF. Exactly one source; a nonblank line excludes CR/LF, NUL, tabs and other C0/DEL: `command must be one non-empty line without control characters.`, exit 2 before I/O. Multiline tasks use one-line script invocations.

An ordinary member requires a recorded run; absence fails before pane I/O with `member <ID> has no recorded run`, exit 1. The runner submits `! ` plus command through profile/fallback/queue. The host owns cwd, environment, permissions, output and exit. Exit 0 establishes submission/queue acceptance; JSON contains `member_id,pane_id,coding_agent,command,dispatched,input_id,delivery`, with `dispatched=false` when queued.

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Pending placement | `member ID has no pane yet (pending placement) — nothing to exec.` |
| Runner refusal before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

A failing first write may partially type; failure after Enter may have executed. Inspect native output and reconcile receipts before a newly authorized retry. Receipts retain operation key/digest/state, without command text. After completion, explicitly prompt continuation if the host has not resumed.

### `member press`

`cafleet member press MEMBER_ID KEY... [--json]` requests 1–64 ordered keys, each at most 32 UTF-8 bytes, with no implicit Escape/Enter or shell submission. For punctuation use `cafleet member press 42 -- '-' '+' ';'`; place `--json` before `--`. Wrapped members receive manual keys through the outer pane. Exit 0 establishes backend acknowledgment, independently of native approval/effects; fields are `member_id,pane_id,keys,status`, with `status=acknowledged`.

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

Pending pane: `member ID has no pane yet (pending placement) — nothing to press.` Operational failures exit 1 with empty stdout, including JSON. Started failures retain possible partial effects and require explicit review before retry. Request-bound decisions use permission respond.

### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` reads trailing lines (default 20); `--ansi` preserves escapes, otherwise clean redraws/escapes. Capture requires no input lock; pending placement fails. `captured_at` stamps reading; `content_sha256` hashes exact emitted `content`.

## Pane input and previews

Writers share one OS lock per concrete pane, including duplicate placements; capture is read-only. Human input remains independent. Busy outcomes retain the exact command diagnostics above.

Keys accept printable ASCII U+0021–U+007E, `Space`, `Escape`, `Enter`, `Tab`, `Backspace`, arrows, `F1`–`F12`, `Ctrl+a`–`Ctrl+z`, `Shift+Tab`. Printable case is preserved; names/modifiers are case-insensitive and control letters canonicalize lowercase.

Runner delivery follows [wrapped delivery methods](#wrapped-delivery-methods). Native commands start `! `.

## Member runtime

### Runtime presentation

Create/show include `member_id,name,registered_at,placement,runtime`; show also includes `description,status,kind`. Every ordinary member has a recorded run; only the root Director has `runtime:null`. Runtime fields describe identity (`run_id,resolved_mode,owner_epoch`), state (`status,reachability,last_health_at,coverage,current_interaction_id`), ownership (`owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`), `unresolved_input_ids` and nullable `effective_policy`.

Unresolved IDs include queued `ready`, `in_progress`, `accepted`, `uncertain`, ascending; accepted needs settlement, in-progress/uncertain needs Director reconciliation. Registry reads establish no live health. Effective policy contains `schema_version=1`, `automatic_review=available|unavailable`, nullable `observed_mode`, `rule_id`, nullable integer `event_id`, `observed_at`; null means no tested observation.

Uncertain errors preserve exit category and known receipt/context, including `exec delivery to member 110 is uncertain: manual_input (method=profile)` and `input_id=INPUT_ID`.

### Delivery reporting

`delivery` contains `status,method,reason`. Method is `profile`, `fallback` or null before selection. Submission/queue exit 0; queued send has `notification_sent=false`, queued exec `dispatched=false`; submitted send/exec set these flags true. Held prompt/exec exits 1; held send retains commit. `uncertain` requires reconciliation; `not_submitted` certifies zero-byte cancellation; `not_applicable` is broker-only.

Queue reasons are `interaction_open,interaction_candidate,composer_busy,earlier_input`; fallback reasons `submission_plan_unavailable,idle_timeout`. Held send reasons: `observer_unavailable` (no observation), `notification_held` (placement/run/receipt hold), `notification_failed` (lock/transport/storage/submission failure). Queued previews stay unnotified until submission or ACK settlement; later results appear in receipts/events without changing original command output.

### Runtime events

Notifications contain full versioned JSON, observed member as sender and Director as recipient. Verify association through event show. Kinds: `coverage_unknown,interaction_opened,interaction_obscured,interaction_closed,permission_denied_observed,automatic_refusal_observed,automatic_review_unavailable,input_delivery_result,runner_degraded,runner_lost,runner_exited`.

Runtime events use `schema_version=2`; readers require it. Identity fields: `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id`; context includes `kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, closure `close_reason` or lifecycle `cause`. Required nullable `director_action` contains `kind,instruction,inspect_argv`: review kinds are `review_permission_request,review_automatic_refusal,review_automatic_review_unavailable`.

Evidence contains `backend,native_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`. Positive `operation_rows` is null exactly with null operation; above one requires confirmation against excerpt. Metadata nulls screen-derived operation/choices/excerpt/native identifier. Excerpts/payloads are bounded at 8/16 KiB.

| Field | `automatic_refusal_observed` | `automatic_review_unavailable` |
|---|---|---|
| `interaction_id`, `classification` | `null` | `null` |
| `source` | `runner_screen` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` | `null`, `"unknown"` |
| `evidence` | `rule_id` names the refusal rule; `operation`, `operation_rows` and `excerpt` follow the run's evidence policy | `rule_id` names the notice rule; `operation` and `operation_rows` are null |
| `director_action.kind` | `review_automatic_refusal` | `review_automatic_review_unavailable` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]` | `["cafleet","event","show","<EVENT_ID>","--json"]` |

Automatic-review events create no interaction/input eligibility; permission commands reject their association before writing.

Delivery results contain `input_id,message_id,delivery,stage`, null interaction/classification/action, source `runner_lifecycle`, outcome `unknown` and receipt-derived `cause={"kind":UNCERTAIN_CAUSE}` only when recorded. Stages are `zero_bytes,partial,submitted,unknown`. First outcome publishes once, without literal payload.

### `event show`

`cafleet event show EVENT_ID [--json]` returns `event,notification_state,message_id,archive_reason,run,interaction`. Event is full versioned payload, run current summary. Nullable interaction contains `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

### `input show` and `input reconcile`

`cafleet input show INPUT_ID [--json]`; `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]`. Result `input` contains `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response,delivery,deferred_at,defer_reason,uncertain_cause`.

Kinds: `preview,prompt,exec,permission_response`; states: `ready,in_progress,accepted,uncertain,settled`. Ready certifies zero bytes. Stored dispositions: `submitted,not_submitted,abandoned_uncertain`; CLI: `submitted,not-submitted,abandoned-uncertain`. Preview keys: `preview:<run_id>:<message_id>`. Receipts hold digests, excluding literals. Ordinary-input SHA-256 covers canonical JSON `run_id,kind,message_id,literal`; previews use canonical wrapped lines ending at the resume clause, prompt/exec submitted literals. Stable receipt IDs.

Projection: deferred ready → queued with original reason; other ready → held/`zero_bytes_ready`; in-progress, uncertain or abandoned-uncertain → uncertain; accepted/submitted → submitted; not-submitted → not_submitted with cancellation reason/null method or reconciled selection. Unrecorded methods stay null with `method_unrecorded` outside ready. Permission responses have null delivery/deferred fields and dedicated validated request/decision/audit metadata digests.

`uncertain_cause` records transition to uncertainty and survives reconciliation; null means never uncertain, pre-V8 input or runner-lost in-progress input.

| Cause | Recorded when |
|---|---|
| `manual_accounting_failed` | Manual-input accounting failed when a staged stage stopped |
| `manual_input` | Human input (terminal replies excluded) arrived after the literal stage, stopping the submit stage |
| `interaction_candidate` | A staged stage stopped on a candidate interaction |
| `interaction_open` | A staged stage stopped on any other blocking interaction |
| `observer_unavailable` | A staged stage stopped because the observer was unavailable or reported a gap |
| `short_write` | A programmatic write accepted fewer bytes than its stage |
| `write_failed` | The attempt failed after a possible child byte, including a failed deferred attempt or permission response |
| `runner_stopped` | Runner shutdown ended an active deferred attempt that may have written a child byte |

At settle checks the first matching cause among the first five wins. Director reconciliation requires nonempty evidence, synchronization with current owner and proof no active writer remains; unreachable owners require exclusive receipt-only ownership, otherwise uncertainty remains. Repeating a settled ordinary disposition returns original evidence; conflicting disposition fails unchanged. Settlement writes no input and establishes no native result.

Nullable `permission_response` audit contains `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id,decided_at`. Settlement grants no answer and releases no possibly transmitted claim. IDs are positive; malformed input exits 2, absent/corrupt rows fail explicitly.

### `permission inspect`

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads pinned-runner state without child bytes, receipt, audit or claim. Missing interaction fails before event/run/database checks; stale/unavailable owner fails operationally.

Result `request` contains identity `event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq`; context `classification,visibility,backend,native_version`; capability `answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq`; `evidence,missing_evidence`.

Visibility: `visible,obscured,not_visible,unknown`; capability: `tested,unknown`. Answerable requests have null `capability_reason`; others return a bounded explanation. Supported decisions are tested `allow,deny`; blocked/unsupported returns empty decisions/null token. Otherwise token is lowercase 64-hex, request-bound. Missing facts are `operation,arguments,cwd,environment,targets,effects,choice_scope`; operation requires nonblank/unredacted/untruncated evidence. Obtain necessary missing facts before deciding.

### `permission respond`

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]
```

Require positive IDs/epoch, exact token, nonblank key and evidence references of 1–2048 UTF-8 bytes with non-whitespace and no C0/DEL. References identify host evidence without embedded commands/screens/secrets. `director-auto` requires current host mode `auto`; `user` records a user decision in any mode. Resolve mode independently of member policy; other/unknown modes require user permission. Native/managed rules remain authoritative. Shape/source-mode errors exit 2 before effects. Record operation key before dispatch.

Validate retained keys first: matching metadata returns stored result with zero new bytes; conflicting reuse fails. New keys cannot bypass active/submitted/uncertain/abandoned claims.

Result `response` contains `status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,decision_source,stage,output_seq,reason`. Status `accepted` means submitted, `held/rejected` zero bytes, `uncertain` partial/submitted/unknown. Input/sequence/reason may be null. Acceptance establishes key submission independently of native approval/execution.

Retained accepted/submitted returns accepted/submitted; in-progress/uncertain returns uncertain/unknown; settled not-submitted/abandoned returns rejected/zero_bytes for this retry, preserving original claim; ready takes exact-metadata live validation. Uncertainty reports recorded cause or `prior transmission remains uncertain`.

Accepted/prior submitted exits 0; structured held/rejected/uncertain prints selected response and exits 1: `Error: permission response STATUS for interaction INTERACTION_ID: REASON`, appending ` (input_id=INPUT_ID)` when known. Pre-reply failures retain ordinary categories/context; usage exits 2.

Blocked current requests create no receipt/audit/claim/byte: `Error: runner control: permission request is blocked: REASON`, followed by known run/interaction/key context; capability reasons are `permission input is fenced`, `manual input accounting failed`, `observation degraded: CODE`, `observation transitions pending`, `observation evidence pending`, `observation update pending`, `observation incomplete`, `observation health unavailable`, `terminal input interrupted`, `outer terminal closed`, `native output closed`, `terminal output still arriving`, `terminal reply pending`, `permission request is not visible`, or `permission request is not the current interaction`, matching inspection.

### `member recover`

`cafleet member recover MEMBER_ID [--json]` reconciles active/deregistered recorded runs without starting agents; absent run fails before mutation.

Exceptional form: `cafleet member recover MEMBER_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. All four options are required, with positive inspected snapshot IDs/epoch and nonempty operator attestation that owner/startup processes ended. Invalid combinations exit 2; changed ownership rejects and positive live evidence protects the owner.

Result fields: `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcomes: `recovered,recovering,live_or_unreachable,operator_reconciliation_required`. Counts cover newly committed work; incomplete/unreachable outcomes remain distinct from recovery.

## `setup` and `doctor`

`cafleet setup [--coding-agent AGENT...]` prepares/migrates the database, then installs embedded skills/presets offline in backend registry order, default every backend. First failure stops installation, exit 1: `failed to install skills into <skills_dir>: <error>`, `failed to install preset into <target>: <error>` or config-path error.

For `hermes`, setup installs the skills, then merges six Director-only globs (`cafleet member prompt *`, `cafleet member exec *`, `cafleet member press *`, `cafleet member recover *`, `cafleet permission respond *`, `cafleet input reconcile *`) into `approvals.deny` through `hermes config path`, `hermes config get --json approvals.deny` and `hermes config set approvals.deny '<compact JSON>'`, keeping existing entries and their order. It prints `hermes: approvals.deny already holds CAFleet's Director-only rules -> <config path>` or `hermes: added approvals.deny rules -> <config path>: <glob>, …`. Failure: `failed to install hermes approval rules into <config path>: <cause>`, no installation record; when `hermes config path` itself fails, `<config path>` is the resolved Hermes base. `--coding-agent hermes` requires `hermes` on `PATH`; plain `cafleet setup` without it installs the skills, records the installation and prints `hermes: skipped approvals.deny rules (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes`.

`cafleet doctor [--json]` emits every `checks` entry with `name,status,detail`; statuses `ok,info,error`. Names cover multiplexer/database, backend configuration/executable/installation, OpenCode presets, `hermes.approvals.mode` and `hermes.approvals.deny` (after `hermes.executable`; `ok` details `smart` and `holds CAFleet's Director-only rules`, `error` details the preflight messages, `info` `unavailable: executable is absent` without `hermes`) and `member_runtime.<run_id>`. Asset-version mismatch is info with `; run 'cafleet setup --coding-agent <agent>'`. Any error exits 1 after all checks with `Error: doctor found errors`.

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
| `member create` | The hermes configuration fails the availability preflight | The preflight errors under § `member create` | 1 |
| `member delete` | A `hermes` run's work stop cannot be verified | `Error: hermes work for member <ID> could not be verified as stopped: <cause>; member was not deleted.` | 1 |
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
| `member prompt` / `member exec` | Ordinary member without a recorded run | `Error: member <ID> has no recorded run` | 1 |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 |
| `message send` / `member create` | `--file <path>` to an empty file | `Error: --file <path>: file is empty.` | 1 |
| `message send` / `member create` | `--file -` with empty stdin | `Error: --file -: stdin is empty.` | 1 |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 |
| `message send` / `member create` | `--file <path>` to invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` | 1 |
