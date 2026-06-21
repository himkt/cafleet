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

## Tips

- Use `2:1` or `1:2` when one column has significantly more content
- Keep content balanced — avoid one column being much taller than the other
- Headers span the full width, so use them for context that applies to both columns
- Tables, bullet lists, and code blocks all work inside columns
