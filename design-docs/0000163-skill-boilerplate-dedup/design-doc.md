# Skill Boilerplate Dedup

**Status**: Approved
**Progress**: 7/65 tasks complete
**Last Updated**: 2026-08-07

## Overview

Simplify the three skills under `skills/` (cafleet, cafleet-design-doc, cafleet-research) by applying the behavior-preserving reductions identified in the four scan reports at `.scan/`. The change deduplicates pointer-with-restatement boilerplate into single canonical owners and applies the per-skill reductions the reports identify, with zero behavior change and zero edits to `cafleet/tests/docs_sync.rs`.

## Success Criteria

- [ ] Every finding from the four scan reports is dispositioned by this doc's Scope tables, and every in-scope finding is applied to the tree.
- [ ] `mise //cafleet:test` passes with zero edits to `cafleet/tests/docs_sync.rs`.
- [ ] Each hoisted block (B1–B11) has exactly one canonical owner; every former copy is a pointer or a genuinely role/workflow-specific delta, verified by the per-block sweep table (Specification § *Per-block ownership sweeps*) executed in Step 9.
- [ ] The three single-copy facts (poll-id-vs-task-id caveat, `--coding-agent` substitution rule, Slidev on-start-failure escalation) each exist at their canonical home, and their owner-add commits land before any copy deletion.
- [ ] No file under `.scan/` is committed; no skill file is moved or renamed.
- [ ] Deployed skill replicas are refreshed with a `cafleet setup` re-run after the edits land.

---

## Background

A four-member simplification scan (fleet 25) produced read-only reports over the skills tree, stored uncommitted at `.scan/`:

| Report | Slice | Findings |
|---|---|---|
| `.scan/cafleet-skill.md` (CF) | `skills/cafleet/` interior | F1–F11 |
| `.scan/cafleet-design-doc.md` (DD) | `skills/cafleet-design-doc/` interior | Findings 1–18 |
| `.scan/cafleet-research.md` (CR) | `skills/cafleet-research/` interior | F1–F18 |
| `.scan/cross-skill.md` (XS) | Content duplicated between the three skills | F1–F12 |

The dominant pattern is **pointer-with-restatement**: a copy that names its canonical owner and then restates the owner's content anyway. The reports enumerate every copy with file:line references and trace behavior preservation per reader entry path. The reports remain on disk during implementation as the detailed file:line enumeration source; this document is the authoritative scope, ownership, and sequencing record. Line numbers in the reports refer to the tree at scan time and drift as edits land — locate by section name, not line number.

User decisions already made: the coding-agent overlay files stay separate (no `overlays.md` merge); medium-risk findings are included with the reports' stated mitigations; the structural member-frame hoist (XS-F12) is deferred.

---

## Specification

### Deduplication blocks and canonical owners

