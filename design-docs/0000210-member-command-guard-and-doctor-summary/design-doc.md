# Member command guard, backend configuration cleanup and doctor summary

**Status**: Approved
**Progress**: 24/60 tasks complete
**Last Updated**: 2026-11-03

## Overview

The PTY runner marks every member process with `CAFLEET_MEMBER_ID`, and the six Director-only commands refuse to run when that variable is set. This one guard replaces the Director-only deny and prompt entries that setup writes into backend configuration, including the Hermes `approvals.deny` merge that blocks a Hermes Director's own commands. `cafleet doctor` gets the same simplification: by default it prints one plain summary line per component, groups members by state with ID ranges and names the next command, and keeps the per-check rows under `--verbose` and `--json`.

## Success Criteria

- [ ] Every member child process spawned by the runner has `CAFLEET_MEMBER_ID=<member id>` in its environment, and Hermes members' terminal tools see it through `terminal.client_env_passthrough`.
- [ ] With `CAFLEET_MEMBER_ID` set to any value, including empty, each of `member prompt`, `member exec`, `member press`, `member recover`, `permission respond` and `input reconcile` exits 1 with the guard error, before reading settings or opening the database or multiplexer. With the variable unset, these commands behave as they do today.
- [ ] Every other command behaves the same whether `CAFLEET_MEMBER_ID` is set or not.
- [ ] `cafleet setup` writes no Director-only deny or prompt entry into any backend configuration. The Hermes `approvals.deny` merge, its spawn preflight and the `hermes.approvals.deny` doctor row are gone. The Codex rules file holds only the broad `cafleet` allow, and the OpenCode presets hold no Director-only denies.
- [ ] `member create --coding-agent hermes` refuses to start a member when `terminal.client_env_passthrough` lacks a CAFleet member name, so a Hermes member never runs without the guard variable.
- [ ] By default, `cafleet doctor` prints one line for each of `multiplexer`, `database` and each backend, plus one `members` line per occupied member state, using ranges such as `1-6, 9-22`. Actionable states have status `warn` and end with ` — run '<command>'`. Ended runs of deregistered members are hidden.
- [ ] `cafleet doctor --verbose` prints one row per check. `cafleet doctor --json` prints `{"summary":[…],"checks":[…]}`. Only an `error` status makes doctor exit 1.
- [ ] `skills/cafleet/roles/member.md` tells members to send Director-only needs to the Director.
- [ ] `SPEC.md`, `docs/`, the skills and their references describe the guard, the reduced setup writes and the new doctor contract. Nothing in them still describes the removed Hermes deny merge, Codex prompt rules or OpenCode preset denies, or the old doctor layout.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Six Director-owned commands write into a member's live native session or runtime record: `member prompt`, `member exec`, `member press`, `member recover`, `permission respond` and `input reconcile`. Today CAFleet keeps members away from them with backend configuration:

| Backend | What setup writes today | Scope of the write | Effect on a Director |
|---|---|---|---|
| claude | Nothing. SPEC §6.7 recommends `permissions.ask` rules for `member prompt`/`exec`/`press`. | Operator settings | The operator confirms each dispatch |
| codex | `<codex home>/rules/cafleet.rules`: a broad `cafleet` allow plus six `decision = "prompt"` rules | Global rules directory | A Codex Director is prompted for its own commands |
| opencode | Six `deny` entries in the `cafleet` and `cafleet-ask` presets | Member-only agent presets | None |
| hermes | Six globs merged into the global `approvals.deny`, checked by a `member create` preflight and a doctor row | The Hermes configuration that every session reads | **A Hermes Director's own Director commands are blocked**, although `skills/cafleet/reference/coding-agents.md` records Hermes Director eligibility `yes` |

`cafleet doctor` also prints one line per check: about 18 `ok` lines for backends, then one `info member_runtime.<run_id>` line per recorded run. Its detail is internal wording such as `matching owner lock is currently available; explicit recovery establishes loss; runtime endpoint unavailable: member runtime: ECONNREFUSED: Connection refused; inspect with member recover`. A real run on 2026-11-03 printed 24 runtime lines, 20 of them identical.

---

## Specification

### 1. Runner guard

#### 1.1 Member environment

The spawn environment (SPEC §6.3 *Member environment*) becomes, in this order:

| # | Name | Value | Condition |
|---|---|---|---|
| 1 | `CAFLEET_DATABASE_URL` | `sqlite:///<canonical database path>` | Always |
| 2 | `CAFLEET_MEMBER_ID` | The decimal member ID that registration allocated | Always |
| 3 | The selected backend's config-location variable | The creator's exact value | When the creator's environment sets it |

`member create` validates the config-location variable **before registration**, as it does today, and assembles the full spawn environment inside the registration closure, where `pending.member.member_id` is known (`src/cli/member.rs`, the closure passed to `runner::create_registered`). The API split:

```rust
impl CodingAgent {
    /// Validates and returns the forwarded config-location entry; called before registration.
    pub(crate) fn forwarded_config(self, env: EnvLookup<'_>)
        -> Result<Option<(String, String)>, CafleetError>;
    /// Infallible assembly in the order of the table above.
    pub(crate) fn spawn_environment(
        database_url: &str,
        member_id: i64,
        forwarded_config: Option<(String, String)>,
    ) -> Vec<(String, String)>;
}
```

The launch descriptor and launch evidence record the spawn environment as they do today. A member ID is not a secret. The runner's own pane environment (`src/runtime/lifecycle.rs`, `split_runner`) keeps only `CAFLEET_DATABASE_URL`. The runner applies `CAFLEET_MEMBER_ID` to the native child only, so neither the multiplexer session nor any later pane inherits it.

`HERMES_CLIENT_ENV_PASSTHROUGH` gains `CAFLEET_MEMBER_ID` as its second entry:

```
CAFLEET_DATABASE_URL, CAFLEET_MEMBER_ID, TMUX, TMUX_PANE, HERDR_ENV, HERDR_PANE_ID,
HERDR_TAB_ID, HERDR_WORKSPACE_ID, HERDR_SOCKET_PATH, HERDR_BIN_PATH, HERDR_SESSION
```

