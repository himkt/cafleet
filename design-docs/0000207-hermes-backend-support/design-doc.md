# Hermes Coding-Agent Backend Support

**Status**: Approved
**Progress**: 39/73 tasks complete
**Last Updated**: 2026-11-02

## Overview

Add Hermes Agent (`hermes`) as a CAFleet coding-agent backend, evaluated independently for the Director and Member roles against SPEC §6.7 Role requirements, and ship only the roles whose live evidence establishes every Eligible-tier requirement. In the same change, restate the Member launch-policy requirement through a backend-neutral `auto`/`manual` mode classification, and generalize the Director background-execution evidence method (the CAFleet stream probe) into the shared Role requirements and evaluation procedure.

## Success Criteria

- [ ] SPEC §6.7 Role requirements, `skills/cafleet/reference/coding-agents.md` § Role requirements, `docs/docs/contributing.md` § Extend a coding-agent backend and `.claude/rules/documentation-maintenance.md` state the reframed Member bar (classified default mode plus detection and request-bound answer coverage for every approval prompt it can raise) and the general CAFleet stream-probe method; the OpenCode verdict-change paragraphs point at the general procedure.
- [ ] Every backend's launch modes carry an `auto`/`manual` classification in SPEC §6.9, `skills/cafleet/roles/director.md` § Member launch policy, each backend section of `coding-agents.md`, `skills/cafleet/reference/cli.md` and `docs/docs/concepts/coding-agents.md`; the Claude, Codex and OpenCode verdicts are unchanged and the CLI contract is unchanged.
- [ ] This document's `## Role evaluation` table records, per requirement group, the Hermes native version `2026.9.24`, source and check date, and each verdict is `yes` only when the recorded evidence establishes every Eligible-tier requirement.
- [ ] When at least one role reaches `yes`, the Hermes backend ships: registry entry, `--coding-agent` choices, setup/doctor support, the `hermes` section in `coding-agents.md`, the SPEC Role eligibility rows and the matching docs. Both verdicts are recorded there. A Director `no` follows the OpenCode precedent: `fleet create` accepts `hermes`, and the Director role's ineligible-backend rule governs. A Member `no` ships no member launch path. Each failure is recorded here.
- [ ] When Member is `yes`: `cafleet member create --coding-agent hermes` launches `hermes chat --cli --source tool --in <cwd> -q <prompt>` under the CAFleet PTY runner with native `smart` approvals; the Hermes profile from `2026.9.24` detects the guardian ESCALATE and guardian-DENY owner-override prompts on tmux and herdr, and `permission respond` answers them with single-request allow/deny.
- [ ] When Member is `yes`: `cafleet member delete` verifies that the member's Hermes session work and every background process it registered have stopped before deregistration, and exits 1 with the member row retained when that stop cannot be verified.
- [ ] Every claimed Hermes detector, composer and answer capability is backed by real native PTY captures on tmux and herdr across the measured pane widths, plus positive and adversarial replay fixtures; `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` pass.

---

## Background

`design-docs/0000191-hermes-backend/design-doc.md` (Aborted) is prior evidence. Its user-relay decision governs this design: no per-member Hermes profiles, the operator's existing Hermes configuration, a fresh conversation per member and explicit ownership of running processes. Profile creation, staging, credential migration and profile deletion from 0000191 are not carried forward. 0000191 also observed that the ordinary classic `hermes chat --cli` path runs a gateway-backed agent, so a gateway-owned session can leave accepted work and background processes running after its pane is killed.

Static findings from the installed Hermes package (`/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4/libexec/lib/python3.14/site-packages`, abbreviated `<pkg>` below), checked 2026-11-02:

| Area | Finding | Source |
|---|---|---|
| Version | `hermes --version` prints `Hermes Agent vunknown (2026.9.24)` on its first line; the parenthesized value is `__release_date__ = "2026.9.24"`, unpadded. The current probe finds no SemVer token in that line. | `<pkg>/hermes_cli/_startup_fast.py`, `<pkg>/hermes_cli/__init__.py` |
| Goal lifecycle | Native `/goal` with `show`, `status`, `pause`, `resume`, `clear`, `wait <pid>`, `unwait` and `gate add/remove/clear` | `<pkg>/hermes_cli/commands.py`, `<pkg>/hermes_cli/goal_command.py` |
| Background execution | The terminal tool accepts `background=true` with `notify=true` (`notify_on_complete` is an accepted legacy alias), and the completion notification is injected into the conversation. `watch_patterns` are rate-limited (15 s cooldown, 8 lifetime hits) and unsuitable for per-record listening. | `<pkg>/tools/terminal_tool.py`, `<pkg>/tools/process_registry_notifications.py`, `<pkg>/tools/process_registry.py`; the process tool is `process_manage` |
| Approvals | `approvals.mode` defaults to `smart`, with `timeout: 300`. Smart sends only pattern-detected dangerous commands to the guardian LLM; other commands run unreviewed. On the CLI, ESCALATE opens the full approval menu and a guardian DENY opens an owner-override menu limited to once/deny. Hardline floors, the sudo guard and `approvals.deny` globs block before any approval layer. `approvals.mode: off` disables approval prompts. | `<pkg>/tools/approval.py`, `<pkg>/tools/approval_prompt.py`, `<pkg>/tools/approval_floors.py`, `<pkg>/hermes_cli/config_defaults.py` |
| Approval gates | Three gates reach a human: command, `execute_code` and plugin-escalated action/protected write | `<pkg>/tools/approval.py` (`_COMMAND_GATE`, `_EXECUTE_CODE_GATE`, `_ACTION_GATE`) |
| Seeded query | On a real TTY, `-q` seeds an interactive session with a literal first turn; it does not set the single-query approval context | `hermes chat --help`, `<pkg>/hermes_cli/cli_single_query.py` |
| Background-process registry | Persisted to `<HERMES_HOME>/processes.json` with `pid`, `host_start_time` and `session_key` | `<pkg>/tools/process_registry_checkpoint.py` |
| Launch flags | `--reasoning none\|minimal\|low\|medium\|high\|xhigh\|max\|ultra`, `-m/--model`, `--provider`, `-s/--skills`, `--source`, `--in`, `--pass-session-id`. There is no `--name` analog. No flag selects an approval mode. | `hermes chat --help` |
| Configuration access | `hermes config get [--json] <key>` prints the resolved value from `load_config()`, which merges built-in defaults, `config.yaml` and the managed scope, with environment expansion. `hermes config set <key> <value>` parses list literals with `yaml.safe_load` and writes through `atomic_roundtrip_yaml_save`, a ruamel round-trip merge that preserves comments, key order, quotes and untouched nodes and replaces the file atomically. Managed keys are refused. | `<pkg>/hermes_cli/config.py` (`set_config_value`, `_coerce_config_set_value`, `load_config`), `<pkg>/utils.py` |

The SQLite `member_runs.backend` column carries `CHECK(backend IN ('claude','codex','opencode'))`. The `cafleet_v9.db` format family has been released (tag `0.30.3` contains `V1__baseline.sql`), so adding a backend to that check needs a `V2` successor migration.

None of the live evidence (stream probes, runner acceptance on tmux and herdr, prompt captures) exists yet. Both verdicts start at `no`.

---

## Role evaluation

| Role | Eligible | Evidence |
|---|---|---|
| Director | `yes` | Goal lifecycle: native `/goal` lifecycle controls in installed source and in-session `/help goal`, 2026.9.24, checked 2026-11-02. Background execution: CAFleet stream probe with `terminal` `background=true`, `notify=true` finite `wait` renewed per completion, consumed at turn boundaries without user input (runtime-evidence.md § 4.4 runs 2–3), 2026.9.24, checked 2026-11-02. |
| Member | `no` | PTY lifecycle, coverage declaration and coverage of the default `smart` mode's approval prompts: not established for 2026.9.24, checked 2026-11-02. |

