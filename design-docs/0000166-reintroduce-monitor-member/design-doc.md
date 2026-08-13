# Reintroduce the monitor member — a cheap-model watcher absorbs the periodic tick

**Status**: Approved
**Progress**: 10/49 tasks complete
**Last Updated**: 2026-08-13

## Overview

Reintroduce a dedicated **monitor member** — spawned `cafleet member create
--role monitor` on a cheap model — as the sole recipient of the fleet-level
periodic wake, reversing the Director-hosted tick from design 0000158
(PR #265). The monitor classifies member panes on each wake and contacts the
Director only when something actually needs attention, so the Director is
never nudged by a timer again.

## Success Criteria

- [ ] The periodic wake keystrokes the **monitor member's** pane; no code path
      keystrokes the Director's pane on a timer. `cafleet/tests/e2e.rs`
      asserts the wake lands in the monitor pane with the § S4 payload.
- [ ] `cafleet member create --role monitor` exists (sole accepted value
      `monitor`), with the one-per-fleet guard and the monitor-first placement
      guard enforced with the § S3 error strings.
- [ ] The interval surface is unchanged: `cafleet monitor <fleet-id>
      --interval N`, `CAFLEET_MONITOR_WAKE_INTERVAL` (default `600`), and live
      per-fleet editing via `PATCH /api/monitor` all still work and now govern
      the monitor-facing wake. No new `cafleet server` flag; the `monitor
      start` subcommand name is NOT reintroduced.
- [ ] Every backend section of
      `skills/cafleet/reference/coding-agent-overlays.md` (including the
      Template) carries a `{monitor_model}` row and a monitor-member-side
      loop-launch worked resolution; `OVERLAY_PLACEHOLDERS` in
      `cafleet/tests/docs_sync.rs` is back to 10.
- [ ] `skills/cafleet/reference/model-list.md` carries the monitor defaults:
      claude → `haiku`, codex → `gpt-5.6-luna`, opencode →
      `opencode/big-pickle`.
- [ ] `skills/cafleet/roles/monitor.md` exists and is the sole normative
      carrier of the on-wake protocol (docs_sync-enforced).
- [ ] The migration chain is untouched: head stays **5**; the member-kind
      marker is application-level JSON in `member_card_json`.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`,
      and `mise //admin:lint` pass.

---

## Background

Design 0000158 deleted the dedicated monitoring member and made the Director
the recipient of an unconditional fleet-level tick (default 600 s). That fixed
the old per-member ping fan-out, but created the problem this design removes:
the tick keystrokes the Director's pane **every interval regardless of need**,
displacing the Director's attention even when every member is healthy.

The remedy is a watcher again — but a cheaper, simpler one than the pre-#265
machinery. The fleet-level cadence, the durable `last_wake_at` ledger, the
deferred first wake (design 0000161), and the live per-fleet interval editing
(design 0000162) all survive unchanged; only the wake's **recipient** and the
recipient's **protocol** change. There is no per-member schedule, no
`monitor_config` table, and no separate stall-check cadence. The monitor
member runs on a cheap model because its work is bounded classification, not
generation.

---

## Specification

### S1. Supervision model, before and after

| Aspect | Today (Director-hosted tick) | After |
|---|---|---|
| Loop host | The Director's pane | The monitor member's pane |
| Who launches | The Director, immediately after `cafleet fleet create` | The monitor member, at startup, before reporting `monitor live` |
| Wake recipient | The Director | The monitor member |
| Wake cadence | Unconditional fleet-level interval (default 600 s) | Unchanged |
| Pane capture + classify | The Director, at its own discretion | The monitor member, on every wake, via `cafleet monitor scan` |
| Automatic `cafleet member ping` | none — Director-only manual primitive | The monitor's fixed-ping exception: one ping per confirmed quiet period (§ S2) |
| Director re-engagement channels | Periodic wake + broker auto-fire | Broker auto-fire + monitor event messages + the monitor's stalled-Director ping |
| Spawn gate on the first ordinary member | The loop's startup-line confirmation | The monitor member's `monitor live` message + the CLI monitor-first guard (§ S3) |
| Teardown | Stop the background task, then delete members | Delete the monitor member first (first-out); the loop dies with its pane (§ S7) |

The Director's own **pre-ping capture gate is unchanged**: every
Director-initiated re-engagement keystroke stays capture-gated, and `cafleet
monitor scan` remains the Director's normative gate capture. What the Director
loses is only the timer keystroke into its pane; what it gains is a watcher
that messages it exactly when attention is needed.

The capture-state taxonomy (`awaiting_user` / `finished` / `working` /
`stall_candidate`) and the per-backend capture cues in the overlays survive
verbatim; they gain a second consumer — the monitor's on-wake classification —
alongside the Director's gate.

**Overlay reader contract.** The Director is no longer the *sole*
cross-section reader: the monitor member classifies panes of members on any
backend, so it reads the **target member's** backend section for capture cues,
exactly as the Director does. The reader-contract prose in
`coding-agent-overlays.md`, `.claude/rules/coding-agent-overlay.md`, and the
cafleet `SKILL.md` names both readers.

### S2. The monitor member's protocol

`skills/cafleet/roles/monitor.md` is restored as the **sole normative carrier
of the on-wake protocol**; the wake trigger points at it and carries no
protocol clauses itself. The role file follows the current role-file
conventions (Required-reading block with the overlay as row #1, canonical
spawn-prompt-skeleton note, literal-ids section) and specifies:

**Startup, in order:**

1. Send the standard ready signal: `cafleet message send --from-member-id
   <my-member-id> --to-member-id <director-member-id> "ready"`.
2. Launch the heartbeat in THIS pane as a background task ({bg_run}):
   `cafleet monitor <fleet-id>`.
3. Confirm the startup line in the task output:
   `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`.
   A task that exits instead (runtime-claim conflict, dead fleet) is a failed
   start — report it to the Director instead of proceeding.
4. Send the gate signal (an anchorless status, deliberately parens-free):
   `monitor live`. This message gates the Director's first ordinary
   `cafleet member create` (belt), alongside the CLI guard (§ S3, suspenders).

**On each wake** (one `[cafleet] tick:` trigger, § S4):

1. **Capture the whole fleet once**: `cafleet monitor scan <fleet-id>
   --lines 120 --json`. Use each entry's emitted `content`, `captured_at`,
   and `content_sha256`; never invent a fingerprint.
2. **Classify content only**, per the **target member's** backend overlay
   section cues. The classification universe is exactly the wake payload's
   `<entries>` members plus the Director; the scan also captures the
   monitor's own pane, and the monitor ignores that section (its own pane is
   always mid-turn during a scan, and the command boundary below already bars
   any self-directed action). Precedence and tie-breaks are the overlay's:
   `awaiting_user` over `finished`; `working` over `stall_candidate`; a
   dead/garbled/failed capture is `unknown`.
