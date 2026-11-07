# Documentation Size Reduction

**Status**: Approved
**Progress**: 45/47 tasks complete
**Last Updated**: 2026-11-07

## Overview

Reduce the byte count of `SPEC.md`, the `docs/` site, `README.md` and the `skills/` tree, and reduce the context every agent loads at startup. Do it through restructuring, simpler phrasing and a single owner for each fact, without losing meaning. The audit in this folder is the work list, and this document fixes the ownership rules, scope decisions and verification that turn it into one PR, which also restores CI-safe wait budgets in two timing-flaky observation tests.

## Success Criteria

- [x] Every finding in § Work sources has a ledger row (§ Ledger) with status `applied`, `kept` or `superseded`. Every `kept` row cites one of the allowed keep reasons.
- [ ] The PR body has a before/after `wc -c` table for every in-scope file and every always-read set (§ Metrics). Each home's percentage is shown against the soft target of ≥12%.
- [x] The Director always-read set shrinks by at least the bytes of the model-selection procedures moved out of `director.md`. A default spawn (fleet backend, Role default, no trigger in § Model-selection relocation) reads none of the moved text.
- [ ] The Director's per-home meaning review passes against that home's ledger before each home commit. The fresh Reviewer approves the full branch.
- [x] Every item that disappears in the fact-inventory diff (§ Verification) is accounted for.
- [x] Every defect in § Defect fixes is corrected.
- [x] The rule edits in § Rule edits are in `.claude/rules/documentation-maintenance.md` and `.claude/skills/update-readme/SKILL.md`.
- [x] The link resolver reports no unresolved link that was resolved at baseline. `mise //docs:build`, `mise //:test` and `mise //:lint` pass.
- [x] Every `SKILL.md` frontmatter block and every `cafleet-model-list-refresh` dependency is byte-identical to the baseline.
- [ ] The two observation tests in § CI wait budgets assert the same behavior with CI-safe budgets and pass repeated runs under parallel load. The PR's CI `test` job passes.

---

## Background

