# Two-Column Layouts

The `two-cols` layout splits slide content into two columns with a shared header. Use it for comparisons, text+image pairs, text+code combinations, and side-by-side data.

## When to Use

| Scenario | Example |
|----------|---------|
| Comparisons | Before vs After, Pros vs Cons |
| Text + Code | Explanation on left, code on right |
| Text + Image | Description on left, diagram on right |
| Data + Analysis | Table on left, interpretation on right |

## Syntax

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

### Frontmatter

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `columns` | String | `'1:1'` | Column ratio using colon notation |

### Slots

| Slot | Purpose |
|------|---------|
| `:: header ::` | Slide title (rendered with left border accent) |
| `:: left ::` | Left column content |
| `:: right ::` | Right column content |

## Column Ratios

The `columns` prop accepts a colon-separated ratio that maps to CSS grid `fr` units.

| Ratio | Left Width | Right Width | Best For |
|-------|-----------|------------|----------|
| `1:1` | 50% | 50% | Equal comparisons |
| `2:1` | 67% | 33% | Main content + sidebar |
| `1:2` | 33% | 67% | Sidebar + main content |
| `3:2` | 60% | 40% | Slightly wider left |
| `2:3` | 40% | 60% | Slightly wider right |

## Examples

### Equal Comparison

```md
---
layout: two-cols
columns: "1:1"
---

:: header ::

# REST vs GraphQL

:: left ::

### REST

- Multiple endpoints
- Over-fetching common
- Simpler caching
- Mature ecosystem

:: right ::

### GraphQL

- Single endpoint
- Fetch exactly what you need
- Complex caching
- Flexible queries
```

### Text + Code (Wide Right)

```md
---
layout: two-cols
columns: "2:3"
---

:: header ::

# Configuration

:: left ::

The config file supports:

- Environment-based settings
- Type-safe defaults
- Runtime validation

:: right ::

```ts
export default defineConfig({
  port: 3000,
  database: {
    host: 'localhost',
    name: 'myapp',
  },
})
```
```

### Data + Summary

```md
---
layout: two-cols
columns: "3:2"
---

:: header ::

# Performance Results

:: left ::

| Metric | Before | After |
|--------|--------|-------|
| Build | 45s | 12s |
| Bundle | 2.1MB | 0.8MB |
| LCP | 3.2s | 1.1s |

:: right ::

Key improvements:

- **73%** faster builds
- **62%** smaller bundles
- **66%** better LCP score
```

## Tips

- Use `2:1` or `1:2` when one column has significantly more content
- Keep content balanced — avoid one column being much taller than the other
- Headers span the full width, so use them for context that applies to both columns
- Tables, bullet lists, and code blocks all work inside columns
