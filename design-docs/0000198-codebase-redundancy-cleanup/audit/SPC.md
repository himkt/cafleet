# Audit: SPC — contract documentation (SPEC.md, docs/docs/**, runtime mirror)

## Summary

The dominant waste is that `SPEC.md` carries a near-verbatim second copy of the `docs/docs/spec/*` pages: `diff --unchanged-line-format` shows 728 of 833 non-empty `cli-options.md` lines, 599 of 664 `member-runtime.md` lines and 196 of 224 `multiplexer-backends.md` lines appear unchanged in SPEC (SPEC §6.3 ≈ cli-options 5–1131, §6.9 ≈ member-runtime 5–971, §6.5 ≈ multiplexer-backends 5–313, §6.4 head ≈ message-envelope). The guard that kept them in sync (`cafleet/tests/docs_sync.rs`, 2725 lines) was deleted in 6712b036, so drift has started; choosing one owner removes ≈2,300 lines. The second-largest class is stale content: the same commit 6712b036 removed `fleet capture`, the stale-assets guard, the sectioned `doctor` layout / member-permissions section and the old `setup` messages from the code, but both copies (and skills) still document them (≈300 lines per copy). Smaller wins: verbatim preset/rules file copies inside SPEC (≈180 lines), duplicated V5 DDL, a stale Python-era multiplexer method surface, and cross-page restatements in concepts/how-to/quickstart. Total estimated removable: ≈3,300 lines (≈2,300 from the one-owner decision in SPC-01 plus ≈1,000 from the other findings; per-copy SPEC deltas in SPC-03/04/05/06/09/11/12/21 overlap SPC-01 and are counted once). The runtime mirror itself (`skills/cafleet/reference/runtime/**`) is 12 git symlinks (mode 120000) into `docs/docs`, so it carries zero duplicated bytes and needs no change.

## Test placement inventory

None in slice.

## Findings

### SPC-01: SPEC.md duplicates the docs/docs/spec pages near-verbatim
- Category: doc-duplication
- Location: `SPEC.md:117-1243` ↔ `docs/docs/spec/cli-options.md:5-1131`; `SPEC.md:2297-3254` ↔ `docs/docs/spec/member-runtime.md:5-971`; `SPEC.md:1334-1642` ↔ `docs/docs/spec/multiplexer-backends.md:5-313`; `SPEC.md:1248-1310` ↔ `docs/docs/spec/message-envelope.md:3-64`
- Evidence: `diff --unchanged-line-format=%L --old-line-format= --new-line-format= SPEC.md <page> | grep -c .` → cli-options 728/833 non-empty lines, member-runtime 599/664, multiplexer-backends 196/224. The remaining differences are heading levels, `{#anchor}` presence, and `§`-references vs Markdown links (SPEC line ≈ docs line + 112 across §6.3). Drift already exists: `message-envelope.md:64` has the `operation_rows` sentence (matches `cafleet/src/broker/runtime/events.rs:675` `MULTI_ROW_INSTRUCTION`) that SPEC §6.4 lacks; `member-runtime.md:653` lacks the "in array order" rule of `cli-options.md:985`; SPEC:2714 has an `ObservationKind` sentence docs lacks. The sync test `cafleet/tests/docs_sync.rs` (2725 lines) was removed in 6712b036 and nothing replaced it. SPEC already delegates to the docs pages 52 times (`grep -c "](docs/docs/" SPEC.md` = 52), including links from inside sections SPEC itself duplicates (e.g. SPEC:3105 → `cli-options.md#member-press` while SPEC:1005 has the same section; SPEC:3140/3142/3197 → `cli-options.md#permission-commands` vs SPEC:1099). The docs pages are also agent runtime reading: `skills/cafleet/SKILL.md:72-170` and `skills/cafleet/roles/director.md:107-407` link into `reference/runtime/spec/*` (symlinks to these pages); SPEC.md is not bundled.
- Proposal: Make the `docs/docs/spec/*` pages the single owner of the CLI, member-runtime, multiplexer and envelope contracts (they ship with the skill and SPEC already defers to them). Replace SPEC §6.3, §6.4 (lines 1248-1310), §6.5 (1334-1642) and §6.9 with one short paragraph each that names the owning page as a normative part of the specification, and keep in SPEC only the reimplementation-only material no docs page carries: §1–5, §6.1–6.2 summaries, §6.4 capture/`strip_ansi`/formatter rules (1312-1330), the corrected adapter surface (SPC-10), §6.7 interface/ordering invariants (1842-1899, 1954-1956, 2057-2066), §7 and §8 DDL. Revise `SPEC.md:3` ("self-contained"), `.claude/rules/documentation-maintenance.md` (SPEC row: "single authoritative reimplementation specification") and the `update-readme` skill in the same change so they describe SPEC as the reimplementation index plus DDL owner with the docs spec pages as normative owners. The alternative (SPEC owns, docs collapse to summaries) removes the same lines but strips detail from the installed skill bundle, so it is the weaker option.
- Spec impact: none — every contract statement survives verbatim in exactly one shipped document; only the duplicate copy goes.
- Risk: medium — a documentation-policy decision (rule text, SPEC preamble, update-readme skill) must change together; all inbound anchors into the docs pages stay valid because the docs copy is the one kept.
- Estimated delta: -2300 lines

