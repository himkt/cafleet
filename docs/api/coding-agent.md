---
icon: lucide/code
---

# coding_agent

The backend abstraction behind `--coding-agent`: the interface each spawned
binary (`claude`, `codex`, `opencode`) implements — spawn argv, model
validation, availability checks. Read this page to add or change a backend —
see the [API Reference landing page](index.md) for who needs which module.

::: cafleet.coding_agent.base
