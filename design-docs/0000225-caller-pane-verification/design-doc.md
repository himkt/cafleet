# Caller pane verification for `fleet create` and `doctor`

**Status**: Approved
**Progress**: 17/70 tasks complete
**Last Updated**: 2026-11-11

## Overview

`cafleet fleet create` and `cafleet doctor` verify the pane named by `TMUX_PANE` / `HERDR_PANE_ID` against the caller's process ancestry before trusting it. When they cannot verify it, they refuse and name a command the user runs in the Director pane to report its pane ID and server. The Director relays that command and reruns `fleet create --pane <pane-id> --mux-server <server>` with the reported values.

## Success Criteria

- [ ] `fleet create` without `--pane` refuses a caller with a shared-server ancestor before any multiplexer command runs. It registers the env pane only when the env pane's process is an ancestor of the caller; every other case exits 1 before any write with one of the exact errors in § Refusal messages.
- [ ] `fleet create --pane <pane-id>` derives the multiplexer from the pane ID form, checks that the pane exists on the chosen server and runs no ancestry check, so a rerun recovers even when the caller environment names another multiplexer.
- [ ] `doctor` reports server availability in the `multiplexer` row and the verification in a new `multiplexer.caller_pane` row. An unverifiable pane is `warn`, so `doctor` exits 0 when no other check is `error` (issue #450 no longer fails `doctor`).
- [ ] Every case in § Evidence fixtures, plus the cross-multiplexer case, runs as an in-file unit test through an injected `ProcessTable` and a fake multiplexer. The Linux `/proc` parsers and the `ps` line parser each have their own pure-function tests, compiled and run on every platform.
- [ ] `HERMES_SESSION_ID` and its refusal string are gone from code, `SPEC.md`, `docs/` and skills; Hermes, Codex and every other backend share one Director pane identification procedure in the skills and docs.
- [ ] The live acceptance checklist passes on tmux and on herdr, including both cross-multiplexer cases (§ Live acceptance protocol).
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

`fleet create` takes the Director's pane from `--pane`, else from `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr), and refuses only when `HERMES_SESSION_ID` is set without `--pane`. `doctor`'s `multiplexer` row resolves the same env pane through context discovery and reports any failure as `error`. A coding agent whose terminal tools run in a long-lived shared server passes that server's environment to `cafleet`. That environment can belong to another pane, and even to another multiplexer, because one Codex daemon serves every pane under a given `CODEX_HOME`.

Evidence gathered live in the Director's session:

1. **Issue #450** (https://github.com/himkt/cafleet/issues/450): Codex's shared app-server (`--managed-daemon`) kept another pane's `HERDR_PANE_ID`, so `cafleet doctor` failed with `pane_not_found`.
2. **Detector prototype.** Two shell prototypes, one for tmux and one for herdr, walked the caller's ancestry with `ps` and applied two checks. Check 1 refuses when an ancestor is a known shared server (`codex ... app-server`, `hermes ... gateway run`). Check 2 refuses unless the env pane's process is an ancestor: tmux `#{pane_pid}` (`tmux display-message -p -t "$TMUX_PANE"`) or herdr `shell_pid` (`herdr pane process-info --pane "$HERDR_PANE_ID"`). They also refuse when the walk stops before PID 1 or when the env pane does not exist. The prototypes and their raw outputs stayed on the Director's host; § Evidence fixtures records the ancestry this design depends on.
3. **Results, identical on tmux and herdr:**

   | Case | Verdict | Deciding check |
   |---|---|---|
   | Codex through the shared daemon, pane A (started the daemon) | REFUSE | 1 |
   | Codex through the shared daemon, pane C, which inherited pane A's variables while pane A's `codex` was an ancestor | REFUSE | 1 (check 2 alone would wrongly accept) |
   | Codex pane C after pane A closed | REFUSE | 1 |
   | Hermes panes A and B, both carrying pane A's variables | REFUSE | 1 (gateway) |
   | Codex `--no-daemon` | ACCEPT | — |
   | Codex `--approve-for-me` (Codex 0.166.0 did not use the running daemon) | ACCEPT | — |
   | Claude Code, pi, OpenCode `--standalone` | ACCEPT | — |
   | OpenCode `opencode serve --service`, the client that started the service | ACCEPT | — |
   | OpenCode `opencode serve --service`, a second client | REFUSE | 2 (safe false refusal; the shared service is not a supported Director launch) |
   | The real Director session (Claude Code in herdr pane `w4:p3N`) | ACCEPT | — |
   | Codex sandbox with a PID namespace | Not verified (bwrap cannot create namespaces on this host) | — |

4. **Codex daemon baseline.** openai/codex#58940 retired `daemon_auto_start`, so Codex 0.167.0 and later start or reuse the shared daemon. A Codex Director therefore refuses often, and the user-run step is its normal path. Codex Director reception (`message listen --codex-thread` through `codex queue`) needs the daemon, so a Codex Director cannot use `--no-daemon`.
5. **Hermes.** A Hermes Director must pass `fleet create --pane`, today read through the Hermes leading-`!` local shell (`! echo $TMUX_PANE`). The gateway signature now refuses a Hermes caller deterministically, so the Hermes special case folds into the general procedure.
6. **PR #590** (https://github.com/himkt/cafleet/pull/590, merged into `main`): Codex members launch with `--no-daemon`, and its behavior is current state. Members never read the caller environment, so only the root Director is affected.

Drafter observations made while preparing this document (herdr 0.9.5, local tmux):

- `herdr pane current --current` returns the focused pane only when `HERDR_PANE_ID` is unset; with it set, the env pane wins. `env -u HERDR_PANE_ID herdr --session default pane current --current` returned the focused Director pane `w4:p3N`. It did so even with `HERDR_SESSION` and `HERDR_SOCKET_PATH` set to another session and a nonexistent socket, so `--session` takes precedence over both.
- `herdr pane process-info --pane <pane>` returns `result.process_info.shell_pid`; an unknown pane exits 1 with error code `pane_not_found`.
- `herdr pane current` and `herdr status server` report no session name; `herdr session list` lists sessions with their status and socket.
- `tmux display-message -p '#{pane_id}'` run outside a client returned the session's active pane and ignored `TMUX_PANE`, so a shell command is not a reliable way to read the tmux pane. The tmux command prompt runs in the attached client's current pane and needs no shell.

---

## Specification

### Scope

| Surface | Behavior after this change |
|---|---|
| `fleet create` without `--pane` | Runs the shared-server check before any multiplexer command, then verifies the env pane (§ Verification algorithm) before reading its context and registering it |
| `fleet create --pane <pane-id>` | The explicit pane is an operator assertion: its ID form selects the multiplexer, and only its existence on the chosen server is checked |
| `doctor` | `multiplexer` reports backend and server availability; `multiplexer.caller_pane` reports the verification |
| Every other command | Unchanged: it targets the recorded Director placement |
| `message listen --codex-thread` | Its Codex-ancestor pin is unchanged and reads processes through the shared process table (§ Process table) |

No opt-out flag exists; `--pane` with `--mux-server` is the single override. Without `--pane`, the tmux socket or herdr session is resolved from the caller environment as today, and only after the shared-server check passes.

### Shared-server signatures

Shared-server signatures are per-backend data on `CodingAgent` in `src/coding_agent/mod.rs`. `SPEC.md` §6.7 owns their contract in a new `#### Shared-server signatures` subsection.

```rust
pub(crate) struct SharedServerSignature {
    /// Final path components that name the server's program token.
    pub(crate) programs: &'static [&'static str],
    /// Tokens that appear, contiguously and in order, after the program token.
    pub(crate) subcommand: &'static [&'static str],
}

impl SharedServerSignature {
    /// `<backend> <subcommand tokens joined by spaces>`, e.g. `codex app-server`.
    pub(crate) fn label(&self, agent: CodingAgent) -> String;
    pub(crate) fn matches(&self, argv: &[String]) -> bool;
}

impl CodingAgent {
    pub(crate) fn shared_server_signatures(self) -> &'static [SharedServerSignature];
}
```

| Backend | `programs` | `subcommand` | Label |
|---|---|---|---|
| `codex` | `codex` | `app-server` | `codex app-server` |
| `hermes` | `hermes`, `hermes_cli.main` | `gateway`, `run` | `hermes gateway run` |
| `claude`, `opencode`, `pi` | — | — | — |

OpenCode has no signature; the ancestry check covers `opencode serve --service`. `matches(argv)` holds when some token `argv[i]`, whose final `/`-separated component equals one of `programs`, is followed somewhere after `i` by the `subcommand` tokens as a contiguous run. Options may sit between the program token and the run. Verification tests every ancestor against the signatures of every backend in registry order (`CodingAgent::all()`), regardless of the declared `--coding-agent`.

### Process table

The shared process reader lives in `src/runtime/process.rs`:

```rust
pub(crate) struct ProcessEntry {
    pub(crate) parent: i64,
    pub(crate) argv: Vec<String>,
}

pub(crate) trait ProcessTable {
    fn own_pid(&self) -> i64;
    /// `None` when the process no longer exists.
    fn entry(&self, pid: i64) -> Result<Option<ProcessEntry>, CafleetError>;
    /// Whether this process runs in a PID namespace nested below the initial one.
    fn nested_pid_namespace(&self) -> Result<bool, CafleetError>;
}

pub(crate) struct SystemProcessTable;
```

| Platform | `entry(pid)` | `nested_pid_namespace()` |
|---|---|---|
| Linux | `/proc/<pid>/stat` field 4 (the second field after the last `)`) is `parent`; `/proc/<pid>/cmdline` split on NUL is `argv`, with a trailing empty segment dropped and an empty `cmdline` giving an empty `argv`. A `NotFound` read of either file is `None` | `/proc/self/status` `NSpid:` lists more than one PID; no `NSpid:` line is `false` |
| Other (macOS) | `ps -o ppid= -o args= -p <pid>` with null stdin: the first whitespace field is `parent` and the remaining fields are `argv`. Output with no non-blank line is `None` | `false` |

The parsing of `stat`, `cmdline`, the `NSpid:` line and a `ps` output line are pure functions compiled on every platform, so each has unit tests on Linux and macOS alike.

| Reader | Error |
|---|---|
| Linux `entry` | `cannot read /proc/<pid>/<file>: <io error>` for a read error other than `NotFound` (`<file>` is `stat` or `cmdline`) |
| Linux `entry` | `malformed /proc/<pid>/stat` when the parent field is absent or not an integer |
| Linux `nested_pid_namespace` | `cannot read /proc/self/status: <io error>` |
| Linux `nested_pid_namespace` | `malformed /proc/self/status NSpid line: <line>` when an `NSpid:` field is not an integer |
| Other `entry` | `cannot run ps: <io error>` |
| Other `entry` | `ps printed no parent for <pid>: <line>` when the first field is absent or not an integer |
| Other `entry` | `ps printed no command for <pid>: <line>` when no field follows the parent |

These are today's `process_parent_and_argv0` strings plus the two `nested_pid_namespace` strings. `src/cli/message.rs` `pin_codex_session` replaces its private `process_parent_and_argv0` with `SystemProcessTable::entry`, uses `argv.first()` as argv[0] (an empty `argv` matches no `codex` ancestor), and propagates every `entry` error unchanged.

### Pane process read

The `Multiplexer` trait gains one operation:

```rust
/// The pid of `pane`'s process on the targeted server, or `None` when the server has no such pane.
fn pane_process(&self, pane: &str) -> Result<Option<i64>, MultiplexerError>;
```

| Backend | argv (after the server selector) | Result |
|---|---|---|
| tmux | `tmux display-message -p -t <pane> "#{pane_id}\|#{pane_pid}"`, split on the first `\|` | A reply whose `pane_id` is empty or differs from `<pane>` is `None`. With a matching `pane_id`, a pid that is not a positive integer fails with `unexpected tmux display-message output: <quoted>` |
| herdr | `herdr pane process-info --pane <pane>` | `result.process_info.shell_pid`; error code `pane_not_found` is `None`; a missing or non-integer field fails with `herdr pane process-info missing 'shell_pid' field` |

### Multiplexer construction cleanup

`context_discovery`, `crate::runtime::resolve_mux` and `crate::multiplexer::resolve_multiplexer` lose their only caller (`doctor`). Everything that existed only for them goes too:

| Item | Fate |
|---|---|
| `Multiplexer::context_discovery` and its tmux and herdr implementations and tests | Deleted |
| `crate::runtime::resolve_mux`, `crate::multiplexer::resolve_multiplexer` | Deleted |
| `BackendCore.env`, `BackendCore::env_var` | Deleted |
| `BackendCore.server: Option<String>` and its "caller-environment multiplexer" doc comment | Becomes `server: String`; `BackendCore::argv` always inserts the selector (`-S <server>` / `--session <server>`) |
| `TmuxMultiplexer::new(runner, env)`, `HerdrMultiplexer::new(runner, env)` and `HerdrMultiplexer::with_server`'s `env` parameter | Deleted; every caller uses `for_server(runner, server)` |
| Tests that build multiplexers with `new` (`tmux.rs`, `herdr.rs`, `mod.rs`, `cli/member.rs`, `cli/fleet.rs`, `runtime/lifecycle.rs`) | Move to `for_server(runner, "<fixture server>")`; their expected argv gain the selector |

The free functions `caller_server`, `default_tmux_socket` and `herdr_session_for_socket` keep running their plain argv through the runner, because they resolve the server before one exists.

### Caller resolution and verification

A new module `src/multiplexer/caller.rs` owns caller resolution for both `fleet create` and `doctor`:

```rust
pub(crate) struct CallerMux {
    pub(crate) backend: MuxBackend,
    pub(crate) server: String,
    pub(crate) mux: Box<dyn Multiplexer>,
}

pub(crate) struct CallerPane {
    pub(crate) variable: &'static str, // "TMUX_PANE" | "HERDR_PANE_ID"
    pub(crate) pane: String,
}

pub(crate) enum WalkEnd {
    Complete,
    Stopped { pid: i64 },
    Unreadable { pid: i64, cause: String }, // cause: the whole `entry` error message
}

pub(crate) struct Ancestry {
    pub(crate) entries: Vec<(i64, Vec<String>)>, // caller first
    pub(crate) end: WalkEnd,
}

pub(crate) enum Refusal {
    SharedServer { label: String, pid: i64 },
    PaneNotFound,
    PaneNotAncestor { pane_pid: i64 },
    WalkStopped { pid: i64 },
    NestedPidNamespace,
    ProcessUnreadable { pid: i64, cause: String },
}

pub(crate) enum Verification {
    Verified { pane_pid: i64 },
    Refused(Refusal),
}

pub(crate) fn walk(table: &dyn ProcessTable) -> Ancestry;
/// Row 1: the nearest ancestor that matches a shared-server signature.
pub(crate) fn shared_server(ancestry: &Ancestry) -> Option<Refusal>;
/// Backend detection, server resolution and availability, without `--pane`.
pub(crate) fn resolve_env_mux(
    env: &HashMap<String, String>,
    runner: Rc<dyn CommandRunner>,
) -> Result<CallerMux, CafleetError>;
/// The backend selected by an explicit `--pane` ID form, its server and availability.
pub(crate) fn resolve_pane_mux(
    pane: &str,
    mux_server: Option<String>,
    env: &HashMap<String, String>,
    runner: Rc<dyn CommandRunner>,
) -> Result<CallerMux, CafleetError>;
/// The non-empty `TMUX_PANE` / `HERDR_PANE_ID` for `backend`, or `None`.
pub(crate) fn env_pane(backend: MuxBackend, env: &HashMap<String, String>) -> Option<CallerPane>;
/// Rows 2–7 for an env pane after `shared_server` returned `None`.
pub(crate) fn verify_pane(
    mux: &dyn Multiplexer,
    table: &dyn ProcessTable,
    ancestry: &Ancestry,
    pane: &str,
) -> Result<Verification, CafleetError>;
/// The user-run command for one multiplexer (§ User-run pane identification).
pub(crate) fn identification_guidance(backend: MuxBackend) -> &'static str;
/// The refusal bodies (§ Refusal messages), without the `cafleet fleet create ` prefix.
pub(crate) fn shared_server_message(refusal: &Refusal) -> String;
pub(crate) fn unverified_message(caller: &CallerPane, refusal: &Refusal, mux: &CallerMux) -> String;
pub(crate) fn unset_message(backend: MuxBackend) -> String;
```

`caller_env` drops `HERMES_SESSION_ID` and reads `TMUX TMUX_PANE HERDR_ENV HERDR_SESSION HERDR_SOCKET_PATH HERDR_PANE_ID`.

#### Verification algorithm

1. **Walk.** Start at `table.own_pid()`. At each `pid`, when `pid == 1` the walk is `Complete`. Otherwise read `table.entry(pid)`:
   - `None` ends the walk as `Stopped { pid }`.
   - An error ends it as `Unreadable { pid, cause }`.
   - `parent == 0` records `(pid, argv)` and ends it as `Stopped { pid }`.
   - Otherwise record `(pid, argv)` and continue at `parent`.

   A walk that records 4096 entries ends as `Stopped` at the last recorded pid.
2. **Row 1**, before backend detection or any multiplexer command (`shared_server`). The walk itself runs no multiplexer command.
3. Backend detection, env pane, server resolution and availability (§ `fleet create` steps 3–5).
4. **Rows 2–7** (`verify_pane`), by the first matching row:

| # | Condition | Result |
|---|---|---|
| 1 | A recorded ancestor's argv matches a shared-server signature (nearest to the caller first) | `Refused(SharedServer { label, pid })` |
| 2 | `mux.pane_process(pane)` is `None` | `Refused(PaneNotFound)` |
| 3 | `table.nested_pid_namespace()` is `true` | `Refused(NestedPidNamespace)` |
| 4 | The pane pid equals a recorded ancestor pid | `Verified { pane_pid }` |
| 5 | The walk ended `Unreadable { pid, cause }` | `Refused(ProcessUnreadable { pid, cause })` |
| 6 | The walk ended `Stopped { pid }` | `Refused(WalkStopped { pid })` |
| 7 | Otherwise (the walk completed) | `Refused(PaneNotAncestor { pane_pid })` |

Row 1 runs first, so pane C in the evidence, whose pane-A ancestor would pass row 4, is refused. A cross-multiplexer caller (a daemon started from a herdr pane serving a Director in a tmux pane) is also refused before its stale `HERDR_ENV` selects a backend or server. Row 3 precedes row 4 because PIDs inside a nested namespace are not comparable with the multiplexer's host PIDs. Rows 3, 5 and 6 are the "unverifiable" causes. These errors propagate as errors, not refusals: a multiplexer command failure in `pane_process`, and an `Err` from `nested_pid_namespace()`.

### `--pane` ID form

An explicit `--pane` selects the multiplexer by its ID form, so the rerun needs no backend flag and ignores the caller environment's `TMUX` / `HERDR_ENV`:

| `--pane` form | Backend |
|---|---|
| `%` followed by one or more ASCII digits (e.g. `%12`) | tmux |
| Exactly one `:` with non-empty text on both sides and no leading `%` (e.g. `w4:p3N`) | herdr |
| Anything else | Exit 1 before any write: `Error: --pane <pane> is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)` |

With `--pane`, `resolve_pane_mux` picks the server by the first matching row. An ambiguous environment (both `HERDR_ENV` and `TMUX` set) is not an error under `--pane`, because the pane form already names the backend:

| `MuxBackend::detect` on the environment | Derived backend | Server |
|---|---|---|
| Any result | Either | `--mux-server` when given |
| Yields the derived backend | Either | `caller_server(<derived backend>, env)` (today's resolution) |
| Fails as ambiguous, fails as absent, or yields the other backend | tmux | The output of the explicit argv `env -u TMUX tmux display-message -p '#{socket_path}'` (the default server); a failure or empty output gives `no tmux server found; pass --mux-server <socket path>` |
| Fails as ambiguous, fails as absent, or yields the other backend | herdr | `default`, with no command |

The tmux row always uses the `env -u TMUX` argv, because `CommandRunner` passes the process environment to every child. Only the ambiguous case carries a set `TMUX`, but the argv does not depend on that.

### User-run pane identification

The user, not the agent, runs the identification command in the Director pane, and the Director relays it verbatim. Each command reports both the pane ID and the server, because the caller environment's server is as untrusted as its pane. `identification_guidance` renders:

| Multiplexer | Guidance text | Reply |
|---|---|---|
| tmux | `the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket)` | Pane ID (e.g. `%12`) and socket path (e.g. `/tmp/tmux-1000/default`) |
| herdr | `! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)` | Pane ID (e.g. `w4:p3N`) and session name |

The tmux path needs no shell; the command prompt evaluates both formats in the client's current pane. The herdr path asks the named session's server for the focused pane, which is the pane the user is typing in. It unsets `HERDR_PANE_ID` because a set value overrides `--current`, and its explicit `--session` overrides any stale `HERDR_SESSION` / `HERDR_SOCKET_PATH`. Its leading `!` runs wherever the backend runs user shell commands, and the answer comes from herdr's focus state, not from that shell's environment. The user substitutes `<session>`; CAFleet never fills it from the untrusted environment.

### Refusal messages

All three bodies end with the same tail: `then run cafleet fleet create with --pane <pane-id> --mux-server <server>`.

| Function | Body |
|---|---|
| `shared_server_message` | `cannot verify this session's pane: a shared <label> server (pid <pid>) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with <tmux guidance> in tmux, or with <herdr guidance> in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |
| `unverified_message` | `cannot verify that <VAR>=<pane> is this session's pane: <cause>; ask the user to read the Director pane ID and server with <guidance for the detected backend>, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |
| `unset_message` | `cannot identify its <backend> pane because <VAR> is not set; ask the user to read the Director pane ID and server with <guidance for the detected backend>, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |

`shared_server_message` names both multiplexers because a shared server's environment can name the wrong one; the other bodies follow a shared-server-free ancestry, so the detected backend is the caller's own.

| Refusal | `<cause>` in `unverified_message` |
|---|---|
| `PaneNotFound` | `the pane does not exist on the <backend> server <server>` |
| `PaneNotAncestor` | `the pane's process <pane_pid> is not an ancestor of this command` |
| `WalkStopped` | `the process ancestry stopped at pid <pid> before reaching pid 1` |
| `NestedPidNamespace` | `this command runs in a nested PID namespace` |
| `ProcessUnreadable` | `the process ancestry could not be read: <cause>` (e.g. `… could not be read: cannot read /proc/412/stat: Permission denied (os error 13)`) |

Example (Codex Director, pane C):

```
Error: cafleet fleet create cannot verify this session's pane: a shared codex app-server server (pid 407596) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket) in tmux, or with ! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>) in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>
```

### `fleet create`

```rust
pub(crate) struct CreateRequest<'a> {
    pub(crate) name: &'a str,
    pub(crate) agent: CodingAgent,
    pub(crate) pane: Option<String>,
    pub(crate) mux_server: Option<String>,
}

/// Production entry: reads `caller_env`, then delegates with the system runner and table.
fn create(conn: &mut Connection, request: CreateRequest<'_>, json: bool) -> Result<(), CafleetError>;

/// Every external read is injected; tests call this directly.
pub(crate) fn create_with(
    conn: &mut Connection,
    request: CreateRequest<'_>,
    env: &HashMap<String, String>,
    runner: Rc<dyn CommandRunner>,
    table: &dyn ProcessTable,
) -> Result<CreatedFleet, CafleetError>;
```

`run` builds the `CreateRequest` and calls `create`. `create` calls `crate::multiplexer::caller_env()` (a non-UTF-8 variable fails with its name, as today), then calls `create_with(conn, request, &env, Rc::new(SystemRunner), &SystemProcessTable)` and emits the result. `create_with`:

1. With `--pane`: `resolve_pane_mux` (pane-form error, server and availability), then step 7.
2. `walk(table)`; `shared_server` → `Error: cafleet fleet create <shared_server_message>`.
3. `MuxBackend::detect`; failure → `cafleet fleet create must be run inside a tmux or herdr session` (unchanged).
4. `env_pane` is `None` → `Error: cafleet fleet create <unset_message>`. This replaces the herdr `HERDR_PANE_ID is not set` error and the detection-failure message for a tmux caller without `TMUX_PANE`.
5. `resolve_env_mux`; its server and availability failures keep their current messages.
6. `verify_pane`; `Refused(r)` → `Error: cafleet fleet create <unverified_message>`.
7. `pane_context` and registration as today, with the existing `pane <pane-id> was not found on the <backend> server <server>; …` error.

Every failure exits 1 before any write. The `HERMES_SESSION_ID` refusal and its string `cafleet fleet create cannot identify its pane inside a Hermes session; pass --pane <pane-id>` are deleted.

Help text, owned by `SPEC.md` §6.3 `fleet create`:

| Option | Help |
|---|---|
| `--pane` | `The Director's pane; its tmux %N or herdr <workspace>:<pane> form selects the multiplexer [default: the caller's TMUX_PANE or HERDR_PANE_ID, verified against the caller's process ancestry]` |
| `--mux-server` | `The Director's tmux socket path or herdr session [default: the caller's server on the pane's multiplexer]` |

### `doctor`

`doctor(settings, args)` calls `doctor_with(settings, args, &SystemProcessTable)`. `doctor_with` builds `DoctorInputs` with a `caller: &dyn Fn() -> CallerReading` closure over that table, replacing `mux`:

```rust
pub(crate) enum CallerReading {
    /// Row 1 refused before any multiplexer command.
    SharedServer { refusal: Refusal },
    Resolved {
        mux: Result<(&'static str, String), CafleetError>, // (backend name, server)
        pane: Option<Result<PaneReading, CafleetError>>,   // None when mux failed
    },
}

pub(crate) enum PaneReading {
    Verified { variable: &'static str, context: MultiplexerContext },
    Unverified { detail: String }, // unverified_message or unset_message
}
```

`detect_caller(table)` runs `caller_env`, `walk` and `shared_server`, then `MuxBackend::detect`, `env_pane`, `resolve_env_mux` and `verify_pane`. When the pane is verified, it also runs `pane_context` for the ok detail.

| Situation | `multiplexer` | `multiplexer.caller_pane` |
|---|---|---|
| Shared server (row 1) | `info` `not checked: a shared <label> server (pid <pid>) supplies the caller environment` | `warn` `shared_server_message` |
| Detection, server or availability error | `error` with the cause | `info` `unavailable: the multiplexer is unavailable` |
| Env pane unset | `ok` `<backend> server=<server>` | `warn` `unset_message` |
| Rows 2, 3, 5–7 refuse | `ok` `<backend> server=<server>` | `warn` `unverified_message` |
| Verified | `ok` `<backend> server=<server>` | `ok` `<VAR>=<pane_id> session=<session> window=<window_id>` |
| A propagated error: a `pane_process` command failure, a `nested_pid_namespace` error, or a failed context read of a verified pane | `ok` `<backend> server=<server>` | `error` with the error message |

The `multiplexer` summary line folds both rows:

| Rows (`multiplexer`, `multiplexer.caller_pane`) | Summary line |
|---|---|
| `ok`, `ok` | `ok multiplexer: <backend> (session <session>, pane <pane_id>)` (unchanged) |
| `info`, `warn` | `warn multiplexer: <caller_pane detail>` |
| `ok`, `warn` | `warn multiplexer: <backend> (server <server>); <caller_pane detail>` |
| `ok`, `error` | `error multiplexer: <caller_pane detail>` |
| `error`, `info` | `error multiplexer: <cause>` |

JSON adds the `multiplexer.caller_pane` entry right after `multiplexer` in `checks`; the summary shape is unchanged. A `warn` row keeps the exit code at 0 when no other check is `error`.

### Director procedure

The Director role (`assets/skills/cafleet/roles/director.md`) gains a `### Director pane identification` section that Spawn Protocol step 3 links:

1. Run `cafleet fleet create --name <name> --coding-agent <backend> --json` as usual.
2. When it fails with `cannot verify this session's pane`, `cannot verify that … is this session's pane` or `cannot identify its … pane`, ask the user through {decision_surface}. Ask them to run the command the error names in the Director pane themselves: the tmux command when the Director runs in tmux, the herdr command in herdr. Then ask them to reply with the pane ID and the server the command shows. Relay the command text verbatim from the error and run nothing in the Director pane yourself.
3. Rerun `cafleet fleet create --name <name> --coding-agent <backend> --pane <pane-id> --mux-server <server> --json` with both reported values: the tmux socket path, or the herdr session name.
4. When the rerun fails with `pane <pane-id> was not found on the <backend> server <server>` or the `--pane` form error, ask the user once more with the same command, reporting the error. When the second rerun also fails, stop and report its error to the user; make no third attempt.

Spawn Protocol step 2 states that a `warn multiplexer` caller-pane line is resolved by this procedure at step 3. A Hermes Director is always refused (gateway), and a Codex 0.167.0+ Director usually is (daemon); both follow the same steps. Hermes's `! echo $TMUX_PANE` reading is removed everywhere.

### Evidence fixtures

Unit tests encode each case below as a literal `ProcessTable` (own pid, then each `pid(parent): argv` toward PID 1) and a fake multiplexer answering `pane_process`. `<prompt>` and `<script>` stand for the agent prompt and the shell `-c` script argument; matching never reads them, so a fixture uses those literal strings. Tokens marked *re-capture* were not recorded in full. Before writing that case's fixture, the implementer re-captures the full argv from a live session (`tr '\0' ' ' < /proc/<pid>/cmdline`).

tmux cases (scratch server `tmux -L cafleet-probe`; the server process `399808(1): tmux -L cafleet-probe -f /dev/null new-session -d -s probe -x 220 -y 50 -c /tmp/cafleet-env-probe/work` ends every chain):

| Case | Chain from the caller (own pid) upward, before the tmux server | Env pane → pane pid | Expected row |
|---|---|---|---|
| Codex daemon, pane A | `401748(401380): cafleet …`; `401380(400625): /root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex app-server --list`; `400625(399808): codex --enable daemon_auto_start --dangerously-bypass-approvals-and-sandbox <prompt>` | `%3` → 400625 | 1 (`codex app-server`, pid 401380) |
| Codex daemon, pane C (pane A's variables) | Same chain as pane A, own pid 402502 | `%3` → 400625 | 1 (row 4 alone would accept) |
| Codex pane C after pane A closed | `402611(401380): cafleet …`; `401380(1): …/bin/codex app-server --list` (reparented) | `%3` → `None` | 1 |
| Hermes, pane A and pane B | `404037(404033): cafleet …`; `404033(403471): /usr/bin/bash -c <script>`; `403471(403341): /home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/python -m hermes_cli.main --profile default gateway run --quiet --idle-exit`; `403341(399808): /home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/python /home/linuxbrew/.linuxbrew/Cellar/hermes-agent/…` (*re-capture*) | `%11` → 403341 (*re-capture*) | 1 (`hermes gateway run`, pid 403471) |
| Codex `--no-daemon` | `403233(402835): cafleet …`; `402835(399808): codex --no-daemon --dangerously-bypass-approvals-and-sandbox <prompt>` | `%6` → 402835 | 4 |
| Codex `--approve-for-me` | `402101(401813): cafleet …`; `401813(399808): codex --enable daemon_auto_start --approve-for-me <prompt>` | `%4` → 401813 | 4 |
| Claude Code | `403317(403316): cafleet …`; `403316(403012): /bin/bash -c <script>`; `403012(399808): claude --allowedTools "Bash(sh /tmp/cafleet-env-probe/detect.sh *)" -- <prompt>` | `%7` → 403012 | 4 |
| pi | `403373(403148): cafleet …`; `403148(399808): pi <prompt>` | `%8` → 403148 | 4 |
| OpenCode `--standalone` | `403403(403187): cafleet …`; `403187(403176): /root/.local/share/mise/installs/aqua-anomalyco-opencode/v2.2.5/opencode serve --stdio --port 0`; `403176(399808): opencode --standalone --auto --prompt <prompt>` | `%9` → 403176 | 4 |
| OpenCode service, first client | `403443(403264): cafleet …`; `403264(403252): /root/.local/share/mise/installs/aqua-anomalyco-opencode/v2.2.5/opencode serve --service`; `403252(399808): opencode --auto --prompt <prompt>` | `%10` → 403252 | 4 |
| OpenCode service, second client | `405295(403264): cafleet …`, then the first client's chain | `%13` → 405228 | 7 |

herdr cases use the same agent chains with the pane's shell between the agent and the herdr server. For example, Codex daemon pane A is `408076(407596): cafleet …`; `407596(407442): …/bin/codex app-server --list`; `407442(407363): codex --enable daemon_auto_start --dangerously-bypass-approvals-and-sandbox <prompt>`; `407363(3340413): /bin/sh`; `3340413(1): /root/.local/share/mise/installs/aqua-ogulcancelik-herdr/v0.9.5/herdr server`, with `w5:p1` → 407363. The herdr server's parent (`1`) is *re-capture*. The real Director session is `410391(410390): cafleet …`; `410390(363045): /bin/bash -c <script>`; `363045(362690): claude`; `362690(362679): zsh`; `362679(3340413): /bin/sh`, then the herdr server, with `w4:p3N` → 362679 (row 4).

Synthetic cases (no live capture needed):

| Case | Fixture | Expected |
|---|---|---|
| Cross-multiplexer | Env `HERDR_ENV=1`, `HERDR_SESSION=default`, `HERDR_PANE_ID=w5:p1`, no `TMUX`; the chain of herdr pane A's daemon | Row 1 refusal with both guidances and zero runner calls; the rerun with `--pane %5 --mux-server /tmp/tmux-0/default` derives tmux, issues only `tmux -S /tmp/tmux-0/default …` argv and records backend `tmux` |
| Nested PID namespace | `nested_pid_namespace()` returns `true`; any chain | Row 3 |
| Walk stops | An entry returns `None`, or a chain reaches `parent == 0` before PID 1 | Row 6 |
| Unreadable | An entry returns `cannot read /proc/412/stat: Permission denied (os error 13)` | Row 5 with that cause |
| Namespace read failure | `nested_pid_namespace()` returns `Err` | Error propagated (no refusal) |

### Live acceptance protocol

The execute workflow's Verifier drives the cases with the branch-built binary `<repo>/target/release/cafleet`, written below as `<cafleet>` and always invoked by that absolute path. The operator's installed `cafleet` binary, skills, presets, plugins, extensions and executable records stay untouched:

- **Scratch setup.** The Verifier runs `HOME=<BASE>/.acceptance/home CLAUDE_CONFIG_DIR=<BASE>/.acceptance/home/.claude CODEX_HOME=<BASE>/.acceptance/home/.codex OPENCODE_CONFIG_DIR=<BASE>/.acceptance/home/.config/opencode HERMES_HOME=<BASE>/.acceptance/home/.hermes PI_CODING_AGENT_DIR=<BASE>/.acceptance/home/.pi/agent <cafleet> setup --database-url sqlite:///<BASE>/.acceptance/accept.db`. This covers every backend config-location variable. Every asset it installs and every `hermes` call it makes lands under `<BASE>/.acceptance/home`, and the only acceptance state outside that directory is the scratch database.
- **Case commands.** Every acceptance command is `<cafleet> … --database-url sqlite:///<BASE>/.acceptance/accept.db`, run with the agent's real `HOME` and configuration so the scratch agents keep their credentials. `doctor`'s backend rows compare the operator's installed assets with the branch binary's embedded copies and may report `warn` or `error`. A case therefore asserts only the `multiplexer` and `multiplexer.caller_pane` rows: their statuses, their details and the folded `multiplexer` summary line. It asserts no `doctor` exit code.
- **Scratch servers.** Each case runs on a scratch server (`tmux -L cafleet-accept`, or a scratch herdr session). The Verifier starts the case's agent in a scratch pane with a prompt that runs exactly `<cafleet> doctor --json --database-url …` and then `<cafleet> fleet create --name accept-<case> --coding-agent <agent> --json --database-url …`, and captures the pane output.
- **Clean start.** Before each multiplexer's matrix and before each cross-multiplexer setup, `ps -eo pid,args` lists no `codex app-server` process and no `hermes_cli.main … gateway run` process (defunct `[hermes]` entries excluded). The Verifier ends the owning sessions and terminates any leftover pid with `kill <pid>`, then repeats the listing.
- **User-run steps.** The Verifier sends the Director the refusal text. The Director relays the named command to the operator through its decision surface, and the operator performs it in the scratch Director pane and reports the pane ID and server. The Director records the reply verbatim in the case's evidence file and sends the values to the Verifier, which has the scratch agent rerun `<cafleet> fleet create … --pane <pane-id> --mux-server <server> --json --database-url …`. No member reads the pane ID out of band.
- **Evidence.** Each case writes `<BASE>/.acceptance/<multiplexer>/<case>.txt` with the captured `doctor` and `fleet create` output, the operator's recorded reply, and `<cafleet> fleet show <id> --json --database-url …` showing the recorded backend, server and pane. The Verifier then deletes the fleet with `<cafleet> fleet delete <id> --database-url …`. A checkbox is checked only when its evidence file shows the expected refusal row, the expected `multiplexer` and `multiplexer.caller_pane` rows, and the true pane in `fleet show`.

### Documentation changes

The order follows `.claude/rules/documentation-maintenance.md`: `SPEC.md`, `docs/`, `README.md`, skills, then rules.

| Target | Change |
|---|---|
| `SPEC.md` §6.3 `fleet create` | Director pane row (`--pane` trusted, its form selects the backend; env pane verified), a `##### Caller pane verification` subsection (algorithm, `--pane` form, guidance and refusal tables), the `--pane` / `--mux-server` help text, the new error rows, and the removed Hermes row and paragraph; "doctor's `multiplexer` and `multiplexer.caller_pane` rows" wording |
| `SPEC.md` §6.3 `cafleet doctor` | Check-rows table (`multiplexer` detail and the new `multiplexer.caller_pane` row) and the summary folding table |
| `SPEC.md` §6.3 Error Messages | The `fleet create` row lists a shared-server caller, an unset or unverifiable env pane and a malformed `--pane` instead of the Hermes session |
| `SPEC.md` §6.5 | Backend selection names both doctor rows, says that `--pane` selects the backend by form, and links §6.3 Caller pane verification; Backend operations gains the `Pane process` row, and the pane context row covers doctor |
| `SPEC.md` §6.7 | New `#### Shared-server signatures` subsection; Interface lists `shared_server_signatures`; Host evidence **codex** says that 0.167.0+ starts or reuses the daemon (openai/codex#58940), so a Director's `fleet create` usually refuses; **hermes** states the gateway refusal |
| `SPEC.md` §6.8, §9 | §6.8 names both doctor rows; §9's covered-behavior list adds caller-pane verification |
| `docs/docs/concepts/members.md` | New `### Director pane verification` subsection under Registration and readiness: the owning page for the user-run command table, stated as user-visible behavior; the Hermes sentence becomes a link to it |
| `docs/docs/quickstart.md` | Hermes steps 3–4 become "register; when CAFleet asks for the pane ID and server, run the command it names", with a link; the Codex section says a Codex Director usually needs this step |
| `docs/docs/mixed-backend-team.md`, `docs/docs/concepts/coding-agents.md` (Hermes configuration), `docs/docs/contributing.md` | The Hermes `--pane` / leading-`!` sentences become a link to Director pane verification |
| `README.md` | No change; confirm it carries no fleet-create pane wording |
| `assets/skills/cafleet/reference/cli.md` § Fleet commands | Opening sentence (cli.md:75, "It and doctor's `multiplexer` row are the only readers…") names both doctor rows and the shared-server check; the input table's Director pane row adds the `--pane` form rule; the error table gains the new rows and loses the Hermes row; the Hermes paragraph becomes the guidance table |
| `assets/skills/cafleet/reference/cli.md` § `setup` and `doctor` | The `multiplexer.caller_pane` row and the summary folding |
| `assets/skills/cafleet/reference/cli.md` exit-code table | The `fleet create` row (cli.md:489, "No server, the pane is not on it, or no identifiable pane") also covers an unverifiable caller pane and a malformed `--pane` |
| `assets/skills/cafleet/roles/director.md` | Spawn Protocol steps 2–3 and the new `### Director pane identification` section |
| `assets/skills/cafleet/reference/coding-agents.md` | The Hermes Note row and reception step 2 follow Director pane identification; a new Codex Note row (0.167.0+ daemon, refusal expected, a Director cannot use `--no-daemon`) is bound to Director launch and Director pane identification |
| `.claude/rules/` | No change |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Prerequisite

- [x] Rebase the implementation branch on `main`, which includes the merged PR #590 (Codex members launch with `--no-daemon`) <!-- completed: 2026-11-11T11:36 -->

### Step 2: SPEC.md

- [x] §6.3 `fleet create`: rewrite the Director pane row and add `##### Caller pane verification` (algorithm, `--pane` form, guidance and refusal tables). Add the `--pane` / `--mux-server` help text, update the error table, and delete the Hermes row and paragraph <!-- completed: 2026-11-11T11:40 -->
- [x] §6.3 `cafleet doctor`: update the check-rows table and add the summary folding rows for `multiplexer.caller_pane` <!-- completed: 2026-11-11T11:40 -->
- [x] §6.3 Error Messages: rewrite the `fleet create` situation <!-- completed: 2026-11-11T11:40 -->
- [x] §6.5: update Backend selection, add the `Pane process` Backend operations row and extend the pane context row to doctor <!-- completed: 2026-11-11T11:40 -->
- [x] §6.7: add `#### Shared-server signatures`, extend Interface, and update the codex and hermes Host evidence paragraphs <!-- completed: 2026-11-11T11:40 -->
- [x] §6.8 and §9: update the caller-environment sentence and the covered-behavior list <!-- completed: 2026-11-11T11:40 -->

### Step 3: docs/

- [x] `concepts/members.md`: add `### Director pane verification` with the per-multiplexer user-run table and replace the Hermes sentence with a link <!-- completed: 2026-11-11T11:42 -->
- [x] `quickstart.md`: rewrite Hermes steps 3–4 and add the Codex Director note <!-- completed: 2026-11-11T11:42 -->
- [x] `mixed-backend-team.md`, `concepts/coding-agents.md`, `contributing.md`: replace the Hermes `--pane` / leading-`!` wording with the link <!-- completed: 2026-11-11T11:42 -->
- [x] Inspect the rendered HTML of every changed table cell that contains a literal `|` or a `#{…}` format <!-- completed: 2026-11-11T11:42 -->

### Step 4: README.md

- [x] Confirm `README.md` needs no change (no fleet-create pane wording) <!-- completed: 2026-11-11T11:42 -->

### Step 5: Skills

- [x] `reference/cli.md` § Fleet commands: update the opening sentence, the Director pane input row (form rule), the error table (new rows, removed Hermes row) and the guidance table that replaces the Hermes paragraph <!-- completed: 2026-11-11T11:44 -->
- [x] `reference/cli.md` § `setup` and `doctor` and the exit-code table: add the `multiplexer.caller_pane` row and summary folding, and widen the `fleet create` exit row <!-- completed: 2026-11-11T11:44 -->
- [x] `roles/director.md`: update Spawn Protocol steps 2–3 and add `### Director pane identification` <!-- completed: 2026-11-11T11:44 -->
- [x] `reference/coding-agents.md`: update the Hermes Note row and reception step 2, and add the Codex Note row <!-- completed: 2026-11-11T11:44 -->
- [x] Grep `SPEC.md`, `docs/` and `assets/skills/` for `HERMES_SESSION_ID`, `inside a Hermes session` and `echo $TMUX_PANE`; none remain <!-- completed: 2026-11-11T11:44 -->

### Step 6: Process table

- [ ] Add `ProcessEntry`, `ProcessTable` and `SystemProcessTable` (Linux `/proc`, non-Linux `ps`) to `src/runtime/process.rs`, with the `stat`, `cmdline`, `NSpid:` and `ps`-line parsers as pure functions compiled on every platform <!-- completed: -->
- [ ] Unit-test the `stat` and `cmdline` parsers: a parent after a `)` inside the command name, a missing or non-integer parent (`malformed /proc/<pid>/stat`), a NUL-split argv with a trailing NUL, and an empty `cmdline` <!-- completed: -->
- [ ] Unit-test the `NSpid:` parser: single, nested, absent and non-integer (`malformed /proc/self/status NSpid line: <line>`) lines <!-- completed: -->
- [ ] Unit-test the `ps` line parser: parent and whitespace argv, blank output (`None`), no parent (`ps printed no parent for <pid>: <line>`) and no command (`ps printed no command for <pid>: <line>`) <!-- completed: -->
- [ ] Unit-test `SystemProcessTable::entry` for its own pid (parent equals `getppid`, argv non-empty) and for a reaped child's pid (`None`) <!-- completed: -->
- [ ] Add a `#[cfg(test)] pub(crate) mod test_support` `FixtureProcessTable` (own pid, entries, per-pid errors, nested flag, namespace error) beside `ProcessTable` <!-- completed: -->
- [ ] Replace `process_parent_and_argv0` in `src/cli/message.rs` with `SystemProcessTable`; the existing Codex-pin tests pass unchanged <!-- completed: -->

### Step 7: Shared-server signatures

- [ ] Add `SharedServerSignature` and `CodingAgent::shared_server_signatures` in `src/coding_agent/mod.rs` <!-- completed: -->
- [ ] Unit-test `matches`: the Codex daemon and Hermes gateway argv from § Evidence fixtures match. `codex --no-daemon …`, `codex --enable daemon_auto_start …`, `hermes chat --tui`, `hermes gateway status`, `opencode serve --service`, `claude` and an empty argv do not <!-- completed: -->
- [ ] Unit-test `label` for both signatures <!-- completed: -->

### Step 8: Pane process read and multiplexer cleanup

- [ ] Add `pane_process` to the `Multiplexer` trait with tmux and herdr implementations <!-- completed: -->
- [ ] tmux unit tests (FakeRunner): the exact argv with `-S <server>`, a found pid, a `|` reply (`None`), another pane ID (`None`) and a non-numeric pid (error) <!-- completed: -->
- [ ] herdr unit tests: the exact argv with `--session <server>`, the `shell_pid` read, `pane_not_found` (`None`) and a missing field (error) <!-- completed: -->
- [ ] Delete `context_discovery` with its implementations and tests, `resolve_mux`, `resolve_multiplexer`, `BackendCore.env`, `BackendCore::env_var` and the `new(runner, env)` constructors. Make `BackendCore.server` a `String`, and move every test to `for_server` with the selector in its expected argv <!-- completed: -->

### Step 9: Caller verification module

- [ ] Add `src/multiplexer/caller.rs` with the types and functions in § Caller resolution and verification <!-- completed: -->
- [ ] Drop `HERMES_SESSION_ID` from `caller_env` <!-- completed: -->
- [ ] Unit-test `walk` termination: PID 1 reached, a `None` entry, `parent == 0`, a read error carrying the whole message, and the 4096-entry cap <!-- completed: -->
- [ ] Unit-test each decision-table row with `FixtureProcessTable` and a fake `Multiplexer`, including a propagated `pane_process` error and a propagated `nested_pid_namespace` error <!-- completed: -->
- [ ] Unit-test every tmux and herdr case in § Evidence fixtures, after re-capturing each *re-capture* token <!-- completed: -->
- [ ] Unit-test the `--pane` form rule: `%12` (tmux), `w4:p3N` (herdr), and `12`, `%`, `%1a`, `a:b:c`, `:p1`, `w4:` and `%1:2` (form error) <!-- completed: -->
- [ ] Unit-test `resolve_pane_mux` server selection for every row of its table: `--mux-server` given; env backend equal to the derived backend; an ambiguous environment (`HERDR_ENV` and `TMUX` set) with `%5`, asserting the runner receives exactly `env -u TMUX tmux display-message -p #{socket_path}`; the ambiguous environment with `w4:p1` (`default`, zero runner calls); a herdr environment with `%5` and an empty environment with `%5` (the same `env -u TMUX` argv); and an empty or failed reply (`no tmux server found; pass --mux-server <socket path>`) <!-- completed: -->
- [ ] Unit-test the exact `shared_server_message`, the `unverified_message` for every cause on both multiplexers, and `unset_message` on both <!-- completed: -->

### Step 10: `fleet create`

- [ ] Add `CreateRequest`, split `create` (reads `caller_env`, injects the system runner and table) from `create_with`, and implement its seven steps; delete `caller_pane` and the Hermes refusal <!-- completed: -->
- [ ] Unit-test that `--pane` never reads the process table (a table that panics on use), derives the backend, and still reports the pane-not-found error <!-- completed: -->
- [ ] Unit-test the cross-multiplexer case from § Evidence fixtures end to end through `create_with`: the refusal with zero runner calls, then the `--pane %5 --mux-server …` rerun registering backend `tmux` <!-- completed: -->
- [ ] Unit-test that an unset env pane, a malformed `--pane` and each refusal exit 1 with the exact message and leave no fleet row <!-- completed: -->
- [ ] Unit-test that a verified env pane registers the pane's context <!-- completed: -->
- [ ] Set the `--pane` and `--mux-server` help text from § `fleet create` <!-- completed: -->

### Step 11: `doctor`

- [ ] Add `doctor_with`, `CallerReading`, `PaneReading` and `detect_caller(table)`, and replace `DoctorInputs.mux` with `caller` <!-- completed: -->
- [ ] Render the `multiplexer` and `multiplexer.caller_pane` rows and the folded summary line <!-- completed: -->
- [ ] Unit-test every row of the doctor situation table and each summary-folding case in the summary, verbose and JSON views <!-- completed: -->
- [ ] Make the doctor child-process tests call `doctor_with` with a `FixtureProcessTable` whose own pid is the child's `std::process::id()` (parent 1). Their `tmux` stub answers the pane-process read with `%1|$PPID`; the stub's `$PPID` is the child itself, so the healthy case verifies in every environment, Codex-daemon and nested-namespace hosts included <!-- completed: -->
- [ ] Add a child-process case whose fixture table holds a `codex app-server` ancestor, and assert `info multiplexer` / `warn multiplexer.caller_pane` and exit 0 <!-- completed: -->

### Step 12: Validation

- [ ] Run `mise //:test`; all tests pass <!-- completed: -->
- [ ] Run `mise //:lint`; no findings <!-- completed: -->
- [ ] Run `mise //docs:build`; the build and link check pass <!-- completed: -->
- [ ] Build the branch binary with `mise //:build` and use `<repo>/target/release/cafleet` by absolute path for live acceptance; run no install task, so the operator's installed binary and assets and the active execute fleet's runtime stay unchanged <!-- completed: -->
- [ ] Run the scratch setup from § Live acceptance protocol and confirm that `<BASE>/.acceptance/home` holds the installed assets and that the operator's installed `cafleet` executable (`command -v cafleet`) and every CAFleet asset under the operator's config directories (the `cafleet` skill directories, the OpenCode and Hermes plugin files and the pi extension) have identical `sha256sum` listings before and after <!-- completed: -->

### Step 13: Live acceptance — tmux

Follow § Live acceptance protocol on `tmux -L cafleet-accept`, starting from a clean start check.

- [ ] Codex through the shared daemon, pane A: `fleet create` refuses with the shared-server body (`codex app-server`); `doctor` shows `warn multiplexer` with `warn multiplexer.caller_pane`; the operator's tmux prompt reply and the rerun register pane A <!-- completed: -->
- [ ] Codex through the shared daemon, pane C (pane A's variables): refuses at row 1; the rerun registers pane C <!-- completed: -->
- [ ] Codex pane C after pane A closed: refuses at row 1; the rerun registers pane C <!-- completed: -->
- [ ] Hermes panes A and B: both refuse at row 1 (`hermes gateway run`); the reruns register each pane <!-- completed: -->
- [ ] Codex `--no-daemon`, Codex `--approve-for-me`, Claude Code, pi and OpenCode `--standalone`: each registers its own pane without `--pane`, and `doctor` shows `ok multiplexer` <!-- completed: -->
- [ ] OpenCode `opencode serve --service`: the first client registers its pane; the second client refuses at row 7 <!-- completed: -->
- [ ] Cross-multiplexer: with the Codex daemon started from a herdr pane, a Codex Director in a tmux pane is refused at row 1 with both guidances. The operator's tmux prompt reply and the rerun register backend `tmux` with the reported socket <!-- completed: -->
- [ ] Record the Codex PID-namespace sandbox as not verified on this host (bwrap cannot create namespaces) <!-- completed: -->

### Step 14: Live acceptance — herdr

Repeat Step 13 in a scratch herdr session after a clean start check. Refused cases use the herdr guidance, which the operator types into the scratch Director's composer.

- [ ] Codex daemon panes A, C and C-after-A-closed: each refuses at row 1, and each rerun registers the true pane. `doctor` from pane C after pane A closed shows `warn multiplexer` with `warn multiplexer.caller_pane` instead of an `error` row (issue #450) <!-- completed: -->
- [ ] Hermes panes A and B: both refuse at row 1; the reruns register each pane <!-- completed: -->
- [ ] Codex `--no-daemon`, Codex `--approve-for-me`, Claude Code, pi and OpenCode `--standalone`: each registers its own pane without `--pane` <!-- completed: -->
- [ ] OpenCode `opencode serve --service`: the first client accepts and the second refuses at row 7 <!-- completed: -->
- [ ] Cross-multiplexer: with the Codex daemon started from a tmux pane, a Codex Director in a herdr pane is refused at row 1. The operator's herdr reply and the rerun register backend `herdr` with the reported session <!-- completed: -->
- [ ] The real Director session (Claude Code in a herdr pane): `doctor` shows `ok multiplexer` with the verified pane <!-- completed: -->
- [ ] Record the Codex PID-namespace sandbox as not verified on this host <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-11 | Initial draft |
| 2026-11-11 | Review round 1: shared-server check before backend/server resolution; guidance reports the server; `--pane` form selects the backend; reader error contract; multiplexer construction cleanup; `create_with` signature; evidence fixtures recorded in the document; injected doctor child tests; live acceptance protocol |
| 2026-11-11 | Review round 2: explicit `--pane` server selection for an ambiguous environment (`env -u TMUX` argv); live acceptance isolated to the branch-built binary and a scratch HOME; Step 12 builds instead of installing |
| 2026-11-11 | Approved by the user: the shared-server error text stays as specified, the implementation ships as one PR including the removals, and live acceptance runs the full matrix with the user-run step for every refused case |
| 2026-11-11 | PR #590 merged into `main`; Background item 6 and Step 1 reflect it |
