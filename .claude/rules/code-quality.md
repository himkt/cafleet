# Code Quality

## No meaningless fallbacks

Do NOT use `.get("key", default)` or `|| "fallback"` when the key is guaranteed to exist. Use direct access (`dict["key"]`, `obj.prop`) and let the code fail loudly if the assumption is wrong.

**Banned patterns:**

| Language | Banned | Use instead |
|----------|--------|-------------|
| Python | `d.get("key", "")`, `d.get("key", [])`, `d.get("key", "?")` | `d["key"]` |
| Python | `d.get("key", {}).get("subkey")` | `d["key"]["subkey"]` |
| TypeScript | `value \|\| "Unknown"`, `value ?? "?"` | `value` (trust the type) |

**Exceptions** (where `.get()` without a fallback IS appropriate):
- Checking whether an optional key exists: `if d.get("optional_key"):`
- JSON from external/untrusted sources where the schema is not guaranteed
- `metadata.get("toAgentId", "")` in `_save_task` — broadcast_summary genuinely lacks this key

When in doubt, prefer a loud KeyError over a silent wrong value.

## No unnecessary comments

Only add comments where the logic is genuinely non-obvious. Do NOT add:
- Comments that restate the code (`# Create the session`, `# Return the result`)
- Docstrings that restate the test name or class name
- Multi-paragraph explanations of design decisions (put those in design docs)
- Comments explaining what a function's parameters do (use type annotations)

## No `cast()` unless unavoidable

Avoid `typing.cast()`. Use `.returning()` for SQLAlchemy result access, type narrowing with `isinstance`, or protocol classes instead.
