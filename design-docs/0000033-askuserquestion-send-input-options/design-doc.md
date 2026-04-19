# Director delegates member send-input choices via AskUserQuestion

**Status**: Complete
**Progress**: 16/16 tasks complete
**Last Updated**: 2026-04-19

## Overview

Replace the current "Director pre-guesses one `cafleet member send-input` message body and hands the command to the user" pattern with a three-beat delegation: (1) `cafleet member capture` to inspect the member's pane, (2) `AskUserQuestion` to present 2–4 complete candidate bodies (or the member's own labelled choices) to the user, (3) Director invokes `cafleet member send-input` itself via its Bash tool so Claude Code's native permission prompt surfaces the action to the user. The permission model is unchanged — `cafleet` and the broker are untouched. This is a documentation-only change to skill files and project docs.

## Success Criteria

- [x] The Director never prints a "please run this command in your shell" instruction block for `member send-input`. All operator consent flows through Claude Code's Bash permission prompt instead.
- [x] Every canonical documentation source for the send-input workflow (global skills, project-local skill copies, project README/ARCHITECTURE/docs) describes the new three-beat shape: `member capture` → `AskUserQuestion` → direct Bash invocation of `cafleet member send-input`.
- [x] Both pane-prompt shapes are covered: open-ended AskUserQuestion (2–4 candidate bodies, submit via `--freetext`) and choice-routing AskUserQuestion (mirror the member's own labelled options, submit via `--choice N`). A third "Other shapes" row explicitly forbids using `send-input` outside the AskUserQuestion 4-option frame.
- [x] `AskUserQuestion` per-call limits (1–4 questions, 2–4 options each, built-in "Other" — no explicit "Write my own" option) are enforced in the written spec, and 5-or-more-body scenarios are explicitly handled by narrowing to 2–4 candidates BEFORE asking (not by paginating across sequential calls).
- [x] A grep across all updated files finds no remaining instruction to the Director to print a fenced `bash` block with `cafleet ... member send-input ...` for the user to copy-paste.

---

## Background

Today, when the Director (either in `/design-doc-create`, `/design-doc-execute`, or any ad-hoc CAFleet coordination) sees a member paused on an `AskUserQuestion`-shaped prompt, the `cafleet` skill documents this flow:

1. `cafleet member capture` to see the prompt.
2. `AskUserQuestion` to ask the user (non-canonical wording — the skill leaves the question text open).
3. `cafleet member send-input` either invoked by the Director or printed for the user to run.

In practice, the Director has drifted into "pre-guess a single freetext body, print the resolved `cafleet member send-input --freetext "<body>"` command, ask the user to run it." That path commits the Director to a specific wording before the user has a chance to weigh alternatives. The user's stated principle is: **the Director does not need to guess — it should just ask the user as-is**.

The fix is to make the delegation explicit (multiple candidate bodies, or the member's own labelled options, relayed through `AskUserQuestion`) and to stop the copy-paste hand-off (the Director invokes the resolved command itself via Bash, and Claude Code's per-tool permission prompt is what gates user consent). No `cafleet` CLI or broker code needs to change — `member send-input` already exposes the right flags, and the authorization boundary it enforces is unchanged.

---

## Specification

### When the pattern applies

Any time the Director would forward a keystroke to a member's pane via `cafleet member send-input` — whether the member's AskUserQuestion is choice-routing (the labelled options 1/2/3 are the decision point) or open-ended (the operator wants to send a free-form body) — the Director MUST delegate the decision to the user via `AskUserQuestion` first. There is no "obvious enough to pick silently" exception. (The third "Other shapes" pane state is excluded — `send-input` does not apply there at all; see the pane-shapes table.)

The Director never decides the body, the choice digit, or the custom free-text on the user's behalf.

### Pane prompt shapes

The member's pane, as revealed by `cafleet member capture`, is ALWAYS on the AskUserQuestion 4-option frame (`1. …`, `2. …`, `3. …`, `4. Type something`) when `send-input` is appropriate — `send-input --freetext` itself sends a literal `4` keystroke first to route into the "Type something" slot, so any pane that is not on that frame will be corrupted by a send-input call. Two usage shapes apply on top of that frame; a third row covers everything else.

| Shape | Member pane looks like | Director's AskUserQuestion options | Resolved send-input call |
|---|---|---|---|
| **Choice-routing** | AskUserQuestion where the labelled options `1. …`, `2. …`, `3. …` ARE the decision point (option labels are meaningful to the user). | Mirror UP TO 3 of the member's labels as AskUserQuestion options. `label` holds the member's short label; `description` holds the member's description if visible in the capture. AskUserQuestion's built-in Other handles custom freetext — do NOT add an explicit 4th option, since `--choice` is `IntRange(1, 3)` and only the CLI's built-in 4-slot routes through `--freetext`. | If the user picked mirror option N (1, 2, or 3), `--choice N`. If the user picked built-in Other and typed a custom body, `--freetext "<typed>"`. |
| **Open-ended** | AskUserQuestion where the labelled options `1. …`, `2. …`, `3. …` are NOT useful for this situation (the member is effectively waiting for free-form instruction). The 4-option frame itself still renders — that frame is exactly what `send-input --freetext` submits through. | 2–4 *complete candidate message bodies*. `label` is a short intent tag (≈12 chars, e.g. `Direct nudge`, `Soft check-in`, `Strict redirect`). `description` holds the FULL draft body so the user can compare wording side-by-side. Built-in Other is the typed-custom-body path. | `--freetext "<picked body>"` when the user picked one of the drafts, or `--freetext "<typed>"` when the user picked built-in Other. |
| **Other shapes** | Pane is NOT on an AskUserQuestion — e.g. mid-command, Codex idle REPL, crashed, awaiting a yes/no confirmation, or mid tool-call. | Do NOT call AskUserQuestion and do NOT call `send-input`. The `send-input` CLI is validated only for the AskUserQuestion 4-option frame; sending a `1`, `2`, `3`, or `4` keystroke into any other shape will corrupt pane state. | None. Escalate to the user via a regular `cafleet send` nudge, or wait for the member to return to an AskUserQuestion prompt. |

### `AskUserQuestion` constraints

| Rule | Value |
|---|---|
| Questions per call | 1–4 |
| Options per question | 2–4 |
| Built-in "Other" | Always exposed by the tool itself. DO NOT add an explicit "Write my own" / "Custom" option. |
| ≥ 5 candidate bodies | Narrow to 2–4 BEFORE asking. Heuristic: drop duplicates and near-duplicates (same intent, different wording), then pick the highest-contrast subset spanning the decision axes (tone, specificity, action). Do NOT paginate across sequential AskUserQuestion calls — each call is a disjoint decision, not a page of a larger list. |
| Preamble text above the question | None. Rely on the `cafleet member capture` output the Director already printed this turn, plus the `AskUserQuestion` question text, to carry all context. |

### Director action shape (end-to-end)

| Step | Actor | Action |
|---|---|---|
| 1 | Director | Detect the stalled member via `cafleet poll` or the `/loop` health check, then run `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 120`. `120` is the recommended default. Re-run with `--lines 200` only if the first capture is truncated above the AskUserQuestion frame (i.e. the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible). |
| 2 | Director | Classify the pane shape (choice-routing vs open-ended vs other) from the capture output per the pane-shapes table. If the shape is "other," abort the flow at this step and escalate via a regular `cafleet send` nudge or wait. |
| 3 | Director | Call `AskUserQuestion` with the shape-appropriate options per the table above. Question text names the member and summarizes what it's paused on (e.g. `Drafter is paused on AskUserQuestion — which reply should I send?`). |
| 4 | User | Picks one of the 2–4 options, or uses the built-in "Other" to type a custom body. |
| 5 | Director | Resolve the command arguments: `--choice N` for a labelled choice-routing pick, otherwise `--freetext "<text>"`. |
| 6 | Director | Invoke the resolved command via its Bash tool: `cafleet --session-id <session-id> member send-input --agent-id <director-agent-id> --member-id <member-agent-id> (--choice N \| --freetext "<text>")`. |
| 7 | Claude Code | Surfaces its native Bash permission prompt to the user (Yes / No). |
| 8 | User | Approves or denies at the permission prompt. No copy-paste; no fenced instruction block to run the command manually. |

### What the Director MUST NOT do

- Pre-draft a single body and tell the user to run the command themselves ("please paste this…").
- Print a fenced `bash` code block containing the resolved `cafleet member send-input` invocation.
- Add a one-line preamble sentence above the `AskUserQuestion` (the capture output plus the question text is enough).
- Add an explicit "Write my own" / "Custom" option to the `AskUserQuestion` payload (the built-in "Other" handles it).
- Silently decide a `--choice` digit, even when the member's labels appear obvious.
- Mix shapes: never send `--choice N` on an open-ended pane, and never default to `--freetext` on a choice-routing pane. The shape classification from `cafleet member capture` determines which flag to use; never invert.
- Call `send-input` when the pane is on an "Other shapes" state per the table above. Escalate or wait instead — sending any keystroke would corrupt pane state.

### Out of scope

- No changes to the `cafleet` CLI, the broker, `tmux` helpers, or `click` surfaces. `member send-input` keeps its current flags, validation rules, exit codes, and authorization boundary.
- No change to the project's Bash-tool `permissions.allow` patterns. The existing pattern that matches `cafleet --session-id <literal-uuid> member send-input ...` already covers Director-side invocation; Claude Code's per-call permission prompt is the user-consent surface.
- No change to the member-side `AskUserQuestion` tool or how members render prompts — this is purely a Director-side delegation discipline.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

All steps are documentation updates. No CLI, broker, or test code changes. The project rule in `.claude/rules/design-doc-numbering.md` requires updating documentation first — that's the entire surface here.

### Step 1: Update the canonical cafleet skill (global + project copy)

Rewrite the section titled `Answer a member's AskUserQuestion prompt` so it specifies the new three-beat shape (capture → `AskUserQuestion` with shape-matched options → direct Bash invocation). Add the choice-routing / open-ended / other-shapes table verbatim from the Specification. Remove the old 3-step list that implied the Director might just pick an option. Note the 1–4 questions / 2–4 options / built-in "Other" rule and the narrow-before-asking heuristic for ≥ 5 candidates.

- [x] Update `~/.claude/skills/cafleet/SKILL.md` — locate the section whose heading is `Answer a member's AskUserQuestion prompt` (currently near the end of the file, after `Member Send-Input`) and rewrite it. Line numbers are a non-authoritative hint only; match by heading text. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/skills/cafleet/SKILL.md` — apply the same rewrite to the project-local copy. After editing, `diff` the two files and confirm the new section is identical in both (any remaining differences must be pre-existing project customizations, not differences in the new section). <!-- completed: 2026-04-19T00:00 -->

### Step 2: Update the cafleet-monitoring skill (global + project copy)

Keep Stage 2 of the health check pointing at the same general mechanism, but update the short explanatory sentence and the escalation-table row for `send-input` so they reference the new `AskUserQuestion`-delegated pattern rather than implying the Director can forward a keystroke on its own judgment.

- [x] Update `~/.claude/skills/cafleet-monitoring/SKILL.md` — locate (a) the Stage 2 paragraph that introduces `send-input` as the unblock mechanism for AskUserQuestion-shaped pauses, and (b) the row of the escalation table whose command cell contains `member send-input`. Update both to reference the AskUserQuestion-delegated pattern and to cross-link the cafleet skill as canonical. Match by content, not line number. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/skills/cafleet-monitoring/SKILL.md` — apply the same edits to the project copy. `diff` after edit to confirm parity of the changed regions. <!-- completed: 2026-04-19T00:00 -->

### Step 3: Update the Director role files for design-doc-create and design-doc-execute

Add a short "User delegation for member send-input" note to each Director role file under the existing Communication Protocol / Progress Monitoring section. Two sentences are enough: point at the cafleet skill as canonical, and reiterate the "never print a fenced command for the user to paste" rule so the Director doesn't backslide. Do NOT duplicate the full table — the cafleet skill is the canonical home.

- [x] Update `~/.claude/skills/design-doc-create/roles/director.md`. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `~/.claude/skills/design-doc-execute/roles/director.md`. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/skills/design-doc-create/roles/director.md` — mirror the global edit. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/skills/design-doc-execute/roles/director.md` — mirror the global edit. <!-- completed: 2026-04-19T00:00 -->

### Step 4: Update project README, ARCHITECTURE, and CLI docs

Only touch the places that already mention `member send-input`. The edits are small — a single sentence or bullet that says "Director-side workflow: AskUserQuestion delegation, then direct Bash invocation; see `skills/cafleet/SKILL.md` for the canonical table." No need to repeat the full table here.

- [x] Update `/home/himkt/work/himkt/cafleet/README.md` — locate (a) the Features-list bullet that introduces the `member send-input` command, and (b) the command-table row whose Command column contains `member send-input`. Extend each with a one-sentence pointer to the delegated-choice pattern. Match by content. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/ARCHITECTURE.md` — locate (a) the CLI→broker mapping row whose Command column is `member send-input`, and (b) the member-commands paragraph that enumerates the member subcommands and mentions `send-input`. Add a one-sentence note on the Director-side delegation workflow to each. Match by content. <!-- completed: 2026-04-19T00:00 -->
- [x] Update `/home/himkt/work/himkt/cafleet/docs/spec/cli-options.md` — locate the subsection whose heading is `### member send-input` and append a new paragraph headed "Director-side usage pattern" that describes the three-beat shape (capture → AskUserQuestion → direct Bash invocation) and cross-links `skills/cafleet/SKILL.md` (by relative repo path) as canonical. Match by heading text. <!-- completed: 2026-04-19T00:00 -->

### Step 5: Verify and sign off

- [x] Phrase-level negative grep: across every file updated in Steps 1–4, run `Grep "please run this"`, `Grep "paste this command"`, `Grep "run this in your shell"`, and `Grep "copy and paste"` (case-insensitive). Confirm zero hits. <!-- completed: 2026-04-19T00:00 -->
- [x] Structural negative grep: across every file updated in Steps 1–4, run a multiline `Grep` with `multiline: true` for the pattern `` ```bash[\s\S]*?member send-input[\s\S]*?``` `` to find any fenced Bash block containing a `member send-input` invocation. For each hit, confirm it is a documentation-style CLI reference (e.g. the canonical syntax block in the cafleet skill's `Member Send-Input` section) and NOT an instruction telling a user to run the block. Any block that reads as an instruction must be rewritten. <!-- completed: 2026-04-19T00:00 -->
- [x] Run `Grep "send-input"` across all updated files. Walk each match to confirm it now reflects the new pattern (AskUserQuestion delegation + direct Bash invocation) or is a neutral reference (e.g. the recovery hint in `member delete`'s timeout message, which is untouched by this design). <!-- completed: 2026-04-19T00:00 -->
- [x] Diff each global ↔ project-local skill pair (`cafleet/SKILL.md`, `cafleet-monitoring/SKILL.md`, both `director.md` files) and confirm the new sections are byte-identical or differ only in pre-existing project customization. Record any intentional divergences in the Changelog below. <!-- completed: 2026-04-19T00:00 -->
- [x] Walk through three end-to-end scenarios — (1) choice-routing pane, (2) open-ended pane, (3) other shape — using ONLY the updated cafleet skill as the reference. For each scenario confirm, in order: (a) the capture output classifies the pane shape correctly per the pane-shapes table; (b) for scenarios 1 and 2, the AskUserQuestion shown has the expected option count and expected label/description mapping; (c) for scenarios 1 and 2, Claude Code's Bash permission prompt shows the fully-resolved `cafleet --session-id ... member send-input --agent-id ... --member-id ... (--choice N | --freetext "...")` command with no placeholders; (d) for scenarios 1 and 2, after approval the pane receives the keystroke and the member resumes; (e) for scenario 3, the Director does NOT call AskUserQuestion or send-input and instead escalates via regular `cafleet send` or waits. <!-- completed: 2026-04-19T00:00 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-19 | Initial draft |
| 2026-04-19 | Reviewer pass 1: corrected pane-shapes table (open-ended row no longer says "no enumerated choices"; choice-routing row clarifies max 3 mirror options because `--choice` is `IntRange(1, 3)`); added third "Other shapes" row forbidding `send-input` outside the AskUserQuestion frame; unified terminology on "three-beat delegation"; replaced ≥ 5-bodies pagination with a narrow-before-asking heuristic; dropped the unclear "pre-approved" out-of-scope bullet; added shape-mixing and wrong-shape rules to "MUST NOT do"; replaced line-number anchors in implementation tasks with section-header anchors; added a structural multiline grep verification task; expanded the walkthrough task into observable checkpoints across three scenarios; bumped progress to 0/16. |
| 2026-04-19 | User approval. Status moved to Approved. Finalization sweep: SC item 4 reworded to align with the narrow-before-asking spec (no pagination); "When the pattern applies" prose updated to use the choice-routing / open-ended terminology from the pane-shapes table; Director-action-shape Step 2 now references all three shapes (including "other," which aborts the flow) and Step 5 uses the canonical choice-routing label. All 16 implementation tasks re-verified as actionable and grep-anchored: Step 1 (2 tasks, anchored by the heading "Answer a member's AskUserQuestion prompt"), Step 2 (2 tasks, anchored by Stage 2 paragraph + escalation-table row containing the CLI name), Step 3 (4 tasks, mirror edits across global / project Director role files), Step 4 (3 tasks, anchored by Features bullet, command-table row, and `### member send-input` heading), Step 5 (5 tasks: phrase grep, structural multiline grep, send-input walk, global↔project diff, three-scenario walkthrough). |
| 2026-04-19 | PR #35 opened; one Copilot review round addressed — cli-options.md gained a Superseded note above the pre-existing `#### Typical Director workflow` subsection pointing readers at the canonical `#### Director-side usage pattern`, and cafleet-monitoring Stage 2 example (global + project) switched to `--lines 120` default with `--lines 200` noted as a truncation fallback. Second Copilot review pass returned `state: "COMMENTED"` with zero new inline comments and body text "generated no new comments" — treated as functional approval with user sign-off (the Copilot reviewer bot does not issue `APPROVED` state on this repo). Status moved to Complete. |
| 2026-04-19 | Step 5 verification complete. Progress 16/16. **Task 1 (phrase-level grep)**: PASS — every "please run this" / "run this in your shell" / "copy-paste" hit is inside a NEGATIVE RULE ("Do NOT print a fenced `bash` block for the user to copy-paste, and do NOT add 'please run this in your shell' instructions" in the new three-beat section, plus the mirrored rule in the four Director role notes). Zero hits for "paste this command" or "copy and paste" (literal). **Task 2 (structural fenced-bash grep)**: the regex `` ```bash[\s\S]*?member send-input[\s\S]*?``` `` over-matched due to non-greedy across-block behavior, so walk was combined with Task 3. Only two fenced-bash blocks contain `member send-input`: the canonical `### Member Send-Input` syntax block in `~/.claude/skills/cafleet/SKILL.md` (≈lines 374–382) and its project-copy counterpart in `skills/cafleet/SKILL.md` (≈lines 417–425). Both are documentation-style CLI references, NOT instructions to run. PASS. **Task 3 (send-input walk)**: all hits reflect the new delegated pattern or are neutral references — the `member delete` timeout recovery hint in `docs/spec/cli-options.md` (line 324) and the `member delete --force` escalation-table row in the project `cafleet-monitoring` skill (line 84) remain neutral and out-of-scope for this design, as expected. One flagged semi-stale artifact: `docs/spec/cli-options.md` lines 430–432 (inside the pre-existing `#### Typical Director workflow` subsection) still imply the Director may pick the arg itself; this subsection is SUPERSEDED by the new `#### Director-side usage pattern` paragraph the design appended, which cross-links `skills/cafleet/SKILL.md` as canonical. Director's Step 4 scope was explicitly "append a new paragraph" (not rewrite), so leaving the old subsection in place is intentional; a follow-up cleanup pass may tighten it. **Task 4 (global↔project diff)**: `cafleet/SKILL.md` — new "Answer a member's AskUserQuestion prompt" section does NOT appear in the diff, confirming byte-identical parity; pre-existing divergences remain (frontmatter `name` line, Required-Flags intro wording, project-only `### --version` / `### Doctor` / `### Shutdown Protocol` / member-delete `--force`, global-only Passing-multi-line-text subsection and tmux-list-panes example, `director` vs `Director` casing). `cafleet-monitoring/SKILL.md` — new Stage-2 paragraph and new escalation-table `send-input` row are byte-identical in both copies; pre-existing divergences remain (description frontmatter, agent-agnostic 15 s-timeout note in project only, Interval-enforcement block in global only, Lifecycle-rule wording, extra `member delete --force` escalation row in project, `/loop` prompt preamble). `design-doc-create/roles/director.md` and `design-doc-execute/roles/director.md` — the new `### User delegation for member send-input` subsection is byte-identical in every pair; only pre-existing title and opening-paragraph customizations ("CAFleet-native", "orchestrated via the CAFleet message broker", "persisted in SQLite and visible in the admin WebUI timeline") differ. PASS. **Task 5 (three-scenario walkthrough)**: (a) Choice-routing — classification from labelled 1/2/3 rows, AskUserQuestion mirrors up to 3 member labels as `label`+`description`, resolved invocation is `--choice N` (1/2/3) or `--freetext "<typed>"` for built-in Other; (b) Open-ended — classification from an unhelpful 1/2/3 + user wants free-form, AskUserQuestion shows 2–4 complete candidate bodies with ≈12-char intent-tag `label` and full-body `description`, resolved invocation is `--freetext "<picked body>"` or `--freetext "<typed>"` for Other; (c) Other shape — Director does NOT call AskUserQuestion and does NOT call `send-input`, instead escalates via a regular `cafleet send` nudge or waits. PASS. Awaiting Director sign-off and commit. |
