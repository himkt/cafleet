# CAFleet CLI reference

Exact command facts for running CAFleet and interpreting its results.

## Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize the version's database and install assets | — | no |
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
| `permission inspect` / `permission respond` | Inspect / answer the current native permission request or question | `INTERACTION_ID` | yes |
| `message send` | Persist a point-to-point message and preview it for an ordinary recipient | Sender/recipient flags | yes |
| `message poll` / `message wait` | Read incoming deliveries / block until deliveries arrive | `MEMBER_ID` | yes |
| `message listen` | Stream pending inbox records through stdout | `MEMBER_ID` | JSONL only |
| `message ack` / `message show` | Consume / read one message | `MESSAGE_ID` | yes |

`cafleet --version` prints `cafleet <version>` and exits 0.

## Positional subject ids

Place the complete DB-assigned integer subject immediately after its subcommand. Related fleet/recipient state comes from that row; missing or malformed IDs use native parser errors, exit 2.

## Director-only command guard

The Director-only commands are exactly `member prompt`, `member exec`, `member press`, `member recover`, `permission respond` and `input reconcile`. Each requires `--from-member-id <director-member-id>`, a positive integer (missing: parse error, exit 2); `permission respond` takes it as its pinned Director. After parsing and the schema-version guard, and before any multiplexer, lock, socket or file access or write, the command resolves its subject's fleet (a member's fleet, or the fleet of the run owning an input or interaction) and requires it live with that ID as its root Director. An unknown subject keeps its not-found error, which comes first. Otherwise it exits 1, prints nothing on stdout and writes `Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.` to stderr.

Every other command, including `permission inspect`, `input show`, `event show`, `member list`, `member capture` and the lifecycle commands, carries no guard refusal; `permission inspect` still requires `--from-member-id` as its pinned Director (§ `permission inspect`). The guard is cooperative, not a security boundary: it stops a member that passes its own `YOUR MEMBER ID:`, and a member that copies `DIRECTOR MEMBER ID:` passes it. A Director-only command the Director dispatches into a member pane with `member exec` is checked by the ID it carries.

## Database selection

The global option `--database-url <url>`, accepted before or after the subcommand, selects the SQLite database; omitted, the default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db` applies, `<VERSION>` being the package version (e.g. `0.31.1`). Explicit `sqlite:///<path>` resolves relative paths against cwd and leaves literal tildes unexpanded. CAFleet reads no `CAFLEET_*` environment variable.

A member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command. One that omits it on a custom-database fleet reaches the default database: absent, the command fails with `no cafleet database; run 'cafleet setup'`; present, its literal IDs may match another fleet.

Config-location overrides `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, `HERMES_HOME` (default `~/.hermes`) and `PI_CODING_AGENT_DIR` (default `~/.pi/agent`) require absolute paths: `Error: <VAR> must be an absolute path (got '<value>')`, exit 1; a set override or `HOME` that is not valid UTF-8 fails with `Error: <VAR> is not valid UTF-8`, exit 1. Creation forwards only the selected backend's set override, and launches the runner with `--database-url sqlite:///<canonical path>`.

## Output shapes

Use JSON for complete untruncated text; text output truncates message bodies at a fixed 200 codepoints. Message records contain `message_id`, `from_member_id`, integer `to_member_id`, `created_at`, `status_state`, `status_timestamp` and `text`.

Fleet creation returns `fleet_id`, `name`, `created_at` and `director` identity/description/registration/placement; lists return records and show includes nullable `deleted_at`. Deletion returns `fleet_id`, `deregistered_count` and `members`. Send returns `message`, `notification_sent`, `input_id`, `delivery`; poll returns an array; ACK/show return `message`. Member deletion returns `member_id`, `pane_status`; capture returns `member_id`, `pane_id`, `lines`, `content`, `captured_at`, `content_sha256`. Runtime commands define their fields below.

### Member list output

Roster records contain `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack`, `idle`. Fleet-reference matches have kind `director`; others `member`. Missing activity may be null and pane placement pending.

## JSON output

Place `--json` before any `--` literal-key separator. JSON is compact UTF-8. Commands marked no in the summary, including the root group, reject `--json`.

## `permissions.allow` coverage

Broker prefix rules such as `Bash(cafleet message *)` cover finite wait/listen, including `cafleet message listen ID --timeout 300`; authorize Monitor separately. Literal IDs preserve matching. An operator may keep `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` under ask to confirm each of the Director's explicit pane-input dispatches; ask outranks allow. Director permission responses remain subject to effective native/managed rules.

Claude receives `Bash(cafleet message *)` at spawn, and setup installs the Codex and OpenCode allow rules (§ `setup` and `doctor`); the Director-only commands refuse a member's own ID (§ Director-only command guard). Members retain broker reception/evidence-routing authority.

## Text bodies

Send/create require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve nonblank text exactly. Missing/both sources or blank positional text exit 2; blank/unreadable/invalid file or stdin exits 1 with § Error messages. Prompt accepts one positional line.

## Fleet commands

`cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]` registers the Director's pane and its multiplexer server as the root Director's placement without spawning. It and doctor's `multiplexer` and `multiplexer.caller_pane` rows are the only readers of the caller's multiplexer environment; every later command targets the recorded server (`tmux -S <socket>` or `herdr --session <name>`) and placement. Without `--pane`, it first refuses a caller whose process ancestry includes a shared server (`codex app-server`, `hermes gateway run`), before any multiplexer command, and then registers `TMUX_PANE` / `HERDR_PANE_ID` only when that pane's process is an ancestor of the `cafleet` process. A failed `TMUX`/`HERDR_ENV` detection gives `cafleet fleet create must be run inside a tmux or herdr session`. Director messages use broker reception.

| Input | Source, first match wins |
|---|---|
| Backend | The `--pane` ID form: `%` and digits (e.g. `%12`) is tmux, and exactly one `:` with text on both sides (e.g. `w4:p3N`) is herdr; else `TMUX`/`HERDR_ENV` detection |
| tmux server | `--mux-server`; everything before the last two commas of `TMUX`; `tmux display-message -p '#{socket_path}'`. With `--pane`, the `TMUX` sources apply only when the environment detects tmux |
| herdr session | `--mux-server`; `HERDR_SESSION`; the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; `default`. With `--pane`, these environment sources apply only when the environment detects herdr |
| Director pane | `--pane`, checked only for existence on the server; else `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr), verified against the caller's process ancestry |

Each failure exits 1 before any write:

| Condition | Error |
|---|---|
| tmux finds no server | `Error: no tmux server found; pass --mux-server <socket path>` |
| The pane is not on the chosen server (a tmux reply with an empty field or another `pane_id`) | `Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane` |
| `--pane` is neither ID form | `Error: --pane <pane> is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)` |
| A shared-server ancestor, without `--pane` | `Error: cafleet fleet create cannot verify this session's pane: a shared <label> server (pid <pid>) runs this command, …` naming both the tmux and the herdr guidance |
| The env pane fails verification, without `--pane` | `Error: cafleet fleet create cannot verify that <VAR>=<pane> is this session's pane: <cause>; …` naming the detected multiplexer's guidance |
| `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr) is unset, without `--pane` | `Error: cafleet fleet create cannot identify its <backend> pane because <VAR> is not set; …` naming the detected multiplexer's guidance |

Each refusal ends `then run cafleet fleet create with --pane <pane-id> --mux-server <server>`. `<cause>` is one of `the pane does not exist on the <backend> server <server>`, `this command runs in a nested PID namespace`, `the process ancestry could not be read: <read error>`, `the process ancestry stopped at pid <pid> before reaching pid 1` or `the pane's process <pane_pid> is not an ancestor of this command`. The guidance names what the user, not the agent, runs in the Director pane:

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
| A member cannot be ended | Exit 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`; earlier members stay deregistered, and a rerun resumes |
| Placement set changed after cleanup began | Exit 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` |

## Messages

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Persist once, then deliver ordinary-recipient previews oldest-first. Self-sends return `delivery.status=not_applicable`, null method/input and reason `self_send`; other root-Director sends use reason `director_stdout`, without multiplexer or pane diagnostics.

Commit succeeds with exit 0 even when notification is queued, held or unavailable. `notification_sent=true` means this invocation submitted the new message; another invocation's submission leaves it false without a diagnostic. `input_id` identifies the new message's nullable preview receipt; `delivery` describes that message even when older ones are retried. Pending previews retry on the next send/wait; runner-queued previews retry automatically.

### Delivery outcome

Failure preserves successful stdout and prints `Message NEW_ID was persisted. ` followed by `message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.` `CAUSE` is `pane input remained busy` for a busy lock, otherwise a runner lock/transport/submission or storage error with known run/input context. Storage failures identify the current candidate or selected wait-rotation entry. An exhausted attempt retains its sent prefix and reports failure with current receipt context.

Receipt retirement failure reports `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE`, retaining `notification_sent=true`. Pre-commit storage failure exits 1. Consume each committed ID once.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns incoming `input_required` records descending by status timestamp/message ID. Unknown/inactive member: `Member ID not found`, exit 1.

### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is integer `1..=1800`, default `300`. Pending messages return immediately; otherwise poll each second, retrying at most one other ordinary recipient per interval and each pending message once per invocation. It never ACKs.

Result fields are `member_id`, `outcome`, `messages`, `notification_errors`; outcome is `messages` with full records or `timeout` with an empty array. Diagnostics contain `member_id`, `message_id`, `error`, ordered by message ID. Both outcomes exit 0; deleted subject/fleet or corrupt/unreadable state exits 1. Deadline allows one in-flight attempt/finalization of at most six seconds.

### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS] [--codex-thread THREAD]`

Streams an active member's pending inbox, requiring a live root Director with concrete placement and no multiplexer. Unknown/inactive startup: `Member ID not found`, exit 1. Optional integer `1..=1800` lease is monotonic from startup; absence runs until closure/cancellation/failure.

Flush UTF-8 JSONL records: `ready` with `member_id,fleet_id` before replay; `message` containing the complete message record; `closed` with `member_id,reason`. There is no text mode or `--json`. Idle checks emit nothing. Closure reasons are `timeout`, `member_inactive`, `fleet_deleted`, and `session_exited` in forwarding mode only; deletion wins simultaneous lifecycle closure. Normal closure exits 0, operational/output failures exit 1, usage errors exit 2; cancellation may omit `closed`.

