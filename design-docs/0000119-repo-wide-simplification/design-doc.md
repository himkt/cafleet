# Repo-wide behavior-preserving simplification

**Status**: Complete
**Progress**: 62/62 tasks complete
**Last Updated**: 2026-07-04

## Overview

Apply a repository-wide sweep of strictly behavior-preserving simplifications — dead-code removal, duplication collapse, stale-reference fixes, and redundant-test pruning — drawn from a verified 20-agent scan. Every change preserves the CLI, output formats, HTTP API, schema, and all documented contracts (README.md, SPEC.md, docs/, skills/), carrying the paired SPEC/docs/skills edit wherever a code removal would otherwise leave documented-contract drift.

## Success Criteria

- [x] All 56 in-scope findings (49 confirmed + 7 rescoped-includable) are applied, each traceable to its `scan/findings.md` id.
- [x] `mise //cafleet:test` passes with no newly-failing test; the only pass-set delta is the intentionally pruned tests (the removals in tests-1..8, broker-db-1, broker-db-3, output-webui-5, runtime-3), and every retained test preserves its production-behavior coverage.
- [x] `mise //cafleet:lint` and `mise //cafleet:typecheck` are clean.
- [x] `mise //admin:lint` and `mise //admin:build` succeed (for the admin/TypeScript changes).
- [x] No CLI flag, `--help` text, output byte, HTTP response, JSON key/order, SQLite schema, or documented error string changes.
- [x] SPEC.md, README.md, docs/, and skills/ remain mutually consistent after the paired doc edits (manual spot-check; no broken cross-references).
- [x] The whole sweep lands as a single PR referencing design 0000119.

---

## Background

A 20-agent scan (10 area scanners + 10 adversarial verifiers, 2026-07-04) swept the entire repository for simplification opportunities with no behavior change. The verified output lives at `scan/findings.md` in this design-doc directory: 66 findings, 49 confirmed and 17 refuted/rescoped. Each verifier note states why a finding was refuted or what paired SPEC/docs edit a change requires to stay contract-preserving.

This document turns the in-scope subset into an actionable single-PR plan. `scan/findings.md` is the authoritative evidence record — every task below cites its finding id, and the implementer consults that file for the full evidence and verifier caveats. This doc records the concrete change, the paired documentation edits, and the load-bearing caveats that keep each change behavior-preserving.

---

## Specification

### Scope

**In scope — 56 findings**: all 49 CONFIRMED findings, plus 7 RESCOPED-INCLUDABLE findings that are behavior-preserving once their paired SPEC.md / docs/ / skills edits are added (broker-db-1, broker-db-2, broker-db-3, broker-db-4, output-webui-6, docs-3, runtime-4).

**Per-area task counts**:

| Area | Confirmed | Rescoped-includable | Tasks |
|------|-----------|---------------------|-------|
| admin-ui | 1, 2, 3, 5 | — | 4 |
| broker-db | 5, 6, 7 | 1, 2, 3, 4 | 7 |
| cli | 1–8 | — | 8 |
| docs | 1, 4, 5, 6, 7 | 3 | 6 |
| output-webui | 1, 4, 5 | 6 | 4 |
| project-config | 2, 3, 4 | — | 3 |
| runtime | 1, 2, 3 | 4 | 4 |
| skills-cafleet | 1–8 | — | 8 |
| skills-workflows | 1, 5, 6, 8 | — | 4 |
| tests | 1–8 | — | 8 |
| **Total** | **49** | **7** | **56** |

### Out of scope — the 10 genuinely-refuted findings (excluded)

These are refuted by their verifiers as NOT behavior-preserving even with doc edits, or wrong as proposed. They are recorded here so future readers know they were deliberately considered and left out.

| Finding | Reason for exclusion |
|---------|----------------------|
| admin-ui-4 | Real UI behavior change — skips fleet re-validation, silently keeps the user on the dashboard for a soft-deleted fleet. |
| broker-db-8 | Proposed home (engine.py) has a module-level side effect (FK enforcement + busy_timeout registration) that would alter migration-connection behavior in isolated `cafleet.db.init` imports. Only a different, side-effect-free refactor would be safe. |
| docs-2 | Not deduplication — the two permission-allowlist recipes are alternative granularities, not two copies of one fact; the proposed change orphans the `skills/cafleet/SKILL.md:78` cross-reference. |
| output-webui-2 | Duplicate of the CONFIRMED cli-1 (same `client_command` dead-branch removal), already covered there. |
| output-webui-3 | Importing `cafleet.output.render` into `broker/messaging.py` adds a broker→output edge that contradicts SPEC's documented module-dependency graph (§ lines 174–206). |
| project-config-1 | `mise //:all-install` is an operator-facing entry point deliberately retained per design 0000110; deleting it removes a working command and changes `mise tasks` output. |
| skills-workflows-2 | Proposed anchor (`skills/cafleet/roles/member.md`) is not read by every spawned member, and the paragraph has real per-role deltas that a single canonical copy would drop. |
| skills-workflows-3 | The cited "canonical" pointers govern a different scenario; a one-line pointer would delete contract text that exists only in these two role files. |
| skills-workflows-4 | The `<unset>` branch differs semantically between execute.md (literal `$ARGUMENTS`) and create/interview (canonicalized path); the shared skeleton cannot be factored without changing execute's contract. |
| skills-workflows-7 | Resolving the text-overflow-priority contradiction necessarily eliminates one currently-documented behavior; standalone readers of `formatting.md` are affected, so it is behavior-relevant, not a no-op. |

