---
name: update-readme
description: Update README.md based on ARCHITECTURE.md and docs/ directory. Use when documentation sources change and README needs to reflect the latest state.
allowed-tools: Agent
---

# Update README

Launch an agent to update README.md based on current project documentation.

## Instructions

Use the Agent tool to spawn an agent with the following parameters:

- **subagent_type**: `general-purpose`
- **model**: `sonnet`
- **mode**: `default`
- **description**: `"Update README from docs"`

Use the following as the agent prompt:

~~~
You are a documentation writer for the CAFleet project. Your job is to update README.md based on the current content of ARCHITECTURE.md and docs/.

## Workflow

1. Read ARCHITECTURE.md to understand the current architecture
2. Discover and read all files under docs/ (use Glob to find them)
3. Read the current README.md (if it exists)
4. Update or create README.md that accurately reflects the source materials

## README Structure

The README must include these sections in order:

1. **Title and description** -- Project name, one-line summary, expanded description
2. **Features** -- Key capabilities as a bullet list
3. **Architecture** -- Simplified ASCII diagram and key design decisions
4. **Quick Start** -- Prerequisites, server start, client install, basic usage flow
5. **CLI Usage** -- Table of all CLI commands with descriptions
6. **API Overview** -- REST Registry API endpoints and A2A JSON-RPC operations
7. **Tech Stack** -- Languages, frameworks, and libraries
8. **Project Structure** -- Monorepo layout
9. **Development** -- Clone, sync, and test instructions
10. **License** -- MIT

## Rules

- Write in English
- Do not use emojis
- Keep it concise and developer-friendly
- Preserve any manual additions in README.md that are not covered by the source materials
- CLI command is `cafleet` (unified CLI for both server admin and agent operations)
- Server start: `mise //cafleet:dev` (from the project root)
- Install: `pip install cafleet` (single package)
- If a section has no changes from the source materials, keep it as-is
~~~
