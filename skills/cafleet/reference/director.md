# Director-only commands (`cafleet member *`)

Reference page for the `cafleet member` subgroup — `member create`, `member delete`, `member list` (with `--activity`), `member capture`, `member send-input`, `member exec`, `member ping`. All require `--agent-id` (the Director's ID) and must be run inside a tmux session.

Members do NOT need to read this file. Member-side flows (poll / send / ack / receive shell-dispatch from the Director) live in `skills/cafleet/SKILL.md` (core) and `skills/cafleet/reference/exec-routing.md`.

## Member Create

Register a new member agent and spawn a coding-agent pane in the Director's own tmux window. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42" \
  -- "Review PR #42, post feedback via cafleet message send, and deregister on completion."

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Codex-A --description "Reviewer for PR #42" --coding-agent codex
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude` (default) or `codex`. The flag both selects the spawn-command builder AND is recorded as `placement.coding_agent`. Validated via `click.Choice(["claude", "codex"])`. Exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH`. |
| *(positional, after `--`)* | no | Prompt for the spawned coding-agent process. If omitted, the default prompt template is used. The default template and any custom prompt go through `str.format()` with `session_id` / `agent_id` / `director_agent_id` as kwargs, so callers may embed those placeholders in custom prompts and have the new member's literal UUIDs substituted in. |

The spawn argv depends on the chosen backend:

| Backend | Spawn command |
|---|---|
| `claude` (default) | `claude --permission-mode dontAsk --name <member-name> <prompt>` |
| `codex` | `codex --ask-for-approval never --sandbox workspace-write <prompt>` |

In both modes the member's Bash tool is enabled and routine permission prompts auto-resolve silently. Members run cafleet (and any shell command) directly via the Bash tool. See [`reference/exec-routing.md`](exec-routing.md) for the fallback path that fires when the harness deny-list (destructive operations such as `git push`) rejects a Bash invocation. Operational details for codex members live in [`docs/codex-members.md`](../../../docs/codex-members.md).

**Template safety**: because custom prompts go through `str.format()` whether or not they contain placeholders, any literal `{` or `}` in the prompt text must be doubled (`{{` / `}}`).

**Spawn prompt size limit (use path-by-reference for role docs)**: cafleet hands the prompt to `tmux split-window` as a single positional argument. tmux fails with `tmux command failed: command too long` and cafleet rolls back the agent registration once the shell-quoted prompt grows past a few KB — well below `ARG_MAX`. Empirically a prompt that inlines a full role definition (~10 KB) already exceeds this threshold.

Do NOT embed long role definitions, full skill specs, or any other multi-page block of text inline. Instead:

1. Resolve the absolute path of the role file in the Director's filesystem (e.g. `/home/.../skills/research-report/roles/manager.md`).
2. In the spawn prompt, write a short instruction like `ROLE DEFINITION: Open <abs path> with the Read tool BEFORE any other action.`
3. Keep the spawn prompt under ~2 KB total. Reserve it for: role-file path, skill-load list, session/agent/director IDs, the operational context (output dir, current date, user request), and "start now" cue.

This applies to every CAFleet-native team skill (`/cafleet:research-report`, `/cafleet:research-presentation`, `/cafleet:design-doc-create`, `/cafleet:design-doc-execute`, `/cafleet:design-doc-interview`, and any future skill). The member loads the role file via `Read` on its first turn; the role files live in the skill directory and are stable, so path-by-reference is safe.

**Scratch and audit files**: Spawn-related scratch (audit re-renders, working notes) MUST be written under `${BASE}` (resolved by `Skill(cafleet:base-dir)`) or under the skill's resolved output directory — never `/tmp`. The audit-file convention (`${BASE}/<role>.md`) is canonical; per-spawn temporary files have no place in `/tmp` because `${BASE}` is the project-local scratch root and keeps everything inspectable from one place.

**Backtick caveat (harness-dependent)**: Some operator environments (including this project) ship a Bash-validator hook that rejects any backtick in a `Bash` invocation — even inside single-quoted positional arguments — because the validator treats backticks as command-substitution syntax. When such a harness is in play, strip backticks from spawn-prompt bodies (use plain text where you would otherwise use markdown code spans). Path-by-reference for role docs sidesteps this entirely: the prompt body becomes short enough that backticks are easy to avoid.

**Pane title (claude backend only)**: `claude --name <member-name>` forwards the name to the spawned process so `#{pane_title}` shows the member name. The `codex` backend has no `--name` analog. Operators discover panes via `cafleet member list` (`pane_id` column is ground truth for both backends).

If the tmux `split-window` fails, the registered agent is rolled back. If the placement PATCH fails, the pane is `/exit`'d and the agent rolled back.

## Member Delete

The CLI sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters the agent and rebalances the layout. On timeout, the pane buffer tail is captured and printed on stderr, and the command exits 2 without deregistering. Rerun with `--force` to skip `/exit` and kill the pane immediately.

```bash
cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id>

cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id> --force
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID (cross-Director authorization check) |
| `--member-id` | yes | Target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, deregister, rebalance layout. Exit 0 even if the pane was already gone. |

Exit codes: `0` success / `1` non-timeout failure / `2` default-path timeout (buffer tail printed on stderr).

Cross-Director delete is rejected: the CLI verifies `placement.director_agent_id == --agent-id` before any tmux call. Exits 1 with `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).`

## Member List (with `--activity`)

```bash
cafleet --session-id <session-id> member list --agent-id <director-agent-id>
cafleet --session-id <session-id> member list --agent-id <director-agent-id> --activity
```

Default columns: `agent_id`, `name`, `status`, `backend`, `session`, `window_id`, `pane_id`, `created_at`. Pending placement renders `(pending)` in text mode, `null` in JSON.

`--activity` adds `last_sent` / `last_recv` / `last_ack` / `idle` columns aggregated from `tasks`:

```
$ cafleet --session-id <s> member list --agent-id <d> --activity
3 members:
  agent_id        name      state   last_sent    last_recv    last_ack     idle
  --------------  --------  ------  -----------  -----------  -----------  -----
  abc12345        alice     active  12:34:56     12:34:50     12:34:50     6s
  def67890        bob       active  12:30:11     12:33:02     12:33:02     2m
  ghi24680        carol     idle    -            12:20:00     12:20:00     14m
