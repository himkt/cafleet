# Reorganize the cafleet Python Package Layout

**Status**: Approved
**Progress**: 11/27 tasks complete
**Last Updated**: 2026-06-11

## Overview

Reorganize `cafleet/src/cafleet` from a layout dominated by two fat modules (`cli.py`, 1315 lines; `broker.py`, 1246 lines) into domain subpackages: `broker/`, `cli/`, `output/`, and `webui/`. The reorganization is behavior-preserving — CLI surface, JSON shapes, exit codes, and error messages are unchanged — with small code cleanups allowed while moving (session-helper deduplication, private-helper renames). `cafleet/tests/` is reorganized into subfolders mirroring the new source layout in the same change.

## Success Criteria

- [ ] `broker.py`, `cli.py`, `output.py`, `server.py`, and `webui_api.py` no longer exist as flat modules; their contents live in the `broker/`, `cli/`, `output/`, and `webui/` subpackages per the placement tables below
- [ ] No module under `src/cafleet` exceeds ~650 lines (largest expected: `cli/member.py`)
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:format` all pass
- [ ] `cafleet --version` and `cafleet server` work after `mise //cafleet:install`; `mise //admin:build` emits assets into `src/cafleet/webui/dist/` and `mise //cafleet:build` packages them into the wheel
- [ ] The console entry point `cafleet = "cafleet.cli:cli"` is unchanged (the `cli` group is exposed from `cli/__init__.py`)
- [ ] A repo-wide grep finds no stale references to `cafleet.server:app`, `webui_api`, or the removed flat module paths in source, tests, docs, skills, rules, or mise tasks
- [ ] `cafleet/tests/` mirrors the source layout (`tests/broker/`, `tests/cli/`, `tests/output/`, `tests/webui/`, `tests/multiplexer/`, `tests/coding_agent/`, `tests/db/`) per the mapping table below

---

## Background

Current layout of `cafleet/src/cafleet` (line counts as of this writing):

| Path | Lines | Content |
|---|---|---|
| `cli.py` | 1315 | All seven command groups (`db`, `fleet`, `agent`, `message`, `member`, `server`, `doctor`) plus decorator machinery, spawn-prompt resolution, and rollback helpers |
| `broker.py` | 1246 | Fleet CRUD, agent registry + placement, member roster/activity, messaging (send/broadcast/poll/ack/cancel), and WebUI read queries |
| `output.py` | 374 | Wire-shape projections (`render_*`, `truncate_*`) mixed with text formatters (`format_*`) |
| `webui_api.py` | 127 | `/api/*` FastAPI router |
| `server.py` | 69 | FastAPI app factory + SPA static-files mount |
| `config.py` | 66 | `Settings` singleton |
| `coding_agent/`, `multiplexer/`, `db/` | — | Already clean subpackages |

Pain points: the two fat files force readers to scan 1200+ lines to find one command or one broker function, and unrelated concerns (e.g. WebUI timeline queries vs. message ACK transitions) share a namespace. `output.py` interleaves two distinct concerns. `server.py` and `webui_api.py` are two halves of one WebUI feature living as flat siblings.

**Constraint discovered during review — `webui/` name collision**: the built admin assets already land at `src/cafleet/webui/` (vite `outDir: '../cafleet/src/cafleet/webui'`), that path is gitignored (`.gitignore:16`), and the wheel includes `src/cafleet/webui/**/*`. Creating a Python package named `webui` therefore requires relocating the assets to `webui/dist/` (see Specification § webui).

**Constraint — monkeypatch seams**: tests patch broker functions as package attributes (`monkeypatch.setattr(broker, "verify_agent_fleet", ...)`) and conftest's `_patch_broker` fixture patches the single name `broker.get_sync_sessionmaker`. The split must preserve a single DB seam and keep consumer-boundary patches working (see Specification § Patch-seam contract).

---

## Specification

### Target layout

