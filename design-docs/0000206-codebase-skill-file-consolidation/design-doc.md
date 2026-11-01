# Codebase, skill and repository file consolidation

**Status**: Approved
**Progress**: 0/88 tasks complete
**Last Updated**: 2026-11-01

## Overview

Consolidate the CAFleet repository to fewer, cohesive files and delete dead code and dead documentation: remove the explicitly-unwrapped member transport, the v1 event-schema and v1 preview-digest compatibility paths, and the `cafleet-design-doc` interview workflow; merge skill, source, fixture, rule, task and docs files; untrack non-`design-doc.md` design artifacts. The work lands in ten independently committable phases, each one PR that leaves every validation task green.

## Success Criteria

- [ ] `git ls-files design-docs` lists only `design-docs/*/design-doc.md` paths, and a new non-`design-doc.md` file under `design-docs/<NNNNNNN-slug>/` is ignored by `.gitignore`.
- [ ] `skills/cafleet-design-doc/interview/` does not exist, and both `grep -rniE 'interview|analyzer|question\.md' skills/ docs/docs/ .claude/ presets/ README.md SPEC.md` and `grep -rn 'numbered question list' skills/` return nothing.
- [ ] `skills/cafleet/` contains exactly five files: `SKILL.md`, `roles/director.md`, `roles/member.md`, `reference/cli.md`, `reference/coding-agents.md`; `wc -w` over them totals at most 19,000 words.
- [ ] `grep -rni unwrapped SPEC.md skills/ docs/docs/ presets/ cafleet/src/` returns only the hits listed in § Phase 5 *Retained hits*.
- [ ] `grep -rn payload_digest_version SPEC.md skills/ cafleet/` returns nothing; the runtime event reader accepts only `schema_version` `2`.
- [ ] The native screen fixtures are exactly `cafleet/src/coding_agent/native/{claude,codex,opencode}.json` plus `cafleet/src/coding_agent/bundled.json`, and the bundled catalog loads with every fixture verified.
- [ ] `git ls-files 'cafleet/src/*.rs' | wc -l` is 46 (25 core + 21 runtime), with the module trees of § Phase 8 and § Phase 9, and `cafleet/src/lib.rs` is gone.
- [ ] The repository has one `mise.toml` (root), four `.claude/rules/*.md` files, a two-file `.claude/skills/clean-docs/`, no `CONTRIBUTING.md`, and no `docs/docs/how-to/` directory.
- [ ] Every phase ends with `mise cafleet-test`, `mise cafleet-lint` and `mise docs-build` passing (pre-Phase-3 names: `mise //cafleet:test`, `mise //cafleet:lint`, `mise //docs:build`).
- [ ] The local clutter set in § Phase 10 is deleted from the working tree.

---

## Background

Five read-only audits (2026-11-01) inventoried the repository at `ccf12cab`:

| Area | Finding |
|---|---|
| Interview workflow | `skills/cafleet-design-doc/interview/` (2 files) plus 12 edit sites in the design-doc skill, the cafleet Director role and one docs page. No code, test, manifest or CI reference. |
| `skills/cafleet/` | 9 files, 30,463 words; four reference files duplicate the role files; the "explicitly unwrapped" transport is described in 19 places but is unreachable. |
| Core source | 43 `.rs` files; ~400 production + ~1,300 test lines implement the unreachable unwrapped path; many single-consumer files. |
| Runtime source | 43 `.rs` files + 115 JSON files; v1 event-schema and v1 preview-digest readers serve data that cannot exist in the `cafleet_v9.db` family. |
| Repository files | 769 tracked files, 280 of them non-`design-doc.md` artifacts under `design-docs/`; ~12 GB of ignored local clutter. |

Two facts make the removals safe:

- **Unwrapped members cannot be created.** `member create` always registers a runner run (`runner::create_registered`); no flag or environment variable opts out; the only run-less ordinary-member rows come from test fixtures. The V1 baseline already contains `member_runs`.
- **The v9 format family is unreleased.** Commit `06d17706` ("start the cafleet_v9.db format") is contained in no release tag (latest `0.30.2`). Every v9 event payload was written with `schema_version` `2`, and every v9 preview receipt with digest version `2`.

---

## Specification

### Decisions

| ID | Decision | Rationale |
|---|---|---|
| DEC-1 | Delete the explicitly-unwrapped member path from SPEC, docs, skills and code. An ordinary member without a run is an invariant violation that fails loudly. | Unreachable through the CLI; ~1,700 lines and the Escape-first multiplexer engine exist only for it. |
| DEC-2 | Readers accept only runtime event `schema_version` `2`; the field stays in every payload with value `2`. | No v1 payload can exist in v9; keeping the field keeps a future bump explicit. |
| DEC-3 | Remove `payload_digest_version` entirely (column, CHECK, JSON field, code) by editing the unreleased `V1__baseline.sql` in place. One digest function covers every input kind. | The digest version is a pure function of kind in v9. Editing the baseline is allowed until the family's first release (new sentence in the database-migrations rule). |
| DEC-4 | Native fixtures: one JSON file per backend plus `bundled.json` (4 files). | −111 tracked files with per-backend review diffs. |
| DEC-5 | Only `design-doc.md` is tracked under `design-docs/`; other artifacts are untracked with `git rm --cached` (local copies stay). | −280 tracked files; agent audit artifacts are local working material. |
| DEC-6 | Delete the interview workflow, including the "validate / review an existing design doc" dispatch intent, with no rerouting. The create workflow's own existing-document handling (resume and review-only modes) stays. | User decision. |
| DEC-7 | `skills/cafleet/` 9 → 5 files with the content trims of the skill audit; `roles/member.md` stays a separate file. | Fewer reads with role-file symmetry preserved. |
| DEC-8 | Fold `lib.rs` into `main.rs` (bin-only crate). | Required for the 25-file core target. |
| DEC-9 | One root `mise.toml` with prefixed tasks invoked as `mise <task>`; no monorepo mode. | Removes two files. `//:` paths require monorepo mode, which warns without `config_roots` on mise 2026.10.24. |
| DEC-10 | Phase order: removals and doc/repo merges first, mechanical source merges last. | Each removal lands where reviewers know the current code; the source merges then move fewer lines. |

### Out of scope

- Typing the runtime lifecycle control frames (only the `ServiceControlRequest::Legacy` → `Lifecycle` rename is in scope).
- Replacing `rustix` with `nix`.
- Relocating or removing the native-capture harness in `observation.rs`.
- Generating `presets/opencode/cafleet-ask.md` from `cafleet.md`; replacing `.bumpversion.toml`; SPEC pointer-only stub sections.
- Narrowing own-file-only `pub(crate)` visibility.
- Pruning "Earlier/Prior" OpenCode Console catalog rows.
- Unifying the `member create` preflight live-fleet error (exit 2 vs exit 1 is a contract decision).

### Phase-wide rules

- Phases 1–9 are one PR each, on branch `refactor/0000206-phase-N-<slug>`. Each branch starts from `main` after the previous phase's PR merges; branches never stack. The execute team implements Phases 1–9 (Steps 1–9). After the execute fleet is deleted, the user runs Phase 10, a manual working-tree cleanup with no commit, and Step 11, which lands as one final PR, `docs: mark design 0000206 complete`.
- The Phase 1 PR commits this design doc (`design-docs/0000206-codebase-skill-file-consolidation/design-doc.md`). Every later PR, including the Step 11 PR, checks off its own step's tasks and updates **Progress** and **Last Updated** in the same PR.
- Each phase follows the project documentation order: `SPEC.md` → `docs/` → `README.md` → `skills/` → `.claude/` → code.
- Each phase ends green: `mise //cafleet:test`, `mise //cafleet:lint`, `mise //docs:build` before Phase 3; `mise cafleet-test`, `mise cafleet-lint`, `mise docs-build` from Phase 3 on.
- Phases 2–9 also run the **scratch setup check**, which confirms that the working-tree build installs its embedded skills and presets without touching the installed CLI or the operator's config:
  1. Build the working tree: `mise //cafleet:build` before Phase 3, `mise cafleet-build` from Phase 3 on. The binary is `cafleet/target/release/cafleet`.
  2. Create a scratch home: `mkdir -p <BASE>/.setup-home/phase-<N>`, where `<BASE>` is the execute workflow's BASE for this design.
  3. Run the following as one invocation, with `<S>` the absolute scratch home path:

     ```bash
     HOME=<S> CAFLEET_DATABASE_URL=sqlite:///<S>/.local/share/cafleet/cafleet_v9.db CLAUDE_CONFIG_DIR=<S>/.claude CODEX_HOME=<S>/.codex OPENCODE_CONFIG_DIR=<S>/.config/opencode cafleet/target/release/cafleet setup
     ```

     Then run the same invocation with `doctor` in place of `setup`. The explicit `CAFLEET_DATABASE_URL` is required: `Settings::from_lookup` derives the path from `HOME` only when the variable is unset, and the operator's shell exports it (pointing at the live database the executing fleet uses).
  4. Confirm four things: setup exits 0; doctor reports a database path under `<S>`; doctor reports the installed skills and presets as current; and `<S>/.claude/skills/` mirrors the working-tree `skills/` file set.
