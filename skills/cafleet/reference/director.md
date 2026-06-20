# tmux-backed member commands (`cafleet member *`)

Reference page for the `cafleet member` subgroup — `create`, `delete`, `list` (with `--activity`), `capture`, `exec`, `ping`, `nudge`. All run inside a tmux session, scoped to the per-subcommand `--fleet-id`. `member create` takes `--agent-id` (the spawning Director, validated to equal the fleet root); `member nudge` is the only Director-targeting one and takes **both** `--agent-id` (sender, the monitoring member) and `--member-id` (target); the rest identify their target by `--member-id` alone.

Members do NOT need to read this file. Member-side flows (poll / send / ack / receive shell-dispatch from the Director) live in `skills/cafleet/SKILL.md` (core) and `skills/cafleet/reference/exec-routing.md`.

## Member Create

Register a new member agent and spawn a coding-agent pane in the Director's own tmux window. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"

cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Codex-A --description "Reviewer for PR #42" --coding-agent codex

cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name monitor --description "Monitoring member: owns the heartbeat" \
  --role monitor --model {monitor_model} \
  --prompt-file /abs/path/to/<BASE>/prompts/monitor-20260514T145000Z.md
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`; also recorded as `placement.coding_agent`. Exits 1 with `Error: binary <name> not found on PATH` when the binary is absent. The `opencode` backend materializes its agent preset on first spawn. |
| `--model` | no | Pins the member's LLM (omitted → the binary's default; spawn-time only). The model-name-to-backend inference table below maps a bare model name to its backend and lists a per-backend example; the available-models-per-backend tables that follow it list the common models for each backend. See [`cli-options.md`](../../../docs/spec/cli-options.md#member-create). |
| `--role` | no | One of `member` (default) or `monitor`. `monitor` spawns the fleet's dedicated **monitoring member** (sets `agent_card_json.cafleet.kind == "monitoring-member"`); the monitoring member is the unenrolled **watcher** that runs the loop — it is **not** enrolled in `monitor_config` and carries no interval (the loop instead watches the Director at 180 s + members at 720 s and wakes the monitoring member when one is due). An ordinary `--role member` with a pane IS enrolled (720 s). The LLM is still set by `--model` (the Director passes the monitor model `{monitor_model}`). One per fleet — a second `--role monitor` is rejected (exit 1). Spawned **first** and runs `cafleet monitor start`; see the `cafleet-agent-team-monitoring` skill § The monitoring member for the canonical prompt and first-in/first-out lifecycle. |
| `--prompt-file` | no | Absolute path to a UTF-8 file used as the spawn prompt (mutually exclusive with the positional prompt; read verbatim, same `str.format()` pass). Path/file errors are catalogued in [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages). The canonical input mode for every team-skill spawn — see § *Member Create — Scratch and audit files*. |
| *(positional, after `--`)* | no | Prompt for the spawned process (mutually exclusive with `--prompt-file`; the default template is used if both are omitted). Goes through `str.format()` with `fleet_id` / `agent_id` / `director_agent_id` as kwargs. |

The per-backend spawn argv is in [`cli-options.md`](../../../docs/spec/cli-options.md#member-create) § Spawn command per backend. In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve; the deny-list fallback is [`reference/exec-routing.md`](exec-routing.md). Per-backend deltas: [`claude`](coding-agent/claude.md) / [`codex`](coding-agent/codex.md) / [`opencode`](coding-agent/opencode.md).

### Model-name-to-backend inference

When the operator names a model rather than a backend ("please create a member with sonnet"), resolve the backend from the model-name shape:

| Model name shape | Backend | Flags to pass |
|---|---|---|
| Contains a `/` — provider-prefixed (e.g. `opencode/gpt-5.5`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>` |
| `gpt-*` (e.g. `gpt-5.5`, `gpt-5.4-mini`) | `codex` | `--coding-agent codex --model <name>` |
| Claude alias or `claude-*` full name — `fable`, `opus`, `sonnet`, `haiku`, `best`, `default`, `opusplan`, `sonnet[1m]`, `opus[1m]`, `claude-opus-4-8`, … | `claude` (default) | `--model <name>` (`--coding-agent claude` is the default and may be omitted) |
| Any other bare name — no shape match (e.g. `gemini-2.5-pro`, `o3-mini`) | none — do NOT infer | Ask the operator for the explicit `--coding-agent` + `--model` pair |

