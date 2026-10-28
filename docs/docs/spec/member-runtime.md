# Member runtime contract

This contract describes runner-managed member launch, terminal observations, durable events, recovery and programmatic input. [Member permission events](../concepts/member-permission-events.md) explains the operator workflow.

## Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | Omitted selection | Explicit selection mapping |
|---|---|---|
| Claude | `dontAsk` | Use the native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `never` with `workspace-write` | Map native approval-policy values through `--ask-for-approval`; retain the explicit workspace sandbox. |
| OpenCode | Existing `cafleet` preset | Accept exactly `cafleet`, preserving the fixed preset; explicit `cafleet` records `mode_source=explicit`. |

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping, native-version eligibility and detector/composer/answer profiles. Help and usage errors report choices from that catalog. Claude accepts native `--permission-mode` values verified from installed help and parser evidence, including documented aliases, recorded by its version-qualified adapter. Claude Code 2.1.302 accepts canonical `default` and its `manual` alias; help displays `manual`, while the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. Codex accepts the native `--ask-for-approval` values read from the installed `codex --help` and recorded the same way. OpenCode accepts exactly the token `cafleet`; every other explicit token fails with usage exit `2`.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent. `effective_policy_json` remains null unless an actual native observation supplies it; an installed version, argv or selected profile alone proves no effective authorization decision.

Keep launch, detection, empty-composer observation, prompt submission, native-exec submission and request-bound answer capabilities separate. Answer profiles declare support for each semantic decision independently. Profiles declare exact backend versions, rule-set version and capability-specific fixture provenance; bundled tested declarations require matching native captures and measured single-request answer stages for the capability claimed. Synthetic fixtures exercise rule and capability validation while unverified components remain unknown or unsupported. A missing version/profile yields `coverage=unknown`, an initial coverage notification and held programmatic input; it still permits manual terminal operation and records the selected mode. Automatic/fixed policies remain useful even where screen detection is unknown. The default OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

### Native executable resolution {#native-executable-resolution}

`member create` resolves the backend's binary to one absolute executable path during the backend availability check, using the creator process's `PATH`. It searches the absolute `PATH` entries in order, skipping empty and relative entries, and selects the first `<entry>/<binary>` that is a regular file with an execute permission bit after following symbolic links. The selected path is used as found, without symbolic-link resolution, so a link or wrapper keeps its own name. When no entry matches, creation fails before registration with exit 1 and `Error: binary <name> not found on PATH`.

The version probe below runs that exact path, and the same path is `argv[0]` of the runner's literal child argv. The probed binary and the launched binary are therefore the same path on every multiplexer; the pane's own `PATH` and shell configuration play no part in selecting the native executable.

### Native version probe {#native-version-probe}

