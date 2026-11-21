# Rust build and dependency slimming

**Status**: Approved
**Progress**: 0/37 tasks complete
**Last Updated**: 2026-11-21

## Overview

A Go reimplementation of CAFleet was evaluated and is not pursued: its build, dependency and test advantages are mostly reachable inside Rust, and the port would make the implementation more complex. This design applies the cheap Rust-side changes the evaluation identified. It replaces `refinery` with a hand-written baseline ledger, drops `libproc` for a `sysctl` process-group read, trims `chrono` features, tunes the dev/test profiles, runs CI at test opt-level 1 and on macOS, and moves the release to one runner OS if a validation gate passes.

## Success Criteria

- [ ] `refinery` and `libproc` are absent from `Cargo.toml` and `Cargo.lock`. `chrono` is declared with `default-features = false, features = ["std", "now"]`. `Cargo.lock` contains no `bindgen`, `clang-sys`, `iana-time-zone` or `core-foundation-sys` package.
- [ ] Each crate count in § Dependency-graph gates equals the pruned count that Step 1 recorded on the same lockfile. On the 2026-11-21 lockfile the expected values are Linux normal 75, Linux normal+build 83 and darwin normal+build 82, counting the `cafleet` line.
- [ ] `db::initialize` applies the baseline, the ledger row and the version row in one `BEGIN IMMEDIATE` transaction, and only to an empty file (identity 0, no non-`sqlite_%` table). A file whose ledger is exactly `(1, 'baseline')` is unchanged. Every other file fails with the incompatible-format error naming the database URL's path and stays unchanged. Two concurrent `initialize` calls on one fresh file both succeed and leave exactly one ledger row and one version row.
- [ ] The ledger row is `(1, 'baseline', <canonical timestamp>, <SHA-256 hex of the embedded V1__baseline.sql>)`, and the ledger DDL is part of `assets/migrations/V1__baseline.sql` and `SPEC.md` §8 exact DDL.
- [ ] On macOS, `group_members` lists a process group through `sysctl(CTL_KERN, KERN_PROC, KERN_PROC_PGRP)`. The crate's only unsafe item is the one `kern.proc` sysctl helper. Permanent macOS CI entries run `mise //:lint` and the full suite, and both pass.
- [ ] Dev/test profiles build `cafleet` with `debug = "line-tables-only"`, and `libsqlite3-sys` at the opt-level the suite-regression check in § Measurements selects: 0 when it does not regress, otherwise 1, otherwise no override. § Measurements records the selection and its runs, and the full suite passes.
- [ ] CI runs `mise //:test` with `CARGO_PROFILE_TEST_OPT_LEVEL=1` on its Linux and macOS test entries only; `mise //:lint` and local `mise //:test` keep the dev/test profiles.
- [ ] The single-runner release gate (§ Gated single-runner release) is run and its result recorded. If it passes, `release.yml` builds all three targets with `cargo zigbuild` on `ubuntu-24.04`. If it fails, `release.yml` keeps the `macos-15` runner.
- [ ] § Measurements holds the before/after values for every metric in its table.
- [ ] `SPEC.md`, `docs/docs/contributing.md` and `CLAUDE.md` describe the result; `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

### Go reimplementation evaluation

A Director-led team evaluated a Go rewrite on 2026-11-21, at repository HEAD `4e8418f8` (cafleet 0.35.4). Six reports covered runtime and core ports, dependencies, build, test speed and risk, and an adversarial cross-review reconciled them. The reconciled verdicts:

| Question | Verdict for Go | Confidence | Key evidence |
|---|---|---|---|
| (a) Implementation simplicity | worse | medium | A port is 0.94–1.26× of today's 139.4k Rust lines (37,560 production). Go also needs a VT emulator fork, because `charmbracelet/x/vt` lacks per-row wrap state and blocks on device-attribute queries. Other port gaps: a U+2028/U+2029 JSON escaping workaround, cobra exit-code glue and 4 regex rewrites. 110 enums and 475 `match` sites lose compile-time exhaustiveness. |
| (b) Build simplicity | better at the end state with pure-Go SQLite; worse during any incremental migration | medium-high | A Go stack cold-builds in about 21–23 s and cross-builds darwin with no C toolchain. Rust takes 87–229 s cold, and its release needs zig, cargo-zigbuild and a macOS runner. Rust can drop libclang (`libproc` → `bindgen`) and probably the macOS runner, but it keeps a C toolchain for bundled SQLite. |
| (c) Third-party dependencies | better, but mostly reachable in Rust | high (direction), medium (magnitude) | 23–28 linked Go modules versus 121 Rust crates (normal edges, Linux). Removing `refinery` alone removes 46 crates. The remaining gap is partly crate granularity, and the Go VT emulator alone adds 16–17 modules. |
| (d) Test speed | better for compile-bound cycles; same for suite execution | medium | Incremental test build: Rust 12.7–13.1 s versus Go 0.6–2 s at prototype scale. At least 137 s of the suite's waits are wall-clock contract budgets and do not depend on the language. The CPU-bound share is an opt-level-0 artefact: at opt-level 1 the suite drops from 60–69 s to 47.5 s. Go `regexp` is 6.9–83× slower than Rust `regex` on the 119 catalog patterns. |

**Decision: the Go rewrite is not pursued.** It would make the implementation more complex, and the changes in this design capture most of its build, dependency and test advantages for about 2–3 days of work. What remains Go-only is accepted: faster incremental compiles (Rust ≈10 s versus Go ≈1–2 s), no C compiler for SQLite, and a single toolchain.

### Current state

| Area | Today | Cost |
|---|---|---|
| Schema ledger | `refinery` 0.10 runs the one embedded `V1__baseline` migration grouped in one transaction and creates `refinery_schema_history` itself (`src/db.rs` `baseline`, `initialize`) | 46 crates used only by `refinery`; 107.9 of 482.8 unit-seconds (22%) of the cold test build |
| macOS process group | `libproc::processes::pids_by_type(ByProgramGroup)` in `src/runtime/process.rs` `group_members`; a test-only `pids_by_path`/`pidinfo` diagnostic in `src/runtime/press.rs` `post_refusal_lock_diagnostic` | `libproc` runs `bindgen`, which needs libclang on macOS; 9 extra darwin crates |
| Dev/test profile | `[profile.dev.package."*"] opt-level = 2` compiles `sqlite3.c` at `-O2`; `cafleet` has full debuginfo | `sqlite3.c` compiles in 26.05 s at `-O2` versus 4.76 s at `-O0`, and the `libsqlite3-sys` build script took 62 s of the 117 s cold critical path under load. Incremental test build 12.7–13.1 s; test binary 238 MB |
| CI tests | `mise //:test` on `ubuntu-latest` at opt-level 0; no macOS job | CPU-bound tests run 2.8–4.6× slower than at opt-level 1; macOS-only code (`sysctl`, setuid group member) is never exercised in CI |
| Release | 3-entry matrix: darwin on `macos-15` with `cargo build`, two musl targets on `ubuntu-24.04` with `cargo zigbuild` | Two runner OSes and a musl/non-musl branch; about 3 min end to end |

