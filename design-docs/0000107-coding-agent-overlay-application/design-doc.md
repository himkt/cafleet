# Coding-agent Overlay Application

**Status**: Complete
**Progress**: 17/23 tasks complete (Step 6 fresh-session behavioral probes deferred)
**Last Updated**: 2026-06-21

## Overview

Design 0000106 forces the agent to **read** its coding-agent overlay (Required-reading row #1), but the only *application* instruction is the passive aside "as you read the base, substitute your overlay's value for each `{placeholder}` you encounter" — so agents still emit literal `{monitor_model}` / `{permission_flags}` tokens, guess default or wrong values, or ignore backend note-lines (codex's "no harness task list", opencode's `--agent cafleet` safety floor). This design makes **resolution a distinct, gated cognitive step** built on 0000106's reading gate (not re-litigating it), in six coordinated parts:

1. a **resolve-then-act checkpoint** (the spine) in `cafleet/SKILL.md`;
2. an **emission self-check** (a literal `{token}` reaching the shell/broker/user is a defect);
3. **worked resolved examples** in every overlay;
4. **bound note-lines** (each overlay note maps to the base instruction it qualifies);
5. a **documented backend-neutral default** per token;
6. a **static consistency checker** guarding the overlay/base/template token set.

## Success Criteria

- [x] Every reader entry point's overlay row #1 reads **read-AND-resolve**, and its consequence cell names all three application-failure modes (literal token, wrong/default value, ignored note) — not just the unresolved-literal-token mode 0000106 records.
- [x] A canonical **Resolve your overlay** checkpoint exists in `cafleet/SKILL.md`: materialize each `{placeholder}` you will use to a concrete value, apply each overlay note at its bound base instruction, and self-check that no literal `{token}` escapes to the shell, the broker, or the user — with an explicit resolution order.
- [x] Every overlay (`claude`, `codex`, `opencode`) binds each note to the base instruction/token it qualifies (a "Note → applies at" mapping) and carries a worked resolved spawn example; `_template.md` documents both as required overlay sections.
- [x] Every `{placeholder}` token has a **documented backend-neutral default** specified as the legitimate neutral-floor behavior (the message-only / POSIX-universal / inherit-parent form), reconciled with the affirmative-writing rule as a correct default, not an error-swallowing fallback.
- [x] An **automated checker** verifies token coverage (every base token defined in all three overlays and the default table), no orphan tokens (no overlay/default defines a token the base never uses), and note-binding consistency (every note anchor resolves; the note-anchor *token set* — tokens carrying ≥ 1 note — is identical across the three overlays, while per-token note count may differ); it excludes the documentation meta-tokens `{placeholder}` / `{token}` via a named ignore-set so it never flags a legitimate base token. The checker passes.
- [ ] **Behavioral validation passes** (fresh-session probes, mirroring 0000105/0000106): the claude monitor is spawned with literal `--model haiku`; codex/opencode members apply their no-task-list / safety-floor notes; no literal `{token}` escapes in any emitted command or message; an unknown-backend agent uses the documented default rather than a literal token or an invented flag. *(DEFERRED — fresh cross-backend sessions cannot run inside this execution; mirrors 0000106's deferred behavioral validation. The claude path is already evidenced by this execution: the Director spawned the monitoring member with literal `--model haiku` and emitted no literal `{token}` across the full run.)*
- [x] **Removal is complete:** no passive "substitute … as you read the base" application aside remains on any load-bearing surface; every entry point reads as resolve-then-act.
- [x] **No re-litigation of 0000106:** the Required-reading block, its per-reader-role scoping, and the reading gate are unchanged except for the row #1 read→resolve upgrade and the appended Resolve checkpoint. No mechanism in 0000106 is duplicated.

---

## Background

### The residual gap after 0000106

GitHub issue #139 ("Overlay is not well respected"): the backend-specific overlay rules at `skills/cafleet/reference/coding-agent/` are often ignored. Design 0000106 (Skill Linked-Content Enforcement) addressed the *reading* half — it made the overlay the first load-bearing read in every reader entry point, with the consequence "every `{placeholder}` in this skill stays unresolved — you emit literal `{monitor_model}` …". That landed: the overlay is now reliably read.

Reading is necessary but not sufficient. **Application is a distinct cognitive step**, and it is still framed passively. The post-0000106 `cafleet/SKILL.md` Required-reading block (`skills/cafleet/SKILL.md:18`) closes with the only application instruction in the file:

> As you read the base, substitute your overlay's value for each `{placeholder}` you encounter.

This is an in-stream aside with no checkpoint, no worked target, and no failure-mode cost beyond the row #1 "literal token" consequence. Three application failures survive it:

| # | Failure mode | Mechanism | Evidence in the overlay system |
|---|--------------|-----------|--------------------------------|
| A1 | **Literal token emitted** | "Substitute as you encounter" has no gate and no self-check at the point of emission, so a `{monitor_model}` reaches the spawn command unresolved. | `cafleet/SKILL.md:59` `--model {monitor_model}`; `cafleet/roles/member.md:3` `({permission_flags})`. |
| A2 | **Wrong / default value** | When the agent does substitute, nothing pins the value to the overlay, so it guesses a plausible default (spawns the monitor on its own model instead of `haiku` / `gpt-5.4-mini`). | `{monitor_model}` differs per backend (`haiku`, `gpt-5.4-mini`, `anthropic/claude-haiku-4-5`); a default guess is silently wrong. |
| A3 | **Note-line ignored** | The overlay's prose notes (below the value table) are bound to nothing — the agent that resolves the token value still misses the caveat. | codex.md "no harness task list — track … as cafleet messages"; opencode.md "the `--agent cafleet` safety floor shows no popup; a popup is a regression to escalate, not a decision point". |

### What 0000106 owns, and where 0000107 attaches

0000106 owns *whether the overlay is read* — the Required-reading block, the per-reader-role scoping, and the gate. 0000107 owns *whether, once read, the overlay is applied*. The attach point is exactly one row and one trailing aside: the overlay's Required-reading row #1 (upgrade its verb and consequence) and the passive "substitute as you encounter" sentence (replace it with the Resolve checkpoint). Everything else 0000106 built is untouched.

This is the same instruction-design lineage as 0000105 (auto-invocation) and 0000106 (linked-content reading): the fix is prose, placement, and a behavioral acceptance test. The one tooling addition (the consistency checker, below) is a **static data-contract guard**, not a runtime application enforcer — it keeps the overlay/base/template token set coherent so the resolve step always has complete data.

---

## Specification

The design has six coordinated parts. Parts 1–4 are instruction-design; Part 5 is the static checker; Part 6 is the acceptance test.

### Part 1 — The Resolve-then-act checkpoint (the spine)

`cafleet/SKILL.md` gains a new `## Resolve your overlay` sub-section immediately beneath the Required-reading block (the canonical definition; every other entry point points to the same three-beat behavior via a one-line directive — see Part 4). Canonical shape:

```markdown
## Resolve your overlay

You have read your overlay (Required-reading row #1). Before your first action, resolve it:

1. **Materialize values.** For every `{placeholder}` token you will use this session, take
   the concrete value from your overlay's table and use that literal value — never the brace
   token. Resolution order for each token: (i) your overlay's value; (ii) the documented
   default below, only if your overlay omits the token or you cannot identify your backend.
   Never a literal `{token}`, never an ad-hoc guess.
2. **Apply notes.** When you reach a base instruction named in your overlay's *Note → applies
   at* table, follow that note's caveat there (e.g. on codex, coordinate via cafleet messages,
   not a harness task list; on opencode, treat a permission popup as a regression to escalate,
   not a decision point).
3. **Self-check at emission.** A literal `{token}` in any command you run, any message you
   send, or anything you show the user is a defect — stop and resolve it before emitting.

### Documented defaults

Used only when your overlay omits a token or your backend is unknown. Each default is the
correct neutral-floor behavior — the form that functions on every backend — not a guess.

| Token | Documented default (overlay silent / backend unknown) |
|-------|-------------------------------------------------------|
| `{decision_surface}` | a Director-relayed operator message (a member always routes to the Director) |
| `{monitor_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly cost-suboptimal (see rationale) |
| `{permission_flags}` | describe the mode neutrally as "workspace-scoped auto-approval" — for prose uses only; spawn-flag construction never falls here (see rationale) |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the skill's `SKILL.md` + your overlay by absolute path |
```

**Why these defaults are legitimate (affirmative-writing reconciliation).** Absence of an overlay value is an expected, valid state — a not-yet-onboarded backend, or an agent unsure of its identity — with a well-defined correct behavior: degrade to the lowest-common-denominator form that works everywhere (message-only coordination, POSIX backgrounding, a neutral mode description). That is a documented default per `affirmative-writing.md` ("a default that *is* the correct behavior, where absence is an expected, valid state"), not a silent sentinel hiding a violated invariant. The defaults are backend-neutral by necessity: an agent that cannot name its backend cannot pick a backend-specific value, so the default floor lives in the base, not in any one overlay.

Two tokens carry a tradeoff the table cell only flags:

- **`{monitor_model}` is a safe-but-cost-suboptimal floor.** The token's *purpose* is the cheapest capable model (`_template.md`: "cheapest capable model for the monitor"), but no portable cheap-model literal exists — `haiku`, `gpt-5.4-mini`, and `anthropic/claude-haiku-4-5` are each backend-specific. So the only backend-neutral floor is "inherit the spawning Director's model," which guarantees the correct *behavior* (the monitor always spawns and runs) while accepting a possible *cost* regression when the Director runs a premium model. The cost-optimal value is the overlay's job; the default only guarantees function. This is exactly the affirmative-writing split: the default is the correct behavior (the monitor runs), not an error-swallowing sentinel.
- **`{permission_flags}` defaults only for prose uses, never for spawn-flag construction.** The flag *names* are backend-specific (`--permission-mode dontAsk` vs `--ask-for-approval never --sandbox workspace-write` vs `--agent cafleet`), so an agent that cannot name its backend cannot construct a flag string — and it never has to. The token appears in two shapes: a **prose description** (`roles/member.md:3` "you run in workspace-scoped auto-approval mode (`{permission_flags}`)"), where the neutral default "workspace-scoped auto-approval" is the correct rendering; and a **spawn-flag** position (`cafleet member create …`), which is a Director action that presupposes a resolved backend (the Director knows its own backend and passes `--coding-agent` / inherits it). The unknown-backend default therefore never feeds spawn-flag construction — that path always has a resolved overlay.

### Part 2 — Strengthen the overlay's Required-reading row #1 (extend 0000106)

In every reader entry point, the overlay row #1 changes from read to **read-and-resolve**, and its consequence cell names all three application-failure modes. The current `cafleet/SKILL.md:24` cell:

> every `{placeholder}` in this skill stays unresolved — you emit literal `{monitor_model}`, `{permission_flags}`, `{decision_surface}`

becomes (illustrative — wording adapts per entry point):

> you skip resolution — you emit a literal `{monitor_model}`, **or** guess a wrong/default value (spawn the monitor on the wrong model), **or** ignore a backend note (codex has no harness task list). Read **and resolve** it (see *Resolve your overlay*).

The gate sentence above the table (`cafleet/SKILL.md:18`) drops its passive trailing clause "As you read the base, substitute your overlay's value for each `{placeholder}` you encounter" and instead points at the Resolve checkpoint. This is a **replacement**, not an addition (removal-completeness): the passive aside leaves no residue.

### Part 3 — Bind every overlay note to the base instruction it qualifies

Each overlay's free-prose notes become a **Note → applies at** table, so a note is no longer a floating caveat but a directive tied to a concrete base location/token. New required overlay structure (after the value table):

`claude.md`:

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` ≤ 4 options/question; built-in "Other" is the free-text path; question-shape taxonomy (choice / approve-or-revise / continue-or-abort / draft-comparison) | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| Decision-prompt frame spans ~120–200 lines — bump `cafleet member capture --lines` | `{decision_surface}` relay — `cafleet/reference/director.md` § Answering a member's relayed question |
| Relay a member's question via `cafleet member send-input --choice N \| --freetext` | `{decision_surface}` relay — same |
| `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet` register/claim/complete/check semantics | `{task_coord}` — `cafleet-research/report/report.md` task coordination |

`codex.md`:

| Note | Applies at |
|------|-----------|
| No in-pane prompt — send a concrete, answerable question to the Director, not free-form prose | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| No harness task list — track registrations/claims/completions as cafleet messages | `{task_coord}` — `cafleet-research/report/report.md` task coordination |

`opencode.md`:

| Note | Applies at |
|------|-----------|
| No in-pane prompt — send a concrete question to the Director; the `--agent cafleet` safety floor shows no popup, and a popup that appears is a regression to escalate, not a decision point | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| No harness task list — track registrations/claims/completions as cafleet messages | `{task_coord}` — `cafleet-research/report/report.md` task coordination |

The exact base anchors are validated against the codebase during implementation (the audit in Background cites the current sites). `_template.md` documents the Note → applies-at table as a required overlay section, with the rule "every note names the base token/instruction it qualifies."

### Part 4 — Worked resolved examples, and the per-entry-point resolve directive

**Worked examples.** Each overlay gains a "Worked resolution" line giving the canonical monitor-spawn command fully resolved for that backend, so the agent has a concrete string to match rather than a transformation to invent:

- `claude.md`: `cafleet member create --role monitor --model haiku` (members spawned `--permission-mode dontAsk`).
- `codex.md`: `cafleet member create --role monitor --model gpt-5.4-mini` (members spawned `--ask-for-approval never --sandbox workspace-write`).
- `opencode.md`: `cafleet member create --role monitor --model anthropic/claude-haiku-4-5` (members spawned `--agent cafleet`).

`_template.md` documents "Worked resolution" as a required overlay section.

**Per-entry-point resolve directive.** Every reader entry point in 0000106's inventory (all three families) carries the Part 2 row #1 upgrade plus a single self-contained sentence beneath its Required-reading block — no link, to avoid 0000106's glide-past failure:

> Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect.

The full Resolve checkpoint (procedure + default table) is defined once in `cafleet/SKILL.md`; the worked examples and bound notes live in each overlay, which row #1 already forces the reader to open. The per-entry-point sentence is the unmissable in-place reminder.

### Part 5 — Automated consistency checker (static data-contract guard)

A static checker keeps the overlay/base/template token set coherent so the resolve step always has complete data. It does **not** enforce runtime application (that is the behavioral probe's job) and it **never** flags a legitimate base `{placeholder}` token — the base keeping its tokens is the whole point of the backend-neutral design.

**Canonical token set:** the eight resolvable tokens `{decision_surface}`, `{monitor_model}`, `{permission_flags}`, `{bg_run}`, `{bg_stop}`, `{task_coord}`, `{pane_title}`, `{skill_loader}`.

**Ignore-set (two documented parts):** the checker's token universe is `matches of \{[a-z_]+\} in base files, minus the ignore-set`, where the ignore-set has two named, extensible parts:

1. **Documentation meta-tokens** — `{placeholder}` and `{token}` name the token *mechanism* in prose (e.g. `cafleet/SKILL.md:18,24`; Part 4's per-entry-point directive deliberately writes "a literal `{token}` … is a defect" into every base file), not resolvable values.
2. **Non-resolvable template/code tokens** — the 13 brace spans the *base markdown legitimately contains that the overlay never resolves*: the CLI `str.format()` spawn-prompt kwargs (`{fleet_id}`, `{agent_id}`, `{director_agent_id}`, `{coding_agent}`), workflow/doc placeholders (`{slug}`, `{dir_path}`, `{upstream}`), the web-researcher query-template tokens (`{topic}`, `{current_year}`, `{current_month}`), and fenced-code / LaTeX placeholders (`{output_path}`, `{script_stem}`, `{n}`). These are substituted by the CLI, a template engine, or example code — not by the overlay — so they are not resolvable tokens. (Brace spans inside non-markdown files such as the slidev `.vue` components — e.g. JS template literals `${type}` / `${p}` — are outside the checker's `*.md` base scan entirely and need no ignore entry.)

Both parts are explicit module-level frozensets (the meta-token set is public — pinned by a test; the non-resolvable set is documented inline with the reason each token is not overlay-resolvable). A base brace match outside the canonical set *and* both ignore-set parts fails the check — the design's intended "deliberate human triage": a genuinely new resolvable prose token is added to the overlays, a new template/code token is added to the non-resolvable set, never a silent pass. This is the concrete mechanism behind SC5 ("never flags a legitimate base `{placeholder}` token") — the exclusion is documented here, not merely asserted in the criterion.

**Checks:**

1. **Token coverage.** Every token in the checker's token universe (matches minus the ignore-set) is in the canonical 8-token set, is defined in all three overlay value tables, and has a row in the base default table. A brace match outside both the canonical set and the ignore-set fails the check — catching either a newly introduced resolvable token that lacks overlay coverage or a new meta-token that must be added to the ignore-set (a deliberate human decision, not a silent pass).
2. **No orphan tokens.** Every token defined in an overlay value table or the default table appears in at least one base file (catches a token removed from the base but left in an overlay).
3. **Note-binding integrity.** Every overlay note's "applies at" anchor names a token in the canonical set (and, where it cites a file/section, that file exists); the **note-anchor set** — defined as *the set of tokens that carry ≥ 1 note* in an overlay — is identical across the three overlays (all three = `{decision_surface, task_coord}`), so no backend silently drops a caveat another carries. Per-token note *count* may differ (claude legitimately carries extra `{decision_surface}` rows for `capture --lines` and `send-input`); the check compares the token set, not note multisets.

**Implementation.** A reusable parser plus a pytest test under `cafleet/tests/coding_agent/test_overlay_coverage.py` (the repo's existing consistency guards — e.g. `tests/cli/test_help_budget.py` — are pytest tests, so this runs under `mise //cafleet:test` in CI). A thin `mise //cafleet:lint-overlay` task wraps the same checker for ad-hoc runs. The parser locates the skill root relative to the repo root; token extraction is the regex `\{[a-z_]+\}` over base files, minus the two-part ignore-set (meta-tokens + non-resolvable template/code tokens), with the canonical-set membership check catching any newly introduced resolvable token that lacks overlay coverage.

### Part 6 — Behavioral validation

Fresh-session probes are the runtime acceptance test (mirroring 0000105 Step 4 and 0000106 Step 6): open a fresh coding-agent session, give a triggering prompt, confirm the reader emits resolved values and applies the bound note. Pass = resolved/applied; fail = literal token, wrong value, or ignored note. Enumerated in Implementation Step 6.

### Out of scope

- Re-litigating 0000106's Required-reading block, its per-reader-role scoping, or the reading gate (only row #1's read→resolve upgrade and the trailing-aside replacement touch it).
- Runtime enforcement of application (e.g. a harness hook that rejects literal-token commands) — efficacy is confirmed behaviorally, consistent with 0000105/0000106; the only tooling is the static coverage checker.
- The human-facing operator docs `docs/reference/coding-agents/` (independent home; never cross-linked from skills, per `.claude/rules/coding-agent-overlay.md` § Two independent homes).
- Frontmatter `description` auto-invocation triggers (owned by 0000105).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation FIRST (project rule): Step 1 lands before any skill-body edits.

### Step 1: Documentation

- [x] Update `.claude/rules/coding-agent-overlay.md` § *How the base and overlay connect*: replace "applies the overlay's deltas on top of every base instruction" with the resolve-then-act model — read → materialize every token to its value → apply each note at its bound base instruction → self-check that no literal `{token}` escapes. Add the documented-default contract (legitimate neutral-floor default, reconciled with affirmative-writing) and state that the static checker guards overlay/base/template token coverage. <!-- completed: 2026-06-21T09:48 (Director — member .claude/ edit blocked by harness) -->
- [x] Update `docs/concepts/token-reduction.md` § *Required-reading convention*: note that the overlay row #1 is **read-and-resolve** (not read-only), pointing one sentence at the resolve checkpoint as the application counterpart of the read gate. <!-- completed: 2026-06-21T09:45 -->
- [x] Check `README.md` for any skill-structure / overlay description; update if present (none exists today — confirm no change needed, or add one line if a structure section warrants). <!-- completed: 2026-06-21T09:45 -->

### Step 2: Define the canonical Resolve checkpoint in `cafleet/SKILL.md`

- [x] Add the `## Resolve your overlay` sub-section (Part 1) immediately beneath the Required-reading block: the three-beat procedure (materialize values / apply notes / emission self-check), the resolution order, and the backend-neutral documented-default table. <!-- completed: 2026-06-21T09:54 -->
- [x] Upgrade the overlay Required-reading row #1 (Part 2): change the verb to read-and-resolve and rewrite the consequence cell to name all three application-failure modes (literal token, wrong/default value, ignored note). <!-- completed: 2026-06-21T09:54 -->
- [x] Replace the passive trailing clause on `cafleet/SKILL.md:18` ("As you read the base, substitute your overlay's value for each `{placeholder}` you encounter") with the active pointer to *Resolve your overlay* — a replacement, leaving no residue (removal-completeness). <!-- completed: 2026-06-21T09:54 -->

### Step 3: Restructure the overlays and `_template.md`

- [x] Convert each overlay's free-prose notes into a **Note → applies at** table binding every note to the base token/instruction it qualifies (`claude.md`, `codex.md`, `opencode.md` — Part 3), verifying each anchor against the current base sites. <!-- completed: 2026-06-21T10:00 -->
- [x] Add a **Worked resolution** line to each overlay: the canonical monitor-spawn command fully resolved for that backend (Part 4). <!-- completed: 2026-06-21T10:00 -->
- [x] Update `_template.md` to document the *Note → applies at* table and the *Worked resolution* line as required overlay sections, with the rule "every note names the base token/instruction it qualifies." <!-- completed: 2026-06-21T10:00 -->

### Step 4: Propagate the resolve directive to every reader entry point

For each entry point: upgrade its overlay row #1 to read-and-resolve (matching consequence wording), add the one-line resolve directive beneath its Required-reading block, and replace any passive "substitute … as you encounter" aside it carries. Split per family (matching 0000106's per-family propagation Steps 3/4/5) for trackability and reviewable per-family commits.

- [x] `cafleet` family: `roles/director.md`, `roles/member.md`, `roles/monitor.md` (`SKILL.md` is already covered by Step 2). <!-- completed: 2026-06-21T10:06 -->
- [x] `cafleet-design-doc` family: `SKILL.md`, the three workflow bodies (`create/create.md`, `interview/interview.md`, `execute/execute.md`), and every `roles/*.md` under them. <!-- completed: 2026-06-21T10:18 -->
- [x] `cafleet-research` family: `SKILL.md`, both workflow bodies (`report/report.md`, `presentation/presentation.md`), and every `roles/*.md` under them. <!-- completed: 2026-06-21T10:30 -->

### Step 5: Automated consistency checker

- [x] Add the overlay-coverage checker: a reusable parser plus `cafleet/tests/coding_agent/test_overlay_coverage.py` asserting token coverage, no-orphan tokens, and note-binding integrity (Part 5); it must not flag legitimate base `{placeholder}` tokens. <!-- completed: 2026-06-21T10:46 -->
- [x] Add a `mise //cafleet:lint-overlay` task wrapping the same checker for ad-hoc runs; document it in `.claude/rules/commands.md`. <!-- completed: 2026-06-21T10:46 -->

### Step 6: Behavioral validation (fresh-session probes)

For each probe: open a fresh coding-agent session, give the triggering prompt, confirm resolved/applied behavior. Pass = resolved value emitted / note applied; fail = literal token, wrong value, or ignored note.

- [ ] **Token value (A1/A2):** a claude Director spawns the monitor with literal `--model haiku` (not `--model {monitor_model}` and not a guessed model). <!-- completed: -->
- [ ] **Note application (A3) — codex:** a codex member coordinates parallel work as cafleet messages (applies the "no harness task list" note), not via a harness task list. <!-- completed: -->
- [ ] **Note application (A3) — opencode:** an opencode member treats a permission popup as a regression to escalate (applies the safety-floor note), not as a decision point. <!-- completed: -->
- [ ] **Emission self-check:** across a fresh multi-step session, no literal `{token}` appears in any emitted command, message, or user-facing text. <!-- completed: -->
- [ ] **Default fallback (Q4):** an agent told its backend is un-overlaid (or that it is unsure) coordinates via cafleet messages and routes decisions to the Director (the documented defaults), rather than emitting a literal token or inventing permission flags. <!-- completed: -->
- [ ] Record outcomes in this doc; if any probe fails, strengthen that entry point's row #1 / directive (placement, wording) and re-run before marking complete. <!-- completed: -->

### Step 7: Removal sweep & consistency check

- [x] Grep the three skill families for residual passive application asides ("substitute … as you read the base", "as you encounter") on load-bearing surfaces; confirm each is replaced by the resolve directive. <!-- completed: 2026-06-21T10:52 -->
- [x] Confirm no entry point ships a literal-token example where a resolved value is required (the base legitimately retains `{placeholder}` tokens in instructional text; examples that model emission show resolved values). <!-- completed: 2026-06-21T10:52 -->
- [x] Confirm the overlay-coverage checker is green and run `mise //cafleet:lint` and `mise //cafleet:test`. <!-- completed: 2026-06-21T10:52 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-21 | Initial draft |
| 2026-06-21 | Implementation complete (Steps 1–5, 7). Resolve-then-act checkpoint, overlay restructuring, resolve directive across 26 entry points, and the static overlay-coverage checker (`mise //cafleet:lint-overlay`) landed; SC 1–5, 7, 8 satisfied. Step 6 (fresh-session cross-backend behavioral probes) deferred per the 0000105/0000106 precedent — the claude path is evidenced by this execution (monitor spawned with literal `--model haiku`, no literal tokens emitted). Shipped as PR #141 with a Copilot review loop (6 findings resolved, 2 declined as immutable audit artifacts) and a dedicated opus change-quality review pass (3 findings resolved, approved). Status → Complete on user instruction. |
