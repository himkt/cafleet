# Consolidate README.md into the docs/ site

**Status**: Approved
**Progress**: 8/12 tasks complete
**Last Updated**: 2026-07-10

## Overview

README.md duplicates almost every descriptive section of the docs/ site, so each change must land twice. This design reduces README.md to a thin entry point (pitch, install, documentation links, license), gives every README-only fact a single home under docs/, and adds operator documentation for pre-trusting a codex member's working directory.

## Success Criteria

- [ ] README.md contains only: title + demo video + pitch, Install, Documentation links, License — the Features, Architecture, Quick Start walkthrough, CLI Usage, API Overview, Tech Stack, Project Structure, and Development sections are gone.
- [ ] Every README-only fact has a docs/ home per the content-mapping table (Tech Stack in `docs/get-started/contributing.md`; Prerequisites in `docs/get-started/install.md`; Features facts folded into `docs/index.md`; the tmux-or-herdr fact merged at the four edit points in `docs/concepts/overview.md` and `docs/index.md`).
- [ ] `docs/get-started/configure.md` § Codex documents pre-trusting the workspace via a `[projects."<abs path>"] trust_level = "trusted"` entry, and `docs/reference/coding-agents/codex.md` links to it.
- [ ] `.claude/skills/update-readme/SKILL.md` prescribes the thin README structure (SPEC.md maintenance rules unchanged).
- [ ] `.claude/rules/documentation-maintenance.md` names docs/ as the primary content home; README updates are required only when the thin surface (pitch / install / links) changes.
- [ ] SPEC.md is untouched.
- [ ] `mise //:docs-build` passes and no repository file links to a removed README section.

---

## Background

README.md carries nine sections; sections 1–8 largely restate content that `docs/index.md`, `docs/get-started/`, `docs/concepts/overview.md`, and `docs/spec/` already own — the exceptions, §6 Tech Stack and the §3 Prerequisites facts, receive docs homes in this design — and the project's own docs style rule (`docs/get-started/contributing.md` § Documentation style) prescribes SSOT: one fact, one home.

Separately, a codex member spawned via `cafleet member create --coding-agent codex` in a directory codex has not yet trusted (no `[projects."<path>"]` entry in `~/.codex/config.toml`) stalls on codex's first-run trust prompt and ignores every incoming message until the operator clears the prompt — on every fresh clone or new git worktree. GitHub issue #154 proposes a code-side spawn-time `-c` override; that code change is out of scope here. This design adds the operator-facing counterpart: document pre-trusting the workspace.

---

## Specification

### Thin README structure

README.md keeps exactly four surfaces, in order:

````markdown
# CAFleet

<demo video (the existing user-attachments asset)>

<one-paragraph pitch, aligned with the docs/index.md description>

