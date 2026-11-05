# Fleet-Lifetime Director Monitoring

**Status**: Approved
**Progress**: 27/27 tasks complete
**Last Updated**: 2026-11-05

## Overview

Director reception follows the fleet's lifetime: member creation, `ready`, final replies, completed work, an empty assignment queue, idle periods and user handoffs keep the receiver running, and only fleet deletion ends it normally. The change rewrites the Hermes native Director reception recipe and the shared Director role text it links, aligns the Claude/Codex recipes, SPEC, docs and design-doc workflow skills with that rule, and raises the Claude Monitor lease to `--timeout 1770` / `timeout_ms 1800000` (GitHub issue #562). Live Hermes and Claude Director runs verify the policy and are recorded as Role eligibility evidence.

## Success Criteria

- [ ] Every surface in § Replacement texts carries its replacement verbatim, with run placeholders substituted.
- [ ] `grep -rn "while delegated work remains\|while authorized work remains\|while authorized delegated work remains\|until work resolves\|until delegated work resolves\|user-feedback phase\|fleet-supervision phase\|bounded supervision phase\|retiring reception while the fleet stays live\|stop reception while the fleet stays live\|retirement or failure\|drain the finite lease\|drain its finite lease\|Drain/cancel native reception\|Collect receiver closure" SPEC.md docs/docs skills` returns nothing.
- [ ] `grep -rn "timeout 210\|240000" SPEC.md docs/docs skills` returns nothing, and the JSON line in `coding-agents.md` § Claude native Director reception is the only line of that file containing `"command":"cafleet message listen 1 --timeout 1770"`, and it carries `"timeout_ms":1800000`.
- [ ] Every `action=kill` in `skills/cafleet/reference/coding-agents.md` lies in the Hermes paragraph that limits it to recovery replacement and post-`fleet_deleted` cleanup.
- [ ] § Live evidence holds a `Pass` row for Run H-A, Run H-B and Run C-A, and the Claude and Hermes Director Evidence cells are identical in `SPEC.md` and `coding-agents.md`.
- [ ] `mise //:lint`, `mise //:test` and `mise //docs:build` pass; `git diff --stat` lists only the files named in § Change surface plus this design doc.

---

## Background

The Hermes recipe (`coding-agents.md` § Hermes native Director reception, design 0000215) bounds the receiver by a "fleet-supervision phase", renews a finite lease "only while delegated work remains" and documents `process_manage` `action=kill` as an ordinary retirement step. The shared Director text it links carries the same per-work lifetime: Spawn Protocol step 7 ("until work resolves or an explicit user handoff occurs"), Director wake step 1 ("while assignments or an authorized user-feedback phase remain outstanding"), the Receiver recovery `timeout` row ("only while authorized delegated work remains") and Shutdown step 3 ("If retiring reception while the fleet stays live …").

On CAFleet 0.31.3 with a Hermes Director on herdr, a creation-only request ended with the Director killing its listener after ACKing `ready`, while the fleet and member stayed alive; the member's later `hello` was persisted but went unnoticed until the user intervened. Claude and Codex recipes, SPEC, docs and the design-doc workflow skills repeat the per-work lifetime, and the create/execute cleanup steps collect receiver closure before fleet deletion.

The Claude Monitor recipe uses `--timeout 210` / `timeout_ms 240000`, re-arming roughly every four minutes while idle. Monitor caps `timeout_ms` at 1,800,000 ms, and `message listen` accepts leases `1..=1800`.

---

## Specification

### Decisions

| Topic | Decision |
|---|---|
| Lifetime rule | One receiver per Director for the fleet's lifetime on every backend. Creation-only requests, `ready`, final replies, completed work, an empty assignment queue, idle periods and user handoffs keep it; fleet deletion is the only normal end. |
| Hermes-centric scope | The Hermes recipe and the shared `director.md` text carry the substantive policy. Claude and Codex recipes get wording-only consistency edits; their mechanics stay, apart from the Claude lease. |
| Finite leases | Hermes uses `--timeout` only when a host policy requires finite leases ("bounded supervision phase" is removed). Every backend renews a clean `timeout` closure while the fleet exists, regardless of outstanding assignments. |
| Timeouts and crashes | Recovery conditions, never successful completion. Existing safeguards stay: verified termination of the old receiver, a fresh successful health/inbox check before replacement, exactly one receiver. |
| Hermes `action=kill` | Allowed in exactly two places: (a) recovery replacement of a receiver whose liveness is in doubt or that is wedged, or of a second receiver left by a session change; each kill is followed by verified exit, a fresh successful health/inbox check and exactly one running receiver; (b) a receiver still running after its `closed` `fleet_deleted` record, as part of fleet deletion, with verified exit and no restart. |
| Wedged receiver | A live process while `poll --json` shows a pending delivery whose ID the receiver has never emitted and that was created after its last emitted record, with no heartbeat carrying output and no read returning new lines across at least two heartbeat intervals. Rolling-log overflow (a heartbeat with a nonzero `omitted`, a read whose `total_lines` is below the retained offset, or heartbeat-turn reads returning fewer records than the heartbeat carried) and emitted-but-unACKed deliveries, which `listen` never re-emits, are healthy states reconciled through `poll --json`, never treated as wedged. |
| Partial deletion | Whenever `fleet delete` exits 1, the fleet stays registered, reception stays active and a lease expiring in that window is renewed. This covers every `fleet was not deleted` outcome in `SPEC.md` § `fleet delete`: a member that could not be ended, pane-input lock contention and a placement or ownership change. |
| No receiver at deletion | When the receiver had exited and was not yet replaced when the fleet is deleted, the Director confirms deletion with `cafleet fleet list` and a final `cafleet message poll <director-member-id> --json`, starting no replacement. |
| Quiet reception | Idle monitoring sends no check-in message, invents no assignment and gives no "nothing received" update. Message-ID deduplication and ACK semantics stay unchanged. |
| No new skill text | Explicit user stop requests, Hermes host exit and `member_inactive` closure keep their existing guidance: the Receiver recovery `member_inactive` / `fleet_deleted` row (retire the handle) and the existing limitation reporting. |
| Claude lease | `--timeout 1770` with `timeout_ms 1800000`: the Monitor maximum, with the CLI lease closing 30 s inside it, so idle re-arm drops from about every 4 minutes to about every 30 minutes. |
| Workflow cleanup order | create and execute workflows delete the fleet first, then collect the `fleet_deleted` closure and verified receiver exit. |
| Code | None. No recipe literal lives in Rust source; the change is SPEC, docs and embedded skill text. No unit tests are added, and no CLI reinstall happens during implementation, so the executing fleet's installed runtime and assets stay intact. |

