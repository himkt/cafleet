# Test Suite Cleanup — Reduce Excessive Tests in cafleet/tests/

**Status**: Approved
**Progress**: 0/33 tasks complete
**Last Updated**: 2026-05-16

## Overview

The cafleet test suite has grown to **841 test functions across 57 Python files** in `cafleet/tests/`, dominated by per-key dict-field assertions, per-revision Alembic regressions for one-way migrations that already shipped, and overlapping CLI files. Apply five principles to remove or consolidate ~460 tests across six sequenced commits, keeping `mise //cafleet:test` green at every step.

## Success Criteria

- [ ] Test function count reduced from **841 → ≤ 400** (≥ 52% reduction, satisfying the AGGRESSIVE ≥ 50% target). Stretch goal: ≤ 340 (≥ 60%) if Step 5 incidental dedup achieves ≥ 30% trim of the untouched-CLI carve-out.
- [ ] `mise //cafleet:test` exits 0 after every commit in the six-step sequence.
- [ ] No regression in behavioural coverage of the public CLI surface (`cafleet message *`, `cafleet agent *`, `cafleet session *`, `cafleet member *`, `cafleet db init`, `cafleet base-dir *`, `cafleet doctor`).
- [ ] The five principles in § *Principles* are documented in this doc and applied uniformly; the lint pass surfaces no new tests violating principle (ii) or (iii) on review.
- [ ] Files in § *Off-Limits* are unchanged.
- [ ] `cafleet/tests/conftest.py` and the four helper modules (`_helpers.py`, `_broker_helpers.py`, `_member_cli_helpers.py`, `__init__.py`) are unchanged in this design — no symbols added, no symbols removed.

---

## Background

`cafleet/tests/` currently contains:

| Group | Files | Test functions |
|:--|---:|---:|
| Alembic per-revision regressions (`test_alembic_0002`, `_0006`, `_0008`, `_typed_columns_upgrade`) | 4 | 31 |
| Alembic smoke (`test_alembic_smoke.py`) | 1 | 4 |
| `db` (init state-machine + PRAGMA listener) | 2 | 6 |
| Broker / DB logic (`test_broker_*`, `test_session_*`) | 9 | 309 |
| CLI commands (`test_cli_*`) | 19 | 234 |
| Tmux helpers (`test_tmux*`) | 4 | 64 |
| Output formatting (`test_output*`) | 7 | 133 |
| Base-dir resolver (`test_base_dir*`) | 2 | 27 |
| Server / WebUI (`test_server_cli.py`, `test_webui_api_format.py`) | 2 | 21 |
| Token-budget (`tests/token_budget/*`) | 3 | 12 |
| **Total** | **53** test modules (+ 4 helpers) | **~841** |

Three recurring smells dominate the bloat:

1. **Per-revision Alembic regression files.** 31 tests across 4 files re-verify schema diffs of migrations that already deployed and are one-way (`test_migration_0002_downgrade__downgrade_raises_not_implemented` codifies the one-way property). The legacy schemas they exercise (`api_keys` table, `task_json` blob column) no longer exist in any deployed database.
2. **Per-key dict-projection assertion chains.** A single function's return is checked by 5–15 separate tests, one per key. Examples: `test_send_message__returns_dict_with_task_key`, `__task_id_is_valid_uuid`, `__context_id_is_recipient`, `__status_state_is_input_required`, `__status_has_timestamp`, `__from_agent_id`, `__to_agent_id`, `__type_is_unicast`, `__text_carries_body` — nine tests for one `broker.send_message(...)` call. The same pattern repeats in `test_broker_registry.py` (`test_create_session__*` × 6, `test_register_agent__*` × 5), `test_broker_typed_columns.py` (`test_unicast_task_dict__*` × 12, `test_broadcast_message__summary_*` × 7), and `test_output_render_task.py` (`test_render_task__compact_*` × 18).
3. **CLI surface overlap.** `test_cli_member.py` (16) and `test_cli_member_create_prompt_file.py` (13) both exercise `_resolve_prompt(...)` placeholder substitution against the same fixtures. `test_cli_member_capture_defaults.py` (11) and `test_tmux_capture_defaults.py` (3) both pin the same default-lines value.

The result is a test suite that is slow to read, hard to maintain, and gives the false impression of coverage depth — many tests differ only in which dict key they assert.

---

## Specification

### Principles

Adopt these five principles verbatim. Every removal and consolidation in § *Implementation* cites the principle it applies.

> **(i) One-way Alembic migrations that already shipped to production are historical, not live behaviour — covered by `alembic_smoke 'upgrade head'` working.** Once a migration is deployed and the legacy schema no longer exists in any environment, the per-revision regression test is exercising a library (Alembic) on a frozen artifact. The single smoke test that runs `command.upgrade(cfg, "head")` and inspects the resulting schema is sufficient.
>
> **(ii) Per-key dict-projection assertions collapse to one parametrized "shape" test.** When N tests each check a different key of the same returned dict, the right shape is one test that asserts the full structure (e.g., via `assert set(result.keys()) == EXPECTED_KEYS` plus a representative-value spot-check), or one parametrized test over the (key, expected) pairs.
>
> **(iii) Tests whose name is `test_<X>__has_<field>` verifying one key existence are a fragmentation smell.** The `__` separator pattern across the suite (`test_unicast_task_dict__from_agent_id_top_level`, `test_render_task__compact_drops_to_agent_id`) reliably marks "one assertion per test, all on the same call result." This is a code-smell signal, not a coverage signal.
>
> **(iv) Tests for PRAGMA / library boilerplate (SQLAlchemy events, Click validators, pydantic validators) are out-of-scope.** When the test asserts that a library hook fired correctly (e.g., `PRAGMA foreign_keys == 1`), the bug it would catch is "our event listener stopped being imported" — which is caught by any other test that uses the broker. Click and pydantic validators are exercised implicitly by every CLI/HTTP smoke test that hits the validation path.
>
> **(v) Member/Tmux tests with `@monkeypatch` mocking `tmux._run` are testing argument construction — consolidate via `pytest.mark.parametrize`.** When the test patches `tmux._run` to a no-op and asserts on the recorded argv, the test is checking string-formatting, not behaviour. One parametrized test per public tmux helper (`split_window`, `send_keys`, `capture_pane`, `display_message`) replaces the per-flag fragmented set.