```
cafleet/src/cafleet/
├── __init__.py                (unchanged, empty)
├── config.py                  (unchanged)
├── broker/
│   ├── __init__.py            re-exports the full public API
│   ├── _shared.py             cross-submodule helpers + session context managers
│   ├── fleets.py              fleet CRUD
│   ├── agents.py              agent registry + placement
│   ├── members.py             member roster + activity proxies
│   ├── messaging.py           send/broadcast/poll/ack/cancel + inline-preview notify
│   └── queries.py             read-only task queries (inbox/sent/timeline/get_task)
├── cli/
│   ├── __init__.py            root `cli` group; registers all subcommands
│   ├── _helpers.py            shared decorators and guards
│   ├── _prompt.py             member spawn-prompt resolution
│   ├── db.py                  `db init`
│   ├── fleet.py               `fleet create/list/show/delete`
│   ├── agent.py               `agent register/list/show/deregister`
│   ├── message.py             `message send/broadcast/poll/ack/cancel/show`
│   ├── member.py              `member create/delete/list/capture/send-input/exec/ping`
│   ├── server.py              `server` command
│   └── doctor.py              `doctor` command
├── output/
│   ├── __init__.py            re-exports the full public API
│   ├── render.py              wire-shape projections, truncation, JSON, ANSI stripping
│   └── formatters.py          human-readable text formatters
├── webui/
│   ├── __init__.py            empty
│   ├── app.py                 FastAPI app factory + SPA mount (from server.py)
│   ├── api.py                 /api/* router (from webui_api.py)
│   └── dist/                  built admin assets (relocated; gitignored)
├── coding_agent/              (unchanged)
├── multiplexer/               (unchanged)
└── db/                        (unchanged)
```

### broker/ — function placement

Module docstrings move with their functions. The local `TmuxMultiplexer` import inside the notify helper stays local (it exists so test monkeypatches bind per-call).

| New module | Contents (from `broker.py`) |
|---|---|
| `broker/_shared.py` | `ADMINISTRATOR_KIND`; renamed cross-submodule helpers: `now_iso` (← `_now_iso`), `is_administrator` (← `_is_administrator`), `placement_dict` (← `_placement_dict`), `agent_is_active_in_fleet` (← `_agent_is_active_in_fleet`), `TASK_COLUMNS` (← `_TASK_COLUMNS`), `NOT_BROADCAST_SUMMARY` (← `_NOT_BROADCAST_SUMMARY`), `row_to_task_dict` (← `_row_to_task_dict`), `read_task` (← `_read_task`), `list_tasks_where` (← `_list_tasks_where`); new `read_session()` / `write_session()` context managers (see cleanup below) |
| `broker/fleets.py` | `_DIRECTOR_NAME`, `_DIRECTOR_DESCRIPTION`, `create_fleet`, `list_fleets`, `get_fleet`, `delete_fleet` |
| `broker/agents.py` | `register_agent`, `get_agent`, `list_agents`, `deregister_agent`, `verify_agent_fleet`, `list_fleet_agents`, `get_agent_names`, `update_placement_pane_id` |
| `broker/members.py` | `_base_members_select`, `list_members`, `list_members_with_activity`, `_idle_seconds` |
| `broker/messaging.py` | `_try_notify_recipient`, `_insert_task`, `_save_task`, `_unicast_task_dict`, `send_message`, `broadcast_message`, `poll_tasks`, `_transition_task_state`, `ack_task`, `cancel_task` |
| `broker/queries.py` | `list_inbox`, `list_sent`, `list_timeline`, `get_task` |

`broker/__init__.py` re-exports (with `__all__`): `ADMINISTRATOR_KIND`, `create_fleet`, `list_fleets`, `get_fleet`, `delete_fleet`, `register_agent`, `get_agent`, `list_agents`, `deregister_agent`, `update_placement_pane_id`, `list_members`, `list_members_with_activity`, `verify_agent_fleet`, `list_fleet_agents`, `get_agent_names`, `send_message`, `broadcast_message`, `poll_tasks`, `ack_task`, `cancel_task`, `list_inbox`, `list_sent`, `list_timeline`, `get_task`.

