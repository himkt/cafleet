---
icon: lucide/file-text
---

# Design-doc-driven development

CAFleet ships three skills that run spec-driven development as
CAFleet-orchestrated teams. Give your coding agent one prompt per stage, in
order.

## Prompts

```text
Create a design doc for <one-line feature description>.
```

Triggers the `cafleet-design-doc` skill's create workflow — a Director /
Drafter / Reviewer team drafts the design document.

```text
Interview me about design-docs/NNNNNNN-<slug>.
```

Triggers the `cafleet-design-doc` skill's interview workflow — a Director +
Analyzer pair annotates the document with your answers.

```text
Implement design-docs/NNNNNNN-<slug>.
```

Triggers the `cafleet-design-doc` skill's execute workflow — a Director /
Programmer / Tester team implements the document and a fresh Reviewer member
reviews it before your approval.

The contributor-facing description of this loop, including what to pass to
each skill, lives in [Contributing](../get-started/contributing.md).

## Where output lands

Each run produces `design-docs/NNNNNNN-<slug>/design-doc.md` in your
repository. The CAFleet repo's own
[`design-docs/`](https://github.com/himkt/cafleet/tree/main/design-docs)
folder holds real examples produced by this loop.

## Watch the team work

Every inter-member message is persisted in SQLite, so the team's coordination
is fully auditable: inspect any message with `cafleet message show`, or query
the database file directly — see [Storage](../concepts/storage.md).

## Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
