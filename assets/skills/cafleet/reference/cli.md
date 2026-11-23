# CAFleet CLI reference

Exact command facts for running CAFleet and interpreting its results.

## Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize the version's database and install assets | — | no |
| `doctor` | Diagnose multiplexer, database and agent assets | — | yes |
| `fleet create` | Register the calling Director atomically | — | yes |
| `fleet list` / `fleet show` | Read fleet registry | `FLEET_ID` for show | yes |
| `fleet delete` | End a fleet's ordinary members, then deregister the fleet | `FLEET_ID` | yes |
| `member create` | Register and spawn an independent agent | `--fleet-id` relationship | yes |
| `member delete` | Kill its owned pane and deregister | `MEMBER_ID` | yes |
| `member show` / `member list` | Read detail / active roster | `MEMBER_ID` / `FLEET_ID` | yes |
| `member prompt` / `member exec` | Submit authorized native input | `MEMBER_ID` | yes |
| `member press` | Request explicit ordered keys | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `event show` | Read a durable runtime event and current state | `EVENT_ID` | yes |
| `input show` / `input reconcile` | Read / settle a durable input receipt | `INPUT_ID` | yes |
| `permission inspect` / `permission respond` | Inspect / answer the current native permission request or question | `INTERACTION_ID` | yes |
| `message send` | Persist a point-to-point message for the recipient's runner to preview | Sender/recipient flags | yes |
| `message poll` | Read and consume incoming deliveries | `MEMBER_ID` | yes |
| `message listen` | Notify pending messages through stdout | `MEMBER_ID` | JSONL only |
| `message show` | Read one message | `MESSAGE_ID` | yes |

`cafleet --version` prints `cafleet <version>` and exits 0.

## Positional subject ids

Place the complete DB-assigned integer subject immediately after its subcommand. Related fleet/recipient state comes from that row; missing or malformed IDs use native parser errors, exit 2.

## Director-only command guard

The Director-only commands are exactly `member prompt`, `member exec`, `member press`, `permission respond` and `input reconcile`. Each requires `--from-member-id <director-member-id>`, a positive integer (missing: parse error, exit 2); `permission respond` takes it as its pinned Director. After parsing and the schema-version guard, and before any multiplexer, lock, socket or file access or write, the command resolves its subject's fleet (a member's fleet, or the fleet of the run owning an input or interaction) and requires it live with that ID as its root Director. An unknown subject keeps its not-found error, which comes first. Otherwise it exits 1, prints nothing on stdout and writes `Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.` to stderr.

Every other command carries no guard refusal; among them, `permission inspect` still requires `--from-member-id` as its pinned Director (§ `permission inspect`). The guard is cooperative, not a security boundary: it stops a member that passes its own `YOUR MEMBER ID:`, and a member that copies `DIRECTOR MEMBER ID:` passes it. A Director-only command the Director dispatches into a member pane with `member exec` is checked by the ID it carries.

## Database selection

The global option `--database-url <url>`, accepted before or after the subcommand, selects the SQLite database; omitted, the default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db` applies, `<VERSION>` being the package version (e.g. `0.31.1`). Explicit `sqlite:///<path>` resolves relative paths against cwd and leaves literal tildes unexpanded. CAFleet reads no `CAFLEET_*` environment variable.

A member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command. One that omits it on a custom-database fleet reaches the default database: absent, the command fails with `no cafleet database; run 'cafleet setup'`; present, its literal IDs may match another fleet.

Config-location overrides `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, `HERMES_HOME` (default `~/.hermes`) and `PI_CODING_AGENT_DIR` (default `~/.pi/agent`) require absolute paths: `Error: <VAR> must be an absolute path (got '<value>')`, exit 1; a set override or `HOME` that is not valid UTF-8 fails with `Error: <VAR> is not valid UTF-8`, exit 1. Creation forwards only the selected backend's set override, and launches the runner with `--database-url sqlite:///<canonical path>`.

## Output shapes

Use JSON for complete untruncated text; text output truncates message bodies at a fixed 200 codepoints. Message records contain `message_id`, `from_member_id`, integer `to_member_id`, `created_at`, `status_state`, `status_timestamp` and `text`.

Fleet creation returns `fleet_id`, `name`, `created_at` and `director` identity/description/registration/placement; lists return records and show includes nullable `deleted_at`. Deletion returns `fleet_id`, `deregistered_count` and `members`. Send returns `message`; poll returns an array of the records it consumed; show returns `message`. Member deletion returns `member_id`, `pane_status`; capture returns `member_id`, `pane_id`, `lines`, `content`, `captured_at`, `content_sha256`. Runtime commands define their fields below.

### Member list output

Roster records contain `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_consumed`, `idle`; `last_consumed` is when the member's poll last consumed an incoming message. Fleet-reference matches have kind `director`; others `member`. Missing activity may be null and pane placement pending.

Every JSON `placement` has ordered keys `backend, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side, mux_pane_id, coding_agent, created_at`; `mux_window_id`, `mux_window_name` and `mux_pane_id` are null together while creation is pending. New fleets name windows `cafleet-<fleet_id>-<ordinal>`: `window_ordinal` is `0` for the Director window and `1` or greater for overflow windows; `column_side` is `left` or `right`. Members using an existing window copy its recorded name.

Text rows list `member_id`, `name`, `kind`, `backend`, `window` (the window name) and `pane_id`, then `idle`; a pending window or pane prints `(pending)` and absent activity `-`:

```text
2 members:
  member_id  name           kind      backend   window         pane_id  idle
  8          director       director  claude    main           %3       12s
  9          drafter        member    claude    cafleet-3-1    %12      5s
```

## JSON output

Place `--json` before any `--` literal-key separator. JSON is compact UTF-8. Commands marked no in the summary, including the root group, reject `--json`.

## `permissions.allow` coverage

Broker prefix rules such as `Bash(cafleet message *)` cover broker operations, including `cafleet message listen ID --timeout 300`; authorize Monitor separately. Literal IDs preserve matching. An operator may keep `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` under ask to confirm each of the Director's explicit pane-input dispatches; ask outranks allow.

Claude receives `Bash(cafleet message *)` at spawn, and setup installs the Codex and OpenCode allow rules (§ `setup` and `doctor`); the Director-only commands refuse a member's own ID (§ Director-only command guard).

## Text bodies

Send/create require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve nonblank text exactly. Missing/both sources or blank positional text exit 2; blank/unreadable/invalid file or stdin exits 1 with § Error messages. Prompt accepts one positional line.

## Fleet commands

`cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]` registers the Director's pane and its multiplexer server as the root Director's placement without spawning. It and doctor's `multiplexer` and `multiplexer.caller_pane` rows are the only readers of the caller's multiplexer environment; every later command targets the recorded server (`tmux -S <socket>` or `herdr --session <name>`) and placement. Without `--pane`, it first refuses a caller whose process ancestry includes a shared server (`codex app-server`, `hermes gateway run`), before any multiplexer command, and then registers `TMUX_PANE` / `HERDR_PANE_ID` only when that pane's process is an ancestor of the `cafleet` process. A failed `TMUX`/`HERDR_ENV` detection gives `cafleet fleet create must be run inside a tmux or herdr session`.

| Input | Source, first match wins |
|---|---|
| Backend | The `--pane` ID form: `%` and digits (e.g. `%12`) is tmux, and exactly one `:` with text on both sides (e.g. `w4:p3N`) is herdr; else `TMUX`/`HERDR_ENV` detection |
| tmux server | `--mux-server`; everything before the last two commas of `TMUX`; `tmux display-message -p '#{socket_path}'`. With `--pane`, the `TMUX` sources apply only when the environment detects tmux |
| herdr session | `--mux-server`; `HERDR_SESSION`; the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; `default`. With `--pane`, these environment sources apply only when the environment detects herdr |
| Director pane | `--pane`, checked only for existence on the server; else `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr), verified against the caller's process ancestry |

Target resolution and the initial read of the pane's concrete session, window and pane IDs precede registration and naming. Each failure below exits 1 before any write:

| Condition | Error |
|---|---|
| tmux finds no server | `Error: no tmux server found; pass --mux-server <socket path>` |
| The pane is not on the chosen server (a tmux reply with an empty field or another `pane_id`) | `Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane` |
| `--pane` is neither ID form | `Error: --pane <pane> is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)` |
| A shared-server ancestor, without `--pane` | `Error: cafleet fleet create cannot verify this session's pane: a shared <label> server (pid <pid>) runs this command, …` naming both the tmux and the herdr guidance |
| The env pane fails verification, without `--pane` | `Error: cafleet fleet create cannot verify that <VAR>=<pane> is this session's pane: <cause>; …` naming the detected multiplexer's guidance |
| `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr) is unset, without `--pane` | `Error: cafleet fleet create cannot identify its <backend> pane because <VAR> is not set; …` naming the detected multiplexer's guidance |

After allocating the fleet ID inside the registration transaction, `fleet create` overwrites the resolved window/tab name with `cafleet-<fleet_id>-0`: `tmux -S <server> rename-window -t <window_id> <name>` or `herdr --session <server> tab rename <tab_id> <name>`. It reads the name back, requires exact equality and rechecks the original session, window and pane IDs before recording the canonical `mux_window_name` and committing. The tmux reader removes exactly the final LF and validates the echoed window ID; the herdr reader validates `result.tab.tab_id` and requires a string `label`. Every fallible registration read precedes commit, and success output follows confirmed commit.

Naming, verification and registration failures exit 1 with no success payload, including under `--json`, and roll back active uncommitted registration. A failed commit reports an unconfirmed registry outcome if SQLite already ended the transaction, and a rollback failure accompanies the original cause. Once naming is attempted, diagnostics identify the attempted fleet ID, backend/server, concrete window and pane IDs, requested name and cause, state that the terminal name may have changed, and direct inspection through `fleet list` and explicit-server multiplexer commands. Rolled-back IDs can be reused. Applied names remain after failure, crash or fleet deletion, while the Director pane keeps running. An output failure after confirmed commit preserves registration. Existing fleets retain their recorded names; subsequent user renames leave snapshots unchanged.

Each caller-verification refusal ends `then run cafleet fleet create with --pane <pane-id> --mux-server <server>`. `<cause>` is one of `the pane does not exist on the <backend> server <server>`, `this command runs in a nested PID namespace`, `the process ancestry could not be read: <read error>`, `the process ancestry stopped at pid <pid> before reaching pid 1` or `the pane's process <pane_pid> is not an ancestor of this command`. The guidance names what the user, not the agent, runs in the Director pane:

| Multiplexer | Guidance text in the refusal | The user replies with |
|---|---|---|
| tmux | `the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket)` | The pane ID (e.g. `%12`) and the socket path, passed as `--pane` and `--mux-server` |
| herdr | `! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)` | The pane ID (e.g. `w4:p3N`) and the session name, passed as `--pane` and `--mux-server` |

With `--pane`, `fleet create` reads no process ancestry, and an environment naming the other multiplexer or both is not an error; without `--mux-server` there, the server is the default tmux server or the herdr session `default`.

`cafleet fleet list [--json]` reads the registry. `cafleet fleet show FLEET_ID [--json]` includes deleted fleets; unknown IDs give `fleet 'X' not found.`, exit 1.

`cafleet fleet delete FLEET_ID [--json]` deletes active ordinary concrete panes through member deletion, then deregisters remaining members/fleet, retaining messages and the Director pane. It is idempotent; partial cleanup resumes on rerun.

| Outcome | Result |
|---|---|
| Lock contention until the deadline | Exit 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`; every pane and registry row is unchanged |
| Fleet layout lock busy for 60 seconds, after the pane locks | Exit 1 with `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; fleet was not deleted.`; every member is unchanged |
| A member cannot be ended | Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered |
| Placement set changed after cleanup began | Exit 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` |

## Messages

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Persist once and return: the send writes no pane input. An ordinary recipient's runner types each pending message, oldest first, at its next preview sweep, and retries it on its own; a member with no live runner keeps its messages pending. A self-send is stored already notified, and a root Director reads its messages through its own reception. Pre-commit storage failure exits 1. Consume each committed ID once.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns every incoming `input_required` record descending by status timestamp/message ID and consumes exactly those records in one transaction: each returned record shows `status_state` `completed`, no later poll returns it and no later listener notice announces it. Two concurrent polls never return the same message. An empty inbox prints `[]` (text: `No messages found.`) and exits 0. Unknown/inactive member: `Member ID not found`, exit 1, consuming nothing. A failure before commit consumes nothing; output lost after commit leaves the messages consumed, and `message show MESSAGE_ID --json` reads each one again.

### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS] [--codex-thread THREAD]`

Notifies an active member's pending messages, requiring a live root Director with concrete placement and no multiplexer. Unknown/inactive startup: `Member ID not found`, exit 1. Optional integer lease from 1 through 1800 inclusive is monotonic from startup; absence runs until closure/cancellation/failure.

Flush UTF-8 JSONL records: `ready` with `member_id,fleet_id` before replay; `pending` with `next`; `closed` with `member_id,reason`. There is no text mode or `--json`. Closure reasons are `timeout`, `member_inactive`, `fleet_deleted`, and `session_exited` in forwarding mode only; deletion wins simultaneous lifecycle closure. Normal closure exits 0, operational/output failures exit 1, usage errors exit 2; cancellation may omit `closed`.

