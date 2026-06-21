# Restore Workflow Auto-Invocation for the CAFleet Design-Doc and Research Skills

**Status**: Approved
**Progress**: 4/11 tasks complete
**Last Updated**: 2026-06-21

## Overview

The `cafleet-design-doc` and `cafleet-research` umbrella skills no longer reliably auto-invoke: a coding agent reading them is not compelled to (a) enter the matching team workflow when the user asks for the matching task, nor (b) run the full CAFleet team orchestration once entered. This design rewrites the two umbrella `SKILL.md` entry points so an agent reliably invokes the matching workflow and executes its full team — **without the user having to say "use cafleet"** — restoring the trigger strength the pre-collapse constellation skills had, while keeping the consolidated two-skill structure and the current monitoring-member mechanism intact.

## Success Criteria

- [ ] Each umbrella `SKILL.md` frontmatter `description` leads with crisp, scenario-keyed triggers phrased in the user's own likely wording, so the coding agent auto-invokes the skill without an explicit "use cafleet" instruction.
- [ ] **Behavioral validation passes**: in a fresh coding-agent session, a triggering prompt that does NOT mention cafleet (e.g. "create a design doc for X", "research topic Y") causes the agent to (a) auto-invoke the matching skill and (b) route into the workflow such that the dedicated monitoring member is spawned first-in. This behavioral outcome — not a structural re-read — is the gate for marking the work complete.
- [ ] Each `SKILL.md` body presents an **imperative, scenario-linked dispatch** that names, for each scenario, the affirmative action (invoke this skill and run the matching workflow as a full CAFleet team) and the link to the workflow body.
- [ ] Both `SKILL.md` files state affirmatively that routing into a workflow means executing its **entire** orchestration: the dedicated monitoring member spawned first-in, then the role team, then the message-broker quality loop — so the "full orchestration once entered" half of the bug is addressed at the dispatch point.
- [ ] All trigger and dispatch language is **affirmative-only** (positive spec, no named "Do NOT write directly / Do NOT use EnterPlanMode / Do NOT web-search-and-summarize" prohibitions), per `affirmative-writing.md`.
- [ ] The base instructions stay backend-neutral: the coding-agent overlay note and the "teammates load this skill by its name … via their backend's skill-loader" note are retained unchanged in intent, per `.claude/rules/coding-agent-overlay.md`.
- [ ] The fix surface is the **two `SKILL.md` files only** — no workflow body (`create`/`execute`/`interview`/`report`/`presentation`), no role file, and no `reference/*` page is modified.
- [ ] No documentation drift: `README.md`, `docs/get-started/*`, and `docs/how-to/design-doc-development.md` continue to describe the skills accurately after the rewrite; a doc is edited only if a trigger statement in it actually drifts.

## Background

### Current state

The two umbrella skills are thin dispatchers. Each `SKILL.md` consists of an intro paragraph, a coding-agent-overlay note, a "teammates load by name" note, and a passive **"## When to use"** routing table that maps "You want to… → Go to <link>". The detailed workflow bodies (`create/create.md`, `execute/execute.md`, `interview/interview.md`, `report/report.md`, `presentation/presentation.md`) and the role files are correct and complete — including the current, correct orchestration mechanism in which the **first** `cafleet member create` is the dedicated monitoring member (`--role monitor`, running `cafleet monitor start`), which gates the rest of the team behind its `ready: monitor live` handshake.

### How the regression was introduced

At reference commit `9eed5bb` the design-doc and research skills were a **constellation** of separate, single-purpose auto-triggering skills, each with a crisp imperative `description` that named both the trigger and the desired behavior. Verified examples:

- `cafleet-design-doc-create`: *"Create a new design document using CAFleet-native orchestration (Director/Drafter/Reviewer team). Use when the user asks to create a design doc, design document, specification, or technical spec. Do NOT write the doc directly with Write or use EnterPlanMode — always invoke this skill."*
- `cafleet-design-doc-execute`: *"Implement features based on a design document… Use when the user asks to implement or execute a design document. … Do NOT implement a design document by reading it and coding manually — always invoke this skill instead."*
- `cafleet-design-doc-interview`: *"Validate an existing design document through fine-grained multi-round Q&A… Use after the cafleet-design-doc-create skill and before the cafleet-design-doc-execute skill. … Do NOT use this to create or execute design documents — use the dedicated skills instead."*
- `cafleet-research-report`: *"Create a comprehensive research report… Do NOT do a quick web search and summarize — invoke this skill for thorough, multi-source research."*
- `cafleet-research-presentation`: *"Create a Slidev presentation and reading transcript from an existing research report folder… Do NOT use for research — use the cafleet-research-report skill for that."*

