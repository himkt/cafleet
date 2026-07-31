# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

Agent Teams reinvented for collaborative coding across multiple coding-agent backends (Claude Code, Codex, and OpenCode), with full code transparency. CAFleet is a message broker and member registry for coding agents, exposing a unified `cafleet` CLI and an admin WebUI over a single-file SQLite database. Fleets partition members into isolated namespaces and the CLI accesses SQLite directly through a shared `broker` module, so no HTTP server is required, for developers and operators running auditable multi-agent coding teams in tmux or herdr.

## Install

Install the CLI with Homebrew, then run the setup:

```bash
brew install himkt/tap/cafleet
cafleet setup
```

Alternatively, download the archive for your platform from [GitHub Releases](https://github.com/himkt/cafleet/releases) — `cafleet-v<version>-<target>.tar.gz` for `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, or `aarch64-unknown-linux-musl` — then extract the single `cafleet` binary onto your `PATH` and run the same setup:

```bash
tar -xzf cafleet-v<version>-<target>.tar.gz
mv cafleet ~/.local/bin/
cafleet setup
```

Full guide: <https://himkt.github.io/cafleet/quickstart>

## Documentation

- [Quickstart](https://himkt.github.io/cafleet/quickstart) — install, configure, and run your first fleet.
- [How-to guides](https://himkt.github.io/cafleet/how-to/mixed-backend-team) — prompt-first task guides.
- [Concepts](https://himkt.github.io/cafleet/concepts/overview) — architecture and the ideas behind it.
- [Specification](https://himkt.github.io/cafleet/spec/data-model) — data model, message envelope, CLI, multiplexer backends, agent backends, WebUI API.
- [Contributing](https://himkt.github.io/cafleet/contributing) — project layout, local development loop, and the contribution flow.

## License

MIT
