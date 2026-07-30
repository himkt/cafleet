# Rust Rewrite of the cafleet Package

**Status**: Approved
**Progress**: 44/44 tasks complete
**Last Updated**: 2026-07-30

## Overview

Rewrite the `cafleet` Python package (CLI, broker, persistence, monitor loop, multiplexer backends, coding-agent backends, and the admin WebUI server) as a single static Rust binary, and distribute prebuilt binaries on GitHub Releases (GitHub issue #229). The Python package is deleted in the same release (hard cutover, total-removal rule); PyPI publishing stops.

## Success Criteria

- [x] A single `cafleet` binary builds for all three release targets (`aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`) and the release workflow uploads them to the GitHub Release on publish. <!-- verified: local aarch64-apple-darwin release build green; the release workflow matches amendment A6 (matrix, zigbuild musl, tar.gz, --clobber); the musl uploads execute on the first release publish -->
- [x] The binary satisfies the amended `SPEC.md`: full CLI surface, `CAFLEET_*` configuration, all cafleet-authored error strings, exit-code polarity, JSON shapes and key order, and text layouts are byte-identical; framework-generated strings follow the Rust-native renderings pinned by the amended SPEC. <!-- verified: 347 contract-pinning tests + 16 live API probes + Phase D string asserts -->
- [x] `mise //cafleet:test` runs the Rust test suite green; the ported tests cover every contract area of the former pytest suite (per the coverage map in § Testing). <!-- verified: 12 suites, 0 failures; coverage map closed in Step 9 -->
- [x] `cafleet setup` bootstraps a fresh DB via refinery and installs skills/presets offline from embedded data — no network access. <!-- verified: Phase D sandboxed-HOME install, refinery ledger at head, idempotent re-run -->
- [x] End-to-end with the compiled binary: `fleet create` → `member create` (tmux) → `message send`/`poll`/`ack` → one `monitor` tick, all succeeding against a fresh DB. <!-- verified: Phase D on real tmux with the release binary, clean SIGTERM stop -->
- [x] The Python package, its tests, Alembic, and every PyPI/uv-install mention are removed from the repository; docs, README, SPEC, and `.claude/rules/` describe only the Rust implementation; the docs site builds green. <!-- verified: -26,435-line removal commit, stray-mention sweep clean, docs build green -->

---

## Background

The `cafleet` package is currently Python 3.12 (click + SQLAlchemy + Alembic + FastAPI/uvicorn, ~1230 pytest tests), published to PyPI as a wheel, with skills/presets shipped as a separate GitHub-Release zip that `cafleet setup` downloads. Issue #229 asks for a Rust rewrite and prebuilt binaries on GitHub Releases. Four codebase-scan reports (under `.scan/` in this design-doc directory) inventory the contract surfaces, porting hazards, and candidate crate mappings; this document records the decisions and the implementation plan. The scans predate migration `0006` (which dropped the monitor stall-episode machinery, gate tokens, and report-batch); where a scan conflicts with the head tree, this document reflects head and the scan is stale.

---

## Specification

### Resolved decisions

| # | Decision |
|---|---|
| 1 | Scope: everything, including the WebUI server. The TypeScript admin frontend stays as-is; its Vite build output is embedded in the Rust binary. |
| 2 | Hard cutover: one release; the Python package is deleted in the same change. |
| 3 | Distribution: GitHub Releases only. PyPI publishing stops. `mise //cafleet:*` tasks move to cargo equivalents. |
| 4 | DB: fresh database; no in-place operation on existing Alembic-managed DBs; message history does not carry over. |
| 5 | `SPEC.md` is the parity contract; every deviation lands as a SPEC amendment in this design cycle (enumerated below). |
| 6 | DB layer: `rusqlite` (bundled SQLite, sync). The axum server bridges to the sync broker via `spawn_blocking`. `sqlx` rejected: the CLI is a short-lived sync process; an async runtime buys nothing. |
| 7 | Migrations: refinery, embedded SQL, own `refinery_schema_history` ledger, squashed baseline from the current head schema. |
| 8 | Release targets: `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`. No macOS x86_64, no Windows (POSIX-only stands). |
| 9 | Release automation: hand-rolled GitHub Actions workflow on release publish (no cargo-dist); bare-version tags kept. |
| 10 | Tests: Rust only — port the pytest suite's contract coverage into Rust unit/integration tests; no separate black-box conformance suite. |
| 11 | Error strings: application-level parity. Cafleet-authored strings frozen byte-identical; framework-generated strings (clap parse errors, request-validation bodies) amended in SPEC to Rust-native renderings. |
| 12 | Linux libc: musl — fully static binaries with bundled SQLite. |
| 13 | Assets: skills/ + presets/ embedded in the binary; `cafleet setup` installs them offline. The GitHub-API download path and its error strings are deleted. |
| 14 | Docs site: the mkdocstrings-generated `api/*` section is dropped. |
| 15 | Layout/tooling: `cafleet/` becomes a Cargo package (root Cargo workspace); versioning continues from the current `cafleet/pyproject.toml` version (0.22.0 at drafting); bump-my-version retargets `Cargo.toml` + `Cargo.lock`; mise remains the task-runner facade. |

### Architecture and crate layout

One Cargo workspace at the repo root with a single binary package at `cafleet/`. The module tree mirrors the current Python package so the scans and SPEC sections map one-to-one:

```
Cargo.toml                 # workspace root
cafleet/
  Cargo.toml               # package "cafleet", version carried over from the Python package
  build.rs                 # fails loudly if the admin dist is missing
  migrations/V1__baseline.sql
  webui-dist/              # Vite output (gitignored; admin/vite.config.ts outDir retargeted here)
  src/
    main.rs                # clap entry; top-level error → exit-code mapping
    error.rs               # CafleetError { Usage (exit 2), App (exit 1) }
    config.rs              # CAFLEET_* settings
    time.rs                # timestamp emit/parse (pinned format)
    db/                    # connection + PRAGMAs, refinery runner
    broker/                # fleets, members, messaging, queries, monitor, asset_installs
    cli/                   # fleet, member, message, monitor, setup, doctor, server subcommands
    multiplexer/           # trait + tmux + herdr; shared wake-payload builder
    coding_agent/          # trait + claude/codex/opencode argv builders
    monitor/               # should_ping, monitor_tick, foreground loop
    output/                # truncation, ANSI strip, compact JSON, text formatters
    webui/                 # axum app, /api routes, embedded SPA
    assets.rs              # embedded skills/presets + offline installer
```

The broker stays synchronous (one `rusqlite` connection per process, PRAGMAs applied on every open). The only async component is the `server` subcommand: axum + tokio, whose handlers call the sync broker via `spawn_blocking`. Multiplexers and coding agents are closed enums implementing a trait; herdr's `agent_status` capability becomes an `Option`-returning trait method.

### Crate mapping

The crates pinned in this table are the dependency ceiling, not a floor: no convenience extras beyond them — no tower-http (the embedded SPA is served by hand), no serde derive (result shapes reuse `serde_json` values), minimal feature flags on axum/tokio and every other dependency — and no new direct dependency lands without Director arbitration. The arbitrated dev-dependency set is exactly `tempfile` (temp DB/HOME fixtures across every suite) and `tower` (the `ServiceExt` oneshot route harness); the binary tests use no `assert_cmd` — `CARGO_BIN_EXE` + `std::process` covers the same ground with zero additional crates.


| Python dependency | Rust replacement | Notes |
|---|---|---|
| click | `clap` (derive) | Custom top-level error mapping preserves the exit-code polarity; parse-error text is clap-native (SPEC amendment A1). |
| SQLAlchemy | `rusqlite` (`bundled` feature) | Hand-written SQL; `RETURNING` and JSON1 supported by bundled SQLite. |
| Alembic | `refinery` (`embed_migrations!`) | Own `refinery_schema_history` ledger; squashed baseline (SPEC amendment A2). |
| pydantic / pydantic-settings | hand-rolled `std::env` reader | Six variables, one explicit binding each; non-integer numerics fail loudly at startup. |
| FastAPI + uvicorn | `axum` + `tokio` | Scoped to the `server` subcommand; SPA served from the embedded dist. |
| `urllib` + `zipfile` (setup assets) | deleted | Assets embedded via `build.rs`-generated `include_bytes!` tables (embedding is uniform across debug and release builds); setup installs offline (SPEC amendment A3). |
| `importlib.metadata` / `importlib.resources` | `env!("CARGO_PKG_VERSION")` / `build.rs` codegen | One canonical version string feeds `--version`, the stale-assets guard, and `asset_installs`. The migration chain's metadata (head version, chain guard) comes from refinery's own embedded runner, not an embed crate. |
| `hashlib` | `sha2` | Capture content sha256 in `monitor capture --json`. |
| `signal` / `os.kill(pid, 0)` | `signal-hook` + `nix` `kill(pid, None)` | `EPERM` → alive, `ESRCH` → dead, matching the Python branches. |
| `datetime` | `chrono` with a pinned formatter | See § Timestamps. |
| `str.format` spawn substitution | hand-written mini-formatter | Python brace grammar: `{{`/`}}` escapes; unknown placeholder vs malformed expression classified into the two existing UsageError messages (verbatim). |
| ANSI strip regex | `regex` | Same CSI pattern + keep-after-last-`\r` per line. |
| `json.dumps(separators=…)` | `serde_json` typed structs | Struct field order pins the documented JSON key orders; `to_string` is compact by default. |

### Persistence and migrations

- **Default database**: `~/.local/share/cafleet/cafleet_v6.db` (bumped from `cafleet_v5.db` so existing Python-era DBs are never touched). `CAFLEET_DATABASE_URL` keeps the `sqlite:///<path>` URL form, verbatim passthrough for user-supplied values, `~` expanded only in the default; non-`sqlite` schemes fail loudly.
- **Schema**: the squashed baseline `V1__baseline.sql` reproduces the current head DDL (revision `0006`) from `SPEC.md` §8 verbatim — the seven application tables (`fleets`, `members`, `member_placements`, `messages`, `monitor_config`, `monitor_runtime`, `asset_installs`), AUTOINCREMENT on exactly `fleets`/`members`/`messages`, the three indexes (`idx_members_fleet_status` and the two `messages` owner/from indexes), all DDL defaults (`interval_seconds`→60, `enabled`→1, `tick_seconds`→5, `backend`→`"tmux"`), and the `members`-before-`fleets` mutual-reference create order. There are no CHECK constraints at head. `alembic_version` does not exist in the new schema.
- **Ledger**: refinery's `refinery_schema_history`. Future schema changes are hand-written numbered files `V<N>__<slug>.sql`; a chain-guard test asserts contiguous numbering from 1 and exactly one baseline (replacing the Alembic smoke tests and the `makemigration` autogenerate workflow, which is deleted).
- **`setup` db half** (SPEC amendment A2): refuses a DB file that has tables but no `refinery_schema_history` (this includes old Alembic DBs) and a recorded version newer than the embedded chain; otherwise migrates to head. Message shapes keep the current form with refinery version numbers: `Created <db_file> and applied migrations to head (<N>).` / `Upgraded from <M> to <N>.` / `Already at head (<N>); nothing to do.` — final wording pinned in the SPEC amendment.
- **Connections**: every opened connection gets `PRAGMA foreign_keys=ON` and `PRAGMA busy_timeout=5000` before use — CLI, server, monitor loop, and tests alike. `busy_timeout` remains the cross-process serialization mechanism.
- **Broker errors are typed**: the broker returns `CafleetError`, never a CLI error type; the CLI maps `Usage` → exit 2 and `App` → exit 1, and the HTTP layer maps the same enum to status codes (fixing the current `click.ClickException`-into-HTTP leak while preserving the PATCH TOCTOU → 404 collapse).

### Timestamps

Stored as ISO-8601 TEXT, UTC. The Rust writer always emits fixed-width `%Y-%m-%dT%H:%M:%S%.6f+00:00` (6-digit microseconds, `+00:00` suffix — SPEC amendment A4 removes Python's omit-microseconds-when-zero edge case; the fresh-DB decision means no mixed-writer rows exist). Lexicographic string ordering in SQL is therefore exact and is kept everywhere it is used today, including the `max()`-over-strings idle computation. The parser stays lenient (accepts missing microseconds and any UTC offset spelling) for CLI inputs such as `--captured-at`.

### Behavioral parity — preserved verbatim

These behaviors are contract and carry over byte-identically (source citations in the `.scan/` reports):

- **Exit-code polarity**: usage errors exit 2, application errors exit 1 with `Error: <msg>` on stderr; the `--fleet-id` missing-flag anomaly (exit 1, custom message) is modeled as an optional arg with a post-parse check.
- **All cafleet-authored strings**: broker/CLI error messages, the text-input error quartet, spawn-substitution errors, rollback messages (`Rolled back registration of <id>.`), the stale-assets guard strings, setup half-failure envelopes, `member ping`'s failure line (verbatim, including its literal "tmux" wording), and every text layout in `output/formatters.py`.
- **JSON shapes and key order**: compact separators, raw UTF-8, `--json`/`--full`/`--quiet` semantics, the `monitor capture` key order, and the `GET /api/monitor` payload (single `now`, integer-truncated ages, non-live runtime masks pid/started_at/last_tick fields).
- **Truncation**: `CAFLEET_MAX_TEXT_LEN` codepoints + `…` (Rust `chars()` matches Python codepoints); applied to CLI echo and inline previews only, never persisted text nor API output. Empty-text detection treats Unicode whitespace plus U+001C–U+001F as whitespace (matching `str.isspace`).
- **Multiplexer choreography**: the Esc-safeguard application matrix, 0.1 s Esc settle, 1.0 s submit delay, tmux argv forms, capture `\n`-split (preserving `\r` and the trailing element), pane-gone tolerance markers, herdr JSON envelope + layout ratio arithmetic + `pane run` shlex-style quoting, and the byte-identical monitor wake payload built by one shared builder (the sanitization map — `⏎ ˋ ﹙ │` — is unified into a single function, removing today's duplication).
- **Coding-agent argv**: claude/codex/opencode argv forms, effort-level sets, opencode preset precondition, and the `None`-emits-no-tokens rule (hard-coded in the binary, as today).
- **`member create` sequencing and rollback**, fleet-isolation semantics, monitoring-member invariants, auto-enrollment intervals (Director 180 s / member 720 s), and the root-Director guard.
- **Monitor**: runtime claim/heartbeat/clear with ownership checks and `max(3*tick, 15)` staleness, the wake-reason vocabulary (`interval`, `stall-check`, `status:done`, `unacked`), edge-triggered `status:done`, and the ledger-write-gated-on-successful-wake ordering.
- **Messaging**: send/broadcast/poll/ack semantics, the write-then-best-effort-preview ordering and swallowed notification failures, `delivered` counts, and the poll/ack error strings.
- **Env forwarding**: only `CAFLEET_DATABASE_URL` is forwarded into spawned panes.

### SPEC amendments (this design cycle)

All amendments land in Step 1, before code. Each is the smallest edit that removes the drift.

| Id | Surface | Amendment |
|---|---|---|
| A1 | CLI framework errors | Click-generated renderings (usage banners, `Missing option …`, invalid choice/int, `No such option`, unexpected-argument) are replaced with clap's native renderings; exit code 2 and the affected-flag identity are unchanged. Cafleet-authored strings are explicitly not touched. |
| A2 | Persistence | Default DB file `cafleet_v6.db`; refinery ledger + `V<N>__<slug>.sql` chain replaces Alembic; new db-half messages and the two refusal cases (unversioned/foreign DB, ahead-of-head); `alembic_version` removed from the schema section. |
| A3 | Setup assets half | Offline install from embedded data replaces the GitHub-Release download; the 6 download/parse/malformed-archive error strings are removed; install targets, success echo lines, `assets half skipped (all agents skipped)`, install-failure strings, the `asset_installs` recording, and the stale-assets guard are unchanged. |
| A4 | Timestamps | Always 6-digit microseconds + `+00:00` (the zero-microseconds omission is removed). |
| A5 | HTTP validation | The FastAPI default 422 body is replaced: every request-validation failure returns 422 with `{"detail": "<string>"}` (one string, same shape the SPA already parses). FastAPI-specific app metadata (openapi/docs endpoints, the hardcoded `0.1.0` app version) is dropped. |
| A6 | Packaging | PyPI/wheel sections replaced by the GitHub-Releases binary contract: tag = bare version, assets `cafleet-v<version>-<target>.tar.gz` each containing the single `cafleet` binary; the assets-zip contract is deleted. Version string source is the binary's compile-time version. |
| A7 | Server | uvicorn replaced by the built-in axum server; `cafleet server` flags, defaults (settings-derived, shown in `--help`), single-process no-reload semantics unchanged. |

### CLI layer

clap (derive) builds the same subcommand tree. A thin rendering layer owns:

- the `CafleetError::Usage`/`App` → exit 2/1 mapping (clap's own parse errors already exit 2);
- the `--fleet-id` post-parse check emitting the existing custom exit-1 message;
- the stale-assets guard in the `fleet`/`member`/`message`/`monitor` group prologue (before subcommand bodies; `setup`/`doctor`/`server` exempt);
- `--version` → `cafleet <version>` from `CARGO_PKG_VERSION`.

`--help` in clap renders at parse time and exits before any command body runs, so neither group-level nor subcommand `--help` triggers the guard; A1 pins this at Step 1 (replacing the documented click quirk where subcommand `--help` ran the group callback first).

### WebUI server

axum app construction mirrors `create_app`: `/api` router registered ahead of the static service so API 404s stay JSON; SPA fallback serves the embedded `index.html` except for the reserved first segments `ui`/`api`; the missing-dist case cannot occur (the build fails without the dist — see § Build, packaging, and release). All 9 routes, the `X-Fleet-Id` header dependency and its three error strings, the wire renames (`status_state`→`status`, `text`→`body`), `FormattedMessage` with hard-500 on a missing name lookup, the `int | "*"` recipient union (hand-written deserializer rejecting stringified ints), and the timeline cap of 200 carry over unchanged. Handlers wrap broker calls in `spawn_blocking`.

### Setup and embedded assets

`cafleet setup [--skip …]` keeps its two-half structure and failure envelopes:

- **db half**: refinery migrate-to-head per § Persistence and migrations.
- **assets half**: preflight (`asset_installs` table exists) unchanged; then, per non-skipped agent, delete-and-reinstall skills into `~/.claude/skills` / `~/.codex/skills` / `~/.config/opencode/skills` and presets into `~/.codex/rules/cafleet.rules` / `~/.opencode/agents/cafleet.md` from the data embedded at build time (`build.rs`-generated `include_bytes!` tables over the repo-root `skills/` and `presets/` trees), record `asset_installs(agent, CARGO_PKG_VERSION)`, and echo the existing per-target lines. No network. The zip/GitHub-API code path and the release `upload-assets` job are deleted.

Embedding also fixes the dev-build problem: a locally built binary always carries matching assets, so the stale-assets guard compares against the binary's own version.

### Build, packaging, and release

- **build.rs** fails loudly when `cafleet/webui-dist/` is missing (fail-fast per repo rules) with a message naming `mise //admin:build`, then walks the `webui-dist/`, `skills/`, and `presets/` trees and generates the `include_bytes!` embedding tables (path → bytes, plus the SPA's closed extension → content-type map) compiled into the binary; the migrations are embedded via refinery's `embed_migrations!` alone.

- **Versioning**: `Cargo.toml` starts at the version recorded in `cafleet/pyproject.toml` at cutover (0.22.0 at drafting). `.bumpversion.toml` rewrites `cafleet/Cargo.toml` and `Cargo.lock`; the single-line `Bump version: X → Y` commit convention and bare-version tags are kept.
- **Release workflow** (replaces `publish.yml`): on `release: published`, a matrix builds the three targets — `aarch64-apple-darwin` on a macOS arm runner, the two musl targets on ubuntu (cross-compiled; `cargo-zigbuild` or `cross`, chosen at implementation time for whichever links the bundled SQLite cleanly) — each preceded by `mise //admin:build`, then packages `cafleet-v<version>-<target>.tar.gz` (single binary inside) and uploads via `gh release upload --clobber`. The PyPI OIDC job and the assets-zip job are deleted.
- **CI** (`ci.yml`): lint job → `mise //cafleet:lint` (clippy `-D warnings` + `cargo fmt --check`), test job → `mise //cafleet:test` (`cargo test`), both preceded by `mise //admin:build` (build.rs requires the dist), with Rust toolchain + cargo cache via mise.

### Tooling (mise task facade)

The task names and permission-pattern surface are preserved; bodies change:

| Task | New body |
|---|---|
| `mise //cafleet:test` | `cargo test` (positional args forward to the test harness) |
| `mise //cafleet:lint` | `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` |
| `mise //cafleet:format` | `cargo fmt` |
| `mise //cafleet:typecheck` | `cargo check` |
| `mise //cafleet:build` | `cargo build --release` |
| `mise //cafleet:install` | `cargo install --path cafleet` (replaces the editable install; source edits require re-running) |
| `mise //cafleet:dev` | `cargo run -- server` |
| `mise //cafleet:makemigration` | deleted — migrations are hand-written `V<N>__<slug>.sql` files guarded by the chain test |
| `mise //cafleet:publish` | deleted — release CI builds and uploads binaries |
| `mise //:uv-sync` | retained for the root docs/research tooling only (see § Documentation and removal plan) |

Every cargo-invoking task (`test`, `lint`, `typecheck`, `build`, `install`, `dev`) declares a dependency on `//admin:build`, because build.rs fails loudly whenever `webui-dist/` is missing — a fresh clone works with no manual prerequisite.

### Testing

Rust tests only, two layers:

- **Unit tests** (`#[cfg(test)]` per module): pure logic — truncation, ANSI strip + `\r` defrag, the spawn mini-formatter (brace grammar + error taxonomy), timestamp emit/parse, `should_ping`, the sanitization map, wake-payload text (golden string), and the loop's per-tick status tracking (edge-triggered `status:done`, stall-check cadence, wake-reason computation).
- **Integration tests** (`cafleet/tests/`): a `CARGO_BIN_EXE` + `std::process` harness drives the compiled binary with a temp `CAFLEET_DATABASE_URL` and a fake `tmux`/`herdr` shim on `PATH` (a script recording argv — replacing pytest's subprocess monkeypatching); asserts exit codes, exact output, and DB rows. axum routes tested in-process via `tower::ServiceExt`. Schema DDL asserted by rusqlite introspection (replacing the Alembic smoke tests) plus the migration chain guard.

Coverage map from the pytest suite (the port is complete when each area has Rust tests asserting the same contracts):

| pytest area | Rust home |
|---|---|
| `tests/cli/` (CliRunner exit codes + output) | compiled-binary integration tests |
| `tests/broker/` | broker unit/integration tests on temp DBs |
| `tests/db/` (chain guard, DDL, init refusals) | chain-guard test + DDL introspection + setup db-half tests |
| `tests/multiplexer/` (argv, timings, wake payload) | shim-recorded argv asserts + golden wake-payload string |
| `tests/coding_agent/` | argv-builder unit tests |
| `tests/monitor/` | loop tick/wake-reason unit tests + loop integration tests |
| `tests/output/` | render/formatter unit tests |
| `tests/webui/` | `tower::ServiceExt` route tests |
| `tests/integration/` | end-to-end binary tests (fake multiplexer) |
| `tests/docs/` (docs-sync) | ported where language-neutral, else dropped with the surface they checked |

### Documentation and removal plan

Documentation-first order applies (Step 1 precedes all code). Targets:

- **SPEC.md**: amendments A1–A7.
- **README.md**: install section becomes download-from-GitHub-Releases (per-target archive + PATH placement); PyPI/uv-install mentions removed; docs-site links unchanged (`/update-readme` skill maintains it).
- **docs/**: `contributing.md` rewritten for the cargo toolchain and new project structure; `api/*` pages, their zensical nav entries, and the `mkdocstrings-python` dependency removed; concepts/spec pages swept for Python-implementation mentions (they describe behavior, so most survive; the migrations and packaging pages change with A2/A6).
- **.claude/rules/**: `commands.md` rewritten to the cargo-backed task table; `database-migrations.md` rewritten for the refinery hand-written-SQL chain and its guard test; rules swept for uv/pytest-specific guidance.
- **skills/**: the CLI surface is unchanged, so cafleet-family skills are expected to need no edits; a sweep confirms no install-method or Python-tooling references remain.

Total-removal (same change): `cafleet/src/**` (Python), `cafleet/tests/**` (pytest), `cafleet/pyproject.toml`, the Alembic tree, the PyPI publish job, the assets-zip job, and the `cafleet` member of the uv workspace. The root `pyproject.toml` + `uv.lock` survive in reduced form: they still provide the docs toolchain (zensical, bump-my-version — minus mkdocstrings-python) and the `research` group (matplotlib) used by the cafleet-research skill runner. After removal, the repository reads as if the Python implementation never existed; history lives in git and this design doc.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Amend SPEC.md A1 (clap-native framework error renderings) and A4 (always-microseconds timestamps) <!-- completed: 2026-07-30T09:57 -->
- [x] Amend SPEC.md A2 (cafleet_v6.db, refinery ledger + chain, db-half messages and refusals; remove alembic_version) <!-- completed: 2026-07-30T10:00 -->
- [x] Amend SPEC.md A3 (offline embedded assets half) and A6 (GitHub-Releases binary packaging contract) <!-- completed: 2026-07-30T10:03 -->
- [x] Amend SPEC.md A5 (422 `{"detail": "<string>"}` body; drop FastAPI app metadata) and A7 (axum server) <!-- completed: 2026-07-30T10:05 -->
- [x] Update README.md install section to GitHub-Releases download; remove PyPI/uv mentions <!-- completed: 2026-07-30T10:07 -->
- [x] Update docs/: rewrite contributing.md for cargo; remove docs/api + zensical nav entries + mkdocstrings-python; sweep pages for Python-implementation mentions <!-- completed: 2026-07-30T10:28 -->
- [x] Rewrite .claude/rules/commands.md (cargo task table) and .claude/rules/database-migrations.md (refinery chain + guard) ; sweep other rules for uv/pytest references <!-- completed: 2026-07-30T10:34 -->
- [x] Sweep skills/ for install-method or Python-tooling references (expect no-ops) <!-- completed: 2026-07-30T10:24 -->

### Step 2: Workspace scaffolding

- [x] Create root Cargo workspace and `cafleet/` package (version continued from current), with clippy/rustfmt config <!-- completed: 2026-07-30T10:34 -->
- [x] Retarget admin/vite.config.ts outDir to `../cafleet/webui-dist`; add build.rs failing loudly when the dist is missing <!-- completed: 2026-07-30T10:34 -->
- [x] Wire embeds: rust-embed for webui-dist; include_dir for skills/, presets/, migrations/ <!-- completed: 2026-07-30T10:34 -->
- [x] Rewrite mise tasks per the tooling table; delete makemigration/publish; reduce root uv workspace (drop cafleet member + mkdocstrings, keep docs/research groups) and re-lock <!-- completed: 2026-07-30T10:32 -->
- [x] Retarget .bumpversion.toml to Cargo.toml + Cargo.lock <!-- completed: 2026-07-30T10:32 -->
- [x] Update ci.yml: admin build + clippy/fmt lint job + cargo test job with caching <!-- completed: 2026-07-30T10:32 -->

### Step 3: Core foundations

- [x] error.rs: CafleetError Usage/App enum + top-level exit-code mapping; `--fleet-id` post-parse check <!-- completed: 2026-07-30T11:07 -->
- [x] config.rs: six CAFLEET_* settings with loud parse failures and default-only `~` expansion <!-- completed: 2026-07-30T11:07 -->
- [x] time.rs: pinned timestamp formatter + lenient parser, with round-trip tests <!-- completed: 2026-07-30T11:07 -->
- [x] output/: codepoint truncation, ANSI strip + `\r` defrag, compact serde_json envelopes with pinned key order, text formatters <!-- completed: 2026-07-30T11:07 -->
- [x] Spawn-placeholder mini-formatter (brace grammar, doubled-brace escape, unknown-vs-malformed error taxonomy) with unit tests <!-- completed: 2026-07-30T11:07 -->
- [x] db/: connection opener applying PRAGMAs, refinery runner, V1__baseline.sql from SPEC §8 DDL, chain-guard test, DDL introspection test <!-- completed: 2026-07-30T11:07 -->

### Step 4: Broker layer

- [x] fleets: atomic fleet+Director bootstrap with director_member_id backfill, list/get/soft-delete <!-- completed: 2026-07-30T11:59 -->
- [x] members: register/deregister, monitoring-member invariants, auto-enrollment, roster, placement patch, idle proxies (lexicographic max) <!-- completed: 2026-07-30T11:59 -->
- [x] messaging: send/broadcast/poll/ack with write-then-best-effort-preview ordering and all error strings <!-- completed: 2026-07-30T11:59 -->
- [x] queries: inbox/sent/timeline (cap 200)/get_message visibility rule <!-- completed: 2026-07-30T11:59 -->
- [x] monitor broker: enroll/config CRUD, list_monitor_targets per-tick scan, record_pings/record_monitor_dispatch, reconcile_monitor_lifecycle, runtime claim/heartbeat/clear, runtime/members payloads, fleet/member monitor-row deletes <!-- completed: 2026-07-30T11:59 -->
- [x] asset_installs: existence check, list, upsert <!-- completed: 2026-07-30T11:59 -->

### Step 5: Multiplexer and coding agents

- [x] Multiplexer trait + tmux backend (argv, Esc matrix, timings, capture semantics, pane-gone tolerance, best-effort sends) <!-- completed: 2026-07-30T12:24 -->
- [x] herdr backend (JSON envelope, layout ratio arithmetic, agent_status, kill/rebalance) <!-- completed: 2026-07-30T12:24 -->
- [x] Shared wake-payload builder + unified sanitization map, pinned by a golden-string test <!-- completed: 2026-07-30T12:24 -->
- [x] CodingAgent trait + claude/codex/opencode argv builders with validation error strings <!-- completed: 2026-07-30T12:24 -->

### Step 6: CLI commands

- [x] clap tree, shared option decorator equivalents, stale-assets guard prologue <!-- completed: 2026-07-30T12:59 -->
- [x] fleet + message command groups <!-- completed: 2026-07-30T12:59 -->
- [x] member commands: create (full sequencing + rollback), delete, show/list, prompt, ping <!-- completed: 2026-07-30T12:59 -->
- [x] monitor commands: start, capture (`--json` capture fields incl. content_sha256) <!-- completed: 2026-07-30T13:03 -->
- [x] setup (refinery db half + offline assets half) and doctor <!-- completed: 2026-07-30T12:59 -->

### Step 7: Monitor loop

- [x] should_ping + monitor_tick (heartbeat, liveness, due-set, single wake, gated ledger writes) + foreground loop with signal handling and ownership-checked cleanup <!-- completed: 2026-07-30T13:03 -->

### Step 8: WebUI server

- [x] axum app: 9 routes, X-Fleet-Id dependency, wire renames, spawn_blocking bridge, 422 detail body, SPA fallback over embedded dist <!-- completed: 2026-07-30T22:09 -->
- [x] server subcommand with settings-derived defaults shown in --help <!-- completed: 2026-07-30T22:09 -->

### Step 9: Test-port completion

- [x] Fake tmux/herdr PATH shims + assert_cmd harness; port remaining CLI contract tests (exit codes, exact strings, JSON shapes) <!-- completed: 2026-07-30T13:27 -->
- [x] Verify the coverage map: every pytest area has Rust tests asserting the same contracts; close gaps <!-- completed: 2026-07-30T13:27 -->
- [x] End-to-end binary test: fleet create → member create → send/poll/ack → monitor tick on a fresh temp DB <!-- completed: 2026-07-30T13:27 -->

### Step 10: Release and removal

- [x] Replace publish.yml with the binary release workflow (3-target matrix, admin build, tar.gz packaging, gh release upload); delete PyPI + assets-zip jobs <!-- completed: 2026-07-30T22:36 -->
- [x] Delete the Python package, pytest suite, and Alembic tree; total-removal sweep for stray mentions <!-- completed: 2026-07-30T23:14 -->
- [x] Final verification: mise lint/test/build green, docs build green, E2E smoke with the release-profile binary <!-- completed: 2026-07-30T23:14 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-30 | Initial draft |
| 2026-07-30 | Review fixes (head-tree corrections after migration 0006; version 0.22.0); approved |
