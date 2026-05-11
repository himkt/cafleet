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

### Superscripts and Subscripts

| Syntax | Renders As | Description |
|--------|-----------|-------------|
| `$x^2$` | x² | Superscript |
| `$x_i$` | xᵢ | Subscript |
| `$x_i^2$` | xᵢ² | Both |
| `$x_{i+1}$` | x₍ᵢ₊₁₎ | Multi-char subscript |

### Fractions

```md
$$\frac{a}{b} \quad \frac{n!}{k!(n-k)!}$$
```

### Greek Letters

| Syntax | Letter | Syntax | Letter |
|--------|--------|--------|--------|
| `$\alpha$` | α | `$\beta$` | β |
| `$\gamma$` | γ | `$\delta$` | δ |
| `$\epsilon$` | ε | `$\lambda$` | λ |
| `$\sigma$` | σ | `$\theta$` | θ |
| `$\pi$` | π | `$\omega$` | ω |

### Common Operators

| Syntax | Description |
|--------|-------------|
| `$\sum_{i=1}^{n}$` | Summation |
| `$\prod_{i=1}^{n}$` | Product |
| `$\int_{a}^{b}$` | Integral |
| `$\lim_{x \to \infty}$` | Limit |
| `$\sqrt{x}$` | Square root |
| `$\leq$`, `$\geq$` | Less/greater than or equal |
| `$\neq$` | Not equal |
| `$\approx$` | Approximately |
| `$\in$` | Element of |

### Matrices

```md
$$\begin{pmatrix} a & b \\ c & d \end{pmatrix}$$
```

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

### Mixed Content with Formulas

```md
---
layout: blank
---

## Attention Mechanism

<Admonition type="formula" title="Scaled Dot-Product Attention">

$$\text{Attention}(Q, K, V) = \text{softmax}\left(\frac{QK^T}{\sqrt{d_k}}\right)V$$

</Admonition>

Where $Q$, $K$, $V$ are the query, key, and value matrices, and $d_k$ is the dimension of the keys.
```

### Formula in Two-Column Layout

```md
---
layout: two-cols
columns: "1:1"
---

:: header ::

# Loss Functions

:: left ::

### Cross-Entropy

<Admonition type="formula" title="Cross-Entropy">

$$L = -\sum_{i} y_i \log(\hat{y}_i)$$

</Admonition>

Used for classification tasks.

:: right ::

### Mean Squared Error

<Admonition type="formula" title="MSE">

$$L = \frac{1}{n}\sum_{i=1}^{n}(y_i - \hat{y}_i)^2$$

</Admonition>

Used for regression tasks.
```

## Formula Admonition Rules

**All display math must use `<Admonition type="formula">`** — no exceptions. The `formula` type uses a larger content font (`1rem`) than other admonition types for better math readability. See `techniques/admonition.md` for full details.

### What Gets Wrapped vs What Stays Inline

| Math Type | Treatment |
|-----------|-----------|
| Display math (`$$...$$`) | **Always** wrap in `<Admonition type="formula">` |
| Inline math (`$...$`) in text | Keep inline, no admonition |
| Math in table cells | Keep inline, no admonition |

### Grouping Multiple Formulas

When a slide has multiple related display formulas, group them in a **single admonition** to save space:

```md
<Admonition type="formula" title="Scaling Laws">

$$L(N) = \left(\frac{N_c}{N}\right)^{\alpha_N}$$

$$L(D) = \left(\frac{D_c}{D}\right)^{\alpha_D}$$

</Admonition>
```

Add brief labels inside the admonition when formulas serve different roles:

```md
<Admonition type="formula" title="MoE">

$$y = \sum_{i=1}^{N} G(x)_i \cdot E_i(x)$$

Top-kゲーティング：

$$G(x) = \text{softmax}\left(\text{TopK}(H(x), k)\right)$$

</Admonition>
```

### Title Guidelines

| Formula Context | Title Example |
|----------------|---------------|
| Named technique or method | `title="Scaled Dot-Product Attention"` |
| Definition | `title="Definition: Cross-Entropy Loss"` |
| Mathematical property | `title="RoPEの相対位置依存性"` |
| Generic equation | `title="学習率スケジュール"` |

## Tips

- Use the `blank` layout for formula-heavy slides to maximize space
- Keep formulas readable at projection size — avoid deeply nested expressions
- Use inline `$...$` for referencing variables in text
- Group related display formulas in a single `<Admonition type="formula">` to save vertical space
- Add `fontSize: "18px"` to slides with formula admonition + table or multiple admonitions
- Combine with `v-click` to reveal formulas progressively
