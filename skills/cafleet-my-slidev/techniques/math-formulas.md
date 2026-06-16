# Math Formulas with KaTeX

KaTeX is a built-in Slidev feature — no theme changes or plugins needed. Use it for equations, formal notation, and mathematical expressions in slides.

## Inline Formulas

Wrap formulas in single dollar signs `$...$` for inline rendering within text.

```md
The Pythagorean theorem states that $a^2 + b^2 = c^2$ for right triangles.
```

## Display (Block) Formulas

**All display math (`$$...$$`) must be wrapped in `<Admonition type="formula">`.**

This ensures visual consistency and makes equations stand out from surrounding text. Only inline math (`$...$`) within sentences and math inside table cells remain unwrapped.

```md
<Admonition type="formula" title="Summation">

$$\sum_{i=1}^{n} x_i = x_1 + x_2 + \cdots + x_n$$

</Admonition>
```

## Common Patterns

For KaTeX syntax — superscripts/subscripts, fractions, Greek letters, operators, matrices — refer to /slidev. KaTeX is stock Slidev; only the theme-specific wrapping rule below is custom.

## Slide Examples

### Formula-Heavy Slide

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

## Formula Admonition Rules

The wrapping rule (§ Display (Block) Formulas above) admits exactly these exceptions; the `formula` type uses a larger content font (`1rem`) for math readability (full Admonition details in `techniques/formatting.md`):

| Math Type | Treatment |
|-----------|-----------|
| Display math (`$$...$$`) | **Always** wrap in `<Admonition type="formula">` |
| Inline math (`$...$`) in text | Keep inline, no admonition |
| Math in table cells | Keep inline, no admonition |

Group multiple related display formulas in a **single** `<Admonition type="formula">` to save vertical space; add brief labels inside when formulas serve different roles. Title by context — a named technique (`title="Scaled Dot-Product Attention"`), a definition (`title="Definition: Cross-Entropy Loss"`), or the equation's role.

## Tips

- Use the `blank` layout for formula-heavy slides; keep formulas readable at projection size (avoid deeply nested expressions).
- Add `fontSize: "18px"` to slides with a formula admonition + table or multiple admonitions.
- Combine with `v-click` to reveal formulas progressively.
