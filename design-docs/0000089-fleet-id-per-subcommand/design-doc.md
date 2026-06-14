# Per-Subcommand `--fleet-id` (Move the Global Flag onto Child Subcommands)

**Status**: Approved
**Progress**: 35/37 tasks complete
**Last Updated**: 2026-06-14

## Overview

Move `--fleet-id` off the top-level `cafleet` click group and onto each child subcommand that needs it, so users write `cafleet agent register --fleet-id <id> ...` instead of `cafleet --fleet-id <id> agent register ...`. This aligns `--fleet-id` with the already-per-subcommand `--agent-id` and removes the "flag must come before the subcommand" confusion. `--json` and `--version` stay global; the single-`permissions.allow`-pattern guarantee that motivated the original global placement is knowingly traded away.

## Success Criteria

- [x] `--fleet-id` is a per-subcommand option on every client/member/monitor leaf command, accepted **after** the subcommand name (e.g. `cafleet message poll --fleet-id 56 --agent-id 234`).
- [x] The top-level `cafleet` group no longer defines `--fleet-id`; `cafleet --fleet-id <id> <subcmd>` (old global position) exits 2 with Click's `No such option: --fleet-id`.
- [x] `--json` and `--version` remain global (before the subcommand) and behave exactly as today.
- [x] `db init`, `fleet *`, `server`, and `doctor` do **not** accept `--fleet-id` at all (Click rejects it); the previous "silently accepted and ignored" behavior is gone.
- [x] Omitting `--fleet-id` on a subcommand that needs it prints the existing custom message (`--fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.`) and exits 1.
- [x] The design doc specifies a recommended per-subcommand `permissions.allow` pattern set; the user applies the global `~/.claude/settings.json` change manually (no committed repo `.claude/settings.json` permissions block is added).
- [x] Every repo-committable `cafleet --fleet-id <id> <subcmd>` command example (docs, all skills, README, `.claude/rules/`, `CLAUDE.md`) is rewritten to the new shape, and every prose claim that `--fleet-id` is a global / before-the-subcommand flag is rewritten. After this lands the repo reads as if `--fleet-id` was always per-subcommand (no deprecation notices), per `.claude/rules/removal.md`.
- [x] `mise //cafleet:format`, `lint`, `typecheck`, and `test` are all green.

---

## Background

**Current state.** The `cafleet` click group (`cafleet/src/cafleet/cli/__init__.py`) defines three global options — `--json`, `--fleet-id`, and `--version` — and stashes `fleet_id` / `json_output` into `ctx.obj`. Client subcommands read `ctx.obj["fleet_id"]`; the `client_command` decorator and the standalone `require_fleet_id(ctx)` guard (both in `cli/_helpers.py`) centralize the "fleet id is required" check. `--agent-id`, by contrast, is already a per-subcommand option declared on each leaf command.

**Why the global placement was deliberate (the regression this change accepts).** `docs/spec/cli-options.md` documents the rationale in two places — the Option Source Matrix and the "Why `--fleet-id` is a literal CLI flag" callout. A literal `cafleet --fleet-id <int> ...` invocation, with the flag immediately after `cafleet`, matches a **single** `permissions.allow` pattern of the shape `cafleet --fleet-id <id> *` across **every** subcommand. To make that single pattern total, the flag is **silently accepted and ignored** on subcommands that do not need it (`db init`, `fleet *`, `server`, `doctor`). Moving the flag onto children breaks the single pattern: each subcommand needs its own `permissions.allow` entry, and — because Click accepts options in any order — permission coverage now depends on a documented canonical flag order. **The user has explicitly accepted this tradeoff** in exchange for the ergonomic win.

**Problem being solved.** Requiring `--fleet-id` *before* the subcommand confuses users, who naturally type the subcommand first. `--agent-id` is already per-subcommand, so per-subcommand `--fleet-id` is the consistent shape.

**Flag-day nature.** This is a breaking change to a *live* orchestration system. The moment the new CLI is installed, the old global form stops parsing, so the code change and every skill/doc example must land together — otherwise agents spawned from stale skill templates emit `cafleet --fleet-id … <subcmd>` commands that now fail. This is why the documentation cleanup (Step 1) is as load-bearing as the code (Step 2), not cosmetic.