Step 4 and Step 7 replace these rows with the recorded evidence. Each Evidence cell keeps the shape of SPEC § Role eligibility: source, native version and `checked YYYY-MM-DD` per requirement group, in at most two sentences. A Member `yes` names its bundled profile ("the Hermes profile from `2026.9.24`").

---

## Specification

### Scope and release gates

The work splits into verdict-independent changes and Hermes changes, which are gated per role:

| Work | Gate | Effect of a failed gate |
|---|---|---|
| Member bar reframing, mode classification, stream-probe generalization (Step 2) | None; ships regardless of Hermes verdicts | — |
| Hermes contract documentation, registry entry, config dir, setup/doctor, launch argv, version probe, policy catalog, `V2` migration (Steps 3–5) | Built on the feature branch to gather live evidence. Ships only when at least one role is `yes`. | Both roles `no`: Step 9 removes every Hermes surface from the branch before merge and keeps the generic changes plus this document's record |
| Director surface: Director runtime bindings and the native reception recipe (Steps 4 and 9) | Director `yes` | Director `no`: the `hermes` section records Director `no` and has no reception recipe. `fleet create --coding-agent hermes` stays accepted, because `fleet create` and `member create` share one `CodingAgent` choice set (`cafleet/src/cli/fleet.rs`). The Director-`no` rule at the top of `skills/cafleet/roles/director.md` governs, as it does for OpenCode. |
| Member surface: Hermes member launch path, bundled profile, deletion settlement, `V2` migration (Steps 5, 6, 7 and 9) | Member `yes` | Member `no`: Step 9 removes the Hermes member launch path, profile, fixtures, settlement and `V2` migration from the branch, and keeps the Hermes registry entry only when Director is `yes`. With Director `yes` and Member `no`, `member create --coding-agent hermes` fails before registration with `Error: hermes is not Member-eligible; see Role eligibility` (exit 1). The refusal also applies when the member inherits `hermes` from the fleet. `skills/cafleet/roles/director.md` § Model selection step 3 skips a fleet backend whose Member verdict is `no`: before spawning, the Director relays an operator choice of a Member-eligible backend via `{decision_surface}`, then passes it as an explicit `--coding-agent`. |

Evidence gathering needs CAFleet to launch Hermes under its runner, so the launch code exists on the feature branch before the verdicts. A role ships only after its Role evaluation row reads `yes`. Paid model calls through the operator's configured default Hermes model are authorized for this document's execution. Evidence files live in this task folder, which the repository's `.gitignore` keeps out of version control apart from `design-doc.md`.

### Member launch-mode classification

Each backend's native launch modes are classified at the documentation/catalog level. `--permission-mode` values stay backend-native, and the CLI contract, argv and run-record fields are unchanged.

| Class | Definition |
|---|---|
| `auto` | Native automatic review, or a fixed allow/deny policy. A command the policy does not settle goes to the automatic reviewer, or is allowed or denied without a native prompt. An `auto` mode may still raise an approval prompt when its reviewer escalates. |
| `manual` | Native interactive approval: the backend asks a human on its native surface for operations its rules do not settle. |

Every backend has one default (omitted) mode. A backend that offers an `auto` mode uses one as its default. When an approval prompt appears in a member pane, the runner's bundled profile detects it as `interaction_opened`, and the Director answers it through the existing runtime-event procedure (`permission inspect` / `permission respond`, single-request allow or deny).

The classification below was checked against `CodingAgent::policy_choices`/`default_policy` in `cafleet/src/coding_agent/mod.rs` and SPEC §6.9 Launch policy and capability registry:

| Backend | `auto` modes (default first) | `manual` modes |
|---|---|---|
| `claude` | `auto` (default); `dontAsk`; `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` |
| `codex` | `approve-for-me` (default); `never` | `on-request` |
| `opencode` | `cafleet` (default) | `cafleet-ask` |
| `hermes` | `smart` (default) | `manual` |

Claude `dontAsk` denies tools that are not pre-approved without prompting, `bypassPermissions` allows without prompting, and Codex `never` returns failures without asking. Each is therefore a fixed policy and classified `auto`. Claude `acceptEdits` and `plan` still prompt, so they are `manual`. The user decided to classify by this definition (§ User decisions, U2).

### Reframed Role requirements

Replace the Member Eligible row and the dependent sentences in SPEC §6.7 Role requirements with:

| Role | Tier | Required backend capability |
|---|---|---|
| Director | Eligible | Native goal lifecycle controls, such as a goal command, and background execution while supervision continues, established by a CAFleet stream probe. Both capabilities are required. |
| Member | Eligible | A runner-compatible PTY lifecycle and an explicit detection-coverage declaration. A classified default launch mode (`auto` whenever the backend offers one) with approval-prompt coverage: the bundled profile for the evaluated native version detects every approval form the default mode renders and declares request-bound allow and deny answers for it, at every pane geometry outside its recorded layout limits. A default mode that renders no approval form meets this without request rules. |
| Member | Bundled support | A native version the version probe parses, selecting a bundled profile that declares detection, automatic review annotations, composer plans and request-bound answers independently from native evidence. Capabilities beyond approval-prompt coverage are optional, and an undeclared one is `unknown` coverage or unsupported. |

**Coverage definitions.** An *approval form* is a native permission request that blocks the member until someone answers it. Question, trust, update and startup dialogs are separate interaction classes outside approval-prompt coverage, whether or not they have answer plans. A *recorded layout limit* is a profile-documented rendering in which no request rule matches, such as a pane narrower than the profile's `W_min`, a wrapped or clipped operation, or a geometry that was never captured. A form rendered inside a recorded limit produces no `interaction_opened` event and is answered manually on the native surface.

Replace the SPEC.md §6.7 verdict sentence "bundled-support capabilities never change a verdict" with: "Approval-prompt coverage of the default mode counts toward the Member verdict; the other bundled-support capabilities never change a verdict." A run of a Member-eligible backend whose version probe fails, or whose version selects no profile, is still a supported launch. It records `coverage=unknown`, ordinary generated input uses delivery fallback, and any approval form it shows is answered manually on the native surface. The verdict is evaluated against the profile selected for the recorded native version and carries forward to newer versions under the existing re-check rule.

Replace "Interactive native policies remain explicit `--permission-mode` choices …; an interactive-only backend is not Member-eligible" with: "Every launch mode other than the default is an explicit `--permission-mode` choice under §6.9 Launch policy and capability registry, which records each mode's `auto`/`manual` class. A backend whose default mode renders an approval form that its bundled profile neither covers nor records as a layout limit is not Member-eligible." The remaining sentences of that paragraph are unchanged.

`skills/cafleet/reference/coding-agents.md` § Role requirements carries the same bar and coverage definitions in one paragraph. `docs/docs/contributing.md` § Extend a coding-agent backend step 2 names the bar. Step 4 makes detection plus request-bound answers for the default mode's approval forms a required claim, with recorded layout limits, rather than an optional one.

Under the new wording each existing verdict is unchanged. The table was checked against SPEC §6.9 Bundled profiles:

| Backend | Default mode and class | Approval forms the default mode renders | Coverage and recorded limits | Member verdict |
|---|---|---|---|---|
| `claude` | `auto`, `auto` | The `permissions.ask` Bash form and the read-outside-working-directories dialog (SPEC §6.9 Claude profile) | `claude-auto-ask-permission` and `claude-read-outside-permission`, each with tested allow-once and deny-once answers. Recorded limit: both forms were captured at 120 columns only, and wrapped, boxed or narrower renderings are answered manually. The `Teach auto mode about your environment?` dialog is a `question` interaction (`claude-auto-startup`) outside approval-prompt coverage. | `yes`, unchanged |
| `codex` | `approve-for-me`, `auto` | None. Codex 0.163.0 routes every escalation to its automatic reviewer and shows no native form under `approve-for-me` (SPEC §6.9). | Met without request rules. The profile's single-line-command limit applies to the explicit `on-request` mode only. | `yes`, unchanged |
| `opencode` | `cafleet`, `auto` | None; the preset resolves every shell check to allow or deny | Met without request rules | `yes`, unchanged |

