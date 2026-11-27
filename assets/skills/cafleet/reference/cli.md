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
| `message show` | Read one message | `MESSAGE_ID` | yes |

`cafleet --version` prints `cafleet <version>` and exits 0. `cafleet help COMMAND…` equals `cafleet COMMAND… --help`; `cafleet` alone, or a group such as `cafleet fleet` without its subcommand, prints help to stderr and exits 2. A flag given twice on one command (`--json --json`) is a parse error, exit 2; `--opt=value` equals `--opt value`.

## Positional subject ids

Place the complete database-assigned integer subject immediately after its subcommand; related fleet and recipient state comes from that row. An ID may carry a leading `+` or leading zeros (`007` is 7). Missing, malformed or out-of-range IDs exit 2 with parser errors whose first line begins `error: `.

## Director-only command guard

`member prompt`, `member exec`, `member press`, `permission respond` and `input reconcile` require `--from-member-id <director-member-id>` (missing: parse error, exit 2). Before any pane access, each requires its subject's fleet live with that ID as root Director; an unknown subject keeps its not-found error. Otherwise it exits 1 with empty stdout and stderr `Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.`

`permission inspect` also requires the Director's `--from-member-id`; every other command carries no guard. The guard is cooperative, checking only the ID a command carries.

## Database selection

The global `--database-url <url>`, before or after the subcommand, selects the SQLite database. Pass it at most once per command level (`cafleet`, the group, the subcommand); a repeat on one level exits 2, and the deepest level wins. Omitted, the default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db` applies, `<VERSION>` being the package version; without `HOME` it fails with `Error: HOME is not set; cannot derive the default database path` (or `HOME is not valid UTF-8; …`), exit 1. Explicit `sqlite:///<path>` takes the text after `sqlite:///` as a plain file path: `sqlite:///etc/x` is relative to cwd, an absolute path needs four slashes (`sqlite:////var/x.db`), and literal tildes stay unexpanded. Another prefix fails with `Error: database URL must use the sqlite scheme (sqlite:///<path>); got '<url>'`, and an empty path or `:memory:` with `Error: database URL has no file path`, both exit 1. CAFleet reads no `CAFLEET_*` environment variable.

A member whose `DATABASE URL:` names a URL passes `--database-url <url>` on every `cafleet` command; without it, the command reaches the default database and fails with `no cafleet database; run 'cafleet setup'` or matches another fleet's IDs.

Config overrides `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, `HERMES_HOME` (default `~/.hermes`) and `PI_CODING_AGENT_DIR` (default `~/.pi/agent`) must be absolute, `Error: <VAR> must be an absolute path (got '<value>')`; a set override or `HOME` that is not UTF-8 fails with `Error: <VAR> is not valid UTF-8`; both exit 1. Creation forwards only the selected backend's set override and launches the runner with `--database-url sqlite:///<canonical path>`.

## Output shapes

Text output truncates message bodies at 200 codepoints; `--json` returns compact, complete UTF-8. Place `--json` before any `--` separator; commands marked no in the summary, and the root group, reject it.

Message records contain `message_id`, `from_member_id`, `to_member_id`, `created_at`, `status_state`, `status_timestamp` and `text`. Send and show return `message`; poll returns an array of the records it consumed. Fleet creation returns `fleet_id`, `name`, `created_at` and `director`; list returns an array of `fleet_id, name, created_at, director_member_id, member_count` (`member_count` counts active members, Director included), and show returns `fleet_id, name, created_at, deleted_at, director_member_id` with nullable `deleted_at`; deletion returns `fleet_id`, `deregistered_count` and `members`. Member deletion returns `member_id`, `pane_status`; capture returns `member_id`, `pane_id`, `lines`, `content`, `captured_at`, `content_sha256`.

### Member list output

