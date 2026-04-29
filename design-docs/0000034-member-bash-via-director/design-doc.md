# CAFleet CLI consolidation: Bash-via-Director, nested-only restructure, codex deprecation

**Status**: Complete
**Progress**: 58/63 tasks complete
**Last Updated**: 2026-04-29

## Overview

Three coupled CLI changes shipped as one design. (1) **Bash-via-Director**: members spawn with `--no-bash` (the spawn argv gains `--disallowedTools "Bash"`) so the harness rejects every Bash call. When a member needs a shell command, it sends a plain CAFleet message asking its Director, and the Director responds by sending `! <command>` keystrokes into the member's pane via the existing `cafleet member send-input`. Claude Code's `!` CLI shortcut handles execution natively — no JSON envelope, no schema, no separate helper subcommand, no allow-list matcher. Concurrent member requests serialize through the broker's existing message queue (the Director processes each `cafleet message poll`-returned request one at a time before moving to the next; the current poll order is newest-first, not FIFO). (2) **Nested-only subcommand restructure** (round 6): every existing flat verb (`send`, `broadcast`, `poll`, `ack`, `cancel`, `register`, `deregister`, `agents`, `get-task`) moves under a noun group (`agent`, `message`, `member`, `session`, `db`); only `server` and `doctor` stay top-level as meta-commands. Hard-break, no aliases. (3) **Codex deprecation** (round 6): `cafleet member create --coding-agent codex` and the `CODEX` config are removed; claude is the only supported member backend. In-flight codex panes keep running, but no new codex registrations. Restoration is documented in §13 Future Work.

## Success Criteria

### Bash-via-Director (round 7 — `!`-keystroke convention)

- [x] `cafleet member create --no-bash` (default for claude) appends `--disallowedTools "Bash"` to the spawned `claude` process; the resulting member pane cannot use the Bash tool. Verified by `test_cli_member.py::TestNoBashFlag` (argv-shape pinning) and `test_coding_agent.py::TestDisallowTools`.
- [x] When a member needs a shell command, it sends a plain CAFleet message via `cafleet message send` to its Director. No JSON envelope, no schema, no special structure — free-text request like `"Please run \`git log -1\` for me — verifying the latest commit on main."`.
- [x] The Director receives the request via the existing `cafleet message poll` push-notification path and responds by sending `! <command>` keystrokes into the member's pane via `cafleet member send-input --member-id <m> --freetext "! <command>"`. Claude Code's `!` CLI shortcut handles execution natively — output appears in the member's pane prompt context for the model to read. No new broker primitives, no separate helper subcommand, no allow-list matcher, no AskUserQuestion gate.
- [x] Concurrent member requests serialize through the existing `cafleet message poll` return order. The Director processes one request at a time — read a poll-returned request, dispatch the `! <command>` keystroke, then move to the next message in the order returned by `cafleet message poll` (currently newest-first). No new queueing primitive.
- [x] Cross-Director boundary: members address requests only to their own `placement.director_agent_id`. Cross-session leakage is prevented by the broker's existing session boundary; the within-session "address only your own Director" rule is documentation-only.
- [x] No member-side timeout. Members block on the Director's reply; the operator manually nudges or cancels (`cafleet message cancel`) if a Director wedges. Consistent with every other CAFleet message-passing path.

### Nested-only restructure (round 6)

- [x] Every flat-verb subcommand (`register`, `deregister`, `agents`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`) is moved under a noun group (`agent`, `message`). Only `server` and `doctor` stay top-level as meta-command exceptions.
- [x] Hard-break: the old subcommand strings stop existing the moment the rename merges. No Click aliases. Every literal occurrence in source code, prompt templates, tmux keystroke injection, SKILL.md files, README, ARCHITECTURE, docs/spec, admin SPA, and this design document is updated in the same documentation-first sweep.
- [x] The `cafleet` binary itself is unchanged; only subcommands restructure. `Bash(cafleet *)` and `mise //cafleet:*` remain valid.

### Codex deprecation (round 6)

- [x] `cafleet member create` no longer accepts a `--coding-agent` flag. Claude is the only supported backend. Anyone running with the flag gets `Error: No such option: '--coding-agent'.` (Click default).
- [x] `CodingAgentConfig.CODEX` and the `CODING_AGENTS` registry's codex entry are removed. The `cafleet/src/cafleet/coding_agent.py` module collapses to a `CLAUDE` constant; the `get_coding_agent()` helper / `CODING_AGENTS` dict are removed since there is exactly one config.
- [x] In-flight codex member panes (any `agent_placements` row with `coding_agent='codex'` predating the rename) keep running their existing process; the broker does not kill or auto-cleanup. Operators retire them with `cafleet member delete` per pane. The `agent_placements.coding_agent` column stays `TEXT NOT NULL DEFAULT 'claude'` — codex rows are preserved for forensic visibility.
- [x] Every codex-aware doc / SKILL / test path is removed in the same documentation-first sweep. §13 Future Work captures the restoration plan for when codex grows the equivalent enforcement primitives.

### Cross-cutting (round 6)

- [x] Documentation is updated **before** code per `.claude/rules/design-doc-numbering.md`: `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `docs/spec/data-model.md`, `README.md`, `cafleet/CLAUDE.md`, `skills/cafleet/SKILL.md`, `skills/cafleet-monitoring/SKILL.md`, both Director role files in `skills/design-doc-create/roles/director.md` and `skills/design-doc-execute/roles/director.md`, and the four admin SPA strings (Sidebar.tsx, Dashboard.tsx ×2, SessionPicker.tsx). Skill drift is a blocker.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` all pass.

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
| Member harnesses each carry their own `permissions.allow` (drifts) | The Director's pane is the single decision point — no per-member `permissions.allow` configuration needed |
| Operator can't audit what commands ran | Every shell-command request is a regular CAFleet message, persisted in SQLite and visible in the admin WebUI timeline; the Director's `! <command>` dispatch is captured in the member's pane via `cafleet member capture` |
| Member crashes can leak Bash side-effects (deleted files, force-pushed branches) the operator never saw | Member loses Bash entirely; only the Director's deliberate `! <command>` dispatches reach the member's pane |

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

The member uses **existing `cafleet message send`** with a free-text request in `--text`. No JSON envelope, no schema, no special structure. Example:

```bash
cafleet --session-id <session-id> message send --agent-id <member-id> \
  --to <director-agent-id> \
  --text "Please run \`git log -1 --oneline\` for me — verifying the latest commit on main before opening a PR."
```

The member always addresses the request to its own `placement.director_agent_id`. Cross-session leakage is prevented by the broker's existing session boundary; the within-session "address only your own Director" rule is documentation-only.

The member then waits. There are two ways the response can arrive:

- The Director sends a `! <command>` keystroke into the member's pane (§3); Claude Code's `!` CLI shortcut runs the command natively and prints the captured output back into the same pane prompt context for the model's next iteration to read.
- The Director sends a follow-up CAFleet message (e.g. "I won't run that — it would touch `main`. Try a different approach"). The member processes it as a regular instruction.

### 3. Director-side: receive and dispatch

The Director's normal `/loop` health-check (`Skill(cafleet-monitoring)` Stage 1, `cafleet message poll`) already surfaces incoming messages. When a polled message is a member shell-command request, the Director (operator at the Director's pane) follows a 3-step protocol:

