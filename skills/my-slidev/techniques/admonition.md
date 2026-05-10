# Admonition Component

The `<Admonition>` component creates colored callout boxes for highlighting key information. It uses a left-border accent style consistent with the theme's visual language.

## Types

| Type | Border/Title Color | Background | Use For |
|------|-------------------|------------|---------|
| `note` | Blue (`--c-primary`) | Blue-50 | General information, context, background |
| `important` | Purple (`--c-important`) | Purple-50 | Critical points, must-know items |
| `tip` | Green (`--c-positive`) | Green-50 | Best practices, recommendations, shortcuts |
| `warning` | Orange (`--c-accent`) | Orange-50 | Potential issues, caveats, gotchas |
| `caution` | Red (`--c-negative`) | Red-50 | Risks, dangers, breaking changes, deprecations |
| `formula` | Blue (`--c-primary`) | Blue-50 | Mathematical formulas, definitions, equations (larger content font) |

## Syntax

```md
<Admonition type="tip" title="Performance Tip">

Use lazy loading for images below the fold.

</Admonition>
```

### Props

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | String | `'note'` | One of: `note`, `important`, `tip`, `warning`, `caution` |
| `title` | String | Auto from type | Box title text. Defaults to capitalized type name |

**Note**: Always leave a blank line after the opening tag and before the closing tag for markdown content to render correctly.

## Examples

### In a Blank Layout

```md
---
layout: blank
---

# Database Migration Guide

Always back up your database before running migrations.

<Admonition type="warning" title="Breaking Change">

The `users.email` column is now unique. Duplicate entries must be resolved before migrating.

</Admonition>

<Admonition type="tip" title="Rollback">

Use `migrate:rollback` to undo the last batch of migrations.

</Admonition>
```

### In a Two-Column Layout

```md
---
layout: two-cols
columns: "1:1"
---

:: header ::

# API Authentication

:: left ::

### OAuth 2.0 Flow

1. Client requests authorization
2. User grants permission
3. Client receives token
4. Client uses token for API calls

:: right ::

<Admonition type="important" title="Token Expiry">

Access tokens expire after 1 hour. Use refresh tokens for long-lived sessions.

</Admonition>

<Admonition type="caution" title="Security">

Never store tokens in localStorage. Use httpOnly cookies instead.

</Admonition>
```

### All Five Types

```md
---
layout: blank
---

# Admonition Types

<Admonition type="note">

This provides additional context or background information.

</Admonition>

<Admonition type="important">

This highlights a critical point the audience must understand.

</Admonition>

<Admonition type="tip">

This shares a best practice or useful recommendation.

</Admonition>

<Admonition type="warning">

This flags a potential issue or non-obvious caveat.

</Admonition>

<Admonition type="caution">

This warns about a risk, danger, or breaking change.

</Admonition>
```

## Math Formulas in Admonitions

Use `type="formula"` to give mathematical equations visual prominence. The `formula` type has a larger content font size (`1rem` vs `0.85rem`) for better readability of KaTeX-rendered math.

### Key Formula Highlight

```md
<Admonition type="formula" title="Scaled Dot-Product Attention">

$$\text{Attention}(Q, K, V) = \text{softmax}\left(\frac{QK^T}{\sqrt{d_k}}\right)V$$

</Admonition>
```

### Definition Box

```md
<Admonition type="formula" title="Definition: Cross-Entropy Loss">

$$L = -\sum_{i=1}^{C} y_i \log(\hat{y}_i)$$

Where $C$ is the number of classes, $y_i$ is the true label, and $\hat{y}_i$ is the predicted probability.

</Admonition>
```

### Multiple Formulas on One Slide

```md
---
layout: blank
---

## Activation Functions

<Admonition type="formula" title="ReLU">

$$\text{ReLU}(x) = \max(0, x)$$

</Admonition>

<Admonition type="formula" title="SwiGLU">

$$\text{SwiGLU}(x) = \text{Swish}(xW_1) \odot (xW_2)$$

</Admonition>
```

## When to Use Admonitions vs Alternatives

| Technique | Best For |
|-----------|----------|
| `<Admonition>` | Multi-line callouts with a title and distinct visual emphasis |
| `<Admonition type="formula">` | Mathematical formulas and definitions that need visual prominence |
| `.c-positive` / `.c-negative` spans | Inline colored text within a sentence |
| `.bg-primary-light` | Light background highlight for a paragraph without a title |
