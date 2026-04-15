# Add `cafleet server` CLI subcommand to launch the admin WebUI server

**Status**: Approved
**Progress**: 5/22 tasks complete
**Last Updated**: 2026-04-15

## Overview

Add a `cafleet server` subcommand that launches the existing admin WebUI FastAPI app via uvicorn with `--host` / `--port` flags (no other options). Remove the `if __name__ == "__main__"` block from `cafleet/src/cafleet/server.py`, rewrite `mise //cafleet:dev` to call `uvicorn` directly (not via `cafleet server`), tighten `Settings.broker_host` default from `0.0.0.0` to `127.0.0.1` to match CAFleet's local-only stance, and wire `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` env-var overrides via pydantic `validation_alias` so the CAFLEET_ prefix is consistent with `CAFLEET_DATABASE_URL`.

## Success Criteria

- [ ] `cafleet server [--host <addr>] [--port <int>]` starts the admin WebUI FastAPI app via uvicorn and serves `/ui/` + `/ui/api/*` identically to the current `mise //cafleet:dev`
- [ ] `--host` defaults to `127.0.0.1` and `--port` defaults to `8000` (sourced from `settings.broker_host` / `settings.broker_port`)
- [ ] `Settings.broker_host` default is `127.0.0.1` (was `0.0.0.0`)
- [ ] `Settings.broker_host` reads `CAFLEET_BROKER_HOST` and `Settings.broker_port` reads `CAFLEET_BROKER_PORT` via explicit `validation_alias` (today they read `BROKER_HOST` / `BROKER_PORT`; this change aligns with `CAFLEET_DATABASE_URL`)
- [ ] `cafleet server` does **not** require `--session-id`; supplying one is silently accepted (matches the `db init` / `session *` pattern)
- [ ] `if __name__ == "__main__"` block is removed from `cafleet/src/cafleet/server.py`; module-level `import uvicorn`, `from cafleet.config import settings`, `import logging`, and the dead `logger` are all removed; `app = create_app()` at module scope is preserved
- [ ] `_default_webui_dist_dir()` is renamed to `default_webui_dist_dir()` (public) since `create_app` and tests now consume it across module boundaries
- [ ] `cafleet/mise.toml` `[tasks.dev]` runs `uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` (direct uvicorn, **not** `cafleet server`) — both paths coexist as independent entry points with identical explicit host/port
- [ ] On startup, if the bundled WebUI dist dir does not exist, a one-line warning is emitted to stderr: `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.`. The warning fires from `create_app()` so every startup path (`cafleet server`, `mise //cafleet:dev`, and any `uv run uvicorn cafleet.server:app`) sees it
- [ ] Port-in-use errors are NOT wrapped — uvicorn's native `OSError` propagates so the user sees the stock uvicorn/click traceback
- [ ] Smoke tests via `CliRunner`: (a) `cafleet server --help` exits 0 and shows both flags; (b) flag parsing succeeds with valid values; (c) `cafleet --session-id <uuid> server --help` is silently accepted; (d) missing WebUI dist produces the warning via `create_app()`; (e) `Settings().broker_host == "127.0.0.1"` default assertion
- [ ] Docs updated FIRST (before code): `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `README.md`, `cafleet/mise.toml`, `.claude/rules/commands.md`, `.claude/skills/cafleet/SKILL.md`
- [ ] `mise //cafleet:test`, `mise //:lint`, `mise //:format`, `mise //:typecheck` all pass

---

## Background

### Current state

| File | Current behavior |
|---|---|
| `cafleet/src/cafleet/server.py` | Defines `create_app()`, mounts `webui_router` + SPA static files; `if __name__ == "__main__"` calls `uvicorn.run("cafleet.server:app", host=settings.broker_host, port=settings.broker_port, reload=True)`. Also has a dead `logger` and unused `import logging` at module scope. |
| `cafleet/mise.toml` `[tasks.dev]` | `uv run src/cafleet/server.py` — relies on the `__main__` block |
| `cafleet/src/cafleet/config.py` | `Settings.broker_host: str = "0.0.0.0"`, `broker_port: int = 8000`. No `validation_alias` on either field, so with `env_prefix=""` and `populate_by_name=True` pydantic-settings reads the bare names `BROKER_HOST` / `BROKER_PORT` from the environment (case-insensitive) — **not** `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`. |
| `cafleet/src/cafleet/cli.py` | Click group with `db`, `session`, `member` subgroups and flat client commands; no `server` subcommand |