- Tests that only exercise removed behavior are deleted outright. No test asserts that a removed path is rejected.
- Contract strings that survive a move are preserved verbatim.
- No phase installs, upgrades or reconfigures the operator's CLI or database. The active CLI is the Homebrew release (`/opt/homebrew/bin/cafleet`, `0.30.2`), and the operator's `CAFLEET_DATABASE_URL` points at a `cafleet_v8.db`. Every in-phase check calls the working-tree build by full path (`cafleet/target/debug/cafleet` or `cafleet/target/release/cafleet`) with the scratch `HOME` and `CAFLEET_DATABASE_URL` above. Upgrading the installed CLI and its database is a user step after the release (§ Post-release user steps).
- Line numbers below refer to `ccf12cab`; each phase re-locates them with `grep` before editing.

### Phase 1 — Untrack design-doc artifacts

`.gitignore` gains, after `/researches/`:

```gitignore
/design-docs/*/*
!/design-docs/*/design-doc.md
```

The design folders themselves stay unignored, so the negation re-includes each `design-doc.md`. Hidden folders inside a design stay covered by the existing `.*` rule.

Untrack every newly ignored tracked path:

```bash
git ls-files --cached --ignored --exclude-standard -z design-docs | xargs -0 git rm --cached --quiet --
```

`.claude/rules/git-workflow.md` § "design-docs/ is committed in this project" states the new policy: stage `design-docs/NNNNNNN-<slug>/design-doc.md` with the implementation; `.gitignore` ignores every other path inside a design folder (prompts, audits, screenshots, scans, clarification files), which stay local. The phase also fixes the run-on line `tracked here.- researches/` into its own bullet.

### Phase 2 — Remove the interview workflow

| File | Change |
|---|---|
| `skills/cafleet-design-doc/interview/` | Delete the directory (`interview.md`, `roles/analyzer.md`). |
| `skills/cafleet-design-doc/SKILL.md` | Frontmatter `description`: drop the "validate, review, or interview … (interview workflow)" clause. Line 18: name the workflows without a count ("the CAFleet-native orchestration workflows linked in Dispatch"). Line 39: "Each workflow body linked in Dispatch carries…". Dispatch: delete the `Validate / review / interview` row. |
| `skills/cafleet-design-doc/reference/coordination.md` | § Payload exemptions: "These exchanges use free-form complete payloads:" (no count); delete the `Interview Director/Analyzer exchange` row; delete the Analyzer no-file-edit stdin paragraph (line 19). |
| `skills/cafleet-design-doc/reference/guidelines.md` | Delete the `question.md` sentence (line 80). Line 88: "The create and execute workflows accept…" and "when resuming or executing". |
| `skills/cafleet-design-doc/create/roles/drafter.md` | Resume Mode trigger: replace "from a previous interview" with a parenthetical saying the existing document carries role-tagged comment markers, such as user-written or Director-recorded user-relay feedback; use the file's existing marker notation for the literal. |
| `skills/cafleet-design-doc/create/create.md` | Step 0 paragraph 2: "For an existing document, retain its content. Existing user-relay markers enter resume mode. Otherwise resolve whether the user wants quality review or a fresh draft before replacing content." (drop the answered-questions reuse). |
| `skills/cafleet/roles/director.md` | Delete the `Read the design document, generate a numbered question list …` START CUE example (line 295). The resume-mode example stays. |
| `docs/docs/how-to/design-doc-development.md` | Line 3: "…through a reviewed design and an implementation."; delete the `2. Refine` prompt row and renumber `Implement` to 2; "These stages invoke create and execute respectively." and delete the "After an interview…" sentence; delete the interview `question.md` clause (lines 29–30). |

No Rust, test, manifest, CI or SPEC change: `cafleet/build.rs` walks `skills/`, and `cafleet setup` replaces each installed skill directory, which prunes an installed `interview/`.

### Phase 3 — Repository file merges

#### Single root `mise.toml`

Delete `cafleet/mise.toml` and `docs/mise.toml`. The root file becomes:

```toml
[tools]
"aqua:pnpm/pnpm" = "12.15.0"
"core:node" = "26.12.1"
"core:rust" = { version = "1.99.1", components = ["clippy", "rustfmt"] }
"core:zig" = "0.15.2"

[tasks.cafleet-test]
dir = "{{config_root}}/cafleet"
run = "cargo test --lib"
description = "Run inline unit tests (positional args forward to the cargo test harness)"

[tasks.cafleet-lint]
dir = "{{config_root}}/cafleet"
run = ["cargo clippy --all-targets -- -D warnings", "cargo fmt --check"]
description = "Run clippy (deny warnings) and the rustfmt check"

[tasks.cafleet-format]
dir = "{{config_root}}/cafleet"
run = "cargo fmt"
description = "Format the Rust sources with rustfmt"

[tasks.cafleet-typecheck]
dir = "{{config_root}}/cafleet"
run = "cargo check"
description = "Type-check the package without producing a binary"

[tasks.cafleet-install]
dir = "{{config_root}}/cafleet"
run = "cargo install --path ."
description = "Install the cafleet CLI from the working tree (re-run after source edits to refresh the installed binary)"

[tasks.cafleet-build]
dir = "{{config_root}}/cafleet"
run = "cargo build --release"
description = "Build the release binary into cafleet/target/release/cafleet"

[tasks.docs-dev]
dir = "{{config_root}}/docs"
run = "pnpm dev"
description = "Serve the documentation site locally"

[tasks.docs-install]
dir = "{{config_root}}/docs"
run = "pnpm install --frozen-lockfile"
description = "Install the documentation site dependencies"

[tasks.docs-build]
dir = "{{config_root}}/docs"
run = "pnpm build"
description = "Build the documentation site and check its links"

[tasks.skill-install]
run = [
  "gh skill install ./ --from-local --agent claude-code --force --scope user",
  "gh skill install ./ --from-local --agent codex --force --scope user",
  "gh skill install ./ --from-local --agent opencode --force --scope user",
]
description = "Install skills"

[tasks.all-install]
run = [{ task = "cafleet-install" }, { task = "skill-install" }]
description = "Install the CLI and the working-tree skills"
```

`monorepo_root` and `[monorepo]` are removed. Rename every reference:

| Old | New |
|---|---|
| `mise //cafleet:<task>` | `mise cafleet-<task>` |
| `mise //docs:<task>` | `mise docs-<task>` |
| `mise //:skill-install` | `mise skill-install` |

| Reference site | Edit |
|---|---|
| `.github/workflows/ci.yml` | `mise cafleet-${{ matrix.task }}` |
| `.github/workflows/docs.yml` | `mise docs-install`, `mise docs-build` |
| `.claude/settings.json` allow | `Bash(mise cafleet-*)`, `Bash(mise docs-*)` replace the `//cafleet*` / `//docs*` patterns; the `mise run *` deny stays |
| `presets/opencode/cafleet.md`, `presets/opencode/cafleet-ask.md` | Allow keys `mise cafleet-test`, `mise cafleet-test *`, `mise cafleet-lint`, `mise cafleet-format`, `mise cafleet-typecheck`, `mise cafleet-build` |
| `SPEC.md` (line 1842 and any other hit) | `mise cafleet-test *` |
| `.claude/rules/commands.md`, `.claude/rules/code-quality.md` | New names; "repository-root mise tasks" replaces "mise full-path tasks"; `mise tasks` remains the registry |
| `docs/docs/contributing.md`, `docs/docs/how-to/mixed-backend-team.md` | New names in prose and examples |

Verify with `mise tasks` (lists every task), `mise cafleet-test -- -- --list` (argument forwarding), `mise docs-build`.