Designs 0000103 and 0000104 collapsed that constellation into the current two umbrella dispatchers. The collapse was intentional and is **not** being reverted; it removed skill-list clutter and centralized routing. The unintended side effect is that the per-workflow imperative trigger language no longer lives where the agent first reads it. The umbrella `description` now spans three (design-doc) or four (research) scenarios at once, and the entry-point body is a passive routing table. The result, observed in practice: agents write design docs or do research freeform, with no monitoring member and no team.

### What is in scope vs. not

- **In scope**: the entry-point trigger and dispatch language in `skills/cafleet-design-doc/SKILL.md` and `skills/cafleet-research/SKILL.md`.
- **Not in scope** (confirmed with the user): re-expanding to a constellation; modifying any workflow body, role file, or `reference/*` page; the monitoring mechanism (current and correct); and any new `docs/concepts` page.

## Specification

### Design decisions (confirmed with the user)

1. **Keep the two umbrella skills; strengthen them in place.** Rewrite each frontmatter `description` and restructure each `SKILL.md` body so routing is unambiguous and scenario-linked.
2. **Fix surface = the two `SKILL.md` files only.** Workflow bodies, role files, and `reference/*` pages are untouched. Because the bodies are off-limits, the "execute the full team once entered" imperative is carried at the **dispatch point in `SKILL.md`** (it states that routing into a workflow means running its entire orchestration), rather than by editing the body openings.
3. **Affirmative-only language.** State the positive spec — *always invoke this skill and route into the matching workflow; routing runs the full CAFleet team*. Do not add named-behavior prohibitions ("Do NOT write directly", "Do NOT use EnterPlanMode", "Do NOT web-search-and-summarize"). The existing residual prohibition phrasings in both files (e.g. *"Do NOT write or implement design docs freeform — route through this skill's workflows"*) are **converted to their affirmative equivalents** and the prohibition phrasing is removed entirely (no deprecation residue, per `removal.md`).

   **Why affirmative-only, not the rule-endorsed paired form.** `affirmative-writing.md` does not ban prohibitions; it explicitly blesses the *paired* form for a genuine hard constraint (*"strongest paired with the affirmative instruction it enforces — 'always use X' alongside 'never use Y'"*), and the empirically-working constellation descriptions used exactly that paired form (*"Do NOT write the doc directly … — always invoke this skill"*). The user nonetheless directed affirmative-only here, so the paired form is set aside by explicit instruction, not because the rule forbids it. The trade-off this records: dropping the rule-blessed prohibition half removes language that demonstrably triggered, so the affirmative half must be made strong and unambiguous enough to carry the trigger on its own (the trigger-leading description of decision below, plus the imperative dispatch and the behavioral-validation gate, are how this design compensates). A future reader should read "affirmative-only" as a deliberate, user-confirmed choice — not as a claim that `affirmative-writing.md` bans prohibitions.
4. **Backend neutrality preserved.** The trigger and dispatch language is backend-neutral; the coding-agent-overlay note and the teammates-load-by-name note remain. Backend specifics continue to live only in `reference/coding-agent/<name>.md`.
5. **Minimal docs.** No new `docs/concepts` page. A doc outside the two skills is edited only if a trigger statement in it actually drifts after the rewrite (audit confirms whether any edit is required).

### Target frontmatter `description` — `cafleet-design-doc`