### Why add `cafleet server`

The CLI is the advertised entry point for every user interaction (registry, messaging, member management). The admin WebUI server is the one operation that still requires `mise` or a raw `python src/cafleet/server.py` call. A first-class `cafleet server` command:

- Makes the server launcher discoverable via `cafleet --help`
- Lets users install the `cafleet` wheel and launch the server on any host without cloning the repo or installing `mise`
- Keeps `cafleet` self-contained as the sole CLI surface

### Why mise dev calls uvicorn directly (not `cafleet server`)

Discussed and decided with the user:

- Contributors restart uvicorn manually between edits. Avoiding `--reload` keeps the dev process deterministic and simpler to debug, at the cost of manual restarts. `--reload` is therefore unnecessary for the dev workflow.
- Introducing delegation (`mise //cafleet:dev` → `cafleet server`) would add an indirection without adding value
- Both `cafleet server` and `mise //cafleet:dev` run plain uvicorn without `--reload`; they are **independent code paths** that happen to launch the same FastAPI app

This is a deliberate "two entry points, one app" design — users pick whichever invocation matches their workflow.

### Why tighten `broker_host` default to `127.0.0.1`

CAFleet is documented (README line 5) as a local-only tool. The `0.0.0.0` default was a Docker/VM convenience that no longer matches the project's stance. Users who genuinely need external binding can still pass `--host 0.0.0.0` or set `CAFLEET_BROKER_HOST=0.0.0.0` (after the alias is wired in this cycle).

### Why add `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` aliases

Today `Settings.broker_host` and `Settings.broker_port` read `BROKER_HOST` / `BROKER_PORT` from the environment because `env_prefix=""` and there is no `validation_alias`. Documenting `CAFLEET_BROKER_HOST` without this code change would be a lie. Options considered:

| Option | Trade-off |
|---|---|
| (a) Add `validation_alias="CAFLEET_BROKER_HOST"` / `validation_alias="CAFLEET_BROKER_PORT"` | Small code change; consistent with `CAFLEET_DATABASE_URL`; matches user mental model of project-scoped env vars |
| (b) Document the actual names `BROKER_HOST` / `BROKER_PORT` | Zero code change; but creates a naming inconsistency with `CAFLEET_DATABASE_URL` |

Option (a) is chosen. The alias keeps the `CAFLEET_` prefix uniform across every env var the project exposes.

---

## Specification

### CLI surface

```
cafleet server [--host <addr>] [--port <int>]
```

| Flag | Default | Source | Notes |
|---|---|---|---|
| `--host` | `127.0.0.1` | `settings.broker_host` | Bind address; override via flag or `CAFLEET_BROKER_HOST` env var |
| `--port` | `8000` | `settings.broker_port` | Bind port; override via flag or `CAFLEET_BROKER_PORT` env var |

**No other flags.** `--reload`, `--workers`, `--log-level`, `--webui-dist-dir` are explicitly **not** exposed. Users who need them invoke uvicorn directly (that is exactly what `mise //cafleet:dev` does).

### Session-id gating

`cafleet server` joins the "does NOT require `--session-id`" list alongside `db init` and `session *`. Supplying `--session-id` to `cafleet server` is silently accepted and ignored (matches the existing "Provided but not required" rule in `docs/spec/cli-options.md`).

### Command behavior

1. Read `settings.broker_host` and `settings.broker_port` from `cafleet.config.settings` (env-var-aware via pydantic-settings + `validation_alias`).
2. If `--host` / `--port` are provided, they override the settings values.
3. Call `uvicorn.run("cafleet.server:app", host=host, port=port)`. No `reload`, no `workers`, no custom `log_level` — use uvicorn defaults.
4. The missing-WebUI-dist warning is emitted by `create_app()` at app-construction time (see "server.py changes" below). The CLI handler itself performs no disk check.
5. Port-in-use and other uvicorn startup errors propagate natively (no try/except wrapping). The user sees the stock uvicorn traceback or `OSError: [Errno 98] Address already in use`.

