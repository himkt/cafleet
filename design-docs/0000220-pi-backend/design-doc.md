# pi coding-agent backend

**Status**: Approved
**Progress**: 52/52 tasks complete
**Last Updated**: 2026-11-08

## Overview

Add the pi coding agent (earendil-works/pi, 1.5.0 minimum) as the CAFleet backend `pi`, eligible for both the Director and the Member role. One CAFleet-vendored pi extension, installed by `cafleet setup`, supplies the tier-2 goal, the tier-2 subscribe listen, a `cafleet_ask` decision surface and a member permission gate. The change also adds argv, model/effort validation, config-dir resolution, launch modes, a bundled runner profile, documentation and the live eligibility probes that record each verdict.

## Success Criteria

- [ ] `pi` is a registry token everywhere the backend registry applies: `--coding-agent`, `member_placements.coding_agent`, the `member_runs.backend` CHECK, `cafleet setup` and `cafleet doctor`. It is last in registry order.
- [ ] `member create --coding-agent pi` builds exactly the argv of §1.2 for every launch mode, model and effort combination, and rejects the §1.3–§1.4 inputs with their exact usage errors before any side effect.
- [ ] `cafleet setup --coding-agent pi` installs the skills and the extension with its executable record under the resolved pi base. `cafleet doctor` reports `pi.extension` `ok`, and `error` for a tampered extension or an unusable record.
- [ ] Under the omitted `cafleet` mode, a pi member runs every allowlisted command without a form, and every other bash command is blocked with the gate's reason. Under `cafleet-ask`, an unmatched command opens the gate's approval form. The runner reports that form as `interaction_opened`, and `cafleet permission respond` answers it allow or deny on tmux and herdr. Every pi launch mode runs the gate.
- [ ] The pi profile from `1.5.0` is in `bundled.json`, backed by native fixtures in `src/coding_agent/native/pi.json` captured on tmux and herdr, and passes the catalog build guard.
- [ ] Live runs A-P and A-M (§7) pass on pi 1.5.0, and their observed results are recorded: subscribe probe, extension-goal minimum bar, `cafleet_ask`, and Member runner acceptance on both multiplexers. A failed run records `no` with the observed failure, and the other role still ships.
- [ ] `SPEC.md`, the docs site, the skills (a complete `## pi` section in `skills/cafleet/reference/coding-agents.md`, plus `cli.md`, `director.md` and `model-selection.md`) describe the shipped behavior, and `README.md` matches the docs index.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

CAFleet ships four backends. Each Director needs a goal and a listen requirement (SPEC §6.7 Role requirements). Each Member needs a runner-compatible PTY, a coverage declaration and a classified default launch mode with approval-prompt coverage.

pi is a multi-provider terminal coding agent with an extension API, which loads TypeScript through jiti without a build step. It has no native goal, no native facility that pushes external text into a session, no per-tool approval and no sandbox. Its extension API supplies everything a tier-2 goal, a tier-2 subscribe listen and a member permission gate need. The OpenCode plugin (`extensions/opencode/cafleet.ts`) is the reference port.

### Design-time verification (2026-11-07)

These facts come from the installed pi 1.5.0 at `/root/.local/share/mise/installs/aqua-earendil-works-pi/v1.5.0/pi`: `pi --help`, its bundled `docs/` (`cli.md`, `extensions.md`, `skills.md`, `keybindings.md`, `security.md`, `settings.md`) and `examples/extensions/`. No live probe ran at design time; §7 establishes every verdict.

| Area | pi 1.5.0 fact | Design use |
|---|---|---|
| Version | `pi --version` prints `1.5.0` | Standard one-token probe (§1.5) |
| Invocation | `pi [options] [--] [@files...] [messages...]`; a positional message seeds the interactive session | Bare trailing prompt after `--` (§1.2) |
| Session name | `--name/-n` sets the display name; the terminal title becomes `π - <name> - <cwd basename>` | `{pane_title}` |
| Model | `--model <pattern>` takes `provider/id`, exact ids and fuzzy names, plus an optional `:<thinking>` suffix | Verbatim pass-through, suffix rejected (§1.3) |
| Thinking | `--thinking off,minimal,low,medium,high,xhigh,max`, clamped to the model's capability and overriding a `--model` suffix | `--effort` mapping (§1.4) |
| Config dir | `PI_CODING_AGENT_DIR`, default `~/.pi/agent`; holds `settings.json`, `auth.json`, `sessions/`, `skills/`, `extensions/` | Config-dir resolution (§1.6) |
| Extension discovery | `<dir>/extensions/*.ts`, `*.js`, `*/index.ts` and `*/package.json` with `pi.extensions`, one level deep | The extension auto-loads in every session; the executable record is ignored (§2.1) |
| Extension flags | `pi.registerFlag(name, {type: "string"})`, read with `pi.getFlag` after `session_start`; `pi --help` lists flags of loaded extensions | `--cafleet-gate` (§3.2) |
| Tool gate | `tool_call` handlers run before execution and return `{block: true, reason}`; nested calls also pass through them; a handler error blocks the tool | Member gate (§3) |
| `!` input | `!cmd` runs a shell command as `user_bash`, outside `tool_call`, and starts no turn; its output reaches the model with the next prompt | `member exec` Note (§6) |
| External push | `pi.sendMessage(msg, {triggerTurn: true, deliverAs: "followUp"})` starts a turn when idle and otherwise waits until the agent has no more tool calls; `file-trigger.ts` uses it for external events | Receiver delivery (§4.2) |
| Lifecycle | `session_start` (startup, reload, new, resume, fork); `session_shutdown` (quit, reload, new, resume, fork), where quit covers Ctrl+C, Ctrl+D, SIGHUP and SIGTERM | Receiver end (§4.3) |
| Continuation | `agent_before_settle` may return `{entries, continue: true}`; an aborted run skips it and reaches `agent_settled` with `event.aborted` | Goal continuation (§4.4) |
| Persistence | `pi.appendEntry(type, data)` persists outside LLM context and is restored from `ctx.sessionManager.getBranch()`; the `context` hook edits the messages of every model request, while `before_agent_start` runs only after input handling and is skipped for a continuation that `agent_before_settle` requests | Goal persistence and goal context (§4.4) |
| Dialogs | `ctx.ui.select(title, options)`, `ctx.ui.input(title, placeholder)`, multi-line titles; Enter confirms, Escape and Ctrl+C cancel (`undefined`); the select list moves with Up/Down | Approval form (§3.5), `cafleet_ask` (§4.5) |
| Project trust | A trust dialog blocks startup when the cwd has `.pi/` resources or `.agents/skills`; `--no-approve` ignores trust-gated project-local files without asking | Members pass `--no-approve` |
| Keys | Enter submits; Enter while working queues a steering message; Escape interrupts the running turn; Ctrl+C twice exits | Notes (§6) |
| Skills | Directories containing `SKILL.md` under `<dir>/skills/` are discovered recursively; pi lists each name, description and path in the system prompt, and the model reads `SKILL.md`; `/skill:<name>` forces it | `{skill_loader}` |

---

## Specification

### 1. Backend registration and spawn

#### 1.1 Registry token and schema

`CodingAgent` gains `Pi`, last in registry order. Its `name` and `binary_name` are `pi`. The §8 DDL and `migrations/V1__baseline.sql` (edited in place) become:

```sql
backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode','hermes','pi')),
```

#### 1.2 Spawn argv (SPEC §6.7 Per-backend `build_spawn_argv`)

`ensure_available` resolves the pi base (§1.6), checks `pi` on `PATH`, and then reads the CAFleet pi extension (§1.7). `display_name` becomes `--name`; `cwd` is unused.

```
["pi", "--cafleet-gate", "cafleet", "--no-approve", "--tui-mode", "fullscreen", "--name", <display_name>]
                                                     # explicit selection replaces "cafleet" with <mode>
  (+ ["--model", <model>]     if model is not None)
  (+ ["--thinking", <effort>] if effort is not None)
  (+ ["--", <prompt>])                               # bare trailing positional after --
```

The permission argv is the `--cafleet-gate <mode>` pair, where `<mode>` is `cafleet` (default) or `cafleet-ask`.

- `--no-approve` ignores trust-gated project-local configuration and resources, so no trust dialog blocks a member. CAFleet skills load from `<base>/skills`, and `AGENTS.md` and `CLAUDE.md` still load.
- `--tui-mode fullscreen` pins the layout the profile matches (§5), whatever the operator's `tuiMode` setting.
- `--` keeps a prompt that begins with `-` positional.
- Every spawn starts a fresh session; the argv never carries `--continue`, `--resume`, `--session`, `--session-id` or `--fork`.
- **Prompt rule.** pi reads a positional beginning with `@` as a file attachment.
  - `member create` therefore checks the first character of the resolved prompt body before registration. It calls a new `CodingAgent::validate_prompt` right after `resolve_body` and before `director_placement` and `ensure_available` (`src/cli/member.rs`).
  - A pi prompt whose first character is `@` fails with usage exit 2 and `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).` No member row, run or pane is created.
  - The raw body is checked, before placeholder substitution. This is equivalent to checking the rendered prompt, because a leading placeholder renders to digits, `default`, a backend token or `sqlite:///…`, none of which starts with `@`.
  - Every other backend accepts every prompt. `build_spawn_argv` asserts the invariant, because a prompt that reaches it has already passed `validate_prompt`.

