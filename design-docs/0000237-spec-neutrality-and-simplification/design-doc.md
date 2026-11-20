# SPEC Neutrality, Test Simplification and Feature Reduction

**Status**: Approved
**Progress**: 0/83 tasks complete
**Last Updated**: 2026-11-21

## Overview

Make `SPEC.md` an implementation-neutral contract from which any implementation could be built and conformance-tested, simplify the Rust test suite (table-driven cases, one injected runtime clock, a runtime scenario harness, a test ledger), and reduce CAFleet toward a necessary-and-sufficient feature set by implementing the 17 feature reductions the user approved. CAFleet stays a Rust project, and every backend (claude, codex, opencode, hermes, pi) and both multiplexers (tmux, herdr) stay supported.

## Success Criteria

- [ ] `SPEC.md` describes the `cafleet` binary's behavior without naming its implementation language, crates, toolchain, internal identifiers or repository tests, outside verbatim contract strings and the named carve-outs of § Neutrality rule; every § Verification check returns no unresolved hit.
- [ ] Every one of the 68 audit findings and every sweep item (S1–S8 and any S9+ recorded during Step A2) is applied as its disposition row states.
- [ ] Pattern syntax, screen-model facts, CLI parse-error/help essentials, ledger DDL, JSON encoding and canonical JSON, quoted-text escaping and every source-only constant have one owning `SPEC.md` definition each, and every other mention points to it.
- [ ] Every bundled catalog pattern uses explicit character classes (no `\s \S \d \D \w \W \b \B`, no `\u{…}`), and `mise //:test` passes with the rewritten catalog, so every native fixture still classifies as before.
- [ ] Test and build mechanics live in `docs/docs/contributing.md`; `SPEC.md` §9 states only the language-neutral conformance scope.
- [ ] `.claude/rules/documentation-maintenance.md` carries the SPEC neutrality rule, and `.claude/rules/code-quality.md` § Test placement carries the three § Rule changes bullets.
- [ ] Every approved feature reduction (R1–R5, R7–R13, R16, R18–R21) is implemented by its Step B1–B16 with its SPEC, docs, skills, DDL, catalog, code and tests removed or rewritten in the same step, and the behavior of every rejected candidate (R6, R14, R15, R17) is unchanged.
- [ ] Every Rust `#[test]` present at Step B0 has a ledger row (kept, merged into a named table test or scenario, or dropped with a reason); a test traced to a `fix:` commit keeps its case unless its fate is `dropped:feature removed (R<n>)` for an approved candidate.
- [ ] Production code in `src/runtime/` and `src/broker/runtime/` reads time and sleeps only through the unified `time::Clock`; tests of that code use `TestClock`, and among them `std::thread::sleep` appears only in class-6 tests (real processes, PTYs, sockets or locks).
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass after each workstream.

---

## Background

A Go-rewrite feasibility study (local reports under `.go-feasibility/`, not committed) preceded the user's decision to keep CAFleet in Rust, and produced three reusable results: a 68-finding audit of Rust-dependent wording in `SPEC.md` (commit `005a42d8`), a classification of the 2,081 Rust tests with a simplification proposal, and slice-by-slice evidence on where the implementation's complexity sits. The runtime alone is about 21.6k production lines (`src/runtime/input.rs` has 5,766 production lines before `mod tests` and 237 tests), and recent `fix:` commits cluster around delivery queue, held-composer, stall, sweep and flush mechanisms, each fix adding a new mechanism (designs 0000201, 0000228, 0000233, 0000236).

This document records the three workstreams. Because the feasibility reports are local, every finding and candidate this document relies on is restated here.

---

## Specification

### Resolved decisions

| # | Decision |
|---|---|
| D1 | Order: Step B0 (test ledger) → Workstream A (SPEC neutrality) → Workstream B (approved feature reductions) → Workstream C (test simplification). B precedes C so tests of removed features are deleted rather than simplified, and the ledger exists before B deletes any test. |
| D2 | Workstream A changes no runtime behavior. Its only non-documentation edits are the semantics-preserving rewrite of bundled catalog patterns (§ Bundled pattern rewrite) and two confirming test changes: the quoted-text table test in `src/output/text.rs` (§ Quoted text) and the explicit-class check in the `src/coding_agent/catalog.rs` guard tests (§ Pattern syntax); `mise //:test` proves fixture classification is unchanged. |
| D3 | `SPEC.md` keeps every authored string verbatim, including strings with Rust-flavored wording (`native launch unwound; performing owned compensation`, `capture_pane: lines must be positive, got <lines>`). Only the surrounding condition text is neutralized. |
| D4 | Text that an operating system or library contributes after an authored prefix (`<io error>`, `<error>`, `<cause>`, regex compiler messages) is non-normative in wording, quoting and numeric suffix. An errno name that `SPEC.md` shows (for example `ECONNREFUSED`) is normative; its description is not. |
| D5 | CLI parse errors: normative essentials only (§ Parse errors and help). Help: usage, subcommands and options with one-line descriptions, exit 0; layout non-normative except strings `SPEC.md` already pins. |
| D6 | Regex: bundled catalog patterns are rewritten to explicit classes, stated as a catalog provenance invariant. Operator `--event-redaction-file` patterns and the built-in patterns `SPEC.md` quotes keep their current semantics, which § Pattern syntax states explicitly; acceptance of a pattern outside the syntax is implementation-defined. |
| D7 | Screen model: `SPEC.md` states only the screen-model facts it already relies on, without naming the parser library (§ Screen model facts). |
| D8 | JSON encoding, canonical JSON and quoted text (`TEXT_REPR`, `<quoted>`) are pinned to today's bytes by explicit definitions. |
| D9 | The ledger table `refinery_schema_history` and `assets/migrations/V1__baseline.sql` keep their names; §8 gains the ledger DDL. |
| D10 | Test/build/release mechanics move to `docs/docs/contributing.md`. `SPEC.md` keeps provenance rules as an invariant of every shipped bundled catalog, the latency target, coverage semantics, and a rewritten language-neutral §9. |
| D11 | The user decided every feature-reduction candidate, TD-1 and TD-2 (§ User decisions). |
| D12 | Source-only constant names (`PREVIEW_SWEEP_INTERVAL`, `LAYOUT_LOCK_WAIT`, …) are internal identifiers: `SPEC.md` replaces each with a prose name and its value (§ Source-only constants). |
| D13 | The test ledger is a local working file at `design-docs/0000237-spec-neutrality-and-simplification/test-ledger.tsv` (kept out of version control by `.gitignore`, like every non-`design-doc.md` file in a design folder); its final counts are recorded in § Gate results. |
| D14 | No new automated SPEC-wording guard; verification uses the one-time checks in § Verification. |

### User decisions

| ID | Question | Answer |
|---|---|---|
| TD-1 | Add a black-box conformance tier outside source files? | (a) No. Conformance-style tests stay in-file in the `src/cli/*.rs` file that owns each command, driving the CLI entry with PATH stubs as today, with no golden files; `CLAUDE.md` § Tests and the Forbidden patterns of `.claude/rules/code-quality.md` § Test placement stay unchanged |
| TD-2 | Add the table-driven, injected-clock and scenario-harness bullets to `.claude/rules/code-quality.md` § Test placement? | (a) Adopt all three (§ Rule changes) |
| R1–R5, R7–R13, R16, R18–R21 | Approve each feature-reduction candidate? | Approved (17 candidates, § Approved candidate inventories) |
| R6, R14, R15, R17 | Approve each feature-reduction candidate? | Rejected: prompt/exec receipts with `input show` and `input reconcile`, the two-column fleet layout, caller-pane verification and `member exec` stay as they are (§ Rejected candidates) |

Every approved candidate's hard prerequisites ("Requires" in § Decision view) are approved.

---

### Workstream A: SPEC.md language neutrality

#### Neutrality rule

`.claude/rules/documentation-maintenance.md` gains one paragraph in § Three audiences, directly after the audience table:

> `SPEC.md` specifies the `cafleet` binary's observable behavior in implementation-neutral terms, so any implementation can be built and conformance-tested from it alone. It names the contract's external parts — the SQLite storage format and its table names, the embedded `assets/` paths, external programs (tmux, herdr, each backend's executable) and the host APIs the bundled extensions use — and does not name the binary's implementation language, libraries, internal identifiers, build tools or test suite. Implementation, build, test and release facts live in `docs/docs/contributing.md`. A contract that a library defines incidentally is restated as the behavior itself.

#### Preamble

Replace the second sentence of the `SPEC.md` preamble with:

> Exact authored errors, JSON fields/order and command layouts govern. Text that an operating-system or library error contributes after an authored prefix (written `<io error>`, `<error>` or `<cause>`) is non-normative: an implementation preserves the error's meaning, not its wording, quoting or numeric suffix. An errno name shown in a diagnostic (such as `ECONNREFUSED`) is normative; the description after it is not.

#### New owning definitions

| Definition | Owning section | Pointers from |
|---|---|---|
| Pattern syntax | §6.9 Bundled profiles and native screen fixtures, new paragraph | §6.4 ANSI stripping, §6.7 Composer drafts, §6.9 Native version probe (Hermes line), §6.9 Persistence (redaction file), every quoted catalog pattern's section |
| JSON encoding and canonical JSON | §6.4 Output & Formatting, new `#### JSON encoding` heading | §6.3 JSON output, §7.3, §6.9 fixture `sha256`, runtime namespace, delivery reporting digest, permission response storage |
| Quoted text (`TEXT_REPR`, `<quoted>`) | §6.4 Formatter helper semantics | §6.3 Output shapes (`member prompt`), §6.3 Error Messages, §6.5 Backend operations |
| Parse errors and help | §6.3 Help text and §6.3 Error Messages (owning row) | §6.3 JSON output, Positional subject ids, `message listen`, `member press` |
| Screen model facts | §6.9 Terminal transport and lifecycle, new paragraph | §6.9 Bundled profiles (continuation cells, pending-wrap cursor), Diff sidebar clipping |
| Release version | §7.6 Packaging & distribution | §6.3 Database selection, §8 |
| Ledger DDL | §8 exact DDL block | §8 diagnosis prose |

##### Pattern syntax (proposed text)

> Profile patterns, frame guards and region matchers of the bundled catalog, operator redaction patterns, and the built-in patterns this specification quotes (the Hermes version line, the ANSI CSI pattern) use this syntax: literals, and a backslash before any ASCII punctuation character matching that character literally (`\.`, `\?`, `\(`, `\[`, `\$`, …); `.` (any scalar except `\n`); bracket classes with ranges, negation, POSIX `[:name:]` classes and `\p{…}` general-category or script names; the escapes `\t \n \r \f \v` and `\xHH` / `\x{H…}` for a code point; `^ $` (line anchors under `(?m)`, text anchors otherwise), `\A`, `\z`; groups `(…)`, `(?:…)`, `(?P<name>…)`; alternation; greedy and lazy quantifiers `* + ? {n} {n,} {n,m}` with counts up to 1000; flags `i m s U` in `(?flags)` and `(?flags:…)`; and the Perl classes `\s \S \d \D \w \W` and boundaries `\b \B`. Perl classes have Unicode meaning: `\s` is White_Space (U+0009–U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000), `\d` is general category Nd, `\w` is Alphabetic, M, Nd, Pc and Join_Control, `\b` is a boundary between `\w` and non-`\w`, and the capitals negate. Matching is leftmost-first over Unicode scalar values, and match offsets are UTF-8 byte offsets. Whether a pattern outside this syntax is accepted, and how it matches, is implementation-defined.

The catalog half is a provenance invariant, added to the provenance rules (F40): "Every bundled catalog pattern spells its classes explicitly: it contains no Perl class or boundary (`\s \S \d \D \w \W \b \B`) and no escape outside the syntax."

Like every other provenance rule, this invariant has a check: Step A3 extends the bundled-catalog guard in the `src/coding_agent/catalog.rs` tests (beside `verify_bundled_fixtures`) with a data-driven pass over every pattern-bearing field of every profile (rule matchers, frame guards, region matchers, `prompt_pattern` and every other regex the catalog decoder compiles). It fails naming the profile, the rule ID and the offending construct, for example ``claude 2.1.302 rule claude-bash-command: `\s` ``. The check exercises the catalog decoder's pattern fields over the embedded catalog, which keeps it within the test rule's use of `assets/` data as input to production code. `docs/docs/contributing.md` § Bundled catalog provenance checks lists it with the other guard checks.

##### Bundled pattern rewrite

Edit `assets/profiles/bundled.json` (data, not code) as follows; Step A2 first updates every catalog pattern quoted in `SPEC.md` (about 60 code spans) to the identical rewritten text:

| Construct | Uses | Rewrite | Why it preserves behavior |
|---|---|---|---|
| `\u{a0}` | 29 | `\x{a0}` | Same code point |
| `\s` | 248 | `[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]` | Exactly the White_Space set the current engine uses, so NBSP- and hair-space-bearing screens match as today |
| `\S` | 73 | `[^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]` | Exact complement |
| `\d` | 69 | `[0-9]` | Native screens render ASCII digits; the test suite proves every fixture still classifies |
| `\b` | 4 (`settings\b`, `Dangerous Command\b`) | `(?:$|[^0-9A-Za-z_])` | Both follow an ASCII letter; the test suite proves every fixture still classifies |

A `\s`/`\S`/`\d` already inside a bracket class is rewritten to its members inside that bracket (`[\s,]` → `[\t-\r \x{85}…\x{3000},]`). Perform the rewrite with a one-off script kept outside the repository and review the diff. Fixture `sha256` values are unaffected because they digest screen objects, not patterns. The built-in Hermes version line keeps `\S` under the Unicode meaning above.

##### JSON encoding (proposed text, §6.4)

> Every JSON document the `cafleet` binary writes — `--json` output, JSONL listener records, stored JSON columns and inbox payloads — is compact: no whitespace between tokens, object members in the order this specification lists, arrays in order. Strings escape `"` as `\"`, `\` as `\\`, U+0008 as `\b`, U+000C as `\f`, U+000A as `\n`, U+000D as `\r`, U+0009 as `\t`, and every other code point below U+0020 as `\u00` followed by two lowercase hex digits; every other scalar, including `/`, U+007F, non-ASCII text, U+2028 and U+2029, is written raw as UTF-8. Numbers in these documents are integers, written in decimal with a leading `-` when negative.
>
> Canonical JSON is this encoding with every object's members sorted by the UTF-8 bytes of their keys, recursively. A digest named "SHA-256 of the canonical JSON of X" is the lowercase hex SHA-256 of those bytes.
>
> JSON that a bundled extension emits on its host's behalf is any valid JSON; the binary parses it by value, independent of whitespace, escaping and member order.

The binary writes no non-integer number today (no `f64` reaches an output, storage or canonical-JSON encoder; herdr layout replies are parsed, not written). Step A3 confirms this; if it finds one, the integer sentence is replaced with the exact float rendering.

##### Quoted text (proposed text, §6.4 Formatter helper semantics)

> `TEXT_REPR` and `<quoted>` denote the value wrapped in `"` with these escapes: `"` → `\"`, `\` → `\\`, U+0009 → `\t`, U+000A → `\n`, U+000D → `\r`, U+0000 → `\0`; any other scalar whose general category is Cc, Cf, Cs, Co, Cn, Zl, Zp or Zs (except U+0020), or which has the Grapheme_Extend property, → `\u{H}` with lowercase hex and no leading zeros. Every other scalar, including `'`, is written as itself. Categories and properties follow the Unicode version the implementation ships; the rendering of a scalar whose classification differs between Unicode versions is non-normative.

Step A3 confirms the definition against the shipped behavior with an in-file table test in `src/output/text.rs` over one representative scalar per listed category and property, using scalars assigned since Unicode 6.0.

##### Parse errors and help (proposed text, §6.3)

Help text:

> `--help` on any command prints its usage line, its subcommands and its options, each with a one-line description, to stdout and exits 0 without opening the database. Its layout is non-normative except for the strings this specification quotes, such as the `--permission-mode` description.

Error Messages owning row (replacing "The parser's native error of that kind"):

> A parse error — an unknown subcommand or option, a missing required argument, an invalid value, or conflicting arguments — writes to stderr, leaves stdout empty and exits 2 before any command body runs. Its first line begins with `error: ` (lowercase, distinguishing it from the `Error: ` prefix of application errors). It names the offending argument: a missing required argument by its placeholder (for example `<MEMBER_ID>`), an invalid value by the argument and the rejected value, a fixed-value option additionally by its accepted values, and a conflict by both arguments. The rest of its wording is non-normative.

Every other site that says "the parser's native error" becomes "a parse error (§6.3 Error Messages)".

##### Screen model facts (proposed text, §6.9 Terminal transport and lifecycle)

> The runner reconstructs the child's screen as a grid of rows × columns of cells and offers detection a snapshot of it after each step. A cell holds one base scalar plus any following zero-width scalars. A wide scalar (width 2; East Asian Ambiguous scalars are width 1) occupies a leading cell and a continuation cell, and writing it resets the continuation cell to the default background and style. Each row records whether its text continued onto the next row by automatic wrap. After a write into the last column the cursor reports column `cols` (pending wrap). A cell's colours are default, one of 256 indexed colours (SGR 30–37/40–47 and 90–97/100–107 denote indexed colours 0–15, equal to their `38;5;n`/`48;5;n` forms) or RGB, and its style includes faint. The model tracks the primary and alternate screens, the OSC window title and the bracketed-paste mode (DEC private mode 2004), resizes with the PTY, and never answers a terminal query itself.

The F43 and F48 sentences point at this paragraph.

##### Source-only constants

Each internal constant name becomes a prose name with its value (D12):

| `SPEC.md` site | Today | Replacement |
|---|---|---|
| §6.9 Measured submission plans L3286, L3291; Stop cause L4201 | `stage_settle()`, `STAGE_WRITE_MARGIN` | New bounds-table row "Stage write margin — 250 ms"; predicate "covers `500 ms × settles + 2 s + 250 ms`"; L3291 and L4201 say "the stage write margin (250 ms)" |
| §6.9 Bounded input draining L3888 | `bytes_per_tick` | "Let B be 16384 bytes, the per-drain transfer quota" |
| §6.9 Runner pending-preview sweep L4048 | `PREVIEW_SWEEP_INTERVAL` (1 s) | "the preview sweep interval (1 s)" |
| §6.9 Queue housekeeping L4022 | `QUEUE_HOUSEKEEPING_INTERVAL` (1 s) | "the housekeeping interval (1 s)" |
| §6.9 Immediate attempt failure bookkeeping L4018 | `DEFERRED_FINALIZATION` | "the deferred finalization interval (1 s)" |
| §6.9 Input delivery stalled event L3707 | `COMPOSER_BUSY_BOUND` | "the stall bound (3 minutes)" |
| §6.9 Forced flush L4103 | `FLUSH_LITERAL_LIMIT` | "the flush literal limit (65,536 bytes)" |
| §6.3 `message listen` L886 and following | `HOLD_CEILING` | "the hold ceiling (300 s)" |
| §6.3 L973, 1031, 1357–1359; §6.5 L1707, 1756 | `LAYOUT_LOCK_WAIT` | "the layout lock wait (60 s)"; the L1705–1707 constants table names rows "Minimum pane height", "Maximum panes per column", "Layout lock wait" |
| §6.5 L1705, 1706, 1709 | `MIN_PANE_HEIGHT`, `MAX_PANES_PER_COLUMN` | Prose row names; formula `capacity(H) = min(8, floor(H / 15))` |
| §6.7 Hermes work settlement L2260 | `POLL_INTERVAL` | "the settlement poll interval (200 ms)" |
| §6.9 Runner boundary L3316 | namespace encoding unspecified | First 26 characters of the unpadded RFC 4648 base32 encoding (alphabet `a`–`z`, `2`–`7`, most-significant bit first) of the SHA-256 digest of the canonical JSON of the database identity object |
| §6.5 herdr column equalization L1862, 1872 | rounding/rendering unspecified | Rounded to 4 decimal places half away from zero, written as the shortest decimal that round-trips, without exponent or trailing zeros (`0.1667`, `0.5`) |
| §6.5 Subprocess output L1500 | "lossy UTF-8" | UTF-8 with each maximal ill-formed subsequence replaced by one U+FFFD |
| §6.5 Backend selection L1471–1478 | "truthy"/"set" | A non-empty `HERDR_ENV` / `TMUX` signals its multiplexer; an empty value counts as unset |
| §6.3 Asset installation L308 | `std::env::current_exe()` | The absolute path of the running executable as the OS reports it (Linux: target of `/proc/self/exe`; macOS: the kernel-reported image path), with every symbolic link resolved |
| §7.6 | "package version", "compile-time version" | "The release version, printed by `cafleet --version` and used as the release tag, names the default database file and asset stamps" |
| §7.6 | undefined `<target>` | Platform name in target-triple form, for example `x86_64-unknown-linux-musl` |

Step A1 also lists every SCREAMING_CASE code span in `SPEC.md` (§ Verification check 3) and adds any further internal constant to this table before applying it.

#### Finding dispositions

Line numbers refer to `SPEC.md` at `005a42d8`. "Accept" applies the replacement summarized in the row; "Keep" leaves the contract with the stated tightening.

| # | Site | Cat. | Disposition and replacement |
|---|---|---|---|
| F1 | Preamble L3 | d | Accept: § Preamble text |
| F2 | §5.3 L61 | a | Accept: "A member's `status` is `active` or `deregistered`; a message's `status_state` is `input_required` or `completed`." |
| F3 | §6.1 L83 | a | Accept: "Every operation that writes the database validates the current format first." |
| F4 | §6.2 L89 | a | Accept: delete the `create_fleet` wrapper sentence |
| F5 | §6.3 Help text L152–162 | b | Accept: § Parse errors and help |
| F6 | §6.3 L211, 215, 840, 1099, 1329, 1369 | b | Accept: owning row in Error Messages; pointers elsewhere |
| F7 | §6.3 L367 | d | Keep; add "and exits 0" |
| F8 | §6.3 L284, 292 | c | Accept: "The binary carries the data files of the `assets/` tree listed below. The native screen fixtures are catalog provenance records: the bundled catalog cites them, and setup installs none of them." Consumer cell: "The profile catalog (§6.9 Bundled profiles and native screen fixtures)" |
| F9 | §6.3 L308 | a | Accept: § Source-only constants, executable path |
| F10 | §6.3 L402 | a | Accept: "through the Rename window operation (§6.5 Backend operations)" |
| F11 | §6.3 L404 | a | Accept: "Keep the registration transaction open until the commit result is known: …" |
| F12 | §6.3 L440, 445 | a | Accept: `NotFound` → `ENOENT` (file not found) |
| F13 | §6.3 L515 | b | Accept: example ends `: <io error>` (the OS description of `EACCES`) |
| F14 | §6.3 L834 | a | Accept: "Optional integer from 1 through 1800 inclusive"; same edit in `assets/skills/cafleet/reference/cli.md` (`1..=1800`) |
| F15 | §6.3 L906 | d | Accept: state only the bound — "Lock waits and statement execution stay within that deadline: a running statement is interrupted once the deadline passes, and the deadline is checked before and after each statement, each decoded row and the commit." The progress-handler mechanism moves to `docs/docs/contributing.md` |
| F16 | §6.7 heading L2039 and pointers L954, 971, 1601, 2016, 2074, 2107 | a | Accept: heading "Per-backend spawn argv (exact, token-by-token)"; update every pointer and the `docs/docs/concepts/coding-agents.md` anchor `#per-backend-build_spawn_argv-exact-token-by-token` |
| F17 | §6.3 L1022 | a | Accept condition cell "An internal fault aborts the launch step before it returns a result"; string verbatim (D3) |
| F18 | §6.3 L1039 | a | Accept: "placement object" |
| F19 | §6.3 L142, §7.6 L4378, §8 L4386, 4647 | d | Keep; "package version"/"compile-time version" → "release version (§7.6)" |
| F20 | §6.3 L1085, §6.9 L3862 | d | Keep (64-bit integer domains match SQLite `INTEGER`) |
| F21 | §6.4 L1444 | a | Accept: heading "ANSI stripping and CR-defrag" |
| F22 | §6.4 L1446 | a | Accept: "An empty input yields the empty string." |
| F23 | §6.4 L1450 | a | Accept: "an absent value or one without a `T` renders `-`" |
| F24 | §6.5 L1471–1478 | a | Accept: § Source-only constants, environment presence; table headers "non-empty" / "unset or empty" |
| F25 | §6.5 L1500 | b | Accept: § Source-only constants, subprocess output |
| F26 | §6.5 L1516 | a | Accept: "Hold the lock through exactly one open file descriptor and close it on every exit path; no duplicate of that descriptor outlives the operation." |
| F27 | §6.5 L1589 | a | Accept: "runs under that absolute deadline through spawn, collection and cleanup" |
| F28 | §6.5 L1591 | a | Accept: "poll for its exit without blocking (`waitpid` with `WNOHANG`), sleeping at most 2 milliseconds between polls … Cleanup on an error path and cleanup of an abandoned operation use the same deadline and perform no blocking wait, each child is cleaned up at most once, and cleanup of an abandoned operation writes its diagnostics to stderr." |
| F29 | §6.5 L1770 | d | Keep verbatim (D3) |
| F30 | §6.5 L1862, 1872 | b | Accept: § Source-only constants, herdr amount |
| F31 | §6.7 Interface L2009–2021 | a | Accept: "Each backend has a registry token (its `--coding-agent` value, stored `placement.coding_agent` value and executable name) and zero or more shared-server signatures. `member create` checks, in order, model, effort, prompt body, availability (config directory, executable on `PATH`, the backend's own prerequisite), then builds argv. A model, effort or prompt failure is a usage error (exit 2) with the backend's message and precedes every side effect; an availability failure exits 1. An omitted `--model` or `--effort` is always valid." |
| F32 | §6.7 L2015, 2021, 2048–2101 | a | Accept: "an omitted `--model` emits no `--model` tokens"; argv comments "when `--model` is given" / "when `--effort` is given" |
| F33 | §6.7 L2107 | a | Accept: replace with "No pi argv carries a prompt that begins with `@`." |
| F34 | §6.7 L2222 | c | Accept: "a test-runner command executes workspace-writable test code" |
| F35 | §6.7 L2373 | c | Accept: "(test code, build scripts, task-runner definitions)" |
| F36 | §6.7 L2389, 2403, 2406; §6.9 L2833, 2909, 2913 | b | Accept: prose "`❯` followed by U+00A0"; pattern literals use `\x{a0}` (§ Bundled pattern rewrite) |
| F37 | §6.7 L2416 | b | Accept per D6: `prompt_pattern` cell "Pattern (§6.9 Pattern syntax) or `null`" |
| F38 | §6.9 L2632 | c | Accept: "Unverified components remain unknown or unsupported." |
| F39 | §6.9 L2688 | c | Accept: "One SemVer 2.0.0 precedence comparison serves profile validation, native-version admission, fixture provenance validation and selection." |
| F40 | §6.9 L2696, 2700, 3148, 3150–3157, 3161, 3198, 3224, 3240, 3525 | c | Accept: "build guard" → "catalog provenance validity", stated as "A shipped bundled catalog satisfies every provenance rule below", plus the § Pattern syntax catalog invariant. The error-string table, synthetic-fixture/test-catalog sentences and "does not pass its test suite" move to `docs/docs/contributing.md` § Bundled catalog provenance checks |
| F41 | §6.9 L2702 | a | Accept: "`sha256` is the SHA-256 of the canonical JSON (§6.4 JSON encoding) of that entry's screen object"; "Run-time catalog loading ignores the `fixtures` registry contents and every `fixture_ids` value; only provenance validation reads them." |
| F42 | §6.9 L3142 | c | Accept: capture-to-fixture conversion and fixture replay procedure move to `docs/docs/contributing.md` § Native screen fixture capture; `SPEC.md` keeps the fixture file format |
| F43 | §6.9 L3144 | b | Accept: point at § Screen model facts |
| F44 | §6.9 L3259 | a | Accept: "outside error messages, diagnostic output and persistence. A write that transfers zero bytes because the descriptor would block (`EAGAIN`/`EWOULDBLOCK`) or the call was interrupted (`EINTR`) yields …" |
| F45 | §6.9 L3286, 3291, 4201 | a | Accept: § Source-only constants, stage write margin |
| F46 | §6.9 L3289 | a | Accept: "returns to the pre-write wait without settling; each further chunk needs a fresh write authorization after fresh ordinary draining" |
| F47 | §6.9 L3316 | b | Accept: § Source-only constants, namespace |
| F48 | §6.9 L3342, 3354 | b | Accept: "propagate resize to the PTY and the screen model"; "the screen model never answers a terminal query itself; every reply comes from the outer terminal"; point at § Screen model facts |
| F49 | §6.9 L3376 | d | Accept: state only the outcome — "Each attempt is bounded by its own deadline, and after each attempt the connection's default busy timeout and handlers are in effect again." The hook mechanism moves to `docs/docs/contributing.md` |
| F50 | §6.9 L3460, 3740, 3886, 3895, 3897, 3903, 4040 | a | Accept: *pending*/*quiescent* drains; "The drain proof records both counters: ordinary input validation compares the keyboard generation, and permission validation compares both."; "an observer that is healthy but holds input eligibility with unknown visibility"; "the lifetime of that runner's terminal transport"; blocker strings verbatim |
| F51 | §6.9 L3897, 3899, 4246, 4248 | a | Accept: `EAGAIN`/`EWOULDBLOCK`, `EINTR`, "end of file"; "releases its write authorization"; "Abandoning a write authorization before any I/O records that no I/O occurred" |
| F52 | §6.9 L3495, 4081 | a | Accept: lowercase `visible`, `obscured`, `unknown` |
| F53 | §6.9 L3629 | b | Accept: "expressions in the pattern syntax (§ Pattern syntax)" |
| F54 | §6.9 L3637 | d | Keep; "The runner writes events and inbox messages through its own database transactions and never depends on its child invoking `cafleet message send`." |
| F55 | §6.9 L3880 | a | Accept: "three samples taken in order: the process's internal monotonic clock, `CLOCK_MONOTONIC`, and the internal monotonic clock again, validating sample order and overflow-checked arithmetic" |
| F56 | §6.9 L3888 | a | Accept: § Source-only constants, quota |
| F57 | §6.9 L3944 | a | Accept: "A zero-byte proof is valid only while the writer exclusion it was taken under is still held and only on the runner thread that holds it; it binds the complete immutable input/audit and current ownership, and only the input coordinator can create one." |
| F58 | §6.9 L4038 | a | Accept: "(SQLite busy/locked contention or an expired storage deadline)" |
| F59 | §6.9 L4049, 4058 | a | Accept: "no client-owned attempt and no sweep attempt is awaiting its result"; "The runner tracks a sweep attempt separately from client-owned attempts" |
| F60 | §6.9 L4161–4164 | a | Accept: "SHA-256 over the canonical JSON of the object with keys `run_id`, `kind`, `message_id` and `literal`"; "in the `literal` field of its `submit_prompt` request"; example in sorted order `{"kind":"preview","literal":"CANONICAL_BASE_PREVIEW","message_id":99,"run_id":42}` |
| F61 | §6.9 L4258 | b | Accept: "canonical JSON (§6.4 JSON encoding)" |
| F62 | §6.9 L4302–4352 | c | Accept: keep coverage semantics (first paragraph's second sentence) and the latency target; the release minimum, automatic-mode acceptance, minimum-version and composer-submit tables, unit-coverage and live-record paragraphs and the case list move to `docs/docs/contributing.md` § Release acceptance |
| F63 | §7.5 L4374 | a | Accept: "Every acquired lock, descriptor and owned child process is released on every error path and at process exit." |
| F64 | §7.6 L4378 | c | Accept: § Source-only constants, `<target>`; "Building the binary requires no documentation-site output."; toolchain sentences move to contributing |
| F65 | §8 L4386, 4403 | d | Keep names (D9) |
| F66 | §8 L4386 | b | Accept: add `CREATE TABLE refinery_schema_history (version INT4 PRIMARY KEY, name VARCHAR(255), applied_on VARCHAR(255), checksum VARCHAR(255));` to the DDL block; "Initialization inserts the ledger row with `version` 1 and `name` `'baseline'`; `applied_on` and `checksum` are non-normative, and diagnosis compares `version`, `name` and the column shape." Step A1 confirms the DDL against a freshly initialized file's `sqlite_master` |
| F67 | §8 L4647 | d | Accept: "Initialization applies the baseline and inserts the `cafleet_database` row for the running version in one transaction, so a failed initialization leaves no identity, table, ledger or version row." |
| F68 | §9 L4655 | c | Accept: § §9 rewrite |

Sweep items found beyond the audit:

| # | Site | Replacement |
|---|---|---|
| S1 | §4 L40 "injected process runners", "explicit rollback guards" | "Multiplexer utilities run processes through one bounded runner …"; "with explicit rollback on every failure path" |
| S2 | §6.5 L1540 "blocks `SIGINT`… restore the outer signal mask" | "defers `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span: a signal received meanwhile takes its default action when the outermost span ends; spans nest." |
| S3 | §6.3 L753 doctor example `ECONNREFUSED: Connection refused` | Keep the example; the preamble makes the description non-normative (D4) |
| S4 | §6.3 L185, 1051, 1362; §6.5 L1785–1789, 1816 (`TEXT_REPR`, `<quoted>`) | Point at § Quoted text |
| S5 | §7.3 | "JSON follows §6.4 JSON encoding with the member orders §6.3 lists; message-body truncation follows §6.3 Message Body Truncation." |
| S6 | §7.6 "The compile-time version owns `--version`…" | "The release version (above) owns `--version`, asset stamps and doctor's installation checks." |
| S7 | Internal constant names (D12) | § Source-only constants |
| S8 | Any further hit of a § Verification check | Recorded here as S9, S10, … with its replacement during Step A2 |

#### §9 rewrite

Replace §9 with:

> ## 9. Testing strategy
>
> A conforming implementation is verified at four levels. Command level: every §6.3 command's exit code, stdout/stderr text and JSON bytes, the multiplexer argv it issues, and the §8 rows it leaves, driven through the CLI with stand-in multiplexer and native executables. Function level: argv construction, multiplexer command choreography, terminal-reply and key parsing, ANSI stripping, catalog validation and screen classification of every catalogued native screen. Runtime level: the §6.9 runner fed scripted screens, commands and storage faults under a controlled clock, asserting events, rows, receipts and bytes written to the child. Operating-system level: real PTYs, process groups, signals, pane-lock exclusion and release on process death, and shutdown deadlines in real time.

#### Contributor-docs moves

`docs/docs/contributing.md` gains `## Testing and release acceptance` with four subsections, each receiving the moved `SPEC.md` text with paths and test-suite wording allowed (contributor exemption in `documentation-maintenance.md`):

| Subsection | Receives |
|---|---|
| `### Bundled catalog provenance checks` | F40 error-string table and synthetic-fixture/test-catalog sentences; F39/F41 build-guard wording; the explicit-class pattern check (§ Pattern syntax) |
| `### Native screen fixture capture` | F42 capture-to-fixture conversion and replay procedure |
| `### Storage deadline mechanisms` | F15 progress-handler interval and F49 busy-wait/hook restoration |
| `### Release acceptance` | F62 checklists and case list |

The existing link at `docs/docs/contributing.md:86` (`SPEC.md#native-acceptance-and-coverage`) points to the new `#release-acceptance` section instead, and its sentence keeps one clause stating that the section owns minimum policies, evidence fields and fault cases. The project-structure row for `assets/` ("which the test suite reads") stays.

#### Verification

After Step A2, run each check over `SPEC.md`; every hit is either a verbatim contract string, a § Neutrality rule carve-out, or resolved and recorded as S9+:

| # | Check | Pattern (grep -nE) |
|---|---|---|
| 1 | CamelCase code spans | `` `[A-Z][a-z]+[A-Z][A-Za-z]*`` `` and `` `[A-Z][a-z]+\(`` `` (catches `MemberStatus`, `AwaitBeforeWrite`, `DrainedInput`, `DatabaseBusy`, `SubmitInput`, `Hold(`) |
| 2 | Call-style spans | `` `[a-z_]+\(\)`` `` and `` `[a-z_]+\(`` `` (catches `ensure_available()`, `try_wait`, `stage_settle()`, `validate_`) |
| 3 | SCREAMING_CASE spans other than environment variables, signals and CLI placeholders | `` `[A-Z][A-Z0-9]*_[A-Z0-9_]+`` `` reviewed against the env-var/placeholder list |
| 4 | Paths and module separators | `[a-z_]+::` |
| 5 | OS error rendering | `os error [0-9]` |
| 6 | Snake_case internal identifiers | `pending_input_client`, `pending_sweep`, `rename_window`, `create_fleet`, `current_exe`, `bytes_per_tick`, `build_spawn_argv`, `strip_ansi`, `canonical_json` |
| 7 | Words | `Rust`, `cargo`, `Cargo`, `crate`, `build\.rs`, `mise`, `clap`, `serde`, `rusqlite`, `refinery` (table name only), `vt100`, `None`, `truthy`, `falsy`, `sub-dict`, `Drop`, `panic`, `unwind` (string only), `Send`, `NotFound`, `WouldBlock`, `Interrupted`, `Instant`, `guard`, `test suite`, `build guard`, `\\u\{` |

---

### Workstream B: Feature reduction

Selection criterion: keep what a coherent experience needs — correct detection of every bundled profile's approval forms and questions, empty-composer and busy/idle states, and reliable delivery of messages and previews — and every backend and multiplexer. The user decided each candidate (§ User decisions). Each approved candidate removes its contract from `SPEC.md`, then from every docs page, skill file and `SPEC.md` anchor link that names its command, flag, output, event or heading, then from the DDL in `assets/migrations/V1__baseline.sql` (edited in place; each version starts from a fresh database), the catalog, the code and its tests, in one step (Steps B1–B16) with no deprecation residue. A rejected candidate's behavior stays unchanged; its inventory stays under § Rejected candidates as the decision record.

#### Decision view

"Requires" marks a hard prerequisite and "Benefits from" a soft link; every approved candidate's prerequisites are approved.

| ID | Candidate | User-visible consequence | Depends on | Rec. | Answer |
|---|---|---|---|---|---|
| R1 | Runner-pull preview delivery: `message send` only stores the message; the recipient's runner delivers it | `send` reports only the stored message; delivery shows later as `notified_at`; first delivery waits for the next preview sweep (1 s), or the run-wide backoff (1 s doubling to 30 s) after failed attempts; no delivery while the member has no live runner | — | Yes: removes the second delivery path and its receipts | Approved |
| R2 | Remove `member flush` and `composer_draft` | No one-shot force-typing of queued previews into a non-empty composer; the Director captures and presses Enter, or asks the user | Requires R1 | Yes: its root cause (an unreached queue turn) disappears with R1 | Approved |
| R3 | One cause-free stall signal | The Director gets one instruction (capture, press Enter or ask the user) instead of a diagnosed cause | Requires R1, R4 | Yes: the other causes are queue states R1/R4 remove | Approved |
| R4 | Deliver only into a verified empty composer; retry uncertain previews | A human draft blocks previews until cleared instead of being interrupted by Escape after 3 minutes; a preview whose submission is uncertain is typed again later, so a recipient can see it twice and deduplicates by message ID; after an uncertain prompt or exec, no preview or queued input reaches the member until the Director runs `input reconcile` | Requires R1 | Yes: one rule replaces four mechanisms | Approved |
| R5 | Stop staged writes only when an interaction appears | Human typing during the 0.5 s write window can mix with the preview; post-submit verification still detects an unsubmitted result | Requires R4 | Yes: spurious manual stops drove two designs | Approved |
| R6 | Drop durable receipts for prompt/exec, `input show` and `input reconcile` | After a lost reply the Director inspects the pane instead of reconciling a receipt; a blind retry of a non-idempotent exec can run it twice | Requires R1, R2, R4, R13 | No: keeps exec double-run protection; revisit after R1/R2/R4/R13 land | Rejected |
| R7 | Remove automatic-review annotations, their events and the effective-policy record | No early alert of an automatic-mode refusal; the member's routed refusal remains the record | — | Yes: not part of detection or delivery correctness | Approved |
| R8 | Metadata-only event evidence | Events and `permission inspect` carry no operation text; the Director uses `member capture` | — | Yes: no bundled skill uses redacted evidence | Approved |
| R9 | Drop `interaction_obscured` | No event when a dialog is covered; open/close/supersede remain | Benefits from R8 | Yes: small and not needed for answers | Approved |
| R10 | Commit events directly to SQLite; remove the outbox spool | Events observed in the last seconds before a hard runner crash during database unavailability can be lost (their run is already dead) | — | Yes: removes the largest recovery subsystem | Approved |
| R11 | Remove the `--owner-terminated` attestation | A runner holding an unlinked live lock is resolved by `member delete` (pane force-close) | Requires R10 | Yes: rare case with a simpler resolution | Approved |
| R12 | Remove `member recover` | A crashed member is deleted and re-created rather than reconciled in place; the agent is lost with its runner either way | Requires R11 | Yes: `member delete` already recovers | Approved |
| R13 | Slim permission responses (keep token-bound inspect/respond; drop audit tables and provenance flags) | No durable record of who answered (user or Director-auto); answers still bind to the inspected request | — | Yes: provenance is attributed, not authenticated | Approved |
| R14 | One window/tab per member instead of the two-column fleet layout | Members no longer appear beside the Director; the user switches windows/tabs to watch a member | — | User preference: a UX trade-off, not a correctness issue | Rejected |
| R15 | Caller-pane verification reduced to the shared-server check plus `--pane` | A stale environment from an unsigned daemon can register the wrong pane; with R14 this affects only where windows open | Benefits from R14 | Yes if R14; no otherwise | Rejected |
| R16 | `member press` uses the shared bounded subprocess runner | Weaker reap guarantee on a hung multiplexer; none in normal use | — | Yes: one subprocess contract | Approved |
| R17 | Remove `member exec` | Commands no longer run in the member's own shell context and output | — | No: changes the member's execution context | Rejected |
| R18 | Flat `doctor` without member-runtime probing | One row per check; no fleet-wide liveness triage in doctor (`member list/show` show run status); `doctor --json` becomes an array | Benefits from R12 | Yes: most of doctor's code serves summary folding | Approved |
| R19 | Codex forwarding exits on any non-zero `codex queue` | A transient full queue ends the listener, which the recipe restarts; the notice bound keeps the queue far below 100 | — | Yes: the retry paths were never observed needed | Approved |
| R20 | Hermes work settlement through the runner's profile delivery | None if the Hermes profile's empty/busy rules cover `/stop` reporting | — | Yes, after a live Hermes check on tmux and herdr | Approved |
| R21 | Drop the `asset_installs` table | None; doctor byte-compares installed skills as it does extensions | — | Yes: redundant with byte comparison | Approved |

#### Approved candidate inventories

Line numbers are `SPEC.md` at `005a42d8`; DDL references are table/column names in `assets/migrations/V1__baseline.sql`; cost is production lines before `mod tests` and `#[test]` counts (approximate). Because R6 is rejected, `member_input_operations` stays, holding `prompt` and `exec` receipts once R1 removes previews, R2 removes flushes and R13 removes permission responses.

**R1 Runner-pull preview delivery.** `message send` commits the message and returns; it sends no wake frame. The runner's existing sweep (every 1 s, SPEC L4048) selects the oldest unnotified message, types it only into a verified empty composer (R4), and stamps `notified_at` after confirmed submission; delivery state is `messages.notified_at` plus in-memory "typed, awaiting verification". Previews leave `member_input_operations` and the runner FIFO, which then holds only prompt and exec requests.
- Uncertain preview: a preview attempt that stops or is not confirmed is uncertain. The runner keeps its stage and cause in memory and publishes one `input_delivery_result` event to the run's Director with `input_id` null, `message_id` the message, `delivery` `{"status":"uncertain","method":METHOD,"reason":null}`, `stage` the written stage, `cause` the stop cause and `director_action` null, deduplicated by run plus `message_id`. On a verified-composer run the message stays unnotified and the sweep retries it (R4); on a run without composer verification the runner stamps `notified_at` first and then publishes the event, so the preview is not typed again. The live runner is the event's only publisher: recovery publishes no preview result, so an event lost to a runner crash between stamp and publication is covered by the run's `runner_lost` event, and the message itself stays in the recipient's inbox for `message poll`, since a preview only notifies. An uncertain preview never stops the FIFO; the FIFO-stop rule (R4) covers only prompt and exec.
- Sweep order: the sweep starts a preview only when the FIFO is empty and not stopped, and no ordinary attempt is active, so a queued prompt or exec always precedes a sweep-selected preview. The sweep preconditions row becomes "The run is live and its native child is running; no ordinary attempt is active and no sweep attempt awaits its result; the runner FIFO is empty and not stopped".
- SPEC: §6.3 `message send` delivery table and outcome (L776–812), output row L172; §6.5 Push notifications steps 1–8 and held delivery (L1625–1648); pane-lock writer row L1531; termination rows L1544, 1557; §6.9 sweep's "first attempt" clause, preconditions row and latest-attempt table (L4044, L4049, L3909–3922), the FIFO's preview membership (L3982), retried-preview results (L3665–3676), the receipt-derived preview event fields and durability sentence (L3685–3695), preview rows of Queue housekeeping (L4020–4040), the control-frame `preview` request kind and its message-ID rule (L3876).
- Output/JSON: `message send` keys `notification_sent`, `input_id`, `delivery`; reasons `notification_held`, `notification_failed`; `input show` key `message_id` and text field `message=MESSAGE_ID` (L1208).
- DDL: `member_input_operations` `kind='preview'`, `preview_attempt`, `message_id`, `UNIQUE(run_id, message_id, preview_attempt)`, `CHECK(kind != 'preview' OR message_id IS NOT NULL)`; only previews set `message_id` (L3876 requires it null for prompt and exec).
- Code: `src/delivery.rs` (575 prod, 28 tests; only `message send` calls `deliver_pending`), `send_delivery` in `src/cli/message.rs` (~170), preview paths in `src/runtime/service.rs`, `src/runtime/input.rs`, `src/runtime/input_client.rs`, `src/broker/runtime/inputs.rs`; ~100 deferred/sweep tests. Fixes f6c52f3c, e6d4973b, 56f745b5.

**R2 Remove `member flush` and `composer_draft`.**
- SPEC: §6.3 `member flush` (L1123–1185), output row L188, error rows L1340–1342, flush reconcile semantics (L1208–1210); §6.5 lock/termination rows L1534–1536, 1547, 1560, trade-off L1669, send-text L1583, 1791; §6.7 Composer drafts `composer_draft` (L2399–2432); §6.9 Forced flush (L4071–4134), control `flush` actions (L3873–3874, 3878), stall cause `forced_flush`.
- DDL: `kind='flush'`, `delivery_method='forced'`, `flush_input_id`, its index and 3 CHECKs.
- Catalog: every `composer_draft` declaration in `assets/profiles/bundled.json` and draft-only fixture entries.
- Code: `src/runtime/flush.rs` (385 prod, 16 tests), flush paths in `src/runtime/input.rs` (~420), catalog draft validation in `src/coding_agent/catalog.rs`; ~44 flush and 21 draft tests. Fixes c598c081, b9c3bb18.

**R3 One cause-free stall signal.** One `input_delivery_stalled` event, carrying `waiting_since` and one instruction (capture the pane, then submit or clear the draft with `member press`, or ask the user), in two cases: an unconsumed preview has waited the stall bound (3 minutes) behind a non-empty or unverified composer, deduplicated by run plus `message_id`; or a queued prompt or exec head has started no attempt within the stall bound of becoming the head, with time an interaction blocks it not counted, deduplicated by run plus input ID (as L4010 today). With rule 4's `idle_timeout` fallback gone (R4), this event is the Director's signal for a prompt or exec waiting behind a busy composer.
- SPEC: §6.9 Input delivery stalled event (L3705–3771) shrinks: 12 `cause.kind` values, `blocking_input_id`, `review_stalled_queue`/`review_stalled_input` collapse.
- Code: `src/broker/runtime/events.rs` (~350), stall derivation in `src/runtime/input.rs` (~120); ~33 tests. Fixes f044047c, 33698569.

**R4 Verified-empty delivery with retry.** On verified-composer runs a preview is typed only into a verified empty composer. A delivery that stops or is not confirmed records the attempt as uncertain with a diagnostic and leaves the message unnotified, so the sweep types it again once the composer is verified empty; duplicates are the accepted trade-off of deduplicating by message ID (§6.5 Accepted trade-offs). Runs without composer verification keep today's no-automatic-re-delivery rule (R1). With R6 rejected, prompt and exec keep their receipts: a stopped or unconfirmed prompt or exec records `uncertain` with its stop cause and exits 1, so the `left_in_composer` disposition and `left_stage` leave every kind, and `submit_unconfirmed` pairs with `uncertain`, stage `submitted`.
- FIFO-stop rule, on every run with or without composer verification: after a prompt or exec records `uncertain`, the FIFO stays stopped, and no queued input and no sweep preview is written, until `input reconcile` settles that input. This replaces the held composer, so no later head can type into a composer that may still hold the stopped text. Rule 1 becomes "An uncertain prior attempt under this operation's own key is an operational failure; while another prompt or exec of this run is uncertain, the FIFO is stopped (§ Runner-owned deferred delivery)"; the L4004–4006 rows become one row "A prompt or exec attempt records `uncertain` | Keep the FIFO stopped until `input reconcile` settles it | The uncertain result"; the held-composer paragraph (L4008) is deleted, and L3295 keeps "keeps the FIFO stopped until reconciliation" as a pointer to that row.
- SPEC: §6.9 rule 1 (L3962), rule 4 `idle_timeout` Escape fallback (L3965), the L4004–4006 rows, held composer (L4008), left-in-composer successor (L3932–3934), `review_left_input` (L3697–3703); Delivery outcome rows L799, 803–804; `abandoned_uncertain` for previews; prompt/exec `left_in_composer` sites L1055, 1079, 1200, 1208, 1210, 1344, 3291, 3293, 3295, 3663, 3673, 3752, 3858, 3920, 3926, 4145, 4183, 4187, 4191, 4202, 4210, 4328, 4331 and the §8 required-shape sentence (L4651).
- DDL: `left_in_composer` disposition, `left_stage` and its 3 CHECKs, `CHECK(state != 'uncertain' OR stop_cause IS NULL OR stop_cause != 'submit_unconfirmed')`; reasons `redelivery`, `idle_timeout`.
- Code: in `src/runtime/input.rs` (~800), the `left_in_composer` settlement and its successor insertion, the `idle_timeout` Escape fallback and the held composer go; composer verification and the staged-write stop stay; ~55 tests. Fixes d09d60af, 3fa83e8d.

**R5 Interaction-only stops.** Staged writes check (no interaction, verified empty, quiescent), then write the literal, settle, submit and verify; only an appearing interaction stops a write, and the stop cause becomes a free-text diagnostic.
- SPEC: §6.9 Stop cause (L4189–4210), stage vocabulary (L4187), pointer/keyboard consumer table (L3447–3455).
- DDL: `stop_cause` enum CHECKs.
- Code: shared with R4 in `src/runtime/input.rs`. Designs 0000201 and 0000228; fix 310a86dc.

**R7 Remove automatic-review annotations.**
- SPEC: §6.9 Annotation detection (L3515–3564), Automatic review events (L3641–3659), Effective policy record (L3773–3802), Director rows (L4287–4298), automatic-mode acceptance (L4306); event kinds `automatic_refusal_observed`, `automatic_review_unavailable`; `runtime.effective_policy`, `review=STATE` (L1200–1202).
- DDL: `member_runs.effective_policy_json`; event-kind CHECK values.
- Catalog: the `annotations` group (interaction-rule frame guards stay).
- Code: `src/runtime/observation.rs` (~150), `src/coding_agent/detector.rs` (~120), `src/coding_agent/catalog.rs` (~100), `src/broker/runtime/events.rs` (~200); ~29 tests.

**R8 Metadata-only evidence.**
- SPEC: §6.3 `--event-evidence`/`--event-redaction-file` (L964–965), help row L160; §6.9 redaction (L3627–3631), evidence refresh worker and ordering barrier (L3497–3499), `operation_rows`; the operator-pattern part of § Pattern syntax.
- DDL: `event_evidence`.
- Code: `src/broker/runtime/evidence.rs` (358 prod), refresh mailbox in `src/runtime/writer.rs` (~150), `update_interaction_evidence` (~70); ~70 tests.

**R9 Drop `interaction_obscured`.**
- SPEC: event kind (L130), 2 s obscured timer (L3491).
- Code: `src/runtime/observation.rs`; ~5 tests.

**R10 Direct event commits.** Events commit to SQLite with bounded retry and `event_seq` allocated in the transaction; a full channel or database failure marks the run `degraded` with held input, as today. Recovery verifies the lock is free, closes the open interaction with `monitor_lost`, writes `runner_lost` and settles.
- SPEC: §6.9 Storage pressure (L3808–3852) shrinks to its degraded paragraph; L3635, 3812; `member recover` `replayed_count` (L1220).
- DDL: `event_seq_reserved_through`.
- Code: outbox in `src/runtime/writer.rs` (~390), replay in `src/runtime/recovery.rs` (~400 of 891); ~25 outbox tests.

**R11 Remove the owner-terminated attestation.**
- SPEC: L3840–3848; the exceptional `member recover` form (L1218); outcome `operator_reconciliation_required`; `owner_termination_attestation` diagnostic.
- Code: `src/runtime/recovery.rs` (~150), `src/cli/member.rs`; ~7 tests.

**R12 Remove `member recover`.**
- SPEC: §6.3 L1214–1220, output row L179, Director-only guard entry, doctor next-commands (L711–717); every run status, owner kind and column that only `member recover` writes (candidates: status `recovering`, owner kind `reconciler`, `recovery_origin_status`), kept wherever `member delete` writes it.
- Skills: `assets/skills/cafleet/roles/director.md` § Recovery.
- Code: dispatch in `src/cli/member.rs`; recovery tests that `member delete` exercises stay. 10 `fix:` commits.

**R13 Slim permission responses.** Keep token-bound `permission inspect`/`respond`; drop the audit and claim tables, rearm, ready-row zero-byte proofs and the flags `--decision-source`, `--director-mode`, `--mode-evidence-ref`, `--decision-evidence-ref`, `--operation-key`; one answer per episode is enforced in runner memory.
- SPEC: §6.3 Permission commands (L1253–1325, partly), error rows L1389–1391; §6.9 Permission response storage (L4250–4266), parts of L4218–4248 and L3942–3948.
- DDL: `member_permission_responses`, `member_permission_response_claims`, `kind='permission_response'` claim columns.
- Code: permission paths in `src/runtime/input.rs` (~700), `src/cli/runtime.rs` (348), `src/runtime/input_client.rs`, `src/broker/runtime/inputs.rs`; ~138 tests.

**R16 Shared subprocess runner for `member press`.**
- SPEC: §6.5 L1589–1593 (absolute deadline, 250 ms cleanup reserve, reap contract).
- Code: `src/runtime/press.rs` (104), the absolute-deadline runner (`run_until` → `run_process_until` in `src/runtime/system.rs`), which the multiplexer core's `press_run` (`src/multiplexer/mod.rs`) also uses for `send_keys`, `send_text` and `check_key_target`; 22 tests; 7 `fix:` commits.

**R18 Flat `doctor`.** One `STATUS NAME: DETAIL` row per check; `--json` an array; multiplexer, database and per-backend install/extension/plugin checks stay.
- SPEC: §6.3 doctor summary (L590–641), verbose rows (L642–646), JSON `{summary, checks}` (L648–667), member runtime checks (L697–766).
- Docs: `docs/docs/mixed-backend-team.md` (`doctor --json` use).
- Code: ~1,000 of `src/cli/setup.rs`'s 1,189 prod lines, `diagnose_runtime_at` in `src/runtime/control.rs`; ~53 tests.

**R19 Simplified Codex forwarding.**
- SPEC: §6.3 L920–922, 929–936 (full-queue retries, in-place `closed` retry, best-effort closed notice).
- Code: `src/cli/message.rs` (~320 of L636–956); 13 tests.

**R20 Hermes settlement through profile delivery.** Submit `/stop` and `/exit` through the runner's Hermes profile delivery; keep the `processes.json` verification.
- SPEC: §6.7 Hermes work settlement (L2244–2263).
- Code: ~480 of `src/coding_agent/hermes.rs`'s 698 prod lines; 35 tests; 6 `fix:` commits.

**R21 Drop `asset_installs`.**
- SPEC: §6.3 L280, 310–312, doctor `.installation` row L572, phrases L629–631.
- DDL: `asset_installs`.

#### Rejected candidates

The user rejected these candidates; their behavior stays as it is, and no step implements them. Their inventories record what each would have removed.

**R6 Drop prompt/exec receipts.** Prompt/exec become one attempt with a reported result; a lost reply reports uncertain.
- SPEC: §6.3 input commands (L1208–1212), `unresolved_input_ids`, the 4096 bound; §6.9 Durable input receipts (L3907–3950), pending finalization (L4012), immediate-attempt failure bookkeeping (L4016–4018), receipt control actions (L3870–3872). With R1, R2 and R13 the whole `member_input_operations` table.
- Code: receipt handling and rearm in `src/runtime/input.rs` (~600); ~116 receipt/reconcile tests.

**R14 One window/tab per member** named `cafleet-<fleet>-<member>` (`new-window -d` / tab create without focus); closing the member's only pane closes its window natively.
- SPEC: §6.5 Fleet window layout (L1691–1767), herdr re-parenting (L1837–1853), column equalization (L1855–1877), Delete-time pane layout (L1508–1512), most of Member pane creation (L1803–1835), Window height/Rename/Window name rows (L1787–1789); §6.2 ordinal CAS (L95–97); `fleet create` window naming/read-back (L402–406); layout lock (L973, 1031); error rows L1357–1361, 1364; the herdr `--amount` and layout-constant rows of § Source-only constants.
- DDL: `fleets.last_window_ordinal`, `member_placements.window_ordinal`, `member_placements.column_side`.
- Code: `src/multiplexer/layout.rs` (84), layout parts of `src/multiplexer/herdr.rs` (~210) and `src/multiplexer/tmux.rs` (~40), slot code in `src/cli/member.rs` (~140), layout lock in `src/pane_lock.rs` and `src/cli/fleet.rs`; ~67 tests; 15 `fix:` commits.

**R15 Reduced caller-pane verification.** Trust `TMUX_PANE`/`HERDR_PANE_ID` unless an ancestor matches a shared-server signature; otherwise require `--pane`. A minimal ancestry reader stays (the Codex session pin uses it).
- SPEC: §6.3 L410–523, doctor rows L579–588, 603–611; §6.5 L1471, Pane process row L1786; §6.8 L2588.
- Code: `src/multiplexer/caller.rs` (276 prod, 43 tests).

**R17 Remove `member exec`.**
- SPEC: §6.3 `member exec`, native `!` plans, the `exec` receipt kind, pi's continuation rule.

#### Retained as load-bearing

These were never candidates: notice hold and ceiling (host queue caps), Director-only guard, pane input lock, termination deferral, setup config-dir resolution and per-backend installs, Hermes plugin enablement, the pi member gate, the Codex forwarding core, creation compensation, capture windowing, diff sidebar clipping, native suggestions, terminal reply classification, continuation screens, focused-choice resolution, plain limits/bracketed paste/pointer form, 2 s post-submit verification, episode stability/debounce/supersede, ownership lock/epoch and the shutdown clock.

---

### Workstream C: Test simplification

#### Current state

2,081 `#[test]` functions in 87.6k lines (63 % of the tree), 349 depending on real time, 179 `std::thread::sleep` calls, 29 `test_support` modules with ~680 cross-module imports, and four separate clock abstractions (`time::Clock`, `observation::ObservationClock` (`Send + Sync`), `input::InputDeadlineClock`, `lifecycle::ShutdownClock`). Duplication is low (22 identical clusters covering 68 tests); the size comes from hand-built state and one-function-per-case style.

#### Techniques

| Technique | Specification |
|---|---|
| Test ledger | `test-ledger.tsv` (D13), generated in Step B0 from `mise //:test -- -- --list` with columns `test_path`, `file`, `fix_commit`, `fate`; Step C1 adds `class` (1–8 below). `fix_commit` is the first commit returned by `git log -S'fn <name>(' --reverse --format=%h%x09%s -- src`, kept only when its subject starts with `fix`; when `<name>` is defined in more than one file, the search is limited to the defining file and its predecessors from `git log --follow --name-only -- <file>`. `fate` is `keep`, `merged:<target test>` or `dropped:<reason>`. A row with a `fix_commit` keeps its case (as a named table row when merged), except fate `dropped:feature removed (R<n>)` for an approved candidate. |
| Table-driven cases | One `#[test]` iterating a `const`/`static` array of named cases, asserting with the case name in the failure message; applied first to classes 3, 4, 5 and 8 below and to the 22 identical clusters. |
| One runtime clock | Scope: every production read of `Instant::now`, `SystemTime::now`, `CLOCK_MONOTONIC` and `std::thread::sleep` in `src/runtime/` and `src/broker/runtime/`. `time::Clock` becomes `Send + Sync` and gains the capabilities of the other three abstractions (monotonic `Instant`, wall stamp, `CLOCK_MONOTONIC` sample bracket, sleep); the runtime dependency structs pass it to services, the observation controller, input deadlines and shutdown. `SystemClock` keeps today's calls, so the change is behavior-preserving. `time::test_support::TestClock` becomes thread-safe (`Mutex`/atomics instead of `Cell`/`RefCell`), advances virtually and records sleeps. In-scope files that sleep directly today, such as `src/runtime/system.rs` and `src/runtime/artifacts.rs`, call `Clock::sleep` instead. Code outside the scope (for example `src/cli/`, `src/broker/messaging.rs`, `src/delivery.rs`, `src/pane_lock.rs`, `src/db.rs`, `src/coding_agent/`) keeps its direct calls. |
| Runtime scenario harness | `#[cfg(test)] pub(crate) mod test_support` in `src/runtime/service.rs` (the file owning the runner loop): a `Scenario` builder with the existing fake PTY/display/input, a scripted native child that emits screens rendered from `assets/profiles/native/*.json` fixtures (inputs only), the fake multiplexer, a temporary SQLite file, `TestClock`, and SQLite fault plans from `db::test_support`. A scenario is data: screens fed, commands issued, expected events, rows and child bytes. Class-7 tests move to scenarios in the file that owns the asserted behavior. |
| Real-time tests | Tests of in-scope code use `TestClock`; among them, `std::thread::sleep` and real deadlines remain only in class-6 tests that drive real processes, PTYs, sockets or locks. Tests of out-of-scope code exercise production code that keeps real time, so they may keep real-time sleeps, each with a bounded deadline. |
| Deduplication | The 22 identical clusters collapse into table tests; the 22 shared-prefix groups (e.g. `runner_recovery_missing_path*`, `event_shape_regression_rejects*`) are reviewed for merging. |
| Removed-feature tests | Tests of every approved Workstream B candidate are deleted with the feature and recorded with fate `dropped:feature removed (R<n>)`. |

Test classes (from the feasibility classification, approximate):

| # | Class | Tests | Target form |
|---|---|---|---|
| 1 | Rust mechanism (negative trait checks, unwinding restoration, drop order) | 28 | Keep (still meaningful in Rust) |
| 2 | CLI/output contract | 158 | Table-driven in-file command tests in the `src/cli/*.rs` file that owns each command (TD-1 (a)) |
| 3 | Multiplexer argv/choreography | 149 | argv tables |
| 4 | Screen classification and catalog | 324 | fixture-id → expected-classification tables plus catalog-validation tables |
| 5 | Terminal byte handling | 151 | byte-input tables |
| 6 | Real OS process/PTY/lock/signal | 125 | Keep, real time |
| 7 | Durable state machines (broker/runtime transactions, receipts, events, recovery, delivery) | 970 | Runtime scenarios and per-transaction operation tables |
| 8 | Pure logic | 176 | tables |

#### Rule changes

Per TD-2 (a), `.claude/rules/code-quality.md` § Test placement gains these three bullets, which are compatible with the in-file rule; its Forbidden patterns are unchanged:

> - Write related cases as one table-driven test over a named case array, naming the case in every assertion message.
> - Runner code in `src/runtime/` and `src/broker/runtime/` takes time from the injected `time::Clock`, and its tests use `TestClock`; real sleeps and real deadlines belong only to tests that drive real processes, PTYs, sockets or locks.
> - Drive runner behavior through the scenario harness in `src/runtime/service.rs` `test_support`, asserting events, rows and child bytes.

Per TD-1 (a), `CLAUDE.md` § Tests is unchanged.

#### Coverage tooling

`mise.toml` adds the Rust component `llvm-tools-preview` to `core:rust`, pins `cargo:cargo-llvm-cov`, and defines task `coverage` (`cargo llvm-cov --bin cafleet --summary-only`), described as "Report line coverage of the inline unit tests".

#### Gates

- `mise //:test` wall time at Step C5 does not exceed the Step C1 baseline.
- Production line coverage reported by `mise //:coverage` at Step C5 is at least the Step C1 baseline minus 0.5 percentage points.
- Every ledger row is resolved, and no `fix:`-traced case is lost outside `dropped:feature removed (R<n>)`.

#### Gate results

| Measure | Step C1 baseline | Step C5 result |
|---|---|---|
| `mise //:test` wall time | — | — |
| Production line coverage | — | — |
| Ledger rows kept / merged / dropped | — | — |

---

### Repository-wide updates

| Surface | Workstream | Change |
|---|---|---|
| `SPEC.md` | A, B | Neutrality edits; removal of every approved candidate's contract |
| `docs/docs/contributing.md` | A, C | Moved test/build/release content; testing techniques summary; coverage task |
| `docs/docs/concepts/coding-agents.md` | A | F16 anchor |
| `assets/skills/cafleet/SKILL.md`, `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md`, `assets/skills/cafleet/roles/director.md` | B | Approved candidates' commands, flags, outputs, events and procedures (known hits per Step B1–B16 task) |
| `docs/docs/concepts/members.md`, `overview.md`, `storage.md`, `coding-agents.md`, `docs/docs/quickstart.md`, `docs/docs/mixed-backend-team.md`, `docs/docs/contributing.md` | B | Same, including `SPEC.md` anchor links to removed headings |
| Every further docs page, skill file and `SPEC.md` anchor link a Step B1–B16 grep finds | B | Removed or rewritten with its candidate |
| `README.md` | B | Only if a docs-site section is added or removed |
| `assets/skills/cafleet/reference/cli.md` | A | F14 wording |
| `.claude/rules/documentation-maintenance.md` | A | § Neutrality rule |
| `.claude/rules/code-quality.md` | C | § Rule changes bullets (TD-2 (a)) |
| `mise.toml` | C | § Coverage tooling |
| `assets/profiles/bundled.json` and `assets/profiles/native/` | A, B | § Bundled pattern rewrite; R2 `composer_draft` declarations and draft-only fixtures; R7 `annotations` group |
| `assets/migrations/V1__baseline.sql` | B | In-place DDL removal for approved candidates |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step B0: Test ledger

- [ ] Generate `test-ledger.tsv` from `mise //:test -- -- --list` with `test_path`, `file`, `fix_commit` and empty `fate` columns per § Techniques <!-- completed: -->

### Step A1: Owning definitions in SPEC.md

- [ ] Rewrite the preamble per § Preamble <!-- completed: -->
- [ ] Add § Pattern syntax (with the catalog invariant), § JSON encoding, § Quoted text, § Parse errors and help and § Screen model facts at their owning sections <!-- completed: -->
- [ ] Run § Verification check 3, extend § Source-only constants with any further internal constant, then apply every row of that table <!-- completed: -->
- [ ] Confirm the ledger DDL against a freshly initialized file's `sqlite_master` and add it to §8 <!-- completed: -->

### Step A2: Findings sweep

- [ ] Apply F1–F68 per § Finding dispositions <!-- completed: -->
- [ ] Apply S1–S7 <!-- completed: -->
- [ ] Update every catalog pattern quoted in `SPEC.md` to its § Bundled pattern rewrite form <!-- completed: -->
- [ ] Rewrite §9 per § §9 rewrite <!-- completed: -->
- [ ] Run every § Verification check, resolve each hit and record further items as S9+ <!-- completed: -->

### Step A3: Dependent surfaces, data and confirmation tests

- [ ] Add `## Testing and release acceptance` to `docs/docs/contributing.md` with the moved content and update its `native-acceptance-and-coverage` link <!-- completed: -->
- [ ] Update the F16 anchor in `docs/docs/concepts/coding-agents.md` <!-- completed: -->
- [ ] Update the F14 wording in `assets/skills/cafleet/reference/cli.md` <!-- completed: -->
- [ ] Add the § Neutrality rule to `.claude/rules/documentation-maintenance.md` <!-- completed: -->
- [ ] Rewrite `assets/profiles/bundled.json` per § Bundled pattern rewrite <!-- completed: -->
- [ ] Extend the bundled-catalog guard tests in `src/coding_agent/catalog.rs` with the data-driven explicit-class check of § Pattern syntax, failing with profile, rule ID and construct, and list it in `docs/docs/contributing.md` § Bundled catalog provenance checks <!-- completed: -->
- [ ] Add the in-file quoted-text table test in `src/output/text.rs` and confirm no JSON encoder receives a non-integer number <!-- completed: -->
- [ ] Run `mise //:test`, `mise //:lint` and `mise //docs:build` <!-- completed: -->

### Workstream B steps

Each step implements one approved candidate (Step B2 implements R1 and R4 together, because R1's delivery path is R4's verified-empty rule) from its § Approved candidate inventories entry, in documentation-first order: `SPEC.md` (including the §8 DDL block and its required-shape sentence), then docs and skills, then data, code and tests. Step order follows code dependencies: Step B1 removes forced flush before Step B2 removes the preview receipts it references, Step B4 collapses stall causes after Steps B1–B2 remove their sources, Step B11 removes `member recover` after Steps B9–B10 remove the replay and attestation it runs, Step B12 flattens doctor after Step B11 removes its recover next-commands, and Step B15 moves pane input to the shared subprocess runner after Steps B1 and B14 remove the flush and Hermes-settlement callers of the absolute-deadline runner.

Every step's docs-and-skills task edits the known hits it lists and every further hit of `grep -rnE '<tokens>' SPEC.md docs/docs assets/skills README.md` for its tokens. Every step's code task removes the step's DDL from `assets/migrations/V1__baseline.sql` and setup's required-shape guard, deletes the candidate's tests with ledger fate `dropped:feature removed (R<n>)`, adds in-file tests of the behavior the step leaves, and ends with `mise //:test`, `mise //:lint` and `mise //docs:build` passing.

### Step B1: Remove forced flush and composer drafts (R2)

- [ ] `SPEC.md`: delete §6.3 `member flush` (L1123–1185), its output row (L188), error rows (L1340–1342) and the flush reconcile semantics with `flush_input_id`/`flushed_input_ids` (L1208–1210); delete the §6.5 flush lock and termination rows (L1534–1536, 1547, 1560), trade-off L1669 and send-text sites L1583, 1791; delete §6.7 Composer drafts (L2399–2432); delete §6.9 Forced flush (L4071–4134) with the flush literal limit, the control `flush` actions (L3873–3874, 3878) and stall cause `forced_flush`; delete from §8 `kind='flush'`, `delivery_method='forced'`, `flush_input_id`, its index and the three forced/flush CHECKs <!-- completed: -->
- [ ] Docs and skills (tokens `member flush`, `composer_draft`, `forced_flush`, `forced`, `flush_input_id`, `flushed_input_ids`, `#member-flush`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md` (including its `SPEC.md#member-flush` link), `docs/docs/concepts/coding-agents.md`, `docs/docs/quickstart.md` <!-- completed: -->
- [ ] Data, code and tests: remove every `composer_draft` declaration and draft-only fixture entry from `assets/profiles/bundled.json` and `assets/profiles/native/`, the draft validation in `src/coding_agent/catalog.rs`, `src/runtime/flush.rs`, the flush paths in `src/runtime/input.rs` and the `member flush` command, with their DDL and tests <!-- completed: -->

### Step B2: Runner-pull, verified-empty preview delivery (R1, R4)

- [ ] `SPEC.md` (R1): delete the §6.3 `message send` delivery table and outcomes (L776–812), the output keys `notification_sent`, `input_id`, `delivery` (L172) and the reasons `notification_held`, `notification_failed`; rewrite §6.5 Push notifications (L1625–1648) so `message send` stores the message and the recipient's runner delivers it at its next preview sweep, reducing pane-lock writer row L1531 and termination rows L1544, 1557 to their remaining input kinds; in §6.9 delete the sweep's first-attempt clause (L4044), the FIFO's preview membership (L3982), the preview rows of the latest-attempt table (L3909–3922), retried-preview results (L3665–3676) and the preview rows of Queue housekeeping (L4020–4040); restate the sweep preconditions row (L4049) and state the sweep-order rule, the R1 delivery state and the uncertain-preview event, its fields, deduplication and live-only publication, replacing the receipt-derived fields and durability sentence for previews (L3685–3695) (§ Approved candidate inventories, R1); delete the control-frame `preview` request kind and its message-ID rule (L3876), and the `input show` key `message_id` and text field `message=MESSAGE_ID` (L1208) <!-- completed: -->
- [ ] `SPEC.md` (R4): state the verified-empty rule, the uncertain-preview retry and the FIFO-stop rule for every run (R4 inventory), restating rule 1 (L3962), replacing the L4004–4006 rows with the one FIFO-stop row, deleting the held-composer paragraph (L4008) and keeping L3295 as a pointer to that row; delete rule 4's `idle_timeout` Escape fallback (L3965), the left-in-composer successor and stages (L3932–3934), `review_left_input` (L3697–3703), Delivery outcome rows L799, 803–804 and preview `abandoned_uncertain`; restate every prompt/exec `left_in_composer` site listed in the R4 inventory as a stop recorded `uncertain`, removing `left_stage` from the `input show` key list (L1208) and moving the `submit_unconfirmed` association (L4202) to `uncertain`, stage `submitted`; delete from §8 `kind='preview'`, `preview_attempt`, `message_id`, `UNIQUE(run_id, message_id, preview_attempt)`, `CHECK(kind != 'preview' OR message_id IS NOT NULL)`, the `left_in_composer` disposition, `left_stage` with its three CHECKs, `CHECK(state != 'uncertain' OR stop_cause IS NULL OR stop_cause != 'submit_unconfirmed')` and the reasons `redelivery` and `idle_timeout`, and remove `preview_attempt` and `left_stage` from the required-shape sentence (L4651) <!-- completed: -->
- [ ] Docs and skills (tokens `notification_sent`, `notification_held`, `notification_failed`, `left_in_composer`, `left_stage`, `held composer`, `idle_timeout`, `redelivery`, `review_left_input`, `preview_attempt`, `#push-notifications`, `#delivery-reporting`): `assets/skills/cafleet/SKILL.md` § Send (Unicast), `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md`, `docs/docs/concepts/overview.md` <!-- completed: -->
- [ ] Code: delete `src/delivery.rs` and `send_delivery` in `src/cli/message.rs`; make the runner sweep the only preview path in `src/runtime/service.rs`, `src/runtime/input.rs`, `src/runtime/input_client.rs` and `src/broker/runtime/inputs.rs`, typing only into a verified empty composer and stamping `notified_at` after confirmed submission; remove the control-frame `preview` kind, the `left_in_composer` settlement and its successor insertion, the `idle_timeout` Escape fallback and the held composer, keeping composer verification and the staged-write stop; stop the FIFO after an uncertain prompt or exec until `input reconcile`; remove the step's DDL <!-- completed: -->
- [ ] Tests: delete the removed paths' tests (fate `dropped:feature removed (R1)` or `(R4)`) and add in-file tests that `message send` writes no child byte; that the next sweep delivers into a verified empty composer; that a non-empty composer blocks delivery; that an unconfirmed preview is typed again once the composer is verified empty; that an uncertain preview publishes one `input_delivery_result` with `input_id` null and its `message_id`, and on a run without composer verification stamps `notified_at` first; that a prompt whose post-submit verification expires records `uncertain/submit_unconfirmed`; that after an uncertain prompt or exec, on a verified-composer run and on a run without composer verification, neither a queued exec nor a sweep preview writes a byte until `input reconcile` settles it; and that when an interaction closes over a queued prompt and a pending preview, the prompt is written first and the sweep starts only after the FIFO is empty <!-- completed: -->

### Step B3: Interaction-only stops (R5)

- [ ] `SPEC.md`: rewrite §6.9 Stop cause (L4189–4210) so a staged write checks no interaction, verified empty and quiescent, then writes, settles, submits and verifies, and only an appearing interaction stops it, recording its stop cause as a free-text diagnostic; reduce the stage vocabulary (L4187) and the pointer/keyboard consumer table (L3447–3455) to that rule; reduce the §8 `stop_cause` CHECKs to non-empty text on `uncertain` and `settled` rows <!-- completed: -->
- [ ] Docs and skills (tokens `stop_cause`, `manual_input`, `manual_accounting_failed`, `pointer_input`, `interaction_candidate`, `observer_unavailable`, `submit_unconfirmed`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md` <!-- completed: -->
- [ ] Code and tests: reduce the staged-write stop logic in `src/runtime/input.rs` to the rule above, with its DDL and tests; add a table test of the interaction stop and of post-submit detection of an unsubmitted result <!-- completed: -->

### Step B4: One stall signal (R3)

- [ ] `SPEC.md`: rewrite §6.9 Input delivery stalled event (L3705–3771) and the stall paragraph (L4010) as one `input_delivery_stalled` event, carrying `waiting_since` and one instruction, for the two cases of the R3 inventory: an unconsumed preview waiting the stall bound behind a non-empty or unverified composer (deduplicated by run plus `message_id`) and a queued prompt or exec head that has started no attempt within the stall bound (deduplicated by run plus input ID); delete the `cause.kind` values, `blocking_input_id`, `review_stalled_queue` and `review_stalled_input` <!-- completed: -->
- [ ] Docs and skills (tokens `input_delivery_stalled`, `cause.kind`, `blocking_input_id`, `review_stalled_queue`, `review_stalled_input`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md` <!-- completed: -->
- [ ] Code and tests: reduce `src/broker/runtime/events.rs` and the stall derivation in `src/runtime/input.rs` to the one event, with their tests; add a table test of the event's emission at the stall bound for a preview and for a queued prompt and exec head, each published once under its deduplication key <!-- completed: -->

### Step B5: Remove automatic-review annotations (R7)

- [ ] `SPEC.md`: delete §6.9 Annotation detection (L3515–3564), Automatic review events (L3641–3659), Effective policy record (L3773–3802) and the Director rows (L4287–4298); delete the event kinds `automatic_refusal_observed` and `automatic_review_unavailable`, `runtime.effective_policy` and `review=STATE` (L1200–1202), and from §8 `member_runs.effective_policy_json` and the two event-kind CHECK values; delete the annotation rows that Step A3 moved from L4306 into `docs/docs/contributing.md` § Release acceptance <!-- completed: -->
- [ ] Docs and skills (tokens `automatic_refusal_observed`, `automatic_review_unavailable`, `effective_policy`, `effective policy`, `review=`, `annotation`, `#automatic-review-events`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md` (including its `SPEC.md#automatic-review-events` link), `docs/docs/concepts/coding-agents.md`, `docs/docs/contributing.md`; text about a member routing its backend's own automatic-review refusal stays <!-- completed: -->
- [ ] Data, code and tests: remove the `annotations` group from `assets/profiles/bundled.json` (interaction-rule frame guards stay) and the annotation code in `src/runtime/observation.rs`, `src/coding_agent/detector.rs`, `src/coding_agent/catalog.rs` and `src/broker/runtime/events.rs`, with their DDL and tests <!-- completed: -->

### Step B6: Metadata-only evidence (R8)

- [ ] `SPEC.md`: delete `--event-evidence` and `--event-redaction-file` (L964–965), help row L160, redaction (L3627–3631), the evidence refresh worker and ordering barrier (L3497–3499) and `operation_rows`, so events and `permission inspect` carry metadata only; remove operator redaction patterns and the §6.9 Persistence pointer from § Pattern syntax; delete §8 `event_evidence` <!-- completed: -->
- [ ] Docs and skills (tokens `event-evidence`, `event-redaction-file`, `event_evidence`, `redact`, `operation_rows`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md`, `docs/docs/concepts/storage.md` <!-- completed: -->
- [ ] Code and tests: delete `src/broker/runtime/evidence.rs`, the refresh mailbox in `src/runtime/writer.rs`, `update_interaction_evidence` and the two `member create` flags, with their DDL and tests <!-- completed: -->

### Step B7: Drop `interaction_obscured` (R9)

- [ ] `SPEC.md`: delete the event kind (L130) and the 2 s obscured timer (L3491), and the kind from the §8 CHECK <!-- completed: -->
- [ ] Docs and skills (tokens `interaction_obscured`, `obscured`): `assets/skills/cafleet/reference/cli.md` <!-- completed: -->
- [ ] Code and tests: remove the obscured timer and event from `src/runtime/observation.rs`, with its DDL and tests <!-- completed: -->

### Step B8: Slim permission responses (R13)

- [ ] `SPEC.md`: keep token-bound `permission inspect` and `permission respond`; delete the flags `--decision-source`, `--director-mode`, `--mode-evidence-ref`, `--decision-evidence-ref` and `--operation-key`, the permission-response audit output and `input show` key `permission_response`, error rows L1389–1391, §6.9 Permission response storage (L4250–4266) and the permission parts of L4218–4248 and L3942–3948 (rearm, ready-row zero-byte proofs, claims); state that the runner accepts one answer per episode, held in memory; delete from §8 `member_permission_responses`, `member_permission_response_claims` and `kind='permission_response'` <!-- completed: -->
- [ ] Docs and skills (tokens `decision-source`, `director-mode`, `mode-evidence-ref`, `decision-evidence-ref`, `operation-key`, `permission_response`, `#request-bound-permission-responses`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md`, `docs/docs/concepts/members.md` <!-- completed: -->
- [ ] Code and tests: remove the permission receipt, audit and claim paths from `src/runtime/input.rs`, `src/cli/runtime.rs`, `src/runtime/input_client.rs` and `src/broker/runtime/inputs.rs`, with their DDL and tests; add a test that a second answer for the same episode is refused <!-- completed: -->

### Step B9: Direct event commits (R10)

- [ ] `SPEC.md`: rewrite §6.9 Storage pressure (L3808–3852) as direct event commits with bounded retry and `event_seq` allocated in the transaction, keeping its degraded paragraph, and state recovery as a check that the pane lock is free, `monitor_lost` close of the open interaction, `runner_lost` and settlement; update L3635, 3812; delete `member recover` `replayed_count` (L1220) and §8 `event_seq_reserved_through` <!-- completed: -->
- [ ] Docs and skills (tokens `outbox`, `spool`, `replayed_count`, `event_seq_reserved_through`, `#storage-pressure-and-durability-boundary`): `docs/docs/concepts/storage.md` (including its link), `assets/skills/cafleet/reference/cli.md` <!-- completed: -->
- [ ] Code and tests: delete the outbox in `src/runtime/writer.rs` and the replay in `src/runtime/recovery.rs`, with their DDL and tests; add tests of direct commit, retry exhaustion marking the run `degraded` and recovery's `runner_lost` <!-- completed: -->

### Step B10: Remove the owner-terminated attestation (R11)

- [ ] `SPEC.md`: delete L3840–3848, the exceptional `member recover` form (L1218), outcome `operator_reconciliation_required` and the `owner_termination_attestation` diagnostic; state that `member delete` resolves a runner holding an unlinked live lock by force-closing its pane <!-- completed: -->
- [ ] Docs and skills (tokens `owner-terminated`, `operator_reconciliation_required`, `owner_termination_attestation`, `attestation`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/roles/director.md` <!-- completed: -->
- [ ] Code and tests: remove the attestation paths from `src/runtime/recovery.rs` and `src/cli/member.rs`, with their tests <!-- completed: -->

### Step B11: Remove `member recover` (R12)

- [ ] `SPEC.md`: delete §6.3 `member recover` (L1214–1220), its output row (L179), its Director-only guard entry and the doctor next-commands (L711–717); delete every run status, owner kind and column that only `member recover` writes (R12 inventory), from §6.9 and §8 <!-- completed: -->
- [ ] Docs and skills (tokens `member recover`, `recover`, `recovering`, `reconciler`): rewrite `assets/skills/cafleet/roles/director.md` § Recovery as delete and re-create; `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md`, `assets/skills/cafleet/SKILL.md`, `docs/docs/concepts/members.md`, `docs/docs/concepts/storage.md`, `docs/docs/mixed-backend-team.md`; uses of "recover" that do not name the command stay <!-- completed: -->
- [ ] Code and tests: remove the `member recover` dispatch from `src/cli/member.rs` and the recovery code only it reaches, with its DDL and tests; recovery code and tests that `member delete` exercises stay <!-- completed: -->

### Step B12: Flat `doctor` (R18)

- [ ] `SPEC.md`: rewrite §6.3 doctor output as one `STATUS NAME: DETAIL` row per check and `--json` as a JSON array of today's check objects in check order, replacing the summary (L590–641), verbose rows (L642–646) and `{summary, checks}` (L648–667); delete the member runtime checks (L697–766); multiplexer, database and per-backend install, extension and plugin checks stay <!-- completed: -->
- [ ] Docs and skills (tokens `doctor --json`, `summary`, `checks`, `doctor`): `docs/docs/mixed-backend-team.md`, `assets/skills/cafleet/roles/director.md`, `assets/skills/cafleet/reference/cli.md`, `docs/docs/concepts/members.md`, `docs/docs/concepts/coding-agents.md` <!-- completed: -->
- [ ] Code and tests: remove the summary folding from `src/cli/setup.rs` and `diagnose_runtime_at` from `src/runtime/control.rs`, with their tests; add a table test of the flat text rows and the JSON array <!-- completed: -->

### Step B13: Simplified Codex forwarding (R19)

- [ ] `SPEC.md`: replace §6.3 L920–922 and 929–936 (full-queue retries, in-place `closed` retry, best-effort closed notice) with: forwarding exits on any non-zero `codex queue` exit <!-- completed: -->
- [ ] Docs and skills (tokens `codex queue`, `full queue`, `closed`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md`, `docs/docs/contributing.md` <!-- completed: -->
- [ ] Code and tests: remove the retry paths from `src/cli/message.rs` (L636–956), with their tests; add a test that a non-zero `codex queue` exit ends forwarding <!-- completed: -->

### Step B14: Hermes settlement through profile delivery (R20)

- [ ] Live check: on a Hermes member under tmux and under herdr, confirm that the Hermes profile's empty and busy rules classify the composer before `/stop`, after each `/stop` report (`Stopped N process(es).`, `No running background processes.`) and before `/exit`; record the result in this task. When a rule misses a state, stop and route the result to the Director, and the remaining Step B14 tasks wait for the user's decision <!-- completed: -->
- [ ] `SPEC.md`: rewrite §6.7 Hermes work settlement (L2244–2263) so settlement submits `/stop` and `/exit` through the runner's Hermes profile delivery, keeping session identity and the `processes.json` verification; delete the clearing loop, its measurements and the settlement poll interval row of § Source-only constants <!-- completed: -->
- [ ] Docs and skills (tokens `/stop`, `/exit`, `settlement`): `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md` <!-- completed: -->
- [ ] Code and tests: replace the clearing and entry code in `src/coding_agent/hermes.rs` with profile delivery, with its tests; add a scripted-screen test of settlement through profile delivery <!-- completed: -->

### Step B15: Shared subprocess runner for pane input (R16)

- [ ] `SPEC.md`: replace §6.5 L1589–1593 (absolute `run_until` deadline, 250 ms cleanup reserve, reap contract) with: every press process — the availability, current-context and target-metadata probes and the write, for `member press` and every other pane-input caller — runs through the shared bounded subprocess runner (§6.5 Subprocess output and deadlines) within the five-second invocation deadline <!-- completed: -->
- [ ] Docs and skills (tokens `member press`, `cleanup reserve`, `reap`, `run_until`): `assets/skills/cafleet/reference/cli.md` <!-- completed: -->
- [ ] Code and tests: route `src/runtime/press.rs` and the multiplexer core's `press_run` (`src/multiplexer/mod.rs`, serving `send_keys`, `send_text` and the tmux/herdr `check_key_target` probes) through the shared runner, then delete `run_until` from the runner trait and `run_process_until` from `src/runtime/system.rs`, with their tests; by this step `src/runtime/flush.rs` is gone (Step B1) and Hermes settlement no longer calls `send_keys` (Step B14) <!-- completed: -->

### Step B16: Drop `asset_installs` (R21)

- [ ] `SPEC.md`: delete `asset_installs` from §6.3 (L280, 310–312), the doctor `.installation` row (L572) and phrases (L629–631), stating that doctor byte-compares installed skills as it does extensions; delete it from §8 <!-- completed: -->
- [ ] Docs and skills (tokens `asset_installs`, `installation`): `docs/docs/concepts/storage.md` <!-- completed: -->
- [ ] Code and tests: remove the table's writes and reads from setup and doctor, with its DDL and tests; add a test that doctor reports a modified installed skill <!-- completed: -->

### Step C1: Baselines and classes

- [ ] Add § Coverage tooling to `mise.toml` and the coverage task to `docs/docs/contributing.md` <!-- completed: -->
- [ ] Measure `mise //:test` wall time and `mise //:coverage` line coverage and record both in § Gate results <!-- completed: -->
- [ ] Add the `class` column to every remaining ledger row <!-- completed: -->

### Step C2: Rules and clock

- [ ] Add the three § Rule changes bullets to `.claude/rules/code-quality.md` § Test placement and summarize the techniques in `docs/docs/contributing.md` <!-- completed: -->
- [ ] Make `time::Clock` `Send + Sync` with the capabilities of `ObservationClock`, `InputDeadlineClock` and `ShutdownClock`, make `TestClock` thread-safe, and remove the three abstractions <!-- completed: -->
- [ ] Route every in-scope production time read and sleep in `src/runtime/` and `src/broker/runtime/` through the injected clock <!-- completed: -->

### Step C3: Harness

- [ ] Build the runtime scenario harness in `src/runtime/service.rs` `test_support` with fixture-rendered screens, fake multiplexer, temporary SQLite, `TestClock` and SQLite fault plans <!-- completed: -->

### Step C4: Conversion

- [ ] Convert classes 3, 4, 5 and 8 and the 22 identical clusters to table-driven tests, updating ledger fates <!-- completed: -->
- [ ] Convert class 7 to scenarios and per-transaction operation tables, replacing real sleeps with `TestClock`, updating ledger fates <!-- completed: -->
- [ ] Convert class 2 to table-driven in-file command tests in the `src/cli/*.rs` file that owns each command, updating ledger fates <!-- completed: -->

### Step C5: Gates

- [ ] Resolve every ledger row and confirm no `fix:`-traced case was lost outside `dropped:feature removed (R<n>)` <!-- completed: -->
- [ ] Confirm in-scope production code reads time only through `time::Clock` and that, among tests of in-scope code, `std::thread::sleep` appears only in class-6 tests <!-- completed: -->
- [ ] Measure wall time and coverage against § Gates and record them and the ledger counts in § Gate results <!-- completed: -->
- [ ] Run `mise //:test`, `mise //:lint` and `mise //docs:build` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-21 | Initial draft |
| 2026-11-21 | Reviewer round 1: neutrality rule carve-outs, pattern syntax escapes and implementation-defined acceptance, JSON scope, quoted-text Unicode version, internal constant names, shape-based verification, decision-view candidate tables with inventories, R4/R7 merged (retry uncertain previews), R1 without wake frame, ledger before Workstream B with corrected fix tracing, clock scope, TD-2, coverage tooling, step order fixes |
| 2026-11-21 | Reviewer round 2: guard test for the explicit-class catalog invariant; real-time sleep rule scoped to tests of in-scope code |
| 2026-11-21 | User decisions recorded: TD-1 (a), TD-2 (a), 17 candidates approved (R1–R5, R7–R13, R16, R18–R21), R6, R14, R15 and R17 rejected; Step B1 placeholder replaced by Steps B1–B16; rejected inventories kept as the decision record; Step C2 rule edit made unconditional |
| 2026-11-21 | Reviewer round 3: receipt-less uncertain preview event (`input_id` null, run plus `message_id` dedup, live-only publication); FIFO-stop rule after an uncertain prompt or exec on every run replaces the held composer; sweep starts only on an empty, unstopped FIFO; stall event kept for queued prompt and exec heads; `submit_unconfirmed` CHECK and `message_id` removed in Step B2; pane-input runner step moved after Hermes settlement (now Step B15); D2 names both Workstream A test changes |
| 2026-11-21 | Reviewer round 4: R4 user-visible consequence states that an uncertain prompt or exec holds back previews and queued input until `input reconcile` |
| 2026-11-21 | Approved by the user |