3. **Confirm quiet across two consecutive wakes.** `stall_candidate` and
   `finished` are both quiet observations. A member is **confirmed quiet**
   only when its `content_sha256` on this wake is byte-identical to the sha
   recorded on the previous wake. A first quiet capture only seeds the
   baseline; a restart clears the notes, so the first post-restart wake
   re-seeds and never pings. Changed content, `working`, or `awaiting_user`
   ends the quiet period and re-arms the member. The monitor's memory between
   wakes is its own conversation notes; no broker state backs it.
4. **Ping an ordinary member at most once per quiet period**:
   `cafleet member ping <member-id>` (no-op-safe against a pending
   placement). Confirmed quiet alone suffices here: a member may have
   stalled mid-task with an empty inbox, and one bounded poll trigger per
   quiet period is cheap.
5. **Ping the Director only when it is actually stalled**: confirmed quiet
   across two consecutive wakes AND its wake-payload `unacked` count is
   greater than 0. A quiet Director with an empty inbox is at legitimate
   rest — leave it. The extra `unacked` condition is deliberate and does NOT
   extend step 4's quiet-alone rule to the Director: pinging the Director on
   quiet alone would recreate the timer-nudge problem this design exists to
   remove. One ping per quiet period, same re-arm rules as step 3.
6. **Message the Director per event** (`cafleet message send`, plain ordinary
   message): a member still unchanged at the next wake after its ping, a ping
   delivery failure, or an `unknown` capture — each said once per quiet
   period, not on every subsequent wake. With no event, send nothing.

**Command boundary on wake:** exactly three command families — `cafleet
monitor scan`, `cafleet member ping`, `cafleet message send` (to the Director
only). Never `message broadcast`, never `member prompt`, never a ping at
itself, never arbitrary instruction text attached to a pane action.

**Who watches the watcher:** the wake keystroke into the monitor's own pane is
`Esc`-first and closes with the resume clause, so a monitor that stalls
mid-turn is re-engaged by its own next wake — the same self-healing property
the Director-facing tick had. If the monitor member dies, the Director
re-spawns it with `--role monitor` (the one-per-fleet guard counts only
*active* members, so a deleted monitor frees the slot); the stale runtime row
reads dead on both liveness axes and is reclaimed by the fresh loop.

