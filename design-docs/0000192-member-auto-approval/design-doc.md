# Member Automatic Approval and Observed-Denial Routing

**Status**: Aborted
**Progress**: 2/8 tasks complete
**Last Updated**: 2026-10-26

Aborted at the user's request on 2026-10-26. [0000194 — Member PTY Permission Events](../0000194-member-pty-permission-events/design-doc.md) takes over the new direction: CAFleet-owned observation and Director notification with per-member permission-mode selection. Preserve this document and its 2/8 historical task ledger as the record of the earlier work; the six unchecked runtime tasks remain incomplete. Statements about `aa6b3724` and its validation below describe that historical work rather than the current source baseline, which 0000194 inspects independently.

## Overview

Members execute role-authorized task commands under each backend's selected native permission policy and route actual permission refusals to the Director. Apply the common instruction to every member-create assignment, including custom prompts that load no skill, while preserving native permission enforcement and host/operator authority. This document extracts that policy from the aborted Hermes design and records the implementation already committed in `aa6b3724`, its evidence, and remaining runtime acceptance work.

## Success Criteria

- [x] The specification covers every backend registered by `CodingAgent::all()` and the conditional Hermes policy, including OpenCode mini's existing fixed preset.
- [x] Source inspection identifies one common member-create prompt boundary for inline and file assignments, literal broker identities, exactly one assignment substitution, and rollback on invalid placeholders.
- [x] The extraction leaves the Hermes design Aborted with its actual remaining task count, retains its native smart/deny requirements and profile-lifecycle decision, and preserves `runtime-validation.md` byte for byte.
- [ ] Authorized future runtime evidence establishes each registered backend's effective selected policy and its actual allowed-command, refusal, and ordinary-error behavior.
- [ ] A returned permission refusal reaches the Director with the exact command, cwd, required environment overrides, and actual refusal; authorized execution/refusal and member continuation retain their original evidence and authority.
- [ ] Unavailable automatic review and unresolved interactive approval receive accurate supervision, including a member that cannot send while the host awaits a decision.
- [ ] Hermes smart behavior is accepted only after a separately reviewed, resumed Hermes integration satisfies its backend gates; existing manual-mode probes supply historical evidence only.

---

## Background

The shared launch-policy work was completed independently of the failed Hermes runtime milestone. Commit `aa6b3724` contains the final selected-policy implementation and corresponding repository documentation; its test contracts were committed separately. The Hermes design later reached Aborted status with unrelated parser/catalog work also complete. Separating this concern makes its cross-backend contract discoverable without implying that Hermes support shipped or that this documentation extraction implements or deploys anything.

Design `0000181-auto-mode-permission-settings` is historical context for Director settings recommendations. This document owns ordinary-member launch policy, assignment assembly, observed-refusal routing and its acceptance boundaries. Existing Director settings, runtime configuration, installed assets, production code, tests and commits remain outside this documentation change.

---

## Specification

### Selected Backend Policies

| Backend | Member launch policy | Enforcement and evidence boundary |
|---|---|---|
| Claude | `--permission-mode auto`; preserve the existing broker-tool allowance and model/effort arguments | Native rules, hooks and managed policy remain authoritative. Requested auto mode establishes launch intent; effective availability and any human checkpoint require runtime evidence. |
| Codex | `--approve-for-me --sandbox workspace-write`; preserve model/effort arguments | Automatic review operates within the effective host policy and explicit workspace sandbox. Launch argv alone establishes neither live review availability nor a refusal outcome. |
| OpenCode | `mini --standalone --agent cafleet`; retain the fixed permission map in `presets/opencode/cafleet.md` | The preset is the source of truth for its allowed and denied operations, including file boundaries. Mini's fixed policy is the user's selected behavior; it has no automatic-mode acceptance requirement. |
| Hermes, conditional | Native `approvals.mode: smart` with the reviewed dangerous-command deny rules | The Hermes design owns concrete deny rules, native matching limits and backend integration. The current registry contains no Hermes backend; its model catalog supplies no executable support or runtime acceptance. |

Members attempt permitted role-authorized commands themselves. They use native file tools within their role and read prerequisites with native readers or host-permitted shell reads before ready. An actual prerequisite refusal follows the same routing path as a task refusal. The shared policy preserves native guards, explicit denies, managed rules, host permissions and sandbox settings throughout startup and task execution.

Broad tool or shell allowances remain scoped by the member's role and assignment. In particular, OpenCode's broad CAFleet pattern supplies no Director-only dispatch or Git-write authority. Its native `member prompt` and `member exec` denials retain their ordering after that broad pattern; the complete permission map remains owned by the preset rather than copied here.

Future Hermes integration consumes the operator's existing Hermes configuration, starts fresh conversations and owns the running member processes, following the user's recorded rejection of per-member profiles. Its lifecycle plan requires a separate revision before renewed execution. The shared policy neither defines a new Hermes profile lifecycle nor weakens the existing abort or failed/unrun gates.

