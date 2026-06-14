# Slim Down the CAFleet Skill `.md` Files (Behavior-Preserving)

**Status**: Approved
**Progress**: 6/10 tasks complete
**Last Updated**: 2026-06-14

## Overview

Several CAFleet operational skill `.md` files are bloated with restated rationale, cross-file duplication, non-enforced catalogs, and over-explanation. This document plans a **behavior-preserving** slimming of those files: every CLI command, flag, error-message string, gate, decision tree, protocol step, spawn-prompt template, and skill-loading trigger (the YAML `description` fields) is **frozen and preserved exactly**; only redundancy and over-explanation are removed.

## Success Criteria

- [ ] Every in-scope file is reviewed against the principles below and slimmed where redundancy exists, with no change to any frozen token (commands, flags, error strings, gates, frontmatter `description` fields, verb/pointer tokens, spawn-prompt template bodies).
- [ ] Each operational fact has exactly one canonical home; every other mention is a one-line cross-reference (extending the existing "canonical there; do not duplicate" pattern already used in the skills).
- [ ] A before/after frozen-token inventory proves the set of CLI commands, flags, error strings, gates, and frontmatter descriptions is identical pre- and post-edit.
- [ ] Cross-reference integrity holds: every "see X § Y" pointer resolves to an existing heading, and every role-file cross-reference points only to a skill the spawned member loads at startup.
- [ ] No "for history" deprecation notes remain (per `.claude/rules/removal.md`): each slimmed file reads as if the removed redundancy never existed.
- [ ] No behavior-describing documentation (`README.md`, `docs/concepts/`, `docs/`) requires any change, because the CLI / API / config surface is unchanged — verified, not assumed.

---

## Background

### Scope (locked)

In scope: the **cafleet operational family only** — `cafleet`, `cafleet-design-doc*` (`-create` / `-execute` / `-interview` and the shared `cafleet-design-doc`), `cafleet-agent-team-*` (`-monitoring` / `-supervision`), and `cafleet-base-dir`. Out of scope: `cafleet-create-figure`, `cafleet-my-slidev`, `cafleet-research-report`, `cafleet-research-presentation`, `skill-author`.

Edit target: the **repo-committed** files under `skills/` (git-tracked source of truth). The installed copy at `~/.claude/skills/` is a separate physical directory (verified: a regular directory, not a symlink) and is **downstream / out of scope** — see *Global Copy Propagation* below.

### The problem

The fattest files carry redundant prose, restated design rationale that belongs in a design doc (not a skill — `.claude/rules/code-quality.md` bans "multi-paragraph explanations of design decisions" in source, and `cafleet-design-doc/guidelines.md` caps decision rationale at "2-3 sentences max"), cross-file duplication of facts that already have a canonical home, and example catalogs the CLI explicitly does not enforce.

### Current inventory (26 files, 4108 lines)

| # | File | Lines |
|--:|:--|--:|
| 1 | `cafleet/SKILL.md` | 304 |
| 2 | `cafleet/reference/director.md` | 249 |
| 3 | `cafleet/reference/exec-routing.md` | 87 |
| 4 | `cafleet/reference/recovery.md` | 76 |
| 5 | `cafleet/reference/broadcast.md` | 57 |
| 6 | `cafleet/reference/output-flags.md` | 38 |
| 7 | `cafleet/roles/director.md` | 52 |
| 8 | `cafleet/roles/member.md` | 108 |
| 9 | `cafleet-design-doc/SKILL.md` | 16 |
| 10 | `cafleet-design-doc/coordination.md` | 153 |
| 11 | `cafleet-design-doc/guidelines.md` | 63 |
| 12 | `cafleet-design-doc/template.md` | 49 |
| 13 | `cafleet-design-doc-create/SKILL.md` | 488 |
| 14 | `cafleet-design-doc-create/roles/director.md` | 116 |
| 15 | `cafleet-design-doc-create/roles/drafter.md` | 103 |
| 16 | `cafleet-design-doc-create/roles/reviewer.md` | 88 |
| 17 | `cafleet-design-doc-execute/SKILL.md` | 818 |
| 18 | `cafleet-design-doc-execute/roles/director.md` | 166 |
| 19 | `cafleet-design-doc-execute/roles/programmer.md` | 128 |
| 20 | `cafleet-design-doc-execute/roles/tester.md` | 99 |
| 21 | `cafleet-design-doc-execute/roles/verifier.md` | 94 |
| 22 | `cafleet-design-doc-interview/SKILL.md` | 329 |
| 23 | `cafleet-design-doc-interview/roles/analyzer.md` | 101 |
| 24 | `cafleet-agent-team-monitoring/SKILL.md` | 110 |
| 25 | `cafleet-agent-team-supervision/SKILL.md` | 132 |
| 26 | `cafleet-base-dir/SKILL.md` | 84 |