The test-only `validate_spawn_policy_argv` gains a pi arm that parses `--name <display_name>`, the optional `--model` and `--thinking` pairs and the `--`/prompt tail.

#### 1.3 Model selection

| Backend | Accepted value format | Create-time validation |
|---|---|---|
| `pi` | Passes through verbatim: `provider/id`, an exact id or a fuzzy name; omitted, pi's `defaultProvider` and `defaultModel` settings apply | The value names a model without a thinking suffix. When the text after its last `:` equals one of `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, creation exits 2 with `Error: --model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '<value>').` pi rejects unknown models itself. |

A `:` followed by any other text passes through, so provider ids such as OpenRouter's `:free` variants stay valid.

#### 1.4 Reasoning effort

| Backend | Accepted levels | Forwarded as | Rejected with (exit 2) |
|---|---|---|---|
| `pi` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | `--thinking <level>` | `Error: --effort for the pi backend must be one of off, minimal, low, medium, high, xhigh, max (got '<value>').` |

pi clamps a level to the model's capability, and a non-reasoning model runs `off`. CAFleet does no per-model check.

#### 1.5 Native version probe and profile boundary

pi uses the standard one-token rule (SPEC §6.9 Native version probe). The probe table gains the row `1.5.0` → `1.5.0`. The pi profile from `1.5.0` is pi's first boundary (§5), so `member create` refuses an earlier probed version with `Error: unsupported native version for pi: VERSION; minimum supported version is 1.5.0`. The Director rows record 1.5.0 as their minimum version.

#### 1.6 Config-dir resolution and member environment (SPEC §6.3)

| Backend | Base | Skills dir | Preset target | Recorded path |
|---|---|---|---|---|
| pi | `$PI_CODING_AGENT_DIR`, else `~/.pi/agent` | `<base>/skills` | — (setup installs the CAFleet pi extension under `<base>/extensions`; § Asset installation) | `<base>` |

- `PI_CODING_AGENT_DIR` follows the existing validation: an absolute path, valid UTF-8, read lazily.
- `member create` forwards a set `PI_CODING_AGENT_DIR` into the member environment, so the member loads the extension and skills that setup installed and spawn checked.
- `doctor` reports a resolution failure as `pi.configuration`.

#### 1.7 Availability check and contract errors

`ensure_available` fails on the first unmet precondition, in this order:

1. The pi base resolves.
2. `pi` is on `PATH`.
3. `<base>/extensions/cafleet.ts` is readable.

The third check is pi's analog of the OpenCode required preset, because every pi member depends on the extension's gate.

| Condition | Error |
|---|---|
| The CAFleet pi extension is missing or unreadable | `cannot read the CAFleet pi extension at {path}: {cause}; run 'cafleet setup --coding-agent pi' first` |

### 2. Asset: the CAFleet pi extension

#### 2.1 Layout, install target and executable record (SPEC §6.3 Asset installation)

One file, `extensions/pi/cafleet.ts`, is embedded verbatim through the `build.rs` `EXTENSIONS` table. It holds the Director extension (§4) and the member gate (§3).

| Agent | Bundled presets → install target | Bundled extension → install target | Executable record |
|---|---|---|---|
| pi | — | `extensions/pi/cafleet.ts` → `<base>/extensions/cafleet.ts` | `<base>/extensions/cafleet.executable` |

- pi discovery loads only `*.ts`, `*.js`, `*/index.ts` and `*/package.json`, so it ignores the record.
- Setup installs the skills, then the extension and its record. It runs no `pi` command, so it stays offline and needs no `pi` on `PATH`.
- Plain `cafleet setup` installs pi assets like every other backend.

Setup prints:

```
pi: installed cafleet, cafleet-design-doc (v<version>) -> <base>/skills
pi: installed extension (v<version>) -> <base>/extensions/cafleet.ts
```

The existing failure strings, the replace-before-write rule, the symlink rule and the single `asset_installs` row per agent apply unchanged.

#### 2.2 Doctor row (SPEC §6.3 `cafleet doctor`)

`pi.extension` follows `pi.executable` and precedes `pi.installation`. It joins the `opencode.extension` and `hermes.extension` rows of § Extension checks, with the same states:

| State | Status | Detail |
|---|---|---|
| The pi configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| `pi` is absent from `PATH` | `info` | `unavailable: executable is absent` |
| `<base>/extensions/cafleet.ts` is missing, unreadable or differs from the embedded bytes | `error` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent pi'` |
| The executable record is missing, unreadable or names no executable file | `error` | `<record path>: <cause>; run 'cafleet setup --coding-agent pi'` |
| Otherwise | `ok` | The extension path |

The `pi` summary line folds these through the existing extension phrases.

### 3. Member permission posture: the member gate

#### 3.1 Launch policy catalog (SPEC §6.9 Launch policy and capability registry)

| Backend | `auto` modes (default first) | `manual` modes | Explicit selection mapping |
|---|---|---|---|
| pi | `cafleet` (default) | `cafleet-ask` | Each mode maps to `--cafleet-gate <mode>` |

- `cafleet` is a fixed allow/deny policy: an allowlisted command runs, and every other bash command is blocked without a form.
- `cafleet-ask` is interactive approval: an unmatched bash command opens the gate's approval form.
- The default mode renders no approval form, so it meets the Member bar without request rules. The `cafleet-ask` request rules (§5) are bundled support.

| Selection | `requested_mode` | `resolved_mode` | `mode_source` | `permission_argv_json` |
|---|---|---|---|---|
| Omitted pi | `null` | `cafleet` | `default` | `["--cafleet-gate","cafleet"]` |
| Explicit pi `cafleet-ask` | `cafleet-ask` | `cafleet-ask` | `explicit` | `["--cafleet-gate","cafleet-ask"]` |

#### 3.2 Gate activation

The extension registers the string flag `--cafleet-gate`. On `session_start` the gate reads `pi.getFlag("cafleet-gate")`:

| Flag value | Gate |
|---|---|
| Absent (a Director or an operator session) | Inert: every `tool_call` passes through the gate untouched |
| `cafleet` | Enforces §3.3 with §3.4 for bash |
| `cafleet-ask` | Enforces §3.3, and opens the §3.5 form for a bash command that holds no control character and that §3.4 does not allow |
| Any other value | Blocks every tool call with `the CAFleet member gate received an unknown --cafleet-gate value '<value>'` |
| Reading the flag fails | Blocks every tool call with `the CAFleet member gate could not read --cafleet-gate: <cause>` |

An active gate shows `ctx.ui.setStatus("cafleet-gate", "cafleet gate: <mode>")`, so a pane capture shows the posture.

**Mode promise.** The gate's `tool_call` handler awaits a mode promise that the `session_start` handler settles in a `finally` block, with the resolved mode or with the read failure. A tool call is therefore never evaluated before the mode is known. It also never waits forever, and it never runs ungated after a failed read.

**Why one auto-loaded file instead of a separately installed gate.** Clarification C2 placed the gate outside auto-discovery, loaded through `-e <absolute path>`.
- That argv would embed the resolved base in `permission_argv_json`.
- Runtime observation re-derives the permission argv from `resolve_policy(requested_mode)` and compares it with the stored `permission_argv_json` (`src/runtime/observation.rs`), so a base-dependent argv breaks that check.
- An inert-by-default gate in the auto-loaded extension keeps the permission argv static.
- A Director session never passes `--cafleet-gate`, so the gate is inert there.
- The gate's patterns live as a constant in the same file (§3.4), which setup overwrites like any embedded asset.

**Why a gated member never runs ungated.** pi 1.5.0 rejects a long option that no loaded extension registers. With no CAFleet extension installed, `pi --cafleet-gate cafleet --offline --help` printed `Error: Unknown option: --cafleet-gate`.
- A gated member whose extension cannot register the flag exits at startup with that error. The extension might be absent after the §1.7 check, excluded from loading (for example disabled through `pi config`), a stale copy without the flag, or failing to load.
- The runner reports the exit as `runner_exited` with the native exit code, and the Director's inbox receives the event, instead of the member running ungated.
- `cafleet doctor` `pi.extension` and `cafleet setup --coding-agent pi` repair the installation. A-S records this exit (§7).

#### 3.3 Tool policy

| Tool | `cafleet` | `cafleet-ask` |
|---|---|---|
| `bash` | Block a command holding a control character (§3.4 rule 1); §3.4 allow, otherwise block | Block a command holding a control character without a form; §3.4 allow, otherwise the §3.5 form |
| `read` | Allow a path that is not a `.env` file | Same |
| `grep`, `find`, `ls` | Allow a path that is not a `.env` file; a missing or empty `path` is `.`, pi's documented default | Same |
| `edit`, `write` | Allow a path inside `ctx.cwd` that is not a `.env` file and not below a protected directory | Same |
| Any tool whose name begins with `cafleet_` | Block | Same |
| Any other tool, MCP and extension tools included | Block | Same |

