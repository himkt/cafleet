# Commands

Run project operations from the repository root through mise tasks. Use `mise tasks` and the root `mise.toml` as the task registry.

Required Rust validation is `mise cafleet-test` and `mise cafleet-lint`; `cafleet-typecheck` remains a registry task. Use `mise docs-build` for documentation/link checks. Find installation, formatting and release-build operations through the root task registry.

Rust tasks embed skills/presets and use the pinned toolchain. The documentation site owns its Node/pnpm tooling. Reinstall the CLI after Rust edits when the installed binary should change; preserve an active executing fleet's installed runtime/assets during implementation.

## mise Tasks

Use `mise <task>` from the project root. Tasks forward positional arguments, so `mise cafleet-test test_name` filters tests. Place two standalone `--` separators before test-harness flags: mise consumes one and Cargo forwards the other.

Run tests, lint, format, type checks, builds and installation through their tasks so project configuration and host permission patterns apply. Verification/smoke commands use an assigned teammate's authorized task scope. Route an observed necessary command denial to the Director through the member protocol.

Release publication belongs to the release CI workflow; the task registry owns available local operations.

## Bash tool

A CAFleet member runs commands directly through its available shell tool under the executing backend's host rules. Inspect real output, reconsider incorrect paths/flags or a permitted alternative, and route a necessary denied command to the Director. Report denied file edits for a host-rule change or Director edit.

### Owning protocols

- Member authority and observed-result reporting: `skills/cafleet/roles/member.md`.
- Required denied-command routing: `skills/cafleet/reference/prompt-routing.md`.
- `cafleet member exec` visibly submits native `! COMMAND`. Use the host's actual output; the Director inspects completion and sends explicit continuation when needed.

Run one command per shell invocation, preserve literal IDs and use a file writer for output. The selected backend's runtime bindings define its permission posture. A broad tool permission remains subject to the assigned role and Director-owned lifecycle/Git boundaries.
