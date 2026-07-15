---
icon: lucide/bot
---

# Coding-agent backends

Per-backend operational reference for the three coding-agent binaries a member
pane can run. Each page specifies that backend's spawn argv, auto-approval
posture, model-flag format, version requirements, and manual verification
recipe:

- [Claude members](claude.md) — Claude Code (`claude`), the only backend
  that sets the pane title to the member name.
- [Codex members](codex.md) — OpenAI Codex CLI (`codex`), the only backend
  with a kernel-enforced sandbox.
- [Opencode members](opencode.md) — opencode (`opencode`), bound to the
  `cafleet` agent preset and its deny-list safety floor.

For the backend-neutral concepts — the required, operator-declared backend on
`fleet create`, member-create selection and inheritance via `--coding-agent`,
mixed-backend teams, identity delivery, and the intentional asymmetries — see
[Coding agents](../../concepts/coding-agents.md).
