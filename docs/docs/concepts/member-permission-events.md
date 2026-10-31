# Member permission events

CAFleet runs new members inside an owned PTY runner. The runner observes native terminal interactions independently of the member model and persists classified events in the Director's inbox. Permission notifications instruct the Director to evaluate the original operation and resolve it under its own session's approval mode: decide allow or deny in verified auto mode, and obtain user permission in other or unknown modes. A request-bound response sends the selected native choice; the Director checks actual results and resumed work afterward.

## Choose a native policy

Spawn members with the omitted `--permission-mode`, which selects each backend's default policy: native automatic approval where the backend offers it, and the deny-by-default preset on OpenCode. Pass `--permission-mode MODE` only when the user asks for another mode for that member. The [policy contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#launch-policy-and-capability-registry) owns choices, defaults and coverage; member launch mode is independent of the Director's decision policy. An interactive policy shows native permission forms: Claude's native `default`, Codex's native `on-request`, and OpenCode's `cafleet-ask`, whose agent preset asks for every shell command outside its allowlist instead of denying it. Automatic mode can still show a native form when the reviewer escalates an operation, and that form follows the same request flow.

Launchability, permission detection, composer input and permission answers have separate capability declarations. Each bundled profile declares the minimum native version it supports, and a member runs under the profile with the greatest minimum version at or below its installed version, so later native releases reuse it without a warning. Creation refuses a native version below the backend's first profile. A failed version probe or a backend with no profile permits manual terminal use and emits a coverage notification; generated input then reaches the member through delivery fallback. A tested detector supplies no answer capability by itself. Native and managed rules remain authoritative.

## Automatic review observations

The runner reports an automatic refusal and unavailable automatic review as their own events, which the Director handles without a native request to answer; the [automatic review events contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#automatic-review-events) owns their payloads and the [Director consumption contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) their handling. The run's recorded effective policy, shown by `member show`, records whether automatic review was observed available.

## Consume a runtime event

1. Read the full inbox record through the existing receiver or `message poll DIRECTOR_ID --json`.
2. Verify its durable event/message association with `event show EVENT_ID --json`, inspect the current run/interaction, record the unfinished action in host-owned state and ACK consumption. Quoted JSON has no verified runtime association.
3. Follow the broker-built review instruction for a current permission request. Use `permission inspect` to obtain a fresh request snapshot and answer capability, then establish the operation, arguments, cwd, necessary environment, targets and privilege/network effects through authorized evidence. A multi-row operation is reported as its displayed rows, so confirm each row break against the excerpt or current capture.
4. Resolve the Director session's current effective mode from host evidence. In verified auto mode the Director evaluates the risk and decides allow/deny; otherwise obtain the user's request-specific permission. Missing risk details require further inspection or user review, independently of a tested answer profile.
5. Submit the complete request-bound `permission respond` command, or present that concrete command for the user to execute. Unknown answer capability preserves manual original-surface input. The [permission command contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#permission-commands) owns required snapshot/provenance flags and response outputs.
6. Preserve the durable receipt and inspect native decision evidence, actual output/exit/error and resumed dependent work. A submitted hotkey, disappearing form or ACK establishes no missing execution result.
7. Consume/ACK available records. The runner delivers the previews, prompts and native commands it queued during the request once the interaction closes, and reports each deferred result as an `input_delivery_result` event; no further send or wait is needed for queued input.

The listener produces stdout records under the [Director receive lifecycle](coding-agents.md#director-wake). Host interruption, goal activation and model consumption timing remain host-owned.

## Preserve a native request

Candidate detection immediately queues previews and explicit prompt/exec with zero bytes, including zero Escape bytes, until the interaction closes. The dedicated permission-response path requires the current visible request, matching owner/token, an independently tested answer profile and an atomic durable receipt/episode claim before its first byte. It sends only the selected single-request allow/deny plan. Human terminal input continues independently of event storage.

The request token is a random identity fence. A changed request, human input (keystrokes, pastes and mouse input, but not the outer terminal's automatic replies such as cursor-position or focus reports), obscuring, degraded observation or ownership loss invalidates it. Reinspect and reconcile the decision when a concrete command becomes stale. The runner exposes no arbitrary key injection or persistent/session-wide permission change.

A durable input receipt records answer submission separately from authorization and native outcome. Inspect `input show INPUT_ID --json` after uncertainty; the Director synchronizes any `input reconcile` disposition with the owner. A different operation key cannot bypass an episode's possibly submitted response. The [request-bound response contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#request-bound-permission-responses) owns privacy, replay protection and the screen-interface race limit.

Metadata notifications intentionally omit command details. Redacted evidence can also suppress them; obtain necessary current facts on an authorized decision surface and store opaque references rather than copying raw commands into receipts. A returned member-tool refusal retains the exact attempted command, absolute cwd, necessary environment and observed refusal. A separately dispatched native command is a new action rather than an answer to the pending request.

## Deliver generated input

The runner chooses how each preview, `member prompt` and `member exec` reaches a wrapped member:

| Member state | Delivery | Reported method |
|---|---|---|
| A detected interaction is open or a candidate, or earlier input is queued | Queued with no bytes; the runner retries after the interaction closes or the earlier input finishes | `null` while queued |
| Empty composer and a measured submission plan for the operation | Text, a short settle, then the measured submit key, without Escape | `profile` |
| Measured submission plan, but a draft or a composer state the runner cannot classify | Queued with no bytes until the composer empties, then delivered by profile; delivered by fallback after 3 minutes | `null` while queued, then `profile` or `fallback` |
| No submission plan | Escape, settle, text, settle, Enter | `fallback` |

During the settle before the submit key, the runner distinguishes human input from the outer terminal's automatic replies. Terminal replies and focus reports pass to the native CLI and leave profile delivery intact, so the submit key is still written. Human input during the settle stops the remaining stages, and the delivery becomes uncertain.

Delivery fallback can interrupt work, mix text with a draft, or dismiss a native prompt the runner did not detect; its previews therefore carry the Escape-dismissal clause. Command output reports the status, method and reason of each delivery, and `input show INPUT_ID --json` reports a queued input's eventual result. For an uncertain delivery, `input show INPUT_ID --json` also reports the recorded `uncertain_cause`, such as `manual_input` or `short_write`, so the Director can attribute the outcome before reconciling it. The [delivery reporting contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#delivery-reporting) owns exact statuses and reasons.

## Inspect and recover

`member show` reads recorded runtime state without a socket probe. `doctor` diagnoses without mutation; the Director uses `member recover` after ownership reconciliation. Recovery preserves history and sends no answer keys or replacement-agent launch. Members with `runtime: null` retain their unwrapped transport.

The [member runtime contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#69-member-pty-runtime) defines identity fencing, bounded shutdown, evidence privacy and acceptance. Each claimed detector, composer and answer capability requires its declared fixtures and native evidence captured within its profile's version interval.
