# Make SPEC.md sufficient for an independent reimplementation

**Status**: Approved
**Progress**: 17/47 tasks complete
**Last Updated**: 2026-11-27

## Overview

Close every gap the 2026-11-27 three-part audit found between `SPEC.md` and the Rust binary, so that a team holding only `SPEC.md`, the embedded `assets/` tree and the external programs can build, in another language, a `cafleet` binary indistinguishable from the Rust binary on every normative output. Each gap is resolved by documenting the current behavior, declaring it implementation-defined, or fixing the Rust code where the behavior is a defect no specification should copy.

## Success Criteria

- [ ] Three fresh audit agents, one per audit area, comparing the post-change `SPEC.md` with the post-change source, report 0 `blocker` and 0 `needs-oracle` findings; every `minor` finding is resolved or listed in § Accepted minor findings.
- [ ] A blind agent with only `SPEC.md` and a copy of `assets/` implements the § Blind slice in Python 3 (standard library only), and the differential harness reports 0 differences against the Rust binary over every scenario of § Blind slice.
- [ ] Every literal in the § Error-literal sweep source set appears in `.sweep/error-literals.md` with a classification of `listed`, `reworded` or `internal`, the sweep's completeness check lists no unhandled site, and every `listed` and `reworded` literal appears verbatim in its owning `SPEC.md` section.
- [ ] `SPEC.md` § 9.2 holds the terminal conformance vectors; every row of the § Terminal emulation sequence table cites at least one vector ID, and the Rust conformance test that reads the vectors from `SPEC.md` passes.
- [ ] Every screen in `assets/profiles/*.json` carries `expected_observation`, and the bundled-catalog guard fails `mise //:test` when the detector's observation of any screen differs from it.
- [ ] `SPEC.md` contains no Rust format-string notation (`{:<`, `:?}`) and names no Rust crate or the implementation language; `grep -nwE 'vt100|vte|unicode-width|clap|serde|chrono|rusqlite|Rust|Cargo' SPEC.md` returns nothing, and both `grep -nF '{:<' SPEC.md` and `grep -nF ':?}' SPEC.md` return nothing.
- [ ] The signal-mask experiment result for Linux and macOS is recorded in row M1 and its chosen resolution has landed.
- [ ] `mise //:test` and `mise //:lint` pass at the end of every Implementation step and on the pull request of § Delivery, and `assets/skills/cafleet/reference/cli.md` carries no fact that contradicts `SPEC.md`.

---

## Background

The audit at `.audit/spec-audit-2026-11-27.md` (HEAD `a084881d`, cafleet 0.35.7) compared `SPEC.md` with the Rust source in three areas and concluded that SPEC is implementation-neutral in wording but not sufficient for a byte-exact reimplementation: the Rust binary is the de facto oracle. Its pointers were re-checked against `SPEC.md` at `a084881d` and remain accurate within a few lines.

| Area | SPEC sections | Blockers | Needs-oracle | Minor |
|---|---|---|---|---|
| 1 — CLI, output, broker, persistence | §1–§6.4 | 4 | 13 | 5 |
| 2 — Multiplexer, pane input, backends, configuration | §6.5–§6.8 | 1 (unverified) | 9 | 6 |
| 3 — Member PTY runtime, cross-cutting, schema, testing | §6.9–§9 | 4 | 5 | 5 |

Screen detection is the largest gap. Its de facto contract is three Rust crates (a VT parser, a screen model and a Unicode width table) plus `src/coding_agent/detector.rs`. The profile rule object and the matching algorithm are unwritten, and the bundled native screen fixtures record no expected observation, so they cannot serve as an oracle.

The aborted design `design-docs/0000245-go-rewrite` planned SPEC amendments A1–A11; A5, A8, A9 and A10 landed. This design adopts A1, A3, A4, A7 and A11 as starting points (rows C8, C13, C17, M5 and the § Error-literal sweep). A2 is not adopted because OS and library cause text stays implementation-defined. A6 is the fallback for row M1. The Go rewrite itself is out of scope.

---

## Specification

### Conformance policy

`SPEC.md`'s opening paragraph gains a normative/implementation-defined split that every later section relies on. Everything not listed as implementation-defined is normative and byte-exact:

| Surface | Status |
|---|---|
| Authored stdout and stderr text, JSON bytes and key order, text layouts, exit codes | Normative |
| Multiplexer and native argv, bytes written to a child PTY, §8 rows and their values | Normative |
| Process, lock, signal and deadline behavior stated as observable outcomes | Normative |
| Operating-system and library cause text after an authored prefix (`<io error>`, `<error>`, `<cause>`) | Implementation-defined (existing rule, kept) |
| `--help` layout, except strings SPEC quotes | Implementation-defined (existing rule, kept) |
| A parse error's stderr after the first line's `error: ` prefix and the named argument | Implementation-defined (A11) |
| A diagnostic for a violated internal invariant that no conforming environment reaches (sweep class `internal`); the process exits non-zero | Implementation-defined (new) |
| The `__member-run` descriptor file exchanged between processes of one binary version | Implementation-defined (new, row R12) |
| Internal scheduling not stated as a constant (idle tick, drain-loop mechanics) | Implementation-defined (rows R12, M5) |

### Resolution buckets

Each part of a row's resolution belongs to one bucket. A row whose resolution has several parts names each part's bucket in its Resolution text (for example "Document: … Fix code: …"), and the Bucket column lists them in that order.

| Bucket | Meaning | Change |
|---|---|---|
| Document | The current behavior is sensible; SPEC states it | `SPEC.md` (and `cli.md` when it carries the fact) |
| Loosen | Nothing observable depends on it; SPEC declares it implementation-defined or states only the observable guarantee | `SPEC.md` |
| Fix code | The behavior is a defect; the Rust code changes to a behavior SPEC can state cleanly | `SPEC.md` first, then Rust code with in-file tests |
| Pending experiment | A temporary state for a row whose bucket depends on an experiment (row M1 only); Step 1 replaces it with one of the buckets above | — |

A row's code pointer is the `src` / `Code` column of its audit finding (the `#` in the Finding column) in `.audit/spec-audit-2026-11-27.md`, re-located by symbol when line numbers have moved. For a Document or Loosen part, the implementer reads that code and writes what it does, and the code wins when it disagrees with the part's text. For a Fix code part, the part's text wins and the code changes. The implementer records each disagreement in the Changelog.

### Decision table — Area 1 (§1–§6.4, §7)

