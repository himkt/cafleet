---
pageType: home
hero:
  name: CAFleet
  text: Agent Teams reinvented
  tagline: Collaborative coding across multiple coding-agent backends, with full code transparency.
  actions:
    - theme: brand
      text: Quickstart
      link: /quickstart
    - theme: alt
      text: GitHub
      link: https://github.com/himkt/cafleet
---

<iframe
    style="width: 100%; aspect-ratio: 16 / 9; border: 0;"
    src="https://www.youtube.com/embed/cLLp-eoWFBg"
    title="CAFleet demo video"
    allowfullscreen>
</iframe>

CAFleet is a message broker and member registry for coding agents, exposing a
unified `cafleet` CLI and an admin WebUI over a single-file SQLite database.
Fleets partition members into isolated namespaces and the CLI accesses SQLite
directly through a shared `broker` module, so no HTTP server is required; three
coding-agent backends — `claude` (Claude Code), `codex` (OpenAI Codex CLI), and
`opencode` — coexist in the same fleet, for developers and operators running
auditable multi-agent coding teams in tmux or herdr.