Roster records contain `member_id`, `name`, `kind` (`director` or `member`), `placement`, `last_sent`, `last_recv`, `last_consumed` (the member's last consuming poll) and `idle`; activity may be null.

`placement` keys, in order, are `backend, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side, mux_pane_id, coding_agent, created_at`; `mux_window_id`, `mux_window_name` and `mux_pane_id` are null while creation is pending. Fleet windows are named `cafleet-<fleet_id>-<ordinal>`, ordinal `0` for the Director window; `column_side` is `left` or `right`.

Rows are in ascending `member_id`. Text starts `N members:` (`1 member:` for one row; an empty roster prints only `0 members.`), then rows listing `member_id`, `name`, `kind`, `backend`, `window`, `pane_id` and `idle`; pending placement prints `(pending)`, absent activity `-`:

```text
2 members:
  member_id  name           kind      backend   window         pane_id  idle
  8          director       director  claude    main           %3       12s
  9          drafter        member    claude    cafleet-3-1    %12      5s
```

## `permissions.allow` coverage

`Bash(cafleet message *)` covers broker operations, including `cafleet message poll ID --json`. Claude members receive it at spawn, and setup installs the Codex and OpenCode allow rules. An operator may keep `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` under ask to confirm each Director pane-input dispatch; ask outranks allow.

## Text bodies

Send and create take exactly one positional body or `--file PATH` (`--file -` reads UTF-8 stdin) and preserve nonblank text exactly. Text is blank when every character is Unicode whitespace, the empty text included. Missing or both sources, or a blank positional, exit 2; a blank, unreadable or invalid file or stdin exits 1 (§ Error messages).

## Fleet commands

`cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]` registers the calling Director's pane and multiplexer server as the root Director's placement without spawning; every later command targets that recorded server. Outside tmux and herdr it fails with `cafleet fleet create must be run inside a tmux or herdr session`. Without `--pane`, it refuses a caller under a shared server (`codex app-server`, `hermes gateway run`) and registers `TMUX_PANE` / `HERDR_PANE_ID` only when that pane's process is an ancestor of `cafleet`. With `--pane`, it reads no process ancestry and accepts an environment naming either multiplexer.

| Input | Source, first match wins |
|---|---|
| Backend | The `--pane` form: `%N` (e.g. `%12`) is tmux, `<workspace>:<pane>` (e.g. `w4:p3N`) is herdr; else `TMUX`/`HERDR_ENV` detection |
| tmux server | `--mux-server`; `TMUX` before its last two commas; the default server's `tmux display-message -p '#{socket_path}'`. With `--pane`, `TMUX` sources apply only when the environment detects tmux |
| herdr session | `--mux-server`; `HERDR_SESSION`; the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; `default`. With `--pane`, environment sources apply only when the environment detects herdr |
| Director pane | `--pane`, checked only for existence on the server; else `TMUX_PANE` or `HERDR_PANE_ID`, verified against process ancestry |

Each failure below exits 1 before any write; the last three arise only without `--pane`:

| Condition | Error |
|---|---|
| tmux finds no server | `Error: no tmux server found; pass --mux-server <socket path>` |
| The pane is not on the chosen server | `Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane` |
| `--pane` is neither ID form | `Error: --pane <pane> is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)` |
| A shared-server ancestor | `Error: cafleet fleet create cannot verify this session's pane: a shared <label> server (pid <pid>) runs this command, …` naming both multiplexers' guidance |
| The env pane fails verification | `Error: cafleet fleet create cannot verify that <VAR>=<pane> is this session's pane: <cause>; …` naming the detected multiplexer's guidance |
| `TMUX_PANE` or `HERDR_PANE_ID` is unset | `Error: cafleet fleet create cannot identify its <backend> pane because <VAR> is not set; …` naming the detected multiplexer's guidance |

Each caller-verification refusal ends `then run cafleet fleet create with --pane <pane-id> --mux-server <server>`. `<cause>` is one of `the pane does not exist on the <backend> server <server>`, `this command runs in a nested PID namespace`, `the process ancestry could not be read: <read error>`, `the process ancestry stopped at pid <pid> before reaching pid 1` or `the pane's process <pane_pid> is not an ancestor of this command`. The guidance names what the user, not the agent, runs in the Director pane:

| Multiplexer | Guidance text in the refusal | The user replies with |
|---|---|---|
| tmux | `the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket)` | The pane ID (e.g. `%12`) and the socket path, passed as `--pane` and `--mux-server` |
| herdr | `! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)` | The pane ID (e.g. `w4:p3N`) and the session name, passed as `--pane` and `--mux-server` |

Registration renames the Director's window or tab to `cafleet-<fleet_id>-0`, verifies it, then commits. A naming, verification or registration failure exits 1 with no success payload, even under `--json`, and rolls back; its diagnostic names the attempted IDs and cause and warns that the terminal name may have changed, so inspect with `fleet list` and explicit-server multiplexer commands.

`cafleet fleet list [--json]` reads the registry. `cafleet fleet show FLEET_ID [--json]` includes deleted fleets; unknown IDs give `fleet 'X' not found.`, exit 1.

`cafleet fleet delete FLEET_ID [--json]` deletes active ordinary members through member deletion, then deregisters the remaining members and the fleet, retaining messages and the Director pane. It is idempotent; a rerun resumes partial cleanup.

Each failure exits 1; only a member that cannot be ended leaves earlier members deregistered:

| Outcome | Error |
|---|---|
| Pane input lock busy | `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.` |
| Fleet layout lock busy for 60 seconds | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; fleet was not deleted.` |
| A member cannot be ended | `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`, where `CAUSE` is that member's deletion cause (§ `member delete`) without trailing `.` and `IDS` is the ended IDs joined by `, ` or `none` |
| Placement set changed during cleanup | `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` |

## Messages

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Stores the message once and returns without pane input. An ordinary recipient's runner types pending previews oldest first and retries on its own; without a live runner, messages stay pending. A self-send is stored already notified; a message to the root Director is typed into its pane by a live member runner (§ Director previews). A storage failure before commit exits 1.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns every incoming `input_required` record, newest first, and consumes exactly those in one transaction: each shows `status_state` `completed`, and no later poll returns it; concurrent polls never share a message. An empty inbox prints `[]` (text: `No messages found.`), exit 0. Unknown or inactive member: `Member ID not found`, exit 1, consuming nothing. A failure before commit consumes nothing; when output is lost after commit, `message show MESSAGE_ID --json` reads each message again.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns the full record in any state without changing it; use it to reread a consumed message.

## Push notifications

Profile previews are `[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.` Fallback delivery leads with Escape and appends ` [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.`

Previews fold newlines to `⏎`, tabs to `␉` and other C0/DEL controls to `\xNN`; stored text stays exact. A body over 4000 Unicode scalars becomes `Body is N characters; run cafleet message poll RECIPIENT_ID --json to read it.`, never carrying `--database-url`. A preview leaves the message `input_required`.

### Director previews

Every live member runner types the root Director's pending messages into the Director's pane as profile previews, oldest first across senders, one per message. On each `[cafleet msg …]` preview, run `cafleet message poll <director-id> --json` once; `[]` means an earlier poll already consumed the message.

A preview is typed only while the Director-screen profile verifies the composer empty with no open interaction, as a plain literal with no Escape and no bracketed paste. A body above the Director profile's plain limit arrives as the `poll` pointer. These hold the oldest Director message, and every newer one behind it, with no runtime event:

- a draft or leftover literal in the Director composer, an unrecognized screen or a pane narrower than the profile's layouts;
- an open permission dialog, question or unknown interaction in the Director pane;
- an unavailable Director pane, multiplexer or storage;
- no live member runner in the fleet.

A key typed during a delivery leaves the literal in the composer unsubmitted; clearing or submitting it resumes delivery, and a submitted leftover may repeat a preview, so deduplicate by message ID. A Director backend or multiplexer without Director-screen coverage receives nothing in its pane and reads by manual poll. A delivering member's `cafleet member show MEMBER_ID --json` `diagnostics` show `director delivery: message <id> held: <reason>` after 3 minutes, `director delivery: message <id> uncertain: <cause>` (`storage_unavailable` when storage fails after a confirmed submission), `director delivery: pane <pane-id> unavailable: <cause>` and `director delivery: storage unavailable: <cause>`.

Run a manual `cafleet message poll <director-id> --json` at these points:

| Point | Reason |
|---|---|
| After a `member delete` that leaves the fleet with no active member | No runner delivers the deleted member's last report |
| Before `fleet delete` | Final inbox consumption |
| At each wake's overdue-assignment check | Catches an unexpected exit of the last runner |
| After a Director session restart or context loss | Earlier previews may be missing from the new context |

### Wrapped delivery methods

With a measured plan from its backend profile, the runner types prompt, exec and preview input without Escape; on a verified-composer run, only into a verified empty composer. An open or candidate interaction (`permission_request`, `question`, `unknown_interaction`) or earlier queued input defers input with zero bytes. A nonempty composer queues prompt and exec under `composer_busy`; a human draft blocks previews until cleared or submitted. A working member with an empty composer receives input queued natively behind its turn; interrupt with `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Escape`.

Without a plan or profile, fallback `submission_plan_unavailable` sends Escape, the literal (`! ` prefix for exec) and Enter; it can interrupt work, dismiss undetected prompts or merge with unsent drafts. Prompt and exec carry no clause, so inspect open prompts first.

Over-limit input uses bracketed paste when available; otherwise a preview becomes the `poll` pointer and prompt or exec is refused (`not_submitted/plain_limit_exceeded` when queued).

### Runner queue

Deferred prompt and exec input returns durable `ready` with `deferred_at`, `defer_reason` and its input ID, drains automatically and is inspected with `cafleet input show INPUT_ID --json`. The Director receives one `input_delivery_result` per input's first outcome and per stopped immediate write.

Zero-byte reasons are `pane_input_busy`, `storage_unavailable`, `observer_unavailable`, `delivery_budget_exhausted`, `plain_limit_exceeded`, `target_inactive`, `target_changed`, `runner_stopped` or `runner_lost`.

Queued input precedes previews. A composer showing only a native suggestion counts as empty. An uncertain preview publishes an `input_delivery_result` with null `input_id` and its `message_id` and may be typed again, so recipients deduplicate by message ID. After a prompt or exec records `uncertain`, the runner writes no queued input and no preview until `input reconcile` settles it. Input waiting 3 minutes raises `input_delivery_stalled` ([Runtime events](#runtime-events)).

### Held delivery

Unhealthy observation, unavailable ownership or storage, uncertain reachability and pending or changed placement hold prompt and exec without writing and leave previews pending.

## Member commands

Pane operations require active placement. Individual deletion protects the Director.

### `member create`

`cafleet member create --fleet-id ID --name NAME --description DESCRIPTION [--coding-agent AGENT] [--model MODEL] [--permission-mode MODE] [--effort LEVEL] (PROMPT | --file PATH) [--json]`

Registers and spawns under the fleet's Director; an omitted backend inherits the Director's placement backend. Description is one sentence. Each member pane is named `<member_id> - <name>`, such as `12 - drafter`; `member list` `window` and `pane_id` locate it. `--model` forwards to the native CLI: OpenCode requires `<provider-id>/<model-id>`, Hermes passes any value, and pi any value without a `:<thinking>` suffix. Hermes runs `hermes chat --cli --source tool --in <cwd> [--model MODEL] [--reasoning LEVEL] -q PROMPT`. pi runs a fresh session `pi --cafleet-gate MODE --no-approve --tui-mode fullscreen --name "<member_id> - <name>" [--model MODEL] [--thinking LEVEL] -- PROMPT`.

Permission choices: `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)`. Invalid selection: `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, exit 2. Class `auto` (automatic review or a fixed policy) is claude `auto`, `dontAsk`, `bypassPermissions`; codex `approve-for-me`, `never`; opencode `cafleet`; hermes `smart`; pi `cafleet`. Class `manual` (interactive approval) is claude `default`, `manual`, `acceptEdits`, `plan`; codex `on-request`; opencode `cafleet-ask`; pi `cafleet-ask`. The Hermes choice emits no permission argv (`permission_argv_json` `[]`); the operator's `approvals.mode` governs.

Effort validation precedes side effects. An unlisted level exits 2 with `Error: --effort for the BACKEND backend must be one of LEVELS (got '<value>').`; any `--effort` for opencode exits 2 with `Error: opencode does not support reasoning effort.`

| Backend | LEVELS |
|---|---|
| claude | low, medium, high, xhigh, max |
| codex | minimal, low, medium, high, xhigh |
| hermes | none, minimal, low, medium, high, xhigh, max, ultra |
| pi | off, minimal, low, medium, high, xhigh, max |

Exit 2 errors before registration:

| Condition | Error |
|---|---|
| OpenCode model not `<provider-id>/<model-id>` | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` |
| pi model ending in `:` plus a pi LEVEL (`:free` and other text pass) | `Error: --model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '<value>').` |
| pi prompt beginning with `@` | `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).` |

Substitute exactly `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}` and `{database_url}`; `{database_url}` renders `default` or `sqlite:///<canonical path>`. Double literal braces `{{`/`}}`; malformed or unknown placeholders exit 2 (§ Error messages). A placeholder name is the exact text between the braces, so `{}` and `{ fleet_id }` are unknown. The malformed `<detail>` is `unexpected '{' in placeholder name`, `expected '}' before end of string` or `single '}' encountered outside a placeholder`. A non-default database with a prompt lacking `{database_url}` exits 1 before registration: `Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url`.

