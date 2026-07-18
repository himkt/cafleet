# Deny-by-Default Bash Allowlist for the opencode Preset

**Status**: Complete
**Progress**: 8/8 tasks complete
**Last Updated**: 2026-07-18

## Overview

Flip the `bash` permission map in `presets/opencode/cafleet.md` from allow-by-default with a specific deny-list to deny-by-default with an explicit allowlist translated one-to-one from the operator's Claude Code `~/.claude/settings.json` `permissions.allow` `Bash(...)` entries. The never-`ask` doctrine is preserved — every check resolves to `allow` or `deny` — and all posture documentation (SPEC.md verbatim contract, backend spec, concepts page, skills) is updated in the same cycle.

## Success Criteria

- [x] The `bash` map in `presets/opencode/cafleet.md` is exactly: the `"*": "deny"` base, the allowlist in § *New preset file content*, and the two `cafleet member exec` deny overrides — no `"ask"` values, and none of the previous explicit deny entries (`bash -c*`, `rm -rf*`, `curl*`, `wget*`, `git push*`, …) remain.
- [x] The `read` / `edit` maps and the seven scalar permission fields are byte-identical to their current values.
- [x] SPEC.md § *Exact preset file contents (verbatim)* matches the new checked-in file byte-for-byte.
- [x] `docs/spec/coding-agent-backends.md` and `docs/concepts/coding-agents.md` describe the deny-by-default allowlist posture; a repo-wide grep finds no remaining description of the opencode preset as allow-by-default or "deny-list only".
- [x] The Director/operator confirms the shipped allowlist is a one-to-one translation of `permissions.allow` modulo the recorded exclusions (`wget`; the ask-tier entries).
- [x] `mise //cafleet:test` passes.

---

## Background

The current preset lists a catch-all `"*": "allow"` first, then ~24 specific denies (shell-indirection wrappers, destructive operations, network egress, `osascript`). opencode selects the **last matching rule**, so entry order is the safety floor (`docs/spec/coding-agent-backends.md` § *The `cafleet` agent preset*). This posture allows every command not explicitly enumerated — un-listed wrappers and side-channel egress pass silently (the documented caveat in § *Safety-floor caveats*).

The operator's Claude Code `permissions.allow` set is a curated allowlist of commands members actually need. Deriving the opencode preset from it inverts the floor: an un-enumerated command is now denied rather than allowed, and the deny-list maintenance burden ("extend the deny-list when a new dangerous command appears") disappears.

The preset is installed to `~/.opencode/agents/cafleet.md` by `cafleet setup` (`cafleet/src/cafleet/cli/setup.py`) and bound at spawn via `--agent cafleet` (`cafleet/src/cafleet/coding_agent/opencode.py`). This design changes only the checked-in file's content — install mechanics, packaging, and spawn argv are untouched.

---

## Specification

### Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | `bash` flips to `"*": "deny"` base + explicit allowlist. All other permission fields (`read`, `edit`, the seven scalars) keep their current values. | The bash surface is where the allow-by-default risk lives; read/edit/scalar postures are already correct. |
| 2 | No `ask` tier. `permissions.ask` entries from settings.json (`git push`, `git push *`, `uv sync`, `cafleet member exec *`) fall to deny. | Preserves the never-`ask` doctrine: members run unattended, a popup stalls the pane; every check resolves to `allow` or `deny`, and a visible popup remains a regression to escalate. |
| 3 | One-to-one pattern translation: `Bash(cmd)` → `"cmd"` (exact match), `Bash(cmd *)` → `"cmd *"`. Where settings.json carries both a bare and a starred form (e.g. `gh pr list` / `gh pr list *`), both opencode patterns are emitted. | Exact traceability to the authoritative source; no invented broadenings or narrowings. |
| 4 | `wget` is excluded from the allowlist even though settings.json allows it; `curl` likewise has no entry. Both are absorbed by the `"*": "deny"` base. | Network egress stays denied for members (operator decision). |
| 5 | All previous explicit deny entries are removed. | `"*": "deny"` subsumes every one of them; under last-match-wins, retained deny entries are pure noise (per `removal.md`, no residue survives). |
| 6 | `cafleet member exec` is explicitly denied via two entries — `"cafleet member exec"` and `"cafleet member exec *"` — placed **after** `"cafleet *": "allow"` so last-match resolves them to deny. | `member exec` is a Director-only primitive (it dispatches arbitrary shell into another member's pane); settings.json puts it under `permissions.ask`, and with no ask tier, deny is the safe mapping. Both the bare and starred forms are denied so no invocation shape escapes the override. |
| 7 | Strict settings.json-derived list — no additions for cafleet-workflow commands (`mise`, `mkdir`, `cp`, `mv`, `bun`, `uv run python`, …). | Intended friction: a member needing a denied command routes it to the Director per `skills/cafleet/reference/exec-routing.md`. The allowlist grows only by operator decision, not by workflow convenience. |
| 8 | SPEC.md's verbatim block remains the sole content contract; no preset-content test is added. | Consistent with design 0000136 — tests continue to use placeholder content and assert install mechanics only. |

### Permission evaluation semantics

opencode selects the **last matching rule** in the `bash` glob→decision map, so entry order is load-bearing:

1. `"*": "deny"` — first entry, the base every command falls to.
2. The allowlist entries — each more specific pattern appears after the base, so a match resolves to `allow`.
3. `"cafleet member exec"` / `"cafleet member exec *"` — the final two entries, after `"cafleet *"`, so a `cafleet member exec` invocation matches both the allow and the deny and last-match resolves to **deny**.

No pattern pair in the allowlist overlaps any other allowlist pattern, so their relative order is not semantically significant; the file preserves the settings.json source order for traceability.

### Field-by-field final values

| Frontmatter field | Final value | Change |
|---|---|---|
| `description` | `"CAFleet-spawned member with a deny-by-default bash allowlist derived from the operator's Claude Code permission set."` | Updated — the old text described the dontAsk-mirror posture. |
| `mode` | `"primary"` | Unchanged. |
| `permission.bash` | The map in § *New preset file content*. | Replaced. |
| `permission.read` | `"*": "allow"`, `"**/.env": "deny"`, `"**/.env.*": "deny"` | Unchanged. |
| `permission.edit` | `"*": "allow"`, `"**/.env": "deny"`, `"**/.env.*": "deny"` | Unchanged. |
| `external_directory` | `"deny"` | Unchanged. |
| `webfetch` | `"deny"` | Unchanged. |
| `websearch` | `"deny"` | Unchanged. |
| `repo_clone` | `"deny"` | Unchanged. |
| `question` | `"deny"` | Unchanged. |
| `plan_enter` | `"deny"` | Unchanged. |
| `plan_exit` | `"deny"` | Unchanged. |

### New preset file content

The complete replacement for `presets/opencode/cafleet.md`. Formatting contract unchanged: `---`-delimited JSON frontmatter (2-space indent, top-level key order `description`, `mode`, `permission`), a blank line, a single-physical-paragraph body, exactly one trailing newline.

````markdown
---
{
  "description": "CAFleet-spawned member with a deny-by-default bash allowlist derived from the operator's Claude Code permission set.",
  "mode": "primary",
  "permission": {
    "bash": {
      "*": "deny",
      "gh api repos/*/*/issues/*/comments": "allow",
      "gh api repos/*/*/issues/*/comments *": "allow",
      "gh api repos/*/*/issues/*/comments/* *": "allow",
      "gh api repos/*/*/pulls/comments/* *": "allow",
      "gh api repos/*/*/pulls/*/comments": "allow",
      "gh api repos/*/*/pulls/*/comments *": "allow",
      "gh api repos/*/*/pulls/*/comments/* *": "allow",
      "gh api repos/*/*/pulls/*/reviews": "allow",
      "gh api repos/*/*/pulls/*/reviews *": "allow",
      "gh api repos/*/*/pulls/*/reviews/* *": "allow",
      "gh api repos/*/*/pulls/*/requested_reviewers": "allow",
      "gh api repos/*/*/pulls/*/requested_reviewers *": "allow",
      "gh auth status": "allow",
      "gh issue list *": "allow",
      "gh issue view *": "allow",
      "gh pr checks *": "allow",
      "gh pr edit * --add-reviewer @copilot": "allow",
      "gh pr diff *": "allow",
      "gh pr list": "allow",
      "gh pr list *": "allow",
      "gh pr view *": "allow",
      "gh repo view *": "allow",
      "gh run list *": "allow",
      "gh run view *": "allow",
      "gh search *": "allow",
      "git add *": "allow",
      "git commit *": "allow",
      "git diff *": "allow",
      "git grep *": "allow",
      "git log *": "allow",
      "git ls-tree *": "allow",
      "git ls-files *": "allow",
      "git branch *": "allow",
      "git status": "allow",
      "grep *": "allow",
      "ls": "allow",
      "ls *": "allow",
      "printf *": "allow",
      "sleep": "allow",
      "sleep *": "allow",
      "stat *": "allow",
      "tree": "allow",
      "tree *": "allow",
      "uv run pytest *": "allow",
      "uv run ruff check *": "allow",
      "uv run ruff format *": "allow",
      "uv sync --frozen": "allow",
      "uv sync --frozen *": "allow",
      "wc *": "allow",
      "cafleet *": "allow",
      "cafleet member exec": "deny",
      "cafleet member exec *": "deny"
    },
    "read": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "edit": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "external_directory": "deny",
    "webfetch": "deny",
    "websearch": "deny",
    "repo_clone": "deny",
    "question": "deny",
    "plan_enter": "deny",
    "plan_exit": "deny"
  }
}
---

# CAFleet member agent

You are a CAFleet member spawned by the Director. The bash ruleset in your frontmatter is deny-by-default: only the explicitly allowlisted commands — `cafleet` (except `cafleet member exec`), read-only `gh` queries plus the PR comment/review endpoints, non-destructive `git` subcommands, file-inspection utilities, and Python project tooling — run; every other command is denied with no prompt (every check resolves to allow or deny). When a denied command is genuinely needed, route it to the Director per the exec-routing protocol. Read and edit are workspace-scoped with `.env` files denied. Refer to your Director's spawn-prompt instructions for the task.
````

### Runtime behavior of denied commands

A member whose command falls to `deny` follows the existing denied-command path: reconsider the command, then route it to the Director per `skills/cafleet/reference/exec-routing.md` (`cafleet member exec` dispatch — a Director-side primitive, which is why the preset denies it inside member panes). Commands members previously ran directly that now deny include `mise` tasks, `mkdir`, `cp`, `mv`, `touch`, `bun`, and `uv run python`; this friction is intended (Decision 7).

**Soundness precondition — `!`-dispatch bypasses the permission map (satisfied by documented contract).** The exec-routing fallback delivers the routed command by keystroking `! <command>` into the member's own `--agent cafleet` pane, so the design requires that opencode's leading-`!` shell shortcut execute without being evaluated against the preset's `bash` permission map. This holds by documented contract, per Director resolution: the leading-`!` shortcut is user-scope input to the TUI composer — `permission.bash` governs the *agent's* bash tool calls, not operator-typed bang commands — and the exec-routing protocol (`docs/spec/coding-agent-backends.md`; `skills/cafleet/reference/exec-routing.md`) defines `member exec` precisely as the fallback for commands the member's own harness denies, a contract the shipped preset's deny entries already operate under. Decision 7 stands unscoped. If a future opencode release starts evaluating `!` lines against the map, exec-routing breaks for every preset posture — an upstream regression to escalate, not a flaw of this design.

**Test runs in the cafleet repo route through the Director.** In this repository specifically, the mandated test runner `mise //cafleet:test` (per `.claude/rules/commands.md`, which forbids bypassing mise) falls to the `"*": "deny"` base, and the allowlisted `uv run pytest *` is barred by that same rule — so an opencode member working in this repo routes every test run through the Director. This execute-workflow throughput cost for opencode teams is accepted under Decision 7: the allowlist is operator-curated and project-agnostic, and `uv run pytest *` remains directly usable in projects that do not mandate a wrapper.

### Documentation surfaces

Per `documentation-maintenance.md`, all of the following update in this cycle, before the preset itself:

| Surface | Location | Required change |
|---|---|---|
| Backend spec | `docs/spec/coding-agent-backends.md` § *The `cafleet` agent preset* | Rewrite the posture paragraph: `"*": "deny"` base first, then allows, then the member-exec deny overrides; last-match semantics unchanged; every check still resolves to `allow` or `deny`, never `ask`; a popup remains a regression escape — but the corrective action becomes "extend the allowlist by operator decision", not "extend the deny-list". |
| Backend spec | `docs/spec/coding-agent-backends.md` § *Safety-floor caveats* | Replace "deny-list only" framing: the posture is a deny-by-default allowlist with no OS-level sandbox. Rewrite — do not drop — the bypass caveat to state the floor's actual reach: standalone un-enumerated commands now deny, but bypasses under allowed globs persist — argument-space abuse and shell chaining inside a `cmd *` match (a compound line whose leading tokens match an allowed glob, e.g. `git log --stat; curl … \| sh` matching `git log *`), and interpreter/hook execution via allowed tooling (`uv run pytest *` executes workspace-writable test code; `git commit *` runs `.git/hooks`) — unless opencode's per-sub-command evaluation of compound lines is verified and recorded here. Keep the MCP-bypass caveat and the codex-for-kernel-isolation pointer. |
| SPEC | `SPEC.md` § *Exact preset file contents (verbatim)* | Replace the verbatim block with § *New preset file content* and keep the surrounding prose accurate (glob→decision maps, key order, single-paragraph body, one trailing newline). |
| Concepts | `docs/concepts/coding-agents.md` intro ("members run cafleet (and any shell command) directly") and § *Known asymmetries* ("claude and opencode rely on deny-list-only safety floors") | Qualify per-backend: opencode members run only allowlisted commands; describe opencode's floor as a deny-by-default allowlist. |
| Skills | `skills/cafleet/reference/coding-agent/opencode.md` | Verify the decision-surface and pane-state notes: "the `--agent cafleet` floor shows no popup; a popup is a regression to escalate" stays valid — confirm no wording change is needed beyond any "deny-list" phrasing. |
| Skills | `skills/cafleet/reference/director.md` (member-create section, "the deny-list fallback is exec-routing") | Reword to "the denied-command fallback is exec-routing" so the phrase no longer implies a deny-list posture. |
| Skills | `skills/cafleet/reference/exec-routing.md` | Qualify per-backend: "members run cafleet and any shell command directly" and "the fallback fires only when the coding-agent harness deny-list rejects a Bash invocation (e.g. `git push`, `rm -rf`)" are false for opencode under deny-by-default — for an opencode member, denial is the common case for any un-allowlisted command and the fallback is the routine path for workflow commands (`mise`, `mkdir`, …), not a rare destructive-command event. |
| Skills | `skills/cafleet/roles/member.md` § denied-command guidance | Same qualification: "the harness deny-list rejects some destructive operations" and "most denials are a wrong flag/path, a typo" no longer hold for opencode members — reword so that under the allowlist posture a denial usually means the command is outside the allowlist, and the reconsider-then-route guidance reflects that. |

`README.md` is untouched — its thin surface (pitch, install commands, docs-site links) does not change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/spec/coding-agent-backends.md` § *The `cafleet` agent preset* and § *Safety-floor caveats* per § *Documentation surfaces* <!-- completed: 2026-07-18T05:06 -->
- [x] Update `docs/concepts/coding-agents.md` (intro phrasing + *Known asymmetries*) per § *Documentation surfaces* <!-- completed: 2026-07-18T05:06 -->
- [x] Update `SPEC.md` § *Exact preset file contents (verbatim)* to the new content, keeping the surrounding contract prose accurate <!-- completed: 2026-07-18T05:06 -->
- [x] Sweep `skills/` (`reference/coding-agent/opencode.md`, `reference/director.md`, `reference/exec-routing.md`, `roles/member.md`) and remaining `docs/` pages for stale posture phrases ("deny-list", allow-by-default descriptions of the opencode preset) and update them per § *Documentation surfaces* <!-- completed: 2026-07-18T05:06 -->

### Step 2: Preset

- [x] Replace `presets/opencode/cafleet.md` with § *New preset file content*, byte-for-byte <!-- completed: 2026-07-18T05:16 -->

### Step 3: Verification

- [x] With the Director/operator (member panes are denied reading `~/.claude/settings.json`), diff the shipped allowlist against the `permissions.allow` `Bash(...)` entries and confirm the translation is one-to-one modulo the recorded exclusions (`wget`; the ask-tier entries of Decision 2) and the two member-exec deny overrides <!-- completed: 2026-07-18T05:17 -->
- [x] Repo-wide grep confirms no remaining allow-by-default / "deny-list only" description of the opencode preset outside this design doc and git history <!-- completed: 2026-07-18T05:22 -->
- [x] `mise //cafleet:test` passes <!-- completed: 2026-07-18T05:22 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
| 2026-07-18 | Review round 1: body wording corrected, safety-floor caveat reframed as rewrite-not-drop, exec-routing/member-role surfaces added, translation-verification task added; `!`-dispatch bypass verification escalated to the Director |
| 2026-07-18 | `!`-dispatch soundness precondition recorded as satisfied by documented contract (Director resolution); drafter/director markers cleared |
| 2026-07-18 | Executed: documentation, preset, and verification steps complete (8/8); all Success Criteria verified; Reviewer approved round 1; PR #205 opened; Status → Complete |