The third failure is the loop's background task exiting mid-run while the
monitor's pane lives: with no next wake, the fleet loses its heartbeat
silently. The role file names the monitor's standing obligation — on
observing its loop task exit (where {bg_run} delivers an exit notification,
as claude's background tasks do), relaunch `cafleet monitor <fleet-id>` (the
stale runtime row reads dead and is reclaimed, § S7) and report the restart
to the Director as an anchorless status (`monitor restarted`). On a backend
whose background primitive delivers no exit notification, the exposure is
residual: the exit surfaces on the monitor's next turn (any broker message
landing in its pane cues a task-liveness check before other work), and the
role file states that check explicitly.

### S3. CLI and member registry

| Surface | Change |
|---|---|
| `cafleet member create --role monitor` | new flag; the sole accepted value is `monitor` (clap rejects others) |
| One-per-fleet guard | `member create --role monitor` when the fleet already has an active monitor member fails: `fleet <fleet-id> already has an active monitor member (member <member-id>)` |
| Monitor-first placement guard | `member create` without `--role` when the fleet has no active monitor member fails: `fleet <fleet-id> has no active monitor member; spawn one with --role monitor first` |
| `cafleet monitor <fleet-id>` / `monitor scan` | surface unchanged (no `start` subcommand; `OLD_CLI_SURFACE` in docs_sync keeps banning `monitor start`) |
| `cafleet member ping` | surface unchanged; ownership widens from "Director-only" to "Director and monitor member" across docs/rules |
| `cafleet server` | unchanged |
| `Settings` / env vars | unchanged (`CAFLEET_MONITOR_WAKE_INTERVAL` default `600`, `0` disables the wake while the loop heartbeats) |

Registry changes in `cafleet/src/broker/members.rs`:

- `register_member` and `member_card` regain a `monitor: bool` parameter. A
  monitor registration writes `"cafleet": {"kind": "monitor"}` into
  `member_card_json`; ordinary members and the Director write no `$.cafleet`
  object. **No migration**: the marker is application-level JSON, the chain
  stays at head 5, and no pre-existing row carries the marker.
- Both guards above live in the **CLI `member create` path**
  (`cafleet/src/cli/member.rs`), evaluated in that order (one-per-fleet
  first) alongside the existing pre-registration validations, before any
  registration or pane effect. `register_member` stays guard-free — the
  deliberate consequence is that the ~38 existing `test_support::register`
  call sites across the broker suites keep working with no monitor-first
  fixture bootstrap, and the guards are exercised end-to-end in
  `cafleet/tests/cli_member.rs` (§ S9). `member create` is the only
  member-registration entry point besides `fleet create`'s root-Director
  registration, which predates any monitor and is exempt by construction.
- `active_monitor_member_id(conn, fleet_id) -> Result<Option<i64>,
  CafleetError>`: the fleet's single `status='active'` member with
  `json_extract(member_card_json, '$.cafleet.kind') = 'monitor'`.
- `derive_member_kind` widens back to three values, over `is_director` plus
  the member card: `"director"`, `"monitor"`, `"member"`. Consumers
  (`member list` rendering, `member show`, the WebUI roster) follow.

**Terminology.** The docs term is **monitor member**; the derived kind value
is `monitor`. The pre-#265 spellings stay banned by `REMOVED_VOCABULARY`
("monitoring member", "monitoring-member", "ready: monitor live"), which
enforces the new naming repo-wide; only the `--role monitor` literal leaves
the banned list (§ S8).

### S4. Contract strings

**(a) Member-facing poll trigger** — unchanged:

```
cafleet message poll <member-id> — then resume your work if something was still running.
```

**(b) Monitor-facing wake** — `build_wake_payload`, replacing the
Director-facing form:

```
[cafleet] tick: fleet <fleet-id> — health-check your <N> members: <entries>. Director: <id> (<name>; coding_agent=<agent>; unacked=<n>). Follow your monitor role protocol. Resume your work if something was still running.
```

Grammar, fully specified:

| Element | Rule |
|---|---|
| `<entries>` | `<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)`, joined by `, `, ordered by `member_id` ascending; excludes the Director and the monitor member itself |
| `Director:` segment | always present; same field grammar as an entry; its `unacked` count is the § S2 step-5 evidence |
| `<name>` | passed through `sanitize_wake_field` (unchanged) |
| `<pending-count>` | count of that member's `input_required` unicast deliveries |
| `N == 1` | `health-check your 1 member: …` (singular noun) |
| `N == 0` | the clause becomes `no members to health-check.` — no `<entries>` segment; the `Director:` segment stays |
| Invalid `coding_agent` (roster or Director) | the wake aborts with `member <id> has invalid coding_agent '<agent>'` and no keystroke is sent |

