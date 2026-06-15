# tmux-backed member commands (`cafleet member *`)

Reference page for the `cafleet member` subgroup — `member create`, `member delete`, `member list` (with `--activity`), `member capture`, `member send-input`, `member exec`, `member ping`, `member nudge`. All must be run inside a tmux session. Every subcommand here is Director-initiated **except `member nudge`**, which the monitoring member invokes to re-engage the Director (its `--agent-id` sender is the monitoring member, not the root Director). `member create` takes `--agent-id` (the spawning Director's ID, validated to equal the fleet root); `member nudge` takes **both** `--agent-id` (the acting sender) **and** `--member-id` (the target); the remaining subcommands identify their target by `--member-id` alone. All are scoped to the per-subcommand `--fleet-id`.

Members do NOT need to read this file. Member-side flows (poll / send / ack / receive shell-dispatch from the Director) live in `skills/cafleet/SKILL.md` (core) and `skills/cafleet/reference/exec-routing.md`.

> **`--member-id` is an integer.** On `member delete` / `capture` / `send-input` / `exec` / `ping`, pass the full integer member id printed by `cafleet member list` — ids are typed `int` and short by construction, so there is no prefix resolution. `member create`'s `--agent-id` is likewise an integer.

## Member Create

Register a new member agent and spawn a coding-agent pane in the Director's own tmux window. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"

cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Codex-A --description "Reviewer for PR #42" --coding-agent codex

cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name monitor --description "Monitoring member: owns the heartbeat" \
  --role monitor --model sonnet \
  --prompt-file /abs/path/to/<BASE>/prompts/monitor-20260514T145000Z.md
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`. The flag both selects the spawn-command builder AND is recorded as `placement.coding_agent`. Validated via `click.Choice(list(CODING_AGENTS.keys()))` — the choice set is registry-driven (currently `["claude", "codex", "opencode"]`) so adding a future backend is one entry in `cafleet.coding_agent.CODING_AGENTS`. Exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH`. For the `opencode` backend, the spawn-precondition step also materializes `~/.opencode/agents/cafleet.md` from the in-source `CAFLEET_AGENT` preset on first spawn (skip-if-exists) — see [`docs/reference/coding-agents/opencode.md`](../../../docs/reference/coding-agents/opencode.md). |
| `--model` | no | Model passed to the backend binary via its `--model` flag, appended immediately before the prompt tokens (for `opencode`, before the `--prompt <prompt>` pair). Omitted → no model tokens in the spawn argv; the binary uses its own default model. `claude` and `codex` accept any string (pass-through — the binary itself rejects unknown models, so newly released models work without a cafleet release). `opencode` requires the `<provider-id>/<model-id>` format (split on the **first** `/` into two non-empty segments; model ids may themselves contain slashes) and rejects violations at create time with exit 2 — `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` — before any agent registration or tmux side effect. Spawn-time only: not recorded in `agent_placements`, not shown in `member list`. Example models (not enforced by cafleet): `claude` → `sonnet`; `codex` → `gpt-5.5`; `opencode` → `anthropic/claude-sonnet-4-6`. |
| `--role` | no | One of `member` (default) or `monitor`. `monitor` spawns the fleet's dedicated **monitoring member**: it sets `agent_card_json.cafleet.kind == "monitoring-member"` and enrolls the agent in `monitor_config` so the monitor loop wakes it; ordinary `member` does neither. `--role` controls only the kind marker and enrollment — the LLM is still selected by `--model` (the Director passes `--model sonnet`). Only one monitoring member is allowed per fleet; a second `--role monitor` spawn is rejected by `register_agent` with `Error: fleet <id> already has an active monitoring member (agent <existing-id>); only one is allowed.` (exit 1). The monitoring member is spawned **first** and is the one process that runs `cafleet monitor start`; see the `cafleet-agent-team-monitoring` skill § The monitoring member for its canonical spawn prompt and the first-in/first-out lifecycle. |
| `--prompt-file` | no | Absolute path to a UTF-8 file whose contents are the spawn prompt. Mutually exclusive with the positional prompt argument. Read verbatim (no stripping); passes through the same `str.format()` substitution as the inline form. Relative paths, missing files, unreadable files, invalid UTF-8, and empty (zero-byte or whitespace-only) files all error non-zero with the messages catalogued in [`docs/spec/cli-options.md`](../../../docs/spec/cli-options.md) § Error Messages. The canonical input mode for every CAFleet-native team-skill spawn — see § *Member Create — Scratch and audit files* below. |
| *(positional, after `--`)* | no | Prompt for the spawned coding-agent process. Mutually exclusive with `--prompt-file`. If both are omitted the default prompt template is used. The default template and any custom prompt go through `str.format()` with `fleet_id` / `agent_id` / `director_agent_id` as kwargs, so callers may embed those placeholders in custom prompts and have the new member's literal ids substituted in. |

The spawn argv depends on the chosen backend:

| Backend | Spawn command (`--model` omitted) | Spawn command (`--model <m>`) |
|---|---|---|
| `claude` (default) | `claude --permission-mode dontAsk --name <member-name> <prompt>` | `claude --permission-mode dontAsk --name <member-name> --model <m> <prompt>` |
| `codex` | `codex --ask-for-approval never --sandbox workspace-write <prompt>` | `codex --ask-for-approval never --sandbox workspace-write --model <m> <prompt>` |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` | `opencode --agent cafleet --model <m> --prompt <prompt>` |

In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve silently. Members run cafleet (and any shell command) directly via the Bash tool. See [`reference/exec-routing.md`](exec-routing.md) for the fallback path that fires when the harness deny-list (destructive operations such as `git push`) rejects a Bash invocation. Operational details for codex members live in [`docs/reference/coding-agents/codex.md`](../../../docs/reference/coding-agents/codex.md); the opencode equivalent (`CAFLEET_AGENT` preset, refresh recipe, MCP caveats) lives in [`docs/reference/coding-agents/opencode.md`](../../../docs/reference/coding-agents/opencode.md).

### Model-name-to-backend inference

When the operator names a model rather than a backend ("please create a member with sonnet"), resolve the backend from the model-name shape:

| User says (model name shape) | Inferred backend | Flags to pass |
|---|---|---|
| `fable`, `opus`, `sonnet`, or a `claude-*` full name | `claude` | `--coding-agent claude --model <name>` (`--coding-agent claude` may be omitted — it is the default) |
| `gpt-*` (e.g. `gpt-5.5`, `gpt-5.4-mini`, `gpt-5.3-codex-spark`) | `codex` | `--coding-agent codex --model <name>` |
| Any name containing a `/` (e.g. `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5`, `opencode/big-pickle`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>` |
| Anything else — no shape match (e.g. `gemini-2.5-pro`, `o3-mini`, any unfamiliar bare name) | none — do NOT infer | Ask the operator which backend to use (or for the explicit `--coding-agent` + `--model` pair) before spawning |

**Template safety**: because custom prompts go through `str.format()` whether or not they contain placeholders, any literal `{` or `}` in the prompt text must be doubled (`{{` / `}}`).

**Spawn prompt size limit (use `--prompt-file` for templated identity blocks)**: cafleet hands the prompt to `tmux split-window` as a single positional argument, subject to `ARG_MAX`. A large inline positional prompt surfaces as `tmux command failed: command too long` and rolls back the agent registration once the shell-quoted prompt grows past a few KB. `--prompt-file` avoids this: only the path flows through the argv, and cafleet reads the file, runs `str.format()`, and hands the substituted text to `tmux split-window` directly. Use `--prompt-file` for every templated identity block + role-file-by-path prompt. Inline `-- "<prompt>"` remains a first-class input for trivial one-line ad-hoc spawns (e.g. test scripts, doctor flows).

Whichever input mode is used, keep the prompt body itself focused: role-file path, skill-load list, fleet/agent/director IDs, the operational context (output dir, current date, user request), and "start now" cue. The member loads the role file via `Read` on its first turn; the role files live in the skill directory and are stable, so path-by-reference is safe.

**Member Create — Scratch and audit files**: Spawn-related scratch (working notes, intermediate renders) MUST be written under `${BASE}` (resolved by the `cafleet-base-dir` skill) or under the skill's resolved output directory — never `/tmp`. The pre-spawn `--prompt-file` write at `<BASE>/prompts/<role>-<UTC-compact>.md` is the canonical audit artifact for every CAFleet-native team-skill spawn:

- `<role>` is the lowercased value of `--name` (e.g., `drafter`, `reviewer`, `programmer`, `tester`, `verifier`, `manager`, `analyzer`).
- `<UTC-compact>` is `YYYYMMDDTHHMMSSZ` (Python: `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`).
- The skill creates the `<BASE>/prompts/` subdirectory on first write (Python: `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`).
- Same-second collisions: skills MUST NOT overwrite an existing file. If the target path already exists, append `_2`, `_3`, … until the name is unique.
- The pre-spawn file IS the audit artifact — there is no second post-spawn re-render write. The file path used for `--prompt-file` is the single source of truth for what was spawned, in perpetuity.

**`${BASE} == <unset>` fallback**: when startup-time `${BASE}` resolution returned the `<unset>` sentinel, follow the guarded-skip protocol in the `cafleet-base-dir` skill § *No-bypass write protocol* — skip the `<BASE>/prompts/<role>-<ts>.md` write, fall back to the inline positional form (keep it under ~2 KB, path-by-reference), and emit the anchorless status `audit-disabled no BASE in spawn prompt` once per spawn cycle. The spawn still proceeds.

**Backtick caveat (harness-dependent)**: Some operator environments (including this project) ship a Bash-validator hook that rejects any backtick in a `Bash` invocation — even inside single-quoted positional arguments — because the validator treats backticks as command-substitution syntax. When such a harness is in play, strip backticks from spawn-prompt bodies (use plain text where you would otherwise use markdown code spans). Path-by-reference for role docs sidesteps this entirely: the prompt body becomes short enough that backticks are easy to avoid.

**Pane title (claude backend only)**: `claude --name <member-name>` forwards the name to the spawned process so `#{pane_title}` shows the member name. Neither the `codex` nor the `opencode` backend has a `--name` analog. Operators discover panes via `cafleet member list` (`pane_id` column is ground truth for all three backends).

**Focus behavior**: the spawn always invokes `tmux split-window` with `-d` so the Director's pane and active window keep focus — the new member pane is created in the Director's window but is not made active, and the calling client's active window is not switched.

If the tmux `split-window` fails, the registered agent is rolled back. If the placement PATCH fails, the pane is `/exit`'d and the agent rolled back.

## Member Delete

The CLI sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters the agent and rebalances the layout. On timeout, the pane buffer tail is captured and printed on stderr, and the command exits 2 without deregistering. Rerun with `--force` to skip `/exit` and kill the pane immediately.

```bash
cafleet member delete --fleet-id <fleet-id> \
  --member-id <member-agent-id>

cafleet member delete --fleet-id <fleet-id> \
  --member-id <member-agent-id> --force
```

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, deregister, rebalance layout. Exit 0 even if the pane was already gone. |

Exit codes: `0` success / `1` non-timeout failure / `2` default-path timeout (buffer tail printed on stderr).

The only boundary is fleet isolation: a `--member-id` outside `--fleet-id` exits 1 with `Error: Agent <member-id> not found`. There is no caller-auth check. (Deleting the root Director stays blocked downstream by `broker.deregister_agent`'s root-Director guard.)

## Member List (with `--activity`)

```bash
cafleet member list --fleet-id <fleet-id>
cafleet member list --fleet-id <fleet-id> --activity
```

Default columns: `agent_id`, `name`, `status`, `backend`, `session`, `window_id`, `pane_id`, `created_at`. Pending placement renders `(pending)` in text mode, `null` in JSON.

`--activity` adds `last_sent` / `last_recv` / `last_ack` / `idle` columns aggregated from `tasks`:

```
$ cafleet member list --fleet-id <s> --activity
3 members:
  agent_id  name      state   last_sent    last_recv    last_ack     idle
  --------  --------  ------  -----------  -----------  -----------  -----
  5         alice     active  12:34:56     12:34:50     12:34:50     6s
  6         bob       active  12:30:11     12:33:02     12:33:02     2m
  7         carol     idle    -            12:20:00     12:20:00     14m
```

The `last_ack` aggregation filters `Task.type != 'broadcast_summary'` (mirrors `poll_tasks`). Use `--activity` for routine supervision ticks instead of capturing every member every tick — capture is reserved for the cases the activity columns flag.

## Member Capture

Capture the last N lines of a member's pane buffer. Default `--lines 30`; `--no-ansi` is the default and strips ANSI escapes.

```bash
cafleet member capture --fleet-id <fleet-id> \
  --member-id <member-agent-id>

cafleet member capture --fleet-id <fleet-id> \
  --member-id <member-agent-id> --lines 200
```

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--lines` / `--tail` | no | Trailing lines to capture (default 30). `--tail` is an alias for `--lines`. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences stripped, carriage returns de-fragmented. `--ansi` emits the raw tmux capture. |

A `--member-id` outside `--fleet-id` is rejected with `Error: Agent <member-id> not found` (fleet isolation; no caller-auth check). Output is the raw captured buffer with no framing in text mode; JSON wraps it in `{member_agent_id, pane_id, lines, content}`.

## Member Send-Input

Forward a restricted keystroke to a member's tmux pane — write-path companion to `member capture`. AskUserQuestion-only.

| Flag | Required | Notes |
|---|---|---|
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

## Member Exec

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into the member's pane via `tmux.send_bash_command` so the coding agent's `!` shortcut runs the command natively. All three backends — `claude`, `codex`, and `opencode` — honor the leading-`!` shortcut. See [`reference/exec-routing.md`](exec-routing.md) for the full bash-via-Director fallback protocol.

```bash
cafleet member exec --fleet-id <fleet-id> \
  --member-id <member-agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading/trailing whitespace stripped. Pipes, `&&`, `;`, `$(...)`, backticks not special-cased. |

Validation: missing `COMMAND` → exit 2 (Click built-in). Empty/whitespace-only → exit 2 (`Error: command may not be empty.`). Newlines → exit 2 (`Error: command may not contain newlines.`).

### Required follow-up: `cafleet member ping`

After every successful `cafleet member exec` (exit 0), the Director MUST immediately invoke `cafleet member ping` against the same member. `member exec` only stages the bang-command's stdout/stderr as context for the member's next turn — it does not advance the turn. The follow-up primitive is `cafleet member ping`, NOT `cafleet message poll` (poll polls the Director's own inbox; ping injects the keystroke into the member's pane).

Skip the ping only on non-zero `member exec` exit — the dispatch did not complete and the supervision tick (driven by the monitoring member's idle nudge on the `cafleet monitor` heartbeat) is the safety net.

For a series of `member exec` calls on the same member, the ping follows each exec, not only the last.

## Member Ping

Director-only manual inbox-poll nudge. The broker's auto-fire on every `cafleet message send` is an inline preview keystroked into the recipient's pane (`tmux.send_inline_preview`). `member ping` is the manually-invokable counterpart for re-poking a member that missed an inline preview. It reuses the `send_poll_trigger` helper, so it keystrokes **`Esc` → `cafleet … message poll` → `Enter`** (Escape, ~0.1 s settle, then the literal poll command and Enter): the leading `Esc` dismisses any pending permission-approval prompt in the member's pane, so the trailing `Enter` cannot blindly confirm it. `send_poll_trigger` is **unchanged** by the monitor-heartbeat narrowing — the monitor loop no longer calls it (it wakes only the monitoring member via `send_wake_trigger`), but the helper and its `esc_first=True` safeguard survive intact for `cafleet member ping`'s manual recovery use.

```bash
cafleet member ping --fleet-id <fleet-id> \
  --member-id <member-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |

The action is wholly determined by the subcommand name — there is no positional argument and no operator-controlled keystroke body, so this subcommand sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

## Member Nudge

The monitoring member's purpose-built re-engage primitive for waking an idle Director. Unlike `member ping` (a pure poll keystroke), `member nudge` carries a summary: it **persists an ACKable broker task** (so the Director's facilitation loop still sees an inbox item naming what needs attention) **and** fires the hardened, `Esc`-safeguarded inline preview into the target's pane.

```bash
cafleet member nudge --fleet-id <fleet-id> --agent-id <monitoring-member-id> \
  --member-id <director-agent-id> --text "<re-engage summary>"
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **sender** (typically the monitoring member). Persisted as the task's `from_agent_id`. |
| `--member-id` | yes | The **target** (typically the root Director). Resolved with fleet-isolation only — any active in-fleet agent with a placement is a valid target. |
| `--text` | yes | The re-engage summary (un-ACKed inbox items, stalled members). Empty / whitespace-only is rejected (exit 2). |

The `--agent-id` + `--member-id` pairing is the **asymmetry** that distinguishes `member nudge` from the other member subcommands: only `nudge` and `create` carry a real acting identity (`--agent-id`); `delete` / `capture` / `exec` / `ping` are pure Director-initiated keystrokes that take `--member-id` alone. **Auth is fleet-isolation only** — there is no caller-auth check (consistent with the rest of the `member` subgroup); the only boundary is that a cross-fleet / unknown / inactive `--member-id` resolves to "Agent `<id>` not found" (exit 1).

`member nudge` reuses the broker send path: `send_message` persists the `unicast` / `input_required` task **and** best-effort fires the now `Esc`-safeguarded inline preview. So a Director sitting on a deny-listed permission prompt has that prompt dismissed by the leading `Esc` before the preview's `Enter` lands — `member nudge` and a monitoring-member `cafleet message send --to <director>` now deliver the same persist + hardened-preview effect; `member nudge` is the named interface over that path. A target with no live pane is tolerated (the task still persists; the keystroke best-effort no-ops). Because `--text` is agent-controlled like `message send`, the subcommand sits in `permissions.allow` (`Bash(cafleet member nudge --fleet-id *)`).

## Cross-references

- For broadcast send/ack semantics, see [`reference/broadcast.md`](broadcast.md).
- For the bash-via-Director fallback protocol, see [`reference/exec-routing.md`](exec-routing.md).
- For crash/disconnect/idle recovery flows including the Shutdown Protocol, see [`reference/recovery.md`](recovery.md).
- For `--full` / `--json` opt-back-in semantics, see [`reference/output-flags.md`](output-flags.md).