### The behavior-preserving contract

A change qualifies only if, after it lands:

- CLI flags, `--help` text, argument parsing, exit codes, and output bytes are byte-identical.
- HTTP API routes, status codes, JSON payload keys, and key order are unchanged.
- The SQLite schema and emitted SQL result sets are unchanged (SQL *text* may differ where only the Python construction is deduplicated).
- Every documented contract surface (README.md, SPEC.md, docs/, skills/*/SKILL.md and their references) either is untouched or receives the paired edit that keeps it accurate in the same change.

**Paired-doc-edit principle**: the 7 rescoped-includable findings, plus several confirmed findings whose verifier notes flag a SPEC/docs mention, remove or change a symbol that a documented contract surface names. Each such task carries its SPEC.md / docs / skill edit inline — the removal and its documentation update land together, per `.claude/rules/removal.md` and `.claude/rules/documentation-maintenance.md`.

### Delivery and ordering

The whole sweep lands as **one PR** referencing design 0000119. Steps below are organized by area (mirroring `scan/findings.md`) for traceability, not as separate PRs. Two intra-PR ordering couplings to respect:

- **tests/cli fixtures** — tests-2 (delete 7 pass-through `_autouse_reset_engine`), tests-5 (consolidate the bootstrapped-fleet fixture into `tests/cli/conftest.py` and relocate `_mock_tmux_for_fleet_create` autouse→opt-in), and tests-7 (four deregister-guard tests adopt `db_runner`) all touch `tests/cli/`, and all three land on `test_fleet_flag.py` specifically (tests-2 deletes its `_autouse_reset_engine`, tests-5 moves its `_mock_tmux_for_fleet_create`, tests-7 rewires its deregister-guard tests). Apply tests-5's conftest additions before tests-7's fixture adoption; confirm the autouse `_cli_registry` interaction holds after tests-2; and re-run the whole `tests/cli/` suite once all three land, since the tmux-stub opt-in change (tests-5) affects every `fleet create` test in that file.
- **broker-db test overlaps** — broker-db-1 (prune `is_monitoring_member` tests) and broker-db-3 (migrate `record_ping` test call sites) edit test files that tests-3/4/8 also restructure. Coordinate edits to `tests/broker/test_monitor.py` so the shared-helper migrations and the dead-test prunes do not conflict.
- **shared documentation-file edits** — two contract files are each edited by many findings, so treat every task's cited SPEC/docs line number as an anchor-at-authoring-time that will DRIFT as earlier edits land. Re-locate each edit by its section/symbol at apply time rather than trusting the absolute line:
  - `docs/api/broker.md` (~line 22, the mkdocstrings-rendered public-API list) is edited by FOUR findings — broker-db-2 (drop `list_agents`), broker-db-3 (drop `record_ping`), broker-db-4 (drop `enroll_agent`), output-webui-1 (add `monitor_runtime_payload`).
  - `SPEC.md` is edited by NINE findings — broker-db-1 (§ Internal predicates / Kind predicates / fail-fast catalog), broker-db-2 (§6.2 broker layer), broker-db-3 (§ Monitor schedule-CRUD), broker-db-7 (§ heartbeat), cli-1 (§ client_command wrapper), output-webui-1 (§ broker-monitor surface), output-webui-4 (§ formatter private helpers), output-webui-6 (§ render-layer signatures), runtime-4 (§6.5 Multiplexer method surface).
  - Apply each file's edits in one coordinated pass so the collective diff is internally consistent.

### Acceptance gate per change class

| Change class | Gate |
|--------------|------|
| Python src | `mise //cafleet:test` green — no newly-failing test; the only pass-set delta is the intentionally pruned tests — plus `mise //cafleet:lint` + `mise //cafleet:typecheck` |
| admin / TypeScript | `mise //admin:lint` + `mise //admin:build` succeed |
| docs / skills | `mise //:docs-build` succeeds (mechanically catches broken internal links / mkdocstrings `::: cafleet.broker` render failures) + manual cross-reference spot-check that SPEC/README/skills stay mutually consistent (`/update-readme` is NOT mandated). The docs-build is a supplement, not a replacement — the hand-maintained `docs/api/broker.md` prose list still needs the manual check. |
| Test edits/deletions | "Retained tests preserve all production-behavior coverage" argued per deletion; no coverage-diff tooling required |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Each task cites its `scan/findings.md` id; consult that file for full evidence and verifier notes.

### Step 1: admin-ui

- [x] **[admin-ui-1]** Delete the unreachable `resp.status === 204` branch in `admin/src/api.ts:38-40`; no `/api` route returns 204. <!-- completed: 2026-07-04T11:04 -->
- [x] **[admin-ui-2]** Drop unused response-type fields: `type` from `TimelineMessage` (`admin/src/types.ts:32`) and `pid`/`tick_seconds`/`last_tick_at`/`last_tick_age_seconds`/`started_at` from `MonitorRuntime` (`types.ts:9-13`). Update the type-position uses at `Dashboard.tsx:29` and `api.ts:80`. Server JSON payloads stay unchanged (TS types are structural + erased). <!-- completed: 2026-07-04T11:04 -->
- [x] **[admin-ui-3]** Extract a `useRefreshKeyLoad(load, refreshKey)` hook (in `admin/src/hooks/`) that owns the in-flight ref + refreshKey effect; refactor `Timeline.tsx:101-124` and `AgentDetail.tsx:231-264` onto it. Caveats: keep `load` in the effect deps (AgentDetail's agent-switch reload depends on it); keep `setLoading(false)` in each component's own `finally`; keep the ref per-component-instance. Folding `usePolling.ts` in is optional. <!-- completed: 2026-07-04T11:04 -->
- [x] **[admin-ui-5]** Move the duplicated zero-padded HH:MM helpers into one module (e.g. `admin/src/format.ts`): shared `formatTime(iso)` with `formatDateTime(iso)` composed from it; import at `AgentDetail.tsx:32-36` and `TimelineMessage.tsx:39-43`. Byte-identical output. <!-- completed: 2026-07-04T11:04 -->

### Step 2: broker-db

- [x] **[broker-db-5]** Introduce module-level `_CONFIG_COLS = (MonitorConfig.agent_id, MonitorConfig.interval_seconds, MonitorConfig.last_ping_at, MonitorConfig.enabled)` in `broker/monitor.py`; build each of the three selects (`get_monitor_config` :102-111, `list_monitor_configs` :119-127, post-update re-read :172-179) as `select(*_CONFIG_COLS)` with its own WHERE/JOIN. Identical statements emitted. <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-6]** Add one `_shared.py` module-level expression `CARD_KIND_SQL = func.coalesce(func.json_extract(Agent.agent_card_json, "$.cafleet.kind"), "")` and use it at all five sites (`agents.py:95-96, 372-374`, `members.py:86-88`, `messaging.py:206-210`, `monitor.py:85-86`); comparisons against kind constants stay per-site. The two raw sites gain a `coalesce` wrap that selects identical rows (NULL and "" both non-match against a non-empty constant). <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-7]** Drop the no-op `pid=pid` from the `.values()` in `heartbeat_monitor_runtime` (`broker/monitor.py:329-335`), leaving `.values(last_tick_at=when)`; the WHERE already pins `pid == pid`. Paired doc edit: update SPEC.md:761-762 ("update pid and last_tick_at ... only where the current pid equals the caller's pid") to match the one-value update. <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-1]** (rescoped) Delete `is_monitoring_member` from `broker/_shared.py:59-60` (zero production callers; SQL `json_extract` does detection). Prune its dedicated tests: reduce `_PREDICATES` in `tests/broker/test_kind_predicate_safety.py` to `is_administrator` (the `_card_kind` malformed-card guard stays covered via `is_administrator`), and drop `test_monitor.py`'s `test_is_monitoring_member__kind_detection`. Paired SPEC edits: remove the three mentions at SPEC.md:371-373 (Internal predicates), :515-518 (Kind predicates), :2377-2378 (fail-fast catalog). <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-2]** (rescoped) Delete `broker.list_agents` (`agents.py:229-251`) and its import/`__all__` entries in `broker/__init__.py:8,52`; migrate the 8 test call sites to `broker.list_roster(fleet_id)` (same population, superset row shape) or assert against `list_fleet_agents`. Paired doc edits: SPEC.md §6.2 (remove the `list_agents` row shape at :598, and the kind-surfacing/soft-delete mentions at :367, :520, :789) and `docs/api/broker.md` (the mkdocstrings-rendered API + Re-export contract). <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-3]** (rescoped) Delete `record_ping` (`broker/monitor.py:201-203`) and its two `broker/__init__.py` lines (:38, :75); change the 5 test call sites to `broker.record_pings([agent_id], when)`. Paired doc edits: SPEC.md:737-738 (remove the "thin wrapper over record_pings" spec) and `docs/api/broker.md:22` (drop from the monitor.py public-API list). <!-- completed: 2026-07-04T12:20 -->
- [x] **[broker-db-4]** (rescoped) Remove `enroll_agent` from the import block and `__all__` in `broker/__init__.py:30,70`; the function stays in `monitor.py` where `agents.py:161` and `fleets.py:84` call it via the module. Paired doc edit: update `docs/api/broker.md` § Re-export contract (line 22 lists `enroll_agent` first among monitor.py's public API; the `::: cafleet.broker` directive renders `__all__`). SPEC.md:505/:719 stay accurate (the function survives). <!-- completed: 2026-07-04T12:20 -->

### Step 3: cli

- [x] **[cli-1]** In `cli/_helpers.py:93-151`, drop the `truncates_task_text` parameter (behavior always on), make `text_formatter` required, delete the three dead branches (:131-132, :139-140, :141-142) and the two-mode docstring (:99-107), and replace `kwargs.get("full", False)` (:127) with `kwargs["full"]`. Update the test-only commands in `tests/cli/test_client_command.py` to the single remaining mode. Paired SPEC edit: update SPEC.md:908-926 ("The client_command wrapper") to describe the single truncate+formatter mode (remove the optional-renderer / else-emit-JSON branches). <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-2]** Add `agent_id_option = click.option("--agent-id", type=int, required=True, help="Agent ID")` to `cli/_helpers.py` and apply at the six `message.py` sites (:24, :59, :90, :107, :128, :146). Keep the same decorator position for identical `--help` ordering. `member.py`'s differently-helped `--agent-id` declarations stay. <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-3]** Add a parametrized `text_body_options(text_help: str)` decorator (in `_helpers.py` or `_text_input.py`) applying `--text` then `--text-file`, passing the per-command `--text` help verbatim; use at the four sites (`message.py:26-32, 60-66`; `member.py:182-189, 690-696`). Apply `--text` before `--text-file` to preserve `--help` ordering; emit one normalized `--text-file` form (click infers STRING for `default=None`, so member create's `type=str` is equivalent). <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-4]** Factor a private `_http_get(req) -> bytes` in `cli/setup.py` owning the `urlopen` + shared HTTPError / (URLError, TimeoutError) except ladder (HTTPError before its URLError parent). `_resolve_download_url` (:61-76) wraps it to intercept the 404 case (:65-68); `_download_and_extract` (:97-108) calls it directly and does its `write_bytes` outside the helper (a plain OSError from the write is not caught today, so keeping the write at the call site is behavior-identical). Error strings and exit codes unchanged. <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-5]** Replace the literal `click.Choice(["claude", "codex", "opencode"])` at `cli/setup.py:206` with `click.Choice(list(AGENT_SKILLS_DIRS))` (dict at :24-28 has identical key order). `--help` metavar, accepted values, and invalid-choice errors byte-identical. <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-6]** Give `_emit_member_delete_output` (`cli/member.py:450-455`) an optional `header: str | None` where `None` means JSON-only (skip the text branch); have the timeout path (:419-424) call `_emit_member_delete_output(ctx, member_id, pane_status, header=None)` before `ctx.exit(2)`. JSON payload identical; text mode still prints nothing to stdout; exit code unchanged. <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-7]** Collapse the two-branch identical-error raise in `ensure_skills_current` (`cli/_helpers.py:24-34`) to `rows = list_skill_installs() if skill_installs_table_exists() else []` followed by one `if not rows: raise click.ClickException(...)`. Preserves the invariant that `list_skill_installs` never runs against a missing table; same exception type, message, exit code. <!-- completed: 2026-07-04T12:20 -->
- [x] **[cli-8]** Hoist a module-level `_json_flag = click.option("--json", "as_json", is_flag=True, help="Output as JSON.")` (in `fleet.py` or `_helpers.py`) applied at `fleet.py:27, 59, 84`; optionally a `_wants_json(ctx, as_json)` helper for the `if as_json or ctx.obj["json_output"]:` check repeated at :52, :65, :93. Parsing, `--help`, and branching unchanged. <!-- completed: 2026-07-04T12:20 -->

### Step 4: docs

- [x] **[docs-1]** In `docs/how-to/monitor-and-recover.md:11-45` and `docs/how-to/mixed-backend-team.md:33-41`, keep the actionable parts (the `member create --role monitor` command block + `monitor status` check) plus one linking sentence to `docs/concepts/monitoring.md`, and delete the restated monitoring mechanics (intervals, Esc semantics, wake-nudge details, teardown rules). Caveat: retain the `monitor-and-recover.md:23-24` pointer to the /cafleet skill's `roles/monitor.md` (it is the only docs/ mention of that pointer). <!-- completed: 2026-07-04T11:22 -->
- [x] **[docs-4]** In `docs/concepts/member-lifecycle.md`, keep the str.format + four-placeholder substitution sentence once (in § Atomic create flow, :52-57) and reduce § Spawn-prompt input modes (:74-79) to the `--text`/`--text-file`/stdin sentence plus its existing link to `cli-options.md`, dropping the repeated placeholder list. Optionally keep the half-sentence brace-doubling caveat to avoid losing the page-local note. <!-- completed: 2026-07-04T11:22 -->
- [x] **[docs-5]** In `docs/concepts/token-reduction.md:19`, replace the nonexistent `--id` flag with `--member-id` (or drop it) in the paste-target flag list. Sole `--id` occurrence in user-facing surfaces. <!-- completed: 2026-07-04T11:22 -->
- [x] **[docs-6]** Rewrite the `docs/get-started/contributing.md:18` project-structure row to reality, e.g. "| `skills/` | Coding-agent skill files (`cafleet`, `cafleet-design-doc`, `cafleet-research`), installed into the agent homes by `cafleet setup` / `mise //:skill-install`. |" — removing the last mention of the removed plugin-manifest mechanism. <!-- completed: 2026-07-04T11:22 -->
- [x] **[docs-7]** In `docs/reference/coding-agents/opencode.md`: trim :62 to "cafleet has been validated against `opencode 1.15.5`; `1.15.5` is also the minimum supported version." and replace the design-doc "Step 0 GATE" pointer at :130-132 with a self-contained instruction (e.g. "STOP and inspect `~/.opencode/agents/cafleet.md` — the preset's deny ruleset is not being applied."). Preserves the validated version, minimum-version statement, and STOP-on-regression instruction. <!-- completed: 2026-07-04T11:22 -->
- [x] **[docs-3]** (rescoped) Delete the README.md §5 "Message broker operations (CLI)" table (:214-222) and fold its one unique fact ("broadcast fans out to all fleet agents excluding the Administrator") into the §4 `message broadcast` row (:171-180). Paired skill edit: amend `.claude/skills/update-readme/SKILL.md` README-structure spec (line ~47, section 6 "API Overview") so the mandated structure no longer requires the deleted table (or retain a minimal message-broker-operations pointer under API Overview) — otherwise the next `/update-readme` run would reintroduce it. <!-- completed: 2026-07-04T11:22 -->

### Step 5: output-webui

- [x] **[output-webui-1]** Extract a shared `broker.monitor_runtime_payload(fleet_id, now)` (natural home: `broker` next to `read_monitor_runtime`/`monitor_is_live`) returning the runtime dict; call it from `cli/monitor.py:74-101` (`monitor_status`) and `webui/api.py:24-53` (`_monitor_runtime_payload` / GET /api/monitor). Same keys, key order, null-when-stale semantics → byte-identical CLI JSON and identical FastAPI JSON. Paired doc edits (additive): add the helper to `docs/api/broker.md:22` and the SPEC broker-monitor surface (~:770). <!-- completed: 2026-07-04T12:20 -->
- [x] **[output-webui-4]** Delete `_agent_id_for_column` (`output/formatters.py:164-165`) and inline `str(m["agent_id"])` at its two callers (:185, :325), matching `format_member_roster` (:303) and `format_monitor_status` (:255). Rendered table text byte-identical (padding stays at call sites). Paired doc edit: trim the "agent-id stringifier" mention in SPEC.md:1472-1473 (Private contract helpers prose). <!-- completed: 2026-07-04T12:20 -->
- [x] **[output-webui-5]** Delete `test_format_indexed_list__empty_and_non_empty_join_behaviour` (`tests/output/test_indexed_list.py:6-20`); its behaviors are covered by `test_compact_formatters.py:145-157/160-167`. If the lazy-eval assertions (formatter not called on empty input; `formatter_calls == ["a","b","c"]` exactly-once-in-order) are deemed load-bearing, fold them into the surviving test. Tests-only. <!-- completed: 2026-07-04T11:06 -->
- [x] **[output-webui-6]** (rescoped) Drop the `limit` parameter from `truncate_task_text` (`output/render.py:56-66`) — it forwards `limit=None` implicitly via `truncate_text`'s `settings.max_text_len` default; the only src caller (`_helpers.py:129`) never passes it. Rewire the six `tests/output/test_output.py` call sites (:135, :143, :148, :154, :171, :176) to set the limit via the settings mechanism (as `test_truncation_settings.py` does). Paired SPEC edits: update the `truncate_task_text(result, full, limit)` signature at SPEC.md:1463 and the truncation-rule prose at :1482. <!-- completed: 2026-07-04T12:20 -->

### Step 6: project-config

- [x] **[project-config-2]** Remove `"@slidev/theme-seriph": "^0.25.0"` from `package.json:13` and refresh `bun.lock`. No tracked deck/skill/config selects seriph; the research decks pin the repo-local custom theme; `@slidev/theme-default` stays as the implicit fallback. <!-- completed: 2026-07-04T12:26 -->
- [x] **[project-config-3]** Delete the two subsumed deny entries at `.claude/settings.json:38-39` (`Bash(bun run agent-browser wait --load networkidle)` and its `*` variant) — both are covered by line 46 `Bash(bun run agent-browser wait *)`. Keep the session-scoped lines 36-37 (NOT subsumed) and lines 44-46. Deny outcome identical; the prose in `.claude/rules/commands.md` stays true. <!-- completed: 2026-07-04T12:26 -->
- [x] **[project-config-4]** In `.bumpversion.toml`, drop `parse`, `serialize`, `allow_dirty`, `message`, and `allow_shell_hooks` (:3-8) — all equal bump-my-version 1.3.0 defaults (`allow_shell_hooks` defaults TRUE and gates nothing with no hooks defined). Keep `current_version`, `commit = true` (non-default), and both `[[tool.bumpversion.files]]` blocks. Byte-identical bump results. <!-- completed: 2026-07-04T12:26 -->

### Step 7: runtime

- [x] **[runtime-1]** Add a module-level `_best_effort_send(*, target_pane_id, payload, esc_first=False) -> bool` in `multiplexer/tmux.py` holding the `shutil.which` pre-check, the `_send_literal_then_enter(..., timeout=5, esc_first=esc_first)` call, and the `TmuxError→False` conversion; reduce `send_poll_trigger`/`send_wake_trigger`/`send_inline_preview` (:203-306) to payload construction + one helper call. `esc_first=False` default preserves `send_wake_trigger`. Existing monkeypatch-based tests pass unchanged (which pre-check stays inside the helper). <!-- completed: 2026-07-04T12:20 -->
- [x] **[runtime-2]** Drop `poll_until_pane_gone` from `multiplexer/__init__.py` (import :4, `__all__` :18) and `ensure_binary_on_path` from `coding_agent/__init__.py` (:1, :18). Both stay in their `base` modules where every real consumer imports them; zero package-path importers exist. SPEC does not document `__all__`/re-export surfaces — no SPEC drift. <!-- completed: 2026-07-04T12:20 -->
- [x] **[runtime-3]** Delete the byte-duplicated spawn-argv / validate_model tests, keeping the parametrized pinned-argv tests in `test_protocol.py` as the single source of truth: remove `test_protocol.py`'s claude/codex standalone byte-exact tests and the codex ordering test (:62-72, :95-117), and the five duplicated opencode tests in `test_opencode.py` (:33-43, :51-62, :148-150, :180-193). Scope caveat: KEEP `test_opencode_agent_name_and_binary_name`'s `binary_name == "opencode"` assertion (no direct retained duplicate — fold it into a kept test rather than deleting outright) and keep the opencode-only regression guards (`run`/`--interactive`/`--dangerously-skip-permissions` absence, `--prompt` positioning, display_name-equivalence). <!-- completed: 2026-07-04T11:06 -->
- [x] **[runtime-4]** (rescoped) Remove the `pane_exists` declaration + docstring from the `Multiplexer` Protocol in `multiplexer/base.py:90-92`; keep concrete `TmuxMultiplexer.pane_exists` (used by `wait_for_pane_gone` and tests). `isinstance` structural check still passes; no code calls `pane_exists` via a `Multiplexer`-typed reference. Paired SPEC edit: remove `pane_exists` from the Multiplexer method surface in SPEC.md §6.5 (~:1738). Note: this is a contract narrowing of the reimplementation interface — apply the SPEC edit in the same change. <!-- completed: 2026-07-04T12:20 -->

### Step 8: skills-cafleet

- [x] **[skills-cafleet-1]** In `skills/cafleet/reference/supervision.md`, keep the full Director re-engagement statement once at § The monitor heartbeat (:33) and reduce the § Communication Model "Facilitation cue" copy (:19) to its one load-bearing sentence + a "see § The monitor heartbeat" pointer. Caveat: preserve each copy's unique clause — :19's "run the entire 5-step loop, NOT read-and-stop" bolded sentence and its loop-wakes-only-the-monitor parenthetical; :33's "the monitor decides *when*; this file defines *what*" closer. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-2]** Make `skills/cafleet/SKILL.md` § Spawned-member identity the owner of the str.format substitution spec. Collapse `reference/director.md:102` to a pointer plus its THREE deltas (unknown-placeholder UsageError lists the four names; malformed-brace "double literal braces" UsageError; just-registered-agent rollback), and drop the repeated placeholder enumeration/brace rule from `director.md:137` (keep an antecedent for "the four identity placeholders"). <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-3]** Consolidate the "Long or multi-line bodies MUST use --text-file" rule: `supervision.md:17` OWNS it (the monitoring member reads supervision.md and is the primary `member nudge` sender), and `reference/director.md:106` § Member Create points to it. (Reverse of the surface-level "keep director.md" reading — supervision.md is in every relevant reader's required path.) <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-4]** Correct the `skills/cafleet/reference/broadcast.md:10-13` flag table to the exactly-one-of `--text` / `--text-file` contract (mirroring `SKILL.md:124`), adding the missing `--text-file` row and removing the false `--text` required=yes. Docs-accuracy fix describing existing CLI behavior. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-5]** In `supervision.md`, keep the poll semantics once at Stall Response Stage 1 (:143, which carries "newest first" + "no last-tick timestamp to track") and trim facilitation step 1 (:113) to the command + a back-reference. Caveat: step 1's trimmed form must keep the "(step 2)" ACK-consumes linkage coherent. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-6]** In `supervision.md`, make § Escalation (:158-160) the owner of the "2 nudges → escalate via {decision_surface}" threshold + its capture-corroboration condition + options list; step 5 (:117) keeps only the "new user decision" trigger + a pointer to § Escalation, and retains its "no passive-hold messages" prohibition. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-7]** In `supervision.md`, collapse § Cleanup Protocol (:181) to the single pointer sentence "Cleanup follows recovery.md § Shutdown Protocol" and keep the first-out fact + race rationale once in the Monitor Lifecycle table row (:127). Keep the § Cleanup Protocol heading (SKILL.md:97 advertises it). `recovery.md:43-51` stays the sole full ordering. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-cafleet-8]** In `supervision.md`, keep the monitor-first spawn + "Director never runs monitor start" statement in full once at § Spawn Protocol (:83) and reduce the :31 and :46 restatements to short forward references. Keep `SKILL.md:95` as the dispatch-level Team-supervision summary (the frontmatter and `roles/director.md:16` lean on it). Realistic savings are modest — :31/:46 are single sentences in otherwise-unique paragraphs. <!-- completed: 2026-07-04T11:53 -->

### Step 9: skills-workflows

- [x] **[skills-workflows-1]** Delete the trailing bare `$ARGUMENTS` line from `interview/interview.md:255`, `cafleet-research/report/report.md:284`, and `cafleet-research/presentation/presentation.md:305` (a slash-command-era leftover; these are Read-loaded sub-pages where no interpolation occurs). Prose `$ARGUMENTS` references untouched. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-workflows-5]** In the four workflow bodies (`create/create.md:104`, `interview/interview.md:115`, `report/report.md:87`, `presentation/presentation.md:104`), keep each body's monitor gate sentence + teardown-tail sentence and replace the repeated render-instructions + `member create --role monitor` code block with the pointer to the cafleet skill's `roles/monitor.md` (as `execute/execute.md:167` already does). The command/prompt are canonical there. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-workflows-6]** Reduce the spawn-prompt audit-file blockquote in the five workflow bodies (`create/create.md:125`, `execute/execute.md:193`, `interview/interview.md:131`, `report/report.md:129`, `presentation/presentation.md:124`) to the `${BASE}/prompts/<role>-<UTC-compact>.md` path template + the existing pointer to base-dir.md § No-bypass write protocol / director.md § Member Create, which already carry the collision rule, placeholder-pre-substitution note, and `<unset>` guarded-skip branch. Keep the pointer so the contract is read before any spawn. <!-- completed: 2026-07-04T11:53 -->
- [x] **[skills-workflows-8]** Rephrase `create/roles/reviewer.md:3` to "provide specific, actionable feedback as inline `COMMENT(reviewer)` markers, signalled via `cafleet message send`", and rewrite the first sentence of § Iterative Improvement Loop (:64) to the marker-based flow (matching `execute/roles/reviewer.md:66-70`). Keep the already-correct second half of :64. Removes stale pre-schema wording that contradicts the canonical verb+pointer+marker protocol. <!-- completed: 2026-07-04T11:53 -->

### Step 10: tests

- [x] **[tests-1]** DELETE the dead helper `_make_alembic_cfg` (`tests/_helpers.py:15-21`) and its then-unused `importlib.resources` / `alembic.config.Config` imports. Do NOT factor the 5 inline sites onto it — all five run alembic ops INSIDE the `as_file` context whereas the helper exits it before returning; keep the inline blocks. <!-- completed: 2026-07-04T11:06 -->
- [x] **[tests-2]** Delete the seven pass-through `_autouse_reset_engine` fixtures (`tests/cli/test_fleet_flag.py:15-17`, `test_monitor.py:32-34`, `test_setup.py:123-125`, `test_doctor.py:30-32`, `test_skills_guard.py:37-39`, `test_fleet_bootstrap.py:18-20`, `test_fleet.py:15-17`). The autouse `_cli_registry` (conftest.py:9-24) already requests `_reset_engine_singletons` for every CLI test; pytest caches one instance per test. <!-- completed: 2026-07-04T12:20 -->
- [x] **[tests-3]** Replace the four local `_create_fleet` copies (`tests/broker/test_inline_preview.py:40-45`, `test_administrator.py:14-20`, `webui/test_api_format.py:31-35`, `webui/test_monitor_api.py:51-55`) with `from tests.broker._helpers import _create_fleet` (test_administrator keeps its name via `_create_fleet_with_ctx = _create_fleet`; its `_FAKE_DIRECTOR_CTX` goes with it). Cross-directory import pattern already established. <!-- completed: 2026-07-04T12:20 -->
- [x] **[tests-4]** Add `tests/broker/conftest.py` with the single autouse `_autouse_broker(broker_session)` fixture and delete the 13 in-file copies in `tests/broker/`. The 3 copies outside `tests/broker/` (monitor/, output/, webui/) stay or get their own one-liner conftest. `test_kind_predicate_safety.py` gains an unused in-memory sessionmaker patch (no observable effect); if unwanted, keep that file's own fixture and still delete 12. <!-- completed: 2026-07-04T12:20 -->
- [x] **[tests-5]** Move to `tests/cli/conftest.py`: (a) the `_mock_tmux_for_fleet_create` stub fixture (kept opt-in, requested by users — not dir-wide autouse) and the shared `_FAKE_DIRECTOR_CTX`, and (b) a bootstrapped-fleet factory fixture invoking `fleet create --json`; drop the per-file `database_url` redirect + `_init_registry` lines (autouse `_cli_registry` already provides a fresh seeded DB). Caveats: existing call sites return different tuple shapes/orders (test_member.py 3-tuple, member_show/list_all 6-tuples, capture_defaults string 5-tuple, make_bootstrapped_fleet a factory with an extra HOME redirect + `--coding-agent` handling) — per-file wrappers/edits are required to match each; cli/test_monitor.py's `fresh_db` returns `db_file` for direct SQL seeding, so if its redirect is dropped switch those reads to `_cli_registry`'s `db_path`. Autouse→opt-in migration (do NOT skip): `_mock_tmux_for_fleet_create` is currently module-level `autouse=True` in `test_fleet_flag.py:20-31` and `test_monitor.py:37-45`; relocating it to an opt-in conftest fixture removes its automatic application, so every current test in those two files that exercises a `fleet create` path MUST now request `_mock_tmux_for_fleet_create` by name (audit each test in both files and add the explicit request wherever a `fleet create` runs). <!-- completed: 2026-07-04T12:20 -->
- [x] **[tests-6]** Delete `test_should_ping__due_when_never_pinged` (`tests/monitor/test_should_ping.py:36-38`); the retained `test_should_ping__last_ping_none_due_immediately` (:94-96) asserts the identical expression on the identical input (`_target()` defaults `last_ping_at=None`). Optionally drop the redundant first assert of `test_should_ping__pinged_regardless_of_pending_count` (:52), keeping the `pending_count=5` assert. <!-- completed: 2026-07-04T11:06 -->
- [x] **[tests-7]** Have the four deregister-guard tests in `tests/cli/test_fleet_flag.py` (:204-214, :232-242, :262-272, :292-302) use the existing `db_runner` fixture instead of inline setup. Caveat: for the fourth test (`__cli_deregister_admin_leaves_row_active`, needs `_fetch_agent_status`), do NOT combine `db_runner` + `_cli_registry` (they redirect to different DBs) — instead extend `db_runner` to return `(runner, db_file)` (updating its other call sites) OR drop `db_runner` for that test and rely on `_cli_registry`'s `db_path` alone. The three tests that never touch `db_file` simply take `db_runner`. <!-- completed: 2026-07-04T12:20 -->
- [x] **[tests-8]** Add `_member_placement(director_agent_id, pane_id, coding_agent="claude")` and `_register_monitoring_member(fleet, name, pane_id)` to `tests/broker/_helpers.py`; replace the local placement copies/inline dicts (`test_monitor.py:40-47`, `monitor/test_loop.py:32-39`, `webui/test_monitor_api.py:58-70`, `broker/test_inline_preview.py:48-60`, `cli/test_member_show.py:55-79`, `cli/test_member_list_all.py:55-79`, `cli/test_member_capture_defaults.py:42-53`). Caveats: the three monitoring-member copies differ in return type (full dict vs `['agent_id']`) and defaults — pick one canonical return + adjust call sites; the cli inline sites use different names/kinds/descriptions and assert on those exact literals + pane ids, so share only the placement builder there and keep each site's own `register_agent` kwargs. <!-- completed: 2026-07-04T12:20 -->

### Step 11: Final verification & delivery

- [x] Run `mise //cafleet:test` — no newly-failing test; the only pass-set delta is the intentionally pruned tests. <!-- completed: 2026-07-04T12:44 -->
- [x] Run `mise //cafleet:lint` and `mise //cafleet:typecheck` — clean. <!-- completed: 2026-07-04T12:44 -->
- [x] Run `mise //admin:lint` and `mise //admin:build` — succeed. <!-- completed: 2026-07-04T12:44 -->
- [x] Run `mise //:docs-build` — succeeds (no broken internal links / mkdocstrings render failures). <!-- completed: 2026-07-04T12:44 -->
- [x] Manual spot-check: no broken cross-references; SPEC.md / README.md / docs/ / skills/ mutually consistent after the paired edits. <!-- completed: 2026-07-04T12:44 -->
- [x] Open a single PR referencing design 0000119 with the whole sweep. <!-- completed: 2026-07-04T12:44 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-04 | Initial draft — 56 in-scope findings (49 confirmed + 7 rescoped-includable), 10 excluded, grouped by area, single-PR delivery. |
| 2026-07-04 | Implemented — all 56 findings applied across 7 commits; fresh Reviewer approved (behavior-preserving, no markers); gates green (cafleet:test 968 passed, lint/typecheck/admin:lint/admin:build/docs-build clean); PR #164. Status → Complete. |
