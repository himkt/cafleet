# Design request: coding-agent instruction overlays (0000099)

## Problem
The cafleet skill family (`skills/cafleet/` plus `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision`, `cafleet-design-doc-*`, `cafleet-research-*`) is written Claude-Code-first. Coding-agent-specific behavior is scattered inline across `SKILL.md` and `roles/*.md` as ad-hoc blockquote callouts and hardcoded values, which drifts and forces every reader to mentally subtract the Claude idioms. We want a base/overlay split: a backend-AGNOSTIC base instruction, plus a standalone per-coding-agent reference that overlays the agent-specific deltas.

## Decisions already made with the user (FIXED inputs — do NOT re-litigate; you MAY ask focused confirmation/scoping questions)

1. **Overlay home**: create NEW agent-facing overlay files at `skills/cafleet/reference/coding-agent/{claude,codex,opencode}.md`. These are the single canonical overlay folder; every cafleet-family skill points to it (siblings link via `../cafleet/reference/coding-agent/<name>.md`).

2. **Audience separation, NO cross-links**: `skills/cafleet/reference/coding-agent/` is for the coding AGENT; `docs/reference/coding-agents/{codex,opencode}.md` stays for HUMANS (operational CLI/sandbox detail). The two homes are independent and must NOT link to each other. As part of this refactor, replace the existing skill→docs links (e.g. `cafleet/SKILL.md` and `reference/director.md` currently link out to `docs/reference/coding-agents/codex.md` / `opencode.md`) with skill→overlay links to the new agent-facing files.

3. **Base layer is fully backend-NEUTRAL**. Push EVEN Claude's idioms (AskUserQuestion, the shape taxonomy, Task* tools, run_in_background/TaskStop, the Skill-tool loader, the `--model sonnet` hardcode) out of the base and into `claude.md`. The base prominently/early instructs every agent to read its own overlay at `reference/coding-agent/<name>.md` for whichever coding agent it is running on, and to apply it on top of the base. Emphasize this pointer so an agent always knows to consult the corresponding reference for its coding agent.

4. **Convention artifacts: BOTH** (a) a terse new `.claude/rules/coding-agent-overlay.md` normative rule written affirmatively (backend-agnostic base; agent-specific deltas live in `coding-agent/<name>.md`, linked via the overlay pointer) AND (b) an extension to the existing `skills/skill-author/SKILL.md` teaching the base/overlay pattern.

## Catalog of agent-specific content that must move from base into the per-agent overlays (verified by exploration)

- **AskUserQuestion**: the canonical user-reaction surface, shape taxonomy, escalation gate, standalone-vs-fleet delegation, and the `cafleet member send-input` 4-option pane frame (`--choice`/`--freetext` keystrokes). Currently in `cafleet/SKILL.md` "Soliciting user reactions" + `reference/director.md` three-beat workflow + callouts in monitoring/supervision `SKILL.md` and `design-doc-*` `roles/director.md`. Base neutral form: "solicit a recorded user reaction via your decision surface"; `claude.md` overlay names AskUserQuestion + taxonomy; codex/opencode overlays name their substitute (plain operator message / native surface) and the read-then-respond cadence.
- **Cheap monitoring-member model**: base currently hardcodes `--model sonnet` everywhere. Base neutral form: "spawn the monitor with the cheapest capable model for your backend." Per-agent overlay names the pick (claude → a cheap claude model; codex → a cheap gpt model, e.g. `gpt-5.x-mini` is cheaper than `gpt-5.5`; opencode → a provider/cheap-model id). This is the user's motivating pricing example.
- **Permission/auto-approval mode**: claude `--permission-mode dontAsk`; codex `--ask-for-approval never --sandbox workspace-write`; opencode `--agent cafleet`. Currently in `roles/member.md` and `design-doc-*` `roles/director.md`.
- **Background task + task-stop primitive**: Claude Code `run_in_background:true` / `TaskStop` / `TaskCreate`/`TaskUpdate`/`TaskList`/`TaskGet` vs the backend-native equivalents. Currently in `cafleet-research-presentation` and `cafleet-research-report` `SKILL.md` + roles.
- **Pane-title asymmetry**: claude `--name` sets pane title; codex/opencode discover via `cafleet member list`. Currently `reference/director.md`.
- **Skill-loader / agent-dispatch recipe**: "load via the Skill tool" / "your backend's skill-loader" / Claude-Agent-dispatch-vs-codex-plugin-auto-discovery. Currently in many `roles/*.md` and `cafleet-research-report` `SKILL.md`.

NOTE: the model-name-to-backend INFERENCE table in `reference/director.md` is a backend-AGNOSTIC selector mechanism — it STAYS in the base, it is NOT an overlay.

## Constraints the design doc must honor (project rules)

- **removal.md**: when content moves out of the base, DELETE it from the base cleanly — no "see claude.md for the old wording" deprecation residue, no historical callouts. After the refactor the base reads as if it were always backend-neutral.
- **affirmative-writing.md**: the new rule and overlay pointers must be written as positive specs (what to do), not piles of "do not".
- **design-doc-numbering.md**: documentation-first ordering — the implementation plan must update `docs/concepts` + `docs` + `README.md` + every affected `SKILL.md` + rules BEFORE any code. (This refactor is largely docs/skills/rules, minimal-to-no code, but the ordering and README/SKILL.md/concepts coverage still apply; consider whether a `docs/concepts` page describing the base/overlay split is warranted.)
- **Scope**: cover the WHOLE cafleet skill family in the design doc as the target spec; phasing/rollout can be a section, but the spec is the full family.

## Naming
- Design number: `0000099`, slug `coding-agent-instruction-overlays`.
- Output path: `design-docs/0000099-coding-agent-instruction-overlays/design-doc.md`.