- **Path input.** `read`, `edit` and `write` require a non-empty string `path`. Without one, the call is blocked, because the gate cannot evaluate a path it cannot read.
- A path resolves against `ctx.cwd` with `path.resolve`, without following symbolic links. A path is a `.env` file when its final component, compared case-insensitively, is `.env` or begins with `.env.`.
- **Protected directories.** `edit` and `write` are blocked for a path any of whose components, relative to `ctx.cwd` and compared case-insensitively, is `.git`, `.pi`, `.agents`, `.claude`, `.codex` or `.opencode`. CAFleet ships a macOS build, and the default macOS file system is case-insensitive, so `.Git/hooks/pre-commit` names the `.git` hook; Git guards `.GIT` for the same reason. These directories hold code and configuration that later run outside the allowlist:
  - `.git/hooks/*` runs under the allowlisted `git commit *`.
  - `.pi/extensions`, `.pi/settings.json`, `.pi/SYSTEM.md` and `.agents/skills` load into a trusted-project pi session started from the same directory, such as the Director on its next start or `/reload` (pi `docs/security.md` § Resources protected by project trust).
  - `.claude`, `.codex` and `.opencode` hold the project configuration of the other backends, such as Claude Code hooks and permission rules, which a Claude Code, Codex or OpenCode session of a mixed-backend fleet loads from the same directory.
  - Claude Code protects `.git` and `.claude` in the same way.
- Only bash opens a form, and only under `cafleet-ask`.
- `!` input (`user_bash`) runs outside `tool_call`, so the gate leaves Director-dispatched `member exec` commands to the Director's authorization.

**Block reasons.** A block reason reaches the member as the tool's error result. The gate returns exactly these reasons, besides the §3.2 activation failures; `<tool>` is the tool name. A file tool checks the path input, then the `.env` rule, then the working directory, then the protected directories.

| Case | Reason |
|---|---|
| bash command holding a control character, either mode | `the CAFleet member gate refuses a command holding a control character; route the command to the Director if it is required` |
| bash command not allowlisted, `cafleet` mode | `the CAFleet member gate allows only allowlisted simple commands; route the command to the Director if it is required` |
| bash command not allowlisted, `cafleet-ask` mode in a session without the interactive TUI | `the approval form needs the interactive pi TUI; route the command to the Director if it is required` |
| Form answered `Deny` | `the operator denied this command` |
| Form dismissed | `the approval form was dismissed; the command did not run` |
| `read` without a path input | `the CAFleet member gate cannot read the path input of read; route the read to the Director if it is required` |
| `read`, `grep`, `find` or `ls` on a `.env` file | `the CAFleet member gate protects .env files from <tool>; route the read to the Director if it is required` |
| `edit` or `write` without a path input | `the CAFleet member gate cannot read the path input of <tool>; route the change to the Director if it is required` |
| `edit` or `write` on a `.env` file | `the CAFleet member gate protects .env files from <tool>; route the change to the Director if it is required` |
| `edit` or `write` outside `ctx.cwd` | `the CAFleet member gate allows edit and write only inside the working directory; route the change to the Director if it is required` |
| `edit` or `write` below a protected directory | `the CAFleet member gate protects .git, .pi, .agents, .claude, .codex and .opencode from edit and write; route the change to the Director if it is required` |
| A tool whose name begins with `cafleet_` | `cafleet tools run only in a Director session` |
| Any other tool | `the CAFleet member gate allows only bash, read, grep, find, ls, edit and write` |

Each bash and file-tool reason that the gate decides without the form ends with a routing clause, so the member follows member routing: `route the command` for bash, `route the read` for `read`, `grep`, `find` and `ls`, and `route the change` for `edit` and `write`.

**Case-insensitive file-tool cases.** Both launch modes return these outcomes:

| Call | Outcome | Reason |
|---|---|---|
| `write` to `.Git/hooks/x` | Blocked | The protected-directory reason; `.Git` compares equal to `.git` |
| `read` of `.ENV` | Blocked | The `.env` reason for `read`; `.ENV` compares equal to `.env` |
| `write` to `.ENV` | Blocked | The `.env` reason for `write` |
| `write` to `src/.Github/x.yml` | Allowed | `.github` is no protected directory |

#### 3.4 Bash allowlist and matching

The allowlist translates the OpenCode `cafleet` preset's shell allowlist and is the gate's content contract:

```ts
const MEMBER_ALLOWLIST = [
  "git add *", "git commit *", "git diff *", "git grep *", "git log *", "git ls-tree *",
  "git ls-files *", "git branch *", "git status", "grep *", "ls", "ls *", "stat *",
  "tree", "mise //:test", "mise //:test *", "mise //:lint", "mise //:format",
  "mise //:typecheck", "mise //:build", "wc *", "cafleet *",
];
```

A command is *allowlisted* when all of these hold:

1. **Simple.** The command holds no control character, and, trimmed of surrounding whitespace, is a single simple command.
   - **Control characters.** In both launch modes, a bash command holding a C0 control character other than tab (U+0000–U+0008, U+000A–U+001F), DEL (U+007F) or a C1 control (U+0080–U+009F), anywhere and quoted or not, is blocked with the control-character reason (§3.3) before any allowlist check or approval form. A newline or carriage return separates commands, and bash removes a backslash-newline pair even inside double quotes. pi 1.5.0 passes the form title to the terminal without stripping control sequences, so an escape sequence or carriage return would render an operation other than the command bash runs.
   - A lexer tracks single quotes, double quotes and backslash escapes, and splits the command into words at unquoted spaces and tabs, removing quotes and escapes.
   - The command is not simple when an unquoted `;`, `&`, `|`, `<`, `>`, `(`, `)` or backtick, or a backtick inside double quotes, appears, or when a quote is unterminated.
   - **The `$` rule.** An unescaped `$` outside single quotes, unquoted or inside double quotes, is accepted only when the next character starts a name (`A-Z`, `a-z` or `_`). Any other `$` makes the command not simple: `$[`, `$(`, `${`, `$'`, `$"`, `$((`, `$?`, `$1`, `$$` and a trailing `$` among them. The rule is an allow-list, because several `$` forms hide a command substitution from the lexer, such as the legacy arithmetic expansion `$[...]`, which bash expands even inside its single quotes.
   - Single-quoted text is literal.
2. **Literal git words.** A command whose first word is `git` has no word holding a `$` outside single quotes or an unquoted `*`, `?`, `[` or `{`. Bash expands such a word after the gate checks it, so parameter, brace and pathname expansion could build an option word the gate never sees; a member can create a file named `-O<cmd>` with `write`.
3. **No command-running or file-writing option.** These options run a command or write chosen text to any path, including the protected directories and paths outside the cwd:
   - A `git` command has no word that begins with a single `-` and contains `O` (`git grep -O<cmd>`, also bundled as `-iO<cmd>`), and no word beginning with `--` whose name before any `=` is `output` or a prefix of `open-files-in-pager` at least two characters long, because git grep accepts abbreviated long options.
   - The rule over-blocks harmless words such as `git log -SFOO`, which the member routes.
4. **Matching.** The whole trimmed command matches at least one pattern, where `*` matches any character sequence, spaces included, and every other character matches itself.

`grep`, `ls`, `stat` and `wc` keep globs and `$NAME`, because none of them has an option that runs a command or writes a file. `tree` runs only bare, through the exact `tree` pattern, because its options write files: `-o` writes the listing to a chosen path, and `-R` writes `00Tree.html` into every listed directory, protected directories included. The lexer makes compound-command handling a decided rule. OpenCode records compound matching as unverified. A leading `NAME=value` assignment matches no pattern, so a member passes `--database-url` instead.

**Gate case table.** The project's tests are Rust-only and the gate is TypeScript, so A-M runs this table through a gated member as the gate's acceptance check (§7). `<own id>` is the member's literal ID, `<newline>` is a line break inside the command, and `<ESC>` is the byte U+001B.