Step 2 re-checks each row against the bundled profile declarations in SPEC §6.9 when the wording is written.

### CAFleet stream probe

SPEC §6.7 Role requirements gains this general definition, mirrored in `coding-agents.md` § Role requirements and in `contributing.md` step 2:

> Background execution while supervision continues is established by a CAFleet stream probe on an installed native version. The candidate Director starts a CAFleet receiver (`cafleet message listen` or a finite `cafleet message wait`) through its native background-execution facility, then continues unrelated work in the same session. The probe passes when broker records that another member sends during that work reach the Director and are consumed without user input, during the work or at the session's next turn boundary. That can happen as streamed records, or as a completion-notified finite receive that the Director renews after each completion. Record the native mechanism, version and check date.

A completion-notified finite receive renewed per completion establishes background execution. Per-record reception remains a property of a backend's native reception recipe, and "background execution alone establishes no unsolicited wake or per-record reception guarantee" stays.

The OpenCode verdict-change paragraph in SPEC §6.7 Role eligibility becomes: "To change a Director verdict, a maintainer re-runs the Director requirements in § Role requirements, including the CAFleet stream probe, for an installed native version; records the version, source and date in this table and in the installed CAFleet skill's backend section; then adds a native reception recipe or states the manual `poll` / finite `wait` reception limit." `.claude/rules/documentation-maintenance.md` § OpenCode Director verdict changes becomes a backend-neutral § Director verdict changes that points at SPEC § Role requirements.

Stream-probe procedure (also used in Step 4):

1. Create an isolated database (`CAFLEET_DATABASE_URL=sqlite:///<BASE>/probe/director.db`) and run `cafleet setup` against it.
2. In a tmux pane, start the candidate Director session and register it with `cafleet fleet create --coding-agent <backend>`.
3. Have the Director spawn one probe member on any Member-eligible backend, with a spawn prompt that sends `ready` and then ends its turn without further broker traffic. The probe member's ID is the sender identity.
4. After the Director consumes the ready, have it start the receiver through its native background facility, then give it an unrelated multi-step task (for example: summarize three named files).
5. The maintainer sends the probe records deterministically from a separate shell at 20 s, 60 s and 120 s after the task prompt: `cafleet message send --from-member-id <probe-id> --to-member-id <director-id> "stream-probe 1"` (then `2`, `3`). Record each printed message ID and timestamp.
6. Pass: each of the three message IDs is ACKed by the Director (`cafleet message show <message-id> --json` shows the `completed` state) during the unrelated task or at its next turn boundary, with no user input after the task prompt, and a finite receiver is renewed after each completion. Capture the Director pane and the three message records.
7. Record the backend, native version, mechanism (tool call and flags), message IDs, send/ACK timestamps and pass/fail in the evidence file.

### Hermes launch contract

| Input | Hermes behavior |
|---|---|
| `--coding-agent hermes` | Registry entry `hermes`, binary `hermes` |
| Spawn argv (omitted model/effort) | `["hermes", "chat", "--cli", "--source", "tool", "--in", <absolute-cwd>, "-q", <prompt>]` |
| `--model VALUE` | Passes through verbatim as `["--model", VALUE]`, immediately before `-q`. No `--provider` is ever emitted; the operator's configuration selects the provider. Hermes rejects unknown models. |
| Omitted `--model` | No model tokens; Hermes uses the operator's configured default (currently provider `nous`, model `openai/gpt-6-astra`) |
| `--effort LEVEL` | Validated against `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra`; forwarded as `["--reasoning", LEVEL]` after the model tokens. Rejection: `Error: --effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got '<value>').` (exit 2) |
| `display_name` | Ignored; no `--name` analog |
| Permission argv | None. The launch policy is the operator configuration's `approvals.mode`, which must be `smart` (preflight below). |
| Fresh conversation | Every spawn starts a new session; argv never contains `--resume`, `--continue` or `--create-if-missing`. `--source tool` keeps member sessions out of the operator's default session lists. |

Full argv shape for SPEC §6.7 Per-backend `build_spawn_argv`:

```
["hermes", "chat", "--cli", "--source", "tool", "--in", <absolute-cwd>]
  (+ ["--model", <model>]       if model is not None)
  (+ ["--reasoning", <effort>]  if effort is not None)
  (+ ["-q", <prompt>])          # prompt via flag, TWO tokens
```

`<absolute-cwd>` is the member's resolved working directory (SPEC §6.5 Pane spawn working directory). `build_spawn_argv` gains the cwd parameter for every backend; the other backends ignore it.

**Configuration directory.** The variable is `HERMES_HOME`. The base when set is `$HERMES_HOME`, and `~/.hermes` when unset. The skills dir is `<base>/skills` and there is no preset target. The recorded path identity is the resolved base. Validation follows the existing absolute-path rule (`Error: HERMES_HOME must be an absolute path (got '<value>')`).

**Director-only deny rules (Decision U1, decided).** CAFleet requires these six `approvals.deny` globs in every Hermes configuration that runs members. They match the Director-owned denies in `presets/opencode/cafleet.md`:

```
cafleet member prompt *
cafleet member exec *
cafleet member press *
cafleet member recover *
cafleet permission respond *
cafleet input reconcile *
```

One Rust constant holds this list. A unit test asserts that the OpenCode `cafleet` preset denies exactly the same command set, so the two backends cannot drift apart.

**Setup.** `cafleet setup --coding-agent hermes` installs the embedded skills into `<base>/skills` and prints `hermes: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>`. It then merges the six globs into the resolved Hermes configuration and touches nothing else under the Hermes home:

1. Run `hermes config path` with `HERMES_HOME` set to the resolved base; its trimmed stdout is `<config path>`. Hermes applies a sticky `<base>/active_profile` to every command, so this path, the merge, the preflight and the member's own `hermes chat` all name the same file.
2. Run `hermes config get --json approvals.deny` with `HERMES_HOME` set to the resolved base. Its value must be a JSON array of strings; anything else fails.
3. Compute the merged list: every existing entry in its existing order, then each missing CAFleet glob in the order above. Comparison is exact string equality.
4. When nothing is missing, print `hermes: approvals.deny already holds CAFleet's Director-only rules -> <config path>` and write nothing.
5. Otherwise run `hermes config set approvals.deny '<merged list as compact JSON>'` and print `hermes: added approvals.deny rules -> <config path>: <glob>, <glob>, …`, naming only the added globs.
6. Re-run step 2 and require every CAFleet glob in the result. A failed command or a missing glob fails setup for `hermes` with `failed to install hermes approval rules into <config path>: <cause>`, and no `asset_installs` row is recorded. When step 1 itself fails, `<config path>` in that error is the resolved Hermes base (`$HERMES_HOME`, else `~/.hermes`), for example `failed to install hermes approval rules into /root/.hermes: binary hermes not found on PATH`.

Mechanism choice: Hermes's own writer performs the edit. It is the reader the runtime uses, it preserves comments, key order, quotes and untouched nodes through ruamel round-trip, it replaces the file atomically, and it refuses managed keys. CAFleet therefore adds no YAML dependency and never parses `config.yaml`. Step 1 observed on a commented fixture that existing deny entries keep their comments and quoting, and that appended entries are written unquoted; SPEC §6.3 Asset installation states this. Setup stays offline and runs only the local `hermes` binary, which must be on `PATH` for `--coding-agent hermes`. Its absence fails setup with the existing `binary hermes not found on PATH` cause. Plain `cafleet setup` (no `--coding-agent`) with `hermes` absent from `PATH` installs the Hermes skills, records the `asset_installs` row, skips the merge and prints `hermes: skipped approvals.deny rules (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes`.

