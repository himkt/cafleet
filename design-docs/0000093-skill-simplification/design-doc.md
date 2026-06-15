# Aggressively Simplify the `skills/` Tree (Tier C)

**Status**: Approved
**Progress**: 3/17 tasks complete
**Last Updated**: 2026-06-15

## Overview

Every skill directory under `skills/` (45 files, ~7,360 lines) carries restated rationale, cross-file and within-file duplication, non-enforced catalogs, generic upstream-recoverable content, and worked-example padding. This document plans an **aggressive (Tier C)** simplification that cuts prose/rationale, collapses duplication into cross-references everywhere (including the cafleet operational family — going beyond the conservative ~10% pass of `0000088`), drops within-file gate restatements, and consolidates the six `cafleet-my-slidev/techniques/*` files into three — while preserving a **byte-exact frozen set** (CLI tokens, error strings, frontmatter `description:` fields, theme tokens, the matplotlib palette/template, agent-browser commands, and the verb+pointer+`COMMENT(role)` vocabulary). Target: a **30–50% aggregate** line reduction as a stretch goal, correctness over percentage at every step.

## Success Criteria

- [ ] Every in-scope file is slimmed per the Tier C principles (§*Slimming principles*), with **zero change to any frozen token** (§*The frozen set*) — proven, not assumed, by the before/after extraction diff (V2).
- [ ] **Aggregate line reduction is ≥ 30%** (stretch band 30–50%; modeled realistic outcome ~30–37%), **OR** any shortfall below 30% is recorded with a per-cluster justification (V10) — **correctness over percentage**: no load-bearing or frozen content is ever cut to hit a number.

- [ ] `cafleet-my-slidev/techniques/` is consolidated **6 → 3** (`formatting.md`, `math-formulas.md`, `two-column-layouts.md`); `animations.md`, `admonition.md`, `highlight.md`, `font-size.md` are removed; **every theme token** (layout/class/slot/component-type/frontmatter-key) survives the merge, and **all four pointer-sources (8 line-instances) are repointed** (SKILL.md techniques table = 4 rows, `admonition.md`→`highlight.md`, `math-formulas.md`→`admonition.md`, and the cross-skill `cafleet-research-presentation/roles/presentation.md`→`highlight.md` = 2 lines).
- [ ] **No other file is merged, deleted, renamed, or split.** `research-report/template.md`, `roles/scout.md`, `roles/researcher.md`, and `research-presentation/roles/director.md` stay as separate files. No new cross-skill shared template files are created.
- [ ] Cross-reference integrity holds: every "see X § Y" pointer resolves to an existing heading; every role-file cross-reference points only to a skill the spawned member loads at startup (P7).
- [ ] No "for history" deprecation breadcrumbs remain (`.claude/rules/removal.md`): each slimmed file reads as if the removed redundancy never existed.
- [ ] No `README.md` / `docs/concepts/` / `docs/` change is required, because no CLI / API / config / theme-token / file-path surface changes — verified (V6), not assumed.

---

## Background

### Scope (locked)

In scope: **all skill directories under `skills/`** — the cafleet operational family (`cafleet`, `cafleet-design-doc`, `cafleet-design-doc-create`, `cafleet-design-doc-execute`, `cafleet-design-doc-interview`, `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-base-dir`) plus the creative/research family (`cafleet-create-figure`, `cafleet-my-slidev`, `cafleet-research-report`, `cafleet-research-presentation`). That is **45 files, ~7,362 lines, 12 directories**.

**Out of scope** (per user decision): `skill-author` and `update-readme`. Both physically live under `.claude/skills/`, **not** under `skills/`, and are excluded entirely — no edits, and the `skill-author` `copilot`-role drift is **not** fixed here.

Edit target: the **repo-committed** files under `skills/` (git-tracked source of truth). The installed copy at `~/.claude/skills/` is a separate physical directory and is **out of scope** — see *Global copy propagation* below.

### Relationship to the prior pass `0000088`

`design-docs/0000088-skill-md-simplification/design-doc.md` performed a **conservative, behavior-preserving** de-duplication of the **cafleet operational family only** — **~10% achieved (−403 net lines of 4108)**; its ~14% (~590-line) figure was the pre-implementation *estimate*, not the realized result — under a **hard freeze** on every CLI command, flag, error string, gate, frontmatter `description`, verb/pointer token, and spawn-prompt template body. That freeze is exactly what held it near ~10%.

This pass:

