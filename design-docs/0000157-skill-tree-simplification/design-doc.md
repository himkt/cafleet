# Skill Tree Simplification

**Status**: Approved
**Progress**: 30/30 tasks complete
**Last Updated**: 2026-08-03

## Overview

Remove five factual defects and four classes of duplication from `skills/` (45 markdown files, 5,897 lines) without weakening any protocol a spawning agent depends on. Duplicated content moves into the page that already owns its subject and is already gated in the reader's Required-reading block, so no reader gains a new load-bearing read. A new structural guard test makes this class of drift fail loudly in CI instead of silently at spawn time.

## Success Criteria

- [x] All five correctness defects (D1–D5) are fixed, and no skill file contradicts `reference/supervision.md`, `docs/docs/spec/cli-options.md`, or `SPEC.md` on the surfaces they own.
- [x] No skill file references a repository path, error string, or protocol that does not exist.
- [x] The path-canonicalization procedure appears in exactly one owning page; the five call sites carry a link plus a one-clause summary.
- [x] Every invariant Director section and every shared bootstrap block exists once, in a page already gated in the reading Director's Required-reading block. No Required-reading row is added, removed, or reworded by this change.
- [x] No `IMPORTANT:` line, hard role-constraint, or start cue was collapsed in any per-role delta (the lossless rule in `cafleet/reference/director.md`).
- [x] `mise //cafleet:test` passes, including a new structural guard test that fails when a skill file references a nonexistent path, drops its overlay Required-reading row, or uses a `{token}` outside the known vocabulary. The nonexistent-path invariant is demonstrated red against the pre-fix tree before the fixes land.
- [x] Every file this change touches is tracked in this repository; nothing under `~/.claude/` is written.

---

## Background

Six prior design docs (0000088, 0000093, 0000094, 0000102, 0000113, 0000129) already ran simplification passes over this tree. The cheap wins are taken; what remains is either load-bearing or a genuine defect. This pass therefore leads with correctness and treats line reduction as a by-product.

### Scope

`skills/` only — the three skills `cafleet`, `cafleet-design-doc`, `cafleet-research`.

**Neither directory named `.claude/skills/` is in scope.** The repository's own `.claude/skills/` (clean-docs, skill-author, update-readme, cafleet-model-list-refresh) is project-local and is neither a simplification target nor a destination. The user-level `~/.claude/skills/` holds the promoted copy of these three skills, but this change does not write to it — promotion is a separate, user-initiated action (see § *Promotion is out of scope*).

### Expected magnitude — correcting the pre-team estimate

The pre-team scan estimated 900–1,300 lines removed (15–22%). **That figure is not reachable even at maximum scope and should not be carried forward.** Scope was subsequently widened to include the full group-A bootstrap preamble; the recomputed, measured expectation follows.

**Bytes are the primary unit.** These files are consumed as spawn-time context, so the cost is bytes/tokens, not screen lines. The two metrics diverge sharply here because the group-A blocks are dense single-line paragraphs — one ~1,100-byte blockquote counts as one line. The tree is **532,308 bytes** across the 45 files.

Two kinds of figure appear below, and the `~` marks distinguish them. **Measured** (exact, no tilde): the matched-line byte totals from `rg --stats` for each named block — 3,923 (spawn blockquotes), 2,344 (`doctor`), 8,542 (B sections), 4,347 (A1), 2,617 (C). **Derived** (tilde): every net figure, obtained by subtracting the one retained canonical copy and the replacement pointers from a measured gross; plus the A2 `fleet create` component and the D1 section span, which are section-span estimates rather than single-pattern matches.

| Change | Sites | Gross B | Net B | Net lines |
|---|---|---|---|---|
| A2 — bootstrap blocks (spawn blockquotes 3,923; `doctor` 2,344; `fleet create` ~2,600) | 15 | ~8,867 | **~6,900** | ~15 |
| B — fold invariant Director sections into `supervision.md` | 22 | 8,542 | ~4,900 | ~40 |
| A1 — trailing "Before acting, resolve every `{token}`…" sentence | 27 | 4,347 | 4,347 | ~54 |
| C — path canonicalization → link + one-clause summary | 5 | 2,617 | ~1,600 | ~8 |
| D1 — delete § *Monitoring aggregate previews* | 1 | ~1,100 | ~1,100 | ~15 |
| D2–D5 — string and glob corrections | 8 | ~0 | ~0 | ~0 |
| **Total** | | **~25,473** | **~18,850 (≈3.5%)** | **~130 (≈2.2%)** |

