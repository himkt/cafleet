# Coalesced ID-only inbox notices for `message listen`

**Status**: Approved
**Progress**: 21/48 tasks complete
**Last Updated**: 2026-11-15

## Overview

`cafleet message listen` stops forwarding one full message body per pending message. It now emits a single ID-only `pending` notice and holds later notices until the Director ACKs every ID the outstanding notice named. While the hold lasts, new arrivals coalesce into the next notice, so notices are bounded by the Director's ACK rounds and the 300 s hold ceiling instead of by message arrivals (§ Notice bound). Each stale notice costs only one empty or partial `message poll`.

## Success Criteria

- [ ] `message listen` emits exactly three record kinds: `ready`, `pending` and `closed`. The body-carrying `message` record and the Codex `message_ref` reference no longer exist in code, SPEC, skills or docs.
- [ ] A burst of N messages arriving while a notice is held produces no further notice until every ID of the held notice is ACKed. After that, one notice names the full pending set.
- [ ] A held notice whose IDs stay partly unACKed is re-sent with the full pending set once 300 s have passed since its emission, but only when IDs the listener has not yet notified are pending.
- [ ] A Codex full-queue refusal never ends the forwarder while the pinned session lives. Each retry rebuilds the notice from the current pending set, and a notice whose set has become empty is abandoned. The `queue stayed full for 600 s` failure is gone.
- [ ] A forwarded `closed` record accepted after a refusal of any length is still written to stdout.
- [ ] The Hermes, OpenCode and pi `cafleet_listen` descriptions describe ID notices. After `cafleet setup` re-runs, `cafleet doctor` reports every extension check `ok`.
- [ ] Unit tests in `src/cli/message.rs` cover coalescing, the hold, release, the hold ceiling, restart replay, `Refused` rebuild and commit, the `Delivered` assertion for `ready`/`closed`, Codex rebuild/abandon, and indefinite closed-record retry with its stdout write. `mise //:test` and `mise //:lint` pass.
- [ ] A live subscribe probe, including the new coalescing and hold-ceiling steps, passes on every bundled Director backend: `claude`, `codex`, `opencode`, `hermes` and `pi`. SPEC § Role eligibility `listen` rows cite the new evidence and check date.
- [ ] The live probes run against an isolated install and database, and the shared installed binary, skills, presets and extensions stay unchanged until the execute Director authorizes their replacement.
- [ ] SPEC §6.7 and the `coding-agents.md` notes record the downstream delivery-queue limit found in installed upstream source for OpenCode `session.synthetic` queue delivery, pi `followUp` delivery and Claude Monitor line delivery.

---

## Background

`listen` currently emits `{"event":"message","message":{…full record…}}` for each newly pending ID, using a process-local set to suppress repeats while an ID stays pending. Codex forwarding (`--codex-thread`) queues each record as its own user turn through `codex queue`. Codex drains one queued turn per turn boundary and offers no way to list or retract queued submissions.

Observed failure: messages 51–86 were created 12:11–12:15. The Director reconciled them through `message poll` and `ack` during long turns, yet their already-ACKed `[cafleet listen record]` turns kept arriving one per turn until after 13:05. Arrivals outpaced the one-record-per-turn drain, the 100-submission queue filled, and the forwarder exited with `codex thread … queue stayed full for 600 s`. Its closed notice met the same full queue, so reception stopped silently.

The extension receivers (OpenCode, Hermes, pi) pass every stdout line through verbatim as `[cafleet listen record] <line>` and inspect only the `{"event":"closed"` prefix. Hermes has the same failure shape against its gateway's 32-row admission queue, with a 1 s retry and a 600 s refusal kill. The repository records no limit for OpenCode, pi or Claude Monitor delivery. Every Director reception recipe already runs `message poll --json` on each received record, so an ID-only notice fits the existing consumption step.

---

## Specification

### Scope

| In scope | Out of scope |
|---|---|
| `message listen` record set and emission rule, on stdout and `--codex-thread` | Ordinary-member pane previews, `message wait`, `message poll`, `message ack` |
| Codex forwarding full-queue handling | The `codex queue` API and the Codex thread queue cap |
| SPEC, `docs/`, skills and `contributing.md` probe text describing listen records | Extension delivery, retry and lifecycle code, including Hermes's 1 s retry and 600 s refusal kill |
| The model-facing `cafleet_listen` description string of every extension (§ Extensions and Claude Monitor) | — |
| Live re-verification on every bundled Director backend | An urgent-bypass path for runtime events |

