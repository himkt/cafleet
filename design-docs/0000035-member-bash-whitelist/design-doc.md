# Member-side Bash whitelist (allow only `Bash(cafleet *)`)

**Status**: Approved
**Progress**: 2/7 tasks complete
**Last Updated**: 2026-04-29

## Overview

CAFleet members today are spawned with `--disallowedTools "Bash"`, which removes the Bash tool entirely. This forces them to use Claude Code's `!` CLI shortcut as a workaround for `cafleet message poll/send/ack` calls — fragile because the LLM has to remember the prefix on every cafleet invocation. We want to instead allow ONLY `Bash(cafleet *)` patterns and deny everything else, so the member can use the Bash tool normally for cafleet calls but cannot run other shell commands.

## Success Criteria

- [ ] A spawned member can invoke `cafleet --session-id <s> message poll --agent-id <a>` via the Bash tool without permission prompt and without using the `!` shortcut.
- [ ] The same member's attempt to invoke `Bash(echo test)` (or any non-cafleet pattern) is denied at the harness level, surfacing as a tool-use error rather than executing.
- [ ] The member retains access to read-only and editing tools it needs (Read, Edit, Grep, Glob, etc.) — no other tool axis is restricted.
- [ ] The existing `--bash` route via `cafleet member send-input --bash <cmd>` continues to work for Director-initiated shell execution (the `!` keystroke shortcut is independent of the Bash tool's allow/deny posture, but verify the integration end-to-end).

---

## Background

### Why the current design needs this

`cafleet member create --no-bash` (default since design 0000034) appends `--disallowedTools "Bash"` to the spawn argv. Consequence: the member's harness rejects every Bash tool call. The member's spawn-prompt template tells it to "Wait for instructions via `cafleet message poll`" but it cannot actually run that command via the Bash tool — only via the `!` shortcut, which is a separate Claude Code primitive.

Two practical problems with the status quo:

1. **The `!` workaround is implicit.** The spawn prompt does not document the prefix, and the LLM does not always reach for it. When the broker pushes a `cafleet message poll` keystroke into the member's pane (via `tmux.send_poll_trigger`), the keystroke is the bare command without `!`. Whether the LLM correctly executes it depends on Claude Code's user-input handling. In smoke tests, members frequently respond with text like "I cannot run that command — Bash is denied" instead of using `!`.

2. **Members cannot directly write back via cafleet.** Even when a member receives a clear instruction like "reply via `cafleet message send`", with Bash denied entirely it must either route the cafleet call through `!` (relying on the LLM to get the prefix right) or it stalls.

### Smoke-test results from 2026-04-29 PR #37 follow-up

Three approaches were tested by editing `coding_agent.py` and re-spawning members:

| Approach (spawn argv) | `cafleet message send` via Bash tool | `Bash(echo test)` | Verdict |
|---|---|---|---|
| `--disallowedTools "Bash"` (status quo) | Denied (must use `!` shortcut) | Denied | Workable but fragile |
| `--allowedTools "Bash(cafleet *)"` | ✅ Allowed | ❌ **Also allowed** (auto-approved) | Leaks all bash |
| `--settings '{"permissions":{"deny":["Bash(*)"],"allow":["Bash(cafleet *)"]}}'` | Denied | Denied | Worse — also blocks `!` |

`--allowedTools` is documented as "tool names to allow" — it adds patterns to the auto-approve list, but the Bash tool itself remains generally available, so non-matching invocations flow through default permission resolution (and in many test environments auto-approve). It is NOT a strict whitelist.

`--settings` with a deny+allow pair removes Bash from the available tool list outright — Claude Code seems to compute "Bash available iff at least one allow pattern matches" before considering deny patterns, OR the deny pattern wins in same-tool conflicts. Either way the allow pattern does not bring Bash back.

There is no single Claude Code CLI flag combination that achieves "allow only `Bash(cafleet *)`, deny everything else" cleanly. The platform's permission model treats Bash as binary-available + pattern-auto-approve, not as a strict allowlist.

---

## Specification

Three candidate paths, in order of preference. Pick one based on Claude Code's evolving permission model.

### Option A — wait for native strict-whitelist support (Recommended short-term)

Status: blocked on Claude Code adding a flag like `--strict-allowedTools` or extending `--permission-mode` with a "denyByDefault" mode. Until that lands, we keep `--disallowedTools "Bash"` and:

1. **Update the CLAUDE spawn-prompt template** to explicitly document the `!` shortcut: "Run cafleet calls via the `!` prefix — `! cafleet --session-id ... message poll --agent-id ...`. Do NOT attempt to use the Bash tool for these calls; it is denied at the harness level."

2. **Update the broker's tmux push notification** in `tmux.send_poll_trigger` to inject `! cafleet message poll ...` (with `!` prefix) instead of the bare command. This way the keystroke arrives in a form the LLM will execute via the shell shortcut without needing to add the prefix itself.

3. **Document the limitation** in `skills/cafleet/SKILL.md` § Routing Bash via the Director: members can ONLY shell out via `!` shortcut for `cafleet *` calls, OR via Director-initiated `cafleet member send-input --bash` keystrokes. They cannot run arbitrary `cafleet` calls through the Bash tool today.

Cost: 2 small edits + 1 doc edit. Lands in a single commit.

### Option B — hook-based gate on the Bash tool

Configure a `PreToolUse` hook in the member's settings that intercepts Bash tool calls and rejects any pattern that doesn't match `cafleet *`. This is an out-of-band enforcement layer that doesn't depend on Claude Code's built-in permission semantics.

1. `cafleet member create` writes a `.claude/settings.json` (or passes `--settings <inline-json>`) into the member's working directory with a `PreToolUse` hook for `Bash`.
2. The hook is a small shell script that reads the tool input from stdin, checks if `command` starts with `cafleet `, and exits 0 (allow) or 2 (block with stderr message).
3. `--disallowedTools "Bash"` is dropped — Bash is generally available, but the hook gates every invocation.

Risks: hook must be reliable; a buggy hook either over-blocks (member wedges) or under-blocks (security hole). Hook failure mode would need clear error messages.

Cost: small new shell script + plumbing in `cafleet member create` to write the settings file. Larger surface than Option A, smaller than Option C.

### Option C — proxy `cafleet` invocations through a dedicated CLI tool

Create a `cafleet-member` CLI subcommand that the member calls instead of `cafleet ... message poll/send/ack`, and grant only `Bash(cafleet-member *)` via Claude Code's allowlist mechanism (same as Option A would, just with a narrower binary name). The proxy validates and forwards to the real broker.

Marginal benefit over Option A/B: clearer attack surface (only one binary need be safe). Marginal cost: another CLI subcommand to maintain.

### Decision

**Option A is the chosen path** for v1, executed in PR #37 alongside design 0000034. Options B and C are deferred to Future Work — they remain valid evolution paths if the `!` shortcut workaround proves insufficient in production.

---

## Implementation

### Step 1: Update CLAUDE spawn-prompt template

- [x] Edit `cafleet/src/cafleet/coding_agent.py` `CLAUDE.default_prompt_template` to mention the `!` shortcut explicitly. The new wording must clearly tell the member: cafleet calls (poll/send/ack) MUST be prefixed with `!` because the Bash tool is denied; do NOT attempt Bash tool calls. <!-- completed: 2026-04-29T02:25 -->
- [x] Add a `TestPromptTemplates` canary in `cafleet/tests/test_coding_agent.py` asserting the new substring (e.g. `"! cafleet"` or `"shell shortcut"`). <!-- completed: 2026-04-29T02:25 -->

### Step 2: Prefix tmux poll trigger with `!`

- [ ] Edit `cafleet/src/cafleet/tmux.py` `send_poll_trigger` to inject `f"! cafleet --session-id {session_id} message poll --agent-id {agent_id}"` (with leading `! `) instead of the bare command. The keystroke arrives in the member's pane and the LLM's harness routes it through Claude Code's `!` CLI shortcut without going through the Bash tool. <!-- completed: -->
- [ ] Update `TestSendPollTriggerKeystroke` in `cafleet/tests/test_tmux.py` to assert the captured first keystroke is `! cafleet --session-id <s> message poll --agent-id <a>` (with the `!` prefix). <!-- completed: -->

### Step 3: Documentation sweep

- [ ] `skills/cafleet/SKILL.md` § Routing Bash via the Director — add a member-side subsection: members MUST prefix every cafleet call with `! ` because the Bash tool is denied; the `!` shortcut is independent of the Bash tool's allow/deny posture. <!-- completed: -->
- [ ] `ARCHITECTURE.md` Bash Routing section — add the same `!` prefix requirement for member-side cafleet calls; clarify that the broker's tmux push notification injects `! cafleet message poll ...` (with prefix). <!-- completed: -->
- [ ] `design-docs/0000034-member-bash-via-director/design-doc.md` — append a final Round 11 Changelog entry referencing design 0000035 as the resolution for the implicit-`!`-shortcut gap, and noting that Round 11 lands in PR #37 alongside the rest of the 0000034 work. <!-- completed: -->

---

## Future Work

If Option A's `!` shortcut workaround proves insufficient in production usage, evaluate:

- **Option B (PreToolUse hook)** — `cafleet member create` writes a per-member `.claude/settings.json` with a `PreToolUse` hook that gates Bash invocations to `cafleet *` patterns. Bash remains generally available but every call is hook-validated. Trade-off: hook reliability becomes a new failure surface.
- **Option C (proxy binary `cafleet-member`)** — a dedicated CLI binary that wraps the relevant cafleet subcommands; allow only `Bash(cafleet-member *)` via Claude Code's allowlist. Marginal benefit over A: narrower attack surface. Marginal cost: another CLI to maintain.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-29 | Initial draft. Three approaches smoke-tested in PR #37 follow-up; all fail to deliver strict whitelist via native Claude Code flags. Option A (`!` shortcut + better docs) recommended as v1 fix. |