### Click implementation sketch

```python
@cli.command("server")
@click.option(
    "--host",
    default=None,
    help="Bind address (default: settings.broker_host = 127.0.0.1).",
)
@click.option(
    "--port",
    default=None,
    type=int,
    help="Bind port (default: settings.broker_port = 8000).",
)
def server(host: str | None, port: int | None) -> None:
    """Start the admin WebUI FastAPI server."""
    import uvicorn

    from cafleet.config import settings

    effective_host = host or settings.broker_host
    effective_port = port if port is not None else settings.broker_port

    uvicorn.run(
        "cafleet.server:app",
        host=effective_host,
        port=effective_port,
    )
```

The `import uvicorn` and `from cafleet.config import settings` stay inside the function to avoid paying the uvicorn import cost for every `cafleet <other-subcommand>` invocation.

### `server.py` changes

1. Delete the `if __name__ == "__main__"` block (lines 58-64).
2. Delete the now-unused module-level `import uvicorn` (line 11).
3. Delete the already-dead `import logging` (line 8) and `logger = logging.getLogger(__name__)` (line 19).
4. Delete the now-unused `from cafleet.config import settings` (line 16) — after the `__main__` block is gone, nothing at module scope uses it, and ruff F401 would fail otherwise.
5. Rename `_default_webui_dist_dir()` to `default_webui_dist_dir()` (drop the leading underscore). It is no longer module-private: `create_app()` and `cafleet/tests/test_server_cli.py` both consume it.
6. Keep `app = create_app()` at module scope so both `uv run uvicorn cafleet.server:app` (the new mise dev command) and `uvicorn.run("cafleet.server:app", ...)` (the `cafleet server` code path) can import this symbol.
7. Add the missing-WebUI-dist warning to `create_app()`. The warning fires **only when `webui_dist_dir` is None** — i.e. the default path is being used. Explicit overrides (tests pass `webui_dist_dir="/tmp/..."`) skip the warning, keeping test output clean while giving every real startup path the signal.

Resulting `create_app()` shape:

```python
def create_app(webui_dist_dir: str | None = None) -> FastAPI:
    app = FastAPI(title="CAFleet Admin", version="0.1.0")
    app.include_router(webui_router)

    emit_warning_if_missing = webui_dist_dir is None
    if webui_dist_dir is None:
        webui_dist_dir = str(default_webui_dist_dir())
    dist_path = Path(webui_dist_dir)

    if emit_warning_if_missing and not dist_path.exists():
        print(
            "warning: admin WebUI is not built. /ui/ will return 404. "
            "Run 'mise //admin:build'.",
            file=sys.stderr,
        )

    if dist_path.exists():
        app.mount(
            "/ui",
            SPAStaticFiles(directory=str(dist_path)),
            name="webui",
        )

    return app
```

Add `import sys` to `server.py` for `sys.stderr`. This is the only new import.

### `config.py` change

```python
from pydantic import Field

class Settings(BaseSettings):
    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="CAFLEET_DATABASE_URL",
    )
    broker_host: str = Field(
        default="127.0.0.1",                      # was "0.0.0.0"
        validation_alias="CAFLEET_BROKER_HOST",   # new
    )
    broker_port: int = Field(
        default=8000,
        validation_alias="CAFLEET_BROKER_PORT",   # new
    )
    broker_base_url: str = "http://localhost:8000"
    ...
```

- `broker_host` default flips `0.0.0.0` → `127.0.0.1`.
- `broker_host` gains `validation_alias="CAFLEET_BROKER_HOST"`.
- `broker_port` gains `validation_alias="CAFLEET_BROKER_PORT"`.
- `broker_base_url` is unchanged (not in scope; nothing in this cycle uses it via env var).

Users who need external binding continue to use `CAFLEET_BROKER_HOST=0.0.0.0` or `cafleet server --host 0.0.0.0`.

### `mise.toml` change

```toml
# cafleet/mise.toml
[tasks.dev]
run = "uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000"
description = "Start the admin WebUI server on 127.0.0.1:8000 (no hot-reload; restart manually between edits)"
```

