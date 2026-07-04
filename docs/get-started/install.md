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
cafleet setup               # create the database schema + install the skills
```

`cafleet setup` is the one-step onboarding command — a command group whose
bare invocation runs two independent halves, in order:

1. **Creates the database schema** — a single baseline (`CREATE TABLE IF NOT
   EXISTS`), with no migration chain.
2. **Installs the skills** that match your installed `cafleet` version. It
   downloads the `cafleet-skills-v<version>.zip` asset from the matching GitHub
   Release and extracts the three skill directories (`cafleet`,
   `cafleet-design-doc`, `cafleet-research`) into every detected coding-agent
   home: `claude` → `~/.claude/skills/`, `codex` → `~/.codex/skills/`,
   `opencode` → `~/.config/opencode/skills/`. Only homes that already exist
   are targeted. After each home's install succeeds, setup records the
   installed `cafleet` version for that home in the database, so stale skills
   are detected after an upgrade (see below).

The halves fail independently — a failure in one does not abort the other —
and the command exits non-zero if either half failed. Bare `cafleet setup`
takes no options; each half is also available as its own subcommand:

- `cafleet setup db` — creates the schema only (idempotent); touches nothing
  else.
- `cafleet setup skill [--agent claude|codex|opencode]...` — installs the
  skills only and records the version rows. `--agent` (repeatable) scopes the
  install to the named agents — an explicitly named agent's home is created if
  it does not yet exist; omit it to auto-detect. The database schema must
  already exist: without it the command fails with guidance to run
  `cafleet setup` or `cafleet setup db` first.

Each skill directory is fully replaced on every run, so re-running
`cafleet setup` (or `cafleet setup skill`) after upgrading the package
refreshes the skills to the new version; the schema create is idempotent and
a no-op when the tables already exist.

## Stale-skills detection

Skills are installed per `cafleet` version, and the recorded version is
checked on use: every fleet-scoped command (`fleet *`, `member *`,
`message *`, `monitor *`) refuses to run when no skills install is recorded
or when any recorded version differs from the running CLI version. After
`uv tool upgrade cafleet`, you don't have to remember to re-run setup — the
first fleet-scoped command errors with
`stale skills detected (...); run 'cafleet setup skill' to reinstall`.
`cafleet setup`, `cafleet doctor`, and `cafleet server` stay runnable so you
can always repair; `cafleet doctor` reports the recorded per-home versions.

The default database lives at `~/.local/share/cafleet/cafleet.db`. Override
with the `CAFLEET_DATABASE_URL` environment variable — use an absolute path,
since SQLAlchemy does not expand `~` in SQLite URLs.

!!! warning "No migration from an older schema"

    The schema is a single baseline, with **no migration path and no backward
    compatibility** with any older schema. `cafleet setup` creates the tables
    fresh (`CREATE TABLE IF NOT EXISTS`) but never migrates or alters
    pre-existing tables, so an old-schema database is **not** upgraded. Delete
    any pre-existing database and let `cafleet setup` recreate it. If you set
    `CAFLEET_DATABASE_URL` to a custom path holding an old schema, delete that
    file and re-run `cafleet setup`.

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
