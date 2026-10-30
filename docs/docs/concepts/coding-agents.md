# Coding agents

Select a member backend through the CLI’s `--coding-agent` choices. Mixed-backend members share the same broker contracts. The value is recorded in the
placement's `coding_agent` column.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |

Plain `cafleet setup` installs assets for every supported backend
in one run; `--coding-agent` narrows the selection. [Quickstart](../quickstart.md#configure)
lists what setup installs for each backend and the configuration you add.

The flag means slightly different things per command:

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on.
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member inherits the spawning Director's placement backend (an explicit
  value still wins).

New members run through the CAFleet PTY runner with a declared detection-coverage state. Omitting `--permission-mode` selects the backend's default policy, its native automatic review where the backend has one; pass the flag only for another native mode the user asks for. Interactive native policies are eligible on every backend — OpenCode's through its `cafleet-ask` agent preset — with request-bound Director/user decisions under the Director session’s effective mode or manual original-surface input. Each bundled runner profile covers its backend from a minimum native version onward. Detector, composer and answer coverage are independent; without a measured submission plan, generated input uses Escape-first delivery fallback. Broker access remains supplied by each backend’s launch configuration, subject to native and managed rules. [Member permission events](member-permission-events.md) explains the workflow, and the [spawn argv contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#per-backend-build_spawn_argv-exact-token-by-token) specifies child arguments, defaults and sandbox boundaries.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders four identity
placeholders to literals, so the member reads its ids as plain text lines
(e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on every
command. The [spawn-prompt substitution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#spawn-prompt-substitution)
lists each placeholder and its label line. The member environment carries
`CAFLEET_DATABASE_URL` and the selected backend's config-location variable
when the creator sets it.
Each backend honors a leading-`!` shell shortcut on the coding agent's input line,
including opencode `mini`; [backend capabilities](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities)
compares their sandbox and shell-command posture.
`cafleet member exec` submits a visible native command through the runner, using the profile's measured native-exec plan or delivery fallback and queueing it while a detected native interaction is open; explicitly unwrapped members retain Escape-first input. A member routes a
command its harness does not run through the Director — see
[Command routing](member-lifecycle.md#command-routing). The
[CLI specification](https://github.com/himkt/cafleet/blob/main/SPEC.md#63-cli)
is the full command reference.

## One-shot command isolation

Members and the Director run each `cafleet` command as the only command of its
shell-tool invocation, so the host's single-command permission rules match it.
A compound line can fall outside those rules and fail with a permission error,
even when every command in it is allowed.

## Model choice

Before each spawn, the Director reads the selected backend's model catalog
and policy in the installed CAFleet skill, then passes the chosen
`--coding-agent` and `--model` values. The CLI forwards the model; selection
is a Director responsibility. The catalog provides exact spawn tokens,
Claude aliases, reviewed capability classes, standard input/output prices
and official sources, ordered from most to least capable within each backend.
The OpenCode catalog lists every Console model with its `opencode/` prefix,
excluding models past their published deprecation date, and every OpenCode Go
model with its `opencode-go/` prefix.

A bare Claude or GPT model reference selects Claude Code or Codex on every
fleet, including an OpenCode fleet. Naming OpenCode for the member selects
OpenCode for that model. When the Director resolves an OpenCode model by name,
it checks the operator's configured providers and uses the OpenCode Go form
when they offer it, otherwise the Console form. A bare name offered only by
OpenCode selects OpenCode when it uniquely matches a catalog ID. The Director
asks the operator to resolve an unknown or ambiguous reference. It also asks
when the provider check fails or offers neither form. A provider-prefixed model,
a role default and a family-word request on OpenCode use their written or
Console form as is.

Use the selected backend’s role defaults and catalog as the source of truth. OpenCode omits unsupported effort.

Only the exact phrase `cost efficiency mode` in the originating user request
activates cheapest-capable selection for ordinary members. The Director
chooses a backend first and compares models within that backend. On OpenCode,
models available through the operator's OpenCode Go subscription count as
effectively free and are preferred. Outside cost efficiency mode, ordinary
members use the backend's other-member default. Reviewers use their role defaults, including supported effort
levels.

Explicit backend, model and effort values remain overrides. A mismatch or
missing suitable model is relayed to the operator. The catalog is maintained
through the model-refresh workflow and refreshed at least every 30 days;
staleness disables cost efficiency mode until a maintainer refreshes and
ships it. Prices are planning estimates and capability classes are reviewed
judgments, rather than invoice guarantees or provider benchmark claims.

Evidence of insufficient capability can trigger a strictly stronger
same-backend replacement, at most twice per task and without repeating a
model. The installed Director policy owns evidence, handoff and selection;
a user-pinned model requires an operator decision before replacement.

`cafleet member create --model <string>` forwards the value to the backend's
own model flag; omission uses that binary's default. Claude and Codex pass any
value through and let the binary reject unknown models, so a newly released
model needs no CAFleet release. OpenCode requires the `<provider-id>/<model-id>`
form, checked when the member is created. The
[model selection contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#model-selection)
has the exact format and error.

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to
the spawned backend binary. Unlike `--model`, the accepted level set is
validated per backend at create time, before any registration or multiplexer
side effect; omit the flag and the binary uses its own default. OpenCode has no
reasoning-effort control, so omit the flag for OpenCode members. The
[reasoning effort contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#reasoning-effort)
lists each backend's levels and forwarding form.

## Backend differences

The backends differ in reasoning effort, pane titles and sandbox isolation;
the [backend capabilities table](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities)
compares them. Because only claude panes display the member name, use the
`pane_id` column of `cafleet member list` to locate a member's pane.

## Complete member prompts

Use the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) for a complete role-by-path member prompt. Preserve supplied identities, BASE and host prerequisites.

## Director wake

The Director receives replies through its backend's native inbox tools; [the manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains reception, consumption and the manual fallback.