---

## Specification

### Scope

| Change | In scope |
|---|---|
| Hand-written baseline ledger replacing `refinery` | yes |
| `libproc` removal through `sysctl` `KERN_PROC_PGRP` | yes |
| `libsqlite3-sys` opt-level 0 (subject to the suite-regression check) and `cafleet` `debug = "line-tables-only"` in the dev profile | yes |
| CI-only test opt-level 1, and permanent macOS `lint` and `test` entries | yes |
| Single-runner release, behind a validation gate | yes |
| Trimming `chrono` to `std` + `now` so the darwin binary links no macOS framework (§ Darwin framework links) | yes |
| Replacing `portable-pty`, `filedescriptor` or `chrono` with hand-written code (needs a second `unsafe_code` exception) | no |
| Splitting the crate into a workspace | no |
| Finishing design 0000222 (deterministic test timing) | no |

### Hand-written baseline ledger

#### Ledger DDL

Append the ledger table to `assets/migrations/V1__baseline.sql`, after `member_input_operations`. `SPEC.md` §8 already lists `refinery_schema_history` and ends its exact baseline DDL with a one-line form of this table; rewrite that statement in the form below so the SPEC block and the embedded file match. The table keeps `refinery`'s exact name and 4-column shape:

```sql
CREATE TABLE refinery_schema_history (
    version INT4 PRIMARY KEY,
    name VARCHAR(255),
    applied_on VARCHAR(255),
    checksum VARCHAR(255)
);
```

The schema guard needs no change. `reference_schema()` builds its reference by running `initialize` on an in-memory database, so the guard keeps checking the ledger's column shape, and `classify_schema` keeps requiring the single `(1, 'baseline')` row through `ledger_state`.

#### Ledger row values

