# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run registry tests: `mise //registry:test`
- Run client tests: `mise //client:test`
- Run MCP server tests: `mise //mcp-server:test`
- Lint (root): `mise //:lint`
- Lint (registry): `mise //registry:lint`
- Lint (client): `mise //client:lint`
- Lint (admin): `mise //admin:lint`
- Lint (mcp-server): `mise //mcp-server:lint`
- Format check (root): `mise //:format`
- Type check: `mise //:typecheck`
- Sync workspace: `uv sync` (from project root)
- Start broker server: `mise //registry:dev`
- Start MCP server: `mise //mcp-server:dev`
- Start admin dev server: `mise //admin:dev`
- Build admin: `mise //admin:build`

## mise Tasks

- Use full-path notation: `mise //[package]:[task]`. Do NOT use short-form `mise <task>`.
- Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- Run all tasks from the project root. No `cd` required.