`member create` records the installed native version of the resolved backend. After launch-policy validation and the backend availability check, and before registration, it runs the [resolved executable](#native-executable-resolution) with exactly the argument `--version`, so `claude`, `codex` and `opencode` each report their own version. The probe has null stdin and captures at most 4096 bytes each of stdout and stderr. It ends within two seconds, and those two seconds belong to the command's existing ten-second creation budget, which starts before the probe. A probe still running at its bound is killed.

The probe yields a version only when it exits with status `0` and the first stdout line has exactly one whitespace-separated token matching `^[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$`. That token is recorded verbatim as the run's `native_version`; for example `2.1.302 (Claude Code)` yields `2.1.302` and `codex-cli 0.163.0` yields `0.163.0`. The bundled catalog entry for that backend and exact version then supplies `profile_version`, and `coverage` is `tested` when that profile declares detection. A version without a bundled profile records `profile_version` null and `coverage=unknown`.

A probe that cannot start, times out, exits nonzero, prints non-UTF-8 output or has zero or several version tokens records `native_version` null, `profile_version` null and `coverage=unknown`. Creation continues. The command writes one stderr line, `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.`, and leaves its stdout text and JSON unchanged. The probed version identifies the installed binary only; it establishes no effective native policy.

### Bundled profiles and native screen fixtures

The bundled catalog is embedded in the CAFleet binary together with the native screen fixtures that its declarations cite. Each profile's capabilities rest on accepted native captures of its exact native version.

Claude Code `2.1.302`, profile `claude-2.1.302-default-2`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native `default` Bash permission form (`Bash command` heading, approval text, proceed question, focused choice and the `Yes`/`No` single-request choices) in the alternate screen |
| Allow answer | `tested` | One stage, byte `31` (digit `1`), selecting `Yes` as `allow-once` |
| Deny answer | `tested` | One stage, byte `34` (digit `4`), selecting `No` as `deny-once` |
| Empty-composer observation | `tested` | The idle prompt line with the terminal cursor immediately after the prompt prefix; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

Both answers select their choice by its number, so they do not depend on which choice is focused. The broadened choices 2 and 3 have no answer plan. The empty-composer observation reads the composer as empty only when the cursor sits at the end of the prompt prefix; a draft whose cursor was moved back to the prefix reads as empty. Every backend and native version without a bundled profile yields `coverage=unknown`.

Detection covers the permission form only when it renders within 16 rows from its `Bash command` heading to its `Esc to cancel · Tab to amend` line, with the operation on a single line between the dashed rules. A narrow pane or a long command makes Claude wrap the form: the operation becomes boxed `│` rows and the choice labels continue on further rows. A wrapped form yields no detection, so no `interaction_opened` event is produced, programmatic input stays held, and the request is answered manually on the native surface.

Codex CLI `0.163.0` launched with `--ask-for-approval on-request --sandbox workspace-write`, profile `codex-0.163.0-on-request-1`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native command-approval form in the alternate screen, from its `Would you like to run the following command?` heading to its `Press enter to confirm or esc to cancel` line within 24 rows: the `Environment:` and `Reason:` lines, the single-line `$ ` command, the focused `›` choice and the `1. Yes, proceed (y)` / `3. No, and tell Codex what to do differently (esc)` single-request choices, with the cursor inside the form |
| Allow answer | `tested` | One stage, byte `79` (`y`), selecting `Yes, proceed` as `allow-once` |
| Deny answer | `tested` | One stage, byte `33` (digit `3`), selecting `No, and tell Codex what to do differently` as `deny-once` |
| Empty-composer observation | `tested` | The idle `› ` composer line with the terminal cursor immediately after the prefix, its placeholder text shown; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

The rules anchor on the live form's heading and closing lines rather than on absolute rows, and the reason line is model-authored text that the rule requires but excludes from the request identity. The deny answer uses the choice's digit because that choice displays only `esc` as its shortcut, and CAFleet never sends a lone Escape to a held form. Choice 2, which approves a command prefix persistently, has no answer plan. The native captures showed the decision on the native surface: after allow, Codex printed `✔ You approved codex to run <command> this time` and ran the command; after deny, it printed `✗ You canceled the request to run <command>` and `■ Conversation interrupted`, and ran nothing. These lines are decision evidence for the Director's continuation check, not bundled rules. A Codex configuration that routes approvals to its automatic reviewer (`approvals_reviewer`) answers escalations without any native form, so detection observes no request. The detection covers a single-line command; a command that wraps in a narrow pane yields no detection, as for the Claude form.

A native screen fixture records one captured terminal screen with exactly these fields:

| Field | Requirement |
|---|---|
| `schema_version` | `1` |
| `fixture_id` | Equals the catalog fixture `id` |
| `provenance` | `native` |
| `backend`, `native_version` | Equal the catalog fixture entry |
| `rows`, `cols` | Positive measured geometry, equal to the catalog fixture entry |
| `alternate_screen`, `screen_seq`, `cursor_row`, `cursor_col` | Measured screen state; `cursor_row` is below `rows`, and `cursor_col` is at most `cols`, where `cursor_col` equal to `cols` records the terminal's pending-wrap position after the last column |
| `lines` | Exactly `rows` captured row texts, in viewport order |
| `masked_rows` | Unique zero-based row indices, each `{"row":N,"reason":"reviewed_privacy"}`; the corresponding `lines` entry is the empty string |

Masking applies only to rows removed by the reviewed capture privacy policy, such as home paths and account, model or plan text. Every other row keeps its captured text exactly, including trailing spaces. The detection region of every rule citing the fixture lies entirely in unmasked rows, and a masked fixture keeps `provenance` `native`.

The bundled catalog fails to load unless each bundled fixture satisfies all of these:

- Its embedded bytes hash to the catalog entry's `sha256`.
- It parses with exactly the fields above.
- It declares `native` provenance.
- It matches the entry's backend, version and geometry.
- Every row declared in `masked_rows` is in range and empty.

A synthetic fixture, an undeclared alteration or a digest mismatch therefore rejects the bundled catalog. Synthetic fixtures remain available to explicitly selected test catalogs.

A rule's cursor condition treats a pending-wrap cursor as standing on the last cell of its row, both in fixtures and on live screens. A region spanning the full width therefore counts a cursor at column `cols` on one of its rows as inside the region.

### Executable native plans

Keep detection, empty-composer observation, prompt submission, native-exec submission and each semantic answer decision independently eligible for the exact backend/native version. Observing an empty composer supplies input-state evidence; executable submission additionally requires its measured literal/submit plan. Existing observation definitions without a submission plan retain their observation coverage and yield no executable composer capability. Production capabilities require accepted exact-version native evidence for the claimed operation; synthetic plans run only in explicitly selected test services.

NativeCapabilities reports empty-composer observation through `composer`, executable prompt submission through `prompt_submission`, and executable native-shell submission through `native_exec_submission`. Each operation field is `tested` exactly when the selected backend/exact native version has a validated executable definition for that operation; otherwise it is `unknown`. A catalog selected explicitly by test services describes its fixture coverage; production declarations additionally require accepted native evidence. Observation coverage and support for another operation leave each operation field independent.

An answer definition retains its validated single-request `choice_id`, `focus_matcher_id` and ordered `stages`, in addition to its decision/request/rule/provenance fields. Each stage has exactly `bytes_hex,required_rule_id,required_choice_id`; the required rule and choice equal the answer's request and selected single-request choice. The top-level focus matcher applies before every supported stage. A plan requiring a different intermediate focus requires its own reviewed profile representation and native evidence.

The existing composer object has required `rule_ids,fixture_ids` and optional `submission`. When present, `submission` has exactly `prompt,exec`, each null or an object with exactly `prefix_hex,submit_hex,fixture_ids`. Prompt has an empty prefix; exec has prefix `2120`, the UTF-8 bytes of `! `, with independently tested native-shell coverage. Every plan's fixture references resolve to the same backend and every declared exact native version, and establish the complete composer/work boundary for that operation. Unknown fields and incomplete or incompatible provenance fail validation.

| Plan property | Inclusive limit or required representation |
|---|---|
| Answer stages | 1–16 ordered stages. |
| Answer bytes | 1–256 decoded bytes per stage; at most 4096 decoded bytes per plan. |
| Composer submit key | 1–32 decoded bytes. |
| Composer prefix | Zero bytes for prompt; exactly two bytes for exec as specified above. |
| Encoded native ordinary submission | At most 65536 bytes for prefix, unchanged literal UTF-8 and submit key together; the complete control JSON must independently satisfy its frame limit. |
| Hex byte definitions | Even-length lowercase hexadecimal, with nonempty bytes wherever the domain above requires them. |

Validate the complete shape, reference set, hex form and byte/stage bounds before decoding and constructing an executable plan. Keep selected native bytes and live semantic identity in memory, outside ordinary errors, Debug output and persistence. An ordinary submission attempts its complete encoded payload as one stage; a positive short write stops remaining bytes and records partial uncertainty. Before any positive ordinary write, WouldBlock or Interrupted yields and requires fresh input/output/state validation before another attempt. Answer stages follow their independently measured order and revalidate request, choice and focus before each stage; a positive short stage stops the remaining plan with uncertainty.

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

During verified ordinary recovery, retain the original ownership lock and pin a leftover launch descriptor by its opened file identity and bounded content. Validate the recorded namespace/directory, private regular file, immutable database/fleet/member/Director/run identity and nonce. A positive historical descriptor epoch at or below the captured current epoch remains valid across takeover and retry; native launch loading still requires current creator ownership and the exact current epoch. Expected absence is idempotent while the pathname remains absent.

Before unlink, revalidate the fixed current run/placement, original lock/directory, named file type/permissions/device/inode/content and original deadline. Remove the pinned descriptor before reporting complete cleanup. Preserve a replacement or symlink and return an explicit identity error; report unlink failure explicitly and retain incomplete, retryable recovery. Keep assignment and environment content out of diagnostics. Exceptional database-only reconciliation leaves surviving unverified descriptors untouched.

Prepare connection setup waits only before any request write for an absent endpoint, a same-user socket awaiting mode 0600, or a transient bound-but-not-listening socket. All retries share the original handshake deadline. Wrong type, owner or symlink, including a dangling symlink, fails explicitly; current-run health keeps strict endpoint checks. A possible prepare write, malformed reply or lost reply returns its explicit error or uncertainty without reconnecting.

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

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, rather than writing a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` remains a registry read, returns the recorded fields and requires neither a socket nor a multiplexer. `doctor` reports registry metadata, pending content reconciliation and the results of nonblocking identity-pinned lock and bounded socket probes. Outbox content inspection belongs to the current lock/epoch owner; diagnosis leaves registry records, events and reserved sequences unchanged. Input dispatch requires a fresh matching health/ownership check; its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

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

The fingerprint contains backend/profile, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration and line-wrapping differences. Redraws and resize matching the fingerprint refresh the stored latest interaction evidence without another opening notification, but only when the post-policy evidence differs in a field other than `screen_seq` and the `evidence_digest` derived from it; a redraw that changes only the output sequence schedules no refresh and leaves the stored evidence and any issued request token unchanged. A different operation/choice/class candidate holds input immediately and needs two compatible observations at least 100 ms apart before replacing the active episode; retain the old episode until then, close it with `superseded`, and open the new episode in order. An incompatible candidate restarts stability, while return to the original fingerprint cancels the pending replacement. After a verified native non-interaction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility. A two-second uncertain interval emits one `interaction_obscured` event. |
| Verified non-interaction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Require two compatible observations at least 100 ms apart, then close with `superseded` and open the next episode in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Returned permission refusal appears | Require two compatible observations at least 100 ms apart and emit `permission_denied_observed` once per stable result. Link an episode only through native correlation evidence. |

A returned refusal resets its deduplication identity only after a profile-verified noninteraction boundary lasting 250 ms. Temporary uncertainty resets pending stability while retaining the previous deduplication identity; a generic working screen establishes a boundary only when its profile verifies the result region's disappearance.

Equivalent redraw evidence uses one bounded coalesced worker slot, with a bounded local snapshot during an in-flight attempt. Apply the evidence policy before queuing, bind the refresh to the current owner/run/open episode and output sequence, and coalesce only newer evidence for that same active episode. Persist it in an ownership-checked transaction under an absolute deadline; equal sequence with identical evidence is idempotent, while regressing, conflicting, closed-episode or stale-episode updates fail explicitly.

Order each refresh after its opening and previously accepted semantic transitions, and before later transitions. A transient event-sequence barrier prevents delayed evidence from overwriting a later closure or visibility transition; preserve a newer coalesced revision when an earlier attempt completes. Refresh only the interaction's latest evidence/digest, preserving original opening event/message bytes and allocating no event, episode, sequence or inbox notification. Failed refreshes retain bounded pending evidence and expose degraded/gap state while manual terminal I/O continues; recovery preserves the last committed evidence and reports the gap rather than reconstructing lost redraws.

Only a fingerprint-matching redraw refreshes an interaction's stored latest evidence. A closing transition keeps that stored evidence of the request; the closing frame's post-policy evidence appears only in the `interaction_closed` event payload.

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message ACK and resumed animation establish none of those outcomes by themselves. In this release, machine decision outcome remains `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

## Persistence, notification and read interfaces

The `cafleet_v8.db` format preserves its application identity and existing rows through contiguous embedded migrations. V4 establishes runtime records and V5 adds the permission-response audit/claim relationships specified below. The singleton `runtime_database_identity(singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1), instance_id TEXT NOT NULL)` has a random 128-bit hex instance ID initialized once during migration. Runtime records have these fields:

| Table | Required fields and constraints |
|---|---|
| `member_runs` | `run_id INTEGER PRIMARY KEY AUTOINCREMENT`; member/fleet/Director foreign keys; backend, launch nonce, canonical runtime path, requested/resolved mode, mode source, permission argv JSON, native version, profile version, coverage, status and lifecycle/health timestamps; nullable runner/child PID plus verified start identity and exit code/signal. At most one nonterminal run per member. |
| `member_interactions` | `interaction_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, `episode_seq`, classification, post-policy evidence digest, opened timestamp, nullable closed timestamp/reason, and latest bounded evidence JSON. Unique `(run_id, episode_seq)` and at most one unclosed interaction per run. |
| `member_runtime_events` | `event_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, positive `event_seq`, nullable interaction foreign key, `kind`, observed/committed timestamps, versioned payload JSON, notification state and nullable unique `message_id` foreign key. Unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `input_id INTEGER PRIMARY KEY AUTOINCREMENT`, run foreign key, unique `(run_id, operation_key)`, kind, payload digest, nullable message foreign key, durable receipt/settlement state and timestamps. Retain settled rows as replay-rejection records for the run's lifetime. |
| `member_permission_responses` | Immutable required decision/provenance fields keyed by the input ID, with event/interaction/Director foreign keys. |
| `member_permission_response_claims` | Interaction primary key and unique input reference, retaining one possibly transmitted answer per episode. |

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

Version-2 event keys are ordered `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for lifecycle events, then required `director_action`. The action object has ordered `kind,instruction,inspect_argv`; other new events use null action. Version-1 reads preserve their original fields and order.

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null; screen parsing never fills it with invented execution data. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Launch-only `--event-evidence <metadata|redacted>` defaults to `metadata`. Metadata mode persists only identities, classification, rule/profile/version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints used for live episode comparison remain in memory and appear in neither logs, outbox nor database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`; there is no fallback to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery. Test a command containing a known secret and assert its absence from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member, with the typed `source` distinguishing runtime observation from model-authored speech. The transaction shares the broker’s existing fleet/active-member checks. Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an `ObservedTransition` keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields; generated IDs and commit timestamps are not inputs to that comparison.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`; it retains the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

Add `cafleet event show <EVENT_ID> --json` for the complete durable event, notification/archive state, nullable associated message ID and current interaction/run state. Add `cafleet input show <INPUT_ID> --json` for a durable programmatic-input receipt. These IDs are positive database-assigned integers. Add a `runtime` object to member detail/create JSON containing run ID, resolved mode, status, recorded reachability, owner epoch, last health observation, coverage, current interaction ID and unresolved input IDs; use `runtime: null` for an unwrapped member. The CLI reference fixes exact layouts and key order. Existing broker message envelopes and ACK semantics stay intact; the wrapped pane-preview text differs as specified below.

## CLI presentation contract

Member create/show JSON preserves existing fields and appends `runtime`. An unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id`, `resolved_mode`, `status`, `reachability`, `owner_epoch`, `last_health_at`, `coverage`, `current_interaction_id`, `unresolved_input_ids`, `owner_kind`, `owner_pid`, `owner_start_identity`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`. Process fields preserve their stored nullability. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, in input-ID order. An accepted input is a durable submitted receipt that needs no reconciliation and stays listed until a settlement records its disposition; `in_progress` and `uncertain` inputs need Director reconciliation before a separate deliberate action. These registry-only fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket. Existing member text rows remain intact; a wrapped create/show appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE` on a separate line.

| Command | Ordered JSON keys and values |
|---|---|
| `event show` | `event`, `notification_state`, `message_id`, `archive_reason`, `run`, `interaction`; `event` is the complete versioned event payload, `run` is the current runtime summary above, and nullable `interaction` contains the stored interaction fields in the interaction wire order below. |
| `input show` / `input reconcile` | `input`, containing the durable input-operation row in the input-operation wire order below. |
| `member recover` | `member_id`, `run_id`, `outcome`, `replayed_count`, `inbox_count`, `archived_count`, `status`, `cleanup_diagnostics`; outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`. A member with no recorded run fails explicitly before mutation. |
| `member prompt` / `member exec` | Preserve existing keys and append nullable `input_id`; a wrapped durable submission returns its input ID, and an unwrapped submission returns null. |

New read/recovery commands emit JSON when `--json` is supplied and compact human-readable text otherwise, with exact text layouts in the shipping CLI reference. Wrapped prompt/exec text appends `Input: INPUT_ID` on a separate line when a durable ID exists. Uncertain submission errors include the durable input ID when known, so a caller can inspect the receipt even though the command reports failure. Existing non-dispatch/uncertainty errors retain their exit category and add run/interaction/input context in the cause. Recovery outcomes distinguish incomplete or unreachable work from confirmed recovery; printed submission and settlement establish no native execution result.


Compact text layouts preserve the existing member rows and use these additional lines (`-` represents an unavailable optional value):

| Command | Text layout |
|---|---|
| `event show` | `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID`, the runtime summary line, then `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. |
| `input show` / `input reconcile` | `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`. |
| `member recover` | `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC`. |

Input-operation wire fields are ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response`; the final field is the nullable permission-response audit object specified below. Interaction wire fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`. Uncertain submission errors include `input_id=INPUT_ID` when the durable identity exists.

## Storage pressure and durability boundary

An event worker owns SQLite and a private append-only per-run outbox. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker fsyncs length-delimited, checksummed, redacted records before attempting SQLite. Retain committed frames while any complete frame remains uncommitted; truncate and fsync the file only after independently validating every complete frame's committed event/message association. Under exclusive ownership, diagnose and remove an incomplete final tail at the last valid boundary before further append; preserve complete corrupt bytes and fail explicitly. Replay valid complete records with their original run/sequence, and compact once the remaining frames commit so pressure can clear.

The current owner reserves event-sequence ranges of 256 by advancing `event_seq_reserved_through` in a bounded ownership-checked transaction before using them. The classifier consumes the reserved range in memory; if the next range is unavailable it enters the defined observation-gap state while terminal I/O continues. Unused reserved values are valid gaps. Recovery allocates new sequences above this durable high-water mark as well as all committed/complete-spool sequences, so missing artifacts cannot cause reuse of a previously issued sequence.

The Director-owned explicit `cafleet member recover <MEMBER_ID> --json` command operates on the recorded run of an active or deregistered member, never starts an agent, and returns recovery counts, final lifecycle/notification state and cleanup diagnostics. A five-second invocation deadline bounds each recovery pass. Start one cleanup clock at entry to the common recovery coordinator, before snapshot, locking, process/health evidence and replay; its existing two/three/four-second boundaries share that origin and are clipped to the supplied original deadline, including a deletion caller's earlier deadline. Incomplete replay retains the spool and `recovering` state for the next pass. `doctor` diagnoses recovery needs without mutating them. Member/fleet deletion invokes the same recovery component after successful runner shutdown and before deregistration when its remaining deadline permits; otherwise retain the namespace for a later explicit pass.

Every recovery counter measures durable work newly committed by this invocation.

| Counter | Commit domain |
|---|---|
| `replayed_count` | Distinct original `(run_id,event_seq)` transitions from this invocation's input spool whose event transaction first commits in this invocation. Already committed originals and identical duplicate frames contribute zero additional count. |
| `inbox_count` | Newly committed runtime event/message associations, including replayed originals and newly synthesized recovery transitions. Each association counts once after commit. |
| `archived_count` | Newly committed runtime events with archived disposition, including replayed originals and newly synthesized recovery transitions. Each event counts once after commit. |

A duplicate lookup, skipped frame, reserved sequence or failed/rolled-back transaction contributes zero. An incomplete report retains its already committed work; a later invocation counts only its additional commits. A transition synthesized during this invocation contributes to the inbox/archive domain rather than its replay count; if an interrupted invocation fsynced it without committing, its successful replay from the next invocation's input spool contributes to that next replay count. The count domains are independent: two unique uncommitted inputs, an identical duplicate, an already committed prefix and two newly synthesized inbox transitions yield `replayed_count=2,inbox_count=4,archived_count=0`. Database-only missing-path recovery has zero replay count while its newly committed closure/loss events contribute to inbox/archive counts.

Before returning an existing completed terminal result, validate supplied expected run/epoch against the currently inspected registry. A matching no-work return preserves existing status, diagnostics and attestation provenance, reports zero counts and advances no ownership; it observes terminal history even if the recorded current reconciler remains alive. A successful exceptional takeover advances the epoch, so repeating its original pre-takeover expected epoch is stale; any later owner change also invalidates an earlier expected epoch.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the existing pane lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below; it is not a free-lock result.
2. After acquiring the matching ownership lock, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, validate the entire complete frame sequence before replay: checksums and sequence identities match, and `event_seq` is nondecreasing in physical order. Reconcile identical same-sequence originals idempotently; conflicting same-sequence originals or any physical sequence regression are complete corruption, which blocks the pass and preserves original spool bytes. Replay valid originals in their existing ascending order, updating interactions before later closures. An incomplete final suffix retains its separate tail diagnostic and exclusive-owner handling. Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. After all complete original frames are reconciled, reserve new sequences above the durable reserved high-water mark, committed events and every valid complete outbox frame. Derive the terminal outcome from the original status and any replayed terminal lifecycle record. If neither establishes an exit/failure/loss, append a pending interaction's `monitor_lost` closure and then `runner_lost` through the same fsynced outbox/idempotent transaction path. Otherwise finish the recorded terminal outcome without inventing a loss; a replayed child exit supersedes the stale pre-crash running status. Repeated passes detect existing closure/loss records instead of generating them again.
5. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then truncate/fsync the outbox after validating all complete frames are committed and release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention and delete the outbox only when all complete frames are durably represented.

Only the lock owner reads/compacts/writes the outbox, including normal event-worker operation. An external pass cannot race the live writer.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity returns `live_or_unreachable`; unknown process identity returns `operator_reconciliation_required`. Preserve suspect state and the open interaction in both cases. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

For unreadable owner identity, expose the exceptional Director-owned `cafleet member recover <MEMBER_ID> --owner-terminated --expected-run-id <RUN_ID> --expected-owner-epoch <EPOCH> --evidence-ref <REFERENCE> --json` path. Require all four options together, with positive IDs/epoch and a nonempty reference to the operator's recorded attestation that the inspected owner and startup processes have terminated; incomplete options fail with usage exit `2`. Without that attestation, unreadable identity continues to return `operator_reconciliation_required`.

The Director obtains the run ID, owner epoch and PID/start identity from the inspected registry record and binds the operator evidence to that snapshot. Revalidate database/run identity, epoch and owner tuple after obtaining the SQLite write lock; a changed run or owner rejects the stale attestation rather than rebinding it. A matching live-process identity, held identity-matching lock or matching live health reply rejects the termination claim even with the flag. For an unchanged owner with unreadable identity and no positive live evidence, record `owner_termination_attestation` in `cleanup_diagnostics_json` with the original run/epoch, owner PID/start token, evidence reference, UTC timestamp and `source=operator_via_director`, then execute the database-only branch in the same CAS transaction. Preserve that provenance separately from machine-verified termination. This attestation enables bookkeeping closure of the identified owner only; it authorizes no child creation, input, approval answer or replay.

Once the recorded owner's termination is machine-verified or accepted through that explicitly attributed attestation, perform database-only reconciliation in one bounded immediate transaction, rereading the fixed database/run identity, placement, epoch, owner tuple and supplied attestation after obtaining the SQLite write lock. A completed terminal result uses the expectation-checked no-work return above; a concurrent live reconciler on unfinished recovery causes deferral. For unfinished recovery, advance the epoch and record the recovering process identity, retain separately attributed provenance, increment the gap count once and add `runtime_path_missing` to cleanup diagnostics.

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code/signal and existing event/message bytes. When a terminal outcome is established, finish that outcome with zero new event counts. Otherwise reserve sequences above the durable high-water mark, close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, while continuing manual terminal I/O; resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic, and preserve the actual error. Neither endless memory growth nor a claim of successful notification is a recovery strategy.

## Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket rather than multiplexer pane input. Retain the existing outer `PaneLock` and placement revalidation for ordering with deletion; the runner serializes the final check and child writes with its own terminal-state loop. The lock order is pane lock, short database validation, then bounded socket request; the runner never waits for the external pane lock while serving that request.

Control frames are length-prefixed JSON, capped at 64 KiB, with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. Encode the serialized UTF-8 JSON body length as a four-byte unsigned big-endian prefix; the inclusive 65536-byte body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. The runner supports `health`, handshake `prepare`/`start`, `receipt`, `submit_prompt`, `submit_exec`, `permission_inspect`, `permission_respond` and `shutdown`. `receipt` has `query`, `retire` and synchronized Director reconciliation actions. Submission replies distinguish `accepted`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence and byte/submission stage; the ID is null for a zero-byte attempt with no durable row. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

### Input control frames and deadlines

New input, permission and receipt requests have required top-level `protocol_version,kind,identity,expires_at_mono_ns`. `protocol_version` is the integer `1`. `identity` has exactly `database_identity_json,fleet_id,member_id,director_member_id,run_id,owner_epoch,launch_nonce`, binding the complete selected database and recorded owner tuple. For submissions, validate recorded database/run/nonce and historical associations before durable key lookup; apply current epoch/owner and active eligibility checks only to a new or live ready attempt. IDs/epoch are positive signed 64-bit integers; sequences and byte counts are unsigned 64-bit integers. `expires_at_mono_ns` is a positive unsigned 64-bit CLOCK_MONOTONIC nanosecond expiration. Additional required request fields depend on `kind`:

| Kind | Additional exact fields |
|---|---|
| `submit_prompt` | `operation_key,input_kind,message_id,literal`; input_kind is preview or prompt. |
| `submit_exec` | `operation_key,input_kind,message_id,literal`; input_kind is exec. |
| `permission_inspect` | `interaction_id,event_id,director_member_id`. |
| `permission_respond` | `interaction_id,event_id,director_member_id,expected_run_id,expected_owner_epoch,request_token,decision,decision_source,director_mode,mode_evidence_ref,decision_evidence_ref,operation_key`. |
| `receipt`, action `query` | `action,operation_key`. |
| `receipt`, action `retire` | `action,input_id`. |
| `receipt`, action `reconcile` | `action,input_id,expected_state,disposition,evidence_ref`. |

Preview requires a positive message ID; prompt/exec require null `message_id`. `literal` is the unchanged message/prompt/command text; the exec plan supplies its own `! ` prefix. Require nonblank operation keys, validate the ordinary single-line/control-character contract and enforce matching outer kind/input_kind. Permission IDs/expected epoch must agree with identity and the stored association. Reconcile is the private action used by the Director-owned input coordinator; its enum values match the durable state/disposition domains. Reject unknown, duplicate, missing or wrongly typed fields before effects. Existing health/prepare/start/shutdown frame compatibility stays unchanged.

Replies retain exactly `protocol_version` plus one of `result` or textual `error`. Input results have exactly `kind` plus the following field: submission/`submission`, inspection/`request`, permission_response/`response`, or receipt/`receipt`. Submission contains exactly `status,input_id,stage,output_seq,reason`; permission response and inspection carry the complete ordered public objects fixed by the CLI contract. A receipt object has exactly `status,input_id,submission`: missing has null ID/submission; present has a positive ID and submission object; retired or settled has a positive ID and null submission. Result kind must match the requested operation. Clients validate and reload durable input/audit rows from their supplied database connection; socket receipt metadata supplies no replacement durable identity.

The original invocation captures one expiration and carries it through framing, transit, queues, SQL and every native stage. Convert using paired clock samples `Instant_before,CLOCK_MONOTONIC,Instant_after`, validating sample order and checked arithmetic. Sender expiration adds the remaining original deadline measured at Instant_after to the earlier monotonic sample. Receiver deadline adds remaining expiration measured at the later monotonic sample to Instant_before. Scheduling delay can shorten this budget and grants no extension; use no wall clock. Clock failure, overflow or expiration fails before new reservation/native bytes while preserving prior committed or possibly transmitted state.

At the service, also cap new-operation admission to five seconds from the first observed frame-header byte and any earlier shutdown deadline. Delayed framing and queueing consume that same cap. Client exchange, lock acquisition, supplied-connection SQL scopes and stage advancement retain their earlier original/inherited deadline. The frame body limit remains 65536 serialized UTF-8 bytes, excluding its four-byte prefix; validate declared size before allocation and the complete serialized size before sending.

### Bounded input draining

Before a programmatic input attempt, drain actual human and child input through the existing manual queue, terminal parser, ordered observer and outer display. A drain returns either `Pending` or `Quiescent` with the actual cumulative `human_generation` and terminal `output_seq`. Quiescence describes transport and observation health; the input coordinator separately validates composer/request eligibility, ownership, reservation and the original deadline before a write.

Each call executes at most eight rounds under the original deadline. A round attempts at most one human read, one manual-queue PTY write, one child read, one display-queue write and one display flush, in that order. Skip an operation only for unavailable capacity or remaining quota, an empty queue or previously observed stream `EOF`. Let B be the positive configured `bytes_per_tick`; each actual read buffer is bounded by the minimum of 16384 bytes, the remaining stream quota and available queue capacity.

| Work per drain call | Inclusive bound |
|---|---|
| Human reads | Eight calls and B positive bytes. |
| Child reads | Eight calls and B positive bytes, including previously deferred child bytes. |
| Manual PTY writes | Eight calls and B positive bytes. |
| Display writes | Eight calls and B positive bytes. |
| Display flushes | Eight calls. |
| Final observation-health samples | One sample, taken only for a candidate completed quiet round. |
| Rounds | Eight; unresolved work returns `Pending`. |

`Quiescent` requires both actual reads to return `WouldBlock` in the same completed round, an empty manual queue, consumed deferred child bytes, successful parser/observer processing and positive free display capacity. The final health sample must be present, nondegraded, have `pending_count=0` and `pending_evidence=false`. An absent health adapter supplies no proof; a healthy `Hold(Unknown)` state may establish physical quiescence while remaining ineligible for programmatic input. The eighth round may certify these actual facts; quota exhaustion, skipped reads, `Interrupted`, `EOF`, outstanding manual bytes or display pressure supply `Pending` instead. A continuously readable source terminates at the round or byte bound. Deferred child bytes precede subsequent reads and consume the same child quota.

Count every actual positive human read immediately with checked cumulative unsigned 64-bit arithmetic, in both ordinary progress and input draining. Retain those counts and literal queued bytes when later PTY, child, display, parser or observer work fails. The service delivers the observed before/after count delta to manual-input invalidation on both successful and failed progress before propagating any error; a simultaneous notification failure accompanies the primary error. Actual nonrecoverable errors propagate, and a zero-length write retains the existing no-progress error. `Interrupted` requires a later fresh read pair; `EOF` records the stream state and leaves shutdown handling to the normal service path.

Clear stored drain proof at entry, then check the original deadline before I/O and before each endpoint/callback. Recheck after completed work, retaining the actual bytes/counts from successful completed I/O before reporting expiry. In-memory bookkeeping and observation of a completed read may finish after expiry; an observed expiration permits no later endpoint call or proof installation. Synchronous I/O and callbacks have no preemption guarantee, so finite work bounds and clock checks supply no hard syscall or callback latency cap. Every call retains the original deadline and performs no internal sleep or deadline renewal.

Retain every completed child read for display and preserve the exact prefix processed by the terminal parser and ordered observer. If a positive child chunk fails to complete that observation, retain an observation-incomplete fence for the `Transport` lifetime. Subsequent drains continue bounded manual and display progress and return `Pending` without proof, including when later health is healthy or the caller supplies a new deadline. Ordinary progress, resize and display flushing retain the fence. Fully completed observation remains complete when only a later deadline or display operation fails. Partly processed chunks are neither replayed nor treated as observed suffixes; the runtime exposes no repair/reset operation for this fence.

Only a successful final check installs an opaque, single-use proof bound to its transport instance, drain turn and actual health/generation/sequence facts. The visible result fields describe evidence and grant no write authority. Taking the stored proof consumes it once. Ordinary progress, resize, prefetch, display mutation/discard and a new drain invalidate it before their validation or I/O, including later error paths; pure generation/snapshot reads preserve it. A new transport begins with generation 0 and no proof. Programmatic writing consumes actual coordinator-owned authority rather than a caller-constructed result.

### Durable input receipts and settlement

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state, permits reevaluating input eligibility. A held/rejected zero-byte attempt creates no side-effect receipt; if a row was already reserved, move it to `ready` only after positively recording that zero bytes were written.

An ordinary `receipt query` reads the durable operation for its fixed run/key and reports its durable status together with the last real retained stage and output sequence. `missing` requires an absent durable row and absent matching cache entry. A durable row remains `present` after cache retirement; a cache entry without its durable reservation is an identity error. Validate the retained immutable input identity before using its stage.

| Durable receipt | Status | Stage without a cache entry |
| --- | --- | --- |
| `ready` | `held` | `zero_bytes`. |
| `in_progress` or `uncertain` | `uncertain` | `unknown`. |
| `accepted` or `settled/submitted` | `accepted` | `submitted`. |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected` | `unknown`. |

With a matching retained entry, return its actual stage and output sequence alongside that durable status; otherwise the output sequence is null. Query preserves pending writers, clients, permits, receipts and native bytes. Its results describe observations: new write authority and zero-byte retirement proof still require their separate validated guards. A settled `not_submitted` or `abandoned_uncertain` disposition alone supplies no observed byte stage; its absent-cache result remains `unknown` and leaves the original transmission uncertainty intact.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may reconcile its retained stage evidence into the same row later; a crashed runner's in-progress row is never an automatic replay instruction.

Retire only the in-memory receipt after validating durable settlement, the current full run/owner and original run lock, exclusion of every active writer, and absence of a leased guard or available attempt permit for that input. Cache absence is idempotent only after the same durable and ownership checks. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout is merely a presentation of its input ID and supplies no durable acknowledgement. Retain settled operation keys/digests/dispositions for the run's database lifetime, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts to 4096, rejecting additional new side-effect attempts while that unresolved set is full; settled receipts leave the bounded set. Keep uncertain receipts until the Director records an evidence-based disposition through `cafleet input reconcile <INPUT_ID> --disposition <submitted|not-submitted|abandoned-uncertain> --evidence-ref <REFERENCE> --json`. This Director-owned operation only records disposition, settles the key and permits cache retirement; it sends no bytes, grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation pins the original input row in its supplied-connection snapshot: `input_id`, `run_id`, `operation_key`, `kind`, `payload_digest`, `message_id`, `created_at`, the complete immutable permission-response audit where present, and expected state remain fixed through live preference and exclusive settlement. For a recorded concrete placement, acquire its genuine pane/input lock and retain that same guard through current-owner synchronization and any receipt-only exclusive ownership proof. Synchronize with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently; an unreachable owner requires the exclusive proof below. Compare-and-swap the original expected operation state in the settlement transaction; a changed row requires a separate invocation. Settlement records the observed input disposition, not the native command's execution outcome.

For an ordinary or permission receipt already settled with the requested disposition, reconciliation returns the unchanged validated durable row and its original `evidence_ref` before live locks, health probes or exchange. A conflicting disposition fails with the row unchanged. This historical return uses the original deadline clipped to any inherited SQL bound and grants no new input authority. For a live nonsettled receipt, validate the full fixed identity and original ownership lock inside the same supplied-connection immediate transaction as the expected-state settlement CAS, after the SQL wait. After a new reconciliation, require the matching `settled` acknowledgment and a validated durable reread, then exactly one `retire` exchange with a matching input ID under the same pane guard and original expiration. A post-commit failure retains the known input context and original cause while the durable settlement remains authoritative. A changed expected state requires a separately reconciled attempt.

Receipt-only reconciliation for ordinary and permission inputs accepts the fixed placement state from that original snapshot. A present placement requires its matching, continuously held pane/input guard. An absent placement requires an absent guard, valid historical member/fleet/Director relationships and a deregistered member or deleted fleet; an active member in a nondeleted fleet requires its recorded placement. Revalidate the exact placement or its exact absence, complete fixed run/input identity and historical relationships inside the supplied-connection immediate transaction after preceding waits. A changed placement, identity or expected input state invalidates the attempt.

Both placement states require the recorded identity-matching original run lock and positive exclusion of the previous owner and relevant startup processes under the original deadline clipped to any inherited SQL bound. A held original lock, matching live process/health, unresolved termination or competing live reconciler preserves uncertainty. Missing or replaced runtime paths supply no original-lock proof. Placement absence, a failed live call and prior exceptional bookkeeping each grant no takeover authority; retain the original live failure when typed exclusion cannot be established. Valid absent-placement history proceeds directly to the same exclusive proof.

Its receipt-only CAS advances only the ownership tuple: increment `owner_epoch`, set `owner_kind=reconciler` and record the current reconciler PID/start identity. Commit that CAS, expected-state settlement and any matching claim removal authorized by a genuine zero-byte proof atomically in one supplied-connection immediate transaction; any failure rolls all those mutations back. Preserve lifecycle status, recovery origin, exit fields, cleanup state/diagnostics, events, reservations, spool and immutable input/audit history. Retain the exclusive guard through commit and release it on every outcome. This path performs no observation recovery, child operation or native input; ordinary member recovery retains its completed-terminal no-work behavior.

A durable ready row is positive zero-byte evidence only under actual writer exclusion and exact associations. Owner death and an attributed not-submitted disposition alone supply no such proof; recovered in-progress, accepted and uncertain records retain their transmission uncertainty. Proven-zero settlement and matching claim release commit atomically under the same proof lifetime. A ready response from a different epoch/token requires a fresh inspection and separate deliberate key; its immutable metadata is never rebound.

The live input guard distinguishes native-write authority, receipt-only writer exclusion and current-plan ready rearm. A production zero-byte proof borrows its still-held guard, binds the complete immutable input/audit and current ownership, and carries a non-Send marker; its constructors and fields remain private. Receipt-only exclusion authorizes bookkeeping without a write permit or current visible prompt. With a retained entry, proof requires its actual zero-byte stage and cumulative count zero; without one, only a validated durable ready row supplies that fact under genuine writer exclusion. A projected retry stage or a settled not-submitted label supplies no replacement observation.

Rearm additionally requires the unchanged old token/epoch/metadata and the actual current request, selected answer plan, manual generation and healthy complete observation drain. Revalidate owner/audit/claim under the bounded immediate transaction, CAS ready to in-progress once, then require fresh normal pre-stage checks before any native byte. Retain the input ID, original audit, decision timestamp, key and digest. Preserve its matching claim, or restore that exact pair only when a released ready row has no competing claim. The exclusion-only proof grants no rearm authority. Each explicit ready invocation captures one expiration; an in-progress stage continuation retains its original attempt expiration.

Live permission query, reconciliation and settled retirement use the existing serialized receipt operations and original run lock. Query preserves the last real retained stage and any pending writer/client. Reconciliation pins the original full input/audit snapshot, excludes active writers and leases, and commits a proven-zero not-submitted settlement plus matching claim deletion in one supplied-connection immediate transaction. Actual retained zero-byte in-progress evidence may establish that synchronized settlement; possible-byte or absent-cache uncertainty retains its claim. Submitted and abandoned-uncertain dispositions retain claims. A mutation failure rolls back settlement and claim removal together. Retirement removes only settled cache after validation; durable audit, key and claim history remain. The client retains its genuine pane guard and original inherited deadline through query, settlement, durable verification and one matching retire; already-settled same-disposition history returns unchanged before live work. An unavailable permission owner uses the same fixed-snapshot receipt-only exclusive guard. Only exact durable ready history supplies its zero-byte proof after positive prior-owner/startup exclusion and ownership CAS; disposition and matching claim removal share that transaction. Possibly transmitted history retains its claim. The native startup probe reports unknown, so unavailable exclusion preserves the original live failure.


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

Retain the existing body folding, inline limit and long-body `message show` pointer. Wrapped previews end at the resume clause because their submission sends no Escape. Unwrapped previews retain their existing full line and Escape-dismissal explanation. Held messages keep `notified_at` unset, and only confirmed submission or the existing ACK settlement stamps them. Holding supplies no new permission event and never closes the native request. Native-exec success means submission, not execution success.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees suppression for observed/candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race, rather than asserting universal prompt preservation. If a profile cannot safely distinguish an empty composer and its concurrent-work boundary, that profile supports manual input only. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; ordinary preview/prompt/exec input remains held during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation. Retain the legacy mux path only for explicitly unwrapped members and preserve its narrower existing semantics.

### Explicit manual key input {#explicit-manual-key-input}

An explicitly authorized `member press` sends only its requested ordered keys through the member's outer multiplexer pane under the shared pane lock and [selected-context/deadline contract](multiplexer-backends.md#explicit-key-dispatch). It appends no Escape or Enter. This manual path remains available during observer degradation or a programmatic composer hold and follows the [separate CLI acknowledgment/error contract](cli-options.md#member-press).

The runner counts actual positive outer-terminal reads, retains their generation before any later I/O error and accounts each new delta once on both successful and failed transport steps before subsequent automatic advancement. Observed manual input invalidates automatic authority through the retained generation; a key argument or backend acknowledgment supplies no byte count, drain proof or permission receipt. Press changes no durable input/response row. Native approval/denial/cancellation and actual continuation remain independently observed outcomes under the existing Director/user policy.

## Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative. Mode/source fields are attributed audit claims in this same-user coordination tool, not authentication credentials.

Runtime event payload schema version 2 includes the broker-generated `director_action` field. A permission-request `interaction_opened` event has `kind=review_permission_request`, a constant `instruction` and `inspect_argv`; every other new event has null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." Build inspection argv from validated stored identities, never from screen text. Validate the action against its event/run/Director association. Preserve existing schema-version-1 records, their exact message bytes and idempotent replay; all new events use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. Metadata/redacted evidence may omit necessary facts; use authorized current capture and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction. Apply the selected evidence policy to inspection output and persistence; store bounded opaque evidence references and decision metadata rather than copying raw commands or secrets into receipts.

The Director-only CLI is:

```text
cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --json
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY --json
```

Require positive IDs/epoch, nonempty operation keys and bounded nonblank references. `director-auto` requires mode `auto`; source `user` records a user decision in any mode. Validate the active pinned Director/fleet/member relationships. `inspect` verifies the current durable event/message/interaction association, retains the genuine concrete pane lock, probes the pinned owner and resolves unread output/manual progress under the original deadline clipped to any inherited SQL bound. Current authority comes from the actual parsed screen, a complete single-request choice/focus match and the stable tracked episode. Observation freshness is checked independently of ordinary composer eligibility; inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Return current identities/visibility/classification, selected detector/answer-profile metadata, supported decisions/capability reason, post-policy evidence, missing-evidence indications and an opaque token. Unsupported or blocked capability is an explicit result, and incomplete observation supplies no token.

The runner issues one current random token bound in memory to the complete database/run/nonce/epoch, interaction/episode, native version, live operation/choice/focus identity, manual-input generation and full immutable decision-to-plan set, including every selected profile, rule and stage identity. A checked invalidation generation records the first incompatible candidate/request, obscurity, observation gap/degradation or incomplete observation, even when the original display returns before inspection. Positive actual manual input, shutdown and ownership loss also revoke authority; arithmetic or randomness failure returns an explicit error. Repeated compatible inspections reuse the token. A tested equivalent redraw/resize preserves it only when the exact profile proves unchanged semantic/choice/focus and plan identity; output sequence alone is evidence rather than revocation. Healthy transient queue pressure defers authority without rebinding the token. The random value is independent of hidden-text hashes and authorization; a revoked token requires fresh inspection and decision reconciliation.

A cursor transit is the one incompatible frame that leaves the generation unchanged. It is a frame that yields no current request while a request is current, is not a verified non-interaction, and keeps every row of the current request's matched live region identical in text to the frame that established the request; typically the native CLI moves the cursor outside the form to redraw another part of the screen. During a transit the request is not current, so inspection returns no token and a response is blocked. A following frame with the same request restores it under the unchanged generation and token. A changed region row, a different request fingerprint, closure, `interaction_obscured`, an observation gap, child exit and positive manual input still invalidate immediately.

Inspection exposes the selected current post-policy evidence independently of private request/match/plan state. Missing facts follow the [permission inspection projection](cli-options.md#permission-commands); neither operation text nor private plan eligibility substitutes for a disclosed structured fact. A proved `single_request` choice establishes answer eligibility independently of whether public evidence states the choice scope. Keep transient raw semantics, focus and plan bytes in memory; inspection preserves the existing evidence schema and persists no token or pre-policy digest.

Validate a retained operation key against its recorded database/run, response identity and canonical digest before any current-request eligibility check, pane lock or live-owner probe. A matching retained receipt returns its stored result after token revocation, interaction closure, deregistration or owner exit, sending zero bytes; conflicting reuse fails. Validate historical audit relationships against recorded ownership while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation. The durable-only response projection preserves the immutable audit epoch and uses a null output sequence under the [permission CLI contract](cli-options.md#permission-commands); a consumed retry's zero-byte stage describes that retry while retaining original disposition and claim uncertainty.

Inspection reports the shared answer-profile version. Its answer_rule_id is the common rule only when every supported decision resolves to that same rule; one supported decision reports its actual rule, and differing rules yield null. Respond selects the requested decision's actual rule internally and retains that required rule in its immutable audit/digest. A blocked or unsupported request retains the empty-decision/null-token representation.

Use versioned control operations `permission_inspect` and `permission_respond` and the existing receipt query/retire boundary. A five-second command deadline covers locking, database work and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer profile, and commits the response receipt, audit and episode claim atomically under the current ownership epoch. Other interaction classes, uncertain state, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's single-request answer plan, rechecking relevant state before each stage; partial writes or ambiguous state retain durable uncertainty. The CLI accepts semantic allow/deny decisions, not arbitrary key sequences.

Track answer progress as cumulative actual positive byte counts across ordered stages. A complete nonfinal stage retains `partial` progress and durable `in_progress`; only complete final-stage submission establishes `submitted`, followed by durable `accepted` before returning acceptance. A stage syscall returning WouldBlock or Interrupted with zero bytes may yield within the same attempt and original expiration. Retain the untouched next-stage index and any cumulative `partial` progress, then retry only that stage after fresh actual drain, current token/request/choice/focus/plan, manual-generation, owner and immediate-transaction validation through a newly consumed guard. Completed stages remain completed. This continuation supplies neither a new invocation nor a ready-state rearm.

Record a syscall result before yielding. A recorded zero-byte WouldBlock or Interrupted preserves the actual prior stage/count and releases its lease while consuming that guard; a yielding operation remains `in_progress` and supplies no zero-byte retirement proof. Dropping a consumed guard before I/O preserves its actual no-I/O state, while possible I/O without a recorded result retains uncertainty and every known earlier byte count. A positive short write, incompatible state, unrecorded possible I/O or expiration stops the remaining plan. Preserve cumulative progress, the original cause and any known input identity; persist uncertainty only within the original budget, retaining durable `in_progress` when bookkeeping fails.

V5 preserves existing records and expands the input-kind domain with `permission_response`. Table `member_permission_responses` is keyed by `input_id` with event/interaction foreign keys, expected owner epoch, request token, decision, pinned Director ID, attributed Director mode/source, mode/decision evidence references, answer-profile version/rule and decision timestamp. Table `member_permission_response_claims` is keyed by interaction ID with a unique input foreign key, to retain one possibly transmitted answer per observed episode across different operation keys. Validate same-run/event/interaction relationships and all required enum/identity constraints transactionally. The storage subsection below fixes exact SQL names, types and associations.


### Permission response storage

V5 preserves V4 identities and rows, expands the input-kind check to `kind IN ('preview','prompt','exec','permission_response')`, and creates the following tables. Preserve the existing input-operation columns, key uniqueness, foreign keys and settlement constraints while replacing its kind check.

```sql
CREATE TABLE member_permission_responses (
    input_id INTEGER PRIMARY KEY REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    event_id INTEGER NOT NULL REFERENCES member_runtime_events(event_id) ON DELETE RESTRICT,
    interaction_id INTEGER NOT NULL REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    expected_owner_epoch INTEGER NOT NULL CHECK(expected_owner_epoch > 0),
    request_token TEXT NOT NULL CHECK(length(request_token) = 64 AND request_token NOT GLOB '*[^0-9a-f]*'),
    decision TEXT NOT NULL CHECK(decision IN ('allow','deny')),
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    director_mode TEXT NOT NULL CHECK(director_mode IN ('auto','manual','unknown')),
    decision_source TEXT NOT NULL CHECK(decision_source IN ('director-auto','user')),
    mode_evidence_ref TEXT NOT NULL CHECK(length(CAST(mode_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    decision_evidence_ref TEXT NOT NULL CHECK(length(CAST(decision_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    answer_profile_version TEXT NOT NULL CHECK(length(trim(answer_profile_version)) > 0),
    answer_rule_id TEXT NOT NULL CHECK(length(trim(answer_rule_id)) > 0),
    decided_at TEXT NOT NULL CHECK(length(trim(decided_at)) > 0),
    UNIQUE(interaction_id, input_id),
    CHECK(decision_source != 'director-auto' OR director_mode = 'auto')
);

CREATE TABLE member_permission_response_claims (
    interaction_id INTEGER PRIMARY KEY REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    input_id INTEGER NOT NULL UNIQUE REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    FOREIGN KEY(interaction_id, input_id)
        REFERENCES member_permission_responses(interaction_id, input_id) ON DELETE RESTRICT
);
```

All audit fields are required and use the displayed column order. `decided_at` is the UTC RFC 3339 time when the runner records the attributed decision, independently of native decision time. Tokens are random 32-byte values encoded as lowercase hex; persist no screen-derived fingerprint or pre-policy hash. Evidence references contain 1–2048 UTF-8 bytes, include a non-whitespace character and contain no C0 or DEL control character; they identify host evidence without embedding commands, screen text or secrets.

In the same immediate transaction, require input kind `permission_response`, an active pinned Director, an inbox-linked permission-request opening event and the matching unclosed interaction/run. Validate the expected epoch, event/message association and answer profile before inserting the input, immutable audit and claim. Other input kinds have no audit row. The composite claim foreign key binds its input to the same interaction; same-run/event and recorded ownership relationships are also checked on reads while preserving closed/inactive history; active/current eligibility applies to new attempts only.

The canonical payload digest covers `run_id,owner_epoch,interaction_id,event_id,director_member_id,request_token,decision,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id`. Encode this metadata as canonical JSON with sorted keys before SHA-256; `input_id`, timestamps, operation key and literal native keys are excluded. Validate a matching key against this digest before any new action.

Retain a claim after possible submission, accepted submission, uncertain settlement or `abandoned_uncertain`. Release it only in the synchronized transaction that proves zero bytes, no active writer and `ready` or settled `not_submitted`; a disposition label alone supplies no zero-byte evidence. Audit rows and consumed operation keys remain retained after claim release.

Reuse input receipt states, settlement dispositions, the 4096 unresolved limit and synchronized retirement. Digest canonical non-secret request identity, token and decision metadata, excluding screen text and literal key bytes. The Director records the operation key before dispatch. Same-key retries return the existing result; conflicting payloads fail. A different key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim. Release a claim only after synchronized evidence proves zero bytes and no active writer; a later deliberate attempt uses fresh inspection and a new key. Recovery sends no answer keys. Key submission and recorded `decision=allow` do not establish native approval or command success.

Append nullable `permission_response` audit data to the ordered input-operation output, null for other input kinds. Inspection returns a `request` wrapper. Response output identifies status (`accepted`, `held`, `rejected`, `uncertain`), nullable input ID, interaction/run, decision/source, submission stage and output-sequence evidence. Exit 0 means accepted key submission or the matching previously submitted receipt; usage failures exit 2 and operational/uncertainty errors retain existing categories and known input IDs. The [permission CLI contract](cli-options.md#permission-commands) fixes complete JSON key order, nullability, text layouts and failure output. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability is exact-version and independently tested for allow and deny. It specifies request form/rule, single-request choice semantics, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes and answers to trust/update/question forms require their own explicit scope. Unsupported answer profiles preserve manual native input. The screen interface has no atomic native request API; serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

## Director consumption and continuation

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

## Native acceptance and coverage

Use synthetic terminal replay and real native PTYs for every declared detector, composer and answer capability on each registered multiplexer. Fixed-policy or unsupported profiles retain explicit unknown coverage and manual operation.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Pin exact installed CLI versions by the acceptance records and corresponding bundled profiles; use permitted disposable fixtures that actually reach the native approval surface. Every minimum case must demonstrate blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and preserved original request. Exercise separate fresh requests for verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, and manual original-surface input; include a user-executed complete bound command. Record zero response bytes before required user permission, actual native results and observed continuation separately from submitted keys. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Record source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, Director mode/provenance, attributed decision/evidence references, request identity/token, receipt/stage, multiplexer, terminal size, cwd and redacted evidence for live acceptance. Include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. Exercise stale tokens/epochs/events, same/different operation keys, conflicting decisions, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.


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
- Test every registered backend's omitted-policy argv and each adapter-declared explicit policy; profiles claiming detection, composer or answer support require both fixtures and real native evidence.
