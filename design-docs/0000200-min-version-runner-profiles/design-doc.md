# Minimum-version runner profiles and reliable wrapped delivery

**Status**: Complete
**Progress**: 27/27 tasks complete
**Last Updated**: 2026-10-30

## Overview

Select each backend's bundled runner profile by its minimum supported native version and reuse its complete capability set until a newer compatibility boundary requires another profile. Supply measured prompt and native-exec submission plans, separate literal input from submission with a settle interval, and use an explicitly reported Escape-based delivery fallback when an executable plan or a verified empty composer is unavailable. Preserve detected native interactions by queueing ordinary input in the runner and retrying after the interaction closes.

## Success Criteria

- [x] A native version selects the profile with the greatest `min_version` at or below it, using SemVer precedence within its backend; adding a later boundary preserves selection for earlier supported versions.
- [x] Claude Code `2.1.308` selects `claude-2.1.302-auto-1` without a version-mismatch warning and uses that profile's detection, annotations, prompt, native-exec and permission-answer capabilities.
- [x] A known version below its backend's first boundary fails before member/run registration or pane creation; failed version probes still permit launch with unknown coverage and use delivery fallback when requested.
- [x] Every bundled profile declares independently measured, native-fixture-backed prompt and native-exec submission plans; a literal write and its submit key always have a settle interval between them.
- [x] Missing operation plans and an unverified composer cause fallback delivery rather than an unexplained hold, including after a failed version probe.
- [x] Candidate or open detected interactions receive zero ordinary-input bytes, including zero Escape bytes; queued previews, prompts and native commands retry from the runner after closure without another send/wait invocation.
- [x] Immediate and deferred delivery expose their status and actual method, and duplicate requests, partial writes and runner failure preserve receipt-based reconciliation.
- [x] Current runtime output and records contain no `profile_match`, the database upgrade preserves existing runtime/audit relationships, and living documentation describes the new behavior.

---

## Background

The reported cafleet `0.30.0` run on herdr selected `claude-2.1.302-auto-1` for Claude Code `2.1.308` and warned that the match was a fallback. Permission detection and `claude-bash-allow-digit` worked, while every message send returned `notification_sent=false`, no preview reached the member, and `member prompt` reported `matching executable composer plan is unavailable`.

Repository inspection finds two distinct causes. `ProfileCatalog::selection` first checks exact declared versions and otherwise chooses the backend's newest profile, including for versions older than that profile; its match distinction propagates into storage and presentation. `composer_plan` already uses this shared selector, but every shipped profile's composer declaration lacks `submission`, and tests explicitly expect prompt/native-exec coverage to be unknown even at the declared version. Changing selection alone therefore leaves wrapped delivery unusable.

The Director reproduced the missing-plan failure on Codex `0.163.0` with an exact match. During the workaround, a single ordered `p o l l Enter` key request typed text without submitting it; a later separate Enter submitted it. The resulting requirements include measured operation plans, staged input with settle intervals, fallback delivery, and runner-owned interaction deferral.

---

## Specification

### Profile boundaries and version ordering

Each profile has required `backend`, `min_version`, `profile_version`, `rules` and `capabilities`. Replace the profile-level `native_versions` array with one nonblank SemVer `min_version`; retain fixture-level `native_version` as the actual capture version. Profiles form an ordered stack within their backend. Multiplexer, platform and permission mode introduce no additional selection axis.

Retain the existing bundled profile identifiers and set their initial boundaries to the native versions previously declared in their entries. Adding operation plans to an existing entry changes its bundled contents without inventing a new native compatibility boundary. Add another profile only when an observed native change requires different rules or plans; its `min_version` is the first native version requiring the change.

Add `semver = "1"` to the Rust dependencies and use one shared version parser and precedence comparison for profile validation, native-version admission, fixture validation and selection:

- Parse canonical `major.minor.patch` SemVer, including optional prerelease and build metadata. Strip one leading `v` only when extracting a native executable's version token.
- Compare numeric components numerically, prerelease identifiers by SemVer precedence, and releases above their prereleases. Build metadata contributes no precedence.
- Reject two profile thresholds with equal precedence for the same backend, including thresholds differing only in build metadata. Selection is independent of catalog order.
- Reject malformed profile or referenced fixture versions during catalog validation with an `invalid native profile: ...` usage error. Synthetic test catalogs use valid SemVer strings, such as `0.0.1-synthetic`.

For a successfully parsed native version `v`, select:

```text
eligible = profiles where profile.backend == backend and profile.min_version <= v
selected = eligible profile with greatest min_version
```

