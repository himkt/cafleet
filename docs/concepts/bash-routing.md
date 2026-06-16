---
icon: lucide/terminal
---

# Bash routing

Members spawn with workspace-scoped auto-approval enabled — `claude` uses
`--permission-mode dontAsk`, `codex` uses `--ask-for-approval never
--sandbox workspace-write`, and `opencode` uses `--agent cafleet` to bind
the `cafleet` agent's permission ruleset (catch-all-allow + specific-deny —
every permission check resolves to `allow` or `deny`, never `ask`). The Bash
tool is enabled and routine permission prompts auto-resolve
silently, so members run cafleet (and any shell command) directly via the
Bash tool. The default spawn-prompt template tells the member explicitly
that its harness runs in workspace-scoped auto-approve mode.

The bash-via-Director protocol is the **fallback** for the harness deny-list:
workspace-scoped auto-approval does not auto-resolve everything — destructive
operations such as `git push` and `rm -rf` are still rejected at the coding
agent's harness layer. When a member's Bash invocation is denied, the member
auto-routes by sending a plain CAFleet message to its Director, and the
Director dispatches the command into the member's pane via `cafleet member
exec "<cmd>"`, which via the `tmux.send_bash_command` helper keystrokes literal
`! <cmd>` + `Enter` and triggers the coding agent's `!` CLI shortcut on the
receiving side (honored by `claude`, `codex`, and `opencode`).

Members must first reconsider whether the rejected command is correct and
necessary — most denials are caused by a wrong command, not a missing
privilege. The full convention, including the member-side reconsider step,
the Director-side `member exec` dispatch, the serialization rules, and the
cross-fleet boundary, lives in `skills/cafleet/reference/exec-routing.md`.