Followed by the routing rule as ordered precedence:

> Resolve the backend in this order — the first match wins:
> 1. **Name contains a `/`** → `opencode`. The provider-prefixed form is the explicit "use opencode" signal; opencode is never inferred from a bare name.
> 2. **Name matches `gpt-*`** → `codex`.
> 3. **Name is a Claude alias (`fable` / `opus` / `sonnet` / `haiku` / `best` / `default` / `opusplan` / `sonnet[1m]` / `opus[1m]`) or a `claude-*` full name** → `claude`, the default backend (`--coding-agent` may be omitted).
> 4. **Anything else** → do not infer; ask the operator for the explicit `--coding-agent` + `--model` pair.

Precedence matters for the slash case: `anthropic/claude-sonnet-4-6` contains both `claude` and a `/`, and rule 1 (slash → opencode) wins over rule 3.

### Available models per backend

**Claude Code (`--coding-agent claude`)** — simple list:

| Model | For |
|---|---|
| `fable` | hardest, longest-running tasks |
| `opus` | complex reasoning |
| `sonnet` | everyday coding |
| `haiku` | fast, simple tasks |
| `best` | Fable 5 if the org has access, else the latest Opus |
| `default` | clears the override; returns to the account-tier model |
| `opusplan` | `opus` in Plan Mode, `sonnet` during execution |
| `sonnet[1m]` | `sonnet` with a 1M-token context window |
| `opus[1m]` | `opus` with a 1M-token context window |

Full names: `claude-fable-5`, `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`.

**Codex (`--coding-agent codex`)**:

| Model | Notes |
|---|---|
| `gpt-5.5` | newest frontier; default / recommended |
| `gpt-5.4` | flagship frontier — professional coding & reasoning |
| `gpt-5.4-mini` | fast, efficient mini — responsive tasks and subagents |
| `gpt-5.3-codex-spark` | text-only research preview (ChatGPT Pro) — near-instant iteration |

