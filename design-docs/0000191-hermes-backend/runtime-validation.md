# Hermes Runtime Compatibility Evidence

Date: 2026-10-25. Scope: implementation Step 1 of `design-doc.md`.

## Decision and evidence boundary

M4 fails the tested fresh-conversation, normal/manual contract on package `0.22.4`: profile-scoped deletion exited `0`, but both regular and persistent owned writers kept progressing before their deadlines and before cleanup stop flags. The profile name also acquired a new directory incarnation after deletion. Keep dependent production implementation and acceptance gated on a reviewed settlement revision; continue evidence preservation, owned-resource cleanup and separately authorized independent work. The user subsequently requested full execution, and the Director assigned settlement design plus D1-D2 source/probe preparation before that revision. Both writers subsequently exited through explicit cooperative cleanup, which does not establish native teardown success.

User relay requires normal command approvals and fresh conversations. Model remains `openai/gpt-6.1-sol`, provider `nous` and reasoning `high`. M1 and native-output/zero-model-turn M3 subchecks pass within their recorded scope. Guarded approval/refusal, full broker/push behavior and remaining M4/D1/D2 scenarios remain unrun. The user subsequently selected native dangerous-command deny rules and `approvals.mode=smart`, resolving the earlier proposed positive-default-deny requirement; static matcher and mode assessments are independent of the failed M4 gate. Preserve the manual-mode runtime observations as historical policy evidence. Smart-mode runtime acceptance remains unrun. No hook or bypass is selected.

The previous named-continuation contract failed M1 on installed package `0.22.4`: both fresh owned profiles returned a creation/resume-policy refusal before a first assistant answer, with shell-recorded exit code `2`. Its commands, source cause, captures and cleanup records remain historical evidence below. That result supplies neither a pass nor a failure for the revised fresh-conversation contract.

Ordinary classic launch uses `HermesCLI` with `GatewayAgent`; bypass/stream paths use `GatewayChatView`. The earlier isolated transport-branch failure applies only to that branch. Current verification uses ordinary `hermes chat --cli` and observes each gate independently; stop dependent work at a concrete current-contract failure.

The Verifier inspected installed source and official documentation. The Director owns Hermes CLI, real-TTY, profile, gateway, and paid model probes. User authorization permits temporary owned profiles and paid probes with `openai/gpt-6.1-sol`, while preserving the configured provider and existing profiles. No Verifier action has launched Hermes, evaluated user credentials, or changed a profile.

## Final native-flow compatibility result

Status: INCOMPATIBLE with the complete currently selected source account/provider inheritance, unrelated-session preservation and isolated managed-settlement contract on installed `hermes-agent/0.22.4`. Full bounded evidence is `.hermes-spike/settlement/native-auth-compatibility.md`; `native-auth-compatibility-source-digests.json` pins inspected source and the current redacted observation. This is a scoped compatibility result, not a claim that every Hermes deployment or static-key configuration is unsupported.

Director's `current-native-auth-observation.json` at 2026-10-25T22:01 UTC confirms selected provider `nous`, the prior `base_url/default/provider` model keys, no static-key configuration in the selected model or `providers.nous`, and a root Nous refresh grant. No secret/account/token value was emitted and no mutation occurred. The observation covers those selected mappings and grant presence; it makes no global absence claim about unrelated environment keys or unselected providers. No already configured compatible static Nous alternative is established by the authorized evidence.

Native same-root empty-child OAuth fallback and originating-store write-through exist (`hermes_cli/auth.py:505-548,826-861,895-907,948-972`); normal native refresh is permitted, while actual M2 identity/refresh acceptance remains unrun. The same-root runtime still has the original M4 settlement failure and independently confirmed foreign cron/configuration effects: scheduler startup recovery/heartbeats precede its per-cycle owner gate (`cron/scheduler_provider.py:534-610`; `cron/jobs.py:630-636,1240-1264`). Moving to a private root also moves the native OAuth fallback, losing the original grant. Stock Nous shared import changes the credential lifecycle, swallows mirror failures and can fall through to enrollment; native refresh/status supplies no separate-root reference to the canonical originating store. The custom grant/controller construction is rejected and closed before implementation/live actions.

The missing native capability is an isolated runtime consuming the original canonical grant through a supported independent reference with native lock-protected continuity and surfaced outcomes, or a same-root own-profile scheduler/configuration domain together with the complete fenced managed-inventory/stop/join settlement phase/receipt. CAFleet implements no OAuth manager to replace it. Keep dependent Hermes implementation/acceptance gated on supported native capability or reviewed build/interface change; approved independent catalog/global-policy work remains valid. M4 remains FAIL; all private-root/auth and D1/D2 live cases remain NOT RUN. No new native, credential, model, profile, process or installed/source-setting operation occurred in this review.

## Private custom-root and authentication assessment: source only

Disposition: the following source-preparation alternatives are closed by the Director's rejection of the custom shared-grant/controller construction. The final native-only result above governs; these artifacts supply no implementation handoff or runtime pass.

The 2026-10-26 local source follow-up is `.hermes-spike/settlement/private-root-assessment.md`, with installed-source integrity witnesses in `private-root-source-digests.json`. A canonical custom `HERMES_HOME` physically outside the native root changes root/profile/cron/config discovery (`hermes_constants.py:93-101,296-308`; `hermes_cli/profiles.py:283-297,1136-1161`). A path under the native root still belongs to that root. The candidate must own its entire root roster, runtime producer and attachment domain; a private directory supplies neither OS process containment nor a complete managed settlement receipt.

Host rendezvous is a separate OS-user domain (`gateway/status.py:321-337`; `host_rendezvous.py:163-184`). Native root-aware attach/lock contention distinguishes tenants, and the per-home protocol checks served canonical home/instance/epoch and mints a matching ticket (`host_attach.py:116-130,362-374`; `run.py:4919-4925`; `hermes_cli/gateway_runtime.py:98-152`; `gateway_client.py:225-258`). The native session listener is loopback with a dynamically assigned port (`gateway/run_api.py:138-178`), distinct from optional platform listeners. Effective root/rendezvous/control/listener identity and child propagation remain live requirements.

Generic OAuth fallback follows the new private root rather than the original grant (`hermes_cli/auth.py:505-548,826-861,948-972`). A concrete Nous-specific interface exists: native `HERMES_SHARED_AUTH_DIR` plus `hermes auth add nous --type oauth` imports from `nous_auth.json` (`auth_nous.py:349-380`; `auth_commands.py:307-348`). Import locks, force-refreshes and writes that shared store (`auth_nous.py:587-618`); runtime also mirrors refreshed state, with best-effort swallowed write failures (`451-474,948-971`). Normal native refresh/writeback under Hermes locks is explicitly permitted by the normative Preparation and Launch section; absolute original-store immutability was an incorrect premise and is corrected in the current assessment. Cross-root import changes the selected same-root fallback interface, so the relevant gates are strict import-only behavior, source account/provider continuity and durable shared generation with private freshest-state retention on uncertainty. Copying/linking rotating grants supplies no accepted solution. Static-key input or a separately enrolled private OAuth grant is a different conditional scope; the study inspected no credential availability and grants no replacement for the accepted Nous/model route. The bounded source-only candidate and stock CLI enrollment-fallback, callback, durability and ongoing-runtime limitations are in `.hermes-spike/settlement/native-import-only-assessment.md`; source witnesses are native-import-source-digests.json. Director review must specify that changed native integration plus the independent fence/inventory/stop/join/recovery sequence before dependent implementation or live cases. M4 remains FAIL; all private-root/auth, D1 and D2 live cases remain NOT RUN. No Hermes import, credential read, root/profile creation, model/network inference, environment/runtime edit or process control was performed.

