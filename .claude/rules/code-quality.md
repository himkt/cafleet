# Code Quality

## No meaningless fallbacks

Do NOT reach for a defaulting accessor when the value is guaranteed to exist. Use direct access and let the code fail loudly if the assumption is wrong.

**Banned patterns:**

| Language | Banned | Use instead |
|----------|--------|-------------|
| Rust | `map.get("key").cloned().unwrap_or_default()`, `opt.unwrap_or(0)` when the invariant guarantees presence | `map["key"]`, `opt.expect("<why the invariant holds>")` |
| Rust | `row.get("col").unwrap_or_else(\|_\| String::new())` | `row.get("col")?` / `.expect(...)` — surface the error |
| TypeScript | `value \|\| "Unknown"`, `value ?? "?"` | `value` (trust the type) |

**Exceptions** (where a default IS appropriate):
- A default that is the documented correct behavior for an expected absent value (e.g. an optional config field with a sensible default)
- Data from external/untrusted sources where the schema is not guaranteed

When in doubt, prefer a loud panic or a typed error over a silent wrong value.

## No unnecessary comments

Only add comments where the logic is genuinely non-obvious. Do NOT add:
- Comments that restate the code (`// Create the session`, `// Return the result`)
- Doc comments that restate the test name or type name
- Multi-paragraph explanations of design decisions (put those in design docs)
- Comments explaining what a function's parameters do (use the type signature)
