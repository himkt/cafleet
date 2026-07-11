---
icon: lucide/code
---

# coding_agent

The backend abstraction behind `--coding-agent`: the interface each spawned
binary (`claude`, `codex`, `opencode`) implements to add or change a backend
— spawn argv, model validation, availability checks.

`build_spawn_argv` receives the spawn prompt already rendered: `cafleet member
create` runs `str.format` over the prompt body first, substituting `{fleet_id}`,
`{member_id}`, `{director_member_id}`, and `{coding_agent}` to literals, so the
backend layer never sees a brace placeholder. The only environment variable
injected at window-split time is `CAFLEET_DATABASE_URL`; see
[Coding agents](../concepts/coding-agents.md) for the identity convention.

::: cafleet.coding_agent.base
