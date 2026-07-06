# Herdr Multiplexer Backend

**Status**: Approved
**Progress**: 22/31 tasks complete
**Last Updated**: 2026-07-06

## Overview

Add [herdr](https://herdr.dev) as a second terminal-multiplexer backend for CAFleet alongside tmux, selected by auto-detecting the runtime environment. The `HerdrMultiplexer` satisfies the existing `Multiplexer` Protocol so every spawn/keystroke/capture path works unchanged, and — because herdr natively tracks each agent's lifecycle state — an isolated, herdr-only capability lets the monitor react to native `blocked`/`done` transitions instead of waiting out a fixed interval.

## Success Criteria

- [ ] Running CAFleet inside a herdr pane spawns members, delivers messages, captures panes, and tears down teams with no tmux dependency.
- [ ] Running CAFleet inside tmux behaves exactly as before (no behavior change on the tmux path).
- [ ] The active backend is auto-detected (`HERDR_ENV` → herdr, `TMUX` → tmux); an ambiguous or empty environment fails loudly, and `CAFLEET_MULTIPLEXER` is an explicit override.
- [ ] `AgentPlacement` stores backend-neutral pane identity (`mux_session` / `mux_window_id` / `mux_pane_id`) plus a `backend` column, migrated via Alembic; no `tmux_*` identifier survives outside git history, this design doc, and the immutable Alembic migrations (`0001` creation + `0008` rename).
- [ ] On the herdr backend, the monitor flags a watched agent due when its native status enters an attention state (`blocked`/`done`), in addition to the existing interval trigger; the tmux path stays interval-only.
- [ ] `cafleet doctor` reports the resolved backend and its identifiers for either backend.
- [ ] Documentation (README, SPEC, `docs/`, affected `SKILL.md`) is updated first and stays drift-free.

---

## Background

CAFleet is already architected for pluggable multiplexers: `cafleet/src/cafleet/multiplexer/base.py` defines a `@runtime_checkable` `Multiplexer` Protocol, `tmux.py` is the sole implementation (all `tmux` subprocess calls isolated there), and `__init__.py` exposes a `MULTIPLEXERS` registry. Three things block a second backend today:

1. **No selection mechanism** — `config.py` has no multiplexer setting; tmux is chosen implicitly by hardcoded `MULTIPLEXERS["tmux"]` lookups, plus two direct `TmuxMultiplexer()` instantiations (`broker/messaging.py:44`, `monitor/loop.py:80`).
2. **tmux-flavored persistence** — `AgentPlacement` stores pane identity in `tmux_session` / `tmux_window_id` / `tmux_pane_id`; herdr pane ids (e.g. `w1:p1`) are opaque strings that do not fit the tmux naming.
3. **Abstraction gap for native state** — herdr tracks agent lifecycle states (`working`/`blocked`/`done`/`idle`/`unknown`) with `wait agent-status` and a `pane.agent_status_changed` event stream; the `Multiplexer` Protocol has no surface for this, and the monitor loop only knows interval-based waking.

herdr is a client-server terminal workspace manager for AI coding agents: a persistent background server owns real terminal processes, clients attach to render them, and processes survive detach/SSH-drop/server-restart. Its concept hierarchy is session → workspace → tab → pane, with an **agent** being a detected process inside a pane carrying a lifecycle state. It exposes both a CLI and a newline-delimited JSON unix-socket API.

---

## Specification

### 1. Concept and operation mapping

herdr's pane primitives map onto every `Multiplexer` Protocol method. Pane ids are treated as **opaque strings** end to end (`w1:p1`), never parsed.

| `Multiplexer` method (tmux realization) | herdr realization (CLI) |
|---|---|
| `context_discovery()` — `$TMUX_PANE` + `display-message` | `HERDR_ENV=1` present + `herdr pane current` → current pane id; `herdr pane get <id>` → owning tab/session |
| `split_window(reference, env, command)` — `split-window -t <window> -d … <argv>` | `herdr pane split <reference.pane_id> --direction down --no-focus [--cwd P] [--env K=V …]` → new pane id, then `herdr pane run <new_id> "<shlex.join(command)>"` (atomic text+Enter) |
| `kill_pane()` — `kill-pane -t <pane>` | `herdr pane close <pane_id>` |
| `list_pane_ids()` — `list-panes -a -F '#{pane_id}'` | `herdr pane list` → set of pane ids |
| `wait_for_pane_gone()` — poll `list-panes` | `poll_until_pane_gone` over `herdr pane get <id>` (absent → gone) |
| `send_exit()` — `send-keys -l "/exit"` + Enter | `herdr pane run <id> "/exit"` |
| `send_poll_trigger()` — Esc, then `send-keys -l "cafleet … poll"` + Enter | `herdr pane send-keys <id> esc`, then `herdr pane run <id> "cafleet message poll …"` |
| `send_wake_trigger()` — `send-keys -l "<payload>"` + Enter (no Esc) | `herdr pane run <id> "<payload>"` |
| `send_inline_preview()` — Esc, then `send-keys -l "<2-line>"` + Enter | `herdr pane send-keys <id> esc`, then `herdr pane send-text <id> "<2-line>"`, then `herdr pane send-keys <id> enter` |
| `send_bash_command()` — `send-keys -l "! <cmd>"` + Enter | `herdr pane run <id> "! <cmd>"` |
| `capture_pane(lines)` — `capture-pane -p -S -<lines>` | `herdr pane read <id> --source recent-unwrapped --lines <lines>` |

**Notes on the mapping:**

- **`split_window` signature generalizes.** tmux splits a *window*; herdr splits a *pane*. The current `target_window_id: str` parameter cannot carry the pane id herdr needs, so the method takes the full reference context: `split_window(*, reference: MultiplexerContext, env, command) -> str`. tmux uses `reference.window_id`; herdr uses `reference.pane_id`. The sole call site (`cli/member.py`) already holds the Director's `MultiplexerContext` and passes it directly.
- **Atomic submit.** herdr `pane run` submits text **and** Enter atomically, so the tmux `_SUBMIT_DELAY` (0.12 s between literal text and Enter) has no herdr analog — the herdr backend omits it.
- **Esc safeguard.** The `esc_first` permission-prompt safeguard maps to a discrete `herdr pane send-keys <id> esc` before the payload (with the same short settle delay) on exactly the paths that use it today (`send_poll_trigger`, `send_inline_preview`).
- **Two-line inline preview.** herdr keeps the tmux contract of "one submit for the whole 2-line payload" by using `pane send-text` (raw, no Enter — the embedded newline is literal) followed by a single `pane send-keys enter`.
- **argv → shell string.** tmux `split-window … <argv>` execs the `command: list[str]` argv directly, preserving argument boundaries with no shell re-parse. herdr `pane run` submits one text line into the pane's shell, so `HerdrMultiplexer.split_window` renders the argv to a single properly-quoted string with `shlex.join(command)` before the `pane run` call — otherwise an argument containing spaces would be re-split. This is a genuine semantic difference from the tmux exec-argv path.

### 2. Access mechanism: CLI (chosen), socket deferred

herdr exposes both a CLI (subprocess) and a newline-delimited JSON unix-socket API. **`HerdrMultiplexer` uses the CLI exclusively**, mirroring `tmux.py`'s `_run()` subprocess dispatcher.

| Concern | CLI | Socket |
|---|---|---|
| Pane primitives (split/run/read/list/close/…) | ✅ full | ✅ full |
| Native agent status (point read) | ✅ `pane get` / `pane read --source detection` | ✅ `agent.get` |
| Bounded wait for a status | ✅ `wait agent-status … --timeout MS` | ✅ via `events.subscribe` |
| **Push** event stream (`agent_status_changed`) | ❌ not available | ✅ `events.subscribe` (connection stays open) |
| Testability | ✅ mock one `_run` dispatcher (parity with `test_tmux.py`) | ✗ mock a persistent JSON client |
| Fit with CAFleet's synchronous, connectionless loop | ✅ | ✗ needs a background reader/concurrency |

**Rationale.** Native agent-state — the Q1 requirement — is fully reachable through the CLI: `herdr pane get` yields the current status (point read) and `herdr wait agent-status` blocks with a timeout. The socket's *only* unique capability is **push** events, which would require a persistent open connection and a concurrent reader thread that CAFleet's plain synchronous `scan → wake → sleep` monitor loop does not have. Choosing the CLI keeps the dispatcher testable exactly like tmux and fits the existing architecture; the monitor gets native state via a cheap per-tick point read (§5). The socket event stream is a deliberately-deferred optimization (documented here, not built) — its payoff is instant reaction, which per-tick polling at the monitor's tick cadence already approximates.

### 3. Backend selection and resolver

Add a resolver that every call site uses instead of a hardcoded `MULTIPLEXERS["tmux"]`.

**Config** (`config.py`): add one field.

```python
multiplexer: str | None = Field(default=None, validation_alias="CAFLEET_MULTIPLEXER")
```

`None` (unset) means "auto-detect" — a legitimate default (absence is a valid, well-defined state), not a fallback for a missing value.

**Resolver** (`multiplexer/__init__.py`): `resolve_multiplexer() -> Multiplexer` with this precedence:

1. **Explicit override.** If `settings.multiplexer` is set, it must be a registry key; otherwise raise.
2. **Auto-detect.** Read the environment: `HERDR_ENV` truthy → herdr present; `TMUX` set → tmux present.
3. **Ambiguity is a hard error.** Both present → raise. Neither present → raise. Exactly one → that backend.

```python
def resolve_multiplexer() -> Multiplexer:
    override = settings.multiplexer
    if override is not None:
        if override not in MULTIPLEXERS:
            raise MultiplexerError(
                f"CAFLEET_MULTIPLEXER={override!r} is not a supported multiplexer "
                f"(expected one of: {', '.join(sorted(MULTIPLEXERS))})"
            )
        return MULTIPLEXERS[override]
    in_herdr = bool(os.environ.get("HERDR_ENV"))
    in_tmux = bool(os.environ.get("TMUX"))
    if in_herdr and in_tmux:
        raise MultiplexerError(
            "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; "
            "set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate"
        )
    if in_herdr:
        return MULTIPLEXERS["herdr"]
    if in_tmux:
        return MULTIPLEXERS["tmux"]
    raise MultiplexerError(
        "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; "
        "run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER"
    )
```

This is fail-fast per `affirmative-writing.md`: dual-detection asserts the invariant and stops rather than silently guessing; the override is the deterministic escape hatch.

**Error taxonomy.** Introduce a shared base `class MultiplexerError(Exception)` in `base.py`; make `TmuxError(MultiplexerError)` and a new `HerdrError(MultiplexerError)`. Every CLI boundary that today catches `TmuxError` to convert to an application error catches `MultiplexerError` instead, so both backends' failures are handled uniformly while each backend keeps its own message text.

### 4. Persistence: backend-neutral placement columns

Rename the three `tmux_*` columns and add a `backend` column recording which multiplexer produced the ids.

| Current column | New column | Type | Constraints |
|---|---|---|---|
| `tmux_session` | `mux_session` | `TEXT` | `NOT NULL` |
| `tmux_window_id` | `mux_window_id` | `TEXT` | `NOT NULL` |
| `tmux_pane_id` | `mux_pane_id` | `TEXT` | nullable (`NULL` = pending until split resolves the pane) |
| — (new) | `backend` | `TEXT` | `NOT NULL`, `DEFAULT 'tmux'` |

- `MultiplexerContext` is already backend-neutral (`session` / `window_id` / `pane_id`) — no change.
- `backend` is set to the resolved `mux.name` at placement-insert time (register + director bootstrap). herdr ids (`w1:p1`) are stored verbatim as opaque strings.
- **Alembic migration `0008`** (`down_revision = "0007"`): `batch_alter_table` on `agent_placements` to rename the three columns and add `backend TEXT NOT NULL DEFAULT 'tmux'`. The default backfills existing rows to `'tmux'`, matching their real provenance (consistent with `coding_agent`'s `DEFAULT 'claude'`).

Per `removal.md`, after this lands no `tmux_session` / `tmux_window_id` / `tmux_pane_id` identifier survives in source, docs, SPEC, or tests. The immutable Alembic history is the one carve-out: migration `0001` (which created the columns) names them, and `0008` names them in its `batch_alter_table(...).alter_column(..., new_column_name=...)` rename — historical migrations must never be rewritten. So the surviving mentions are exactly: git history, this design doc, and `db/alembic/versions/` (`0001` + `0008`).

### 5. Native agent-state: an isolated, herdr-only capability

Keep the base `Multiplexer` Protocol clean (tmux need not implement anything new) by adding a **separate optional capability** Protocol that only herdr implements:

```python
@runtime_checkable
class AgentStateAware(Protocol):
    def agent_status(self, *, target_pane_id: str) -> str | None:
        """Current native agent state (working|blocked|done|idle|unknown),
        or None if no agent is detected in the pane."""
        ...

    def wait_agent_status(
        self, *, target_pane_id: str, status: str, timeout_ms: int
    ) -> bool:
        """Block until the pane's agent reaches `status` or the timeout
        elapses. Returns True if reached."""
        ...
```

- `HerdrMultiplexer` implements both (`agent_status` → `herdr pane get`/`pane read --source detection`; `wait_agent_status` → `herdr wait agent-status <id> --status <s> --timeout <ms>`). `TmuxMultiplexer` does **not** implement `AgentStateAware`.
- **Monitor consumption (the "when").** The new state and logic live **entirely in `monitor/loop.py`**; `broker/monitor.py`'s `monitor_tick` is unchanged — it keeps computing interval-due-ness only, with no knowledge of native status. The single long-running loop process owns an in-memory `dict[agent_id, last_status]` and resolves its backend via `resolve_multiplexer()`. Each tick, when `isinstance(mux, AgentStateAware)`, the loop point-reads `agent_status` for each watched agent whose pane is alive and detects a transition **into an attention state** (`blocked` or `done`) against that dict, so one episode wakes only once. The loop then unions those native-due agents with `monitor_tick`'s interval-due set to decide the wake, tagging each native one with the `status:<state>` wake-reason label. No DB column is added. On the tmux backend the `isinstance` guard is false, so the branch never runs and the interval-only behavior is byte-for-byte unchanged.
- This augments, never replaces, the interval trigger: an agent is due when its interval elapsed **or** (herdr only) its native status entered an attention state.

### 6. `doctor` and CLI-surface generalization

- **`cli/doctor.py`** resolves the backend via `resolve_multiplexer()`, calls `ensure_available()` + `context_discovery()`, and reports under a backend-neutral key. The JSON `tmux` object and text `tmux:` block become a `multiplexer` object / `multiplexer:` block carrying `backend` plus `session` / `window_id` / `pane_id` (and the backend's presence env var). This is a documented contract change to `doctor` output (SPEC §doctor, `cli-options.md`, and the doctor test update accordingly).
- **`cli/_helpers.py`** `ensure_tmux_or_die` → `ensure_multiplexer_or_die`, calling `resolve_multiplexer().ensure_available()`. Callers updated.
- **`fleet create` precondition string** generalizes from `cafleet fleet create must be run inside a tmux session` to name both backends: `cafleet fleet create must be run inside a tmux or herdr session`.
- **Direct-instantiation leaks fixed.** `broker/messaging.py:44` and `monitor/loop.py:80` call `resolve_multiplexer()` instead of `TmuxMultiplexer()`.

### 7. Dependencies and assumptions

- The `herdr` binary is expected on `PATH` when running inside a herdr environment; `ensure_available()` fails fast (`HerdrError`) when it is missing or `HERDR_ENV` is unset. No version pin is added; the CLI surface used is the stable `pane`/`wait` command set.
- No new Python dependency — the backend shells out to `herdr` exactly as `tmux.py` shells out to `tmux`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Add/update `docs/concepts/` for the multiplexer-backend concept: generalize `docs/api/multiplexer.md` beyond "the tmux abstraction", and add a backend-selection/auto-detection description (new concept page or a section in an existing one). <!-- completed: 2026-07-06T10:13 -->
- [x] Update `docs/concepts/monitoring.md` to describe the herdr-only native-status due trigger (attention states `blocked`/`done`), isolated from the interval heartbeat. <!-- completed: 2026-07-06T10:13 -->
- [x] Update `docs/spec/data-model.md` `agent_placements` table: rename the three columns to `mux_*` and add the `backend` column with constraints and provenance notes. <!-- completed: 2026-07-06T10:13 -->
- [x] Update `docs/spec/cli-options.md` for `cafleet doctor` output (backend-neutral `multiplexer` block), the `CAFLEET_MULTIPLEXER` env var, the generalized `fleet create` precondition string, and the shared `placement` projection keys at lines 692-693 (`tmux_session` / `tmux_window_id` / `tmux_pane_id` → `mux_*`, plus `backend`). <!-- completed: 2026-07-06T10:13 -->
- [x] Update `SPEC.md`: §6.5 (Multiplexer — the `Multiplexer` Protocol, add `AgentStateAware`, the `split_window(reference=…)` signature, the `MULTIPLEXERS` registry + `resolve_multiplexer` precedence and error strings, `MultiplexerError` taxonomy), §6.6 (monitor native-status trigger), the `agent_placements` column names, the placement projection keys, the formatter renders, and the `doctor` output contract. <!-- completed: 2026-07-06T10:13 -->
- [x] Update `README.md` to describe herdr as a supported backend and how selection works. <!-- completed: 2026-07-06T10:13 -->
- [x] Update every affected `skills/*/SKILL.md` and `docs/reference/coding-agents/` / `docs/how-to/mixed-backend-team.md` mention that names tmux-specific placement columns or assumes tmux-only hosting. <!-- completed: 2026-07-06T10:13 -->
- [x] Generalize `docs/concepts/tmux-push.md`: update its `tmux_pane_id` references (lines 31, 42, 80) to the `mux_pane_id` column, and generalize the tmux-only framing to cover herdr's analogous push path — deciding whether the page/filename itself needs a backend-neutral rename. <!-- completed: 2026-07-06T10:13 -->

### Step 2: Config and resolver

- [x] Add `multiplexer: str | None` field to `Settings` (`config.py`) bound to `CAFLEET_MULTIPLEXER`. <!-- completed: 2026-07-06T10:22 -->
- [x] Add `MultiplexerError` base to `base.py`; make `TmuxError` subclass it. <!-- completed: 2026-07-06T10:22 -->
- [x] Add `resolve_multiplexer()` to `multiplexer/__init__.py` with the §3 precedence and exact error strings; export it and `MultiplexerError` / `AgentStateAware` (`AgentStateAware` Protocol added to `base.py` here to satisfy the export; Step 3's `AgentStateAware` task lands with it). <!-- completed: 2026-07-06T10:22 -->

### Step 3: Protocol generalization

- [x] Change `Multiplexer.split_window` signature to `split_window(*, reference: MultiplexerContext, env, command) -> str` in `base.py`; update `TmuxMultiplexer.split_window` to use `reference.window_id` and update its `cli/member.py` call site to pass the Director's context. <!-- completed: 2026-07-06T10:26 -->
- [x] Add the `AgentStateAware` `@runtime_checkable` Protocol to `base.py`. <!-- completed: 2026-07-06T10:26 -->

COMMENT(programmer): The `split_window(reference=…)` signature change breaks `tests/multiplexer/test_tmux.py::test_split_window__argv_construction` (calls `_tmux.split_window(target_window_id="@3"/"@5", …)`). Impl is correct per spec; the test needs the Tester to switch to `reference=MultiplexerContext(...)` in Step 7. CLI tests are unaffected (their `fake_split_window(self, **kwargs)` monkeypatch absorbs the kwarg rename).

### Step 4: HerdrMultiplexer

- [x] Create `multiplexer/herdr.py`: `HerdrError(MultiplexerError)`, a `_run()` CLI dispatcher (argv list, no shell) mirroring `tmux.py` (binary-not-found / timeout / non-zero mapped to `HerdrError`; a `not_found`-tolerant helper for `ignore_missing`), and `HerdrMultiplexer` implementing every `Multiplexer` method per the §1 mapping (no `_SUBMIT_DELAY`; `esc_first` → discrete `send-keys esc`). <!-- completed: 2026-07-06T10:36 -->
- [x] Implement `AgentStateAware` on `HerdrMultiplexer` (`agent_status`, `wait_agent_status`) per §5. <!-- completed: 2026-07-06T10:36 -->
- [x] Register `"herdr": HerdrMultiplexer()` in `MULTIPLEXERS`. <!-- completed: 2026-07-06T10:36 -->

COMMENT(programmer): `capture_pane` (`herdr pane read`) — the exact `result` key holding the pane text is pending the operator's validation run; `herdr.py` reads `result["output"]`, falling back to `result["content"]`. All other herdr JSON keys are now per the confirmed contract. Step 7's `test_herdr.py` pins the `pane read` argv + whichever key the operator confirms.

### Step 5: Route every call site through the resolver

- [x] `cli/member.py` — replace all `MULTIPLEXERS["tmux"]` uses (create/delete/capture/exec/ping) with `resolve_multiplexer()`; pass the Director context to `split_window`; catch `MultiplexerError`. <!-- completed: 2026-07-06T10:54 -->
- [x] `cli/fleet.py` — `resolve_multiplexer()` for `ensure_available`/`context_discovery`; catch `MultiplexerError`; generalize the precondition string. <!-- completed: 2026-07-06T10:54 -->
- [x] `cli/_helpers.py` — rename `ensure_tmux_or_die` → `ensure_multiplexer_or_die` via the resolver; swap its `TmuxError` import/catch to `MultiplexerError`; update callers. <!-- completed: 2026-07-06T10:54 -->
- [x] `cli/doctor.py` — resolve backend; emit the backend-neutral `multiplexer` block (§6); swap its `TmuxError` import/catch to `MultiplexerError`. <!-- completed: 2026-07-06T10:54 -->
- [x] `broker/messaging.py:44` — replace `TmuxMultiplexer()` with `resolve_multiplexer()`. <!-- completed: 2026-07-06T10:54 -->
- [x] `monitor/loop.py:80` — replace `TmuxMultiplexer()` with `resolve_multiplexer()`; add the herdr-only `AgentStateAware` native-status branch (§5), with `loop.py` owning the in-memory `dict[agent_id, last_status]` and the `status:<state>` wake-reason label (`monitor_tick` stays interval-only). <!-- completed: 2026-07-06T10:54 -->

COMMENT(programmer): Step 5 introduces 5 expected test breakages from the documented contract changes (Tester to fix in Step 7): `test_doctor.py` (×4, old tmux-block output → new `multiplexer` block) and `test_fleet_bootstrap.py::test_fleet_create_outside_tmux…` (old precondition string → "tmux or herdr session"). Also: routing `broker/messaging.py` and `monitor/loop.py` through `resolve_multiplexer()` makes those paths env-dependent — `test_inline_preview.py` and `test_loop.py` currently PASS only because this run's shell has `TMUX` set (so resolve returns tmux and the existing `TmuxMultiplexer` class-method monkeypatches apply). In a no-`TMUX` CI they would fail; Step 7 should make them env-independent (monkeypatch `resolve_multiplexer` or set `CAFLEET_MULTIPLEXER`/`TMUX` in the broker/monitor fixtures). Native-status branch is inert on tmux (interval echo byte-for-byte unchanged).

### Step 6: Persistence rename + migration

- [ ] `db/models.py` — rename the three `AgentPlacement` columns to `mux_*` and add `backend` (`server_default="tmux"`). <!-- completed: -->
- [ ] Author Alembic `0008` (`down_revision="0007"`): `batch_alter_table` rename the three columns + add `backend TEXT NOT NULL DEFAULT 'tmux'`. <!-- completed: -->
- [ ] Update every read/write site to the new names + set `backend=mux.name` on insert: `broker/agents.py` (register INSERT, `update_placement_pane_id` UPDATE), `broker/fleets.py` (director placement), `broker/members.py` (roster SELECTs), `broker/monitor.py` (pane-id SELECTs), `broker/messaging.py` (recipient pane SELECT), `broker/_shared.py` (`placement_dict`), `cli/member.py` (placement dict + `placement["mux_pane_id"]` reads), `output/formatters.py` (dict-key reads; display labels already neutral). <!-- completed: -->

### Step 7: Tests

- [ ] `tests/multiplexer/test_herdr.py` — monkeypatch the herdr `_run` dispatcher; assert the exact herdr argv for each Protocol method + `agent_status`/`wait_agent_status`, and `not_found`-tolerant teardown. <!-- completed: -->
- [ ] `tests/multiplexer/test_protocol.py` — assert `HerdrMultiplexer` satisfies `Multiplexer` and `AgentStateAware`, and that `TmuxMultiplexer` satisfies `Multiplexer` but **not** `AgentStateAware`. <!-- completed: -->
- [ ] Add resolver tests: override valid/invalid, and the auto-detect env matrix incl. both-set → raise and neither-set → raise (exact messages). <!-- completed: -->
- [ ] Add a monitor test: with an `AgentStateAware` fake, a `blocked`/`done` transition flags the agent due; on a non-capable backend the branch is inert. <!-- completed: -->
- [ ] Update **all tests and shared test helpers** referencing the placement columns for the renamed `mux_*` columns, the `backend` column, and the generalized `doctor` output — including `tests/db/test_alembic_smoke.py`, the `tests/cli/` suite (`test_member*.py`, `test_monitor.py`, `test_compact_echo.py`, `test_fleet_bootstrap.py`), the broker/formatter/doctor tests, and the shared helpers `tests/broker/_helpers.py` and `tests/cli/_member_helpers.py` (the suite fails to import otherwise). <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-06 | Initial draft |
