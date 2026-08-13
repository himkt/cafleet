# Custom Config Paths: Env-Var Resolution, Path-Aware Install Records, Setup Selector, and Doctor Redesign

**Status**: Approved
**Progress**: 21/24 tasks complete
**Last Updated**: 2026-08-12

## Overview

The CLI hard-codes each coding-agent backend's user-level config directories when installing and checking assets. This design resolves those directories through the backends' native config-location environment variables (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`), records the install path in `asset_installs` so install status is computed per (coding agent, resolved path), replaces `cafleet setup --skip` with a `--coding-agent` selector, and redesigns `cafleet doctor` into a three-section, no-early-abort diagnosis report (GitHub issue #296).

## Success Criteria

- [ ] `cafleet setup` installs skills and presets under the env-var-resolved directories when the variables are set (opencode skills excepted — their install dir is a fixed discovery path), and under today's defaults when unset.
- [ ] `cafleet member create --coding-agent opencode` checks the preset-existence spawn precondition at the same resolved path `setup` installs to.
- [ ] A set variable whose value is not an absolute path (including the empty string) fails loudly with the pinned error message and exit 1.
- [ ] `asset_installs` is keyed on `(coding_agent, path)`; migration `V6` recreates the table with the composite key and the chain-guard test names head 6.
- [ ] `cafleet setup --coding-agent AGENT` (repeatable) selects the agents to install; `--skip` is gone from the CLI, every doc, every skill, and every test; the no-flag default refreshes only agents recorded at their currently-resolved paths.
- [ ] `cafleet doctor` renders all three sections (multiplexer, database, coding agents) without early abort, frames the coding-agents table with display-width alignment, keys the setup column on the resolved path only, exits non-zero iff any issue, and mirrors the report in `--json`.
- [ ] The stale-assets guard checks only each agent's row at its currently-resolved path; superseded rows never block fleet-scoped commands.
- [ ] `SPEC.md`, `docs/docs/spec/cli-options.md`, `docs/docs/spec/data-model.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/quickstart.md`, `docs/docs/concepts/storage.md`, `docs/docs/contributing.md`, `skills/cafleet/reference/cli.md`, `skills/cafleet/reference/supervision.md`, and `.claude/rules/database-migrations.md` describe the new contracts.
- [ ] Unit tests cover the resolver (all four outcomes per variable), the migration chain, the guard, the setup selector, the opencode precondition, and the doctor renderings via injected lookups.

---

## Background

Three sites compute backend config-dir paths today, all hard-coded against `$HOME`:

| Site | Location | Paths |
|---|---|---|
| `setup` skills install | `cafleet/src/assets.rs` `skills_dir` | `~/.claude/skills`, `~/.codex/skills`, `~/.config/opencode/skills` |
| `setup` preset install | `cafleet/src/assets.rs` `preset` | `~/.codex/rules/cafleet.rules`, `~/.opencode/agents/cafleet.md` |
| opencode spawn precondition | `cafleet/src/coding_agent/opencode.rs` `ensure_available` | `~/.opencode/agents/cafleet.md` |

Backend variable semantics, per each backend's official docs: `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate the whole config directory (`~/.claude` / `~/.codex`). opencode's `OPENCODE_CONFIG` names a config **file**; the directory-shaped variable is `OPENCODE_CONFIG_DIR`, documented as "searched for agents, commands, modes, and plugins just like the standard `.opencode` directory" — skills are absent from that list, and opencode's skills docs enumerate only fixed discovery paths (`~/.config/opencode/skills` among the user-level ones). cafleet therefore reads `OPENCODE_CONFIG_DIR` for the preset base only, keeps the opencode skills install at its fixed discovery path, and ignores `OPENCODE_CONFIG`.

Current state of the surrounding surfaces:

- `asset_installs` is keyed on `coding_agent` alone (`coding_agent TEXT PRIMARY KEY, cafleet_version, installed_at`), so a record cannot distinguish installs at different locations.
- `cafleet setup` takes only `--skip AGENT`; the documented schema-only invocation is `--skip claude --skip codex --skip opencode`.
- `cafleet doctor` aborts before printing anything when the multiplexer fails to resolve, and its assets report is a flat per-agent version list with no path awareness.
- The stale-assets guard errors when any recorded row's version differs from the CLI version, regardless of location.
- The cafleet skill's supervision reference documents `cafleet doctor` as the gating pre-spawn env check: a non-zero exit aborts the spawn protocol.

The agent-executed base-dir resolution procedure in the cafleet skill (its user-level config-dir table) is out of scope: this design touches only the CLI.

---

## Specification

### Part 0 — Config-dir resolution

#### Resolution table

| Backend | Variable | Base when set | Base when unset | Skills dir | Preset target |
|---|---|---|---|---|---|
| claude | `CLAUDE_CONFIG_DIR` | `$CLAUDE_CONFIG_DIR` | `~/.claude` | `<base>/skills` | — |
| codex | `CODEX_HOME` | `$CODEX_HOME` | `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` |
| opencode (skills) | — (fixed discovery path) | — | `~/.config/opencode` | `<base>/skills` | — |
| opencode (preset) | `OPENCODE_CONFIG_DIR` | `$OPENCODE_CONFIG_DIR` | `~/.opencode` | — | `<base>/agents/cafleet.md` |

opencode splits by purpose because its discovery rules differ. `agents/` is in `OPENCODE_CONFIG_DIR`'s documented search list, so the preset may relocate to `<custom>/agents/cafleet.md` and remain a valid `--agent cafleet` discovery path; when the variable is unset it keeps today's `~/.opencode/agents/cafleet.md` (SPEC §6.7's mandated discovery path). Skills are not in that search list — opencode discovers them only at fixed paths, of which `~/.config/opencode/skills` is cafleet's user-level install target — so the skills install ignores the variable.

#### Validation

A set variable must hold an absolute path. Any other value — the empty string, a relative path, a literal unexpanded `~/…` — fails at resolution time with:

```
Error: <VAR> must be an absolute path (got '<value>')
```

Exit 1 (`CafleetError::App`, matching the `CAFLEET_*` numeric-validation style in `config.rs`). Validation is lazy: a variable is read and validated only when a site actually resolves that backend's directory. `cafleet setup --coding-agent claude` with an invalid `CODEX_HOME` succeeds; `member create --coding-agent claude` never reads any of the three variables (claude and codex spawn preconditions are PATH-check-only). One exception to strict lazy failure: `doctor` catches per-agent resolution errors and renders them as issues instead of aborting (Part 3).

#### The shared resolver

New module `cafleet/src/config_dir.rs`, the single owner of backend config-dir resolution. Each env-resolving function takes an injected env lookup (the `Settings::from_lookup` pattern) and the home directory, so tests run against fakes; `opencode_skills_base` reads no variable and stays infallible. Resolution also reports the winning origin, which `doctor`'s `source` column and `--json` need:

```rust
type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

pub enum DirSource {
    EnvVar(&'static str), // the variable name that supplied the path
    Default,
}

pub struct ResolvedDir {
    pub path: PathBuf,
    pub source: DirSource,
}

pub fn claude_config_dir(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError>;
// CLAUDE_CONFIG_DIR | home/.claude

pub fn codex_home(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError>;
// CODEX_HOME | home/.codex

pub fn opencode_skills_base(home: &Path) -> PathBuf;
// always home/.config/opencode — opencode discovers skills only at fixed paths

pub fn opencode_preset_base(env: EnvLookup, home: &Path) -> Result<ResolvedDir, CafleetError>;
// OPENCODE_CONFIG_DIR | home/.opencode
```

A private `resolve(var: &'static str, env: EnvLookup, default: PathBuf)` implements the shared behavior: unset → `Default` + the default path; set and absolute → `EnvVar(var)` + `PathBuf::from(value)`; set otherwise → the pinned error. The three variables stay out of `Settings` — `Settings` is the `CAFLEET_*` configuration surface, while these are backend-native variables read at point of use.

#### Recorded-path identity per agent

Every surface that keys on "the agent's resolved path" uses one canonical path per agent — the resolved **base** directory:

| Agent | Recorded / status-keyed path |
|---|---|
| claude | `claude_config_dir(...)` (e.g. `/Users/x/.claude` or `$CLAUDE_CONFIG_DIR`) |
| codex | `codex_home(...)` |
| opencode | `opencode_preset_base(...)` — the only opencode root that can vary; the skills base is fixed and carries no identity |

Paths are stored absolute, exactly as resolved (no canonicalization beyond the absolute-path validation).

### Part 1 — Path-aware `asset_installs`

#### Migration `V6__path_aware_asset_installs.sql`

A `PRIMARY KEY` change has no in-place `ALTER TABLE` form in SQLite, and `asset_installs` has no FK parents, so drop-and-recreate is the legitimate path under `.claude/rules/database-migrations.md`. The table starts empty — pre-existing rows are **not** carried over, because preservation is impossible rather than skipped: the old rows' install locations were the `$HOME`-dependent hard-coded defaults, and a plain-SQL migration cannot know `$HOME` to backfill a truthful `path` value, so an empty restart is the only non-fabricated option. Every machine re-runs `setup` per agent after upgrading; no "legacy install, location unknown" state exists anywhere downstream.

```sql
DROP TABLE asset_installs;

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);
```

The chain-guard test in `cafleet/src/db/mod.rs` bumps its expected head from 5 to 6 (test rename included), and the `cli_setup_doctor.rs` head-message assertions follow (`applied migrations to head (6).` / `Already at head (6); nothing to do.`).

#### Broker API (`cafleet/src/broker/asset_installs.rs`)

- `record_asset_install(conn, coding_agent, path, cafleet_version)` — upsert `ON CONFLICT(coding_agent, path) DO UPDATE SET cafleet_version, installed_at`. `path` is the agent's recorded-path identity (absolute string).
- `list_asset_installs(conn)` — rows gain `"path"`; ordering becomes ascending `(coding_agent, path)`.
- `asset_installs_table_exists` is unchanged.

Consumers partition an agent's rows by comparing `path` against the currently-resolved identity path: the row at the resolved path (at most one, by the primary key) is **current**; every other row of that agent is **superseded**.

### Part 2 — `cafleet setup` selector

#### CLI surface

`--skip` is removed entirely. The one flag becomes:

| Flag | Required | Notes |
|---|---|---|
| `--coding-agent AGENT` | no | Repeatable. A choice over `claude` / `codex` / `opencode`; an unknown value fails with clap's native invalid-value error (exit 2). Duplicates are deduplicated. Help: `Install the named agent's assets (repeatable; default: refresh agents already installed at their resolved paths).` |

The db half is unchanged and always runs first (same refusal states, same messages). The assets half:

| Invocation | Assets-half behavior |
|---|---|
| `--coding-agent` given (one or more) | Install exactly the named agents, in the fixed order `claude`, `codex`, `opencode`, each at its resolved paths; upsert the `(agent, resolved identity path)` row after that agent's skills and preset (where one exists) install successfully. |
| No flag, table has rows | Per agent in the fixed order: a row exists at the resolved identity path → reinstall (refresh) and upsert; rows exist only at other paths → print the hint line below, install nothing; no rows for that agent → nothing. |
| No flag, table is empty (first-ever run) | Install nothing; print the guidance line below. Counts as the assets half succeeding — exit 0 when the db half succeeds. |

Validation applies uniformly wherever the assets half resolves an agent's identity path — a targeted agent in the selector form, and any agent being classified in the no-flag-with-rows form: a config-path validation failure fails the assets half with the pinned error as `<msg>` (`assets half failed: <msg>`, exit 1; the db half's outcome is unaffected). The no-flag empty-table form resolves nothing, so it cannot fail validation.

Guidance line (empty table, no flag):

```
no assets install recorded; run 'cafleet setup --coding-agent <agent>' to install (agents: claude, codex, opencode)
```

Hint line (agent recorded only at other paths, no flag; `<resolved>` / `<old>` render with `~` abbreviation for `$HOME`; one line per such agent). `<old>` is the agent's superseded row with the greatest `installed_at`, ties broken by ascending `path`:

```
<agent>: no install at <resolved> (previously set up at <old>); run 'cafleet setup --coding-agent <agent>'
```

The install procedure, echo lines, and failure messages keep their exact shapes and now print the resolved directories:

```
<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```

An install failure still aborts the loop; rows recorded before the failure remain. The `assets half skipped (all agents skipped)` outcome no longer exists.

#### `--skip` removal (complete, same change)

Per `.claude/rules/removal.md`, every mention of `--skip` goes in this change. The schema-only invocation is replaced by plain `cafleet setup`: on a records-free database it is naturally schema-only (db half runs; assets half installs nothing and prints the guidance line), and on a machine with records the idempotent asset refresh is acceptable. Affected surfaces:

| Surface | Change |
|---|---|
| `cafleet/src/cli/setup.rs` | `skip: Vec<String>` → `coding_agent: Vec<String>`; assets-half selection logic above. |
| `docs/docs/spec/cli-options.md` § `cafleet setup` | Flag row, assets-half targets, the `#schema-only` section (plain `cafleet setup` is the documented migrations-apply path — no dedicated db-only flag), the failure-modes table row for all-skipped. |
| `docs/docs/concepts/storage.md` | Schema-only invocation → `cafleet setup`. |
| `docs/docs/contributing.md` | Schema-only command → `cafleet setup`. |
| `.claude/rules/database-migrations.md` | Apply-migration recipe → `cafleet setup`. |
| `SPEC.md` §5.2, §6.3, §8, compliance checklist | Flag, targets-minus-skipped phrasing, schema-only invocation, choice-set note. |
| `cafleet/tests/common/mod.rs`, `cafleet/tests/cli_setup_doctor.rs` | The schema-only helper and every `--skip` invocation; seeded `asset_installs` rows gain `path`. |

#### Stale-assets guard (path-aware)

The guard (`cafleet/src/cli/helpers.rs`, every fleet-scoped surface) resolves each agent's identity path per Part 0 and checks only the row at that path:

| Recorded install state | Result | Exit |
|---|---|---|
| A config-path variable fails validation | The pinned validation error | 1 |
| No agent has a row at its currently-resolved path (DB/table/rows missing included) | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup --coding-agent <agent>' to install (agents: claude, codex, opencode)` — `<agent>` is shown literally, resolved by the trailing enumeration, exactly as in the setup guidance line | 1 |
| A row at a resolved path has `cafleet_version` ≠ the runtime CLI version (string inequality — either direction) | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall`, stale agents in ascending order | 1 |
| Every row at a resolved path matches | The command proceeds silently | 0 |

Agents with no row at their resolved path are not checked (they contribute nothing to staleness); superseded rows at other paths are ignored everywhere in the guard. Plain `cafleet setup` remains the correct stale remedy: a stale agent by definition has a row at its resolved path, so the no-flag refresh covers it. The exempt surfaces (`setup`, `doctor`, `server`) are unchanged.

### Part 3 — `cafleet doctor` redesign

`doctor` becomes a full-environment diagnosis that renders **all** sections even when the multiplexer is unavailable or the database is missing or stale — no early abort. Diagnosis order: multiplexer, database, coding agents. It remains exempt from the stale-assets guard.

#### Text layout

The first output line of the whole report is `cafleet <version>`. Each section is led by a single-width verdict glyph (`✓` U+2713 / `✗` U+2717) plus the section name; detail lines are indented two spaces beneath. A worked example (behind-head schema, one stale agent, one superseded record):

```
cafleet 0.22.0
✓ multiplexer
  backend:   tmux
  session:   main
  window_id: @3
  pane_id:   %0
  presence:  TMUX=/tmp/tmux-501/default,12345,0
✗ database
  schema 10, head is 12 — run: cafleet setup
✗ coding agents
  ┌──────────────┬──────────────┬────────────────────┬───────────────────────────────────────────────┐
  │ coding agent │ path         │ source             │ setup                                         │
  ├──────────────┼──────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ claude       │ ~/cfg/claude │ $CLAUDE_CONFIG_DIR │ ✓ 0.22.0                                      │
  │ codex        │ ~/.codex     │ default            │ ✗ 0.21.0 → cafleet setup --coding-agent codex │
  │ opencode     │ ~/.opencode  │ default            │ – cafleet setup --coding-agent opencode       │
  └──────────────┴──────────────┴────────────────────┴───────────────────────────────────────────────┘
  note: codex was previously set up at ~/.codex-old
2 issues found
```

#### Multiplexer section

`✓` with the five detail lines (`backend`, `session`, `window_id`, `pane_id`, `presence` — today's fields). On any multiplexer or environment failure (no supported multiplexer, ambiguous environment, binary not on `PATH`, pane not discoverable): `✗ multiplexer` with the resolver's error message as the single detail line, and the report continues. One issue.

#### Database section

One detail line; the five states (`<M>` recorded version, `<N>` embedded head):

| State | Glyph | Detail line | Issue |
|---|---|---|---|
| Ledger present, `<M>` = `<N>` | `✓` | `schema <N> (head)` | no |
| Ledger present, `<M>` < `<N>` | `✗` | `schema <M>, head is <N> — run: cafleet setup` | yes |
| Ledger present, `<M>` > `<N>` | `✗` | `schema <M> is newer than this CLI (head <N>) — upgrade cafleet` | yes |
| Ledger absent, foreign tables present | `✗` | `database has tables but no schema history — not a cafleet database?` | yes |
| Ledger absent, no tables (or no DB file) | `✗` | `no database — run: cafleet setup` | yes |

A connection failure (unreadable path) renders `✗` with the connection error as the detail line (one issue). A `✗` database never suppresses the coding-agents section: the recorded rows are read when the `asset_installs` table exists (the ledger-absent-with-tables state included), and a missing table renders every agent as the `–` state.

#### Coding agents section

A light box-drawing framed table (`┌ ─ ┬ ┐ │ ├ ┼ ┤ └ ┴ ┘`), header separator only, no per-row rules. Column alignment uses **display width** via the `unicode-width` crate (new dependency), never byte length — the glyphs and `→` are single-width, but the rule is general. One row per agent in the fixed order `claude`, `codex`, `opencode`.

| Column | Content |
|---|---|
| `coding agent` | The agent name. |
| `path` | The resolved identity path (Part 0), with `~` abbreviation when under `$HOME`. On a resolution error: the raw invalid variable value. |
| `source` | The winning origin: `$<VAR>` (e.g. `$CLAUDE_CONFIG_DIR`) or `default`. |
| `setup` | The three-valued state below, keyed on the **resolved path only**. |

| State | Cell | Issue |
|---|---|---|
| A row exists at the resolved path, version = CLI version (string equality) | `✓ <version>` | no |
| A row exists at the resolved path, version ≠ CLI version (string inequality, either direction — never semver comparison) | `✗ <recorded-version> → cafleet setup --coding-agent <agent>` | yes |
| No row exists at the resolved path (regardless of rows elsewhere) | `– cafleet setup --coding-agent <agent>` (`–` U+2013 EN DASH) | never |
| The agent's config-path variable fails validation (per C3: caught per-agent, not fatal) | `✗ <VAR> is not an absolute path` | yes |

Records at other paths only feed informational footnote lines under the table, one per superseded row, ordered ascending `(coding_agent, path)`, `~`-abbreviated:

```
note: <agent> was previously set up at <path>
```

Footnotes are informational — they never count as issues.

#### Footer and exit code

Last line: `no issues found`, `1 issue found`, or `<N> issues found` (proper pluralization). Exit code: 0 when no issues, 1 otherwise — the `–` state and footnotes never count. The old behavior of exiting before any output on multiplexer failure is gone; every failure is a rendered issue.

#### `--json`

Mirrors the sections with `ok` booleans, unabbreviated absolute paths, and the issue count. `source` holds the winning env-var **name** (no `$`) or the literal `"default"`; `state` is `"ok" | "stale" | "not_installed" | "error"` (`"error"` is the C3 per-agent resolution-error extension; `"not_installed"` never contributes to `issues`). Every agent row carries the same keys; `error` is `null` except in the `"error"` state, where it holds the pinned validation message (without the `Error: ` prefix). Section `error` fields likewise hold the detail text when `ok` is false, else `null`.

```json
{
  "multiplexer": {
    "ok": true,
    "backend": "tmux",
    "session": "main",
    "window_id": "@3",
    "pane_id": "%0",
    "presence_var": "TMUX",
    "presence_value": "/tmp/tmux-501/default,12345,0",
    "error": null
  },
  "database": {
    "ok": false,
    "schema_version": 10,
    "head_version": 12,
    "error": "schema 10, head is 12 — run: cafleet setup"
  },
  "coding_agents": {
    "ok": false,
    "cli_version": "0.22.0",
    "agents": [
      {"coding_agent": "claude", "path": "/Users/x/cfg/claude", "source": "CLAUDE_CONFIG_DIR", "recorded_version": "0.22.0", "installed_at": "2026-08-12T00:00:00.000000+00:00", "state": "ok", "error": null},
      {"coding_agent": "codex", "path": "/Users/x/.codex", "source": "default", "recorded_version": "0.21.0", "installed_at": "2026-08-01T00:00:00.000000+00:00", "state": "stale", "error": null},
      {"coding_agent": "opencode", "path": "/Users/x/.opencode", "source": "default", "recorded_version": null, "installed_at": null, "state": "not_installed", "error": null}
    ],
    "superseded": [
      {"coding_agent": "codex", "path": "/Users/x/.codex-old", "recorded_version": "0.20.0", "installed_at": "2026-07-01T00:00:00.000000+00:00"}
    ]
  },
  "issues": 2
}
```

On a multiplexer failure the `multiplexer` object is `{"ok": false, "backend": null, "session": null, "window_id": null, "pane_id": null, "presence_var": null, "presence_value": null, "error": "<message>"}`. On an agent resolution error the row is `{"coding_agent": "...", "path": null, "source": "<VAR>", "recorded_version": null, "installed_at": null, "state": "error", "error": "<VAR> must be an absolute path (got '<value>')"}` (the raw invalid value appears only inside `error` — `path` stays `null` because no path resolved). `schema_version` is `null` when the ledger is absent. Exit-code semantics are identical to text mode.

### Part 4 — opencode spawn precondition

The `SpawnProbe` trait gains `fn env_var(&self, name: &str) -> Option<String>`. `SystemProbe` (`cli/system.rs`) reads `std::env::var(name).ok()`; the test-support `FakeProbe` gains a settable env map defaulting to empty. `opencode::ensure_available` resolves the preset as `opencode_preset_base(...)?.path.join("agents/cafleet.md")`, and an invalid `OPENCODE_CONFIG_DIR` surfaces the validation error before the existence check. The precondition error becomes `opencode agent preset not found at <preset>; run 'cafleet setup --coding-agent opencode' first` — the path is now the resolved one, and the remedy names the selector because the triggering state (no preset at the resolved path) coincides with opencode having no row at its resolved identity path, where plain `cafleet setup` installs nothing for opencode.

### Part 5 — Supervision gate interaction

`skills/cafleet/reference/supervision.md` documents `cafleet doctor` as the gating pre-spawn env check (its § pre-spawn env-check step and its command-catalog row): non-zero exit ⇒ abort the spawn protocol. Under the new semantics `doctor` exits non-zero on **any** rendered issue — a multiplexer failure, a database-schema issue, or a stale/invalid coding-agent state — so the gate becomes deliberately stricter: it now catches, pre-spawn, what the stale-assets guard would reject at `member create` anyway, plus a behind-head schema. The `–` (not-installed-at-resolved-path) state never fails the gate, and nothing rejects it at `member create` either: per the Part 2 guard table an agent with no row at its resolved path is unchecked, the guard errors only when **no** agent has a row at its resolved path, and only opencode carries its own preset-existence precondition — so claude/codex members on a partially-installed machine spawn with missing skills, and `doctor`'s `–` remedy cell is the surface that points at the missing install. Both supervision.md locations are updated to describe the broadened failure set rather than scoping the gate to the multiplexer section.

### Documentation deltas

| Surface | Delta |
|---|---|
| `docs/docs/spec/cli-options.md` | § `cafleet setup`: the `--coding-agent` flag row, the no-flag/selector semantics table, the guidance and hint lines, the resolution table, the validation error; `#schema-only` becomes the plain-`cafleet setup` migrations-apply path; the assets-half preset table switches to resolved defaults (opencode skills dir stays fixed). § stale-assets guard: the path-aware rules and the two updated error strings. § `cafleet doctor`: full rewrite — layout mock, section states, table spec, footnote, footer, exit code, `--json` schema. § `member create`: the opencode precondition resolves through `OPENCODE_CONFIG_DIR` and its error string gains the `--coding-agent opencode` remedy (Part 4). The `CAFLEET_*` env table stays CAFLEET-only. |
| `docs/docs/spec/data-model.md` | `asset_installs`: composite key, `path` column, the current-vs-superseded partition, the guard/doctor consumers. |
| `docs/docs/spec/coding-agent-backends.md` | The codex rules-file and opencode preset path mentions note the `CODEX_HOME` / `OPENCODE_CONFIG_DIR` override. |
| `docs/docs/spec/multiplexer-backends.md` | The `doctor` pointer sentence stays accurate (reports the resolved backend); verify no exit-semantics restatement drifts. |
| `docs/docs/quickstart.md` | The per-backend table's fixed paths gain a note that the backend config-location variables relocate them. |
| `docs/docs/concepts/storage.md` | Schema-only invocation → `cafleet setup`; the stale-guard prose reflects path-keyed checking; the `doctor` pointer stays. |
| `docs/docs/concepts/overview.md` | The `doctor` row's description covers the three-section diagnosis (environment check wording stays behavior-level). |
| `docs/docs/contributing.md` | Schema-only command → `cafleet setup`. |
| `SPEC.md` | §5.2 *AssetInstalls* (composite key, path identity per agent, recreate migration); §6.3 `setup` (flag, selector semantics, guidance/hint lines, schema-only), stale-assets guard (path-aware rules + error strings), `doctor` (whole section: layout, states, table, footnote, footer, exit codes, JSON key order); §6.7 opencode `ensure_available`, the opencode-preset and codex-rules sections, the `SpawnProbe` seam; §8 schema DDL + migration chain (V6) + the compliance checklist lines. |
| `skills/cafleet/reference/cli.md` | The `doctor` entry and Typical Workflow step 0 describe the three-section report and the any-issue non-zero exit. |
| `skills/cafleet/reference/supervision.md` | Both gate mentions describe the broadened failure set (Part 5). |
| `.claude/rules/database-migrations.md` | The apply-migration recipe becomes plain `cafleet setup`. |

No `SKILL.md` names an install path, and the one skills-tree mention — the opencode discovery hint in `skills/cafleet-research/reference/slidev.md`, naming `~/.config/opencode/skills/` — stays accurate because this design keeps that directory fixed. `README.md`'s thin surface is untouched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/docs/spec/cli-options.md` per the documentation-deltas table (setup, stale-assets guard, doctor, member create) <!-- completed: 2026-08-13T07:58 -->
- [x] Update `docs/docs/spec/data-model.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/spec/multiplexer-backends.md`, `docs/docs/quickstart.md` <!-- completed: 2026-08-13T08:01 -->
- [x] Update `docs/docs/concepts/storage.md`, `docs/docs/concepts/overview.md`, `docs/docs/contributing.md` <!-- completed: 2026-08-13T08:03 -->
- [x] Update `SPEC.md` §5.2, §6.3, §6.7, §8, and the compliance checklist <!-- completed: 2026-08-13T08:12 -->
- [x] Update `skills/cafleet/reference/cli.md` and `skills/cafleet/reference/supervision.md` (broadened doctor gate) <!-- completed: 2026-08-13T08:15 -->
- [x] Update `.claude/rules/database-migrations.md` (plain `cafleet setup` as the apply path) <!-- completed: 2026-08-13T08:10 -->

### Step 2: Migration

- [x] Add `cafleet/migrations/V6__path_aware_asset_installs.sql` (drop-and-recreate with the composite key) <!-- completed: 2026-08-13T08:20 -->
- [x] Bump the chain-guard test to head 6; update the `cli_setup_doctor.rs` head-message assertions and the `tests/common/mod.rs` seeded rows/helpers <!-- completed: 2026-08-13T08:20 -->

### Step 3: Shared resolver

- [x] Add `cafleet/src/config_dir.rs` with `ResolvedDir`/`DirSource`, the four public functions, and the private `resolve` <!-- completed: 2026-08-13T08:38 -->
- [x] Colocated tests: per variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`) — unset → default + `Default` source, absolute → used verbatim + `EnvVar` source, empty → pinned error, relative → pinned error; `opencode_skills_base` pinned to `home/.config/opencode` <!-- completed: 2026-08-13T08:38 -->

### Step 4: Broker rows and stale-assets guard

- [x] Extend `record_asset_install`/`list_asset_installs` with `path`; update the colocated tests (composite upsert, ordering) <!-- completed: 2026-08-13T08:29 -->
- [x] Rewrite `stale_assets_guard` per the path-aware rules table (resolution errors, the updated no-install error, resolved-path-only staleness) <!-- completed: 2026-08-13T08:47 -->
- [x] Guard tests: superseded rows are ignored; stale-at-resolved-path fails; no-row-at-resolved-path is unchecked; the all-agents-uninstalled error fires <!-- completed: 2026-08-13T08:47 -->

### Step 5: Setup selector

- [x] Replace `--skip` with repeatable `--coding-agent` in `cli/setup.rs`; implement the selector/no-flag semantics, the guidance line, and the hint lines; route `assets.rs` `skills_dir`/`preset` through `config_dir` and record `(agent, path)` rows <!-- completed: 2026-08-13T08:57 -->
- [x] Tests: explicit selector installs only named agents at resolved paths; no-flag refreshes only recorded-at-resolved-path agents; empty table prints the guidance line and exits 0; records-elsewhere prints the hint line; invalid variable fails the assets half with the pinned error; opencode skills stay at `~/.config/opencode/skills` even when `OPENCODE_CONFIG_DIR` is set <!-- completed: 2026-08-13T08:57 -->
- [x] Remove every remaining `--skip` mention from tests and helpers <!-- completed: 2026-08-13T08:57 -->

### Step 6: opencode spawn precondition

- [x] Add `SpawnProbe::env_var`; implement on `SystemProbe` and `FakeProbe` <!-- completed: 2026-08-13T09:06 -->
- [x] Resolve the preset path in `opencode::ensure_available` via `opencode_preset_base`; tests for the custom-dir check, the default-dir check, and the invalid-variable error <!-- completed: 2026-08-13T09:06 -->

### Step 7: Doctor

- [x] Add the `unicode-width` dependency; rewrite `cli/doctor.rs`: three sections, no early abort, verdict glyphs, database states, the framed table with display-width alignment, footnotes, footer, exit code <!-- completed: 2026-08-13T09:27 -->
- [x] Implement the `--json` payload per the pinned schema (key order, null contracts, exit parity) <!-- completed: 2026-08-13T09:27 -->
- [x] Tests: each database state; each setup-cell state including the per-agent resolution error; superseded footnotes; multiplexer-failure rendering; issue counting and exit codes; frame alignment with a multi-width path; JSON shape <!-- completed: 2026-08-13T09:27 -->

### Step 8: Verification

- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` all pass <!-- completed: -->
- [ ] `rg -- --skip` over docs, skills, rules, source, and tests returns no setup-flag mentions <!-- completed: -->
- [ ] Confirm the supervision gate doc matches the shipped exit semantics and the docs build renders the new tables intact <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-12 | Rewritten in place with expanded scope: path-aware `asset_installs` (V6), `--coding-agent` setup selector replacing `--skip`, three-section `doctor` redesign |
