# Refactor Test Suite to Functional Pytest Style

**Status**: Approved
**Progress**: 37/44 tasks complete
**Last Updated**: 2026-05-01

## Overview

The cafleet test suite currently uses pytest class-style flavor (`class Test...:` containers grouping `test_*` methods). This design refactors every class-style file under `cafleet/tests/` to plain top-level `test_*` functions. The change is mechanical — same test count, same assertions, same fixtures, same coverage — and produces a flatter, more idiomatic pytest layout.

## Success Criteria

- [ ] `git grep "^class Test"` inside `cafleet/tests/` returns no matches
- [ ] `pytest --collect-only -q` reports the **same number of tests** as the pre-refactor baseline
- [ ] `mise //cafleet:test` passes
- [ ] `mise //cafleet:lint` passes
- [ ] `mise //cafleet:format` reports no changes
- [ ] `mise //cafleet:typecheck` passes
- [ ] No new tests are added; the existing suite is the oracle

---

## Background

The CAFleet test suite has 32 `test_*.py` files under `cafleet/tests/` (plus `conftest.py` and `__init__.py`, neither in scope for renaming). 28 of those 32 files currently use class-style organization with about 120 `class Test*` containers; the remaining 4 (`test_alembic_smoke.py`, `test_cli_version.py`, `test_db_init.py`, `test_db_pragmas.py`) are already function-style and stay untouched. The class-style containers are **purely organizational** — none of them use class-scoped fixtures, none inherit from a base class, none define `setup_method` / `teardown_method` / `setup_class` / `teardown_class`, and none share state across methods via `self` (with one exception, two files use class-level constants accessed via `self`, see Specification §C). All fixtures are defined at module level or in `conftest.py`.

The `[tool.pytest.ini_options]` table in `cafleet/pyproject.toml` is empty, so pytest's default collection rules apply (`Test*` classes and `test_*` functions). Removing the `Test*` containers does not alter collection because top-level `test_*` functions are also collected by default.

The §A recipe assumes there are no class-scoped marks (`@pytest.mark.parametrize` or other `@pytest.mark.*`) sitting directly on the class declarations. This is verifiable today via `git grep -nE "^@pytest\.mark\." cafleet/tests/`, which returns zero matches. Existing `@pytest.mark.parametrize` decorators all sit on individual methods inside classes (see §A step 5).

Functional style is the more common modern convention in pytest projects: less ceremony, fewer indentation levels, no `self` argument noise, and no organizational hierarchy that pytest does not actually use beyond test ID generation.

---

## Specification

### A. Conversion recipe (per class)

For each `class TestSomeThing:` in a test file:

1. Convert the class name `TestSomeThing` to a snake_case prefix: `some_thing` (drop the leading `Test`, then PEP 8 lowercase with underscore boundaries between camel-case words).
2. For every method `def test_method_name(self, ...args):` inside the class, emit a top-level function `def test_some_thing__method_name(...args):` — note the **double underscore** between class context and original method name.
3. Drop the `self` parameter from every method's signature.
4. Replace any `self.X` reference in the method body with the module-level binding `X` (see §C for class-attribute promotion rules).
5. Move `@pytest.mark.parametrize(...)` decorators that sit on a method directly onto the resulting top-level function. No semantics change because the suite has no class-scoped parametrization (no `pytest.mark.parametrize` decorators on classes themselves).
6. Preserve the original method order inside each file. Class boundaries become blank lines between function groups; an inline comment `# --- some_thing ---` MAY be added for readability when a file contains many groups, but is not required.
7. Delete the now-empty `class TestSomeThing:` line.

The class docstring (if any) MUST be either dropped (if it merely restates the class name) or relocated as a module-level section comment above the resulting function group.

### B. Naming convention — examples

