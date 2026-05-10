# Research Report Template

## Document Structure

```markdown
# [Title]

**Date:** YYYY-MM-DD
**Scope:** [Brief description of research scope]

---

## Table of Contents

1. [Section Name](#section-name)
   - [Subsection](#subsection)
2. ...

---

## Executive Summary

[2-5 sentences summarizing key findings. Include inline citations for major claims.]

---

## [Section Title]

### [Subsection Title]

[Content with inline numbered citations. Example:]

Acme Corp's annual revenue grew 18% year-over-year to $4.2B in FY2024 [1]. The growth
was primarily driven by expansion into the European market and new enterprise contracts [2,3].

---

## Risk Analysis

| Risk | Severity | Timeline | Status |
|------|----------|----------|--------|
| [Risk name] | High/Medium/Low | [Timeframe] | [Current status] |

---

## References

* [1] "Article Title", https://example.com/article
* [2] "Article Title", https://example.com/another
* [3] Author/Organization, "Article Title", https://example.com/third
```

---

## Reference Format Rules

### Inline Citations

- Use `[N]` immediately after the claim it supports
- Multiple sources for one claim: `[1,2,3]` (no spaces around commas)
- Every factual claim from an external source **MUST** have at least one citation
- Analysis and opinions by the report author do not need citations
- Place the citation before the period: `...grew 18% [1].`

### Reference List

- Number sequentially `[1]`, `[2]`, `[3]`, ... in order of **first appearance** in text
- Format: `[N] "Article Title", URL`
- Optionally include author/org: `[N] Author, "Article Title", URL`
- Each URL appears only once — reuse the same `[N]` for repeated citations
- **Single flat list** — do NOT group by category or section
- Every reference must be cited at least once in the text

---

## Formatting Rules

- **Tables** for comparative data and structured metrics
- **Bullet lists** for enumerated items
- **Bold** for emphasis (not ALL CAPS)
- **Horizontal rules** (`---`) between major sections
- Report language: use the language specified by the user
- Include a Table of Contents for reports with 3+ sections

---

## Quality Standards

- Cross-reference claims across multiple sources when possible
- Prefer recent sources (within 1-2 years of the report date)
- Source priority: official docs > reputable publications > community sources
- Include both supporting and opposing viewpoints for balanced coverage
- Every section should cite at least one reference
- When a claim relies on a single source with low reliability, flag it explicitly (e.g., "Note: this claim is based on a single unverified source")
- The executive summary must be a self-contained overview that accurately reflects the full report
