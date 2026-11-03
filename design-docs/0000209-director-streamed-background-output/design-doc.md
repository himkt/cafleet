# Director Requirement: Streamed Background Output

**Status**: Approved
**Progress**: 21/21 tasks complete
**Last Updated**: 2026-11-03

## Overview

Replace the Director's "background execution while supervision continues" requirement with "streamed background output while supervision continues": the native background facility must deliver each record a running process emits to the session without waiting for the process to exit. Completion-notified finite receives no longer qualify. Live CAFleet stream probes of every Hermes 2026.9.24 and OpenCode 2.1.4 mechanism that can read a running background process decide their verdicts; the expected Hermes verdict is `no` unless a mechanism passes.

## Success Criteria

- [ ] Every contract home (SPEC.md §6.7, `skills/cafleet/reference/coding-agents.md` § Role requirements, `docs/docs/contributing.md` § Run a CAFleet stream probe, `.claude/rules/documentation-maintenance.md` § Director verdict changes) states the streamed-background-output requirement and the `listen`-only, first-observation probe defined in § Specification, with identical meaning.
- [ ] § Probe evidence records a valid run (or three invalid runs) with an observed result for every mechanism in § Probed mechanisms on Hermes 2026.9.24 and OpenCode 2.1.4.
- [ ] Hermes's Director verdict, evidence and Hermes Director material match the § Probe outcomes branch selected by the recorded results; OpenCode's Director verdict stays `no` with its streamed-background-output evidence taken from the probe.
- [ ] Claude and Codex Director verdicts stay `yes` with their existing mechanisms, versions and dates under the new requirement label.
- [ ] `grep -rn -i "completion-notified\|renewed per completion\|renewed after each completion"` over `SPEC.md docs/docs skills .claude/rules README.md` returns nothing. Under the `no` branch, `grep -rn -i "hermes-native-director-reception\|Hermes native Director reception\|fleet's Director runs on Hermes\|Hermes fleet backend"` over the same paths also returns nothing.
- [ ] Member requirements, Member verdicts and every non-Director Hermes contract are unchanged.
- [ ] `mise //docs:build`, `mise //:test` and `mise //:lint` pass.

---

## Background

The Director supervises members through a broker listener it keeps running in the background while doing other work. That only works when the backend surfaces the listener's output to the session as the listener emits it. Claude Code (Monitor, push) and Codex (retained `exec_command` session read through `write_stdin`, pull) do this.

The user observed that OpenCode and Hermes background jobs deliver their output to the session only when the process exits; Hermes's `terminal` `background=true`, `notify=true` injects one completion notification at exit. Hermes also exposes `process_manage` `poll` and `log` (with `offset`/`limit`) and watch-pattern `notify`, which can read a running background process (`design-docs/0000207-hermes-backend-support/runtime-evidence.md` § 1.4), and their streamed delivery has not been probed. The verdicts therefore follow live probes of every such mechanism.

The current requirement (SPEC.md §6.7 Role requirements) accepts "a completion-notified finite receive that the Director renews after each completion". That allowance made Hermes Director-eligible (verdict `yes`, 2026.9.24, checked 2026-11-02) and would equally admit OpenCode. Director eligibility is documentation-only: no Rust code or bundled JSON encodes it, so this change touches documentation, skills and rules only.

---

## Specification

### Decisions

| Topic | Decision |
|---|---|
| Requirement name | "streamed background output while supervision continues" replaces "background execution while supervision continues" everywhere it names the Director requirement. |
| Incremental delivery | Each record the running process emits reaches the session without waiting for the process to exit, at the latest at the session's next turn boundary. Mid-turn delivery is not required. |
| Push or pull | Delivery counts whether the host pushes records into the session (Claude Monitor) or the model reads the running process's accumulated output on demand (Codex `write_stdin`). |
| Idle wake | Waking an idle session is not required; Codex stays `yes`. |
| Disqualified mechanism | Output that reaches the session only at process exit, including a completion notification for a finite receive renewed after each completion. |
| Probe receiver | `cafleet message listen` only. A finite `cafleet message wait` returns after its first delivery and cannot demonstrate per-record delivery from a running process. |
| Background-facility attribution | Each record must be first observed in the listener's output delivered through the mechanism under test; a record discovered through a foreground `poll`, `wait` or `show` invalidates the run. |
| Verdict source | Hermes and OpenCode verdicts follow live probes of every mechanism that can read a running background process (§ Probed mechanisms); the expected Hermes verdict is `no` unless a mechanism passes. |
| Scope | Director requirement, the CAFleet stream probe, and every backend's Director verdict/evidence. Member requirements and verdicts are unchanged. |
| Claude / Codex evidence | Carried forward unchanged (mechanism, version, date); only the requirement label changes. |
| Hermes `{decision_surface}` | Unchanged (`clarify`; members relay). |
| Hermes selection | Under the `no` branch, `hermes` is selected only when the user names the backend; the "fleet's Director runs on Hermes" path is removed because no eligible Hermes Director exists. The `yes` branch keeps it. |