```json
{"event":"pending","next":"cafleet message poll 1 --json"}
```

A `pending` notice is a wake signal, never a payload: `next` is the exact consuming command, `cafleet message poll <member-id> --json`, plus `--database-url <url>` when the listener was started with one (single-quoted when the URL needs it). Run it once per notice; it returns and consumes every pending body, and `[]` means an earlier poll already consumed what the notice announced.

**Hold.** After a notice is delivered, the listener holds later notices while any message pending at that notice stays unpolled. Any poll consumes the whole pending set and so releases the hold; the next snapshot with pending messages emits one notice announcing only messages committed after that poll. A hold lasting 300 s with messages committed after the held notice re-sends one bulk notice covering every unpolled message, held and fresh, and restarts the 300 s; held messages alone never trigger a re-notice.

**Notice bound.** A listener emits at most 1 notice, plus one per poll that consumed held messages, plus one per 300 s ceiling firing, so a burst of any size adds no notice beyond the outstanding one. Within one listener, two notices announce the same message only through a ceiling re-notice. A turn with k mid-turn polls can queue at most k + 1 notices behind it. A restarted listener emits one notice covering every unpolled message right after `ready`. Messages you poll before any notice announces them are never notified.

Listening changes no consumption/notification state and retries no pane delivery; concurrent listeners are independent. Preserve incomplete JSONL across native reads.

`--codex-thread THREAD` forwards notices into a Codex thread and keeps the record, hold, replay, lease and idle contracts:

- Stdout carries only `ready` and the final `closed` record, written right before exit with every reason, `session_exited` included.
- Each `pending` notice, and each `closed` record with reason `timeout`, `member_inactive` or `fleet_deleted`, is forwarded in order through `codex queue --thread THREAD --message '[cafleet listen record] <compact record JSON>'`, one at a time with a 10 s deadline each. `ready` and `closed` `session_exited` are never forwarded.
- Any non-zero `codex queue` exit ends forwarding, a full thread queue (`queue cannot contain more than 100 submissions`) included; nothing is retried.
- At startup the listener pins its nearest ancestor whose argv[0] basename is exactly `codex`. When that process exits (a reused pid counts as exited), stdout gets `closed` `session_exited` and the listener exits 0.

| Condition | Result |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| Process inspection fails at startup | The read error propagates before `ready`, including macOS `cannot run ps: Operation not permitted (os error 1)` |
| No Codex ancestor at startup | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| `codex queue` exits non-zero while the pinned session is alive | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, exit 1 |
| `codex queue` exceeds 10 s while the pinned session is alive | killed; stderr `codex queue for thread <THREAD> did not finish within 10 s`, exit 1 |
| `codex queue` exits non-zero or exceeds 10 s after the pinned session exited | stdout `closed` `session_exited`, exit 0 |
| Any other failure after `ready` | its usual diagnostic, exit 1 |

