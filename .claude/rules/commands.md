# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run tests: `mise //cafleet:test`
- Lint: `mise //cafleet:lint`
- Lint (admin): `mise //admin:lint`
- Format: `mise //cafleet:format`
- Type check: `mise //cafleet:typecheck`
- Sync dependencies: `mise //cafleet:sync`
- Start admin WebUI server: either `cafleet server` (packaged launcher; `--host` / `--port` flags, defaults `127.0.0.1:8000` from `settings.broker_host` / `settings.broker_port`, also honors `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`) **or** `mise //cafleet:dev` (runs `uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` directly; does NOT delegate to `cafleet server`). Both are independent entry points for the same FastAPI app and neither runs with `--reload` — contributors restart manually between edits. WebUI-only: CLI commands do not require a running server. Serves `/ui/` only after `mise //admin:build` has been run.
- Start admin dev server: `mise //admin:dev`
- Build admin: `mise //admin:build`

## mise Tasks

- Use full-path notation: `mise //[package]:[task]`. Do NOT use short-form `mise <task>`.
- Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- Run all tasks from the project root. No `cd` required.

## NEVER bypass mise with the underlying tool

The commands above are the **only** way to run these operations. Do NOT invoke the underlying tool directly, even when the underlying invocation "would work" or "is faster":

| NEVER | Use instead | Why |
|---|---|---|
| `uv run ruff check .` | `mise //cafleet:lint` | bypasses project lint config |
| `uv run ruff format [--check] .` | `mise //cafleet:format` | bypasses project format config |
| `uv run ty check` | `mise //cafleet:typecheck` | bypasses project typecheck config |
| `uv run --frozen --package cafleet python -m pytest ...` | `mise //cafleet:test` | bypasses the project's test runner config and env setup |
| `uv run cafleet ...` for verification/smoke | delegate to a teammate that already has permission, or ask the user | see `.claude/rules/skill-discovery.md` (Authorization scope section) |

This rule applies **even when a teammate is blocked on permissions** and you are tempted to "just run it yourself" — using `mise` keeps commands matching the project's `permissions.allow` patterns, which is the entire point of this project's session-id / agent-id CLI design.
