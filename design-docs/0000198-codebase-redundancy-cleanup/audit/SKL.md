# Audit: SKL — agent-facing documentation and project config

## Summary

The largest waste in this slice is copy-pasted member scaffolding: the seven design-doc role files repeat an identical 13-line Required-reading block plus loader/broker/shutdown boilerplate, and the six clean-docs role files are ~85% identical across workflows (affirmative vs simplification differ in ~30 of ~80 lines). The second cluster is procedure restatement across the cafleet skill: the Director wake loop, the member→Director→user relay, the "routed request carries exact command/cwd/environment/refusal" rule and the Director's mode-dependent permission decision each appear in 4–8 files, and `director.md` / `member-runtime.md` carry the same automatic-review event table near-verbatim. Several stale contracts were found: `director.md`'s spawn-skeleton slot table and lossless rule point at per-role delta tables in `create/create.md` / `execute/execute.md` that no longer exist (and the execute Reviewer expects a `BASE BRANCH:` line nobody defines), shipped skills assert project-specific hooks/bans that this repository does not have, and several skills point at `~/.claude/rules/*.md` files absent on the maintainer host. The generated `skills/cafleet/reference/runtime/` mirror is 15 symlinks into `docs/docs/` (no content duplication; its link graph is closed), so it needs no change. Total estimated removable: ~545 lines, plus ~1.6 KB of duplicated CSS data URL.

## Test placement inventory

None in slice.

## Findings

### SKL-01: Consolidate the six clean-docs role files into two shared roles
- Category: duplication
- Location: .claude/skills/clean-docs/{residue,affirmative,simplification}/roles/{scanner,reviewer}.md (6 files, 75–81 lines each, 465 lines total)
- Evidence: `diff affirmative/roles/scanner.md simplification/roles/scanner.md` differs only in the title, the workflow name in row 4 of Required reading, the class letters (P1/P2/P4 vs P3/P5) and one zero-behavior-change bullet; `diff affirmative/roles/reviewer.md simplification/roles/reviewer.md` differs only in checks 3/5/6 and the post-apply sentence. The Required-reading table (rows 1–4), "Coordination protocol" paragraph, "Authority"/"Hard limits" and "Shutdown" sections are shared by all six (residue/roles/reviewer.md:14-25, 64-78 vs the same blocks in the others). The post-apply verdict flow is also restated from reference/review-format.md:58-70 in both judgment reviewers.
- Proposal: Create `clean-docs/roles/scanner.md` and `clean-docs/roles/reviewer.md` holding the shared startup, coordination, authority, apply-gate and shutdown text once. Move each workflow's role-specific obligations (scan mechanics, class letters, reviewer checks, run pointer) into a "Scanner obligations" / "Reviewer checks" section of its workflow body (`residue.md`, `affirmative.md`, `simplification.md`), which the spawn prompt already loads. Update the SKILL.md skeleton's role path and "Skill file layout" accordingly (see SKL-04).
- Spec impact: none — maintainer-only skill under `.claude/skills/`, not embedded in the binary; every obligation survives verbatim in one place.
- Risk: medium — role text is the member's authoritative instruction set; the split must keep each workflow's reviewer checks intact.
- Estimated delta: -200 lines