| # | Command | Outcome under `cafleet` | Rule |
|---|---|---|---|
| 1 | `git status` | Allowed | Exact pattern |
| 2 | `  git status  ` | Allowed | Trimmed, then exact |
| 3 | `git log --oneline -3` | Allowed | `git log *`; `--oneline` is no refused option |
| 4 | `ls 'a;b'` | Allowed | `;` inside single quotes is literal |
| 5 | `git log "a;b"` | Allowed | `;` inside double quotes is literal |
| 6 | `ls "$HOME"` | Allowed | A `$` followed by a name passes the `$` rule, and `ls` words may expand |
| 7 | `ls *.md` | Allowed | `ls` words may hold a glob |
| 8 | `git grep -e 'a*b' -- '*.rs'` | Allowed | A single-quoted glob is literal |
| 9 | `cafleet message poll <own id> --json` | Allowed | `cafleet *` |
| 10 | `tree` | Allowed | Exact pattern |
| 11 | `git status; rm x` | Blocked | Unquoted `;` |
| 12 | `git status && true` | Blocked | Unquoted `&` |
| 13 | <code>git log &#124; head</code> | Blocked | Unquoted pipe |
| 14 | `ls > f` | Blocked | Unquoted `>` |
| 15 | `git log $(id)` | Blocked | `$` rule: unquoted `$(` |
| 16 | `git log "$(id)"` | Blocked | `$` rule: `$(` inside double quotes |
| 17 | ``git log `id` `` | Blocked | Backtick |
| 18 | `ls "a` | Blocked | Unterminated quote |
| 19 | `git status` and `ls` on two lines | Blocked in both modes, no form | Control character: newline |
| 20 | `git log "--\<newline>output=f"` | Blocked in both modes, no form | Control character: newline inside double quotes (bash removes the backslash-newline pair) |
| 21 | `git grep "--\<newline>open-files-in-pager=echo x" -e y` | Blocked in both modes, no form | Control character: newline inside double quotes |
| 22 | `tree "\<newline>-o" f` | Blocked in both modes, no form | Control character: newline inside double quotes |
| 23 | `ls <ESC>[2K` | Blocked in both modes, no form | Control character: ESC |
| 24 | `ls $'\'' ; echo BYPASSED # '` | Blocked | `$` rule: unquoted `$'` (ANSI-C quoting) |
| 25 | `ls $"a;b"` | Blocked | `$` rule: unquoted `$"` (locale quoting) |
| 26 | `ls "${HOME}"` | Blocked | `$` rule: `${` inside double quotes |
| 27 | `git grep "${HOME/*/-Oecho x}" -e y` | Blocked | `$` rule: `${` inside double quotes (parameter expansion) |
| 28 | `ls $['$(id)'0]` | Blocked | `$` rule: unquoted `$[` (legacy arithmetic expansion) |
| 29 | `stat $[a['$(id)']]` | Blocked | `$` rule: unquoted `$[` (array subscript inside arithmetic) |
| 30 | `ls "$[1]"` | Blocked | `$` rule: `$[` inside double quotes |
| 31 | `ls $?` | Blocked | `$` rule: a special parameter |
| 32 | `git log "$HOME"` | Blocked | A git word holds `$` |
| 33 | `git grep {-O,-O}"echo x" -e y` | Blocked | A git word holds an unquoted `{` (brace expansion) |
| 34 | `git grep -e y *` | Blocked | A git word holds an unquoted `*` (pathname expansion) |
| 35 | `git grep -O"echo x" -e y` | Blocked | A single-dash git word contains `O` |
| 36 | `git grep -iO"echo x" -e y` | Blocked | A single-dash git word contains `O` (bundled) |
| 37 | `git grep --open-files="echo x" -e y` | Blocked | `--open-files` abbreviates `--open-files-in-pager` |
| 38 | `git diff --output=f` | Blocked | `--output=` |
| 39 | `git log --output f` | Blocked | `--output` |
| 40 | `tree -o f` | Blocked | No pattern matches |
| 41 | `tree -ao f` | Blocked | No pattern matches |
| 42 | `tree -R -L 1` | Blocked | No pattern matches |
| 43 | `FOO=1 cafleet message poll <own id>` | Blocked | Leading assignment matches no pattern |
| 44 | `git push` | Blocked | No pattern matches |
| 45 | `echo hi` | Blocked | No pattern matches |

None of the blocked cases runs. A control-character case returns the control-character reason in both launch modes and opens no form. Every other blocked case returns the `cafleet` bash reason (§3.3), and under `cafleet-ask` opens the §3.5 form instead.

#### 3.5 The `cafleet-ask` approval form

For a bash command that holds no control character and is not allowlisted, the gate calls `ctx.ui.select(title, ["Allow once", "Deny"])` with this CAFleet-authored title:

```
CAFleet member gate: allow this bash command?

$ <command>
```

A command holding a control character never reaches the form (§3.4 rule 1), so the title shows the command on one line, exactly as bash runs it.

| Result | Gate action |
|---|---|
| `Allow once` | Run the command once |
| `Deny` | Block: `the operator denied this command` |
| `undefined` (Escape, Ctrl+C) | Block: `the approval form was dismissed; the command did not run` |

- The form has no timeout and stays open until it is answered.
- The CAFleet title, the two choice labels and pi's select hint line are the request rule's anchors (§5).
- One form is open at a time: concurrent bash calls of one assistant message are evaluated in sequence through one promise chain.
- "Allow once" grants no persistent approval.
- A dismissal reason tells the member that a delivery-dismissed prompt can be reissued (member role § Delivery-dismissed prompts).

#### 3.6 Limits

| Surface | Limit |
|---|---|
| Allowed tooling | An allowed command also runs the code and hooks it executes: `mise //:test *` runs workspace test code, and `git commit *` runs `.git/hooks`. §3.4 refuses the git options that run a command or write a chosen file, and allows `tree` only bare |
| No sandbox | pi has no OS sandbox. `read`/`grep`/`find`/`ls` reach any path, and a symbolic link inside the cwd can target a path outside it |
| `.env` files | The `.env` rule refuses only a `read`, `grep`, `find`, `ls`, `edit` or `write` call whose path names a `.env` file. A directory search (`grep` on `.`) and allowlisted commands (`grep '' .env`, `git grep`, `git diff`, `git log -p` on a tracked `.env`) still read `.env` contents |
| Protected directories | `edit` and `write` are blocked below `.git/`, `.pi/`, `.agents/`, `.claude/`, `.codex/` and `.opencode/` in both launch modes (§3.3), so a member cannot plant a Git hook, a project pi resource or another backend's project configuration that a later session runs. The path check follows no symbolic link, so a link elsewhere in the cwd that targets one of these directories stays writable. An allowlisted command writes where its own tool writes, as `git` does under `.git/` |
| Project files | Project-root configuration files such as `opencode.json` and context files such as `AGENTS.md` and `CLAUDE.md` stay writable, because members edit them as ordinary work. A later session of any backend loads what a member writes there |
| Delivery fallback | Escape interrupts a running pi turn and cancels an open form. While coverage is `tested`, profile delivery queues instead of falling back to Escape; a `coverage=unknown` member's Escape-first delivery interrupts its run or dismisses the form |
| Multi-line commands | Every command holding a newline is blocked in both launch modes without a form, including `cafleet message send` with a multi-line positional body (observed in the round-2 re-check). A member sends a multi-line body with `--file PATH` from a file written inside the cwd |

### 4. Director extension

#### 4.1 Director verification and member inertness

Members auto-load the same file. Every CAFleet tool takes a required `director_id: integer` and an optional `database_url: string`. Before any work, it runs `<recorded cafleet> member show <director_id> --json` (adding `--database-url <url>` when given) through `node:child_process` `execFile`. It requires exit 0 with `"kind": "director"`. Otherwise it returns `{"ok": false, "error": "cafleet tools run only in a Director session: <stderr>"}` with `isError: true`.

- Each tool's result text is its JSON object.
- The tools read the executable record at `new URL("./cafleet.executable", import.meta.url)`. A missing or unusable record returns `{"ok": false, "error": "executable record <path>: <cause>; run 'cafleet setup --coding-agent pi'"}`.
- The goal hooks act only on a session holding a goal set by the operator-typed `/cafleet-goal` command or by a goal tool whose `director_id` verified, so the hooks stay inert in members.
- An active member gate blocks every `cafleet_*` tool first (§3.3).

#### 4.2 Receiver: `cafleet_listen(director_id, database_url?)`

1. Verify the Director (§4.1).
2. When this extension runtime already owns a live receiver, return `{"ok": true, "pid": <pid>, "sessionId": <id>, "existing": true}`.
3. Spawn `<recorded cafleet> message listen <director_id>` (plus `--database-url <url>`, without `--timeout`) with `node:child_process` `spawn`, in the pi process's environment. When `child.pid` is undefined, return `ok: false` with the spawn error.
4. Return `{"ok": true, "pid": <pid>, "sessionId": <ctx.sessionManager session id>}`.
5. For each stdout line, call `pi.sendMessage({customType: "cafleet-listen", content: "[cafleet listen record] <line>", display: true}, {triggerTurn: true, deliverAs: "followUp"})` in line order.

`followUp` starts a turn on an idle session. On a busy session it waits until the agent has no more tool calls, so a record never interrupts a running turn and arrives no later than its end. Records that arrive while a run is starting join that run.

| Event | Extension action |
|---|---|
| The child closes (`close` event, after its stdio drains) without a `closed` record, and the extension did not stop it | Send `[cafleet listen closed] exit <code>: <last stderr line>` the same way |
| A child `error` event, such as an asynchronous spawn failure | Remove the receiver and send the same closed notice |
| A `sendMessage` call throws | Kill the child and send the closed notice once |
| A `closed` record | Delivered as a record; the receiver ends |

The extension replaces no receiver automatically. Every replacement belongs to the Director's receiver recovery.

#### 4.3 Receiver lifecycle

`session_shutdown`, for every reason, marks the receiver stopped and kills the child with SIGTERM. A stopped receiver sends no closed notice.

When the reason is `reload`, `new`, `resume` or `fork`, the pi process keeps running and loads a replacement runtime. The old runtime stores `{reason}` under `globalThis[Symbol.for("cafleet.pi.receiver-stopped")]`. The new runtime's `session_start` reads and clears that value, then sends `[cafleet listen closed] the extension runtime was replaced (<reason>); re-arm with cafleet_listen` with `triggerTurn: true`. Receiver recovery then re-arms reception in the current session. `quit` stores nothing, because the process exits.

A SIGKILL of the pi process runs no handler. A-P records whether a pane kill and `/quit` each leave no `cafleet message listen` process.

#### 4.4 Goal extension

State `{objective, status, generation, continuations, idle_streak}` is persisted with `pi.appendEntry("cafleet-goal", state)` after every change. `status` is `active`, `paused`, `complete` or `cleared`. On `session_start` the state is restored from the last `cafleet-goal` entry on `ctx.sessionManager.getBranch()`.

| Operation | Surface | Effect |
|---|---|---|
| set | `/cafleet-goal <objective>` | Status `active`, next generation, both counters 0; send the first continuation |
| status | `/cafleet-goal status`; tool `cafleet_goal_status` | The command shows `[cafleet goal] status: <json>` through `ctx.ui.notify`; the tool returns the stored JSON |
| pause | `/cafleet-goal pause` | Status `paused`, bump `generation` |
| resume | `/cafleet-goal resume` | Status `active`, bump `generation`, reset both counters, send a continuation |
| clear | `/cafleet-goal clear` | Status `cleared`, bump `generation`; the objective is dropped |
| complete | tool `cafleet_goal_complete(director_id, summary, database_url?)` | Status `complete`, bump `generation` |
| progress | tool `cafleet_goal_progress(director_id, note, database_url?)` | Reset `idle_streak` |