#### `.claude/rules` 7 → 4

| Source | Target | Merge rule |
|---|---|---|
| `bash-tool.md` | `commands.md` § Bash tool | Keep the member-routing pointers and the one-command-per-invocation line once. |
| `coding-agent-overlay.md` | `documentation-maintenance.md` § Coding-agent overlay | Its "Three homes" table folds into the existing "Three audiences" table; "Skills never link `docs/` or `SPEC.md`" appears once. |
| `database-migrations.md` | `code-quality.md` § Database migrations | Verbatim content. |

Rewrite every link to a deleted rule file. The link sites at `ccf12cab` are `.claude/skills/clean-docs/{SKILL.md, reference/review-format.md, affirmative/affirmative.md, simplification/simplification.md}`, all of which the clean-docs merge below rewrites. Afterward `grep -rn -e bash-tool.md -e coding-agent-overlay.md -e database-migrations.md . --exclude-dir=.git --exclude-dir=design-docs --exclude-dir=target --exclude-dir=node_modules --exclude-dir=doc_build` returns nothing.

#### `.claude/skills/clean-docs` 8 → 2

| Target | Sources |
|---|---|
| `SKILL.md` | Current `SKILL.md` spine; one `##` section per workflow (residue, affirmative, simplification) from the three workflow bodies; `residue/reference/patterns.md` inside the residue section; `reference/review-format.md` as § Review format. Dispatch rows link the in-file section anchors. |
| `roles.md` | `## Scanner` (from `roles/scanner.md`) and `## Reviewer` (from `roles/reviewer.md`); text shared between them is stated once in a leading `## Shared member rules` section. |

The spawn deltas name `roles.md` with ROLE-DEF SUFFIX `Follow the Scanner section.` / `Follow the Reviewer section.`. The IMPORTANT lines that cite `.claude/rules/bash-tool.md` cite `.claude/rules/commands.md` § Bash tool. Remove the duplicated affirmative/simplification text identified by the repo audit (44% overlap) so each rule lives in one section.

#### Other files

| Change | Detail |
|---|---|
| Delete `CONTRIBUTING.md` | `README.md` already links the Contributing page. |
| Merge `docs/docs/concepts/member-lifecycle.md` + `member-permission-events.md` → `docs/docs/concepts/members.md` | Lifecycle sections first, then permission events. Keep the existing heading texts so `#command-routing`, `#consume-a-runtime-event` and `#deliver-generated-input` survive. `concepts/_meta.json` gets one `members` entry labelled `Members`. |
| Merge `docs/docs/how-to/design-doc-development.md` → `docs/docs/concepts/overview.md` § Design document workflow | Keep the `{#prompts}` heading anchor. |
| Move `docs/docs/how-to/mixed-backend-team.md` → `docs/docs/mixed-backend-team.md`; delete `how-to/_meta.json` | `docs/docs/_meta.json` replaces the `how-to` dir entry with `{ "type": "file", "name": "mixed-backend-team", "label": "Run a fleet" }`; fix its relative links (`../concepts/` → `concepts/`). |
| Link rewrites | `docs/docs/quickstart.md`, `contributing.md`, `concepts/overview.md`, `concepts/coding-agents.md`, `mixed-backend-team.md`; `README.md` bullets "How-to guides" → "Run a fleet" (`https://himkt.github.io/cafleet/mixed-backend-team`) and "Member permission events" → "Members" (`…/concepts/members`). |

### Phase 4 — `skills/cafleet` 9 → 5

#### Merge map

| Source | Section | Target |
|---|---|---|
| `reference/prompt-routing.md` | The Director's pane primitives; Director-side dispatch | `roles/director.md` `## Director-side dispatch`, holding the pane primitives and the existing Member Exec / Member Prompt / Member Press sections as `###` subsections |
| `reference/prompt-routing.md` | Member-side: reconsider, then route | `roles/member.md` § Command execution `### Reconsider, then route` (single owner of the `Need to run: …` template) |
| `reference/supervision.md` | Spawn Protocol; Director wake; Receiver recovery; Sender follow-up; Authorization-Scope Guard; Stall Response; Escalation; User Delegation Protocol; Recovery; Shutdown | `roles/director.md` `## Supervision`, same heading texts (anchors `#spawn-protocol`, `#director-wake`, `#receiver-recovery`, `#stall-response`, `#user-delegation-protocol`, `#recovery`, `#shutdown` survive). The pointer-only `## Director wake` section is replaced. |
| `reference/supervision.md` | Core Principle; Communication Model | Two sentences in the `## Supervision` intro |
| `reference/supervision.md` | Runner observations; Routing member command requests; Stage 1 | Delete (duplicates of `cli.md` § Wrapped delivery methods, Director-side dispatch, Director wake step 3) |
| `reference/supervision.md` | Reading captures | Fold into `## Member Capture` |
| `reference/member-runtime.md` | Consume observed interactions; Consume automatic review events; Reconcile generated input | `roles/director.md` `## Runtime events`, same heading texts; Reconcile drops its delivery table (owned by `cli.md`) |
| `reference/member-runtime.md` | Explicit manual keys; Inspect and recover | Fold into `### Member Press` and `### Recovery` |
| `SKILL.md` | Listen (Native Inbox Stream) | `roles/director.md` § Director wake |
| `reference/base-dir.md` | Member input and write states | `roles/member.md` `## BASE` |
| `reference/base-dir.md` | No-bypass write protocol; `<unset>` sentinel; Steps 1–2 | `roles/director.md` `### Base directory resolution` under § Member Create — Scratch and audit files |
| `reference/base-dir.md` | Step 0 task-scope resolution | `skills/cafleet-design-doc/reference/guidelines.md` § File Layout `### Task-scope resolution` |

Delete the four reference files. Rewrite every inbound link to the new file and anchor. Afterward `grep -rn -e prompt-routing -e member-runtime.md -e supervision.md -e base-dir.md skills/ .claude/ docs/docs/ presets/` returns nothing; the bare `prompt-routing` pattern also catches prose that names the deleted file.

| Old target | New target |
|---|---|
| `reference/prompt-routing.md#member-side-reconsider-then-route` | `roles/member.md#reconsider-then-route` (the anchor changes with the new heading) |
| `reference/prompt-routing.md#director-side-dispatch` | `roles/director.md#director-side-dispatch` |
| `reference/prompt-routing.md#the-directors-pane-primitives` | `roles/director.md#the-directors-pane-primitives` (kept as a `###` heading) |
| `reference/supervision.md#<anchor>` | `roles/director.md#<anchor>` (same anchor) |
| `reference/member-runtime.md#<anchor>` | `roles/director.md#<anchor>` (same anchor) |
| `reference/base-dir.md`, `#procedure` | `roles/member.md#base` (member reads) or `roles/director.md#base-directory-resolution` (Director reads) |
| `reference/base-dir.md#step-0-task-scope-resolution` | `skills/cafleet-design-doc/reference/guidelines.md#task-scope-resolution` |

Both embedded OpenCode presets (`presets/opencode/cafleet.md:63`, `presets/opencode/cafleet-ask.md:63`) replace "route a genuinely needed denied command through prompt-routing after reconsidering it" with "after reconsidering a denied command, route a genuinely needed one to the Director as the member role's command-routing section describes". Known link sites: `skills/cafleet/SKILL.md`, `roles/director.md` (spawn skeleton BASE read), `roles/member.md`, `reference/coding-agents.md`; `skills/cafleet-design-doc/{SKILL.md, create/create.md, execute/execute.md, reference/coordination.md, reference/guidelines.md}`; `.claude/skills/{clean-docs/SKILL.md, clean-docs/roles.md, skill-author/SKILL.md}`; `docs/docs/mixed-backend-team.md` (the example prompt reads `roles/member.md`, which now carries BASE). The anchors `coding-agents.md#claude-native-director-reception` and `#codex-native-director-reception`, which `SPEC.md` links by URL, are preserved.

#### Content trims (skill audit §2–§5)