Order records by creation/message ID; suppress already-emitted IDs per process. Listening changes no consumption/notification state and retries no pane delivery. Restart replays pending IDs; concurrent listeners are independent. Reconcile stale/replayed envelopes through poll/show JSON, record deferred actions and ACK consumption once. Preserve incomplete JSONL across native reads.

`--codex-thread THREAD` forwards records into a Codex thread and keeps the record, ordering, replay, lease and idle contracts:

- Stdout carries only `ready` and the final `closed` record, written right before exit with every reason, `session_exited` included.
- Each `message` record, and each `closed` record with reason `timeout`, `member_inactive` or `fleet_deleted`, is forwarded in order through `codex queue --thread THREAD --message '[cafleet listen record] <compact record JSON>'`, one at a time with a 10 s deadline each. `ready` and `closed` `session_exited` are never forwarded.
- A `codex queue` that exits non-zero with a stderr line containing `queue cannot contain more than` is a full-queue refusal: the listener keeps the record, waits 1 s, checks the pinned session and retries it, so later records keep their order and none is dropped. The pinned session's exit during the wait ends the listener with `session_exited`.
- A forwarded text over 65,536 bytes becomes `[cafleet listen record] {"event":"message_ref","message_id":<id>,"from_member_id":<id>,"bytes":<n>} — read it with: cafleet message show <id> --json`, where `bytes` is the length of the text it replaces.
- At startup the listener pins its nearest ancestor whose argv[0] basename is exactly `codex`. When that process exits (a reused pid counts as exited), stdout gets `closed` `session_exited` and the listener exits 0.

| Condition | Result |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| No Codex ancestor at startup | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| `codex queue` exits non-zero, other than a full-queue refusal, while the pinned session is alive | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, closed notice, exit 1 |
| Full-queue refusals for one record reach 600 s | stderr `codex thread <THREAD> queue stayed full for 600 s`, closed notice, exit 1 |
| `codex queue` exceeds 10 s while the pinned session is alive | killed; stderr `codex queue for thread <THREAD> did not finish within 10 s`, closed notice, exit 1 |
| `codex queue` exits non-zero or exceeds 10 s after the pinned session exited | stdout `closed` `session_exited`, exit 0, no closed notice |
| Any other failure after `ready` | its usual diagnostic, closed notice, exit 1 |

