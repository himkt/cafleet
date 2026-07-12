# Sweep pattern catalog (canonical)

The multi-pass sweep each scanner runs over its slice. `roles/scanner.md` cites
this page. The catalog is a **floor, not a ceiling**: the scanner hand-inspects
the surrounding context of every hit and may classify a hit no pattern named.

## Scope: whole tracked tree minus the exempt set

Sweep **tracked files only** — a multi-pass `git grep` over the tracked tree
naturally excludes untracked generated output such as `webui/dist`. Exclude the
R2 exempt set from every pass:

- `design-docs/` — the historical record.
- `researches/` — gitignored analysis (not tracked; already excluded).
- `cafleet/src/cafleet/db/alembic/versions/**` — a migration legitimately
  references prior/renamed state.
- `cafleet/src/cafleet/webui/dist/**` — generated bundle, not authored prose.
- Lock files (`uv.lock`, `bun.lock` / `bun.lockb`, `package-lock.json`, …).

Everything else tracked is in scope (R3): `docs/`, `README.md`, `SPEC.md`,
`skills/`, `.claude/`, `cafleet/src/`, `cafleet/tests/`, `admin/src/`, and root
files like `CLAUDE.md`, `pyproject.toml`, `mise.toml`, `package.json`. This list
is illustrative — any tracked file outside the exempt set is swept.

A convenient exclusion form for a `git grep`:

```
git grep -nI -e '<pattern>' -- \
  ':(exclude)design-docs/**' \
  ':(exclude)cafleet/src/cafleet/db/alembic/versions/**' \
  ':(exclude)cafleet/src/cafleet/webui/dist/**' \
  ':(exclude)*.lock' ':(exclude)uv.lock' ':(exclude)bun.lockb'
```

## The pattern passes

| Pattern (regex) | Catches |
|---|---|
| `deprecat` | deprecation notices |
| `no longer\|formerly\|previously\|used to` | past-state narration (hand-filter the known-benign present-tense `no longer` / `used to` cases — see `rubric.md`) |
| `\blegacy\b` | "legacy" framing (hand-filter arbitrary fixture names like `legacy_squat`) |
| `sentinel` | removal-sentinel framing |
| `historical` | "historical rows / narration" |
| `forensic\|preserved for\|for history` | forensic-visibility / for-history pointers (hand-filter present-tense `preserved for` / `retained for`) |
| `restoration` | restoration-plan pointers |
| `design[ -]?0[0-9]{6}` **and** `\b0[0-9]{6}\b` | design-number-as-reason citations. The `0[0-9]{6}` form spans the full 7-digit id space (so ids past `0000999` are not silently skipped); the `\b…\b` word-boundary guard keeps all-zero UUID constants (`00000000-0000-…`) from false-matching. Illustrative example slugs are KEEP-listed. |
| `in v1\|first cut` | version qualifiers |
| `backfill\|pre-existing` | migration-backfill narration in prose (keep migration *filenames*) |
| `this replaces\|renamed from\|inverted by\|after design` | replacement / rename / trajectory narration |

## Multi-pass discipline (R4)

- Run **every** pass over the slice — a single grep is never sufficient.
- Hand-inspect **every** hit with its surrounding context before classifying
  (`rubric.md` § Decision procedure). Never classify from the matched substring.
- Record every hit in the partial inventory with its `<file>:<line>` anchor, the
  quoted text, and the rubric class — including KEEP and known-benign hits, so the
  merged inventory can prove the sweep is complete (zero unaccounted matches).
- After apply, re-run every pass over the slice → each remaining hit must be
  KEEP-listed, known-benign, or exempt.
