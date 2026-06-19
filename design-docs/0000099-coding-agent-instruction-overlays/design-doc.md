# Coding-agent instruction overlays

**Status**: Approved
**Progress**: 31/31 tasks complete
**Last Updated**: 2026-06-19

## Overview

Split the cafleet skill family into a backend-neutral **base** instruction set and a per-coding-agent **overlay** (`skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`). Every base instruction that varies by backend states the neutral behavior and points the agent at its overlay; the overlay carries the concrete delta (decision surface, monitor model, permission flags, task primitives, skill-loading recipe, pane discovery). After the refactor the base reads as if it were always backend-neutral.

## Success Criteria

- [x] Three overlay files exist at `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`, each a compact `| Placeholder | Value |` table defining the 9 canonical placeholders (§2) for its backend.
- [x] An overlay template exists at `skills/cafleet/reference/coding-agent/_template.md`, carrying the 9-placeholder value-table skeleton with angle-bracket fill-in guidance and no backend-specific values (§5a). The three overlays follow its table structure.
- [x] No base instruction file names a backend-specific value. The base is every cafleet-family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` EXCEPT everything under `skills/cafleet/reference/coding-agent/` (the three overlays and the `_template.md`). A grep across the base for `--model sonnet`, `--permission-mode`, `--ask-for-approval`, `--sandbox workspace-write`, `--agent cafleet`, `AskUserQuestion`, `send-input`, `run_in_background`, `TaskStop`, `TaskCreate`, `TaskUpdate`, `TaskList`, `TaskGet`, and the skill-loader variants (`Skill tool`, the backticked `` `Skill` tool ``, `via the Skill tool`) returns nothing — with two exemptions: (a) the `allowed-tools:` YAML front matter of any family skill (a functional Claude tool grant that cannot live in an overlay — the research skills' `Task*`/`TaskStop` and `cafleet-design-doc-interview`'s `AskUserQuestion`; see §4b), and (b) delta-6, which is additionally verified by reviewing the skill-loading prose, not the literal grep alone. A skill's `description:` field is not exempt — it is neutralized like body prose.
- [x] The model-name-to-backend inference table remains in `skills/cafleet/reference/director.md` unchanged (it is a backend-agnostic selector, not a delta).
- [x] `skills/cafleet/SKILL.md` carries a prominent early "apply your overlay" instruction explaining the `{placeholder}` → value-table substitution convention; every other family `SKILL.md` carries a one-line overlay pointer linking `../cafleet/reference/coding-agent/<name>.md`. The base uses the 9 canonical `{placeholder}` tokens (§2) for every backend-varying value, each substituting cleanly from the overlay's value table.
- [x] The canonical spawn-prompt skeleton in `reference/director.md` and the monitoring-member spawn prompt in `cafleet-agent-team-monitoring/SKILL.md` both include a `CODING AGENT: <name>` identity line.
- [x] No skill→`docs/reference/coding-agents/` link remains; no link exists in EITHER direction between `skills/cafleet/reference/coding-agent/` and `docs/reference/coding-agents/`.
- [x] `.claude/rules/coding-agent-overlay.md` exists and is written affirmatively (positive spec, not a pile of prohibitions).
- [x] `.claude/skills/skill-author/SKILL.md` teaches the base/overlay pattern (a new section).
- [x] removal.md compliance: the base contains zero deprecation residue — no "see claude.md for the old wording", no historical callouts, no flag rows for relocated values.
- [x] `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, and `mise //cafleet:test` all pass. The one intended behavioral change is the monitor model (claude `sonnet` → `haiku`, per the user's cheapest-capable decision — see §4a); otherwise this is a docs/skills/rules refactor with no behavioral change.

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

- **Base** = every cafleet-family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` EXCEPT everything under `skills/cafleet/reference/coding-agent/` (the three overlays and the `_template.md` skeleton). The base is fully backend-neutral: it states *what* to do in backend-agnostic terms and, wherever behavior varies, points at the overlay. Among the `reference/*.md` files, `director.md` and `recovery.md` carry backend idioms (the `AskUserQuestion` / `send-input` surface) and ARE neutralized; `broadcast.md`, `exec-routing.md`, and `output-flags.md` contain no backend deltas (verified) and are left as-is. **Scope of "cafleet-family":** the family is the enumerated set in the Background section — `skills/cafleet/`, `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, the `cafleet-design-doc-*` skills, and the `cafleet-research-*` skills. Utility skills loaded by the family but not part of it — `cafleet-base-dir`, `cafleet-create-figure`, `cafleet-my-slidev` — are OUT of scope here and are not neutralized by this design; `cafleet-base-dir`'s `AskUserQuestion` usage is a candidate for a follow-up that extends the base/overlay pattern to utility skills. The `cafleet-research-report` § *Spawnable Agents* web-researcher dispatch recipe IS in scope (initially ruled out, reversed on Opus re-review): its "Claude Code recipe" names the `Agent` tool, which is delta-6 content (§4), so the per-backend dispatch split is the inline-per-backend pattern §1/§10 remove. It is neutralized to a backend-neutral dispatch instruction that points at the overlay (the per-step bullets in Step 7 are non-exhaustive; the goal binds, as in Steps 4–6).
- **Overlay** = `skills/cafleet/reference/coding-agent/<name>.md` for `<name>` in `{claude, codex, opencode}`. The overlay states *how* its backend realizes each neutral instruction. It is the single canonical home for every backend delta; sibling skills link to it via `../cafleet/reference/coding-agent/<name>.md`.
- An agent reads the base, identifies its coding agent, reads its overlay, and applies the overlay's deltas on top of every base instruction.

### 2. The placeholder model (base tokens + overlay value table)

The base is written backend-neutral with `{placeholder}` tokens for every value that varies by coding agent; each overlay (`reference/coding-agent/<name>.md`) is a compact **value table** that defines those tokens for its backend. An agent reads the base, identifies its coding agent, reads its overlay's value table, and substitutes the overlay's value for each `{placeholder}` as it reads. Two placements make every agent aware of this:

1. **Canonical statement** — a new early section in `skills/cafleet/SKILL.md` (right after the "Reference files" list), worded affirmatively:

   > **Apply your coding-agent overlay.** CAFleet instructions are backend-neutral, written with `{placeholder}` tokens for everything that varies by coding agent. Your overlay — `reference/coding-agent/<name>.md` — is a value table defining each token. Identify your coding agent (your spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity), then substitute your overlay's value for each `{placeholder}` you encounter.

2. **Sibling pointer** — every other family `SKILL.md` (`cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-design-doc-*`, `cafleet-research-*`) gets a one-line pointer near its top linking `../cafleet/reference/coding-agent/<name>.md`.

**Canonical placeholder set** (9): `{decision_surface}`, `{monitor_model}`, `{member_model}`, `{permission_flags}`, `{bg_run}`, `{bg_stop}`, `{task_coord}`, `{pane_title}`, `{skill_loader}`. Where a Claude-specific structure (an option-count cap, a harness task list) does not generalize, the base conditionalizes it — "if your surface caps how many options it shows, paginate"; "on a harness task list, set owner …" — so substitution reads correctly on every backend. Concepts richer than a single value — the `send-input` relay/recovery primitive, the decision-prompt frame shape, the sub-agent dispatch recipe — stay as overlay pointers rather than tokens.

### 3. Backend self-identification (spawn-prompt change)

A spawned member must know which overlay to read. The Director already knows the backend (it chose `--coding-agent`), so it stamps the backend into the spawn prompt.

- Add a `CODING AGENT: <name>` line to the **identity block** of the canonical spawn-prompt skeleton in `skills/cafleet/reference/director.md` § *Canonical spawn-prompt skeleton*, adjacent to `FLEET ID` / `DIRECTOR AGENT ID` / `YOUR AGENT ID` / `BASE`.
- The Director fills this slot the same way it fills `BASE` (a rendered literal — `CODING AGENT: codex`), so **no CLI code change is required**. The member reads `reference/coding-agent/<name>.md` deterministically from this line.
- Add the same line to the monitoring-member spawn prompt in `cafleet-agent-team-monitoring/SKILL.md` § *The monitoring member*.
- A standalone agent (no spawn prompt) uses its own identity to select its overlay.

> Alternative considered, not chosen: have `cafleet member create` inject `coding_agent` as a fourth `str.format()` kwarg so authors cannot forget it. This is a small code change; it is recorded here for the implementer's awareness but the spec defaults to the code-free Director-stamp approach to honor the minimal-code constraint and to stay consistent with how `BASE` is already handled.

### 4. The six deltas — base-neutral form and per-overlay content

Every value below moves OUT of the base and into the overlays. The base carries only a `{placeholder}` token — delta 1 → `{decision_surface}`; 2 → `{monitor_model}` (+ `{member_model}` for an ordinary member); 3 → `{permission_flags}`; 4 → `{bg_run}` / `{bg_stop}` / `{task_coord}`; 5 → `{pane_title}`; 6 → `{skill_loader}`. The three overlay columns give each backend's value for that token (the overlay's value table). The "base intent" column states what the neutral instruction conveys — the prose the `{placeholder}` stands in for.

| # | Concept | Base intent (the `{placeholder}` stands in for this) | `claude.md` | `codex.md` | `opencode.md` |
|---|---|---|---|---|---|
| 1 | **User-reaction surface** | "When you need a recorded user reaction (approve / choose / confirm / continue-or-abort), solicit it through your decision surface — never in free-form prose, which records no answer. A fleet member never talks to the user; it sends its question to the Director, which relays it. See your overlay for the concrete surface and the question-shape taxonomy." | Names `AskUserQuestion` as the surface; the full question-shape taxonomy table; the no-explicit-"Other" rule; the every-escalation-is-a-decision-point gate; standalone-vs-fleet (standalone calls `AskUserQuestion` itself, a member routes to the Director); the `cafleet member send-input` 4-option pane frame — the three-beat capture → `AskUserQuestion` → `send-input` workflow, the pane-shape table, the `--choice`/`--freetext` keystrokes, and the constraints. | No interactive in-pane decision prompt: the member sends its question to the Director via `cafleet message send`, and the Director answers as a plain operator message (read-then-respond cadence). The overlay states this affirmatively and does NOT reference the claude `AskUserQuestion` idiom. | No interactive in-pane decision prompt; opencode normally shows no permission popup (the safety floor resolves every check), so the Director answers as a plain operator message, same read-then-respond cadence. The overlay does NOT reference `AskUserQuestion`. |
| 2 | **Monitor model** | "Spawn the monitor with the cheapest capable model for the monitor's own backend (claude by default — see §4a)." | `--model haiku` | `--model gpt-5.4-mini` (cheaper than `gpt-5.5`) | `--model anthropic/claude-haiku-4-5` |
| 3 | **Auto-approval / permission mode** | "Members are spawned in workspace-scoped auto-approval mode: the Bash tool is enabled and routine permission prompts auto-resolve. See your overlay for the exact flags." | `--permission-mode dontAsk` | `--ask-for-approval never --sandbox workspace-write` | `--agent cafleet` |
| 4 | **Background-task + task-list primitives** | "Run long-lived background work (e.g. the Slidev dev server) via your backend's background-run primitive and stop it at teardown via the matching stop primitive. Coordinate parallel sub-work via your backend's task-list primitive; if your backend has none, coordinate via cafleet messages." | Bash tool `run_in_background: true`; the returned task id feeds `TaskStop`; `TaskCreate`/`TaskUpdate`/`TaskList`/`TaskGet` are the work-coordination substrate. | No harness task primitive: run long-lived work via the leading-`!` shell shortcut backgrounded, stop it at teardown, and coordinate parallel sub-work via cafleet messages. | Same as codex: no harness task primitive — leading-`!` backgrounded run, stop at teardown, cafleet-message coordination. |
| 5 | **Pane discovery / pane title** | "Discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Some backends additionally set the pane title from the member name — see your overlay." | `claude --name <member-name>` sets `#{pane_title}` to the member name. | No `--name` analog; locate panes via `cafleet member list`. | No `--name` analog; locate panes via `cafleet member list`. |
| 6 | **Skill-loading recipe** | "Load the listed skills at startup via your overlay's skill-loading recipe; if your backend cannot load skills, read the referenced files by the absolute paths your spawn prompt provides. Do not read skill files directly when a loader exists." | Load via the Skill tool; dispatch sub-agents via the Agent tool. | codex cannot load Claude Code skills — read the cafleet `SKILL.md` core and `reference/coding-agent/codex.md` by the absolute paths the spawn prompt provides. | opencode cannot load Claude Code skills either — read the cafleet `SKILL.md` core and `reference/coding-agent/opencode.md` by the absolute paths the spawn prompt provides. |

**Stays in the base (NOT a delta):** the model-name-to-backend inference table in `reference/director.md` § *Model-name-to-backend inference* is a backend-agnostic selector mechanism. It does not move and is not duplicated into any overlay.

**§4a — Monitor model is a runnable value (delta-2).** The monitor model is set inside a runnable spawn command (`cafleet member create … --role monitor --model <value>`), so the base cannot replace it with a pure "see overlay" pointer — the Director needs a concrete `--model <value>` at spawn time. The base monitor-spawn template therefore carries `--model <cheapest capable model for the monitor's backend>` and the Director substitutes the concrete value from the overlay when it runs the command (the same render-time substitution it already performs for `BASE`). The monitoring member is spawned WITHOUT `--coding-agent`, so it runs on claude by default: the resolved value is `--model haiku` and the monitor's `CODING AGENT:` line is the literal `claude`. The three delta-2 overlay values document the cheapest-capable pick per backend so the principle holds for whichever backend the monitor runs on — the codex / opencode values apply only if a Director ever spawns the monitor with `--coding-agent codex` / `opencode`; this design does NOT change the monitor's default backend. The claude value `haiku` replaces the current `sonnet` — the one intentional behavioral change, per the user's cheapest-capable decision.

**§4b — `allowed-tools` front matter stays (functional tool grant).** A skill's `allowed-tools:` YAML front matter is a functional Claude-harness tool grant, not instructional prose, and it is not read from an overlay file — so it MUST stay in the skill's own `SKILL.md` and is exempt from the delta-token grep (Success Criteria, Step 9). This covers the research skills (`cafleet-research-report`, `cafleet-research-presentation`), which declare `Task*` / `TaskStop` (delta-4), AND `cafleet-design-doc-interview`, which declares `AskUserQuestion` (delta-1) because its Director needs the grant to call `AskUserQuestion` on Claude. Only the PROSE that describes how to use these primitives moves to the overlay; on codex / opencode the front matter is simply ignored and the overlay's fallback applies. A skill's `description:` field is NOT exempt — it is loader-facing metadata and is neutralized like body prose (drop the delta token, keep the meaning).

### 5. Overlay file outline

Each overlay is a compact **value table**: it opens with `# Overlay: <backend>` and the one-liner "Substitute these into the base `{…}` placeholders.", then a `| Placeholder | Value |` markdown table with one row per canonical placeholder (§2), each filled with this backend's value. Values are short noun phrases that read correctly when substituted into the base sentences. The claude overlay additionally ends with a one-line `send-input` relay note pointing at `docs/spec/cli-options.md` (the deep keystroke detail is NOT reproduced in the overlay). Overlays are self-contained; they may reference the shared CLI spec (`docs/spec/cli-options.md`) but **must not** link to `docs/reference/coding-agents/`.

#### §5a — The overlay template (`_template.md`)

Because all overlays share the same value-table structure, the canonical skeleton lives in one place: `skills/cafleet/reference/coding-agent/_template.md`. An author starts a new overlay by copying the template and filling each row. The template:

- Opens with `# Overlay: <backend name>` and "Substitute these into the base `{…}` placeholders. Fill every row with this backend's concrete value; keep values short."
- Carries the `| Placeholder | Value |` table with all 9 canonical placeholder rows (§2), each value an angle-bracket guidance description of what the backend states there (no backend-specific values).
- Carries **no backend-specific value**. It lives under `coding-agent/`, so it is excluded from the base delta grep (§1, Step 9) and **must not** link to `docs/reference/coding-agents/`.

Every overlay's table must define all 9 canonical placeholders; a backend with "no analog" for a value states that *as* the value (e.g. `{task_coord}` = "none — coordinate via cafleet messages") rather than omitting the row.

The codex / opencode operational facts the overlays state are sourced from the human docs (`docs/reference/coding-agents/{codex,opencode}.md`) and RESTATED in the overlay (deliberate restatement, no link). The resolved facts both overlays must carry:

1. **Decision surface** — neither codex nor opencode has an interactive in-pane decision prompt; the member sends its question to the Director via `cafleet message send`, and the Director answers as a plain operator message (opencode normally shows no permission popup — the safety floor resolves every check, so a popup is a regression to escalate, not a decision point). The codex / opencode overlays state this affirmatively and MUST NOT reference the claude `AskUserQuestion` idiom — only `claude.md` names `AskUserQuestion`, because it is claude's actual surface.
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

- [x] `cafleet-agent-team-monitoring/SKILL.md`: neutralize every `--model sonnet` to delta-2; neutralize the `AskUserQuestion`-idiom callout to delta-1. <!-- completed: 2026-06-19T11:47 -->
- [x] `cafleet-agent-team-supervision/SKILL.md`: neutralize every `--model sonnet` to delta-2; neutralize the § *User Delegation Protocol* `AskUserQuestion` references to delta-1 (decision surface + overlay). <!-- completed: 2026-06-19T11:47 -->

### Step 6: Neutralize the base — design-doc family

- [x] `cafleet-design-doc-create/SKILL.md` + `roles/director.md`, `roles/drafter.md`, `roles/reviewer.md`: neutralize `--model sonnet`, `--permission-mode dontAsk`, and `AskUserQuestion` references to deltas 2/3/1. <!-- completed: 2026-06-19T12:03 -->
- [x] `cafleet-design-doc-execute/SKILL.md` + `roles/director.md` (and other roles): neutralize `--model sonnet`, `--permission-mode dontAsk`, and `AskUserQuestion` references to deltas 2/3/1. <!-- completed: 2026-06-19T12:03 -->
- [x] `cafleet-design-doc-interview/SKILL.md` + `roles/*.md`: neutralize `--model sonnet` to delta-2 and the `AskUserQuestion`-rounds mechanism to delta-1 (the Director's decision surface, per overlay). <!-- completed: 2026-06-19T12:03 -->
- [x] `cafleet-design-doc/SKILL.md`: confirm the skill-loader phrasing is the neutral delta-6 form (it already says "via their backend's skill-loader"); add the overlay pointer if missing. <!-- completed: 2026-06-19T12:03 -->

### Step 7: Neutralize the base — research family

- [x] `cafleet-research-report/SKILL.md` + `roles/{director,manager,researcher}.md`: neutralize the PROSE for `--model sonnet` (delta-2), the `Task*` work-coordination substrate (delta-4), the "load skills via the Skill tool" recipe (delta-6), and the `AskUserQuestion` reference (delta-1). KEEP the `allowed-tools:` front matter intact (§4b). <!-- completed: 2026-06-19T12:13 -->
- [x] `cafleet-research-presentation/SKILL.md` + `roles/*.md`: neutralize the PROSE for `--model sonnet` (delta-2), `run_in_background`/`TaskStop` (delta-4), and the `AskUserQuestion` reference (delta-1). KEEP the `allowed-tools:` front matter intact (§4b). <!-- completed: 2026-06-19T12:13 -->

### Step 8: Narrow docs-staleness fixes

- [x] `docs/reference/coding-agents/codex.md`: update the stale "spawn prompt points them at this page" sentence per §7 (no link to the overlay). <!-- completed: 2026-06-19T12:16 -->
- [x] `docs/reference/coding-agents/opencode.md`: apply the equivalent fix if it carries the same claim. <!-- completed: 2026-06-19T12:16 -->
- [x] `README.md`: grep for coding-agent-specific values / skill-structure references; update only what this change makes stale, otherwise leave untouched. <!-- completed: 2026-06-19T12:16 -->

### Step 9: Verification

- [x] Grep the base (every family `SKILL.md`, every `roles/*.md`, and every `skills/cafleet/reference/*.md` excluding `reference/coding-agent/*`; also excluding the new rule and the skill-author extension) for the delta tokens listed in Success Criteria; confirm zero hits, exempting the `allowed-tools:` front matter (§4b). Verify delta-6 by reviewing the skill-loading prose, not the literal grep alone. The model-name-to-backend inference table contains none of the tokens. <!-- completed: 2026-06-19T12:33 -->
- [x] Confirm no skill→`docs/reference/coding-agents/` link remains and no link exists in either direction between the two homes. <!-- completed: 2026-06-19T12:33 -->
- [x] Confirm every family `SKILL.md` carries the overlay pointer and `reference/director.md` + the monitor prompt carry the `CODING AGENT:` line. <!-- completed: 2026-06-19T12:33 -->
- [x] removal.md sweep: confirm no deprecation residue (no "see claude.md", no historical callouts, no relocated-flag rows) in the base. <!-- completed: 2026-06-19T12:33 -->
- [x] Run `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test`; confirm green. <!-- completed: 2026-06-19T12:33 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-19 | Initial draft |
| 2026-06-19 | Add standalone overlay template `_template.md` (§5a, new Success Criterion, Step 1 task) per user request; broaden no-cross-link check to cover the template; Progress denominator 30 → 31. |
| 2026-06-19 | Step 9 ruling (per Administrator): `cafleet-base-dir` is out of scope; added a "Scope of cafleet-family" clarification to §1 (the family is the Background enumeration; utility skills are excluded). All 5 Step-9 verification checks pass. |
| 2026-06-19 | Opus review round 1 (3 findings): fixed incomplete delta-1 neutralization + stale cross-ref in `cafleet-agent-team-monitoring/SKILL.md` (claude 4-option frame / `--lines` sizing → overlay), and neutralized residual AskUserQuestion `"Other"` option-label phrasing → "free-text" across interview / research-report / create / execute. Initially deferred the `cafleet-research-report` § *Spawnable Agents* dispatch recipe (reversed in round 2 — see below). |
| 2026-06-19 | User-flagged fix: the `codex.md` / `opencode.md` / `_template.md` overlays defined their decision surface by reference to claude's `AskUserQuestion` ("no AskUserQuestion analog") — a claude-centric leak. Rewrote them to state the plain-message-via-Director surface affirmatively, with no `AskUserQuestion` reference; only `claude.md` names it (its actual surface). Updated §4 delta-1 codex/opencode cells, §5/§5a, accordingly. |
| 2026-06-19 | Opus review round 2: reversed the round-1 out-of-scope ruling on the `cafleet-research-report` § *Spawnable Agents* recipe — the "Claude Code recipe" names the `Agent` tool (delta-6), so it is an inline-per-backend leak (§1/§10), in scope by the same non-exhaustive-bullets principle applied in Steps 4–6. Neutralized the per-backend dispatch split to a backend-neutral instruction pointing at the overlay. |
| 2026-06-19 | Opus review round 3: neutralized the matching per-backend dispatch pointer in `cafleet-research-report/roles/web-researcher.md` (base roles file) to the backend-neutral form. |
| 2026-06-19 | Opus review round 4: **approved** — base reads backend-neutral, overlays self-contained, no cross-links, removal-clean. Review fixes pushed to PR #130. Status → Complete. Non-blocking follow-up (out of scope): `web-researcher.md` frontmatter `model: sonnet` is vestigial Claude agent-spec metadata (§4b-class, ignored on codex). |
| 2026-06-19 | Post-completion fix (per Administrator audit): bare backend-specific **model names** had leaked past the Success-Criteria grep (which only tokenized `--model sonnet`, not bare names). Relocated the `director.md` `--model` per-backend examples + opencode `<provider-id>/<model-id>` note into each overlay's delta-2 (with a neutral pointer left in `director.md`); dropped the vestigial `web-researcher.md` `model: sonnet`. The model-name-to-backend inference table stays (backend-agnostic selector, used before the backend is known — cannot live in a per-backend overlay). |
| 2026-06-19 | **Variable-substitution redesign (per Administrator).** The overlays were too wordy and the "see your overlay" prose-pointer model was indirect. Reworked to a template-variable model: the base is written with 9 canonical `{placeholder}` tokens (`{decision_surface}` `{monitor_model}` `{member_model}` `{permission_flags}` `{bg_run}` `{bg_stop}` `{task_coord}` `{pane_title}` `{skill_loader}`), and each overlay is a compact `\| Placeholder \| Value \|` table defining them (claude.md ~74→18 lines). Rewrote §2/§4/§5/§5a + SC accordingly. Claude-specific structures (option-count cap, harness task list) are conditionalized in the base so substitution reads correctly on every backend; concepts richer than a value (`send-input` relay/recovery, decision-prompt frame, sub-agent dispatch) stay as overlay pointers. |
| 2026-06-19 | Step 6 arbitration: generalize §4b + Success-Criteria exemption (a) so the `allowed-tools:` front-matter exemption covers any functional tool grant (research `Task*`/`TaskStop` AND interview `AskUserQuestion`); clarify that a skill's `description:` field is neutralized like body prose, not exempt. |