**OpenCode Zen (`--coding-agent opencode`)** — the OpenCode Zen catalog ([opencode.ai/docs/zen](https://opencode.ai/docs/zen/)). Every Zen model is passed with the `opencode/` gateway prefix, i.e. `opencode/<model-id>` (e.g. `opencode/gpt-5.5`, `opencode/claude-sonnet-4-6`, `opencode/gemini-3.5-flash`). The Models column lists the bare `<model-id>`; prepend `opencode/`:

| Provider | Models (pass as `opencode/<model-id>`) |
|---|---|
| OpenAI | `gpt-5.5`, `gpt-5.5-pro`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.4-nano`, `gpt-5.3-codex` |
| Anthropic | `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5` |
| Google | `gemini-3.5-flash` |

Other providers: Qwen, DeepSeek, Kimi, GLM, MiniMax, Grok.
Free (limited beta): Big Pickle, DeepSeek V4 Flash Free, MiMo-V2.5 Free, North Mini Code Free, Nemotron 3 Ultra Free.

The S1 routing rule accepts any `<provider-id>/<model-id>` for the `opencode` backend, including direct-provider forms such as `anthropic/claude-sonnet-4-6` or `openai/gpt-5.5`; the Zen catalog above is normalized to the `opencode/` gateway prefix, and the direct-provider examples elsewhere in `director.md` / `README.md` / `coding-agents.md` stay valid.

**Template safety**: because custom prompts go through `str.format()` whether or not they contain placeholders, any literal `{` or `}` in the prompt text must be doubled (`{{` / `}}`).

**Spawn prompt size limit**: cafleet passes the prompt to `tmux split-window` as one positional argument, so a large inline prompt fails with `tmux command failed: command too long` (and rolls back the registration) past a few KB. Use `--prompt-file` for every templated identity block + role-file-by-path prompt; inline `-- "<prompt>"` stays first-class for trivial one-line ad-hoc spawns.

Keep the prompt body focused (the skeleton below): the member loads its role file via `Read` on its first turn, so path-by-reference to the stable in-skill role docs is safe.

### Canonical spawn-prompt skeleton

Every CAFleet-native team skill spawns its ordinary members from this one shared frame; each skill supplies only a compact **per-role delta** (a table in that skill) for the parts that vary. (The dedicated **monitoring member** is the exception — its canonical prompt lives in the `cafleet-agent-team-monitoring` skill § The monitoring member.)

Fixed frame — `{fleet_id}` / `{agent_id}` / `{director_agent_id}` are filled by the CLI's `str.format()` pass; `[INSERT …]` markers are shell-substituted by the Director before `member create`; the `‹…›` slots are filled from the per-role delta:

```text
You are ‹ROLE TITLE› in a ‹TEAM NAME› team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/‹role›.md] with the Read tool BEFORE any other action. That file is your authoritative role definition.‹ROLE-DEF SUFFIX› Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — ‹cafleet-load purpose›
‹EXTRA SKILL LOADS›

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
CODING AGENT: [INSERT the backend name the Director chose at member create — claude / codex / opencode]
‹CONTEXT LINES›

COMMUNICATION PROTOCOL:
- Report to Director: cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "‹report-hint›"
- ‹POLL-HANDLING LINE›
‹EXTRA COMMS LINES›

‹IMPORTANT / ROLE-CONSTRAINT LINES›

‹START CUE›
```

The `CODING AGENT:` line names the member's coding-agent backend (`claude` / `codex` / `opencode`). The Director fills it as a rendered literal the same way it fills `BASE` — from the `--coding-agent` value it chose at `member create` — so no CLI code change is required. The member reads its overlay `coding-agent/<name>.md` deterministically from this line and applies the overlay's deltas on top of the base.

Per-role delta slots (each consuming skill's spawn section fills these):

| Slot | Filled per role with |
|---|---|
| `‹ROLE TITLE›` / `‹TEAM NAME›` | e.g. `the Programmer` / `design document execution`; `a Scout Researcher` / `research`; `the Presentation Specialist` / `research presentation`. |
| `‹role›` + `‹ROLE-DEF SUFFIX›` | The `roles/<role>.md` filename, plus any addendum after "…role definition." — e.g. resume-mode `Follow the Resume Mode section in particular.`; the research roles' `— accountability, …, and shutdown.` enumeration. Empty for most roles. |
| `‹cafleet-load purpose›` + `‹EXTRA SKILL LOADS›` | The cafleet purpose phrase (`for communication with the Director`, or `for the broker primitives and bash-via-Director routing`), plus any extra startup skills — `cafleet-design-doc` (design-doc family); `cafleet-my-slidev` + `cafleet-create-figure` (Presentation Specialist). |
| `‹CONTEXT LINES›` | Role inputs, one per line: `DESIGN DOCUMENT` / `OUTPUT PATH` / `CURRENT DATE` / `USER REQUEST` / `OUTPUT DIRECTORY` / `LANGUAGE` / `YOUR ASSIGNMENT` / `OUTPUT FILE` / `YOUR TASK ID` / `REPORT` / `SLIDE FILE` / `SERVER URL` / `ROUND`, etc. |
| `‹report-hint›` + `‹POLL-HANDLING LINE›` + `‹EXTRA COMMS LINES›` | The `--text` hint (`your report` / `your numbered question list`). The poll-handling line is either the simple `When you see cafleet message poll output with a message from the Director, act on those instructions.` (create / execute / interview) or the **ack-inline** form `…capture the id: integer id from each entry as [task-id] and ack it via cafleet message ack … --task-id [task-id], then act on the instructions.` (research / presentation). Extra comms lines: the Manager's `You do NOT talk to Scouts or Researchers directly…` + shared-task-list lines; the Researcher's task-list claim/complete lines. |
| `‹IMPORTANT / ROLE-CONSTRAINT LINES›` | Every `IMPORTANT:` line and hard role constraint, verbatim (see lossless rule). |
| `‹START CUE›` | The role's closing instruction — e.g. `Start by reading the design document. Then wait for the Director to assign your first step.`; `Read the design document, generate a numbered question list …`; `When complete, send the file path to the Director …`. |

**Lossless rule (non-negotiable).** When a skill collapses its inline spawn prompts to "this skeleton + a per-role delta", the per-role delta MUST reproduce **every** `IMPORTANT:` line, hard role-constraint, and start cue from the original prompt **verbatim** — none dropped or paraphrased. These lines are the behavioral contract of the spawn; the reconstruction check asserts each maps to a delta row. Lines that MUST survive every collapse include:

- Programmer: `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.`
- Tester: `IMPORTANT: Do NOT write implementation code — only test code.` (plus the Programmer no-commit line).
- Verifier: `IMPORTANT: Do NOT commit code or modify implementation/test files.`
- All execute roles: `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` and `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`
- Drafter (normal mode): `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.` and `Do NOT create any design document file until you have received answers.`; (resume mode) `Do NOT ask clarifying questions — the COMMENTs contain the needed information.`

**Member Create — Scratch and audit files**: Spawn-related scratch (working notes, intermediate renders) MUST be written under `${BASE}` (resolved by the `cafleet-base-dir` skill) or under the skill's resolved output directory — never `/tmp`. The pre-spawn `--prompt-file` write at `<BASE>/prompts/<role>-<UTC-compact>.md` is the canonical audit artifact for every CAFleet-native team-skill spawn:

- `<role>` is the lowercased `--name`; `<UTC-compact>` is `YYYYMMDDTHHMMSSZ`. Create `<BASE>/prompts/` on first write; on a same-second collision append `_2`, `_3`, … (never overwrite).
- The pre-spawn file IS the audit artifact — there is no post-spawn re-render. The `--prompt-file` path is the single source of truth for what was spawned, in perpetuity.

**`${BASE} == <unset>` fallback**: when startup-time `${BASE}` resolution returned the `<unset>` sentinel, follow the guarded-skip protocol in the `cafleet-base-dir` skill § *No-bypass write protocol* — skip the `<BASE>/prompts/<role>-<ts>.md` write, fall back to the inline positional form (keep it under ~2 KB, path-by-reference), and emit the anchorless status `audit-disabled no BASE in spawn prompt` once per spawn cycle. The spawn still proceeds.

**Backtick caveat (harness-dependent)**: some environments (including this project) run a Bash-validator hook that rejects any backtick in a `Bash` invocation. When in play, strip backticks from spawn-prompt bodies (plain text instead of code spans); path-by-reference keeps the body short enough that this is easy.

**Pane discovery**: discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Pane title: {pane_title}. The spawn is atomic — a `split-window` or placement-patch failure rolls back the registration (and `/exit`s the pane on a patch failure) — and uses `-d` so the Director keeps focus. See [`member-lifecycle.md`](../../../docs/concepts/member-lifecycle.md).

## Member Delete

The CLI sends `/exit`, waits for the pane to close (15 s timeout), then deregisters and rebalances the layout; on timeout it exits 2 with the pane buffer tail on stderr (no deregister). `--force` / `-f` skips the wait and kill-panes immediately (exit 0 even if the pane was already gone).

```bash
cafleet member delete --fleet-id <fleet-id> --member-id <member-agent-id>
cafleet member delete --fleet-id <fleet-id> --member-id <member-agent-id> --force
```

Fleet-isolation only: a `--member-id` outside `--fleet-id` exits 1 (`Error: Agent <member-id> not found`); deleting the root Director stays blocked by the root-Director guard. Exit codes and the timeout output shape: [`cli-options.md`](../../../docs/spec/cli-options.md#member-delete).

## Member List (with `--activity`)

```bash
cafleet member list --fleet-id <fleet-id>
cafleet member list --fleet-id <fleet-id> --activity
```

Default columns: `agent_id`, `name`, `status`, `backend`, `session`, `window_id`, `pane_id`, `created_at` (pending placement renders `(pending)` / `null`). `--activity` adds `last_sent` / `last_recv` / `last_ack` / `idle` aggregated from `tasks` (output shape in [`cli-options.md`](../../../docs/spec/cli-options.md#member-list-activity-output)). Use `--activity` for routine supervision ticks instead of capturing every member every tick — capture is reserved for the cases the activity columns flag.

## Member Capture

Capture the last N lines of a member's pane buffer (read-only). `--lines` / `--tail` default `30`; `--ansi` / `--no-ansi` (default) strips ANSI escapes and de-fragments carriage returns. Output is the raw buffer in text mode, `{member_agent_id, pane_id, lines, content}` in JSON.

```bash
cafleet member capture --fleet-id <fleet-id> --member-id <member-agent-id>
cafleet member capture --fleet-id <fleet-id> --member-id <member-agent-id> --lines 200
```

## Answering a member's relayed question

A fleet member never talks to the user. When it needs a recorded user reaction (approve / choose / confirm / continue-or-abort), it relays the question to the Director via `cafleet message send`, and the Director answers it through {decision_surface}. The question-shape taxonomy and any pane-keystroke relay for forwarding the answer are backend deltas — see your overlay (`coding-agent/<name>.md`). The canonical user-reaction rule is the `cafleet` skill § *Soliciting user reactions*.

## Member Exec

Director-only shell-dispatch primitive: keystrokes `! <command>` + `Enter` so the coding agent's `!` shortcut runs the command natively (all three backends honor it). The positional `COMMAND` is a single shell command (leading/trailing whitespace stripped; pipes / `&&` / `;` / `$(...)` / backticks not special-cased; empty or newline-containing commands exit 2). See [`reference/exec-routing.md`](exec-routing.md) for the full fallback protocol and [`cli-options.md`](../../../docs/spec/cli-options.md#member-exec) for validation.

```bash
cafleet member exec --fleet-id <fleet-id> --member-id <member-agent-id> "git log -1 --oneline"
```

### Required follow-up: `cafleet member ping`

After every successful `cafleet member exec` (exit 0), the Director MUST immediately invoke `cafleet member ping` against the same member. `member exec` only stages the bang-command's stdout/stderr as context for the member's next turn — it does not advance the turn. The follow-up primitive is `cafleet member ping`, NOT `cafleet message poll` (poll polls the Director's own inbox; ping injects the keystroke into the member's pane).

Skip the ping only on non-zero `member exec` exit (the dispatch did not complete; the supervision tick is the safety net). For a series of execs on the same member, the ping follows each one, not only the last.

## Member Ping

Director-only manual inbox-poll nudge: keystrokes **`Esc` → `cafleet … message poll` → `Enter`** into a member's pane (the leading `Esc` dismisses any pending permission-approval prompt, so the trailing `Enter` cannot blindly confirm it) for re-poking a member that missed the broker's auto-fired inline preview. The action is fully fixed by the subcommand name — no positional argument, no operator-controlled body — so it sits in `permissions.allow` while `member exec` stays in `permissions.ask`. Keystroke mechanics: [`tmux-push.md`](../../../docs/concepts/tmux-push.md).

```bash
cafleet member ping --fleet-id <fleet-id> --member-id <member-agent-id>
```

## Member Nudge

The monitoring member's purpose-built re-engage primitive for waking an idle Director. Unlike `member ping` (a pure poll keystroke), `member nudge` **persists an ACKable broker task** (so the Director's facilitation loop sees an inbox item naming what needs attention) **and** fires the hardened, `Esc`-safeguarded inline preview into the target's pane — the same persist + preview effect as a monitoring-member `cafleet message send --to <director>`, just the named interface over it.

```bash
cafleet member nudge --fleet-id <fleet-id> --agent-id <monitoring-member-id> \
  --member-id <director-agent-id> --text "<re-engage summary>"
```

It is the **asymmetric** member subcommand: with `create` it is one of only two carrying a real acting `--agent-id` (the sender, typically the monitoring member); `--member-id` is the target (typically the root Director, fleet-isolation resolution); `--text` is the summary (empty rejected, exit 2). A target with no live pane is tolerated (the task still persists). Because `--text` is agent-controlled, it sits in `permissions.allow`. Full surface: [`cli-options.md`](../../../docs/spec/cli-options.md#member-nudge).

## Cross-references

- For broadcast send/ack semantics, see [`reference/broadcast.md`](broadcast.md).
- For the bash-via-Director fallback protocol, see [`reference/exec-routing.md`](exec-routing.md).
- For crash/disconnect/idle recovery flows including the Shutdown Protocol, see [`reference/recovery.md`](recovery.md).
- For `--full` / `--json` opt-back-in semantics, see [`reference/output-flags.md`](output-flags.md).