### SPC-02: `fleet capture` is documented but no longer exists in the CLI
- Category: stale-doc
- Location: `SPEC.md:126,195,218,410-416,1069,1341,1517`; `docs/docs/spec/cli-options.md:14,83,106,298-304,957`; `docs/docs/spec/multiplexer-backends.md:12,188`; `docs/docs/concepts/member-lifecycle.md:23`; `docs/docs/how-to/design-doc-development.md:35`; also `skills/cafleet/SKILL.md:68,164`, `skills/cafleet/reference/supervision.md:32,102`, `skills/cafleet/roles/director.md:372-374`
- Evidence: `FleetCommand` has only `Create/List/Show/Delete` (`cafleet/src/cli/fleet.rs:14-46`); `grep FleetCapture|fleet_capture|write_scan|ScanEntry cafleet/src` → 0. `git show 6712b036 -- cafleet/src/cli/fleet.rs` shows `-    Capture {` and `-use crate::capture::{CaptureSnapshot, ScanEntry, write_scan};`. `skills/cafleet/SKILL.md:164` links to `cli-options.md#fleet-capture` and `director.md:374` instructs Directors to run it.
- Proposal: Decide whether 6712b036 removed `fleet capture` deliberately. If deliberate, delete every mention (docs, SPEC, `permissions.allow` row `Bash(cafleet fleet capture *)`, skills) per the Removal rule. If accidental, restoring the command is a code task outside this slice. needs-verification: the design doc behind 6712b036 (or the user) confirms intent.
- Spec impact: none — docs are reconciled to the shipped binary, which already lacks the subcommand.
- Risk: low — doc-only once intent is confirmed; skills currently tell agents to run a command that fails.
- Estimated delta: -25 lines per copy (-50 total, plus ~6 skill lines)

### SPC-03: `cafleet doctor` text/JSON/footer contract describes a removed implementation
- Category: stale-doc
- Location: `docs/docs/spec/cli-options.md:319-417,454-501,514-596` ↔ `SPEC.md:431-529,566-613,626-708`
- Evidence: `cafleet/src/cli/doctor.rs:117-134` prints one `"{status} {name}: {detail}"` line per check, JSON `{"checks":[{name,status,detail}]}`, and fails with `doctor found errors`. Documented strings `issues found` (cli-options:516), the ✓/✗ sectioned layout, nested JSON, and the Member permissions section (`member_permissions`, `PROBE_COMMANDS`, `allowManagedPermissionRulesOnly`) have 0 hits in `cafleet/src` (7 hits in SPEC, 6 in cli-options); `member_permissions.rs` was deleted in 6712b036 and `git log -S "issues found" -- cafleet/src` ends at 6712b036. Version-mismatched installs are `info` in code (`doctor.rs:283-290`) but an issue in docs (cli-options:404). The OpenCode preset checks (418-452) and member runtime checks (503-512) still match code.
- Proposal: Replace the text layout, Database/Multiplexer/Coding-agents/Member-permissions section prose, footer and JSON subsections with one ≈25-line specification of the check-row model: check names (`<agent>.configuration` / `.executable` / `.installation`, `opencode.preset[.<mode>]`, `member_runtime.<id>`, database/multiplexer rows), status values, newline escaping, JSON shape, exit rule. Keep the OpenCode preset and member runtime subsections.
- Spec impact: none — aligns the doc with the shipped output.
- Risk: low — doc-only; verify the full check-name list against `doctor.rs` while rewriting.
- Estimated delta: -200 lines per copy

