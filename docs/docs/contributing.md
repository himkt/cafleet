# Contributing

Develop changes through reviewed designs and the bundled CAFleet workflows. Run repository tasks from the root using mise full-path notation.

## Project structure

| Entry | Responsibility |
|---|---|
| `cafleet/` | Rust CLI, typed synchronous SQLite broker, multiplexer/process adapters and embedded assets |
| `skills/` | Installed agent-facing skills, roles and references |
| `presets/` | Backend permission/configuration assets |
| `docs/` | Independent operator documentation and its Node/pnpm site tooling |
| `design-docs/` | Numbered design documents and local evidence |

## Rust boundaries

Broker queries return typed records; CLI presenters construct wire output. Shared diagnosis validates identity, schema and assets. Keep durable storage separate from synchronous notification, finite receiving, input ownership and multiplexer I/O. Required state fails at its read boundary. Preserve process pipe draining, owned-child cleanup, pane ownership compensation and layout cleanup.

## Development

```bash
git clone https://github.com/himkt/cafleet.git
```

Enter the checkout, then run repository-root tasks:

```bash
mise //cafleet:install
```

```bash
cafleet setup
```

Use `mise tasks` and package task definitions as the task registry. Run Rust validation with `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. The release build needs Rust and embedded skills/presets. Reinstall the binary after Rust changes to use the new implementation.

### Installing the skills from your checkout

Setup installs assets embedded at binary build time. Contributors can install working-tree skills with `mise //:skill-install`; run it after setup when you want those local files. Preserve an active executing team's installed assets until its workflow finishes.

## Building docs locally

The documentation site retains its Node/pnpm toolchain and lockfile. Run `mise //docs:install`, then `mise //docs:build`; the build checks links. Use the docs task registry for preview and other available operations. Update dependencies through the manifest and regenerate its pnpm lockfile deliberately.

## Contributing changes

Follow the [design document workflow](how-to/design-doc-development.md#prompts). Preserve role boundaries, clarification, review and user approval. The active session's host rules determine Git and artifact handling.

## Documentation style

Write human/operator behavior in the site and agent instructions in skills. State intended behavior directly. Link each shared enumeration to its owner; describe growing groups through their selecting property or registry. Use tables when parallel entries carry multiple shared attributes, with at least two data rows and at most two sentences per cell. Keep literal commands, flags, paths and errors in code spans.
