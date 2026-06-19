# Coding-agent instruction overlays

**Status**: Approved
**Progress**: 15/31 tasks complete
**Last Updated**: 2026-06-19

## Overview

Split the cafleet skill family into a backend-neutral **base** instruction set and a per-coding-agent **overlay** (`skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`). Every base instruction that varies by backend states the neutral behavior and points the agent at its overlay; the overlay carries the concrete delta (decision surface, monitor model, permission flags, task primitives, skill-loading recipe, pane discovery). After the refactor the base reads as if it were always backend-neutral.

## Success Criteria

- [ ] Three overlay files exist at `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`, each covering all six backend deltas for its backend.
- [ ] An overlay template exists at `skills/cafleet/reference/coding-agent/_template.md`, carrying all six delta-section headings (in §4 order) with fill-in guidance and no backend-specific values (§5a). The three overlays follow its section structure.
- [ ] No base instruction file names a backend-specific value. The base is every cafleet-family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` EXCEPT everything under `skills/cafleet/reference/coding-agent/` (the three overlays and the `_template.md`). A grep across the base for `--model sonnet`, `--permission-mode`, `--ask-for-approval`, `--sandbox workspace-write`, `--agent cafleet`, `AskUserQuestion`, `send-input`, `run_in_background`, `TaskStop`, `TaskCreate`, `TaskUpdate`, `TaskList`, `TaskGet`, and the skill-loader variants (`Skill tool`, the backticked `` `Skill` tool ``, `via the Skill tool`) returns nothing — with two exemptions: (a) the `allowed-tools:` YAML front matter of the research skills (a functional Claude tool grant that cannot live in an overlay — see §4b), and (b) delta-6, which is additionally verified by reviewing the skill-loading prose, not the literal grep alone.
- [ ] The model-name-to-backend inference table remains in `skills/cafleet/reference/director.md` unchanged (it is a backend-agnostic selector, not a delta).
- [ ] `skills/cafleet/SKILL.md` carries a prominent early "apply your overlay" instruction; every other family `SKILL.md` carries a one-line overlay pointer linking `../cafleet/reference/coding-agent/<name>.md`.
- [ ] The canonical spawn-prompt skeleton in `reference/director.md` and the monitoring-member spawn prompt in `cafleet-agent-team-monitoring/SKILL.md` both include a `CODING AGENT: <name>` identity line.
- [ ] No skill→`docs/reference/coding-agents/` link remains; no link exists in EITHER direction between `skills/cafleet/reference/coding-agent/` and `docs/reference/coding-agents/`.
- [ ] `.claude/rules/coding-agent-overlay.md` exists and is written affirmatively (positive spec, not a pile of prohibitions).
- [ ] `.claude/skills/skill-author/SKILL.md` teaches the base/overlay pattern (a new section).
- [ ] removal.md compliance: the base contains zero deprecation residue — no "see claude.md for the old wording", no historical callouts, no flag rows for relocated values.
- [ ] `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, and `mise //cafleet:test` all pass. The one intended behavioral change is the monitor model (claude `sonnet` → `haiku`, per the user's cheapest-capable decision — see §4a); otherwise this is a docs/skills/rules refactor with no behavioral change.

---

## Background

The cafleet skill family (`skills/cafleet/` plus `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, the `cafleet-design-doc-*` skills, and the `cafleet-research-*` skills) is written Claude-Code-first. Coding-agent-specific behavior is scattered inline as ad-hoc blockquote callouts and hardcoded values — `--model sonnet`, `--permission-mode dontAsk`, the `AskUserQuestion` taxonomy, the `Task*` tools, the "load via the Skill tool" recipe — so the base drifts and every reader on a non-Claude backend must mentally subtract the Claude idioms.

Two audiences are deliberately kept separate and **must not cross-link**:

| Home | Audience | Content |
|---|---|---|
| `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md` (NEW) | the coding AGENT | the backend deltas an agent applies on top of the base |
| `docs/reference/coding-agents/{codex,opencode}.md` (existing) | HUMANS / operators | install, sandbox, CLI version pin, verification recipes |

Deliberate restatement of the same fact in both homes is acceptable where each audience genuinely needs it; the hard rule is no link in either direction between them.

---

## Specification

### 1. Architecture: base + overlay

- **Base** = every cafleet-family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` EXCEPT everything under `skills/cafleet/reference/coding-agent/` (the three overlays and the `_template.md` skeleton). The base is fully backend-neutral: it states *what* to do in backend-agnostic terms and, wherever behavior varies, points at the overlay. Among the `reference/*.md` files, `director.md` and `recovery.md` carry backend idioms (the `AskUserQuestion` / `send-input` surface) and ARE neutralized; `broadcast.md`, `exec-routing.md`, and `output-flags.md` contain no backend deltas (verified) and are left as-is.
- **Overlay** = `skills/cafleet/reference/coding-agent/<name>.md` for `<name>` in `{claude, codex, opencode}`. The overlay states *how* its backend realizes each neutral instruction. It is the single canonical home for every backend delta; sibling skills link to it via `../cafleet/reference/coding-agent/<name>.md`.
- An agent reads the base, identifies its coding agent, reads its overlay, and applies the overlay's deltas on top of every base instruction.

### 2. The overlay pointer (prominent, early)

The base must make every agent aware that it has to consult its overlay. Two placements:

1. **Canonical statement** — a new early section in `skills/cafleet/SKILL.md` (placed right after the intro / "Reference files" list), worded affirmatively, e.g.:

   > **Apply your coding-agent overlay.** CAFleet instructions are split into a backend-neutral base (this skill family) and a per-coding-agent overlay at `reference/coding-agent/<name>.md`. Identify your coding agent — your spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity — then read `reference/coding-agent/<name>.md` and apply its deltas on top of every base instruction.

2. **Sibling pointer** — every other family `SKILL.md` (`cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-design-doc-*`, `cafleet-research-*`) gets a one-line pointer near its top linking `../cafleet/reference/coding-agent/<name>.md` and instructing the agent to read and apply it.

### 3. Backend self-identification (spawn-prompt change)

A spawned member must know which overlay to read. The Director already knows the backend (it chose `--coding-agent`), so it stamps the backend into the spawn prompt.

- Add a `CODING AGENT: <name>` line to the **identity block** of the canonical spawn-prompt skeleton in `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton*, adjacent to `FLEET ID` / `DIRECTOR AGENT ID` / `YOUR AGENT ID` / `BASE`.
- The Director fills this slot the same way it fills `BASE` (a rendered literal — `CODING AGENT: codex`), so **no CLI code change is required**. The member reads `reference/coding-agent/<name>.md` deterministically from this line.
- Add the same line to the monitoring-member spawn prompt in `cafleet-agent-team-monitoring/SKILL.md` § *The monitoring member*.
- A standalone agent (no spawn prompt) uses its own identity to select its overlay.

> Alternative considered, not chosen: have `cafleet member create` inject `coding_agent` as a fourth `str.format()` kwarg so authors cannot forget it. This is a small code change; it is recorded here for the implementer's awareness but the spec defaults to the code-free Director-stamp approach to honor the minimal-code constraint and to stay consistent with how `BASE` is already handled.

### 4. The six deltas — base-neutral form and per-overlay content

Every row below moves OUT of the base and into the overlays. The base keeps only the neutral form (which carries the overlay pointer); the three overlay columns are the new canonical content.

| # | Concept | Base-neutral form (stays in base) | `claude.md` | `codex.md` | `opencode.md` |
|---|---|---|---|---|---|
| 1 | **User-reaction surface** | "When you need a recorded user reaction (approve / choose / confirm / continue-or-abort), solicit it through your decision surface — never in free-form prose, which records no answer. A fleet member never talks to the user; it sends its question to the Director, which relays it. See your overlay for the concrete surface and the question-shape taxonomy." | Names `AskUserQuestion` as the surface; the full question-shape taxonomy table; the no-explicit-"Other" rule; the every-escalation-is-a-decision-point gate; standalone-vs-fleet (standalone calls `AskUserQuestion` itself, a member routes to the Director); the `cafleet member send-input` 4-option pane frame — the three-beat capture → `AskUserQuestion` → `send-input` workflow, the pane-shape table, the `--choice`/`--freetext` keystrokes, and the constraints. | No `AskUserQuestion` analog and no validated 4-option `send-input` frame: the member sends its question to the Director via `cafleet message send`, and the Director answers as a plain operator message (read-then-respond cadence). | No `AskUserQuestion` analog; opencode normally shows no permission popup (the safety floor resolves every check), so the Director answers as a plain operator message, same read-then-respond cadence. |
| 2 | **Monitor model** | "Spawn the monitor with the cheapest capable model for the monitor's own backend (claude by default — see §4a)." | `--model haiku` | `--model gpt-5.4-mini` (cheaper than `gpt-5.5`) | `--model anthropic/claude-haiku-4-5` |
| 3 | **Auto-approval / permission mode** | "Members are spawned in workspace-scoped auto-approval mode: the Bash tool is enabled and routine permission prompts auto-resolve. See your overlay for the exact flags." | `--permission-mode dontAsk` | `--ask-for-approval never --sandbox workspace-write` | `--agent cafleet` |
| 4 | **Background-task + task-list primitives** | "Run long-lived background work (e.g. the Slidev dev server) via your backend's background-run primitive and stop it at teardown via the matching stop primitive. Coordinate parallel sub-work via your backend's task-list primitive; if your backend has none, coordinate via cafleet messages." | Bash tool `run_in_background: true`; the returned task id feeds `TaskStop`; `TaskCreate`/`TaskUpdate`/`TaskList`/`TaskGet` are the work-coordination substrate. | No harness task primitive: run long-lived work via the leading-`!` shell shortcut backgrounded, stop it at teardown, and coordinate parallel sub-work via cafleet messages. | Same as codex: no harness task primitive — leading-`!` backgrounded run, stop at teardown, cafleet-message coordination. |
| 5 | **Pane discovery / pane title** | "Discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Some backends additionally set the pane title from the member name — see your overlay." | `claude --name <member-name>` sets `#{pane_title}` to the member name. | No `--name` analog; locate panes via `cafleet member list`. | No `--name` analog; locate panes via `cafleet member list`. |
| 6 | **Skill-loading recipe** | "Load the listed skills at startup via your overlay's skill-loading recipe; if your backend cannot load skills, read the referenced files by the absolute paths your spawn prompt provides. Do not read skill files directly when a loader exists." | Load via the Skill tool; dispatch sub-agents via the Agent tool. | codex cannot load Claude Code skills — read the cafleet `SKILL.md` core and `reference/coding-agent/codex.md` by the absolute paths the spawn prompt provides. | opencode cannot load Claude Code skills either — read the cafleet `SKILL.md` core and `reference/coding-agent/opencode.md` by the absolute paths the spawn prompt provides. |

**Stays in the base (NOT a delta):** the model-name-to-backend inference table in `reference/director.md` § *Model-name-to-backend inference* is a backend-agnostic selector mechanism. It does not move and is not duplicated into any overlay.

**§4a — Monitor model is a runnable value (delta-2).** The monitor model is set inside a runnable spawn command (`cafleet member create … --role monitor --model <value>`), so the base cannot replace it with a pure "see overlay" pointer — the Director needs a concrete `--model <value>` at spawn time. The base monitor-spawn template therefore carries `--model <cheapest capable model for the monitor's backend>` and the Director substitutes the concrete value from the overlay when it runs the command (the same render-time substitution it already performs for `BASE`). The monitoring member is spawned WITHOUT `--coding-agent`, so it runs on claude by default: the resolved value is `--model haiku` and the monitor's `CODING AGENT:` line is the literal `claude`. The three delta-2 overlay values document the cheapest-capable pick per backend so the principle holds for whichever backend the monitor runs on — the codex / opencode values apply only if a Director ever spawns the monitor with `--coding-agent codex` / `opencode`; this design does NOT change the monitor's default backend. The claude value `haiku` replaces the current `sonnet` — the one intentional behavioral change, per the user's cheapest-capable decision.

**§4b — `allowed-tools` front matter stays (delta-4).** The research skills (`cafleet-research-report`, `cafleet-research-presentation`) declare `Task*` / `TaskStop` in their `allowed-tools:` YAML front matter. That front matter is a functional Claude-harness tool grant, not instructional prose, and it is not read from an overlay file — so it MUST stay in the skill's own `SKILL.md` and is exempt from the delta-token grep (Success Criteria, Step 9). Only the PROSE that describes how to use the primitives moves to the overlay (delta-4); on codex / opencode the front matter is simply ignored and the overlay's cafleet-message fallback applies.

### 5. Overlay file outline

Each overlay opens with a one-line statement of which backend it covers and "apply these deltas on top of the cafleet base", then one section per applicable delta from the table above (§4). Section order is the §4 numbering. The overlays are self-contained; they may reference the shared CLI spec (`docs/spec/cli-options.md`) and `docs/concepts/` pages, but **must not** link to `docs/reference/coding-agents/`.

#### §5a — The overlay template (`_template.md`)

Because all three overlays share the same six-section structure, the canonical skeleton lives in one place: `skills/cafleet/reference/coding-agent/_template.md`. An author starts a new overlay by copying the template and filling each section with the backend's concrete delta. The template:

- Opens with `# Overlay: <backend name>` and the one-liner "Apply these deltas on top of the cafleet base."
- Carries all six delta-section headings in §4 order, each with a short angle-bracket placeholder describing what the backend states there:
  1. `## 1. Decision surface` — how this backend solicits a recorded user reaction (the `AskUserQuestion` analog, or the plain-message fallback).
  2. `## 2. Monitor model` — `--model <cheapest capable model for this backend>`.
  3. `## 3. Auto-approval / permission mode` — the exact spawn flags.
  4. `## 4. Background-task + task-list primitives` — the background-run + stop primitive and the task-list (or cafleet-message) coordination substrate.
  5. `## 5. Pane discovery / pane title` — `cafleet member list` is ground truth; note any `--name`-style pane-title analog.
  6. `## 6. Skill-loading recipe` — the loader, or the read-by-absolute-path fallback.
- Carries **no backend-specific value** — only the structure and placeholders. It lives under `coding-agent/`, so it is excluded from the base delta grep (§1, Step 9) and, like the overlays, **must not** link to `docs/reference/coding-agents/`.

Each overlay's section structure must match the template (all six sections, §4 order); a backend with "no analog" for a section states that *as* the section's content rather than omitting the section.

The codex / opencode operational facts the overlays state are sourced from the human docs (`docs/reference/coding-agents/{codex,opencode}.md`) and RESTATED in the overlay (deliberate restatement, no link). The resolved facts both overlays must carry:

1. **Decision surface** — neither codex nor opencode has an `AskUserQuestion` analog; the member sends its question to the Director via `cafleet message send`, and the Director answers as a plain operator message (opencode normally shows no permission popup — the safety floor resolves every check, so a popup is a regression to escalate, not a decision point).
2. **Background / task primitives** — neither backend has a harness task primitive; run long-lived work via the leading-`!` shell shortcut backgrounded, stop it at teardown, and coordinate parallel sub-work via cafleet messages.
3. **Skill loading** — neither codex nor opencode can load Claude Code skills, so the member reads the cafleet `SKILL.md` core and its overlay by the absolute paths the spawn prompt provides.

### 6. Link rewrites (no cross-links)

- Remove the skill→docs links to `docs/reference/coding-agents/{codex,opencode}.md` in `skills/cafleet/SKILL.md` (§ *Coding-agent backends*) and `skills/cafleet/reference/director.md` (§ *Member Create* "Backend operational detail" line and the `opencode` first-spawn note). Replace each "backend operational detail" pointer with an overlay link to `reference/coding-agent/{codex,opencode}.md`.
- Audit both homes after the rewrite: `skills/cafleet/reference/coding-agent/*` must contain no link to `docs/reference/coding-agents/*`, and `docs/reference/coding-agents/*` must contain no link to `skills/cafleet/reference/coding-agent/*`.

### 7. Human-facing docs (leave intact except where stale)

Per the decision to keep the overlay an agent-facing, skill-internal concern, do **not** add a `docs/concepts` page and do **not** add user-facing docs coverage of the overlay mechanism. Leave `docs/concepts/coding-agents.md`, `docs/reference/coding-agents/{codex,opencode}.md`, and the rest of `docs/` untouched **except** where existing content goes stale because of this change:

- `docs/reference/coding-agents/codex.md` § *cafleet usage from inside a codex pane* currently states the codex member's "spawn prompt points them at this page instead". After the refactor, codex spawn prompts point at `reference/coding-agent/codex.md`. Update this sentence so it is accurate (point at the overlay, not the human doc), without adding a link to the overlay (state the path in prose, or drop the spawn-prompt claim). Apply the equivalent fix to `docs/reference/coding-agents/opencode.md` if it carries the same claim.
- `README.md`: update only if it documents a coding-agent specific value or skill structure that this change makes stale; otherwise leave it untouched.

### 8. The new rule — `.claude/rules/coding-agent-overlay.md`

Terse and affirmative. It states the convention positively:

- cafleet skill instructions are backend-neutral by default.
- Backend-specific deltas live in `skills/cafleet/reference/coding-agent/<name>.md`; every base instruction that varies by backend states the neutral behavior and points at the overlay.
- Authors write the base neutrally and put backend specifics in the overlay.
- The agent overlay home and the human `docs/reference/coding-agents/` home are independent and never cross-link.

### 9. The skill-author extension — `.claude/skills/skill-author/SKILL.md`

Add a new section ("Keep the base neutral; put backend deltas in the overlay") teaching the pattern: when writing a CAFleet-orchestrated skill, keep `SKILL.md`/`roles` backend-neutral; put backend-specific deltas (monitor model, permission flags, decision surface, task primitives, skill-loading, pane discovery) in the overlay; reference the overlay via the pointer; stamp `CODING AGENT: <name>` into the spawn prompt so members know which overlay to read.

### 10. removal.md compliance

When content moves from base to overlay, DELETE it from the base cleanly. The base must carry no "see claude.md for the old wording" pointer, no historical callout, no flag row documenting a relocated value, and no "X is a Claude Code idiom" residue. After the refactor the base reads as if it were always backend-neutral.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Build order is docs-first by construction: the overlays, the rule, and the skill-author extension ARE the documentation deliverable; the base neutralization points at them.

### Step 1: Author the overlay template and the three overlay files

- [x] Create `skills/cafleet/reference/coding-agent/_template.md` — the canonical overlay skeleton with all six delta-section headings (§4 order), fill-in placeholders, and no backend-specific values (§5a). Build this first; the three overlays follow its structure. <!-- completed: 2026-06-19T11:09 -->
- [x] Create `skills/cafleet/reference/coding-agent/claude.md` covering deltas 1–6 for claude per §4 (AskUserQuestion taxonomy + send-input frame, `--model haiku`, `--permission-mode dontAsk`, `Task*`/`run_in_background`/`TaskStop`, `--name` pane title, the Skill tool). <!-- completed: 2026-06-19T11:09 -->
- [x] Create `skills/cafleet/reference/coding-agent/codex.md` covering the applicable deltas for codex per §4/§5 (plain-message decision surface, `--model gpt-5.4-mini`, `--ask-for-approval never --sandbox workspace-write`, leading-`!` backgrounded run + cafleet-message fallback, `cafleet member list` pane discovery, read-by-path skill loading). <!-- completed: 2026-06-19T11:09 -->
- [x] Create `skills/cafleet/reference/coding-agent/opencode.md` covering the applicable deltas for opencode per §4/§5 (plain-message decision surface, `--model anthropic/claude-haiku-4-5`, `--agent cafleet`, leading-`!` backgrounded run + cafleet-message fallback, `cafleet member list` pane discovery, read-by-path skill loading). <!-- completed: 2026-06-19T11:09 -->
- [x] Confirm no overlay or the template links to `docs/reference/coding-agents/*` (no-cross-link rule). <!-- completed: 2026-06-19T11:09 -->

### Step 2: Add the convention artifacts

- [x] Create `.claude/rules/coding-agent-overlay.md` per §8, written affirmatively. <!-- completed: 2026-06-19T11:22 -->
- [x] Add the base/overlay-pattern section to `.claude/skills/skill-author/SKILL.md` per §9. <!-- completed: 2026-06-19T11:22 -->

### Step 3: Spawn-prompt skeleton + overlay pointer

- [x] Add the `CODING AGENT: <name>` identity line to the canonical spawn-prompt skeleton in `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton* (identity block + the per-role delta notes), and document that the Director fills it like `BASE`. <!-- completed: 2026-06-19T11:27 -->
- [x] Add the `CODING AGENT: <name>` line to the monitoring-member spawn prompt in `cafleet-agent-team-monitoring/SKILL.md` § *The monitoring member*. <!-- completed: 2026-06-19T11:27 -->
- [x] Add the prominent "Apply your coding-agent overlay" section to `skills/cafleet/SKILL.md` per §2(1). <!-- completed: 2026-06-19T11:27 -->
- [x] Add the one-line overlay pointer (`../cafleet/reference/coding-agent/<name>.md`) near the top of each sibling `SKILL.md`: `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-design-doc`, `cafleet-design-doc-create`, `cafleet-design-doc-execute`, `cafleet-design-doc-interview`, `cafleet-research-report`, `cafleet-research-presentation`. <!-- completed: 2026-06-19T11:27 -->

### Step 4: Neutralize the base — `skills/cafleet/` core

- [x] `skills/cafleet/SKILL.md`: replace § *Soliciting user reactions (AskUserQuestion)* with the neutral delta-1 form pointing at the overlay; remove the "AskUserQuestion is a Claude Code idiom" callout; replace the § *Coding-agent backends* docs link with the overlay link (§6). <!-- completed: 2026-06-19T11:40 -->
- [x] `skills/cafleet/reference/director.md`: neutralize `--model sonnet` (§ *Member Create*, monitor row) to delta-2 form; neutralize the pane-title asymmetry note to delta-5 form; move the § *Member Send-Input* three-beat `AskUserQuestion` workflow, pane-shape table, and constraints into `claude.md`, leaving a neutral delta-1 pointer; replace the docs links with overlay links; KEEP the model-name-to-backend inference table verbatim. <!-- completed: 2026-06-19T11:40 -->
- [x] `skills/cafleet/roles/member.md`: neutralize the three-backend permission-mode enumeration (opening paragraph) to the delta-3 form pointing at the overlay. <!-- completed: 2026-06-19T11:40 -->
- [x] `skills/cafleet/reference/recovery.md`: neutralize every `AskUserQuestion` / `send-input` reference to the delta-1 form pointing at the overlay. The three locations are § *2-stage health check* (the "bump to `--lines 120`/`200` to show an AskUserQuestion frame" mention), the § *Stalled member shapes* table (the `AskUserQuestion`-paused row, plus the "no AskUserQuestion frame" qualifiers in the REPL-idle and truly-wedged rows), and § *Recovering from a wedged `/exit`* (the AskUserQuestion-paused → `send-input` branch). <!-- completed: 2026-06-19T11:40 -->

### Step 5: Neutralize the base — monitoring & supervision skills

- [ ] `cafleet-agent-team-monitoring/SKILL.md`: neutralize every `--model sonnet` to delta-2; neutralize the `AskUserQuestion`-idiom callout to delta-1. <!-- completed: -->
- [ ] `cafleet-agent-team-supervision/SKILL.md`: neutralize every `--model sonnet` to delta-2; neutralize the § *User Delegation Protocol* `AskUserQuestion` references to delta-1 (decision surface + overlay). <!-- completed: -->

### Step 6: Neutralize the base — design-doc family

- [ ] `cafleet-design-doc-create/SKILL.md` + `roles/director.md`, `roles/drafter.md`, `roles/reviewer.md`: neutralize `--model sonnet`, `--permission-mode dontAsk`, and `AskUserQuestion` references to deltas 2/3/1. <!-- completed: -->
- [ ] `cafleet-design-doc-execute/SKILL.md` + `roles/director.md` (and other roles): neutralize `--model sonnet`, `--permission-mode dontAsk`, and `AskUserQuestion` references to deltas 2/3/1. <!-- completed: -->
- [ ] `cafleet-design-doc-interview/SKILL.md` + `roles/*.md`: neutralize `--model sonnet` to delta-2 and the `AskUserQuestion`-rounds mechanism to delta-1 (the Director's decision surface, per overlay). <!-- completed: -->
- [ ] `cafleet-design-doc/SKILL.md`: confirm the skill-loader phrasing is the neutral delta-6 form (it already says "via their backend's skill-loader"); add the overlay pointer if missing. <!-- completed: -->

### Step 7: Neutralize the base — research family

- [ ] `cafleet-research-report/SKILL.md` + `roles/{director,manager,researcher}.md`: neutralize the PROSE for `--model sonnet` (delta-2), the `Task*` work-coordination substrate (delta-4), the "load skills via the Skill tool" recipe (delta-6), and the `AskUserQuestion` reference (delta-1). KEEP the `allowed-tools:` front matter intact (§4b). <!-- completed: -->
- [ ] `cafleet-research-presentation/SKILL.md` + `roles/*.md`: neutralize the PROSE for `--model sonnet` (delta-2), `run_in_background`/`TaskStop` (delta-4), and the `AskUserQuestion` reference (delta-1). KEEP the `allowed-tools:` front matter intact (§4b). <!-- completed: -->

### Step 8: Narrow docs-staleness fixes

- [ ] `docs/reference/coding-agents/codex.md`: update the stale "spawn prompt points them at this page" sentence per §7 (no link to the overlay). <!-- completed: -->
- [ ] `docs/reference/coding-agents/opencode.md`: apply the equivalent fix if it carries the same claim. <!-- completed: -->
- [ ] `README.md`: grep for coding-agent-specific values / skill-structure references; update only what this change makes stale, otherwise leave untouched. <!-- completed: -->

### Step 9: Verification

- [ ] Grep the base (every family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` excluding `reference/coding-agent/*`; also excluding the new rule and the skill-author extension) for the delta tokens listed in Success Criteria; confirm zero hits, exempting the `allowed-tools:` front matter (§4b). Verify delta-6 by reviewing the skill-loading prose, not the literal grep alone. The model-name-to-backend inference table contains none of the tokens. <!-- completed: -->
- [ ] Confirm no skill→`docs/reference/coding-agents/` link remains and no link exists in either direction between the two homes. <!-- completed: -->
- [ ] Confirm every family `SKILL.md` carries the overlay pointer and `reference/director.md` + the monitor prompt carry the `CODING AGENT:` line. <!-- completed: -->
- [ ] removal.md sweep: confirm no deprecation residue (no "see claude.md", no historical callouts, no relocated-flag rows) in the base. <!-- completed: -->
- [ ] Run `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test`; confirm green. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-19 | Initial draft |
| 2026-06-19 | Add standalone overlay template `_template.md` (§5a, new Success Criterion, Step 1 task) per user request; broaden no-cross-link check to cover the template; Progress denominator 30 → 31. |