Creation waits up to 60 seconds for the fleet's pane layout lock. The member fills the Director window's right column, then overflow windows' columns (one member per 15 rows, up to 8), opening overflow window `cafleet-<fleet_id>-<ordinal>` without taking focus when all are full. Existing members never move.

Text output is `ID NAME backend=AGENT pane=PANE window=WINDOW` plus the runtime line; `PANE` and `WINDOW` print `(pending)` while pending.

Each layout failure below exits 1 with nothing registered; a failed herdr re-parenting move instead lets creation continue with stderr `layout re-parenting incomplete: <cause>`.

| Condition | Error |
|---|---|
| Fleet layout lock busy for 60 seconds | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not created.` |
| An overflow window cannot be read | `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.` |
| The window ordinal changed under the lock | `Error: fleet <fleet-id> window ordinal changed under the layout lock` |
| A malformed tmux `new-window` or herdr `tab create` reply | `Error: unexpected tmux new-window output: <quoted>` or `Error: herdr tab create missing '<key>' field`, then `pane ID unknown; pane cleanup unconfirmed` |

Backend preflights exit 1 before registration:

| Condition | Error |
|---|---|
| `hermes config path` fails or times out under the member's `HERMES_HOME` | `Error: cannot read hermes configuration through 'hermes config path': <cause>` |
| `hermes config path` prints no absolute file path | `Error: cannot read hermes configuration through 'hermes config path': printed no absolute file path: <stdout>` |
| The pi extension `<pi base>/extensions/cafleet.ts` is unreadable | `Error: cannot read the CAFleet pi extension at <path>: <cause>; run 'cafleet setup --coding-agent pi' first` |

A gated pi member whose extension cannot register `--cafleet-gate` exits at startup with `Error: Unknown option: --cafleet-gate`, reported as `runner_exited`.

Creation records the native `--version` as the run's `native_version`; it selects nothing, and every member launches with its backend's bundled profile. A failed probe warns `Warning: native version unavailable for BACKEND: CAUSE.`, records null and continues.

A failed creation shuts down its runner and child, closes the pane, marks the run `failed` and deregisters, keeping the primary exit category. Inspect resources named by `cleanup failed for pane <id>: <detail>`, `owned child cleanup remains unconfirmed` or `pane ID unknown; pane cleanup unconfirmed`.

### `member delete`

`cafleet member delete MEMBER_ID [--json]` shuts down the member's runner, closes its pane and deregisters. Rerun the delete after `member runtime: ECONNREFUSED`. A `hermes` member's session work is first stopped through pane input (`Ctrl+C` when busy, then `/stop` and `/exit`) and its background processes verified stopped, within 20 seconds per Hermes member, also in `fleet delete`. Deleting the Director fails per § Error messages.

The first three outcomes exit 1 and keep the member; retry after inspecting the cause.

| Outcome | Result |
|---|---|
| Busy pane input | `member ID's pane input remained busy for 5 seconds; member was not deleted.` |
| Fleet layout lock busy for 60 seconds | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not deleted.` |
| Unverified Hermes stop | `Error: hermes work for member ID could not be verified as stopped: CAUSE; member was not deleted.`, CAUSE being `the Hermes turn did not stop after Ctrl-C`, `the Hermes composer still holds text after clearing`, `Hermes did not report /stop`, `Hermes printed no exit summary after /exit`, `the Hermes client did not exit after /exit`, `background processes of session <id> are still running: <pids>` or `member run <run-id> has no Hermes session record` |
| Hermes client already exited | Deletion completes with `warning: hermes client for member ID had already exited; its session work may still be running in the Hermes gateway` |

Other deletion causes exit 1 and keep the member; rerun the delete after inspecting with `cafleet doctor`:

| Cause | Meaning |
|---|---|
| `member placement or runtime ownership changed <stage>` | Another process changed the member; `<stage>` is `before cleanup`, `before pane close` or `after pane close` |
| `member <member-id> child cleanup remains unconfirmed: <diagnostics>; inspect with cafleet doctor` | The runner shutdown could not confirm its child stopped |
| `member recovery remains incomplete; inspect with cafleet doctor` | Recovery of a stopped runner did not finish |
| `pane <pane-id> could not be closed: <cause>. Verify with 'cafleet doctor'.` | The multiplexer refused the pane close |
| `pane <pane-id> was closed, but deregister failed: <cause>` | The pane is gone; rerun the delete to deregister |

Text output is `Member deleted.`, `  member_id: <member-id>` and `  pane_id:   <pane-id> (killed)` (or `(pending — no pane)`).

### `member list`

`cafleet member list FLEET_ID [--json]` lists active Director and ordinary records (§ Member list output).

### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active member, Director included, without a multiplexer. Text is `ID NAME STATUS window=WINDOW`, then the runtime line for an ordinary member. For a `running` or `degraded` run it probes runner health (1 s deadline) into the runtime object (§ Runtime presentation); a failed probe still exits 0.

### `member prompt`

`cafleet member prompt MEMBER_ID --from-member-id DIRECTOR_ID TEXT [--json]` validates its line: `text must be one non-empty line without control characters.`, exit 2. The Director's own pane fails before I/O with `cannot prompt the Director's own pane`, exit 1. Exit 0 establishes submission or queue acceptance; fields are `member_id,pane_id,text,input_id,delivery`.

