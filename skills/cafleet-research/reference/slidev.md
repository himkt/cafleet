# Custom Slidev Theme Presentation Guide

Theme location: `slidev/theme/` next to this reference page. For Slidev syntax, refer to /slidev or /slidev:slidev — do not read Slidev's upstream source files directly.

## Headmatter

```yaml
---
theme: <cafleet-plugin-install-dir>/skills/cafleet-research/reference/slidev/theme
# Replace <cafleet-plugin-install-dir> with the absolute path to the installed cafleet plugin's directory on this machine.
# Discovery hints (per coding-agent backend):
#   - claude:    ~/.claude/plugins/cache/cafleet/cafleet/<version>/   (run `claude plugin list` to find <version>)
#   - codex:     the path printed by `codex plugin list` for the cafleet plugin
#   - opencode:  ~/.config/opencode/skills/   (cafleet skills install dir; no plugin cache)
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

### Application

- `<Highlight type="positive">+99%</Highlight>` — positive emphasis (green)
- `<Highlight>81.2%</Highlight>` — neutral emphasis (blue, default)
- `<Admonition type="tip" title="Key Takeaway">text</Admonition>` — callout box
- `<div class="bg-primary-light">text</div>` — lightweight single-line highlight

### Discipline

- **1-2 colored elements per slide max**; **color for data, not decoration** — only color the specific number or keyword. The color tokens, the full semantic palette, and the decision flow are canonical in § *Color Discipline* below.

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

## Two-Column Layouts

The `two-cols` layout splits slide content into two columns with a shared header. Use it for comparisons, text+image pairs, text+code combinations, and side-by-side data.

### When to Use

| Scenario | Example |
|----------|---------|
| Comparisons | Before vs After, Pros vs Cons |
| Text + Code | Explanation on left, code on right |
| Text + Image | Description on left, diagram on right |
| Data + Analysis | Table on left, interpretation on right |

### Syntax

```md
---
layout: two-cols
columns: "1:1"
---

:: header ::

# Slide Title

:: left ::

Left column content here.

:: right ::

Right column content here.
```

#### Frontmatter

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `columns` | String | `'1:1'` | Column ratio using colon notation |

#### Slots

| Slot | Purpose |
|------|---------|
| `:: header ::` | Slide title (rendered with left border accent) |
| `:: left ::` | Left column content |
| `:: right ::` | Right column content |

### Column Ratios

The `columns` prop accepts a colon-separated ratio that maps to CSS grid `fr` units.

| Ratio | Left Width | Right Width | Best For |
|-------|-----------|------------|----------|
| `1:1` | 50% | 50% | Equal comparisons |
| `2:1` | 67% | 33% | Main content + sidebar |
| `1:2` | 33% | 67% | Sidebar + main content |
| `3:2` | 60% | 40% | Slightly wider left |
| `2:3` | 40% | 60% | Slightly wider right |

### Two-Column Tips

- Use `2:1` or `1:2` when one column has significantly more content
- Keep content balanced — avoid one column being much taller than the other
- Headers span the full width, so use them for context that applies to both columns
- Tables, bullet lists, and code blocks all work inside columns

## Formatting Components

`<Admonition>` callout boxes, `<Highlight>` inline emphasis, and per-slide `fontSize` control.

### Admonition

The `<Admonition>` component creates colored callout boxes for highlighting key information, using a left-border accent style consistent with the theme.

#### Types

| Type | Border/Title Color | Background | Use For |
|------|-------------------|------------|---------|
| `note` | Blue (`--c-primary`) | Blue-50 | General information, context, background |
| `important` | Purple (`--c-important`) | Purple-50 | Critical points, must-know items |
| `tip` | Green (`--c-positive`) | Green-50 | Best practices, recommendations, shortcuts |
| `warning` | Orange (`--c-accent`) | Orange-50 | Potential issues, caveats, gotchas |
| `caution` | Red (`--c-negative`) | Red-50 | Risks, dangers, breaking changes, deprecations |
| `formula` | Blue (`--c-primary`) | Blue-50 | Mathematical formulas, definitions, equations (larger content font) |

#### Admonition Syntax

```md
<Admonition type="tip" title="Performance Tip">

