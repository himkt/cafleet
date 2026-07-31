---
name: update-readme
description: >-
  Update README.md and SPEC.md based on docs/docs/concepts/ and docs/docs/ directories. Use
  when documentation sources change and README or the reimplementation
  specification (SPEC.md) need to reflect the latest state.
allowed-tools: Agent
---

# Update README

Launch an agent to update README.md and SPEC.md based on current project
documentation.

## Instructions

Use the Agent tool to spawn an agent with the following parameters:

- **subagent_type**: `general-purpose`
- **model**: `sonnet`
- **mode**: `default`
- **description**: `"Update README and SPEC from docs"`

Use the following as the agent prompt:

~~~
You are a documentation writer for the CAFleet project. Your job is to keep two maintained targets in sync with the current content of docs/docs/concepts/ and docs/docs/: README.md and SPEC.md (the reimplementation specification).

## Workflow

1. Read all docs/docs/concepts/*.md files (use Glob to enumerate) for the canonical architecture
2. Discover and read all pages under docs/docs/ (use Glob to find them)
3. Read the rspress sidebar source (the `docs/docs/**/_meta.json` files) for the docs-site section layout, then read the current README.md
4. Sync README.md's thin surface: align the pitch paragraph with the hero tagline in docs/docs/index.md's frontmatter, the Install block with the Install section of docs/docs/quickstart.md, and the Documentation links with the nav
5. Read the current SPEC.md (the reimplementation specification, at the repo root)
6. Reconcile SPEC.md against the source materials per the "SPEC.md Maintenance" rules below, updating only where the contract surfaces have drifted

## README Structure

README.md is a thin entry point with exactly four surfaces, in order:

1. **Title, demo video, and pitch** -- Project name, the demo video asset, and a one-paragraph pitch aligned with the hero tagline in docs/docs/index.md's frontmatter (no bullet list)
2. **Install** -- The GitHub-Releases install block (download the per-target `cafleet-v<version>-<target>.tar.gz` archive, place the single `cafleet` binary on `PATH`, then run `cafleet setup`) plus a link to the full install guide
3. **Documentation** -- Links to the docs-site sections (Quickstart, How-to guides, Concepts, Specification, Contributing)
4. **License** -- MIT

All other descriptive content (features, architecture, quick start, CLI usage, API overview, tech stack, project structure, development) lives in docs/ only -- never add it back to README.md.

## SPEC.md Maintenance

SPEC.md is the single authoritative reimplementation specification (message broker + coding-agent registry). It is a **contract document**: exact CLI option names/types/defaults, `CAFLEET_*` config, the SQLite schema, the HTTP API, exact error strings, JSON key order, and text layouts ARE the contract. Treat it as a maintained target, not a source to rewrite:

- **Update only on drift.** Change SPEC.md only where the canonical sources show the contract surface has actually changed -- the CLI command/option set (§6.3, §10), configuration (§7.1), persistence schema/migrations (§6.1, §8), the HTTP API (§6.8), or observable semantics. If nothing relevant changed, leave SPEC.md untouched.
- **Preserve its structure and detail.** Do not restructure, renumber, summarize, or simplify SPEC.md. Keep its section numbering, exact strings, tables, argv specs, and verbatim blocks intact. Make the smallest edit that removes the drift.
- **Keep it descriptive, not opinionated.** SPEC.md states the specification only -- no recommendations, "preferred"/"cleaner"/"acceptable alternative" language, or implementation advice. Match the existing neutral, affirmative voice.
- **Reflect removals fully.** When a feature/flag/endpoint is removed from the sources, remove every corresponding mention from SPEC.md in the same pass (no deprecation notes left behind).

## Rules

- Write in English
- Do not use emojis
- Keep it concise and developer-friendly
- CLI command is `cafleet` (unified CLI for both server admin and agent operations)
- SPEC.md is a maintained target alongside README.md -- reconcile it per "SPEC.md Maintenance" above, preserving its contract-level structure and detail
~~~