| Class definition | Method | Resulting top-level function |
|---|---|---|
| `class TestSendMessage:` | `def test_persists_row(self): ...` | `def test_send_message__persists_row(): ...` |
| `class TestBroadcastAdministratorExclusion:` | `def test_skips_admin(self, runner): ...` | `def test_broadcast_administrator_exclusion__skips_admin(runner): ...` |
| `class TestMigration0002Upgrade:` | `def test_inserts_session_row(self): ...` | `def test_migration_0002_upgrade__inserts_session_row(): ...` |
| `class TestDoctorJsonOutput:` | `def test_emits_pane_id(self, monkeypatch): ...` | `def test_doctor_json_output__emits_pane_id(monkeypatch): ...` |

The double underscore separator (`__`) is used uniformly so that the `pytest -k` selector and pytest test ID stay self-describing.

### C. Class attributes / helper classes

Two files use class-level constants accessed via `self`. Both promote to module-level constants with the **same name**:

| File | Current | Converted |
|---|---|---|
| `test_cli_claude_helpers.py` | `class TestClaudePromptTemplate: _STANDARD_KWARGS = {...}` accessed via `self._STANDARD_KWARGS` | Top-level `_STANDARD_KWARGS = {...}` placed immediately above the function group, accessed bare |
| `test_server_cli.py` | `class TestWebUIDistWarning: _WARNING_PREFIX = "..."` accessed via `self._WARNING_PREFIX` | Top-level `_WARNING_PREFIX = "..."` placed immediately above the function group, accessed bare |

Helper classes that are NOT pytest test classes (no `Test` prefix) MUST be left untouched. Specifically `class _FakeClock:` in `cafleet/tests/test_tmux.py` is a stateful test helper; preserve its definition and all `self.now` / `self.sleep_calls` usage exactly as-is.

### D. What is NOT changed

- Module-level fixtures (every fixture in this suite already lives at module level or in `conftest.py`).
- The single autouse fixture `_silence_real_tmux_subprocess` in `cafleet/tests/conftest.py`.
- Test logic: no assertion text, no monkeypatch target, no parametrize value, no fixture argument is altered.
- Test file count, test file names, and the `cafleet/tests/` directory layout — no files are split, renamed, or merged. Mechanical conversion is the default for every file. Splitting an oversize file is **out of scope** for this refactor; a separate cleanup pass can address it later if desired.
- Pytest collection IDs change shape (from `tests/test_x.py::TestFoo::test_bar` to `tests/test_x.py::test_foo__test_bar`), but the suite has no consumers pinning on those IDs and no migration note is required.

### E. Files in scope

28 files under `cafleet/tests/`, listed in the implementation step that owns them. Already-functional files (`test_alembic_smoke.py`, `test_cli_version.py`, `test_db_init.py`, `test_db_pragmas.py`) are out of scope.

### F. Worked example

Before (`cafleet/tests/test_cli_doctor.py`, abbreviated):

```python
class TestDoctorTextOutput:
    def test_emits_pane_block(self, runner_with_tmux_env):
        result = runner_with_tmux_env.invoke(cli, ["doctor"])
        assert result.exit_code == 0
        assert "tmux:" in result.output


class TestDoctorJsonOutput:
    def test_emits_pane_id(self, runner_with_tmux_env):
        result = runner_with_tmux_env.invoke(cli, ["--json", "doctor"])
        assert result.exit_code == 0
```

After:

```python
def test_doctor_text_output__emits_pane_block(runner_with_tmux_env):
    result = runner_with_tmux_env.invoke(cli, ["doctor"])
    assert result.exit_code == 0
    assert "tmux:" in result.output


def test_doctor_json_output__emits_pane_id(runner_with_tmux_env):
    result = runner_with_tmux_env.invoke(cli, ["--json", "doctor"])
    assert result.exit_code == 0
```

### G. Opt-in cleanup evaluation

The user authorization permits opportunistic cleanup beyond the mechanical conversion: splitting overgrown test files into per-concern files, and merging near-duplicate test groups into `@pytest.mark.parametrize`. To keep this refactor reviewable, both forms of cleanup require a **named criterion**, evaluated up-front against every in-scope file, before any conversion work begins.

