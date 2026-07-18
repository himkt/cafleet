# Suppress Director Nudges to Busy or Prompt-Parked Members

**Status**: Approved
**Progress**: 11/11 tasks complete
**Last Updated**: 2026-07-18

## Overview

A Director today can `cafleet member ping` or `cafleet message send` a member whose pane is mid-work or parked on a pending user prompt, and the primitive's leading `Esc` keystroke interrupts the work or destroys the prompt (GitHub issue #208). This design adds a **pre-nudge capture gate** to the supervision governance: before any re-engagement nudge, the Director captures the target member's pane and fires the nudge only when the capture shows a `finished` or `stalled` pane, skipping the round otherwise. The change is docs/role-definition only — no runtime behavior changes.

## Success Criteria

- [x] `skills/cafleet/reference/supervision.md` prescribes the pre-nudge capture gate: every Director `cafleet member ping`, every non-exempt `cafleet message send`, and every `cafleet message broadcast` (all recipients) is preceded by a fresh `cafleet member capture` at `--lines 120`, classified on the existing five-state rubric using the target member's backend overlay cues, and fires only on `finished` or `stalled`.
- [x] A target classified `working` or `awaiting_user` has its nudge skipped for that round with the entire send deferred; the deferred send is re-evaluated with a fresh capture on a later facilitation tick, and skipped rounds do not count toward the 2-nudge escalation threshold.
- [x] The monitor-side suppression rule (never re-engage an `awaiting_user` pane; send nothing when the Director's pane is `awaiting_user`) is preserved verbatim in `roles/monitor.md`, `docs/concepts/monitoring.md`, and the wake-nudge payload string.
- [x] All three backend overlays and `_template.md` bind their *Pane-state capture cues* to both consumers: the monitor's classification rubric and the Director's pre-nudge gate.
- [x] No runtime change: the wake-nudge payload strings in `tmux.py` / `herdr.py` and the `cafleet/tests/multiplexer/` suite (`test_tmux.py`, `test_herdr.py`, `test_tmux_send_inline_preview.py`) are verified unchanged.

---

## Background

The `awaiting_user` guard is currently policy-only and one-directional:

- **Monitor → Director** (covered): the monitoring member never re-engages a pane it classified `awaiting_user`, and when the *Director's* pane is `awaiting_user` it sends nothing that wake (`roles/monitor.md` § On each wake, step 4; `docs/concepts/monitoring.md`).
- **Director → member** (the gap): `supervision.md` § Idle Semantics says "never keystroke a member you *know* to be awaiting a user answer" — but the Director has no prescribed way to know. A member the monitor classified `awaiting_user` is silently skipped and never surfaced, while the Stall Response ladder simultaneously tells the Director to nudge quiet members. Both `cafleet member ping` (`send_poll_trigger`) and the `cafleet message send` inline preview (`send_inline_preview`) keystroke `Esc` first, unconditionally (`cafleet/src/cafleet/multiplexer/tmux.py`, `herdr.py`; `broker/messaging.py:49` fires the preview with no pane-state check). The `Esc` cancels a pending prompt on an `awaiting_user` pane and interrupts the in-flight turn on a `working` pane.

No multiplexer backend exposes a native "waiting for user" state to the Director; the only observable is `cafleet member capture` content, which is already the sole classification input of the five-state rubric (`awaiting_user`, `unknown`, `finished`, `stalled`, `working`). The fix therefore extends the same capture-content judgment to the Director side, as governance in the supervision docs.

---

## Specification

### The pre-nudge capture gate

Before firing a gated primitive (table below) at a member, the Director runs a fresh read-only capture of the target and classifies it from capture content only, using the existing five-state rubric and the *Pane-state capture cues* of the **target member's** backend overlay — fleets can mix backends (`member create --coding-agent`), so the Director reads the target's backend from the `backend` column of `cafleet member list` and applies that overlay's cue table, not necessarily its own. The ambiguity tie-break carries over: a capture that cannot distinguish `awaiting_user` from `finished` classifies `awaiting_user`.

```bash
cafleet member capture --fleet-id <fleet-id> --member-id <target-member-id> --lines 120
```

| Capture classifies | Director action |
|---|---|
| `finished` | Fire the nudge/send. |
| `stalled` (quiet, unchanged, no prompt, no in-flight work) | Fire the nudge/send. |
| `awaiting_user` | **Skip this round.** Defer the entire send (nothing persisted, nothing keystroked). Do not relay the pane's prompt anywhere — the round is simply skipped. |
| `working` | **Skip this round.** Defer the entire send. The member will surface its own result via `cafleet message send` when done. |
| `unknown` (dead / unreadable pane) | Do not nudge. Enter the recovery path (`reference/recovery.md`) / escalation ladder instead. |

The gate capture depth is normative: `--lines 120`, matching the monitoring member's on-wake capture depth in `roles/monitor.md`. A § Stall Response Stage-2 capture may double as the gate capture only when it was taken at that depth and is still fresh (same facilitation turn, no intervening keystroke into the pane); the default Stage-2 `--lines 20` capture does not satisfy the gate.

The Director does not maintain stall-check baselines; for the gate, `stalled` means the capture shows a quiet pane with no pending prompt and no in-flight work, in a context where the monitoring member has reported the member stalled or the Director's own prior capture showed the same content. When in doubt between `stalled` and `working`, treat as `working` (skip the round) — a deferred nudge costs one tick; an `Esc` into an in-flight turn destroys work.

The gate is judgment applied at use time: knowledge from a monitor report or an earlier capture is *stale* and never substitutes for the fresh capture immediately before the keystroke.

### Gated vs exempt primitives

| Primitive | Gated? | Rationale |
|---|---|---|
| `cafleet member ping` | **Gated** | Pure re-engagement nudge; its only purpose is waking a resting pane. |
| `cafleet message send` — re-engagement, stall nudge, or new/queued work dispatch | **Gated** | The inline preview keystrokes `Esc` + text + `Enter` into the target pane. |
| `cafleet message broadcast` | **Gated (all recipients)** | Fires the same `Esc`-first preview into every recipient pane, and recipients cannot be skipped individually within one send — so the broadcast fires only when **every** recipient's fresh capture classifies `finished` or `stalled`; otherwise the entire broadcast is deferred (or replaced by per-recipient gated unicasts, the Director's choice). |
| `cafleet message send` — immediate reply to a **reply-soliciting** message (a question or blocker) received from that member in the current facilitation turn | Exempt | The member ended its turn to await this reply; its pane is at rest with no live prompt (existing rule in § Idle Semantics, preserved). A reply to a progress-only anchorless message ("still working", per `coordination.md` § Anchorless Status) is **not** exempt — the member may still be mid-turn — and routes through the gate. |
| `cafleet member exec` | Exempt | Member-requested shell dispatch per `reference/exec-routing.md`; the member is blocked *expecting* this keystroke. |
| Monitor wake nudge (`send_wake_trigger`) | Out of scope | Targets the monitoring member's own pane, fired by the loop, never a watched pane. Unchanged. |
| Monitor → Director `cafleet message send` | Out of scope | Already governed by the monitor's own suppression rule (unchanged, see below). |