| Condition | Error body (exit 1) |
|---|---|
| Pre-write error, including a held runner reply | `prompt was not dispatched to member ID: CAUSE` |
| Partial uncertainty | `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

Uncertain errors include `input_id=INPUT_ID`; inspect it and the pane before a deliberate retry. Over-limit input without safe framing gets the pre-write error with a CAUSE below (UNIT `bytes` or `characters`).

| Missing delivery mechanism | CAUSE |
|---|---|
| Profile declares plain framing | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the BACKEND profile has no bracketed-paste delivery` |
| Profile declares bracketed paste, child has not enabled it | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the native CLI has not enabled bracketed paste` |

### `member exec`

`cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID (COMMAND | --file PATH) [--json]`; `--file -` reads UTF-8 stdin, stripping at most one final LF/CRLF. A blank line or any C0/DEL control fails with `command must be one non-empty line without control characters.`, exit 2. Run multiline tasks as one-line script invocations.

The runner submits `! ` plus the command; the host owns cwd, environment, permissions, output and exit. Exit 0 establishes submission or queue acceptance; JSON contains `member_id,pane_id,coding_agent,command,dispatched,input_id,delivery`, with `dispatched=false` when queued.

| Condition | Error body (exit 1) |
|---|---|
| Root Director, before I/O | `cannot exec in the Director's own pane` |
| Runner refusal before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

