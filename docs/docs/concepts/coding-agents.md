# Coding agents

Select a member backend through the CLI’s `--coding-agent` choices. Mixed-backend members share the same broker contracts. The value is recorded in the
placement's `coding_agent` column.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |
| `hermes` | Hermes Agent | Reads the cafleet skill files by absolute path |

Plain `cafleet setup` installs assets for every supported backend
in one run; `--coding-agent` narrows the selection. [Quickstart](../quickstart.md#configure)
lists what setup installs for each backend and the configuration you add.

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on.
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member inherits the spawning Director's placement backend (an explicit
  value still wins).

Run the root Director on a backend whose Director verdict is `yes`, and spawn members on backends whose Member verdict is `yes`; a backend whose Director verdict is `no` runs only as a member. [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records each backend's verdicts with their evidence. A Director backend needs native goal lifecycle controls and must deliver the output of the Director's background listener to the session while the listener runs, at the latest at the session's next turn boundary; [Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) defines both requirements and the probe.

## Launch modes

New members run through the CAFleet PTY runner with a declared detection-coverage state. Every launch mode is classified `auto` or `manual`. An `auto` mode is native automatic review or a fixed allow/deny policy, and a `manual` mode asks a human on the native surface for operations its rules do not settle. Omitting `--permission-mode` selects the backend's default mode, which is an `auto` mode on every backend:

| Backend | Default mode | Other `auto` modes | `manual` modes |
|---|---|---|---|
| `claude` | `auto` | `dontAsk`, `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` |
| `codex` | `approve-for-me` | `never` | `on-request` |
| `opencode` | `cafleet` | — | `cafleet-ask` |
| `hermes` | `smart` | — | `manual` |

Hermes has no launch flag for its mode: CAFleet records the `smart` or `manual` choice, and the `approvals.mode` of your Hermes configuration governs the session. [Quickstart](../quickstart.md#hermes) covers the configuration.

When the member's bundled runner profile recognizes an approval prompt, the Director handles it as a runtime event ([Consume a runtime event](members.md#consume-a-runtime-event)). A prompt the profile does not recognize, such as one in a pane narrower than the profile's captured layouts, is answered manually on the member's native surface. A Member-eligible backend's profile recognizes every approval prompt its default mode can show, outside its recorded layout limits ([Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements)). Broker access remains supplied by each backend’s launch configuration, subject to native and managed rules. [Choose a native policy](members.md#choose-a-native-policy) covers mode selection and profile coverage, and the [spawn argv contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#per-backend-build_spawn_argv-exact-token-by-token) specifies child arguments, defaults and sandbox boundaries.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders four identity
placeholders to literals, so the member reads its ids as plain text lines
(e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on every
command. The [spawn-prompt substitution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#spawn-prompt-substitution)
lists each placeholder and its label line. The member environment carries `CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and the selected backend's config-location variable when the creator sets it; `CAFLEET_MEMBER_ID` only marks the process for the [Director-only command guard](members.md#command-routing).
Each backend honors a leading-`!` shell shortcut on the coding agent's input line, including the minimal `mini` interface that OpenCode member panes run; [backend capabilities](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities) compares their sandbox and shell-command posture.
`cafleet member exec` submits a visible native command through the runner, using the profile's measured native-exec plan or delivery fallback and queueing it while a detected native interaction is open. A member routes a
command its harness does not run through the Director — see
[Command routing](members.md#command-routing). The
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

A bare Claude or GPT model reference selects Claude Code or Codex on every fleet, including an OpenCode fleet, and a provider-prefixed reference such as `opencode/glm-5.3` selects OpenCode. Naming OpenCode for the member selects OpenCode for that model, and a bare name offered only by OpenCode selects OpenCode when it uniquely matches a catalog ID. For an OpenCode model named without a provider, the Director uses the OpenCode Go form when your configured providers offer it, otherwise the Console form. The Director asks you to resolve an unknown, ambiguous or mismatched reference, or an OpenCode model your providers do not offer. The installed Director policy owns the exact resolution rules.

Use the selected backend's role defaults and catalog as the source of truth.

Hermes has no maintained model catalog. The Director selects `hermes` only when
you name the backend or the fleet's Director runs on Hermes, never from a model
name. A Hermes member omits `--model` and `--effort` by default, so your
configured default model and provider apply; a model you name passes through
verbatim, provider-prefixed ids included. Cost efficiency mode has no catalog
prices to compare for Hermes, so the Director asks you to choose.

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

Evidence of insufficient capability can trigger a replacement with a strictly stronger model on the same backend, a bounded number of times per task. The installed Director policy owns evidence, handoff, limits and selection; a user-pinned model requires an operator decision before replacement.

`cafleet member create --model <string>` forwards the value to the backend's
own model flag; omission uses that binary's default. Claude, Codex and Hermes
pass any value through and let the binary reject unknown models, so a newly released
model needs no CAFleet release. OpenCode requires the `<provider-id>/<model-id>`
form, checked when the member is created. The
[model selection contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#model-selection)
has the exact format and error.

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to
the spawned backend binary. Unlike `--model`, the accepted level set is
validated per backend at create time, before any registration or multiplexer
side effect; omit the flag and the binary uses its own default. OpenCode has no
reasoning-effort control, so omit the flag for OpenCode members. Hermes
forwards the level as its own reasoning flag. The
[reasoning effort contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#reasoning-effort)
lists each backend's levels and forwarding form.

## Backend differences

The backends differ in reasoning effort, pane titles and sandbox isolation;
the [backend capabilities table](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities)
compares them. Because only claude panes display the member name, use the
`pane_id` column of `cafleet member list` to locate a member's pane.

A Hermes member's session can keep accepted work and background processes
running after its pane's client exits. `cafleet member delete` stops that
session's work and the background processes it registered, and keeps the
member when it cannot verify the stop; CAFleet never touches your Hermes
gateway or sessions it does not own.
