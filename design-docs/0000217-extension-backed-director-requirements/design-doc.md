# Extension-backed Director requirements

**Status**: Approved
**Progress**: 47/57 tasks complete
**Last Updated**: 2026-11-06

## Overview

Turn the Director requirements in SPEC.md §6.7 into one general acceptance criterion for every coding-agent backend. The criterion evaluates goal lifecycle and listen (reception) independently and admits CAFleet-vendored extensions that `cafleet setup` installs, ranking listen as native subscribe > extension subscribe >> built-in compromise and goal as native > extension > no. Under this criterion, Codex moves to native subscribe through `codex queue`, Hermes moves to a plugin subscribe receiver, and OpenCode becomes Director-eligible through a vendored goal and receiver plugin.

## Success Criteria

- [ ] SPEC.md §6.7 Role requirements states:
  - the two independent Director requirements
  - the three listen tiers in order, with the gap between tiers 2 and 3
  - the subscribe probe
  - the tier-3 negative-evidence rule
  - the goal order and the extension-goal minimum bar
- [ ] SPEC.md § Role eligibility and every backend's Role eligibility table in `skills/cafleet/reference/coding-agents.md` record, per requirement and backend, the source (`native`/`extension`), the delivery style (`subscribe`/`compromise`) and the evidence, plus one Director verdict row.
- [ ] `cafleet setup` installs every embedded extension of the selected agent and performs the Hermes plugin enablement itself. `cafleet doctor` verifies each installed extension and the Hermes enablement.
- [ ] `cafleet message listen --codex-thread THREAD` forwards each record through `codex queue` and exits when its Codex session process ends.
- [ ] The CAFleet plugin tools refuse to run in member sessions, and the OpenCode member presets deny them.
- [ ] Live acceptance runs of the shipped mechanisms pass on the installed versions, and their observed results are recorded as evidence. The versions are Claude 2.1.309, Codex 0.165.0, Hermes 2026.9.24 and OpenCode 2.2.0. No CAFleet listener survives its owning session in any run.
- [ ] Every mention of the Hermes `heartbeat=60` / `process_manage` reception recipe and the Codex `write_stdin` read loop is removed from skills, docs and the SPEC backend sections. SPEC keeps tier 3 only as a general acceptance tier.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Today a Director needs "native goal lifecycle controls" and "streamed background output while supervision continues", established by a CAFleet stream probe. That probe also accepts a model-driven read of a running process, so it does not distinguish push delivery from polling.

| Backend | Goal | Reception today | Director |
|---|---|---|---|
| `claude` | native `/goal` | Monitor pushes each stdout line | `yes` |
| `codex` | native `/goal` | model reads a retained `exec_command` session through `write_stdin` | `yes` |
| `opencode` | none in 2.1.4 | `shell` `background=true` delivers only at exit | `no` |
| `hermes` | native `/goal` | `terminal` `background=true` + `process_manage` reads + `heartbeat=60` batches | `yes` |

The user wants Director eligibility to be "more native", and push delivery is the property that matters most. CAFleet may vendor backend extensions and plugins when a backend has no native push.

### Design-time verification (2026-11-05)

The claims below were checked against the installed binaries and their source. The live probes ran through Director-spawned probe members that observed the Director inbox read-only. Probe artifacts stay local under the task folder's `.drafter/probe/`.

| Probe | Backend / version | Mechanism | Result |
|---|---|---|---|
| C1 | Codex 0.165.0 (the member binary; its queue, exec-env and `/goal` sources are identical to 0.164.0) | background bridge <code>cafleet message listen 4 &#124; while read …; codex queue --thread "$CODEX_THREAD_ID" --message …</code> | Idle: each record started its own no-input turn (~2 s; once ~8.8 s through the 10 s cross-process watcher). Busy: `sleep 120` ran to exit 0; the two records queued during it started one turn each, 4 ms after it completed. A burst of 3 gave three ordered turns. |
| C2 | Codex 0.165.0 | same bridge under an active native `/goal` | Queued records ran as separate turns at goal-turn boundaries. The goal resumed and completed without input. |
| H1 | Hermes 2026.9.24, classic CLI (`hermes chat --cli`) | probe plugin whose tool spawns `cafleet message listen` in the gateway; `ctx.inject_message(text, session_key=…)` per line | Every record became its own model turn (session export). Records injected during the busy startup turn ran 0.11 s after it ended, without interrupting it. Idle wake took ~0.3 s. The classic CLI pane rendered none of these turns. |
| H2 | Hermes 2026.9.24, TUI (`hermes chat --tui`) | same plugin | The TUI rendered each injected turn. During a `sleep 60` the records waited in a `queued (N)` panel without interrupting it, then drained one per turn in order. |
| O1 | OpenCode 2.2.0, `mini --standalone` | probe plugin whose tool spawns `cafleet message listen`; `ctx.session.synthetic({sessionID, text, delivery: "queue"})` per line | Idle wake gave a no-input turn per record. Records sent during a 120 s foreground command waited until it finished, then ran right after it. Burst order was kept. |
| O2 | OpenCode 2.2.0, `mini --standalone` | probe goal plugin: command, tools, storage, instruction entry, continuation on `session.execution.succeeded` | Set + automatic continuation ran 5 turns, then completed with no further continuation. Status, pause/resume, compaction (objective retained and restated), restart on the same session and clear worked. |

The probes produced findings the design must handle:

| Finding | Observed | Design response |
|---|---|---|
| Hermes premise correction | `plugins_injection.py` routes to `_pending_input`/`_interrupt_queue` only when `PluginManager._cli_ref` is set, and 2026.9.24 never sets it. Plugins run in the per-home gateway, where a keyed injection is admitted into the session's durable FIFO (`gateway/run_plugin_injection.py:233-245`, `:363-392`). | No plugin-side busy buffer and no `pre_llm_call` flush (§4.1) |
| Hermes injection order (A-H, 2026-11-06) | `inject_message` returns before its dispatch coroutine commits, so back-to-back records reordered (994→995→993, 996→998→997). | Ordered injection: await each dispatch before the next record (§4.1) |
| Hermes key-less injection | `gateway/plugin_keyless_inject.py:92-98` refuses when several classic CLI sessions share a home, and the Director and Hermes members do. | The tool captures the calling session key and injects with `session_key=` (§4.1) |
| Hermes enablement | `hermes plugins enable` fails on the Homebrew install (its PM venv sync needs `site-packages/uv.lock`). Only `plugins.enabled`/`plugins.disabled` gate loading (`plugins_discovery.py:112-131`, `:257-302`). | Setup merges the keys through `hermes config set` (§2.3) |
| Hermes classic CLI visibility | Gateway turns injected into a classic CLI session run without rendering in that pane. | A Hermes Director runs the TUI client; members keep `--cli` (§4.3) |
| Listener outlives its session | Codex: `member delete` left the bridge running. Hermes: the gateway-owned listener survived `member delete` (classic) and `/exit` (TUI). OpenCode: plugin cleanup killed the child at member delete. | Each receiver binds its lifetime to its owning session (§3.2, §4.2, §5.1) |
| Codex sandbox | On this host bwrap cannot create namespaces, so the bridge and its `codex queue` children needed the escalated path (automatic review approved it). | The Codex recipe states the escalated path (§3.3) |
| OpenCode pause race | A continuation queued 34 ms before `pause` still ran once. | Generation-tagged continuations, dropped by a `context` hook (§5.2) |
| OpenCode no-progress loop | After restart, the goal's mandated command failed every turn, and continuations ran without progress. | No-progress stop (§5.2) |
| Codex queue cap (A-C, 2026-11-06) | `codex queue` refuses a submission while the thread queue holds 100 (`queue cannot contain more than 100 submissions`); a 200-record burst ended the forwarder at record 97. | Full-queue wait-and-retry with a 600 s limit (§3.1) |

