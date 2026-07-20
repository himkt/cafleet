# tmux-backed member commands (`cafleet member *`)

Reference page for the Director-only lifecycle and pane-interaction commands — `member create`, `member delete`, `member list`, `member capture`, `member exec`, `member ping`. All run inside a tmux or herdr session, scoped to the per-subcommand `--fleet-id` (`member list` and `member show` are registry reads with no multiplexer requirement). `member create` takes **no identity flag** — the CLI auto-resolves the spawning Director from `fleets.director_member_id`; every other lifecycle verb identifies its **target** by `--member-id`.

Members do NOT need to read this file. Member-side flows (poll / send / ack / receive shell-dispatch from the Director) live in `skills/cafleet/SKILL.md` (core) and `skills/cafleet/reference/exec-routing.md`.

## Member Create

Register a new member and spawn a coding-agent pane in the Director's own tmux window. The command auto-resolves the Director from the fleet row, atomically registers the member, creates a placement row, renders the spawn prompt, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-B --description "Reviewer for PR #42" \
  --text-file /abs/path/to/<BASE>/.prompts/reviewer-b-20260514T145000Z.md

cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-C --description "Reviewer for PR #42" --coding-agent codex \
  --text-file /abs/path/to/<BASE>/.prompts/reviewer-c-20260514T145000Z.md

cafleet member create --fleet-id <fleet-id> \
  --name monitor --description "Monitoring member: owns the heartbeat" \
  --role monitor --model {monitor_model} \
  --text-file /abs/path/to/<BASE>/.prompts/monitor-20260514T145000Z.md
```

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Display name of the new member. |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; also recorded as `placement.coding_agent`. When omitted, the member — every role — inherits **your** (the spawning Director's) backend from your placement row, so an unflagged team runs on the same backend as its Director. An explicit value always wins. Exits 1 with `Error: binary <name> not found on PATH` when the binary is absent, or with `opencode agent preset not found at <preset>; run 'cafleet setup' first` when the opencode agent preset is missing. |
| `--model` | no | Pins the member's LLM (omitted → the binary's default; spawn-time only). The model-name-to-backend inference table below maps a bare model name to its backend; the model list at [`model-list.md`](model-list.md) lists the models for each backend. See [`cli-options.md`](../../../docs/spec/cli-options.md#member-create). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary (omitted → the binary's default; spawn-time only, never persisted). claude: `low`, `medium`, `high`, `xhigh`, `max` (spawned as `--effort <level>`); codex: `minimal`, `low`, `medium`, `high`, `xhigh` (spawned as `--config=model_reasoning_effort=<level>`); opencode: unsupported — any value exits 2 with `opencode does not support reasoning effort.`. An unknown level exits 2 before any side effect. The per-backend level set is your overlay's `{effort_levels}` value. |
| `--role` | no | One of `member` (default) or `monitor`. `monitor` spawns the fleet's dedicated **monitoring member** (sets `member_card_json.cafleet.kind == "monitoring-member"`); the monitoring member is the unenrolled **watcher** that runs the loop — it is **not** enrolled in `monitor_config` and carries no interval (the loop watches the Director and members on their own intervals and wakes the monitoring member when one is due — see [`reference/supervision.md`](supervision.md)). An ordinary `--role member` with a pane IS enrolled. The LLM is still set by `--model` (the Director passes the monitor model `{monitor_model}`, resolved from its overlay — see § *Model selection before member create*); the backend follows the general inheritance rule when `--coding-agent` is omitted (see the `--coding-agent` row). One per fleet — a second `--role monitor` is rejected (exit 1). Spawned **first** and runs `cafleet monitor start`; see [`roles/monitor.md`](../roles/monitor.md) for the canonical prompt and first-in/first-out lifecycle. |
| `--text` | no | Inline spawn prompt. Mutually exclusive with `--text-file`; exactly one of the two is required. |
| `--text-file` | no | Path to a UTF-8 file used as the spawn prompt — absolute, or relative to CWD; `-` reads the whole prompt from stdin. Mutually exclusive with `--text`; exactly one of the two is required. Path/file errors are catalogued in [`cli-options.md`](../../../docs/spec/cli-options.md#error-messages). The canonical input mode for every team-skill spawn — see § *Member Create — Scratch and audit files*. |

The per-backend spawn argv is in [`cli-options.md`](../../../docs/spec/cli-options.md#member-create) § Spawn command per backend. In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve; the denied-command fallback is [`reference/exec-routing.md`](exec-routing.md). Per-backend deltas: [`claude`](coding-agent/claude-overlay.md) / [`codex`](coding-agent/codex-overlay.md) / [`opencode`](coding-agent/opencode-overlay.md).

### Model-name-to-backend inference

When the operator names a model rather than a backend ("please create a member with sonnet"), resolve the backend from the model-name shape:

| Model name shape | Backend | Flags to pass |
|---|---|---|
| Contains a `/` — provider-prefixed (e.g. `opencode/gpt-5.5`, `anthropic/claude-sonnet-4-6`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>` |
| `gpt-*` (e.g. `gpt-5.6-terra`, `gpt-5.6-luna`) | `codex` | `--coding-agent codex --model <name>` |
| Claude alias or `claude-*` full name — `fable`, `opus`, `sonnet`, `haiku`, `claude-opus-4-8`, … | `claude` | `--coding-agent claude --model <name>` (omitting the flag inherits your own backend, which matches only when you are a claude Director) |
| Any other bare name — no shape match (e.g. `gemini-2.5-pro`, `o3-mini`) | none — do NOT infer | Ask the operator for the explicit `--coding-agent` + `--model` pair |