No body-carrying mode remains behind a flag. Each cafleet version uses its own database and binary-embedded skills, so the old record shape needs no compatibility layer.

### Record set

```json
{"event":"ready","member_id":1,"fleet_id":1}
{"event":"pending","member_id":1,"message_ids":[51,52,53]}
{"event":"closed","member_id":1,"reason":"timeout"}
```

| Field | Type | Contract |
|---|---|---|
| `event` | string | `"pending"` |
| `member_id` | integer | The listened subject (the pinned recipient) |
| `message_ids` | array of integers, length ≥ 1 | The full `input_required` set for the subject at the snapshot that produced the notice, ordered `(created_at ASC, message_id ASC)`, the snapshot order |

`json.rs` key order is `event`, `member_id`, `message_ids`. `ready` and `closed` are unchanged. A `pending` record is never empty: an empty pending set emits nothing.

### Emission rule

The listener holds one piece of state, the **hold**. A hold exists exactly while some ID named by the last delivered notice is still pending. It carries the **held set** `H` (those IDs, never empty) and `held_at`, the monotonic instant at which that notice was delivered. With no hold, `H` is empty and there is no `held_at`. The listener starts with no hold. Constant: `HOLD_CEILING = 300 s`.

On every successful snapshot with pending set `P` (ordered), in order:

1. **Reconcile:** `H ← H ∩ P`. ACKed IDs leave the hold; when `H` becomes empty the hold, `held_at` included, ends.
2. **Fresh IDs:** `F ← P \ H`.
3. **Decide:**

| Condition | Action |
|---|---|
| `P` empty | Emit nothing |
| `H` empty and `P` non-empty (hold released) | Emit `pending(P)` |
| `H` non-empty, `F` non-empty and `now − held_at ≥ HOLD_CEILING` | Emit `pending(P)`, the stall-guard re-notice |
| Otherwise (`H` non-empty and `F` empty, or ceiling not reached) | Emit nothing |

4. **Commit:** when the output returns `Delivered` (stdout: the complete record plus LF flushed; Codex: `codex queue` exited 0), the hold becomes `H ← P`, `held_at ← now`. A `Refused` notice leaves the hold exactly as step 1 left it, and the next snapshot decides again (§ Output trait). An `Err` from the output ends the listener with exit 1, so it has no hold effect.

Invariant: every notice carries the full pending set, so every pending ID that any earlier notice named is in `H`. One set therefore distinguishes notified from fresh IDs, and the listener keeps no separate per-ID history. The listener asserts `!ids.is_empty()` when building a notice.

### Notice bound

Notices are bounded by Director activity, not by arrivals:

- A hold release emits at most one notice, at the first later snapshot whose `P` is non-empty.
- A hold-ceiling firing emits at most one notice and restarts the ceiling.
- No other event emits a notice.

A Director that ACKs every held ID in k separate poll rounds during one long turn, while messages keep arriving, can therefore queue up to k notices behind that turn, plus one per 300 s of the turn. Each notice after the first that turn consumes is likely stale, costing one empty or partial poll. Without a release or a ceiling firing, a burst of any size produces no notice beyond the one already outstanding. SPEC and the docs state this bound in these terms: at most one new notice per hold release and per ceiling firing.

Idle behavior keeps its current shape. A snapshot that emits nothing waits `IDLE_INTERVAL` (1 s, shortened by the lease) before the next snapshot. A snapshot that emits rechecks immediately. Lease, expiry, closure, the five-second work budget and snapshot deadlines are unchanged.

### Accepted behavior

| Situation | Behavior |
|---|---|
| The Director handled a notice's IDs through a mid-turn poll before the queued notice arrived | The stale notice costs one empty or partial poll |
| Listener restart | The new process emits one notice naming every pending ID right after `ready`, including IDs a previous listener already notified |
| The Director polls and ACKs IDs before any notice names them | Those IDs are never notified |
| A new message, runtime events such as `interaction_opened` included, arrives while a notice is held | No notice until the held IDs are ACKed or the ceiling re-notice fires; the ID is visible to any `poll` immediately |
| The Director leaves a notified ID unACKed and nothing new arrives | No re-notice; the ceiling fires only for fresh IDs |

