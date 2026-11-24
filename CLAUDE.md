# CLAUDE.md

This file provides repository guidance to coding agents.

## Skills

Load relevant skills through the executing backend's supported loader. Assigned CAFleet members continue their existing team workflow with the installed role/core/backend references from their spawn prompt.

## Project: CAFleet

CAFleet multiplexes coding-agent sessions and coordinates them through durable point-to-point messages. The Go module at the repository root builds the unified CLI with the standard library, ncruces go-sqlite3 and an embedded SQL schema. The binary embeds its data files from `assets/`; the documentation site retains its own Node/pnpm toolchain.

## Commands

Use full mise monorepo task paths and discover available operations with `mise tasks --all`. The root `mise.toml` owns the Go tasks and selects the documentation site's task file through `monorepo.config_roots`; see `.claude/rules/commands.md` for invocation conventions.

## Development models

Launch these backends with the model below for development sessions, experiments and live probes:

| Backend | Launch |
|---|---|
| `opencode` | `--model opencode-go/gpt-6-luna` |
| `hermes` | `--provider openrouter -m openai/gpt-6-luna` |
| `pi` | `--provider openrouter --model openai/gpt-6-luna` |

## Tests

Write every Go test in the `_test.go` file beside the implementation file it exercises, in the same package; never add a test data file or directory. `.claude/rules/code-quality.md` § Test placement owns the convention.
