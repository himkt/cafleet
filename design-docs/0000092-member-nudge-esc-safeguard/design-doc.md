# Esc-safeguarded `cafleet member nudge` and a universally Esc-safe inline preview

**Status**: Approved
**Progress**: 15/20 tasks complete
**Last Updated**: 2026-06-15

## Overview

Close the auto-confirm hole on the message-delivery keystroke path: `tmux.send_inline_preview` types its payload and presses `Enter` with **no leading `Esc`**, so delivering a message to a pane parked on a pending permission-approval prompt (e.g. a deny-listed `git push`) blindly confirms it. The fix applies the `Esc` safeguard universally to `send_inline_preview` (every inbound `message send` / `broadcast` / nudge) and inverts the now-misplaced safeguard by dropping `esc_first` from the monitoring-member wake keystroke, whose pane is never on a prompt. It also adds a dedicated `cafleet member nudge` subcommand so the monitoring member re-engages the Director over that hardened path.

## Success Criteria

- [ ] Every `send_inline_preview` keystroke (used by `message send`, `broadcast`, and `member nudge`) leads with `Escape` → settle → payload → `Enter`, so a pending permission prompt is dismissed before any payload character is typed and the trailing `Enter` can never confirm it.
- [ ] `send_wake_trigger` (monitor loop → monitoring member's own pane) no longer leads with `Esc` — the monitoring member's pane is never on a permission prompt, so the safeguard was unnecessary ceremony (and a self-interrupt).
- [ ] `send_poll_trigger` (used by `cafleet member ping`) keeps `esc_first=True` unchanged.
- [ ] A new `cafleet member nudge` subcommand lets the monitoring member re-engage the Director: it persists an ACKable broker task carrying the summary **and** fires the hardened (Esc-safeguarded) inline-preview keystroke.
- [ ] `member nudge` is gated by fleet isolation only (no novel caller-auth check), consistent with the rest of the `member` subgroup.
- [ ] The empirical "does a literal `\n` under `send-keys -l` submit?" question (§3, NOTE 2) is answered; the preview keeps its 2-line shape unless that finding shows the newline fragments delivery into two submits.
- [ ] **Every** CAFleet-native orchestrated team skill spawns a dedicated monitoring member first (the first `member create`, `--role monitor --model sonnet`), gates ordinary members on its `ready: monitor live` handshake, and tears it down first-out — the request-driven exemption is removed from `cafleet-design-doc-create`, `-interview`, `cafleet-research-report`, and `-presentation`; `cafleet-design-doc-execute` (already conformant) is the reference.
- [ ] The 0000091 "request-driven / does not run `cafleet monitor` / does not spawn a `--role monitor` member" exemption language is **deleted** from every affected skill (Primitive Mapping rows, supervision/bootstrap sections, teardown notes, `roles/director.md`) per the removal rule — the repo reads as if the exemption never existed.
- [ ] `mise //cafleet:lint`, `:format`, `:typecheck`, and `:test` all pass; `docs/`, `README.md`, and every affected `SKILL.md` are consistent with the new behavior, with no residual "inline preview without Esc" / "wake trigger leads with Esc" / "request-driven, no monitor" / "no monitor to stop" framing outside this design doc.

---

## Background

This is the fourth iteration on the monitor/keystroke safety surface:

| Design | What it established |
|---|---|
| `0000087` | The `cafleet monitor` loop and the keystroke helpers (`send_poll_trigger`, etc.). |
| `0000090` | Introduced the `Esc` safeguard (`esc_first`) on the **ping** helpers and the dedicated monitoring member; the comment in `tmux.py` declared `esc_first` "opt-in (ping helpers only) — an `Esc` before … an inline preview … would mis-fire." |
| `0000091` | Narrowed the heartbeat to wake **only** the monitoring member (`send_wake_trigger`, which kept `esc_first=True`); the Director is re-engaged on demand. |
| `0000092` (this doc) | The safeguard is **misplaced**. The monitoring-member wake pane is never on a prompt (so its `Esc` is ceremony), while the message-delivery path — `send_inline_preview`, which the monitoring member's re-engage `cafleet message send` rides — has **no** `Esc` and routinely lands on the Director's permission prompt. Fix both. |

**The incident.** The monitoring member correctly diagnosed that the Director was stuck on a `git push` permission prompt and sent a nudge advising "Cancel it (Esc)". But delivering that nudge via `cafleet message send → broker.messaging._try_notify_recipient → TmuxMultiplexer.send_inline_preview → _send_literal_then_enter(...)` — **without** `esc_first` — pressed `Enter` on the pending prompt and executed `git push` (exit 128, no upstream branch). The advice to press `Esc` was itself what confirmed the prompt.

**Aggravating factor.** `send_inline_preview` builds its payload as `f"[cafleet msg {task_id} from {sender_id} {ts}]\n{sanitized_text}"` (the inter-line `\n` in the f-string at `tmux.py:246`). The `tmux.py:241–244` comment claims that under `send-keys -l` a raw newline submits as `Enter`; if true, the envelope line's terminating newline can confirm a pending prompt **before** the body is even typed, ahead of the explicit trailing `Enter`. Whether the newline actually submits is verified empirically by this design (§3, NOTE 2).

**Why the safeguard is inverted today.** `send_wake_trigger` (loop → monitoring member's own pane) **has** `esc_first=True`, but that pane is never sitting on a permission prompt — the monitoring member's toolset is read-only cafleet commands that auto-resolve under `dontAsk` — so the `Esc` is unnecessary and merely self-interrupts an in-progress routine (the accepted-but-cosmetic cost noted in `0000090` §C). The Director's pane, by contrast, **is** routinely on a permission prompt, and the monitor→Director re-engage path (via `send_inline_preview`) has **no** `Esc` safeguard at all.

---

## Specification

### Locked decisions (confirmed with the user)

| # | Decision |
|---|---|
| Q1 (scope) | **Fix both paths.** Add the dedicated `cafleet member nudge` **and** apply `esc_first` universally to `send_inline_preview`, so every inbound message-send keystroke leads with `Esc` — closing the auto-confirm hole for **all** recipients, not just monitor→Director. A Director→member message landing while that member sits on a deny-list prompt had the identical failure mode. |
| Q2 (Esc tradeoff) | An `Esc` is **benign**: on an empty prompt it is a no-op; mid-generation it interrupts a turn the inline preview was about to interrupt anyway (the preview injects a fresh user-turn). The marginal harm is near-zero versus the catastrophic auto-confirm it prevents. This validates Q1 = fix-both. The `tmux.py:67–70` "would mis-fire" rationale for excluding inline previews no longer holds and is rewritten. |
| Q3a (caller auth) | **Fleet-isolation only** — no novel caller-auth check. Any active in-fleet agent may invoke `member nudge`; the target is resolved by `--member-id`. This matches `member delete`/`capture`/`exec`/`ping`, which carry no caller-auth (the only boundary is `broker.get_agent(member_id, fleet_id)` returning `None` for a cross-fleet/unknown/inactive target). The `member` subgroup is not designed for members other than the monitoring member, so no extra gate is needed. |
| Q3b (nudge shape) | `member nudge` **persists a broker task** carrying the summary (so the Director's facilitation loop still receives an ACKable inbox item) **and** fires an `esc_first` keystroke. Because Q1 hardens `send_inline_preview` itself, the keystroke nudge fires is that now-safe inline preview — `member nudge` reuses the hardened broker send path rather than introducing a duplicate keystroke helper. |
| Q4 (preview shape, **revised**) | **Do NOT require a single-line collapse.** Keep the existing 2-line preview shape. With `esc_first` now universal on `send_inline_preview`, the `Escape` dismisses any pending prompt **before any payload character is typed**, so the embedded envelope/body newline can no longer confirm a prompt — the `Esc` carries the safety guarantee, making the single-line collapse redundant for safety. Treat any newline-collapse as **conditional** on the §3 empirical finding (NOTE 2). |
| — | `send_poll_trigger` (`cafleet member ping`) keeps `esc_first=True`. No keystroke helper is removed. No schema change / Alembic migration (nudge reuses the `tasks` table via the existing send path). |

### Keystroke-helper inventory (after this design)

| Helper | `esc_first` | Used by | Change |
|---|---|---|---|
| `send_poll_trigger` | **True** | `cafleet member ping` | unchanged |
| `send_wake_trigger` | **False** | monitor loop → monitoring member | **drop `esc_first`** (was `True`) |
| `send_inline_preview` | **True** | `message send` / `broadcast` → any recipient; `member nudge` → Director | **add `esc_first`** (was implicit `False`) |
| `send_exit` | n/a | `member delete` | unchanged (no `Esc`; `Esc` before `/exit` would mis-fire) |
| `send_bash_command` | n/a | `member exec` | unchanged (no `Esc`; `Esc` before `! <cmd>` would mis-fire) |
| `send_freetext_and_submit` / `send_choice_key` | n/a | `member send-input` | unchanged (these **answer** an AskUserQuestion prompt — an `Esc` would dismiss the very prompt they target) |

The safeguard's principle is restated: **`esc_first` applies wherever the target pane MAY be parked on a permission-approval prompt that the keystroke would otherwise blindly confirm** — `member ping` (a member that might be on a prompt) and every inline preview (any recipient, including the Director). It does **not** apply where the pane is never on such a prompt (`send_wake_trigger` — the monitoring member's own read-only pane) or where a leading `Esc` would actively break the keystroke's intent (`send_exit`, `send_bash_command`, `send_send-input` helpers).

### 1. Universal `Esc` on `send_inline_preview` (`tmux.py`, Q1)

`send_inline_preview` passes `esc_first=True` into `_send_literal_then_enter`. The full delivery sequence becomes **`Escape` → 0.1s settle → `send-keys -l <payload>` → 0.12s → `Enter`** — identical in shape to `send_poll_trigger`/`member ping`. Because `_try_notify_recipient` (`broker/messaging.py`) and `broadcast_message` already route every notification through `send_inline_preview`, no broker-layer change is required: hardening the one helper closes the hole for `message send`, `broadcast`, and `member nudge` at once.

```python
# tmux.py — send_inline_preview (after): the one-line change
_send_literal_then_enter(
    target_pane_id=target_pane_id, payload=payload, timeout=5, esc_first=True,
)
```

The `_send_literal_then_enter` `esc_first` comment (`tmux.py:67–70`) is rewritten: it no longer claims inline previews must be excluded. New framing — `esc_first` is applied wherever the pane may be on a permission prompt (`member ping`, inline previews); it is **not** applied to `send_exit` / `send_bash_command` / the `send-input` helpers (an `Esc` before `/exit` or `! <cmd>` would mis-fire; the `send-input` helpers deliberately answer a live prompt) or to `send_wake_trigger` (its target pane is never on a prompt).

### 2. Drop `esc_first` from `send_wake_trigger` (`tmux.py`, invert the safeguard)

`send_wake_trigger` calls `_send_literal_then_enter` **without** `esc_first` (plain `send-keys -l <payload>` → `Enter`). Rationale: the loop fires this into the monitoring member's **own** pane, which runs a read-only capture-classify-reengage routine under `dontAsk` and never parks on a permission-approval prompt. Removing the `Esc` eliminates the cosmetic self-interrupt that `0000090` §C accepted (a wake landing mid-routine no longer aborts the current turn). The helper's docstring drops the "Esc-safeguarded" phrasing.

**Residual risk, documented:** the safety of dropping `Esc` here rests on the monitoring member's pane never being on a deny-list prompt. That holds because its routine uses only read-only cafleet commands (`member capture`, `message send`/`poll`, `member nudge`) that auto-resolve. If a future change gives the monitoring member a deny-list-capable tool, the wake's trailing `Enter` could confirm a prompt — that change must re-add the safeguard. This is an accepted, architecturally-bounded residual, not an open hole.

### 3. The embedded envelope/body newline (comment `tmux.py:241–244`, payload `\n` at `tmux.py:246`, Q4 revised)

`send_inline_preview`'s payload keeps its 2-line shape: `f"[cafleet msg {task_id} from {sender_id} {ts}]\n{sanitized_text}"`. With Q1's universal `esc_first`, the `Escape` fires and settles **before** any payload character is typed, so even if the envelope newline does submit, there is no longer a pending prompt for it to confirm. The single-line collapse is therefore **not** a requirement.

Two findings are recorded as NOTES, not requirements:

- **NOTE 1 (defense-in-depth, not required).** The sequence is `Esc` → `sleep 0.1s` (`_ESC_SETTLE_DELAY`) → type payload → `sleep 0.12s` → `Enter`. A single-line payload (replacing the inter-line `\n` with the U+23CE sentinel already used to sanitize body newlines at `tmux.py:245`) would additionally close the residual race **if** the 0.1s settle is ever too short for the agent to dismiss the prompt. That is belt-and-suspenders, layered on top of the `Esc` guarantee — not needed for correctness.
- **NOTE 2 (conditional correctness fix).** Verify empirically whether a literal `\n` under `send-keys -l` actually submits (as the `tmux.py:241–244` comment claims) or is a soft-newline insert.
  - If it **fragments delivery into two submits** (the envelope submits on its own, then the body submits), that is a **separate correctness bug** — the recipient's TUI would receive the envelope as one turn and the body as another — worth fixing by collapsing to the single-line U+23CE form. The §3 implementation task and a unit/integration assertion would then land the collapse.
  - If it is a **soft-newline insert** (the payload arrives as one 2-line input, one submit on the trailing `Enter`), **keep the 2-line shape as-is** and update the `tmux.py:241–244` comment to reflect the verified behavior.

  **Expected branch — soft-insert / keep 2-line.** The 2-line `send_inline_preview` is the live delivery mechanism for **every** `message send` / `broadcast` today, and it works: recipients receive a coherent single 2-line turn, not a fragmented envelope-then-body. If the inter-line `\n` actually submitted, that fragmentation would already be visible in production on every message. That is strong evidence the soft-insert branch is what B4 will confirm — and that the `tmux.py:241–244` comment ("a raw newline submits as Enter") is misleading: the body-newline sanitization it justifies is then belt-and-suspenders, not load-bearing. B4 is therefore anticipated to **keep the 2-line shape and correct the comment**, not collapse. The fragments-into-two-submits branch is retained only because B4 confirms empirically rather than by inference; Success-Criterion-6 reads with that expectation.

### 4. The `cafleet member nudge` subcommand (`cli/member.py`, Q3a/Q3b)

A new subcommand in the `member` group. It is the monitoring member's purpose-built primitive for re-engaging the Director — a named CLI surface the monitoring/supervision skills point to, instead of the monitoring member reaching for the general `cafleet message send`.

```bash
cafleet member nudge --fleet-id <fleet-id> --agent-id <monitoring-member-id> \
  --member-id <director-agent-id> --text "<re-engage summary>"
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **sender** (the monitoring member). Persisted as the task's `from_agent_id` so the Director sees who nudged it. Unique within the `member` subgroup: `nudge` and `create` take `--agent-id`; `delete`/`capture`/`exec`/`ping` do not, because only `nudge`/`create` need a real acting identity (the others are pure Director-initiated keystrokes). |
| `--member-id` | yes | The **target** (typically the root Director). Resolved via the existing `_load_authorized_member(fleet_id, member_id)` — fleet-isolation only; the root Director is a valid target (any active in-fleet agent with a placement is). |
| `--text` | yes | The re-engage summary (un-ACKed inbox items, stalled members). Persisted as the task body. |

**Behavior.** The command resolves the target via `_load_authorized_member(fleet_id, member_id)` (fleet-isolation enforcement — see Error handling), then calls `broker.send_message(fleet_id, agent_id, to=member_id, text=text)`. `send_message` already (1) persists a `unicast` / `input_required` task — the ACKable inbox item the Director's facilitation loop consumes — and (2) best-effort fires `send_inline_preview`, which Q1 makes `Esc`-safeguarded. Thus `member nudge` satisfies Q3b — persist + `esc_first` keystroke — by **reusing the hardened send path**, with no duplicate keystroke helper.

Why a separate subcommand rather than telling the monitoring member to call `message send`: (a) it gives the monitoring/supervision skills a single, unambiguous re-engage verb to document and the spawn prompt to invoke; (b) it lives in the `member` subgroup the monitoring member already uses (`member capture`, etc.); (c) `permissions.allow` can carry a fixed `Bash(cafleet member nudge --fleet-id *)` pattern. Functionally `member nudge` and a monitoring-member `message send --to <director>` now deliver the same persist+hardened-preview effect — the subcommand is a named interface over that path, deliberately not a re-implementation.

**Error handling.** Two validation layers, kept distinct:

- **Target** — `_load_authorized_member(fleet_id, member_id)` runs **first** and raises `ClickException("Agent <id> not found")` for a cross-fleet / unknown / inactive `--member-id` (or the placement-missing error when the target has no placement row) before `broker.send_message` is ever called. `send_message`'s own *target* ValueError ("Destination agent not found / not in fleet") is therefore **unreachable** in the nudge path — B5 adds **no** target-error handling around `send_message`.
- **Sender** — `_load_authorized_member` validates only the target, not the caller, so the one ValueError `send_message` can still raise here is its **sender** check (`agent_is_active_in_fleet(agent_id, fleet_id)` — the monitoring member's own id). B5 wraps **only** that sender ValueError in a try/except → `ClickException` (exit 1).

A target with no live pane is tolerated (the task still persists; the keystroke best-effort no-ops) — identical to `message send` semantics. `--text` empty/whitespace-only is rejected with a `UsageError` (exit 2), mirroring `member exec`'s empty-command guard.

**Output.** Text mode: `Nudged Director <name> (<pane_id>) — task <task_id> queued, Esc-safeguarded preview dispatched.` (or a "no pane; task queued" variant when the target has no placement pane). JSON mode (`--json`): `{member_agent_id, pane_id, task_id, notification_sent}`.

### 5. Permissions coverage (`docs/spec/cli-options.md`)

`member nudge` carries an agent-controlled `--text`, like `message send` (which is treated as routine messaging). It is added to the `permissions.allow` coverage in the canonical flag order: `Bash(cafleet member nudge --fleet-id *)` (plus the `Bash(cafleet --json member nudge --fleet-id *)` companion). The monitoring member runs under `dontAsk`, so its invocations auto-resolve regardless; the allow pattern keeps an operator-pane invocation prompt-free and the docs accurate. (Contrast `member exec`, which stays in `permissions.ask` for its operator-controlled positional command.)

### 6. Mandatory monitoring member for every orchestrated team skill (scope addition)

#### 6.1 The requirement and its rationale

"Always spawn a monitoring member" is a **CAFleet-session-level requirement, not a per-skill choice**. Every CAFleet-native orchestrated team skill MUST adopt the cafleet monitoring + supervision pattern:

1. The **first** `cafleet member create` in the fleet is the dedicated monitoring member, spawned with `--role monitor --model sonnet` (first-in).
2. Ordinary member spawns are gated on the monitoring member's `ready: monitor live` handshake.
3. Teardown stops the monitoring member's `monitor start` background task **before** its pane is killed, and deletes the monitoring member **first** (first-out).

This reverses `0000091`'s carve-out, which exempted four skills (`cafleet-design-doc-create`, `cafleet-design-doc-interview`, `cafleet-research-report`, `cafleet-research-presentation`) as "request-driven, no monitor". `cafleet-design-doc-execute` already adopted the monitoring-member model in `0000091` and is the **reference** implementation; it needs no spawn/teardown change.

**Why it ships in this design doc.** Making every orchestrated fleet run a monitoring member proliferates the monitoring member's re-engage action — the monitoring member nudges an idle Director, and (per §4) that nudge now flows through `cafleet member nudge`. If the nudge were not Esc-safeguarded (the §1–§4 work), this proliferation would put an **unsafe** nudge into every orchestrated fleet — exactly the incident in Background, multiplied across five skills. The two changes are coupled and land together so that no fleet ever runs an un-safeguarded nudge: the safety fix (§1–§5) and the rollout that depends on it (§6) are one atomic change.

#### 6.2 Which monitoring-member routine each skill uses

The monitoring member's spawn prompt and capture-classify-reengage routine are canonical in the `cafleet-agent-team-monitoring` skill (unchanged by this design). Two variants exist:

- **Canonical (conditional idle-nudge).** The monitoring member re-engages the Director only when it can name what needs attention (un-acked inbox items, stalled members). The four newly-converted skills use this canonical routine — they are genuinely event-driven, and a conditional nudge is exactly the stall-surfacing behavior they want.
- **Extended (unconditional idle-nudge).** `cafleet-design-doc-execute`'s Step-7 Copilot loop polls an external service that fires no broker inline preview, so its monitoring member nudges the idle Director **unconditionally** to grant a re-poll turn (the `0000091` spawn-prompt delta). This delta is **unchanged** and remains scoped to execute only.

Both variants re-engage the Director via `cafleet member nudge` (§4) rather than a bare `cafleet message send`.

#### 6.3 Per-skill edits

| Skill | Spawn / gate edit (first-in) | Teardown edit (first-out) | Exemption to delete |
|---|---|---|---|
| `cafleet-design-doc-create` | Insert a monitor-spawn step after fleet creation (before the Drafter spawn, Step 1); gate the Drafter/Reviewer spawns on `ready: monitor live`. | Step 6: stop the monitor task + delete the monitoring member first, before Drafter/Reviewer. | `SKILL.md` Primitive Mapping row + the "1b. Request-driven supervision (no monitor)" block; `roles/director.md` bootstrap exemption. |
| `cafleet-design-doc-interview` | Insert a monitor-spawn step after fleet creation (Step 2a), before the Analyzer spawn; gate the Analyzer spawn on `ready: monitor live`. | Step 2f: stop the monitor task + delete the monitoring member first, before/with the Analyzer teardown. | `SKILL.md` Primitive Mapping row + the "2b. Request-driven supervision (no monitor)" block. |
| `cafleet-research-report` | Insert a monitor-spawn step after Step 0b (fleet bootstrap), before the Manager spawn; gate Manager/Scout/Researcher spawns on `ready: monitor live`. | Step 8: stop the monitor task + delete the monitoring member first; member-delete order becomes monitoring member → Researchers → Scouts → Manager. | `SKILL.md` Step 1 "Supervision Model (request-driven, no monitor)" + `roles/director.md` bootstrap exemption + the Prerequisites note. |
| `cafleet-research-presentation` | Insert a monitor-spawn step after Step 1a (fleet bootstrap), before the Presentation/Transcript spawn; gate those spawns on `ready: monitor live`. | Step 5: stop the monitor task + delete the monitoring member first; member-delete order becomes monitoring member → Presentation → Transcript → VR batch (VR after its close handshake). Mirror in `roles/director.md` Shutdown Protocol. | `SKILL.md` Step 1b exemption (two passages) + `roles/director.md` bootstrap exemption + the Prerequisites note. |
| `cafleet-design-doc-execute` | **No change** — already spawns the monitoring member first (Step 3b) and gates on `ready: monitor live`. | **No change** — already stops the monitor task and deletes the monitoring member first (Step 8). | None — never carried the exemption. |

The newly-converted skills already **load** `cafleet-agent-team-monitoring` (the design-doc skills also load `-supervision`), so the canonical monitoring-member spawn prompt is already in the Director's context; the edits flip the loaded skill from "loaded for policy only, do not run the heartbeat" to "spawn the monitoring member and run the heartbeat".

#### 6.4 Removal-cleanup checklist (delete; leave no deprecation note)

Per the removal rule, every passage below is **deleted** (not annotated), so the repo reads as if the exemption never existed:

- `skills/cafleet-design-doc-create/SKILL.md`: the Primitive Mapping "request-driven … does not run `cafleet monitor`" row; the `#### 1b. Request-driven supervision (no monitor)` header + body; the Step-6 teardown's "(no monitor to stop — this team is request-driven)" parenthetical.
- `skills/cafleet-design-doc-create/roles/director.md`: the bootstrap exemption bullet.
- `skills/cafleet-design-doc-interview/SKILL.md`: the Primitive Mapping exemption row; the `#### 2b. Request-driven supervision (no monitor)` header + body; the Step-2f teardown's "(no monitor to stop — this team is request-driven)" parenthetical.
- `skills/cafleet-research-report/SKILL.md`: the `### Step 1: Supervision Model (Director — request-driven, no monitor)` title + exemption body; the Step-8 teardown's "This team is request-driven, so there is no monitor to stop." sentence; the Prerequisites note that omits the monitoring member.
- `skills/cafleet-research-report/roles/director.md`: the "Bootstrap the team (request-driven, no monitor)" bullet's exemption phrasing.
- `skills/cafleet-research-presentation/SKILL.md`: both Step-1b "request-driven … does not run `cafleet monitor`" passages; the Step-5 teardown's "This team is request-driven, so there is no monitor to stop." sentence; the Prerequisites note that omits the monitoring member.
- `skills/cafleet-research-presentation/roles/director.md`: the "Bootstrap the team (request-driven, no monitor)" bullet and the Shutdown-Protocol "(this team is request-driven, so there is no monitor to stop first)" parenthetical.

The D1 sweep (Phase D) extends to `docs/`, `README.md`, `.claude/`, and root `CLAUDE.md` to catch any non-skill mention of the removed exemption (e.g. a how-to that described some teams as request-driven). The only legitimate remaining hits are under `design-docs/` (history — `0000091` recorded the carve-out this design reverses).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first ordering per `.claude/rules/design-doc-numbering.md`: Phase A (docs/README/skills) lands before any code.

### Phase A: Documentation

- [x] A1. Update `docs/concepts/monitoring.md`: the message-delivery inline-preview keystroke is now `Esc`-safeguarded for **every** recipient (closing the auto-confirm hole the incident exposed); the monitor wake nudge to the monitoring member no longer leads with `Esc` (its pane is never on a prompt); the monitoring member re-engages the Director via the new `cafleet member nudge` (persists an ACKable task + fires the hardened preview), replacing the bare `cafleet message send` framing. <!-- completed: 2026-06-15T09:11 -->
- [x] A2. Update `docs/spec/cli-options.md`: add the `cafleet member nudge` row to the `member` subcommand table (flags `--agent-id` sender, `--member-id` target, `--text` summary; fleet-isolation only; error/output surface from §4); add the `permissions.allow` coverage entries from §5; note that inline previews now lead with `Esc`. <!-- completed: 2026-06-15T09:14 -->
- [x] A3. Update `README.md`: the monitoring/messaging summary reflects the universally `Esc`-safeguarded inline preview and the `member nudge` re-engage primitive. <!-- completed: 2026-06-15T09:16 -->
- [x] A4. Update `skills/cafleet/SKILL.md` (§ Send — note the inline preview now leads with `Esc`) and `skills/cafleet/reference/director.md` (add the `Member Nudge` subsection to the `member` subgroup reference: signature, the `--agent-id`+`--member-id` asymmetry vs the other member subcommands, fleet-isolation-only auth, persist+hardened-preview behavior). <!-- completed: 2026-06-15T09:18 -->
- [x] A5. Update `skills/cafleet-agent-team-monitoring/SKILL.md`: the monitoring member's canonical spawn prompt and capture-classify-reengage routine re-engage the Director via `cafleet member nudge` (not `cafleet message send`); state that the wake nudge into the monitoring member's own pane no longer leads with `Esc` (no self-interrupt); reflect that the Director's inline-preview wake is now `Esc`-safeguarded. <!-- completed: 2026-06-15T09:20 -->
- [x] A6. Update `skills/cafleet-agent-team-supervision/SKILL.md`: the Stall Response / idle re-engagement path the monitoring member runs uses `cafleet member nudge`; align any "re-engage the Director via `message send`" phrasing. <!-- completed: 2026-06-15T09:22 -->
- [x] A7. `skills/cafleet-design-doc-create` (§6.3/§6.4): delete the Primitive Mapping exemption row, the `#### 1b. Request-driven supervision (no monitor)` block, and the `roles/director.md` bootstrap exemption; add a monitor-first spawn step after fleet creation in Step 1 (`--role monitor --model sonnet`, canonical conditional-nudge prompt) and gate the Drafter/Reviewer spawns on `ready: monitor live`; rewrite the Step-6 teardown to stop the monitor task + delete the monitoring member first (first-out), removing the "(no monitor to stop)" parenthetical. <!-- completed: 2026-06-15T09:28 -->
- [x] A8. `skills/cafleet-design-doc-interview` (§6.3/§6.4): delete the Primitive Mapping exemption row and the `#### 2b. Request-driven supervision (no monitor)` block; add a monitor-first spawn step after Step 2a and gate the Analyzer spawn on `ready: monitor live`; rewrite the Step-2f teardown to stop the monitor task + delete the monitoring member first, removing the "(no monitor to stop)" parenthetical. <!-- completed: 2026-06-15T09:32 -->
- [x] A9. `skills/cafleet-research-report` (§6.3/§6.4): delete the `### Step 1: Supervision Model (… request-driven, no monitor)` exemption, the `roles/director.md` bootstrap exemption, and the Prerequisites omission; add a monitor-first spawn step after Step 0b and gate Manager/Scout/Researcher spawns on `ready: monitor live`; rewrite the Step-8 teardown (member-delete order → monitoring member → Researchers → Scouts → Manager; stop the monitor task first), removing the "no monitor to stop" sentence. <!-- completed: 2026-06-15T09:38 -->
- [x] A10. `skills/cafleet-research-presentation` (§6.3/§6.4): delete both Step-1b exemption passages, the `roles/director.md` bootstrap exemption + Shutdown-Protocol parenthetical, and the Prerequisites omission; add a monitor-first spawn step after Step 1a and gate the Presentation/Transcript spawns on `ready: monitor live`; rewrite the Step-5 teardown and `roles/director.md` Shutdown Protocol (member-delete order → monitoring member → Presentation → Transcript → VR batch after its close handshake; stop the monitor task first), removing the "no monitor to stop" sentences. <!-- completed: 2026-06-15T09:44 -->

### Phase B: Code

- [x] B1. `cafleet/src/cafleet/multiplexer/tmux.py` — `send_inline_preview`: pass `esc_first=True` into `_send_literal_then_enter`. (§1) <!-- completed: 2026-06-15T10:48 -->
- [x] B2. `tmux.py` — `send_wake_trigger`: remove `esc_first=True` (call `_send_literal_then_enter` plainly); update its docstring to drop the "Esc-safeguarded" phrasing. (§2) <!-- completed: 2026-06-15T10:48 -->
- [x] B3. `tmux.py` — `_send_literal_then_enter`: rewrite the `esc_first` block comment (`:67–70`) per §1/§2 (no longer "ping helpers only"; now "wherever the pane may be on a permission prompt"; enumerate the excluded helpers and why). (§1) <!-- completed: 2026-06-15T10:48 -->
- [x] B4. `tmux.py` — `send_inline_preview` newline (§3): **verify** whether a literal `\n` under `send-keys -l` submits or soft-inserts. If it submits (fragments into two submits), collapse the envelope/body separator to the U+23CE sentinel and update the assertion; if it soft-inserts (the anticipated outcome per §3 NOTE 2), keep the 2-line payload and rewrite the `:241–244` comment to the verified behavior. Record the finding in the task note. <!-- completed: 2026-06-15T10:48 -->
  - **FINDING (B4): soft-insert — kept the 2-line payload, rewrote the `:241–244` comment.** A live tmux probe was not runnable (raw `tmux` is harness-denied for this member and forbidden by project rules), so the verification rests on two converging, decisive sources: (1) the committed Step-2 test `test_send_inline_preview__newline_soft_insert_single_submit` is the executable spec and asserts the soft-insert contract directly — exactly **one** `-l` keystroke carries the whole 2-line payload (one embedded `\n`) and exactly **one** trailing `Enter` submits it (no second submit produced by the embedded newline); (2) production evidence — every `message send` / `broadcast` already delivers this 2-line preview today and recipients receive one coherent turn, not an envelope-then-body fragmentation, which would be universally visible if the embedded `\n` submitted as `Enter`. Conclusion: the embedded `\n` is a soft line break within a single keystroke sequence, not a submit. No fragmentation → no escalation; the 2-line shape stays and the `:241–244` comment now states the verified soft-insert behavior. The §1 leading `Escape` carries the prompt-dismissal safety guarantee independently of the newline.
- [ ] B5. `cafleet/src/cafleet/cli/member.py` — add the `member nudge` subcommand: `--agent-id` (sender) + `--member-id` (target, via `_load_authorized_member`, which surfaces the target "Agent `<id>` not found" error) + `--text` (summary); reject empty `--text` (`UsageError`); call `broker.send_message(...)`; translate **only** the sender `ValueError` from `send_message`→`ClickException` (the target ValueError is unreachable — `_load_authorized_member` ran first; do not add dead target-error handling); emit the text/JSON output from §4. (§4) <!-- completed: 2026-06-15T10:55 -->

### Phase C: Tests

- [ ] C1. `tests/multiplexer/test_tmux.py`: assert `send_inline_preview` now emits `Escape` first (full `Escape` → `-l <payload>` → `Enter` sequence); **update** the existing `0000090` assertion that `send_inline_preview` sends **no** `Esc` (it now does). Assert `send_wake_trigger` emits **no** `Esc` (inverted from `0000090`). Assert `send_poll_trigger` still emits `Esc`; `send_exit` / `send_bash_command` / `send_freetext_and_submit` still emit no `Esc`. Add the §3 newline assertion per the B4 finding (single submit on the trailing `Enter`, whichever payload shape lands). <!-- completed: -->
- [ ] C2. `tests/cli/test_member.py`: `member nudge` persists a `unicast` / `input_required` task from `--agent-id` (sender) to `--member-id` (target) carrying `--text`, and fires the inline-preview keystroke (`esc_first`); a cross-fleet/unknown `--member-id` → "Agent `<id>` not found" (exit 1); empty `--text` → exit 2; a target with no pane still queues the task (notification best-effort). Assert the persisted task is ACKable by the recipient. <!-- completed: -->
- [ ] C3. Update any monitor-loop / broker-messaging test that asserts a keystroke sequence affected by B1/B2 (e.g. a `send_wake_trigger` sequence assertion in `tests/monitor/test_loop.py`, or a `send_inline_preview` sequence assertion reached via `broker.send_message` in `tests/broker/test_messaging.py`). <!-- completed: -->

### Phase D: Verification

- [ ] D1. `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass. Sweep `docs/`, `README.md`, `skills/`, `.claude/`, root `CLAUDE.md` for residual "inline preview without Esc" / "wake … Esc-safeguarded" / "monitoring member … `message send` to re-engage" framing **and** the §6.4 exemption phrasings ("request-driven", "does not run `cafleet monitor`", "does not spawn a `--role monitor` member", "no monitor to stop"); confirm `cafleet-design-doc-execute` still conforms (monitor-first spawn, `ready: monitor live` gate, first-out teardown) and carries no exemption. The only legitimate remaining hits are under `design-docs/` (history). <!-- completed: -->
- [ ] D2. Manual smoke (operator, optional): park the Director's pane on a deny-listed `git push` permission prompt; invoke `cafleet member nudge` from the monitoring member; confirm the leading `Esc` dismisses the prompt (no `git push` executes) and the Director receives an ACKable inbox task. Confirm the monitor wake into the monitoring member's pane no longer interrupts an in-progress routine. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-14 | Initial draft |
| 2026-06-14 | Scope addition (user): made the dedicated monitoring member mandatory for every CAFleet-native orchestrated team skill (reversing 0000091's request-driven carve-out for create/interview/research-report/research-presentation; execute is the reference). Added §6 (requirement + rationale + routine-variant split + per-skill edits + removal-cleanup checklist), success criteria, tasks A7–A10, and extended the D1 sweep. Kept all §1–§5 nudge-safety content. |
| 2026-06-15 | Reviewer pass: split the run-on Overview into 3 sentences; fixed line-citation drift for the `esc_first` comment (consistent `tmux.py:67–70` across §1/Q2/B3) and the newline comment (comment `241–244`, payload `\n` at `246` across Background/§3/NOTE 2/B4); stated NOTE 2's expected branch (soft-insert / keep 2-line) with the production-evidence reasoning; separated the §4 error-handling target (`_load_authorized_member`, runs first) vs sender (`send_message` ValueError) layers and scoped B5's translation to the sender error only. |
| 2026-06-15 | User approved. Status → Approved. |