| Step | Director-pane action |
|---|---|
| 1 | **Read the request** via `cafleet message poll`. The member's text is plain language ("Please run `git log -1`"). The Director's Claude Code pane interprets the request and decides whether to fulfill it; the operator stays in control via Claude Code's normal pane-side interaction. |
| 2 | **Decide**. The Director either fulfills the request (continue to step 3), declines via a follow-up CAFleet message ("I won't run that, here's why…"), or asks for clarification (also via `cafleet message send`). If the request asks for something destructive or out of scope, the Director chooses the message-reply path; the `! <command>` keystroke is reserved for the fulfilled case. |
| 3 | **Dispatch the command** via `cafleet member send-input --member-id <m> --freetext "! <command>"`. The trailing newline (Enter) is appended automatically by `member send-input --freetext`. Claude Code's `!` CLI shortcut intercepts the line, runs the command via the harness's native primitive (bypassing the Bash tool permission system that `--disallowedTools "Bash"` denies), and prints the captured stdout/stderr back into the member's pane. The member's next prompt iteration sees the output as context. |

#### Why this works

- **Member's Bash tool is denied** (`--disallowedTools "Bash"`), so the member cannot execute shell commands itself.
- **Claude Code's `!` shortcut is a separate primitive** from the Bash tool — `claude --disallowedTools "Bash"` does NOT disable the `!` CLI shortcut. The Director triggers it via `tmux send-keys`, which lands as keystrokes in the member's input prompt.
- **Operator stays in control** at the Director's pane: every member shell-request surfaces as a plain message in the Director's inbox, and the Director (with the operator at the keyboard) chooses whether to fulfill it.
- **No new broker primitives**, no JSON envelopes, no separate helper subcommand, no allow-list matcher, no AskUserQuestion gate. Just the existing message-passing + tmux-keystroke infrastructure.

#### Serialization

Concurrent member requests serialize through the broker queue. The Director MUST process command-request messages one at a time in the order returned by `cafleet message poll` — read a request, send `! <command>` via `cafleet member send-input`, then move to the next message. Don't interleave or batch. The current poll order is newest-first; that returned order is the serialization mechanism, no separate queueing primitive is needed.

### 5. Cross-Director boundary

The bash-routing flow uses `cafleet message send` under the hood, so the existing **cross-session boundary** already prevents cross-session leakage at the broker level. ONE extra rule applies *within* a session:

- The member's request MUST be addressed to its own `placement.director_agent_id`. Sending to any other agent in the same session is a misuse — the recipient does not know the bash-routing convention and will treat it as a generic message.

There is no CLI-level enforcement of this rule because `cafleet message send` is a generic primitive. Enforcement is purely documentation: `skills/cafleet/SKILL.md` documents the rule under "Routing Bash via the Director", and the member's spawn prompt is amended to remind it.

### 6. Director-side requirements

#### `permissions.allow` entry

The operator should add `Bash(cafleet member send-input *)` to the Director's `permissions.allow` (project `.claude/settings.json` or user `~/.claude/settings.json`) so the Director's harness can dispatch `! <command>` keystrokes without a per-call permission prompt. Without this entry, the Director's pane fires Claude Code's native Bash-tool prompt for every `cafleet member send-input` invocation; the bash-routing flow still works but with degraded UX (one extra prompt per dispatch). This is preexisting CAFleet UX (`cafleet member send-input` predates this design — designs 0000027 / 0000033) — not a new requirement introduced by bash-routing.

#### Operator upgrade checklist

After upgrading the cafleet binary, **restart any Director session that was running pre-upgrade**. Claude Code caches skill text per-session at session start; the cached `Skill(cafleet)` will reference pre-upgrade-era commands until the session reloads from disk. Members spawned by a pre-upgrade Director session would receive stale spawn-prompt instructions that point at command names the new binary no longer accepts. There is no in-session skill-reload command — the only supported reload mechanism is exiting and restarting the Claude Code session so the harness re-reads `skills/<name>/SKILL.md` from disk.

#### Member-backend support

Claude is the only supported coding-agent backend in this design. The codex backend was deprecated as part of round 6 — see §15 for the deprecation, the migration story, and §13 Future Work for the restoration plan.

`cafleet member create` always spawns claude. There is no `--coding-agent` flag.

### 7. Member-side blocking semantics (no timeout)

Members do NOT have a timeout on shell-command replies. The member's loop is:

1. Send the request via `cafleet message send` (plain free-text).
2. Wait. Either Claude Code's `!` shortcut runs the dispatched command and the captured output appears in the member's own pane (the most common path), or a follow-up CAFleet message arrives via the broker's tmux push notification (which injects a `cafleet message poll` keystroke into the member's pane).
3. Resume.

If `cafleet message poll` returns messages in addition to (or instead of) the dispatched `! command`, the member processes those messages as plain instructions per existing CAFleet semantics. The member is not strictly gated on a specific reply — it's free to act on whatever next signal it receives.

If the Director wedges (crashed pane, busy mid-tool-call, or the operator stepped away from the keyboard), the member sits idle. Recovery is operator-driven via existing CAFleet primitives:

- `cafleet member capture` to inspect the wedged Director's pane.
- `cafleet message send` to nudge the Director directly (the original request remains in the queue).
- `cafleet message cancel` to retract the original request if the operator wants the member to give up.

This is consistent with every other CAFleet message-passing path — no built-in timeouts. Documented explicitly in §13 (Future Work) so reviewers know it is a deliberate v1 choice.

### 9. Out of scope

Explicit non-goals so reviewers do not request these:

- No broker schema changes (no new `tasks` columns, no new tables).
- No changes to the `tmux` push-notification path. The Director's reply (whether a `! <command>` keystroke via `member send-input` or a follow-up CAFleet message) reuses the existing infrastructure.
- No new helper subcommand. The `!` keystroke is dispatched via the existing `cafleet member send-input --freetext` (designs 0000027 / 0000033). No JSON envelope, no allow-list matcher, no AskUserQuestion gate, no `cafleet member exec`.
- No changes to the Agent Teams primitive (TeamCreate / SendMessage / `Drafter`/`Reviewer`). This design covers CAFleet members only.
- No admin WebUI affordance. The shell-command request and any reply messages render as ordinary plain-text rows in the timeline.
- No member-side timeout flag.
- No structured Director-side throttling.
- No global skill-copy parity sweep. The cafleet, cafleet-monitoring, design-doc-create, and design-doc-execute skills exist only in the project tree (`skills/<name>/...`); there are no `~/.claude/skills/cafleet*` copies to keep in sync.

### 10. Documentation surface

Per `.claude/rules/design-doc-numbering.md`, every doc surface that mentions the affected commands MUST be updated **before** any code change.

