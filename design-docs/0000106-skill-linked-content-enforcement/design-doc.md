# Skill Linked-Content Enforcement

**Status**: Approved
**Progress**: 11/27 tasks complete
**Last Updated**: 2026-06-21

## Overview

Agents reliably enter a cafleet-family skill but routinely do **not** follow its `Read [reference/X.md]` links, so they miss load-bearing protocol (base-dir write rules, supervision governance, the coordination schema, overlay token values). Keep the reference-file architecture and make the load-bearing links unskippable by giving every reader entry point a per-reader-role **Required reading** convention: gated numbered load steps plus a load-bearing-vs-optional classification that states the concrete cost of skipping each file. Apply it uniformly across all three skill families (`cafleet`, `cafleet-design-doc`, `cafleet-research`).

## Success Criteria

- [ ] Every reader entry point (each `SKILL.md` dispatch surface, each workflow body, each `roles/*.md`) carries a **Required reading** block scoped to that reader's role.
- [ ] Every linked reference an entry point names is classified **load-bearing** (for that reader) or **optional/on-demand**, and every load-bearing entry carries a one-line "what you lose if you skip it" consequence.
- [ ] The overlay (`reference/coding-agent/<name>.md`) is the first load-bearing read in every entry point that uses `{placeholder}` tokens.
- [ ] **Per-reader-role is preserved (hard constraint):** no member entry point force-reads a Director-only reference page; the 0000103/0000104 token-reduction split stays intact (members still skip `cafleet/reference/*` governance).
- [ ] **Behavioral validation passes:** in a fresh coding-agent session, a triggering prompt causes the reader to apply at least one protocol it could only know by reading a linked file (per the per-family probes in Step 6).
- [ ] No new tooling is added (prose/instruction-design only): no mise lint task, no runtime read-assertion handshake.
- [ ] Removal is complete: no load-bearing link is left in the old undifferentiated "Read on demand" phrasing; the entry points read as if the soft phrasing for load-bearing content never existed.

---

## Background

GitHub issue #137 ("Agent does not read the contents linked on SKILL.md", empty body — the title is the whole report). The cafleet skill family uses a heavy path-by-reference design: `SKILL.md` and workflow bodies stay compact and point at `reference/*.md`, `roles/*.md`, and `reference/coding-agent/<name>.md` overlays, deliberately, to keep per-context-load cost down ([token-reduction](../../docs/concepts/token-reduction.md) § *Skill-file split*; designs 0000088/0000093/0000094/0000103/0000104). That split is correct and stays. The defect is that the links are easy to glide past, so the saved-tokens architecture leaks critical instructions.

### Where and why agents skip — audit findings

An audit of ~120 cross-references across all three families isolated four failure factors. Every reader entry point exhibits at least one.

| # | Factor | Mechanism | Representative site |
|---|--------|-----------|---------------------|
| F1 | **Undifferentiated verb + syntax** | Load-bearing, on-demand, and purely-optional reads all use the same `Read [X.md]` form and the same `Read` verb. The reader cannot tell a must-read from a footnote and treats them uniformly — as optional. | `cafleet/SKILL.md:18-27` lists eight reference files under one **"Read on demand"** heading; `reference/base-dir.md` (load-bearing for any writer) sits in the same list as `reference/cli.md` (genuinely optional). |
| F2 | **No stated consequence** | The link says *what* to read, never *what breaks* if skipped. Under token/latency pressure the reader rationalizes skipping because no cost is visible. | `cafleet-research/report/roles/manager.md:8` "also Read its `reference/base-dir.md`" — names the file with no stated cost; skipping silently mis-roots every file write. |
| F3 | **No single unmissable gate** | Startup reads are scattered down the entry point (overlay near the top, base-dir at Step 0, supervision at Step 1). The reader can begin acting before reaching the instruction for a file it already needed. | `cafleet-design-doc/create/create.md` — overlay (L5), `reference/base-dir.md` (Step 0, L49), `reference/supervision.md` (Step 1, L72) are three separated reads, none at a "before any action" gate. |
| F4 | **Passive overlay framing** | The overlay is introduced as "as you read the base, substitute your overlay's value for each `{placeholder}`" — an in-stream aside with no gate. Readers skip it and then emit unresolved `{placeholder}` tokens (e.g. spawn a monitor without the `{monitor_model}=haiku` value). | `cafleet/SKILL.md:31-33`; mirrored in `cafleet-design-doc/SKILL.md:19-21` and `cafleet-research/SKILL.md:20`. |