SPEC §6.9's "two seconds from commit to receiver output" latency target applies when no notice is held. While a notice is held, the committed ID reaches receiver output in the next notice.

### Output trait

`ListenOutput::emit` returns a delivery outcome, so a refusal is not an error:

```rust
pub(crate) enum Delivery {
    Delivered,
    /// The receiver declined the record for now; the listener retries from a fresh snapshot.
    Refused,
}

pub(crate) trait ListenOutput {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError>;
    fn session_exited(&self) -> bool { false }
}
```

Only a `pending` record may return `Refused`:

| Output | `ready` | `pending` | `closed` |
|---|---|---|---|
| `DescriptorOutput` (stdout) | `Delivered` or `Err` | `Delivered` or `Err` | `Delivered` or `Err` |
| `CodexForwarding` | Written to stdout: `Delivered` or `Err` | One `codex queue` attempt: `Delivered`, `Refused` on a full-queue refusal, or `Err` | Retried in-output until accepted, then written to stdout: `Delivered`, or `Err` from a non-refusal failure |

The `listen` loop's ready emit and `close()` assert the contract: they unwrap the result with `?` and then `assert!(matches!(delivery, Delivery::Delivered), "only a pending notice may be refused")`. A violation therefore panics instead of passing as success. `DescriptorOutput`'s backpressure, write-zero, broken-pipe and deadline failures stay fatal errors, as today.

A `Refused` notice leaves the hold exactly as reconcile left it. The loop then sleeps `IDLE_INTERVAL` (1 s, shortened by the lease), checks `io.session_exited()` and takes a fresh snapshot, so the next attempt is rebuilt from the current pending set by the emission rule. A notice is abandoned when the rule emits nothing: a released hold whose `P` is now empty, or a re-notice that no longer has fresh IDs. `IDLE_INTERVAL` has no test override.

`EmittedMessages` is replaced by `NoticeHold(Option<Held>)` with `struct Held { ids: BTreeSet<i64>, at: Instant }`. The type makes the invariant structural: `held_at` exists exactly while a hold exists, and `Held.ids` is never empty. Its methods are:

| Method | Contract |
|---|---|
| `reconcile(&mut self, pending: &[MessageRecord])` | Intersects `ids` with `pending`; sets `None` when the intersection is empty |
| `notice(&self, pending: &[MessageRecord], now: Instant) -> Option<Vec<i64>>` | The § Emission rule decision; reads `at` only from a present `Held`, so no default exists |
| `commit(&mut self, ids: Vec<i64>, now: Instant)` | Sets `Some(Held { ids, at: now })`; asserts `ids` is non-empty |

`crate::output::json::listen_message` is replaced by `listen_pending(member_id, &[i64])`.

### Codex forwarding

| Record | Forwarding behavior |
|---|---|
| `ready` | Stdout only, never forwarded (unchanged) |
| `pending` | One `codex queue --thread THREAD --message '[cafleet listen record] <compact JSON>'` attempt with the 10 s deadline. A full-queue refusal returns `Delivery::Refused` immediately, with no in-output sleep. Exit 0 returns `Delivered`. |
| `closed` with reason `timeout`, `member_inactive` or `fleet_deleted` | Forwarded and retried in-output on every full-queue refusal at `full_queue_retry_interval()` (1 s), checking the pinned session before each retry, with no time limit. After acceptance it is written to stdout under a fresh output deadline, `work_deadline(Instant::now(), None)`, taken after `forward_record` returns. |
| `closed` `session_exited` | Stdout only (unchanged) |

The fresh deadline matters because `close()` passes a deadline five seconds from closure. A closed retry longer than that would otherwise fail `write_json_line`'s first `deadline.remaining()` check and silently drop the stdout `closed` record. The Codex recipe's recovery step reads that record from the exec session. The same fresh deadline applies to the `session_exited` substitute record written after a refused closing record meets the pinned session's exit.

