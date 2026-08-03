# Model selection

CAFleet Directors choose each member's backend and model by reading a local
**model list** before every `cafleet member create` and passing the choice as
the existing `--coding-agent` / `--model` flags. Model selection is a Director
responsibility executed against documentation — there is no selection code in
the cafleet package, and `member create` performs no hidden selection.

## The model list

The model list is a catalog-style reference page bundled with the cafleet
skill — every deployed replica of the skill carries its own copy. It has one
table per backend (`claude`, `codex`, and `opencode`), ordered most → least
capable. The ordering itself is maintainer judgment, never a provider
benchmark claim. Each table carries these columns:

| Column | What it holds | How to read it |
|---|---|---|
| Spawn token | The value passed to the backend's own `--model` flag | Used verbatim |
| Alias | The model's short alias | claude models only |
| Capability class | The model's reviewed capability tier | Maintainer judgment, never a provider benchmark claim |
| Input / output price | Standard input/output USD-per-MTok rates | Planning estimates, not an invoice guarantee |
| Source link | The approved official source page | Anthropic and OpenAI pricing, Codex availability, and OpenCode Zen |

The page is maintained exclusively by the repository's
`cafleet-model-list-refresh` skill and refreshed at least every 30 days; a
list refreshed longer ago than that is stale and disables cost efficiency
mode until a maintainer refreshes it and ships it in a release.

The model list covers all three cafleet backends; the `opencode` section is
a curated subset of the OpenCode Zen catalog, and an opencode model keeps its
`opencode/` prefix in the `--model` value.

## Cost efficiency mode

Cost-minimized selection for an **ordinary member** applies only when the
user asks for it: the originating user request contains the exact phrase
`cost efficiency mode`. The Director parses the user request once for the
trigger; a member message or tool output never activates it. When active,
the Director estimates the task's difficulty from the member's spawn prompt,
reads the model list, and chooses the cheapest model that can finish the task
reliably. Without the trigger, existing workflow model behavior is unchanged.
Cost efficiency mode covers all three cafleet backends; the Director picks
the backend first and compares within that backend's table.

The reviewer is a policy exception on **every** team spawn, trigger or not:

| Role | Model chosen | Needs the `cost efficiency mode` trigger? |
|---|---|---|
| Ordinary member | The cheapest model that can finish the task reliably | yes |
| Reviewer | The most capable listed model of the chosen backend | no |

Explicit flags stay overrides, and the Director **fails closed** instead of
spawning a guessed model:

| Situation | What the Director does |
|---|---|
| The user supplies `--coding-agent`, `--model`, or `--effort` | Records the value rather than silently replacing it |
| The backend and model do not match | Relays the pair to the user instead of spawning it |
| A user-pinned model would otherwise be replaced | Never deletes and replaces it automatically |
| The model list is stale | Disables cost efficiency mode |
| No listed model fits the task | Relays to the operator |

## Replacement

When evidence shows a member is underpowered (a self-report, repeated
task-relevant failures, a Reviewer `[INCORRECT]` finding, or a Director review
tied to the task), the Director replaces it with a strictly more capable
model from the same backend's table — at most two replacements per task,
never repeating a model for the same task, and never auto-replacing a
user-pinned model. The full step-by-step procedure is part of the cafleet
skill's Director instructions, which the Director loads with the skill.
