# Contributing

Run repository operations from the root through mise full-path tasks. Follow the [design workflow](how-to/design-doc-development.md) for substantial changes and use proportionate checks for routine edits.

## Project owners

| Path | Responsibility |
|---|---|
| `cafleet/` | Rust CLI, typed broker, native process/multiplexer adapters and embedded asset build |
| `cafleet/migrations/` | Application storage shape and supported upgrades |
| `skills/` | Agent procedures and offline runtime references |
| `presets/` | Executable backend permissions/configuration |
| `docs/` | Operator content, theme and Node/pnpm publication toolchain |
| `design-docs/` | Local specifications and evidence |

## Development

Install the pinned tools with the repository's mise configuration. Use `mise tasks` and package task definitions as the task registry. Run the validation and build tasks defined by each affected package.

Keep necessary unit coverage inline in owning production files. Exercise real storage/file-lock/process behavior where appropriate and use small explicit fakes at meaningful boundaries. Assert required state and preserve transaction/input/deadline/delivery guarantees.

Migrations extend the contiguous current-family chain. Preserve existing migration files/checksums and valid data; refuse incompatible/ahead databases before mutation.

### Install a checkout

`mise //cafleet:install` installs a rebuilt CLI; then run `cafleet setup` to install its embedded skills/presets. Setup refreshes assets captured at binary build time.

For working-tree skills, use `mise //:skill-install` after setup. Preserve an active executing team's installed binary/assets until teardown; validate checkout builds using isolated configuration/database and disposable owned panes.

## Documentation and publication

Run `mise //docs:install`, then `mise //docs:build`; the build checks site links. Use the docs task registry for preview. Preserve the theme, demo and pnpm toolchain/lockfile.

CLI help owns option choices, migrations own storage shape, source owns rendering, the site owns operator behavior and skills own agent procedure. Mirror every surviving site Markdown page except the homepage under `skills/cafleet/reference/runtime/` as a relative symlink with the same path. Keep that installed tree's links closed. The build embeds canonical content and setup writes ordinary files.

Inspect skill/root Markdown links separately from the site build. Update navigation with page changes and fully remove maintained references to deleted features/paths. Historical design/research artifacts retain their evidence.

Write intended behavior directly, describe growing groups by their selecting property and link authoritative registries. Use tables for parallel comparisons and concise prose for rationale.

To publish a binary release, run the Bump workflow, merge its version-change pull request, then publish the GitHub release that triggers the release workflow. The workflow definitions own current choices, build targets and permissions. The Pages workflow publishes documentation. Follow the active session's Git/artifact/publication rules and existing user authorization.
