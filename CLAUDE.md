# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

The system-reminder lists every available skill with its trigger description. Read that list at the start of every session and invoke matching skills via the Skill tool BEFORE taking any other action.

## Project: CAFleet

A message broker and member registry for coding agents.

- **Single package**:
  - `cafleet/` — `cafleet` (FastAPI + SQLAlchemy + Alembic + click)
- **Unified CLI command**: `cafleet` (with `setup` — and its `db` / `skill` subcommands — for onboarding and schema management, and all member/messaging commands)

## Tech Stack

- Python 3.12+ managed with uv
- Server: FastAPI + SQLAlchemy + Alembic
- CLI: click (direct SQLite via broker module)

## Commands

See `.claude/rules/commands.md` for the full command reference.

