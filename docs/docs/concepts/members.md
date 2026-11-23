# Members

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` overwrites the current Director window/tab name with `cafleet-<fleet_id>-0`, verifies the name and pane location, then atomically registers the Director with that placement and multiplexer server. Every later command drives the multiplexer through the recorded placement. When CAFleet cannot confirm the Director's pane, registration needs one step from you ([Director pane verification](#director-pane-verification)). A naming or verification failure rolls back uncommitted registration and leaves the Director pane running; an applied name remains, and the error identifies the attempted target for inspection. `member create` inherits that Director's backend unless overridden, validates model/effort, native policy and prompt identity before registration, then starts the member's owned PTY runner. A failed member creation rolls back or deregisters its pending member and reports the original cause with any cleanup error ([creation failure compensation](https://github.com/himkt/cafleet/blob/main/SPEC.md#creation-failure-compensation)).

Members send ready, then poll their inbox; the Director can assign a member once its ready arrives.

### Director pane verification

Some coding agents run their tool commands in a long-lived shared server: a Codex Director on Codex 0.167.0 or later through its shared daemon, and a Hermes Director through its gateway. Such a server can hand `cafleet` the multiplexer variables of another pane, even one in another multiplexer. Without `--pane`, `fleet create` therefore registers the pane only after confirming that the `cafleet` command descends from that pane's own process. Otherwise it registers nothing and names a command for you to run in the Director pane.

The Director then asks you to run that command and reply with the pane ID and server it shows, and reruns `fleet create` with `--pane <pane-id> --mux-server <server>`. Run the command yourself in the Director pane; the agent's own shell cannot report that pane reliably.

| Multiplexer | What you do in the Director pane | Reply with |
|---|---|---|
| tmux | Press the tmux prefix key, type `:display-message -d 0 "#{pane_id} #{socket_path}"` and press Enter; the status line shows the pane ID, then the server socket | The pane ID (e.g. `%12`) and the socket path (e.g. `/tmp/tmux-1000/default`) |
| herdr | With the Director pane focused, type `! env -u HERDR_PANE_ID herdr --session <session> pane current --current` into the Director's composer. `<session>` is the herdr session you attached, `default` unless herdr was started with `--session <name>`, and `herdr session list` shows the running sessions | `result.pane.pane_id` (e.g. `w4:p3N`) and the session name |

The pane ID's form names its multiplexer (`%N` for tmux, `<workspace>:<pane>` for herdr), so the rerun works even when the Director's environment names the other multiplexer. If the rerun reports that the pane was not found, the Director asks you once more. `cafleet doctor` reports the same check on its `multiplexer` line as `warn`, which leaves its exit status unchanged. The [caller pane verification contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#caller-pane-verification) lists every refusal and its exact message.

## Pane layout

A fleet spreads its member panes across several windows, a tmux window or a herdr tab. Creation names the Director window `cafleet-<fleet_id>-0`, such as `cafleet-3-0`. The Director keeps the left half of that window alone, and members stack in its right column. When that column is full, `member create` opens an overflow window named `cafleet-<fleet_id>-<ordinal>`, starting at `cafleet-3-1` for that fleet, in the background without taking focus. An overflow window fills its left column top to bottom, then its right column. After every member creation and deletion, the panes of each column in that window have equal heights.

| Rule | Effect |
|---|---|
| Column capacity | A column holds one member for every 15 rows of window height, up to 8. The capacity is read when a member is created, so resizing your terminal affects only later members |
| Small terminals | A Director window shorter than 15 rows holds no members, so every member opens in an overflow window. An overflow window always takes at least one member per column |
| Free slots | A slot freed by `member delete` is reused first, so a newer member can land in the Director window while older members sit in overflow windows |
| Existing members | A member never moves to another window or column after creation |
| Window numbers | Overflow numbers only increase within a fleet, and a failed creation can skip one. An overflow window closes when its last member leaves; the Director window stays open |

Each ordinary member's pane carries `<member ID> - <member name>`, such as `12 - drafter`, so the pane shows the ID the `member` commands take: herdr labels the pane, and tmux sets its pane title, which shows when `pane-border-status` is on and which a coding agent may retitle. Ordinary pane naming is best effort; Director window naming is required for successful fleet registration. `member list` shows each member's `window` and `pane_id`, and `member create` and `member show` print `window=<name>`. These names are placement snapshots: existing fleets retain their recorded names, and later user renames leave the registry unchanged. Fleet deletion leaves the Director window's current name in place.

Member creations and deletions in one fleet wait for each other, up to 60 seconds, so concurrent calls never pick the same slot. If you close an overflow window by hand while its members stay registered, the next `member create` fails and names the window and its members; run `cafleet member delete MEMBER_ID` for each, then retry. The [fleet window layout contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#fleet-window-layout) defines capacity, slot selection, naming and the layout lock.

## Assignment and receiving

Messages are point-to-point: send one to each selected recipient. Runner-managed members receive previews through their runner, which delivers, queues or holds each one as [Deliver generated input](#deliver-generated-input) describes. The root Director receives replies as previews that the fleet's member runners type into its pane; [run a fleet](../mixed-backend-team.md#receive-results) explains delivery and consumption.

## Command routing

A member runs commands its host permits and routes a required denied command to the Director, who authorizes `member exec`: it visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, continue a member the host has not resumed with an explicit `member prompt`. Inspect a failed or uncertain dispatch and its durable input receipt before any newly authorized retry. A held permission request follows [Member permission events](#member-permission-events).

The [native dispatch contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

Commands that write into a member's live native session or runtime record — explicit pane input, permission answers, input reconciliation and run recovery — refuse unless `--from-member-id` is the root Director of the subject's fleet, and a member routes them like any denied command. The guard is cooperative, not a security boundary: a member that copies the Director's ID passes it, so member authority still comes from the member role and the Director's assignment. The [Director-only command guard](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-only-command-guard) lists the commands and the exact error.

## Inspection and recovery

Check roster and pane evidence with `member list` and `member capture`, which returns the last 20 lines unless `--lines` asks for more. Judge completion from messages and artifacts: an empty composer does not establish completion, and a timeout alone establishes neither failure nor completion. Replace an unusable member only when its replacement is authorized, carrying its assignment, messages and artifacts into the replacement prompt.

`cafleet doctor` prints one `STATUS NAME: DETAIL` row per check of the multiplexer, the database and each backend's installed assets, and `--json` prints the same checks as a JSON array. It also prints a `member.<id>.runtime` row for every active ordinary member whose latest run has not ended, and probes the runtime when that run is `running` or `degraded`; a run in another state gets an `info` row without a probe. A member row is `error` when the probe fails or the member's observation gap has stayed open for more than 30 seconds, and its detail then names the `member show` command to inspect with and the `member delete` command to run if the member does not recover. An `error` member row makes doctor exit 1, like every other `error` row. The [doctor contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#cafleet-doctor--environment-diagnosis) defines the checks, statuses and exit rule.

`member show` reads recorded runtime state and probes a running member's runtime for its live observation state; every ordinary member has a runtime record, and the root Director has none. The text line ends with `observation=<state>`, plus `degraded` while the runtime cannot store its events, and `--json` adds the probe outcome, the observation details and the runtime's five most recent stored diagnostics ([runtime presentation](https://github.com/himkt/cafleet/blob/main/SPEC.md#runtime-presentation)). A member whose runtime is lost or unusable is deleted and re-created; deletion settles its runtime records, preserves history and sends no answer keys. The [member runtime contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#69-member-pty-runtime) defines identity fencing, bounded shutdown, evidence privacy and acceptance.

## Teardown

Runner-managed deletion coordinates shutdown and event recovery under each member's pane-input lock, explicitly retaining unconfirmed child cleanup and recoverable artifacts. A runner that already exited without recording it is not contacted: deletion settles its runtime as lost and completes, and a runner that exits mid-deletion is completed by a rerun. Member deletion kills the owned pane and deregisters the member. Fleet deletion ends every remaining ordinary member the same way, reporting each pane outcome, then deregisters the root Director and the fleet, leaving the Director's pane open; a member that cannot be ended stops it with the fleet still registered, and a rerun resumes. Both serialize placement changes with pane input and with the fleet's other creations and deletions, and fail without registry changes when pane input stays busy for five seconds or the fleet's pane layout for 60 seconds. Member deletion refuses the root Director.

## Member permission events

CAFleet runs new members inside an owned PTY runner, which observes native terminal interactions independently of the member model and persists classified events in the Director's inbox. The Director resolves each notification under its own session's approval mode, deciding itself in verified auto mode and obtaining the user's decision in other or unknown modes: for a permission it evaluates the original operation and allows or denies it; for a question, such as Claude's auto-mode dialogs, it picks a choice CAFleet can answer — a tested dismissal choice — or asks the user to answer on the member's surface when none is offered. A request-bound response sends the selected native choice; the Director then checks actual results and resumed work.

### Choose a native policy

Spawn members with the omitted `--permission-mode`, which selects the backend's default mode, and pass `--permission-mode MODE` only when the user asks for another mode for that member; [Launch modes](coding-agents.md#launch-modes) classifies each backend's modes `auto` or `manual`, and the [policy contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#launch-policy-and-capability-registry) owns choices, defaults and coverage. Member launch mode is independent of the Director's decision policy. A `manual` mode shows a native permission form for each operation its rules do not settle, including every shell command outside the member broker commands under OpenCode or pi `cafleet-ask` instead of a denial (pi still blocks a command holding a control character without a form) and, on Claude, the forms for reading, editing or creating a file outside the working directories and the three-choice Bash form shown when auto mode cannot be offered. On OpenCode and pi the default `cafleet-ask` opens a form for every such command, so a Director outside verified auto mode asks you about each one, and a form too wide for the member's pane is answered on that pane. An `auto` mode can still show one, following the same request flow, when its reviewer escalates an operation, as Hermes's guardian does, and on Claude before a file tool reads outside the working directories, until `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`.

Launchability, permission detection, composer input and permission answers have separate capability declarations; a tested detector supplies no answer capability by itself. Each backend has one bundled profile, which every member of that backend uses whatever its installed native version. A native version that renders a form differently is not recognized, and that form is answered on the member's surface. A failed version probe only leaves the member's recorded version empty. A backend with no profile permits manual terminal use and emits a coverage notification, and generated input reaches the member through delivery fallback. Native and managed rules remain authoritative.

### Consume a runtime event

The Director's poll consumes the inbox record; the Director records the unfinished action at once, then verifies its durable event association with `event show EVENT_ID --json`. It then runs `permission inspect` for a fresh snapshot and answer capability, establishes the operation's arguments, cwd, environment, targets and effects from authorized evidence, and resolves its own session's effective mode: in verified auto mode it decides allow/deny or picks a listed question choice itself; otherwise it obtains your request-specific permission or choice. It answers with a request-bound `permission respond` (`--decision` or `--choice`), or presents that command for you to run; without answer capability the request stays on the member's native surface. Afterward it inspects the durable receipt, actual output and resumed work — a submitted hotkey, closed form or consumed message proves no execution result. Input queued during the request is delivered once it closes and reported as `input_delivery_result` events, with no further send. The [Director consumption contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) and the [permission command contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#permission-commands) own the exact steps, flags and outputs.

Runtime events reach the Director through Director delivery under the [Director receive lifecycle](../mixed-backend-team.md#receive-results). Host interruption, goal activation and model consumption timing remain host-owned.

### Preserve a native request

While an interaction is open or a candidate, the runner queues explicit prompt/exec and leaves previews pending, with zero bytes, Escape included, and human terminal input continues independently of event storage. A response answers only the current visible request or question through an independently tested answer profile, with a single-request allow/deny or a tested question dismissal; the runner injects no arbitrary keys and changes no persistent or session-wide permission. A random request token fences each answer: a changed request, human input (keystrokes, pastes and mouse input, not the terminal's automatic replies), obscuring, degraded observation or ownership loss invalidates it, so reinspect before another command. The runner accepts one answer per request and keeps no record of it; after an uncertain answer, confirm the native result from the pane. Notifications carry no command details, so obtain current facts on an authorized decision surface; a separately dispatched native command is a new action, not an answer. The [request-bound response contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#request-bound-permission-responses) owns privacy, the one-answer rule and the screen-interface race limit.

### Deliver generated input

`cafleet message send` only stores a message; the recipient's own runner types its preview when the member is ready, oldest message first, with no further command from anyone. Queued prompts and native commands go before any preview. The runner chooses how each preview, `member prompt` and `member exec` reaches a wrapped member:

| Member state | Delivery | Reported method |
|---|---|---|
| A detected interaction is open or a candidate, or earlier input is queued | Prompts and native commands queue with no bytes and previews stay pending; the runner retries after the interaction closes or the earlier input finishes | `null` while queued |
| Empty composer and a measured submission plan for the operation | Text, a short settle, then the measured submit key, without Escape | `profile` |
| Measured submission plan, but a draft or a composer state the runner cannot classify | Waits until the composer empties, then delivered by profile | `null` while queued, then `profile` |
| No submission plan | Escape, settle, text, settle, Enter | `fallback` |

A preview stays pending while the runner's observer, ownership or storage is unavailable, and the runner retries it. A member with no live runner keeps its messages pending until it polls them.

Only a native interaction that appears while CAFleet types stops delivery. Typing, pasting, mouse input and the terminal's automatic replies leave the delivery running, so your text can mix with the typed line; the runner then sees an unsubmitted result after the submit key. A stopped or unconfirmed preview is typed again once the composer is verified empty, so it can appear twice with the same message ID. A stopped or unconfirmed prompt or native command records an uncertain result that needs a deliberate retry, and until the Director reconciles it no queued input and no preview reaches the member.

Later input waits while text remains in the composer; clearing or submitting it lets a fresh empty-composer observation resume delivery. A composer that shows only a native suggestion, the dimmed or grey hint text some CLIs draw, counts as empty. An OpenCode composer in shell mode never counts as empty, so input waits until the member returns to normal mode. A message the member's poll has consumed is never typed.

### Stalled queues

When a queued prompt or native command starts no delivery attempt within 3 minutes of reaching the front of the queue, or a message preview waits 3 minutes behind text in the composer, the Director receives one `input_delivery_stalled` event for it. Time an open interaction blocks the input does not count, because the interaction already has its own event. The event carries one instruction: the Director captures the pane and submits or clears the composer's text with `member press`, or asks you to clear or submit it on the member's native surface. When queued input or a message preview instead waits 3 minutes behind an earlier prompt or native command whose delivery is uncertain, the Director receives one `input_delivery_stalled` event that names that uncertain input; the Director inspects it and the pane and settles it with `input reconcile`, after which the waiting input resumes. The [stalled-event contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#input-delivery-stalled-event) owns the exact fields.

Long previews use bracketed paste when the backend supports it and has enabled it; otherwise they show a short pointer to the poll that reads the full message. An oversized explicit prompt or native command without a safe delivery mechanism is refused before typing.

Delivery fallback can interrupt work, mix text with a human draft, or dismiss a native prompt the runner did not detect, so its previews carry the Escape-dismissal clause. Command output reports status, method and reason; `input show INPUT_ID --json` gives the eventual result and `stop_cause`. An uncertain prompt or native command needs pane evidence and Director reconciliation. The [delivery reporting contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#delivery-reporting) owns exact statuses and reasons.
