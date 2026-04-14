# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

When a task matches a skill below, you MUST invoke it via the Skill tool BEFORE taking any other action. Pay attention to override instructions (what NOT to do) in each entry.

- `/cafleet` — Interact with the CAFleet A2A message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents.
- `/cafleet-monitoring` — Mandatory supervision protocol for a Director managing member agents via CAFleet. Defines monitoring loop, spawn protocol, and stall response.
- `/cafleet-design-doc` — Standardized design document format with template and guidelines. Load when writing or editing a design document.
- `/cafleet-design-doc-create` — Create a new design document using CAFleet-native orchestration (Director / Drafter / Reviewer). Use when the user wants to create a specification with CAFleet message broker coordination.
- `/cafleet-design-doc-execute` — Implement features based on a design document using CAFleet-native orchestration with TDD cycle (Director / Programmer / Tester / optional Verifier).

## Project: CAFleet

A2A-inspired message broker + agent registry for coding agents.

- **Design document**: `design-docs/0000001-a2a-registry-broker/design-doc.md` (Status: Complete)
- **Design document**: `design-docs/0000002-access-control/design-doc.md` — Access-control via shared API key (superseded by 0000015 session model) (Status: Complete)
- **Design document**: `design-docs/0000010-sqlite-store-migration/design-doc.md` — SQLite + SQLAlchemy + Alembic store migration (Status: Complete)
- **Single package** (uv workspace):
  - `cafleet/` — `cafleet` (FastAPI + SQLAlchemy + Alembic + click)
- **Unified CLI command**: `cafleet` (with `db init` for schema management, `session` for session CRUD, and all agent/messaging commands)

## Tech Stack

- Python 3.12+ with uv workspace
- Server: FastAPI + SQLAlchemy + Alembic
- CLI: click (direct SQLite via broker module)

## Commands

See `.claude/rules/commands.md` for the full command reference.

## Skill Discovery & Authorization Scope

See `.claude/rules/skill-discovery.md`. Two mandatory rules:
1. **Load the matching skill BEFORE running ad-hoc commands** — especially `github-cli` for any `gh pr *` / `gh api repos/.../comments` / reviewer-request operation. Do NOT guess reviewer slugs or API paths.
2. **Authorization is scoped to the specific action** — when the user says "PR 24 created", stop acting on the push/PR workflow. Do NOT run further `git push`, `gh pr edit`, or similar remote-visible commands without explicit re-authorization. When the user signals stop (including profanity/frustration), acknowledge and wait; skip cron firings and idle notifications until they re-engage.