Worked example — fleet 3, Director 4, monitor 5, two ordinary members:

```
[cafleet] tick: fleet 3 — health-check your 2 members: 6 (drafter; coding_agent=claude; unacked=2), 7 (reviewer; coding_agent=codex; unacked=0). Director: 4 (Director; coding_agent=claude; unacked=1). Follow your monitor role protocol. Resume your work if something was still running.
```

The Director's `<name>` renders as stored — `create_fleet` registers the root
Director as `Director` — with no case transformation.

The `Scan panes with 'cafleet monitor scan <fleet-id>', poll your inbox, ACK,
dispatch.` clause of the Director-facing form is gone — the wake is a pure
trigger again, and the protocol lives in the monitor role file. Both strings
are pinned in `SPEC.md`, `docs/docs/spec/multiplexer-backends.md`, and the
multiplexer colocated tests. The keystroke event sequence is unchanged:
`Esc` → 0.1 s settle → payload → 1.0 s → `Enter`, byte-identical on tmux and
herdr. The operator-typing hazard documented in
`docs/docs/concepts/monitoring.md` shrinks: the wake now lands at the monitor
member's pane, which an operator is far less likely to be typing in — the
page states this and keeps `--interval 0` as the hands-on-session escape.

### S5. The tick

`MonitorMux` and `build_wake_payload` gain the Director descriptor:

```rust
pub trait MonitorMux {
    fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>;
    fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError>;
}

pub fn build_wake_payload(
    fleet_id: i64,
    members: &[Value],
    director: &Value,
) -> Result<String, MultiplexerError>;
```

`monitor_tick` steps 1–4 (ownership-checked heartbeat, fleet liveness, the
per-tick `wake_interval_seconds` re-read with the `0` disable, `wake_due`
with the `started_at` deferred-first-wake baseline) are **unchanged**. Steps
5–8 retarget:

5. Resolve the **monitor member**: `broker::active_monitor_member_id` → its
   placement's `mux_pane_id`. No active monitor member, no pane, or a pane
   absent from `mux.list_pane_ids()` → `Continue` with no wake and no stamp
   (the fleet stays due).
6. Build the roster with `broker::list_fleet_wake_targets` — now excluding
   the monitor member as well as the Director (a pending-placement ordinary
   member still makes the roster) — and the Director descriptor with the new
   `broker::fleet_wake_director(conn, fleet_id) -> Result<Value,
   CafleetError>` (`member_id`, `name`, `coding_agent`, `pending_count`; a
   live fleet always records its Director with a placement, so a missing row
   is a loud error, not a skip).
7. `mux.send_wake_trigger(monitor_pane, fleet_id, &roster, &director)`.
8. On `woke == true` only: `record_monitor_wake` stamps `last_wake_at` and
   the echo line becomes:

   ```
   <iso> tick -> wake monitor <monitor-member-id> (<N> members)
   ```

   A failed wake commits nothing and retries next tick (unchanged).

| Condition | Behavior |
|---|---|
| No active monitor member | `Continue`; no wake, no stamp, no echo |
| Monitor pane missing from `list_pane_ids` | `Continue`; no wake, no stamp, no echo |
| Fleet has no ordinary members | wake fires with the `N == 0` payload form |
| A roster member's `coding_agent` is unregistered | `send_wake_trigger` returns `Err`; the tick surfaces `CafleetError::App` and the loop exits (unchanged abort semantics) |

`run_monitor_loop`, `wake_due`, the single-instance claim, the SIGTERM/SIGINT
stop flag, `interruptible_sleep`, the ownership-checked exit clear, and the
startup line are all unchanged.

### S6. Overlay and model policy

`skills/cafleet/reference/coding-agent-overlays.md` — every backend section
plus the Template:

- **`{monitor_model}` placeholder row** added to each substitution table:

  | Backend section | `{monitor_model}` value |
  |---|---|
  | `## claude` | `haiku` |
  | `## codex` | `gpt-5.6-luna` |
  | `## opencode` | `opencode/big-pickle` |
  | `## Template` | `<this backend's monitor default from the model list's *Monitor and reviewer defaults* table>` |

