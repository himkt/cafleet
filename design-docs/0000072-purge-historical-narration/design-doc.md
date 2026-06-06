# 0000072 — Purge Historical-Trajectory Narration

**Status**: Approved
**Progress**: 12/24 tasks complete
**Last Updated**: 2026-06-06

## Overview

The repository carries "historical-trajectory" narration — text meaningful only to someone who knows the project's past ("X was deprecated in design NNNN", "previously Y, now Z", "preserved for forensic visibility", design-doc-number citations as the reason for current code, and sentinel "removed-flag → error" tests). This change deletes or rewrites every such mention across `docs/`, `README.md`, `.claude/`, `skills/`, `cafleet/src/` (excluding Alembic migrations), and `cafleet/tests/` so the repository reads as if the removed/rejected features never existed.

## Success Criteria

- [ ] Every inventory item in the Specification table is resolved (removed or reworded to present-tense current-behavior phrasing).
- [ ] `docs/spec/data-model.md` no longer documents `coding_agent = "unknown"` as a current value, and its root-Director bootstrap description matches `docs/spec/cli-options.md` (default `"claude"`).
- [ ] No source, doc, skill, or test (outside the exempt areas) cites a design-doc number as the reason for current code, or narrates a removal/deprecation.
- [ ] Every "removal-sentinel" test (asserts a non-existent flag/key errors) is deleted; every test that mixes a sentinel with live coverage keeps the live assertion and loses only the sentinel framing — no live coverage is lost.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.
- [ ] A re-run of the verification sweep (Step 7) returns zero **unaccounted** matches — every remaining hit is KEEP-listed, exempt, or a benign present-tense usage confirmed by judgment (see "Known-benign sweep matches" in the KEEP list); no historical-trajectory narration remains.

---

## Background

The governing rule is `~/.claude/rules/removal.md` ("Removal"). Its principle: git history and the design doc are the historical record; source, user-facing docs, skills, and examples describe **only the current state**. Deprecation notices and "for history" pointers turn user-facing surfaces into archaeological digs.

This is a documentation/test-hygiene cleanup. It removes **no runtime behavior** — every CLI flag, column, and code path that exists today still exists after this change. Only the *narration about what used to exist* is removed.

### The keep / remove line (authoritative for this change)

`removal.md` contains two passages that, read together, leave the test-side line ambiguous. The user resolved it explicitly for this change:

| Surface | Action |
|---|---|
| Historical narration in **user-facing docs** (deprecation notices, "previously/now/no longer/formerly", "preserved for forensic visibility", restoration pointers, design-number-as-reason) | **Remove or reword** to present tense |
| **Removal-sentinel tests** — a test whose *only* job is to assert a removed/never-added flag or key produces an error (`--bash`/`--pretty`/`--all` → Click "No such option"; "removed wrapper keys re-emerging"; `FORBIDDEN_LEGACY_KEYS`) | **Delete the test/assertion outright** (this change overrides removal.md's "keep the absence guard" reading) |
| A test that **mixes** a current-behavior assertion with a removal sentinel | **Keep the current-behavior assertion; drop only the sentinel part.** Never lose live coverage. Each such case is flagged below. |
| Alembic migration files (`cafleet/src/cafleet/alembic/versions/*`) | **Exempt** — a migration's purpose is to describe a transformation, so it must reference the prior state. Treat like git history. |
| A real **Non-goals / out-of-scope** section scoping *this* feature, with no historical reference | **Keep** (not cruft) |
| Design-number **example paths / test fixtures** that are illustrative input, not citations-as-reason | **Keep** |
| Forward-looking design-debt notes (e.g. the `acknowledged_at` note) and "do-not-refactor-this-into-that" editing guards | **Keep** (forward-facing, not historical) |

---

## Specification

### Decisions (from the user)

1. **Q1 — Alembic migrations: exempt entirely.** No edits to `cafleet/src/cafleet/alembic/versions/*`. This includes the `0006`/`0007` design-doc citations and the `0009`/`0010` "legacy"/"unknown" framing.
2. **Q2 — Removal-sentinel tests: delete outright** (not reword). Safety clause: if a single test function mixes a current-behavior assertion with a sentinel, keep the current-behavior assertion and drop only the sentinel part.
3. **Q3 — Migration prose in user docs: strip the "pre-existing session" / "backfill" narration clauses; keep the real migration *filename* references.**
4. **Q4 — Version qualifiers: sweep them.** Reword "in v1" / "first cut" / "no pagination in the first cut" into plain present-tense statements.
5. **Q5 — Output format: a `file → action` inventory table plus an explicit KEEP list as the doc's spine.**

> **Line numbers below are current-HEAD references and may drift as edits are applied. The quoted text is the stable anchor — match on the text, not the number.**

### Inventory — `docs/` (user-facing prose)

| # | Location | Current text (anchor) | Action |
|---|---|---|---|
| D1 | `docs/spec/data-model.md` ~L31 | `coding_agent='unknown'` (auto-detection is deferred) | **Reword** → `coding_agent=<value of --coding-agent>` (default `'claude'`), matching `cli-options.md` "Root Director bootstrap" (the placement records the chosen backend; new sessions never write `"unknown"`). |
| D2 | `docs/spec/data-model.md` ~L126 | `coding_agent` row: "Current known values are `"claude"` … and `"unknown"` for the session's root Director placement" | **Reword** → drop `"unknown"`. Keep the column constraints (`NOT NULL`, `DEFAULT 'claude'`). Describe current values: `"claude"` (default) and `"codex"` / `"opencode"` when chosen at create time. Do **not** document the dead `"unknown"` value (migration `0010` already backfilled it to `"claude"`). |
| D3 | `docs/spec/data-model.md` ~L214 | table row `Historical row from before the 0002_add_origin_task_id migration | NULL (no backfill)` | **Remove the row.** |
| D4 | `docs/spec/data-model.md` ~L230 | "The previous `DEREGISTERED_TASK_TTL_DAYS` and `CLEANUP_INTERVAL_SECONDS` settings have been removed." | **Remove the sentence.** Also reword the preceding "it can be **reintroduced** as an opt-in admin command" → "it can be **added** as an opt-in admin command" (drop the implication it once existed). |
| D5 | `docs/spec/webui-api.md` ~L154 | table row `Historical row from before the origin_task_id migration | null` | **Remove the row.** |
| D6 | `docs/concepts/tmux-push.md` ~L58–61 | "This replaces the design-0000049-pre keystroke (the literal … sequence, which forced the recipient to dump its full unacked-inbox envelope on every send and dominated per-message token cost)." | **Remove the whole sentence.** The surrounding paragraph already states current behavior. |
| D7 | `docs/concepts/coding-agents.md` ~L10–11 | "the default is `claude` so existing invocations behave unchanged" | **Reword** → "the default is `claude`." |
| D8 | `docs/reference/coding-agents/codex.md` ~L16 | "The default is `--coding-agent claude`, so existing invocations are unchanged." | **Reword** → drop "so existing invocations are unchanged". |
| D9 | `docs/reference/coding-agents/opencode.md` ~L16 | "The default is `--coding-agent claude`, so existing invocations are unchanged." | **Reword** → drop "so existing invocations are unchanged". |
| D10 | `docs/concepts/storage.md` ~L138 | "are not exposed via the CLI in v1." | **Reword (Q4)** → "are not exposed via the CLI." |
| D11 | `docs/spec/data-model.md` ~L23 | "no `--all` flag in v1" | **Reword (Q4)** → "no `--all` flag". |
| D12 | `docs/spec/data-model.md` ~L143 | "There is no path in v1 that physically deletes an agent" | **Reword (Q4)** → "There is no path that physically deletes an agent". |
| D13 | `docs/spec/data-model.md` ~L224 | "This is accepted residual risk for the first cut of the Discord-style timeline and is explicitly flagged here so the next contributor…" | **Reword (Q4)** → drop "for the first cut of the Discord-style timeline". **Keep** the rest of this forward-looking design-debt note. |
| D14 | `docs/spec/webui-api.md` ~L144 | "Hard-capped at 200 rows. No pagination in the first cut." | **Reword (Q4)** → "Hard-capped at 200 rows; no pagination." |
| D15 | `docs/spec/data-model.md` ~L80 | "Alembic revision `0006_seed_administrator_agent.py` **backfills one Administrator into each pre-existing session**. …" | **Reword (Q3)** → strip **only** the "into each pre-existing session" clause; state present-tense that revision `0006_seed_administrator_agent.py` is the migration that seeds the Administrator. **Keep** the filename, the `json_extract` probe, the idempotency note, **and** the downgrade-behavior sentence ("Downgrade is provided for empty sessions only … `tasks.context_id` uses `ON DELETE RESTRICT` … raises `IntegrityError`") — that documents the migration's current downgrade behavior, not historical trajectory, and is outside Q3's scope. |
| D16 | `docs/concepts/session-isolation.md` ~L106–108 | "Alembic revision `0006_seed_administrator_agent.py` **backfills one into each pre-existing session** on `cafleet db init` (idempotent via a `json_extract` probe)." | **Reword (Q3)** → strip the "backfills one into each pre-existing session on `cafleet db init`" clause. Keep "Every session has exactly one Administrator" and a bare current-state filename reference. |
| D17 | `docs/spec/message-envelope.md` ~L26 | "… and the `0009_drop_task_json_add_text` migration shape **with the operator backup procedure**)." | **Reword (Q3, judgment)** → keep the `0009_drop_task_json_add_text` filename reference; drop "with the operator backup procedure". |

> **Note — D1/D2 vs. Out-of-scope.** `"unknown"` is a *removed* coding-agent value: migration `0010` backfilled it to `"claude"` and `broker.create_session` never writes it, so documenting it is historical narration, in scope per removal.md's "do not document the removed value." This is distinct from the `click.Choice(['claude','codex'])` docstring listed under Out of scope — that is a *live-but-undercounted* enum (plain accuracy drift, not a removed value), which is why it stays out of scope.

### Inventory — config

| # | Location | Current text (anchor) | Action |
|---|---|---|---|
| C1 | `.gitignore` ~L40–43 | "# Legacy spawn-prompt audit directory at the repo root. Audit files are now written under per-task folders …; this entry keeps any straggler writes during the rollout out of version control." | **Reword** → present tense, drop "Legacy" / "now" / "during the rollout". E.g. "# Stray spawn-prompt audit writes at the repo root; the canonical location is per-task `prompts/` folders (`researches/<slug>/prompts/`, `design-docs/<NNNNNNN>-<slug>/prompts/`)." Keep the `/prompts/` ignore entry itself. |
| C2 | `.claude/rules/commands.md` ~L28 | "`mise //cafleet:test -- --collect-only -q tests/` — design-docs/0000044 uses this form." | **Reword** → drop "— design-docs/0000044 uses this form" (a design-number-as-reason citation; forbidden by removal.md). The guidance stands alone: "…`mise //cafleet:test -- --collect-only -q tests/`. The bare form (without `--`) works in practice…". |

### Inventory — `cafleet/tests/`: delete pure sentinels (Q2)

| # | Location | Function | Action |
|---|---|---|---|
| T1 | `cafleet/tests/test_cli_member_send_input.py` ~L284–291 | `test_bash_flag_removed__old_bash_flag_form_errors_with_no_such_option` | **Delete the entire function.** Pure sentinel (only asserts `--bash` → "No such option"). |
| T2 | `cafleet/tests/test_cli_version.py` ~L24–28 | `test_pretty_flag_rejected_as_unknown_option` | **Delete the entire function.** Pure sentinel (only asserts `--pretty` → "No such option"). The `--version` test above it is unrelated — keep it. |
| T3 | `cafleet/tests/test_cli_session_bootstrap.py` ~L335–339 | `test_session_list_hides_soft_deleted__list_has_no_all_flag` | **Delete the entire function.** Pure sentinel (only asserts `session list --all` → exit 2) **and** carries a `Design 0000026 explicitly removes/omits an --all flag` docstring. The live "hides soft-deleted" coverage lives in the **preceding** `test_session_list_*` function and is untouched. |

### Inventory — `cafleet/tests/`: mixed cases — keep live coverage, drop sentinel (Q2 safety)

| # | Location | Action |
|---|---|---|
| T4 | `cafleet/tests/test_cli_session_flag.py` ~L157–166 (`test_session_id_flag_flows_into_broker__session_id_not_read_from_environment`) | **Keep** the test + assertion (register without `--session-id`, with `CAFLEET_SESSION_ID` set, exits 1 → the env var is never consulted). **Replace** the docstring "The env var CAFLEET_SESSION_ID was removed; only the CLI flag works." with present tense, e.g. "Session id is read only from the `--session-id` flag; the environment is never consulted." |
| T5 | `cafleet/tests/test_output_render_broadcast_summary.py` ~L36–40 | **Delete** the `# Defensive guards against removed wrapper keys re-emerging.` comment and the `for forbidden in ("recipient_ids", "recipientIds", "metadata"): assert forbidden not in summary` loop. **Keep** the exact-key-set assertion at ~L36–37 (`extra = set(summary.keys()) - expected_keys; assert not extra`) — it already excludes those keys, so **no coverage is lost**. |
| T6 | `cafleet/tests/test_broker_typed_columns.py` ~L42–54, ~L60–61 | **Delete** the `FORBIDDEN_LEGACY_KEYS` constant and the forbidden-absent check in `_assert_flat_typed_shape` (`forbidden = FORBIDDEN_LEGACY_KEYS & d.keys(); assert not forbidden, …`). **Keep** the `REQUIRED_TASK_KEYS` present-check (positive live coverage of the flat typed shape). This intentionally drops the named "old keys must not re-emerge" guard (the exact anti-pattern the user named). *Optional, non-required hardening:* the executor MAY tighten the present-check to exact-set equality (`assert d.keys() == REQUIRED_TASK_KEYS`) to retain strictness, **but must not re-introduce a named legacy-key enumeration.** **Most delicate item — verify the full suite is green after the edit** (`_assert_flat_typed_shape` is shared across the module). |
| T7 | `cafleet/tests/test_cli_session_bootstrap.py` ~L239–245 (`test_session_create_coding_agent__default_is_claude`) | **Keep** the test + assertion (new session → `coding_agent == "claude"`). **Simplify** the docstring "No flag → 'claude'. 'unknown' must not appear for newly-created sessions." → "No `--coding-agent` flag defaults the placement to `'claude'`." (drop the dead-value reference). |

### Inventory — non-sentinel test rewords (design-number citations + incidental sweep matches; reword, keep coverage)

| # | Location | Action |
|---|---|---|
| T8 | `cafleet/tests/test_server_routing.py` ~L3 | **Reword** the module docstring: drop "Guards the behaviors specified in design-doc 0000068 §Part A Backend and §Risks:" → "Guards the following behaviors:". Keep the behavior bullets. |
| T9 | `cafleet/tests/test_server_routing.py` ~L64 | **Reword** the comment "Mount-order regression guard from design-doc 0000068 §Risks: if the SPA…" → "Mount-order regression guard: if the SPA…". |
| T10 | `cafleet/tests/test_base_dir_spawn_flow.py` ~L1 | **Reword** the module docstring "Integration tests for the design-0000055 spawn-prompt + audit-write contract." → "Integration tests for the spawn-prompt + audit-write contract." |
| T11 | `cafleet/tests/test_base_dir_spawn_flow.py` ~L194 | **Reword** the comment "# Design 0000060 — task-scoped spawn-prompt audit-write flow" → "# Task-scoped spawn-prompt audit-write flow". |
| T12 | `cafleet/tests/test_output_render_task.py` ~L165 | **Reword** the incidental fixture string `_typed_task(text="legacy body")` → `text="message body"`. Matches the `\blegacy\b` sweep but is plain test data, not a removal note; no coverage change. |

### Inventory — `skills/` design-doc pointers (judgment — Reviewer-confirm)

| # | Location | Action |
|---|---|---|
| S1 | `skills/cafleet-design-doc/coordination.md` ~L1 | **Remove** the bare pointer line "Rationale-of-record: design-docs/0000050-design-doc-as-medium/design-doc.md." (a design-number-as-reason pointer in a skill reference). |
| S2 | `skills/cafleet-design-doc-interview/SKILL.md` ~L26 | **Reword** — drop the parenthetical "(see `design-docs/0000050-design-doc-as-medium/design-doc.md` Step 7 for the plugin-install self-containment rationale)". Keep the substantive statement that no cross-skill markdown link is added, by design. |

### Explicit KEEP list (do **not** delete — prevents over-deletion)

- **Live test assertions** in T4–T11 — only framing changes; every assertion stays.
- `docs/concepts/tmux-push.md` ~L63–69 — "The `TmuxMultiplexer.send_inline_preview` method is **NOT** a reuse of `send_freetext_and_submit` …" — a current "do-not-refactor-this-into-that" editing guard, not historical narration.
- `skills/cafleet/reference/broadcast.md` — "`--full` … preserved for surface consistency …" / "the broker only ever returns the single summary …" — current rationale for a current flag's behavior, not a removal note.
- `docs/spec/data-model.md` ~L224 — the forward-looking `acknowledged_at` design-debt note (only the "first cut" qualifier is reworded per D13).
- Design-number **example paths / fixtures** (illustrative input-path examples, not citations-as-reason — leave them): `cafleet/src/cafleet/base_dir.py` ~L213 (`design-docs/0000060-foo/design-doc.md`); the `0000060`/`0000099` fixture slugs in `cafleet/tests/test_base_dir.py` and `test_base_dir_spawn_flow.py`; and the `0000060-…` example slugs in `skills/cafleet-design-doc-create/SKILL.md` (~L209, L215), `skills/cafleet-design-doc-execute/SKILL.md` (~L220, L222, L228), `skills/cafleet-design-doc-interview/SKILL.md` (~L110, L116), and `skills/cafleet-base-dir/SKILL.md` (~L25).
- `docs/spec/cli-options.md` `--coding-agent` rows — current flag; this file is the source of truth the D1 reword aligns to.
- `README.md`, `CLAUDE.md` — **swept and already clean** of historical-trajectory narration; no edits needed (recorded so the absence of edits is intentional, not an oversight). `.claude/rules/*` — swept; the only narration-style citation to fix is item **C2** (`commands.md` ~L28). The `previously` at `commands.md` ~L11 is benign (see "Known-benign sweep matches").
- **Known-benign sweep matches** — present-tense English that trips the broad Step 7 patterns but is **not** historical narration; do **not** reword:
  - `cafleet/src/cafleet/broker.py` ~L1214 — "Session UUID **used to** gate visibility" (`used to` = *utilized to*, not *formerly*).
  - `.claude/rules/commands.md` ~L11 — "the global `cafleet` binary was **previously** installed non-editably" (a conditional describing the user's own environment, not project history).
  - `skills/cafleet-design-doc-execute/SKILL.md` ~L258 — "Tier 3 is **preserved for** the no-argument branch" (`preserved for` = *retained for*, current behavior).
  - The implementer confirms each remaining sweep hit falls in this class (or is exempt / KEEP-listed) before declaring the sweep clean.

### Out of scope

- `cafleet/src/cafleet/alembic/versions/*` — **exempt** (Q1).
- `cafleet/src/cafleet/webui/assets/**` — generated WebUI build output (minified `index-*.js` / `index-*.css` bundles, rebuilt by `mise //admin:build`; not authored prose). Excluded from the sweep like lock files; minified strings produce false matches and must never be hand-edited.
- `design-docs/` and `researches/` — the historical record; not touched.
- General doc-drift that is **not** historical narration. Example: `cafleet/tests/test_cli_session_bootstrap.py` ~L249 docstring says `click.Choice(['claude','codex'])` but the live choice set is `['claude','codex','opencode']`. That is an accuracy bug, not trajectory narration — leave it for a separate change.
- Any runtime behavior change. Flags, columns, and code paths are unchanged.

---

## Implementation

> Documentation is edited first (project rule), then tests, then verification.
> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`

### Step 1: User-facing docs — remove/reword historical narration

- [x] D1 + D2 — `docs/spec/data-model.md`: fix the root-Director `coding_agent` description and the `coding_agent` column row (drop `"unknown"`; align to `cli-options.md`) <!-- completed: 2026-06-06T02:28 -->
- [x] D3 + D5 — remove the two "Historical row from before the … migration" table rows (`data-model.md`, `webui-api.md`) <!-- completed: 2026-06-06T02:28 -->
- [x] D4 — `docs/spec/data-model.md`: remove the "previous … settings have been removed" sentence; reword "reintroduced" → "added" <!-- completed: 2026-06-06T02:28 -->
- [x] D6 — `docs/concepts/tmux-push.md`: remove the "This replaces the design-0000049-pre keystroke …" sentence <!-- completed: 2026-06-06T02:28 -->
- [x] D7 + D8 + D9 — drop "so existing invocations (behave/are) unchanged" in `coding-agents.md`, `codex.md`, `opencode.md` <!-- completed: 2026-06-06T02:28 -->
- [x] D10 + D11 + D12 + D13 + D14 — sweep version qualifiers ("in v1", "first cut", "no pagination in the first cut") to present tense <!-- completed: 2026-06-06T02:28 -->
- [x] D15 + D16 + D17 — strip "pre-existing session"/backfill narration in migration prose (`data-model.md`, `session-isolation.md`, `message-envelope.md`); keep filename references <!-- completed: 2026-06-06T02:28 -->

### Step 2: Config

- [x] C1 — `.gitignore`: reword the `/prompts/` comment to present tense (drop "Legacy"/"now"/"during the rollout") <!-- completed: 2026-06-06T02:31 -->
- [x] C2 — `.claude/rules/commands.md`: drop the "— design-docs/0000044 uses this form" citation <!-- completed: 2026-06-06T02:33 -->


### Step 3: Tests — delete pure removal-sentinels (Q2)

- [x] T1 — delete `test_bash_flag_removed__old_bash_flag_form_errors_with_no_such_option` <!-- completed: 2026-06-06T02:35 -->
- [x] T2 — delete `test_pretty_flag_rejected_as_unknown_option` <!-- completed: 2026-06-06T02:35 -->
- [x] T3 — delete `test_session_list_hides_soft_deleted__list_has_no_all_flag` <!-- completed: 2026-06-06T02:35 -->

### Step 4: Tests — mixed cases (keep live coverage, drop sentinel)

- [ ] T4 — `test_cli_session_flag.py`: reword the `CAFLEET_SESSION_ID was removed` docstring; keep the assertion <!-- completed: -->
- [ ] T5 — `test_output_render_broadcast_summary.py`: delete the forbidden-keys loop + comment; keep the exact-key-set assertion <!-- completed: -->
- [ ] T6 — `test_broker_typed_columns.py`: delete `FORBIDDEN_LEGACY_KEYS` + its check; keep the required-keys present-check (verify suite green) <!-- completed: -->
- [ ] T7 — `test_cli_session_bootstrap.py`: simplify the `default_is_claude` docstring (drop the `'unknown'` reference) <!-- completed: -->

### Step 5: Tests — design-doc-number citations (reword)

- [ ] T8 + T9 — `test_server_routing.py`: drop the `design-doc 0000068` citations from the docstring and the mount-order comment <!-- completed: -->
- [ ] T10 + T11 — `test_base_dir_spawn_flow.py`: drop the `design-0000055` / `Design 0000060` citations from the docstring and comment <!-- completed: -->
- [ ] T12 — `test_output_render_task.py`: reword the incidental `text="legacy body"` fixture → `text="message body"` <!-- completed: -->

### Step 6: Skills — design-doc pointers (judgment)

- [ ] S1 — `skills/cafleet-design-doc/coordination.md`: remove the "Rationale-of-record: design-docs/0000050…" pointer line <!-- completed: -->
- [ ] S2 — `skills/cafleet-design-doc-interview/SKILL.md`: drop the `design-docs/0000050` parenthetical; keep the "by design" statement <!-- completed: -->

### Step 7: Verification

- [ ] Run `mise //cafleet:test` — full suite green (special attention to T6) <!-- completed: -->
- [ ] Run `mise //cafleet:lint` and `mise //cafleet:typecheck` — clean <!-- completed: -->
- [ ] Re-run the sweep (use `git grep` over tracked files, which also excludes the untracked WebUI build output) and confirm zero matches **outside** the exempt areas (`design-docs/`, `researches/`, `cafleet/src/cafleet/alembic/`, `cafleet/src/cafleet/webui/assets/**`, lock files), for: `deprecat`, `no longer|formerly|previously|used to|historically`, `forensic|preserved for|for history`, `restoration`, `\blegacy\b`, `design[ -]?0000[0-9]{3}|\b0000[0-9]{3}\b` (word-boundary form so all-zero UUID constants like `00000000-0000-…` do not false-match; the KEEP-listed example slugs `0000060`/`0000099` are expected matches to ignore), `in v1|first cut`, `was removed|removed .* re-emerg`. Each remaining hit is either KEEP-listed or exempt — record any judgment calls. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-06 | Initial draft |
| 2026-06-06 | Reviewer pass: added C2 (`.claude/rules/commands.md`) and T12 (`test_output_render_task.py`); exempted generated WebUI build output; resolved the D15 downgrade-narration judgment (keep); added the D1/D2-vs-Out-of-scope note; extended example-slug KEEP coverage to skill files; tightened the Step 7 sweep to a word-boundary form |
| 2026-06-06 | Reviewer pass: reworded the sweep success criterion to "zero unaccounted matches"; added the "Known-benign sweep matches" KEEP bullet |
| 2026-06-06 | User approved; Status → Approved |
