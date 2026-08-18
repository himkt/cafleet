# Affirmative & Simplification Sweep of docs/, skills/, README, SPEC, and Rules

**Status**: Approved
**Progress**: 20/28 tasks complete
**Last Updated**: 2026-08-18

## Overview

A one-time documentation sweep that simplifies the repository's Markdown documentation and rewrites reactive-prohibition passages affirmatively, per `~/.claude/rules/affirmative-writing.md`. The sweep covers `docs/docs/`, `skills/`, `README.md`, `SPEC.md`, and `.claude/rules/`, preserves every contract surface verbatim, and is executed via the cafleet-design-doc execute workflow using the clean-docs finding taxonomy.

## Success Criteria

- [ ] Zero prohibition-only sections (P1) and zero unpaired load-bearing prohibitions (P2) remain in touched files, Reviewer-verified.
- [ ] Every duplication cluster in § Ownership assignments is reduced to the owning location(s) listed there; every non-owning mention is a link plus at most a one-clause summary.
- [ ] All contract surfaces — CLI flags and error strings, protocol verbs and pointer forms, `{placeholder}` tokens, pane-state cues, command shapes, `SPEC.md` literals — appear unchanged in the final diff, except the edits enumerated in § Flagged edits.
- [ ] `mise //:docs-build` completes cleanly.
- [ ] Every intra-repo Markdown link and anchor in touched files resolves to an existing file/heading.
- [ ] Each edit listed in § Flagged edits is applied exactly as specified (or explicitly rejected with a recorded reason).

---

## Background

Four Scanner passes read the target trees in full and produced 105 findings: 24 in `docs/docs/` (22 simplify, 2 affirmative), 22 in `skills/cafleet` (18 simplify, 4 affirmative), 44 in `skills/cafleet-design-doc` + `skills/cafleet-research` (40 simplify, 4 affirmative), and 15 in the repo root — `README.md`, `SPEC.md`, `.claude/rules/` (14 simplify, 1 affirmative). The dominant defects are: the same fact restated in 3–9 locations (fleet-bootstrap story, `--json` truncation semantics, pointer-marker pairing rule), design-doc/maintainer voice inside spec pages, a reference-implementation register and internal source paths inside the stack-agnostic `SPEC.md`, prohibition-only "Do NOT" walls in role files, and one live spec drift caused by duplication (the slidev color cap carries two contradictory values in one file). The scan artifacts under `.scan/` are ephemeral agent scratch and stay uncommitted; this document is the self-contained specification of the sweep.

---

## Specification

### Scope

| Tree | In scope | Notes |
|---|---|---|
| `docs/docs/**/*.md` | yes | All pages including `spec/` |
| `skills/**/*.md` | yes | Three skills: `cafleet`, `cafleet-design-doc`, `cafleet-research` |
| `README.md`, `SPEC.md` | yes | First-class targets per the user's answer |
| `.claude/rules/*.md` | yes | First-class targets per the user's answer |
| Non-Markdown assets (`rspress.config.ts`, theme files, `_meta.json` / `_nav.json`, Vue/CSS) | no | — |
| `skills/cafleet/reference/coding-agent-overlays.md`, `skills/cafleet/reference/model-list.md` | no | Scanner-verified clean; per-backend repetition is the documented self-containment contract |

### Hard constraints (KEEP guardrails)

Adopted from the clean-docs shared review format; a guarded surface produces no edit.

- **Contract detail is untouchable.** Exact CLI options, error strings, output layouts, JSON key order, schemas, protocol verbs, pointer forms, `{placeholder}` tokens, pane-state cues, and command examples are preserved verbatim. In `docs/docs/spec/*` and `SPEC.md`, only surrounding prose is eligible.
- **Overlay self-containment stays.** The per-backend sections of `coding-agent-overlays.md` and the per-entry-point Required-reading blocks are explicit contracts (`.claude/rules/coding-agent-overlay.md`); their repetition is by design.
- **Disjoint-reader duplication stays.** Duplication between member role files with disjoint readers (each agent loads only its own file) is intentional self-containment — such twins are synced to identical wording where drifting, never deduplicated. Duplication between a workflow body and its Director role file (same reader) IS real cost and is deduplicated to owner + pointer.
- **Legitimate prohibitions stay.** A hard constraint stated as a prohibition paired with its affirmative counterpart is compliant. The sweep rewrites only prohibition-only passages lacking a positive spec and reactive residue.
- **No new narration.** No proposed text introduces "previously / now / formerly / renamed from / this replaces" phrasing.