| Column | Value |
|---|---|
| `version` | `1` |
| `name` | `'baseline'` |
| `applied_on` | `crate::time::now_stamp()` at initialization (canonical `YYYY-MM-DDTHH:MM:SS.ffffff+00:00`) |
| `checksum` | `crate::digest::sha256_hex(BASELINE_SQL.as_bytes())`: 64 lowercase hex characters over the embedded `V1__baseline.sql` bytes, ledger DDL included |

Nothing reads `applied_on` or `checksum`. They record when and from which baseline the file was created.

#### `initialize` algorithm

The signature becomes `pub(crate) fn initialize(conn: &mut Connection, database_url: &str) -> Result<(), CafleetError>`. The URL supplies the incompatible-format path through `crate::db::database_path`, so `initialize` and `classify_selected_schema` report the same `PATH` for the same file, including relative URLs. Every caller passes the URL it connected with. `reference_schema()` and the in-memory test connections pass `"sqlite:///:memory:"`, the form the existing `require_current` test helper already uses.

1. Open `conn.transaction_with_behavior(TransactionBehavior::Immediate)`. The write lock serializes concurrent initializers, and the connection's busy timeout bounds the wait: `busy_timeout=5000` from `connect`, or the scoped budget when the caller runs `initialize` inside `with_database_deadline`.
2. Read the file state inside the transaction, so the check and the writes share the write lock:

   | State | Condition | Result |
   |---|---|---|
   | **Empty** | No `refinery_schema_history` table, `PRAGMA application_id` is 0, and `sqlite_master` holds no non-`sqlite_%` table | Go to step 3 |
   | **Baseline** | The ledger table exists and its `(version, name)` rows ordered by `version` equal exactly `[(1, "baseline")]` | Drop the transaction with no writes and return `Ok(())` |
   | **Other** | Every other file: a ledger with any other row set (zero rows included), or no ledger with a non-zero identity or any non-`sqlite_%` table | Drop the transaction with no writes and return `Err(crate::db::incompatible_format(crate::db::database_path(database_url)?))` |

   **Empty** is exactly the file `SPEC.md` §6.3 allows setup to initialize: "a missing file, or an empty SQLite file with identity zero and no user tables".
3. `execute_batch(BASELINE_SQL)`, which includes `PRAGMA application_id` and the ledger DDL.
4. Insert the ledger row with bound parameters: `INSERT INTO refinery_schema_history(version, name, applied_on, checksum) VALUES(1, 'baseline', ?1, ?2)`.
5. Insert the version row with a bound parameter: `INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, ?1)` with `crate::cli::VERSION`.
6. Commit.

Every SQLite failure in steps 1 and 3–6 maps to the existing `database initialization failed: <cause>` error. Dropping the uncommitted transaction rolls back identity, tables, ledger and version together, so `failed_baseline_initialization_rolls_back_identity_tables_and_ledger` keeps passing.

Replace `ledger_is_baseline` with one helper, `ledger_state`, that returns whether the ledger is absent, holds exactly the baseline row, or holds anything else. `classify_schema` treats absent and other as incompatible, exactly as today. `initialize` combines `ledger_state` with the identity and table checks that `classify_schema` already runs for its `Missing` result; extract those checks into one shared `is_empty_file(conn)` helper. Delete the `baseline()` function and the `format!`-built SQL string.

```rust
enum LedgerState { Absent, Baseline, Other }

fn ledger_state(conn: &Connection) -> Result<LedgerState, CafleetError>;
fn is_empty_file(conn: &Connection) -> Result<bool, CafleetError>; // application_id == 0 && no non-sqlite_% table

pub(crate) fn initialize(conn: &mut Connection, database_url: &str) -> Result<(), CafleetError> {
    let failed = |e: rusqlite::Error| CafleetError::App(format!("database initialization failed: {e}"));
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(failed)?;
    match ledger_state(&tx)? {
        LedgerState::Baseline => return Ok(()),
        LedgerState::Absent if is_empty_file(&tx)? => {}
        LedgerState::Absent | LedgerState::Other => {
            return Err(incompatible_format(database_path(database_url)?));
        }
    }
    tx.execute_batch(BASELINE_SQL).map_err(failed)?;
    tx.execute(
        "INSERT INTO refinery_schema_history(version, name, applied_on, checksum) VALUES(1, 'baseline', ?1, ?2)",
        rusqlite::params![crate::time::now_stamp(), crate::digest::sha256_hex(BASELINE_SQL.as_bytes())],
    ).map_err(failed)?;
    tx.execute(
        "INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, ?1)",
        [crate::cli::VERSION],
    ).map_err(failed)?;
    tx.commit().map_err(failed)
}
```

