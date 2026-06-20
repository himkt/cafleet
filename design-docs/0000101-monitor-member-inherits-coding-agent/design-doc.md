# Monitor member inherits the administrator's coding agent

**Status**: Approved
**Progress**: 17/17 tasks complete
**Last Updated**: 2026-06-20

## Overview

When the Director spawns the dedicated monitoring member with `cafleet member create --role monitor` and omits `--coding-agent`, the monitor must inherit the spawning Director's coding-agent backend (`claude` / `codex` / `opencode`) instead of falling back to the hardcoded `claude` default. The inherited backend drives both the spawned binary and the monitor's spawn-prompt `CODING AGENT:` line, so the monitor runs and reads its overlay on the same backend the administrator runs on. Resolves GitHub issue #132.

## Success Criteria

- [ ] `cafleet member create --role monitor` with `--coding-agent` **omitted** spawns the same backend binary recorded in the spawning Director's placement row, on all three backends.
- [ ] The monitor's spawn-prompt `CODING AGENT:` line names the inherited backend, so the monitor reads the matching `coding-agent/<name>.md` overlay.
- [ ] The monitor's own `agent_placements.coding_agent` records the inherited backend (visible in `cafleet member list`).
- [ ] An **explicit** `--coding-agent <x>` on a monitor still wins — inheritance applies only when the flag is omitted.
- [ ] Ordinary members (`--role member`, the default) keep today's behavior: omitting `--coding-agent` defaults to `claude`.
- [ ] When the spawning Director's backend cannot be resolved (no agent row / no placement row), `member create --role monitor` fails loudly with an actionable error and spawns nothing.
- [ ] `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` pass.

---

## Background

`cafleet member create` (`cafleet/src/cafleet/cli/member.py:107-114`) declares `--coding-agent` with `default="claude"`, and that value flows verbatim into the new member's placement (`member.py:183`) and into the backend selection `CODING_AGENTS[coding_agent]` (`member.py:159`) regardless of `--role`. So a monitoring member spawned by a `codex` or `opencode` Director without an explicit `--coding-agent` is launched as `claude` — a backend mismatch with the administrator it watches.

The administrator's backend is **already recorded** and queryable: `cafleet fleet create` writes the operator-declared `--coding-agent` into the root Director's `agent_placements.coding_agent` row (`cafleet/src/cafleet/cli/fleet.py:48` → `cafleet/src/cafleet/broker/fleets.py:53`). The `--agent-id` passed to `member create` **is** that spawning Director, so `broker.get_agent(<director-agent-id>, fleet_id)["placement"]["coding_agent"]` yields the backend to inherit. No new schema, migration, or backend auto-detection is required.

A second, easily-missed dimension: a member's spawn prompt carries a `CODING AGENT:` line that tells the member which backend overlay to read (the spawn-skeleton line at `skills/cafleet/reference/director.md:123`, explained at `director.md:136`). For ordinary members the Director fills this line literally; the monitoring member is the documented exception — its canonical prompt lives in the `cafleet-agent-team-monitoring` skill and currently hardcodes `CODING AGENT: claude` (`skills/cafleet-agent-team-monitoring/SKILL.md:50`). If only the binary inherited but the prompt line stayed `claude`, the monitor would run on `codex`/`opencode` while reading the `claude` overlay. Both must move together.

---

## Specification

### Resolved decisions (issue #132)

| # | Decision |
|---|----------|
| Scope | `--role monitor` only. Ordinary members keep today's `claude` default. |
| Source of truth | The **spawning Director's** placement row — the agent named by `--agent-id`. "Administrator" means that Director pane, **not** the reserved builtin `Administrator` agent. |
| Explicit flag | An explicit `--coding-agent <x>` **wins** for both roles. Inheritance happens only when the flag is omitted. No `UsageError` for combining `--role monitor` with `--coding-agent`. |
| Error handling | If the Director's backend cannot be resolved, **fail loudly** (exit 1) with an actionable message — no silent `claude` fallback. |
| Model | Out of scope. `--model` stays operator-controlled exactly as today; no auto model selection. |

### Backend resolution

Change the `--coding-agent` Click default from `"claude"` to `None`, then resolve the effective backend before any backend lookup or spawn:

```python
def _resolve_coding_agent(
    coding_agent: str | None,
    role: str,
    director_agent_id: int,
    fleet_id: int,
) -> str:
    """Resolve the backend for a new member.

    An explicit ``--coding-agent`` always wins. When the flag is omitted
    (``coding_agent is None``): ``--role monitor`` inherits the spawning
    Director's backend from its placement row; an ordinary member defaults
    to ``claude``.
    """
    if coding_agent is not None:
        return coding_agent
    if role != "monitor":
        return "claude"
    try:
        director = broker.get_agent(director_agent_id, fleet_id)
    except Exception as exc:  # broker/DB failure — surface, do not mask
        raise click.ClickException(
            f"cannot resolve the monitor's coding agent: failed to fetch "
            f"Director {director_agent_id}: {exc}. "
            f"Re-run with an explicit --coding-agent."
        ) from exc
    if director is None:
        raise click.ClickException(
            f"cannot resolve the monitor's coding agent: Director "
            f"{director_agent_id} not found in fleet {fleet_id}. "
            f"Re-run with an explicit --coding-agent."
        )
    placement = director["placement"]
    if placement is None:
        raise click.ClickException(
            f"cannot resolve the monitor's coding agent: Director "
            f"{director_agent_id} has no placement row recording its backend. "
            f"Re-run with an explicit --coding-agent."
        )
    return placement["coding_agent"]
```