`--host 127.0.0.1` is passed explicitly rather than relying on uvicorn's own default. Rationale: once `CAFLEET_BROKER_HOST` exists as an override path for `cafleet server`, relying on uvicorn's implicit default in mise dev would create silent divergence (user sets `CAFLEET_BROKER_HOST=0.0.0.0` expecting both entry points to honor it — but mise dev ignores the env var because it bypasses pydantic settings). Hardcoding `--host 127.0.0.1` in mise dev documents "mise dev is intentionally localhost-only for the dev loop"; users who need external binding use `cafleet server` (or invoke uvicorn directly with their own flags).

### Documentation updates

| File | Change |
|---|---|
| `ARCHITECTURE.md` | In the Component Layout section, note `cafleet server` as the packaged launcher alongside `mise //cafleet:dev`. Update line 227 (`/ui/` 404 note) to reference the new `create_app()` warning. Change any reference to `broker_host: 0.0.0.0` to `127.0.0.1`. Note the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` env var names. |
| `docs/spec/cli-options.md` | Add `server` to the "Subcommands that do NOT require `--session-id`" list (line 33). Add a new "## `cafleet server` — Admin WebUI Server" section documenting `--host` and `--port` defaults and the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` env-var override paths. |
| `README.md` | Update line 75 from "Start `mise //cafleet:dev` only if you want the admin WebUI" to "Start `cafleet server` (or `mise //cafleet:dev` from a repo clone) only if you want the admin WebUI". Update line 163 and line 236 similarly. Add a brief `cafleet server` example in the Quick Start. |
| `cafleet/mise.toml` | Rewrite `[tasks.dev]` as above (with explicit `--host 127.0.0.1 --port 8000`). |
| `.claude/rules/commands.md` | Update line 12 from "Start admin WebUI server: `mise //cafleet:dev`" to include the `cafleet server` alternative. Clarify that `mise //cafleet:dev` calls uvicorn directly (no delegation to `cafleet server`). |
| `.claude/skills/cafleet/SKILL.md` | Add a new "### Server" subsection in the Command Reference documenting `cafleet server --host <addr> --port <int>`. Clarify it does not need `--session-id`. |

### Removed surface

| Item | Reason |
|---|---|
| `if __name__ == "__main__"` block in `cafleet/src/cafleet/server.py` (lines 58-64) | Single source of truth for "start the server" is now `cafleet server` (or `mise //cafleet:dev` calling uvicorn directly). `python server.py` is not a user-facing entry point. |
| Module-level `import uvicorn` in `server.py` | Uvicorn is now imported inside the CLI handler. `server.py` no longer calls `uvicorn.run` at module scope. |
| `from cafleet.config import settings` in `server.py` | Only the `__main__` block used it; becomes unused after deletion and would fail ruff F401. |
| `import logging` + `logger = logging.getLogger(__name__)` in `server.py` | Already dead code; no call site. Cleaned up in the same cycle. |
| `broker_host: str = "0.0.0.0"` default in `config.py` | Replaced by `"127.0.0.1"` to match CAFleet's local-only stance. |
| Private `_default_webui_dist_dir()` symbol | Renamed to public `default_webui_dist_dir()` since tests and (optionally) future callers now consume it across module boundaries. |

### Tests

New file: `cafleet/tests/test_server_cli.py`, mirroring the style of `test_cli_session_flag.py` and `test_session_cli.py`.

| Test class | Coverage |
|---|---|
| `TestServerCommandHelp` | `cafleet server --help` exits 0 and the help text lists `--host` and `--port` |
| `TestServerCommandFlagParsing` | With `uvicorn.run` monkey-patched to capture args: `cafleet server` passes `host=settings.broker_host, port=settings.broker_port`; `cafleet server --host 0.0.0.0 --port 9000` passes `host="0.0.0.0", port=9000` |
| `TestServerDoesNotRequireSessionId` | `cafleet server --help` succeeds without `--session-id`; `cafleet --session-id <uuid> server --help` is silently accepted; `--help` is used so `uvicorn.run` is never called |
| `TestWebUIDistWarning` | With `default_webui_dist_dir()` monkey-patched to return a non-existent `tmp_path`, `create_app()` (called directly, not via the CLI) emits the "admin WebUI is not built" warning to stderr. With an explicit `webui_dist_dir=<tmp_path>` override, no warning is emitted (proves the `webui_dist_dir is None` gate). With an existing dist directory, no warning is emitted. |
| `TestBrokerHostDefault` | `Settings().broker_host == "127.0.0.1"` (new default assertion). Covers the `config.py` change. Placed here since the project has no `test_config.py` today. |

