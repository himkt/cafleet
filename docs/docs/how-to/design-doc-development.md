# Design document workflow

Use the `cafleet-design-doc` skill to create, refine or implement a specification with scoped members and independent review.

## Prompts

| Work | Example prompt | Result |
|---|---|---|
| Create | `Create a design doc for <feature>.` | Implementable specification with resolved decisions and independent review |
| Refine | `Interview me about design-docs/NNNNNNN-<slug>.` | Focused questions, preserved answers and revised decisions |
| Implement | `Implement design-docs/NNNNNNN-<slug>.` | Scoped implementation, appropriate checks and reviewed evidence |

The Director supplies absolute artifact paths, receives ready, reads full reports and owns Git/user decisions. Members own bounded assignments and suitable checks. Substantial work receives independent review; routine edits use proportionate validation. Existing authorization and explicit approval gates govern the next action.

## Artifacts and progress

New documents use `design-docs/NNNNNNN-short-kebab-slug/design-doc.md`. Select the highest existing numeric prefix plus one, padded to seven digits; start at `0000001`. Preserve explicit/existing targets and prior answers/annotations until incorporated.

A design states intended behavior, compatibility, failures, measurable success criteria and executable tasks. Task checkboxes are progress truth. Complete each task with its timestamp in the same edit; keep any existing progress metadata accurate.

## Follow the work

Read durable reports with poll/show and inspect roster activity, targeted captures and artifacts. Send findings/blockers/evidence directly, naming artifact paths for long reports. The Director resolves consequential missing information and preserves stop requests and pending user decisions.

Follow [Run a fleet](mixed-backend-team.md) for reception, recovery and owned teardown. Contributor setup and publication are in [Contributing](../contributing.md).
