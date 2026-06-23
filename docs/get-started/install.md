---
icon: lucide/download
---

# Install

CAFleet has two install surfaces: the **broker CLI** (`cafleet`), which every
fleet needs, and the **coding-agent skills**, one set per backend you intend to
use. The recommended end-user path installs both in two commands.

## Quick start (recommended)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet setup               # install the skills + migrate the database
```

`cafleet setup` is the one-step onboarding command. It does two independent
things, and runs both on every invocation:

- **Installs the skills** that match your installed `cafleet` version. It
  downloads the `cafleet-skills-v<version>.zip` asset from the matching GitHub
  Release and extracts the three skill directories (`cafleet`,
  `cafleet-design-doc`, `cafleet-research`) into every detected coding-agent
  home: `claude` → `~/.claude/skills/`, `codex` → `~/.codex/skills/`,
  `opencode` → `~/.config/opencode/skills/`. With auto-detect, only homes that
  already exist are targeted. Scope the install to specific agents with
  `--agent claude|codex|opencode` (repeatable) — an explicitly named agent's
  home is created if it does not yet exist.
- **Migrates the database** to the head Alembic revision — the same code path
  as `cafleet db init`.

Each skill directory is fully replaced on every run, so re-running
`cafleet setup` after upgrading the package refreshes the skills to the new
version and applies any new migrations in the same step.

The default database lives at `~/.local/share/cafleet/cafleet.db`. Override
with the `CAFLEET_DATABASE_URL` environment variable — use an absolute path,
since SQLAlchemy does not expand `~` in SQLite URLs.

!!! warning "Upgrading across the integer-PK rearchitecture"

    There is **no data migration and no backward compatibility** across the
    integer-PK rearchitecture. Delete any pre-existing database. The default
    file moved from `~/.local/share/cafleet/registry.db` to
    `~/.local/share/cafleet/cafleet.db`, so the old file is left untouched and
    ignored — remove it manually. If you set `CAFLEET_DATABASE_URL` to a custom
    path holding an old (UUID-era) schema, the database half of `cafleet setup`
    (like `cafleet db init`) refuses to run against its unknown Alembic
    revision; delete that file and re-run `cafleet setup`.

## Contributor / local-dev install

`cafleet setup` installs the skills from a published Release, so it is the
**end-user (installed-CLI)** path. Contributors working from a clone install
the skills from the working tree instead:

```bash
mise //:skill-install
```

This runs `gh skill install ./ --from-local` for each backend, placing the
skills from your checkout (not a Release) into the three agent homes.

Once the CLI and at least one coding-agent skill set are installed, continue to
the [Configure](configure.md) page for the recommended per-agent settings.