The rows apply as ordered precedence — the first match wins. This matters for the slash case: `anthropic/claude-sonnet-4-6` contains both `claude` and a `/`, and row 1 (slash → opencode) wins over row 3 — the provider-prefixed form is the explicit "use opencode" signal; opencode is never inferred from a bare name.

### Model list

Model availability, reviewed capability classes, standard token prices, and the official source links live in the model list at [`reference/model-list.md`](model-list.md), maintained via the `cafleet-model-list-refresh` skill. Consult the model list for the current model set and its spawn tokens; pass a listed model name or alias to `--model` exactly as written there.

The routing rule above accepts any `<provider-id>/<model-id>` for the `opencode` backend, including direct-provider forms such as `anthropic/claude-sonnet-4-6` or `openai/gpt-5.5`.

**Identity substitution (`str.format`)**: the four-placeholder `str.format` render (and the brace-doubling rule) is canonical in the cafleet [`SKILL.md`](../SKILL.md) § *Spawned-member identity via `str.format` substitution*. Director-side deltas: an unknown placeholder raises a `UsageError` listing the four supported names, a malformed brace expression raises the distinct "double literal braces" `UsageError`, and both (exit 2) roll back the just-registered member.

**Spawn prompt size limit**: cafleet passes the prompt to `tmux split-window` as one positional argument, so a large inline prompt fails with `tmux command failed: command too long` (and rolls back the registration) past a few KB. Use `--text-file` for every templated identity block + role-file-by-path prompt; inline `--text "<prompt>"` stays first-class for trivial one-line ad-hoc spawns.

**Long or multi-line message bodies**: the same `ARG_MAX` cliff applies to `message send` / `message broadcast` — a long or multi-line body MUST be passed via `--text-file` (or `--text-file -` for stdin), never inline `--text`. This rule is canonical in [`reference/supervision.md`](supervision.md) § Communication Model.

Keep the prompt body focused (the skeleton below): the member loads its role file via `Read` on its first turn, so path-by-reference to the stable in-skill role docs is safe.

### Canonical spawn-prompt skeleton

Every CAFleet-native team skill spawns its ordinary members from this one shared frame; each skill supplies only a compact **per-role delta** (a table in that skill) for the parts that vary.

Fixed frame — the identity lines carry the CLI's four `str.format` placeholders, rendered to literals by `cafleet member create` at spawn time; `[INSERT …]` markers are rendered by the Director before `member create`; the `‹…›` slots are filled from the per-role delta:

```text
You are ‹ROLE TITLE› in a ‹TEAM NAME› team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/‹role›.md] with the Read tool BEFORE any other action. That file is your authoritative role definition.‹ROLE-DEF SUFFIX› Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the cafleet skill — ‹cafleet-load purpose›
‹EXTRA SKILL LOADS›

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path]
CODING AGENT: {coding_agent}
‹CONTEXT LINES›

‹IMPORTANT / ROLE-CONSTRAINT LINES›

‹START CUE›
```