Uncertain errors include `input_id=INPUT_ID`, as in `native command delivery to member 110 is uncertain: write_failed (method=profile)`; the command may have typed partially or executed, so inspect native output and reconcile receipts before a newly authorized retry. Over-limit CAUSE strings match [member prompt](#member-prompt). After completion, explicitly prompt continuation if the host has not resumed.

### `member press`

`cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID KEY... [--json]` requests 1–64 ordered keys, each at most 32 UTF-8 bytes, with no implicit Escape, Enter or shell submission. For punctuation use `cafleet member press 42 --from-member-id 1 -- '-' '+' ';'`. Wrapped members receive keys through the outer pane. Every probe and the write share one five-second deadline. Exit 0 establishes backend acknowledgment, not native effect; fields are `member_id,pane_id,keys,status`, with `status=acknowledged`.

| Condition | Error body (exit 2) |
|---|---|
| Key count outside the domain | `press requires between 1 and 64 keys` |
| A key exceeds its UTF-8 bound | `press key at position N exceeds 32 UTF-8 bytes` |
| A key is outside the common grammar | `invalid press key at position N: JSON_QUOTED_TOKEN` |

Operational failures exit 1 with empty stdout, even under `--json`, printing the primary error and then an effect diagnostic on its own stderr line:

| Observed boundary | Effect diagnostic |
|---|---|
| Before any writing process starts | `keys were not dispatched to member ID.` |
| A writing process fails, times out or returns an unreadable acknowledgment | `key delivery to member ID is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.` |

| Local refusal | Primary error body |
|---|---|
| Root Director target, before I/O | `cannot press keys in the Director's own pane` |
| Input lock remains busy | `member ID's pane input remained busy` |
| Revalidated target changes | `member ID placement changed before key dispatch` |
| Live context differs from placement | `press target context does not match the recorded placement` |

Answer request-bound decisions with `permission respond`.

### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` reads trailing lines (default 20, at least 1) without the input lock; a failure, `--lines 0` included, exits 1 with `Error: capture failed: <cause>`; `--ansi` preserves escapes, otherwise redraws and escapes are cleaned. Pending placement fails. `captured_at` stamps the read; `content_sha256` hashes the emitted `content`.

## Pane input and previews

Writers share one lock per concrete pane; capture is read-only, and human input is independent.

A multiplexer process failure names the backend and the full argv joined by spaces: `<backend> binary not found: <cause>`, `<backend> command was not started: <cause>: <argv>`, `<backend> command timed out at the pane input deadline: <argv>`, or `<backend> command failed: <argv>` followed by a `stderr: <stderr>` line. `fleet create`, `doctor` and `member capture` run their multiplexer commands without a deadline; every other command's are bounded by its operation deadline.

Keys accept printable ASCII U+0021–U+007E, `Space`, `Escape`, `Enter`, `Tab`, `Backspace`, arrows, `F1`–`F12`, `Ctrl+a`–`Ctrl+z` and `Shift+Tab`. Printable case is preserved; names and modifiers are case-insensitive, control letters canonicalize lowercase.

## Member runtime

### Runtime presentation

Create and show include `member_id,name,registered_at,placement,runtime`; show adds `description,status,kind`. `runtime` is null for the root Director. Runtime fields cover identity (`run_id,resolved_mode,owner_epoch`), state (`status,last_health_at,coverage,current_interaction_id`), ownership (`owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`) and `unresolved_input_ids` (ascending `ready`, `in_progress`, `accepted` and `uncertain` inputs; accepted needs settlement, in-progress and uncertain Director reconciliation). Only member show's probe keys establish live health.

Member show appends, after `child_start_identity`:

| Key | Value |
|---|---|
| `observation_probe` | `skipped` (status neither `running` nor `degraded`), `answered` or `failed` |
| `observation` | Ordered `visibility,eligibility,degraded,diagnostic,writer_diagnostic,pending_count,gap_count,gap_open_since`; null when skipped, failed or before child start |
| `observation_error` | The probe failure message when `failed`, otherwise null |
| `diagnostics` | The run's latest 5 runner diagnostics, newest first, each `source,message,first_at,last_at,count` |

`eligibility` is `composer_empty` or `hold:<reason>` (`candidate`, `interaction`, `unknown`, `observer_unavailable`, `gap`). `degraded` true means the runner cannot store its events, named by `writer_diagnostic`, and holds generated input. The text runtime line appends ` observation=STATE` (the eligibility; `pending` without an observation, `unavailable` when failed, `-` when skipped), then ` degraded` when degraded: `Runtime: run=37 mode=auto status=running coverage=tested observation=hold:gap degraded`.

### Delivery reporting

`delivery` contains `status,method,reason`; method is `profile`, `fallback` or null before selection. Held and `uncertain` prompt or exec exit 1, `uncertain` requiring reconciliation; `not_submitted` records cancellation or an evidence-based reconciliation.

Queue reasons are `interaction_open,interaction_candidate,composer_busy,earlier_input,delivery_budget_exhausted`; the fallback reason is `submission_plan_unavailable`. Queued text output ends with `The member's runner will retry it after the blocking work ends; run cafleet input show INPUT_ID --json for its result.` Later results appear in receipts and events, leaving the original command output unchanged.

### Runtime events

Event notifications carry the full event JSON to the Director; verify with `event show`. Kinds: `coverage_unknown,interaction_opened,interaction_closed,input_delivery_result,input_delivery_stalled,runner_degraded,runner_lost,runner_exited`.

Identity fields are `event_id,run_id,event_seq,member_id,fleet_id,director_member_id`; context includes `kind,interaction_id,classification,observed_at,source,decision_outcome,evidence`, plus closure `close_reason` or lifecycle `cause`. `director_action` contains `kind,instruction,inspect_argv`; every permission request carries `review_permission_request` and every question `interaction_opened` carries `review_question`, each with its `permission inspect` argv. Events without a listed action carry null.

Evidence contains `backend,native_version,rule_id,screen_seq,evidence_digest` and no screen text; read the operation and choices from `member capture`. Payloads are bounded at 16 KiB.

Delivery results contain `input_id,message_id,delivery,stage`, source `runner_lifecycle`, null `director_action` and `cause={"kind":STOP_CAUSE}` when a stop cause is recorded. A prompt or exec result has its input ID and null `message_id`; an uncertain preview has null `input_id`, its `message_id` and delivery `{"status":"uncertain","method":METHOD,"reason":null}`. Stages are `zero_bytes,partial,submitted,unknown`; a prefix short of the submit key is `partial`.

`input_delivery_stalled` reports a queued head or preview that waited 3 minutes without an attempt, or input held behind an `uncertain` input. It carries ordered `input_id,message_id,blocked_by_input_id,waiting_since`, exactly one ID set. Its action is `review_stalled_delivery` with `inspect_argv` `["cafleet","member","capture","MEMBER_ID","--json"]` (clear or submit a composer draft), or, when blocked, `reconcile_uncertain_input` with `["cafleet","input","show","BLOCKER_ID","--json"]`.

A `runner_degraded` event with cause `{"kind":"event_rejected","rejected_kind":KIND,"rejected_observed_at":TIMESTAMP,"error":MESSAGE}` replaces an event the runner could not store; a rejected `interaction_opened` leaves the request visible only on the member's pane. Repeated delivery records and stalled heads no longer `ready` are archived (`archive_reason` `duplicate_delivery` or `head_not_ready`) without an inbox message. Every argv appends `--database-url URL` for a non-default database. [Director recovery](../roles/director.md#reconcile-generated-input) owns the procedure.

### `event show`

`cafleet event show EVENT_ID [--json]` returns `event,notification_state,message_id,archive_reason,run,interaction`: the full versioned payload, the run's current summary and nullable interaction `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

### `input show` and `input reconcile`

`cafleet input show INPUT_ID [--json]`; `cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]`. Result `input` contains `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref,delivery,deferred_at,defer_reason,stop_cause`.

Kinds are `prompt,exec`; states `ready,in_progress,accepted,uncertain,settled`, where ready certifies zero bytes. Stored dispositions are `submitted,not_submitted,abandoned_uncertain`; the CLI takes `submitted,not-submitted,abandoned-uncertain`. Receipts hold digests, not literals.

`delivery` reports deferred ready as queued with its reason; other ready as held `zero_bytes_ready`; in-progress, uncertain and abandoned-uncertain as uncertain; accepted and submitted as submitted; not-submitted as not_submitted with its cancellation reason and null method or its reconciled selection, and `method_unrecorded` when no method was recorded. `stop_cause` records why a write ended without confirmed submission; it is set on transition to uncertain, retained through settlement, and otherwise null.

| Cause | Recorded when |
|---|---|
| `interaction_open` | An interaction appeared mid-write |
| `interaction_candidate` | A candidate interaction appeared mid-write |
| `submit_unconfirmed` | Submission stayed unconfirmed; stage `submitted` |
| `deadline_expired` | Time ran out before the submit key |
| `short_write` | A prefix stopped inside a scalar or paste marker, or an answer wrote short |
| `write_failed` | A PTY, checkpoint, close or submit-key write failed |
| `runner_stopped` | Shutdown ended a possibly written attempt |

Reconciling uncertainty requires nonempty evidence, owner synchronization and no active writer; an unreachable owner preserves uncertainty. Missing live placement fails with `Error: receipt reconciliation requires the member's live runner; the input stays uncertain`. Repeating a settled disposition returns the original evidence; a conflicting one fails. Reconciling a queued `ready` input as `not-submitted` cancels it with zero bytes. Settlement writes no input and grants no answer. IDs are positive; malformed input exits 2.

### `permission inspect`

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads pinned-runner state without child bytes; a stale or unavailable owner fails.

Result `request` contains identity `event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq`; context `classification,visibility,backend,native_version`; capability `answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq`; and `evidence,missing_evidence`.

Visibility is `visible,obscured,not_visible,unknown`; capability `tested,unknown`, with null `capability_reason` when answerable and a bounded explanation otherwise. A permission request lists the tested subset of `allow,deny` in `supported_decisions`, `[]` choices, and `missing_evidence` `operation,arguments,cwd,environment,targets,effects,choice_scope`: obtain the facts you need before deciding. A question has `[]` decisions and one `{choice_id,label,scope}` per tested choice, `missing_evidence` `[]`, and capability `unknown` with `question has no tested answer` when none is tested. Blocked or unsupported requests return empty arrays and a null token; otherwise the token is request-bound lowercase 64-hex. Text `answers=` lists the decisions or choice ids.

### `permission respond`

`cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) [--json]`

Pass `--decision` for a permission request or `--choice` with an inspected `supported_choices` id for a question; both, neither, or a choice id outside 1–64 bytes of lowercase hyphen-separated words exits 2, as do non-positive IDs. Resolve the mode independently of member policy; other or unknown modes require user permission. Native and managed rules remain authoritative.

The runner accepts one answer per episode: after its first byte, a later respond returns `rejected/zero_bytes` with `an answer was already sent for this episode`; a zero-byte refusal leaves the episode answerable.

Result `response` contains `status,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,stage,output_seq,reason`, exactly one of `decision` and `choice` non-null; text adds `choice=CHOICE` after `decision=DECISION`. `accepted` means submitted, `held` and `rejected` zero bytes, `uncertain` partial, submitted or unknown, reporting its stop cause or `manual_input` when keyboard or pointer input revoked a written answer. Acceptance establishes key submission, not native approval.

Accepted exits 0; held, rejected or uncertain prints the response and exits 1 with `Error: permission response STATUS for interaction INTERACTION_ID: REASON`.

A blocked request writes no byte: `Error: runner control: permission request is blocked: REASON`, followed by known run and interaction context. REASON, matching inspection, is `permission input is fenced`, `manual input accounting failed`, `observation degraded: CODE`, `observation transitions pending`, `observation update pending`, `observation incomplete`, `terminal input interrupted`, `outer terminal closed`, `native output closed`, `terminal output still arriving`, `terminal reply pending`, `permission request is not visible` or `permission request is not the current interaction`.

A wrong-kind answer is a zero-byte `rejected` response (exit 1): `choice answers apply only to question interactions`, `decision answers apply only to permission requests` or `choice CHOICE_ID has no tested answer for this question`.

## `setup` and `doctor`

`cafleet setup [--coding-agent AGENT...]` initializes the version's database, then installs embedded skills, presets and extensions offline for each named backend, default every backend. The first failure exits 1: `failed to install skills into <skills_dir>: <error>`, `failed to install preset into <target>: <error>`, `failed to install extension into <target>: <error>` or a config-path error.

Setup installs Codex `<codex home>/rules/cafleet.rules`, allowing every `cafleet` command; the OpenCode `cafleet` and `cafleet-ask` presets (shell rules for the four member broker commands, `"cafleet": "deny"` tool permission) and plugin `<preset base>/plugins/cafleet.ts`; and the pi extension `<pi base>/extensions/cafleet.ts` with its `cafleet.executable` record. Each plugin and extension runs only the `cafleet` its record names. Setup runs no backend command; Claude and Hermes print only the skills line. `cafleet setup --coding-agent pi` prints:

```
pi: installed cafleet-design-doc, cafleet (v<version>) -> <pi base>/skills
pi: installed extension (v<version>) -> <pi base>/extensions/cafleet.ts
```

`cafleet doctor [--json]` runs every check, printing `<status> <name>: <detail>` rows or a JSON array of `name,status,detail` objects. Statuses are `ok`, `info`, `warn` and `error`; any `error` makes doctor exit 1 with `Error: doctor found errors` after printing.

When `database` is `ok`, one `member.<member_id>.runtime` row follows per active ordinary member with a non-terminal run, in member-ID order, probing `running` and `degraded` runs within 1 s; values follow § Runtime presentation. The first matching row applies:

| Situation | Status | Detail |
|---|---|---|
| Run `starting`, `prepared` or `recovering` | `info` | `run=<run_id> status=<status>: not probed` |
| Probe failed | `error` | `run=<run_id> health probe failed: <cause>; <hint>` |
| Answered without an observation | `info` | `run=<run_id> observation=pending` |
| Observation gap open over 30 s | `error` | `run=<run_id> observation=<eligibility> diagnostic=<code> gap_open_since=<ts>; <hint>` |
| Otherwise degraded | `warn` | `run=<run_id> observation=<eligibility> diagnostic=<code>` |
| Otherwise | `ok` | `run=<run_id> observation=<eligibility>` |

`<hint>` is `inspect with 'cafleet member show <member_id> --json'; if it does not recover, run 'cafleet member delete <member_id>' and create a replacement member`, each quoted command appending ` --database-url URL` for a non-default database.

`multiplexer` reports `ok` `<backend> server=<server>`; the following `multiplexer.caller_pane` reports the `fleet create` pane verification without `--pane` (§ Fleet commands):

| Situation | `multiplexer` | `multiplexer.caller_pane` |
|---|---|---|
| Pane verified | `ok` | `ok` `<VAR>=<pane_id> session=<session> window=<window_id>` |
| Shared-server ancestor | `info` `not checked: a shared <label> server (pid <pid>) supplies the caller environment` | `warn` with the refusal body |
| Env pane unset or unverified | `ok` | `warn` with the refusal body |
| Verification or context read errored | `ok` | `error` with the error |
| Detection, server or availability failed | `error` with the cause | `info` `unavailable: the multiplexer is unavailable` |

The refusal body omits the `cafleet fleet create ` prefix.

Each backend reports `<agent>.configuration`, `<agent>.executable`, its preset and extension checks, then `<agent>.skills`. An absent executable is `info` `<binary> is optional and absent from PATH`, and its later checks are `info` `unavailable: executable is absent`. `<agent>.skills` byte-compares installed skill files with the embedded copies: missing skills are `warn` `skills not installed at <skills dir>; run 'cafleet setup --coding-agent <agent>'`; a differing or unreadable file is `error` `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'`.

An extension check is `info` `unavailable: configuration failed: <cause>` when its backend configuration fails. It is `ok` only when every embedded file equals its installed copy and the executable record names an existing executable.

`opencode.extension` and `pi.extension` report `ok` with the installed path, or `error` `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'` or `<record path>: <cause>; run 'cafleet setup --coding-agent <agent>'`.

## Error messages

Application errors print to stderr prefixed `Error: `, even with `--json`; structured permission responses are the exception. Usage, parser, model, effort and line-validation errors exit 2; application, database and transport errors exit 1. Each command section owns its command-specific errors; the rows below exit 1 unless marked `(exit 2)`.

| Command | Situation | Error message |
|---|---|---|
| Any but `setup` and `doctor` | No database | `no cafleet database; run 'cafleet setup'` |
| Any but `setup` and `doctor` | Incompatible database file, including another version's | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` |
| Any with output | Writing stdout fails other than by broken pipe | `Error: failed to write output: <cause>` |
| `member create` | The backend binary is missing | `Error: binary <name> not found on PATH` |
| `member create` | The selected opencode preset is unreadable | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` |
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID | `Error: fleet '<fleet-id>' not found.` |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` |
| `member create` | The fleet's root Director is inactive | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` (exit 2) |
| `member delete` | Against the root Director | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` |
| `member capture` / `member prompt` / `member exec` / `member press` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&#124;press&gt;.</code> |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member | `Error: Member <member-id> not found` |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` |

Text-body errors of `message send` and `member create`:

| Situation | Error message |
|---|---|
| Positional body blank | `Error: text may not be empty.` (exit 2) |
| `--file <path>` to a blank file | `Error: --file <path>: file is empty.` |
| `--file -` with blank stdin | `Error: --file -: stdin is empty.` |
| `--file <path>` to a missing path or a directory | `Error: --file <path>: file does not exist or is not a regular file. (<io error>)` |
| `--file <path>` that cannot be read otherwise | `Error: --file <path>: file is not readable. (<io error>)` |
| `--file <path>` or `--file -` to invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` (`--file -: …` for stdin) |
| Reading stdin for `--file -` fails | `Error: --file -: <io error>` |
