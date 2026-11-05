# Hermes Director: Persistent Heartbeat-Woken Message Listener

**Status**: Complete
**Progress**: 18/18 tasks complete
**Last Updated**: 2026-11-04

## Overview

A Hermes Director receives member messages through one persistent `cafleet message listen` receiver started with Hermes's native `heartbeat=60`, which wakes an idle Director with the receiver's new output, so no user prompt, listener restart or auxiliary `message wait` is needed between exchanges. Two live runs on Hermes 2026.9.24 decide adoption: the persistent recipe becomes the documented default only when both pass; any failure blocks for a user decision. The change touches `skills/cafleet/reference/coding-agents.md` § hermes and `SPEC.md` §6.7 only (GitHub issue #556).

## Success Criteria

- [x] § Live evidence records the installed versions and a classified Run A and Run B on Hermes 2026.9.24, every row labelled `observed`.
- [x] With both runs `Pass`, `coding-agents.md` § hermes carries the § Replacement texts Note row, `#### Hermes native Director reception` subsection and Director eligibility row verbatim, with placeholders substituted.
- [x] With both runs `Pass`, `SPEC.md` §6.7 carries the § Replacement texts hermes backend paragraph and Hermes Director eligibility row verbatim, with placeholders substituted.
- [x] `grep -rn "woken only by the receiver's exit notification\|<absolute cafleet path> message listen <director-id> --timeout 300" SPEC.md skills` returns nothing.
- [x] `grep -rn "heartbeat" docs/docs` returns nothing; no Rust source, CLI flag, test or other backend section changes; the Director requirements in `SPEC.md` §6.7 Role requirements and `coding-agents.md` § Role requirements are unchanged.
- [ ] With any run `Fail` or `not established`, § Live evidence holds the Programmer's paired blocker marker, `blocked (paragraph-Specification > Live evidence)` was sent, and Steps 5–7 remain unchecked until the user decides.

---

## Background

The current recipe (`coding-agents.md` § Hermes native Director reception) starts `<absolute cafleet path> message listen <director-id> --timeout 300` with `terminal` `background=true`, `notify=true`, and reads it with `process_manage` `action=log` between work steps. `notify=true` notifies only at process exit, so an idle Director wakes only when the 300-second lease ends; in practice operators either restarted the listener around each exchange or added a completion-notified `message wait --timeout 1800 --json` that had to be rearmed after every batch and produced timeout turns with no traffic.

Hermes's `heartbeat=N` (design 0000209 recorded a passing `heartbeat=60` stream probe) injects a notification with the output a still-running background process emitted since the previous tick. A bounded live test (2026-11-04, CAFleet 0.31.2, member 20, message IDs 98 and 102) showed heartbeat turns admitting real member output into an idle Director without a restart or auxiliary wait, but the replies were also inspected manually first, so it is neither a clean first-observation run nor a long-duration test. The installed `terminal` schema describes `heartbeat` as "For bounded jobs you must react to mid-run, never servers", which this design resolves for fleet-scoped reception.

---

## Specification

### Decisions

| Topic | Decision |
|---|---|
| Change surface | Skills and SPEC only: `skills/cafleet/reference/coding-agents.md` § hermes and `SPEC.md` §6.7 (hermes backend paragraph, Hermes Director eligibility row). `docs/docs` stays silent. No Rust, CLI, test or other-backend change; `heartbeat` is a Hermes tool argument, not a CAFleet flag. |
| Director requirements | Unchanged. Streamed background output (#547) remains established by `process_manage` `action=log`; heartbeat adds idle wake and is recorded as separate evidence in the eligibility row. |
| Default recipe | One persistent receiver without `--timeout`, started with `background=True`, `notify=True`, `heartbeat=60`, kept across messages, replies, idle periods and turn endings. Adopted only when Run A and Run B both pass. |
| Heartbeat interval | Fixed at `60`. Hermes clamps smaller positive values to 60, so a smaller value never shortens delivery. |
| Finite lease | Documented alternative for a bounded supervision phase or a host policy requiring finite leases: the same call with `--timeout <seconds>` (`1..=1800`), renewed only after verified termination while delegated work remains. |
| `persist_on_release=True` | Documented as an explicit operator opt-in for surviving conversation-lifecycle cleanup, with its limits: no survival across Hermes host exit, no guaranteed notification ownership after a session change. |
| Bounded-job caveat | A receiver scoped to one authorized fleet-supervision phase is a bounded job: it ends with its `closed` record (`fleet_deleted` or `member_inactive`) or explicit retirement, and the Director keeps it only while delegated work or an authorized user-feedback phase remains. The recipe states this; CAFleet claims no daemon support. |
| Heartbeat role | A batched wake signal, not a transport. Records are consumed from `process_manage` `action=log` reads and reconciled with `cafleet message poll <director-id> --json`; deduplication is by message ID. No `message wait` serves as a wake or rearm mechanism; a finite `wait` for ordinary-member retry rotation (Director wake step 4) stays available. |
| Failure policy | Any `Fail` or `not established` run blocks for a user decision; the finite-lease recipe is not adopted automatically. |

### Hermes behavior relied on

`Basis` distinguishes what this design's runs must observe from what was read in the installed source (`tools/terminal_tool.py`, `tools/process_registry.py`, `tools/process_registry_notifications.py`, Hermes 2026.9.24 / Homebrew 0.22.4). Source-inspected thresholds are implementation facts, not API guarantees.

| Behavior | Basis | Run that observes it |
|---|---|---|
| A heartbeat is injected while the process runs and wakes an idle session without user input | observed | A, B |
| Ticks with no new output emit nothing | source-inspected (`_emit_heartbeat`); observed | B (quiet gaps) |
| Minimum interval 60 s; positive smaller values clamp to 60; scheduler ticks every 5 s | source-inspected (`HEARTBEAT_MIN_SECONDS`, `HEARTBEAT_TICK_SECONDS`) | — |
| A heartbeat carries at most the newest 2,000 characters; `omitted` counts the characters cut | source-inspected (`heartbeat_output_tail`); observed | B (burst) |
| Heartbeats have no lifetime cap; watch-pattern `notify` stops after 8 delivered matches | source-inspected (`WATCH_LIFETIME_MAX_HITS`); observed | B (≥ 12 heartbeats) |
| `heartbeat` implies `notify=true`; the exit notification still follows process exit | source-inspected (`notify_on_complete = True`); observed | B (shutdown) |
| Consecutive queued heartbeats can be joined into one notification | source-inspected (`join_heartbeat_outputs`) | — |
| The first heartbeat carries all output since the process started, including output already read through `process_manage` `action=log` | source-inspected (`arm_heartbeat`); observed | A, B (startup replay) |
| `process_manage` `action=log` slices lines `[offset:offset + limit]` of a 200,000-character rolling buffer; once output exceeds it, older lines drop, `total_lines` stops growing and a retained `offset` at or past `total_lines` returns no lines | source-inspected (`MAX_OUTPUT_CHARS`, `read_log`) | — |
| `persist_on_release=True` exempts the process from agent-lifecycle kill sweeps only | source-inspected (`persist_on_release`) | — |

### Live verification

Both runs follow the environment and probe-member setup of `docs/docs/contributing.md` § Run a CAFleet stream probe steps 1–3, with these settings:

- An isolated database in a fresh short `mktemp -d` directory, set up with plain `cafleet setup`, which installs every backend's assets and, with `hermes` on `PATH`, merges the Hermes `terminal.client_env_passthrough` names; its output must include a `hermes: … terminal.client_env_passthrough …` line other than the skipped line.
- A `hermes chat --cli` Director in a multiplexer pane, registered with `cafleet fleet create --coding-agent hermes`.
- One probe member created with `--coding-agent claude --model haiku`, whose spawn prompt sends `ready` and ends its turn.

Records are sent from a separate operator shell with the probe member's identity: `cafleet message send --from-member-id <probe-id> --to-member-id <director-id> "<body>"`. Each run uses its own fleet. "Idle" means the Director's turn has ended and its composer shows the empty `❯ ` row. A scheduled send waits until the Director is idle.

**Startup replay and T0.** Heartbeats carry output the Director already read in its startup turn: the receiver's `ready` record and the probe member's `ready` message, in one heartbeat or split across the first two, depending on when the probe `ready` arrives relative to the first tick. Every heartbeat whose records are all startup output is startup replay; the turns it admits are expected, and the Director treats its records as replay, with no ACK and no new-message report. T0, the anchor of both schedules, is the end of the turn admitted by the heartbeat that carries the probe member's `ready` message.

Director task prompt for both runs, sent once after the fleet is registered:

> Load the cafleet skill. For this run, `§ Replacement texts > Hermes native Director reception` of `<absolute path of this design doc>` replaces the installed Hermes native Director reception recipe; follow it exactly. Start the receiver, spawn one probe member with `--coding-agent claude --model haiku` whose spawn prompt sends `ready` and then ends its turn, consume its ready, then end your turn. On every later turn, follow the recipe and the Director wake procedure, write one line naming each message ID you consumed and the heartbeat `seq` that woke you (writing "replay" for IDs already consumed), then end your turn.

#### Run A: clean first observation

1. Send the task prompt, wait for the startup turn and every startup replay turn to end, and record T0 and the receiver's `session_id`.
2. Send `heartbeat-probe A1`, `A2`, `A3` at T0 + 30 s, T0 + 150 s and T0 + 300 s. Record message IDs and send timestamps.
3. After the Director consumes `A3` and is idle again, end the run with `cafleet fleet delete <fleet-id>` from the operator shell; record the `closed` record and exit.
4. Export the Director session with `hermes sessions export` into the hidden `probe/` directory beside this design doc; capture the Director pane.

| Classification | Condition |
|---|---|
| Invalid | User input after the task prompt; a record sent while the Director's turn was running; the Director exited or crashed for a cause outside reception. Repeat, up to three runs. |
| Pass | Each of `A1`–`A3` is first observed in a heartbeat notification injected into an idle Director, before any `process_manage` `log`, `poll`, `wait` or `show` in that turn; each is ACKed exactly once in its heartbeat-admitted turn; the startup replay produces no ACK and no new-message report; the same `session_id` serves the whole run with no restart and no `message wait` started as a wake or rearm mechanism; the Director spans at least three separate heartbeat-admitted turns after T0. |
| Fail | A valid run in which a record never reached the Director through a heartbeat while the receiver ran, a record needed user input to be consumed, or the receiver was restarted or supplemented by a `message wait` started as a wake or rearm mechanism. |

#### Run B: extended

Each event starts when its "When" condition holds and the Director is idle, except event 8's dedup prompt. Quiet windows Q1 and Q2 begin when the named heartbeat turn ends, so the heartbeat for the last record before each window falls outside it.

| Event | When | Action | Exercises |
|---|---|---|---|
| 1 | T0 + 30 s | One record, `heartbeat-probe B1` | Repeated delivery |
| 2–5 | 80 s after the previous send | One record each, `B2`…`B5` | Repeated delivery |
| Q1 | From the end of the heartbeat turn carrying `B5`, 4 minutes | No sends | Quiet-tick suppression |
| 6 | End of Q1 | Six records within 10 s, `heartbeat-probe B6a`…`B6f`, each body padded to 800 characters | Burst over the 2,000-character tail |
| 7 | 80 s after event 6's last send | One record, `B7` | Recovery after burst |
| 8 | 80 s after `B7` | One record, `B8`; immediately type the user prompt "Run cafleet message poll <director-id> --json, consume and ACK everything pending, then end your turn." | Dedup of an already-consumed record |
| Q2 | From the end of the heartbeat turn carrying `B8`, 4 minutes | No sends | Quiet-tick suppression |
| 9 | End of Q2 | One record, `B9` | Heartbeats beyond 8 |
| 10–14 | 80 s after the previous send | One record each, `B10`…`B14` | Heartbeats beyond 8 |
| 15 | 80 s after the heartbeat turn carrying `B14` ends | `cafleet fleet delete <fleet-id>` | Shutdown without leaks |

Each padded record is about 1,040 characters of listen output, so event 6's records, whether they arrive in one heartbeat or split across two adjacent ticks, put at least three records (over 3,000 characters) into one notification; a split burst is valid. When joined notifications leave fewer than 12 heartbeat notifications after T0 by event 14, append single records 80 s apart until 12 are delivered, then run event 15 after the last one's heartbeat turn. After shutdown, verify no live `cafleet message listen <director-id>` process (`ps -eo pid,args`) and no `<hermes home>/processes.json` entry for the receiver with a live `pid`. Export and capture as in Run A step 4.

| Classification | Condition |
|---|---|
| Invalid | User input other than event 8's prompt; a send while the Director's turn was running, other than `B8`; `B8` first observed in a heartbeat rather than in the prompted `poll`; a crash outside reception. Repeat, up to three runs. |
| Pass | All hold: ≥ 12 heartbeat notifications after T0 from the same `session_id`; no heartbeat notification and no Director turn within Q1 or Q2; at least one heartbeat carrying event-6 records shows `omitted` > 0, and all six burst records are consumed from `log` reads and ACKed once; `B8` is consumed in the prompted turn and the heartbeat carrying it produces no second ACK and no report of `B8` as new; no restart and no `message wait` started as a wake or rearm mechanism; shutdown shows `closed` with `reason=fleet_deleted`, exit 0, the exit notification treated as history, and no leaked process. |
| Fail | A valid run violating any Pass condition. |

A run type with three invalid runs is `not established` and blocks like `Fail`.

### Live evidence

Filled by the implementer. One row per run attempt; `Basis` is `observed` for every row.

Versions: `Hermes Agent vunknown (2026.9.24)`; `cafleet 0.31.2`.

| Run | Attempt | Date (UTC) | `session_id` | T0 | Message IDs and send times | First observation per record (heartbeat `seq`, time) | ACK times | Heartbeats after T0 | Turns within Q1/Q2 | Shutdown (`closed` reason, exit, leak check) | Basis | Classification |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| A | 1 | 2026-11-04 | `proc_28dd020` | — (never reached) | — (none sent) | — | — | — | — | `fleet delete 1` teardown at ~15:33:40; no `cafleet message listen 1` process remains | observed | Invalid: probe member 2 stayed on Claude Code's folder-trust dialog for the fresh `mktemp -d` directory and never sent `ready`; the Director blocked on a `clarify` question, so T0 required user input (cause outside reception) |
| A | 2 | 2026-11-04 | `proc_8415e2ada441` | 15:36:20 (end of heartbeat seq 1 replay turn) | A1 = 3 @ 15:36:53.744; A2 = 4 @ 15:38:54.983; A3 = 5 @ 15:41:24.381 | A1 seq 2 @ 15:37:05.340; A2 seq 3 @ 15:39:05.811; A3 seq 4 @ 15:42:04.597; each injected into an idle Director before any `log`/`poll`/`wait`/`show` | 3 @ 15:37:21.587; 4 @ 15:39:15.709; 5 @ 15:42:13.496; each once; seq 1 replay turn issued no ACK | 3 (seq 2–4); quiet ticks emitted nothing | — | `closed` `fleet_deleted`, exit 0, exit notification treated as history; no `listen 3` process, `processes.json` `[]` | observed | Pass |
| B | 1 | 2026-11-04 | `proc_c4345d1ee7bf` | 15:48:17 (end of heartbeat seq 1 replay turn) | B1–B5 = 8–12 @ 15:48:51.282, 15:50:16.172, 15:51:42.957, 15:53:07.966, 15:54:32.636; B6a–f = 13–18 @ 15:59:40.151–.361 (800-character bodies); B7 = 19 @ 16:01:04.954; B8 = 20 @ 16:02:30.631, event-8 prompt 16:02:38.0; B9–B14 = 21–26 @ 16:07:25.564, 16:08:50.084, 16:10:16.595, 16:11:40.684, 16:13:06.935, 16:14:31.190 | B1–B5 seq 2–6 @ 15:49:10.124, 15:51:10.557, 15:52:10.765, 15:53:11.017, 15:55:09.482; B6a–f seq 7 @ 16:00:10.519 (`omitted` 4168, all six consumed from the `log` read); B7 seq 8 @ 16:01:10.727; B8 prompted `poll` @ 16:02:37.610, then seq 9 @ 16:03:11.066 as replay; B9–B14 seq 10–15 @ 16:08:09.897, 16:09:10.074, 16:11:10.428, 16:12:10.624, 16:13:10.804, 16:15:11.163 | 8–12 @ 15:49:19.152, 15:51:18.007, 15:52:19.587, 15:53:17.795, 15:55:18.818; 13–18 @ 16:00:39.686–16:00:40.210; 19 @ 16:01:22.946; 20 @ 16:02:48.944 (prompted turn; none at seq 9); 21–26 @ 16:08:22.220, 16:09:28.047, 16:11:20.736, 16:12:21.939, 16:13:19.465, 16:15:19.636; each once | 14 (seq 2–15), same `session_id`, no joined notifications | Q1 (15:55:27–15:59:27): none; Q2 (16:03:22–16:07:22): none | `closed` `fleet_deleted`, exit 0, exit notification treated as history; no `listen 5` process, `processes.json` `[]` | observed | Pass |

Selected outcome: both Pass; `<run date>` = 2026-11-04.

### Replacement texts

Applied only when both runs pass. `<run date>` is the date recorded above.

#### `coding-agents.md` § hermes, Note row

The row beginning "A Hermes Director reads its background `listen` receiver through `process_manage` `action=log` between work steps" becomes, with Applies at unchanged (`[Director wake](../roles/director.md#director-wake)`):

> A Hermes Director keeps one background `listen` receiver started with `heartbeat=60` (§ Hermes native Director reception): Hermes injects a heartbeat notification carrying the receiver's new output, which wakes an idle Director, and the Director reads the running receiver through `process_manage` `action=log` on that turn and between work steps.

#### Hermes native Director reception

The `#### Hermes native Director reception` subsection of `coding-agents.md` is replaced in full:

````markdown
#### Hermes native Director reception

After registration, start one receiver with the `terminal` tool, using the literal Director ID and the `cafleet` path your own shell resolves:

```python
terminal(
    command="<absolute cafleet path> message listen <director-id>",
    background=True,
    notify=True,
    heartbeat=60,
)
```

Omit `--timeout`: the receiver runs until its `closed` record, retirement or failure. Require its `ready` record. Retain the returned process `session_id`, the Director ID, the log line offset already read, incomplete JSONL and unfinished actions, and keep this one receiver across messages, replies, idle periods and turn endings.

Every 60 seconds Hermes injects a heartbeat notification carrying the output the running receiver emitted since the previous one, which wakes an idle Director without user input; a tick with no new output sends nothing. Keep `heartbeat=60`; Hermes raises smaller values to 60. A heartbeat is a batched wake signal: it can carry several records, keeps only the newest 2,000 characters (its `omitted` field counts the rest), can arrive after its records were consumed, and turn admission adds latency beyond the interval.

On every heartbeat turn and between work steps, read the receiver with `process_manage` `action=log`, `offset` advanced past lines already read and repeated until no new lines remain, on the retained `session_id`. Accumulate incomplete JSONL, consume complete records and follow [Director wake](../roles/director.md#director-wake), whose `cafleet message poll <director-id> --json` is the completeness check. Hermes keeps a 200,000-character rolling log; once the receiver's output outgrows it, older lines drop and `total_lines` stops growing. When a read returns a `total_lines` below the retained offset, or returns no lines on a turn whose heartbeat carried output, reconcile through `poll --json` and set the offset to the returned `total_lines`. Deduplicate by message ID: a record already consumed, including the startup output the first heartbeat repeats, is replay, never reported as new or ACKed again. Then end the turn with the same receiver running and no "nothing received" update. Start no `message wait` as a wake or rearm mechanism; a finite `wait` for ordinary-member retry rotation under Director wake step 4 remains available.

Hermes documents `heartbeat` for bounded jobs rather than servers. This receiver is bounded by its fleet-supervision phase: keep it only while delegated work or an authorized user-feedback phase remains, and it ends with `closed` when the fleet is deleted, the Director becomes inactive or the Director retires it.

For a bounded supervision phase or a host policy that requires finite leases, add `--timeout <seconds>` (`1..=1800`) to the same call; after its `closed` `timeout` record, verify termination and start one replacement only while delegated work remains. Add `persist_on_release=True` only when the operator explicitly asks for reception to survive conversation cleanup; it does not survive Hermes host exit and does not guarantee notification ownership after a session change, so reconcile with `poll --json` after any such change.

The exit notification follows the receiver's `closed` record or exit: verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery). After `fleet_deleted`, reconcile the exit output as history and end reception. Retire the receiver with `process_manage`, `action=kill` on the retained `session_id`, and verify that it exited. Preserve the process `session_id`, log offset, incomplete JSONL and unfinished actions across compaction. When the background facility is unavailable or denied, follow [receiver recovery](../roles/director.md#receiver-recovery).

Reception uses process output, heartbeat notifications and broker reads, makes no pane input and leaves `/goal` `gate`/`wait` authority with the host; heartbeat wake needs no `/goal`.
````

#### Hermes Director eligibility row

In both `coding-agents.md` § hermes § Role eligibility and `SPEC.md` § Role eligibility (the `hermes` Director row), the Evidence cell becomes:

> Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Streamed background output: CAFleet stream probe with `terminal` `background=true` read through `process_manage` `action=log`, 2026.9.24, checked 2026-11-03. Idle wake: persistent `listen` receiver with `terminal` `heartbeat=60`, clean first-observation and extended live runs, 2026.9.24, checked `<run date>`.

#### `SPEC.md` §6.7 hermes backend paragraph

The paragraph beginning "**hermes**: Hermes 2026.9.24 ships" is replaced in full:

> **hermes**: Hermes 2026.9.24 ships a user-operated `/goal` command with `status`, `show`, `pause`, `resume`, `clear`, `wait <pid>`, `unwait` and `gate` subcommands, a `terminal` tool whose `background=true` with `notify=true` injects one completion notification into the conversation when the process exits and whose `heartbeat=N` (minimum 60) also injects, every N seconds while the process runs, a notification carrying its output since the previous one, skipping ticks with no new output, and `process_manage`, which polls, reads the log of, waits on or kills a background process by its `session_id`. In the classic REPL, leading-`!` input runs a native shell command locally; the Director authorizes each routed command under its host rules. The hermes Director verdict is `yes` for 2026.9.24 (§ Role eligibility). A Director keeps one persistent background `listen` receiver with `heartbeat=60`, whose heartbeat notifications wake an idle session, and reads it through `process_manage` between work steps; heartbeats are a wake signal, so broker reads remain the completeness check. CAFleet makes no goal lifecycle calls and preserves activation, pause, completion and gate/wait authority.

### Unchanged surfaces

- `SPEC.md` §6.7 Role requirements and `coding-agents.md` § Role requirements, including the sentence that streamed background output establishes no unsolicited wake of an idle session.
- `skills/cafleet/roles/director.md` (Director wake, receiver recovery, the generic `listen --timeout 300` snippet) and `skills/cafleet/reference/cli.md` (`message listen` already accepts an omitted `--timeout`).
- `docs/docs`, including `contributing.md` § Run a CAFleet stream probe.
- Every other backend section and every Rust source file.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Prepare the live environment

- [x] Confirm `hermes --version` prints `Hermes Agent vunknown (2026.9.24)` (or another `v… (2026.9.24)` build) and record it with `cafleet --version` on the § Live evidence `Versions:` line. <!-- completed: 2026-11-04T15:47 -->
- [x] Create the isolated probe database per `docs/docs/contributing.md` § Run a CAFleet stream probe step 1, run plain `cafleet setup` against it, and confirm its output includes the Hermes passthrough line required by § Live verification. <!-- completed: 2026-11-04T15:47 -->
- [x] Create the hidden `probe/` directory beside this design doc for session exports and pane captures. <!-- completed: 2026-11-04T15:47 -->

### Step 2: Run A (clean first observation)

- [x] Start the Director, send the § Live verification task prompt and run § Run A steps 1–4. <!-- completed: 2026-11-04T15:47 -->
- [x] Fill the Run A row of § Live evidence and classify it; repeat an `Invalid` attempt as a new row, up to three attempts. <!-- completed: 2026-11-04T15:47 -->

### Step 3: Run B (extended)

- [x] In a fresh fleet, send the task prompt and execute the § Run B schedule, appending records when fewer than 12 heartbeats were delivered by event 14. <!-- completed: 2026-11-04T16:21 -->
- [x] Run the post-shutdown leak checks (`ps -eo pid,args`, `<hermes home>/processes.json`) and export the session. <!-- completed: 2026-11-04T16:21 -->
- [x] Fill the Run B row of § Live evidence and classify it; repeat an `Invalid` attempt as a new row, up to three attempts. <!-- completed: 2026-11-04T16:21 -->

### Step 4: Outcome gate

- [x] Record the selected outcome and `<run date>` in § Live evidence. <!-- completed: 2026-11-04T16:21 -->
- [x] When either run is `Fail` or `not established`, add the Programmer's paired blocker marker (coordination protocol, role `programmer`) in § Live evidence naming the failed condition and its evidence, send `blocked (paragraph-Specification > Live evidence)` and stop until the user decides; otherwise check this task and continue. <!-- completed: 2026-11-04T16:21 -->

### Step 5: Update `skills/cafleet/reference/coding-agents.md`

- [x] Replace the Hermes Director-wake Note row per § Replacement texts. <!-- completed: 2026-11-04T16:24 -->
- [x] Replace the `#### Hermes native Director reception` subsection per § Replacement texts. <!-- completed: 2026-11-04T16:24 -->
- [x] Replace the Hermes Director Role eligibility Evidence cell per § Replacement texts, substituting `<run date>`. <!-- completed: 2026-11-04T16:24 -->

### Step 6: Update `SPEC.md` §6.7

- [x] Replace the hermes backend paragraph per § Replacement texts. <!-- completed: 2026-11-04T16:25 -->
- [x] Replace the `hermes` Director row's Evidence cell in § Role eligibility per § Replacement texts, substituting `<run date>`. <!-- completed: 2026-11-04T16:25 -->

### Step 7: Verify

- [x] `grep -rn "woken only by the receiver's exit notification\|<absolute cafleet path> message listen <director-id> --timeout 300" SPEC.md skills` returns nothing. <!-- completed: 2026-11-04T16:26 -->
- [x] `grep -rn "heartbeat" docs/docs` returns nothing, and `git diff --stat` lists only `SPEC.md` and `skills/cafleet/reference/coding-agents.md`. <!-- completed: 2026-11-04T16:26 -->
- [x] The Hermes Director Evidence cells in `SPEC.md` and `coding-agents.md` are identical, and the replaced subsection's links (`#director-wake`, `#receiver-recovery`) resolve to existing headings in `skills/cafleet/roles/director.md`. <!-- completed: 2026-11-04T16:26 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-04 | Initial draft |
| 2026-11-04 | Review round 1: startup-replay T0 anchor, quiet windows after the preceding heartbeat turn, split-burst rule, log offset re-anchor, `wait` scope, pinned probe backend and setup, versions line, Hermes-only grep |
| 2026-11-04 | Review round 2: startup replay and T0 defined by heartbeat content rather than `seq`; Live evidence table delimiter row matches its 13 columns |
