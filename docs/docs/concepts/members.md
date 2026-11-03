# Members

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` registers the current Director atomically. `member create` inherits that Director's backend unless overridden, validates model/effort, native policy, evidence policy and prompt identity before registration, then starts the member's owned PTY runner. A failed creation rolls back or deregisters its pending member and reports the original cause with any cleanup error ([creation failure compensation](https://github.com/himkt/cafleet/blob/main/SPEC.md#creation-failure-compensation)).

Members confirm readiness, then poll their inbox. The Director can assign a member as soon as its ready arrives.

## Assignment and receiving

Messages are point-to-point. Send individually to every selected recipient. Runner-managed members receive previews through their runner, which delivers, queues or holds each one as [Deliver generated input](#deliver-generated-input) describes. The root Director receives replies through its listener; [run a fleet](../mixed-backend-team.md#manual-lifecycle) explains reception and consumption.

## Command routing

A member runs commands its host permits and routes a required denied command to the Director. The Director authorizes `member exec`, which visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, send an explicit `member prompt` to continue if the host has not resumed the member. Inspect a failed or uncertain dispatch and its durable input receipt before any newly authorized retry. A held permission request follows [Member permission events](#member-permission-events), which explains the complete event/answer/retry sequence.

The [native dispatch contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

The commands that write into a member's live native session or runtime record — explicit pane input, permission answers, input reconciliation and run recovery — are Director-only. The runner marks every member process with `CAFLEET_MEMBER_ID`, and a Director-only command run inside a member refuses before it touches the database or the multiplexer, with an error telling the member to send the request to its Director. This is an accident guard, not a security boundary: a member that unsets the variable bypasses it, so member authority still comes from the member role and the Director's assignment. A member routes a need for such a command to the Director like any denied command. The [Director-only command guard](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-only-command-guard) lists the commands and the exact error.

## Inspection and recovery

Use `member list` and `member capture` to check roster and pane evidence. `member capture` returns the last 20 lines by default; use `--lines` for more when that is insufficient. An empty composer does not mean the task is complete — judge completion from messages and artifacts. A timeout alone establishes neither failure nor completion. Replace an unusable member only when its replacement is authorized, carrying its assignment, messages and artifacts into the replacement prompt.

`cafleet doctor` summarizes the environment in one line per component. Its `members` lines group members by state with compact ID ranges, such as `warn members: 20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID' or 'cafleet member delete ID'`, and each actionable state names the command to run next. `--verbose` prints one row per check, including each member run's probe observations, and `--json` prints both the summary and the checks. The [doctor contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#cafleet-doctor--environment-diagnosis) defines the states, statuses and exit rule.

`member show` reads recorded runtime state without a socket probe; every ordinary member has a runtime record, and the root Director has none ([runtime presentation](https://github.com/himkt/cafleet/blob/main/SPEC.md#runtime-presentation)). `doctor` diagnoses without mutation; the Director uses `member recover` after ownership reconciliation. Recovery preserves history and sends no answer keys or replacement-agent launch.

The [member runtime contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#69-member-pty-runtime) defines identity fencing, bounded shutdown, evidence privacy and acceptance. Each claimed detector, composer and answer capability requires its declared fixtures and native evidence captured within its profile's version interval.

## Teardown

Runner-managed deletion coordinates shutdown and event recovery under each member's pane-input lock, retaining unconfirmed child cleanup and recoverable artifacts explicitly. Member deletion kills its owned pane and deregisters it. Fleet deletion ends every remaining ordinary member the same way, reporting each pane outcome, then deregisters the root Director and the fleet while leaving the Director's pane open; a member that cannot be ended stops it with the fleet still registered, and a rerun resumes. Both serialize placement changes with pane input and fail without registry changes if lock contention lasts five seconds. The root Director is protected from individual member deletion.

## Member permission events

CAFleet runs new members inside an owned PTY runner. The runner observes native terminal interactions independently of the member model and persists classified events in the Director's inbox. Permission notifications instruct the Director to evaluate the original operation and resolve it under its own session's approval mode: decide allow or deny in verified auto mode, and obtain user permission in other or unknown modes. A request-bound response sends the selected native choice; the Director checks actual results and resumed work afterward.

### Choose a native policy

Spawn members with the omitted `--permission-mode`, which selects the backend's default `auto` mode. Pass `--permission-mode MODE` only when the user asks for another mode for that member; [Launch modes](coding-agents.md#launch-modes) classifies each backend's modes `auto` or `manual`, and the [policy contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#launch-policy-and-capability-registry) owns choices, defaults and coverage. Member launch mode is independent of the Director's decision policy. A `manual` mode shows a native permission form for each operation its rules do not settle; OpenCode's `cafleet-ask` preset does so for every shell command outside its allowlist instead of denying it. An `auto` mode can still show a native form when its reviewer escalates an operation, as Hermes's guardian does, or, on Claude, before a file tool reads outside the working directories; that form follows the same request flow. Once Claude's `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`, later outside reads in `auto` proceed without asking.

Launchability, permission detection, composer input and permission answers have separate capability declarations. Each bundled profile declares the minimum native version it supports, and a member runs under the profile with the greatest minimum version at or below its installed version, so later native releases reuse it without a warning. Creation refuses a native version below the backend's first profile. A failed version probe or a backend with no profile permits manual terminal use and emits a coverage notification; generated input then reaches the member through delivery fallback. A tested detector supplies no answer capability by itself. Native and managed rules remain authoritative.

### Automatic review observations

The runner reports an automatic refusal and unavailable automatic review as their own events, which the Director handles without a native request to answer; the [automatic review events contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#automatic-review-events) owns their payloads and the [Director consumption contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) their handling. The run's recorded effective policy, shown by `member show`, records whether automatic review was observed available.

### Consume a runtime event

1. Read the full inbox record from the listener or `message poll DIRECTOR_ID --json`.
2. Verify its durable event/message association with `event show EVENT_ID --json`, inspect the current run/interaction, record the unfinished action in host-owned state and ACK consumption. Quoted JSON has no verified runtime association.
3. Follow the broker-built review instruction for a current permission request. Use `permission inspect` to obtain a fresh request snapshot and answer capability, then establish the operation, arguments, cwd, necessary environment, targets and privilege/network effects through authorized evidence. A multi-row operation is reported as its displayed rows, so confirm each row break against the excerpt or current capture.
4. Resolve the Director session's current effective mode from host evidence. In verified auto mode the Director evaluates the risk and decides allow/deny; otherwise obtain the user's request-specific permission. Missing risk details require further inspection or user review, independently of a tested answer profile.
5. Submit the complete request-bound `permission respond` command, or present that concrete command for the user to execute. Unknown answer capability preserves manual original-surface input. The [permission command contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#permission-commands) owns required snapshot/provenance flags and response outputs.
6. Preserve the durable receipt and inspect native decision evidence, actual output/exit/error and resumed dependent work. A submitted hotkey, disappearing form or ACK establishes no missing execution result.
7. Consume/ACK available records. The runner delivers the previews, prompts and native commands it queued during the request once the interaction closes, and reports each deferred result as an `input_delivery_result` event; no further send or wait is needed for queued input.

The listener produces stdout records under the [Director receive lifecycle](../mixed-backend-team.md#receive-results). Host interruption, goal activation and model consumption timing remain host-owned.

### Preserve a native request

Candidate detection immediately queues previews and explicit prompt/exec with zero bytes, including zero Escape bytes, until the interaction closes. A permission response answers only the current visible request and its token, through an independently tested answer profile, with the selected single-request allow or deny choice ([request-bound response contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#request-bound-permission-responses)). Human terminal input continues independently of event storage.

The request token is a random identity fence. A changed request, human input (keystrokes, pastes and mouse input, but not the outer terminal's automatic replies such as cursor-position or focus reports), obscuring, degraded observation or ownership loss invalidates it. Reinspect and reconcile the decision when a concrete command becomes stale. The runner exposes no arbitrary key injection or persistent/session-wide permission change.

A durable input receipt records answer submission separately from authorization and native outcome. Inspect `input show INPUT_ID --json` after uncertainty; the Director synchronizes any `input reconcile` disposition with the owner. A different operation key cannot bypass an episode's possibly submitted response. The [request-bound response contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#request-bound-permission-responses) owns privacy, replay protection and the screen-interface race limit.

Metadata notifications intentionally omit command details. Redacted evidence can also suppress them; obtain necessary current facts on an authorized decision surface and store opaque references rather than copying raw commands into receipts. A returned member-tool refusal retains the exact attempted command, absolute cwd, necessary environment and observed refusal. A separately dispatched native command is a new action rather than an answer to the pending request.

### Deliver generated input

The runner chooses how each preview, `member prompt` and `member exec` reaches a wrapped member:

| Member state | Delivery | Reported method |
|---|---|---|
| A detected interaction is open or a candidate, or earlier input is queued | Queued with no bytes; the runner retries after the interaction closes or the earlier input finishes | `null` while queued |
| Empty composer and a measured submission plan for the operation | Text, a short settle, then the measured submit key, without Escape | `profile` |
| Measured submission plan, but a draft or a composer state the runner cannot classify | Queued with no bytes until the composer empties, then delivered by profile; delivered by fallback after 3 minutes | `null` while queued, then `profile` or `fallback` |
| No submission plan | Escape, settle, text, settle, Enter | `fallback` |

A preview is held rather than queued while an earlier receipt is uncertain or the runner's observer, ownership or storage is unavailable; a later `message send` or finite receive retries a held preview.

During the settle before the submit key, the runner distinguishes human input from the outer terminal's automatic replies. Terminal replies and focus reports pass to the native CLI and leave profile delivery intact, so the submit key is still written. Human input during the settle stops the remaining stages, and the delivery becomes uncertain.

Delivery fallback can interrupt work, mix text with a draft, or dismiss a native prompt the runner did not detect; its previews therefore carry the Escape-dismissal clause. Command output reports the status, method and reason of each delivery, and `input show INPUT_ID --json` reports a queued input's eventual result. For an uncertain delivery, `input show INPUT_ID --json` also reports the recorded `uncertain_cause`, such as `manual_input` or `short_write`, so the Director can attribute the outcome before reconciling it. The [delivery reporting contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#delivery-reporting) owns exact statuses and reasons.