#### Ledger tests (in `src/db.rs` `mod tests`)

| Test | Asserts |
|---|---|
| `baseline_initialization_records_the_identity_ledger_version_and_shape` (extended) | The ledger row is `(1, 'baseline', applied_on, checksum)`. `applied_on` parses with `crate::time::parse_lenient`, and `checksum == crate::digest::sha256_hex(BASELINE_SQL.as_bytes())`. |
| `repeated_initialization_preserves_all_application_rows` (unchanged) | Re-initialization is a no-op for application rows and the ledger. |
| `failed_baseline_initialization_rolls_back_identity_tables_and_ledger` (unchanged) | A failed initialization leaves no identity, table, ledger or version row. |
| New: `initialization_refuses_every_other_file` (table-driven over a named case array) | Each case prepares a temp file in the **Other** state: an initialized file with an added `(2, …)` ledger row, an initialized file with its ledger row deleted, and a file holding one unrelated table and no ledger. For every case, `initialize` fails with the incompatible-format message naming the URL's path, and the file bytes are unchanged. Each assertion message names the case. |
| New: `concurrent_initialization_of_one_fresh_file_applies_the_baseline_once` | Two threads, each with its own `connect`ed connection to one fresh temp file, call `initialize` behind a `std::sync::Barrier`. Both return `Ok`, the ledger holds one row, `cafleet_database` holds one row and `require_current` passes. |

`initialization_refuses_every_other_file` shares the temp-file preparation and byte-snapshot comparison with `assert_incompatible_and_untouched`: extract them into one helper inside `mod tests` that both use. The existing `a_ledger_row_beyond_the_baseline_is_incompatible` keeps covering `classify_schema`; the new test covers `initialize`.

### macOS process group without `libproc`

#### One unsafe `kern.proc` helper

Replace `kinfo_proc_bytes` in `src/runtime/process.rs` with one item-scoped `#[allow(unsafe_code)]` helper that serves both `KERN_PROC_PID` and `KERN_PROC_PGRP`:

```rust
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn kern_proc(selector: c_int, argument: c_int, buffer: Option<&mut [u8]>) -> io::Result<usize>;
```

- The MIB is `[CTL_KERN, KERN_PROC, selector, argument]`.
- With `None`, `oldp` is null, and the kernel stores the required byte count in `length`, which the helper returns.
- With `Some(buffer)`, the call passes `buffer.as_mut_ptr()` and `length = buffer.len()`, and returns the written count.
- A non-zero status returns `io::Error::last_os_error()`.
- The `SAFETY` comment states both cases: the MIB array is live and its length passed exactly, the buffer is live and writable with its capacity in `length`, and no new value is set.

The PID read in `kernel_process` fills a named local buffer, so the kernel-written `p_starttime` (0..12), `p_stat` (36) and `p_pid` (40..44) stay readable:

```rust
let mut bytes = [0u8; KINFO_PROC_BYTES];
let length = kern_proc(nix::libc::KERN_PROC_PID, pid32, Some(&mut bytes))
    .map_err(|cause| error(format!("cannot establish process start identity: {cause}")))?;
```

`kernel_process` keeps its `length == 0`, `length == KINFO_PROC_BYTES` and `p_pid` checks and its field reads on `bytes`.

#### `group_members` on macOS

1. Run up to 3 attempts. In each, call `kern_proc(KERN_PROC_PGRP, group, None)` for the size, allocate `size + 8 * KINFO_PROC_BYTES` zero bytes, and call `kern_proc(KERN_PROC_PGRP, group, Some(&mut buffer))`.
   - `ENOMEM` (the group grew between the two calls) starts the next attempt.
   - Any other error returns at once.
   - After 3 `ENOMEM` results, fail with `kern.proc.pgrp kept growing during the read`.
2. A returned length of 0 is an empty group, so return an empty `Vec`.
3. A length that is not a multiple of `KINFO_PROC_BYTES` fails with `kern.proc.pgrp returned a partial kinfo_proc record`.
4. For each 648-byte record, read `p_pid` from bytes 40..44 (native-endian `i32`), the same offset `kernel_process` uses, and return the PIDs as `i64`.

`kern.proc` reports every process regardless of effective UID, so setuid group members stay visible, as they were with `proc_listpids`. Zombie entries may appear; `live_group_members` already filters them through `stopped`. Delete the `Errno::clear()` call and its comment, which existed only for `libproc`'s errno handling.

