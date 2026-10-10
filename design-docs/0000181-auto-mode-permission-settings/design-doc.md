# Recommended Permission Settings for Auto Mode and Other Modes

**Status**: Approved
**Progress**: 0/26 tasks complete
**Last Updated**: 2026-10-10

## Overview

CAFleet's documentation recommends a single Claude Code permission block built around `Bash(cafleet *)`, written before auto mode existed. This design replaces it with two recommended profiles — one for a Director session running in auto mode, one for every other permission mode — and documents which role needs each entry and what a `dontAsk` member can run. The change is documentation only: no CLI, preset, or `cafleet setup` behavior changes.

## Success Criteria

- [ ] Quickstart § Configure › Claude Code presents two `settings.json` profiles, auto mode first, and each snippet parses as valid JSON.
- [ ] The backend spec § Claude carries a per-entry table naming the role that needs each entry and why, and states that a `dontAsk` member is denied every shell command and file edit outside the read-only set and the allow list.
- [ ] Both profiles keep `Bash(cafleet member prompt *)` under `permissions.ask`, use the un-namespaced `Skill(cafleet)` / `Skill(cafleet-design-doc)` entries, and carry the `Edit` / `Write` entries members need to write files.
- [ ] `grep -ri` over `docs/docs/`, `skills/`, and `.claude/rules/` finds none of the strings listed in § Specification › Stale strings.
- [ ] cli-options § `permissions.allow` coverage keeps its heading and anchor, states the pattern rule, and links to the quickstart profiles instead of restating them.
- [ ] The Codex rules reference documents the minimum rule set and states that it is sufficient for members and not the recommendation for a Director session.
- [ ] `mise //docs:build` succeeds, and the rendered quickstart and backend spec show both snippets and all new tables intact.
- [ ] `git diff --stat` lists only the files named in § Specification › Files changed; `SPEC.md`, `README.md`, and everything under `cafleet/` are untouched.

---

## Background

### Current documentation

| Surface | Current content |
|---|---|
| Quickstart § Configure › Claude Code | One snippet: allow `Bash(cafleet *)`, `Skill(cafleet:cafleet)`, `Skill(cafleet:cafleet-design-doc)`; ask `Bash(cafleet * member prompt *)` |
| cli-options § `permissions.allow` coverage | "One pattern per subcommand" rule with three example patterns; `member prompt` excluded so it stays under `permissions.ask` |
| coding-agent-backends § Spawn argv | Claude members "run cafleet and any shell command directly"; config prerequisite "none" |
| `skills/cafleet/reference/prompt-routing.md` | For Claude members "denial is the rare case — the harness deny-list rejects a few destructive operations"; "most denials are a wrong flag, wrong path, or an unnecessary command" |
| `skills/cafleet/roles/director.md`, `skills/cafleet/reference/supervision.md` | Members "run shell commands themselves … no Director routing required"; "permission prompts auto-resolve" |
| `.claude/rules/bash-tool.md` | Member "permission prompts auto-resolve silently"; "No … Director routing" |

No page mentions auto mode or `permissions.defaultMode`.

### What changed

The maintainer simplified their own Claude Code settings for auto mode: `defaultMode: "auto"` with the CAFleet allow entries reduced to `Bash(cafleet message *)`, `Bash(cafleet monitor *)`, `Bash(cafleet member ping *)`, `Skill(cafleet)`, and `Skill(cafleet-design-doc)`. Their Codex rules were aligned to the same three prefixes. The same settings file also carries general-purpose entries that CAFleet members depend on: bare `Edit` / `Write` / `NotebookEdit`, `Read(/skills/**)`, and `additionalDirectories: ["~/.claude"]`. The documentation should describe the full posture a fleet needs, alongside the existing one.

### Claude Code facts the design relies on

Verified against the Claude Code permission-mode and permission-rule documentation on 2026-10-10.