Official context: [Profiles](https://hermes-agent.nousresearch.com/docs/user-guide/profiles), [Session Storage](https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage), [Running Many Gateways](https://hermes-agent.nousresearch.com/docs/user-guide/multi-profile-gateways), and [Environment Variables](https://hermes-agent.nousresearch.com/docs/reference/environment-variables). Installed source determines the precise accepted-build findings; directory overrides for shared auth and gateway locks were absent from the inspected environment reference.

## Automatic approval mode assessment: read-only

The revised shared command contract permits member task commands under the host's existing permissions and routes an observed permission denial, unavailable automatic mode or unresolved approval to the Director. Route the exact command, cwd, environment overrides and actual refusal or pending reason. Preserve native and managed authority when reviewing that evidence. An ordinary command failure remains an ordinary error-recovery case. This assessment ran source reads and official-documentation retrieval only; it started no coding-agent CLI, profile or model session and changed no runtime policy.

### Hermes smart: installed decision boundaries

The following findings apply to installed package `0.22.4`, using source root `P` defined below. The official [Security guide](https://hermes-agent.nousresearch.com/docs/user-guide/security), inspected 2026-10-25, describes auxiliary review with manual escalation for uncertainty. Installed source adds an important distinction between a guardian verdict and an unconditional deny rule.

| Boundary | Observed installed source | Required interpretation |
|---|---|---|
| Explicit deny and mandatory guards | `tools/approval.py:1058-1092,1151-1194` applies the guard floor before mode bypass, permanent allowlist, dangerous-pattern detection or smart review. | Preserve the approved concrete `approvals.deny` entries. A smart approval or allowlist match supplies no exception to a matching floor denial. |
| Scope of automatic review | `tools/approval.py:1170-1194` reaches smart review for a detected dangerous command after existing approval/allowlist checks. `tools/approval_smart.py:88-146` requests one auxiliary verdict. | Smart is conditional risk assessment; it is not review of every command or a positive-default-deny map. |
| APPROVE | `tools/approval.py:764-780` grants the assessed operation and resets the guardian-denial counter. | The guardian approval persists no pattern-level permission. |
| DENY with an interactive owner | `tools/approval.py:781-833,857-865` continues to the human surface with `smart_denied=true`; `tools/approval_prompt.py:118-129,156-174` offers once/deny. The grant path suppresses persistence even for an older callback returning session/always. | Guardian DENY can require a one-operation owner decision. Keep this distinct from an unconditional configured deny; relay the actual decision surface to the Director without treating routing as permission. |
| Uncertain or failed assessment | `tools/approval_smart.py:128-146` maps ESCALATE, an unrecognized/empty response or an exception to escalation. `tools/approval.py:781-814` then enters normal human approval. | A failed guardian call supplies no automatic approval. Normal human choices may persist according to their selected scope; preserve the actual choice and resulting policy. |
| Pending, timeout and cancellation | `tools/approval.py:840-930` waits on a registered gateway notifier, falls back to a usable CLI surface, or returns `approved=false`, `status=pending_approval`, `approval_pending=true` through `:567-602`. Gateway wait is bounded and interruptible (`tools/approval_gateway_wait.py:48-77`). Explicit denial, timeout, notification failure and an unanswered/withdrawn prompt return refusal. | Preserve pending state and request correlation; settle the original approval rather than duplicate the command. Report cancellation as unanswered/withdrawn when that is the observed cause, and attribute a user denial only to an actual decision. |

Noninteractive contexts use their configured unattended/single-query/cron handling before the interactive smart path (`tools/approval.py:933-965,1175-1184`). Their behavior establishes no equivalence to the selected real-TTY launch. The ordinary native bang path calls the terminal guard (`hermes_cli/bang_shell.py:71-86`); Director dispatch retains that native decision boundary.

Smart calls `call_llm(task="approval", ...)` with a configured task timeout and its own reasoning override (`tools/approval_smart.py:75-125`). Auxiliary automatic routing initially selects the main runtime provider/model, while per-task overrides and configured fallback routing can select another route (`agent/auxiliary_client.py:4553-4577,4666-4697`). The main `--model` flag alone therefore establishes no exclusive approval-model route or billing result. Any future authorized smart probe must preserve a redacted effective auxiliary route and failure outcome before claiming the requested model/provider constraint. Existing manual snapshots demonstrate neither smart execution nor its effective auxiliary route.

### Claude auto: official contract and remaining verification

Official documentation prescribes `--permission-mode auto`. Availability depends on supported model/provider, installed version, settings and server eligibility; an unavailable mode starts in Manual. Explicit ask rules and hook-forced prompts remain human checkpoints. Repeated classifier blocks can pause auto and restore prompting (three consecutive or twenty total). A server that does not review the session triggers client-side classifier fallback; a missing verdict for an action denies that action. Classifier requests may use a separate model. These are documented constraints, not observations of this fleet's installed Claude session. Source: [Permission modes](https://code.claude.com/docs/en/permission-modes), inspected 2026-10-25.

Permission rules resolve deny, then ask, then allow; a narrower allow does not override deny or ask. Prompt instructions shape attempted actions while native rules enforce access. Source: [Configure permissions](https://code.claude.com/docs/en/permissions), inspected 2026-10-25.

For the revised shared task, verify requested argv and prompt assembly with inert tests, and report unavailable mode or pending approval as an unresolved runtime condition. Keep the host's native decision and managed policy authoritative. Live effective Claude auto availability, actual classifier behavior, Hermes smart approval/refusal, and smart auxiliary routing remain unverified; this source assessment grants no E2E pass and does not reopen M4.

## Environment

| Field | Evidence |
|---|---|
| Executable | `/opt/homebrew/bin/hermes` is a symlink to `../Cellar/hermes-agent/0.22.4/bin/hermes`; observed with `ls -l`. |
| Package | Homebrew directory `0.22.4`; wrapper pins `HERMES_REVISION=v0.22.4`. These labels do not establish a supported version floor. |
| Reported CLI build | Director reports `Hermes Agent vunknown (2026.9.24)` from `hermes --version`; not independently executed by the Verifier. |
| Interpreter / SDK | Director reports Python `3.14.9` and OpenAI SDK `2.24.0`. The Director also reports that the packaged interpreter provides `hermes_yaml`, while `import yaml` fails. Use the accepted packaged YAML module in the disposable spike. |
| Installed source root | `/opt/homebrew/Cellar/hermes-agent/0.22.4/libexec/lib/python3.14/site-packages` (abbreviated `P` below). |
| OS | `uname -srm` returned `Darwin 27.0.0 arm64`. |
| Intended interface | Ordinary `hermes chat --cli`, with the design's normative argv; omit stream JSON and user-config bypass flags. |
| Source provider / model | Director's redacted source-home inspection reports a model mapping with `base_url`, `default`, and `provider`, configured provider `nous`, auth `active_provider`/provider `nous`, and no `.env`. Both attempted launches requested `openai/gpt-6.1-sol` and reasoning `high`, preserving provider `nous`. No first model answer or effective model reasoning/tool loop was observed. |
| Multiplexer | Director reports Herdr `0.9.5`; primary used `w2Z:p32`, sentinel used `w2Z:p33`. The Verifier read Director-authorized durable ANSI captures from both real panes. |
| Owned probe resources | After recovery of the initial unpublished stage, both fresh profiles were published and launched. Both failed M1, then their profile-scoped deletions exited `0`. The Verifier confirmed both active paths absent. Director reports both dedicated probe panes closed after captures. No owned writer PID records were present; usage/billing was not measured. |

## Gate matrix

This matrix records the observed fresh-conversation, normal/manual attempt and the remaining live evidence. `NOT RUN` means the required live evidence is absent. Source support and historical attempt results are prerequisite observations, not runtime passes for the subsequently selected smart policy. The partial first-response observations from the terminated bypass-mode attempt establish no normal-approval acceptance.

| Gate | Required acceptance evidence | Current result | Evidence / remaining work |
|---|---|---|---|
| M1: Interactive launch | Literal substituted first prompt on a real multiplexer TTY, subsequent messages, and effective model/reasoning. | PARTIAL: observed compatibility subchecks pass | Both real panes preserve exact first-query bytes and expected first replies; primary answers its second prompt, and sentinel answers its post-primary-delete prompt. Frozen model/provider/reasoning and full session IDs are preserved below. No production launch or full-role prompt acceptance is inferred. |
| M2: Host context and broker | Correct local cwd, executable/multiplexer access, explicit alternate database, full poll/show/ACK, and already-running gateway adoption of completed configuration. | NOT RUN: frozen-policy subchecks observed | Both frozen policies show local terminal, intended cwd, distinct broker paths, pane IDs and manual approvals. Actual tool cwd/executable/broker access, gateway reuse, refresh behavior and approval decisions remain unrun. |
| M3: Pane input and native commands | Escape-first push/prompt, local native execution with output/exit status, zero model turn, and preserved dispatch uncertainty. | PARTIAL: native output / nonzero exit / unchanged counters observed | Primary native `printf` displayed its marker, `false` displayed exit 1, and paired before/after snapshots remained 4 messages / 2 API calls / 0 tool calls. Director reports Escape-first submission. Guarded approval/refusal, full broker/push checks and uncertain-dispatch cases remain unrun. |
| M4: Isolation and teardown | Concurrent profiles, pane loss, active/queued/background execution, publication/crash/race recovery, bounded cleanup, and unrelated sentinel survival. | FAIL | Actual profile deletion exits 0 while both same owned writers continue advancing before deadlines and before cooperative stops; a new directory incarnation reappears at the profile name. Later explicit stop flags settle the writers as experiment cleanup. Remaining M4 cases stay unrun. |
| D1: Native receive | Exact terminal/process tool schemas, one bounded handle, message wake, renewal/cancel/join, full reconciliation, duplicate tolerance, ACK. | NOT RUN | Installed schemas advertise `terminal` and `process_manage`, with `background=true, notify=true` for finite background completion. Live exposure, wake, cancel/join, and renewal remain unobserved. |
| D2: Host continuation | Delegation across turns and compaction, existing standing-goal authority/budget preservation, and user pause/cancel behavior. | NOT RUN | Preserve the existing goal's state and budget during authorized probes. |

## Observed fresh-conversation attempt under manual approvals

The observed manual attempt root is `.hermes-spike/normal-20261025T115000Z/`, with fresh profile identities and audit records. Primary used Herdr pane `w2Z:p36`; sentinel used `w2Z:p37`; both are now closed as recorded below. Preserve both earlier attempt roots unchanged. The terminated `.hermes-spike/fresh-20261025T113600Z/` attempt is historical partial evidence below. Neither attempt verifies the subsequently selected smart policy.

The current normative child argv is:

```text
hermes --profile <fresh-owned-profile-name> chat --cli
  --in <absolute-cwd>
  --source tool --pass-session-id
  [--model <verbatim-model>] [--reasoning <validated-level>]
  --query <single literal substituted prompt argument>
```

The source root remains explicitly pinned through `HERMES_HOME`. For the authorized spike, model is `openai/gpt-6.1-sol`, provider remains `nous`, and explicit reasoning remains part of M1 verification. Preserve normal approvals, mandatory guards and configured deny rules; obtain the operator's decision through the Director before a guarded command runs. Hermes assigns the new conversation's session ID. Record that ID and a redacted creation-policy snapshot before cleanup; the durable owned profile remains the cancellation boundary. Replacement creation allocates a new profile and a fresh conversation.

### Preserved normal/manual session snapshots

The Verifier opened only the two audited owned `state.db` files with SQLite URI `mode=ro`, `PRAGMA query_only=ON`, and a read transaction per database. Each owner marker matched its audit record. The local creation receipt is stored at `state_meta["gateway.local_policy.v1:" + session_id]`; selecting its safe fields preserves the frozen request and configuration without exporting credential references or secret values. The snapshot is `.hermes-spike/normal-20261025T115000Z/captures/verifier-first-turn-snapshot.json`, with observation times `2026-10-25T12:09:35.752446+00:00` and `2026-10-25T12:09:35.753762+00:00`.

| Slot | Confirmed session ID | Snapshot message / API / tool counts |
|---|---|---|
| Primary | `local-d97ff90759d25e133432a4e2d064fa15329a1a8571b1c66eac228db613c75853` | 4 / 2 / 0 |
| Sentinel | `local-6be054d1398f8aff7dcb989e887b3bf83a5f475ce86b223e3f6bcc064269bf4c` | 2 / 1 / 0 |

Both frozen requests contain exact model `openai/gpt-6.1-sol`, reasoning `high`, `pass_session_id=true`, `source=tool`, cwd `/Users/himkt/work/himkt/cafleet`, and empty `row_cwd`. Both effective runtime records show provider `nous`, API mode `chat_completions`, host `inference-api.nousresearch.com`, and `fallback_active=false`. Both frozen `agent.reasoning_effort` values are `high`, confirmed by a follow-up read saved in `captures/verifier-second-turn-observation.json`. These are persisted session settings; model-request wire inspection and actual tool behavior remain outside this snapshot.

Both frozen configurations select `approvals.mode=manual`, `hooks={}`, and `hooks_auto_accept=false`. Neither creation request contains `yolo`; both receipts have `yolo=false` and `yolo_toggle=null`, and neither configuration selects mode off. This preserves the observed persisted bypass sources. The gateway process's import-frozen bypass variable was not introspected; observed live guarded-command behavior remains necessary. The absence of an approval-deny field matches this spike's inspected source shape and establishes no permission to discard a configured source deny policy.

Both policies freeze `TERMINAL_ENV=local`, the intended `TERMINAL_CWD`, private terminal-cache paths, and `client_env_remote=false`. Their scoped client environment contains distinct `CAFLEET_DATABASE_URL` values ending in the corresponding `databases/<slot>.db`, selector `CAFLEET_MULTIPLEXER=herdr`, and the matching pane ID. These are accepted session-policy values, not proof that a terminal command has opened the correct broker database. The ordinary `sessions.cwd` column is null while the frozen terminal cwd is correct; interpret the actual runtime scope through the creation policy and subsequent tool observation.

Both first persisted user messages exactly match all 579 characters of `m1-prompt.txt`, including the final newline. Expected and actual SHA-256 are `ccae50db50714d63dcd121309d3ed282a8a549d6f99c777ed671e1c064ee1d14`. Each first assistant message is exactly `CAFLEET_NORMAL_M1_READY`. A follow-up read confirms primary's second user query and exact answer `CAFLEET_NORMAL_SECOND_READY`; sentinel has no second exchange at that checkpoint. The snapshot's primary counters therefore already include its Director-run second message and must not be described as first-turn-only counts.

Director-saved first-reply captures are `captures/primary-first-reply.ansi` and `captures/sentinel-first-reply.ansi` under this attempt. Full M3 before/after evidence must pair these recorded counters with the timing of later native dispatch. No hook was activated or tested by this collection.

### Observed manual-mode native command subchecks

The Verifier read `captures/primary-native-output.ansi` and paired `primary-before-native-baseline.json` / `primary-after-native-baseline.json` under the normal attempt. The Director reports Escape-first native submission of `! /usr/bin/printf CAFLEET_NORMAL_NATIVE_OK` and `! /usr/bin/false`. The capture visibly contains `CAFLEET_NORMAL_NATIVE_OK`, `! exited 1`, and `/approvals` output `Approval mode: manual (persistent profile setting).`

Both baseline files identify the same primary session and exactly `message_count=4`, `api_call_count=2`, `tool_call_count=0`. This supports local native output and failure-status presentation without a model turn for the exercised benign commands. It supplies no actual dangerous-command approval/refusal decision, no complete broker/push test and no uncertain-dispatch recovery test. No hook was enabled.

### Writer teardown probe: baseline, failed deletion and cleanup

The Director started the audited regular and persistent bounded writers through the primary model's terminal tools. The Verifier independently sampled only their attempt-owned PID/heartbeat/exit/stop files, 0.8 seconds apart, without invoking Hermes or controlling a process. The raw observation is `captures/verifier-writer-observation.json` under the normal attempt.

| Writer | PID | Heartbeat counts | Deadline evidence | State at second sample |
|---|---|---|---|---|
| Regular | 35273 | 70 to 72 | `deadline_monotonic_ns=92802958362208` | Profile present, no stop flag, no exit record; observed monotonic time `92658922849041`. |
| Persistent | 35338 | 66 to 67 | `deadline_monotonic_ns=92805088830416` | Same observation conditions; deadline remains in the future. |

Both PID records match run `f20-m82-normal-20261025t115000z`, slot primary, and their expected kind. Their finite lifetime is 180 seconds. This independent file sample confirms pre-delete heartbeat progress; the Director's following observations add process-identity checks and the actual teardown result.

`captures/primary-writers-started.ansi` records model-started regular handle `proc_17e3b4898ab8` / PID 35273 with `persist_on_release=false`, and persistent handle `proc_cc9e1f599275` / PID 35338 with `persist_on_release=true`. Both use `background=true, notify=false`; the model completed its answer and waited before deletion.

`records/primary-cleanup-finish-1792930411159960000.json` records both writers progressing before dispatch (`baseline_writers_live=true`) and the literal owned-profile delete's actual exit `0`. Its delete interval is monotonic `92680837630125` to `92681557258333`. The immediate post-delete record `results/primary-observation-1792930413511611000.json` reports the profile and stage absent, a tombstone present, and both writers still progressing:

| Writer | Post-delete heartbeat counts | Process/stop evidence | Deadline comparison |
|---|---|---|---|
| Persistent PID 35338 | 112 to 114 | Both samples identify the owned writer, with `pid_reused=false`, `cleanup_stop_issued=false`, `exit=null`. | Second sample `92682335670666` precedes deadline `92805088830416`. |
| Regular PID 35273 | 118 to 119 | Same verified identity and pre-cleanup conditions. | Second sample `92683127602291` precedes deadline `92802958362208`. |

Both dispositions are `live_writer_before_deadline`. These observations prove the current adapter's cancellation boundary fails even for a regular writer after a completed turn; the persistent writer independently fails too. A success exit or absent profile is insufficient. Preserve the existing mandatory guards and require a reviewed cancellation/settlement protocol or compatible native support before implementation resumes.

The Director reports that the harness's final absent-profile invariant then failed because the deleted name was recreated. An independent read-only Verifier check, saved as `captures/verifier-primary-residue-check.json`, found a regular directory with device `16777233`, inode `107529952`, birth witness `1792930415.0728102`, replacing original inode `107527584` / birth witness `1792929981.2387946`. At `observed_wall_ns=1792930579200939000` it held `state/dashboard_clients.heartbeat`, `processes.json.lock`, and cache/runtime/sandbox infrastructure paths. The Director's earlier narrower inventory contained only the heartbeat and process lock. Preserve both point-in-time observations; the original-incarnation cleanup helper correctly cannot authorize removal of this replacement merely from its path.

`records/primary-writer-stop-1792930462354446000.json` records the later explicit stop request at monotonic `92730400721083`. The matching `primary-regular.exited.json` and `primary-persistent.exited.json` record `exit_reason=cooperative_stop`, at `92730701117458` and `92730838771916`, respectively, before their natural deadlines. `results/primary-observation-1792930475834107000.json` then reports both owned PIDs absent and heartbeat counts stable (regular 213, persistent 209). Its `inconclusive_cleanup_stop` dispositions correctly prevent treating this cleanup as native cancellation acceptance. The earlier pre-stop failure remains conclusive.

The recreated primary residue remained at the earlier checkpoint and was then settled through the separate guarded recovery described below. The Verifier performed no destructive cleanup and preserved the original-incarnation checks. Remaining active/queued/pane-loss/crash/race cases stay unrun; no additional acceptance probe is authorized past this failed gate.

### Final owned-resource cleanup recheck

Director message 860 reports both dedicated clients closed, sentinel usability confirmed after primary deletion, and native cleanup of sentinel plus the separately identified primary residue. The Verifier read the durable residue preflight/deletion records and independently checked final filesystem/PID absence.

| Check | Evidence | Final observation |
|---|---|---|
| Sentinel still usable after failed primary deletion | `captures/sentinel-after-primary-delete.ansi` contains the assistant reply `CAFLEET_SENTINEL_AFTER_DELETE_READY`. | The unrelated sentinel answered after the primary failure, before its own cleanup. This observed survival subcheck does not undo the owned-writer cancellation failure. |
| Sentinel deletion | `records/sentinel-cleanup-finish-1792930593441702000.json` and matching observations. | Native profile-scoped deletion exited `0`; no sentinel writers were present. |
| Distinct primary residue preflight | `captures/director-residue-preflight.json`. | Exact recreated inode/birth witness and narrow empty runtime tree matched; tombstone remained, owned writer PIDs were absent. Director separately reports no open handles from `lsof +D` (exit `1`, no output) before the mutation. |
| Distinct primary native deletion | `captures/director-residue-delete.txt`. | One native profile-scoped deletion exited `0` and reported the residue path removed. Original-incarnation helper remained unchanged. |
| Independent primary absence action | Verifier executed read-only `residue-preflight.py verify-absent`. | Returned `absent=true`, `tombstone_preserved=true`, `mutation=false`; its evidence validation includes both audited writer PIDs absent. |
| Independent two-profile/stage/PID recheck | `captures/verifier-final-cleanup-check.json`, observed `2026-10-25T12:25:19.495056+00:00`. | Both exact active and stage paths absent, with no dangling symlinks; both deletion tombstones intact. Signal-zero existence checks found PID 35273 and PID 35338 absent; both durable exits remain `cooperative_stop`. |
| Dedicated client panes | Verifier read `captures/primary-final-pane-absence.txt` and `captures/sentinel-final-pane-absence.txt`, supplied by Director message 864. | Native Herdr reads each exit `1` with structured `error.code=pane_not_found` for the exact targets `w2Z:p36` and `w2Z:p37`. These are missing-pane results, distinct from the earlier sandbox access denials. |

The Verifier's `/bin/ps` inspection was denied by the sandbox; the permitted signal-zero existence checks supplied independent PID absence instead. Direct Herdr socket access was unavailable, so Director-authorized native outputs supplied the final pane-absence evidence and were read by the Verifier. Owned-resource cleanup is complete for the audited profiles, stages, writers and client panes; retained tombstones/audit infrastructure remain by design. No shared gateway, source profile or installed package mutation was performed by the Verifier. Cleanup preserves the earlier writer-survival and directory-recreation counterexamples; M4 remains failed.

## Historical partial attempt: fresh conversation before permission-policy change

Attempt root: `.hermes-spike/fresh-20261025T113600Z/`. Director message 824 reported first replies in Herdr panes `w2Z:p34` and `w2Z:p35`; message 828 then stopped this approval-bypass attempt. The Verifier ran no model or native-command probe in either session. Preserve the observed subchecks without passing the revised permission policy.

The Verifier inspected both owned `state.db` files through SQLite URI `mode=ro` and `PRAGMA query_only=ON` before cleanup. Each contained one conversation, stored model `openai/gpt-6.1-sol`, `source=tool`, `created_source=tool`, `message_count=2`, `tool_call_count=0`, and `api_call_count=1`. Full IDs are:

| Slot | Confirmed session ID |
|---|---|
| Primary | `local-4fe56a4cd2147f0eefb499f98c3dfbc8a6cad9c7551b8664fee59059612af911` |
| Sentinel | `local-822f7ac53f4ee20f2de0e1009fccb077b54899629f193a1fd61ed928dccd781e` |

The redacted reconstruction of the observed database fields and rendered-query checks is saved at `.hermes-spike/fresh-20261025T113600Z/captures/verifier-session-baseline-before-policy-change.json`. Only key names were sampled from `model_config.gateway_runtime`; effective reasoning and the complete frozen creation policy were not collected before deletion. The `sessions.cwd` field was null, which establishes no terminal-tool cwd result. A later read found both owned stores already absent after Director cleanup.

Director-saved `primary-first-reply-before-policy-change.ansi` and `sentinel-first-reply-before-policy-change.ansi` each show the expected assistant answer `CAFLEET_FRESH_M1_READY`, model `gpt-6.1-sol`, provider label `Nous Research`, and an explicit YOLO-mode banner. Removing ANSI controls from each capture preserves the complete expected 487-character query except its final terminal newline, including quotes, braces, dollar text and backticks. Expected prompt SHA-256 is `d69cd0cbf321b4fbd815af3554cd30f44f255d4717ba4fa8ae9945dcc8c7cac2`. This rendered-text comparison is narrower than an exact persisted-message byte comparison, which was not collected. The primary capture includes a typed second-message draft; it supplies no second assistant reply.

The old message/API counters are historical pre-native-command baselines only. Recollect both counters and relevant transcript rows before and after normal-approval M3 commands. The banner also lists the built-in `hermes-agent` skill and discovery tools, while displaying terminal execution without a visible `process_manage` entry. The Director assigned source inspection of that skill/tool exposure to the Programmer; the banner alone proves neither callable absence nor acceptance of the intended private surface.

Director-owned cleanup completed for both profiles: `records/primary-cleanup-finish-1792928949791496000.json` and `records/sentinel-cleanup-finish-1792928952151150000.json` each record actual profile-scoped deletion exit `0` with empty writer baselines. Matching start/observation records and `captures/primary-cleanup-after-policy-change.txt` / `sentinel-cleanup-after-policy-change.txt` remain under this attempt root. This is cleanup of a partial, no-tool workload, not M4 acceptance. Further probes use the new normal-approval attempt.

Director follow-up on 2026-10-25 at approximately 12:03 UTC: native profile deletion had removed the owned stores while the two classic client panes still displayed their previous composers. This observation establishes that profile absence alone does not prove client-process exit. The Director saved `primary-client-before-pane-close.ansi` and `sentinel-client-before-pane-close.ansi` under the historical fresh-attempt captures directory, then closed only owned panes `w2Z:p34` and `w2Z:p35`. Both close operations returned success; subsequent reads returned `pane_not_found`. The primary's typed second-message draft was never submitted. This finishes the owned client-pane cleanup and supplies no active-work M4 pass.

## Historical attempt: prepublication preparation and stage recovery

Checkpoint: Director message 792 and Verifier inspection on 2026-10-25, following the Director's matching-stage recovery. This is a disposable-harness preparation result. It changes no M1-M4/D1-D2 gate to pass or fail.

The documented Director entry point is `/opt/homebrew/Cellar/hermes-agent/0.22.4/libexec/bin/python3 -B /Users/himkt/work/himkt/cafleet/.hermes-spike/harness.py`, with separate `prepare --slot primary`, `inspect-stage --slot primary`, and `recover-stage --slot primary` operations. The Director reports those preparation/inspection/recovery actions through the Step 1 marker; raw command captures and exact exit codes were not supplied to the Verifier.

| Observation | Result and evidence |
|---|---|
| Preparation | FAIL before publication, attributed to the Director. Skill copying rejected `skills/cafleet/reference/runtime/concepts/coding-agents.md` as a source symlink after creating the hidden stage. This is a spike materialization defect, not an observed Hermes runtime incompatibility. |
| Before recovery | The Director reports `inspect-stage` confirmed the matching owner marker, audited directory incarnation, and absent active path. The Verifier did not observe the stage before its removal. |
| Recovery | The Director reports successful matching-stage recovery. The retained artifact `.hermes-spike/records/primary-stage-recovered-1792927325046360000.json` records the same owner/incarnation/path as `.hermes-spike/records/primary.json`. The inspected recovery implementation writes that archive after stage removal and parent-directory fsync. |
| Independent post-recovery filesystem check | `ls -ld` reports `No such file or directory` for both `/Users/himkt/.hermes/profiles/cafleet-spike-f20-m82-20261025-1107-primary` and `/Users/himkt/.hermes/profiles/.cafleet-staging/cafleet-spike-f20-m82-20261025-1107-primary`. The staging container remains with mode `0700`. The combined check exits 1 because both requested leaf paths are absent. |
| Retained ownership witness | Both inspected records identify fleet 20, Programmer member 82, run `f20-m82-20261025-1107`, and profile `cafleet-spike-f20-m82-20261025-1107-primary`. Their directory witness is device `16777233`, inode `107470399`, and `birth_time_seconds` `1792927259.4756196`. This is the spike's recorded witness; production nanosecond identity remains untested. |
| Source-link target | Independent `ls -l` confirms the failing path points to `../../../../../docs/docs/concepts/coding-agents.md`; the target `/Users/himkt/work/himkt/cafleet/docs/docs/concepts/coding-agents.md` is a regular repository file. It lies outside the repository's `skills/` subtree. |
| Retry prerequisites | At the inspection checkpoint, the original live slot reservation still exists and the source copier still rejects links. Archive and durably retire that live reservation after verified recovery, preserving existing archives. Review a source boundary admitting the required repository runtime-documentation files, then materialize accepted regular-file targets as private plain destination files. Tester has already recorded this boundary issue in Step 1. |

The recovery observation passes for the exact unpublished spike stage: the owned leaf is absent and the audit evidence remains. It establishes neither native Hermes profile teardown nor cessation of gateway/background writers, because the profile had not yet been published or launched. All runtime gates remained `NOT RUN` at that recovery checkpoint. The materializer and reservation-retirement fixes subsequently allowed fresh preparation; the following live attempt produced the distinct M1 failure.

## Historical attempt: named-continuation M1 failure

This section records the superseded contract's real failure and retains its exact original argv for audit. Current fresh-conversation verification uses the separate attempt above.

The Director supplied `.hermes-spike/captures/primary-startup.ansi` and `.hermes-spike/captures/sentinel-startup.ansi`. The Verifier read both files directly. Each captures its real shell launch, the same startup refusal, a return to the shell, and `CAFLEET_HERMES_STARTUP_EXIT=2`.

The displayed Director commands are separate invocations of the packaged interpreter:

```text
/opt/homebrew/Cellar/hermes-agent/0.22.4/libexec/bin/python3 -B /Users/himkt/work/himkt/cafleet/.hermes-spike/harness.py launch --slot primary --reasoning high
/opt/homebrew/Cellar/hermes-agent/0.22.4/libexec/bin/python3 -B /Users/himkt/work/himkt/cafleet/.hermes-spike/harness.py launch --slot sentinel --reasoning high
```

The inspected historical harness's `launch` constructs literal argv and uses `os.execve` with the pane's TTY. For each slot, that attempt's effective argv requested:

```text
/opt/homebrew/Cellar/hermes-agent/0.22.4/bin/hermes
  --profile cafleet-spike-f20-m82-20261025-1107-<slot>
  chat --cli --yolo --in /Users/himkt/work/himkt/cafleet
  --continue cafleet-spike-f20-m82-20261025-1107-<slot>
  --create-if-missing --source tool --pass-session-id
  --model openai/gpt-6.1-sol --reasoning high
  --query <single literal argument read from .hermes-spike/m1-prompt.txt>
```

Both captures contain this exact refusal, followed by exit `2`:

```text
Error: Resume retains gateway session policy; creation overrides are unsupported on resume. -m/--model, --reasoning, --pass-session-id differ from this session's launch: resume without them and use /model or /reasoning after resuming to switch.
```

| Profile / pane | Fresh ownership evidence | Observed result |
|---|---|---|
| `cafleet-spike-f20-m82-20261025-1107-primary`, `w2Z:p32` | `.hermes-spike/records/primary.json` records device `16777233`, inode `107474911`, birth witness `1792927545.7969935`; the Verifier independently read its matching live owner marker before cleanup. This is a new incarnation after the archived preparation failure. | `primary-startup.ansi`: policy refusal, no assistant answer, shell-recorded exit `2`. |
| `cafleet-spike-f20-m82-20261025-1107-sentinel`, `w2Z:p33` | `.hermes-spike/records/sentinel.json` records device `16777233`, inode `107475900`, birth witness `1792927560.61264`; the Verifier independently read its matching live owner marker before cleanup. | `sentinel-startup.ansi`: same policy refusal, no assistant answer, shell-recorded exit `2`. |

The names above are requested logical session titles. The retained evidence contains no gateway-generated session IDs or persisted `launch_request` snapshots. Cleanup removed the owned profile stores before a Verifier session-policy snapshot was collected; the forwarding defect below is independently source-confirmed rather than inferred from an exported session row. Future accepted probes must save redacted actual session identity/policy before cleanup.

The Verifier's initial direct Herdr socket reads each exited `1` with `PermissionDenied: Operation not permitted`. Director-authorized durable captures resolved that evidence-access gap. The captured Hermes exit `2` is separate from those shell-tool access failures.

## Historical M1 source cause and accepted contract revision

The Verifier independently read the installed source chain identified by the Programmer and Tester:

1. `hermes_cli/main.py:1695,1765-1769` resolves continuation/session arguments before invoking ordinary `cli.main`.
2. On a missing named continuation, `main.py:1401-1431` calls `gateway_session_store.create_titled_session(title)` and assigns its newly created ID to `args.resume`.
3. `hermes_cli/gateway_session_store.py:539-558` creates that title with `_creation_policy({})`, hard-codes `source="cli"`, and sends `session.create`. The requested model, reasoning and pass-session-ID launch fields are absent from that early policy input; the explicit `tool` source is also lost at this point.
4. `hermes_cli/cli_init_mixin.py:41-60` subsequently reconstructs the explicit launch fields. `gateway_session_store.py:142-158,504-516` treats the newly created row as a resume and checks those requested fields against its frozen launch policy.
5. `hermes_cli/gateway_chat.py:297-321` computes those differences and raises the captured refusal. Fresh profile ownership therefore does not avoid this creation/resume-policy defect.

The user subsequently selected fresh conversations on every launch, matching CAFleet's existing backends. The current specification retains owned profiles for isolation/cleanup and supplies complete requested model, reasoning, pass-session-ID, source, cwd, environment and unattended policy during ordinary fresh creation. The new attempt evaluates that accepted contract independently. The installed named-continuation defect remains an explanation of the historical failure and establishes no verdict for the current launch.

## Historical attempt: owned-resource cleanup

The Director ran the recorded profile-specific deletion command for each owned name:

```text
/opt/homebrew/Cellar/hermes-agent/0.22.4/bin/hermes --profile default profile delete <owned-profile-name> --yes
```

| Resource | Durable evidence | Result |
|---|---|---|
| Primary profile | `.hermes-spike/records/primary-cleanup-start-1792927705487276000.json`, matching `primary-cleanup-finish-1792927705487276000.json`, and `.hermes-spike/results/primary-observation-1792927706340632000.json`. | Actual subprocess exit `0`; post-delete profile/stage absent, tombstone present, writers empty. |
| Sentinel profile | `.hermes-spike/records/sentinel-cleanup-start-1792927785956977000.json`, matching `sentinel-cleanup-finish-1792927785956977000.json`, `.hermes-spike/results/sentinel-observation-1792927786783429000.json`, and `.hermes-spike/captures/sentinel-cleanup.txt`. | Actual subprocess exit `0`; post-delete profile/stage absent, tombstone present, writers empty. Raw cleanup output identifies `openai/gpt-6.1-sol (nous)` and reports the profile removed. |
| Dedicated probe panes | Director's final cleanup marker. | Both panes closed after durable capture; pane closure is Director-reported. |
| Independent filesystem check | Verifier `ls -ld` of both exact active profile paths after cleanup. | Both report `No such file or directory`; the check exits `1` because both paths are absent. |

Both cleanup baselines contain `writers: []` and `baseline_writers_live: false`. These are successful owned-resource cleanup observations, not M4 acceptance of active/queued/background cancellation, writer-release barriers, sentinel-session survival, or crash/race recovery. Both M1 launches ended before the first assistant answer. Preserve the empty-workload limitation and keep M4 unrun.

## Installed launch-path findings

All source references below are under `P`; reads used `sed -n` and `rg -n --no-ignore` against the installed files. The package source is readable but hidden from ordinary `rg --files` by ignore rules; `rg --files --hidden --no-ignore` found it.

1. `hermes_cli/main.py:1680-1687` enters `launch_gateway_chat(args)` only for `stream_json_requested(args)` or `_bypass_chat_launch(args)`. Its inline explanation describes ordinary classic chat as `HermesCLI` with a gateway-backed agent.
2. `hermes_cli/main_tui_launch.py:24-28` defines the bypass as `safe_mode` or `ignore_user_config`, with explicit TUI selection excluded. The design's normative argv supplies neither bypass flag nor stream JSON.
3. `hermes_cli/main.py:1745-1769` reads the query file, builds ordinary chat arguments, and calls `cli.main`. `hermes_cli/cli_agent_setup_mixin.py:649` imports `GatewayAgent` for that client's agent execution.
4. `hermes_cli/gateway_chat_view.py:301-307,576-583` routes non-slash text through `prompt.submit`; its inspected loop has no bang-shell branch. The Director's isolated execution of this branch therefore has a concrete, narrowly scoped failure result.
5. `hermes_cli/cli_tui_runtime_mixin.py:136-145` handles a non-seeded input through `handle_bang_shell` before slash routing and model work. `hermes_cli/cli_tui_mixin.py:1426-1433` classifies bang input as local dispatch while busy, preventing direct steering into the model. Busy input may queue until the loop drains; live timing remains unverified.
6. `hermes_cli/cli_info_mixin.py:520-560` parses the bang command, checks the approval gate, calls `run_bang_command`, prints a nonzero exit code, and returns handled. `hermes_cli/bang_shell.py:41-55` disables this path when gateway/cron/session-platform environment flags are active. The actual launched client environment must therefore be part of M3 evidence.
7. `hermes_cli/bang_shell.py:93-126` constructs a filtered local subprocess environment and starts a local shell subprocess. Source supports the intended dispatch mechanism; host execution, cwd, broker environment, output, and model-turn accounting still require direct runtime evidence.

The source-path interpretation is resolved by Director arbitration. If the real normative client fails, retain that concrete failure and stop the member milestone. If it passes, continue the remaining gates; this source review alone grants no release acceptance.

## M2: Configuration, environment, and authentication prerequisites

The Verifier independently read the installed sources cited below, confirming the Programmer's source findings. Every acceptance result remains `NOT RUN`.

| Concern | Installed source evidence | Required runtime observation |
|---|---|---|
| Broker and multiplexer variables | `agent/session_client_env.py:27-30,41-95` adds names through `terminal.client_env_passthrough`, filters at both client collection and gateway acceptance, excludes secret names and `HERMES_` control names, and conditions host values on local-client proof. `CAFLEET_DATABASE_URL` is absent from built-ins. | Explicitly allow broker and selected multiplexer names in the owned profile. From both agent terminal and bang shell, observe cwd, executable lookup, correct database, multiplexer access, and full broker poll/show/ACK against an already-running gateway. |
| Session environment isolation | `agent/session_client_env.py:114-174` binds environment through a context variable and replaces/unsets the gateway's toolchain values per local session. | Run simultaneous profiles with distinct database/context values; each observes only its own selected values. Capture effective environment names and redacted results. |
| Root and named profile resolution | `hermes_cli/profiles.py:2432-2460` maps a profile-shaped `HERMES_HOME` to its root, maps explicit `default` to that root, and requires live identity for named selection. | Exercise a custom root and a conflicting sticky active profile. Observe the intended source and owned profile; preserve the user's active selection. |
| Inert staging and skills | `hermes_cli/profiles.py:476-492` requires a valid profile ID and identity file for enumeration; hidden staging names fail that ID grammar. `hermes_constants.py:394-427` defines identity/live checks. `hermes_cli/profiles.py:94` defines `.no-bundled-skills`. | Publish only the fully materialized private profile, verify gateway discovery after publication, and verify that skill seeding leaves the private skill surface intact. |
| Root-shared provider grant | `hermes_cli/auth.py:826-861` selects local provider state before root fallback and retains source-path identity under auth locks. `auth.py:895-907` writes refresh state back to its source without changing active-provider choice. `auth.py:948-974` makes a nonempty local credential pool shadow the root pool. | Use empty child provider state with preserved source active-provider metadata. Verify effective shared-grant identity and refresh write-through, plus unavailable-root/profile-specific-grant failures; copy no OAuth grant. |
| Accepted private config and secrets | The Director reports source top-level keys `_config_version`, `agent`, `model`, and `telemetry`; only names/shapes and selected provider metadata are recorded here. | Complete the accepted-build config/provider/environment fixture before production materialization. Verify the disabled behavior surface and scoped inference credentials; a source trace alone accepts no configuration schema or credential path. |

## M4: Teardown risk and prioritized acceptance probe

The Verifier independently confirmed the following source chain:

1. `hermes_cli/profiles.py:1760-1860` disables profile services, stops profile-bound backends, creates a tombstone, notifies the multiplexer, attempts identity purge, waits for profile ownership release, and removes the directory. A pending identity result raises a typed partial-settlement error.
2. `hermes_cli/profiles_remove.py:13-33` allows 60 seconds for `gateway.lock` release through `exclusive_maintenance`. This is a gateway reservation observation, not direct terminal-process enumeration.
3. `gateway/run_runtime.py:323-365` retires an authority by stopping/joining turns, side work and hosted-room services. `gateway/session_runtime_workers.py:151-173` joins physical workers, executing sessions, mutations and side agents; it does not itself enumerate terminal background processes.
4. `gateway/run_profile_reconcile.py:455-536` evicts cached agents and retains the reservation when retirement fails. On successful retirement, it releases stores, scoped MCP resources and logs before releasing the reservation.
5. `gateway/run_agent_cache.py:822-870` explicitly uses soft eviction, preserving terminal environments and background processes. A completed/evicted turn is therefore a required teardown test case.
6. `agent/client_lifecycle.py:109-121` skips running processes marked `persist_on_release`. `tools/process_registry.py:2508-2543` likewise skips them for lifecycle kill sources; deliberate `process_manage` cancellation is described separately.

This establishes a concrete source-level teardown risk, not a runtime M4 failure or proof that profile deletion leaves a process alive. The Director's disposable spike should start one ordinary and one explicitly persisted bounded writer, retain process identities, complete/evict a turn, and delete only the owned profile while an unrelated sentinel remains active. Observe helper/gateway completion, process-tree settlement and writer cessation after the reservation releases; record tombstone/discovery and identity settlement. The probe must explicitly stop/reap any surviving owned writers and record that cleanup.

Accept the native reservation as the last writer-release barrier only after these observations prove it. A live writer after successful profile deletion fails M4 and stops the member milestone; the implementation then requires a reviewed adapter/protocol revision. Preserve every unrun race/crash/background case rather than deriving full M4 acceptance from a single smoke probe.

## D1: Installed native tool-schema inventory

`toolsets.py:102-154` defines the design's required toolset identifiers. In particular, its terminal toolset contains `terminal` and `process_manage`. Source names establish probe inputs; actual session exposure and behavior remain runtime gates.

| Tool | Installed schema | Probe contract |
|---|---|---|
| `terminal` | `tools/terminal_tool.py:1568-1620`: required string `command`; optional boolean `background`, integer `timeout` of at least 1, string `workdir`, boolean `pty`, boolean-or-string-array `notify`, nonnegative integer `heartbeat`, and boolean `persist_on_release`. | A finite receive uses `background=true, notify=true` with the bounded broker wait command. Retain the returned process handle; verify exactly one completion wake before renewing. |
| `process_manage` | `tools/process_registry.py:2582-2630`: required string `action`; `session_id` is required except for list; optional string `data`, integer `timeout` of at least 1, integer `offset`, and integer `limit` of at least 1. | Verify polling/waiting and explicit cancellation/join of the retained handle. Preserve incomplete status and captured output until cancellation or completion is observed. |

`notify_on_complete` is an unadvertised legacy terminal argument; publish examples using advertised `notify`. The public process tool name is `process_manage`. A schema does not prove wake delivery, compaction persistence, duplicate handling, or goal preservation; executable examples await D1/D2 runtime acceptance.

## D1-D2 preparation after full-execution authorization

On 2026-10-25 the Verifier re-read the installed terminal/process schemas, gateway-backed classic-CLI notification path, wait/kill/consumption behavior, goal authority/budget handling and compression publication. Source anchors and exact advertised fields are preserved in `.hermes-spike/director-runtime/source-evidence.md`. The bounded Director runbook is `.hermes-spike/director-runtime/probe-plan.md`, including the 30-second native broker lease, one receiver at a time, one planned timeout renewal, full-message/ACK/deduplication evidence, cancellation, goal states, compaction, clarification and user interruption. These are prepared operations; every live D1-D2 case remains NOT RUN.

The broker's own timeout bounds a background receiver; terminal background dispatch does not pass its execution-plan timeout to the local spawn. A successful spawn is distinct from command exit, admitted notification, started model turn and completed reconciliation. A process wait timeout or interruption can leave the process alive. Explicit wait/log/kill consumption can suppress a wake. Async-unsupported contexts disable accepted completion intent, including managed workers; actual tool exposure and accepted delivery capability are therefore prerequisites.

Goal renewal preserves the user's existing authority and budget. Native `/goal resume` resets usage, so CAFleet reception renews independently. Snapshot the actual physical target and account for native judge turns without resetting usage or increasing max_turns. Preserve an absent or paused goal. Capture the auxiliary `goal_judge` route separately from the main model.

The source review identified a rotating-manual-compaction goal-migration gap candidate: `hermes_state_mutation_compress.py:6-28` publishes/retargets through `hermes_state_worker_compression.py:216-225` and `hermes_state_compression.py:400-485`, while goal migration appears in the separate managed-worker publication (`hermes_state_worker_compression.py:289-307,344-345`). Goal RPC reads the newly resolved physical target's flat key, and the legacy migration helper skips RuntimeSessionStore. This is source evidence, not a reproduced runtime failure; noop/preview supplies no compaction pass.

Director assignment 939 requested a native in-place constraint rather than a goal-migration workaround. `.hermes-spike/director-runtime/in-place-constraint.md` confirms supported boolean `compression.in_place=true` through defaults, agent parsing and ordinary automatic/manual publication paths. Require the exact effective boolean in the agent and session policy, accepted built-in engine/API route, and actual manual plus automatic committed compression before D2 acceptance. Existing source-copy rules omit source compression/context customizations; the Director must review the narrow managed-config addition before implementation.

The pin does not govern the separate compression-exhaustion reset in `gateway/run_turn_persistence.py:323-407`, which can change a local conversation's target independently, or the separate provider-native/external-engine paths. Review the accepted recovery boundary or native barrier/build before promising universal goal/identity preservation. Preserve the user's goal and budget instead of creating, recreating or automatically resuming it. Every live D1-D2 case remains NOT RUN; no source profile, runtime configuration or installed package was changed.

## Historical alternative: read-only command-policy hook assessment

Assignment: Director message 839. A supported shell `pre_tool_call` hook was identified as a source-supported extension candidate for immediate refusal outside a positive allowlist. The user later selected native dangerous-command deny rules instead, so this section is retained as historical assessment rather than a current activation requirement or production blocker. The hook was never activated or live verified. Current generated profiles retain `hooks: {}`; this assessment changed no profile, installed source, runtime setting or registration and executed no Hermes command.

The official [Event Hooks](https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks) page documents shell hooks, blocking directives, per-entry `fail_closed`, first-use consent and profile-scoped gateway refresh. The official [Security](https://hermes-agent.nousresearch.com/docs/user-guide/security) page distinguishes manual dangerous-command approval, unconditional deny patterns and mandatory native guards. These primary sources support investigating the extension; source inspection below determines its installed implementation boundaries. Both pages were inspected 2026-10-25.

### Smallest proposed supported configuration

The following is a proposal for review, not an activated configuration:

```yaml
approvals:
  mode: manual
hooks:
  pre_tool_call:
    - matcher: "terminal"
      command: "<reviewed-absolute-interpreter> <owned-profile>/agent-hooks/cafleet-command-policy.py"
      timeout: 5
      fail_closed: true
hooks_auto_accept: false
```

The owned, reviewed script receives JSON on stdin and returns `{}` only for an explicitly permitted command form. For every other terminal command it returns `{"action":"block","message":"Command requires Director-authorized native execution"}`. Parse a reviewed command grammar and validated argv, preserving literal broker payloads; reject shell syntax or execution forms outside that grammar. Derive the permitted operations from the operator-approved policy and the member's CAFleet authority rather than copying a separate command registry. In particular, preserve the broker operations needed for readiness, full-message retrieval, ACK and routing while retaining Director-only command authority. `presets/opencode/cafleet.md` is the existing repository comparison point for a shell policy whose default is refusal and whose listed patterns permit ordinary member work.

An allowed hook result continues into Hermes's existing guards; it grants no dangerous-command approval and selects no approval-bypass mode. An `action: approve` hook result requests a human gate, so it does not implement prompt-free refusal. Use `block` for the policy rejection. Keep `approvals.mode: manual` and the source deny policy intact. Review argument-bearing control surfaces such as process input and delegated/discovery dispatch before claiming a complete shell policy; this single matcher defines only model-issued `terminal` calls.

### Installed contract and enforcement order

All paths below are under installed source root `P`.

| Boundary | Source evidence and implication |
|---|---|
| Hook payload and result | `agent/shell_hooks.py:88-104,636-653` supplies `tool_name`, `tool_input`, session/profile metadata and cwd, and normalizes canonical `block`/`modify`/human-approval directives. `hermes_cli/plugins.py:1979-2031` makes a valid block outrank approval directives. |
| Refusal before execution | `agent/tool_executor.py:672-690,710-763` evaluates pre-tool hooks before real dispatch, terminal approval preparation and the terminal tool. A block becomes a tool error. Source therefore supports refusal without opening the ordinary dangerous-command prompt. |
| Normal guards remain | `tools/approval.py:1152-1163` retains command floors and deny checks in the terminal tool's combined guard. Passing the extension allows that guard to make its normal decision; it does not overwrite it. |
| Hook execution failures | `agent/shell_hooks.py:550-624` honors exit `2` as refusal and, with `fail_closed: true`, blocks spawn failures, timeouts, nonzero exits lacking a directive and non-JSON output. Clean valid no-op JSON proceeds. The script must produce a deliberate block on invalid policy input. |
| Callback failure handling | `hermes_cli/plugins_dispatch.py:256-300` blocks pre-tool callback exceptions, timeout and skipped still-running callbacks. This is stronger than a generic observer's skip-on-error behavior. |
| Outer pipeline limitation | `agent/tool_executor.py:672-690` catches an unexpected exception from the overall hook-dispatch call and returns no block. Per-hook `fail_closed` therefore does not establish an unconditional failure guarantee for every infrastructure error. |

### Activation, consent and trust

`agent/shell_hooks.py:150-180` registers config-defined scripts through the plugin manager even without installing a plugin package. The exact `(event, command)` pair must be consented for that profile. Ordinary first-use consent can be presented by the classic client's foreground terminal before it opens the gateway session (`shell_hooks.py:239-254`). A gateway never asks for this startup consent: an unconsented configured hook is skipped with a warning, so `fail_closed: true` protects an executing hook, not a hook that was never registered.

Keep automatic hook-consent settings disabled. The Director relays the actual hook-consent request to the operator; consent is distinct from permission for the later terminal commands. The native allowlist is `shell-hooks-allowlist.json` in the owned profile. `shell_hooks.py:42,153-180,239-260` and the official hook contract use exact event/command identity, not a script-content hash. A reviewed script must remain the intended artifact; editing its contents does not itself revoke native consent. Hooks execute with the OS user's privileges, not in a new sandbox.

`hermes_cli/plugins.py:1701-1717` keeps plugin managers per resolved profile home. `gateway/session_policy.py:747-766` re-reads the owning profile's live config and consent on session open; `gateway/run_profile_reconcile.py:136-162` re-syncs changes for served profiles. `shell_hooks.py:180-237` updates that home's registrations and retains existing fail-closed registrations when configuration/consent reads are incomplete. `gateway/session_policy.py:769-781` also registers worker-side hooks. This is source support for a profile-local extension on an already-running gateway, not observed gateway isolation or propagation.

### Director-authorized native shell path

`hermes_cli/cli_info_mixin.py:520-560` sends typed bang commands to `bang_shell.py`, which runs its local command after `tools.terminal_tool._check_all_guards` (`bang_shell.py:71-86`). It does not call the model tool-executor pre-hook pipeline. This supports a distinct native path for the Director's explicitly authorized command while the model's terminal call receives the extension's refusal. Native dangerous-command approvals, configured deny rules and mandatory guards still apply. Runtime M3 must establish the actual native output/exit status, approval behavior and zero model turns; this source trace grants no native-dispatch pass.

### Acceptance boundary and missing guarantee

This build exposes no demonstrated built-in positive terminal allowlist with prompt-free fallback refusal for ordinary interactive members. It exposes a supported extension that can express that policy once registered. Required live checks are: the exact hook is active in the owned gateway before a model command; an allowed broker command reaches the intended database; a disallowed command has no side effect and opens no approval prompt; callback/script failure refuses execution; another profile remains unaffected; and a Director-authorized native command retains normal guards and visible output. Required activation failures must fail the launch before agent work rather than silently accepting an unregistered policy.

The candidate cannot yet be described as equivalent to a mandatory backend permission map: initial consent/registration can be skipped, profile re-sync is best effort, and the outer hook-dispatch exception path can proceed without a policy decision. A reviewed design must explicitly bound those conditions or require native mandatory-policy support before claiming that guarantee. The current hooks-disabled contract remains in force; hook enforcement and actual normal-approval decisions remain unrun.

## Official documentation cross-check

The official CLI guide describes classic chat as gateway-owned execution and documents local bang-shell commands with no model turn and visible failing exit status. It separately documents the reduced gateway-client slash surface. Those statements motivate an exact path/build probe, rather than resolving installed behavior by documentation alone. Source: [CLI Interface](https://hermes-agent.nousresearch.com/docs/user-guide/cli), inspected 2026-10-25.

The official session guide says client disconnect detaches while accepted work remains gateway-owned. M4 must therefore prove cancellation and writer settlement for the owned profile. Source: [Sessions](https://hermes-agent.nousresearch.com/docs/user-guide/sessions), inspected 2026-10-25.

The official profile guide describes identity-file discovery and root-shared OAuth refresh state. These are prerequisites for the staged-profile/auth plan; runtime isolation and refresh write-through remain unverified. Source: [Profiles](https://hermes-agent.nousresearch.com/docs/user-guide/profiles), inspected 2026-10-25.

## Required evidence before Step 1 closes

Retain the exact argv/environment names, interface selection, normal approval/guard policy, provider/model/reasoning, OS/multiplexer, redacted captures, session identity/policy, and observed output/exit status already collected. Keep secret values out of this report and attribute Director-run results explicitly. M4 has a concrete current-contract failure. Preserve the passing M1/M3 subchecks and every unrun acceptance item; continue authorized settlement design, evidence and independent work, then resume dependent probes after the required specification/build review. Director support additionally requires D1/D2.
