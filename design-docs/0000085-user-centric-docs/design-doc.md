# User-Centric Documentation Refactor

**Status**: Approved
**Progress**: 14/14 tasks complete
**Last Updated**: 2026-06-12

## Overview

Refactor the deployed docs (https://himkt.github.io/cafleet/, built from `docs/` + `zensical.toml`) around how users actually drive CAFleet — through a coding agent, not by typing CLI commands. Three changes: remove the Troubleshooting page, lead each how-to guide with example prompts (CLI demoted to an appendix), and remove the API Reference index page.

## Success Criteria

- [x] `docs/get-started/troubleshooting.md` is deleted and `grep -rn "troubleshooting" docs/ README.md zensical.toml` returns nothing (case-insensitive).
- [x] `docs/api/index.md` is deleted; no nav entry, no `(index.md)` link in `docs/api/*.md`, and `docs/index.md` links a concrete API page.
- [x] `mixed-backend-team.md` and `monitor-and-recover.md` each lead with a natural-language prompt block plus a one-line note naming the skills the agent loads, and end with `## Appendix: the CLI underneath` as their final section — no command blocks appear before the appendix.
- [x] `design-doc-development.md` gains a `## Prompts` section containing the three stage prompts, each followed by a one-line skill note.
- [x] `docs/how-to/index.md` frames the section as prompt-first and identifies the WebUI guide as the one human-operated exception; `use-the-webui.md` itself is unchanged.
- [x] `mise //:docs-build` (Zensical clean rebuild) completes without errors or broken-nav warnings.

---

## Background

Users interact with CAFleet by prompting a coding agent that has the CAFleet skills installed; the agent runs the `cafleet` CLI internally. Today's `docs/how-to/` guides are copy-paste CLI walkthroughs aimed at a human typing commands, the Troubleshooting page mixes user content with contributor-only commands (`mise //admin:build`), and the API Reference carries an index page although the sibling Specification section has none.

This design assumes design doc `0000084-src-package-reorganization` is fully implemented: module paths are `cafleet/src/cafleet/{broker/,cli/,output/,webui/}`, the uvicorn target is `cafleet.webui.app:app`, and WebUI assets live in `webui/dist/`. No page edited here contradicts that state.

Decisions confirmed with the user:

| Question | Decision |
|---|---|
| README scope | Link sweep + wording only; no prompt-first restyle of README §2. |
| Troubleshooting inbound links | Drop with **no replacement** — do not repoint to the error-messages table. |
| `cafleet doctor` blurb | Not rehomed; existing coverage in `docs/spec/cli-options.md` is sufficient. |
| `use-the-webui.md` | Kept as-is; the how-to index acknowledges it as the one human-operated guide. |
| Prompt style | Natural-language task prompts + a one-line note listing the skills the agent will load. |
| Appendix depth | Full command walkthrough retained, but trimmed — the prompt section is the priority. |
| API index fold-in | `docs/index.md` links `api/broker.md`; each module page gets a one-sentence audience line replacing the dead landing-page sentence; the mkdocstrings note is dropped. |
| Acceptance | Clean `mise //:docs-build` + repo-wide grep sweep; no deployed-site check. |

---

## Specification

### Part 1 — Remove the Troubleshooting page

Delete `docs/get-started/troubleshooting.md`. Nothing is rehomed: the symptom→fix table largely duplicates the canonical [Error Messages] table in `docs/spec/cli-options.md#error-messages`, and `cafleet doctor` is already documented in `docs/spec/cli-options.md`. Per the removal rule, no deprecation notice, pointer, or "moved to" text is left anywhere.

Reference sweep (every known inbound mention):

| Location | Edit |
|---|---|
| `zensical.toml` nav (line 14) | Delete `{ "Troubleshooting" = "get-started/troubleshooting.md" },`. |
| `docs/index.md:29` | Delete the entire `- [Troubleshooting](get-started/troubleshooting.md) — …` bullet. No replacement bullet. |
| `docs/get-started/index.md:18–19` | Delete the entire `- [Troubleshooting](troubleshooting.md) — …` bullet. |
| `docs/get-started/quickstart.md:145–146` | Delete the entire `- [Troubleshooting](troubleshooting.md) — …` bullet from "Where to go next" (the CLI-options and How-to bullets remain). |
| `README.md:53` | Drop the trailing clause `, and the symptom→fix table at <https://himkt.github.io/cafleet/get-started/troubleshooting/>` (see Part 2 for the same sentence's how-to wording update — apply both as one edit). |

Out of scope: the `mise //admin:build` mention in `use-the-webui.md` stays (the user chose to keep that page as-is), as does the one in `contributing.md` (contributor-facing by design).

### Part 2 — Prompt-first how-to guides

#### Section framing — `docs/how-to/index.md`

Rewrite the body to frame the section as prompt-first. Replacement content (icon front matter unchanged):

```markdown
# How-to guides

You drive CAFleet through your coding agent: each guide below leads with an
example prompt to give the agent, plus the skills the agent loads to act on
it. The two team guides collect the CLI commands the agent runs internally
in a closing appendix, with the full flag surface in the
[spec pages](../spec/cli-options.md).

- [Run a mixed-backend team](mixed-backend-team.md)
- [Monitor and recover members](monitor-and-recover.md)
- [Use the admin WebUI](use-the-webui.md) — the one human-operated guide:
  the WebUI is a browser dashboard you use directly, not through an agent.
- [Design-doc-driven development](design-doc-development.md)
```

#### Common structure for the three prompt-led guides

Each refactored guide follows this skeleton:

1. **Intro** — 1–3 sentences on what the reader accomplishes, with the existing concept links preserved.
2. **`## Prompt`** — one (or two, for guides covering two tasks) fenced `text` block containing a natural-language prompt to paste into the coding agent, followed by a single-line note: *"Your agent loads the `<skill>` … skill(s) to act on this prompt."* Prompts do not name skills inline — they describe the task in natural language so the skill descriptions trigger.
3. **`## What to expect`** — 2–4 sentences describing what the agent does and what the user sees (panes opening, messages flowing), reusing existing concept links (e.g. [tmux push], [Coding agents]).
4. **`## Appendix: the CLI underneath`** — the surviving command walkthrough: the commands + expected-output blocks from today's guide, with connecting prose compressed to one line per command group. Keeps the existing sample-id cast (fleet `1`, Director `2`, members `4`+, per the documentation-style rules in `contributing.md`) and the existing closing link to `../spec/cli-options.md`. The appendix must stay visibly subordinate — no new content, only today's walkthrough condensed.

Guides keep their existing `icon` front matter, titles, and nav entries; only body content changes.

#### `mixed-backend-team.md`

Prompt block (the user-requested mixed-backend example):

```text
Create a CAFleet team for this repo with three members — one on claude,
one on codex, and one on opencode. Name them alice, bob, and carol.
Once they are up, send each member a message asking it to report its
backend, and confirm all three reply. Then tear the team down.
```

Skill note: the agent loads the `cafleet` skill plus `cafleet-agent-team-supervision` (which loads `cafleet-agent-team-monitoring`) before spawning members.

Appendix: today's five command groups (fleet create → three member creates → member list → message send → teardown) with their output blocks; the per-command explanatory paragraphs (e.g. why `--coding-agent` is operator-declared, the pane-title asymmetry note) are kept but compressed to single sentences with their existing concept links.

Prerequisites section: kept, moved under the intro unchanged (backend binaries on `PATH`, install/configure done, inside tmux) — these constrain the human's environment, not the agent.

#### `monitor-and-recover.md`

Prompt block (the user-requested monitoring/recovery example):

```text
Check on my CAFleet team in fleet 1. List member activity, find any
member that has gone quiet, inspect its pane, and recover it with the
mildest intervention that works — only delete it as a last resort.
```

Skill note: the agent loads the `cafleet` skill plus `cafleet-agent-team-monitoring` / `cafleet-agent-team-supervision` (recovery ladder and idle semantics).

Appendix: today's full ladder — `member list --activity`, `member capture`, then `ping` → `send-input` → `exec` → `delete` with output blocks — keeping the activity-column explanation and the `member delete` failure/`--force` path, compressed as above. The existing concept links ([tmux push], [Bash routing], [CLI options]) are preserved.

#### `design-doc-development.md`

Already skill-driven; add concrete prompts. The numbered three-skill list is replaced by a `## Prompts` section with one fenced prompt per stage:

```text
Create a design doc for <one-line feature description>.
```

```text
Interview me about design-docs/NNNNNNN-<slug>.
```

```text
Implement design-docs/NNNNNNN-<slug>.
```

Each prompt is followed by one line naming the skill it triggers (`cafleet-design-doc-create`, `cafleet-design-doc-interview`, `cafleet-design-doc-execute`) and the team it orchestrates (Director/Drafter/Reviewer; Director + Analyzer; Director/Programmer/Tester). The existing sections (contributor pointer to `contributing.md`, "Where output lands", "Watch the team work", "Invocation syntax") are kept; "Invocation syntax" becomes the guide's appendix-equivalent closing note (this guide has no CLI walkthrough, so it gets no `## Appendix`).

#### Cross-page wording (`README.md:53`, `docs/index.md:28`, `docs/get-started/quickstart.md:144`)

Combined with the Part 1 edit, the README sentence becomes:

```markdown
The full walkthrough with every expected output is the quickstart: <https://himkt.github.io/cafleet/get-started/quickstart/>. Prompt-first task guides live at <https://himkt.github.io/cafleet/how-to/>.
```

`docs/index.md:28` (How-to bullet) is reworded to match: `- [How-to guides](how-to/) — prompt-first task guides: mixed-backend teams, monitoring and recovery, the admin WebUI, design-doc-driven development.`

`docs/get-started/quickstart.md:144` ("Where to go next") is reworded to match: `- [How-to guides](../how-to/index.md) — prompt-first task guides.` (its adjacent Troubleshooting bullet is deleted by Part 1).

### Part 3 — Remove the API Reference index page

Delete `docs/api/index.md`. The mkdocstrings provenance note is dropped entirely. The contributors-vs-embedders framing survives as a one-sentence audience line on each module page.

| Location | Edit |
|---|---|
| `zensical.toml` nav (line 45) | Delete `"api/index.md",` so the API Reference section, like Specification, lists only concrete pages. |
| `docs/index.md:32` | Change the bullet link from `[API Reference](api/)` to `[API Reference](api/broker.md)`; bullet text otherwise unchanged. |
| `docs/api/broker.md:9–10` | Replace `— see the\n[API Reference landing page](index.md) for who needs which module.` with the audience sentence below. |
| `docs/api/config.md:9–10`, `docs/api/coding-agent.md:9–10`, `docs/api/multiplexer.md:9–10` | Same replacement. |

Audience sentence (identical on all four pages, appended as its own sentence after each page's existing "Read this page to …" sentence, replacing the dead clause):

```markdown
Like every API page, it is for contributors changing cafleet and embedders
driving it from Python; CLI users find the command surface in
[CLI options](../spec/cli-options.md).
```

Grammar per page: the existing em-dash clause `— see the [API Reference landing page](index.md) for who needs which module.` is deleted and the sentence it hung off is closed with a period; the audience sentence follows.

### Non-goals

- No restyle of README §2 "See it work" (stays a raw-CLI demo).
- No change to `use-the-webui.md`.
- No rehoming of the `cafleet doctor` blurb or the symptom→fix table.
- No changes under `skills/` or `.claude/rules/` — a repo-wide grep confirmed no skill or rule references the removed pages or the how-to framing.
- No code changes; `mise //:docs-build` is the only build surface touched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Ordering note: this change is documentation-only, so the documentation-first rule collapses to docs → README; no `docs/concepts/` page, skill, or rule is affected.

### Step 1: Remove the Troubleshooting page

- [x] Delete `docs/get-started/troubleshooting.md` (`git rm`) <!-- completed: 2026-06-12T09:02 -->
- [x] Remove the Troubleshooting nav entry from `zensical.toml` <!-- completed: 2026-06-12T09:01 -->
- [x] Delete the Troubleshooting bullets from `docs/index.md`, `docs/get-started/index.md`, and `docs/get-started/quickstart.md` <!-- completed: 2026-06-12T09:01 -->

### Step 2: Remove the API Reference index page

- [x] Delete `docs/api/index.md` (`git rm`) and remove `"api/index.md"` from the `zensical.toml` nav <!-- completed: 2026-06-12T09:04 -->
- [x] Repoint `docs/index.md` API Reference bullet to `api/broker.md` <!-- completed: 2026-06-12T09:04 -->
- [x] Replace the landing-page clause with the audience sentence in `docs/api/broker.md`, `docs/api/config.md`, `docs/api/coding-agent.md`, `docs/api/multiplexer.md` <!-- completed: 2026-06-12T09:04 -->

### Step 3: Prompt-first how-to guides

- [x] Rewrite `docs/how-to/index.md` with the prompt-first framing and the WebUI-exception note <!-- completed: 2026-06-12T09:08 -->
- [x] Refactor `docs/how-to/mixed-backend-team.md` to the prompt-led skeleton (prompt + skill note + what-to-expect + CLI appendix) <!-- completed: 2026-06-12T09:08 -->
- [x] Refactor `docs/how-to/monitor-and-recover.md` to the prompt-led skeleton <!-- completed: 2026-06-12T09:08 -->
- [x] Add the three stage prompts to `docs/how-to/design-doc-development.md` <!-- completed: 2026-06-12T09:08 -->
- [x] Reword the How-to bullets in `docs/index.md` and `docs/get-started/quickstart.md` ("Where to go next") to "prompt-first task guides" <!-- completed: 2026-06-12T09:08 -->

### Step 4: README and verification

- [x] Update `README.md:53` (drop the troubleshooting clause, reword to "Prompt-first task guides") <!-- completed: 2026-06-12T09:10 -->
- [x] Run `mise //:docs-build` and confirm a clean build with no missing-page or nav warnings <!-- completed: 2026-06-12T09:10 -->
- [x] Grep sweep: `troubleshooting` (case-insensitive) and `api/index` / `(index.md)` under `docs/`, `README.md`, `zensical.toml`, `skills/`, `.claude/rules/` return no hits <!-- completed: 2026-06-12T09:10 -->