Per Q4, there is **no hard numeric line-reduction target** — the principles drive the outcome. The per-cluster estimates below are guidance only.

---

## Specification

### Slimming principles (reusable, mechanical)

These make execution deterministic and verifiably behavior-preserving. Apply each cut only when it removes redundancy, never an operational instruction.

| ID | Principle | What it cuts | What it keeps |
|:--|:--|:--|:--|
| **P1** | **One canonical home per fact.** | A command/flag/gate/protocol/convention stated in file B that already lives in file A. | The canonical statement in A, plus a one-line `see A § Y` cross-reference in B (extends the skills' existing "canonical there; do not duplicate" pattern). |
| **P2** | **Cut restated design rationale.** | Multi-paragraph "Why …" blocks that justify a past design decision. | The one-line operational rule. **Exception:** rationale that is *decision-critical for correctness* (stops a reader from doing the wrong thing) stays, tightened. |
| **P3** | **Cut non-enforced catalogs.** | Long example lists the CLI does not validate against (e.g. the `--model` model-name catalog). | The rule (format + pass-through) plus at most one example per backend. |
| **P4** | **De-duplicate within a file.** | The same fact stated multiple times in one file. | One statement. |
| **P5** | **Cut mechanics obvious from the command surface.** | Prose re-explaining what a flag plainly does. | Non-obvious behavior only. |
| **P6** | **FROZEN set — never edited.** | (nothing) | Every frontmatter `description:` field; every literal CLI command / subcommand; every flag name; every error-message string; every gate / decision condition; every verb + pointer token; every spawn-prompt template body; every skill-load trigger. |
| **P7** | **Role-file standalone constraint.** | (limits P1 for role files) | A role file is read in isolation by its spawned member, so a cross-reference may point **only** to a skill the member is told to load at startup (the `cafleet` skill, the `cafleet-design-doc` skill). Never cross-reference a file the member will not load. |
| **P8** | **Removal totality.** | "For history" notes, "X was here" pointers, deprecation breadcrumbs (per `.claude/rules/removal.md`). | A file that reads as if the redundancy never existed. The historical record lives in git and in this design doc. |

### The frozen set in detail (P6)

The implementation MUST preserve, byte-for-byte where it is a literal:

1. **Frontmatter `description:` fields** — these are the skill-loading triggers; any edit risks a skill loading at the wrong moment. Zero changes.
2. **CLI commands & subcommands** — `cafleet fleet create`, `member create/delete/list/capture/send-input/exec/ping`, `message send/poll/ack/cancel/show/broadcast`, `agent register/list/show/deregister`, `monitor start/status`, `doctor`, `db init`, `server`. (This list is illustrative; the **extraction rule is mechanical** — every span beginning `cafleet `, per V1.)
3. **Flags & options** — `--fleet-id`, `--agent-id`, `--to`, `--text`, `--full`, `--quiet`, `--json`, `--coding-agent`, `--model`, `--prompt-file`, `--member-id`, `--choice`, `--freetext`, `--lines`/`--tail`, `--ansi`/`--no-ansi`, `--force`/`-f`, `--activity`, `--label`, env vars (`CAFLEET_*`).
4. **Error-message strings** — e.g. `Error: --fleet-id <int> is required for this subcommand…`, `Error: Agent <member-id> not found`, `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead.`, `Error: --model for the opencode backend must be '<provider-id>/<model-id>'…`, `tmux command failed: command too long`, `Error: BASE is <unset>; refusing to fall back to /tmp`. (These are illustrative; the **extraction rule is mechanical** — every line matching `Error:` / `failed:` / ` must be ` / ` is required`, per V1.)
5. **Gates & decision conditions** — clarification gate, `SKIP_CLARIFICATION` / `QUALITY_REVIEW_ONLY` / `SKIP_ANALYZER` flags, the three-tier path detection, the Copilot loop branch table (`state == "APPROVED"`, `submittedAt > last_push_ts`, `silence_ticks >= 30`, round limits), the `<unset>` sentinel branch.
6. **Verb + pointer vocabulary** — the 6 verbs (`ready`/`complete`/`addressed`/`blocked`/`escalating`/`approved`), the 3 pointer forms, the ` > ` separator, the `COMMENT(role)` marker grammar and role taxonomy.
7. **Spawn-prompt template bodies** — the literal Drafter/Reviewer/Director/Programmer/Tester/Verifier/Analyzer spawn prompts (including the `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders and `[INSERT …]` markers).

### Cross-cutting duplication map

The findings, ranked by redundancy removed. Each maps to a per-cluster plan below.

> **Locators are pre-edit coordinates.** Every line number in this map and the per-cluster plan is a **pre-edit coordinate** that shifts as earlier lines are cut; each is paired with a durable anchor (section heading) so the execute-time implementer locates the target by heading, not by a drifted line number.

| ID | Duplication | Canonical home | Redundant copies | Est. lines removed |
|:--|:--|:--|:--|--:|
| **D1** | **Coordination Protocol** (verb vocab, pointer forms, message format, COMMENT marker, issue/status split, Copilot routing, anchorless status, finalize cleanup, per-file recovery) inlined near-verbatim. | `cafleet-design-doc/coordination.md` | `create/SKILL.md` §Coordination Protocol (~146 ln); `execute/SKILL.md` §Coordination Protocol (~151 ln). | ~270 |
| **D2** | **Shutdown Protocol / "stop the monitor first; no `monitor stop` command; belt-and-suspenders"** rationale re-spelled. | `cafleet/reference/recovery.md` §Shutdown Protocol | `monitoring/SKILL.md`, `supervision/SKILL.md` §Cleanup, `create/roles/director.md`, `execute/roles/director.md`, `create/SKILL.md` Step 6, `execute/SKILL.md` Step 8. (Most already cross-ref recovery.md — trim the re-spelled rationale, keep the procedure steps.) | ~30 |
| **D3** | **`member list --activity` example table** duplicated. | `cafleet/reference/director.md` §Member List | `cafleet/reference/recovery.md` (already cross-refs it — pre-edit line 20, § *Routine monitoring via `member list --activity`* — drop the duplicated example block). | ~8 |
| **D4** | **Bash-via-Director forbidden-behaviors / reconsider-before-routing** content triplicated. | `cafleet/reference/exec-routing.md` | `cafleet/roles/member.md` (MUST NEVER + WHEN DENIES + WHY THIS WORKS); plus `exec-routing.md`'s own "Why no operator-prompts-for-routing" §, which verbatim repeats its earlier MUST-NOT bullet. (`.claude/rules/bash-tool.md` is not a skill file → out of scope.) | ~35 |
| **D5** | **Restated "Why" rationale blocks.** | (inline rule) | `cafleet/SKILL.md` "Why literal flags, not env vars?"; `director.md` two-paragraph spawn-prompt-size ARG_MAX derivation; `create/SKILL.md` "Why path-by-reference". **Keep (tighten):** `execute/SKILL.md` "Why no auto-exit on silence" / "Why not `reviewDecision`" — decision-critical (P2 exception). | ~25 |
| **D6** | **`--model` non-enforced model catalog** (fable/opus/sonnet, 4× gpt, 7× opencode examples) in the flag Notes cell. | `cafleet/reference/director.md` §Member Create (`--model` row) | (same cell) — trim to the rule + ≤1 example per backend; keep the inference table with one example per row. | ~6 |
| **D7** | **`--full` no-op on `broadcast`** restated 4× within one file. | `cafleet/reference/output-flags.md` (cross-subcommand `--full` table) | `cafleet/reference/broadcast.md` (pre-edit lines 13/17/29/55-57 — the flag table, § *What the broker does*, § *Default echo*, § *Flag-surface consistency*) — state once, cross-ref output-flags.md. | ~12 |
| **D8** | **SKILL boilerplate** (Architecture, Prerequisites `cafleet doctor`, Primitive Mapping, the "Members receive messages via push notification … inline preview" paragraph) near-identical across the 3 design-doc SKILLs. | (per-skill local) | Light trim: collapse the repeated inline-preview paragraph to a cross-reference to the `cafleet` skill; keep the per-skill Primitive Mapping + Architecture. | ~20 |
| **D9** | **Role-file boilerplate** ("Load at Startup", "Placeholder convention", the broker-keystroke "Receiving tasks" paragraph, the 7-line "Shutdown" block) repeated verbatim across 7 member role files; plus the two design-doc Director roles share verbatim **Idle Semantics + Stall Response Ladder** blocks (also in `supervision`/`monitoring`). | `cafleet` skill (member loads it) for the generic blocks; `cafleet-agent-team-supervision`/`-monitoring` (Director loads them) for Idle/Stall. | Trim Shutdown to ~2 lines (drop the Agent-Teams-primitive history); trim Placeholder to ~1 line; cross-ref the broker-keystroke paragraph; collapse the Director roles' Idle/Stall blocks to a cross-reference. Honor P7. | ~110 |
| **D10** | **Spawn audit-file convention** (`${BASE}/prompts/<role>-<UTC>.md`, `mkdir`, `strftime`, same-second `_2/_3` collision, `<unset>` guarded-skip) re-spelled in full at each spawn site; in `execute/SKILL.md` the identical "two-step render → write → `--prompt-file`" procedure is repeated 3× (Programmer/Tester/Verifier). | `cafleet/reference/director.md` §*Member Create — Scratch and audit files* | `create/SKILL.md` 1d/1e; `execute/SKILL.md` 3e (×3); `interview/SKILL.md` 2d; `supervision/SKILL.md` Spawn Protocol. State the two-step pattern once per skill; keep every spawn-prompt template + `member create` command verbatim. | ~45 |
| **D11** | **interview internal repetition** — the `COMMENT(claude)` spec stated 3× (§COMMENT(claude) Marker, Step 4, §COMMENT Annotation Format) and `question.md` format stated 2×. | one §COMMENT(claude) + one §`question.md` Format | the duplicate statements — Step 4 references them. Keep the deliberate single plugin-independent inline copy (pre-edit line 26, the § *Coordination Protocol* **Maintainer source-of-truth** note documents this choice). | ~25 |

Estimated total removable: ~590 lines (~14% of 4108), concentrated in the three fat SKILLs and the role files. This is an estimate, not a target.

### Per-cluster plan

**Cluster A — `cafleet/` core + reference + roles (8 files, 971 ln).** The user flagged `reference/*.md` as "really fat and bloated."
- `reference/exec-routing.md`: delete the "Why no operator-prompts-for-routing" § (D4 — verbatim repeat of the member-side MUST-NOT bullet); keep the two-primitive table, reconsider ladder, both fallback recipes, serialization, cross-fleet boundary.
- `reference/broadcast.md`: collapse the 4× `--full` no-op restatement to one statement + cross-ref `output-flags.md` (D7). The retained single statement MUST be the **operationally-complete** one — `--full` renders the single `broadcast_summary` task in full but never adds per-recipient delivery rows or a `recipient_ids` list, because the broker only ever returns that one summary task — not the bare "no-op" label. (output-flags.md carries only the generic cross-subcommand `--full` semantics; this per-recipient / `recipient_ids` fact is broadcast-only and lives nowhere else, so it must survive in broadcast.md's retained statement.) Keep the broker behavior, threading, ack, summary semantics.
- `reference/recovery.md`: drop the duplicated `member list --activity` example block (D3 — already cross-references director.md); keep the canonical Shutdown Protocol here (D2 canonical home), the 2-stage health check, stalled-member shapes, disconnect/wedged-`/exit` trees.
- `reference/director.md`: trim the `--model` catalog to rule + ≤1 example/backend (D6); trim the two-paragraph ARG_MAX spawn-size derivation to the rule (D5); keep the `--activity` table here as canonical (D3); keep every `member *` command, flag table, error string, send-input pane-shapes table.
- `reference/output-flags.md` (38 ln, already lean): light P5 trim only.
- `SKILL.md`: trim the "Why literal flags, not env vars?" multi-paragraph block to the one-line rule (D5); keep the required-flags table, every command section, every error string, the reserved-name/root-Director guards.
- `roles/member.md`: collapse the triple forbidden-behaviors statement (MUST NEVER / WHEN DENIES / WHY THIS WORKS → one tight forbidden-behaviors block + one denial-routing pointer to `exec-routing.md`) (D4); keep the ready-signal first-action, the default rule, the id-source note.
- `roles/director.md` (52 ln, mostly a reading-order index already): light trim.

**Cluster B — `cafleet-design-doc/` shared (4 files, 281 ln).**
- `coordination.md`: confirm as the **canonical** Coordination Protocol home (D1); minor self-trim only. This file *grows in authority* — the create/execute inlined copies collapse into references to it.
- `guidelines.md`, `template.md`, `SKILL.md` (16 ln): already lean — leave essentially as-is.

**Cluster C — `cafleet-design-doc-create/` (4 files, 795 ln).**
- `SKILL.md`: replace the inlined `## Coordination Protocol` (~146 ln) with a short cross-reference to `../cafleet-design-doc/coordination.md` (add it to "Additional resources") **plus** the skill-specific bits that are not in coordination.md: the role-subset note (this skill uses `director`/`drafter`/`reviewer`/`claude`/`copilot`) and the existing Clarification Exemption (D1). Collapse the repeated spawn audit-file procedure to the two-step pattern stated once (D10). Keep every Step 0–6 command, gate, AskUserQuestion table, and both Drafter spawn-prompt templates (normal + resume) verbatim.
- `roles/director.md`: collapse Idle Semantics + Stall Response Ladder to a cross-reference to the `cafleet-agent-team-supervision`/`-monitoring` skills the Director loads (D9, honoring P7 — the Director does load them); trim Shutdown Protocol to the recovery.md cross-reference (D2); keep accountability bullets, skill-specific milestones table, COMMENT-handling workflow.
- `roles/drafter.md`, `roles/reviewer.md`: trim Placeholder convention + Shutdown block + broker-keystroke paragraph (D9); keep the structured-question framework, accountability, review tag taxonomy, approval signal.

**Cluster D — `cafleet-design-doc-execute/` (5 files, 1305 ln — the fattest).**
- `SKILL.md`: same Coordination Protocol replacement as create (~151 ln → cross-reference + Verifier Phase-1 exemption + role note) (D1); de-triplicate the spawn two-step procedure (state once before the three role templates) (D10); **keep and tighten** the "Why no auto-exit on silence" / "Why not `reviewDecision`" rationale (P2 exception — decision-critical for the Copilot gate). **Guardrail:** only the surrounding justification prose may be tightened — the operational conclusion embedded in those blocks (the gate condition: exit only on a post-push `state == "APPROVED"` with `submittedAt > last_push_ts`; never auto-exit on silence; the `silence_ticks >= 30` / `round >= 5` thresholds) is FROZEN under P6 #5 and MUST NOT be touched. Keep all of Steps 1–8 — the three-tier path detection, discovery/pagination/error blocks, the full Copilot loop (6a/6b/7a–7e), the per-wake checklist, error-handling tables, every `gh`/`git` command and template.
- `roles/director.md`: collapse the verbatim Idle/Stall blocks and Shutdown to cross-references (D9, D2); keep the accountability bullets, Escalation Protocol, Commit Protocol Summary table, Copilot/PR milestones — these are skill-specific and load-bearing.
- `roles/programmer.md` / `tester.md` / `verifier.md`: trim the shared boilerplate (D9); keep each role's Workflow phases, escalation/defect/graceful-degradation logic, and every verb/pointer/marker instruction.

**Cluster E — `cafleet-design-doc-interview/` (2 files, 430 ln).**
- `SKILL.md`: consolidate the 3× `COMMENT(claude)` spec to one and the 2× `question.md` format to one (D11); **keep** the single deliberate plugin-independent inline of the marker convention (the file documents this choice at pre-edit line 26, the § *Coordination Protocol* **Maintainer source-of-truth** note — do not convert it to a cross-link); keep Steps 0–5, the progress-tracking marker, the multi-session resume tables, the mandatory-completion rule, `$ARGUMENTS`.
- `roles/analyzer.md`: trim shared boilerplate (D9); keep question categories, priority order, output format, workflow.

**Cluster F — agent-team skills (2 files, 242 ln).**
- `monitoring/SKILL.md` / `supervision/SKILL.md`: collapse the Shutdown/monitor-stop restatement to the recovery.md cross-reference (D2); the three-beat AskUserQuestion workflow is already canonical in `reference/director.md` and cross-referenced — remove any remaining re-spelling; keep the heartbeat mechanism, the 5-step facilitation loop, the health-check sequence, the Authorization-Scope Guard, the Spawn Protocol, the Quick Reference table.

**Cluster G — `cafleet-base-dir/SKILL.md` (84 ln).** Already dense and load-bearing (Step 0/1/2 resolution, `<unset>` sentinel, no-bypass protocol). Light P5 trim only; preserve every resolution branch and the `<unset>` contract.

### Verification strategy (Q4 — rigorous, proves no behavior change)

The proof obligation is: **the set of operational tokens is identical before and after.** A relocation (fact moves from an inlined copy to its canonical home) is allowed iff the canonical occurrence still exists and is reachable; an outright *disappearance* of any frozen token is a defect.

| ID | Gate | Method |
|:--|:--|:--|
| **V1** | **Frozen-token baseline (pre-edit).** | Before any edit, extract from all 26 files into a baseline artifact (`${BASE}/verification/frozen-inventory-before.md`) using these **mechanical extractors**: (a) **commands** = every span beginning `cafleet ` (continuation lines joined on a trailing `\`); (b) **flags** = every `--[a-z][a-z-]*` token; (c) **error strings** = every line matching `Error:` / `failed:` / ` must be ` / ` is required`; (d) **frontmatter `description:`** = the full `---`-delimited block of each SKILL.md; (e) **verb + pointer tokens** = the six verbs and three pointer forms; (f) the byte content (hash) of every spawn-prompt template. Categories (a)–(f) are mechanically extractable and feed V2's multiset diff. **Gates / decision conditions have no consistent syntactic delimiter and are NOT mechanically extractable** — they are explicitly DOWNGRADED to V4 manual review and are not claimed under V2's automatic diff. |
| **V2** | **Frozen-token diff (post-edit).** | Re-extract the mechanical sets (V1 a–f) after editing into `…-after.md`. The multisets of commands / flags / error strings / frontmatter descriptions MUST be identical. Verb/pointer vocabulary unchanged. Spawn-prompt templates byte-identical. Gates / decision conditions are excluded here — covered by V4. Any delta is investigated and either reverted or justified as a pure relocation whose canonical occurrence is confirmed present. |
| **V3** | **Frontmatter byte-identical check.** | `git diff` on each file shows zero changes inside any `---` frontmatter block. |
| **V4** | **Per-file diff review.** | Every file's `git diff` is read to confirm each removed line is redundant prose / duplication / non-enforced catalog — never an operational instruction. This is the human/Reviewer gate. |
| **V5** | **Cross-reference integrity.** | Every new "see X § Y" pointer resolves to an existing heading in an existing file. For role files (P7), the target must be a skill the spawned member loads at startup (the `cafleet` skill or the `cafleet-design-doc` skill) — never a file the member will not load. |
| **V6** | **Behavior-doc drift check.** | Confirm no `README.md` / `docs/concepts/` / `docs/` change is needed (the CLI/API/config surface is unchanged). If any such doc *would* need a change, that signals an accidental behavior change — stop and revert. |
| **V7** | **Removal-totality scan (P8).** | Confirm no "for history" / "X was deprecated" / restoration-pointer breadcrumbs were introduced. |
| **V8** | **Content-superset for collapsed prose (P1 / D1 / D2 / D9).** | For every prose block collapsed into a cross-reference (the Coordination Protocol body, the Idle Semantics / Stall Response Ladder rungs, the Shutdown ordering), confirm the cross-reference target already states **every** operational step being removed — the target's content must be a SUPERSET of the removed block. A dropped prose rung (e.g. "do NOT skip rungs," or the create Director's re-spawn / redistribute / drop-scope escalation options) is a defect even though V2 (which diffs only commands / flags / error strings / descriptions) cannot detect it. V5 proving the target heading merely *exists* is not sufficient. This gate covers the plan's single biggest behavior-preservation risk. |

All V-gates (V2–V8) operate at **execute time** over the slimmed skill-file diffs: the execute run's own Verifier (Phase D) runs V2/V3/V4/V5/V6/V7/V8 as its E2E pass. The create run's Reviewer reviews **this design doc** (the present pass) for internal consistency and completeness — that review is not itself a V-gate, because the skill-file diffs the V-gates inspect do not exist yet at design-creation time.

### Global copy propagation (Q3 — documented, out of scope)

- **Source of truth:** the repo `skills/` tree (git-tracked). This is the only edit target.
- **Downstream copy:** `~/.claude/skills/` is a **separate physical directory** (verified: a regular directory, not a symlink). Edits to the repo `skills/` do **not** automatically appear there.
- **Propagation:** the global copy is refreshed by the operator's skill-install/sync step (a copy or re-link performed outside the skill files themselves; the repo-config probe for an automated task is harness-restricted in this environment, so the exact command is the operator's environment detail, not part of this change).
- **Scope decision:** updating `~/.claude/skills/` is **out of scope** for this change. After the repo change merges, re-syncing the global copy is an **operator follow-up**, noted here so it is not forgotten. No `.md` edits are planned against the global copy.

### Meta-safety note (editing the orchestrating skills)

This change edits the very skills (`cafleet-design-doc-create` / `-execute` and their dependencies) that may orchestrate the edit. This is safe: a running Director/member loaded its skills into context at spawn time, so mid-run file edits do not alter the in-flight team's behavior — the slimmed files take effect on the **next** skill load. No special sequencing is required to protect an in-flight run; the verification gates run after edits land.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first note: per `.claude/rules/design-doc-numbering.md`, docs are updated before code. Here the only "docs" affected are the skill files being slimmed; `README.md` / `docs/concepts/` / `docs/` need no change because behavior is frozen (verified in Step 9, V6). This design doc is the documentation artifact.

### Step 1: Frozen-token baseline

- [x] Extract the V1 frozen-token inventory from all 26 in-scope files into `${BASE}/verification/frozen-inventory-before.md` (commands, flags, error strings, gates, frontmatter descriptions, verb/pointer tokens, spawn-prompt template hashes). <!-- completed: 2026-06-14T02:35 -->

> **`verification/` is throwaway scratch.** The before/after inventory artifacts live under the committed design-doc directory but are NOT part of the permanent record. Do NOT stage them in the finalize commit — leave `${BASE}/verification/` unstaged / ignored. The durable record is this design doc; the per-file diffs live in git history.

### Step 2: Cluster A — `cafleet/` reference files

- [x] `reference/exec-routing.md`: remove the "Why no operator-prompts-for-routing" § (D4). <!-- completed: 2026-06-14T02:45 -->
- [x] `reference/broadcast.md`: collapse the 4× `--full` no-op restatement to one + cross-ref `output-flags.md` (D7). <!-- completed: 2026-06-14T02:45 -->
- [x] `reference/recovery.md`: drop the duplicated `member list --activity` example block (D3); keep Shutdown Protocol canonical here. <!-- completed: 2026-06-14T02:45 -->
- [x] `reference/director.md`: trim the `--model` catalog (D6) and the ARG_MAX spawn-size derivation (D5); keep `--activity` table canonical. <!-- completed: 2026-06-14T02:45 -->
- [x] `reference/output-flags.md`: light P5 trim. <!-- completed: 2026-06-14T02:45 -->

### Step 3: Cluster A — `cafleet/` core + roles

- [x] `cafleet/SKILL.md`: trim "Why literal flags, not env vars?" to the one-line rule (D5). <!-- completed: 2026-06-14T02:53 -->
- [x] `cafleet/roles/member.md`: collapse the triple forbidden-behaviors statement (D4). <!-- completed: 2026-06-14T02:53 -->
- [x] `cafleet/roles/director.md`: light trim. <!-- completed: 2026-06-14T02:53 -->

### Step 4: Cluster B — `cafleet-design-doc/` shared

- [x] `coordination.md`: confirm as canonical Coordination Protocol home; minor self-trim. Leave `guidelines.md` / `template.md` / `SKILL.md` essentially as-is. <!-- completed: 2026-06-14T02:57 -->

### Step 5: Cluster C — `cafleet-design-doc-create/`

- [x] `SKILL.md`: replace inlined Coordination Protocol with cross-ref to `coordination.md` + role-subset note + Clarification Exemption (D1); collapse spawn audit-file procedure to the two-step pattern (D10). <!-- completed: 2026-06-14T03:22 -->
- [x] `roles/director.md`: cross-ref Idle/Stall to supervision/monitoring + Shutdown to recovery.md (D9, D2). <!-- completed: 2026-06-14T03:22 -->
- [x] `roles/drafter.md` + `roles/reviewer.md`: trim Placeholder + Shutdown + broker-keystroke boilerplate (D9). <!-- completed: 2026-06-14T03:22 -->

### Step 6: Cluster D — `cafleet-design-doc-execute/`

- [x] `SKILL.md`: replace inlined Coordination Protocol (D1); de-triplicate the spawn two-step procedure (D10); keep + tighten the silence / `reviewDecision` rationale (P2 exception); preserve all Steps 1–8 commands/tables/templates. <!-- completed: 2026-06-14T03:34 -->
- [x] `roles/director.md`: cross-ref Idle/Stall + Shutdown (D9, D2); keep Escalation + Commit Protocol + PR milestones. <!-- completed: 2026-06-14T03:34 -->
- [x] `roles/programmer.md` + `roles/tester.md` + `roles/verifier.md`: trim shared boilerplate (D9); keep all workflow/escalation logic. <!-- completed: 2026-06-14T03:34 -->

### Step 7: Cluster E — `cafleet-design-doc-interview/`

- [ ] `SKILL.md`: consolidate 3× `COMMENT(claude)` + 2× `question.md` format (D11); keep the single deliberate plugin-independent inline. <!-- completed: -->
- [ ] `roles/analyzer.md`: trim shared boilerplate (D9). <!-- completed: -->

### Step 8: Cluster F + G — agent-team + base-dir

- [ ] `monitoring/SKILL.md` + `supervision/SKILL.md`: collapse Shutdown/monitor-stop restatement to the recovery.md cross-ref (D2); remove residual three-beat re-spelling. <!-- completed: -->
- [ ] `cafleet-base-dir/SKILL.md`: light P5 trim; preserve every resolution branch + `<unset>` contract. <!-- completed: -->

### Step 9: Verification

- [ ] Re-extract the frozen-token inventory (`…-after.md`) and run V2 (multiset diff identical); investigate/justify every delta as a pure relocation or revert it. <!-- completed: -->
- [ ] Run V3 (frontmatter byte-identical), V5 (cross-reference integrity, incl. P7 for role files), V6 (no behavior-doc drift), V7 (removal totality). <!-- completed: -->
- [ ] Per-file `git diff` review (V4) confirming only redundancy was removed; run V8 (content-superset) for every prose block collapsed to a cross-reference. <!-- completed: -->

### Step 10: Finalize

- [ ] Record the global `~/.claude/skills/` re-sync as an operator follow-up in the completion report (out of scope per Q3). Mark this design doc complete. <!-- completed: -->

---

## Changelog (spec revisions only)

| Date | Changes |
|------|---------|
| 2026-06-14 | Initial draft |
