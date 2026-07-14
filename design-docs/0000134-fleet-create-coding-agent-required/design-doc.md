# Require an Explicit Coding Agent: Mandatory `--coding-agent` on `fleet create`, Director Inheritance on `member create`

**Status**: Complete
**Progress**: 33/33 tasks complete
**Last Updated**: 2026-07-14

## Overview

`cafleet fleet create` silently records `claude` as the Director's backend when `--coding-agent` is omitted, so a Director running on codex or opencode is misrecorded — and the monitoring member then inherits that wrong value (GitHub issue #195). This design removes every silent `claude` default: `--coding-agent` becomes required on `fleet create`, `member create` resolves an omitted flag by inheriting the Director's recorded backend for all roles, the DDL `server_default` is dropped, and every skill page instructs the Director to pass the backend it is actually running on.

## Success Criteria

- [x] `cafleet fleet create --name x` (flag omitted) exits 2 with Click's `Missing option '--coding-agent'`; choices remain `claude` / `codex` / `opencode`.
- [x] `cafleet member create` with `--coding-agent` omitted records the Director's placement backend for **every** role (ordinary member and monitor alike); an explicit flag still wins.
- [x] No coding-agent `"claude"` default remains in `cafleet/src/cafleet/cli/` or `cafleet/src/cafleet/db/models.py` — neither `default="claude"` at the Click layer nor `server_default="claude"` in the model. The historical migration chain under `db/alembic/versions/` is exempt: `0001` is immutable and `0002`'s `downgrade()` restores the literal by design.
- [x] Every `cafleet fleet create` invocation in `docs/` and `skills/` shows `--coding-agent <backend>`, and the skill pages carry the instruction that the Director substitutes the backend it is actually running on.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