### Default Policy

| Situation | Action |
|:--|:--|
| The behaviour is exercised by **another file** (smoke test, CLI integration test, or higher-level scenario) | **Delete.** Do not consolidate. |
| The behaviour is the **only** locus of coverage for that public API | **Consolidate** via `pytest.mark.parametrize` or a single richer-assertion test. |
| The behaviour is **library boilerplate** (principle iv) | **Delete.** |
| The behaviour is a **happy-path constructor / `__init__` / `__repr__`** | **Delete.** Dataclass / pydantic / Click already provide it. |

### Off-Limits

These files are NOT touched by this design. Justification follows each.

| Path | Tests | Why preserved |
|:--|---:|:--|
| `cafleet/tests/token_budget/test_envelope_size.py` | 4 | Load-bearing budget assertion — protects against silent envelope inflation. |
| `cafleet/tests/token_budget/test_skill_size.py` | 4 | Skill-size budget — same rationale. |
| `cafleet/tests/token_budget/test_spawn_prompt_size.py` | 4 | Spawn-prompt size budget — same rationale. |
| `cafleet/tests/test_alembic_smoke.py` | 4 | Sole live coverage of the migration-vs-model drift gap (per principle i). |
| `cafleet/tests/test_db_init.py` | 4 | State-machine tests for `cafleet db init` — distinct fresh / idempotent / legacy / ahead branches, all are live cafleet code, not Alembic itself. |
| `cafleet/tests/test_base_dir*.py` | 27 | Behaviour-level tests of the `cafleet base-dir resolve` CLI / Python API. Each test exercises a distinct resolution scenario (repo-root, subfolder, abs-outside-repo, unset-sentinel, anchor-file fallback, gitignored anchor, symlinked checkout). No `__has_<field>` fragmentation, no per-key projection chains — each test is a behaviour-level scenario. Preserved as a coherent live-behaviour suite. |
| `cafleet/tests/conftest.py` | (fixtures) | Out of scope per Q6. |
| `cafleet/tests/_helpers.py`, `_broker_helpers.py`, `_member_cli_helpers.py`, `__init__.py` | (helpers) | Out of scope per Q6. |

### Greenness Invariant

Every commit in the six-step sequence MUST leave `mise //cafleet:test` exiting 0. The sequence is ordered so that:

- Step 1 deletes whole files (cannot break neighbouring files).
- Step 2 deletes a 2-test file (cannot break neighbours).
- Step 3 rewrites two large fragmented files in place — each rewrite preserves the public-API contract being tested, only collapses the per-key chain.
- Steps 4–6 each touch one logical group at a time; each step ends with the suite green before the next step starts.

If a step exposes a real regression in source (not in tests) — `STOP`, raise a `COMMENT(programmer)` marker at the affected step, and do not advance the sequence. Test-suite cleanup does not modify source code under `cafleet/src/cafleet/`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Delete per-revision Alembic regression files

**Principle**: (i). All four migrations are one-way and already shipped; `test_alembic_smoke.py` (kept) covers `upgrade head` end-to-end against a tempfile DB. The legacy schemas these files reconstruct (`api_keys` table, `task_json` column) no longer exist in any deployed environment.

**Files to delete**:

| File | Tests | Notes |
|:--|---:|:--|
| `cafleet/tests/test_alembic_0002_upgrade.py` | 11 | Covers `api_keys` → `sessions` rename + index changes. `api_keys` is gone. |
| `cafleet/tests/test_alembic_0006_upgrade.py` | 6 | Administrator seed migration; same behaviour is asserted live in `test_broker_registry.py::test_create_session_administrator_seed__*`. |
| `cafleet/tests/test_alembic_0008_upgrade.py` | 5 | Column capitalisation migration; the schema invariant survives in `test_alembic_smoke.py`. |
| `cafleet/tests/test_alembic_typed_columns_upgrade.py` | 9 | Surface-14 `task_json → text` migration + pre-flight check. The `task_json` column no longer exists; pre-flight code is dead in any post-Surface-14 DB. The live invariant ("`text` column exists, `task_json` does not") is in `test_alembic_smoke.py::test_alembic_upgrade_head_creates_expected_tables`. |

**Total deleted**: 31 tests.

**Helper-file note**: After the deletions, some symbols inside `cafleet/tests/_helpers.py` (e.g. `_make_alembic_cfg`, `_now_iso`) may become unreferenced. Per Success Criteria #6, this design does NOT touch `_helpers.py` — leave the unused symbols in place. A follow-up sweep can prune them in a separate design.

#### Step 1 tasks

- [ ] Delete `cafleet/tests/test_alembic_0002_upgrade.py` <!-- completed: -->
- [ ] Delete `cafleet/tests/test_alembic_0006_upgrade.py` <!-- completed: -->
- [ ] Delete `cafleet/tests/test_alembic_0008_upgrade.py` <!-- completed: -->
- [ ] Delete `cafleet/tests/test_alembic_typed_columns_upgrade.py` <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `chore: drop per-revision alembic regression tests (design 0000061 step 1)` <!-- completed: -->

### Step 2: Delete PRAGMA / library-boilerplate tests