The closed notice is one best-effort `codex queue` of `[cafleet listen closed] exit 1: <stderr diagnostic>`, bounded by 10 s and not retried on a full queue. When it also fails, stderr adds `failed to forward the closed notice to codex thread <THREAD>: <cause>` and the exit stays 1.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` changes `input_required` to `completed`, stamping consumption independently of assignment progress.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns its full record.

## Push notifications

Ordinary concrete panes receive previews; Directors and self-sends use broker reception. Profile previews are `[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.` Delivery fallback leads with Escape and appends ` [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.`

Stored text stays exact. Previews fold newlines to `⏎`, tabs to `␉`, other C0/DEL controls to `\xNN`. At most 4000 Unicode scalars appear inline; longer text becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.` independently of the 200-codepoint text-output limit. Delivery method selects the clause before writing. Preview leaves consumption `input_required`.

### Wrapped delivery methods

Validate identity, ownership, reachability, storage and receipt history first; failure or uncertain prior input sends no fallback bytes. Unknown detection alone permits fallback.

An open/candidate interaction (`permission_request`, `question`, `unknown_interaction`, including obscured) or earlier queued input defers with zero bytes, including Escape. A verified empty composer with a measured plan receives literal input, 500 ms settle and the measured submit key without Escape. Terminal replies leave submission valid; human input stops it with `uncertain_cause=manual_input`.

A measured plan with an unverified/nonempty composer queues under `composer_busy`, then drains when empty; three minutes waiting solely on that condition permits fallback `idle_timeout`. Interaction/earlier-input time is excluded. Working members with empty composers receive profile input queued natively behind their turn. Explicit interruption uses `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Escape`.

Absent plan/profile/version probe uses fallback `submission_plan_unavailable`: Escape, settle, literal (`! ` prefix for exec), settle, Enter. Recheck interactions before each stage; permission answers require their tested plan.

### Runner queue

Accepted deferred input returns durable `ready`, `deferred_at`, `defer_reason` and its input ID; it outlives the invocation and drains automatically. Inspect `cafleet input show INPUT_ID --json`. The Director receives one `input_delivery_result` for its first submitted/not-submitted/uncertain outcome.

Zero-byte reasons are `pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted`, `message_consumed`, `target_inactive`, `target_changed`, `runner_stopped` or `runner_lost`.

### Held delivery

Degraded/recovering/lost observation, unavailable ownership/storage or uncertain receipts/reachability hold input without queue acceptance or writing. Pending/changed placements also hold. Held/queued previews leave notification unset and grant no permission.

### Accepted trade-offs

Delivery fallback can interrupt work, dismiss undetected prompts or combine unsent drafts. A host may need multiple Escapes; submission remains valid when the host queues instead of interrupting. Resume and dismissal clauses are conditional.

Prompt/exec carry no clause, so inspect open prompts first. Repeated previews are consumed by ID; concurrent sends may interrupt earlier turns, and interruption can end shell tools. The CLI defers termination during pane input. OpenCode captures may show only a status bar; inspect retained sessions for omitted output and broker poll/ACK for consumption.

## Member commands

Pane operations require active placement; opaque pane IDs may be pending. Individual deletion protects the Director.

### `member create`

`cafleet member create --fleet-id ID --name NAME --description DESCRIPTION [--coding-agent AGENT] [--model MODEL] [--permission-mode MODE] [--event-evidence POLICY] [--event-redaction-file PATH] [--effort LEVEL] (PROMPT | --file PATH) [--json]`

Registers and spawns, resolving the acting Director from the fleet. Omitted backend inherits its placement backend. Description is one sentence; only Claude and pi display the name as pane title. Model forwards to the native CLI; OpenCode requires `<provider-id>/<model-id>`, Hermes passes any value through, and pi passes any value without a `:<thinking>` suffix through. Hermes argv: `hermes chat --cli --source tool --in <cwd>`, then optional `--model MODEL` and `--reasoning LEVEL`, then `-q PROMPT`; it never carries `--provider`, `--resume`, `--continue` or `--create-if-missing`. pi argv: `pi --cafleet-gate MODE --no-approve --tui-mode fullscreen --name NAME`, then optional `--model MODEL` and `--thinking LEVEL`, then `-- PROMPT`, as a fresh session.

Evidence defaults to `metadata`; `redacted` requires an absolute readable UTF-8 redaction file. Invalid policy/configuration combinations exit 2 before registration.

Permission choices: `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)`. Invalid selection: `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, exit 2. Mode classes (documentation only; argv and run records are unchanged): `auto` (native automatic review or a fixed allow/deny policy) is claude `auto`, `dontAsk`, `bypassPermissions`; codex `approve-for-me`, `never`; opencode `cafleet`; hermes `smart`; pi `cafleet`. `manual` (native interactive approval) is claude `default`, `manual`, `acceptEdits`, `plan`; codex `on-request`; opencode `cafleet-ask`; pi `cafleet-ask`. pi choices emit `--cafleet-gate MODE` and select the CAFleet member gate's mode. The Hermes choice emits no permission argv (`permission_argv_json` `[]`) and is recorded only; the operator configuration's `approvals.mode` governs the session.

Effort validation precedes side effects. An unlisted level exits 2 with `Error: --effort for the BACKEND backend must be one of LEVELS (got '<value>').`, LEVELS exactly as listed; any `--effort` for opencode exits 2 with `Error: opencode does not support reasoning effort.`

| Backend | LEVELS |
|---|---|
| claude | low, medium, high, xhigh, max |
| codex | minimal, low, medium, high, xhigh |
| hermes | none, minimal, low, medium, high, xhigh, max, ultra |
| pi | off, minimal, low, medium, high, xhigh, max |

Hermes forwards the level as `--reasoning`, and pi as `--thinking`.

Invalid OpenCode model: `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`, exit 2. A pi model whose text after its last `:` is a thinking level (`off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`): `Error: --model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '<value>').`, exit 2; other text after a `:`, such as `:free`, passes through.

A pi prompt body beginning with `@` exits 2 before registration with `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`; start the prompt with text.

The split reference (session, window, Director pane) and the multiplexer server come from the Director's recorded placement; the new placement copies its server.

Hermes availability preflight runs `hermes config path` under the member's `HERMES_HOME` after the config-dir and PATH checks. Each failure exits 1 before registration:

| Condition | Error |
|---|---|
| `hermes config path` fails or times out | `Error: cannot read hermes configuration through 'hermes config path': <cause>` |
| `hermes config path` prints no absolute file path | `Error: cannot read hermes configuration through 'hermes config path': printed no absolute file path: <stdout>` |

The member environment forwards a set `HERMES_HOME`.

A pi spawn requires the CAFleet pi extension `<pi base>/extensions/cafleet.ts` in every launch mode: `Error: cannot read the CAFleet pi extension at <path>: <cause>; run 'cafleet setup --coding-agent pi' first`, exit 1 before registration. The member environment forwards a set `PI_CODING_AGENT_DIR`. A gated member whose extension cannot register `--cafleet-gate` exits at startup with `Error: Unknown option: --cafleet-gate`, reported as `runner_exited`.

Substitute exactly `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}` and `{database_url}` into literal lines; `{database_url}` renders `default` for the default database and `sqlite:///<canonical path>` otherwise. A non-default database with a prompt lacking `{database_url}` exits 1 before registration: `Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url`. Double literal braces `{{`/`}}`; malformed/unknown placeholders exit 2 before registration (§ Error messages).

