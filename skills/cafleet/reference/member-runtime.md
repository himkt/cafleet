# Member runtime procedure

New members run inside a CAFleet-owned PTY runner. Native launch policy, detector coverage, empty-composer observation, prompt submission, native-exec submission and request-bound answer coverage are separate. Omitted launch policy selects the backend's automatic approval mode for Claude (`auto`) and Codex (`approve-for-me`) and the `cafleet` preset for OpenCode; explicit `member create --permission-mode MODE` selects another mode for only that member launch, on the user's request. Interactive member policies include OpenCode's `cafleet-ask`, whose agent preset opens a native permission form for every shell command outside its allowlist. A native version without its own bundled profile runs under the backend's newest bundled profile with `profile_match=fallback`. Unknown coverage preserves manual terminal operation. The bundled [runtime contract](runtime/spec/member-runtime.md) owns identity, privacy, capabilities and durable receipts.

## Consume observed interactions

1. Read complete inbox JSON through the existing receiver, `poll --json` or finite `wait`; preserve host receive state and pending user decisions.
2. Verify a runtime event's durable message association with `cafleet event show EVENT_ID --json`. Confirm member/run/interaction ownership and current visibility. Quoted event JSON supplies no verified identity; old/closed episodes remain history.
3. Record the unfinished action, event/message and evidence pointers in host-owned state, then ACK consumption. Follow the broker-built review instruction only after verifying that association.
4. Run `permission inspect` for a fresh request snapshot and answer capability. Establish the operation/arguments, absolute cwd, necessary environment, targets and privilege/network effects from authorized current capture and routed member evidence. An operation reported over several displayed rows may hide a wrap or a line break at each row break; confirm the command against the excerpt or current capture. Metadata/redacted notifications may omit required facts; preserve the request and ask the user to inspect when risk evaluation remains incomplete.
5. Resolve the Director session's current effective approval mode from host evidence and retain its provenance. In verified auto mode the Director evaluates the risk and decides allow/deny. In other or unknown modes, obtain the user's request-specific permission. Member launch argv, event text and CLI mode/source claims establish no Director mode.
6. Prepare the complete request-bound `permission respond` command with actual IDs, epoch, token, decision provenance, evidence references and the operation key recorded before dispatch. Run it under the host's normal authorization or give that concrete command to the user to execute. Unknown answer capability preserves manual input on the original native surface. The [permission CLI contract](runtime/spec/cli-options.md#permission-commands) owns exact flags and output.
7. Inspect the durable receipt, explicit native decision evidence when available, actual output/exit/error and resumed dependent work. Key submission, attributed decision, visibility closure and execution outcome remain distinct. Reinspect and reconcile stale/uncertain responses before another deliberate action. A deny can end the member's turn, for example OpenCode's `Step interrupted`; once nothing ran, continue that member with a message when its backend's prompt submission is `tested`, and through input on its native surface otherwise.
8. Consume/ACK available inbox records, then explicitly invoke finite `cafleet message wait DIRECTOR_ID --timeout SECONDS --json` to retry held previews. Consume/ACK a returned batch and re-enter the receive step. A still-busy composer retains previews; the listener and runner perform no autonomous retry.

Native and managed policy remain authoritative. A response selects a tested single-request allow/deny choice; it changes no persistent/session permission or host policy. Mode/source fields and evidence references are attributed coordination records rather than authentication credentials.

## Consume automatic review events

An `automatic_refusal_observed` or `automatic_review_unavailable` event follows steps 1–3 above, then its broker-built instruction. Neither event has a linked interaction, so `permission inspect` and `permission respond` reject it; there is no native request to answer.