| Native version and catalog state | Creation behavior | Profile and coverage |
|---|---|---|
| Version at a boundary or between boundaries | Launch with the selected profile | Its identifier and independently declared capabilities |
| Version newer than every boundary | Launch with the newest eligible profile | The same capabilities; there is no upper cap or version-mismatch warning |
| Version below the backend's first boundary | Exit 1 before registration/pane creation | `unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION` |
| Failed version probe | Continue launch with the existing probe warning | Null native/profile versions, unknown coverage; ordinary input uses fallback |
| Backend with no profile entries | Continue launch | Null selected profile and unknown coverage; ordinary input uses fallback |

For example, boundaries `2.1.302` and `2.1.310` select the first profile for `2.1.308` and `2.1.310-beta.1`, and the second for `2.1.310` and `2.2.0`. `2.1.301` is unsupported. A prerelease threshold can itself be introduced when a measured prerelease requires a new profile.

Keep the existing probe executable, time/output bounds and first-line single-version policy. Identify version tokens with the shared SemVer parser after stripping one leading `v`, accepting prerelease and build metadata together; exactly one whitespace-delimited token must parse. Zero or several version tokens cause a failed probe with the existing unknown-coverage warning and launch behavior; an incomplete version such as `2.1` is not normalized into a supported version.

### One selected profile for all capabilities

Resolve detection, annotations, empty-composer observation, prompt submission, native-exec submission and request-bound permission answers through the same selected profile. Keep their coverage declarations independent: selecting a profile or observing an empty composer alone supplies no executable plan or answer capability.

The run retains the actual probed `native_version` and selected `profile_version`. `ComposerPlan` and `AnswerCapability` retain the actual run version, selected profile identifier and cited fixture identities; the original capture versions stay in their fixture records. Input validation compares these identities with the run, rather than requiring the run version to equal a capture version or boundary.

Validate every cited fixture's backend and capture version against its owning profile's interval: at or above `min_version` and, when another threshold exists, below the next threshold for that backend. Require nonempty capability-specific provenance and preserve native-source, digest, geometry, rule, choice, focus and stage checks. Native evidence establishes the claimed operation within the profile interval; each subsequent native release reuses the declaration without requiring another exact-version capture.

The runner pins its selected profile for its lifetime. Fresh creation and observer initialization verify that deterministic selection agrees with the recorded profile and coverage. Historical run/event/audit reads preserve their recorded identities; recovery reports an unavailable or mismatched historical profile explicitly rather than silently attaching another profile. Catalog updates affect newly launched runners.

### Measured submission plans and settle intervals

Add `capabilities.composer.submission` to every bundled profile. It has required `prompt` and `exec` operation definitions; each has exactly `prefix_hex`, `submit_hex` and `fixture_ids`. Prompt uses an empty prefix, exec uses `2120` (`! `), and `submit_hex` contains the measured native submit key. Preserve the existing hexadecimal and byte-length validation and independent prompt/exec provenance.

Measurement is implementation work, not an assumed Enter mapping. For each bundled profile, capture native empty-composer, typed-literal/pre-submit and post-submit behavior for prompt and native shell input on a version in its interval. Record the native version, multiplexer, screen geometry, policy, exact stage bytes/times, observed native result and privacy-reviewed fixture digests. An idle screen capture alone is insufficient evidence that either operation executes.

Use a 500 ms monotonic settle interval, matching the existing pane-input settle delay. Every profile-driven submission follows these separate runner stages:

1. Revalidate the run, active placement, observer and interaction state, then durably reserve the input before any child byte.
2. Write the operation prefix plus unchanged literal UTF-8 as the literal stage.
3. Continue servicing child output, manual input and control traffic while a monotonic 500 ms timer runs from completion of the literal stage.
4. Revalidate identity, interaction state and absence of intervening human input, then write the separately measured submit-key stage.
5. Record accepted submission only after the entire submit-key stage is written.

Store the stages separately in memory; the current single buffer combining literal and submit bytes is insufficient. Ordinary output redraw caused by the literal is expected and must not make its own nonempty composer invalidate the pending submit stage. Candidate/new interactions, human input, ownership/placement changes or observer failure stop the remaining stages.

A positive short write, a failure after any child byte, or a timer-stage cancellation after a positive write produces `uncertain` with the input ID and last stage. Treat an Escape byte as a side effect too. A zero-byte transient write may retry only after fresh validation; an uncertain operation is reconciled before any new attempt. Keep the inclusive 65536-byte encoded input limit and existing control-frame limit.

### Delivery method and interaction priority

The following ordered rules apply to wrapped previews, `member prompt` and `member exec`:

1. Validate target/run identity, reachability, writer ownership, storage and receipt history. A failed invariant or an uncertain prior attempt is an explicit operational failure, not a reason to send fallback bytes; unknown detection coverage alone is valid capability absence, not an observer-health failure.
2. A candidate or open detected interaction queues ordinary input with zero bytes. Preserve an interaction while obscured and treat `permission_request`, `question`, `trust`, `update` and `unknown_interaction` as blocking interactions.
3. A verified empty composer with the operation's executable plan uses profile delivery without Escape. This includes a working member whose composer row is empty: the native CLI queues the submitted input behind its current turn without interrupting it.
4. When the operation has an executable plan but the composer is not verified empty (a draft is present or the observer cannot classify the composer), queue the input with zero bytes under defer reason `composer_busy`. The runner drains it with profile delivery as soon as it observes a verified empty composer. A queued head that has waited 3 minutes solely on a composer that is not verified empty is delivered by fallback with reason `idle_timeout`. The bound starts when the head first waits only on that composer and restarts whenever an interaction blocks it; time spent behind an interaction or earlier input never counts. A detector that stops matching a newer native UI therefore delays delivery rather than stranding it.
5. Otherwise use delivery fallback: Escape, settle 500 ms, literal prefix/text, settle 500 ms, Enter (`0d`). This covers missing operation plans, missing profile coverage and failed version probes.

The Director interrupts a working member explicitly with `cafleet member press MEMBER_ID Escape`; ordinary message delivery never sends Escape to a member whose profile has an executable plan before the 3-minute bound.

Fallback executes through the owned child PTY and the same input coordinator/receipts, rather than through a second multiplexer input writer. It preserves the caller's literal and the exec `! ` prefix. Revalidate the higher-priority interaction rule before Escape and before every later stage; a newly detected interaction before the first byte requeues the input, and one after a positive write stops the remaining stages with uncertainty.

The user accepts that fallback may interrupt work, mix text with a draft, and dismiss an undetected permission request or question on unknown-coverage runs. The guarantee is suppression for detected/candidate interactions, rather than detection of every native form. A changed native UI can therefore trigger ordinary-input fallback while its unsupported permission answer remains unavailable. Semantic permission answers continue to require their independently tested request-bound plans and never use this fallback.

### Runner-owned deferred delivery

The runner owns a FIFO of ordinary input requests, including preview requests. Accept a blocked request into the queue only after allocating its durable input ID and committing a `ready` receipt under its operation key, digest and target identity. Retain the literal/control payload in runner memory; receipt tables, result events and diagnostics retain metadata, not literal prompts or commands.

Use the existing unresolved-input limit of 4096 to bound queued plus active work. A full queue returns an explicit zero-byte capacity error. A matching repeated operation key returns its queued or recorded result; a different digest fails. Preview keys remain `preview:<run_id>:<message_id>` and bind the canonical message content under the versioned digest contract below, independently of the eventual delivery method. Explicit input keys remain stable within their existing coordinator transaction.

An `interaction_closed` transition schedules queue processing from the runner event loop after its event transaction commits. Drain pending output and revalidate the current state before selecting a method: a superseding interaction keeps the queue blocked, and closure due to exit or monitor loss dispatches nothing. Also retry a queued head when a candidate clears or the active input completes; every retry rechecks the current interaction state. No external `message send`, `message wait` or Director wake is required to resume the accepted queue.

Only one ordinary request writes at a time, in enqueue order. A request queued behind another receives the same explicit queued result. Permission responses and manual native input retain their separate paths and can resolve the interaction that blocks ordinary input. Closing the interaction grants no permission and supplies no native execution result.

Recheck that a queued preview's message remains unconsumed and its target remains active immediately before writing. An ACK or target removal cancels its zero-byte queued receipt as not submitted and advances the queue, rather than typing an already consumed preview.

Move ownership of the shared physical pane-input lock for wrapped writes into the runner. Wrapped clients validate and submit without retaining that lock while awaiting the runner; the runner acquires/revalidates the concrete placement lock for actual immediate or deferred stages. Release it while a request is merely queued, and retain it across an active staged write so explicit press, deletion and other programmatic writers serialize correctly. Acquire it nonblockingly from the event loop under an attempt deadline, avoiding a client/runner lock deadlock. The legacy unwrapped path retains its existing client-owned lock.

A queued request outlives the originating CLI connection and command deadline. Each actual drain attempt receives the existing five-second ordinary-input budget; waiting for an interaction is not an active write attempt and consumes no such budget. A client timeout after queue acceptance reports the known input ID and directs the caller to inspect the receipt; it is not an instruction to generate a new operation key.