### Transformation classes

Reused from the clean-docs taxonomy (its P4 code-fallback class is out of scope — this sweep touches Markdown only), plus one residue class for historical comparatives found by the scan.

| Class | Definition | Action |
|---|---|---|
| **P1 prohibition-pile** | A section that is mostly DO-NOT/NEVER items with no statement of the desired behavior they protect. | Rewrite affirmatively: state what to do and what correct looks like; keep every genuine hard constraint paired with its affirmative counterpart. |
| **P2 unpaired prohibition** | A "never X" with no "instead do Y", or a negatively-phrased instruction rewritable affirmatively with the same constraint. | Add the affirmative pairing, or rephrase as a pure affirmative instruction carrying the same constraint. |
| **P3 redundant prose** | A statement that restates what an adjacent sentence, table, linked owner page, or same-page section already says. | Delete, or merge into the surviving statement; non-owning cross-page mentions become link + one clause. |
| **P5 verbose phrasing** | Prose rewritable materially shorter with zero meaning loss (run-on multi-rule sentences, defensive over-enumeration, reviewer-addressed justification). | Rewrite tighter; every constraint, condition, qualifier, and cross-reference survives. |
| **R historical residue** | A comparative against a removed predecessor or roadmap voice ("unchanged", "much reduced from the X it replaces", "deliberately-deferred optimization", speculative future-failure warnings). | State the current behavior directly; delete the historical/speculative frame per `removal.md`. |

Style-only churn — words change but neither shrink nor clarify — is dropped, not applied.

### Ownership assignments

The single-owner decisions for every cross-location duplication cluster the scan found. The owner carries the full statement; every other listed location keeps at most a command block (where the workflow step needs one) plus a link and a one-clause summary.

| Fact | Owner | Non-owning locations reduced |
|---|---|---|
| Fleet-create bootstrap + `monitor live` gate (concept) | `docs/docs/concepts/monitoring.md` | `overview.md` § Monitoring, `fleet-isolation.md`, `quickstart.md`, `mixed-backend-team.md` ×2 |
| Fleet-create bootstrap (CLI contract) | `docs/docs/spec/cli-options.md` § fleet create | — (contract home; concept pages link to it) |
| `--json` complete/untruncated vs text truncated (cross-page) | `cli-options.md` § JSON output | `message-envelope.md` ×3 (its § JSON output stays as the page-local single mention), `cli-options.md` other prose mentions |
| Stale-assets guard semantics | `cli-options.md` § Stale-assets guard | `storage.md` § Assets-install recording |
| Monitor bootstrap mechanics + loop-launch exclusivity (skills) | `supervision.md` (Director view) + `roles/monitor.md` (actor view) | `cafleet/SKILL.md` § Team supervision, `reference/cli.md` § Monitor + Typical Workflow, second in-file statements in `supervision.md` |
| Workflow-body bootstrap restatements (9 locations) | `supervision.md` § Spawn Protocol | `create.md` 1a–1b, `execute.md` 3a–3b, `interview.md` 2a–2b, `report.md` 0b + Step 1, `presentation.md` 1a–1b, four Director role files' first Accountability bullet |
| Reuse-running-fleet rule + spawn-health audit | `supervision.md` § Spawn Protocol | `create.md` 1a/1f, `execute.md` 3a/3f |
| Overlay-resolution procedure | `cafleet/SKILL.md` § Resolve your overlay | `supervision.md` header, `reference/director.md` rendering paragraph |
| `member ping`/`member prompt` two-forms + permission split | `reference/prompt-routing.md` | `reference/director.md` § Member Prompt/Ping, `roles/director.md` § Director-only primitives |
| ARG_MAX / `--file` for long bodies | `supervision.md` § Communication Model | `cafleet/SKILL.md` § Send, `reference/cli.md`, `reference/director.md` |
| Raw-tmux ban | `recovery.md` Shutdown intro (positive rule) | `recovery.md` step 3 restatement (delete); `supervision.md` mentions become clause + pointer |
| Monitor-first-out teardown order | `recovery.md` § Shutdown Protocol | `supervision.md` § Monitor Lifecycle + § Cleanup Protocol keep the one-clause "first-out" summary |
| `<unset>` sentinel contract | `base-dir.md` § The `<unset>` sentinel | `base-dir.md` no-bypass item 3 keeps one clause + pointer |
| Pointer-marker pairing rule (~11 sites) | `cafleet-design-doc/reference/coordination.md` | All execute-tree and drafter restatements shrink to "(pairing rule, coordination.md)" |
| Separate-commands / no-`&&` rule (execute tree) | `execute/roles/director.md` § Commit Protocol Summary | Five per-site parentheticals in `execute.md` |
| "Aim for 2–3 rounds" cap (same-reader pairs) | The Director role file of each workflow | The workflow-body mention points to it |
| Register-task-first discipline | `report/roles/manager.md` § Task-Based Coordination | Two other in-file statements become one clause each |
| Slidev per-slide color cap | `reference/slidev.md` § Highlight → Usage Rules | § Color reduces to component syntax + pointer (see § Flagged edits for the value resolution) |
| Director agent-browser restriction | `presentation/roles/director.md` | `presentation.md` Step 3 item 5 becomes one clause + pointer |
| VR close handshake (Director side) | `presentation.md` Step 3 loop pseudo-code | Step 5 and `roles/director.md` § Shutdown Protocol |
| Passive-hold prohibition | `supervision.md` § Authorization-Scope Guard | Two other in-file statements become clause + section reference |
| Member Bash-conduct enumerations (detection signals, fabrication prohibitions, denial routing) | `skills/cafleet/roles/member.md` (member side) + `reference/prompt-routing.md` (Director-side dispatch) | `.claude/rules/bash-tool.md` reduces to trigger + MUST rule + links |
| "Drift is a blocker for documentation complete" rule | `documentation-maintenance.md` § First-class documentation targets (as one owning statement over a targets table) | The § Implementation Order README bullet links instead of restating |