Each block below is one hoisted unit of boilerplate. The owner is the single file/section that carries the full content after the change; every other copy becomes a pointer (naming the owner's path and section) plus any genuinely role- or workflow-specific delta, which is preserved verbatim.

| Block | Content | Canonical owner | Copies reduced | Source findings |
|---|---|---|---|---|
| B1 | Required-reading row-#1 "What you lose" cell + preamble framing | `skills/cafleet/SKILL.md` § *Resolve your overlay* | 27 entry points (3 cafleet, 13 design-doc, 11 research) | XS-F1, DD-16, CR-F17 |
| B2 | Monitor-launch mechanism (launch via `{bg_run}`, confirm `monitor loop started (…)`, gate spawns) | `skills/cafleet/reference/supervision.md` § *Spawn Protocol* | 5 workflow bodies + 4 Director role bullets + 3 intra-supervision repeats | XS-F2, CR-F3, CF-F7 |
| B3 | `--coding-agent <backend>` substitution rule | `supervision.md` § *Spawn Protocol* → *Fleet bootstrap* (**owner-add**) | 5 workflow-body sites + 4 role embeds; `reference/cli.md` § Typical Workflow points at the owner | XS-F3, DD-12 (part), CR-F3 (part) |
| B4 | Five-step fleet teardown sequence | `skills/cafleet/reference/recovery.md` § *Shutdown Protocol* | 5 workflow bodies adopt the pointer + delta form already used by the 4 Director role files | XS-F4 |
| B5 | Member Shutdown stanza (two drifted dialects) | `skills/cafleet/roles/member.md` — new § *Shutdown* (**owner-add**; wrap-up-first variant becomes the general rule) | 12 consumer member role files get a one-line pointer; visual-reviewer keeps its CLOSE handshake | XS-F5, DD-11, CR-F1 |
| B6 | Member Communication Protocol (broker cycle, never-talks-to-user, id provenance, poll-id caveat) | `skills/cafleet/SKILL.md` (cycle + never-talks-to-user; § *Poll* gains the poll-id caveat as an **owner-add**) and `roles/member.md` (id provenance) | 13 member role files + the 2 research Director role files keep one pointer sentence + role deltas | XS-F6, DD-10, CR-F2 |
| B7 | Spawn-prompt two-stage rendering mechanics (identity placeholders, `[INSERT …]`, brace doubling) | `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton* | 5 consumer spawn sections (+4 short in-file repeats) shrink to one clause keeping the "brace rules at the skeleton" cue | XS-F7 |
| B8 | Consumer IMPORTANT lines and the two poll-handling forms | Consumer delta tables own their IMPORTANT/coordination lines; `director.md` § Lossless rule names them by label; `director.md` (delta-slot table) owns the two poll-handling form strings | director.md drops verbatim consumer quotes; 6 ack-inline parentheticals reduce to the named form (first expansion per workflow file kept) | XS-F8, CR-F5, CF-F11 |
| B9 | Placeholder-convention section (angle-bracket tokens) | `skills/cafleet/SKILL.md` § *Placeholder convention* | 7 design-doc member role files delete the section; Director role variants keep token roster + pointer | XS-F9, DD-1 |
| B10 | Director stall-response ladder (2 nudges → capture → user escalation) | `supervision.md` § *Stall Response* → *Escalation* | 2 design-doc Director role files reduce to true deltas; supersedes DD-3 | XS-F10, DD-3 |
| B11 | tmux/herdr + `cafleet doctor` prerequisite | `supervision.md` § *Spawn Protocol* | 3 Prerequisites lines stay at one sentence; 4 parentheticals inside fleet-create blocks and Director bullets are dropped | XS-F11 |

Block-specific mitigations (from the reports, all adopted):

- **B1**: every row keeps the word "overlay" (docs_sync guard), the "read **and resolve**" directive, the *Resolve your overlay* pointer, and one per-file token example (`{bg_run}` for Directors, `{skill_loader}` for members, `{task_coord}` where role-relevant). The preamble keeps the selector clause "Identify your coding agent first — your spawn prompt's `CODING AGENT:` line names it".
- **B2**: workflow bodies keep the workflow delta — which spawns the startup-line confirmation gates and which step stops the loop. The 4 Director role bullets keep their "Load the `cafleet` skill and Read its `reference/supervision.md`" clause (their only mechanism source after compression).
- **B5**: the pointer names the file path explicitly (`skills/cafleet/roles/member.md` § *Shutdown*) so a member can follow it lazily; the one-line summary "nothing is required of you" rides in the pointer sentence. Unification removes the dialect drift — research members gain the wrap-up-first instruction.
- **B6**: each copy becomes a single pointer *sentence*, never a bare deletion, so the cafleet-skill load dependency stays visible. Preserved deltas: Manager/Scout/Researcher hub-and-spoke restrictions, Verifier any-time relay note, visual-reviewer report-shape note, research pane-silence norm. The scope includes the two research Director role files (`report/roles/director.md`, `presentation/roles/director.md`), whose Communication-Protocol paragraphs get the same treatment with their Director-specific deltas preserved (e.g. the presentation Director's VR-batch stall nuance).
- **B8**: naming the consumer lines by label ("the Programmer no-commit line") preserves the drift tripwire without duplicating the strings.
- **B10**: the canonical escalation option list in `supervision.md` governs; the role files keep only the true deltas (capture with `--lines 200`; never silently `member delete` and re-spawn). The false framing sentence "Two skill-specific rungs are NOT in those skills" is dropped.

### Single-copy facts (owner-adds land first)

Facts that today exist **only** in copies slated for deletion, or only in a non-owner file. Each MUST be written at its canonical home before any copy is deleted — Implementation Step 1 is dedicated to this.

| Fact | Current only home(s) | Canonical home after this change |
|---|---|---|
| Poll-id-vs-task-id caveat ("the poll `id:` integer is the cafleet message id — distinct from any harness task-list id") | 3–5 cafleet-research role-file copies; absent from `skills/cafleet/` entirely | `skills/cafleet/SKILL.md` § *Poll* |
| `--coding-agent <backend>` substitution rule | Workflow-body copies + `cafleet/reference/cli.md` § Typical Workflow | `supervision.md` § *Spawn Protocol* → *Fleet bootstrap* |
| Slidev-server on-start-failure escalation (retry once → `{decision_surface}` options) | `presentation/roles/director.md` § Server Lifecycle Management only | `presentation/presentation.md` Step 3 |

Separately, B8 carries a **verification precondition** that is not an owner-add: every consumer IMPORTANT line quoted in `director.md` § Lossless rule must be verified to exist verbatim in its owning delta table (`create/create.md`, `execute/execute.md`) before the quote is replaced with a label — Step 1's B8-precondition task.

### Scope: finding disposition

Dispositions: **In (Bn)** — applied as part of block Bn; **In (Step n)** — applied as a per-skill reduction in that implementation step; **Superseded** — covered by a block that resolves it differently; **Out** — not applied, rationale recorded below.

**Cross-skill report (XS):**

| Finding | Action | Disposition |
|---|---|---|
| XS-F1 | Overlay row-#1 cell + preamble standardization | In (B1) |
| XS-F2 | Monitor-launch sentence dedup | In (B2) |
| XS-F3 | `--coding-agent` substitution dedup | In (B3) |
| XS-F4 | Teardown sequence → pointer + delta form | In (B4) |
| XS-F5 | Member Shutdown stanza hoist | In (B5) |
| XS-F6 | Member Communication Protocol dedup | In (B6) |
| XS-F7 | Skeleton rendering mechanics dedup | In (B7) |
| XS-F8 | Reverse duplication in `director.md` | In (B8) |
| XS-F9 | Placeholder-convention dedup | In (B9) |
| XS-F10 | Stall-rung drift resolution | In (B10) |
| XS-F11 | tmux/herdr prerequisite dedup | In (B11) |
| XS-F12 | Structural shared member frame | Out |

**cafleet interior report (CF):**

| Finding | Action | Disposition |
|---|---|---|
| CF-F1 | Overlay consolidation into one `overlays.md` | Out |
| CF-F2 | Overlay tie-break paragraph → pointer to `supervision.md` § *The pre-ping capture gate* (3 overlays + `_template.md`) | In (Step 6) |
| CF-F3 | Denial semantics → `prompt-routing.md` single owner | In (Step 6) |
| CF-F4 | `monitor scan` / `member capture` prose → `director.md` owner; `cli.md` points | In (Step 6) |
| CF-F5 | `member list` columns → `cli.md` owner; `director.md` keeps the supervision-specific sentence | In (Step 6) |
| CF-F6 | `str.format` error strings → `SKILL.md` owner; `director.md` keeps the exit-2 rollback delta | In (Step 6) |
| CF-F7 | Intra-supervision monitor-launch repeats → § Spawn Protocol canonical | In (B2) |
| CF-F8 | `recovery.md` 2-stage health-check restatement → two sentences + link | In (Step 6) |
| CF-F9 | `cli.md` truncation stated twice → fold into parent § Output switch | In (Step 6) |
| CF-F10 | Self-TOC preamble sentences ×3 → cut to first sentence | In (Step 6) |
| CF-F11 | Consumer role bullets in `director.md` § Lossless rule → labels | In (B8) |

**cafleet-design-doc interior report (DD):**

| Finding | Action | Disposition |
|---|---|---|
| DD-1 | Delete Placeholder-convention section from 7 member role files | In (B9) |
| DD-2 | Delete the three Architecture ASCII-diagram sections | In (Step 7) |
| DD-3 | Hoist stall two-rung block into `coordination.md` | Superseded (B10) |
| DD-4 | Compress spawn-block frames to one full example per workflow file | In (Step 7) |
| DD-5 | Path-canonicalization bullets → one sentence + `base-dir.md` § Consumer contract pointer | In (Step 7) |
| DD-6 | Delete the two Additional-resources sections | In (Step 7) |
| DD-7 | Cut restated escalation loop from `execute/roles/director.md` | In (Step 7) |
| DD-8 | COMMENT-scan procedure → workflow body owns; role files keep role-only residue | In (Step 7) |
| DD-9 | Team-composition matrix → `execute/roles/director.md` owns; `execute.md` 3c keeps pointer + Reviewer-never-initial invariant | In (Step 7) |
| DD-10 | Member Communication-Protocol lead paragraphs | In (B6) |
| DD-11 | Shutdown block single-sourcing | In (B5) |
| DD-12 | Director-bootstrap bullet trim | In (B2/B3) |
| DD-13 | Merge Interview Progress Tracking into § `question.md` Format | In (Step 7) |
| DD-14 | Delete `SKILL.md` restated dispatch line | In (Step 7) |
| DD-15 | Pairing-rule citations → "(pairing rule)" after first use per file | In (Step 7) |
| DD-16 | Overlay-cell observation (no per-skill action proposed) | Superseded (B1) |
| DD-17 | Trim `drafter.md` accountability bullets to obligation + § Workflow pointer | In (Step 7) |
| DD-18 | Collapse `programmer.md` Phase 1.5 to one numbered list | In (Step 7) |

**cafleet-research interior report (CR):**

| Finding | Action | Disposition |
|---|---|---|
| CR-F1 | Shutdown paragraph ×5 hoist | In (B5) |
| CR-F2 | Communication-Protocol boilerplate ×7 (poll-id caveat owner-add first) | In (B6) |
| CR-F3 | Monitor-launch / fleet-bootstrap ×4 | In (B2/B3) |
| CR-F4 | Prerequisites paragraph → one sentence per workflow body | In (Step 8) |
| CR-F5 | ack-inline parenthetical → named form after first expansion | In (B8) |
| CR-F6 | Per-role spawn-mechanics repetition → audit-file filename only | In (Step 8) |
| CR-F7 | Scout cap ×3 → `report.md` owner; Manager keeps clause; `scout.md` bullet deleted | In (Step 8) |
| CR-F8 | Scout/Researcher shared File Output bullets → `report.md` hoist (preferred variant; **no** file merge) | In (Step 8) |
| CR-F9 | `presentation.md` role-file echoes of slidev.md/visualization.md → link + one clause | In (Step 8) |
| CR-F10 | Tag-taxonomy re-enumerations → keep link, drop enumerations | In (Step 8) |
| CR-F11 | Slidev/agent-browser teardown ×4 → `presentation.md` Steps 3/5 own; escalation moved first | In (Step 8) |
| CR-F12 | Visual-reviewer round/persist rules stated twice → § Persist the report owns | In (Step 8) |
| CR-F13 | `slidev.md` formula rule ×3 + color tables ×2 → § Math Formulas owns the rule; colors merge into § Color Discipline | In (Step 8) |
| CR-F14 | `web-researcher.md` description rewrite (drop the cafleet-design-doc pairing) | In (Step 8) |
| CR-F15 | Report stall heuristic → `report/roles/director.md` § Progress Monitoring owns; `manager.md` copy stays | In (Step 8) |
| CR-F16 | Research `SKILL.md` internal routing-fact repetition | In (Step 8) |
| CR-F17 | Overlay row-#1 cell standardization | In (B1) |
| CR-F18 | Director visual-quality check table vs VR checks | Out |

### Out of scope, with rationale

| Item | Rationale |
|---|---|
| CF-F1 — merging the four coding-agent overlay files into one `overlays.md` | User decision: files stay separate. The per-backend file split is the isolation mechanism that keeps wrong-backend values out of every member's context, and two docs_sync tests pin the four-file layout. |
| CR-F18 — collapsing the presentation Director's visual-quality check table | The overlap is a deliberate second gate executed by a different actor ("VR verdicts have been empirically unreliable"); collapsing it would weaken the gate, i.e. change behavior. |
| XS-F12 — the structural shared "member frame" owned by `cafleet/roles/member.md` | Deferred as a possible follow-up design doc. Its four faces (B1, B5, B6, B9) are applied independently here; the structural change touches 13 role files plus the spawn-skeleton conventions and needs its own design cycle. |

### Reconciliations between reports

| Conflict | Resolution |
|---|---|
| DD-3 proposes hoisting the stall two-rung block into `coordination.md`; XS-F10 shows `supervision.md` already owns the ladder | Adopt XS-F10 (B10): reduce both role-file blocks to true deltas over `supervision.md` § Stall Response. No `coordination.md` hoist — it would widen that file's message-schema charter and create a second owner. |
| Escalation option lists drifted ("re-spawn / redistribute / drop scope" vs "re-send / re-spawn / drop its task") | The canonical `supervision.md` list governs; the role files do not restate an option list. |
| DD-12 and CR-F3 each trim their own skill's Director-bootstrap bullet; DD-12 flags cross-skill asymmetry as a risk | Void: both skills' Director role bullets are trimmed in the same pass (B2), so no asymmetry arises. |
| DD-16 and CR-F17 defer the overlay-cell standardization as "repo-wide, out of a single-skill scope" | B1 is exactly that repo-wide pass, covering all 27 entry points across the three skills in one step. |
| XS-F3 leaves "cli.md points at the owner vs stays as a second in-skill mention" open | `cli.md` § Typical Workflow points at `supervision.md` § *Fleet bootstrap* — one owner, consistent with the rest of this change. |
| XS-F8 leaves the poll-handling-form ownership direction open | `director.md`'s delta-slot table owns the two canonical strings; consumer delta tables reference them by name ("the simple / ack-inline poll-handling form"). |
| CR-F13 leaves the color-table merge direction open | Merge the § Color tokens table into § Color Discipline's table (add a "Use for" column); § Color keeps application examples + link. Heading names *Color Discipline* and *Usage Rules* stay stable — `presentation/roles/presentation.md` and `slidev.md` link them by name. |

### docs_sync.rs constraints (zero test edits)

All guards in `cafleet/tests/docs_sync.rs` must pass unmodified. The edits are constrained accordingly:

| Guard | Constraint on this change |
|---|---|
| `every_role_file_gates_its_overlay_as_required_reading_row_one` | Every `roles/*.md` keeps a `Required-reading` heading with a `\| 1 \|` row containing the word "overlay" (B1's short cell keeps it). `report/roles/web-researcher.md` stays at its pinned path. |
| `skill_files_reference_no_path_that_is_missing_from_disk` | Every new pointer uses an existing repo path; no skill file is moved, renamed, or deleted. |
| `every_brace_token_in_skills_belongs_to_the_known_vocabulary` | New prose introduces no brace token outside the known vocabulary. |
| `every_backend_overlay_defines_the_capture_cues` | CF-F2's trim keeps, per overlay file: `working`, `stall_candidate`, `quiet`, `ambiguous`, `pre-ping capture gate` (the replacement pointer sentence names *the pre-ping capture gate* section, satisfying the last term). `_template.md` keeps `working`, `stall_candidate`, `Note → applies at`. |
| `every_backend_overlay_defines_the_full_placeholder_vocabulary` | The overlays' placeholder tables are untouched. |
| `the_supervision_contract_covers_quiet_members_and_plain_messages` | `supervision.md` keeps: `member ping`, `quiet`, `finished`, `pre-ping`, `member capture`, `cafleet monitor`, `monitor loop started`, `health-check`. |
| `the_director_and_member_roles_keep_the_ping_protocol` | `director.md` keeps `## Member Ping (manual inbox-poll)`, `member ping`, `pre-ping`, `member capture`, `then resume your work`; `roles/member.md` keeps `member ping`, `member prompt`, `Director`. |
| `the_cafleet_skill_and_bash_rule_document_the_director_ping` | `SKILL.md` keeps `cafleet monitor`, `monitor loop started`, `member ping`, `message send`, `health-check`. |
| `fixed_ping_surfaces_carry_no_nudge_vocabulary` | `recovery.md`, `prompt-routing.md`, `director.md` gain no occurrence of "nudge" (B10's delta wording lives in the design-doc role files, where the term is allowed). |

### Editing rules

- **Owner-adds before deletions.** No copy of a single-copy fact is deleted before its owner-add commit exists (Step 1 gates Steps 2–8).
- **Pointer form.** A pointer names the owning file (skill-relative path) and section by name — never a bare deletion that leaves a sentence emptier. Deltas that are genuinely role- or workflow-specific are preserved verbatim at the consumer.
- **No file moves.** Every role file is a spawn-prompt target referenced by path; every reference page is a multi-consumer canonical home. This change edits content only.
- **Historical record untouched.** `design-docs/` (other than this doc) and `.scan/` are never edited; `.scan/` is never committed.
- **Test after every step.** `mise //cafleet:test` runs at the end of each implementation step; a failure is fixed within the step that caused it (by adjusting the skill edit, never by editing docs_sync.rs).

### Per-block ownership sweeps

Executed in Step 9 with `rg` under `skills/`. Each phrase is a verbatim fragment of the long form being deduplicated, verified present in the tree at drafting time; "none" means zero hits anywhere under `skills/`.

| Block | Sweep phrase | Expected hits after the change |
|---|---|---|
| B1 | `codex has no harness task list` | None — the failure-mode enumeration lives compressed in `cafleet/SKILL.md` § *Resolve your overlay*, in its own wording. |
| B2 | `monitor loop started` | Only files under `skills/cafleet/` — consumer bodies and Director role bullets no longer quote the startup line. |
| B3 | `substitute the coding agent you are actually running on` | Exactly 1 — `supervision.md` § *Fleet bootstrap*. |
| B4 | `stop the monitor loop's background task first` | None in the 5 workflow bodies; remaining hits only in `skills/cafleet/` and the Director role files' existing delta lines. |
| B5 | `The Director terminates you via` and `You are terminated by the Director` | Exactly 1 (the unified stanza in `cafleet/roles/member.md` § *Shutdown*) and none (the research dialect disappears). |
| B6 | `do NOT speak to the user directly` | None — the canonical rule in `cafleet/SKILL.md` § *Soliciting user reactions* uses its own wording; pointer sentences do not restate this string. |
| B7 | `stray single braces` | Only files under `skills/cafleet/` (`SKILL.md`, `reference/director.md`, `reference/base-dir.md`). |
| B8 | `then act` (the ack-inline expansion's closer) | At most 1 per file, only in `cafleet/reference/director.md`, `report/report.md`, `presentation/presentation.md`. |
| B9 | `placeholders, **not** shell variables` | Exactly 1 — `cafleet/SKILL.md` § *Placeholder convention*. |
| B10 | `Do NOT skip rungs` | None. |
| B11 | `inside a tmux or herdr session` | At most 1 per design-doc workflow body (the Prerequisites sentence) and none in the design-doc Director role files; `skills/cafleet/` hits unchanged. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Owner-side additions (land before any copy deletion)

- [x] Precondition: confirm the four scan reports exist at `.scan/` (`cafleet-skill.md`, `cafleet-design-doc.md`, `cafleet-research.md`, `cross-skill.md`). They are uncommitted, so a fresh worktree or clone would lack them — if any is missing, stop and escalate to the user before editing. <!-- completed: 2026-08-07T09:21 -->
- [x] Add the poll-id-vs-task-id caveat to `skills/cafleet/SKILL.md` § *Poll*: the `id:` integer printed by `message poll` is the cafleet message id — distinct from any harness task-list id. <!-- completed: 2026-08-07T09:22 -->
- [x] Add the `--coding-agent <backend>` substitution rule to `supervision.md` § *Spawn Protocol* → *Fleet bootstrap*, keeping the existing copies' phrasing "substitute the coding agent you are actually running on" (the B3 sweep phrase) — a spawned agent's `CODING AGENT:` line names it; a standalone Director uses its own identity (e.g. Claude Code → `claude`). <!-- completed: 2026-08-07T09:23 -->
- [x] Add a new § *Shutdown* to `skills/cafleet/roles/member.md` carrying the unified stanza, opening with the design-doc dialect's phrasing "The Director terminates you via `cafleet member delete`" (the B5 sweep phrase): the pane is killed immediately, nothing is required of the member; if instead messaged to wrap up first, send one final report via `cafleet message send`, then return to the prompt. <!-- completed: 2026-08-07T09:24 -->
- [x] Move the Slidev-server on-start-failure escalation (retry once → `{decision_surface}` options) from `presentation/roles/director.md` § Server Lifecycle Management into `presentation/presentation.md` Step 3. <!-- completed: 2026-08-07T09:25 -->
- [x] B8 precondition: verify every consumer line quoted in `director.md` § Lossless rule (Programmer no-commit, Tester test-only, Verifier, all-execute-roles pair, Drafter clarifying-questions) exists verbatim in its owning delta table (`create/create.md`, `execute/execute.md`); add any missing line to its delta table. <!-- completed: 2026-08-07T09:26 -->
- [x] Run `mise //cafleet:test` — green, no test edits. <!-- completed: 2026-08-07T09:28 -->

### Step 2: B1 — overlay row and preamble standardization (27 entry points)

- [ ] Standardize the Required-reading row-#1 "What you lose if you skip it" cell to the short form in all 27 entry points, keeping per file: the word "overlay", the "read **and resolve**" directive, the *Resolve your overlay* pointer, and one role-relevant token example. <!-- completed: -->
- [ ] Compress the Required-reading preamble in the same files to the selector clause ("Identify your coding agent first — your spawn prompt's `CODING AGENT:` line names it; a standalone reader uses its own identity"), dropping the repeated "Each carries a protocol you cannot reconstruct from this page" framing where the table is 1–3 pointer rows. <!-- completed: -->
- [ ] Run `mise //cafleet:test` (row-#1 guard is the sensitive one here). <!-- completed: -->

### Step 3: Orchestration dedup — B2, B3, B4, B11

- [ ] B2: in the 5 workflow bodies (`create.md` 1b, `interview.md` 2b, `execute.md` 3b, `report.md` Step 1, `presentation.md` 1b), replace the monitor-launch restatement with a pointer to `supervision.md` § *Spawn Protocol* plus the workflow delta: which spawns the startup-line confirmation gates and which step stops the loop. `presentation.md` keeps its deliverables/roster note. <!-- completed: -->
- [ ] B2: shrink the Bootstrap accountability bullet in the 4 consumer Director role files (`create/roles/director.md`, `execute/roles/director.md`, `report/roles/director.md`, `presentation/roles/director.md`) to obligation + pointer, keeping the "Load the `cafleet` skill and Read its `reference/supervision.md`" clause. <!-- completed: -->
- [ ] B2/CF-F7: inside `supervision.md`, keep § Spawn Protocol as the canonical launch statement and the Quick Reference row; reduce the § heartbeat intro, the spawn-step-1 line, and the Lifecycle Launch row to pointer clauses (`monitor loop started` remains present in the file). <!-- completed: -->
- [ ] B3: delete the `--coding-agent` substitution paragraph at the 5 workflow-body sites and the 4 Director role embeds, leaving the existing "per `supervision.md` § *Spawn Protocol* → *Fleet bootstrap*" citation to cover both id capture and backend substitution; point `cli.md` § Typical Workflow at the owner. <!-- completed: -->
- [ ] B4: replace the 5 workflow-body teardown enumerations with the pointer + delta form ("Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol*. Workflow delta: …") — deltas: create = delete Drafter + Reviewer; interview = teardown after question-list ack, delete Analyzer; execute = delete Programmer, Tester, Verifier, Reviewer if spawned; report = delete order Researchers → Scouts → Manager; presentation = VR close-handshake + agent-browser/Slidev release steps kept verbatim. <!-- completed: -->
- [ ] B11: keep one Prerequisites sentence per workflow body; drop the tmux/herdr parentheticals inside fleet-create blocks and Director bullets. <!-- completed: -->
- [ ] Run `mise //cafleet:test` (supervision.md term pins are the sensitive ones here). <!-- completed: -->

### Step 4: Member-frame dedup — B5, B6, B9

- [ ] B5: replace the § Shutdown stanza in the 12 consumer member role files (7 design-doc, 5 research) with a one-line pointer naming `skills/cafleet/roles/member.md` § *Shutdown* — "nothing is required of you"; `visual-reviewer.md` keeps its CLOSE-handshake section unchanged. <!-- completed: -->
- [ ] B6: in the 13 member role files and the 2 research Director role files (`report/roles/director.md`, `presentation/roles/director.md`), shrink the Communication Protocol paragraph to one pointer sentence (broker cycle, ids from the spawn prompt, never the user directly — the `cafleet` skill core) plus the preserved role deltas listed in the Specification. <!-- completed: -->
- [ ] B9: delete the standalone Placeholder-convention section from the 7 design-doc member role files; the Director role variants keep only their per-workflow token roster + the canonical pointer. <!-- completed: -->
- [ ] Run `mise //cafleet:test`. <!-- completed: -->

### Step 5: Skeleton dedup — B7, B8

- [ ] B7: compress the two-stage rendering paragraph at the 5 consumer spawn sections to one clause — "Render the canonical spawn-prompt skeleton with the delta below (two-stage rendering + brace rules at the skeleton)" — and drop the later-in-file short repeats in `report.md` / `presentation.md`; workflow-specific additions (create's ~2 KB/resume-mode column, execute's load-both-skills note) stay. <!-- completed: -->
- [ ] B8: in `director.md`, replace the § Lossless rule's verbatim consumer quotes with labels and state that each consuming skill's delta table is the authoritative inventory; keep the two poll-handling form strings in the delta-slot table as their single owner. <!-- completed: -->
- [ ] B8/CR-F5: reduce the repeated ack-inline parentheticals in `report.md` / `presentation.md` delta tables to the bare named form, keeping the first expansion per workflow file. <!-- completed: -->
- [ ] Run `mise //cafleet:test`. <!-- completed: -->

### Step 6: cafleet interior reductions

- [ ] CF-F2: replace the tie-break paragraph in the three overlays with one pointer sentence ("Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*") and update `_template.md` to require the pointer instead of the restatement, preserving the capture-cue term pins per file. <!-- completed: -->
- [ ] CF-F3: make `prompt-routing.md` the single owner of the per-backend denial semantics (keep one of its two statements); shorten `roles/member.md` § denial and `supervision.md` § routing to trigger cue + link. <!-- completed: -->
- [ ] CF-F4: collapse `cli.md` § Monitor's scan/capture prose to one clause each + link to `director.md` § Fleet Scan / § Member Capture. <!-- completed: -->
- [ ] CF-F5: keep `cli.md` § List Members as the column-enumeration owner; reduce `director.md` to the supervision-specific sentence + link; retarget or keep `recovery.md`'s idle-heuristic link accordingly. <!-- completed: -->
- [ ] CF-F6: in `director.md`, drop the restated `str.format` error strings/hint; keep the exit-2-rolls-back-the-member delta. <!-- completed: -->
- [ ] CF-F8: shorten `recovery.md` § 2-stage health check to the ordering sentence + the `--lines` bump note + the existing link; keep § Routine monitoring's idle-column heuristic. <!-- completed: -->
- [ ] CF-F9: fold the `CAFLEET_MAX_TEXT_LEN` subsection's two unique facts (U+2026, spec link) into `cli.md` § Output switch and delete the subsection heading. <!-- completed: -->
- [ ] CF-F10: cut the self-TOC preamble sentences (`supervision.md`, `cli.md`, and the `SKILL.md` § Team supervision re-enumeration) to the first sentence each. <!-- completed: -->
- [ ] Run `mise //cafleet:test`. <!-- completed: -->

### Step 7: cafleet-design-doc interior reductions

- [ ] DD-2: delete the three § Architecture sections (`create.md`, `execute.md`, `interview.md`). <!-- completed: -->
- [ ] DD-4: per workflow file, state the spawn frame once (render to `${BASE}/.prompts/<role>-<UTC-compact>.md` → `cafleet member create … --file … --json` → parse `member_id`), keep ONE full code block as the worked example, and give the other roles only `--name` / `--description` (+ the Step-5 Reviewer's `--model {reviewer_model}`) in the delta tables. <!-- completed: -->
- [ ] DD-5: reduce the path-canonicalization bullet pairs in the three workflow bodies to one sentence with an explicit pointer to `base-dir.md` § *Consumer contract*; keep each workflow's branch-on-outcome paragraph. <!-- completed: -->
- [ ] DD-6: delete the § Additional resources sections in `execute.md` and `interview.md`. <!-- completed: -->
- [ ] DD-7: in `execute/roles/director.md` § Escalation Protocol, keep the pointer sentence + the commit delta; delete the restated arbitration loop. <!-- completed: -->
- [ ] DD-8: make the workflow bodies the owner of the user-feedback/COMMENT-scan procedure; shrink each Director role file's § COMMENT Marker Handling to the role-only residue + pointer; re-aim the five workflow-body links; the no-markers step survives exactly once. <!-- completed: -->
- [ ] DD-9: keep the team-composition decision matrix in `execute/roles/director.md`; reduce `execute.md` 3c to the pointer + the "Reviewer is never part of the initial composition" invariant. <!-- completed: -->
- [ ] B10: reduce the identical stall-rung block in `create/roles/director.md` and `execute/roles/director.md` to "Stall ladder per `supervision.md` § Stall Response" + the two true deltas (`--lines 200`; never silently `member delete` and re-spawn); drop the "NOT in those skills" framing. <!-- completed: -->
- [ ] DD-13: merge § Interview Progress Tracking into § `question.md` Format (keep the format example + the two non-redundant facts; delete the lifecycle bullets already owned by Step 1/Step 4). <!-- completed: -->
- [ ] DD-14: delete the restated dispatch line at the end of `cafleet-design-doc/SKILL.md`. <!-- completed: -->
- [ ] DD-15: keep the full pointer-marker pairing-rule citation on first use per file; shorten subsequent uses to "(pairing rule)". <!-- completed: -->
- [ ] DD-17: trim the two `drafter.md` accountability bullets to obligation + § Workflow pointer. <!-- completed: -->
- [ ] DD-18: collapse `programmer.md` Phase 1.5's four sub-headings into one numbered list, keeping both `complete (doc)` signals and the only-proceed-after-confirmation closer. <!-- completed: -->
- [ ] Run `mise //cafleet:test`. <!-- completed: -->

### Step 8: cafleet-research interior reductions

- [ ] CR-F4: cut each workflow body's Prerequisites to one sentence (`cafleet` on `PATH`, verified by `cafleet doctor`; the rest gated in Steps 0–1). <!-- completed: -->
- [ ] CR-F6: reduce the per-role spawn-mechanics repetitions to the audit-file filename above each `member create` fence; keep the scout naming and per-batch no-overwrite notes. <!-- completed: -->
- [ ] CR-F7: `report.md` owns the scout 3-iteration cap; `manager.md` keeps a short clause; delete the `scout.md` bullet. <!-- completed: -->
- [ ] CR-F8 (preferred variant): hoist the three shared File Output bullets into `report.md`; `scout.md` / `researcher.md` keep their path example and role-specific delta. No file merge. <!-- completed: -->
- [ ] CR-F9: cut `presentation/roles/presentation.md`'s re-enumerations to link + one clause each (layouts per `slidev.md` Layouts; charts per `visualization.md` § Chart Type Selection; emphasis per `slidev.md` § Usage Rules / Color Discipline; placeholder note per `visualization.md`). <!-- completed: -->
- [ ] CR-F10: drop the tag-taxonomy re-enumerations in `presentation.md` role files and `manager.md`; keep the canonical-taxonomy links (anchors unchanged). <!-- completed: -->
- [ ] CR-F11: `presentation.md` Steps 3/5 own the server lifecycle (escalation already moved in Step 1); shrink `roles/director.md` § Server Lifecycle Management and § Shutdown Protocol to pointers; in `visual-reviewer.md`, merge § Browser lifecycle into § Shutdown. <!-- completed: -->
- [ ] CR-F12: cut the two `visual-reviewer.md` accountability bullets to pointers at § Persist the report. <!-- completed: -->
- [ ] CR-F13: make § Math Formulas the formula rule's single owner (reduce the early mention to the cross-link; fold the table-intro prose); merge the § Color tokens table into § Color Discipline's table with a "Use for" column, keeping both heading names stable. <!-- completed: -->
- [ ] CR-F14: rewrite the `web-researcher.md` frontmatter description and mission sentence to the current purpose; drop the cafleet-design-doc pairing framing; keep the file path and the whitelisted `{topic}` / `{current_year}` / `{current_month}` tokens. <!-- completed: -->
- [ ] CR-F15: `report/roles/director.md` § Progress Monitoring owns the stall heuristic; `report.md` Step 1 keeps the deliverables list + a pointer; the `manager.md` copy stays (different actor). <!-- completed: -->
- [ ] CR-F16: in `cafleet-research/SKILL.md`, keep the On-demand table + one dispatch sentence; drop the two restatements. <!-- completed: -->
- [ ] Run `mise //cafleet:test`. <!-- completed: -->

### Step 9: Verification and deployment

- [ ] Full `mise //cafleet:test` green; `git diff` confirms zero changes under `cafleet/tests/`. <!-- completed: -->
- [ ] Execute the per-block ownership sweeps (Specification § *Per-block ownership sweeps*): every one of the B1–B11 phrases matches its expected-hits row. Additionally confirm the poll-id caveat's `distinct from` phrasing has exactly 1 hit under `skills/`, in `cafleet/SKILL.md` § *Poll*. <!-- completed: -->
- [ ] `git status` confirms nothing under `.scan/` is staged or committed. <!-- completed: -->
- [ ] Re-run `cafleet setup` for the backends in use so the deployed skill replicas pick up the edited skills. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-07 | Initial draft |
| 2026-08-07 | Reviewer round 1: B3 site count corrected to 5; single-copy-facts table split from the B8 precondition; B6 extended to the two research Director role files; `.scan/` existence precondition task added; owner-add wordings aligned with sweep phrases; per-block ownership sweep table added |
