# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

Agent Teams reinvented for collaborative coding across multiple coding-agent backends (Claude Code, Codex, and OpenCode), with full code transparency. CAFleet is a message broker and member registry for coding agents — a unified `cafleet` CLI and an admin WebUI over a single-file SQLite database — for developers and operators running auditable multi-agent coding teams in tmux or herdr.

## Install

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # migrate the database schema + install the assets (skills and presets)
```

Full guide: <https://himkt.github.io/cafleet/quickstart/>

## Documentation

- [Quickstart](https://himkt.github.io/cafleet/quickstart/) — install, configure, and run your first fleet.
- [How-to guides](https://himkt.github.io/cafleet/how-to/mixed-backend-team/) — prompt-first task guides.
- [Concepts](https://himkt.github.io/cafleet/concepts/overview/) — architecture and the ideas behind it.
- [Specification](https://himkt.github.io/cafleet/spec/data-model/) — data model, message envelope, CLI, multiplexer backends, WebUI API, coding-agent backends.
- [Contributing](https://himkt.github.io/cafleet/contributing/) — project layout, local development loop, and the contribution flow.
- [API Reference](https://himkt.github.io/cafleet/api/broker/) — Python API generated from source.

## License

MIT