### SPC-04: The stale-assets guard is documented but not implemented
- Category: stale-doc
- Location: `docs/docs/spec/cli-options.md:160-173,238-265,312-314,662,1090-1091` ↔ `SPEC.md:272-285,350-377,424-426,774,1202-1203`, `SPEC.md:3259` ("guards that deliberately inspect every supported asset identity"); `docs/docs/concepts/storage.md:35`
- Evidence: `stale assets detected` and `no assets install is recorded` → 0 hits in `cafleet/src`; `git log -S "stale assets detected" -- cafleet/src` ends at 6712b036. `asset_installs` is read only by `doctor.rs:272` (`install_check`, reported as `info`); `invocation_connection` (`cafleet/src/cli/mod.rs:1372-1400`) validates schema only.
- Proposal: Delete the Stale-assets guard section, its error-table rows, the "schema/assets guards" wording and the lazy-validation paragraph premised on it; reword `storage.md:35` to say doctor reports the recorded version. needs-verification only for intent, as in SPC-02.
- Spec impact: none — the binary already performs no such guard.
- Risk: low.
- Estimated delta: -40 lines per copy (-80 total)

### SPC-05: `cafleet setup` messages and half-failure model are stale
- Category: stale-doc
- Location: `docs/docs/spec/cli-options.md:122,124,200,1093` ↔ `SPEC.md:234,236,312,1205`
- Evidence: Documented `Created PATH and applied migrations to head (N).`, `Upgraded from M to N.`, `Already at head (N); nothing to do.`, `db half failed:` / `assets half failed:` and `the database schema is missing or outdated; run 'cafleet setup' first` → 0 hits in `cafleet/src`. Code prints `Applied migrations to head ({head}) at {}.` and `Already at head ({head}).` (`cafleet/src/cli/setup.rs:66,70`), and stops asset installation at the first failing agent (test `setup_records_successes_before_the_first_asset_failure`), contradicting "Independent half-error reporting" (cli-options:122). The `cafleet-research` leftover cleanup (cli-options:200) has 0 hits in `cafleet/src/assets.rs`.
- Proposal: Rewrite the setup output paragraph to the two shipped messages and the shipped failure order; delete the `cafleet-research` sentence and the dead error row.
- Spec impact: none — doc matches the binary.
- Risk: low.
- Estimated delta: -8 lines per copy

### SPC-06: `member create` error strings are stale
- Category: stale-doc
- Location: `docs/docs/spec/cli-options.md:805-813,1128-1129` ↔ `SPEC.md:917-925,1240-1241`
- Evidence: `cannot resolve the member's coding agent` → 0 hits; code emits `Director (member {director_id}) not found.` / `Director (member {director_id}) has no placement; supply --coding-agent.` (`cafleet/src/cli/member.rs:230,233`). Compensation strings `tmux split-window failed`, `placement update failed`, `placement row vanished before pane-id`, `cleanup failed for member`, `cleanup failed for fleet … transaction` → 0 hits; only `cleanup failed for pane` exists (`cafleet/src/multiplexer/mod.rs:426`). Creation now runs through `cafleet/src/runtime/runner/creation.rs`. Undocumented shipped error: `fleet {id}'s root Director (member {d}) has no concrete placement.` (`cafleet/src/broker/fleets.rs:135`).
- Proposal: Replace the two error rows with the shipped strings, rewrite the compensation-string list from `runtime/runner/creation.rs`, add the `fleets.rs:135` error. needs-verification: the compensation order table (cli-options:792-797) against `runtime/runner/creation.rs`.
- Spec impact: none — doc matches the binary.
- Risk: low.
- Estimated delta: -10 lines per copy

