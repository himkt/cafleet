# Backend Evaluation Requirements by Role

**Status**: Approved
**Progress**: 17/17 tasks complete
**Last Updated**: 2026-10-31

## Overview

Turn the shipped § Role requirements into the written bar a maintainer evaluates a coding-agent backend against: the Director row stays, and the Member row becomes an Eligible tier plus a Bundled support tier whose native evidence is named per capability. Record every backend's verdict — Director eligible `yes`/`no` and Member eligible `yes`/`no`, each with checked native version, source and date — in a new required Role eligibility subsection and in `SPEC.md`, making OpenCode's Director status (`no`) explicit. Document how a maintainer applies the bar to a candidate and records the result (GitHub issue #504; documentation-only).

## Success Criteria

- [ ] `SPEC.md` §6.7 Role requirements and `skills/cafleet/reference/coding-agents.md` § Role requirements contain the final texts in § The bar, with every shipped sentence handled as § Shipped sentence dispositions states.
- [ ] `SPEC.md` §6.7 has a `#### Role eligibility` table with one Director row and one Member row per bundled backend, each `yes`/`no` with version, source and date; OpenCode's Director row is `no`.
- [ ] Every backend section in `coding-agents.md` and its `## Template` carry a fifth `### Role eligibility` subsection, and `.claude/rules/coding-agent-overlay.md` specifies five subsections.
- [ ] The OpenCode conditional manual-reception note is replaced by its explicit Director verdict, and the check that would flip it is written beneath its Role eligibility table.
- [ ] `docs/docs/contributing.md` § Extend a coding-agent backend contains the evaluation procedure, the capability → evidence table and the candidate design-doc `## Role evaluation` recording rule.
- [ ] Director backend selection (`skills/cafleet/roles/director.md`, `skills/cafleet/SKILL.md`, the coding-agents selector table and `docs/docs/concepts/coding-agents.md`) checks the recorded verdicts, and a session whose backend's Director verdict is `no` reports it, names the eligible backends and runs no `fleet create`.
- [ ] The diff is confined to the Markdown files listed in § Placement, `mise //docs:build` passes and every new or changed link/anchor resolves.

---

## Background

Design 0000194 shipped a § Role requirements table (commit `bbd7a1c2`) in `skills/cafleet/reference/coding-agents.md` and in the operator reference, which now lives in `SPEC.md` §6.7 (the `docs/docs/spec/coding-agent-backends.md` page named in the issue no longer exists; `docs/` links to `SPEC.md#role-requirements`). It replaced design 0000193's Member requirement ("auto or dontAsk mode") with "runner-compatible PTY lifecycle and an explicit detection-coverage declaration". Issue #502 then shipped as design 0000197: the omitted Member policy is each backend's native automatic review (Claude `auto`, Codex `--approve-for-me`), and OpenCode's omitted policy is the fixed allow/deny `cafleet` preset.

Three gaps remain:

| Gap | Current state |
|---|---|
| Permission posture | The Member bar no longer requires a non-interactive omitted policy, so an interactive-only backend would pass while blocking on a native form for every unlisted command. |
| Evidence | Bundled-profile evidence rules exist in `SPEC.md` §6.9 Native acceptance and coverage and in prose in `docs/docs/contributing.md`, but the bar does not name which evidence each capability needs. |
| Verdicts | No document records whether a backend meets each role. Claude and Codex carry goal evidence and a native reception recipe; OpenCode has neither, only the conditional sentence "A host satisfying the Director requirements without a verified native stream adapter uses explicit manual `poll`/finite `wait`". |

---

## Specification

### The bar

The Director row is unchanged; the Member row splits into two tiers. The version probe belongs to Bundled support because only profile selection depends on it: `SPEC.md` §6.9 Native version probe keeps its failed-probe behavior (`coverage=unknown`, creation continues, manual terminal operation and delivery fallback), which is exactly the `unknown`-coverage path that stays Member-eligible. The non-interactive omitted policy restores 0000193's intent in its post-#502 form.

Both homes use this table:

| Role | Tier | Required backend capability |
|---|---|---|
| Director | Eligible | Native goal lifecycle controls, such as a goal command, and background execution while supervision continues. Both capabilities are required. |
| Member | Eligible | A runner-compatible PTY lifecycle and an explicit detection-coverage declaration. A launch-policy catalog whose omitted policy is non-interactive: native automatic review where the backend offers one, otherwise a fixed allow/deny policy that runs the spawn-allowed broker commands. |
| Member | Bundled support | A native version the version probe parses, selecting a bundled profile that declares detection, automatic review annotations, composer plans and request-bound answers independently from native evidence. A failed probe or an undeclared capability is `unknown` coverage or unsupported, and the backend stays Member-eligible. |

#### Final text: `skills/cafleet/reference/coding-agents.md` § Role requirements

The heading and the table above, followed by exactly these paragraphs:

```markdown
Evaluate roles independently; a backend ships for the roles it meets. A verdict is `yes` only when recorded evidence establishes every Eligible-tier requirement for that role; otherwise it is `no`, with "not established" or the observed failure as its evidence. Bundled-support capabilities never change a verdict. Each requirement group's evidence holds for its own recorded native version, and a newer installed version carries the verdict forward until the next re-check. Each backend section's Role eligibility records its verdicts.

Interactive native policies remain explicit `--permission-mode` choices on an eligible backend; an interactive-only backend is not Member-eligible. Preserve native guards, managed policy, explicit denies and sandbox boundaries. Unknown detection/composer coverage permits manual native terminal work; ordinary CAFleet input then uses Escape-first delivery fallback, and semantic permission answers stay unavailable. The [member runtime procedure](member-runtime.md) owns event, input and continuation handling.

Goal support is a host capability; retain supplied activation, pause, completion and budget authority. Evaluate the decision surface and native reception separately from Director eligibility: a backend without a structured decision surface uses its documented default, and a host satisfying both Director prerequisites can use explicit manual `poll` or finite `wait` when no verified receive adapter is available. Background execution alone establishes no unsolicited wake or per-record reception guarantee.
```

#### Final text: `SPEC.md` §6.7 `#### Role requirements`

The heading and the table above, followed by exactly these paragraphs (the third is the shipped catalog paragraph, unchanged):

```markdown
Evaluate each role independently; a backend ships for the roles it meets. A verdict is `yes` only when recorded evidence establishes every Eligible-tier requirement for that role, and otherwise `no` with "not established" or the observed failure as its evidence; bundled-support capabilities never change a verdict. Each requirement group's evidence holds for its own recorded native version, and a newer installed version carries the verdict forward until the next re-check, which follows any change to the backend's bundled profile boundary or recorded evidence. § Role eligibility records each backend's verdicts.

Interactive native policies remain explicit `--permission-mode` choices under §6.9 Launch policy and capability registry; an interactive-only backend is not Member-eligible. Preserve native guards, managed policy, explicit denies, sandbox boundaries and the host's activation, pause, completion and budget authority. A backend with unknown detector/composer coverage permits manual terminal work and delivers ordinary generated input through delivery fallback. The decision surface and native receive support are evaluated independently of Director eligibility: a Director meeting both requirements may use manual `poll` or finite `wait` when its native adapter is unavailable, and background execution alone establishes no unsolicited wake or per-record reception guarantee.

The bundled backend policy catalog owns launch-mode choices and defaults, exact argv mapping, version eligibility and detection/composer profiles. The omitted native argv is specified below; an explicit native mode changes only its permission argument and preserves the remaining argv and literal assignment.
```

#### Shipped sentence dispositions

| Shipped sentence | Home | Disposition |
|---|---|---|
| "Evaluate roles independently." / "Evaluate each role independently." | Both | Retained, extended with "a backend ships for the roles it meets". |
| "A backend's native automatic review is its omitted Member policy where the backend offers one; interactive and fixed allow/deny policies remain explicit choices." | Both | Replaced by the Member Eligible row and the "Interactive native policies remain explicit `--permission-mode` choices" sentence, because fixed allow/deny is OpenCode's omitted policy. |
| "Preserve native guards, managed policy, explicit denies and sandbox boundaries" (SPEC adds the host's goal authority) | Both | Retained verbatim. |
| Unknown-coverage sentence (manual terminal work, delivery fallback; skill adds unavailable semantic answers) | Both | Retained verbatim; the skill keeps its member-runtime link sentence. |
| "Goal support is a host capability; retain supplied activation, pause, completion and budget authority." | Skill | Retained verbatim. |
| Native reception sentence (manual `poll` / finite `wait`) and "Background execution alone establishes no unsolicited wake…" | Both | Retained, with the decision surface added as a separately evaluated capability. |
| "The bundled backend policy catalog owns launch-mode choices…" paragraph | `SPEC.md` | Retained verbatim. |

### Verdict record

Each backend's verdict appears in two homes with identical cell text:

| Home | Shape |
|---|---|
| `skills/cafleet/reference/coding-agents.md`, every backend section | `### Role eligibility` with a `Role \| Eligible \| Evidence` table: one Director row, one Member row |
| `SPEC.md` §6.7, `#### Role eligibility` after the Role requirements catalog paragraph | One `Backend \| Role \| Eligible \| Evidence` table, two rows per bundled backend |

`Eligible` is exactly `yes` or `no` in a code span. The Evidence cell holds at most two sentences. Each names, per requirement group, the source, the native version it was evidenced on and `checked YYYY-MM-DD`; a Member row's second sentence names its bundled profile as the Bundled support pointer instead of copying the catalog's capability list.

A verdict holds per requirement group: each group's evidence applies to its own recorded native version, so one backend's groups may cite different versions. A newer installed version carries the verdict forward until the next re-check (§ Ownership and re-check). "Checked" is the date the evidence was established; for existing evidence without a recorded probe date, it is the date of the commit that recorded it.

Exact rows for the bundled backends:

| Backend | Role | Eligible | Evidence |
|---|---|---|---|
| `claude` | Director | `yes` | Goal lifecycle: [Claude Code goal](https://code.claude.com/docs/en/goal), 2.1.296, checked 2026-10-22. Background execution: Monitor stream probes, 2.1.301, checked 2026-10-25. |
| `claude` | Member | `yes` | PTY lifecycle, coverage declaration and omitted `auto` automatic review: live runner acceptance on tmux and herdr (omitted-default launch, auto-approved and auto-refused cases), 2.1.302, checked 2026-10-29. Bundled support: profile `claude-2.1.302-auto-1`. |
| `codex` | Director | `yes` | Goal lifecycle: [Codex goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), 0.162.1, checked 2026-10-22. Background execution: retained `exec_command` session stream probes, 0.162.1, checked 2026-10-25. |
| `codex` | Member | `yes` | PTY lifecycle, coverage declaration and omitted `--approve-for-me` automatic review: live runner acceptance on tmux and herdr (omitted-default launch and auto-refused cases, host-limited cases recorded as limits), 0.163.0, checked 2026-10-29. Bundled support: profile `codex-0.163.0-approve-for-me-1`. |
| `opencode` | Director | `no` | Goal lifecycle controls: not established for 2.1.4, with no goal evidence recorded, checked 2026-10-31. Background execution while supervision continues: not established for 2.1.4, checked 2026-10-31. |
| `opencode` | Member | `yes` | PTY lifecycle, coverage declaration and the omitted `cafleet` preset, which resolves every shell check to allow or deny: live runner acceptance on herdr only (`coverage=tested`, no native form, native denial itself not exercised), 2.1.4, checked 2026-10-29. Bundled support: profile `opencode-2.1.4-cafleet-ask-1`. |

Sources of these values, all existing records (no live probe runs in this design):

| Row group | Source |
|---|---|
| Director goal lifecycle | The `Evidence:` lines of the `claude` and `codex` backend sections |
| Director background execution | The native reception recipes' `Evidence:` lines, recorded in commit `7404be90` (2026-10-25) |
| `claude` / `codex` Member | Design 0000197 Step 7 live acceptance on tmux and herdr (completed 2026-10-29), including its recorded Codex host limits |
| `opencode` Member | Design 0000196 § Acceptance "Default policy coverage" row (herdr, 2026-10-29) |
| Bundled support pointers | Profile entries added in commits `394d99dc` (2026-10-29) and `5b874953` (2026-10-28) |

The herdr-only OpenCode default-policy evidence is judged sufficient to establish the Member Eligible tier: the verdict is `yes`, and its herdr-only scope and the unexercised native denial are recorded in the Evidence cell as stated limits.

Beneath the OpenCode Role eligibility table, add this flip check as prose:

> To change the Director verdict, a maintainer establishes, for an installed OpenCode version, native goal lifecycle controls, such as a goal command, from official documentation or installed help, and background execution while supervision continues from a CAFleet stream probe. Record the version, source and date in this table, then add a native reception recipe or state the manual `poll` / finite `wait` reception limit.

In `SPEC.md` the equivalent sentence records the result "in this table and in the installed CAFleet skill's OpenCode backend section"; the skill copy names no `SPEC.md` target, because skills carry their own facts.

### Ownership and re-check

Runtime documentation maintenance owns Role eligibility, as it owns Runtime bindings and bound notes; the `cafleet-model-list-refresh` skill preserves it. Re-check a backend's verdict whenever its bundled profile boundary (a new or changed profile entry) or its `Evidence:` line changes. There is no fixed cadence.

### Applying the bar to a candidate

`docs/docs/contributing.md` § Extend a coding-agent backend gains this numbered procedure (maintainer audience):

1. Open a design doc for the candidate. It carries a `## Role evaluation` section using the `Role | Eligible | Evidence` table shape of § Verdict record.
2. For each role, check every Eligible-tier requirement in [SPEC §6.7 Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) against an installed native version, recording the version per requirement group: official documentation and installed `--help` / `--version` for host capabilities, and CAFleet PTY probes on each registered multiplexer for runtime behavior.
3. Derive each verdict: `yes` only when every requirement is established; otherwise `no` with "not established" or the observed failure.
4. For a Member-eligible candidate, choose the bundled-support capabilities to claim and collect the evidence in the table below; an unclaimed capability ships as `unknown` coverage.
5. When the backend ships, copy the verdicts into its `coding-agents.md` Role eligibility subsection and the `SPEC.md` Role eligibility table, and ship only the roles marked `yes`.

The capability → evidence table (new, converting the evidence prose currently in that section; exact fields stay owned by SPEC §6.9):

| Capability | Required native evidence |
|---|---|
| Permission detection | Live captures of each permission form the claimed policies reach, plus positive and adversarial replay fixtures for redraw, wrapping, fragmented ANSI/UTF-8, alternate screens, competing forms and quoted transcript text. |
| Automatic review annotations | Captures of each automatic refusal and unavailable-review notice in the idle, working and overlay states where the native host shows them. |
| Composer plans | For prompt and native exec each, empty-composer, typed-literal and post-submit captures with separate literal and submit-key stages. |
| Request-bound answers | Allow and deny pre-stage captures and a continuation capture for each decision. Pin single-request semantics, focus/key stages, request-token revalidation and human/output race behavior. |

Prose beneath the table keeps the existing requirements: every capture is real native PTY evidence on a native version within the profile's interval on each registered multiplexer; [SPEC §6.9 Native acceptance and coverage](https://github.com/himkt/cafleet/blob/main/SPEC.md#native-acceptance-and-coverage) owns the minimum policies, evidence fields and fault cases; a launch-help probe supplies no detector/composer/answer claim.

### Director backend selection

| Surface | Change |
|---|---|
| `skills/cafleet/roles/director.md` intro (line 7) | Replace with the two sentences quoted below this table. |
| `skills/cafleet/roles/director.md` § Model selection (line 52) | Append: "Confirm the selected member backend's Role eligibility records Member eligible `yes`; for a `no` verdict, relay an operator choice via {decision_surface} instead of spawning." |
| `skills/cafleet/SKILL.md` § Resolve your overlay step 1 | Add "confirm its Role eligibility Member verdict" to the spawn-backend reads. |
| `coding-agents.md` selector table, "Select/configure a spawned member" row | Read "Its Model catalog, Role defaults and Role eligibility". |
| `docs/docs/concepts/coding-agents.md`, after the `--coding-agent` bullets | New paragraph quoted below this table. |

`director.md` intro replacement:

> Use a backend whose Role eligibility records Director eligible `yes` under the [Director requirements](../reference/coding-agents.md#role-requirements), preserving host-owned goal and receiver authority. When your executing backend's Role eligibility records Director eligible `no`, tell the user that this backend's Director verdict is `no`, name the backends whose Role eligibility records Director eligible `yes`, and run no `cafleet fleet create`.

This rule applies wherever a session would become a Director, including the design-doc Dispatch table's proactive team start, because every Director reads this role before orchestration.

`docs/docs/concepts/coding-agents.md` paragraph:

> Run the root Director on a backend whose Director verdict is `yes`, and spawn members on backends whose Member verdict is `yes`. [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records each backend's verdicts with their evidence; a backend whose Director verdict is `no` runs only as a member.

The CLI is unchanged: `fleet create --coding-agent` still accepts every backend. The fleet-backend rows of Director model selection stay unchanged, because they specify member resolution for any recorded fleet backend and the Director role's precondition already excludes an ineligible Director backend.

### Placement

| File | Change |
|---|---|
| `SPEC.md` | §6.7 Role requirements, Role eligibility and the OpenCode host-evidence sentence (edits below). |
| `docs/docs/concepts/coding-agents.md` | Director-selection paragraph and the interactive-policies rewording (edits below). |
| `docs/docs/how-to/mixed-backend-team.md` | Line 80: "On a backend whose [Director verdict](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) is `yes`, state …" (rest unchanged). |
| `docs/docs/contributing.md` | § Applying the bar to a candidate: procedure, evidence table and prose; the existing evidence paragraph is absorbed. |
| `README.md` | Unchanged: its pitch, install commands and section list are unaffected. Verify only. |
| `skills/cafleet/reference/coding-agents.md` | Six edits below. |
| `skills/cafleet/roles/director.md` | Intro and § Model selection per § Director backend selection. |
| `skills/cafleet/SKILL.md` | § Resolve your overlay step 1 per § Director backend selection. |
| `.claude/rules/coding-agent-overlay.md` | Five edits below. |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | Line 20: add "Role eligibility" to the preserved runtime documentation list. |

`SPEC.md` edits:

1. Replace §6.7 `#### Role requirements` content with its § The bar final text.
2. Add `#### Role eligibility` after the retained catalog paragraph: the six verdict rows and the `SPEC.md` form of the OpenCode flip check.
3. Append to the **opencode** paragraph of `#### Host evidence and native-input authorization`: "Goal lifecycle controls and Director background execution are not established for opencode 2.1.4, so its Director verdict is `no` (§ Role eligibility)."

`docs/docs/concepts/coding-agents.md` edits:

1. Add the Director-selection paragraph after the `--coding-agent` bullets.
2. In the PTY-runner paragraph, replace "Interactive native policies are eligible on every backend" with "Interactive native policies are available as explicit choices on every backend", so "eligible" stays reserved for role verdicts.

`skills/cafleet/reference/coding-agents.md` edits:

1. Preamble (line 12): extend "Runtime bindings, bound runtime notes, and worked resolutions belong to runtime documentation maintenance" to include Role eligibility, and add "Re-check a backend's Role eligibility whenever its bundled profile boundary or `Evidence:` line changes."
2. Selector table: the "Select/configure a spawned member" row reads "Its Model catalog, Role defaults and Role eligibility; …" (rest unchanged).
3. § Role requirements: replace with its § The bar final text.
4. Append `### Role eligibility` at the end of the `claude`, `codex` and `opencode` sections (after any `####` recipe) with that backend's two rows from § Verdict record; add the OpenCode flip check beneath the OpenCode table.
5. OpenCode `Note → applies at` row 2: replace the Note cell with "Members perform assignments using the selected provider/model, workspace reads/edits, permitted tools and broker communication. OpenCode's Director verdict is `no` (Role eligibility below), so an OpenCode session follows the Director role's ineligible-backend rule." Applies at stays unchanged.
6. `## Template`: add `### Role eligibility` after `### Note → applies at`, containing the guidance quoted below.

`.claude/rules/coding-agent-overlay.md` edits:

1. "contains these four subsections in order" becomes "five".
2. Add item 5: "**Role eligibility**: per-role `yes`/`no` verdicts for Director and Member, each with checked native version, source and date."
3. Backend-specific `####` subsections follow Note → applies at and precede Role eligibility; "supplying all four subsections" becomes "all five".
4. Runtime documentation maintenance owns Role eligibility and re-checks it whenever a backend's bundled profile boundary or `Evidence:` line changes.
5. Three homes: the `SPEC.md` content cell adds "role requirements and per-backend role eligibility".

`Template` guidance text:

> Use a `Role | Eligible | Evidence` table with one Director row and one Member row. Mark `yes` only when every Eligible-tier requirement in § Role requirements is established; otherwise `no` with "not established" or the observed failure. Each Evidence cell names the source, native version and `checked YYYY-MM-DD` for each requirement group in at most two sentences, and a Member row names its bundled profile instead of listing its capabilities.

`skills/cafleet/reference/supervision.md`, `skills/cafleet/roles/member.md` and the Claude/Codex "host satisfying the Director requirements" sentences stay unchanged: they remain correct and already link § Role requirements, which now points to the verdicts.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] Replace §6.7 `#### Role requirements` content with its final text from § The bar. <!-- completed: 2026-10-31T05:03 -->
- [x] Add §6.7 `#### Role eligibility` with the six verdict rows and the OpenCode flip check. <!-- completed: 2026-10-31T05:03 -->
- [x] Append the OpenCode Director-verdict sentence to §6.7 Host evidence and native-input authorization. <!-- completed: 2026-10-31T05:03 -->

### Step 2: docs/

- [x] Apply both `docs/docs/concepts/coding-agents.md` edits (Director-selection paragraph, interactive-policies rewording). <!-- completed: 2026-10-31T05:04 -->
- [x] Update `docs/docs/how-to/mixed-backend-team.md` line 80 to the Director-verdict link. <!-- completed: 2026-10-31T05:04 -->
- [x] Add the evaluation procedure, capability → evidence table and recording rule to `docs/docs/contributing.md` § Extend a coding-agent backend, absorbing the existing evidence paragraph. <!-- completed: 2026-10-31T05:04 -->

### Step 3: README.md

- [x] Verify `README.md` needs no change (pitch, install commands and section list unaffected). <!-- completed: 2026-10-31T05:04 -->

### Step 4: skills/

- [x] Apply `coding-agents.md` edits 1–3 (preamble, selector table, § Role requirements). <!-- completed: 2026-10-31T05:05 -->
- [x] Apply `coding-agents.md` edit 4: the three `### Role eligibility` subsections and the OpenCode flip check. <!-- completed: 2026-10-31T05:05 -->
- [x] Apply `coding-agents.md` edits 5–6 (OpenCode note, Template guidance). <!-- completed: 2026-10-31T05:05 -->
- [x] Update `skills/cafleet/roles/director.md` intro (including the ineligible-backend rule) and § Model selection. <!-- completed: 2026-10-31T05:05 -->
- [x] Update `skills/cafleet/SKILL.md` § Resolve your overlay step 1. <!-- completed: 2026-10-31T05:05 -->

### Step 5: Project rules and skills

- [x] Apply the five `.claude/rules/coding-agent-overlay.md` edits. <!-- completed: 2026-10-31T05:06 -->
- [x] Add Role eligibility to the preserved list in `.claude/skills/cafleet-model-list-refresh/SKILL.md`. <!-- completed: 2026-10-31T05:06 -->

### Step 6: Validate

- [x] Run `mise //docs:build` and confirm it passes. <!-- completed: 2026-10-31T05:06 -->
- [x] Review every new or changed link and anchor (`SPEC.md#role-requirements`, `SPEC.md#role-eligibility`, `coding-agents.md#role-requirements`, intra-file references) and confirm table cells keep at most two sentences. <!-- completed: 2026-10-31T05:06 -->
- [x] Review the diff: Markdown only, files limited to § Placement, identical verdict text in both homes, every shipped sentence handled per § Shipped sentence dispositions, skills link no `docs/` or `SPEC.md`. <!-- completed: 2026-10-31T05:06 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-31 | Initial draft |
| 2026-10-31 | Review round 1: version probe moved to Bundled support; final texts and shipped-sentence dispositions per home; per-requirement-group version rule; Member evidence cites live runner acceptance (OpenCode herdr-only, judged sufficient); flip check restates the bar; ineligible-Director behavior; concepts rewording; Placement split into per-file edit lists |
| 2026-10-31 | Approved by the user |