#### Test-only diagnostic in `press.rs`

Remove the `#[cfg(target_os = "macos")]` and `#[cfg(not(target_os = "macos"))]` `open_processes` bindings from `post_refusal_lock_diagnostic`. Also remove the `path_open_processes_total_and_pid_ppid_start=…` and `open_file_observation_is_not_lock_ownership_proof` fields from its format string, which becomes:

```text
case={:?}; parent_pid={}; placement={:?}; lock_path={path:?}; named={metadata:?}; snapshot=after_refusal
```

#### Manifest

Delete the `[target.'cfg(target_os = "macos")'.dependencies]` table (its only entry is `libproc`).

#### macOS process-group tests (in `src/runtime/process.rs` `mod tests`, `#[cfg(target_os = "macos")]`)

| Test | Asserts |
|---|---|
| New: `kern_proc_pgrp_lists_a_spawned_group_leader_and_then_nothing` | A `/bin/sleep 30` child spawned with `std::os::unix::process::CommandExt::process_group(0)` is listed alone by `group_members(child_pid)`. After `kill` and `wait`, `group_members` returns an empty `Vec`. |
| `refresh_pins_a_live_setuid_root_group_member_and_cleanup_terminates_it` (unchanged) | A setuid-root group member is listed, pinned and terminated. |

### Dev/test profiles

```toml
[profile.dev.package."*"]
opt-level = 2

[profile.dev.package.libsqlite3-sys]
opt-level = 0

[profile.dev.package.cafleet]
debug = "line-tables-only"
```

A package override names one package, so it takes precedence over the `"*"` override. Cargo passes the package's opt-level to the `libsqlite3-sys` build script, and `cc` compiles `sqlite3.c` with `-O0`. Step 6 confirms this by finding `-O0` in the `sqlite3.c` compile line of `cargo test --no-run -vv`. The `test` profile inherits both overrides from `dev`. The release profile is unchanged, so `cargo install` and the release binaries are unaffected. `line-tables-only` keeps file:line panic backtraces and drops local-variable debug info.

### CI

`.github/workflows/ci.yml`, job `check`:

```yaml
    name: ${{ matrix.task }} (${{ matrix.runner }})
    runs-on: ${{ matrix.runner }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - task: lint
            runner: ubuntu-latest
          - task: lint
            runner: macos-15
          - task: test
            runner: ubuntu-latest
          - task: test
            runner: macos-15
    steps:
      # checkout, mise-action and cache steps unchanged; the cache key already includes runner.os
      - if: matrix.task == 'lint'
        run: mise //:lint
      - if: matrix.task == 'test'
        run: mise //:test
        env:
          CARGO_PROFILE_TEST_OPT_LEVEL: "1"
```

The macOS `lint` entry runs `cargo clippy --all-targets -- -D warnings` over the `#[cfg(target_os = "macos")]` code: the `kern_proc` unsafe item, the macOS `group_members` and the macOS-only tests. The Linux entries never compile that code.

`main` has no required status checks (its rulesets cover deletion, non-fast-forward and Copilot review only), so the new job names need no settings change.

#### First macOS CI run

This is the suite's first macOS CI run, so it can fail for causes outside this design, such as the runner environment, wall-clock budgets on a 3-vCPU runner, or a macOS test failure that already exists. Handle each failing macOS lint or test entry this way:

1. If this design's changes cause the failure, fix it in this PR.
2. Otherwise, record the test name and error in § Measurements under a § macOS first-run failures subsection. Then send `blocked` to the Director, who obtains the user's decision to fix, skip or defer it.
3. Merge requires passing macOS `lint` and `test` entries.

The `//:test` task resolves its default thread count with `nproc`, which macOS does not ship. Change it to pick the CPU-count command by platform and keep the 32-thread cap:

```toml
[tasks.test]
run = "RUST_TEST_THREADS=${RUST_TEST_THREADS:-$(cpus=$(if [ \"$(uname)\" = Darwin ]; then sysctl -n hw.logicalcpu; else nproc; fi); echo $(( cpus < 32 ? cpus : 32 )))} cargo test --bin cafleet"
```

Linux keeps `nproc`, which honours CPU affinity and cgroup quotas. The suite fails when oversubscribed, so the host CPU count must not replace it.

### Gated single-runner release

#### Gate

On the implementation PR, add a temporary pair of jobs to `ci.yml`, observe their run, and remove both jobs before merge:

| Job | Runner | Steps |
|---|---|---|
| `darwin-zigbuild` | `ubuntu-24.04` | checkout, mise-action, `rustup target add aarch64-apple-darwin`, `pipx install cargo-zigbuild`, `cargo zigbuild --release --target aarch64-apple-darwin`, upload `target/aarch64-apple-darwin/release/cafleet` as an artifact |
| `darwin-zigbuild-smoke` | `macos-15`, `needs: darwin-zigbuild` | Download the artifact and `chmod +x` it, then run, with `HOME` and the database under `$RUNNER_TEMP`: `codesign --verify --verbose ./cafleet`, `./cafleet --version`, `./cafleet setup --database-url sqlite:///$RUNNER_TEMP/smoke.db`, `./cafleet fleet list --json --database-url sqlite:///$RUNNER_TEMP/smoke.db` |

The gate passes when both jobs succeed, `--version` prints the `Cargo.toml` version, and `fleet list --json` prints `[]`. Record the run URL, the darwin zigbuild build-step duration and the outcome in § Release gate result. The 0.35.4 native darwin build step took 111 s.

#### Darwin framework links

Zig ships libSystem stubs for macOS but no framework stubs, so a zig-linked darwin binary must link no macOS framework unless the job supplies an SDK through `SDKROOT`. Today one framework link exists: `cargo tree -e normal --target aarch64-apple-darwin -i core-foundation-sys` shows `chrono` (default `clock` feature) → `iana-time-zone` → `core-foundation-sys`, which links CoreFoundation.

The crate uses only `chrono::Utc`, `DateTime` (including `parse_from_rfc3339` and `format`), `TimeZone` and `Duration`, and no `Local`; `chrono` has no other dependent in the graph. Declare it as:

```toml
chrono = { version = "0.4", default-features = false, features = ["std", "now"] }
```

`now` provides `Utc::now`, and `std` (which implies `alloc`) provides `format(...).to_string()`. This drops `iana-time-zone` and `core-foundation-sys` from the darwin graph and `iana-time-zone` from the Linux graphs. The gate job then needs no SDK. The trim lands in Step 4, before the Step 7 gate. If the gate still fails at link time on another framework, the § Fallback applies and the failure names that framework.

#### Adopted shape (gate passed)

Each `release.yml` `build` matrix entry keeps its `target` and `bottle_tag` and drops `runner`. Every entry `runs-on: ubuntu-24.04`, runs `pipx install cargo-zigbuild` and `cargo zigbuild --release --target ${{ matrix.target }}` unconditionally, and drops both `contains(matrix.target, 'musl')` conditions and the plain `cargo build` step. The three entries stay parallel, which keeps the about-3-minute wall time; one runner OS and one build command replace the macOS runner and the musl/non-musl branch. Packaging and upload steps are unchanged.

#### Fallback (gate failed)

`release.yml` is unchanged. § Release gate result records the failing job, step and error.

### Dependency-graph gates

Run each command from the repository root (`--offline` uses the cached registry). The count includes the `cafleet` line.

| Graph | Command |
|---|---|
| Linux normal | `cargo tree -e normal --target x86_64-unknown-linux-gnu --prefix none --no-dedupe --offline` piped to `sort -u` and `wc -l` |
| Linux normal+build | `cargo tree -e normal,build --target x86_64-unknown-linux-gnu --prefix none --no-dedupe --offline` piped to `sort -u` and `wc -l` |
| darwin normal+build | `cargo tree -e normal,build --target aarch64-apple-darwin --prefix none --no-dedupe --offline` piped to `sort -u` and `wc -l` |

Step 1 records each count twice: as is, and with `--prune refinery --prune libproc --prune iana-time-zone`, which is the expected after-value. On the 2026-11-21 lockfile the as-is counts are 122 / 130 / 139 and the pruned counts are 75 / 83 / 82. After Steps 3–4 each as-is count must equal its Step 1 pruned value.

### Measurements

Timings vary up to 2× with host load, so each timing is the median of 3 runs on the same host. Record `/proc/loadavg` beside each timing. Only the crate-count gates and the suite-regression check below decide acceptance; the timings are recorded, not gated.

The evaluation reference column was measured at `4e8418f8`, before design 0000237 removed about 34k lines and reworked the tests; design 0000237 recorded a 37.6 s local suite wall time at `21ea7be8`. Step 1 measures the current before-values.