```

The `last_ack` aggregation filters `Task.type != 'broadcast_summary'` (mirrors `poll_tasks`). Use `--activity` for routine `/loop` ticks instead of capturing every member every minute — capture is reserved for the cases the activity columns flag.

## Member Capture

Capture the last N lines of a member's pane buffer. Default `--lines 30`; `--no-ansi` is the default and strips ANSI escapes.

```bash
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id>

cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id> --lines 200
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |
| `--lines` / `--tail` | no | Trailing lines to capture (default 30). `--tail` is an alias for `--lines`. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences stripped, carriage returns de-fragmented. `--ansi` emits the raw tmux capture. |

Cross-Director capture is rejected. Output is the raw captured buffer with no framing in text mode; JSON wraps it in `{member_agent_id, pane_id, lines, content}`.

## Member Send-Input

Forward a restricted keystroke to a member's tmux pane — write-path companion to `member capture`. AskUserQuestion-only.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |
| `--choice` | one-of | Integer `1`, `2`, or `3`. Sends the digit (no Enter). Validated via `click.IntRange(1, 3)`. |
| `--freetext` | one-of | Sends `4`, then literal text via `tmux send-keys -l`, then `Enter`. Newlines rejected. Values whose first non-whitespace character is `!` rejected (use `member exec` for shell dispatch). |

Exactly one of `--choice` / `--freetext` must appear. Three separate tmux invocations for `--freetext` (`4` → `-l "<text>"` → `Enter`) preserve shell meta, key names, and multi-byte characters as literal input.

### Answer a member's AskUserQuestion prompt

When `cafleet member capture` reveals a member paused on an AskUserQuestion-shaped 4-option frame (`1. …`, `2. …`, `3. …`, `4. Type something`), the Director MUST delegate the decision to the user via the three-beat shape:

1. **Capture** with `--lines 120` (recommended default; bump to `--lines 200` only if the AskUserQuestion frame is truncated).
2. **Ask the user via `AskUserQuestion`** with shape-appropriate options (table below). The question text names the member; no preamble sentence above the question.
3. **Invoke the resolved `cafleet member send-input`** via the Director's own Bash tool. Claude Code's per-call Bash permission prompt is the user-consent surface — never print a fenced `bash` block as an instruction.

#### Pane prompt shapes

