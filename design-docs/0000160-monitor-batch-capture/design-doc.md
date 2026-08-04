# Batch Capture — `cafleet monitor scan`

**Status**: Approved
**Progress**: 12/12 tasks complete
**Last Updated**: 2026-08-04

## Overview

Add `cafleet monitor scan`, a one-shot command that captures the Director's own pane and every active member's pane in a single invocation — one synchronized fleet snapshot instead of N sequential `cafleet member capture` calls. The periodic wake payload gains an instruction to run it, and one fresh scan satisfies the Director's pre-ping capture gate for every member for that facilitation turn. Implements GitHub issue #269.

## Success Criteria

- [x] `cafleet monitor scan <FLEET_ID>` prints one section per pane — Director first, then members ascending by `member_id` — in a single invocation; `--json` emits the pinned array shape.
- [x] A pending placement or a failed pane capture renders an annotated entry and the scan still completes with exit 0.
- [x] The loop form `cafleet monitor <FLEET_ID> [--tick N] [--interval N]` parses and behaves exactly as before.
- [x] The wake payload carries the scan instruction, byte-identical on tmux and herdr.
- [x] The supervision protocol documents one fresh scan per facilitation turn as satisfying the pre-ping capture gate for all members; `cafleet member capture` remains the targeted deeper-investigation primitive.
- [x] `docs/`, `SPEC.md`, and the affected `skills/` pages are updated with zero drift; `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

Issue #269 lists four bullets. The first two — "drop per-member monitor interval" and "common global monitor interval" — are already satisfied by design 0000158, which removed the dedicated monitoring member and the per-member schedule (migration `V4__fleet_level_wake_schedule.sql`) and left a single fleet-level wake interval. This design delivers only the remaining bullets: "self + all members monitor at the same time" via the new `cafleet monitor scan` command (user decision).

Today the Director's on-wake health check reads panes one at a time: `cafleet member capture <member-id> --lines 120` per member, so a 5-member fleet costs 6 CLI calls (including a self check) and the captures are taken at different moments. The batch scan replaces that with one invocation capturing all panes back-to-back — milliseconds apart, which the user confirmed satisfies "at the same time".

---

## Specification

### CLI surface

`cafleet monitor` becomes a two-form command. The bare positional form stays the loop; `scan` is a new one-shot subcommand. Both run behind the existing stale-assets guard.

| Form | Behavior |
|---|---|
| `cafleet monitor FLEET_ID [--tick N] [--interval N]` | Unchanged: the in-process scheduler loop. |
| `cafleet monitor scan FLEET_ID [--lines N] [--ansi] [--json]` | New: capture the Director's pane + every active member's pane once, print, exit. No loop, no `monitor_runtime` claim, no DB writes. |

clap wiring: `MonitorArgs` gains `#[command(subcommand)] command: Option<MonitorCommand>` with `args_conflicts_with_subcommands = true` and `subcommand_negates_reqs = true`, so `cafleet monitor 16` still parses the loop positionals while `cafleet monitor scan 16` dispatches the subcommand. `FLEET_ID` is an integer and `scan` is not, so the two forms cannot collide. Because the `scan` form supplies no loop positional, the loop's `fleet_id: i64` field becomes `Option<i64>` — the `Scan` variant carries its own `FLEET_ID` positional plus the three flags — and the loop dispatch unwraps it with `.expect(...)` (clap guarantees the positional whenever no subcommand is given).

Scan flags:

| Flag | Required | Notes |
|---|---|---|
| `--lines` | no | Trailing lines captured per pane (an integer ≥ 1 via `value_parser` range, default **20** — user decision). |
| `--ansi` | no | Preserve ANSI escapes in every captured content (default: stripped, as in `member capture`). |
| `--json` | no | Emit the JSON array below instead of the text sections. |

### Scan roster

1. Require a live fleet (soft-deleted or unknown → `Error: fleet <id> not found`, exit 1) and resolve the multiplexer (`ensure_available` failure → exit 1) — the same guards as the loop form.
2. Read the fleet's `director_member_id` and `broker::list_members(fleet_id)` (active members only).
3. Roster = the Director's row first, then every other active member **owning a placement row**, ascending by `member_id`. A member with no placement row (not spawned via `member create`) is excluded, mirroring the wake roster's join. A placement row with a `NULL` pane (pending placement) stays in the roster as an annotated entry.
4. A fleet with no members scans the Director's pane only (user decision).

### Per-entry capture

For each roster entry, in order:

| Condition | Entry outcome |
|---|---|
| `mux_pane_id` is `NULL` (pending placement) | Annotated entry: `pane not available (pending placement)`. |
| `capture_pane(pane, lines)` errors (dead pane, backend failure — includes the Director's own pane) | Annotated entry: `capture failed: <error>`. |
| Capture succeeds | Content (ANSI-stripped unless `--ansi`), `captured_at` stamped from local UTC at that entry's read boundary, `content_sha256` over the emitted content. |

The scan always completes: an annotated entry never aborts the remaining captures, and a scan whose every entry is annotated still exits 0 (user decision). Capture content is never stored in SQLite; the command performs no DB writes (user decision).

### Output contract

**Text mode** — one section per roster entry, separated by one blank line. `<name>` is the raw DB value (stdout is not a keystroke path, so no sanitization). `kind` is `director` or `member`, making the Director row self-identifying beyond its first position.

```text
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=<pane-id>; captured_at=<ts>) ===
<content>
```

Annotated entry (pane token is `—` when no pane exists; a failed capture keeps its real pane id):

```text
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=—) ===
pane not available (pending placement)
```

**JSON mode** — a top-level array, same order, one object per entry mirroring `member capture`'s keys plus `name` / `kind` / `coding_agent` / `error`, in this pinned key order:

```json
{
  "member_id": 4,
  "name": "drafter",
  "kind": "member",
  "coding_agent": "claude",
  "pane_id": "%2",
  "lines": 20,
  "content": "…",
  "captured_at": "2026-08-04T11:20:00.000000+00:00",
  "content_sha256": "…",
  "error": null
}
```

On an annotated entry: `content`, `captured_at`, and `content_sha256` are `null`; `error` carries the exact annotation string from text mode; `pane_id` is `null` for a pending placement and the real pane id for a failed capture. `lines` always echoes the requested depth.

Exit codes: `0` completed scan (annotated entries included), `1` unknown/deleted fleet or multiplexer unreachable, `2` usage errors.

### Wake payload

In `build_wake_payload`, the poll sentence gains a leading scan clause — `Poll your inbox, ACK, dispatch.` becomes `Scan panes with 'cafleet monitor scan <fleet-id>', poll your inbox, ACK, dispatch.` — so the instruction travels with the trigger (user decision). The clause is unconditional (present in the `N == 0` form too), keeps the payload single-line and byte-identical on tmux and herdr, and uses single quotes (no backticks, keeping the envelope free of shell-sensitive characters like the sanitized name fields). The pinned block below is normative:

```text
[cafleet] tick: fleet <fleet-id> — health-check your <N> members: <entries>. Scan panes with 'cafleet monitor scan <fleet-id>', poll your inbox, ACK, dispatch. Resume your work if something was still running.
```

`N == 0` clause unchanged in shape: `no members to health-check.` followed by the same scan-poll sentence and resume sentence. Entry grammar (`<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)`), sanitization, and the fail-closed unregistered-coding-agent check are unchanged.

### Supervision-gate semantics

The pre-ping capture gate's normative capture becomes the batch scan (user decision):

- One fresh `cafleet monitor scan <fleet-id>` at the default depth (20 lines per pane) satisfies the gate for **every** member for that facilitation turn.
- The gate depth is **20 lines everywhere**: the previous normative `--lines 120` single-member capture is retired, and a default-depth capture — batch scan or single-member `member capture` — satisfies the gate. Deeper captures remain valid gate captures.
- Per-target freshness caveat: once the Director keystrokes a pane (`ping` / non-exempt `send` / `prompt`), the scan snapshot of that pane is stale — a further re-engagement of the same member within the turn needs a fresh capture (single-member `member capture` or a new scan).
- `cafleet member capture` stays unchanged as the targeted deeper-investigation primitive ("scan for all and capture if you need to investigate more" — user decision); a fresh single-member capture at default depth or deeper satisfies the gate for that one member.
- The Stage-2 "doubles as the gate capture" note in `skills/cafleet/reference/supervision.md` becomes: a Stage-2 `member capture` doubles as the gate capture for that member while still fresh (same facilitation turn, no intervening keystroke into that pane) — the depth qualifier disappears with the 120-line rule.
- Classification cues, the classification table, the skip rules, and the broadcast all-recipients rule are untouched.

### Permission coverage

The claude-side `permissions.allow` set is mechanical prefix patterns (`docs/docs/spec/cli-options.md` § `permissions.allow` coverage), so the loop form's existing `Bash(cafleet monitor *)` pattern already covers `cafleet monitor scan …` — no new pattern is required, and no permission prompt fires on the per-wake scan. The coverage section gains one sentence stating that both `monitor` forms ride the single pattern (Step 1). The opencode preset's `cafleet *` allow rule likewise already covers the subcommand.

### Out of scope / unchanged

Loop cadence and flags, the `monitor_runtime` schema and wake ledger, `member capture` / `ping` / `prompt`, the WebUI API, the data model, environment variables, and `README.md` (thin surface untouched).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: User-facing docs

- [x] `docs/docs/concepts/monitoring.md`: describe the batch scan as the facilitation snapshot (heartbeat/facilitation split unchanged), update the wake-payload description and the capture-gate paragraph to the scan-based gate <!-- completed: 2026-08-04T12:25 -->
- [x] `docs/docs/spec/cli-options.md`: rewrite § `cafleet monitor` as the two-form command; add the `cafleet monitor scan` subsection (flags, roster, output contract, exit codes) with anchor `#cafleet-monitor-scan`; update the command summary row and the Error Messages rows; add the § `permissions.allow` coverage sentence stating both `monitor` forms ride the single `Bash(cafleet monitor *)` pattern <!-- completed: 2026-08-04T12:25 -->
- [x] `docs/docs/spec/multiplexer-backends.md`: update the wake payload grammar block and the example to the scan-instruction form <!-- completed: 2026-08-04T12:25 -->

### Step 2: SPEC.md

- [x] Update the CLI surface (§6.3): the `monitor` two-form spec, scan flags, roster rules, text/JSON output shapes with pinned key order, exit codes, and the capture-never-stored note <!-- completed: 2026-08-04T12:30 -->
- [x] Update the wake-payload text everywhere SPEC pins it (`build_wake_payload`, the §6.5/§6.6 payload grammar and examples) <!-- completed: 2026-08-04T12:30 -->

### Step 3: Skills

- [x] `skills/cafleet/reference/supervision.md`: make the fresh scan the normative gate capture (one scan per facilitation turn covers all members; gate depth 20 everywhere per § Supervision-gate semantics, retiring the `--lines 120` rule; per-target freshness caveat after any keystroke); update health-check step 4(c) and the Quick Reference table; replace the Stage-2 "doubles as the gate capture" note with the freshness-only form pinned in § Supervision-gate semantics <!-- completed: 2026-08-04T12:33 -->
- [x] `skills/cafleet/reference/cli.md`: document `monitor scan` in the monitor paragraph and reposition `member capture` as the targeted deeper-investigation primitive <!-- completed: 2026-08-04T12:33 -->
- [x] `skills/cafleet/reference/director.md`: add the batch scan as the fleet-wide read primitive alongside the `member capture` section <!-- completed: 2026-08-04T12:33 -->

### Step 4: Wake payload (code)

- [x] `cafleet/src/multiplexer/mod.rs`: add the scan clause to the poll sentence in `build_wake_payload` (both the roster and `N == 0` forms); update the colocated payload tests and the verbatim payload assertion in `cafleet/tests/e2e.rs` (the tmux/herdr colocated tests compare against `build_wake_payload` output and need no edit) <!-- completed: 2026-08-04T12:39 -->

### Step 5: `cafleet monitor scan` (code)

- [x] `cafleet/src/cli/monitor.rs`: restructure `MonitorArgs` with `args_conflicts_with_subcommands` + `subcommand_negates_reqs`, turning the loop's `fleet_id` positional into `Option<i64>` (the `Scan` variant carries its own `FLEET_ID` positional; the loop dispatch unwraps); implement the scan handler (live-fleet guard, mux resolution, roster, per-entry capture with annotations, text/JSON emit) <!-- completed: 2026-08-04T12:52 -->
- [x] Tests: loop-form parse/behavior regression; scan text sections and ordering (Director first); scan JSON key set and key order; annotated entries (pending placement, failed capture) with exit 0; unknown fleet error <!-- completed: 2026-08-04T12:52 -->

### Step 6: Verification

- [x] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` all green <!-- completed: 2026-08-04T12:55 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-04 | Initial draft |
| 2026-08-04 | Review round 1: pinned the `Option<i64>` clap field change, the normative wake-payload block (scan clause folded into the poll sentence), the 20-line gate depth everywhere with the new Stage-2 note text, the `Bash(cafleet monitor *)` permission-coverage outcome, and the `e2e.rs` payload assertion; dropped the unresolvable decision letter codes |