- **Re-opens the operational family** under Tier C: aggressive prose/rationale cuts, collapse duplication into cross-references, and **drop within-file gate restatements** (a gate's *canonical* statement and its literal conditions stay; a *second restatement of the same gate in the same file* may go).
- **Softens the spawn-prompt-template freeze**: template **bodies** may be slimmed *in place* (the surrounding authoring prose and repeated procedures collapse; the frozen tokens *inside* each body — the literal `cafleet message send …` command, the `{fleet_id}`/`{agent_id}`/`{director_agent_id}` placeholders, the identity block — stay).
- **Adds the net-new creative/research family**, which `0000088` never touched and which holds the largest headroom (worked-example padding, embedded agent specs, generic upstream Slidev/KaTeX content).

### Current inventory (45 files, 7,362 lines)

| Cluster | Dir | Files | Lines |
|:--|:--|--:|--:|
| A | `cafleet/` (SKILL + 5 reference + 2 roles) | 8 | 962 |
| B | `cafleet-design-doc/` (SKILL + coordination + guidelines + template) | 4 | 279 |
| C | `cafleet-design-doc-create/` (SKILL + 3 roles) | 4 | 645 |
| D | `cafleet-design-doc-execute/` (SKILL + 4 roles) | 5 | 1132 |
| E | `cafleet-design-doc-interview/` (SKILL + analyzer) | 2 | 418 |
| F | `cafleet-agent-team-monitoring` + `cafleet-agent-team-supervision` | 2 | 309 |
| G | `cafleet-base-dir/` (SKILL) | 1 | 84 |
| H | `cafleet-create-figure/` (SKILL) | 1 | 265 |
| I | `cafleet-my-slidev/` (SKILL + 6 techniques) | 7 | 1159 |
| J | `cafleet-research-report/` (SKILL + template + 4 roles) | 6 | 1133 |
| K | `cafleet-research-presentation/` (SKILL + 4 roles) | 5 | 976 |
| | **Total** | **45** | **7362** |

Operational subtotal (A–G): **3,829 lines / 26 files.** Creative/research subtotal (H–K): **3,533 lines / 19 files.**

---

## Specification

### The frozen set (byte-exact — never edited)

The implementation MUST preserve, byte-for-byte:

1. **Frontmatter `description:` fields** — the skill auto-load triggers. Zero changes to any `---`-delimited frontmatter block.
2. **Literal CLI commands, subcommands, flags, options, and env vars** — every span beginning `cafleet `, every `--flag`, every `CAFLEET_*` / `BASE` / `FIGURE_BASE` token, exactly as written.
3. **Error / sentinel strings** — every `Error: …`, `… failed:`, `… is required`, `… must be …`, plus sentinels like `<unset>`, `ready: monitor live`, `command too long`.
4. **agent-browser commands** — every `bun run agent-browser …` invocation and its flags (`--session`, `open`/`screenshot`/`snapshot`/`console`/`errors`/`close`, `--open false`, the `script -qfc` wrap), and the documented `wait`-is-discouraged constraint.
5. **Slidev theme tokens** — layout names (`cover`, `bullets`, `bullets-sm`, `two-cols`, `blank`, `stats-grid`, `section-divider`, `end`); `Admonition` types (`note`, `important`, `tip`, `warning`, `caution`, `formula`); `Highlight` types (`primary`, `positive`, `negative`, `accent`, `important`); `stats-grid` type values; slot markers (`::header::`, `::default::`, `::left::`, `::right::`, **including the `:: header ::` whitespace variant exactly where it currently appears** — do NOT normalize); frontmatter keys (`theme:`, `layout:`, `columns:`, `stats:`, `section:`, `totalSections:`, `fontSize:`, `fonts:`); CSS classes/vars (`.figure-caption`, `.bg-primary-light`, `.v-center`, `--c-primary`, `--c-accent`, `--c-positive`, `--c-negative`, `--c-important`); the literal `theme:` path tail; the per-layout `fontSize` defaults (`cover "22px"`, `bullets "18px"`, `bullets-sm "14px"`, …).
6. **matplotlib palette + save-template** — every hex literal (`#3B82F6`, `#94A3B8`, `#1E40AF`, `#DC2626`, `#1E293B`, `#64748B`, `#E2E8F0`, `#93C5FD`, `#CBD5E1`); the save template (`matplotlib.use("Agg")`, `font.family = 'Noto Sans'`, `savefig(..., dpi=150, bbox_inches="tight", facecolor='white')`, `plt.close()`, the `script_stem`→`.png` naming); the hard rules (no `plt.show()`, no `ax.set_title()`, ≤2 colors, English-only text); the chart-type→API table.
7. **Verb + pointer + marker vocabulary** — the six verbs (`ready`/`complete`/`addressed`/`blocked`/`escalating`/`approved`), the three pointer forms, the ` > ` separator, the `COMMENT(role)` grammar and role taxonomy.
8. **Gate *conditions*** — the literal thresholds and branch conditions (`state == "APPROVED"`, `submittedAt > last_push_ts`, `silence_ticks >= 30`, round limits, the three-tier path-detection conditions, the `<unset>` branch, the discovery-flow count table). The canonical statement of each gate stays; only a **duplicate restatement of the same gate within the same file** is removed.
9. **The CLI command and `{…}` placeholders *inside* spawn-prompt bodies** — the body prose may be slimmed, but the literal `cafleet message send …` line, the `{fleet_id}`/`{agent_id}`/`{director_agent_id}` placeholders, the `[INSERT …]` markers, and the `ROLE DEFINITION:`/`COMMUNICATION PROTOCOL:` structural lines stay.

A **relocation** (a fact moves from an inlined copy to its canonical home + a one-line cross-reference) is allowed iff the canonical occurrence still exists and is reachable. An outright **disappearance** of any frozen token is a defect.

### Slimming principles (Tier C — reusable, mechanical)

Extends `0000088`'s P1–P8; P9–P11 are new for Tier C and the creative family.

| ID | Principle | What it cuts | What it keeps |
|:--|:--|:--|:--|
| **P1** | One canonical home per fact. | A command/gate/protocol/convention stated in file B that already lives canonically in file A. | The statement in A + a one-line `see A § Y` cross-reference in B. |
| **P2** | Cut restated rationale. | Multi-paragraph "Why …" blocks justifying a past decision. | The one-line operational rule. **Exception:** rationale that is *decision-critical for correctness* stays, **tightened** (Tier C tightens harder than `0000088`). |
| **P3** | Cut non-enforced catalogs + generic upstream content. | Example lists the tool does not validate against; generic Slidev/KaTeX syntax recoverable from `/slidev`. | The rule + ≤1 example; a `/slidev` pointer for upstream syntax. |
| **P4** | De-duplicate within a file. | The same fact (or **the same gate**) stated multiple times in one file. | One statement, at the canonical location. |
| **P5** | Cut mechanics obvious from the command/code surface. | Prose re-explaining what a flag/line plainly does. | Non-obvious behavior only. |
| **P6** | FROZEN set — never edited. | (nothing) | Everything in §*The frozen set*. |
| **P7** | Role-file standalone constraint. | (limits P1 for role files) | A role file may cross-reference **only** a skill its spawned member loads at startup. Never point a member at a file it will not load. |
| **P8** | Removal totality. | "For history" / "X was here" / deprecation breadcrumbs. | A file that reads as if the redundancy never existed. |
| **P9** | Cut worked-example padding. | Full multi-line slide/code blocks shown to illustrate a rule already stated; redundant secondary examples. | One compact example per distinct construct; the rule itself; every frozen token shown in the example. |
| **P10** | Spawn-template in-place slim. | Repeated render→write→`--prompt-file` procedure prose; redundant `IMPORTANT:` lines duplicated across sibling templates. | Each template body's frozen tokens (P6 #9). **No new cross-skill shared template file** is created (any such proposal is flagged for user approval, not done unilaterally). |
| **P11** | Technique consolidation (my-slidev only). | The six `techniques/*` files. | Merged into three; every theme token (P6 #5) preserved; every inbound pointer repointed. |

### Cross-cutting duplication map

The highest-yield redundancies, by family. Locators are pre-edit coordinates paired with durable anchors; the implementer locates by heading, not by drifted line number.

| ID | Duplication | Canonical home | Where the copies live |
|:--|:--|:--|:--|
| **X1** | Role-file boilerplate: "Load at Startup", "Communication Protocol" send-block + broker-keystroke poll paragraph + "Pane silence is normal", and the `/exit`-15s "Shutdown" block. | The `cafleet` skill (members load it) + the spawn prompt's "Load these skills" list. | Every member role file in clusters C/D/E/J/K (≈4 copies per family). |
| **X2** | The render→write→`--prompt-file` **two-step spawn procedure** re-spelled at each spawn site (3× in execute, 3× in research-report, 3× in research-presentation). | One "Spawn procedure" statement per SKILL.md. | `execute/SKILL.md` 3e (×3); `research-report/SKILL.md` (×3); `research-presentation/SKILL.md` (×3). |
| **X3** | Shutdown / teardown sequence stated in full in both a SKILL.md Step and its `roles/director.md`. | The `cafleet` skill § Shutdown Protocol (both already cite it). | `execute`, `research-report`, `research-presentation` (SKILL Step ↔ director.md). |
| **X4** | Sibling-skill restatement: slide-layout / figure / highlight tables copied into a member role file that already loads the owning skill. | `cafleet-my-slidev` + `cafleet-create-figure` (loaded at startup). | `research-presentation/roles/presentation.md` (Layout, Figures, Text-Emphasis tables). |
| **X5** | Embedded agent spec inlined into a SKILL.md (web-researcher / slide-creator) + three dispatch recipes (Claude / codex-inline / codex-member). | (per-skill, trim in place) | `research-report/SKILL.md` (~193 ln); `my-slidev/SKILL.md` (~108 ln). |
| **X6** | Within-file gate restatement: the Copilot loop branch table (7b) restated imperatively in the "Per idle-nudge turn checklist". | 7b's branch table (canonical). | `execute/SKILL.md` lines 610–634. |
| **X7** | Cross-file formula-wrap rule + color-token table stated 4× across SKILL.md / admonition / highlight / math-formulas; worked-example slide blocks are >50% of each technique file. | After merge: `formatting.md` (color tokens + admonition/highlight) and the formula-wrap rule once. | `my-slidev` SKILL + all 6 technique files. |

### The `cafleet-my-slidev/techniques/` consolidation (6 → 3)

The only approved structural change. End state:

| New file | Absorbs | Keeps (load-bearing) | Drops |
|:--|:--|:--|:--|
| **`formatting.md`** (new) | `admonition.md` + `highlight.md` + `font-size.md` | `Admonition` types table + props; `Highlight` types table + props; the `fontSize` per-layout defaults table; the bold-redundancy rule; the color-discipline rule (stated **once**). | Repeated full-slide example blocks; the color-token table restated 3×; the layout frontmatter re-shown per example. |
| **`math-formulas.md`** (kept) | (itself) | The `<Admonition type="formula">` **wrapping rule** + the wrapped-vs-inline table; **1–2** trimmed slide examples. | The generic KaTeX syntax tables (Greek/operators/superscripts/matrices) → replaced by a `/slidev` pointer; the duplicate Attention/Cross-Entropy examples that also live in admonition; the in-file 3× restatement of the wrap rule. |
| **`two-column-layouts.md`** (kept) | (itself) | The Syntax block; Frontmatter + Slots tables (frozen slot markers + `columns`); the Column-Ratios mapping. | Two of the three worked examples (keep one compact). |
| ~~`animations.md`~~ | **DELETED** | (nothing theme-specific exists in it — verified by audit) | The whole file: `v-click`/`v-clicks`/`v-after`/line-ranges are stock upstream Slidev. Access preserved via the existing `/slidev` pointer. |

**Pointer repoints (four pointer-sources = 8 line-instances, all mandatory — V5):** the SKILL.md techniques table (4 rows: lines 159/160/161/163) + `presentation.md` (2 lines: 75, 77) + `admonition.md:191` + `math-formulas.md:154`.

1. `my-slidev/SKILL.md` techniques table (pre-edit lines 158–163): `admonition.md`/`highlight.md`/`font-size.md` rows → **`techniques/formatting.md`**; `animations.md` row → a `/slidev` pointer (or removed with a one-line "for click animations see `/slidev`"); `math-formulas.md` / `two-column-layouts.md` rows unchanged.
2. `my-slidev/techniques/admonition.md:191` `see techniques/highlight.md` → internal to `formatting.md` (the reference dissolves once both live in one file).
3. `my-slidev/techniques/math-formulas.md:154` `See techniques/admonition.md for full details` → **`techniques/formatting.md`**.
4. **Cross-skill:** `cafleet-research-presentation/roles/presentation.md:75,77` `techniques/highlight.md` → **`techniques/formatting.md`** (the presentation member loads the my-slidev skill and opens this path; it MUST resolve).

Verified: `README.md` and `docs/` contain **no** references to any technique filename, so the renames cause no behavior-doc drift.

### Per-cluster plan with before → after estimates

> Estimates are **conservative floors** derived from the per-file audits; aggressive editing typically exceeds them. Every cut is a P1–P11 application; no frozen token (P6) is touched.

**Cluster A — `cafleet/` (962 → ~832).**
- `SKILL.md` 306→268: trim restated Global-Options examples, Typical-Workflow ↔ command-section overlap, obvious-from-states Message-Lifecycle prose. Keep every command section, required-flags table, error strings, reserved-name/root-Director guards.
- `reference/director.md` 272→238: trim the `--model` model-name prose and the member-create scratch/audit narration (cross-ref `cafleet-base-dir`); keep every `member *` command, flag table, error string, send-input pane-shape table.
- `reference/{broadcast,exec-routing,recovery,output-flags}.md` 240→206: P5/P2 trims; keep the two-primitive table, recovery trees, Shutdown Protocol (canonical home), broadcast threading/ack semantics.
- `roles/director.md` 52→44, `roles/member.md` 92→76: trim X1 boilerplate; keep the ready-signal first-action, default rule, denial-routing pointer.

**Cluster B — `cafleet-design-doc/` (279 → ~253).** `coordination.md` 151→132 (canonical Coordination Protocol home — self-trim restated examples only; it *grows* in authority as other files cross-ref it). `guidelines.md`/`template.md`/`SKILL.md`: near-as-is.

**Cluster C — `cafleet-design-doc-create/` (645 → ~500).** `SKILL.md` 365→270 (aggressive Step 0–6 prose cuts; slim both Drafter spawn templates — normal + resume — in place per P10; drop within-file restatements). `roles/{director,drafter,reviewer}.md` 280→230 (X1 boilerplate → cross-refs; Idle/Stall → supervision/monitoring pointer; keep the structured-question framework, milestones, review-tag taxonomy).

**Cluster D — `cafleet-design-doc-execute/` (1132 → ~840) — the fattest.**
- `SKILL.md` 678→470: dedup the 3× spawn-procedure prose to one (P10/X2); collapse the "Per idle-nudge turn checklist" by dropping its re-spelled gate conditions and keeping only the command list that references 7b (X6 — **7b's branch table and all literal thresholds stay frozen**); tighten the "Why no auto-exit on silence" / "Why not `reviewDecision`" rationale (P2 exception — keep the conclusion, cut the prose); trim the Step 1 base-dir edge-case narration and the discovery-flow example blocks; trim the Architecture ↔ Primitive-Mapping overlap. **Keep** every `gh`/`git`/`cafleet` command, the three-tier table, the Copilot loop gate conditions, the error tables, and all three spawn-template bodies (frozen tokens intact).
- `roles/{director,programmer,tester,verifier}.md` 454→370: X1/X3 boilerplate → cross-refs; keep each role's workflow phases, escalation/defect logic, Commit Protocol, and every verb/pointer/marker instruction.

**Cluster E — `cafleet-design-doc-interview/` (418 → ~320).** `SKILL.md` 321→240 (prose cuts; keep the consolidated single `COMMENT(claude)` + single `question.md` spec from `0000088`, the multi-session resume tables, the mandatory-completion gate, `$ARGUMENTS`). `roles/analyzer.md` 97→80 (X1 boilerplate).

**Cluster F — agent-team (309 → ~248).** `monitoring/SKILL.md` 173→138, `supervision/SKILL.md` 136→110: collapse the Shutdown/monitor-stop restatement to the `recovery.md` cross-ref; remove residual three-beat re-spelling; keep the heartbeat mechanism, facilitation loop, health-check sequence, Authorization-Scope Guard, Spawn Protocol, Quick-Reference table.

**Cluster G — `cafleet-base-dir/` (84 → ~76).** Light P5 trim; preserve every resolution branch and the `<unset>` contract.

**Cluster H — `cafleet-create-figure/` (265 → ~205, ~23% — reference floor).** Collapse the triplicated placeholder-vs-shell-var warning, the two duplicate base-dir resolution walk-throughs, the thrice-stated "runner is host-specific" point, the GridSpec secondary example, and the decision-table ↔ prohibition-prose overlap; drop key-points bullets that merely narrate visible template code. **Keep** the palette hex, save-template, chart-type→API table, and all hard rules (P6 #6). This file cannot reach 30% without losing reference value — accepted.

**Cluster I — `cafleet-my-slidev/` (1159 → ~600, ~48%).**
- `SKILL.md` 272→205: collapse the three `slide-creator` dispatch recipes (Claude / codex-inline / codex-member) to one + pointer; dedup the twice-stated `theme:` path paragraph; cut the Generation-Workflow ↔ Output-Constraints overlap and the Layout-Examples padding; repoint the techniques table. **Keep** the headmatter template, the Layouts table, color tokens, and the `slide-creator` spec contract the parent claims is read verbatim.
- techniques 887→~395: the 6→3 consolidation above (P9/P11/X7).

**Cluster J — `cafleet-research-report/` (1133 → ~750).**
- `SKILL.md` 547→300: trim the embedded `web-researcher` spec + its three dispatch recipes (X5); collapse the 3× spawn-procedure (X2); cross-ref monitoring/base-dir/shutdown; dedup the Architecture tree ↔ channel bullets ↔ prose. Keep every command, the three spawn-template bodies, the output-path conventions, the verification-tag literals.
- `template.md` 95→78 (**kept separate**): trim the quality-standards dup + reference-format verbosity.
- `roles/director.md` 140→105 (merge Checklist ↔ Feedback-Tags into one tag-keyed table; X3 shutdown → cross-ref). `roles/manager.md` 171→120 (trim delegation philosophy, scout-phase overlap, task-coordination dup). `roles/{researcher,scout}.md` 85→70 / 95→75 (**kept separate**; X1 boilerplate → cross-refs; keep verification-tag table / output-format schema).

**Cluster K — `cafleet-research-presentation/` (976 → ~670).**
- `SKILL.md` 384→255: collapse the 3× spawn-procedure (X2); make Shutdown canonical in ONE place and cross-ref (X3 — **`roles/director.md` stays a separate file**); trim the teardown-overhead and path-by-reference rationale blockquotes; dedup the repeated literal-id warnings.
- `roles/director.md` 157→110 (**kept separate**): Shutdown → cross-ref; dedup the tag tables to one home; User-Delegation → pointer. **Keep the Visual Quality Ownership checklist intact** (the unique quality gate).
- `roles/presentation.md` 153→110: repoint Layout/Figure/Text-Emphasis tables to the owning skills (X4 — incl the `formatting.md` repoint); merge the overlapping Single-Line-Bullet and Text-Wrapping sections.
- `roles/transcript.md` 103→80 (X1 boilerplate; dedup the tag table). `roles/visual-reviewer.md` 179→140: strip the `MULTILINE_BULLET` *remediation* pedagogy (keep the *detection* criterion); dedup the thrice-stated persist rule. **Keep** the per-slide capture loop and every `bun run agent-browser` command (P6 #4).

### Aggregate target reconciliation (honest)

| Cluster | Before | Est. after | Est. cut |
|:--|--:|--:|--:|
| A `cafleet/` | 962 | 832 | 13.5% |
| B `cafleet-design-doc/` | 279 | 253 | 9% |
| C `…-create/` | 645 | 500 | 22% |
| D `…-execute/` | 1132 | 840 | 26% |
| E `…-interview/` | 418 | 320 | 23% |
| F agent-team | 309 | 248 | 20% |
| G `…-base-dir/` | 84 | 76 | 10% |
| H `…-create-figure/` | 265 | 205 | 23% |
| I `…-my-slidev/` | 1159 | 600 | 48% |
| J `…-research-report/` | 1133 | 750 | 34% |
| K `…-research-presentation/` | 976 | 670 | 31% |
| **Total** | **7362** | **~5294** | **~28%** |

The modeled floor lands at **~28%**, just under the 30% target, because two of the heaviest levers — folding `research-report/template.md`, merging `scout.md`+`researcher.md`, and dissolving `research-presentation/roles/director.md` — were **declined** (the files stay separate), and the operational family is reference-dense. The estimates above are **conservative**; aggressive editing of the worked-example padding (clusters I/J/K) and the operational rationale (cluster D) typically exceeds them, so the **realistic outcome is ~30–37%**. The implementation commits to **≥30% as the stretch target**; if a cluster cannot reach its estimate without cutting load-bearing content, the shortfall is **recorded, not forced** (the 30–50% band is a stretch goal, not a hard per-file requirement — `create-figure` at ~23% is explicitly accepted). The upper half of the band (40–50%) was foreclosed by the declined merges and is out of reach under these constraints.

End-state file count: **45 → 42** (the 6→3 techniques consolidation removes 3 files; no other structural change).

### Verification strategy (proves the frozen set is preserved)

All V-gates run at **execute time** over the slimmed diffs. The create-time Reviewer reviews *this design doc* for completeness; the skill-file diffs do not exist yet at design time.

| ID | Gate | Method |
|:--|:--|:--|
| **V1** | Frozen-token baseline (pre-edit). | Before any edit, extract from all 45 files into `${BASE}/verification/frozen-inventory-before.md` using mechanical extractors: (a) commands = every `cafleet ` span; (b) flags/env = every `--[a-z-]+` and `CAFLEET_*`/`BASE`/`FIGURE_BASE`; (c) error/sentinel strings = lines matching `Error:`/`failed:`/` must be `/` is required`/`<unset>`; (d) frontmatter `description:` blocks; (e) verb+pointer vocabulary; (f) spawn-template body hashes; (g) **agent-browser** = every `bun run agent-browser ` span; (h) **theme tokens** = the layout/component-type/slot/class/frontmatter-key set; (i) **matplotlib** = every `#`-hex literal + the save-template lines. |
| **V2** | Frozen-token diff (post-edit). | Re-extract (a)–(i) into `…-after.md`. Multisets MUST be identical (modulo relocation: a moved token's canonical occurrence must be confirmed present). Spawn-template bodies: frozen tokens (P6 #9) byte-identical even though surrounding prose changed. Any delta is reverted or justified as a pure relocation. |
| **V3** | Frontmatter byte-identical. | `git diff` shows zero changes inside any `---` frontmatter block. |
| **V4** | Per-file diff review. | Every file's `git diff` read to confirm each removed line is redundant prose / duplication / non-enforced catalog / example padding — never an operational instruction or frozen token. The human/Reviewer gate. |
| **V5** | Cross-reference integrity. | Every "see X § Y" pointer resolves to an existing heading. **Technique-merge pointers**: grep the whole `skills/` tree for `admonition.md`/`highlight.md`/`font-size.md`/`animations.md`; every hit is repointed (the four enumerated above) or gone. Role-file refs (P7) point only to a skill the member loads. |
| **V6** | Behavior-doc drift. | Confirm no `README.md`/`docs/concepts/`/`docs/` change is needed (CLI/API/config/theme-token/file-path surface unchanged; technique renames verified absent from README/docs). If any such doc *would* need a change, a behavior change leaked — stop and revert. |
| **V7** | Removal totality (P8). | No "for history"/deprecation/restoration breadcrumbs introduced. |
| **V8** | Content-superset for collapsed prose. | For every block collapsed into a cross-reference (X1 Communication/Shutdown, X3 teardown, X4 sibling tables, Idle/Stall rungs), confirm the target already states **every** operational step removed — the target must be a superset. V5 proving the heading merely exists is insufficient. |
| **V9** | Theme-token survival (my-slidev merge). | Diff the union of theme tokens (P6 #5) across the 6 pre-merge technique files vs the 3 post-merge files — the set MUST be identical. No `Admonition`/`Highlight`/`stats-grid` type, slot marker, `fontSize` default, or CSS class disappears in the merge. |
| **V10** | Line-count delta. | Compute per-file and aggregate before/after line counts. Aggregate ≥ 30% is the target; a cluster below its estimate must carry a recorded justification (V4 confirms no load-bearing content was cut to compensate). |

### Global copy propagation (out of scope)

The repo `skills/` tree is the only edit target. `~/.claude/skills/` is a separate physical directory; re-syncing it after merge is an **operator follow-up**, noted here so it is not forgotten. No `.md` edits are planned against the global copy.

### Meta-safety note

This change edits the orchestrating skills (`cafleet-design-doc-create`/`-execute` and dependencies) that may run the edit. A running Director/member loaded its skills at spawn time, so mid-run file edits do not alter the in-flight team — the slimmed files take effect on the next skill load. No special sequencing is required; the V-gates run after edits land.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first note: the only "docs" affected are the skill files being slimmed; `README.md`/`docs/concepts/`/`docs/` need no change because no CLI/API/config/theme-token/file-path surface changes (verified in V6). This design doc is the documentation artifact.
> `${BASE}/verification/` is throwaway scratch — do NOT stage the before/after inventory artifacts in any commit.

### Step 1: Frozen-token baseline (V1)

- [x] Extract the V1 frozen-token inventory (extractors a–i) from all 45 in-scope files into `${BASE}/verification/frozen-inventory-before.md`; record per-file and aggregate baseline line counts. <!-- completed: 2026-06-15T13:51 -->

### Step 2: Cluster A — `cafleet/` core + reference + roles

- [x] Slim `SKILL.md`, `reference/{director,broadcast,exec-routing,recovery,output-flags}.md`, `roles/{director,member}.md` per the cluster-A plan (P1/P2/P5/X1); keep every command, flag table, error string, guard. <!-- completed: 2026-06-15T14:09 -->

### Step 3: Cluster B — `cafleet-design-doc/` shared

- [x] Self-trim `coordination.md` (canonical home); leave `guidelines.md`/`template.md`/`SKILL.md` near-as-is. <!-- completed: 2026-06-15T14:17 -->

### Step 4: Cluster C — `cafleet-design-doc-create/`

- [ ] Slim `SKILL.md` (Step 0–6 prose; in-place slim both Drafter spawn templates per P10; drop within-file restatements) and `roles/{director,drafter,reviewer}.md` (X1; Idle/Stall → supervision/monitoring cross-ref). <!-- completed: -->

### Step 5: Cluster D — `cafleet-design-doc-execute/`

- [ ] Slim `SKILL.md`: dedup the 3× spawn-procedure (X2/P10), collapse the per-idle-nudge checklist (X6 — keep 7b's gate conditions frozen), tighten the Copilot rationale (P2), trim Step 1 base-dir narration + discovery examples + Architecture/Primitive-Mapping overlap. Preserve all Steps 1–8 commands, tables, gate conditions, and the three spawn-template bodies. <!-- completed: -->
- [ ] Slim `roles/{director,programmer,tester,verifier}.md` (X1/X3 → cross-refs); keep all workflow/escalation/commit logic. <!-- completed: -->

### Step 6: Cluster E — `cafleet-design-doc-interview/`

- [ ] Slim `SKILL.md` (prose; keep the single `COMMENT(claude)`/`question.md` spec, resume tables, completion gate) and `roles/analyzer.md` (X1). <!-- completed: -->

### Step 7: Cluster F + G — agent-team + base-dir

- [ ] Collapse the Shutdown/monitor-stop restatement + residual three-beat re-spelling in `monitoring/SKILL.md` + `supervision/SKILL.md`; light P5 trim of `cafleet-base-dir/SKILL.md` preserving every resolution branch + `<unset>` contract. <!-- completed: -->

### Step 8: Cluster H — `cafleet-create-figure/`

- [ ] Slim `SKILL.md` (collapse triplicated placeholder/runner warnings, duplicate base-dir examples, GridSpec example, decision-table↔prohibition overlap); keep palette, save-template, API table, hard rules (P6 #6). <!-- completed: -->

### Step 9: Cluster I — `cafleet-my-slidev/` SKILL + techniques consolidation

- [ ] Slim `SKILL.md` (collapse 3 dispatch recipes → 1, dedup the `theme:` path paragraph, cut Generation-Workflow↔Output-Constraints + Layout-Examples padding). <!-- completed: -->
- [ ] Create `techniques/formatting.md` (merge admonition + highlight + font-size; dedup color tables; keep all type/props/fontSize tables + bold-redundancy rule); trim `techniques/math-formulas.md` (strip generic KaTeX → `/slidev`; keep formula-wrap rule + table + 1–2 examples) and `techniques/two-column-layouts.md` (1 example; keep all tables); **delete** `admonition.md`, `highlight.md`, `font-size.md`, `animations.md`. <!-- completed: -->
- [ ] Repoint all four pointer-sources (8 line-instances): SKILL.md techniques table (4 rows), `math-formulas.md`→`formatting.md`, the internal admonition↔highlight ref, and **`research-presentation/roles/presentation.md:75,77`→`techniques/formatting.md`** (V5). <!-- completed: -->

### Step 10: Cluster J — `cafleet-research-report/`

- [ ] Slim `SKILL.md` (trim web-researcher spec + dispatch recipes X5; dedup 3× spawn-procedure X2; cross-ref monitoring/base-dir/shutdown). Slim `roles/{director,manager,researcher,scout}.md` and `template.md` (X1; merge director Checklist↔Tags) — **all kept as separate files**; keep verification-tag literals, output-path conventions, spawn-template bodies. <!-- completed: -->

### Step 11: Cluster K — `cafleet-research-presentation/`

- [ ] Slim `SKILL.md` (dedup 3× spawn-procedure X2; ONE canonical Shutdown + cross-ref X3; trim rationale blockquotes) and `roles/{director,presentation,transcript,visual-reviewer}.md` — **all kept separate**; repoint sibling-skill tables (X4) incl `formatting.md`; strip `MULTILINE_BULLET` remediation (keep detection); **keep Visual Quality Ownership + every agent-browser command intact**. <!-- completed: -->

### Step 12: Verification — frozen-set diff

- [ ] Re-extract the frozen-token inventory (`…-after.md`); run V2 (multiset diff identical), V3 (frontmatter byte-identical), V9 (theme-token survival across the merge). Investigate/justify every delta as a pure relocation or revert. <!-- completed: -->

### Step 13: Verification — integrity + drift + totality

- [ ] Run V5 (cross-reference integrity, incl. the technique-merge grep + P7), V6 (no behavior-doc drift), V7 (removal totality), V8 (content-superset for every collapsed prose block). <!-- completed: -->

### Step 14: Verification — line-count + finalize

- [ ] Run V4 (per-file `git diff` review — only redundancy removed) and V10 (per-file + aggregate line-count delta; aggregate ≥ 30% or recorded justification). Record the global `~/.claude/skills/` re-sync as an operator follow-up. Mark this design doc complete. <!-- completed: -->

---

## Changelog (spec revisions only)

| Date | Changes |
|------|---------|
| 2026-06-15 | Initial draft |
