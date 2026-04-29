# Member spawn permission posture (dontAsk mode)

**Status**: Approved
**Progress**: 4/4 tasks complete
**Last Updated**: 2026-04-29

## Overview

CAFleet members were originally spawned with `--disallowedTools "Bash"` (design 0000034 default), which removed the Bash tool from the member's harness entirely. The intent was to force shell commands through the Director for human oversight (the bash-via-Director protocol). In practice this proved fragile: the Bash-tool deny posture had no clean strict-allowlist alternative for cafleet calls (the smoke tests below explored three flag combinations, none delivered), and the `!` CLI-shortcut workaround documented as a fallback was unreliable — the LLM frequently composed `! cafleet ...` as response text without firing the shortcut, leaving messages stuck in the inbox.

This design switches the spawn posture to `--permission-mode dontAsk` (Bash tool **enabled**, permission prompts auto-resolve silently) and removes the `--no-bash` / `--allow-bash` flag pair entirely. Members run cafleet (and any shell command) directly via the Bash tool, no operator interaction, no `!` workaround. The bash-via-Director protocol from design 0000034 stays in the codebase as an opt-in escape hatch (`cafleet member send-input --bash <cmd>` still works for Director-driven dispatch), but is no longer the default flow.

## Success Criteria

- [x] `cafleet member create` spawns members with `--permission-mode dontAsk` injected into the spawn argv. Asserted by `tests/test_cli_member.py::TestPermissionMode::test_claude_default_injects_dontask_permission_mode`.
- [x] The legacy `--no-bash` / `--allow-bash` flags are removed from `cafleet member create` and Click rejects them with `No such option`. Asserted by `tests/test_cli_member.py::TestPermissionMode::test_no_bash_flag_no_longer_parses` and `test_allow_bash_flag_no_longer_parses`.
- [x] The CLAUDE spawn-prompt template tells the member that its harness runs in dontAsk mode and the Bash tool is enabled — no `!` prefix workaround required. Asserted by `tests/test_coding_agent.py::TestPromptTemplates::test_claude_template_documents_dontask_mode` and `test_claude_template_omits_legacy_bang_prefix_guidance`.
- [x] The broker's `tmux.send_poll_trigger` keystroke is the bare cafleet command (no `!` prefix). Asserted by `tests/test_tmux.py::TestSendPollTriggerKeystroke::test_keystroke_starts_with_bare_cafleet`.

---

## Background

### Why the original design needed revision

`cafleet member create --no-bash` (the design 0000034 default) appended `--disallowedTools "Bash"` to the spawn argv. Consequence: the member's harness rejected every Bash tool call. The member had to either (a) ask the Director to run the command via `cafleet member send-input --bash <cmd>` (the bash-via-Director protocol), or (b) use Claude Code's `!` CLI shortcut for cafleet calls (`! cafleet message poll ...`), which is a separate primitive unaffected by the Bash-tool deny posture.

Two practical problems with the deny-Bash posture:

1. **The `!` workaround is implicit and unreliable.** The spawn prompt could document the prefix, and the broker's tmux push notification could inject the prefix automatically — both were done in design 0000035's first iteration (Steps 1 and 2). But when the member needs to compose a NEW cafleet call mid-turn (e.g., reply to the Director after processing a poll result), the LLM frequently emits the line as response text without firing the `!` shortcut. Smoke tests on commit `ca3462e` showed the member retrieving messages successfully (broker push fires the shortcut) but failing to compose-and-fire the reply.

2. **Operator overhead from permission prompts on cafleet.** Even if we drop `--disallowedTools "Bash"` and let the member call cafleet via the Bash tool directly, every Bash invocation triggers a permission prompt unless allowlisted. Allowlisting `Bash(cafleet *)` works in `default` mode but is not strict (other Bash patterns auto-approve too — the smoke test confirmed `echo` went through), and `dontAsk` mode with `--allowedTools` is over-restrictive (denies Read/Edit/Grep too — member becomes wedged).

### Smoke-test results from 2026-04-29 PR #37 follow-up

Three approaches were tested by editing `coding_agent.py` and re-spawning members:

| Approach (spawn argv) | `cafleet message send` via Bash tool | `Bash(echo test)` | Verdict |
|---|---|---|---|
| `--disallowedTools "Bash"` (Round-7 default; Round-8 doc-template-only `!` reminder) | Denied (must use `!` shortcut) | Denied | LLM unreliable at firing `!` mid-turn |
| `--allowedTools "Bash(cafleet *)"` | ✅ Allowed | ❌ **Also allowed** (`default` mode auto-approves unmatched) | Leaks all bash; not a strict whitelist |
| `--settings '{"permissions":{"deny":["Bash(*)"],"allow":["Bash(cafleet *)"]}}'` | Denied | Denied | Worse — `deny` removes Bash entirely; `allow` doesn't bring it back |
| `--permission-mode dontAsk --allowedTools "Bash(cafleet *)"` | Denied | Denied | Member wedges — `dontAsk` denies all non-listed tools (Read, Edit, Grep…) silently |

