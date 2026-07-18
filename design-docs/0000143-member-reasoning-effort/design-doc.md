# Member Reasoning Effort (`--effort` on `member create`)

**Status**: Approved
**Progress**: 0/17 tasks complete
**Last Updated**: 2026-07-18

## Overview

Add a backend-neutral `--effort <level>` option to `cafleet member create` (GitHub issue #211) that forwards a reasoning-effort level to the spawned coding-agent binary: claude via `--effort LEVEL`, codex via `--config=model_reasoning_effort=LEVEL`. The opencode backend does not expose a reasoning-effort control and rejects the flag with a hard error.

## Success Criteria

- [ ] `cafleet member create ... --effort high` on the claude backend spawns an argv containing `--effort high`; on the codex backend, `--config=model_reasoning_effort=high`.
- [ ] An invalid level for the selected backend exits 2 with a `UsageError` naming the accepted levels, before any registration or multiplexer side effect.
- [ ] `--effort` with the opencode backend exits 2 with the message `opencode does not support reasoning effort.`, before any side effect.
- [ ] When `--effort` is omitted, the spawn argv is byte-identical to today's (no effort tokens; the backend binary's own default applies).
- [ ] No DB schema change: `member show` / `member list`, the webui API, and the Alembic chain are untouched.
- [ ] SPEC.md, `docs/spec/cli-options.md`, `docs/spec/coding-agent-backends.md`, `docs/concepts/coding-agents.md`, the cafleet skill pages, and all four coding-agent overlays document the flag; the opencode overlay explicitly marks effort as unsupported.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

