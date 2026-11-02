# CLAUDE.md

This file provides repository guidance to coding agents.

## Skills

Load relevant skills through the executing backend's supported loader. Assigned CAFleet members continue their existing team workflow with the installed role/core/backend references from their spawn prompt.

## Project: CAFleet

CAFleet multiplexes coding-agent sessions and coordinates them through durable point-to-point messages. The Rust package at the repository root builds the unified CLI with clap, rusqlite and embedded refinery migrations. Skills and presets are embedded assets; the documentation site retains its own Node/pnpm toolchain.

## Commands

Use full mise monorepo task paths and discover available operations with `mise tasks --all`. The root `mise.toml` owns the Rust tasks and selects the documentation site's task file through `monorepo.config_roots`; see `.claude/rules/commands.md` for invocation conventions.

## Tests

Write every Rust test as a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises; never add a test-only file or folder. `.claude/rules/code-quality.md` § Test placement owns the convention.