The new `description` **leads with the trigger phrasings** (in the user's likely wording), keyed to each scenario, then states the affirmative directive as a short tag. Per the model the working constellation descriptions used (lead with action+object), the first words after "Use when" are trigger keywords. Orchestration mechanics (monitoring member, role team, quality loop) are deliberately kept OUT of the description and carried only by the body dispatch section (Body structure item 2 below) — the description's job is skill selection, and adding orchestration detail there both lengthens the trigger surface and duplicates the body. Proposed text (the execute team may refine wording within these constraints):

> Use when the user asks to create a design doc, design document, specification, or technical spec (create workflow → Director/Drafter/Reviewer team); to validate, review, or interview an existing design doc through multi-round Q&A (interview workflow); or to implement or execute a design doc (execute workflow → TDD team). Also the standardized format spec — consult the template and guidelines when editing a design doc. Always invoke this skill and route into the matching workflow, orchestrated as a CAFleet team. Teammates in agent teams load this skill by its name cafleet-design-doc via their backend's skill-loader.

### Target frontmatter `description` — `cafleet-research`

Same construction: triggers lead, the "always invoke / CAFleet team" framing follows as a short tag, and orchestration mechanics live only in the body dispatch.

> Use when the user asks to research a topic or create a multi-source research report (report workflow → Director/Manager/Researcher team writing to researches/<topic>/); to build a presentation, slide deck, or reading transcript from a report (presentation workflow); to create a chart, plot, graph, figure, or data visualization (visualization reference); or to author slides with the custom Slidev theme (slidev reference). Always invoke this skill and route into the matching workflow, orchestrated as a CAFleet team. Members in agent teams load this skill by its name cafleet-research via their backend's skill-loader.

### Target `SKILL.md` body structure (both files)

Replace the passive **"## When to use"** table with an imperative, scenario-linked **dispatch** section. The section MUST:

1. Open with an affirmative directive: when the user's request matches a scenario below, invoke this skill and run the linked workflow as a full CAFleet team — proactively, without waiting for the user to say "use cafleet".
2. State that **routing into a workflow means executing its entire orchestration**: the monitoring member is spawned first-in, then the role team, then the quality loop iterates to approval (the linked body is the authoritative procedure).
3. Present the scenarios as a table whose rows pair the **user-phrasing trigger** with the **affirmative action + link** (the team workflow), kept separate from a secondary "consult, no team" table for the format/utility reference pages.

Proposed dispatch table for `cafleet-design-doc` (team workflows):

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Create a design doc / specification / technical spec | the **create** workflow ([create/create.md](create/create.md)) — Director/Drafter/Reviewer team |
| Validate / review / interview an existing design doc | the **interview** workflow ([interview/interview.md](interview/interview.md)) — multi-round Q&A |
| Implement / execute a design doc | the **execute** workflow ([execute/execute.md](execute/execute.md)) — TDD team |

Secondary "consult (no team)" table for `cafleet-design-doc`: the template, guidelines, and coordination reference pages (unchanged links).

Proposed dispatch table for `cafleet-research` (team workflows + utility references):

| When the user wants to… | Invoke this skill and run |
|:--|:--|
| Research a topic / create a multi-source research report | the **report** workflow ([report/report.md](report/report.md)) — Director/Manager/Researcher team → `researches/<topic>/` |
| Build a presentation / slide deck / reading transcript from a report | the **presentation** workflow ([presentation/presentation.md](presentation/presentation.md)) |
| Create a chart / plot / graph / figure or visualize data | the **visualization** reference ([reference/visualization.md](reference/visualization.md)) |
| Author slides with the custom Slidev theme | the **slidev** reference ([reference/slidev.md](reference/slidev.md)) |

**Preserve the research chaining context.** The current `cafleet-research` body carries an informational sentence that is routing context, not a prohibition: *"The report workflow chains into the presentation workflow after user approval. The two reference pages are standalone utilities the presentation workflow also reads."* The restructure MUST preserve this (folded in immediately under the `cafleet-research` dispatch tables) — it tells the agent the two workflows chain and that the reference pages are shared utilities. The `cafleet-design-doc` body has no equivalent sentence, so this applies to the research file only.

The intro paragraph, the coding-agent-overlay note, and the teammates-load-by-name note are retained (the latter two backend-neutral, per the overlay rule). The closing residual-prohibition sentence in each file is converted to an affirmative restatement (e.g. *"Always route design-doc work through the workflow bodies above — each runs the full CAFleet team."*) with the prohibition phrasing removed.

### Why this addresses both halves of the bug

- **Entering the workflow**: the frontmatter `description` now leads with the user's likely phrasings keyed to each scenario, restoring the auto-trigger strength of the constellation while keeping one skill per family.
- **Full orchestration once entered**: the dispatch section affirmatively equates "routing into a workflow" with "executing its full team orchestration (monitoring member first-in, role team, quality loop)", so an agent that reads the entry point understands that invoking is not freeform writing/researching but running the team defined in the linked body.

### Constraints honored

- `affirmative-writing.md`: affirmative-only positive spec; no reactive prohibition residue.
- `removal.md`: residual "Do NOT … freeform" phrasings are removed (not deprecated in place) as they are replaced by the affirmative spec.
- `.claude/rules/coding-agent-overlay.md`: base stays backend-neutral; overlay note retained; no backend specifics added to the base.
- `design-doc-numbering.md`: `SKILL.md` files are first-class documentation targets and are the documentation updated in this cycle; README/docs are audited for drift in the same cycle.

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Per `design-doc-numbering.md`, documentation is updated first. Here the two `SKILL.md` files ARE the documentation-and-artifact target; README/docs are audited for drift in the same cycle.

### Step 1: Rewrite `cafleet-design-doc/SKILL.md`

**Frontmatter preservation invariant (both steps).** Replace ONLY the `description` value; preserve every other frontmatter key verbatim. In the on-disk source files the only other keys are `name` and `allowed-tools` (no `metadata`/`local-path` block is present), but the invariant is stated generally so that any key present at implementation time — including one injected by packaging — survives the description swap untouched.

- [x] Replace the frontmatter `description` with the affirmative, scenario-keyed, trigger-leading text per the Specification, applying the frontmatter preservation invariant above. <!-- completed: 2026-06-21T05:32 -->
- [x] Replace the passive "## When to use" table with the imperative scenario-linked dispatch section: opening affirmative directive, the "routing = full team orchestration (monitoring member first-in, role team, quality loop)" statement, the team-workflow table, and the secondary consult-only references table. <!-- completed: 2026-06-21T05:32 -->
- [x] Convert the residual "Do NOT … freeform — route through this skill's workflows" phrasing (frontmatter and body) to its affirmative equivalent and remove the prohibition phrasing. <!-- completed: 2026-06-21T05:32 -->
- [x] Confirm the coding-agent-overlay note and the teammates-load-by-name note are retained and unchanged in intent. <!-- completed: 2026-06-21T05:32 -->

### Step 2: Rewrite `cafleet-research/SKILL.md`

- [ ] Replace the frontmatter `description` with the affirmative, scenario-keyed, trigger-leading text per the Specification, applying the frontmatter preservation invariant from Step 1. <!-- completed: -->
- [ ] Replace the passive "## When to use" table with the imperative scenario-linked dispatch section (team workflows table + utility references), including the "routing = full team orchestration" statement for the report and presentation workflows. <!-- completed: -->
- [ ] Fold the preserved research chaining-context sentence in under the dispatch tables (per Specification § Preserve the research chaining context). <!-- completed: -->
- [ ] Convert the residual "Do NOT do a quick web search and summarize — route through the workflows above" phrasing to its affirmative equivalent and remove the prohibition phrasing. <!-- completed: -->
- [ ] Confirm the coding-agent-overlay note and the members-load-by-name note are retained and unchanged in intent. <!-- completed: -->

### Step 3: Drift audit (docs)

- [ ] Re-read both rewritten `SKILL.md` files and confirm: triggers lead the description, language is affirmative-only, dispatch is scenario-linked, and the full-team statement is present in the body. Then audit `README.md`, `docs/get-started/*`, and `docs/how-to/design-doc-development.md` for trigger-statement drift; edit a doc only if it actually drifted (expectation: no edits needed, since those surfaces reference the skills by name and by workflow). <!-- completed: -->

### Step 4: Behavioral validation (gate for completion)

- [ ] In a fresh coding-agent session, issue a triggering prompt for each skill that does NOT mention cafleet — e.g. "create a design doc for <X>" and "research <Y>" — and confirm the agent (a) auto-invokes the matching skill and (b) routes into the workflow such that the dedicated monitoring member is the first `cafleet member create`. Record the outcome. This behavioral pass — not the Step 3 structural re-read — is what authorizes marking the design complete. <!-- completed: -->

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-21 | Initial draft |
| 2026-06-21 | Reviewer round 1: triggers lead both descriptions (orchestration mechanics moved to body only); added behavioral-validation gate (Step 4); recorded rationale for affirmative-only vs. the rule-endorsed paired form; preserved the research chaining-context sentence; stated the frontmatter preservation invariant (corrected — no `metadata` block in the on-disk source). |