| Fact | Consequence for CAFleet |
|---|---|
| Rules resolve first in every mode, in the order deny → ask → allow. An ask rule prompts even when an allow rule also matches. | `Bash(cafleet member prompt *)` under ask holds beside `Bash(cafleet *)` under allow. |
| In auto mode, a call matching no rule goes to a classifier; read-only actions and working-directory edits are auto-approved. A content-matching ask rule still prompts. | The Director's remaining `cafleet` commands and its own file edits need no allow entry in auto mode. |
| `dontAsk` runs only what needs no approval in Manual mode — read-only Bash commands and file reads inside the working directories — plus calls matching `permissions.allow`. Every call that would prompt is denied, including one matching an ask rule. | A member's shell commands and file edits are denied by default. A member can never run `member prompt`. |
| File modification needs approval in Manual mode. `Edit` rules apply to every built-in file-editing tool; a path rule for those tools is written as `Edit(path)`. | A member writes files only when an `Edit` allow entry matches. |
| Writes to protected paths — directories such as `.git` and `.claude`, files such as `.mcp.json` — are denied in `dontAsk`, routed to the classifier in auto mode, and prompted in Manual mode. Allow rules in settings files do not pre-approve them. | No profile entry lets a member edit a project's `.claude/` or `.git/`; the Director session makes those edits. |
| `--permission-mode` on the command line outranks `permissions.defaultMode`. | Members stay in `dontAsk` whatever the operator's default mode is. |
| `defaultMode: "auto"` takes no effect from a project `.claude/settings.json` or `.claude/settings.local.json`. When set there, Claude Code uses the built-in default instead of the value in `~/.claude/settings.json`. | The profiles belong in `~/.claude/settings.json`, and a project-level `auto` value must be moved, not shadowed. |
| A `Skill(name)` allow rule matches the skill's own name; `plugin:skill` names only a plugin-provided skill. | `cafleet setup` installs personal skills, so `Skill(cafleet:cafleet)` never matches them. |
| File reads outside the working directories need approval; a `~/path` rule is home-relative. | A member reading role and reference pages under `~/.claude/skills/` needs a Read allow entry. |

---

## Specification

The normative wording of every edit is § Target text. The subsections before it record the decisions and rationale that the target text does not carry.

### Decisions

| # | Decision | Rationale |
|---|---|---|
| D1 | Two profiles: **auto mode** and **other permission modes**. The split key is the mode the Director session runs in. | Members always spawn in `dontAsk`; only the Director's mode varies. Every non-auto mode needs the same pre-approval. |
| D2 | Quickstart § Configure › Claude Code owns the lead paragraph, the profile comparison table, and the two snippets. The backend spec § Claude owns the per-entry table, the member-permission semantics, and the symptom table. cli-options § `permissions.allow` coverage keeps only the pattern rule. | One owner per enumeration, and the same split Codex and OpenCode use: a snippet in the quickstart, posture detail in the backend spec. |
| D3 | The auto mode profile is presented first. | It is the maintainer's primary setup and the built-in starting mode of current Claude Code. |
| D4 | Both profiles keep `Bash(cafleet member prompt *)` under `permissions.ask`. | The command keystrokes operator-controlled text into a pane. This intentionally differs from the maintainer's deployed settings, which carry no such entry. |
| D5 | The ask pattern is `Bash(cafleet member prompt *)`. | Ids are positional after the subcommand; nothing sits between `cafleet` and `member`, so the documented `Bash(cafleet * member prompt *)` matches no invocation. |
| D6 | Skill entries are `Skill(cafleet)` and `Skill(cafleet-design-doc)`. | They match the personal skills `cafleet setup` installs. |
| D7 | Both profiles carry `Read(~/.claude/skills/**)`. | The maintainer's settings already hold this entry as `Read(/skills/**)`; the profiles spell it home-relative so it reads the same in any settings file. See § Read entry. |
| D8 | Both profiles carry bare `Edit` and `Write`. | Without them every member file edit is denied. See § File-edit entries. |
| D9 | Codex is described through a minimum rule set owned by the Codex rules reference, stated as sufficient for members only. The installed `cafleet.rules` and the OpenCode preset are unchanged. | Docs only. See § Codex minimum rule set. |
| D10 | Each backend's denial semantics move into a `{permission_flags}` row of its `Note → applies at` table. The neutral skill pages (`prompt-routing.md`, `director.md`, `supervision.md`) are reworded to hold on every backend and point at that note. | The project's overlay rule keeps backend specifics in the backend reference. The current neutral sentences ("denial is rare", "no Director routing required") are wrong for a Claude member under a minimal allow list. |
| D11 | The documentation states that each routed command costs one operator confirmation, and that project task commands members run belong in the allow list. | Routing dispatches through `cafleet member prompt --shell`, which D4 keeps under `ask`. Allow-listing task commands is therefore a practical requirement, not polish. |