**Round 7 (committed, then revised)** prefixed the broker's tmux push keystroke with `! ` and added wording to the spawn template, but the LLM-judgment problem on member-initiated cafleet sends remained. Smoke test on `ca3462e`: broker push poll worked; member-initiated send didn't fire reliably.

**The chosen path (Option D)** is `--permission-mode dontAsk` with no allowlist restriction — Bash tool stays enabled, permission prompts auto-resolve, member calls cafleet directly via the Bash tool, no LLM-judgment dependency. Smoke test on the dontAsk wiring (verified before commit): member ran `git branch --show-current` and replied via `cafleet message send` cleanly with no operator interaction, no `!` prefix anywhere.

---

## Specification

### Options considered

#### Option A — `!` shortcut + better docs (initially shipped, later revised)

Keep `--disallowedTools "Bash"`. Document the `!` prefix in the spawn template. Pre-prefix the broker's tmux push keystroke. Members use `! cafleet ...` for every cafleet call.

**Why this didn't ship:** the `!` shortcut is reliable when the harness types it from outside (broker push notification works), but unreliable when the LLM has to emit-and-fire it mid-turn. Member-initiated `cafleet message send` calls regularly stalled.

#### Option B — PreToolUse hook gate

Drop `--disallowedTools "Bash"`. Configure a `PreToolUse` hook that intercepts every Bash invocation and rejects non-cafleet patterns. Bash is generally available; the hook is the gate.

**Why this didn't ship:** larger surface (new shell script + `--settings` plumbing), and the hook becomes a new failure mode. Deferred.

#### Option C — Proxy `cafleet-member` CLI

Drop `--disallowedTools "Bash"`. Create a dedicated `cafleet-member` binary; allow only `Bash(cafleet-member *)`. The proxy validates and forwards.

**Why this didn't ship:** marginal benefit over A/B; adds a whole CLI to maintain. Deferred.

#### Option D — `dontAsk` mode (chosen)

Drop `--disallowedTools "Bash"`. Add `--permission-mode dontAsk` to the spawn argv. Bash tool is enabled and permission prompts auto-resolve (no operator interaction). Members call cafleet (and any shell command) directly via the Bash tool. The bash-via-Director protocol from design 0000034 (`cafleet member send-input --bash`) stays available as an opt-in for cases where Director-driven dispatch is wanted, but is no longer the default flow.

**Why this ships:**

- Simplest possible wiring — single argv change, no protocol layer, no LLM-judgment dependency.
- Verified working end-to-end via smoke test (member ran `git branch --show-current` and replied via `cafleet message send` cleanly).
- The `--no-bash` / `--allow-bash` flag pair becomes vestigial; removing it cleans up the CLI surface.
- The bash-via-Director protocol in design 0000034 is preserved verbatim (the `--bash` flag, the `cafleet member send-input` machinery) — operators who want Director-level oversight on shell commands can still get it by using `cafleet member send-input --bash <cmd>` directly. The protocol is just no longer the default.

### Trust model

The dontAsk model assumes the spawned member is **trusted to the same level as the operator**. If you wouldn't trust the LLM to run `rm -rf` in your home dir, don't use this default. Future opt-in modes (Options B/C, or sandboxing) can be layered on later if richer trust gradients are needed.

---

## Implementation

### Step 1: Spawn argv

- [x] `cafleet/src/cafleet/coding_agent.py` — replace the `disallow_tools_args` field with `permission_args` (always-injected; default `()`). Set `CLAUDE.permission_args = ("--permission-mode", "dontAsk")`. Drop the `deny_bash` parameter from `build_command`. <!-- completed: 2026-04-29T03:10 -->
- [x] `cafleet/src/cafleet/cli.py` — remove the `--no-bash` / `--allow-bash` flag pair from `member create`. Update the `member_create` body to drop the `no_bash` / `deny_bash` plumbing. <!-- completed: 2026-04-29T03:10 -->

### Step 2: Broker tmux push keystroke

- [x] `cafleet/src/cafleet/tmux.py` — `send_poll_trigger` keystroke reverts to bare `f"cafleet --session-id {sid} message poll --agent-id {aid}"` (no `! ` prefix). Update docstring to describe the dontAsk model. <!-- completed: 2026-04-29T03:10 -->

### Step 3: CLAUDE prompt template + tests + docs

