# CAFleet

<p align="center">
  <img src=".github/assets/cafleet.png" alt="cafleet" width="15%" />
</p>

<p align="center">
  <a href="https://www.himkt.dev/cafleet">himkt.dev/cafleet</a> · <a href="https://www.himkt.dev/cafleet/quickstart#install">install</a> · <a href="https://www.himkt.dev/cafleet/quickstart">quick start</a> · <a href="https://herdr.dev/docs/">docs</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?labelColor=333333" alt="MIT license" /></a>
  <a href="https://github.com/himkt/cafleet/stargazers"><img src="https://img.shields.io/github/stars/himkt/cafleet?labelColor=333333&color=666666&logo=github" alt="GitHub stars" /></a>
  <a href="https://github.com/himkt/cafleet/releases/latest"><img src="https://img.shields.io/github/v/release/himkt/cafleet?label=release&labelColor=333333&color=666666" alt="latest stable release" /></a>
  <a href="https://x.com/himako_h"><img src="https://img.shields.io/badge/follow-%40himako_h-000000?logo=x&logoColor=white" alt="follow @himako_h on X" /></a>
</p>

---

https://github.com/user-attachments/assets/06e8251e-54a8-41bf-b7be-0d752f605b6e

CAFleet multiplexes independent coding-agent sessions in tmux or herdr and coordinates them through durable messages: collaborative coding across multiple coding-agent backends, with full code transparency. Use its CLI and skills to delegate research, implementation and review, then collect durable replies while your session continues the task.

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
- [Members](https://himkt.github.io/cafleet/concepts/members) — member lifecycle, command routing and permission handling.
- [Specification](SPEC.md) — runtime, broker and CLI contracts.
- [Contributing](https://himkt.github.io/cafleet/contributing) — project layout, local development loop, and the contribution flow.

## License

MIT
