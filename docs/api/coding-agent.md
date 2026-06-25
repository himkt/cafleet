---
icon: lucide/code
---

# coding_agent

The backend abstraction behind `--coding-agent`: the interface each spawned
binary (`claude`, `codex`, `opencode`) implements to add or change a backend
— spawn argv, model validation, availability checks.

`build_spawn_argv` receives the spawn prompt **verbatim** — there is no brace
`{placeholder}` mini-language. Identity reaches the spawned pane as environment
variables injected at window-split time (`CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID`,
`CAFLEET_DIRECTOR_AGENT_ID`, alongside `CAFLEET_DATABASE_URL`); see
[Coding agents](../concepts/coding-agents.md) for the read-then-pass convention.

::: cafleet.coding_agent.base
