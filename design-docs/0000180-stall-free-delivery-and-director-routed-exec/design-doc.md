# Stall-free delivery and Director-routed exec

**Status**: Approved
**Progress**: 33/34 tasks complete
**Last Updated**: 2026-10-10

## Overview

Resolve GitHub issues #395, #438 and #443 so that a fleet keeps moving when a pane is busy, when a prompt is open, and when a member's harness denies shell commands. The broker holds every pane keystroke until the target pane is at rest and the monitor loop delivers it afterwards; the loop is started by `cafleet fleet create` instead of by a member. One Director command, `cafleet member exec`, runs a member's requested command to completion, and claude members are spawned with the broker commands they need already allowed.

## Success Criteria

- [x] A `cafleet message send` to a pane that shows a permission prompt or is mid-turn sends no keystroke; the preview lands within one loop tick after the pane returns to rest, with no user input (#438, #443).
- [x] A member that sends `ready` and keeps working receives its first assignment automatically when it goes idle; the Director sends that assignment in the same turn it receives `ready` (#395).
- [x] A held message is keystroked exactly once, whether the sender, the loop, or both evaluate it, and an ACKed message is never keystroked.
- [x] A hold older than `CAFLEET_DELIVERY_HOLD_TIMEOUT` is delivered as a forced delivery, carrying the broker note when the pane was `awaiting_user` and the resume clause when it was `working`.
- [x] `cafleet fleet create` returns only after the fleet's monitor loop is live, and no member role file instructs a member to launch the loop.
- [x] `cafleet member exec` runs a command in a member's pane, records its exit status, notifies the Director on completion, and resumes the member, with no `member ping` and no manual pane capture.
- [x] `cafleet member prompt --shell` no longer parses.
- [ ] A claude member spawned while the user's `settings.json` holds no `Bash(cafleet ...)` allow rule sends `ready`, polls, and ACKs.
- [x] `cafleet doctor` exits 1 and names the file and rule when a claude `deny` or `ask` rule matches a member broker command.
- [x] A spawned member that sends no message within 180 s produces a broker notice in the Director's inbox.
- [x] The migration chain is contiguous 1..9 with head V9; `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint` and `mise //admin:test` pass.
- [ ] The user has run the manual live checklist in § S11 once and every item passed.

---

## Background

| Issue | Failure | Root cause in the current code |
|---|---|---|
| #395 | The Director ACKs `ready`, sees the member `working`, defers the first assignment, and nothing ever re-opens the Director's turn. | `message send` persists and keystrokes in one step, so the only way to protect a busy pane is for the Director to hold the whole send in its own notes. No wake exists for a deferred send. |
| #438 | A member's message arrives while the Director shows a permission prompt; the preview's leading `Escape` rejects the tool call and halts the Director. | Every keystroke path is unconditionally `Esc`-first. On Claude Code, `Escape` on a permission prompt is a rejection. |
| #443 | The user's allow list lost its `Bash(...)` rules; members could not send, poll, or ACK, the monitor could not scan or start its loop, and the fleet stalled silently. | Member broker commands depend on the user's `settings.json`. The loop is launched by the monitor member's own shell tool. `member prompt --shell` followed by `member ping` races, and neither reports completion. |

Facts that shape the design:

- The binary has no pane classifier. `awaiting_user`, `working` and `finished` are judged only by an agent reading the capture cues in `skills/cafleet/reference/coding-agents.md`.
- `messages` carries no notification state, and the only long-lived cafleet process is the `cafleet monitor` loop (5 s tick), which keystrokes only the monitor member's pane.
- Claude members are spawned as `claude --permission-mode dontAsk --name <name> <prompt>`. Codex members get their `cafleet` allowance from `~/.codex/rules/cafleet.rules` and opencode members from the `cafleet` agent preset; `cafleet setup` installs both and `cafleet doctor` already reports their freshness.
- Claude Code documents `--allowedTools "<rule>" "<rule>"` as session allow rules and shows it combined with `--permission-mode dontAsk`; a `deny` rule at any settings level overrides it ([CLI reference](https://code.claude.com/docs/en/cli-reference), [permissions](https://code.claude.com/docs/en/permissions)).
- A `!` command typed at a coding agent's prompt runs without a permission check. Claude Code answers its output with a model turn unless `respondToBashCommands` is `false` ([settings reference](https://code.claude.com/docs/en/settings-reference)).

### Issue coverage

| Source | Criterion or proposal | Covered by | Verified by |
|---|---|---|---|
| #395 | `ready` arrives while working; later idle triggers dispatch without user input | § S3 gate, § S4 delivery pass | § S11 *Loop*: held-then-fired case; checklist 3 |
| #395 | Completion arrives before or after the Director starts waiting, including the notification-before-idle race | § S3: the preview is owed from the insert and re-evaluated every tick, so no idle notification is needed | § S11 *Loop*: held-then-fired case |
| #395 | ACKed or duplicate notifications cause neither lost nor duplicate assignments | § S3 item stamp and pane claim | § S11 *Delivery step*, *Pane claim* |
| #395 | Normal idle members receive no unnecessary polling or interruptions | § S4 tick step 4: only a pane with owed work is captured | § S11 *Loop*: idle-tick case |
| #438 | A pending prompt stays open and the message is consumed afterwards | § S3 gate | § S11 *Delivery step*; checklist 2 |
| #438 | Scope the change to the Director pane first | Not adopted: one rule for every pane is the smaller design | — |
| #438 comment | Broker note that the rejection did not come from the user | § S3 *Preview wording*, on a forced delivery only | § S11 *Delivery step*; checklist 5 |
| #443 criterion 1 | A fleet whose member panes run only read-only commands completes an execute workflow | Out of scope as written (user decision: no second transport). `cafleet message send` stays the only channel; § S6 makes it allowed at spawn and § S7 reports what can still block it | checklist 1 and 6 |
| #443 proposals 2, 4 | A request and report transport that needs no member shell tool | Out of scope, same decision | — |
| #443 criterion 2 | Members run what their harness allows and route the rest | § S5 *Member side* | § S10 skill edits |
| #443 criterion 3 | One Director command runs a command to completion and resumes the member, with no capture and no race | § S5 | § S11 *Exec*; checklist 4 |
| #443 criterion 4 | The Director is notified of a request, a report, and a finished command | Requests and reports are ordinary messages under § S3; § S8 exec notice | § S11 *Exec* |
| #443 criterion 5 | A member that cannot reach the Director produces a visible failure | § S7 before spawn; § S8 watchdog for a member that never speaks; the monitor member's stall report afterwards | § S11 *Doctor*, *Loop* |
| #443 criterion 6 | The role files describe the Director-routed path as the standard one | § S10 | Step 2 |
| #443 proposal 5 | Monitoring must not depend on the monitor member's shell tool, or must fail loudly | § S4 loop start; § S8 denied-command event | § S11 *Loop start* |

---

## Specification

### S1. Decisions

| # | Decision | Rationale |
|---|---|---|
| 1 | A new in-binary classifier decides, from a pane capture, whether a keystroke may be sent. | Holding must not depend on an agent's judgment or on a member's shell permission. |
| 2 | A broker delivery (a preview, an exec dispatch, an exec resume) fires only into a pane classified `finished`. | Covers the rejected prompt (#438) and the interrupted Director turn (#443) with one rule. Exceptions are listed below the table. |
| 3 | A hold older than a configurable timeout becomes a forced delivery. | Bounds every classifier miss to a delay: the worst case is today's behavior, later. |
| 4 | The sending command tries delivery once; the monitor loop retries held work every tick. Both call one function. | Delivery to a pane at rest stays immediate, and held work has a guaranteed wake. |
| 5 | `cafleet fleet create` starts the loop as a detached process, and a command that leaves work owed restarts a dead loop. | The actor that guarantees delivery must not depend on a member's shell tool. |
| 6 | #395 needs no second member signal. The Director dispatches on `ready`; the broker holds the keystroke. | The Director-side capture gate and deferred sends leave the skill because the binary enforces them. |
| 7 | `cafleet member exec` replaces `cafleet member prompt --shell`. | One Director command, no follow-up ping, completion recorded by the command itself. |
| 8 | A member reaches the Director with `cafleet message send`, as today. Claude members receive the broker commands as spawn-time allow rules. | No second transport. The spawn flags remove the dependency on the user's `settings.json`. |
| 9 | `cafleet doctor` gains a gating member-permissions section, and the loop reports a member that never speaks. | A `deny` rule still overrides the spawn flags; both failures must be visible. |
| 10 | The preview carries both candidate lines, each only on a forced delivery and only for the state the broker observed. | See § S3 *Preview wording*. |

Keystrokes that reach a pane that is not `finished`:

| Keystroke | Rule | Why |
|---|---|---|
| Forced delivery | Decision 3 | The bound on a hold |
| `member ping` on an unclassified pane | § S3 *Command behavior* | A quiet pane the classifier cannot name is the stalled member a ping exists for |
| `member prompt` | Not gated | The Director's deliberate direct turn |
| Monitor wake | Not gated | It re-engages a monitor member stuck mid-turn and already ends with the resume sentence; the monitor's work is an idempotent re-scan, and its spawn posture shows no user prompt |

### S2. Pane-state classifier

New module `cafleet/src/pane_state.rs`:

```rust
pub enum PaneState { AwaitingUser, Working, Finished, Unclassified }

pub fn classify(coding_agent: &str, content: &str) -> PaneState
```

`content` is the ANSI-stripped tail of the pane, `DELIVERY_CAPTURE_LINES = 40` lines, taken with the existing `capture_pane`. The same function serves tmux and herdr because it reads captured text only; herdr's native `agent_status` is not consulted.

Each coding agent contributes one cue table: three lists of regular expressions, each matched line by line. Evaluation order is fixed and encodes the existing tie-breaks:

1. Any `awaiting_user` cue matches in its region → `AwaitingUser`.
2. Any `working` cue matches in its region → `Working`.
3. Any `finished` cue matches in its region → `Finished`.
4. Otherwise → `Unclassified`.

**Regions.** A cue reads only the bottom of the pane, where the coding agent draws its live prompt box, activity line and composer. Blank lines are dropped first; a region is the last N remaining lines.

| Cue | Region | Reason for the size |
|---|---|---|
| `awaiting_user` | Last `AWAITING_REGION_LINES = 20` lines | A selection list with descriptions is tall |
| `working` | Last `WORKING_REGION_LINES = 10` lines | The activity line sits directly above the composer |
| `finished` | Last `FINISHED_REGION_LINES = 6` lines | The composer and its status lines only |

The sizes are asymmetric on purpose. A false `awaiting_user` or `working` only delays a delivery, bounded by the timeout. A false `finished` sends the `Escape` this design exists to withhold, so that cue reads the smallest region and is evaluated last: an echoed user turn higher in the transcript does not match, and a prompt box the `awaiting_user` cue does not recognise leaves no composer in the last six lines, so the pane is `Unclassified` and holds. Cue text that an agent prints as the last lines of a reply can still read as `working`; that costs a delay and nothing else.

| Coding agent | `awaiting_user` cue | `working` cue | `finished` cue |
|---|---|---|---|
| `claude` | A numbered option under the selection cursor: `^[\s│]*❯\s+\d+\.\s` | `esc to interrupt` | The composer prompt line: `^[\s│]*[>❯](\s.*)?$` |
| `codex` | A numbered option under the cursor, `^\s*›\s+\d+\.\s`, or a `\[y/n\]` approval line | `esc to interrupt` | The composer prompt line: `^\s*[›▌](\s.*)?$` |
| `opencode` | The permission popup title: `Permission required` | `esc\s+(to\s+)?interrupt` | The prompt box line: `^\s*┃(\s.*)?$` |

Every pattern above is derived from the capture cues documented for that backend and is implemented as written. A pattern that misreads a live pane is corrected when the manual live checklist of § S11 exposes it; the hold timeout bounds the cost of a miss in the meantime.

`finished` means the composer is visible with no prompt box and no active-work cue. Text already typed into the composer does not change the result; a keystroke then appends to it, as it does today.

### S3. Held delivery

#### Data model — migration V9

```sql
-- cafleet/migrations/V9__held_delivery_and_member_execs.sql
ALTER TABLE messages ADD COLUMN notified_at TEXT;
UPDATE messages SET notified_at = created_at;

ALTER TABLE member_placements ADD COLUMN keystroke_at TEXT;
ALTER TABLE member_placements ADD COLUMN forced_at TEXT;
ALTER TABLE member_placements ADD COLUMN silence_notice_at TEXT;

CREATE TABLE member_execs (
    exec_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    command TEXT NOT NULL,
    created_at TEXT NOT NULL,
    dispatched_at TEXT,
    started_at TEXT,
    pid INTEGER,
    finished_at TEXT,
    exit_code INTEGER,
    resumed_at TEXT
);
```

| Column | Meaning |
|---|---|
| `messages.notified_at` | `NULL` while the preview is owed; the UTC timestamp of the keystroke once it landed. The backfill marks every existing row as settled. The message JSON envelope is unchanged. |
| `member_placements.keystroke_at` | The last time a cafleet process claimed this pane for a keystroke (§ *Pane claim*). |
| `member_placements.forced_at` | The time of the pane's last forced delivery; `NULL` until one happens. |
| `member_placements.silence_notice_at` | When the ready watchdog reported this member (§ S8); `NULL` until then. |
| `member_execs` | One row per `cafleet member exec` (§ S5). |

A **pending preview** is a row with `type = 'unicast'`, `status_state = 'input_required'` and `notified_at IS NULL` whose recipient is active and owns a pane. An ACK settles a preview without a keystroke, because the row leaves `input_required`. A CLI self-send inserts with `notified_at = created_at`, so it attempts nothing, as today.

#### Terms

| Term | Meaning |
|---|---|
| Ordinary delivery | A delivery into a pane classified `finished` |
| Forced delivery | A delivery into any other pane, made because the hold age reached the timeout |
| Base payload | For a preview, the existing two lines: `[cafleet msg <id> from <sender> <ts>]` and the truncated text |
| Owed work | For a fleet: a pending preview, or a `member_execs` row with `resumed_at` `NULL`, for any of its members |

Both kinds use today's keystroke shape: `Escape`, settle, payload, `Enter`. The `Escape` stays on an ordinary delivery because a prompt can open between the capture and the keystroke; in that window the keystroke rejects the prompt instead of confirming it.

#### The pane delivery step

New module `cafleet/src/delivery.rs`. `deliver_pane` performs at most one keystroke into one member's pane:

1. **Exec in flight.** If the pane has a `member_execs` row with `dispatched_at` set and `finished_at` `NULL`, hold. No timeout applies while a command is running.
2. **Select one item**, in this priority: a finished exec not yet resumed (§ S5), else the oldest queued exec, else the oldest pending preview. No item → nothing is owed.
3. **Capture and classify** the pane (§ S2). A capture error holds.
4. **Exec resume by observation** (§ S5), when the item is a finished exec.
5. **Decide** per the gate table below.
6. **Fire**: claim the pane, stamp the item (`notified_at`, `dispatched_at` or `resumed_at`) and, on a forced delivery, `forced_at`; then send the keystroke. A failed claim holds. A failed keystroke clears the item stamp, so the item stays owed.

| Pane state | Hold age below the timeout | Hold age at or above the timeout |
|---|---|---|
| `finished` | Ordinary delivery | Ordinary delivery |
| `awaiting_user` | Hold | Forced delivery; a preview adds the broker note |
| `working` | Hold | Forced delivery; a preview adds the resume clause |
| unclassified | Hold | Forced delivery; base payload only |

**Hold age** is `now − max(item timestamp, finished_at of the pane's latest exec, forced_at)`. The item timestamp is `created_at` for a preview or a queued exec and `finished_at` for a resume. The two pane-level terms restart the clock for everything queued behind an event: a long command does not immediately force the keystrokes that waited for it, and several overdue items do not force back to back into a prompt the recipient has just re-raised.

**Timeout.** `CAFLEET_DELIVERY_HOLD_TIMEOUT`, a non-negative integer of seconds, default `300`. `0` disables forcing: a hold then lasts until the pane is at rest. It is parsed with the existing non-negative-integer rule and its error string.

**Pane claim.** Immediately before a keystroke, the process runs one statement:

```sql
UPDATE member_placements SET keystroke_at = :now
WHERE member_id = :member_id
  AND (keystroke_at IS NULL OR keystroke_at <= :now_minus_spacing);
```

with `KEYSTROKE_SPACING_SECONDS = 3`. One row changed means the claim is held; zero rows means another cafleet process is typing into that pane, and this one holds. The window covers the `Esc` settle and submit delays (1.1 s), so two processes never interleave their keystrokes and the loser's capture cannot mistake a half-typed pane for one at rest. `member ping`, `member prompt` and the monitor wake take the same claim before typing.

#### Code seam

The broker persists; the caller delivers.

```rust
// broker: persistence only, no notifier parameter
pub fn send_message(conn, from_member_id, to, text) -> Result<MessageRecord, CafleetError>
pub fn broadcast_message(conn, from_member_id, text) -> Result<BroadcastRows, CafleetError>
// BroadcastRows { summary: MessageRecord, deliveries: Vec<(member_id, message_id)> }

// delivery
pub trait PaneIo {
    fn capture_pane(&self, pane_id: &str, lines: i64) -> Result<String, String>;
    fn send_inline_preview(&self, pane_id: &str, message_id: i64, sender_id: i64,
                           ts: &str, text: &str, note: Option<&str>) -> Result<(), String>;
    fn send_prompt(&self, pane_id: &str, text: &str) -> Result<(), String>;
}

pub fn deliver_pane(conn: &mut Connection, io: &dyn PaneIo, settings: &Settings,
                    member_id: i64, now: DateTime<Utc>) -> Result<PaneOutcome, CafleetError>

pub enum PaneOutcome { Fired { item: Item, forced: bool }, Held(HoldReason), Idle }
pub enum Item { Preview { message_id: i64 }, ExecDispatch { exec_id: i64 }, ExecResume { exec_id: i64 } }
pub enum HoldReason { ExecRunning, ResumeGrace, AwaitingUser, Working, Unclassified,
                      CaptureFailed(String), Busy, KeystrokeFailed(String) }
```

`PaneIo` is implemented for every `Multiplexer`; tests use a fake. It replaces `InlinePreviewSender`, `RuntimeNotifier` and `NotificationAttempt`, which are deleted. A caller that cannot resolve a multiplexer skips `deliver_pane` and leaves the row owed.

| Output field | Computed as |
|---|---|
| `message send` → `notification_sent` | `true` when the outcome is `Fired` with `Item::Preview` carrying this message's id |
| `message broadcast` → `delivered` | The number of recipients whose outcome is `Fired` with `Item::Preview` carrying that recipient's delivery id |

#### Preview wording

| Delivery | Payload |
|---|---|
| Ordinary, or forced on an unclassified pane | The base payload |
| Forced on `working` | The base payload, then `[cafleet] Resume your work if something was still running.` |
| Forced on `awaiting_user` | The base payload, then `[cafleet] This delivery's Escape dismissed a pending prompt in your pane; that rejection did not come from the user. Re-issue the tool call if you still need it.` |

The design adopts both lines, each conditional. An ordinary preview lands in a pane at rest, where nothing was interrupted, so an unconditional resume clause would tell every recipient to resume work that was never stopped. The resume clause is accurate exactly when the broker interrupted a turn, and it reuses the sentence the monitor wake already carries. The broker note is the only wording that lets an agent tell a broker-caused rejection from a user's: it is emitted only when the broker observed a pending prompt immediately before its own `Escape`, so its absence still means the user rejected the call. Re-issuing the tool call raises the prompt again, so the user's decision is preserved.

Typing a forced delivery into a `working` claude pane without the `Escape`, so that Claude Code queues it instead of interrupting the turn, was considered and not adopted: without the safeguard, a prompt that opens between the capture and the keystroke would receive the payload's `Enter` as a confirmation.

#### Command behavior

| Command | Change |
|---|---|
| `message send` | Persist, run `deliver_pane` for the recipient, then ensure the loop when the fleet has owed work (§ S4). A held preview exits 0. `notification_sent` follows the table above. Text output is unchanged. |
| `message broadcast` | Persist all rows, run `deliver_pane` once per recipient, then ensure the loop when the fleet has owed work. `delivered` follows the table above; the other previews are held. |
| `POST /api/messages/send` | Persist, and run `deliver_pane` when the server process can resolve a multiplexer. The route does not start the loop, because the server may run outside the multiplexer session the loop needs. The response is unchanged. |
| `member ping` | Check for an exec in flight, then classify the target, then take the pane claim. Fire on `finished` or unclassified; skip on an exec in flight, `awaiting_user`, `working`, or a failed pane claim. A skip exits 0. |
| `member prompt` | Plain form only (§ S5). It takes the pane claim and fails with `member <member_id>'s pane is receiving another keystroke; retry in a few seconds.` when the claim is refused. |
| Monitor wake | Unchanged keystroke. It takes the pane claim; a refused claim leaves the wake due for the next tick. |

The `message send` partial-failure exit (`Message <id> was persisted, but pane notification failed: ...`) is removed with its no-resend procedure: a keystroke failure now leaves the preview owed and the loop retries it.

`member ping` output gains a `reason` key after `skipped`: `null` when the keystroke was sent, otherwise one of `pending_placement`, `awaiting_user`, `working`, `exec_running`, `busy`. Text for a gated skip: `Member <name> (<pane_id>) is <reason> — ping skipped.`

### S4. Monitor loop

#### Start and liveness

`ensure_monitor_loop(conn, settings, fleet_id)` in `cafleet/src/runtime/`:

1. Return when `broker::monitor_is_live` is true.
2. Spawn `cafleet monitor <fleet-id>`, with no flags, from the current executable: stdin null, stdout and stderr appended to `<database directory>/monitor-<fleet_id>.log`, in its own process group (`CommandExt::process_group(0)`).
3. Poll `monitor_is_live` every 100 ms for up to 5 s. On timeout return `monitor loop for fleet <fleet_id> did not start; see <log path>`.

Two racing callers are safe: the existing single-instance claim lets one loop win and the other exits.

| Caller | When | On failure |
|---|---|---|
| `fleet create` | After the bootstrap transaction commits | Compensate, then exit 1 with the error (below) |
| `message send` | After the delivery attempt, when the fleet has owed work | Exit 1: `Message <id> was persisted, but <error>. Do not resend this message; run 'cafleet doctor'.` |
| `message broadcast` | After the delivery attempts, when the fleet has owed work | Exit 1: `Broadcast <summary id> was persisted, but <error>. Do not resend it; run 'cafleet doctor'.` |
| `member exec` | After the delivery attempt; the new exec is itself owed work | Exit 1: `Exec <id> was queued, but <error>` |

The condition is the fleet's owed work (§ S3 *Terms*), not the caller's own row, so a command also restarts a dead loop for items held earlier. A command that leaves nothing owed never calls `ensure_monitor_loop`: a self-send, a send to a member without a pane, and a send or broadcast whose every preview was keystroked exit 0 without a loop and without a multiplexer, as today.

**`fleet create` keeps one failure contract.** When the loop does not start, the command kills the monitor pane it spawned, soft-deletes the fleet it committed (`broker::delete_fleet`), prints no ids, and exits 1 with the loop-start error, appending any cleanup diagnostic in the existing form. The Director reads the named log, fixes the cause, and runs `fleet create` again. On success `--json` gains a trailing `monitor_loop` object, `{"pid": <int>}`, and the text output gains the line `monitor loop: pid <pid>`.

**Tick and wake interval.** `cafleet monitor` resolves each value in this order, and `claim_monitor_runtime` takes both as optional so that an absent flag keeps the stored value:

| Value | Precedence |
|---|---|
| Wake interval | `--interval` → the fleet's stored `wake_interval_seconds` when a runtime row exists → `CAFLEET_MONITOR_WAKE_INTERVAL` → `600` |
| Tick | `--tick` → the stored `tick_seconds` when a runtime row exists → `5` |

A restart therefore preserves a value set through `PATCH /api/monitor`. `fleet create` gains no flags for either; a new fleet starts from the environment default and is adjusted through the WebUI or by restarting the loop in the foreground with flags.

**A loop that cannot reach the multiplexer must not look live.** A loop restarted by a member's `message send` inherits that member's process environment, which can be a sandbox without access to the multiplexer socket.

- Before claiming the runtime row, the loop runs one `list_pane_ids`. A failure exits 1 without a claim, so `ensure_monitor_loop` times out and the caller reports the loop-start error.
- A running loop counts consecutive ticks whose `list_pane_ids` fails and exits after `MUX_FAILURE_LIMIT = 3`, clearing its runtime row, so the next caller starts a replacement. A success resets the count.

The loop registers `SIGHUP` to a no-op handler so a closing terminal does not stop it. The foreground form `cafleet monitor <fleet-id> [--tick N] [--interval N]` still fails with `monitor already running for fleet <fleet_id>` when a loop is live.

#### Tick

`monitor_tick` becomes: heartbeat → fleet liveness → **delivery pass** → the existing wake schedule. The delivery pass runs on every tick, including when the wake interval is `0`.

1. Read the live pane set once (`list_pane_ids`) and reuse it for the wake. If the root Director's pane is not in it, print `<iso> director pane <pane_id> is gone; stopping` and stop cleanly. A fleet without a Director pane has no one to facilitate it, and the check keeps a detached loop from outliving its session.
2. Close lost execs (§ S5).
3. Run the ready watchdog (§ S8).
4. For every active member of the fleet, the Director included, whose pane is live and that has an exec row not yet closed or a pending preview, run `deliver_pane`. A member with nothing owed is neither captured nor keystroked. A per-pane error is echoed and never aborts the tick.
5. Echo one line per keystroke: `<iso> tick -> preview msg <id> member <member_id>`, `forced preview ...`, `dispatch exec <id> member <member_id>` or `resume exec <id> member <member_id>`.

#### Monitor member

The monitor member no longer launches, confirms, or relaunches the loop. Its startup is `ready`, then end the turn. The `monitor live` and `monitor restarted` signals are removed; the Director's gate for the first ordinary `member create` is the monitor member's `ready`, backed by the unchanged CLI monitor-first guard.

Launching the loop was the only consumer of the `{bg_run}` and `{bg_stop}` runtime bindings and of the *Worked resolution* subsection in `coding-agents.md`. Both are removed: a backend section then has five runtime bindings and five subsections.

Teardown order is unchanged (monitor member first). The loop stops on its next tick after `cafleet fleet delete`, which already clears the runtime row.

### S5. `cafleet member exec`

```
cafleet member exec MEMBER_ID (COMMAND | --file PATH) [--wait] [--json]
cafleet member exec-run EXEC_ID
```

| Argument | Notes |
|---|---|
| positional `MEMBER_ID` | An active member with a pane. The fleet's root Director is rejected with exit 1: `cannot exec in the Director's own pane`. A pending placement fails like `member prompt`. |
| positional `COMMAND` / `--file PATH` | Exactly one. `--file -` reads stdin. The body may span lines and is stored verbatim; an empty body exits 2 with `command may not be empty.` |
| `--wait` | Block until the exec closes, polling the row every second, then print its exit status. |

`member exec` inserts a `member_execs` row, runs `deliver_pane` for the member, and ensures the loop. The dispatch keystroke is the fixed line `! cafleet member exec-run <exec_id>`, so no part of the command passes through the keystroke and quoting cannot corrupt it.

Output: `Queued exec <exec_id> for member <name> (<pane_id>): dispatched.` or `...: held.` JSON keys, in order: `exec_id`, `member_id`, `pane_id`, `dispatched`. With `--wait` the command additionally prints `exec <exec_id> exited <code> after <n> s.` and exits 0, or prints the lost-exec text and exits 1.

`member exec-run` is the internal half, typed into the member's pane by cafleet:

1. Claim the row: `UPDATE member_execs SET started_at = :now, pid = :pid WHERE exec_id = :id AND dispatched_at IS NOT NULL AND started_at IS NULL AND finished_at IS NULL`. Zero rows → exit 1 with `exec <exec_id> is not runnable`.
2. Run `sh -c <command>` with inherited stdio in the pane's working directory, so the output becomes the `!` command's output in the member's context.
3. Record `finished_at` and `exit_code` (128 + signal number for a signal death).
4. Post a broker notice to the Director (§ S8) and run `deliver_pane` for the Director.
5. Print `[cafleet] exec <exec_id> exited <code>` and exit with the command's code.

**Resuming the member is decided by observation**, the same way on every coding agent. A harness either answers `!` output with a turn of its own (Claude Code by default) or only stages it (Claude Code with `respondToBashCommands: false`, and the documented behavior of the other backends). For a finished exec, `deliver_pane` step 4 applies:

| Observation | Action |
|---|---|
| The pane is `working` | The harness answered. Stamp `resumed_at`; no keystroke. |
| Not `working`, and less than `EXEC_RESUME_GRACE_SECONDS = 10` since `finished_at` | Hold (`ResumeGrace`). |
| Not `working`, and the grace has passed | Deliver `[cafleet] exec <exec_id> finished with exit <code>. Continue your work using its output above.` through the gate table, then stamp `resumed_at`. |

A member that answered and returned to rest inside the grace without being observed receives one redundant resume line; that costs a short turn and loses nothing.

Exec states, derived from the timestamps:

| State | Condition | Pane effect |
|---|---|---|
| queued | `dispatched_at` is `NULL` | The dispatch is the pane's next item |
| dispatched | `dispatched_at` set, `started_at` `NULL` | Every other item holds |
| running | `started_at` set, `finished_at` `NULL` | Every other item holds |
| finished | `finished_at` set, `resumed_at` `NULL` | The resume observation is the pane's next item |
| closed | `resumed_at` set | None |

**Lost execs.** The loop closes an exec (`finished_at = resumed_at = now`, `exit_code` `NULL`) and posts a notice when it is dispatched but not started after `EXEC_START_GRACE_SECONDS = 30`, or running with a `pid` that is no longer alive.

**Removal.** `cafleet member prompt` loses `--shell` and its JSON output becomes `{member_id, pane_id, text}`; the multiplexer's `send_prompt` loses its `shell` parameter and the exec dispatch and resume keystrokes call the plain form. Every mention of the shell form and of the `prompt --shell → ping → ack` sequence is deleted.

**Permissions.** `member exec` carries an operator-controlled body. The Director runs it under its own harness mode, which decides when the user is asked.

| Surface | Rule |
|---|---|
| Generated `permissions.allow` set (`cli-options.md` § *`permissions.allow` coverage*) | `member exec` and `member exec-run` are excluded, alongside `member prompt`: one carries a free-form command and the other executes a stored one. |
| `presets/codex/cafleet.rules` | A new `prompt` rule for `["cafleet", "member", "exec"]`, justified as `cafleet member exec runs an arbitrary shell command in a member pane`. The existing `["cafleet", "member", "prompt"]` rule stays for the plain form; its justification drops "or shell commands". |
| `presets/opencode/cafleet.md` | A new `deny` entry for `cafleet member exec *`, placed after the `cafleet *` allow. The preset has no `member prompt` entry today; it configures members, which never dispatch. |

**Member side.** A member asks with `cafleet message send`: `Need to run: <command>. My harness denied it.` For a command that is hard to quote, the member writes it to a file under its `BASE` and names the path; the Director passes that path to `--file`. The member then ends its turn. Routing is the standard path for every command the member's harness does not run, not a rare fallback: a member runs what its harness allows and routes the rest.

### S6. Spawn-time allow rules

The claude spawn argv becomes:

```
claude --permission-mode dontAsk --allowedTools <rule>... --name <member-name> [--model <m>] [--effort <e>] <prompt>
```

| Member | Rules |
|---|---|
| Every claude member | `Bash(cafleet message *)` |
| The monitor member, additionally | `Bash(cafleet monitor scan *)`, `Bash(cafleet member ping *)` |

`--allowedTools` takes several values, so it sits before `--name`, which ends its value list and keeps the prompt positional. `build_spawn_argv` gains a `monitor: bool` parameter; codex and opencode ignore it.

Codex and opencode are unchanged: their `cafleet` allowance comes from the rules file and the agent preset that `cafleet setup` installs.

What the flags do not cover, stated in the docs:

- A `deny` or `ask` rule at any settings level, including managed policy, still blocks the command. § S7 reports it before spawn.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code ignore `--allowedTools`. § S7 reports it.
- A blocking `PreToolUse` hook stops a tool call before permission rules are evaluated. `doctor` cannot evaluate a hook.
- A `deny` rule or hook added after the member is spawned is not seen by `doctor`, and the ready watchdog does not report a member that has already spoken. The runtime signal is the monitor member: it pings a member that stays quiet and reports one that is unchanged after the ping, using commands that are themselves spawn-allowed. A mid-run loss of *allow* rules, the case observed in #443, no longer affects a member, because its rules were fixed at spawn.
- Work commands (`mise`, `git commit`, package managers) stay under the user's own rules; a denied one is routed through `member exec`.

### S7. `cafleet doctor` — member permissions

A fourth section, after *coding agents*. It runs for `claude` when the claude assets are installed, and reads each file that exists:

| File | Location |
|---|---|
| User settings | `<claude config dir>/settings.json`, using the existing config-dir resolution (`CLAUDE_CONFIG_DIR`) |
| Project settings | `<cwd>/.claude/settings.json` and `<cwd>/.claude/settings.local.json` |
| Managed settings | `/Library/Application Support/ClaudeCode/managed-settings.json` on macOS, `/etc/claude-code/managed-settings.json` on Linux |

Probe commands:

```
cafleet message send --from-member-id 1 --to-member-id 2 x
cafleet message broadcast --from-member-id 1 x
cafleet message poll 1
cafleet message ack 1
cafleet message show 1
cafleet monitor scan 1
cafleet member ping 1
```

A rule matches a probe when it is `Bash`, or `Bash(<pattern>)` whose pattern matches the probe. `*` matches any text; a trailing ` *` or the legacy `:*` also matches the bare command.

| Finding | Condition |
|---|---|
| Blocking rule | A `permissions.deny` or `permissions.ask` rule in any file matches a probe. |
| Managed rules only | The managed file sets `allowManagedPermissionRulesOnly` to `true` and its own `permissions.allow` does not match every probe. |
| Unreadable file | A settings file exists but does not parse as JSON. A missing file is expected and is not a finding. |

```
✓ member permissions
  claude: no setting blocks the member broker commands

✗ member permissions
  claude: ~/.claude/settings.json permissions.deny "Bash(cafleet *)" matches "cafleet message send --from-member-id 1 --to-member-id 2 x"
  claude: <file> allowManagedPermissionRulesOnly is true and its permissions.allow does not match "<command>"
  claude: <file> is not valid JSON
```

The second and third lines are the managed-rules-only and unreadable-file findings.

When the claude assets are not installed the row reads `claude: – not installed` and never counts. JSON gains `member_permissions` between `coding_agents` and `issues`: `{"ok": <bool>, "findings": [{"coding_agent", "file", "list", "rule", "command"}]}`. `list` is `deny`, `ask` or `allowManagedPermissionRulesOnly`; `rule` is `null` for the managed-rules-only finding, whose `command` is the first unmatched probe; all three are `null` for an unreadable file. Each finding adds one to `issues`, so the existing exit-1 rule gates the spawn protocol.

### S8. No silent stalls

**Broker notices.** `broker::post_notice(conn, fleet_id, text)` inserts a `unicast` row whose `owner_member_id`, `from_member_id` and `to_member_id` are all the fleet's root Director, with `notified_at` `NULL`. It is delivered, polled and ACKed like any message. Every notice body starts with `[cafleet] `.

| Notice | Text |
|---|---|
| Exec finished | `[cafleet] exec <id> on member <member_id> (<name>) exited <code> after <n> s.` |
| Exec never started | `[cafleet] exec <id> on member <member_id> (<name>) did not start within 30 s of dispatch. Inspect the pane with cafleet member capture <member_id> and run it again if still needed.` |
| Exec lost | `[cafleet] exec <id> on member <member_id> (<name>) ended without reporting an exit status. Inspect the pane with cafleet member capture <member_id>.` |
| Silent member | `[cafleet] member <member_id> (<name>) has sent no message <n> s after spawn. Its broker commands may be denied: inspect it with cafleet member capture <member_id> and run cafleet doctor.` |

**Ready watchdog.** On each tick the loop selects every active non-Director member of the fleet that owns a pane, whose placement `created_at` is at least `READY_GRACE_SECONDS = 180` old, that has never sent a message, and whose `silence_notice_at` is `NULL`. It posts the silent-member notice and stamps `silence_notice_at`. `ready` is the only signal that the agent in a pane booted; the watchdog is the first thing that notices its absence. A member that goes silent after it has spoken is the monitor member's to report (§ S6).

**Monitor member.** Its role file gains one event: when one of its own commands is denied, it sends the Director a message once, naming the command and the denial text. When its `message send` is denied from the start, the ready watchdog reports it.

**Residual.** A loop that dies while no cafleet command is being run stays down until the next `message send`, `message broadcast` or `member exec` that leaves work owed restarts it; the WebUI shows it stopped in the meantime.

### S9. Contract summary

| Surface | Before | After |
|---|---|---|
| `messages` | No notification state | `notified_at` |
| `member_placements` | — | `keystroke_at`, `forced_at`, `silence_notice_at` |
| Tables | — | `member_execs` |
| Environment | — | `CAFLEET_DELIVERY_HOLD_TIMEOUT` (default `300`) |
| `fleet create` | Spawns the monitor member | Also starts the loop, reports `monitor_loop`, and compensates when it does not start |
| `cafleet monitor` | Flags, then environment, stamped at every start | Flags, then the stored row, then environment; a multiplexer probe before the claim; exit after three failing ticks |
| `message send` | One keystroke attempt; exit 1 on a failed keystroke | Gated attempt; held work retried by the loop; exit 1 only when work is owed and the loop cannot start |
| `message broadcast` | One keystroke attempt per recipient; always exit 0 | Gated attempts; exit 1 only when work is owed and the loop cannot start |
| `member ping` | Always keystrokes | Gated; `reason` key |
| `member prompt` | Plain and `--shell` forms | Plain form only; takes the pane claim |
| `member exec`, `member exec-run` | — | New |
| `doctor` | Three sections | Four sections; `member_permissions` key |
| Claude spawn argv | No allow rules | `--allowedTools <rule>...` |
| Monitor signals | `ready`, `monitor live`, `monitor restarted` | `ready` |
| Runtime bindings | Seven, with *Worked resolution* | Five |

### S10. Documentation and skill changes

`skills/cafleet/reference/runtime/` links into `docs/docs/`, so the docs edits also update the skill's runtime reference.

| File | Edit |
|---|---|
| `docs/docs/spec/data-model.md` | The V9 columns and `member_execs`; the pending-preview definition; broker notices. |
| `docs/docs/spec/cli-options.md` | `member exec` and `member exec-run`; `member prompt` plain-only; gated `member ping` and its `reason` key; the `message send` and `broadcast` outcome fields; removal of the partial-failure section and error; `fleet create` loop start, output and compensation; the `cafleet monitor` precedence and exits; the doctor section; `CAFLEET_DELIVERY_HOLD_TIMEOUT`; the `permissions.allow` coverage paragraph; the new error strings. |
| `docs/docs/spec/multiplexer-backends.md` | Push notifications rewritten around held delivery: the terms, the gate table, the timeout, the pane claim, the three preview payloads; the loop now keystrokes member panes; `send_prompt` without the shell form. |
| `docs/docs/spec/coding-agent-backends.md` | The claude spawn argv with `--allowedTools`, the rule table, and what the flags do not cover; the preset rules of § S5. |
| `docs/docs/spec/webui-api.md` | `POST /api/messages/send` delivery wording; the sentence on who launches the loop. |
| `docs/docs/concepts/monitoring.md` | The loop is started by `fleet create`; the delivery pass and the classifier states; the ready watchdog; the lifecycle without `monitor live`. |
| `docs/docs/concepts/member-lifecycle.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/how-to/mixed-backend-team.md`, `docs/docs/quickstart.md` | Remove the loop launch, `monitor live`, the `--shell` form and the shell-prompt sequence; describe `member exec`; state that claude members need no user-level allow rule for broker commands. |
| `SPEC.md` | The schema DDL, §6.2 broker delivery, §6.3 CLI, §6.5 multiplexer, §6.6 loop, §6.7 spawn argv, §7.1 environment, §10 checklist. |
| `skills/cafleet/SKILL.md` | Send and Broadcast sections without the no-resend procedure and the fresh-capture gate; held delivery in one paragraph; the team-supervision summary; "five runtime placeholders" and the documented-defaults table without `{bg_run}` and `{bg_stop}`; *One-shot command isolation* without its "sole exception" paragraph. |
| `skills/cafleet/reference/supervision.md` | Replace *The pre-ping capture gate* and *Deferred sends* with a short *Broker-held delivery* section; dispatch-on-ready is unconditional; bootstrap, monitor lifecycle, stall response, quick reference, recovery and shutdown follow § S4 and § S5. |
| `skills/cafleet/reference/prompt-routing.md` | Rewritten: the member asks, the Director runs `member exec`, the broker reports completion; plain `member prompt` for slash commands. |
| `skills/cafleet/roles/member.md`, `director.md`, `monitor.md` | Member: the command policy in § S5. Director: `member exec`, plain `member prompt`, gated `member ping`, notices. Monitor: startup without the loop, the denied-command event. |
| `skills/cafleet/reference/coding-agents.md` | Remove `{bg_run}`, `{bg_stop}`, the codex monitor-session note and *Worked resolution* from every backend section and the Template. |
| `skills/cafleet-design-doc/` | In `SKILL.md` (including its `{bg_run}` example token and its bootstrap sentence), the create, interview and execute workflow bodies and their Director role files: the monitor gate is `ready`; remove the capture gate, deferred sends and the shell-prompt sequence. |
| `.claude/rules/coding-agent-overlay.md`, `.claude/rules/bash-tool.md`, `.claude/skills/clean-docs/SKILL.md`, `.claude/skills/skill-author/SKILL.md` | Five bindings and five subsections; `member exec` in place of the shell-prompt sequence; the monitor gate is `ready`; replace the `{bg_run}` example token. |
| `presets/codex/cafleet.rules`, `presets/opencode/cafleet.md` | The rules of § S5 *Permissions*. |

The removal is total. Besides the rows above, Step 8 searches the repository outside `design-docs/` for every removed term and expects no hit: `bg_run`, `bg_stop`, `Worked resolution`, `monitor live`, `monitor restarted`, `--shell`, `capture gate`, `Deferred sends`, `pane notification failed`.

### S11. Tests and manual checklist

| Area | Cases |
|---|---|
| Classifier | Each state per coding agent from a short inline capture; precedence when two cues co-occur; a cue outside its region does not match; unknown coding agent → `Unclassified`. |
| Delivery step | Ordinary delivery on `finished`; hold on each other state; forced payload per state at the timeout; timeout `0` never forces; an exec in flight holds without forcing; hold age restarts at an exec's `finished_at` and at `forced_at`, so three overdue previews on an `awaiting_user` pane produce one forced keystroke per timeout; oldest item first; a failed keystroke clears the stamp; an ACKed row is never keystroked. |
| Pane claim | Two `deliver_pane` calls on one pane within the spacing window produce one keystroke; the wake and `member prompt` respect a held claim. |
| Loop | A preview held on `working` is keystroked by a later tick once the capture turns `finished`, with no further command; a tick with nothing owed captures no pane and sends no keystroke; the delivery pass runs with the wake interval `0`; a missing Director pane stops the loop; a per-pane capture error does not abort the tick; three consecutive `list_pane_ids` failures stop the loop and one success resets the count; lost execs close with a notice; the watchdog reports once. |
| Loop start | `ensure_monitor_loop` is a no-op when live, spawns when not, and reports the timeout error; a self-send, a send to a member without a pane, and a fully delivered send or broadcast never call it and exit 0 without a multiplexer; a send and a broadcast that leave work owed where no loop can start exit 1 with their own texts; a loop whose startup probe fails claims nothing; a restart keeps the stored tick and wake interval; `fleet create` compensates when the loop does not start. |
| Exec | Argument validation; the Director is rejected; `exec-run` runs once and records the exit code; resume by observation for each row of its table; `--wait` output. |
| Spawn argv | Claude argv for an ordinary member and for the monitor; codex and opencode unchanged. |
| Doctor | No finding; a `deny` match; an `ask` match; bare `Bash`; managed rules only, with and without a covering managed allow; an unreadable file; assets not installed; exit code and JSON shape. |
| Chain guard | In `cafleet/src/db/mod.rs`, the idempotence test renamed for version 9 and the chain guard with head `(9, "held_delivery_and_member_execs")` and versions 1..9; in `cafleet/src/diagnosis.rs`, the `(8, SchemaState::Head { version: 8 })` case. The integration tests read `head_version()` and need no edit. |
| `docs_sync` | Five runtime bindings, five subsections, no `monitor live`. |

Manual live checklist, run once by the user on a real fleet:

1. With no `Bash(cafleet ...)` allow rule in the user's `settings.json`, a claude member sends `ready` and ACKs its first message.
2. While the Director's pane shows a permission prompt, a member sends a message: the prompt stays open, and the preview lands after the Director's turn ends.
3. On codex with herdr, a member sends `ready` and keeps reading; the Director dispatches at once and the assignment lands when the member goes idle, with no user input.
4. `cafleet member exec` with a command that runs for three minutes: the Director receives the completion notice and the member continues, with no ping and no capture.
5. With `CAFLEET_DELIVERY_HOLD_TIMEOUT=30` and the Director left on a prompt, the forced preview carries the broker note and the Director re-issues the tool call.
6. With `"Bash(cafleet *)"` in `permissions.deny`, `cafleet doctor` exits 1 and names the file and rule.
7. On each coding agent, after the loop process is killed, a `message send` from a member pane restarts it and the message is delivered.
8. On each coding agent, a member continues exactly once after an exec finishes: no resume line where the harness answers `!` output, one where it only stages it.
9. On each coding agent, the detached loop is still running after the `fleet create` tool call returns.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first, per the project's documentation-maintenance rule; code starts at Step 3.

### Step 1: User-facing documentation

- [x] `docs/docs/spec/data-model.md`: V9 columns, `member_execs`, pending previews, broker notices. <!-- completed: 2026-10-10T14:35 -->
- [x] `docs/docs/spec/cli-options.md`: every item of its § S10 row, with the output shapes and error strings of § S3–S7. <!-- completed: 2026-10-10T14:39 -->
- [x] `docs/docs/spec/multiplexer-backends.md`: held delivery, terms, gate table, pane claim, preview payloads, `send_prompt`. <!-- completed: 2026-10-10T14:40 -->
- [x] `docs/docs/spec/coding-agent-backends.md` and `docs/docs/spec/webui-api.md`: spawn argv and rules, preset rules, send-route and loop-launch wording. <!-- completed: 2026-10-10T14:41 -->
- [x] `docs/docs/concepts/monitoring.md`, `docs/docs/concepts/member-lifecycle.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/how-to/mixed-backend-team.md`, `docs/docs/quickstart.md`: loop start, delivery pass, watchdog, lifecycle, `member exec`. <!-- completed: 2026-10-10T14:44 -->
- [x] `SPEC.md`: the sections listed in § S10. <!-- completed: 2026-10-10T14:53 -->

### Step 2: Skills, rules and presets

- [x] `skills/cafleet/SKILL.md` and `skills/cafleet/reference/supervision.md` per § S10. <!-- completed: 2026-10-10T15:01 -->
- [x] `skills/cafleet/reference/prompt-routing.md` and the three role files under `skills/cafleet/roles/` per § S10. <!-- completed: 2026-10-10T15:01 -->
- [x] `skills/cafleet/reference/coding-agents.md`: remove the two bindings, the codex note and *Worked resolution*. <!-- completed: 2026-10-10T15:01 -->
- [x] `skills/cafleet-design-doc/`: the edits listed in § S10. <!-- completed: 2026-10-10T15:04 -->
- [x] The two rule files and two skill files under `.claude/` named in § S10, and both preset files. <!-- completed: 2026-10-10T15:08 -->
- [x] `cafleet/tests/docs_sync.rs`: expectations for five bindings, five subsections and the removed signals. <!-- completed: 2026-10-10T15:08 -->

### Step 3: Migration V9

- [x] Add `cafleet/migrations/V9__held_delivery_and_member_execs.sql`. <!-- completed: 2026-10-10T15:06 -->
- [x] Bump the head literals: the two tests in `cafleet/src/db/mod.rs` and the `Head` case in `cafleet/src/diagnosis.rs`, per the *Chain guard* row of § S11. <!-- completed: 2026-10-10T15:06 -->

### Step 4: Classifier

- [x] `cafleet/src/pane_state.rs`: `PaneState`, the regions, the per-agent cue tables of § S2, `classify`. <!-- completed: 2026-10-10T15:16 -->
- [x] The classifier tests of § S11. <!-- completed: 2026-10-10T15:17 -->

### Step 5: Held delivery and the loop

- [x] `cafleet/src/config.rs`: `delivery_hold_timeout` from `CAFLEET_DELIVERY_HOLD_TIMEOUT`. <!-- completed: 2026-10-10T15:33 -->
- [x] Broker per § S3 *Code seam*: `send_message` and `broadcast_message` persist only; pending-preview queries; `post_notice`; the pane claim; delete `InlinePreviewSender`, `RuntimeNotifier` and `NotificationAttempt`. <!-- completed: 2026-10-10T15:33 -->
- [x] Multiplexer: `send_inline_preview` with `note`; `send_prompt` without `shell`, on tmux and herdr; the `PaneIo` implementation. <!-- completed: 2026-10-10T15:33 -->
- [x] `cafleet/src/delivery.rs`: `deliver_pane`, `PaneOutcome` and the gate table. <!-- completed: 2026-10-10T15:33 -->
- [x] `message send`, `message broadcast`, `POST /api/messages/send`, `member ping`, `member prompt` and the monitor wake per § S3 *Command behavior*; remove the partial-failure exit. <!-- completed: 2026-10-10T15:33 -->
- [x] `ensure_monitor_loop` and its call sites; `fleet create` output and compensation; the `cafleet monitor` precedence, startup probe, failure limit and `SIGHUP` handler. <!-- completed: 2026-10-10T15:33 -->
- [x] `monitor_tick`: the delivery pass, the Director-pane stop, the echo lines. <!-- completed: 2026-10-10T15:33 -->
- [x] The ready watchdog. <!-- completed: 2026-10-10T15:33 -->
- [x] Tests for the delivery step, pane claim, loop and loop start per § S11. <!-- completed: 2026-10-10T15:49 -->

### Step 6: `member exec`

- [x] `member exec` and `member exec-run`, and the `member_execs` broker functions. <!-- completed: 2026-10-10T15:46 -->
- [x] Exec items and resume by observation in `deliver_pane`; lost-exec closing in the loop; the exec notices. <!-- completed: 2026-10-10T15:46 -->
- [x] Remove `--shell` from `member prompt` and every code path that served it. <!-- completed: 2026-10-10T15:46 -->
- [x] Exec tests per § S11. <!-- completed: 2026-10-10T15:49 -->

### Step 7: Spawn rules and doctor

- [x] `build_spawn_argv` with `monitor`, the claude `--allowedTools` rules, and the argv tests. <!-- completed: 2026-10-10T15:55 -->
- [x] The doctor member-permissions section, its JSON key, and its tests. <!-- completed: 2026-10-10T15:55 -->

### Step 8: Verification

- [x] The removed-term search of § S10 returns no hit outside `design-docs/`. <!-- completed: 2026-10-10T15:58 -->
- [x] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test`, `mise //admin:lint` and `mise //admin:test` pass. <!-- completed: 2026-10-10T15:59 -->
- [ ] The user runs the manual live checklist of § S11 and every item passes. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-10 | Initial draft |
| 2026-10-10 | Reviewer round 1: issue-coverage table; cue regions, candidate cues for every backend and the fixture procedure; delivery terms, `forced_at`, the code seam; the wake, `member prompt` and the HTTP route in the command table; loop tick and interval precedence, multiplexer probe and failure limit; `fleet create` compensation; exec resume by observation; preset and permission corrections; managed-rules-only and hook limits; the removed-term search; the #395 loop tests; head-literal locations |
| 2026-10-10 | Reviewer round 2: `message send` and `message broadcast` ensure the loop only when the fleet has owed work; the broadcast failure text; the no-work test cases |
| 2026-10-10 | Approved: Reviewer approved in round 2; user approved |
| 2026-10-10 | User decision at execute start: the recorded pane fixtures, their user prerequisite and the fixture-driven test are dropped; the classifier implements the § S2 cue table as written and is covered by inline-capture tests |
| 2026-10-10 | Director arbitration: the `member prompt --shell` parse test is dropped — the flag's absence is the check, and the removed-term search of § S10 stays free of `--shell` |
| 2026-10-10 | Director confirmation of Step 5 implementation choices: a loop that reaches the failure limit or fails its startup probe exits 1 with the multiplexer's error; `ensure_monitor_loop` reports `cannot open monitor log <path>: <error>` and `cannot spawn the monitor loop for fleet <id>: <error>`; `member ping` reports a failed capture as `capture failed: <error>` with exit 1; the delivery pass echoes per-pane capture, keystroke and delivery failures |