---

## Specification

### 1. Director acceptance criterion (SPEC.md §6.7 Role requirements)

The Director role has two requirements, goal and listen, evaluated independently. Director is `yes` only when both are met. Otherwise it is `no` and records the unmet requirement with "not established" or the observed failure. Member requirements are unchanged.

#### 1.1 Listen requirement and tiers

A *subscribe-style* listen delivers each record that the running `cafleet message listen` emits into the session as model-visible input. The host pushes the input; the model neither polls nor reads the process. Its semantics:

- When the session is idle, delivery starts a new turn.
- When the session is busy, the record is delivered no later than the end of the running turn and never interrupts it. Earlier delivery between tool calls inside the turn also passes.
- No record waits for an unrelated future turn. A record queued behind earlier undelivered records is delivered after their turns; that wait passes, because each of those turns consumes an earlier record.
- Records keep their emission order.

| Tier | Name | Definition | Standing |
|---|---|---|---|
| 1 | Native subscribe | The push facility is part of the backend as installed. CAFleet contributes only CLI invocations: `cafleet message listen` and its forwarding options. | Preferred |
| 2 | Extension subscribe | Subscribe-style delivery through a CAFleet-vendored extension or plugin that `cafleet setup` installs. | Accepted; ranks below tier 1 |
| 3 | Built-in compromise | Reception built from built-in tools, where the model reads the running process's output or receives periodic batches. It must pass the stream probe: each record is first observed in the running listener's output, at the latest at the next turn boundary. | Degraded fallback |

Tiers rank 1 > 2 >> 3, and within one delivery style native outranks extension. Tier 3 sits far below both subscribe tiers. It spends model turns on reads, delays records to read or batch points, and needs heartbeat-style wake and log reconciliation.

A backend's recorded tier is the highest one its evidence establishes. Tier 3 is recorded only together with negative evidence for tiers 1 and 2 on that version:

- **Tier 1 negative evidence:** the installed version's built-in tools and CLI expose no facility that pushes externally produced text into a running session as model-visible input that starts a turn when the session is idle. The check covers built-in tool docs, `--help` of the binary and its subcommands, and the native source or official documentation for that version.
- **Tier 2 negative evidence:** the version's extension surfaces expose no way for an extension to do the same. The surfaces are plugins, hooks, extensions, MCP notifications and any client/server protocol an extension can call. The check uses the native source at that version's tag, or the official extension documentation.
- **Recording:** each source is named with its version, plus `checked YYYY-MM-DD`.

**Subscribe probe.** This probe establishes tier 1 or 2 on an installed version. The candidate starts the receiver through its tier's mechanism, and another member sends broker records:

1. One record while the session is idle: a new turn starts without user input and consumes it.
2. Two records while the session runs a foreground command of at least 60 s: the command is not interrupted, and each record is delivered no later than the end of that turn, without input.
3. Three records back to back: all are delivered, in order.
4. The receiver ends when its owning session ends (§3.2, §4.2, §5.1).

A record discovered through a foreground `poll`, `wait` or `show` invalidates the run. The current SPEC stream probe remains the tier-3 probe.

#### 1.2 Goal requirement and tiers

| Tier | Name | Definition |
|---|---|---|
| 1 | Native | Goal lifecycle controls built into the backend |
| 2 | Extension | A CAFleet-vendored extension that meets the minimum bar below |
| — | No | Neither |

**Extension-goal minimum bar.** Every item is verified live on the installed version:

- **Operations:** set, status, pause, resume, clear and complete.
- **Persistence:** the goal survives context compaction and a process restart on the same session.
- **Continuation:** turns continue automatically until the goal completes, is paused or is cleared.

#### 1.3 Evidence recording

SPEC § Role eligibility uses per-requirement rows:

```
| Backend | Role | Requirement | Source | Delivery | Eligible | Evidence |
```

| Column | Values |
|---|---|
| Requirement | `goal`, `listen` or `verdict` for Director rows; `—` for the Member row |
| Source | `native` or `extension` for `goal` and `listen` rows; `—` for `verdict` and Member rows |
| Delivery | `subscribe` or `compromise` for `listen` rows; `—` otherwise |
| Eligible | `yes`/`no`. The `verdict` row is `yes` only when the `goal` and `listen` rows are both `yes`. |
| Evidence | Requirement rows: mechanism, native version and `checked YYYY-MM-DD`, at most two sentences. `verdict` row: `goal and listen rows` for `yes`, or the unmet requirement names for `no`. Member row: the current content. |

Each backend section's Role eligibility table in `coding-agents.md` uses the same rows without the Backend column. The `.claude/rules/documentation-maintenance.md` Template § Role eligibility changes to this shape.

#### 1.4 Resulting verdicts

| Backend | Goal | Listen | Director |
|---|---|---|---|
| `claude` 2.1.309 | native `/goal` | native subscribe (Monitor), from run A-Cl | `yes` |
| `codex` 0.165.0 | native `/goal` | native subscribe (`codex queue` via `cafleet message listen --codex-thread`), from run A-C | `yes` |
| `opencode` 2.2.0 | extension (CAFleet plugin), from run A-O | extension subscribe (CAFleet plugin), from run A-O | `yes`, effective from 2.2.0 |
| `hermes` 2026.9.24 | native `/goal` on the TUI client, from run A-H | extension subscribe (CAFleet plugin, TUI client), from run A-H | `yes` |

These are the target verdicts. Each row is recorded only from the observed result of its §6 run, and a failed run records `no` with the observed failure. No backend remains on tier 3; tier 3 stays in SPEC as the acceptance tier for future backends.

OpenCode's Member evidence (2.1.4) carries forward unchanged. Its Director rows name 2.2.0 as the minimum version, because the plugin uses session instruction entries, which first appear in 2.2.0.

### 2. Vendored extensions as embedded assets

#### 2.1 Layout and embedding

```
extensions/
  hermes/cafleet/plugin.yaml
  hermes/cafleet/__init__.py
  opencode/cafleet.ts
```

`build.rs` adds `println!("cargo:rerun-if-changed=extensions")` and `emit_table(&mut generated, "EXTENSIONS", &extensions)`, matching the `SKILLS` and `PRESETS` tables. The files are embedded verbatim. `plugin.yaml` carries the fixed literal `version: "1"`, so the embedded bytes equal the installed bytes.

#### 2.2 Install targets and executable record (SPEC §6.3 Asset installation)

| Agent | Bundled extension | Install target | Executable record |
|---|---|---|---|
| claude | — | — | — |
| codex | — | — | — |
| opencode | `extensions/opencode/cafleet.ts` | `<preset base>/plugins/cafleet.ts` | `<preset base>/plugins/cafleet.executable` |
| hermes | `extensions/hermes/cafleet/` (every file) | `<hermes home>/plugins/cafleet/` | `<hermes home>/plugins/cafleet/executable` |

- **`<preset base>`:** the existing OpenCode resolution: `OPENCODE_CONFIG_DIR`, else `~/.config/opencode`.
- **`<hermes home>`:** the parent directory of the path that `hermes config path` prints.
  - Hermes discovers user plugins at `get_hermes_home() / "plugins"`. A sticky `<base>/active_profile` re-homes `get_hermes_home()` to the profile directory, and `hermes config path` follows that profile, as the passthrough merge already relies on.
  - Installing the Hermes extension therefore requires `hermes` on `PATH`. Without it, setup skips the Hermes extension together with both merges (§2.3).