### SPC-07: SPEC copies bundled preset and rules files verbatim
- Category: doc-duplication
- Location: `SPEC.md:2119-2185` (`cafleet.md`), `SPEC.md:2191-2257` (`cafleet-ask.md`), `SPEC.md:1999-2037` (codex rules); `docs/docs/spec/coding-agent-backends.md:256-294` (codex rules)
- Evidence: Each SPEC block is line-identical to `presets/opencode/cafleet.md`, `presets/opencode/cafleet-ask.md` (65 lines each) and `presets/codex/cafleet.rules` (37 lines). `SPEC.md:2111` already states "The checked-in `presets/opencode/cafleet.md` is the content contract", and `SPEC.md:1990` names the rules source path. `cafleet/src/assets.rs:460-494` tests the preset content invariants. The rules file therefore exists three times in the repository.
- Proposal: Keep SPEC's format description (2111-2117) and newline convention (2259-2261); replace the three verbatim blocks with path-as-contract references to the bundled files (allowed by documentation-maintenance § Exemptions). In `coding-agent-backends.md`, replace the 39-line rules dump with a rule → decision summary table that names the installed `~/.codex/rules/cafleet.rules`.
- Spec impact: none — the bundled files remain the contract and ship unchanged.
- Risk: low.
- Estimated delta: -210 lines

