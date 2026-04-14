# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

When a task matches a skill below, you MUST invoke it via the Skill tool BEFORE taking any other action. Pay attention to override instructions (what NOT to do) in each entry.

- `/cafleet` — Interact with the CAFleet A2A message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents.
- `/cafleet-monitoring` — Mandatory supervision protocol for a Director managing member agents via CAFleet. Defines monitoring loop, spawn protocol, and stall response.
- `/cafleet-design-doc` — Standardized design document format with template and guidelines. Load when writing or editing a design document.
- `/cafleet-design-doc-create` — Create a new design document using CAFleet-native orchestration (Director / Drafter / Reviewer). Use when the user wants to create a specification with CAFleet message broker coordination.
- `/cafleet-design-doc-execute` — Implement features based on a design document using CAFleet-native orchestration with TDD cycle (Director / Programmer / Tester / optional Verifier).

## Plugin Skills

When a task matches a skill below, you MUST invoke it via the Skill tool BEFORE taking any other action. Pay attention to override instructions (what NOT to do) in each entry.

- `/cafleet:cafleet` — Interact with the CAFleet A2A message broker.
- `/cafleet:cafleet-monitoring` — Mandatory supervision protocol for a Director managing member agents via CAFleet.
- `/cafleet:cafleet-design-doc` — Standardized design document format with template and guidelines (plugin-local copy).
- `/cafleet:cafleet-design-doc-create` — Create a new design document using CAFleet-native orchestration.
- `/cafleet:cafleet-design-doc-execute` — Implement features based on a design document using CAFleet-native orchestration with TDD cycle.

## Project: CAFleet

A2A-native message broker + agent registry for coding agents.

- **Design document**: `design-docs/0000001-a2a-registry-broker/design-doc.md` (Status: Complete)
- **Design document**: `design-docs/0000002-access-control/design-doc.md` — Tenant isolation via shared API key (Status: Complete)
- **Design document**: `design-docs/0000010-sqlite-store-migration/design-doc.md` — SQLite + SQLAlchemy + Alembic store migration (Status: Complete)
- **Single package** (uv workspace):
  - `cafleet/` — `cafleet` (FastAPI + SQLAlchemy + Alembic + click)
- **Unified CLI command**: `cafleet` (with `db init` for schema management, `session` for namespace CRUD, and all agent/messaging commands)

## Tech Stack

- Python 3.12+ with uv workspace
- Server: FastAPI + SQLAlchemy + Alembic
- CLI: click (direct SQLite via broker module)

## Commands

See `.claude/rules/commands.md` for the full command reference.