### Common Member-Create Interface

`cafleet/src/cli/member.rs::create_with_runtime` resolves the backend and authored assignment, obtains the real fleet/Director/member identities, and passes them to `cafleet/src/spawn_prompt.rs::assemble_member_prompt` before building spawn argv. `assemble_member_prompt` applies `substitute_spawn_placeholders` once to the assignment and prefixes the common runtime instruction plus literal broker identities and ready/refusal commands. The pure formatter retains its existing supported-placeholder and usage-error contract.

This boundary applies to every backend selected through the registry, both explicit and inherited selection, and every accepted assignment source. Inline, file and custom/ad-hoc prompts receive the instruction even when their authored body loads no skill. File/stdin resolution follows the existing `PromptArgs`/`resolve_body` contract; the focused assembly tests currently exercise inline and filesystem-file inputs.

Correct assembly preserves the assignment's remaining UTF-8 bytes, CRLF, whitespace, quotes and escaped braces after the single substitution. The complete assembled prompt reaches the backend as one literal argv element. Unknown or malformed placeholders retain the pure formatter's usage error and exit code `2`; creation rolls back the pending registration before any pane split. Native permissions enforce execution; the preamble supplies agent guidance and broker addressing, with no deterministic tool-event interceptor or automatic denial handler.

Required preamble behavior is:

1. Execute role-authorized task commands under the selected backend policy and inspect the actual result.
2. Route an observed permission refusal with the required evidence; recover normally from an ordinary command failure.
3. Preserve role, native, managed and host/sandbox authority.
4. Surface unresolved approval under every selected policy; surface unavailable automatic review when automatic review is selected.
5. Wait for an actual result or explicit continuation before resuming work dependent on a routed request.

### Refusal Evidence and Member Continuation

The refusal request is a broker message, using the member and Director IDs rendered into the runtime prompt. Its fields describe an observed action; they introduce no CLI flag, database schema or machine-parsed refusal event.

| Required evidence | Contract |
|---|---|
| Exact command | Preserve the literal attempted command and arguments. |
| Working directory | Supply the absolute cwd in which the command was attempted. |
| Environment | Supply required overrides, or explicitly state `none`. |
| Refusal | Include the actual native/host permission refusal and relevant pending-request identity when available. |

```text
cafleet message send --from-member-id <member-id> --to-member-id <director-id> "Need to run: <exact command>. Cwd: <absolute working directory>. Environment: <required overrides or none>. Refusal: <actual permission denial>."
```

For a hard-to-quote command, a permitted native editor may preserve the exact command and refusal evidence in a file under the supplied BASE and send its path. The member reports actual results; missing essential command/cwd/refusal evidence remains an unresolved request that the Director must clarify before dispatch. A denied file edit requires an authorized Director edit or host-rule change. An observed broker failure is reported to the operator through the remaining available host surface.

| Observed condition | Required behavior |
|---|---|
| Command completes, including an ordinary nonzero exit | Apply normal task-specific debugging and recovery; retain its real exit status. |
| Native/host permission returns a refusal | Send the evidence above, retain the original outcome and stop dependent work for Director review. |
| Selected automatic review is unavailable | Surface the actual unavailable/fallback state to the Director; effective approval remains unresolved. |
| Approval is pending | Preserve the original request and its actual decision surface; settle that request before another attempt. |
| An interactive approval prevents sending | Director supervision inspects the pane/host state and obtains the authorized decision through its normal surface. |
| Dispatch or execution outcome is uncertain | Inspect and reconcile existing evidence before authorizing a retry. |

A classifier rejection, explicit native denial, unavailable reviewer, timeout and unanswered/withdrawn prompt retain their observed meanings. Attribute a user refusal only to an actual user decision. Guidance cannot guarantee that a blocked member will send a message; supervision remains necessary for approvals that suspend the member's turn.

### Director Review and Dispatch

The Director reviews command/cwd/environment/refusal evidence under the host's and operator's existing authority. A routed request supplies a request for judgment; native and managed denials remain authoritative. The Director may refuse with an explanatory broker reply, relay a required operator decision, or submit a separately authorized native execution. Preserve the selected cwd and required environment explicitly in any authorized command rather than assuming the pane already has that context.

For authorized native execution, use the existing `cafleet member exec <member-id>` interface, which submits Escape-first native shell input under the shared pane-input lock. Its successful receipt establishes submission only. The Director captures/inspects actual output, exit status and terminal state, records the outstanding action in host-owned state, consumes/ACKs the broker request under the normal protocol, and explicitly continues a member that has not resumed when needed. Dependent work resumes from observed output or explicit continuation.

