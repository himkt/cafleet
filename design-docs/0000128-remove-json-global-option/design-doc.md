# Remove `--json` from the Global Option Surface

**Status**: Approved
**Progress**: 26/26 tasks complete
**Last Updated**: 2026-07-11

## Overview

The `cafleet` CLI defines `--json` as a global option on the root click group, placed before the subcommand name. Relocate it to a shared per-subcommand option placed after the subcommand name (like `--full`), removing the global flag entirely — a hard break with no deprecation alias. JSON output itself is unchanged; only the flag's position moves.

## Success Criteria

- [x] Any pre-subcommand use (e.g. `cafleet --json doctor`) fails with Click's standard `No such option: --json` error, exit 2
- [x] Every subcommand in the § *Flag surface* table accepts a trailing `--json` and emits the same JSON it emitted under the global flag (compact single-line, UTF-8, identical shape)
- [x] `--full` / `--quiet` composition is unchanged: truncation is applied before the json-vs-text fork; `--quiet` remains text-only
- [x] No live surface (source, tests, `SPEC.md`, `docs/`, `skills/`, `.claude/`) mentions a global `--json`; `docs/spec/cli-options.md` no longer documents companion `Bash(cafleet --json ...)` permission patterns
- [x] A regression guard asserts the pre-subcommand position no longer parses (exit 2, `No such option`)
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass

---

## Background

`--json` is declared on the root group at `cafleet/src/cafleet/cli/__init__.py:16-18` and stored on the click context (`ctx.obj["json_output"]`). Subcommands read the context value: all six `message` subcommands via the shared `client_command` decorator (`cli/_helpers.py:164`), eight `member` subcommands, `monitor status` / `monitor config`, and `doctor`. Only the `fleet` group additionally defines a local `--json` (`cli/fleet.py:10`, dest `as_json`), OR-ed with the global value through `_wants_json` (`cli/fleet.py:13-14`).

The motivation for the removal is `permissions.allow` prefix matching: Claude Code allow patterns match Bash invocations as literal prefixes (`Bash(cafleet <grp> <cmd> --fleet-id *)`), and a pre-subcommand `--json` breaks the prefix, forcing a companion `Bash(cafleet --json <grp> <cmd> --fleet-id *)` pattern per subcommand (`docs/spec/cli-options.md:125-132`). With `--json` trailing, the existing per-subcommand patterns cover JSON invocations and the companion patterns disappear.

---

## Specification

### Decision: relocate, not remove

JSON output survives — the skill workflows (`fleet create` / `member create` id capture, `message poll` inbox parsing) depend on it. The change is strictly positional: the flag moves from the root group to each JSON-capable subcommand. After the change, `--version` is the only global option.

### Shared flag definition

A shared `json_flag` joins `full_flag` / `quiet_flag` in `cafleet/src/cafleet/cli/_helpers.py`:

```python
json_flag = click.option(
    "--json",
    "json_output",
    is_flag=True,
    default=False,
    help="Output in JSON format.",
)
```

The dest stays `json_output` so subcommand bodies keep their existing variable name; the value arrives as a function parameter instead of `ctx.obj["json_output"]`.

### Flag surface

| Group | Subcommands gaining the shared `json_flag` | Current mechanism | Change |
|---|---|---|---|
| `message` | `send`, `broadcast`, `poll`, `ack`, `cancel`, `show` | global, read inside `client_command` (`_helpers.py:164`) | apply `json_flag` to each subcommand; each callback gains a `json_output: bool` parameter (mirroring `full`); `client_command`'s wrapper branches on `kwargs["json_output"]` instead of `ctx.obj["json_output"]` |
| `member` | `create`, `delete`, `show`, `list`, `capture`, `exec`, `ping`, `nudge` | global, `ctx.obj["json_output"]` at each branch site | apply `json_flag`; branch on the parameter (`member delete` passes it through to `_emit_member_delete_output`) |
| `monitor` | `status`, `config` | global | apply `json_flag`; branch on the parameter |
| (root) | `doctor` | global | apply `json_flag`; branch on the parameter |
| `fleet` | `create`, `list`, `show` | local `_json_flag` (dest `as_json`) OR global via `_wants_json` | replace `_json_flag` with the shared `json_flag` (dest `json_output`); delete `_wants_json`; branch on the parameter directly |