**Availability preflight** (`ensure_available`, after the existing config-dir and PATH checks). Run `hermes config get --json approvals` once, with null stdin, `HERMES_HOME` set as the member will have it, captured output bounded like the version probe and a 2-second deadline inside the existing creation budget. The command took about 0.5 s on 2026-11-02, and Step 1 records the measurement. The result is the effective merged configuration, including the managed scope.

| Condition | Result (exit 1, before registration) |
|---|---|
| Command fails, times out or prints anything other than a JSON object | `Error: cannot read hermes configuration through 'hermes config get': <cause>` |
| `mode` differs from the run's launch mode (`smart` when omitted) | `Error: hermes approvals.mode is '<value>'; this member's launch mode requires '<mode>'` |
| `deny` lacks any CAFleet glob | `Error: hermes approvals.deny lacks CAFleet's Director-only rules: <glob>, …; run 'cafleet setup --coding-agent hermes'` |

`cafleet doctor` runs the same command read-only and reports the same conditions as `hermes.approvals.mode` and `hermes.approvals.deny` rows after `hermes.executable`, with the same corrective command. `hermes.configuration` stays the config-dir row every backend has. With the executable absent, both rows report `info` with `unavailable: executable is absent`, as the `opencode.preset` rows do.

**Effective configuration owner.** Background records that ordinary `hermes chat --cli` runs a gateway-backed agent. If a gateway process evaluates approvals and runs tools for the member's session, the effective `approvals.mode`, `approvals.deny` and tool environment belong to that process. Step 6 establishes which process evaluates approvals and runs terminal tools for a member session, and the outcome is fixed in advance:

| Step 6 finding | Specification outcome |
|---|---|
| The pane's `hermes` process evaluates approvals and runs tools | The preflight and child-environment rules above govern; no further check |
| A gateway does, one per `HERMES_HOME` (Step 6 observed it) | The preflight and member-environment rules govern; the gateway serves every session of that home, so the per-session tool environment comes from `terminal.client_env_passthrough` (§ Member environment) |
| A gateway does, and its home or environment cannot be established | Member is `no`, with that failure recorded |

**Member environment.** SPEC §6.3 Member environment gains a `hermes` row forwarding `HERMES_HOME`. One gateway per Hermes home runs every session's terminal tools with its own launch environment, so a member's tool sees its own `CAFLEET_*` and multiplexer values only for names in `terminal.client_env_passthrough` (Step 6). `cafleet setup --coding-agent hermes` merges that list exactly like `approvals.deny`: existing entries first, then each missing name among every `CAFLEET_*` name CAFleet sets in a member environment and the multiplexer names `TMUX`, `TMUX_PANE` and the runner's `HERDR_*` names, printing `hermes: added terminal.client_env_passthrough names -> <config path>: <name>, …` or `hermes: terminal.client_env_passthrough already holds CAFleet's member names -> <config path>`. Doctor row `hermes.client_env_passthrough` and the availability preflight report a missing name with `Error: hermes terminal.client_env_passthrough lacks CAFleet's member names: <name>, …; run 'cafleet setup --coding-agent hermes'`. Background terminal work runs through a login shell that rebuilds `PATH`, so members invoke CAFleet through an absolute executable path in background commands.

**Version probe.** SPEC §6.9 Native version probe gains a backend-specific token rule. For `hermes`, the first stdout line must match `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$`. The captured value must parse as canonical SemVer and becomes `native_version`; `Hermes Agent vunknown (2026.9.24)` records `2026.9.24`. A non-matching line is a probe failure, which records null and `coverage=unknown` with the existing warning. The general one-token rule stays for the other backends. The package directory version `0.22.4` is recorded only as metadata in evidence.

**Launch-policy catalog** (SPEC §6.9 table row): Hermes choices are `smart` (default, class `auto`) and `manual` (class `manual`), version-independent like OpenCode's: `(Self::Hermes, _) => &["smart", "manual"]`. Neither selects native argv; the preflight requires the effective `approvals.mode` to equal the run's launch mode. Any other value fails with `--permission-mode for hermes must be one of smart, manual (got '<value>')`. The run record for an omitted selection is `requested_mode` `null`, `resolved_mode` `smart`, `mode_source` `default` and `permission_argv_json` `[]`; an explicit `manual` records `requested_mode` and `resolved_mode` `manual` and `mode_source` `explicit`.

**Model selection policy.** Hermes has no maintained model catalog. Its Role defaults set `{reviewer_model}` and `{other_model}` to `—`, meaning omit `--model`, and the operator's configured default applies. Their effort placeholders are also `—`, meaning omit `--effort`. The Director selects `hermes` only when the user names the backend or the fleet's Director runs on Hermes, never through model-name inference. A provider-prefixed model on a Hermes-pinned member passes verbatim, and inference row 1 (`/` → `opencode`) applies only when no backend is named. Cost efficiency mode on a Hermes member relays an operator choice, because there are no catalog prices. `.claude/skills/cafleet-model-list-refresh/SKILL.md` treats a backend whose catalog is "no maintained catalog" as outside the refresh scope.

### Hermes bundled profile

The Hermes profile from `2026.9.24` applies to `hermes chat --cli` under `smart`. Each capability ships with status `tested` only after its captures and fixtures exist. A capability without them ships as `unknown`. Approval-prompt coverage, as defined in § Reframed Role requirements, is mandatory for Member `yes`: detection plus allow-once and deny answers for every approval form `smart` renders, at every geometry outside the recorded layout limits (`W_min`, clipped or wrapped operations, uncaptured geometries).

| Capability | Required for Member `yes` | Target declaration |
|---|---|---|
| Detection: guardian ESCALATE prompt (command gate) | yes | Request rule `hermes-approval-escalate` on the live prompt region: header, command/description rows, and the menu with once/session/always/deny choices as rendered |
| Detection: guardian-DENY owner-override prompt (command gate) | yes | Request rule `hermes-approval-override` for the once/deny-only menu |
| Detection: `execute_code` and action/protected-write gate prompts | yes, for each one `smart` reaches in Step 6 captures | One request rule per distinct rendered form; a gate shown to share a form reuses its rule |
| Detection: other native forms (first-run setup, hook consent, update or trust notices, `clarify` questions) | Each observed form gets a rule with its own interaction class (`question`, trust, update) and no answer plan | Rules named `hermes-<form>` |
| Allow answer | yes | Gated stages that select "once" only: the typed choice `o` plus the measured submit key, or the measured focus/keys of the rendered menu |
| Deny answer | yes | Gated stages that select "deny" only |
| Empty-composer observation | Required for composer plans | The idle classic REPL prompt line with the cursor after its prefix |
| Prompt submission | Claimed when measured | Empty prefix plus the measured submit key, as separate literal and submit stages |
| Native-exec submission | Claimed when measured | Prefix `! ` plus the measured submit key; Step 6 verifies that the classic REPL runs `!` input locally (`handle_bang_shell`) with zero model turns |
| Automatic review annotations | Optional | Claimed only when captures show a stable guardian refusal or review-unavailable notice |

The answer contract:

- The Director answers single-request choices only: allow once or deny. Session and always choices have no choice entry or answer plan, so focus on them satisfies no gate.
- The Director may answer allow or deny on both the ESCALATE and the guardian-DENY override prompt. Hermes itself limits an override to once/deny.
- An unanswered prompt fails closed after `approvals.timeout` (default 300 s). Hermes then prints its timeout notice and returns a BLOCKED result to the model. The form disappears, so the runner closes the episode with the existing `not_visible` close reason, and no schema change is needed. The timeout notice is continuation evidence for the Director, recorded in the profile's decision-evidence table, not a bundled rule. A later bound response fails request-token revalidation.
- When the guardian LLM is unavailable, Hermes falls back to its human prompt. That prompt is the same form and is detected by the same rule; Step 6 captures it when the fixture can reach it.