### Canonical requirement text

SPEC.md §6.7 Role requirements, Director row ("Required backend capability" cell):

> Native goal lifecycle controls, such as a goal command, and streamed background output while supervision continues, established by a CAFleet stream probe. Both capabilities are required.

SPEC.md §6.7 Role requirements, the paragraph that today begins "Background execution while supervision continues is established…", replaced in full:

> Streamed background output while supervision continues is established by a CAFleet stream probe on an installed native version. The candidate Director starts `cafleet message listen` through its native background-execution facility, then continues unrelated work in the same session. The facility must deliver each record the running listener emits to the session without waiting for the process to exit, at the latest at the session's next turn boundary; the host may push records into the session, or the model may read the running process's accumulated output on demand. The probe passes when broker records that another member sends at separate times during that work are each first observed in the listener's output delivered through that facility, and consumed without user input, while the same listener process is still running. A record discovered through a foreground `poll`, `wait` or `show` invalidates the run. Output that reaches the session only when the process exits, including a completion notification for a finite receive, fails the probe. Mid-turn delivery and waking an idle session are not required. Record the native mechanism, version and check date.

SPEC.md §6.7 Role requirements, last sentence of the paragraph beginning "Every launch mode other than the default…", replaced:

> The decision surface and native receive support are evaluated independently of Director eligibility: a Director meeting both requirements may use manual `poll` or finite `wait` when its native adapter is unavailable, and streamed background output establishes no unsolicited wake of an idle session.

`skills/cafleet/reference/coding-agents.md` § Role requirements, the Director sentence (from "Directors need" through "after each completion.") replaced:

> Directors need native goal lifecycle controls and streamed background output while supervision continues, established by a CAFleet stream probe: the candidate starts `cafleet message listen` through its native background facility, keeps working on unrelated tasks, and first observes each of another member's broker records in that listener's output, then consumes it without user input, while the listener is still running, during the work or at the session's next turn boundary, whether the host pushes the records or the model reads the running process's output. A record found through a foreground `poll`, `wait` or `show` invalidates the probe; output that reaches the session only at process exit, such as a completion notification for a finite receive, does not meet the bar; waking an idle session is not required.

The rest of that paragraph (Member requirements, carry-forward and manual `poll`/finite `wait` sentence) is unchanged.

### Probe procedure

`docs/docs/contributing.md` § Run a CAFleet stream probe is the procedure owner. Its intro paragraph is replaced:

> The stream probe establishes a candidate Director's streamed background output while supervision continues. The candidate starts `cafleet message listen` through its native background-execution facility and keeps working on something unrelated in the same session. The probe passes when broker records another member sends at separate times during that work are each first observed in the listener's output delivered through that facility, then consumed without user input, while the same listener is still running, during the work or at the session's next turn boundary. The host may push the records or the Director may read the running process's output on demand. A record discovered through a foreground `poll`, `wait` or `show` invalidates the run. Output that reaches the session only when the process exits, such as a completion notification for a finite receive, fails the probe. Probe each native mechanism that can read a running background process separately. Passing establishes streamed background output; the backend's native reception recipe owns the concrete tool calls.

Its numbered steps change as follows (steps 1–3 unchanged):

| Step | New content |
|---|---|
| 4 | After the Director consumes the ready, have it start `cafleet message listen <director-id> --timeout 300` through the mechanism under test, then send the task prompt below the steps. |
| 5 | Unchanged (three records at 20 s, 60 s and 120 s after the task prompt; record IDs and timestamps). |
| 6 | Classify the run by the run classification below the steps. Capture the Director pane and retain the session records that show each record's first observation. |
| 7 | Record the backend, native version, mechanism (tool call and flags), message IDs, send timestamps, each record's first observation (the tool result or injected event carrying its JSONL line, and its timestamp), ACK timestamps, the listener's exit timestamp, the task-turn end timestamp and the classification in the design doc's evidence record. |

