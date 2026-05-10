# Font Size Control

When text is too long and would wrap at an awkward position on a slide, use the `fontSize` prop in the slide frontmatter to adjust font size for that slide.

## Usage

Add `fontSize` with a CSS font-size value to the slide's frontmatter:

```md
---
layout: bullets
fontSize: "18px"
---

::header::

# Long Content Slide

::default::

- This bullet has longer text that would wrap awkwardly at normal size
- Another detailed point that benefits from slightly smaller text
```

Any valid CSS font-size value is accepted (`"18px"`, `"0.9em"`, `"1rem"`, etc.).

## Supported Layouts

| Layout | Default | Affects |
|--------|---------|---------|
| `cover` | `"22px"` | Content/subtitle (`p`), not `h1` |
| `bullets` | `"18px"` | Bullet items (`li`) |
| `bullets-sm` | `"14px"` | Bullet items (`li`) |
| `blank` | `""` (inherit) | All content via inheritance |
| `two-cols` | `""` (inherit) | Both columns via inheritance |

## When to Use

- A bullet wraps mid-word or at an unnatural break point
- The slide has 4-7 bullets that are slightly too long to fit at normal size
- A `blank` slide with text + table that is borderline overflowing
- A `bullets-sm` reference slide with long URLs that clip

## When NOT to Use — Split the Slide Instead

- Content fundamentally exceeds one slide's capacity
- More than 7 bullets even at reduced size
- Multiple tables or diagrams on one slide

## Priority Order

When text overflows, apply these strategies in order:

1. **Shorten text** — edit bullets to be more concise
2. **Use fontSize prop** — add `fontSize: "18px"` (or other value) to frontmatter
3. **Split into multiple slides** — last resort for genuinely large content
