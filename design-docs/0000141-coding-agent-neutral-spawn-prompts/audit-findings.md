# Coding-agent-neutrality audit findings (2026-07-18)

Input for design doc 0000141. Conducted by the Director against the repo at
`/Users/himkt/work/himkt/cafleet` (branch `bump`, clean tree). Trigger: codex and
opencode members tried to access `~/.claude`, which is Claude-Code-private.

## 1. Root cause — spawn prompts order every member to read `~/.claude`

The execute-workflow role spawn prompts contain this verbatim IMPORTANT line, rendered
into the prompt of every Programmer / Tester / Verifier / Reviewer regardless of backend:

> `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol)
> and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.`

Occurrences (5):

- `skills/cafleet-design-doc/execute/execute.md:202` (Programmer delta)
- `skills/cafleet-design-doc/execute/execute.md:223` (Tester delta)
- `skills/cafleet-design-doc/execute/execute.md:246` (Verifier delta)
- `skills/cafleet-design-doc/execute/execute.md:363` (Reviewer delta)
- `skills/cafleet/reference/director.md:149` (lossless-rule summary: "All execute roles")

Problems:

- `~/.claude/rules/bash-command.md` is the operator's private Claude-Code-global config —
  not in the repo, does not exist for codex/opencode members, and prompts them to probe
  `~/.claude`.
- `.claude/rules/bash-tool.md` is a repo file but its content is Claude-specific
  (`--permission-mode dontAsk`, the `⏵⏵ don't ask on` status line, the `!` shortcut).
- The line bypasses the overlay architecture entirely (`.claude/rules/coding-agent-overlay.md`
  mandates backend specifics live in `skills/cafleet/reference/coding-agent/<name>.md`).

**User's direction**: replace the line with a pointer to the cafleet skill's
backend-neutral member Bash protocol. The protocol already exists, skill-shipped and
neutral, in `skills/cafleet/roles/member.md` (default rule, MUST-NEVER list, denial
handling) and `skills/cafleet/reference/exec-routing.md` (bash-via-Director fallback) —
and members already load the cafleet skill at startup per the canonical spawn skeleton.
Blast radius is skills-only: the line is not mirrored in `docs/` or `SPEC.md`.

## 2. Other agent-facing pages that hardcode `~/.claude` / Claude paths

- `skills/cafleet/reference/base-dir.md:5,9,37-39,45,49` — the `${BASE}` resolution
  procedure special-cases `~/.claude` (`claude_subdir = $HOME/.claude`), offers
  `/tmp/claude-code` as the scratch candidate, says "Claude's job is to…", and names
  `AskUserQuestion` directly instead of the `{decision_surface}` token.
  `skills/cafleet-design-doc/execute/execute.md:82` repeats the `~/.claude` /
  `/tmp/claude-code` edge case.
- `skills/cafleet-research/reference/slidev.md:11-13` — plugin-install-dir discovery
  hints name Claude Code (`~/.claude/plugins/cache/...`, `claude plugin list`) and Codex,
  with no opencode row.

## 3. Backend-neutrality violations in base skill bodies (no `~/.claude`, but Claude-first wording)

Per `.claude/rules/coding-agent-overlay.md`, base bodies must read the same on any backend.

- "Your `claude` process is terminated" — termination boilerplate in 13 role files:
  `execute/roles/{programmer,tester,verifier,reviewer}.md`,
  `create/roles/{drafter,reviewer}.md`, `interview/roles/analyzer.md`,
  `report/roles/{manager,researcher,scout}.md`,
  `presentation/roles/{presentation,transcript,visual-reviewer}.md`.
  → "your coding-agent process".
- "spawn subagents or run `claude` commands" in the Do-NOT lists of
  `execute/roles/programmer.md:37`, `tester.md:37`, `verifier.md:37`
  → neutral wording (e.g. "run coding-agent CLI commands").
- `execute/roles/tester.md:47` "Check project's `CLAUDE.md`" — codex/opencode read
  `AGENTS.md` → "your harness's project-instructions file (`CLAUDE.md` / `AGENTS.md`)".
- Role tables / org charts saying "Main Claude" / "claude pane (member)" / "(main Claude …)":
  `interview/interview.md:22,42`, `execute/execute.md:22,48`, `create/create.md:22-24,46`,
  `report/report.md:21-24,44-47`, `presentation/presentation.md:21-24,38-41`
  → "Main agent" / "member pane".
- `cafleet-research/reference/visualization.md:12` "Bash calls in Claude Code are
  ephemeral" → neutral ("in coding-agent harnesses").
- `cafleet-design-doc/reference/guidelines.md:55` "complete enough that Claude can
  implement" → "an agent".
- `cafleet/reference/director.md:13` spawn example `--name Claude-B` → neutral name.

## 4. Borderline (flagged, arguably fine)

- References to the repo-committed `.claude/rules/` directory in
  `presentation/presentation.md:198-201`, `reference/visualization.md:4,72`,
  `execute/roles/reviewer.md:23`. Any backend can Read a repo path, and the project's
  `commands.md` codifies `.claude/rules/` as the host-project rules home; still,
  phrasing like "the host project's agent rules directory" would be layout-agnostic.

## 5. By design — NOT violations (keep)

- Three-backend enumerations and per-backend delta pointers in `cafleet/reference/cli.md`,
  `director.md`, `exec-routing.md`, `roles/member.md`, `reference/supervision.md`.
- "e.g. Claude Code → `claude`" identity-mapping examples on `--coding-agent`.
- Per-backend model tables in `reference/director.md`.
- The `COMMENT(claude)` / `FIXME(claude)` marker-role grammar (a fixed role token defined
  in `cafleet-design-doc/reference/coordination.md`; renaming is out of scope unless the
  user asks).

## 6. Harness observation (separate decision)

`skills/cafleet/reference/coding-agent/claude.md` collides with `CLAUDE.md` on
case-insensitive filesystems (macOS default): Claude Code auto-injects it as a
project-instructions file for any session touching that directory. Fixing it requires a
deliberate change to the deterministic `coding-agent/<name>.md` lookup scheme.

## 7. Simplification angle (user explicitly requested)

The user asked the design doc to also cover improvements/simplifications of the current
skills, spawn prompts, and documentation discovered during this audit — e.g. the
IMPORTANT-line replacement is also a simplification (one neutral pointer instead of two
rule-file paths; the member Bash protocol is already required reading via the cafleet
skill, so the spawn-prompt line may arguably shrink to a reminder or be dropped from the
per-role deltas and stated once in the canonical skeleton / lossless rule).