**Principle**: (iv). `test_db_pragmas.py` contains exactly two tests that connect to a fresh in-memory SQLite engine and assert `PRAGMA foreign_keys == 1` and `PRAGMA busy_timeout == 5000`. These verify that `cafleet.db.engine` registers its SQLAlchemy `@event.listens_for(Engine, "connect")` listener. The bug they would catch ("listener not loaded") is caught implicitly by every test that exercises the broker over SQLite — any FK-violating insert would raise.

**Files to delete**:

| File | Tests |
|:--|---:|
| `cafleet/tests/test_db_pragmas.py` | 2 |

**Total deleted**: 2 tests.

#### Step 2 tasks

- [ ] Delete `cafleet/tests/test_db_pragmas.py` <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `chore: drop PRAGMA boilerplate tests (design 0000061 step 2)` <!-- completed: -->

### Step 3: Consolidate per-key projection fragmentations

**Principle**: (ii), (iii). Two files each fragment one return-shape into 30–50 tests, one assertion per key. Rewrite each as a single parametrized "shape" test plus a small number of behaviour-distinguishing tests.

#### 3.1 `cafleet/tests/test_broker_typed_columns.py` — 53 → ~8

Current shape: 53 tests that each pick one key off the dict returned by `broker.send_message`, `broker.broadcast_message`, `broker.poll_tasks`, `broker.ack_task`, `broker.cancel_task`, `broker.get_task`, `broker.list_inbox`, `broker.list_sent`, `broker.list_timeline`. Example pile-up around `send_message` (file lines 75–212):

```
test_unicast_task_dict__returns_flat_typed_shape
test_unicast_task_dict__no_metadata_wrapping
test_unicast_task_dict__text_field_holds_body
test_unicast_task_dict__from_agent_id_top_level
test_unicast_task_dict__to_agent_id_top_level
test_unicast_task_dict__context_id_is_recipient
test_unicast_task_dict__type_is_unicast
test_unicast_task_dict__task_id_is_valid_uuid
test_unicast_task_dict__status_state_is_input_required
test_unicast_task_dict__status_timestamp_matches_now_arg
test_unicast_task_dict__origin_task_id_default_none
test_unicast_task_dict__origin_task_id_propagates_when_supplied
```

**Target shape — 8 tests**:

| New test | Asserts |
|:--|:--|
| `test_send_message__unicast_returns_flat_typed_envelope` | `set(task.keys()) == {"task_id", "context_id", "from_agent_id", "to_agent_id", "type", "created_at", "status_state", "status_timestamp", "origin_task_id", "text"}` + spot-checks on each value. Subsumes 9 of the 12 unicast tests. |
| `test_send_message__origin_task_id_default_and_propagate` | parametrize `[(None, None), ("origin-tid", "origin-tid")]`. Subsumes the remaining 2. |
| `test_broadcast_message__summary_envelope_shape` | One assertion over the full broadcast-summary dict (subsumes the 7 `test_broadcast_message__summary_*` tests). |
| `test_broadcast_message__delivery_task_shape_and_origin_link` | Combined: delivery tasks are flat-typed and `origin_task_id` links back to the summary (subsumes 2 tests). |
| `test_poll_tasks__returns_flat_typed_task_dicts_with_filters` | Combined: shape + broadcast_summary exclusion + `since` filter (subsumes 5 tests). |
| `test_ack_and_cancel__transition_and_round_trip` | parametrize over `[ack, cancel]` × `[round_trip_text, rejects_unauthorized]` — collapses `test_ack_task__*` and `test_cancel_task__*` (8 tests) into one. |
| `test_list_apis__no_metadata_wrapping_and_filter_broadcast_summary` | parametrize over `[list_inbox, list_sent, list_timeline]` — collapses the 9 `test_list_*` tests into one. |
| `test_task_table_and_model__text_column_present_task_json_absent` | Combined schema + ORM attribute assertion (subsumes 3 tests). |

**Net delete**: 45 tests.

#### 3.2 `cafleet/tests/test_output_render_task.py` — 42 → ~12

Target post-rewrite count is **12 tests**: 9 newly-written rewrites + 3 budget-test KEEPs documented separately.

**Rewritten target shape — 9 new tests** (replacing the 39 fragmented projection / format / render tests):

| New test | Asserts |
|:--|:--|
| `test_render_task__compact_typical_unicast_shape` | `set(rendered.keys()) == {"id", "from", "ts", "text"}` + spot-checks on each value (subsumes `compact_has_required_minimum_keys`, `compact_id_is_first_8_chars`, `compact_from_is_first_8_chars`, `compact_ts_equals_status_timestamp`, `compact_text_equals_body`, `compact_unicast_typical_shape_has_only_4_keys`). |
| `test_render_task__compact_drops_default_state_and_metadata` | parametrize over the dropped-key set `{"to_agent_id", "context_id", "status_state", "created_at", "type"}` + the legacy-camelcase forbidden set. |
| `test_render_task__compact_broadcast_summary_shape_includes_kind_and_origin` | One assertion on `{"id", "from", "ts", "text", "kind", "origin"}`. |
| `test_render_task__compact_kind_and_origin_present_only_when_meaningful` | parametrize `(type_, origin_task_id, expected_keys_added)`. Subsumes the four `compact_omits_kind_*` / `compact_includes_kind_*` / `compact_omits_origin_*` / `compact_includes_origin_*` tests. |
| `test_render_task__full_preserves_long_form_keys` | parametrize over `{"to_agent_id", "context_id", "status_state", "task_id", "text"}`. |
| `test_format_json__compact_no_whitespace_and_round_trips` | Combined: default is compact + `json.loads` round-trips + `pretty=False` matches default. Subsumes 3 tests. |
| `test_format_json__pretty_indented_and_longer` | Combined: indented form + longer than default + `json.loads` round-trips. Subsumes 3 tests. |
| `test_format_task__compact_two_lines_with_expected_fields` | parametrize over `("line1_contains", "needle")` for each id8 / from8 / timestamp expectation. Subsumes 5 `test_format_task__compact_*` tests. |
| `test_format_task__full_legacy_layout_has_more_lines_and_field_labels` | Combined: legacy labels present + more lines than compact. Subsumes 2 tests. |