### SKL-02: Hoist the design-doc member startup block into one shared section
- Category: duplication
- Location: skills/cafleet-design-doc/create/roles/drafter.md:5-17,21,28,81-83; create/roles/reviewer.md:5-17,21,30,64-66; execute/roles/programmer.md:5-17,21,29,90-92; execute/roles/tester.md:5-17,21,29,81-83; execute/roles/verifier.md:5-17,21,29,76-78; execute/roles/reviewer.md:5-17,21,30,68-70; interview/roles/analyzer.md:5-17,21,31,84-86
- Evidence: `grep -rlF "Open this authoritative role first. Use an available non-shell text reader; prerequisite file reads may use shell when it is the only reader."` → 7 files; `grep -rcF "Codex/OpenCode load the cores, own backend and required references by absolute path"` → 1 in each of the same 7. Each file also repeats `Load the listed skills at startup. Skill loading: {skill_loader}.`, `Broker protocol (poll/ack/send, ids from your spawn prompt, never the user directly): the cafleet skill core.` and a `## Shutdown` section that only says "Per `skills/cafleet/roles/member.md` § *Shutdown* — nothing is required of you." (7 hits). Rows 1–3 of the table restate skills/cafleet/roles/member.md:5-17, which row 2 already requires reading.
- Proposal: Add one `## Member startup` section to skills/cafleet-design-doc/SKILL.md containing the 5-row table and the loader/routing paragraph. Replace each role's Required-reading block with a single sentence linking it, drop the per-role "Load the listed skills" and "Broker protocol" lines, and delete the seven Shutdown sections (member.md owns shutdown).
- Spec impact: none — read order and obligations are unchanged; members already load the design-doc core (row 4).
- Risk: low — one extra link hop; the startup gate stays before the first assignment.
- Estimated delta: -90 lines

### SKL-03: Condense interview.md to the pointer style used by create.md / execute.md
- Category: doc-duplication
- Location: skills/cafleet-design-doc/interview/interview.md:5, 33-35, 44-52, 56-64, 82-100, 116, 130
- Evidence: Line 5 restates the Required-reading rows 1–2. "Prerequisites" (33-35) repeats Step 0.3's `cafleet doctor` (64) and Spawn Protocol step 2. Step 0.1 (58-62) restates guidelines.md:76-78 File Layout and base-dir.md Step 0 inline. 2a/2b (82-94) restate supervision.md:15-33 (registration, arming, poll/reconcile/ACK). The audit-file rule is stated three times (98 size-limit note, 100 callout, 116). "Shared spawn deltas" (44-52) says "Every role and mode" and "per-role tables below" although the workflow has one member role (Analyzer, table at 108-114), and 52 restates director.md:306/325. Line 130 restates the `--json` truncation rule from coordination.md:15 / cafleet SKILL.md:119. create.md and execute.md express the same steps in 1–2 sentences each.
- Proposal: Rewrite Steps 0 and 2a–2e as short pointers (normalize per File Layout, resolve BASE, follow Spawn Protocol / Director wake, render the Analyzer delta), keep the Analyzer delta table, the progress-check table, the interview loop and the `question.md` format; merge "Prerequisites" into Step 0; reword "Shared spawn deltas" for the single Analyzer role.
- Spec impact: none — workflow behavior, Analyzer delta, `question.md` format and round rules are unchanged.
- Risk: low — the removed text duplicates linked owners.
- Estimated delta: -35 lines

### SKL-04: Remove copied registries and repeated sections from clean-docs SKILL.md
- Category: doc-duplication
- Location: .claude/skills/clean-docs/SKILL.md:140-143, 157-159, 188-193, 237-245, 247-267
- Evidence: "Skill file layout" (247-267) is a hand-maintained copy of the directory tree (Describing Groups: copied registry). The `${BASE}` convention is stated twice (140-143 and 188-193). Step 7 (157-159) copies the validation task list owned by .claude/rules/commands.md:5. "Backend-neutrality" (237-245) restates .claude/rules/coding-agent-overlay.md and the already-present Required reading (32).
- Proposal: Delete "Skill file layout"; keep the `${BASE}` convention section and reduce step 1 to a link; make step 7 "the repository's Rust validation tasks (per `.claude/rules/commands.md`)"; delete "Backend-neutrality" (the overlay rule binds all skills).
- Spec impact: none — maintainer skill text only.
- Risk: low
- Estimated delta: -30 lines