The *executable record* is a one-line file holding the absolute, canonicalized path of `std::env::current_exe()` of the `cafleet` that runs setup. Setup writes it after the extension files. It is not an embedded asset and is not byte-compared. OpenCode ignores it, because plugin discovery takes only `.ts`/`.js` files and directories (`packages/core/src/plugin/source-directory.ts:23-28`). The plugins execute only this recorded path, never a path a tool argument supplies (§4.1, §5.1).

**Order and output.** Setup installs skills, then presets, then extensions with their executable record, then runs the Hermes configuration merges. It prints one line per extension target after the target and its executable record are written; a directory extension prints its directory once:

```
<agent>: installed extension (v<version>) -> <target>
```

Each existing target is deleted before its replacement is written. A symlink target is unlinked as an entry, and its referent is left intact.

**Failure.** A failure fails setup with `failed to install extension into <target>: <error>` and records no `asset_installs` row. The one `asset_installs` row per agent covers skills, presets and extensions.

#### 2.3 Hermes enablement merge

`HermesConfigRunner::run` changes its contract:

- It returns `HermesOutput { exit_code: i32, stdout: String, stderr: String }` for every process that ran to completion.
- It returns `Err` only for a spawn failure or the 10 s deadline.
- Callers decide which exit codes are acceptable.
- The existing passthrough merge keeps treating any non-zero exit as a failure, with the stderr text as the cause.

A key is *not set* when the exit code is 1 and one stderr line, trimmed, equals exactly `Config key not set: <key>`. Hermes prints an unrelated `⚠ install out of sync …` line to stderr on every command, so the rule matches a line, not the whole stream.

After the passthrough merge, setup for `hermes` runs these steps, each through the local binary with `HERMES_HOME` set to the resolved base:

1. Run `hermes config get --json plugins.disabled`. If the key is set, its value must be a JSON array of strings. When it lists `cafleet`, setup fails with the cause `plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'`. The deny-list wins over `plugins.enabled`, and the operator's explicit disable is preserved.
2. Run `hermes config get --json plugins.enabled`. *Not set* means an empty list; otherwise the value must be a JSON array of strings.
3. When `cafleet` is missing, run `hermes config set plugins.enabled '<existing entries in order, then "cafleet", as compact JSON>'`.
4. Run `hermes config get --json plugins.entries.cafleet.allow_gateway_injection`. When the key is *not set* or its value is not `true`, run `hermes config set plugins.entries.cafleet.allow_gateway_injection true`.
5. Re-read both keys and require `cafleet` in `plugins.enabled` and `true` for `allow_gateway_injection`.

**Effect of setting `plugins.enabled`.** A missing `plugins.enabled` currently enables nothing (`plugins_discovery.py:122-131`). Hermes's one-time config migration 20→21 (`config_migrations.py:310-329`) grandfathers installed user plugins only while the key is missing. After setup writes the key, other user plugins that were installed but not enabled stay disabled, as they are now. The operator enables them explicitly; the docs Hermes section states this.

Setup prints exactly one of these lines:

```
hermes: plugin cafleet already enabled -> <config path>
hermes: enabled plugin cafleet -> <config path>
```

Any failed command, a timeout, an unexpected value or a listed `plugins.disabled` entry fails setup with `failed to enable hermes plugin cafleet in <config path>: <cause>`. In those cases no `asset_installs` row is recorded.

Setup never runs `hermes plugins enable`. That command's dependency admission re-syncs the Hermes venv, the plugin declares no Python dependencies, and the admission fails on Homebrew installs (`uv.lock` absent). When plain `cafleet setup` runs without `hermes` on `PATH`, Hermes skills are installed and this skip line replaces the existing one:

```
hermes: skipped the plugin and terminal.client_env_passthrough names (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes
```

SPEC § Hermes environment passthrough currently says setup "touches nothing else under the Hermes home" and "reads and writes no other Hermes key". It is rewritten to list exactly what setup touches:

- the skills
- `<hermes home>/plugins/cafleet/`
- `terminal.client_env_passthrough`
- `plugins.enabled`
- `plugins.disabled` (read only)
- `plugins.entries.cafleet.allow_gateway_injection`

#### 2.4 Doctor checks (SPEC §6.3 `cafleet doctor`)

Check order inserts three rows into § Check rows. Each follows the rows named below and precedes `<agent>.installation`:

| Check name | Follows |
|---|---|
| `opencode.extension` | `opencode.preset[.<mode>]` |
| `hermes.extension` | `hermes.client_env_passthrough` |
| `hermes.plugin_enabled` | `hermes.extension` |

**`opencode.extension`**

| State | Status | Detail |
|---|---|---|
| The `opencode` executable is absent from `PATH` | `info` | `unavailable: executable is absent` |
| The opencode configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| `<preset base>/plugins/cafleet.ts` equals the embedded bytes and the executable record names an existing executable file | `ok` | The plugin path |
| The plugin is missing, unreadable or differs from the embedded bytes | `error` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent opencode'` |
| The executable record is missing, unreadable or names no executable file | `error` | `<record path>: <cause>; run 'cafleet setup --coding-agent opencode'` |

**`hermes.extension`.** This row runs `hermes config path` with the passthrough check's bounds.

| State | Status | Detail |
|---|---|---|
| The `hermes` executable is absent from `PATH` | `info` | `unavailable: executable is absent` |
| The hermes configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| `hermes config path` fails or times out | `error` | `cannot read hermes configuration through 'hermes config path': <cause>` |
| Every embedded file equals its installed copy under `<hermes home>/plugins/cafleet/`, and the executable record names an existing executable file | `ok` | The plugin directory |
| A file is missing, unreadable or differs | `error` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent hermes'` |
| The executable record is missing, unreadable or names no executable file | `error` | `<record path>: <cause>; run 'cafleet setup --coding-agent hermes'` |

**`hermes.plugin_enabled`.** This row runs three bounded, read-only `hermes config get --json` calls: `plugins.disabled`, `plugins.enabled` and `plugins.entries.cafleet.allow_gateway_injection`. It applies the *not set* rule from §2.3.

| State | Status | Detail |
|---|---|---|
| The `hermes` executable is absent from `PATH` | `info` | `unavailable: executable is absent` |
| The hermes configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| A call fails other than *not set*, times out or prints a value of the wrong JSON type | `error` | `cannot read hermes configuration through 'hermes config get': <cause>` |
| `plugins.disabled` lists `cafleet` | `error` | `hermes plugins.disabled lists cafleet; remove it, then run 'cafleet setup --coding-agent hermes'` |
| `plugins.enabled` lacks `cafleet`, or `allow_gateway_injection` is not `true` | `error` | `hermes plugin cafleet is not enabled: <key>, …; run 'cafleet setup --coding-agent hermes'` |
| Otherwise | `ok` | `cafleet enabled with gateway injection` |

A backend summary line also folds these rows. § Summary lines gains:

| Check state | Status | Phrase | Setup fix |
|---|---|---|---|
| Extension missing, unreadable or differing | `error` | `extension <path> is missing or differs from the embedded copy` | yes |
| Executable record unusable | `error` | `extension executable record <path> is unusable: <cause>` | yes |
| Hermes config path unreadable | `error` | `cannot read hermes configuration through 'hermes config path': <cause>` | no |
| Hermes plugin listed in `plugins.disabled` | `error` | `hermes plugins.disabled lists cafleet` | no |
| Hermes plugin not enabled | `error` | `hermes plugin cafleet is not enabled: <key>, …` | yes |

