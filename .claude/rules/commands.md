# Commands

**IMPORTANT**: Always use mise full-path tasks. Run from the project root — do NOT `cd` into package directories.

- Run tests: `mise //cafleet:test`
- Lint: `mise //cafleet:lint`
- Lint (admin): `mise //admin:lint`
- Format: `mise //cafleet:format`
- Type check: `mise //cafleet:typecheck`
- Install the `cafleet` CLI: `mise //cafleet:install` — runs `cargo install --path cafleet`. The install copies the compiled binary, so re-run it after pulling or editing any Rust source under `cafleet/src/` for the global `cafleet` binary to pick up the change.
- Build the release binary: `mise //cafleet:build` — emits `cafleet/target/release/cafleet`.
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
  - **`--` separator**: pass test-name filters bare; insert `--` before any `--`-prefixed arg (everything after the first `--` reaches the test harness).

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
