---
icon: lucide/file-text
---

# Design-doc-driven development

CAFleet ships three skills that run spec-driven development as
CAFleet-orchestrated teams. Invoke them in order from your coding agent:

1. The `cafleet-design-doc-create` skill — a Director / Drafter / Reviewer
   team drafts the design document.
2. The `cafleet-design-doc-interview` skill — a fine-grained Q&A pass that
   annotates the document with your answers.
3. The `cafleet-design-doc-execute` skill — a Director / Programmer / Tester
   team implements the document.

The contributor-facing description of this loop, including what to pass to
each skill, lives in [Contributing](../get-started/contributing.md).

## Where output lands

Each run produces `design-docs/NNNNNNN-<slug>/design-doc.md` in your
repository. The CAFleet repo's own
[`design-docs/`](https://github.com/himkt/cafleet/tree/main/design-docs)
folder holds real examples produced by this loop.

## Watch the team work

Every inter-agent message is persisted in SQLite, so you can follow the
team's coordination live: open the WebUI timeline for the team's fleet —
see [Use the admin WebUI](use-the-webui.md).

## Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
