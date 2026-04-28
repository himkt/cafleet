# CAFleet CLI consolidation: Bash-via-Director, nested-only restructure, codex deprecation

**Status**: Approved
**Progress**: 7/63 tasks complete
**Last Updated**: 2026-04-28

## Overview

Three coupled CLI changes shipped as one design. (1) **Bash-via-Director** (rounds 1–5c, locked): members lose their `Bash` tool and route shell commands through their Director via a `bash_request` JSON payload; the Director runs each command through a new `cafleet member exec` helper (auto-running on allow-list matches, otherwise gating through `AskUserQuestion`) and replies with a `bash_result`. (2) **Nested-only subcommand restructure** (round 6): every existing flat verb (`cafleet message send`, `poll`, `ack`, `cancel`, `broadcast`, `register`, `deregister`, `agents`, `get-task`) moves under a noun group (`agent`, `message`, `member`, `session`, `db`); only `server` and `doctor` stay top-level as meta-commands. Hard-break, no aliases. (3) **Codex deprecation** (round 6): `cafleet member create --coding-agent codex` and the `CODEX` config are removed; claude is the only supported member backend. In-flight codex panes keep running, but no new codex registrations. Restoration is documented in §13 Future Work.

## Success Criteria

### Bash-via-Director (locked round 5c — preserved)