No functional test spins up a real uvicorn server — smoke tests only, per Director's Q5(a) answer.

### Edge cases

| Case | Behavior |
|---|---|
| Port already in use | `OSError` from uvicorn propagates; the click handler does not wrap it (per Q4a) |
| `--port` value not a valid int | Click's built-in `type=int` validation produces the standard "Invalid value for '--port'" error and exits 2 |
| `--host` value is a malformed address | uvicorn surfaces the bind error; we do not pre-validate |
| `CAFLEET_BROKER_HOST` env var set AND `--host` flag passed | Flag wins (click flag is evaluated first; settings is only a fallback) |
| WebUI dist dir exists but is empty | The warning is NOT emitted (directory existence is the trigger, matching the existing `create_app()` mount-check). `/ui/` will 404 from `SPAStaticFiles` at request time. |
| `create_app(webui_dist_dir="/some/explicit/path")` (test override) where path is missing | No warning emitted — the gate `webui_dist_dir is None` catches only the default-path case so tests stay quiet |
| `--session-id <uuid>` supplied | Silently accepted, per the existing "Provided but not required" rule |

---

## Implementation

> Documentation must be updated **before** any code change (per `.claude/rules/design-doc-numbering.md`).
> Task format: `- [x] Done task <!-- completed: 2026-04-14T14:30 -->`

### Step 1: Documentation — Top-level docs

- [x] Update `ARCHITECTURE.md`: in Component Layout, note `cafleet server` as the packaged launcher; update line 227 `/ui/` 404 note to reference the new `create_app()` warning; change any `broker_host: 0.0.0.0` mention to `127.0.0.1`; document `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` env-var names. <!-- completed: 2026-04-15T11:35 -->
- [x] Update `docs/spec/cli-options.md`: add `server` to the "Subcommands that do NOT require `--session-id`" list; add a new "## `cafleet server` — Admin WebUI Server" section documenting `--host`, `--port`, defaults, env-var overrides (including the new `validation_alias` wiring), and startup warning. <!-- completed: 2026-04-15T11:35 -->
- [x] Update `README.md`: rewrite references to `mise //cafleet:dev` on lines 75, 163, 236 to mention the `cafleet server` alternative; add a short Quick Start snippet. <!-- completed: 2026-04-15T11:35 -->

### Step 2: Documentation — mise and rules

- [x] Update `cafleet/mise.toml` `[tasks.dev]` to `run = "uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000"` with a matching description. <!-- completed: 2026-04-15T11:35 -->
- [x] Update `.claude/rules/commands.md` line 12 to note both `cafleet server` and `mise //cafleet:dev` as entry points, and clarify that mise dev does NOT delegate to `cafleet server`. <!-- completed: 2026-04-15T11:35 -->

### Step 3: Documentation — SKILL.md

- [ ] Update `.claude/skills/cafleet/SKILL.md`: add a "### Server" subsection in Command Reference documenting `cafleet server [--host <addr>] [--port <int>]`; note it does not require `--session-id`; mention `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` env vars. <!-- completed: -->

### Step 4: Code — config changes

- [ ] Modify `cafleet/src/cafleet/config.py`: change `broker_host: str = "0.0.0.0"` to `broker_host: str = "127.0.0.1"`. Convert `broker_host` and `broker_port` from bare defaults to `Field(default=..., validation_alias="CAFLEET_BROKER_HOST")` / `Field(default=..., validation_alias="CAFLEET_BROKER_PORT")`. Leave `broker_base_url` untouched. <!-- completed: -->
- [ ] Verify no other code currently reads `BROKER_HOST` / `BROKER_PORT` directly (should be no hits; pydantic-settings is the only consumer). Grep before and after. <!-- completed: -->

### Step 5: Code — server.py cleanup and warning

