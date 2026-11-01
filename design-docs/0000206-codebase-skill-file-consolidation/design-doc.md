# Codebase, skill and repository file consolidation

**Status**: Complete
**Progress**: 92/92 tasks complete
**Last Updated**: 2026-11-01

## Overview

Consolidate the CAFleet repository to fewer, cohesive files and delete dead code and dead documentation: remove the explicitly-unwrapped member transport, the v1 event-schema and v1 preview-digest compatibility paths, and the `cafleet-design-doc` interview workflow; merge skill, source, fixture, rule and docs files while retaining mise monorepo task ownership; untrack non-`design-doc.md` design artifacts. The source changes are published as ordered, independently verified phases on PR #541. The design is complete after user-confirmed manual acceptance; final design-document publication remains user-owned.

## Success Criteria

- [x] `git ls-files design-docs` lists only `design-docs/*/design-doc.md` paths, and a new non-`design-doc.md` file under `design-docs/<NNNNNNN-slug>/` is ignored by `.gitignore`.
- [x] `skills/cafleet-design-doc/interview/` does not exist, and both `grep -rniE 'interview|analyzer|question\.md' skills/ docs/docs/ .claude/ presets/ README.md SPEC.md` and `grep -rn 'numbered question list' skills/` return nothing.
- [x] `skills/cafleet/` contains exactly five files: `SKILL.md`, `roles/director.md`, `roles/member.md`, `reference/cli.md`, `reference/coding-agents.md`; `wc -w` over them totals at most 19,000 words.
- [x] `grep -rni unwrapped SPEC.md skills/ docs/docs/ presets/ cafleet/src/` returns only the hits listed in § Phase 5 *Retained hits*.
- [x] `grep -rn payload_digest_version SPEC.md skills/ cafleet/` returns nothing; the runtime event reader accepts only `schema_version` `2`.
- [x] The native screen fixtures are exactly `cafleet/src/coding_agent/native/{claude,codex,opencode}.json` plus `cafleet/src/coding_agent/bundled.json`, and the bundled catalog loads with every fixture verified.
- [x] `git ls-files 'cafleet/src/*.rs' | wc -l` is 46 (25 core + 21 runtime), with the module trees of § Phase 8 and § Phase 9, and `cafleet/src/lib.rs` is gone.
- [x] The repository keeps mise monorepo mode with root `mise.toml` and package-local task files selected by `monorepo.config_roots`; the consolidated rules/docs layout remains unchanged.
- [x] Every completed phase has successful registered Rust test/lint and documentation-build receipts, attributable to its actual task names and source/artifacts.
- [x] The local clutter set in § Phase 10 is deleted from the working tree.

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
| DEC-3 | Remove `payload_digest_version` entirely (column, CHECK, JSON field, code) by editing the unreleased `V1__baseline.sql` in place. One ordinary-input digest covers preview, prompt and exec; permission responses retain their decision/audit metadata digest. | The digest version is a pure function of kind in v9. Editing the baseline is allowed until the family's first release (new sentence in the database-migrations rule). |
| DEC-4 | Native fixtures: one JSON file per backend plus `bundled.json` (4 files). | −111 tracked files with per-backend review diffs. |
| DEC-5 | Only `design-doc.md` is tracked under `design-docs/`; other artifacts are untracked with `git rm --cached` (local copies stay). | −280 tracked files; agent audit artifacts are local working material. |
| DEC-6 | Delete the interview workflow, including the "validate / review an existing design doc" dispatch intent, with no rerouting. The create workflow's own existing-document handling (resume and review-only modes) stays. | User decision. |
| DEC-7 | `skills/cafleet/` 9 → 5 files with the content trims of the skill audit; `roles/member.md` stays a separate file. | Fewer reads with role-file symmetry preserved. |
| DEC-8 | Fold `lib.rs` into `main.rs` (bin-only crate). | Required for the 25-file core target. |
| DEC-9 | Retain mise monorepo mode, root tool versions and package-local task files; invoke tasks through monorepo full paths. | User explicitly retained the original monorepo configuration after the initial consolidation. Preserve the current binary test target and pinned tool versions. |
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