- **Removed:** `FORWARDED_TEXT_LIMIT`, the `message_ref` reference text, `FULL_QUEUE_LIMIT`, `full_queue_limit()` and its `CAFLEET_TEST_CODEX_FULL_QUEUE_LIMIT_SECS` override, and the `codex thread <THREAD> queue stayed full for 600 s` diagnostic and its table rows.
- **Retained for the `closed` path only:** `FULL_QUEUE_RETRY_INTERVAL`, `full_queue_retry_interval()` and its `CAFLEET_TEST_CODEX_FULL_QUEUE_RETRY_MS` override, which only the in-output closed retry reads. A refused `pending` waits the loop's `IDLE_INTERVAL` instead.
- **Retained everywhere:** the 10 s `codex queue` deadline and its `CAFLEET_TEST_CODEX_QUEUE_BUDGET_MS` override, the full-queue refusal detection (`queue cannot contain more than`), session pinning and liveness, the `session_exited` paths and the closed notice for every non-zero exit after `ready`.
- **Argument size:** a notice's text grows by about one ID's digits plus a comma per pending message. Its forwarded text is bounded only by the platform's single-argument limit (131,072 bytes on Linux, roughly 16,000 seven-digit IDs). A spawn failure such as `E2BIG` takes the existing `failed to forward record to codex thread <THREAD>: could not start codex: …` row: closed notice, exit 1.
- A pinned session that ends while a notice is refused is caught by the loop's `io.session_exited()` check and closes with `session_exited`, as today.

### Error conditions (Codex forwarding, after the change)

| Condition | Behavior |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| No Codex session ancestor at startup | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| Full-queue refusal of a `pending` notice | Not an error: retried from a fresh snapshot after 1 s while the pinned session lives |
| Full-queue refusal of a forwarded `closed` record | Not an error: same record retried every 1 s while the pinned session lives |
| `codex queue` exits non-zero, other than a full-queue refusal, while the pinned session is alive | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, closed notice, exit 1 |
| `codex queue` exceeds 10 s while the pinned session is alive | Killed; stderr `codex queue for thread <THREAD> did not finish within 10 s`, closed notice, exit 1 |
| The pinned Codex process exits | stdout `closed` `session_exited`, exit 0, nothing forwarded |
| Any other failure after `ready` | Its existing diagnostic, closed notice, exit 1 |

### Extensions and Claude Monitor

The OpenCode, Hermes and pi receivers keep their delivery, retry and lifecycle code. Each `pending` line is delivered as `[cafleet listen record] <line>`, and the `{"event":"closed"` detection is unaffected. Claude Monitor delivers `pending` lines like any other stdout line.

Hermes keeps its 1 s retry and 600 s refusal kill against the 32-row admission queue. Under § Notice bound, listen alone fills that queue only when the Director completes more than 32 hold releases or ceiling firings within one busy turn. Otherwise the refusal path is reached when other injections fill the queue. The docs state that condition instead of a per-message rate.

The model-facing `cafleet_listen` description string changes in every extension, because "every broker record addressed to the Director arrives as its own turn" no longer holds:

| Extension | Location | New description |
|---|---|---|
| Hermes | `_SCHEMA["description"]` in `assets/extensions/hermes/cafleet/__init__.py` | `Start CAFleet Director reception for this session: each turn beginning '[cafleet listen record]' carries a pending notice naming unACKed message IDs; run 'cafleet message poll <director_id> --json' and ACK each consumed message.` |
| OpenCode | `cafleet_listen` `description` in `assets/extensions/opencode/cafleet.ts` | The Hermes text |
| pi | `cafleet_listen` `description` in `assets/extensions/pi/cafleet.ts` | The Hermes text with "each turn beginning" replaced by "each message beginning" |

Each of these is an embedded extension file, so `cafleet doctor`'s embedded-copy check reports an installed extension from the previous version as `differs from the embedded copy` until `cafleet setup --coding-agent <backend>` re-runs. The live verification in Step 9 runs that setup for every extension backend before probing.

### Director consumption

A `pending` notice is a wake signal, never a payload. On every notice the Director runs `cafleet message poll <director-id> --json`, reads full bodies, reconciles, records deferred actions and ACKs each consumed delivery immediately. ACKing every polled ID is what releases the hold. A notice whose IDs the poll no longer returns needs no action. The existing Director wake procedure keeps its step order and replaces "`message` record" with "`pending` notice".

### Probe redefinition

Both probes (SPEC §6.7 Listen requirement, `contributing.md` § Run a CAFleet subscribe probe and § Run a CAFleet stream probe) are restated in message-ID terms. A probe message ID is **observed in a notice** when a `pending` record in the listener's output, as delivered through the mechanism under test, names it.