- [ ] Modify `cafleet/src/cafleet/server.py`: delete the `if __name__ == "__main__"` block (lines 58-64); delete module-level `import uvicorn`, `from cafleet.config import settings`, `import logging`, and `logger = logging.getLogger(__name__)`; add `import sys`. Keep `app = create_app()` at module scope. <!-- completed: -->
- [ ] Rename `_default_webui_dist_dir()` → `default_webui_dist_dir()` in `server.py` (drop leading underscore) so the CLI handler and tests can consume it without touching a private symbol. <!-- completed: -->
- [ ] In `create_app()`, compute `emit_warning_if_missing = webui_dist_dir is None` before falling back to `default_webui_dist_dir()`. If the gate is true and the resolved path does not exist, `print("warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.", file=sys.stderr)` once at startup. This makes the warning visible from every real startup path (`cafleet server`, `mise //cafleet:dev`, direct `uv run uvicorn cafleet.server:app`) while staying silent when tests pass an explicit override. <!-- completed: -->

### Step 6: Code — CLI subcommand

- [ ] Add `@cli.command("server")` in `cafleet/src/cafleet/cli.py` at an appropriate spot in the top-level commands section. Follow the Click implementation sketch in the Specification (no warning logic in the handler — `create_app()` owns that). Import `uvicorn` and `cafleet.config.settings` lazily inside the handler. <!-- completed: -->
- [ ] Verify `_require_session_id()` is NOT called in the `server` handler (session-id is silently accepted, never required). <!-- completed: -->

### Step 7: Tests

- [ ] Create `cafleet/tests/test_server_cli.py` mirroring `test_cli_session_flag.py` style. Include the five test classes listed in the Specification: `TestServerCommandHelp`, `TestServerCommandFlagParsing`, `TestServerDoesNotRequireSessionId`, `TestWebUIDistWarning`, `TestBrokerHostDefault`. Use `monkeypatch` to replace `uvicorn.run` with a no-op capturing call args; `TestWebUIDistWarning` exercises `create_app()` directly (both with `webui_dist_dir=None` for the warning path and with an explicit `tmp_path` override to prove the gate). <!-- completed: -->
- [ ] Include the `Settings().broker_host == "127.0.0.1"` assertion in `TestBrokerHostDefault`. <!-- completed: -->
- [ ] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: -->

### Step 8: Quality gates

- [ ] Run `mise //:lint` — must pass (ruff F401 would catch any missed dead imports from Step 5). <!-- completed: -->
- [ ] Run `mise //:format` — must pass. <!-- completed: -->
- [ ] Run `mise //:typecheck` — must pass. <!-- completed: -->
- [ ] Manual smoke: in a terminal, run `cafleet server`; curl `http://127.0.0.1:8000/ui/api/sessions` (or open `http://127.0.0.1:8000/ui/` if the WebUI dist is built); verify the startup warning appears when the WebUI dist is missing; Ctrl-C cleanly shuts down. Confirm `mise //cafleet:dev` still starts the server on `127.0.0.1:8000` with the new uvicorn-direct invocation AND emits the same warning when dist is missing. Confirm `CAFLEET_BROKER_PORT=9001 cafleet server` binds 9001. <!-- completed: -->

### Step 9: Finalize

- [ ] Update Status to Complete and refresh Last Updated. <!-- completed: -->
- [ ] Add a Changelog entry. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-14 | Initial draft |
| 2026-04-14 | Reviewer revisions: (BLOCKER 1) added `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` `validation_alias` wiring to `config.py` as a new Step 4 bullet plus matching Success Criteria; (BLOCKER 2) replaced incorrect "pip-install no-restart" rationale with accurate "contributors restart manually between edits" wording; (BLOCKER 3) moved the missing-WebUI-dist warning from the CLI handler into `create_app()` gated on `webui_dist_dir is None` so every startup path fires it and tests with explicit overrides stay quiet; (NB 4) added explicit Step 5 bullets for removing dead `from cafleet.config import settings`, `import logging`, `logger`; (NB 5) added `TestBrokerHostDefault` asserting the `127.0.0.1` default; (NB 6) added explicit `--host 127.0.0.1` to mise dev to prevent silent divergence from `cafleet server`'s env-var-aware defaults; (NB 7) renamed `_default_webui_dist_dir` → `default_webui_dist_dir` since it now crosses module boundaries. Progress 18 → 22 tasks. |
