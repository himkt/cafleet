# Zensical Documentation Site

**Status**: Complete
**Progress**: 44/47 tasks complete (3 remaining are post-merge / external Pages-config checks)
**Last Updated**: 2026-05-27

## Post-completion follow-ups

- `docs/get-started/authoring.md` was created (Step 3) and then removed during the Copilot-review pass per operator decision ("really needed this page??"). The nav entry, IA-table row, and Step 3 task below describe the original plan; the page is no longer in the tree and not wired into `zensical.toml` nav.
- `/ARCHITECTURE.md` was created as the 3-line redirect stub (Step 7) and then removed per operator decision after PR open. The Success Criteria item describes the original plan; the file is no longer in the tree. All cross-references already pointed at the new Concepts pages (rewritten in Step 7), so the deletion has no broken-link impact.

## Overview

Replace the monolithic `ARCHITECTURE.md` plus scattered `docs/` Markdown with a Zensical-built documentation site as the single canonical documentation surface for CAFleet, and wire mkdocstrings so the Python source tree publishes an API reference alongside the prose docs. The site is built from `docs/`, deployed to GitHub Pages at `https://himkt.github.io/cafleet/`, and verified locally via a new `mise //:docs-build` task.

## Success Criteria

- [x] `mise //:docs-build` (equivalent to `uv run zensical build --clean`) exits 0 with no broken links and no missing-handler warnings.
- [x] The published site contains four top-level sections — *Get Started*, *Concepts*, *Specification*, *API Reference* — wired via explicit `nav` in `zensical.toml`.
- [x] Every section of the legacy `ARCHITECTURE.md` is reachable as a focused Zensical page under *Concepts*; six Mermaid diagrams render in the built site.
- [x] mkdocstrings renders complete API reference pages for `cafleet.broker`, `cafleet.config`, `cafleet.coding_agent.base`, and `cafleet.multiplexer.base` with no empty docstring entries.
- [x] The repo-root `ARCHITECTURE.md` is reduced to a 3-line redirect stub pointing at the published site and the `docs/` source path; every intra-repo link that previously pointed at it (README, skills, rules, design docs) either points at the stub or is updated to point at the new Zensical page.
- [x] The repo-root `CONTRIBUTING.md` is reduced to a 3-line redirect stub pointing at the published *Contributing* page; the file is kept (not deleted) because GitHub special-cases `/CONTRIBUTING.md` as the contributor-link target on the PR-author affordance.
- [x] The GitHub Actions `Documentation` workflow uses `uv sync --group dev` + `uv run zensical build --clean` so the published build matches the version locked in `uv.lock`.

---

## Background

Today CAFleet's documentation lives in three uncoordinated places: the 340-line `ARCHITECTURE.md` at the repo root (the de facto canonical reference), four `docs/spec/*.md` files (`data-model.md`, `message-envelope.md`, `cli-options.md`, `webui-api.md`) cross-linked from `ARCHITECTURE.md`, and two operational `docs/{codex,opencode}-members.md` files. A `zensical new .` scaffold has been committed (`zensical.toml`, scaffolding `docs/index.md` and `docs/markdown.md`, `.github/workflows/docs.yml`, and a build artifact `site/`), but no real content has been ported into Zensical and nothing is published yet. This design closes that gap and makes the Zensical site the single documentation surface.

---

## Specification

### Information architecture