| Probe | Pass condition (ID terms) | Invalidation rule |
|---|---|---|
| Subscribe | Each probe message ID is first observed in a notice; the Director then consumes it through the poll that notice prompts | A poll, `wait` or `show` run without a preceding notice turn that first reveals a probe ID invalidates the run; a notice-prompted `poll` is part of consumption |
| Stream | Each probe message ID is first observed in a notice in the running listener's output, and is ACKed while the same listener process still runs | Any `poll`, `wait` or `show` invalidates the run; the Director ACKs IDs directly from the notice |

The subscribe-probe Director runs under this instruction, given once after it consumes the probe member's ready:

> Consume broker messages only through your receiver. On each `[cafleet listen record]` turn, run `cafleet message poll <director-id> --json`, then ACK every listed message except one whose body is exactly `hold-probe keep`; run no other `poll`, `wait` or `show`. When I tell you to release the hold, ACK the `hold-probe keep` message.

| Step | Procedure | Pass condition |
|---|---|---|
| Idle | Send one message to the idle Director | A notice naming it starts a turn without input; the Director polls and ACKs it |
| Busy | During a foreground command of at least 60 s, send two messages | The command is not interrupted; a notice arrives no later than the end of that turn, and the poll it prompts returns both IDs |
| Burst | During a foreground command of at least 60 s and under 300 s, send 150 messages back to back | At most two notice turns arrive for the burst, and every ID is consumed through a notice-prompted poll. On Codex the forwarder stays alive and no `codex queue` refusal occurs. |
| Hold ceiling | With the Director idle, send `hold-probe keep`. Record t1, the time its notice turn starts. After that turn ends and 60 s have passed since t1, send `hold-probe 2`. | No notice arrives before t1 + 285 s. One notice naming both IDs arrives between t1 + 285 s and t1 + 315 s. The Director ACKs `hold-probe 2` and, when told, `hold-probe keep`. |
| Owner exit | End the Director session | No `cafleet message listen` survives (`ps`) |

t1 observes the held notice's delivery from the session side. The listener sets `held_at` when delivery succeeds, which precedes t1 by the receiver latency, and checks the ceiling once per 1 s idle interval. The ±15 s tolerance around t1 + 300 s covers both.

The stream probe's step-4 task prompt changes to: "Learn about broker messages only from that listener's output; run no `cafleet message poll`, `wait` or `show`. When a `pending` record appears there, ACK each message ID it names with `cafleet message ack <message-id>` and continue." Its run classification restates per ID. Invalid: an ID first observed outside a notice, or any `poll`, `wait` or `show`. Pass: every probe ID first observed in a notice and ACKed before the listener exited. Fail: an ID first observed only after the listener exited, or never observed in a notice.

### Live verification environment

The live probes run against an isolated install, so the shared installed binary, skills, presets and extensions stay untouched. This honors `.claude/rules/commands.md`, which requires preserving an active executing fleet's installed runtime and assets: the fleet executing this document, and another fleet with a Codex Director, run on this host from the shared install.

| Surface | Isolated value |
|---|---|
| Probe root | `<BASE>/.probe/`, a hidden agent-only folder under the supplied BASE |
| Binary | `mise //:build`, then `target/release/cafleet` copied to `<probe root>/bin/cafleet`. Every probe shell and probe Director session runs with `<probe root>/bin` first on `PATH`, so the extensions' executable record (`current_exe()`) names the isolated copy. |
| `HOME` | `<probe root>/home`, so every `~`-relative path resolves inside the probe root, including OpenCode's fixed `~/.config/opencode` skills path, which no variable relocates |
| Backend config roots | `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, `HERMES_HOME` and `PI_CODING_AGENT_DIR` set explicitly to their default subpaths under the isolated `HOME`. Each root is seeded by copying only the authentication and model-configuration files that backend needs to start a session, never by symlinking a shared directory or file, because setup writes into these roots (Hermes `plugins.enabled`, for example). |
| Database | `--database-url sqlite:///<probe root>/cafleet.db` on every `cafleet` command, the probe Directors' spawn prompts carrying that `DATABASE URL:` line, and `database_url` passed to every `cafleet_listen` call |
| Assets | `cafleet setup` run from the isolated binary under the isolated environment, followed by `cafleet doctor` under the same environment |

Before probing, the probe shell confirms isolation: `command -v cafleet` names `<probe root>/bin/cafleet`, and the backend's resolved config directory, which `cafleet doctor` reports, lies under the probe root. Isolated Hermes and Codex homes start their own gateway or daemon, so the probes queue no input into another fleet's sessions.