### Read entry

`cafleet setup` installs skills under `~/.claude/skills/`, outside any project working directory. A member loads the skill core through the Skill tool, then opens its role and reference pages with a file read. Under `dontAsk` a read outside the working directories is denied unless a rule allows it, and in auto mode the Director's first such read prompts. When `CLAUDE_CONFIG_DIR` relocates the config directory, the operator writes the entry against that directory's `skills/**`.

### File-edit entries

A Drafter writing a design doc and a Programmer editing source both need file edits, and the routing protocol carries shell commands only, so a denied edit has no fallback. The Claude Code documentation states that `Edit` rules apply to every built-in file-editing tool. The profiles still list `Write` beside `Edit`, because that pair is what the maintainer's deployed settings carry and what this project's fleets have exercised; no live check is planned, so the profiles recommend the exercised form.

The entries are bare because a member's working directory varies per fleet. They are read by every session that loads the settings file, so they also pre-approve file edits in the operator's own non-auto sessions. The target text states this and offers the documented narrowing, a path rule such as `Edit(~/work/**)`, which covers every file-editing tool.

No allow entry reaches Claude Code's protected paths. A member's edit under a project's `.claude/` (rules, project-local skills) or `.git/` is denied under both profiles whatever the allow list holds, so the Director session makes it — reviewed by the classifier in auto mode, prompted otherwise. The target text names this as a second cause of a denied edit wherever it describes one, and this design's own `.claude/rules/` edit is assigned to the Director for the same reason.

### Codex minimum rule set

The installed `~/.codex/rules/cafleet.rules` allows the whole `cafleet` prefix and prompts for `cafleet member prompt`; it remains the recommended Codex configuration. The minimum rule set serves a rules directory maintained without the installed file, such as the maintainer's dotfiles-managed one. It is the three `allow` prefixes members run plus a `prompt` rule that applies D4 to Codex.

The set mirrors the auto mode profile for members only. Claude's auto mode leaves the Director's remaining commands to a classifier; Codex has none, so under the minimum set a Codex Director's other `cafleet` commands match no rule and follow that session's own approval policy and sandbox, instead of running pre-approved outside it. The target text therefore recommends the installed file for a Director session.

The `match` / `not_match` examples use real CAFleet invocations. The maintainer's personal rules use `cafleet monitor start 1` and `cafleet member ping 5 --timeout 10`, which are valid prefix matches and not real command forms.

### Files changed

| # | File | Change |
|---|---|---|
| 1 | `docs/docs/quickstart.md` | § Configure table cell, § Claude Code rewritten, § Codex closing paragraph |
| 2 | `docs/docs/spec/cli-options.md` | § `permissions.allow` coverage rewritten; one sentence in § JSON output; one clause in § `member ping` |
| 3 | `docs/docs/spec/coding-agent-backends.md` | Capability-table `claude` cells, § Claude rewritten with two new subsections, new § Minimum rule set |
| 4 | `docs/docs/concepts/coding-agents.md` | One cell of the asymmetries table |
| 5 | `skills/cafleet/reference/coding-agents.md` | One `{permission_flags}` row added to each backend's `Note → applies at` table; `## Template` requires that row |
| 6 | `skills/cafleet/reference/prompt-routing.md` | Opening paragraph and the reconsider sentence made backend-neutral |
| 7 | `skills/cafleet/roles/director.md` | Introduction sentence and one § Member Create sentence |
| 8 | `skills/cafleet/reference/supervision.md` | § Routing member bash requests, first two sentences |
| 9 | `.claude/rules/bash-tool.md` | The MUST rule |
| 10 | `design-docs/0000181-auto-mode-permission-settings/design-doc.md` | This document |

The runtime reference pages under `skills/cafleet/reference/runtime/` are symlinks to the docs pages, so files 1–4 update them without a separate edit. `README.md` only links to the quickstart and `SPEC.md` holds no recommended-settings content; neither changes.