**Session-helper cleanup** (the one substantive cleanup): nearly every broker function repeats the two-line `sm = get_sync_sessionmaker()` / `with sm() as session[, session.begin()]:` boilerplate. `broker/_shared.py` replaces it with:

```python
from contextlib import contextmanager
from cafleet.db.engine import get_sync_sessionmaker

@contextmanager
def read_session():
    sm = get_sync_sessionmaker()
    with sm() as session:
        yield session

@contextmanager
def write_session():
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        yield session
```

All broker submodules use `with _shared.read_session() as session:` (reads) or `write_session()` (transactional writes). Because `get_sync_sessionmaker` is resolved from `_shared`'s module globals at call time, `cafleet.broker._shared.get_sync_sessionmaker` becomes the single DB patch seam.

### Patch-seam contract

| Seam | Rule |
|---|---|
| DB sessions | conftest's `_patch_broker` fixture changes its target from `broker` to `cafleet.broker._shared` (same attribute name `get_sync_sessionmaker`). No other DB seam exists. |
| Consumer boundary | `cli/` and `webui/` modules access broker and output **only** via package-namespace attribute access (`from cafleet import broker, output` … `broker.send_message(...)`). Existing test patches of `broker.<name>` / `webui_api.broker.<name>` (→ `webui.api.broker.<name>`) keep intercepting these calls because they patch the package attribute. |
| `_shared` helpers | Broker submodules access `_shared` helpers **attribute-style** (`from cafleet.broker import _shared` … `_shared.now_iso()`), never `from cafleet.broker._shared import now_iso`. Attribute access resolves through `_shared`'s module globals at call time, so `cafleet.broker._shared.<name>` is a single patchable seam per helper regardless of which submodule calls it. |
| Broker-internal calls | Cross-submodule calls to **public** broker functions import directly (e.g. `agents.py` does `from cafleet.broker.fleets import get_fleet`); these are not interceptable via `cafleet.broker.<name>` package attributes. Two existing tests patch broker-internal names on the flat module and get new targets: `tests/test_broker_registry.py:115` (`broker._now_iso`, consumed by `register_agent`/`deregister_agent` in `agents.py`) → `cafleet.broker._shared.now_iso` (intercepts via the attribute-style convention above); `tests/test_fleet_bootstrap.py:124` (`broker.AgentPlacement`, consumed by `create_fleet`) → `cafleet.broker.fleets.AgentPlacement`. |
| tmux | Tests patch `TmuxMultiplexer` methods at class level and `cafleet.multiplexer.tmux._run`; both are unaffected by this reorganization. |

### cli/ — placement

Each submodule defines its own standalone `@click.group()` (or `@click.command()`); `cli/__init__.py` defines the root `cli` group (global `--json` / `--fleet-id` / version option, `ctx.obj` setup) and registers everything via `cli.add_command(...)`. The `if __name__ == "__main__":` block is dropped (the console script is the only supported entry). The entry point string `cafleet.cli:cli` keeps working unchanged because `cli/__init__.py` exposes `cli`.

