# SPEC.md / docs vs Implementation Consistency Reconciliation

**Status**: Approved
**Progress**: 9/44 tasks complete
**Last Updated**: 2026-07-04

## Overview

A multi-agent static audit found **32 confirmed contradictions** between the reimplementation
specification (`SPEC.md` + `docs/`) and the running implementation. This design doc reconciles
every one of them with an exact, directly-executable edit — aligning docs to code where the code
is correct, and fixing the code (with tests) where the doc/SPEC is correct and the code is the
buggy side. The authoritative audit input is `audit-findings.md` in this directory.

## Success Criteria

- [ ] All 32 contradictions have a landed resolution (doc edit or code fix) — zero remaining drift.
- [ ] Every code fix ships with a regression test that would fail against the pre-fix code.
- [ ] `SPEC.md` and `docs/` describe only the current, true behavior (first-class targets, per
      `documentation-maintenance.md`); `README.md` and every `skills/*/SKILL.md` are verified
      consistent with the reconciled CLI/API/schema surfaces.
- [ ] `--json` broadcast output carries separate `recipients` / `delivered` keys; `tasks.to_agent_id`
      is nullable and emits `null` (not `0`) for broadcast-summary rows.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` pass.

---

## Background

The audit de-duplicated 49 raw findings into 32 contradictions across 8 clusters (9 high, 13
medium, 10 low). The user's resolution policy: **the implementation is basically correct** — default
to aligning docs to code. A small set of items is the reverse (doc/SPEC correct, code buggy); those
are explicit **code fixes** and must not be "fixed" by rewriting the doc to match a bug.

Four decisions from the user (relayed by the Director) govern the ambiguous items:

- **Q1 — `to_agent_id` (1.1 + 1.2):** Align **code to SPEC**. Migrate the schema to a nullable
  `to_agent_id`, write `NULL` (not `0`) on broadcast-summary rows, and change both `to:`-surfacing
  checks from truthiness to `is None` / `IS NULL`. A **new Alembic migration** is required; `--json`
  output changes `0` → `null`. SPEC §5.5 already describes this target state and stays as-is.
- **Q2 — hidden flags (3.4 + 3.5):** Per-flag disposition — **UNHIDE** a genuinely-needed flag
  (remove `hidden=True`, honoring SPEC §10 "no hidden flags"), or **REMOVE** an unneeded one
  entirely (code + every doc mention, per `removal.md`). No blanket hide-vs-document.
- **Q3 — broker-guard exit codes (2.2 + 3.3):** Normalize **both** guards to `click.ClickException`
  (exit 1), matching the sibling guard at `member.py:328-331`. Both become code fixes; docs already
  expect exit 1, so no doc edit.
- **Q4 — scope:** One executable design doc. The Implementation section lists **both** doc-alignment
  edits and code fixes (with test updates) as concrete, ordered, directly-executable tasks.

---

## Specification

### Resolution direction per cluster

| Fix side | Items |
|---|---|
| **Code** (doc/SPEC is correct) | 1.1, 1.2, 2.1, 2.2, 2.4, 3.1 (option→positional), 3.3, 4.1, plus the UNHIDE code edits of 3.4/3.5 |
| **Docs** (code is correct) | 1.3, 2.3, 2.5, 2.6, 3.2, 3.6, 4.2, 4.3, 4.4, 5.1, 5.2, 6.1, 6.2, 6.3, 7.1, 7.2, 7.3, 7.4, 8.1, 8.2, 8.3, 8.4, plus the `configure.md` blanket sentence in 3.1 and the flag-documentation edits of 3.4/3.5 |

### `--quiet` / `--activity` / `--ansi` / `--full` disposition (Q2, items 3.4 + 3.5)

Each hidden flag gets exactly one disposition. All four are **UNHIDE** (user-confirmed). Rationale:

| Flag | Sites | Disposition | Rationale | Certainty |
|---|---|---|---|---|
| `--full` | `_helpers.py:47` (`full_flag`), applied on `fleet show` (via create/list group), all six `message` cmds, `member create` | **UNHIDE** | Core token-reduction opt-back-in; documented throughout SPEC (`output-flags.md`, `message-envelope.md`) and already **visibly** present on `member show` (`member.py:466`). Hiding it elsewhere is the drift. | High |
| `--activity` | `member.py:490` (`member list`; `hidden=True` at 494) | **UNHIDE** | Distinct operator view backed by its own broker fn (`list_members_with_activity`) and formatter (`format_member_list_activity`); mutually exclusive with `--all`. A real feature. | High |
| `--ansi/--no-ansi` | `member.py:543` (`member capture`; `hidden=True` at 545) | **UNHIDE** | Real capture option — passes raw ANSI escapes through instead of stripping. Needed when the operator wants colored pane output. | High |
| `--quiet` | `_helpers.py:48` (`quiet_flag`), applied at exactly three sites: `message.py:34` (send), `message.py:110` (ack), `member.py:635` (ping) | **UNHIDE** | Emits the bare `task_id` for shell capture — a scripting convenience distinct from `--json`. Wired and functional. | High. `SPEC.md:1007` "No `--quiet` flag" is corrected to document it (item 3.4). |

**Consequence for the SPEC "no hidden flags" claims:** After all four are unhidden, SPEC §10:2518
("there are no hidden flags") and SPEC §6.3 "`--full` (documented)" become **true as written** — no
edit to those assertions. Newly-visible `--activity` / `--ansi` / `--quiet` must be
**added** to the docs so the "documented" claim is honest (Step 9).

### Contradiction resolution table

Every item's exact edit. "Doc" = align docs to code; "Code" = fix code (doc is correct).
Line numbers are from `audit-findings.md` (spot-verified accurate).

#### Cluster 1 — Persistence / `to_agent_id`

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 1.1 | HIGH | Code | `db/models.py:73` → `nullable=True`. Add a **new Alembic migration** (next revision after head) altering `tasks.to_agent_id` to nullable. `broker/messaging.py:218` → write `"to_agent_id": None` (not `0`) on the broadcast-summary row. SPEC:299/371-379/662-663/2612, `data-model.md:80`, `message-envelope.md:87` stay (already describe nullable + NULL + no-sentinel). |
| 1.2 | MED | Code | `broker/queries.py:68-70` → `if to_id is not None:` (or `IS NOT NULL` in SQL). `output/formatters.py:39-40` → `if task.get("to_agent_id") is not None:`. Docs SPEC:374-379/706-708/1525-1526/1573-1576 stay. |
| 1.3 | MED | Doc | `docs/spec/data-model.md:92-93` → change documented index order `status_timestamp DESC` to **ascending** (match `0001_initial_schema.py:77-87`, `db/models.py:81-84`, SPEC:2424-2427). |

#### Cluster 2 — Broker

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 2.1 | HIGH | Code | `broker/messaging.py:253-255` → return both `"recipients": len(recipient_ids)` (real N) and `"delivered": notifications_sent_count` (preview-success k), not a single `notifications_sent_count` key. `cli/message.py:70-75` → print `recipients={recipients} delivered={delivered}`. Docs SPEC:670-674/1013-1022, `message-envelope.md:74`, `cli-options.md:557-561/81/131` already contract both keys. |
| 2.2 | HIGH | Code | `broker/agents.py:272-276` → raise `click.ClickException` (exit 1) instead of `click.UsageError` (exit 2). Docs SPEC:596-603/804/2348/2352-2354 already say exit 1. (Paired with 3.3 per Q3.) |
| 2.3 | HIGH | Doc | `SPEC.md:362-366` (§5.4 summary) and `514-517` → rewrite the two-value kind-projection summary to match code: `get_agent` returns 4 values incl `"monitor"`; `list_agents` returns none; `list_fleet_agents` collapses. Per-function bullets 588-595/611-614 already match. |
| 2.4 | MED | Code | `broker/_shared.py:34-51` → guard the kind predicates so a null / non-object `cafleet` card (`{"cafleet": null}`, `{"cafleet":"x"}`) returns `False` instead of raising `AttributeError` (catch/guard beyond the current `except ValueError`). Docs SPEC:511-514 already specify false-on-malformed. |
| 2.5 | MED | Doc | `docs/concepts/fleet-isolation.md:16-17` → replace "always indistinguishable not found" with the code's two-string behavior: distinct "not found" vs "not in fleet" (match `messaging.py:150-153`, SPEC:810-811). |
| 2.6 | LOW | Doc | `docs/api/broker.md:15-22,26-30` → add `monitor.py` and `skill_installs.py` to the module layout, correct the re-export contract, and add `list_roster` (match `broker/__init__.py:27-41`, `broker/members.py:66`). |

#### Cluster 3 — CLI

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 3.1 | HIGH | Code + Doc | **Code:** `cli/fleet.py:82-86` (`show`) and `105-107` (`delete`) → replace `@click.argument("fleet_id", type=int)` with `@fleet_id_option`; read `fleet_id` from `ctx.obj` (add `@click.pass_context` where needed). This makes documented `cafleet fleet delete --fleet-id 1` work. Docs SPEC/`cli-options.md`/`quickstart.md:134`/`configure.md` already say `--fleet-id`. **Doc:** `docs/get-started/configure.md:110-111` → narrow the blanket "every fleet-scoped command takes `--fleet-id`" to exclude `fleet create` / `fleet list`. |
| 3.2 | HIGH | Doc | `docs/spec/cli-options.md:1029` → change "Click exit-2 usage error" for missing `--fleet-id` to the exit-1 custom `ClickException` (match `_helpers.py:57-78`, SPEC:845-847). |
| 3.3 | HIGH | Code | `broker/agents.py:65-66` → raise `click.ClickException` (exit 1) instead of `click.UsageError` (exit 2) for `member create` into a soft-deleted fleet. Doc `cli-options.md:1033` already expects exit 1 → no doc edit. (Normalized with 2.2 per Q3.) |
| 3.4 | MED | Code + Doc | **Code:** `_helpers.py:48` → remove `hidden=True` from `quiet_flag`; add `help=`. **Doc:** `docs/spec/cli-options.md:146,899-901` and `SPEC.md:1007` → document `--quiet` (drop "No `--quiet` flag"). SPEC:1210-1212 already documents the ping flag. |
| 3.5 | MED | Code + Doc | **Code:** `_helpers.py:47` → remove `hidden=True` from `full_flag` (add `help=`); `member.py:494` → remove `hidden=True` from `--activity` (add `help=`); `member.py:545` → remove `hidden=True` from `--ansi/--no-ansi` (add `help=`). **Doc:** add `--activity` (`member list`) and `--ansi` (`member capture`) to `docs/spec/cli-options.md` + SPEC §6.3. SPEC §10:2518 and `cli-options.md:76,304,636` "documented" claims become correct as-is (no edit). |
| 3.6 | LOW | Doc | `SPEC.md:992-994` → `fleet list` is **5 columns**: FLEET_ID/DIRECTOR/LABEL/AGENTS padded `40/40/20/8` + CREATED_AT trailing (unpadded), not `40/20/8`. `docs/spec/cli-options.md:367-370` → fix the impossible sample spacing (match `cli/fleet.py:71-79`). |

#### Cluster 4 — Output / formatters

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 4.1 | HIGH | Code | `output/formatters.py:216-220` → `_format_ping_age` returns ASCII `"-"`, not EM DASH `"—"` (U+2014), for the `age_seconds is None` case (used at `:252`). Docs SPEC:1495-1503/1633/1648-1651/2481-2483/2618, `cli-options.md:982,985` mandate ASCII. Sibling helpers already use `-`. |
| 4.2 | MED | Doc | `docs/spec/message-envelope.md:31` → `status_state` is omitted **unconditionally** from the compact envelope, not "conditionally omitted" (match `render.py:77-89`, SPEC:1457,1531). |
| 4.3 | MED | Doc | `docs/concepts/token-reduction.md:25` → correct the default and `--full` agent-render description (match `formatters.py:73-100`, `member.py:481-484`, SPEC:1578-1585,1163). |
| 4.4 | LOW | Doc | `SPEC.md:1652` → remove `coding_agent` from the truthiness-gated conditional-fields list; it is always emitted (`_shared.py:73-81`) and SPEC:1550-1551 already marks it required. |

#### Cluster 5 — Multiplexer

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 5.1 | HIGH | Doc | `SPEC.md:1668-1670` (§6.5), `2379-2381` (§7.2 verbatim-error contract), `docs/spec/cli-options.md:420` → change the tmux-guard error string "tmux-pane commands…" to "member commands…" (match `multiplexer/tmux.py:128-129`; five test files already assert "member"). Leftover from the `tmux-pane` → `member` rename. |
| 5.2 | MED | Doc | `SPEC.md:1718-1721` → `send_bash_command` rejects newline **and CR**, not newline-only (match `tmux.py:313-314`, SPEC:1200 phrasing "newline/CR"). |

#### Cluster 6 — Monitor

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 6.1 | MED | Doc | `docs/concepts/monitoring.md:180,194-196` → heartbeat runs **first** in the tick (STOP on zero-row), not last (match `monitor/loop.py:73-113`, SPEC:1866-1871). |
| 6.2 | LOW | Doc | `SPEC.md:1829,1833-1834` (and recurrences 1870,1873,1899,1917,1924) → marker names are `CONTINUE` / `STOP`, not `Continue` / `Stop` (match `loop.py:34-35`; SPEC:2492 already uses `STOP`). |
| 6.3 | LOW | Doc | `SPEC.md:1841-1843` → the module exposes **three** public functions, not four (match `loop.py:38,60,134`; SPEC:1827-1832 already lists three). |

#### Cluster 7 — WebUI API

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 7.1 | MED | Doc | `docs/spec/webui-api.md:232` → the timeline is scoped by **sender** (`from_agent_id`), not recipient (`context_id`) (match `queries.py:33-42`, `webui/api.py:192`, SPEC:702-704). |
| 7.2 | MED | Doc | `docs/spec/webui-api.md:23-36` → `GET /api/fleets` returns five-key rows **including `director_agent_id`** and excludes soft-deleted (`deleted_at IS NULL`); drop "all" (match `broker/fleets.py:126-164`, `api.py:114-116`, SPEC:546-549,2222-2223). |
| 7.3 | LOW | Doc | `SPEC.md:2254-2255` (§6.8) → `/api/timeline` is hard-capped at **200** messages, not "all messages" (match `queries.py:19,41`; SPEC §6.2 + `webui-api.md:258` already correct). |
| 7.4 | LOW | Doc | `SPEC.md:2259-2263` (§6.8) → the send path also requires the recipient `status == "active"`, in addition to the "not in fleet" check (match `webui/api.py:201-210`, `agents.py:186-192`; `webui-api.md:293,310-311` already correct). |

#### Cluster 8 — Get-started / reference

| Item | Sev | Side | Exact edit |
|---|---|---|---|
| 8.1 | MED | Doc | `docs/reference/coding-agents/opencode.md:84` → add the mandatory `--fleet-id` to the `member capture` example (match `member.py:530-532`, `_helpers.py:57-67`). |
| 8.2 | LOW | Doc | `docs/get-started/configure.md:59,76-80` → remove the nonexistent `cafleet agent` group from the Codex allowlist; add the real `member show` (match `cli/__init__.py:26-32`, SPEC:2593). |
| 8.3 | LOW | Doc | `docs/get-started/install.md:89` → the skill-install command is not bare `gh skill install ./ --from-local`; it adds `--agent`, `--force`, `--scope user` (match `mise.toml:22-27`). |
| 8.4 | LOW | Doc | `docs/get-started/contributing.md:36` → `cafleet:format` first runs `ruff check --fix`, then `ruff format` — not "ruff format" only (match `cafleet/mise.toml:16-21`). |

### Cross-surface consistency (README + SKILL.md)

The audit did not cite `README.md` or any `skills/*/SKILL.md`, but `documentation-maintenance.md`
makes them first-class. Two reconciled surfaces can plausibly appear there and MUST be verified:
- **`fleet show` / `fleet delete` invocation** (3.1) — after the code fix these take `--fleet-id`;
  the `cafleet` SKILL already documents `--fleet-id` for both, so it becomes correct, but verify.
- **Newly-visible flags** (3.4/3.5: `--quiet`, `--activity`, `--ansi`) — verify no SKILL/README
  text claims they do not exist.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Code-fix steps are test-first (write the failing regression test, then the fix).

### Step 1: SPEC.md doc-alignment edits (docs-first)

- [x] 2.3 — rewrite §5.4 kind-projection summary at `SPEC.md:362-366`, `514-517` <!-- completed: 2026-07-04T07:18 -->
- [x] 3.6 — `fleet list` is 5 columns (`40/40/20/8` padded + CREATED_AT trailing unpadded) at `SPEC.md:992-994` <!-- completed: 2026-07-04T07:18 -->
- [x] 4.4 — remove `coding_agent` from conditional-fields list at `SPEC.md:1652` <!-- completed: 2026-07-04T07:18 -->
- [x] 5.1 — "tmux-pane commands…" → "member commands…" at `SPEC.md:1668-1670`, `2379-2381` <!-- completed: 2026-07-04T07:18 -->
- [x] 5.2 — newline-only → newline/CR at `SPEC.md:1718-1721` <!-- completed: 2026-07-04T07:18 -->
- [x] 6.2 — `Continue`/`Stop` → `CONTINUE`/`STOP` at `SPEC.md:1829,1833-1834` (+1870,1873,1899,1917,1924) <!-- completed: 2026-07-04T07:18 -->
- [x] 6.3 — "four functions" → three at `SPEC.md:1841-1843` <!-- completed: 2026-07-04T07:18 -->
- [x] 7.3 — `/api/timeline` "all" → capped at 200 at `SPEC.md:2254-2255` <!-- completed: 2026-07-04T07:18 -->
- [x] 7.4 — send path also requires `status=="active"` at `SPEC.md:2259-2263` <!-- completed: 2026-07-04T07:18 -->

### Step 2: docs/ doc-alignment edits (docs-first)

- [ ] 1.3 — index order ascending at `docs/spec/data-model.md:92-93` <!-- completed: -->
- [ ] 2.5 — two-string not-found behavior at `docs/concepts/fleet-isolation.md:16-17` <!-- completed: -->
- [ ] 2.6 — module layout + re-exports + `list_roster` at `docs/api/broker.md:15-22,26-30` <!-- completed: -->
- [ ] 3.1(doc) — narrow blanket sentence at `docs/get-started/configure.md:110-111` <!-- completed: -->
- [ ] 3.2 — missing `--fleet-id` is exit-1 `ClickException` at `docs/spec/cli-options.md:1029` <!-- completed: -->
- [ ] 3.6(doc) — fix sample spacing at `docs/spec/cli-options.md:367-370` <!-- completed: -->
- [ ] 4.2 — `status_state` unconditionally omitted at `docs/spec/message-envelope.md:31` <!-- completed: -->
- [ ] 4.3 — default + `--full` agent render at `docs/concepts/token-reduction.md:25` <!-- completed: -->
- [ ] 5.1(doc) — error string at `docs/spec/cli-options.md:420` <!-- completed: -->
- [ ] 6.1 — heartbeat runs first at `docs/concepts/monitoring.md:180,194-196` <!-- completed: -->
- [ ] 7.1 — timeline scoped by sender at `docs/spec/webui-api.md:232` <!-- completed: -->
- [ ] 7.2 — `/api/fleets` shape incl `director_agent_id`, excludes soft-deleted at `docs/spec/webui-api.md:23-36` <!-- completed: -->
- [ ] 8.1 — add `--fleet-id` to `member capture` example at `docs/reference/coding-agents/opencode.md:84` <!-- completed: -->
- [ ] 8.2 — Codex allowlist: drop `cafleet agent`, add `member show` at `docs/get-started/configure.md:59,76-80` <!-- completed: -->
- [ ] 8.3 — full skill-install command at `docs/get-started/install.md:89` <!-- completed: -->
- [ ] 8.4 — `cafleet:format` runs `ruff check --fix` first at `docs/get-started/contributing.md:36` <!-- completed: -->

### Step 3: Code fix — `to_agent_id` nullable + NULL sentinel (1.1 + 1.2)

- [ ] Test: broadcast-summary row persists `to_agent_id = NULL`; `--json` emits `null` <!-- completed: -->
- [ ] Test: `to:` surfacing (`queries.py`, `formatters.py`) hides on `None`, shows on a real id <!-- completed: -->
- [ ] `db/models.py:73` → `nullable=True`; new Alembic migration (next rev after head) <!-- completed: -->
- [ ] `broker/messaging.py:218` → write `None`; `queries.py:68-70` + `formatters.py:39-40` → `is None`/`IS NULL` <!-- completed: -->

### Step 4: Code fix — broadcast `recipients` / `delivered` keys (2.1)

- [ ] Test: broker result carries separate `recipients` (N) and `delivered` (k); CLI prints `recipients=N delivered=k` <!-- completed: -->
- [ ] `broker/messaging.py:253-255` return both keys; `cli/message.py:70-75` print both <!-- completed: -->

### Step 5: Code fix — normalize broker guards to exit 1 (2.2 + 3.3)

- [ ] Test: root-Director deregistration guard and `member create` into soft-deleted fleet both exit 1 <!-- completed: -->
- [ ] `broker/agents.py:272-276` and `broker/agents.py:65-66` → `click.ClickException` <!-- completed: -->

### Step 6: Code fix — kind-predicate null/non-object safety (2.4)

- [ ] Test: kind predicates return `False` (no raise) on `{"cafleet": null}` and `{"cafleet":"x"}` <!-- completed: -->
- [ ] `broker/_shared.py:34-51` → guard null / non-object `cafleet` card <!-- completed: -->

### Step 7: Code fix — `fleet show` / `fleet delete` take `--fleet-id` (3.1 code)

- [ ] Test: `cafleet fleet show --fleet-id N` and `cafleet fleet delete --fleet-id N` succeed; positional form no longer accepted <!-- completed: -->
- [ ] `cli/fleet.py:82-86` + `105-107` → `@fleet_id_option`, read from `ctx.obj` <!-- completed: -->

### Step 8: Code fix — ping-age ASCII hyphen (4.1)

- [ ] Test: `_format_ping_age(None)` returns `"-"` (ASCII), no U+2014 anywhere in monitor-status render <!-- completed: -->
- [ ] `output/formatters.py:216-220` → return `"-"` <!-- completed: -->

### Step 9: Unhide the four flags (3.4 + 3.5)

- [ ] Test: `--full`, `--quiet`, `--activity`, `--ansi/--no-ansi` appear in the relevant `--help` output with help text <!-- completed: -->
- [ ] `_helpers.py:47,48` → drop `hidden=True`, add `help=` on `full_flag` + `quiet_flag`; `member.py:494,545` → drop `hidden=True`, add `help=` <!-- completed: -->
- [ ] Doc: document `--quiet` at `cli-options.md:146,899-901` + `SPEC.md:1007`; add `--activity` / `--ansi` to `cli-options.md` + SPEC §6.3 <!-- completed: -->

### Step 10: Cross-surface verification + gates

- [ ] Verify `README.md` and every `skills/*/SKILL.md` against the reconciled `fleet show`/`fleet delete` `--fleet-id` invocation and the newly-visible flags; align any drift <!-- completed: -->
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //admin:lint` all pass <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-04 | Initial draft — 32 contradictions reconciled per user decisions Q1–Q4 |
| 2026-07-04 | Approved — `--quiet` disposition finalized as UNHIDE; conditional REMOVE alternative removed |