| ID | Finding (audit 1 #) | Bucket | Resolution | Owning section |
|---|---|---|---|---|
| C1 | `fleet list` / `fleet show` JSON keys unspecified (#1) | Document | `fleet list` JSON is an array of objects ordered `fleet_id, name, created_at, director_member_id, member_count`; `member_count` counts the fleet's `active` members, root Director included. `fleet show` JSON is ordered `fleet_id, name, created_at, deleted_at, director_member_id` | §6.3 Output shapes |
| C2 | `fleet list` / `fleet show` text layout (#2) | Document | `fleet list`: header `FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT`, then one row per fleet with the five values joined by one U+0020 and no padding; no fleets prints `No fleets found.`. `fleet show`: lines `fleet_id: `, `name: `, `created_at: `, then `deleted_at: ` only when non-null | §6.3 `fleet list`, `fleet show` |
| C3 | Queued prompt/exec third line paraphrased (#3) | Document | Quote verbatim: `The member's runner will retry it after the blocking work ends; run cafleet input show {id} --json for its result.` with `{id}` the input ID | §6.3 Runtime presentation |
| C4 | `member delete` text (#4) | Document | Three lines: `Member deleted.`, `  member_id: N`, `  pane_id:   STATUS` (three spaces after `pane_id:`) | §6.3 `member delete` |
| C5 | Poll envelope separator; body line (#5) | Document | Envelopes are joined by one empty line; the body line is always present. Text-mode projection drops "when nonempty" | §6.4 Text-mode projection |
| C6 | Roster heading and Rust format string (#6) | Document | Heading `N members:` for N ≠ 1, `1 member:` for one row, `0 members.` for none. Replace the format string with a behavioral rule: two leading spaces, columns separated by two spaces, each of the first six columns left-aligned and right-padded with U+0020 to widths 9, 13, 8, 8, 13, 7 counted in Unicode scalars, never truncated, and the idle column unpadded | §6.3 Member list output |
| C7 | Roster order; future idle (#7) | Document | Rows in ascending `member_id`; an activity timestamp later than now yields idle `0s` | §6.3 Member list output |
| C8 | Repeated `--database-url` (#8) | Document | A3: the option may appear once before and once after the subcommand; a repeat on the same side is a parse error (exit 2), and when both appear the one after the subcommand wins | §6.3 Database selection |
| C9 | URL path forms; SQLite URI mode (#9) | Document + Fix code | Document: the path is the text after the literal prefix `sqlite:///`, so `sqlite:///etc/x` is the relative path `etc/x` and an absolute path needs four slashes; any other prefix fails with `database URL must use the sqlite scheme (sqlite:///<path>); got '<url>'`. Fix code: open the path as a plain filename, never as a SQLite `file:` URI, and reject the path `:memory:` with the row C10 error `database URL has no file path`, exit 1 | §6.3 Database selection |
| C10 | Setup parent directories, printed path, empty path (#10) | Document | Setup creates missing parent directories and prints the path as given in the URL; `sqlite:///` with an empty path fails with `database URL has no file path` | §6.3 `cafleet setup` |
| C11 | Non-SQLite file error; `database error: ` prefix (#11) | Fix code + Document | Fix code: a file SQLite reports as not a database gets the schema-version guard's incompatible-database refusal. Document: `database error: <cause>` is the authored prefix of every other SQLite failure, `<cause>` implementation-defined | §6.3 Schema-version guard; Error Messages |
| C12 | "Blank" undefined; two definitions (#12) | Fix code | § Blank text and validation order | §6.3 Text bodies |
| C13 | Validation order unspecified (#13) | Document | § Blank text and validation order; A4 | §6.3 Validation order (new) |
| C14 | Malformed-prompt details (#14) | Document | The three `<detail>` texts verbatim: `unexpected '{' in placeholder name`, `single '}' encountered outside a placeholder`, `expected '}' before end of string`. The placeholder name is the exact text between braces, so `{}` and `{ fleet_id }` are unknown placeholders | §6.3 Spawn-prompt substitution |
| C15 | `fleet delete` partial-error composition; internal name (#15) | Document + Fix code | Document: `IDS` joined by `, `; `CAUSE` loses one trailing `.`. Fix code: the pane-close cause becomes `pane <pane-id> could not be closed: <cause>. Verify with 'cafleet doctor'.` | §6.3 `fleet delete`, `member delete` |
| C16 | Rust Debug formatting in errors (#16) | Fix code | String values keep the §6.4 Quoted text rendering, which already equals the current output. The Director pane-context error renders each context as `session=<quoted> window_id=<quoted> pane_id=<quoted>` from the context's three fields `session`, `window_id` and `pane_id`, or as `none` when the pane has no context; the herdr submit-bytes error renders the bytes as lowercase two-digit hex with no separator, the `submit_hex` form | §6.3 `fleet create`; §6.5 Backend operations |
| C17 | Parse rules defined by the parser library (#17) | Document + Loosen | § CLI parse rules | §6.3 Global Options, Positional subject ids |
| C18 | Timestamp parse leniency (#18) | Loosen | Stored-timestamp parsing accepts at least the canonical form and every RFC 3339 `date-time`; acceptance of anything else is implementation-defined | §5.1 Timestamps |
| C19 | 200-codepoint boundary (#19) | Document | A body of at most 200 scalars renders unchanged; a longer body renders its first 200 scalars followed by `…` | §6.4 Text-mode projection |
| C20 | `--file` error suffix; stdin read failure (#20) | Document | `--file <path>` read errors append ` (<io error>)`; a stdin read failure is `--file -: <io error>` | §6.3 Error Messages |
| C21 | `--lines` range; capture prefix (#21) | Document | `--lines` accepts any 64-bit signed integer at parse time; §6.5 capture windowing's rejection is wrapped as `capture failed: <cause>` | §6.3 `member capture` |
| C22 | `HOME` errors (#22) | Document | A1: `HOME is not set; cannot derive the default database path` and `HOME is not valid UTF-8; cannot derive the default database path` | §6.3 Database selection; Error Messages |

Row C9 rejects `:memory:` instead of documenting it. An in-memory database is private to one process, so no second `cafleet` process (a member's command, a runner) can reach the fleet it holds, and the pane-input and layout locks are keyed by a canonical database file that does not exist. Rejecting it removes the only SQLite special filename left once URI mode is off, so the path rule becomes "a filesystem path". The in-file tests that open `sqlite:///:memory:` move to a `#[cfg(test)]` in-memory constructor in `src/db.rs` `test_support`.

### Decision table — Area 2 (§6.5–§6.8)

| ID | Finding (audit 2 #) | Bucket | Resolution | Owning section |
|---|---|---|---|---|
| M1 | Child signal mask inside a deferral span (#1) | Document | The Linux experiment (`.experiments/sigmask.md`) showed in-span multiplexer clients inherit the blocked `SIGINT`, `SIGTERM` and `SIGHUP`, so the SPEC sentence stays, gains the per-thread scope and states the guarantee as the blocked set the client process starts with (a shell the client runs may change its own mask). The Step 3 child-mask test pins it on both spawn paths; a failure on `macos-15` alone makes macOS the A6 case | §6.5 Termination during pane input |
| M2 | Per-thread vs per-process deferral (#2) | Loosen | SPEC states the effect: within a span, no `SIGINT`, `SIGTERM` or `SIGHUP` delivered to the process takes its default action until the outermost span ends. The masking mechanism is implementation-defined | §6.5 Termination during pane input |
| M3 | Lock kind, release/reacquire, file mode (#3) | Document | The pane-input lock is a whole-file advisory lock with BSD `flock(2)` semantics: held per open file description and conflicting with other `flock` holders of the same file; whether it interacts with `fcntl` record locks is platform-defined. The holder keeps one descriptor across release and reacquire, and after acquiring it verifies that the lock path still names the same device and inode, reopening otherwise. The file is created with mode `0600` minus the umask | §6.5 Pane input lock |
| M4 | Working-directory source (#4) | Document | The invoking working directory is the physical path `getcwd(3)` reports, not `$PWD`; this governs `--cwd`, relative database paths, `-c` and Hermes `--in` | §6.5 Pane spawn working directory |
| M5 | Undeadlined calls; subprocess mechanics (#5) | Document + Loosen | Document the list of multiplexer calls that run without a deadline (`member capture` among them). A7: replace the drain-loop mechanics (64 KiB reads, 20 ms polls) with the observable guarantees: concurrent drain from spawn, deadline-exact timeout, kill and reap of the direct child, both pipes closed, interrupted operations retried against the same deadline, and the decoding rule | §6.5 Subprocess output and deadlines |
| M6 | herdr `pane_not_found` location (#6) | Document | It is read from the JSON `error.code` member of herdr's stderr | §6.5 Backend operations |
| M7 | herdr JSON numeric and ordering rules (#7) | Document | Integer fields (`area.height`, `shell_pid`) require a JSON number written without fraction or exponent; `x`, `y`, `width` and `ratio` accept any JSON number. Columns group by exact binary64 equality of `x`; the sort is stable, a missing `rect.y` sorts first, and a repeated JSON key takes its last occurrence | §6.5 herdr column equalization, herdr re-parenting |
| M8 | Ratio rounding; failed resize (#8) | Document | A ratio is `round_half_away_from_zero(v × 10⁴) / 10⁴` in binary64; the first failed resize stops the remaining columns | §6.5 herdr column equalization |
| M9 | tmux reply parsing (#9) | Document | Each reply line is trimmed of Unicode White_Space, then split; numeric fields accept a leading `+`; a malformed line is one without exactly five fields, a field outside 0–65535, or a flag other than `0`/`1` | §6.5 Backend operations |
| M10 | Character and hex units (#10) | Document | "256 characters" counts Unicode scalars; `send-keys -H` takes one lowercase two-digit token per byte; `0xXX` renders lowercase | §6.5 Explicit key dispatch |
| M11 | Shell quoting (#11) | Document | A word made only of ASCII alphanumerics and `_@%+=:,./-` is written as is; any other word is wrapped in `'` with each `'` written as `'"'"'`; the empty word is `''` | §6.5 Member pane creation |
| M12 | Process-error rendering (#12) | Document | `<argv>` is the argv joined by single spaces without quoting; `<trimmed stderr>` is trimmed of Unicode White_Space; add `command deadline expired before launch` | §6.5 Error taxonomy |
| M13 | Hermes process registry (#13) | Document | A missing `processes.json` counts as verified-stopped; each entry requires `pid` (u32), `host_start_time` (u64) and `owner_task_id` (string), ignores unknown fields, and one malformed entry fails with the verbatim `is not a Hermes process registry` error | §6.7 Hermes work settlement |
| M14 | Hermes settlement constants; bottom-border width (#14) | Document | State every constant the audit lists (last 8 non-blank stripped lines, one key per character for `/stop` and `/exit`, 200 ms before Escape and each Enter, report-match count rise, last `Session:` match, 400-line capture, italic SGR 3/23/0 with colon subparameters ignored). The bottom-border width is the bottom row's own width, as the code computes it | §6.7 Hermes work settlement |
| M15 | Slot selection vs preflight order (#15) | Document | State that slot selection runs between the backend availability check and the Hermes preflight, and which error wins when both fail | §6.3 `member create`; §6.5 Slot selection |
| M16 | Pattern dialect corners; catalog decode (#16) | Document + Loosen | Document: POSIX `[:name:]` classes are ASCII-only and `(?i)` uses Unicode simple case folding; the rule-object schema (§ Screen detection additions) states every key's null/absent handling. Loosen: run-time decoding of a catalog that breaks a provenance rule is implementation-defined, because the binary loads only its embedded catalog and a shipped catalog satisfies every provenance rule | §6.9 Pattern syntax; Bundled profiles |

Row M3 leaves the `fcntl` interaction platform-defined instead of verifying and recording it per platform. Every process that takes a lock on a pane-input or layout lock file is a `cafleet` process, and each takes it with `flock`. SQLite's `fcntl` locks apply to the database file, which is a different file. No conforming configuration therefore puts an `fcntl` lock on a lock file, and per-platform facts would bind reimplementations to something no `cafleet` process can observe.

### Decision table — Area 3 (§6.9–§9)

| ID | Finding (audit 3 #) | Bucket | Resolution | Owning section |
|---|---|---|---|---|
| R1 | Profile rule object unspecified (#1) | Document | New subsection Profile rule object (§ Screen detection additions) | §6.9 |
| R2 | Matching algorithm unwritten (#2) | Document | New subsection Detection algorithm | §6.9 |
| R3 | Cell → line text (#3) | Document | New subsection Line text | §6.9 |
| R4 | VT model semantics (#4) | Document | New subsection Terminal emulation plus §9.2 vectors | §6.9; §9.2 |
| R5 | Snapshot granularity; `screen_seq` (#5) | Document | The model is observed after each ASCII control byte (0x00–0x1F, 0x7F) on its own and after each maximal run of other bytes within one PTY read; `screen_seq` grows by the number of bytes fed and by 1 per resize | §6.9 Screen model facts |
| R6 | OSC title tracked in SPEC, not in code (#6) | Document | Remove the OSC window title from Screen model facts and from the detector's inputs in Classification and episode identity; no rule consumes it | §6.9 |
| R7 | Fixture → screen step; fixtures without expected observation (#7) | Document + Fix code | New subsection Fixture replay; new screen field `expected_observation` | §6.9 Bundled profiles |
| R8 | §9 is one paragraph (#8) | Document | § Testing strategy rewrite | §9 |
| R9 | Fingerprint composition (#9) | Document | State the exact composition, including each matcher's full capture, and the encoding the comparison uses | §6.9 Classification and episode identity |
| R10 | Runtime namespace identity keys (#10) | Document | Name the keys `canonical_path`, `instance_id`, `device`, `inode` and the derivation of the `.cafleet-runs/<ns>` directory name | §6.9 Runner boundary |
| R11 | Attempt-budget margin (#11) | Document | Runs without a verified composer also require the 250 ms margin | §6.9 Measured submission plans |
| R12 | Unspecified constants (#12) | Document + Loosen | Document the runner's re-observe cadence during the pre-submit window and the native capture sample count of 2. Loosen the 2 ms idle tick and the descriptor file (format and 1 MiB bound) | §6.9 |
| R13 | Lifecycle frame strictness; reply shapes (#13) | Fix code + Document | Fix code: lifecycle frames reject a repeated key, as input frames do. Document: exact key sets and order of the `prepared`, `started` and `health` replies | §6.9 Input control frames |
| R14 | Process start identity (#14) | Document | Linux: `<boot_id>:<starttime>` from `/proc/sys/kernel/random/boot_id` and field 22 of `/proc/<pid>/stat`; macOS: the `kinfo_proc` `p_starttime` rendering the code uses | §6.9 Persistence |

### Blank text and validation order

One definition replaces both current ones: a text is **blank** when every scalar in it has the Unicode `White_Space` property (the empty text included). `src/cli/mod.rs` `resolve_body` drops its extra U+001C–U+001F range, so a `message send` or `member create` body made only of those scalars becomes accepted; `pane_lock::valid_input` already trims by `White_Space`. Each command keeps its current exit code for a blank body (positional `2`, `--file` / `--file -` `1`, prompt/exec `2`).

A new §6.3 subsection, Validation order, states the order the code applies, as one numbered list per command family. The known skeleton:

1. Argument parsing; a parse error exits 2.
2. `member press` key parsing; an invalid key exits 2.
3. Database open and the schema-version guard; exit 1.
4. The Director-only guard for the guarded commands; exit 1.
5. Command-body validation: the exec command line, the send/create body source (blank, `--file` errors) and prompt text validity.
6. Command-specific checks (for `member create`: fleet checks, placeholder substitution, model/effort, backend availability, slot selection, preflight), in code order.

The implementer confirms each position by reading `src/cli/mod.rs` and each command's `run` path, and adds one table-driven test per family asserting that each adjacent pair fails in the stated order.

### CLI parse rules

§6.3 Global Options and Positional subject ids state the parse rules that change behavior:

- `-h` and `--help` print help; `-V` and `--version` print `cafleet <version>`; `cafleet help [COMMAND…]` prints help like `--help`. All exit 0 without opening the database.
- `--opt=value` and `--opt value` are equivalent.
- Integer IDs accept an optional leading `+` and leading zeros (`+5`, `007`); a value outside the 64-bit signed range is an invalid-value parse error.
- A flag given twice (for example `--json --json`) is a parse error; `--database-url` follows row C8.
- `cafleet` with no arguments is a parse error.
- `__member-run` is a hidden subcommand absent from help; its use outside the runner launch is unsupported (descriptor format per row R12).
- A parse error writes nothing to stdout and exits 2; its first stderr line begins with `error: ` and names the offending argument; the rest of its text is implementation-defined (A11).

### Error-literal sweep

The sweep's source set is every authored string in non-test source (outside `#[cfg(test)]` items) that can reach stderr, a stored diagnostic (`member_runner_diagnostics` rows) or a stored event or receipt text field (for example a `runner_degraded` cause's `error`). The script matches these patterns, together with every `format!` and string literal passed to them:

| Pattern | Examples |
|---|---|
| Text-carrying `CafleetError` variants | `Usage`, `App`, `DatabaseBusy`, `DatabaseIo`, `DatabaseInterrupted`, `WithDiagnostics { diagnostics }`, `InvalidStoredValue { … }` |
| Fixed-text variants, through their `Display` text | `DeadlineExceeded` |
| `MultiplexerError` construction | `MultiplexerError::new` |
| Module error helpers | `runtime::error` (`src/runtime/mod.rs`), `invalid` and `require` in `src/coding_agent/catalog.rs` and `src/broker/runtime/mod.rs` |
| `Display` implementations of error types | Every `impl fmt::Display` for a type that reaches stderr |
| Direct stderr writes | `eprintln!`, `writeln!` to stderr (the native-version-probe warning among them) |
| Stored diagnostic and event text | Every value written to a diagnostic or event text field |

A completeness check runs after the match. It lists every non-test site that builds an error value of `CafleetError` or `MultiplexerError` without a matched pattern, other than one that propagates an existing error (`?`, `map_err` around a matched constructor, `From` conversions). It also lists every non-test stderr write the patterns missed. For each listed site, the sweep either adds a pattern and reruns or records the site in an exclusions table with its reason. The sweep is complete when the check lists no unhandled site. Its output, `.sweep/error-literals.md` under BASE, has the exclusions table and one row per literal:

| Column | Content |
|---|---|
| Location | `file:line` |
| Literal | The format string as written |
| Reachability | The user action or environment that reaches it, or `invariant` |
| Class | `listed`, `reworded` or `internal` |
| Owner | The owning `SPEC.md` section (§6.3 Error Messages, an action owner, or §6.7 Contract error strings) |

`listed` literals go verbatim into their owner, with placeholders in SPEC's `<name>` form. `reworded` literals are code fixes (rows C15 and C16 among them) and then go in as listed. `internal` literals report a violated invariant and fall under the § Conformance policy implementation-defined class. Each row is reviewed by hand; a literal whose reachability is unclear is `listed`. `cli.md` gains each listed literal an agent must recognize to choose its next command.

### Signal-mask experiment

The experiment uses `cafleet member press`, whose span covers both of its multiplexer calls (`src/runtime/press.rs` `press_member_with_mux`) and which needs no runner or native child. Run it on Linux before Step 3's SPEC edits:

1. Create a database with `cafleet setup --database-url sqlite:///<BASE>/.experiments/sigmask.db`. Insert by SQL, per §8: one live fleet, its active root Director with a concrete tmux placement, and one active ordinary member with a concrete tmux placement (`backend` `tmux`, `mux_session` `s`, `mux_window_id` `@1`, `mux_pane_id` `%2`). The placement needs no `member_runs` row.
2. Put a stand-in `tmux` executable first on `PATH`. It appends its argv to a log under BASE and answers by subcommand:

   | Invocation | Stand-in behavior |
   |---|---|
   | <code>display-message -p -t %2 '#{pane_id}&#124;#{window_id}&#124;#{session_name}'</code> (key-target check, before `send-keys`) | Prints <code>%2&#124;@1&#124;s</code> and exits 0 |
   | `send-keys -t %2 <keys…>` | Writes its blocked set (`signal.pthread_sigmask(signal.SIG_BLOCK, [])` in Python) to a file under BASE, sleeps 2 seconds, exits 0 |
   | Any other invocation | Logs it and exits 1 |

3. Make a dry run, `cafleet member press 2 --from-member-id <director-id> -- Enter`, with no signal. The log must show exactly the two invocations above (with whatever server selector the placement yields) and the command must exit 0. If the log shows another invocation, add its reply to the table, record it in `.experiments/sigmask.md` and repeat the dry run.
4. Run the command again and send `SIGINT` to its process group during the `send-keys` sleep.
5. Record the blocked set the stand-in saw, whether the stand-in survived the `SIGINT`, and whether `cafleet` took the default action only after the span ended.

macOS evidence comes from CI, which runs `mise //:test` on `macos-15`. Step 3 adds an in-file test in `src/runtime/system.rs`, the file that spawns every multiplexer client and where a Fix code change lands. Inside a `DeferTermination` span, the test drives both production spawn paths, `SystemRunner`'s `CommandRunner::run` (through `spawn_piped`) and `run_until_exit`. Each re-executes the test binary as a probe child that prints the blocked set it inherited. A `/bin/sh -c` child cannot serve as the probe, because dash and bash reset an inherited mask at startup (`.experiments/sigmask.md` § Follow-up). The test asserts that the decoded set contains `SIGINT`, `SIGTERM` and `SIGHUP`, as one table-driven test over the two spawn paths. When the Linux result selects Document, that test pins the behavior on both platforms. When the result selects Fix code, the test checks the fix, and a failure on `macos-15` alone makes that platform the A6 case below. A local macOS host, when one is available, runs steps 1–5 as well.

Decision rule: when the stand-in's set contains `SIGINT`, `SIGTERM` and `SIGHUP`, the SPEC sentence stays and row M1 becomes Document. When it does not, row M1 becomes Fix code: multiplexer clients started inside a span start with the span's blocked set (for example by re-blocking the set in the child before `exec`), with an in-file test that inspects a child's mask. This is the recommendation because the deferral exists to keep a host interrupt from killing a client mid-input. A6 (inherited and blocked signals are not part of the contract) is adopted only for a platform where that fix proves infeasible, and the reason is recorded in row M1.

### Screen detection additions

§6.9 gains four subsections after Screen model facts and one field on fixture screens. Each subsection is written from the cited code and the screen-model crates pinned in `Cargo.lock`, but in implementation-neutral terms.

**Terminal emulation** (row R4). A sequence table with one row per control function the model acts on: C0 controls, ESC sequences, CSI sequences (cursor movement, erase, insert/delete, scroll, scroll region, SGR), DEC private modes (including 1, 6, 25, 47, 1049 and 2004; modes 7 and 1047 are listed as leaving state unchanged, because autowrap is always on and 1047 is not acted on) and OSC, DCS, APC, PM and SOS handling. Each row gives the effect on the snapshot-visible state and cites at least one §9.2 vector ID. The snapshot-visible state is each cell's contents, wide-leading and continuation status, foreground and background colours and the cell attributes the detector reads (faint only; SGR 1 clears faint), together with each row's wrap flag, the cursor with its pending-wrap column, the primary/alternate screen and the bracketed-paste mode. A cell attribute the detector never reads is outside the contract, and the table omits it. The section also states:

- Every sequence not in the table leaves the snapshot-visible state unchanged.
- Invalid UTF-8 handling, byte by byte.
- Scalar width: the Unicode version of the East_Asian_Width and emoji data the model uses, ambiguous width 1, and the handling of emoji presentation, ZWJ sequences, VS16 and combining marks (including a combining mark with no preceding cell).
- Wrap-flag set and clear rules, and resize behavior (truncation or extension of rows, cursor clamping, the two-column floor).

The implementer derives the table from the crates' source in the Cargo registry, so the list is exhaustive by construction; `.inventory/vt-model.md` under BASE holds the working inventory.

**Line text** (row R3). How a snapshot becomes rows and logical lines:

- A row's text is its cells' contents left to right. A wide scalar's continuation cell contributes nothing, a cell with no contents (never written, or erased, whatever its attributes) contributes nothing at the end of a row, a written space is kept, and an interior cell with no contents contributes one U+0020.
- A logical line joins a row with the following rows while each row's wrap flag is set.
- A clipped row (Diff sidebar clipping) ends its logical line.
- Region text is the region's logical lines joined with `\n`.

**Profile rule object** (rows R1, M16). A code block with the complete schema: rule keys `id`, `priority`, `observation`, `region` (`screen`, `start_pattern`, `end_pattern`, `max_rows`, `cursor`, `panel_separator`), `matchers` (`id`, `scope`, `pattern`, `focus_style`), `extract` (`operation`, `choices` with `id`, `label`, `scope`), `frame`, `suggestion` (`style`), `verified_noninteraction`, `continues_rule_id`, `continues_choice_id`, `text_framed`, `text_prefixes`, `text_wrap` and `fixture_ids`. Each key has its type, required/optional status, the meaning of an explicit `null` versus absence, its allowed values (for example `screen`: `primary`, `alternate`, `either`; `cursor`: `any`, `at_end`, `after_prompt`, `inside`; `scope`: `line`, `region`; `focus_style`: `marker`, `background`) and the observation kinds that accept it. Existing scattered statements (Operation extraction, Interaction frame guard, Diff sidebar clipping) stay at their owners and the schema links them.

**Detection algorithm** (rows R2, R9). A numbered procedure from snapshot to primary observation, written from `src/coding_agent/detector.rs`. It covers:

- The start-pattern match, once per logical line.
- End-pattern candidates within `max_rows`.
- The `cursor` conditions, including `at_end` measured against the boundary after the end match.
- The exactly-one-candidate rule and its cursor-free variant.
- Matcher matching: a line-scoped matcher must match exactly one line of the region, and a region-scoped matcher must match exactly once in the region text.
- The frame guard, the priority order, and tie and conflict handling (yielding `unknown_interaction`).
- Choice and focus extraction.
- The fingerprint's exact composition (R9).

**Fixture replay and expected observation** (row R7). Bundled profiles and native screen fixtures gains two rules.

Fixture replay is normative. It follows the test replay in `src/coding_agent/detector.rs` (`fixture_bytes`, `attribute_rewrite`). A fixture's screen is the state a fresh model of `rows × cols` reaches after two feeds:

1. Text feed:
   1. `ESC [ ? 1049 h` when `alternate_screen` is true.
   2. `ESC [ 2 J`.
   3. For each row `r` whose `lines` entry is non-empty, `ESC [ <r+1> ; 1 H` followed by the entry's UTF-8 bytes. Masked rows are empty, so they stay unwritten.
2. Attribute feed, computed from the model state after the text feed:
   1. Give every cell `(row, col)` covered by a `cell_backgrounds` or `cell_styles` run an SGR parameter list. It holds, in order: the background parameters of its `cell_backgrounds` run (`48;5;<index>` for `indexed`, `48;2;<r>;<g>;<b>` for `rgb`), then `2` when its `cell_styles` run has `faint: true`, then the foreground parameters of that run when `foreground` is non-null (`38;5;<index>` or `38;2;<r>;<g>;<b>`). Parameters are joined with `;` and numbers are decimal.
   2. Visit the covered cells in ascending `(row, col)` order. A cell continues the open segment when it is on the segment's row, directly after the previous visited cell and has the same joined parameter list. Otherwise it starts a segment: emit `ESC [ 0 m` when a segment is open, then `ESC [ <row+1> ; <col+1> H` and `ESC [ <parameters> m`.
   3. Emit each visited cell's text: nothing for a wide continuation cell, the cell's contents (base scalar plus zero-width scalars) when it has contents, and one U+0020 otherwise.
   4. After the last cell, emit `ESC [ 0 m` when any segment was opened.
3. When both cursor coordinates are recorded, feed `ESC [ <cursor_row+1> ; <cursor_col+1> H`.

The resulting snapshot carries the recorded cursor and screen mode. When `alternate_screen`, `cursor_row` and `cursor_col` are `null` (a cursor-free fixture), the snapshot's cursor and screen mode are set to absent after feeding, since no byte sequence produces that state, and detection applies Cursor-free classification.

Every screen requires a new field, `expected_observation`. It records the detector's own output for each detection entry point the screen's profile supports, so every value is defined as what the Detection algorithm yields, with no mapping:

```json
{
  "expected_observation": {
    "observe": {
      "kind": "permission_request",
      "rule_id": "claude-bash-permission",
      "match_evidence": "complete",
      "operation": " ls -la",
      "choices": ["Yes", "No"],
      "verified_noninteraction": false,
      "continues_rule_id": null
    },
    "director": null,
    "composer_text": null,
    "answers": { "claude-bash-allow-digit": { "focused_choice_id": "allow-once" } }
  }
}
```

| Key | Entry point | Value |
|---|---|---|
| `observe` | Runner observation over the profile's detection and composer rules plus every continuation rule (`Detector::observe`) | An observation object (table below) |
| `director` | Director observation over the profile's `director_delivery.rule_ids` plus every interaction and continuation rule (`Detector::observe_director`) | An observation object when the profile declares `director_delivery`; `null` otherwise |
| `composer_text` | Composer-text extraction (`Detector::composer_text`) | `null` when the profile declares no `composer.text` or the extraction yields none; otherwise `{"mode_prefix", "segments"}` with `mode_prefix` a string or `null`, and each segment `{"kind":"text","items":[…]}` (items are strings, with `{"break":true}` for a word-wrap row break), `{"kind":"scalars","count":N}` or `{"kind":"lines","count":N}` |
| `answers` | Answer matching against the `observe` result (`Detector::match_answers` over every `capabilities.answers` entry whose `request_rule_id` is `observe.rule_id`) | An object keyed by each matched entry's `answer_rule_id`, each value `{"focused_choice_id": <id or null>}` from Focused-choice resolution; `{}` when no entry matches, including every observation whose `match_evidence` is not `complete` |

| Observation key | Value |
|---|---|
| `kind` | The observation kind: `permission_request`, `permission_continuation`, `question`, `unknown_interaction`, `composer_empty`, `composer_text` or `unknown`. A conflict yields `unknown_interaction` when any conflicting rule is an interaction rule and `unknown` otherwise |
| `rule_id` | `unmatched-screen` when no selected rule matches; otherwise the first matched rule in ascending rule-id order among the matches at the highest priority, on a conflict too |
| `match_evidence` | `absent` (no match), `conflicting` (the top-priority matches differ in kind, operation or choices) or `complete` |
| `operation` | The extracted operation, rows joined with `\n`; `null` when unmatched, on a conflict, or when the rule extracts none |
| `choices` | The captured labels of the rule's declared `extract.choices`, in declaration order; `[]` for a matched rule without choices; `null` when unmatched or on a conflict |
| `verified_noninteraction` | `true` only for a non-conflicting match of a rule declaring `verified_noninteraction: true` |
| `continues_rule_id` | The continued request rule's id for a `permission_continuation`; `null` otherwise |

The bundled-catalog guard replays each screen (Fixture replay), runs each entry point with the screen's own backend profile, and fails with `fixture <ID> expected observation differs: <key path>` on the first differing key in the order of the tables above. `verify_bundled_screen` adds `expected_observation` to the required screen field set. SPEC names the entry points by their role (runner observation, Director observation, composer-text extraction, answer matching); the code identifiers above are pointers for the implementer only.

Row R7's buckets split as follows. Fixture replay and the observation values are Document: the values are populated once from the current Rust detector, which wins at population. The new field and the guard are Fix code. After population, the fixture value is the oracle: a change that alters any detector output must update the affected `expected_observation` values in the same change, and the guard makes that update visible in review. A capture added later through the capture-to-fixture conversion records the values the capturer verified on the live screen.

### Testing strategy (§9)

§9 becomes three subsections:

| Subsection | Content |
|---|---|
| §9.1 Conformance levels | The four existing levels (command, function, runtime, operating system), each naming the SPEC sections whose contract it checks and the observable it compares |
| §9.2 Terminal conformance vectors | Fenced blocks with info string `vt-vector`, each one JSON object (schema below), covering every Terminal emulation row, each width class, each wrap rule, invalid UTF-8, resize and observation granularity |
| §9.3 Detection conformance | The bundled screens' `expected_observation` values are the detection vectors: replaying a screen and detecting with its backend's profile must yield them |

```json
{
  "id": "wrap-pending-then-cr",
  "rows": 2,
  "cols": 4,
  "steps": [{ "feed_hex": "61626364" }, { "feed_hex": "0d" }, { "resize": [2, 6] }],
  "observations": [4, 5],
  "expect": {
    "lines": ["abcd", ""],
    "wrapped": [],
    "cursor": [0, 0],
    "alternate_screen": false,
    "bracketed_paste": false,
    "screen_seq": 6,
    "cells": [{ "row": 0, "col": 3, "contents": "d", "fg": "default", "bg": "default", "faint": false }]
  }
}
```

| Key | Required | Value |
|---|---|---|
| `id` | yes | A string unique among the vectors |
| `rows`, `cols` | yes | Positive integers: the initial terminal size |
| `steps` | yes | A non-empty array, each element exactly one of `{"feed_hex": H}` (the bytes of lowercase hex string `H`, delivered as one PTY read) or `{"resize": [ROWS, COLS]}` (rows first) |
| `observations` | no | When present, the exact sequence of `screen_seq` values the observer sees across all steps, under the row R5 granularity. A resize step produces no observation |
| `expect` | yes | The final state; each key below is asserted only when present |

| `expect` key | Value |
|---|---|
| `lines` | One string per row, in the Line text row rendering |
| `wrapped` | The exact set of row indexes whose wrap flag is set, ascending |
| `cursor` | `[row, col]`, zero-based; `col` equals the column count while a wrap is pending |
| `alternate_screen`, `bracketed_paste` | Booleans |
| `screen_seq` | An integer |
| `cells` | Cell assertions, each with required `row` and `col`, plus any of: `contents` (the base scalar and its zero-width scalars, `""` for an empty cell), `wide` and `continuation` (booleans), `fg` and `bg` (`"default"`, `{"kind":"indexed","index":N}` or `{"kind":"rgb","r":R,"g":G,"b":B}`, the fixture colour form plus `"default"`), and one boolean per detector-read attribute named in Terminal emulation (`faint` at least) |

The values in the example illustrate the schema; each committed vector's expected values come from the Rust model. A new in-file test in `src/runtime/transport.rs` reads `SPEC.md` with `include_str!`, extracts every `vt-vector` block, drives `TerminalState` and asserts each expectation, naming the vector ID in every assertion message. The vectors therefore have one source of truth, and a SPEC edit that breaks one fails `mise //:test`. The test feeds each `feed_hex` step through the observed feed path, so `observations` checks row R5 with the same vectors.

### Blind slice

The blind slice checks that the § Conformance policy split and the Area 1 rows suffice on their own.

| Aspect | Decision |
|---|---|
| Scope | `setup` (with `HOME` and config directories under a temporary directory), `message send`, `message poll`, `message show`, `fleet list`, `fleet show`, `member list`, `member show`, `event show`, `input show`, `--version`, `--database-url` forms, the schema-version guard (missing, empty, incompatible, non-SQLite files) and parse errors, in text and `--json` modes |
| Implementer | A fresh agent whose working directory holds only a copy of `SPEC.md` and `assets/`; it gets no repository, source, git history or Rust binary |
| Release version | The brief states the Rust binary's version (the `<version>` that `cafleet --version` prints), and the implementation embeds it as a constant, as a release build does (§7.6). It governs `--version`, the default `cafleet_<VERSION>.db` name and the version the schema-version guard compares |
| Language | Python 3, standard library only (`sqlite3`, `json`, `hashlib`, `unicodedata`) |
| Harness author | A different agent with full repository access. It writes the scenarios and the harness under `.blind-slice/` in BASE |
| Seed data | The harness creates each scenario's database with the Rust `cafleet setup` and inserts fleets, members, placements, messages and runtime rows with SQL that follows §8; each binary runs on its own copy |
| Comparison | Exit code, stdout bytes, stderr bytes and a dump of every §8 table after each scenario |
| Normalization | Timestamps written during the run become `<TS#n>` by order of first appearance; temporary paths become placeholders; the implementation-defined regions of § Conformance policy are masked |
| Coverage | At least one scenario per §6.3 Error Messages row and per Output shapes row that falls in the scope, in each output mode the command supports |
| Pass | 0 differences after the difference loop below |

The harness author classifies each difference:

| Class | Condition | Fix | The blind agent receives |
|---|---|---|---|
| `spec-gap` | SPEC is silent, ambiguous or wrong about the observed Rust behavior | A `SPEC.md` edit | The SPEC diff only |
| `implementer-error` | A SPEC sentence states the Rust behavior unambiguously | The blind agent's own change | The scenario's command, inputs and normalized expected output, plus the SPEC sentence the classification cites; no source |

The blind agent may contest an `implementer-error` by quoting the SPEC text that supports its reading. The Director decides contested cases, and a case where both readings are reasonable is a `spec-gap`. The harness author records every difference, its class and its resolution in `.blind-slice/differences.md`.

### Re-audit

After Step 4 is complete, three fresh agents repeat the audit, one per area with the same section split. Each gets `SPEC.md`, the source tree and `.audit/spec-audit-2026-11-27.md` as the methodology template. The severities keep the audit's meaning:

| Severity | Meaning |
|---|---|
| `blocker` | A reimplementer cannot produce the behavior from SPEC |
| `needs-oracle` | A reimplementer must consult the binary or source to choose between plausible behaviors |
| `minor` | A small ambiguity with low impact |

Every `blocker` and `needs-oracle` finding gets a fix and a re-check by the same agent. Each `minor` finding is fixed or recorded in § Accepted minor findings with its reason.

### Accepted minor findings

Filled during Implementation Step 5.

| Finding | Reason accepted |
|---|---|
| — | — |

### Documentation and rules

| Target | Change |
|---|---|
| `SPEC.md` | Every row above; § Conformance policy; new §6.3 Validation order; new §6.9 subsections; §9 rewrite |
| `docs/docs/contributing.md` | Bundled catalog provenance checks: the `expected_observation` row and error. Native screen fixture capture: recording `expected_observation`, and correcting the sentence "fixture replay applies each run's background and style before writing its row text" to the actual order (the row text first, then the attribute runs, per Fixture replay). The terminal conformance test reading `SPEC.md` §9.2 |
| `README.md` | No change (no pitch, install or section change) |
| `assets/skills/cafleet/reference/cli.md` | Facts it carries from rows C1–C8, C12, C14, C15, C22 and listed sweep literals an agent acts on |
| `.claude/rules/code-quality.md` | Test placement: a conformance test may read the normative vectors of `SPEC.md` §9.2 as its inputs and expected values |

### Delivery

All work lands on one branch, `docs/spec-reimplementation-readiness`, and in one pull request. The steps are stages of that branch:

| Step | Scope | Rows |
|---|---|---|
| 2 | §1, §5, §6.1–§6.4, §7, the §6.5 Slot selection pointer for row M15, Area 1 sweep literals | C1–C15, C16 Director pane-context part, C17–C22, M15 |
| 3 | §6.5–§6.8, Area 2 sweep literals | C16 herdr submit-bytes part, M1–M14 |
| 4 | §6.9, §8, §9, profiles, conformance test, Area 3 sweep literals | M16, R1–R14 |
| 5 | The SPEC fixes the blind slice and the re-audit produce, at their owning sections | Findings of Step 5 |

Row M15 lands in Step 2 because its order belongs to the §6.3 Validation order subsection that Step 2 writes; §6.5 Slot selection keeps a pointer to it. Row M16 lands in Step 4 because its owner is §6.9.

This design doc is committed with the Step 1 results (row M1's decision included), before the first SPEC edit. Each step ends with `mise //:test` and `mise //:lint` passing and commits the design doc's checkbox, Progress, Last Updated and Changelog updates for that step. Step 5 also fills § Accepted minor findings.

Within each step the commits follow the project's documentation-first order: `SPEC.md`, `docs/`, skills, rules, then code. The branch is pushed and its pull request opened once Step 3 is complete, so that the `macos-15` CI run supplies the child-mask evidence; later steps push to the same pull request. Fix-code rows change shipped behavior (blank text, URI opening, error texts, possibly M1), so one patch release follows the merge through the release workflow; SPEC-only work needs none.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Baseline evidence

- [x] Write the sweep script and produce `.sweep/error-literals.md` with every source-set literal located, its reachability and its owner, rerunning until the completeness check lists no unhandled site <!-- completed: 2026-11-27T08:48 -->
- [x] Classify every sweep row `listed`, `reworded` or `internal` <!-- completed: 2026-11-27T08:48 -->
- [x] Run the § Signal-mask experiment on Linux (and on a local macOS host when available), record results in `.experiments/sigmask.md` and set row M1's bucket and resolution in this document <!-- completed: 2026-11-27T08:33 -->
- [x] Build `.inventory/vt-model.md`: every control function the screen model acts on, its effect, and the Unicode data version, from the crates pinned in `Cargo.lock` <!-- completed: 2026-11-27T08:43 -->

### Step 2: CLI, output and persistence

- [x] SPEC: § Conformance policy in the opening paragraph and §1 <!-- completed: 2026-11-27T09:00 -->
- [x] SPEC: rows C1–C7 and C19 (fleet, member delete, queued line, poll and roster layouts) <!-- completed: 2026-11-27T09:00 -->
- [x] SPEC: rows C8–C11 and C22 (database selection, setup path, schema guard) <!-- completed: 2026-11-27T09:00 -->
- [x] SPEC: § Blank text and validation order (rows C12, C13, M15), with the order confirmed from code and the §6.5 Slot selection pointer <!-- completed: 2026-11-27T09:00 -->
- [x] SPEC: rows C14–C18, C20, C21 and § CLI parse rules <!-- completed: 2026-11-27T09:00 -->
- [x] SPEC: every Area 1 `listed` and `reworded` sweep literal at its owner <!-- completed: 2026-11-27T09:00 -->
- [x] `cli.md`: carried facts for Area 1 <!-- completed: 2026-11-27T09:00 -->
- [x] Code: unify blank text in `resolve_body`, with a table-driven test over White_Space and U+001C–U+001F bodies <!-- completed: 2026-11-27T09:13 -->
- [x] Code: validation-order tests per command family <!-- completed: 2026-11-27T09:13 -->
- [x] Code: open database paths as plain filenames and reject `:memory:` (row C9), with a table-driven test that `sqlite:///file:x?mode=memory` creates a file of that name and `sqlite:///:memory:` fails with `database URL has no file path`; move the in-file tests that open `sqlite:///:memory:` to an in-memory constructor in `src/db.rs` `test_support` <!-- completed: 2026-11-27T09:13 -->
- [x] Code: route a not-a-database file to the incompatible-database refusal (row C11), with a test <!-- completed: 2026-11-27T09:13 -->
- [x] Code: reword the pane-close cause (row C15), the Director pane-context error (row C16's Director pane-context part) and every other Area 1 `reworded` literal, with tests <!-- completed: 2026-11-27T09:13 -->
- [x] Run `mise //:test` and `mise //:lint` and commit the step <!-- completed: 2026-11-27T09:16 -->

### Step 3: Multiplexer, pane input and backends

- [ ] SPEC: row M1 per the experiment, and rows M2–M5 (signals, lock, working directory, A7 subprocess guarantees, undeadlined calls) <!-- completed: -->
- [ ] SPEC: rows M6–M12 (herdr and tmux parsing, units, shell quoting, process errors) <!-- completed: -->
- [ ] SPEC: rows M13 and M14 (Hermes registry and settlement) <!-- completed: -->
- [ ] SPEC: every Area 2 `listed` and `reworded` sweep literal at its owner <!-- completed: -->
- [ ] `cli.md`: carried facts for Area 2 <!-- completed: -->
- [ ] Code: herdr submit-bytes hex rendering (row C16's herdr part) and every other Area 2 `reworded` literal, with tests <!-- completed: -->
- [ ] Code: the child-mask test in `src/runtime/system.rs` over both spawn paths inside a `DeferTermination` span (§ Signal-mask experiment), plus the row M1 fix in those spawn paths when the experiment selects Fix code <!-- completed: -->
- [ ] Confirm the child-mask test's result on the `macos-15` CI run and, when it fails there alone, record the A6 case for macOS in row M1 and its SPEC text <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint`, commit the step, then push the branch and open the pull request <!-- completed: -->

### Step 4: Runtime, detection and testing strategy

- [ ] SPEC: Terminal emulation subsection from `.inventory/vt-model.md`, and rows R5 and R6 in Screen model facts <!-- completed: -->
- [ ] SPEC: Line text, Profile rule object and Detection algorithm subsections (rows R1–R3, R9, M16) <!-- completed: -->
- [ ] SPEC: Fixture replay and the `expected_observation` field (row R7) <!-- completed: -->
- [ ] SPEC: rows R10–R14 <!-- completed: -->
- [ ] SPEC: every Area 3 `listed` and `reworded` sweep literal at its owner <!-- completed: -->
- [ ] SPEC: §9 rewrite with §9.1, §9.3 and the §9.2 vectors, generated from `TerminalState` with expected values reviewed against the Terminal emulation table <!-- completed: -->
- [ ] `docs/docs/contributing.md`: provenance row, capture conversion, the corrected fixture replay order sentence and conformance test <!-- completed: -->
- [ ] `.claude/rules/code-quality.md`: the SPEC-vector test placement line <!-- completed: -->
- [ ] Code: `expected_observation` in the screen field set and the guard comparison in `src/coding_agent/catalog.rs`, with tests for a matching and each differing field <!-- completed: -->
- [ ] Code: populate `expected_observation` for every screen of `assets/profiles/*.json` from the current detector, with a temporary generator removed before commit <!-- completed: -->
- [ ] Code: reject repeated keys in lifecycle frames (row R13), with a test <!-- completed: -->
- [ ] Code: every Area 3 `reworded` literal, with tests <!-- completed: -->
- [ ] Code: the `vt-vector` conformance test in `src/runtime/transport.rs` <!-- completed: -->
- [ ] Run `mise //:test` and `mise //:lint` and commit the step <!-- completed: -->

### Step 5: Verification

- [ ] Harness author: write the § Blind slice scenarios, seeds and harness under `.blind-slice/` and confirm every scenario runs against the Rust binary <!-- completed: -->
- [ ] Blind agent: implement the slice from `SPEC.md` and `assets/` only <!-- completed: -->
- [ ] Run the harness; resolve each difference per its class (`spec-gap` or `implementer-error`), record it in `.blind-slice/differences.md` and re-run until 0 differences <!-- completed: -->
- [ ] Run the three § Re-audit agents and record their reports under `.audit/` <!-- completed: -->
- [ ] Fix every `blocker` and `needs-oracle` finding and re-check it; fill § Accepted minor findings <!-- completed: -->
- [ ] Commit the Step 5 SPEC fixes and § Accepted minor findings with `mise //:test` and `mise //:lint` passing, and push them to the pull request <!-- completed: -->

### Step 6: Closeout

- [ ] Cut one patch release through the release workflow, covering the fix-code changes <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-27 | Initial draft |
| 2026-11-27 | Reviewer round 1: multi-bucket rows and code pointers, `:memory:` rejection, `flock`-only lock contract, sweep source set and completeness check, seeded signal-mask experiment with CI macOS evidence, byte-exact fixture replay, per-entry-point `expected_observation`, `vt-vector` key tables with observations, blind-slice version and difference classes, PR 4 and row-to-PR assignment |
| 2026-11-27 | Reviewer round 2: child-mask test moved to `src/runtime/system.rs` over both production spawn paths |
| 2026-11-27 | User decision: one branch and one pull request in place of four; the steps become stages of that branch |
| 2026-11-27 | Step 2 code-over-text findings: `--database-url` may appear once per command level (root, group, leaf) and the deepest level wins, more general than A3's before/after wording (row C8); `fleet delete` strips every trailing `.` from `CAUSE`, not one (row C15); `cafleet` and a command group without its subcommand print help to stderr with exit 2 instead of an `error: ` line (§ CLI parse rules); `HOME` derivation precedes `member press` key parsing, and `member create` checks `--model`, `--effort` and `--permission-mode` before its body source and validates placeholders after the layout lock and preflight (§ Blank text and validation order). Spec choice: the `database URL has no file path` refusal of rows C9 and C10 applies to every command, so `sqlite:///` with an empty path no longer reports `no cafleet database` outside `setup` |
