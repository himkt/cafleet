# Rewrite the cafleet binary in Go

**Status**: Approved
**Progress**: 3/69 tasks complete
**Last Updated**: 2026-11-24

## Overview

Replace the Rust implementation of the `cafleet` binary with a Go implementation that reproduces every observable behavior of the current Rust binary. The goals, in priority order, are a simpler build, no dropped functionality and fewer dependencies; reduced compile-time type safety is accepted. The port is a big-bang transliteration on one long-lived branch: the Rust binary from the same tree serves as the black-box oracle, and the switchover is a single change that also deletes the Rust code.

## Success Criteria

- [ ] On the rewrite branch's final commit, `mise //:lint`, `mise //:vulncheck`, `mise //:test` and `mise //:test-shipped` pass on `ubuntu-latest` and `macos-15`, and the repository contains no `.rs` file, `Cargo.toml`, `Cargo.lock`, `build.rs` or `migration/` directory.
- [ ] `mise.toml` pins no `core:rust`, `core:python` or `core:zig` and no `cargo:` tool, and no workflow installs rustup targets, zig or cargo-zigbuild. One `CGO_ENABLED=0 go build` per release target produces all three release archives on `ubuntu-24.04`.
- [ ] `go list -deps -f '{{with .Module}}{{.Path}}{{end}}' ./cmd/cafleet` lists, apart from `github.com/himkt/cafleet`, only the modules in § Dependencies: at most 5 third-party modules from the Go team, ncruces and creack.
- [ ] Before the cutover commit, the migration differential harness reports 0 differences between the Go and Rust binaries for every command and runner scenario on Linux and macOS (gates G1–G2).
- [ ] The Go terminal emulator matches the Rust `vt100` 0.18.1 oracle at every observation step on the fixture, live, capture and fuzz corpora. The fuzz corpora hold at least 2 MB of seeded input (gate G3).
- [ ] The Go detector reproduces the Rust detector's golden on every bundled native screen fixture and every capture checkpoint (gate G4).
- [ ] The migration test map shows that every Rust `#[test]` on the merged main head has a Go counterpart in the mapped file, or appears in the omission list with its replacement test (gate G5).
- [ ] Live acceptance records (docs `contributing.md` § Release acceptance) are re-collected against the Go binary for every bundled backend on tmux and herdr, on a dedicated database (gate G6).
- [ ] Every SPEC.md amendment in § SPEC amendments has landed, and the documentation, README, skill and rule edits in § Documentation and rules have landed, before the first Go production file.
- [ ] The first Go-only release is the next minor version after the last Rust release. Its `cafleet --version`, default database file name and `cafleet_database` row all carry that version.

---

## Background

`cafleet` is about 30.9k non-blank production lines and 79.9k test lines of Rust (1,468 in-file tests), on 14 normal crates (74 unique linked crates plus about 7 build-time crates). The release build needs rustup targets, zig, pipx `cargo-zigbuild`, `cargo-llvm-cov` and a 74-line `build.rs`. Over half of production is the member PTY runner (`src/runtime/`), and its `InputCoordinator` state machine alone is about 4.2k lines.

The feasibility study and its experiments live locally under `researches/go-rewrite/` (gitignored): `review.md`, the eight scope reports, and `exp-runner.md`, `exp-interop.md`, `exp-capture.md` and `exp-vt.md`. `exp-macos.md` reports the macOS validation run on `macos-15` (macOS 15.8.2, arm64). The PoCs, corpora and oracles are preserved under `researches/go-rewrite/poc/` (for example `poc/exp/vt`, `poc/exp/capture`, `poc/exp/interop`, `poc/pty`). The findings this design relies on:

| Finding | Evidence |
|---|---|
| A cgo-free Go build cross-compiles every release target; incremental rebuilds take about 1 s | build.md; review.md §4 |
| `github.com/ncruces/go-sqlite3` provides every SQLite hook cafleet uses (busy handler, progress-handler interrupt, autocommit, commit/rollback/update hooks, authorizer) without cgo | sqlite.md; review.md C2 (39/39 checks) |
| ncruces OFD locks and rusqlite POSIX locks see each other on Linux in both directions | exp-interop.md §4.2 |
| Go 1.27 `encoding/json/v2` writes SPEC-exact JSON bytes. It still needs a hand-written strict reader for required keys, present-null and tri-state optionals | review.md C4 |
| A Go transliteration of vt100 0.18.1 + vte 0.15.0 + unicode-width 0.2.2 (2,550 lines + 428 generated table lines) matched the Rust oracle on 1,355/1,355 cases, 527,778 snapshots | exp-vt.md |
| `charmbracelet/x/vt` is not viable: its 0x9C title defect fires on every Claude Code session | exp-capture.md; pty.md |
| A transliterated `InputCoordinator` slice passed its ported tests first time under `-race`; `exhaustive` + `go-check-sumtype` restore exhaustiveness except for the zero value | exp-runner.md |
| Go runtime start-up drops an inherited block of SIGTERM and inherited `SIG_IGN` for SIGQUIT/SIGTERM/SIGUSR2/SIGPIPE; other blocks and the ignores of SIGHUP, SIGINT and SIGTSTP survive | exp-interop.md §5.3 |
| Child processes forked from a locked OS thread inherit that thread's signal mask on Linux | review.md C1 |
| On darwin, `RawSyscall(SYS___PTHREAD_SIGMASK)` (329) on a locked thread is per thread and its blocked set reaches direct-exec children, cgo-free; `SYS_SIGPROCMASK` (48) is process-wide and leaks the mask | exp-macos.md experiment 1 |
| ncruces OFD locks and POSIX locks (rusqlite, Apple `sqlite3`) exclude each other on macOS at every lock level, in rollback and WAL modes | exp-macos.md experiment 2 (20/20 checks) |
| A `CGO_ENABLED=0` darwin/arm64 binary cross-built on Linux runs unchanged on macOS. A hand-written `cfmakeraw` equals Apple libc and `nix`, while `x/term.MakeRaw` differs | exp-macos.md experiment 3 |
| On darwin, a PTY master read at end of stream returns 0 (never `EIO`), and unread output is discarded when the last slave fd closes | exp-macos.md experiment 3 |
| The production Rust modules form one dependency cycle: 11 of the 13 top-level modules reach each other through `crate::` references, and tests reference other modules' `test_support` about 690 times | Module graph of `src/` at `740462d3` |
| Estimated effort is 48–70 person-weeks (most likely about 58) for one engineer, or 10–16 calendar weeks for an AI fleet bounded by review and integration | review.md §5; exp-runner.md §5 |

---

## Specification

### Decisions

| Area | Decision | Rationale |
|---|---|---|
| Migration model | Big-bang on the branch `feat/go-rewrite`. The Rust binary built from the same tree is the black-box oracle, and one cutover change merges the port and deletes Rust | No release ships two binaries. The version-pair, executable-record and runner-binary pitfalls of a mixed install never reach users |
| Oracle precedence | Rust's observable behavior wins for every output SPEC treats as normative, and every Rust-vs-SPEC divergence becomes SPEC text before the corresponding Go code is written. Output SPEC marks non-normative (help bodies, parse-error lines after the first) may differ, as A11 states | Keeps "no dropped functionality" testable against one reference |
| Known bugs | Fix them in Rust on `main` before the parity baseline is frozen (§ Pre-baseline Rust fixes) | The Go port never inherits a known bug as correct behavior |
| Code structure | Transliterate function for function. Do not redesign | Line-level review against the Rust source stays cheap; the spike's 1:1 mapping found 0 test-detected defects |
| Package structure | The whole Rust crate becomes one Go package, `internal/core`. Only the transliterated emulator is separate | The Rust modules form one dependency cycle, and crate-private visibility maps exactly onto Go's unexported identifiers |
| Toolchain | Go 1.27.4 pinned in mise; `go 1.27` in `go.mod` | `encoding/json/v2` is standard API in 1.27 |
| SQLite | `github.com/ncruces/go-sqlite3` (cgo-free) | It is the only cgo-free driver with public busy and progress handlers. `modernc.org/sqlite` is a re-planning option, not a drop-in fallback (§ Risks and decision gates) |
| Terminal emulator | In-repo transliteration of vt100 0.18.1, vte 0.15.0 and unicode-width 0.2.2 | Zero diffs against the oracle and no third-party module |
| CLI parser | Hand-written on the standard library | cobra breaks four SPEC parse rules by default |
| Inherited signals | SPEC states that signal dispositions and blocked signals inherited from cafleet's parent are not part of the contract (A6) | The Go runtime resets some of them; the alternative, a cgo constructor, gives up the cgo-free build |
| Tests | `<file>_test.go` beside the implementation file, in the same package. No data files: test inputs are literals or generated bytes in `_test.go` source | Go's toolchain requires `_test.go`; the project keeps its no-fixture-file rule |
| Release artifacts | Same archive and bottle names, with Rust target triples; version from a Go constant | Tap continuity; SPEC §7.6 already names target-triple archives |
| First Go-only version | Next minor after the last Rust release (0.36.0 if cut over from 0.35.x) | The per-version database file isolates it from Rust installs |

