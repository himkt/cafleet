---
icon: lucide/download
---

# Install

CAFleet has two install surfaces: the **broker CLI** (`cafleet`), which every
fleet needs, and the **coding-agent plugins**, one per backend you intend to
use.

## CAFleet CLI

The CLI is the binary that every fleet needs. It is published as a Python
package; pick whichever installer matches your environment:

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init             # apply schema migrations (idempotent; rerun after upgrades)
```

The default database lives at `~/.local/share/cafleet/registry.db`. Override
with the `CAFLEET_DATABASE_URL` environment variable — use an absolute path,
since SQLAlchemy does not expand `~` in SQLite URLs.

`cafleet db init` is idempotent: re-run it after upgrading the package and it
will apply any new Alembic migrations without disturbing existing data.

## CAFleet skills

CAFleet supports three coding agents — `claude` (Claude Code), `codex` (OpenAI
Codex CLI), and `opencode` — and the broker CLI is shared by all three. Install
the plugin in whichever coding agent you use.

### (a) GitHub CLI (recommended)

```bash
gh skill install himkt/cafleet --agent claude-code
gh skill install himkt/cafleet --agent codex
gh skill install himkt/cafleet --agent opencode
```

### (b) Claude Code marketplace

```text
/plugin marketplace add himkt/cafleet
/plugin install cafleet@cafleet
```

### (c) Codex

```text
codex plugin marketplace add himkt/cafleet
```

Once the CLI and at least one coding-agent plugin are installed, continue to
the [Configure](configure.md) page for the recommended per-agent settings.