| New module | Contents (from `cli.py`) |
|---|---|
| `cli/__init__.py` | root `cli` group; `add_command` registrations for `db`, `fleet`, `agent`, `message`, `member`, `server`, `doctor` |
| `cli/_helpers.py` | renamed shared helpers: `ensure_tmux_or_die`, `full_flag`, `quiet_flag`, `full_flag_with_help`, `quiet_flag_with_help`, `director_member_options`, `require_fleet_id`, `client_command` (← their `_`-prefixed forms) |
| `cli/db.py` | `db` group, `init` command, `_sync_db_url` (only used by `db init`) |
| `cli/fleet.py` | `fleet` group: `fleet_create`, `fleet_list`, `fleet_show`, `fleet_delete` |
| `cli/agent.py` | `agent` group: `agent_register`, `agent_list`, `agent_show`, `agent_deregister` |
| `cli/message.py` | `message` group: `message_send`, `message_broadcast`, `message_poll`, `message_ack`, `message_cancel`, `message_show` |
| `cli/member.py` | `member` group: `member_create`, `member_delete`, `member_list`, `member_capture`, `member_send_input`, `member_exec`, `member_ping`; member-local helpers `_PLACEMENT_MISSING_DEFAULT`, `_require_member_pane`, `_load_authorized_member`, `_deregister_with_warning`, `_rollback_register`, `_emit_member_delete_output` |
| `cli/_prompt.py` | renamed spawn-prompt machinery: `MEMBER_PROMPT_TEMPLATE` (← `_MEMBER_PROMPT_TEMPLATE`), `read_prompt_file` (← `_read_prompt_file`), `resolve_prompt` (← `_resolve_prompt`) |
| `cli/server.py` | `server` command; the `uvicorn.run` target string changes to `"cafleet.webui.app:app"` |
| `cli/doctor.py` | `doctor` command |

Expected post-split sizes: `member.py` ~620 lines (largest), `message.py` ~120, all others < 250.

### output/ — placement

| New module | Contents (from `output.py`) |
|---|---|
| `output/render.py` | `_TRUNCATION_SUFFIX`, `_ANSI_ESCAPE_RE`, `strip_ansi`, `format_json`, `truncate_text`, `truncate_task_text`, `render_task`, `render_tasks_in_result`, `_render_item`, `render_agent`, `render_agents_in_result`, `_render_agent_item` |
| `output/formatters.py` | `format_register`, `format_task`, `format_indexed_list`, `format_agent`, `format_fleet_create`, `format_member`, `format_member_list`, `format_member_list_activity`, `_AGENT_ID_COLUMN_WIDTH`, `_agent_id_for_column`, `_format_iso_hms`, `_format_idle` |

`formatters.py` imports `render_task` / `truncate_text` from `render.py`. `output/__init__.py` re-exports (with `__all__`) every public name from both modules so `from cafleet import output` + `output.<name>` access in `cli/` keeps working.

### webui/ — placement and asset relocation

| New module | Contents |
|---|---|
| `webui/app.py` | from `server.py`: `SPAStaticFiles`, `default_webui_dist_dir`, `create_app`, module-level `app` |
| `webui/api.py` | from `webui_api.py`: `webui_router`, `get_webui_fleet`, `_format_messages`, `SendMessageRequest`, all endpoint functions |
| `webui/__init__.py` | empty |
| `webui/dist/` | built admin assets (relocated from `src/cafleet/webui/`) |

Asset-pipeline changes required by the relocation:

| File | Change |
|---|---|
| `admin/vite.config.ts` | `outDir: '../cafleet/src/cafleet/webui'` → `'../cafleet/src/cafleet/webui/dist'` |
| `.gitignore` (line 16) | `cafleet/src/cafleet/webui/` → `cafleet/src/cafleet/webui/dist/` |
| `cafleet/pyproject.toml` | wheel `include`: `"src/cafleet/webui/**/*"` → `"src/cafleet/webui/dist/**/*"` (the `.py` files are covered by the `packages` directive) |
| `webui/app.py` | `default_webui_dist_dir()` returns `Path(__file__).resolve().parent / "dist"` |
| `cafleet/mise.toml` (`dev` task) | `uvicorn cafleet.server:app` → `uvicorn cafleet.webui.app:app` |

The stale pre-relocation asset files at `src/cafleet/webui/{index.html,favicon.svg,assets/}` are untracked (gitignored); delete them locally during implementation so only `dist/` remains.

### tests/ — mirror mapping