**Capture matrix.** Each form the default mode can raise is captured on tmux and on herdr at pane widths 120, 80, 57 and 40 columns, 36 rows. Step 6 adds the narrowest width at which the menu still fits on one row as `W_min`, and records the layout breaks it observes. Each capture is a native 36-row screen converted through the existing capture-to-fixture path into `cafleet/src/coding_agent/native/hermes.json`, with reviewed privacy masks only for account, model, plan or home-path rows. Every rule region lies in unmasked rows.

**Replay fixtures.** These are positive and adversarial fixtures in the detector's `#[cfg(test)] mod tests`, with inputs written inline:

| Fixture class | Required cases |
|---|---|
| Positive | Each request rule at each captured width and on both multiplexers; allow and deny pre-stage screens; continuation screens after each decision |
| Redraw | Prompt re-rendered after spinner/status updates; repeated identical prompts produce separate episodes |
| Wrapping | Long command wrapped by the terminal; command clipped or truncated by the form (no match, manual answer) |
| Fragmented bytes | ANSI sequences and multi-byte UTF-8 split across PTY reads |
| Alternate screen | Prompt drawn while the alternate screen is active, and after returning to the primary screen |
| Competing forms | Approval prompt while a `clarify` question or startup notice is visible; two prompts in sequence |
| Transcript false positives | The prompt text quoted in model output, in a `cat` of a log, and in scrollback after the prompt closed |
| Timeout | The screen after the `approvals.timeout` notice: no request rule matches, and the episode closes as `not_visible` |

### Member work ownership and deletion

Deletion stops the member's Hermes session work and every background process it registered, and verifies the stop before deregistration. An unverifiable stop makes `member delete` exit 1 and keeps the member row. CAFleet never stops or restarts the Hermes gateway, kills by process name or touches sessions it does not own.

Step 6 established that the member's turn and background processes run in the gateway and survive the death of the pane's `hermes chat` client, and that the session-scoped stop control is native input to the live client (Decision U4):

| Requirement | Mechanism |
|---|---|
| Durable identity of the member's Hermes session | The runner reads the launch banner `Session: <short id>` and resolves it to the unique full `local-<hex>` ID through `hermes sessions list --source tool`. The identity is stored in `member_runs.native_session_json`; no unique match fails the launch. |
| Stop of in-flight session work | While the client is alive, the runner submits Ctrl-C, then `/stop`, then `/exit` with the measured submit stages, before runner shutdown |
| Stop of registered background processes | `/stop` stops the session's registered processes |
| Verification | No `processes.json` entry whose owner is the session ID has a live PID with the recorded start time (`/proc/<pid>/stat` field 22 against `host_start_time`), and the client exited after printing `Session: <id>` |

Deletion order extends SPEC §6.3 `member delete` for `hermes` runs. Before runner shutdown, run the Hermes settlement within the existing deletion deadline. When the client is already gone, or verification fails, exit 1 with `Error: hermes work for member <ID> could not be verified as stopped: <cause>; member was not deleted.`, retain the runner's cleanup records and the member row, and let retry re-run settlement. The `hermes` notes document manual recovery for a dead client and that its in-flight turn runs to completion. Creation compensation (SPEC §6.3 Creation failure compensation) runs the same settlement when a run started.

### Director support

| Requirement / binding | Specification |
|---|---|
| Goal lifecycle | Native `/goal`, user-operated. CAFleet makes no goal lifecycle calls and preserves activation, pause, completion and gate/wait authority. Evidence comes from installed help, using an in-session `/goal` help or `show` capture plus the installed source. |
| Background execution | CAFleet stream probe with the Hermes terminal tool: `background=true`, `notify=true`, command `cafleet message wait <director-id> --timeout 300`, renewed after each completion notification |
| `{decision_surface}` | The `clarify` tool when Step 4 shows it records a choice; otherwise the documented default (a Director-relayed operator message) |
| `{pane_title}` | No `--name` analog |
| `{skill_loader}` | Reading each requested skill core, own backend section and role-required references by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` (spawn flag `--effort <level>`, forwarded as `--reasoning <level>`) |
| `{permission_flags}` | No permission argv; native `approvals.mode: smart` from the operator configuration (`auto` class, default) with hardline floors, the sudo guard and `approvals.deny` enforced |

When Director is `yes`, `coding-agents.md` gains `#### Hermes native Director reception`. The Director starts one background terminal process running the finite `wait` with completion notification, retaining its process handle, Director ID and unfinished actions. On each completion it reads the process output, runs `cafleet message poll <director-id> --json`, reconciles replayed IDs, records deferred work, ACKs consumption and renews one receiver while delegated work remains. Retiring the receiver uses `process_manage` with `action=kill` on the retained `session_id`, verified as exited. The exact tool names and arguments are copied from the Step 4 probe. A host with the terminal background facility unavailable or denied discloses that limit and uses manual `poll`/finite `wait`.

### `coding-agents.md` hermes section

The `## hermes` section follows the Template in `.claude/rules/documentation-maintenance.md` in subsection order: Runtime bindings (the table above), Role defaults (all four placeholders `—`), Model catalog ("No maintained catalog" plus the pass-through rule and the operator-default source), Note → applies at, `#### Hermes native Director reception` (Director `yes` only) and Role eligibility. Note rows bind:

| Note | Applies at |
|---|---|
| Default `smart` (`auto` class): non-dangerous commands run unreviewed; guardian-reviewed commands may open an ESCALATE or owner-override prompt, which the Director answers allow-once or deny through the runtime procedure; an unanswered prompt fails closed at `approvals.timeout`. Hardline floors, the sudo guard and `approvals.deny` block before review and route like any refusal. | `{permission_flags}`; member routing; runtime procedure |
| Setup installs the six Director-only `approvals.deny` globs, and `member create` refuses a configuration that lacks any of them. A matching command is blocked before review and routes like any refusal. | `{permission_flags}`; member routing; setup |
| A gateway-backed session can keep work running after its pane closes; deletion settles it (see `member delete`) | Director shutdown and recovery |
| Native shell input through leading `!` runs locally in the classic REPL; the Director authorizes routed native input under its host rules | Director-side dispatch |
| The member's terminal tool needs `CAFLEET_DATABASE_URL` (prerequisite from Step 6) | Member startup; setup |

### Schema

Step 5 adds `cafleet/migrations/V2__hermes_backend.sql` on the feature branch, and Step 9 removes it when Member stays `no`. It rebuilds `member_runs` with `CHECK(backend IN ('claude','codex','opencode','hermes'))` and adds nullable `native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object'))`. The rebuild preserves every column, constraint, index and referencing foreign key, following SQLite's table-rebuild procedure inside the migration transaction. SPEC §8 Database schema documents the new shape. The chain guard in `cafleet/src/db.rs` accepts V2, and a successor test migrates a V1 database with rows for each existing backend to V2, asserting preserved rows and the widened check.

### User decisions