**Kept as-is — 3 budget tests**:

`test_budget__compact_json_at_most_30_percent_of_pretty_baseline`, `test_budget__no_whitespace_separators`, `test_budget__round_trips` are load-bearing reduction-target assertions named in design 0000049. Do not touch.

**Net delete**: 42 − 12 = 30 tests.

#### Step 3 tasks

- [ ] Rewrite `cafleet/tests/test_broker_typed_columns.py` from 53 to 8 tests per § 3.1 <!-- completed: -->
- [ ] Rewrite `cafleet/tests/test_output_render_task.py` from 42 to 12 tests per § 3.2 (9 new + 3 budget kept) <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `refactor(tests): consolidate per-key projection chains (design 0000061 step 3)` <!-- completed: -->

### Step 4: Parametrize tmux argument-construction tests

**Principle**: (v). All four tmux test files mock `tmux._run` (the autouse `_silence_real_tmux_subprocess` fixture in `conftest.py` plus per-test recorders). What they test is "given args (X, Y, Z), the argv passed to `tmux._run` is `[...]`" — pure string formatting.

#### 4.1 `cafleet/tests/test_tmux.py` — 34 → 8

Current shape: 34 tests covering 10 public tmux helpers. Most are 1–3-line argv assertions.

**Target shape — 8 tests** (the 10 helpers fold into 8 by grouping the two pair-shaped probe-helpers each into one parametrized test):

| New test | Helper(s) covered | Parametrize rows |
|:--|:--|:--|
| `test_tmux_available__detects_tmux_binary_and_session_env` | `tmux_available` | `[(no_tmux_path, False), (path_present_no_TMUX_env, False), (path_and_TMUX_set, True)]` |
| `test_context_probes__current_pane_and_director` | `current_pane_context`, `director_context` | `[("current_pane_context", expected_argv_a), ("director_context", expected_argv_b)]` — both probe `display-message -p` against the live pane. |
| `test_parse_display_message__pane_and_window_extraction` | `parse_display_message` | `[(canonical_two_field, parsed_pair), (single_field, parsed_lone), (empty_string, None)]` |
| `test_split_window__argv_construction` | `split_window` | `[(default_kwargs, [...]), (with_target_pane, [...]), (with_size_percent, [...]), (with_env_passthrough, [...])]` |
| `test_send_keys__argv_construction_and_literal_quoting` | `send_keys` | `[(plain_text, expected), (with_enter_kw, expected_with_C-m), (multibyte_text, expected_utf8), (dollar_sign_escaped, expected_quoted)]` |
| `test_kill_pane__argv_targets_pane_id` | `kill_pane` | `[(canonical_pane_id, ["kill-pane", "-t", "%42"])]` |
| `test_capture_pane__argv_with_lines_and_target` | `capture_pane` | `[(default_30, [...]), (custom_lines, [...]), (target_pane, [...]), (non_positive_lines_rejected, raises)]` |
| `test_display_message_and_is_pane_dead__pair` | `display_message`, `is_pane_dead` | `[("display_message", expected_argv_display), ("is_pane_dead__alive", returns_False), ("is_pane_dead__dead", returns_True)]` — both inspect display-message output. |

#### 4.2 `cafleet/tests/test_tmux_send_helpers.py` — 12 → ~3

Current shape: 12 tests on `send_inline_preview`, `send_poll_trigger`, `send_message_to_pane` covering escaping (`$`, backtick, newline, double-quote, multibyte).

**Target shape**: One parametrized escaping test per helper. ~3 tests.

#### 4.3 `cafleet/tests/test_tmux_send_inline_preview.py` — 15 → ~3

Current shape: 15 tests on inline-preview shape — pane-dead skip, self-send skip, missing-placement skip, header format, body truncation, multibyte body.

**Target shape**: One test for the happy-path 2-line preview shape; one parametrized skip-condition test (pane-dead / self / no placement / no `tmux` on PATH); one body-truncation parametrized test. ~3 tests.

#### 4.4 `cafleet/tests/test_tmux_capture_defaults.py` — 3 → 0

Current shape: 3 tests pinning `default_lines == 30`, override semantics, and non-positive rejection. The same invariants are **already** covered by `cafleet/tests/test_cli_member_capture_defaults.py` lines 92, 119, 149 — that coverage exists today and is preserved through Step 5.3 (the consolidation in 5.3 keeps the default-lines parametrize row). Delete `test_tmux_capture_defaults.py` in Step 4 — the suite stays green because the dependent coverage already lives outside this file.

**Total Step 4 delete**: 64 − 14 = 50 tests.

#### Step 4 tasks

- [ ] Rewrite `cafleet/tests/test_tmux.py` from 34 to 8 parametrized tests per § 4.1 <!-- completed: -->
- [ ] Rewrite `cafleet/tests/test_tmux_send_helpers.py` from 12 to ~3 parametrized tests per § 4.2 <!-- completed: -->
- [ ] Rewrite `cafleet/tests/test_tmux_send_inline_preview.py` from 15 to ~3 tests per § 4.3 <!-- completed: -->
- [ ] Delete `cafleet/tests/test_tmux_capture_defaults.py` per § 4.4 <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `refactor(tests): parametrize tmux argv-construction tests (design 0000061 step 4)` <!-- completed: -->

### Step 5: CLI overlap dedup

