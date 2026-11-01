# CAFleet

https://github.com/user-attachments/assets/bd2b195a-f3de-4fa3-bcc8-3c6ef9f1016a

CAFleet multiplexes independent coding-agent sessions in tmux or herdr and coordinates them through durable messages. Use its CLI and skills for delegated research, implementation and review across coding-agent backends. The host owns continuation; the Director receives durable replies through native inbox tools and inspects visible command results.

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
- [Run a fleet](https://himkt.github.io/cafleet/mixed-backend-team) — prompt-first fleet guide.
- [Concepts](https://himkt.github.io/cafleet/concepts/overview) — architecture and the ideas behind it.
- [Members](https://himkt.github.io/cafleet/concepts/members) — member lifecycle, Director/user decisions, request-bound native responses and continuation.
- [Specification](SPEC.md) — runtime, broker and CLI contracts.
- [Contributing](https://himkt.github.io/cafleet/contributing) — project layout, local development loop, and the contribution flow.

## License

MIT
