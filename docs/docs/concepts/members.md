# Members

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` atomically registers the current Director with its pane and multiplexer server, and every later command drives the multiplexer through that recorded placement. When CAFleet cannot confirm the Director's pane, registration needs one step from you ([Director pane verification](#director-pane-verification)). `member create` inherits that Director's backend unless overridden, validates model/effort, native policy, evidence policy and prompt identity before registration, then starts the member's owned PTY runner. A failed creation rolls back or deregisters its pending member and reports the original cause with any cleanup error ([creation failure compensation](https://github.com/himkt/cafleet/blob/main/SPEC.md#creation-failure-compensation)).

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

A fleet spreads its member panes across several windows, a tmux window or a herdr tab. The Director keeps the left half of its own window alone, and members stack in that window's right column. When that column is full, `member create` opens an overflow window named `cafleet-<fleet_id>-<ordinal>`, such as `cafleet-3-1`, in the background without taking focus. An overflow window fills its left column top to bottom, then its right column. After every member creation and deletion, the panes of each column in that window have equal heights.

| Rule | Effect |
|---|---|
| Column capacity | A column holds one member for every 30 rows of window height, up to 8. The capacity is read when a member is created, so resizing your terminal affects only later members |
| Small terminals | A Director window shorter than 30 rows holds no members, so every member opens in an overflow window. An overflow window always takes at least one member per column |
| Free slots | A slot freed by `member delete` is reused first, so a newer member can land in the Director window while older members sit in overflow windows |
| Existing members | A member never moves to another window or column after creation |
| Window numbers | Overflow numbers only increase within a fleet, and a failed creation can skip one. An overflow window closes when its last member leaves; the Director window stays open |

Each member's pane carries the member name: herdr labels the pane, and tmux sets its pane title, which shows when `pane-border-status` is on and which a coding agent may retitle. `member list` shows each member's `window` and `pane_id`, and `member create` and `member show` print `window=<name>`. The Director window's name is the one it had when `fleet create` ran.

Member creations and deletions in one fleet wait for each other, up to 60 seconds, so concurrent calls never pick the same slot. If you close an overflow window by hand while its members stay registered, the next `member create` fails and names the window and its members; run `cafleet member delete MEMBER_ID` for each, then retry. The [fleet window layout contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#fleet-window-layout) defines capacity, slot selection, naming and the layout lock.

## Assignment and receiving

Messages are point-to-point: send one to each selected recipient. Runner-managed members receive previews through their runner, which delivers, queues or holds each one as [Deliver generated input](#deliver-generated-input) describes. The root Director receives replies through its listener; [run a fleet](../mixed-backend-team.md#manual-lifecycle) explains reception and consumption.

## Command routing

A member runs commands its host permits and routes a required denied command to the Director, who authorizes `member exec`: it visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, continue a member the host has not resumed with an explicit `member prompt`. Inspect a failed or uncertain dispatch and its durable input receipt before any newly authorized retry. A held permission request follows [Member permission events](#member-permission-events).

The [native dispatch contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

Commands that write into a member's live native session or runtime record — explicit pane input, permission answers, input reconciliation and run recovery — refuse unless `--from-member-id` is the root Director of the subject's fleet, and a member routes them like any denied command. The guard is cooperative, not a security boundary: a member that copies the Director's ID passes it, so member authority still comes from the member role and the Director's assignment. The [Director-only command guard](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-only-command-guard) lists the commands and the exact error.

## Inspection and recovery

Check roster and pane evidence with `member list` and `member capture`, which returns the last 20 lines unless `--lines` asks for more. Judge completion from messages and artifacts: an empty composer does not establish completion, and a timeout alone establishes neither failure nor completion. Replace an unusable member only when its replacement is authorized, carrying its assignment, messages and artifacts into the replacement prompt.

`cafleet doctor` prints one summary line per component. Its `members` lines group members by state with compact ID ranges, such as `warn members: 20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID --from-member-id 2' or 'cafleet member delete ID'`, and each actionable state names the command to run next. `--verbose` prints one row per check, including each member run's probe observations, and `--json` prints both the summary and the checks. The [doctor contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#cafleet-doctor--environment-diagnosis) defines the states, statuses and exit rule.

`member show` reads recorded runtime state without a socket probe; every ordinary member has a runtime record, and the root Director has none ([runtime presentation](https://github.com/himkt/cafleet/blob/main/SPEC.md#runtime-presentation)). `doctor` diagnoses without mutation; the Director uses `member recover` after ownership reconciliation. Recovery preserves history, sends no answer keys and launches no replacement agent. The [member runtime contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#69-member-pty-runtime) defines identity fencing, bounded shutdown, evidence privacy and acceptance.

## Teardown

Runner-managed deletion coordinates shutdown and event recovery under each member's pane-input lock, explicitly retaining unconfirmed child cleanup and recoverable artifacts. Member deletion kills the owned pane and deregisters the member. Fleet deletion ends every remaining ordinary member the same way, reporting each pane outcome, then deregisters the root Director and the fleet, leaving the Director's pane open; a member that cannot be ended stops it with the fleet still registered, and a rerun resumes. Both serialize placement changes with pane input and with the fleet's other creations and deletions, and fail without registry changes when pane input stays busy for five seconds or the fleet's pane layout for 60 seconds. Member deletion refuses the root Director.

## Member permission events

CAFleet runs new members inside an owned PTY runner, which observes native terminal interactions independently of the member model and persists classified events in the Director's inbox. The Director resolves each notification under its own session's approval mode, deciding itself in verified auto mode and obtaining the user's decision in other or unknown modes: for a permission it evaluates the original operation and allows or denies it; for a question, such as Claude's auto-mode dialogs, it picks a choice CAFleet can answer — a tested dismissal choice — or asks the user to answer on the member's surface when none is offered. A request-bound response sends the selected native choice; the Director then checks actual results and resumed work.

### Choose a native policy

Spawn members with the omitted `--permission-mode`, which selects the backend's default mode, and pass `--permission-mode MODE` only when the user asks for another mode for that member; [Launch modes](coding-agents.md#launch-modes) classifies each backend's modes `auto` or `manual`, and the [policy contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#launch-policy-and-capability-registry) owns choices, defaults and coverage. Member launch mode is independent of the Director's decision policy. A `manual` mode shows a native permission form for each operation its rules do not settle, including every shell command outside the member broker commands under OpenCode or pi `cafleet-ask` instead of a denial (pi still blocks a command holding a control character without a form) and, on Claude, the forms for reading, editing or creating a file outside the working directories and the three-choice Bash form shown when auto mode cannot be offered. On OpenCode and pi the default `cafleet-ask` opens a form for every such command, so a Director outside verified auto mode asks you about each one, and a form too wide for the member's pane is answered on that pane. An `auto` mode can still show one, following the same request flow, when its reviewer escalates an operation, as Hermes's guardian does, and on Claude before a file tool reads outside the working directories, until `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`.

Launchability, permission detection, composer input and permission answers have separate capability declarations; a tested detector supplies no answer capability by itself. Each bundled profile declares its minimum native version, and a member runs under the profile with the greatest minimum at or below its installed version, so later native releases reuse it without a warning. Creation refuses a version below the backend's first profile. A failed version probe or a backend with no profile permits manual terminal use and emits a coverage notification, and generated input reaches the member through delivery fallback. Native and managed rules remain authoritative.

### Automatic review observations

The runner reports an automatic refusal and unavailable automatic review as their own events, which the Director handles without a native request to answer; the [automatic review events contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#automatic-review-events) owns their payloads and the [Director consumption contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) their handling. `member show` displays the run's recorded effective policy, including whether automatic review was observed available.

### Consume a runtime event

The Director reads the full inbox record, verifies its durable event association with `event show EVENT_ID --json`, records the unfinished action and ACKs it. It then runs `permission inspect` for a fresh snapshot and answer capability, establishes the operation's arguments, cwd, environment, targets and effects from authorized evidence, and resolves its own session's effective mode: in verified auto mode it decides allow/deny or picks a listed question choice itself; otherwise it obtains your request-specific permission or choice. It answers with a request-bound `permission respond` (`--decision` or `--choice`), or presents that command for you to run; without answer capability the request stays on the member's native surface. Afterward it inspects the durable receipt, actual output and resumed work — a submitted hotkey, closed form or ACK proves no execution result. Input queued during the request is delivered once it closes and reported as `input_delivery_result` events, with no further send or wait. The [Director consumption contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) and the [permission command contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#permission-commands) own the exact steps, flags and outputs.

The listener produces stdout records under the [Director receive lifecycle](../mixed-backend-team.md#receive-results). Host interruption, goal activation and model consumption timing remain host-owned.

### Preserve a native request

While an interaction is open or a candidate, the runner queues previews and explicit prompt/exec with zero bytes, Escape included, and human terminal input continues independently of event storage. A response answers only the current visible request or question through an independently tested answer profile, with a single-request allow/deny or a tested question dismissal; the runner injects no arbitrary keys and changes no persistent or session-wide permission. A random request token fences each answer: a changed request, human input (keystrokes, pastes and mouse input, not the terminal's automatic replies), obscuring, degraded observation or ownership loss invalidates it, so reinspect and reconcile a stale command. A durable input receipt records submission separately from authorization and native outcome; inspect `input show INPUT_ID --json` after uncertainty before any `input reconcile`. Metadata notifications omit command details, and redacted evidence can also suppress them, so obtain current facts on an authorized decision surface; a separately dispatched native command is a new action, not an answer. The [request-bound response contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#request-bound-permission-responses) owns privacy, replay protection and the screen-interface race limit.

### Deliver generated input

The runner chooses how each preview, `member prompt` and `member exec` reaches a wrapped member:

| Member state | Delivery | Reported method |
|---|---|---|
| A detected interaction is open or a candidate, or earlier input is queued | Queued with no bytes; the runner retries after the interaction closes or the earlier input finishes | `null` while queued |
| Empty composer and a measured submission plan for the operation | Text, a short settle, then the measured submit key, without Escape | `profile` |
| Measured submission plan, but a draft or a composer state the runner cannot classify | Queued with no bytes until the composer empties, then delivered by profile; delivered by fallback after 3 minutes | `null` while queued, then `profile` or `fallback` |
| No submission plan | Escape, settle, text, settle, Enter | `fallback` |

A preview is held, not queued, while an earlier receipt is uncertain or the runner's observer, ownership or storage is unavailable; a later `message send` or finite receive retries it.

Human input during the settle before the submit key stops the remaining stages and makes the delivery uncertain; the outer terminal's automatic replies and focus reports pass to the native CLI and leave profile delivery intact.

Delivery fallback can interrupt work, mix text with a draft, or dismiss a native prompt the runner did not detect, so its previews carry the Escape-dismissal clause. Command output reports each delivery's status, method and reason; `input show INPUT_ID --json` reports a queued input's eventual result and, for an uncertain delivery, its recorded `uncertain_cause`, such as `manual_input` or `short_write`, so the Director can attribute the outcome before reconciling it. The [delivery reporting contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#delivery-reporting) owns exact statuses and reasons.