The event loop owns a 100 ms retry timer for a queued head whose active attempt encounters a temporary zero-byte obstacle. Retry lock acquisition, storage readiness and observer health on that timer within the same five-second attempt deadline, even if no child output or broker command arrives. Timer retries preserve its input ID, FIFO position and `ready` receipt; they do not repeatedly renew the deadline. Reappearance of a candidate/open interaction pauses dispatch and ends the zero-byte attempt; the next closure/clearance starts another bounded attempt.

| Deferred-attempt condition | Receipt and queue outcome | Result publication |
|---|---|---|
| Busy pane lock or temporary storage/observer unavailability before the deadline, with zero bytes | Retain the queued head and retry on the 100 ms timer | Input inspection remains `queued` with its original defer reason |
| Five-second deadline expires with zero bytes | Settle `not_submitted`, clear the unused selected method, and advance after the terminal transaction commits | Emit `input_delivery_result` with reason `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`, according to the last obstacle |
| Target/run/placement identity fails revalidation or a queued preview is consumed, with zero bytes | Settle `not_submitted` immediately and advance after commit | Emit the result with reason `target_inactive`, `target_changed` or `message_consumed` and diagnostic context |
| Any attempt may have written a child byte | Record `uncertain`, stop automatic ordinary dispatch and retain its method/selection reason | Emit the uncertain result and require existing receipt reconciliation before advancing |

If storage prevents the terminal transaction, discard its zero-byte payload and retain a metadata-only pending finalization at the FIFO head. Continue retrying that transaction on the 100 ms timer without sending bytes or starting another input; commit the receipt when storage recovers, then advance; its result event follows from that durable receipt. Emit a runner-stderr diagnostic identifying the input ID and persistence failure while publication is unavailable; the Director receives the durable result after recovery. Ownership loss transfers finalization to the existing fenced recovery path, rather than allowing the old writer to update or dispatch again. Thus an accepted request becomes queued, submitted, not submitted or uncertain; `held` remains a result for work the runner did not accept into its queue.

On orderly shutdown, cancel queued zero-byte requests as `settled/not_submitted` with reason `runner_stopped`. After runner loss, recovery may settle a deferred `ready` receipt as not submitted with reason `runner_lost` once ownership fencing establishes that its in-memory payload/writer is gone. Explicit payloads are not restored from disk; a later deliberate submission is a new request. Preserve existing `in_progress`/`uncertain` reconciliation for any attempt that may have written bytes. A preview settled not submitted remains an unnotified broker message and may be explicitly retried under a new run's preview key.

### Delivery reporting

Append a `delivery` object with ordered `status,method,reason` to `message send`, `member prompt` and `member exec` JSON. Ordered top-level keys are `message,notification_sent,input_id,delivery` for send; `member_id,pane_id,text,input_id,delivery` for prompt; and `member_id,pane_id,coding_agent,command,dispatched,input_id,delivery` for exec. Send's input ID is newly exposed and nullable; prompt/exec retain their existing input-ID fields. `method` is `profile`, `fallback`, `unwrapped` or null before selection. The shared result projection is:

| Status | Meaning | CLI behavior |
|---|---|---|
| `submitted` | Complete literal and submit stages were written | Exit 0; send has `notification_sent=true`, exec has `dispatched=true` |
| `queued` | Runner accepted durable zero-byte deferred work | Exit 0; send has `notification_sent=false`, exec has `dispatched=false`; return the input ID and reason |
| `held` | No queue acceptance or write, due to unavailable health/ownership/storage | Send preserves its committed message and prints the reason; explicit prompt/exec exits 1 with the existing delivery context |
| `uncertain` | Some stage may have written; reconciliation is required | Preserve existing error categories, input identity and stage diagnostics, including the selected method |
| `not_submitted` | Work was cancelled with established zero-byte evidence | Visible through input inspection and, for deferred work, its result notification |
| `not_applicable` | A self-send or root Director target uses broker consumption without automatic pane delivery | Send retains `notification_sent=false`, null method/ID and no warning |

Evaluate self-send before recipient delivery: when sender and recipient IDs are equal, return `delivery={status:"not_applicable",method:null,reason:"self_send"}` and null `input_id`, including for an ordinary member with a healthy wrapped pane. Self-sends persist only in the broker and remain excluded from automatic preview selection and the runner queue. Other sends to a root Director use the same status/null fields with reason `director_stdout`; self-sends to the Director use `self_send` by the preceding rule.