| Metric | Command or source | Evaluation reference (`4e8418f8`) |
|---|---|---|
| Crate counts | § Dependency-graph gates | 122 / 130 / 139 |
| Cold test build | `cargo test --bin cafleet --no-run` with a fresh `CARGO_TARGET_DIR` | 87–229 s |
| Incremental test build | Edit the body of `now_stamp` in `src/time.rs`, then `cargo test --bin cafleet --no-run` | 12.7–13.1 s; 9.6–10.3 s with `line-tables-only` |
| Test binary size | Size of the `target/debug/deps/cafleet-*` test executable | 238 MB; 155 MB with `line-tables-only` |
| Local suite wall time | `mise //:test` | 60–69 s on 8 threads |
| CI test job duration | Ubuntu `test` job of the last `main` run before the change versus the PR run; the macOS `test` job on the PR run | 47.5 s suite at opt-level 1 (local) |

**Suite-regression check (`libsqlite3-sys` opt-level).** This is the only timing that decides acceptance, so it compares two builds of one tree on one host:

1. Use the Step 5 tree with two variants, each with its own `CARGO_TARGET_DIR`: B has no `libsqlite3-sys` override, and A adds `[profile.dev.package.libsqlite3-sys] opt-level = 0`. Build each variant once with `cargo test --bin cafleet --no-run` before timing, so later runs reuse its cached artifacts.
2. Run `mise //:test` alternately, B, A, B, A, B, A. Before each run, set `Cargo.toml` to that variant's override state and `CARGO_TARGET_DIR` to its directory. Record the wall time and `/proc/loadavg` of each run.
3. A regresses when its median exceeds the largest B run.
4. On regression, rebuild A with opt-level 1 and repeat steps 2–3 against fresh B runs. If opt-level 1 still regresses, the tree keeps no `libsqlite3-sys` override.

Record every run, its load and the selected opt-level.

Record the results in this section as a table with metric, before, after and load columns. Add a § Release gate result subsection for the gate outcome.

### Documentation updates

| Target | Change |
|---|---|
| `SPEC.md` §8 | Rewrite the existing one-line `refinery_schema_history` statement to match `V1__baseline.sql`, and change "ending with the ledger table that initialization creates beside it" to state that the baseline itself declares the ledger. Replace the paragraph "Initialization applies the baseline and inserts the `cafleet_database` row for the running version in one transaction, …" with the outcome of § `initialize` algorithm: one `BEGIN IMMEDIATE` transaction, and the empty, baseline and other file states with their results. Replace "`applied_on` and `checksum` are non-normative" with the normative row values in implementation-neutral terms: `applied_on` is the canonical storage timestamp of initialization and `checksum` is the lowercase SHA-256 hex of the embedded baseline file; diagnosis still compares only `version`, `name` and the column shape. |
| `docs/docs/contributing.md` Rust safety paragraph | The single unsafe exception becomes the macOS `kern.proc` sysctl helper serving `KERN_PROC_PID` and `KERN_PROC_PGRP` into size-checked `kinfo_proc` buffers, still item-scoped with its safety invariants inline |
| `docs/docs/contributing.md` validation paragraph | Add: CI runs `mise //:test` on Linux and macOS with `CARGO_PROFILE_TEST_OPT_LEVEL=1`; local runs keep opt-level 0 for faster incremental rebuilds |
| `CLAUDE.md` | "an embedded refinery schema baseline" becomes "an embedded SQL schema baseline" |

`README.md`, `assets/skills/` and `.claude/rules/` need no change: no CLI surface, output or convention changes. Check each anyway in Step 2, together with `docs/docs/concepts/storage.md`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Record baseline measurements

- [ ] Record the as-is and pruned crate counts from § Dependency-graph gates in § Measurements <!-- completed: -->
- [ ] Measure cold test build, incremental test build, test binary size and local suite wall time (3 runs each, with load) on the unchanged tree, and record them <!-- completed: -->
- [ ] Record the ubuntu `test` job duration of the latest `main` CI run <!-- completed: -->

### Step 2: Update documentation

- [ ] `SPEC.md` §8: add the ledger DDL to the exact baseline DDL, and replace the initialization paragraph per § Documentation updates <!-- completed: -->
- [ ] `docs/docs/contributing.md`: update the Rust safety paragraph and the validation paragraph <!-- completed: -->
- [ ] `CLAUDE.md`: replace "refinery schema baseline" with "SQL schema baseline" <!-- completed: -->
- [ ] Confirm that `README.md`, `assets/skills/`, `.claude/rules/` and `docs/docs/concepts/storage.md` need no change <!-- completed: -->

### Step 3: Hand-written baseline ledger