| Target | Trim |
|---|---|
| `SKILL.md` | Required reading becomes one table, overlay row first and role row second. Merge Required ids / Spawned-member identity / Placeholder convention into one `## IDs` section; the placeholder errors stay owned by `cli.md`. The permission-error diagnostic becomes one sentence. The 180-word delivery paragraph becomes two sentences that link `cli.md` § Wrapped delivery methods. Drop the overlay example sentence. Member authority is Send/Poll/ACK/show. Drop command-index rows that point only into merged Director sections. § Resolve your overlay and § Documented defaults keep their names. |
| `roles/member.md` | Drop the backend-requirements sentence, the absolute-path loader duplicate, the native-interactions paragraph (replaced by: "If a native permission prompt is open in your pane, leave it; the Director or user answers it. Continue only from the result your host shows."), the committed-send duplicate and the shutdown internals (one sentence remains). The dismissed-prompt rule becomes a single ~60-word owner. |
| `roles/director.md` | Worked resolutions: at most 6 rows, written with role-default placeholders. Delete the Role-defaults restatement (line 194). Merge the OpenCode cost ranking and provider choice into one `#### OpenCode token selection`. § Model routing failures is the single owner of decision-surface relays. Fold § Model list into § Model selection. Halve the skeleton rendering prose. Remove § Placeholder convention (duplicate of `SKILL.md` § IDs). |
| `reference/cli.md` | Apply skill audit §4.1 row by row. Keep invocation forms, the field names the Director reads, disposition tokens, the evidence-ref requirement, the `uncertain_cause` table, exit semantics and every exact error/diagnostic string. Drop key-order dumps, text layouts, lock budgets, the tmux/herdr argument mapping, the 15-row blocked-reason table (one sentence remains) and runner queue internals. |
| `reference/coding-agents.md` | Preamble keeps "last refreshed: YYYY-MM-DD" and "prices are planning estimates". Role requirements shrink to the verdict rule. Delete stale probe-history `Evidence:` lines and version qualifiers (skill audit §2.4), the duplicate delivery note (claude row "The root Director receives broker records…"), the `hasSeenAutoModeOutsideReadPrompt` sentence, the OpenCode Console long-context/promo tables and the Go plan-limit / long-context / promo / Off-Peak / privacy tables. Trim the native-reception recipes ~40% while keeping their anchors. Role eligibility evidence cells stay. |
| Maintainer content | Move `## Template`, the OpenCode Director verdict-change procedure and the catalog/eligibility ownership sentences from `coding-agents.md` into `.claude/rules/documentation-maintenance.md` § Coding-agent overlay ("Add a backend by copying the Template in this rule"). |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | Remove the mandates for the deleted side tables (lines 135–136, 175–177, 191–206) and update the "worked resolutions" wording (line 20). |

Phase 4 removes no unwrapped mention; Phase 5 owns those (~700 words). The word budget is therefore checked in two stages: at most 19,700 words after Phase 4, and at most 19,000 words after Phase 5.

### Phase 5 — Remove the explicitly-unwrapped member path

#### Contract after removal

- Every ordinary member has a run; `runtime` is `null` only for the root Director in `member create` / `member show`.
- `delivery.method` is `profile`, `fallback`, or `null` before selection.
- Escape-first input exists only as wrapped delivery fallback (no measured plan, unknown coverage, or `idle_timeout`).
- An ordinary member row without a run is an invariant violation. It surfaces as `member <ID> has no recorded run`, and a single helper pair in `src/broker/members.rs` owns that string:

  ```rust
  pub(crate) fn missing_run(member_id: i64) -> CafleetError {
      CafleetError::App(format!("member {member_id} has no recorded run"))
  }

  pub(crate) fn require_runtime(member: &MemberRecord) -> Result<&RuntimeSnapshot, CafleetError> {
      member.runtime.as_ref().ok_or_else(|| missing_run(member.member_id))
  }
  ```

  `runtime/runner/recovery.rs:173` (`member recover`) replaces its inline `format!` with `missing_run(member_id)`, so the CLI and the runner emit the same string from one place.

#### Run-less row behavior per caller

| Caller | Behavior for a run-less ordinary member |
|---|---|
| `member prompt`, `member exec` | `require_runtime` fails before any pane I/O; exit 1 with `Error: member <ID> has no recorded run`. |
| Preview drain (`deliver_pending`, used by `message send` and `message wait`) | Recorded as a per-attempt notification failure whose `CAUSE` is `member <ID> has no recorded run`. The message stays pending, and `message wait` reports the same diagnostic on each attempt. |
| `message send` | The message is committed first. Stdout prints the normal successful record (with the persisted ID), and the `delivery` object takes the failed-attempt projection with `method` `null`. Stderr prints `Message NEW_ID was persisted. message NEW_ID was not notified to member <ID>: member <ID> has no recorded run; the next send to member <ID> or the next message wait retries it.` Exit 0, as for every notification failure. |
| `message send` delivery projection | The `else if notification_sent` → `DeliveryMethod::Unwrapped` branch is deleted; a recipient without a run takes the drain row above. |
| `member recover` | Existing behavior, now through `missing_run`. |
| `member delete`, `fleet delete` | Teardown always succeeds. A run-less row skips runner shutdown, closes its recorded pane under the existing deletion lock, and deregisters through the retained run-less branch of `finalize_deletion` (renamed parameter `has_run`). It never calls `require_runtime`. |

SPEC § `message send` failure-cause table (line ~480) gains the row "Recipient ordinary member has no recorded run | `member MEMBER_ID has no recorded run`". It drops the rows only the unwrapped drain produces; each row is checked against the wrapped path in `delivery.rs` before deletion. SPEC § `member prompt` / `member exec` gain the exit-1 error, and § `member delete` / `fleet delete` state run-less teardown.

#### SPEC edits

Delete or rewrite every unwrapped sentence: lines 26, 693, 716, 728, 815, 1018, 1183, 1192–1194 (the 3.5-second unwrapped reservation paragraph), 1218, 1266 (the delivery sequence describes the wrapped path only), 1307, 1313, 1319–1324 and 1329 (drop the unwrapped steps from the numbered drain procedure), 1345 (trade-offs apply to wrapped delivery fallback), 1881, 2730, 2778, 2877, 2932, 3402 (drop the unwrapped test-checklist clause). Add the `require_runtime` error string to the CLI errors contract.

#### Retained hits

These `grep -rni unwrapped` hits are unrelated to the transport and stay:

| Location | Meaning |
|---|---|
| SPEC § Multiplexer capture table (line 1404) | herdr `--source recent-unwrapped` capture argument |
| SPEC § Claude Bash permission detection (lines 1987, 1997) | Native permission form rendered "unwrapped or wrapped" by the terminal |
| `cafleet/src/multiplexer/herdr.rs` (`"recent-unwrapped"` at lines 409, 564) | herdr capture argument and its test |
| `cafleet/src/coding_agent/profiles/detector.rs` (`unwrapped_rendering` helpers, lines 1311–1720) | Native-form wrapping test helpers |
| `cafleet/src/coding_agent/profiles/catalog.rs` (lines 3459, 3506–3522, 5007–5056) | Native-form wrapping tests |

After Phase 9 the two `profiles/` files live at `coding_agent/detector.rs` and `coding_agent/catalog.rs`.

#### docs, skills

| File (after Phase 3) | Edit |
|---|---|
| `docs/docs/concepts/members.md` | Delete "Explicitly unwrapped members receive Escape-first previews." (from member-lifecycle line 13). Replace "Members with `runtime: null` retain their unwrapped transport." (from member-permission-events line 54) with "Only the root Director has `runtime: null`." |
| `docs/docs/concepts/coding-agents.md` (line 43) | Delete the clause "explicitly unwrapped members retain Escape-first input". |
| `docs/docs/mixed-backend-team.md` (line 60) | Delete "Explicitly unwrapped members retain Escape-first input." |

Afterward `grep -rni unwrapped docs/docs` returns nothing.

- `skills/cafleet`: remove every remaining hit from the skill audit §2.1 table. Delete the coding-agents claude note row "Legacy unwrapped members run `dontAsk`…" and the cli.md lead table. `dontAsk` stays a valid explicit `--permission-mode`.

#### Code