Keep existing JSON keys and their relative order, then append the new fields. For a send, the delivery projection describes the newly sent message, even if the same invocation also retries an older pending message. A queued preview is acknowledged only as queued: `notified_at` stays null until actual submission or the existing ACK settlement.

Text output adds `Delivery: status=STATUS method=METHOD reason=REASON`, using `-` for a missing method/reason, and `Input: INPUT_ID` when allocated. Preserve the existing submitted-success lines only for actual submissions; queued prompt/exec instead prints `Queued prompt for member ID (PANE).` or `Queued native command for member ID (PANE).` A fallback submission always prints `method=fallback` with reason `submission_plan_unavailable` or `idle_timeout`; unknown-profile/probe cases use the former. A queued result names `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input` and states that the runner will retry.

Keep existing preview body folding, inline limits and long-body message-show pointers. Profile delivery uses the existing wrapped preview's resume clause; fallback delivery uses the existing Escape-dismissal clause as well, so a member can attribute an undetected prompt's delivery-caused rejection correctly. Derive rendering from the selected delivery method before its first byte, and update the member/overlay explanation of this distinction.

For new preview receipts, use payload digest version `2`. The preview client sends the canonical base preview in `SubmitInput.literal`: the existing method-independent wrapped preview, including its resume clause and excluding the Escape-dismissal clause. The runner reloads the immutable broker message, validates its run/recipient/key relationship, reconstructs that same canonical base and requires byte equality before reserving or reusing the receipt. Compute SHA-256 over the existing canonical JSON encoding of this object:

```json
{"payload_digest_version":2,"run_id":42,"kind":"preview","message_id":99,"literal":"CANONICAL_BASE_PREVIEW"}
```

The runner renders actual wire text from this validated canonical base only after choosing a delivery method: profile delivery uses it unchanged; fallback appends the existing Escape-dismissal clause. Method, appended clause, native prefix/key bytes and timing are outside the digest preimage. Select/render afresh only while zero bytes are established; once a stage writes, pin its method/rendered bytes through submission or uncertainty. Queries and duplicate requests validate canonical identity before considering current composer state, so a later method change cannot create a different payload digest or authorize retyping an existing input.

Persist `payload_digest_version` alongside each input digest. V7 backfills every existing receipt to `1` and leaves its digest bytes unchanged; new explicit ordinary inputs and permission-response inputs retain their existing version-1, kind-specific algorithms. Validate existing preview receipts using the frozen pre-change wrapped-preview renderer and the original preimage `run_id,kind,message_id,literal`, without a version field or a method-dependent clause. Read an existing receipt's version before computing an expected digest; choose version `2` only when allocating a new preview receipt, atomically with reservation. Unsupported versions or a version/kind mismatch fail explicitly. This is stored request identity, separate from any native profile version.

An asynchronous completion cannot rewrite the originating CLI's output. Persist its actual method in the input receipt, expose it through `input show`, and emit one `input_delivery_result` runtime event to the run's Director for a deferred input's first submitted/not-submitted/uncertain result. The event has the standard version-2 envelope, appends ordered `input_id,message_id,delivery,stage` immediately before its null `director_action`, and has null `interaction_id` and `classification`; `message_id` is null for explicit input. Its `stage` uses the existing `zero_bytes,partial,submitted,unknown` domain and it carries no literal payload. Queue output points to `cafleet input show INPUT_ID --json` for the eventual result, and its method stays null until an attempt actually selects it.

The terminal receipt (with any preview notification stamp) is the durable source of the deferred-result event: the live runner publishes the event after the receipt commits, and recovery publishes any durable first result that has no event yet, so a crash or ownership loss between the two never loses a result. Deduplicate result publication by run/input identity; orderly shutdown publishes every pending result before `runner_exited`. Existing event listen/poll delivers the result without injecting anything into the Director's pane. For immediate submissions, the command output and queryable receipt supply the result.

### Storage and current interfaces

Add the contiguous `V7__minimum_version_delivery.sql` migration in the existing database family. Preserve the application identity, instance identity, existing rows and sequences, foreign keys, audit relationships and previous migration files.

