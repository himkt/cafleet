# 0000131 — Historical-Residue Cleanup Skill (CAFleet-orchestrated)

**Status**: Approved
**Progress**: 7/11 tasks complete
**Last Updated**: 2026-07-12

## Overview

Specify and build a reusable, project-local CAFleet-orchestrated skill at `.claude/skills/historical-residue-cleanup/` that, on demand, sweeps the whole repository for historical-context narration and historical-guard residue and removes it per `~/.claude/rules/removal.md` and `~/.claude/rules/affirmative-writing.md` — preserving every runtime behavior and every live test assertion. The residue present in the current tree (the Director's 14-site scan plus the broader ~28-citation design-number family across 20 test files, of which D1–D3 overlap the 14-site scan) is the skill's first worked example and its first-run target, not the deliverable itself. This change removes **no** runtime behavior: every CLI flag, table, column, and code path that exists today still exists after each run.

## Success Criteria

- [ ] The skill exists at `.claude/skills/historical-residue-cleanup/` with a `SKILL.md`, `roles/scanner.md`, `roles/reviewer.md`, `reference/rubric.md`, and `reference/patterns.md`, and loads via the Skill tool (its `description` triggers on "clean up historical narration / residue", "remove deprecation notes", "affirmative-writing sweep").
- [ ] **R1 encoded** — the skill both *removes* existing narration and is *forbidden from introducing* any new narration: every edit it makes reads as a clean present-tense statement of current behavior (no "previously/now/no longer/formerly", no "this replaces X", no "renamed from Y"). Stated as a `SKILL.md` instruction, a `reviewer.md` check, and a rubric rule.
- [ ] **R2 encoded** — the skill never modifies `design-docs/`, `researches/`, `cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`, or lock files.
- [ ] **R3 encoded** — the skill sweeps the whole tracked tree minus the R2 exempt set (canonical definition); the enumerated surfaces (`docs/`, `README.md`, `SPEC.md`, `skills/`, `.claude/`, `cafleet/src/`, `cafleet/tests/`, `admin/src/`, root `CLAUDE.md`) are illustrative of the major surfaces, not an exhaustive allow-list.
- [ ] **R4 encoded** — the skill runs a structured multi-pass sweep plus hand-inspection of every hit, fanned out across scanner members; never a shallow single grep.
- [ ] The fixed classification rubric (sentinel-test / narration-citation / keep, plus the known-benign class) is specified in `reference/rubric.md` and both role files reference it.
- [ ] The skill's per-run output is a file→action inventory (grouped tables) + an explicit KEEP list + a "Known-benign sweep matches" subsection, written under the run's task-scoped `${BASE}`.
- [ ] The worked-example section classifies the current-tree residue correctly against the rubric (proves the rubric on real hits).
- [ ] Dogfood: invoking the skill (or applying its first-run inventory) against the current tree leaves `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` green, and a re-sweep returns zero unaccounted matches (every remaining hit KEEP-listed or exempt). No live test coverage is lost.

---

## Background

`~/.claude/rules/removal.md` and `~/.claude/rules/affirmative-writing.md` require that source, user-facing docs, skills, and tests describe **only the current state**: the git history and the design doc are the historical record, and every artifact should read as a clean present-tense specification of intended behavior with no reactive residue. Design 0000072 (`purge-historical-narration`) applied this once as a one-off inventory. Residue recurs — every subsequent design-doc cycle can reintroduce a "this replaces X" note, a `design 0000NNN` provenance citation, or a removal-sentinel test — so a one-off sweep decays. This design promotes the 0000072 procedure into a **repeatable skill** that can be run on demand to keep the repository clean continuously.

**Why CAFleet-orchestrated (not a single subagent).** R4 demands a thorough, multi-pass read with hand-inspection of *every* hit across the whole repo — work that fans out naturally across scanner members, each owning a disjoint surface slice. The classification also needs role specialization: a scanner that finds-and-classifies is a different posture from a reviewer whose sole job is guarding against over-deletion and lost coverage. That fan-out + adversarial-review shape is exactly what the CAFleet team pattern provides and a single subagent does not.

**Precedent spine.** The skill's per-run output mirrors `design-docs/0000072-purge-historical-narration/design-doc.md`: a grouped file→action inventory plus an explicit KEEP list. The worked example in this doc reuses that spine on the current tree.

---

## Specification

### 1. Skill identity and location

| Property | Value |
|---|---|
| Slug | `historical-residue-cleanup` |
| Location | `.claude/skills/historical-residue-cleanup/` (project-local skills dir) |
| Invocation | `/historical-residue-cleanup` (optional arg: a surface subset to restrict the sweep; default = whole repo) |
| Orchestration | CAFleet team — Director + monitor (first-in) + N scanners + 1 reviewer |
| Backend-neutrality | `SKILL.md` and `roles/*.md` are backend-neutral; backend deltas resolve via `skills/cafleet/reference/coding-agent/<name>.md` (§9) |

### 2. Behavioral contract (the four hard requirements)

Each is encoded **explicitly** as a `SKILL.md` instruction, a rubric rule and/or a `reviewer.md` check, and a Success Criterion above.

| # | Requirement |
|---|---|
| **R1** | **No-add + remove.** The skill removes existing historical narration AND is forbidden from introducing new narration in its own edits. Every edit is a clean present-tense statement of current behavior (affirmative-writing.md). |
| **R2** | **Exempt set (never modified).** `design-docs/`, `researches/`, `cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`, lock files. A migration legitimately references prior/renamed state; the generated `dist/` bundle is not authored prose; the design/research folders are the historical record. |
| **R3** | **In scope (swept every run).** The **whole tracked tree minus the R2 exempt set** (canonical). The major authored surfaces — `docs/`, `README.md`, `SPEC.md`, `skills/`, `.claude/`, `cafleet/src/`, `cafleet/tests/`, `admin/src/`, plus root files like `CLAUDE.md`, `pyproject.toml`, `mise.toml`, `package.json` — are illustrative, not an exhaustive allow-list: any tracked file outside R2 is in scope. |
| **R4** | **Thorough read.** A structured multi-pass sweep (pattern catalog in §7) plus hand-inspection of every hit, fanned out across scanner members. Never a shallow single grep. |

Two invariants ride alongside: **no runtime behavior is removed** (every flag/table/column/code path survives), and **no live test coverage is lost** (only sentinel framing and narration are removed; every live assertion stays).

### 3. Team shape

```
User → /historical-residue-cleanup
 └─ Director (main Claude — resolves BASE, bootstraps fleet, partitions surfaces, merges inventory, arbitrates, verifies, tears down)
     ├─ monitor         (first-in, --role monitor; heartbeat + idle-nudge; gates the first ordinary spawn)
     ├─ scanner-1       (owns a disjoint surface slice: sweep → hand-inspect → classify → apply)
     ├─ scanner-2       (…)
     ├─ scanner-N       (…)
     └─ reviewer        (over-deletion guard: validates the merged inventory + KEEP list before any edit; re-checks after apply that no live coverage or runtime behavior was lost, and that no new narration was introduced)
```

| Role | Responsibility |
|---|---|
| **Director** | Resolve task-scoped `${BASE}`; `cafleet fleet create`; spawn monitor first, gate on `ready: monitor live`; partition R3 surfaces into **disjoint file-ownership** slices (no two scanners edit the same file); spawn scanners + reviewer; merge partial inventories into the run's canonical inventory; route the merged inventory to the reviewer and hold the apply until `approved`; after apply, run verification; tear down monitor-first. |
| **monitor** | The mandatory dedicated monitoring member (canonical conditional idle-nudge). Owns the heartbeat; the Director never runs `cafleet monitor start`. |
| **scanner** (×N) | For its assigned slice: run the multi-pass sweep, hand-inspect every hit, classify each with the rubric (§4), write a partial file→action inventory. After the reviewer approves the merged inventory, apply its slice's edits (disjoint files → no merge conflicts, no worktree isolation needed) and re-run the sweep on its slice. |
| **reviewer** | Validate the merged inventory against removal.md + affirmative-writing.md **before** any edit: catch mis-classification, over-deletion (a KEEP-item marked for removal), and any planned edit that would lose live coverage or introduce new narration (R1). Sign off with `approved (inventory)`. After apply, confirm the re-sweep is clean and no assertion was lost. |

Disjoint file-ownership is the concurrency contract: the Director partitions by file path so scanners never contend, which is why parallel apply is safe without worktrees. **One scanner owns every edit to a given file, regardless of which rubric class those edits fall under.** The §8 worked example groups edits by rubric class (8a–8f) for readability, but that grouping is orthogonal to ownership: a file touched by several classes (e.g. `test_broadcast_recipients_delivered.py` in B3 + C3 + D1/D2, or `test_alembic_smoke.py` in B4 + C1 + C2) is assigned whole to one scanner — never split across scanners by class.

**Coordination.** The skill adopts the `cafleet` verb+pointer schema (`skills/cafleet-design-doc/reference/coordination.md`) with two skill-local extensions, since a cleanup run produces an `inventory.md`, not a design document:

- **Role taxonomy** gains a `scanner` role: a scanner records each classified hit as a scanner-tagged marker in the run's inventory at the hit's `<file>:<line>` pointer.
- **Pointer** `inventory` denotes the run's `inventory.md` as a whole — the cleanup-run analog of `doc`. The reviewer's sign-off is `approved (inventory)`; per-file routing uses `<file>:<line>` pointers.

### 4. The fixed classification rubric

Specified canonically in `reference/rubric.md`; both role files point to it. Every swept hit lands in exactly one class.

| Class | Definition | Action |
|---|---|---|
| **(a) Sentinel test** | A test whose *only* job is to assert that a removed / never-added flag or key produces an error (e.g. Click "No such option", a dropped wrapper key absent). | **Delete outright.** If a single test *mixes* a sentinel with live coverage, **keep the live assertion and drop only the sentinel framing** — never lose coverage. |
| **(b) Narration / citation / trajectory** | Prose that narrates the past: "previously X, now Y", "no longer / formerly / deprecated", "this replaces X", "backfills each pre-existing session", a design-number-as-reason citation (`design 0000NNN`), a trajectory note ("inverted by 0000092", "after design 0000124"), or a version qualifier ("in v1", "first cut"). | **Reword to pure present-tense current behavior** (describe only what the code does now) / **drop the citation clause**. Keep the behavior description and all coverage. |
| **(c) Keep** | A current-behavior statement, an illustrative example path / fixture slug (not a citation-as-reason), or a forward-looking rationale / editing guard ("do not refactor X into Y", "runtime config has no historical value; drop it"). | **Keep unchanged.** |

**Known-benign class (a KEEP sub-case worth naming).** Present-tense English that trips a broad pattern but is not historical narration — reword nothing:

- `no longer` as *current* behavior ("a canceled message no longer appears", "the loop no longer owns the slot"). Present-tense, not "formerly".
- `used to` = *utilized to* (not *formerly*).
- `preserved for` = *retained for* (current behavior).
- `\bstale\b`/`STALE` naming live monitor-liveness and skill-install-staleness features.
- Issue-provenance citations (`(issue #174 bullet 3)`) — issue provenance is in scope only for design-number citations, not issue numbers; keep them.

### 5. The skill's per-run process

The `SKILL.md` procedure, wiring the five skill-author sub-systems (resolve BASE → bootstrap fleet → spawn monitor first → spawn members → tear down monitor-first):

1. **Resolve `${BASE}`** — task-scoped, per the `cafleet` skill's `reference/base-dir.md`. Task convention: `researches/historical-residue-<UTC-compact>/` (analysis-shaped, one folder per run; `researches/` is gitignored, so run artifacts stay out of version control automatically).
2. **Bootstrap** — `cafleet fleet create`; spawn the monitor first (`--role monitor --model {monitor_model}`), gate on `ready: monitor live`.
3. **Partition** — split the R3 surfaces into disjoint file-ownership slices sized to the scanner count (a scanner per top-level surface group, or finer for a large tree).
4. **Sweep + classify (fan-out)** — each scanner runs the multi-pass pattern catalog (§7) over its slice, hand-inspects every hit, classifies with the rubric (§4), and writes its partial inventory as scanner-tagged markers (the `scanner` role from §3 Coordination) or a partial table under `${BASE}`.
5. **Merge + review (gate)** — the Director merges partials into the run's canonical inventory (grouped tables + KEEP list + known-benign subsection). The reviewer validates it and replies `approved (inventory)`. **No edit happens before approval.**
6. **Apply** — each scanner applies its own slice's edits; `design-docs/` and the rest of the R2 set are never touched; every live assertion is preserved.
7. **Verify** — `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` green; re-run the sweep across tracked files outside the exempt set → zero unaccounted matches (every remaining hit KEEP-listed or exempt). The reviewer confirms no coverage or runtime behavior was lost and no new narration was introduced (R1).
8. **Report + teardown** — the Director reports the run summary and tears down monitor-first (stop the monitor's `monitor start` task and delete the monitoring member first, then ordinary members, then `fleet delete`).

### 6. Per-run output artifact

The skill produces, under `${BASE}`, an `inventory.md` with the 0000072 spine:

- Grouped **file→action inventory** tables (by surface / by rubric class), each row = location + quoted text anchor + action.
- An explicit **KEEP list** to prevent over-deletion.
- A **"Known-benign sweep matches"** subsection listing the present-tense false positives.

The inventory is the run's record and audit artifact (ephemeral, under gitignored `researches/`). The **applied cleanup — the git diff plus green verification — is the real deliverable.**

### 7. Sweep pattern catalog

Specified in `reference/patterns.md`. A multi-pass `git grep` over tracked files outside the exempt set (git-tracking naturally excludes untracked generated `webui/dist` output):

| Pattern (regex) | Catches |
|---|---|
| `deprecat` | deprecation notices |
| `no longer\|formerly\|previously\|used to` | past-state narration (hand-filter the known-benign present-tense `no longer` / `used to` cases) |
| `\blegacy\b` | "legacy" framing (hand-filter arbitrary fixture names like `legacy_squat`) |
| `sentinel` | removal-sentinel framing |
| `historical` | "historical rows / narration" |
| `forensic\|preserved for\|for history` | forensic-visibility / for-history pointers |
| `restoration` | restoration-plan pointers |
| `design[ -]?0[0-9]{6}` and `\b0[0-9]{6}\b` | design-number-as-reason citations. The `0[0-9]{6}` form spans the full 7-digit id space (so ids past `0000999` are not silently skipped); the `\b…\b` word-boundary guard keeps all-zero UUID constants (`00000000-0000-…`) from false-matching. Illustrative example slugs are KEEP-listed. |
| `in v1\|first cut` | version qualifiers |
| `backfill\|pre-existing` | migration-backfill narration in prose (keep migration *filenames*) |

The catalog is a floor, not a ceiling: the scanner hand-inspects surrounding context and may classify a hit no pattern named.

### 8. Worked example — current-tree residue (proves the rubric)

This is the skill's first-run inventory: the Director's 14-site scan plus the design-number test-citation family. It demonstrates the rubric classifying real current-HEAD residue. Line numbers are drift-prone anchors — **match on the quoted text**. The 8a–8f grouping below is by rubric class for readability; ownership is by file (§3) — a file appearing under multiple classes (e.g. `test_broadcast_recipients_delivered.py` in B3 + C3 + D1/D2) is assigned whole to one scanner.

#### 8a. Skill / doc prose — reword (rubric b)

| # | Location | Anchor | Action |
|---|---|---|---|
| A1 | `skills/cafleet/reference/supervision.md` ~L54 | `This corrects the old "do not nudge a member just because it went idle" reading.` | Remove the trailing sentence. Keep the affirmative rule and the `(issue #174 bullet 3)` provenance. |
| A2 | `skills/cafleet/reference/recovery.md` ~L47 | `(the Director no longer runs it)` | Drop the parenthetical → "The `cafleet monitor` loop runs as a background task in the monitoring member's pane, and there is no `cafleet monitor stop`." |

#### 8b. Tests — pure removal-sentinels, delete outright (rubric a)

| # | Location | Function | Action |
|---|---|---|---|
| B1 | `cafleet/tests/cli/test_message.py` ~L213-233 | `test_message_poll__removed_flags_rejected` | Delete (asserts removed `--since`/`--page-size` error; Click rejects unknown options already). |
| B2 | `cafleet/tests/cli/test_message.py` ~L236-254 | `test_message_task_id_flag_rejected` | Delete (pre-rename `--task-id` sentinel). |
| B3 | `cafleet/tests/broker/test_broadcast_recipients_delivered.py` ~L33-40 | `test_broadcast_result_drops_legacy_notifications_sent_count_key` | Delete. Live coverage lives in the preceding `test_broadcast_result_carries_separate_recipients_and_delivered` — untouched, no coverage lost. |
| B4 | `cafleet/tests/db/test_alembic_smoke.py` ~L48-49 | (in `test_alembic_upgrade_head_creates_expected_tables`) | Delete the two negatives `assert "api_keys" not in tables` / `assert "tasks" not in tables`. **Keep** the positive `missing = expected - tables; assert not missing`. Optional hardening: tighten to `assert tables == expected` **only if** the full suite stays green. Most delicate item — verify. |

#### 8c. Tests + docs — legacy/removed-state narration, reword keep coverage (rubric b)

| # | Location | Action |
|---|---|---|
| C1 | `cafleet/tests/db/test_alembic_smoke.py` ~L255 | `# Must be nullable because unicast + historical rows store NULL` → drop "historical rows" (e.g. "# Nullable: a unicast message references no origin, so it stores NULL"). |
| C2 | `cafleet/tests/db/test_alembic_smoke.py` ~L262-263 | `test_messages_to_member_id_is_nullable_after_migration` docstring: drop "instead of the ``0`` sentinel"; keep "nullable so broadcast-summary rows persist NULL". |
| C3 | `cafleet/tests/broker/test_broadcast_recipients_delivered.py` ~L6 | Remove module-docstring sentence "The legacy single ``notifications_sent_count`` key is gone." Keep the affirmative two-counts description. |
| C4 | `cafleet/tests/cli/test_broadcast_recipients_delivered.py` ~L5 | Drop "rather than the single legacy count mislabeled as ``recipients``." Keep the affirmative both-counts sentence. |
| C5 | `cafleet/tests/broker/test_to_member_id_nullable.py` ~L4-5 | Drop "rather than the legacy ``0`` sentinel". Keep the affirmative NULL/None description. |

#### 8d. Design-number-as-reason citations — drop citation, keep behavior (rubric b)

The Director's scan named the three `design 0000118` provenance citations in the broadcast/nullable test files (D1–D3 below). The broader sweep finds the same class across **20 test files, ~28 citation lines** — module docstrings and section-anchor comments that pair a real behavior description with a `(design 0000NNN …)` clause. Every one: **drop the citation clause, keep the behavior description, never delete a test.** Representative:

| # | Location | Action |
|---|---|---|
| D1 | `cafleet/tests/broker/test_broadcast_recipients_delivered.py` L1 | "Regression tests for split broadcast counts (design 0000118, item 2.1)." → drop the parenthetical. |
| D2 | `cafleet/tests/cli/test_broadcast_recipients_delivered.py` L1 | "…prints ``recipients=N delivered=k`` (design 0000118, item 2.1)." → drop the citation. |
| D3 | `cafleet/tests/broker/test_to_member_id_nullable.py` L1 | "Regression tests for nullable ``messages.to_member_id`` (design 0000118, items 1.1/1.2)." → drop the citation. |
| D4 | `cafleet/tests/cli/test_fleet.py:256` | `# --- fleet show / delete take --fleet-id, not a positional (design 0000118, item 3.1) ---` → drop the citation, keep the anchor. |
| D5 | `cafleet/tests/cli/test_member.py` (L3, L189, L606, L728) | Drop `(design 0000112 §3)` / `(design 0000112 §1)` / `(design 0000090 §5)` / `(design 0000101)` clauses; keep each behavior comment. |
| D6 (trajectory) | `cafleet/tests/multiplexer/test_tmux.py:637,746` | `# --- Esc keystroke safeguard (design 0000090 §1, §2; inverted by 0000092) ---` and `after design 0000092 §2, emits NO leading Escape:` → reword to **pure present-tense current behavior** ("Esc keystroke safeguard: …" / "emits no leading Escape: …"); drop "inverted by"/"after design NNNN". |
| D7 (trajectory) | `cafleet/tests/multiplexer/test_tmux_send_inline_preview.py:64` | `# After design 0000092 §1 the inline preview leads with ``Escape`` …` → "The inline preview leads with ``Escape`` …". |
| D8 (trajectory) | `cafleet/tests/monitor/test_loop.py:434,443` | `# After design 0000124: _WAKE_ON_STATUS = ("done",) …` → present-tense "``_WAKE_ON_STATUS = ("done",)``: a transition into … is a wake trigger."; drop "After design 0000124". |

Remaining files in the family (drop citation, keep behavior): `test_guard_exit_codes.py`, `test_inline_preview.py`, `test_broadcast_json_to_member_id.py`, `test_member_create_director_autodiscovery.py`, `test_member_ping.py`, `test_member_show.py`, `test_message_text_input.py`, `test_text_input.py`, `test_unhidden_flags.py`, `test_format_member_detail.py`, `test_ping_age_ascii.py`, `test_to_member_id_render.py`.

#### 8e. Explicit KEEP list (do **not** touch — prevents over-deletion)

- `skills/cafleet/reference/supervision.md` ~L46 `(the Director never runs it — see § Spawn Protocol)` — affirmative present-tense RULE, not "no longer". Keep.
- The `(issue #174 bullet N)` provenance citations in supervision.md — issue provenance, not a design-number citation. Keep.
- `cafleet/src/cafleet/db/alembic/versions/0001_initial_schema.py` — the single initial migration legitimately references prior/renamed state. Exempt (R2).
- `cafleet/tests/db/test_init.py` `test_setup_db_errors_on_unversioned_db` / `test_setup_db_ahead_errors` — exercise the CURRENT db-init guards; `legacy_squat` is an arbitrary fixture table name. Keep.
- `cafleet/src/cafleet/broker/members.py` ~L251 `# Runtime config has no historical value; drop it…` — forward-looking rationale, not a legacy guard. Keep.
- `monitor/` and `cli/` `stale`/`STALE` — live monitor-liveness and skill-install-staleness features. Keep.
- `SPEC.md` "status is the renamed status_state; body the renamed text" — live DB-column→API-field projection (current-state mapping). Keep.
- Illustrative design-number **example paths / fixture slugs** — KEEP-listed (not citations-as-reason).

#### 8f. Known-benign sweep matches (present-tense false positives — reword nothing)

- `cafleet/src/cafleet/broker/messaging.py:267` "a canceled message **no longer appears**" — current behavior.
- `cafleet/src/cafleet/monitor/loop.py:83` "when this process **no longer owns** the slot" — current behavior.
- `skills/cafleet/reference/recovery.md:30` "**no longer** attached to a supported multiplexer" — current behavior.
- Occurrences of `no longer` / `previously` in `.claude/rules/*` describing the user's own environment or a conditional, not project history.

The scanner confirms each remaining sweep hit falls in the benign / KEEP / exempt class before declaring the sweep clean.

### 9. Skill file layout

```
.claude/skills/historical-residue-cleanup/
  SKILL.md              # dispatch + the CAFleet orchestration procedure (§5), backend-neutral
  roles/
    scanner.md          # sweep → hand-inspect → classify (§4) → apply own slice
    reviewer.md         # over-deletion / lost-coverage / new-narration (R1) guard
  reference/
    rubric.md           # the fixed classification rubric (§4) — canonical
    patterns.md         # the sweep pattern catalog (§7) — canonical
```

The monitoring member reuses the `cafleet` skill's canonical `roles/monitor.md` (no monitor role file in this skill). `SKILL.md` and `roles/*.md` are backend-neutral: they use `{monitor_model}` / `{skill_loader}` / `{decision_surface}` tokens and carry a one-line pointer to `skills/cafleet/reference/coding-agent/<name>.md`; the spawn-prompt identity block includes a `CODING AGENT: {coding_agent}` line so each member resolves its overlay (skill-author §8). Role files are referenced by absolute path in spawn prompts (never inlined); spawns use `--text-file`; spawn-prompt audit renders land at the dot-prefixed `${BASE}/.prompts/<role>-<UTC-compact>.md` (agent-only scratch, per base-dir.md § "Hidden agent-only folders vs visible deliverables").

### Out of scope

- Any runtime behavior change — flags, columns, code paths, and API/CLI surfaces are unchanged by every run.
- The R2 exempt set (§2) — never modified.
- General doc-drift that is not historical narration (e.g. an enum docstring under-counting a live choice set) — an accuracy bug for a separate change, not this skill's job.

---

## Implementation

> Documentation-and-skill-first (project rule): the skill files ARE the deliverable, authored before the dogfood run.
> Task format: `- [x] Done task <!-- completed: 2026-07-12T14:30 -->`

### Step 1: Skill scaffold — `SKILL.md`

- [x] Create `.claude/skills/historical-residue-cleanup/SKILL.md` with the trigger `description` (Success Criterion 1), the R1–R4 behavioral contract (§2), the team shape (§3), and the per-run process (§5) wiring the five skill-author sub-systems (BASE resolve → fleet bootstrap → monitor-first → member spawn → monitor-first teardown). Backend-neutral with the overlay pointer + `CODING AGENT:` line (§9). <!-- completed: 2026-07-12T01:52 -->
- [x] State the two invariants explicitly in `SKILL.md`: no runtime behavior removed; no live coverage lost. <!-- completed: 2026-07-12T01:52 -->

### Step 2: Role files

- [x] Create `roles/scanner.md` — required-reading block (overlay row #1, base-dir, coordination); accountability (sweep own slice with §7 patterns, hand-inspect every hit, classify with §4, write partial inventory, apply own slice after `approved`); disjoint file-ownership contract. <!-- completed: 2026-07-12T01:52 -->
- [x] Create `roles/reviewer.md` — accountability as the over-deletion / lost-coverage / new-narration (R1) guard; validates the merged inventory before apply, re-checks after apply; signs off with `approved (inventory)`. <!-- completed: 2026-07-12T01:52 -->

### Step 3: Reference pages

- [x] Create `reference/rubric.md` — the fixed classification rubric (§4) incl. the known-benign class, as the canonical source both role files cite. <!-- completed: 2026-07-12T01:52 -->
- [x] Create `reference/patterns.md` — the multi-pass sweep pattern catalog (§7) with the exempt-set exclusion and the word-boundary design-number form. <!-- completed: 2026-07-12T01:52 -->

### Step 4: Backend-neutrality wiring

- [x] Verify `SKILL.md` + `roles/*.md` use only neutral `{…}` tokens for backend-varying behavior and carry the `skills/cafleet/reference/coding-agent/<name>.md` pointer; the spawn-prompt skeleton includes the `CODING AGENT: {coding_agent}` line and doubles every literal brace. <!-- completed: 2026-07-12T01:52 -->

### Step 5: Load + trigger check

- [ ] Confirm the skill loads via the Skill tool and its `description` triggers on the intended phrases (Success Criterion 1); no `{token}` leaks in any user-facing string. <!-- completed: -->

### Step 6: Dogfood — first run against the current tree

- [ ] Apply the §8 worked-example inventory to the current tree (rubric a: delete B1–B4 sentinels preserving live coverage; rubric b: reword A1–A2, C1–C5, D1–D8 and the remaining citation-family files; honor the §8e KEEP list and §8f known-benign matches). This validates the rubric, the cleanup edits, and the verification loop, and completes the original cleanup intent; the skill's full orchestration path (fleet bootstrap, scanner fan-out, merge, review-gate) is exercised on subsequent live `/historical-residue-cleanup` runs. <!-- completed: -->
- [ ] Verify: `mise //cafleet:test` (special attention to B4 and B3), `mise //cafleet:lint`, `mise //cafleet:typecheck` all green. <!-- completed: -->
- [ ] Re-run the §7 sweep over tracked files outside the exempt set → zero unaccounted matches (every remaining hit KEEP-listed / benign / exempt); confirm no live assertion or runtime behavior was lost and no new narration was introduced (R1). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-12 | Initial draft — specifies the reusable CAFleet-orchestrated cleanup skill; the current-tree residue (Director scan + citation family) is the §8 worked example and Step-6 dogfood target |
| 2026-07-12 | Reviewer pass: fixed `webui/assets`→`webui/dist` exempt path and audit-render `prompts/`→`.prompts/`; pinned R3 as "whole tracked tree minus R2" (enumerated list illustrative); adopted the cafleet coordination schema with a `scanner` role + `inventory` pointer (`approved (inventory)`); stated one-scanner-owns-a-whole-file; widened the design-number pattern to `0[0-9]{6}`; removed the bare in-prose scanner-marker token; softened the Step-6 dogfood claim |
