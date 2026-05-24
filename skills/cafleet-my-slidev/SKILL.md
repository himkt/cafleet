---
name: cafleet-my-slidev
description: Create Slidev presentations using the custom theme with cover, bullets, two-cols, blank, stats-grid, section-divider, and end layouts. Use when generating presentations from research reports, outlines, or other content. References /slidev for syntax details.
---

# Custom Slidev Theme Presentation Guide

Theme location: `theme/` inside this skill's directory. For Slidev syntax, refer to /slidev or /slidev:slidev. **NEVER READ FILES DIRECTLY**.

## Headmatter

```yaml
---
theme: <cafleet-plugin-install-dir>/skills/cafleet-my-slidev/theme
# Replace <cafleet-plugin-install-dir> with the absolute path to the installed cafleet plugin's directory on this machine.
# Discovery hints:
#   - Claude Code:  ~/.claude/plugins/cache/cafleet/cafleet/<version>/   (run `claude plugin list` to find <version>)
#   - Codex:        the path printed by `codex plugin list` for the cafleet plugin
# The skill is install-location-agnostic; the absolute path resolves at Slidev render time.
title: <Presentation Title>
author: <Author Name>
fonts:
  sans: Noto Sans JP
  provider: google
---
```

## Layouts

| Layout | When to Use | Pattern |
|--------|-------------|---------|
| `cover` | First slide only | `# Title` + author paragraph |
| `bullets` | Content with header + points | `::header::` `# Title`, `::default::` `- items` |
| `bullets-sm` | References, bibliography | Same as `bullets`, smaller text, no markers |
| `two-cols` | Comparisons, chart+insight | `::header::`, `::left::`, `::right::`. Prop: `columns: "2:1"` |
| `blank` | Tables, figures, free-form | Any content. `class: v-center` for centering |
| `stats-grid` | 2-4 hero numbers as KPI cards | `::header::`, frontmatter `stats: [{value, label, source?, type?}]` |
| `section-divider` | Chapter breaks (every 5-8 slides) | `# Title` + subtitle. Props: `section: N`, `totalSections: N` |
| `end` | Last slide | `# Thank You` + subtitle |

`stats-grid` types: `primary` (default), `accent`, `positive`, `negative`, `important`.

## Self-Review Checklist (mandatory)

After generating all slides, check every slide:

1. **Numbers buried in bullets?** → Replace with `stats-grid`
2. **3+ consecutive `bullets` slides?** → Insert `stats-grid`, `blank`, `two-cols`, or `section-divider`
3. **Comparison (X vs Y)?** → Use `two-cols`
4. **Figure with source text?** → Use `.figure-caption`, not raw `<div>`
5. **Negative data (vulnerabilities, failures)?** → Use `type: "negative"` on stats, semantic colors in charts
6. **All `section-divider` slides have `totalSections`?**
7. **`end` layout as final slide?**
8. **Layout variety**: 20+ slides need 6+ non-bullets slides

## Content Rules

- **One message per slide** — if you can't state it in one sentence, split
- **Max 7 bullets, ~15 words each**
- **No nested layouts**
- **Content must fit** — if it overflows, split or switch layout
- **Bad text wrapping is a critical defect.** The issue is NOT line breaks themselves — long text may need to wrap. The issue is breaking at wrong boundaries: mid-word, mid-unit ("$9-" / "13B"), or leaving meaningless orphan fragments. Wrapping must occur at natural unit boundaries (between words, between logical groups). Do NOT shorten text just to avoid wrapping — that loses information. Instead: use non-breaking characters (U+2011 `‑`, `&nbsp;`) within units that must stay together, adjust `fontSize`, or restructure the layout.

## Color

### Tokens

| Token | Use For |
|-------|---------|
| `--c-primary` (blue) | Key metrics, links |
| `--c-accent` (orange) | Warnings |
| `--c-positive` (green) | Growth, upside |
| `--c-negative` (red) | Decline, risks |
| `--c-important` (purple) | Critical points |

### Application

- `<Highlight type="positive">+99%</Highlight>` — positive emphasis (green)
- `<Highlight>81.2%</Highlight>` — neutral emphasis (blue, default)
- `<Admonition type="tip" title="Key Takeaway">text</Admonition>` — callout box
- `<div class="bg-primary-light">text</div>` — lightweight single-line highlight

### Discipline

- **1-2 colored elements per slide max**
- **Color for data, not decoration** — only color the specific number or keyword
- **Consistent semantics** — green = positive, red = negative throughout

## Figures