For sandbox startup failures, confirm exit and follow the [Codex native reception recipe](coding-agents.md#codex-native-director-reception) for approved escalation and readiness checks.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns its full record in any state and changes no state; use it to read a consumed message again.

## Push notifications

Profile previews are `[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.` Delivery fallback leads with Escape and appends ` [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.`

Stored text stays exact. Previews fold newlines to `⏎`, tabs to `␉`, other C0/DEL controls to `\xNN`. At most 4000 Unicode scalars appear inline; longer text becomes `Body is N characters; run cafleet message poll RECIPIENT_ID --json to read it.` independently of the 200-codepoint text-output limit. `RECIPIENT_ID` is the recipient; the pointer never carries `--database-url`. Delivery method selects the clause before writing. Preview leaves consumption `input_required`.

### Wrapped delivery methods

Validate identity, ownership, reachability, storage and the operation's own receipt first. Its own uncertain attempt requires inspection; while another prompt or exec of the run is uncertain, the runner's FIFO is stopped. Unknown detection alone permits fallback. A verified-composer run has empty-composer observation and the operation's measured plan from its recorded backend's profile.

An open/candidate interaction (`permission_request`, `question`, `unknown_interaction`, including obscured) or earlier queued input defers a prompt or exec with zero bytes, including Escape, and leaves a preview pending. A verified empty composer with a measured plan receives text, a short settle and the measured submit key without Escape. Only an interaction that opens or becomes a candidate during the write stops it; keyboard, paste and pointer input and terminal replies leave the write running, and post-submit verification detects a literal that was not submitted. Permission tokens and gated answers still revoke on keyboard and pointer input.

A measured plan with an unverified/nonempty composer queues a prompt or exec under `composer_busy` and leaves a preview pending, then delivers when empty; a human draft blocks previews until it is cleared or submitted. Working members with empty composers receive profile input queued natively behind their turn. Explicit interruption uses `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Escape`.

An absent plan or profile uses fallback `submission_plan_unavailable`: Escape, settle, literal (`! ` prefix for exec), settle, Enter. Recheck interactions before each stage; permission answers require their tested plan.

Long input uses a plan's declared bracketed-paste framing when the child has enabled it; otherwise a preview over its measured plain limit uses the short `poll` pointer. Oversized prompt/exec input without safe framing is refused before typing; a queued refusal settles `not_submitted/plain_limit_exceeded`. On verified-composer runs, `submitted` requires confirmation by a verified empty composer or a newly opened interaction; an ineffective submit key records `uncertain/submit_unconfirmed`, stage `submitted`. Insufficient immediate budget returns zero-byte `queued` with `delivery_budget_exhausted`.

### Runner queue

The runner's FIFO holds prompt and exec input only. Accepted deferred input returns durable `ready`, `deferred_at`, `defer_reason` and its input ID; it outlives the invocation and drains automatically. Inspect `cafleet input show INPUT_ID --json`. The Director receives one `input_delivery_result` for its first submitted/not-submitted/uncertain outcome and for every immediate stopped staged write; `input show` reports every receipt.

Zero-byte reasons are `pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted`, `plain_limit_exceeded`, `target_inactive`, `target_changed`, `runner_stopped` or `runner_lost`.

A preview is typed only from the runner's sweep, when the FIFO is empty and not stopped and no attempt is active, so queued prompt and exec input always goes first. On a verified-composer run it is typed only into a verified empty composer; a preview whose submission is uncertain stays unnotified and is typed again once the composer is verified empty, so a recipient can see it twice and deduplicates by message ID. On a run without composer verification an uncertain preview is stamped notified and not typed again. An uncertain preview publishes one `input_delivery_result` with `input_id` null and its `message_id`. A composer showing only a native suggestion counts as empty. Runner housekeeping checks the queue head each second, independently of the queue turn, and cancels a removed head target as `not_submitted/target_inactive`. A written immediate attempt that fails records `uncertain/write_failed`. After a prompt or exec records `uncertain`, on every run, the FIFO stays stopped and no queued input and no preview is written until `input reconcile` settles it; a queued head or pending preview held 3 minutes behind it generates one `input_delivery_stalled` naming it in `blocked_by_input_id`. A head that starts no attempt within 3 minutes of becoming the head, excluding time an open interaction blocks it, generates one `input_delivery_stalled` with a `cause`; [Runtime events](#runtime-events) owns the recovery action.

### Held delivery

Degraded/recovering/lost observation, unavailable ownership/storage, uncertain reachability and pending/changed placement hold prompt/exec input without queue acceptance or writing, and leave previews pending. Pending previews leave notification unset and grant no permission.

### Accepted trade-offs

Delivery fallback can interrupt work, dismiss undetected prompts or combine unsent drafts. A host may need multiple Escapes; submission remains valid when the host queues instead of interrupting.

Prompt/exec carry no clause, so inspect open prompts first. Repeated previews are consumed by ID; concurrent sends may interrupt earlier turns, and interruption can end shell tools. The CLI defers termination during pane input.

## Member commands

Pane operations require active placement; opaque pane IDs may be pending. Individual deletion protects the Director.

### `member create`

`cafleet member create --fleet-id ID --name NAME --description DESCRIPTION [--coding-agent AGENT] [--model MODEL] [--permission-mode MODE] [--effort LEVEL] (PROMPT | --file PATH) [--json]`

Registers and spawns, resolving the acting Director from the fleet. Omitted backend inherits its placement backend. Description is one sentence. CAFleet names every member pane `<member_id> - <name>`, such as `12 - drafter` (herdr pane label, tmux pane title; Claude and pi also set their own title), and `member list` `window` and `pane_id` locate it. Model forwards to the native CLI; OpenCode requires `<provider-id>/<model-id>`, Hermes passes any value through, and pi passes any value without a `:<thinking>` suffix through. Hermes argv: `hermes chat --cli --source tool --in <cwd>`, then optional `--model MODEL` and `--reasoning LEVEL`, then `-q PROMPT`; it never carries `--provider`, `--resume`, `--continue` or `--create-if-missing`. pi argv: `pi --cafleet-gate MODE --no-approve --tui-mode fullscreen --name "<member_id> - <name>"`, then optional `--model MODEL` and `--thinking LEVEL`, then `-- PROMPT`, as a fresh session.


Permission choices: `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)`. Invalid selection: `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, exit 2. Mode classes (documentation only; argv and run records are unchanged): `auto` (native automatic review or a fixed allow/deny policy) is claude `auto`, `dontAsk`, `bypassPermissions`; codex `approve-for-me`, `never`; opencode `cafleet`; hermes `smart`; pi `cafleet`. `manual` (native interactive approval) is claude `default`, `manual`, `acceptEdits`, `plan`; codex `on-request`; opencode `cafleet-ask`; pi `cafleet-ask`. pi choices emit `--cafleet-gate MODE` and select the CAFleet member gate's mode. The Hermes choice emits no permission argv (`permission_argv_json` `[]`) and is recorded only; the operator configuration's `approvals.mode` governs the session.

Effort validation precedes side effects. An unlisted level exits 2 with `Error: --effort for the BACKEND backend must be one of LEVELS (got '<value>').`, LEVELS exactly as listed; any `--effort` for opencode exits 2 with `Error: opencode does not support reasoning effort.`

| Backend | LEVELS |
|---|---|
| claude | low, medium, high, xhigh, max |
| codex | minimal, low, medium, high, xhigh |
| hermes | none, minimal, low, medium, high, xhigh, max, ultra |
| pi | off, minimal, low, medium, high, xhigh, max |

Invalid OpenCode model: `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`, exit 2. A pi model whose text after its last `:` is a thinking level (`off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`): `Error: --model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '<value>').`, exit 2; other text after a `:`, such as `:free`, passes through.

A pi prompt body beginning with `@` exits 2 before registration with `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`

After prompt validation, creation waits up to 60 seconds for the fleet's pane layout lock, which serializes every member creation and deletion in the fleet, and holds it until creation returns. Under it the member takes the first free slot: the Director window's right column, then each overflow window's left and right columns, each holding one member per 15 rows of window height up to 8 (an overflow column at least one). When every column is full, creation opens an overflow window `cafleet-<fleet_id>-<ordinal>` in the Director's session without taking focus; ordinals only increase. Existing members never move. The multiplexer server comes from the Director's recorded placement, and the new placement copies it.

Text output is `ID NAME backend=AGENT pane=PANE window=WINDOW` plus the runtime line; `PANE` and `WINDOW` print `(pending)` until the placement is published.

| Outcome | Result |
|---|---|
| Fleet layout lock busy for 60 seconds | Exit 1, nothing registered: `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not created.` |
| An overflow window cannot be read (closed by hand, or every member pane in it died) | Exit 1, nothing registered: `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.` |
| The fleet's window ordinal changed under the lock | Exit 1, registration rolled back: `Error: fleet <fleet-id> window ordinal changed under the layout lock` |
| A malformed tmux `new-window` or herdr `tab create` reply | Exit 1 with `Error: unexpected tmux new-window output: <quoted>` or `Error: herdr tab create missing '<key>' field`, followed by `pane ID unknown; pane cleanup unconfirmed` |
| A herdr re-parenting move failed | Creation continues; stderr shows `layout re-parenting incomplete: <cause>` |

Hermes availability preflight runs `hermes config path` under the member's `HERMES_HOME` after the config-dir and PATH checks. Each failure exits 1 before registration:

| Condition | Error |
|---|---|
| `hermes config path` fails or times out | `Error: cannot read hermes configuration through 'hermes config path': <cause>` |
| `hermes config path` prints no absolute file path | `Error: cannot read hermes configuration through 'hermes config path': printed no absolute file path: <stdout>` |

A pi spawn requires the CAFleet pi extension `<pi base>/extensions/cafleet.ts` in every launch mode: `Error: cannot read the CAFleet pi extension at <path>: <cause>; run 'cafleet setup --coding-agent pi' first`, exit 1 before registration. A gated member whose extension cannot register `--cafleet-gate` exits at startup with `Error: Unknown option: --cafleet-gate`, reported as `runner_exited`.

Substitute exactly `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}` and `{database_url}` into literal lines; `{database_url}` renders `default` for the default database and `sqlite:///<canonical path>` otherwise. A non-default database with a prompt lacking `{database_url}` exits 1 before registration: `Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url`. Double literal braces `{{`/`}}`; malformed/unknown placeholders exit 2 before registration (§ Error messages).