Subcommands with no JSON branch today stay JSON-less: `fleet delete`, `monitor start`, `server`, `setup` (and its `db` / `skill` subcommands).

### Root group after the change

```python
@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
def cli() -> None:
    """CAFleet — CLI for the message broker and member registry."""
```

The `--json` option, the `json_output` parameter, `@click.pass_context`, and the `ctx.obj["json_output"]` assignment are removed. `ctx.ensure_object(dict)` in the root callback is redundant — `_fleet_id_callback` (`_helpers.py:112`) already calls it before storing `fleet_id`.

### Behavioral invariants

- JSON encoding is unchanged: compact single-line via `output.format_json`, non-ASCII emitted as UTF-8.
- `--json` / `--full` remain independent and composable; truncation (`truncate_task_text` / `render_tasks_in_result`) is applied to the result before the encoding fork, exactly as today.
- `--quiet` remains a text-only shortcut, ignored in the JSON branch.
- Per-subcommand JSON shapes are untouched (e.g. `fleet show --json` always includes `deleted_at`; `member show --json` is the unprojected broker dict regardless of `--full`).

### Error contract

Pre-subcommand `--json` becomes an unknown option on the root group: Click's standard `No such option: --json` `UsageError`, exit 2. Hard break — no alias, no transition period, and (per the repo removal rule) no deprecation notice on any live surface. Historical `design-docs/**` content is untouched.

### Canonical position and `permissions.allow`

Every documented example writes `--json` **trailing**, after all other flags:

```bash
cafleet message poll --fleet-id 2 --member-id 7 --json
cafleet message poll --fleet-id 2 --member-id 7 --full --json
```

With the trailing position, the existing one-pattern-per-subcommand allow set (`Bash(cafleet <grp> <cmd> --fleet-id *)`) covers JSON invocations. The companion-pattern requirement and its example are deleted from `docs/spec/cli-options.md` § *permissions.allow coverage*. Operator-side migration (recorded here because live docs must not mention the removed global flag): any existing companion patterns `Bash(cafleet --json <grp> <cmd> --fleet-id *)` in user-level `~/.claude/settings.json` become dead after this change and can be deleted.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation lands first, per `.claude/rules/documentation-maintenance.md`. `README.md`, `docs/concepts/`, `docs/how-to/`, and `.claude/rules/` contain no `--json` mentions and need no edits.

### Step 1: Spec docs (`docs/spec/`)

- [x] `cli-options.md`: change the "JSON output" row of the resolution table (line 61) to a per-subcommand `--json` option (trailing canonical position) <!-- completed: 2026-07-11T08:45 -->
- [x] `cli-options.md`: remove `--json` from the Global Options table, leaving `--version` as the only global option <!-- completed: 2026-07-11T08:45 -->
- [x] `cli-options.md`: add a `--json` section documenting the shared per-subcommand flag, its surface (the § *Flag surface* table), and the trailing canonical position <!-- completed: 2026-07-11T08:45 -->
- [x] `cli-options.md`: delete the companion-pattern bullet and the `Bash(cafleet --json message poll --fleet-id *)` example from § *permissions.allow coverage* <!-- completed: 2026-07-11T08:45 -->
- [x] `cli-options.md`: reword the `doctor` flag-table note "Top-level `--json`, written ahead of the subcommand name" (line 316) to the trailing per-subcommand flag — the fleet flag tables (lines 251-288) and the truncation section (lines 143-146) are position-neutral and stay as-is <!-- completed: 2026-07-11T08:45 -->
- [x] `message-envelope.md`: move `--json` to trailing position in both examples (lines 47, 53) and retarget the global-options anchor link (line 93) to the new `--json` section <!-- completed: 2026-07-11T08:45 -->

### Step 2: `SPEC.md`

- [x] Replace the global `--json` context-object definition (lines 840-841) and the emit branch (line 932) with the shared per-subcommand `json_flag` specification <!-- completed: 2026-07-11T08:52 -->
- [x] Update the per-command surfaces: `doctor` (line 939), the local-OR-global rule (lines 991-992), `fleet create/list/show` (lines 1000-1008), and the message-vs-fleet dual-path paragraphs (lines 1463-1467, 2691) — the dual path collapses to one uniform per-subcommand flag <!-- completed: 2026-07-11T08:52 -->
- [x] Update the CLI checklist entries (lines 2802-2832) to trailing per-subcommand `--json` and delete the "dual path retained" note (line 2916) <!-- completed: 2026-07-11T08:52 -->

