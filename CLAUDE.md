# CLAUDE.md

This file provides repository guidance to coding agents.

## Skills

Load relevant skills through the executing backend's supported loader. Assigned CAFleet members continue their existing team workflow with the installed role/core/backend references from their spawn prompt.

## Project: CAFleet

CAFleet multiplexes coding-agent sessions and coordinates them through durable point-to-point messages. The Rust package under `cafleet/` builds the unified CLI with clap, rusqlite and embedded refinery migrations. Skills and presets are embedded assets; the documentation site retains its own Node/pnpm toolchain.

## Commands

Use repository-root mise full-path tasks. See `.claude/rules/commands.md` and the task registry for available operations.
