# Coding agents

Select a member backend through the CLI’s `--coding-agent` choices. Mixed-backend members share the same broker contracts. The value is recorded in the
placement's `coding_agent` column.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |

Plain `cafleet setup` installs assets for every supported backend
in one run; `--coding-agent` narrows the selection — see
[CLI options § cafleet setup](../spec/cli-options.md#cafleet-setup).

The flag means slightly different things per command:

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on.
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member inherits the spawning Director's placement backend (an explicit
  value still wins).

New members run through the CAFleet PTY runner with a declared detection-coverage state. Select native permission behavior with `--permission-mode`; omission retains the backend default. Interactive native policies are eligible on every backend — OpenCode's through its `cafleet-ask` agent preset — with request-bound Director/user decisions under the Director session’s effective mode or manual original-surface input. Detector, composer and answer coverage are independent; unknown coverage holds generated input. Broker access remains supplied by each backend’s launch configuration, subject to native and managed rules. [Member permission events](member-permission-events.md) explains the workflow, and [Spawn argv](../spec/coding-agent-backends.md#spawn-argv) specifies child arguments and sandbox boundaries.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders four identity
placeholders to literals, so the member reads its ids as plain text lines
(e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on every
command. Each placeholder and its label line is in
[CLI options § Spawn-prompt substitution](../spec/cli-options.md#spawn-prompt-substitution). The member environment carries
`CAFLEET_DATABASE_URL` and the selected backend's config-location variable
when the creator sets it — see
[CLI options § Member environment](../spec/cli-options.md#member-environment).
Each backend honors a leading-`!` shell shortcut on the coding agent's input line,
including opencode `mini`; backend-specific permission limits are in
[Coding-agent backends](../spec/coding-agent-backends.md).
`cafleet member exec` submits a visible native command through a tested runner composer profile; explicitly unwrapped members retain Escape-first input. A member routes a
command its harness does not run through the Director — see
[Command routing](member-lifecycle.md#command-routing). For the full
broker CLI reference, see [CLI options](../spec/cli-options.md).

## One-shot command isolation {#one-shot-command-isolation}

Run every one-shot `cafleet` command as the only command of its shell-tool
invocation, and run a sequence of CAFleet operations as separate shell-tool
calls. This rule is backend-neutral and applies identically to `claude`,
`codex`, and `opencode` panes.

Keep each actual process explicit for the host's permission rules. Submit
commands separated by a newline, `;`, `&&`, a pipe or shell `&` as individual
invocations. Opencode's permission matching for compound shell lines is
unverified; its [permission limits](../spec/coding-agent-backends.md#safety-floor-caveats)
apply alongside command isolation.

Leading `NAME=value` environment assignments immediately preceding the
`cafleet` executable are allowed: they set the CAFleet process's environment
without starting another process. Shell redirection does not authorize another
process either; a command that needs a long body should use the positional
argument or `--file <path>` rather than a pipe.

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
own model flag; omission uses that binary's default. Accepted formats and
create-time validation are in
[Model selection](../spec/coding-agent-backends.md#model-selection).

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to
the spawned backend binary. Unlike `--model`, the accepted level set is
validated per backend at create time, before any registration or multiplexer
side effect; omit the flag and the binary uses its own default. Per-backend
accepted levels, forwarding forms, and rejection strings are in
[Coding-agent backends § Reasoning effort](../spec/coding-agent-backends.md#reasoning-effort).

## Known asymmetries (intentional non-goals) {#known-asymmetries-intentional-non-goals}

| Dimension | `claude` | `codex` | `opencode` |
|---|---|---|---|
| Reasoning effort | supported | supported | not supported |
| Pane title | supported, via `--name` | not supported | not supported |
| Sandbox isolation | not supported — the operator's allow list is the safety floor | supported — OS-level, kernel-enforced | not supported — a shell allowlist that denies or, under `cafleet-ask`, asks for every other command |

`--effort` with the `opencode` backend exits 2 with
`opencode does not support reasoning effort.` before any side effect. Because
`codex` and `opencode` panes do not display the member name, the `pane_id`
column of `cafleet member list` is ground truth for all three. Operators who
need kernel-enforced isolation should use the `codex` backend.

## Complete member prompts

Use the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) for a complete role-by-path member prompt. Preserve supplied identities, BASE and host prerequisites.

## Director wake

Director reception uses the executing backend's native stdout tools, with one owned receiver and explicit ACK consumption. The installed CAFleet backend instructions own concrete recipes; a host satisfying the [Director requirements](../spec/coding-agent-backends.md#role-requirements) may use explicit manual broker reads when a verified adapter is unavailable and states that reception limit. Follow [the manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle), preserving host continuation controls and pending user decisions.