### What already works — build on it, do not reinvent

Three partial instances of the target pattern already exist and are effective where present; they are applied inconsistently and without the consequence column:

- **Numbered "Reading order"** in `cafleet/roles/director.md:7-13` ("Before spawning your first member, Read [`reference/director.md`]…") — a per-trigger gated list. The model for mechanism (ii).
- **"Load at Startup" blocks** in every `roles/*.md` (e.g. `cafleet-research/report/roles/manager.md:8`, "Load these skills at startup… also Read its `reference/base-dir.md`") — a per-reader load list, but soft-verbed and consequence-free.
- **The monitor `ready: monitor live` handshake** gating ordinary spawns (`cafleet/SKILL.md:48-52`) — proof that a gate-before-proceed works in this codebase.

This design generalizes those into one convention, adds the missing consequence column and the top-of-file gate, and applies it uniformly.

### Relationship to prior design docs

- **0000105** fixed *whether the agent enters the skill at all* (auto-invocation via the dispatcher `description`). 0000106 fixes *whether, once inside, the agent reads the skill's load-bearing linked files*. Complementary layers — 0000106 does not touch frontmatter `description` triggers.
- **0000103/0000104** established that members skip Director-only `reference/*` to save tokens. 0000106's per-reader-role scoping (Q5) preserves this exactly: the Required-reading block lists only the reader's own subset, making the existing separation explicit and enforced rather than implicit.
- Its validation method mirrors **0000105 Step 4**: a fresh-session behavioral probe, not an automated checker.

---

## Specification

### The Required reading convention

Every reader entry point gains one **Required reading** block as the first content after the title/frontmatter — before any actionable step. It fuses the two mechanisms this design adopts: **(i) classification** — every linked reference is labeled load-bearing or optional — and **(ii) a gated numbered load step** — the load-bearing reads are a "Read these, in order, before acting" list. The block has three sections: **Load-bearing** (Read eagerly, before any action), **Load-bearing on trigger** (Read at a named moment such as teardown — deferred, but mandatory then, and carrying its own consequence column), and **On-demand** (genuinely optional capabilities). The standalone overlay paragraph that currently sits near the top of each entry point is **removed**; its content becomes row #1 of the Load-bearing section — the content moves, it is not duplicated (removal-completeness).

Canonical shape (this exact structure is what every entry point adopts; the rows vary by reader):

```markdown
## Required reading

Before your first action other than these Reads, Read every file in the **Load-bearing**
table below, in order. Each carries a protocol you cannot reconstruct from this page.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay `reference/coding-agent/<name>.md` | every `{placeholder}` below stays unresolved — you emit literal `{monitor_model}` etc. |
| 2 | `reference/base-dir.md` | the no-bypass write protocol and `<unset>` contract — you mis-root every write |
| … | … | … |

**Load-bearing on trigger — Read at the named moment, before that action:**

| Read | Read before you… | What you lose if you skip it |
|------|------------------|------------------------------|
| `reference/recovery.md` | tear down a member or fleet | the first-out teardown order — you orphan panes / leak the fleet |
| `reference/exec-routing.md` | route a Bash-denied command to the Director | the dispatch shape — you stall or fabricate output |
| … | … | … |

**On-demand — Read only when you need that capability:**

| Read | When |
|------|------|
| `reference/cli.md` | you need a CLI subcommand beyond send/poll/ack |
| … | … |
```

### Design rules

