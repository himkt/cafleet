# Design document workflow

Develop a feature through a reviewed design, an interview and an implementation.
The `cafleet` and `cafleet-design-doc` skills coordinate the teams; give your
coding agent one prompt per stage.

## Prompts {#prompts}

Each prompt triggers one `cafleet-design-doc` workflow:

| Stage | Prompt | Outcome | Team |
|---|---|---|---|
| 1. Draft | `Create a design doc for <one-line feature description>.` | Clarification, drafting and review, followed by your approval | Director + Drafter + Reviewer |
| 2. Refine | `Interview me about design-docs/NNNNNNN-<slug>.` | Questions and persisted answers, with document annotations for revision | Director + Analyzer |
| 3. Implement | `Implement design-docs/NNNNNNN-<slug>.` | Implementation and checks, fresh review, then your approval | Director + Programmer + Tester for code, optional Verifier, then a fresh Reviewer |

These stages invoke create, interview and execute respectively. After an
interview, resume creation to incorporate its annotations and review the
revised design before implementation. Documentation/configuration work may
use a Programmer without a Tester. Contributor setup is in
[Contributing](../contributing.md).

## Where output lands

Creation writes `design-docs/NNNNNNN-<slug>/design-doc.md` in your repository.
The interview keeps questions and answers in `question.md` beside it and
annotates the design; execution updates the design's task progress while
implementing the approved work.

## Watch the team work

Every inter-member message is persisted in SQLite, so you can follow the
team's coordination live: open the WebUI timeline for the team's fleet —
see [Use the admin WebUI](use-the-webui.md).

## Invocation syntax

See your coding-agent's skill documentation for the literal invocation
syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill
discovery).