The observed symptom (issue #195): `cafleet member list` always shows `backend claude` for the Director, regardless of the coding agent actually driving the director pane.

Current behavior:

| Surface | Behavior today |
|---|---|
| `fleet create --coding-agent` | `cafleet/src/cafleet/cli/fleet.py:24-31` — `default="claude"`, `show_default=True`. Omitting the flag records `claude` as the root Director's `member_placements.coding_agent`. |
| `member create --coding-agent` | `cafleet/src/cafleet/cli/member.py:139-177` (`_resolve_coding_agent`) — explicit flag wins; `--role monitor` inherits the Director's placement backend (design 0000101); an ordinary member defaults to `claude`. |
| DDL | `db/models.py:55-57` and migration `0001_initial_schema.py:73` — `member_placements.coding_agent` carries `server_default="claude"`. Every writer passes an explicit value, so this default never fires at runtime. |
| Skills / docs | Most `fleet create` examples omit the flag (quickstart, `skills/cafleet/reference/cli.md`, every cafleet-design-doc and cafleet-research workflow body), so Directors on any backend copy an invocation that records `claude`. |

The monitor-inheritance path (design 0000101) is behaviorally correct — it propagates a recorded value — but the value it propagates is wrong whenever the fleet was created with the flag omitted on a non-claude Director. The user's direction: eliminate the silent `claude` default everywhere, at both the CLI layer and the instruction (skill) layer.

---

## Specification

### 1. `fleet create`: `--coding-agent` becomes required

`cafleet/src/cafleet/cli/fleet.py` — the option keeps its choices and loses its default:

```python
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(list(CODING_AGENTS.keys())),
    required=True,
    help="Coding-agent binary the Director is actually running on.",
)
```

- **Error contract**: omitting the flag produces Click's standard missing-option error for a required `Choice` option (`Choice.get_missing_message` appends the choices), exit 2, after the auto-generated usage block. No custom error message. The full stderr message:

  ```
  Error: Missing option '--coding-agent'. Choose from:
  	claude,
  	codex,
  	opencode
  ```

  Verify the literal output once during implementation (it is Click-version-dependent in whitespace) and record it verbatim in SPEC.md and `docs/spec/cli-options.md`, which treat error strings as exact contract. Test assertions match on the `Missing option '--coding-agent'` substring.
- The help text names the enforcement intent: the operator states the backend of the agent occupying the director pane.
- `broker.create_fleet` is unchanged (`coding_agent` is already a required parameter; its docstring note that the default lives at the Click layer is deleted).

### 2. `member create`: an omitted flag inherits the Director's backend for all roles

> **⚠ Design decision (per user direction to eliminate the silent `claude` default on `member create`)**: when `--coding-agent` is omitted, **every** member — ordinary and monitor alike — inherits the spawning Director's placement backend. The alternative (making the flag required on `member create` too) was rejected: it would break every skill workflow's `member create` invocations, diverge from the established monitor-inheritance path, and force redundant flags on homogeneous teams. Inheritance is a legitimate default per `affirmative-writing.md` — absence is an expected state whose correct value is a real recorded fact (the Director's own backend), not a hardcoded constant.

`_resolve_coding_agent` (`cafleet/src/cafleet/cli/member.py:139-177`) drops its `role` branch entirely and no longer takes `role`:

```python
def _resolve_coding_agent(
    coding_agent: str | None,
    director_member_id: int,
    fleet_id: int,
) -> str:
    """Resolve the backend for a new member.

    An explicit ``--coding-agent`` always wins. When the flag is omitted,
    the member inherits the spawning Director's backend from its placement row.
    """
    if coding_agent is not None:
        return coding_agent
    ...  # existing Director fetch + the three fail-loud surfaces, unchanged in structure
    return placement["coding_agent"]
```

- The three fail-loud error surfaces (Director fetch failure / Director not found / no placement row) are retained and now apply to every role. Their messages generalize `the monitor's coding agent` → `the member's coding agent`:
  - `cannot resolve the member's coding agent: failed to fetch Director <director-id>: <exc>. Re-run with an explicit --coding-agent.` (exit 1)
  - `cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` (exit 1)
  - `cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` (exit 1)
- The Click option keeps `default=None` and updates `show_default="inherits the Director's backend"`.
- Because `fleet create` now always records an explicit, operator-declared backend, inheritance propagates a correct value by construction.

### 3. DDL: drop `server_default="claude"` on `member_placements.coding_agent`

- `db/models.py`: `coding_agent: Mapped[str] = mapped_column(String, nullable=False)` — no `server_default`.
- New Alembic migration `0002` generated via `mise //cafleet:makemigration "drop coding_agent server default on member_placements"` (DB at head first via `cafleet setup db`). Autogenerate does not diff server defaults, so hand-edit the generated file per `.claude/rules/database-migrations.md`:
  - `upgrade()`: `with op.batch_alter_table("member_placements") as batch_op: batch_op.alter_column("coding_agent", existing_type=sa.String(), existing_nullable=False, server_default=None)`.
  - `downgrade()`: same batch alter restoring `server_default="claude"`.
  - FK safety: `member_placements` is a leaf table (no table FK-references it), so the SQLite batch recreate is safe on a populated DB; the recreate copies rows, keeping the migration data-preserving.
- Chain-guard updates in `cafleet/tests/db/test_alembic_smoke.py`:
  - `test_single_initial_migration_revision_exists` → rename to reflect a 2-revision chain; assert `len(revisions) == 2`, revision `0002` with `down_revision == "0001"`, head `0002`.
  - `test_alembic_version_table_records_head_0001` → asserts head `0002` (rename to match).
  - `test_member_placements_table_created_by_migration`: add the assertion that `coding_agent` declares no column default (`cols["coding_agent"]["default"] is None`).

### 4. Skill and Director enforcement: pass the backend you are running on

Prose instruction only — no new overlay token. Every skill page whose `fleet create` invocation omits the flag changes to show it, with one shared instruction sentence reusing the existing "identify your coding agent" convention:

> `--coding-agent <backend>` — substitute the coding agent you are actually running on: your spawn prompt's `CODING AGENT:` line names it; a standalone Director uses its own identity (e.g. Claude Code → `claude`).

Applied to (each invocation gains `--coding-agent <backend>` plus the instruction at first use on the page):

| Page | Site |
|---|---|
| `skills/cafleet/reference/cli.md` | `fleet create` example (line 135) and the backends sentence (line 43, "claude (default)") |
| `skills/cafleet/reference/director.md` | member-create `--coding-agent` row (line 30: ordinary-member default → inheritance), model-inference table "default backend" claims (lines 46, 54) |
| `skills/cafleet/roles/monitor.md` | line 99 — "omit `--coding-agent`" stays valid; reword from monitor-only exception to the general inheritance rule |
| `skills/cafleet-design-doc/create/create.md`, `execute/execute.md`, `interview/interview.md` | `fleet create` invocations (lines 91 / 156 / 105) + roles/director.md prose copies |
| `skills/cafleet-research/report/report.md`, `presentation/presentation.md` | `fleet create` invocations (lines 78 / 93) + both `roles/director.md` inline copies (line 20) |

Per `removal.md`, no page narrates the removal — every page states only the current behavior (flag required on `fleet create`; omitted flag on `member create` inherits the Director's backend).

### 5. Contract surface changes (SPEC.md and docs/spec)

| Contract | Before | After |
|---|---|---|
| `fleet create --coding-agent` (SPEC.md:995-997, `docs/spec/cli-options.md:277`) | choice, default `claude`, shown in help | choice, **required**; omitted → the full `Missing option '--coding-agent'. Choose from:` message (§1), exit 2 |
| `member create --coding-agent` resolve (SPEC.md:1062-1065, `docs/spec/cli-options.md:496`) | explicit wins; non-monitor → `claude`; monitor inherits Director | explicit wins; omitted → inherit Director's placement backend (all roles), three fail-loud surfaces |
| Resolve error strings (`docs/spec/cli-options.md:724-725` + SPEC equivalents) | `cannot resolve the monitor's coding agent: …` | `cannot resolve the member's coding agent: …`; rows no longer scoped to `--role monitor` |
| DDL default (SPEC.md:290, 465, 2667) | `coding_agent` DDL default `"claude"` | no DDL default; migration chain `0001 → 0002` |
| Release checklist (SPEC.md:2755) | `--coding-agent`=claude | `--coding-agent` required |
| Error table addition (`docs/spec/cli-options.md` §errors + SPEC) | — | `fleet create` with `--coding-agent` omitted → the full Click missing-option message from §1, recorded verbatim (exit 2) |

`docs/spec/data-model.md` needs no change (its mermaid comments list the value choices, not a default).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation first (per `.claude/rules/documentation-maintenance.md`), then skills, then code and tests.

### Step 1: docs/

- [x] `docs/spec/cli-options.md`: fleet-create `--coding-agent` row (line 277) → required + the full missing-option message from §1 (verbatim, including the `Choose from:` choices list); member-create row (line 496) → inheritance wording; error-table rows 724-725 → generalized strings, drop the `--role monitor` scoping; add the fleet-create missing-option error row <!-- completed: 2026-07-14T22:45 -->
- [x] `docs/concepts/coding-agents.md`: line 8 ("claude … the default") and lines 16-22 (operator-declared metadata + ordinary-member default) → required flag on `fleet create`, inheritance on `member create` <!-- completed: 2026-07-14T22:46 -->
- [x] `docs/reference/coding-agents/claude.md`: drop the "default backend" framing (lines 7, 13, 23); worked example line 50 keeps `--coding-agent claude`; line 57 ("backend defaults to claude, no --coding-agent needed") → member-create example relies on inheritance from the claude Director <!-- completed: 2026-07-14T22:47 -->
- [x] `docs/reference/coding-agents/codex.md` and `opencode.md`: update "default backend" cross-refs (line 21 in each); fleet-create examples (codex.md:82, opencode.md:108) already pass `--coding-agent claude` — verify they still describe the Director's actual backend and adjust prose <!-- completed: 2026-07-14T22:48 -->
- [x] `docs/reference/coding-agents/index.md`: line 12 "the default backend" → the required, operator-declared backend; line 18 selection prose <!-- completed: 2026-07-14T22:49 -->
- [x] `docs/get-started/quickstart.md`: line 198 `cafleet fleet create --name "demo"` → add `--coding-agent claude` with a sentence stating the operator passes the backend the Director runs on <!-- completed: 2026-07-14T22:50 -->
- [x] `docs/how-to/mixed-backend-team.md`: verify the fleet-create example (line 64, already explicit) and surrounding prose match the required-flag contract <!-- completed: 2026-07-14T22:51 -->
- [x] `docs/how-to/use-the-webui.md`, `docs/concepts/*` fleet-create prose mentions: sweep for any statement of the `claude` default and align <!-- completed: 2026-07-14T22:51 -->

### Step 2: SPEC.md

- [x] Fleet-create contract (lines 995-1000): `--coding-agent` required; record the full missing-option message from §1 verbatim + exit 2 <!-- completed: 2026-07-14T22:56 -->
- [x] Resolve-coding-agent contract (lines 1062-1065) and member-create options (line 1072): inheritance for all roles, generalized error strings <!-- completed: 2026-07-14T22:56 -->
- [x] Data-model rows (lines 290, 463-465), the §11 list (line 2667), the §11 migration-chain lead-in (line 2653, "A single initial revision, `0001` …"), and the decisions bullet (lines 2817-2818, "a single initial revision (`0001`) …"): no DDL default; document the `0001 → 0002` migration chain at every site <!-- completed: 2026-07-14T22:56 -->
- [x] `create_fleet` broker contract (lines 529-538): remove the Click-layer-default note <!-- completed: 2026-07-14T22:56 --> <!-- verified: SPEC.md's create_fleet bullet carries no Click-layer note; the note lives only in broker/fleets.py's docstring (Step 4) -->
- [x] Release checklist (line 2755): `--coding-agent` required on `fleet create` <!-- completed: 2026-07-14T22:56 -->

### Step 3: skills/

- [x] `skills/cafleet/reference/cli.md`: fleet-create example (line 135) gains `--coding-agent <backend>` + the instruction sentence; line 43 drops "(default)" <!-- completed: 2026-07-14T23:03 -->
- [x] `skills/cafleet/reference/director.md`: member-create `--coding-agent` row (line 30) → inheritance for all roles; "default backend" claims in the model-inference table (lines 46, 54); monitor-inheritance restatements (lines 32, 136) <!-- completed: 2026-07-14T23:03 -->
- [x] `skills/cafleet/roles/monitor.md` line 99: reword the omit-flag note to the general inheritance rule <!-- completed: 2026-07-14T23:03 -->
- [x] `skills/cafleet-design-doc`: create/create.md:91, execute/execute.md:156, interview/interview.md:105 fleet-create invocations + create/execute roles/director.md prose <!-- completed: 2026-07-14T23:03 -->
- [x] `skills/cafleet-research`: report/report.md:78, presentation/presentation.md:93 + both roles/director.md:20 inline invocations <!-- completed: 2026-07-14T23:03 -->

### Step 4: CLI implementation

- [x] `cafleet/src/cafleet/cli/fleet.py`: `--coding-agent` → `required=True`, remove `default` / `show_default`, update help text <!-- completed: 2026-07-14T23:11 -->
- [x] `cafleet/src/cafleet/cli/member.py`: `_resolve_coding_agent` drops the `role` parameter and branch; error strings generalized; call site updated; `show_default="inherits the Director's backend"` <!-- completed: 2026-07-14T23:11 -->
- [x] `cafleet/src/cafleet/broker/fleets.py`: delete the docstring note that the `claude` default lives at the Click layer <!-- completed: 2026-07-14T23:11 -->

### Step 5: DB migration

- [x] `db/models.py`: remove `server_default="claude"` from `MemberPlacement.coding_agent` <!-- completed: 2026-07-14T23:10 -->
- [x] Generate migration `0002` via `mise //cafleet:makemigration "drop coding_agent server default on member_placements"`; hand-edit to the batch `alter_column` form (upgrade drops the default, downgrade restores it) <!-- completed: 2026-07-14T23:11 -->
- [x] `cafleet/tests/db/test_alembic_smoke.py`: update the chain guard to the 2-revision chain (`0002`, `down_revision="0001"`, head `0002`), rename the head-version test, and assert `coding_agent` declares no column default in `test_member_placements_table_created_by_migration` <!-- completed: 2026-07-14T23:16 -->

### Step 6: Tests

- [x] `cafleet/tests/cli/test_fleet_bootstrap.py`: replace `test_fleet_create_coding_agent__default_is_claude` with an omitted-flag guard asserting exit 2 and `Missing option '--coding-agent'`; add `--coding-agent claude` to `test_fleet_create_json_output__placement_sub_dict_matches_spec` and the other invocations that omit the flag <!-- completed: 2026-07-14T23:16 -->
- [x] `cafleet/tests/cli/conftest.py`: fleet-create fixtures (lines 36-53) and the flag-omitting invocation `runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])` at lines 56-59 pass `--coding-agent claude` <!-- completed: 2026-07-14T23:16 -->
- [x] `cafleet/tests/cli/test_member.py` `make_bootstrapped_fleet` (lines 743-747): pass `--coding-agent` unconditionally — it currently appends the flag only when the backend is not `claude`, and its claude path would exit 2 under the required flag <!-- completed: 2026-07-14T23:16 -->
- [x] `cafleet/tests/cli/test_member.py`: `test_member_create__role_member_omitted_flag_stays_claude` (line 817) → rename to `test_member_create__role_member_omitted_flag_inherits_director_backend`, assert a codex Director's ordinary member inherits `codex`, and replace its "inheritance is monitor-only" inline comment; monitor fail-loud tests (lines 838, 863) update expected strings to `the member's coding agent`; verify `_invoke_member_create` and the claude-Director-based tests (lines 326, 582) still hold under inheritance <!-- completed: 2026-07-14T23:16 -->
- [x] Sweep the remaining test fixtures for `fleet create` invocations relying on the removed default (e.g. `tests/cli/test_fleet.py`, `test_fleet_flag.py`, WebUI/broker fixtures that shell through the CLI) <!-- completed: 2026-07-14T23:16 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` <!-- completed: 2026-07-14T23:17 -->

### Step 7: Verification

- [x] Grep `cafleet/src/cafleet/cli/` and `cafleet/src/cafleet/db/models.py` for `default="claude"` / `server_default="claude"` — zero coding-agent hits remain (`db/alembic/versions/` is exempt: `0001` is immutable and `0002`'s `downgrade()` restores the literal by design) <!-- completed: 2026-07-14T23:24 -->
- [x] Grep `docs/` and `skills/` for `fleet create` invocations — every one carries `--coding-agent` <!-- completed: 2026-07-14T23:24 -->
- [x] Manual smoke: `cafleet fleet create --name t` exits 2 with the missing-option error; `cafleet fleet create --name t --coding-agent codex` then `cafleet member create` (flag omitted) records `codex` for the member <!-- completed: 2026-07-14T23:24 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-14 | Initial draft |
| 2026-07-14 | Review round 1: full Click missing-option message as the error contract; default-grep criterion scoped to `cli/` + `db/models.py`; SPEC.md §11 chain lead-in and decisions bullet added; conftest.py:56-59 and `make_bootstrapped_fleet` fixtures named; inheritance test rename specified |
| 2026-07-14 | Executed: all 33 tasks and 5 Success Criteria complete; 986 tests, lint, typecheck green; Verifier live smoke passed; Reviewer approved in round 1; PR #198 opened. Status → Complete |
