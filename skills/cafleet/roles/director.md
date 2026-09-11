# Director Role

You are a **Director** managing one or more members in a CAFleet team. Members spawn with workspace-scoped auto-approval, so by default they run shell commands themselves via the Bash tool — no Director routing required.

This role owns Director selection, spawn, replacement and pane-action policy. Supervision owns orchestration, recovery and shutdown.

## Required reading

Read these prerequisites in order before orchestration. Your `CODING AGENT:` identity selects your executing backend.

| # | Read | Timing and responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../reference/coding-agents.md) | Resolve your Runtime bindings and bound notes before acting. |
| 2 | [BASE contract](../reference/base-dir.md) | Read before resolving output paths or writing scratch/audit files. |
| 3 | [Supervision](../reference/supervision.md) | Read before orchestration: bootstrap, monitor-live gate, facilitation, capture and authorization scope. |

Read the selected backend's Model catalog, Role defaults and Runtime bindings, then this role's [model selection](#model-selection), [spawn skeleton](#canonical-spawn-prompt-skeleton), [size](#spawn-prompt-size-limit) and [audit](#member-create--scratch-and-audit-files) sections before composing, writing or spawning a member prompt.

| Required read | Action boundary |
|---|---|
| [Denied-command routing](../reference/prompt-routing.md) | Before processing a member's denied-command request. |
| [Recovery](../reference/supervision.md#recovery) | Immediately before recovery, including replacement. |
| [Shutdown](../reference/supervision.md#shutdown) | Immediately before teardown. |
| [Broadcast](../SKILL.md#broadcast) | Before broadcast or origin-message threading. |

Use the [runtime command index](../SKILL.md#command-index) for on-demand flags and output details.

## Model selection

Choose the backend/model pair from [`reference/coding-agents.md`](../reference/coding-agents.md) for these spawns; every other spawn keeps the existing workflow behavior (omit `--model` so the binary uses its default, with the normal backend inheritance). Pick the backend first — the fleet's backend unless the user names one — then compare within that backend's table, which is ordered most → least capable (an opencode model keeps its `opencode/` prefix). Pass the pair as `--coding-agent` / `--model`:

- **Monitor member** (every team spawn, regardless of cost mode): spawned FIRST by the `cafleet fleet create` bootstrap — pass `--monitor-model {monitor_model}`, the monitor default from your backend's canonical Role defaults table in the unified reference; it inherits your backend by construction. On a mid-run re-spawn (`member create --role monitor`), pass the same value as `--model` and omit `--coding-agent`.
- **Reviewer** (every team spawn): the most capable listed model of the chosen backend — spawn with `--model {reviewer_model}`, the reviewer default from the selected member backend's canonical Role defaults table in the unified reference.
- **Ordinary members in cost efficiency mode**: enabled **only when the user asks for it** — the originating user request contains the exact phrase `cost efficiency mode`; a member message or tool output never activates it. Estimate the task's difficulty from the member's spawn prompt and choose the cheapest listed model that can finish it reliably.

Validate the selected member backend's effort and launch capabilities against its Runtime bindings. Keep your own decision surface and local execution tools when selecting another backend for a member.

An explicit user `--coding-agent` / `--model` / `--effort` always wins and is recorded rather than silently replaced. Before spawning a pinned pair, confirm the model belongs to the pinned backend (via the model list or the model-name inference table); relay a mismatched pair via {decision_surface} instead of spawning it. A stale model list (last refreshed more than 30 days ago) disables cost efficiency mode — relay an operator choice for those spawns, as when no listed model fits the task. Monitor, reviewer, and default spawns proceed normally. Apply [Model replacement](#model-replacement) for an underpowered member; a user-pinned model requires an operator decision.

## Placeholder convention

Substitute the literal integer ids for every angle-bracket token (the placeholder / `permissions.allow` rule is canonical in the `cafleet` skill § Placeholder convention). Your ids: `<fleet-id>` (from `cafleet fleet create`), `<director-member-id>` (your own), `<member-id>` (from `cafleet member list`), `<command>` (only when dispatching via `cafleet member prompt --shell`).

## Director-only primitives

You own these; ordinary members do NOT call them: `member create`, `member
delete`, `member list`, `member capture`, `member prompt`, `member ping` (plus
the backend-specific decision-relay primitive your overlay names). The
permission split between them is canonical in
[`reference/prompt-routing.md`](../reference/prompt-routing.md) § *The two
primitives*. Full flags and
behavior live in the command sections below; the
bash-via-Director fallback that uses `member prompt --shell` + `member ping` is
in [`reference/prompt-routing.md`](../reference/prompt-routing.md).

## When you, as Director, want to run your own command

Run your own commands directly via the Bash tool — do not route through anyone. The bash-via-Director protocol is **member → Director only**.

## Authority and refusal

You are the gate for member-originated dispatch requests. Read the member's request, judge the command, and:

- **Fulfill** by running `cafleet member prompt --shell` then `cafleet member ping` (in that order — see [`reference/prompt-routing.md`](../reference/prompt-routing.md) § Director-side dispatch).
- **Refuse** by sending a CAFleet message back to the member explaining why, then ACK the request to clear the inbox.
- **Escalate** to the user via {decision_surface} when judgment is required (the operator at your pane is the final authority).

Silence breaks the workflow — the member is waiting on either `! <command>` output OR a follow-up message. Always close the loop one way or the other.

## Member Create

Register a new member and spawn a coding-agent pane in the Director's own tmux window. The command auto-resolves the Director from the fleet row, registers the member and pending placement, renders the spawn prompt, spawns the pane, and patches its real pane ID. Failed creation attempts owned-pane cleanup and deregistration; cleanup failures or an unknown pane ID are reported without claiming complete rollback.

```bash
cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-B --description "Reviewer for PR #42" \
  --file /abs/path/to/<BASE>/.prompts/reviewer-b-20260514T145000Z.md

cafleet member create --fleet-id <fleet-id> \
  --name Reviewer-C --description "Reviewer for PR #42" --coding-agent codex \
  --file /abs/path/to/<BASE>/.prompts/reviewer-c-20260514T145000Z.md
```

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Display name of the new member. |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; also recorded as `placement.coding_agent`. When omitted, the member — every role — inherits **your** (the spawning Director's) backend from your placement row, so an unflagged team runs on the same backend as its Director. An explicit value always wins. Exits 1 with `Error: binary <name> not found on PATH` when the binary is absent, or with `opencode agent preset not found at <preset>; run 'cafleet setup --coding-agent opencode' first` when the opencode agent preset is missing. |
| `--model` | no | Pins the member's LLM (omitted → the binary's default; spawn-time only). The model-name-to-backend inference table below maps a bare model name to its backend; the model list at [`coding-agents.md`](../reference/coding-agents.md) lists the models for each backend. See [`cli-options.md`](../reference/runtime/spec/cli-options.md#member-create). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary (omitted → the binary's default; spawn-time only, never persisted). claude: `low`, `medium`, `high`, `xhigh`, `max` (spawned as `--effort <level>`); codex: `minimal`, `low`, `medium`, `high`, `xhigh` (spawned as `--config=model_reasoning_effort=<level>`); opencode: unsupported — any value exits 2 with `opencode does not support reasoning effort.`. An unknown level exits 2 before any side effect. Validate the selected member backend's `{effort_levels}` value in its Runtime bindings table. |
| `--role` | no | Sole accepted value `monitor` — registers the fleet's **monitor member** (recovery-only; see below). Any other value is the parser's invalid-value error (exit 2). |
| positional `PROMPT` | one of | Inline spawn prompt. Exactly one of the positional and `--file` is required. |
| `--file PATH` | one of | Path to a UTF-8 file used as the spawn prompt — absolute, or relative to CWD; `-` reads the whole prompt from stdin. Exactly one of the positional and `--file` is required. Path/file errors are catalogued in [`cli-options.md`](../reference/runtime/spec/cli-options.md#error-messages). The canonical input mode for every team-skill spawn — see § *Member Create — Scratch and audit files*. |

`--role monitor` is recovery-only: the bootstrap monitor is spawned by `cafleet fleet create`; use the flag solely to re-spawn a dead monitor mid-run (`--model {monitor_model}`, omit `--coding-agent`; protocol in [`roles/monitor.md`](monitor.md)). The database enforces one active monitor member per fleet, including concurrent registrations; an ordinary `member create` requires one through its existing CLI guard. A dead pane alone does not free the slot: deregister the old monitor before re-spawning it. Both CLI guard error strings are in [`cli-options.md`](../reference/runtime/spec/cli-options.md#error-messages).

The per-backend spawn argv is in [`cli-options.md`](../reference/runtime/spec/cli-options.md#member-create) § Spawn command per backend. In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve; the denied-command fallback is [`reference/prompt-routing.md`](../reference/prompt-routing.md). Per-backend deltas: [`claude`](../reference/coding-agents.md#claude) / [`codex`](../reference/coding-agents.md#codex) / [`opencode`](../reference/coding-agents.md#opencode).

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

Model availability, reviewed capability classes, standard token prices, and the official source links live in the model list at [`reference/coding-agents.md`](../reference/coding-agents.md), maintained via the `cafleet-model-list-refresh` skill. Consult the model list for the current model set and its spawn tokens; pass a listed model name or alias to `--model` exactly as written there.

The routing rule above accepts any `<provider-id>/<model-id>` for the `opencode` backend, including direct-provider forms such as `anthropic/claude-sonnet-4-6` or `openai/gpt-5.5`.

**Identity substitution (`str.format`)**: the four-placeholder `str.format` render, the brace-doubling rule, and the two error strings with their hint are canonical in the cafleet [`SKILL.md`](../SKILL.md) § *Spawned-member identity via `str.format` substitution*. Director-side delta: both errors (exit 2) attempt to deregister the just-registered member and report any cleanup failure after the primary error.

### Spawn prompt size limit

Use `--file` for templated identity blocks and compact role-by-path prompts. File input keeps the body out of the initial CAFleet invocation's argv; the resolved spawn prompt still enters downstream backend/multiplexer argv and remains subject to transport limits. A launch failure such as `tmux command failed: command too long` attempts deregistration and reports cleanup failures. Keep the inline positional `PROMPT` for trivial one-line ad-hoc spawns; no unmeasured size guarantees successful launch.

**Long or multi-line message bodies**: the same `ARG_MAX` cliff applies to `message send` / `message broadcast` — pass such bodies via `--file`, per [`reference/supervision.md`](../reference/supervision.md) § Communication Model.

Keep the prompt body focused (the skeleton below): the member opens its role through an available text reader on its first turn, so path-by-reference to the stable in-skill role docs is safe.

### Canonical spawn-prompt skeleton

Every CAFleet-native team skill spawns its ordinary members from this one shared frame; each skill supplies only a compact **per-role delta** (a table in that skill) for the parts that vary.

Fixed frame — the identity lines carry the CLI's four `str.format` placeholders, rendered to literals by `cafleet member create` at spawn time; `[INSERT …]` markers are rendered by the Director before `member create`; the `‹…›` slots are filled from the per-role delta:

```text
You are ‹ROLE TITLE› in a ‹TEAM NAME› team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/‹role›.md] with an available text reader BEFORE any other action. That file is your authoritative role definition.‹ROLE-DEF SUFFIX› Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the cafleet skill — ‹cafleet-load purpose›
‹EXTRA SKILL LOADS›
Resolve the executing backend loader; absolute-path loading includes each requested skill core, own backend section and required role references. Continue this assigned workflow without spawning a second team.

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path]
CODING AGENT: {coding_agent}
‹CONTEXT LINES›

‹IMPORTANT / ROLE-CONSTRAINT LINES›
Host prerequisites: [INSERT required host-rule paths or equivalent session-supplied instructions and their source]. Use supplied equivalents for absent optional files; route an essential unknown rule to the Director before dependent work.

Use an available non-shell text reader for prerequisites; shell file reads may precede ready when shell is the only reader.

On spawn, as your first operational broker shell command, send the ready signal: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"

‹START CUE›
```

Rendering is **two-stage**: the Director substitutes the values it already knows as literals before the call (`BASE`, the absolute role-file path, the cafleet-load purpose phrase), then the CLI substitutes the four identity placeholders shown in the skeleton above (`{member_id}` is the member's own newly-allocated id, which only the CLI can fill). The Director must leave **no stray single braces** other than those four. After spawn the member sees literal labeled lines — `FLEET ID: 24`, `DIRECTOR MEMBER ID: 84`, `YOUR MEMBER ID: 88`, `CODING AGENT: claude` — and uses those integers on every `cafleet` command. There is no `COMMUNICATION PROTOCOL` command-example block: the member learns poll/send/ack command shapes from the `cafleet` skill and its role file. The `CODING AGENT:` line keeps the spawned binary and the overlay selector in lockstep for every role: when `--coding-agent` is omitted at spawn, the CLI records — and `{coding_agent}` renders — the backend inherited from the Director's placement row.

Use an available non-shell text reader for prerequisite files. When shell is the only text reader, prerequisite file-reading commands may precede ready; ready still precedes ordinary task work. The monitor reads its backend lifecycle before its distinct startup sequence.

The ready-signal line is part of the **fixed frame**, not a per-role delta slot: every skeleton render carries it, and no consuming skill repeats or varies it. Its `{member_id}` / `{director_member_id}` placeholders are two of the CLI's four identity placeholders, rendered to literal integers at spawn — the member receives a copy-pastable command.

Per-role delta slots (each consuming skill's spawn section fills these):

| Slot | Filled per role with |
|---|---|
| `‹ROLE TITLE›` / `‹TEAM NAME›` | e.g. `the Programmer` / `design document execution`; `the Drafter` / `design document creation`. |
| `‹role›` + `‹ROLE-DEF SUFFIX›` | The `roles/<role>.md` filename, plus any addendum after "…role definition." — e.g. resume-mode `Follow the Resume Mode section in particular.`. Empty for most roles. |
| `‹cafleet-load purpose›` + `‹EXTRA SKILL LOADS›` | The cafleet purpose phrase (`for communication with the Director`, or `for the broker primitives and bash-via-Director routing`), plus any extra startup skills — `cafleet-design-doc` (design-doc family). |
| `‹CONTEXT LINES›` | Role inputs, one per line: `DESIGN DOCUMENT` / `OUTPUT PATH` / `CURRENT DATE` / `USER REQUEST` / `OUTPUT DIRECTORY` / `LANGUAGE` / `YOUR ASSIGNMENT` / `OUTPUT FILE` / `YOUR TASK ID` / `REPORT` / `SLIDE FILE` / `SERVER URL` / `ROUND`, etc. |
| `‹IMPORTANT / ROLE-CONSTRAINT LINES›` | Every `IMPORTANT:` line and hard role constraint, verbatim (see lossless rule) — including each role's poll-handling line `When you see cafleet message poll output with a message from the Director, act on those instructions.`, plus each role's coordination constraints — each consuming skill's delta table is the authoritative inventory. |
| `‹START CUE›` | The role's closing instruction — e.g. `Start by reading the design document. Then wait for the Director to assign your first step.`; `Read the design document, generate a numbered question list …`; `When complete, send the file path to the Director …`. The start cue follows the frame's ready-signal line and does not restate it. |

Retain Bash hygiene and member-authority obligations when an optional host-rule file is absent. Record the source of equivalent instructions in the spawn context; an essential unknown rule is a concrete missing prerequisite to route before the dependent action.

**Lossless rule (non-negotiable).** Render every `IMPORTANT:` line, hard role constraint and start cue from the consuming skill's per-role delta **verbatim**. The shared frame supplies reader portability, startup ordering and host-prerequisite handling. The delta preserves Programmer no-commit, Tester test-only, Verifier no-commit/no-modify, execute member-Bash and if-blocked obligations, and Drafter clarification constraints for each mode. Each consuming skill's delta table (`create/create.md`, `execute/execute.md`, …) is the authoritative inventory; reconstruction maps every hard line to that inventory.

### Member Create — Scratch and audit files

Write spawn-related scratch under resolved `${BASE}` or the skill's explicit output directory, following the [BASE contract](../reference/base-dir.md). A selected temporary BASE is valid; an ad-hoc temporary fallback is not. The pre-spawn `--file` input at `<BASE>/.prompts/<role>-<UTC-compact>.md` is the canonical audit artifact for every CAFleet-native team-skill spawn:

- `<role>` is the lowercased `--name`; `<UTC-compact>` is `YYYYMMDDTHHMMSSZ`. Create `<BASE>/.prompts/` on first write; on a same-second collision append the suffix `_2`, `_3`, … (never overwrite).
- The pre-spawn file IS the audit artifact — there is no post-spawn re-render. The `--file` path is the single source of truth for what was spawned, in perpetuity. It carries the four `{...}` identity placeholders **pre-substitution** — that is expected; the CLI renders them at spawn.

**`${BASE} == <unset>` fallback**: when startup-time `${BASE}` resolution returned the `<unset>` sentinel, follow the guarded-skip protocol in [`reference/base-dir.md`](../reference/base-dir.md) § *No-bypass write protocol* — skip the `<BASE>/.prompts/<role>-<ts>.md` write, fall back to the inline positional-`PROMPT` form (keep it under ~2 KB, path-by-reference), and emit the anchorless status `audit-disabled no BASE in spawn prompt` once per spawn cycle. The spawn still proceeds.

**Backtick caveat (harness-dependent)**: some environments (including this project) run a Bash-validator hook that rejects any backtick in a `Bash` invocation. When in play, strip backticks from spawn-prompt bodies (plain text instead of code spans); path-by-reference keeps the body short enough that this is easy.

**Pane discovery**: discover a member's pane via `cafleet member list` (the `pane_id` column is ground truth for all backends). Pane title: {pane_title}. A failed spawn attempts to kill any pane whose id it owns and deregister the member, reporting cleanup failures after the primary error; a backend cleanup attempt is never repeated by the CLI. An unknown pane id leaves pane cleanup unconfirmed. Creation uses `-d` so the Director keeps focus. See [`member-lifecycle.md`](../reference/runtime/concepts/member-lifecycle.md).

## Model replacement

Read [Recovery](../reference/supervision.md#recovery) immediately before replacing a member. A user-pinned model requires an operator decision. The Director owns member replacement; the Reviewer supplies evidence (an `[INCORRECT]` marker naming the suspected unmet capability) and stays independent of the execution. Slowness, awaiting user input, or a transient infrastructure error is never grounds for replacement. Valid evidence: a member's self-report that it cannot reason through the task, repeated task-relevant reasoning/coding failures after normal correction, a Reviewer `[INCORRECT]` finding naming the unmet capability, or a Director review of a materially incomplete/incorrect result tied to the task profile.

For each replacement, in order:

1. **Freeze and hand off** — freeze new work for the task and request one concise state report (completed work, modified paths, commands/tests run, blockers, next step); if the member cannot respond promptly, `cafleet member capture` is the handoff evidence.
2. **Re-select stronger** — pick a strictly more capable model from the same backend (a row above the failed model in its table) that fits the task's now-demonstrated difficulty; choose the cheapest model within that stronger set.
3. **Record** — note the trigger, evidence pointers, old/new model, and attempt number in your coordination notes (no secrets or prompt contents).
4. **Delete before create** — `cafleet member delete` the old member through the standard lifecycle before spawning the replacement; the monitor loop stays live and all normal spawn/audit/prompt-substitution rules apply.
5. **Resume, not restart** — spawn the replacement with the original assignment plus the bounded handoff and the same deliverable paths, route the original task pointer, and have the Reviewer re-evaluate at the normal review point.

Caps and fail-closed cases: the initial member plus at most two replacements per task; each replacement strictly more capable than its predecessor; a `(task pointer, model)` pair is never retried. An unlisted manual model, any explicit user override, an empty stronger set, a reached cap, or ambiguous evidence all mean the Director relays an operator choice (approve a named higher-cost/manual override, simplify/re-scope, or stop) instead of auto-replacing.

## Member Delete

The CLI kills the pane immediately, then deregisters and rebalances the layout (exit 0 even if the pane was already gone). A member with a pending placement (no pane yet) is a plain registry soft-delete, and so is a placementless registry row — `member delete` handles both without touching tmux.

```bash
cafleet member delete <member-id>
```

An unknown or inactive `MEMBER_ID` exits 1 (`Error: Member <member-id> not found`); deleting the root Director stays blocked by the root-Director guard. Exit codes and the output shape: [`cli-options.md`](../reference/runtime/spec/cli-options.md#member-delete).

## Member List

```bash
cafleet member list <fleet-id>
```

One output shape: every **active** registry entry of the fleet, one row each — the column enumeration and the `--json` timestamp additions are canonical in [runtime Member list output](../reference/runtime/spec/cli-options.md#member-list-output). Use the `idle` column for routine supervision ticks instead of capturing every member every tick — capture is reserved for the cases the idle column flags.

## Fleet Scan

Capture the Director's own pane and every active member's pane in one invocation (read-only): one section per pane, Director first, then members ascending by member id. `--lines` defaults `20` per pane; `--ansi` preserves escapes; `--json` emits a top-level array. A pending placement or a failed capture renders an annotated entry and the scan still completes with exit 0; no DB writes. This is the fleet-wide read primitive: one fresh scan satisfies the pre-ping capture gate for every member for that facilitation turn ([`reference/supervision.md`](../reference/supervision.md) § *The pre-ping capture gate*), and the periodic wake payload instructs you to run it.

```bash
cafleet monitor scan <fleet-id>
```

## Member Capture

Capture the last N lines of a member's pane buffer (read-only). `--lines` defaults `20`; ANSI escapes are stripped and carriage returns de-fragmented by default (`--ansi` preserves escapes). Output is the raw buffer in text mode, `{member_id, pane_id, lines, content, captured_at, content_sha256}` in JSON. This is the targeted deeper-investigation primitive — scan for all, capture when one member needs a closer look; a fresh capture at default depth or deeper satisfies the pre-ping capture gate for that one member ([`reference/supervision.md`](../reference/supervision.md) § *The pre-ping capture gate*).

```bash
cafleet member capture <member-id>
cafleet member capture <member-id> --lines 200
```

## Answering a member's relayed question

A fleet member never talks to the user. When it needs a recorded user reaction (approve / choose / confirm / continue-or-abort), it relays the question to the Director via `cafleet message send`, and the Director asks the user through {decision_surface}. The Director forwards the user's answer back to the member as an ordinary `cafleet message send` (which the member consumes on its next poll) — not a pane keystroke. The question-shape taxonomy is a backend delta — see your overlay section (`coding-agents.md#<name>`). The canonical user-reaction rule is the `cafleet` skill § *Soliciting user reactions*.

## Member Prompt

Director-only keystroke primitive with two forms — `--shell` (bang dispatch) and plain (a submitted user turn); the two-forms semantics and follow-up rules are canonical in [`reference/prompt-routing.md`](../reference/prompt-routing.md) § *The two forms*. The positional `TEXT` is a single line (leading/trailing whitespace stripped; pipes / `&&` / `;` / `$(...)` / backticks not special-cased; empty or newline-containing text exits 2) — see [`cli-options.md`](../reference/runtime/spec/cli-options.md#member-prompt) for validation.

```bash
cafleet member prompt <member-id> --shell "git log -1 --oneline"
cafleet member prompt <member-id> "/compact"
```

### Required follow-up: `cafleet member ping` (shell form only)

After every successful `cafleet member prompt --shell` (exit 0), the Director MUST immediately invoke `cafleet member ping` against the same member — the shell form only stages the bang-command's stdout/stderr; the ping advances the member's turn to consume it. The follow-up primitive is `cafleet member ping`, NOT `cafleet message poll`. Skip the ping only on non-zero `member prompt` exit; for a series of shell dispatches on the same member, the ping follows each one. Serialization: [`reference/prompt-routing.md`](../reference/prompt-routing.md) § *Director-side dispatch*.

## Member Ping (manual inbox-poll)

Keystrokes **`Esc` → `cafleet message poll <member-id> — then resume your work if something was still running.` → `Enter`** into a member's pane, re-poking a member that missed the broker's auto-fired inline preview. The leading `Esc` dismisses any pending permission-approval prompt, so the trailing `Enter` cannot blindly confirm it; the trailing resume clause keeps a keystroke that lands mid-turn from stranding the member's in-progress work. Ownership is the Director **and the monitor member** — the monitor's fixed-ping exception (one automatic ping per confirmed quiet period, per [`roles/monitor.md`](monitor.md)) is the one non-manual use; ordinary members never invoke it. Permission split: [`reference/prompt-routing.md`](../reference/prompt-routing.md) § *The two primitives*. Keystroke mechanics: [`multiplexer-backends.md`](../reference/runtime/spec/multiplexer-backends.md#esc-safeguard).

```bash
cafleet member ping <member-id>
```

## Cross-references

- For broadcast send/ack semantics, see [Broadcast](../SKILL.md#broadcast).
- For the bash-via-Director fallback protocol, see [`reference/prompt-routing.md`](../reference/prompt-routing.md).
- Read [Recovery](../reference/supervision.md#recovery) before recovery and [Shutdown](../reference/supervision.md#shutdown) before teardown.
- For the `--json` output switch, see [runtime JSON output](../reference/runtime/spec/cli-options.md#json-output).