An unreadable `config get` reuses the existing `cannot read hermes configuration through 'hermes config get': <cause>` phrase.

### 3. Codex native subscribe: `cafleet message listen --codex-thread`

#### 3.1 Contract (SPEC §6.3 `message listen`; `skills/cafleet/reference/cli.md`)

```
cafleet message listen MEMBER_ID [--timeout SECONDS] [--codex-thread THREAD]
```

With `--codex-thread THREAD`, the listener keeps the record, ordering, replay, lease and idle contracts. It changes its output as follows:

- **Stdout** carries the `ready` record and the final `closed` record, and nothing between them. The recipe never reads the exec session again, and an unread PTY must not stall the listener's five-second output deadline.
  - The final `closed` record is written right before exit and carries every reason, `session_exited` included.
  - Writing that record is the listener's last action. A write failure, for example on the PTY of an exited session, leaves the exit status unchanged.
- **Forwarded events.** Each `message` record, and each `closed` record whose reason is `timeout`, `member_inactive` or `fleet_deleted`, is forwarded in emission order. `ready` is never forwarded, and neither is `closed` `session_exited`.
- **Forwarding command:** `codex queue --thread THREAD --message <text>`. The text is `[cafleet listen record] <compact record JSON>`.
- **Large records.** When that text exceeds 65,536 bytes, the forwarded text is instead the reference below; Linux caps one argument at 128 KiB (`MAX_ARG_STRLEN`), and message bodies have no limit.

  ```
  [cafleet listen record] {"event":"message_ref","message_id":<id>,"from_member_id":<id>,"bytes":<n>} — read it with: cafleet message show <id> --json
  ```
- **One at a time.** Each `codex queue` runs to exit before the next record is forwarded, with a 10 s deadline.
- **Full queue.** Codex 0.165.0 refuses a submission while the thread's queue holds 100 (`queue cannot contain more than 100 submissions (code -32600)`). A `codex queue` that exits non-zero with a stderr line containing `queue cannot contain more than` is a *full-queue refusal*, not a failure: the listener keeps the same record, waits 1 s, checks the pinned session (§3.2) and retries it. Later records wait behind it, so emission order holds and no record is dropped. The pinned session's exit during the wait ends the listener with `session_exited`.
- **Full-queue limit.** When one record meets full-queue refusals for 600 s without being accepted, the listener fails (table below).
- **Binary resolution.** `codex` resolves through the listener's `PATH` with the absolute-entry search of § Native executable resolution.

| Condition | Behavior |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| No Codex session ancestor at startup (§3.2) | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| `codex queue` exits non-zero, other than a full-queue refusal | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, closed notice, exit 1 |
| Full-queue refusals for one record reach 600 s | stderr `codex thread <THREAD> queue stayed full for 600 s`, closed notice (one attempt, which can itself meet the full queue), exit 1 |
| `codex queue` exceeds 10 s | kill it; stderr `codex queue for thread <THREAD> did not finish within 10 s`, closed notice, exit 1 |
| Any other failure after `ready` (database, schema guard, deadline) | its existing stderr diagnostic, closed notice, exit 1 |
| The pinned Codex process exits | stdout `closed` with reason `session_exited`, exit 0 (nothing is forwarded, since the session is gone) |

**Closed notice.** Before every non-zero exit after `ready`, the listener makes one best-effort attempt, bounded by 10 s, to forward `[cafleet listen closed] exit 1: <stderr diagnostic>` with `codex queue`. This mirrors the plugins' `[cafleet listen closed]` turn, so an idle Codex Director learns that reception ended. When the attempt also fails, stderr keeps the original diagnostic and adds `failed to forward the closed notice to codex thread <THREAD>: <cause>`, and the exit status stays 1.

`closed.reason` gains `session_exited`. It is emitted only in forwarding mode, and only on stdout. Fleet deletion still takes precedence.

#### 3.2 Receiver lifecycle (Codex)

**Pinning the session process.** At startup the listener walks its ancestors, starting from its parent. It reads `/proc/<pid>/stat` (ppid) and `/proc/<pid>/cmdline` on Linux, and `ps -o ppid= -o args= -p <pid>` elsewhere.

It pins the nearest ancestor whose argv[0] basename is exactly `codex`. Shells, `bwrap`, node launchers and the sandbox helper are skipped. The sandbox helper is the same binary, but it runs with argv[0] `codex-linux-sandbox`.

The executable path is not compared, because the helper shares it. Under npm-installed Codex, the native process's argv[0] may differ from `codex`.

A-C establishes the rule on this host's mise/aqua install, on the escalated path only. On this host bwrap cannot create namespaces, so the forwarder cannot run sandboxed; the sandboxed path and other installs stay unverified. If a sandbox unshares the PID namespace, the Codex ancestor is invisible to the walk. Startup then fails with `--codex-thread requires a codex ancestor process`, and the recipe's escalated path applies.

**Liveness.** Each idle tick (at most 1 s) checks the pinned pid with `kill(pid, 0)`. On Linux it also compares the pinned process's start time from `/proc/<pid>/stat`, so a reused pid is treated as exited. The pinned process's exit ends the listener with `session_exited`.

**No `PR_SET_PDEATHSIG`.** It fires when the forking *thread* exits, and Codex is a multi-threaded tokio process.

The reason for this lifecycle: in C1, `member delete` killed the Codex pane while the bridge stayed alive as an orphan.

#### 3.3 Codex Director reception recipe (coding-agents.md `#### Codex native Director reception`)

1. After registration, start through native `exec_command`, with the literal Director ID and a short initial yield: `cafleet message listen <director-id> --codex-thread "$CODEX_THREAD_ID"`.
   - Codex injects `CODEX_THREAD_ID` into every exec shell (`core/src/exec_env.rs:30-42`).
   - Where the sandbox cannot start the forwarder (for example, bwrap namespace creation fails), it runs on the escalated path under the host's approval rules.
2. Retain the exec `session_id`, and require `ready` in the initial output. The session is not read again.
3. Each record arrives as a queued user turn beginning `[cafleet listen record]`.
   - Consume it, reconcile with `poll --json`, ACK, and act within authorization.
   - For a `message_ref`, read the message with `cafleet message show <id> --json`.
4. Records interleave with native `/goal` continuation turns at turn boundaries (C2).
5. A forwarded `closed` record or a `[cafleet listen closed]` turn follows receiver recovery. When even the closed notice could not be forwarded, the Director's per-wake `poll --json` and overdue-reply checks under § Director wake surface the gap. On recovery, the Director reads the retained exec session once for the final output.

The `write_stdin` read loop and its "no unsolicited per-line wake" Note text are removed. `docs/docs/mixed-backend-team.md:78` ("A fully idle Codex Director reconciles its inbox on the next user or native activation") is rewritten, since a queued record now starts a turn on an idle Codex Director.

### 4. Hermes extension subscribe: the `cafleet` plugin

#### 4.1 Plugin contract

`plugin.yaml`:

```yaml
name: cafleet
version: "1"
description: "CAFleet Director reception: push each broker record into the owning session."
author: CAFleet
kind: standalone
provides_tools:
  - cafleet_listen
provides_hooks:
  - on_session_finalize
```