**A2 is the largest item by bytes (~6,900) while being the smallest by lines (~15)** — the measurement confirms the expectation. The scope escalation is therefore the highest-value single part of the pass on the metric that matters: it removes roughly a third of the total byte saving, and it removes it from exactly the five files every workflow Director loads at spawn time. On the line metric it looked marginal; on the byte metric it is not.

Neither metric reaches the 15–22% of the original estimate. Measured, the pass removes ~3.5% of the tree's bytes. Per the agreed success criteria it is judged behaviorally, not by either number; the table is an expectation, not a target.

**One measured finding beyond the authorized scope.** The "Angle-bracket tokens … are placeholders, **not** shell variables" sentence occurs in **15 files / 20 lines / 7,658 bytes** — not the 8 files the pre-team scan recorded. Roughly 5,000 bytes of that sits in member and role files outside the five workflow bodies A2 covers, in a near-byte-identical form that already points at `cafleet/SKILL.md` § *Placeholder convention* as canonical. Folding it would roughly double A2's yield. It is **not** included in the implementation below, because the authorized A2 scope is the five workflow bodies; it is recorded here as a decision the user may take separately.

### Two scan claims corrected by verification

- The tree holds **45** markdown files, not 46 (plus 11 non-markdown Slidev theme assets under `cafleet-research/reference/slidev/`).
- The promoted copy is **not** byte-identical. `~/.claude/skills/cafleet/roles/monitor.md` and `~/.claude/skills/cafleet/reference/supervision.md` are stale: they still describe the root Director enrolled in the watched set at 180 s, which design 0000156 removed. The repo tree is authoritative; promotion is one-directional (repo → `~/.claude/skills/`). That drift is left standing — see § *Promotion is out of scope*.

### Promotion is out of scope

This change writes only to files tracked in this repository. Copying `skills/` over `~/.claude/skills/` is a separate action the user runs when they choose to, so two consequences follow and are accepted:

- The user-level tree does not carry this design's edits. Any agent loading these skills by name keeps the pre-change instructions until a promotion happens.
- The 0000156 staleness noted above stays in `~/.claude/skills/cafleet/roles/monitor.md` and `reference/supervision.md`. It predates this design and is not this design's to clear.

---

## Specification

### D. Correctness fixes

Each is a factual defect verified against the implementation, not a style preference.

| # | Site | Defect | Fix |
|---|---|---|---|
| D1 | `cafleet/roles/director.md:65-79` | § *Monitoring aggregate previews* specifies a `monitor report batch:` aggregate with dedup-by-message-ID and a `monitor finished:` entry class. Those strings appear nowhere in `cafleet/src/`, `docs/`, or `SPEC.md` — nothing implements the protocol. `reference/supervision.md:52` states the opposite: "no gate token, no aggregate, no batching, and no deferral". | Delete the section outright. `supervision.md` already owns the correct per-event spec; per `.claude/rules/removal.md` leave no deprecation note or residue. |
| D2 | `cafleet/roles/member.md:65-66` | Cites "the durable stall state machine returns `action = ping`". `roles/monitor.md` step 3 and `supervision.md:42` place the quiet baselines in the monitoring member's own conversation notes — "not the broker". | Restate as the current mechanism: two byte-identical stall-check captures in the monitoring member's own notes. Preserve the genuinely durable element — `last_stall_check_at` still persists dispatch cadence across loop restart (`supervision.md:40`) — so do not over-correct to "nothing is durable". |
| D3 | `cafleet/SKILL.md:111`, `SKILL.md:89`, `cafleet/reference/director.md:58` | Click/Python-era error vocabulary after the Rust rewrite (0000152). `SKILL.md:111` names "Click's standard not-a-valid-integer error"; `SKILL.md:89` and `reference/director.md:58` name `UsageError`, a Click class. | Adopt the backend-neutral wording already used by the owning spec: `cli-options.md:172` says "the parser's invalid-integer error". For the placeholder failure the contract string is `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` (`spawn_prompt.rs:43`, `cli-options.md:811`), exit 2. Do **not** touch `str.format` — `SPEC.md:1133,1405` uses that term for the substitution semantics and it is correct. |
| D4 | `cafleet-design-doc/reference/coordination.md:35,73` | The canonical `<file>:<line>` pointer example is `cafleet/src/cafleet/cli/main.py:142` — a Python path that does not exist. | Replace with a real path from the Rust tree, e.g. `cafleet/src/cli/member.rs:142`. |
| D5 | `cafleet-design-doc/execute/execute.md:387`, and the `git log` globs at `execute.md` and `execute/roles/programmer.md` | Routes review findings by `**/test_*.py`, `**/*_test.py`, `**/tests/**`, and passes `'**/test_*'` to `git log`, in a Rust repo. | This is a **semantic** fix, not only a glob swap. Rust tests live in two places: integration tests at `cafleet/tests/**/*.rs`, and unit tests as `#[cfg(test)]` modules **inside** the source file they cover. The routing rule must therefore state that a finding inside a `#[cfg(test)]` module routes to the Tester even though the file is a source file — a pure glob cannot express this. |