Resolution truth table:

| `--coding-agent` | `--role` | Director placement | Effective backend |
|---|---|---|---|
| explicit `<x>` | any | — | `<x>` (explicit wins) |
| omitted | `member` (default) | — | `claude` (today's behavior) |
| omitted | `monitor` | resolvable | the Director's `placement.coding_agent` |
| omitted | `monitor` | missing agent / missing placement | **exit 1**, fail loud |

The resolved backend is the single value used for everything downstream: `CODING_AGENTS[coding_agent]` (binary + `ensure_available()`), the placement's recorded `coding_agent`, and the `{coding_agent}` prompt substitution below — so the binary, the stored metadata, and the overlay-selector can never diverge.

`placement["coding_agent"]` is `NOT NULL` (`server_default='claude'`), so once a placement row exists the inherited value is always a valid backend name already validated at the Director's own create time; no extra `CODING_AGENTS`-membership check is needed.

### Spawn-prompt `CODING AGENT:` substitution

`resolve_prompt` (`cafleet/src/cafleet/cli/_prompt.py:70-107`) today substitutes only `fleet_id` / `agent_id` / `director_agent_id` and raises `UsageError` on any other placeholder. Add `coding_agent` as a fourth substituted kwarg, passing the **resolved** backend:

```python
def resolve_prompt(ctx, director_agent_id, new_agent_id,
                   prompt_argv, prompt_file=None, coding_agent=None):
    ...
    return template.format(
        fleet_id=fleet_id,
        agent_id=new_agent_id,
        director_agent_id=director_agent_id,
        coding_agent=coding_agent,
    )
    # UsageError text → "Supported placeholders: {fleet_id}, {agent_id},
    #                    {director_agent_id}, {coding_agent}."
```

The monitor prompt template then uses `CODING AGENT: {coding_agent}` (instead of the hardcoded `claude`), and `cafleet member create` stamps the resolved backend into it. This is backward-compatible: existing prompts that contain no `{coding_agent}` token are unaffected; a Director may still fill an ordinary member's `CODING AGENT:` line literally as before.

### Call-site wiring (`member.py`)

1. Change the `--coding-agent` option: `default=None`, and a `show_default` string describing the dynamic behavior, e.g. `show_default="claude for members; the Director's backend for --role monitor"`.
2. Immediately after `fleet_id = ctx.obj["fleet_id"]` (before `agent = CODING_AGENTS[coding_agent]` at `member.py:159`), resolve: `coding_agent = _resolve_coding_agent(coding_agent, role, agent_id, fleet_id)`. Failing here (fail-loud) spawns nothing, since it precedes every tmux/registration side effect.
3. The existing placement dict (`member.py:178-184`) and `CODING_AGENTS[coding_agent]` lookup then use the resolved value with no further change.
4. Pass the resolved backend into the prompt: `resolve_prompt(ctx, agent_id, new_agent_id, prompt_argv, prompt_file, coding_agent=coding_agent)` (`member.py:196`).

### Out of scope (explicitly unchanged)

- `cafleet fleet create --coding-agent` (`fleet.py:22`) keeps `default="claude"` — it is the operator's declaration of the Director's own backend and the source of truth for inheritance.
- The `agent_placements.coding_agent` column default (`server_default="claude"`) and its Alembic migration — unchanged.
- `--model` handling and the per-backend `{monitor_model}` overlay values — unchanged. A Director reading its own overlay already substitutes a backend-appropriate `{monitor_model}` when it renders `--model`; inheriting the matching binary is exactly what aligns the two. The four monitor-spawn docs currently disagree on the example model (README `haiku`; `cli-options.md`/`monitoring.md` `sonnet`; `director.md`/`SKILL.md` the `{monitor_model}` token = `haiku`). Since Step 1 rewrites the hardcoded lines anyway, those rewrites align on one representation — the `{monitor_model}` token in skill templates, a single canonical `haiku` example in human-facing prose — so the reword resolves the inconsistency rather than swapping one hardcoded model for another.
- The model-name-to-backend inference table (`director.md:37-46`) — it governs ordinary members when the operator names a model; untouched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation and skills are updated FIRST (per `.claude/rules/design-doc-numbering.md`), before any code.

### Step 1: Documentation & skills

- [x] `docs/concepts/coding-agents.md` — in the `cafleet member create` paragraph (lines 30-34), state that for `--role monitor`, omitting `--coding-agent` inherits the spawning Director's backend (explicit flag still wins). <!-- completed: 2026-06-20T09:00 -->
- [x] `docs/spec/cli-options.md` — update the `--coding-agent` row (line 545) and the `--role` row (line 547) of `member create` to document monitor inheritance, replacing the `--model sonnet` example with the canonical `--model haiku`; add `coding_agent` to the substituted-placeholders list (lines 567-572); add the two fail-loud error strings to the Error Messages section. <!-- completed: 2026-06-20T09:00 -->
- [x] `docs/concepts/monitoring.md` — at "The monitoring member" spawn line (lines 108-109), which already omits `--coding-agent`, change the `--model sonnet` example to `--model haiku` and add a sentence that omitting `--coding-agent` now inherits the spawning Director's backend. <!-- completed: 2026-06-20T09:00 -->
- [x] `README.md` — at the §4 monitor spawn example (line 85), which already omits `--coding-agent` and uses `--model haiku`, add a clause that omitting `--coding-agent` inherits the spawning Director's backend (keep the `haiku` example). <!-- completed: 2026-06-20T09:00 -->
- [x] `skills/cafleet-agent-team-monitoring/SKILL.md` — change `CODING AGENT: claude` → `CODING AGENT: {coding_agent}` in the canonical monitor prompt (line 50); update the placeholder note (line 41) to include `{coding_agent}` and state that `--coding-agent` is omitted so the monitor inherits the Director's backend; keep the Monitor Lifecycle spawn command (line 132) free of `--coding-agent`. <!-- completed: 2026-06-20T09:00 -->
- [x] `skills/cafleet/reference/director.md` — update the `--coding-agent` row (line 29) and the `--role` row (line 31) for monitor inheritance; add `coding_agent` to the positional-prompt `str.format()` kwargs note (line 33). The spawn-skeleton `CODING AGENT: [INSERT …]` line (line 123) stays a Director-filled literal for ordinary members; reword the explanatory note (line 136) to distinguish the two paths — ordinary member = Director-filled literal, monitor = CLI-substituted `{coding_agent}` — and drop the blanket "no CLI code change is required" assertion. <!-- completed: 2026-06-20T09:00 -->

### Step 2: CLI implementation

- [x] `cafleet/src/cafleet/cli/_prompt.py` — add a `coding_agent` parameter to `resolve_prompt`, substitute it in the `template.format(...)` call, and update the docstring + the unknown-placeholder `UsageError` message to list `{coding_agent}`. <!-- completed: 2026-06-20T09:14 -->
- [x] `cafleet/src/cafleet/cli/member.py` — change `--coding-agent` to `default=None` with a `show_default` string describing the per-role default. <!-- completed: 2026-06-20T09:14 -->
- [x] `cafleet/src/cafleet/cli/member.py` — add the `_resolve_coding_agent` helper (explicit wins; `--role monitor` inherits the Director's `placement.coding_agent`; ordinary member → `claude`; fail loud on unresolvable Director). <!-- completed: 2026-06-20T09:14 -->
- [x] `cafleet/src/cafleet/cli/member.py` — call `_resolve_coding_agent(...)` right after `fleet_id` is read (before `CODING_AGENTS[coding_agent]`), and pass `coding_agent=coding_agent` into the `resolve_prompt(...)` call. <!-- completed: 2026-06-20T09:14 -->

### Step 3: Tests

- [x] `cafleet/tests/cli/test_member.py` — `resolve_prompt` unit tests: a `{coding_agent}` template substitutes the passed backend; an unknown placeholder still raises `UsageError` and the message now lists `{coding_agent}`. <!-- completed: 2026-06-20T09:06 -->
- [x] `cafleet/tests/cli/test_member.py` — monitor inheritance: with a non-claude Director (fleet created with `--coding-agent codex` / `opencode`, or its placement patched) and `--coding-agent` omitted, assert the spawned binary (`split_window_recorder[0]["command"][0]`), the monitor's `placement.coding_agent`, and the rendered prompt's `CODING AGENT:` line all equal the Director's backend. <!-- completed: 2026-06-20T09:06 -->
- [x] `cafleet/tests/cli/test_member.py` — explicit override: on a non-claude Director, `--role monitor --coding-agent claude` spawns `claude` (explicit wins). <!-- completed: 2026-06-20T09:06 -->
- [x] `cafleet/tests/cli/test_member.py` — scope guard: on a non-claude Director, an ordinary member (`--role member`) with `--coding-agent` omitted still spawns `claude`. <!-- completed: 2026-06-20T09:06 -->
- [x] `cafleet/tests/cli/test_member.py` — fail-loud (missing placement): `--role monitor` with `--coding-agent` omitted against a Director that has no placement row exits 1 with the "has no placement row" message and records no spawn. <!-- completed: 2026-06-20T09:06 -->
- [x] `cafleet/tests/cli/test_member.py` — fail-loud (Director not found): `--role monitor` with `--coding-agent` omitted and `--agent-id` pointing at a nonexistent/inactive agent exits 1 with the "not found in fleet" message and records no spawn, exercising the `director is None` branch. <!-- completed: 2026-06-20T09:06 -->
- [x] Run `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` — all green; existing monitor tests (`test_member_create__role_monitor_*`) still pass because a claude Director inherits `claude`. <!-- completed: 2026-06-20T09:15 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-20 | Initial draft |
| 2026-06-20 | Reviewer revisions applied; user-approved |