Statements reviewed and left unchanged:

| Statement | Why it stays |
|---|---|
| `skills/cafleet/roles/member.md` — "workspace-scoped auto-approval ({permission_flags}). Run task commands yourself" and § Command execution | Already neutral: it tells the member to attempt the command and follow the denial path. |
| `skills/cafleet/SKILL.md` and `director.md` — "`permissions.allow` matches Bash invocations as fixed strings" | Still true under both profiles. |
| `prompt-routing.md` § The two primitives, **Permission gate** column | `member prompt` under ask and `member ping` under allow hold in both profiles. |
| backend spec — "All three postures enable the Bash tool with no runtime permission prompts" | True: a call is approved or denied, never prompted. |
| The `freshness` date in `skills/cafleet/reference/coding-agents.md` | It covers model data; a note-table edit preserves it. |

### Stale strings

After the edits, a case-insensitive search of `docs/docs/`, `skills/`, and `.claude/rules/` finds none of:

| String | Removed from |
|---|---|
| `Skill(cafleet:` | quickstart |
| `cafleet * member prompt` | quickstart |
| `prompts auto-resolve` | `director.md`, `supervision.md`, `bash-tool.md` |
| `no Director routing` | `director.md`, `bash-tool.md` |
| `deny-list` | `prompt-routing.md`, concepts page |
| `per-subcommand allow patterns` | cli-options |
| `most denials are` | `prompt-routing.md` |
| `any shell command directly` (on a `claude` table row) | backend spec |

The last string remains on the `codex` row of the capability table, which this design does not change; the check for it is scoped to the `claude` row.

### Target text

#### 1. `docs/docs/quickstart.md`

In the § Configure table, the `claude` row's **Manual configuration** cell becomes `The permission profile below that matches the Director's mode`.

§ Claude Code is replaced in full by:

````markdown
### Claude Code

Add the profile below that matches the permission mode your Director session
runs in to your user-level `~/.claude/settings.json`. Members start in
`dontAsk` mode whichever mode the Director uses, so both profiles pre-approve
what members run.

| Behavior | Auto mode profile | Other-modes profile |
|---|---|---|
| Director session's permission mode | `auto` | Any other mode |
| The Director's `cafleet` commands | Reviewed by the auto mode classifier, except the allow-listed member patterns | Pre-approved by `Bash(cafleet *)` |
| A member's `cafleet` commands | Pre-approved by the narrow member patterns | Pre-approved by `Bash(cafleet *)` |
| `cafleet member prompt` | Prompts the operator | Prompts the operator |

#### Auto mode {#auto-mode-profile}

```json
{
  "permissions": {
    "defaultMode": "auto",
    "allow": [
      "Bash(cafleet message *)",
      "Bash(cafleet monitor *)",
      "Bash(cafleet member ping *)",
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)"
    ]
  }
}
```

In auto mode a classifier reviews each Director command that no rule matches,
so the allow list carries only what members need. `defaultMode` selects auto
mode from user-level or managed settings; a project or local settings file
cannot select it.

#### Other permission modes {#other-modes-profile}

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)"
    ]
  }
}
```

`Bash(cafleet *)` is the single allow-everything entry that the literal
integer-id convention enables — one pattern covers every subcommand for every
fleet, for the Director and for members.

