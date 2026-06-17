# Monitor wake nudge names the due agents

**Status**: Approved
**Progress**: 0/14 tasks complete
**Last Updated**: 2026-06-17

## Overview

The `cafleet monitor` loop computes the exact set of freshly-due watched agents each tick but throws it away when it wakes the monitoring member, so the wake nudge always names only the Director and the monitoring member inspects the Director on every wake — never the member that actually came due. This design threads the freshly-due agents (and the Director id) into `send_wake_trigger` and embeds them in the wake nudge, so the nudge names exactly which agents to inspect this wake, plus the Director as the standing actuation target.

## Success Criteria

- [ ] `monitor_tick` passes the freshly-due agents (the `due` target dicts the loop already builds) **and** the Director id into `send_wake_trigger`; the loop's existing semantics are unchanged (`record_pings` stamps only on a successful wake, a failed keystroke leaves the due agents flagged for retry, nothing is recorded when there is no live watcher).
- [ ] The wake nudge is a **single line** that names every freshly-due agent as `<role> <id> (<name>)` (role `director`/`member`), names the Director id as the standing inspect-and-re-engage target, starts with the `[monitor]` provenance tag, does **not** start with `cafleet`, leads with no `Esc`, contains no embedded CR/LF/tab, and contains no backtick or command-substitution sequence (`$(…)`) — the realistic target is the monitoring member's coding-agent input box, where the parentheses, semicolons, and em-dashes the payload uses for readability are inert.
- [ ] User-controlled agent names are sanitized (CR/LF/tab → U+23CE) so a crafted name cannot break the single-line guarantee.
- [ ] `Multiplexer.send_wake_trigger` (protocol) and `TmuxMultiplexer.send_wake_trigger` take `due_agents` + `director_agent_id`; the stale "Both ping helpers lead with an `Esc` safeguard" claim in `base.py` is corrected to the true post-`0000092` behavior.
- [ ] The monitoring member's on-wake routine (in `skills/cafleet-agent-team-monitoring/SKILL.md` and `docs/concepts/monitoring.md`) inspects exactly the agents the nudge names **plus** the Director; the ambiguous "re-derive the due set from the smallest `last_ping` age" step is removed (`cafleet monitor status` remains available as optional context, not as the source of the due set).
- [ ] `mise //cafleet:lint`, `:format`, `:typecheck`, `:test`, and `mise //admin:build` all pass; docs, `README.md`, and every affected `SKILL.md` read consistently with the new payload, with no residue of the old fixed Director-centric nudge string or the re-derivation framing outside `design-docs/`.

---

## Background

The monitor heartbeat has evolved across several designs. `0000096` (Complete) **inverted enrollment**: the watched set became the root Director (180 s) + every ordinary member (720 s), each on its own interval, and the loop wakes the dedicated **monitoring member** (the unenrolled watcher) whenever ≥ 1 watched agent is due. The monitoring member then inspects the due agents read-only and re-engages the Director.

That inversion is correct, but the wake path has a confirmed bug. `monitor_tick` (`cafleet/src/cafleet/monitor/loop.py:83-112`) builds the `due` list correctly — each entry is a full target dict carrying `agent_id`, `name`, and `is_director` — and uses it for `broker.record_pings(...)` and the per-due-agent stdout echo. But the wake call passes **only the watcher's coordinates**:

```python
woke = mux.send_wake_trigger(
    target_pane_id=watcher["pane_id"],
    fleet_id=fleet_id,
    agent_id=watcher["agent_id"],
)
```

and `send_wake_trigger` (`cafleet/src/cafleet/multiplexer/tmux.py:204-238`) emits a **fixed, Director-centric** single-line payload:

```text
[monitor] wake: run your monitoring routine now — capture the Director pane, judge it active vs idle, and if idle assess the inbox and members and re-engage the Director with an Esc-safeguarded nudge.
```

The computed due set is never communicated to the monitoring member. **Net effect:** every wake tells the monitoring member to inspect the Director; members enrolled at 720 s have their cadence stamped by `record_pings` but are never actually inspected. The user's report — "on every schedule, monitoring target becomes director" — is exactly this discard.

`0000096 §4` papered over the gap by having the monitoring member **re-derive** the due set from "the smallest `last_ping` age" via `cafleet monitor status`. That inference is ambiguous when more than one agent comes due on the same tick — precisely the computation the loop already performed and discarded.

Two adjacent inaccuracies are fixed in the same change:

- `cafleet/src/cafleet/multiplexer/base.py:153-174` — the `Multiplexer.send_wake_trigger` protocol docstring claims "Both ping helpers lead with an `Esc` safeguard." That is false: `0000092 §2` removed the leading `Esc` from the wake nudge (the monitoring member's own pane is never on a permission-approval prompt).

---

## Specification

### 1. The fix at a glance

| Axis | Current (post-`0000096`) | This design |
|---|---|---|
| What the wake call passes | the watcher's `target_pane_id` / `fleet_id` / `agent_id` only | the watcher's `target_pane_id` + the freshly-due agents (`due_agents`) + `director_agent_id` |
| What the wake nudge says | a fixed Director-only string | names each freshly-due agent (`<role> <id> (<name>)`) + the Director id, with the inspect/judge/re-engage instruction |
| How the monitoring member learns which agents to inspect | re-derives from the smallest `last_ping` age (ambiguous on multi-due ticks) | reads the agents named in the nudge (authoritative) |
| What the monitoring member inspects | the Director only | each named due agent **plus** the Director (always) |
| `record_pings` / retry / no-watcher semantics | stamp on success; retry on failure; record nothing with no live watcher | **unchanged** |

The load-bearing change: the due set the loop already computes is **conveyed** to the monitoring member instead of discarded. The monitoring member's *actuation* stays Director-only (it re-engages via `cafleet member nudge`); only the *observation target* is now driven by the nudge's named list.

### 2. The wake-nudge payload

`send_wake_trigger` builds the payload from `due_agents` (the list of due target dicts) and `director_agent_id`. The payload is a single line:

```text
[monitor] wake: {N} {agent|agents} due — {due_list}. Capture each named pane read-only, with the Director pane ({director_id}) always inspected; judge each active/idle and progressing/stalled; re-engage the Director via cafleet member nudge when it is idle with un-acked work or any due agent looks stalled.
```

- `{N}` is `len(due_agents)`; the noun is `agent` when `N == 1`, else `agents`.
- `{due_list}` is the due agents joined by `, `, each rendered as `<role> <id> (<name>)` where `role` is `director` when `target["is_director"]` else `member`. The `<id> (<name>)` core mirrors the loop's existing stdout echo `due agent <id> (<name>)`.
- `{director_id}` is `director_agent_id`, named in **every** payload as the standing inspect-and-re-engage target. The phrasing "with the Director pane ({director_id}) always inspected" reads cleanly whether or not the Director is itself in `{due_list}` (when it is, the clause reaffirms the standing anchor; when it is not, the clause adds it). The nudge is **self-contained** — it does not rely on the spawn prompt's `{director_agent_id}` surviving context compaction.

**Worked example A — the Director (332) and member 336 "alice" are both due:**

```text
[monitor] wake: 2 agents due — director 332 (Director), member 336 (alice). Capture each named pane read-only, with the Director pane (332) always inspected; judge each active/idle and progressing/stalled; re-engage the Director via cafleet member nudge when it is idle with un-acked work or any due agent looks stalled.
```

**Worked example B — only member 336 "alice" is due:**

```text
[monitor] wake: 1 agent due — member 336 (alice). Capture each named pane read-only, with the Director pane (332) always inspected; judge each active/idle and progressing/stalled; re-engage the Director via cafleet member nudge when it is idle with un-acked work or any due agent looks stalled.
```

**Name sanitization.** Agent names are user-controlled, so each `<name>` is passed through a single-line sanitizer that maps `\r\n`, `\n`, `\r`, and `\t` to U+23CE (⏎) before interpolation, preserving the single-line guarantee. This extends the CR/LF cosmetic sanitization that `send_inline_preview` already applies (which neutralizes only CR/LF) with the tab case; the two helpers are deliberately distinct, and `send_inline_preview` is **not** modified (its 2-line contract is pinned by `test_tmux_send_inline_preview.py`).

**Preserved payload constraints** (each pinned by an existing or updated test):

| Constraint | Why | Test |
|---|---|---|
| Single line — no `\n` / `\r` | the nudge is one keystroke turn | `test_send_wake_trigger__payload_is_single_line_monitor_nudge` |
| Starts with `[monitor]`, not `cafleet` | provenance tag; it is an instruction, not a poll command | same |
| No leading `Esc` | the watcher's own pane is never on a permission prompt; an `Esc` would self-interrupt an in-progress routine | `test_send_wake_trigger__return_branches_and_argv` (asserts plain `-l`/`Enter`, no `Escape`) |
| No backtick or command-substitution sequence (`$(…)`) | the payload's only realistic target is the monitoring member's coding-agent input box; backticks and `$(…)` are the genuinely dangerous metacharacters there, so the payload must never contain them | `test_send_wake_trigger__payload_is_single_line_monitor_nudge` (asserts the payload has no backtick and no `$(`) |

The payload deliberately uses parentheses, semicolons, and em-dashes for readability. The wake nudge only ever lands in the monitoring member's coding-agent input box (never at a shell prompt), where those characters are inert, so the documented shell-safety guarantee is narrowed honestly to the one class that would matter even there: no backtick and no `$(…)` command substitution.

### 3. The keystroke helper (`base.py` protocol + `tmux.py`)

`send_wake_trigger`'s signature changes to carry the data the payload needs and to drop the now-vestigial parameters:

```python
def send_wake_trigger(
    self, *, target_pane_id: str, due_agents: list[dict], director_agent_id: int
) -> bool:
```

- **`due_agents`** — the loop's `due` list (each dict carries `agent_id`, `name`, `is_director`, the fields `list_monitor_targets` already returns).
- **`director_agent_id`** — sourced from `get_fleet(...)["director_agent_id"]`, so the Director clause is self-contained.
- **`fleet_id` / `agent_id` are removed.** They were retained only "for signature parity with `send_poll_trigger`" and were never echoed into the wake nudge. The payload's content is now fully determined by `due_agents` + `director_agent_id`, so the parity rationale no longer holds and the parameters are dropped rather than carried dead (per `~/.claude/rules/affirmative-writing.md` and `~/.claude/rules/removal.md`). `send_poll_trigger` keeps `fleet_id` / `agent_id` because it genuinely keystrokes them into the poll command.

`cafleet/src/cafleet/multiplexer/base.py` — the `Multiplexer.send_wake_trigger` protocol method:

- Update the signature to match.
- Rewrite the docstring: the wake nudge **names the freshly-due agents and the Director id**; correct the stale "Both ping helpers lead with an `Esc` safeguard" sentence to state that the wake nudge does **not** lead with `Esc` (only `send_poll_trigger` and the inline preview do, because their targets may be parked on a prompt); note the payload carries no backtick or `$(…)` command substitution, so it is safe in the monitoring member's coding-agent input box.
- Update the `Args:` block to document `due_agents` and `director_agent_id`; drop the `fleet_id` / `agent_id` entries.

`cafleet/src/cafleet/multiplexer/tmux.py` — `TmuxMultiplexer.send_wake_trigger`:

- Update the signature to match.
- Build the §2 payload from `due_agents` + `director_agent_id` (with the name sanitizer); the `shutil.which` guard, the `_send_literal_then_enter` call with no `esc_first`, the `try/except TmuxError → return False`, and the `timeout=5` are unchanged.
- Rewrite the docstring for the new payload and parameters (noting the payload carries no backtick or `$(…)` command substitution); remove the "`fleet_id` / `agent_id` keep the keystroke-helper signature uniform" sentence.

The `_send_literal_then_enter` comment inventory (`tmux.py:66-79`) already lists `send_wake_trigger` as a no-`Esc` helper "(the monitoring member's own read-only pane)" — that remains accurate and is left unchanged.

### 4. The loop (`monitor/loop.py::monitor_tick`)

The wake call conveys the due set and the Director id:

```python
if due and watcher is not None and watcher["pane_id"] in live_panes:
    woke = mux.send_wake_trigger(
        target_pane_id=watcher["pane_id"],
        due_agents=due,
        director_agent_id=fleet["director_agent_id"],
    )
    if woke:
        broker.record_pings([t["agent_id"] for t in due], now.isoformat())
        for target in due:
            click.echo(
                f"{now.isoformat()} due agent {target['agent_id']} "
                f"({target['name']}) -> wake monitor"
            )
```

- `fleet` is already in scope and non-`None` at this point (the `fleet is None or fleet["deleted_at"]` guard above returns `STOP` first), and an active fleet always has a non-null `director_agent_id`, so `fleet["director_agent_id"]` is a direct, safe access.
- **`record_pings`, the stdout echo, the retry-on-failed-wake behavior, and the no-watcher path are all unchanged.** Only the arguments to `send_wake_trigger` change.
- Update the inline comment (the "single best-effort wake nudge" block) and the `monitor_tick` docstring to say the nudge **names** the freshly-due agents and the Director, rather than carrying a fixed string.

### 5. The monitoring member's routine (skill + concepts doc)

Because the nudge is now authoritative, the routine reads the named agents instead of re-deriving them. The two-command on-wake scope from `0000095` (read-only `cafleet member capture` + `cafleet member nudge`) is preserved; only the source of the due set changes.

New "On each wake" routine (replacing the `0000096 §6` step 1 re-derivation):

1. **Read the freshly-due agents named in the wake nudge** — each `<role> <id> (<name>)`. These, plus the Director, are who you inspect this wake. (`cafleet monitor status --fleet-id {fleet_id}` is available as optional context — e.g. to read intervals or pending counts — but it is **not** the source of the due set.)
2. **Capture each named due agent's pane** read-only (`cafleet member capture --member-id <id> --lines 120`) and judge it active/idle and progressing/stalled.
3. **Always also capture the Director's pane** (`cafleet member capture --member-id {director_agent_id} --lines 120`) and classify it ACTIVE vs IDLE — the Director is the only actuation target. (If the Director is itself among the named due agents, step 2 already captured it; step 3 only adds the Director when it is not in the named list.)
4. **Re-engage the Director via `cafleet member nudge`** when the Director is IDLE with un-acked inbox / stalled members, **or** when any named due agent looks stalled — naming what needs attention. If the Director is ACTIVE and no due agent looks stalled, do nothing; end the turn. Never keystroke task instructions into an ordinary member's pane — all member-driving routes back through the Director.

This affects `skills/cafleet-agent-team-monitoring/SKILL.md` — the line-18 wake-nudge paragraph (body prose beginning "Each tick the loop scans the watched set …", **not** the YAML `description:` field at line 3, which reads correctly), the spawn-prompt routine-summary paragraph (≈ lines 48-51, "It opens with a read-only cafleet monitor status schedule query, then keeps every wake within those two member actions."), the "On each wake" preamble (≈ lines 63-66, "your routine opens with a read-only cafleet monitor status schedule query"), the numbered "On each wake" steps (the step-1 `Re-query the watched schedule` re-derivation), and the quoted wake-nudge block — and `docs/concepts/monitoring.md` ("The monitoring member" routine + the wake-nudge description). The two spawn-prompt framings open the routine with a `monitor status` schedule query; under this design the routine opens by **reading the nudge's named agents**, with `monitor status` demoted to optional context — rewriting only the numbered steps and the quoted nudge block would leave this stale framing behind.

### 6. Documentation change surface

Per `.claude/rules/design-doc-numbering.md`, documentation lands **before** code.

| File | Change |
|---|---|
| `docs/concepts/monitoring.md` | The wake-nudge description (≈ line 47-54) — the nudge **names** the freshly-due agents (role + id + name) and the Director id, directing the monitoring member to inspect each named pane plus the Director. "The monitoring member" routine (≈ line 120-133) — replace the "re-query `monitor status` … smallest `last_ping` age" step 1 with "read the agents named in the wake nudge"; steps 2-4 inspect the named due agents + always the Director; keep `monitor status` only as optional context. |
| `docs/spec/cli-options.md` | (a) The `cafleet monitor` wake-nudge sentence (≈ line 933) — the nudge names the freshly-due agents + the Director, not a fixed instruction. (b) The `monitor status` last-ping rationale (≈ line 953, "`last_ping` renders as a human age … smallest age = freshly due") — reframe so `last_ping` ages are optional context, not the monitoring member's source of the due set (which is now the wake nudge). |
| `README.md` | The monitoring summary (line 85) — the loop wakes the monitoring member with a nudge that **names the freshly-due agents**; on each wake the monitoring member inspects each named agent **plus the Director** read-only and re-engages the Director on a stall. Light touch; the high-level framing is otherwise correct. |
| `skills/cafleet-agent-team-monitoring/SKILL.md` | Per §5 — the line-18 wake-nudge paragraph (body prose, not the YAML `description:` field); the spawn-prompt routine-summary paragraph (≈ lines 48-51) and "On each wake" preamble (≈ lines 63-66) that open the routine with a `monitor status` schedule query; the numbered "On each wake" steps (drop the step-1 re-derivation; add the Director-dedupe note); and the quoted wake-nudge block (≈ lines 95-101) replaced with a representative rendering of the new payload plus a note that the count, named agents, and Director id are filled per wake. Preserve the two-command on-wake scope. |
| `skills/cafleet-agent-team-supervision/SKILL.md`, `skills/cafleet/*` | Verify; the new payload content is not quoted there (confirmed by repo search), so no edit is expected — confirm during the removal sweep. |

### 7. Removal — zero residue (`~/.claude/rules/removal.md`)

The new payload fully replaces the old fixed Director-centric string. After this lands, the repo carries no copy of the old nudge text and no re-derivation framing — neither the "re-derive the due set from the smallest `last_ping` age" instruction nor the "opens with a read-only cafleet monitor status schedule query" / "Re-query the watched schedule" spawn-prompt framing — outside `design-docs/`. The Step-1 `git grep` sweep, whose patterns include `monitor status schedule query` and `Re-query the watched schedule` so the spawn-prompt framing is surfaced, confirms it. The `0000096` design doc keeps its re-derivation description as the historical record.

### 8. Tests

| File | Change |
|---|---|
| `cafleet/tests/monitor/test_loop.py` | Update the `fake_wake` stub signature to `(self, *, target_pane_id, due_agents, director_agent_id)`; capture the conveyed due-agent ids + the director id. Assert each wake conveys the correct due set: the due-Director test conveys the Director; the due-member test conveys the member (not the Director) and the correct `director_agent_id`; the multi-due test conveys both. Update the no-wake / failed-wake / `STOP` tests for the new signature — `test_monitor_tick__failed_wake_does_not_advance_or_log` pins the *captured tuple* (`wakes == [("%7", sid, watcher)]`, asserting a wake was attempted), so its content changes to the new capture shape (e.g. `wakes == [("%7", [director_id], director_id)]`), while the no-wake / `STOP` tests assert `wakes == []` and are content-unaffected. The "never keystroke a watched pane" assertions (`polls == []`) are unchanged. |
| `cafleet/tests/multiplexer/test_tmux.py` | `test_send_wake_trigger__return_branches_and_argv` — call with `due_agents=[…]` + `director_agent_id=…`; keep the return-branch, no-`Esc`, and plain `-l`/`Enter` assertions. `test_send_wake_trigger__payload_is_single_line_monitor_nudge` — call with a couple of due agents (one with a name containing a newline and a tab) + a `director_agent_id`; assert the payload is single-line, starts with `[monitor]`, not `cafleet`, **names each due agent** (role + id + name substrings) **and the Director id**, that the crafted name is sanitized (no raw CR/LF/tab survives), and that the payload contains no backtick and no `$(` command-substitution sequence (the narrowed shell-safety guarantee, §2). |
| `cafleet/tests/multiplexer/test_protocol.py` | `test_protocol_declares_send_wake_trigger` and `test_impl_satisfies_protocol` stay green — the `runtime_checkable` parity check is method-presence, and `base.py` + `tmux.py` change in lockstep, so no edit is expected. Confirm during verification. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Run commands via mise full-path tasks from the repo root: `mise //cafleet:test`, `:lint`, `:typecheck`, `:format`, `mise //admin:build`. Package-relative test paths (`tests/...`).

### Step 1: Documentation & skills first (no code)

- [ ] `docs/concepts/monitoring.md`: rewrite the wake-nudge description and "The monitoring member" routine per §5/§6 — the nudge names the freshly-due agents (role + id + name) + the Director; the routine inspects the named due agents plus the Director; remove the "re-query `monitor status` … smallest `last_ping` age" re-derivation (keep `monitor status` as optional context). <!-- completed: -->
- [ ] `docs/spec/cli-options.md`: update the `cafleet monitor` wake-nudge sentence (≈ line 933) — the nudge names the freshly-due agents + the Director — AND reframe the `monitor status` last-ping rationale (≈ line 953) so `last_ping` ages are optional context, not the monitoring member's source of the due set. <!-- completed: -->
- [ ] `README.md`: light-touch the monitoring summary (line 85) — the nudge names the freshly-due agents; the monitoring member inspects each named agent plus the Director and re-engages the Director on a stall. <!-- completed: -->
- [ ] `skills/cafleet-agent-team-monitoring/SKILL.md`: rewrite, per §5, the line-18 wake-nudge paragraph (body prose, not the YAML `description:` field), the spawn-prompt routine-summary paragraph (≈ lines 48-51) and "On each wake" preamble (≈ lines 63-66) that open the routine with a `monitor status` schedule query, the numbered "On each wake" steps (drop the step-1 `Re-query the watched schedule` re-derivation; add the Director-dedupe note), and the quoted wake-nudge block (≈ lines 95-101, a representative rendering of the new payload with a note that the count, named agents, and Director id are filled per wake) — preserve the two-command (`cafleet member capture` + `cafleet member nudge`) on-wake scope. <!-- completed: -->
- [ ] Verify `skills/cafleet-agent-team-supervision/SKILL.md` and `skills/cafleet/*` need no edit (the new payload content is not quoted there). <!-- completed: -->
- [ ] Removal sweep: `git grep -nI -e "capture the Director pane, judge it" -e "smallest .*last.ping.* age" -e "run your monitoring routine now" -e "monitor status schedule query" -e "Re-query the watched schedule"` over the tree; every hit outside `design-docs/` is a removal-rule blocker. <!-- completed: -->

### Step 2: Protocol & keystroke payload

- [ ] `cafleet/src/cafleet/multiplexer/base.py`: update `Multiplexer.send_wake_trigger` to `(*, target_pane_id, due_agents, director_agent_id)`; rewrite the docstring (the nudge names the freshly-due agents + the Director id) and **correct** the stale "Both ping helpers lead with an `Esc` safeguard" claim (the wake nudge does not lead with `Esc`); update the `Args:` block (add `due_agents` / `director_agent_id`, drop `fleet_id` / `agent_id`). <!-- completed: -->
- [ ] `cafleet/src/cafleet/multiplexer/tmux.py`: update `TmuxMultiplexer.send_wake_trigger` to the new signature; build the §2 single-line payload from `due_agents` + `director_agent_id` (with the CR/LF/tab → U+23CE name sanitizer); rewrite the docstring and remove the "`fleet_id` / `agent_id` keep the signature uniform" sentence. Leave `_send_literal_then_enter` and `send_inline_preview` unchanged. <!-- completed: -->

### Step 3: Loop

- [ ] `cafleet/src/cafleet/monitor/loop.py::monitor_tick`: call `send_wake_trigger(target_pane_id=watcher["pane_id"], due_agents=due, director_agent_id=fleet["director_agent_id"])`; update the inline comment and the `monitor_tick` docstring to say the nudge names the freshly-due agents + the Director. `record_pings`, the stdout echo, the retry-on-failure path, and the no-watcher path stay unchanged. <!-- completed: -->

### Step 4: Tests

- [ ] `cafleet/tests/monitor/test_loop.py`: update the `fake_wake` stub to `(self, *, target_pane_id, due_agents, director_agent_id)` and capture the conveyed due-agent ids + director id; assert the due-Director, due-member, and multi-due tests each convey the correct due set and `director_agent_id`; update the no-wake / failed-wake / `STOP` tests for the new signature; keep the `polls == []` (never-keystroke-a-watched-pane) assertions. <!-- completed: -->
- [ ] `cafleet/tests/multiplexer/test_tmux.py`: update `test_send_wake_trigger__return_branches_and_argv` and `test_send_wake_trigger__payload_is_single_line_monitor_nudge` to pass `due_agents` + `director_agent_id`; assert the payload names each due agent (role + id + name) and the Director id, stays single-line / `[monitor]`-prefixed / non-`cafleet` / no-`Esc`, sanitizes a crafted CR/LF/tab name, and contains no backtick and no `$(` command-substitution sequence. <!-- completed: -->
- [ ] `cafleet/tests/multiplexer/test_protocol.py`: confirm `test_protocol_declares_send_wake_trigger` and `test_impl_satisfies_protocol` stay green (no edit expected). <!-- completed: -->

### Step 5: Verification

- [ ] `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test`, and `mise //admin:build` all pass. <!-- completed: -->
- [ ] Final removal sweep confirms no copy of the old fixed Director-centric nudge string and no re-derivation framing ("re-derive from smallest `last_ping` age", "monitor status schedule query", or "Re-query the watched schedule") survives outside `design-docs/`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-17 | Initial draft |
| 2026-06-17 | Reviewer pass: narrowed the payload shell-safety guarantee to no-backtick / no-`$(…)` (keeping the readable `role id (name)` rendering) with a backing test assertion; enumerated the two stale spawn-prompt `monitor status` framings + named the line-18 wake-nudge paragraph; added the Director-dedupe note to the routine; added cli-options.md ≈ line 953 to the change surface; strengthened the removal-sweep patterns; corrected the failed-wake test description; fixed the Progress count. |
| 2026-06-17 | User approved. Status → Approved. |