- **Generations.** Generations increase monotonically per session. The `cleared` entry keeps the last generation, and set uses that value plus one.
- **Continuation message.** A continuation is a `custom_message` with `customType` `cafleet-goal`, `details: {generation}`, `display: true` and the content `[cafleet goal] Continue working toward the active goal: <objective>. Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved.` Set and resume send it with `pi.sendMessage(..., {triggerTurn: true, deliverAs: "followUp"})`.
- **Automatic continuation.** `agent_before_settle` acts on a session whose goal is `active`:
  1. If the run was a continuation run without a `cafleet_goal_progress` or `cafleet_goal_complete` call, increment `idle_streak`.
  2. At 3, pause, and return the entry `[cafleet goal] paused: no progress in 3 consecutive continuation turns; the operator resumes it with /cafleet-goal resume` with `continue: false`.
  3. At 50 continuations, pause, and return `[cafleet goal] paused: reached 50 continuations; the operator resumes it with /cafleet-goal resume`.
  4. Otherwise increment `continuations`, persist, and return the continuation entry with `continue: true`.
- **Tracking.** The extension keeps two in-memory flags and uses no `before_agent_start` hook, because pi 1.5.0 skips that hook for a continuation that `agent_before_settle` requests (`docs/extensions.md` § Lifecycle Overview).
  - `pendingContinuation` is set whenever the extension starts a continuation: when `agent_before_settle` returns `continue: true`, and when set or resume sends the continuation message.
  - `progressed` is set by every `cafleet_goal_progress` or `cafleet_goal_complete` execution.
  - Each `agent_before_settle` reads both flags for step 1 of the automatic continuation and clears them before it returns. A returned continuation sets `pendingContinuation` again.
  - An aborted run clears both flags at `agent_settled`.
- **Interrupt.** `agent_settled` with `event.aborted` on an active goal pauses it and shows `[cafleet goal] paused: the run was interrupted; the operator resumes it with /cafleet-goal resume` through `ctx.ui.notify`.
- **Stale continuations.** The `context` hook removes each `cafleet-goal` continuation after the last assistant message whose generation differs from the stored one, or whose goal is not `active`. Earlier history stays intact.
- **Goal context.** Custom entries stay outside LLM context. The `context` hook runs before every model request, inside a continuation chain too, and therefore carries the objective.
  - While the goal is `active`, it appends one transient user-role message, `[cafleet goal] Active CAFleet goal: <objective>`, after the last message of `event.messages`.
  - The message is not persisted and is rebuilt on every request, so it survives manual and threshold compaction alike.
  - Appending at the tail keeps the provider's cached prefix intact.
  - The hook appends nothing for a `paused`, `complete` or `cleared` goal, so a paused goal gives the model no standing instruction. Resume sends the objective again in its continuation message.
- **Restart.** `pi --continue` (or `--session <id>`) on the same session restores the state at `session_start`. The Director's re-arming `cafleet_listen` turn then ends at `agent_before_settle`, which continues the active goal without further input.
- **Failures.** Each handler is wrapped in `try`/`catch`. A failure logs its cause. While the goal is `active`, it also pauses the goal and shows `[cafleet goal] paused: the goal cannot continue (<cause>); the operator resumes it with /cafleet-goal resume`.

#### 4.5 Decision surface: `cafleet_ask(director_id, question, choices, database_url?)`

1. Verify the Director (§4.1).
2. Require `ctx.hasUI`. Otherwise return `{"ok": false, "error": "cafleet_ask needs the interactive pi TUI"}`.
3. Require `choices` to be 1–9 non-empty, distinct strings, none equal to `Other`. Otherwise return `ok: false` naming the violated rule.
4. Call `ctx.ui.select(question, [...choices, "Other"])`. When the result is `Other`, call `ctx.ui.input(question, "")`.

| Outcome | Result |
|---|---|
| A listed choice | `{"ok": true, "status": "answered", "answer": "<choice>", "choice": <1-based index>}` |
| `Other` with non-empty text | `{"ok": true, "status": "answered", "answer": "<text>", "choice": null}` |
| Cancelled, or `Other` with empty text | `{"ok": true, "status": "cancelled"}`, which records no decision |

The tool registers with `executionMode: "sequential"`.

#### 4.6 Director launch and reception recipe

The Director runs plain `pi` (fullscreen TUI) from the project directory, so the extension auto-loads. pi's bash tool inherits the pane environment, so `cafleet fleet create --name <name> --coding-agent pi --json` registers the pane without `--pane`. A pi Director runs its own tool calls without approval; the member gate is inert in its session.

The `#### pi native Director reception` recipe in coding-agents.md:

1. After registration, call `cafleet_listen` with the literal Director ID as `director_id`, adding `database_url` for a non-default database. Require `ok` and the `ready` record; a repeat call returns `existing: true`.
2. Each record arrives as a `[cafleet listen record]` message: it starts a turn when you are idle and otherwise arrives at the end of the running turn. Consume it, reconcile with `cafleet message poll <director-id> --json`, record deferred actions, ACK and act within authorization.
3. The operator sets, pauses, resumes and clears the goal with `/cafleet-goal`; `/cafleet-goal status` and `cafleet_goal_status` report it.
4. While working toward an active goal, report each step with `cafleet_goal_progress` and finish with `cafleet_goal_complete`, passing `director_id` and, when needed, `database_url`. Reception itself calls no goal tool.
5. A `[cafleet listen closed]` message (including the runtime-replaced notice after `/reload`, `/new`, `/resume` or `/fork`), an `ok: false` result or a missing tool follows receiver recovery. A missing tool or an executable-record error is fixed by `cafleet setup --coding-agent pi`, which `cafleet doctor` names.

After a restart on the same session, re-arm on the first turn; that turn also continues an active goal. Reception makes no pane input.

### 5. Bundled runner profile: the pi profile from `1.5.0`

The profile applies to pi under every pi launch mode. Only `cafleet-ask` shows an approval form.

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Request rule `pi-gate-approval`: the §3.5 form, anchored on the `CAFleet member gate: allow this bash command?` title row, the `$ ` operation rows, the `Allow once` and `Deny` choice rows and pi's select hint row. The operation is the `$ ` row plus its two-space continuation rows. |
| Allow answer | `tested` | One stage: Enter (`0d`) gated on focus `allow-once`, as `allow-once` |
| Deny answer | `tested` | Two stages: `j` (`6a`) gated on focus `allow-once`, then Enter (`0d`) gated on focus `deny-once`, as `deny-once` |
| Empty-composer observation | `tested` | `pi-idle-composer`: the idle editor between its two full-width `─` border rows, with the terminal cursor in the editor |
| Prompt submission | `tested` | Composer plan `prompt`, submit `0d` |
| Native-exec submission | `tested` | Composer plan `exec` for `! COMMAND`, submit `0d` |
| Annotations | `unknown` | None declared |

The captures (§7 A-F) establish these values, which SPEC §6.9 records with the rule patterns:

| Element | Measured value |
|---|---|
| Screen | The alternate screen (`--tui-mode fullscreen`) |
| Focus | A textual `→ ` marker before the focused label, so the focus matcher declares `focus_style` `marker` |
| Deny stage 1 | pi's selector alias `j` (`6a`). The Down key's CSI bytes begin with Escape, which answer stages never contain and which dismisses the form |
| Cursor | Hidden; on the form it sits on the footer or the focused choice row (`cursor` `any`), and in the idle editor on the editor row (`cursor` `inside`) |
| Composer plans | Prefix `""` (prompt) or `2120` (exec), submit `0d`, with the shared 500 ms settle interval |
| `W_min` | 49 columns; at 48 the hint row wraps |
| Recorded layout limits | A pane narrower than `W_min`, a wrapped command line, and a form taller than 24 rows or the pane. A resize while the form is open redraws it and is not a limit |

A form inside a recorded layout limit opens no `interaction_opened` event and is answered manually on the native surface. The deny answer sends no Escape, because Escape dismisses the form (§3.5).

Fixtures live in `src/coding_agent/native/pi.json` with ids `pi-1.5.0-<screen>-<multiplexer>-<width>-1`. They are captured on tmux and with herdr's terminal identity at 120 columns and at `W_min`, and cover the form, each answer's pre-stage screen, the idle composer and both submission plans. The test fixture registry in `src/coding_agent/catalog.rs` embeds the file beside the other backend fixture files.

### 6. Backend documentation content

`skills/cafleet/reference/coding-agents.md` gains `## pi` after `## hermes`, with every subsection of the documentation-maintenance Template.