Probe native `--version`; select the greatest bundled `min_version` not exceeding it by SemVer precedence (ignore build metadata). Newer versions reuse the newest profile. Below-minimum versions fail before registration/pane creation: `unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION`, exit 1. Hermes takes its version from a first line matching `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$` (`Hermes Agent vunknown (2026.9.24)` records `2026.9.24`); pi prints the bare token (`1.5.0` records `1.5.0`). Failed probes, including a first line with zero or several SemVer tokens or a non-matching Hermes line, warn `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and continue with unknown coverage/fallback. Creation waits for native-child startup.

Failure compensation shuts down owned runner/child, closes pane, removes descriptor, marks run `failed` and deregisters/removes placement, retaining the primary exit category. Cleanup diagnostics include `cleanup failed for pane <id>: <detail>`, `owned child cleanup remains unconfirmed`, descriptor/run/deregistration errors and `pane ID unknown; pane cleanup unconfirmed`. Inspect unconfirmed resources.

### `member delete`

`cafleet member delete MEMBER_ID [--json]` shuts down the member's runner, closes the locked pane and deregisters. Post-close database errors retain the cause and closed-pane evidence; layout-restoration diagnostics preserve successful deregistration. Director protection uses § Error messages.

Before runner shutdown, a `hermes` run's session work is stopped through native pane input (`Ctrl+C` when busy, then `/stop` and `/exit`) and its registered background processes are verified as stopped, within a 20-second settlement budget per Hermes member (also in `fleet delete`) that extends the 5-second deletion deadline.

| Outcome | Result |
|---|---|
| Busy pane input | Exit 1, member unchanged: `member ID's pane input remained busy for 5 seconds; member was not deleted.` |
| Unverified Hermes stop | Exit 1, member kept, a retry re-runs the settlement: `Error: hermes work for member ID could not be verified as stopped: CAUSE; member was not deleted.` |
| Hermes client already exited | Deletion completes with `warning: hermes client for member ID had already exited; its session work may still be running in the Hermes gateway` |

### `member list`

`cafleet member list FLEET_ID [--json]` lists active Director and ordinary records (§ Member list output).

### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active member, including Director, without a multiplexer; includes `kind`, placement and runtime.

### `member prompt`

`cafleet member prompt MEMBER_ID --from-member-id DIRECTOR_ID TEXT [--json]` requires a nonblank control-free line: `text must be one non-empty line without control characters.`, exit 2.

Director target, before I/O: `cannot prompt the Director's own pane`, exit 1. Runner delivery uses profile/fallback/queue, and exit 0 establishes submission/queue acceptance. Result fields: `member_id,pane_id,text,input_id,delivery`.

| Condition | Error body (exit 1) |
|---|---|
| Pre-write error, including a held runner reply | `prompt was not dispatched to member ID: CAUSE` |
| Partial uncertainty | `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

### `member exec`

`cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID --file PATH [--json]`; `--file -` reads UTF-8 stdin, stripping at most one final LF/CRLF. Exactly one source; a nonblank line excludes CR/LF, NUL, tabs and other C0/DEL: `command must be one non-empty line without control characters.`, exit 2 before I/O. Multiline tasks use one-line script invocations.

The runner submits `! ` plus command through profile/fallback/queue. The host owns cwd, environment, permissions, output and exit. Exit 0 establishes submission/queue acceptance; JSON contains `member_id,pane_id,coding_agent,command,dispatched,input_id,delivery`, with `dispatched=false` when queued.

| Condition | Error body (exit 1) |
|---|---|
| Root Director, before I/O | `cannot exec in the Director's own pane` |
| Runner refusal before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

A failing first write may partially type; failure after Enter may have executed. Inspect native output and reconcile receipts before a newly authorized retry. Receipts retain operation key/digest/state, without command text. After completion, explicitly prompt continuation if the host has not resumed.

### `member press`

`cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID KEY... [--json]` requests 1–64 ordered keys, each at most 32 UTF-8 bytes, with no implicit Escape/Enter or shell submission. For punctuation use `cafleet member press 42 --from-member-id 1 -- '-' '+' ';'`. Wrapped members receive manual keys through the outer pane. Exit 0 establishes backend acknowledgment, independently of native approval/effects; fields are `member_id,pane_id,keys,status`, with `status=acknowledged`.

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

Create/show include `member_id,name,registered_at,placement,runtime`; show also includes `description,status,kind`. Member creation records every ordinary member's run; `runtime` is null for the root Director. Runtime fields describe identity (`run_id,resolved_mode,owner_epoch`), state (`status,last_health_at,coverage,current_interaction_id`), ownership (`owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`), `unresolved_input_ids` and nullable `effective_policy`.

Unresolved IDs include queued `ready`, `in_progress`, `accepted`, `uncertain`, ascending; accepted needs settlement, in-progress/uncertain needs Director reconciliation. Registry reads establish no live health. Effective policy contains `automatic_review=available|unavailable`, nullable `observed_mode`, `rule_id`, nullable integer `event_id`, `observed_at`; null means no tested observation.

Uncertain errors exit 1 with known receipt/context, including `native command delivery to member 110 is uncertain: manual_input (method=profile)` and `input_id=INPUT_ID`.

### Delivery reporting

`delivery` contains `status,method,reason`. Method is `profile`, `fallback` or null before selection. Held prompt/exec exits 1; held send retains commit. `uncertain` requires reconciliation; `not_submitted` certifies zero-byte cancellation; `not_applicable` is broker-only.

Queue reasons are `interaction_open,interaction_candidate,composer_busy,earlier_input`; fallback reasons `submission_plan_unavailable,idle_timeout`. Held send reasons: `observer_unavailable` (no observation), `notification_held` (placement/run/receipt hold), `notification_failed` (lock/transport/storage/submission failure). Queued previews stay unnotified until submission or ACK settlement; later results appear in receipts/events without changing original command output.

### Runtime events

Notifications contain the full event JSON, observed member as sender and Director as recipient. Verify association through event show. Kinds: `coverage_unknown,interaction_opened,interaction_obscured,interaction_closed,automatic_refusal_observed,automatic_review_unavailable,input_delivery_result,runner_degraded,runner_lost,runner_exited`.

