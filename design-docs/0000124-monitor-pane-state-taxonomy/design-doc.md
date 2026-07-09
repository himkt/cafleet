# Monitor Pane-State Taxonomy

**Status**: Complete
**Progress**: 23/23 tasks complete
**Last Updated**: 2026-07-09

## Overview

The monitoring member classifies every due pane on a 2x2 grid (`active|idle` x `progressing|stalled`) that has no cell for "awaiting a user answer", and the monitor loop derives herdr's native `blocked`/`done` signal only to discard it. Replace the grid with a five-state pane-state taxonomy whose classification is capture-content-only, and narrow the loop's native-status trigger so an agent parked on a user prompt is never woken *about*. This closes GitHub issue himkt/cafleet#174.

## Success Criteria

- [x] A pane awaiting a user answer is classified `awaiting_user` and receives no re-engagement primitive from any agent, including when other due agents are `stalled` or `finished`.
- [x] A `blocked` native-status transition no longer flags a wake; a `done` transition still does.
- [x] The monitoring member is woken on a stall-detection cadence independent of the 720 s member interval, and the wake payload tells it which wakes are stall checks.
- [x] A pane whose capture is unchanged across two consecutive stall-check observations, and which is not `awaiting_user`, is classified `stalled` and reported to the Director.
- [x] A pane at a completed turn is classified `finished` and reported to the Director, who alone judges whether assigned work remains.
- [x] The classification rubric is byte-identical across the tmux and herdr backends; native `agent_status` is never consulted as classification evidence.
- [x] No Alembic migration, no new CLI subcommand, and no change to any `Esc` keystroke behavior.

---

## Background

Issue himkt/cafleet#174 states three requirements:

| # | Requirement | Current behavior |
|---|---|---|
| 1 | Coding agent awaiting a user response → do **not** nudge | Unrepresentable in the taxonomy; nudging is destructive |
| 2 | Coding agent stuck mid-execution → **do** nudge | No rubric; one capture per wake, no obligation to compare |
| 3 | Coding agent finished its turn while the task is unfinished → **do** nudge | `supervision.md` says "do not nudge a member just because it went idle" |

Verified against the tree at `80b3f6b5`:

- The 2x2 taxonomy is stated at `skills/cafleet/roles/monitor.md:54` and duplicated verbatim in the wake payload built by `tmux.py:270-276` and `herdr.py:295-301`. Neither has an awaiting-user cell.
- `send_poll_trigger` and `send_inline_preview` both pass `esc_first=True` (`tmux.py:230-243`, `tmux.py:279-307`; herdr's `_send_esc` at `herdr.py:281,324`). The `Esc` is a **deliberate safeguard**, documented at `tmux.py:70-81`: it prevents the trailing `Enter` from blindly *confirming* a permission prompt. It also cancels a prompt the user was about to answer. Both hazards are real; neither may be traded for the other.
- `loop.py:23` flattens `_ATTENTION_STATES = ("blocked", "done")`. `loop.py:164` tags a `status:<state>` wake reason, `loop.py:126-132` echoes it to the launching task's stdout, and `send_wake_trigger` builds its payload from `is_director`/`agent_id`/`name` only — the reason never reaches the monitoring member.
- `AgentStateAware` is herdr-only (`base.py:252-259`). `member capture` returns `{member_agent_id, pane_id, lines, content}` (`cli/member.py:561-571`) and never surfaces `agent_status`.
- `_last_agent_status` (`loop.py:28`) is process-local to the loop and unreachable by the monitoring member, which captures once per wake (`roles/monitor.md:54-61`).

Two facts the issue does not name, established during verification:

**herdr already carries the discriminator.** Native states are `idle`/`working`/`blocked`/`done`/`unknown`. `blocked` ≈ bullet 1 and `done` ≈ bullet 3. `_ATTENTION_STATES` flattens precisely the two states that map to *opposite* actions, one line before the distinction would have been useful.

**The Director is the acute victim of bullet 1, not a member.** Ordinary members run `--permission-mode dontAsk` and auto-approve. The Director is the one agent that legitimately parks awaiting a *user* answer, via `AskUserQuestion`. The monitoring member's own `cafleet member nudge` fires `send_inline_preview(esc_first=True)` into the Director's pane, dismissing that question box. `skills/cafleet/roles/monitor.md:67` advertises this as a feature ("a Director sitting on a permission prompt has it dismissed before the preview's Enter lands"); `skills/cafleet/reference/supervision.md:33` and `docs/concepts/monitoring.md:153-154` carry weaker variants of the same claim.

> Citation correction: the Drafter's round-1 questions and the Director's answer relay both cited this line as `supervision.md:67`. It is `skills/cafleet/roles/monitor.md:67`. The doctrine fix targets all three surfaces listed above.

---

## Specification

### Decision record

| Q | Decision |
|---|---|
| Q1 | Code + docs (full scope) |
| Q2 | Vocabulary designed by the Drafter, justified below |
| Q3 | Policy only — classify first, never keystroke an `awaiting_user` pane; no CLI refusal surface |
| Q4 | `Esc` stays mandatory and in advance; Finding B is a doctrine fix |
| Q5 | Prior observation lives in the monitoring member's own LLM context; no persistence |
| Q6 | Stall detection gets its own cadence; the 720 s default is unchanged |
| Q7 | The Director alone judges task completeness; `pending_count` is not a proxy for it |
| Q8 | Identical capture-content rubric on both backends; native `agent_status` never classifies |

### Resolving the Q1/Q8 contradiction

Q1 authorized "surface `agent_status` through a read primitive". Q8 forbade using native `agent_status` for classification and rejected both concrete read primitives (`member status`, an `agent_status` key on `member capture --json`).

These are reconcilable, and the resolution is not a compromise. The Q1 clause is verbatim boilerplate from the Drafter's own option label, written *before* Q6/Q8 were answered; Q8 is the later, narrower decision on exactly that clause. **The specific overrides the general: this design adds no read primitive.**

They reconcile because `agent_status` has two separable jobs:

| Job | Status | Where |
|---|---|---|
| Decide **when** to wake the watcher | Retained, and **narrowed** | `loop.py::_flag_native_status_due` |
| Decide **what state** a pane is in | Never granted | — classification is capture-content only |

Native status keeps its existing job and gains no new one. Q8 is honored at the rubric level: the mapping from capture → state is byte-identical on both backends.

**The `_ATTENTION_STATES` split is kept, because it becomes a suppression rather than an addition.** `blocked` means the agent awaits a user answer. The monitoring member's *only* actuation is nudging the Director. So waking it because an agent went `blocked` has exactly one correct outcome — capture, classify `awaiting_user`, do nothing. That is a wake whose only correct action is inaction: pure token cost, plus a nonzero chance the monitor misjudges and nudges the Director, whose pane then eats an `Esc`. When the `blocked` agent *is* the Director, that `Esc` destroys the operator's pending `AskUserQuestion`. Removing `blocked` from the wake set closes Finding B **at the source, in code**, while leaving every `Esc` keystroke untouched — exactly what Q3 and Q4 require.

**The `wake_reason` plumbing is kept, because Q6 makes it load-bearing.** Q5 puts the baseline capture in the monitoring member's context and Q6 adds a separate stall cadence; the monitoring member therefore *must* be told which wakes are stall checks, or it cannot know when to compare against that baseline. That is the reason the payload must carry the tag. `status:done` rides along as provenance.

The tag is safe **by construction, not by discipline**: after the split, the only native tag that can ever be emitted is `status:done`, whose worst-case bias on the monitor's judgment is toward `finished` → report to the Director → the Director judges (Q7). That path is non-destructive. The destructive class, `awaiting_user`, can never be *suggested* by a tag, because `blocked` no longer produces one.

Residual, accepted deviation: on herdr the tag `status:done` enters the monitoring member's context and tmux has no equivalent, so Q8's parity holds at the rubric level but not at the context level. The bounded blast radius above is why this is acceptable. **Rule: a wake reason may order attention; it may never substitute for capture evidence.**

### The pane-state taxonomy

Five states. The vocabulary is derived, not chosen: from (a) the monitoring member's actuation set — exactly `{do nothing, report to the Director}` — and (b) what a single pane capture can establish.

| State | Evidence | Monitor action | Issue bullet |
|---|---|---|---|
| `awaiting_user` | Pane shows an unanswered question or permission prompt | **None.** Never re-engage. | 1 |
| `working` | In-flight work, matched by no earlier rule | None | — |
| `stalled` | In-flight work; capture identical to the previous **stall-check** capture of the same pane | Report to the Director | 2 |
| `finished` | A completed turn at an empty input prompt, no pending question | Report to the Director | 3 |
| `unknown` | Dead/unreadable pane, or a required previous stall-check capture is missing | **None.** Fail-safe. | — |

Justification for exactly these:

- `finished_task_complete` / `finished_task_incomplete` is **excluded**. Q7 assigns task-completeness to the Director; the monitoring member cannot see the dispatch ledger, so the distinction is not capture-derivable and must not be guessed. `finished` is one state, always reported.
- `unknown` is **included**. Q5's no-persistence choice makes "I lost my baseline to context compaction" a reachable, expected state. Per `affirmative-writing.md`, absence of evidence maps to a well-defined correct behavior — inaction — not to a guess.
- The old 2x2 maps in as `active`→`working`, `idle`→`finished`, `progressing`/`stalled`→`working`/`stalled`. `awaiting_user` and `unknown` were both unrepresentable.

### Classification rubric (identical on both backends)

Applied in **precedence order**; the first match wins and short-circuits.

1. Capture shows an unanswered question or permission prompt → **`awaiting_user`**. Stop.
2. Pane is dead or unreadable, or this is a stall-check wake and no previous stall-check capture of this pane is remembered → **`unknown`**. Stop.
3. Capture shows a completed turn at an empty input prompt → **`finished`**.
4. This is a stall-check wake, and the capture is identical to the previous **stall-check** capture of this same pane → **`stalled`**.
5. Otherwise → **`working`**.

**The comparand is the previous stall-check capture, and nothing else.** The monitoring member remembers exactly one baseline capture per pane, taken at that pane's last stall-check wake. A capture taken on an `interval` or `status:done` wake is read, classified, and discarded — it never becomes a baseline.

**Baseline maintenance is unconditional, and outside the precedence chain.** On *every* stall-check wake, after classifying, the member replaces that pane's baseline with the capture it just took — whatever state it classified, including `awaiting_user` and `unknown`. The `Stop.` in rules 1 and 2 halts *classification*, never baseline storage. Were storage skipped on an `unknown` wake, the next stall-check would again find no baseline and classify `unknown` again, and `stalled` would be unreachable forever.

This is the only definition that makes `stalled` mean what the success criterion says. Comparing against "the most recent capture from any wake" would let an `interval` wake at t=700 s and a `stall-check` wake at t=720 s declare a stall from a 20-second window — a slow tool call misreported as a hang. The stall window is always one full `monitor_stall_interval`.

The precedence is load-bearing, not cosmetic:

- **`awaiting_user` precedes `stalled`.** A pane parked on a user prompt is byte-identical across observations — exactly the `stalled` signature. Without rule 1 short-circuiting, every awaiting-user pane would be misclassified `stalled` and nudged, which is the destructive outcome the issue exists to prevent.
- **`finished` precedes `stalled`.** A finished pane is also unchanged across observations.
- **`working` is the catch-all and matches last.** It performs no comparison of its own; it is what remains when rules 1–4 do not fire.

**Ambiguity tie-break (invariant).** When the capture cannot distinguish `awaiting_user` from `finished`, classify **`awaiting_user`**. The costs are asymmetric: under-reporting a real `finished` delays a nudge by one wake cycle; misclassifying a real `awaiting_user` destroys the user's pending prompt.

Because native `agent_status` is barred from classification (Q8), the capture-content cues are the *sole* discriminator for the destructive class. The per-backend cue tables in the coding-agent overlays are therefore load-bearing safety artifacts, not documentation garnish.

### Wake-layer changes

```python
# loop.py
_WAKE_ON_STATUS = ("done",)   # replaces _ATTENTION_STATES = ("blocked", "done")
```

- A transition into `done` flags the agent due, tagged `status:done`.
- A transition into `blocked` is **recorded** in `_last_agent_status` (so the episode is tracked and a later recovery is detected) but **never flags a wake**.
- Each due target carries `wake_reasons: list[str]`, ordered and deduped, drawn from `{"interval", "status:done", "stall-check"}`.

Commit gating, mirroring the existing successful-wake gate:

| Ledger | Stamped for | When |
|---|---|---|
| `broker.record_pings` | due agents whose reasons include `interval` or `status:done` | on successful wake |
| `_last_agent_status` | non-flagged reads immediately; flagged reads on successful wake | unchanged |
| `_last_stall_check_at` | due agents whose reasons include `stall-check` | on successful wake |

A stall-check-only agent is **not** passed to `record_pings`. This keeps the 720 s interval cadence and the stall cadence independent, so the `monitor_config.interval_seconds` default of 720 s remains live and unlowered, as Q6 requires.

### Stall-detection cadence

A new setting, no schema change (Q5/Q7 both forbid a migration):

| Field | Env var | Default | Meaning |
|---|---|---|---|
| `monitor_stall_interval` | `CAFLEET_MONITOR_STALL_INTERVAL` | `240` | Seconds between stall-check wakes per watched agent. `0` disables stall detection. |

Tracked in a process-local `_last_stall_check_at: dict[int, datetime]`, cleared per run in `run_monitor_loop` — the same pattern `_last_agent_status` already establishes. Two observations at 240 s means a hang is called in ~8 minutes, against ~24 minutes if bullet 2 rode the 720 s interval.

**First-tick semantics.** An agent absent from `_last_stall_check_at` is stall-check due, mirroring `should_ping`'s existing `last_ping_at is None → due` convention. The dict is **not** pre-seeded.

Two reasons. First, tick 1 already wakes the watcher regardless: the root Director's `last_ping_at` is `None`, so it is interval-due on the very first tick. Flagging the members stall-check due on that same tick therefore costs extra *captures*, not an extra *wake*. Second, it establishes each pane's baseline one full interval earlier, so `stalled` becomes reachable at ~240 s rather than ~480 s. The tick-1 stall-check classifies every pane `unknown` by rule 2 — no baseline exists yet — and takes no action; the fail-safe already specifies exactly that.

### Wake payload

Single line, no backtick, no `$(`, no `|` (the pane may momentarily sit at a shell prompt — the edge `_sanitize_wake_name` already defends). Per-agent reasons render as `[<reasons joined by ",">]`.

```
[monitor] wake: {N} {noun} due — {due_list}. Capture each named pane read-only, with the Director pane ({director_agent_id}) always inspected. From capture content only, classify each pane in this precedence order: awaiting_user, unknown, finished, stalled, working. For an agent tagged stall-check, compare its capture against your previous stall-check capture of that pane, then keep the new capture as that pane's baseline; with no previous stall-check capture, classify unknown. Never re-engage a pane classified awaiting_user: when the Director is awaiting_user, send nothing this wake, whatever the other panes show. Otherwise re-engage the Director via cafleet member nudge when a due agent is stalled or finished, or the Director is finished with un-acked work.
```

A `due_list` entry is `<role> <agent_id> (<sanitized name>) [<reasons>]`, e.g. `member 336 (alice) [interval,stall-check]`.

The enumeration order in the payload is the rubric's precedence order, verbatim. The payload is the instruction the monitoring member actually executes; an enumeration that put the `working` catch-all before `finished`/`stalled` would short-circuit both for every in-flight pane.

### The `Esc` doctrine (Q3 + Q4)

The user's ruling, verbatim: *"esc must be sent in advance"*. `Esc` therefore stays mandatory and precedes the payload on every primitive that already sends it. Protection against destroying a pending user prompt comes entirely from policy — never keystroking an `awaiting_user` pane — never from removing or conditioning the `Esc`.

Concretely, **zero code change to any `Esc` keystroke**:

| Primitive | `esc_first` | Change |
|---|---|---|
| `send_poll_trigger` (`member ping`) | YES | none |
| `send_inline_preview` (`message send` / `broadcast` / `member nudge`) | YES | none |
| `send_wake_trigger` (loop → watcher's own pane) | NO | none — Q3's "Esc stays as-is for every other path" |
| `send_exit`, `send_bash_command` | NO | none |

No `agent_status` probing, no non-zero-exit refusal, no new CLI surface. The safety property is established one layer up: **classify first; never send any re-engagement primitive at a pane classified `awaiting_user`.**

The doctrine fix (per `removal.md` and `affirmative-writing.md`): the three surfaces that advertise the `Esc` dismissal of a Director's prompt as a *feature* must instead state the `Esc`'s actual purpose — preventing the trailing `Enter` from blindly confirming a prompt — paired with the affirmative policy that an `awaiting_user` pane is never keystroked at all. No deprecation notice is left behind.

### Actuation and layering (Q7)

The monitoring member never decides bullet 3. On `finished` it reports to the Director; the Director alone judges whether assigned work remains, against its own dispatch context. No schema change, no ledger.

`pending_count` is **not** used as a proxy for outstanding assigned work. It retains its own correct meaning — the count of un-acked inbound deliveries — which is what makes "the Director is `finished` **and** has un-acked inbox" a legitimate nudge trigger. The two uses are distinct and must not be conflated.

#### The never-re-engage rule outranks every nudge trigger

The Director is the monitoring member's only actuation target, so the Director's own classification gates the entire wake:

> **Invariant.** When the Director's pane is classified `awaiting_user`, the monitoring member performs **no** `cafleet member nudge` on that wake — regardless of how many due agents are `stalled` or `finished`. The bar on re-engaging an `awaiting_user` pane takes precedence over every trigger that would otherwise fire.

This is the exact case the design exists to close. A stalled member plus a Director parked on an `AskUserQuestion` is precisely when the old rubric would have nudged, and that nudge Escs the operator's pending question box (Finding B).

**The suppressed report is not buffered, and is not lost.** Q5 forbids persistence, so nothing is queued; the condition re-surfaces because the underlying pane state persists and the agent stays due on its own cadence:

| Suppressed report | How it re-surfaces |
|---|---|
| A `stalled` member | Still unchanged at its next stall-check wake (≤ `monitor_stall_interval`), so it re-classifies `stalled`. |
| A `finished` member | Still finished at its next `interval` or `stall-check` wake, so it re-classifies `finished`. |

Note the asymmetry that makes this work: the `status:done` **transition** fires only once per episode (`_last_agent_status` records it), so a `done`-triggered wake that is suppressed will not be re-triggered by another `done` transition. Re-surfacing is carried entirely by the interval and stall-check cadences, which are level-triggered on the pane's current state rather than edge-triggered on a transition. Detection is delayed by at most one cadence; it is never dropped.

The same bar applies one layer down, and belongs in the Director's own doctrine: **the Director never `cafleet member ping`s, nor `message send`s to, a member it knows to be `awaiting_user`** — both primitives keystroke `Esc` first (§ *The `Esc` doctrine*). That is bullet 1 for ordinary members.

### Edge cases

| Case | Behavior |
|---|---|
| Monitoring member's context is compacted, losing the baseline | Classify `unknown` on the next stall-check wake (rule 2), take no action, and store that capture as the new baseline. The wake after it can classify `stalled`. Detection is delayed, never wrong. |
| An `awaiting_user` pane is unchanged across stall checks | Rule 1 short-circuits before rule 4. Never `stalled`. |
| A `finished` pane is unchanged across stall checks | Rule 3 precedes rule 4. Never `stalled`. |
| Agent recovers `blocked` → `working` | `_last_agent_status` records the recovery on a no-wake tick, as today; no wake is emitted for either edge. |
| First tick of a run | Every watched agent is stall-check due (`_last_stall_check_at` is empty). Each pane classifies `unknown` by rule 2 and seeds its baseline; no action is taken. The wake itself is not extra — the Director is interval-due on tick 1 regardless. |
| `CAFLEET_MONITOR_STALL_INTERVAL=0` | Stall detection disabled: no `stall-check` tag is ever emitted, so rule 2's stall-check clause and rule 4 never fire and an in-flight pane falls through to rule 5 → **`working`**. `stalled` becomes unreachable, so bullet 2 is not served; bullets 1 and 3 remain fully served. |
| The Director is `awaiting_user` while a member is `stalled`/`finished` | No `member nudge` this wake (§ *The never-re-engage rule outranks every nudge trigger*). The report re-surfaces on the member's next interval or stall-check wake. |
| A long, legitimately-slow tool call | A capture unchanged across two consecutive stall-check observations classifies it `stalled` and reports to the Director — a report, not a keystroke. The Director judges. Reporting a slow-but-healthy agent is the accepted false-positive direction; raising `CAFLEET_MONITOR_STALL_INTERVAL` widens the window. |
| tmux backend | No native status, so no `status:done` tag; agents come due by `interval` and `stall-check` only. The rubric is unchanged, so all three bullets are served with a longer detection latency for bullet 3. |
| Pane dies between the tick's `list_pane_ids` and the capture | `unknown`; no action. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
>
> Steps 1–3 are documentation and MUST land before any code (`.claude/rules/documentation-maintenance.md`).

### Step 1: Concepts and user documentation

- [x] `docs/concepts/monitoring.md` § *Native agent-state due trigger*: rewrite for `_WAKE_ON_STATUS = ("done",)`; state that `blocked` is recorded but never wakes, and why. <!-- completed: 2026-07-09T00:15 -->
- [x] `docs/concepts/monitoring.md` § *The monitoring member* steps 2–4: replace the `active/idle` + `progressing/stalled` judgment with the five-state taxonomy and the precedence rubric; fix the `Esc`-dismissal claim at lines 153–154. <!-- completed: 2026-07-09T00:15 -->
- [x] `docs/concepts/monitoring.md` § *Cadence and tick precision*: add the stall-check interval knob and its env var. <!-- completed: 2026-07-09T00:15 -->

### Step 2: README and SPEC

- [x] `README.md`: update the monitoring description to the five-state taxonomy and the stall cadence. <!-- completed: 2026-07-09T00:18 -->
- [x] `SPEC.md` `send_wake_trigger` contract — **two edit sites**: the tmux contract at ~lines 1753–1764 and the herdr contract at ~line 1938. Replace the payload template with the new single-line text and the `[<reasons>]` due-list entry shape in both, keeping them byte-identical. <!-- completed: 2026-07-09T00:18 -->
- [x] `SPEC.md` configuration section: add `CAFLEET_MONITOR_STALL_INTERVAL` (default `240`, `0` disables) alongside the other `CAFLEET_*` vars. <!-- completed: 2026-07-09T00:18 -->
- [x] `SPEC.md` monitor-loop algorithm section (`Compute the due set` / `Wake the watcher` steps ~2047–2074, § *Native agent-state due trigger* ~2081–2108): rewrite for `_WAKE_ON_STATUS = ("done",)` (only `done` flags a wake; `blocked` is recorded but never flags), per-agent `wake_reasons: list[str]` drawn from `{interval, status:done, stall-check}`, the `status:done`-only stdout suffix, `record_pings` receiving only interval/`status:done` agents (a stall-check-only agent is excluded), and add a § *Stall-detection cadence* covering `_last_stall_check_at`, first-tick due-ness, and `monitor_stall_interval == 0` disabling stall detection. Keep the § *Native agent-state due trigger* subsection accurate to the new behavior. <!-- completed: 2026-07-09T00:24 -->

### Step 3: Skills

- [x] `skills/cafleet/roles/monitor.md`: replace the step-2 2x2 judgment with the five-state taxonomy, the precedence rubric, and the ambiguity tie-break; add the stall-check comparand rule, the unconditional baseline-replacement rule, the `unknown` fail-safe, and the never-re-engage-outranks-every-trigger invariant; update the sample wake nudge at line 74. <!-- completed: 2026-07-09T00:30 -->
- [x] `skills/cafleet/roles/monitor.md:67`: remove the "has it dismissed before the preview's Enter lands" feature claim; state the `Esc`'s real purpose and the never-nudge-`awaiting_user` policy. <!-- completed: 2026-07-09T00:30 -->
- [x] `skills/cafleet/reference/supervision.md`: amend § *Idle Semantics* so a `finished` member with outstanding assigned work IS nudged (bullet 3) while an `awaiting_user` member is never nudged (bullet 1); fix the line-33 `Esc` claim. <!-- completed: 2026-07-09T00:30 -->
- [x] `skills/cafleet/reference/coding-agent/{_template,claude,codex,opencode}.md`: add a *Pane-state capture cues* table per backend giving the concrete `awaiting_user` vs `finished` discriminators; register the note in each overlay's *Note → applies at* table, bound to the rubric's rule 1. <!-- completed: 2026-07-09T00:30 -->

### Step 4: Configuration and monitor loop

- [x] `cafleet/src/cafleet/config.py`: add `monitor_stall_interval: int = Field(default=240, validation_alias="CAFLEET_MONITOR_STALL_INTERVAL")` and document it in the `Settings` docstring. <!-- completed: 2026-07-09T00:52 -->
- [x] `cafleet/src/cafleet/monitor/loop.py`: replace `_ATTENTION_STATES` with `_WAKE_ON_STATUS = ("done",)`; `blocked` is recorded, never flagged. <!-- completed: 2026-07-09T00:52 -->
- [x] `cafleet/src/cafleet/monitor/loop.py`: give each due target `wake_reasons: list[str]`; tag `interval` in `monitor_tick`, `status:done` in `_flag_native_status_due`. <!-- completed: 2026-07-09T00:52 -->
- [x] `cafleet/src/cafleet/monitor/loop.py`: add `_last_stall_check_at` and `_flag_stall_check_due(targets, due, now)`; clear the dict in `run_monitor_loop`; skip entirely when `monitor_stall_interval == 0`. <!-- completed: 2026-07-09T00:52 -->
- [x] `cafleet/src/cafleet/monitor/loop.py`: pass only agents whose reasons include `interval` or `status:done` to `record_pings`; commit `_last_stall_check_at` on successful wake; render joined reasons in the stdout echo. <!-- completed: 2026-07-09T00:52 -->

### Step 5: Multiplexer wake payload

- [x] `cafleet/src/cafleet/multiplexer/tmux.py::send_wake_trigger`: emit the new payload with per-agent `[<reasons>]` tags; keep `esc_first=NO` and the no-backtick / no-`$(` / no-`|` guarantee. <!-- completed: 2026-07-09T00:52 -->
- [x] `cafleet/src/cafleet/multiplexer/herdr.py::send_wake_trigger`: apply the byte-identical payload change. <!-- completed: 2026-07-09T00:52 -->

### Step 6: Tests

- [x] `cafleet/tests/monitor/test_loop.py`: assert a `blocked` transition flags no wake, a `done` transition still does, and a `blocked` read is still recorded in `_last_agent_status`. <!-- completed: 2026-07-09T00:35 -->
- [x] `cafleet/tests/monitor/test_loop.py`: assert stall-check due-ness fires on `monitor_stall_interval`, that a stall-check-only agent is absent from the `record_pings` call, and that `monitor_stall_interval == 0` emits no `stall-check` tag. <!-- completed: 2026-07-09T00:35 -->
- [x] `cafleet/tests/monitor/test_loop.py`: assert `_last_stall_check_at` is committed only on a successful wake, so a failed keystroke re-flags the agent next tick. <!-- completed: 2026-07-09T00:35 -->
- [x] `cafleet/tests/monitor/test_loop.py`: assert first-tick semantics — an agent absent from `_last_stall_check_at` is stall-check due, and the dict is not pre-seeded by `run_monitor_loop`. <!-- completed: 2026-07-09T00:35 -->
- [x] `cafleet/tests/multiplexer/`: pin the new payload text for both backends, assert the two are byte-identical, and assert the payload contains no backtick, no `$(`, and no `|`. <!-- completed: 2026-07-09T00:35 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-08 | Initial draft |
| 2026-07-09 | Implemented via execute workflow: five-state taxonomy, `_WAKE_ON_STATUS = ("done",)`, independent stall-check cadence, byte-identical wake payload. All 23 tasks + 7 Success Criteria complete; Reviewer-approved (2 rounds); PR #176. Status → Complete. |