In `contributing.md`, the three blocks below follow step 7 directly, in this order: the task prompt, the run classification table with its lead-in sentence, and the "not established" sentence.

Task prompt for step 4, with `<mechanism instruction>` set to "between steps, read your background listener with `<tool call>`" for a pull mechanism and "consume each record your background listener delivers" for a push mechanism:

> Summarize the files `<file 1>`, `<file 2>`, … one at a time. `<mechanism instruction>`. Learn about broker records only from that listener's output; run no `cafleet message poll`, `wait` or `show`. When a `message` record appears there, ACK its message ID with `cafleet message ack <message-id>` and continue. Keep working until you have ACKed `stream-probe 3` or 240 seconds have passed since this prompt, then end your turn.

Run classification (the 240-second turn bound leaves 60 seconds of the 300-second lease for turn-boundary delivery):

| Classification | Condition |
|---|---|
| Invalid | The task turn ended before 240 s without the Director having ACKed `stream-probe 3`; the task turn was still running when the listener exited; a record was first observed through a foreground `poll`, `wait` or `show`; or user input was given after the task prompt. Repeat the run, up to three runs per mechanism. |
| Pass | A valid run in which every record was first observed in the listener's output delivered through the mechanism under test and ACKed before the listener exited. |
| Fail | A valid run in which any record was first observed only after the listener exited, or was never observed in the listener's output. |

A mechanism with three invalid runs is recorded as "not established" with the invalidating cause.

### Probed mechanisms

| Backend | Mechanism | Kind | Mechanism instruction |
|---|---|---|---|
| Hermes | `terminal` `background=true`, `notify=true` (exit notification) | push | consume each record your background listener delivers |
| Hermes | `terminal` `background=true`, `notify` set to the pattern list `["\"event\":\"message\""]` (watch patterns; at most 8 hits per process) | push | consume each record your background listener delivers |
| Hermes | `terminal` `background=true`, then `process_manage` `action=poll` on the returned `session_id` | pull | between steps, read your background listener with `process_manage` `action=poll` |
| Hermes | `terminal` `background=true`, then `process_manage` `action=log` with `offset`/`limit` on the returned `session_id` | pull | between steps, read your background listener with `process_manage` `action=log`, advancing `offset` past output already read |
| Hermes | Any other running-output facility the installed `terminal` or `process_manage` schema exposes, such as `heartbeat` | per schema | per kind |
| OpenCode | Every tool in the installed 2.1.4 session's tool list that starts a background job, combined with each tool that reads a running job's output; plus the job's exit-time delivery | per tool | per kind |

Hermes background commands run in a login shell that rebuilds `PATH`, so every Hermes listener command uses the absolute path of the `cafleet` executable.

### Probe outcomes

| Backend | Recorded results | Verdict | Branch |
|---|---|---|---|
| Hermes | No mechanism passes | `no` | § Hermes `no` branch |
| Hermes | `process_manage` `poll` or `log` passes | `yes` | § Hermes `yes` branch, using the passing read (`log` when both pass) |
| Hermes | Only another mechanism passes | Undecided | Record the evidence, add the Programmer's paired blocker marker in § Probe evidence and send `blocked (paragraph-Specification > Probe evidence)` for a recipe decision before Step 2. |
| OpenCode | Any results | `no` (goal lifecycle controls are not established) | Evidence per § Verdict rows |

### Verdict rows

SPEC.md § Role eligibility and each backend's `### Role eligibility` table in `coding-agents.md` carry identical Evidence text.