Use lazy loading for images below the fold.

</Admonition>
```

Always leave a blank line after the opening tag and before the closing tag for markdown content to render correctly.

#### Admonition Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'note'` | One of: `note`, `important`, `tip`, `warning`, `caution`, `formula` |
| `title` | String | Auto from type | Box title text. Defaults to capitalized type name |

The `formula` type has a larger content font size (`1rem` vs `0.85rem`) for better readability of KaTeX-rendered math — the display-math wrapping rule is § *Math Formulas with KaTeX* below.

#### When to use Admonitions vs alternatives

`<Admonition>` for multi-line titled callouts (`type="formula"` for math/definitions needing prominence); `<Highlight type="...">` for inline semantic emphasis within a sentence (see § Highlight); `.bg-primary-light` for a light paragraph highlight without a title.

### Highlight

The `<Highlight>` component applies a marker-style background + colored text to inline content. Use it to visually emphasize key terms, numbers, or short phrases within a sentence.

#### Highlight Types

| Type | Background | Text Color | Use For |
|------|-----------|------------|---------|
| `primary` | Blue tint | Blue (`--c-primary`) | Key terms, important numbers, default |
| `positive` | Green tint | Green (`--c-positive`) | Positive trends, growth, improvements |
| `negative` | Red tint | Red (`--c-negative`) | Negative trends, decline, risks |
| `accent` | Orange tint | Orange (`--c-accent`) | Warnings, items needing attention |
| `important` | Purple tint | Purple (`--c-important`) | Critical points, must-know items |

#### Highlight Syntax

```md
Revenue grew <Highlight type="positive">+99.3%</Highlight> year-over-year
```

#### Highlight Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'primary'` | One of: `primary`, `positive`, `negative`, `accent`, `important` |

#### Color Discipline

Every colored element must have a semantic reason for its color. Never use color for decoration.

| Color | Token | Semantic | When to use | Example |
|-------|-------|----------|-------------|---------|
| `positive` (green) | `--c-positive` | Good news | Growth, improvement, achievement, gain | Revenue +93%, adoption 84% |
| `negative` (red) | `--c-negative` | Bad news | Decline, risk, vulnerability, loss, stagnation | Vulnerability 45%, -19% slower |
| `primary` (blue) | `--c-primary` | Neutral key metric | Quantities without positive/negative connotation; links | Context window 1M tokens, 5 product forms |
| `accent` (orange) | `--c-accent` | Caution / noteworthy | Transitional states, mixed signals, warnings, items needing attention | -4% (improved but unreliable) |
| `important` (purple) | `--c-important` | Critical / structural | Must-know points that don't fit other categories | — |

**Decision flow**: Ask "is this good or bad for the audience?" → Good: `positive`. Bad: `negative`. Neither: `primary`. Mixed/uncertain: `accent`.

#### Usage Rules

Always use `<Highlight>` for colored emphasis. Do NOT use `<span class="c-...">` utility classes for inline emphasis — they exist for theme internals only.

1. **All colored numbers and keywords use `<Highlight>`.** No exceptions. This keeps visual weight consistent across slides.
2. **Max 3 per slide.** More than 3 highlights dilute attention. If a slide needs more, move data to a table or chart instead.
3. **`<Highlight>` + `**bold**` is redundant** — Highlight already applies `font-weight: 600`.

### Font Size Control

When text is too long and would wrap at an awkward position, use the `fontSize` prop in the slide frontmatter to adjust font size for that slide. Any valid CSS font-size value is accepted (`"18px"`, `"0.9em"`, `"1rem"`, etc.).

```md
---
layout: bullets
fontSize: "18px"
---
```

#### Supported Layouts (per-layout defaults)

| Layout | Default | Affects |
|--------|---------|---------|
| `cover` | `"22px"` | Content/subtitle (`p`), not `h1` |
| `bullets` | `"18px"` | Bullet items (`li`) |
| `bullets-sm` | `"14px"` | Bullet items (`li`) |
| `blank` | `""` (inherit) | All content via inheritance |
| `two-cols` | `""` (inherit) | Both columns via inheritance |