- [ ] `cafleet member create --no-bash` (default ON for claude member panes) appends `--disallowedTools "Bash"` to the spawned `claude` process; the resulting member pane cannot use the Bash tool. Verified with a unit test that asserts the spawn argv contains the flag, plus a real-world smoke that confirms the member's harness rejects Bash calls.
- [ ] When a member needs a shell command, it sends a JSON `bash_request` payload via `cafleet message send --to <director-id> --text '{...}'`. The CLI / payload format is documented in this doc and in `skills/cafleet/SKILL.md`.
- [ ] The Director receives the request via the existing `cafleet message poll` push-notification path. No new IPC, no new broker primitives, no schema changes.
- [ ] If the requested command matches a pattern in the Director's resolved `permissions.allow`, the Director runs it without an `AskUserQuestion` beat. The `bash_result` reply carries `note: "ran without operator prompt (matched allow rule: <pattern>)"` for audit.
- [ ] If the command does NOT match an allow pattern, the Director shows a 3-option `AskUserQuestion`: `Approve as-is` / `Approve with edits` / `Deny with reason`. Built-in "Other" is the typed-edit / typed-rejection slot.
- [ ] The Director runs the resolved command via a new `cafleet member exec` helper invoked through its own Bash tool. The helper enforces deterministic limits (64 KiB stdout/stderr each, SIGKILL at the request `timeout`) AND input validation (`cmd != ""`, `timeout ≤ 600`); on input-validation failure the helper writes a denied JSON object to stdout and exits 0, which the Director copies into `bash_result` verbatim.
- [ ] The operator faces a single consent gate per non-allowlisted command (the `AskUserQuestion` in the Director's pane). The Director's Bash-tool native prompt does not fire because the operator has added `Bash(cafleet member exec *)` to `permissions.allow` — required setup, see §6 / §10.
- [ ] The `cafleet member exec` helper hard-kills the subprocess at the per-request `timeout` (default 30 s, capped at 600 s); on timeout the result reports `status: "timeout"` (exit_code is opaque per the canonical-status rule).
- [ ] `bash_result.status` is the sole source of truth for outcome branching. Clients MUST switch on `status`, not on `exit_code`. `exit_code` is meaningful only when `status == "ran"`.
- [ ] Cross-Director leakage is prevented by the existing CAFleet session boundary (the broker rejects cross-session sends). The "address `bash_request` only to your own `placement.director_agent_id`" rule is documentation-only — `skills/cafleet/SKILL.md` and the member's spawn prompt direct members to follow it. No CLI-level guard in v1; future work covers a typed `cafleet message bash-request` subcommand that would enforce it.
- [ ] No member-side timeout. Members block on the Director's reply forever; the operator manually nudges or cancels (`cafleet message cancel`) if a Director wedges. Document this explicitly so reviewers do not ask for a timeout fallback.

### Nested-only restructure (round 6)

- [ ] Every flat-verb subcommand (`register`, `deregister`, `agents`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`) is moved under a noun group (`agent`, `message`). Only `server` and `doctor` stay top-level as meta-command exceptions.
- [ ] The bash-routing helper introduced in rounds 1–5c is named `cafleet member exec` (it operates on behalf of a member, groups with the other Director-side member ops, and was originally called `bash-exec` in earlier round-5c drafts before round-6 ratchet renamed it as part of the nested-only restructure). The required `permissions.allow` entry is correspondingly `Bash(cafleet member exec *)`.
- [ ] Hard-break: the old subcommand strings stop existing the moment the rename merges. No Click aliases. Every literal occurrence in source code, prompt templates, tmux keystroke injection, SKILL.md files, README, ARCHITECTURE, docs/spec, admin SPA, and this design document is updated in the same documentation-first sweep.
- [ ] The `cafleet` binary itself is unchanged; only subcommands restructure. `Bash(cafleet *)` and `mise //cafleet:*` remain valid.

### Codex deprecation (round 6)

- [ ] `cafleet member create` no longer accepts a `--coding-agent` flag. Claude is the only supported backend. Anyone running with the flag gets `Error: No such option: '--coding-agent'.` (Click default).
- [ ] `CodingAgentConfig.CODEX` and the `CODING_AGENTS` registry's codex entry are removed. The `cafleet/src/cafleet/coding_agent.py` module collapses to a `CLAUDE` constant; the `get_coding_agent()` helper / `CODING_AGENTS` dict are removed since there is exactly one config.
- [ ] In-flight codex member panes (any `agent_placements` row with `coding_agent='codex'` predating the rename) keep running their existing process; the broker does not kill or auto-cleanup. Operators retire them with `cafleet member delete` per pane. The `agent_placements.coding_agent` column stays `TEXT NOT NULL DEFAULT 'claude'` — codex rows are preserved for forensic visibility.
- [ ] Every codex-aware doc / SKILL / test path is removed in the same documentation-first sweep. §13 Future Work captures the restoration plan for when codex grows the equivalent enforcement primitives.

### Cross-cutting (round 6)

- [ ] Documentation is updated **before** code per `.claude/rules/design-doc-numbering.md`: `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `docs/spec/data-model.md`, `README.md`, `cafleet/CLAUDE.md`, `skills/cafleet/SKILL.md`, `skills/cafleet-monitoring/SKILL.md`, both Director role files in `skills/design-doc-create/roles/director.md` and `skills/design-doc-execute/roles/director.md`, and the four admin SPA strings (Sidebar.tsx, Dashboard.tsx ×2, SessionPicker.tsx). Skill drift is a blocker.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` all pass.

---

## Background

### Current state

Members spawned via `cafleet member create` inherit the operator's harness as-is — they have full access to every tool the operator has, including `Bash`. Each member's first unfamiliar Bash call (e.g. `git log -1`, `mise //cafleet:test`, `cat /etc/os-release`) triggers a Claude Code permission prompt in **that member's own pane**. This is the bottleneck the user is reporting:

- The operator is supervising the Director pane, not the member panes — the prompt fires somewhere they are not looking, and the team stalls until the operator notices.
- Per-member `permissions.allow` would in principle help, but in practice the operator does not know which commands a member will need until the member tries one. Pre-allowlisting "everything safe" duplicates the project-level `permissions.allow` for every member.
- The current literal-flag pattern (design 0000023) makes `cafleet --session-id <uuid> *` a single allow rule, but every other tool the member touches still triggers fresh prompts.

### What "route through the Director" buys us

| Today | After this change |
|---|---|
| N members × M unique commands → N×M scattered permission prompts | All prompts fire in the **Director's pane only**, where the operator is already focused |
| Member harnesses each carry their own `permissions.allow` (drifts) | Single project-level `permissions.allow` on the Director governs auto-run vs. ask |
| Operator can't audit what commands ran | Every `bash_request` and `bash_result` is a regular CAFleet message, persisted in SQLite and visible in the admin WebUI timeline |
| Member crashes can leak Bash side-effects (deleted files, force-pushed branches) the operator never saw | Member loses Bash entirely; only the Director's vetted invocations touch the filesystem |

### Why this is the right shape

CAFleet already has the message-passing primitive (`cafleet message send` + tmux push-notification) and the cross-Director authorization boundary (`placement.director_agent_id`). The only missing piece is the discipline of "members route through the Director" — and a way to enforce that the member harness cannot bypass it. The user's chosen enforcement (`claude --disallowedTools "Bash"`, propagated by `cafleet member create --no-bash`) keeps the entire change at the harness boundary; no broker schema, no new RPC.

### Feasibility — `--disallowedTools` flag

Verified directly from `claude --help`:

```
--disallowedTools, --disallowed-tools <tools...>  Comma or space-separated list of tool names to deny (e.g. "Bash(git *) Edit")
```

`claude --disallowedTools "Bash"` denies the entire Bash tool for the spawned session. Tool-pattern syntax matches the same spec as `permissions.allow`/`permissions.deny`, so `Bash`, `Bash(*)`, and `Bash(git *)` are all valid forms. The flag is per-invocation; nothing has to be written to a config file.

`codex --help` showed no equivalent at the time of this design's first draft (rounds 1–5c). The lack of a `--disallowedTools`-equivalent primitive was the original justification for the codex prompt-only fallback. As of round 6, codex support is removed entirely from CAFleet (see §15) — the deprecation eliminates the asymmetry rather than maintaining a soft-discipline fallback. Restoration is documented in §13 Future Work.

---

## Specification

### 1. Member-side: deny Bash at spawn time

`cafleet member create` gains one new flag pair:

```
--no-bash / --allow-bash    (default: --no-bash. claude is the only supported
                             backend; pass --allow-bash to opt out as an
                             escape hatch for one-off members.)
```

| Default | `--no-bash` semantics |
|---|---|
| `--no-bash` (default) | The spawn argv gains `--disallowedTools "Bash"`. The member's harness rejects every Bash call. |
| `--allow-bash` (opt-out) | No harness lock. Documented but not the default — the member's spawn prompt still directs it to route Bash through the Director (soft discipline). |

The implementation extends `CodingAgentConfig` with one new field and one new `build_command` keyword:

```python
@dataclass(frozen=True)
class CodingAgentConfig:
    # ... existing fields ...
    disallow_tools_args: tuple[str, ...] = ()  # e.g. ("--disallowedTools", "Bash")

    def build_command(
        self,
        prompt: str,
        *,
        display_name: str | None = None,
        deny_bash: bool = False,
    ) -> list[str]:
        deny_args = self.disallow_tools_args if deny_bash else ()
        name_args = (*self.display_name_args, display_name) if (display_name and self.display_name_args) else ()
        return [self.binary, *self.extra_args, *deny_args, *name_args, prompt]
```

`CLAUDE.disallow_tools_args = ("--disallowedTools", "Bash")`. The `CODING_AGENTS` registry, the `get_coding_agent()` helper, and the `CODEX` constant are all removed in round 6 (§15) — there is exactly one config now, so callers import `CLAUDE` directly.

**Pinned argv ordering**: `[binary, *extra_args, *deny_tools, *name_args, prompt]` — deny_tools come BEFORE name_args. This ordering is mirrored verbatim in Implementation Step 3 task 2 so the test assertion has a single canonical shape to match.

`build_command` injects the deny args only when `deny_bash=True` AND the dataclass has a non-empty `disallow_tools_args` tuple.

`cli.member_create` passes `deny_bash=resolved_no_bash` into `CLAUDE.build_command(...)` at the existing `tmux.split_window(... command=...)` site. The existing rollback paths (split-window failure, placement update failure) are unchanged — adding one CLI flag and one dataclass field does not introduce any new failure modes.

### 2. Member-side: how the member asks for Bash

The member uses **existing `cafleet message send`** with a JSON-typed payload in the `--text` body. No new member-side CLI subcommand. The `text` field carries:

```json
{
  "type": "bash_request",
  "cmd": "git log -1 --oneline",
  "cwd": "/home/himkt/work/himkt/cafleet",
  "stdin": null,
  "timeout": 30,
  "reason": "verifying the latest commit on main before opening PR #36"
}
```

Schema:

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | `"bash_request"` | yes | Discriminator. Director dispatches on this exact string. |
| `cmd` | `string` | yes | The shell command to run. Single string; `cafleet member exec` invokes it via `bash -c <cmd>` so pipes / `&&` / quoting work as the member typed them. |
| `cwd` | `string \| null` | no | Working directory. Defaults to the Director's current cwd if `null` or omitted. |
| `stdin` | `string \| null` | no | Stdin payload (UTF-8). Use sparingly — embedding large binaries here will hit the 64 KiB outbound truncation. |
| `timeout` | `int \| null` | no | Seconds. Defaults to 30. Capped at 600 (10 min) — the helper rejects oversized `timeout` as a denied JSON output (§3 helper subsection bullet 1; `stderr: "bash_request.timeout exceeds 600s cap."`). |
| `reason` | `string` | yes | Short justification (≤ 200 chars). Surfaced in the Director's `AskUserQuestion` so the operator knows *why* the command is being asked for. |

The member always addresses the request to its own `placement.director_agent_id`. Sending a `bash_request` to any other agent ID in the session is undefined behavior — the recipient simply will not know the convention. Cross-session leakage is prevented by the broker's existing session boundary; the cross-Director rule (within a session) is documentation-only in v1.

The send call shape (use `--json` so `task_id` is machine-parseable; `--json` is a global flag on the parent `cli` group, placed BEFORE the subcommand):

```bash
cafleet --session-id <session-id> --json message send --agent-id <member-id> \
  --to <director-agent-id> \
  --text '{"type":"bash_request","cmd":"git log -1 --oneline","cwd":"/home/himkt/work/himkt/cafleet","reason":"verifying main before PR"}'
```

Then the member blocks on `cafleet message poll` for the matching `bash_result`. **Reply correlation**: the broker assigns each delivered task a server-side `task_id`, surfaced in the `cafleet --json message poll` and `cafleet --json message send` output. The Director copies the request task's `task_id` into `bash_result.in_reply_to` (§3 step 6). The member reads `task_id` from its own `cafleet --json message send` response when sending the request, holds it locally, and matches against `bash_result.in_reply_to` on the next poll. (Note: `--json` is global — `cafleet message send --json` does not work; use `cafleet --json message send` instead.)

### 3. Director-side: receive, classify, dispatch

The Director's normal `/loop` health-check (`Skill(cafleet-monitoring)` Stage 1, `cafleet message poll`) already surfaces incoming messages. The new discipline added in this design is a 6-step dispatch with three distinct phases: **discriminator** (step 1), **matcher** (step 2), and **run + reply** (steps 3–6). The Director's Claude Code pane is the actor for every step.

The discriminator phase has a **future-proof rule**: anything that does not have `type == "bash_request"` is a plain instruction; nothing in this section produces a denied `bash_result` without going through the helper. Field-level validation (empty `cmd`, oversized `timeout`) lives inside `cafleet member exec` itself — see the helper subsection below.

| Step | Director-pane action |
|---|---|
| 1 | **Discriminator**. Parse the polled `text` body as JSON. If parsing fails, or the parsed object lacks `type`, or `type != "bash_request"`, treat the message as a plain instruction. End — no `bash_result` emitted. Otherwise continue to step 2. |
| 2 | **Matcher**. Apply `match_allow` (§4) against the project's resolved `permissions.allow` and `permissions.deny`. The matcher returns exactly one of `auto-run` or `ask`. |
| 3 | If `auto-run`: continue to step 5 with the original `cmd`. Set `note = "ran without operator prompt (matched allow rule: <pattern>)"`. |
| 4 | If `ask`: present the 3-option `AskUserQuestion` below. Resolve the operator's choice and act per branch: <br>• On `run-as-is(cmd)`: continue with the original `cmd`; do not set `note`. <br>• On `run-edited(new_cmd)`: continue with `new_cmd` AND set `note = "operator edited cmd before running. original: <verbatim-original-cmd>"`. <br>• On `deny(reason)`: skip the helper invocation; emit a denied `bash_result` directly with `status: "denied"`, `note: "operator denied: <reason>"`, `stderr: <reason or default>`. Reply via step 6, end dispatch. |
| 5 | **Run via `cafleet member exec`**. The Director's pane invokes the helper through its Bash tool: `cafleet member exec --cmd '<cmd>' [--cwd '<cwd>'] [--timeout <timeout>] [--stdin '<stdin>']`. Each optional flag is included only when the corresponding request field is present and non-null; otherwise the helper's defaults apply (cwd: pane cwd; timeout: 30 s; stdin: empty). The Director's Bash-tool prompt does not fire because `Bash(cafleet member exec *)` is in the operator's `permissions.allow` (required setup, see §10 / §6). AskUserQuestion (step 4) is the only consent surface in this flow. The helper handles deterministic limits (64 KiB caps, SIGKILL at timeout) AND input validation (`cmd != ""`, `timeout ≤ 600`); on input-validation failure the helper writes a denied JSON object to its own stdout (status `"denied"`) which the Director copies into `bash_result` exactly as it would for any other helper output. The Director's pane parses the helper's JSON output. |
| 6 | **Reply**. The Director's pane invokes `cafleet message send --agent-id <director-id> --to <member-id> --text '<bash_result-json>'` via its Bash tool to deliver the reply. The push notification injects a `cafleet message poll` keystroke into the member's pane and the member resumes. The operator does not type anything; the Director's pane is the actor for both the `cafleet member exec` helper invocation and the reply send. |

#### `AskUserQuestion` shape (step 4)

| # | Label | Description holds | Resolves to |
|---|---|---|---|
| 1 | `Approve as-is` | The request's `reason` and the verbatim `cmd`. | `run-as-is(cmd)` |
| 2 | `Approve with edits` | "Operator types the edited command body via Other." (built-in Other) | `run-edited(<typed-cmd>)` |
| 3 | `Deny with reason` | "Operator types the rejection reason via Other." (built-in Other) | `deny(<typed-reason>)` |

Notes:

- The 4th built-in "Other" slot routes to either edit-or-deny based on what the operator typed. The Director cannot tell them apart structurally; convention is that the operator who picks built-in Other directly (without first picking option 2 or 3) is editing. The 3 explicit options exist precisely to disambiguate.
- The `AskUserQuestion` per-call limits (1 question per call, 2–4 options, built-in "Other" already exposed) are observed. No 5th option, no preamble sentence, no fenced-bash instruction blocks (per design 0000033 discipline).
- The question text names the member: `"<member-name> wants to run a Bash command. Approve, edit, or deny?"`

#### `bash_result` payload (step 6)

```json
{
  "type": "bash_result",
  "in_reply_to": "<request-task-id>",
  "status": "ran",
  "exit_code": 0,
  "stdout": "abc1234 docs: mark design doc as complete\n",
  "stderr": "",
  "duration_ms": 47,
  "note": "ran without operator prompt (matched allow rule: Bash(git *))"
}
```

**Canonical-status rule**: `status` is the sole source of truth for the outcome. `exit_code` is meaningful **only when `status == "ran"`**, where it carries the subprocess's verbatim return code. For `status: "denied"` and `status: "timeout"` the `exit_code` field is present (so shell tooling that always reads it does not crash) but its value is documented as **opaque** — clients MUST switch on `status`, not on `exit_code`.

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | `"bash_result"` | yes | Discriminator. |
| `in_reply_to` | `string` | yes | The `task_id` of the originating `bash_request`. Members correlate replies via this field (§2). |
| `status` | `"ran" \| "denied" \| "timeout"` | yes | Top-level outcome. **Sole source of truth.** `ran` means the helper invoked the subprocess and it terminated normally (any `exit_code`). `denied` means the helper or operator denied the request (matched no allow + operator picked Deny, OR helper input-validation failed: empty `cmd`, oversized `timeout`). `timeout` means the helper hard-killed the subprocess at the configured `timeout`. |
| `exit_code` | `int` | yes | When `status == "ran"`: the subprocess's verbatim exit code. When `status == "denied"` or `"timeout"`: **value is opaque — do not switch on it.** The helper internally uses `126` for denied and `124` for timeout for shell-legibility, but clients MUST use `status` for branching. |
| `stdout` | `string` | yes | UTF-8, truncated to 64 KiB. If truncation occurred, the last line is replaced by `\n[truncated: original was N bytes; last 65536 bytes shown]\n`. |
| `stderr` | `string` | yes | Same shape as stdout. On `status: "denied"` it carries the operator's typed rejection reason, the default `"Director denied the request."`, or the helper's input-validation message (e.g. `"bash_request.cmd may not be empty."`). On `status: "timeout"` it carries `"hard-killed at <N> seconds."` plus any partial stderr captured before SIGKILL. |
| `duration_ms` | `int` | yes | Wall-clock duration in milliseconds. On `status: "denied"` this is 0. |
| `note` | `string` | no | Optional human note. Set on `status: "ran"` when the command was auto-run via the allow-list path: `note: "ran without operator prompt (matched allow rule: <pattern>)"`. Set on `status: "ran"` when the operator edited the command: `note: "operator edited cmd before running. original: <verbatim original cmd>"`. Set on `status: "denied"` when the operator denied: `note: "operator denied: <reason>"`. Omitted otherwise. |

#### `cafleet member exec` helper (new Director-side subcommand)

```
cafleet member exec --cmd '<cmd>' [--cwd <path>] [--timeout <int>] [--stdin <text>]
```

| Flag | Required | Notes |
|---|---|---|
| `--cmd` | yes | Single-string shell command. Invoked via `subprocess.run(["bash", "-c", cmd], ...)`. |
| `--cwd` | no | Working directory. Defaults to the helper's own cwd. |
| `--timeout` | no | Seconds. Default 30. Capped at 600. |
| `--stdin` | no | UTF-8 text passed on the subprocess's stdin. |

The helper:

1. **Validates input first.** If `--cmd` is empty/absent OR `--timeout` exceeds 600, the helper does NOT invoke the subprocess. Instead it writes a denied JSON object to stdout: `{"status": "denied", "exit_code": 126, "stdout": "", "stderr": "<reason>", "duration_ms": 0}` and **exits with helper-process exit code 0** (the input was syntactically valid CLI args; the validation failure is a payload-level result, not an arg error). The Director copies this verbatim into `bash_result` exactly as it would for any other helper output. Reasons: `"bash_request.cmd may not be empty."` for empty cmd; `"bash_request.timeout exceeds 600s cap."` for oversized timeout.
2. **Hard-kills the subprocess at `--timeout`** via `Popen.kill()` (delivers SIGKILL). On hard-kill the helper sets `status: "timeout"` and `exit_code: 124`; per the canonical-status rule, the `exit_code` value is opaque to clients — it is internally `124` for shell-legibility but clients switch on `status`.
3. **Caps captured stdout and stderr at 64 KiB each.** If truncation occurred, the last line of the captured stream is replaced by `\n[truncated: original was N bytes; last 65536 bytes shown]\n` (matching the `bash_result.stdout` / `stderr` truncation marker so the Director can pass it through verbatim).
4. **Prints exactly one JSON object on its OWN stdout**, structured as:
   ```json
   {
     "status": "ran" | "denied" | "timeout",
     "exit_code": <int>,
     "stdout": "<captured-or-truncated>",
     "stderr": "<captured-or-truncated>",
     "duration_ms": <int>
   }
   ```
   The Director copies these five fields verbatim into the `bash_result` payload, prepending `type` and `in_reply_to` (and for the operator-Deny branch, overriding `status` to `"denied"` and adding the `note` per §3 step 4).
5. **Does NOT** know about `permissions.allow`, `bash_request`, or CAFleet messaging. It is a pure subprocess-runner with deterministic limits and input validation. The matcher / dispatch logic lives in the Director's prompt-side workflow, not in the helper.
6. Does NOT require `--session-id` (consistent with `db init` / `session *` / `server`). Silently accepts and ignores the global `--session-id` flag so a single allow pattern stays usable across all subcommands.

The helper's process exit code is `0` for every payload outcome (ran / denied / timeout). It is non-zero only when its own CLI-arg parsing fails (e.g. unknown flag — handled by Click's built-in `UsageError`, exit 2). The Director never relies on the helper's process exit code for the bash-routing reply — it parses the JSON on stdout.

### 4. Allow-list matcher

§4 covers exclusively the allow×deny matcher. Field-level validation (`cmd != ""`, `timeout ≤ 600`) is the helper's responsibility — see §3 helper subsection bullet 1. There is no separate "reject thresholds" phase in the Director's dispatch.

#### Resolved sources

The Director resolves its `permissions.allow` and `permissions.deny` from the layered config:

1. `~/.claude/settings.json` (user-global)
2. `<project-root>/.claude/settings.json` (project)
3. `<project-root>/.claude/settings.local.json` (project, gitignored)

The matcher concatenates all three lists per category (later layers append; deduplication is not required for correctness).

#### Pattern syntax

Matching uses the same fnmatch-style glob syntax Claude Code itself uses for `permissions.allow`:

| Pattern | Matches |
|---|---|
| `Bash(git *)` | Any `cmd` whose token-0 is `git`. |
| `Bash(cafleet *)` | Any `cmd` whose token-0 is `cafleet`. |
| `Bash(mise //cafleet*)` | Any `cmd` starting with `mise //cafleet`. |
| `Bash(*)` | Every `cmd`. (Effectively disables the gating; document but do not recommend.) |

The matcher only considers patterns that start with `Bash(...)`. Other tool patterns (`Edit`, `Skill(...)`) are ignored.

#### Allow × deny truth table

| Allow match | Deny match | Result |
|---|---|---|
| yes | no | `auto-run` |
| yes | yes | `ask` (deny match overrides — operator is shown the AskUserQuestion) |
| no | yes | `ask` |
| no | no | `ask` |

Single rule restated: **an `auto-run` outcome requires an allow match AND no deny match**. Any other combination falls through to `ask`. There is no `auto-deny` outcome from the permission matcher — `permissions.deny` only downgrades a would-be `auto-run` to `ask`; the operator still gets the chance to approve through `AskUserQuestion`. (This matches the spirit of `permissions.deny` in Claude Code, where deny rules block auto-execution but do not prevent the operator from manually granting permission.)

The matcher is intentionally simple — exact-prefix and one-token-glob coverage is enough for the project's existing `Bash(mise //cafleet*)` / `Bash(cafleet *)` / `Bash(git mv *)` / `Bash(uv run cafleet *)` patterns. A future doc can extend it with full glob semantics if a project's `permissions.allow` patterns drift more complex.

### 5. Cross-Director boundary

The bash-request flow is just `cafleet message send` under the hood, so the existing **cross-session boundary** already prevents cross-session leakage at the broker level. ONE extra rule applies *within* a session:

- The member's request MUST be addressed to its own `placement.director_agent_id`. Sending to any other agent in the same session is a misuse — the recipient does not know the bash-request convention and will treat it as a generic message.

There is no CLI-level enforcement of this rule in v1 because `cafleet message send` does not parse the `--text` payload. Enforcement is purely documentation: `skills/cafleet/SKILL.md` documents the rule under "Routing Bash via the Director", and the member's spawn prompt (Step 4 below) is amended to remind it.

If the user later wants programmatic enforcement, a follow-up doc can add a typed `cafleet message bash-request` subcommand that wraps `cafleet message send` with the cross-Director guard built in. That is explicitly out of scope here — listed in §13 Future Work.

### 6. Director-side requirements

#### Required `permissions.allow` entry

The operator MUST add `Bash(cafleet member exec *)` to the Director's `permissions.allow` (project `.claude/settings.json` or user `~/.claude/settings.json`). Without this entry the Director's pane gets a duplicate native Bash-tool prompt for every command, defeating the single-consent-gate goal — `AskUserQuestion` is supposed to be the sole consent surface in this flow. The cafleet-monitoring skill's setup checklist verifies the entry; future work (§13) may make `cafleet member create` warn when it detects the entry is missing.

This is operator-facing setup, not a programmatic guarantee. A Director that runs without the entry still works, just with worse UX (two prompts per non-allowlisted command). The doc treats the entry as **required** because the design's UX guarantee (single consent surface) does not hold otherwise.

Order is irrelevant — adding the `permissions.allow` entry before or after the binary upgrade both work. Intermediate states (entry installed but old binary, or new binary but no entry) are degraded-UX (extra Bash prompts) but functional.

#### Operator upgrade checklist

After upgrading the cafleet binary, **restart any Director session that was running pre-upgrade**. Claude Code caches skill text per-session at session start; the cached `Skill(cafleet)` will reference round-5c-era commands (`cafleet bash-exec`, `cafleet poll`, etc.) until the session reloads from disk. Members spawned by a pre-upgrade Director session would receive stale spawn-prompt instructions that point at command names the new binary no longer accepts. There is no in-session skill-reload command — the only supported reload mechanism is exiting and restarting the Claude Code session so the harness re-reads `skills/<name>/SKILL.md` from disk.

#### Member-backend support

Claude is the only supported coding-agent backend in this design. The codex backend was deprecated as part of round 6 — see §15 for the deprecation, the migration story, and §13 Future Work for the restoration plan.

`cafleet member create` always spawns claude. There is no `--coding-agent` flag.

### 7. Member-side blocking semantics (no timeout)

Per the user's explicit answer to Q9, members do NOT have a timeout on bash_request replies. The member's loop is:

1. Send the request via `cafleet message send`. Capture the returned `task_id`.
2. Wait for the broker's tmux push notification, which injects a `cafleet message poll` keystroke into the member's pane when the bash_result arrives. The member polls (one-shot, not a loop), filters by `in_reply_to == task_id`, and resumes.
3. Resume.

If `cafleet message poll` returns messages in addition to the bash_result (or before the bash_result arrives), the member processes those messages as plain instructions per existing CAFleet semantics; the bash_request continuation only resumes when a `bash_result` with the matching `in_reply_to` arrives. The member's task is not gated on bash_result arrival — it's gated on the bash_result correlation.

If the Director wedges (crashed pane, busy mid-tool-call, or the operator stepped away from the keyboard), the member sits idle. Recovery is operator-driven via existing CAFleet primitives:

- `cafleet member capture` to inspect the wedged Director's pane.
- `cafleet message send` to nudge the Director directly (the bash_request task remains in the queue).
- `cafleet message cancel` to retract the original request if the operator wants the member to give up.

This is consistent with every other CAFleet message-passing path — no built-in timeouts. Documented explicitly in §13 (Future Work) so reviewers know it is a deliberate v1 choice.

### 8. Director subprocess hard limits

Independent of the member-side "no timeout" stance, the **Director's `cafleet member exec` helper** is hard-bounded:

| Limit | Value | Source |
|---|---|---|
| Wall-clock | request `timeout` field, default 30 s, capped at 600 s | Schema in §2; helper validates and rejects via §3 helper bullet 1 |
| stdout capture | 64 KiB | hardcoded in helper |
| stderr capture | 64 KiB | hardcoded in helper |

Truncation marker (appended after the captured bytes, in both `cafleet member exec` JSON output and `bash_result.stdout` / `stderr`):

```
[truncated: original was <N> bytes; last 65536 bytes shown]
```

On hard-kill, stderr additionally carries `hard-killed at <N> seconds.`. `status` is `"timeout"`; `exit_code` is opaque per the canonical-status rule (the helper sets it to `124` internally for shell-legibility, but clients switch on `status`).

### 9. Out of scope

Explicit non-goals so reviewers do not request these:

- No broker schema changes (no new `tasks` columns, no new tables).
- No changes to the `tmux` push-notification path. Existing `_try_notify_recipient` handles the `bash_result` reply identically to any other unicast message.
- No changes to `cafleet member send-input` (the AskUserQuestion-answer write-path from designs 0000027 / 0000033). Those keystroke flows are orthogonal.
- No changes to the Agent Teams primitive (TeamCreate / SendMessage / `Drafter`/`Reviewer`). This design covers CAFleet members only.
- No new dedicated `cafleet message bash-request` member-side subcommand. v1 reuses `cafleet message send`.
- No admin WebUI affordance. `bash_request` and `bash_result` payloads render as plain JSON text in the timeline.
- No member-side timeout flag.
- No structured Director-side throttling (e.g. "Approve next 5 from this member" memoization).
- No global skill-copy parity sweep. The cafleet, cafleet-monitoring, design-doc-create, and design-doc-execute skills exist only in the project tree (`skills/<name>/...`); there are no `~/.claude/skills/cafleet*` copies to keep in sync.

### 10. Documentation surface

Per `.claude/rules/design-doc-numbering.md`, every doc surface that mentions the affected commands MUST be updated **before** any code change.

| File | Change |
|---|---|
| `ARCHITECTURE.md` | (a) Add a new "Bash Routing via Director" subsection after "Member Lifecycle". Document the `--no-bash` flag on `cafleet member create`, the `cafleet member exec` helper, the `bash_request` / `bash_result` payload schemas (highlight the canonical-status rule — `status` is the source of truth, `exit_code` is opaque except on `ran`), and the no-timeout member semantics. Cross-reference `skills/cafleet/SKILL.md` as canonical. (b) In the same subsection add one sentence: "Director-side requirement: the operator MUST add `Bash(cafleet member exec *)` to `permissions.allow` for the single-consent-gate UX to hold (see `skills/cafleet/SKILL.md` for setup details)." (c) Drop the existing "Multi-runner support" paragraph and the "Pane display-name propagation" paragraph's codex examples — claude is the only supported backend per §15. (d) The `coding_agent.py` Component Layout row collapses to "claude-only spawn config." |
| `docs/spec/cli-options.md` | (a) Under the `cafleet member create` subsection, add `--no-bash` AND `--allow-bash` flag entries (the Step 1 flag-pair). The `--coding-agent` flag entry is removed entirely (codex deprecation §15). (b) Add a new subsection `### member exec` (under the `member` group) documenting the helper's flags, JSON output schema, exit codes, the canonical-status rule, the input-validation behavior (helper-internal, not Director-side), and 64 KiB / 30 s limits. (c) Add a new top-level subsection `### permissions.allow setup` listing the required `Bash(cafleet member exec *)` entry alongside the project's existing `Bash(cafleet *)` patterns. |
| `README.md` | In the member-commands section, add a one-line bullet: "`cafleet member create --no-bash` — spawn a member with the Bash tool denied; the member must route shell commands through the Director (see `skills/cafleet/SKILL.md` § Routing Bash via the Director)." Also add a one-line bullet for the new `cafleet member exec` helper under the top-level CLI bullets. |
| `skills/cafleet/SKILL.md` | (a) Extend the `### Member Create` subsection with `--no-bash` / `--allow-bash` flag rows; remove the existing `--coding-agent` flag row and every codex example invocation (codex deprecation §15). (b) Add a new top-level section `## Routing Bash via the Director` after `### Answer a member's AskUserQuestion prompt`. Include both payload schemas verbatim (highlight the canonical-status rule), the 3-option AskUserQuestion shape, the auto-allow path, the **required** `Bash(cafleet member exec *)` `permissions.allow` entry, the cross-Director rule, and the no-timeout note. (c) Add `### Member Exec` to Command Reference, between `Member Send-Input` and `Server`, documenting the helper. |
| `skills/cafleet-monitoring/SKILL.md` | (a) Add a row to the Stall Response escalation table: when `cafleet message poll` shows an unresponded `bash_request` from a member, the Director MUST process it before any other inbox item — bash_request is blocking on the member side. Reference the new cafleet-skill section. (b) Add a new "### Director setup: required `permissions.allow` entries" subsection listing the `Bash(cafleet member exec *)` entry as required setup before spawning any `--no-bash` member. The supervisor checklist treats a missing entry as a setup gap to flag to the operator. |
| `skills/design-doc-create/roles/director.md` | One-paragraph note: when a Drafter/Reviewer member sends a `bash_request`, follow the workflow in the cafleet skill. Do NOT print fenced-bash blocks for the user (carries forward design 0000033 discipline). |
| `skills/design-doc-execute/roles/director.md` | Same paragraph. |

(No global-skill-copy mirror step. The four skills above exist only in the project tree per §9 "Out of scope".)

### 11. Tests

| Test | Coverage |
|---|---|
| `test_coding_agent.py::TestDisallowTools` | `CLAUDE.build_command(prompt, deny_bash=True)` argv equals `["claude", "--disallowedTools", "Bash", prompt]` (no display_name) or `["claude", "--disallowedTools", "Bash", "--name", "<n>", prompt]` (with display_name) — verifying the pinned `[binary, *extra_args, *deny_tools, *name_args, prompt]` ordering. `CLAUDE.build_command(prompt, deny_bash=False)` argv does NOT contain the `--disallowedTools` tokens. (No codex assertions: `CodingAgentConfig.CODEX` and the `CODING_AGENTS` registry are removed in §15.) |
| `test_cli_member.py::TestNoBashFlag` | (a) `cafleet member create --no-bash` (default) passes `deny_bash=True` to `build_command` (verified by monkey-patching `tmux.split_window` and capturing the `command` arg). (b) `cafleet member create --allow-bash` passes `deny_bash=False`. The `--coding-agent` flag is removed entirely (§15) — any caller passing it gets Click's default "no such option" error; this is asserted as part of the codex-deprecation tests in §15, not here. |
| `test_bash_routing_payload.py` | Schema-shape tests for `bash_request` and `bash_result` JSON: payload-helper functions (`parse_bash_request`, `format_bash_result`) round-trip through `json.dumps` / `json.loads`. `parse_bash_request` returns `None` for non-`bash_request` shapes (parse fail / missing type / `type` mismatch); does NOT raise on field-level issues like empty `cmd` or oversized `timeout` — those propagate to the helper. `format_bash_result` round-trips with each `status` value; truncation-marker formatting in input strings is preserved verbatim (`[truncated: original was N bytes; last 65536 bytes shown]`). The canonical-status rule is asserted: `format_bash_result(status="denied", exit_code=999)` round-trips with `exit_code=999` (caller-opaque). |
| `test_bash_routing_matcher.py` | Allow-list matcher cases: `Bash(git *)` matches `git log -1`; `Bash(cafleet *)` matches `cafleet --session-id ... message poll ...`; `Bash(mise //cafleet*)` matches `mise //cafleet:test` and `mise //cafleet:lint`; non-`Bash(...)` patterns are ignored; allow×deny truth table from §4 (yes/no → auto-run; yes/yes → ask; no/yes → ask; no/no → ask) with one row each; empty allow-list returns `ask` for everything. The matcher has no reject-threshold tests — those live in `test_cli_member_exec.py`. |
| `test_cli_member_exec.py` | Switch on `status` in every assertion (per the canonical-status rule); `exit_code` is asserted only for `status == "ran"`. (a) `cafleet member exec --cmd 'echo hi'` → JSON with `status: "ran"`, `exit_code: 0`, `stdout: "hi\n"`. (b) `cafleet member exec --cmd 'sleep 5' --timeout 1` → JSON with `status: "timeout"`, `stderr` containing `"hard-killed at 1 seconds."`; helper process exit code is 0. (c) `cafleet member exec --cmd 'python -c "print(\"x\" * 200000)"'` → truncated `stdout` ending in the truncation marker; `status: "ran"`. (d) `cafleet member exec --cmd 'cat' --stdin 'hello'` → `status: "ran"`, `stdout: "hello"`. (e) `cafleet member exec --cmd ''` → JSON with `status: "denied"`, `stderr: "bash_request.cmd may not be empty."`; helper process exit code is 0. (f) `cafleet member exec --cmd 'true' --timeout 9999` → JSON with `status: "denied"`, `stderr: "bash_request.timeout exceeds 600s cap."`; helper process exit code is 0. (g) `cafleet member exec --cmd 'true' --cwd '/no/such/dir'` → `status: "ran"`, `exit_code: 1`, `stderr` contains `"no such cwd"` (FileNotFoundError surfaced through the runtime path, not as a denied result). |
| `test_coding_agent.py::TestPromptTemplates` | `CLAUDE.default_prompt_template` contains the canary substring `"Routing Bash via the Director"`, contains literal doubled braces `{{` and `}}` if a JSON-envelope example is embedded (template safety per design 0000018), and passes `str.format(session_id=..., agent_id=..., director_name=..., director_agent_id=...)` with the standard kwargs without raising. (No codex template assertions: the CODEX template is removed in §15.) |
| `test_bash_dispatch.py::TestSingleConsentGate` (smoke-only) | Documents the single-consent-gate invariant: when the matcher returns `ask`, the Director's pane fires exactly one `AskUserQuestion` and zero native Bash-tool prompts (because `Bash(cafleet member exec *)` is in `permissions.allow`). Not unit-testable without harness mocking — covered by Implementation Step 7's real-world smoke (`b_with_allow` re-run case). |
| `test_cli_restructure.py::TestFlatVerbsRejected` (round 6) | For each old flat-verb subcommand, assert the invocation fails with Click's default `Error: No such command '<name>'.`. Cases: `cafleet send`, `cafleet poll`, `cafleet ack`, `cafleet cancel`, `cafleet broadcast`, `cafleet register`, `cafleet deregister`, `cafleet agents`, `cafleet get-task`, `cafleet bash-exec`. Regression guard against any future contributor accidentally re-adding a Click alias and silently re-enabling the flat form (which would break the hard-break SC). |
| `test_tmux.py::TestSendPollTriggerKeystroke` (round 6) | Monkey-patch `tmux._run` to capture argv. Call `tmux.send_poll_trigger(target_pane_id="%0", session_id="<uuid>", agent_id="<uuid>")` once and assert the captured keystroke string contains the literal `message poll` (not `poll`). Regression guard against any future revert of Step 12 task 5 — without it the flat-form keystroke would silently land in member panes and the bash-routing flow would break with `Error: No such command 'poll'.` at the recipient's pane. |
| Real-world smoke (see Implementation Step 7 / Step 16 for the canonical case lists) | The Director team itself uses `cafleet member create --no-bash` once landed. No separate disposable smoke. |

### 12. Edge cases

| Case | Behavior |
|---|---|
| Member sends a `bash_request` with `cmd: ""` (empty) | The Director invokes `cafleet member exec --cmd ''`; the helper short-circuits with denied JSON (`status: "denied"`, `stderr: "bash_request.cmd may not be empty."`); the Director relays it as `bash_result` with `status: "denied"`. (Helper input-validation, §3 helper bullet 1.) |
| Member sends a `bash_request` with `timeout: 9999` | The Director invokes `cafleet member exec --timeout 9999 ...`; the helper short-circuits with denied JSON (`status: "denied"`, `stderr: "bash_request.timeout exceeds 600s cap."`); the Director relays it as `bash_result` with `status: "denied"`. |
| Member sends malformed JSON or `type != "bash_request"` | NOT a denied `bash_result`. The Director treats the message as a plain instruction (existing logic). The member is responsible for using the documented JSON format if it wants the bash-routing dispatch. |
| Operator picks "Approve with edits" but submits an empty edited command | Director re-asks via `AskUserQuestion` with the same 3 options. The previous attempt is discarded. |
| Operator picks "Deny with reason" but submits an empty reason | Director uses the default `"Director denied the request."` for `stderr`. `status` is `"denied"`; `exit_code` is opaque. |
| Member's session is deleted between request and reply | The Director's reply `cafleet message send` fails with the soft-deleted-session error. The Director surfaces the error to the operator (who already approved/denied via AskUserQuestion) but does not retry. Per `broker.delete_session` semantics, the cascade marks every member agent as `deregistered` and physically deletes their `agent_placements`, so subsequent polls by the (now-deregistered) member return nothing — the orphaned reply task remains in `tasks` for forensic inspection but is invisible. |
| Member crashes between request send and reply receive | The reply task lands in the queue with `status: "input_required"`. If the same member is recreated (different `agent_id`), the reply is orphaned. Operator can `cafleet message cancel` the reply if they care; otherwise it stays as a soft-leak in `tasks` and is invisible to the new member. |
| Two `bash_request`s in flight from the same member | Director processes them in poll-order. Each reply carries its own `in_reply_to`; the member correlates by `task_id`. No serialization is enforced — the member is responsible for waiting for one reply before sending the next if it wants ordering. |
| Helper subprocess killed by SIGSEGV (not the timeout path) | `subprocess.run` returns a negative `returncode`; the helper reports `status: "ran"`, `exit_code: 128 + signal` (e.g. `139` for SIGSEGV), and stderr carries whatever the subprocess emitted before death. No special handling. |
| `bash_request.cwd` does not exist | Logically a payload error but mechanically the helper invokes the shell which raises `FileNotFoundError`; `status: "ran"` with non-zero `exit_code` is the closest fit. Helper emits `status: "ran"`, `exit_code: 1`, `stderr: "no such cwd: <path>"`. (See §3 helper subsection for the runtime path.) |
| The member uses `cafleet message send` directly without the JSON envelope | Director treats the message as a plain text instruction (existing behavior, no bash dispatch path). The member is responsible for using the documented JSON payload format. |
| Operator forgets to add `Bash(cafleet member exec *)` to `permissions.allow` | The single-consent-gate UX guarantee does NOT hold — the operator is prompted twice (AskUserQuestion + native Bash) per non-allowlisted command. The bash-routing flow still works; the only cost is duplicate prompts. The cafleet-monitoring skill setup checklist (§10) flags a missing entry as a setup gap. Future work (§13) may make `cafleet member create` warn on detection. |
| The operator closes the Director's tmux pane after AskUserQuestion fires or during `member exec` | The member is left blocked on the bash_result. On next session start, the operator runs `cafleet message poll --agent-id <director-id>` to surface the unanswered bash_request and resume the dispatch. The bash_request task is persisted in SQLite, so nothing is lost. |
| The broker server restarts mid-bash-request | The tmux push is best-effort and lost on restart, but the bash_request task persists in SQLite. After the broker comes back, the Director's `/loop` cron will surface the request on its next poll cycle, and the dispatch resumes normally. |
| Two or more different members send `bash_request`s in quick succession | Broker queues each as its own task; tmux pushes both to the Director's pane (best-effort, sequential). Director's `cafleet message poll` returns both in poll order. Dispatcher processes them sequentially — each gets its own AskUserQuestion turn before moving to the next. |

### 13. Future Work (pointer)

These are deliberately out of scope for v1 and listed here so reviewers do not block on them:

- **Per-Director throttling / memoization**: "Approve the next 5 `git status` calls from this member" without re-asking. Today every miss-the-allow-list call asks again.
- **Structured WebUI rendering**: Render `bash_request` / `bash_result` as collapsible panels with syntax-highlighted `cmd` and exit-code badges in the admin timeline.
- **Member-side timeouts**: Add `cafleet message send --reply-timeout <s>` so a member can declare it will give up if the Director does not reply in time. Today it blocks forever.
- **Typed `cafleet message bash-request` member-side subcommand**: Dedicated CLI surface that wraps `cafleet message send` with the JSON envelope built in, plus the cross-Director boundary check enforced at the CLI layer (Specification §5).
- **Codex restoration**: Codex was deprecated entirely in round 6 (§15). When upstream codex ships an equivalent of `--disallowedTools` (or any other primitive that gates the `shell` tool at the binary level) AND a stable JSON-payload interface for AskUserQuestion-style prompts (so the bash-routing flow's three-beat shape from designs 0000027/0000033 has a codex-side counterpart), restore `CodingAgentConfig.CODEX` and the `--coding-agent codex` flag with full bash-routing parity. The restoration design doc should: (i) reference 0000034 as prior art for the deprecation rationale; (ii) reintroduce the `CODEX` constant with the new `disallow_tools_args` populated from the codex flag spelling; (iii) reintroduce `CODING_AGENTS` / `get_coding_agent()` if more than one config emerges, or keep direct imports if only two; (iv) restore `cafleet member create --coding-agent codex` with full `--no-bash` parity (which the new codex primitive should make possible without the round-5 rejection branch); (v) add a migration check for surviving `agent_placements` rows with `coding_agent='codex'` (the rows that survived the 0000034 hard-break), validating they spawn under the new codex config; (vi) update every doc that 0000034 round 6 narrowed to "claude-only" (ARCHITECTURE.md "Multi-runner support" / "Pane display-name propagation", README features bullet, SKILL.md Member Create section, data-model.md `coding_agent` column docstring); (vii) re-add `CODEX.default_prompt_template` with the inlined JSON envelope (round 5c shipped this template; round 6 deleted it via Step 13 task 1; restoration must reintroduce a parallel template so codex members get the same bash-routing reminder claude members get); (viii) revert the round-6 Step 13 task 3 `FIXME(claude)` comment edit at `cafleet/src/cafleet/broker.py:17` so the codex env-var auto-detection note is restored alongside the codex agent type itself; (ix) remove the round-6 Step 14 regression-guard tests (`TestCodingAgentFlagRemoved` in `test_cli_member.py` and `TestCodexConstantRemoved` in `test_coding_agent.py`) — both will start failing the moment codex is restored because the flag and the constant exist again. Replace with positive tests that assert codex `--coding-agent` parsing and `CODEX` import both succeed.
- **Stronger matcher**: full glob / regex semantics if `permissions.allow` patterns drift past the `Bash(<token0> *)` and `Bash(<exact-prefix>*)` shapes the v1 matcher covers.
- **Operator-setup verifier**: `cafleet member create` reads the Director's resolved `settings.json` and warns when `Bash(cafleet member exec *)` is absent, since the single-consent-gate UX guarantee depends on it.
- **Round-7 subscribe primitive**: replace today's pull-based `cafleet message poll` plus the broker's tmux-keystroke push with a long-running `cafleet subscribe` (or wherever it lands under the round-6 `message` group). Deferred so the nested restructure (§14) lands first; subscribe is a separate design doc.

---

### 14. Nested-only subcommand restructure (round 6)

The existing CLI mixes flat verbs with nested groups: `cafleet send`, `poll`, `ack`, `cancel`, `broadcast`, `register`, `deregister`, `agents`, `get-task` are top-level, while `member create`, `session create`, `db init` are nested. Round 6 collapses the asymmetry by moving every entity-scoped operation under its noun group. Two meta-commands stay top-level by exception: `server` and `doctor` (they operate on the local OS, not a CAFleet entity).

**Hard-break, no aliases.** Every literal occurrence of an old subcommand string in source code, prompt templates, tmux keystroke injection, SKILL.md files, README, ARCHITECTURE, docs/spec, admin SPA, and this design doc is updated in the same documentation-first sweep. The user has explicitly accepted the blast radius.

#### Final shape

```
cafleet                        # binary unchanged
├── agent                       (new group)
│   ├── register                # was: cafleet register
│   ├── deregister              # was: cafleet deregister
│   ├── list                    # was: cafleet agents
│   └── show --id <x>           # was: cafleet agents --id <x> (now its own subcommand)
├── message                     (new group)
│   ├── send                    # was: cafleet send
│   ├── broadcast               # was: cafleet broadcast
│   ├── poll                    # was: cafleet poll
│   ├── ack                     # was: cafleet ack
│   ├── cancel                  # was: cafleet cancel
│   └── show --task-id <x>      # was: cafleet get-task
├── member                      (existing group, gains new subcommand)
│   ├── create
│   ├── delete
│   ├── list
│   ├── capture
│   ├── send-input
│   └── exec                    # new in this design (was bash-exec — see ripple in §3 / §6)
├── session                     (existing group, unchanged)
│   ├── create
│   ├── list
│   ├── show
│   └── delete
├── db                          (existing group, unchanged)
│   └── init
├── server                      # exception: meta-command
└── doctor                      # exception: meta-command
```

#### Rule for top-level exceptions

Every command that operates on a CAFleet entity (agent, message, member, session, db row) lives under that entity's group. Meta-commands that operate on the local OS or the broker process — `server` (start the FastAPI app), `doctor` (print local tmux context) — stay top-level. The same exception keeps `--version` and `--json` as global flags rather than promoted into a group. Future meta-commands follow the same rule.

#### Subcommand mapping

| Old (current) | New (nested) | Notes |
|---|---|---|
| `cafleet register` | `cafleet agent register` | Behavior unchanged. Returns the new `agent_id`. |
| `cafleet deregister` | `cafleet agent deregister` | Behavior unchanged. Root-Director / Administrator protections preserved. |
| `cafleet agents` | `cafleet agent list` | Behavior unchanged. The `agents --id <x>` form moves to `agent show`. |
| `cafleet agents --id <x>` | `cafleet agent show --id <x>` | Materialized as its own subcommand. |
| `cafleet send` | `cafleet message send` | Behavior unchanged. |
| `cafleet broadcast` | `cafleet message broadcast` | Behavior unchanged. |
| `cafleet poll` | `cafleet message poll` | Behavior unchanged. (Round 7 may rename this further; round 6 only moves it.) |
| `cafleet ack` | `cafleet message ack` | Behavior unchanged. |
| `cafleet cancel` | `cafleet message cancel` | Behavior unchanged. |
| `cafleet get-task` | `cafleet message show --task-id <x>` | Renamed verb (`get-task` → `show`) for taxonomy uniformity with `agent show` / `session show`. The `--task-id` flag stays explicit so the noun-verb form is unambiguous. |
| `cafleet member create` | `cafleet member create` | Unchanged location. Internal: `--coding-agent` flag is removed (§15). |
| `cafleet member delete` | `cafleet member delete` | Unchanged. |
| `cafleet member list` | `cafleet member list` | Unchanged. |
| `cafleet member capture` | `cafleet member capture` | Unchanged. |
| `cafleet member send-input` | `cafleet member send-input` | Unchanged. |
| (new from this doc) | `cafleet member exec` | The bash-routing helper. (`bash-exec` from rounds 1–5c renamed to `member exec` per the round-6 rename ripple; semantics unchanged. The required `permissions.allow` entry is correspondingly `Bash(cafleet member exec *)`.) |
| `cafleet session create` | `cafleet session create` | Unchanged. |
| `cafleet session list` | `cafleet session list` | Unchanged. |
| `cafleet session show` | `cafleet session show` | Unchanged. |
| `cafleet session delete` | `cafleet session delete` | Unchanged. |
| `cafleet db init` | `cafleet db init` | Unchanged. |
| `cafleet server` | `cafleet server` | Top-level meta-command exception. Unchanged. |
| `cafleet doctor` | `cafleet doctor` | Top-level meta-command exception. Unchanged. |

#### Click implementation

`cli.py` reorganizes to:

```python
@click.group()
@click.option("--json", ...)
@click.option("--session-id", ...)
def cli(...): ...

@cli.group()
def agent() -> None:
    """Agent registry commands."""

@cli.group()
def message() -> None:
    """Message broker commands."""

@cli.group()
def member() -> None:
    """Member lifecycle commands."""

@cli.group()
def session() -> None:
    """Session lifecycle commands."""

@cli.group()
def db() -> None:
    """Database schema management commands."""

@agent.command("register")
def agent_register(...): ...
# ... etc — each existing handler moves under its group.

@cli.command("server")
def server(...): ...   # top-level exception

@cli.command("doctor")
def doctor(...): ...   # top-level exception
```

The handler bodies are unchanged — only their click decorator changes (`@cli.command()` → `@<group>.command(...)`). Help text on the parent groups documents the noun-verb structure.

#### Hard-coded literal strings to update

Every string that today contains `cafleet <flat-verb>` or `cafleet --session-id <s> <flat-verb>` MUST be updated. The exhaustive inventory:

| Surface | What changes |
|---|---|
| `cafleet/src/cafleet/cli.py` | Group reorganization (above). |
| `cafleet/src/cafleet/coding_agent.py` | The `CLAUDE.default_prompt_template` injects literal `cafleet --session-id {session_id} message poll --agent-id {agent_id}` (was `... poll ...`). The CODEX template is deleted entirely (§15). |
| `cafleet/src/cafleet/tmux.py` (`send_poll_trigger`) | The keystroke string changes from `cafleet --session-id <s> poll --agent-id <r>` to `cafleet --session-id <s> message poll --agent-id <r>`. |
| `admin/src/components/Sidebar.tsx:56` | `"cafleet register"` → `"cafleet agent register"`. |
| `admin/src/components/Dashboard.tsx:76` | `"cafleet db init"` → `"cafleet db init"` (unchanged — `db init` already nested). |
| `admin/src/components/Dashboard.tsx:88` | `"cafleet register"` → `"cafleet agent register"`. |
| `admin/src/components/SessionPicker.tsx:53` | `"cafleet session create"` → `"cafleet session create"` (unchanged — already nested). |
| `ARCHITECTURE.md` | Every example invocation updates. The Operation Mapping table (CLI Command → broker function) updates. |
| `README.md` | Every example invocation. |
| `cafleet/CLAUDE.md` | Every example invocation. |
| `docs/spec/cli-options.md` | Group restructure: the doc now organizes commands by noun group. Every section heading updates. |
| `docs/spec/data-model.md` | If any literal commands appear, update. |
| `skills/cafleet/SKILL.md` | Every example, every Command Reference entry, every Multi-Session Coordination invocation. |
| `skills/cafleet-monitoring/SKILL.md` | The `/loop` prompt template's literal commands; every Stage 1 / Stage 2 / Escalation table entry. |
| `skills/design-doc-create/roles/director.md` | Every literal cafleet invocation. |
| `skills/design-doc-execute/roles/director.md` | Every literal cafleet invocation. |
| `design-docs/0000034-member-bash-via-director/design-doc.md` | This doc itself — round-6 changelog entry confirms the in-place renames. Earlier-round changelog rows are preserved verbatim as historical record. |

mise tasks (`mise.toml` / `cafleet/mise.toml`) and the `permissions.allow` patterns (`Bash(cafleet *)` is a wildcard, unaffected) are zero-work surfaces.

#### Compatibility / migration

There is none. Hard-break: any process that invokes `cafleet send`, `cafleet poll`, etc. after this lands fails with Click's default `Error: No such command 'send'.` (or similar). Operators retrain on the new shape. The user has accepted this blast radius explicitly.

The one place this matters most for in-flight processes: existing member panes will receive `tmux send-keys` of the OLD keystroke string (`cafleet --session-id ... poll ...`) until the broker is restarted with the new code. Once `tmux.send_poll_trigger` is updated, those keystrokes become `cafleet --session-id ... message poll ...`, but in-flight panes that already saw the old string and ran it would have already errored — `cafleet poll` no longer exists. The smoke step (Implementation) verifies a fresh member pane round-trips correctly under the new keystroke.

---

### 15. Codex deprecation (round 6)

CAFleet drops codex support entirely. The existing `CodingAgentConfig.CODEX` constant, the `CODING_AGENTS` dict, the `get_coding_agent()` helper, the `--coding-agent` flag on `cafleet member create`, and every codex-aware doc / SKILL / test path are removed. Claude is the only supported member backend.

#### Rationale

Codex has no `--disallowedTools` analog. Round-5c's design accommodated this by carving out a "prompt-only discipline" fallback for codex members and rejecting `--no-bash --coding-agent codex` at the CLI. That fallback works but creates a tax on every change in the bash-routing flow: new test rows for codex paths, new doc rows in §6 / §10 / §11 / §12, new spawn-prompt-template plumbing in `coding_agent.py`. The user has decided the tax is not worth it for v1 — codex support will be restored later via a future design doc when codex grows the equivalent enforcement primitives (§13 Future Work).

#### Migration

The user has accepted a hard-break with the following explicit migration story:

1. **CLI surface**: `cafleet member create --coding-agent codex` no longer parses. Anyone passing the flag gets Click's default `Error: No such option: '--coding-agent'.` (the flag is removed entirely, not narrowed to a single-value choice — see FQ5 round-6 answer).
2. **In-flight codex panes survive**: Any `agent_placements` row with `coding_agent='codex'` predating the rename keeps its row and its tmux pane. The broker does NOT auto-cleanup codex placements. The pane's `codex` process keeps running until either the operator runs `cafleet member delete` for it or the session is soft-deleted (which deregisters the agent and drops the placement row).
3. **No broker-side guard**: Per FQ6 round-6 answer, the rejection is purely at the CLI flag layer. The broker continues to accept any string in the `agent_placements.coding_agent` column. Surviving `coding_agent='codex'` rows render in `cafleet member list` exactly as today.
4. **No auto-cleanup pass**: Operators retire codex members manually with `cafleet member delete` per pane. There is no "kill all codex panes on next startup" sweep. If `cafleet member delete` blocks on the codex pane (codex's `/exit` may not reliably terminate the process the way claude's does), use `cafleet member delete --force` to skip `/exit` and kill-pane immediately. The `--force` flag is the same one introduced for claude members in design 0000032 and works identically for codex placements — there is no codex-specific retirement command.
5. **Data preservation**: The `agent_placements.coding_agent` column stays `TEXT NOT NULL DEFAULT 'claude'` — codex rows are preserved for forensic visibility and for the §13 restoration plan's migration check.

#### Code surface

| File | Codex-drop edits |
|---|---|
| `cafleet/src/cafleet/coding_agent.py` | Delete `CODEX = CodingAgentConfig(...)`. Delete `CODING_AGENTS = {...}` registry. Delete `get_coding_agent()`. The module collapses to: `CodingAgentConfig` dataclass + a single module-level `CLAUDE` instance. Callers (`cli.member_create`) `from cafleet.coding_agent import CLAUDE` directly. Round-5c's `disallow_tools_args` field on `CodingAgentConfig` stays — it earns its place even with one config and round-7's restoration is a one-line edit (re-add `CODEX = ...`). |
| `cafleet/src/cafleet/cli.py` | Remove `--coding-agent` flag from `member_create` (Click pattern: delete the `@click.option("--coding-agent", ...)` decorator and the corresponding handler kwarg). Remove the round-5c `--no-bash --coding-agent codex` rejection branch (now unreachable). Member creation always uses `CLAUDE.build_command(...)`. |
| `cafleet/src/cafleet/broker.py` | Update the `FIXME(claude)` comment at line 17 to drop the codex env-var reference. No runtime gate is added (FQ6 round-6 answer). |
| `cafleet/tests/test_coding_agent.py` | Delete every codex-specific test case (`TestCodexBuildCommand`, `TestCodexDisplayName`, registry-lookup tests, etc.). Round-5c's `TestPromptTemplates` shrinks to claude-only (the CODEX template no longer exists). |
| `cafleet/tests/test_cli_member.py` | Delete `--coding-agent` flag tests; delete `--no-bash --coding-agent codex` rejection tests. Add one new test asserting `cafleet member create --coding-agent codex` exits with Click's `Error: No such option: '--coding-agent'.` (regression guard). Round-5c's `TestNoBashFlag` shrinks from 4 sub-cases to 2 (just claude default `--no-bash` + explicit `--allow-bash`). |
| `cafleet/tests/test_tmux.py` | Drop codex-specific spawn-command tests if any are codex-specific; verify that mocked `_run` tests are agent-agnostic (most are). |
| `cafleet/tests/test_output.py` | Drop codex backend strings from format tests. |

#### Doc surface

| File | Codex-drop edits |
|---|---|
| `ARCHITECTURE.md` | Drop the "Multi-runner support" paragraph (lines ≈159 of round-5c text) entirely. Rewrite the "Pane display-name propagation" paragraph (lines ≈161) to drop CODEX/codex examples — only the claude case remains. Collapse the `coding_agent.py` Component Layout row (lines ≈66) to "claude-only spawn config." Update the "Spawn the coding agent (Claude or Codex, selected via `--coding-agent`)" line (≈151) to "Spawn the claude member pane via `tmux split-window`." |
| `README.md` | Drop the multi-runner Features bullet. |
| `docs/spec/cli-options.md` | Remove the `--coding-agent` flag row from `member create`. Add a one-sentence note: "codex support was removed in design 0000034 (§15); see §13 Future Work for restoration plan." |
| `docs/spec/data-model.md` | Line 126 — narrow the `coding_agent` column docstring from `"claude" or "codex"` to `"claude" (codex deprecated as of design 0000034 §15; existing rows preserved for forensic visibility and round-7 restoration)`. The column type, nullability, and `DEFAULT 'claude'` server default are unchanged. |
| `cafleet/CLAUDE.md` | Verify and trim any codex references discovered during the inventory. |
| `skills/cafleet/SKILL.md` | Remove the `--coding-agent` flag row from `### Member Create`. Remove every `--coding-agent codex` example invocation. Remove every codex-specific output sample. |
| `skills/cafleet-monitoring/SKILL.md` | Reframe "Agent-agnostic monitoring" copy as "claude-only monitoring (codex deprecated as of design 0000034 §15; until codex grows enforcement primitives, route any codex pane through manual `cafleet member capture` only — `cafleet member exec` and `--no-bash` do not apply)." |
| `skills/design-doc-create/roles/director.md` | If any codex references exist, drop them. |
| `skills/design-doc-execute/roles/director.md` | Same. |

#### Tests

| Test | Coverage |
|---|---|
| `test_cli_member.py::TestCodingAgentFlagRemoved` | `cafleet member create --coding-agent claude` fails with Click's `Error: No such option: '--coding-agent'.` (regression guard so no future contributor accidentally re-adds the flag). |
| `test_coding_agent.py::TestCodexConstantRemoved` | Importing `CodingAgentConfig` works; importing `CODEX` raises `ImportError`; importing `CODING_AGENTS` raises `ImportError`; importing `get_coding_agent` raises `ImportError`. (Regression guard against re-adding the registry.) |

The §13 Future Work entry covers the restoration plan in detail — the deprecation is explicitly scoped to "remove now, restore later when codex catches up."

#### Rollback

If round 6 needs to be reverted (broken behavior, operator pushback, etc.), the rollback shape is **revert the round-6 atomic PR** (Steps 11 / 12 / 13 / 14 land as one commit per Step 12's atomic-landing instruction, so reverting that one commit restores round-5c CLI shape including codex). DB schema is unchanged across the round-6 boundary: the `agent_placements.coding_agent` column persists with `DEFAULT 'claude'` and accepts both `'claude'` and `'codex'` strings, so a revert is safe — no migration script is needed in either direction. In-flight codex placements continue working under the restored round-5c CLI (`cafleet member create --coding-agent codex` parses again, the codex prompt template is back).

Active claude member panes that received post-round-6 keystrokes (e.g. `cafleet --session-id <s> message poll --agent-id <r>` from `tmux.send_poll_trigger`) may be wedged on `Error: No such command 'message'.` after the rollback restores the flat-form binary. The operator retires those wedged panes via `cafleet member delete --force` and respawns; the broker's tmux push will then inject the round-5c-era flat-form keystroke `cafleet poll` and the new pane processes it correctly.

The codex restoration plan in §13 (i)–(ix) is **Future Work, not a rollback path** — if codex needs to come back as an end-state (i.e., the user reverses the round-6 deprecation decision rather than reverting the entire round-6 PR), write the restoration design doc instead. Rollback is for "round 6 was a mistake, revert the whole thing"; restoration is for "codex is back on the menu in v2."

---

## Implementation

> Documentation must be updated **before** any code change (per `.claude/rules/design-doc-numbering.md`).
> Task format: `- [x] Done task <!-- completed: 2026-04-28T14:30 -->`

### Step 1: Documentation — top-level project docs

- [x] Update `ARCHITECTURE.md`: add a new `## Bash Routing via Director` subsection after `## Member Lifecycle`. Cover the `--no-bash` flag, the `cafleet bash-exec` helper (round-5c-era name; Step 12 task 2 renames it later), the JSON payload schemas, and the no-timeout member semantics. Cross-link `skills/cafleet/SKILL.md`. (Codex-related copy deletions are owned by Step 9 task 1.) <!-- completed: 2026-04-28T15:00 -->
- [x] Update `docs/spec/cli-options.md`: (a) under the `cafleet member create` subsection, add `--no-bash` AND `--allow-bash` flag entries. (b) Add a new top-level subsection `### bash-exec` (round-5c-era placement; Step 9 task 3 reorganizes the doc by noun group and renames this to `### member exec`) documenting the helper's flags, JSON output schema, exit codes, and 64 KiB / 30 s limits. (`--coding-agent` flag removal is owned by Step 9 task 3.) <!-- completed: 2026-04-28T15:00 -->
- [x] Update `README.md`: in the member-commands bullet list, add the `cafleet member create --no-bash` entry; in the top-level CLI bullets, add a one-line `cafleet bash-exec` entry (round-5c-era name; Step 9 task 3 renames it). Both link to the cafleet skill. <!-- completed: 2026-04-28T15:00 -->

### Step 2: Documentation — project-local skills

- [x] Update `skills/cafleet/SKILL.md`: (a) extend the `### Member Create` subsection with `--no-bash` / `--allow-bash` flag rows. (b) Add a new top-level `## Routing Bash via the Director` section after `### Answer a member's AskUserQuestion prompt`. Include both payload schemas verbatim, the 3-option AskUserQuestion shape, the auto-allow path, the **required** `Bash(cafleet bash-exec *)` allow rule, the cross-Director rule, and the no-timeout note. (c) Add `### Bash Exec` to Command Reference, between `Member Send-Input` and `Server`. (Codex-related deletions and the `bash-exec` → `member exec` rename are owned by Step 10 task 1.) <!-- completed: 2026-04-28T15:40 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md`: add a row to the Stall Response escalation table for the `bash_request` blocking case. Reference the new cafleet-skill section. <!-- completed: 2026-04-28T15:40 -->
- [x] Update `skills/design-doc-create/roles/director.md`: add a one-paragraph note pointing at the cafleet skill for the bash-routing workflow. <!-- completed: 2026-04-28T15:40 -->
- [x] Update `skills/design-doc-execute/roles/director.md`: same edit, mirrored. <!-- completed: 2026-04-28T15:40 -->

### Step 3: Code — `CodingAgentConfig` extension and spawn-prompt amendments

- [ ] Add `disallow_tools_args: tuple[str, ...] = ()` field to `CodingAgentConfig` in `cafleet/src/cafleet/coding_agent.py`. Set `CLAUDE.disallow_tools_args = ("--disallowedTools", "Bash")` and `CODEX.disallow_tools_args = ()`. (Round-5c-era state. CODEX deletion is owned by Step 13 task 1.) <!-- completed: -->
- [ ] Extend `CodingAgentConfig.build_command(...)` with a `deny_bash: bool = False` keyword. When `deny_bash=True` AND `disallow_tools_args` is non-empty, inject the tokens. Pinned argv ordering: `[binary, *extra_args, *deny_args, *name_args, prompt]` — deny_args BEFORE name_args. (Mirrors §1's snippet exactly.) <!-- completed: -->
- [ ] Update `CLAUDE.default_prompt_template` to add the bash-routing reminder. New round-5c-era template (Step 12 task 6 renames the literal `cafleet poll` invocation to `cafleet message poll` as part of the round-6 nested-only restructure):
  ```
  Load Skill(cafleet). Your session_id is {session_id} and your agent_id is {agent_id}.
  You are a member of the team led by {director_name} ({director_agent_id}).
  Wait for instructions via `cafleet --session-id {session_id} poll --agent-id {agent_id}`.
  Your Bash tool is denied. Route any shell command through your Director —
  see Skill(cafleet) > Routing Bash via the Director for the bash_request JSON envelope.
  ```
  <!-- completed: -->
- [ ] Update `CODEX.default_prompt_template` to add the same bash-routing reminder, inlined since codex has no skills to load. New round-5c-era template (Step 13 task 1 deletes the entire CODEX constant + this template as part of the round-6 codex deprecation; until then the template lives at parity with CLAUDE):
  ```
  Your session_id is {session_id} and your agent_id is {agent_id}.
  You are a member of the team led by {director_name} ({director_agent_id}).
  Check for instructions using `cafleet --session-id {session_id} poll --agent-id {agent_id}`.
  Use `cafleet --session-id {session_id} ack --agent-id {agent_id} --task-id <id>` to acknowledge messages
  and `cafleet --session-id {session_id} send --agent-id {agent_id} --to <id> --text "..."` to reply.

  When you need to run a shell command, do NOT use your shell tool directly. Instead send a JSON
  bash_request to your Director ({director_agent_id}) via `cafleet send`:
    {{"type":"bash_request","cmd":"<shell-command>","cwd":"<absolute-path>","reason":"<short-reason>"}}
  Then poll for a bash_result reply correlated by in_reply_to == <your-send-task-id>.
  ```
  Note: literal `{` / `}` in the JSON example are doubled (`{{` / `}}`) so `str.format()` collapses them per the design 0000018 template-safety rule. <!-- completed: -->

### Step 4: Code — `cafleet member create` flag and `cafleet member exec` helper

- [ ] Add `--no-bash` / `--allow-bash` boolean flag pair to `member_create` in `cafleet/src/cafleet/cli.py:555`. Click pattern: `@click.option("--no-bash/--allow-bash", default=None)`. Resolve the per-coding-agent default in the handler body: `claude` → True if unset; `codex` → False if unset and reject if explicitly True with the verbatim error message in §6 round-5c text. Pass `deny_bash=resolved_no_bash` into `coding_agent_config.build_command(...)` at the existing `tmux.split_window(... command=...)` site. (Round-5c-era state. Step 13 task 2 removes the `--coding-agent` flag and the codex rejection branch as part of the round-6 codex deprecation.) <!-- completed: -->
- [ ] In the `member_create` handler body, after resolving `coding_agent_config`, reject `--no-bash --coding-agent codex` with the verbatim error message in §6 round-5c text. Exit 1 BEFORE the `register_agent` call so no broker rows are created. (Round-5c-era state. Step 13 task 2 removes this branch in round 6.) <!-- completed: -->
- [ ] Add a new top-level `cafleet bash-exec` click command in `cafleet/src/cafleet/cli.py` (round-5c-era name and placement; Step 12 task 2 renames it to `cafleet member exec` under the `member` group as part of the round-6 nested-only restructure). Flags: `--cmd` (required, accepts empty), `--cwd` (optional), `--timeout` (optional, default 30; the helper itself validates `1 <= timeout <= 600` rather than Click — see input-validation note below), `--stdin` (optional). Handler order: (1) **Input validation** — if `cmd == ""` OR `timeout > 600`, write a denied JSON object (`{"status": "denied", "exit_code": 126, "stdout": "", "stderr": "<reason>", "duration_ms": 0}`) to stdout and exit 0 (do NOT raise Click UsageError; the validation failure is a payload-level outcome, not a CLI-arg error). (2) **Run** — call `subprocess.run(["bash", "-c", cmd], cwd=cwd, input=stdin, timeout=timeout, capture_output=True)`. On `subprocess.TimeoutExpired`, hard-kill via `Popen.kill()` and emit `status: "timeout"` (exit_code internally `124`, but doc treats it as opaque per the canonical-status rule). Truncate stdout/stderr at 64 KiB with the exact marker spec. Print exactly one JSON object on stdout. Helper process exit code is 0 for every payload outcome (ran/denied/timeout); non-zero only for Click's own UsageError on unknown flags. Lazy-import `subprocess` inside the handler to keep CLI startup cheap. The command silently accepts and ignores `--session-id` (matches the `db init` / `session *` / `server` pattern). <!-- completed: -->
- [ ] Add a `cafleet/src/cafleet/bash_routing.py` module exposing the three pinned-signature helpers below plus a `BashRequest` / `BashResult` dataclass pair (or TypedDicts) for type narrowing. Truncation-marker logic lives in the `cafleet member exec` helper's own output (§3 helper bullet 3); field-level validation lives in the helper too (§3 helper bullet 1). `parse_bash_request` is a pure JSON-shape parser; `format_bash_result` is a pure formatter that wraps the helper's already-truncated streams with the discriminator and audit fields.

  ```python
  def match_allow(
      cmd: str,
      allow_patterns: list[str],
      deny_patterns: list[str],
  ) -> Literal["auto-run", "ask"]:
      """Apply the §4 allow×deny truth table. No auto-deny outcome."""

  def parse_bash_request(text: str) -> BashRequest | None:
      """Parse a polled `text` body. Returns None for non-bash_request shapes
      (parse fail / missing type / type != 'bash_request') — those are NOT
      bash-routing payloads. Does NOT validate field semantics (empty cmd,
      oversized timeout): those are the helper's responsibility (see
      `cafleet member exec` input-validation bullet in §3). Returns the parsed
      object as-is for any payload with `type == 'bash_request'`."""

  def format_bash_result(
      *,
      in_reply_to: str,
      status: Literal["ran", "denied", "timeout"],
      exit_code: int,
      stdout: str = "",
      stderr: str = "",
      duration_ms: int = 0,
      note: str | None = None,
  ) -> str:
      """Return a JSON-encoded bash_result payload suitable for `cafleet message send --text`.
      Pure formatter — does NOT truncate (caller passes already-truncated streams).
      Per the canonical-status rule, callers passing `status != 'ran'` may set
      `exit_code` to any value; clients are required to switch on `status`, so
      the value is opaque on denied/timeout paths."""
  ```
  <!-- completed: -->

### Step 5: Tests

- [ ] Extend `cafleet/tests/test_coding_agent.py` with `TestDisallowTools` AND `TestPromptTemplates` cases from §11 round-5c text (argv-shape pinning for `TestDisallowTools` against both CLAUDE and CODEX; canary-substring + `str.format()` smoke for `TestPromptTemplates` against both CLAUDE and CODEX templates). (Round-5c-era state. Step 14 task 2 deletes every codex-specific test case in this file as part of the round-6 codex deprecation.) <!-- completed: -->
- [ ] Extend `cafleet/tests/test_cli_member.py` with `TestNoBashFlag` cases from §11 round-5c text (four sub-cases: claude default `--no-bash`; explicit `--allow-bash`; codex `--no-bash` rejection with verbatim error; codex default `--allow-bash`). Reuse the existing fixture style; monkey-patch `tmux.split_window` to capture the `command` argv. (Round-5c-era state. Step 14 task 1 / task 3 prune the codex sub-cases and add `TestCodingAgentFlagRemoved` regression guard as part of the round-6 codex deprecation.) <!-- completed: -->
- [ ] Add a new file `cafleet/tests/test_bash_routing_payload.py` covering `parse_bash_request` / `format_bash_result` helpers from Step 4 (round-trip, missing-field errors, truncation marker exact match). <!-- completed: -->
- [ ] Add a new file `cafleet/tests/test_bash_routing_matcher.py` covering `match_allow` (per-pattern matches; allow×deny truth table from §4; ignored non-`Bash(...)` patterns; empty list returns `ask`). <!-- completed: -->
- [ ] Add a new file `cafleet/tests/test_cli_bash_exec.py` covering the `cafleet bash-exec` helper's seven cases from §11 round-5c text (happy path; timeout / SIGKILL; truncation; stdin propagation; empty-cmd denied JSON; over-cap timeout denied JSON; nonexistent-cwd runtime path). All assertions switch on `status` per the canonical-status rule; `exit_code` is asserted only for `status == "ran"`. Use `CliRunner`; for the timeout test prefer a real `sleep` invocation since `subprocess.run` timeout behavior is what's under test. (Round-5c-era file name and CLI-invocation strings. Step 14 task 1 renames the file to `test_cli_member_exec.py` and rewrites every `cafleet bash-exec` invocation inside it to `cafleet member exec` as part of the round-6 nested-only restructure.) <!-- completed: -->

### Step 6: Quality gates

- [ ] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: -->
- [ ] Run `mise //cafleet:lint` — must pass. <!-- completed: -->
- [ ] Run `mise //cafleet:format` — must pass. <!-- completed: -->
- [ ] Run `mise //cafleet:typecheck` — must pass. <!-- completed: -->

### Step 7: Real-world smoke (round 5c)

> Step 7 is the round-5c end-state smoke; it exercises the bash-routing flow under the round-5c CLI shape (flat `cafleet poll`, top-level `cafleet bash-exec`). Step 16 is the round-6 end-state smoke that re-exercises the same flow under the nested CLI shape (`cafleet message poll`, `cafleet member exec`). Both smokes are live tasks because round 5c and round 6 land as separate states in the implementation order; whoever is running the implementation can collapse them only if Steps 1–17 land in a single PR (in which case Step 7 is redundant and Step 16 is canonical).

- [ ] Spawn a fresh member via `cafleet member create --no-bash` (claude). The argv-shape check is covered by `test_cli_member.py::TestNoBashFlag` — Step 7 only verifies behavior end-to-end. Have the member attempt a Bash call; verify the harness rejects it (this is the behavioral proof that `--disallowedTools Bash` took effect). Have the member send a `bash_request` JSON message and round-trip a real shell command through each of these cases:
  - (a) **auto-allow path**: `git log -1 --oneline` matching `Bash(git *)` already in `permissions.allow`. Verify no `AskUserQuestion` fires; the `bash_result` carries `note: "ran without operator prompt (matched allow rule: ...)"`.
  - (b) **`ask` path with `Approve as-is`**: e.g. `whoami` (assumed not in allow-list). Verify the 3-option `AskUserQuestion` fires with the expected labels; operator picks option 1; `bash_result` arrives with `status: "ran"`.
  - (c) **`ask` path with `Approve with edits`**: same starting cmd; operator picks option 2 (built-in Other), edits the cmd; `bash_result` carries `note: "operator edited cmd before running. original: ..."`.
  - (d) **`ask` path with `Deny with reason`**: operator picks option 3; `bash_result` carries `status: "denied"` (per the canonical-status rule, do not assert on `exit_code`), the typed reason in `stderr`, and `note: "operator denied: <reason>"`.
  - (e) **timeout path on the auto-allow lane**: a member sends `bash_request {cmd: "sleep 60", timeout: 1}` with `Bash(sleep *)` (or equivalent) added to `permissions.allow`. The matcher routes to auto-allow so AskUserQuestion is bypassed; the helper hard-kills the subprocess at 1 second; `bash_result` carries `status: "timeout"` (per the canonical-status rule, do not assert on `exit_code`), `stderr` containing `"hard-killed at 1 seconds."`. This binds the timeout test to the auto-allow path so it purely exercises the helper's SIGKILL behavior.

  Verify each `bash_result` JSON arrives back at the member's pane via `cafleet poll` (round-5c flat-form keystroke; Step 12 task 5 renames it to `cafleet message poll` in round 6) and the member resumes. Cases (a)–(e) above assume the operator has the **required** `Bash(cafleet bash-exec *)` entry installed in `permissions.allow` per §6 round-5c text — only the AskUserQuestion fires, never the native Bash-tool prompt. To verify the §12 "operator forgets" degraded-UX edge case, temporarily remove `Bash(cafleet bash-exec *)` from `permissions.allow` and re-run case (b). Confirm BOTH the AskUserQuestion AND the native Bash-tool prompt fire (the documented degraded UX). Restore the allow rule afterward. <!-- completed: -->

### Step 8: Finalize (round 5c)

- [ ] Update Status to Complete and refresh Last Updated. <!-- completed: -->
- [ ] Add a Changelog entry. <!-- completed: -->

### Step 9: Documentation — round-6 nested-only restructure (top-level docs)

- [ ] Update `ARCHITECTURE.md`: rewrite every cafleet invocation to its nested form (`cafleet message send`, `cafleet message poll`, etc.). Update the Operation Mapping table (CLI Command → broker function) so the left column carries the new nested commands. Drop codex copy per §15 (Multi-runner support paragraph removed; Pane display-name propagation paragraph rewritten claude-only; coding_agent.py Component Layout row collapsed). <!-- completed: -->
- [ ] Update `README.md`: rewrite every cafleet invocation example. Drop the multi-runner Features bullet (codex §15). <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md`: reorganize sections by noun group (`agent`, `message`, `member`, `session`, `db`) plus the two top-level meta-command exceptions (`server`, `doctor`). Every example invocation updates. Remove the `--coding-agent` flag row from `member create` and add the codex-deprecation pointer per §15. <!-- completed: -->
- [ ] Update `docs/spec/data-model.md`: narrow the `coding_agent` column docstring to claude-only per §15. <!-- completed: -->
- [ ] Update `cafleet/CLAUDE.md`: rewrite every cafleet invocation example to its nested form; drop codex references per §15. <!-- completed: -->

### Step 10: Documentation — round-6 project skills

- [ ] Update `skills/cafleet/SKILL.md`: every command-reference entry, every Multi-Session Coordination invocation, every Typical Workflow step, and every example invocation moves to the nested form. Drop the `--coding-agent` flag row from `### Member Create` and every `--coding-agent codex` example (§15). Add `### Member Exec` (already specified in §10). <!-- completed: -->
- [ ] Update `skills/cafleet-monitoring/SKILL.md`: the `/loop` prompt template's literal commands move to nested form; every Stage 1 / Stage 2 / Escalation table entry updates; reframe "Agent-agnostic monitoring" copy as claude-only per §15. Add an explicit instruction near the Stall Response section: "When the Director's poll output contains a `bash_request` JSON payload, load `Skill(cafleet)` § Routing Bash via the Director and follow the 6-step dispatch." This bridges the case where a Director loaded only `Skill(cafleet-monitoring)` at session start and discovers a bash_request later — without the hint, the dispatch flow won't be in context. <!-- completed: -->
- [ ] Update `skills/design-doc-create/roles/director.md`: every literal cafleet invocation moves to nested form; drop codex references if any. <!-- completed: -->
- [ ] Update `skills/design-doc-execute/roles/director.md`: same edit, mirrored. <!-- completed: -->

### Step 11: Documentation — admin SPA

- [ ] Update `admin/src/components/Sidebar.tsx:56`: `"cafleet register"` → `"cafleet agent register"`. <!-- completed: -->
- [ ] Update `admin/src/components/Dashboard.tsx:88`: `"cafleet register"` → `"cafleet agent register"`. <!-- completed: -->

(`Dashboard.tsx:76` "cafleet db init" and `SessionPicker.tsx:53` "cafleet session create" are already nested — verify in spec phase that no edit is needed.)

### Step 12: Code — round-6 nested-only restructure (Click groups)

> **Atomic-landing instruction**: Steps 12 and 14 land atomically (same commit/PR) — Step 12's renames break existing CLI tests until Step 14's invocation rewrites land. Steps 13 and 14 also land atomically — Step 13's CODEX deletion breaks test imports until Step 14's test deletions land. Concretely: package Steps 11, 12, 13, 14 into one PR (Step 11 shares the docs-first sweep over `admin/`). Step 15's quality gates run only at the head of that combined PR, never against intermediate states.

- [ ] Refactor `cafleet/src/cafleet/cli.py` to introduce `@cli.group()` decorators for `agent` and `message`. Move the existing flat-verb handlers under their new groups, applying rename-during-move atomically per the §14 mapping table: `agents` → `agent list`, `register` → `agent register`, `deregister` → `agent deregister`, `send` → `message send`, `broadcast` → `message broadcast`, `poll` → `message poll`, `ack` → `message ack`, `cancel` → `message cancel`, `get-task` → `message show`. Decorator changes + handler renames only; handler bodies unchanged. Hard-break — no Click aliases. <!-- completed: -->
- [ ] Rename the round-5c `cafleet bash-exec` handler to `cafleet member exec` — move the handler from a top-level `@cli.command()` to `@member.command("exec")`. Handler body unchanged. The `Bash(cafleet member exec *)` allow-rule entry follows from this rename. <!-- completed: -->
- [ ] Materialize `cafleet agent show --id <x>` as its own subcommand by extracting the existing `agents --id <x>` branch from the `agent list` handler (renamed in task 1) into a dedicated `agent.command("show")` handler. The `agent list` handler no longer accepts `--id`. <!-- completed: -->
- [ ] Update the `tmux.send_poll_trigger` keystroke literal in `cafleet/src/cafleet/tmux.py` from `cafleet --session-id <s> poll --agent-id <r>` to `cafleet --session-id <s> message poll --agent-id <r>`. <!-- completed: -->
- [ ] Update the `CLAUDE.default_prompt_template` literal in `cafleet/src/cafleet/coding_agent.py` so the bash-routing reminder references `cafleet --session-id {session_id} message poll --agent-id {agent_id}` (was `... poll ...`). <!-- completed: -->

### Step 13: Code — round-6 codex deprecation

> **Atomic-landing instruction**: Steps 13 and 14 land atomically with Step 12 (same PR). Step 13's CODEX deletion breaks test imports until Step 14's test deletions land. See Step 12's atomic-landing note for the full PR-packaging guidance.

- [ ] Delete `CODEX = CodingAgentConfig(...)` from `cafleet/src/cafleet/coding_agent.py`. Delete `CODING_AGENTS` registry and `get_coding_agent()` helper. The module collapses to `CodingAgentConfig` dataclass + a single `CLAUDE` instance. <!-- completed: -->
- [ ] Remove the `--coding-agent` flag from `member_create` in `cafleet/src/cafleet/cli.py`. Remove the round-5c codex rejection branch. Member creation always uses `CLAUDE.build_command(...)` directly. <!-- completed: -->
- [ ] Update the `FIXME(claude)` comment at `cafleet/src/cafleet/broker.py:17` to drop the codex env-var reference. <!-- completed: -->

### Step 14: Tests — round-6 restructure + codex deprecation

> **Atomic-landing instruction**: Step 14 lands atomically with Steps 12 and 13 (same PR). Without this step's invocation rewrites, Step 12's CLI renames break every existing CLI test; without this step's codex deletions, Step 13's CODEX import breakage cascades through test_coding_agent.py.

- [ ] Rewrite every CLI test in `cafleet/tests/test_cli_*.py` to invoke commands in their new nested form (`cafleet agent register`, `cafleet message send`, `cafleet member exec`, etc.). Rename `cafleet/tests/test_cli_bash_exec.py` to `cafleet/tests/test_cli_member_exec.py` and rewrite every `cafleet bash-exec` invocation inside to `cafleet member exec`. Test logic is unchanged; only the CLI invocation strings + file name update. <!-- completed: -->
- [ ] Add `TestFlatVerbsRejected` to `cafleet/tests/test_cli_restructure.py` (new file): assert each old flat-verb invocation fails with Click's default `Error: No such command '<name>'.`. Cases: `cafleet send`, `cafleet poll`, `cafleet ack`, `cafleet cancel`, `cafleet broadcast`, `cafleet register`, `cafleet deregister`, `cafleet agents`, `cafleet get-task`, `cafleet bash-exec`. Regression guard against any future contributor accidentally re-adding a Click alias. <!-- completed: -->
- [ ] Add `TestSendPollTriggerKeystroke` to `cafleet/tests/test_tmux.py`: monkey-patch `tmux._run` to capture argv; call `tmux.send_poll_trigger(target_pane_id="%0", session_id="<uuid>", agent_id="<uuid>")` once and assert the captured keystroke string contains the literal `message poll` (not `poll`). Regression guard against any future revert of Step 12 task 5. <!-- completed: -->
- [ ] Delete every codex-specific test case in `cafleet/tests/test_coding_agent.py` (`TestCodexBuildCommand`, `TestCodexDisplayName`, registry-lookup tests, codex sub-cases of `TestDisallowTools` / `TestPromptTemplates`). Round-5c's `TestPromptTemplates` shrinks to claude-only per §15. <!-- completed: -->
- [ ] Delete the codex sub-cases of `TestNoBashFlag` in `cafleet/tests/test_cli_member.py` (the `--no-bash --coding-agent codex` rejection case and the `codex` default `--allow-bash` case). Round-5c's `TestNoBashFlag` shrinks from 4 sub-cases to 2 per §15. <!-- completed: -->
- [ ] Add `TestCodingAgentFlagRemoved` to `cafleet/tests/test_cli_member.py`: `cafleet member create --coding-agent claude` fails with `Error: No such option: '--coding-agent'.` (Click default) — regression guard. <!-- completed: -->
- [ ] Add `TestCodexConstantRemoved` to `cafleet/tests/test_coding_agent.py`: importing `CODEX`, `CODING_AGENTS`, `get_coding_agent` from `cafleet.coding_agent` each raise `ImportError` — regression guard. <!-- completed: -->
- [ ] Drop codex-specific spawn-command tests in `cafleet/tests/test_tmux.py` if any are codex-specific (most tests are agent-agnostic via `_run` mocking). <!-- completed: -->
- [ ] Drop codex backend strings from `cafleet/tests/test_output.py` format tests. <!-- completed: -->

### Step 15: Quality gates (round 6)

- [ ] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: -->
- [ ] Run `mise //cafleet:lint` — must pass. <!-- completed: -->
- [ ] Run `mise //cafleet:format` — must pass. <!-- completed: -->
- [ ] Run `mise //cafleet:typecheck` — must pass. <!-- completed: -->

### Step 16: Real-world smoke (round 6)

- [ ] Spawn a fresh member via `cafleet member create --no-bash` (default; the `--coding-agent` flag is gone — passing it MUST fail with Click's "no such option" error). As in Step 7, the argv-shape check is covered by `test_cli_member.py::TestNoBashFlag`; this smoke verifies behavior end-to-end. Verify the member's harness rejects a Bash call. Have the member round-trip a `bash_request` through (a) auto-allow, (b) ask-Approve-as-is, (c) ask-Approve-with-edits, (d) ask-Deny-with-reason, (e) auto-allow timeout — same Step 7 case list as round-5c, but every member-side and Director-side cafleet invocation now uses the nested form. Confirm the `tmux send-keys` keystroke fired by the broker after each Director reply lands as `cafleet --session-id <s> message poll --agent-id <r>` in the member's pane and the member resumes correctly. <!-- completed: -->
- [ ] Spawn a fresh member at the head of round 6 with the `--coding-agent codex` flag (a developer migrating from the old code path). Verify it fails with Click's `Error: No such option: '--coding-agent'.`. Run `cafleet member list` and confirm the rendering. If a developer has surviving codex placement rows from before the rename (typical for upgrade-in-place environments), they should still appear in `cafleet member list` with `coding_agent='codex'` and a non-null pane_id (the migration story in §15 is verified). <!-- completed: -->

### Step 17: Finalize (round 6)

- [ ] Update Status to Complete and refresh Last Updated. <!-- completed: -->
- [ ] Add a Changelog entry. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-28 | Initial draft. User answers (Q1 CAFleet members only; Q2 `--disallowedTools` flag with codex limitation; Q4 reuse `cafleet message send` with JSON payload; Q8a auto-run + audit note; Q8b/Q12 Approve / Edit / Deny three-option AskUserQuestion; Q9 no member-side timeout; Q13 plain-text WebUI; Q14 unit + integration + real-world smoke) baked into the spec. |
| 2026-04-28 | Reviewer pass 1: dropped Step 3 (no `~/.claude/skills/cafleet*` copies exist to mirror to); renumbered remaining steps. Adopted Option B for §3 step 5 — added new `cafleet member exec` helper subcommand owning 64 KiB / SIGKILL semantics; documented `Bash(cafleet member exec *)` allow-rule trade-off explicitly. Rewrote Success Criteria line 20 to match §5 (documentation-only cross-Director rule, no CLI guard). Pinned the `[binary, *extra_args, *deny_args, *name_args, prompt]` argv ordering in §1 and Step 3 task 2. Resolved `type` discriminator: parse failures and non-`bash_request` shapes fall through to plain-instruction handling; only `cmd == ""` and `timeout > 600` emit denied bash_results, with `exit_code: 126` everywhere (no `exit_code: 2`). Added §4 allow×deny truth table. Made Director the explicit actor for the reply `cafleet message send` (§3 step 6). Added explicit `task_id` correlation explainer in §2. Provided exact spawn-prompt amendments for both CLAUDE and CODEX in Step 3 (codex's reminder inlines the JSON envelope since it has no skills). Added §3 "Double-prompt and the bash-exec allow rule" subsection. Reconciled §8 timeout-cap wording with §4. Documented `bash_routing.py` exposing both matcher and payload helpers in Step 4 task 4. Added §11 cross-reference to Step 7's smoke checklist. Verified §12 session-delete cascade row against `broker.delete_session` semantics. Recounted tasks: 27 (Step 1: 3, Step 2: 4, Step 3: 4, Step 4: 4, Step 5: 5, Step 6: 4, Step 7: 1, Step 8: 2). |
| 2026-04-28 | Reviewer pass 3: restructured §3 into 7 distinct steps separating discriminator (step 1) / field validation (step 2) / matcher-run-reply (steps 3–7). Dropped the `auto-deny` outcome from §3 and replaced it with explicit "matcher returns `auto-run` or `ask`" (matching the `Literal["auto-run", "ask"]` signature in Step 4 task 4). Renumbered cross-references: AskUserQuestion shape moved from "(step 4)" to "(step 5)"; bash_result payload moved from "(step 6)" to "(step 7)"; §2 task_id reference updated to "§3 step 7". Future-proof rule above the §3 table now explicitly says it covers ONLY the discriminator phase, NOT field-validation failures. Removed the `cwd does not exist` row from §4 reject thresholds (it is a runtime path, not a reject threshold) and replaced it with a one-line note pointing at §12; "two reject thresholds" sentence now matches the row count. §3 step 6 invocation rewritten with proper optional-flag bracket syntax (`[--cwd '<cwd>'] [--timeout <timeout>] [--stdin '<stdin>']`) and added the "each flag included only when non-null; otherwise helper defaults" sentence. §2 example invocation gains `cafleet --session-id <s> --json send ...` and the `--json` placement disclaimer (`--json` is global, placed before subcommand). Added §11 row `test_coding_agent.py::TestPromptTemplates` (canary substrings + `str.format()` smoke for both CLAUDE and CODEX templates). §11 "Real-world smoke" row collapsed to "(See Implementation Step 7 for the canonical case list)". Step 7 case (e) rebound to the auto-allow path (`Bash(sleep *)` in `permissions.allow`) so the timeout test purely exercises the helper's SIGKILL — AskUserQuestion is bypassed. Step 7's mechanically-wrong "Confirm spawn argv via `cafleet member capture`" instruction removed; behavioral check (member attempts Bash, harness rejects) is the end-to-end proof, with argv shape covered by `test_cli_member.py::TestNoBashFlag`. Step 4 task 4 `format_bash_result` signature pinned with full keyword list and "pure formatter — does NOT truncate" docstring; `format_bash_result` and the other three helpers now have full pinned signatures inside a code block. Task count unchanged at 27. |
| 2026-04-28 | Round 5 (user-requested unifications + 2 round-4 nits): **Unification 1** — collapsed Director dispatch from 4 phases (discriminate / validate / match / run+reply) to 3 phases (discriminate / match / run+reply). §3 dispatch table is now 6 steps; field validation lives entirely inside `cafleet member exec`. §4 stripped of its "Reject thresholds" subsection (and the cwd note); §4 is now exclusively the allow×deny matcher. `parse_bash_request` no longer raises on field-semantic issues; the helper handles them. `test_bash_routing_matcher.py` has no reject-threshold rows; `test_cli_bash_exec.py` gains `(e)` and `(f)` denied-JSON cases plus `(g)` cwd-runtime case. **Unification 2** — single consent surface. Deleted §3 "Double-prompt and the bash-exec allow rule" subsection. §3 step 5 explicitly notes the Director's Bash-tool prompt does NOT fire because `Bash(cafleet member exec *)` is required in the operator's `permissions.allow`. §6 retitled "Director-side requirements and codex member fallback" with a leading "Required `permissions.allow` entry" subsection. §10 expanded: ARCHITECTURE.md / cli-options.md / SKILL.md / cafleet-monitoring SKILL.md all gain the operator-setup item; cli-options.md gains a `### permissions.allow setup` subsection; cafleet-monitoring gains a "Director setup: required `permissions.allow` entries" subsection. §1 SC: replaced the double-prompt line with the single-consent-gate line. §13 Future Work gains "Operator-setup verifier" bullet. §11 gains `TestSingleConsentGate` (smoke-only). **Unification 3** — `status` is canonical, `exit_code` opaque except on `ran`. §3 / bash_result table re-documents the contract with the prominent canonical-status rule. §8 SIGKILL note updated. §11 test assertions switch on `status`; `exit_code` is asserted only for `status == "ran"`. `format_bash_result` docstring notes the opaque-exit_code behavior. §1 SC gains a dedicated canonical-status line. **Nit A**: §12 cwd-row stale "documented in §4 reject thresholds" rewritten to point at §3 helper subsection. **Nit B**: §3 step 4 (was step 5) `run-as-is` / `run-edited` / `deny` branches now have explicit `note`-setting instructions inline. Task count unchanged at 27 (Step 5 task 1 expanded to cover both `TestDisallowTools` and `TestPromptTemplates` in the same file). Success Criteria checkbox count grew from 12 to 14 (added single-consent-gate + canonical-status lines). Total `- [ ]` markers: 41. |
| 2026-04-28 | Round 5b (stale cross-reference cleanup after round 5): five 1–2 line fixes flagged by the Reviewer's structural pass. (1) §2 task_id paragraph "(§3 step 7)" → "(§3 step 6)" — round 5 collapsed §3 to 6 steps. (2) §2 schema-table `timeout` row stale `(§4 "Reject thresholds")` → `(§3 helper subsection bullet 1; ...)` — §4's reject-thresholds subsection was deleted in round 5 and rejection moved into the helper. (3) Step 7 case (d) `bash_result` assertion `status: "denied", exit_code: 126` → drop `exit_code` per the canonical-status rule; add the `note: "operator denied: <reason>"` element. (4) Step 7 case (e) `status: "timeout", exit_code: 124` → drop `exit_code` per the canonical-status rule. (5) Step 7's last paragraph rewritten — the "double-prompt" cross-reference to a deleted subsection is gone, and the (a)–(e) framing now declares them as the **required-setup happy path** (§6: `Bash(cafleet member exec *)` MUST be in `permissions.allow`). The degraded-UX verification step (temporarily remove the rule, re-run case (b), confirm BOTH prompts fire) is repositioned as the §12 "operator forgets" edge-case validation. No structural changes; task count, SC count, and total checkbox count (41) all unchanged. |
| 2026-04-28 | Round 6 (scope expansion: nested-only restructure + codex deprecation, both folded into 0000034). User decisions ratchetted up: nested-only (FQ1, no flat verbs), hard-break (FQ3, no aliases), codex dropped entirely (new scope). **Title** broadened to "CAFleet CLI consolidation: Bash-via-Director, nested-only restructure, codex deprecation." **Overview** rewritten to cover the three features. **Success Criteria** reorganized into four subsections (Bash-via-Director / Nested-only restructure / Codex deprecation / Cross-cutting) growing from 14 lines to 21; existing bash-routing criteria preserved verbatim with the `bash-exec` → `member exec` rename, the codex-rejection criterion deleted (now unreachable), and seven new criteria added for the restructure + codex-drop. **§1** scrubbed of codex semantics: the coding-agent table collapses to claude-only (`--no-bash` default + `--allow-bash` opt-out); `CODEX.disallow_tools_args = ()` line removed; "For Codex this combination is unreachable" caveat deleted. **§2 Background "feasibility" subsection** rewritten to explain codex's lack of `--disallowedTools` was the original justification for the round-5c fallback that round-6 now eliminates by removing codex entirely. **§3 dispatch table** ripple-renamed `cafleet bash-exec` → `cafleet member exec` everywhere (locked semantics preserved); cross-references like `cafleet poll` → `cafleet message poll` and `cafleet send` → `cafleet message send` throughout the live content (changelog history rows preserved verbatim). **§6 retitled** "Director-side requirements" (was "Director-side requirements and codex member fallback"); the entire "Codex member fallback" subsection deleted; new "Member-backend support" subsection points at §15. **§10 Documentation surface** updated: ARCHITECTURE.md row now drops "Multi-runner support" + "Pane display-name propagation" codex copy; cli-options.md row drops the `--coding-agent` flag and points at §15; SKILL.md row drops codex examples and renames `### Bash Exec` → `### Member Exec` per the rename ripple. **§11 Tests**: `TestNoBashFlag` shrinks from 4 sub-cases to 2; `TestPromptTemplates` becomes claude-only; `test_cli_bash_exec.py` → `test_cli_member_exec.py`; matcher-test allow patterns updated to `cafleet message poll`. **§13 Future Work**: codex restoration bullet rewritten with full restoration plan (i)–(vi); round-7 subscribe-primitive bullet added (deferred). **§14 added** (Nested-only subcommand restructure) — full mapping table (every old subcommand → new home), Click implementation sketch, exhaustive hard-coded-literal inventory across source code / templates / SKILL.md / docs / admin SPA, hard-break compatibility note. **§15 added** (Codex deprecation) — rationale, migration story (CLI rejection only, no broker gate, no auto-cleanup, data preserved), code-surface and doc-surface tables, two regression-guard test rows. **Implementation steps**: round 5c's Step 8 keeps its "Finalize (round 5c)" framing; nine new steps (9–17) added for round 6: docs / project skills / admin SPA / Click groups / codex deletion / tests / quality gates / smoke / finalize. **Header** progress recounted from 0/27 to 0/59. Round 5c implementation shrank from 27 to 25 tasks because round 6 deleted two now-unreachable codex-aware sub-tasks (Step 3 task "Update `CODEX.default_prompt_template`" and Step 4 task "reject `--no-bash --coding-agent codex`") rather than carrying them forward as no-ops. Round 6 added 34 new implementation tasks across Steps 9–17 (Step 9: 5 docs, Step 10: 4 skills, Step 11: 2 admin SPA, Step 12: 6 Click-group refactor, Step 13: 3 codex deletion, Step 14: 6 tests, Step 15: 4 quality gates, Step 16: 2 smoke, Step 17: 2 finalize). Implementation total: 25 + 34 = **59**. Success Criteria grew from 14 to 21 (added 4 restructure + 4 codex + cross-cutting reorg, net +7). Total `- [ ]` markers in the doc: **80** (21 SC + 59 implementation). |
| 2026-04-28 | Round 6b (Reviewer fixes — 1 blocking + 4 polish): (1) **Blocking** §2 example invocation `cafleet --session-id <s> --json send ...` → `cafleet --session-id <s> --json message send ...` (the surrounding prose at lines 158 and 166 was already nested but the code block in between was stale flat-form; would have failed after round 6 hard-break). Sweep verified — no remaining stale flat invocations outside §14 mapping table / hard-break compat note / admin-SPA replacement rows / Step 12 rename descriptions / changelog history (all intentional historical references). (2) Round-6 changelog row internal "14 lines to 18" textual slip → "14 lines to 21" (matches the actual SC count and the row's later "14 to 21" phrasing). (3) Step 4 task 2 wording "top-level `cafleet member exec`" was contradictory (`member exec` is by definition under the `member` group). Reverted to the round-by-round narrative: Step 4 task 2 now adds `cafleet bash-exec` at top-level (round-5c-era name and placement); Step 12 task 2 does the rename + group move (round-6 work). This matches the changelog framing of "round-5c → round-6 rename ripple" and avoids re-sequencing the implementation steps. (4) Step 16 task 1 wording "Verify the spawn argv... (smoke covers behavior)" was self-contradictory. Rewrote to mirror Step 7's framing: "As in Step 7, the argv-shape check is covered by `test_cli_member.py::TestNoBashFlag`; this smoke verifies behavior end-to-end." (5) Step 16 task 2 conditional framing "if there are surviving codex placement rows..." rewritten to "If a developer has surviving codex placement rows from before the rename (typical for upgrade-in-place environments), they should still appear..." — clarifies that the conditional describes the upgrade-in-place scenario, not a skip-if-not-applicable. No structural changes; task count, SC count, and total checkbox count (80) all unchanged. |
| 2026-04-28 | Round 6d (Reviewer's deeper round-6 pass — 3 blocking + 5 polish, structural untangling): **(Fix 1)** Step 3 task 3 reverted from nested `cafleet message poll` to flat `cafleet poll` in the round-5c-era CLAUDE template literal; Step 12 task 6 (round 6) owns the rename. **(Fix 2)** Step 7 line 749 reverted from `cafleet message poll` to `cafleet poll` for the round-5c smoke (the keystroke is still flat-form at that point in the implementation order). Added a one-line preamble at Step 7 explaining it is the round-5c smoke; Step 16 is the round-6 end-state smoke. **(Fix 3)** Major structural untangling: Steps 1 / 2 / 3 / 5 / 7 reverted to round-5c-only content. Specific reversions: Step 1 task 1 (ARCHITECTURE.md) drops "drop the existing Multi-runner support paragraph and codex-aware copy" language; Step 9 task 1 owns it. Step 1 task 2 (cli-options.md) drops "remove the `--coding-agent` flag entry entirely"; Step 9 task 3 owns it. Step 1 task 3 (README.md) reverted to `cafleet bash-exec` round-5c-era name. Step 2 task 1 (cafleet skill) drops codex-deletion + bash-exec-rename language; Step 10 task 1 owns both. Step 3 task 1 says `CODEX.disallow_tools_args = ()` is set (round-5c-era state); Step 13 task 1 deletes CODEX in round 6. Step 3 task 4 re-added (CODEX template) so Step 13 task 1 has something to delete. Step 4 task 1 / 2 reverted to round-5c-era flag-resolution code path with codex rejection branch; Step 13 task 2 removes both in round 6. Step 5 task 1 / 2 / 5 reverted to round-5c-era test descriptions (4-sub-case TestNoBashFlag, codex assertions present, `test_cli_bash_exec.py` filename); Step 14 owns all the round-6 prunes / renames / regression guards. Step 7 line 732 keystroke reverted to flat `cafleet poll`; Step 12 task 5 renames it. **(Fix 4)** Step 12 task 1 made explicit: rename-during-move is atomic; full mapping list (`agents` → `agent list`, `register` → `agent register`, etc.) inlined in the task body. Step 12's redundant standalone get-task rename task removed (folded into task 1). Step 12 now has 5 tasks (was 6). **(Fix 5)** §11 added `TestFlatVerbsRejected` row + Step 14 task to assert each old flat-verb invocation fails with Click's "no such command" error. Regression guard against future Click-alias re-introduction silently breaking the hard-break SC. **(Fix 6)** §11 added `TestSendPollTriggerKeystroke` row + Step 14 task to monkey-patch `tmux._run` and assert the captured keystroke contains `message poll`. Regression guard against any future revert of Step 12 task 5. **(Fix 7)** §13 codex restoration plan extended with (vii) re-add `CODEX.default_prompt_template`, (viii) revert the round-6 `FIXME(claude)` comment edit at `broker.py:17`, (ix) remove the round-6 regression-guard tests (`TestCodingAgentFlagRemoved`, `TestCodexConstantRemoved`) — they will start failing the moment codex is restored; replace with positive tests. **(Fix 8)** Atomic-landing instructions added at the top of Steps 12 and 13: "Steps 12, 13, 14 land atomically (same PR); Step 11 shares the docs-first sweep so package it too. Step 15's quality gates run only at the head of the combined PR." **(Bonus polish)** §15 retirement guidance gained one sentence on `cafleet member delete --force` for codex panes that block on `/exit` (the `--force` flag was introduced in design 0000032 and works identically for codex). **Counts**: Step 3 grew from 3 to 4 tasks (re-added CODEX template); Step 4 grew from 3 to 4 tasks (re-added codex rejection branch); Step 12 shrank from 6 to 5 tasks (atomic rename-during-move folds get-task task into task 1); Step 14 grew from 6 to 9 tasks (added 3 tasks: TestFlatVerbsRejected, TestSendPollTriggerKeystroke, codex sub-case prune). Net round-5c implementation: 25 → 27 (back to original); net round-6 implementation: 34 → 36. Implementation total: 27 + 36 = **63**. SC unchanged at 21. Total `- [ ]` markers: **84**. Header `**Progress**: 0/59` → `0/63`. |
| 2026-04-28 | Round 6f (Reviewer polish — 7 clarity items, no structural changes): **(A)** §7 line 328 rewritten from "Block on `cafleet message poll`..." to "Wait for the broker's tmux push notification, which injects a `cafleet message poll` keystroke into the member's pane when the bash_result arrives. The member polls (one-shot, not a loop), filters by `in_reply_to == task_id`, and resumes." Removes the misleading "block on poll" wording that read as a polling loop. **(B)** Step 10 task 2 (cafleet-monitoring SKILL.md) gained an explicit instruction near Stall Response: "When the Director's poll output contains a `bash_request` JSON payload, load `Skill(cafleet)` § Routing Bash via the Director and follow the 6-step dispatch." This bridges the case where a Director loaded only `Skill(cafleet-monitoring)` at session start and discovers a bash_request later. **(C)** §12 gained 3 new edge-case rows: operator closes Director's pane mid-bash-request (member resumes via persisted task on next poll); broker server restarts mid-bash-request (tmux push lost, task persists, `/loop` recovers on next cycle); concurrent bash_requests from multiple members (sequential AskUserQuestion turns). **(D)** §7 gained a paragraph after the new Fix-A wait/poll/resume sentence clarifying member-side concurrency: "If `cafleet message poll` returns messages in addition to the bash_result (or before it arrives), the member processes those messages as plain instructions per existing CAFleet semantics; the bash_request continuation only resumes when a `bash_result` with the matching `in_reply_to` arrives. The member's task is not gated on bash_result arrival — it's gated on the bash_result correlation." **(E)** §15 gained a new "Rollback" subsection: revert the round-6 atomic PR; DB schema is unchanged so rollback is safe; wedged claude panes that saw post-round-6 keystrokes get `member delete --force`-ed and respawned. Distinguished rollback ("round 6 was a mistake, revert all of it") from restoration ("codex is back in v2 — write a new design doc"). **(F)** §6 "Required `permissions.allow` entry" gained one sentence on operator upgrade ordering: "Order is irrelevant — adding the entry before or after the binary upgrade both work. Intermediate states... are degraded-UX (extra Bash prompts) but functional." **(G)** §6 gained a new "Operator upgrade checklist" subsection: "After upgrading the cafleet binary, **restart any Director session that was running pre-upgrade**." Verified the actual reload mechanism — Claude Code does NOT expose an in-session skill-reload command; per `claude --help` the only mechanism is exiting and restarting the session so the harness re-reads `skills/<name>/SKILL.md` from disk. The Reviewer's hypothetical `/skill cafleet` command is not real; the doc states the honest mechanism. **Counts**: no new tasks (all fixes are edits to existing prose / table rows / step-task descriptions). Task count unchanged at 63 implementation + 21 SC = **84** total `- [ ]` markers. Header `**Progress**: 0/63 tasks complete` unchanged. |
| 2026-04-28 | **User-approved.** Status moved from Draft to Approved. Last Updated 2026-04-28. Implementation-only `- [ ]` count verified at 63 by counting checkboxes between `## Implementation` (line 657) and `## Changelog` (line 850); matches header `**Progress**: 0/63 tasks complete`. Implementation steps spot-verified for actionability across the round-5c (Steps 1–8) and round-6 (Steps 9–17) spans: every task carries concrete file paths (e.g. `cafleet/src/cafleet/coding_agent.py`, `cafleet/tests/test_cli_member.py`, `ARCHITECTURE.md`, `skills/cafleet/SKILL.md`), function or symbol names (e.g. `CodingAgentConfig`, `CLAUDE.disallow_tools_args`, `build_command`, `tmux.send_poll_trigger`, `TestNoBashFlag`), and specific assertions (e.g. "argv contains `--disallowedTools Bash`", "fails with Click's `Error: No such option: '--coding-agent'.`", "captured keystroke contains the literal `message poll`"). Step ownership for each rename / deletion is cross-referenced explicitly (e.g. "Codex-related copy deletions are owned by Step 9 task 1") so no doc surface is edited twice. Doc is ready for `/design-doc-execute` to begin implementation. |
