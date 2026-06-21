# Custom Slidev Theme Presentation Guide

Theme location: `slidev/theme/` next to this reference page. For Slidev syntax, refer to /slidev or /slidev:slidev — do not read Slidev's upstream source files directly.

## Headmatter

```yaml
---
theme: <cafleet-plugin-install-dir>/skills/cafleet-research/reference/slidev/theme
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

- **1-2 colored elements per slide max**; **color for data, not decoration** — only color the specific number or keyword. The full semantic-color palette (green = positive, red = negative, blue = neutral, orange = caution, purple = critical) + decision flow is canonical in [`slidev/techniques/formatting.md`](slidev/techniques/formatting.md) § Color Discipline.

## Figures

1. **No duplicate titles** — slide heading IS the chart title
2. **Caption**: `<div class="figure-caption">Source: [N]</div>` — never raw `<div class="text-sm">`
3. **Colors**: must match `visualization.md`'s palette
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

## Techniques

On-demand — Read a technique page only when the slide you are authoring needs it:

| Read | When |
|------|------|
| `slidev/techniques/two-column-layouts.md` | you are building a two-column slide |
| `slidev/techniques/formatting.md` | you need Admonition / Highlight / font-size formatting (or the Color-Discipline palette) |
| `slidev/techniques/math-formulas.md` | the slide carries math formulas |
| /slidev (stock `v-click` / `v-clicks` / line-range highlighting) | you need code animations |

## Autonomous slide generation

To generate a complete deck autonomously from input content (a research report, outline, or notes), follow this skill's own sections directly — there is no separate agent spec to dispatch:

1. Read the input; extract the title + author (infer a suitable title if absent, default author "Author").
2. Break the content into one-idea-per-slide topics; pick a layout for each per the [Layouts](#layouts) table (`cover` first, `bullets` for most content, `blank` for diagrams/figures/code, `end` last).
3. Apply the [Content Rules](#content-rules), [Color](#color) discipline, and [Citations](#citations) (renumber by first appearance; keep body↔references consistent).
4. Run the [Self-Review Checklist](#self-review-checklist-mandatory) plus a two-pass overflow + citation-ordering review, then write `slide.md` to the working directory.

Start from the [Headmatter](#headmatter) template (substitute the literal `theme:` install path), and add presenter notes (`<!-- notes -->`) with expanded talking points.
