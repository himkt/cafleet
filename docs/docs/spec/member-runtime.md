# Member runtime contract

This contract describes runner-managed member launch, terminal observations, durable events, recovery and programmatic input. [Member permission events](../concepts/member-permission-events.md) explains the operator workflow.

## Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | Omitted selection | Explicit selection mapping |
|---|---|---|
| Claude | `dontAsk` | Use the native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `never` with `workspace-write` | Map native approval-policy values through `--ask-for-approval`; retain the explicit workspace sandbox. |
| OpenCode | Existing `cafleet` preset | Accept exactly `cafleet`, preserving the fixed preset; explicit `cafleet` records `mode_source=explicit`. |

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping, native-version eligibility and detector/composer profiles. Help and usage errors report choices from that catalog. Claude accepts native `--permission-mode` values verified from installed help and parser evidence, including documented aliases, recorded by its version-qualified adapter. Claude Code 2.1.302 accepts canonical `default` and its `manual` alias; help displays `manual`, while the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. Codex accepts the native `--ask-for-approval` values read from the installed `codex --help` and recorded the same way. OpenCode accepts exactly the token `cafleet`; every other explicit token fails with usage exit `2`.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent. `effective_policy_json` remains null unless an actual native observation supplies it; an installed version, argv or selected profile alone proves no effective authorization decision.

Keep three capabilities separate: a mode can be launchable, its permission screen can have tested detection coverage, and its empty composer can have tested programmatic-input coverage. Profiles declare exact backend versions tested, rule-set version and fixture IDs. A missing version/profile yields `coverage=unknown`, an initial coverage notification and held programmatic input; it still permits manual terminal operation and records the selected mode. Automatic/fixed policies remain useful even where screen detection is unknown. The default OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

## Runner boundary and creation transaction

The internal, hidden `cafleet __member-run --descriptor <ABSOLUTE_PATH>` command accepts only a descriptor allocated by `member create`; the public launch interface remains `member create`. Its dispatch validates storage but bypasses ordinary command deadlines for its long-lived event loop.

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

The runner owns the child PTY and reconstructs terminal state independently of the multiplexer. Terminal modes, signals and process handling preserve native input and owned-child cleanup.

Store runtime artifacts in `.cafleet-runs/<database_namespace>/<run_id>/` beside the canonical selected database. The migration creates a singleton `runtime_database_identity` row with a random 128-bit `instance_id`; the database identity is the canonical path, filesystem device/inode and this stored instance ID. Compute `database_namespace` as the first 26 base32 characters of its SHA-256 digest. A private namespace manifest stores the complete identity; a prefix collision with a different manifest fails explicitly rather than sharing artifacts. Distinct databases under one parent therefore remain separate even with equal run IDs.

Directories use `0700`, descriptor/spool files use `0600`, and a control socket is accessible only to its owner. Create each run directory exclusively and reconcile an existing directory before reuse. Reject symlinks, ownership mismatches, malformed descriptors and socket paths exceeding the platform limit against the complete resulting path before pane creation. The descriptor contains schema version, complete database identity, fleet/member/Director/run identities, ownership epoch, a random launch nonce, literal child argv, absolute cwd, required environment overrides, evidence policy and initial terminal dimensions. Preserve the existing inherited native configuration; avoid copying the entire environment or creating per-member native profiles.

Replacing a database file creates a different filesystem identity and namespace even when restored bytes retain the instance ID. Runners validate the selected path's file/instance identity before input and storage mutations; replacement fences the old run from the new database, holds input and initiates owned-child cleanup. Retain the old namespace for explicit reconciliation; a new creator never adopts its socket or outbox. A copied database at another canonical path also receives a separate namespace.

The descriptor and control socket are local coordination mechanisms, not an authorization boundary against the same operating-system user. Require matching identities and the nonce to reject stale endpoints. The creator's guarded cleanup deletes the descriptor on every outcome: success, handshake failure, timeout, rollback and uncertain post-`start` timeout. Failed run records retain only the non-secret `permission_argv_json`, never the assignment-bearing prompt element or raw environment. Report a failed unlink explicitly. After a creator crash, `member recover` removes the descriptor only after identity/ownership validation; read-only `doctor` reports the leftover and its recovery action. The runner removes its loaded descriptor after preparation when safe, independently of the creator's idempotent cleanup.

Creation follows this ordered protocol:

1. Validate assignment input/placeholders, backend policy eligibility, runtime paths and backend/multiplexer prerequisites. Preserve one assignment substitution and one literal prompt argv element.
2. Register the pending member and allocate its run row in one transaction. Write the private descriptor and create the pane with the absolute current CAFleet executable and the internal runner argv.
3. The runner validates the descriptor, acquires the precreated identity-pinned ownership lock, records its process identity, opens the PTY, initializes terminal state and event machinery, and binds its control endpoint. It reports `prepared` without spawning the native CLI.
4. The creator connects, validates the nonce/run identity, and publishes the concrete pane placement with `status=prepared`. Only then send the control `start` frame. The runner revalidates placement, spawns the child and records `running` before reporting `started` with child PID/start identity.
5. Return creation success only after `started`, meaning child spawn succeeded without a native-policy/liveness guarantee; include the run identity and detection coverage with the member result. A child that immediately exits is reported by `runner_exited`, including its native status and Director notification. Use a ten-second monotonic handshake budget and preserve existing rollback/cleanup diagnostics if any phase fails.

Until `start`, the runner exits if its creator disconnects or the handshake expires. Failure after child spawn kills and reaps that owned child, closes the known pane, deregisters the pending member and retains a failed run record. A failed cleanup retains the original failure plus explicit orphan/pane diagnostics. A timeout after `start` is an uncertain launch: inspect the existing run and stop its owned child during compensation; creating a second child is not a retry mechanism. Registration and pane ownership remain guarded until the handshake is complete.

## Terminal transport and lifecycle

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

Run every runner SQLite attempt under a bounded database budget using the remaining absolute work/shutdown deadline; it replaces the connection's default five-second busy wait with bounded waiting and restores the prior handler afterward. Cancel long-running statements within the same deadline and restore connection hooks after each attempt. Shutdown attempts share the four-second boundary, including transaction completion and retry delays; they receive no additional grace budget.

Restore terminal settings and release ownership after stopping/joining the writer, regardless of EOF, display or database completion. Retain actual child code/signal separately from `cleanup_state=complete|incomplete` and bounded diagnostic reasons for truncated output, pending events or unconfirmed descendants. When cleanup/restoration is complete, return the native status (`128 + signal` for a signal exit). With incomplete cleanup, preserve a native nonzero return; use runner exit `1` if the native status was zero or unavailable, and report its distinct recorded native status. Hard-kill or restoration failure remains an explicit host-recovery condition.

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, rather than writing a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` remains a registry read, returns the recorded fields and requires neither a socket nor a multiplexer. `doctor` may perform nonblocking identity-pinned lock and bounded socket probes, reporting their observed results without mutating registry state. Input dispatch requires a fresh matching health/ownership check; its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

The creator precreates `owner.lock`, records its device/inode and runtime-directory identity, and records the current owner PID/start identity. Every ownership transfer advances `owner_epoch` with a CAS, including creator-to-runner transfer after the runner verifies/acquires that inode; the runner's `prepared` reply reports its new epoch for subsequent control frames. Recoverers likewise record their identity and advance the epoch. The runner holds the ownership lock through every writer/input thread and bounded shutdown. Validate the open descriptor's identity against both registry and current pathname; reopening a replacement lock is not acquisition of the original lock. A held matching lock returns `live_or_unreachable`; a deleted directory/lock or changed inode requires the distinct missing-path recovery branch below. An open unlinked inode may still have a live owner.

Normal recovery acquires the identity-matching lock before a CAS epoch takeover. Every event mutation validates complete database identity, owner kind and epoch; input additionally validates its ownership/lock identities immediately before child writes. A delayed live runner retaining the original lock prevents normal takeover. The missing-path branch also requires verified original-owner termination before fencing; missing files alone establish no exclusive ownership or safe input. Every takeover records the recovering process identity and advances the epoch, rejecting stale writes/input.

Only the current lock/epoch owner allocates event sequences. Confirmed loss and its events are produced by the explicit recovery procedure below after exclusive takeover, ordered after complete outbox replay. Concurrent reconcilers contend for the same lock and CAS rather than allocating competing loss events. PID/start identity assists child reconciliation; it never substitutes for ownership. There is no unattended restart or approval/command replay.

## Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version. Return a typed observation with profile/rule identifiers, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Compile rules at startup and fail on invalid rule definitions. Bundle versioned profiles in CAFleet; profiles are bundled locally rather than loaded from remote rules or herdr manifests.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_denied` | A tested permission-specific returned-result region | A displayed refusal, not an arbitrary nonzero exit and not proof the user refused. |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `trust` / `update` | A tested startup trust or update form | A distinct native interaction, preserving its own consequences. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Hold programmatic input and request inspection. |
| `working` / `composer_empty` / `unknown` | The corresponding tested activity/composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The exact persisted episode-class domain is `permission_request`, `question`, `trust`, `update`, `unknown_interaction`; `trust` and `update` are distinct values. `permission_denied` is a returned-result observation recorded only through `permission_denied_observed`, with a nullable episode link. `working`, `composer_empty` and `unknown` are input-eligibility states and create no interaction row.

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and remain explicit about wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. Unsupported versions advertise unknown coverage; missing matches never imply that all requests were observed.