| ID | Decision | Where specified |
|---|---|---|
| U1 | Missing Director-only deny globs block launch, and setup installs them. `cafleet setup --coding-agent hermes` appends only the missing globs through `hermes config set`, preserving existing entries, their order and the rest of the file. It is idempotent and reports what it added. `member create` and doctor fail or report when any glob is missing from the effective configuration, and name the corrective command. | § Hermes launch contract, Director-only deny rules, Setup and Availability preflight |
| U2 | Classify launch modes by definition: Claude `dontAsk` and `bypassPermissions` and Codex `never` are `auto`; Claude `acceptEdits` and `plan` are `manual` | § Member launch-mode classification |
| U3 | The CAFleet stream probe passes when records sent during the unrelated work are consumed without user input, during that work or at the session's next turn boundary | § CAFleet stream probe |
| U4 | Deletion stops a Hermes member through native input to its live client (Ctrl-C, `/stop`, `/exit`) and verifies the stop; a dead client makes `member delete` exit 1 with the row kept | § Member work ownership and deletion |
| U5 | Hermes launch modes are `smart` (default) and `manual`, matched against the effective `approvals.mode`; setup merges `terminal.client_env_passthrough`; skill self-edits are a documented note restored by setup | § Hermes launch contract |

The deny globs' effectiveness under `smart` is verified live, not assumed. Step 6 checks the following and records the outcome:

- A plain `cafleet member prompt …` is blocked.
- A leading-assignment form `CAFLEET_DATABASE_URL=… cafleet member press …` is blocked. The cafleet skill's one-shot isolation permits that form, and Hermes's `_deny_command_variants` projects command words.
- How the refusal renders in the member's pane and tool result, for routing.

If a permitted invocation form evades the six globs, the specification adds the glob forms that close it (for example a leading `*` variant) before Step 7 proceeds. If no glob set closes it, Member is `no` with that failure recorded.

### Documentation targets

Order per `.claude/rules/documentation-maintenance.md`: `SPEC.md`, `docs/`, `README.md`, `skills/`, then rules. Generic changes (Step 2) ship regardless of verdicts. Hermes changes (Steps 3 and 9) ship when at least one role is `yes`, record both verdicts and carry member-specific content only when Member is `yes`.

| File | Generic change (Step 2) | Hermes change |
|---|---|---|
| `SPEC.md` §6.7 Role requirements | Reframed Member row and paragraph; stream-probe definition | — |
| `SPEC.md` §6.7 Role eligibility | OpenCode paragraph becomes a general verdict-change paragraph | `hermes` Director and Member rows |
| `SPEC.md` §6.7 Interface, Per-backend argv, Model selection, Reasoning effort, Backend capabilities, Contract error strings, Host evidence | cwd parameter in `build_spawn_argv` | `hermes` entries |
| `SPEC.md` §6.3 Config-dir resolution, Asset installation, doctor check rows, `member delete`, Creation failure compensation | — | `HERMES_HOME` row. Asset installation states that setup for `hermes` edits the Hermes configuration's `approvals.deny` through `hermes config set` (merge rules, output lines, failure cause, comment caveat). Also the `hermes.approvals.mode` and `hermes.approvals.deny` doctor rows and the Hermes settlement. |
| `SPEC.md` §6.3 Member environment | — | `hermes` row forwarding `HERMES_HOME`; `terminal.client_env_passthrough` merge, doctor row and preflight check |
| `SPEC.md` §6.9 Launch policy and capability registry | `auto`/`manual` class column for every backend | `hermes` row; Native version probe Hermes rule; Hermes profile section and fixtures |
| `SPEC.md` §8 Database schema | — | `V2` shape |
| `docs/docs/concepts/coding-agents.md` | Default-mode classification sentence and table | `hermes` row; Hermes model/effort/backend-difference notes |
| `docs/docs/contributing.md` | Steps 2 and 4 reworded; stream-probe procedure | — |
| `docs/docs/quickstart.md`, `docs/docs/concepts/members.md`, `docs/docs/mixed-backend-team.md` | — | Hermes setup, prerequisite configuration and member notes where these pages enumerate backends per-backend |
| `README.md` | — | Only if its backend pitch or install commands change |
| `skills/cafleet/reference/coding-agents.md` | § Role requirements reframed; each backend's `{permission_flags}` binding and note names its default mode and class | `## hermes` section |
| `skills/cafleet/roles/director.md` § Member launch policy | Table restated as `Backend \| auto mode (default) \| manual mode(s)` | Hermes row; inference and cost-mode rules for Hermes |
| `skills/cafleet/reference/cli.md` | `--permission-mode` facts carry the classes | `hermes` choices, argv, effort and errors |
| `skills/cafleet/SKILL.md`, `skills/cafleet-design-doc/SKILL.md` | — | Backend references where they enumerate backends |
| `.claude/rules/documentation-maintenance.md` | Template requires each new backend's mode classification and a Role defaults `—` option for a backend without a maintained catalog; § Director verdict changes generalized | — |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | — | Hermes outside refresh scope |

### Evidence-driven documentation updates

Step 3 documents the Hermes contract as targeted. Before the final reinstall and validation, Step 9 rewrites these surfaces from `runtime-evidence.md`, so the shipped documentation describes measured results:

| Surface | Content taken from the evidence |
|---|---|
| SPEC §6.9 Hermes profile section | Final rule IDs, per-capability `tested`/`unknown` status, measured allow/deny key stages, `W_min` and layout limits, which gates share a form, decision-evidence lines |
| `coding-agents.md` `## hermes` notes | Effective-configuration owner, `terminal.env_passthrough` prerequisite, observed native forms, deny-glob refusal rendering |
| `docs/` prerequisite pages, doctor rows and their SPEC §6.3 entries | The passthrough prerequisite and the gateway check, when Step 6 shows they apply |

### Error contracts

| Condition | Result |
|---|---|
| Invalid `--effort` for hermes | Exit 2 with the effort error above, before any side effect |
| Explicit `--permission-mode` other than `smart` or `manual` | Exit 2: `--permission-mode for hermes must be one of smart, manual (got '<value>')` |
| `HERMES_HOME` empty or relative | Exit 1: `Error: HERMES_HOME must be an absolute path (got '<value>')` |
| `approvals.mode` differing from the launch mode, unreadable configuration, or `terminal.client_env_passthrough` lacking a CAFleet member name | Exit 1 with the preflight errors above, before registration |
| A CAFleet Director-only glob missing from `approvals.deny` | Exit 1 with the missing-glob error naming `cafleet setup --coding-agent hermes`, before registration |
| Setup cannot read or write `approvals.deny` | Setup exits 1 with `failed to install hermes approval rules into <config path>: <cause>`; no `asset_installs` row for `hermes` |
| `member create` selecting `hermes` (explicit or fleet-inherited) with Director `yes` and Member `no` | Exit 1: `Error: hermes is not Member-eligible; see Role eligibility`. The Director prevents it by relaying a Member-eligible backend choice before spawning. |
| Version line not matching the Hermes rule | Probe warning, `coverage=unknown`, launch continues |
| Known version below `2026.9.24` | Exit 1: `Error: unsupported native version for hermes: <v>; minimum supported version is 2026.9.24` |
| Hermes settlement unverifiable at deletion | Exit 1 with the settlement error above; member row kept |
| Approval prompt unanswered past `approvals.timeout` | The episode closes as `not_visible`; Hermes returns BLOCKED to the model; the member routes the refusal |

### Evidence record

`design-docs/0000207-hermes-backend-support/runtime-evidence.md` holds one section per Step 1, 4, 6 and 7 case. Raw captures live under `captures/<multiplexer>/<width>/`. Each record states:

- CAFleet source commit and installed version; Hermes `native_version` plus package `0.22.4`
- The operator's provider and model (redacted to the provider/model ID), multiplexer and version, terminal size and cwd
- The exact commands, observed output, broker IDs and timestamps
- Pass, fail or recorded limit

Paid calls use the operator's configured default model. Secrets and account identifiers are redacted before writing.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Record static Hermes evidence