**Private seams.** The plugin depends on four Hermes seams, each a private core interface outside the plugin API. They are the only ways in 2026.9.24 to identify the calling session, read its environment, observe its attachment and inject records in order:

| Seam | Use |
|---|---|
| `hermes_cli.plugin_session.session_key()` | calling session key |
| `gateway.session_context.get_session_env(name, default)` | session environment (`CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID`) |
| `gateway.session_authorities.authority_for_home(...)` → `authority.sessions[...].subscribers` | client attachment (§4.2) |
| the published gateway runner (`ctx._manager._gateway_message_injector[0]`), its `_gateway_loop` and `_dispatch_plugin_message_injection(session_key=, content=, plugin_id=, plugin_home=)` | ordered injection (below) |

The tool resolves all four on every call. If an import or lookup fails, or the attachment read raises, the tool returns `{"ok": false, "error": "<seam>: <exception>"}` and starts no receiver, so no receiver ever runs without its lifecycle signals.

**Ordered injection.** `ctx.inject_message` only schedules its dispatch coroutine on the gateway loop and returns `True` before the record is committed (`gateway/run_plugin_injection.py:76-110`). Consecutive calls therefore run concurrent coroutines whose awaited session lookups can reorder them before the session authority's ordered write (`gateway/session_automation.py:465-487`); A-H observed 994→995→993 and 996→998→997 for three back-to-back records. The plugin instead submits each record's `runner._dispatch_plugin_message_injection(...)` coroutine with `asyncio.run_coroutine_threadsafe(coro, runner._gateway_loop)` and waits for its result, with a 10 s deadline, before the next record. The tool checks `ctx._gateway_injection_allowed()` before spawning the child, because this path bypasses the consent check `inject_message` performs; a missing consent returns `{"ok": false, "error": "plugins.entries.cafleet.allow_gateway_injection is not true; run 'cafleet setup --coding-agent hermes'"}`.

**Tool `cafleet_listen(director_id: integer)`**, in toolset `cafleet`:

1. When `get_session_env("CAFLEET_MEMBER_ID")` is non-empty, return `{"ok": false, "error": "cafleet_listen runs only in a Director session"}`. Member sessions export `CAFLEET_MEMBER_ID` through `terminal.client_env_passthrough`.
2. Read the session key. An empty key returns `{"ok": false, "error": "cafleet_listen must be called from a session turn"}`.
3. When this key already owns a live receiver, return `{"ok": true, "pid": <pid>, "session_key": <key>, "existing": true}`.
4. Read the executable record (`Path(__file__).with_name("executable")`). If it is missing or is not an executable file, return `{"ok": false, "error": "executable record <path>: <cause>; run 'cafleet setup --coding-agent hermes'"}`.
5. Spawn `<recorded cafleet> message listen <director_id>` inside the gateway. The child gets the gateway's environment, plus `CAFLEET_DATABASE_URL` from the session environment when that value is non-empty. Return `{"ok": true, "pid": <pid>, "session_key": <key>}`.
6. A reader thread injects each stdout line as `[cafleet listen record] <line>` into the session `<key>` through ordered injection, one record at a time.

The listener runs without `--timeout`; the plugin performs no automatic replacement.

**Failure surfacing.**

| Event | Plugin action |
|---|---|
| An ordered injection returns `False` | Kill the child, because the session is gone |
| An ordered injection raises or exceeds its 10 s deadline | Kill the child and log the cause; the closed notice below is attempted once through the same path |
| The child exits without a `closed` record | Inject `[cafleet listen closed] exit <code>: <last stderr line>` into the owning session |
| A `closed` record | Already injected as a record; the receiver ends |

The Director owns every replacement through receiver recovery.

#### 4.2 Receiver lifecycle (Hermes)

The child runs inside the shared Hermes gateway, which outlives client sessions. In H1/H2 the listener survived `member delete` and TUI `/exit`. The plugin binds each receiver to its owning session with three signals. Any one of them kills the child and confirms that it exited:

| Signal | Source |
|---|---|
| Session finalize | `on_session_finalize` for the owning session. CLI shutdown and the TUI session lifecycle fire it (`hermes_cli/lifecycle.py:81-97`). `on_session_end` is per-turn and unused. |
| Client detach | A watchdog thread checks every 5 s that the gateway's live local session for the key has at least one subscriber. After 30 s with no subscriber the receiver ends; the grace covers client reconnects. A read error kills the child and logs the seam error. |
| Injection refusal | An ordered injection returns `False` |

`closed` `fleet_deleted` remains the normal end of reception.

#### 4.3 Hermes Director reception recipe (coding-agents.md `#### Hermes native Director reception`)

1. The Hermes Director runs the TUI client (`hermes chat --tui`, or `display.interface: tui`). In the classic CLI, injected turns run without rendering.
2. After registration, call `cafleet_listen` with the literal Director ID. Require `ok` and the `ready` record.
3. Each record arrives as its own turn beginning `[cafleet listen record]`. Consume it, reconcile with `poll --json`, ACK, and act within authorization.
4. A `[cafleet listen closed]` turn, an `ok: false` result, or a missing `cafleet_listen` tool follows receiver recovery. A missing tool or an executable-record error is fixed by `cafleet setup --coding-agent hermes`, which `cafleet doctor` names.

The following are removed: the `terminal` `background=true` / `process_manage` / `heartbeat=60` recipe, its overflow and wedge rules, the heartbeat Note row, and the "invoke `cafleet` by its absolute path in a background command" Note row. `/goal` authority stays with the host.

**Client-specific facts.** The Hermes Note rows state the client each fact applies to:

- The Director uses the TUI.
- Members use `--cli`.
- The leading-`!` native shell input row names the client it was verified on.

A-H verifies the TUI's `/goal` lifecycle, the `clarify` decision surface and leading-`!` input. The native `/goal` and `clarify` evidence currently comes from the classic CLI.

### 5. OpenCode extension goal and subscribe: the `cafleet` plugin

One file, `extensions/opencode/cafleet.ts`, defines `Plugin.define({ id: "cafleet", setup })` for OpenCode V2 plugins.

**Supported Director launch:** `opencode mini --standalone` from the project directory, with the default agent. Plugins load in that private server, which inherits the launcher's environment (the O1/O2 surface). In the shared background service, plugins do not receive the client's environment, so the service is not a supported Director launch. A-O verifies the plugin under this launch with the default agent.

**Member refusal.** When `process.env.CAFLEET_MEMBER_ID` is set in the server, every tool and the `/cafleet-goal` command refuse with `cafleet tools run only in a Director session`. Member servers inherit the member environment. Each tool also calls `ctx.permission.assert({ sessionID, agent, source, action: "cafleet", resources: [<tool name>] })` before doing any work. Both OpenCode member presets add `"cafleet": "deny"` to their permission map, so a member's call is denied even before the environment check.

**Director-side resolution.** In OpenCode 2.2.0 every agent, the default `build` agent included, starts with an ordered base policy whose first rule is `{ "action": "*", "resource": "*", "effect": "allow" }`. Later global and agent rules override it. The `cafleet` action therefore resolves to `allow` on a default Director, without prompting. Source: v2.2.0 `services/www/src/docs/content/permissions.mdx` § Defaults; the "no rule matches → `ask`" fallback in § Rules applies only when no rule at all matches.

An operator configuration that adds a broad `ask` or `deny` rule overrides the base allow. The quickstart § OpenCode task documents the operator rule `{ "action": "cafleet", "resource": "*", "effect": "allow" }`, which restores the allow for the Director. A-O observes that `cafleet_listen`, `cafleet_goal_progress` and continuation turns open no permission prompt on a default Director.