An episode is a CAFleet observation identity, `(run_id, episode_seq)`, not a native request ID. Allocate `episode_seq` monotonically within a run when a stable live interaction first appears. Use two compatible observations at least 100 ms apart before opening; recheck after 100 ms even if no bytes arrive. Gate programmatic input immediately on the first candidate, before this notification debounce.

The fingerprint contains backend/profile, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration and line-wrapping differences. Redraws and resize matching the fingerprint update evidence without another opening notification. A strong operation/choice change supersedes the previous episode and opens a new one. After a verified native non-interaction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility. A two-second uncertain interval emits one `interaction_obscured` event. |
| Verified non-interaction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Close with `superseded` and open the next episode, in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Returned permission refusal appears | Emit `permission_denied_observed` once per stable result/boundary. Link an episode only when native evidence supplies the correlation; otherwise leave it null. |

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message ACK and resumed animation establish none of those outcomes by themselves. In this release, machine decision outcome remains `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

## Persistence, notification and read interfaces

The `cafleet_v8.db` format preserves its application identity and existing rows through a contiguous embedded migration. The singleton `runtime_database_identity(singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1), instance_id TEXT NOT NULL)` has a random 128-bit hex instance ID initialized once during migration. Runtime records have these fields:

| Table | Required fields and constraints |
|---|---|
| `member_runs` | `run_id INTEGER PRIMARY KEY AUTOINCREMENT`; member/fleet/Director foreign keys; backend, launch nonce, canonical runtime path, requested/resolved mode, mode source, permission argv JSON, native version, profile version, coverage, status and lifecycle/health timestamps; nullable runner/child PID plus verified start identity and exit code/signal. At most one nonterminal run per member. |
| `member_interactions` | `interaction_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, `episode_seq`, classification, post-policy evidence digest, opened timestamp, nullable closed timestamp/reason, and latest bounded evidence JSON. Unique `(run_id, episode_seq)` and at most one unclosed interaction per run. |
| `member_runtime_events` | `event_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, positive `event_seq`, nullable interaction foreign key, `kind`, observed/committed timestamps, versioned payload JSON, notification state and nullable unique `message_id` foreign key. Unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `input_id INTEGER PRIMARY KEY AUTOINCREMENT`, run foreign key, unique `(run_id, operation_key)`, kind, payload digest, nullable message foreign key, durable receipt/settlement state and timestamps. Retain settled rows as replay-rejection records for the run's lifetime. |

Persisted enums and decoded records enforce the values specified in this contract. `member_runs.status` is `starting`, `prepared`, `running`, `degraded`, `stopping`, `recovering`, `exited`, `failed` or `lost`; terminal states are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; retain these foreign-key-linked records under existing database ownership/retention behavior.

The table below fixes names/types beyond the primary-key fields above. Use `INTEGER` for identities, counters, PIDs and exit values; `TEXT` for enum/string/digest/JSON/timestamp values. Timestamps use UTC RFC 3339 and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Exact fields |
|---|---|
| Run identity/policy | Required `member_id`, `fleet_id`, `director_member_id`, `backend`, `database_identity_json`, `launch_nonce`, `runtime_path`, `resolved_mode`, `mode_source`, `permission_argv_json`, `event_evidence`, `coverage`, `status`, `created_at`; nullable `requested_mode`, `native_version`, `profile_version`, `effective_policy_json`. |
| Run lifecycle/ownership | Required `owner_epoch INTEGER` starts at 1, `owner_kind TEXT` is `creator`, `runner` or `reconciler`, `owner_pid INTEGER`, `owner_start_identity TEXT`, `reachability TEXT` is `reachable` or `suspect`, `cleanup_state TEXT` is `complete` or `incomplete`, `event_seq_reserved_through INTEGER` starts at zero, and `observation_gap_count INTEGER` starts at zero. Nullable `runtime_dir_identity_json`, `run_lock_identity_json`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`, `started_at`, `last_health_at`, `ended_at`, `exit_code`, `exit_signal`, `cleanup_diagnostics_json`, `recovery_origin_status`; the latter preserves the original lifecycle status while recovering. |
| Interaction | Required `run_id`, `episode_seq`, `classification`, `evidence_digest`, `opened_at`, `evidence_json`; nullable `closed_at`, `close_reason`. `classification` is an interactive class from the classification table; both closure fields are null together or present together. |
| Runtime event | Required `run_id`, `event_seq`, `kind`, `observed_at`, `committed_at`, `payload_json`, `notification_state`; nullable `interaction_id`, `message_id`, `archive_reason`. `notification_state=inbox` requires a message and null archive reason; `archived` requires no message and a closure reason. |
| Input operation | Required `run_id`, `operation_key TEXT`, `kind TEXT` (`preview`, `prompt`, `exec`), `payload_digest TEXT`, `state TEXT` (`ready`, `in_progress`, `accepted`, `uncertain`, `settled`), `created_at`, `updated_at`; nullable `message_id`, `disposition TEXT`, `evidence_ref TEXT`. `ready` certifies a zero-byte attempt; settled disposition is `submitted`, `not_submitted` or `abandoned_uncertain`, and preview kind requires its original message ID. |