#### G.1 File-split criterion

A class-style file is a split candidate iff **all three** conditions hold:

1. The file exceeds 800 source lines.
2. It contains 8 or more `class Test*` containers.
3. The classes group into 3 or more orthogonal subjects, each with a coherent name independent of the others, such that splitting yields per-subject files that each tell a self-contained story.

Failing any one disqualifies the file. The 800-line threshold is deliberately conservative: a 600-line file that happens to have 9 classes is still cohesive enough to read end-to-end, and splitting trades the navigation cost of "find the class" for the navigation cost of "find the file."

#### G.2 Parametrize-merge criterion

A group of adjacent test functions is a merge candidate iff **all three** conditions hold:

1. There are 3 or more such functions.
2. Their bodies differ only in input value(s) and expected output(s) — same setup, same assertion shape, same monkeypatch targets.
3. The individual function names are not load-bearing prose (i.e., they do not document a distinct intent that would be lost as a parametrize id).

#### G.3 Evaluation result

The Drafter inventoried every in-scope file under §E. **No file meets the §G.1 split criterion** and **no test group meets the §G.2 parametrize-merge criterion** under conservative reading. The largest files by class count are `test_tmux.py` (12 classes) and `test_broker_registry.py` (11 classes); both stay below the 800-line threshold and their class groupings already align with single subjects (tmux primitive operations; registry CRUD operations) that read coherently as one file each. Existing `@pytest.mark.parametrize` decorators already cover the obvious table-driven groups (digit ranges, newline variants, etc.).

Therefore this design proceeds with **mechanical conversion only**. A separate cleanup pass remains the right vehicle if the suite later grows past the §G.1 / §G.2 thresholds.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Establish baseline

- [x] Run `mise //cafleet:test` from the project root and confirm a green baseline. <!-- completed: 2026-05-01T12:00 -->
- [x] Capture the pre-refactor test count via `mise //cafleet:test -- --collect-only -q` (mise forwards arguments after `--` to the underlying `uv run python -m pytest`) and record the count in this design doc immediately below this checklist as `Baseline test count: N`. <!-- completed: 2026-05-01T12:00 -->
- [x] Confirm the §A step 5 invariant by running `git grep -nE "^@pytest\.mark\." cafleet/tests/` and verifying the result is empty (no class-scoped marks). <!-- completed: 2026-05-01T12:00 -->

Baseline test count: 557

### Step 2: Refactor broker-layer tests

Apply the §A recipe to:

- [x] `cafleet/tests/test_broker_administrator.py` — 5 classes <!-- completed: 2026-05-01T12:30 -->
- [x] `cafleet/tests/test_broker_messaging.py` — 7 classes <!-- completed: 2026-05-01T12:30 -->
- [x] `cafleet/tests/test_broker_registry.py` — 11 classes <!-- completed: 2026-05-01T12:30 -->
- [x] `cafleet/tests/test_broker_webui.py` — 8 classes <!-- completed: 2026-05-01T12:30 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T12:30 -->

### Step 3: Refactor CLI agent / message / generic tests

- [x] `cafleet/tests/test_cli_agent.py` — 1 class <!-- completed: 2026-05-01T13:00 -->
- [x] `cafleet/tests/test_cli_claude_helpers.py` — 3 classes; promote `_STANDARD_KWARGS` to module-level (§C) <!-- completed: 2026-05-01T13:00 -->
- [x] `cafleet/tests/test_cli_client_command.py` — 4 classes <!-- completed: 2026-05-01T13:00 -->
- [x] `cafleet/tests/test_cli_doctor.py` — 4 classes <!-- completed: 2026-05-01T13:00 -->
- [x] `cafleet/tests/test_cli_message.py` — 4 classes <!-- completed: 2026-05-01T13:00 -->
- [x] `cafleet/tests/test_cli_message_truncation.py` — 4 classes <!-- completed: 2026-05-01T13:00 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T13:00 -->

### Step 4: Refactor CLI member-family tests