### Migration model

1. **Branch.** All Go work happens on `feat/go-rewrite`, created from `main` after the pre-baseline Rust fixes land. One draft PR to `main` tracks it, so CI runs on every push.
2. **Tracking main.** Merge `main` into the branch at least weekly. Each merge runs this procedure, in order:
   1. Port every Rust change in the merge to Go.
   2. Set `const Version` in `internal/core/version.go` to the merged `Cargo.toml` package version. The `migration` CI job's version check fails while they differ.
   3. Rebuild the oracle from the merged tree.
   4. Re-export the G3 and G4 references from the oracle, rerun both gates, and regenerate the frozen golden files with the Go update mode (§ Frozen parity inputs). Review the golden diff with the merge.
   5. Rerun G1, G2 and G5.

   The cutover gates run against the oracle built from the final merge.
3. **Coexistence.** Until cutover the tree holds both the Rust crate (`Cargo.toml`, `build.rs`, `src/`) and the Go module (`go.mod`, `cmd/`, `internal/`, root `assets.go`). The Rust crate is built only by migration tooling, as the oracle. The mise tasks and CI checks target Go.
4. **Migration tooling.** Everything that needs the Rust oracle lives under a top-level `migration/` directory that exists only on the branch (§ Migration tooling). The cutover change deletes it. It is exempt from the test-placement rule because it is not part of the permanent tree.
5. **Cutover.** One change, at the end of the branch, deletes the Rust crate, `migration/` and the branch-only tools, and applies the CI, release and version edits. The minor-version bump follows through the bump workflow (Implementation Step 13).

Rust and Go binaries never share a database in production. Each release has its own default database file, and the schema guard refuses a file recorded by another version. The branch's Go binary does carry the oracle's version, so every pre-cutover Go run, G6 included, uses a dedicated `--database-url` and leaves the installed Rust binary, its default database and its installed assets untouched.

### Repository layout

```
go.mod                          module github.com/himkt/cafleet; go 1.27; ignore ./docs
assets.go                       package cafleet: //go:embed all:assets → var Assets embed.FS
cmd/cafleet/main.go             package main: calls core.Main(os.Args, cafleet.Assets) and exits with its code
internal/core/*.go              the transliterated crate (one package, mapping below)
internal/core/version.go        const Version = "<release version>"
internal/vt/vte/                vte 0.15.0 parser
internal/vt/unicodewidth/       unicode-width 0.2.2 (non-CJK; generated tables)
internal/vt/                    vt100 0.18.1 screen model
third_party/                    upstream LICENSE texts for vt100, vte and unicode-width
.golangci.yml                   lint configuration
```

- `assets.go` sits at the module root because `go:embed` cannot reach a parent directory, and the `assets/` paths are a SPEC contract. The root package holds only the embedded filesystem, so `internal/core` and its tests import it without a cycle.
- The `ignore ./docs` directive keeps `docs/node_modules` out of every `./...` pattern, including `go mod tidy`, Renovate's `gomodTidy` and gopls.
- `go install ./cmd/cafleet` installs a binary named `cafleet`.
- **File mapping.** Each Rust source file becomes one Go file in `internal/core`, named by its module path with `_` separators: `src/<a>.rs` → `<a>.go`, `src/<a>/mod.rs` → `<a>.go`, `src/<a>/<b>.rs` → `<a>_<b>.go`, `src/<a>/<b>/mod.rs` → `<a>_<b>.go`, `src/<a>/<b>/<c>.rs` → `<a>_<b>_<c>.go`. Examples: `src/broker/runtime/events.rs` → `broker_runtime_events.go`, `src/coding_agent/catalog.rs` → `coding_agent_catalog.go`. A `#[cfg(target_os = "linux")]` or `"macos"` item moves to a sibling `<stem>_linux.go` or `<stem>_darwin.go`. `src/main.rs` becomes `cmd/cafleet/main.go` plus `core.Main` in `main.go`.
- **Identifiers.** Everything in `internal/core` is unexported except `Main`, matching the crate's `pub(crate)` visibility. Names are derived mechanically, so review against the Rust source stays 1:1:

  | Rust item | Go name |
  |---|---|
  | Type, trait, free function, constant or static | Its Rust name in lowerCamelCase (`InputCoordinator` → `inputCoordinator`, `run_until_exit` → `runUntilExit`) |
  | Method (`self` receiver) | lowerCamelCase method on the receiver type |
  | `fn new` (associated, no `self`) | `new` + type name (`newInputCoordinator`) |
  | Other associated function (no `self`) | Type name + function name in lowerCamelCase (`MemberRecord::from_row` → `memberRecordFromRow`, `PressKeys::parse` → `pressKeysParse`) |
  | A derived name that collides with another module's package-level item, or that is a Go keyword or predeclared identifier (`error`, `delete`, `recover`, `len`, `type`, `range`, …) | The module-path prefix plus the derived name (`brokerFleetsCreate` and `cliFleetCreate`; `runtimeError`, `cliMemberDelete`, `runtimeRecoveryRecover`) |

  The `predeclared` linter rejects any package-level name that shadows a predeclared identifier.

- **Byte-exact helpers** that have no Rust counterpart file (the JSON writer and strict reader, lossy UTF-8, `lines()`, the non-cleaning join, the chrono-compatible RFC 3339 normalizer) go in `internal/core/textcodec.go`. Helpers with a Rust home (ANSI strip, truncation, quoted text) stay in that home's Go file.

### Dependencies

| Module | Role | Replaces |
|---|---|---|
| Go 1.27 standard library | CLI parser, `encoding/json/v2` + `encoding/json/jsontext`, `regexp`, `crypto/sha256`, `time`, `embed`, `os/signal`, `syscall` | clap, serde, serde_json, chrono, regex, semver, sha2, tempfile, build.rs |
| `github.com/ncruces/go-sqlite3` (+ `go-sqlite3-wasm/v6`, `julianday`) | SQLite engine, busy/progress handlers, hooks, OFD-locking VFS | rusqlite (bundled C SQLite) |
| `golang.org/x/sys` | `unix` syscalls: `PthreadSigmask` (Linux), termios, `Wait4`, `SysctlKinfoProc`/`SysctlKinfoProcSlice` (darwin) | nix, rustix, signal-hook, filedescriptor |
| `github.com/creack/pty` | PTY master/slave allocation only (`pty.Open`). Children start through `syscall.ForkExec` | portable-pty (with § Mandatory fidelity fixes for its child-side steps) |