**Principle**: (iii). 234 tests across 19 CLI files with heavy overlap on `_resolve_prompt`, capture defaults, truncation, and pretty/compact echo.

#### 5.1 Merge `test_cli_member.py` (16) + `test_cli_member_create_prompt_file.py` (13) → 13

Both files exercise `_resolve_prompt(...)` placeholder substitution against synthetic prompt-file fixtures. Cross-file duplication: `test_default_prompt_substitution__default_path_substitutes_all_placeholders` (test_cli_member.py:56) and `test_prompt_file_substitutes_session_id_placeholder` (test_cli_member_create_prompt_file.py:102) test the same code path with the same `_resolve_prompt` call.

**Target shape**: One merged file `test_cli_member.py` with 13 tests:

- 3 parametrized tests covering the placeholder substitution matrix (default-path, custom-path-with-placeholders, doubled-brace-escape, unknown-placeholder, unmatched-brace, attribute-access).
- 2 tests on the `--prompt-file` flag specifically: relative-path-rejected, not-found / not-regular-file / empty / invalid-utf8 / not-readable (parametrized over the error variants).
- 1 test: `--prompt-file` and positional are mutually exclusive.
- 1 test: `--prompt-file` parity with positional form.
- 2 tests: trailing-newline / surrounding-whitespace preserved.
- 2 tests on coding-agent backend selection (claude default vs codex spawn argv shape; parametrize over `{claude, codex}`).
- 1 test: claude / codex binary missing → exit message.
- 1 test: permission-mode injection.

Delete `test_cli_member_create_prompt_file.py`. Net delete: 29 − 13 = 16 tests.

#### 5.2 Collapse `test_cli_message_truncation.py` — 23 → ~5

Current shape: 23 tests parametrizing `(message_poll | message_show | message_send | message_broadcast) × (text_output | json_output) × (default | full)`. The Cartesian product is the point, but each combination today is a separate `def`.

**Target shape**: 5 tests, each itself parametrized:

- `test_truncation__poll_show_send_text_output` — parametrized over `[poll, show, send]` × `[default, full]`.
- `test_truncation__poll_show_send_json_output` — same parametrization, JSON envelope.
- `test_truncation__broadcast_summary_emitted_verbatim` — parametrized over `(text_output | json_output) × (default | full)`.
- `test_truncation__non_text_fields_byte_identical_between_default_and_full` — parametrized over `[poll, show, send]`.
- `test_truncation__list_of_three_tasks_each_truncated` — kept as-is (one scenario).

Net delete: 18 tests.

#### 5.3 Collapse `test_cli_member_capture_defaults.py` — 11 → ~4

Current shape: 11 tests on `member capture` default lines, ANSI stripping, carriage-return defragmentation, JSON envelope.

**Target shape**:

- 1 parametrized test on default-lines (`[no-flag, --lines, --tail alias]`).
- 1 parametrized ANSI-stripping test (`[default__strips_ansi, default__strips_complex, ansi_flag__preserves]` × `[carriage_returns_handled, simple]`).
- 1 parametrized CR-defragmentation test (`[single_redraw, multiple_redraws_per_line]`).
- 1 test: JSON envelope `lines` field reflects new default.

Net delete: 7 tests.

#### 5.4 Collapse `test_cli_member_send_input.py` — 27 → 8

Current shape: 27 tests grouped by `test_flag_validation__*` (5), `test_authorization_boundary__*` (4), `test_choice_dispatch__*` (1), `test_freetext_dispatch__*` (4), `test_output_format__*` (5), `test_bash_flag_removed__*` (1), `test_freetext_bang_rejection__*` (7).

**Target shape — 8 tests**:

- 1 parametrized `test_flag_validation` over `(no_flag, choice+freetext combo, choice_out_of_range, freetext_newlines, freetext_empty)`.
- 1 parametrized `test_authorization_boundary` over `(missing_agent, placement_none, cross_director, pending_pane)`.
- 1 test `test_choice_dispatch__matching_digit_and_pane`.
- 1 parametrized `test_freetext_dispatch` over `(plain_ascii, shell_meta_literal, multibyte_literal, key_name_lookalike)`.
- 1 parametrized `test_output_format__text` over `(choice, choice_varies_by_digit, freetext)`.
- 1 parametrized `test_output_format__json` over `(choice, freetext)` — assert 4-key envelope.
- 1 test `test_bash_flag_removed`.
- 1 parametrized `test_freetext_bang_rejection` over `(leading, whitespace_then, lone, not_in_leading_position_accepted, empty_accepted, whitespace_only_accepted, error_wording_backend_neutral)`.

Net delete: 19 tests.

#### 5.5 Collapse `test_cli_compact_echo.py` — 11 → ~4

Current shape: 4 broadcast-default-echo tests + 1 broadcast-full-echo + 2 send-quiet/default + 2 ack-quiet/default + 2 ping-quiet/default.

**Target shape**:

- 1 parametrized test over `(broadcast, send, ack, ping)` × `(quiet, default)` — assert one-line vs multi-line shape.
- 1 test on broadcast summary canonical pattern (the specific format string match).
- 1 test on broadcast full echo (multi-line per recipient envelope).
- 1 test on `--quiet` emits only 8-char id prefix (subsumes the four quiet-mode tests via parametrize).

Net delete: 7 tests.

#### 5.6 Collapse `test_cli_pretty_flag.py` — 13 → ~5

Current shape: 13 tests across pretty-flag existence, default-off compact, on-indented, position-flexibility, compact-keys, 8-char-prefix, full-flag, body-truncation, text-mode-default, text-mode-full, message-show-compact, message-send-compact.

**Target shape**:

- 1 parametrized `test_pretty_flag` over `(default_off_compact, explicit_on_indented, can_precede_session_id, listed_in_root_help)`.
- 1 parametrized `test_compact_json__envelope_shape` over `(poll, message_show, message_send)` — assert compact keys + 8-char id prefixes.
- 1 parametrized `test_full_flag__restores_long_form` over `(typed_columns, body_truncation_disabled)`.
- 1 test: default body-truncation applies under compact envelope.
- 1 parametrized `test_text_mode` over `(default_two_lines, full_legacy_verbose)`.

Net delete: 8 tests.

#### Step 5 total

| Sub-step | Before | After | Delete |
|:--|---:|---:|---:|
| 5.1 member + prompt-file | 29 | 13 | 16 |
| 5.2 message truncation | 23 | 5 | 18 |
| 5.3 capture defaults | 11 | 4 | 7 |
| 5.4 send-input | 27 | 8 | 19 |
| 5.5 compact echo | 11 | 4 | 7 |
| 5.6 pretty flag | 13 | 5 | 8 |
| **Subtotal** | **114** | **39** | **75** |

Untouched in Step 5 (kept as-is, low-fragmentation): `test_cli_agent.py` (2), `test_cli_claude_helpers.py` (13), `test_cli_client_command.py` (7), `test_cli_doctor.py` (4), `test_cli_help_budget.py` (5), `test_cli_member_delete.py` (16), `test_cli_member_exec.py` (12), `test_cli_member_list_activity.py` (6), `test_cli_member_ping.py` (13), `test_cli_message.py` (8), `test_cli_session_bootstrap.py` (19), `test_cli_session_flag.py` (13), `test_cli_version.py` (2). These 13 files = **120 tests**. (`test_session_cli.py` is NOT in this list — it is consolidated in § 6.5.) Inspect during the step; if any reveal the same `__has_<field>` pattern, parametrize in line, otherwise leave them.

Expected savings from incidental dedup in the untouched files: ~20 tests. Plan: trim conservatively, document deltas in commit body.

#### Step 5 tasks

- [ ] Merge `test_cli_member_create_prompt_file.py` into `test_cli_member.py` per § 5.1 <!-- completed: -->
- [ ] Collapse `test_cli_message_truncation.py` per § 5.2 <!-- completed: -->
- [ ] Collapse `test_cli_member_capture_defaults.py` per § 5.3 <!-- completed: -->
- [ ] Collapse `test_cli_member_send_input.py` per § 5.4 <!-- completed: -->
- [ ] Collapse `test_cli_compact_echo.py` per § 5.5 <!-- completed: -->
- [ ] Collapse `test_cli_pretty_flag.py` per § 5.6 <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `refactor(tests): dedup CLI test overlap (design 0000061 step 5)` <!-- completed: -->

### Step 6: Broker / output formatting consolidation

**Principle**: (ii), (iii). Apply the same per-key-fragmentation consolidation to the rest of the broker and output suites.

#### 6.1 `cafleet/tests/test_broker_messaging.py` — 58 → ~25

Current shape: 58 tests grouped:

- `test_send_message__*` × 15 (return-shape, validation, persistence) — collapse to 4: shape (parametrize over key set), validation (parametrize over `[invalid_uuid, missing_agent, deregistered_agent, cross_session]`), persistence (1 test), notification at wrapper level (1 test).
- `test_broadcast_message__*` × 9 — collapse to 3: summary shape, delivery shape + origin link, recipient exclusion (parametrize over `[no_other_agents, excludes_sender, admin_exclusion_from_user_broadcast, admin_broadcast_reaches_all, bootstrap_session_admin_reaches_only_director]`).
- `test_poll_tasks__*` × 9 — collapse to 4: empty/non-empty + shape, filters (parametrize over `[status, page_size, since, broadcast_summary_excluded]`), ordering, agent-scoped.
- `test_ack_task__*` × 7 + `test_cancel_task__*` × 7 — collapse to 5: parametrize over `(ack | cancel)` × `(state_transition, timestamp, authorization, double_action_rejection, persistence)`.
- `test_get_task__*` × 7 — collapse to 4: returns task, nonexistent raises, session boundary, sender-or-recipient can read.

Target: ~25 tests. Net delete: 33.

#### 6.2 `cafleet/tests/test_broker_registry.py` — 56 → ~25

Current shape: 56 tests grouped by `create_session__*` (6), `create_session_administrator_seed__*` (6), `list_sessions__*` (6), `get_session__*` (2), `register_agent__*` (8), `get_agent__*` (5), `list_agents__*` (4), `verify_agent_session__*` (3), `deregister_agent__*` (5), `update_placement_pane_id__*` (4), `list_members__*` (4).

Target: per-API parametrized "shape + behaviour" pairs. ~25 tests total. Net delete: 31.

#### 6.3 `cafleet/tests/test_broker_webui.py` — 37 → ~12

Current shape: 37 tests over WebUI API formatter helpers covering session/timeline/inbox/sent listing, member-list, status-aggregate, pretty/compact rendering.

**Target shape — 12 tests**:

| New test | Asserts |
|:--|:--|
| `test_endpoint_listing__shape` | parametrize over `[sessions, timeline, inbox, sent, member_list]` — assert JSON envelope key set per endpoint. |
| `test_endpoint_listing__pagination_params` | parametrize over `[page, page_size, since]`. |
| `test_status_aggregate__counts_by_state` | One assertion over the full aggregate dict. |
| `test_error_envelope__shape` | parametrize over `[404, 400, 500]` — error JSON has `error` + `detail` keys. |
| `test_format_task__pretty_vs_compact` | parametrize over `[pretty, compact]` flag. |
| `test_format_agent__shape` | Combined: agent envelope key set + display-name spot-check. |
| `test_listing__filters` | parametrize over `[status_filter, agent_filter]`. |
| `test_timeline__ordering_recent_first` | One scenario. |
| `test_inbox_and_sent__ordering` | parametrize over `[inbox, sent]`. |
| `test_endpoint_listing__empty_returns_empty_array` | parametrize over the 5 endpoints. |
| `test_cross_session_boundary__rejects_foreign_session_id` | One scenario. |
| `test_root_endpoint__advertises_routes` | Sanity check that `/` lists active endpoints. |