#### 5.1 Receiver

- **Tool:** `cafleet_listen(director_id: integer)` returns `{ ok, pid, sessionID }` for the calling `context.sessionID`. It keeps one receiver per session; a repeat call returns the existing pid with `existing: true`.
- **Executable:** the tool reads the executable record at `new URL("./cafleet.executable", import.meta.url)`. A missing or unusable record returns `ok: false` with the record path and the setup fix.
- **Delivery:** the plugin spawns `<recorded cafleet> message listen <director_id>` with `node:child_process`. It calls `ctx.session.synthetic({ sessionID, text: "[cafleet listen record] <line>", delivery: "queue" })` for each stdout line.
- **Failure:** a rejected `synthetic` kills the child. An exit without a `closed` record queues `[cafleet listen closed] exit <code>: <last stderr line>`. No automatic replacement happens.
- **Lifecycle:** the plugin's cleanup kills every child. O2-e showed that deleting the member ends the receiver: the standalone server's stdin lease ends, and plugin cleanup runs. A-O repeats the check for the Director launch, by exiting the client.

#### 5.2 Goal extension

| Operation | Surface | Effect |
|---|---|---|
| set | `/cafleet-goal <objective>` | Store `{objective, status: "active", generation, continuations: 0, idle_streak: 0}` under storage key `goal/<sessionID>`, put instruction entry `cafleet-goal`, queue the first continuation |
| status | `/cafleet-goal status`; tool `cafleet_goal_status` | The command queues a synthetic `[cafleet goal] status: <json>` turn. The tool returns the stored JSON. |
| pause | `/cafleet-goal pause` | Status `paused`, bump `generation`, update the entry |
| resume | `/cafleet-goal resume` | Status `active`, bump `generation`, reset `idle_streak`, queue a continuation |
| clear | `/cafleet-goal clear` | Bump `generation`, remove the storage key and the entry |
| complete | tool `cafleet_goal_complete(summary)` | Status `complete`, bump `generation`, update the entry |
| progress | tool `cafleet_goal_progress(note)` | Reset `idle_streak` |

**Continuation.**
- On `session.execution.succeeded` for a session with an `active` goal, the plugin queues a synthetic item with `delivery: "queue"` and metadata `{ cafleetGoalGeneration: <generation> }`. Its text is `[cafleet goal] Continue working toward the active goal: <objective>. Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved.`
- A `session.execution.interrupted` with `reason: "user"` pauses the goal.

**Stale continuations.** The `context` session hook removes every `[cafleet goal]` continuation message whose generation differs from the stored one, or whose goal is not `active`. A continuation queued just before `pause`, `clear` or `complete` therefore never reaches the model.

The reason: in O2-c, one queued continuation ran after `pause`. OpenCode 2.2.0 exposes `session.inbox.cancel` only on the HTTP API, not in the plugin `SessionDomain`.

**No-progress stop.**
- A continuation turn that ends without a `cafleet_goal_progress` or `cafleet_goal_complete` call increments `idle_streak`.
- At 3, the goal pauses. The plugin then queues `[cafleet goal] paused: no progress in 3 consecutive continuation turns; the operator resumes it with /cafleet-goal resume`, with `resume: false`.
- The 50-continuation cap also pauses the goal.

The reason: in O2-e, a mandated command failed every turn, and the goal looped without progress.

**Persistence.**
- *Compaction.* The instruction entry (2.2.0) carries the goal across compaction; O2-d showed this.
- *Restart.* Restoration is scoped to sessions that this server serves. A stored `active` goal for `sessionID` is restored on the server's first event, tool call or command for that `sessionID`. After a restart, the Director's first turn re-arms the receiver with `cafleet_listen`. That turn restores the goal, and the next `session.execution.succeeded` queues the continuation without further input.
- *Never at setup.* Setup scans no storage. A member server, or any other OpenCode server sharing the config directory, never restores the Director's goal.

O2 ran one server, so it did not establish whether `ctx.storage` is shared across servers. A-O establishes the sharing scope and checks that spawning a member while the Director's goal is active queues nothing.

#### 5.3 OpenCode Director reception and goal recipe

coding-agents.md gains `#### OpenCode native Director reception`:

1. Launch the Director with `opencode mini --standalone`.
2. After registration, call `cafleet_listen` with the literal Director ID. Require `ok` and `ready`.
3. Consume each `[cafleet listen record]` turn as on Hermes.
4. The operator sets, pauses, resumes and clears the goal with `/cafleet-goal`.
5. While working toward an active goal, the Director reports steps with `cafleet_goal_progress` and finishes with `cafleet_goal_complete`, as Codex's model-callable goal tools allow.
6. Reception itself calls no goal tool.

`skills/cafleet/roles/director.md` § Director wake is reworded to match. It says that reception never starts, resumes, pauses, clears or re-budgets a goal, and that progress and completion reports belong to the goal-working Director.

The opencode Note row "Director verdict is `no`" is replaced with the extension facts.

### 6. Live acceptance (implementation-time)

**Isolation.** The implementation fleet runs on the installed skills, presets and binary, which `.claude/rules/commands.md` requires to stay intact.

- **During implementation:** run A-S with the freshly built binary from the build directory, against temporary `HERMES_HOME`, `OPENCODE_CONFIG_DIR` and `CAFLEET_DATABASE_URL`. Real homes are never written.
- **After implementation:** the backend runs need real backend credentials, so they need real-home setup. They start only after the Director has deleted every implementation member and confirmed that no executing member depends on the installed assets. Only then does the Director reinstall the CLI and run `cafleet setup` for each backend in the real homes.

**Probe residue.** Before the first backend run, confirm that the design-time probe changes are absent. The user reverted them on 2026-11-05:

| Check | Expected |
|---|---|
| `hermes config get --json plugins.entries` | `Config key not set: plugins.entries` |
| `hermes config get --json plugins.enabled` | `Config key not set: plugins.enabled`, or a list without `cafleet-probe` |
| `/root/.hermes/plugins/cafleet-probe` | absent |
| `/root/.config/opencode/plugins/cafleet-probe.ts` | absent |
| `/root/.config/opencode/plugins/cafleet-goal-probe.ts` | absent |

Stop if any probe change is present. Real-home setup may legitimately create `plugins.enabled` (holding `cafleet`) and `plugins.entries.cafleet` afterwards.

Run each backend step through Director-spawned probe members. Record the observed result before writing evidence:

| Run | Steps |
|---|---|
| A-S | `cafleet setup --coding-agent hermes` and `--coding-agent opencode` on temporary homes: clean home, rerun idempotence, a home with an active profile (extension lands in the profile's plugins directory), and `plugins.disabled` listing `cafleet` (setup fails with the stated cause). Then `cafleet doctor` rows: `ok`, a tampered extension file, a missing executable record and a missing enablement key, each `error`. |
| A-Cl | Claude: subscribe probe through the Monitor recipe (idle, busy ≥ 60 s, burst of 3) |
| A-C | Codex, under an active `/goal`, on the escalated path (the sandboxed path is unverified on this host):<br>• subscribe probe with `--codex-thread`<br>• a record over 65,536 bytes arrives as `message_ref`<br>• 200 records sent in one burst all arrive, in order, with the listener alive<br>• the listener stays alive across 10 idle minutes<br>• a forced `codex queue` failure, from a `codex` wrapper earlier on the forwarder's `PATH`, arrives as a `[cafleet listen closed]` turn<br>• `member delete` makes the listener exit within 1 s with `session_exited`, and `ps` shows no surviving `cafleet message listen` |
| A-H | Hermes TUI:<br>• subscribe probe through `cafleet_listen`<br>• `/goal` set/status/pause/resume/clear, `clarify` and leading-`!` input in the TUI<br>• TUI `/exit` and pane kill each end the child within the 30 s grace<br>• a second `cafleet_listen` returns `existing: true`<br>• `cafleet_listen` in a member session returns the refusal |
| A-O | OpenCode, as Director with `opencode mini --standalone` and the default agent:<br>• subscribe probe<br>• no permission prompt opens for the CAFleet tools or continuation turns<br>• goal bar: set, status, pause with an in-flight continuation dropped, resume, compaction, restart on the same session with the goal restored by the re-arm turn, clear, complete<br>• no-progress stop after 3 idle continuations<br>• spawning a member while the goal is active queues nothing into the Director session<br>• the `ctx.storage` sharing scope is recorded<br>• client exit leaves no child<br>• a member's `cafleet_listen` and `/cafleet-goal` are denied |
| A-M | Member re-check for OpenCode and Hermes with the plugins installed: the bundled profiles' approval-prompt coverage and default launch behave as recorded, and the new tools are unavailable or denied |


---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Steps 1–5 write the documentation before code. They restate the current verdicts in the per-requirement row shape. For each new mechanism, they write the row with Eligible `no` and Evidence `not established (acceptance run A-x pending)`. Step 10 rewrites those rows, and the prose that depends on them, from the observed results. The dependent prose covers the host-evidence paragraphs, the Note rows and the docs verdict mentions.

### Step 1: SPEC.md

- [x] Rewrite §6.7 Role requirements: the two independent Director requirements, listen tiers with the 2 >> 3 gap, the tier-3 negative-evidence definition, the subscribe probe with the busy rule, goal tiers and the extension-goal minimum bar (§1.1–1.2) <!-- completed: 2026-11-05T22:22 -->
- [x] Replace §6.7 Role eligibility with the per-requirement table (§1.3), with pending rows for new mechanisms, and update the verdict-change paragraph <!-- completed: 2026-11-05T22:22 -->
- [x] Update §6.3 Asset installation: extension table, executable record, install order, output line, failure string, `<hermes home>` resolution and the combined skip line, plus the `cafleet setup` output examples <!-- completed: 2026-11-05T22:22 -->
- [x] Add the Hermes enablement merge (§2.3) and rewrite § Hermes environment passthrough to list exactly what setup touches <!-- completed: 2026-11-05T22:22 -->
- [x] Add the doctor rows, their order, states and details, and the summary phrases (§2.4) <!-- completed: 2026-11-05T22:22 -->
- [x] Add `--codex-thread` to §6.3 `message listen`: stdout scope, forwarded events, message reference, deadline, startup errors, `session_exited` and ancestor pinning (§3.1–3.2) <!-- completed: 2026-11-05T22:22 -->
- [x] Rewrite the codex and hermes host-evidence paragraphs in §6.7, add an opencode one, and remove the heartbeat recipe text <!-- completed: 2026-11-05T22:22 -->

### Step 2: Documentation site

- [x] `docs/docs/concepts/coding-agents.md`: Director requirements in tier terms, extension support, the Hermes Director on the TUI client and the OpenCode Director launch <!-- completed: 2026-11-05T22:25 -->
- [x] `docs/docs/contributing.md`: add the subscribe probe and the tier recording next to the stream probe, and describe the stream probe as the tier-3 probe <!-- completed: 2026-11-05T22:25 -->
- [x] `docs/docs/quickstart.md`: § Hermes (TUI Director launch, plugin install and enablement by setup, other user plugins stay as enabled) and § OpenCode (plugin install, `opencode mini --standalone` Director launch, `/cafleet-goal`, and the `cafleet` allow rule for configurations whose broad rules override the base allow) <!-- completed: 2026-11-05T22:25 -->
- [x] `docs/docs/mixed-backend-team.md:78`: replace the idle-Codex sentence with queued-turn reception <!-- completed: 2026-11-05T22:25 -->
- [x] Update the setup and doctor descriptions on the remaining docs pages that list what `cafleet setup` installs <!-- completed: 2026-11-05T22:25 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-05T22:25 -->

### Step 3: README.md

- [x] Check the README pitch and install commands against the docs index; edit only if they drift <!-- completed: 2026-11-05T22:26 -->

### Step 4: Skills

- [x] `skills/cafleet/reference/coding-agents.md` § Role requirements: restate the criterion (§1) <!-- completed: 2026-11-05T22:29 -->
- [x] claude section: per-requirement Role eligibility rows <!-- completed: 2026-11-05T22:29 -->
- [x] codex section: forwarder reception recipe (§3.3), Note rows and per-requirement rows <!-- completed: 2026-11-05T22:29 -->
- [x] opencode section: Director launch, reception and goal recipe (§5.3), Note rows and per-requirement rows <!-- completed: 2026-11-05T22:29 -->
- [x] hermes section: TUI requirement, plugin recipe (§4.3), client-specific Note rows, removal of the heartbeat recipe and rows, and per-requirement rows <!-- completed: 2026-11-05T22:29 -->
- [x] `skills/cafleet/reference/cli.md`: `message listen --codex-thread`, `session_exited`, `message_ref`, setup extension lines and doctor rows <!-- completed: 2026-11-05T22:29 -->
- [x] `skills/cafleet/roles/director.md` § Director wake: wording that holds for handle-read (tier 3) and turn-delivered (tiers 1–2) reception, and the reception goal-authority sentence (§5.3) <!-- completed: 2026-11-05T22:29 -->
- [x] `skills/cafleet/roles/director.md` § Receiver recovery and Spawn Protocol step 4: rows for an unavailable or refused extension tool, `[cafleet listen closed]`, forwarded `closed` and `session_exited` <!-- completed: 2026-11-05T22:29 -->

### Step 5: Project rules

- [x] `.claude/rules/documentation-maintenance.md`: Template § Role eligibility uses the per-requirement shape, and § Director verdict changes names the subscribe probe, tier recording and negative evidence <!-- completed: 2026-11-05T22:30 -->

### Step 6: Embedded extensions and presets

- [x] Add `extensions/hermes/cafleet/plugin.yaml` and `__init__.py` (§4.1–4.2), and validate them with `hermes plugins validate` <!-- completed: 2026-11-05T22:42 -->
- [x] Update SPEC.md (the Hermes plugin delivery row and private-seam list) and any skill or docs text that names `inject_message` as the delivery to the §4.1 ordered injection <!-- completed: 2026-11-06T04:11 -->
- [ ] `extensions/hermes/cafleet/__init__.py`: ordered injection through the gateway runner's dispatch coroutine with a 10 s deadline, the consent check before spawning, the fourth seam and its failure rows (§4.1–4.2); validate with `hermes plugins validate` on a temporary `HERMES_HOME` <!-- completed: -->
- [x] Add `extensions/opencode/cafleet.ts` (§5) <!-- completed: 2026-11-05T22:42 -->
- [x] Add `"cafleet": "deny"` to `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` <!-- completed: 2026-11-05T22:42 -->
- [x] Add the `EXTENSIONS` table to `build.rs` <!-- completed: 2026-11-05T22:42 -->

### Step 7: Setup and doctor

- [x] `src/assets.rs`: extension paths per agent (`<hermes home>` from `hermes config path`), installation after presets, the executable record, output lines, failure string, and the `asset_installs` row covering extensions <!-- completed: 2026-11-05T22:57 -->
- [x] `src/assets.rs` and `src/cli/setup.rs`: change `HermesConfigRunner::run` to return `HermesOutput { exit_code, stdout, stderr }`, keep the passthrough merge's failure rule, and add the *not set* line match <!-- completed: 2026-11-05T22:57 -->
- [x] `src/assets.rs`: the Hermes enablement merge (§2.3) and the combined skip line <!-- completed: 2026-11-05T22:57 -->
- [x] `src/cli/setup.rs`: doctor rows `opencode.extension`, `hermes.extension` and `hermes.plugin_enabled`, and their summary phrases (§2.4) <!-- completed: 2026-11-05T22:57 -->
- [x] Unit tests in `src/assets.rs`: installed bytes equal embedded bytes at the selected targets, the executable record content, install order and output lines, and a failure records nothing <!-- completed: 2026-11-05T22:57 -->
- [x] Unit tests in `src/assets.rs`: the Hermes merge appends `cafleet` after existing entries, sets `allow_gateway_injection`, treats *not set* as empty only for an exact stderr line, fails on `plugins.disabled` and on other non-zero exits, and is idempotent <!-- completed: 2026-11-05T22:57 -->
- [x] Update `hermes_setup_reads_only_the_config_path_and_the_passthrough` (`src/assets.rs:761`) to assert the complete new call sequence <!-- completed: 2026-11-05T22:57 -->
- [x] Unit tests in `src/cli/setup.rs`: doctor `ok` and every `info`/`error` state of the three rows, and the summary phrases <!-- completed: 2026-11-05T22:57 -->

### Step 8: Codex forwarder

- [x] `src/cli/message.rs`: `--codex-thread` option, `codex` resolution, ancestor pinning by argv[0], liveness with start-time check, forwarding-mode stdout, forwarded event set, `message_ref` above 65,536 bytes, 10 s deadline, failures, the closed notice and `session_exited` closure (§3.1–3.2) <!-- completed: 2026-11-05T23:05 -->
- [x] Unit tests in `src/cli/message.rs`, using a fake `codex` executable and re-executed children via `test_path!`:
  - forwarded events, order and text
  - `message_ref` above the threshold
  - non-zero exit and deadline expiry
  - missing binary and missing ancestor
  - ancestor pinning past a `codex-linux-sandbox` intermediate
  - `session_exited` on ancestor exit, with nothing forwarded
  - stdout holds `ready` and the final `closed` record only
  - the closed notice is forwarded before a non-zero exit after `ready`
  - a failed notice keeps the original diagnostic and exit 1
  <!-- completed: 2026-11-05T23:05 -->
- [x] Document the full-queue retry and the 600 s full-queue limit in SPEC §6.3 `message listen`, `skills/cafleet/reference/cli.md` and the codex Note rows (Codex's 100-submission queue cap) <!-- completed: 2026-11-06T02:00 -->
- [x] `src/cli/message.rs`: treat a full-queue refusal as a 1 s wait-and-retry of the same record with the pinned-session check, and fail with the full-queue-limit diagnostic after 600 s (§3.1) <!-- completed: 2026-11-06T02:12 -->
- [x] Unit tests in `src/cli/message.rs` with a fake `codex` that refuses with the full-queue text:
  - refusals followed by acceptance forward the record once, and later records keep their order
  - the pinned session's exit during the wait ends the listener with `session_exited`
  - refusals past the limit (shortened for the test) exit 1 with the full-queue-limit diagnostic after the closed-notice attempt
  - other non-zero exits still fail immediately
  <!-- completed: 2026-11-06T02:12 -->

### Step 9: Validation

- [ ] `mise //:test` <!-- completed: -->
- [x] `mise //:lint` <!-- completed: 2026-11-05T23:19 -->
- [x] Run A-S with the built binary against temporary homes and database (§6 Isolation) <!-- completed: 2026-11-05T23:19 -->

### Step 10: Live acceptance and evidence

- [x] After every implementation member is deleted, confirm that no executing member depends on the installed assets. Then reinstall the CLI (`mise tasks --all` names the install task) and run `cafleet setup` for each backend in the real homes. <!-- completed: 2026-11-06T00:04 -->
- [x] Verify the probe-residue table in §6 and stop if any probe change is present <!-- completed: 2026-11-06T00:03 -->
- [x] Run A-Cl (Claude) <!-- completed: 2026-11-06T00:07 -->
- [x] After the full-queue fix, delete its implementation members, reinstall the CLI and rerun `cafleet setup` for each backend <!-- completed: 2026-11-06T02:14 -->
- [ ] After the Hermes ordered-injection fix, delete its implementation members, reinstall the CLI and rerun `cafleet setup --coding-agent hermes` <!-- completed: -->
- [ ] Run A-C (Codex) <!-- completed: -->
- [ ] Run A-H (Hermes TUI) <!-- completed: -->
- [ ] Run A-O (OpenCode) <!-- completed: -->
- [ ] Run A-M (Member re-check for OpenCode and Hermes) <!-- completed: -->
- [ ] Rewrite the SPEC § Role eligibility rows and the coding-agents.md rows from the observed results, recording `no` with the observed failure for any failed run. Update the dependent prose: host-evidence paragraphs, Note rows and docs verdict mentions. <!-- completed: -->

### Step 11: Final consistency

- [ ] Search SPEC, docs, skills and rules for the remaining `heartbeat`, `process_manage` and `write_stdin` reception mentions and for the old Director wording ("streamed background output"). Each surviving mention must describe the current state. <!-- completed: -->
- [ ] Commit `design-docs/0000217-extension-backed-director-requirements/design-doc.md` with the implementation <!-- completed: -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-11-05 | Initial draft from clarification rounds 1–2 and design-time probes C1, C2, H1, H2, O1, O2 |
| 2026-11-05 | Review round 1. Relaxed the busy rule and added A-Cl. Defined tier-3 negative evidence and the `verdict` row cells. Resolved the Hermes home through `hermes config path`, handled *not set* and `plugins.disabled`, and fully specified the doctor rows. Codex forwarding now covers the forwarded events, a message reference, a 10 s deadline, a stdout scope and argv[0] ancestor pinning, and drops PDEATHSIG. The plugins execute only a setup-recorded `cafleet` and refuse member sessions. Restore is scoped per session, and the goal-authority wording is reconciled. Added implementation isolation, probe-residue verification and the docs pages. |
| 2026-11-05 | Review round 2. Overview is now three sentences. Forwarding-mode stdout carries `ready` and the final `closed` record. A Codex closed notice is forwarded before a non-zero exit. The OpenCode Director-side `cafleet` permission now resolves through the base allow. A-C is limited to the escalated path. |
| 2026-11-05 | Approved by the user after Reviewer approval; Status set to Approved |
| 2026-11-06 | A-C found Codex 0.165.0's 100-submission thread-queue cap: a 200-record burst ended the forwarder at record 97. Approved by the user: a full-queue refusal is now a 1 s wait-and-retry of the same record with a 600 s limit (§3.1), the busy rule states that waiting behind earlier records passes (§1.1), and the forwarder stays tier 1 because Codex still starts every turn. |
| 2026-11-06 | A-H found that Hermes `inject_message` returns before its dispatch commits, so three back-to-back records reordered twice. Approved by the user: the plugin awaits each record's gateway dispatch before the next (ordered injection, §4.1), adding a fourth private seam and a consent check; Hermes stays tier 2. |