| File | Change |
|---|---|
| `ARCHITECTURE.md` | Add a new short "Bash Routing via Director" subsection after "Member Lifecycle" describing the `--no-bash` flag on `cafleet member create` plus the `!`-keystroke convention (member sends a plain message → Director sends `! <command>` via `cafleet member send-input` → Claude Code's `!` shortcut runs it natively in the member's pane). Append the serialization note (concurrent requests serialize through the existing `cafleet message poll` return order, currently newest-first). Drop the existing "Multi-runner support" paragraph and the "Pane display-name propagation" paragraph's codex examples — claude is the only supported backend per §15. The `coding_agent.py` Component Layout row collapses to "claude-only spawn config." |
| `docs/spec/cli-options.md` | Under the `cafleet member create` subsection, add `--no-bash` AND `--allow-bash` flag entries with a link to `skills/cafleet/SKILL.md` § Routing Bash via the Director. The `--coding-agent` flag entry is removed entirely (codex deprecation §15). No new subcommand subsection. |
| `README.md` | In the Features list, add a one-line "Bash Routing via Director" bullet covering the convention. In the member-create row of the Agent Commands table, mention `--no-bash` (default) / `--allow-bash`. No new top-level CLI row. |
| `skills/cafleet/SKILL.md` | (a) Extend the `### Member Create` subsection with `--no-bash` / `--allow-bash` flag rows; remove the existing `--coding-agent` flag row and every codex example invocation (codex deprecation §15). (b) Add a new top-level section `## Routing Bash via the Director` after `### Answer a member's AskUserQuestion prompt`. Include the convention (member sends plain message → Director responds with `cafleet member send-input --freetext "! <command>"`), a "Why this works" rationale, the cross-Director boundary rule, and the serialization note. |
| `skills/cafleet-monitoring/SKILL.md` | Add a row to the Stall Response escalation table: when `cafleet message poll` shows an unresponded shell-command request from a member, the Director MUST respond promptly via `cafleet member send-input --freetext "! <command>"`. The member is blocked until the keystroke lands. Reference the new cafleet-skill section. |
| `skills/design-doc-create/roles/director.md` | One-paragraph note pointing at the cafleet skill for the bash-routing workflow: members spawn with `--no-bash`; Drafter / Reviewer requests are plain CAFleet messages; the Director responds with `cafleet member send-input --freetext "! <command>"`. |
| `skills/design-doc-execute/roles/director.md` | Same paragraph, mirrored. |
| `admin/src/components/Sidebar.tsx`, `admin/src/components/Dashboard.tsx` | Update CLI hint strings to nested form (`cafleet agent register` etc.) per the round-6 restructure. |

(No global-skill-copy mirror step. The four skills above exist only in the project tree per §9 "Out of scope".)

### 11. Tests

| Test | Coverage |
|---|---|
| `test_coding_agent.py::TestDisallowTools` | `CLAUDE.build_command(prompt, deny_bash=True)` argv equals `["claude", "--disallowedTools", "Bash", prompt]` (no display_name) or `["claude", "--disallowedTools", "Bash", "--name", "<n>", prompt]` (with display_name) — verifying the pinned `[binary, *extra_args, *deny_tools, *name_args, prompt]` ordering. `CLAUDE.build_command(prompt, deny_bash=False)` argv does NOT contain the `--disallowedTools` tokens. |
| `test_cli_member.py::TestNoBashFlag` | (a) `cafleet member create --no-bash` (default) passes `deny_bash=True` to `build_command` (verified by monkey-patching `tmux.split_window` and capturing the `command` arg). (b) `cafleet member create --allow-bash` passes `deny_bash=False`. |
| `test_coding_agent.py::TestPromptTemplates` | `CLAUDE.default_prompt_template` contains the canary substring `"If your Bash tool is denied"` (verifies the bash-routing reminder is in the template) and passes `str.format(session_id=..., agent_id=..., director_name=..., director_agent_id=...)` with the standard kwargs without raising. |
| `test_cli_restructure.py::TestFlatVerbsRejected` (round 6) | For each old flat-verb subcommand, assert the invocation fails with Click's default `Error: No such command '<name>'.`. Cases: `cafleet send`, `cafleet poll`, `cafleet ack`, `cafleet cancel`, `cafleet broadcast`, `cafleet register`, `cafleet deregister`, `cafleet agents`, `cafleet get-task`, `cafleet bash-exec`. Regression guard against any future contributor accidentally re-adding a Click alias and silently re-enabling the flat form. |
| `test_tmux.py::TestSendPollTriggerKeystroke` (round 6) | Monkey-patch `tmux._run` to capture argv. Call `tmux.send_poll_trigger(target_pane_id="%0", session_id="<uuid>", agent_id="<uuid>")` once and assert the captured keystroke string contains the literal `message poll` (not `poll`). Regression guard against any future revert of the round-6 keystroke rename. |
| `test_cli_member.py::TestCodingAgentFlagRemoved` (round 6) | `cafleet member create --coding-agent claude` fails with `Error: No such option: '--coding-agent'.` (Click default) — regression guard against re-adding the flag. |
| `test_coding_agent.py::TestCodexConstantRemoved` (round 6) | Importing `CODEX`, `CODING_AGENTS`, `get_coding_agent` from `cafleet.coding_agent` each raise `ImportError` — regression guard against re-adding the registry. |
| Real-world smoke | Operator-driven: spawn a fresh member via `cafleet member create --no-bash`, have it ask for a shell command via `cafleet message send`, respond from the Director's pane with `cafleet member send-input --freetext "! <command>"`, verify the captured output appears in the member's pane and the model reads it. No separate automated smoke — the bash-routing convention is too operator-driven to mock cleanly. |

### 12. Edge cases

| Case | Behavior |
|---|---|
| Member's session is deleted between request and the Director's reply | The Director's `cafleet member send-input` fails with the soft-deleted-session error. The Director surfaces the error in its own pane but does not retry. Per `broker.delete_session` semantics, the cascade marks every member agent as `deregistered` and physically deletes their `agent_placements`, so the member's pane is gone too — the request remains in the `tasks` table for forensic inspection. |
| Member crashes between sending the request and the keystroke landing | The request stays in the queue with `status: "input_required"`. The Director's `cafleet member send-input` can still target the (now-dead) pane, but the keystroke goes nowhere. Operator can `cafleet message cancel` the request if they care. |
| Two or more concurrent shell-command requests from the same member, or from different members | The broker queues each as its own task. The Director's `cafleet message poll` returns them in its current order (newest-first). The Director processes them one at a time in that returned order — read a request, dispatch the `! <command>` keystroke, wait for the keystroke to land, then move to the next message. No new queueing primitive — the existing poll-order serialization handles concurrency. |
| Operator closes the Director's tmux pane mid-flow | The member is left blocked. On next session start, the operator runs `cafleet --session-id <s> message poll --agent-id <director-id>` to surface the unanswered request and resume. Requests are persisted in SQLite, so nothing is lost. |
| Broker server restarts mid-flow | The tmux push is best-effort and lost on restart, but the request task persists in SQLite. After the broker comes back, the Director's `/loop` cron will surface the request on its next poll cycle, and the dispatch resumes normally. |
| Operator forgets to add `Bash(cafleet member send-input *)` to `permissions.allow` | The Director's harness fires a per-call native Bash-tool prompt for each `cafleet member send-input` invocation. The bash-routing flow still works; the only cost is one extra prompt per `! <command>` dispatch. This is preexisting CAFleet UX (the same allow-rule applies to AskUserQuestion-answer dispatches from designs 0000027 / 0000033) — not a new requirement introduced by bash-routing. |
| The member sends an ambiguous request the Director can't fulfill | The Director responds via `cafleet message send` with a plain message ("Need more detail — which file?" / "I won't run that — explain why you need it"). The member processes the reply as a regular instruction. The `! <command>` keystroke is reserved for the actual fulfillment case. |
| The dispatched `! <command>` runs into a long-running process | Claude Code's `!` shortcut blocks the member's prompt context until the command finishes. There is no Director-side timeout — Claude Code's harness governs how long it waits. Operator can intervene via `cafleet member send-input` (e.g., send `Ctrl-C`) if needed. |