Setup's existing merge appends missing names after existing entries, so rerunning setup on an existing installation adds `CAFLEET_MEMBER_ID` at the end of the operator's list. A Hermes Director's client does not export the variable, and the gateway leaves an unexported listed name unset, so the Director's tools never see it.

#### 1.2 Guarded commands and refusal

The Director-only command set is exactly: `member prompt`, `member exec`, `member press`, `member recover`, `permission respond`, `input reconcile`. Every other subcommand is unguarded, including `permission inspect`, `input show`, `event show`, `member list`, `member capture` and lifecycle commands.

- **Registry.** One function in `src/cli/mod.rs` is the single source of truth: `fn director_only_name(command: &Command) -> Option<&'static str>`. It returns the space-separated command path (`"member prompt"`, …) for exactly the six variants above and `None` for everything else. SPEC and `skills/cafleet/reference/cli.md` list the same six commands as contract facts.
- **Check point.** In `cli::run`, immediately after `CliArgs::parse()` and before `Settings::from_env()`. A parse error (exit 2) still comes first. The refusal therefore opens no database, multiplexer, lock, socket or file, and writes nothing.
- **Trigger.** `std::env::var_os("CAFLEET_MEMBER_ID").is_some()`: the variable is present with any value, including an empty or non-UTF-8 one. The value is not parsed.
- **Refusal.** The command exits 1 with this application error on stderr and prints nothing on stdout. This applies to `permission respond` as well: the structured-response exception does not apply because no response attempt began. `<value>` is the variable's value rendered with `to_string_lossy`:

```
Error: 'cafleet <command>' is a Director-only command and this process runs inside CAFleet member <value> (CAFLEET_MEMBER_ID is set); send the request to your Director.
```

Example: `Error: 'cafleet member prompt' is a Director-only command and this process runs inside CAFleet member 34 (CAFLEET_MEMBER_ID is set); send the request to your Director.`

The refusal also applies when the Director dispatches one of these commands into a member pane with `member exec` (`! cafleet member prompt …`). That command runs inside the member and is refused, which is the intended behavior.

**Selecting property.** The guarded set is the commands that write into a member's live native session or its runtime record: explicit native pane input (`member prompt`, `member exec`, `member press`), native permission answers (`permission respond`), input-receipt settlement (`input reconcile`) and run reconciliation (`member recover`). This is exactly the set the Codex prompt rules, the OpenCode preset denies and the Hermes deny globs covered. Lifecycle commands (`member create`, `member delete`, `fleet create`, `fleet delete`), roster listing and captures stay Director-owned by role but are not guarded. The guard replaces those configuration guards one for one, and they never covered lifecycle commands.

#### 1.3 Limits (stated in SPEC and docs)

- **Accident guard, not a security boundary.** A member can unset the variable (`env -u CAFLEET_MEMBER_ID cafleet …`) or override it. The guard stops an accidental or instruction-confused invocation. Member authority is still defined by the member role and the Director's assignment.
- **Depends on environment inheritance.** The guard holds only when the backend passes the member's process environment to its tool commands, the same condition `CAFLEET_DATABASE_URL` already needs. Claude, Codex (default `shell_environment_policy`) and OpenCode inherit it. Hermes gets it through `terminal.client_env_passthrough`, which the Hermes preflight (§2.4) now requires.

#### 1.4 Rejected alternative

Process-ancestry detection, which would refuse when a runner process is an ancestor, was rejected. Hermes runs a session's terminal tools in the shared Hermes gateway, not as descendants of the member's runner, so ancestry cannot identify a Hermes member. An environment variable works for every backend, through passthrough on Hermes.

### 2. Backend configuration writes after the guard

Principle: setup writes only what members need in order to run. It writes no Director-only deny, ask or prompt entry into any backend configuration.

| Backend | Write | Decision | Rationale |
|---|---|---|---|
| claude | Nothing (skills only) | Unchanged. The SPEC/docs `permissions.ask` guidance stays as an **optional Director-side operator-confirmation posture**. | Ask rules let the operator confirm each explicit native pane-input dispatch. The rationale "a member's own attempt reaches the ask rule" is dropped, because the guard covers members. |
| codex | `<codex home>/rules/cafleet.rules` | Keep the file, reduced to `prefix_rule(pattern = ["cafleet"], decision = "allow")`. Drop the six `prompt` rules. | Codex members have no spawn-time allow, so they need the broad allow to run broker commands. The prompt rules sit in the global rules directory and prompt a Codex Director for its own commands, while the guard already covers members. |
| opencode | `agents/cafleet.md`, `agents/cafleet-ask.md` | Keep both presets (spawn preconditions). Drop the six Director-only `deny` entries. `cafleet *` allow stays. | One guard and one refusal message on every backend. Under `cafleet-ask`, a denied entry would otherwise raise no form, while the guard gives a clear routing message. |
| hermes | `terminal.client_env_passthrough` merge | Keep, with `CAFLEET_MEMBER_ID` added (§1.1). Remove the `approvals.deny` merge, the `approvals.deny` preflight and the `hermes.approvals.deny` doctor row. | Members need passthrough to see their own database, pane and member-ID values. The deny merge blocks the Hermes Director. |

#### 2.1 Codex rules file

`presets/codex/cafleet.rules` content contract, with exactly one trailing newline:

```
prefix_rule(pattern = ["cafleet"], decision = "allow")
```

SPEC §6.7 *codex rules file* describes one broad allow. Codex loads every `*.rules` file and applies the strictest decision, so an operator who wants confirmation adds `prompt` rules in a separate file. The minimum set for a rules directory maintained without the installed file shrinks to the `["cafleet", "message"]` allow, in both SPEC and the docs quickstart. The quickstart paragraph saying the installed file "keeps the Director-owned commands prompting" is rewritten: the file allows every `cafleet` command.

