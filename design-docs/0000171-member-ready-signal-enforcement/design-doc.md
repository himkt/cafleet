# Member Ready-Signal Enforcement via the Canonical Spawn-Prompt Skeleton

**Status**: Approved
**Progress**: 12/15 tasks complete
**Last Updated**: 2026-08-19

## Overview

The on-spawn ready-signal directive — mandated by `skills/cafleet/reference/supervision.md` Spawn Protocol step 3 — currently rides only per-role start cues, and ten of the repo's fifteen ordinary member spawn deltas omit it. This design moves the directive into the fixed frame of the canonical spawn-prompt skeleton so every skeleton render inherits it, removes the now-redundant per-role start-cue prefixes, normalizes the two self-contained skeletons (`clean-docs`, `skill-author`), and makes the `skill-author` guide teach the requirement.

## Success Criteria

- [ ] The canonical spawn-prompt skeleton in `skills/cafleet/reference/director.md` carries the fixed ready-signal line, so rendering any workflow's per-role delta yields a prompt containing it.
- [ ] `rg "Send your on-spawn ready signal" skills/ .claude/skills/ docs/` returns zero hits — no start-cue delta row restates the directive (single owner).
- [ ] The canonical ready-signal line appears verbatim in exactly four skeleton blocks: the `director.md` canonical skeleton, the `clean-docs` skeleton, and the `skill-author` § 3 anatomy skeleton and worked-example prompt.
- [ ] `supervision.md` Spawn Protocol step 3 names the skeleton frame as the directive's carrier and retains the "a prompt missing it is a defect — fix and re-spawn" rule.
- [ ] The `skill-author` guide teaches the ready-signal requirement in its spawn-prompt anatomy section.
- [ ] Every ordinary-member spawn section in the five consuming workflow files (`create.md`, `interview.md`, `execute.md`, `report.md`, `presentation.md`) still renders the canonical skeleton by anchor link, so all of them inherit the line with no per-workflow edit.

---

## Background

`skills/cafleet/roles/member.md` § *On spawn — send the ready signal (FIRST ACTION)* defines the member-side protocol: a spawned member's very first Bash call sends `ready` to the Director, which dispatches the member's first task on that signal (dispatch-on-ready, `supervision.md` § *Spawn Protocol*). The ready signal is the only evidence that the coding agent inside the pane has actually booted. Supervision.md Spawn Protocol step 3 therefore mandates that **every spawn prompt** include the directive.

The spawn prompt is the only channel that can deliver the instruction: no member role file lists `roles/member.md` in its Required-reading table (each cites it only for § *Shutdown*), and the directive must fire before the member reads anything else.

An audit of every ordinary-member spawn surface found three coexisting mechanisms and ten missing surfaces:

| Spawn surface | Mechanism today | Carries the directive |
|---|---|---|
| create Drafter (`skills/cafleet-design-doc/create/create.md`, normal + resume columns) | — | no |
| create Reviewer (`create/create.md:126`) | start-cue prefix | yes |
| interview Analyzer (`skills/cafleet-design-doc/interview/interview.md:100`) | — | no |
| execute Programmer / Tester / Verifier (`execute/execute.md:171,190,204`) | start-cue prefix | yes |
| execute Reviewer (`execute/execute.md:312`) | start-cue prefix | yes |
| report Manager / Scout / Researcher (`skills/cafleet-research/report/report.md:130,159,211`) | — | no |
| presentation Presentation / Transcript / Visual Reviewer (`skills/cafleet-research/presentation/presentation.md:129,154,250`) | — | no |
| clean-docs scanner / reviewer (`.claude/skills/clean-docs/SKILL.md:248`) | skeleton-level line | yes |
| skill-author guide (`.claude/skills/skill-author/SKILL.md` § 3 anatomy + worked example) | — | no (never teaches the requirement) |

