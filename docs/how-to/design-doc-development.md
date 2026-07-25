---
icon: lucide/file-text
---

# Design-doc-driven development

CAFleet ships three skills that run spec-driven development as
CAFleet-orchestrated teams. Give your coding agent one prompt per stage, in
order.

## Prompts {#prompts}

Each prompt triggers one `cafleet-design-doc` workflow:

| Stage | Prompt | Workflow | Team |
|---|---|---|---|
| 1. Draft | `Create a design doc for <one-line feature description>.` | create | Director + Drafter + Reviewer |
| 2. Refine | `Interview me about design-docs/NNNNNNN-<slug>.` | interview | Director + Analyzer, annotating the document with your answers |
| 3. Implement | `Implement design-docs/NNNNNNN-<slug>.` | execute | Director + Programmer + Tester, with a fresh Reviewer before your approval |

The contributor-facing description of this loop, including what to pass to
each skill, lives in [Contributing](../contributing.md).

## Where output lands

Each run produces `design-docs/NNNNNNN-<slug>/design-doc.md` in your
repository. The CAFleet repo's own
[`design-docs/`](https://github.com/himkt/cafleet/tree/main/design-docs)
folder holds real examples produced by this loop.

## Watch the team work

Every inter-member message is persisted in SQLite, so you can follow the
team's coordination live: open the WebUI timeline for the team's fleet —
see [Use the admin WebUI](use-the-webui.md).

## Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