Each new subdirectory gets an `__init__.py` (the tests tree is already a package; per-directory `__init__.py` keeps duplicate basenames like `test_protocol.py` unambiguous). `tests/conftest.py`, `tests/_helpers.py`, and `tests/__init__.py` stay at the root. Intra-test imports update accordingly (e.g. `tests._broker_helpers` → `tests.broker._helpers`).

| Current | New |
|---|---|
| `_broker_helpers.py` | `broker/_helpers.py` |
| `test_broker_administrator.py` | `broker/test_administrator.py` |
| `test_broker_inline_preview.py` | `broker/test_inline_preview.py` |
| `test_broker_member_activity.py` | `broker/test_member_activity.py` |
| `test_broker_messaging.py` | `broker/test_messaging.py` |
| `test_broker_registry.py` | `broker/test_registry.py` |
| `test_broker_typed_columns.py` | `broker/test_typed_columns.py` |
| `test_broker_webui.py` | `broker/test_queries.py` |
| `test_fleet_bootstrap.py` | `broker/test_fleet_bootstrap.py` |
| `test_fleet_list_director.py` | `broker/test_fleet_list_director.py` |
| `_member_cli_helpers.py` | `cli/_member_helpers.py` |
| `test_cli_agent.py` | `cli/test_agent.py` |
| `test_cli_client_command.py` | `cli/test_client_command.py` |
| `test_cli_compact_echo.py` | `cli/test_compact_echo.py` |
| `test_cli_doctor.py` | `cli/test_doctor.py` |
| `test_cli_fleet_bootstrap.py` | `cli/test_fleet_bootstrap.py` |
| `test_cli_fleet_flag.py` | `cli/test_fleet_flag.py` |
| `test_cli_help_budget.py` | `cli/test_help_budget.py` |
| `test_cli_member.py` | `cli/test_member.py` |
| `test_cli_member_capture_defaults.py` | `cli/test_member_capture_defaults.py` |
| `test_cli_member_delete.py` | `cli/test_member_delete.py` |
| `test_cli_member_exec.py` | `cli/test_member_exec.py` |
| `test_cli_member_list_activity.py` | `cli/test_member_list_activity.py` |
| `test_cli_member_ping.py` | `cli/test_member_ping.py` |
| `test_cli_member_prompt_template.py` | `cli/test_member_prompt_template.py` |
| `test_cli_member_send_input.py` | `cli/test_member_send_input.py` |
| `test_cli_message.py` | `cli/test_message.py` |
| `test_cli_message_truncation.py` | `cli/test_message_truncation.py` |
| `test_cli_version.py` | `cli/test_version.py` |
| `test_fleet_cli.py` | `cli/test_fleet.py` |
| `test_server_cli.py` | `cli/test_server.py` |
| `test_output.py` | `output/test_output.py` |
| `test_output_compact_formatters.py` | `output/test_compact_formatters.py` |
| `test_output_indexed_list.py` | `output/test_indexed_list.py` |
| `test_output_render_agent.py` | `output/test_render_agent.py` |
| `test_output_render_broadcast_summary.py` | `output/test_render_broadcast_summary.py` |
| `test_output_render_task.py` | `output/test_render_task.py` |
| `test_output_truncation_settings.py` | `output/test_truncation_settings.py` |
| `test_server_routing.py` | `webui/test_routing.py` |
| `test_webui_api_format.py` | `webui/test_api_format.py` |
| `test_multiplexer_protocol.py` | `multiplexer/test_protocol.py` |
| `test_multiplexer_tmux.py` | `multiplexer/test_tmux.py` |
| `test_multiplexer_tmux_send_helpers.py` | `multiplexer/test_tmux_send_helpers.py` |
| `test_multiplexer_tmux_send_inline_preview.py` | `multiplexer/test_tmux_send_inline_preview.py` |
| `test_coding_agent_opencode.py` | `coding_agent/test_opencode.py` |
| `test_coding_agent_protocol.py` | `coding_agent/test_protocol.py` |
| `test_opencode_preset.py` | `coding_agent/test_opencode_preset.py` |
| `test_alembic_smoke.py` | `db/test_alembic_smoke.py` |
| `test_db_init.py` | `db/test_init.py` |

