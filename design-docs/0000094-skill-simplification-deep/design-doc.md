# Skill Simplification (Deep): Aggressive De-duplication of `skills/`

**Status**: Complete
**Progress**: 26/26 tasks complete
**Last Updated**: 2026-06-16

## Overview

The `skills/` tree has grown to 53 files / 6,923 lines / 62,340 words, of which 42 prose files (~5,849 lines) carry instructions Claude Code loads as ground truth. This design aggressively de-duplicates that prose — stating each shared convention once in a canonical home, relocating CLI internals into `docs/spec/`, and cutting non-runtime rationale — to reach a ≈30% per-file line reduction overall (≈28% net) and 32–42% on the orchestration-heavy clusters (cafleet core, design-doc-execute, research), with zero behavioral drift.

## Success Criteria

- [ ] **NOT MET (line numbers).** Target was ~5,849 → ~4,099 (30% per-file) / ~4,180 net (≈28%) with clusters 42%/38%/32%. **Actual: 4,967 per-file (~15%); 5,048 net (~14%); clusters cafleet 27% / execute 10% / research+slidev 13–20%.** Verified by `wc -l` at Step 9. The estimates assumed more redundancy than exists under the zero-drift contract — every *safe* dedup the design named was applied (verified per step), but reaching the targets would require cutting canonical, cross-referenced content (the spawn-prompt skeleton + delta tables, command indexes, workflow steps, load-bearing matplotlib/Slidev reference), which guardrails 1/2/5 forbid. Correctness was prioritized over the line count. See Changelog 2026-06-16 (Step 9).
- [x] Every shared convention (placeholder rule, verb+pointer protocol, BASE/`<unset>`, monitor heartbeat, shutdown ordering, Esc-safeguarded inline preview, spawn-prompt skeleton, member-side boilerplate) appears in **exactly one** canonical file; all other occurrences are one-line cross-references.
- [x] Every CLI internal removed from a skill (exit code, verbatim error string, rollback/timeout mechanic, internal DB field name) is preserved in `docs/spec/` or `docs/concepts/` — no information is lost, only relocated.
- [x] Zero behavioral drift: no CLI command, flag, required argument, output contract, or workflow step changes. A diff review confirms only prose/dedup/relocation edits (`git diff --stat main...HEAD`: only `skills/`, `docs/concepts/`, and this design doc changed — no source, config, or test files).
- [x] No dangling cross-references: every pointer resolves to a real file and heading anchor (independent Step-9 audit: 0 dangling / 0 suspect across ~200+ links).
- [x] Frontmatter `description:` trigger keywords are preserved on every SKILL.md (skill auto-routing unaffected); only wording is tightened — verified byte-identical at base vs HEAD.

---

## Background

Two compounding forces inflated the tree:

1. **Family-wide boilerplate copied per file.** The CAFleet-native team skills (create, execute, interview, research-report, research-presentation) each restate the same member-role preamble — the placeholder/shell-variable convention, the send/poll/ack mechanics, the ready-signal directive, the shutdown ordering, and a ~25-line spawn prompt per role. These blocks recur 4–7 times.
2. **Skills documenting CLI internals.** SKILL and reference files spell out exact exit codes, verbatim error strings, rollback mechanics, and internal DB field names (`agent_card_json.cafleet.kind`, `monitor_runtime`, `origin_task_id` threading) that already live — or belong — in `docs/spec/`. Layered on top are "Why X" rationale paragraphs (why-no-auto-exit-on-silence, why-not-`reviewDecision`, why-literal-flags) and within-file restatements (e.g. `design-doc-execute` Step 7 prints the same Copilot branch table twice; `cafleet-base-dir` defines `<unset>` three times).

The historical record for *why* these conventions exist lives in git history and design docs — not in the skill prose that agents load every session. Per the repo removal rule, the cleanup is total.

---

## Specification

### Scope