A partial typing failure or failure after Enter can leave an executed command with uncertain delivery metadata. Retain the pane and original evidence, inspect them, and authorize any retry only after reconciliation. The existing routing and supervision instructions own these dispatch mechanics; harmless native-output probes establish visibility only and supply no proof of a second member-tool permission check.

### Source Mapping and Existing Implementation Evidence

| Source owner | Existing responsibility |
|---|---|
| `cafleet/src/coding_agent/mod.rs` | Backend registry and literal launch argv, including native Claude/Codex modes and preserved OpenCode mini arguments. |
| `cafleet/src/spawn_prompt.rs` | Pure placeholder formatter and the common member-prompt assembly wrapper. |
| `cafleet/src/cli/member.rs` | Production creation boundary, resolved assignment sources, backend inheritance, registration rollback and spawn publication. |
| `presets/opencode/cafleet.md` | Fixed native permission map and member behavior within that map. |
| `cafleet/src/assets.rs` | Embedded-preset test that installs into a temporary home and checks the packaged permission map and ordering. |
| `skills/cafleet/reference/coding-agents.md` | Backend-specific permission bindings and limits; conditional Hermes model data remains separate from runtime registration. |
| `skills/cafleet/roles/member.md` and role instructions under `skills/cafleet-design-doc/` | Startup, actual-result reporting and assigned-member command behavior. |
| `skills/cafleet/reference/prompt-routing.md`, `roles/director.md` and `reference/supervision.md` | Refusal evidence, host-authorized dispatch, uncertain outcomes, pending approvals and continuation. |
| Human documentation under `docs/docs/`, plus `README.md` | Operator-facing launch, lifecycle, CLI and mixed-backend descriptions reconciled by the existing implementation commit. |

The final source implementation is `aa6b37243a61b607f7580c0c93f3c9a79c4c3aab` (`feat: use automatic approvals and route member denials`). The final test handoff includes `2cb664e9`, `7cbfc4dc` and `8af4a6fc`; their changing proposals are historical steps, while the current source and final implementation commit define the retained behavior.

| Current test | What its source establishes | Remaining boundary |
|---|---|---|
| `literal_argv_preserves_permission_and_optional_flag_boundaries` | Exact selected-mode argv; optional model/effort and single prompt-argument boundaries. | Effective native automatic review and permission decisions. |
| `member_create_routes_inline_and_file_assignments_for_every_registered_backend` | Fake-multiplexer production creation path iterating `CodingAgent::all()`; explicit/inherited backends, inline/file assignments, common preamble, literal identities and byte preservation. | Actual model compliance, real TTY approvals, stdin-specific coverage and future unregistered backends. |
| `member_create_rolls_back_invalid_assignments_before_spawning_for_every_backend` | Unknown/malformed placeholder usage errors, preserved roster/placement state and no pane split. | Runtime authorization and live approval recovery. |
| `installed_opencode_preset_preserves_routine_permissions_and_dispatch_denials` | Embedded preset content, complete permission map and dispatch-denial ordering. | Live mini permission-engine decisions. |

Historical implementation validation is recorded in [.hermes-spike/shared-routing/automatic-mode-checkpoint.md](../../.hermes-spike/shared-routing/automatic-mode-checkpoint.md): `mise //cafleet:test member_create_` passed 2 tests with 259 filtered out; the full library suite passed 261 tests; lint, typecheck and documentation build passed. Those are the earlier implementation run's reported results. This extraction inspected the source and records; it has not rerun those checks and claims no new suite or live-runtime result.

The preserved [Hermes runtime record](../0000191-hermes-backend/runtime-validation.md), especially its automatic-approval assessment, distinguishes installed-source/documentation findings from live probes. It records Claude auto availability/ask limits as documented constraints and Hermes smart guardian/native-deny/auxiliary-routing behavior as source findings. Effective live Claude automatic review, Hermes smart execution and auxiliary review routing remain unverified there. The [OpenCode assessment](../../.hermes-spike/opencode-auto/assessment.md) records the inspected mini limitation; the final fixed-preset choice does not require another automatic mode.

Current repository source is distinct from installed binaries/assets and the effective configuration of an already-running member. The historical checkpoint explicitly left installed runtime/assets unchanged. Confirm that installation and host policy in a future authorized runtime validation before attributing observed behavior to this source version. This extraction changes neither installation nor runtime settings.

### Extraction and Acceptance Boundaries

The [Hermes design](../0000191-hermes-backend/design-doc.md) retains native smart approval, concrete dangerous-command deny requirements and matching limits, Hermes runtime gates, its completed compatibility-evidence task and independent completed parser/catalog tasks. This extraction removes the shared policy paragraph, its completed task and the corresponding shared-policy commentary; mixed commentary retains genuine Hermes-only findings. Its actual ledger becomes 3 checked tasks out of 20, with Status Aborted. The no-per-member-profile decision remains recorded for the separate lifecycle revision; unrelated lifecycle text remains outside this split.

