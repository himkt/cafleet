# Highlight Component

The `<Highlight>` component applies a marker-style background + colored text to inline content. Use it to visually emphasize key terms, numbers, or short phrases within a sentence.

## Types

| Type | Background | Text Color | Use For |
|------|-----------|------------|---------|
| `primary` | Blue tint | Blue (`--c-primary`) | Key terms, important numbers, default |
| `positive` | Green tint | Green (`--c-positive`) | Positive trends, growth, improvements |
| `negative` | Red tint | Red (`--c-negative`) | Negative trends, decline, risks |
| `accent` | Orange tint | Orange (`--c-accent`) | Warnings, items needing attention |
| `important` | Purple tint | Purple (`--c-important`) | Critical points, must-know items |

## Syntax

```md
Revenue grew <Highlight type="positive">+99.3%</Highlight> year-over-year
```

### Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'primary'` | One of: `primary`, `positive`, `negative`, `accent`, `important` |

## Examples

### Key Metrics

```md
- Revenue grew <Highlight type="positive">+99.3%</Highlight> year-over-year
- Margin dropped to <Highlight type="negative">37.4%</Highlight> from 59.1%
- Target price: <Highlight>¥16,000</Highlight>
- Tariff risk rated <Highlight type="accent">High</Highlight>
```

### In a Bullets Slide

```md
---
layout: bullets
---

::header::

# Quarterly Performance

::default::

- Q3 revenue reached <Highlight>¥806.3B</Highlight> (+86% YoY)
- Operating margin improved to <Highlight type="positive">19.25%</Highlight>
- Memory costs surged <Highlight type="negative">+41%</Highlight> in 3 months
- Full-year progress rate at 81.2%
```

### In a Table

```md
| Metric | FY2024 | FY2025 | Change |
|--------|--------|--------|--------|
| Revenue | ¥1.16T | ¥2.25T | <Highlight type="positive">+93%</Highlight> |
| Op. Margin | 24.2% | 16.4% | <Highlight type="negative">-7.8pp</Highlight> |
```

## Color Discipline

Every colored element must have a semantic reason for its color. Never use color for decoration.

| Color | Semantic | When to use | Example |
|-------|----------|-------------|---------|
| `positive` (green) | Good news | Growth, improvement, achievement, gain | Revenue +93%, adoption 84% |
| `negative` (red) | Bad news | Decline, risk, vulnerability, loss, stagnation | Vulnerability 45%, -19% slower |
| `primary` (blue) | Neutral key metric | Quantities without positive/negative connotation | Context window 1M tokens, 5 product forms |
| `accent` (orange) | Caution / noteworthy | Transitional states, mixed signals, items needing attention | -4% (improved but unreliable) |
| `important` (purple) | Critical / structural | Must-know points that don't fit other categories | — |

**Decision flow**: Ask "is this good or bad for the audience?" → Good: `positive`. Bad: `negative`. Neither: `primary`. Mixed/uncertain: `accent`.

## Usage Rules

Always use `<Highlight>` for colored emphasis. Do NOT use `<span class="c-...">` utility classes for inline emphasis — they exist for theme internals only.

1. **All colored numbers and keywords use `<Highlight>`.** No exceptions. This keeps visual weight consistent across slides.
2. **Max 3 per slide.** More than 3 highlights dilute attention. If a slide needs more, move data to a table or chart instead.
3. **`<Highlight>` + `**bold**` is redundant** — Highlight already applies `font-weight: 600`.