| | In scope | Out of scope |
|---|---|---|
| Files | 42 prose files: every `SKILL.md`, `roles/*.md`, `reference/*.md`, `techniques/*.md`, `template.md`, `guidelines.md`, `coordination.md` | 11 Slidev theme assets under `cafleet-my-slidev/theme/` (`*.vue`, `styles/index.css`) |
| Why | Instruction prose with redundancy to remove | Functional render code; editing risks breaking Slidev, carries no redundant instructions |
| Lines | ~5,849 | ~1,074 (untouched) |

### Approved decisions (from user)

1. **Aggressive de-duplication.** State each shared convention once in a canonical location; replace copies with terse cross-references. More inter-file coupling is acceptable — this is the primary lever.
2. **Relocate CLI internals.** Move exact exit codes, verbatim error strings, rollback mechanics, and internal DB field names into `docs/spec/` / `docs/concepts/`. Skills keep usage-focused content plus a pointer to the spec.
3. **Cut non-runtime rationale (most aggressive).** Remove any "Why X" / justification paragraph whose removal does not change an agent's runtime decision. Keep only rationale that guides a runtime decision.
4. **Frontmatter.** Keep all trigger keywords intact (do not degrade auto-routing); tighten wording and cut filler only.

### Canonical-home map (state once, reference everywhere)

The de-duplication direction is **into existing files** — almost no new shared files are needed because every convention already has a natural owner. Every other occurrence becomes a one-line pointer.

| Convention | Canonical home | Occurrences to collapse to pointers |
|---|---|---|
| Placeholder / shell-variable rule (literal ids, `permissions.allow` matches literally) | `cafleet/SKILL.md` § Placeholder convention | cafleet roles ×2; every team-skill role file (~13); create/interview/research SKILLs |
| verb + pointer + `COMMENT(role)` protocol | `cafleet-design-doc/coordination.md` | create/SKILL.md (clarification-exemption stated 3×); interview/SKILL.md inline `COMMENT(claude)` block; the role files' protocol summaries |
| BASE resolution / no-bypass write / `<unset>` sentinel | `cafleet-base-dir/SKILL.md` | its own 3× self-restatement; create-figure; slidev; every team skill's audit-file note |
| Monitor heartbeat, 5-step facilitation loop, health-check, stall response, spawn-gate | `cafleet-agent-team-monitoring/SKILL.md` | supervision (comms/spawn-gate restatements); execute/roles/director.md progress-monitoring |
| Shutdown / teardown ordering (first-out), **Director-side** | `cafleet/reference/recovery.md` § Shutdown Protocol | monitoring (Lifecycle prose); supervision § Cleanup; every team Director role + SKILL Step "finalize". **Member-side stays inline:** the passive "/exit arrives, nothing required of you" note remains in each member role (members are told to skip `reference/recovery.md`) — see guardrail 6 |
| Esc-safeguarded inline-preview mechanics | `cafleet-agent-team-monitoring/SKILL.md` § The monitor heartbeat (+ one mention in `cafleet/SKILL.md`) | ~8 occurrences across monitoring + supervision |
| Spawn-prompt skeleton (identity block + COMMUNICATION PROTOCOL + skill-load list) | `cafleet/reference/director.md` § Member Create | the 3-per-skill spawn prompts in create, execute, interview, research-report, research-presentation |
| Member-side universal boilerplate (Bash-direct rule, forbidden behaviors, ready signal) | `cafleet/roles/member.md` + `cafleet/reference/exec-routing.md` | each team-skill member role; the triple-stated "THE DEFAULT RULE" |

