---
icon: lucide/scale
---

# Model selection

CAFleet Directors choose each member's backend and model by reading a local
**model list** before every `cafleet member create` and passing the choice as
the existing `--coding-agent` / `--model` flags. Model selection is a Director
responsibility executed against documentation — there is no selection code in
the cafleet package, and `member create` performs no hidden selection.

## The model list

The model list lives at `skills/cafleet/reference/model-list.md` in every
deployed cafleet skill replica. It is a catalog-style reference page: one
table per backend (`claude`, `codex`, and `opencode`), ordered most → least
capable, with each model's spawn token, its alias (claude models), reviewed
capability class, and standard input/output USD-per-MTok prices, plus links
to the approved official pricing pages (Anthropic, OpenAI, and OpenCode Zen).
Capability classes and the ordering are maintainer judgment, never a provider
benchmark claim; prices are standard provider rates — planning estimates,
not an invoice guarantee.

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

Two roles are policy exceptions on **every** team spawn, trigger or not:

- **Monitor** — the cheapest listed model that can run the monitoring
  protocol reliably.
- **Reviewer** — the most capable listed model of the chosen backend.

Explicit flags stay overrides: a user-supplied `--coding-agent`, `--model`, or
`--effort` is recorded rather than silently replaced (a mismatched
backend/model pair is relayed to the user instead of spawned), and a
user-pinned model is never deleted and replaced automatically. A stale model
list disables cost efficiency mode, and a task no listed model fits gets an
operator relay — the Director **fails closed** instead of spawning a guessed
model.

## Replacement

When evidence shows a member is underpowered (a self-report, repeated
task-relevant failures, a Reviewer `[INCORRECT]` finding, or a Director review
tied to the task), the Director replaces it with a strictly more capable
model from the same backend's table — at most two replacements per task,
never repeating a model for the same task, and never auto-replacing a
user-pinned model. The procedure is
specified in the cafleet skill's `reference/director.md`.
