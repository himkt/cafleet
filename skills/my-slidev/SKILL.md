---
name: my-slidev
description: Create Slidev presentations using the custom theme with cover, bullets, two-cols, blank, stats-grid, section-divider, and end layouts. Use when generating presentations from research reports, outlines, or other content. References /slidev for syntax details.
---

# Custom Slidev Theme Presentation Guide

Theme location: `theme/` inside this skill's directory. For Slidev syntax, refer to /slidev or /slidev:slidev. **NEVER READ FILES DIRECTLY**.

## Headmatter

```yaml
---
theme: <absolute-path-to-this-skill's-theme-directory>
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
3. **Colors**: must match `/create-figure` palette
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
