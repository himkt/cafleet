# Formatting Components

`<Admonition>` callout boxes, `<Highlight>` inline emphasis, and per-slide `fontSize` control. For Slidev syntax, refer to /slidev — do not read Slidev's upstream source files directly.

## Admonition

The `<Admonition>` component creates colored callout boxes for highlighting key information, using a left-border accent style consistent with the theme.

### Types

| Type | Border/Title Color | Background | Use For |
|------|-------------------|------------|---------|
| `note` | Blue (`--c-primary`) | Blue-50 | General information, context, background |
| `important` | Purple (`--c-important`) | Purple-50 | Critical points, must-know items |
| `tip` | Green (`--c-positive`) | Green-50 | Best practices, recommendations, shortcuts |
| `warning` | Orange (`--c-accent`) | Orange-50 | Potential issues, caveats, gotchas |
| `caution` | Red (`--c-negative`) | Red-50 | Risks, dangers, breaking changes, deprecations |
| `formula` | Blue (`--c-primary`) | Blue-50 | Mathematical formulas, definitions, equations (larger content font) |

### Syntax

```md
<Admonition type="tip" title="Performance Tip">

Use lazy loading for images below the fold.

</Admonition>
```

Always leave a blank line after the opening tag and before the closing tag for markdown content to render correctly.

### Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'note'` | One of: `note`, `important`, `tip`, `warning`, `caution`, `formula` |
| `title` | String | Auto from type | Box title text. Defaults to capitalized type name |

The `formula` type has a larger content font size (`1rem` vs `0.85rem`) for better readability of KaTeX-rendered math. All display math (`$$...$$`) must be wrapped in `<Admonition type="formula">` — see `math-formulas.md`.

### When to use Admonitions vs alternatives

`<Admonition>` for multi-line titled callouts (`type="formula"` for math/definitions needing prominence); `<Highlight type="...">` for inline semantic emphasis within a sentence (see § Highlight); `.bg-primary-light` for a light paragraph highlight without a title.

## Highlight

The `<Highlight>` component applies a marker-style background + colored text to inline content. Use it to visually emphasize key terms, numbers, or short phrases within a sentence.

### Types

| Type | Background | Text Color | Use For |
|------|-----------|------------|---------|
| `primary` | Blue tint | Blue (`--c-primary`) | Key terms, important numbers, default |
| `positive` | Green tint | Green (`--c-positive`) | Positive trends, growth, improvements |
| `negative` | Red tint | Red (`--c-negative`) | Negative trends, decline, risks |
| `accent` | Orange tint | Orange (`--c-accent`) | Warnings, items needing attention |
| `important` | Purple tint | Purple (`--c-important`) | Critical points, must-know items |

### Syntax

```md
Revenue grew <Highlight type="positive">+99.3%</Highlight> year-over-year
```

### Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'primary'` | One of: `primary`, `positive`, `negative`, `accent`, `important` |

### Color Discipline

Every colored element must have a semantic reason for its color. Never use color for decoration.

| Color | Semantic | When to use | Example |
|-------|----------|-------------|---------|
| `positive` (green) | Good news | Growth, improvement, achievement, gain | Revenue +93%, adoption 84% |
| `negative` (red) | Bad news | Decline, risk, vulnerability, loss, stagnation | Vulnerability 45%, -19% slower |
| `primary` (blue) | Neutral key metric | Quantities without positive/negative connotation | Context window 1M tokens, 5 product forms |
| `accent` (orange) | Caution / noteworthy | Transitional states, mixed signals, items needing attention | -4% (improved but unreliable) |
| `important` (purple) | Critical / structural | Must-know points that don't fit other categories | — |

**Decision flow**: Ask "is this good or bad for the audience?" → Good: `positive`. Bad: `negative`. Neither: `primary`. Mixed/uncertain: `accent`.

### Usage Rules

Always use `<Highlight>` for colored emphasis. Do NOT use `<span class="c-...">` utility classes for inline emphasis — they exist for theme internals only.

1. **All colored numbers and keywords use `<Highlight>`.** No exceptions. This keeps visual weight consistent across slides.
2. **Max 3 per slide.** More than 3 highlights dilute attention. If a slide needs more, move data to a table or chart instead.
3. **`<Highlight>` + `**bold**` is redundant** — Highlight already applies `font-weight: 600`.

## Font Size Control

When text is too long and would wrap at an awkward position, use the `fontSize` prop in the slide frontmatter to adjust font size for that slide. Any valid CSS font-size value is accepted (`"18px"`, `"0.9em"`, `"1rem"`, etc.).

```md
---
layout: bullets
fontSize: "18px"
---
```

### Supported Layouts (per-layout defaults)

| Layout | Default | Affects |
|--------|---------|---------|
| `cover` | `"22px"` | Content/subtitle (`p`), not `h1` |
| `bullets` | `"18px"` | Bullet items (`li`) |
| `bullets-sm` | `"14px"` | Bullet items (`li`) |
| `blank` | `""` (inherit) | All content via inheritance |
| `two-cols` | `""` (inherit) | Both columns via inheritance |

### When to use vs split the slide

Use `fontSize` when a bullet wraps mid-word, a slide has 4-7 slightly-too-long bullets, or a `blank` / `bullets-sm` slide is borderline overflowing. **Split the slide instead** when content fundamentally exceeds one slide's capacity (more than 7 bullets even at reduced size, or multiple tables/diagrams).

**Priority order when text overflows**: (1) shorten text; (2) `fontSize` prop; (3) split into multiple slides (last resort).