- Drop `member_runs.profile_match` and its column-local CHECK with `ALTER TABLE member_runs DROP COLUMN profile_match`; update schema shape expectations and all current run readers/writers.
- Add nullable `delivery_method`, `delivery_reason`, `deferred_at` and `defer_reason` to `member_input_operations`. Method is `profile` or `fallback` for ordinary wrapped operations; deferred timestamp/reason are present together, and deferred work has ordinary input kind. Existing rows retain null metadata because those facts were not recorded.
- Add `payload_digest_version INTEGER NOT NULL DEFAULT 1`, constrain it to `1` or `2`, and allow `2` only for preview inputs. Existing rows retain their exact version-1 digest; all new preview reservations explicitly choose `2`.
- Use `ready` for committed deferred zero-byte inputs and include deferred ready IDs in `unresolved_input_ids`. Wire replies add `queued` as a distinct submission status, preserve zero-byte stage evidence, and carry the delivery projection; input inspection preserves its existing ordered fields and appends `delivery,deferred_at,defer_reason,payload_digest_version` after `permission_response`.
- Extend the runtime-event kind CHECK with `input_delivery_result`, preserving the existing event/permission-response/claim dependency rebuild order and sequences as in V6. Validate the new event envelope explicitly and preserve permission-response semantics.

`defer_reason` records why a request first entered the queue and remains paired with its original `deferred_at`. `delivery_reason` records the eventual selected-method reason or a terminal zero-byte cancellation reason. In the same owned transaction that changes an ordinary receipt to `in_progress`, commit its selected `delivery_method` and `delivery_reason` before the first byte: profile delivery has a null reason; fallback has `submission_plan_unavailable` or `idle_timeout`. An immediate fallback records these fields too. A zero-byte requeue clears unexecuted selection metadata before any later reselection; a terminal zero-byte cancellation commits null method and its outcome reason. After any possible positive write, preserve the selected method/reason through accepted, uncertain and settled states.

All result consumers project these durable fields rather than recomputing a reason from the current catalog/composer. Use the original `defer_reason` while status is queued, the stored fallback-selection reason for an attempted fallback, and the stored outcome reason for a zero-byte cancellation. An uncertain event may additionally carry the existing diagnostic `cause`, without overwriting the method-selection reason. Receipt outcome, eventual method/reason, deferred-result event and preview notification settlement are committed consistently.

| Ordinary receipt state/disposition | Delivery status | Method and reason |
|---|---|---|
| `ready` with deferred metadata | `queued` | Null method; original `defer_reason` |
| `ready` without deferred metadata | `held` | No active queue ownership; null method and `zero_bytes_ready` |
| `in_progress` or `uncertain` | `uncertain` | Recorded selection; no automatic dispatch/replay |
| `accepted` or `settled/submitted` | `submitted` | Recorded selection |
| `settled/not_submitted` | `not_submitted` | Null method and recorded zero-byte outcome reason for a zero-byte cancellation; recorded selection for a reconciled attempt that may have written bytes |
| `settled/abandoned_uncertain` | `uncertain` | Preserve recorded selection; abandonment ends retries without establishing zero bytes |

For ordinary receipts whose method/reason were unrecorded, including pre-V7 history, determine status from the same state/disposition mapping and retain null method. A `ready` row without deferred metadata uses `zero_bytes_ready`; other such rows expose `method_unrecorded` instead of inventing a delivery method or cancellation reason. Missing deferred metadata establishes no queue ownership. Permission-response receipts expose `delivery:null`, null deferred metadata and digest version `1`; their existing semantic decision/submission/settlement projection remains authoritative. Unknown stored combinations fail validation. In particular, method absence on a historical receipt or a permission response is a specified stored state, not an unresolved future method choice.

Remove the `ProfileMatch` enum and every field/copy/assertion that propagates it through launch identity, run settings, records, deletion, observation fencing and presentation. Runtime JSON retains its remaining ordered keys with `coverage` followed by `current_interaction_id`; text becomes `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE review=STATE`. Retain actual native/profile identities in the existing event/evidence/inspection surfaces.

Historical migration SQL and tests validating those immutable migration steps retain their required definitions. Current schema documentation describes the resulting V7 shape, and historical design documents retain their original record. Current code, help, living examples and skills describe only minimum-version profiles and the separately named delivery fallback; remove exact/fallback profile-match wording and its warning completely from those surfaces.

### Documentation and acceptance

Update documentation before implementation, in repository order: `SPEC.md`, affected site pages, `README.md` if its entry-point claims change, affected skills, then project rules only if their conventions change. Cover native probe/selection, executable plans and timings, delivery arbitration, queue lifecycle, result/event output, storage and acceptance. The affected agent references include `skills/cafleet/reference/cli.md`, `member-runtime.md` and the member/overlay guidance describing Escape provenance and retry ownership.

Describe measured native evidence as the provenance of a profile interval, rather than an exact-version admission requirement. Preserve independent operation coverage, privacy/source validation and native/managed authorization boundaries. Skill references contain the facts agents need without links to the site or `SPEC.md`; user site pages describe behavior rather than source-module paths.