Store process-start identities as platform-specific tokens in `TEXT`. JSON fields validate as objects except `permission_argv_json` and `cleanup_diagnostics_json`, which are arrays. Payloads carry `schema_version=1`; `event_evidence` is `metadata` or `redacted`. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` or `unknown`; tested means the pinned profile passed its declared release gates, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership. Use partial unique indexes for the active-run and open-interaction invariants.

Runtime event kinds are `coverage_unknown`, `interaction_opened`, `interaction_obscured`, `interaction_closed`, `permission_denied_observed`, `runner_degraded`, `runner_lost` and `runner_exited`. The event payload has this versioned shape:

```json
{
 "schema_version": 1,
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
 }
}
```

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null; screen parsing never fills it with invented execution data. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Launch-only `--event-evidence <metadata|redacted>` defaults to `metadata`. Metadata mode persists only identities, classification, rule/profile/version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints used for live episode comparison remain in memory and appear in neither logs, outbox nor database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`; there is no fallback to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member, with the typed `source` distinguishing runtime observation from model-authored speech. The same fleet and active-member checks apply to runtime inbox insertion. Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an `ObservedTransition` keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields; generated IDs and commit timestamps are not inputs to that comparison.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`; it retains the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

`cafleet event show <EVENT_ID> --json` returns the complete durable event, notification/archive state, nullable associated message ID and current interaction/run state. `cafleet input show <INPUT_ID> --json` returns a durable programmatic-input receipt. These IDs are positive database-assigned integers. Member detail/create JSON appends a `runtime` object containing run ID, resolved mode, status, recorded reachability, owner epoch, last health observation, coverage, current interaction ID and unresolved input IDs; use `runtime: null` for an unwrapped member. The CLI output contract specifies layouts and key order. Existing broker message envelopes and ACK semantics stay intact; the wrapped pane-preview text differs as specified below.

## CLI presentation contract

Member create/show JSON preserves existing fields and appends `runtime`. An unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id`, `resolved_mode`, `status`, `reachability`, `owner_epoch`, `last_health_at`, `coverage`, `current_interaction_id`, `unresolved_input_ids`, `owner_kind`, `owner_pid`, `owner_start_identity`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`. Process fields preserve their stored nullability. These registry-only fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket. Existing member text rows remain intact; a wrapped create/show appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE` on a separate line.