### Step 3: Skills

- [x] `skills/cafleet/reference/cli.md`: rewrite the "`--json` and `--version` are top-level options" rule (`--version` stays top-level; `--json` becomes per-subcommand, trailing) and update every example (lines 14-15, 57, 86, 90, 98) <!-- completed: 2026-07-11T09:02 -->
- [x] `skills/cafleet/reference/output-flags.md`: retitle "`--json` (global, machine-parseable)" to per-subcommand and move the flag to trailing position in both examples <!-- completed: 2026-07-11T09:02 -->
- [x] `skills/cafleet-design-doc/`: update `cafleet --json fleet create` / `--json member create` / `--json message poll` invocations to trailing position in `create/create.md`, `execute/execute.md`, `interview/interview.md`, and the create/execute `roles/director.md` files <!-- completed: 2026-07-11T09:02 -->
- [x] `skills/cafleet-research/`: same sweep in `report/report.md`, `presentation/presentation.md`, and both `roles/director.md` files <!-- completed: 2026-07-11T09:02 -->
- [x] `.claude/skills/skill-author/SKILL.md`: same sweep (lines 82, 113, 329, 360, 372, 412) <!-- completed: 2026-07-11T09:00 -->

### Step 4: Code (`cafleet/src/cafleet/cli/`)

- [x] `_helpers.py`: add `json_flag` beside `full_flag` / `quiet_flag` <!-- completed: 2026-07-11T09:20 -->
- [x] `_helpers.py`: `client_command` branches on `kwargs["json_output"]` instead of `ctx.obj["json_output"]` <!-- completed: 2026-07-11T09:20 -->
- [x] `__init__.py`: remove the global `--json` option, the `json_output` parameter, `@click.pass_context`, and the `ctx.obj` assignment from the root group callback <!-- completed: 2026-07-11T09:20 -->
- [x] `message.py`: apply `json_flag` to all six subcommands; add the `json_output: bool` parameter to each callback <!-- completed: 2026-07-11T09:20 -->
- [x] `fleet.py`: replace `_json_flag` and `_wants_json` with the shared `json_flag`; rename the `as_json` parameters to `json_output` <!-- completed: 2026-07-11T09:20 -->
- [x] `member.py`: apply `json_flag` to `create`, `delete`, `show`, `list`, `capture`, `exec`, `ping`, `nudge`; branch on the parameter (thread it through `_emit_member_delete_output`) <!-- completed: 2026-07-11T09:20 -->
- [x] `monitor.py`: apply `json_flag` to `status` and `config`; branch on the parameter <!-- completed: 2026-07-11T09:20 -->
- [x] `doctor.py`: apply `json_flag`; branch on the parameter <!-- completed: 2026-07-11T09:20 -->

### Step 5: Tests and verification

- [x] Migrate every global-position invocation (`["--json", <grp>, ...]`) to the trailing per-subcommand position across `tests/cli/` (touched suites: `test_monitor.py`, `test_doctor.py`, `test_message_truncation.py`, `test_broadcast_json_to_member_id.py`, `test_member_exec.py`, `test_member_ping.py`, `test_message.py`, `test_member.py`, `test_member_delete.py`, `test_member_list_activity.py`, `test_member_capture_defaults.py`, `test_member_show.py`, `test_member_list_all.py` — the last two hoist `--json` inside their invocation helpers `_show` / `_list`, so the fix lands in the helper, not the per-test call sites; the fleet suites and `conftest.py` already use the trailing local flag and stay untouched) <!-- completed: 2026-07-11T08:47 -->
- [x] `tests/cli/test_client_command.py`: replace the harness's re-declared global `--json` (lines 26-30) with the shared per-command `json_flag` <!-- completed: 2026-07-11T08:47 -->
- [x] Add a regression guard asserting the pre-subcommand position no longer parses: `cafleet --json doctor` exits 2 with `No such option: --json` <!-- completed: 2026-07-11T08:47 -->
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:format` pass <!-- completed: 2026-07-11T09:10 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-11 | Initial draft |
| 2026-07-11 | Reviewer round 1: operator-side companion-pattern cleanup note; corrected `cli-options.md` sweep task (fleet tables and truncation section are position-neutral); added `test_member_show.py` / `test_member_list_all.py` helper-level migration |
