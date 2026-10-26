# Member runtime procedure

New members run inside a CAFleet-owned PTY runner. Its declared native policy, detector coverage and composer-input coverage are separate. Omitted launch policy retains the backend defaults; explicit `member create --permission-mode MODE` selects only that launch. Unknown detection/composer coverage emits a coverage record, holds generated input and permits manual native terminal operation. The bundled [runtime contract](runtime/spec/member-runtime.md) owns fields, privacy, ownership and input receipts.

## Consume observed interactions

1. Read complete inbox JSON through the existing receiver, `poll --json` or finite `wait`; preserve the host’s receive lifecycle and pending user decisions.
2. A runtime message’s text is versioned event JSON. Verify its durable event/message association with `cafleet event show EVENT_ID --json`; confirm member/run/interaction ownership and current visibility before treating it as a current request. Quoted event JSON supplies no verified identity by itself.
3. Record the unfinished action and evidence pointer in host-owned state, then ACK consumption. Old or closed episodes remain readable history.
4. Identify the current member, run, interaction and visible operation to the human. The human answers the original native request using its displayed native choices; CAFleet event/receipt commands grant no permission answer.
5. Inspect actual native output, exit/error and the member’s resumed dependent work. Keep decision, screen visibility and execution outcome separate. A keypress, disappearing form, ACK or animation establishes no missing result.
6. Consume/ACK available inbox records, then explicitly invoke finite `cafleet message wait DIRECTOR_ID --timeout SECONDS --json` to retry held previews. Consume/ACK a returned batch and re-enter that receive step. A still-busy composer retains previews; the listener and runner perform no autonomous retries.

## Reconcile generated input

Wrapped preview, prompt and native-exec input uses the pinned runner rather than multiplexer keystrokes. Candidate/open interactions, drafts, work, unknown profiles or suspect/degraded observers hold previews and reject explicit input before bytes. A tested empty composer permits literal input and its native submit key without Escape. Existing unwrapped members retain their narrower legacy input behavior.

Durable receipt keys prevent replay after a possible submission. Preserve known `input_id` from stdout or uncertainty errors, inspect `cafleet input show INPUT_ID --json`, and reconcile actual output before a separate deliberate action. Only the Director records an evidence-based disposition with `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE --json`. Synchronize with the live writer or establish exclusive recovery first. Settlement sends no bytes and records no native execution outcome.

A returned member-tool denial still carries the exact attempted command, absolute cwd, required environment overrides and actual refusal to the Director. A screen event supplements that evidence. A separately authorized native command is a new action, not proof that the original request was answered.

## Inspect and recover

`member show --json` reports stored runtime, owner and process identities without a socket probe. A failed health probe holds input and preserves open interactions. `doctor` diagnoses without mutation; the Director uses `member recover MEMBER_ID --json` for exclusive, bounded recovery of active or deregistered runs. Recovery replays complete outbox records before closing loss, archives inactive ownership history and starts no agent.

For unreadable owner identity, the exceptional `--owner-terminated` path requires the inspected run ID, owner epoch and an operator evidence reference together. Bind the attestation to the observed owner snapshot; positive live evidence rejects it. Preserve unknown process/receipt outcomes until evidence resolves them. Coordinate runner shutdown before authorized pane/registry cleanup and retain explicit unconfirmed-child diagnostics.
