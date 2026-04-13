# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run tests: `mise //cafleet:test`
- Lint (root): `mise //:lint`
- Lint (cafleet): `mise //cafleet:lint`
- Lint (admin): `mise //admin:lint`
- Format check (root): `mise //:format`
- Type check: `mise //:typecheck`
- Sync workspace: `uv sync` (from project root)
- Start admin WebUI server: `mise //cafleet:dev` (WebUI-only; CLI commands do not require a running server. Serves `/ui/` only after `//admin:build` has been run)
- Start admin dev server: `mise //admin:dev`
- Build admin: `mise //admin:build`

## mise Tasks

- Use full-path notation: `mise //[package]:[task]`. Do NOT use short-form `mise <task>`.
- Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- Run all tasks from the project root. No `cd` required.