### Flagged edits (behavior-affecting or decision-carrying)

Per the user's answer to Q2, deletions of reactive residue that change what agents or readers are literally instructed are allowed but must be flagged individually. These are the flagged edits; everything else in the sweep is voice- and behavior-preserving rewording.

1. § Schema-only invocation, in both `cli-options.md` and `SPEC.md` §6.3 — delete the verbatim-equivalent absence-framed sections; fold any affirmative sentence not already present into each document's `setup` section.
2. `webui-api.md` GET /api/timeline — delete the speculative future-failure warning about ACK timestamps; retarget its dead link ("Data model § ACK timestamp inference") to `data-model.md` § Broadcast Grouping.
3. `supervision.md` § User Delegation Protocol — delete the two "What you MUST NOT do" bullets that restate step 3's positive spec; fold the two bullets with novel content (no batching of distinct questions; invoke pane primitives via your own Bash tool) into the numbered steps.
4. `prompt-routing.md` § Member-side — delete the first "routing-specific addition" (restates the section's own opening rule); keep the no-options-menu rule.
5. `recovery.md` § Shutdown Protocol step 3 — delete the per-step raw-tmux restatement (the intro's positive rule governs).
6. `execute/roles/programmer.md` — keep the Accountability bullet as the single blocked-escalate statement (delete the other two); delete the "CRITICAL: the design document MUST always reflect…" paragraph after folding its one novel clause ("if you forgot one, stop and fix before continuing") into Phase 2 step 6.
7. Role-boundary "Do NOT:" walls in `execute/roles/programmer.md`, `tester.md`, `verifier.md`, `reviewer.md` — recast as owner-naming affirmative boundary statements ("the Director owns all git operations; the Tester owns test files; blockers route to the Director via `cafleet message send`"), keeping any residual hard prohibition paired inline.
8. `presentation/roles/presentation.md` §§ Layout Selection and Information Representation — fold both one-sentence sections into § Core Rules and delete the headings.
9. `reference/slidev.md` color-cap contradiction — the file carries "1-2 colored elements per slide max" and "Max 3 per slide" for the same rule. Resolution: § Usage Rules' **max 3** is the single value; § Color drops its number entirely.
10. Accuracy corrections (aligning prose to the actual contract): `quickstart.md` § Codex — add `network_access = true` to the TOML snippet, which the adjacent prose already requires; `fleet-isolation.md` Lifecycle table — "run inside tmux" becomes "inside a tmux or herdr session".
11. `SPEC.md` §6.7 opencode preset block — no sweep change. The SPEC block and the shipped artifact `presets/opencode/cafleet.md` are byte-identical; the disagreement (the body text claims read-only `gh` queries plus the PR comment/review endpoints are allowlisted while the frontmatter `bash` map carries no `gh` rules, and the frontmatter's trailing comma sits under a "JSON (not YAML)" framing) is internal to the shipped artifact. The artifact is a runtime permission surface, outside this documentation sweep's scope, so the sweep leaves both the artifact and the SPEC block untouched. This entry is the recorded follow-up defect: the frontmatter permission map is the enforced surface and wins — the follow-up corrects the artifact body's `gh` claim to match the frontmatter, resolves the trailing comma against opencode's actual parser behavior, and re-syncs SPEC §6.7 byte-for-byte.
12. `.claude/rules/bash-tool.md` — reduce to the trigger conditions, the MUST rule, and links; the member role file owns the member-side enumerations and `prompt-routing.md` owns the Director-side dispatch mechanics (see § Ownership assignments). This changes the rule text agents load, with meaning preserved via the owning files every member already reads.

### Per-tree acceptance rules

| Tree | A touched file is acceptable when… |
|---|---|
| `docs/docs/` | It follows `user-facing-docs.md` (behavior, not code locations — `webui-api.md` loses `list_roster(...)` and index names), `documentation-tables.md` (tabulate threshold, cell caps, one owner per enumeration, term tie-break to the Core terms table), and spec-page prose reads as contract, not rationale ("accepted trade-off", "kept as defense", "legitimately carries" removed). |
| `skills/` | Contract surfaces (verbs, pointer forms, command shapes, tag taxonomies, templates) are byte-identical; same-reader duplication is owner + pointer; disjoint-reader twins are synced, not thinned; Required-reading blocks and overlay structure untouched. |
| `README.md` | Only the thin surface (pitch, install, section links) — swept for P3/P5 without adding content; sync via the `/update-readme` skill if the sweep's docs edits move owned facts. |
| `SPEC.md` | Prose framing only; every contract literal (options, error strings, schema, key order, layouts) byte-identical; stays descriptive (no recommendations). Stack-agnostic register: no reference-implementation attribution, no internal source paths — each contract stated directly by role. |
| `.claude/rules/` | Rules themselves satisfy `affirmative-writing.md` (positive spec first, prohibitions paired); no per-rule content loss. |


### Verification gates

| Gate | Method |
|---|---|
| Docs site builds cleanly | `mise //:docs-build` exits 0 |
| Links and anchors resolve | Every intra-repo Markdown link and `#anchor` in touched files points at an existing file/heading (checked by enumerating links in the diff'd files) |
| Table conventions hold | Touched `docs/` pages re-checked against `documentation-tables.md`, including rendered-HTML inspection of any piped cells |
| No prohibition-only sections remain | Reviewer scans every touched file for P1/P2 residue |
| Contract surfaces unchanged | Reviewer checks the diff: no CLI flag, error string, verb, placeholder token, pane-state cue, or `SPEC.md` literal altered, except the edits enumerated in § Flagged edits |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: docs/ tree

- [x] Apply the docs-tree ownership assignments: bootstrap story to `monitoring.md` + `cli-options.md` § fleet create; `--json` truncation to `cli-options.md` § JSON output (page-local: `message-envelope.md` § JSON output); stale-assets guard to `cli-options.md` § Stale-assets guard — non-owning mentions become link + clause <!-- completed: 2026-08-18T20:50 -->
- [x] Tabulate `data-model.md` § `monitor_runtime` (column / meaning / written-cleared-by), keeping coalescing/reclaim semantics as adjacent prose <!-- completed: 2026-08-18T20:50 -->
- [x] Remove design-doc/maintainer voice and historical residue (class R) from spec pages: `data-model.md` § fleets trade-off framing, `cli-options.md` row-count defense + "kept as defense" + "unchanged", `multiplexer-backends.md` deferred-alternative narration + meta-justification clause, `coding-agent-backends.md` § Safety-floor record-here clause <!-- completed: 2026-08-18T20:50 -->
- [x] `webui-api.md`: replace implementation names (`list_roster(...)`, index name) with behavioral descriptions; let the three-endpoint comparison table own shared attributes; apply flagged edit 2 <!-- completed: 2026-08-18T20:50 -->
- [x] Apply flagged edit 1 (`cli-options.md` § Schema-only invocation deletion) <!-- completed: 2026-08-18T20:50 -->
- [x] Apply the remaining docs-tree P3/P5 findings: `contributing.md` tables bullet, `overview.md` § Monitoring, `monitoring.md` intro + § Keystroke safety, `member-lifecycle.md` intro split + multiplexer-neutral terms, `model-selection.md` duplicate clause, `coding-agents.md` heading restatement, `storage.md` guard paragraph, `mixed-backend-team.md` duplicate bootstrap paragraphs, `coding-agent-backends.md` table-cell/section duplication <!-- completed: 2026-08-18T20:50 -->
- [x] Apply flagged edit 10 (accuracy corrections in `quickstart.md` and `fleet-isolation.md`) <!-- completed: 2026-08-18T20:50 -->

### Step 2: skills/cafleet tree

- [x] `SKILL.md`: compress § Team supervision to 2–3 sentences + the two existing pointers; split the § Send run-on into three sentences; shorten the Required-reading row #1 loss cell to "unresolved `{token}`s, guessed values, ignored backend notes" — mirrored in lockstep in the three role files <!-- completed: 2026-08-18T21:00 -->
- [x] `supervision.md` deduplication: re-engagement channels enumerated once (Facilitation cue), passive-hold owned by § Authorization-Scope Guard, Idle-Semantics bullets defer to the gate table's state→action rows, single freshness definition, loop-launch exclusivity stated once + pointer to `roles/monitor.md`, § Cleanup Protocol teardown replay reduced to first-out clause + `recovery.md` pointer <!-- completed: 2026-08-18T21:00 -->
- [x] `supervision.md` affirmative rewrites: § Authorization-Scope Guard leads with the positive persistence rule; apply flagged edit 3 (§ User Delegation); § Spawn Protocol doctor bullet states the two positive rules with the do-NOT enumeration compressed to one paired clause <!-- completed: 2026-08-18T21:00 -->
- [x] `reference/cli.md` + `reference/director.md` + `roles/director.md`: split the multi-rule run-on sentences (`reference/cli.md` § Coding-agent backends; `reference/director.md` § Member Ping; `roles/director.md` § Model selection); in `reference/director.md`, move the `--role` cell detail to adjacent prose, end the rendering paragraph at the lockstep sentence, and reduce the two-forms sentence and permission split to pointers at `prompt-routing.md` <!-- completed: 2026-08-18T21:00 -->
- [x] Apply flagged edits 4 and 5 (`prompt-routing.md`, `recovery.md`) <!-- completed: 2026-08-18T21:00 -->
- [x] `base-dir.md`: hoist the absolute-path caveat to one sentence below the consumer table; `<unset>` contract single-homed per the ownership table; no-bypass item 2 drops the restated spawn mechanics (pointer stays). `roles/member.md` wait-loop enumeration compressed to the positive rule + single-poll allowance; `roles/monitor.md` step 5 rationale aside cut to one clause <!-- completed: 2026-08-18T21:00 -->

### Step 3: skills/cafleet-design-doc + skills/cafleet-research trees

- [x] Reduce the 9-location bootstrap restatements to command block + pointer sentence (`supervision.md` § Spawn Protocol owns), including the reuse-running-fleet and spawn-health-audit twins <!-- completed: 2026-08-18T21:18 -->
- [x] Shrink all pointer-marker pairing restatements across the execute tree and `create/roles/drafter.md` to "(pairing rule, coordination.md)"; `coordination.md` verb-choice paragraph keeps only the when-in-doubt heuristic <!-- completed: 2026-08-18T21:18 -->
- [x] Compress the four Director role files to responsibility + pointer: Accountability bullets stop re-narrating workflow steps (judgment content stays), milestone tables state the stall-nudge rule once above the table, § Shutdown/§ User Delegation defer to their owners; fix the "Critical Review Checklist" dangling reference to "§ Review & Feedback" <!-- completed: 2026-08-18T21:18 -->
- [x] Apply flagged edits 6, 7, and 8 (programmer.md consolidation; the four "Do NOT:" wall recasts; presentation.md section folds) plus the remaining execute-tree P5 items (execute.md opening paragraph, run-to-completion ownership, Phase B item 1 reduction, separate-commands rule to Commit Protocol owner) <!-- completed: 2026-08-18T21:18 -->
- [x] Apply flagged edit 9 (slidev color-cap resolution) and consolidate `slidev.md` color guidance into the § Highlight block <!-- completed: 2026-08-18T21:18 -->
- [x] Apply the remaining report/presentation same-reader dedupes (report.md ACK/user-delegation/rounds-cap pointers; manager.md three-homes items; researcher/scout delegation-bullet compression with thresholds owned by `web-researcher.md`; presentation-tree handshake/agent-browser/threshold re-pointering; visual-reviewer table-cell trims) and sync the flagged disjoint-reader twins (reviewer front-load paragraph) to identical wording <!-- completed: 2026-08-18T21:18 -->
- [x] `create/create.md` + `interview/interview.md` residual items: resume-detection parenthetical compressed to one clause; drafter MANDATORY double-framing reduced to one; interview 2c size-limit rationale reduced to the director-reference pointer <!-- completed: 2026-08-18T21:18 -->

### Step 4: README.md, SPEC.md, .claude/rules/

- [ ] `README.md`: split the pitch run-on into two–three short sentences (what it is; how it works; who it is for), thin surface preserved; `/update-readme` sync if the Step 1 docs edits moved owned facts <!-- completed: -->
- [ ] `SPEC.md` register sweep: state each contract directly without reference-implementation attribution (§2, §5.6, §6.1, §6.8, §11) or internal source paths (schema-version guard, server launcher, §6.5); delete the doc-grouping reconciliation paragraph; single-home the reference-parity non-goal in § Non-goals; drop both "absence is a valid, well-defined state" justification clauses (§6.5, §7.1) and the §6.7 "keep each as written" sentence; rewrite the §5.4 and schema-version-guard closings as plain invariants (class R) <!-- completed: -->
- [ ] Apply flagged edit 1 in `SPEC.md` (§6.3 deletion); per flagged edit 11, leave the §6.7 preset block untouched (the artifact defect is recorded there as a follow-up) <!-- completed: -->
- [ ] `.claude/rules/` sweep: `commands.md` `--` separator condition becomes one plain rule ("pass test-name filters bare; insert `--` before any `--`-prefixed arg"); `bash-tool.md` per flagged edit 12, including deleting the prior-protocol comparison sentence and the § Director side restatement; `documentation-maintenance.md` first-class targets become a target/update-trigger/drift-consequence table with one owning blocker statement, § Implementation Order linking instead of restating <!-- completed: -->

### Step 5: Verification

- [ ] Run `mise //:docs-build` and confirm a clean exit <!-- completed: -->
- [ ] Verify every intra-repo link/anchor in touched files resolves; re-check `documentation-tables.md` conventions (incl. rendered HTML for piped cells) on touched docs pages <!-- completed: -->
- [ ] Reviewer pass: zero P1/P2 residue in touched files; every § Flagged edit applied as specified or rejected with recorded reason <!-- completed: -->
- [ ] Reviewer diff check: no contract surface altered (CLI flags, error strings, verbs, placeholder tokens, pane-state cues, `SPEC.md` literals), except the edits enumerated in § Flagged edits <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-17 | Initial draft |
| 2026-08-17 | Fold in root-slice evidence (README/SPEC/.claude-rules): flagged edits 11–12, two ownership rows, evidence-based Step 4 |
| 2026-08-17 | Review round 1: touched-files criterion, owning-location(s) wording, flagged-edits carve-out in criterion/gate/task, edit 11 respecified as recorded follow-up (artifact-internal defect, frontmatter wins), Step 2 file-to-section mapping |
