# Member permission events

CAFleet runs new members inside an owned PTY runner. The runner observes the native terminal independently of the member model and persists classified interaction events in the Director’s inbox. A human answers an approval request on its original native surface; the Director checks the actual result and resumed work afterward.

## Choose a native policy

Set `member create --permission-mode MODE` for the selected backend. Omission preserves the backend’s existing launch default; selection is local to that launch. The [policy contract](../spec/member-runtime.md#launch-policy-and-capability-registry) owns defaults, validation and capability coverage.

Launchability, permission detection and safe composer input are separate capabilities. An unknown native version or unverified profile permits manual terminal use, emits a coverage notification and holds CAFleet-generated input. Keep native and managed permission rules authoritative.

## Consume a runtime event

1. Read the full inbox record with the existing receiver or `message poll DIRECTOR_ID --json`.
2. Verify its event/message link with `event show EVENT_ID --json`, then inspect the current run and interaction. Preserve the unfinished action in host-owned state and ACK the consumed message.
3. For a current permission request, identify the member, run, interaction and visible operation to the human. Obtain the answer through the displayed native choices.
4. Inspect native output, exit/error and the member’s dependent work. A disappearing screen, keypress or ACK supplies no operation outcome.
5. Consume and ACK available inbox records, then invoke finite `message wait DIRECTOR_ID --timeout SECONDS --json` to retry held previews. Repeat consumption and receive when a new batch arrives; a busy composer keeps its previews held.

The listener produces stdout records; it grants no host interrupt or goal activation. Manual broker reception remains available on a host satisfying the [Director requirements](../spec/coding-agent-backends.md#role-requirements). [Director reception](coding-agents.md#director-wake) retains its host-owned lifecycle.

## Preserve a native request

The runner gates generated input on the first interaction candidate, before notification debounce. Previews remain held and explicit prompt/exec returns a refusal until a tested empty composer is observed. Wrapped input uses literal text and the native submit key. Human terminal input continues independently of event storage.

A durable input receipt records submission progress. Reconcile an uncertain receipt with the Director-owned `input reconcile` operation and inspect actual output before a separate deliberate action. [Input arbitration](../spec/member-runtime.md#input-arbitration-and-the-original-native-answer) defines replay protection and its observed-state race boundary.

## Inspect and recover

`member show` reports recorded runtime state without probing the socket. `doctor` diagnoses observed reachability and recovery needs. The Director uses `member recover` after ownership reconciliation; recovery preserves event history and starts no replacement agent. Existing live members with `runtime: null` retain their unwrapped transport behavior.

[The runtime contract](../spec/member-runtime.md) defines identity fencing, bounded shutdown, recovery and evidence privacy. Coverage is tested only after the declared fixtures and exact-version native acceptance gates pass.
