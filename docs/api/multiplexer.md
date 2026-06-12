---
icon: lucide/code
---

# multiplexer

The tmux abstraction: pane discovery, window splitting, keystroke delivery,
and capture used by the spawn and push-notification paths. Read this page to
change how cafleet drives tmux. Like every API page, it is for contributors
changing cafleet and embedders driving it from Python; CLI users find the
command surface in [CLI options](../spec/cli-options.md).

::: cafleet.multiplexer.base