`Edit` and `Write` let members write files. They apply to every session that
reads the settings file, your own included: each then edits files outside
Claude Code's protected paths without a prompt or classifier review.
[What each profile entry is for](spec/coding-agent-backends.md#claude-profile-entries)
names the role that needs each entry and gives the path rule that confines
this approval.
[What members can run](spec/coding-agent-backends.md#claude-member-permissions)
covers denied commands and the entries to add for your project's own task
commands.
````

In § Codex, the closing paragraph becomes:

```markdown
The Codex rules for `cafleet` commands allow every subcommand while keeping
`cafleet member prompt` prompting; the reference above covers their precedence
and where operator customizations belong. A rules directory maintained without
the installed file needs the
[minimum rule set](spec/coding-agent-backends.md#minimum-rule-set).
```

#### 2. `docs/docs/spec/cli-options.md`

§ `permissions.allow` coverage keeps its heading and is replaced in full by:

````markdown
## `permissions.allow` coverage

Every `cafleet` invocation is coverable by a prefix pattern, because each id
rides after the subcommand name as a literal positional or trailing argument:

- **A pattern is a command prefix followed by ` *`.** The prefix is the whole
  CLI (`Bash(cafleet *)`), a command group (`Bash(cafleet message *)`), or one
  subcommand (`Bash(cafleet member ping *)`). The positional subject id and
  trailing flags such as [`--json`](#json-output) are covered by the same
  pattern. Both `monitor` forms ride the single `Bash(cafleet monitor *)`
  pattern — `cafleet monitor scan` needs no pattern of its own.
- **`member prompt` stays under `permissions.ask`** as
  `Bash(cafleet member prompt *)` — its positional text body is
  operator-controlled, in both the plain and the `--shell` form. An ask rule
  outranks every allow rule, so the entry holds beside `Bash(cafleet *)`.

Which prefixes to allow depends on the permission mode the Director session
runs in. The recommended profiles are in
[Claude Code configuration](../quickstart.md#claude-code), and the role that
needs each entry is in
[What each profile entry is for](coding-agent-backends.md#claude-profile-entries).
Apply a profile to your user-level `~/.claude/settings.json` manually; the
repo does not ship a committed permissions block.
````

In § JSON output, the sentence `The trailing position keeps JSON invocations inside the existing per-subcommand allow patterns (see [`permissions.allow` coverage](#permissionsallow-coverage)).` becomes `The trailing position keeps a JSON invocation inside the same prefix pattern that covers the command (see [`permissions.allow` coverage](#permissionsallow-coverage)).`

In § `member ping`, the clause `which is why `member ping` sits in `permissions.allow` while `member prompt` stays in `permissions.ask`` becomes `which is why both recommended permission profiles allow `member ping` while keeping `member prompt` under `permissions.ask``.

#### 3. `docs/docs/spec/coding-agent-backends.md`

In the per-backend capabilities table, the `claude` row changes two cells:

| Column | New cell |
|---|---|
| Shell-command posture | Runs read-only commands and every command matching the operator's `permissions.allow`; denies the rest without a prompt |
| Preset / config prerequisite | A permission profile in `~/.claude/settings.json` — a permission posture, not a spawn dependency ([Claude](#claude)) |

§ Claude is replaced in full by:

````markdown
## Claude {#claude}

`--permission-mode dontAsk` is the reference auto-approval posture the other
backends match. A member runs Claude Code's built-in read-only commands, file
reads inside its working directory, and every tool call matching
`permissions.allow` without a prompt. Every other call — a shell command or a
file edit, including one matching `permissions.ask` — is denied without a
prompt. A write to one of Claude Code's protected paths, such as a project's
`.git` or `.claude` directory, is denied whatever the allow list holds. The
operator's settings are therefore the member's allow list; the recommended
profiles are in [Claude Code configuration](../quickstart.md#claude-code).

### What each profile entry is for {#claude-profile-entries}

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet message *)` | Auto mode | Every member | Each member sends, polls, and acknowledges its own messages. |
| `Bash(cafleet monitor *)` | Auto mode | The monitor member | It hosts the wake loop and runs the per-wake fleet scan. |
| `Bash(cafleet member ping *)` | Auto mode | The monitor member | Its fixed re-poke of a quiet pane carries no operator-controlled text. |
| `Bash(cafleet *)` | Other modes | The Director and every member | One pattern pre-approves every subcommand, in place of the narrow member patterns. |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool. |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory. |
| `Edit`, `Write` | Both | Every member that writes files | A member's file edit is denied unless an entry matches it. No entry covers a protected path. |
| `Bash(cafleet member prompt *)` under `ask` | Both | The Director | The operator confirms each dispatch; a member's own attempt is denied. |

In the auto mode profile the Director needs no further entry: the classifier
reviews each of its commands that no rule matches, and auto mode approves its
file edits inside the working directory.

`cafleet member prompt` stays under `ask` in both profiles because it
keystrokes arbitrary text or shell commands into a member's pane. An ask rule
outranks every allow rule, so the entry holds beside `Bash(cafleet *)`.

`Edit` and `Write` apply to every session that reads the settings file, so
they also pre-approve file edits in your own sessions. To confine the
approval, replace both with a path rule such as `Edit(~/work/**)`, which
covers every file-editing tool. No entry reaches a protected path: a member's
edit under a project's `.git` or `.claude` directory is always denied, and the
Director session makes that edit instead. When `CLAUDE_CONFIG_DIR` relocates
the config directory, write the `Read` entry against that directory's
`skills/**`.

### What members can run {#claude-member-permissions}

A member whose shell command is denied asks its Director to dispatch it. The
Director dispatches with `cafleet member prompt --shell`, which is under
`ask`, so each routed command costs the operator one confirmation. Add an
allow entry for every project command members run as part of their work — a
test or lint task, for example, as `Bash(npm test *)` — so those commands run
without routing. Routing carries shell commands only: a denied file edit
needs a matching `Edit` entry, or, on a protected path, the Director to make
the edit.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a denied shell command | The command is outside the read-only set and the allow list | Add an allow entry for the command. Until then, each Director dispatch of it asks the operator to confirm. |
| A member reports a denied file edit | No `Edit` entry matches the file's path, or the path is protected | Add `Edit` and `Write`, or widen the `Edit` path rule to cover the member's working directory. For a protected path, the Director makes the edit. |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add a pattern for that subcommand per [`permissions.allow` coverage](cli-options.md#permissionsallow-coverage) |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`, removing it from the project file. When auto mode is unavailable, use the other-modes profile. |
````

A new subsection is appended to the end of § The `cafleet` rules file:

````markdown
#### Minimum rule set {#minimum-rule-set}

A rules directory maintained without the installed file — for example one
managed from a dotfiles repository — needs at least these rules:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)

prefix_rule(
    pattern = ["cafleet", "monitor"],
    decision = "allow",
    match = ["cafleet monitor 1", "cafleet monitor scan 1 --lines 120 --json"],
    not_match = ["cafleet member list 1", "cafleet message poll 5"],
)

prefix_rule(
    pattern = ["cafleet", "member", "ping"],
    decision = "allow",
    match = ["cafleet member ping 5", "cafleet member ping 5 --json"],
    not_match = ["cafleet member prompt 5 hello", "cafleet member list 1"],
)

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text or shell commands into a member pane",
)
```

The `allow` rules cover what members run themselves, each mirroring the
`Bash` entry of the same prefix in the
[auto mode profile](../quickstart.md#auto-mode-profile); the `prompt` rule
keeps `cafleet member prompt` under operator approval.

The set is sufficient for members and is not the recommendation for a
Director session. Codex has no classifier to review what these rules leave
unmatched, so every other `cafleet` command a Director runs follows that
session's own approval policy and sandbox instead of running pre-approved
outside it. Keep the installed file for a Director session; it is a superset
of these rules, so they add nothing when it is also present.
````

#### 4. `docs/docs/concepts/coding-agents.md`

In the § Known asymmetries table, the **Sandbox isolation** cell for `claude` becomes `not supported — the operator's allow list is the safety floor`.

#### 5. `skills/cafleet/reference/coding-agents.md`

One row is appended to each backend's `Note → applies at` table. All three rows share this **Applies at** cell:

```markdown
`{permission_flags}` — `cafleet/roles/member.md` § Command execution; `cafleet/reference/prompt-routing.md` opening paragraph / § Member-side: reconsider, then route; `cafleet/roles/director.md` introduction / § Member Create; `cafleet/reference/supervision.md` § Routing member bash requests
```

The **Note** cells:

| Section | Note cell |
|---|---|
| `## claude` | `dontAsk` runs the harness's built-in read-only commands, file reads inside the working directory, and every tool call matching the operator's `permissions.allow`; every other call — shell command or file edit — is denied without a prompt. A task command outside that allow list is routed on every use, and each routed dispatch costs the operator one `member prompt` confirmation. Routing carries shell commands only: report a denied file edit to the Director — as a missing `Edit` allow entry, or, for a path under the harness's protected directories (such as `.git` and `.claude`), as an edit the Director must make. |
| `## codex` | Denial is the rare case: the harness rejects a few destructive operations (e.g. `git push`, `rm -rf`). |
| `## opencode` | The `--agent cafleet` preset is a deny-by-default bash allowlist: denial is the common case for any un-allowlisted command, and routing workflow commands (`mise`, `mkdir`, …) through the Director is the routine path. Before routing, check whether an allowlisted command covers the need. |

The codex and opencode notes relocate the wording that `prompt-routing.md` carries today; their meaning is unchanged.

The neutral pages now send every reader to this note, so `## Template` requires it of a new backend. In `## Template` § Note → applies at, the paragraph is replaced in full by:

```markdown
Use a `Note | Applies at` table, one caveat per row. Every Applies-at cell names the token and affected `<skill>/<file>` section. Bind the pane-cue table to monitor on-wake classification and the Director's capture gate. Bind a `{permission_flags}` row stating the posture's denial semantics — what a member runs unprompted, what the harness denies, and when the member routes — to the member command-execution, prompt-routing, Director, and supervision instructions that point at it.
```

#### 6. `skills/cafleet/reference/prompt-routing.md`

The opening paragraph is replaced in full by:

```markdown
The bash-via-Director protocol is the **fallback** for a harness-denied command. Members run shell commands directly via the Bash tool by default (workspace-scoped auto-approval — see [`roles/member.md`](../roles/member.md)). What your harness denies, and so how often the fallback fires, is backend-specific: read the `{permission_flags}` note in your overlay section of [`coding-agents.md`](coding-agents.md). Either way the member auto-routes a plain CAFleet message to its Director, which dispatches the command into the member's pane via `cafleet member prompt --shell` (keystrokes literal `! <cmd>` + `Enter`, honored by `claude` / `codex` / `opencode`).
```

In § Member-side: reconsider, then route, the first paragraph is replaced in full by:

```markdown
Reconsider first, using your overlay's `{permission_flags}` note: check whether the flag or path is wrong, whether the command is unnecessary, and whether a command your posture approves covers the need. Fix or drop what you can yourself. Only a genuinely-correct, genuinely-needed, still-denied command gets routed:
```

#### 7. `skills/cafleet/roles/director.md`

In the introduction, the sentence `Members spawn with workspace-scoped auto-approval, so by default they run shell commands themselves via the Bash tool — no Director routing required.` becomes:

```markdown
Members spawn with workspace-scoped auto-approval: each runs the commands its backend's posture approves itself and routes a denied command through you. What each backend denies is the `{permission_flags}` note in that backend's section of [`coding-agents.md`](../reference/coding-agents.md).
```

In § Member Create, the sentence `In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve; the denied-command fallback is [`reference/prompt-routing.md`](../reference/prompt-routing.md).` becomes:

```markdown
In all three modes the member's Bash tool is enabled and never waits on a permission prompt — an approved command runs and any other is denied; the denied-command fallback is [`reference/prompt-routing.md`](../reference/prompt-routing.md).
```

#### 8. `skills/cafleet/reference/supervision.md`

In § Routing member bash requests, the first two sentences become:

```markdown
The workflow's spawned members run in workspace-scoped auto-approval mode ({permission_flags}; Bash tool enabled, no permission prompt ever waits), so they run the commands their backend's posture approves directly. When a member's harness denies a command (what each backend denies is its `{permission_flags}` note in [`coding-agents.md`](coding-agents.md)), it auto-routes a plain shell-command request via `cafleet message send`, and you respond via `cafleet member prompt --shell`.
```

The paragraph's last sentence (`Process such requests one at a time in poll order.`) is unchanged.

#### 9. `.claude/rules/bash-tool.md`

The blockquote under § The MUST rule is replaced by:

```markdown
> **If you are a CAFleet member spawned by `cafleet member create`, your harness runs in `--permission-mode dontAsk`. Your Bash tool is ENABLED: read-only commands and every call matching the operator's `permissions.allow` run without a prompt, and every other shell command or file edit is denied without a prompt. An edit under a protected directory such as `.git` or `.claude` is denied whatever the allow list holds. Run cafleet commands and your task commands directly via the Bash tool, with no prefix and no operator prompt. When a correct, necessary command is denied, follow the reconsider-then-route protocol; report a denied file edit to the Director.**
```

§ The owning protocols already links the reconsider-then-route protocol and is unchanged.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation only. Follow the project order: `docs/` pages first, then skills, then project rules. Use the text in § Specification › Target text verbatim.

### Step 1: Quickstart

- [ ] In `docs/docs/quickstart.md` § Configure, update the `claude` row's Manual configuration cell <!-- completed: -->
- [ ] Replace § Claude Code with the lead paragraph, the profile comparison table, the two profile snippets (auto mode first), and the closing links to the backend spec <!-- completed: -->
- [ ] Replace the closing paragraph of § Codex with the version linking the minimum rule set <!-- completed: -->

### Step 2: CLI options spec

- [ ] Replace the body of § `permissions.allow` coverage in `docs/docs/spec/cli-options.md`, keeping the heading text so the `#permissionsallow-coverage` anchor is unchanged <!-- completed: -->
- [ ] Reword the allow-pattern sentence in § JSON output <!-- completed: -->
- [ ] Reword the `permissions.allow` / `permissions.ask` clause in § `member ping` <!-- completed: -->

### Step 3: Backend spec and concepts

- [ ] Update the two `claude` cells of the per-backend capabilities table in `docs/docs/spec/coding-agent-backends.md` <!-- completed: -->
- [ ] Replace the § Claude paragraph with the `dontAsk` allow-list paragraph <!-- completed: -->
- [ ] Add § What each profile entry is for with its table and the three paragraphs beneath it <!-- completed: -->
- [ ] Add § What members can run with its paragraph and the symptom table <!-- completed: -->
- [ ] Append § Minimum rule set to § The `cafleet` rules file, with the four `prefix_rule` blocks and the two closing paragraphs <!-- completed: -->
- [ ] Update the `claude` Sandbox isolation cell in `docs/docs/concepts/coding-agents.md` <!-- completed: -->

### Step 4: Skill pages

- [ ] Append the `{permission_flags}` row to the `Note → applies at` table of each backend section in `skills/cafleet/reference/coding-agents.md`, leaving the freshness date unchanged <!-- completed: -->
- [ ] Replace the `## Template` § Note → applies at paragraph in the same file so a new backend must supply the `{permission_flags}` row <!-- completed: -->
- [ ] Replace the opening paragraph of `skills/cafleet/reference/prompt-routing.md` <!-- completed: -->
- [ ] Replace the first paragraph of § Member-side: reconsider, then route in the same file <!-- completed: -->
- [ ] Replace the introduction sentence and the § Member Create sentence in `skills/cafleet/roles/director.md` <!-- completed: -->
- [ ] Replace the first two sentences of § Routing member bash requests in `skills/cafleet/reference/supervision.md` <!-- completed: -->

### Step 5: Project rule

The Director session performs this step. `.claude/` is a Claude Code protected directory, so a `dontAsk` member's write there is denied whatever the allow list holds, and routing carries shell commands only.

- [ ] Director: replace the MUST-rule blockquote in `.claude/rules/bash-tool.md` <!-- completed: -->

### Step 6: README and SPEC check

- [ ] Confirm `README.md` needs no edit (it only links to the quickstart) and that `SPEC.md` is unchanged <!-- completed: -->

### Step 7: Verification

- [ ] Parse each of the two quickstart JSON snippets with a JSON parser and confirm both are valid <!-- completed: -->
- [ ] Run a case-insensitive `grep -r` over `docs/docs/`, `skills/`, and `.claude/rules/` for each string in § Specification › Stale strings; confirm no match, apart from the `codex` capability-table row noted there <!-- completed: -->
- [ ] Run `mise //docs:build` and confirm it succeeds <!-- completed: -->
- [ ] Inspect the rendered quickstart, cli-options, and coding-agent-backends pages: both snippets and every new table render intact, and the `#auto-mode-profile`, `#claude-profile-entries`, `#claude-member-permissions`, `#minimum-rule-set`, and `#permissionsallow-coverage` links resolve <!-- completed: -->
- [ ] Run `mise //cafleet:test` and `mise //cafleet:lint`, since the skill files are embedded in the binary <!-- completed: -->
- [ ] Run `git diff --stat` and confirm only the files in § Specification › Files changed appear <!-- completed: -->
