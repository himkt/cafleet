# Coding agents

Select a member backend through the CLI’s `--coding-agent` choices. Mixed-backend members share the same broker contracts. The value is recorded in the
placement's `coding_agent` column.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |
| `hermes` | Hermes Agent | Reads the cafleet skill files by absolute path |

Plain `cafleet setup` installs assets for every supported backend
in one run; `--coding-agent` narrows the selection. [Quickstart](../quickstart.md#configure)
lists what setup installs for each backend and the configuration you add.

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on. `fleet create`
  records the calling pane and its multiplexer server; a Hermes Director,
  whose terminal tools run in a shared gateway, names its pane with
  `--pane <pane-id>`, and `--mux-server <server>` selects another server
  ([Hermes setup](../quickstart.md#hermes)).
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member inherits the spawning Director's placement backend (an explicit
  value still wins).

Run the root Director on a backend whose Director verdict is `yes`, and spawn members on backends whose Member verdict is `yes`; a backend whose Director verdict is `no` runs only as a member. [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records each backend's verdicts with their evidence.

## Director requirements

A Director backend needs two capabilities, checked independently: goal lifecycle controls and reception of the Director's broker records. Goal controls come from the backend itself or from a CAFleet extension that supports the full goal lifecycle across compaction and restarts. Reception falls into three tiers:

| Tier | How records reach the Director | Standing |
|---|---|---|
| Native subscribe | The backend itself pushes each record into the session, starting a turn when the session is idle | Preferred |
| Extension subscribe | A CAFleet plugin that `cafleet setup` installs pushes each record the same way | Accepted, below native |
| Built-in compromise | The model reads the running listener's output or receives periodic batches | Far below both; recorded only when neither subscribe tier exists |

[Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) defines the tiers and the probes that establish them. Each bundled backend meets both Director requirements as follows:

| Backend | Director launch | Reception | Goal |
|---|---|---|---|
| `claude` | Your usual Claude Code session | The native Monitor tool | Native `/goal` |
| `codex` | Your usual Codex session | `cafleet message listen --codex-thread` hands each record to the session as a queued turn | Native `/goal` |
| `opencode` | OpenCode 2.2.0 or later as `opencode mini --standalone` from the project directory, with the default agent | The CAFleet plugin's `cafleet_listen` tool | The CAFleet plugin's `/cafleet-goal` command |
| `hermes` | The TUI client, `hermes chat --tui` | The CAFleet plugin's `cafleet_listen` tool | Native `/goal` |

A Hermes Director runs the TUI because a classic CLI Director receives and answers member messages without showing the messages or its replies in its pane; Hermes members keep the classic CLI. The shared OpenCode background service passes no client environment to plugins, so it is not a supported Director launch.

`cafleet setup` installs the CAFleet plugin for OpenCode and Hermes next to the skills and presets, records which `cafleet` executable the plugin runs, and for Hermes enables the plugin in your Hermes configuration. `cafleet doctor` checks that each installed plugin matches the copy embedded in the CLI and that Hermes enables it. Each plugin tool takes the Director's ID and runs only for a root Director, and the OpenCode member presets deny them.

## Launch modes

New members run through the CAFleet PTY runner with a declared detection-coverage state. Every launch mode is classified `auto` or `manual`. An `auto` mode is native automatic review or a fixed allow/deny policy, and a `manual` mode asks a human on the native surface for operations its rules do not settle. Omitting `--permission-mode` selects the backend's default mode, which is an `auto` mode on every backend:

| Backend | Default mode | Other `auto` modes | `manual` modes |
|---|---|---|---|
| `claude` | `auto` | `dontAsk`, `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` |
| `codex` | `approve-for-me` | `never` | `on-request` |
| `opencode` | `cafleet` | — | `cafleet-ask` |
| `hermes` | `smart` | — | `manual` |

Hermes has no launch flag for its mode: CAFleet records the `smart` or `manual` choice, and the `approvals.mode` of your Hermes configuration governs the session. [Hermes configuration](#hermes-configuration) covers the configuration.

When the member's bundled runner profile recognizes an approval prompt, the Director handles it as a runtime event ([Consume a runtime event](members.md#consume-a-runtime-event)). A prompt the profile does not recognize, such as one in a pane narrower than the profile's captured layouts, is answered manually on the member's native surface. A Member-eligible backend's profile recognizes every approval prompt its default mode can show, outside its recorded layout limits ([Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements)). Broker access remains supplied by each backend’s launch configuration, subject to native and managed rules. [Choose a native policy](members.md#choose-a-native-policy) covers mode selection and profile coverage, and the [spawn argv contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#per-backend-build_spawn_argv-exact-token-by-token) specifies child arguments, defaults and sandbox boundaries.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders the identity
and database placeholders to literals, so the member reads its ids as plain
text lines (e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly
on every command, adding `--database-url <url>` when its `DATABASE URL:` line
names a URL rather than `default`. The [spawn-prompt substitution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#spawn-prompt-substitution)
lists each placeholder and its label line. The member environment carries only the selected backend's config-location variable when the creator sets it.
Each backend honors a leading-`!` shell shortcut on the coding agent's input line, including the minimal `mini` interface that OpenCode member panes run; [backend capabilities](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities) compares their sandbox and shell-command posture.
`cafleet member exec` submits a visible native command through the runner, using the profile's measured native-exec plan or delivery fallback and queueing it while a detected native interaction is open. A member routes a
command its harness does not run through the Director — see
[Command routing](members.md#command-routing). The
[CLI specification](https://github.com/himkt/cafleet/blob/main/SPEC.md#63-cli)
is the full command reference.

## One-shot command isolation

Members and the Director run each `cafleet` command as the only command of its
shell-tool invocation, so the host's single-command permission rules match it.
A compound line can fall outside those rules and fail with a permission error,
even when every command in it is allowed.

## Model choice

Before each spawn, the Director reads the selected backend's model catalog
and policy in the installed CAFleet skill, then passes the chosen
`--coding-agent` and `--model` values. The CLI forwards the model; selection
is a Director responsibility. The catalog provides exact spawn tokens,
Claude aliases, reviewed capability classes, standard input/output prices
and official sources, ordered from most to least capable within each backend.
The OpenCode catalog lists every Console model with its `opencode/` prefix,
excluding models past their published deprecation date, and every OpenCode Go
model with its `opencode-go/` prefix.

A bare Claude or GPT model reference selects Claude Code or Codex on every fleet, including an OpenCode fleet, and a provider-prefixed reference such as `opencode/glm-5.3` selects OpenCode. Naming OpenCode for the member selects OpenCode for that model, and a bare name offered only by OpenCode selects OpenCode when it uniquely matches a catalog ID. For an OpenCode model named without a provider, the Director uses the OpenCode Go form when your configured providers offer it, otherwise the Console form. The Director asks you to resolve an unknown, ambiguous or mismatched reference, or an OpenCode model your providers do not offer. The installed Director policy owns the exact resolution rules.

Use the selected backend's role defaults and catalog as the source of truth.

Hermes has no maintained model catalog. The Director selects `hermes` only when
you name the backend or the fleet's Director runs on Hermes, never from a model
name. A Hermes member omits `--model` and `--effort` by default, so your
configured default model and provider apply; a model you name passes through
verbatim, provider-prefixed ids included. Cost efficiency mode has no catalog
prices to compare for Hermes, so the Director asks you to choose.

Only the exact phrase `cost efficiency mode` in the originating user request
activates cheapest-capable selection for ordinary members. The Director
chooses a backend first and compares models within that backend. On OpenCode,
models available through the operator's OpenCode Go subscription count as
effectively free and are preferred. Outside cost efficiency mode, ordinary
members use the backend's other-member default. Reviewers use their role defaults, including supported effort
levels.

Explicit backend, model and effort values remain overrides. A mismatch or
missing suitable model is relayed to the operator. The catalog is maintained
through the model-refresh workflow and refreshed at least every 30 days;
staleness disables cost efficiency mode until a maintainer refreshes and
ships it. Prices are planning estimates and capability classes are reviewed
judgments, rather than invoice guarantees or provider benchmark claims.

Evidence of insufficient capability can trigger a replacement with a strictly stronger model on the same backend, a bounded number of times per task. The installed Director policy owns evidence, handoff, limits and selection; a user-pinned model requires an operator decision before replacement.

`cafleet member create --model <string>` forwards the value to the backend's
own model flag; omission uses that binary's default. Claude, Codex and Hermes
pass any value through and let the binary reject unknown models, so a newly released
model needs no CAFleet release. OpenCode requires the `<provider-id>/<model-id>`
form, checked when the member is created. The
[model selection contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#model-selection)
has the exact format and error.

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to
the spawned backend binary. Unlike `--model`, the accepted level set is
validated per backend at create time, before any registration or multiplexer
side effect; omit the flag and the binary uses its own default. OpenCode has no
reasoning-effort control, so omit the flag for OpenCode members. Hermes
forwards the level as its own reasoning flag. The
[reasoning effort contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#reasoning-effort)
lists each backend's levels and forwarding form.

## Backend differences

The backends differ in reasoning effort, pane titles and sandbox isolation;
the [backend capabilities table](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities)
compares them. Because only claude panes display the member name, use the
`pane_id` column of `cafleet member list` to locate a member's pane.

A Hermes member's session can keep accepted work and background processes
running after its pane's client exits. `cafleet member delete` stops that
session's work and the background processes it registered, and keeps the
member when it cannot verify the stop; CAFleet never touches your Hermes
gateway or sessions it does not own.

## Backend configuration

[Quickstart](../quickstart.md#configure) gives each backend's starting
configuration. This section explains it and lists the fixes for common
symptoms.

### Config directories

`CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `HERMES_HOME` relocate their backend's
installed assets. `OPENCODE_CONFIG_DIR` relocates only the OpenCode presets and
plugin; its skills remain under `~/.config/opencode/skills`. Set the variable in
the environment you run `cafleet setup` and `cafleet member create` from:
`member create` forwards it to the member, so the member reads the
configuration CAFleet installed into. The
[config-dir resolution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#config-dir-resolution)
specifies validation and every install target.

### Claude Code configuration

Use the auto mode profile when your Director session runs in `auto` mode, and
the other-modes profile for any other mode. Both profiles pre-approve what
members need beyond their broker commands, and auto mode's classifier reviews
each other member call.

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs, for every fleet, because IDs are literal integer arguments |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory |
| `Edit`, `Write` | Both | Every member that writes files | A matching entry pre-approves the member's edits; no entry covers a protected path such as `.git` or `.claude` |
| The `member prompt`, `member exec` and `member press` prefixes under `ask` | Both, optional | The Director | The operator confirms each of the Director's dispatches; `ask` entries outrank allow entries |

In auto mode a classifier reviews each Director command that no rule matches,
so the auto mode profile carries no `Bash(cafleet ...)` entry. `defaultMode`
selects auto mode from user-level or managed settings; a project or local
settings file cannot select it.

`Edit` and `Write` apply to every session that reads the settings file, your
own included. To confine the approval, replace both with a path rule such as
`Edit(~/work/**)`, which covers every file-editing tool. When
`CLAUDE_CONFIG_DIR` relocates the config directory, write the `Read` entry
against that directory's `skills/**`.

A claude member needs no allow rule of its own for the broker commands:
cafleet passes `Bash(cafleet message *)` on the member's spawn command line. A
`deny` or `ask` rule that matches a broker command, the managed setting
`allowManagedPermissionRulesOnly` or a blocking `PreToolUse` hook still stops
a member. Every Director-only command takes the caller's ID as
`--from-member-id` and refuses unless it is the fleet's root Director, which
catches a member's accidental invocation; this guard is not a security
boundary ([Command routing](members.md#command-routing)).

A member whose shell command is denied or refused by auto mode asks its
Director to run it, and with the `ask` entries each such `cafleet member exec`
asks you to confirm.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a refused or denied shell command | Auto mode's classifier refused it, or a `deny` rule matched it | Add an allow entry for the command |
| A member reports a refused or denied file edit | No `Edit` entry matches, or the path is protected | Add `Edit` and `Write` or widen the path rule; for a protected path, the Director makes the edit |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add an allow pattern for that subcommand |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`; when auto mode is unavailable, use the other-modes profile |

Claude Code can show two auto-mode dialogs in a member pane after a turn:
`Make auto mode your default permission mode?` when your
`~/.claude/settings.json` pins a non-`auto` `defaultMode`, and `Teach auto mode
about your environment?` in auto mode. The runner holds input behind each one
and sends the Director a question event. The Director dismisses the first with
`No, keep …` and the second with `Not now`, and asks you before choosing `Don't
show again`; each answer changes only that dialog's own flag in the member's
Claude configuration. The `Yes` choices are never sent for you; answer them in
the member pane if you want them.

### Codex configuration

`network_access = true` is required because tmux and herdr communicate over a
local socket, which the Codex sandbox classifies as network access; without it
cafleet commands fail with `Operation not permitted`. `writable_roots` grants
write access to the directory of the selected database file, the default
`~/.local/share/cafleet` unless `--database-url` selects another.

`cafleet setup` installs `~/.codex/rules/cafleet.rules`, which allows every
`cafleet` command. Codex members need it to run their broker commands, and
the Director-only commands refuse a member's own ID on their own. Codex
applies the strictest decision across every `*.rules` file in that directory,
and setup overwrites its own file on every install, so put your own rules,
such as `prompt` rules to confirm the Director's commands, in a separate file
there. A rules directory maintained without the installed file, for example
from a dotfiles repository, needs at least this rule:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)
```

That rule is enough for members. Keep the installed file for a Director
session: Codex has no classifier to review the other `cafleet` commands the
Director runs, so without a matching allow each of them follows the session's
own approval policy and sandbox.

### OpenCode configuration

`cafleet setup` installs both agent presets into `~/.config/opencode/agents/`
and the CAFleet plugin as `~/.config/opencode/plugins/cafleet.ts`, with a
`cafleet.executable` file naming the `cafleet` it runs; rerun setup after
moving or upgrading the CLI. The `cafleet` preset allows its listed commands
and denies every other shell command without a prompt, and `cafleet-ask` opens
a native permission form for the Director to answer instead. Neither preset is
an OS-level sandbox: keep every loaded OpenCode configuration free of MCP
servers, submit one shell command per invocation, and use the `codex` backend
when you need kernel-enforced isolation.

While a goal is active the plugin continues the session turn by turn, and it
pauses the goal after three continuation turns without reported progress or
after 50 continuations; `/cafleet-goal resume` restarts it with a fresh
continuation budget.

OpenCode allows every action by default, so the plugin tools run on a default
Director without a prompt. If your configuration adds a broad `ask` or `deny`
rule, add this rule after it so the Director keeps the plugin tools:

```json
{ "action": "cafleet", "resource": "*", "effect": "allow" }
```

### Hermes configuration

Hermes members use your existing Hermes configuration: your configured default
provider and model, and your profile when `active_profile` selects one. Each
member starts a fresh conversation.

`cafleet setup --coding-agent hermes` installs the CAFleet plugin into the
`plugins/cafleet/` directory of the Hermes home that `hermes config path`
reports, including an active profile's home, with an `executable` file naming
the `cafleet` it runs. It then edits the file `hermes config path` reports,
keeping the rest of the file: it adds `cafleet` to `plugins.enabled` and sets
`plugins.entries.cafleet.allow_gateway_injection` to `true`. A plain
`cafleet setup` on a host without `hermes` installs the skills and skips the
plugin.

A Hermes Director names its pane with `--pane` because Hermes runs every
session's terminal tools in one shared gateway, which may carry another pane's
variables; the leading-`!` local shell runs in your pane's own environment.
Each broker record arrives as its own turn, in the order the broker delivered
it.

Under `approvals.mode: smart`, the default, commands Hermes does not consider
dangerous run without review, and its guardian model reviews the rest. A
guardian escalation shows an approval prompt in the member's pane, which the
Director answers. CAFleet needs no `approvals.deny` entries.

| Symptom | Cause | Resolution |
|---|---|---|
| Member messages and the Director's replies do not appear in the Director's pane | The Director runs the classic CLI | Restart the Director with `hermes chat --tui` |
| `cafleet doctor` reports `error hermes: hermes plugin cafleet is not enabled: plugins.enabled — run 'cafleet setup --coding-agent hermes'` | The plugin was skipped or is not enabled, for example after a plain `cafleet setup` before Hermes was installed | Run `cafleet setup --coding-agent hermes` |
| Setup fails on the plugin | `plugins.disabled` lists `cafleet` | Remove `cafleet` from `plugins.disabled` and rerun setup |
| Another user plugin stopped loading after setup | Once `plugins.enabled` exists, Hermes loads only the user plugins it lists | Add that plugin to `plugins.enabled` |
| A Director command fails with `BLOCKED: this command matches the user-defined deny rule '<glob>' …` | An `approvals.deny` glob matches a `cafleet` command; Hermes applies the list to every session | Read the list with `hermes config get --json approvals.deny`, then write it back without the `cafleet` globs with `hermes config set approvals.deny '<JSON list without them>'` |