- **§ Worked resolution** in each backend section becomes the
  **monitor-member-side** loop launch, fully resolved (the section's purpose —
  one fully materialized command per backend — is preserved). For claude:
  launch `cafleet monitor <fleet-id>` via the Bash tool with
  `run_in_background: true`, confirm
  `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in the
  task output, then send `monitor live` to the Director. For codex and
  opencode: the same, launched as a backgrounded `!` shell command. The
  Template section's instructions follow.
- **Pane-state capture cues**: the intro sentence and the *Note → applies at*
  binding retarget from "the Director's on-tick health check" to both
  consumers — the monitor member's on-wake classification (its role file's
  § On each wake) and the Director's pre-ping capture gate.
- The reader-contract intro (file top) names the Director **and the monitor
  member** as the two cross-section readers (§ S1).

`skills/cafleet/reference/model-list.md`: § *Reviewer defaults* becomes
§ *Monitor and reviewer defaults*, one three-column table the overlays mirror.
The rename ripples to every reference to the old heading, each updated in the
same change: the overlays Template's `{reviewer_model}` row ("from the model
list's *Reviewer defaults* table"), `.claude/rules/coding-agent-overlay.md`'s
mirror sentence, and `skills/cafleet/roles/director.md` § Model selection.

| Backend | Reviewer | Monitor |
|---|---|---|
| claude | fable | haiku |
| codex | gpt-5.6-sol | gpt-5.6-luna |
| opencode | opencode/glm-5.2 | opencode/big-pickle |

The `cafleet-model-list-refresh` skill regains the obligation to keep the
monitor column in sync. The Director's model policy
(`skills/cafleet/roles/director.md` § Model selection and
`docs/docs/concepts/model-selection.md`) regains the monitor row: the monitor
member is a policy exception on every team spawn, always spawned
`--model {monitor_model}` regardless of cost mode. The cafleet `SKILL.md`
documented-defaults table regains a `{monitor_model}` row; its default when
the section is silent is the reviewer default's floor: inherit the spawning
Director's model (safe, possibly cost-suboptimal).

### S7. Lifecycle — spawn, teardown, recovery

**Spawn.** The Director's protocol (canonical in
`skills/cafleet/reference/supervision.md` § Spawn Protocol) becomes:

1. `cafleet fleet create --coding-agent <backend> --json`; `cafleet doctor`.
2. Spawn the monitor member FIRST:
   `cafleet member create --fleet-id <fleet-id> --role monitor
   --model <monitor default> --name monitor --description ... --file <abs
   path to ${BASE}/.prompts/monitor-<UTC-compact>.md>`. Omit
   `--coding-agent` — the monitor inherits the Director's backend; the model
   comes from the model list's monitor column for that backend.
3. Wait for the monitor's `ready`, then `monitor live` (the loop startup
   confirmation). `monitor live` gates the first ordinary `member create`;
   the CLI's monitor-first guard backstops a Director that skips the wait.
4. Spawn ordinary members as today.

The Director **no longer launches the loop** and no longer confirms the
startup line itself; the launch step moves to the monitor role file.

**Teardown**, in order (updates `skills/cafleet/reference/recovery.md`
§ Shutdown Protocol and supervision.md § Cleanup Protocol):

1. `cafleet member delete` the **monitor member first** (first-out) — the
   pane kill takes the loop process down with it, ending the wake source
   before any other member disappears.
2. `cafleet member delete` each remaining member.
3. `cafleet member list` — only the root Director's row remains.
4. `cafleet fleet delete <fleet-id>` (removes the `monitor_runtime` row
   unconditionally, as today).
5. `cafleet fleet list` to confirm.

**Recovery.** A pane-death without a graceful stop leaves a stale
`monitor_runtime` row; it reads dead on both liveness axes (stale heartbeat +
signal-0 probe) and is reclaimed by the next `cafleet monitor <fleet-id>` —
unchanged semantics, now exercised by monitor re-spawn instead of Director
restart. Mid-run monitor death: the Director re-spawns with `--role monitor`
(§ S2).

### S8. HTTP API, admin WebUI, and docs_sync

| Surface | Change |
|---|---|
| `GET /api/monitor` | shape unchanged; the `members` array re-sources to the wake roster, which now also excludes the monitor member |
| `PATCH /api/monitor` | unchanged (`wake_interval_seconds`, picked up within one tick — the § Success interval surface) |
| `GET /api/members` roster | `kind` widens back to `"director" \| "monitor" \| "member"` (falls out of `derive_member_kind`) |
| `admin/src/types.ts` | the `kind` union widens to the three values |
| Admin roster rendering | the `monitor` kind badge returns in the roster component |

`cafleet/tests/docs_sync.rs`:

- `REMOVED_VOCABULARY` 17 → 16: drop `--role monitor` (reintroduced
  literally). `monitoring member`, `monitoring-member`,
  `ready: monitor live`, `monitor_config`, `CAFLEET_MONITOR_STALL_INTERVAL`,
  and `stall-check` **stay banned** — the new surfaces use the new spellings
  and no per-member schedule or stall-check cadence returns.
- `OLD_CLI_SURFACE` unchanged — `monitor start` stays banned.
- `OVERLAY_PLACEHOLDERS` 9 → 10: add `monitor_model`; the
  full-placeholder-vocabulary and known-brace-token tests follow.
- `monitoring_concept_covers_the_director_tick_and_capture_taxonomy` is
  renamed to cover the monitor member; its required terms add
  `monitor member`, `monitor live`, `--role monitor`, and
  `Follow your monitor role protocol`.
- A restored `the_monitor_role_is_the_sole_normative_protocol_carrier` test
  targets `skills/cafleet/roles/monitor.md` (required terms: the three
  on-wake command families, `monitor live`, the two-wake confirmation, the
  Director-ping condition).
- `multiplexer_backends_pins_the_pure_trigger_payload` changes additively:
  `Director:` and `Follow your monitor role protocol` join its required
  terms (the existing pins — `[cafleet] tick:`, `coding_agent=`, the resume
  clause, the poll-trigger prefix — stay). The `Scan panes with` literals to
  delete live in the multiplexer colocated tests, covered by § S9.
- The supervision/director/member/cafleet-skill required-term tests and the
  `every_backend_overlay_defines_the_capture_cues` bindings are rewritten to
  the new protocol; the module-header `//!` block re-narrates the
  monitor-member contract.

