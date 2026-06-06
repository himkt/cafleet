# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run tests: `mise //cafleet:test`
- Lint: `mise //cafleet:lint`
- Lint (admin): `mise //admin:lint`
- Format: `mise //cafleet:format`
- Type check: `mise //cafleet:typecheck`
- Sync dependencies: `mise //:uv-sync`
- Install the `cafleet` CLI (editable uv tool): `mise //cafleet:install` — run this after pulling any change under `cafleet/src/cafleet/` if the global `cafleet` binary was previously installed non-editably; the editable reinstall makes future source edits take effect without another install.
- Build cafleet wheel: `mise //cafleet:build` — emits the wheel into `cafleet/dist/`.
- Publish cafleet: `mise //cafleet:publish` — chained task that builds admin assets, builds the wheel, then runs `uv publish`.
- Start admin WebUI server: either `cafleet server` (packaged launcher; `--host` / `--port` flags, defaults `127.0.0.1:8000` from `settings.broker_host` / `settings.broker_port`, also honors `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`) **or** `mise //cafleet:dev` (runs `uv run --package cafleet uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` directly; does NOT delegate to `cafleet server`). Both are independent entry points for the same FastAPI app and neither runs with `--reload` — contributors restart manually between edits. WebUI-only: CLI commands do not require a running server. Serves `/` only after `mise //admin:build` has been run.
- Start admin dev server: `mise //admin:dev`
- Build admin: `mise //admin:build`

## mise Tasks

- Use full-path notation: `mise //[package]:[task]`. Do NOT use short-form `mise <task>`.
- Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- Run all tasks from the project root. No `cd` required.
- **mise tasks forward positional args to the underlying command.** When you need to pass pytest args (test selectors, `-x`, `-v`, etc.), pass them directly to the mise task — do NOT fall back to `uv run python -m pytest` to "get more control". Examples:
  - Run one test: `mise //cafleet:test tests/test_base_dir.py::test_my_case`
  - Stop on first failure with verbose output: `mise //cafleet:test -xvs tests/test_my.py`
  - Match a keyword: `mise //cafleet:test -k my_keyword`
  - Package-relative paths only (`tests/...`, not `cafleet/tests/...`), because the mise task's working directory is `cafleet/`.
  - **Defensive `--` separator** for args that might collide with a mise flag: `mise //cafleet:test -- --collect-only -q tests/`. The bare form (without `--`) works in practice for the common pytest flags above; reach for `--` when an arg starts with two dashes AND could plausibly be parsed by mise itself.

## NEVER bypass mise with the underlying tool

The commands above are the **only** way to run these operations. Do NOT invoke the underlying tool directly, even when the underlying invocation "would work" or "is faster":

| NEVER | Use instead | Why |
|---|---|---|
| `uv run ruff check .` | `mise //cafleet:lint` | bypasses project lint config |
| `uv run ruff format [--check] .` | `mise //cafleet:format` | bypasses project format config |
| `uv run ty check` | `mise //cafleet:typecheck` | bypasses project typecheck config |
| `uv run --frozen --package cafleet python -m pytest ...` | `mise //cafleet:test` | bypasses the project's test runner config and env setup |
| `uv run cafleet ...` for verification/smoke | delegate to a teammate that already has permission, or ask the user | see `skills/agent-team-supervision/SKILL.md` § *Authorization-Scope Guard* |

This rule applies **even when a teammate is blocked on permissions** and you are tempted to "just run it yourself" — using `mise` keeps commands matching the project's `permissions.allow` patterns, which is the entire point of this project's session-id / agent-id CLI design.

## Skill artifact runners (project-specific glue)

Skills under `skills/` are deliberately invocation-agnostic — they describe the artifact they produce (a matplotlib script, a Slidev deck) but not the host-project run command. This section is the cafleet repo's catalog of those commands, paired one-to-one with the skill artifact they run. Skills MUST NOT embed the cafleet-specific runner in their own body; they direct users to the host project's `.claude/rules/`, which in this repo is this very file.

| Skill artifact | Cafleet runner |
|---|---|
| `cafleet-create-figure` matplotlib scripts | `uv run --frozen --group research python <script>` — the `[dependency-groups].research = ["matplotlib"]` block in root `pyproject.toml` provides matplotlib; `uv.lock` pins the version. |
| `cafleet-research-presentation` bun deps install | `mise //:bun-install` (equivalent to `bun install --frozen-lockfile`). |
| `cafleet-research-presentation` Slidev dev server | `mise //:slidev <folder>/slide.md` — PTY-wrapped via `script -qfc 'bun run slidev --open false <slide>' /dev/null`. Default URL `http://localhost:3030`. Run with `run_in_background: true`. |
| Calling-pane working directory for `bun` / `agent-browser` / Slidev | The cafleet repo root (where `package.json` and `node_modules/` live). |
| `agent-browser wait` | Discouraged — unreliable across renderers and slow CI environments. The repo's `.claude/settings.json` `permissions.deny` blocks the common `wait --load networkidle` forms and bare `wait` forms but does not cover every variant; treat the whole family as off-limits and use `sleep N` + `bun run agent-browser ... open` retry loops instead. |