New in-tree file (one only): `cafleet-research-report/roles/web-researcher.md` — the ~98-line web-researcher agent spec extracted verbatim from `research-report/SKILL.md` (currently dispatched identically by `researcher.md` and `scout.md`). The Slidev `slide-creator` spec is **cut**, not relocated (it near-duplicates the host skill's own Layouts/Content/Citation rules — see per-file note), so no second new file is added.

### Relocation map (CLI internals → docs)

All destination files exist. Content moves in the same change it is removed (no orphan window).

| Content type | Examples | Destination |
|---|---|---|
| CLI-**emitted** strings, exit codes, flag-validation (agent only recognizes them) | `Error: --fleet-id <int> is required…`, `No such option: --json`, `binary <name> not found on PATH`, `command too long`, member-delete exit-2 timeout | `docs/spec/cli-options.md` |
| Skill-**instructed** contract strings (the skill tells the implementer to emit this exact literal) | `Error: BASE is <unset>; refusing to fall back to /tmp`, `audit-disabled no BASE in spawn prompt` | **keep inline at the instruction** — do NOT relocate (see guardrail 2) |
| Internal DB fields / state machine | `agent_card_json.cafleet.kind`, `monitor_config`, `monitor_runtime`, `origin_task_id` threading, `input_required`→`completed` | `docs/spec/data-model.md`, `docs/spec/message-envelope.md` |
| Helper / keystroke / lifecycle mechanics | `tmux.send_inline_preview`, `send_poll_trigger`, Esc-settle timing, SIGTERM/`finally` runtime-row clearing, 15 s `/exit` wait, default 60 s interval | `docs/concepts/monitoring.md`, `docs/concepts/member-lifecycle.md`, `docs/concepts/tmux-push.md` |
| Rejected-alternative rationale | `--full-envelope`/`--full-recipients`/… variants considered-and-rejected | this design doc only (the historical record) |

### Per-file simplification plan

Before = current `wc -l`. After = estimated target from line-referenced analysis. Primary lever keyed: **D**=de-dup to canonical home, **R**=relocate internals to docs, **W**=cut "Why"/non-runtime rationale, **X**=extract/restructure within file.

| # | File | Before | After | Cut | Levers |
|--:|---|--:|--:|--:|:--|
| | **cafleet/** | | | | |
| 1 | `cafleet/SKILL.md` | 247 | 150 | 39% | R W D |
| 2 | `cafleet/reference/director.md` | 246 | 150 | 39% | R D X |
| 3 | `cafleet/reference/broadcast.md` | 51 | 22 | 57% | R W |
| 4 | `cafleet/reference/exec-routing.md` | 72 | 38 | 47% | D R W |
| 5 | `cafleet/reference/output-flags.md` | 38 | 24 | 37% | R W |
| 6 | `cafleet/reference/recovery.md` | 64 | 38 | 41% | R W D |
| 7 | `cafleet/roles/director.md` | 52 | 34 | 35% | D R W |
| 8 | `cafleet/roles/member.md` | 81 | 40 | 51% | D X |
| | **cafleet-agent-team-monitoring/** | | | | |
| 9 | `SKILL.md` | 159 | 110 | 31% | X D R W |
| | **cafleet-agent-team-supervision/** | | | | |
| 10 | `SKILL.md` | 132 | 88 | 33% | D R W |
| | **cafleet-base-dir/** | | | | |
| 11 | `SKILL.md` | 82 | 50 | 39% | X R W |
| | **cafleet-create-figure/** | | | | |
| 12 | `SKILL.md` | 236 | 210 | 11% | D W |
| | **cafleet-design-doc/** | | | | |
| 13 | `SKILL.md` | 16 | 15 | 6% | — (index file) |
| 14 | `template.md` | 49 | 49 | 0% | — (literal template) |
| 15 | `guidelines.md` | 63 | 54 | 14% | W X |
| 16 | `coordination.md` | 146 | 132 | 10% | R W (canonical owner) |
| | **cafleet-design-doc-create/** | | | | |
| 17 | `SKILL.md` | 321 | 265 | 17% | D R W X |
| 18 | `roles/director.md` | 86 | 70 | 19% | D R W |
| 19 | `roles/drafter.md` | 85 | 68 | 20% | D W |
| 20 | `roles/reviewer.md` | 72 | 58 | 19% | D W |
| | **cafleet-design-doc-execute/** | | | | |
| 21 | `SKILL.md` | 573 | 370 | 35% | X D R W |
| 22 | `roles/director.md` | 136 | 70 | 49% | D R W |
| 23 | `roles/programmer.md` | 116 | 70 | 40% | D R W |
| 24 | `roles/tester.md` | 87 | 52 | 40% | D W |
| 25 | `roles/verifier.md` | 82 | 50 | 39% | D W |
| | **cafleet-design-doc-interview/** | | | | |
| 26 | `SKILL.md` | 281 | 235 | 16% | D R W (COMMENT block → pointer) |
| 27 | `roles/analyzer.md` | 91 | 73 | 20% | D W (+drop stray base-dir load) |
| | **cafleet-research-report/** | | | | |
| 28 | `SKILL.md` | 410 | 250 | 39% | X D R W |
| 29 | `template.md` | 82 | 74 | 10% | W |
| 30 | `roles/director.md` | 82 | 55 | 33% | D W |
| 31 | `roles/manager.md` | 111 | 72 | 35% | D W |
| 32 | `roles/researcher.md` | 79 | 52 | 34% | D W |
| 33 | `roles/scout.md` | 89 | 60 | 33% | D W |
| | **cafleet-research-presentation/** | | | | |
| 34 | `SKILL.md` | 348 | 245 | 30% | D R W X |
| 35 | `roles/director.md` | 120 | 78 | 35% | D W |
| 36 | `roles/presentation.md` | 114 | 78 | 32% | D W |
| 37 | `roles/transcript.md` | 99 | 70 | 29% | D W |
| 38 | `roles/visual-reviewer.md` | 156 | 120 | 23% | D W |
| | **cafleet-my-slidev/** (prose only) | | | | |
| 39 | `SKILL.md` | 203 | 125 | 38% | X D W (remove Spawnable-Agents) |
| 40 | `techniques/formatting.md` | 123 | 105 | 15% | D W |
| 41 | `techniques/math-formulas.md` | 70 | 50 | 29% | D W X |
| 42 | `techniques/two-column-layouts.md` | 99 | 80 | 19% | X D |
| | **Total** | **5,849** | **4,099** | **30%** | |

**Net-of-relocation accounting.** Only the ~80-line web-researcher block relocates into a *new* in-tree file (`roles/web-researcher.md`); the slide-creator spec is **cut**, not relocated. So the net `skills/` prose tree lands at ~4,180 lines (**≈28% net**, within the ~4,200 ceiling in Success Criteria #1). Content relocated to `docs/spec/` leaves the tree entirely (already reflected in the per-file afters). Per-file content reduction by orchestration-heavy cluster: cafleet core **42%**, design-doc-execute **38%**, research **32%** (report 34% / presentation 29%).

### Notable per-file specifics

- **`cafleet/reference/director.md` (#2)** is dense CLI-internals: exact exit codes, `register_agent`/`member create` rollback mechanics, `agent_card_json.cafleet.kind`, the spawn-argv tables, and `--model` validation errors. Most duplicate `docs/spec/cli-options.md`. Keep a usage-focused command index + the canonical spawn-prompt skeleton; relocate the rest.
- **`design-doc-execute/SKILL.md` (#21)** — the single largest file. Cuts: collapse Step 7's duplicated Copilot branch table (the "7b Per-turn procedure" and the "Per idle-nudge turn checklist" state the same gate table twice) into one; replace the 3 near-identical ~25-line spawn prompts + their 3 "render to `${BASE}/prompts`" paragraphs with the canonical skeleton + a per-role delta table; relocate the PR-loop thresholds/error strings; delete why-no-auto-exit-on-silence and why-not-`reviewDecision` rationale (keep the one-line runtime rules).
- **monitoring ↔ supervision (#9, #10)** — the highest cross-file overlap. Assign single ownership: monitoring owns the heartbeat, the 5-step facilitation loop, stall response, and the Esc-safeguard; recovery owns shutdown ordering. Fold monitoring's Health-Check table into facilitation step 4; reduce supervision's Communication Model / Spawn Protocol / Cleanup to the supervision-specific deltas plus pointers; trim its Quick-Reference "Notes" to terse cross-refs.
- **`cafleet-base-dir/SKILL.md` (#11)** is the **canonical owner** of the no-bypass protocol (20+ files reference it) — its cuts are internal dedup: `<unset>` is contracted 3× and the verbatim string `Error: BASE is <unset>; refusing to fall back to /tmp` appears twice. Merge to one statement, but **keep the contract strings inline** (`Error: BASE is <unset>…` and `audit-disabled no BASE in spawn prompt` are skill-instructed literals the implementer must emit — per the Relocation map split, they stay at the instruction). Relocate only non-contract internals (e.g. the `${BASE}/prompts/<role>-<UTC>.md` path template).
- **`cafleet-my-slidev/SKILL.md` (#39)** carries a 77-line § "Spawnable Agents" — a verbatim `slide-creator` spec (near-duplicating the host skill's own Layouts/Content/Citation rules) plus `cafleet member create` dispatch recipes. A non-orchestration skill should not carry team boilerplate; cut it to a short autonomous-use pointer. The slide-creator spec is removed, not relocated — its Layouts/Content/Citation rules already live in this same SKILL.md.
- **`create-figure/SKILL.md` (#12)** and the Slidev technique files have lower ceilings — they are load-bearing matplotlib/Slidev reference (color hex, chart-selection tables, code templates). Cuts are limited to cross-file dedup (the formula-wrapping rule, stated 4× in math-formulas + 2× in formatting; the semantic-color palette, stated 3×) and non-runtime rationale.

### Resolved decision: interview `COMMENT(claude)` block

- **interview/SKILL.md inline `COMMENT(claude)` block (≈ lines 28–48) → convert to a pointer to `coordination.md`.** The block currently mirrors `coordination.md` inline; the "standalone-plugin packaging" justification does not apply here because the interview skill ships inside this repo alongside `coordination.md`, so the pointer always resolves. This is a firm decision (saves ~15 lines), not a TBD — Step 4 converts it unconditionally. The user may still veto at the approval gate, in which case the block stays inline; the spec carries a definite default either way.

### Behavior-preservation guardrails (apply to every edit)

1. Never alter a CLI command, flag name, required argument, output format, or the order of a workflow step. Only prose, duplication, and rationale change.
2. A fact may leave a skill only if it lands in `docs/spec/` or `docs/concepts/` in the same change. Pointers must name the exact destination section. **Exception:** a skill-instructed contract string (a literal the skill tells the implementer to emit, e.g. `Error: BASE is <unset>…`) stays inline at the instruction — relocating it would force a dereference to learn the literal to write.
3. Keep rationale that changes a runtime decision (e.g. "serialize `member exec`: concurrent dispatch corrupts the pane"; "don't auto-exit on Copilot silence — escalate to the user"). Cut rationale that only explains *why a mechanism is built that way*.
4. Preserve every frontmatter trigger keyword; tighten wording only.
5. Each pointer must resolve — verified by a final cross-reference grep.
6. **A role-file pointer may target only (a) content inline in a skill the member loads at startup, or (b) content whose file is added to that role's Load-at-Startup list — otherwise keep the content inline.** A role file is the authoritative spawn-time definition ("Open `<role>.md` BEFORE any other action"), so a pointer is behavior-safe only when its target is already in the member's context at spawn. Members are told to skip `cafleet/reference/*`, so the member-side "/exit arrives, nothing required of you" note stays inline in each member role — distinct from the Director-side teardown *ordering*, which lives in `recovery.md` and which Directors do read.

### In-scope correctness fixes

Both are genuine role-file defects, promoted to in-scope (consistent with this doc's zero-drift / correctness framing):

- `interview/roles/analyzer.md` loads `cafleet-base-dir` but is forbidden from writing files — drop the unnecessary load (Step 4).
- `execute/roles/verifier.md` lists `curl`/`wget` fallback rows that conflict with the project's global Bash ban — reconcile to WebFetch / delegate (Step 5). A verifier following the current rows would attempt a banned command.

### Risks

| Risk | Mitigation |
|---|---|
| A relocated pointer dangles or points to a missing anchor | Step 9 grep verifies every cross-reference resolves to a real file + heading |
| Information lost when internals leave a skill | Step 1 stages all destination content *before* any deletion; Step 9 cross-checks |
| A member loses the literal-id / audit-file / shutdown contract at spawn time because a role file now only points to it | Guardrail 6: a role-file pointer may target only content the member loads at startup (or a file added to its Load-at-Startup list); otherwise the content stays inline. The member-side `/exit` note stays inline; spawn prompts still load the `cafleet` skill, which carries the universal contract |
| Increased inter-file coupling | Accepted by the user; offset by single-source-of-truth correctness |
| Skill auto-routing degrades | Frontmatter trigger keywords are preserved verbatim |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> Documentation-first ordering (project rule): relocation destinations and canonical homes are established before any skill content is deleted.

### Step 1: Stage relocation destinations (no skill deletions yet)

- [x] Add the relocated CLI-internals (exit codes, verbatim error strings, rollback/timeout mechanics) to `docs/spec/cli-options.md` — covering fleet create/delete, agent deregister, member create/delete/exec/ping/send-input/nudge, message poll truncation, and `--full` semantics. <!-- completed: 2026-06-16T18:53 -->
- [x] Add internal DB fields / state-machine details (`agent_card_json.cafleet.kind`, `monitor_config`, `monitor_runtime`, `origin_task_id` threading, `input_required`/`completed`) to `docs/spec/data-model.md` and `docs/spec/message-envelope.md`. <!-- completed: 2026-06-16T18:53 -->
- [x] Add helper/keystroke/lifecycle mechanics (`tmux.send_inline_preview`, `send_poll_trigger`, Esc-settle timing, SIGTERM/`finally` runtime-row clearing, 15 s `/exit` wait, 60 s default interval) to `docs/concepts/{monitoring.md, member-lifecycle.md, tmux-push.md}`. <!-- completed: 2026-06-16T18:53 -->
- [x] Verify every fact later removed from a skill has a home in `docs/spec/` or `docs/concepts/` (no information loss). <!-- completed: 2026-06-16T18:53 -->
- [x] Confirm up front whether `README.md` references any relocated content or the skills-tree shape (the new `roles/web-researcher.md` file); note required README edits now rather than discovering drift at Step 9 (project rule: README is a first-class same-cycle target). <!-- completed: 2026-06-16T18:53 -->

### Step 2: Establish canonical homes for shared conventions

- [x] Confirm/clean the single canonical statement of each convention per the canonical-home map (placeholder rule, verb+pointer protocol, BASE/`<unset>`, monitor heartbeat + facilitation + stall + spawn-gate, shutdown ordering, Esc-safeguarded inline preview, member-side universal boilerplate). <!-- completed: 2026-06-16T19:24 -->
- [x] Add the canonical spawn-prompt skeleton (identity block + COMMUNICATION PROTOCOL + skill-load list) to `cafleet/reference/director.md` § Member Create. The per-role **delta table MUST capture every IMPORTANT / role-constraint line** from each current spawn prompt (e.g. Tester "Do NOT write implementation code", Programmer "Do NOT commit code yourself", "Read `.claude/rules/bash-tool.md`") so no behavioral line is lost in the collapse. <!-- completed: 2026-06-16T19:24 -->
- [x] Extract the web-researcher agent spec from `research-report/SKILL.md` into `cafleet-research-report/roles/web-researcher.md`; leave the two dispatch recipes as pointers. <!-- completed: 2026-06-16T19:24 -->

### Step 3: Simplify the cafleet core cluster (851 → ~496)

- [x] Simplify `cafleet/SKILL.md` + the 5 reference files (`director`, `broadcast`, `exec-routing`, `output-flags`, `recovery`): relocate CLI-internals; dedup the placeholder / inline-preview / forbidden-behaviors blocks; cut rejected-alternative and why-literal-flags rationale. <!-- completed: 2026-06-16T19:48 -->
- [x] Simplify `cafleet/roles/{director,member}.md`: point to canonical placeholder / comms / forbidden-behaviors / shutdown homes; keep only role-unique content. <!-- completed: 2026-06-16T19:48 -->


### Step 4: Simplify the design-doc family (create/interview/doc + roles)

- [x] Simplify `cafleet-design-doc/{SKILL,guidelines,coordination}.md` (leave `template.md` untouched); relocate coordination.md's per-file git-recovery commands to `docs/spec/`. <!-- completed: 2026-06-16T20:05 -->
- [x] Simplify `create/SKILL.md` + `roles/{director,drafter,reviewer}.md`: dedup the 3 spawn prompts to the canonical skeleton; collapse the clarification-exemption (3×) and COMMENT-workflow restatements; point role boilerplate to canonical homes. <!-- completed: 2026-06-16T20:05 -->
- [x] Simplify `interview/SKILL.md` + `roles/analyzer.md`: convert the inline `COMMENT(claude)` block to a `coordination.md` pointer (firm decision — see Resolved decision above); drop the analyzer's unnecessary `cafleet-base-dir` load. <!-- completed: 2026-06-16T20:05 -->


### Step 5: Simplify design-doc-execute (994 → ~612)

- [x] Simplify `execute/SKILL.md` (573 → ~370): collapse Step 7's duplicate Copilot branch table to one; dedup the 3 spawn prompts + audit-file paragraphs to the skeleton; relocate PR-loop thresholds/error strings; cut why-no-auto-exit / why-not-`reviewDecision` rationale. <!-- completed: 2026-06-16T20:21 -->
- [x] Simplify `execute/roles/{director,programmer,tester,verifier}.md`: canonical-home pointers + per-role deltas only; reconcile verifier `curl`/`wget` rows with the Bash ban (WebFetch / delegate). <!-- completed: 2026-06-16T20:21 -->


### Step 6: Simplify the team skills (monitoring/supervision)

- [x] Simplify `monitoring/SKILL.md`: fold Health-Check into facilitation step 4; one Esc-preview statement; relocate the 60 s / DB-field internals; trim spawn-prompt rationale comments. <!-- completed: 2026-06-16T20:30 -->
- [x] Simplify `supervision/SKILL.md`: canonical-home pointers for comms / spawn-gate / teardown; collapse the passive-hold restatements (~4×); trim Quick-Reference notes; relocate env-check error strings. <!-- completed: 2026-06-16T20:30 -->


### Step 7: Simplify the research skills (1,690 → ~1,154)

- [x] Simplify `research-report` (`SKILL.md` + `template.md` + 4 roles): land the web-researcher extraction pointers; dedup tag/verification tables to one producer-owned copy; point role boilerplate to canonical homes; collapse in-file shutdown double-statements. <!-- completed: 2026-06-16T20:51 -->
- [x] Simplify `research-presentation` (`SKILL.md` + 4 roles): dedup the spawn prompts; keep each tag table in one producer role; point boilerplate to canonical homes; relocate ack/state strings. <!-- completed: 2026-06-16T20:51 -->


### Step 8: Simplify base-dir / figure / slidev prose

- [x] Simplify `cafleet-base-dir/SKILL.md` (merge the 3× `<unset>` restatement, relocate the two verbatim strings) and `cafleet-create-figure/SKILL.md` (cut non-runtime rationale; dedup BASE rules to a pointer). <!-- completed: 2026-06-16T20:59 -->
- [x] Simplify `cafleet-my-slidev/SKILL.md` (remove the Spawnable-Agents block — cut the slide-creator spec, do not relocate) + the 3 technique files (canonical formula-wrapping rule in `math-formulas.md`; canonical semantic-color palette in `formatting.md`). <!-- completed: 2026-06-16T20:59 -->


### Step 9: Verify (targets met, no behavior change)

- [x] Re-measured every in-scope file via `wc -l`. Aggregate **~15% per-file (5,849→4,967) / ~14% net (→5,048)** — below the 28–30% target (clusters cafleet 27% / execute 10% / research+slidev 13–20%); before/after recorded in the Changelog. Targets not reached without violating zero-drift — see SC#1 note. <!-- completed: 2026-06-16T21:10 -->
- [x] Cross-references audited independently across all of `skills/` — every pointer resolves to a real file + heading anchor (0 dangling / 0 suspect, ~200+ links, incl. the new `docs/spec` relocation anchors, the canonical-skeleton anchor, `web-researcher.md`, and no surviving `slide-creator` pointer). No relocated fact dropped vs the Step-1 destinations. <!-- completed: 2026-06-16T21:10 -->
- [x] Diff reviewed per step + holistically (`git diff --stat main...HEAD`): only `skills/`, `docs/concepts/`, and this design doc changed — no source/config/test files; only prose/dedup/relocation. No repo skill-lint / markdown-lint task exists (`.claude/rules/commands.md` lists only Python lint/format/typecheck). <!-- completed: 2026-06-16T21:10 -->
- [x] **Lossless spawn-prompt reconstruction:** verified per role during each step's review — every IMPORTANT / role-constraint line and start cue maps verbatim to a skeleton + per-role delta row (the lossless rule itself lives in `cafleet/reference/director.md`). No IMPORTANT line dropped. <!-- completed: 2026-06-16T21:10 -->
- [x] No `README.md` edits required (confirmed up-front at Step 1: README references skills only at install/architecture level, not the tree shape, and cites no relocated internal). `docs/concepts/` updated as relocation destinations; no README drift. <!-- completed: 2026-06-16T21:10 -->

---

## Changelog (spec revisions only)

| Date | Changes |
|------|---------|
| 2026-06-16 | Initial draft |
| 2026-06-16 | Addressed reviewer findings: reconciled the reduction bands/ceiling across Overview / SC#1 / accounting; split CLI-emitted vs skill-instructed contract strings (latter stay inline); cut (not relocate) slide-creator; resolved the interview `COMMENT(claude)` decision; added pointer-dereferencing guardrail 6 + member-side-inline split; promoted the verifier `curl`/`wget` fix to in-scope; added the lossless spawn-prompt reconstruction check and up-front README confirmation |
| 2026-06-16 | Reviewer + user approved; Status → Approved |
| 2026-06-16 | **Execution complete (26/26 tasks), zero behavioral drift.** All 8 implementation steps applied every safe dedup/relocation/cut the design named, verified per step + holistically at Step 9. **SC #2–6 met and verified** (single canonical home; no info loss; zero drift — only `skills/` + `docs/concepts/` + this doc changed, no source/config/test; 0 dangling cross-references across ~200+ links; frontmatter `description:` byte-identical). **SC #1 (line numbers) NOT met:** in-scope per-file **5,849 → 4,967 (~15%)** vs 30% target; **net tree → 5,048 (~14%)** vs ~28% / ~4,200 ceiling. Per cluster (before→after, target): cafleet core **851→617 (27%, t42%)**; design-doc-execute **994→891 (10%, t38%)**; research **1690→1413 (16%, t32%)**; design-doc family **1161→1027 (12%, t16%)**; monitoring+supervision **291→272 (6.5%, t32%)**; base-dir/figure/slidev **813→704 (13%, t24%)**. Root cause: the estimates assumed more redundancy than exists under the zero-drift contract — director.md absorbed the new ~51-line canonical spawn-prompt skeleton + lossless rule the 150 estimate never counted; the rest is canonical, cross-referenced content (command indexes, workflow steps, contract strings kept inline per guardrail 2, load-bearing matplotlib/Slidev reference) that guardrails 1/2/5 forbid cutting. Two spec deviations, both correctness-preserving: coordination.md git-recovery condensed in place (no `docs/spec` home → would dangle); base-dir `<unset>` contract strings kept inline (skill-instructed literals, guardrail-2 exception — overrides the Step-8 "relocate the two verbatim strings" wording). New file: `cafleet-research-report/roles/web-researcher.md` (98-line spec extracted verbatim). Cut (not relocated): the my-slidev slide-creator spec. |
| 2026-06-16 | **Status → Complete.** PR #124 opened with `@copilot` review. Copilot's single inline finding was a genuine zero-drift regression the refactor had introduced — `member.md` rewrote `cafleet message poll/send/ack` into the invalid flat forms `cafleet poll/send/ack` (verified absent from `main`); fixed in commit `4d9d795`. Re-review returned "reviewed 46/46 files, generated no new comments" (Copilot's clean-pass signal). The transient `COMMENT(director)`/`COMMENT(programmer)` coordination markers from Steps 3–8 were swept at finalize (their substance is consolidated in the Step-9 entry above); the historical record lives in git + this Changelog. |