The canonical spawn-prompt skeleton itself (`skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton*) has no ready-signal line — the directive is left to each per-role start cue, which is exactly the per-role duplication that drifted. The consequence is live: in the run that produced this document, the Director had to hand-patch the Drafter's spawn prompt with an ad-hoc ready-signal instruction because the create Drafter delta omits it.

All ten missing member spawns render the canonical skeleton by anchor link, so a skeleton-frame fix covers every one of them with zero per-workflow edits.

---

## Specification

### D1 — Mechanism: a fixed line in the canonical skeleton frame

The ready-signal directive becomes a **fixed line of the canonical spawn-prompt skeleton's frame** — part of the shared frame every consuming skill renders, not a per-role delta slot. Chosen over the two alternatives:

| Alternative | Why rejected |
|---|---|
| Per-role start-cue prefix in every delta row | Repeats the exact per-role duplication that drifted; every new role re-creates the risk. |
| Required-reading row for `roles/member.md` in every member role file | Front-loads a ~70-line read per member and still depends on the role file being read before the first action — weaker than a literal command in the prompt, and `member.md` itself exempts the ready handshake from its read-first gate. |

### D2 — The canonical line (contract string)

The canonical wording adopts the `clean-docs` skeleton's existing line **verbatim**:

```text
On spawn, as your first Bash call, send the ready signal: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"
```

Properties this wording is chosen for:

- `{member_id}` / `{director_member_id}` are two of the CLI's four `str.format` identity placeholders, so `cafleet member create` renders them to literal integers at spawn — the member receives a copy-pastable command, not a transformation to invent.
- Plain text, no backticks — compatible with the backtick caveat in `director.md` (harnesses whose Bash-validator hooks reject backticks).
- "as your first Bash call" preserves the FIRST-ACTION ordering from `roles/member.md` § *On spawn*.

Because the canonical wording equals the `clean-docs` line, the Q3 normalization of `clean-docs` is expected to be a verified no-op (Implementation Step 5 confirms byte-identity).

### D3 — Placement in the skeleton frame

In the canonical skeleton frame, the line sits between the `‹IMPORTANT / ROLE-CONSTRAINT LINES›` slot and the `‹START CUE›` slot, separated by blank lines. Because `director.md` folds each role's poll-handling line into the `‹IMPORTANT / ROLE-CONSTRAINT LINES›` slot, the rendered prompt reads: identity block → context lines → IMPORTANT and poll-handling lines → ready-signal line → start cue. The ready line is the last line before the start cue, so the start cue describes the member's first *substantive* action and never restates the ready signal.

For any skeleton, exactly two placement properties are contractual: the line appears **after the identity block** and **before the start cue**. Its position relative to neighboring instruction lines (e.g. the poll-handling line) is presentational, not contractual — execution ordering is carried by the line's own "as your first Bash call" wording, not by its visual position in the prompt. `clean-docs`'s existing internal order (ready line before its poll-handling line) is therefore tolerated as-is: Implementation Step 5 verifies the line's byte-identity and the two contractual placement properties only, and does not normalize neighbor order.

The `‹START CUE›` slot description in the per-role delta table gains one clause: the start cue follows the frame's ready-signal line and does not restate it.

### D4 — Wording ownership and replication rule

| Surface | Relationship to the canonical line |
|---|---|
| `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton* | **Owner.** The fixed frame carries the line; this is the single authoritative source of its wording. |
| `skills/cafleet/roles/member.md` § *On spawn — send the ready signal* | Unchanged. Owns the member-side protocol (send first, then poll, then idle); does not restate the spawn-prompt line. |
| `skills/cafleet/reference/supervision.md` Spawn Protocol step 3 | Links to the owner; does not quote the wording (§ D6). |
| `.claude/skills/clean-docs/SKILL.md` skeleton | Replicates the contract string verbatim inside its self-contained skeleton block. |
| `.claude/skills/skill-author/SKILL.md` § 3 anatomy skeleton and worked-example prompt | Replicates the contract string verbatim inside its template blocks; the surrounding prose teaches the requirement (§ D7). |

Replication of the literal line inside a **self-contained skeleton block** is legitimate and required — a skeleton is a template that must produce compliant prompts on its own, the same self-containment trade-off the repo already accepts for backend overlay sections. Prose surfaces, by contrast, link to the owner instead of quoting.

### D5 — Scope

- **Ordinary team members only** — every ordinary member spawned from a spawn prompt, including the resume-mode Drafter and the interview Analyzer, with no exceptions.
- **The monitor member is excluded.** Its `ready` + `monitor live` handshake is owned by `roles/monitor.md` and the `cafleet fleet create` bootstrap; it is not spawned from the canonical skeleton.
- **Ad-hoc spawn prompts are covered by the rule, not the skeleton.** A hand-written prompt (e.g. the inline positional-`PROMPT` form for trivial one-line spawns) must still carry the line — supervision.md step 3's defect rule binds every spawn prompt, skeleton-rendered or not.

### D6 — `supervision.md` Spawn Protocol step 3 rewrite

Current text (`supervision.md:125`):

> **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its first Bash call, to send `cafleet message send … "ready"` (canonical wording in [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing it is a defect — fix and re-spawn.

Replacement:

> **Carry the skeleton's ready-signal line.** Every spawn prompt carries the fixed ready-signal line of the canonical spawn-prompt skeleton ([`reference/director.md`](director.md) § *Canonical spawn-prompt skeleton*), instructing the member, as its first Bash call, to send its `ready` message (member-side protocol: [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). A skeleton render inherits the line automatically; a hand-written prompt must include it explicitly. It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing the line is a defect — fix and re-spawn.

### D7 — `skill-author` guide edits

The guide's § 3 *Spawn-prompt anatomy* gains the requirement in three places:

1. The anatomy skeleton block (currently `SKILL.md:146–164`): split the combined `<role-specific instructions — every IMPORTANT: line, poll-handling line, and start cue>` placeholder into three elements mirroring the canonical frame (D3) — `<role-specific instructions — every IMPORTANT: line and poll-handling line>`, then the canonical ready-signal line as a distinct fixed line, then `<start cue>`.
2. A new numbered subsection (peer of § 3.1/3.2, e.g. "§ 3.x The ready-signal line") teaches the requirement self-containedly: the line is fixed frame, not a role slot; the two identity placeholders render to a copy-pastable command at spawn; the ready signal is the only boot evidence and the Director dispatches the member's first task on it; omitting it is a spawn defect. The subsection describes the line and points to the anatomy skeleton block for the wording — it does **not** restate the contract string (D4 split: skeleton blocks replicate verbatim, prose teaches and links), keeping the Step 6 carrier sweep at exactly four hits.
3. The worked-example Summarizer prompt (currently `SKILL.md:368–388`) gains the canonical line between its poll-handling line (`SKILL.md:385`) and its start-cue line (`SKILL.md:387`), matching the canonical frame order (D3), so authors copying the example produce compliant prompts.

### D8 — Redundant start-cue prefix removal

With the frame owning the directive, the five existing start-cue prefixes are removed. Each edit deletes exactly the leading `Send your on-spawn ready signal, then ` (capitalizing the following word); the remainder of every start cue is preserved verbatim:

| File and row | New start-cue opening |
|---|---|
| `create/create.md:126` (create Reviewer) | `Wait for the Director to assign a document for review (cafleet body: ready (doc)). …` |
| `execute/execute.md:171` (Programmer) | `Read the design document and wait for the Director to assign your first step.` |
| `execute/execute.md:190` (Tester) | `Read the design document and wait for the Director to assign your first step.` |
| `execute/execute.md:204` (Verifier) | `Read the design document and discover available tools. Wait for the Director to assign your first verification task.` |
| `execute/execute.md:312` (execute Reviewer) | `Read the design document and the branch diff. Then act on the Director's ready (doc) assignment.` |

These edits change the canonical delta rows themselves, so the lossless rule (`director.md` § *Canonical spawn-prompt skeleton*) is satisfied going forward — the delta tables remain the authoritative inventory, and future collapses reproduce the new cues verbatim.

### D9 — Enforcement and non-goals

- **Docs-level enforcement only**: supervision.md step 3's defect rule plus the lossless-rule reconstruction check. No automated markdown-content test is added (user decision, Q4 — the repo has no markdown-content test infrastructure and this change does not introduce one).
- **No `README.md`, `SPEC.md`, or `docs/` change**: no CLI, schema, or HTTP contract surface changes, and `docs/docs/concepts/monitoring.md` already describes dispatch-on-ready behaviorally without quoting spawn-prompt wording (user decision, Q5).
- The ten missing workflow deltas need **no edit**: all of them render the canonical skeleton by anchor link and inherit the line from the frame.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

All changes are documentation edits to skill files; the documentation-first order is inherently satisfied. Order follows the ownership dependency: owner first, then linking prose, then consumers.

### Step 1: Canonical skeleton (`skills/cafleet/reference/director.md`)

- [x] Insert the canonical ready-signal line (D2) into the skeleton frame between `‹IMPORTANT / ROLE-CONSTRAINT LINES›` and `‹START CUE›` (D3) <!-- completed: 2026-08-19T10:50 -->
- [x] Add a short prose note after the frame stating the line is fixed frame (not a delta slot), that the CLI renders its two identity placeholders at spawn, and update the `‹START CUE›` slot description with the does-not-restate clause (D3) <!-- completed: 2026-08-19T10:50 -->

### Step 2: Spawn Protocol (`skills/cafleet/reference/supervision.md`)

- [x] Replace Spawn Protocol step 3 with the D6 wording <!-- completed: 2026-08-19T10:52 -->

### Step 3: Remove redundant start-cue prefixes (D8)

- [x] `skills/cafleet-design-doc/create/create.md:126` — create Reviewer start cue <!-- completed: 2026-08-19T10:53 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:171` — Programmer start cue <!-- completed: 2026-08-19T10:53 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:190` — Tester start cue <!-- completed: 2026-08-19T10:53 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:204` — Verifier start cue <!-- completed: 2026-08-19T10:53 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:312` — execute Reviewer start cue <!-- completed: 2026-08-19T10:53 -->

### Step 4: `skill-author` guide (`.claude/skills/skill-author/SKILL.md`)

- [x] Add the canonical line to the § 3 anatomy skeleton block (D7.1) <!-- completed: 2026-08-19T10:57 -->
- [x] Add the teaching subsection under § 3 (D7.2) <!-- completed: 2026-08-19T10:57 -->
- [x] Add the canonical line to the worked-example Summarizer prompt (D7.3) <!-- completed: 2026-08-19T10:57 -->

### Step 5: `clean-docs` normalization check

- [x] Verify `.claude/skills/clean-docs/SKILL.md:248` is byte-identical to the canonical line and satisfies the two contractual placement properties (after the identity block, before the start cue — neighbor order is not normalized, D3); edit only if drifted (expected: no-op, D2) <!-- completed: 2026-08-19T11:06 -->

### Step 6: Verification sweeps

- [ ] `rg "Send your on-spawn ready signal" skills/ .claude/skills/ docs/` returns zero hits <!-- completed: -->
- [ ] `rg "On spawn, as your first Bash call, send the ready signal" skills/ .claude/skills/` hits exactly the four carrier blocks: `director.md` skeleton, `clean-docs` skeleton, `skill-author` anatomy skeleton, `skill-author` worked example <!-- completed: -->
- [ ] Confirm every ordinary-member spawn section in `create.md`, `interview.md`, `execute.md`, `report.md`, `presentation.md` still links the canonical skeleton anchor (`director.md#canonical-spawn-prompt-skeleton`), so each render inherits the line <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-19 | Initial draft |
| 2026-08-19 | Review round 1: pinned the ready line's contractual placement (after the identity block, before the start cue; neighbor order presentational — clean-docs tolerated as-is), specified the skill-author skeleton split and worked-example insertion point, and barred the D7.2 teaching subsection from quoting the contract string |