The pane is ALWAYS on the AskUserQuestion 4-option frame when `send-input` is appropriate.

| Shape | Member pane looks like | Director's AskUserQuestion options | Resolved send-input call |
|---|---|---|---|
| **Choice-routing** | Option labels `1.`/`2.`/`3.` are the decision point. | Mirror UP TO 3 of the member's labels (don't add a 4th — `--choice` is `IntRange(1, 3)` and built-in "Other" handles freetext). | `--choice N` for picked mirror option; `--freetext "<typed>"` for built-in Other. |
| **Open-ended** | Option labels are NOT useful — the member is waiting for free-form instruction. | 2–4 *complete candidate message bodies*. `label` is a short intent tag (≈12 chars); `description` holds the full draft body. | `--freetext "<picked body>"` or `--freetext "<typed>"`. |
| **Other shapes** | Pane is NOT on an AskUserQuestion (mid-command, REPL, crashed, yes/no confirmation, mid tool-call). | Do NOT call `AskUserQuestion`; do NOT call `send-input`. Sending any keystroke would corrupt pane state. | None. Escalate via `cafleet message send`, or wait. |

#### AskUserQuestion constraints

- 1–4 questions per call; 2–4 options per question.
- Built-in "Other" is always exposed by the tool — do NOT add an explicit "Write my own" / "Custom" option.
- ≥ 5 candidate bodies → narrow to 2–4 (drop near-duplicates, span decision axes). Do NOT paginate.
- No preamble text above the question — the capture output already printed plus the question text carry all context.

#### MUST NOT

- Pre-draft a body and tell the user to run the command themselves.
- Print a fenced `bash` block of the resolved `send-input` invocation.
- Add a one-line preamble above `AskUserQuestion`.
- Add explicit "Write my own" / "Custom" option.
- Silently decide a `--choice` digit.
- Mix shapes (`--choice` on open-ended pane, `--freetext` on choice-routing pane).
- Call `send-input` on an "Other shapes" state.

## Member Exec

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into the member's pane via `tmux.send_bash_command` so the coding agent's `!` shortcut runs the command natively. Both `claude` and `codex` honor the leading-`!` shortcut. See [`reference/exec-routing.md`](exec-routing.md) for the full bash-via-Director fallback protocol.

```bash
cafleet --session-id <session-id> member exec --agent-id <director-agent-id> \
  --member-id <member-agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading/trailing whitespace stripped. Pipes, `&&`, `;`, `$(...)`, backticks not special-cased. |

Validation: missing `COMMAND` → exit 2 (Click built-in). Empty/whitespace-only → exit 2 (`Error: command may not be empty.`). Newlines → exit 2 (`Error: command may not contain newlines.`).

### Required follow-up: `cafleet member ping`

After every successful `cafleet member exec` (exit 0), the Director MUST immediately invoke `cafleet member ping` against the same member. `member exec` only stages the bang-command's stdout/stderr as context for the member's next turn — it does not advance the turn. The follow-up primitive is `cafleet member ping`, NOT `cafleet message poll` (poll polls the Director's own inbox; ping injects the keystroke into the member's pane).

Skip the ping only on non-zero `member exec` exit — the dispatch did not complete and the supervision tick (agent-team-monitoring `/loop`) is the safety net.

For a series of `member exec` calls on the same member, the ping follows each exec, not only the last.

## Member Ping

Director-only manual inbox-poll nudge. The broker's auto-fire on every `cafleet message send` is an inline preview keystroked into the recipient's pane (`tmux.send_inline_preview`). `member ping` is the manually-invokable counterpart for re-poking a member that missed an inline preview.

```bash
cafleet --session-id <session-id> member ping --agent-id <director-agent-id> \
  --member-id <member-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |

The action is wholly determined by the subcommand name — there is no positional argument and no operator-controlled keystroke body, so this subcommand sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

## Cross-references

- For broadcast send/ack semantics, see [`reference/broadcast.md`](broadcast.md).
- For the bash-via-Director fallback protocol, see [`reference/exec-routing.md`](exec-routing.md).
- For crash/disconnect/idle recovery flows including the Shutdown Protocol, see [`reference/recovery.md`](recovery.md).
- For `--full` / `--pretty` / `--json` opt-back-in semantics, see [`reference/legacy-flags.md`](legacy-flags.md).