## Install

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # migrate the database schema + install the skills
```

Full guide: <https://himkt.github.io/cafleet/get-started/install/>

## Documentation

- [Get Started](https://himkt.github.io/cafleet/get-started/) — install, configure, quickstart, contributing.
- [How-to guides](https://himkt.github.io/cafleet/how-to/) — prompt-first task guides.
- [Concepts](https://himkt.github.io/cafleet/concepts/overview/) — architecture and the ideas behind it.
- [Specification](https://himkt.github.io/cafleet/spec/data-model/) — data model, message envelope, CLI, WebUI API, coding-agent backends.
- [API Reference](https://himkt.github.io/cafleet/api/broker/) — Python API generated from source.

## License

MIT
````

The pitch paragraph is the same register as `docs/index.md`'s opening (multi-backend agent teams, message broker + agent registry, SQLite, full transparency); it may keep one or two sentences of flavor, but no bullet list.

### Content mapping

| README section | Already in docs/? | Action |
|---|---|---|
| §1 Features | Mostly (concepts pages) | Fold the capability list into `docs/index.md` as a short linked bullet list (see below); delete from README. |
| §2 Architecture (ASCII diagram + key design decisions) | Yes — `docs/concepts/overview.md` has the mermaid diagram and decision prose | Verify each README decision bullet against overview.md; the one missing fact — members are bound to panes on **tmux or herdr**, not tmux only — is merged at the four edit points listed below the table; delete from README. |
| §3 Quick Start (prerequisites, install, basic usage, try-with-an-agent) | Mostly — install / usage / try-with-an-agent covered by `docs/get-started/install.md`, `configure.md`, `quickstart.md`; the Prerequisites facts (Python 3.12+, at least one backend binary) are README-only | Move the Prerequisites facts to a short `## Prerequisites` note at the top of `docs/get-started/install.md` — Python 3.12+; a terminal multiplexer, tmux or herdr (link [Multiplexer backends](../concepts/multiplexer-backends.md)); at least one of `claude` / `codex` / `opencode`. Delete the rest; README keeps only the two-command install block. |
| §4 CLI Usage (command tables) | Yes — `docs/spec/cli-options.md`, `docs/concepts/overview.md` § CLI | Delete. |
| §5 API Overview (REST table) | Yes — `docs/spec/webui-api.md` | Delete. |
| §6 Tech Stack | No | Move to `docs/get-started/contributing.md` as a new `## Tech stack` section adjacent to Project structure. |
| §7 Project Structure | Yes — `docs/get-started/contributing.md` § Project structure | Delete. |
| §8 Development | Yes — `docs/get-started/contributing.md` § Development | Delete. |
| §9 License | — | Stays in README. |

The §2 tmux-or-herdr merge covers exactly four edit points — no other stale-phrasing sweeps are in scope:

1. `docs/concepts/overview.md` Core terms row **member**: "bound to a tmux pane" → "bound to a multiplexer pane (tmux or herdr)".
2. `docs/concepts/overview.md` Core terms row **placement**: "tmux session/window/pane" → "multiplexer session/window/pane".
3. `docs/concepts/overview.md` mermaid subgraph label `tmux` → `tmux / herdr`.
4. `docs/index.md` pitch closing "…coding teams in tmux" → "…coding teams in tmux or herdr".

`docs/index.md` gains a compact capability list under the existing pitch. The final bullet list is exactly these five (the README §1 features whose facts the index pitch does not already state), each a one-liner linking to its owning page rather than restating detail:

- Persistent, auditable messages → [Storage](concepts/storage.md)
- Pluggable multiplexer backends (tmux / herdr) → [Multiplexer backends](concepts/multiplexer-backends.md)
- Push notifications → [tmux push notifications](concepts/tmux-push.md)
- Monitoring member → [Monitoring](concepts/monitoring.md)
- Design-doc-driven development (SDD skills) → [Design-doc-driven development](how-to/design-doc-development.md)

Multi-backend teams, the single-file broker, fleet isolation, no-HTTP-server access, and the unified CLI are already stated in the index pitch and are not repeated as bullets.

### Codex workspace pre-trust documentation

Primary home: `docs/get-started/configure.md` § Codex — the get-started page operators land on before spawning members. Add a subsection (e.g. `### Trust the working directory`) that:

1. States the requirement affirmatively: before spawning codex members, add a trust entry for the workspace to `~/.codex/config.toml`:

   ```toml
   [projects."/abs/path/to/workspace"]
   trust_level = "trusted"
   ```

   The path is the absolute working directory the member panes run in (one entry per workspace — including each git worktree, since codex trusts paths, not repositories).
2. States the failure mode it prevents: in an untrusted directory, codex's first-run trust prompt stalls a freshly spawned member — the member ignores every incoming message until the prompt is cleared.
3. Documents prevention only — no recovery-path walkthrough.

`docs/reference/coding-agents/codex.md` gains a one-line pointer to that configure.md subsection (placed with the other spawn preconditions, near the `writable_roots` admonition), per the SSOT rule — the reference page links, it does not restate the snippet.