| Command | Ordered JSON keys and values |
|---|---|
| `event show` | `event`, `notification_state`, `message_id`, `archive_reason`, `run`, `interaction`; `event` is the complete versioned event payload, `run` is the current runtime summary above, and nullable `interaction` contains the stored interaction fields in schema order. |
| `input show` / `input reconcile` | `input`, containing the durable input-operation row in schema order. |
| `member recover` | `member_id`, `run_id`, `outcome`, `replayed_count`, `inbox_count`, `archived_count`, `status`, `cleanup_diagnostics`; outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`. A member with no recorded run fails explicitly before mutation. |
| `member prompt` / `member exec` | Preserve existing keys and append nullable `input_id`; a wrapped durable submission returns its input ID, and an unwrapped submission returns null. |

New read/recovery commands emit JSON when `--json` is supplied and compact human-readable text otherwise; the [CLI reference](cli-options.md#runtime-presentation) owns exact layouts and field order. Wrapped prompt/exec text appends `Input: INPUT_ID` on a separate line when a durable ID exists. Uncertain submission errors include the durable input ID when known, so a caller can inspect the receipt even though the command reports failure. Existing non-dispatch/uncertainty errors retain their exit category and add run/interaction/input context in the cause. Recovery outcomes distinguish incomplete or unreachable work from confirmed recovery; printed submission and settlement establish no native execution result.


## Storage pressure and durability boundary

An event worker owns SQLite and a private append-only per-run outbox. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker fsyncs length-delimited, checksummed, redacted records before attempting SQLite; remove an outbox record only after its event/message transaction commits. On restart/reconciliation, replay complete valid records with their original run/sequence, discard an incomplete final frame with an explicit diagnostic, and fail on corruption in a completed frame.

The current owner reserves event-sequence ranges of 256 by advancing `event_seq_reserved_through` in a bounded ownership-checked transaction before using them. The classifier consumes the reserved range in memory; if the next range is unavailable it enters the defined observation-gap state while terminal I/O continues. Unused reserved values are valid gaps. Recovery allocates new sequences above this durable high-water mark as well as all committed/complete-spool sequences, so missing artifacts cannot cause reuse of a previously issued sequence.

The Director-owned explicit `cafleet member recover <MEMBER_ID> --json` command operates on the recorded run of an active or deregistered member, never starts an agent, and returns recovery counts, final lifecycle/notification state and cleanup diagnostics. A five-second invocation deadline bounds each recovery pass; incomplete replay retains the spool and `recovering` state for the next pass. `doctor` diagnoses recovery needs without mutating them. Member/fleet deletion invokes the same recovery component after successful runner shutdown and before deregistration when its remaining deadline permits; otherwise retain the namespace for a later explicit pass.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the existing pane lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below; it is not a free-lock result.
2. After acquiring the matching the ownership lock, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, read the entire outbox in frame order, validate checksums/sequence identities and reconcile previously committed frames idempotently. Replay every complete frame by ascending `event_seq`, updating interactions before their later closures. Retain an incomplete tail diagnostic; corruption blocks the pass with its original bytes preserved. Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. After all complete original frames are reconciled, reserve new sequences above the durable reserved high-water mark, committed events and every valid complete outbox frame. Derive the terminal outcome from the original status and any replayed terminal lifecycle record. If neither establishes an exit/failure/loss, append a pending interaction's `monitor_lost` closure and then `runner_lost` through the same fsynced outbox/idempotent transaction path. Otherwise finish the recorded terminal outcome without inventing a loss; a replayed child exit supersedes the stale pre-crash running status. Repeated passes detect existing closure/loss records instead of generating them again.
5. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then compact committed frames and release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention and delete the outbox only when all complete frames are durably represented.

Only the lock owner reads/compacts/writes the outbox, including normal event-worker operation. An external pass cannot race the live writer.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity returns `live_or_unreachable`; unknown process identity returns `operator_reconciliation_required`. Preserve suspect state and the open interaction in both cases. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

For unreadable owner identity, expose the exceptional Director-owned `cafleet member recover <MEMBER_ID> --owner-terminated --expected-run-id <RUN_ID> --expected-owner-epoch <EPOCH> --evidence-ref <REFERENCE> --json` path. Require all four options together, with positive IDs/epoch and a nonempty reference to the operator's recorded attestation that the inspected owner and startup processes have terminated; incomplete options fail with usage exit `2`. Without that attestation, unreadable identity continues to return `operator_reconciliation_required`.

The Director obtains the run ID, owner epoch and PID/start identity from the inspected registry record and binds the operator evidence to that snapshot. Revalidate database/run identity, epoch and owner tuple after obtaining the SQLite write lock; a changed run or owner rejects the stale attestation rather than rebinding it. A matching live-process identity, held identity-matching lock or matching live health reply rejects the termination claim even with the flag. For an unchanged owner with unreadable identity and no positive live evidence, record `owner_termination_attestation` in `cleanup_diagnostics_json` with the original run/epoch, owner PID/start token, evidence reference, UTC timestamp and `source=operator_via_director`, then execute the database-only branch in the same CAS transaction. Preserve that provenance separately from machine-verified termination. This attestation enables bookkeeping closure of the identified owner only; it authorizes no child creation, input, approval answer or replay.

Once the recorded owner's termination is machine-verified or accepted through that explicitly attributed attestation, perform database-only reconciliation in one bounded immediate transaction, rereading the owner identity/epoch after obtaining the SQLite write lock. A concurrent live reconciler causes deferral; an already terminal result is returned idempotently unless the supplied expected run/epoch has changed, which remains a stale-attestation error. Advance the epoch and record the recovering process identity, reserve sequences above the durable high-water mark, increment the gap count once, add `runtime_path_missing` to cleanup diagnostics, and close any persisted interaction with `monitor_lost` before `runner_lost`. Use inbox or archived event insertion according to current lifecycle state. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new records atomically. A failed transaction leaves the original state for retry.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, while continuing manual terminal I/O; resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic, and preserve the actual error. Neither endless memory growth nor a claim of successful notification is a recovery strategy.

## Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket rather than multiplexer pane input. Retain the existing outer `PaneLock` and placement revalidation for ordering with deletion; the runner serializes the final check and child writes with its own terminal-state loop. The lock order is pane lock, short database validation, then bounded socket request; the runner never waits for the external pane lock while serving that request.

Control frames are length-prefixed JSON, capped at 64 KiB, with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. The runner supports `health`, handshake `start`, `receipt`, `submit_prompt`, `submit_exec` and `shutdown`. `receipt` has `query` and `retire` actions. Replies distinguish `accepted`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence and byte/submission stage; the ID is null for a zero-byte attempt with no durable row. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state, permits reevaluating input eligibility. A held/rejected zero-byte attempt creates no side-effect receipt; if a row was already reserved, move it to `ready` only after positively recording that zero bytes were written.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may reconcile its retained stage evidence into the same row later; a crashed runner's in-progress row is never an automatic replay instruction.

Retire only the in-memory receipt after durable settlement. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout is merely a presentation of its input ID and supplies no durable acknowledgement. Retain settled operation keys/digests/dispositions for the run's database lifetime, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts to 4096, rejecting additional new side-effect attempts while that unresolved set is full; settled receipts leave the bounded set. Keep uncertain receipts until the Director records an evidence-based disposition through `cafleet input reconcile <INPUT_ID> --disposition <submitted|not-submitted|abandoned-uncertain> --evidence-ref <REFERENCE> --json`. This Director-owned operation only records disposition, settles the key and permits cache retirement; it sends no bytes, grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation acquires the pane/input lock and synchronizes with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently. If that owner is unreachable, first establish its termination/exclusive recovery under the same ownership rules; retain uncertainty while an active writer cannot be ruled out. Compare-and-swap the expected operation state in the settlement transaction, and reconcile a changed row before another attempt. Settlement records the observed input disposition, not the native command's execution outcome.

Held-preview retry remains owned by external `message send` / finite `message wait`, using the existing pending-preview rotation. The runner never fetches/stamps the next preview merely because an interaction closes. After a human answers and the Director verifies the resulting native state, consume/ACK available inbox records and explicitly invoke finite `message wait <DIRECTOR_ID>` to retry pending previews. If wait returns new inbox records first, consume/ACK them and re-enter that receive step; if the composer remains busy, previews stay held for a later receive step. This preserves the scope of the runner as observer and input arbiter.

A process loss, ownership change or unavailable receipt permits no automatic replay of an attempted submission. Keep notification unstamped, retain its durable uncertainty and reconcile actual output before a separate deliberate action. Explicit prompt/exec use generated keys and printed durable input IDs with the same rule.

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

Retain the existing body folding, inline limit and long-body `message show` pointer. Wrapped previews end at the resume clause because their submission sends no Escape. Unwrapped previews retain their existing full line and Escape-dismissal explanation. Both preview forms are specified by the message-envelope and multiplexer contracts. Held messages keep `notified_at` unset, and only confirmed submission or the existing ACK settlement stamps them. Holding supplies no new permission event and never closes the native request. Native-exec success means submission, not execution success.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees suppression for observed/candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race, rather than asserting universal prompt preservation. If a profile cannot safely distinguish an empty composer and its concurrent-work boundary, that profile supports manual input only. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. CAFleet does not interpret an input byte as authorization or a verified outcome. A human uses the original displayed request and native choices; a Director asking for that human action references member/run/interaction and visible operation. Human answers use the original native surface; event and input commands record observation or disposition only. Retain the legacy mux path only for explicitly unwrapped members and preserve its narrower existing semantics.

## Director consumption and continuation

Use the existing `message listen`, `poll --json` and finite `wait` interfaces. Runtime events reach the Director's stdout receiver and durable inbox; they introduce no composer injection, Escape-first wake, goal activation or new host interrupt guarantee. Detection and receiver latency have separate acceptance targets. Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

On a wake or poll, the Director reads the complete event, verifies its durable event/message link with `event show`, reconciles the current run/interaction, records the unfinished action in host-owned state, and ACKs the message as consumption. An ordinary member can quote event JSON; a claimed event identity is verified against stored ownership, never trusted from message text alone. Old/closed events remain readable but are not a new request to answer.

| Stage | Observable evidence | What remains outstanding |
|---|---|---|
| Detection | Rule/profile and current episode evidence | Durability and Director reception. |
| Persistence | Event ID plus committed inbox message ID | Director consumption. |
| Consumption | ACK plus Director-owned pending-action record | Human answer and actual operation outcome. |
| Human native answer | Attributed human/native evidence tied to the displayed request | Actual result and dependent continuation. |
| Episode disappears | Closure reason and current screen | Decision and execution outcome may remain unknown. |
| Continuation verified | Actual operation result/output and member's resumed dependent work | Any remaining task-specific work. |

For a permission refusal returned to the member, preserve the existing evidence route: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, obtain the human decision on the original surface, then inspect native output/exit/error and the member's actual continuation. Consume/ACK any new inbox records and run an explicit finite `message wait <DIRECTOR_ID>` after that inspection to retry held previews; a new received batch repeats that consumption/receive step. The listener alone performs no preview retries, and a still-busy composer retains its pending messages. A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.


## Native acceptance and coverage

Use synthetic terminal replay to validate parser/classifier/episode behavior and real PTYs to validate transport, lifecycle and input arbitration. Cover each registered multiplexer. Fixed-policy backends without an interactive profile must show their explicit unknown-coverage/manual-input behavior rather than borrowing another backend's acceptance.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Pin exact installed CLI versions by the acceptance records and corresponding bundled profiles; use permitted disposable fixtures that actually reach the native approval surface. Every minimum case must demonstrate blocked-model-independent detection, a committed event/inbox record, Director consumption, preserved original request, a human native answer and observed continuation. Missing native availability, unknown coverage or an unobserved case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Record source/installed CAFleet version, native CLI version, native configuration/mode, detector profile, multiplexer, terminal size, cwd and redacted evidence for live acceptance. Include a permitted operation, an ordinary command failure, an actual permission refusal, an approval that blocks the model, a human native answer and observed continuation. Exercise redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.


Acceptance additionally covers the transport, lifecycle and persistence cases below. With a live receiver and available storage, measure a target of two seconds from a stable prompt to inbox commit and another two seconds from commit to receiver output.

- Test exact child argv for the selected native policy, preserving native/managed restrictions and existing sandbox boundaries.
- Test equal run IDs across sibling databases and replacement at an unchanged path.
- Test a held write transaction and a long statement to prove restoration/exit are independent of the connection's default timeout.
- Test a child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.
- Test timeout followed by the original runner's recovery and concurrent recovery attempts.
- Test a command containing a known secret and assert its absence from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.
- Test hard kill with complete uncommitted frames, replay of an opening before loss closure, concurrent recovery, interruption during recovery, and deregistration before replay with archived rather than fabricated inbox records.
- Test a live unlinked lock, verified death, unknown identity without attestation, matching/stale attestations, positive live evidence rejecting an attestation and concurrent database-only recovery.
- Test cross-invocation accepted-then-failed-stamp recovery, held-then-eligible delivery triggered by an external wait, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.
- Test every registered backend's omitted-policy argv and each adapter-declared explicit policy; profiles claiming detection or composer support require both fixtures and real native evidence.