Identity fields: `event_id,run_id,event_seq,member_id,fleet_id,director_member_id`; context includes `kind,interaction_id,classification,observed_at,source,decision_outcome,evidence`, closure `close_reason` or lifecycle `cause`. Required nullable `director_action` contains `kind,instruction,inspect_argv`: review kinds are `review_permission_request,review_question,review_automatic_refusal,review_automatic_review_unavailable`; `review_question` accompanies every question `interaction_opened`, with the same `permission inspect` argv.

Evidence contains `backend,native_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,excerpt,evidence_digest`. Positive `operation_rows` is null exactly with null operation; above one requires confirmation against excerpt. Metadata nulls screen-derived operation/choices/excerpt. Excerpts/payloads are bounded at 8/16 KiB.

Both automatic-review kinds carry `null` `interaction_id` and `classification`, `source` `runner_screen`, `decision_outcome` `"unknown"` and `director_action.inspect_argv` `["cafleet","event","show","<EVENT_ID>","--json"]`:

| Field | `automatic_refusal_observed` | `automatic_review_unavailable` |
|---|---|---|
| `evidence` | `rule_id` names the refusal rule; `operation`, `operation_rows` and `excerpt` follow the run's evidence policy | `rule_id` names the notice rule; `operation` and `operation_rows` are null |
| `director_action.kind` | `review_automatic_refusal` | `review_automatic_review_unavailable` |

Automatic-review events create no interaction/input eligibility; permission commands reject their association before writing.

Delivery results contain `input_id,message_id,delivery,stage`, null interaction/classification/action, source `runner_lifecycle`, outcome `unknown` and receipt-derived `cause={"kind":UNCERTAIN_CAUSE}` only when recorded. Stages are `zero_bytes,partial,submitted,unknown`. First outcome publishes once, without literal payload.

### `event show`

`cafleet event show EVENT_ID [--json]` returns `event,notification_state,message_id,archive_reason,run,interaction`. Event is full versioned payload, run current summary. Nullable interaction contains `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

### `input show` and `input reconcile`

`cafleet input show INPUT_ID [--json]`; `cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]`. Result `input` contains `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response,delivery,deferred_at,defer_reason,uncertain_cause`.

Kinds: `preview,prompt,exec,permission_response`; states: `ready,in_progress,accepted,uncertain,settled`. Ready certifies zero bytes. Stored dispositions: `submitted,not_submitted,abandoned_uncertain`; CLI: `submitted,not-submitted,abandoned-uncertain`. Preview keys: `preview:<run_id>:<message_id>`. Receipts hold digests, excluding literals. Stable receipt IDs.

Projection: deferred ready → queued with original reason; other ready → held/`zero_bytes_ready`; in-progress, uncertain or abandoned-uncertain → uncertain; accepted/submitted → submitted; not-submitted → not_submitted with cancellation reason/null method or reconciled selection. Unrecorded methods stay null with `method_unrecorded` outside ready. Permission responses have null delivery/deferred fields and dedicated validated request/decision/audit metadata digests.

`uncertain_cause` records the transition to uncertainty and survives reconciliation; it is null for input that never became uncertain and for `in_progress` input left by runner loss.

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

At settle checks the first matching cause among the first five wins. Director reconciliation requires nonempty evidence, synchronization with current owner and proof no active writer remains; an unreachable owner leaves uncertainty. A deregistered member or deleted fleet has no live runner: reconciliation fails with `Error: receipt reconciliation requires the member's live runner; the input stays uncertain`. Repeating a settled ordinary disposition returns original evidence; conflicting disposition fails unchanged. Settlement writes no input and establishes no native result.

Nullable `permission_response` audit contains `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,choice_id,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id,decided_at`; exactly one of `decision` and `choice_id` is null, and the text line `Permission decision: … decision=DECISION choice=CHOICE …` renders it as `-`. Settlement grants no answer and releases no possibly transmitted claim. IDs are positive; malformed input exits 2, absent/corrupt rows fail explicitly.

### `permission inspect`

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads pinned-runner state without child bytes, receipt, audit or claim. Missing interaction fails before event/run/database checks; stale/unavailable owner fails operationally.

Result `request` contains identity `event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq`; context `classification,visibility,backend,native_version`; capability `answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq`; `evidence,missing_evidence`.

Visibility: `visible,obscured,not_visible,unknown`; capability: `tested,unknown`. Answerable requests have null `capability_reason`; others return a bounded explanation. For a permission request, supported decisions are the tested subset of `allow,deny` and `supported_choices` is `[]`. For a question, `supported_decisions` is `[]` and `supported_choices` lists one `{choice_id,label,scope}` per tested choice; a question without one returns capability `unknown` with reason `question has no tested answer`. Blocked/unsupported returns empty arrays/null token. Otherwise token is lowercase 64-hex, request-bound. For a permission request, missing facts are `operation,arguments,cwd,environment,targets,effects,choice_scope`; operation requires nonblank/unredacted/untruncated evidence; obtain necessary missing facts before deciding. A question returns `missing_evidence` `[]`. Text `answers=` lists the supported decisions or choice ids.

### `permission respond`

`cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]`