- [x] `cafleet/src/cafleet/coding_agent.py` `CLAUDE.default_prompt_template` — rewrite to describe the dontAsk model. Wait-instructions example uses bare `cafleet message poll` (no `!`). Body says "Your harness runs in dontAsk mode — your Bash tool is enabled and permission prompts auto-resolve, so call cafleet (and any other shell command) directly via the Bash tool. No prefix workaround is needed." <!-- completed: 2026-04-29T03:10 -->
- [x] `cafleet/tests/test_coding_agent.py` — replace `TestDisallowTools` with `TestPermissionArgs`. Replace `test_claude_template_contains_bash_routing_canary` and `test_claude_template_documents_bang_prefix_for_cafleet` with `test_claude_template_documents_dontask_mode` (canary: `"dontAsk"`) + `test_claude_template_omits_legacy_bang_prefix_guidance` (negative canary: `"! cafleet"` NOT present). Update `TestBuildCommand` expected argv. <!-- completed: 2026-04-29T03:10 -->
- [x] `cafleet/tests/test_cli_member.py` — replace `TestNoBashFlag` with `TestPermissionMode`. Three tests: `test_claude_default_injects_dontask_permission_mode` (positive — the new mode is in argv), `test_no_bash_flag_no_longer_parses` (regression guard — Click rejects), `test_allow_bash_flag_no_longer_parses` (regression guard — Click rejects). <!-- completed: 2026-04-29T03:10 -->
- [x] `cafleet/tests/test_tmux.py` — `TestSendPollTrigger::test_success_returns_true` and `TestSendPollTriggerKeystroke::test_keystroke_starts_with_bare_cafleet` updated to assert the bare keystroke (no `! ` prefix). <!-- completed: 2026-04-29T03:10 -->

### Step 4: Doc sweep + design 0000034 R12 changelog

- [x] `skills/cafleet/SKILL.md` — `## Routing Bash via the Director` section: clarify that the protocol is now opt-in (Director-initiated dispatch) rather than the default, and remove the member-side `!`-prefix subsection introduced in the prior design 0000035 iteration. <!-- completed: 2026-04-29T03:11 -->
- [x] `skills/cafleet/roles/member.md` — rewrite for the dontAsk model: members can run cafleet (and other shell commands) directly via the Bash tool. The "ask Director" path is preserved as an OPTIONAL fallback when the member wants Director-level oversight. <!-- completed: 2026-04-29T03:11 -->
- [x] `skills/cafleet/roles/director.md` — note that the bash-routing protocol is opt-in under the dontAsk model. <!-- completed: 2026-04-29T03:11 -->
- [x] `ARCHITECTURE.md` `## Bash Routing via Director` — same: protocol is opt-in. Update the spawn-argv description to reflect `--permission-mode dontAsk` instead of `--disallowedTools "Bash"`. <!-- completed: 2026-04-29T03:11 -->
- [x] `design-docs/0000034-member-bash-via-director/design-doc.md` — append a Round 12 changelog row noting that R11's `!`-prefix wiring was reverted in favor of dontAsk mode, and the bash-routing protocol introduced in 0000034 is now opt-in. <!-- completed: 2026-04-29T03:11 -->

---

## Future Work

If the dontAsk model proves too permissive in production (e.g. teams that want to gate destructive commands behind operator confirmation), revisit:

- **Option B (PreToolUse hook)** — `cafleet member create` writes a per-member `.claude/settings.json` with a `PreToolUse` hook that gates Bash invocations to a configurable pattern set. Bash remains generally available but every call is hook-validated.
- **Option C (proxy binary `cafleet-member`)** — a dedicated CLI binary that wraps relevant cafleet subcommands; allow only `Bash(cafleet-member *)` via Claude Code's allowlist. Narrower attack surface; another CLI to maintain.
- **Layered trust modes** — `cafleet member create --trust-level <low|default|high>` selects between `dontAsk`, hook-gated, and fully-restricted.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-29 | Initial draft (Option A — `!` shortcut + better docs). Three approaches smoke-tested; Option A chosen as v1 fix. |
| 2026-04-29 | **Revised in-place to Option D (dontAsk mode).** Smoke testing on commit `ca3462e` (Option A as committed) revealed the LLM-judgment problem: members reliably run broker-pushed `!`-shortcut keystrokes, but unreliably fire the shortcut for member-composed cafleet calls (e.g. `cafleet message send` replies). Three additional flag combinations were tested (`--allowedTools` strict-whitelist, `--settings` deny+allow, `dontAsk + --allowedTools` restrictive) — none delivered. The simplest working solution is to drop the deny-Bash posture entirely and use `--permission-mode dontAsk` instead. Smoke test on the dontAsk wiring confirmed end-to-end: member ran `git branch --show-current` and replied via `cafleet message send` with no operator interaction, no `!` prefix. R11's `!`-prefix wiring on `coding_agent.py` (CLAUDE template) and `tmux.py` (`send_poll_trigger` keystroke) is reverted; the `--no-bash` / `--allow-bash` flag pair on `cafleet member create` is removed. The bash-via-Director protocol from design 0000034 is preserved as an opt-in escape hatch (`cafleet member send-input --bash <cmd>` still works) but is no longer the default flow. |