### 13. Future Work (pointer)

These are deliberately out of scope for v1 and listed here so reviewers do not block on them:

- **Member-side timeouts**: Add `cafleet message send --reply-timeout <s>` so a member can declare it will give up if the Director does not reply in time. Today it blocks indefinitely.
- **Codex restoration**: Codex was deprecated entirely in round 6 (§15). When upstream codex ships an equivalent of `--disallowedTools` (a primitive that gates the `shell` tool at the binary level), restore `CodingAgentConfig.CODEX` and the `--coding-agent codex` flag with full `--no-bash` parity. The restoration design doc should: (i) reference 0000034 as prior art for the deprecation rationale; (ii) reintroduce the `CODEX` constant with the new `disallow_tools_args` populated from the codex flag spelling; (iii) reintroduce `CODING_AGENTS` / `get_coding_agent()` if more than one config emerges, or keep direct imports if only two; (iv) restore `cafleet member create --coding-agent codex`; (v) add a migration check for surviving `agent_placements` rows with `coding_agent='codex'` (the rows that survived the 0000034 hard-break), validating they spawn under the new codex config; (vi) update every doc that 0000034 round 6 narrowed to "claude-only" (ARCHITECTURE.md "Multi-runner support" / "Pane display-name propagation", README features bullet, SKILL.md Member Create section, data-model.md `coding_agent` column docstring); (vii) re-add `CODEX.default_prompt_template`; (viii) revert the round-6 `FIXME(claude)` comment edit at `cafleet/src/cafleet/broker.py:17` so the codex env-var auto-detection note is restored alongside the codex agent type itself; (ix) remove the round-6 regression-guard tests (`TestCodingAgentFlagRemoved` in `test_cli_member.py` and `TestCodexConstantRemoved` in `test_coding_agent.py`) — both will start failing the moment codex is restored because the flag and the constant exist again. Replace with positive tests that assert codex `--coding-agent` parsing and `CODEX` import both succeed. Also: codex's analog (or equivalent) of Claude Code's `!` CLI shortcut needs to exist for the bash-routing convention to apply; if it doesn't, codex-spawned members would need a different routing mechanism — out of scope until the upstream primitive lands.
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
├── member                      (existing group, unchanged surface in this design)
│   ├── create
│   ├── delete
│   ├── list
│   ├── capture
│   └── send-input
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
| `cafleet member send-input` | `cafleet member send-input` | Unchanged. The bash-routing convention (round 7) reuses this command verbatim — the Director dispatches `! <command>` keystrokes via `cafleet member send-input --freetext "! <command>"`. |
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
| `skills/cafleet-monitoring/SKILL.md` | Reframe "Agent-agnostic monitoring" copy as "claude-only monitoring (codex deprecated as of design 0000034 §15; until codex grows enforcement primitives, route any codex pane through manual `cafleet member capture` only — the `--no-bash` default does not apply to codex)." |
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