### SKL-05: Replace clean-docs' private spawn skeleton with a per-role delta on the canonical frame
- Category: duplication
- Location: .claude/skills/clean-docs/SKILL.md:195-235
- Evidence: skills/cafleet/roles/director.md:273-275 states every CAFleet-native team skill spawns from the one shared frame with only a per-role delta; interview.md:106-114 does exactly that. clean-docs carries a full 40-line copy that has drifted from the canonical frame (director.md:279-304): it lacks "That file is your authoritative role definition… Re-read it whenever you are unsure of protocol.", lacks the "Resolve the executing backend loader…" line, and rewords the host-prerequisites line.
- Proposal: Replace the code block with a delta table (ROLE TITLE/TEAM, role path, extra skill loads, context lines, IMPORTANT lines verbatim, start cue) rendered on the canonical skeleton.
- Spec impact: none — spawn prompts gain the canonical frame lines they currently miss; IMPORTANT lines are preserved verbatim.
- Risk: low
- Estimated delta: -25 lines

### SKL-06: Make supervision.md § Director wake the only home of the wake/reception loop
- Category: doc-duplication
- Location: skills/cafleet/reference/supervision.md:78-86, 96-98; skills/cafleet/roles/director.md:409-411; skills/cafleet-design-doc/interview/interview.md:92-94; .claude/skills/clean-docs/SKILL.md:269-271; .claude/skills/skill-author/SKILL.md:32, 62; skills/cafleet-design-doc/create/create.md:13; execute/execute.md:13
- Evidence: supervision.md "Team-facilitation instructions" (80-86) is a five-step list that repeats Director wake steps 1–5 (29-33) in the same file; "Stage 1 — Message-based check" (96-98) repeats wake step 3. director.md:409-411, interview.md:94 and clean-docs SKILL.md:271 each paraphrase the same poll → reconcile → record → ACK → inspect roster/panes/artifacts loop before linking supervision.
- Proposal: Delete supervision "Team-facilitation instructions" (keep its "Routing member command requests" subsection, see SKL-11) and fold Stage 1 into one sentence linking wake step 3. In the other files keep only the link to [Director wake](supervision.md#director-wake).
- Spec impact: none — one canonical procedure remains.
- Risk: low
- Estimated delta: -25 lines

### SKL-07: Trim director.md sections that restate other owners
- Category: doc-duplication
- Location: skills/cafleet/roles/director.md:66-76, 257-263, 269, 378, 403-407, 412-413
- Evidence: "Cross-references" (403-407) repeats the Required-reading action table (21-27) and the JSON link already in the SKILL command index. "Model list" (257-261) restates coding-agents.md:14-16 and inference row 1 (147); 263 restates SKILL.md § Spawned-member identity. 269 restates SKILL.md:119 / supervision.md:11 (`--file` for long bodies). "Member Capture" (378) copies the JSON key list and defaults owned by docs/docs/spec/cli-options.md:934-953. "Director-only primitives" + "When you want to run your own command" (66-76) restate prompt-routing.md:5-13 and SKILL.md. 412-413 is a double blank line.
- Proposal: Delete Cross-references; reduce Model list to one sentence ("Pass a token exactly as listed in the selected backend's Model catalog") and keep only the Director-side deregistration delta from 263; drop 269; replace the capture output description with a link; merge 66-76 into one sentence linking prompt-routing.
- Spec impact: none — all contract detail stays in its owners.
- Risk: low
- Estimated delta: -20 lines

### SKL-08: Collapse the triple statement of the overlay reader contract and model-data ownership
- Category: doc-duplication
- Location: skills/cafleet/SKILL.md:35-44; skills/cafleet/reference/coding-agents.md:3-14; .claude/rules/coding-agent-overlay.md:15-35; .claude/skills/cafleet-model-list-refresh/SKILL.md:20, 243-249
- Evidence: The subject-selection procedure and the identical "a Codex Director selecting an OpenCode reviewer…" example appear in SKILL.md:44, coding-agents.md:12 and coding-agent-overlay.md:24. coding-agents.md:10 restates SKILL.md's 4-step resolution. The ownership split (model-list-refresh owns catalogs/Role defaults/freshness; runtime documentation owns bindings/notes) is stated in coding-agents.md:14, coding-agent-overlay.md:16, model-list-refresh SKILL.md:20 and again in its "Ownership boundary" section (243-249).
- Proposal: Keep the procedure in SKILL.md § Resolve your overlay (the rule already names it canonical, coding-agent-overlay.md:33); reduce coding-agents.md's preamble to the selector table plus freshness/price notes; reduce the rule's Reader contract and "How the base and overlay connect" to the author-facing requirements with a link; delete model-list-refresh "Ownership boundary" (line 20 already states it).
- Spec impact: none
- Risk: low
- Estimated delta: -20 lines

### SKL-09: Remove intra-file repetition in design-doc role files and the design-doc core
- Category: doc-duplication
- Location: skills/cafleet-design-doc/create/roles/drafter.md:3, 23, 56, 62-64; create/roles/reviewer.md:32, 46, 50, 62; execute/roles/reviewer.md:26, 38; execute/roles/tester.md:25, 75; execute/roles/programmer.md:23, 37, 76-77, 85; interview/roles/analyzer.md:3, 22, 25, 27; skills/cafleet-design-doc/SKILL.md:53
- Evidence: Drafter states "use the template, omit optional sections" three times (3, 23, 56) and has a "COMMENT Processing" section that is only a pointer already given at 30. Create Reviewer states `complete (doc) — N issues` at 32 and 46, re-lists the five criteria at 50 (19-26), and "Front-load your effort" also appears in execute/roles/reviewer.md:38. Tester repeats "Use the project's existing test patterns" (25, 75). Programmer states progress/checkbox upkeep at 23 and 76-77 and composition scope at 37 and 85. Analyzer's Accountability bullets 22/25/27 repeat Workflow 78, Output rules 73 and the footer at 65; line 3 repeats the Role-table "Does NOT". design-doc SKILL.md:53 repeats the On-demand table (32-37).
- Proposal: Keep each obligation once (Accountability for the rule, Workflow for the step) and delete the duplicates; drop Drafter "COMMENT Processing" and SKILL.md:53.
- Spec impact: none — no obligation is removed, only its second statement.
- Risk: low
- Estimated delta: -20 lines

### SKL-10: Remove internal repetition in coordination.md
- Category: doc-duplication
- Location: skills/cafleet-design-doc/reference/coordination.md:15, 21-23, 144-150, 152-154; execute/roles/programmer.md:78; execute/roles/tester.md:59
- Evidence: "Core Principle" (21-23) restates the page intro (3). "Finalize-Time Cleanup" (144-150) restates the Issue/Status split table (127-132) and guidelines.md Completeness Check (already linked at 131); its step 2 says "nothing to strip". Line 15 restates the cafleet SKILL.md poll/show `--json` and `--file` rules. "Director Per-File Detail Recovery" is re-explained in programmer.md:78 and tester.md:59.
- Proposal: Delete Core Principle, reduce Finalize-Time Cleanup to its one new fact (source-file reviewer markers are removed by the routed member's fix commit), shorten line 15 to a link, and keep the per-file recovery note only in coordination.md.
- Spec impact: none
- Risk: low
- Estimated delta: -12 lines

### SKL-11: Give the routed-request evidence rule and the Director permission decision one owner each
- Category: doc-duplication
- Location: skills/cafleet/roles/director.md:3, 115, 134, 416; skills/cafleet/reference/prompt-routing.md:27; skills/cafleet/reference/supervision.md:56, 88-90; skills/cafleet/reference/member-runtime.md:11, 24, 39; skills/cafleet/reference/coding-agents.md:18, 80, 144; skills/cafleet/roles/member.md (Native interactions section)
- Evidence: "exact command, absolute cwd, required environment and refusal" appears in prompt-routing.md:27, member-runtime.md:24 and 39, director.md:134, coding-agents.md:80 and 144 and member.md (twice). "In verified auto mode decide allow/deny; otherwise obtain user permission" appears in director.md:3 and 115, prompt-routing.md:27, supervision.md:56, member-runtime.md:11 and coding-agents.md:18. supervision.md:88-90 also restates director.md:78-86 Authority and refusal.
- Proposal: prompt-routing.md § Member-side owns the evidence fields; member-runtime.md step 5 owns the mode-dependent decision. Every other site keeps a link plus at most a one-clause summary (echo rule).
- Spec impact: none
- Risk: medium — these are safety-relevant instructions; each surviving pointer must stay on the reader's path (Director role, member role).
- Estimated delta: -12 lines

### SKL-12: Deduplicate the automatic-review event table
- Category: doc-duplication
- Location: skills/cafleet/roles/director.md:129-139; skills/cafleet/reference/member-runtime.md:18-29; launch-policy defaults also at member-runtime.md:3 and coding-agents.md:18
- Evidence: Both tables list the same four rows (`automatic_refusal_observed` with/without routed request, the Codex 0.163.0 three-denial interruption, `automatic_review_unavailable`) with near-identical procedure text. The omitted-policy defaults (`auto` / `approve-for-me` / `cafleet`) are stated in director.md:119-125, member-runtime.md:3 and the backends' Runtime bindings.
- Proposal: Keep the event table in member-runtime.md (the procedure owner) and replace director.md:129-139 with one sentence linking it; keep the launch-policy table in director.md and replace member-runtime.md:3's restatement with a link.
- Spec impact: none
- Risk: low
- Estimated delta: -10 lines

### SKL-13: Merge the duplicated CI job setup
- Category: duplication
- Location: .github/workflows/ci.yml:10-42
- Evidence: The `lint` and `test` jobs repeat the same checkout, mise-action and cache steps (15 lines each), differing only in the cache-key segment and the final task. `actions/cache@v6` is tag-referenced while every other action in ci.yml/docs.yml/release.yml/bump.yml is SHA-pinned (renovate.yaml is also tag-referenced).
- Proposal: Use one job with `strategy.matrix.task: [lint, test]`, cache key `${{ runner.os }}-cargo-${{ matrix.task }}-…` and `run: mise //cafleet:${{ matrix.task }}`; pin `actions/cache` (and renovate.yaml's actions) by SHA for consistency.
- Spec impact: none — CI behavior (two parallel checks, separate caches) is unchanged.
- Risk: low
- Estimated delta: -14 lines

### SKL-14: Keep the user-relay rule in two owners
- Category: doc-duplication
- Location: skills/cafleet/SKILL.md:99-101; skills/cafleet/roles/director.md:84, 385-387; skills/cafleet/reference/supervision.md:74-76, 108-110
- Evidence: "A member never talks to the user; it sends the question to the Director, who relays it through the decision surface and returns the answer as a message" is stated in all five places; supervision has it twice ("When you genuinely need user input" and "User Delegation Protocol").
- Proposal: Keep SKILL.md § Soliciting user reactions (the rule) and supervision § User Delegation Protocol (Director procedure). Delete supervision 74-76 and reduce director.md 385-387 to its one delta (answers go back as a broker message, not a pane keystroke) plus a link.
- Spec impact: none
- Risk: low
- Estimated delta: -8 lines

### SKL-15: Remove supervision.md sections that only restate siblings
- Category: doc-duplication
- Location: skills/cafleet/reference/supervision.md:11-13, 50-52, 112-114
- Evidence: Communication Model (11) restates SKILL.md:119-123 (fan-out, `--json`, `--file`). Sender follow-up (52) is verbatim member.md:37 plus one Director clause. "Cleanup Protocol" (112-114) only says "read Shutdown" and summarizes its steps.
- Proposal: Reduce Communication Model to its ACK/deferred-action semantics (13), keep only the Director-specific clause of Sender follow-up with a link to member.md, delete Cleanup Protocol and point its inbound links at § Shutdown.
- Spec impact: none
- Risk: low
- Estimated delta: -6 lines

### SKL-16: Fix the spawn-skeleton slot table that points at nonexistent per-role delta tables
- Category: stale-doc
- Location: skills/cafleet/roles/director.md:312-325; skills/cafleet-design-doc/execute/roles/reviewer.md:23
- Evidence: director.md:312 says "each consuming skill's spawn section fills these", 320 says "each consuming skill's delta table is the authoritative inventory", and 325 names `create/create.md`, `execute/execute.md` as delta-table owners. `grep -rn -i "delta\|per-role" skills/cafleet-design-doc .claude/skills` finds a delta table only in interview.md:44-114; create.md and execute.md have none. The ‹CONTEXT LINES› row (319) lists labels used by no skill in the repository (`SLIDE FILE`, `SERVER URL`, `OUTPUT DIRECTORY`, `LANGUAGE`, `YOUR TASK ID`, `REPORT`, `ROUND`, `CURRENT DATE`, `USER REQUEST`; grep finds only `OUTPUT PATH` in drafter.md:56). execute/roles/reviewer.md:23 requires a `BASE BRANCH:` spawn-prompt line that no workflow defines.
- Proposal: Describe the slot inventory by its selecting property ("the consuming workflow's per-role delta table, or its role files' hard constraints when the workflow has no table") instead of naming files and listing unused labels; either restore compact delta tables in create.md/execute.md (including `BASE BRANCH:` for the execute Reviewer) or point the lossless rule at the role files' Role-boundaries/Accountability lines. Pick one owner in the cleanup design doc.
- Spec impact: none for the CLI; this repairs a skill-visible contract that currently has no owner.
- Risk: medium — Directors reconstruct hard role constraints from this inventory; the chosen owner must contain every hard line.
- Estimated delta: -4 lines

### SKL-17: Drop the restated reader/startup lines under the canonical skeleton
- Category: doc-duplication
- Location: skills/cafleet/roles/director.md:308, 310, 323
- Evidence: 308 restates skeleton line 299 ("Use an available non-shell text reader…") almost verbatim; 310 restates that the ready line is fixed frame (already marked in 277-301); 323 restates skeleton line 297 (host prerequisites). The same reader/ready rule is also in SKILL.md:17, member.md:7 and skill-author SKILL.md:24.
- Proposal: Delete 308 and 323; reduce 310 to its unique fact (placeholders render to literal integers) if not already covered by 306.
- Spec impact: none
- Risk: low
- Estimated delta: -3 lines

### SKL-18: Remove project-specific claims from shipped generic skills
- Category: stale-doc
- Location: skills/cafleet/roles/director.md:336; skills/cafleet-design-doc/execute/roles/verifier.md:55, 72; skills/cafleet-design-doc/execute/roles/reviewer.md:23-24
- Evidence: director.md:336 says "some environments (including this project) run a Bash-validator hook that rejects any backtick"; .claude/settings.json has no hooks and `grep -i backtick` finds no such hook in the project. verifier.md:55/72 says `curl`/`wget` are "blocked by the project Bash ban"; neither .claude/settings.json nor .claude/rules contains such a ban. execute/roles/reviewer.md:23-24 hard-codes `.claude/rules/code-quality.md in this repo` and `mise //cafleet:test` / `mise //cafleet:lint` into a skill the binary installs for every repository.
- Proposal: Delete the backtick caveat paragraph; replace the curl/wget clauses with "use the host-permitted HTTP/browser tools"; phrase the Reviewer's checks as "the host project's code-quality rules and validation tasks".
- Spec impact: none — no CLI contract; shipped skill text becomes repository-neutral.
- Risk: low
- Estimated delta: -3 lines

### SKL-19: Drop no-op .gitignore negations (needs-verification)
- Category: other
- Location: .gitignore:3, 5, 6, 15
- Evidence: Line 1 `.*` ignores only dot-prefixed names. `!CLAUDE.md` (3) negates a path `.*` never matches; `!.claude/skills` and `!.claude/settings.json` (5-6) sit under the already re-included `!.claude` (4) and are not dot-prefixed; `/.prompts/` (15) is already matched by `.*`. Confirm with `git check-ignore -v --no-index CLAUDE.md .claude/skills/x .claude/settings.json .prompts/x` (my harness denied running it).
- Proposal: Delete lines 3, 5, 6 and 15 once the check shows identical ignore results.
- Spec impact: none
- Risk: low
- Estimated delta: -4 lines

### SKL-20: Stop listing `//cafleet:typecheck` as a separate validation gate
- Category: duplication
- Location: .claude/rules/commands.md:5; docs/docs/contributing.md:35; .claude/skills/clean-docs/SKILL.md:157-158; cafleet/mise.toml:16-18
- Evidence: `//cafleet:lint` runs `cargo clippy --all-targets -- -D warnings`, which type-checks every target (a superset of `typecheck`'s `cargo check`). The task itself is referenced by the shipped OpenCode presets (presets/opencode/cafleet.md:27, cafleet-ask.md) and SPEC.md:2146, 2218, so deleting the task would change a shipped contract.
- Proposal: Keep the task (preset/SPEC contract) but list only `test` and `lint` as the required Rust validation in commands.md, contributing.md and clean-docs step 7. Deleting the task and its preset entries is a spec change for a separate decision.
- Spec impact: none with the proposed scope — the task and preset allowlists remain.
- Risk: low
- Estimated delta: -3 lines

### SKL-21: Deduplicate the GitHub-mark data URL in styles.css
- Category: duplication
- Location: docs/theme/styles.css:171-172; also 11-13 vs 16-19
- Evidence: The same ~1.6 KB SVG data URL is written twice for `-webkit-mask` and `mask`. Separately, the comment at 16-19 says the band uses `%` (not `vw`) for identical right edges, while `--rp-content-max-width` (11-13) uses `min(90vw, 1680px)` (needs-verification: visual check whether the vw is intentional).
- Proposal: Define `--cafleet-github-mask: url("…") no-repeat center / contain;` once and use it in both properties; align or explain the vw/% discrepancy.
- Spec impact: none — documentation-site styling only.
- Risk: low
- Estimated delta: -1 line (≈ -1.6 KB)

### SKL-22: Replace backend-specific tool names in neutral base files
- Category: stale-doc
- Location: skills/cafleet/roles/director.md:76, 90; skills/cafleet-design-doc/interview/interview.md:181, 184; skills/cafleet-design-doc/execute/roles/programmer.md:45; skills/cafleet-design-doc/execute/roles/verifier.md:41-42
- Evidence: .claude/rules/coding-agent-overlay.md:3 requires base instructions to read the same on every backend. director.md:76 says "via the Bash tool", director.md:90 says "in the Director's own tmux window" although herdr is a supported multiplexer, interview.md:181/184 say "Use `Edit`" / "Use `Grep`", programmer.md:45 says "Use Grep", verifier.md:41-42 refers to `mcp__*` tool prefixes and "the system-reminder" (Claude Code specifics).
- Proposal: Use neutral phrasing ("your shell tool", "the Director's multiplexer window", "edit the document", "search the codebase", "inventory available tools, MCP servers and skills").
- Spec impact: none
- Risk: low
- Estimated delta: 0 lines

### SKL-23: Repoint references to nonexistent `~/.claude/rules/*.md` files (needs-verification)
- Category: stale-doc
- Location: .claude/skills/clean-docs/SKILL.md:222; .claude/skills/clean-docs/residue/residue.md:5-6; .claude/skills/clean-docs/affirmative/affirmative.md:6, 26, 64; .claude/skills/clean-docs/affirmative/roles/reviewer.md:47; .claude/rules/git-workflow.md:3
- Evidence: `ls /root/.claude/rules` → no such directory on the maintainer host; the referenced rules (Bash hygiene, Removal, Affirmative Writing, Git Workflow) exist as sections of `~/.claude/CLAUDE.md`. clean-docs' IMPORTANT line tells every scanner to read `~/.claude/rules/bash-command.md`. Confirm whether any maintainer environment still ships the split files.
- Proposal: Reference the rules by name as host-supplied instructions ("the host's Affirmative Writing and Removal rules"), matching the existing "use supplied session equivalents" clause, and drop the file paths.
- Spec impact: none — maintainer skill/rule text only.
- Risk: low
- Estimated delta: 0 lines

### SKL-24: Fix broken or misdirected section references
- Category: stale-doc
- Location: skills/cafleet-design-doc/reference/coordination.md:142; skills/cafleet-design-doc/interview/interview.md:181; skills/cafleet/roles/director.md:387; skills/cafleet/SKILL.md:101
- Evidence: coordination.md:142 attributes the stall-response ladder to `skills/cafleet/SKILL.md` "and the existing role files"; it lives in skills/cafleet/reference/supervision.md § Stall Response. interview.md:181 cites "§ *COMMENT(user-relay) Marker*", a heading that exists nowhere (the placement rule is interview.md § Coordination Protocol, line 31). director.md:387 and SKILL.md:101 send readers to the overlay for a "question-shape taxonomy"; no backend section has one (only the Claude note's four-option limit, coding-agents.md:76).
- Proposal: Link supervision.md#stall-response; point interview.md:181 at § Coordination Protocol; reword the two overlay pointers to "your backend's decision-surface note".
- Spec impact: none
- Risk: low
- Estimated delta: 0 lines

### SKL-25: Align the coding-agent-overlay rule with the actual backend sections
- Category: stale-doc
- Location: .claude/rules/coding-agent-overlay.md:7-14, 25; skills/cafleet/reference/coding-agents.md:86, 148, 303, 418-436
- Evidence: The rule says each backend section contains "these four subsections in order", but `claude` and `codex` add `#### <backend> native Director reception` and `opencode` adds `#### OpenCode Go`; the Template (418-436) gives no guidance for them. The rule also says each reader entry point places the overlay at "row #1 of the Required-reading block", while create.md:7, execute.md:7 and clean-docs SKILL.md:32 use prose Required-reading paragraphs.
- Proposal: State that backend-specific `####` subsections may follow Note → applies at (and add that to the Template), and phrase the entry-point requirement as "first in the Required-reading block" without mandating a table row.
- Spec impact: none
- Risk: low
- Estimated delta: 0 lines

### SKL-26: Rewrite CONTRIBUTING.md without "has moved" narration
- Category: stale-doc
- Location: CONTRIBUTING.md:3
- Evidence: "The contributor guide has moved." is historical narration (Removal rule); the file exists only as GitHub's contributing entry point.
- Proposal: "The contributor guide lives at <https://himkt.github.io/cafleet/contributing> (source: `docs/docs/contributing.md`)."
- Spec impact: none
- Risk: low
- Estimated delta: 0 lines

## Observations (outside this slice or no-change)

- `skills/cafleet/reference/runtime/**` is 15 symlinks into `docs/docs/` (verified with `ls -la`; `diff -rq` shows no content differences). Every relative link inside the mirror resolves to another mirrored page, and `cafleet/build.rs:10` `rerun-if-changed=../docs/docs` is required because `include_bytes!` follows the symlinks. No action.
- `presets/opencode/cafleet.md` and `cafleet-ask.md` share 62 of 65 lines; OpenCode loads each agent preset as a standalone file and SPEC.md reproduces both verbatim, so the duplication is contract-mandated. No action.
- DOC slice: docs/docs/spec/cli-options.md:955-960 names an implementation symbol (`CaptureSnapshot::from_raw`, against documentation-maintenance § Describe behavior, not code locations) and carries residue wording ("Existing line validation/windowing and timestamp format stay unchanged", "JSON retains its existing fields").
- Skill links are not covered by `mise //docs:build` dead-link checking (rspress checks only `docs/docs`); every skill anchor sampled in this audit resolved, but nothing enforces it.