Net delete: 25.

#### 6.4 `cafleet/tests/test_session_bootstrap.py` — 29 → ~12

Current shape: Director seeding, Administrator seeding, table creation, idempotency.

**Target shape — 12 tests**:

| New test | Asserts |
|:--|:--|
| `test_bootstrap__seed_rows_shape` | parametrize over `[director, administrator]` — assert seed row key set + role value. |
| `test_bootstrap__idempotent_on_replay` | Re-bootstrap leaves rows unchanged. |
| `test_bootstrap__preserves_existing_unrelated_rows` | Pre-seeded rows survive bootstrap. |
| `test_bootstrap__attaches_pane_placement` | parametrize over `[current_pane, explicit_pane]`. |
| `test_bootstrap__tmux_binding_row_shape` | Binding row has expected keys. |
| `test_bootstrap__pins_schema_version` | After bootstrap, `alembic_version` matches `head`. |
| `test_bootstrap__removes_legacy_tables` | parametrize over `[api_keys, task_json_blob]` — confirmed absent. |
| `test_bootstrap__refuses_on_dirty_db` | Bootstrap on existing-conflicting-rows raises. |
| `test_bootstrap__atomic_on_failure` | Mid-bootstrap exception → DB unchanged. |
| `test_bootstrap__seed_row_uniqueness` | parametrize over `[name, role]` — unique constraints fire. |
| `test_bootstrap__returns_ids_match_get_agent` | Returned IDs are retrievable via `get_agent`. |
| `test_bootstrap__empty_inert_when_already_done` | No work, exit 0. |

Net delete: 17.

#### 6.5 `cafleet/tests/test_session_cli.py` — 24 → ~10

Current shape: `session create / list / delete` CLI verbs.

**Target shape — 10 tests**:

| New test | Asserts |
|:--|:--|
| `test_session_create__happy_path` | parametrize over `[text_output, json_output]` — assert envelope shape. |
| `test_session_create__label_flag_round_trip` | `--label` value is echoed in subsequent `get`. |
| `test_session_create__attaches_director_to_current_pane` | Director's placement row points at the calling pane. |
| `test_session_list__shape` | parametrize over `[text_output, json_output]`. |
| `test_session_list__filters` | parametrize over `[--include-deleted_shows_soft, default_hides_soft]`. |
| `test_session_delete__soft_deletes_session` | Row's `deleted_at` is set. |
| `test_session_delete__authorization_boundary` | parametrize over `[unknown_session_id, non_administrator_actor]`. |
| `test_session_create__rejects_outside_tmux` | Without `$TMUX`, exit 2 with clear message. |
| `test_session_delete__idempotent_on_already_deleted` | Second `delete` exits 0 with informational message. |
| `test_session_create__bootstraps_administrator_and_director` | Aggregate check: after create, both seed rows exist. |

Net delete: 14.

#### 6.6 `cafleet/tests/test_broker_administrator.py` — 18 → ~6

Administrator-specific invariants.

**Target shape — 6 tests**:

| New test | Asserts |
|:--|:--|
| `test_administrator__non_deregisterable` | `deregister_agent(admin_id)` raises. |
| `test_administrator__excluded_from_user_broadcast` | User-initiated broadcast does not deliver to administrator. |
| `test_administrator__broadcast_reaches_all` | Administrator-initiated broadcast reaches all non-admin agents. |
| `test_administrator__bootstrap_creates_exactly_one` | parametrize over `[fresh_session, double_bootstrap]`. |
| `test_administrator__authorization_boundary` | parametrize over `[register_attempt_as_admin_role, escalation_via_update]`. |
| `test_administrator__metadata_shape` | Administrator row key set + role value. |

Net delete: 12.

#### 6.7 `cafleet/tests/test_broker_inline_preview.py` — 16 → ~6

Inline-preview rendering.

**Target shape — 6 tests**:

| New test | Asserts |
|:--|:--|
| `test_inline_preview__happy_path_2_line_shape` | Header + body, both expected lines. |
| `test_inline_preview__skip_conditions` | parametrize over `[pane_dead, self_send, missing_placement, no_tmux_on_path]`. |
| `test_inline_preview__body_truncation` | parametrize over `[ascii_long, multibyte_long, embedded_newlines]`. |
| `test_inline_preview__header_from_agent_display` | Short agent-id displayed in header. |
| `test_inline_preview__header_timestamp_format` | Header timestamp matches expected format. |
| `test_inline_preview__end_to_end_after_message_send` | After `message send`, preview keystroke landed in recipient pane. |

Net delete: 10.

#### 6.8 `cafleet/tests/test_broker_member_activity.py` — 18 → ~6

Member activity tracking.

**Target shape — 6 tests**:

| New test | Asserts |
|:--|:--|
| `test_member_activity__row_created_on_member_create` | parametrize over `[claude_backend, codex_backend]`. |
| `test_member_activity__row_updated_on_action` | parametrize over `[message_send, member_capture, member_ping]`. |
| `test_member_activity__row_schema_shape` | Row key set: `kind`, `ts`, `payload_keys`. |
| `test_member_activity__per_member_ordering` | Recent-first per member. |
| `test_member_activity__per_session_aggregation` | Aggregate counts per kind. |
| `test_member_activity__stale_pruning` | TTL/prune behaviour. |

Net delete: 12.

#### 6.9 Output formatting consolidation — 91 → ~28