| Location | Change |
|---|---|
| `src/delivery.rs` | Delete the unwrapped preview drain (lines 194–326), `notification_line`, `storage_failure`. Keep `fallback_preview_line` and `DISMISSED_PROMPT_NOTE` (the runner's wrapped fallback uses them). |
| `src/cli/member.rs` | Delete the unwrapped `member prompt` / `member exec` branch (742–797) and its help text (143–144); both commands call `require_runtime`. |
| `src/cli/message.rs` | Delete the `DeliveryMethod::Unwrapped` projection (237–247). |
| `broker/runtime` | Delete the `DeliveryMethod::Unwrapped` variant and its `unwrapped` wire value. |
| `src/input.rs` | Delete `lock_member` (the unwrapped prompt/exec lock). `finalize_deletion` keeps its run-less branch for teardown; its `wrapped` parameter is renamed `has_run`. |
| `src/deletion.rs` | Keep the `snapshot.runtime.is_some()` selector (273), passed as `has_run`. |
| `src/runtime/runner/recovery.rs` | Line 173 uses `crate::broker::members::missing_run(member_id)`. |
| `src/multiplexer/{mod,tmux,herdr}.rs` | Delete `SUBMIT_DELAY`, `ESCAPE_*`, `MIN_LITERAL_BUDGET`, `REVALIDATION_MARGIN`, `*_RESERVATION`, `INPUT_RESERVATION`, `INSUFFICIENT_INPUT_BUDGET`, `Lead`, `PaneInput`, `BackendCore::send_prompt`, `submit_prompt`, `AnyMultiplexer::send_prompt`, and the backend `send_prompt` argv builders. |
| `src/broker/members.rs` | Add `missing_run` and `require_runtime`. |

Tests:

| Test | Action |
|---|---|
| Unwrapped-only drains, Escape argv order and budget diagnostics: `delivery.rs` ~1524–2318, `multiplexer/mod.rs` 1156–1236 and 1330–1445, tmux ~400–440, herdr ~780–800, `input.rs` `lock_member` tests, `presentation.rs` 531–536 (runtime-null ordinary member) | Delete |
| `cli/mod.rs:629` `member_exec_help_describes_wrapped_and_unwrapped_input_and_orders_member_before_command` | Rewrite as `member_exec_help_describes_runner_delivery_and_orders_member_before_command`. It asserts that the help describes runner delivery and fallback and orders `MEMBER_ID` before the command, with no `unwrapped`/`escape` term requirement. |
| `cli/member.rs:2373` `runner_recovery_entry_unwrapped_member_fails_explicitly_before_registry_or_artifact_mutation` and fixture `unwrapped_registry` (2355) | Keep. Rename to `member_recover_run_less_member_fails_explicitly_before_registry_or_artifact_mutation` and `run_less_registry`; this is the `member recover` `missing_run` test. |
| `cli/message.rs:288` `send_delivery_reports_self_sends_the_root_director_and_unwrapped_notifications` | Rename to `send_delivery_reports_self_sends_the_root_director_and_run_less_recipients`. The self-send and root-Director assertions stay unchanged. The run-less case asserts the committed record, `method` `null` and the persisted stderr diagnostic. |
| `deletion.rs:1031` `runner_review_fleet_mutable_shutdown_evidence_keeps_one_deadline_and_unwrapped_behavior` | Rename to `…_keeps_one_deadline_and_run_less_teardown`. It keeps asserting that the run-less row's pane is killed and the row deregistered. |
| New | `member prompt` and `member exec` against `run_less_registry` exit 1 with `member <ID> has no recorded run` and perform no pane I/O. `message wait` reports the same cause for a run-less recipient. |

`message wait` keeps its round-robin `deliver_pending` loop for wrapped previews.

### Phase 6 — Remove v1 event-schema and v1 preview-digest compatibility

#### Event schema

- `src/broker/runtime/events.rs`: the reader requires `schema_version == 2` (message `unsupported event schema`); delete the separate "automatic review and delivery result events use the v2 schema" check.
- Delete the v1 fixture helper and tests: `events.rs` 1480–1575, `runtime_payload_v1_*` (1709, 1729, 1749, 1784, 1805), `runtime_payload_v2_new_event_and_old_v1_message_coexist_without_rewriting_history` (1868), `recovery/replay.rs` `runner_recovery_preserves_committed_v1_bytes_ids_and_disposition_while_new_records_use_v2` (352).
- SPEC 2577: "Event payloads carry `schema_version=2`; readers accept only version 2 and return existing records and their associated message bytes unchanged on replay." SPEC 2621: drop "Version-1 reads preserve…" and rename "Version-2 event keys" to "Event keys". SPEC 3012: drop "Preserve existing schema-version-1 records…"; keep idempotent replay and the 16 KiB limit.

#### Payload digest

One digest for every input kind:

```json
{"run_id":42,"kind":"preview","message_id":99,"literal":"CANONICAL_BASE_PREVIEW"}
```

SHA-256 over `digest::canonical_json` of that object. For previews, `literal` is the canonical base preview (method-independent wrapped line ending at the resume clause); for other kinds, it is the submitted literal, exactly as `input_payload_digest` computes today.

| Location | Change |
|---|---|
| `migrations/V1__baseline.sql` | Delete the `payload_digest_version` column and its CHECK (lines 153–154). |
| `src/broker/runtime/inputs.rs` | Delete `preview_payload_digest_v2`, `versioned_payload_digest`, `NewReservation.payload_digest_version`, and the version checks in the delivery projection (lines 49–60). |
| `src/broker/runtime/records.rs` | Delete `InputOperation.payload_digest_version`. |
| `src/runtime/runner/input.rs`, `input_client.rs` | Delete the field's propagation and the version lookup; reservation and duplicate validation call `input_payload_digest`. |
| `src/presentation.rs`, `src/delivery.rs`, `src/db/mod.rs` | Remove the field from `input show` JSON, delivery and the column-shape diagnosis. |
| Preview-receipt reuse tests that assert the v2 preimage with `"payload_digest_version":2`: `delivery.rs` 1396–1401 and 2468, `runtime/runner/input.rs` 5615–5618, 6446 and 6476, `presentation.rs` 410, 436, 732 and 821 | Rewrite to the single preimage above (drop the key from expected preimages and JSON); the reuse behavior they cover stays |
| Tests whose only subject is digest-version selection or version/kind mismatch | Delete |
| SPEC 823, 2575, 2955–2964, 3124, 3348–3349 | Remove the field from the key list, schema table and SQL block. Replace the version paragraph with the single-preimage rule above. 3124 reads "unchanged digests". |
| `skills/cafleet/reference/cli.md` | Remove any remaining digest-version mention. |
| `.claude/rules/code-quality.md` § Database migrations | Add: "Until a format family's first tagged release, edit its `V1__baseline.sql` in place; from that release onward, extend the chain." |

The column-shape diagnosis classifies a `cafleet_v9.db` created by an earlier main-branch build (with the column) as incompatible and preserves its bytes. In-phase checks always run against a fresh scratch database (§ Phase-wide rules), so the change never meets an old-shape file during implementation. The operator's own CLI and database move to v9 only through § Post-release user steps.

### Phase 7 — Native fixtures: one JSON file per backend

#### Layout

```text
cafleet/src/coding_agent/profiles/bundled.json        (moves to coding_agent/bundled.json in Phase 9)
cafleet/src/coding_agent/profiles/native/claude.json
cafleet/src/coding_agent/profiles/native/codex.json
cafleet/src/coding_agent/profiles/native/opencode.json
```

#### Per-backend file

```json
{
  "schema_version": 1,
  "backend": "claude",
  "provenance": "native",
  "screens": {
    "claude-2.1.302-default-permission-1": {
      "rows": 36,
      "cols": 120,
      "alternate_screen": true,
      "screen_seq": 1234,
      "cursor_row": 32,
      "cursor_col": 2,
      "lines": ["…one string per row…"],
      "masked_rows": [],
      "cell_backgrounds": []
    }
  }
}
```

- Screen keys appear in `bundled.json` `fixtures[]` order; formatting is 2-space pretty JSON with one array element per line, as today.
- A screen object has exactly `rows, cols, alternate_screen, screen_seq, cursor_row, cursor_col, lines, masked_rows` plus optional `cell_backgrounds`; field semantics are unchanged. `schema_version`, `fixture_id`, `provenance`, `backend` and `native_version` leave the screen (the key is the id; the file carries backend and provenance; the catalog entry carries the capture version).

#### `bundled.json` fixture entry

Unchanged keys `id, kind, backend, native_version, rows, cols, source_ref, sha256`, where:

- `source_ref` is the backend file, e.g. `native/claude.json`.
- `sha256` is the SHA-256 hex of `digest::canonical_json(screen_object)`.

#### Verification (`verify_native_screen`), all fail-loud catalog load errors

| Condition | Error |
|---|---|
| File `schema_version` ≠ 1, `provenance` ≠ `native`, or `backend` ≠ entry backend | `bundled fixture file identity differs from its catalog entry` |
| Entry id absent from `screens` | `bundled fixture screen is missing` |
| A `screens` key with no catalog entry | `bundled fixture file holds an uncited screen` |
| Digest mismatch | `bundled fixture digest mismatch for <ID>: catalog <SHA>, computed <SHA>` |
| `rows`/`cols` differ from the entry | `bundled fixture identity differs from its catalog entry` (existing string) |

The geometry, cursor, lines, masked-row and cell-background checks keep their current strings.

#### Code and docs

- `catalog.rs`: replace the 115-line `bundled!` list with three `include_bytes!` constants keyed by backend.
- `test_support::bundled_fixture(id)` returns the screen object's canonical JSON `String`, which the detector and input replay helpers parse as today. `bundled_embedded()` returns the three files.
- All 114 screens stay, including the 22 profile-uncited regression screens.
- One-time conversion runs from a scratch script under the execute BASE (never committed). It asserts that each screen object equals its old file minus the five dropped keys, writes the three files, recomputes `sha256`, then deletes the 114 files.
- SPEC 1979 and 2180–2209 describe the per-backend file, the screen field table without the moved fields, and the new verification bullets. The capture-to-fixture conversion sentence (2195) says the converted screen is inserted under its id in its backend file.
- `.claude/rules/code-quality.md` names `coding_agent/profiles/native/<backend>.json`.

### Phase 8 — Core source merges (43 → 25 `.rs`)

Target tree; each target absorbs the listed sources with their `#[cfg(test)] mod tests` / `test_support` blocks merged into one block per file:

| Target | Sources |
|---|---|
| `main.rs` | `main.rs` + `lib.rs` (macros at the top, before the first `mod`; `cli`/`error` become private) |
| `error.rs` | `error.rs`; `CafleetError::Value` merges into `App` at every construction site crate-wide |
| `config.rs` | `config.rs` + `config_dir.rs` (one `home_dir()`) |
| `time.rs` | `time.rs` + `input::Deadline` + `receive::{Clock, SystemClock}` + `signals.rs` |
| `digest.rs` | `digest.rs` (`#[cfg(test)] sha256_file_hex` moves to `test_support`) |
| `assets.rs` | `assets.rs` + `embedded.rs` |
| `db.rs` | `db/mod.rs` + `diagnosis.rs` + the database-budget half of `input.rs`; adds `open_existing(url, read_only)` replacing the three exists-then-classify copies |
| `pane_lock.rs` | Pane-lock half of `input.rs` + `valid_input` |
| `delivery.rs` | `delivery.rs` + `receive.rs` (breaks the import cycle) |
| `broker/mod.rs` | `broker/mod.rs` + `records.rs`; callers use `crate::broker::<item>` through the re-exports |
| `broker/fleets.rs` | `fleets.rs` + `asset_installs.rs` + doctor's inline `asset_installs` / `member_runs` reads |
| `broker/members.rs` | `members.rs` + `deletion::active_member`; free `#[cfg(test)]` helpers move to `test_support` |
| `broker/messaging.rs` | `messaging.rs` + `panes.rs` |
| `multiplexer/mod.rs` | `mod.rs` + `keys.rs`; `pane_cleanup` into a `#[cfg(test)] impl` |
| `multiplexer/tmux.rs`, `multiplexer/herdr.rs` | unchanged |
| `output/mod.rs` | `output/mod.rs` + `render.rs` |
| `output/json.rs` | `presentation.rs` |
| `output/text.rs` | `formatters.rs` + `output/runtime.rs` |
| `cli/mod.rs` | `cli/mod.rs` + `helpers.rs`; one `available_mux(settings, Option<Deadline>)` |
| `cli/fleet.rs` | `cli/fleet.rs` + `deletion.rs` |
| `cli/member.rs` | `cli/member.rs` + `spawn_prompt.rs` + `capture.rs`; `MemberCommand::Create` takes a `#[derive(Args)] CreateArgs`, which removes the `#[allow(clippy::too_many_arguments)]` |
| `cli/message.rs` | `cli/message.rs` + `listen.rs` |
| `cli/runtime.rs` | unchanged |
| `cli/setup.rs` | `setup.rs` + `doctor.rs` |

Also:

- Use `CafleetError::merge_cleanup` in `listen` and `messaging`.
- Add one `broker::members::active_member` and one pinned-fleet drift check, both keeping their existing error strings verbatim.
- Move `PaneLock::acquire_released` and `RELEASED_LOCK_GRACE` into a `#[cfg(test)] impl PaneLock`.
- The `cafleet-test` task becomes `cargo test --bin cafleet`. `.claude/rules/code-quality.md` says crate-wide test macros sit at the top of `main.rs`. `test_path!` keeps its `cafleet::` prefix strip; the binary crate's module paths start with `cafleet::`.
- Runtime-scope files change only their `use` paths and their `CafleetError::Value` → `App` construction sites. The production sites are `coding_agent/policy.rs:154`, `coding_agent/profiles.rs:57`, `coding_agent/profiles/composer.rs:73`, `broker/runtime/inputs.rs:399`, `broker/runtime/mod.rs:191` and `broker/runtime/events.rs:108`; test sites are in `runner/input_client.rs`, `process.rs` and `transport/drain.rs`. `grep -rn 'CafleetError::Value' cafleet/src` returns nothing afterward.

### Phase 9 — Runtime source merges (43 → 21 `.rs`)

| Target | Sources |
|---|---|
| `runtime/mod.rs` | `runtime/mod.rs` + `runner/mod.rs` (the `runner/` level is dropped; glob re-exports collapse) |
| `runtime/press.rs`, `runtime/system.rs`, `runtime/process.rs`, `runtime/service.rs` | unchanged content (`service.rs`, `process.rs` move up from `runner/`) |
| `runtime/input.rs` | `runner/input.rs` + `input_clock.rs` |
| `runtime/input_client.rs` | `input_client.rs` + `input_recovery.rs` |
| `runtime/control.rs` | `codec.rs` + `input_protocol.rs` + `control.rs` + `health.rs` (`ObservationHealth` moves to `observation.rs`; `ServiceControlRequest::Legacy` → `Lifecycle`, `decode_legacy_control_body` → `decode_lifecycle_control_body`) |
| `runtime/observation.rs` | `observation.rs` + `episodes.rs` |
| `runtime/transport.rs` | `transport.rs` + `transport/drain.rs` + `transport/replies.rs` + `terminal.rs` |
| `runtime/artifacts.rs` | `artifacts.rs` + `artifacts/recovery_descriptor.rs` |
| `runtime/writer.rs` | `writer.rs` + `outbox.rs` |
| `runtime/recovery.rs` | `recovery.rs` + `recovery/exceptional.rs` + `recovery/replay.rs` |
| `runtime/lifecycle.rs` | `lifecycle.rs` + `creation.rs` + `NativeLaunchHost` from `control.rs` |
| `coding_agent/mod.rs` | `mod.rs` + `policy.rs` + `native_version.rs` |
| `coding_agent/catalog.rs` | `profiles.rs` + `profiles/catalog.rs` + `profiles/composer.rs` (the `profiles/` level is dropped) |
| `coding_agent/detector.rs` | `profiles/detector.rs` |
| `broker/runtime/mod.rs` | `mod.rs` + `records.rs` |
| `broker/runtime/events.rs` | `events.rs` + `effective_policy.rs` |
| `broker/runtime/inputs.rs` | `inputs.rs` + `permissions.rs` |
| `broker/runtime/evidence.rs` | unchanged |

Data files move with the catalog: `coding_agent/profiles/bundled.json` → `coding_agent/bundled.json`, `coding_agent/profiles/native/` → `coding_agent/native/`. Update the `include_*!` paths and the `.claude/rules/code-quality.md` path.

In-scope dedupes, with error strings preserved:

| ID | Change |
|---|---|
| R1 | `InputOperation::same_identity` replaces 4 hand-written identity predicates. |
| R2 | One `revalidate_run_snapshot` replaces the 3 run+placement checks. |
| R3 | One `placement_of(conn, member_id)` replaces the 3 SELECTs. |
| R4 | `reserve_event_sequences_in_transaction` shared by exceptional recovery and `reserve_event_sequences`. |
| R5 | One `delivery_result_transition` replaces 2 literals. |
| R6 | `validate_database` replaces the inline DB-identity checks. |
| R7 | `WriterHooks`/`InputHooks` and their fields become `#[cfg(test)]`; delete `InertWriterHooks`/`InertInputHooks`. |
| R11 | One `crate::error::Result` alias replaces 3 local aliases. |

`test_path!` launchers recompute their paths automatically after the moves.

### Phase 10 — Local untracked clutter (manual, no commit)

The user runs Phase 10 (Step 10) and the finalize step (Step 11) after the execute fleet for this design has been removed with `fleet delete`. The execute team stops at Step 9.

Run Phase 10 from the repository root in this order:

1. Confirm that no CAFleet runner or receiver remains: `pgrep -f 'cafleet __member-run'` and `pgrep -f 'cafleet message listen'` both print nothing (exit 1).
2. Stop leftover CAFleet tmux servers. List the sockets in the tmux socket directory (`/tmp/tmux-<uid>/` or `$TMUX_TMPDIR/tmux-<uid>/`, with `<uid>` from `id -u`). For each socket named `cafleet-*` (for example `cafleet-194-acceptance`, whose pane runs under `.execution/194/`), run `tmux -L <name> kill-server`. Repeat the listing to confirm none remain.
3. Delete the paths below.

| Delete | Note |
|---|---|
| `.execution/`, `.p194/`, `.hermes-spike/`, `.cafleet-evidence/`, `.scan/`, `.verifier/`, `.verifier-smoke/`, `.screenshots/`, `.demo/`, `.scratch/`, `.pnpm-store/` | Completed or aborted design evidence; no current reference |
| `.DS_Store`, `docs/.DS_Store` | Finder metadata |
| `cafleet/.pytest_cache/`, `cafleet/.ruff_cache/` | Python-era remnants |
| `.claude/worktrees/` | Empty; recreated on demand |
| `design-docs/0000175-reliable-pane-message-notifications/` | Orphan folder without `design-doc.md` |
| `.prompts/verifier-*/headless-profile/` | Browser profiles only; other `.prompts/` files stay |
| Empty untracked directories `cafleet/src/assets/`, `cafleet/src/broker/runtime_contract_tests/`, `cafleet/src/runtime/press/`, `cafleet/src/db/runtime_tests/` | Leftovers from earlier moves |

Kept: hidden per-design scratch folders `design-docs/*/.*/`, `cafleet/target/`, `docs/node_modules/`, `docs/doc_build/`, `.claude/settings.local.json`, and Phase 1's untracked design artifacts.

### Post-release user steps

These steps run after the first release that contains Phases 5–9. They are the user's own upgrade, outside the implementation tasks, and the release notes state them:

1. Identify the active CLI with `which -a cafleet` (today `/opt/homebrew/bin/cafleet`, the Homebrew release). Upgrade it through its installer, for example `brew upgrade cafleet`, and confirm with `cafleet --version`.
2. Resolve the database. A v9 binary refuses a `cafleet_v8.db` and preserves its bytes. Unset `CAFLEET_DATABASE_URL` in the shell profile to use the default `~/.local/share/cafleet/cafleet_v9.db`, or point it at a v9 path.
3. If the resolved path already holds a `cafleet_v9.db` created by a pre-release main-branch build, delete it; its input-operation shape is incompatible.
4. Run `cafleet setup`, then confirm that `cafleet doctor` reports the expected database path and current assets.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Phase 1 — Untrack design-doc artifacts

- [ ] Add the two `design-docs` ignore lines to `.gitignore` <!-- completed: -->
- [ ] Run the `git ls-files --cached --ignored … | xargs -0 git rm --cached` command and confirm `git ls-files design-docs | grep -v '/design-doc.md$'` is empty <!-- completed: -->
- [ ] Confirm a new `design-docs/<existing>/scratch.txt` shows as ignored (`git check-ignore`) and `design-doc.md` does not <!-- completed: -->
- [ ] Rewrite `.claude/rules/git-workflow.md` with the tracking policy and fix the run-on bullet <!-- completed: -->
- [ ] Stage this design doc in the Phase 1 PR <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //docs:build`; open the Phase 1 PR <!-- completed: -->

### Step 2: Phase 2 — Remove the interview workflow

- [ ] Edit `docs/docs/how-to/design-doc-development.md` per § Phase 2 <!-- completed: -->
- [ ] Edit `skills/cafleet-design-doc/SKILL.md` (description, workflow wording, Dispatch row) <!-- completed: -->
- [ ] Edit `reference/coordination.md` (exemption sentence, row, Analyzer paragraph) and `reference/guidelines.md` (lines 80, 88) <!-- completed: -->
- [ ] Edit `create/roles/drafter.md` Resume Mode trigger and `create/create.md` Step 0 <!-- completed: -->
- [ ] Delete the Analyzer START CUE example in `skills/cafleet/roles/director.md` <!-- completed: -->
- [ ] Delete `skills/cafleet-design-doc/interview/` <!-- completed: -->
- [ ] Run both interview greps from Success Criteria, the three validation tasks and the scratch setup check; open the Phase 2 PR <!-- completed: -->

### Step 3: Phase 3 — Repository file merges

- [ ] Write the root `mise.toml`; delete `cafleet/mise.toml` and `docs/mise.toml` <!-- completed: -->
- [ ] Rename task references in SPEC, CI workflows, `.claude/settings.json`, both OpenCode presets, rules, `docs/docs/contributing.md` and `mixed-backend-team.md` <!-- completed: -->
- [ ] Verify `mise tasks`, `mise cafleet-test -- -- --list` and `mise docs-build`; `grep -rn -e '//cafleet:' -e '//docs:' -e 'mise //:' . --exclude-dir=.git --exclude-dir=design-docs --exclude-dir=target --exclude-dir=node_modules --exclude-dir=doc_build` returns nothing <!-- completed: -->
- [ ] Merge `docs/docs/concepts/member-lifecycle.md` and `member-permission-events.md` into `concepts/members.md`; update `concepts/_meta.json` <!-- completed: -->
- [ ] Merge `docs/docs/how-to/design-doc-development.md` into `concepts/overview.md` keeping `{#prompts}` <!-- completed: -->
- [ ] Move `how-to/mixed-backend-team.md` to `docs/docs/`, delete `how-to/_meta.json`, update `docs/docs/_meta.json` <!-- completed: -->
- [ ] Rewrite docs links and the `README.md` bullets <!-- completed: -->
- [ ] Delete `CONTRIBUTING.md` <!-- completed: -->
- [ ] Merge `.claude/rules/bash-tool.md` into `commands.md`, `coding-agent-overlay.md` into `documentation-maintenance.md`, `database-migrations.md` into `code-quality.md`; rewrite every link to them <!-- completed: -->
- [ ] Merge `.claude/skills/clean-docs` into `SKILL.md` + `roles.md`; update spawn deltas and IMPORTANT lines; remove the duplicated workflow text <!-- completed: -->
- [ ] Run `mise cafleet-test`, `mise cafleet-lint`, `mise docs-build` (link check) and the scratch setup check (confirm the installed OpenCode presets carry the renamed allow-list keys); open the Phase 3 PR <!-- completed: -->

### Step 4: Phase 4 — `skills/cafleet` 9 → 5

- [ ] Merge `reference/supervision.md` into `roles/director.md` § Supervision per the merge map <!-- completed: -->
- [ ] Merge `reference/member-runtime.md` into `roles/director.md` § Runtime events, Member Press and Recovery <!-- completed: -->
- [ ] Merge `reference/prompt-routing.md` into `roles/director.md` § Director-side dispatch and `roles/member.md` § Reconsider, then route <!-- completed: -->
- [ ] Merge `reference/base-dir.md` into `roles/member.md` § BASE, `roles/director.md` § Base directory resolution, and `cafleet-design-doc/reference/guidelines.md` § Task-scope resolution <!-- completed: -->
- [ ] Move § Listen from `SKILL.md` into `roles/director.md` § Director wake <!-- completed: -->
- [ ] Delete the four reference files and rewrite every inbound link per the old → new target table, including the `#reconsider-then-route` anchor change; the deleted-file grep returns nothing <!-- completed: -->
- [ ] Rewrite the `prompt-routing` sentence in `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` <!-- completed: -->
- [ ] Apply the `SKILL.md` trims <!-- completed: -->
- [ ] Apply the `roles/member.md` trims <!-- completed: -->
- [ ] Apply the `roles/director.md` model-selection and skeleton trims <!-- completed: -->
- [ ] Apply the `reference/cli.md` trims (skill audit §4.1) <!-- completed: -->
- [ ] Apply the `reference/coding-agents.md` trims and move the maintainer content into `.claude/rules/documentation-maintenance.md` <!-- completed: -->
- [ ] Update `.claude/skills/cafleet-model-list-refresh/SKILL.md` for the removed tables and wording <!-- completed: -->
- [ ] Confirm `skills/cafleet` has 5 files totalling ≤ 19,700 words and the SPEC-linked coding-agents anchors exist <!-- completed: -->
- [ ] Run `mise cafleet-test`, `mise cafleet-lint`, `mise docs-build` and the scratch setup check; open the Phase 4 PR <!-- completed: -->

### Step 5: Phase 5 — Remove the explicitly-unwrapped member path

- [ ] Edit SPEC per § Phase 5 SPEC edits and the run-less behavior paragraph (failure-cause row, prompt/exec error, run-less teardown) <!-- completed: -->
- [ ] Edit `docs/docs/concepts/members.md`, `docs/docs/concepts/coding-agents.md` and `docs/docs/mixed-backend-team.md` per § Phase 5 docs table <!-- completed: -->
- [ ] Remove the remaining unwrapped mentions from `skills/cafleet` <!-- completed: -->
- [ ] Add `missing_run` / `require_runtime`; wire `member prompt`, `member exec`, the preview drain and `recovery.rs:173` per the caller table <!-- completed: -->
- [ ] Delete the unwrapped drain and helpers in `src/delivery.rs` <!-- completed: -->
- [ ] Delete the unwrapped prompt/exec branch and help text in `src/cli/member.rs` and the projection in `src/cli/message.rs` <!-- completed: -->
- [ ] Delete `DeliveryMethod::Unwrapped` and `lock_member`; rename the `finalize_deletion` parameter to `has_run`, keeping run-less teardown <!-- completed: -->
- [ ] Delete the Escape-first multiplexer engine from `multiplexer/{mod,tmux,herdr}.rs` <!-- completed: -->
- [ ] Apply the Phase 5 test table (delete, rewrite, rename-and-keep, new tests) <!-- completed: -->
- [ ] Confirm `skills/cafleet` totals ≤ 19,000 words <!-- completed: -->
- [ ] Run the case-insensitive unwrapped grep (only § Retained hits remain), the three validation tasks and the scratch setup check; open the Phase 5 PR <!-- completed: -->

### Step 6: Phase 6 — Remove v1 event-schema and preview-digest compatibility

- [ ] Edit SPEC 2577, 2621, 3012 (event schema) <!-- completed: -->
- [ ] Edit SPEC 823, 2575, 2955–2964, 3124, 3348–3349 (digest) <!-- completed: -->
- [ ] Remove digest-version mentions from `skills/cafleet/reference/cli.md` <!-- completed: -->
- [ ] Add the editable-baseline sentence to `.claude/rules/code-quality.md` § Database migrations <!-- completed: -->
- [ ] Make the event reader require version 2; delete the v1 helper and tests in `events.rs` and `recovery/replay.rs` <!-- completed: -->
- [ ] Delete the `payload_digest_version` column from `V1__baseline.sql` <!-- completed: -->
- [ ] Collapse the digest code to `input_payload_digest` and remove the field from records, runner input, input client, presentation, delivery and diagnosis <!-- completed: -->
- [ ] Rewrite the preview-receipt reuse tests to the single preimage; delete the version-selection-only tests; confirm the `payload_digest_version` grep is empty <!-- completed: -->
- [ ] Run the three validation tasks and the scratch setup check; in the PR body, tell the reader to delete any local `cafleet_v9.db` created by an earlier main-branch build before running `cafleet setup` <!-- completed: -->

### Step 7: Phase 7 — Native fixtures per backend

- [ ] Edit SPEC 1979, 2180–2209 and 2195 for the per-backend file and verification rules <!-- completed: -->
- [ ] Convert the 114 fixtures with a scratch script under BASE (equality assertion per screen) into `native/{claude,codex,opencode}.json`; recompute every `bundled.json` `sha256` and `source_ref` <!-- completed: -->
- [ ] Replace the `bundled!` list with three `include_bytes!` constants and rewrite `verify_native_screen` with the new conditions and error strings <!-- completed: -->
- [ ] Update `test_support::bundled_fixture` / `bundled_embedded` and every test caller <!-- completed: -->
- [ ] Add unit tests for the missing-screen, uncited-screen, file-identity and digest-mismatch errors <!-- completed: -->
- [ ] Delete the 114 per-fixture files; update the fixture path in `.claude/rules/code-quality.md` <!-- completed: -->
- [ ] Run the three validation tasks and the scratch setup check; open the Phase 7 PR <!-- completed: -->

### Step 8: Phase 8 — Core source merges

- [ ] Merge `config_dir`→`config`, `embedded`→`assets`, `panes`→`broker/messaging`, `asset_installs`→`broker/fleets`, `records`→`broker/mod`, `render`→`output/mod`, `keys`→`multiplexer/mod` <!-- completed: -->
- [ ] Merge `presentation`→`output/json`, `formatters`+`output/runtime`→`output/text` <!-- completed: -->
- [ ] Merge `helpers`→`cli/mod`, `spawn_prompt`+`capture`→`cli/member`, `listen`→`cli/message`, `doctor`→`cli/setup`, `deletion`→`cli/fleet` (+ `active_member`→`broker/members`) <!-- completed: -->
- [ ] Merge `receive`→`delivery` <!-- completed: -->
- [ ] Split `input.rs` into `time.rs` (`Deadline`, clocks, `signals`), `db.rs` (budget) and `pane_lock.rs`; move `db/mod.rs` + `diagnosis` to `db.rs` <!-- completed: -->
- [ ] Fold `lib.rs` into `main.rs`; change `cafleet-test` to `cargo test --bin cafleet`; update `.claude/rules/code-quality.md` <!-- completed: -->
- [ ] Merge `CafleetError::Value` into `App` crate-wide <!-- completed: -->
- [ ] Add `CreateArgs`, `available_mux`, `db::open_existing`, `active_member`, the pinned-fleet check and the `merge_cleanup` reuse <!-- completed: -->
- [ ] Move free `#[cfg(test)]` items into `test_support` / `#[cfg(test)] impl` blocks <!-- completed: -->
- [ ] Confirm the core tree matches § Phase 8 (25 files); run the three validation tasks and the scratch setup check; open the Phase 8 PR <!-- completed: -->

### Step 9: Phase 9 — Runtime source merges

- [ ] Flatten `runtime/runner/` into `runtime/` with the § Phase 9 merges <!-- completed: -->
- [ ] Rename `ServiceControlRequest::Legacy` → `Lifecycle` and `decode_legacy_control_body` → `decode_lifecycle_control_body`; move `ObservationHealth` and `NativeLaunchHost` <!-- completed: -->
- [ ] Merge `coding_agent/` into `mod.rs`, `catalog.rs`, `detector.rs`; move `bundled.json` and `native/` up one level and update the `include_*!` paths and rule path <!-- completed: -->
- [ ] Merge `broker/runtime/` into `mod.rs`, `events.rs`, `inputs.rs`, `evidence.rs` <!-- completed: -->
- [ ] Apply R1–R7 and R11 <!-- completed: -->
- [ ] Confirm 46 tracked `.rs` files under `cafleet/src/`; run the three validation tasks and the scratch setup check; open the Phase 9 PR <!-- completed: -->

### Step 10: Phase 10 — Local clutter cleanup (user, after the execute fleet is deleted)

- [ ] Confirm `pgrep -f 'cafleet __member-run'` and `pgrep -f 'cafleet message listen'` print nothing <!-- completed: -->
- [ ] Stop every leftover `cafleet-*` tmux server with `tmux -L <name> kill-server` and confirm none remain <!-- completed: -->
- [ ] Delete every path in the § Phase 10 delete table <!-- completed: -->
- [ ] Confirm `git status --short` is unchanged by the cleanup <!-- completed: -->

### Step 11: Finalize (user, after the execute fleet is deleted)

- [ ] Re-check every Success Criteria item <!-- completed: -->
- [ ] Mark this design doc Complete and open the `docs: mark design 0000206 complete` PR <!-- completed: -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-11-01 | Initial draft |
