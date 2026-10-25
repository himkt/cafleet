# Member lifecycle

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` registers the current Director atomically. `member create` inherits that Director's backend unless overridden, validates model/effort, registers a pending placement, renders the assignment's identity placeholders and prepends the shared mandatory routing policy with literal broker identities. It then splits a pane and patches its owned pane ID. Creation compensates for failures while preserving the original cause and any cleanup error.

Each member reads its role and required host rules with native file tools, using permitted shell reads when shell is the only reader. An actual prerequisite denial routes with command/cwd/environment/refusal evidence using the runtime prompt's broker identities. After startup reads, it sends `ready` and immediately polls its inbox. The Director can assign that member as soon as its ready arrives. The member polls before completion reports and on every subsequent wake, then handles queued assignments before becoming idle.

## Assignment and receiving

Messages are point-to-point. Send individually to every selected recipient. Ordinary members receive Escape-first pane previews; the root Director uses a persistent native inbox stream or explicit manual broker reads. Record deferred actions and ACK consumption, retaining unfinished work in host-owned task state. Arm Director reception before relying on ready replies and collect the receiver's exit during teardown. See [run a fleet](../how-to/mixed-backend-team.md#manual-lifecycle).

## Command routing

A member executes task shell commands under the backend's selected permission policy and routes an observed permission denial to the Director. It supplies the exact command, working directory, required environment and actual refusal reason, then awaits a result or explicit continuation before dependent work. Surface unresolved approval to the Director under every policy. When automatic review is selected, surface an unavailable automatic mode to the Director. Ordinary nonzero command results follow normal recovery. Native file tools remain within role scope. This policy is injected for every member-create path, including ad-hoc prompts without a skill, while native guards, explicit denies, managed policies and host permission/sandbox settings remain in force.

The Director reviews each routed refusal under existing host and operator authority. Routing grants no override of a native or managed denial. An authorized `member exec` visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, send an explicit `member prompt` to continue if the host has not resumed the member. A failed or uncertain dispatch requires inspection before any newly authorized retry. Supervise interactive approvals that can prevent the member from sending. The prompt instruction establishes member behavior; deterministic refusal-event interception is outside this contract.

The [native dispatch contract](../spec/cli-options.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

## Inspection and recovery

Use `member list`, `member capture` and `fleet capture` to check roster and pane evidence. Capture about 20 lines with `member capture` (its default) and read the content directly; use `--lines` for more when it is insufficient. An empty composer does not mean the task is complete — judge completion from messages and artifacts. A timeout alone establishes neither failure nor completion. Replace an unusable member only under the existing authorization, carrying its assignment, messages and artifacts into the replacement prompt.

## Teardown

Delete owned ordinary members before deleting the fleet. Member deletion kills its owned pane and deregisters it; fleet deletion deregisters the registry without closing panes. Both serialize placement changes with pane input and fail without registry changes if lock contention lasts five seconds. The root Director is protected from individual member deletion.