**Member → Director sends are deliberately out of scope.** An ordinary member's own `cafleet message send` (e.g. `complete (doc)`) fires the same `Esc`-first preview into a Director pane that may be `awaiting_user`. Members hold no supervisory role and run no facilitation loop, so they cannot defer-and-retry a held send; the accepted residual protections in that direction are the preview's `Esc` safeguard and the monitor-side rule that never re-engages an `awaiting_user` Director. Gating member sends would require member-side supervision machinery this design does not introduce.

### Deferred-send semantics

`cafleet message send` both persists a broker message and fires the destructive inline-preview keystroke; there is no persist-without-keystroke mode and this design adds none. A skipped round therefore defers the **entire send**:

1. The Director holds the deferred instruction as queued work — § Team-facilitation step 3's standing duty ("if a member is idle and inputs are available, send the instruction immediately") already obliges the Director to carry queued work across ticks; Step 2 below adds an explicit "hold deferred sends and re-evaluate each on the next tick" clause to § Stall Response so the deferral is named, not implied.
2. On each subsequent facilitation tick (monitoring-member nudge or inbound broker message re-opening the Director's turn), the Director re-runs the gate — fresh capture, then fire or skip again.
3. No additional wake channel is introduced: the watched member stays on its existing interval and stall-check cadences, so a deferral resolves within at most one member interval once the pane clears.

### Escalation counting

The Stall Response escalation threshold ("still unresponsive after 2 nudges") counts only nudges that actually fired. A skipped round is not a nudge and never advances the count. A member that remains `awaiting_user` or `working` across many rounds is not "unresponsive" — it is parked on the user or making progress, and the Director keeps skipping.

### What stays unchanged (monitor side)

The monitoring member's behavior is untouched, satisfying the issue's first requirement ("monitor should not nudge director if director is waiting user's response") which is already in force:

- The five-state taxonomy, precedence order, capture-content-only rule, and `awaiting_user` tie-break.
- "Never re-engage a pane classified `awaiting_user`"; when the Director's pane is `awaiting_user`, the monitor sends nothing that wake.
- The wake-nudge payload string (`send_wake_trigger` in `tmux.py` / `herdr.py`) and its test assertions — the monitor's on-wake instructions do not change, so the strings do not change.

### Affected surfaces

| Surface | Change |
|---|---|
| `skills/cafleet/reference/supervision.md` | Primary change: the gate, deferred-send semantics, escalation counting (details in Implementation Step 2). |
| `docs/concepts/monitoring.md` | One-paragraph addition: the facilitation layer's re-engagement is capture-gated. |
| `skills/cafleet/reference/coding-agent/{claude,codex,opencode}-overlay.md`, `_template.md` | Extend the *Pane-state capture cues* "applies at" binding to the Director's pre-nudge gate. |
| `skills/cafleet/roles/monitor.md` | No behavioral change; verify cross-references only. |
| `cafleet/src/cafleet/multiplexer/tmux.py`, `herdr.py`, `cafleet/tests/multiplexer/` (`test_tmux.py`, `test_herdr.py`, `test_tmux_send_inline_preview.py`) | Verified unchanged. |
| `SPEC.md`, `README.md`, `docs/spec/multiplexer-backends.md` | No change — no CLI, schema, HTTP, or keystroke-mechanism contract is touched. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Concepts page

- [x] In `docs/concepts/monitoring.md`, extend the facilitation-layer description (§ Heartbeat vs facilitation and the "actuation is Director-only" paragraph in § The monitoring member) with the pre-nudge capture gate: Director re-engagement of a member is itself capture-gated — a fresh `cafleet member capture` classified on the same five-state rubric, firing only on `finished` or `stalled`, skipping the round on `awaiting_user` or `working`. <!-- completed: 2026-07-18T13:10 -->

### Step 2: Supervision reference

- [x] In `supervision.md` § Idle Semantics, replace the know-based prohibition bullet ("never keystroke a member you know to be awaiting a user answer") with the affirmative gate rule per § Specification: capture first (at the normative `--lines 120` depth, using the target member's backend overlay cues), nudge only on `finished` / `stalled`, skip the round on `awaiting_user` / `working`; narrow the reply-exemption bullet to replies to reply-soliciting messages (questions/blockers), routing replies to progress-only messages through the gate; reword the `finished`/`stalled` re-engagement bullets to route through the gate. <!-- completed: 2026-07-18T13:13 -->
- [x] In `supervision.md`, include `cafleet message broadcast` in the gate rule: a broadcast fires only when every recipient's fresh capture classifies `finished` or `stalled`; otherwise defer the entire broadcast or split it into per-recipient gated unicasts. <!-- completed: 2026-07-18T13:13 -->
- [x] In `supervision.md` § How ordinary members are woken, bind the manual-recovery path (path 2: `cafleet member ping` / re-sent instruction) to the gate. <!-- completed: 2026-07-18T13:14 -->
- [x] In `supervision.md` § Team-facilitation instructions step 4, make the capture sub-step (c) the gating precondition for the send sub-step (d): (d) fires only for a member whose (c) capture classified `finished` or `stalled`. <!-- completed: 2026-07-18T13:14 -->
- [x] In `supervision.md` § Stall Response, add the gate as a precondition of every nudge (a Stage-2 capture doubles as the gate capture only when taken at `--lines 120` and still fresh), add the deferred-send semantics (defer the entire send; hold deferred sends and re-evaluate each with a fresh capture on the next facilitation tick), and amend § Escalation to count only fired nudges. <!-- completed: 2026-07-18T13:15 -->
- [x] In `supervision.md` § Quick Reference, annotate the "Message member" and "Manual inbox-poll nudge" rows with the gate precondition ("gated: fresh capture must classify finished/stalled"). <!-- completed: 2026-07-18T13:15 -->

### Step 3: Backend overlays

- [x] In `claude-overlay.md`, extend the *Pane-state capture cues* note's "applies at" binding to include the Director's pre-nudge gate (`supervision.md` § Idle Semantics / § Stall Response), noting the Director applies the cues of the **target member's** backend overlay, alongside the monitor's classification rubric. <!-- completed: 2026-07-18T13:16 -->
- [x] Apply the same "applies at" extension to `codex-overlay.md` and `opencode-overlay.md`. <!-- completed: 2026-07-18T13:16 -->
- [x] In `_template.md`, require the cue-table skeleton to name both consumers (monitor classification + Director pre-nudge gate). <!-- completed: 2026-07-18T13:16 -->

### Step 4: Verification

- [x] Verify `roles/monitor.md` needs no edit (monitor behavior unchanged; its `supervision.md` section anchors — Monitor Lifecycle, Idle Semantics, the 5-step facilitation loop — still resolve after Step 2) and verify the wake-nudge payload strings in `tmux.py` / `herdr.py` and the whole `cafleet/tests/multiplexer/` suite (`test_tmux.py`, `test_herdr.py`, `test_tmux_send_inline_preview.py` — wake payloads, cross-backend byte-identity, `Esc`-first contracts) are byte-identical to before (run `mise //cafleet:test tests/multiplexer/`). <!-- completed: 2026-07-18T13:18 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
| 2026-07-18 | Reviewer round 1: normative gate capture depth (`--lines 120`); target-member backend overlay cues; `message broadcast` gating; reply exemption narrowed to reply-soliciting messages; member→Director scope-out made explicit; deferred-send holding clause; test-verification surface widened to `cafleet/tests/multiplexer/` |