1. **Per-reader-role scoping (Q5, hard constraint).** Each block lists only the load-bearing subset *for the reader of that file*. A member `roles/*.md` lists the member's reads (overlay, base-dir, its skill); it MUST NOT list Director-only governance (`supervision.md`, `director.md`, `recovery.md`, `broadcast.md`). A Director role/workflow lists the Director subset. This keeps the 0000103/0000104 token split intact — the classification documents the boundary instead of erasing it.
2. **Consequence column is mandatory and concrete.** Every row in **both** load-bearing tables (eager and on-trigger) states a specific failure (a wrong write path, an unresolved token, a dropped protocol), not a generic "you need this." The concrete cost is what defeats F2. The On-demand table omits the column — those reads are genuinely optional capabilities.
3. **Overlay is always row #1** in any entry point that uses `{placeholder}` tokens. It is the most universally needed and most-skipped read (F4); promoting it to a gated first step replaces the passive "as you read, substitute" aside.
4. **Affirmative framing (project rule).** The block prescribes the read positively ("Read X before acting; it carries Y"). The gate sentence is the single imperative; the rows are positive specs, not a wall of "do not."
5. **Two coordinated surfaces.** (A) Reader entry points — `roles/*.md`, workflow bodies, and the standalone/main-session dispatch path of each `SKILL.md` — get the full gated block. (B) Reference-file *menus* — the `SKILL.md` "Read on demand" lists that train optionality (F1) — are rewritten into the load-bearing-vs-optional classification so the menu itself stops signalling "all of these are skippable."
6. **Instruction-design only (Q3).** The gate's force is placement (top-of-file, pre-action), imperative framing, the consequence column, and the Step 6 behavioral probe as the acceptance test. No mise task, no runtime read-assertion. Efficacy is confirmed behaviorally, as in 0000105.
7. **Removal-completeness (project rule).** Where a load-bearing link is currently phrased as soft "Read on demand," convert it; do not leave the old undifferentiated phrasing as residue. After the change each entry point reads as if load-bearing content were never presented as optional.
8. **Deferred reads stay deferred.** A read that is load-bearing only at a trigger (teardown, broadcast, a Bash denial) belongs in the **Load-bearing on trigger** table, not the eager one. This preserves the existing per-trigger gates (e.g. `cafleet/roles/director.md:7-13` "Reading order") and the 0000103/0000104 lazy-load token savings — upgrade those gates in place by adding consequence cells; never flatten a deferred read into an eager one.

### Reader inventory and per-family application

Every entry point below gains a Required-reading block. The "Load-bearing for this reader" column is the authoritative scoping; anything not listed is on-demand or out-of-role.

#### Family: `cafleet`

| Entry point | Reader | Load-bearing for this reader |
|-------------|--------|------------------------------|
| `SKILL.md` (dispatch surface) | any agent loading the skill | overlay; `base-dir.md` *if it writes*; (Director extras live in the Director entry points, not here) |
| `roles/director.md` | Director | overlay, `reference/director.md`, `reference/supervision.md` (eager); `reference/recovery.md` (teardown), `reference/broadcast.md` (broadcasting) — load-bearing on trigger |
| `roles/member.md` | ordinary member | overlay; `base-dir.md` (eager); `reference/exec-routing.md` (load-bearing on trigger: a Bash denial) |
| `roles/monitor.md` | monitoring member | overlay; `reference/supervision.md` |
| `reference/supervision.md` "Read on demand" menu | Director | reclassify the menu (surface B) |

The `SKILL.md:18-27` "Read on demand" list is rewritten into the three-way classification: `base-dir.md` (eager load-bearing for any writer); the trigger reads `recovery.md` / `broadcast.md` / `exec-routing.md` (load-bearing on trigger, each with a consequence cell); and the genuinely optional `cli.md` / `output-flags.md`. `supervision.md` / `director.md` are load-bearing for Directors and point to the Director role block.

**Precedence (spawned member vs dispatch surface).** For a spawned member, its `roles/*.md` Required-reading block is authoritative; the `SKILL.md` dispatch-surface block governs only standalone/main-session readers that load the skill without a role file. A member follows its role block and does not separately re-execute the dispatch block — no double-read.

#### Family: `cafleet-design-doc`

