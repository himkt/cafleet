# Design document workflow

Develop a feature through a reviewed design and an implementation.
The `cafleet` and `cafleet-design-doc` skills coordinate the teams; give your
coding agent one prompt per stage.

## Prompts {#prompts}

Each prompt triggers one `cafleet-design-doc` workflow:

| Stage | Prompt | Outcome | Team |
|---|---|---|---|
| 1. Draft | `Create a design doc for <one-line feature description>.` | Clarification, drafting and review, followed by your approval | Director + Drafter + Reviewer |
| 2. Implement | `Implement design-docs/NNNNNNN-<slug>.` | Implementation and checks, fresh review, then your approval | Director + Programmer + Tester for code, optional Verifier, then a fresh Reviewer |

These stages invoke create and execute respectively. Documentation/configuration
work may use a Programmer without a Tester. Contributor setup is in
[Contributing](../contributing.md).

## Where output lands

Creation writes `design-docs/NNNNNNN-<slug>/design-doc.md` in your repository.
CAFleet assigns a seven-digit, zero-padded number one above the highest existing
number in that repository's `design-docs/`, starting at `0000001`. The slug is a
short kebab-case description, such as `add-search`, giving `0000001-add-search`.
Execution updates the design's task progress while
implementing the approved work.

## Watch the team work

Read durable coordination with message poll/show and inspect the team’s visible panes with `member capture`. The Director receives replies and checks evidence until delegated work resolves; the [manual lifecycle](mixed-backend-team.md#manual-lifecycle) explains reception.

## Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