Fourteen auditors measured the corpus at `f7b1db3d` (branch `docs/quickstart-slim-hermes-tui`, PR #572). Within the current rules they estimate ≈116 KB (≈14%) of ≈849 KB is removable. That splits into ≈5–7% of safe phrasing and restructuring and ≈7% of removing restatements whose fact survives in an owner. Rule changes unlock several KB more. Their reports in `audit/` (local working input, not committed) hold the finding tables, the measured sample rewrites and the consumer inventory. `audit/SUMMARY.md` indexes them.

`SPEC.md` and `skills/` on `main` (`fb313842`) match the audited text byte for byte. PR #572, which changed only `docs/docs/quickstart.md` and `docs/docs/concepts/coding-agents.md`, is now merged. Line numbers in the audit refer to `f7b1db3d`. Locate every finding by its heading text and treat line numbers only as hints.

PR #572 touched no Rust, yet its CI `test` job (run 42355541014) failed: 1525 passed, 2 failed. Both failures panicked at `src/runtime/writer.rs:2797` with `bounded worker condition did not complete`, the assertion in `test_support::wait_until_within`. That makes them timing failures on CI runners. Commit `2221223e` (`test: restore CI-safe wait budgets in the exporter encode-stall test`) fixed the same class in `src/runtime/observation.rs` by raising each wait budget tenfold (300 ms → 3 s, 500 ms → 5 s, 200 ms → 2 s). The CI log has no backtrace, so it does not name the exceeded call site.

---

## Specification

### Scope

| Path | Treatment |
|---|---|
| `SPEC.md` | Reduce |
| `docs/docs/**/*.md` | Reduce |
| `README.md` | Reduce |
| `skills/**` (both skill families) | Reduce; add `skills/cafleet/reference/model-selection.md` |
| `.claude/rules/documentation-maintenance.md` | Only the edits in § Rule edits |
| `.claude/skills/update-readme/SKILL.md` | Only the one-sentence edit in § Rule edits, which enables deleting the empty SPEC sections |
| `src/runtime/observation.rs` `mod tests` (`controller_tests`, `injected_failures` and the shared `quiet` helper) | Wait budgets only (§ CI wait budgets) |
| `src/runtime/service.rs` and `src/runtime/writer.rs` `test_support` | Wait budgets only, and only when an exceeded wait is inside one of their helpers |
| Other `.claude/skills/` and `.claude/rules/` files, `CLAUDE.md`, other `src/` files, `docs/` tooling | Unchanged |

The only Rust edits are the test wait budgets in § CI wait budgets. Production code, test assertions and docs-site configuration stay unchanged. `build.rs` → `src/assets.rs` embeds skills byte for byte and embeds every non-hidden file under `skills/`, so the edits and the new reference page ship in the next binary without code changes.

### Metrics

1. **Corpus bytes.** This is `wc -c` per in-scope file, summed per home (SPEC; docs + README; skills). The corpus is English, so bytes ≈ characters. Moving text between files saves nothing.
2. **Always-read bytes.** This is the `wc -c` sum of the files a reader loads before its first assignment.

| Reader | Always-read set | Baseline bytes at `fb313842` |
|---|---|---|
| Every CAFleet member | `skills/cafleet/SKILL.md`, `roles/member.md`, `reference/coding-agents.md` | 68,199 |
| Director | `skills/cafleet/SKILL.md`, `roles/director.md`, `reference/coding-agents.md` | 118,189 |
| Design-doc member, in addition | `cafleet-design-doc/SKILL.md`, `reference/guidelines.md`, `reference/coordination.md`, its role file | Measured per role in Step 1 |
| Design-doc Director, in addition | `cafleet-design-doc/SKILL.md`, `create/create.md` or `execute/execute.md` | Measured in Step 1 |

Step 1 re-measures every baseline at the branch point, and the byte tables use those figures. The soft target is ≥12% corpus reduction per home. It is recorded and does not gate the change.

### Invariants

When a rewrite breaks an invariant, revert it and record it as `kept` with reason `breaks-invariant`.

| Invariant | Rule |
|---|---|
| Meaning | Every contract fact, constraint, exact string, identifier, exit code, value, condition and instruction stays, in the rewritten passage or in a named owner (§ Ownership), with nothing weakened or broadened. |
| Order | Numbered procedure steps, first-match and precedence rule lists, validation and error-check order, ordered key and column lists, and every sequence the text states as ordered keep their order. Other content may move, merge or be regrouped. |
| Verbatim strings | Error messages, output lines, JSON keys, regexes, rule/plan/fixture ids, argv and placeholder spellings stay byte-identical. |
| Frontmatter | Every `SKILL.md` frontmatter block stays byte-identical; `description` is trigger text. |
| Inbound targets | Every heading with an inbound link or `§` reference keeps its exact text. SPEC `§` numbering is unchanged except for deleting §10/§11. A tiny heading-only subsection with no inbound target may be folded into its parent. |
| Model-list refresh | Catalog headings, `#### OpenCode Go`, column headers, Role defaults tables and the `last refreshed: YYYY-MM-DD` sentence stay. |
| Spawn prompts | Installed skill paths (`skills/cafleet/SKILL.md`, `roles/member.md`, `reference/coding-agents.md`) and CLI placeholders stay. |
| Overlay structure | Each backend section of `coding-agents.md` stays self-contained with its five subsections. Reception steps and catalog prefixes stay repeated per backend. |
| Table conventions | A table has ≥2 data rows, a cell holds ≤2 sentences, an empty cell is `—`, and a literal pipe uses the `<code>` entity form. Overlay Note cells over the cap are fixed during the pass. |
| Audience homes | Skills never link `docs/` or `SPEC.md`. `SPEC.md` never links `docs/`. |
| Reflow | A hard-wrapped paragraph becomes one line only when that paragraph is rewritten anyway. |

### Ownership

Each row names the owner of a fact. Every other place keeps only its own interaction with the fact plus a pointer: a `§` reference inside SPEC, a relative link between skill files, or a SPEC link from docs.

| Fact class | Owner | Non-owner treatment |
|---|---|---|
| CLI surface: synopsis, flags, validation order, output fields | SPEC §6.3 CLI | Module, runtime and multiplexer sections keep their behavior plus `§6.3 <heading>` |
| Broker, multiplexer, runtime and backend behavior | The module section (§6.1–§6.9) | §6.3 keeps the command-level outcome plus a `§` pointer |
| Verbatim error strings | SPEC §6.3 `#### Error Messages` table; backend contract errors in §6.7 `#### Contract error strings` | Action prose cites the row |
| Table columns, enums, nullability, CHECKs, indexes | SPEC §8 exact DDL, which stays in §8 | §6.9 states runtime semantics only: identity, retention, transactional checks, privacy, claim release, audit immutability |
| Cross-cutting policy | SPEC §6 owner | §7.1, §7.3 and §7.7 become `§` pointers; §7.2 and §7.4–§7.6 keep their unique policy |
| Role requirement criteria, probes, negative evidence, extension-goal bar, eligibility evidence | SPEC §6.7 Role requirements / Role eligibility | The skill overlay carries runtime-relevant requirement facts and a `Role \| Eligible \| Basis` table (§ Overlay Role sections) |
| Director-verdict change procedure | `.claude/rules/documentation-maintenance.md` § Director verdict changes; `docs/docs/contributing.md` contributor steps | SPEC §6.7 Role eligibility drops its last paragraph |
| Recommended user permission profiles | `docs/docs/concepts/coding-agents.md`, `docs/docs/quickstart.md` | SPEC §6.7 Claude permission posture keeps the contract facts: members receive `Bash(cafleet message *)` at spawn, ask outranks allow, the guard refusal, and the `Read` entry relocating with `CLAUDE_CONFIG_DIR` |
| Command facts an agent acts on: invocations, flags, outputs it reads, errors it must recognize, next commands | `skills/cafleet/reference/cli.md` | Role files carry the procedure and link `cli.md` for command behavior |
| Implementation timing, concurrency and internal mechanics | SPEC | `cli.md` drops them (skill-cli F21, cross X10) |
| Director procedure (numbered steps) | `skills/cafleet/roles/director.md` | Docs state user-visible behavior and user decisions plus a SPEC link (cross X5, X12; docs F22–F24) |
| Director and consumer instructions | Skills | SPEC drops usage instructions a skill carries (spec-a2 consumer rows, §6.7 Host evidence Director instructions, cross X11) and keeps the observed host facts |
| Model-selection policy | `director.md` § Model selection | — |
| Model-resolution procedures | `skills/cafleet/reference/model-selection.md` | `director.md` links it under the read triggers |

### Deletion classes in SPEC

| Class | Decision |
|---|---|
| A worked example fully derivable from a stated template (spec-a1 F04/F15/guard example; spec-a2 example rows) | Delete |
| A rationale clause (`because …`, `so …`) | Keep it when it prevents a misreading of the contract; delete pure rationale |
| A consumer or usage instruction a skill already carries | Delete |
| A §7 subsection restating a §6 owner | Replace with a `§` pointer |
| Mechanism detail that is not externally observable (spec-a2 §6 row 2) | Keep in SPEC |

### Rewrite patterns

The § Recurring patterns tables in the audit reports apply in every home. These patterns yield the most.

| Pattern | Rewrite |
|---|---|
| A fact restated next to a pointer to its owner | Keep the pointer |
| Prose restating an adjacent table, or a summary repeating its rows | Keep the table plus the prose facts it lacks |
| Parallel tables sharing a key column, or rows with identical cells | Merge them, or lift the shared value into the lead sentence |
| The same qualifier repeated throughout a subsection | State it once per subsection |
| A copied member list where a property selects the members | Write the property (host § Describing Groups) |
| A pointer-only subsection (`X is specified in §Y`) | Fold it into one sentence of the parent when the heading has no inbound target |
| A one-line exact string in its own fenced block | Make it an inline code span |
| The same SPEC anchor linked twice in one docs section | Link it once |
| Spawn-delta tables repeating cells across role columns (`create.md`, `execute.md`) | Hoist the shared values into the lead sentence |
| `via cafleet message send` after a verb the protocol already routes | Drop it |
| A prohibition-only bullet | Replace it with the affirmative rule it protects |
| A wordy connective (`only together with`, `in the shape of`, `rather than`) | Use the shorter form |
| `:--` / `:-:` alignment colons in skill tables (skill-rest §6 row 4) | Use default left alignment `---` |

Converting a short prose list of three or four items into a table is roughly byte-neutral. Do it only when the tabulate rule requires it.

### Work sources

Apply each report's Findings table, classifying every row by the Specification decisions.

| Home | Reports | Cross-document findings |
|---|---|---|
| SPEC | `spec-a1.md` … `spec-a8.md` | X3, X9, X11, X13, X15 |
| docs + README | `docs.md` | X4, X5, X6, X7, X8, X12, X14 |
| skills | `skill-cli.md`, `skill-overlay.md`, `skill-director.md`, `skill-rest.md` | X1, X2, X10 |

**Precedence.** A cross-document finding Xn overrides slice findings for the passage it targets. Slice rows inside that passage are recorded `superseded by Xn`, and slice rows outside it still apply. Known overlaps:

- X1 / X2 with skill-overlay F02–F05 (Role requirements / Role eligibility)
- X10 with the skill-cli `setup` / `doctor` rows
- X11 with the spec-a5 Host evidence rows
- X13 with the spec-a1 §7 rows
- X5, X7 and X8 with the docs.md rows for the same sections

A rewrite pattern applied inside an Xn passage after its target shape is in place is recorded under Xn.

**Added findings.** An opportunity the audit missed is applied when it follows a § Rewrite patterns row and the invariants. It gets a ledger row with id `new-<home>-<n>`.

These candidates are excluded and recorded `kept` with reason `excluded`:

| Candidate | Reason |
|---|---|
| spec-a1 DDL → pointer to `V1__baseline.sql` | §8 stays the exact-DDL owner |
| spec-a2 §6 row 1, spec-a5 R3, skill-cli §6 row 1, cross RC3 (`cli.md` links or generation from SPEC) | Installed skills cannot reach SPEC |
| spec-a5 R1 (SPEC eligibility → skill pointer) | SPEC owns evidence |
| skill-overlay F15, F16, F17, F30, F31, Class cross-reference | Overlay self-containment and the `cafleet-model-list-refresh` formats stay |
| docs R1 (short SPEC link tooling), R3 (README list → single link), R4 (profile base + delta) | Not approved |
| skill-rest §6 rows 1–3, skill-director §6 row 2 (inherit overlay-first Required reading; move the shared spawn line into `clean-docs`) | Every reader entry point keeps its overlay-first gate; `clean-docs` is out of scope |
| skill-director §6 row 3, spec-a6 and spec-a7 cell-cap rows | The two-sentence cap stays |
| skill-rest A1/C1 (frontmatter) | Trigger text |
| Splitting `coding-agents.md` per backend | It breaks the single canonical overlay, its SPEC, rule and refresh-skill links, and the five-subsection template |

### Ledger

The Programmer keeps one ledger per home in this folder: `.review/spec.md`, `.review/docs.md` (docs + README), `.review/skills.md` and `.review/rules.md`. They are hidden and uncommitted. Each row:

```markdown
| Finding | Status | Reason | Meaning-check items confirmed | Removed facts → where they survive |
|---|---|---|---|---|
| spec-a2 F5 | applied | — | per-check states, first-match order, every status string | — (merged in place) |
| spec-a7 F20 | applied | — | column list, enums, nullability | `SPEC.md` § 8. Database schema (DDL `member_runs`) |
| skill-overlay F03 | superseded | superseded by X1 | — | — |
| spec-a3 F17 | kept | owner-lacks-fact | — | §6.9 Native version probe does not state the retry bound |
```

| Column | Content |
|---|---|
| Finding | The audit id (`<report> <Fn>`, `Xn`) or `new-<home>-<n>` |
| Status | `applied`, `kept` or `superseded` |
| Reason | `—` for `applied`. For `superseded`, `superseded by Xn`. For `kept`, exactly one of `owner-lacks-fact`, `breaks-invariant: <invariant>`, `text-changed` (the passage differs from `f7b1db3d`) or `excluded` |
| Meaning-check items confirmed | Every item in the finding's audit Meaning-check column, each confirmed present after the rewrite |
| Removed facts → where they survive | Each removed fact with its surviving file + heading, or `—` |

A `kept` row covers a reverted safe finding as well as a needs-judgment one. The PR body summarizes the counts per status and reason.

### Overlay Role sections

`coding-agents.md` § Role requirements keeps:

- verdict semantics and the two independent Director requirements;
- one requirement × tier table: Listen tiers 1–3 with standing and the tier-3 cost, and Goal tiers 1–2;
- the subscribe-listen delivery behavior;
- receiver recovery for eligible Directors without verified reception;
- "Evidence carries forward until re-check";
- the Member requirements paragraph, byte-identical.

It drops the probe procedures, the probe-invalidation rule, the tier-3 negative-evidence recording rule and the extension-goal item list, all of which SPEC §6.7 states verbatim. The measured target shape is `audit/cross-scratch/s1-after.md` (4,554 → 2,227 bytes).

Each backend's § Role eligibility becomes:

```markdown
| Role | Eligible | Basis |
|---|---|---|
| Director | `yes` | `goal` `native` (0.162.1); `listen` `native` `subscribe` through `codex queue` on the escalated path (0.165.0); the sandboxed path is unverified |
| Member | `yes` | Omitted `--approve-for-me` automatic review; the Codex profile from `0.163.0` |
```

Basis names each requirement's source, delivery and checked native version, plus every runtime caveat an agent acts on, such as `under opencode mini --standalone` or `on the TUI client`. A `no` names each unmet requirement. Every consumer of the old row names changes in the same commit, for example `director.md` line 5, "Director `verdict` row".

### Model-selection relocation

The moved text is `director.md` `### Model-name-to-backend inference` through the end of `#### Model routing failures`, after its own phrasing pass. It goes to `skills/cafleet/reference/model-selection.md`, titled `# Model Selection Procedures`. Each moved heading becomes `##` or `###` and keeps its exact text, so its slug is unchanged.

`director.md` keeps `## Model selection`: the policy, the backend-resolution order, the cost-efficiency trigger and the replacement table. It reads the new page only under these triggers:

| Trigger | Sections the Director needs |
|---|---|
| The operator names a model, alias, family word or backend for a member | Inference, worked resolutions, normalization, explicit mapping, routing failures |
| The originating user request contains `cost efficiency mode` | OpenCode token selection (for whole-catalog comparison), routing failures |
| The selected member backend is `opencode` | OpenCode token selection |
| A Model replacement re-selection (§ Model replacement step 2) | OpenCode token selection when the backend is `opencode`, routing failures |
| A backend/model mismatch or an unsuitable or missing model | Routing failures |

A spawn with none of these triggers — fleet backend, Role default, a non-`opencode` backend — needs only `director.md` § Model selection. The Required-reading sentence in `director.md` ("Before prompt composition, read …") names the new page together with these triggers. The new page's opening sentence repeats them and states that `director.md` § Model selection owns the policy.

| Reference | Rewrite |
|---|---|
| Links in the moved text | Relative to `reference/`: `../roles/director.md#…`, `coding-agents.md#…` |
| `director.md` links to moved anchors | `../reference/model-selection.md#…` |
| Plain-text names crossing the move boundary | Become links. In the moved text, "backend-resolution step 1 / step 3" → `../roles/director.md#model-selection`. In `director.md` § Model replacement step 2, "OpenCode token selection" → `../reference/model-selection.md#opencode-token-selection` |
| Other inbound links | `SKILL.md`, `coding-agents.md`, `.claude/` and SPEC link only `director.md#model-selection`, which stays |

Step 6 greps `skills/` and `.claude/` for each moved heading name: `Model-name-to-backend inference`, `Worked resolutions`, `Model-name normalization`, `Explicit backend model mapping`, `OpenCode token selection`, `Model routing failures`. Every hit outside `model-selection.md` must be a link to the new location.

### Defect fixes

| ID | Location | Fix |
|---|---|---|
| D1 | `cli.md` § Director-only command guard ("Every other command, including `permission inspect` … takes no caller ID"); `member.md` § Identity and authority ("the rest take no caller ID"); `director.md` § Runtime events ("each of them except `permission inspect` takes your literal ID") | The unguarded commands carry no guard refusal, and `permission inspect` still requires the pinned Director's `--from-member-id` (`src/cli/runtime.rs` `PermissionCommand::Inspect`; SPEC §6.3 Permission commands) |
| D2 | `cli.md` § Error messages, pending-placement row | Cover `member press` (`… nothing to press.`) alongside `capture\|prompt\|exec` |
| D3 | `director.md` § Member Create — Scratch and audit files ("four identity placeholders") | Five, including `{database_url}` |
| D4 | `director.md` § Model selection ("stale cost selection" routed to Model routing failures) | Drop the route; the same paragraph already says a stale catalog disables cost efficiency selection |
| D5 | `director.md` § Director wake (`message listen <my-member-id> --timeout 300` example) | Point to the backend's reception recipe |
| D6 | `director.md` § Runtime events ("Select the profile with greatest `min_version` …", "Refuse a known version below the first boundary") | `member create` selects the profile and refuses the version (cli.md § member create); keep the Director's handling of the outcome |
| D7 | `director.md` § Model routing failures ("Opencode catalog row …"); `SPEC.md` §6.7 opencode preset paragraph ("Opencode selects the last matching rule") | `OpenCode` |

The SPEC part of D7 lands in commit 1. Every skills fix lands in commit 4.

### Rule edits

`.claude/rules/documentation-maintenance.md`:

| Location | Edit |
|---|---|
| § Three audiences, SPEC row | `permission posture` → `the permission posture contract (recommended user permission profiles are docs/ content)` |
| § Three audiences, Users row | Add that docs state user-visible behavior and the decisions a user makes, with numbered Director procedures in the skills and mechanism detail in `SPEC.md` |
| § Three audiences, Agents row | Add that `cli.md` carries the command facts an agent acts on (invocations, flags, outputs it reads, errors it must recognize, next commands) and that implementation timing, concurrency and internal mechanics stay in `SPEC.md`. Role files carry procedure and link `cli.md` for command behavior |
| § First-class documentation targets, `SPEC.md` paragraph | `preserve its section structure` → `preserve its section numbering (an empty numbered section is removed without renumbering the others)`. Add the SPEC ownership rule from § Ownership: §6.3 CLI surface, module-section behavior, Error Messages strings, §8 DDL structure with §6.9 semantics, §7 pointers |
| § One owner per enumeration | Add that `SPEC.md` applies the same rule between its sections |
| § Coding-agent overlay item 5 | Role eligibility is a `Role \| Eligible \| Basis` table with a Director row and a Member row. Basis carries each requirement's source, delivery and checked version plus runtime caveats, while the evidence prose lives in `SPEC.md` § Role eligibility |
| § Where backend specifics live, `Model selection belongs to …` | Append `, whose resolution procedures live in skills/cafleet/reference/model-selection.md` |
| § Director verdict changes | Record each requirement row in `SPEC.md` § Role eligibility, then update the backend's skill Role eligibility Basis |
| § Template › Role eligibility | Describe the `Role \| Eligible \| Basis` shape. The establishing link moves from `coding-agents.md#role-requirements` to `SPEC.md` §6.7 Role requirements |

In `.claude/skills/update-readme/SKILL.md`, the SPEC reconciliation sentence changes `Preserve its numbered structure` → `Preserve its section numbering (an empty numbered section is removed without renumbering the others)`.

SPEC §8's last paragraph ("Section 6.9 specifies every runtime table field, enum, nullability constraint …") is rewritten to the new split: the §8 DDL owns structure and §6.9 owns runtime semantics. In `docs/docs/contributing.md` § Extend a coding-agent backend, step 5 copies the rows into SPEC Role eligibility and summarizes each role as `Role | Eligible | Basis` in the skill.

### CI wait budgets

Each failing test reaches `writer::test_support::wait_until_within` through one of these waits. The candidates are the waits each test runs:

| Test (`runtime::observation::tests::…`) | Candidate wait | Helper and current budget |
|---|---|---|
| `controller_tests::controller_exit_closes_the_open_episode_before_recording_the_real_native_signal` | `wait_until(\|\| committed(&fixture) == 1)` after the first `poll` | `writer::test_support::wait_until`, 2 s |
| same | `wait_until(\|\| committed(&fixture) == 3)` after `child_exited` | `writer::test_support::wait_until`, 2 s |
| `injected_failures::runner_resize_detector_failure_keeps_actual_resized_service_and_native_child` | `quiet` → `count(… "interaction_opened") == 1` | `service::test_support::observation_services::wait_until`, 3 s |
| same | `assert_fired` → `observation-fault-fired` exists | `service::test_support::injected_failures`, 3 s |
| same | `assert_live_after_fault` → `assert_durable_gap` → `count(… "runner_degraded") >= 2` | `service::test_support::injected_failures`, 3 s |

The fix:

1. **Identify** the exceeded wait in each test by reproducing it under parallel load with `RUST_BACKTRACE=1`: the scratch loop in § Verification, run while a full `mise //:test` provides load. Record the call site (file:line) per test in `.measure/ci-waits.md`. If no run reproduces the failure, record that, and treat every candidate wait of that test as exceeded.
2. **Budget** each exceeded wait at ten times its current budget, the ratio `2221223e` applied: 2 s → 20 s, and 3 s → 30 s. The budget is only a ceiling, so passing runs take no longer. A wait directly in the test calls `crate::runtime::writer::test_support::wait_until_within(Duration::from_secs(N), …)` at its call site. A wait inside a shared `service.rs` helper (`assert_fired`, `assert_durable_gap`, `observation_services::wait_until` as used by `quiet`) gets the budget in that helper only when the helper's wait is the one exceeded. `writer::test_support::wait_until`'s default stays 2 s unless the fix belongs in the helper itself.
3. **Preserve** behavior. Each condition closure, assertion, fixture and test name is unchanged. Tests stay in their `#[cfg(test)]` modules (`.claude/rules/code-quality.md` § Test placement), and no test file or helper module is added.

### Verification

The helpers below are scratch scripts under this folder's `.tools/`. They are uncommitted and run with `python3 -I`. Their outputs go to `.measure/`.

| Check | Method |
|---|---|
| Meaning | The Director reviews each home diff against its ledger before committing it. Every applied row confirms its Meaning-check items, and every removed fact names a surviving file + heading that states it. A row that fails the review is reverted and becomes `kept` |
| Fact inventory | Per file, before and after, the helper inventories backtick code spans, numeric tokens with their following unit word (`60 s`, `30 seconds`, `two sentences`, `1,000 VM instructions`), `§` references and link targets. An item that disappears from a file must still appear in that file, appear in its named owner, or be listed in a ledger row |
| Links | The resolver builds a heading inventory per Markdown file using GitHub slug rules: lowercase, strip punctuation except `-` and `_`, spaces → `-`, and `-1`/`-2` suffixes for repeated headings (as in `coding-agents.md`'s repeated backend subsections). It honors explicit `{#id}` anchors on docs pages. It resolves every (target file, slug) pair from relative links in `SPEC.md`, `docs/`, `README.md`, `skills/` and `.claude/`, and from absolute `https://github.com/himkt/cafleet/blob/main/<path>#<slug>` links (docs → SPEC, SPEC → skills). It runs at baseline and after each home. A link resolved at baseline must resolve after. Links already unresolved at baseline are reported in the PR as pre-existing |
| `§` references | Every `§` text reference names an existing SPEC heading or number in the final tree |
| CI wait budgets | `mise //:test` and `mise //:lint` pass. A scratch loop script under `.tools/` runs each of the two tests 20 times (`mise //:test <test name>`) while a concurrent full `mise //:test` provides parallel load, and all 40 runs pass. After push, the PR's CI `test` job passes |
| Builds | `mise //docs:build` (dead links), `mise //:test` (embedded-asset install tests), `mise //:lint` |
| Bytes | Re-run the Step 1 measurement and write the before/after tables |

The CLI is not reinstalled during execution, because the executing fleet keeps its installed skills. The operator reinstalls after merge.

### Delivery

The execute workflow runs it with a Programmer and Director review: a Programmer implements, and the Director reviews each home against its ledger and makes every commit. The Programmer also owns the test-budget edit, which changes no asserted behavior and adds no test. The Programmer never commits. The fresh Reviewer reviews the full branch after Step 8's checks, and push and PR creation follow user approval.

The work happens on one branch, `docs/0000219-documentation-size-reduction`, created from an up-to-date `main` after PR #572 merges, with one PR. Commits use single-line conventional messages, one per home, in this order. Each stages only its home's paths, and the Step 3 skill edits stay uncommitted until commit 4.

| # | Message | Paths |
|---|---|---|
| 0 | `test: restore CI-safe wait budgets in observation tests` | `src/runtime/observation.rs`, plus `src/runtime/service.rs` / `src/runtime/writer.rs` when their helpers change |
| 1 | `docs: reduce SPEC.md size without losing meaning` | `SPEC.md` |
| 2 | `docs: reduce docs site size without losing meaning` | `docs/docs/` |
| 3 | `docs: reduce README size` (only when it changes) | `README.md` |
| 4 | `docs: reduce skill size and move model-selection procedures to a reference page` | `skills/` |
| 5 | `docs: record documentation ownership rules for SPEC, docs and skills` | `.claude/rules/documentation-maintenance.md`, `.claude/skills/update-readme/SKILL.md` |
| 6 | `docs: add design doc 0000219-documentation-size-reduction` | `design-docs/0000219-documentation-size-reduction/design-doc.md` |

Commit 0 precedes the documentation commits because the documentation-first rule (`.claude/rules/documentation-maintenance.md` § Implementation Order) has nothing to order here: no SPEC, docs, README, skill or rule text describes these wait budgets, and SPEC §9 is unaffected.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Preconditions and baseline

- [x] Confirm that PR #572 is merged (met on 2026-11-07). The Director creates `docs/0000219-documentation-size-reduction` from an up-to-date `main` <!-- completed: 2026-11-07T13:10 -->
- [x] Diff `SPEC.md`, `docs/docs/` and `skills/` against `f7b1db3d`. For every changed section, re-validate each finding located in it against the new text and record invalidated ones as `kept` / `text-changed`. Re-locate the remaining findings by heading text <!-- completed: 2026-11-07T13:10 -->
- [x] Record the branch-point `wc -c` for every in-scope file, every home and every always-read set in `.measure/baseline.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Write the fact-inventory helper and the link resolver under `.tools/`, record their baseline outputs, and create the four empty ledgers under `.review/` <!-- completed: 2026-11-07T13:10 -->

### Step 2: CI wait budgets

- [x] Write the scratch loop script under `.tools/`, reproduce both tests under parallel load with `RUST_BACKTRACE=1`, and record each exceeded call site (or "not reproduced") in `.measure/ci-waits.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Give each exceeded wait (every candidate wait of a test that did not reproduce) a tenfold budget at its call site or in its owning helper, per § CI wait budgets, with every condition and assertion unchanged <!-- completed: 2026-11-07T13:10 -->
- [x] Run `mise //:test`, `mise //:lint` and the 20-run loop of each test under a concurrent full suite. All runs pass <!-- completed: 2026-11-07T13:10 -->
- [x] Hand the test edit to the Director for review. On approval, the Director makes commit 0 <!-- completed: 2026-11-07T13:10 -->

### Step 3: Defect fixes

- [x] Fix D1 in `cli.md`, `member.md` and `director.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Fix D2 in `cli.md` § Error messages <!-- completed: 2026-11-07T13:10 -->
- [x] Fix D3–D6 in `director.md`, and D7 in `director.md` and `SPEC.md` <!-- completed: 2026-11-07T13:10 -->

### Step 4: SPEC.md

- [x] Apply the spec-a1 findings (§1–§6.3 fleet, §7–§11), including X13 (§7.1/§7.3/§7.7 pointers) and X15 (delete §10 and §11) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a2 findings (doctor, message), including the merged per-check state tables and the removed consumer instructions <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a3 findings (member, runtime, errors), with the Error Messages table owning the strings <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a4 findings (output, multiplexer) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a5 findings (§6.7), including X3 (drop the verdict-change paragraph), X9 (Claude profiles → contract facts) and X11 (Host evidence keeps observed facts) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a6 findings (§6.9 launch policy) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a7 findings (§6.9 runner–storage), removing the §8 DDL restatements and rewriting §8's last paragraph <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the spec-a8 findings (§6.9 input–acceptance), with §6.3 as the CLI-surface owner <!-- completed: 2026-11-07T13:10 -->
- [x] Complete `.review/spec.md`, then run the fact-inventory diff and the link resolver for SPEC <!-- completed: 2026-11-07T13:10 -->
- [x] Hand SPEC to the Director for the meaning review. On approval, the Director makes commit 1 <!-- completed: 2026-11-07T13:10 -->

### Step 5: docs and README

- [x] Apply the `docs.md` findings to `concepts/coding-agents.md`, including X7 (tier table → sentence + link), X8 (Model choice echo) and X14 (Hermes TUI restatement) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `docs.md` findings to `concepts/members.md`, including X5 (runtime-event sections → user-visible summary) and X6 (guard paragraph) <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `docs.md` findings to `contributing.md`, including X4 (probe criteria → SPEC link), and rewrite step 5 for the `Role | Eligible | Basis` skill table <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `docs.md` findings to `mixed-backend-team.md` (including X12), `quickstart.md`, `concepts/overview.md` and `concepts/storage.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Apply any `README.md` finding, keeping the pitch aligned with `docs/docs/index.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Complete `.review/docs.md`, then run `mise //docs:build`, the fact-inventory diff and the link resolver <!-- completed: 2026-11-07T13:10 -->
- [x] Hand docs and README to the Director for the meaning review. On approval, the Director makes commit 2 and, when README changed, commit 3 <!-- completed: 2026-11-07T13:10 -->

### Step 6: skills

- [x] Apply the `skill-cli.md` findings and X10 to `cli.md`, dropping the implementation mechanics SPEC owns <!-- completed: 2026-11-07T13:10 -->
- [x] Rewrite `coding-agents.md` § Role requirements (X1) and each backend's § Role eligibility (X2), and update every consumer of the old row names <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the remaining `skill-overlay.md` findings within the self-containment invariant, including the Note-cell cap fixes <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `skill-director.md` findings to `director.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Move the model-resolution procedures to `skills/cafleet/reference/model-selection.md` with the conditional read triggers, rewrite links and cross-boundary name references, and run the moved-heading-name grep <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `skill-rest.md` findings to `cafleet/SKILL.md` and `roles/member.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `skill-rest.md` findings to the design-doc family: `SKILL.md`, the create/execute bodies, the references and every role file <!-- completed: 2026-11-07T13:10 -->
- [x] Drop alignment colons from every skill table <!-- completed: 2026-11-07T13:10 -->
- [x] Confirm that the frontmatter blocks and the `cafleet-model-list-refresh` dependencies are byte-identical <!-- completed: 2026-11-07T13:10 -->
- [x] Complete `.review/skills.md`, then run `mise //:test`, `mise //:lint`, the fact-inventory diff and the link resolver <!-- completed: 2026-11-07T13:10 -->
- [x] Hand skills to the Director for the meaning review. On approval, the Director makes commit 4 <!-- completed: 2026-11-07T13:10 -->

### Step 7: Rule edits

- [x] Apply the `documentation-maintenance.md` edits in § Rule edits <!-- completed: 2026-11-07T13:10 -->
- [x] Apply the `update-readme` sentence edit and complete `.review/rules.md` <!-- completed: 2026-11-07T13:10 -->
- [x] Hand the rule edits to the Director for review. On approval, the Director makes commit 5 <!-- completed: 2026-11-07T13:10 -->

### Step 8: Final verification and publication

- [x] Re-run the link resolver and the `§` reference check across every home and `.claude/` on the final tree <!-- completed: 2026-11-07T13:10 -->
- [x] Re-run `mise //docs:build`, `mise //:test` and `mise //:lint` on the final tree <!-- completed: 2026-11-07T13:10 -->
- [x] Measure the final bytes, and write the per-file, per-home and always-read before/after tables with the soft-target comparison and the ledger status summary <!-- completed: 2026-11-07T13:10 -->
- [ ] The Director makes commit 6 (design doc). After the fresh Reviewer approves and the user approves publication, the Director pushes and opens the PR with the byte tables and the ledger summary <!-- completed: -->
- [ ] Confirm that the PR's CI `test` job passes. A failure goes back to Step 2 for the failing wait, with the same identify, budget and verify cycle <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-07 | Initial draft |
| 2026-11-07 | Added the CI wait-budget fix for the two timing-flaky observation tests as commit 0 (Step 2), its verification and the CI `test` success criterion; recorded the PR #572 merge |