### SPC-08: SPEC §6.7 body restates coding-agent-backends.md, with drift
- Category: doc-duplication
- Location: `SPEC.md:1918-1931,1946-1949,1978-1984,2052-2055,2068-2107,2279-2289` ↔ `docs/docs/spec/coding-agent-backends.md:139-161,36,224-226,359-361,367-415,428-442,100-102,218-220,357`
- Evidence: Same `--allowedTools`, codex explicit-selection, `opencode mini`, preset table, prerequisites, merge, shell ruleset, safety caveat table and host-evidence prose. Drift: opencode paragraph SPEC:2289 ("uses explicit manual broker reads…") vs docs:357 ("coordinates next steps and routed native shell commands"); `cafleet-ask` cell SPEC:2090 vs docs:401-402. SPEC already delegates argv (845-848) and model/effort errors (1234-1237) to the same page.
- Proposal: Folded into SPC-01: keep only the interface/ordering invariants in SPEC and link the rest to `coding-agent-backends.md` anchors.
- Spec impact: none — single owner retains every statement.
- Risk: low.
- Estimated delta: -110 lines (counted separately from SPC-01's §6.3/§6.5/§6.9 total)

### SPC-09: V5 permission-response DDL is written three times
- Category: doc-duplication
- Location: `SPEC.md:3157-3185` (§6.9), `SPEC.md:3453-3481` (§8), `docs/docs/spec/member-runtime.md:877-905`
- Evidence: All three match `cafleet/migrations/V5__permission_responses.sql:29-53`. SPEC already delegates V6 DDL from §6.9 to §8 (`SPEC.md:2914`).
- Proposal: Keep the DDL in SPEC §8 only; replace the §6.9 block and the docs block with "Exact DDL: SPEC §8 Permission response extension migration" (the docs page already links SPEC as DDL owner in `data-model.md`), keeping the semantics paragraphs that follow.
- Spec impact: none.
- Risk: low.
- Estimated delta: -56 lines

### SPC-10: SPEC multiplexer adapter surface names Python-era / nonexistent items
- Category: stale-doc
- Location: `SPEC.md:1377-1383,1644-1655,1681-1817`; `docs/docs/spec/multiplexer-backends.md:50-53,112`
- Evidence: `list_pane_ids`, `HerdrError`, `select_layout` → 0 hits in `cafleet/src`. Renamed: `_equalize_tab_column` → `resize_tab_column` (`multiplexer/herdr.rs:391`), `_rebalance_after_close` → `resize_after_close` (547), `_restore_director_full_width` → `restore_director_full_width` (580), plus `_read_tab_layout`, `_equalize_column`, `_pane_tab_id`, `_surviving_pane_in_tab`, `_run()`. Python idioms remain (`os.getcwd()`, `OSError`, `shlex.join`, `lines=20`, `ignore_missing=False`, `timeout=5`). Unlisted live methods: `send_prompt`, `validate_key_target`, `send_keys`, `close_pane`, `restore_after_close`/`PostCloseLayout` (`multiplexer/mod.rs:675-710`). tmux capture argv is documented `-e -p` but code is `-p -e` (`tmux.rs:331-333`) under an "exact argv" claim (SPEC:1662). Error taxonomy claims per-backend subclasses; code has one `MultiplexerError` struct (`multiplexer/mod.rs:385`). `MultiplexerContext` is called "frozen, constructed only by context_discovery" but is a pub-field `Clone` struct built directly in tests (`mod.rs:366-371`, `tmux.rs:370`). SPEC:1653 cites a nonexistent heading "Creation ownership and compensation". §1813-1817 and §6.6 (1821-1825) restate 1562-1568 / 1466-1494.
- Proposal: Replace 1681-1817 with a ≈20-line language-neutral table (operation / tmux argv / herdr argv / failure handling) built from `tmux.rs`/`herdr.rs`, fix the error-taxonomy and context sentences in both copies, delete the restating Literal-input and §6.6 paragraphs, and fix the dead cross-reference.
- Spec impact: none — the argv/behavior contract is preserved and corrected to the shipped binary.
- Risk: low.
- Estimated delta: -100 lines

### SPC-11: Runtime CLI presentation contract is restated in the runtime page
- Category: doc-duplication
- Location: `docs/docs/spec/member-runtime.md:549,633-655,663,689` ↔ `docs/docs/spec/cli-options.md:965-985` (and SPEC 2916-2938/2946/2972 ↔ 1075-1097)
- Evidence: Same 18-key runtime object order, `Runtime: … profile=MATCH review=STATE` line (code `cafleet/src/output/runtime.rs:27`), event/input show keys, recover JSON/text and `--owner-terminated` format. `member-runtime.md:549` and `:644` themselves say the CLI reference fixes the layouts.
- Proposal: Keep the layouts in the CLI page; reduce member-runtime's CLI presentation contract to 2–3 sentences plus a link; drop the 549 summary.
- Spec impact: none.
- Risk: low.
- Estimated delta: -20 lines per copy

### SPC-12: Repeated rules inside member-runtime.md
- Category: doc-duplication
- Location: `docs/docs/spec/member-runtime.md` 541↔967, 797↔970, 920↔960, 869 (pre-summary of 872-906), 912↔910/774/859, 792↔776-790, 644↔655, 31↔89/52/58-65, 17↔152, 19↔311, 291↔463, 535↔830; blank-line pairs at 645-646, 793-794, 870-871, 958-959 (SPEC mirrors at +2294/+2283)
- Evidence: Verbatim or same-rule sentences stated twice within the page (e.g. the secret-test sentence at 541 and 967, the two-second target at 920 and 960).
- Proposal: State each rule once at its owning subsection; delete the restatements and double blank lines.
- Spec impact: none.
- Risk: low.
- Estimated delta: -15 lines per copy

### SPC-13: Schema-compatibility paragraph and application identity repeated six times
- Category: doc-duplication
- Location: "fresh baseline creates exactly…" at `SPEC.md:81,346,3287`, `docs/docs/spec/data-model.md:75`, `docs/docs/concepts/storage.md:17`, `docs/docs/spec/cli-options.md:234`; `1128351288` at `storage.md:19`, `data-model.md:77`, `cli-options.md:236,362`; identity/ledger/shape rules again at `cli-options.md:122,230,360-370,588-594`; doctor exemption at `cli-options.md:236,255-260,312-314,1085`
- Evidence: Same paragraph text in all six locations; value verified as `APPLICATION_ID` (`cafleet/src/db/mod.rs:70`).
- Proposal: Owner in docs = `data-model.md` (schema) and cli-options § Schema-version guard (errors); SPEC owner = §8. Other mentions become a link plus one clause (echo rule: no exact identity value outside the owner).
- Spec impact: none.
- Risk: low.
- Estimated delta: -20 lines

### SPC-14: storage.md claims per-process shape caching that does not exist
- Category: stale-doc
- Location: `docs/docs/concepts/storage.md:29`
- Evidence: `reference_schema` (`cafleet/src/db/mod.rs:194-204`) re-runs migrations in memory on each call; no `OnceLock`/`LazyLock`/`thread_local` in `db/mod.rs` or `diagnosis.rs`. The "consistent read snapshot" part is correct (`db/mod.rs:215-222`).
- Proposal: Delete the caching sentences; keep the snapshot and per-validation identity/ledger sentence.
- Spec impact: none.
- Risk: low.
- Estimated delta: -2 lines

### SPC-15: SPEC §8 omits V2/V3 and names migrations inconsistently; data-model wording predates V4
- Category: stale-doc
- Location: `SPEC.md:3285-3516`; `docs/docs/spec/data-model.md:47`
- Evidence: `cafleet/migrations/` holds `V1__baseline.sql`, `V2__preview_attempts.sql` (adds `messages.preview_started_at`), `V3__drop_preview_claims.sql` (drops it), `V4__member_runtime.sql`, `V5__permission_responses.sql`, `V6__automatic_review_events.sql`; `preview_started_at` → 0 hits in SPEC/docs although SPEC:3287 says known versions use their recorded shape. Headings mix "Runtime extension migration" (V4), "Permission response extension migration" (V5), "V6 automatic review migration". `data-model.md:47` says "initialized once by the next contiguous current-family migration" though V4 exists.
- Proposal: Add a two-line V2/V3 note to §8, rename §8 headings after the migration slugs (`V4 member runtime migration`, …), and reword data-model:47 to name V4.
- Spec impact: none — documents existing migrations.
- Risk: low.
- Estimated delta: +2 lines (net)

### SPC-16: Automatic-review consumption steps and launch-policy defaults are restated across pages
- Category: doc-duplication
- Location: table `docs/docs/spec/member-runtime.md:937-947`, `docs/docs/concepts/member-permission-events.md:13-20,24-30`, `skills/cafleet/reference/member-runtime.md:5-29`; exact defaults (`auto`, `approve-for-me`, `cafleet`) at `concepts/member-permission-events.md:7`, `concepts/coding-agents.md:26`, `cli-options.md:723`, `coding-agent-backends.md:41` (owner `member-runtime.md:9-13`)
- Evidence: The concepts page ships in the skill bundle (`reference/runtime/concepts/member-permission-events.md` symlink), so agents read the consumption steps twice; the echo rule forbids exact values outside the owner.
- Proposal: Concepts table → link to the spec table plus one clause; skills reference links `runtime/concepts/member-permission-events.md#consume-a-runtime-event` and keeps only its agent-specific deltas (record operation key before dispatch, OpenCode `Step interrupted` continuation, mode provenance); non-owner pages describe defaults qualitatively.
- Spec impact: none.
- Risk: low.
- Estimated delta: -20 lines

### SPC-17: concepts/coding-agents.md repeats spec tables and agent-only rules
- Category: doc-duplication
- Location: `docs/docs/concepts/coding-agents.md:125-137` (Known asymmetries), `:47-64` (One-shot command isolation)
- Evidence: The asymmetry table restates `coding-agent-backends.md:70-74,92-96` and quotes the exact error `opencode does not support reasoning effort.` (echo rule); 135-137 restate `coding-agent-backends.md:73-74,442`. The one-shot section restates `skills/cafleet/SKILL.md:103-111` (agent instruction; contributing.md:51 "agent instructions in skills") and also appears as SPEC §7.7 (`SPEC.md:3281-3283`); the docs anchor `#one-shot-command-isolation` has no inbound links. `SPEC.md:831` links the `#known-asymmetries-intentional-non-goals` anchor.
- Proposal: Replace the asymmetry section with a one-clause summary linking the backend capability table (retarget SPEC:831); reduce the one-shot section to two sentences for operators explaining why members issue one command per call.
- Spec impact: none.
- Risk: low.
- Estimated delta: -25 lines

### SPC-18: Quickstart restates backend rationale and its permission profiles are stale
- Category: stale-doc
- Location: `docs/docs/quickstart.md:41-42,61-66,74-79,97-100,105-108,123-126,131-133,143-151`
- Evidence: Both JSON profiles list only `Bash(cafleet member prompt *)` / `Bash(cafleet member exec *)` under `ask` (98-99, 124-125), while `coding-agent-backends.md:175,183` and `SPEC.md:220` require `Bash(cafleet member press *)` too. The codex summary (149-151) names prompt/exec only, while `presets/codex/cafleet.rules:18` also prompts for press (and respond/reconcile/recover). Rationale paragraphs duplicate `coding-agent-backends.md:167-193,212,235-239`; the "same-name agent definitions can merge" sentence appears five times (SPEC:257, SPEC:2077, cli-options:145, coding-agent-backends:388, quickstart:42).
- Proposal: Add `Bash(cafleet member press *)` to both profiles and the table row; describe the codex rules by property ("Director-owned commands prompt"); trim rationale to steps plus links to the backend page.
- Spec impact: none — the doc catches up with the shipped presets and SPEC.
- Risk: low.
- Estimated delta: -20 lines

### SPC-19: SPEC §6.7 Interface describes absent identifiers and a wrong check order
- Category: stale-doc
- Location: `SPEC.md:1840,1845,1859-1860,1905,1907,1934,1936,1961,1993-1994,1922-1924`
- Evidence: "three methods" then four listed (1853/1863/1869/1875); `CODING_AGENTS` and module-level `EFFORT_LEVELS` → 0 hits (code: `CodingAgent::all()` `coding_agent/mod.rs:560,700-705`, per-arm slices 625-626); `SpawnProbe` omits `home_dir` (`mod.rs:546`); `ensure_available` resolves the config dir before the PATH check (`mod.rs:649-650`), unlike "PATH check only". 1922-1924 is a one-data-row table.
- Proposal: Correct the count, use behavior names instead of absent identifiers, fix the order, and turn the one-row table into a sentence.
- Spec impact: none.
- Risk: low.
- Estimated delta: -3 lines

### SPC-20: Preview-line and delivery prose duplicated between envelope, multiplexer and data-model pages
- Category: doc-duplication
- Location: `SPEC.md:1292-1306` ↔ `1586-1598`; `docs/docs/spec/message-envelope.md:46-60` ↔ `multiplexer-backends.md:257-269`; `docs/docs/spec/data-model.md:39-43` ↔ `multiplexer-backends.md:271-289`; `SPEC.md:75` ↔ `1254-1266`
- Evidence: Same preview forms and pending/at-least-once delivery explanation on two pages each.
- Proposal: Owner = multiplexer-backends § Push notifications; message-envelope and data-model keep column semantics and link.
- Spec impact: none.
- Risk: low.
- Estimated delta: -30 lines

### SPC-21: cli-options internal restatements, agent procedure and code locations
- Category: doc-duplication
- Location: `docs/docs/spec/cli-options.md:1081-1112` (Error Messages), `:628`, `:947-958`, `:139-145,171-182`, `:54,98`; code identifiers at `:467,788,800,803,818,955,1102`; historical phrasing at `:816,819-820,838,959,967,1077`; fixed counts at `:159,161,164,199`; 3-sentence cell at `:721` (SPEC mirrors at +112)
- Evidence: 1086-1089 are four rows whose message and note cells all link the schema guard; 1094-1112 repeat strings fixed in each command section (96, 274, 284, 294, 632, 861, 883, 884/932) while 1081 says those errors are specified at their owners. 628 carries the agent follow-up rule (`message N was not delivered to your pane; poll your inbox`) owned by `skills/cafleet/roles/member.md:37` (0 hits in src). Capture hash rules appear at 947-953 and 955-958. Config-dir prose 139-145/171-182 restates the table at 132-137. Code identifiers (`split_window`, `PaneCleanup::Attempted`, `kill_pane(id, true)`, `CaptureSnapshot::from_raw`, `register_member`, broken source link at 467) violate documentation-maintenance "Describe behavior, not code locations"; "stay unchanged"/"No new … contract is introduced" is change narration.
- Proposal: Collapse 1086-1089 to one row; drop table rows whose strings are owned by command sections; delete 628's agent rule, the second hash paragraph and the restating config-dir prose; rewrite code-location and historical phrasing as behavior; replace counts with properties; split the long cell.
- Spec impact: none — every error string remains at its owning section.
- Risk: low.
- Estimated delta: -35 lines per copy

### SPC-22: SPEC §7.7, §9–§11 and §6.2 errors restate other owners or carry process advice
- Category: doc-duplication
- Location: `SPEC.md:3281-3283` (§7.7), `3517-3521` (§9), `3523-3525` (§10), `3527-3537` (§11), `113` (§6.2 Broker errors), `3259/3267/3271` (§7.1/7.3/7.4)
- Evidence: §7.7 restates `skills/cafleet/SKILL.md:103-109`; §9's second paragraph ("Report meaningful current test results and production/documentation size reductions…") is process advice, which documentation-maintenance forbids in SPEC ("specification only, no recommendations"); §9 still says "Escape-first argv order", while runner-wrapped previews omit Escape (`skills/cafleet/SKILL.md:121`); §10 is a single pointer to §6.3; §11 is a section index duplicating the TOC; §6.2 Broker errors restates five §6.3 error rows; §7.1/7.3/7.4 restate cli-options 38, 90-92, 110-112, 604.
- Proposal: Keep the section headings (rule: preserve SPEC section structure) but reduce §7.7 to one sentence linking the skill rule, delete §9's process paragraph and scope "Escape-first" to unwrapped members, reduce §11 to nothing beyond its heading-level statement or drop the table, and replace §6.2's error list with a link to the error table.
- Spec impact: none.
- Risk: low — structure preserved; only restating prose goes.
- Estimated delta: -20 lines

### SPC-23: Director reception lifecycle restated on five operator pages
- Category: doc-duplication
- Location: `SPEC.md:15`; `docs/docs/concepts/overview.md:42`; `docs/docs/concepts/member-lifecycle.md:9,13`; `docs/docs/concepts/coding-agents.md:143-145`; `docs/docs/how-to/design-doc-development.md:35`; owner `docs/docs/how-to/mixed-backend-team.md:31,62-84`
- Evidence: Each page restates "one owned native receiver, explicit ACK, deferred actions in host-owned state, preserve goal controls/user gates" and links the manual lifecycle anyway. `member-lifecycle.md:9` restates the member ready/poll protocol owned by `skills/cafleet/roles/member.md:19-33` (contributing.md:51: agent instructions belong in skills).
- Proposal: Keep the procedure in mixed-backend-team § Manual lifecycle; reduce the other mentions to one clause plus link; drop member-lifecycle.md:9's protocol detail to "members confirm readiness, then poll".
- Spec impact: none.
- Risk: low.
- Estimated delta: -10 lines

### SPC-24: Runtime mirror mechanism — keep as is
- Category: other
- Location: `skills/cafleet/reference/runtime/**`, `cafleet/build.rs:10`
- Evidence: `git ls-files -s skills/cafleet/reference/runtime` → 12 entries with mode 120000 (symlinks into `docs/docs`); `diff -r -q docs/docs skills/cafleet/reference/runtime` differs only by the unbundled pages (`overview.md`, `contributing.md`, `index.md`, `how-to/design-doc-development.md`, `_meta.json`/`_nav.json`). `build.rs` walks `../skills` and `include_bytes!` follows the symlinks; `cargo:rerun-if-changed=../docs/docs` is load-bearing because edits land in the symlink targets. Every relative link in the 12 bundled pages resolves to another bundled page (checked all `](…)` targets), so the closure is consistent. The generator/checker `cafleet/src/cli/runtime_docs.rs` was removed in 7098c55a; nothing now fails if a bundled page gains a link to an unbundled page.
- Proposal: No change for redundancy. The SPC-01 decision keeps the docs pages as owners, which this mechanism already ships.
- Spec impact: none.
- Risk: low.
- Estimated delta: 0 lines