`cafleet member create` already forwards one backend tuning knob, `--model`, as a spawn-time pass-through: it is validated per backend (`CodingAgent.validate_model`), emitted into the spawn argv by `build_spawn_argv`, and never persisted. Reasoning effort is a second knob of the same shape, with one asymmetry: only the claude and codex binaries expose a reasoning-effort control. `--effort` mirrors the `--model` pattern exactly, plus per-backend enum validation (precedent: opencode's `validate_model` format check) and a hard error on the unsupported backend.

---

## Specification

### CLI surface

New option on `cafleet member create` (`cafleet/src/cafleet/cli/member.py`), declared alongside `--model`:

```python
@click.option(
    "--effort",
    "effort",
    type=str,
    default=None,
    help="Reasoning-effort level (claude, codex only).",
)
```

- `type=str` (not `click.Choice`): the accepted level set differs per backend, so validation is delegated to the backend's `validate_effort` (see below). A CLI-level union Choice would wrongly accept e.g. `minimal` for claude.
- The help string renders on one line in `member create`'s wide-option-column help layout at Click's 80-column width, so the option adds exactly one line to `cafleet member create --help` (see the help-budget task in Step 4). `--effort` does not join `_UNHIDDEN_FLAGS` in `tests/cli/test_unhidden_flags.py`: that guard covers the shared once-hidden operator flags (`--full` / `--quiet` / `--ansi`); `--effort`, like `--model`, is an ordinarily visible option.
- `default=None`: omitted flag → no effort tokens in the argv, no inheritance from the Director. Absence is a valid state whose correct behavior is "backend default".
- Validation ordering in `member_create`: call `agent.validate_effort(effort)` immediately after the existing `agent.validate_model(model)` call, with the same `ValueError` → `click.UsageError` wrapping (exit 2). Both run before `read_text_input`, `ensure_available`, `register_member`, and any multiplexer side effect.
- Spawn construction: `agent.build_spawn_argv(prompt, display_name=name, model=model, effort=effort)`.

### Per-backend contract

| Backend | Accepted levels (ordered) | Argv mapping | Behavior on invalid / unsupported |
|:--|:--|:--|:--|
| claude | `low`, `medium`, `high`, `xhigh`, `max` | `["--effort", level]`, emitted immediately after the model tokens (before the prompt) | `ValueError` → `UsageError`, exit 2 |
| codex | `minimal`, `low`, `medium`, `high`, `xhigh` | `[f"--config=model_reasoning_effort={level}"]` (single token, the issue #211 spelling), emitted immediately after the model tokens (before the prompt) | `ValueError` → `UsageError`, exit 2 |
| opencode | — (unsupported) | never emitted | any non-`None` effort: `ValueError` → `UsageError`, exit 2 |

Level sets are module-level tuple constants in each backend module: `EFFORT_LEVELS = ("low", "medium", "high", "xhigh", "max")` in `claude.py`, `EFFORT_LEVELS = ("minimal", "low", "medium", "high", "xhigh")` in `codex.py`. The codex set is the `model_reasoning_effort` value list in the Codex CLI configuration reference (`minimal | low | medium | high | xhigh`, verified 2026-07-18) — the CLI config surface, not the broader API-level `reasoning.effort` list, is what the spawned binary accepts.

Client-side enum validation (unlike `validate_model`'s pass-through) is deliberate: an invalid effort level rejected only by the binary would first register the member and spawn its pane, leaving a registered-but-dead member to clean up, whereas model names form a wide, frequently-changing set where pass-through is the lesser cost. The drift risk of hard-coding is accepted and mitigated by pinning the verified source above.

### Error strings (exact)

| Case | Message |
|:--|:--|
| claude, unknown level | `--effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| codex, unknown level | `--effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| opencode, any value | `opencode does not support reasoning effort.` |

### Protocol change (`cafleet/src/cafleet/coding_agent/base.py`)

Extend the `CodingAgent` Protocol with `validate_effort` (mirroring `validate_model`: `None` is always valid; called by `member create` before any side effect) and add the `effort` keyword to `build_spawn_argv`:

```python
def validate_effort(self, effort: str | None) -> None:
    """Raise ValueError if ``effort`` is not acceptable to this backend.

    ``None`` (flag omitted) is always valid. Backends without a
    reasoning-effort control reject every non-None value.
    """
    ...

def build_spawn_argv(
    self,
    prompt: str,
    *,
    display_name: str,
    model: str | None = None,
    effort: str | None = None,
) -> list[str]: ...
```

The `build_spawn_argv` docstring documents `effort` next to `model`: forwarded via the backend's reasoning-effort flag, emitted immediately after the model tokens; `None` emits no effort tokens, keeping the argv byte-identical to the no-effort form.

### Backend implementations

`claude.py`:

```python
EFFORT_LEVELS = ("low", "medium", "high", "xhigh", "max")

def validate_effort(self, effort: str | None) -> None:
    if effort is None:
        return
    if effort not in EFFORT_LEVELS:
        raise ValueError(
            "--effort for the claude backend must be one of "
            f"{', '.join(EFFORT_LEVELS)} (got '{effort}')."
        )

# in build_spawn_argv, after the model block:
if effort is not None:
    argv.extend(["--effort", effort])
```

`codex.py`: same `validate_effort` shape over its own `EFFORT_LEVELS`; argv block emits the single token:

```python
if effort is not None:
    argv.append(f"--config=model_reasoning_effort={effort}")
```

`opencode.py`:

```python
def validate_effort(self, effort: str | None) -> None:
    if effort is None:
        return
    raise ValueError("opencode does not support reasoning effort.")

# in build_spawn_argv, after `del display_name`:
assert effort is None, "opencode effort must be rejected by validate_effort"
```

The two parameters differ in kind, so opencode handles them differently: `display_name` is a valid input the backend has no flag for (silently ignored via `del`), while a non-`None` `effort` is a can't-happen state — the CLI rejects it in `validate_effort` before any spawn — so `build_spawn_argv` asserts `effort is None` and fails loudly if the invariant is ever violated.

### Skill overlay placeholder: `{effort_levels}`

Each coding-agent overlay (`skills/cafleet/reference/coding-agent/<name>-overlay.md`, plus `_template.md`) gains one placeholder-table row so spawn recipes can name the backend's effort surface:

| Overlay | `{effort_levels}` value |
|:--|:--|
| claude | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |
| codex | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`, forwarded as `--config=model_reasoning_effort=<level>`) |
| opencode | unsupported — omit `--effort` |

The documented default (overlay silent / backend unknown) added to `skills/cafleet/SKILL.md` § *Resolve your overlay* is `unsupported — omit --effort`: the neutral floor that functions on every backend. Existing spawn recipes (monitor, reviewer) are unchanged — no recipe passes `--effort`; the placeholder makes the per-backend level sets available to Directors that choose to.

### Non-goals

- **No persistence**: no `member_placements` column, no Alembic migration, no `member show` / `member list` column, no webui API field — identical to `--model`.
- **No inheritance**: unlike `--coding-agent`, effort is never inherited from the Director.
- **No opencode support path**: opencode exposes no reasoning-effort CLI control; rejection is the specified behavior, not a stopgap.
- **No changes to existing spawn recipes** in skills (monitor / reviewer spawn lines keep their current shape).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [ ] `docs/concepts/coding-agents.md`: add a "Reasoning effort" subsection beside "Model selection" (flag, per-backend mapping) and an entry under "Known asymmetries (intentional non-goals)" stating opencode does not support effort <!-- completed: -->
- [ ] `docs/spec/coding-agent-backends.md`: add effort to the spawn-argv table / shared contract, the claude and codex sections (levels + exact argv form), and the opencode section (unsupported + error string) <!-- completed: -->
- [ ] `docs/spec/cli-options.md`: add the `--effort` row to the `member create` flag table and the three exact error strings to the error catalogue <!-- completed: -->
- [ ] `SPEC.md`: update the member-create option list, the validation step (validate_model then validate_effort), the per-backend argv construction, the error-string catalogue, and the member-create flag checklist <!-- completed: -->
- [ ] `skills/cafleet/reference/director.md` (flag table) and `skills/cafleet/reference/cli.md` (member-create prose): document `--effort` with the per-backend level sets and the opencode rejection <!-- completed: -->
- [ ] Overlays: add the `{effort_levels}` row to `skills/cafleet/reference/coding-agent/_template.md`, `claude-overlay.md`, `codex-overlay.md`, and `opencode-overlay.md` (opencode: unsupported — omit `--effort`); add the `{effort_levels}` documented default to `skills/cafleet/SKILL.md` § Resolve your overlay <!-- completed: -->

### Step 2: Backend protocol and implementations

- [ ] `coding_agent/base.py`: add `validate_effort` to the `CodingAgent` Protocol and the `effort` keyword (with docstring) to `build_spawn_argv` <!-- completed: -->
- [ ] `coding_agent/claude.py`: `EFFORT_LEVELS` tuple, `validate_effort`, `--effort` argv emission after the model block <!-- completed: -->
- [ ] `coding_agent/codex.py`: `EFFORT_LEVELS` tuple, `validate_effort`, single-token `--config=model_reasoning_effort=<level>` emission after the model block <!-- completed: -->
- [ ] `coding_agent/opencode.py`: `validate_effort` raising `ValueError("opencode does not support reasoning effort.")`; `build_spawn_argv` gains the `effort` kwarg and asserts `effort is None` <!-- completed: -->

### Step 3: CLI wiring

- [ ] `cli/member.py`: add the `--effort` option, call `agent.validate_effort(effort)` immediately after `agent.validate_model(model)` with the same `UsageError` wrapping, and pass `effort=effort` to `build_spawn_argv` <!-- completed: -->

### Step 4: Tests and verification

- [ ] `tests/coding_agent/test_protocol.py`: per-backend `validate_effort` accept/reject cases and argv assertions (claude `--effort` tokens, codex single `--config=` token, omitted-effort argv unchanged) <!-- completed: -->
- [ ] `tests/coding_agent/test_opencode.py`: `validate_effort` rejects every non-None value with the exact message <!-- completed: -->
- [ ] `tests/cli/test_member.py`: `--effort` happy path per backend, invalid-level exit 2 before registration/multiplexer side effects, opencode rejection exit 2 <!-- completed: -->
- [ ] `tests/cli/test_help_budget.py`: bump `_PER_SUBCOMMAND_BUDGETS[("member", "create")]` from 20 to 21 (the new single-line option row); keep the aggregate byte budget at 6500 — if the ~80-byte row pushes the aggregate assertion over, set it to 6600 in the same change (pre-authorized here). No `_UNHIDDEN_FLAGS` change (see § CLI surface) <!-- completed: -->
- [ ] Run `mise //cafleet:test` <!-- completed: -->
- [ ] Run `mise //cafleet:lint` and `mise //cafleet:typecheck` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
| 2026-07-18 | Review round 1: codex levels re-verified against the Codex CLI config reference (`xhigh` added); recorded the client-side-enum rationale; opencode error string gains a trailing period; opencode `build_spawn_argv` asserts `effort is None`; concrete help string and help-budget values specified |