- [x] `cafleet/tests/test_cli_member.py` — 7 classes <!-- completed: 2026-05-01T13:30 -->
- [x] `cafleet/tests/test_cli_member_delete.py` — 8 classes <!-- completed: 2026-05-01T13:30 -->
- [x] `cafleet/tests/test_cli_member_exec.py` — 4 classes <!-- completed: 2026-05-01T13:30 -->
- [x] `cafleet/tests/test_cli_member_ping.py` — 5 classes <!-- completed: 2026-05-01T13:30 -->
- [x] `cafleet/tests/test_cli_member_send_input.py` — 8 classes <!-- completed: 2026-05-01T13:30 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T13:30 -->

### Step 5: Refactor session + bootstrap tests

- [x] `cafleet/tests/test_cli_session_bootstrap.py` — 5 classes <!-- completed: 2026-05-01T14:00 -->
- [x] `cafleet/tests/test_cli_session_flag.py` — 5 classes <!-- completed: 2026-05-01T14:00 -->
- [x] `cafleet/tests/test_session_bootstrap.py` — 7 classes <!-- completed: 2026-05-01T14:00 -->
- [x] `cafleet/tests/test_session_cli.py` — 6 classes <!-- completed: 2026-05-01T14:00 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T14:00 -->

### Step 6: Refactor alembic migration tests

- [x] `cafleet/tests/test_alembic_0002_upgrade.py` — 2 classes <!-- completed: 2026-05-01T14:30 -->
- [x] `cafleet/tests/test_alembic_0006_upgrade.py` — 3 classes <!-- completed: 2026-05-01T14:30 -->
- [x] `cafleet/tests/test_alembic_0008_upgrade.py` — 3 classes <!-- completed: 2026-05-01T14:30 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T14:30 -->

### Step 7: Refactor remaining tests

- [x] `cafleet/tests/test_output.py` — 4 classes <!-- completed: 2026-05-01T15:00 -->
- [x] `cafleet/tests/test_output_indexed_list.py` — 1 class <!-- completed: 2026-05-01T15:00 -->
- [x] `cafleet/tests/test_server_cli.py` — 5 classes; promote `_WARNING_PREFIX` to module-level (§C) <!-- completed: 2026-05-01T15:00 -->
- [x] `cafleet/tests/test_tmux.py` — 12 `Test*` classes; **preserve `class _FakeClock`** untouched (§C) <!-- completed: 2026-05-01T15:00 -->
- [x] `cafleet/tests/test_tmux_send_helpers.py` — 1 class <!-- completed: 2026-05-01T15:00 -->
- [x] `cafleet/tests/test_webui_api_format.py` — 7 classes <!-- completed: 2026-05-01T15:00 -->
- [x] After this step, run `mise //cafleet:test` and confirm green. <!-- completed: 2026-05-01T15:00 -->

### Step 8: Final verification

- [ ] `git grep "^class Test" cafleet/tests/` returns no matches. <!-- completed: -->
- [ ] `class _FakeClock` still present in `cafleet/tests/test_tmux.py` (helper class is preserved). <!-- completed: -->
- [ ] `mise //cafleet:test -- --collect-only -q` count matches the Step 1 baseline test count exactly. <!-- completed: -->
- [ ] `mise //cafleet:test` passes. <!-- completed: -->
- [ ] `mise //cafleet:lint` passes. <!-- completed: -->
- [ ] `mise //cafleet:format` reports no changes (run as a check; no files should be modified). <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-01 | Initial draft |
| 2026-05-01 | Reviewer round 1: corrected file count to 32 (28 class-style + 4 already-functional); added §G opt-in cleanup criteria with explicit "no candidates" evaluation; fixed progress denominator to match checkbox count; replaced bypass `uv run pytest --collect-only` with mise-routed `mise //cafleet:test -- --collect-only -q`; added §A step 5 verification grep into Step 1 |
| 2026-05-01 | Approved by user — Status flipped Draft → Approved |
