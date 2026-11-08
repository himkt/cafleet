# Minimal member broker allowlist with ask-by-default routing

**Status**: Approved
**Progress**: 0/28 tasks complete
**Last Updated**: 2026-11-08

## Overview

The OpenCode `cafleet` / `cafleet-ask` presets and the pi member gate shrink their shell allowlists to the four member broker commands `cafleet message send`, `poll`, `ack` and `show`, and route every other shell command to the Director (GitHub issue #579). The ask mode `cafleet-ask` (class `manual`) becomes the default launch mode on OpenCode and pi, so a member's work command opens a native form the Director answers instead of being denied, and the pi gate drops the lexer and option rules that only defended the removed entries.

## Success Criteria

- [ ] `presets/opencode/cafleet.md` and `presets/opencode/cafleet-ask.md` each have a `shell` map of exactly the `"*"` base plus the four allow entries of § 2, and their `description` and body paragraphs match § 4.
- [ ] `MEMBER_ALLOWLIST` in `extensions/pi/cafleet.ts` holds exactly the four § 2 patterns; the git/mise literal-word rule and option rules and their helpers are gone; the control-character rule, the simple-command lexer and the `$` rule remain; the `cafleet`-mode bash reason is the § 5.4 text.
- [ ] An omitted `--permission-mode` on OpenCode and pi resolves to `cafleet-ask` with `mode_source=default` and argv `["--agent","cafleet-ask"]` / `["--cafleet-gate","cafleet-ask"]`; explicit `cafleet` records `mode_source=explicit`; `--permission-mode` help reads `opencode: cafleet, cafleet-ask (default cafleet-ask)` and `pi: cafleet, cafleet-ask (default cafleet-ask)`.
- [ ] `SPEC.md` § Role requirements accepts a classified `auto` or `manual` default mode, and the OpenCode and pi Member eligibility rows rest on the `cafleet-ask` approval-form coverage of their bundled profiles.
- [ ] `presets/codex/cafleet.rules` and the Claude spawn rule `Bash(cafleet message *)` are byte-for-byte unchanged.
- [ ] The § 9 consistency grep finds no mention of a removed allowlist entry, the removed gate rules, the old block reason or an all-`auto` default in `SPEC.md`, `docs/`, `skills/`, `presets/` or `extensions/`.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Both OpenCode presets carry a 24-entry shell allowlist translated from the operator's Claude Code permission set: Git commands, the inspection utilities `grep`, `ls`, `stat`, `tree` and `wc`, this repository's `mise //:…` tasks and `cafleet *`. The pi gate (design 0000220) copied it as `MEMBER_ALLOWLIST`, with `cafleet *` narrowed to the four broker patterns. Review of #578 found repeated escapes through allowlisted options (`git grep -O<cmd>`, `git diff --output=<path>`, `tree -o`, cargo `--config … runner`, the mise `:::` chain); the pi gate closed them with dedicated lexer and option rules, and the OpenCode presets, which match by glob only, still admit them. The `mise` task names are this repository's tasks, and allowlisted `mise` tasks and `git commit` run member-editable workspace code and hooks, so the list never provided containment.

The Director can already decide every other command: it answers a native permission form with `cafleet permission inspect` / `permission respond` (deciding itself in verified auto mode, otherwise asking the user per request) and runs a routed command with `cafleet member exec`. Claude members already receive only `Bash(cafleet message *)`, Codex members run inside the kernel sandbox, and Hermes members use the operator's Hermes approval configuration, so those backends are unchanged.

---

## Specification

### 1. Scope

| Surface | Change |
|---|---|
| OpenCode `cafleet` and `cafleet-ask` presets | Shell map reduced to the four broker patterns; `description` and body rewritten (§ 4) |
| pi member gate (`extensions/pi/cafleet.ts`) | Allowlist reduced to the four broker patterns; git/mise rules removed; lexer reduced to a simplicity check; `cafleet` bash reason reworded (§ 5) |
| OpenCode and pi default launch mode | `cafleet` → `cafleet-ask` in the backend policy catalog (§ 3) |
| `SPEC.md` § Role requirements and launch-policy contract | The Member requirement accepts an `auto` or `manual` default; OpenCode/pi eligibility, run-record and posture rows follow the new default (§ 3, § 7) |
| Member routing guidance | One sentence on naming every routed command of a step in one request (§ 6) |
| Codex rules file `presets/codex/cafleet.rules` | Unchanged |
| Claude spawn rule `Bash(cafleet message *)` | Unchanged |
| OpenCode read/edit maps, `.env` exclusions, scalar denials and the `"cafleet": "deny"` plugin-tool rule | Unchanged |
| pi tool policy, file-tool path rules and the six protected directories | Unchanged behavior; rationale and Limits text reworded (§ 5.5) |

### 2. The member broker allowlist

Both backends allow exactly these four shell patterns, where `*` matches any character sequence, spaces included:

```text
cafleet message send *
cafleet message poll *
cafleet message ack *
cafleet message show *
```

Every word after the subcommand is free, so a member passes `--database-url <url>` after the subcommand (`cafleet message poll 7 --json --database-url sqlite:///x`). Every other command matches no pattern: other `cafleet` subcommands (`message listen`, `message wait`, every Director-owned command), Git, `mise` tasks, inspection utilities, a leading `NAME=value` assignment and `cafleet --database-url … message …` with the option before the subcommand. The pi gate's lexer establishes this for every command; on OpenCode it is established only for one command per invocation, because OpenCode 2.1.4's matching of a compound line such as `cafleet message poll 7; curl … | sh` against the broker globs is unverified (§ 7.2), under the deny-base `cafleet` preset as well. File reading and searching stay available through each backend's non-shell tools (OpenCode's read/search tools under its `read` map; pi's `read`, `grep`, `find` and `ls` tools), which keep their existing rules.

### 3. Default launch mode `cafleet-ask`

#### 3.1 Registry

`CodingAgent::default_policy` in `src/coding_agent/mod.rs` returns `"cafleet-ask"` for `Opencode` and `Pi`; `policy_choices` keeps its order (`cafleet`, `cafleet-ask`). Resolution, argv mapping and run-record fields follow unchanged code paths:

| Selection | `requested_mode` | `resolved_mode` | `mode_source` | `permission_argv_json` |
|---|---|---|---|---|
| Omitted OpenCode | `null` | `cafleet-ask` | `default` | `["--agent","cafleet-ask"]` |
| Explicit OpenCode `cafleet` | `cafleet` | `cafleet` | `explicit` | `["--agent","cafleet"]` |
| Omitted pi | `null` | `cafleet-ask` | `default` | `["--cafleet-gate","cafleet-ask"]` |
| Explicit pi `cafleet` | `cafleet` | `cafleet` | `explicit` | `["--cafleet-gate","cafleet"]` |

The `--permission-mode` help string becomes:

```text
Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart, manual (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)
```

The omitted-mode spawn precondition reads the selected policy's preset, so an omitted OpenCode spawn requires `<preset base>/agents/cafleet-ask.md`; its existing missing-preset error names the selector as the repair. Mode classes are unchanged: `cafleet` is `auto`, `cafleet-ask` is `manual`.

`doctor` names each OpenCode preset check from the default (`SPEC.md` §6.3 § OpenCode preset checks: the default choice's check is `opencode.preset`, every other choice's `opencode.preset.<mode>`, in catalog order), so its observable output changes while that SPEC text stays as written:

| Check, in output order | Preset it reports |
|---|---|
| `opencode.preset.cafleet` | `<preset base>/agents/cafleet.md` |
| `opencode.preset` | `<preset base>/agents/cafleet-ask.md` |

No `opencode.preset.cafleet-ask` check remains. `preset_check` in `src/cli/setup.rs` needs no code change.

#### 3.2 Role requirements

`SPEC.md` §6.7 § Role requirements, Member Eligible row, replaces "a classified default launch mode (`auto` whenever the backend offers one) with approval-prompt coverage" with "a classified default launch mode, `auto` or `manual`, with approval-prompt coverage". The rest of the row is unchanged: the bundled profile detects every approval form the default mode renders and declares request-bound allow and deny answers outside its recorded layout limits. `skills/cafleet/reference/coding-agents.md` § Role requirements makes the same change: "a classified default launch mode (`auto` — native automatic review or a fixed allow/deny policy — or `manual`, native interactive approval) whose bundled profile …".

#### 3.3 Member eligibility

The verdicts stay `yes`: each default form is already covered by a `tested` detection and allow/deny answer declaration (OpenCode rules `opencode-shell-permission` / `-compact` from `2.1.4`; pi rule `pi-gate-approval` with `pi-gate-allow-once` / `pi-gate-deny-once` from `1.5.0`). No bundled profile boundary changes. Live acceptance is not re-run (§ 8), so the evidence cells cite only what was checked.

| Surface | New text |
|---|---|
| `SPEC.md` § Role eligibility, `opencode` Member | PTY lifecycle and coverage declaration: live runner acceptance on herdr only (`coverage=tested`), 2.1.4, checked 2026-10-29. Default-mode approval-prompt coverage: the omitted `cafleet-ask` preset's shell permission form, whose detection and allow and deny answers the OpenCode profile from `2.1.4` declares `tested` (§6.9 Bundled profiles and native screen fixtures). Bundled support: the OpenCode profile from `2.1.4`. |
| `SPEC.md` § Role eligibility, `pi` Member | PTY lifecycle, coverage declaration and the omitted `cafleet-ask` gate mode's approval form: live runner acceptance on tmux and herdr (protected directories, routed command and `member exec`, busy delivery, and `cafleet-ask` allow and deny answers), 1.5.0, checked 2026-11-08. Bundled support: the pi profile from `1.5.0`. |
| Skill Role eligibility, `opencode` Member Basis | The omitted `cafleet-ask` preset, whose shell permission form the OpenCode profile from `2.1.4` detects and answers; runner acceptance on herdr only |
| Skill Role eligibility, `pi` Member Basis | The omitted `cafleet-ask` gate mode, whose approval form the pi profile from `1.5.0` detects and answers |

#### 3.4 Director load

Under the default, every non-broker command a member runs (tests, lint, `git status`, `git diff`) opens one form. The Director answers it through the existing runtime procedure: in verified auto mode it evaluates the risk and decides, otherwise it obtains the user's request-specific permission. Explicit `--permission-mode cafleet` keeps the deny-and-route posture for a member whose commands should instead be denied and routed. The user decided the default without a measurement step.

#### 3.5 Forms inside a recorded layout limit

Under the default, recorded layout limits become a routine path rather than a rare one. A form rendered inside one raises no `interaction_opened` event, so the member waits silently on an open form until someone answers it on the native surface:

| Backend | Recorded layout limits |
|---|---|
| pi | A pane narrower than `W_min` = 49 columns, a `$ ` command row that wraps (for example `mise //:test <filter> -- --nocapture` in an 80-column split), a form taller than the pane |
| OpenCode | A pane narrower than 36 columns, a command clipped by the form's command box |

The Director's handling:

1. **Detect.** The existing stall response applies: a member past its expected reply point gets a read-only `cafleet member capture`. A capture showing an approval form with no matching `interaction_opened` event is a form inside a recorded layout limit.
2. **Decide.** Decide it as step 5 of Consume observed interactions directs: in verified auto mode the Director evaluates the operation shown in the capture; otherwise it obtains the user's request-specific permission.
3. **Answer.** Press the backend's allow or deny keys with explicitly authorized `cafleet member press` (§ Explicit manual keys: manual keys carry no request-bound receipt, so inspect the result afterward), or ask the user to answer on the member's native surface.

`skills/cafleet/roles/director.md` § Stall Response gains one sentence carrying steps 1–3 (§ 7). User guidance: keep OpenCode and pi member panes wide enough for the profile's covered layouts and for each command to fit on one form row, or expect to answer some forms on the member's pane (§ 7 `members.md`, OpenCode configuration and pi configuration rows).

### 4. OpenCode presets

Both frontmatter blocks keep every key except `description` and the `shell` map. Shell maps:

```json
"shell": {
  "*": "deny",
  "cafleet message send *": "allow",
  "cafleet message poll *": "allow",
  "cafleet message ack *": "allow",
  "cafleet message show *": "allow"
},
```

`cafleet-ask.md` is identical with `"*": "ask"`. The four allow patterns overlap only the base, so their order carries no precedence.

| Field | `cafleet.md` | `cafleet-ask.md` |
|---|---|---|
| `description` | `CAFleet-spawned member whose shell runs only its broker message commands and denies every other command.` | `CAFleet-spawned member whose shell runs only its broker message commands and opens a native permission form for the Director to answer for every other command.` |
| Body paragraph 1 | § 4.1 `cafleet` | § 4.1 `cafleet-ask` |

#### 4.1 Body paragraph 1

`cafleet.md`:

```text
You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows only your broker commands `cafleet message send`, `poll`, `ack` and `show`, and denies every other command without a permission prompt. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials.
```

`cafleet-ask.md`:

```text
You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows only your broker commands `cafleet message send`, `poll`, `ack` and `show`, and asks for every other command. An asked command opens a native permission form; CAFleet notifies the Director, who answers it, and you continue from the result OpenCode returns. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials.
```

#### 4.2 Body paragraphs 2 and 3

Body paragraph 2, identical in both files:

```text
Your role and the Director's assignment determine which permitted tools you may use for this task. Director-only CAFleet commands refuse inside a member, so you send those needs to the Director. Follow the CAFleet member protocol and route a genuinely needed command that is denied to the Director as the member role's command-routing section describes, naming every command a step needs in one request. Read the Director's spawn instructions for your assigned scope.
```

Paragraph 3 is unchanged. Each file keeps the § Preset file format shape: JSON frontmatter with 2-space indent and key order `description`, `mode`, `permission`; three body paragraphs without hard line breaks; one trailing newline.

### 5. pi member gate

#### 5.1 Allowlist constant

```ts
const MEMBER_ALLOWLIST = [
  "cafleet message send *", "cafleet message poll *", "cafleet message ack *", "cafleet message show *",
]
```

`ALLOWLIST_RULES` keeps its glob-to-RegExp translation.

#### 5.2 Matching rules

A command is allowlisted when both hold:

1. **Simple.** Unchanged control-character pre-check (both modes, before any allowlist check or form). The trimmed command is one simple command: the lexer tracks single quotes, double quotes and backslash escapes, and rejects an unquoted `;`, `&`, `|`, `<`, `>`, `(`, `)` or backtick, a backtick inside double quotes, an unterminated quote, a trailing unquoted backslash, and every unescaped `$` outside single quotes that the next character does not start a name with (`A-Z`, `a-z`, `_`).
2. **Matching.** The whole trimmed command matches at least one § 5.1 pattern.

Removed with their SPEC text: rule 2 (literal git and mise words), rule 3 (git and mise option rules), and the paragraph on `git log -SFOO`, `grep`/`ls`/`stat`/`wc` expansion and bare `tree`. The `NAME=value` sentence stays: "A leading `NAME=value` assignment matches no pattern, so a member passes `--database-url <url>` after the broker subcommand instead."

#### 5.3 Code changes

| Element | Change |
|---|---|
| `isShortOptionWith`, `isRefusedGitWord`, `isRefusedMiseWord`, `LITERAL_WORD_COMMANDS`, `hasRefusedWord` | Deleted |
| `Word` interface and its `expands` tracking | Deleted |
| `simpleWords(command): Word[] \| undefined` | Becomes `isSimpleCommand(command): boolean`, keeping every rejection of § 5.2 rule 1 and dropping word assembly; its doc comment states that `command` holds no control character |
| `isAllowlisted` | `isSimpleCommand(trimmed) && ALLOWLIST_RULES.some((rule) => rule.test(trimmed))` |
| `startsName`, `controlBlock`, `CONTROL_CHARACTER`, the approval form, file-tool rules and protected directories | Unchanged |
| `cafleet`-mode bash reason in `gateBash` | The § 5.4 text |

#### 5.4 Block reason

| Case | Reason |
|---|---|
| Bash command not allowlisted, `cafleet` mode | `the CAFleet member gate runs only cafleet message send, poll, ack and show; route the command to the Director if it is required` |

Every other § Block reasons row is unchanged.

#### 5.5 Protected directories and Limits

The protected-directory rationale replaces "`.git/hooks` runs under the allowlisted `git commit *`" with "`.git/hooks` runs whenever a Git command such as `git commit` runs in the working directory, including one the Director allows or dispatches". All six directories stay protected.

`SPEC.md` § pi member gate **Limits** rows:

| Surface | New limit |
|---|---|
| Allowed tooling | The gate constrains a cooperative member's tool calls and is not containment. A command the Director allows on the form or dispatches with `member exec` runs the workspace code it executes, which a member can edit (test code, `build.rs`, the `mise.toml` task definitions), and that code can write anywhere the user can, protected directories included |
| `.env` files | The `.env` rule refuses only a `read`, `grep`, `find`, `ls`, `edit` or `write` call whose path names a `.env` file; a directory search (`grep` on `.`), an allowlisted broker `message send --file` naming a `.env` file, which copies its contents into a broker message, and a command the Director allows or dispatches still read `.env` contents |
| Protected directories | The rule covers only `edit` and `write`, and its path check follows no symbolic link, so a link elsewhere in the cwd that targets a protected directory stays writable; a command the Director allows or dispatches writes where its own tool writes, as `git` does under `.git/`, and workspace code it runs writes these directories like any other path |

The No sandbox, Project files, Delivery fallback and Multi-line commands rows are unchanged.

The same SPEC path-rule paragraph ends "The rule governs the `edit` and `write` tools only; code that an allowlisted command runs writes outside it (Limits below)." It becomes "The rule governs the `edit` and `write` tools only; code that a command the Director allows or dispatches runs writes outside it (Limits below)."

The `.env` reader through `message send --file` follows from the broker allowlist on both backends: the pi `.env` Limits row above, the pi `.env` skill note and the pi configuration `.env` sentence name it for pi, and the OpenCode preset Limits table, the OpenCode configuration page and the OpenCode `{permission_flags}` posture note name it for OpenCode, whose `read`/`edit` `.env` denials govern only its file tools (§ 7).

#### 5.6 Gate cases

The project's tests are Rust-only and the gate is TypeScript; the Director's code review checks the § 5.3 implementation against this table. `<own>` is the member's ID, `<dir>` the Director's, `<newline>` a line break inside the command and `<ESC>` the byte U+001B.

| # | Command | `cafleet-ask` (default) | `cafleet` | Rule |
|---|---|---|---|---|
| 1 | `cafleet message poll <own> --json` | Runs | Runs | Pattern |
| 2 | `  cafleet message poll <own> --json  ` | Runs | Runs | Trimmed, then pattern |
| 3 | `cafleet message poll <own> --json --database-url sqlite:///x` | Runs | Runs | `--database-url` after the subcommand |
| 4 | `cafleet message send --from-member-id <own> --to-member-id <dir> "a;b"` | Runs | Runs | `;` inside double quotes is literal |
| 5 | `cafleet message send --from-member-id <own> --to-member-id <dir> 'cost $(x)'` | Runs | Runs | Single-quoted text is literal |
| 6 | `cafleet message send --from-member-id <own> --to-member-id <dir> --file "$HOME/x"` | Runs | Runs | `$` followed by a name |
| 7 | `cafleet message ack 5; rm x` | Form | Blocked | Unquoted `;` |
| 8 | <code>cafleet message show 5 &#124; sh</code> | Form | Blocked | Unquoted pipe |
| 9 | `cafleet message send --from-member-id <own> --to-member-id <dir> "$(id)"` | Form | Blocked | `$` rule: `$(` inside double quotes |
| 10 | ``cafleet message send --from-member-id <own> --to-member-id <dir> `id` `` | Form | Blocked | Backtick |
| 11 | `cafleet message send --from-member-id <own> --to-member-id <dir> "${HOME}"` | Form | Blocked | `$` rule: `${` |
| 12 | `cafleet message send --from-member-id <own> --to-member-id <dir> $'x'` | Form | Blocked | `$` rule: unquoted `$'` |
| 13 | `cafleet message send --from-member-id <own> --to-member-id <dir> "a` | Form | Blocked | Unterminated quote |
| 14 | `cafleet message send --from-member-id <own> --to-member-id <dir> "a<newline>b"` | Blocked, no form | Blocked | Control character: newline |
| 15 | `cafleet message poll <own> <ESC>[2K` | Blocked, no form | Blocked | Control character: ESC |
| 16 | `FOO=1 cafleet message poll <own>` | Form | Blocked | Leading assignment matches no pattern |
| 17 | `cafleet --database-url sqlite:///x message poll <own>` | Form | Blocked | Option before the subcommand |
| 18 | `cafleet message listen <own>` | Form | Blocked | No member broker pattern |
| 19 | `cafleet member exec 2 --from-member-id 1 "id"` | Form | Blocked | Director-owned command |
| 20 | `git status` | Form | Blocked | No pattern |
| 21 | `mise //:test` | Form | Blocked | No pattern |
| 22 | `ls` | Form | Blocked | No pattern |

"Form" opens the § Approval form; "Blocked" under `cafleet` returns the § 5.4 reason; a control-character case returns the control-character reason in both modes.

### 6. Member routing guidance

`skills/cafleet/roles/member.md` § Reconsider, then route adds one sentence after the routing command block: "When a step needs several such commands, name them all in one request." The OpenCode preset paragraph 2 carries the same guidance (§ 4).

### 7. Documentation changes

Each row is the smallest edit that removes the drift; § 3–§ 6 hold the exact texts.

| Target | Section | Change |
|---|---|---|
| `SPEC.md` | §6.3 `member create` option table (`--permission-mode` row) | "omission selects the backend's default mode" |
| `SPEC.md` | §6.3 help text line for `--permission-mode` | § 3.1 help string |
| `SPEC.md` | §6.7 § Role requirements | § 3.2 |
| `SPEC.md` | §6.7 § Role eligibility | § 3.3 rows |
| `SPEC.md` | §6.7 backend posture table, `opencode` and `pi` rows | `cafleet-ask` (default): the member broker commands run and every other shell (pi: bash) command opens a native permission form (pi: the gate's form for a command without a control character); `cafleet`: the same broker commands, every other command denied |
| `SPEC.md` | §6.7 paragraph after the posture table | The omitted OpenCode and pi policy `cafleet-ask` is a `manual` mode whose form the Director answers; explicit `cafleet` is an `auto` mode with a fixed allow/deny policy that renders no approval form |
| `SPEC.md` | §6.7 opencode preset | Default marker moves to `cafleet-ask` in the launch-policy table; the ruleset paragraph states the `"*"` base plus the four § 2 entries, that every other `cafleet` command falls to the base, and that tool permission and assignment authority remain separate; Limits table per § 7.2 |
| `SPEC.md` | §6.7 pi member gate | § 5.1 constant, § 5.2 rules, § 5.4 reason, § 5.5 rationale and Limits |
| `SPEC.md` | §6.9 Launch policy and capability registry | Table headers become `` `auto` modes `` / `` `manual` modes `` with `(default)` after each default; OpenCode and pi mark `cafleet-ask` and their mapping cell says explicit `cafleet` records `mode_source=explicit`; "Each backend has one default (omitted) mode, marked in the table above."; the OpenCode/pi choice sentences say "outside the member broker commands"; the run-record table uses the § 3.1 pi rows |
| `docs/docs/concepts/coding-agents.md` | Launch modes | Lead sentence ends "selects the backend's default mode:"; table columns `Default mode` (with its class), `Other auto modes`, `Other manual modes`; OpenCode and pi rows `` `cafleet-ask` (`manual`) `` / `cafleet` / —; the pi sentence says both modes run only the member's broker message commands, `cafleet-ask` opens the gate's form for every other command and `cafleet` blocks it |
| `docs/docs/concepts/coding-agents.md` | OpenCode configuration | Both presets allow only the member's broker message commands; the default `cafleet-ask` opens a native permission form for every other shell command and `cafleet` denies it without a prompt; keep member panes wide enough that each form and its command show in full, or expect to answer some forms on the member's pane; a broker `message send --file` naming a `.env` file copies its contents into a message despite the `.env` file-tool denials |
| `docs/docs/concepts/coding-agents.md` | pi configuration | Operation table columns `` `cafleet-ask` (default) `` then `cafleet`; rows: a member broker message command runs; a control-character command is blocked without a form; any other shell command (compound, `NAME=value`, Git, `mise`, other `cafleet` commands) opens the form / is blocked; the containment paragraph cites commands the Director allows or dispatches instead of allowed `mise` tasks and `git commit`; the `.env` sentence follows § 5.5, naming the broker `message send --file` reader; add: keep member panes wide enough that each command fits on one form row, or expect to answer some forms on the member's pane |
| `docs/docs/concepts/members.md` | Choose a native policy | "selects the backend's default mode"; "every shell command outside the member broker commands under OpenCode or pi `cafleet-ask`"; add: "On OpenCode and pi the default `cafleet-ask` opens a form for every such command, so a Director outside verified auto mode asks you about each one, and a form too wide for the member's pane is answered on that pane." |
| `docs/docs/quickstart.md` | OpenCode and pi | Members run under `cafleet-ask`: broker message commands run and every other shell command opens a native permission form for the Director; choose `--permission-mode cafleet` for a member whose other commands should be denied and routed to the Director instead |
| `README.md` | — | No change (no allowlist or default-mode text) |
| `skills/cafleet/reference/coding-agents.md` | Role requirements; opencode and pi Runtime bindings, Notes and Role eligibility | § 3.2; `{permission_flags}` per § 7.1; the OpenCode `cafleet *` note becomes the four broker patterns; the OpenCode `cafleet` / `cafleet-ask` notes swap to default `cafleet-ask` (class `manual`) and explicit `cafleet` (class `auto`), and the default-mode note adds that a broker `message send --file` naming a `.env` file bypasses the `.env` file-tool denials; the pi gate note follows § 5.2 and § 5.4 without the git/mise/`tree` rules; the pi `.env` note follows § 5.5, naming the broker `message send --file` reader in place of `grep '' .env` / `git diff`; the pi form note says "Default `cafleet-ask`"; Role eligibility per § 3.3 |
| `skills/cafleet/reference/cli.md` | `member create` permission choices; setup | § 3.1 choices; the sentence "Every default is `auto`." is deleted; setup says OpenCode receives the `cafleet` and `cafleet-ask` presets (shell rules for the four member broker commands, `"cafleet": "deny"` tool permission) |
| `skills/cafleet/roles/director.md` | Member launch policy | "which selects the backend's default mode" |
| `skills/cafleet/roles/director.md` | Stall Response | Add: "A capture showing an approval form with no matching `interaction_opened` event is a form inside a recorded layout limit: decide it as step 5 of [Consume observed interactions](#consume-observed-interactions) directs, then press the backend's allow or deny keys with explicitly authorized [`member press`](#member-press), or ask the user to answer it on the member's native surface." (§ 3.5) |
| `skills/cafleet/roles/member.md` | Reconsider, then route | § 6 |
| `.claude/rules/` | — | No change |

#### 7.1 `{permission_flags}` bindings

| Backend | Value |
|---|---|
| opencode | `` `--agent cafleet-ask` (default mode `cafleet-ask`, class `manual`: the member broker commands run and every other shell command opens a native permission form); `--agent cafleet` (class `auto`: every other shell command is denied) when `--permission-mode cafleet` is selected `` |
| pi | `` `--cafleet-gate cafleet-ask` (default mode `cafleet-ask`, class `manual`: the CAFleet member gate runs the member broker commands, opens its approval form for every other bash command and protects `.git`/`.pi`/`.agents`/`.claude`/`.codex`/`.opencode`); `--cafleet-gate cafleet` (class `auto`: every other bash command is blocked without a form) when `--permission-mode cafleet` is selected `` |

#### 7.2 `SPEC.md` opencode preset Limits

| Surface | Example | Limit |
|---|---|---|
| MCP-contributed tools | — | Unchanged |
| Compound shell commands | <code>cafleet message poll 7; curl … &#124; sh</code> against `cafleet message poll *` | Permission matching for compound lines is unverified on opencode 2.1.4, under both presets; matching is established for one command per invocation. |
| Interpreter or hook execution via a command the Director allows | `mise //:test` executes workspace-writable test code; `git commit` runs `.git/hooks` | An allowed command also runs the code and hooks it executes, under that one-time permission. |
| `.env` files | `cafleet message send --from-member-id 7 --to-member-id 1 --file .env` | The `read` and `edit` `.env` denials govern OpenCode's file tools; an allowed broker `message send --file` naming a `.env` file copies its contents into a broker message. |

### 8. Tests and validation

Rust tests stay in-file (`.claude/rules/code-quality.md` § Test placement) and assert no preset text. Existing embedded-asset tests, which compare installed bytes with embedded bytes, cover the new preset and extension bytes unchanged.

| Test | File | Change |
|---|---|---|
| `literal_argv_preserves_permission_and_optional_flag_boundaries` | `src/coding_agent/mod.rs` | OpenCode omitted argv `--agent cafleet-ask`; pi omitted argv `--cafleet-gate cafleet-ask` |
| `--permission-mode` help assertion | `src/coding_agent/mod.rs` | § 3.1 help string |
| `pi_argv_for_every_mode_carries_the_gate_trust_layout_and_name_then_the_prompt_after_a_separator` | `src/coding_agent/mod.rs` | `(None, "cafleet-ask")` |
| `pi_launch_policy_records_the_gate_pair_for_the_omitted_and_explicit_modes` | `src/coding_agent/mod.rs` | Omitted resolves `cafleet-ask` / `default`; explicit `cafleet` resolves `cafleet` / `explicit` with `["--cafleet-gate","cafleet"]` |
| New `opencode_launch_policy_records_the_agent_for_the_omitted_and_explicit_modes` | `src/coding_agent/mod.rs` | Mirrors the pi test for `--agent` |
| `launch_policy_omits_the_backend_default_and_resolves_every_configured_case` | `src/runtime/observation.rs` | OpenCode capture cases now use the default, so `requested_mode()` is `None` |
| `runtime_policy_omission_selects_each_backends_automatic_default_argv` | `src/coding_agent/mod.rs` | Renamed `runtime_policy_omission_selects_each_backends_default_argv`; expected OpenCode and pi modes `cafleet-ask` |
| `independent_failures_keep_executable_discovery_and_mark_dependent_checks_unavailable` | `src/cli/setup.rs` | Unavailable check names `opencode.preset.cafleet` and `opencode.preset` in place of `opencode.preset` and `opencode.preset.cafleet-ask` |
| `opencode_preset_checks_follow_catalog_order_and_report_each_preset_state` | `src/cli/setup.rs` | Expected order `opencode.preset.cafleet`, `opencode.preset`, `opencode.extension`, `opencode.installation`; each check still pairs with `cafleet.md` then `cafleet-ask.md` in catalog order |

Any other test the default change breaks is updated to the § 3.1 table. No live acceptance runs (user decision); the OpenCode compound-command matching limit stays "unverified" in `SPEC.md`. Validation: `mise //:test`, `mise //:lint`, `mise //docs:build`, then the § 9 grep.

### 9. Consistency grep

After implementation, this search returns no line in `SPEC.md`, `docs/`, `skills/`, `presets/` or `extensions/`:

```text
grep -rnE 'git (add|commit|diff|grep|log|ls-tree|ls-files|branch) \*|"cafleet \*": "allow"|mise //:(test|lint|format|typecheck|build)": "allow"|isRefused(Git|Mise)Word|LITERAL_WORD_COMMANDS|allowlisted simple commands|default `auto` mode|`auto` on every backend|Every default is `auto`|allowlisted `git commit|allowlisted `mise|allowlisted command runs|git log --stat; curl|opencode\.preset\.cafleet-ask' SPEC.md docs/docs skills presets extensions
```

### 10. Clarification decisions

| Question | Decision |
|---|---|
| Allowlist content | Four broker patterns only; read/edit rules, `.env` exclusions, scalar denials, the plugin-tool deny and pi's protected directories unchanged |
| Default mode | `cafleet-ask` (class `manual`) on OpenCode and pi; SPEC § Role requirements, eligibility and the all-`auto` default contract change accordingly |
| Read-only inspection set | None; Git inspection is routed too |
| Director-load measurement | None; decided now |
| Codex rules file; Claude `Bash(cafleet message *)` | Out of scope, unchanged |
| pi gate rules | Remove the git/mise literal-word and option rules; keep the control-character, simple-command and `$` rules |
| Protected directories | Keep all six; reword the rationale |
| `.env` and Allowed-tooling Limits | Rewrite to the new state |
| pi block reason | Reworded to name the broker commands |
| Routing guidance | One sentence on naming every routed command of a step in one request |
| Acceptance | Rust tests and documentation only; no live acceptance; OpenCode compound matching stays "unverified" |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [ ] §6.3: `--permission-mode` option row and help string (§ 7, § 3.1) <!-- completed: -->
- [ ] §6.7 Role requirements and Role eligibility (§ 3.2, § 3.3) <!-- completed: -->
- [ ] §6.7 posture table rows and the following paragraph; opencode preset section, its Limits table included (§ 2, § 7, § 7.2) <!-- completed: -->
- [ ] §6.7 pi member gate: allowlist constant, matching rules, block reason, path-rule paragraph (protected-directory rationale and closing sentence) and Limits (§ 5) <!-- completed: -->
- [ ] §6.9 Launch policy and capability registry: table, default sentence, choice sentences and run-record rows (§ 3.1, § 7) <!-- completed: -->

### Step 2: Documentation site

- [ ] `docs/docs/concepts/coding-agents.md`: Launch modes, OpenCode configuration and pi configuration (§ 7) <!-- completed: -->
- [ ] `docs/docs/concepts/members.md`: Choose a native policy (§ 7) <!-- completed: -->
- [ ] `docs/docs/quickstart.md`: OpenCode and pi member posture paragraphs (§ 7) <!-- completed: -->

### Step 3: README.md

- [ ] Confirm `README.md` carries no allowlist or default-mode text and needs no edit <!-- completed: -->

### Step 4: Skills

- [ ] `skills/cafleet/reference/coding-agents.md`: Role requirements, opencode and pi Runtime bindings, Notes and Role eligibility (§ 3.2, § 3.3, § 7, § 7.1) <!-- completed: -->
- [ ] `skills/cafleet/reference/cli.md`: permission choices and setup sentence (§ 7) <!-- completed: -->
- [ ] `skills/cafleet/roles/director.md`: Member launch policy and Stall Response (§ 3.5, § 7) <!-- completed: -->
- [ ] `skills/cafleet/roles/member.md`: routing guidance sentence (§ 6) <!-- completed: -->

### Step 5: Registry default

- [ ] `src/coding_agent/mod.rs`: `default_policy` returns `cafleet-ask` for `Opencode` and `Pi` (§ 3.1) <!-- completed: -->
- [ ] Update the `src/coding_agent/mod.rs` tests listed in § 8 and add the OpenCode launch-policy test <!-- completed: -->
- [ ] Update `launch_policy_omits_the_backend_default_and_resolves_every_configured_case` in `src/runtime/observation.rs` and any other test the default change breaks (§ 8) <!-- completed: -->
- [ ] Update the `src/cli/setup.rs` doctor preset-check tests to the renamed checks (§ 3.1, § 8) <!-- completed: -->

### Step 6: OpenCode presets

- [ ] `presets/opencode/cafleet.md`: shell map, `description`, body paragraphs 1 and 2 (§ 4) <!-- completed: -->
- [ ] `presets/opencode/cafleet-ask.md`: shell map, `description`, body paragraphs 1 and 2 (§ 4) <!-- completed: -->
- [ ] Check both files against § Preset file format: key order, indentation, three paragraphs, one trailing newline <!-- completed: -->

### Step 7: pi member gate

- [ ] `extensions/pi/cafleet.ts`: `MEMBER_ALLOWLIST` (§ 5.1) <!-- completed: -->
- [ ] Delete the git/mise helpers and `Word`; replace `simpleWords` with `isSimpleCommand`; update `isAllowlisted` (§ 5.3) <!-- completed: -->
- [ ] Reword the `cafleet`-mode bash reason (§ 5.4) <!-- completed: -->
- [ ] Review the gate against the § 5.6 cases <!-- completed: -->

### Step 8: Validation

- [ ] `mise //:test` passes <!-- completed: -->
- [ ] `mise //:lint` passes <!-- completed: -->
- [ ] `mise //docs:build` passes, and the rendered pi configuration and Launch modes tables keep their cells intact <!-- completed: -->

### Step 9: Final consistency

- [ ] Run the § 9 grep and resolve every hit; confirm `presets/codex/cafleet.rules` and the Claude `Bash(cafleet message *)` spawn rule are unchanged <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-08 | Initial draft |
| 2026-11-08 | Review round 1: forms inside recorded layout limits and their Director handling (§ 3.5); doctor preset-check renames and the affected tests (§ 3.1, § 8); the broker `message send --file` `.env` reader on pi and OpenCode (§ 5.5, § 7.2); the pi path-rule closing sentence (§ 5.5); the OpenCode compound-matching example and its scope in § 2 (§ 7.2) |
