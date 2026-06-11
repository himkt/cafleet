# API Reference

The Python API matters to two audiences: **contributors** changing cafleet
itself, and **embedders** driving the broker from Python instead of the CLI.
CLI users never need it — every operation on these pages has a CLI
equivalent in [CLI options](../spec/cli-options.md).

One page per module:

- [broker](broker.md) — all agent, fleet, and message operations.
- [config](config.md) — settings and their environment variables.
- [coding_agent](coding-agent.md) — the coding-agent backend abstraction.
- [multiplexer](multiplexer.md) — the tmux abstraction.

The reference is generated from the source docstrings via mkdocstrings.