### S9. Test-contract changes (Rust)

| Test surface | Change |
|---|---|
| `cafleet/src/broker/members.rs` colocated | add: monitor registration writes the `$.cafleet.kind='monitor'` card marker; `active_monitor_member_id` over active/deleted members; `derive_member_kind` three-value. No guard tests here — the guards live in the CLI layer (§ S3), so every existing `test_support::register` call site (`messaging.rs`, `queries.rs`, `fleets.rs` included) is untouched. The `register_member_writes_no_monitor_config_row` absence test stays |
| `cafleet/src/broker/monitor.rs` colocated | `list_fleet_wake_targets` excludes the monitor member; `fleet_wake_director` field set; runtime claim/heartbeat/clear suite unchanged |
| `cafleet/src/monitor/mod.rs` colocated | the tick resolves the monitor pane (dead-monitor-pane skip, no-monitor skip), the Director-descriptor pass-through, the `N == 0` form, the new echo line; wake-due gating, durable stamp, `woke`-gated write, and `interval 0` tests survive with retargeted fixtures |
| `cafleet/src/multiplexer/{mod,tmux,herdr}.rs` colocated | `build_wake_payload` tests rewritten to the § S4 grammar (Director segment, exclusions, singular, `N == 0`); the `Esc`-first event-sequence and poll-trigger assertions unchanged |
| `cafleet/src/output/formatters.rs` colocated | the `member list` roster fixture regains a `monitor` kind row |
| `cafleet/src/cli/member.rs` / `cafleet/tests/cli_member.rs` | `--role monitor` accepted, `--role other` rejected by clap; both § S3 guards exercised end-to-end with their pinned error strings; the existing `--role`-is-unknown regression guard is deleted (superseded by the flag's return) |
| `cafleet/tests/webui_routes.rs` | roster `kind` three-value; `GET /api/monitor` members array excludes the monitor member |
| `cafleet/tests/e2e.rs` | spawn includes `--role monitor` first; assert the wake payload lands in the **monitor** pane and the new echo line |
| `cafleet/src/config.rs` colocated | unchanged |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
>
> Documentation precedes code (`.claude/rules/documentation-maintenance.md`).
> Steps 1–4 land the full documentation surface; only then does code begin.

### Step 1: Concepts and user-facing docs

- [x] Rewrite `docs/docs/concepts/monitoring.md`: the monitor member as wake recipient, the on-wake classification + two-wake quiet confirmation, the fixed-ping exception and the stalled-Director ping condition, the `monitor live` gate, the § S4 payload, the reduced operator-typing hazard, lifecycle per § S7 <!-- completed: 2026-08-13T09:52 -->
- [x] Update `docs/docs/concepts/overview.md`: the Core terms row for `monitor`, the member-kinds sentence (three-value union), § Monitoring, and the `member create` flag mention <!-- completed: 2026-08-13T09:54 -->
- [x] Update `docs/docs/concepts/model-selection.md`: restore the monitor row in the policy-exception table and the "monitor and reviewer are policy exceptions on every team spawn" wording <!-- completed: 2026-08-13T09:55 -->
- [x] Update `docs/docs/spec/cli-options.md`: the `--role` flag row (sole value `monitor`), both § S3 guard error strings, the `member ping` ownership wording <!-- completed: 2026-08-13T09:58 -->
- [x] Update `docs/docs/spec/data-model.md`: document the `$.cafleet.kind` member-card marker and the three-value derived kind <!-- completed: 2026-08-13T09:59 -->
- [x] Update `docs/docs/spec/webui-api.md`: the roster `kind` union, the `GET /api/monitor` members-array exclusion of the monitor member <!-- completed: 2026-08-13T10:01 -->
- [x] Update `docs/docs/spec/multiplexer-backends.md`: the § S4 monitor-facing wake payload and recipient (§ *The Director wake and the fixed direct ping* heading retargets) <!-- completed: 2026-08-13T10:03 -->
- [x] Update `docs/docs/how-to/mixed-backend-team.md`: add the monitor-first spawn step to the walkthrough <!-- completed: 2026-08-13T10:06 -->
- [x] Run the `/update-readme` skill to sync `README.md` and `SPEC.md` <!-- completed: 2026-08-13T10:24 -->
- [x] Hand-verify `SPEC.md` against §§ S1–S8 at: §5.4 *Member kind discriminator*; §6.2 *Broker*; §6.3 *CLI* (`member create`, `monitor` group); §6.5 *Multiplexer* (wake payload); §6.6 *Monitor heartbeat loop*; §6.8 *WebUI + Config*; §10 *CLI command checklist* <!-- completed: 2026-08-13T10:35 -->

### Step 2: Model list and overlays

- [ ] `skills/cafleet/reference/model-list.md`: § *Reviewer defaults* → § *Monitor and reviewer defaults* with the § S6 three-column table <!-- completed: -->
- [ ] `skills/cafleet/reference/coding-agent-overlays.md`: add the `{monitor_model}` row to all four substitution tables; retitle the Template's `{reviewer_model}` row reference to *Monitor and reviewer defaults*; replace each § *Worked resolution* with the monitor-member-side launch; retarget the capture-cues intro + Note bindings to both consumers; name both cross-section readers in the file intro <!-- completed: -->
- [ ] `.claude/rules/coding-agent-overlay.md`: add `{monitor_model}` to the model-policy paragraph, retitle its *Reviewer defaults* mirror sentence to *Monitor and reviewer defaults*, and name the monitor member as a cross-section reader <!-- completed: -->
- [ ] `.claude/skills/cafleet-model-list-refresh/SKILL.md`: restore the monitor-model refresh obligation <!-- completed: -->

### Step 3: The `cafleet` skill

- [ ] Create `skills/cafleet/roles/monitor.md` per § S2 (Required-reading block with the overlay row #1, startup order, on-wake protocol, command boundary, teardown, literal-ids, canonical-skeleton delta: `--role monitor --model {monitor_model}`, omit `--coding-agent`) <!-- completed: -->
- [ ] Rewrite `skills/cafleet/reference/supervision.md`: § The monitor heartbeat (monitor-hosted), § Spawn Protocol (monitor-first + `monitor live` gate; the Director no longer launches the loop), the facilitation-cue paragraph (Director re-engagement channels: broker auto-fire + monitor events), § Monitor Lifecycle, § Cleanup Protocol (§ S7 order), § Quick Reference rows <!-- completed: -->
- [ ] Update `skills/cafleet/SKILL.md`: § Team supervision rewritten to the monitor-member model; documented-defaults table regains `{monitor_model}` (default: inherit the spawning Director's model) <!-- completed: -->
- [ ] Update `skills/cafleet/roles/director.md`: Required-reading, the monitor-first spawn step, § Model selection monitor rule (including its *Reviewer defaults* heading reference → *Monitor and reviewer defaults*) <!-- completed: -->
- [ ] Update `skills/cafleet/roles/member.md`: note that the monitor member's role file overrides the generic member protocol where they conflict <!-- completed: -->
- [ ] Update `skills/cafleet/reference/director.md`: the `--role` row in the `member create` flag table; § Member Ping ownership wording <!-- completed: -->
- [ ] Update `skills/cafleet/reference/recovery.md` § Shutdown Protocol: monitor-first-out order + the stale-row reclaim note <!-- completed: -->
- [ ] Update `skills/cafleet/reference/cli.md`: the monitor member as loop launcher; `member create --role` <!-- completed: -->
- [ ] Update `.claude/rules/bash-tool.md`: `member ping` ownership becomes "the Director and the monitor member"; add the monitor's fixed-ping exception sentence <!-- completed: -->

### Step 4: The workflow skills

- [ ] Update `skills/cafleet-design-doc/`: `SKILL.md`, `create/create.md`, `create/roles/director.md`, `execute/execute.md`, `execute/roles/director.md`, `interview/interview.md` — replace the Director loop-launch + startup-line gate with the monitor-first spawn + `monitor live` gate; update each teardown block <!-- completed: -->
- [ ] Update `skills/cafleet-research/`: `SKILL.md`, `report/report.md`, `report/roles/director.md`, `presentation/presentation.md`, `presentation/roles/director.md` — same substitution <!-- completed: -->
- [ ] Update `.claude/skills/clean-docs/**`: the team table and every spawn/teardown mention <!-- completed: -->
- [ ] Update `.claude/skills/skill-author/SKILL.md`: the bootstrap sections and the worked example <!-- completed: -->

### Step 5: Rust — broker and member registry

- [ ] `cafleet/src/broker/members.rs`: `monitor: bool` on `register_member` / `member_card`, the card marker write, `active_monitor_member_id`, three-value `derive_member_kind` (no guards here — § S3 puts them in the CLI layer) <!-- completed: -->
- [ ] `cafleet/src/broker/monitor.rs`: exclude the monitor member from `list_fleet_wake_targets` and `monitor_members_payload`; add `fleet_wake_director` <!-- completed: -->
- [ ] `cafleet/src/broker/test_support.rs`: catalogue the new/changed functions; add a monitor-registration helper for the suites that need a monitor fixture (existing `register` call sites stay as they are — the guards are CLI-layer) <!-- completed: -->
- [ ] Colocated tests in `members.rs` and `monitor.rs` per § S9 <!-- completed: -->
- [ ] Bring each touched `cafleet/src/broker/*.rs` module `//!` header in line with the new surface <!-- completed: -->

### Step 6: Rust — the tick, the multiplexer, the CLI

- [ ] `cafleet/src/multiplexer/mod.rs`: `build_wake_payload(fleet_id, members, director)` per § S4; `send_wake_trigger` trait signature gains `director` <!-- completed: -->
- [ ] `cafleet/src/multiplexer/tmux.rs` and `herdr.rs`: thread the new signature; keystroke sequence unchanged <!-- completed: -->
- [ ] `cafleet/src/monitor/mod.rs`: retarget `monitor_tick` steps 5–8 per § S5 (monitor-pane resolution, roster + director descriptor, new echo line); narrow nothing else <!-- completed: -->
- [ ] `cafleet/src/cli/member.rs`: add `--role` (sole value `monitor`), implement both § S3 guards before any registration or pane effect, thread `monitor` into `register_member` <!-- completed: -->
- [ ] `cafleet/src/output/formatters.rs`: the roster fixture regains the `monitor` kind row <!-- completed: -->
- [ ] Colocated tests in all six files per § S9 <!-- completed: -->

### Step 7: HTTP API and admin WebUI

- [ ] `cafleet/src/webui/mod.rs`: the members-array exclusion follows the broker re-source; no route changes <!-- completed: -->
- [ ] `admin/src/types.ts`: widen the `kind` union to `"director" | "monitor" | "member"` <!-- completed: -->
- [ ] Admin roster component: restore the `monitor` kind badge <!-- completed: -->
- [ ] `mise //admin:lint` and `mise //admin:build` pass <!-- completed: -->

### Step 8: Integration tests and verification

- [ ] `cafleet/tests/docs_sync.rs` per § S8 <!-- completed: -->
- [ ] `cafleet/tests/cli_member.rs` per § S9 <!-- completed: -->
- [ ] `cafleet/tests/webui_routes.rs` per § S9 <!-- completed: -->
- [ ] `cafleet/tests/e2e.rs` per § S9 <!-- completed: -->
- [ ] `rg -n "monitoring member|monitoring-member|ready: monitor live|monitor_config|CAFLEET_MONITOR_STALL_INTERVAL|stall-check" -g '!design-docs/**' -g '!cafleet/migrations/**' -g '!cafleet/tests/docs_sync.rs'` hits only the permanent enforcement carve-outs established by design 0000158 <!-- completed: -->
- [ ] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` pass <!-- completed: -->
- [ ] `mise //cafleet:install`, then a manual smoke run: `cafleet fleet create`, spawn a monitor member with `--role monitor --interval`-shortened cadence, confirm one `Esc`-first wake lands in the **monitor** pane with the § S4 payload and that the Director's pane receives none <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-12 | Initial draft |
| 2026-08-12 | Review round 1: pinned the on-wake classification universe; added the ping-asymmetry rationale; specified the monitor's loop-restart obligation for a mid-run loop exit; moved both spawn guards to the CLI `member create` layer (broker fixtures untouched); fixed the worked example's Director name casing; named the *Monitor and reviewer defaults* rename ripple targets; corrected the docs_sync payload-pin bullet and the SPEC §6.8 heading |
| 2026-08-12 | Approved by the user |
