# Member PTY Permission Events

**Status**: In Progress
**Progress**: 22/22 tasks complete
**Last Updated**: 2026-10-28

**Execution state**: Implementation and the release-minimum native acceptance are complete on branch `feat/0000194-member-pty-permission-events` (2026-10-28); the fresh Reviewer loop and user approval follow.

## Overview

Run members inside a CAFleet-owned PTY runner that observes native approval screens and durably notifies the Director independently of the member model. Preserve the original native decision surface, protect it from unrelated CAFleet input, and make native permission policy selectable per member while retaining current launch defaults. Each permission-request notification instructs the Director to assess the requested command and resolve it under the Director session's approval mode: auto mode permits a Director allow/deny decision, while other modes require the user's permission before answering. Separate observation, notification, Director consumption, the attributed decision and verified continuation.

## Success Criteria

- [x] A supported native permission prompt produces one durable opening event and one Director inbox record per observed episode while the member model is blocked, under either supported multiplexer.
- [x] Redraws, fragmented escape sequences, resize, identical consecutive requests and process replacement follow the episode identity rules below; ordinary errors, questions and startup prompts retain their own classifications.
- [x] Runner-managed delivery, prompt and native-exec paths preserve an observed native approval screen and send no Escape to dismiss it; uncertain or unsupported input states remain held.
- [x] `member press MEMBER_ID KEY...` delivers only the explicitly requested ordered keys through the selected multiplexer, with complete prevalidation, shared placement/input locking and bounded error reporting; wrapped panes account for those keys as actual manual input. No implicit Escape or Enter is added.
- [x] Each backend accepts its supported explicit launch-policy choices. Omitted selection preserves Claude `dontAsk`, Codex `never` with `workspace-write`, and OpenCode's existing `cafleet` preset.
- [x] Event persistence, notification delivery, message consumption and task continuation have independently observable outcomes. Permission notifications instruct the Director to evaluate the command and allow/deny in auto mode or request user permission in other modes; the selected answer reaches the original native request and the Director verifies actual continuation afterward.
- [x] Startup failure, runner loss, database contention, output pressure and member/fleet deletion leave explicit recoverable states rather than claiming a live observer or a completed approval.
- [x] The current-family migration preserves existing members/messages; existing live panes remain identifiable as unwrapped, and production documentation describes only the implementation that ships with this feature.
- [x] Contributor backend-extension guidance retains the pinned herdr implementation references, their purpose, typed-observation mapping and required CAFleet fixture/native acceptance.

---

## Background

The design started from source baseline `4a7a57409ad69b5d8e77088fc11c7762669b7332`. At that baseline, `CodingAgent::build_spawn_argv()` selected Claude `dontAsk`, Codex `--ask-for-approval never --sandbox workspace-write`, and OpenCode `mini --standalone --agent cafleet`, and `member::create()` passed that argv directly to the multiplexer. The source had neither a member PTY runner nor a per-member permission selector. Historical automatic-review work described in [0000192](../0000192-member-auto-approval/design-doc.md) was evidence of that earlier work, not proof that its launch policy or prompt-assembly wrapper existed at the baseline.

The baseline broker already persisted member-to-Director messages. `delivery::eligible_member()` limited automatic pane previews to ordinary members; Directors received through broker reads and their host's stdout receiver. Ordinary-member delivery and `member prompt` / `member exec` used Escape-first multiplexer input, which can dismiss the very approval screen this design preserves.

The supplied discussion, `cafleet-pty-permission-propagation-discussion-2026-10-26.md`, distinguishes historical source findings from proposed runner/event interfaces. This document chose those interfaces after clarification; the Implementation section records the shipped runtime and its native acceptance.

---

## Specification

### Scope and relationship to existing plans

CAFleet owns terminal observation, classification, episode tracking, persistence and notification. tmux and herdr own pane placement, display and ordinary terminal transport. The runner observes the child PTY directly; herdr detection APIs and tmux capture polling are diagnostic aids, not dependencies of event detection.

The first implementation includes launch-time policy selection, the runner, classified events, durable Director notification with actionable review instructions, prompt-preserving input arbitration, request-bound native answers and Director continuation checks. The Director evaluates and allows or denies a requested command in auto mode; other modes obtain the user's permission before an answer is sent. Native permission hooks and machine-authenticated request outcomes remain separate capabilities. Omitted member launch defaults retain their documented behavior.

| Existing plan or record | Relationship |
|---|---|
| 0000192 | Aborted at the user's request. Its 2/8 completed-task ledger and historical records remain intact; its unchecked runtime tasks remain incomplete. |
| [0000193](../0000193-backend-role-requirements/design-doc.md) | Preserve the Approved document. It plans a Member requirement for `auto` / `dontAsk` or equivalent; this design replaces that execution-mode prerequisite for runner-observed members after the runtime acceptance gates below pass. |
| Existing unwrapped members | Keep their recorded launch behavior and existing supervision procedures. They acquire neither a runner nor new detection guarantees while already running. |

The implementation documentation must reconcile the Member requirement from 0000193 with the final shipping behavior, whether that plan lands before or after this feature. For wrapped members, backend suitability requires the PTY lifecycle and an explicit detection-coverage declaration; the selected execution mode may be interactive. A backend may retain automatic or fixed-policy capability without requiring that every member select it. Preserve the Director requirements and its host-owned goal/receiver authority from 0000193. The implementation reconciled that Member requirement in the shipping documentation and skills, and changed no existing approval status except the authorized abort of 0000192.

### herdr references for backend extensions

Add backend-extension guidance to `docs/docs/contributing.md` that retains these implementation references from the supplied discussion, pinned to herdr commit `5608de0608918aba2f9d501e7793b984a0ca9cfa`. Explain each reference's purpose so future coding-agent adapters can use it without treating historical source as their current native CLI contract.