| Backend | Eligible | Evidence |
|---|---|---|
| `claude` | `yes` | Goal lifecycle: [Claude Code goal](https://code.claude.com/docs/en/goal), 2.1.296, checked 2026-10-22. Streamed background output: Monitor stream probes, 2.1.301, checked 2026-10-25. |
| `codex` | `yes` | Goal lifecycle: [Codex goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), 0.162.1, checked 2026-10-22. Streamed background output: retained `exec_command` session stream probes, 0.162.1, checked 2026-10-25. |
| `opencode` | `no` | Goal lifecycle controls: not established for 2.1.4, with no goal evidence recorded, checked 2026-10-31. Streamed background output: `<opencode result>`, 2.1.4, checked `<probe date>`. |
| `hermes` (`no` branch) | `no` | Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Streamed background output: `<outcome label>`, CAFleet stream probes of `<mechanism results>`, 2026.9.24, checked `<probe date>`. |
| `hermes` (`yes` branch) | `yes` | Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Streamed background output: CAFleet stream probe with `terminal` `background=true` read through `process_manage` `action=<read>`, 2026.9.24, checked `<probe date>`. |

Placeholders come from § Probe evidence:

| Placeholder | Value |
|---|---|
| `<probe date>` | The run date of that backend's last probe, `YYYY-MM-DD` |
| `<outcome label>` | `observed failure` when every probed mechanism of that backend has a valid failing run; `not established` when any probed mechanism ended "not established" and none passed |
| `<mechanism results>` | Each probed Hermes mechanism with its outcome in parentheses: the observed failure, or "not established" with the invalidating cause, e.g. "`notify=true` (records only at process exit), `process_manage` `poll` (not established: task turn ended early)" |
| `<opencode result>` | "`<outcome label>`, CAFleet stream probes of `<mechanisms with results>`" (same per-mechanism form as `<mechanism results>`) when no OpenCode mechanism passes; "CAFleet stream probe with `<passing mechanism>`" when one passes |
| `<read>` | The passing `process_manage` action, `log` or `poll` |

### Backend paragraphs (SPEC.md §6.7)

**opencode** — last sentence replaced:

> Goal lifecycle controls are not established for opencode 2.1.4, and the CAFleet stream probes recorded `<opencode result>`, so its Director verdict is `no` (§ Role eligibility).

**hermes** — paragraph replaced in full. The first two sentences are common to both branches:

> **hermes**: Hermes 2026.9.24 ships a user-operated `/goal` command with `status`, `show`, `pause`, `resume`, `clear`, `wait <pid>`, `unwait` and `gate` subcommands, a `terminal` tool whose `background=true` with `notify=true` injects one completion notification into the conversation when the process exits, and `process_manage`, which polls, reads the log of, waits on or kills a background process by its `session_id`. In the classic REPL, leading-`!` input runs a native shell command locally; the Director authorizes each routed command under its host rules.

`no` branch closing sentence:

> The in-session `/help goal` capture establishes goal lifecycle controls, and no CAFleet stream probe of its background mechanisms passed, so its Director verdict is `no` for 2026.9.24 (§ Role eligibility).

`yes` branch closing sentences:

> The in-session `/help goal` capture and the CAFleet stream probe with `process_manage` `action=<read>` establish its Director verdict `yes` for 2026.9.24 (§ Role eligibility). A Director reads its background `listen` receiver through `process_manage` between work steps; CAFleet makes no goal lifecycle calls and preserves activation, pause, completion and gate/wait authority.

The completion-notified reception sentence is removed in both branches.

### Hermes `no` branch (`coding-agents.md` § hermes and other homes)

| Element | Change |
|---|---|
| `#### Hermes native Director reception` subsection | Deleted in full. |
| Note row "A Hermes Director receives broker records through a completion-notified finite `wait` …" | Deleted. |
| Note row "Background terminal commands run through a login shell …" | Applies-at cell becomes `Member startup` (the native reception link is removed). |
| Model catalog sentence "Select `hermes` only when the user names the backend or the fleet's Director runs on Hermes, never by model-name inference." | Becomes "Select `hermes` only when the user names the backend, never by model-name inference." |
| `### Role eligibility` Director row | Per § Verdict rows, `no` branch. |
| `skills/cafleet/roles/director.md` § Model-name-to-backend inference | "Inference never selects `hermes`: a Hermes member requires the operator to name the backend (step 1) or a Hermes fleet backend (step 3)." becomes "Inference never selects `hermes`: a Hermes member requires the operator to name the backend (step 1)." The rest of the paragraph is unchanged. |
| `docs/docs/concepts/coding-agents.md` Hermes model paragraph | "The Director selects `hermes` only when you name the backend or the fleet's Director runs on Hermes, never from a model name." becomes "The Director selects `hermes` only when you name the backend, never from a model name." |

### Hermes `yes` branch (`coding-agents.md` § hermes)

| Element | Change |
|---|---|
| `#### Hermes native Director reception` subsection | Replaced in full by the recipe below. |
| Note row "A Hermes Director receives broker records through a completion-notified finite `wait` …" | Becomes "A Hermes Director reads its background `listen` receiver through `process_manage` `action=<read>` between work steps (§ Hermes native Director reception); records reach the session when the Director reads, and an idle session is woken only by the receiver's exit notification." Applies at: `[Director wake](../roles/director.md#director-wake)`. |
| Note row "Background terminal commands run through a login shell …" | Unchanged. |
| Model catalog sentence, director.md and docs Hermes selection sentences | Unchanged. |
| `### Role eligibility` Director row | Per § Verdict rows, `yes` branch. |

Replacement recipe (`<read call>` is `process_manage` `action=log` with `offset` advanced past output already read, or `process_manage` `action=poll`, per the passing read):

> After registration, start one receiver with the `terminal` tool, `background=true` and `notify=true`, running `<absolute cafleet path> message listen <director-id> --timeout 300` with the literal ID. Background commands run in a login shell that rebuilds `PATH`, so use the absolute path of the cafleet executable your own shell resolves. Retain the returned process `session_id`, the Director ID, the output position already read and unfinished actions.
>
> While delegated work remains, read the running receiver between work steps with `<read call>` on the retained `session_id`. Accumulate incomplete JSONL, consume complete records, reconcile replayed and known IDs with `cafleet message poll <director-id> --json`, record deferred actions and ACK consumption. Continue independent authorized work between reads.
>
> The receiver's exit notification follows its `closed` record or exit: verify termination and follow [receiver recovery](../roles/director.md#receiver-recovery), renewing while delegated work remains. Retire the receiver with `process_manage`, `action=kill` on the retained `session_id`, and verify that it exited. Preserve the process `session_id`, output position, partial JSONL and unfinished actions across compaction. When the background facility is unavailable or denied, disclose that limit and use manual `poll` or a foreground finite `wait`.
>
> Preserve goal activation, pause, completion and gate/wait authority. Reception uses process output and broker reads and makes no goal lifecycle calls or pane input.

### Other homes

| File | Change |
|---|---|
| `docs/docs/concepts/coding-agents.md` § role paragraph (the "Run the root Director on a backend whose Director verdict is `yes`…" paragraph) | Append: "A Director keeps a broker listener running in the background while it works, so its backend must deliver that running process's output to the session as it arrives — at the latest at the session's next turn boundary. A backend whose background jobs surface output only when the process exits runs only as a member; [Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) defines the probe." |
| `.claude/rules/documentation-maintenance.md` § Director verdict changes | "background execution while supervision continues from the CAFleet stream probe defined there" becomes "streamed background output while supervision continues from the CAFleet stream probe defined there". |
| `README.md` | No change; it makes no per-backend Director claim. |

### Probe evidence

Implementation fills one row per mechanism run, adding rows for repeated runs and for mechanisms found by schema or tool-list enumeration:

| Backend and version | Mechanism (tool call and flags) | Run | Listener start | Message IDs and send times | First observation per record (tool result or event, time) | ACK times | Task-turn end | Listener exit | Classification |
|---|---|---|---|---|---|---|---|---|---|
| Hermes 2026.9.24 | `terminal` `background=true`, `notify=true` | 1 | 2026-11-02T23:45:44Z | 2 at 23:46:02Z, 3 at 23:46:42Z, 4 at 23:47:42Z | All three in the exit completion notification, 23:50:44Z | 23:50:54Z, 23:50:58Z, 23:51:02Z | 23:50:20Z | 23:50:44Z | Fail |
| Hermes 2026.9.24 | `terminal` `background=true`, `notify=["\"event\":\"message\""]` (watch pattern) | 1 | 2026-11-02T23:58:58Z | 11 at 23:59:16Z, 12 at 23:59:56Z, 13 at 00:00:56Z | Watch-match notifications queued during the turn, injected after it at 00:03:27Z, 00:03:46Z, 00:03:55Z | 00:03:38Z, 00:03:49Z, 00:03:58Z | 00:03:27Z (271 s) | 00:03:58Z | Unclassified: record 13 ACKed at listener exit; repeated |
| Hermes 2026.9.24 | `terminal` `background=true`, `notify=["\"event\":\"message\""]` (watch pattern) | 2 | 2026-11-03T00:11:34Z | 17 at 00:11:52Z, 18 at 00:12:32Z, 19 at 00:13:32Z | Watch-match notifications injected at the turn boundary, 00:15:56Z, 00:16:03Z, 00:16:14Z | 00:15:59Z, 00:16:09Z, 00:16:20Z | 00:15:56Z (264 s) | 00:16:34Z | Pass |
| Hermes 2026.9.24 | `terminal` `background=true`, then `process_manage` `action=poll` | 1 | 2026-11-02T23:51:48Z | 5 at 23:52:07Z, 6 at 23:52:47Z, 7 at 23:53:47Z | `process_manage` poll results with status `running`, 23:52:29Z, 23:52:49Z, 23:53:52Z | 23:52:31Z, 23:52:55Z, 23:53:58Z | 23:54:16Z (ACKed `stream-probe 3`) | 23:56:48Z | Pass |
| Hermes 2026.9.24 | `terminal` `background=true`, then `process_manage` `action=log` with `offset` | 1 | 2026-11-02T23:55:31Z | 8 at 23:55:50Z, 9 at 23:56:30Z, 10 at 23:57:30Z | `process_manage` log results with status `running`, 23:55:59Z, 23:56:38Z, 23:57:35Z | 23:56:04Z, 23:56:42Z, 23:57:40Z | 23:57:56Z (ACKed `stream-probe 3`) | 00:00:31Z | Pass |
| Hermes 2026.9.24 | `terminal` `background=true`, `heartbeat=60` | 1 | 2026-11-03T00:06:03Z | 14 at 00:06:18Z, 15 at 00:06:58Z, 16 at 00:07:58Z | Heartbeat #2 ("still running", output since last heartbeat) injected at the turn boundary, 00:10:20Z | 00:10:24Z, 00:10:28Z, 00:10:33Z | 00:10:20Z (262 s) | 00:11:03Z | Pass |
| OpenCode 2.1.4 | `shell` `background=true` (completion notification; no tool reads a running job's output) | 1 | 2026-11-03T00:19:52Z | 21 at 00:20:10Z, 22 at 00:20:50Z, 23 at 00:21:50Z | All three in the completion notification after listener exit | 00:24:57Z, 00:24:59Z, 00:25:01Z | 00:24:13Z (263 s) | 00:24:52Z | Fail |

Enumeration: the installed Hermes `terminal` schema exposes `notify` (`true` or a watch-pattern list) and `heartbeat` (seconds, minimum 60, implying completion notification); `process_manage` exposes `list`, `poll`, `log`, `wait`, `kill`, `write`, `submit`, `close` and `handoff`, of which `poll` and `log` read a running process. The OpenCode 2.1.4 session tool list has `shell` (`background`, completion-notified) and `subagent` (`background`, a child agent session rather than a process) and no tool that reads a running job's output. Every listen record uses the `event` key, so the watch pattern matches `"event":"message"`. The probe database lived at `/tmp/p209/d.db` because a database under this design folder makes the member runtime socket path exceed the platform limit; the Hermes session export and run notes stay in the hidden `probe/` directory.

Selected branch: Hermes `yes`, `<read>` = `log`. Hermes `<probe date>` = 2026-11-03; OpenCode `<probe date>` = 2026-11-03, `<outcome label>` = `observed failure`, `<opencode result>` = "observed failure, CAFleet stream probes of `shell` `background=true` (records only at process exit)".

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Run live stream probes

Follow § Probe procedure (new steps 4–7 and the run classification) for every row of § Probed mechanisms, with an isolated `CAFLEET_DATABASE_URL` under this design folder's hidden `probe/` directory, a tmux pane per candidate Director and a probe member on a Member-eligible backend.

- [x] Confirm installed versions with `hermes --version` (expect `2026.9.24`) and `opencode --version` (expect `v2.1.4`); record them in § Probe evidence. <!-- completed: 2026-11-03T00:27 -->
- [x] Enumerate Hermes's running-output facilities from the installed `terminal` and `process_manage` schemas and OpenCode 2.1.4's background-job and job-output tools from its installed help and session tool list; add a § Probe evidence row for each mechanism found. <!-- completed: 2026-11-03T00:27 -->
- [x] Hermes probes: in a `hermes chat --cli` Director session registered with `cafleet fleet create --coding-agent hermes`, run the probe once per Hermes mechanism with `<absolute cafleet path> message listen <director-id> --timeout 300`; fill every field of its row. <!-- completed: 2026-11-03T00:27 -->
- [x] OpenCode probes: in an OpenCode 2.1.4 Director session registered with `cafleet fleet create --coding-agent opencode`, run the probe once per OpenCode mechanism; fill every field of its row. <!-- completed: 2026-11-03T00:27 -->
- [x] Select the branch per § Probe outcomes and record it, with `<probe date>`, each backend's `<outcome label>`, `<mechanism results>`, `<opencode result>` and (for `yes`) `<read>`, below the § Probe evidence table. For the undecided outcome, stop with the blocked report § Probe outcomes specifies. <!-- completed: 2026-11-03T00:27 -->

### Step 2: Update SPEC.md

- [x] §6.7 Role requirements: replace the Director row cell, the background-execution paragraph and the last sentence of the launch-mode paragraph with the text in § Canonical requirement text. <!-- completed: 2026-11-02T23:35 -->
- [x] §6.7 Role eligibility: update all four Director rows per § Verdict rows (the selected Hermes branch), substituting the recorded placeholders. <!-- completed: 2026-11-03T00:37 -->
- [x] §6.7 backend paragraphs: replace the opencode last sentence and the hermes paragraph (common sentences plus the selected branch's closing) per § Backend paragraphs. <!-- completed: 2026-11-03T00:37 -->

### Step 3: Update docs/

- [x] `docs/docs/contributing.md` § Run a CAFleet stream probe: replace the intro paragraph and steps 4, 6 and 7, and add the task prompt, run-classification table and "not established" sentence directly after step 7, per § Probe procedure. <!-- completed: 2026-11-02T23:35 -->
- [x] `docs/docs/concepts/coding-agents.md`: append the Director-backend sentences per § Other homes and, under the `no` branch, edit the Hermes selection sentence per § Hermes `no` branch. <!-- completed: 2026-11-03T00:37 -->

### Step 4: Update skills/

- [x] `skills/cafleet/reference/coding-agents.md` § Role requirements: replace the Director sentence per § Canonical requirement text. <!-- completed: 2026-11-02T23:35 -->
- [x] `coding-agents.md` claude, codex and opencode `### Role eligibility` Director rows: update per § Verdict rows. <!-- completed: 2026-11-03T00:37 -->
- [x] `coding-agents.md` § hermes: apply every `coding-agents.md` row of the selected § Hermes `no` branch or § Hermes `yes` branch table. <!-- completed: 2026-11-03T00:37 -->
- [x] Under the `no` branch, `skills/cafleet/roles/director.md` § Model-name-to-backend inference: edit the Hermes sentence per § Hermes `no` branch. Under the `yes` branch, confirm the file needs no change. <!-- completed: 2026-11-03T00:37 -->

### Step 5: Update project rules

- [x] `.claude/rules/documentation-maintenance.md` § Director verdict changes: apply the wording change per § Other homes. <!-- completed: 2026-11-02T23:35 -->

### Step 6: Verify

- [x] Run the greps from § Success Criteria that apply to the selected branch and confirm they return nothing; also grep for "background execution while supervision continues" and confirm no remaining requirement-name occurrence. <!-- completed: 2026-11-03T00:37 -->
- [x] Confirm SPEC.md § Role eligibility and each backend's Role eligibility table carry identical Director Evidence text, and that no Member row or Member requirement sentence changed (`git diff` review). <!-- completed: 2026-11-03T00:37 -->
- [x] Confirm the SPEC.md, `coding-agents.md` and `contributing.md` requirement texts each carry the first-observation and invalid-run conditions. <!-- completed: 2026-11-03T00:37 -->
- [x] Run `mise //docs:build` and confirm it passes with no broken links (under the `no` branch, the removed `#hermes-native-director-reception` anchor has no remaining referrer). <!-- completed: 2026-11-03T00:37 -->
- [x] Run `mise //:test` and confirm it passes. <!-- completed: 2026-11-03T00:37 -->
- [x] Run `mise //:lint` and confirm it passes. <!-- completed: 2026-11-03T00:37 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-02 | Initial draft |
| 2026-11-02 | Verdicts follow live probes of every running-output mechanism; first-observation and run-validity rules; Hermes `no`/`yes` branches; Hermes Director-wake Note row deleted under `no` |
| 2026-11-02 | Approved |