Pass exactly one of `--decision` (permission request) and `--choice` (question, one of the inspected `supported_choices` ids); both, neither or a choice id outside 1–64 bytes of lowercase hyphen-separated words exit 2. Require positive IDs/epoch, exact token, nonblank key and evidence references of 1–2048 UTF-8 bytes with non-whitespace and no C0/DEL. References identify host evidence without embedded commands/screens/secrets. `director-auto` requires current host mode `auto`; `user` records a user decision in any mode. Resolve mode independently of member policy; other/unknown modes require user permission. Native/managed rules remain authoritative. Shape/source-mode errors exit 2 before effects. Record operation key before dispatch.

Validate retained keys first: matching metadata returns stored result with zero new bytes; conflicting reuse fails. New keys cannot bypass active/submitted/uncertain/abandoned claims.

Result `response` contains `status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,decision_source,stage,output_seq,reason`, exactly one of `decision` and `choice` non-null; text adds `choice=CHOICE` after `decision=DECISION`. Status `accepted` means submitted, `held/rejected` zero bytes, `uncertain` partial/submitted/unknown. Input/sequence/reason may be null. Acceptance establishes key submission independently of native approval/execution.

Retained accepted/submitted returns accepted/submitted; in-progress/uncertain returns uncertain/unknown; settled not-submitted/abandoned returns rejected/zero_bytes for this retry, preserving original claim; ready takes exact-metadata live validation. Uncertainty reports recorded cause or `prior transmission remains uncertain`.

Accepted/prior submitted exits 0; structured held/rejected/uncertain prints selected response and exits 1: `Error: permission response STATUS for interaction INTERACTION_ID: REASON`, appending ` (input_id=INPUT_ID)` when known. Pre-reply failures retain ordinary categories/context; usage exits 2.

Blocked current requests create no receipt/audit/claim/byte: `Error: runner control: permission request is blocked: REASON`, followed by known run/interaction/key context; capability reasons are `permission input is fenced`, `manual input accounting failed`, `observation degraded: CODE`, `observation transitions pending`, `observation evidence pending`, `observation update pending`, `observation incomplete`, `terminal input interrupted`, `outer terminal closed`, `native output closed`, `terminal output still arriving`, `terminal reply pending`, `permission request is not visible`, or `permission request is not the current interaction`, matching inspection.

A wrong-kind answer is a structured zero-byte `rejected` response (exit 1): `choice answers apply only to question interactions` (`--choice` on a non-question), `decision answers apply only to permission requests` (`--decision` on a question), or `choice CHOICE_ID has no tested answer for this question`.

### `member recover`

`cafleet member recover MEMBER_ID --from-member-id DIRECTOR_ID [--json]` reconciles active/deregistered recorded runs without starting agents; absent run fails before mutation.

Exceptional form: `cafleet member recover MEMBER_ID --from-member-id DIRECTOR_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. All four options are required, with positive inspected snapshot IDs/epoch and nonempty operator attestation that owner/startup processes ended. Invalid combinations exit 2; changed ownership rejects and positive live evidence protects the owner.

Result fields: `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcomes: `recovered,recovering,live_or_unreachable,operator_reconciliation_required`. Counts cover newly committed work; incomplete/unreachable outcomes remain distinct from recovery.

## `setup` and `doctor`

`cafleet setup [--coding-agent AGENT...]` initializes the version's database, then installs embedded skills, presets and extensions offline in backend registry order, default every backend. First failure stops installation, exit 1: `failed to install skills into <skills_dir>: <error>`, `failed to install preset into <target>: <error>`, `failed to install extension into <target>: <error>`, `failed to enable hermes plugin cafleet in <config path>: <cause>` or config-path error. One installation record per backend covers its skills, presets and extensions.

Codex receives `<codex home>/rules/cafleet.rules`, allowing every `cafleet` command; OpenCode the `cafleet` and `cafleet-ask` presets (shell rules for the four member broker commands, `"cafleet": "deny"` tool permission) and the plugin `<preset base>/plugins/cafleet.ts`; Hermes the plugin under `<hermes home>/plugins/cafleet/`, enabled with gateway injection in the file `hermes config path` reports, refusing when `plugins.disabled` lists `cafleet`; pi the extension `<pi base>/extensions/cafleet.ts` with its `cafleet.executable` record, without running `pi`. Each plugin and extension runs only the `cafleet` recorded in its executable record by the setup that installed it. Setup writes no Director-only deny, ask or prompt entry. `--coding-agent hermes` requires `hermes` on `PATH`; plain setup without it installs the skills, records the installation and prints `hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes`. `cafleet setup --coding-agent pi` prints:

```
pi: installed cafleet, cafleet-design-doc (v<version>) -> <pi base>/skills
pi: installed extension (v<version>) -> <pi base>/extensions/cafleet.ts
```

`cafleet doctor [--verbose] [--json]` runs every check without aborting. Statuses are `ok`, `info`, `warn` and `error`; doctor exits 1 with `Error: doctor found errors` after printing when any check is `error`, otherwise 0, `warn` included.

| Invocation | stdout |
|---|---|
| `cafleet doctor` | One `<status> <component>: <detail>` summary line per component: `multiplexer`, `database`, each backend, then `members` lines |
| `cafleet doctor --verbose` | One `<status> <name>: <detail>` row per check, no summary |
| `cafleet doctor --json` | `{"summary":[…],"checks":[…]}`; summary entries `component,status,detail` (`members` adds `state,member_ids`), check entries `name,status,detail` (`member_runtime` entries add `member_id,state`) |