- [ ] Append the `refinery_schema_history` DDL to `assets/migrations/V1__baseline.sql` <!-- completed: -->
- [ ] Add the `ledger_state` helper and use it in `classify_schema` <!-- completed: -->
- [ ] Rewrite `initialize` per § `initialize` algorithm, including the `database_url` parameter and the `is_empty_file` helper, and delete `baseline()` <!-- completed: -->
- [ ] Pass the connected database URL at every `initialize` call site <!-- completed: -->
- [ ] Extend `baseline_initialization_records_the_identity_ledger_version_and_shape` and add `initialization_refuses_every_other_file` and `concurrent_initialization_of_one_fresh_file_applies_the_baseline_once` <!-- completed: -->
- [ ] Remove `refinery` from `Cargo.toml`; `cargo check` drops its packages from `Cargo.lock` <!-- completed: -->
- [ ] `mise //:test` and `mise //:lint` pass <!-- completed: -->

### Step 4: Remove `libproc` and trim `chrono` features

- [ ] Replace `kinfo_proc_bytes` with the `kern_proc` helper and move the PID read onto its named local buffer <!-- completed: -->
- [ ] Rewrite the macOS `group_members` per § `group_members` on macOS <!-- completed: -->
- [ ] Add `kern_proc_pgrp_lists_a_spawned_group_leader_and_then_nothing` <!-- completed: -->
- [ ] Remove the macOS open-file observation from `post_refusal_lock_diagnostic` <!-- completed: -->
- [ ] Delete the macOS `libproc` dependency table and refresh `Cargo.lock` <!-- completed: -->
- [ ] Declare `chrono` with `default-features = false, features = ["std", "now"]` per § Darwin framework links <!-- completed: -->
- [ ] `mise //:test` and `mise //:lint` pass on Linux; Step 5's macOS `lint` and `test` entries validate the macOS code <!-- completed: -->
- [ ] Verify the crate-count gates and the absence of `refinery`, `libproc`, `bindgen`, `clang-sys`, `iana-time-zone` and `core-foundation-sys` in `Cargo.lock` <!-- completed: -->

### Step 5: CI test entries

- [ ] Change the `//:test` task's CPU-count command per § CI <!-- completed: -->
- [ ] Restructure `ci.yml` into the Linux and macOS `lint` and `test` entries, with `CARGO_PROFILE_TEST_OPT_LEVEL=1` on the test step <!-- completed: -->
- [ ] Push the branch and confirm all four CI entries pass, with the macOS `test` entry running the macOS process-group tests; handle failures per § First macOS CI run <!-- completed: -->

### Step 6: Dev/test profiles

- [ ] Build variants B and A and run the alternating B/A suite runs of § Measurements' suite-regression check, recording wall time and load per run <!-- completed: -->
- [ ] Confirm `-O0` on the `sqlite3.c` compile line of variant A's `cargo test --bin cafleet --no-run -vv` <!-- completed: -->
- [ ] Apply the check's outcome to `Cargo.toml` (opt-level 0, opt-level 1 after a repeated check, or no override) <!-- completed: -->
- [ ] Add `[profile.dev.package.cafleet] debug = "line-tables-only"` <!-- completed: -->
- [ ] `mise //:test` and `mise //:lint` pass <!-- completed: -->

### Step 7: Single-runner release gate

- [ ] Add the temporary `darwin-zigbuild` and `darwin-zigbuild-smoke` jobs to `ci.yml` and push <!-- completed: -->
- [ ] Record the gate run URL, the darwin build-step duration and the outcome in § Release gate result <!-- completed: -->
- [ ] On pass, convert `release.yml` to the adopted shape; on failure, leave `release.yml` unchanged <!-- completed: -->
- [ ] Remove both temporary jobs from `ci.yml` <!-- completed: -->

### Step 8: After-measurements and final validation

- [ ] Measure every § Measurements metric on the final tree (3 runs each, with load) and record before/after <!-- completed: -->
- [ ] Record the PR's ubuntu and macOS `test` job durations <!-- completed: -->
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass, and every CI entry passes on the PR <!-- completed: -->

Implementers leave the installed `cafleet` untouched, because the executing fleet runs on it. After the execute fleet is deleted, the Director reinstalls the CLI with `mise //:install`.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-21 | Initial draft |
| 2026-11-21 | Reconcile with design 0000237: ledger DDL anchor, existing SPEC §8 ledger statement and paragraphs, table-driven refusal test, scoped busy budget, dropped removed flake from scope, annotated evaluation references |