No other module is added: no cobra, no `x/term` (raw mode is hand-written, including Apple's `cfmakeraw` flag set on darwin, where `x/term.MakeRaw` differs from libc), no `x/mod` (SemVer is the semver.org regex on `regexp`), and no third-party emulator. Contributor tools pinned in `mise.toml`: `core:go` 1.27.4, `aqua:golangci/golangci-lint` and `govulncheck` (`go:golang.org/x/vuln/cmd/govulncheck`). The pinned version of each is the latest release when its task is added.

The transliterated emulator keeps its upstream notices:

- each Go file in `internal/vt` starts with a header naming its source file, crate version, MIT license and copyright holder;
- `third_party/{vt100-0.18.1,vte-0.15.0,unicode-width-0.2.2}/LICENSE` hold the MIT texts (vte and unicode-width used under their MIT option);
- each release archive includes a `THIRD_PARTY_LICENSES` file with those three notices and the licenses of the linked Go modules.

### Transliteration rules

| Rust construct | Go construct | Enforcement |
|---|---|---|
| Payload-free `enum` + exhaustive `match` | `type x uint8` with constants starting at `iota + 1`. Every `switch` lists all cases and ends in `default: panic("invalid x")`, so the zero value fails loudly at its first use | `exhaustive` linter for missing cases; the trailing panic plus tests for the zero value |
| Data-carrying `enum` | Sealed interface with an unexported marker method and `//sumtype:decl`, matched with a type switch | `gochecksumtype` with `default-signifies-exhaustive=false` |
| `Option<T>` | `(T, bool)` returns or a generic comparable `opt[T]`. Pointer optionals are not used where a nil can hide inside an interface | `nilnil`, `nilness`, review |
| `Result` + `?` | `if err != nil { return …, err }`. A typed `*cafleetError` carries the usage/application category and the exit code; `errors.Join` merges cleanup errors | `errcheck` |
| `impl Drop` guard / RAII | Idempotent `close()` deferred at the acquisition site. A field that releases an owned resource when cleared (for example `pending`) is cleared only through one funnel method | Tests and the per-sub-machine mutation check (Step 11), which includes a "funnel bypassed" mutant |
| Lifetimes, `Send`/`Sync` | Single-goroutine ownership as in the Rust design, documented on the owning type | `go test -race` |
| `try_from` / `checked_*` | Helpers that return an error on overflow (`checkedU16`, `checkedAdd`, …) | `gosec` G115 |
| `#[derive(PartialEq)]` on records | Comparable structs with `==` where every field is comparable; otherwise a hand-written `equal` method beside the type | Review |
| `BTreeMap`/`BTreeSet` iteration order | `map` plus sorted-key iteration (`slices.Sorted(maps.Keys(m))`) at every iteration site | Review |
| `#[cfg(test)]` fields and statements | Always-present hook fields, nil in production, checked before each call | — |
| `#[cfg(test)] impl` constructors and methods; `#[cfg(test)] pub(crate) mod test_support` | Functions and methods in the owning file's `<file>_test.go`. Every `_test.go` in `internal/core` shares the package scope, so all tests see them | The Go toolchain excludes `_test.go` from production builds |
| `assert_not_impl!` privacy (7 types) | Unexported fields plus redacting `String`, `GoString`, `Format` and a `MarshalJSON` that returns an error. A reflection test per type asserts them | Tests |
| `String` is UTF-8 | `utf8.ValidString` at every byte → string boundary SPEC names | Tests |
| `thread_local!` deadline stack | Explicit `context.Context` per call; ncruces `SetInterrupt(ctx)` | — |

`depguard` bans the `unsafe` and `C` imports in every cafleet package. The single exception is `internal/core/time_darwin.go`, the darwin span-mask call (§ Signals and processes), excluded by path in `.golangci.yml` and carrying its safety invariants inline.

### Mandatory fidelity fixes

Each fix is a named requirement with a Go test in the `_test.go` of the file that implements it.

| # | Requirement | Go mechanism |
|---|---|---|
| F1 | Native children receive no inherited fd above 2 | At startup, before any other open, mark every fd > 2 `FD_CLOEXEC`, enumerating `/proc/self/fd` on Linux and `/dev/fd` on darwin |
| F2 | A PTY child starts with SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM and SIGALRM at default disposition, an empty mask, `setsid()` and the slave as controlling terminal (portable-pty 0.9 `pre_exec`) | `signal.Notify` for those signals before the first spawn; spawn from a thread with an empty mask; `SysProcAttr{Setsid: true, Setctty: true, Ctty: 0}` |
| F3 | A broken stdout pipe ends output quietly with the command's own result; children start with SIGPIPE at default | `signal.Notify(…, SIGPIPE)` at startup and map `EPIPE` to success. Never `signal.Ignore`, which leaks `SIG_IGN` into children |
| F4 | A process-group leader is reaped only after the zombie check that pins its PGID | `syscall.ForkExec` + `syscall.Wait4(pid, …, WNOHANG, nil)` after the `/proc` or `SysctlKinfoProc` zombie check. No background `cmd.Wait` |
| F5 | Every child started inside a termination-deferral span inherits the blocked INT/TERM/HUP set; no child started outside the spawner does | One spawner goroutine locked to an OS thread, masked with `unix.PthreadSigmask` on Linux and `SYS___PTHREAD_SIGMASK` on darwin (§ Signals and processes). Every in-span start routes through it: multiplexer clients, the native `--version` probe and the Hermes `config path` probe. Tests on both CI OSes: the masked thread reads the set while a second locked thread reads none; an in-span child reports the set and defers its own SIGTERM; children spawned concurrently from ordinary goroutines report none; the mask is clear after restore |
| F6 | JSON output follows SPEC §6.4 JSON encoding byte for byte | `jsontext.Encoder` defaults (U+2028/2029, `<&>`, `/` and DEL raw). Struct field order, or an ordered token writer for dynamic objects. Canonical digests sort keys by UTF-8 bytes |
| F7 | Decoding rejects duplicate keys, unknown keys, case variants, invalid UTF-8, a missing required key and a present `null` where the Rust decoder rejects them; tri-state optionals and internally tagged enums decode as in serde | `encoding/json/v2` with `RejectUnknownMembers(true)`, plus a token-level strict reader for required keys, present-null, tri-state and tagged dispatch. Large integers decode into typed fields or `jsontext.Value` |
| F8 | Lossy UTF-8 replaces each maximal ill-formed subsequence with one U+FFFD | Hand-written decoder |
| F9 | `<quoted>` text matches Rust `str` Debug escaping | `unicode` tables, with Grapheme_Extend = Mn ∪ Me ∪ Other_Grapheme_Extend |
| F10 | Line splitting matches Rust `str::lines()` (strip a trailing `\r`), and `capture_window` keeps its `\n`-only split | A `lines` helper |
| F11 | User-supplied base paths are joined without cleaning | A non-cleaning join, never `filepath.Join` on a user base |
| F12 | Embedded assets keep today's set and order | `go:embed all:assets`; the walker drops `.`-prefixed names, sorts full relative paths bytewise per tree and panics on an empty tree |
| F13 | herdr pane order is a stable sort on `rect.y` | `slices.SortStableFunc` |
| F14 | The three code regexes keep their matches after R4 removes their Perl classes | Transliterate R4's explicit ASCII classes |
| F15 | The per-step observer reads the screen without copying it | The observer gets a read-only `*vt.Screen` view. `Screen.CopyInto` with a reused buffer serves only where a snapshot outlives the step; no allocating clone per step |
| F16 | Raw mode matches `cfmakeraw` on each OS | Hand-written termios flag sets on `unix.IoctlGetTermios`/`IoctlSetTermios` for Linux (glibc/musl) and darwin (BSD libc: sets `IGNBRK`, clears `IMAXBEL` and `ECHOE`; exp-macos confirmed equality with libc and `nix`) |
| F17 | RFC 3339 parsing accepts what chrono accepts (`t`/`T`/space separators, lowercase `z`, second `60`) | A normalizer before `time.Parse` |
| F18 | Library-defined texts in contract strings match the Rust binary (A2) | Rust `io::Error` display `<Strerror> (os error N)` rebuilt from the errno table (first letter capitalized); SQLite messages without ncruces's `sqlite3: ` prefix; `ExitStatus` as `exit status: N` / `signal: N (SIGNAME)` |
| F19 | `PATH` lookup runs executables from relative entries as Rust does | Resolve through the transliterated `find_executable_in` and clear `exec.ErrDot` |
| F20 | The database deadline budget behaves as today: 10 ms busy slices to the shared deadline for `send` and schema validation; the plain 5 s busy timeout for `poll` | ncruces `BusyHandler` + `SetInterrupt(ctx)`; the progress handler polls every 1,000 VM steps |
| F21 | The production catalog loader stops reading each profile file after `profile`. The guard test still validates the whole file | Token-stream early stop in the strict reader |
| F22 | A PTY master read that returns 0 bytes or `EIO` ends the stream on both OSes (darwin returns 0, Linux `EIO`) | The non-blocking master read loop maps both to EOF, as `src/runtime/process.rs:663-665` does. Test: a child that writes a payload and exits is read to EOF on both CI OSes |
| F23 | Every byte a running PTY child writes reaches the runner | The parent closes its slave fd right after spawn (`process.rs:430`), and the master read loop runs continuously from spawn until EOF. Test: 258,048 bytes written while the runner reads all arrive, followed by EOF |
| F24 | Runner shutdown never deadlocks on a darwin session-leader child that blocks in exit while its output is unread | Shutdown drains the master before waiting for the child, or waits with a deadline and keeps draining. No path waits on the child while the master goes unread. Test: a session-leader child with unread output exits once the drain runs, within the shutdown deadline |
| F25 | On darwin, a PID lookup distinguishes an absent process from a read failure | `unix.SysctlRaw("kern.proc.pid", pid)` with length 0 is "absent"; `SysctlKinfoProc` supplies typed fields only for a present process. Test: a reaped PID reads as absent, not as an error |

### Signals and processes

- **Process-wide deferral.** On the outermost span begin, `signal.Notify` every deferred signal that is not `signal.Ignored`; count nesting depth. On the outermost end, `Stop`, drain, `signal.Reset`, then `syscall.Kill(self, sig)` for the first captured signal, so its default action happens. Every deferred signal's default action is termination, so re-raising the first one gives the SPEC outcome.
- **Span isolation for children (F5), Linux.** `unix.PthreadSigmask(SIG_BLOCK, {INT, TERM, HUP})` on the locked spawner thread for the span's duration. The Go runtime saves the forking thread's mask in `syscall_runtime_BeforeFork` and restores it in the child, so the clients inherit the blocked set as SPEC §6.5 states.
- **Span isolation for children (F5), darwin.** `syscall.RawSyscall(unix.SYS___PTHREAD_SIGMASK, how, uintptr(unsafe.Pointer(set)), uintptr(unsafe.Pointer(old)))` on the locked spawner thread (syscall 329; `SIG_BLOCK` = 1, `SIG_SETMASK` = 3, `sigset_t` = `uint32`). exp-macos showed it is per thread and that direct-exec children inherit the blocked set, so SPEC §6.5 holds on darwin without cgo. `SYS_SIGPROCMASK` (48) is never used: on XNU it changes the mask of every thread of the process, and children of unrelated goroutines inherited it (28–40 of 80). The call lives in `internal/core/time_darwin.go`, the single audited `unsafe.Pointer` site.
- **Spawner discipline (both OSes).** The spawner goroutine calls `runtime.LockOSThread`, blocks the span signals, forks every in-span child itself, and restores the previous mask with `SIG_SETMASK` before it unlocks; alternatively it ends while still locked, so the runtime discards the thread. Ordinary goroutines then never fork with the span mask (exp-macos: 0 of 80).
- **Process identity.** Use `/proc/<pid>/stat` field 22 plus `boot_id` on Linux. On darwin, `unix.SysctlRaw("kern.proc.pid", pid)` with length 0 means the process is absent, and `SysctlKinfoProc`/`SysctlKinfoProcSlice("kern.proc.pgrp", pgid)` supply the typed fields (no `unsafe` in cafleet code). `SysctlKinfoProc` alone reports an absent PID as `EIO`. Zombie detection uses `Z` / `SZOMB`; `P_starttime` stays stable from running to zombie.
- **PTY on darwin (kernel behavior, exp-macos).** Three properties differ from Linux:
  - A master read at end of stream returns 0 and never `EIO`.
  - Output the master has not read is discarded when the last slave fd closes.
  - A session-leader child with unread output does not finish exiting while another slave fd stays open.

  F22–F25 encode the design rules that follow. The Rust runner already drops its slave after spawn (`src/runtime/process.rs:430`), reads the master continuously and maps `EIO` to EOF (`:663-665`). It therefore likely lives under the same kernel behavior, including the loss of output a child writes in its last moments before the master reads it. This is unverified for the Rust binary.
- **Files and sockets.** `O_NOFOLLOW|O_CLOEXEC|O_EXCL` with mode `0600`, directories `0700`, `Lstat` dev/ino/uid/mode checks and `flock` locks, as today.

### SPEC amendments

A5, A9 and A10 land on `main` with their pre-baseline fixes, and A8 lands on `main` with R4 (Implementation Step 3). The rest land on the branch in Step 5, before any Go production file. Each amendment is true of the Rust binary when it lands. That holds either because the pre-baseline fixes made it true, or because the amendment only restates Rust behavior or loosens a contract the Rust binary already exceeds.

| # | SPEC section | Amendment |
|---|---|---|
| A1 | §6.3 Error Messages and each action owner | Add every user-visible error string the Rust binary emits that SPEC lacks, as the migration harness collects them (Implementation Step 4). The known set: `database URL must use the sqlite scheme (sqlite:///<path>); got '<url>'`, `HOME is not set; cannot derive the default database path`, `HOME is not valid UTF-8; cannot derive the default database path`, `failed to open database at '<path>': <sqlite message>`, `cannot inspect database at '<path>': <io error>`, `database error: <sqlite message>`, `operation exceeded its deadline`, `--file -: file is not valid UTF-8.` and the Hermes `has no Hermes session record; member was not deleted.` row |
| A2 | §6.4 (new paragraph beside Quoted text) | Restate the library-defined cause texts as behavior: an OS error cause is the platform's error description with its first letter capitalized, followed by ` (os error N)`. An SQLite cause is SQLite's own message text with no prefix. A child exit status is `exit status: N`, or `signal: N (SIGNAME)` |
| A3 | §6.3 Database selection | `--database-url` may appear once per command level; a repeat at the same level is a parse error, and across levels the deepest occurrence wins |
| A4 | §6.3 Schema-version guard | Usage validation of a text body (positional, `--file`, `--file -`) runs after the schema-version guard |
| A5 | §6.3 Asset installation | The skills line lists skill names in bytewise order of their full relative paths: `installed cafleet-design-doc, cafleet (v<version>)` in every example |
| A6 | §6.5 Termination during pane input | Append one paragraph: "Signal dispositions and blocked signals that cafleet inherits from its parent are not part of this contract; cafleet may reset any of them for itself and for the processes it starts." The existing sentence that in-span multiplexer clients inherit the blocked set stays unchanged on every platform |
| A7 | §6.5 Subprocess output and deadlines | Replace the loop mechanics (nonblocking reads, 64 KiB per stream per loop, 20 ms polls) with the observable guarantees: concurrent drain from spawn, a deadline-exact timeout, exited vs. not exited, kill and reap of the direct child, both pipes closed, interrupted operations retried against the same deadline and the decoding rule. Keep the second paragraph unchanged |
| A8 | §6.9 Pattern syntax and §6.9 Native version probe | Remove the Perl classes `\s \S \d \D \w \W` and boundaries `\b \B` from the syntax. Every built-in pattern SPEC quotes uses explicit bracket classes: the Hermes version line becomes `^Hermes Agent v[^\t\n\v\f\r ]+ \((?P<version>[^)]+)\)$` (SPEC.md:2121). The bundled-catalog guard already rejects the removed constructs |
| A9 | §6.7 Hermes availability preflight | `hermes config path` runs under the smaller of five seconds and the remaining creation deadline. The concurrent native version probe keeps its two seconds |
| A10 | §6.9 Screen model facts | The screen model is at least two columns wide: a one-column terminal is modelled as two columns, while the child PTY keeps the pane's size |
| A11 | §6.3 Help text / Error Messages | No change in wording. The Go parser emits the SPEC-required first `error: ` line, empty stdout and exit 2, followed by a `Usage:` line; clap's `tip:` suggestions are not reproduced. Those lines are already non-normative |

`SPEC.md` names no implementation language, library or build tool today (`grep -i -w "rust|cargo|clippy|crate"` finds none), and the rewrite adds none. §7.6 Packaging & distribution already describes target-triple archives and offline embedding, so it needs no edit.

### Pre-baseline Rust fixes

The fixes land together in one PR on `main` (branch `fix/go-rewrite-pre-baseline`), one commit per fix, each updating SPEC first when the fix changes a contract. They land before the branch is created, so the oracle and the Go port start from fixed behavior.

| # | Bug | Fix | Test |
|---|---|---|---|
| R1 | `hermes config path` takes 2.40–2.55 s under load, beyond the 2 s preflight deadline, so `member create` fails with `did not finish within its deadline` | Amendment A9: the `config path` probe gets its own five-second bound inside the creation deadline (`src/cli/member.rs:484-491`) | A probe stub that sleeps 3 s succeeds; one that sleeps past 5 s fails with the existing error |
| R2 | At 30x20 the hermes composer text absorbs the status row (` ☤ gpt-6-luna · 3m`) and drops the space at the soft-wrap point | Adjust the hermes profile's composer region in `assets/profiles/hermes.json` so the status row is excluded and the wrap-point space is kept, with a new captured 30x20 fixture. If no region rule can separate them, record the geometry as a hermes layout limit in the profile and SPEC instead | Fixture replay reads the draft back exactly |
| R3 | vt100 panics at `screen.rs:1110` on a one-column grid (`☺` U+FE0F U+FE0F at 1×1), and the runner accepts any terminal of at least 1×1 (`src/runtime/mod.rs:137-143`) | Amendment A10: `TerminalState` sizes the vt100 screen to at least two columns on creation and on every resize, while the PTY keeps the pane size | Feeding `e298baefb88fefb88f` at 1×1 and resizing 1→2→1 columns leaves the runner live |
| R4 | Three code regexes use Perl classes that amendment A8 removes: `src/coding_agent/hermes.rs:312` (`\d`), `hermes.rs:320` (`\s`) and the Hermes version pattern `src/coding_agent/mod.rs:677` (`\S`), which SPEC.md:2121 quotes | Amendment A8 first, then rewrite the three patterns with explicit ASCII classes (`[0-9]`, `[\t\n\v\f\r ]`, `[^\t\n\v\f\r ]`) | Existing hermes and native-version-probe tests pass unchanged |

Amendment A5 is also a pre-baseline SPEC fix: it records the order the binary already prints. `assets/skills/cafleet/reference/cli.md` carries the Hermes `config path` error text but not its bound, so A9 changes no `cli.md` fact; Step 3 confirms this.

### Build, lint, CI and release

mise tasks (the root `mise.toml` keeps the task names; `./...` excludes `docs/` through the `go.mod` `ignore` directive):

| Task | Command |
|---|---|
| `//:test` | `go test -race -timeout 30m ./...`, with `CGO_ENABLED=1` set on Linux only (extra arguments after one standalone `--` are forwarded, e.g. `mise //:test -- -run 'TestRuntimeService_PreviewSweep'`) |
| `//:test-shipped` | `CGO_ENABLED=0 go test -timeout 30m ./...` (the shipped build configuration, without the race detector) |
| `//:coverage` | `go test -cover -timeout 30m ./...` |
| `//:lint` | `CGO_ENABLED=0 go vet ./...`, then `golangci-lint run ./...`, then `golangci-lint fmt --diff` |
| `//:vulncheck` | `govulncheck ./...` (needs network; CI runs it in the lint job) |
| `//:format` | `golangci-lint fmt` |
| `//:typecheck` | `go vet ./...` |
| `//:build` | `CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o dist/cafleet ./cmd/cafleet` |
| `//:install` | `CGO_ENABLED=0 go install ./cmd/cafleet` (installs to `$GOBIN`, default `~/go/bin`) |

The race detector needs cgo on Linux, so `//:test` on Linux needs a system C compiler (gcc on the CI runners). On macOS the race detector runs without cgo. The `CGO_ENABLED=0` vet in `//:lint` and `//:test-shipped` catch code that compiles only with cgo (for example `import "C"` or a cgo-only standard-library variant).

**Test timeout.** The one-package layout makes the whole suite one test binary, and `go test` aborts a binary at its `-timeout` (default 10 minutes). The Rust suite takes about 4 minutes, and `-race` slows execution 2–20×, so every test task passes an explicit `-timeout`. It starts at 30 minutes. At each stage gate (Steps 8–12), measure the full `//:test` wall time on the slower of the two CI runners and set the tasks' `-timeout` to three times that time, rounded up to the next 5 minutes. Record the measurement in the step's PR description. Parallelism inside the binary comes from `t.Parallel()` (default `-parallel` = `GOMAXPROCS`).

`.golangci.yml` enables: `govet` (with `nilness`), `staticcheck`, `errcheck`, `exhaustive` (`default-signifies-exhaustive: false`), `gochecksumtype` (`default-signifies-exhaustive: false`), `nilnil`, `gosec` (G115), `predeclared`, `depguard` (`unsafe` and `C` denied, with the single `internal/core/time_darwin.go` exclusion), `paralleltest` and `tparallel`, plus the `gofmt` formatter.

| File | Change |
|---|---|
| `mise.toml` | Step 7: add the three Go tools and `core:python` (branch only, for `migration/`); remove `core:zig` and `cargo:cargo-llvm-cov`; keep `core:rust` for the oracle; rewrite every task as above. Cutover: remove `core:rust` and `core:python` |
| `.github/workflows/ci.yml` | Step 7: cache `$(go env GOCACHE)` and `$(go env GOMODCACHE)` keyed on `go.sum`; drop `CARGO_PROFILE_TEST_OPT_LEVEL`; the lint job also runs `mise //:vulncheck`; the `ubuntu-latest` test job also runs `mise //:test-shipped`. Same `{lint,test} × {ubuntu-latest, macos-15}` matrix. Step 4 adds the branch-only `migration` job, which the cutover removes |
| `.github/workflows/release.yml` | Cutover: read the version from `internal/core/version.go` (`sed -n 's/^const Version = "\(.*\)"$/\1/p'`). Per matrix target, build with `CGO_ENABLED=0 GOOS=<os> GOARCH=<arch> go build -trimpath -ldflags "-s -w" -o dist/<target>/cafleet ./cmd/cafleet`, replacing `rustup target add`, `pipx install cargo-zigbuild` and `cargo zigbuild`. Archive and bottle names are unchanged; the archive also holds `THIRD_PARTY_LICENSES` |
| `.bumpversion.toml` | Cutover: one file entry, `internal/core/version.go`, `const Version = "{current_version}"` |
| `.github/renovate.jsonc` | Cutover: replace the `cargo` rule with `gomod` (`postUpdateOptions: ["gomodTidy"]`, patch group `go`) |
| `.gitignore` | Step 7: add `!.golangci.yml` (the file would otherwise match `.*`) and `/dist/`. Cutover: remove `/target/` |

Target mapping: `aarch64-apple-darwin` = `darwin/arm64`, `x86_64-unknown-linux-musl` = `linux/amd64`, `aarch64-unknown-linux-musl` = `linux/arm64`.

### Migration tooling

`migration/` is committed on the branch and deleted at cutover. It contains Python 3 scripts (pinned by `core:python` in `mise.toml` on the branch) and one Rust crate. It contains no Go code, so the module's `./...` never sees it.

The no-fixture-file and no-`testdata/` rule governs the permanent tree, the tree that remains after cutover. `migration/` is temporary tooling outside that tree: the "Migration model" row of § Decisions and § Migration model item 4 record the user's decision to keep migration tooling outside the permanent tree and delete it at cutover. The data files it holds therefore sit outside the rule. The only committed data files are the scrubbed real streams; everything else is generated at run time.

| Path | Content |
|---|---|
| `migration/harness/` | The exp-interop differential harness, extended to every §6.3 command and `__member-run`, with stand-in `tmux`, `herdr` and native executables (G1, G2) |
| `migration/vtoracle/` | A Rust crate pinned to `vt100 =0.18.1`, `vte =0.15.0`, `unicode-width =0.2.2` with its own `Cargo.lock`, printing the per-step canonical dump hashes (G3) |
| `migration/corpus/` | The scrubbed, approved real streams as gzip-compressed JSONL (one case per line: size, timestamped output/resize events): `live.jsonl.gz` and `capture.jsonl.gz` |
| `migration/fuzz.py` | The exp-vt seeded fuzz generator, with its fixed seeds as constants. The `migration` job generates the grammar and random families from the seeds alone, and the mutated family from the seeds plus `migration/corpus/`, producing at least 2 MB per run |
| `migration/cases.py` | Generates the fixture cases from `assets/profiles/` and the synthetic edge cases at run time |
| `migration/diffrun.py` | The exp-vt per-step comparator |
| `migration/golden.py` | Runs the Rust detector golden test and writes its JSONL reference (G4) |
| `migration/testmap.py` | The G5 test map |
| `migration/scrub.py` | The stream scrubber (§ Captured-stream privacy) |
| `migration/check_version.py` | Asserts `internal/core/version.go` equals the `Cargo.toml` package version |

The detector golden reaches `Detector::observe` and `composer_text` through an ignored, environment-driven `#[cfg(test)] mod capture_golden` added to `src/coding_agent/detector.rs` on the branch, as in exp-capture. `migration/golden.py` runs it with `cargo test --bin cafleet capture_golden -- --ignored`. The binary's observable surface does not change, and the module disappears with `src/` at cutover.

### Captured-stream privacy

The branch is pushed to GitHub, so every committed stream is public, including `migration/corpus/` history. Before the first commit of any stream:

1. `migration/scrub.py` normalizes each stream. Every match of the following is replaced with a placeholder of identical byte length and display width, so layouts and wrap points are preserved:
   - absolute paths under the capturing user's home, the repository root and `/tmp`;
   - user, host and account names;
   - `work/<name>` and other experiment directory names;
   - session and pane titles that name the host or user.
2. The scrubber reruns the exp-capture secret scan (key, bearer, JWT, AWS and private-key shapes) and lists every remaining typed prompt and model reply.
3. The Director reviews the rendered screens of the scrubbed streams, and the user signs off on each stream before its first commit.

The oracle references (G3, G4) and the frozen golden files are produced from the scrubbed streams.

### Testing

**Placement (the rewritten `.claude/rules/code-quality.md` § Test placement).**

- A test lives in `<file>_test.go` beside the implementation file whose behavior it asserts, in the same package (white-box). Related tests of one Rust nested `mod` stay together in that file.
- A helper or test-only constructor shared by tests of several files lives in the `_test.go` of the file that owns the type it builds; a helper used by one file's tests stays in that file's `_test.go`. Because the crate is one package, every test sees these helpers, which covers what Rust `test_support` modules and `#[cfg(test)] impl` blocks provide today. Crate-wide test helpers (the re-exec child naming that replaces `test_path!` and the `-update-golden` flag) live in `internal/core/main_test.go`.
- Test inputs are literals or generated bytes in `_test.go` source. No data file and no data directory (`testdata/` included). Tests read `assets/` through `cafleet.Assets` only as input to production code or as the expected value of its output.
- A golden test over a frozen native capture records what production code reports for it: screen hashes, observation kinds, rule ids, regions, operations, choices and composer text. These are outputs of production code, regenerated with `-update-golden` and reviewed as behavior diffs. They are the only tests that record rule ids from the embedded profiles.
- A re-executed child test runs `os.Args[0] -test.run '^<name>$'` with an environment guard and `t.Skip` when the guard is unset. A helper derives the name from the function value, so renames stay safe.
- Related cases form one table-driven test over a named case slice, naming the case in every assertion message.
- Runner code takes time from the injected `clock`, and its tests use `testClock`. Real time is used only by tests that drive real processes, PTYs, sockets or locks. Runner behavior is driven through the scenario harness in `internal/core/runtime_service_test.go`.
- Every test and subtest calls `t.Parallel()` unless it changes process-wide state (environment, working directory, umask, signal handlers, rlimits). `paralleltest` and `tparallel` enforce this.

**Porting the Rust tests.** Port every Rust `#[test]` case by case into the `_test.go` of the Go file that the file mapping gives its Rust source file. The Go name is `Test<File>_<Fn>` for a test directly in `mod tests`, and `Test<File>_<Nested1>_…_<NestedN>_<Fn>` for one in nested modules. `<File>` is the CamelCase Go file stem, and each other part is the CamelCase of the Rust module or function name. Examples:

- `runtime::service::tests::preview_sweep::an_un…` → `TestRuntimeService_PreviewSweep_AnUn…` in `internal/core/runtime_service_test.go`;
- the same-named tests in `src/runtime/input.rs:7270` and `src/runtime/service.rs:8279` become `TestRuntimeInput_…` and `TestRuntimeService_…`.

Rust test paths are unique, so the names are unique; `migration/testmap.py` also rejects any duplicate. G5 checks both the Go file and the test name.

The only allowed omissions are tests that assert Rust-library internals (for example clap `ErrorKind`). Each omission names the Go test that asserts the same SPEC property.

The 7k-line `native_capture` tooling (`src/runtime/observation.rs:1020-7993`) is ported as env-gated tests in `internal/core/runtime_observation_test.go`, beside `runtime_observation.go`. It uses the exp-capture recorder topology: a PTY recorder inside a private multiplexer pane that forwards the pane's query answers to the child.

### Frozen parity inputs

These tests stay after cutover as permanent regression tests, each in the `_test.go` of the file that owns the behavior it asserts. Each expected-value block sits between `// BEGIN GENERATED: mise //:test -- -run '<TestName>' -update-golden` and `// END GENERATED` lines. The update mode rewrites only that block, from the current Go output; the rest of the file is hand-written.

| Owner file | Content | Asserted behavior |
|---|---|---|
| `internal/core/runtime_transport_test.go` | The scrubbed backend capture streams (the exp-capture recordings and the exp-vt claude, codex and pi live streams) as Go string literals of their timestamped events, under a comment recording the scrub and approval date. A generated block holds, per checkpoint (each capture checkpoint plus each stream's final state), the FNV-1a hash of the canonical screen dump and a chain hash over every observation step since the previous checkpoint | `TestRuntimeTransport_CaptureScreens`: `TerminalState` replays each stream with its per-step split; on mismatch it prints the dump at the failing checkpoint |
| `internal/core/coding_agent_detector_test.go` | A generated block holding, per capture checkpoint: kind, `rule_id`, `match_evidence`, `verified_noninteraction`, matched region, operation, choices and composer text | `TestCodingAgentDetector_CaptureGolden`: the `Detector` observes the screens `TerminalState` produces from the streams in `runtime_transport_test.go` (same package scope) |
| `internal/vt/unicodewidth/tables.go` | unicode-width 0.2.2 non-CJK tables, as production code with a `// Code generated from unicode-width 0.2.2 tables.rs; DO NOT EDIT.` header | Width tests in `internal/vt/unicodewidth/width_test.go` |

The canonical screen dump is the exp-vt oracle serialization: size, cursor, the 8 mode fields, current attrs, raw title/icon bytes, `contents()`, each row string with its wrap flag, and every non-default cell's has_contents/wide/continuation, fg, bg, five attribute bits and contents bytes. The dump function in `runtime_transport_test.go` transliterates `migration/vtoracle`'s dump.

Maintenance:

- **On the branch.** The generated blocks are written by the update mode only after G3 and G4 pass at that merge (§ Migration model step 2), so the Go output they record equals the Rust oracle's.
- **After cutover.** A change to the emulator, the detector or a profile that alters a recorded value regenerates the block with the update mode in the same PR, and the reviewed golden diff is part of that behavior change. Adding a new capture follows § Captured-stream privacy and the update mode. Removing a capture needs the design doc of the change that makes it obsolete.

### Parity gates

All six gates must pass on the final merge of `main` before the cutover change. G1–G5 run in the branch-only CI job `migration`, on `ubuntu-latest` and `macos-15`.

| Gate | Check | Pass condition |
|---|---|---|
| G1 Command parity | `migration/harness` runs every §6.3 command through both binaries, in same-path worlds with stand-in `tmux`, `herdr` and native executables that log argv and answer from scripts. It compares exit code, stdout bytes, stderr bytes, stand-in argv logs and normalized database dumps, and covers `/dev/full` and closed-pipe sinks | 0 differences. Parse errors compare exit code, empty stdout, the first stderr line and the accepted-value list; help compares exit 0 and the strings SPEC quotes |
| G2 Runner parity | `migration/harness` launches `__member-run` from each binary under the determinism rules below | Normalized `member_runs`, event and input rows and the child's received bytes are identical per scenario |
| G3 Emulator parity | `migration/vtoracle` vs `internal/vt`, hashing after every observation step | 0 diffs on the 624 fixtures, the synthetic cases, every live and capture stream for all five backends and ≥ 2 MB of seeded fuzz (grammar, random and mutated families); every per-scalar and 500,000 string-width checks equal |
| G4 Detector parity | `migration/golden.py` exports the Rust `Detector::observe` and `composer_text` results for every bundled fixture and capture checkpoint | Go results equal on every field |
| G5 Test map | `migration/testmap.py` lists every Rust `#[test]` on the merged head and its Go counterpart by the file mapping and naming rule | No unmapped test outside the omission list; no duplicate Go name |
| G6 Live acceptance | The Director re-runs docs `contributing.md` § Release acceptance and the Director delivery probe against the branch's Go binary, for every bundled backend on tmux and herdr | Records complete, with native and multiplexer versions and dates |

**G2 determinism.**

- The fake native program is deterministic: it draws fixture screens in response to the bytes it receives, with no timers of its own.
- Each scenario step waits for quiescence before the next one. Quiescence means that the control-socket `health` reply shows no pending input, the same `output_seq` on two polls 500 ms apart, and a stable observation.
- Normalization replaces timestamps with first-occurrence ordinals (the exp-interop scheme), pids and socket paths with ordinals, and durations with a placeholder.
- Before Go is compared, every scenario runs three times against the Rust oracle with 0 diffs. A scenario whose Rust runs differ depends on timing (settle windows, pre-submit races, sweep ticks). It leaves G2, and its behavior must be covered by a clock-driven white-box test in G5.

**G6 isolation.**

- The Director starts the G6 Director session from a shell whose `PATH` begins with the absolute directory holding the branch's `dist/cafleet`. That `PATH` is inherited by the multiplexer panes, the member runners and the native agents, so every bare `cafleet` command, the Director's and every member's, runs the Go binary.
- Every command passes a dedicated `--database-url sqlite:///<fresh short dir>/g6.db`.
- Each G6 record includes `command -v cafleet` and `cafleet --version` output from the Director and from one member of each backend, showing the `dist/cafleet` path.
- `mise //:install` is not run, and `cafleet setup` never targets the user's configuration homes, so the installed Rust binary, its default database and its installed assets stay untouched.
- pi and opencode extensions run the recorded executable. Their cases use an isolated configuration home prepared by the Go binary's `cafleet setup`, so the executable record names the Go binary. The user approves which credentials that home receives.

### Documentation and rules

| File | Change |
|---|---|
| `SPEC.md` | Amendments A1–A11 |
| `assets/skills/cafleet/reference/cli.md` | The error rows and database-selection facts it carries for A1, A3 and A4 (Step 5). A9 changes no `cli.md` fact (Step 3 confirms) |
| `docs/docs/contributing.md` | Project-structure `src/` row → the Go module (`cmd/`, `internal/`, root `assets.go`, `go.mod`). "Rust boundaries" → "Go boundaries", with the test sentence restated for `_test.go`. Development paragraph: Go validation, no `CARGO_PROFILE_TEST_OPT_LEVEL`, and the C compiler that `//:test` needs on Linux. A switch-over note for existing contributors: run `cargo uninstall cafleet`, put `~/go/bin` (or `$GOBIN`) first on `PATH`, run `mise //:install`, then re-run `cafleet setup` so the extensions' executable record names the new binary. The unsafe/PTY paragraph: cafleet uses no cgo and no `unsafe` except the single audited darwin `__pthread_sigmask` call in `internal/core/time_darwin.go`, `creack/pty` for PTY allocation, and the in-repo emulator transliterated from vt100 0.18.1 / vte 0.15.0 with its notices in `third_party/`. The bundled-catalog guard location → `internal/core/coding_agent_catalog_test.go` |
| `README.md` | No change: the pitch, install commands and archive names stay |
| Other `assets/skills/` files | No change: the `#[cfg(test)]` mentions in the design-doc programmer and tester roles are examples for any project |
| `CLAUDE.md` | Project: "The Go module at the repository root builds the unified CLI with the standard library, ncruces go-sqlite3 and an embedded SQL schema." Tests: "Write every Go test in the `_test.go` file beside the implementation file it exercises, in the same package; never add a test data file or directory." |
| `.claude/rules/code-quality.md` | Banned-pattern table: Go rows replace the Rust rows (`v := m[k]` where presence is required → `v, ok := m[k]` plus a panic naming the invariant; `x, _ := f()` → handle or return the error; returning a default with a nil error on failure → return the error). § Test placement is replaced by the placement rules in § Testing, including the golden-test rule. § Database schema names the `schemaSQL` constant in `internal/core/db.go` |
| `.claude/rules/commands.md` | "Rust tasks" → "Go tasks"; required validation is `mise //:test` and `mise //:lint`. The test-filter sentence becomes `mise //:test -- -run '<regexp>'` with one standalone `--`, which mise consumes |
| `.claude/skills/clean-docs/SKILL.md` | "the repository's Rust validation tasks" → "the repository's validation tasks" |

### Risks and decision gates

| Risk | Mitigation |
|---|---|
| A regression to the process-wide `SYS_SIGPROCMASK` on darwin would leak the span mask into unrelated children | The darwin per-thread and sibling-unaffected tests in F5 fail loudly on `macos-15` CI |
| darwin discards PTY output a child writes just before its last slave fd closes, if the master has not read it yet (kernel behavior; likely shared by the Rust runner, unverified) | F22–F24 keep the master reading continuously, so the window is the same as today's. Measuring it needs a live macOS probe with a backend that prints a final line on exit. That probe is outside the gates, because the behavior is the kernel's and unchanged from the Rust binary |
| ncruces is pre-1.0, generated (wasm2go) and effectively single-maintainer | Pin the version and keep every driver call in `internal/core/db.go`. If ncruces becomes unusable, switching to `modernc.org/sqlite` is a re-planning event, not a drop-in change. It needs a re-armed timer in place of the busy handler for F20, and a `Done()` channel for the 1,000-step cancellation. Its database/sql `Commit` rolls back silently, so `GetAutocommit` detection and its diagnostic change (a SPEC-visible change), and it adds about 10 non-stdlib packages, relaxing the module success criterion. That switch gets its own design doc |
| Runner port effort (largest unit) | Track Step 11 against the 16–25 person-week band at each sub-machine boundary. Above 26 person-weeks, re-plan before continuing |
| Type-safety loss in the `InputCoordinator` and other state machines | Transliteration rules, linters, the per-sub-machine mutation check in Step 11, and `-race` |
| Load-sensitive real-time tests (3/1,468 flaked under load in Rust) | Real time only in OS-level tests, with tolerant deadlines; the `t.Parallel()` policy keeps clock-driven tests deterministic |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Preserve the study evidence

- [x] Copy `/tmp/go-rewrite/` (minus build targets and module caches) into `researches/go-rewrite/poc/` (gitignored), keeping the subtree layout <!-- completed: 2026-11-24T08:58 -->
- [ ] Confirm that each reproduction command in exp-vt.md, exp-capture.md and exp-interop.md runs with `researches/go-rewrite/poc/` in place of `/tmp/go-rewrite/` <!-- completed: -->

### Step 2: Record the macOS validation results

- [x] Fold `researches/go-rewrite/exp-macos.md` into § Signals and processes (the `SYS___PTHREAD_SIGMASK` route), A6, the depguard exclusion, the `contributing.md` row and the Background findings <!-- completed: 2026-11-24T09:40 -->
- [x] Record the cross-built binary, raw-mode, `SZOMB`/`SysctlRaw`, OFD-lock and darwin PTY results as F16 and F22–F25 and in § Risks and decision gates <!-- completed: 2026-11-24T09:40 -->

### Step 3: Pre-baseline Rust fixes on main

- [ ] R1: SPEC amendment A9 and the separate `hermes config path` bound, with its tests; confirm that `cli.md` carries no A9 fact <!-- completed: -->
- [ ] R2: hermes narrow-composer profile fix with a captured 30x20 fixture, or a recorded hermes layout limit in the profile and SPEC <!-- completed: -->
- [ ] R3: SPEC amendment A10 and the two-column screen floor in `TerminalState`, with the 1×1 repro test <!-- completed: -->
- [ ] R4: SPEC amendment A8 (pattern syntax and the quoted Hermes version line), then explicit ASCII classes in `hermes.rs:312`, `hermes.rs:320` and `mod.rs:677` <!-- completed: -->
- [ ] SPEC amendment A5 (setup skills-line order) <!-- completed: -->

### Step 4: Branch, migration tooling and baseline

- [ ] Create `feat/go-rewrite` from `main` and open its draft PR <!-- completed: -->
- [ ] Add `migration/` per § Migration tooling, with the `capture_golden` test module in `src/coding_agent/detector.rs` <!-- completed: -->
- [ ] Scrub every stream with `migration/scrub.py`; the Director reviews the rendered screens and the user signs off on each stream; only then commit `migration/corpus/` <!-- completed: -->
- [ ] Extend the harness from exp-interop's four commands to every §6.3 command and `__member-run`, with stand-in multiplexer and native executables and the G2 determinism rules <!-- completed: -->
- [ ] Run G2's three-run Rust baseline and drop every scenario whose Rust runs differ, listing each dropped scenario for G5 coverage <!-- completed: -->
- [ ] Run the harness against the Rust binary alone and list every observed output that SPEC does not state; this list completes amendment A1 <!-- completed: -->
- [ ] Add the branch-only `migration` CI job on `ubuntu-latest` and `macos-15`, including `check_version.py` <!-- completed: -->

### Step 5: SPEC amendments

- [ ] A1: the error strings from Step 4, at their owning sections <!-- completed: -->
- [ ] A2: library-defined cause texts <!-- completed: -->
- [ ] A3: the `--database-url` repeat rule <!-- completed: -->
- [ ] A4: body validation after the schema guard <!-- completed: -->
- [ ] A6: inherited signal dispositions and blocked signals are not part of the contract <!-- completed: -->
- [ ] A7: observable subprocess-runner guarantees <!-- completed: -->
- [ ] Update `assets/skills/cafleet/reference/cli.md` for the facts it carries from A1, A3 and A4 <!-- completed: -->

### Step 6: Documentation and rules

- [ ] `docs/docs/contributing.md` edits from § Documentation and rules, including the contributor switch-over note; run `mise //docs:build` <!-- completed: -->
- [ ] `CLAUDE.md` Project and Tests sections <!-- completed: -->
- [ ] `.claude/rules/code-quality.md` banned patterns, Test placement (with the golden-test rule) and Database schema <!-- completed: -->
- [ ] `.claude/rules/commands.md` task wording and test-filter form <!-- completed: -->
- [ ] `.claude/skills/clean-docs/SKILL.md` validation wording <!-- completed: -->

### Step 7: Go module skeleton and toolchain

- [ ] `go.mod` (with `ignore ./docs`), root `assets.go`, `cmd/cafleet/main.go`, `internal/core/main.go`, `internal/core/version.go` and the `third_party/` license texts <!-- completed: -->
- [ ] `mise.toml` tools and tasks per § Build, lint, CI and release <!-- completed: -->
- [ ] `.golangci.yml` with the linters in § Build, lint, CI and release; `.gitignore` `!.golangci.yml` and `/dist/` <!-- completed: -->
- [ ] `ci.yml` Go caching, the `//:vulncheck` step and the `//:test-shipped` step <!-- completed: -->
- [ ] `internal/core/main_test.go` with the re-exec child naming helper and the `-update-golden` flag <!-- completed: -->

### Step 8: Stage 1 — foundation, broker and CLI surface

- [ ] `error.go`, `time.go` (clock, test clock, deadlines, deferral spans), `config.go`, `digest.go` <!-- completed: -->
- [ ] `textcodec.go`: F6–F8, F10–F11 and F17 helpers; F9 and F18 in their Rust-home files <!-- completed: -->
- [ ] `db.go`: `schemaSQL`, connection pragmas, format diagnosis, deadline budget (F20), test fault plans on ncruces hooks <!-- completed: -->
- [ ] `broker*.go` including `broker_runtime*.go` row mapping <!-- completed: -->
- [ ] `output*.go` presenters and the CLI parser, help generation and parse-error rendering (A11) <!-- completed: -->
- [ ] `assets.go` (F12), `setup`, `doctor`, `message *`, `fleet list/show`, `member list/show`, `event`/`input`/`permission` read commands <!-- completed: -->
- [ ] G1 passes for every Stage 1 command <!-- completed: -->

### Step 9: Stage 2 — emulator, detector, backends and multiplexer

- [ ] `internal/vt` from the exp-vt port, with the read-only view and `CopyInto` (F15), generated width tables and per-file notices <!-- completed: -->
- [ ] `runtime_transport.go` `TerminalState` and its screen snapshot types; G3 passes; add the scrubbed streams and `TestRuntimeTransport_CaptureScreens` to `runtime_transport_test.go` and write its generated block with the update mode <!-- completed: -->
- [ ] `coding_agent_catalog.go` strict decode (F7, F21), validation, catalog runtime and the provenance guard test <!-- completed: -->
- [ ] `coding_agent_detector.go`, `coding_agent.go` (backend argv, version probe), `coding_agent_hermes.go` (F14) <!-- completed: -->
- [ ] G4 passes; add `TestCodingAgentDetector_CaptureGolden` to `coding_agent_detector_test.go` and write its generated block with the update mode <!-- completed: -->
- [ ] `multiplexer*.go`: tmux, herdr (F13), caller verification and layout <!-- completed: -->
- [ ] `pane_lock.go` <!-- completed: -->

### Step 10: Stage 3 — OS layer and lifecycle commands

- [ ] Startup fd hygiene (F1), SIGPIPE (F3), raw mode (F16), PATH lookup (F19) <!-- completed: -->
- [ ] Masked spawner (F5): `PthreadSigmask` on Linux, `SYS___PTHREAD_SIGMASK` in `time_darwin.go`, with the per-thread and sibling-unaffected tests on both CI OSes <!-- completed: -->
- [ ] `runtime_system.go` bounded subprocess runner, `runtime_process.go` PTY spawn (F2), master drain and EOF (F22–F24), process identity (F25) and zombie-pinned reaping (F4); private files, sockets and locks <!-- completed: -->
- [ ] `fleet create/delete`, `member create/delete/prompt/exec/press/capture`, `input reconcile`, `permission respond` <!-- completed: -->
- [ ] G1 passes for every §6.3 command <!-- completed: -->

### Step 11: Stage 4 — member runner

- [ ] `runtime_writer.go`, `runtime_artifacts.go` and the rest of `runtime_transport.go` <!-- completed: -->
- [ ] `InputCoordinator` staged composer write path (exp-runner slice), then the permission-answer, reservation/selection and deferred-queue sub-machines. For each sub-machine, add a scripted mutation check (including a funnel-bypass mutant) and record the elapsed effort against the 16–25 person-week band <!-- completed: -->
- [ ] `input_client`, control socket and wire, observation, recovery, lifecycle, preview and Director delivery sweeps, diagnostics, press <!-- completed: -->
- [ ] `runtime_service.go` and the scenario harness; port the runner scenario tests <!-- completed: -->
- [ ] `native_capture` port as env-gated tests in `runtime_observation_test.go` <!-- completed: -->
- [ ] G2 passes <!-- completed: -->

### Step 12: Stage 5 — test completion and stabilisation

- [ ] Port the remaining Rust tests until G5 passes, with the omission list reviewed <!-- completed: -->
- [ ] `mise //:lint`, `//:vulncheck`, `//:test` and `//:test-shipped` green on `ubuntu-latest` and `macos-15`, with every test seen failing in CI, and every test touched since the previous stage gate, passing `-count=20` when run alone with `-run` <!-- completed: -->
- [ ] Run G1–G5 on Linux and macOS against the oracle built from the latest `main` merge <!-- completed: -->
- [ ] Measure the stripped binary size and the RSS of one ncruces connection, and record both in the PR description <!-- completed: -->
- [ ] G6: the Director collects the live acceptance and Director delivery records against `dist/cafleet` under § Parity gates G6 isolation <!-- completed: -->

### Step 13: Cutover

- [ ] Final `main` merge through the § Migration model step 2 procedure; G1–G6 green <!-- completed: -->
- [ ] Delete `src/`, `Cargo.toml`, `Cargo.lock`, `build.rs` and `migration/`; remove `core:rust`, `core:python` and the `migration` CI job <!-- completed: -->
- [ ] `release.yml`, `.bumpversion.toml`, `.github/renovate.jsonc` and `.gitignore` edits from § Build, lint, CI and release, with `THIRD_PARTY_LICENSES` in each archive <!-- completed: -->
- [ ] Repository-wide check: no living document, rule, skill or comment mentions Rust, Cargo, clippy, rustfmt, `#[cfg(test)]` (outside the generic skill examples), `portable-pty`, `vt100` as a dependency, `migration/` or `~/.cargo/bin` <!-- completed: -->
- [ ] Merge the cutover PR, then run the bump workflow for the next minor version; confirm that the release job publishes all three archives and bottles <!-- completed: -->
- [ ] Download each published archive on its platform (macOS arm64, Linux amd64, Linux arm64) and, under a throwaway `HOME`, run `cafleet --version` and `cafleet setup --database-url sqlite:///<tmp>/c.db`; record the results <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-24 | Initial draft |
| 2026-11-24 | Reviewer round 1: one-package layout, darwin outcomes pending exp-macos, loosened A6, full R4/A8 scope, migration tooling, stream privacy, golden maintenance, G2 determinism, G6 isolation, cgo-free test configuration, version sync |
| 2026-11-24 | Reviewer round 2: identifier derivation rules, explicit test timeout, run-time fuzz generation, capture tests in their owners' `_test.go`, G6 `PATH` isolation |
| 2026-11-24 | Reviewer round 3 and exp-macos: darwin `SYS___PTHREAD_SIGMASK` route, macOS lock interop and raw-mode results, darwin PTY rules F22–F25 |