#### When to use vs split the slide

Use `fontSize` when a bullet wraps mid-word, a slide has 4-7 slightly-too-long bullets, or a `blank` / `bullets-sm` slide is borderline overflowing. **Split the slide instead** when content fundamentally exceeds one slide's capacity (more than 7 bullets even at reduced size, or multiple tables/diagrams).

**Priority order when text overflows**: (1) shorten text; (2) `fontSize` prop; (3) split into multiple slides (last resort).

## Math Formulas with KaTeX

KaTeX is a built-in Slidev feature — no theme changes or plugins needed. Use it for equations, formal notation, and mathematical expressions in slides.

### Inline Formulas

Wrap formulas in single dollar signs `$...$` for inline rendering within text.

```md
The Pythagorean theorem states that $a^2 + b^2 = c^2$ for right triangles.
```

### Display (Block) Formulas

**All display math (`$$...$$`) must be wrapped in `<Admonition type="formula">`.**

This ensures visual consistency and makes equations stand out from surrounding text. Only inline math (`$...$`) within sentences and math inside table cells remain unwrapped.

```md
<Admonition type="formula" title="Summation">

$$\sum_{i=1}^{n} x_i = x_1 + x_2 + \cdots + x_n$$

</Admonition>
```

### Common Patterns

For KaTeX syntax — superscripts/subscripts, fractions, Greek letters, operators, matrices — refer to /slidev. KaTeX is stock Slidev; only the theme-specific wrapping rule below is custom.

### Slide Examples

#### Formula-Heavy Slide

```md
---
layout: blank
---

## Gradient Descent

<Admonition type="formula" title="パラメータ更新則">

$$\theta_{t+1} = \theta_t - \eta \nabla J(\theta_t)$$

</Admonition>

Where:
- $\theta_t$ — parameters at step $t$
- $\eta$ — learning rate
- $\nabla J(\theta_t)$ — gradient of the loss function
```

### Formula Admonition Rules

The wrapping rule (§ Display (Block) Formulas above) admits exactly these exceptions (full Admonition details in § *Admonition* above):

| Math Type | Treatment |
|-----------|-----------|
| Display math (`$$...$$`) | **Always** wrap in `<Admonition type="formula">` |
| Inline math (`$...$`) in text | Keep inline, no admonition |
| Math in table cells | Keep inline, no admonition |

Group multiple related display formulas in a **single** `<Admonition type="formula">` to save vertical space; add brief labels inside when formulas serve different roles. Title by context — a named technique (`title="Scaled Dot-Product Attention"`), a definition (`title="Definition: Cross-Entropy Loss"`), or the equation's role.

### Math Tips

- Use the `blank` layout for formula-heavy slides; keep formulas readable at projection size (avoid deeply nested expressions).
- Add `fontSize: "18px"` to slides with a formula admonition + table or multiple admonitions.
- Combine with `v-click` to reveal formulas progressively.

## Code animations

For stock `v-click` / `v-clicks` / line-range highlighting, refer to /slidev.

## Autonomous slide generation

To generate a complete deck autonomously from input content (a research report, outline, or notes), follow this skill's own sections directly — there is no separate agent spec to dispatch:

1. Read the input; extract the title + author (infer a suitable title if absent, default author "Author").
2. Break the content into one-idea-per-slide topics; pick a layout for each per the [Layouts](#layouts) table (`cover` first, `bullets` for most content, `blank` for diagrams/figures/code, `end` last).
3. Apply the [Content Rules](#content-rules), [Color](#color) discipline, and [Citations](#citations) (renumber by first appearance; keep body↔references consistent).
4. Run the [Self-Review Checklist](#self-review-checklist-mandatory) plus a two-pass overflow + citation-ordering review, then write `slide.md` to the working directory.

Start from the [Headmatter](#headmatter) template (substitute the literal `theme:` install path), and add presenter notes (`<!-- notes -->`) with expanded talking points.