Acceptance covers every bundled backend on each registered multiplexer. Use isolated native sessions/databases and benign marker commands; retain the executing fleet's installed runtime/assets until its members are retired. Collect native fixtures and stage/result records under this task's hidden audit directory, promote only privacy-reviewed native fixtures into the embedded production registry, and place Rust tests in each exercised implementation file's inline test module.

| Case | Required evidence |
|---|---|
| Threshold selection | Equal/between/above/below boundaries, catalog permutation, multiple backends, numeric component ordering, prerelease ordering, build-metadata ties, malformed thresholds and failed probes |
| Complete profile reuse | Detection, annotations, both composer operations and each available semantic answer resolve the same profile and retain actual run/capture identities |
| Native operation plans | Prompt preview and direct prompt create a native user turn; native exec runs a harmless marker command and exposes its result, with literal-to-submit timing recorded |
| Fallback operation | Missing plan, unverified composer, draft/working state and failed-probe cases show Escape, 500 ms settle, literal, 500 ms settle, Enter and explicit fallback results |
| Interaction deferral | Permission and question remain open with no Escape/literal bytes; answering on the original native surface yields closure and exactly one queued delivery without another send/wait |
| Adversarial transition | Candidate/obscured/superseding interaction, human input during either settle, observer/owner loss and partial stage writes preserve no-write or uncertain outcomes |
| Durable retry | A queued canonical preview drains by fallback and retains one ID/digest after composer changes and repeated queries; duplicate keys, conflicting digests, concurrent sender/drain, lost replies and stamp failures avoid duplicate dispatch |
| Deferred failure and liveness | After closure with a busy pane lock and no further output/commands, the runner timer either delivers when it becomes free or publishes not submitted at the deadline; temporary storage/observer failure and terminal-commit recovery preserve FIFO/result visibility |
| Migration and reporting | Fresh/head/previous-head upgrade, unchanged version-1 digests, referential integrity, sequence preservation, self-send/no-pane output and durable reason/state projection for ordinary/history/permission-response receipts |

Reproduce the original Claude `2.1.308`/herdr case explicitly: create without a mismatch warning, receive a real `[cafleet msg ...]` preview, deliver a direct prompt, execute a marker command, and verify that the reported permission detector and allow answer still work. Also repeat direct prompt delivery on Codex `0.163.0` to cover the exact-version missing-plan reproduction. Measurements establish submitted native input separately from model consumption, permission decisions and command completion.

---

## Implementation

> When completing a task, check its box and record a UTC timestamp in the same edit.

### Step 1: Specify contracts and native measurement procedure

- [x] Update `SPEC.md` profile/version, probe, delivery arbitration, stage timing, queue/result/event, output and V7 storage contracts to match this design. <!-- completed: 2026-10-30T10:35Z -->
- [x] Update affected site concepts/contributor guidance and any changed README claim, then update agent CLI/runtime/member/overlay references; remove exact-profile matching requirements and document delivery fallback and runner-owned retry. <!-- completed: 2026-10-30T10:35Z -->
- [x] Prepare isolated per-backend/per-multiplexer native measurement cases under the task's hidden audit directory for prompt, preview, native exec, interaction deferral and staged timing. <!-- completed: 2026-10-30T10:35Z -->

### Step 2: Implement minimum-version catalogs

- [x] Add the SemVer dependency/shared parser, strict profile thresholds and fixture interval validation; convert bundled and synthetic profile definitions to `min_version` while retaining capture versions. <!-- completed: 2026-10-30T11:00Z -->
- [x] Replace exact/newest selection with greatest eligible threshold, make native launch identity fallible for known below-minimum versions before registration, and preserve failed-probe unknown-coverage launch. <!-- completed: 2026-10-30T11:00Z -->
- [x] Route detection, annotations, both composer operations, answers and runtime profile fencing through the selected profile; add boundary, provenance, parser and capability-reuse unit coverage. <!-- completed: 2026-10-30T11:00Z -->

### Step 3: Migrate receipts, events and match metadata

- [x] Add `V7__minimum_version_delivery.sql` to remove the match column, add ordinary-delivery reasons/deferred metadata and digest versions, and admit the deferred-result event while preserving all existing rows and digest bytes. <!-- completed: 2026-10-30T11:00Z -->
- [x] Remove match metadata from current Rust run/launch/settings/observation/deletion/presentation paths; update schema guards, run queries and unresolved-input projection. <!-- completed: 2026-10-30T11:00Z -->
- [x] Add queued/result wire validation, durable reason/state projection and deferred-result event validation/publication, with previous-head, historical/permission-response projection and integrity tests. <!-- completed: 2026-10-30T11:27Z -->

