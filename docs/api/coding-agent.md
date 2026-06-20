---
icon: lucide/code
---

# coding_agent

The backend abstraction behind `--coding-agent`: the interface each spawned
binary (`claude`, `codex`, `opencode`) implements to add or change a backend
— spawn argv, model validation, availability checks.

::: cafleet.coding_agent.base
