# Coding agents

Select a member backend through the CLI’s `--coding-agent` choices; mixed-backend members share the same broker contracts, and the placement's `coding_agent` column records the value.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |
| `hermes` | Hermes Agent | Reads the cafleet skill files by absolute path |
| `pi` | pi coding agent | Lists the installed cafleet skills and reads each `SKILL.md` with its read tool |

Plain `cafleet setup` installs assets for every supported backend in one run, and `--coding-agent` narrows the selection; [Quickstart](../quickstart.md#configure) lists what setup installs for each backend and the configuration you add.

- `cafleet fleet create --coding-agent` is required operator-declared metadata naming the backend the Director actually runs on: cafleet does not spawn the root Director's process and cannot detect what already runs in the calling pane. `fleet create` records the calling pane and its multiplexer server; a Hermes Director names its pane with `--pane <pane-id>`, and `--mux-server <server>` selects another server ([Hermes setup](../quickstart.md#hermes)).
- `cafleet member create --coding-agent` selects the spawned backend and is recorded as placement metadata; when omitted, the member inherits the spawning Director's placement backend.

Run the root Director on a backend whose Director verdict is `yes` and members on backends whose Member verdict is `yes`; a backend whose Director verdict is `no` runs only as a member. [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records each backend's verdicts with their evidence.

## Director requirements

A Director backend needs two capabilities, checked independently: goal lifecycle controls, from the backend itself or a CAFleet extension supporting the full goal lifecycle across compaction and restarts, and reception of its broker records. Reception preferably pushes each record into the session, natively or through a CAFleet plugin, starting a turn when idle; a model reading the listener's output ranks far below both ([Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) defines the tiers and their probes). Each bundled backend supplies them as follows, and [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records which requirements its evidence establishes:

| Backend | Director launch | Reception | Goal |
|---|---|---|---|
| `claude` | Your usual Claude Code session | The native Monitor tool | Native `/goal` |
| `codex` | Your usual Codex session | `cafleet message listen --codex-thread` hands each record to the session as a queued turn | Native `/goal` |
| `opencode` | OpenCode 2.2.0 or later as `opencode mini --standalone` from the project directory, with the default agent | The CAFleet plugin's `cafleet_listen` tool | The CAFleet plugin's `/cafleet-goal` command |
| `hermes` | The TUI client, `hermes chat --tui` | The CAFleet plugin's `cafleet_listen` tool | Native `/goal` |
| `pi` | Plain `pi` from the project directory | The CAFleet pi extension's `cafleet_listen` tool | The CAFleet pi extension's `/cafleet-goal` command |

The shared OpenCode background service passes no client environment to plugins, so it is not a supported Director launch.

For OpenCode and Hermes, `cafleet setup` installs the CAFleet plugin beside the skills and presets, records the `cafleet` executable it runs and, for Hermes, enables it in your Hermes configuration; `cafleet doctor` checks that each plugin matches the CLI's embedded copy and that Hermes enables it. For pi, setup installs the CAFleet pi extension, which every pi session loads ([pi configuration](#pi-configuration)). Each plugin and extension tool takes the Director's ID and runs only for a root Director; the OpenCode member presets and the pi member gate deny them.

## Launch modes

New members run through the CAFleet PTY runner with a declared detection-coverage state. Each launch mode is `auto` (native automatic review or a fixed allow/deny policy) or `manual` (a human answers on the native surface for operations its rules do not settle). Omitting `--permission-mode` selects the backend's default mode, `auto` on every backend:

| Backend | Default mode | Other `auto` modes | `manual` modes |
|---|---|---|---|
| `claude` | `auto` | `dontAsk`, `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` |
| `codex` | `approve-for-me` | `never` | `on-request` |
| `opencode` | `cafleet` | — | `cafleet-ask` |
| `hermes` | `smart` | — | `manual` |
| `pi` | `cafleet` | — | `cafleet-ask` |

Hermes has no launch flag for its mode: CAFleet records the `smart` or `manual` choice, and your Hermes configuration's `approvals.mode` ([Hermes configuration](#hermes-configuration)) governs the session. pi has no native per-tool approval, so both pi modes run the CAFleet member gate: `cafleet` runs its allowlisted commands and blocks every other one without a prompt, and `cafleet-ask` opens the gate's approval form instead ([pi configuration](#pi-configuration)).

The Director handles an approval prompt the member's bundled runner profile recognizes as a [runtime event](members.md#consume-a-runtime-event); an unrecognized one, such as in a pane narrower than the profile's captured layouts, is answered manually on the member's native surface. A Member-eligible backend's profile recognizes every approval prompt its default mode can show outside its recorded layout limits ([Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements)). Each backend's launch configuration supplies broker access, subject to native and managed rules. [Choose a native policy](members.md#choose-a-native-policy) covers mode selection and profile coverage; the [spawn argv contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#per-backend-build_spawn_argv-exact-token-by-token) specifies child arguments, defaults and sandbox boundaries.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. `cafleet member create` renders the spawn prompt's identity and database placeholders to literals, so a member reads its IDs as plain text lines (e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on every command, including a non-default database as [Storage](storage.md#backend) describes; the [spawn-prompt substitution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#spawn-prompt-substitution) lists each placeholder and its label line. Each backend honors a leading-`!` shell shortcut on its input line, including the minimal `mini` interface of OpenCode member panes; [backend capabilities](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities) compares their sandbox and shell-command posture. `cafleet member exec` submits a visible native command through the runner ([Deliver generated input](members.md#deliver-generated-input)), and a member routes a command its harness does not run through the Director ([Command routing](members.md#command-routing)). The [CLI specification](https://github.com/himkt/cafleet/blob/main/SPEC.md#63-cli) is the full command reference.

## One-shot command isolation

Members and the Director run each `cafleet` command as the only command of its shell-tool invocation, so the host's single-command permission rules match it; a compound line can fail with a permission error even when every command in it is allowed.

## Model choice

Before each spawn, the Director resolves the member's backend and model from the installed CAFleet skill's Director policy, which owns the exact rules, and that backend's model catalog — exact spawn tokens, Claude aliases, reviewed capability classes, standard input/output prices and official sources, most to least capable within each backend — then passes `--coding-agent` and `--model`; the CLI only forwards the model. The OpenCode catalog lists every Console model (`opencode/` prefix) not past its published deprecation date and every OpenCode Go model (`opencode-go/` prefix).

Name a model, a backend or both. A bare Claude or GPT reference selects Claude Code or Codex on any fleet. A provider-prefixed reference such as `opencode/glm-5.3`, naming OpenCode, or a bare name only OpenCode offers that uniquely matches a catalog ID selects OpenCode; an OpenCode model named without a provider uses the Go form when your providers offer it, otherwise the Console form. Hermes has no catalog: the Director selects it only when you name it or the Director runs on it, and a Hermes member omits `--model` and `--effort` by default so your configured model and provider apply; a model you name passes through verbatim, provider-prefixed IDs included. pi likewise has no catalog and is selected the same way; a pi member omits `--model` and `--effort` by default so your configured pi default applies. The Director asks you to resolve an unknown, ambiguous or mismatched reference, a missing suitable model, an OpenCode model your providers do not offer, or a Hermes or pi model under cost efficiency mode.

Explicit backend, model and effort values are overrides. Only the exact phrase `cost efficiency mode` in your originating request activates cheapest-capable selection for ordinary members: the Director chooses a backend, then compares models within it, preferring models in your OpenCode Go subscription as effectively free. Otherwise ordinary members use the backend's other-member default; reviewers use their role defaults, including supported effort levels. The catalog is refreshed at least every 30 days through the model-refresh workflow; a stale catalog disables cost efficiency mode until a refreshed release ships. Prices are planning estimates and classes reviewed judgments, not invoice guarantees or provider benchmark claims.

Evidence of insufficient capability can trigger a replacement with a strictly stronger model on the same backend, a bounded number of times per task; a user-pinned model requires your decision first.

`cafleet member create --model <string>` forwards the value to the backend's model flag; omitted, the binary's default applies. Claude, Codex and Hermes pass any value through and let the binary reject unknown models, so a new model needs no CAFleet release; pi passes the value through the same way but refuses a `:<thinking>` suffix, which belongs in `--effort`; OpenCode requires `<provider-id>/<model-id>`, checked at creation ([model selection contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#model-selection)).

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to the spawned backend binary. Unlike `--model`, the level is validated per backend at create time, before any registration or multiplexer side effect; omitted, the binary uses its own default. OpenCode has no reasoning-effort control, so omit the flag for OpenCode members; Hermes forwards the level as its own reasoning flag, and pi as its thinking level, which pi clamps to what the model supports. The [reasoning effort contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#reasoning-effort) lists each backend's levels and forwarding form.

## Backend differences

The backends differ in reasoning effort, pane titles and sandbox isolation, as the [backend capabilities table](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities) compares. Only claude and pi panes display the member name, so for the other backends locate a member's pane by the `pane_id` column of `cafleet member list`.

A Hermes member's session can keep accepted work and background processes running after its pane's client exits. `cafleet member delete` stops that session's work and registered background processes, keeping the member when it cannot verify the stop; CAFleet never touches your Hermes gateway or sessions it does not own.

A pi member has no OS-level sandbox: the CAFleet member gate decides which tools and shell commands it runs, but its file reads reach any path outside `.env` files.

## Backend configuration

[Quickstart](../quickstart.md#configure) gives each backend's starting configuration; this section explains it and lists fixes for common symptoms.

### Config directories

`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `HERMES_HOME` and `PI_CODING_AGENT_DIR` relocate their backend's installed assets; `OPENCODE_CONFIG_DIR` relocates only the OpenCode presets and plugin, leaving its skills under `~/.config/opencode/skills`. Set the variable where you run `cafleet setup` and `cafleet member create`: `member create` forwards only the selected backend's variable to the member, which then reads the configuration CAFleet installed into. The [config-dir resolution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#config-dir-resolution) specifies validation and every install target.

### Claude Code configuration

Use the auto mode profile when your Director session runs in `auto` mode, and the other-modes profile for any other mode. Both pre-approve what members need beyond their broker commands; in auto mode the classifier reviews every other member call and every Director command no rule matches, so that profile carries no `Bash(cafleet ...)` entry.

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs, for every fleet, because IDs are literal integer arguments |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory; under a relocated `CLAUDE_CONFIG_DIR`, use that directory's `skills/**` |
| `Edit`, `Write` | Both | Every member that writes files | A matching entry pre-approves the member's edits; no entry covers a protected path such as `.git` or `.claude` |
| The `member prompt`, `member exec` and `member press` prefixes under `ask` | Both, optional | The Director | The operator confirms each Director dispatch, including each `member exec` for a member's denied or refused command; `ask` entries outrank allow entries |

`defaultMode` selects auto mode only from user-level or managed settings. `Edit` and `Write` apply to every session that reads the settings file, your own included; to confine them, replace both with a path rule such as `Edit(~/work/**)`, which covers every file-editing tool.

A claude member needs no broker allow rule: cafleet passes `Bash(cafleet message *)` on its spawn command line. A `deny` or `ask` rule matching a broker command, the managed setting `allowManagedPermissionRulesOnly` or a blocking `PreToolUse` hook still stops it. A member whose shell command is denied or refused by auto mode asks its Director to run it, and Director-only commands refuse a member's own ID ([Command routing](members.md#command-routing)).

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a refused or denied shell command | Auto mode's classifier refused it, or a `deny` rule matched it | Add an allow entry for the command |
| A member reports a refused or denied file edit | No `Edit` entry matches, or the path is protected | Add `Edit` and `Write` or widen the path rule; for a protected path, the Director makes the edit |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add an allow pattern for that subcommand |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`; when auto mode is unavailable, use the other-modes profile |

Claude Code can show two auto-mode dialogs in a member pane after a turn: `Make auto mode your default permission mode?` when your `~/.claude/settings.json` pins a non-`auto` `defaultMode`, and `Teach auto mode about your environment?` in auto mode. The runner holds input behind each and sends the Director a question event. The Director dismisses the first with `No, keep …` and the second with `Not now`, asking you before `Don't show again`; each answer changes only that dialog's flag in the member's Claude configuration. CAFleet never sends the `Yes` choices; answer them in the member pane if you want them.

### Codex configuration

`network_access = true` is required because tmux and herdr communicate over a local socket, which the Codex sandbox classifies as network access; without it cafleet commands fail with `Operation not permitted`. `writable_roots` grants write access to the selected database file's directory, `~/.local/share/cafleet` unless `--database-url` selects another.

`cafleet setup` installs `~/.codex/rules/cafleet.rules`, which allows every `cafleet` command. Codex members need it for their broker commands, and the Director-only commands refuse a member's own ID by themselves. Codex applies the strictest decision across every `*.rules` file in that directory, and setup overwrites its own file on every install, so put your own rules, such as `prompt` rules confirming the Director's commands, in a separate file there. A rules directory maintained without the installed file, for example from a dotfiles repository, needs at least this rule:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)
```

That rule is enough for members. Keep the installed file for a Director session: Codex has no classifier to review the Director's other `cafleet` commands, so without a matching allow each follows the session's own approval policy and sandbox.

### OpenCode configuration

`cafleet setup` installs both agent presets into `~/.config/opencode/agents/` and the CAFleet plugin as `~/.config/opencode/plugins/cafleet.ts`, with a `cafleet.executable` file naming the `cafleet` it runs; rerun setup after moving or upgrading the CLI. The `cafleet` preset allows its listed commands and denies every other shell command without a prompt; `cafleet-ask` instead opens a native permission form for the Director to answer. Neither preset is an OS-level sandbox: keep every loaded OpenCode configuration free of MCP servers, submit one shell command per invocation, and use the `codex` backend for kernel-enforced isolation.

While a goal is active the plugin continues the session turn by turn, pausing the goal after three continuation turns without reported progress or after 50 continuations; `/cafleet-goal resume` restarts it with a fresh continuation budget.

OpenCode allows every action by default, so the plugin tools run on a default Director without a prompt. If your configuration adds a broad `ask` or `deny` rule, add this rule after it so the Director keeps the plugin tools:

```json
{ "action": "cafleet", "resource": "*", "effect": "allow" }
```

### Hermes configuration

Hermes members use your existing Hermes configuration: your configured default provider and model, and your profile when `active_profile` selects one. Each member starts a fresh conversation.

`cafleet setup --coding-agent hermes` installs the CAFleet plugin into `plugins/cafleet/` of the Hermes home that `hermes config path` reports, including an active profile's home, with an `executable` file naming the `cafleet` it runs. It then edits the reported file, keeping the rest of it: it adds `cafleet` to `plugins.enabled` and sets `plugins.entries.cafleet.allow_gateway_injection` to `true`. A plain `cafleet setup` on a host without `hermes` installs the skills and skips the plugin.

Hermes runs every session's terminal tools in one shared gateway, which may carry another pane's variables, so a Hermes Director names its pane with `--pane`; the leading-`!` local shell runs in your pane's own environment. Each broker record arrives as its own turn, in the broker's delivery order.

Under `approvals.mode: smart`, the default, commands Hermes does not consider dangerous run without review and its guardian model reviews the rest; a guardian escalation shows an approval prompt in the member's pane, which the Director answers. CAFleet needs no `approvals.deny` entries.

| Symptom | Cause | Resolution |
|---|---|---|
| Member messages and the Director's replies do not appear in the Director's pane | The Director runs the classic CLI | Restart the Director with `hermes chat --tui` |
| `cafleet doctor` reports `error hermes: hermes plugin cafleet is not enabled: plugins.enabled — run 'cafleet setup --coding-agent hermes'` | The plugin was skipped or is not enabled, for example after a plain `cafleet setup` before Hermes was installed | Run `cafleet setup --coding-agent hermes` |
| Setup fails on the plugin | `plugins.disabled` lists `cafleet` | Remove `cafleet` from `plugins.disabled` and rerun setup |
| Another user plugin stopped loading after setup | Once `plugins.enabled` exists, Hermes loads only the user plugins it lists | Add that plugin to `plugins.enabled` |
| A Director command fails with `BLOCKED: this command matches the user-defined deny rule '<glob>' …` | An `approvals.deny` glob matches a `cafleet` command; Hermes applies the list to every session | Read the list with `hermes config get --json approvals.deny`, then write it back without the `cafleet` globs with `hermes config set approvals.deny '<JSON list without them>'` |

### pi configuration

pi members use your existing pi configuration: the `defaultProvider` and `defaultModel` in `~/.pi/agent/settings.json` and the credentials pi already holds. Each member starts a fresh session in pi's fullscreen interface and ignores trust-gated project-local pi resources, so no project-trust dialog blocks it; `AGENTS.md` and `CLAUDE.md` still load.

`cafleet setup --coding-agent pi` installs the skills into `~/.pi/agent/skills/` and the CAFleet pi extension as `~/.pi/agent/extensions/cafleet.ts`, with a `cafleet.executable` file naming the `cafleet` it runs; setup runs no `pi` command, and you rerun it after moving or upgrading the CLI. pi loads the extension in every session. In a Director session it supplies reception, the `/cafleet-goal` command and the `cafleet_ask` question tool; in a member session it is the member gate.

pi has no per-tool approval and no OS-level sandbox, so the member gate is a pi member's whole permission posture:

| Operation | `cafleet` | `cafleet-ask` |
|---|---|---|
| A shell command on the gate's allowlist, run as one simple command | Runs | Runs |
| Any other shell command, including a compound line or a leading `NAME=value` assignment | Blocked; the member asks the Director to run it | Opens the gate's approval form for the Director to answer |
| Reading and searching files | Allowed, except `.env` files | Same |
| Editing and writing files | Allowed inside the working directory, except `.env` files and anything below `.git/`, `.pi/` or `.agents/` | Same |
| Any other tool, including MCP and other extension tools | Blocked | Same |

The protected directories keep a member from planting a Git hook or a project pi resource that a later trusted pi session runs. An allowed command still runs the code it executes, such as test code and Git hooks, and file reads reach any path outside `.env` files; use the `codex` backend for kernel-enforced isolation.

While a goal is active the extension continues the session turn by turn, pausing the goal after three continuation turns without reported progress, after 50 continuations or when you interrupt the run; `/cafleet-goal resume` restarts it with a fresh continuation budget. The goal survives compaction and `pi --continue` on the same session.

| Symptom | Cause | Resolution |
|---|---|---|
| `cafleet member create --coding-agent pi` fails with `cannot read the CAFleet pi extension at …` | Setup has not installed the extension at the resolved pi directory | Run `cafleet setup --coding-agent pi` |
| A pi member exits at startup with `Error: Unknown option: --cafleet-gate` | pi did not load the extension, for example because it is disabled in pi's configuration or is a stale copy | Re-enable it if you disabled it, then run `cafleet doctor` and the `cafleet setup --coding-agent pi` it names |
| The Director has no `cafleet_listen` tool, or a tool reports an executable record error | The extension or its `cafleet.executable` file is missing or outdated | Run `cafleet setup --coding-agent pi` and restart the Director |
| The Director receives `[cafleet listen closed] the extension runtime was replaced …` | `/reload`, `/new`, `/resume` or `/fork` replaced the extension runtime | The Director re-arms reception with `cafleet_listen` |