- Phases 3–9 accumulate on [PR #541](https://github.com/himkt/cafleet/pull/541), using branch `refactor/0000206-phase-3-repository-file-merges` based on main at `a2e9ab0f` after Phase 2. Keep phase-ordered commits and complete each phase's validation and fresh independent review before pushing its changes. The user authorized all remaining implementation changes on this existing PR. Update its title and body to describe the cumulative implemented scope and preserve required migration notes. The execute team implements Steps 1–9. After its fleet is deleted, the user runs Phase 10's local cleanup and Step 11's final design-document completion.
- The Phase 1 source PR has merged. During this execution, update each completed step, Progress and Last Updated locally; the session Git policy keeps design-doc edits out of agent commits. Source/configuration changes land in their phase PRs. The user owns the final design-document PR in Step 11.
- Each phase follows the project documentation order: `SPEC.md` → `docs/` → `README.md` → `skills/` → `.claude/` → code.
- Each phase ends with the registered Rust test/lint and documentation-build tasks passing. Current invocations use `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build`; completed historical receipts retain their original task names.
- Phases 2–9 run the user-approved **scoped scratch setup check**, preserving the operator's installed CLI, configuration and database:
  1. Build the working tree through the registered release-build task, currently `mise //cafleet:build`. The artifact is `cafleet/target/release/cafleet`.
  2. Create `<BASE>/.setup-home/phase-<N>` as the phase's dedicated scratch directory; let `<S>` be its absolute path. Preserve `HOME` and `CODEX_HOME`.
  3. Select Claude and explicitly isolate its configuration and database:

     ```bash
     CAFLEET_DATABASE_URL=sqlite:///<S>/.local/share/cafleet/cafleet_v9.db CLAUDE_CONFIG_DIR=<S>/.claude cafleet/target/release/cafleet setup --coding-agent claude
     ```

     Run `doctor --json` with the same explicit configuration/database overrides and the actual multiplexer context. The explicit database URL selects the scratch file independently of the operator's environment.
  4. Require setup and doctor to exit 0, doctor to report the scratch Claude installation current and schema compatible, the installed skill file set and bytes to match the working tree, and the explicit URL plus read-only SQLite inspection to establish the scratch database path and integrity.
  5. Run the applicable installation tests that iterate every registered backend and compare embedded/installed skill and preset bytes. Verify changed preset allow keys against the current task registry and changed source assets against the compiled artifact. Report the scoped CLI and all-backend unit evidence at their actual boundaries.
  6. Retain every phase-specific test, invariant, acceptance observation and fresh independent review. Phase 2 additionally seeds an obsolete workflow file and verifies setup prunes its directory.
- Tests that only exercise removed behavior are deleted outright. No test asserts that a removed path is rejected.
- Contract strings that survive a move are preserved verbatim.
- No phase installs, upgrades or reconfigures the operator's CLI or database. The active CLI is the Homebrew release (`/opt/homebrew/bin/cafleet`, `0.30.2`), and the operator's `CAFLEET_DATABASE_URL` points at a `cafleet_v8.db`. Every in-phase check calls the working-tree build by full path (`cafleet/target/debug/cafleet` or `cafleet/target/release/cafleet`) with its phase-approved scratch configuration and database settings. Upgrading the installed CLI and its database is a user step after the release (§ Post-release user steps).
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

#### Mise monorepo configuration

The root `mise.toml` keeps `monorepo_root = true`, the registered package directories in `[monorepo].config_roots`, the current pinned tool versions, and repository-wide tasks. Package-local `mise.toml` files own their tasks and execute from their respective package directories.

Restore the pre-consolidation task ownership from `d33623b9^`, preserving current behavior:

- `cafleet/mise.toml` owns the Cargo tasks. Its `test` task runs `cargo test --bin cafleet`, retaining the current binary-owned unit harness and positional argument forwarding.
- `docs/mise.toml` owns the pnpm tasks and uses the existing documentation package/lockfile.
- Root `all-install` delegates to `//cafleet:install` and `//:skill-install`; its skill installation behavior remains unchanged.

Use `mise tasks --all` as the task registry. Every live command reference uses its monorepo full path, including CI, agent permission maps, rules, documentation and examples. Preserve permission scope by mapping each existing allowed task to its corresponding full path; retain all unrelated allow/deny rules.

| Surface | Required behavior |
|---|---|
| Rust CI | Select the package task through `mise //cafleet:` and the existing matrix task name. |
| Documentation CI | Invoke `mise //docs:install` and `mise //docs:build`. |
| Agent permission settings and OpenCode presets | Allow corresponding full-path task commands with the same argument and deny boundaries. |
| Contributor and agent instructions | Discover tasks through the complete registry; show qualified Rust, docs and root-task invocations. |

Verify task discovery from the repository and package directories, the root installation delegation without running installation, actual binary-test argument forwarding, full Rust tests/lint, docs and release builds. Validate the changed OpenCode permission keys, all-backend installation tests and exact embedded/installed preset bytes. Use owned scratch resources for checks and preserve the operator's installed CLI, configuration and database.

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

The original skill-audit artifact is absent from the retained workspace. Record a section-by-section inventory of the current CLI reference under the execution BASE and apply the explicit preservation/drop requirements below to every section. This regenerated inventory supplies the executable trim checklist: retain each invocation form, required field, disposition token, evidence rule, exit semantic and exact error/diagnostic string, and account for every collapsed or moved section.

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
| `reference/cli.md` | Apply the regenerated section-by-section inventory and these explicit requirements. Keep invocation forms, the field names the Director reads, disposition tokens, the evidence-ref requirement, the `uncertain_cause` table, exit semantics and every exact error/diagnostic string. Drop key-order dumps, text layouts, lock budgets, the tmux/herdr argument mapping, the 15-row blocked-reason table (one sentence remains) and runner queue internals. |
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
| `src/delivery.rs` | Delete the unwrapped preview drain (lines 194–326), `notification_line`, `storage_failure`, and the obsolete multiplexer-resolver parameter/arguments of `deliver_pending_with`; preserve its injected input host. Keep `fallback_preview_line` and `DISMISSED_PROMPT_NOTE` (the runner's wrapped fallback uses them). |
| `src/cli/member.rs` | Delete the unwrapped `member prompt` / `member exec` branch (742–797) and its help text (143–144); both commands call `require_runtime`. |
| `src/cli/message.rs` | Delete the `DeliveryMethod::Unwrapped` projection (237–247). |
| `broker/runtime` | Delete the `DeliveryMethod::Unwrapped` variant and its `unwrapped` wire value. |
| `src/input.rs` | Delete `lock_member` (the unwrapped prompt/exec lock). `finalize_deletion` keeps its run-less branch for teardown; its `wrapped` parameter is renamed `has_run`. |
| `src/deletion.rs` | Keep the `snapshot.runtime.is_some()` selector (273), passed as `has_run`. |
| `src/runtime/runner/recovery.rs` | Line 173 uses `crate::broker::members::missing_run(member_id)`. |
| `src/multiplexer/{mod,tmux,herdr}.rs` | Delete `SUBMIT_DELAY`, `ESCAPE_*`, `MIN_LITERAL_BUDGET`, `REVALIDATION_MARGIN`, `*_RESERVATION`, `INPUT_RESERVATION`, `INSUFFICIENT_INPUT_BUDGET`, `Lead`, `PaneInput`, `BackendCore::send_prompt`, `submit_prompt`, `AnyMultiplexer::send_prompt`, and the backend `send_prompt` argv builders. |
| `src/broker/members.rs` | Add `missing_run` and `require_runtime`. |
| `src/broker/messaging.rs` | Delete the now-unreferenced direct-drain helper `stamp_notified_validated`; retain atomic runner settlement/stamping. |
| `src/receive.rs`, `src/cli/message.rs` | Delete the direct-drain-only `AttemptEnd::Drained` / `BudgetExhausted` variants and obsolete projection arm; adapt retained fake outcomes to `NoPending`. |
| `src/multiplexer/mod.rs`, `src/runtime/system.rs` | Delete engine-only `CommandRunner::sleep(f64)`, its implementations and unused `RunErrorMapper`; remove fake Sleep events/methods from test support. Preserve receiver `Clock::sleep(Duration)` and general process/deadline behavior. |

Tests:

| Test | Action |
|---|---|
| Unwrapped-only drains, Escape argv order and budget diagnostics: `delivery.rs` ~1524–2318, `multiplexer/mod.rs` 1156–1236 and 1330–1445, tmux ~400–440, herdr ~780–800, `input.rs` `lock_member` tests | Delete |
| `presentation.rs` runtime-null assertion | Keep the root Director fixture's `runtime: null` assertion and rename its local `unwrapped` binding to `director`; this verifies retained root behavior. |
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

One digest for ordinary input kinds (`preview`, `prompt` and `exec`). Permission responses retain `permission_response_digest` and its validated request/decision/audit metadata binding unchanged:

```json
{"run_id":42,"kind":"preview","message_id":99,"literal":"CANONICAL_BASE_PREVIEW"}
```

SHA-256 over `digest::canonical_json` of that object. For previews, `literal` is the canonical base preview (method-independent wrapped line ending at the resume clause); for prompt and exec, it is the submitted literal, exactly as `input_payload_digest` computes today. Removing the shared digest-version column and record field applies to every input record; it leaves the permission-response digest preimage and immutable audit validation unchanged.

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
- Runtime-scope files change only their `use` paths and their `CafleetError::Value` → `App` construction sites. The production sites include `coding_agent/profiles/composer.rs:73`, `broker/runtime/inputs.rs:399`, `broker/runtime/mod.rs:191` and `broker/runtime/events.rs:108`; test-owned sites include `coding_agent/policy.rs:154`, the `#[cfg(test)] AnswerCapability::fixture` in `coding_agent/profiles.rs:57`, and the sites in `runner/input_client.rs`, `process.rs` and `transport/drain.rs`. `grep -rn 'CafleetError::Value' cafleet/src` returns nothing afterward.

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

Data files move with the catalog: `coding_agent/profiles/bundled.json` → `coding_agent/bundled.json`, `coding_agent/profiles/native/` → `coding_agent/native/`. Update the `include_*!` paths, the fixture-source paths in SPEC introduced in Phase 7, and the `.claude/rules/code-quality.md` path.

In-scope dedupes, with error strings preserved:

| ID | Change |
|---|---|
| R1 | `InputOperation::same_identity` replaces 4 hand-written identity predicates. |
| R2 | One `revalidate_run_snapshot` replaces the 3 run+placement checks. |
| R3 | One `placement_of(conn, member_id)` replaces the duplicate direct placement SELECTs. |
| R4 | `reserve_event_sequences_in_transaction` shared by exceptional recovery and `reserve_event_sequences`. |
| R5 | One `delivery_result_transition` replaces 2 literals. |
| R6 | `validate_database` replaces the inline DB-identity checks. |
| R7 | `WriterHooks`/`InputHooks` and their fields become `#[cfg(test)]`; delete `InertWriterHooks`/`InertInputHooks`. |
| R11 | One `crate::error::Result` alias replaces 3 local aliases. |

R1 uses the existing complete immutable identity predicate as the source of truth: input_id, run_id, operation_key, kind, payload_digest, message_id, created_at and permission_response. The shared method compares those fields at every identity fence, including the receipt cache; caller-specific run/state checks stay at their current boundary. A focused cache entrypoint regression covers a changed creation stamp before implementation.

R3 covers the two identical direct six-column placement reads remaining in recovery and receipt recovery. Keep the permission placement path's full get_member/live-fleet policy and every caller's existing admission/errors; it is outside the duplicate direct-query extraction.

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

- [x] Add the two `design-docs` ignore lines to `.gitignore` <!-- completed: 2026-11-01T02:43 -->
- [x] Run the `git ls-files --cached --ignored … | xargs -0 git rm --cached` command and confirm `git ls-files design-docs | grep -v '/design-doc.md$'` is empty <!-- completed: 2026-11-01T02:43 -->
- [x] Confirm a new `design-docs/<existing>/scratch.txt` shows as ignored (`git check-ignore`) and `design-doc.md` does not <!-- completed: 2026-11-01T02:59 -->
- [x] Rewrite `.claude/rules/git-workflow.md` with the tracking policy and fix the run-on bullet <!-- completed: 2026-11-01T02:43 -->
- [x] Confirm this design doc is already tracked; keep subsequent design-doc edits local under the session Git policy <!-- completed: 2026-11-01T02:59 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //docs:build`; open the Phase 1 PR <!-- completed: 2026-11-01T02:43 -->

### Step 2: Phase 2 — Remove the interview workflow

- [x] Edit `docs/docs/how-to/design-doc-development.md` per § Phase 2 <!-- completed: 2026-11-01T02:59 -->
- [x] Edit `skills/cafleet-design-doc/SKILL.md` (description, workflow wording, Dispatch row) <!-- completed: 2026-11-01T02:59 -->
- [x] Edit `reference/coordination.md` (exemption sentence, row, Analyzer paragraph) and `reference/guidelines.md` (lines 80, 88) <!-- completed: 2026-11-01T02:59 -->
- [x] Edit `create/roles/drafter.md` Resume Mode trigger and `create/create.md` Step 0 <!-- completed: 2026-11-01T02:59 -->
- [x] Delete the Analyzer START CUE example in `skills/cafleet/roles/director.md` <!-- completed: 2026-11-01T02:59 -->
- [x] Delete `skills/cafleet-design-doc/interview/` <!-- completed: 2026-11-01T03:01 -->
- [x] Run both interview greps from Success Criteria, the three validation tasks and the approved Phase 2 scoped scratch check; open the Phase 2 PR <!-- completed: 2026-11-01T07:01 -->

### Step 3: Phase 3 — Repository file merges

- [x] Write the root `mise.toml`; delete `cafleet/mise.toml` and `docs/mise.toml` <!-- completed: 2026-11-01T07:16 -->
- [x] Rename task references in SPEC, CI workflows, `.claude/settings.json`, both OpenCode presets, rules, `docs/docs/contributing.md` and `mixed-backend-team.md` <!-- completed: 2026-11-01T07:16 -->
- [x] Verify `mise tasks`, `mise cafleet-test -- -- --list` and `mise docs-build`; `grep -rn -e '//cafleet:' -e '//docs:' -e 'mise //:' . --exclude-dir=.git --exclude-dir=design-docs --exclude-dir=target --exclude-dir=node_modules --exclude-dir=doc_build` returns nothing <!-- completed: 2026-11-01T07:19 -->
- [x] Merge `docs/docs/concepts/member-lifecycle.md` and `member-permission-events.md` into `concepts/members.md`; update `concepts/_meta.json` <!-- completed: 2026-11-01T07:16 -->
- [x] Merge `docs/docs/how-to/design-doc-development.md` into `concepts/overview.md` keeping `{#prompts}` <!-- completed: 2026-11-01T07:16 -->
- [x] Move `how-to/mixed-backend-team.md` to `docs/docs/`, delete `how-to/_meta.json`, update `docs/docs/_meta.json` <!-- completed: 2026-11-01T07:16 -->
- [x] Rewrite docs links and the `README.md` bullets <!-- completed: 2026-11-01T07:16 -->
- [x] Delete `CONTRIBUTING.md` <!-- completed: 2026-11-01T07:16 -->
- [x] Merge `.claude/rules/bash-tool.md` into `commands.md`, `coding-agent-overlay.md` into `documentation-maintenance.md`, `database-migrations.md` into `code-quality.md`; rewrite every link to them <!-- completed: 2026-11-01T07:19 -->
- [x] Merge `.claude/skills/clean-docs` into `SKILL.md` + `roles.md`; update spawn deltas and IMPORTANT lines; remove the duplicated workflow text <!-- completed: 2026-11-01T07:19 -->
- [x] Run `mise cafleet-test`, `mise cafleet-lint`, `mise docs-build` (link check) and the approved Phase 3 scoped scratch check (confirm updated OpenCode preset allow keys and embedded/installed bytes through the existing all-backend installation tests); open the Phase 3 PR <!-- completed: 2026-11-01T09:12 -->

### Step 4: Phase 4 — `skills/cafleet` 9 → 5

- [x] Merge `reference/supervision.md` into `roles/director.md` § Supervision per the merge map <!-- completed: 2026-11-01T09:42 -->
- [x] Merge `reference/member-runtime.md` into `roles/director.md` § Runtime events, Member Press and Recovery <!-- completed: 2026-11-01T09:42 -->
- [x] Merge `reference/prompt-routing.md` into `roles/director.md` § Director-side dispatch and `roles/member.md` § Reconsider, then route <!-- completed: 2026-11-01T09:42 -->
- [x] Merge `reference/base-dir.md` into `roles/member.md` § BASE, `roles/director.md` § Base directory resolution, and `cafleet-design-doc/reference/guidelines.md` § Task-scope resolution <!-- completed: 2026-11-01T09:42 -->
- [x] Move § Listen from `SKILL.md` into `roles/director.md` § Director wake <!-- completed: 2026-11-01T09:42 -->
- [x] Delete the four reference files and rewrite every inbound link per the old → new target table, including the `#reconsider-then-route` anchor change; the deleted-file grep returns nothing <!-- completed: 2026-11-01T09:42 -->
- [x] Rewrite the `prompt-routing` sentence in `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` <!-- completed: 2026-11-01T09:42 -->
- [x] Apply the `SKILL.md` trims <!-- completed: 2026-11-01T09:42 -->
- [x] Apply the `roles/member.md` trims <!-- completed: 2026-11-01T09:42 -->
- [x] Apply the `roles/director.md` model-selection and skeleton trims <!-- completed: 2026-11-01T09:42 -->
- [x] Apply the `reference/cli.md` trims with the regenerated preservation inventory <!-- completed: 2026-11-01T09:42 -->
- [x] Apply the `reference/coding-agents.md` trims and move the maintainer content into `.claude/rules/documentation-maintenance.md` <!-- completed: 2026-11-01T09:42 -->
- [x] Update `.claude/skills/cafleet-model-list-refresh/SKILL.md` for the removed tables and wording <!-- completed: 2026-11-01T09:42 -->
- [x] Confirm `skills/cafleet` has 5 files totalling ≤ 19,700 words and the SPEC-linked coding-agents anchors exist <!-- completed: 2026-11-01T09:42 -->
- [x] Run `mise cafleet-test`, `mise cafleet-lint`, `mise docs-build` and the scratch setup check; push reviewed Phase 4 changes to PR #541 <!-- completed: 2026-11-01T10:04 -->

### Step 5: Phase 5 — Remove the explicitly-unwrapped member path



- [x] Edit SPEC per § Phase 5 SPEC edits and the run-less behavior paragraph (failure-cause row, prompt/exec error, run-less teardown) <!-- completed: 2026-11-01T10:09 -->
- [x] Edit `docs/docs/concepts/members.md`, `docs/docs/concepts/coding-agents.md` and `docs/docs/mixed-backend-team.md` per § Phase 5 docs table <!-- completed: 2026-11-01T10:09 -->
- [x] Remove the remaining unwrapped mentions from `skills/cafleet` <!-- completed: 2026-11-01T10:09 -->
- [x] Add `missing_run` / `require_runtime`; wire `member prompt`, `member exec`, the preview drain and `recovery.rs:173` per the caller table <!-- completed: 2026-11-01T11:05 -->
- [x] Delete the unwrapped drain and helpers in `src/delivery.rs` <!-- completed: 2026-11-01T11:05 -->
- [x] Delete the unwrapped prompt/exec branch and help text in `src/cli/member.rs` and the projection in `src/cli/message.rs` <!-- completed: 2026-11-01T11:05 -->
- [x] Delete `DeliveryMethod::Unwrapped` and `lock_member`; rename the `finalize_deletion` parameter to `has_run`, keeping run-less teardown <!-- completed: 2026-11-01T11:05 -->
- [x] Delete the Escape-first multiplexer engine from `multiplexer/{mod,tmux,herdr}.rs` <!-- completed: 2026-11-01T11:05 -->
- [x] Apply the Phase 5 test table (delete, rewrite, rename-and-keep, new tests) <!-- completed: 2026-11-01T10:26 -->
- [x] Confirm `skills/cafleet` totals ≤ 19,000 words <!-- completed: 2026-11-01T10:09 -->
- [x] Run the case-insensitive unwrapped grep (only § Retained hits remain), the three validation tasks and the scratch setup check; push reviewed Phase 5 changes to PR #541 <!-- completed: 2026-11-01T11:30 -->

### Step 6: Phase 6 — Remove v1 event-schema and preview-digest compatibility

COMMENT(reviewer): Approved (doc), reported by Reviewer 56 in broker message 451. Independently reviewed the complete design and current decisions, the entire cumulative main...HEAD diff including deletions from a2e9ab0faee0dc05f0c01bae11dc8e267bbf34dc to b9d846d179e93e938063dac6721f120d5800a88c, and the Phase 6 increment from c473a2e45cb9c98a3a545d036a474e9ffbd772ac. No substantive findings remain for implemented Phases 3–6. The schema-2 reader retains schema_version and independent versioned domains; shared digest-version removal is complete, ordinary canonical hashing/reuse/rendering and duplicate validation remain intact, and permission-response metadata hashing/audit/claims retain their contract. Evidence reviewed: [.checks/phase-6.md](.checks/phase-6.md), [digest TDD handoff](.checks/phase-6-digest-tdd.md) and its native run records, [implementation validation](.checks/phase-6-programmer-validation.md), [final verifier results](.checks/phase-6-verifier-results.md), [frozen identities](.checks/phase-6-verifier-freeze.json), and [final CLI/build/database receipts](.checks/phase-6-verifier/result.json). Independently matched 256 recorded source/artifact/log hashes and 72 protected operator CLI/configuration/asset records; confirmed exact installed path sets and bytes, all 18 skill/preset assets embedded in both tested binaries, the five-file/18,999-word skill boundary, and read-only scratch database identity/schema/integrity with unchanged bytes. The operator database inode/application/instance identity also matches. Re-ran the frozen unit artifact's event group (67), permission group (31), method_rendering_ cases (2), and installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths (1): all 101 tests passed. Verified the recorded full suite (1,478 passed, 8 ignored), 215 focused cases, lint/format and docs/release builds; seven behavioral assertion failures remain separately attributed from the ten-error interface compile gap. Removal/link searches and git diff --check pass; retained schema-1 hits belong to independent policy/evidence domains. Both queued CLI retries and final stored input rows match, with zero multiplexer calls. Real CLI setup/doctor covers scoped Claude configuration/database paths; seeded CLI reads/history reuse and all-backend installation/permission unit evidence retain their separate scopes, with HOME/CODEX_HOME preserved. Latest release 0.30.2, the empty containing-tag set and matching 53 remote/local tag records support the unreleased-baseline policy. The prepared [.pr/phase-6.md](.pr/phase-6.md) accurately instructs stopping users of an earlier-main cafleet_v9.db, deleting that local file and running setup. Publication remains Director-owned; later-phase preparation is outside this approval. Review changed no code or Git state.

COMMENT(director): Reviewer56 publication context: remote PR541 intentionally still contains published Phase5 head c473a2e4 while Phase6 review gates push/body update. The concrete prepared cumulative Phase6 title/body material is BASE/.pr/phase-6.md; review its pre-release cafleet_v9.db cleanup/setup instruction and accurate validation scopes. The existing remote Phase5 description is not the proposed Phase6 publication. User authority already covers normal push and this same PR for all remaining phases; no merge wait or renewed publication approval is needed.

COMMENT(director): Reviewer 56: ready 443 is consumed/ACKed. Begin fresh independent review on b9d846d179e93e938063dac6721f120d5800a88c: full cumulative main...HEAD against a2e9ab0faee0dc05f0c01bae11dc8e267bbf34dc and incremental Phase 6 against c473a2e45cb9c98a3a545d036a474e9ffbd772ac. Read the complete design/current annotations, entire branch diff/deletions and actual linked .checks/phase-6.md, digest TDD/freeze/validation, final verifier results/receipts and remote tag proof. All implemented Phases 3–6 criteria/checks pass; later phases are scratch-only preparation, not implemented branch scope. Independently assess schema-2 reader, total shared discriminator/selector removal, ordinary canonical preimage/reuse/rendering/no-retyping, unchanged permission metadata/audit/claim behavior, exact current shape/diagnosis, unreleased-baseline policy/lineage and required PR cleanup note. Verify genuine-red vs interface-gap evidence, preserved tests/independent schemas, five-file18999-word limit, source/operator/binary identities and truthful direct CLI/seeded fixture/unit boundaries. Coordinate any expensive repeats; source is frozen. Report actionable findings with paired markers or approve after independent assessment.

COMMENT(director): Final Phase 6 operational completion 440 is consumed/ACKed and accepted after full report/receipt review. Frozen source b9d846d179e93e938063dac6721f120d5800a88c passes full1478/ignored8,215 focused cases,lint/format,docs/release builds and all eleven scoped CLI cases. Exact installed/embedded bytes, current field-free input schema, read-only DB identity/integrity, repeated queued receipt reuse with zero pane calls, permission metadata hashing and operator/HOME/CODEX_HOME preservation pass. The final phase task remains open only for fresh independent review/publication. Completed coordination is archived; actual evidence is linked from .checks/phase-6.md and phase-6-verifier-results.md. Live source stays frozen while Phase 7 scratch rebase/planning proceeds.

- [x] Edit SPEC 2577, 2621, 3012 (event schema) <!-- completed: 2026-11-01T11:32 -->
- [x] Edit SPEC 823, 2575, 2955–2964, 3124, 3348–3349 (digest) <!-- completed: 2026-11-01T12:08 -->
- [x] Remove digest-version mentions from `skills/cafleet/reference/cli.md` <!-- completed: 2026-11-01T12:08 -->
- [x] Add the editable-baseline sentence to `.claude/rules/code-quality.md` § Database migrations <!-- completed: 2026-11-01T11:32 -->
- [x] Make the event reader require version 2; delete the v1 helper and tests in `events.rs` and `recovery/replay.rs` <!-- completed: 2026-11-01T11:45 -->
- [x] Delete the `payload_digest_version` column from `V1__baseline.sql` <!-- completed: 2026-11-01T12:39 -->
- [x] Collapse the digest code to `input_payload_digest` and remove the field from records, runner input, input client, presentation, delivery and diagnosis <!-- completed: 2026-11-01T12:39 -->
- [x] Rewrite the preview-receipt reuse tests to the single preimage; delete the version-selection-only tests; confirm the `payload_digest_version` grep is empty <!-- completed: 2026-11-01T12:39 -->
- [x] Run the three validation tasks and the scratch setup check; push reviewed Phase 6 changes to PR #541 and in its body, tell the reader to delete any local `cafleet_v9.db` created by an earlier main-branch build before running `cafleet setup` <!-- completed: 2026-11-01T13:18 -->

### Step 7: Phase 7 — Native fixtures per backend

COMMENT(reviewer): [COMPLIANCE] APPROVED by Reviewer58 in message505 for frozen02f5719fddfdb55f3ac237cb9629aae8345256f2: cumulative Phases3–7 from maina2e9ab0faee0dc05f0c01bae11dc8e267bbf34dc and incremental Phase7 from b9d846d179e93e938063dac6721f120d5800a88c. Read the complete design, resolved decisions, handoffs and branch diff including deletions. No substantive finding remains. Independently compared all114 original Git captures and Director backup bytes with converted screens after exactly the five specified identity fields were removed; verified canonical digests, serialization/order, all catalog identities and unchanged profile citations, including all22 profile-uncited captures. Inspected the scratch converter, original-surface execution receipt00a937, complete loader membership/exact errors and retained geometry/cursor/lines/mask/background checks. Complete-file inverse comparisons preserve detector assertions/replay and establish that input.rs changes only17 borrowed helper calls; all original owner test identities remain. Evidence: .checks/phase-7.md, .checks/phase-7-stage1-red.json, .checks/phase-7-stage2-tdd.md, .checks/phase-7-director-conversion-execution.json, .checks/phase-7-director-conversion-result.json and .checks/phase-7-director-input-borrow-proof.json.

COMMENT(reviewer): [COMPLIANCE] Approval evidence and limits: independent frozen-unit executions passed9 new loader cases and76 detector cases. Reused matching catalog125/full1487-passed/8-ignored and lint/format receipts in .checks/phase-7-programmer-runs.json and .checks/phase-7-final-validation.json; verified actual docs/release terminal receipts and complete logs in .checks/phase-7-verifier/build-evidence.json. Independently matched all23 complete asset/native/catalog byte sequences in both binaries, exact5+11 installed skill trees, scratch database identity/schema/integrity and139 frozen source inputs against .checks/phase-7-verifier/embedding.json, result.json and freeze.json. Release SHA-256:5b496ba6d6688e8a02e0ff8a0ea26af0123b31af9b38a32c11d855bff3161e47; unit SHA-256:65074107ecb0881469c08d1b6300a3c8d0206ec56c74303430cded96d1c781e9. Actual scoped cases and .checks/phase-7-verifier-results.md establish real Claude setup/doctor with scratch configuration/database and unchanged HOME/CODEX_HOME; all-backend installation is separate unit evidence. Operator CLI/configuration/assets and broker database identity/schema remain preserved. Independent checks confirmed243 concrete local references,116 unchanged model rows,18,999 skill words and removal cleanup. The proposed .pr/phase-7.md accurately records validation scopes and required earlier-main v9 database cleanup. Stage1's8 assertion failures remain separate from draft compile errors and stage2's old-asset gap. Reviewer read-only token/normalization drafts failed before corrected exact equality passed; those audit-draft failures are separate from product findings. The optional additional catalog125 run has no captured terminal exit and is excluded from acceptance; process inspection found no remaining child, and no rerun occurred (messages503/506/507). Phase8 preparation remains outside implemented review scope. Recorded under Director request508 without new checks or source/Git changes.

COMMENT(director): Phase7 final review505/507 is consumed/ACKed and accepted; no substantive issue, independent9+76 pass, optional uncaptured125 excluded with no live child, required actual125/full/lint/build proofs retained. Normal push and prepared PR body update succeeded; GitHub PR541 is OPEN at02f5719fddfdb55f3ac237cb9629aae8345256f2 with new-head CI started/build success. Final Phase7 task checked; Progress66/88. Reviewer58 is writing its durable approval marker before retirement. Phase8 reviewed/rebased task/rule replacements at .stage-phase-8-docs-final and accepted Tester plan are ready; next release follows final reviewer retirement. No merge wait or new user approval is required under existing authority.

COMMENT(director): Reviewer58 status checkpoint after independent full-source/data/unit and installed/DB checks: report a concise current status and any substantive finding/open required check with evidence pointers. Complete the independent assessment without optional overlapping execution expansion; preserve real reader/schema/metadata draft failures separately from source findings. Source02f5719f and accepted artifacts stay frozen; final disposition remains Reviewer-owned. No source fixes are released without an actual finding.

COMMENT(director): Final Phase7 Verifier496 is consumed/ACKed and accepted after complete actual report/result/build/DB/source/embedding review. Frozen02f5719f passes full1487/ignored8, focused9/125/76,lint/format,docs/release,all4scoped cases,114-screen conversion and both-artifact19asset+4native/catalog byte proofs. All139 verification source inputs/operator resources and current16-column DB shape are preserved. Optional inverse-ledger reader drafts are reconciled as substring/indentation counting errors; independent exact17-borrow reconstruction proves no input source discrepancy. Actual report: .checks/phase-7-verifier-results.md; no open acceptance finding. Verifier57 retired; fresh Reviewer58(run52,pane w2:p12,gpt-6-astra/high) spawned and waits ready. Review target is full cumulative maina2e9ab0f...02f5719f through Phases3–7 plus increment from b9d846d1, with full design/diff/deletions and all actual linked evidence. Prepared PR body .pr/phase-7.md now records actual passing gates and required pre-release DB cleanup; remote PR541 intentionally remains published Phase6 until fresh approval. No source/test/data changes; Progress65/88 and final publication task open. Phase8 rebase494 reviewed as prepared; live future work remains gated.

- [x] Edit SPEC 1979, 2180–2209 and 2195 for the per-backend file and verification rules <!-- completed: 2026-11-01T13:22 -->
- [x] Convert the 114 fixtures with a scratch script under BASE (equality assertion per screen) into `native/{claude,codex,opencode}.json`; recompute every `bundled.json` `sha256` and `source_ref` <!-- completed: 2026-11-01T14:17 -->
- [x] Replace the `bundled!` list with three `include_bytes!` constants and rewrite `verify_native_screen` with the new conditions and error strings <!-- completed: 2026-11-01T14:17 -->
- [x] Update `test_support::bundled_fixture` / `bundled_embedded` and every test caller <!-- completed: 2026-11-01T14:17 -->
- [x] Add unit tests for the missing-screen, uncited-screen, file-identity and digest-mismatch errors <!-- completed: 2026-11-01T14:17 -->
- [x] Delete the 114 per-fixture files; update the fixture path in `.claude/rules/code-quality.md` <!-- completed: 2026-11-01T14:17 -->
- [x] Run the three validation tasks and the scratch setup check; push reviewed Phase 7 changes to PR #541 <!-- completed: 2026-11-01T15:01 -->


### Step 8: Phase 8 — Core source merges

COMMENT(director): Final independent acceptance passed at b1c46901866e9a5c322f901641eed2f7e75077c8 under release609; .checks/phase-8-verifier-results.md and .checks/phase-8-verifier/result.json (da11c57b2c023a2808b5522c5fae341a2eb7d982838df1434e58dc396a99ca1c) bind final unit87eb95ca/release6fa8bfd1 to source. Full1490passed/8ignored, final lint/format, actual docs/release builds, scoped Claude setup/doctor/help/readers, exact embedded/installed assets, read-only database and operator preservation pass. Fresh independent Reviewer60 now owns complete cumulative main-base a2e9ab0faee0dc05f0c01bae11dc8e267bbf34dc...HEAD and incremental02f5719fddfdb55f3ac237cb9629aae8345256f2...b1c46901866e9a5c322f901641eed2f7e75077c8 review; source frozen. Publication and final Phase8 checkbox remain pending; Phase9 live changes stay gated.

COMMENT(director): Current authority is .checks/phase-8-current-scope.md. Tester578/581 freezes final1db4 artifacts; root116artifact/1499original-body+attribute/cfg/3442declaration/114syntax/8child checks accepted. Production576/577 complete ordered pack and static561 accepted. Programmer579 composes the exact88slots/25patches into a staged68-file candidate with guarded direct publication. Director candidate audit passed68files/1499exact bodies+attrs/cfg and syntax0; exact live assembly/binary-check authority is .checks/phase-8-director-live-release.json. Programmer integrates and validates; current source still643e996e at release. Verifier final checks, fresh review and publication follow; Progress75/88; final Phase8 verification/review/publication stays open.

- [x] Merge `config_dir`→`config`, `embedded`→`assets`, `panes`→`broker/messaging`, `asset_installs`→`broker/fleets`, `records`→`broker/mod`, `render`→`output/mod`, `keys`→`multiplexer/mod` <!-- completed: 2026-11-01T17:30 -->
- [x] Merge `presentation`→`output/json`, `formatters`+`output/runtime`→`output/text` <!-- completed: 2026-11-01T17:30 -->
- [x] Merge `helpers`→`cli/mod`, `spawn_prompt`+`capture`→`cli/member`, `listen`→`cli/message`, `doctor`→`cli/setup`, `deletion`→`cli/fleet` (+ `active_member`→`broker/members`) <!-- completed: 2026-11-01T17:30 -->
- [x] Merge `receive`→`delivery` <!-- completed: 2026-11-01T17:30 -->
- [x] Split `input.rs` into `time.rs` (`Deadline`, clocks, `signals`), `db.rs` (budget) and `pane_lock.rs`; move `db/mod.rs` + `diagnosis` to `db.rs` <!-- completed: 2026-11-01T17:30 -->
- [x] Fold `lib.rs` into `main.rs`; change `cafleet-test` to `cargo test --bin cafleet`; update `.claude/rules/code-quality.md` <!-- completed: 2026-11-01T17:30 -->
- [x] Merge `CafleetError::Value` into `App` crate-wide <!-- completed: 2026-11-01T17:30 -->
- [x] Add `CreateArgs`, `available_mux`, `db::open_existing`, `active_member`, the pinned-fleet check and the `merge_cleanup` reuse <!-- completed: 2026-11-01T17:30 -->
- [x] Move free `#[cfg(test)]` items into `test_support` / `#[cfg(test)] impl` blocks <!-- completed: 2026-11-01T17:30 -->
- [x] Confirm the core tree matches § Phase 8 (25 files); run the three validation tasks and the scratch setup check; push reviewed Phase 8 changes to PR #541 <!-- completed: 2026-11-01T17:58 -->

### Step 9: Phase 9 — Runtime source merges

COMMENT(director): Agent-owned Phases1-9 are complete. Phase9 source1a6aab8332273e6171c7165bb280e67e0ba838d6 passed final1492unit tests/0failed/8ignored, lint/format, docs/release builds, source/test preservation and independent semantic/operational acceptance817. Fresh Reviewer62 approved the full cumulative and incremental diff without substantive issues in823, including independent exact R1/R7 reruns and artifact/assets/database checks. Normal push published the exact approved head to existing PR541; title and final body match .pr/phase-9.md, including the required earlier-v9 database recreation note. .checks/phase-9-publication.json pins remote identity and body. GitHub lint/build/test all pass; deploy is skipped. Owned fleet6 was deleted, every remaining ordinary pane was killed, the listener closed with fleet_deleted and native exit0, and fleet list confirms absence. User10/11 may now proceed. The user subsequently confirmed the remaining acceptance; Progress88/88 and global design Complete. Final design-document publication remains user-owned.

COMMENT(director): Phase8 completed and published to existing PR541 at0fa60cde0b3a76e098f47ca8562367ab9da12ccc after Reviewer652/653 approval and documentation-only fix; CI running. Current Phase9 authority supersedes earlier preparation-only markers: .checks/phase-9-docs-first-receipt.json pins the complete original source/data snapshot and applies exactly two SPEC and one code-quality native-path replacements. Joint source/test plan639 accepted. Tester baseline-additions draft642 still requires Director review before live tests/red observation/commit. Production implementation waits for that Tester-first gate; final validation, independent acceptance/review and normal publication remain required. Progress76/88; user10/11 remain pending.

COMMENT(director): Joint source/test interface plan639 is accepted. Tester52 may draft only baseline-additions.patch plus a concise handoff under .stage-phase-9-tests/, against frozen b1c46901 input.rs, for the approved R1 current receipt-query creation-stamp regression and R7 injected-hook error/deadline/blocking preservation coverage. Existing body/attribute/helper bytes and all live source/data remain unchanged. Draft through existing entrypoints with meaningful side-effect/state assertions; identify original/new helper ownership and exact baseline hash. This preparation executes no build/test/product command and applies no live edit. Director review and Phase8 reviewed publication precede live tests/red observation/test commit, then production implementation.

COMMENT(director): Tester634 identified missing InputHooks-returned-error preservation coverage. Decisions635/636 approve a current-entrypoint injected hook-error/expired-deadline precedence and pending-Blocked regression that should pass on baseline, separately from R1 behavioral-red evidence. Accept cfg(test)-only default_writer_hooks/default_input_hooks, the normal four-input EventWriter::start plus conditional injected/common worker initialization, and one shared cfg control Endpoint/StartupInterval/bound_socket/independent_frame seam for the nine moved readiness cases. Keep retained checkpoint lookup, hook-before-deadline error precedence, original real deadline/blocking behavior and every production snapshot consumer. Programmer binding confirmation remains preparation-only; live writing waits for Phase8 reviewed publication.

COMMENT(director): Read-only interface decisions625/629 accept .checks/phase-9-source-plan.md R2 explicit Live/HistoricalReceipt policy plus exact message/query-order table; R3 runtime/artifacts::placement_of for the two direct reads; R4 SequenceReservation selecting the existing normal CAS versus recovery UPDATE/error modes inside the caller-owned transaction; R5 existing delivery tuple with caller time/evidence; R6 one pure value-level validate_database(actual,expected,mismatch factory) with all reads, categories and order retained by callers; and R11 one crate Result alias preserving genuine two-parameter/I/O results. Narrow ControlClient::connect_starting and nine readiness cases moving to lifecycle are accepted; exact shared test-helper and cfg-hook constructor seams await Tester coordination. These settle routine approved deduplication details without releasing live Phase9 changes before Phase8 reviewed publication.

COMMENT(director): Phase9 remains preparation-only until Phase8 final review/publication. Accepted earlier bounded test locator plan .checks/phase-9-test-plan.md requires rebase onto completed8 source. Docs-first drafts at .stage-phase-9-director-docs/manifest.json contain exactly two SPEC and one code-quality native-path replacements; live docs/source/test/build application has not run. Current Phase8 work takes precedence. Historical preparation messages are preserved in .checks/phase-9-completed-coordination.md.

- [x] Flatten `runtime/runner/` into `runtime/` with the § Phase 9 merges <!-- completed: 2026-11-01 -->
- [x] Rename `ServiceControlRequest::Legacy` → `Lifecycle` and `decode_legacy_control_body` → `decode_lifecycle_control_body`; move `ObservationHealth` and `NativeLaunchHost` <!-- completed: 2026-11-01 -->
- [x] Merge `coding_agent/` into `mod.rs`, `catalog.rs`, `detector.rs`; move `bundled.json` and `native/` up one level and update the `include_*!` paths and rule path <!-- completed: 2026-11-01 -->
- [x] Merge `broker/runtime/` into `mod.rs`, `events.rs`, `inputs.rs`, `evidence.rs` <!-- completed: 2026-11-01 -->
- [x] Apply R1–R7 and R11 <!-- completed: 2026-11-01 -->
- [x] Confirm 46 tracked `.rs` files under `cafleet/src/`; run the three validation tasks and the scratch setup check; push reviewed Phase 9 changes to PR #541 <!-- completed: 2026-11-01 -->


COMMENT(director): Tester52 may now perform bounded read-only Phase9 test-plan rebase at .checks/phase-9-test-plan.md against actual current68-file cafleet/src after Phase8 live import fix593. Preserve frozen578 artifacts and all current live source; account for final8changes explicitly. Refresh exact runtime21-target/case/helper/cfg/child mapping, known nested collisions and R1–R7/R11 coverage/gaps, with source-confirmed ownership and proposed interfaces. Use current source rather than old prepare.py immutable-source defaults. This writes only the plan under BASE; no Phase9 source/test/docs application, build/test/listing/probe/setup is released. Real Phase8 findings preempt planning, and completed8finalsource still requires rebase before later test writing. Existing Phase9 docs-only drafts remain unchanged.


COMMENT(director): Programmer43 may prepare only .checks/phase-9-source-plan.md read-only against committed b1c46901/current68source tree. Propose exact21runtime owners and R1–R7/R11 helper interfaces/caller order/error preservation, cfg-hook ownership and source/test assembly seams; reconcile Tester607's two R3 direct placement readers against historical design count3 using actual queries. Coordinate with .checks/phase-9-test-plan.md and existing docs-only drafts. No live Phase9 docs/source/test move/build/setup is released until Phase8 final acceptance/review/publication. RealPhase8 findings preempt preparation; preserve all frozen artifacts and operator resources.


COMMENT(director): Planning clarification from current source: R1 adopts permissions::same_input_identity's complete eight immutable fields, including created_at and input_id at the cache fence, as required by one shared immutable identity invariant. Keep mutable state and caller-specific run checks separate, preserve exact causes and add a behavioral-red cache-creation-stamp regression through its current entrypoint before implementation. R3 extracts only the two actual duplicate direct six-column reads; permission_placement already uses get_member and retains that stronger caller policy. Proposed placement_of(&Connection,member_id)->Result<Option<Placement>> is accepted for this extraction; final helper owner/interfaces remain coordinated with source plan610. No live9 writing is released yet.


### Step 10: Phase 10 — Local clutter cleanup (user, after the execute fleet is deleted)

- [x] Confirm `pgrep -f 'cafleet __member-run'` and `pgrep -f 'cafleet message listen'` print nothing <!-- completed: 2026-11-01 -->
- [x] Stop every leftover `cafleet-*` tmux server with `tmux -L <name> kill-server` and confirm none remain <!-- completed: 2026-11-01 -->
- [x] Delete every path in the § Phase 10 delete table <!-- completed: 2026-11-01 -->
- [x] Confirm `git status --short` is unchanged by the cleanup <!-- completed: 2026-11-01 -->

### Step 11: Finalize (user, after the execute fleet is deleted)

COMMENT(director): The user confirmed the remaining user-owned acceptance and authorized local completion. All Success Criteria and manual acceptance checks are recorded as user-confirmed. Final design-document publication remains user-owned under the repository Git policy; the agent makes no design-document commit or pull request.

- [x] Re-check every Success Criteria item <!-- completed: 2026-11-01 -->
- [x] Record the user-confirmed acceptance and mark this design doc Complete locally <!-- completed: 2026-11-01 -->

### Step 12: Follow-up — Retain mise monorepo configuration

COMMENT(director): The user-confirmed mise monorepo layout is restored and published on PR541 at ea741901d9caecbe2cf42d39f770743cde9a25e2. All14 owned configuration/docs files,50 unchanged Rust/native inputs, task discovery from root/packages, installation delegation, binary-test argument forwarding, full1492passed/0failed/8ignored, lint/fmt/docs/release builds, preset installed/embedded bytes and protected operator resources are verified. Fresh Reviewer65 approved without findings in841; GitHub test/lint/build pass and deploy is skipped. Owned fleet7 and both ordinary panes are deleted; the listener closed with fleet_deleted/native exit0 and fleet list confirms absence. .checks/mise-monorepo/publication.json pins the exact remote head/body, checks and cleanup. Prior user acceptance is preserved; all92tasks and Success Criteria are complete. Final design-document publication remains user-owned.

- [x] Restore monorepo root and package-local task ownership, retaining current tool pins and binary test behavior <!-- completed: 2026-11-01T23:36 -->
- [x] Update CI, permission maps and every live task reference consistently with the monorepo registry <!-- completed: 2026-11-01T23:36 -->
- [x] Verify task discovery/delegation/argument forwarding, current Rust tests/lint, docs/release builds and changed preset installation/embedding <!-- completed: 2026-11-01T23:42 -->
- [x] Complete fresh independent review, publish the follow-up on PR541 and remove its owned execution fleet <!-- completed: 2026-11-01T23:52 -->

---


## Changelog

| Date | Changes |
|---|---|
| 2026-11-01 | Completed the user-approved mise monorepo follow-up on PR541 at ea741901: all native checks, fresh review and CI pass; owned execution fleet removed; design Complete92/92. |
| 2026-11-01 | User explicitly retained the pre-consolidation mise monorepo configuration; reopened the scoped configuration follow-up while preserving current versions, binary tests and prior acceptance. |
| 2026-11-01 | User confirmed the remaining user-owned acceptance and authorized local completion; all checks are complete, with final design-document publication retained as user-owned. |
| 2026-11-01 | User scoped digest unification to ordinary preview/prompt/exec and preserved the dedicated permission-response request/decision/audit metadata hash. |
| 2026-11-01 | Initial draft |
| 2026-11-01 | User approved Phase 2 scoped CLI setup, all-backend installation tests and explicit-URL/read-only-SQLite database-path evidence. |
| 2026-11-01 | User approved the Phase 3 scoped scratch check plus OpenCode preset allow-key and embedded/installed-byte verification. |
| 2026-11-01 | User authorized all remaining implementation phases on PR #541 and extended the scoped scratch validation policy through Phase 9 while retaining phase-specific checks and independent review. |
| 2026-11-01 | Corrected the Phase 5 test inventory after source inspection: preserve the root Director runtime-null presentation assertion and rename its local binding. |
