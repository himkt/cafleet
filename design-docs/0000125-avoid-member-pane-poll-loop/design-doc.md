# Avoid polling loop on member pane

**Status**: Approved
**Progress**: 2/3 tasks complete
**Last Updated**: 2026-07-09

## Overview

CAFleet members sometimes self-schedule a repeated wait-then-poll cycle (e.g. `sleep NN && cafleet message poll`) to wait for work after going idle, which only burns tokens — the broker's inline preview and the Director's `cafleet member ping` already re-open a member's turn on demand. This is an instruction-only fix: sharpen the member-facing wording so a member ends its turn after an empty poll and relies on those wake channels, banning the self-driven repeat cycle in any form while keeping the legitimate single poll.

## Success Criteria

- [ ] `skills/cafleet/roles/member.md` states affirmatively that a member ends its turn on an empty poll and names the wake channels that re-open it, and explicitly bans a self-scheduled repeated wait-then-poll cycle in any form (with `sleep … && cafleet message poll` as one illustrative example).
- [ ] The legitimate single poll (on wake, or while awaiting a reply just routed to the Director) is preserved — the wording bans the repeated timed loop, not polling itself.
- [ ] `skills/cafleet/SKILL.md § Poll` frames poll as an on-demand inbox check, consistent with the sharpened `member.md`.
- [ ] No code, no `permissions.deny` entry, and no change to `docs/concepts/monitoring.md` or `reference/supervision.md`.

---

## Background

A member's turn ends when it stops emitting; the broker then re-opens it on the next inbound keystroke. The member role file already tells members they are re-woken without self-polling — `member.md:60`: "if you missed an inline preview, your Director re-pokes you via `cafleet member ping`" — and the Director has the parallel *Asynchronous Wait Rule* (`reference/supervision.md`) that says: end the turn, let the wake channels re-open it.

Members lack that same sharpening. The current cue at `member.md:41` — "if the poll is empty, go idle" — is too weak to stop a coding agent from interpreting "wait for work" as an active `sleep && poll` loop. The fix closes that gap with the smallest possible wording change; it is not a new mechanism.

---

## Specification

### The behavioral rule to encode

State the desired behavior directly (affirmative), then the one prohibition it protects:

- **Affirmative**: after an empty poll with no outstanding assigned work, the member **ends its turn and goes genuinely idle**. The broker's inline preview on the next `message send` re-opens the turn when there is work; the Director's `cafleet member ping` re-pokes the member if a preview is missed.
- **Prohibition**: the member **never sets up a repeated wait-then-poll cycle** to wait for work — in **any** form: a `sleep`-then-`poll` sequence (chained, or split across separate Bash calls / turns), a backgrounded sleep, or a self-scheduled wake-up. `sleep … && cafleet message poll` is one illustrative form, not the whole ban — anchoring on that literal string alone would let a member split it into two Bash calls and still run the loop.
- **Preserved (not banned)**: a **single** `cafleet message poll` remains correct when the member has a reason to check now — on wake, or while awaiting a reply it just routed to the Director (e.g. a Bash-denied command dispatched via exec-routing). The banned pattern is the *self-scheduled repeat*, not polling itself.

### Exact edits

Two edits, both wording-only. No behavior in the CLI changes.

| # | File / anchor | Current | Change |
|---|---|---|---|
| 1 | `skills/cafleet/roles/member.md` — the on-spawn poll paragraph (line 41) | "If a task is queued, ACK and process it; if the poll is empty, go idle. The broker keystrokes an inline preview into your pane when the Director sends one, and your next turn picks it up." | Replace with the sharpened paragraph below. |
| 2 | `skills/cafleet/SKILL.md` — `§ Poll (Check Inbox)` (line 128) | "Returns only un-acked (`input_required`) deliveries addressed to this agent, newest first; ACKing one drops it from `poll` output. `--full` emits the untruncated typed-column envelope." | Add one sentence framing poll as an on-demand check, not a self-scheduled timer loop. |

`member.md`'s "Where the IDs come from" section (line 60) already states the recovery is Director-driven ("if you missed an inline preview, your Director re-pokes you via `cafleet member ping`"); edit #1 makes the no-loop rule explicit, so line 60 needs no second statement of it.

**Proposed replacement paragraph for edit #1** (`member.md:41`):

> If a task is queued, ACK and process it. If the poll is empty and no assigned work is outstanding, **end your turn and go idle** — do not keep the turn alive waiting. The broker's inline preview re-opens your turn when the Director sends work, and the Director's `cafleet member ping` re-pokes you if a preview is missed. **Never set up a repeated wait-then-poll cycle to wait for work** — in any form (a `sleep`-then-`poll` sequence, chained or split across turns; a backgrounded sleep; a self-scheduled wake-up). A single `cafleet message poll` when you have a reason to check now — on wake, or while awaiting a reply you just routed to the Director — is fine; a self-scheduled repeat is not.

**Proposed sentence for edit #2** (`SKILL.md § Poll`, appended after the existing description):

> Poll is an on-demand inbox check — run it on wake or when you have a reason to check now, never on a self-scheduled `sleep`-timer loop; the broker re-opens your turn when work arrives.

### Out of scope

- No `permissions.deny` guard or any other runtime enforcement — the fix is wording only.
- No edit to `docs/concepts/monitoring.md` or `reference/supervision.md`; they are monitor/Director-facing and already describe the wake mechanism correctly.
- No change to any CLI behavior, flag, or output.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Sharpen the member-facing idle/poll instruction

- [x] Replace the on-spawn poll paragraph in `skills/cafleet/roles/member.md` (line 41) with the sharpened paragraph from Specification § edit #1. <!-- completed: 2026-07-09T13:47 -->
- [x] Append the on-demand-check sentence to `skills/cafleet/SKILL.md § Poll (Check Inbox)` per edit #2. <!-- completed: 2026-07-09T13:47 -->

### Step 2: Verify consistency

- [ ] Re-read the two edited passages and confirm the affirmative rule reads cleanly, the banned pattern is unambiguous (behavior-anchored, not keyed to the literal `sleep && poll` string), the legitimate single poll is preserved, and no cross-reference (`member.md` ↔ `SKILL.md`) now contradicts another; confirm `docs/concepts/monitoring.md` and `reference/supervision.md` are untouched. <!-- completed: -->
