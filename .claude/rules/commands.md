# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run tests: `mise //cafleet:test`
- Lint: `mise //cafleet:lint`
- Lint (admin): `mise //admin:lint`
- Format: `mise //cafleet:format`
- Type check: `mise //cafleet:typecheck`
- Sync docs/research dependencies: `mise //:uv-sync` — the root uv workspace provides the docs toolchain (zensical, bump-my-version) and the `research` group (matplotlib) only.
- Install the `cafleet` CLI: `mise //cafleet:install` — runs `cargo install --path cafleet`. The install copies the compiled binary, so re-run it after pulling or editing any Rust source under `cafleet/src/` for the global `cafleet` binary to pick up the change.
- Build the release binary: `mise //cafleet:build` — emits `target/release/cafleet`.
- Database migrations are hand-written SQL files guarded by the chain test — see `database-migrations.md`. There is no migration-autogeneration task.
- Publishing is release-CI-only: on release publish, the GitHub Actions release workflow builds the three target binaries and uploads them to the GitHub Release. There is no local publish task.
- Start admin WebUI server: either `cafleet server` (packaged launcher; `--host` / `--port` flags, defaults `127.0.0.1:8000` from `settings.broker_host` / `settings.broker_port`, also honors `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`) **or** `mise //cafleet:dev` (runs `cargo run -- server`). Both are entry points to the same app and neither auto-reloads — contributors restart manually between edits. WebUI-only: CLI commands do not require a running server. The WebUI dist is embedded into the binary at build time.
- Start admin dev server: `mise //admin:dev`
- Build admin: `mise //admin:build`

Every cargo-invoking task (`test`, `lint`, `typecheck`, `build`, `install`, `dev`) depends on `//admin:build`, because the cargo build embeds the admin dist and fails loudly when `cafleet/webui-dist/` is missing — a fresh clone works with no manual prerequisite.

## mise Tasks

- Use full-path notation: `mise //[package]:[task]`. Do NOT use short-form `mise <task>`.
- Do NOT use `mise run <task>` — the `run` subcommand is unnecessary.
- **mise tasks forward positional args to the underlying command.** When you need to pass cargo-test args (test-name filters, `--nocapture`, etc.), pass them directly to the mise task — do NOT fall back to `cargo test` to "get more control". Examples:
  - Run tests matching a name: `mise //cafleet:test my_test_name`
  - Show test output: `mise //cafleet:test my_test_name -- --nocapture`
  - **Defensive `--` separator** for args that might collide with a mise flag: the bare form works for plain test-name filters; reach for `--` when an arg starts with two dashes AND could plausibly be parsed by mise itself (all `--`-prefixed args after the first `--` reach the test harness).

## NEVER bypass mise with the underlying tool

The commands above are the **only** way to run these operations. Do NOT invoke the underlying tool directly, even when the underlying invocation "would work" or "is faster":

| NEVER | Use instead | Why |
|---|---|---|
| `cargo clippy ...` | `mise //cafleet:lint` | bypasses project lint config |
| `cargo fmt [--check]` | `mise //cafleet:format` | bypasses project format config |
| `cargo check` | `mise //cafleet:typecheck` | bypasses project typecheck config |
| `cargo test ...` | `mise //cafleet:test` | bypasses the project's test runner config and env setup |
| `cargo run -- ...` / `cafleet ...` for verification/smoke | delegate to a teammate that already has permission, or ask the user | see `skills/cafleet/reference/supervision.md` § *Authorization-Scope Guard* |

This rule applies **even when a teammate is blocked on permissions** and you are tempted to "just run it yourself" — using `mise` keeps commands matching the project's `permissions.allow` patterns, which is the entire point of this project's fleet-id / member-id CLI design.

## Skill artifact runners (project-specific glue)

Skills under `skills/` are deliberately invocation-agnostic — they describe the artifact they produce (a matplotlib script, a Slidev deck) but not the host-project run command. This section is the cafleet repo's catalog of those commands, paired one-to-one with the skill artifact they run. Skills MUST NOT embed the cafleet-specific runner in their own body; they direct users to the host project's `.claude/rules/`, which in this repo is this very file.

| Skill artifact | Cafleet runner |
|---|---|
| `cafleet-research` visualization (matplotlib) scripts | `uv run --frozen --group research python <script>` — the `[dependency-groups].research = ["matplotlib"]` block in root `pyproject.toml` provides matplotlib; `uv.lock` pins the version. |
| `cafleet-research` presentation pnpm deps install | `mise //:pnpm-install` (equivalent to `pnpm install --frozen-lockfile`). |
| `cafleet-research` presentation Slidev dev server | `mise //:slidev <folder>/slide.md` — PTY-wrapped via `script -qfc 'pnpm exec slidev --open false <slide>' /dev/null`. Default URL `http://localhost:3030`. Run with `run_in_background: true`. |
| Calling-pane working directory for `pnpm` / `agent-browser` / Slidev | The cafleet repo root (where `package.json` and `node_modules/` live). |
| `agent-browser wait` | Discouraged — unreliable across renderers and slow CI environments. The repo's `.claude/settings.json` `permissions.deny` blocks the common `wait --load networkidle` forms and bare `wait` forms but does not cover every variant; treat the whole family as off-limits and use `sleep N` + `pnpm exec agent-browser ... open` retry loops instead. |