Rendering is **two-stage**: the Director substitutes the values it already knows as literals before the call (`BASE`, the absolute role-file path, the cafleet-load purpose phrase), then the CLI substitutes the four identity placeholders shown in the skeleton above (`{member_id}` is the member's own newly-allocated id, which only the CLI can fill). The Director must leave **no stray single braces** other than those four. After spawn the member sees literal labeled lines — `FLEET ID: 24`, `DIRECTOR MEMBER ID: 84`, `YOUR MEMBER ID: 88`, `CODING AGENT: claude` — and uses those integers on every `cafleet` command. There is no `COMMUNICATION PROTOCOL` command-example block: the member learns poll/send/ack command shapes from the `cafleet` skill and its role file. The `CODING AGENT:` line keeps the spawned binary and the overlay selector in lockstep for every role: when `--coding-agent` is omitted at spawn, the CLI records — and `{coding_agent}` renders — the backend inherited from the Director's placement row (see [`roles/monitor.md`](../roles/monitor.md) for the monitoring member's spawn). The member reads its overlay `coding-agent/<name>-overlay.md` deterministically from this line and resolves it onto the base — materializing each `{placeholder}` to its overlay value (or the documented default) and applying each bound note before emitting, per the cafleet `SKILL.md` § *Resolve your overlay*.

Per-role delta slots (each consuming skill's spawn section fills these):

| Slot | Filled per role with |
|---|---|
| `‹ROLE TITLE›` / `‹TEAM NAME›` | e.g. `the Programmer` / `design document execution`; `a Scout Researcher` / `research`; `the Presentation Specialist` / `research presentation`. |
| `‹role›` + `‹ROLE-DEF SUFFIX›` | The `roles/<role>.md` filename, plus any addendum after "…role definition." — e.g. resume-mode `Follow the Resume Mode section in particular.`; the research roles' `— accountability, …, and shutdown.` enumeration. Empty for most roles. |
| `‹cafleet-load purpose›` + `‹EXTRA SKILL LOADS›` | The cafleet purpose phrase (`for communication with the Director`, or `for the broker primitives and bash-via-Director routing`), plus any extra startup skills — `cafleet-design-doc` (design-doc family); `cafleet-research` (Presentation Specialist — its `reference/slidev.md` + `reference/visualization.md`). |
| `‹CONTEXT LINES›` | Role inputs, one per line: `DESIGN DOCUMENT` / `OUTPUT PATH` / `CURRENT DATE` / `USER REQUEST` / `OUTPUT DIRECTORY` / `LANGUAGE` / `YOUR ASSIGNMENT` / `OUTPUT FILE` / `YOUR TASK ID` / `REPORT` / `SLIDE FILE` / `SERVER URL` / `ROUND`, etc. |
| `‹IMPORTANT / ROLE-CONSTRAINT LINES›` | Every `IMPORTANT:` line and hard role constraint, verbatim (see lossless rule) — including each role's poll-handling line: either the simple `When you see cafleet message poll output with a message from the Director, act on those instructions.` (create / execute / interview) or the **ack-inline** form `…capture the id: integer id from each entry as [message-id] and ack it via cafleet message ack … --message-id [message-id], then act on the instructions.` (research / presentation), plus role coordination constraints such as the Manager's `You do NOT talk to Scouts or Researchers directly…` + shared-task-list lines and the Researcher's task-list claim/complete lines. |
| `‹START CUE›` | The role's closing instruction — e.g. `Start by reading the design document. Then wait for the Director to assign your first step.`; `Read the design document, generate a numbered question list …`; `When complete, send the file path to the Director …`. |

**Lossless rule (non-negotiable).** When a skill collapses its inline spawn prompts to "this skeleton + a per-role delta", the per-role delta MUST reproduce **every** `IMPORTANT:` line, hard role-constraint, and start cue from the original prompt **verbatim** — none dropped or paraphrased. These lines are the behavioral contract of the spawn; the reconstruction check asserts each maps to a delta row. Lines that MUST survive every collapse include:

- Programmer: `IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.`
- Tester: `IMPORTANT: Do NOT write implementation code — only test code.` (plus the Programmer no-commit line).
- Verifier: `IMPORTANT: Do NOT commit code or modify implementation/test files.`
- All execute roles: `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/exec-routing.md), which you load at startup.` and `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`
- Drafter (normal mode): `IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.` and `Do NOT create any design document file until you have received answers.`; (resume mode) `Do NOT ask clarifying questions — the COMMENTs contain the needed information.`

**Member Create — Scratch and audit files**: Spawn-related scratch (working notes, intermediate renders) MUST be written under `${BASE}` (resolved per [`reference/base-dir.md`](base-dir.md)) or under the skill's resolved output directory — never `/tmp`. The pre-spawn `--text-file` write at `<BASE>/.prompts/<role>-<UTC-compact>.md` is the canonical audit artifact for every CAFleet-native team-skill spawn:

- `<role>` is the lowercased `--name`; `<UTC-compact>` is `YYYYMMDDTHHMMSSZ`. Create `<BASE>/.prompts/` on first write; on a same-second collision append `_2`, `_3`, … (never overwrite).
- The pre-spawn file IS the audit artifact — there is no post-spawn re-render. The `--text-file` path is the single source of truth for what was spawned, in perpetuity. It carries the four `{...}` identity placeholders **pre-substitution** — that is expected; the CLI renders them at spawn.

**`${BASE} == <unset>` fallback**: when startup-time `${BASE}` resolution returned the `<unset>` sentinel, follow the guarded-skip protocol in [`reference/base-dir.md`](base-dir.md) § *No-bypass write protocol* — skip the `<BASE>/.prompts/<role>-<ts>.md` write, fall back to the inline `--text` form (keep it under ~2 KB, path-by-reference), and emit the anchorless status `audit-disabled no BASE in spawn prompt` once per spawn cycle. The spawn still proceeds.

**Backtick caveat (harness-dependent)**: some environments (including this project) run a Bash-validator hook that rejects any backtick in a `Bash` invocation. When in play, strip backticks from spawn-prompt bodies (plain text instead of code spans); path-by-reference keeps the body short enough that this is easy.

**Pane discovery**: discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Pane title: {pane_title}. The spawn is atomic — a `split-window` or placement-patch failure rolls back the registration (and exits the pane on a patch failure) — and uses `-d` so the Director keeps focus. See [`member-lifecycle.md`](../../../docs/concepts/member-lifecycle.md).

## Model selection before member create

The selection policy — the monitor/reviewer choices, cost efficiency mode and its exact trigger, the pick-backend-first / within-backend comparison rule, and the override and fail-closed rules — is canonical in [`roles/director.md`](../roles/director.md) § *Model selection*. Apply it against the model list of the exact `cafleet` skill root you loaded ([`reference/model-list.md`](model-list.md)) and pass the chosen pair to `member create`. A user-pinned model is never deleted and replaced automatically.

### Underpowered-member replacement

The Director owns member replacement; the Reviewer supplies evidence (an `[INCORRECT]` marker naming the suspected unmet capability) and stays independent of the execution. Slowness, awaiting user input, or a transient infrastructure error is never grounds for replacement. Valid evidence: a member's self-report that it cannot reason through the task, repeated task-relevant reasoning/coding failures after normal correction, a Reviewer `[INCORRECT]` finding naming the unmet capability, or a Director review of a materially incomplete/incorrect result tied to the task profile.

For each replacement, in order:

1. **Freeze and hand off** — freeze new work for the task and request one concise state report (completed work, modified paths, commands/tests run, blockers, next step); if the member cannot respond promptly, `cafleet member capture` is the handoff evidence.
2. **Re-select stronger** — pick a strictly more capable model from the failed model's backend table (a row above the failed model) that fits the task's now-demonstrated difficulty; choose the cheapest model within that stronger set.
3. **Record** — note the trigger, evidence pointers, old/new model, and attempt number in your coordination notes (no secrets or prompt contents).
4. **Delete before create** — `cafleet member delete` the old member through the standard lifecycle before spawning the replacement; the monitoring member stays live and all normal spawn/audit/prompt-substitution rules apply.
5. **Resume, not restart** — spawn the replacement with the original assignment plus the bounded handoff and the same deliverable paths, route the original task pointer, and have the Reviewer re-evaluate at the normal review point.

Caps and fail-closed cases: the initial member plus at most two replacements per task; each replacement strictly more capable than its predecessor; a `(task pointer, model)` pair is never retried. An unlisted manual model, any explicit user override, an empty stronger set, a reached cap, or ambiguous evidence all mean the Director relays an operator choice (approve a named higher-cost/manual override, simplify/re-scope, or stop) instead of auto-replacing.

## Member Delete

The CLI kills the pane immediately, then deregisters and rebalances the layout (exit 0 even if the pane was already gone). A member with a pending placement (no pane yet) is a plain registry soft-delete, and so is a placementless registry row — `member delete` handles both without touching tmux.

```bash
cafleet member delete --fleet-id <fleet-id> --member-id <member-id>
```

Fleet-isolation only: a `--member-id` outside `--fleet-id` exits 1 (`Error: Member <member-id> not found`); deleting the root Director stays blocked by the root-Director guard. Exit codes and the output shape: [`cli-options.md`](../../../docs/spec/cli-options.md#member-delete).

## Member List

```bash
cafleet member list --fleet-id <fleet-id>
```

One output shape: every **active** registry entry of the fleet (the root Director, the monitoring member, ordinary members, placementless rows), one row each with `member_id`, `name`, `kind` (`director` / `monitor` / `member`), `backend`, `pane_id` (a pending placement renders `(pending)`; placementless rows render `-` placement cells), and `idle` — the humanized wall-time since the member's most recent message activity (`Ns`/`Nm`/`Nh`, `-` when none). `--json` adds the underlying `last_sent` / `last_recv` / `last_ack` timestamps (output shape in [`cli-options.md`](../../../docs/spec/cli-options.md#member-list)). Use the `idle` column for routine supervision ticks instead of capturing every member every tick — capture is reserved for the cases the idle column flags.

## Member Capture

Capture the last N lines of a member's pane buffer (read-only). `--lines` defaults `20`; `--ansi` / `--no-ansi` (default) strips ANSI escapes and de-fragments carriage returns. Output is the raw buffer in text mode, `{member_id, pane_id, lines, content}` in JSON.

```bash
cafleet member capture --fleet-id <fleet-id> --member-id <member-id>
cafleet member capture --fleet-id <fleet-id> --member-id <member-id> --lines 200
```

## Answering a member's relayed question

A fleet member never talks to the user. When it needs a recorded user reaction (approve / choose / confirm / continue-or-abort), it relays the question to the Director via `cafleet message send`, and the Director asks the user through {decision_surface}. The Director forwards the user's answer back to the member as an ordinary `cafleet message send` (which the member consumes on its next poll) — not a pane keystroke. The question-shape taxonomy is a backend delta — see your overlay (`coding-agent/<name>-overlay.md`). The canonical user-reaction rule is the `cafleet` skill § *Soliciting user reactions*.

## Member Exec

Director-only shell-dispatch primitive: keystrokes `! <command>` + `Enter` so the coding agent's `!` shortcut runs the command natively (all three backends honor it). The positional `COMMAND` is a single shell command (leading/trailing whitespace stripped; pipes / `&&` / `;` / `$(...)` / backticks not special-cased; empty or newline-containing commands exit 2). See [`reference/exec-routing.md`](exec-routing.md) for the full fallback protocol and [`cli-options.md`](../../../docs/spec/cli-options.md#member-exec) for validation.

```bash
cafleet member exec --fleet-id <fleet-id> --member-id <member-id> "git log -1 --oneline"
```

### Required follow-up: `cafleet member ping`

After every successful `cafleet member exec` (exit 0), the Director MUST immediately invoke `cafleet member ping` against the same member. `member exec` only stages the bang-command's stdout/stderr as context for the member's next turn — it does not advance the turn. The follow-up primitive is `cafleet member ping`, NOT `cafleet message poll` (poll polls the Director's own inbox; ping injects the keystroke into the member's pane).

Skip the ping only on non-zero `member exec` exit (the dispatch did not complete; the supervision tick is the safety net). For a series of execs on the same member, the ping follows each one, not only the last.

## Member Ping (manual inbox-poll nudge)

Keystrokes **`Esc` → `cafleet … message poll` → `Enter`** into a member's pane (the leading `Esc` dismisses any pending permission-approval prompt, so the trailing `Enter` cannot blindly confirm it) for re-poking a member that missed the broker's auto-fired inline preview. The action is fully fixed by the command — no operator-controlled body — so it sits in `permissions.allow` while `member exec` stays in `permissions.ask`. Keystroke mechanics: [`multiplexer-backends.md`](../../../docs/spec/multiplexer-backends.md#esc-safeguard).

```bash
cafleet member ping --fleet-id <fleet-id> --member-id <member-id>
```

## Cross-references

- For broadcast send/ack semantics, see [`reference/cli.md`](cli.md) § *Broadcast*.
- For the bash-via-Director fallback protocol, see [`reference/exec-routing.md`](exec-routing.md).
- For crash/disconnect/idle recovery flows including the Shutdown Protocol, see [`reference/recovery.md`](recovery.md).
- For `--full` / `--json` opt-back-in semantics, see [`reference/cli.md`](cli.md) § *Output flags*.