Out of scope: the agent-facing overlay `skills/cafleet/reference/coding-agent/codex.md` (per `.claude/rules/coding-agent-overlay.md`, operator docs and agent overlays never cross-link; this fact is operator-facing), and issue #154's code-side `-c` spawn override. The docs describe the current, config-file-based procedure and do not reference the issue.

### Meta-documentation updates

`.claude/skills/update-readme/SKILL.md` — rewrite the README half end-to-end so the skill describes only the thin surface:

- **README Structure**: replace the 10-section list with the thin structure (Title + video + pitch / Install / Documentation links / License).
- **Workflow**: keep steps 1–2 (the full `docs/` reads) — they remain the SPEC half's inputs. Replace steps 3–4 (the README read/update) with the thin-surface sync job: read the `zensical.toml` nav, then align the README pitch with `docs/index.md`, the install block with `docs/get-started/install.md`, and the Documentation links with the nav.
- **Rules**: delete the fat-README lines — "Server start: `mise //cafleet:dev`", "Install: `pip install cafleet`", "Preserve any manual additions in README.md", and "If a section has no changes from the source materials, keep it as-is". The language/tone rules (English, no emojis, concise) stay.

The "SPEC.md Maintenance" rules and the SPEC workflow steps (5–6) are unchanged.

`.claude/rules/documentation-maintenance.md` — reword the README paragraph: docs/ is the primary home for all descriptive content; README.md is a thin entry point that must be updated only when its own surface changes (pitch, install commands, docs-site section links). The blocker language narrows accordingly (drift on the thin surface is still a blocker). SPEC.md and SKILL.md remain first-class exactly as written today.

SPEC.md itself is untouched by this design — no contract surface (CLI, config, schema, API) changes.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Move README-only content into docs/

- [x] Add the five-bullet capability list from the Specification to `docs/index.md` <!-- completed: 2026-07-10T20:39 -->
- [x] Add the `## Prerequisites` note (Python 3.12+, tmux or herdr, at least one backend binary) to `docs/get-started/install.md` <!-- completed: 2026-07-10T20:39 -->
- [x] Merge the tmux-or-herdr fact at the four edit points in the Specification (`docs/concepts/overview.md` ×3, `docs/index.md` pitch) <!-- completed: 2026-07-10T20:39 -->
- [x] Add a `## Tech stack` section to `docs/get-started/contributing.md` <!-- completed: 2026-07-10T20:39 -->

### Step 2: Document codex workspace pre-trust

- [x] Add the `### Trust the working directory` subsection to `docs/get-started/configure.md` § Codex (trust snippet + stall failure mode, prevention only) <!-- completed: 2026-07-10T20:42 -->
- [x] Add the one-line pointer from `docs/reference/coding-agents/codex.md` to the configure.md subsection <!-- completed: 2026-07-10T20:42 -->

### Step 3: Thin the README

- [x] Rewrite `README.md` to the thin structure in the Specification <!-- completed: 2026-07-10T20:53 -->
- [x] Sweep the repository for links to removed README sections/anchors and fix them <!-- completed: 2026-07-10T20:53 -->

### Step 4: Update meta-documentation

- [ ] Rewrite the README half of `.claude/skills/update-readme/SKILL.md` (structure spec, workflow steps 3–4, fat-README rules) per the Specification <!-- completed: -->
- [ ] Reword the README paragraph in `.claude/rules/documentation-maintenance.md` <!-- completed: -->

### Step 5: Verify

- [ ] `mise //:docs-build` passes <!-- completed: -->
- [ ] Confirm `SPEC.md` has no diff (`git status`) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-10 | Initial draft |
| 2026-07-10 | Review round 1: Prerequisites moved to install.md; four tmux-or-herdr edit points enumerated; final index capability list fixed at five bullets; update-readme skill rewrite scoped end-to-end; README skeleton fence fixed |