### C. Path canonicalization — one owner

`cafleet/reference/base-dir.md:24-29` § *Consumer contract* already owns the canonicalize-then-resolve procedure as a two-row table. The same procedure is restated near-verbatim in `create/create.md:58`, `execute/execute.md:74`, `interview/interview.md:78`, `presentation/presentation.md:75-77`, and `report/report.md` Step 0 — a direct conflict with the one-owner-per-enumeration rule in `.claude/rules/documentation-tables.md`.

Each of the five sites is replaced by a link to the owning table plus a one-clause summary naming only that consumer's canonical form (`design-docs/<slug>` or `researches/<topic-slug>`). The accepted-input examples (`0000060-foo`, `design-docs/0000060-foo/design-doc.md`, …) stay at the call site: they are workflow-specific argument handling, not the shared enumeration.

### B. Director sections — `supervision.md` absorbs the invariants

Five director role files exist, totalling 534 lines:

| File | Lines (`wc -l`) |
|---|---|
| `cafleet/roles/director.md` | 93 |
| `cafleet-design-doc/create/roles/director.md` | 98 |
| `cafleet-design-doc/execute/roles/director.md` | 138 |
| `cafleet-research/report/roles/director.md` | 83 |
| `cafleet-research/presentation/roles/director.md` | 122 |


**Fold target: `cafleet/reference/supervision.md`.** It is Required-reading row #3 in all five workflow bodies (`create.md:15`, `execute.md:15`, `interview.md:15`, `report.md:15`, `presentation.md:15`), so folded content lands in a file every workflow Director demonstrably already loads — no new Required-reading row, and no added Read hop. `cafleet/roles/director.md` is **not** the target: it is named nowhere in the four workflow director files, so folding there would require adding the very gated read this choice avoids.

**Provenance of the fold target.** The original scoping decision named `cafleet/roles/director.md`, chosen expressly to avoid imposing an extra Read hop on a spawning agent. Verification then showed that none of the four workflow director role files requires that page, so implementing the choice as written would have contradicted the reason it was made — the fold would have needed a new gated read. The substitution to `supervision.md`, which every workflow Director already loads at row #3, was put to the user as an explicit question and **approved by the user** over both the original target and dropping group B. It is recorded here as a settled decision, not as this design's proposal.

Each section merges into supervision.md's existing structure rather than being appended as a new one:

| Section | Variance across the four | Disposition |
|---|---|---|
| LLM Intent Judgment | Byte-identical (create/execute) | Merge into supervision.md § *User Delegation Protocol* — it governs interpreting the user's free-form reply |
| Abort Detection | Differs on where feedback goes (design doc vs changed source files) | Merge the shared shape into § *User Delegation Protocol* beside LLM Intent Judgment; each workflow keeps only its feedback target |
| User delegation for a member's relayed question | Byte-identical (create/execute) | Merge into § *User Delegation Protocol* |
| Progress Monitoring | First sentence differs (wake sources); remainder verbatim | Merge the invariant remainder into § *Stall Response*; each workflow keeps its one-sentence wake-source delta |
| Shutdown Protocol | Canonical teardown sentence verbatim; execute adds a Step-8 precondition, presentation adds Slidev / agent-browser teardown | Merge the canonical teardown sentence into § *Cleanup Protocol*; each workflow keeps its genuine extra steps |
| Routing member bash requests | Identical but for the member-role list (create/execute **only**) | Merge into § *Team-facilitation instructions*, phrased over "the workflow's spawned members". The existing pointer to the `cafleet` skill § *Routing Bash via the Director* is preserved — `reference/prompt-routing.md` remains the owner of the invocation detail |
| Placeholder convention | Differs only in the member-id token list (create/execute **only**) | **Not** a supervision.md fold. `cafleet/SKILL.md` § *Placeholder convention* already owns the invariant sentence, and each file already points at it; only the per-workflow role-token list is genuine. Reduce each to the token list plus the existing pointer |
| Skill-specific milestones, Your Accountability, COMMENT Marker Handling | Fully workflow-specific | Leave in place |


### A. Shared team-bootstrap preamble

Two classes of non-gated boilerplate collapse here. The Required-reading tables themselves are untouched throughout — `.claude/rules/coding-agent-overlay.md` mandates row #1 at each reader entry point, so only non-gated prose moves. The lossless rule in `cafleet/reference/director.md` is likewise untouched: no `IMPORTANT:` line, hard role-constraint, or start cue in any per-role delta is collapsed.

#### A1. The trailing resolve sentence (27 files)

27 of the 45 files carry "Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect." *in addition to* their Required-reading row #1, which already says "read **and resolve** it", and `cafleet/SKILL.md` § *Resolve your overlay*, which owns the rule including the emission self-check.

**Precondition verified**: all 27 files carry both a Required-reading block and an overlay row — no file has the sentence without the row, so none needs the row added instead. The three files with a Required-reading block but no resolve sentence (`cafleet/SKILL.md`, `reference/base-dir.md`, `reference/supervision.md`) are unaffected.

`coding-agent-overlay.md` mandates the overlay as row #1; it does not mandate this trailing sentence. Removing it leaves the rule carried by the row and the SKILL.md section.

#### A2. Bootstrap blocks across the five workflow bodies