The `multiplexer` check reports the backend and server (`ok` `<backend> server=<server>`), and the `multiplexer.caller_pane` check that follows it reports the `fleet create` pane verification without `--pane` (§ Fleet commands). Their `multiplexer` summary line folds both:

| Situation | Checks (`multiplexer`, `multiplexer.caller_pane`) | Summary line |
|---|---|---|
| Pane verified | `ok`, `ok` `<VAR>=<pane_id> session=<session> window=<window_id>` | `ok multiplexer: <backend> (session <session>, pane <pane_id>)` |
| Shared-server ancestor | `info` `not checked: a shared <label> server (pid <pid>) supplies the caller environment`, `warn` with the refusal body | `warn multiplexer: <refusal body>` |
| Env pane unset or unverified | `ok`, `warn` with the refusal body | `warn multiplexer: <backend> (server <server>); <refusal body>` |
| Verification or verified-pane context read errored | `ok`, `error` with the error | `error multiplexer: <error>` |
| Detection, server or availability failed | `error` with the cause, `info` `unavailable: the multiplexer is unavailable` | `error multiplexer: <cause>` |

The refusal body is the `fleet create` refusal without its `cafleet fleet create ` prefix, and a `warn` keeps the exit code at 0.

A backend line reads `ok <agent>: ready`, `info <agent>: not installed (<agent> is not on PATH)`, `error <agent>: <configuration cause>`, or its problem phrases joined by `; ` at the worst status, an identical phrase listed once (`info hermes: no Hermes home at <base> — run 'cafleet setup --coding-agent hermes'`), ending ` — run 'cafleet setup --coding-agent <agent>'` when setup fixes one, for example `warn codex: assets not installed at /home/user/.codex — run 'cafleet setup --coding-agent codex'`. No installation recorded is `warn`.

Extension checks precede `<agent>.installation`: `opencode.extension` follows the OpenCode preset checks, `hermes.extension` follows `hermes.executable`, `hermes.plugin_enabled` follows `hermes.extension`, and `pi.extension` follows `pi.executable`. Each is `info` `unavailable: executable is absent` or `unavailable: configuration failed: <cause>` when its backend is unusable. When the resolved Hermes base does not exist, doctor runs no `hermes` process and both Hermes rows are `info` `unavailable: no Hermes home at <base>`.

| Check | `ok` detail | `error` details |
|---|---|---|
| `opencode.extension` | The plugin path | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent opencode'`; `<record path>: <cause>; run 'cafleet setup --coding-agent opencode'` |
| `hermes.extension` | The plugin directory | `cannot read hermes configuration through 'hermes config path': <cause>`; `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent hermes'`; `<record path>: <cause>; run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | `cafleet enabled with gateway injection` | `cannot read hermes configuration through 'hermes config get': <cause>`; `hermes plugins.disabled lists cafleet; remove it, then run 'cafleet setup --coding-agent hermes'`; `hermes plugin cafleet is not enabled: <key>, …; run 'cafleet setup --coding-agent hermes'` |
| `pi.extension` | The extension path | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent pi'`; `<record path>: <cause>; run 'cafleet setup --coding-agent pi'` |

An extension check is `ok` only when every embedded file equals its installed copy and the executable record names an existing executable file. Their summary phrases are `extension <path> is missing or differs from the embedded copy` and `extension executable record <path> is unusable: <cause>` (setup fix), the two `cannot read hermes configuration through …` details above and `hermes plugins.disabled lists cafleet` (no setup fix), `hermes plugin cafleet is not enabled: <key>, …` (setup fix) and `info` `no Hermes home at <base>` (setup fix).

`members` lines group member IDs by state, one line per occupied state: `<status> members: <N> <label>: <ID ranges>[ — run '<command>'[ or '<command>']]`. Three or more consecutive IDs collapse to `a-b`; `ID` in the command stays literal, and a `member recover` command fills in `--from-member-id` with the fleet Director ID when every listed member shares one Director (else `<director-id>` stays literal). With no live member the line is `ok members: none running`; a database that is not current gives `info members: not checked (database unavailable)`; a failed run query gives `error members: could not be checked: <cause>`.

| Label | Status | Next command |
|---|---|---|
| `running` | `ok` | — |
| `not responding` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| `lost runtime` | `warn` | `cafleet member recover ID --from-member-id <director-id>` or `cafleet member delete ID` |
| `interrupted launch` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| `agent exited` | `warn` | `cafleet member delete ID` |
| `deleted with leftover runtime` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| `could not be checked` | `error` | `cafleet doctor --verbose` |
| `not checked (time limit)` | `info` | `cafleet member show ID` |

Ended runs of deregistered members are hidden from the summary and appear in `--verbose` as `info member_runtime.<run_id>: member <id> ended: run <status>, cleanup complete`. Other verbose runtime rows read `<status> member_runtime.<run_id>: member <id> <label>: <phrase>; …`, ending ` — run '<command>'` with the literal member ID, for example `warn member_runtime.9: member 9 lost runtime: owner lock is free; runtime did not answer the health probe: <cause> — run 'cafleet member recover 9 --from-member-id 2' or 'cafleet member delete 9'`.

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
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID | `Error: fleet '<fleet-id>' not found.` |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` |
| `member create` | The fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` |
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
