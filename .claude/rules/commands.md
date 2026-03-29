# Commands

**IMPORTANT**: Always use `mise` tasks when available. `cd` into the package directory first, then run `mise <task>`.

- Run registry tests: `cd registry` then `mise test`
- Run client tests: `cd client` then `mise test`
- Run MCP server tests: `cd mcp-server` then `mise test`
- Lint: `mise lint` (from project root)
- Format check: `mise format` (from project root)
- Type check: `mise typecheck` (from project root)
- Sync workspace: `uv sync` (from project root)
- Start broker server: `cd registry` then `uv run uvicorn hikyaku_registry.main:app`
- Start MCP server: `cd mcp-server` then `uv run hikyaku-mcp`

## mise Tasks

- Use `mise <task>` directly. Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- Always prefer `mise` tasks over raw `uv run` commands when a task is defined.