---

## Specification

### Decisions (confirmed with the user)

| # | Question | Decision |
|---|---|---|
| Q1 | Does `--json` move per-subcommand too? | **No** — `--json` stays global (before the subcommand). Only `--fleet-id` moves. `--json` is an optional, rarely-typed output modifier that is not the source of the ergonomic complaint; re-declaring it on ~25 subcommands is churn. |
| Q2 | Do `db init` / `fleet *` / `server` / `doctor` still accept `--fleet-id`? | **No** — they reject it (`No such option`). The only reason for silent-accept was the single shared pattern, now gone. Cleanest per `removal.md`. |
| Q3 | Missing-`--fleet-id` error style | **Keep the custom helpful message** (`--fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.`, exit 1) via a shared `fleet_id_option` decorator + guard. A deliberate asymmetry vs `--agent-id` (which uses Click's built-in `required=True`). |
| Q4 | `permissions.allow` strategy + target file | Doc **specifies** a recommended per-subcommand pattern set and updates all repo-committable examples; the user applies the global `~/.claude/settings.json` change manually. Do **not** add a committed repo `.claude/settings.json` permissions block. |
| Q5 | Canonical flag position | `--fleet-id` immediately **after** the leaf subcommand name and before other flags, e.g. `cafleet message poll --fleet-id 56 --agent-id 234`. Used consistently across all docs/skills so the per-subcommand `permissions.allow` patterns match. |

### CLI surface changes

#### Top-level group (`cli/__init__.py`)

Remove the `--fleet-id` option and the `fleet_id` parameter from `cli()`. Keep `--json` and `--version`. `ctx.obj` is still created by the group and still carries `json_output`; it no longer carries `fleet_id` from the group (the per-subcommand option sets it — see below).

```python
@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
@click.option("--json", "json_output", is_flag=True, default=False, help="Output in JSON format")
@click.pass_context
def cli(ctx, json_output):
    """CAFleet — CLI for the message broker and agent registry."""
    ctx.ensure_object(dict)
    ctx.obj["json_output"] = json_output
```

#### Shared `fleet_id_option` decorator (`cli/_helpers.py`)

A single reusable decorator applied to each subcommand that needs a fleet id. It uses a Click callback with `expose_value=False`, so it (a) enforces the requirement with the **custom** message at parse time, and (b) writes the value into `ctx.obj["fleet_id"]` — meaning `client_command` and every handler keep reading `ctx.obj["fleet_id"]` **unchanged**, and no handler signature gains a `fleet_id` parameter.

```python
def _fleet_id_callback(ctx: click.Context, param: click.Parameter, value: int | None) -> int:
    if value is None:
        raise click.ClickException(
            "--fleet-id <int> is required for this subcommand. "
            "Create a fleet with 'cafleet fleet create' and pass its id."
        )
    ctx.ensure_object(dict)
    ctx.obj["fleet_id"] = value
    return value


fleet_id_option = click.option(
    "--fleet-id",
    "fleet_id",
    type=int,
    default=None,
    callback=_fleet_id_callback,
    expose_value=False,
    help="Fleet ID (integer); required for this subcommand.",
)
```

Notes:
- `type=int` preserves today's `Invalid value for '--fleet-id': '<x>' is not a valid integer.` (exit 2) for non-integers.
- A missing flag raises `ClickException` → **exit 1** with the custom message (same exit code and wording as today's `require_fleet_id`).
- `require_fleet_id(ctx)` is **deleted**: the callback subsumes it. Remove the `require_fleet_id(ctx)` call from the `client_command` wrapper (`cli/_helpers.py`), and remove the `require_fleet_id` import + call sites in `member.py` and `monitor.py` (keep each handler's `fleet_id = ctx.obj["fleet_id"]`).
- **Decorator stacking order**: place `@fleet_id_option` in the same position as the existing `@click.option("--agent-id", ...)` on each leaf — among the option decorators, above `@click.pass_context`, and (for the `agent`/`message` commands) above the `@client_command(...)` wrapper. `client_command` uses `functools.wraps`, so Click still collects `__click_params__` through it; mirroring the proven `--agent-id` placement avoids any parameter-registration pitfall.

#### Which subcommands get `@fleet_id_option`

| File | Subcommands gaining `@fleet_id_option` | How fleet id is consumed today |
|---|---|---|
| `cli/agent.py` | `register`, `list`, `show`, `deregister` | `client_command` reads `ctx.obj["fleet_id"]` |
| `cli/message.py` | `send`, `broadcast`, `poll`, `ack`, `cancel`, `show` | `client_command` reads `ctx.obj["fleet_id"]` |
| `cli/member.py` | `create`, `delete`, `list`, `capture`, `send-input`, `exec`, `ping` | handler reads `ctx.obj["fleet_id"]` after `require_fleet_id(ctx)` |
| `cli/monitor.py` | `start`, `status`, `config` | handler reads `ctx.obj["fleet_id"]` after `require_fleet_id(ctx)` |

20 leaf subcommands total (agent 4 + message 6 + member 7 + monitor 3). The 19-line `permissions.allow` set below is 20 minus the deliberately excluded `member exec`.

#### Which subcommands DROP `--fleet-id` (Q2)

`db init`, `fleet create`/`list`/`show`/`delete`, `server`, and `doctor` get **no** `@fleet_id_option`. Because the group no longer defines `--fleet-id` and these commands don't either, both `cafleet --fleet-id <id> db init` (old global position) and `cafleet db init --fleet-id <id>` (per-subcommand position) exit 2 with `No such option`. These commands already only read `ctx.obj["json_output"]` (or nothing — `db init` and `server` take no context), so no body changes are required.

#### Behavioral note — error precedence on `member create`

`member create` currently checks `--prompt-file`/positional mutual-exclusion **before** `require_fleet_id`. With the callback, a missing `--fleet-id` is now reported at parse time, i.e. **before** the prompt-file conflict. Any test that exercised the prompt-file conflict without passing `--fleet-id` must now also pass `--fleet-id`. The body's own `require_fleet_id(ctx)` line is removed.

### `permissions.allow` strategy (Q4)

The old single pattern `cafleet --fleet-id <id> *` is replaced by **one pattern per allow-listed subcommand**, all matching the canonical flag order (`--fleet-id` first after the subcommand name). The user applies these to their global `~/.claude/settings.json` manually; **no committed repo `.claude/settings.json` permissions block is added**.

Recommended pattern set:

```
Bash(cafleet agent register --fleet-id *)
Bash(cafleet agent list --fleet-id *)
Bash(cafleet agent show --fleet-id *)
Bash(cafleet agent deregister --fleet-id *)
Bash(cafleet message send --fleet-id *)
Bash(cafleet message broadcast --fleet-id *)
Bash(cafleet message poll --fleet-id *)
Bash(cafleet message ack --fleet-id *)
Bash(cafleet message cancel --fleet-id *)
Bash(cafleet message show --fleet-id *)
Bash(cafleet member create --fleet-id *)
Bash(cafleet member delete --fleet-id *)
Bash(cafleet member list --fleet-id *)
Bash(cafleet member capture --fleet-id *)
Bash(cafleet member send-input --fleet-id *)
Bash(cafleet member ping --fleet-id *)
Bash(cafleet monitor start --fleet-id *)
Bash(cafleet monitor status --fleet-id *)
Bash(cafleet monitor config --fleet-id *)
```

Three consequences worth recording:

- **`member exec` is deliberately omitted** so it stays under `permissions.ask` (operator-controlled command body), exactly as today. Under the old single-pattern scheme this required a *carve-out* (a separate `ask`/`deny` for `cafleet --fleet-id * member exec *`); under the per-subcommand scheme the split is cleaner — `member exec` is simply not in the allow set. This is the one ergonomic *upside* of the regression.
- **Coverage depends on canonical flag order.** `cafleet message poll --agent-id 234 --fleet-id 56` (fleet-id last) does **not** match `Bash(cafleet message poll --fleet-id *)` and would prompt. This is the concrete form of the "flag position becomes variable" regression; the mitigation is the documented canonical order (Q5) and every skill/doc example using it.
- **`--json` invocations need companion patterns.** Because `--json` stays global (Q1) it sits *before* the subcommand, so `cafleet --json message poll --fleet-id 56 ...` does **not** match `Bash(cafleet message poll --fleet-id *)` — the leading `--json` breaks the prefix. This is the same regression facet (under the old single pattern, `--json` came *after* `--fleet-id` and was covered). The canonical JSON shape therefore changes: today's `cafleet --fleet-id <id> --json agent register` becomes `cafleet --json agent register --fleet-id <id>`. For every subcommand an agent scripts with `--json` — the skill recipes use `cafleet --json agent register --fleet-id <id> ...`, `cafleet --json agent list --fleet-id <id>`, and `cafleet --json message poll --fleet-id <id> --agent-id <id>` — add a companion pattern of the shape `Bash(cafleet --json <grp> <cmd> --fleet-id *)`:

```
Bash(cafleet --json agent register --fleet-id *)
Bash(cafleet --json agent list --fleet-id *)
Bash(cafleet --json message poll --fleet-id *)
Bash(cafleet --json message send --fleet-id *)
Bash(cafleet --json message ack --fleet-id *)
Bash(cafleet --json member create --fleet-id *)
Bash(cafleet --json member list --fleet-id *)
```

  (Mint one companion per subcommand an agent actually runs with `--json`; the rest fall outside the allow set by design and will prompt.)

### The "Why" rewrite in `cli-options.md`

The existing callout conflates two claims. After this change:

- **Still true (keep):** `--fleet-id` is a literal CLI flag rather than an environment variable, because Claude Code's `permissions.allow` matches Bash invocations as literal command strings; an env-var/shell-expansion approach would break that matching and force per-invocation prompts.
- **No longer true (replace):** "a single `permissions.allow` pattern covers every subcommand" and "silently accepted/ignored where not needed". Replace with: the flag is positioned per-subcommand (immediately after the subcommand name), coverage uses one `permissions.allow` pattern per subcommand (reference the set above), the canonical flag order is required for matching, and subcommands that don't need a fleet id reject the flag.

### Documentation cleanup (total, per `removal.md`)

The change to flag position invalidates **every** `cafleet --fleet-id <id> <subcmd>` command example in the repo and **every** prose statement that `--fleet-id` is global / before the subcommand. The cleanup is repo-wide and must be total. Highest-value canonical targets, with the specific edits:

| Target | What changes |
|---|---|
| `docs/spec/cli-options.md` (30 hits) | Option Source Matrix (Fleet ID source → per-subcommand option); rewrite the "Why" callout (above); remove the `--fleet-id` row from **Global Options** and add a per-subcommand "Fleet ID (`--fleet-id`)" subsection mirroring the `--agent-id` one; Subcommand-summary note (the `--fleet-id` "no" rows now mean "rejected", not "silently ignored"); remove the doctor "silently accepted and ignored" row; rewrite the server "silently accepted and ignored" paragraph to "not accepted"; update the agent/message/member/monitor section intros ("require the **global** `--fleet-id`" → "require the per-subcommand `--fleet-id`") and **all** example commands; add a `permissions.allow` subsection with the recommended pattern set + `member exec` exclusion + canonical-order caveat. |
| `skills/cafleet/SKILL.md` (38 hits) | Required Flags table (`--fleet-id` scope: global → per-subcommand, placed after the subcommand); Global Options section (only `--json`/`--version` are global; show the new shape and that `cafleet --fleet-id … <subcmd>` now fails); the "Why literal flags, not env vars?" callout (same split as cli-options); all command examples. |
| `skills/cafleet/reference/*.md` | `director.md` (16), `recovery.md` (5), `exec-routing.md` (5), `broadcast.md` (2), `output-flags.md` (1) — rewrite every command example to the new shape. |
| `skills/cafleet/roles/*.md` | `director.md` (1), `member.md` (6) — command examples **and** the "flag placement" doctrine sentence (see the doctrine-class note below the table). |
| `.claude/rules/bash-tool.md` | Every `cafleet --fleet-id <s> message send …`, `member ping`, `member exec` example (member side and Director side) → new shape. |
| `skills/cafleet-design-doc-create/roles/drafter.md` | The **"Flag placement"** paragraph asserting "`--fleet-id` is a global flag (placed before the subcommand)" must be rewritten — it now directly contradicts the design — plus its command examples. |

**"Flag placement" doctrine sentence — an explicit rewrite class.** Beyond `drafter.md`, the same doctrine sentence (some variant of "`--fleet-id` is a global flag (placed before the subcommand)") lives in `skills/cafleet/roles/member.md` and `roles/director.md`, and in `skills/cafleet-design-doc-create/roles/reviewer.md` and `roles/director.md`. These are the highest-risk contradictions — the role-file analog of the cli-options "Why" callout — so rewrite the doctrine *sentence* in each of these files (not merely their command examples). The catch-all doctrine-prose grep below also catches them; naming the class here is belt-and-suspenders so totality does not rest solely on the grep.

Long-tail (catch-all) — same mechanical rule (`cafleet --fleet-id <id> <subcmd>` → `cafleet <subcmd> --fleet-id <id> …`; rewrite prose claiming global placement). Authoritative method: `grep -rn "cafleet --fleet-id" docs/ skills/ README.md .claude/ CLAUDE.md` and fix each hit, then `grep -rniE "global .*fleet-id|fleet-id.*global|before the subcommand|placed before" docs/ skills/ .claude/` for the doctrine prose.

- **Orchestration skills** (spawn-prompt templates + role-file command examples): `cafleet-design-doc-create` (SKILL + `roles/director.md`, `roles/reviewer.md`), `cafleet-design-doc-execute` (SKILL + roles), `cafleet-design-doc-interview` (SKILL + `roles/analyzer.md`), `cafleet-research-report` (SKILL + roles), `cafleet-research-presentation` (SKILL + roles), `cafleet-agent-team-monitoring/SKILL.md`, `cafleet-agent-team-supervision/SKILL.md`.
- **docs/**: `get-started/quickstart.md` (6), `get-started/configure.md` (5), `concepts/data-model.md` (2), `concepts/member-lifecycle.md` (1), `concepts/monitoring.md` (1), `how-to/mixed-backend-team.md` (7), `how-to/monitor-and-recover.md` (8), `reference/coding-agents/codex.md` (11), `reference/coding-agents/opencode.md` (11).
- **README.md** (1).

The canonical repo edit target is `skills/` (not the promoted `~/.claude/skills/` copies, which the promotion flow re-syncs after merge).

### Tests

The broker/webui/multiplexer tests pass `fleet_id` as a Python argument to broker functions — the broker API is unchanged, so **those tests do not change**. Only tests that invoke the CLI with `--fleet-id` on the command line change (flag moves from before the group to after the leaf subcommand).

| Test target | Change |
|---|---|
| `tests/cli/test_fleet_flag.py` | Rewrite. Keep the missing-fleet-id custom-message test (invoke without `--fleet-id` → exit 1, custom message). Move `--fleet-id` after the subcommand in the flows-into-broker tests. **Replace** the two `…silently_accepted…` tests with removal guards: `db init` / `fleet create` reject `--fleet-id` in **both** the old global position and the per-subcommand position (exit 2, `no such option`). Add `test_old_surface_removed__global_fleet_id_no_longer_parses` (`cafleet --fleet-id 100 agent list` → exit 2, `no such option`), modeled on the existing `test_old_surface_removed__session_flag_and_group_no_longer_parse`. |
| `tests/cli/test_agent.py`, `test_message.py`, `test_member*.py`, `test_monitor.py`, `test_fleet.py`, `test_compact_echo.py`, `test_message_truncation.py`, `test_fleet_bootstrap.py` | Move `--fleet-id <id>` from before the group to immediately after the leaf subcommand in every `runner.invoke(cli, [...])` arg list. |
| `tests/cli/test_doctor.py`, `test_server.py` | `doctor`/`server` reject `--fleet-id` (SC#4), so convert their silently-accepted invocations into both-position rejection guards (exit 2, `No such option`) rather than flag-moves. |
| `tests/cli/test_client_command.py` | If its in-test harness group declares a global `--fleet-id`, switch the harness to the per-subcommand `fleet_id_option` shape; otherwise no change. |
| `tests/cli/test_help_budget.py` | Bump the per-subcommand line budgets (+1 for every fleet-scoped subcommand — each gains one `--fleet-id` help line) and the aggregate byte budget (4480 → accommodate the measured ~5769). Do **not** shorten the spec-mandated `fleet_id_option` help string. |
| `tests/cli/test_member_ping.py`, `tests/multiplexer/test_tmux.py` | Update the `send_poll_trigger` keystroke-payload assertions to the new shape (`cafleet message poll --fleet-id <s> --agent-id <m>`), paired with the `multiplexer/tmux.py` source change. `test_member_prompt_template.py` is shape-agnostic — no change. |
| `tests/output/test_render_agent.py` | Move the one `--fleet-id` CLI invocation to the new shape (the Tests table's earlier "output no-change" assumption was wrong). |
| `tests/broker/**`, `tests/webui/**`, `tests/monitor/test_loop.py`, `tests/db/**` | No change — verify none invoke the CLI with a global `--fleet-id`. |

Per `removal.md`, the removal guards above test the **absence** of the old surface (Click's built-in `No such option`), which is allowed; no deprecation shims are added.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first (per `.claude/rules/design-doc-numbering.md`)

- [x] `docs/spec/cli-options.md`: Option Source Matrix — change Fleet ID source to a per-subcommand `--fleet-id <int>` option. <!-- completed: 2026-06-14T09:13 -->
- [x] `docs/spec/cli-options.md`: rewrite the "Why `--fleet-id` is a literal CLI flag" callout — keep the env-var-vs-flag rationale, replace the single-pattern/silently-ignored claims with the per-subcommand reality (one pattern per subcommand, canonical-order dependency, non-fleet subcommands reject it). <!-- completed: 2026-06-14T09:13 -->
- [x] `docs/spec/cli-options.md`: remove the `--fleet-id` row from **Global Options**; add a per-subcommand "Fleet ID (`--fleet-id`)" subsection mirroring `--agent-id`; update the Subcommand-summary note so the `--fleet-id` "no" rows read as "rejected" not "silently ignored". <!-- completed: 2026-06-14T09:13 -->
- [x] `docs/spec/cli-options.md`: doctor — remove the "`--fleet-id` silently accepted and ignored" row; server — rewrite the silently-accepted paragraph + the `cafleet --fleet-id 1 server` example to "not accepted". <!-- completed: 2026-06-14T09:13 -->
- [x] `docs/spec/cli-options.md`: update the agent/message/member/monitor section intros and **every** example command to the new flag position; add a `permissions.allow` subsection with the recommended per-subcommand pattern set, the `member exec` exclusion, the canonical-order caveat, and the `--json` companion-pattern note (global `--json` precedes the subcommand, so JSON invocations need `Bash(cafleet --json <grp> <cmd> --fleet-id *)` companions). <!-- completed: 2026-06-14T09:13 -->
- [x] `skills/cafleet/SKILL.md`: Required Flags table, Global Options section, the "Why literal flags, not env vars?" callout, and all command examples → new shape (only `--json`/`--version` global; `cafleet --fleet-id … <subcmd>` now fails). <!-- completed: 2026-06-14T09:18 -->
- [x] `skills/cafleet/reference/*.md` (`director.md`, `recovery.md`, `exec-routing.md`, `broadcast.md`, `output-flags.md`): rewrite every command example. <!-- completed: 2026-06-14T09:18 -->
- [x] `skills/cafleet/roles/director.md` + `roles/member.md`: rewrite command examples **and** the "flag placement" doctrine sentence (per the doctrine-class note in the Specification). <!-- completed: 2026-06-14T09:18 -->
- [x] `.claude/rules/bash-tool.md`: rewrite every `cafleet --fleet-id <s> …` example (member side `message send`; Director side `member ping` / `member exec`). <!-- completed: 2026-06-14T09:36 (Director-owned: harness denies member edits under .claude/) -->
- [x] `skills/cafleet-design-doc-create/roles/drafter.md`: rewrite the **"Flag placement"** doctrine paragraph (no longer global) and its examples; update `cafleet-design-doc-create` SKILL + `roles/director.md` + `roles/reviewer.md` examples and spawn-prompt templates — including the "flag placement" doctrine sentence in `roles/director.md` and `roles/reviewer.md` (not just their examples). <!-- completed: 2026-06-14T09:36 -->
- [x] Orchestration-skill catch-all: `grep -rn "cafleet --fleet-id" skills/` and rewrite every command example + spawn-prompt template across `cafleet-design-doc-execute`, `cafleet-design-doc-interview`, `cafleet-research-report`, `cafleet-research-presentation`, `cafleet-agent-team-monitoring`, `cafleet-agent-team-supervision` to the new shape. <!-- completed: 2026-06-14T09:36 -->
- [x] docs/ examples: `get-started/quickstart.md`, `get-started/configure.md`, `concepts/data-model.md`, `concepts/member-lifecycle.md`, `concepts/monitoring.md`, `how-to/mixed-backend-team.md`, `how-to/monitor-and-recover.md`, `reference/coding-agents/codex.md`, `reference/coding-agents/opencode.md` — rewrite command examples. <!-- completed: 2026-06-14T09:36 (data-model.md is docs/spec/, schema-only — no flag-placement edits needed) -->
- [x] `README.md`: rewrite the `cafleet --fleet-id …` example to the new shape; run `/update-readme` if the surface change is material. <!-- completed: 2026-06-14T09:36 -->
- [x] Sweep for doctrine prose: `grep -rniE "global .*fleet-id|fleet-id.*global|before the subcommand|placed before" docs/ skills/ .claude/ CLAUDE.md` and rewrite each remaining hit. <!-- completed: 2026-06-14T09:36 -->

### Step 2: Code

- [x] `cli/_helpers.py`: add `_fleet_id_callback` + `fleet_id_option` (custom missing message, `type=int`, `expose_value=False`, sets `ctx.obj["fleet_id"]`). <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/_helpers.py`: delete `require_fleet_id`; remove the `require_fleet_id(ctx)` call from the `client_command` wrapper. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/__init__.py`: remove the `--fleet-id` option and `fleet_id` param from `cli()`; keep `--json` + `--version`; `ctx.obj` no longer sets `fleet_id`. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/agent.py`: add `@fleet_id_option` to `register`, `list`, `show`, `deregister` — place it among the option decorators, mirroring the existing `--agent-id` decorator position (above `@click.pass_context` and the `@client_command` wrapper). The same stacking applies to `message.py`, `member.py`, and `monitor.py` below. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/message.py`: add `@fleet_id_option` to `send`, `broadcast`, `poll`, `ack`, `cancel`, `show`. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/member.py`: add `@fleet_id_option` to all 7 subcommands; remove the `require_fleet_id` import and the `require_fleet_id(ctx)` body lines (keep `fleet_id = ctx.obj["fleet_id"]`); note the `member create` error-precedence change. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/monitor.py`: add `@fleet_id_option` to `start`, `status`, `config`; remove the `require_fleet_id` import and body lines. <!-- completed: 2026-06-14T10:37 -->
- [x] Confirm `cli/fleet.py`, `cli/db.py`, `cli/doctor.py`, `cli/server.py` need no body change (none read `ctx.obj["fleet_id"]`) and now reject `--fleet-id` automatically. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/_prompt.py`: rewrite the `MEMBER_PROMPT_TEMPLATE` poll line to the new shape (`cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}`) so the default member spawn prompt keystrokes a command the post-change CLI accepts. <!-- completed: 2026-06-14T10:37 -->
- [x] `cli/member.py`: rewrite the orphan-rollback warning's suggested command in `_deregister_with_warning` to the new shape (`cafleet agent deregister --fleet-id {fleet_id} --agent-id {new_agent_id}`). <!-- completed: 2026-06-14T10:37 -->
- [x] `multiplexer/tmux.py`: rewrite the `send_poll_trigger` payload to the new shape (`cafleet message poll --fleet-id {fleet_id} --agent-id {agent_id}`) — the keystroke `member ping` and the monitor loop inject into agent panes. <!-- completed: 2026-06-14T10:37 -->

### Step 3: Tests

- [x] `tests/cli/test_fleet_flag.py`: rewrite per the Tests table — keep custom missing-message test; move flag position in flows-into-broker tests; replace the two `silently_accepted` tests with removal guards (db init / fleet create reject `--fleet-id` in both positions, exit 2); add `test_old_surface_removed__global_fleet_id_no_longer_parses`. <!-- completed: 2026-06-14T10:15 -->
- [x] `tests/cli/*.py` (agent, message, member*, monitor, fleet, doctor, server, compact_echo, message_truncation, fleet_bootstrap): move `--fleet-id` after the leaf subcommand in every `runner.invoke` arg list. <!-- completed: 2026-06-14T10:15 -->
- [x] `tests/cli/test_client_command.py`: update the in-test harness to the per-subcommand `fleet_id_option` shape if it declares a global `--fleet-id`; else leave unchanged. <!-- completed: 2026-06-14T10:15 -->
- [x] Confirm `tests/broker/**`, `tests/webui/**`, `tests/monitor/test_loop.py`, `tests/db/**` require no change (no CLI global `--fleet-id` invocations). <!-- completed: 2026-06-14T10:15 -->
- [x] Internal-keystroke payload assertions: update `tests/cli/test_member_ping.py::test_ping__keystrokes_escape_first`, `tests/multiplexer/test_tmux.py::test_send_poll_trigger__return_branches_and_argv`, and `tests/multiplexer/test_tmux_send_inline_preview.py::test_send_poll_trigger__keystroke_contract` to assert the new-shape payload (`cafleet message poll --fleet-id <s> --agent-id <m>`). `tests/cli/test_member_prompt_template.py` is shape-agnostic — no change. <!-- completed: 2026-06-14T10:20 -->
- [x] `tests/cli/test_help_budget.py`: bump the fleet-scoped per-subcommand line budgets (+1 each; `member create` +2 — its wide option column wraps `--fleet-id` help to 2 lines) and the aggregate byte budget (4480 → 5800, measured 5769) for the new per-subcommand option; kept the spec-mandated help string. <!-- completed: 2026-06-14T10:37 -->

### Step 4: Verification

- [x] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck` all clean. <!-- completed: 2026-06-14T10:40 -->
- [x] `mise //cafleet:test` green. <!-- completed: 2026-06-14T10:40 (828 passed) -->
- [ ] `mise //cafleet:install` (editable reinstall) so the new CLI surface is live for any manual/skill use. <!-- completed: -->
- [x] Repo grep (excluding `design-docs/`): `grep -rn "cafleet --fleet-id" docs/ skills/ README.md .claude/ CLAUDE.md` returns zero command-example hits; the doctrine-prose grep returns zero. <!-- completed: 2026-06-14T10:40 -->
- [ ] Manual smoke: `cafleet agent list --fleet-id <id>` works; `cafleet --fleet-id <id> agent list` → exit 2 `No such option`; `cafleet db init --fleet-id <id>` → exit 2 `No such option`; omitting `--fleet-id` → custom message exit 1. <!-- completed: -->
- [x] Stage the design doc with the implementation commits (project git-workflow override: `design-docs/` is committed here). <!-- completed: 2026-06-14T10:40 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-14 | Initial draft |
| 2026-06-14 | Director scope amendment during execution (Tester-surfaced): the original Step 2 list enumerated `--fleet-id` option *definitions* but omitted three internal command-string *emission* sites that hardcode the old shape and would break at runtime under the new surface — `cli/_prompt.py` (`MEMBER_PROMPT_TEMPLATE` poll line), `cli/member.py` (`_deregister_with_warning` suggested command), `multiplexer/tmux.py` (`send_poll_trigger` keystroke payload). Added 3 Step-2 tasks + 1 Step-3 task (update the two payload assertions in `test_member_ping.py` and `multiplexer/test_tmux.py`; `test_member_prompt_template.py` is shape-agnostic). Corrected the Tests table: `test_doctor.py`/`test_server.py` get rejection guards (not flag-moves) per SC#4; `tests/multiplexer/test_tmux.py` and `tests/output/test_render_agent.py` moved out of the "no change" row. Confirmed the Tester's three reconciliation decisions. Progress 32 → 36. |
| 2026-06-14 | Programmer-surfaced escalation, Director-arbitrated: the 11 code tasks landed correctly (820/828 green) but `tests/cli/test_help_budget.py` (never in the Tests table) failed 8 cases — the spec-mandated `fleet_id_option` help string adds one `--help` line to every fleet-scoped subcommand, blowing the per-subcommand line budgets and the aggregate byte budget (5769 > 4480). Arbitration: keep the spec help string; bump the budgets (the per-subcommand option intrinsically grows help). Added 1 Step-3 task + a Tests-table row. Progress 36 → 37. |
