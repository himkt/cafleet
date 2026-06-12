---
icon: lucide/code
---

# coding_agent

The backend abstraction behind `--coding-agent`: the interface each spawned
binary (`claude`, `codex`, `opencode`) implements — spawn argv, model
validation, availability checks. Read this page to add or change a backend.
Like every API page, it is for contributors changing cafleet and embedders
driving it from Python; CLI users find the command surface in
[CLI options](../spec/cli-options.md).

::: cafleet.coding_agent.base