**Owning pages: `base-dir.md` and `supervision.md` — two pages, not one.** Both are already gated in all five workflow bodies (row #2 and row #3 respectively), so neither adds a Read hop. A single new preamble page was rejected: it would have no coherent subject and would need a new Required-reading row — the exact hop § B was rewritten to avoid. Each block goes to the page that already owns its subject matter:

| Block | Sites | Owning page | Why |
|---|---|---|---|
| `cafleet doctor` prerequisites | `create.md:48`, `execute.md:57`, `interview.md:48` (byte-identical but for "anyone"/"the Analyzer"); `report.md:71`, `presentation.md:98` (shorter variant) | `supervision.md` § *Spawn Protocol* | It is a precondition of the monitor-first spawn that section already governs |
| `fleet create` + never-shell-variables | 5 bodies | `supervision.md` § *Spawn Protocol*, pointing at `cafleet/SKILL.md` § *Required Flags* | SKILL.md already owns "never your own exported shell variables" |
| Spawn-prompt audit-file two-step | 5 bodies | `base-dir.md` § *No-bypass write protocol* | It already owns the `${BASE}/.prompts/<role>-<UTC-compact>.md` convention and the `<unset>` guarded skip |
| Spawn-mechanics / path-by-reference | `report.md:121`, `presentation.md:116` (byte-identical) | `base-dir.md`, pointing at `reference/director.md` § *Spawn prompt size limit* | The blockquote already ends by naming both as canonical — it is a restatement of content they own |

The last two are one combined blockquote in `report.md` / `presentation.md` and separate prose in the design-doc bodies; they are split to their owners rather than kept together.

### Guard test

`cafleet/tests/docs_sync.rs` already exists, already asserts over `skills/` paths, and already provides `assert_terms` / `assert_absent` helpers. The guard extends that file rather than adding a new one.

Three invariants. Each is a static check over the repo tree, and each fails on a distinct real defect class:

| Invariant | Catches | Status on the unmodified tree |
|---|---|---|
| No skill file references a repository path that does not exist on disk | D4 (`cafleet/src/cafleet/cli/main.py`) | **Fails** — a present defect |
| Every role file's Required-reading row #1 names the reader's overlay | A future fold silently dropping the gated overlay read | Passes — regression guard |
| Every `{token}` used anywhere in `skills/` belongs to the known vocabulary, and every backend overlay defines all ten | A token introduced with no overlay value and no documented default, which resolves to a literal brace at spawn | Passes — regression guard |

The path check must tolerate the legitimate non-path uses of slash-bearing text (URLs, glob patterns, `<placeholder>` forms). It asserts over paths that look repo-relative and resolve under the repo root.

**Why the token invariant is a vocabulary check, not an "unresolved token" check.** Base files carrying `{skill_loader}` / `{monitor_model}` / `{decision_surface}` for the reader to resolve *is* the overlay mechanism `.claude/rules/coding-agent-overlay.md` mandates — 37 of the 45 files carry such a token deliberately, so a test forbidding them would fail on the tree's intended design. The defect worth catching statically is a token entering the vocabulary without a home: the check asserts every brace token in `skills/` is one of the ten overlay placeholders, one of the four `str.format` identity placeholders (`{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}`), or one of a small named set that has a documented home outside the overlay mechanism — `{token}` / `{placeholder}` (meta-references inside the resolution rule itself), `{slug}` / `{dir_path}` (workflow-local path variables), and `{topic}` / `{current_year}` / `{current_month}` (web-researcher discovery-query examples). It further asserts each of the three overlays plus `_template.md` defines all ten. Naming that third set explicitly is what keeps the invariant green on the current tree while a genuinely homeless token still fails. The emission-time self-check in `cafleet/SKILL.md` § *Resolve your overlay* — a token surviving into a rendered prompt or a sent message — is an agent-runtime concern with no artifact in the repo, so no static test can cover it.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Structural guard test — authored first, confirmed red

The nonexistent-path invariant is written **before** the content fixes so its red run is against the real pre-fix tree. Written after Step 2, D4's `main.py` would already be gone and the invariant could never fail, leaving it unproven. The other two are regression guards with no present-tree defect to fail on; they are expected to pass from the outset.

- [x] Add the nonexistent-path invariant to `cafleet/tests/docs_sync.rs`, tolerating URLs, globs, and `<placeholder>` forms <!-- completed: 2026-08-02T20:08 -->
- [x] Run `mise //cafleet:test` and confirm the nonexistent-path invariant **fails** on D4's `main.py` before any fix lands <!-- completed: 2026-08-02T20:08 -->
- [x] Add the overlay-row-#1 invariant across every role file; expect it to pass on the unmodified tree <!-- completed: 2026-08-02T20:08 -->
- [x] Add the token-vocabulary invariant (ten overlay placeholders + four `str.format` identity placeholders; every overlay defines all ten); expect it to pass on the unmodified tree <!-- completed: 2026-08-02T20:08 -->

### Step 2: Correctness fixes (D1–D5)

- [x] Delete `cafleet/roles/director.md` § *Monitoring aggregate previews* entirely; leave no residue or deprecation note (D1) <!-- completed: 2026-08-03T05:11 -->
- [x] Rewrite `cafleet/roles/member.md:62-69` to name the two-byte-identical-captures mechanism, preserving the durable `last_stall_check_at` cadence fact (D2) <!-- completed: 2026-08-03T05:11 -->
- [x] Replace "Click's standard not-a-valid-integer error" at `cafleet/SKILL.md:111` with the neutral wording from `cli-options.md:172` (D3) <!-- completed: 2026-08-03T05:11 -->
- [x] Replace `UsageError` at `cafleet/SKILL.md:89` and `cafleet/reference/director.md:58` with the contract strings owned by `cli-options.md:811`; leave `str.format` untouched (D3) <!-- completed: 2026-08-03T05:11 -->
- [x] Replace the dead `main.py` pointer example at `cafleet-design-doc/reference/coordination.md:35,73` with a real Rust path (D4) <!-- completed: 2026-08-03T05:11 -->
- [x] Rewrite the test-file routing rule at `execute/execute.md:387` for Rust: route by `cafleet/tests/**/*.rs` **and** by a finding landing inside a `#[cfg(test)]` module of a source file (22 files under `cafleet/src/` carry one) (D5) <!-- completed: 2026-08-03T05:11 -->
- [x] Replace the `git log` pathspec filter in `execute/execute.md` and `execute/roles/programmer.md` with an unfiltered commit-range read (`git log <base>..HEAD --name-only`, no pathspec). `'**/test_*'` matches nothing in this tree and no pathspec can reach a `#[cfg(test)]` module, so the filter is dropped rather than corrected; the Programmer identifies test content from the Tester's commit range instead (D5) <!-- completed: 2026-08-03T05:11 -->

### Step 3: Path canonicalization — one owner (C)

- [x] Confirm `cafleet/reference/base-dir.md` § *Consumer contract* covers both consumers correctly; extend it if a call site carries a rule the table lacks <!-- completed: 2026-08-03T05:17 -->
- [x] Replace the restatement in `create/create.md:58` with a link plus one-clause summary, keeping the accepted-input examples <!-- completed: 2026-08-03T05:17 -->
- [x] Same for `execute/execute.md:74` <!-- completed: 2026-08-03T05:17 -->
- [x] Same for `interview/interview.md:78` <!-- completed: 2026-08-03T05:17 -->
- [x] Same for `presentation/presentation.md:75-77` <!-- completed: 2026-08-03T05:17 -->
- [x] Same for `report/report.md` Step 0 <!-- completed: 2026-08-03T05:17 -->

### Step 4: Fold invariant Director sections into `supervision.md` (B)

- [x] Merge LLM Intent Judgment, Abort Detection's shared shape, and User delegation into `supervision.md` § *User Delegation Protocol* as one coherent section, not three appended ones <!-- completed: 2026-08-03T05:23 -->
- [x] Merge the invariant remainder of Progress Monitoring into § *Stall Response* <!-- completed: 2026-08-03T05:23 -->
- [x] Merge the canonical teardown sentence into § *Cleanup Protocol* <!-- completed: 2026-08-03T05:23 -->
- [x] Merge Routing member bash requests (present in `create` and `execute` only) into § *Team-facilitation instructions*, generalizing the member-role list and preserving the pointer to `reference/prompt-routing.md` <!-- completed: 2026-08-03T05:23 -->
- [x] Reduce Placeholder convention to its role-token list plus the existing `cafleet/SKILL.md` pointer — in `create/roles/director.md` and `execute/roles/director.md` only; the report and presentation director files carry no such section <!-- completed: 2026-08-03T05:23 -->
- [x] Reduce all four workflow director files to their genuine deltas, verifying each retains its workflow-specific wake sources, feedback targets, extra teardown steps, and milestone table <!-- completed: 2026-08-03T05:23 -->
- [x] Confirm no Required-reading row was added or removed anywhere in this step <!-- completed: 2026-08-03T05:23 -->

### Step 5: Shared team-bootstrap preamble (A)

- [x] A1: remove the trailing resolve sentence from all 27 files, confirming per file that Required-reading row #1 is present before removing; flag to the Director any file that has the sentence without the row rather than choosing silently <!-- completed: 2026-08-03T05:34 -->
- [x] A2: move the `cafleet doctor` prerequisites and the `fleet create` + never-shell-variables block into `supervision.md` § *Spawn Protocol*; replace each of the 5 sites with a one-clause pointer <!-- completed: 2026-08-03T05:34 -->
- [x] A2: move the spawn-prompt audit-file two-step and the spawn-mechanics / path-by-reference blockquote into `base-dir.md` § *No-bypass write protocol*; replace each site with a one-clause pointer <!-- completed: 2026-08-03T05:34 -->
- [x] Verify no `IMPORTANT:` line, hard role-constraint, or start cue was collapsed in any per-role delta (the lossless rule), and that every Required-reading table is byte-unchanged <!-- completed: 2026-08-03T05:34 -->

### Step 6: Verify

Both checks run against the repository tree; nothing outside it is written (§ *Promotion is out of scope*).

- [x] Run `mise //cafleet:test` and confirm all three invariants pass, including the nonexistent-path one that was red in Step 1 <!-- completed: 2026-08-03T05:40 -->
- [x] Run `mise //cafleet:lint` and confirm clean <!-- completed: 2026-08-03T05:40 -->