Probe native `--version` and record it as the run's `native_version`; the version selects nothing, and every member launches with its backend's bundled profile. Hermes takes its version from a first line matching `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$` (`Hermes Agent vunknown (2026.9.24)` records `2026.9.24`); pi prints the bare token (`1.5.0` records `1.5.0`). Failed probes, including a first line with zero or several SemVer tokens or a non-matching Hermes line, warn `Warning: native version unavailable for BACKEND: CAUSE.`, record a null `native_version` and continue with the backend's profile. Creation waits for native-child startup.

Failure compensation shuts down owned runner/child, closes pane, removes descriptor, marks run `failed` and deregisters/removes placement, retaining the primary exit category. Cleanup diagnostics include `cleanup failed for pane <id>: <detail>`, `owned child cleanup remains unconfirmed`, descriptor/run/deregistration errors and `pane ID unknown; pane cleanup unconfirmed`. Inspect unconfirmed resources.

### `member delete`

`cafleet member delete MEMBER_ID [--json]` shuts down the member's live runner (a runner already gone is settled as lost without a shutdown request), closes the locked pane and deregisters. `member runtime: ECONNREFUSED` from a runner that exited during deletion is resolved by rerunning the delete. Post-close database errors retain the cause and closed-pane evidence; layout-restoration diagnostics preserve successful deregistration. Director protection uses § Error messages.

Before runner shutdown, a `hermes` run's session work is stopped through native pane input (`Ctrl+C` when busy, then `/stop` and `/exit`) and its registered background processes are verified as stopped, within a 20-second settlement budget per Hermes member (also in `fleet delete`) that extends the 5-second deletion deadline.

| Outcome | Result |
|---|---|
| Busy pane input | Exit 1, member unchanged: `member ID's pane input remained busy for 5 seconds; member was not deleted.` |
| Fleet layout lock busy for 60 seconds, after the pane lock and before cleanup | Exit 1, run, pane and row unchanged: `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not deleted.` |
| Unverified Hermes stop | Exit 1, member kept, a retry re-runs the settlement: `Error: hermes work for member ID could not be verified as stopped: CAUSE; member was not deleted.` |
| Hermes client already exited | Deletion completes with `warning: hermes client for member ID had already exited; its session work may still be running in the Hermes gateway` |

### `member list`

`cafleet member list FLEET_ID [--json]` lists active Director and ordinary records (§ Member list output).

### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active member, including Director, without a multiplexer; includes `kind`, placement and runtime. Text is `ID NAME STATUS window=WINDOW`, then the runtime line for an ordinary member; `WINDOW` prints `(pending)` while pending. For a `running` or `degraded` run it also probes the runner's health (1 s deadline) and reports the outcome in the runtime object (§ Runtime presentation); a failed probe still exits 0.

### `member prompt`

`cafleet member prompt MEMBER_ID --from-member-id DIRECTOR_ID TEXT [--json]` requires a nonblank control-free line: `text must be one non-empty line without control characters.`, exit 2.

Director target, before I/O: `cannot prompt the Director's own pane`, exit 1. Runner delivery uses profile/fallback/queue, and exit 0 establishes submission/queue acceptance. Result fields: `member_id,pane_id,text,input_id,delivery`.

| Condition | Error body (exit 1) |
|---|---|
| Pre-write error, including a held runner reply | `prompt was not dispatched to member ID: CAUSE` |
| Partial uncertainty | `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

Uncertain errors include `input_id=INPUT_ID`; inspect that result and pane before a deliberate retry. Above-limit input with no safe framing uses the pre-write error and the CAUSE strings below, with zero bytes and no reservation at an immediate attempt. UNIT is `bytes` or `characters` for the plan's byte/scalar unit; N includes prefix plus literal.

| Missing delivery mechanism | CAUSE |
|---|---|
| Profile declares plain framing | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the BACKEND profile has no bracketed-paste delivery` |
| Profile declares bracketed paste, child has not enabled it | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the native CLI has not enabled bracketed paste` |

### `member exec`

`cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID --file PATH [--json]`; `--file -` reads UTF-8 stdin, stripping at most one final LF/CRLF. Exactly one source; a nonblank line excludes CR/LF, NUL, tabs and other C0/DEL: `command must be one non-empty line without control characters.`, exit 2 before I/O. Multiline tasks use one-line script invocations.

The runner submits `! ` plus command through profile/fallback/queue. The host owns cwd, environment, permissions, output and exit. Exit 0 establishes submission/queue acceptance; JSON contains `member_id,pane_id,coding_agent,command,dispatched,input_id,delivery`, with `dispatched=false` when queued.

| Condition | Error body (exit 1) |
|---|---|
| Root Director, before I/O | `cannot exec in the Director's own pane` |
| Runner refusal before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