1. **No duplicate titles** — slide heading IS the chart title
2. **Caption**: `<div class="figure-caption">Source: [N]</div>` — never raw `<div class="text-sm">`
3. **Colors**: must match the `cafleet-create-figure` skill's palette
4. **Figure-only slide**: `blank` layout with `## Title` + image + caption
5. **Figure + insight**: `two-cols` with `columns: "3:2"`, chart in `::left::`, text in `::right::`

## Tables

Use `blank` layout. Theme auto-styles: blue header, alternating rows.

## Mermaid Diagrams

Use `blank` layout for flows, timelines, relationships.

## Page Numbers

Auto-rendered on `bullets`, `bullets-sm`, `two-cols`, `stats-grid`, `blank`. Not on `cover`, `section-divider`, `end`.

## Bullet Markers

Auto-rendered: top-level = filled blue circle, nested = hollow. `bullets-sm` has no markers.

## Citations

- Renumber sequentially by first appearance (ignore source report numbering)
- Body ↔ References must be two-way consistent, contiguous, no duplicates
- Add References slide(s) at end listing only cited sources

## Layout Examples

### Stats-Grid

```md
---
layout: stats-grid
stats:
  - value: "84%"
    label: "Developer AI tool adoption"
    source: "[1]"
  - value: "41%"
    label: "GitHub code is AI-generated"
    source: "[2]"
---

::header::

# Market at a Glance
```

### Section Divider

```md
---
layout: section-divider
section: 2
totalSections: 5
---

# Security & Privacy

Key risks in AI-generated code
```

### Techniques

| Technique | Reference |
|-----------|-----------|
| Two-column layout | techniques/two-column-layouts.md |
| Admonition boxes | techniques/admonition.md |
| Highlight markers | techniques/highlight.md |
| Code animations | techniques/animations.md |
| Math formulas | techniques/math-formulas.md |
| Font size control | techniques/font-size.md |

## Spawnable Agents

### slide-creator

This skill ships an embedded agent spec for generating a complete Slidev presentation autonomously from input content (research reports, outlines, notes). The spec is reproduced verbatim below so it is reachable from both Claude Code (load the `cafleet-my-slidev` skill then dispatch via `Agent`) and codex (via plugin auto-discovery — see *Dispatching this agent (codex inline-follow)* and *Dispatching this agent (codex member-spawn)* below).

    ---
    name: slide-creator
    description: Generate a complete Slidev presentation from input content (research reports, outlines, notes). Fully autonomous — produces slide.md without asking clarifying questions. Takes a file path or inline content as input.
    color: green
    ---

    You generate Slidev presentations from input content. You work autonomously — do not ask the user clarifying questions.

    ## Required Skills

    Before generating any presentation, load these two skills:

    1. **`cafleet-my-slidev`** — Custom theme with cover, bullets, and blank layouts. Provides layout selection guide, content structuring rules, formatting conventions, and headmatter template.
    2. **`slidev`** — Slidev syntax reference for markdown features, components, animations, and other capabilities.

    ## Input

    You receive input content in one of two ways:

    - **File path**: A path to a markdown file (e.g., a research report). Read the file and transform its content into a presentation.
    - **Inline content**: Content provided directly in the prompt. Parse and structure it into slides.

    ## Generation Workflow

    1. **Read the input content** — Load the file or parse the inline content. Understand the structure, key topics, and overall narrative.
    2. **Identify title and author** — Extract the presentation title and author from content metadata (e.g., document title, author field). If not available, infer a suitable title from the content and use "Author" as the default author name.
    3. **Break content into logical topics** — Each topic becomes one slide. One idea per slide — do not overload slides with multiple concepts.
    4. **Choose the appropriate layout for each slide**:
       - `cover` for the first slide (title + author)
       - `bullets` for most content slides (topic heading + supporting points)
       - `blank` for slides with diagrams, images, code blocks, or free-form content that doesn't fit the bullets pattern
    5. **Apply content structuring rules** from the `my-slidev` skill:
       - Max 5 bullets per slide; split into multiple slides if needed
       - Bullet text max ~15 words; keep concise
       - Use sub-bullets sparingly
       - Use `**bold**` for key terms, max 1-2 per bullet
    6. **Generate the initial slide content**:
       - Start with the headmatter template from the `my-slidev` skill
       - Use the literal `theme:` path documented in the embedding skill's headmatter template — `<cafleet-plugin-install-dir>/skills/cafleet-my-slidev/theme`. Substitute the absolute path to the installed cafleet plugin directory on this machine (Claude Code: `~/.claude/plugins/cache/cafleet/cafleet/<version>/`; Codex: as reported by `codex plugin list`). The path is a fixed, documented location; do NOT try to derive it dynamically (the `cafleet-base-dir` skill resolves a CWD-based working directory, not the install location of the calling skill).
       - First slide is always `cover` layout
       - Content slides follow with appropriate layouts
       - Add presenter notes (`<!-- notes -->`) with expanded talking points from the source content
       - Assemble the full presentation in memory — do NOT write the file yet
    7. **Quality Review** (two-pass, mandatory before writing the file):
       - **Pass 1 — Review and Fix**:
         1. Content overflow check: Walk through every slide and assess whether its content fits on a single slide. Fix any overflow using remediation strategies from the `my-slidev` skill (split, condense, change layout, simplify).
         2. Citation ordering check: Scan all slides from first to last, recording the order of first appearance of each citation. If citations are not in sequential order, renumber them throughout the entire presentation (both body text and References section).
         3. Citation correctness check: Verify body→references and references→body consistency. Fix any mismatches (remove orphan references, add missing references, remove uncited `[N]` markers).
       - **Pass 2 — Verification**: Re-scan the entire presentation to verify all fixes are correct and no new issues were introduced. Confirm: no overflowing slides, sequential citation numbers starting from `[1]`, body↔references consistency, and no broken formatting (slide separators, layout frontmatter).
       - Only after Pass 2 confirms zero issues, write the file to the current working directory as `slide.md`.

    ## Output Constraints

    - Always start with a `cover` slide
    - Use `bullets` layout by default for content slides
    - Add presenter notes with expanded talking points from the source content
    - Use the literal `theme:` path documented in the embedding skill's headmatter template — `<cafleet-plugin-install-dir>/skills/cafleet-my-slidev/theme`. Substitute the absolute path to the installed cafleet plugin directory on this machine (Claude Code: `~/.claude/plugins/cache/cafleet/cafleet/<version>/`; Codex: as reported by `codex plugin list`). The path is a fixed, documented location; do NOT try to derive it dynamically (the `cafleet-base-dir` skill resolves a CWD-based working directory, not the install location of the calling skill).
    - All slides must pass content overflow review — no slide may have content that exceeds its viewport
    - Citations must be numbered sequentially by order of first appearance
    - Every citation in body must have a reference; every reference must be cited in body
    - Quality review is mandatory — never skip the two-pass review

