# Phase D live-E2E dispatch list (Verifier, agent 138)

My harness denies every direct CLI runner (`uv run --frozen --package cafleet cafleet …`, `.venv/bin/cafleet …`, and env-prefixed variants), so I cannot produce the live command transcripts myself. Please dispatch these via `cafleet member exec --fleet-id 33 --member-id 138 "<command>"` — each output lands in my pane and I will assess it. All commands are offline (no network, no real registry, no real skill homes): scratch DBs under `/tmp/cafleet-verify-0000117/`, skills half exercised only to its pre-flight / target-resolution errors, per the COMMENT(director) constraints.

Runner: `.venv/bin/cafleet` (repo-root workspace venv; editable, so it runs the branch source; `importlib.metadata.version` resolves to 0.14.0).

Dispatch in order:

1. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet setup db`
   — expect `schema ready at /tmp/cafleet-verify-0000117/verify.db`, exit 0 (also proves parent-dir auto-creation).
2. Re-run command 1 verbatim — expect identical output (idempotency).
3. `sqlite3 /tmp/cafleet-verify-0000117/verify.db "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;"`
   — expect exactly the seven baseline tables incl. `skill_installs`, no `alembic_version`.
4. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet fleet list`
   — expect `Error: no skills install is recorded; run 'cafleet setup' first` (empty-table guard), exit 1.
5. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/fresh.db .venv/bin/cafleet setup skill`
   — expect `Error: the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first`, exit 1 (pre-flight fires before any network).
6. `HOME=/tmp/cafleet-verify-0000117/home CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/bare.db .venv/bin/cafleet setup`
   — expect `skills half failed: no coding-agent homes detected …` then `Error: skills half failed`, exit 1; `bare.db` exists afterwards (db half ran first, halves independent).
7. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet setup --agent claude`
   — expect Click `No such option: --agent`, exit 2 (`--agent` removed from bare setup).
8. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet db init`
   — expect Click `No such command 'db'.`, exit 2.
9. `sqlite3 /tmp/cafleet-verify-0000117/verify.db "INSERT INTO skill_installs VALUES('claude','0.5.0','2026-06-20T10:00:00.000000+00:00'),('codex','0.6.0','2026-06-20T10:00:00.987654+00:00');"`
   — seeds two stale rows.
10. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet fleet list`
    — expect `Error: stale skills detected (claude=0.5.0, codex=0.6.0; CLI 0.14.0); run 'cafleet setup skill' to reinstall`, exit 1 (ascending agent order).
11. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet fleet --help`
    — expect help text, exit 0 (group-level help works under a stale install).
12. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet fleet create --help`
    — expect the stale-skills guard error instead of help, exit 1 (documented help contract).
13. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet doctor`
    — expect the `skills:` block after the tmux block: `cli_version: 0.14.0`, `claude: 0.5.0 (2026-06-20T10:00:00.000000+00:00) STALE`, `codex: … STALE` (timestamps verbatim, microseconds intact); doctor runs despite staleness (exempt).
14. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet --json doctor`
    — expect a sibling `"skills"` key: `{"cli_version": "0.14.0", "installs": [... "current": false ...]}`.
15. `sqlite3 /tmp/cafleet-verify-0000117/verify.db "UPDATE skill_installs SET cafleet_version='0.14.0';"`
16. `CAFLEET_DATABASE_URL=sqlite:////tmp/cafleet-verify-0000117/verify.db .venv/bin/cafleet fleet list`
    — expect a normal (empty) fleet listing, exit 0 (matching rows pass the guard).
17. `rm -rf /tmp/cafleet-verify-0000117`
    — cleanup.

Alternatively: if you judge the recorded fallback evidence sufficient (all four mise gates green, full suite 925 passed, the 69 targeted contract tests enumerated and green, repo sweep clean, release asset 0.14.0 verified present), reply and I will finish the report on that basis and note the live-transcript gap as a COMMENT(verifier) test-gap marker.