- [x] Create `runtime-evidence.md` in this task folder with the record template from § Evidence record <!-- completed: 2026-11-02T06:23 -->
- [x] Record `hermes --version` output, package path/version and `__release_date__`, and confirm that the Hermes version rule yields `2026.9.24` <!-- completed: 2026-11-02T06:23 -->
- [x] Record which sources `load_config()` merges into `approvals.mode` and `approvals.deny` (defaults, `config.yaml`, managed scope, environment expansion, profile overlays). Confirm that `hermes config get --json approvals` reports the effective values, including the `smart` default for an absent key or file, and record its run time. <!-- completed: 2026-11-02T06:23 -->
- [x] On a fixture copy of a commented `config.yaml` in a temporary `HERMES_HOME`, run the setup merge sequence (`config get`, then `config set approvals.deny '<json>'`) twice. Record the preserved comments and order, the effect on comments attached to existing deny entries, the idempotent second run and the refusal for a managed key. <!-- completed: 2026-11-02T06:23 -->
- [x] Record the `/goal` command definition and handlers, the terminal background/notification parameters, the process-tool action list and the `clarify` tool schema, with source paths <!-- completed: 2026-11-02T06:23 -->

### Step 2: Generic documentation — Member bar, mode classification, stream probe

- [x] Verify each backend's mode classification against `policy_choices`/`default_policy` and SPEC §6.9, and verify the unchanged-verdict table against the bundled profile declarations <!-- completed: 2026-11-02T06:30 -->
- [x] `SPEC.md` §6.7 Role requirements: reframed Member Eligible row, replacement paragraph sentence, stream-probe definition <!-- completed: 2026-11-02T06:30 -->
- [x] `SPEC.md` §6.7 Role eligibility: replace the OpenCode verdict-change paragraph with the general paragraph <!-- completed: 2026-11-02T06:30 -->
- [x] `SPEC.md` §6.9 Launch policy and capability registry: add the class definition and an `auto`/`manual` classification for every existing backend's modes <!-- completed: 2026-11-02T06:30 -->
- [x] `docs/docs/concepts/coding-agents.md`: default-mode classification and the Director-answered approval-prompt flow <!-- completed: 2026-11-02T06:30 -->
- [x] `docs/docs/contributing.md` § Extend a coding-agent backend: steps 2 and 4 reworded, stream-probe procedure added <!-- completed: 2026-11-02T06:30 -->
- [x] `skills/cafleet/reference/coding-agents.md`: § Role requirements reframed with the stream probe; each existing backend's `{permission_flags}` binding and note name the default mode and its class <!-- completed: 2026-11-02T06:30 -->
- [x] `skills/cafleet/roles/director.md` § Member launch policy: table restated as `Backend | auto mode (default) | manual mode(s)` <!-- completed: 2026-11-02T06:30 -->
- [x] `skills/cafleet/reference/cli.md` § member create: permission choices annotated with classes <!-- completed: 2026-11-02T06:30 -->
- [x] `.claude/rules/documentation-maintenance.md`: Template requires the mode classification and allows Role defaults `—` without a maintained catalog; § OpenCode Director verdict changes generalized to § Director verdict changes <!-- completed: 2026-11-02T06:30 -->
- [x] Run `mise //docs:build` and confirm links and table cells render <!-- completed: 2026-11-02T06:30 -->

### Step 3: Hermes contract documentation (on the feature branch, verdicts `no`)

- [x] `SPEC.md` §6.3: config-dir row; Asset installation with the `hermes` skills line and the `approvals.deny` merge through `hermes config set` (merge rules, output lines, failure cause, comment caveat); Member environment `HERMES_HOME` row; `hermes.approvals.mode` and `hermes.approvals.deny` doctor rows; `member delete` and creation-compensation settlement <!-- completed: 2026-11-02T06:41 -->
- [x] `SPEC.md` §6.7: argv block, model/effort rows, backend capabilities, preflight and contract error strings, host-evidence paragraph; Role eligibility rows copied from § Role evaluation <!-- completed: 2026-11-02T06:41 -->
- [x] `SPEC.md` §6.9: `hermes` launch-policy row with class, Hermes version-probe rule, Hermes profile section as targeted (status per capability) <!-- completed: 2026-11-02T06:41 -->
- [x] `SPEC.md` §8: `member_runs` V2 shape <!-- completed: 2026-11-02T06:41 -->
- [x] `docs/` pages from § Documentation targets: Hermes rows, setup and prerequisite configuration (`approvals.mode: smart`, the setup-installed Director-only deny globs, `CAFLEET_DATABASE_URL` passthrough if required) <!-- completed: 2026-11-02T06:41 -->
- [x] `skills/cafleet/reference/coding-agents.md` `## hermes` section with Runtime bindings, Role defaults, Model catalog, Note → applies at and Role eligibility (`no`) <!-- completed: 2026-11-02T06:41 -->
- [x] `skills/cafleet/roles/director.md` Hermes launch-policy row, inference rule and cost-mode relay; `skills/cafleet/reference/cli.md` Hermes choices, argv, effort and errors <!-- completed: 2026-11-02T06:41 -->
- [x] `.claude/skills/cafleet-model-list-refresh/SKILL.md`: Hermes outside refresh scope <!-- completed: 2026-11-02T06:41 -->

### Step 4: Hermes registry, launch and Director evidence

- [x] Add `CodingAgent::Hermes` with name/binary `hermes`, config variable `HERMES_HOME`, default `~/.hermes`, skills dir `<base>/skills`, and add it to the `fleet`, `member` and `setup` CLI choices and asset installation <!-- completed: 2026-11-02T07:11 -->
- [x] Implement effort validation and `--reasoning` forwarding, verbatim `--model`, and the cwd parameter in `build_spawn_argv` for every backend. Argv unit tests per backend. <!-- completed: 2026-11-02T07:11 -->
- [x] Implement the Hermes version-probe rule with unit tests (matching line, `vunknown`, non-matching, several parentheses, non-SemVer date) <!-- completed: 2026-11-02T07:11 -->
- [x] Implement the version-independent launch-policy catalog (`smart` only, `permission_argv_json` `[]`), with unit tests <!-- completed: 2026-11-02T07:11 -->
- [x] Add the Director-only deny-glob constant, plus the unit test that it equals the OpenCode `cafleet` preset's Director-owned denies <!-- completed: 2026-11-02T07:11 -->
- [x] Implement the setup merge of `approvals.deny` through `hermes config get`/`config set`, with its output lines and failure cause. Unit tests use an injected command runner for the missing-some, missing-none, non-array, failed-set and post-write-verification cases. <!-- completed: 2026-11-02T07:11 -->
- [x] Implement the availability preflight through one bounded `hermes config get --json approvals` call (mode, deny globs, unreadable output), using serde_json with no YAML dependency. Unit tests cover each error string. <!-- completed: 2026-11-02T07:11 -->
- [x] Implement the Hermes doctor rows (`hermes.configuration` config dir, skills install, `hermes.approvals.mode`, `hermes.approvals.deny`) <!-- completed: 2026-11-02T07:11 -->
- [x] Install the branch build with `mise //cafleet:install` into the evidence environment, preserving any active fleet's installed runtime <!-- completed: 2026-11-02T09:26 -->
- [x] Director evidence: capture in-session `/goal` help/`show` output in a root Hermes session on tmux <!-- completed: 2026-11-02T09:26 -->
- [x] Director evidence: run the stream-probe procedure with a Hermes Director (`terminal` `background=true` + `notify=true`, finite `wait` renewed per completion) and record pass/fail <!-- completed: 2026-11-02T09:26 -->
- [x] Director evidence: verify `clarify` as `{decision_surface}` (records the chosen option) or keep the documented default <!-- completed: 2026-11-02T09:26 -->
- [x] Update § Role evaluation Director row with the recorded evidence and verdict <!-- completed: 2026-11-02T09:26 -->

### Step 5: Schema successor

- [x] Add `V2__hermes_backend.sql` (rebuild `member_runs` with the widened check and `native_session_json`) <!-- completed: 2026-11-02T07:11 -->
- [x] Add the successor test migrating a populated V1 database to V2 and asserting preserved rows, constraints, indexes, foreign keys and the new column; confirm the chain guard <!-- completed: 2026-11-02T07:11 -->