Uncertain errors include `input_id=INPUT_ID`. The pre-write over-limit CAUSE strings are shared with [member prompt](#member-prompt); queued over-limit refusals settle `not_submitted/plain_limit_exceeded` and publish a result.

A failing first write may partially type; failure after Enter may have executed. Inspect native output and reconcile receipts before a newly authorized retry. After completion, explicitly prompt continuation if the host has not resumed.

### `member press`

`cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID KEY... [--json]` requests 1–64 ordered keys, each at most 32 UTF-8 bytes, with no implicit Escape/Enter or shell submission. For punctuation use `cafleet member press 42 --from-member-id 1 -- '-' '+' ';'`. Wrapped members receive manual keys through the outer pane. Every probe and the write share one five-second invocation deadline. Exit 0 establishes backend acknowledgment, independently of native approval/effects; fields are `member_id,pane_id,keys,status`, with `status=acknowledged`.

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
| Root Director target, before I/O | `cannot press keys in the Director's own pane` |
| Input lock remains busy | `member ID's pane input remained busy` |
| Revalidated target changes | `member ID placement changed before key dispatch` |
| Selected live context differs from placement | `press target context does not match the recorded placement` |

Request-bound decisions use permission respond.

### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` reads trailing lines (default 20); `--ansi` preserves escapes, otherwise clean redraws/escapes. Capture requires no input lock; pending placement fails. `captured_at` stamps reading; `content_sha256` hashes exact emitted `content`.

## Pane input and previews

Writers share one OS lock per concrete pane, including duplicate placements; capture is read-only. Human input remains independent.

Keys accept printable ASCII U+0021–U+007E, `Space`, `Escape`, `Enter`, `Tab`, `Backspace`, arrows, `F1`–`F12`, `Ctrl+a`–`Ctrl+z`, `Shift+Tab`. Printable case is preserved; names/modifiers are case-insensitive and control letters canonicalize lowercase.

## Member runtime

### Runtime presentation

Create/show include `member_id,name,registered_at,placement,runtime`; show also includes `description,status,kind`. Member creation records every ordinary member's run; `runtime` is null for the root Director. Runtime fields describe identity (`run_id,resolved_mode,owner_epoch`), state (`status,last_health_at,coverage,current_interaction_id`), ownership (`owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`) and `unresolved_input_ids`.

Unresolved IDs include queued `ready`, `in_progress`, `accepted`, `uncertain`, ascending; accepted needs settlement, in-progress/uncertain needs Director reconciliation. These registry fields establish no live health; member show's probe keys do.

Member show alone appends, after `child_start_identity`:

| Key | Value |
|---|---|
| `observation_probe` | `skipped` (status neither `running` nor `degraded`), `answered` or `failed` |
| `observation` | Ordered `visibility,eligibility,degraded,diagnostic,writer_diagnostic,pending_count,gap_count,gap_open_since`; null when skipped, failed or the child has not started |
| `observation_error` | The probe failure message when `failed`, otherwise null |
| `diagnostics` | The run's latest 5 stored runner diagnostics, newest first, each `source,message,first_at,last_at,count` |

`eligibility` is `composer_empty` or `hold:<reason>` (`candidate`, `interaction`, `unknown`, `observer_unavailable`, `gap`). `degraded` true means the runner cannot store its events and holds generated input; `writer_diagnostic` then names the storage failure, and `gap_open_since` is the RFC 3339 start of the open observation gap (null for a gap recorded after the child exited). Member show's text runtime line appends ` observation=STATE`, then ` degraded` when degraded; `STATE` is the eligibility, `pending` when answered without an observation, `unavailable` when failed and `-` when skipped, as in `Runtime: run=37 mode=auto status=running coverage=tested observation=hold:gap degraded`.

Example uncertain exec error: `native command delivery to member 110 is uncertain: write_failed (method=profile)`, then `input_id=INPUT_ID`.

### Delivery reporting

`delivery` contains `status,method,reason`. Method is `profile`, `fallback` or null before selection. Held prompt/exec exits 1. `uncertain` requires reconciliation and exits 1; `not_submitted` records cancellation or an evidence-based reconciliation.

Queue reasons are `interaction_open,interaction_candidate,composer_busy,earlier_input,delivery_budget_exhausted`; the fallback reason is `submission_plan_unavailable`. Pending previews stay unnotified until the sweep submits them; later results appear in receipts/events without changing original command output.

### Runtime events

Notifications contain the full event JSON, observed member as sender and Director as recipient. Verify association through event show. Kinds: `coverage_unknown,interaction_opened,interaction_closed,input_delivery_result,input_delivery_stalled,runner_degraded,runner_lost,runner_exited`.

Identity fields: `event_id,run_id,event_seq,member_id,fleet_id,director_member_id`; context includes `kind,interaction_id,classification,observed_at,source,decision_outcome,evidence`, closure `close_reason` or lifecycle `cause`. Permission/question `director_action` contains `kind,instruction,inspect_argv`: review kinds are `review_permission_request,review_question`; `review_question` accompanies every question `interaction_opened`, with the same `permission inspect` argv. Delivery actions have their receipt-derived forms below; other events carry null action.

Evidence contains `backend,native_version,rule_id,screen_seq,evidence_digest` and no screen text; read the operation and choices from `member capture`. Payloads are bounded at 16 KiB.

Delivery results contain `input_id,message_id,delivery,stage`, null interaction/classification, source `runner_lifecycle`, outcome `unknown`, null `director_action` and `cause={"kind":STOP_CAUSE}` whenever a stop cause is recorded. A prompt or exec result has its input ID and null `message_id`, with delivery and cause derived from its receipt; an uncertain preview has null `input_id`, its `message_id` and delivery `{"status":"uncertain","method":METHOD,"reason":null}`, deduplicated by run and message. Stages are `zero_bytes,partial,submitted,unknown`; an uncertain or in-progress prefix short of the submit key is `partial`. Deferred first results and immediate stopped staged writes publish once, without literal payload.

`input_delivery_stalled` reports input that waited 3 minutes without a delivery attempt: a queued prompt or exec head, once per run and input; the sweep's selected preview held because the composer is not verified empty, once per run and message; or a queued head or pending preview held behind the run's lowest-ID `uncertain` prompt or exec, once per run and blocker. Time an open interaction blocks the input does not count, and a head held behind an uncertain input reports only the blocked form. It carries ordered `input_id,message_id,blocked_by_input_id,waiting_since`, with exactly one of the three IDs set and `waiting_since` in UTC RFC 3339; its null interaction/classification, lifecycle source and evidence shape match delivery results. A head or preview action is `{"kind":"review_stalled_delivery","instruction":INSTRUCTION,"inspect_argv":["cafleet","member","capture","MEMBER_ID","--json"]}`. INSTRUCTION is exactly `Capture the member's pane. When the composer holds text, submit or clear it with member press keys the capture establishes, or ask the user to clear or submit it on the member's native surface.` A blocked action is `{"kind":"reconcile_uncertain_input","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","BLOCKER_ID","--json"]}`, with `BLOCKER_ID` the `blocked_by_input_id`. INSTRUCTION is exactly `Inspect the uncertain input and the member's pane, then settle the input with input reconcile from that evidence. Queued input and message previews resume once it settles.`

A `runner_degraded` event with cause `{"kind":"event_rejected","rejected_kind":KIND,"rejected_observed_at":TIMESTAMP,"error":MESSAGE}` replaces a runtime event the runner could never store, at that event's sequence number; its action is null, the runner keeps working and no observation gap opens. `rejected_kind` names the lost event's kind. A rejected `interaction_opened` creates no interaction, so the native request stays visible only on the member's pane, and its later `interaction_closed` is rejected the same way.

A repeated delivery record for an already-reported input, preview or blocker, and a stalled head that is no longer `ready`, are stored as archived events (`archive_reason` `duplicate_delivery` or `head_not_ready`) with no inbox message.

Every argv array appends `--database-url URL` for a non-default database. [Director recovery](../roles/director.md#reconcile-generated-input) owns the procedure.

### `event show`

`cafleet event show EVENT_ID [--json]` returns `event,notification_state,message_id,archive_reason,run,interaction`. Event is full versioned payload, run current summary. Nullable interaction contains `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

### `input show` and `input reconcile`

`cafleet input show INPUT_ID [--json]`; `cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]`. Result `input` contains `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref,delivery,deferred_at,defer_reason,stop_cause`.

Kinds: `prompt,exec`; states: `ready,in_progress,accepted,uncertain,settled`. Ready certifies zero bytes. Stored dispositions: `submitted,not_submitted,abandoned_uncertain`; CLI: `submitted,not-submitted,abandoned-uncertain`. Receipts hold digests, excluding literals. Stable receipt IDs.

Projection: deferred ready → queued with original reason; other ready → held/`zero_bytes_ready`; in-progress, uncertain or abandoned-uncertain → uncertain; accepted/submitted → submitted; not-submitted → not_submitted with cancellation reason/null method or reconciled selection. A not-submitted receipt without a recorded method reports a null method with `method_unrecorded`.

`stop_cause` records why an attempt that wrote bytes ended without confirmed submission; it is required on transition to uncertain, survives settlement/reconciliation and is otherwise null.

| Cause | Recorded when |
|---|---|
| `interaction_open` | An interaction appeared during the write |
| `interaction_candidate` | A candidate interaction appeared during the write |
| `submit_unconfirmed` | Submit effect remained unconfirmed; uncertain, stage `submitted` |
| `deadline_expired` | The chunk bound was reached or too little budget remained before the submit key |
| `short_write` | A stopped prefix ends inside a scalar/paste marker, or an answer stage writes short |
| `write_failed` | PTY/checkpoint error, unrecorded possible I/O, failed/short close, short submit key, failed deferred, written immediate or permission attempt |
| `runner_stopped` | Shutdown ended an active attempt that may have written a child byte |

Every stop records `uncertain`, and the runner sends no second submit key. Director reconciliation of uncertainty requires nonempty evidence, owner synchronization and no active writer; an unreachable owner preserves uncertainty. Missing live placement fails with `Error: receipt reconciliation requires the member's live runner; the input stays uncertain`; repeating a settled ordinary disposition returns original evidence and a conflicting disposition fails. Settlement writes no input and establishes no native result.

Reconciling a queued `ready` prompt or exec as `not-submitted` cancels it with zero bytes and removes it from the queue.

Settlement grants no answer. IDs are positive; malformed input exits 2, absent/corrupt rows fail explicitly.

### `permission inspect`

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads pinned-runner state without child bytes. Missing interaction fails before event/run/database checks; stale/unavailable owner fails operationally.

Result `request` contains identity `event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq`; context `classification,visibility,backend,native_version`; capability `answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq`; `evidence,missing_evidence`.

Visibility: `visible,obscured,not_visible,unknown`; capability: `tested,unknown`. Answerable requests have null `capability_reason`; others return a bounded explanation. For a permission request, supported decisions are the tested subset of `allow,deny` and `supported_choices` is `[]`. For a question, `supported_decisions` is `[]` and `supported_choices` lists one `{choice_id,label,scope}` per tested choice; a question without one returns capability `unknown` with reason `question has no tested answer`. Blocked/unsupported returns empty arrays/null token. Otherwise token is lowercase 64-hex, request-bound. For a permission request, missing facts are `operation,arguments,cwd,environment,targets,effects,choice_scope`, because evidence carries none of them; obtain necessary missing facts before deciding. A question returns `missing_evidence` `[]`. Text `answers=` lists the supported decisions or choice ids.

### `permission respond`

`cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) [--json]`

Pass exactly one of `--decision` (permission request) and `--choice` (question, one of the inspected `supported_choices` ids); both, neither or a choice id outside 1–64 bytes of lowercase hyphen-separated words exit 2. Require positive IDs/epoch and the exact token; shape errors exit 2 before effects. Resolve mode independently of member policy; other/unknown modes require user permission. Native/managed rules remain authoritative.

The runner accepts one answer per episode and stores nothing about it. Once an answer's first byte is written, a later respond for that episode returns `rejected/zero_bytes` with reason `an answer was already sent for this episode`; a zero-byte refusal leaves the episode answerable.

Result `response` contains `status,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,stage,output_seq,reason`, exactly one of `decision` and `choice` non-null; text adds `choice=CHOICE` after `decision=DECISION`. Status `accepted` means submitted, `held/rejected` zero bytes, `uncertain` partial/submitted/unknown. Sequence/reason may be null; an uncertain reply reports its stop cause, or `manual_input` when keyboard or pointer input revoked the answer after a written stage. Acceptance establishes key submission independently of native approval/execution.

Accepted exits 0; structured held/rejected/uncertain prints selected response and exits 1: `Error: permission response STATUS for interaction INTERACTION_ID: REASON`. Pre-reply failures retain ordinary categories/context; usage exits 2.

Blocked current requests write no byte: `Error: runner control: permission request is blocked: REASON`, followed by known run/interaction context; capability reasons are `permission input is fenced`, `manual input accounting failed`, `observation degraded: CODE`, `observation transitions pending`, `observation update pending`, `observation incomplete`, `terminal input interrupted`, `outer terminal closed`, `native output closed`, `terminal output still arriving`, `terminal reply pending`, `permission request is not visible`, or `permission request is not the current interaction`, matching inspection.

A wrong-kind answer is a structured zero-byte `rejected` response (exit 1): `choice answers apply only to question interactions` (`--choice` on a non-question), `decision answers apply only to permission requests` (`--decision` on a question), or `choice CHOICE_ID has no tested answer for this question`.

## `setup` and `doctor`

`cafleet setup [--coding-agent AGENT...]` initializes the version's database, then installs embedded skills, presets and extensions offline in backend registry order, default every backend. First failure stops installation, exit 1: `failed to install skills into <skills_dir>: <error>`, `failed to install preset into <target>: <error>`, `failed to install extension into <target>: <error>`, `failed to enable hermes plugin cafleet in <config path>: <cause>` or config-path error.

Codex receives `<codex home>/rules/cafleet.rules`, allowing every `cafleet` command; OpenCode the `cafleet` and `cafleet-ask` presets (shell rules for the four member broker commands, `"cafleet": "deny"` tool permission) and the plugin `<preset base>/plugins/cafleet.ts`; Hermes the plugin under `<hermes home>/plugins/cafleet/`, enabled with gateway injection in the file `hermes config path` reports, refusing when `plugins.disabled` lists `cafleet`; pi the extension `<pi base>/extensions/cafleet.ts` with its `cafleet.executable` record, without running `pi`. Each plugin and extension runs only the `cafleet` recorded in its executable record by the setup that installed it. Setup writes no Director-only deny, ask or prompt entry. `--coding-agent hermes` requires `hermes` on `PATH`; plain setup without it installs the skills and prints `hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes`. `cafleet setup --coding-agent pi` prints:

```
pi: installed cafleet, cafleet-design-doc (v<version>) -> <pi base>/skills
pi: installed extension (v<version>) -> <pi base>/extensions/cafleet.ts
```

`cafleet doctor [--json]` runs every check without aborting. Text prints one `<status> <name>: <detail>` row per check in check order; `--json` prints one JSON array of `name,status,detail` objects in the same order, each detail equal to its text form without the `<status> <name>: ` prefix. Statuses are `ok`, `info`, `warn` and `error`; doctor exits 1 with `Error: doctor found errors` after printing when any check is `error`, otherwise 0, `warn` included.

When the `database` check is `ok`, one `member.<member_id>.runtime` row follows it for each active ordinary member with a non-terminal run, in member-ID order; a `running` or `degraded` run is probed with a 1 s deadline. `<eligibility>` and `<code>` follow § Runtime presentation. The first matching row applies:

| Situation | Status | Detail |
|---|---|---|
| Run `starting`, `prepared` or `recovering` | `info` | `run=<run_id> status=<status>: not probed` |
| Probe failed | `error` | `run=<run_id> health probe failed: <cause>; <hint>` |
| Answered without an observation | `info` | `run=<run_id> observation=pending` |
| Observation gap open for more than 30 s | `error` | `run=<run_id> observation=<eligibility> diagnostic=<code> gap_open_since=<ts>; <hint>` |
| Otherwise degraded | `warn` | `run=<run_id> observation=<eligibility> diagnostic=<code>` |
| Otherwise | `ok` | `run=<run_id> observation=<eligibility>` |

`<hint>` is exactly `inspect with 'cafleet member show <member_id> --json'; if it does not recover, run 'cafleet member delete <member_id>' and create a replacement member`, each quoted command appending ` --database-url URL` for a non-default database.

The `multiplexer` check reports the backend and server (`ok` `<backend> server=<server>`), and the `multiplexer.caller_pane` check that follows it reports the `fleet create` pane verification without `--pane` (§ Fleet commands):

| Situation | `multiplexer` | `multiplexer.caller_pane` |
|---|---|---|
| Pane verified | `ok` | `ok` `<VAR>=<pane_id> session=<session> window=<window_id>` |
| Shared-server ancestor | `info` `not checked: a shared <label> server (pid <pid>) supplies the caller environment` | `warn` with the refusal body |
| Env pane unset or unverified | `ok` | `warn` with the refusal body |
| Verification or verified-pane context read errored | `ok` | `error` with the error |
| Detection, server or availability failed | `error` with the cause | `info` `unavailable: the multiplexer is unavailable` |

The refusal body is the `fleet create` refusal without its `cafleet fleet create ` prefix.

Each backend reports `<agent>.configuration`, `<agent>.executable`, its preset and extension checks, then `<agent>.skills`. An absent executable is `info` `<binary> is optional and absent from PATH`, and that backend's preset, extension and skills checks are `info` `unavailable: executable is absent`. `<agent>.skills` byte-compares every embedded skill file with its installed copy, as the extension checks do: missing skills are `warn` `skills not installed at <skills dir>; run 'cafleet setup --coding-agent <agent>'`, and a differing or unreadable file is `error` `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'`.

Extension checks precede `<agent>.skills`: `opencode.extension` follows the OpenCode preset checks, `hermes.extension` follows `hermes.executable`, `hermes.plugin_enabled` follows `hermes.extension`, and `pi.extension` follows `pi.executable`. Each is `info` `unavailable: executable is absent` or `unavailable: configuration failed: <cause>` when its backend is unusable. When the resolved Hermes base does not exist, doctor runs no `hermes` process and both Hermes rows are `info` `unavailable: no Hermes home at <base>`.

| Check | `ok` detail | `error` details |
|---|---|---|
| `opencode.extension` | The plugin path | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent opencode'`; `<record path>: <cause>; run 'cafleet setup --coding-agent opencode'` |
| `hermes.extension` | The plugin directory | `cannot read hermes configuration through 'hermes config path': <cause>`; `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent hermes'`; `<record path>: <cause>; run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | `cafleet enabled with gateway injection` | `cannot read hermes configuration through 'hermes config get': <cause>`; `hermes plugins.disabled lists cafleet; remove it, then run 'cafleet setup --coding-agent hermes'`; `hermes plugin cafleet is not enabled: <key>, …; run 'cafleet setup --coding-agent hermes'` |
| `pi.extension` | The extension path | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent pi'`; `<record path>: <cause>; run 'cafleet setup --coding-agent pi'` |

An extension check is `ok` only when every embedded file equals its installed copy and the executable record names an existing executable file.

## Error messages

Application errors print as stderr text even with `--json`, each prefixed `Error: `; structured permission responses are the exception. Usage/parser/model/effort/line-validation errors exit 2; application, database and transport errors exit 1. Every row below exits 1 unless marked `(exit 2)`.

| Command | Situation | Error message |
|---|---|---|
| (any command except `setup` and `doctor`) | No database | `no cafleet database; run 'cafleet setup'` |
| (any command except `setup` and `doctor`) | Incompatible database file, including one from another cafleet version | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` |
| (any command with output) | Writing stdout fails with an error other than a broken pipe; a broken pipe stops output and keeps the command's own result | `Error: failed to write output: <cause>` |
| Director-only commands | `--from-member-id` is not the live subject fleet's root Director | § Director-only command guard |
| `fleet create` | No server, the pane is not on it, a malformed `--pane`, or without `--pane` a shared-server caller or an unset or unverifiable caller pane | The errors under § Fleet commands |
| `member create` | The backend binary is missing | `Error: binary <name> not found on PATH` |
| `message listen --codex-thread` | `codex` is missing, or no Codex ancestor process exists | The startup errors under § `message listen` |
| `member create` | The selected opencode preset is missing or unreadable | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` |
| `member create` | `hermes config path` is unreadable | The preflight errors under § `member create` |
| `member create` | The CAFleet pi extension is missing or unreadable | `Error: cannot read the CAFleet pi extension at <path>: <cause>; run 'cafleet setup --coding-agent pi' first` |
| `member create` | A pi prompt begins with `@` (exit 2) | The prompt error under § `member create` |
| `member create` | A non-default database and a prompt without `{database_url}` | The placeholder error under § `member create` |
| `member delete` | A `hermes` run's work stop cannot be verified | The settlement error under § `member delete` |
| `fleet create` | Director window naming, name read-back, pane-context verification or subsequent registration fails | The naming and transaction outcomes under § Fleet commands |
| `member create` | The fleet layout lock stays busy, an overflow window cannot be read, the window ordinal changed, or a new-window reply is malformed | The outcomes under § `member create` |
| `member delete` / `fleet delete` | The fleet layout lock stays busy for 60 seconds | The busy outcomes under § `member delete` and § Fleet commands |
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID | `Error: fleet '<fleet-id>' not found.` |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` |
| `member create` | The fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` |
| `member capture` / `member prompt` / `member exec` / `member press` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&#124;press&gt;.</code> |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` |

Text-body errors of `message send` and `member create`:

| Situation | Error message |
|---|---|
| Positional body empty or whitespace-only | `Error: text may not be empty.` (exit 2) |
| `--file <path>` to an empty file | `Error: --file <path>: file is empty.` |
| `--file -` with empty stdin | `Error: --file -: stdin is empty.` |
| `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` |
| `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` |
| `--file <path>` to invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` |