| Reference | What the contributor should learn |
|---|---|
| [Backend manifests](https://github.com/herdrdev/herdr/tree/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifests) | Backend-specific screen/title rules and examples of how controls and prompt shapes distinguish live regions from old transcript text. |
| [Manifest evaluator](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifest.rs) | Region extraction, rule priority and combined matching conditions; adapt those concepts to CAFleet's typed observations and explicit conflict behavior. |
| [Pane processing](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane.rs) and [terminal snapshots](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane/terminal.rs) | The path from PTY bytes through terminal state to detection snapshots, including live-screen and title evidence. |

herdr's `blocked` state also covers questions, trust screens and update interactions. Map those observations into CAFleet's separate episode/result/eligibility types instead of equating `blocked` with a permission request. An adapted rule requires CAFleet-owned fixtures for positive prompts, redraw/wrapping and false positives, plus exact-version native PTY evidence for every claimed detector/composer capability under the acceptance rules below. These references support development; CAFleet retains its own runner, profiles, classification, persistence and notification runtime.

### Launch policy and capability registry

Add `cafleet member create --permission-mode <MODE>`. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | Omitted selection | Explicit selection mapping |
|---|---|---|
| Claude | `dontAsk` | Use the native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `never` with `workspace-write` | Map native approval-policy values through `--ask-for-approval`; retain the explicit workspace sandbox. |
| OpenCode | Existing `cafleet` preset | Accept exactly `cafleet`, preserving the fixed preset; explicit `cafleet` records `mode_source=explicit`. |

`PermissionAdapter` is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping, native-version eligibility and detector/composer profiles. Generate help/error choices and table-driven coverage from that catalog. For the first release, Claude accepts native `--permission-mode` values verified from installed help and parser evidence, including documented aliases, recorded by its version-qualified adapter. Claude Code 2.1.302 accepts canonical `default` and its `manual` alias; help displays `manual`, while the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. Codex accepts the native `--ask-for-approval` values read from the installed `codex --help` and recorded the same way. OpenCode accepts exactly the token `cafleet`; every other explicit token fails with usage exit `2`. Test exact argv rather than treating the earlier discussion as an API reference, preserving native/managed restrictions and existing sandbox boundaries.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent. `effective_policy_json` remains null unless an actual native observation supplies it; an installed version, argv or selected profile alone proves no effective authorization decision.

Keep launch, detection, empty-composer input and request-bound answer capabilities separate. Answer profiles declare support for each semantic decision independently. Profiles declare exact backend versions tested, rule-set version and fixture IDs. A missing version/profile yields `coverage=unknown`, an initial coverage notification and held programmatic input; it still permits manual terminal operation and records the selected mode. Automatic/fixed policies remain useful even where screen detection is unknown. The default OpenCode preset's returned denials continue through member evidence routing; this design promises no universal detection of tool responses absent from its terminal.

### Runner boundary and creation transaction

Use an internal, hidden `cafleet __member-run --descriptor <ABSOLUTE_PATH>` command. It accepts only a descriptor allocated by `member create`; the public launch interface remains `member create`. Its dispatch validates storage but bypasses ordinary command deadlines for its long-lived event loop.

```text
tmux/herdr pane <--> CAFleet runner <--> child PTY <--> native agent CLI
                         |
                         +--> terminal state --> detector --> event writer
                         +<-> local control socket                 |
                                                                 v
                                                      SQLite event + inbox message
                                                                 |
                                                      Director stdout receiver
```

Use `portable-pty` for the child PTY and `vt100` for terminal state, behind CAFleet-owned traits so byte-level and fault tests use deterministic substitutes. Their upstream documentation establishes the intended responsibilities: [portable-pty](https://docs.rs/portable-pty/0.9.0/portable_pty/) supplies PTY/child interfaces; [vt100](https://docs.rs/vt100/0.18.1/vt100/) supplies an in-memory terminal screen. Resolve compatible versions through Cargo and commit the resulting lockfile during implementation. Deny `unsafe_code` crate-wide and use safe APIs for terminal modes, signals and process handling. The single exception is one audited `sysctl(CTL_KERN, KERN_PROC, KERN_PROC_PID)` call that reads a process's start time and state on macOS, because every safe start-time interface applies a same-effective-user check that fails for setuid descendants in the owned process group; it carries an item-scoped `#[allow(unsafe_code)]` and its invariants inline.

Store runtime artifacts in `.cafleet-runs/<database_namespace>/<run_id>/` beside the canonical selected database. The migration creates a singleton `runtime_database_identity` row with a random 128-bit `instance_id`; the database identity is the canonical path, filesystem device/inode and this stored instance ID. Compute `database_namespace` as the first 26 base32 characters of its SHA-256 digest. A private namespace manifest stores the complete identity; a prefix collision with a different manifest fails explicitly rather than sharing artifacts. Distinct databases under one parent therefore remain separate even with equal run IDs.

Directories use `0700`, descriptor/spool files use `0600`, and a control socket is accessible only to its owner. Create each run directory exclusively and reconcile an existing directory before reuse. Reject symlinks, ownership mismatches, malformed descriptors and socket paths exceeding the platform limit against the complete resulting path before pane creation. The descriptor contains schema version, complete database identity, fleet/member/Director/run identities, ownership epoch, a random launch nonce, literal child argv, absolute cwd, required environment overrides, evidence policy and initial terminal dimensions. Preserve the existing inherited native configuration; avoid copying the entire environment or creating per-member native profiles.

Replacing a database file creates a different filesystem identity and namespace even when restored bytes retain the instance ID. Runners validate the selected path's file/instance identity before input and storage mutations; replacement fences the old run from the new database, holds input and initiates owned-child cleanup. Retain the old namespace for explicit reconciliation; a new creator never adopts its socket or outbox. A copied database at another canonical path also receives a separate namespace. Test equal run IDs across sibling databases and replacement at an unchanged path.

The descriptor and control socket are local coordination mechanisms, not an authorization boundary against the same operating-system user. Require matching identities and the nonce to reject stale endpoints. The creator's guarded cleanup deletes the descriptor on every outcome: success, handshake failure, timeout, rollback and uncertain post-`start` timeout. Failed run records retain only the non-secret `permission_argv_json`, never the assignment-bearing prompt element or raw environment. Report a failed unlink explicitly. After a creator crash, `member recover` removes the descriptor only after identity/ownership validation; read-only `doctor` reports the leftover and its recovery action. The runner removes its loaded descriptor after preparation when safe, independently of the creator's idempotent cleanup.

During verified ordinary recovery, retain the original ownership lock and pin a leftover launch descriptor by its opened file identity and bounded content. Validate the recorded namespace/directory, private regular file, immutable database/fleet/member/Director/run identity and nonce. A positive historical descriptor epoch at or below the captured current epoch remains valid across takeover and retry; native launch loading still requires current creator ownership and the exact current epoch. Expected absence is idempotent while the pathname remains absent.

Before unlink, revalidate the fixed current run/placement, original lock/directory, named file type/permissions/device/inode/content and original deadline. Remove the pinned descriptor before reporting complete cleanup. Preserve a replacement or symlink and return an explicit identity error; report unlink failure explicitly and retain incomplete, retryable recovery. Keep assignment and environment content out of diagnostics. Exceptional database-only reconciliation leaves surviving unverified descriptors untouched.

Creation follows this ordered protocol:

1. Validate assignment input/placeholders, backend policy eligibility, runtime paths and backend/multiplexer prerequisites. Preserve one assignment substitution and one literal prompt argv element.
2. Register the pending member and allocate its run row in one transaction. Write the private descriptor and call `split_window()` with the absolute current CAFleet executable and the internal runner argv.
3. The runner validates the descriptor, acquires the precreated identity-pinned ownership lock, records its process identity, opens the PTY, initializes terminal state and event machinery, and binds its control endpoint. It reports `prepared` without spawning the native CLI.
4. The creator connects, validates the nonce/run identity, and publishes the concrete pane placement with `status=prepared`. Only then send the control `start` frame. The runner revalidates placement, spawns the child and records `running` before reporting `started` with child PID/start identity.
5. Return creation success only after `started`, meaning child spawn succeeded without a native-policy/liveness guarantee; include the run identity and detection coverage with the member result. A child that immediately exits is reported by `runner_exited`, including its native status and Director notification. Use a ten-second monotonic handshake budget and preserve existing rollback/cleanup diagnostics if any phase fails.

Until `start`, the runner exits if its creator disconnects or the handshake expires. Failure after child spawn kills and reaps that owned child, closes the known pane, deregisters the pending member and retains a failed run record. A failed cleanup retains the original failure plus explicit orphan/pane diagnostics. A timeout after `start` is an uncertain launch: inspect the existing run and stop its owned child during compensation; creating a second child is not a retry mechanism. Registration and pane ownership remain guarded until the handshake is complete.

### Terminal transport and lifecycle

The runner relays terminal input and child output as bytes, with the outer terminal in raw mode under a restoration guard. Preserve interactive control sequences, UTF-8 fragments, bracketed paste and alternate-screen transitions. Size the PTY from the outer terminal, propagate resize to both PTY and parser, and preserve the native child's cwd, intended environment and exit status. Keep child bytes and runner diagnostics on separate logical channels; diagnostics must not become child input or detector evidence.

The PTY-reader path updates terminal state before publishing the same bytes to display and passes immutable observations to the classifier. Track cursor/cell state, primary/alternate screens, OSC title and a monotonic output sequence. Keep scrollback out of live-prompt matching. Terminal queries and replies continue through the terminal transport, and parser callbacks must not synthesize a second reply to a query already handled by the outer terminal.

Separate terminal transport, classification and event storage. Database waits, Director receiver loss and event-spool retries must not stop draining child output or accepting a human's native answer. Use bounded byte buffers and ordinary terminal backpressure for a slow outer display; a broken outer terminal triggers owned-child cleanup. Classification may coalesce intermediate working-screen snapshots, but it processes every potential interaction transition from the ordered parser stream before such coalescing. This is an observation contract for output actually rendered/read, not a guarantee to capture a dialog opened and closed between observable states.

| Condition | Required lifecycle behavior |
|---|---|
| Native child exits | Record its actual code/signal immediately, then apply the bounded drain/cleanup protocol below and close the episode with `process_exited`. Restore the outer terminal independently of storage completion. |
| Runner receives termination or outer EOF/HUP | Stop programmatic input, terminate the owned process group, allow two seconds for cleanup, then kill verified remaining owned children and reap. Record the real termination cause. |
| Runner fails while child is live | Mark observation degraded, stop programmatic input and attempt the same owned-child cleanup; return failure rather than a fabricated native success. |
| Runner is killed without cleanup | Closing the PTY may cause native hangup, but descendants may survive. Mark reachability suspect until exclusive recovery establishes loss; inspect verified PID/start identity before signaling. |
| Member/fleet deletion | Acquire existing pane locks in their established order, revalidate placement/run, request runner shutdown and finish pane/registry cleanup under existing semantics. Report any unconfirmed child cleanup explicitly. |
| Restart or replacement | Create a fresh member/run after explicit reconciliation. Keep earlier episodes/events; a process generation never inherits an earlier pending request or causes automatic command re-execution. |

Use one shutdown clock starting at normal native-child exit, runner failure or termination request. Stop programmatic input immediately. During the first two seconds, drain available PTY bytes and terminate verified surviving descendants in the owned process group: send TERM immediately and KILL any verified survivors at the two-second boundary. Close the master at that boundary even if a descendant retains the slave. Nonblocking outer-display flushing ends by the three-second boundary; discard only undelivered display-tail bytes, record their count and retain a truncation diagnostic. Event-worker flush/SQLite attempts end by the four-second boundary; retain complete uncommitted outbox frames for explicit recovery and record any unsaved in-memory gap. All waits share these absolute deadlines.

Run every runner SQLite attempt under `input::with_database_budget` using the remaining absolute work/shutdown deadline; it replaces the connection's default five-second busy wait with bounded waiting and restores the prior handler afterward. Use the listener's deadline-aware progress-handler/cancellation pattern, with an interrupt handle for a long-running statement, and restore those hooks after each attempt. Shutdown attempts share the four-second boundary, including transaction completion and retry delays; they receive no additional grace budget. Test a held write transaction and a long statement to prove restoration/exit are independent of the connection's default timeout.

Restore terminal settings and release ownership after stopping/joining the writer, regardless of EOF, display or database completion. Retain actual child code/signal separately from `cleanup_state=complete|incomplete` and bounded diagnostic reasons for truncated output, pending events or unconfirmed descendants. When cleanup/restoration is complete, return the native status (`128 + signal` for a signal exit). With incomplete cleanup, preserve a native nonzero return; use runner exit `1` if the native status was zero or unavailable, and report its distinct recorded native status. Hard-kill or restoration failure remains an explicit host-recovery condition. Test a child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, rather than writing a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` remains a registry read, returns the recorded fields and requires neither a socket nor a multiplexer. `doctor` may perform nonblocking identity-pinned lock and bounded socket probes, reporting their observed results without mutating registry state. Input dispatch requires a fresh matching health/ownership check; its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

The creator precreates `owner.lock`, records its device/inode and runtime-directory identity, and records the current owner PID/start identity. Every ownership transfer advances `owner_epoch` with a CAS, including creator-to-runner transfer after the runner verifies/acquires that inode; the runner's `prepared` reply reports its new epoch for subsequent control frames. Recoverers likewise record their identity and advance the epoch. The runner holds `RunLock` through every writer/input thread and bounded shutdown. Validate the open descriptor's identity against both registry and current pathname; reopening a replacement lock is not acquisition of the original lock. A held matching lock returns `live_or_unreachable`; a deleted directory/lock or changed inode requires the distinct missing-path recovery branch below. An open unlinked inode may still have a live owner.

Normal recovery acquires the identity-matching lock before a CAS epoch takeover. Every event mutation validates complete database identity, owner kind and epoch; input additionally validates its ownership/lock identities immediately before child writes. A delayed live runner retaining the original lock prevents normal takeover. The missing-path branch also requires verified original-owner termination before fencing; missing files alone establish no exclusive ownership or safe input. Every takeover records the recovering process identity and advances the epoch, rejecting stale writes/input.

Only the current lock/epoch owner allocates event sequences. Confirmed loss and its events are produced by the explicit recovery procedure below after exclusive takeover, ordered after complete outbox replay. Concurrent reconcilers contend for the same lock and CAS rather than allocating competing loss events. PID/start identity assists child reconciliation; it never substitutes for ownership. Test timeout followed by the original runner's recovery and concurrent recovery attempts. There is no unattended restart or approval/command replay.

### Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version. Return a typed observation with profile/rule identifiers, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Compile rules at startup and fail on invalid rule definitions. Bundle versioned profiles in CAFleet; remote rule updates and herdr manifest loading are outside this design.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_denied` | A tested permission-specific returned-result region | A displayed refusal, not an arbitrary nonzero exit and not proof the user refused. |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `trust` / `update` | A tested startup trust or update form | A distinct native interaction, preserving its own consequences. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Hold programmatic input and request inspection. |
| `working` / `composer_empty` / `unknown` | The corresponding tested activity/composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The exact SQL/Rust episode-class domain is `permission_request`, `question`, `trust`, `update`, `unknown_interaction`; `trust` and `update` are distinct values. `permission_denied` is a returned-result observation recorded only through `permission_denied_observed`, with a nullable episode link. `working`, `composer_empty` and `unknown` are input-eligibility states and create no interaction row.

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and test wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. Unsupported versions advertise unknown coverage; missing matches never imply that all requests were observed.

An episode is a CAFleet observation identity, `(run_id, episode_seq)`, not a native request ID. Allocate `episode_seq` monotonically within a run when a stable live interaction first appears. Use two compatible observations at least 100 ms apart before opening; recheck after 100 ms even if no bytes arrive. Gate programmatic input immediately on the first candidate, before this notification debounce.

The fingerprint contains backend/profile, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration and line-wrapping differences. Redraws and resize matching the fingerprint update the stored evidence without another opening notification, but only when the post-policy evidence changes beyond its output sequence; a cursor transit through rows outside an unchanged live region keeps the request's generation and token. A strong operation/choice change supersedes the previous episode and opens a new one. After a verified native non-interaction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility. A two-second uncertain interval emits one `interaction_obscured` event. |
| Verified non-interaction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Close with `superseded` and open the next episode, in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Returned permission refusal appears | Emit `permission_denied_observed` once per stable result/boundary. Link an episode only when native evidence supplies the correlation; otherwise leave it null. |

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message ACK and resumed animation establish none of those outcomes by themselves. In this first release, machine decision outcome remains `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

### Persistence, notification and read interfaces

Extend the existing `cafleet_v8.db` format with the next contiguous migration after the embedded chain head, currently V3. Preserve its application identity and existing rows; update schema-shape guards and migration tests. Add the singleton `runtime_database_identity(singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1), instance_id TEXT NOT NULL)` and initialize its random 128-bit hex instance ID once during migration. Add these tables and typed Rust records:

| Table | Required fields and constraints |
|---|---|
| `member_runs` | `run_id INTEGER PRIMARY KEY AUTOINCREMENT`; member/fleet/Director foreign keys; backend, launch nonce, canonical runtime path, requested/resolved mode, mode source, permission argv JSON, native version, profile version, coverage, status and lifecycle/health timestamps; nullable runner/child PID plus verified start identity and exit code/signal. At most one nonterminal run per member. |
| `member_interactions` | `interaction_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, `episode_seq`, classification, post-policy evidence digest, opened timestamp, nullable closed timestamp/reason, and latest bounded evidence JSON. Unique `(run_id, episode_seq)` and at most one unclosed interaction per run. |
| `member_runtime_events` | `event_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, positive `event_seq`, nullable interaction foreign key, `kind`, observed/committed timestamps, versioned payload JSON, notification state and nullable unique `message_id` foreign key. Unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `input_id INTEGER PRIMARY KEY AUTOINCREMENT`, run foreign key, unique `(run_id, operation_key)`, kind, payload digest, nullable message foreign key, durable receipt/settlement state and timestamps. Retain settled rows as replay-rejection records for the run's lifetime. |

Use the enum values specified in this document as SQL CHECK and Rust conversion contracts. `member_runs.status` is `starting`, `prepared`, `running`, `degraded`, `stopping`, `recovering`, `exited`, `failed` or `lost`; terminal states are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; retain these foreign-key-linked records under existing database ownership/retention behavior.

The table below fixes names/types beyond the primary-key fields above. Use `INTEGER` for identities, counters, PIDs and exit values; `TEXT` for enum/string/digest/JSON/timestamp values. Timestamps use UTC RFC 3339 and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Exact fields |
|---|---|
| Run identity/policy | Required `member_id`, `fleet_id`, `director_member_id`, `backend`, `database_identity_json`, `launch_nonce`, `runtime_path`, `resolved_mode`, `mode_source`, `permission_argv_json`, `event_evidence`, `coverage`, `status`, `created_at`; nullable `requested_mode`, `native_version`, `profile_version`, `effective_policy_json`. |
| Run lifecycle/ownership | Required `owner_epoch INTEGER` starts at 1, `owner_kind TEXT` is `creator`, `runner` or `reconciler`, `owner_pid INTEGER`, `owner_start_identity TEXT`, `reachability TEXT` is `reachable` or `suspect`, `cleanup_state TEXT` is `complete` or `incomplete`, `event_seq_reserved_through INTEGER` starts at zero, and `observation_gap_count INTEGER` starts at zero. Nullable `runtime_dir_identity_json`, `run_lock_identity_json`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`, `started_at`, `last_health_at`, `ended_at`, `exit_code`, `exit_signal`, `cleanup_diagnostics_json`, `recovery_origin_status`; the latter preserves the original lifecycle status while recovering. |
| Interaction | Required `run_id`, `episode_seq`, `classification`, `evidence_digest`, `opened_at`, `evidence_json`; nullable `closed_at`, `close_reason`. `classification` is an interactive class from the classification table; both closure fields are null together or present together. |
| Runtime event | Required `run_id`, `event_seq`, `kind`, `observed_at`, `committed_at`, `payload_json`, `notification_state`; nullable `interaction_id`, `message_id`, `archive_reason`. `notification_state=inbox` requires a message and null archive reason; `archived` requires no message and a closure reason. |
| Input operation | Required `run_id`, `operation_key TEXT`, `kind TEXT` (`preview`, `prompt`, `exec`, `permission_response`), `payload_digest TEXT`, `state TEXT` (`ready`, `in_progress`, `accepted`, `uncertain`, `settled`), `created_at`, `updated_at`; nullable `message_id`, `disposition TEXT`, `evidence_ref TEXT`. `ready` certifies a zero-byte attempt; settled disposition is `submitted`, `not_submitted` or `abandoned_uncertain`, and preview kind requires its original message ID. Permission responses additionally require the audit and episode-claim relationships specified below. |

Store process-start identities as platform-specific tokens in `TEXT`. JSON fields validate as objects except `permission_argv_json` and `cleanup_diagnostics_json`, which are arrays. New event payloads carry `schema_version=2`; readers validate v1 and v2 explicitly and return existing records and their associated message bytes unchanged on replay. `event_evidence` is `metadata` or `redacted`. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` or `unknown`; tested means the pinned profile passed its declared release gates, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership. Use partial unique indexes for the active-run and open-interaction invariants.

Runtime event kinds are `coverage_unknown`, `interaction_opened`, `interaction_obscured`, `interaction_closed`, `permission_denied_observed`, `runner_degraded`, `runner_lost` and `runner_exited`. The event payload has this versioned shape:

```json
{
  "schema_version": 2,
  "event_id": 481,
  "run_id": 37,
  "event_seq": 4,
  "member_id": 92,
  "fleet_id": 23,
  "director_member_id": 91,
  "kind": "interaction_opened",
  "interaction_id": 108,
  "classification": "permission_request",
  "observed_at": "2026-10-26T02:30:00Z",
  "source": "runner_screen",
  "native_request_id": null,
  "decision_outcome": "unknown",
  "evidence": {
    "backend": "codex",
    "native_version": "recorded-at-launch",
    "profile_version": "bundled-profile-version",
    "rule_id": "native-tool-permission",
    "screen_seq": 120,
    "evidence_policy": "metadata",
    "evidence_digest": "sha256-of-post-policy-evidence",
    "excerpt": null,
    "truncated": false
  },
  "director_action": {
    "kind": "review_permission_request",
    "instruction": "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward.",
    "inspect_argv": ["cafleet", "permission", "inspect", "108", "--event-id", "481", "--from-member-id", "91", "--json"]
  }
}
```

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null; screen parsing never fills it with invented execution data. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Add launch-only `--event-evidence <metadata|redacted>`, defaulting to `metadata`. Metadata mode persists only identities, classification, rule/profile/version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints used for live episode comparison remain in memory and appear in neither logs, outbox nor database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`; there is no fallback to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery. Test a command containing a known secret and assert its absence from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member, with the typed `source` distinguishing runtime observation from model-authored speech. Refactor the broker's internal validated insert so this transaction shares existing fleet/active-member checks. Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an `ObservedTransition` keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields; generated IDs and commit timestamps are not inputs to that comparison.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`; it retains the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

Add `cafleet event show <EVENT_ID> --json` for the complete durable event, notification/archive state, nullable associated message ID and current interaction/run state. Add `cafleet input show <INPUT_ID> --json` for a durable programmatic-input receipt. These IDs are positive database-assigned integers. Add a `runtime` object to member detail/create JSON containing run ID, resolved mode, status, recorded reachability, owner epoch, last health observation, coverage, current interaction ID and unresolved input IDs; use `runtime: null` for an unwrapped member. Append fields through presentation helpers and specify exact layouts/key order in shipping CLI documentation. Existing broker message envelopes and ACK semantics stay intact; the wrapped pane-preview text differs as specified below.

### CLI presentation contract

Member create/show JSON preserves existing fields and appends `runtime`. An unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id`, `resolved_mode`, `status`, `reachability`, `owner_epoch`, `last_health_at`, `coverage`, `current_interaction_id`, `unresolved_input_ids`, `owner_kind`, `owner_pid`, `owner_start_identity`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`. Process fields preserve their stored nullability. These registry-only fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket. Existing member text rows remain intact; a wrapped create/show appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE` on a separate line.

| Command | Ordered JSON keys and values |
|---|---|
| `event show` | `event`, `notification_state`, `message_id`, `archive_reason`, `run`, `interaction`; `event` is the complete versioned event payload, `run` is the current runtime summary above, and nullable `interaction` contains the stored interaction fields in schema order. |
| `input show` / `input reconcile` | `input`, containing the durable input-operation row in schema order. |
| `member recover` | `member_id`, `run_id`, `outcome`, `replayed_count`, `inbox_count`, `archived_count`, `status`, `cleanup_diagnostics`; outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`. A member with no recorded run fails explicitly before mutation. |
| `member prompt` / `member exec` | Preserve existing keys and append nullable `input_id`; a wrapped durable submission returns its input ID, and an unwrapped submission returns null. |

New read/recovery commands emit JSON when `--json` is supplied and compact human-readable text otherwise, with exact text layouts specified in the shipping CLI reference before implementation. Wrapped prompt/exec text appends `Input: INPUT_ID` on a separate line when a durable ID exists. Uncertain submission errors include the durable input ID when known, so a caller can inspect the receipt even though the command reports failure. Existing non-dispatch/uncertainty errors retain their exit category and add run/interaction/input context in the cause. Recovery outcomes distinguish incomplete or unreachable work from confirmed recovery; printed submission and settlement establish no native execution result.

Compact text layouts preserve the existing member rows and use these additional lines (`-` represents an unavailable optional value):

| Command | Text layout |
|---|---|
| `event show` | `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID`, the runtime summary line, then `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. |
| `input show` / `input reconcile` | `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`. |
| `member recover` | `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC`. |

Input-operation wire fields are ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response`; the final field is the nullable permission-response audit object specified below. Interaction wire fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`. Uncertain submission errors include `input_id=INPUT_ID` when the durable identity exists.

### Storage pressure and durability boundary

An event worker owns SQLite and a private append-only per-run outbox. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker fsyncs length-delimited, checksummed, redacted records before attempting SQLite; remove an outbox record only after its event/message transaction commits. On restart/reconciliation, replay complete valid records with their original run/sequence, discard an incomplete final frame with an explicit diagnostic, and fail on corruption in a completed frame.

The current owner reserves event-sequence ranges of 256 by advancing `event_seq_reserved_through` in a bounded ownership-checked transaction before using them. The classifier consumes the reserved range in memory; if the next range is unavailable it enters the defined observation-gap state while terminal I/O continues. Unused reserved values are valid gaps. Recovery allocates new sequences above this durable high-water mark as well as all committed/complete-spool sequences, so missing artifacts cannot cause reuse of a previously issued sequence.

Add the Director-owned explicit `cafleet member recover <MEMBER_ID> --json` command. It operates on the recorded run of an active or deregistered member, never starts an agent, and returns recovery counts, final lifecycle/notification state and cleanup diagnostics. A five-second invocation deadline bounds each recovery pass. Start one cleanup clock at entry to the common recovery coordinator, before snapshot, locking, process/health evidence and replay; its existing two/three/four-second boundaries share that origin and are clipped to the supplied original deadline, including a deletion caller's earlier deadline. Incomplete replay retains the spool and `recovering` state for the next pass. `doctor` diagnoses recovery needs without mutating them. Member/fleet deletion invokes the same recovery component after successful runner shutdown and before deregistration when its remaining deadline permits; otherwise retain the namespace for a later explicit pass.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the existing pane lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below; it is not a free-lock result.
2. After acquiring the matching `RunLock`, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, read the entire outbox in frame order, validate checksums/sequence identities and reconcile previously committed frames idempotently. Replay every complete frame by ascending `event_seq`, updating interactions before their later closures. Retain an incomplete tail diagnostic; corruption blocks the pass with its original bytes preserved. Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. After all complete original frames are reconciled, reserve new sequences above the durable reserved high-water mark, committed events and every valid complete outbox frame. Derive the terminal outcome from the original status and any replayed terminal lifecycle record. If neither establishes an exit/failure/loss, append a pending interaction's `monitor_lost` closure and then `runner_lost` through the same fsynced outbox/idempotent transaction path. Otherwise finish the recorded terminal outcome without inventing a loss; a replayed child exit supersedes the stale pre-crash running status. Repeated passes detect existing closure/loss records instead of generating them again.
5. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then compact committed frames and release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention and delete the outbox only when all complete frames are durably represented.

Only the lock owner reads/compacts/writes the outbox, including normal event-worker operation. An external pass cannot race the live writer. Test hard kill with complete uncommitted frames, replay of an opening before loss closure, concurrent recovery, interruption during recovery, and deregistration before replay with archived rather than fabricated inbox records.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity returns `live_or_unreachable`; unknown process identity returns `operator_reconciliation_required`. Preserve suspect state and the open interaction in both cases. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

For unreadable owner identity, expose the exceptional Director-owned `cafleet member recover <MEMBER_ID> --owner-terminated --expected-run-id <RUN_ID> --expected-owner-epoch <EPOCH> --evidence-ref <REFERENCE> --json` path. Require all four options together, with positive IDs/epoch and a nonempty reference to the operator's recorded attestation that the inspected owner and startup processes have terminated; incomplete options fail with usage exit `2`. Without that attestation, unreadable identity continues to return `operator_reconciliation_required`.

The Director obtains the run ID, owner epoch and PID/start identity from the inspected registry record and binds the operator evidence to that snapshot. Revalidate database/run identity, epoch and owner tuple after obtaining the SQLite write lock; a changed run or owner rejects the stale attestation rather than rebinding it. A matching live-process identity, held identity-matching lock or matching live health reply rejects the termination claim even with the flag. For an unchanged owner with unreadable identity and no positive live evidence, record `owner_termination_attestation` in `cleanup_diagnostics_json` with the original run/epoch, owner PID/start token, evidence reference, UTC timestamp and `source=operator_via_director`, then execute the database-only branch in the same CAS transaction. Preserve that provenance separately from machine-verified termination. This attestation enables bookkeeping closure of the identified owner only; it authorizes no child creation, input, approval answer or replay.

Once the recorded owner's termination is machine-verified or accepted through that explicitly attributed attestation, perform database-only reconciliation in one bounded immediate transaction, rereading the fixed database/run identity, placement, epoch, owner tuple and supplied attestation after obtaining the SQLite write lock. A completed terminal result uses the expectation-checked no-work return above; a concurrent live reconciler on unfinished recovery causes deferral. For unfinished recovery, advance the epoch and record the recovering process identity, retain separately attributed provenance, increment the gap count once and add `runtime_path_missing` to cleanup diagnostics.

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code/signal and existing event/message bytes. When a terminal outcome is established, finish that outcome with zero new event counts. Otherwise reserve sequences above the durable high-water mark, close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry. Test a live unlinked lock, verified death, unknown identity without attestation, matching/stale attestations, positive live evidence rejecting an attestation and concurrent database-only recovery.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, while continuing manual terminal I/O; resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic, and preserve the actual error. Neither endless memory growth nor a claim of successful notification is a recovery strategy.

### Explicit member key input

The user selected `cafleet member press MEMBER_ID KEY...` on 2026-10-27 as the explicit key-input counterpart to `member prompt` and `member exec`. Request only the specified ordered keys; append neither Escape nor Enter. Use one writing invocation of the selected multiplexer adapter (`herdr pane send-keys` or `tmux send-keys`). Success means the backend acknowledged the key request; actual PTY bytes, native approval and command completion require separate observation. This boundary accounts for mode-dependent encoding and backend key interception. The command is explicit operator input, distinct from request-bound automatic permission decisions and from a prompt or shell-command submission.

Accept one through64 keys, each at most32 UTF-8 bytes. The common grammar consists of a single printable ASCII character U+0021–U+007E, Space, Escape, Enter, Tab, Backspace, Up/Down/Left/Right, F1–F12, Ctrl+a through Ctrl+z, and Shift+Tab. Preserve literal character case; match named keys/modifiers case-insensitively and normalize control letters to lowercase. Validate the complete vector before dispatch. Map characters to tmux hexadecimal keys, including punctuation, and map herdr plus/minus/semicolon to its named forms; the implemented typed key registry owns the grammar and adapters. Invalid tails produce zero writing invocations. Once the writing process starts, failure or timeout leaves delivery uncertain, including any possible prefix; preserve diagnostics and require explicit operator action instead of automatic retry.

Bind dispatch to the active revalidated member placement under the existing canonical PaneLock and deletion ordering, using one inherited-or-new five-second absolute deadline through database/lock checks, backend execution and bounded cleanup. Wrapped members receive these keys through their outer pane and actual manual-input path; generation accounting comes from bytes actually read. Backend acknowledgment supplies neither a byte count nor a permission receipt. Preserve the existing Director-target restriction and fail before writing when the selected backend/server context cannot establish the recorded placement.

Before authoring its tests, specify the supported key grammar and backend mapping, placement validation and shared input-lock/deletion ordering, bounded dispatch and partial-delivery errors, and how wrapped panes account for this input through their real manual-input path. Preserve the current host's command authorization. `permission respond` remains the request-bound allow/deny interface; choosing this new command supplies no permission to answer a pending native request.

### Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket rather than `mux.send_prompt()`. Retain the existing outer `PaneLock` and placement revalidation for ordering with deletion; the runner serializes the final check and child writes with its own terminal-state loop. The lock order is pane lock, short database validation, then bounded socket request; the runner never waits for the external pane lock while serving that request.

Control frames are length-prefixed JSON, capped at 64 KiB, with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. Encode the serialized UTF-8 JSON body length as a four-byte unsigned big-endian prefix; the inclusive 65536-byte body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. The runner supports `health`, handshake `prepare`/`start`, `receipt`, `submit_prompt`, `submit_exec`, `permission_inspect`, `permission_respond` and `shutdown`. `receipt` has `query` and `retire` actions. Submission replies distinguish `accepted`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence and byte/submission stage; the ID is null for a zero-byte attempt with no durable row. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state, permits reevaluating input eligibility. A held/rejected zero-byte attempt creates no side-effect receipt; if a row was already reserved, move it to `ready` only after positively recording that zero bytes were written.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may reconcile its retained stage evidence into the same row later; a crashed runner's in-progress row is never an automatic replay instruction.

Retire only the in-memory receipt after durable settlement. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout is merely a presentation of its input ID and supplies no durable acknowledgement. Retain settled operation keys/digests/dispositions for the run's database lifetime, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts to 4096, rejecting additional new side-effect attempts while that unresolved set is full; settled receipts leave the bounded set. Keep uncertain receipts until the Director records an evidence-based disposition through `cafleet input reconcile <INPUT_ID> --disposition <submitted|not-submitted|abandoned-uncertain> --evidence-ref <REFERENCE> --json`. This Director-owned operation only records disposition, settles the key and permits cache retirement; it sends no bytes, grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation acquires the pane/input lock and synchronizes with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently. If that owner is unreachable, first establish its termination/exclusive recovery under the same ownership rules; retain uncertainty while an active writer cannot be ruled out. Compare-and-swap the expected operation state in the settlement transaction, and reconcile a changed row before another attempt. Settlement records the observed input disposition, not the native command's execution outcome.

Held-preview retry remains owned by external `message send` / finite `message wait`, using the existing pending-preview rotation. The runner never fetches/stamps the next preview merely because an interaction closes. After an attributed Director/user response or manual native answer and verification of the resulting native state, consume/ACK available inbox records and explicitly invoke finite `message wait <DIRECTOR_ID>` to retry pending previews. If wait returns new inbox records first, consume/ACK them and re-enter that receive step; if the composer remains busy, previews stay held for a later receive step. This preserves the scope of the runner as observer and input arbiter.

A process loss, ownership change or unavailable receipt permits no automatic replay of an attempted submission. Keep notification unstamped, retain its durable uncertainty and reconcile actual output before a separate deliberate action. Explicit prompt/exec use generated keys and printed durable input IDs with the same rule. Test cross-invocation accepted-then-failed-stamp recovery, held-then-eligible delivery triggered by an external wait, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.

| Current runner input state | CAFleet-generated input |
|---|---|
| Permission candidate or open interaction of any class | Hold previews; reject explicit prompt/exec before writing any byte, identifying the current interaction or candidate. |
| Working, draft composer, unknown screen/profile, suspect reachability, degraded/recovering/lost observer | Hold/reject before writing; preserve the displayed native state. |
| Verified empty native composer with no interaction or background-work evidence | Permit one literal submission through the tested backend composer profile; recheck unread output and state immediately before writing. |
| State changes after a submission begins | Stop remaining planned input, return `uncertain` with the written stage and require inspection before retry. |

Wrapped submissions send literal text and the native submit key without a leading Escape. Native-exec submission retains `! <command>` only for a backend with a tested native-shell composer profile.

The exact wrapped preview line is:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Retain the existing body folding, inline limit and long-body `message show` pointer. Wrapped previews end at the resume clause because their submission sends no Escape. Unwrapped previews retain their existing full line and Escape-dismissal explanation. Update `SPEC.md`, the message-envelope reference and the multiplexer push-notification reference for these two owned rendering paths. Held messages keep `notified_at` unset, and only confirmed submission or the existing ACK settlement stamps them. Holding supplies no new permission event and never closes the native request. Native-exec success means submission, not execution success.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees suppression for observed/candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race, rather than asserting universal prompt preservation. If a profile cannot safely distinguish an empty composer and its concurrent-work boundary, that profile supports manual input only. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; ordinary preview/prompt/exec input remains held during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation. Retain the legacy mux path only for explicitly unwrapped members and preserve its narrower existing semantics.

### Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative. Mode/source fields are attributed audit claims in this same-user coordination tool, not authentication credentials.

Add a broker-generated `director_action` field to runtime event payload schema version 2. A permission-request `interaction_opened` event has `kind=review_permission_request`, a constant `instruction` and `inspect_argv`; every other new event has null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." Build inspection argv from validated stored identities, never from screen text. Validate the action against its event/run/Director association. Preserve existing schema-version-1 records, their exact message bytes and idempotent replay; all new events use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. Metadata/redacted evidence may omit necessary facts; use authorized current capture and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction. Apply the selected evidence policy to inspection output and persistence; store bounded opaque evidence references and decision metadata rather than copying raw commands or secrets into receipts.

The Director-only CLI is:

```text
cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --json
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY --json
```

Require positive IDs/epoch, nonempty operation keys and bounded nonblank references. `director-auto` requires mode `auto`; source `user` records a user decision in any mode. Validate the active pinned Director/fleet/member relationships. `inspect` verifies the durable event/message association, acquires/revalidates the concrete pane lock and probes the pinned owner without sending child bytes or reserving a side-effect receipt. Return request identities, current visibility/classification, detector/answer-profile versions, supported decisions/capability reason, post-policy evidence, missing-evidence indications and an opaque request token. Unsupported capability is an explicit result.

The runner issues a random token bound in memory to the complete database/run/nonce/epoch, current interaction/episode, native version, answer profile, live operation/choice fingerprint and relevant native input state. The token is neither a hash of hidden text nor an authorization credential. Revoke it on request/semantic change, obscuring, human input, degradation, ownership change or runner loss. A tested equivalent redraw/resize may retain it. A revoked token requires fresh inspection and decision reconciliation; a caller cannot rebind a stale command automatically.

Use versioned control operations `permission_inspect` and `permission_respond` and the existing receipt query/retire boundary. A five-second command deadline covers locking, database work and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer profile, and commits the response receipt, audit and episode claim atomically under the current ownership epoch. Other interaction classes, uncertain state, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's single-request answer plan, rechecking relevant state before each stage; partial writes or ambiguous state retain durable uncertainty. The CLI accepts semantic allow/deny decisions, not arbitrary key sequences.

Add a contiguous migration after V4 that preserves existing records and expands the input-kind domain with `permission_response`. Add `member_permission_responses`, keyed by `input_id` with event/interaction foreign keys, expected owner epoch, request token, decision, pinned Director ID, attributed Director mode/source, mode/decision evidence references, answer-profile version/rule and decision timestamp. Add `member_permission_response_claims`, keyed by interaction ID with a unique input foreign key, to retain one possibly transmitted answer per observed episode across different operation keys. Validate same-run/event/interaction relationships and all required enum/identity constraints transactionally. Shipping storage documentation pins the exact SQL names/types before test authoring.

Reuse input receipt states, settlement dispositions, the 4096 unresolved limit and synchronized retirement. Digest canonical non-secret request identity, token and decision metadata, excluding screen text and literal key bytes. The Director records the operation key before dispatch. Validate a retained key against its recorded identity/digest before current-request eligibility, pane locking or live-owner probing; return the matching retained result after token revocation, closure, deregistration or owner exit without typing, and reject conflicting reuse. Historical audit reads validate recorded relationships while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation. A different key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim. Release a claim only after synchronized evidence proves zero bytes and no active writer; a later deliberate attempt uses fresh inspection and a new key. Recovery sends no answer keys. Key submission and recorded `decision=allow` do not establish native approval or command success.

Append nullable `permission_response` audit data to the ordered input-operation output, null for other input kinds. Inspection returns a `request` wrapper. Response output identifies status (`accepted`, `held`, `rejected`, `uncertain`), nullable input ID, interaction/run, decision/source, submission stage and output-sequence evidence. Exit 0 means accepted key submission or the matching previously submitted receipt; usage failures exit 2 and operational/uncertainty errors retain existing categories and known input IDs. Pin complete ordered JSON/text contracts in the CLI reference before implementation. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability is exact-version and independently tested for allow and deny. It specifies request form/rule, single-request choice semantics, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes and answers to trust/update/question forms require their own explicit scope. Unsupported answer profiles preserve manual native input. The screen interface has no atomic native request API; serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

### Director consumption and continuation

Use the existing `message listen`, `poll --json` and finite `wait` interfaces. Runtime events reach the Director's stdout receiver and durable inbox; they introduce no composer injection, Escape-first wake, goal activation or new host interrupt guarantee. With a live receiver and available storage, test a target of two seconds from a stable prompt observation to the pending inbox row, and another two seconds to receiver output. Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

On a wake or poll, the Director reads the complete event, verifies its durable event/message link with `event show`, reconciles the current run/interaction, records the unfinished action in host-owned state, and ACKs the message as consumption. An ordinary member can quote event JSON; a claimed event identity is verified against stored ownership, never trusted from message text alone. Old/closed events remain readable but are not a new request to answer.

| Stage | Observable evidence | What remains outstanding |
|---|---|---|
| Detection | Rule/profile and current episode evidence | Durability and Director reception. |
| Persistence | Event ID plus committed inbox message ID | Director consumption. |
| Consumption | ACK plus Director-owned pending-action record | Risk review, decision and actual operation outcome. |
| Decision | Director auto-mode evidence and attributed decision, or the user's request-specific decision | Answer-key submission and native result. |
| Answer-key submission | Bound durable receipt/stage, or attributed manual native input | Native decision evidence and actual operation result. |
| Native decision observed | Explicit profile-supported native evidence, when available | Actual operation result and dependent continuation. |
| Episode disappears | Closure reason and current screen | Decision and execution outcome may remain unknown. |
| Continuation verified | Actual operation result/output and member's resumed dependent work | Any remaining task-specific work. |

For a permission refusal returned to the member, preserve the existing evidence route: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, verify the event and current request, establish operation/risk evidence and resolve the Director session's effective mode. Decide allow/deny in auto mode; obtain user permission in other or unknown modes. Submit the bound response or give the user its complete concrete command, then inspect native output/exit/error and the member's actual continuation. Unknown answer capability preserves manual original-surface input. Consume/ACK any new inbox records and run an explicit finite `message wait <DIRECTOR_ID>` after that inspection to retry held previews; a new received batch repeats that consumption/receive step. The listener alone performs no preview retries, and a still-busy composer retains its pending messages. A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.

### Integration and acceptance evidence

| Implementation owner | Change |
|---|---|
| `cafleet/src/coding_agent/mod.rs`, `spawn_prompt.rs`, `cli/member.rs` | Policy catalog/argv, literal assignment preservation, runner handshakes and creation rollback. Read current source before introducing the earlier plan's absent prompt wrapper. |
| New runner/control/detection modules; `runtime/`, `signals.rs` and `input.rs` | PTY transport, terminal restoration, profiles, identity, liveness, socket arbitration and lock integration. |
| `broker/`, migrations, `db/mod.rs`, `diagnosis.rs` | Typed runtime/input-operation records, durable receipt settlement, atomic inbox association, idempotence, schema compatibility and loss/orphan diagnostics. |
| `delivery.rs`, `receive.rs`, `cli/message.rs`, member/fleet deletion | Held previews, wrapped submission, unchanged Director stdout reception and coordinated shutdown. |
| `cli/mod.rs`, new event/input/permission CLI, `presentation.rs`, output helpers | Hidden runner dispatch, event/input reads, bound permission inspection/response, Director-owned receipt reconciliation and additive runtime detail contracts. |
| `docs/`, `SPEC.md`, affected skill cores/roles/references and presets | Document shipping behavior, wrapped/legacy preview rendering and contributor herdr references; reconcile the 0000193 Member contract in each independent documentation home. |

Use synthetic terminal replay to validate parser/classifier/episode behavior and real PTYs to validate transport, lifecycle and input arbitration. Test every registered backend's omitted-policy argv and each adapter-declared explicit policy; profiles claiming detection or composer support require both fixtures and real native evidence. Cover each multiplexer supported by the repository registry. Fixed-policy backends without an interactive profile must show their explicit unknown-coverage/manual-input behavior rather than borrowing another backend's acceptance.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Pin exact installed CLI versions by the acceptance records and corresponding bundled profiles; use permitted disposable fixtures that actually reach the native approval surface. Every minimum case must demonstrate blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and preserved original request. Exercise separate fresh requests for verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, and manual original-surface input; include a user-executed complete bound command. Record zero response bytes before required user permission, actual native results and observed continuation separately from submitted keys. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Record source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, Director mode/provenance, attributed decision/evidence references, request identity/token, receipt/stage, multiplexer, terminal size, cwd and redacted evidence for live acceptance. Include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. Exercise stale tokens/epochs/events, same/different operation keys, conflicting decisions, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.

---

## Implementation

> Check a task only after its stated work is complete, recording the completion timestamp in the same edit. Record documentation completion independently of production implementation and live-runtime acceptance.

### Execution record

Execution ran from 2026-10-26 to 2026-10-28 across several fleets and two machines (macOS, then Linux). This record keeps the facts that explain the shipped behavior and its acceptance. Raw evidence (captures, command plans, decision records and run logs) lives under `.execution/194/` as local-only audit material; it is not part of the repository.

#### Respond-binding defect and fixes

The first shipping-CLI end-to-end run (fleet 25, Claude Code 2.1.302 `default`, tmux) produced an `interaction_opened` event and a tested inspection, but every `permission respond` failed before any byte with `permission request token or current plan binding changed`. Three independent causes were found and fixed.

| Cause | Fix | Commits |
|---|---|---|
| Inspection issued or reused tokens without the respond-only conjuncts (visibility, held interaction eligibility, retained transition/draft/evidence, observation diagnostic, fenced input, failed manual accounting). | One shared blocker predicate names a reason per conjunct. A blocked inspection returns a null token, empty decisions, `answer_capability=unknown` and the reason. Respond fails with `permission request is blocked: REASON` at admission, after a required quiescent admission drain, and before each stage. Every pending drain names its reason, and a retained operation key replays before the admission drain. | `005d9090`, `8db0a496`, `7efe728f`, `309d1c22`, `145a0c4a` |
| Claude redraws a static form continuously; each redraw scheduled an evidence refresh, so the request never settled. | A redraw whose post-policy evidence differs only in `screen_seq` and its derived digest schedules no refresh and keeps the stored evidence and token. A clear/rebuild redraw still revokes the token. | `efd48ed0`, `8db0a496`, `38346834` |
| Claude 2.1.302 blinks a status bullet about twice a second; each write moves the cursor outside the form, the cursor-inside rule fails mid-write, and the request generation advanced. | Cursor transit: a frame that leaves every row of the live region unchanged keeps the request generation and token while the request is temporarily not answerable. The committed 441-byte native blink capture is a regression fixture. | `7cb3c6f9`, `e360aa0b`, `20529919` |

#### Further fixes found during acceptance

| Finding | Fix | Commits |
|---|---|---|
| The version probe and the launched child could resolve different native binaries (the pane's `PATH` selected another version). | `member create` resolves the backend executable once to an absolute path from the creator's `PATH`, probes that path and launches it as `argv[0]`. | `7efe728f`, `309d1c22`, `145a0c4a` |
| Shipping-surface review V1-V7: missing help descriptions; `member exec` help and usage order; closure overwrote the stored request evidence; schema-order references; undocumented doctor runtime checks; inspect error order and label. | Every argument of `event show`, `input show`, `input reconcile`, `member press` and `member recover` is described; `member exec` help states wrapped/unwrapped Escape behavior with `MEMBER_ID` first; a closure keeps the request evidence and carries the closing frame only in its event; explicit field orders and the `member_runtime` doctor checks are documented; inspect reports `Interaction N not found` first and labels an identity mismatch `member_runs.database_identity_json`. The broader doctor layout and setup output predate this branch and remain a separate follow-up. | `c355342e`, `9ce307ab`, `f393322b`, `7f73bf9a` |
| macOS `proc_pidinfo` returned `EPERM` for setuid members of the owned process group. | Process start identity reads `kern.proc.pid` through one audited `sysctl` call. | `09aeb8a0`, `0b83178d` |
| A runner failure with a live child did not persist `degraded`. | The run records `degraded`; recovery still finishes `lost`. | `d58127fc`, `66148ca1` |

#### Bundled profiles

| Profile | Native evidence | Answer stages | Commits |
|---|---|---|---|
| `claude-2.1.302-default-2` | Permission form, allow and deny pre-stage forms, idle composer (captured on macOS, rows 0-11 masked) | allow `31` (digit 1), deny `34` (digit 4) | `ec732cac`, `3a989b81`, `093335ef`, `c531a746`, `e3ccf252`, `b62562df`, `b25d94ed` |
| `codex-0.163.0-on-request-1` | Approval form, allow and deny pre-stage forms, idle composer (captured on Linux under the runner, 36x120, rows 0-11 masked; the form captures record the pending-wrap cursor at column 120) | allow `79` (`y`), deny `33` (digit 3, because the deny choice shows only `esc`) | `3cfb078b`, `0b03d52e`, `510f83d9`, `f1281ef4`, `e0de1d68`, `91c79928`, `8206ac1b`, `d6c61782` |

The native version probe at `member create` (`78010795`, `e229776f`, `d121603f`) selects the bundled profile; catalog-derived help shows each backend's policy choices (`cccc5ea1`, `cf6a9a34`, `edcabc0e`). A fresh Codex case folder showed no trust screen under the explicit `on-request`/`workspace-write` flags.

#### Native acceptance (2026-10-28, Linux)

Every case below used the shipping release build, a private database and a dedicated tmux server or herdr tab. Each member's child executable was the pinned native binary, its cwd was the case folder, and no Director session variable reached it.

| Backend and mode | Multiplexer | Decision | Observed outcome |
|---|---|---|---|
| Claude 2.1.302 `default` | tmux (240x80) | allow (director-auto) | `accepted`/`submitted`, `allow.executed` created, `interaction_closed` `not_visible` within 1 s, Claude reported the marker and returned to idle |
| Claude 2.1.302 `default` | tmux | deny (director-auto) | `accepted`/`submitted`, no marker, `Interrupted · What should Claude do instead?`, closed `not_visible` |
| Claude 2.1.302 `default` | herdr (dedicated tab, short case path) | allow / deny | The same outcomes as tmux |
| Codex 0.163.0 `on-request` + `workspace-write` | tmux (240x80) | allow / deny | allow: `✔ You approved codex to run <command> this time`, marker and gate created; deny: `✗ You canceled the request to run <command>`, no marker or gate; both closed `not_visible` |
| Codex 0.163.0 `on-request` + `workspace-write` | herdr (about 89 columns, choice 2 wrapping its `(p)` row) | allow / deny | The same outcomes as tmux |

User-attended cases (Claude 2.1.302 on herdr):

| Case | Observed outcome |
|---|---|
| User-sourced response (`--decision-source user`) | The first attempt was refused with zero bytes because opening the tab resized the pane and revoked the token; a fresh inspection and respond returned `accepted`/`submitted`, the marker was created and the episode closed. |
| User-executed bound command | The user ran the complete presented `permission respond` command; it returned `accepted`/`submitted` with zero Director response bytes, and the marker and closure followed. |
| Manual original-surface answer | The user chose `1. Yes` in the native pane; the marker and closure followed, inspection then fails closed and a respond with the earlier token is refused before any byte. |

Deleting each member produced `runner_exited` (native exit 143) with status `exited` and no surviving native process. An accepted response remains listed in `unresolved_input_ids` after the run exits; accepted is the intended durable end state. Live detection-to-inbox latency (from the stabilized opening observation to the committed inbox row) was 4.071-6.540 ms against the two-second target. Receiver output was observed through a finite `message wait`, not through a `message listen` receiver.

#### Environment facts

| Fact | Consequence |
|---|---|
| The acceptance machine is Linux x86_64, and the repository lives on an NFS mount that reports root-created files as owned by `nobody`. | The runner correctly refuses such files as an ownership mismatch, so private databases, sockets and harness inputs lived under `/tmp`. |
| Unoptimized dependency code made hashing the 158 MB Linux test binary exceed fixture budgets. | Dev and test builds optimize dependency crates at `opt-level = 2` (`0736822d`); release builds are unchanged. |
| The global Claude Code version was 2.1.304. | Claude 2.1.302 was installed side by side and placed first on `PATH`; other versions record `coverage=unknown`. |
| The user's Codex configuration routes approvals to the automatic reviewer (`approvals_reviewer`), which answers escalations without a native form. | With the user's approval it was disabled for the Codex captures and end-to-end runs and restored afterwards. |
| The container disallows unprivileged user namespaces, so the Codex `workspace-write` sandbox cannot run. | The captures and end-to-end runs exercised escalated requests, which open the approval form this feature handles. |

#### Test placement

At the user's direction, the necessary unit tests live inline under `cfg(test)` in the implementation module they exercise. Task-introduced standalone test files were relocated group by group without assertion changes (`fd5235af`, `fbfdce7c`, `1797353e`, `bfb33897`, `0f8c905a`, `b1fac3fc`, `7a1c49e9`, `ea5d6c22`, `de7299d3`, `cadad73b`); genuine integration boundaries (creation, fleet deletion, control service and recovery) remain separate. Load-sensitive test budgets were corrected in `226167a6`, `d8816375` and `a260b348`. The last Programmer full run before the Codex profile was 1135 passed / 0 failed / 7 ignored, with lint, typecheck and the documentation build passing.

#### Remaining limits

| Limit | Status |
|---|---|
| Wrapped Claude form (narrow pane or long command) | Not detected by `claude-2.1.302-default-2`; documented, with a wrapped-form profile as a follow-up |
| Codex command that wraps | Detection covers a single-line command only; documented |
| Native trust and update screens | Classified only by synthetic fixtures; no bundled native rule |
| Codex sandboxed (non-escalated) path | Not exercisable in this container |
| Listener-path receiver latency | Unmeasured; only finite-wait reception was observed |
| Ordinary command failure after allow, an actual refusal returned to the member, native deletion and recovery under load | Unobserved natively; covered by committed synthetic and process-level tests |

The fault-condition-to-test map is recorded in `.execution/194/step-6-fault-latency-evidence.md`. Design 0000192 remains Aborted with its 2/8 ledger unchanged since the branch merge-base.

### Step 1: Specify the shipping contracts

- [x] Inspect native backend help/source and pin adapter policy/profile contracts, including the minimum Claude `default` and Codex `on-request` coverage; preserve current omitted defaults and specify the metadata/redacted evidence interfaces. <!-- completed: 2026-10-26T10:20 -->
- [x] Update affected `docs/` pages, then `SPEC.md` and other first-class targets for policy/lifecycle/event/input contracts; specify wrapped and legacy preview lines in the message-envelope and multiplexer references. <!-- completed: 2026-10-26T10:45 -->
- [x] Add backend-extension guidance to `docs/docs/contributing.md` retaining the pinned herdr sources, each reference's purpose, typed `blocked` mapping and CAFleet fixture/exact-version native acceptance requirements. <!-- completed: 2026-10-26T10:38 -->
- [x] Reconcile the 0000193 Member requirement in affected skill and operator references independently, preserving Director eligibility, native authority and manual reception behavior. <!-- completed: 2026-10-26T10:45 -->
- [x] Specify the Director-mode permission response extension across owning documentation, SPEC and skills, including exact v2 notification, CLI, V5 audit/claim and answer-profile contracts. <!-- completed: 2026-10-26T13:50 -->

### Step 2: Add durable runtime records

- [x] Add the next current-family migration, database instance identity, typed runtime/ownership records and schema guards; preserve existing data and the ahead/incompatible-database refusal contract. <!-- completed: 2026-10-26T11:13 -->
- [x] Implement event/interaction/inbox transactions, archived recovery, sequence-range reservation and durable input dispositions/read interfaces with ownership checks and additive runtime presentation. <!-- completed: 2026-10-26T11:25 -->
- [x] Test retry/sequence conflicts, rollback/archival, historical retention, durable receipt retirement/replay rejection and secret-free metadata/redacted output across persisted fields. <!-- completed: 2026-10-26T11:28 -->

### Step 3: Own the member PTY lifecycle

- [x] Add safe PTY/terminal dependencies and runner transport abstractions; implement raw-terminal restoration, resize, ordered screen reconstruction, literal argv/cwd/environment and exact child exit reporting. <!-- completed: 2026-10-26T12:48 -->
- [x] Integrate namespaced descriptors/sockets, nonce/epoch validation and spawn-only handshake semantics; test descriptor removal on every outcome, fast native exit, sibling databases and database replacement. <!-- completed: 2026-10-26T14:35 -->
- [x] Implement read-only show/diagnosis, fresh input health checks, identity-pinned ownership and deadline-bounded shutdown/SQLite attempts; test delayed owners, long/blocked statements and descendants retaining the PTY. <!-- completed: 2026-10-26T14:33 -->

### Step 4: Classify and persist observed episodes

- [x] Add the contiguous permission-response migration and typed audit/claim storage, v2 actionable events and explicit v1 read/replay compatibility; test transactional ownership, episode claims and metadata/redacted privacy. <!-- completed: 2026-10-26T15:29 -->
- [x] Implement the backend policy catalog/selector and versioned classification/composer profiles, with exact default/explicit argv tests and explicit unknown-coverage behavior. <!-- completed: 2026-10-26T16:23 -->
- [x] Implement candidate gating, debounce, semantic episode fingerprints and typed visibility/refusal transitions; replay positive and adversarial terminal fixtures including indistinguishable-request limits. <!-- completed: 2026-10-26T16:23 -->
- [x] Implement bounded outbox/recovery with ordered replay, reserved sequences and archival; test hard kills, concurrent/interrupted recovery, live unlinked locks, missing paths, snapshot-bound operator attestations and manual terminal I/O. <!-- completed: 2026-10-26T22:09 -->

### Step 5: Protect original requests and consume events

- [x] Route wrapped preview/prompt/exec input through serialized frames, durable receipt settlement and bounded unresolved caches; preserve legacy handling and add Director-only input reconciliation. Implement explicit `member press` with the reviewed key grammar/backend mapping, shared lock and placement revalidation, no implicit keys, bounded dispatch errors and wrapped manual-input accounting. <!-- completed: 2026-10-27T22:54 -->
- [x] Implement bound permission inspection/response with independent exact-version answer profiles, current-token revalidation and durable episode claims; test auto/user attribution, zero-byte failures, human/output races and uncertain replay prevention. <!-- completed: 2026-10-28T06:30 -->
- [x] Test zero input to unsafe states, cross-invocation/stamp failures, externally retried held delivery, partial writes, retired-key rejection, lost explicit stdout and more than 4096 settled submissions. <!-- completed: 2026-10-27T22:54 -->
- [x] Update Director/member guidance for durable events, ACK consumption, mode-dependent decisions and bound responses, manual native input, result-based continuation and an explicit finite wait to retry held previews afterward. <!-- completed: 2026-10-26T13:39 -->

### Step 6: Validate and accept the runtime

- [x] Run the appropriate repository-root Rust test/lint/typecheck tasks and documentation build; verify schema/JSON/help and embedded assets match shipping behavior. <!-- completed: 2026-10-28T06:30 -->
- [x] Complete the mandatory Claude `default` and Codex `on-request` real-native cases on tmux and herdr, plus every additional claimed profile; pin versions and fail feature acceptance for a missing minimum detection/notification/continuation case. <!-- completed: 2026-10-28T06:30 -->
- [x] Record fault-injection and detection/receiver latency evidence, unsupported/unobserved cases and reconciliation limits; confirm 0000192 stays Aborted and historical ledgers remain unchanged. <!-- completed: 2026-10-28T06:30 -->
