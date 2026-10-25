# Commands

Run project operations from the repository root through mise full-path tasks. Use `mise tasks` and the package task definitions as the task registry.

For Rust validation, run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. Use `mise //docs:build` for documentation/link checks. Find installation, formatting and release-build operations through the package task registry.

Rust tasks embed skills/presets and use the pinned toolchain. The documentation site owns its Node/pnpm tooling. Reinstall the CLI after Rust edits when the installed binary should change; preserve an active executing fleet's installed runtime/assets during implementation.

## mise Tasks

Use `mise //<package>:<task>` from the project root. Tasks forward positional arguments, so `mise //cafleet:test test_name` filters tests. Place two standalone `--` separators before test-harness flags: mise consumes one and Cargo forwards the other.

Run tests, lint, format, type checks, builds and installation through their tasks so project configuration and host permission patterns apply. Verification/smoke commands use an assigned teammate's authorized task scope. Route an observed necessary command denial to the Director through the member protocol.

Release publication belongs to the release CI workflow; the task registry owns available local operations.