### Change surface

| File | Sections |
|---|---|
| `SPEC.md` | §2 Architecture stance (Director paragraph); §6.7 Role eligibility (`claude` and `hermes` Director rows); §6.7 Host evidence and native-input authorization (**claude**, **codex**, **hermes** paragraphs) |
| `docs/docs/mixed-backend-team.md` | Receive results (lease-renewal paragraph); Close the fleet (two paragraphs after the commands) |
| `docs/docs/concepts/overview.md` | Watch the team work |
| `skills/cafleet/reference/coding-agents.md` | § claude: Claude native Director reception, Role eligibility Director row; § codex: Note row, Codex native Director reception; § hermes: Hermes native Director reception, Role eligibility Director row |
| `skills/cafleet/roles/director.md` | Spawn Protocol step 7; Director wake step 1; Receiver recovery `timeout` row; Shutdown steps 2–3 |
| `skills/cafleet-design-doc/SKILL.md` | Dispatch |
| `skills/cafleet-design-doc/create/create.md` | Operating rules paragraph ("Maintain native reception …"); cleanup paragraph |
| `skills/cafleet-design-doc/execute/execute.md` | Step 8: Finalize & Clean Up |

`README.md`, `.claude/rules/`, `skills/cafleet/reference/cli.md`, `skills/cafleet/SKILL.md`, the `message listen` contract and every Rust file stay unchanged.

### Replacement texts

Each entry names the existing text it replaces; unquoted text in the same paragraph stays unchanged.

#### `SPEC.md` §2 Director paragraph

The sentence beginning "A Director arms native reception before relying on replies" becomes:

> A Director arms native reception before relying on replies and retains one receiver for the fleet's lifetime, which ends normally only through fleet deletion, preserving supplied goal controls and user decision gates.

#### `SPEC.md` §6.7 Host evidence and native-input authorization

- **claude** paragraph: "retaining its handle across separate events and renewing after observed source-task exit." becomes "retaining its handle across separate events and renewing after observed source-task exit while the fleet exists."
- **codex** paragraph: "explicitly read it while delegated work remains, independently of whether a host goal exists." becomes "explicitly read it while the fleet exists, independently of whether a host goal exists."
- **hermes** paragraph: insert after "so broker reads remain the completeness check.":

> The receiver follows the fleet's lifetime: creation-only requests, completed work, idle periods and user handoffs keep it, a finite lease is renewed while the fleet exists, and only fleet deletion ends it normally; `process_manage` kill serves receiver replacement during recovery and ends a receiver still running after `fleet_deleted`.

#### Director Evidence cells

Both the `SPEC.md` § Role eligibility rows and the matching `coding-agents.md` Role eligibility rows receive the same Evidence cell, with at most two sentences per cell (`.claude/rules/documentation-maintenance.md` § Table conventions).

`claude` Director:

> Goal lifecycle: [Claude Code goal](https://code.claude.com/docs/en/goal), 2.1.296, checked 2026-10-22. Streamed background output: Monitor stream probes, 2.1.301, checked 2026-10-25; fleet-lifetime Monitor lease (`--timeout 1770` / `timeout_ms 1800000`, CLI `timeout` closure before Monitor expiry, single-receiver re-arm), `<claude version>`, checked `<claude run date>`.

`hermes` Director:

> Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Streamed background output and idle wake: CAFleet stream probe with `terminal` `background=true` read through `process_manage` `action=log`, 2026.9.24, checked 2026-11-03; persistent `listen` receiver with `terminal` `heartbeat=60`, clean first-observation and extended live runs, 2026.9.24, checked 2026-11-04; fleet-lifetime runs (creation-only receiver kept through 60 idle minutes, rolling-log overflow reconciliation, unexpected-exit replacement, failed-deletion retention, finite-lease renewal), `<hermes version>`, checked `<hermes run date>`.

#### `docs/docs/mixed-backend-team.md`

- Receive results: "The Director renews a listener whose lease expired while authorized work remains." becomes "The Director keeps its listener for the fleet's lifetime and renews one whose lease expired while the fleet exists."
- Close the fleet: the two paragraphs after the `cafleet fleet list` block become:

> Fleet deletion closes the listener with reason `fleet_deleted` ([message listen](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen)), the listener's only normal end; the Director confirms that the listener exited and starts no replacement. Pending messages stay available for later reads.
>
> Fleet deletion retains history. When `cafleet fleet delete` exits 1, the fleet stays registered and the Director keeps its listener; resolve the reported cause and rerun `cafleet fleet delete`. Limit cleanup to the team you own.

#### `docs/docs/concepts/overview.md` Watch the team work

"The Director receives replies and checks evidence until delegated work resolves." becomes "The Director receives replies and checks evidence for as long as the fleet exists."

#### `coding-agents.md` Claude native Director reception

The subsection is replaced in full:

````markdown
#### Claude native Director reception

Discover Monitor through the host's tool discovery and authorize it separately from Bash broker rules. After registration, start one Monitor with the literal Director ID:

```json
{"command":"cafleet message listen 1 --timeout 1770","description":"CAFleet Director inbox","timeout_ms":1800000}
```

Monitor caps `timeout_ms` at 1,800,000 (30 minutes); the 1,770-second CLI lease closes 30 seconds inside that deadline, so the `closed` `timeout` record and stream completion precede Monitor expiry.

Retain its task ID, pinned member/fleet IDs, receive status and unfinished actions. Require `ready`; consume complete stdout records, reconcile known/replayed IDs against the durable inbox, record deferred work, ACK consumption and act within authorization. Keep Monitor active across other work, idle periods and turn endings while the fleet exists. Events during a user question preserve that pending decision.

On `closed` or stream completion, verify task/source-child termination before replacement. Renew while the fleet exists, reconcile replay and deduplicate closure/completion by owned task ID. A host timeout before CLI closure follows [receiver recovery](../roles/director.md#receiver-recovery).

Reception ends with the fleet: after `closed` `fleet_deleted`, verify task/source-child exit and start no replacement. Use verified native cancellation with observed child cleanup only to replace a receiver under [receiver recovery](../roles/director.md#receiver-recovery). `Bash(run_in_background=true)` completion notifications establish no per-line reception. Unavailable or denied Monitor reception follows [receiver recovery](../roles/director.md#receiver-recovery).
````

#### `coding-agents.md` § codex

- Note row: "Retain native session handles and explicitly read the owned receiver while delegated work remains." becomes "Retain native session handles and explicitly read the owned receiver while the fleet exists."
- Codex native Director reception, second paragraph: "While delegated work remains, read that session" becomes "While the fleet exists, read that session".
- Codex native Director reception, the paragraph beginning "On `closed`/exit" is replaced in full:

> On `closed`/exit, verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery). Renew clean expiry while the fleet exists; unexpected failures require a fresh health/inbox check. Preserve session/identity, status, partial JSONL and unfinished actions across compaction. Reception ends with the fleet: after `closed` `fleet_deleted`, verify exit, discard partial final data and start no replacement; use verified native cancellation only to replace a receiver under [receiver recovery](../roles/director.md#receiver-recovery).

#### `coding-agents.md` Hermes native Director reception

The subsection is replaced in full:

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

Omit `--timeout`: the receiver runs until its `closed` record or failure. Require its `ready` record. Retain the returned process `session_id`, the Director ID, the log line offset already read, incomplete JSONL and unfinished actions, and keep this one receiver across messages, replies, idle periods and turn endings.

Every 60 seconds Hermes injects a heartbeat notification carrying the output the running receiver emitted since the previous one, which wakes an idle Director without user input; a tick with no new output sends nothing. Keep `heartbeat=60`; Hermes raises smaller values to 60. A heartbeat is a batched wake signal: it can carry several records, keeps only the newest 2,000 characters (its `omitted` field counts the rest), can arrive after its records were consumed, and turn admission adds latency beyond the interval.

On every heartbeat turn and between work steps, read the receiver with `process_manage` `action=log`, `offset` advanced past lines already read and repeated until no new lines remain, on the retained `session_id`. Accumulate incomplete JSONL, consume complete records and follow [Director wake](../roles/director.md#director-wake), whose `cafleet message poll <director-id> --json` is the completeness check. Hermes keeps a 200,000-character rolling log; once the receiver's output outgrows it, older lines drop, `total_lines` counts only the retained lines and the retained offset no longer marks the unread position, so records can be absent from every read. Treat the log as overflowed when a heartbeat carries a nonzero `omitted`, a read returns a `total_lines` below the retained offset, or a heartbeat turn's reads return fewer records than its heartbeat carried; then reconcile through `poll --json` and set the offset to the returned `total_lines`. Deduplicate by message ID: a record already consumed, including the startup output the first heartbeat repeats, is replay, never reported as new or ACKed again. Then end the turn with the same receiver running, inventing no assignment and sending no check-in message or "nothing received" update. Start no `message wait` as a wake or rearm mechanism; a finite `wait` for ordinary-member retry rotation under Director wake step 4 remains available.

Hermes documents `heartbeat` for bounded jobs rather than servers. This receiver is bounded by the fleet's lifetime; it ends with `closed` `fleet_deleted`. Keep it running while the fleet exists, including after a creation-only request, the `ready` handshake, a final reply, completed work, an empty assignment queue, idle periods and user handoffs.

When a host policy requires finite leases, add `--timeout <seconds>` (`1..=1800`) to the same call; after its `closed` `timeout` record, verify termination and start one replacement while the fleet exists. Add `persist_on_release=True` only when the operator explicitly asks for reception to survive conversation cleanup; it does not survive Hermes host exit and does not guarantee notification ownership after a session change, so reconcile with `poll --json` after any such change.

The exit notification follows the receiver's `closed` record or exit: verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery). While the fleet exists, a `timeout` closure, unexpected exit or failure is a recovery condition, never the end of monitoring. Use `process_manage`, `action=kill` on a retained `session_id` only for recovery replacement: of a receiver whose liveness is in doubt, of a wedged receiver, or of a second receiver left by a session change. A receiver is wedged when `poll --json` shows a pending delivery whose ID the receiver has never emitted and that was created after its last emitted record, while no heartbeat carries output and no read returns new lines across at least two heartbeat intervals. Rolling-log overflow and emitted-but-unACKed deliveries, which the receiver never re-emits, are healthy states reconciled through `poll --json`, never wedged. Verify that the killed receiver exited, complete a fresh successful health/inbox check, then run exactly one receiver: the replacement, or the retained one when a duplicate was ended. After `fleet_deleted`, reconcile the exit output as history, verify that the receiver exited and start no replacement; a receiver still running after that record ends with `action=kill` as part of fleet deletion, followed by verified exit. Preserve the process `session_id`, log offset, incomplete JSONL and unfinished actions across compaction. When the background facility is unavailable or denied, follow [receiver recovery](../roles/director.md#receiver-recovery).

Reception uses process output, heartbeat notifications and broker reads, makes no pane input and leaves `/goal` `gate`/`wait` authority with the host; heartbeat wake needs no `/goal`.
````

#### `skills/cafleet/roles/director.md`

- Spawn Protocol step 7 becomes:

> 7. Keep native reception active and follow [Director wake](#director-wake) for the fleet's lifetime; [Shutdown](#shutdown) ends it through fleet deletion.

- Director wake step 1 becomes:

> 1. Retain one owned native handle for the fleet's lifetime. Member creation, `ready`, a final reply, completed work, an empty assignment queue, idle periods and user handoffs keep it; only [fleet deletion](#shutdown) ends it normally. Store its pinned member/fleet IDs, receive status, incomplete JSONL and unfinished actions across context compaction.

- Receiver recovery, `Clean closed with timeout` row response becomes "Establish task exit, then renew while the fleet exists". Every other row stays unchanged.
- Shutdown steps 2 and 3 become:

> 2. When `cafleet fleet delete` exits 1, the fleet stays registered and native reception stays active. Run `cafleet member list <fleet-id>`, resolve the reported cause, then rerun `cafleet fleet delete <fleet-id>`; it resumes with the remaining members.
> 3. Collect the listener's `fleet_deleted` closure and native task exit, then retire its handle without replacement. When no receiver was running at deletion, run a final `cafleet message poll <director-member-id> --json` instead. Discard an incomplete final line and preserve pending messages for later reads.

#### `skills/cafleet-design-doc`

- `SKILL.md` Dispatch: "The Director retains native inbox reception while delegated work remains;" becomes "The Director retains native inbox reception for the fleet's lifetime, ending it only through fleet deletion;".
- `create/create.md`: "Maintain native reception and supervise outstanding work under the shared Director procedure." becomes "Maintain native reception for the fleet's lifetime and supervise outstanding work under the shared Director procedure."
- `create/create.md` cleanup paragraph becomes:

> Delete owned ordinary members, verify the root-only roster, delete the owned fleet, then collect the receiver's `fleet_deleted` closure and observed process exit and confirm the fleet's absence. When a deletion fails and the fleet stays registered, keep native reception active, resolve the cause and rerun. Preserve resources outside this team's ownership.

- `execute/execute.md` Step 8, the paragraph beginning "Drain/cancel native reception" becomes:

> Delete the fleet, which ends every owned ordinary member and reports each pane outcome; when it exits 1 and the fleet stays registered, keep native reception active, resolve the reported cause and rerun. Then collect native reception's `fleet_deleted` closure, observe its source-process exit and verify the fleet's absence. Limit cleanup to owned resources.

### Live verification

The implementing execute-team member performs every run. Each run starts its own fleet Director in a separate multiplexer pane against an isolated database, so the executing fleet's installed runtime and assets stay intact.

**Environment.** Follow `docs/docs/contributing.md` § Run a CAFleet stream probe step 1 for an isolated database in a fresh short `mktemp -d` directory, and run `cafleet setup --coding-agent <director backend>` against it with the installed binary, which reinstalls that binary's own embedded assets unchanged. Start the Director in a directory Claude Code already trusts, so the probe member opens without a folder-trust dialog (0000215 Run A attempt 1 was invalidated by one). Write session exports, pane captures and generated bodies to the hidden `probe/` directory beside this design doc.

**Director pane (herdr).** Runs use herdr, the issue's environment. `<db url>` is `sqlite:///<probe dir>/director.db`.

| Purpose | Command |
|---|---|
| Open the Director pane | `herdr tab create --cwd <trusted dir> --label <run name> --env CAFLEET_DATABASE_URL=<db url> --no-focus`; take the new tab's pane ID from its JSON result, or from `herdr pane list` |
| Launch a Hermes Director | `herdr pane run <pane> "hermes chat --cli"` |
| Launch a Claude Director | `herdr pane run <pane> "claude"`, in the operator's configured permission mode with Monitor authorized |
| Type the task prompt or a scheduled prompt | `herdr pane run <pane> "<one-line prompt>"`, which sends the text and Enter |
| Capture the Director pane | `herdr pane read <pane> --format ansi --source recent-unwrapped --lines 2000` |

The Director pane is not a CAFleet member, so the implementer types into it with herdr directly rather than `cafleet member prompt`. On tmux, the equivalents are `tmux new-window -d -c <trusted dir> -e CAFLEET_DATABASE_URL=<db url> -P -F '#{pane_id}'`, `tmux send-keys -t <pane> -l '<text>'` followed by `tmux send-keys -t <pane> Enter`, and `tmux capture-pane -p -e -t <pane> -S -2000`. `cafleet setup --coding-agent hermes` puts `CAFLEET_DATABASE_URL` in `terminal.client_env_passthrough`, so the Hermes Director's terminal tools see the pane's database. Every operator `cafleet` call (sends, `member show`, `message show`, `fleet list`) carries the leading assignment `CAFLEET_DATABASE_URL=<db url>`.

**Skill source.** Each Director reads the edited working-tree skills by absolute path instead of any installed copy: `<worktree>/skills/cafleet/SKILL.md`, its own backend section of `<worktree>/skills/cafleet/reference/coding-agents.md` and `<worktree>/skills/cafleet/roles/director.md`, where `<worktree>` is the implementation checkout's absolute path.

**Probe member and sends.** The Director creates one probe member with `--coding-agent claude --model haiku`, whose spawn prompt sends `ready` and then ends its turn. Operator records are sent from a separate shell with the probe's identity: `cafleet message send --from-member-id <probe-id> --to-member-id <director-id> "<body>"` or `--file <path>`. "Idle" means the Director's turn has ended and its composer shows an empty input row; every scheduled send waits for an idle Director, except within a Run H-A phase 2 burst.

**Director task prompt** (`<lease clause>` is empty except in Run H-B):

> Read `<worktree>/skills/cafleet/SKILL.md`, the `<backend>` section of `<worktree>/skills/cafleet/reference/coding-agents.md` and `<worktree>/skills/cafleet/roles/director.md` by absolute path, in place of any installed cafleet skill, and follow them exactly. `<lease clause>` Create one probe member with `--coding-agent claude --model haiku` whose spawn prompt sends `ready` and then ends its turn. Consume its ready, report that the member is created, and end your turn. On every later turn, follow the reception recipe and the Director wake procedure, write one line naming each message ID you consumed and what woke you (writing "replay" for IDs already consumed), then end your turn.

T0 is the end of the turn in which the Director reports the member created, after every startup replay turn has ended.

**Common checks.** "One receiver" means exactly one process qualifies under both filters. First, `ps -C cafleet -o pid=,args= | grep -E ' message listen <director-id>( |$)'` selects processes whose executable name is `cafleet`, which excludes wrapper shells (Hermes's login shell, Monitor's shell) and the `grep` itself, and its anchored pattern excludes longer IDs. Second, for each listed PID, `grep -a -z '^CAFLEET_DATABASE_URL=' /proc/<pid>/environ` must show the run's `<db url>`, which excludes listeners of other fleets that share the Director ID. "Consumed once" means the session export shows exactly one `cafleet message ack <id>` for that ID and `cafleet message show <id> --json` reports `completed`. "No leak" means no process qualifies under the one-receiver filters and, for Hermes, `<hermes home>/processes.json` has no entry with a live `pid`.

#### Run H-A: Hermes fleet lifetime

Hermes Director, persistent recipe (no `--timeout`). Phases run in order in one fleet.

| Phase | Action | Pass conditions |
|---|---|---|
| 1. Creation-only idle | After T0, send nothing for 60 minutes; sample "one receiver" at T0 + 30 min and T0 + 60 min. Then send `lifetime-probe H1`. | Same `session_id` throughout; no `process_manage` `kill`, new `terminal` listener or `message wait`; no Director turn and no Director-sent message between T0 and H1's heartbeat; both samples show one receiver; H1 is first observed in a heartbeat injected into the idle Director and consumed once. |
| 2. Rolling-log overflow | Generate 24 bodies of exactly 10,000 characters, each starting `overflow-probe NN`. Send 12 within 10 s; once the Director has consumed them and is idle, send the other 12 within 10 s. Only each burst's first send waits for an idle Director. Then send `lifetime-probe H2`. | The receiver's output exceeds 200,000 characters (an offset-0 `log` read no longer starts with the `ready` record); on at least one heartbeat turn an overflow signal of the recipe (a nonzero heartbeat `omitted`, a `total_lines` below the retained offset, or reads returning fewer records than the heartbeat carried) is followed by `poll --json` reconciliation and an offset reset to the returned `total_lines`; all 24 IDs and H2 are consumed once; same `session_id`, one receiver. |
| 3. Unexpected exit | From the operator shell, `kill -TERM <listener pid>`. After the Director's recovery turn ends, sample "one receiver", then send `lifetime-probe H3`. | The Director inspects the exit, completes a fresh successful `poll --json` before starting exactly one replacement with `heartbeat=60`, retains its new `session_id`; one receiver; H3 is consumed once through the replacement. |
| 4. Failed deletion | Hold the probe's pane input lock with the recipe below the table, then prompt the Director: "Delete the fleet. If deletion fails, report the failure and end your turn; I will tell you when to retry." After `flock` exits, send `lifetime-probe H4`. | `fleet delete` exits 1 with `fleet <fleet-id> pane input remained busy for 5 seconds at member <probe-id>; fleet was not deleted.` and `cafleet fleet list` still shows the fleet; the receiver keeps its `session_id` with no `kill`; one receiver; H4 is consumed once through a heartbeat. |
| 5. Deletion | Prompt the Director: "Retry the fleet deletion." | `fleet delete` succeeds; the receiver emits `closed` `fleet_deleted` and exits 0; the Director treats the exit notification as history, starts no replacement and confirms the fleet's absence; no leak. |

Phase 4 lock recipe (`SPEC.md` § Pane input lock; `src/pane_lock.rs` `lock_path`):

1. `realpath <probe dir>/director.db` gives `<canonical db>`.
2. From the `placement` object of `cafleet member show <probe-id> --json`, read `backend`, `mux_session` and `mux_pane_id`. `backend` is the multiplexer (`herdr` or `tmux`), not the coding agent.
3. `printf '%s\0%s\0%s' <backend> <mux_session> <mux_pane_id> | sha256sum` prints the lowercase hex `<key>` as its first field.
4. `mkdir -p <canonical db>.pane-locks` creates the lock directory, which exists only after a pane-input writer has run.
5. In a separate shell, `flock <canonical db>.pane-locks/<key>.lock sleep 90` holds the lock for 90 seconds; prompt the Director while it runs.

#### Run H-B: Hermes finite lease

Hermes Director in a fresh fleet, with `<lease clause>` = "Host policy for this run requires finite receiver leases of 120 seconds."

1. After T0, send nothing until three `closed` `timeout` records have occurred; sample "one receiver" after each replacement turn.
2. Send `lease-probe HB`.
3. Prompt the Director: "Delete the fleet."

Pass: every receiver carries `--timeout 120`; after each `closed` `timeout`, the Director verifies termination before starting exactly one replacement, with no assignment outstanding; every sample shows one receiver; no Director-sent message after the ready ACK; HB is consumed once; deletion yields `closed` `fleet_deleted`, no replacement and no leak.

#### Run C-A: Claude Monitor lease

Claude Director (installed Claude Code version) in a fresh fleet, persistent recipe. Record the Monitor start time as M0.

1. Send nothing until the first Monitor stream completes; record the CLI `closed` `timeout` record time and the stream completion time.
2. After the Director's re-arm turn ends, sample "one receiver" and confirm the Monitor task list shows exactly one running CAFleet Director inbox task.
3. Send `lease-probe C1`.
4. Prompt the Director: "Delete the fleet."

Pass: `closed` `timeout` arrives between M0 + 1,770 s and M0 + 1,800 s, and the stream completes before the Monitor deadline with no Monitor timeout; the Director verifies source-task exit before re-arming exactly one Monitor with the same recipe; one receiver; C1 is consumed once; deletion yields `closed` `fleet_deleted`, task exit, no re-arm and no leak.

#### Classification

| Classification | Condition |
|---|---|
| Invalid | User input other than the scheduled prompts; a send while the Director's turn was running, except the second through twelfth sends of a Run H-A phase 2 burst; a Director or probe crash or host dialog outside reception. Repeat as a new attempt, up to three per run. |
| Pass | A valid attempt meeting every Pass condition of its run. |
| Fail | A valid attempt violating any Pass condition. |

A run with three invalid attempts is `not established` and blocks like `Fail`.

### Live evidence

Filled by the implementer. One row per attempt.

Versions: `Hermes Agent vunknown (2026.9.24)`; `2.1.309 (Claude Code)`; `cafleet 0.31.3`.

| Run | Attempt | Date (UTC) | Receiver handles | Key times (T0/M0, sends, first observations, closures) | Message IDs and ACKs | Receiver samples | Shutdown (`closed` reason, exit, leak check) | Classification |
|---|---|---|---|---|---|---|---|---|
| H-A | 1 | 2026-11-05 | `proc_6136b0f00cbb` (pid 4049835, 06:59:36–08:13:08), then `proc_b00c1e5f3ead` (pid 4071259, 08:13:20–08:21), each `listen 1` with `heartbeat=60`, no `--timeout` | T0 07:01:00; no Director turn 07:00:47–08:02:37; H1 08:01:37 → heartbeat #2 08:02:37; bursts 08:03:41.9–42.4 and 08:06:41.5–42.0; H2 08:10:23 → heartbeat #5 08:10:36; SIGTERM 08:13:08, exit notice 08:13:10, log read + `doctor` + `poll --json` before replacement 08:13:20; H3 08:15:22 → heartbeat 08:16:21; H4 08:18:56 → heartbeat 08:19:22 | ready 1, H1 2, overflow 3–26, H2 27, H3 28, H4 29 each ACKed once and `completed` (bursts ACKed by one loop each, with read-back) | one receiver at 07:31:20, 08:01:32, 08:13:00, 08:14:36, 08:15:17, 08:18:35, 08:20:42 | phase 4 `fleet delete` 08:17:44 exit 1 `fleet 1 pane input remained busy for 5 seconds at member 2; fleet was not deleted.`, fleet listed, no `kill`; phase 5 `fleet delete` 08:21:02, `closed` `fleet_deleted`, exit 0, no replacement, fleet absent; no receiver and no live `processes.json` entry at 08:22:06 | Fail: the 08:07:43 heartbeat-#4 read (offset 15) returned 5 of 12 burst records with `total_lines` 20, firing no overflow signal of the pre-revision rule. `poll --json` still consumed every ID once. |
| H-A | 2 | 2026-11-05 | `proc_545523210e65` (pid 4076971, 08:32:53–09:46:13), then `proc_a02210a19f6b` (pid 4093178, 09:46:28–09:54), each `listen 1` with `heartbeat=60`, no `--timeout`; Hermes skill/memory review disabled | T0 08:35:00; no Director turn 08:34–09:36:57; H1 09:36:09 → heartbeat #2 09:36:57; bursts 09:38:14.2–14.8 and 09:41:14.4–14.9; heartbeat #4 09:41:58 `omitted` 120,736 and read at offset 15 returned 5 of 12 records (`total_lines` 20), then `poll --json` reconciliation and offset reset to 20; H2 09:43:58 → heartbeat #5 09:44:57; SIGTERM 09:46:13, exit notice 09:46:16, log read + `doctor` + `poll --json` before replacement 09:46:28; H3 09:48:15 → heartbeat 09:48:33; H4 09:51:57 → heartbeat 09:52:32 | ready 1, H1 2, overflow 3–26, H2 27, H3 28, H4 29 each ACKed once and `completed` (bursts ACKed by one loop each, with read-back) | one receiver at 08:35:24, 09:05:54, 09:36:04, 09:46:07, 09:48:10, 09:51:51, 09:53:43 | phase 4 `fleet delete` 09:50:27 exit 1 `fleet 1 pane input remained busy for 5 seconds at member 2; fleet was not deleted.`, fleet listed, no `kill`; phase 5 `fleet delete` 09:54:01, `closed` `fleet_deleted`, exit 0, no replacement, fleet absent; no receiver and no live `processes.json` entry at 09:54:59 | Pass |
| H-B | 1 | 2026-11-05 | `proc_961f3f5ab7e4`, `proc_841fd59b1fd6`, `proc_7b217f34f6c6`, `proc_5e8512ccb0f9`, `proc_75fd002cce09`, each `listen 1 --timeout 120` | T0 07:10:00 (heartbeat replay turn end); `closed` `timeout` exit notifications at 07:10:41, 07:13:01, 07:15:17 and 07:17:29, each followed by log read, `poll --json`, `doctor` and one replacement (07:10:56, 07:13:12, 07:15:24, 07:17:40); HB sent 07:16:03, first observed in heartbeat 07:16:28 | ready 1 ACKed once 07:09:37; HB 2 ACKed once 07:16:38, `completed`; no Director `message send` after the ready ACK | one receiver at 07:11:47, 07:14:02, 07:15:57, 07:18:31 | "Delete the fleet." 07:18:43 (submitted 1 s after heartbeat #1 of the fifth receiver and redirected that turn); `fleet delete` 07:18:53, `closed` `fleet_deleted`, exit 0, no replacement; no receiver and no live H-B entry in `processes.json` at 07:19:55 | Pass |
| C-A | 1 | 2026-11-05 | Monitor `b1c32oyps`, then `bj8iu4oww`, each `{"command":"cafleet message listen 1 --timeout 1770","description":"CAFleet Director inbox","timeout_ms":1800000}` | M0 07:04:02; `closed` `timeout` with stream completion (`status` `completed`, "stream ended") 07:33:32.9 = M0 + 1,770.9 s, before the 07:34:02 Monitor deadline, no Monitor timeout; one re-arm 07:33:38 after the completion; C1 sent 07:36:21, first observed as a Monitor event 07:36:22 | ready 1 ACKed once 07:04:22; C1 2 ACKed once 07:36:29, `completed` | one receiver (pid 4061449) and one running Monitor task at 07:35:37 | "Delete the fleet." 07:37:19; `fleet delete` 07:37:26, `closed` `fleet_deleted`, Monitor exit 0, no re-arm; no receiver and no running Monitor at 07:38:31 | Pass |

Selected outcome: `all Pass` (Run H-A attempt 2, Run H-B attempt 1, Run C-A attempt 1); `<hermes run date>` = `2026-11-05`; `<claude run date>` = `2026-11-05`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update `SPEC.md`

- [x] Replace the §2 Director sentence per § Replacement texts. <!-- completed: 2026-11-05T06:51 -->
- [x] Apply the **claude**, **codex** and **hermes** edits in §6.7 Host evidence and native-input authorization per § Replacement texts. <!-- completed: 2026-11-05T06:51 -->

### Step 2: Update `docs/`

- [x] Apply the `docs/docs/mixed-backend-team.md` Receive results and Close the fleet edits. <!-- completed: 2026-11-05T06:52 -->
- [x] Apply the `docs/docs/concepts/overview.md` Watch the team work edit. <!-- completed: 2026-11-05T06:52 -->

### Step 3: Update `skills/cafleet/reference/coding-agents.md`

- [x] Replace the `#### Claude native Director reception` subsection. <!-- completed: 2026-11-05T06:54 -->
- [x] Apply the three § codex edits. <!-- completed: 2026-11-05T06:54 -->
- [x] Replace the `#### Hermes native Director reception` subsection. <!-- completed: 2026-11-05T06:54 -->

### Step 4: Update `skills/cafleet/roles/director.md`

- [x] Replace Spawn Protocol step 7 and Director wake step 1. <!-- completed: 2026-11-05T06:55 -->
- [x] Replace the Receiver recovery `timeout` row response and Shutdown steps 2–3. <!-- completed: 2026-11-05T06:55 -->

### Step 5: Update `skills/cafleet-design-doc`

- [x] Apply the `SKILL.md` Dispatch edit. <!-- completed: 2026-11-05T06:55 -->
- [x] Apply both `create/create.md` edits. <!-- completed: 2026-11-05T06:55 -->
- [x] Replace the `execute/execute.md` Step 8 cleanup paragraph. <!-- completed: 2026-11-05T06:55 -->

### Step 6: Prepare the live environment

- [x] Record `hermes --version`, `claude --version` and `cafleet --version` on the § Live evidence `Versions:` line. <!-- completed: 2026-11-05T06:55 -->
- [x] Create the hidden `probe/` directory beside this design doc and generate the 24 Run H-A phase 2 bodies there. <!-- completed: 2026-11-05T06:55 -->
- [x] For each run, create its isolated database and run `cafleet setup --coding-agent <director backend>` against it per § Live verification. <!-- completed: 2026-11-05T06:55 -->

### Step 7: Run H-A (Hermes fleet lifetime)

- [x] Start the Hermes Director, send the task prompt and run phases 1–5. <!-- completed: 2026-11-05T08:22 -->
- [x] Export the session with `hermes sessions export` and capture the Director pane into `probe/`. <!-- completed: 2026-11-05T08:22 -->
- [x] Fill and classify the Run H-A row; repeat an `Invalid` attempt as a new row, up to three attempts. <!-- completed: 2026-11-05T08:23 -->

### Step 8: Run H-B (Hermes finite lease)

- [x] Start a fresh Hermes Director with the lease clause and run steps 1–3; export the session and capture the pane. <!-- completed: 2026-11-05T07:21 -->
- [x] Fill and classify the Run H-B row; repeat an `Invalid` attempt as a new row, up to three attempts. <!-- completed: 2026-11-05T07:21 -->

### Step 9: Run C-A (Claude Monitor lease)

- [x] Start a fresh Claude Director and run steps 1–4; capture the pane and retain the session transcript. <!-- completed: 2026-11-05T07:39 -->
- [x] Fill and classify the Run C-A row; repeat an `Invalid` attempt as a new row, up to three attempts. <!-- completed: 2026-11-05T07:39 -->

### Step 10: Outcome gate and evidence

- [x] When any run is `Fail` or `not established`, add the Programmer's paired blocker marker in § Live evidence naming the failed condition and its evidence, send `blocked (paragraph-Specification > Live evidence)` and stop until the user decides; otherwise record the selected outcome and run dates. <!-- completed: 2026-11-05T09:56 -->
- [x] Replace the `claude` and `hermes` Director Evidence cells in `SPEC.md` § Role eligibility and `coding-agents.md` per § Replacement texts, substituting versions and run dates. <!-- completed: 2026-11-05T09:56 -->

### Step 11: Verify

- [x] Run the Success Criteria `grep` checks and confirm every `action=kill` placement. <!-- completed: 2026-11-05T10:08 -->
- [x] Confirm every link in the replaced texts (`#director-wake`, `#receiver-recovery`, `#shutdown`, the SPEC `message-listen` anchor) resolves to an existing heading. <!-- completed: 2026-11-05T10:08 -->
- [x] Run `mise //:lint`, `mise //:test` and `mise //docs:build`; confirm `git diff --stat` lists only § Change surface files and this design doc. <!-- completed: 2026-11-05T10:08 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-05 | Initial draft |
| 2026-11-05 | Run H-A attempt 1 showed that rolling-log overflow keeps `total_lines` at the retained count; overflow detection now also uses a nonzero heartbeat `omitted` and heartbeat-turn reads returning fewer records than the heartbeat carried |
