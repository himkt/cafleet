# User request: user-centric documentation refactor

The deployed docs live at https://himkt.github.io/cafleet/ and are built from `docs/` + `zensical.toml` (nav). Three changes are requested. Assume design doc `0000084-src-package-reorganization` is FULLY implemented when writing this doc (module paths are `cafleet/src/cafleet/{broker/,cli/,output/,webui/}`; uvicorn target `cafleet.webui.app:app`; assets in `webui/dist/`).

## 1. Remove the Troubleshooting page

`docs/get-started/troubleshooting.md` is not needed. Rationale (user's words): it contains `mise //admin:build`, which is only available to cafleet developers, not users — the page merely confuses users. Remove the page and sweep EVERY reference per the repo removal rule (`~/work/himkt/config/claude/rules/removal.md` semantics: no deprecation notices left behind). Known references: `docs/index.md:29`, `docs/get-started/index.md:18`, `docs/get-started/quickstart.md:145`, `README.md:53`, `zensical.toml` nav line 14. Decide and specify where (if anywhere) genuinely user-relevant content rehomes — note `docs/spec/cli-options.md#error-messages` already carries the exhaustive error table.

## 2. Refactor how-to guides to be human-centric

Today's `docs/how-to/` guides are copy-paste `cafleet` CLI walkthroughs. But users drive CAFleet through a coding agent, not by typing CLI commands. Refactor the section so each guide leads with **example prompts the user feeds into their coding agent** (prompts that cause the agent to load the relevant skills — e.g. `cafleet`, `cafleet-agent-team-supervision`/`-monitoring`, `cafleet-design-doc-*`). Specifically requested examples: a prompt for launching a mixed-backend agent team, and a prompt for monitoring and recovering members. As an **appendix** in each guide, show the CLI commands that are used internally by the agent. `design-doc-development.md` is currently the closest to the right style (it is skill/prompt-driven) but is still not enough — it also needs concrete example prompts. Rework `docs/how-to/index.md` framing accordingly (it currently advertises "copy-paste walkthroughs" of commands). Decide what happens to `use-the-webui.md` (it is inherently a human-operated GUI guide).

## 3. Remove the API Reference index page

`docs/api/index.md` is not needed — the Specification nav section has no index page, and API Reference should match. Remove it, update `zensical.toml` nav (drop `api/index.md`), fix `docs/index.md:32` (link to a concrete page instead of `api/`), and remove/rework the "[API Reference landing page](index.md)" sentences in `docs/api/broker.md`, `docs/api/config.md`, `docs/api/coding-agent.md`, `docs/api/multiplexer.md`. Any genuinely useful framing from the index (audiences: contributors vs embedders; mkdocstrings note) may be folded into the module pages if it earns its place.

## Constraints

- Follow `.claude/rules/design-doc-numbering.md`: documentation-first implementation ordering (docs → README → skills → rules) and README/SKILL.md as first-class targets.
- The deployed site was verified to match the repo state on 2026-06-11.