| File | Before | After | Notes |
|:--|---:|---:|:--|
| `test_output.py` | 22 | 8 | `format_member` / `format_member_list` / `truncate_text` — parametrize over shape × edge cases. |
| `test_output_compact_formatters.py` | 27 | 8 | Compact-rendering helpers — parametrize over helper × scenario. |
| `test_output_indexed_list.py` | 4 | 2 | Keep happy-path + empty-list. |
| `test_output_render_agent.py` | 13 | 4 | Same per-key fragmentation as render_task — collapse. |
| `test_output_render_broadcast_summary.py` | 5 | 2 | Collapse to "full envelope" + "recipient-count formatting". |
| `test_output_truncation_settings.py` | 20 | 4 | Parametrize over `(env_var_present, codepoint_limit_value)`. |

Net delete: 63.

#### 6.10 Server / WebUI — 21 → ~10

| File | Before | After |
|:--|---:|---:|
| `test_server_cli.py` | 15 | 7 |
| `test_webui_api_format.py` | 6 | 3 |

Net delete: 11.

#### Step 6 total

| Sub-step | Before | After | Delete |
|:--|---:|---:|---:|
| 6.1 broker messaging | 58 | 25 | 33 |
| 6.2 broker registry | 56 | 25 | 31 |
| 6.3 broker webui | 37 | 12 | 25 |
| 6.4 session bootstrap | 29 | 12 | 17 |
| 6.5 session CLI | 24 | 10 | 14 |
| 6.6 administrator | 18 | 6 | 12 |
| 6.7 inline preview | 16 | 6 | 10 |
| 6.8 member activity | 18 | 6 | 12 |
| 6.9 output formatting | 91 | 28 | 63 |
| 6.10 server / webui | 21 | 10 | 11 |
| **Subtotal** | **368** | **140** | **228** |

#### Step 6 tasks

- [ ] Consolidate `test_broker_messaging.py` per § 6.1 <!-- completed: -->
- [ ] Consolidate `test_broker_registry.py` per § 6.2 <!-- completed: -->
- [ ] Consolidate `test_broker_webui.py` per § 6.3 <!-- completed: -->
- [ ] Consolidate `test_session_bootstrap.py` per § 6.4 <!-- completed: -->
- [ ] Consolidate `test_session_cli.py` per § 6.5 <!-- completed: -->
- [ ] Consolidate `test_broker_administrator.py` per § 6.6 <!-- completed: -->
- [ ] Consolidate `test_broker_inline_preview.py` per § 6.7 <!-- completed: -->
- [ ] Consolidate `test_broker_member_activity.py` per § 6.8 <!-- completed: -->
- [ ] Consolidate the 6 output-formatting files per § 6.9 (one PR, but commit may stage them together) <!-- completed: -->
- [ ] Consolidate `test_server_cli.py` and `test_webui_api_format.py` per § 6.10 <!-- completed: -->
- [ ] Run `mise //cafleet:test`; commit as `refactor(tests): consolidate broker/output formatting tests (design 0000061 step 6)` <!-- completed: -->

### Step 7: Final verification

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`. Confirm all exit 0 and the test count is in the **≤ 400** band (stretch: ≤ 340 if Step 5 incidental dedup achieved ≥ 30%). Tally the per-file post-cleanup counts in a comment on the design doc PR for the reviewer. <!-- completed: -->

---

## Projected totals

| Category | Before | After | Delta |
|:--|---:|---:|---:|
| Alembic per-revision (Step 1) | 31 | 0 | -31 |
| PRAGMA (Step 2) | 2 | 0 | -2 |
| Per-key projection (Step 3) | 95 | 20 | -75 |
| Tmux argv construction (Step 4) | 64 | 14 | -50 |
| CLI overlap (Step 5, parametrized 6 files) | 114 | 39 | -75 |
| Broker / output / server (Step 6) | 368 | 140 | -228 |
| **Touched subtotal** | **674** | **213** | **-461** |
| Untouched CLI files (Step 5 carve-out, 13 files) | 120 | ~100 | ~-20 incidental |
| Untouched base-dir suite | 27 | 27 | 0 |
| Off-limits (token_budget + alembic_smoke + db_init) | 20 | 20 | 0 |
| **Grand total** | **841** | **~ 360** | **~ -481** |

The "Before" grand total of 841 matches the `grep -cE "^def test_"` count over `cafleet/tests/*.py`. The "After" projection of ~360 satisfies the Success Criteria ceiling of **≤ 400** with ~40 tests of execution-variance margin. If Step 5's incidental dedup of the untouched-CLI carve-out reaches a deeper trim (≥ 30%, i.e. ≥ 36 deletions instead of ~20), the final figure can reach the **stretch ≤ 340 band** — a 60% reduction.

---

## Out of scope

- **Source code under `cafleet/src/cafleet/`** is not modified by this design.
- **Conftest and helper modules** are not touched (per Q6).
- **Admin frontend** has no test suite (confirmed); irrelevant to this design.
- **New test additions** are not part of this design — only deletions and consolidations of existing tests.
- **`pytest.mark` taxonomy / coverage thresholds / CI-level gating** changes are out of scope.

## Changelog

| Date | Changes |
|:--|:--|
| 2026-05-16 | Initial draft. |
| 2026-05-16 | Round 2 (Director-resolved): reconciled Success Criteria ceiling (≤ 400, stretch ≤ 340) with Projected totals (~360); removed `test_session_cli` double-count from Step 5 carve-out; corrected § 3.2, § 5.1, § 5.4 target counts; added base-dir off-limits rationale; dropped `_helpers.py` mutation directive from Step 1; enumerated § 4.1 per-helper parametrize table; enumerated § 6.3–§ 6.8 target test shapes; clarified § 4.4 ordering invariant. |