#### 2.2 OpenCode presets

The six lines `"cafleet member prompt *": "deny"` … `"cafleet input reconcile *": "deny"` are removed from the `shell` map of both presets. `"cafleet *": "allow"` becomes the map's last entry. Body updates:

- `cafleet-ask.md` first paragraph: drop "denies the Director-owned CAFleet commands,".
- Second paragraph of both presets: replace "Broad tool patterns do not grant Director-only actions or Git write authority to an ordinary member." with "Your role and assignment, not the broad `cafleet *` pattern, decide which CAFleet commands you run; Director-only commands refuse inside a member and you send those needs to the Director." Keep the Git-write authority clause.

SPEC §6.7 *opencode preset* and *Preset file format* drop every "Director-only deny" mention, including the "later denies … win over `cafleet *`" sentence and "every Director-only deny" in the `cafleet-ask` difference paragraph. The preset still has three body paragraphs, no internal hard line breaks and one trailing newline.

#### 2.3 Hermes setup

SPEC `###### Hermes approval rules` is renamed `###### Hermes environment passthrough` and keeps only the passthrough merge:

1. `hermes config path` gives `<config path>` (with `HERMES_HOME` set to the resolved base on every call, as today).
2. `hermes config get --json terminal.client_env_passthrough` must print a JSON array of strings.
3. Merge: existing entries in order, then each missing CAFleet name in canonical order (exact string comparison).
4. Nothing missing: print `hermes: terminal.client_env_passthrough already holds CAFleet's member names -> <config path>` and write nothing.
5. Otherwise `hermes config set terminal.client_env_passthrough '<compact JSON>'` and print `hermes: added terminal.client_env_passthrough names -> <config path>: <name>, …`.
6. Re-read and require every CAFleet name.

Failure: `failed to install hermes terminal.client_env_passthrough names into <config path>: <cause>`, and no `asset_installs` row is recorded. When step 1 fails, `<config path>` is the resolved Hermes base. This sentence moves here from the removed deny section. Plain `cafleet setup` with `hermes` absent from `PATH` prints:

```
hermes: skipped terminal.client_env_passthrough names (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes
```

Code: remove `HERMES_DENY_LIST`, `HERMES_DIRECTOR_ONLY_DENIES` and the deny branch of `install_agent` in `src/assets.rs`. `HermesList` keeps a single instance (`HERMES_CLIENT_ENV_LIST`). If the struct only remains to parameterize one list, inline it.

**Existing installations.** Setup does not touch `approvals.deny`. The docs quickstart Hermes section and the coding-agents overlay tell the operator that CAFleet needs no `approvals.deny` entries. They also say that a `cafleet …` glob there blocks the matching command in a Hermes Director's own session, which Hermes reports as `BLOCKED: this command matches the user-defined deny rule '<glob>' …`. Inspect the list with `hermes config get --json approvals.deny` and remove CAFleet's globs with `hermes config set approvals.deny '<JSON list without them>'`. The six glob strings are spelled out in that docs paragraph, because the operator has to recognize them.

#### 2.4 Hermes availability preflight

`ensure_available` for hermes replaces the `approvals` read with the passthrough read. After the config-dir and `PATH` checks it runs `hermes config get --json terminal.client_env_passthrough`. It uses the same bounds as today's approvals read: null stdin, the member's `HERMES_HOME`, captured and bounded output, and a two-second deadline inside the ten-second creation budget. It applies `hermes_client_env_names`:

| Condition | Error (exit 1, before registration) |
|---|---|
| The command cannot start, times out, exits nonzero, or prints something other than a JSON array of strings | `Error: cannot read hermes configuration through 'hermes config get': <cause>` |
| The list lacks a CAFleet member name | `Error: hermes terminal.client_env_passthrough lacks CAFleet's member names: <name>, …; run 'cafleet setup --coding-agent hermes'` |
| `hermes config path` fails (unchanged, § Hermes work settlement) | unchanged |

Rationale: without `CAFLEET_MEMBER_ID` in passthrough, a Hermes member would run with no guard, and with no `CAFLEET_DATABASE_URL` either. Failing at spawn is the fail-fast form of that invariant. `SpawnProbe::hermes_approvals`, `SystemProbe::hermes_approvals`, `hermes_approvals_preflight`, `hermes_approvals` and `hermes_approval_denies` are removed. `SpawnProbe::hermes_client_env_passthrough` becomes the spawn and doctor read.

### 3. Member instruction

Append one paragraph to `skills/cafleet/roles/member.md` § *Identity and authority*. The group is described by its property, and the command list stays in `cli.md`:

> Commands that write into a member's live native session or runtime record — explicit pane input, permission answers, input reconciliation and run recovery — refuse inside a member with `Error: 'cafleet <command>' is a Director-only command …; send the request to your Director.` Lifecycle commands, roster listing and captures run without that refusal and remain the Director's as well. Send a need for any Director-owned command to the Director with the routing message in [Reconsider, then route](#reconsider-then-route), naming the command and its purpose.

Spawn-prompt templates are unchanged. Every spawn prompt already points members at `member.md`, and the refusal message itself names the routing.

### 4. `cafleet doctor` output

#### 4.1 Flags and output selection

| Invocation | stdout |
|---|---|
| `cafleet doctor` | Summary lines (§4.3–4.4) |
| `cafleet doctor --verbose` | Per-check rows (§4.5), no summary |
| `cafleet doctor --json` (with or without `--verbose`) | `{"summary":[…],"checks":[…]}` (§4.6) |

Statuses are `ok`, `info`, `warn` and `error`. After printing, doctor exits 1 with `Error: doctor found errors` when any check has status `error`. Otherwise it exits 0, even with `warn`. The checks still run in their current order and none aborts early. Help: `--verbose` reads `Print one row per individual check instead of the per-component summary`.

#### 4.2 Check-level changes

| Check | Change |
|---|---|
| `hermes.approvals.deny` | Removed |
| `<agent>.installation` recorded at another version | `info` → `warn`. Detail unchanged. |
| `<agent>.installation` with no installation recorded | `info` → `warn`. Detail unchanged. |
| `member_runtime.<run_id>` | Status and detail follow §4.4. JSON entries gain `member_id` and `state`. |
| `member_runtime` budget-exhausted aggregate row | Removed. Each unprobed run gets its own `not_checked` row (§4.4). |

#### 4.3 Component summary lines

Text form: `<status> <component>: <detail>`, with LF and CR escaped as today. Components appear in check order: `multiplexer`, `database`, each backend in registry order, then the `members` lines.

| Component | `ok` detail | Problem detail |
|---|---|---|
| `multiplexer` | `<backend> (session <session>, pane <pane_id>)` | `error`: the check's cause |
| `database` | `schema <N>` | `error`: the check's cause, which already names `run 'cafleet setup'` or `upgrade cafleet` |
| `<agent>` | `ready` | Rules below |

A backend line folds that backend's checks: `configuration`, `executable`, the OpenCode `preset` checks, `hermes.client_env_passthrough` and `installation`. Rules, first match wins:

1. `configuration` is `error`: the line is `error <agent>: <cause>`.
2. `executable` is absent: the line is `info <agent>: not installed (<agent> is not on PATH)`. The other checks of an unused backend raise nothing.
3. Otherwise, each non-`ok` check contributes a problem phrase. The line's status is the worst contributed status (`error` > `warn` > `info`). The detail joins the phrases with `; ` in check order and, when any phrase has the setup fix, ends once with ` — run 'cafleet setup --coding-agent <agent>'`.

| Check state | Status | Phrase | Setup fix |
|---|---|---|---|
| OpenCode preset missing or unreadable | `error` | `preset <path> is unreadable: <cause>` | yes |
| Hermes passthrough lacks names | `error` | `terminal.client_env_passthrough lacks CAFleet's member names: <name>, …` | yes |
| Hermes passthrough unreadable | `error` | `cannot read hermes configuration through 'hermes config get': <cause>` | no |
| Installation recorded at another version | `warn` | `assets from cafleet <version> (this is <cli version>)` | yes |
| No installation recorded | `warn` | `assets not installed at <path>` | yes |
| Installation unavailable because the database is not at head | `info` | `assets unchecked (database unavailable)` | no |
| Installation query failed | `error` | `installation query failed: <cause>` | no |

Examples:

```
ok claude: ready
warn codex: assets from cafleet 0.30.0 (this is 0.31.1) — run 'cafleet setup --coding-agent codex'
info opencode: not installed (opencode is not on PATH)
error hermes: terminal.client_env_passthrough lacks CAFleet's member names: CAFLEET_MEMBER_ID — run 'cafleet setup --coding-agent hermes'
```

#### 4.4 Member states

Doctor classifies every recorded run in two passes against a read-only connection:

1. **Registry pass** (every run, no probes, outside the time budget). A single query reads `run_id`, `member_id`, run `status`, `cleanup_state` and the member's registry `status`.
2. **Probe pass** (only runs that the registry pass leaves unclassified). The existing descriptor, run-path, owner-lock and health probes run under today's budget: five seconds in total, each run bounded by the smaller of 100 ms and the remaining budget.

Classification, first match wins:

| # | Condition | State token | Text label | Status | Next command |
|---|---|---|---|---|---|
| 1 | The run's diagnosis returned an error, or the owner-lock probe failed | `check_failed` | `could not be checked` | `error` | `cafleet doctor --verbose` |
| 2 | Run `status` in (`exited`, `failed`, `lost`), `cleanup_state = complete`, member deregistered | `ended` | (hidden) | `info` | — |
| 3 | Run `status` in (`exited`, `failed`, `lost`), `cleanup_state = complete`, member active | `agent_exited` | `agent exited` | `warn` | `cafleet member delete ID` |
| 4 | Any other run of a deregistered member | `deleted_with_leftover_runtime` | `deleted with leftover runtime` | `warn` | `cafleet member recover ID` |
| 5 | Budget exhausted before probing this run | `not_checked` | `not checked (time limit)` | `info` | `cafleet member show ID` |
| 6 | Leftover launch descriptor, or run paths fail validation | `interrupted_launch` | `interrupted launch` | `warn` | `cafleet member recover ID` |
| 7 | Health reply observed | `running` | `running` | `ok` | — |
| 8 | The health probe was skipped because the deadline passed | `not_checked` | `not checked (time limit)` | `info` | `cafleet member show ID` |
| 9 | Owner lock held, no health reply | `not_responding` | `not responding` | `warn` | `cafleet member recover ID` |
| 10 | Owner lock free, no health reply | `lost_runtime` | `lost runtime` | `warn` | `cafleet member recover ID` or `cafleet member delete ID` |

Rows 2–4 come from the registry pass. Rows 1 and 5–10 come from the probe pass. In row 1, "diagnosis returned an error" includes the diagnosis's own per-run read of the run record.

**Registry-pass failure.** When the registry query fails, no run IDs are known, so the probe pass is skipped. Doctor still prints every other check, then exits 1 with `Error: doctor found errors`. Today's `member_run_ids(conn)?` aborts doctor instead, and this replaces that. The members output is:

| Surface | Value |
|---|---|
| Summary line | `error members: could not be checked: <cause>` (no command suffix, because the cause is the whole finding) |
| Check row | `error member_runtime: member runtimes could not be read: <cause>` |
| JSON summary entry | `{"component":"members","status":"error","detail":"could not be checked: <cause>","state":"check_failed","member_ids":[]}` |
| JSON check entry | `{"name":"member_runtime","status":"error","detail":"member runtimes could not be read: <cause>","member_id":null,"state":"check_failed"}` |

`RuntimeDiagnosis` (`src/runtime/control.rs`) returns a typed `state` enum alongside its plain diagnostic phrases, and doctor formats from the enum. The diagnostic phrases are reworded:

| Today | New phrase |
|---|---|
| `leftover launch descriptor; use member recover after ownership validation` | `launch descriptor left over` |
| `matching owner lock is currently available; explicit recovery establishes loss` | `owner lock is free` |
| `matching owner lock held: live_or_unreachable` | `owner lock is held` |
| `matching runtime health reply observed` | `runtime answered the health probe` |
| `runtime endpoint unavailable: <cause>; inspect with member recover` | `runtime did not answer the health probe: <cause>` |
| `<cause>; explicit member recover required` | `runtime files are invalid: <cause>` |

**Summary lines.** Members are grouped by member ID. Each member has at most one non-terminal run (`idx_member_runs_active`). If a member has several runs, it is listed once per distinct state.

One line is printed per occupied state, in this order: `running`, `not responding`, `lost runtime`, `interrupted launch`, `agent exited`, `deleted with leftover runtime`, `could not be checked`, `not checked (time limit)`. Format:

```
<status> members: <N> <label>: <ID ranges>[ — run '<command>'[ or '<command>']]
```

- `<N>` is the number of distinct member IDs in that state.
- **ID ranges**: IDs ascending and de-duplicated. A run of three or more consecutive IDs collapses to `a-b`; one or two consecutive IDs stay separate (`7, 8`). Items join with `, `.
- In the next-command text, `ID` stays literal: the line covers several members.
- With no live member (no runs, or only `ended`), print `ok members: none running`.
- When the database is not at head, print `info members: not checked (database unavailable)`.

The real run from Background would print:

```
ok members: 4 running: 7, 8, 23, 24
warn members: 20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID' or 'cafleet member delete ID'
```

The second line assumes those runs belong to active members. Runs of deregistered members would appear as `deleted with leftover runtime` or would be hidden as `ended`, as the table classifies them.

#### 4.5 `--verbose` rows

One `<status> <name>: <detail>` row per check, in today's order, with the §4.2 status changes. A member runtime row:

```
<status> member_runtime.<run_id>: member <member_id> <label>: <phrase>[; <phrase>…][ — run '<command with the literal member ID>']
```

Examples:

```
ok member_runtime.7: member 7 running: owner lock is held; runtime answered the health probe
warn member_runtime.9: member 9 lost runtime: owner lock is free; runtime did not answer the health probe: member runtime: ECONNREFUSED: Connection refused — run 'cafleet member recover 9' or 'cafleet member delete 9'
info member_runtime.3: member 3 ended: run exited, cleanup complete
error member_runtime.5: member 5 could not be checked: database is locked
```

An `ended` row uses the label `ended` and the phrase `run <status>, cleanup complete`. A row from the registry pass for state 3 or 4 uses the phrase `run <status>, cleanup <cleanup_state>`. A `not_checked` row uses `runtime diagnosis time budget exhausted`.

A `check_failed` row is `error member_runtime.<run_id>: member <member_id> could not be checked: <cause>`, with no command suffix. `<cause>` is the message of the diagnosis error or of the owner-lock probe error, and no other phrase is added. `cafleet doctor --verbose` is the next command only on the summary line, where it leads to this row's cause.

#### 4.6 JSON

```json
{"summary":[
  {"component":"multiplexer","status":"ok","detail":"herdr (session w2, pane w2:p2N)"},
  {"component":"members","status":"warn","detail":"20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID' or 'cafleet member delete ID'","state":"lost_runtime","member_ids":[1,2,3,4,5,6,9,10,11,12,13,14,15,16,17,18,19,20,21,22]}
 ],
 "checks":[
  {"name":"multiplexer","status":"ok","detail":"herdr session=w2 window=w2:tP pane=w2:p2N"},
  {"name":"member_runtime.9","status":"warn","detail":"member 9 lost runtime: …","member_id":9,"state":"lost_runtime"}
 ]}
```

- Key order: summary entries use `component, status, detail`. `members` entries add `state, member_ids`: the `none running` entry has `state` `running` and `member_ids` `[]`, and the database-unavailable entry has `state` `not_checked` and `member_ids` `[]`.
- Check entries use `name, status, detail`. `member_runtime.*` entries add `member_id, state`.
- `summary` holds exactly the lines the text summary prints. `checks` holds exactly the `--verbose` rows, including `ended` rows.
- The detail strings in `summary` and `checks` equal the text forms, minus the `<component>: ` and `<name>: ` prefixes. The verbose member-row prefix `member <id> <label>: ` stays.

### 5. Documentation placement

| Fact | Owner | Other mentions |
|---|---|---|
| Guard contract (variable, command set, trigger, error, check point, limits) | SPEC §6.3 new `#### Director-only command guard` | `cli.md` carries the command set and error string. `docs/docs/concepts/members.md` § Command routing gets one paragraph on the behavior and its limit. `member.md` carries the routing instruction. |
| Spawn environment | SPEC §6.3 *Member environment* | — |
| Setup writes per backend | SPEC §6.3 *Asset installation* and *Hermes environment passthrough*, §6.7 codex/opencode sections | The docs quickstart describes what setup installs and gives the Hermes removal instruction. `cli.md` § setup and doctor. |
| Doctor contract | SPEC §6.3 *`cafleet doctor`* | `cli.md` § setup and doctor. A docs sentence in `concepts/members.md` § Inspection and recovery. |
| `CAFLEET_MEMBER_ID` as a read variable | SPEC §6.3 *Environment variables*: a row with Field `—`, Default unset, Meaning "Set by the runner in each member's spawn environment. Presence refuses the Director-only commands (§ Director-only command guard); the value is shown, never parsed." | SPEC §6.8 *Configuration*: "Spawned members receive `CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and, when the creator sets it, the selected backend's config-location variable". `cli.md` § Environment variables: the same row, and its last sentence names `CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and the selected backend's set override. |
| IDs still come from literals | `skills/cafleet/SKILL.md` § IDs keeps "environment variables supply no member identity" and adds: "Commands take literal IDs; `CAFLEET_MEMBER_ID` only marks a member process, and the Director-only commands check only its presence." SPEC §6.3 *Positional subject ids* gets the same clarification in one sentence. | — |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation is updated first, in the order `.claude/rules/documentation-maintenance.md` requires. Tests are in-file unit tests in the `#[cfg(test)] mod tests` block of the file that owns the asserted behavior (`.claude/rules/code-quality.md` § Test placement).

### Step 1: SPEC.md

- [x] §6.3 *Output shapes*: the `doctor` row becomes text `One STATUS COMPONENT: DETAIL summary line per component (--verbose: one STATUS NAME: DETAIL row per check)` and JSON `{summary, checks}` <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3: add `#### Director-only command guard` after *Sender and recipient*, with §1.2–§1.4 (command set, trigger, check point before settings/database/multiplexer, refusal string and exit 1, no stdout for `permission respond`, accident-guard and inheritance limits, the rejected process-ancestry alternative stated descriptively) <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *Environment variables*: add the `CAFLEET_MEMBER_ID` row (§5). §6.3 *Positional subject ids*: add the literal-ID clarification (§5) <!-- completed: 2026-11-03T04:51 -->
- [x] §6.8 *Configuration*: the spawned-member sentence names `CAFLEET_MEMBER_ID` (§5) <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *`permissions.allow` coverage*: reword the `permissions.ask` paragraph as an optional Director-side operator-confirmation posture and point member protection at the guard section <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *Config-dir resolution* table, hermes row: setup merges only `terminal.client_env_passthrough` (§ Hermes environment passthrough) <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *Asset installation*: hermes table row and the "Hermes prints the skills line followed by …" sentence name only the member-names line. Replace `###### Hermes approval rules` with `###### Hermes environment passthrough` per §2.3, including the new skip line. Repoint every `§6.3 Hermes approval rules` reference in SPEC <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *Member environment*: the spawn environment per §1.1, including `CAFLEET_MEMBER_ID`, runner-pane exclusion and the passthrough sentence <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *`cafleet doctor`*: flags (`--json`, `--verbose`), output selection, statuses with `warn`, exit rule, check-row table without `hermes.approvals.deny` and with `warn` installation states, summary rules (§4.3), member states (§4.4), verbose rows (§4.5), JSON (§4.6). Replace `###### Hermes approval checks` with a single-row `hermes.client_env_passthrough` check section and rewrite `##### Member runtime checks` <!-- completed: 2026-11-03T04:51 -->
- [x] §6.3 *Error Messages*: add the guard row (the six commands, `CAFLEET_MEMBER_ID` set, exit 1, before settings/database/multiplexer). Change the hermes `member create` row to the passthrough preflight. Remove the `approvals.deny` merge `setup` row <!-- completed: 2026-11-03T04:51 -->
- [x] §6.7 *Backend capabilities*, hermes row: prerequisite becomes `approvals.mode` plus the `terminal.client_env_passthrough` names, a spawn precondition. Add that an operator `approvals.deny` glob matching a Director-only command blocks it in a Hermes Director's session <!-- completed: 2026-11-03T04:51 -->
- [x] §6.7 *Claude permission posture*: profile table ask row "Why" becomes the optional operator confirmation of explicit native pane-input dispatch, without the member-attempt rationale. Adjust the "An ask rule outranks every allow rule …" sentence to match <!-- completed: 2026-11-03T04:51 -->
- [x] §6.7 *codex rules file*: single broad allow (§2.1), minimum set reduced to the `["cafleet", "message"]` allow, separate-file operator rules for confirmation <!-- completed: 2026-11-03T04:51 -->
- [x] §6.7 *opencode preset* and *Preset file format*: remove the Director-only deny description and the "every Director-only deny" wording (§2.2) <!-- completed: 2026-11-03T04:51 -->
- [x] §6.7 *Hermes availability preflight* and *Contract error strings*: the passthrough preflight per §2.4. Remove the `approvals` read and the deny-glob error string. `doctor` reuses the same read <!-- completed: 2026-11-03T04:51 -->
- [x] Grep SPEC for `approvals.deny`, `Director-only`, `member_runtime`, `prompt` rules and `HERMES_` names, and resolve every leftover reference <!-- completed: 2026-11-03T04:51 -->

### Step 2: docs/

- [x] `docs/docs/quickstart.md` configuration table: the hermes "setup installs" cell names only the skills and the member variables merged into `terminal.client_env_passthrough` <!-- completed: 2026-11-03T04:53 -->
- [x] `docs/docs/quickstart.md` *Other permission modes*: describe the `ask` entries as optional confirmation of the Director's explicit pane input. Members are covered by the guard <!-- completed: 2026-11-03T04:53 -->
- [x] `docs/docs/quickstart.md` *Codex*: the installed file allows every `cafleet` command, the minimum set is the `["cafleet", "message"]` allow, and the "Keep the installed file for a Director session" reasoning is updated <!-- completed: 2026-11-03T04:53 -->
- [x] `docs/docs/quickstart.md` *Hermes*: remove the deny-merge text and list. Keep the passthrough paragraph with `CAFLEET_MEMBER_ID`. State that `member create` refuses when a member name is missing. Add the §2.3 existing-installation instruction (CAFleet needs no `approvals.deny` entries; a `cafleet …` glob there blocks a Hermes Director; how to inspect and remove it; the six glob strings). Replace the doctor-row sentence with the `hermes` summary line <!-- completed: 2026-11-03T04:53 -->
- [x] `docs/docs/concepts/members.md` § Command routing: one paragraph saying that Director-only commands refuse inside a member process, that this is an accident guard and not a security boundary, and that members route such needs to the Director (link the SPEC guard section) <!-- completed: 2026-11-03T04:53 -->
- [x] `docs/docs/concepts/members.md` § Inspection and recovery: doctor's default summary groups members by state with the next command, and `--verbose`/`--json` give per-check detail (link the SPEC doctor section) <!-- completed: 2026-11-03T04:53 -->
- [x] Check `docs/docs/concepts/coding-agents.md`, `docs/docs/mixed-backend-team.md` and `docs/docs/index.md` for setup-write, preset-deny or doctor-layout statements, and update any that drift <!-- completed: 2026-11-03T04:53 -->

### Step 3: README.md

- [x] Confirm that README's pitch, install commands and section list are unaffected. Edit only if a drifted statement is found <!-- completed: 2026-11-03T04:53 -->

### Step 4: skills/

- [ ] `skills/cafleet/roles/member.md` § Identity and authority: add the §3 paragraph <!-- completed: -->
- [ ] `skills/cafleet/reference/cli.md`: add the guard (six commands, trigger, error string, exit 1) to the command facts. Rewrite the `setup`/`doctor` section: Hermes passthrough only with the new skip line, Codex/OpenCode preset facts, doctor summary/verbose/JSON, statuses with `warn` and exit rule. Error table: replace the deny-glob row with the passthrough preflight row and add the guard row <!-- completed: -->
- [ ] `skills/cafleet/reference/cli.md` § Environment variables: add the `CAFLEET_MEMBER_ID` row and name it in the forwarding sentence (§5) <!-- completed: -->
- [ ] `skills/cafleet/SKILL.md` § IDs: add the literal-ID clarification sentence (§5) <!-- completed: -->
- [ ] `skills/cafleet/reference/coding-agents.md` opencode notes: rewrite the "last matching permission rule … denies" row as "the broad `cafleet *` allow lets members run broker commands; Director-only commands refuse inside the member through the runner guard". Remove "and Director-only denies" from the `cafleet-ask` row <!-- completed: -->
- [ ] `skills/cafleet/reference/coding-agents.md` hermes notes: replace the setup deny-merge row with a row saying that a Hermes Director whose command returns `BLOCKED: … user-defined deny rule '<glob>'` asks the operator, through the Director's decision surface, to remove that CAFleet glob from `approvals.deny`. The passthrough row names `CAFLEET_MEMBER_ID` and the spawn precondition. Preserve the model freshness date <!-- completed: -->
- [ ] `skills/cafleet/roles/director.md`: the Hermes preflight sentence (missing passthrough names with `run 'cafleet setup --coding-agent hermes'`). The Recovery section reads doctor's member summary and its next command <!-- completed: -->
- [ ] Grep `skills/` for `approvals.deny`, `Director-only deny`, `prompt` rules, `member_runtime` and `hermes.approvals`, and resolve every leftover <!-- completed: -->

### Step 5: Project rules

- [ ] Confirm that no `.claude/rules/` convention changes. Edit only if one does <!-- completed: -->

### Step 6: Director-only command guard (code)

- [ ] `src/cli/mod.rs`: add `director_only_name(&Command) -> Option<&'static str>` for the six variants and `refuse_director_only(command: &Command, member_id: Option<OsString>) -> Result<(), CafleetError>` with the §1.2 message. Call it in `run()` right after `CliArgs::parse()` and before `Settings::from_env()`, passing `std::env::var_os("CAFLEET_MEMBER_ID")` <!-- completed: -->
- [ ] Tests in `src/cli/mod.rs`: each of the six parsed commands (`CliArgs::try_parse_from`) is refused with the exact message for the values `34`, `""` and a non-UTF-8 value. Each passes with `None`. A representative set of unguarded commands passes with the variable set: `message send`, `message poll`, `member list`, `member capture`, `member create`, `permission inspect`, `input show`, `event show`, `doctor` and `setup` <!-- completed: -->
- [ ] `src/cli/mod.rs`: make `run()` delegate to `run_args(args: impl IntoIterator<Item = OsString>)`, which runs `CliArgs::parse_from(args)`, the guard, `Settings::from_env()` and `dispatch`. Add the ordering test, an in-file re-executed child test named with `test_path!` and modelled on `isolated_cli_child`. The child reads its argv from a JSON test variable, calls `run_args`, prints `Error: {error}` and exits with `error.exit_code()`. For each of the six commands, the parent launches it with `Command::env` setting `CAFLEET_MEMBER_ID=34`, `CAFLEET_MAX_TEXT_LEN=bogus` (which `Settings::from_env()` rejects) and `CAFLEET_DATABASE_URL=sqlite:///<tempdir>/absent.db`. The process environment is never mutated in-process, because tests run in parallel. The parent asserts exit 1, stderr equal to the guard error rather than the `CAFLEET_MAX_TEXT_LEN must be a non-negative integer` error, empty stdout and an empty tempdir. A control launch without `CAFLEET_MEMBER_ID` asserts the settings error, which proves the rejected value is in effect <!-- completed: -->

### Step 7: Member environment (code)

- [ ] `src/coding_agent/mod.rs`: split `spawn_environment` into `forwarded_config` (validated) and the infallible `spawn_environment(database_url, member_id, forwarded_config)` of §1.1. Add `CAFLEET_MEMBER_ID` as the second `HERMES_CLIENT_ENV_PASSTHROUGH` entry <!-- completed: -->
- [ ] `src/cli/member.rs`: call `forwarded_config` before registration (validation errors unchanged), and build the environment inside the `create_registered` closure with `pending.member.member_id` <!-- completed: -->
- [ ] Update the test callers to the new API: `src/runtime/artifacts.rs` (`runner_descriptor_records_the_forwarded_config_location_in_order`), `src/runtime/process.rs` (`runner_real_pty_child_receives_the_forwarded_config_location`), and the `src/coding_agent/mod.rs` tests `spawn_environment_*` and `hermes_spawn_environment_forwards_hermes_home`. Split the latter so validation cases (invalid selected value, ignored invalid value of another backend, unset variable) test `forwarded_config` and ordering cases test `spawn_environment` <!-- completed: -->
- [ ] Tests: in `src/coding_agent/mod.rs`, the environment order for each backend with the config variable set and unset, and the passthrough constant including `CAFLEET_MEMBER_ID` after `CAFLEET_DATABASE_URL`. In `src/runtime/artifacts.rs`, the descriptor round-trips the three entries in order. In `src/runtime/process.rs`, the real PTY child prints `CAFLEET_MEMBER_ID`. In `src/cli/member.rs`, the spawn spec handed to the host carries the allocated member ID <!-- completed: -->

### Step 8: Hermes setup and preflight (code)

- [ ] `src/assets.rs`: remove the `approvals.deny` merge (`HERMES_DENY_LIST`, the deny branch, deny-specific messages and fakes). The passthrough merge runs step 1 (`hermes config path`) and owns the base-path failure form. Use the new skip line <!-- completed: -->
- [ ] `src/coding_agent/mod.rs`: remove `HERMES_DIRECTOR_ONLY_DENIES`, `hermes_approvals_preflight`, `hermes_approvals`, `hermes_approval_denies` and `SpawnProbe::hermes_approvals`. `ensure_available` for hermes applies `hermes_client_env_names(probe.hermes_client_env_passthrough(..))` <!-- completed: -->
- [ ] `src/runtime/system.rs`: remove `SystemProbe::hermes_approvals`. Update every test `SpawnProbe` implementation (`src/cli/setup.rs`, `src/coding_agent/mod.rs`, `src/cli/member.rs`) <!-- completed: -->
- [ ] Tests in `src/assets.rs`: setup issues no `approvals.deny` command (the fake records none), plain setup without hermes prints the new skip line, passthrough merge idempotence/append order/failure cases keep their coverage, and a `hermes config path` failure reports the resolved base <!-- completed: -->
- [ ] Tests in `src/coding_agent/mod.rs`: the hermes preflight reads passthrough once from the resolved home, accepts a complete list in any order among other entries, names every missing name in canonical order, and reports unreadable output with its cause. Remove the deny-equality and deny preflight tests <!-- completed: -->
- [ ] `src/cli/member.rs`: the hermes `member create` preflight failure test asserts the passthrough error before registration <!-- completed: -->

### Step 9: Presets (code assets)

- [ ] `presets/codex/cafleet.rules`: reduce to the single broad allow (§2.1) <!-- completed: -->
- [ ] `presets/opencode/cafleet.md` and `cafleet-ask.md`: remove the six denies and update the body paragraphs (§2.2) <!-- completed: -->
- [ ] Update every test that asserts preset content (grep `src/` for `cafleet.rules`, `cafleet-ask.md` and the deny strings), so it checks the new contract: the codex file equals the one-line allow, both OpenCode shell maps end with `"cafleet *": "allow"` and hold no `cafleet member`/`permission`/`input` entry, and the `cafleet-ask` frontmatter differs only in `description` and the `"*"` value <!-- completed: -->

### Step 10: Doctor summary (code)

- [ ] `src/runtime/control.rs`: `RuntimeDiagnosis` gains a typed `state` (the §4.4 tokens) and returns the reworded phrases. Probe-only classification (rows 1 and 5–10) stays here <!-- completed: -->
- [ ] `src/broker/fleets.rs`: replace `member_run_ids` with a registry read that returns `run_id, member_id, run status, cleanup_state, member status` in ascending `run_id` <!-- completed: -->
- [ ] `src/cli/setup.rs`: add `--verbose` to `DoctorArgs`. Remove `hermes_approval_checks`' deny row. Set `warn` on installation version mismatch and on no installation recorded. Run the registry pass, then the budgeted probe pass. A registry-pass failure produces the §4.4 failure entries instead of aborting. Build summary entries per §4.3–4.4, verbose rows per §4.5 and JSON per §4.6. Keep the exit rule <!-- completed: -->
- [ ] `src/cli/setup.rs`: add the ID-range formatter (ascending, de-duplicated, runs of three or more collapse) <!-- completed: -->
- [ ] Tests in `src/runtime/control.rs`: each probe outcome combination maps to its state and phrases (descriptor leftover, invalid paths, lock held with and without health, lock free, lock error, deadline before health) <!-- completed: -->
- [ ] Tests in `src/cli/setup.rs`: the ID-range formatter (`[]`, `[7]`, `[7,8]`, `[1..6,9..22]`, duplicates, unsorted input). Registry-pass rows 2–4. Member summary line order, counts, labels, next commands, `none running` and database-unavailable lines. Backend summary rules (configuration error, absent executable, version mismatch `warn` with the setup fix, preset error, passthrough error, mixed problems joined in order with a single fix suffix) <!-- completed: -->
- [ ] Tests in `src/cli/setup.rs`: the default text prints only summary lines, `--verbose` prints only check rows (including `ended` rows), and `--json` has the §4.6 key order, `member_id`/`state` on runtime checks and `state`/`member_ids` on member summaries. Exit status is 0 with `warn`, and 1 with `Error: doctor found errors` on any `error`. Update the existing hermes doctor tests to the single passthrough row <!-- completed: -->
- [ ] Tests in `src/cli/setup.rs` for failures: with a failing registry query, doctor still prints every other check, emits the §4.4 registry-failure summary line, check row and JSON entries, and exits 1. Inject the failure by passing the registry read to the member-check builder as a function argument, because a schema mutation would make the database incompatible and skip runtime checks altogether. A `check_failed` run prints `error member_runtime.<run_id>: member <id> could not be checked: <cause>` with no command suffix, and its summary line ends with ` — run 'cafleet doctor --verbose'` <!-- completed: -->

### Step 11: Validation

- [ ] Run `mise //:test` and `mise //:lint`. Both pass <!-- completed: -->
- [ ] Run `mise //docs:build`. It passes, and the rendered HTML of any changed table cell containing a literal pipe is intact <!-- completed: -->
- [ ] Final grep across `SPEC.md`, `docs/`, `README.md`, `skills/`, `presets/` and `src/` for `approvals.deny` (only the operator instruction, the Hermes Director note and Hermes's own enforcement description remain), `HERMES_DIRECTOR_ONLY_DENIES`, `hermes.approvals.deny` and `Director-only rules` (none remain) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-03 | Initial draft |
