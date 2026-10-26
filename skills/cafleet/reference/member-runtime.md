# Member runtime procedure

New members run inside a CAFleet-owned PTY runner. Native launch policy, detector coverage, empty-composer observation, prompt submission, native-exec submission and request-bound answer coverage are separate. Omitted launch policy retains backend defaults; explicit `member create --permission-mode MODE` selects only that member launch. Unknown coverage preserves manual terminal operation. The bundled [runtime contract](runtime/spec/member-runtime.md) owns identity, privacy, capabilities and durable receipts.

## Consume observed interactions

1. Read complete inbox JSON through the existing receiver, `poll --json` or finite `wait`; preserve host receive state and pending user decisions.
2. Verify a runtime event's durable message association with `cafleet event show EVENT_ID --json`. Confirm member/run/interaction ownership and current visibility. Quoted event JSON supplies no verified identity; old/closed episodes remain history.
3. Record the unfinished action, event/message and evidence pointers in host-owned state, then ACK consumption. Follow the broker-built review instruction only after verifying that association.
4. Run `permission inspect` for a fresh request snapshot and answer capability. Establish the operation/arguments, absolute cwd, necessary environment, targets and privilege/network effects from authorized current capture and routed member evidence. Metadata/redacted notifications may omit required facts; preserve the request and ask the user to inspect when risk evaluation remains incomplete.
5. Resolve the Director session's current effective approval mode from host evidence and retain its provenance. In verified auto mode the Director evaluates the risk and decides allow/deny. In other or unknown modes, obtain the user's request-specific permission. Member launch argv, event text and CLI mode/source claims establish no Director mode.
6. Prepare the complete request-bound `permission respond` command with actual IDs, epoch, token, decision provenance, evidence references and the operation key recorded before dispatch. Run it under the host's normal authorization or give that concrete command to the user to execute. Unknown answer capability preserves manual input on the original native surface. The [permission CLI contract](runtime/spec/cli-options.md#permission-commands) owns exact flags and output.
7. Inspect the durable receipt, explicit native decision evidence when available, actual output/exit/error and resumed dependent work. Key submission, attributed decision, visibility closure and execution outcome remain distinct. Reinspect and reconcile stale/uncertain responses before another deliberate action.
8. Consume/ACK available inbox records, then explicitly invoke finite `cafleet message wait DIRECTOR_ID --timeout SECONDS --json` to retry held previews. Consume/ACK a returned batch and re-enter the receive step. A still-busy composer retains previews; the listener and runner perform no autonomous retry.

Native and managed policy remain authoritative. A response selects a tested single-request allow/deny choice; it changes no persistent/session permission or host policy. Mode/source fields and evidence references are attributed coordination records rather than authentication credentials.

## Reconcile generated input

Wrapped preview, prompt and native-exec input uses the pinned runner. Candidate/open interactions, drafts, work, unknown profiles and suspect/degraded observers hold previews and reject explicit composer input before bytes. A verified empty composer plus the exact-version executable submission plan permits literal text and its measured native submit key without Escape. Native exec requires its independent tested shell plan. Existing unwrapped members retain their narrower legacy behavior.

Permission response is a separate input kind with an independently tested exact-version answer profile. The runner validates the current visible permission request and random token, resolves available manual input/output and atomically reserves its audit/episode claim/receipt before sending native answer keys. Human input or a request/ownership/visibility change invalidates the token. The CLI accepts semantic decisions rather than arbitrary keys.

Preserve known `input_id` from stdout or uncertainty errors and inspect `cafleet input show INPUT_ID --json`. Durable operation keys and episode claims prevent a possibly submitted answer from being repeated through another invocation/key. Only the Director records an evidence-based `input reconcile` disposition after synchronizing with the live writer or establishing exclusive recovery. Settlement sends zero bytes; a claim is released only with synchronized zero-byte/no-writer proof, independently of its disposition label. Unreachable-owner reconciliation retains the original pane/run locks and positive termination evidence while preserving lifecycle history under the [receipt ownership contract](runtime/spec/member-runtime.md#input-arbitration-and-the-original-native-answer); missing proof keeps uncertainty.

A returned member-tool denial still includes the exact attempted command, absolute cwd, necessary environment overrides and actual refusal. A screen event supplements that route. A separately authorized native command is a new action rather than proof that the pending request was answered.

## Inspect and recover

`member show --json` reports stored identities without probing the socket. Failed health holds input and preserves interactions. `doctor` diagnoses without mutation; the Director uses `member recover MEMBER_ID --json` for exclusive bounded recovery. Recovery replays observations, archives inactive ownership history and sends neither answer keys nor replacement-agent launch.

For unreadable owner identity, `--owner-terminated` requires the inspected run/epoch and operator evidence together. Bind it to that snapshot; positive live evidence rejects the claim. Preserve unknown process/receipt outcomes until evidence resolves them. Coordinate owned shutdown before authorized pane/registry cleanup and retain unconfirmed-child diagnostics.