### Out of scope

- `coding_agent/`, `multiplexer/`, `db/`, `config.py`, and the root `__init__.py` are untouched.
- No behavior changes: CLI text/JSON output, exit codes, error-message strings, HTTP responses, and DB schema are byte-identical. The only string-level changes are the uvicorn target (`cafleet.webui.app:app`) and the asset directory.
- The admin frontend source (`admin/`) is unchanged except the vite `outDir`.
- Renames are confined to private helpers promoted into shared modules (tables above); every public API name is unchanged.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `docs/concepts/overview.md` (mermaid diagram: `broker.py` → `broker/` package, `server.py` / `webui_api.py` → `webui/app.py` + `webui/api.py`) <!-- completed: 2026-06-11T12:23 -->
- [x] Update `docs/api/broker.md` to describe the `broker/` package layout (submodule table + re-export contract) <!-- completed: 2026-06-11T12:23 -->
- [x] Sweep the rest of `docs/` (`index.md`, `spec/cli-options.md`, `get-started/`, `how-to/`, `reference/`, remaining `concepts/` pages) for references to `broker.py`, `cli.py`, `output.py`, `server.py`, `webui_api.py`, `cafleet.server:app`, or `src/cafleet/webui` asset paths, and update each <!-- completed: 2026-06-11T12:23 -->
- [x] Update `.claude/rules/commands.md`: `//cafleet:dev` description (`uvicorn cafleet.webui.app:app`) and the test-selector example path (`tests/test_fleet_cli.py::…` → `tests/cli/test_fleet.py::…`) — applied by the Director (member harness denies writes under `.claude/`) <!-- completed: 2026-06-11T12:25 -->
- [x] Sweep `skills/*/SKILL.md` and skill reference files for module-path references and update any that name the flat modules — sweep found zero references to the flat modules; the only `cafleet/src/cafleet/...` mentions are the fictional `cli/main.py:142` pointer-schema examples, which already read as subpackage paths and stay valid <!-- completed: 2026-06-11T12:23 -->
- [x] Confirm `README.md` consistency (it currently contains no module-path references; update only if the sweep finds drift) — sweep confirmed zero module-path references; no change needed <!-- completed: 2026-06-11T12:23 -->

### Step 2: broker/ package

- [x] Create `broker/_shared.py` with the renamed helpers and the `read_session` / `write_session` context managers <!-- completed: 2026-06-11T12:44 -->
- [x] Create `broker/fleets.py`, `broker/agents.py`, `broker/members.py`, `broker/messaging.py`, `broker/queries.py` per the placement table, converting all session boilerplate to `_shared` context managers <!-- completed: 2026-06-11T12:44 -->
- [x] Create `broker/__init__.py` re-exporting the full public API with `__all__`; delete `broker.py` (deletion dispatched by the Director — `rm` denied in the Programmer harness) <!-- completed: 2026-06-11T12:46 -->
- [x] Update test references to relocated/renamed private broker names: `tests/test_fleet_bootstrap.py:9` and `tests/test_broker_administrator.py:11` import `is_administrator` from `cafleet.broker._shared` (← `_is_administrator` from `cafleet.broker`); `tests/test_broker_typed_columns.py:73` calls `_unicast_task_dict` via `cafleet.broker.messaging` (← `broker._unicast_task_dict`); `tests/test_broker_registry.py:115` patches `cafleet.broker._shared.now_iso` (← `broker._now_iso`); `tests/test_fleet_bootstrap.py:124` patches `cafleet.broker.fleets.AgentPlacement` (← `broker.AgentPlacement`) — per the Patch-seam contract (Tester commits 4b94509 + 6dd2166) <!-- completed: 2026-06-11T12:44 -->
- [x] Update `tests/conftest.py` `_patch_broker` to target `cafleet.broker._shared` and run `mise //cafleet:test` to confirm the suite is green before proceeding — 711 passed (Tester commit 4b94509) <!-- completed: 2026-06-11T12:44 -->