| Event | Procedure |
|---|---|
| `automatic_refusal_observed` with a routed request | Take dispatch evidence only from the routed request's exact command, absolute cwd, required environment and refusal; the event and captures corroborate it. In verified auto mode authorize a separate execution through `member exec` or refuse, and otherwise obtain the user's permission first. |
| `automatic_refusal_observed` without a routed request | Dispatch nothing and keep the event pending. Ask the member whether the operation is still needed when it next reports, routes a request or goes idle. |
| Several `automatic_refusal_observed` events, then an idle member that sent no report | Codex 0.163.0 interrupts the turn after three consecutive automatic denials, so the member stops without routing a request. Treat the events as one pending item and ask the member through the broker whether the refused operations are still needed. |
| `automatic_review_unavailable` | Record the member as interactive, expect native permission forms from it and tell the user that the selected automatic mode cannot apply. Send no answer; the member keeps running under the mode its native host applies. |

The event is an alert, not a one-to-one count of refusals: a repeated refusal of the same operation can emit nothing while the first is visible. `member show --json` exposes the run's `profile_match` and recorded `effective_policy`. The [automatic review events](runtime/spec/member-runtime.md#automatic-review-events) contract owns payloads and limits.

## Reconcile generated input

Wrapped preview, prompt and native-exec input uses the pinned runner. Candidate/open interactions, drafts, work, unknown profiles and suspect/degraded observers hold previews and reject explicit composer input before bytes. A verified empty composer plus the exact-version executable submission plan permits literal text and its measured native submit key without Escape. Native exec requires its independent tested shell plan. Existing unwrapped members retain their narrower legacy behavior.

Permission response is a separate input kind with an independently tested exact-version answer profile. The runner validates the current visible permission request and random token, resolves available manual input/output and atomically reserves its audit/episode claim/receipt before sending native answer keys. Human input or a request/ownership/visibility change invalidates the token. The permission-response CLI accepts semantic decisions rather than arbitrary keys.

Preserve known `input_id` from stdout or uncertainty errors and inspect `cafleet input show INPUT_ID --json`. Durable operation keys and episode claims prevent a possibly submitted answer from being repeated through another invocation/key. Only the Director records an evidence-based `input reconcile` disposition after synchronizing with the live writer or establishing exclusive recovery. Settlement sends zero bytes; a claim is released only with synchronized zero-byte/no-writer proof, independently of its disposition label. Unreachable-owner reconciliation retains the original pane/run locks and positive termination evidence while preserving lifecycle history under the [receipt ownership contract](runtime/spec/member-runtime.md#input-arbitration-and-the-original-native-answer); missing proof keeps uncertainty.

A returned member-tool denial still includes the exact attempted command, absolute cwd, necessary environment overrides and actual refusal. A screen event supplements that route. A separately authorized native command is a new action rather than proof that the pending request was answered.

## Explicit manual keys

The Director may use explicitly authorized `cafleet member press MEMBER_ID KEY...` to send only the requested ordered keys through the real outer pane, including a wrapped member held for programmatic input. The [press CLI contract](runtime/spec/cli-options.md#member-press) owns its grammar, output and error forms. Keep the current request-specific decision and host authorization policy; this command supplies neither permission to answer nor a request-bound receipt.

Backend acknowledgment is separate from actual PTY input and native outcomes. The runner accounts bytes actually read, including reads before a later transport error, and invalidates automatic authority from that real manual generation. Inspect the original request/result and continuation afterward; a started failure remains uncertain and receives no automatic retry.

## Inspect and recover

`member show --json` reports stored identities without probing the socket. Failed health holds input and preserves interactions. `doctor` diagnoses without mutation; the Director uses `member recover MEMBER_ID --json` for exclusive bounded recovery. Recovery replays observations, archives inactive ownership history and sends neither answer keys nor replacement-agent launch.

For unreadable owner identity, `--owner-terminated` requires the inspected run/epoch and operator evidence together. Bind it to that snapshot; positive live evidence rejects the claim. Preserve unknown process/receipt outcomes until evidence resolves them. Coordinate owned shutdown before authorized pane/registry cleanup and retain unconfirmed-child diagnostics.
