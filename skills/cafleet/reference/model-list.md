# CAFleet Model List

The machine-readable source of truth for CAFleet model availability, reviewed
capability policy, and standard token-price estimates. The tables below are
maintained exclusively by the `cafleet-model-list-refresh` skill from the two
approved official pricing sources recorded in the Sources table. Capability
levels (0–5 per dimension) and the unique global rank are reviewed maintainer
judgment, not provider benchmark claims. Prices are standard direct-provider
USD API rates per MTok and are estimates, not an invoice guarantee; a row with
`—` in every price cell (for example a gateway model without an approved
source price) is visible for diagnostics but never an automatic-selection
candidate. A `—` in the Aliases cell means the model has no alias; the `Model`
cell is always a valid spawn token, and a model's key is `<backend>:<model>`.
Role and token profiles are reviewed code constants in the `cafleet` package,
not model-list data.

## Metadata

| Field | Value |
|---|---|
| schema_version | 1 |
| generated_at | 2026-07-19T12:12:20Z |
| freshness_days | 30 |

## Sources

| Source | URL | Retrieved at | Content SHA-256 |
|---|---|---|---|
| anthropic | https://platform.claude.com/docs/en/about-claude/pricing | 2026-07-19T12:12:20Z | 465c891550c135716901a2679c1c7a693b03fe3d2eac2aff200f9f7f745a1ea3 |
| openai | https://developers.openai.com/api/docs/pricing | 2026-07-19T12:12:20Z | 6b5ed5e1cdd1edded9e5cc4b187f1603d150318b9a03191bd549a8790c193b9d |

## Models

| Backend | Model | Aliases | Active | Rank | Cod | Pln | Rsc | Rev | Mon | In | Cached | Write | Out | Max tokens |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| claude | claude-fable-5 | fable | yes | 100 | 5 | 5 | 5 | 5 | 5 | 10.0 | 1.0 | 12.5 | 50.0 | 1000000 |
| claude | claude-opus-4-8 | opus | yes | 85 | 5 | 5 | 4 | 5 | 5 | 5.0 | 0.5 | 6.25 | 25.0 | 1000000 |
| claude | claude-sonnet-5 | sonnet | yes | 70 | 4 | 4 | 4 | 4 | 4 | 2.0 | 0.2 | 2.5 | 10.0 | 1000000 |
| claude | claude-haiku-4-5 | haiku | yes | 40 | 2 | 2 | 2 | 1 | 4 | 1.0 | 0.1 | 1.25 | 5.0 | 200000 |
| codex | gpt-5.6-sol | — | yes | 90 | 5 | 5 | 4 | 5 | 4 | 5.0 | 0.5 | 0.0 | 30.0 | 400000 |
| codex | gpt-5.6-terra | — | yes | 75 | 4 | 4 | 4 | 4 | 4 | 2.5 | 0.25 | 0.0 | 15.0 | 400000 |
| codex | gpt-5.6-luna | — | yes | 55 | 3 | 3 | 3 | 3 | 4 | 1.0 | 0.1 | 0.0 | 6.0 | 400000 |
| codex | gpt-5.5 | — | yes | 80 | 5 | 5 | 4 | 4 | 4 | 5.0 | 0.5 | 0.0 | 30.0 | 400000 |
| codex | gpt-5.4 | — | yes | 60 | 4 | 3 | 3 | 3 | 4 | 2.5 | 0.25 | 0.0 | 15.0 | 400000 |
| codex | gpt-5.4-mini | — | yes | 30 | 2 | 2 | 2 | 2 | 3 | 0.75 | 0.075 | 0.0 | 4.5 | 400000 |
| opencode | opencode/gpt-5.5-pro | — | yes | 82 | 5 | 5 | 4 | 5 | 4 | — | — | — | — | 400000 |
| opencode | opencode/gpt-5.5 | — | yes | 78 | 5 | 5 | 4 | 4 | 4 | — | — | — | — | 400000 |
| opencode | opencode/claude-opus-4-8 | — | yes | 76 | 5 | 5 | 4 | 5 | 5 | — | — | — | — | 1000000 |
| opencode | opencode/claude-sonnet-4-6 | — | yes | 65 | 4 | 4 | 4 | 4 | 4 | — | — | — | — | 1000000 |
| opencode | opencode/claude-haiku-4-5 | — | yes | 35 | 2 | 2 | 2 | 1 | 4 | — | — | — | — | 200000 |