Replacing the shared install (`mise //:all-install` and `cafleet setup` under the operator's real `HOME`) is a separate, final step. The execute Director authorizes it only after confirming that no active fleet on the host depends on the shared install.

### Documentation surfaces

| Target | Change |
|---|---|
| SPEC §2 Architecture stance ("A persistent read-only listener emits pending inbox records through JSONL") | The listener notifies pending message IDs through JSONL |
| SPEC §6.3 command table row for `message listen` ("Stream pending inbox records through stdout") | "Notify pending message IDs through stdout" |
| SPEC §6.3 `message listen` | Record set, `pending` example, emission rule and hold, notice bound, accepted behavior, memory clause ("the held IDs") |
| SPEC §6.3 Codex forwarding | Forwarded events, removed large-record and full-queue-limit bullets, refusal and closed-retry rules, fresh stdout deadline after a forwarded `closed`, error table |
| SPEC §6.3 Output shapes and §6.4 Persisted shape summary | `ready`, `pending`, `closed` tagged records; listen carries IDs, not the full record |
| SPEC §6.7 Listen requirement and probes | ID-terms pass conditions, the split invalidation rule, the subscribe-probe instruction and steps, and the stream-probe prompt (§ Probe redefinition) |
| SPEC §6.7 Director extensions | Pass-through of `pending` lines; the new `cafleet_listen` descriptions; the Hermes refusal condition under § Notice bound; upstream queue findings |
| SPEC §6.7 Role eligibility | New `listen` evidence for every backend |
| SPEC §6.9 acceptance latency target | Applies while no notice is held |
| `docs/docs/concepts/overview.md` Core terms | Listener "notifies pending message IDs as JSONL" |
| `docs/docs/mixed-backend-team.md` § Receive results | "reads each notice from its listener, then polls" |
| `docs/docs/concepts/coding-agents.md` | Director-requirements paragraph and Codex row: reception pushes each pending notice; Hermes section: "Each pending notice arrives as its own turn" |
| `docs/docs/contributing.md` probes | Subscribe-probe instruction and steps, stream-probe prompt, the per-ID Invalid/Pass/Fail rows |
| `assets/skills/cafleet/reference/cli.md` subcommand summary row for `message listen` | "Notify pending message IDs through stdout" |
| `assets/skills/cafleet/reference/cli.md` § `message listen` | Record set, hold rule, notice bound, Codex bullets and error table |
| `assets/skills/cafleet/reference/coding-agents.md` | Claude, Codex, OpenCode, Hermes and pi reception recipes; Codex 100-cap note; upstream queue findings as notes |
| `assets/skills/cafleet/roles/director.md` § Director wake and § Receiver recovery | Notice-driven poll; drop "incomplete messages" wording for a notice |
| `README.md` | No change expected; confirm via `/update-readme` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Research downstream delivery-queue limits

- [x] Inspect the installed OpenCode 2.2.x source for `session.synthetic` with `delivery: "queue"` and record any queue cap, its refusal behavior and the version checked <!-- completed: 2026-11-15T14:40 -->
- [x] Inspect the installed pi 1.5.x source for `sendMessage` `deliverAs: "followUp"` and record any queue cap, its refusal behavior and the version checked <!-- completed: 2026-11-15T14:40 -->
- [x] Inspect Claude Code Monitor line delivery (installed version and official docs) for buffering or coalescing limits and record the finding <!-- completed: 2026-11-15T14:40 -->
- [x] Collect the three findings, plus the known Codex 100-submission and Hermes 32-row caps, in a hidden BASE research note for the documentation steps <!-- completed: 2026-11-15T14:40 -->

### Step 2: Update SPEC.md

- [x] Update the §2 Architecture stance sentence and the §6.3 command table row for `message listen` to say the listener notifies pending message IDs <!-- completed: 2026-11-15T14:44 -->
- [x] Rewrite §6.3 `message listen`: record set, `pending` example and field table, emission rule, hold ceiling, notice bound, accepted-behavior table and the memory clause <!-- completed: 2026-11-15T14:44 -->
- [x] Rewrite §6.3 Codex forwarding: `pending` refusal behavior, indefinite `closed` retry with a fresh stdout deadline, argument-size row; remove the large-record and full-queue-limit bullets and the 600 s error row <!-- completed: 2026-11-15T14:44 -->
- [x] Update §6.3 Output shapes and the §6.4 persisted-shape summary sentence for the `pending` record <!-- completed: 2026-11-15T14:44 -->
- [x] Restate §6.7 Listen requirement subscribe and stream probes in ID terms: pass conditions, split invalidation rule, subscribe-probe instruction and steps (§ Probe redefinition) <!-- completed: 2026-11-15T14:44 -->
- [x] Update §6.7 Director extensions with `pending` pass-through, the new `cafleet_listen` descriptions, the Hermes refusal condition and the Step 1 findings <!-- completed: 2026-11-15T14:44 -->
- [x] Qualify the §6.9 acceptance latency target to apply while no notice is held <!-- completed: 2026-11-15T14:44 -->

### Step 3: Update docs/

- [x] Update the listener Core term in `docs/docs/concepts/overview.md` <!-- completed: 2026-11-15T14:46 -->
- [x] Update `docs/docs/mixed-backend-team.md` § Receive results for notice-driven polling <!-- completed: 2026-11-15T14:46 -->
- [x] Update `docs/docs/concepts/coding-agents.md`: the Director-requirements paragraph, the Codex reception row and the Hermes per-record sentence <!-- completed: 2026-11-15T14:46 -->
- [x] Update `docs/docs/contributing.md`: subscribe-probe instruction and steps (including burst and hold ceiling), the stream-probe task prompt and its per-ID Invalid/Pass/Fail rows <!-- completed: 2026-11-15T14:46 -->
- [x] Run `/update-readme` to confirm `README.md` needs no change <!-- completed: 2026-11-15T14:46 -->

### Step 4: Update skills

- [x] Rewrite `assets/skills/cafleet/reference/cli.md` § `message listen` (record set, hold rule, notice bound, Codex bullets, error table) and its subcommand summary row <!-- completed: 2026-11-15T14:49 -->
- [x] Update every reception recipe in `assets/skills/cafleet/reference/coding-agents.md`: Claude, Codex (drop the `message_ref` sentence and restate the 100-cap note), OpenCode, Hermes and pi <!-- completed: 2026-11-15T14:49 -->
- [x] Add the Step 1 findings as Note → applies at rows bound to each backend's native reception recipe <!-- completed: 2026-11-15T14:49 -->
- [x] Update `assets/skills/cafleet/roles/director.md` § Director wake steps 2–3 and the § Receiver recovery incomplete-output row <!-- completed: 2026-11-15T14:49 -->
- [x] Run `mise //docs:build` and fix broken links or anchors <!-- completed: 2026-11-15T14:49 -->

### Step 5: Implement the notice and hold in listen

- [ ] Replace `listen_message` with `listen_pending(member_id, &[i64])` in `src/output/json.rs`, key order `event`, `member_id`, `message_ids` <!-- completed: -->
- [ ] Add the `Delivery` enum and change `ListenOutput::emit` to return `Result<Delivery, CafleetError>`; `DescriptorOutput` returns `Delivered` <!-- completed: -->
- [ ] Replace `EmittedMessages` with `NoticeHold(Option<Held>)` (reconcile, notice decision, commit) and `HOLD_CEILING = 300 s` <!-- completed: -->
- [ ] Rewrite the `listen` loop: one notice decision per snapshot, commit only on `Delivered`, sleep `IDLE_INTERVAL` after an emit-nothing or `Refused` snapshot, and assert a non-empty ID list <!-- completed: -->
- [ ] Assert `Delivery::Delivered` for the ready emit and in `close()` <!-- completed: -->

### Step 6: Implement Codex forwarding changes

- [ ] `CodexForwarding::emit` for `pending`: one `codex queue` attempt; full-queue refusal → `Refused`; other failures keep their existing errors and `session_exited` paths <!-- completed: -->
- [ ] Keep the in-output retry for forwarded `closed` records at `full_queue_retry_interval()`, with `FULL_QUEUE_RETRY_INTERVAL` and its `CAFLEET_TEST_CODEX_FULL_QUEUE_RETRY_MS` override, and remove its time limit <!-- completed: -->
- [ ] Write the stdout `closed` record, and its `session_exited` substitute, under a fresh `work_deadline(Instant::now(), None)` taken after `forward_record` returns <!-- completed: -->
- [ ] Remove `FORWARDED_TEXT_LIMIT`, the `message_ref` text, `FULL_QUEUE_LIMIT`, `full_queue_limit()` and the `CAFLEET_TEST_CODEX_FULL_QUEUE_LIMIT_SECS` override <!-- completed: -->

### Step 7: Update extension descriptions

- [ ] Replace the `cafleet_listen` description string in the Hermes, OpenCode and pi extensions with the texts in § Extensions and Claude Monitor, leaving all other extension code unchanged <!-- completed: -->

### Step 8: Unit tests in src/cli/message.rs

- [ ] Rewrite the scripted-inbox `listen` tests for `pending`: burst coalescing, the hold blocking new notices, release emitting the full pending set, partial ACK keeping the hold, and no notice for an empty set <!-- completed: -->
- [ ] Add hold-ceiling tests with the test clock: a re-notice at 300 s only with fresh IDs, none without fresh IDs, and the next ceiling measured from the re-notice <!-- completed: -->
- [ ] Add a restart-replay test: a fresh listener's first notice names every pending ID <!-- completed: -->
- [ ] Add a scripted-output test that returns `Refused` and then `Delivered`, with the inbox changing between them. Assert that the retried notice is rebuilt from the then-current pending set and that the hold commits only on `Delivered`. Also assert that a released hold whose pending set empties during refusals emits nothing. <!-- completed: -->
- [ ] Add a test that a scripted output returning `Refused` for `ready` or `closed` panics with `only a pending notice may be refused` <!-- completed: -->
- [ ] Rewrite the `codex_forwarding` subprocess tests: a forwarded `pending` text, refusal retry rebuilding from the current pending set, abandonment after the pending set is ACKed during refusals, and indefinite `closed` retry ending with `session_exited` when the session ends. The `pending` refusal tests run at the real 1 s loop interval, because `IDLE_INTERVAL` has no test override. <!-- completed: -->
- [ ] Add a `codex_forwarding` test: a forwarded `closed` refused for longer than 5 s is still written to stdout after acceptance <!-- completed: -->
- [ ] Delete `a_record_text_above_the_argument_limit_is_forwarded_as_a_message_reference` and `full_queue_refusals_past_the_limit_exit_one_after_a_single_closed_notice_attempt` <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` and fix every failure <!-- completed: -->

### Step 9: Live verification on every backend

- [ ] Build the isolated install under `<BASE>/.probe/` (§ Live verification environment): copy the release binary, create the isolated `HOME` and backend config roots seeded with copied authentication/config files, and select the probe database <!-- completed: -->
- [ ] Run `cafleet setup` for every bundled backend with the isolated binary and environment. Confirm that `command -v cafleet` names the isolated copy, that `cafleet doctor` reports every extension check `ok`, and that every resolved config directory lies under the probe root. <!-- completed: -->
- [ ] Run the redefined subscribe probe (idle, busy, 150-message burst, hold ceiling, owner exit) on an isolated `claude` Director through Monitor <!-- completed: -->
- [ ] Run the same probe on an isolated `codex` Director through `--codex-thread` on the escalated path, confirming the forwarder stays alive and the queue never refuses during the burst <!-- completed: -->
- [ ] Run the same probe on isolated `opencode` (`opencode mini --standalone`), `hermes` (TUI client) and `pi` (plain `pi`) Directors through `cafleet_listen` with the probe `database_url` <!-- completed: -->
- [ ] Record each run's evidence (backend, native version, mechanism, IDs, send/notice/ACK timestamps, receiver exit) in a hidden BASE evidence note <!-- completed: -->
- [ ] Update every SPEC § Role eligibility `listen` row and each backend's skill Role eligibility Basis cell with the new evidence and check date <!-- completed: -->

### Step 10: Replace the shared install

- [ ] Once the execute Director confirms that no active fleet on the host depends on the shared install and authorizes the replacement, run `mise //:all-install` and `cafleet setup` under the real `HOME` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-15 | Initial draft |
| 2026-11-15 | Review round 1: notice bound, the `Refused`/`Delivered` contract, structural hold invariant, fresh stdout deadline for a forwarded `closed`, retained closed-retry interval, extension descriptions in scope, ID-terms probes with an executable hold-ceiling step, missing documentation surfaces |
| 2026-11-15 | Review round 2: live probes run against an isolated install and database; replacing the shared install becomes a separate Director-authorized step |