#### Dispatching this agent (Claude Code recipe)

On Claude Code, dispatch the embedded `slide-creator` spec via the `Agent` tool with `subagent_type="general-purpose"`. Paste the spec body verbatim into the `prompt` field, then append the per-call inputs (file path or inline content):

```
Agent(
  subagent_type="general-purpose",
  description="Generate Slidev presentation",
  prompt="""<paste the slide-creator spec body verbatim>

Input: <file path to a markdown report OR inline content>"""
)
```

This is the post-promotion equivalent of the named `Agent(subagent_type="slide-creator")` call that worked when `slide-creator` lived as a standalone `.claude/agents/slide-creator.md`. The structured `subagent_type` name is lost (Claude Code's plugin loader does not register skill-embedded agent specs as named subagents), but the behavior is identical because the spec body is the same.

#### Dispatching this agent (codex inline-follow)

On codex, the simplest dispatch path is **inline-follow**: the codex agent reads the embedded `## Spawnable Agents > slide-creator` block in this SKILL.md (codex reads SKILL.md directly — see `cafleet/docs/codex-members.md`) and follows the spec's instructions in its own turn, treating the spec body as additional instructions for the current task. No new agent is spawned; the calling agent absorbs the spec's role for one turn.

Use this when:
- A codex session needs to generate slides ad-hoc as part of a larger task.
- You do not need a separate pane or member for the work.

#### Dispatching this agent (codex member-spawn)

When you want a dedicated codex member running the spec, spawn it via `cafleet member create` with `--coding-agent codex`. The spawn prompt is positional (`[PROMPT_ARGV]...`) — there is no `--spawn-prompt-from-text` flag. Paste the embedded spec body verbatim into the positional argument, then append the per-call inputs:

```bash
cafleet --session-id <session-id> member create \
  --agent-id <director-agent-id> \
  --name slide-creator-codex \
  --description "Generate a Slidev presentation autonomously" \
  --coding-agent codex \
  "<paste the slide-creator spec body verbatim>

Input: <file path to a markdown report OR inline content>"
```

The new member opens its own tmux pane and works autonomously on the spec. Use this when the work benefits from a separate pane (parallel decks, longer-running generations, isolation from the director's context).