The Zensical site exposes four top-level sections, declared explicitly in `zensical.toml` `nav` (Zensical's implicit nav from `docs_dir` does not produce the desired ordering or grouping):

| Section | Source path | Contents |
|---|---|---|
| (site root) | `docs/index.md` | Site landing page served at `/`. Project pitch paraphrased from `README.md` intro paragraph (lines 1-6, above `## 1. Install`) plus the component-overview Mermaid diagram and a prominent "Get started" call-to-action linking to `get-started/`. Intentionally NOT listed in the nav block — Zensical's home-link in the header reaches it. |
| Get Started | `docs/get-started/index.md` | Section index served at `/get-started/`. With `navigation.indexes` enabled in `zensical.toml`, this is the Material/Zensical-conventional section landing — a one-paragraph intro orienting readers to the five sub-pages (Install, Configure, Quickstart, Contributing, Authoring). |
| Get Started | `docs/get-started/install.md` | Install via `uv tool install cafleet` / `pip install cafleet`, `cafleet db init`, plugin install commands per coding agent. Distilled from `README.md` §1. |
| Get Started | `docs/get-started/configure.md` | Recommended `permissions.allow` blocks for `claude` / `codex` / `opencode` and the `mise //:docs-build` invocation. Distilled from `README.md` §2. |
| Get Started | `docs/get-started/quickstart.md` | One-screen session-create + member-create + message-send walkthrough using literal commands. |
| Get Started | `docs/get-started/contributing.md` | Ported from `/CONTRIBUTING.md`. Project layout, local development loop (`mise //:uv-sync`, `mise //cafleet:{install,lint,format,typecheck,test}`, `mise //admin:{build,dev}`), and the design-doc-driven contribution path. Sibling to the other Get Started entries because the contributor onboarding flow is parallel to the user onboarding flow — same audience tier ("first time touching the repo"), different verb ("contribute" vs "install"). |
| Get Started | `docs/get-started/authoring.md` | Contributor guide on enabled Markdown extensions, admonition syntax, Mermaid syntax, code-block features. Repurposes the `zensical new .` scaffolding placeholder `docs/markdown.md` (the file is renamed and rewritten — the placeholder content is discarded). |
| Concepts | `docs/concepts/overview.md` | Top-of-stack architecture overview + component-overview Mermaid diagram. Distilled from `ARCHITECTURE.md` § *Architecture Diagram*, § *Component Layout*, § *Operation Mapping*, § *Package Structure*, § *Plugin Packaging*, § *Design Document Orchestration Skills*, § *WebUI* (sender-selector, auto-refresh, session-scoped endpoints), and the *CLI Option Sources* sub-section of § *Key Design Decisions* (placed adjacent to *Operation Mapping*; the `docs/spec/cli-options.md` reference page remains the canonical CLI surface — the Concepts blurb is the "why these flags exist, not env vars" rationale). |
| Concepts | `docs/concepts/session-isolation.md` | Distilled from `ARCHITECTURE.md` § *Session Isolation*. Includes the session-create transaction sequence diagram. |
| Concepts | `docs/concepts/storage.md` | Distilled from `ARCHITECTURE.md` § *Storage Layer*. Includes the ER diagram for `agents` / `sessions` / `tasks` / `agent_placements`. |
| Concepts | `docs/concepts/member-lifecycle.md` | Distilled from `ARCHITECTURE.md` § *Member Lifecycle*. Includes the member-lifecycle state diagram. |
| Concepts | `docs/concepts/coding-agents.md` | Distilled from `ARCHITECTURE.md` § *Coding Agents*. Includes the coding-agent backend Protocol resolution diagram. Links to the per-backend operational pages under *Specification*. |
| Concepts | `docs/concepts/bash-routing.md` | Distilled from `ARCHITECTURE.md` § *Bash Routing via Director*. |
| Concepts | `docs/concepts/tmux-push.md` | Distilled from `ARCHITECTURE.md` § *tmux Push Notifications*. Includes the message-send + push-notification sequence diagram. |
| Concepts | `docs/concepts/token-reduction.md` | Distilled from `ARCHITECTURE.md` § *Token Reduction*. |
| Specification | `docs/spec/data-model.md` | Unmodified port of the existing file. Path stays as-is so existing intra-repo links continue to resolve. |
| Specification | `docs/spec/message-envelope.md` | Unmodified port. |
| Specification | `docs/spec/cli-options.md` | Unmodified port. |
| Specification | `docs/spec/webui-api.md` | Unmodified port. |
| Specification | `docs/reference/coding-agents/codex.md` | Moved from `docs/codex-members.md`. |
| Specification | `docs/reference/coding-agents/opencode.md` | Moved from `docs/opencode-members.md`. |
| API Reference | `docs/api/broker.md` | Single `::: cafleet.broker` directive. |
| API Reference | `docs/api/config.md` | Single `::: cafleet.config` directive. |
| API Reference | `docs/api/coding-agent.md` | Single `::: cafleet.coding_agent.base` directive. |
| API Reference | `docs/api/multiplexer.md` | Single `::: cafleet.multiplexer.base` directive. |

The legacy `ARCHITECTURE.md` § *WebUI*, § *Package Structure*, § *Plugin Packaging*, § *Operation Mapping*, § *Key Design Decisions*, § *Design Document Orchestration Skills*, and § *CLI Message Body Truncation* sub-sections fold into the existing eight *Concepts* pages where they thematically belong:

| ARCHITECTURE.md section | Destination Concepts page |
|---|---|
| *Operation Mapping* table | `concepts/overview.md` |
| *Key Design Decisions* / *contextId Convention* | `concepts/storage.md` |
| *Key Design Decisions* / *Task Lifecycle Mapping* | `concepts/storage.md` |
| *Key Design Decisions* / *CLI Option Sources* | `concepts/overview.md` (placed next to *Operation Mapping*) |
| *Design Document Orchestration Skills* table | `concepts/overview.md` (closing reference table) |
| *WebUI* | `concepts/overview.md` |
| *Package Structure* | `concepts/overview.md` |
| *Plugin Packaging* | `concepts/overview.md` |
| *CLI Message Body Truncation* | `concepts/tmux-push.md` |

No content is lost in the port; the Step 4 task list below names every source section explicitly.

### Mermaid diagrams

Six diagrams render in the built site:

| # | Page | Diagram | Source |
|---|---|---|---|
| 1 | `concepts/overview.md` | Component overview: CLI ↔ `broker.py` ↔ SQLite ← `webui_api.py` ← `server.py`; tmux panes containing coding-agent processes. | New, derived from `ARCHITECTURE.md` § *Architecture Diagram*. |
| 2 | `concepts/session-isolation.md` | Sequence diagram for `cafleet session create`: tmux context read → 5-step transaction (INSERT sessions, INSERT director agent, INSERT placement, UPDATE sessions.director_agent_id, INSERT administrator). | New, derived from `ARCHITECTURE.md` § *Session bootstrap (transactional)*. |
| 3 | `concepts/storage.md` | ER diagram: `sessions` 1—N `agents`, `agents` 1—1 `agent_placements` (CASCADE), `agents` 1—N `tasks` (via `from_agent_id` / `to_agent_id` / `context_id`), self-reference on `tasks.origin_task_id`, AND the `agent_placements.director_agent_id` parent-child edge (nullable FK → `agents` RESTRICT, used by `_try_notify_recipient` to resolve the recipient pane and by `cafleet member list` to enumerate Director-owned members). | New, derived from `docs/spec/data-model.md`. |
| 4 | `concepts/member-lifecycle.md` | State diagram: `register` → `spawn pane` → `patch placement.pane_id` → (active) → `/exit + pane-gone` → `deregister`, plus rollback arrows on each failure edge. | New, derived from `ARCHITECTURE.md` § *Member Lifecycle*. |
| 5 | `concepts/tmux-push.md` | Sequence diagram for `message send`: sender → `broker.send_message` → SQLite INSERT → `_try_notify_recipient` → `TmuxMultiplexer.send_inline_preview` → recipient pane displays preview → recipient consumes preview as user-turn input → recipient `message ack`. | New, derived from `ARCHITECTURE.md` § *tmux Push Notifications*. |
| 6 | `concepts/coding-agents.md` | Flow diagram: `cli.member_create(--coding-agent X)` → `CODING_AGENTS[X]` → `.ensure_available()` → `.build_spawn_argv(prompt, display_name)` → `TmuxMultiplexer.split_window(argv)`. Branches for `claude` / `codex` / `opencode` annotated with their distinct flags. | New, derived from `ARCHITECTURE.md` § *Coding Agents*. |

All diagrams use the ` ```mermaid` fenced-code form, which `zensical.toml` already routes through `pymdownx.superfences` with the `mermaid` custom fence (`zensical.toml:340-343` already configures this).

### Cross-link form

Every intra-doc link between Zensical source files uses a **relative path** rooted at the current page's directory — e.g. from `docs/concepts/coding-agents.md`, link to the codex page as `../reference/coding-agents/codex.md`. This is the form Zensical's internal-link rewriter recognizes natively: it rewrites the `.md` suffix to the rendered site URL and validates the target during build. Do not use root-relative paths (`/reference/coding-agents/codex.md`) or absolute site URLs (`https://himkt.github.io/cafleet/reference/coding-agents/codex/`) for in-source links — those skip the rewriter's validation and break if `site_url` changes. Absolute site URLs are reserved for references emitted from outside the Zensical source tree (the `/ARCHITECTURE.md` redirect stub, `README.md`, skill files), where there is no `.md → html` rewriter on the path.

### mkdocstrings configuration

Add `mkdocstrings-python` to the `[dependency-groups].dev` block in `pyproject.toml` (the user already ran `uv add --dev mkdocstrings-python` — the lockfile reflects this). Configure the Python handler in `zensical.toml` so the `::: <module>` directive resolves to the in-tree source under `cafleet/src/`:

```toml
[project.plugins.mkdocstrings]
default_handler = "python"

[project.plugins.mkdocstrings.handlers.python]
paths = ["cafleet/src"]

[project.plugins.mkdocstrings.handlers.python.options]
docstring_style = "google"
show_source = true
show_root_heading = true
show_root_full_path = false
members_order = "source"
separate_signature = true
show_signature_annotations = true
filters = ["!^_"]
```

> **Verify the exact zensical/mkdocstrings TOML key path during implementation.** The Zensical plugin block name is `[project.plugins.mkdocstrings]` per Zensical's MkDocs-compatible plugin schema; if the v0.0.43 release uses a different prefix, adapt the keys but preserve every option value above. The first `mise //:docs-build` invocation surfaces any key-path mismatch as a Zensical config error.

The `paths = ["cafleet/src"]` entry is required because the cafleet package lives at `cafleet/src/cafleet/`, not at the repo root. The `filters = ["!^_"]` entry suppresses private functions (the `_` prefix). The `docstring_style = "google"` value is the project-wide style going forward (see § *Docstring policy*).

### Docstring policy

Google-style is the project convention going forward. The CodingAgent and Multiplexer Protocol classes already use this style; broker.py uses one-line docstrings on most public functions and none on the rest. As part of this design doc, every public (non-underscore-prefixed) function and class in the four C1-selected modules — `cafleet.broker`, `cafleet.config`, `cafleet.coding_agent.base`, `cafleet.multiplexer.base` — gets a Google-style docstring before mkdocstrings is wired, so the first published API reference has no empty entries.

Google-style docstring template (apply to public functions in the C1 set):

```python
def send_message(session_id: str, agent_id: str, to: str, text: str) -> dict:
    """Create a unicast task addressed to ``to`` and best-effort notify it.

    Persists a new ``Task`` row with ``type='unicast'`` and
    ``status_state='input_required'``, then calls
    ``_try_notify_recipient`` to keystroke an inline preview into the
    recipient's tmux pane. Notification failure does not roll back the
    insert — the message remains available via ``poll_tasks``.

    Args:
        session_id: Session UUID; sender and recipient must both belong to it.
        agent_id: Sender's agent UUID.
        to: Recipient's agent UUID.
        text: Message body. Truncation is render-side; the persisted row holds the full string.

    Returns:
        The rendered task dict, with the boolean ``notification_sent`` field
        appended at the top level (not persisted on the row).

    Raises:
        click.ClickException: If the recipient is not in the session, the
            sender is the built-in Administrator, or any other broker-level
            invariant is violated.
    """
```

Existing one-line docstrings (e.g. `list_sessions`, `ack_task`) stay if they accurately describe the function; they are extended with `Args:` / `Returns:` / `Raises:` blocks only when the signature has more than one parameter or a non-trivial error contract. Private helpers (`_`-prefixed) are out of scope for the backfill and remain undocumented — `filters = ["!^_"]` keeps them out of the API reference.

The four modules outside the C1 set — `cafleet.cli`, `cafleet.server`, `cafleet.webui_api`, `cafleet.output`, `cafleet.base_dir`, `cafleet.db.*`, `cafleet.coding_agent.{claude,codex,opencode,opencode_preset}`, `cafleet.multiplexer.tmux` — retain their current mixed docstring coverage. Going forward, new code in those modules uses Google style by convention; opportunistic backfill is welcome but is not part of this design doc.

### Legacy ARCHITECTURE.md disposition

After the port, `/ARCHITECTURE.md` at the repo root is replaced with a 3-line redirect stub:

```markdown
# CAFleet — Architecture

This document has moved. The canonical architecture documentation now lives at
<https://himkt.github.io/cafleet/concepts/overview/> (source: `docs/concepts/`).
```

The stub is intentionally short and is NOT a "deprecation notice" of the kind the project's removal rule prohibits — it is a redirect placeholder for the many cross-references in `README.md`, `CLAUDE.md`, skills, and design docs that still point at `ARCHITECTURE.md`. Updating every cross-reference is in scope for this design doc (see Step 7 below).

`README.md` cross-references to `ARCHITECTURE.md` § anchors are rewritten to point at the corresponding `https://himkt.github.io/cafleet/concepts/<page>/` URL (or, for in-repo readers, the `docs/concepts/<page>.md` source). `skills/*/SKILL.md` and `.claude/rules/*.md` cross-references receive the same rewrite. The Step 7 task list enumerates every file that needs updating, sourced from `grep -rln ARCHITECTURE.md` at design-doc-execute time.

### Legacy CONTRIBUTING.md disposition

`/CONTRIBUTING.md` at the repo root is replaced with a 3-line redirect stub (analogous to the ARCHITECTURE.md disposition), but the file is **kept** rather than deleted:

```markdown
# Contributing to CAFleet

The contributor guide has moved. See <https://himkt.github.io/cafleet/get-started/contributing/>
(source: `docs/get-started/contributing.md`).
```

The file MUST remain at `/CONTRIBUTING.md` because GitHub special-cases that path on the repository UI — it surfaces a "Contributing guidelines" link on the issue/PR authoring affordances and the community-standards page. Deleting the file would silently break that affordance even though the content has moved. The 3-line stub satisfies both GitHub's "file exists at expected path" check and the project's removal rule (the stub is a redirect placeholder, not a deprecation notice; the historical content lives in `docs/get-started/contributing.md` and the git history).

### zensical.toml changes

The existing `zensical.toml` requires four edits beyond adding the mkdocstrings handler block:

| Field | Change | Rationale |
|---|---|---|
| `site_name` | `"Documentation"` → `"CAFleet"` | Browser title and header carry the project name. |
| `site_description` | scaffolding default → `"Message broker and agent registry for coding agents."` | HTML head + search engine surfacing. |
| `site_author` | `"<your name here>"` → `"himkt"` | HTML head element. |
| `site_url` | commented-out → `"https://himkt.github.io/cafleet/"` | Required for internal-link rewriting and sitemap generation. Without this, mkdocstrings deep links can be wrong. |
| `nav` | commented-out → explicit 4-section block per the IA table above | Implicit nav from `docs_dir` does not produce the desired grouping or ordering. |
| `copyright` | scaffolding `Copyright © 2026 The authors` → `Copyright © 2026 himkt` | Footer attribution. |

The Markdown extension block at the bottom of `zensical.toml` is unchanged — every extension required by the new content (admonitions, Mermaid via `pymdownx.superfences`, code-tabs, footnotes, attr_list) is already enabled.

The explicit `nav` block uses Zensical's TOML form. The first entry inside each multi-page section is the section's `index.md` written as a **bare string** (not as a labeled `{ "..." = "..." }` table); this is the Material for MkDocs convention that `navigation.indexes` consumes to make the section header itself link to the index page. Mixing bare strings and inline tables inside the same TOML list is valid TOML:

```toml
nav = [
    { "Get Started" = [
        "get-started/index.md",
        { "Install" = "get-started/install.md" },
        { "Configure" = "get-started/configure.md" },
        { "Quickstart" = "get-started/quickstart.md" },
        { "Contributing" = "get-started/contributing.md" },
        { "Authoring" = "get-started/authoring.md" },
    ] },
    { "Concepts" = [
        { "Overview" = "concepts/overview.md" },
        { "Session isolation" = "concepts/session-isolation.md" },
        { "Storage" = "concepts/storage.md" },
        { "Member lifecycle" = "concepts/member-lifecycle.md" },
        { "Coding agents" = "concepts/coding-agents.md" },
        { "Bash routing" = "concepts/bash-routing.md" },
        { "tmux push notifications" = "concepts/tmux-push.md" },
        { "Token reduction" = "concepts/token-reduction.md" },
    ] },
    { "Specification" = [
        { "Data model" = "spec/data-model.md" },
        { "Message envelope" = "spec/message-envelope.md" },
        { "CLI options" = "spec/cli-options.md" },
        { "WebUI API" = "spec/webui-api.md" },
        { "Coding agents" = [
            { "Codex members" = "reference/coding-agents/codex.md" },
            { "Opencode members" = "reference/coding-agents/opencode.md" },
        ] },
    ] },
    { "API Reference" = [
        { "broker" = "api/broker.md" },
        { "config" = "api/config.md" },
        { "coding_agent" = "api/coding-agent.md" },
        { "multiplexer" = "api/multiplexer.md" },
    ] },
]
```

> **Why the operator-facing per-backend pages live under *Specification* (not their own top-level *Reference* section).** The user's binding IA decision is four top-level sections. The per-backend operational guides (`reference/coding-agents/{codex,opencode}.md`) are nested as a "Coding agents" sub-group inside *Specification* so the visual grouping matches their content shape (per-backend install / verify recipes), without breaking the 4-section nav. The source path stays `docs/reference/coding-agents/` because that is the user's explicit B2 choice; the nav label drift is contained inside one sub-group rather than mixing with the four `spec/*.md` siblings.

### Build pipeline

Add a `mise //:docs-build` task to `mise.toml` (the project-root task file) that runs `uv run zensical build --clean`. Wire it into the lint/CI flow so doc breakage surfaces alongside Python lint failures. The exact `mise.toml` block:

```toml
[tasks."docs-build"]
description = "Build the Zensical documentation site (clean rebuild)"
run = "uv run zensical build --clean"
```

> **`uv run` vs `uv tool run`.** `uv run` uses the workspace's dependency-group resolution and pulls `zensical` from the locked version in `uv.lock` (where `uv add --dev zensical` placed it). This is what we want; do NOT switch to `uv tool run zensical`, which would resolve from PyPI ignoring the lock.

Update `.github/workflows/docs.yml` so the published build uses the locked version AND the workflow runs on pull requests for pre-merge validation without publishing. The workflow becomes a two-job pipeline:

```yaml
on:
  push:
    branches: [main, master]
  pull_request:
    branches: [main]

permissions:
  contents: read
  pages: write
  id-token: write

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-python@v6
        with:
          python-version: 3.12
      - uses: astral-sh/setup-uv@v3
      - run: uv sync --group dev
      - run: uv run zensical build --clean
      - uses: actions/upload-pages-artifact@v5
        with:
          path: site

  deploy:
    if: github.event_name == 'push'
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/configure-pages@v6
      - uses: actions/deploy-pages@v5
        id: deployment
```

The split is mandatory, not cosmetic. `environment: github-pages` is a job-level declaration in GitHub Actions, and any job carrying it binds to the Pages environment regardless of which steps run inside it. A single-job workflow gated only at the step level would still run the job under `environment: github-pages` on PR triggers, which is rejected when the environment has branch-protection rules (PR branches do not satisfy `main`-only secrets) and is misleading otherwise (every PR shows up as a "deployment" that never happened). Splitting `build` (runs on push + PR, no environment) from `deploy` (runs on push only, owns the environment + `deploy-pages`) keeps PR runs clean and validates the docs build pre-merge.

Python is pinned to 3.12 to match `requires-python = ">=3.12"` in `pyproject.toml` and the project's own `mise //cafleet:test` runtime. `astral-sh/setup-uv@v3` is the official uv setup action. `uv sync --group dev` installs the locked dev-group dependencies (which now include both `zensical` and `mkdocstrings-python`).

### Out of scope

The following are explicitly NOT covered by this design doc and remain for future work:

- Custom Zensical theme work, custom CSS overrides, palette tuning beyond the existing default `lucide/sun` ↔ `lucide/moon` toggle.
- Publishing to a custom domain. The site stays at the default GitHub Pages URL `https://himkt.github.io/cafleet/`.
- Per-API versioning (e.g. `/v1/api/broker/` vs `/v2/api/broker/`). The published API reference always reflects `main`.
- Migrating the `admin/` frontend's `README.md` and JSDoc into Zensical. The Vite/React SPA has its own documentation surface and is not part of this port.
- Search-result tuning, full-text search index size optimization, offline-mode bundling.
- Backfilling Google-style docstrings on modules outside the C1 set (`cli`, `server`, `webui_api`, `output`, `base_dir`, `db.*`, `coding_agent.{claude,codex,opencode,opencode_preset}`, `multiplexer.tmux`).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Dependencies and project config

- [x] Confirm `pyproject.toml` `[dependency-groups].dev` lists both `zensical>=0.0.43` and `mkdocstrings-python` (the user already ran `uv add --dev` for both — verify `uv.lock` reflects them; re-run `uv sync` if not). <!-- completed: 2026-05-26T10:46 -->
- [x] Update `zensical.toml` `[project]` block: set `site_name = "CAFleet"`, `site_description = "Message broker and agent registry for coding agents."`, `site_author = "himkt"`, uncomment and set `site_url = "https://himkt.github.io/cafleet/"`, rewrite `copyright` to `"Copyright © 2026 himkt"`. <!-- completed: 2026-05-26T10:46 -->
- [x] Add the explicit `nav = [...]` block from § *zensical.toml changes* above. <!-- completed: 2026-05-26T10:46 -->
- [x] Add the `[project.plugins.mkdocstrings]` and `[project.plugins.mkdocstrings.handlers.python]` blocks from § *mkdocstrings configuration*. Do NOT run a build at this point — the nav block references `concepts/*`, `spec/*`, `reference/*`, `api/*` files that Steps 3-6 create, so `zensical build` would fail with ~16 missing-file errors and drown the diagnostic signal we care about. Plugin-key-path verification happens in Step 6 after every referenced file exists. <!-- completed: 2026-05-26T10:46 -->


### Step 2: Backfill Google-style docstrings (BEFORE wiring mkdocstrings pages)

- [x] `cafleet.broker` — backfill Google-style docstrings on every public function (no leading underscore). The function list as of design time: `create_session`, `list_sessions`, `get_session`, `delete_session`, `register_agent`, `get_agent`, `list_agents`, `deregister_agent`, `update_placement_pane_id`, `list_members`, `list_members_with_activity`, `verify_agent_session`, `send_message`, `broadcast_message`, `poll_tasks`, `ack_task`, `cancel_task`, `list_session_agents`, `list_inbox`, `list_sent`, `list_timeline`, `get_agent_names`, `get_task`. Existing one-line docstrings are extended with `Args:` / `Returns:` / `Raises:` blocks where the signature has more than one parameter or a non-trivial error contract. <!-- completed: 2026-05-26T10:55 -->
- [x] `cafleet.config` — add module docstring and Google-style class docstring on `Settings` documenting each field (`database_url`, `broker_host`, `broker_port`, `max_text_len`) including the `validation_alias` env var name and the default. Add a one-line docstring on `_default_database_url` (the only function in the module). <!-- completed: 2026-05-26T10:55 -->
- [x] `cafleet.coding_agent.base` — already has Google-style docstrings on the Protocol; verify completeness, add a module docstring describing the Protocol's role as the registry contract. <!-- completed: 2026-05-26T10:55 -->
- [x] `cafleet.multiplexer.base` — confirm Protocol / dataclass / helper docstrings are Google-style and document every public method (`ensure_available`, `context_discovery`, `split_window`, `kill_pane`, `pane_exists`, `wait_for_pane_gone`, `send_exit`, `send_poll_trigger`, `send_inline_preview`, `send_choice_key`, `send_freetext_and_submit`, `send_bash_command`, `capture_pane`). Document `MultiplexerContext` fields. Add module docstring if missing. <!-- completed: 2026-05-26T10:55 -->
- [x] Run `mise //cafleet:lint` and `mise //cafleet:typecheck` to confirm the docstring backfill introduces no static-analysis regressions. <!-- completed: 2026-05-26T10:55 -->

### Step 3: Reset the docs/ scaffolding

- [x] Replace `docs/index.md` with the site landing page (paraphrased from `README.md` intro paragraph — lines 1-6, above `## 1. Install`: project pitch + component-overview Mermaid diagram + a prominent "Get started" call-to-action linking to `get-started/`). Keep the front-matter `icon: lucide/rocket`. This page is NOT in the nav block — it is served at `/` and reached via the home link in the header. <!-- completed: 2026-05-26T11:00 -->
- [x] Create `docs/get-started/index.md` as the Get Started section index — a one-paragraph intro and a list of the five sub-pages with a one-line each. This file is the section landing per the `navigation.indexes` convention. <!-- completed: 2026-05-26T11:00 -->
- [x] Delete the scaffolding `docs/markdown.md` placeholder and create `docs/get-started/authoring.md` in its place as the contributor authoring guide (admonition syntax, Mermaid syntax, code-block features, code-annotations, footnotes — anchor each section to the Zensical upstream doc URL). <!-- completed: 2026-05-26T11:00 -->
- [x] Create `docs/get-started/install.md`, `docs/get-started/configure.md`, `docs/get-started/quickstart.md` from `README.md` §1 / §2 / §3–§4. <!-- completed: 2026-05-26T11:00 -->
- [x] Create `docs/get-started/contributing.md` by porting `/CONTRIBUTING.md` verbatim into Zensical (preserve every table, code block, and intra-doc link). Rewrite the existing `[design-docs/](design-docs/)` link to point at the in-repo path resolvable from the Zensical source (`../../design-docs/`) and verify the build does not flag the path. <!-- completed: 2026-05-26T11:00 -->

### Step 4: Port ARCHITECTURE.md into Concepts pages

- [x] `docs/concepts/overview.md` — port `ARCHITECTURE.md` § *Architecture Diagram*, § *Component Layout*, § *Operation Mapping*, § *Package Structure*, § *Plugin Packaging*, § *Design Document Orchestration Skills*, and the *CLI Option Sources* sub-section of § *Key Design Decisions* (placed next to *Operation Mapping* so the "session-id / agent-id are literal flags, not env vars" rationale lives with the per-CLI-command surface). Include Mermaid diagram #1 (component overview). <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/overview.md` — additionally port `ARCHITECTURE.md` § *WebUI* into the same page (sender-selector, auto-refresh cadence, session-scoped endpoint contract, the `X-Session-Id` header convention). Keep this as a separate sub-section heading inside `overview.md` so the WebUI narrative is locatable from the page TOC. <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/session-isolation.md` — port `ARCHITECTURE.md` § *Session Isolation* (every sub-bullet: Registration, Isolation rules, Session bootstrap transactional, Session soft-delete, Soft-delete visibility, Root Director protection, Built-in Administrator agent). Include Mermaid diagram #2 (session-create sequence). <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/storage.md` — port `ARCHITECTURE.md` § *Storage Layer* (every sub-section: Backend, Predominantly relational model, Session ownership, Schema management, No physical cleanup) plus § *Key Design Decisions* / *contextId Convention* and / *Task Lifecycle Mapping*. Include Mermaid diagram #3 (ER diagram — verify the rendered diagram includes the `agent_placements.director_agent_id` parent-child edge per the diagram-table description, not just the `tasks.origin_task_id` self-reference). <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/member-lifecycle.md` — port `ARCHITECTURE.md` § *Member Lifecycle*. Include Mermaid diagram #4 (state diagram). <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/coding-agents.md` — port `ARCHITECTURE.md` § *Coding Agents*. Cross-link to the per-backend operational pages using the relative-path form per § *Cross-link form*: `../reference/coding-agents/codex.md` and `../reference/coding-agents/opencode.md` (these target paths match the IA table, the nav block, and the Step 5 `git mv` targets). Include Mermaid diagram #6 (backend resolution flow). <!-- completed: 2026-05-26T11:08 -->

- [x] `docs/concepts/bash-routing.md` — port `ARCHITECTURE.md` § *Bash Routing via Director*. <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/tmux-push.md` — port `ARCHITECTURE.md` § *tmux Push Notifications* plus § *CLI Message Body Truncation*. Include Mermaid diagram #5 (message-send + push-notification sequence). <!-- completed: 2026-05-26T11:08 -->
- [x] `docs/concepts/token-reduction.md` — port `ARCHITECTURE.md` § *Token Reduction*. Convert the Surface-N table to a Zensical-rendered Markdown table with the same columns. <!-- completed: 2026-05-26T11:08 -->
- [x] Verify every internal cross-link inside the new Concepts pages resolves and conforms to § *Cross-link form* (relative path rooted at the current page's directory; no `[`...`](../ARCHITECTURE.md#...)` style links should remain — rewrite to `<sibling>.md#<anchor>` for same-section targets or `../<section>/<page>.md#<anchor>` for cross-section targets). Root-relative and absolute-site-URL forms inside Zensical sources are not allowed. <!-- completed: 2026-05-26T11:08 -->

### Step 5: Move per-backend member docs

- [x] `git mv docs/codex-members.md docs/reference/coding-agents/codex.md` and update the page's intra-doc cross-link (`[ARCHITECTURE.md](../ARCHITECTURE.md)`) to point at `../../concepts/coding-agents.md`. <!-- completed: 2026-05-26T11:10 -->
- [x] `git mv docs/opencode-members.md docs/reference/coding-agents/opencode.md` and apply the same cross-link rewrite. <!-- completed: 2026-05-26T11:10 -->
- [x] `docs/spec/*.md` files stay at their current paths; no moves needed. Verify each spec page's intra-doc cross-links to `ARCHITECTURE.md` or `codex-members.md` are rewritten to the new Concepts pages or to `reference/coding-agents/`. <!-- completed: 2026-05-26T11:10 -->

### Step 6: Wire API reference pages

- [x] Create `docs/api/broker.md`, `docs/api/config.md`, `docs/api/coding-agent.md`, `docs/api/multiplexer.md`. Each file is a one-line H1 plus the appropriate mkdocstrings directive: <!-- completed: 2026-05-26T11:14 -->

  ```markdown
  # broker

  ::: cafleet.broker
  ```

- [x] Run `mise //:docs-build` and confirm: (a) every API page renders without "no docstring" warnings (which indicates a missed backfill in Step 2 — fix any flagged functions); (b) no "plugin not configured" / "unknown plugin key" warning surfaces — that signals the `[project.plugins.mkdocstrings]` key path in Step 1 does not match the Zensical v0.0.43 plugin schema, in which case adjust the TOML key path and re-run; (c) the rendered *Get Started* section header in the sidebar links to `/get-started/` (the `get-started/index.md` page) — if the section header is non-clickable, the `navigation.indexes` attachment did not take and the bare-string entry needs another form. <!-- completed: 2026-05-26T11:18 -->

### Step 7: Replace legacy ARCHITECTURE.md and update cross-references

- [x] Replace `/ARCHITECTURE.md` with the 3-line redirect stub from § *Legacy ARCHITECTURE.md disposition*. <!-- completed: 2026-05-26T11:19 -->
- [x] Replace `/CONTRIBUTING.md` with the 3-line redirect stub from § *Legacy CONTRIBUTING.md disposition* (the file is kept — do NOT delete). <!-- completed: 2026-05-26T11:19 -->
- [x] `grep -rln ARCHITECTURE.md` and apply path-only rewrites to every match outside `design-docs/`: rewrite paths to point at the relevant `docs/concepts/<page>.md` (for in-repo readers) or `https://himkt.github.io/cafleet/concepts/<page>/` (for rendered docs / READMEs). Expected hit-set: `README.md`, `.claude/rules/*.md`, `skills/*/SKILL.md`, AND `.claude/skills/*/SKILL.md` (the latter directory holds project-local skill copies including `skill-author` and `update-readme` — easy to miss because `.claude/rules/design-doc-numbering.md` only enumerates `skills/*/SKILL.md`). Exclude `design-docs/*/design-doc.md` from rewrites — those are historical records and the redirect stub keeps their links resolving. <!-- completed: 2026-05-26T11:22 -->
- [x] Substantive rewrite of `.claude/skills/update-readme/SKILL.md` (separate from the path-only sweep above). The skill's prompt body currently treats `ARCHITECTURE.md` as the canonical source for README generation ("Read ARCHITECTURE.md to understand the current architecture", "based on the current content of ARCHITECTURE.md and docs/"). After this design lands, `ARCHITECTURE.md` is a 3-line redirect stub — reading it returns nothing useful. Rewrite the prompt body to consume `docs/concepts/*.md` (Glob the full Concepts set) as the authoritative architecture source, and keep `docs/` (now containing Concepts + Specification + reference pages) as the second source. The `.claude/skills/skill-author/SKILL.md` reference is a passing glossary example that the path-only sweep above handles correctly. <!-- completed: 2026-05-26T11:22 -->

- [x] Run `grep -rln CONTRIBUTING.md` and rewrite cross-references that should bypass the redirect stub. Specifically, `README.md` §6 currently says `[CONTRIBUTING.md](CONTRIBUTING.md)`; that link should be rewritten to `https://himkt.github.io/cafleet/get-started/contributing/` so README readers reach the canonical contributing page directly instead of routing through the stub. The `/CONTRIBUTING.md` stub itself stays (GitHub UI dependency, per § *Legacy CONTRIBUTING.md disposition*); only the cross-references from non-stub files get updated. Same hit-set scope as the ARCHITECTURE.md rewrite (README, `.claude/rules/*`, `skills/*/SKILL.md`, `.claude/skills/*/SKILL.md`; design-docs/ are historical records and stay untouched). <!-- completed: 2026-05-26T11:22 -->

- [x] Rewrite the substantive guideline in `.claude/rules/design-doc-numbering.md` § *Implementation Order*. The current text reads "Update `ARCHITECTURE.md` with the new feature's architecture." After this design lands, the canonical destination for new architectural prose is the relevant `docs/concepts/<page>.md` page — change the guideline to "Update the appropriate `docs/concepts/<page>.md` page (or add a new Concepts page if the feature introduces a new architectural axis)." A path-only swap to `ARCHITECTURE.md → docs/concepts/<page>.md` is insufficient because the rule does not say which page, and pointing every future feature at the redirect stub would silently let Concepts pages rot. <!-- completed: 2026-05-26T11:22 -->
- [x] Update `README.md` to reference the published site as the canonical architecture entry point ("Architecture documentation is published at https://himkt.github.io/cafleet/"). The README's own architectural prose stays, but pointer paragraphs that previously said "see ARCHITECTURE.md §X" become "see the [X page](https://himkt.github.io/cafleet/concepts/X/)". <!-- completed: 2026-05-26T11:20 -->

### Step 8: Build pipeline and CI

- [x] Add the `[tasks."docs-build"]` block from § *Build pipeline* to `mise.toml`. <!-- completed: 2026-05-26T11:15 -->
- [x] Add `/site/` to the repo-root `.gitignore` (leading slash so the pattern matches the repo-root build artifact only, not a future `cafleet/site/` or similar). Without this entry, every contributor running `mise //:docs-build` sees the entire built site as untracked changes and risks staging it. <!-- completed: 2026-05-26T11:25 -->
- [x] Restructure `.github/workflows/docs.yml` into the two-job pipeline documented in § *Build pipeline*: a `build` job (runs on push + PR, no `environment:` declaration, performs `actions/checkout@v6` + `actions/setup-python@v6` with `python-version: 3.12` + `astral-sh/setup-uv@v3` + `uv sync --group dev` + `uv run zensical build --clean` + `actions/upload-pages-artifact@v5`) and a `deploy` job (gated by `if: github.event_name == 'push'`, `needs: build`, declares `environment: github-pages`, performs `actions/configure-pages@v6` + `actions/deploy-pages@v5`). Also extend the `on:` trigger to include `pull_request: branches: [main]` for pre-merge build validation. The two-job split is required (not optional) because `environment: github-pages` is a job-level binding — a single-job + step-level `if:` would still bind every PR run to the Pages environment, which is rejected under branch-protection rules and misleading otherwise. <!-- completed: 2026-05-26T11:25 -->

- [ ] Verify GitHub Pages is enabled on the `himkt/cafleet` repository (Settings → Pages → Source: "GitHub Actions") BEFORE the first push-triggered deploy run in Step 9. `actions/deploy-pages@v5` requires this setting; if Pages is disabled or set to "Deploy from a branch", the first push to `main` after merging this design fails the `deploy-pages` step. Capture as a one-time pre-merge check; no code change needed in the repo if Pages is already enabled. <!-- completed: -->

- [x] Add `mise //:docs-build` to whatever lint/CI orchestration the project uses (verify by inspecting `mise.toml` for any lint-aggregate task or the CI workflow that runs lint/typecheck). <!-- completed: 2026-05-26T11:26 -->

### Step 9: Validate

- [x] Run `mise //:docs-build` locally; exit code 0 with no broken-link warnings, no missing-handler warnings, no Mermaid-rendering errors. <!-- completed: 2026-05-26T11:35 -->
- [x] Open the built `site/index.html` in a browser, click through every nav entry, confirm all six Mermaid diagrams render and every API page populates from mkdocstrings. <!-- completed: 2026-05-26 -->
- [x] Push the branch as a pull request against `main` and confirm the GitHub Actions `Documentation` workflow's PR-build run succeeds (this is the validation enabled by the Step 8 `pull_request:` trigger; the deploy steps are skipped on PR runs via the `if: github.event_name == 'push'` gate). <!-- completed: 2026-05-26 -->
- [ ] After merging to `main`, confirm the same workflow's push-triggered run succeeds end-to-end, including `upload-pages-artifact` and `deploy-pages`. <!-- completed: -->
- [ ] Verify the published site at `https://himkt.github.io/cafleet/` matches the local build. <!-- completed: -->
- [x] Run `mise //cafleet:lint` and `mise //cafleet:typecheck` once more to confirm the docstring backfill from Step 2 still passes after every edit. <!-- completed: 2026-05-26T11:35 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-25 | Initial draft |