- [x] Add `disallow_tools_args: tuple[str, ...] = ()` field to `CodingAgentConfig` in `cafleet/src/cafleet/coding_agent.py`. Set `CLAUDE.disallow_tools_args = ("--disallowedTools", "Bash")` and `CODEX.disallow_tools_args = ()`. (Round-5c-era state. CODEX deletion is owned by Step 13 task 1.) <!-- completed: 2026-04-28T16:10 -->
- [x] Extend `CodingAgentConfig.build_command(...)` with a `deny_bash: bool = False` keyword. When `deny_bash=True` AND `disallow_tools_args` is non-empty, inject the tokens. Pinned argv ordering: `[binary, *extra_args, *deny_args, *name_args, prompt]` — deny_args BEFORE name_args. (Mirrors §1's snippet exactly.) <!-- completed: 2026-04-28T16:10 -->
- [x] Update `CLAUDE.default_prompt_template` to add the bash-routing reminder. New round-5c-era template (Step 12 task 6 renames the literal `cafleet poll` invocation to `cafleet message poll` as part of the round-6 nested-only restructure):
  ```
  Load Skill(cafleet). Your session_id is {session_id} and your agent_id is {agent_id}.
  You are a member of the team led by {director_name} ({director_agent_id}).
  Wait for instructions via `cafleet --session-id {session_id} poll --agent-id {agent_id}`.
  Your Bash tool is denied. Route any shell command through your Director —
  see Skill(cafleet) > Routing Bash via the Director for the bash_request JSON envelope.
  ```
  <!-- completed: 2026-04-28T16:10 -->
- [x] Update `CODEX.default_prompt_template` to add the same bash-routing reminder, inlined since codex has no skills to load. New round-5c-era template (Step 13 task 1 deletes the entire CODEX constant + this template as part of the round-6 codex deprecation; until then the template lives at parity with CLAUDE):
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
  Note: literal `{` / `}` in the JSON example are doubled (`{{` / `}}`) so `str.format()` collapses them per the design 0000018 template-safety rule. <!-- completed: 2026-04-28T16:10 -->

### Step 4: Code — `cafleet member create` flag and `cafleet member exec` helper

- [x] Add `--no-bash` / `--allow-bash` boolean flag pair to `member_create` in `cafleet/src/cafleet/cli.py:555`. Click pattern: `@click.option("--no-bash/--allow-bash", default=None)`. Resolve the per-coding-agent default in the handler body: `claude` → True if unset; `codex` → False if unset and reject if explicitly True with the verbatim error message in §6 round-5c text. Pass `deny_bash=resolved_no_bash` into `coding_agent_config.build_command(...)` at the existing `tmux.split_window(... command=...)` site. (Round-5c-era state. Step 13 task 2 removes the `--coding-agent` flag and the codex rejection branch as part of the round-6 codex deprecation.) <!-- completed: 2026-04-28T16:50 -->
- [x] In the `member_create` handler body, after resolving `coding_agent_config`, reject `--no-bash --coding-agent codex` with the verbatim error message in §6 round-5c text. Exit 1 BEFORE the `register_agent` call so no broker rows are created. (Round-5c-era state. Step 13 task 2 removes this branch in round 6.) <!-- completed: 2026-04-28T16:50 -->
- [x] Add a new top-level `cafleet bash-exec` click command in `cafleet/src/cafleet/cli.py` (round-5c-era name and placement; Step 12 task 2 renames it to `cafleet member exec` under the `member` group as part of the round-6 nested-only restructure). Flags: `--cmd` (required, accepts empty), `--cwd` (optional), `--timeout` (optional, default 30; the helper itself validates `1 <= timeout <= 600` rather than Click — see input-validation note below), `--stdin` (optional). Handler order: (1) **Input validation** — if `cmd == ""` OR `timeout > 600`, write a denied JSON object (`{"status": "denied", "exit_code": 126, "stdout": "", "stderr": "<reason>", "duration_ms": 0}`) to stdout and exit 0 (do NOT raise Click UsageError; the validation failure is a payload-level outcome, not a CLI-arg error). (2) **Run** — call `subprocess.run(["bash", "-c", cmd], cwd=cwd, input=stdin, timeout=timeout, capture_output=True)`. On `subprocess.TimeoutExpired`, hard-kill via `Popen.kill()` and emit `status: "timeout"` (exit_code internally `124`, but doc treats it as opaque per the canonical-status rule). Truncate stdout/stderr at 64 KiB with the exact marker spec. Print exactly one JSON object on stdout. Helper process exit code is 0 for every payload outcome (ran/denied/timeout); non-zero only for Click's own UsageError on unknown flags. Lazy-import `subprocess` inside the handler to keep CLI startup cheap. The command silently accepts and ignores `--session-id` (matches the `db init` / `session *` / `server` pattern). <!-- completed: 2026-04-28T16:50 -->
- [x] Add a `cafleet/src/cafleet/bash_routing.py` module exposing the three pinned-signature helpers below plus a `BashRequest` / `BashResult` dataclass pair (or TypedDicts) for type narrowing. Truncation-marker logic lives in the `cafleet member exec` helper's own output (§3 helper bullet 3); field-level validation lives in the helper too (§3 helper bullet 1). `parse_bash_request` is a pure JSON-shape parser; `format_bash_result` is a pure formatter that wraps the helper's already-truncated streams with the discriminator and audit fields.

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
  <!-- completed: 2026-04-28T16:50 -->

### Step 5: Tests

- [x] Extend `cafleet/tests/test_coding_agent.py` with `TestDisallowTools` AND `TestPromptTemplates` cases from §11 round-5c text (argv-shape pinning for `TestDisallowTools` against both CLAUDE and CODEX; canary-substring + `str.format()` smoke for `TestPromptTemplates` against both CLAUDE and CODEX templates). (Round-5c-era state. Step 14 task 2 deletes every codex-specific test case in this file as part of the round-6 codex deprecation.) <!-- completed: 2026-04-28T17:00 -->
- [x] Extend `cafleet/tests/test_cli_member.py` with `TestNoBashFlag` cases from §11 round-5c text (four sub-cases: claude default `--no-bash`; explicit `--allow-bash`; codex `--no-bash` rejection with verbatim error; codex default `--allow-bash`). Reuse the existing fixture style; monkey-patch `tmux.split_window` to capture the `command` argv. (Round-5c-era state. Step 14 task 1 / task 3 prune the codex sub-cases and add `TestCodingAgentFlagRemoved` regression guard as part of the round-6 codex deprecation.) <!-- completed: 2026-04-28T17:00 -->
- [x] Add a new file `cafleet/tests/test_bash_routing_payload.py` covering `parse_bash_request` / `format_bash_result` helpers from Step 4 (round-trip, missing-field errors, truncation marker exact match). <!-- completed: 2026-04-28T17:00 -->
- [x] Add a new file `cafleet/tests/test_bash_routing_matcher.py` covering `match_allow` (per-pattern matches; allow×deny truth table from §4; ignored non-`Bash(...)` patterns; empty list returns `ask`). <!-- completed: 2026-04-28T17:00 -->
- [x] Add a new file `cafleet/tests/test_cli_bash_exec.py` covering the `cafleet bash-exec` helper's seven cases from §11 round-5c text (happy path; timeout / SIGKILL; truncation; stdin propagation; empty-cmd denied JSON; over-cap timeout denied JSON; nonexistent-cwd runtime path). All assertions switch on `status` per the canonical-status rule; `exit_code` is asserted only for `status == "ran"`. Use `CliRunner`; for the timeout test prefer a real `sleep` invocation since `subprocess.run` timeout behavior is what's under test. (Round-5c-era file name and CLI-invocation strings. Step 14 task 1 renames the file to `test_cli_member_exec.py` and rewrites every `cafleet bash-exec` invocation inside it to `cafleet member exec` as part of the round-6 nested-only restructure.) <!-- completed: 2026-04-28T17:00 -->

### Step 6: Quality gates

- [x] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: 2026-04-28T17:10 -->
- [x] Run `mise //cafleet:lint` — must pass. <!-- completed: 2026-04-28T17:10 -->
- [x] Run `mise //cafleet:format` — must pass. <!-- completed: 2026-04-28T17:10 -->
- [x] Run `mise //cafleet:typecheck` — must pass. <!-- completed: 2026-04-28T17:10 -->

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

- [x] Update `ARCHITECTURE.md`: rewrite every cafleet invocation to its nested form (`cafleet message send`, `cafleet message poll`, etc.). Update the Operation Mapping table (CLI Command → broker function) so the left column carries the new nested commands. Drop codex copy per §15 (Multi-runner support paragraph removed; Pane display-name propagation paragraph rewritten claude-only; coding_agent.py Component Layout row collapsed). <!-- completed: 2026-04-28T17:50 -->
- [x] Update `README.md`: rewrite every cafleet invocation example. Drop the multi-runner Features bullet (codex §15). <!-- completed: 2026-04-28T17:50 -->
- [x] Update `docs/spec/cli-options.md`: reorganize sections by noun group (`agent`, `message`, `member`, `session`, `db`) plus the two top-level meta-command exceptions (`server`, `doctor`). Every example invocation updates. Remove the `--coding-agent` flag row from `member create` and add the codex-deprecation pointer per §15. <!-- completed: 2026-04-28T17:50 -->
- [x] Update `docs/spec/data-model.md`: narrow the `coding_agent` column docstring to claude-only per §15. <!-- completed: 2026-04-28T17:50 -->
- [x] Update `cafleet/CLAUDE.md`: rewrite every cafleet invocation example to its nested form; drop codex references per §15. <!-- completed: 2026-04-28T17:50 -->

### Step 10: Documentation — round-6 project skills

- [x] Update `skills/cafleet/SKILL.md`: every command-reference entry, every Multi-Session Coordination invocation, every Typical Workflow step, and every example invocation moves to the nested form. Drop the `--coding-agent` flag row from `### Member Create` and every `--coding-agent codex` example (§15). Add `### Member Exec` (already specified in §10). <!-- completed: 2026-04-28T18:30 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md`: the `/loop` prompt template's literal commands move to nested form; every Stage 1 / Stage 2 / Escalation table entry updates; reframe "Agent-agnostic monitoring" copy as claude-only per §15. Add an explicit instruction near the Stall Response section: "When the Director's poll output contains a `bash_request` JSON payload, load `Skill(cafleet)` § Routing Bash via the Director and follow the 6-step dispatch." This bridges the case where a Director loaded only `Skill(cafleet-monitoring)` at session start and discovers a bash_request later — without the hint, the dispatch flow won't be in context. <!-- completed: 2026-04-28T18:30 -->
- [x] Update `skills/design-doc-create/roles/director.md`: every literal cafleet invocation moves to nested form; drop codex references if any. <!-- completed: 2026-04-28T18:30 -->
- [x] Update `skills/design-doc-execute/roles/director.md`: same edit, mirrored. <!-- completed: 2026-04-28T18:30 -->

### Step 11: Documentation — admin SPA

- [x] Update `admin/src/components/Sidebar.tsx:56`: `"cafleet register"` → `"cafleet agent register"`. <!-- completed: 2026-04-28T19:00 -->
- [x] Update `admin/src/components/Dashboard.tsx:88`: `"cafleet register"` → `"cafleet agent register"`. <!-- completed: 2026-04-28T19:00 -->

(`Dashboard.tsx:76` "cafleet db init" and `SessionPicker.tsx:53` "cafleet session create" are already nested — verify in spec phase that no edit is needed.)

### Step 12: Code — round-6 nested-only restructure (Click groups)

> **Atomic-landing instruction**: Steps 12 and 14 land atomically (same commit/PR) — Step 12's renames break existing CLI tests until Step 14's invocation rewrites land. Steps 13 and 14 also land atomically — Step 13's CODEX deletion breaks test imports until Step 14's test deletions land. Concretely: package Steps 11, 12, 13, 14 into one PR (Step 11 shares the docs-first sweep over `admin/`). Step 15's quality gates run only at the head of that combined PR, never against intermediate states.

- [x] Refactor `cafleet/src/cafleet/cli.py` to introduce `@cli.group()` decorators for `agent` and `message`. Move the existing flat-verb handlers under their new groups, applying rename-during-move atomically per the §14 mapping table: `agents` → `agent list`, `register` → `agent register`, `deregister` → `agent deregister`, `send` → `message send`, `broadcast` → `message broadcast`, `poll` → `message poll`, `ack` → `message ack`, `cancel` → `message cancel`, `get-task` → `message show`. Decorator changes + handler renames only; handler bodies unchanged. Hard-break — no Click aliases. <!-- completed: 2026-04-28T19:30 -->
- [x] Rename the round-5c `cafleet bash-exec` handler to `cafleet member exec` — move the handler from a top-level `@cli.command()` to `@member.command("exec")`. Handler body unchanged. The `Bash(cafleet member exec *)` allow-rule entry follows from this rename. <!-- completed: 2026-04-28T19:30 -->
- [x] Materialize `cafleet agent show --id <x>` as its own subcommand by extracting the existing `agents --id <x>` branch from the `agent list` handler (renamed in task 1) into a dedicated `agent.command("show")` handler. The `agent list` handler no longer accepts `--id`. <!-- completed: 2026-04-28T19:30 -->
- [x] Update the `tmux.send_poll_trigger` keystroke literal in `cafleet/src/cafleet/tmux.py` from `cafleet --session-id <s> poll --agent-id <r>` to `cafleet --session-id <s> message poll --agent-id <r>`. <!-- completed: 2026-04-28T19:30 -->
- [x] Update the `CLAUDE.default_prompt_template` literal in `cafleet/src/cafleet/coding_agent.py` so the bash-routing reminder references `cafleet --session-id {session_id} message poll --agent-id {agent_id}` (was `... poll ...`). <!-- completed: 2026-04-28T19:30 -->

### Step 13: Code — round-6 codex deprecation

> **Atomic-landing instruction**: Steps 13 and 14 land atomically with Step 12 (same PR). Step 13's CODEX deletion breaks test imports until Step 14's test deletions land. See Step 12's atomic-landing note for the full PR-packaging guidance.

- [x] Delete `CODEX = CodingAgentConfig(...)` from `cafleet/src/cafleet/coding_agent.py`. Delete `CODING_AGENTS` registry and `get_coding_agent()` helper. The module collapses to `CodingAgentConfig` dataclass + a single `CLAUDE` instance. <!-- completed: 2026-04-28T19:30 -->
- [x] Remove the `--coding-agent` flag from `member_create` in `cafleet/src/cafleet/cli.py`. Remove the round-5c codex rejection branch. Member creation always uses `CLAUDE.build_command(...)` directly. <!-- completed: 2026-04-28T19:30 -->
- [x] Update the `FIXME(claude)` comment at `cafleet/src/cafleet/broker.py:17` to drop the codex env-var reference. <!-- completed: 2026-04-28T19:30 -->

### Step 14: Tests — round-6 restructure + codex deprecation

> **Atomic-landing instruction**: Step 14 lands atomically with Steps 12 and 13 (same PR). Without this step's invocation rewrites, Step 12's CLI renames break every existing CLI test; without this step's codex deletions, Step 13's CODEX import breakage cascades through test_coding_agent.py.

- [x] Rewrite every CLI test in `cafleet/tests/test_cli_*.py` to invoke commands in their new nested form (`cafleet agent register`, `cafleet message send`, `cafleet member exec`, etc.). Rename `cafleet/tests/test_cli_bash_exec.py` to `cafleet/tests/test_cli_member_exec.py` and rewrite every `cafleet bash-exec` invocation inside to `cafleet member exec`. Test logic is unchanged; only the CLI invocation strings + file name update. <!-- completed: 2026-04-28T20:00 -->
- [x] Add `TestFlatVerbsRejected` to `cafleet/tests/test_cli_restructure.py` (new file): assert each old flat-verb invocation fails with Click's default `Error: No such command '<name>'.`. Cases: `cafleet send`, `cafleet poll`, `cafleet ack`, `cafleet cancel`, `cafleet broadcast`, `cafleet register`, `cafleet deregister`, `cafleet agents`, `cafleet get-task`, `cafleet bash-exec`. Regression guard against any future contributor accidentally re-adding a Click alias. <!-- completed: 2026-04-28T20:00 -->
- [x] Add `TestSendPollTriggerKeystroke` to `cafleet/tests/test_tmux.py`: monkey-patch `tmux._run` to capture argv; call `tmux.send_poll_trigger(target_pane_id="%0", session_id="<uuid>", agent_id="<uuid>")` once and assert the captured keystroke string contains the literal `message poll` (not `poll`). Regression guard against any future revert of Step 12 task 5. <!-- completed: 2026-04-28T20:00 -->
- [x] Delete every codex-specific test case in `cafleet/tests/test_coding_agent.py` (`TestCodexBuildCommand`, `TestCodexDisplayName`, registry-lookup tests, codex sub-cases of `TestDisallowTools` / `TestPromptTemplates`). Round-5c's `TestPromptTemplates` shrinks to claude-only per §15. <!-- completed: 2026-04-28T20:00 -->
- [x] Delete the codex sub-cases of `TestNoBashFlag` in `cafleet/tests/test_cli_member.py` (the `--no-bash --coding-agent codex` rejection case and the `codex` default `--allow-bash` case). Round-5c's `TestNoBashFlag` shrinks from 4 sub-cases to 2 per §15. <!-- completed: 2026-04-28T20:00 -->
- [x] Add `TestCodingAgentFlagRemoved` to `cafleet/tests/test_cli_member.py`: `cafleet member create --coding-agent claude` fails with `Error: No such option: '--coding-agent'.` (Click default) — regression guard. <!-- completed: 2026-04-28T20:00 -->
- [x] Add `TestCodexConstantRemoved` to `cafleet/tests/test_coding_agent.py`: importing `CODEX`, `CODING_AGENTS`, `get_coding_agent` from `cafleet.coding_agent` each raise `ImportError` — regression guard. <!-- completed: 2026-04-28T20:00 -->
- [x] Drop codex-specific spawn-command tests in `cafleet/tests/test_tmux.py` if any are codex-specific (most tests are agent-agnostic via `_run` mocking). <!-- completed: 2026-04-28T20:00 -->
- [x] Drop codex backend strings from `cafleet/tests/test_output.py` format tests. <!-- completed: 2026-04-28T20:00 -->

### Step 15: Quality gates (round 6)

- [x] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: 2026-04-28T20:00 -->
- [x] Run `mise //cafleet:lint` — must pass. <!-- completed: 2026-04-28T20:00 -->
- [x] Run `mise //cafleet:format` — must pass. <!-- completed: 2026-04-28T20:00 -->
- [x] Run `mise //cafleet:typecheck` — must pass. <!-- completed: 2026-04-28T20:00 -->

### Step 16: Real-world smoke (round 6)

- [ ] Spawn a fresh member via `cafleet member create --no-bash` (default; the `--coding-agent` flag is gone — passing it MUST fail with Click's "no such option" error). As in Step 7, the argv-shape check is covered by `test_cli_member.py::TestNoBashFlag`; this smoke verifies behavior end-to-end. Verify the member's harness rejects a Bash call. Have the member round-trip a `bash_request` through (a) auto-allow, (b) ask-Approve-as-is, (c) ask-Approve-with-edits, (d) ask-Deny-with-reason, (e) auto-allow timeout — same Step 7 case list as round-5c, but every member-side and Director-side cafleet invocation now uses the nested form. Confirm the `tmux send-keys` keystroke fired by the broker after each Director reply lands as `cafleet --session-id <s> message poll --agent-id <r>` in the member's pane and the member resumes correctly. <!-- completed: -->
- [ ] Spawn a fresh member at the head of round 6 with the `--coding-agent codex` flag (a developer migrating from the old code path). Verify it fails with Click's `Error: No such option: '--coding-agent'.`. Run `cafleet member list` and confirm the rendering. If a developer has surviving codex placement rows from before the rename (typical for upgrade-in-place environments), they should still appear in `cafleet member list` with `coding_agent='codex'` and a non-null pane_id (the migration story in §15 is verified). <!-- completed: -->

### Step 17: Finalize (round 6)

- [x] Update Status to Complete and refresh Last Updated. <!-- completed: 2026-04-29T00:05 -->
- [x] Add a Changelog entry. <!-- completed: 2026-04-29T00:05 -->

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
| 2026-04-29 | **Round 7 (Post-implementation redesign)**: User reviewed PR #37 and identified that the elaborate `bash_request` / `bash_result` JSON envelope + AskUserQuestion 3-option gate + `cafleet member exec` helper was not the intended design. New protocol: members ask via plain CAFleet messages; the Director responds with `! <command>` keystrokes via the existing `cafleet member send-input`; Claude Code's `!` CLI shortcut handles execution. Deleted `bash_routing.py`, `cafleet member exec`, and three test files (52 tests). Test suite: 545 → 493. CLAUDE prompt-template canary updated to `"If your Bash tool is denied"`. Doc surfaces rewritten to describe the simpler convention. Sections affected: §1 unchanged (flag pair stays); §2 rewritten (plain message, no envelope); §3 collapsed from 6-step dispatch + matcher to a 3-step protocol; §4 (allow-list matcher), §8 (subprocess limits) deleted; §5–§7, §9–§13 trimmed accordingly; §14 (restructure) + §15 (codex) untouched. SC § Bash-via-Director rewritten 11 → 6 criteria. Implementation step checkboxes stay as historical record of the build + unwind. Six prior Changelog rounds retained verbatim. Net unchecked markers: 7 (deferred round-5c + round-6 smoke + finalize). Concurrent requests serialize through `cafleet message poll`'s return order (currently newest-first); no new queueing primitive. |
| 2026-04-29 | **Status: Complete.** Implementation landed on PR #37. Five Copilot review rounds folded in (R1: 9 comments → R2 fixes; redesign push; R3: 6 doc fixes; R4: 8 fixes incl. FIFO factual bug; R5: 1 design-doc FIFO sweep follow-up; R6: 7-file skill-tree sweep miss). Final Copilot review (00:03Z) reported "no new comments" — quiescent exit. 493/493 tests pass; lint/format/typecheck green. Step 16 (operator-driven smoke) and the round-5c Step 7 / Step 8 deferred-finalize tasks remain unchecked — they are operator-driven verification deferred to post-merge usage. Step 17 (round-6 finalize) checked off. Header `**Progress**: 56/63` → `58/63`. |
| 2026-04-29 | **Round 8 (Copilot review follow-up: dedicated `--bash` flag + auth-check + symbol rename)**: Three implementation deltas surfaced by the Copilot review of the round-7 finalize commit `2c5fcf2`. **(1)** New `cafleet member send-input --bash "<cmd>"` flag — dedicated bash-routing path that keystrokes literal `! <cmd>` + Enter (no leading `4` digit). Replaces the round-7 idiom of `--freetext "! <cmd>"`, which was misleading because `--freetext` is AskUserQuestion-only (it prepends `4` to select "Type something"). New `tmux.send_bash_command(*, target_pane_id, command)` helper issues exactly two `send-keys` calls (literal `! <cmd>` then Enter); rejects empty commands and embedded `\n` / `\r`. Mutual-exclusion error rewritten: `--choice, --freetext, --bash are mutually exclusive; supply exactly one.` (legacy 2-flag wording retired). Audit log records `action="bash"`, `value="<cmd>"`. **(2)** `cafleet message show` now calls `broker.verify_agent_session(agent_id, session_id)` before `broker.get_task`, mirroring `agent list` / `agent show`. Closes a hole where any process holding the SQLite file could fetch any task by ID without proving session membership. Error wording: `agent <id> is not a member of session <sid>.` (matches the existing pattern). **(3)** Symbol rename `cli.agent_list_` → `cli.agent_list`. The trailing underscore was an unnecessary defense against shadowing Python's built-in `list`; the local in-function variable was renamed `agent_list` → `agents` to avoid shadowing the new function name. Click registration via `@agent.command("list")` is unchanged. **Tests added (16)**: `TestSendBashCommand` (×7 in `test_tmux.py`: keystroke order, empty-command rejection, newline rejection ×5), `TestBashFlag` (×5 in `test_cli_member_send_input.py`: send semantics, mutual exclusion ×3, audit log JSON), `TestMessageShowAuthCheck` (×2 in new `test_cli_message.py`: gate-rejected and gate-accepted), `TestAgentListHandlerName` (×2 in `test_cli_restructure.py`: function `__name__` and old-symbol non-export). **Test count delta**: 493 → 509 passing. Doc surface updates: cli-options.md (add `--bash` row + key-sequence + validation rows + JSON output example), `skills/cafleet/SKILL.md` (rewrite `### Member Send-Input` to describe 3 flags; replace `--freetext "! <cmd>"` examples with `--bash "<cmd>"`; remove the misleading "trailing newline appended automatically" sentence), `skills/cafleet-monitoring/SKILL.md` (escalation table row mentions `--bash` alongside `--choice` / `--freetext`; bash-request blocking-case row references `--bash` directly), ARCHITECTURE.md (`tmux.py` helper inventory adds `send_bash_command`; operation mapping row for `member send-input` lists all three helpers; Bash Routing § rewritten to use `--bash`), README.md (CLI surface row updated). The two pre-existing `TestFlagValidation` tests in `test_cli_member_send_input.py` had assertions on the legacy 2-flag wording; updated to assert the new 3-flag wording (substring contains `--choice`, `--freetext`, `--bash`, `mutually exclusive`). All quality gates pass: 509 tests, lint clean, format clean, typecheck clean. Round 8 is post-Complete maintenance, not a new implementation step — Status remains Complete and Progress remains 58/63. |
| 2026-04-29 | **Round 9 (Copilot review follow-up: auth-check coverage extended to 4 more handlers)**: Round-8 added the `verify_agent_session` gate to `message show`. Round-9 addresses Copilot's suppressed-comments observation that the same hole was open on every other read/write CLI handler that takes `--agent-id` + `--session-id`: a caller could pass any session_id and either drain another session's inbox (`message poll`), tamper with another session's task lifecycle (`message ack` / `message cancel`), or unilaterally evict another session's agents (`agent deregister`). All four handlers in `cafleet/src/cafleet/cli.py` now call `broker.verify_agent_session(agent_id, ctx.obj["session_id"])` first, raising `click.ClickException(f"agent {agent_id} is not a member of session {sid}.")` on False — matching the existing `agent list` / `agent show` / `message show` wording exactly. The gate runs inside `with _handle_broker_errors():` so the exit code (1) and message format are uniform across the auth surface. **(1)** `message_poll` (cli.py:364) — gate before `broker.poll_tasks`. **(2)** `message_ack` (cli.py:383) — gate before `broker.ack_task`. **(3)** `message_cancel` (cli.py:399) — gate before `broker.cancel_task`. **(4)** `agent_deregister` (cli.py:472) — gate before `broker.deregister_agent`. The root-Director protection inside `broker.deregister_agent` is unaffected: it still fires for the root-Director's `agent_id` (which IS a session member, so the new gate accepts and the broker layer rejects). The Administrator-deregister guard likewise still fires. **Tests added (8)**: `TestMessagePollAuthCheck` (×2 in `test_cli_message.py`), `TestMessageAckAuthCheck` (×2), `TestMessageCancelAuthCheck` (×2), `TestAgentDeregisterAuthCheck` (×2 in new `test_cli_agent.py`). Each pair verifies (a) gate-rejected: `verify_agent_session` returns False → exit 1 with the expected substrings (`agent_id`, `"not a member of session"`, `session_id`) AND the underlying broker call NOT invoked, and (b) gate-accepted: `verify_agent_session` returns True → broker call proceeds with the expected args. **Test count delta**: 509 → 517 passing. The 1 pre-existing test in `test_cli_session_flag.py::TestDeregisterAdministratorCliGuard::test_cli_deregister_unknown_agent_exits_nonzero` had a stale assertion on the broker-layer "not found or already deregistered" wording; updated to assert the new gate's "not a member of session" wording (the new path fires earlier for unknown agents and is more accurate — the gate cannot distinguish "unknown" from "known but in a different session" and treats both as not-a-member). Doc surface updates: cli-options.md gained one row in the Error Messages table covering all 7 commands now under the gate (the 3 R8/pre-R9 commands plus the 4 R9 commands). All quality gates pass: 517 tests, ruff check clean, ty check clean. Round 9 is post-Complete maintenance like Round 8 — Status remains Complete and Progress remains 58/63. |
| 2026-04-29 | **Round 11 + Round 12 (Design 0000035 — initially shipped Option A, then revised in-place to Option D dontAsk mode)**: Design 0000035 was filed mid-PR-#37 review to address an implicit-`!`-shortcut gap in the bash-via-Director feature: members spawned with `--disallowedTools "Bash"` could not reliably invoke `cafleet message poll/send/ack` themselves. R11 first shipped Option A (keep the deny posture, document and wire the `!` prefix everywhere — spawn-prompt template + `tmux.send_poll_trigger` keystroke). R11 smoke testing revealed the LLM-judgment problem: members reliably ran broker-pushed `!`-shortcut keystrokes but unreliably fired the shortcut for member-composed cafleet calls (e.g. `cafleet message send` replies stalled). R12 revised design 0000035 in-place to Option D: switch member spawn argv from `--disallowedTools "Bash"` to `--permission-mode dontAsk` and remove the `--no-bash` / `--allow-bash` flag pair from `cafleet member create` entirely. Members now spawn with the Bash tool ENABLED and permission prompts auto-resolve silently — they run cafleet (and any other shell command) directly via the Bash tool, no `!` prefix needed, no operator interaction. The bash-via-Director protocol introduced in 0000034 (`cafleet member send-input --bash <cmd>` keystroke dispatch) is preserved as an opt-in escape hatch for cases that warrant Director-level oversight, but is no longer the default flow. **Reverted from R11**: the `! ` prefix on `tmux.send_poll_trigger`'s keystroke; the `!`-prefix wording in `CLAUDE.default_prompt_template`; the `--no-bash` / `--allow-bash` Click options on `member create`; the `disallow_tools_args` field name (renamed `permission_args`) and the `deny_bash` parameter on `build_command`. **Test class renames**: `TestDisallowTools` → `TestPermissionArgs`; `TestNoBashFlag` → `TestPermissionMode` with new regression guards confirming Click rejects `--no-bash` and `--allow-bash`; `TestPromptTemplates` canaries swapped (`"dontAsk"` is the new positive canary; `"! cafleet"` is the new negative canary); `TestSendPollTriggerKeystroke::test_keystroke_carries_bang_prefix_for_message_poll` → `test_keystroke_starts_with_bare_cafleet`. **Doc sweep**: every reference to `--no-bash` / `--allow-bash` / `--disallowedTools "Bash"` / "prefix every cafleet call with `!`" removed from `README.md`, `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `skills/cafleet/SKILL.md`, `skills/cafleet/roles/{member,director}.md`, `skills/design-doc-create/roles/director.md`, `skills/design-doc-execute/roles/director.md`, and `.claude/rules/bash-tool.md` (rewritten for the dontAsk default). **Smoke verified end-to-end** before commit: spawned member ran `git branch --show-current` directly via Bash tool (no operator prompt, no `!` prefix) and replied via `cafleet message send` cleanly. **Test count**: 517 passing. All quality gates green. Round 11+12 is post-Complete maintenance — Status remains Complete and Progress remains 58/63. See [`design-docs/0000035-member-bash-whitelist/design-doc.md`](../0000035-member-bash-whitelist/design-doc.md) for the full revised design (Option D chosen; A/B/C in Future Work). |