| Entry point | Reader | Load-bearing for this reader |
|-------------|--------|------------------------------|
| `SKILL.md` dispatch | dispatcher / main session | overlay; the three reference pages classified (template/guidelines/coordination as on-demand-by-task) |
| `create/create.md` | create Director | overlay; `cafleet` `base-dir.md`; `cafleet` `supervision.md`; `reference/coordination.md` |
| `interview/interview.md` | interview Director | overlay; `cafleet` `base-dir.md`; `cafleet` `supervision.md`; `reference/coordination.md` |
| `execute/execute.md` | execute Director | overlay; `cafleet` `base-dir.md`; `cafleet` `supervision.md`; `reference/coordination.md` |
| `create/roles/{director,drafter,reviewer}.md` | each member | overlay; `cafleet` `base-dir.md`; `reference/coordination.md` |
| `interview/roles/analyzer.md` | analyzer | overlay; `cafleet` `base-dir.md`; `reference/coordination.md` |
| `execute/roles/{director,programmer,tester,verifier}.md` | each member | overlay; `cafleet` `base-dir.md`; `reference/coordination.md` |

#### Family: `cafleet-research`

| Entry point | Reader | Load-bearing for this reader |
|-------------|--------|------------------------------|
| `SKILL.md` dispatch | dispatcher / main session | overlay; `reference/visualization.md` + `reference/slidev.md` classified as on-demand utilities |
| `report/report.md` | report Director | overlay; `cafleet` `base-dir.md`; `cafleet` `supervision.md` |
| `presentation/presentation.md` | presentation Director | overlay; `cafleet` `base-dir.md`; `cafleet` `supervision.md` |
| `report/roles/{director,manager,scout,researcher}.md` | each member | overlay; `cafleet` `base-dir.md`; (scout/researcher add `web-researcher.md`) |
| `presentation/roles/{director,presentation,transcript,visual-reviewer}.md` | each member | overlay; `cafleet` `base-dir.md`; (presentation adds `reference/slidev.md` + `reference/visualization.md`) |
| `reference/slidev.md` technique menu | presentation member | reclassify the techniques table (surface B) |

`report/roles/web-researcher.md` is itself an embedded-agent spec read by Scout/Researcher; its load-bearing status is named in the Scout/Researcher blocks, consistent with the existing "Delegate every web-research turn… read its canonical spec… follow it" instruction.

### Out of scope

- Collapsing reference files inline (the architecture stays — issue direction).
- Any automated/runtime enforcement (Q3).
- Frontmatter `description` triggers (owned by 0000105).
- The human-facing `docs/reference/coding-agents/` operator docs (independent home; never cross-linked from skills).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation FIRST (project rule): Step 1 (docs/concepts, rules, README) lands before any skill-body edits in Steps 2–5.

### Step 1: Documentation

- [x] Update `docs/concepts/token-reduction.md` § *Skill-file split* row to note that the split's load-bearing links are protected by the per-reader-role Required-reading convention (the counterpart that keeps the split safe). <!-- completed: 2026-06-21T07:44 -->
- [x] Add a short subsection to `docs/concepts/token-reduction.md` (it is the safety-counterpart of the Skill-file split, so it lives beside it) defining the Required-reading convention: gated Load-bearing table + Load-bearing-on-trigger table + On-demand table, per-reader-role scoping. <!-- completed: 2026-06-21T07:44 -->

- [x] Update `.claude/rules/coding-agent-overlay.md` so the "read your overlay" base instruction is described as Required-reading row #1, not an in-stream aside. <!-- completed: 2026-06-21T07:52 --> (Director applied; Programmer's Edit tool is denied on the .claude/ tree.)

- [x] Check `README.md` for any description of skill structure / the path-by-reference design; update it if present (use `/update-readme` if the surface is large). <!-- completed: 2026-06-21T07:44 --> README describes only install/SDD usage and broker/CLI/monitoring architecture; it does not describe the path-by-reference / reference-file design, so no change is required.

### Step 2: Define the canonical block in the `cafleet` skill

- [x] Add the canonical **Required reading** block format to `cafleet/SKILL.md`, scoped to a skill-loading agent (overlay; base-dir if writing). <!-- completed: 2026-06-21T07:58 -->
- [x] Rewrite the `cafleet/SKILL.md:18-27` "Read on demand" menu into the three-way classification (surface B): `base-dir.md` eager load-bearing-for-writers; `recovery.md`/`broadcast.md`/`exec-routing.md` load-bearing-on-trigger (with consequence cells); `cli.md`/`output-flags.md` genuinely optional; `supervision.md`/`director.md` point to the Director role block. <!-- completed: 2026-06-21T07:58 -->
- [x] Move the overlay paragraph (`cafleet/SKILL.md:31-33`) into Required-reading row #1 with the unresolved-`{placeholder}` consequence — delete the standalone paragraph (content moves, not duplicated). <!-- completed: 2026-06-21T07:58 -->