### Step 6: Member live evidence and captures

- [ ] Launch a Hermes member under the runner on tmux and on herdr; record literal first-turn delivery, ready, poll/show/ACK and a second assignment after the first answer <!-- completed: -->
- [ ] Establish which process evaluates approvals and runs terminal tools for a member session (pane `hermes` process or a gateway), and whether a serving gateway's `HERMES_HOME` and environment can be read. Apply the matching outcome from § Hermes launch contract, Effective configuration owner, including the gateway check or a Member `no`. <!-- completed: -->
- [ ] Verify that the member's terminal tool sees its own `CAFLEET_DATABASE_URL`, multiplexer variables and intended cwd through `terminal.client_env_passthrough`, with two concurrent members under one Hermes home <!-- completed: -->
- [ ] Verify that the six deny globs block, under `smart`, a plain `cafleet member prompt …`, a leading-assignment `CAFLEET_DATABASE_URL=… cafleet member press …` and the other four commands. Record how each refusal renders in the pane and tool result for routing, and extend the glob set or record Member `no` per § User decisions. <!-- completed: -->
- [ ] Capture the guardian ESCALATE prompt (command gate) at 120, 80, 57 and 40 columns on tmux and herdr, plus each choice's continuation <!-- completed: -->
- [ ] Capture the guardian-DENY owner-override prompt at the same widths on both multiplexers, plus each choice's continuation <!-- completed: -->
- [ ] Capture the `execute_code` and action/protected-write gate prompts that `smart` reaches, and record which gates share a form <!-- completed: -->
- [ ] Capture the `approvals.timeout` closure (with a short test timeout in an isolated `HERMES_HOME` copy of the operator configuration), plus the guardian-unavailable fallback when reachable <!-- completed: -->
- [ ] Capture every other native form observed at startup or during work (setup wizard, hook consent, update or trust notice, `clarify`) <!-- completed: -->
- [ ] Capture the empty composer, a typed literal and the post-submit screen for prompt and for `! ` native exec, with separate literal and submit stages; verify local `!` execution with zero model turns <!-- completed: -->
- [ ] Measure the allow-once and deny key stages per form (typed choice and submit, or focus keys), including human-input and output races at the observed-state boundary <!-- completed: -->
- [ ] Determine the member's Hermes session identity source and the session-scoped stop control; verify on an active turn and on running `notify=true` background processes that settlement leaves no running `processes.json` entry and no active turn for the key <!-- completed: -->
- [ ] Find the narrowest width at which each prompt's menu fits one row (`W_min`), and record the layout breaks and limits <!-- completed: -->

### Step 6a: Apply the Step 6 revisions

- [ ] Remove the spawn-environment removal mechanism and every approval-bypass reference from code, tests, SPEC.md, docs/ and skills/ <!-- completed: -->
- [ ] Launch-policy catalog `smart` and `manual` with the preflight launch-mode match and its error string, in code, tests and every documentation target <!-- completed: -->
- [ ] Setup merge, doctor row and preflight check for `terminal.client_env_passthrough`, in code, tests and documentation <!-- completed: -->
- [ ] `hermes` notes: gateway-owned tools, absolute CAFleet path in background commands, skill self-edits restored by setup, dead-client deletion recovery <!-- completed: -->

### Step 7: Hermes bundled profile, deletion settlement and Member acceptance

- [ ] Convert accepted captures into `cafleet/src/coding_agent/native/hermes.json` and `bundled.json` fixture entries with reviewed masks and digests <!-- completed: -->
- [ ] Add the Hermes profile from `2026.9.24` with request rules for ESCALATE, override and each distinct gate form, interaction rules for other observed forms, continuation rules and the timeout closure <!-- completed: -->
- [ ] Add allow-once and deny answer plans with gates, plus empty-composer, prompt and native-exec plans from the measured stages <!-- completed: -->
- [ ] Add positive and adversarial replay tests covering every row of § Hermes bundled profile Replay fixtures <!-- completed: -->
- [ ] Implement session-identity capture into `native_session_json` and the Hermes settlement in `member delete` and creation compensation, with unit tests for success, unverifiable stop, stale PID identity and retry <!-- completed: -->
- [ ] Run live runner acceptance on tmux and herdr. Cases: default launch, a non-dangerous command, ESCALATE answered allow and deny via `permission respond`, override answered allow and deny, timeout closure as `not_visible`, prompt and native-exec submission, deletion with a running background process, a Director-only command (plain and leading-assignment forms) blocked by the deny globs and routed, and `member create` refused for a missing glob. <!-- completed: -->
- [ ] Update § Role evaluation Member row with the recorded evidence and verdict <!-- completed: -->

### Step 8: Validation

- [ ] Run `mise //cafleet:test` and `mise //cafleet:lint` <!-- completed: -->
- [ ] Run `mise //docs:build` <!-- completed: -->
- [ ] Check that every rendered table cell containing a literal `|` uses the `<code>&#124;</code>` form, by inspecting the built HTML <!-- completed: -->

### Step 9: Verdict gate and shipping

- [ ] Copy the final verdicts into `SPEC.md` § Role eligibility and the `hermes` Role eligibility subsection <!-- completed: -->
- [ ] Update the shipped surfaces from `runtime-evidence.md` per § Evidence-driven documentation updates <!-- completed: -->
- [ ] Director `yes`: add `#### Hermes native Director reception` from the Step 4 probe; Director `no`: record the failure in § Role evaluation and keep the ineligible-backend note <!-- completed: -->
- [ ] Member `no`: remove the Hermes member launch path, profile, fixtures, settlement, setup deny-glob merge and `V2` migration from the branch and their documentation, recording the failure here. With Director `yes`, add the `Error: hermes is not Member-eligible; see Role eligibility` refusal to `member create` (explicit and fleet-inherited backend) with unit tests. Document it in SPEC §6.7 Contract error strings and the `skills/cafleet/reference/cli.md` errors. Add the § Model selection step 3 rule in `skills/cafleet/roles/director.md`: skip a Member-`no` fleet backend and relay an operator choice of a Member-eligible backend via `{decision_surface}` before spawning. <!-- completed: -->
- [ ] Both `no`: remove every Hermes surface (registry entry, CLI choices, setup/doctor, docs, skills, SPEC rows) from the branch, keeping Step 2 and this document's record <!-- completed: -->
- [ ] Reinstall the CLI with `mise //cafleet:install` and run `cafleet setup --coding-agent hermes` plus `cafleet doctor` for each shipped role <!-- completed: -->
- [ ] Re-run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` after the gate edits <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-02 | Initial draft |
| 2026-11-02 | Review round 1. Changes: approval-prompt coverage definition and Bundled support/verdict wording; existing-backend recheck; deterministic stream-probe sender; OpenCode precedent for Director `no`. Also: effective configuration through `hermes config get` and the gateway-owner outcomes; timeout closure as `not_visible`; version-independent `smart`. Recorded user decisions U1 (setup-installed Director-only deny globs, launch blocked when missing) and U2 (classification by definition); evidence-driven documentation updates. |
| 2026-11-02 | Review round 2: Member-ineligible fleet backend handling in Director model selection and the `member create` refusal. Approved by the user. |
| 2026-11-02 | Execution rulings: plain-setup skip line, `hermes.approvals.mode` and `hermes.approvals.deny` doctor rows, `<config path>` from `hermes config path` (base fallback), `notify=true` and `process_manage`, observed deny-entry comment preservation, Claude row recorded limit. |
| 2026-11-02 | User decisions U3 (stream probe turn-boundary consumption), U4 (pane-input deletion stop) and U5 (`smart` and `manual` launch modes, `terminal.client_env_passthrough` merge); Step 6a added. |