### Step 4: Execute staged profile and fallback delivery

- [x] Refactor composer plans and the input coordinator to separate literal and submit stages with an event-loop-driven 500 ms settle; retain stage/checkpoint/receipt uncertainty across partial writes. <!-- completed: 2026-10-30T11:58Z -->
- [x] Implement Escape/settle/literal/settle/Enter fallback for ordinary wrapped input, including unknown coverage, with higher-priority detected-interaction suppression before every stage. <!-- completed: 2026-10-30T11:58Z -->
- [x] Transfer actual wrapped pane-lock ownership to the runner without holding client locks across its replies; test timing, exclusion, human/state races and explicit fallback selection. <!-- completed: 2026-10-30T11:58Z -->

### Step 5: Queue and resume ordinary input in the runner

- [x] Add bounded FIFO admission with durable ready receipts, memory-only literals, versioned canonical-preview digests and explicit zero-byte queue results for previews, prompts and exec. <!-- completed: 2026-10-30T12:20Z -->
- [x] Drain after closure/candidate clearance/prior-input completion, add timed retry within a fixed attempt deadline, and commit cancellation/finalization outcomes before FIFO advancement. <!-- completed: 2026-10-30T12:20Z -->
- [x] Implement shutdown/loss treatment and deferred-result publication; test no-output busy-lock liveness, temporary-failure finalization, CLI disconnection, cancellation, duplicate retries and recovery without replay. <!-- completed: 2026-10-30T12:20Z -->
- [x] Queue planned input for a busy composer under `composer_busy`, drain it with profile delivery on a verified empty composer, and deliver it by fallback with `idle_timeout` after 3 minutes of busy-only waiting; test both outcomes and that no Escape is sent before the bound. <!-- completed: 2026-10-30T12:48Z -->

### Step 6: Bundle measured operation plans

- [x] Measure prompt and native-exec input for each bundled profile on a native version in its interval, capturing separate literal/submit stages and successful native results on every registered multiplexer. <!-- completed: 2026-10-30T11:25Z -->
- [x] Add reviewed native fixtures, catalog digests/embedding entries and both operation definitions to each bundled profile; preserve existing detector, annotation and permission-answer plans. <!-- completed: 2026-10-30T12:40Z -->
- [x] Replace bundled unknown-submission expectations with validated operation coverage, and replay positive/adversarial fixtures against the production catalog. <!-- completed: 2026-10-30T12:45Z -->

### Step 7: Expose delivery outcomes and synchronize guidance

- [x] Add ordered delivery projections and input IDs to send/prompt/exec output, preserve submitted-vs-queued semantics, and surface actual fallback method/reason in immediate results and deferred receipts/events. <!-- completed: 2026-10-30T12:32Z -->
- [x] Integrate canonical client requests, runner-owned method-specific preview rendering and versioned receipt lookup; test queued fallback followed by composer changes and retries with one input ID and no duplicate typing. <!-- completed: 2026-10-30T12:32Z -->
- [x] Test output contracts, self-send/root-Director/no-pane semantics and deferred-result reception; sweep living documentation, skills, help and code for stale match metadata, no-Escape claims and external-only retry instructions. <!-- completed: 2026-10-30T12:32Z -->

### Step 8: Validate native behavior and the complete change

- [x] Route every non-verified-empty composer with an executable plan to `composer_busy` (removing the `composer_unverified` fallback reason), set the `idle_timeout` bound to 3 minutes counted from busy onset, and verify natively that a human draft receives zero bytes until it clears. <!-- completed: 2026-10-30T13:27Z -->
- [x] Make Claude line matchers tolerate trailing padding cells left by partial repaints (anchor with `\s*$`, trim padding from the captured operation), add a stream-derived production fixture with a padded question row, and verify detection plus allow on herdr and tmux 200x35. <!-- completed: 2026-10-30T14:04Z -->
- [x] Complete the native acceptance matrix, including Claude `2.1.308` on herdr and Codex `0.163.0`, with explicit interaction preservation, automatic retry, fallback reporting and observed execution evidence. <!-- completed: 2026-10-30T14:04Z -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build`; address failures with tests in their owning implementation modules. <!-- completed: 2026-10-30T14:11Z -->
- [x] Verify migration/idempotence/privacy and repository-wide documentation consistency, record acceptance evidence and remaining measured limits, and have the Director review the final diff while preserving the active fleet's installed runtime. <!-- completed: 2026-10-30T20:10Z -->