### Step 3: Apply to `cafleet` role + reference entry points

- [x] Add the Required-reading block to `cafleet/roles/director.md`: upgrade the existing "Reading order" per-trigger gates into the **Load-bearing on trigger** sub-table (add consequence cells), keeping their deferred nature — do not force them eager. <!-- completed: 2026-06-21T08:05 -->
- [x] Add the Required-reading block to `cafleet/roles/member.md` (overlay, base-dir eager; exec-routing as load-bearing on trigger — a Bash denial). <!-- completed: 2026-06-21T08:05 -->
- [x] Add the Required-reading block to `cafleet/roles/monitor.md` (overlay, supervision). <!-- completed: 2026-06-21T08:05 -->
- [x] Reclassify the on-demand menus inside `cafleet/reference/supervision.md` (surface B). <!-- completed: 2026-06-21T08:05 -->

### Step 4: Apply to `cafleet-design-doc`

- [ ] Reclassify the `cafleet-design-doc/SKILL.md` reference-page menu (template/guidelines/coordination) and promote the overlay to Required-reading. <!-- completed: -->
- [ ] Add the Required-reading block to `create/create.md`, `interview/interview.md`, `execute/execute.md`. <!-- completed: -->
- [ ] Add the Required-reading block to every `roles/*.md` under `create/`, `interview/`, `execute/`. <!-- completed: -->

### Step 5: Apply to `cafleet-research`

- [ ] Reclassify the `cafleet-research/SKILL.md` reference-page menu (visualization/slidev as on-demand utilities) and promote the overlay to Required-reading. <!-- completed: -->
- [ ] Add the Required-reading block to `report/report.md` and `presentation/presentation.md`. <!-- completed: -->
- [ ] Add the Required-reading block to every `roles/*.md` under `report/` and `presentation/` (scout/researcher include `web-researcher.md`; presentation includes `slidev.md` + `visualization.md`). <!-- completed: -->
- [ ] Reclassify the techniques menu in `reference/slidev.md` (surface B). <!-- completed: -->

### Step 6: Behavioral validation (fresh-session probes)

For each probe: open a fresh coding-agent session, give the triggering prompt, and confirm the reader applies a protocol knowable only from a linked file. Pass = protocol applied; fail = generic/wrong behavior.

- [ ] **cafleet / base-dir:** spawn a member with no `BASE:` line; confirm it emits the parens-free anchorless status `audit-disabled no BASE in spawn prompt` (knowable only from `base-dir.md` § No-bypass write protocol) rather than falling back to `/tmp`. <!-- completed: -->
- [ ] **cafleet / overlay:** confirm a Director spawns the monitor with `--model haiku` (the `{monitor_model}` value lives only in `reference/coding-agent/claude.md`). <!-- completed: -->
- [ ] **cafleet-design-doc / coordination:** confirm a Drafter/Reviewer uses the verb + pointer `COMMENT(role)` schema from `reference/coordination.md` rather than free-form bodies. <!-- completed: -->
- [ ] **cafleet-research / utility:** confirm a Presentation member applies a `reference/slidev.md` Layouts-table choice or a `reference/visualization.md` color rule it could only know from those pages. <!-- completed: -->
- [ ] Record outcomes in this doc; if any probe fails, strengthen that entry point's block (placement/wording) and re-run before marking complete. <!-- completed: -->

### Step 7: Removal sweep & consistency check

- [ ] Grep the three skill families for residual undifferentiated "Read on demand" phrasing on load-bearing links; confirm each is either reclassified or genuinely on-demand. <!-- completed: -->
- [ ] Confirm no member entry point lists a Director-only reference page (per-reader-role hard constraint); confirm the 0000103/0000104 token split is intact. <!-- completed: -->
- [ ] Verify every Required-reading link resolves to an existing file/section (no dangling pointers introduced). <!-- completed: -->
- [ ] Run `mise //cafleet:lint` and `mise //cafleet:test` to confirm no skill-loading test or doc check regressed. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-21 | Initial draft |