### Step 3: output/ package

- [ ] Create `output/render.py` and `output/formatters.py` per the placement table; `output/__init__.py` re-exports with `__all__`; delete `output.py` <!-- completed: -->
- [ ] Run `mise //cafleet:test` to confirm green <!-- completed: -->

### Step 4: webui/ package and asset pipeline

- [ ] Create `webui/app.py` (from `server.py`, `default_webui_dist_dir` → `parent / "dist"`) and `webui/api.py` (from `webui_api.py`); delete `server.py` and `webui_api.py` <!-- completed: -->
- [ ] Update `admin/vite.config.ts` `outDir`, `.gitignore`, `cafleet/pyproject.toml` wheel include, `cafleet/mise.toml` dev task, and the `server` command's uvicorn target (still in flat `cli.py` at this step); remove the stale untracked assets at `src/cafleet/webui/{index.html,favicon.svg,assets/}` <!-- completed: -->
- [ ] Update `tests/test_server_cli.py` / `tests/test_server_routing.py` / `tests/test_webui_api_format.py` imports and the asserted uvicorn target; run `mise //cafleet:test` and `mise //admin:build` to confirm assets land in `webui/dist/` <!-- completed: -->

### Step 5: cli/ package

- [ ] Create `cli/_helpers.py` and `cli/_prompt.py` with the renamed helpers per the placement tables <!-- completed: -->
- [ ] Create `cli/db.py`, `cli/fleet.py`, `cli/agent.py`, `cli/message.py`, `cli/member.py`, `cli/server.py`, `cli/doctor.py` per the placement table <!-- completed: -->
- [ ] Create `cli/__init__.py` with the root group and `add_command` registrations; delete `cli.py`. Do NOT smoke-test via `uv run cafleet --version` (`.claude/rules/commands.md` forbids direct `uv run cafleet` invocations); verify group wiring via `mise //cafleet:test` — CliRunner covers entry-point resolution <!-- completed: -->
- [ ] Update test imports that reference `cafleet.cli` internals (helpers, prompt machinery) to the new submodule paths; run `mise //cafleet:test` <!-- completed: -->

### Step 6: tests mirror

- [ ] Create `tests/{broker,cli,output,webui,multiplexer,coding_agent,db}/__init__.py` and move/rename every test file per the mapping table (`git mv`) <!-- completed: -->
- [ ] Update intra-test imports (`tests._broker_helpers` → `tests.broker._helpers`, `tests._member_cli_helpers` → `tests.cli._member_helpers`) <!-- completed: -->
- [ ] Run `mise //cafleet:test` to confirm the full suite passes from the new layout <!-- completed: -->

### Step 7: Verification

- [ ] Run `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` — all green <!-- completed: -->
- [ ] Run `mise //admin:build`, then `mise //cafleet:build`; inspect the wheel to confirm `webui/dist/` assets and all new subpackages are included <!-- completed: -->
- [ ] Run `mise //cafleet:install` (editable reinstall), then smoke-test `cafleet --version`. For the server: start `cafleet server` in the background, `sleep 2`, then confirm `/` renders via `bun run agent-browser open http://127.0.0.1:8000/` from the repo root (retry with `sleep N` + `open` if the server is not up yet — the `agent-browser wait` family is off-limits per `.claude/rules/commands.md`), then stop the background server <!-- completed: -->
- [ ] Repo-wide grep for stale references (`broker.py`, `cli.py` as module paths, `output.py`, `server.py`, `webui_api`, `cafleet.server:app`, old test paths) across source, tests, docs, skills, rules, mise tasks — zero hits <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-11 | Initial draft |