**Runtime bindings**

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the `cafleet_ask` tool of the CAFleet pi extension, with the literal Director ID, one question and its choices; an `answered` result records the decision (members relay) |
| `{permission_flags}` | `--cafleet-gate cafleet` (default, class `auto`: the CAFleet member gate's deny-by-default bash allowlist and protected `.git`/`.pi`/`.agents`/`.claude`/`.codex`/`.opencode` directories, no form); explicit `cafleet-ask` (class `manual`: an unmatched bash command opens the gate's approval form) |
| `{pane_title}` | `--name <member-name>`; pi titles the terminal `π - <name> - <cwd basename>` |
| `{skill_loader}` | pi's skill loader: pi lists each installed skill's name, description and `SKILL.md` path; read that `SKILL.md` with the `read` tool |
| `{effort_levels}` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`, forwarded as `--thinking <level>`; pi clamps the level to the model) |

**Role defaults.** All four placeholders are `—`: omit `--model` and `--effort`, so the operator's configured pi default applies.

**Model catalog.** No maintained catalog. `--model VALUE` passes through verbatim as pi's `--model VALUE` (`provider/id`, an exact id or a fuzzy name) without a `:<thinking>` suffix, and pi rejects unknown models. Omitted, the member uses `defaultProvider` and `defaultModel` from `<base>/settings.json`.

**Note → applies at.** These rows apply, each bound to its consuming instruction:

| Note | Applies at |
|---|---|
| The CAFleet pi extension (`<base>/extensions/cafleet.ts` with its `cafleet.executable` record) auto-loads in every pi session. It provides `cafleet_listen`, `cafleet_ask`, `/cafleet-goal` and the goal tools, each verifying its literal `director_id` and otherwise refusing with `cafleet tools run only in a Director session: <stderr>` | `{decision_surface}`; `cafleet/reference/cli.md` § `setup` and `doctor`; `cafleet/roles/director.md` § Director wake; `cafleet/roles/member.md` § Reconsider, then route |
| Default `cafleet` mode (class `auto`): the member gate runs each allowlisted simple command, with `tree` only bare. It blocks every other bash command, including any `$` outside single quotes that is not followed by a name character (`A-Z`, `a-z` or `_`), such as `$(`, `${`, `$[`, `$'`, `$?` or `$1`, a `git` word holding `$` or an unquoted `*`, `?`, `[` or `{`, and the options `git grep -O`/`--open-files-in-pager` and `--output`, with a reason ending in `route the command to the Director if it is required`, and every tool outside bash, read, grep, find, ls, edit and write. In both launch modes, a command holding a C0 control character other than tab, DEL or a C1 control, such as a newline, carriage return or ESC, quoted or not, is blocked without a form with `the CAFleet member gate refuses a command holding a control character; route the command to the Director if it is required`. `read`, `edit` and `write` need a non-empty `path`, and a file-tool reason ends in `route the read …` or `route the change …`. Route a required blocked command or file access | `{permission_flags}`; `cafleet/roles/member.md` § Reconsider, then route; `cafleet/roles/director.md` § Director-side dispatch |
| In both launch modes, `edit` and `write` are blocked below `.git/`, `.pi/`, `.agents/`, `.claude/`, `.codex/` and `.opencode/`, compared case-insensitively (`.Git/hooks` included), with `the CAFleet member gate protects .git, .pi, .agents, .claude, .codex and .opencode from edit and write; route the change to the Director if it is required`, because Git hooks, project pi resources and other backends' project configuration run outside the allowlist in later sessions. Project-root files such as `opencode.json`, `AGENTS.md` and `CLAUDE.md` stay writable. The path check follows no symbolic link. The `.env` rule refuses only a file-tool call whose path names a `.env` file, compared case-insensitively (`.ENV` included); a directory search and allowlisted commands such as `grep '' .env` or `git diff` still read `.env` contents | `{permission_flags}`; `cafleet/roles/member.md` § Reconsider, then route; `cafleet/roles/director.md` § Director-side dispatch |
| The gate refuses a bash command holding a newline, so a pi member sends a one-line body as the positional `TEXT`, and a multi-line body with `--file PATH` from a file it writes inside the working directory | `cafleet/SKILL.md` § Send (Unicast); `cafleet/reference/cli.md` § `message send` |
| Explicit `cafleet-ask` (class `manual`): an unmatched bash command without a control character opens the `CAFleet member gate` form. The member waits while the Director answers it; a deny returns `the operator denied this command` to the member, which continues its run | `{permission_flags}`; `cafleet/roles/director.md` § Consume observed interactions; `cafleet/roles/member.md` § Native interactions |
| A pane runs `pi --cafleet-gate <mode> --no-approve --tui-mode fullscreen --name <name>`. Optional `--model` and `--thinking` follow, then `--` and the prompt, as a fresh session. `member create` forwards a set `PI_CODING_AGENT_DIR`, and a gated member whose extension cannot register `--cafleet-gate` exits at startup (`runner_exited`) | `{permission_flags}`; `cafleet/reference/cli.md` § `member create` |
| Leading-`!` input runs the command and stores its output without starting a turn. After `member exec`, send an explicit continuation message so the member acts on the output | `cafleet/roles/director.md` § Director-side dispatch; `cafleet/reference/cli.md` § `member exec` |
| Escape interrupts a running pi turn and dismisses an open gate form, and Enter while the member works queues the text as a steering message | `cafleet/roles/member.md` § Command execution; `cafleet/reference/cli.md` § `member prompt` |
| A pi Director runs plain `pi` from the project directory, registers with plain `fleet create`, and replaces its receiver after `/reload`, `/new`, `/resume` or `/fork` when the runtime-replaced notice arrives | `cafleet/roles/director.md` § Spawn Protocol and § Receiver recovery |

The native reception recipe (§4.6) follows the Note table.

**Role eligibility.**

| Role | Eligible | Basis |
|---|---|---|
| Director | from A-P | `goal` `extension` and `listen` `extension` `subscribe` through the CAFleet pi extension (1.5.0 minimum), or each unmet requirement with its observed failure |
| Member | from A-M | The omitted `cafleet` gate mode, accepted on tmux and herdr; the pi profile from `1.5.0` |

SPEC § Role eligibility gains the `pi` rows: `goal`, `listen` and `verdict` for the Director, plus one Member row. Each carries mechanism, `1.5.0` and `checked YYYY-MM-DD` from §7.

### 7. Live acceptance (implementation-time)

**Isolation.** As in 0000217: during implementation, A-S runs the freshly built binary against a temporary `PI_CODING_AGENT_DIR` and database. The model runs (A-F, A-P, A-M) need the operator's pi credentials, so they use the real home. They start only after every implementation member is deleted and no executing member depends on the installed assets; the Director then reinstalls the CLI and runs `cafleet setup --coding-agent pi`. Every model run uses the operator default `openrouter` / `moonshotai/kimi-k2.6`; no cost cap applies.

| Run | Steps |
|---|---|
| A-S | Built binary, temporary home:<br>• `cafleet setup --coding-agent pi` on a clean home and a rerun (idempotent output and bytes)<br>• doctor `pi.extension` `ok`, then `error` for a tampered extension and for a missing record<br>• `PI_CODING_AGENT_DIR=<tmp> pi --help` lists `--cafleet-gate`<br>• pi's startup diagnostics load both CAFleet skills without a warning<br>• `member create --coding-agent pi` with the extension removed fails with the §1.7 error, and a prompt beginning with `@` fails with the §1.2 usage error, leaving no member row or pane<br>• a gated member whose extension passes the §1.7 check but cannot register the flag exits at startup with `Error: Unknown option: --cafleet-gate`, reported as `runner_exited`. The extension is excluded from loading (for example disabled through `pi config`), or replaced by a copy that registers no `--cafleet-gate`. pi rejects the option before any model call, so the temporary home needs no credentials |
| A-F | Fixture capture on pi 1.5.0, on tmux and herdr, at 120 columns and while narrowing to `W_min`: the `cafleet-ask` form, each answer stage, the idle composer, prompt and exec submissions. Converts the captures into `native/pi.json` |
| A-P | pi Director (plain `pi`):<br>• subscribe probe (idle wake, two records during a ≥ 60 s foreground command, a burst of 3 in order)<br>• `/quit` and a pane kill each leave no listener<br>• `/reload` delivers the runtime-replaced notice<br>• goal bar: set, status, pause, resume, clear, complete, `/compact` with the objective retained, `/quit` then `pi --continue` restoring the goal state and re-arming reception, automatic continuation, the no-progress stop after 3 idle automatic continuations in one chain, and an interrupt pausing<br>• `cafleet_ask` with a listed choice, with `Other` and text, and cancelled<br>• `cafleet_listen`, `cafleet_ask` and `cafleet_goal_status` blocked by the gate in a gated member session; the operator-typed `/cafleet-goal` works in any session (§4.1) |
| A-M | pi Members on tmux and on herdr:<br>• omitted `cafleet`: broker commands run without a form; the member runs every §3.4 gate case table row verbatim as one bash call, and each outcome matches the table; a blocked required command is routed to the Director<br>• omitted `cafleet`: `edit` and `write` below each protected directory (§3.3) are blocked with the protected-directory reason, while a workspace file elsewhere is written<br>• `cafleet-ask`: the form raises `interaction_opened`; `permission respond` allow runs the command once, and deny blocks it, with the member continuing<br>• `member exec` `! COMMAND` stores output without a turn, then a continuation message acts on it<br>• prompt delivery to an idle and to a busy member |

Each row's verdict is written only from its observed result. A failed run records `no` with the observed failure, and the other role still ships.

**Director decisions on unobserved acceptance steps.** The Director accepts these steps without a live observation, each on its basis:

| Step | Decision | Basis |
|---|---|---|
| A threshold compaction inside an automatic continuation chain | Accepted by construction | The `context` hook appends the goal message before every model request (pi `docs/extensions.md` § context), and a manual `/compact` retained the goal. A threshold compaction needs a context near the model's window and was not run |
| The re-arm turn continuing an active goal after `pi --continue` | Accepted from its parts | Goal restore after a restart and automatic continuation were each observed; their combination was not |
| `member delete` of a pi Director-shaped probe leaving no listener | Not applicable | Every pi member runs the gate, which blocks `cafleet_listen`, so no member owns a receiver. The pane kill and `/quit` cases cover receiver end |
| The effect of an Escape-first delivery on a running turn | Not applicable while coverage is `tested` | Profile delivery queues instead of falling back to Escape; the fallback applies only to a `coverage=unknown` member (§3.6) |

### 8. Clarification decisions

| Item | Decision |
|---|---|
| A1–A3 | Token `pi`, last in registry order; 1.5.0 minimum for both roles; no maintained model catalog |
| B1 | Argv of §1.2, with `--` added before the prompt and the leading-`@` rule checked before registration |
| B2 | `--model` passes through verbatim; a `:<thinking>` suffix is a usage error |
| B3, B4 | `--effort` maps to `--thinking`; the config dir and install targets of §1.6 and §2.1 |
| C1 | `cafleet` (default, `auto`) and `cafleet-ask` (`manual`) with its profile request rule, answer plans and tmux/herdr fixtures; every mode runs the gate |
| C2 | Refined: the gate is inert by default inside the one auto-loaded extension and activated by `--cafleet-gate` (§3.2 rationale), with the allowlist as a constant in that file. pi rejects the flag when the extension cannot register it, so a gated member never runs ungated |
| Review round 1 | Director decision: in both launch modes, `edit` and `write` are blocked below `.git/`, `.pi/` and `.agents/` (§3.3, §3.6, the `{permission_flags}` Note) |
| Review round 2 | Director decisions: a command holding a newline or carriage return, quoted or not, is not simple (§3.4); `.claude`, `.codex` and `.opencode` join the protected directories, while project-root files such as `opencode.json`, `AGENTS.md` and `CLAUDE.md` stay writable (§3.3, §3.6, the `{permission_flags}` Note) |
| Review round 3 | Director decisions: an unescaped `$` outside single quotes is accepted only before a name character, and every other `$` makes a command not simple (§3.4, the `{permission_flags}` Note). Four unobserved acceptance steps are accepted with their bases (§7 Director decisions): threshold compaction by construction of the `context` hook, the restart re-arm continuation from its observed parts, and `member delete` of a Director-shaped probe and the Escape-first delivery effect as not applicable |
| Review round 4 | Director decisions: `tree` runs only bare, and the tree option rule is dropped, because tree kept exposing file-writing options (`-o`, `-R`) (§3.4); protected-directory components and the `.env` basename compare case-insensitively (§3.3); in both launch modes a bash command holding a C0 control character other than tab, DEL or a C1 control is blocked before any allowlist check or form with its own reason (§3.3–§3.5, the `{permission_flags}` Note) |
| C3, C4 | Members pass `--no-approve`; the tools verify the Director, and the gate blocks `cafleet_*` tools in members |
| D1, D2 | `followUp` delivery with `session_shutdown` cleanup; the goal port with `appendEntry` persistence, a `context`-hook goal message on every request, `agent_before_settle` continuation tracked by extension flags, the 3-idle stop and the 50 cap |
| D3 | `cafleet_ask` is pi's `{decision_surface}`, gaining `director_id` and `database_url` for Director verification |
| D4, E1–E3 | Plain `pi` Director launch; `!` starts no turn (Note row); `fullscreen` pinned; the update notice is left as is |
| F1–F3 | The operator default model with no cost cap; tmux and herdr; a failed run records `no` and the other role ships |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Steps 1–5 write the documentation before code. They write the pi Role eligibility rows with Eligible `no` and Evidence `not established (acceptance run A-x pending)`. Step 11 rewrites those rows, and the prose that depends on them, from the observed results.

### Step 1: SPEC.md

- [x] §6.3: Config-dir resolution row and validation mention for `PI_CODING_AGENT_DIR` (§1.6), Asset installation row, executable record, pi output lines and the setup example (§2.1) <!-- completed: 2026-11-07T23:50 -->
- [x] §6.3 `cafleet doctor`: `pi.extension` in Check rows and § Extension checks, and the summary folding (§2.2) <!-- completed: 2026-11-07T23:50 -->
- [x] §6.3 `member create`: the pi `--model` note and the pi availability sentence <!-- completed: 2026-11-07T23:50 -->
- [x] §6.7: pending pi rows in § Role eligibility; `validate_prompt` in the Interface operations and the ordering invariant (`validate_model` → `validate_effort` → `validate_prompt` → `ensure_available` → `build_spawn_argv`); the pi `build_spawn_argv` block with the prompt rule (§1.2); Model selection and Reasoning effort rows (§1.3–1.4); Backend capabilities row; Contract error strings row (§1.7) <!-- completed: 2026-11-07T23:50 -->
- [x] §6.7: new `#### pi member gate` section (§3.2–3.6) and the pi paragraph in Host evidence and native-input authorization <!-- completed: 2026-11-07T23:50 -->
- [x] §6.7 Director extensions: the pi extension's tools, delivery, Director verification, receiver end, runtime-replaced notice, goal extension and `cafleet_ask` (§4) <!-- completed: 2026-11-07T23:50 -->
- [x] §6.9: launch policy table and run-record rows (§3.1), the version probe row (§1.5), and the pi profile from `1.5.0` section. Write its structure and the capture-independent declarations of §5 (the capability table, the CAFleet-authored form anchors and the answer semantics), and mark each capture-dependent value `pending (capture run A-F)`: row patterns, focus style, key bytes, `W_min`, layout limits and the fixture table. Step 10 writes the measured rows <!-- completed: 2026-11-07T23:50 -->
- [x] §8: the `member_runs.backend` CHECK (§1.1) <!-- completed: 2026-11-07T23:50 -->

### Step 2: Documentation site

- [x] `docs/docs/concepts/coding-agents.md`: the pi backend, its launch modes in Launch modes, the member gate posture, no sandbox and the extension-backed Director <!-- completed: 2026-11-07T23:53 -->
- [x] `docs/docs/quickstart.md`: a § pi section (install, `cafleet setup --coding-agent pi`, plain `pi` Director launch, `/cafleet-goal`, `PI_CODING_AGENT_DIR`) <!-- completed: 2026-11-07T23:53 -->
- [x] `docs/docs/concepts/members.md` and `docs/docs/mixed-backend-team.md`: pi mentions where these pages carry backend-specific content (launch-mode forms, models without a catalog) <!-- completed: 2026-11-07T23:53 -->
- [x] `docs/docs/contributing.md`: `extensions/pi/` and `native/pi.json` in the project-structure content <!-- completed: 2026-11-07T23:53 -->
- [x] Run `mise //docs:build` and inspect any changed piped table cell in the rendered HTML <!-- completed: 2026-11-07T23:53 -->

### Step 3: README.md

- [x] Check the README pitch and install commands against the docs index; edit only on drift <!-- completed: 2026-11-07T23:53 -->

### Step 4: Skills

- [x] `skills/cafleet/reference/coding-agents.md`: `## pi` with the §6 subsections and the native reception recipe (§4.6) <!-- completed: 2026-11-07T23:55 -->
- [x] `skills/cafleet/reference/cli.md`: config-location override list, `member create` pi argv and model rule, permission choices and mode classes, the effort row, the version-probe example, setup lines and the `pi.extension` doctor row <!-- completed: 2026-11-07T23:55 -->
- [x] `skills/cafleet/roles/director.md`: the pi config-dir row, and receiver-recovery wording covering the runtime-replaced notice <!-- completed: 2026-11-07T23:55 -->
- [x] `skills/cafleet/reference/model-selection.md`: pi resolution rows (no catalog, inference never selects pi, explicit pass-through without a thinking suffix) <!-- completed: 2026-11-07T23:55 -->

### Step 5: Project rules

- [x] Confirm that `.claude/rules/` describes the change through existing backend-neutral rules; edit only a rule whose convention changes <!-- completed: 2026-11-07T23:55 -->

### Step 6: Registry, argv and validation

- [x] `src/coding_agent/mod.rs`: `Pi` in `CodingAgent` with `name`, `config_variable` (`PI_CODING_AGENT_DIR`), `config_dir` (`~/.pi/agent`), `default_policy`, `policy_choices`, `permission_arguments`, `validate_model`, `validate_effort`, `validate_prompt`, the gated-mode extension read in `ensure_available` and the §1.2 argv <!-- completed: 2026-11-08T00:03 -->
- [x] `src/cli/member.rs`: call `validate_prompt` on the resolved prompt body before registration, between `resolve_body` and `director_placement` (§1.2); `src/coding_agent/mod.rs`: the pi arm of `validate_spawn_policy_argv` <!-- completed: 2026-11-08T00:03 -->
- [x] `migrations/V1__baseline.sql`: add `'pi'` to the backend CHECK <!-- completed: 2026-11-08T00:03 -->
- [x] Unit tests in `src/coding_agent/mod.rs`: argv for every mode with and without model, effort and prompt; the thinking-suffix rejection and a `:free` pass-through; every effort level and a rejected one; `validate_prompt` rejecting a leading `@` for pi only, and the leading-`-` prompt kept positional after `--`; the precondition order and the missing-extension error in both modes; config-dir default and override; the `1.5.0` version token <!-- completed: 2026-11-08T00:03 -->
- [x] Unit test in `src/cli/member.rs`: `member create --coding-agent pi` with a prompt beginning with `@` exits 2 with the §1.2 error and leaves no member row, run or pane <!-- completed: 2026-11-08T00:03 -->
- [x] Update every exhaustive backend match and registry-driven test that the new variant reaches (`src/cli/mod.rs`, `src/cli/member.rs`, `src/db.rs`, `src/runtime/`), keeping backend-specific branches pi-free where pi needs no behavior <!-- completed: 2026-11-08T00:03 -->

### Step 7: The CAFleet pi extension

- [x] `extensions/pi/cafleet.ts`: executable record, Director verification and member inertness (§4.1) <!-- completed: 2026-11-08T00:03 -->
- [x] `extensions/pi/cafleet.ts`: `cafleet_listen` with `followUp` delivery, closed notices, `session_shutdown` stop and the runtime-replaced notice (§4.2–4.3) <!-- completed: 2026-11-08T00:03 -->
- [x] `extensions/pi/cafleet.ts`: goal extension, with `pendingContinuation`/`progressed` tracking and the `context`-hook goal message (§4.4) <!-- completed: 2026-11-08T00:03 -->
- [x] `extensions/pi/cafleet.ts`: `cafleet_ask` (§4.5) <!-- completed: 2026-11-08T00:03 -->
- [x] `extensions/pi/cafleet.ts`: member gate with `--cafleet-gate` and the fail-closed mode promise, the tool policy with the protected directories, the allowlist lexer and matcher, the approval form and serialization (§3.2–3.5). The §3.4 gate case table is its acceptance check, run in A-M <!-- completed: 2026-11-08T00:03 -->
- [x] `build.rs`: confirm that the `EXTENSIONS` table embeds `extensions/pi/cafleet.ts` <!-- completed: 2026-11-08T00:03 -->

### Step 8: Setup and doctor

- [x] `src/assets.rs`: pi skills and extension targets, the executable record and output lines (§2.1) <!-- completed: 2026-11-08T00:06 -->
- [x] `src/cli/setup.rs`: the `pi.extension` doctor row and its summary folding (§2.2) <!-- completed: 2026-11-08T00:06 -->
- [x] Unit tests in `src/assets.rs`: installed bytes equal embedded bytes at the pi targets, the record content, output lines, rerun idempotence and a failure recording no installation <!-- completed: 2026-11-08T00:06 -->
- [x] Unit tests in `src/cli/setup.rs`: `pi.extension` `ok` and every `info`/`error` state, and the summary phrases <!-- completed: 2026-11-08T00:06 -->

### Step 9: Validation and A-S

- [x] `mise //:test` <!-- completed: 2026-11-08T00:06 -->
- [x] `mise //:lint` <!-- completed: 2026-11-08T00:06 -->
- [x] Run A-S with the built binary against a temporary `PI_CODING_AGENT_DIR` and database (§7) <!-- completed: 2026-11-08T00:09 -->

### Step 10: Captures and the pi profile

- [x] After every implementation member is deleted and no executing member depends on the installed assets, reinstall the CLI (`mise tasks --all` names the install task) and run `cafleet setup --coding-agent pi` in the real home <!-- completed: 2026-11-08T01:19 -->
- [x] Run A-F on tmux and herdr, and convert the captures into `src/coding_agent/native/pi.json` <!-- completed: 2026-11-08T01:48 -->
- [x] `src/coding_agent/bundled.json`: the pi profile from `1.5.0` with `pi-gate-approval`, `pi-idle-composer`, the allow and deny stages, the composer plans and the fixture entries (§5) <!-- completed: 2026-11-08T01:48 -->
- [x] `src/coding_agent/catalog.rs`: embed `native/pi.json` in the fixture registry <!-- completed: 2026-11-08T01:54 -->
- [x] Unit tests in the owning implementation files: the catalog guard accepts the pi fixtures; detection, focus resolution and both answer plans replay on the form fixtures; the idle composer and both submission plans replay <!-- completed: 2026-11-08T01:54 -->
- [x] Write the §6.9 pi profile tables in SPEC.md and the layout-limit Note in coding-agents.md from the captures <!-- completed: 2026-11-08T01:51 -->
- [x] `mise //:test` and `mise //:lint` <!-- completed: 2026-11-08T01:55 -->

### Step 11: Live acceptance and evidence

- [x] After the profile lands, delete its implementation members, reinstall the CLI and rerun `cafleet setup --coding-agent pi` <!-- completed: 2026-11-08T02:20 -->
- [x] Run A-P (pi Director) <!-- completed: 2026-11-08T02:20 -->
- [x] Run A-M (pi Members on tmux and herdr) <!-- completed: 2026-11-08T02:20 -->
- [x] Rewrite the SPEC § Role eligibility pi rows and the coding-agents.md pi Role eligibility table from the observed results, recording `no` with the observed failure for a failed run, and update the dependent Notes and docs verdict mentions <!-- completed: 2026-11-08T02:20 -->

### Step 12: Final consistency

- [x] Search SPEC, docs, skills, rules and help text for backend enumerations, and add pi to every one that is a registry copy, or rewrite it to point at the registry <!-- completed: 2026-11-08T02:22 -->
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` <!-- completed: 2026-11-08T02:22 -->
- [x] Commit `design-docs/0000220-pi-backend/design-doc.md` with the implementation <!-- completed: 2026-11-08T02:22 -->

---

## Changelog

| Date | Changes |
|---|---|
| 2026-11-07 | Initial draft from clarification round 1 and the pi 1.5.0 desk verification |
| 2026-11-07 | Review round 1. Changes by area:<br>• Prompt rule: the leading-`@` rule is checked by `validate_prompt` before registration (§1.2).<br>• Gate startup: the gate fails closed through a mode promise, and a gated member whose extension cannot register `--cafleet-gate` exits with pi's unknown-option error, which A-S records (§3.2).<br>• Protected directories: `edit` and `write` are blocked below `.git/`, `.pi/` and `.agents/`, per the Director decision (§3.3, §3.6, Notes).<br>• Gate acceptance: a gate case table, run in A-M (§3.4).<br>• Goal extension: continuation tracking uses extension flags, and the goal reaches every request through the `context` hook, because `before_agent_start` is skipped for automatic continuations; A-P adds a threshold compaction inside a continuation chain (§4.4).<br>• Note rows: the Applies-at cells name skill files and sections.<br>• Step 1: writes only the capture-independent profile declarations. |
| 2026-11-07 | User decision: the ungated launch mode is dropped. pi has two launch modes, `cafleet` (default) and `cafleet-ask`, both gated, and `ensure_available` always reads the extension (§1.2, §1.7, §3.1, §6, §7). |
| 2026-11-08 | Review round (implementation). Changes by area:<br>• Lexer: unquoted `$'` and `$"`, and `${` unquoted or inside double quotes, make a command not simple (§3.4).<br>• Allowlist: `git` and `tree` words must be literal, and the git `-O`/`--open-files-in-pager`/`--output` and `tree -o` options are refused, bundled and abbreviated forms included; the gate case table gains the escape rows (§3.4).<br>• File tools: `read`, `edit` and `write` without a usable `path` are blocked; every block reason is listed, and the `.env` reason for reads routes the read (§3.3).<br>• Limits: the `.env` rule covers only a file-tool path naming a `.env` file (§3.6, Notes).<br>• Goal context: the goal message is appended only while the goal is `active` (§4.4). |
| 2026-11-08 | Review round 2 (implementation). Changes by area:<br>• Lexer: a command holding a newline or carriage return, quoted or not, is not simple, because bash removes a backslash-newline pair inside double quotes; the gate case table gains three quoted-newline rows (§3.4, Notes).<br>• Protected directories: `.claude`, `.codex` and `.opencode` join `.git`, `.pi` and `.agents`, and the block reason names all six (§3.3, Notes).<br>• Limits: project-root files such as `opencode.json`, `AGENTS.md` and `CLAUDE.md` stay writable (§3.6). |
| 2026-11-08 | Review round 3 (implementation). Changes by area:<br>• Lexer: the `$` rule accepts an unescaped `$` outside single quotes only before a name character, so `$[`, `$((`, special and positional parameters and a trailing `$` join the refused forms; the gate case table gains four `$` rows (§3.4, Notes).<br>• Acceptance: Director decisions accept four unobserved A-P/A-M steps with their bases, and the A-P and A-M rows list what ran (§7, §8).<br>• A-P: the member check covers the `cafleet_*` tools, and the operator-typed `/cafleet-goal` works in any session (§4.1, §7).<br>• Limits: Escape-first delivery applies only to a `coverage=unknown` member (§3.6). |
| 2026-11-08 | Review round 4 (implementation). Changes by area:<br>• Allowlist: `tree *` and the tree option rule are removed, so only bare `tree` runs; the gate case table gains `tree` (Allowed) and `tree -R -L 1`, and the `tree -o` rows match no pattern (§3.4, Notes).<br>• File tools: protected-directory components and the `.env` basename compare case-insensitively, with `.Git/hooks/x` and `.ENV` cases (§3.3, Notes).<br>• Control characters: in both launch modes a bash command holding a C0 control character other than tab, DEL or a C1 control is blocked before any allowlist check or form, with its own reason; the newline rows move to this rule, the table gains an ESC row, and the form title shows the command on one line (§3.3–§3.6, Notes). |