Keep `runtime-validation.md`, captures and spike artifacts as historical evidence. Their earlier approvals, task counts, manual-mode observations and failed/unrun gates remain point-in-time records. The new document carries the final shared decisions and evidentiary summary rather than reproducing the superseded instruction thread. Documentation checks validate the split, local reference targets, source mapping, timestamped task counts and preservation scope. Production suites need a new run only when future authorized implementation changes justify one.

The extraction's source/document checks passed: local Markdown targets resolve, task counts match both headers, and the Hermes ledger retains every task verbatim except the extracted shared task. Its ownership/lifecycle sections, implementation Steps 2–6 and no-per-member-profile decision remain unchanged. The preserved runtime record has SHA-256 `c85a72951dcf0a918cdd6a1ef5f83c9c6e050cc9a64f46213c99b586c6622d74`; tracked `git status --short` remains empty. Both design documents remain ignored and uncommitted. These checks establish documentation consistency and preservation only.

Runtime acceptance requires observations from the actual selected host/backend with the source/installed versions, multiplexer, cwd, policy and redacted command/result evidence recorded. Use disposable fixtures and approved operations to distinguish a permitted command, an ordinary failing command and a genuine permission refusal. Exercise unavailable/pending states when the host can expose them; otherwise retain an explicit unverified result. Synthetic/fake-host tests and requested launch arguments retain their narrower evidence scope.

Hermes live acceptance additionally requires a separately reviewed lifecycle revision and explicit resumed implementation assignment. Preserve its Aborted status, M4 failure and unrun dependent gates throughout this documentation workflow. Its concrete native deny rules and smart auxiliary-model/provider boundary remain owned by the Hermes integration specification.

---

## Implementation

> Check a task only when its stated work is complete and record the completion timestamp in the same edit. The inherited implementation timestamp below is historical; future runtime tasks require their own observed evidence.

### Step 1: Preserve the Existing Baseline and Extract the Specification

- [x] Retain the committed common member-prompt instruction, Claude/Codex selected modes, OpenCode fixed preset, assignment/identity preservation, rollback coverage and reconciled role/operator documentation in `aa6b3724`; record the historical implementation evidence without claiming deployment or new runtime verification. <!-- completed: 2026-10-25T14:32 -->
- [x] Complete the documentation split, check source mappings and local references, recompute the Hermes ledger to 3/20, and confirm its abort/profile decision, independent completed work, historical runtime record and tracked repository state are preserved. <!-- completed: 2026-10-25T23:32 -->

### Step 2: Validate the Registered Backends in a Future Authorized Session

- [ ] Verify Claude's effective native auto policy with source/installed versions recorded, an allowed role command, an ordinary failure and actual refusal evidence; retain explicit deny/ask/hook/managed authority and record any unavailable or pending condition. <!-- completed: -->
- [ ] Verify Codex automatic review with the explicit workspace-write sandbox, recording effective host eligibility/policy, permitted and refused operations, sandbox enforcement and ordinary-error recovery. <!-- completed: -->
- [ ] Verify OpenCode mini's current fixed preset in the real host, including a permitted operation, observed refusal, dispatch-denial precedence and retained native file boundaries; preserve the existing launch interface and role authority. <!-- completed: -->

### Step 3: Validate Refusal, Supervision and Continuation in a Future Authorized Session

- [ ] Observe a real member refusal and complete command/cwd/environment/refusal payload, then verify Director refusal or separately authorized dispatch, actual result inspection, ACK and dependent continuation; reconcile an uncertain dispatch before any retry. <!-- completed: -->
- [ ] Exercise unavailable automatic review and unresolved approval where supported, including a host prompt that prevents a member send; preserve the original request, distinguish timeout/cancellation from user denial, and record unsupported/unobserved cases accurately. <!-- completed: -->

### Step 4: Accept Hermes Only with Its Separate Resumed Integration

- [ ] After the Hermes lifecycle revision is reviewed and implementation is explicitly resumed, validate the native smart/deny policy and effective auxiliary route through the accepted real-TTY integration, preserving native refusal floors, existing configuration, fresh conversations and every applicable backend release gate. <!-- completed: -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-10-26 | Mark Aborted at the user's request and hand the new PTY-observation, notification and selectable-mode direction to 0000194. Preserve the historical 2/8 ledger and leave its remaining runtime tasks incomplete. |
| 2026-10-26 | Extract the cross-backend member permission/routing specification, common prompt boundary, completed shared task and implementation-evidence summary from design 0000191. Preserve its Aborted status, Hermes smart/deny details, profile-lifecycle decision and independent completed tasks; recompute its remaining ledger to 3/20. Record `aa6b3724` as existing implementation and keep unobserved runtime acceptance pending. |
