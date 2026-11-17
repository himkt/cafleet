# Contributing

Develop changes through reviewed designs and the bundled CAFleet workflows, running repository tasks through their full mise monorepo paths. [SPEC.md](https://github.com/himkt/cafleet/blob/main/SPEC.md) is the specification for the CLI, runtime and storage contracts.

## Project structure

| Entry | Responsibility |
|---|---|
| `src/` | Rust CLI, typed synchronous SQLite broker, multiplexer/process adapters; built by the root `Cargo.toml` and `build.rs` together with the embedded `assets/` tree |
| `assets/` | Data files: `assets/skills/` installed agent-facing skills, roles and references; `assets/presets/` backend permission/configuration assets; `assets/extensions/` one directory per backend whose plugin or extension `cafleet setup` installs, for Director reception, goals and, where the backend has no native per-tool approval, the member gate; `assets/migrations/` the SQL schema baseline; `assets/profiles/` the bundled runner-profile catalog and one native screen fixture file per backend with catalogued screens. The binary embeds every file except the native screen fixtures, which the test suite reads |
| `docs/` | Independent operator documentation and its Node/pnpm site tooling |
| `design-docs/` | Numbered design documents and local evidence |

## Rust boundaries

Broker queries return typed records; CLI presenters construct wire output. Shared diagnosis validates database identity and schema. Keep durable storage separate from synchronous notification, runner-owned preview retries, input ownership and multiplexer I/O. Required state fails at its read boundary. Preserve process pipe draining, owned-child cleanup, pane ownership compensation and layout cleanup.

Every Rust test is an in-file unit test; [Test placement](https://github.com/himkt/cafleet/blob/main/.claude/rules/code-quality.md#test-placement) owns the convention.

## Development

```bash
git clone https://github.com/himkt/cafleet.git
```

Enter the checkout, then install the CLI through its root task:

```bash
mise //:install
```

```bash
cafleet setup
```

Use `mise tasks --all` as the task registry. The root `mise.toml` pins tools, owns the Rust tasks and selects the documentation site's task file through `monorepo.config_roots`. Run Rust validation with `mise //:test` and `mise //:lint`. The release build needs Rust and embedded skills/presets. Reinstall the binary after Rust changes to use them.

### Installing the skills from your checkout

Setup installs the assets embedded at binary build time; run `mise //:skill-install` after setup to install your working-tree skills instead. Preserve an active executing team's installed assets until its workflow finishes.

## Building docs locally

The documentation site retains its Node/pnpm toolchain and lockfile. Run `mise //docs:install`, then `mise //docs:build`; the build checks links. Use `mise tasks --all` for preview and other available documentation operations. Update dependencies through the manifest and regenerate its pnpm lockfile deliberately.

## Contributing changes

Follow the [design document workflow](concepts/overview.md#prompts). Preserve role boundaries, clarification, review and user approval. The active session's host rules determine Git and artifact handling.

## Documentation style

State intended behavior directly and describe growing groups through their selecting property or registry; [documentation maintenance](https://github.com/himkt/cafleet/blob/main/.claude/rules/documentation-maintenance.md) owns audiences, shared enumerations, tables and literal formatting.

## Extend a coding-agent backend

Implement a typed permission adapter as the single source of truth for launch-policy choices/default, literal argv mapping and versioned detector/composer/answer profiles. Add a new profile only when an observed native change requires different rules or plans, with `min_version` set to the first native version requiring it; [profile selection](https://github.com/himkt/cafleet/blob/main/SPEC.md#profile-selection-by-minimum-version) owns the profile stack, capability independence and coverage. Preserve omitted defaults, native/managed restrictions and sandbox boundaries.

These herdr references, pinned to commit `5608de0608918aba2f9d501e7793b984a0ca9cfa`, provide implementation patterns; each CAFleet profile still requires its own native contract and acceptance evidence.

| Pinned reference | Purpose |
|---|---|
| [Backend manifests](https://github.com/herdrdev/herdr/tree/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifests) | Backend-specific live-screen/title rules and controls that distinguish current forms from old transcript text. |
| [Manifest evaluator](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifest.rs) | Region extraction, priority and combined matching conditions for adapting rules with explicit conflicts. |
| [Pane processing](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane.rs) | Ordered PTY-byte processing into terminal state and detector observations. |
| [Terminal snapshots](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane/terminal.rs) | Live screen and title evidence without treating scrollback as a current request. |

Map herdr’s broad `blocked` state into CAFleet’s typed interaction classes, returned-result observations and input-eligibility states. Questions, trust and update screens retain their own classes; a permission request needs the live operation and choice/input region together. Conflicting strongest rules produce an unknown interaction and preserve the native surface. CAFleet owns its runner, profiles, classification, persistence and notification; herdr APIs remain diagnostic, not runtime detection dependencies.

Use safe PTY/terminal APIs behind deterministic transport traits; the crate sets `unsafe_code = "deny"`. The single exception is the macOS process identity read: one `sysctl(CTL_KERN, KERN_PROC, KERN_PROC_PID)` call into a fixed, size-checked `kinfo_proc` buffer, carrying an item-scoped `#[allow(unsafe_code)]` with its safety invariants stated inline; add no other unsafe code. Every safe start-time interface applies a same-effective-user check that fails for setuid descendants in the owned process group. The selected PTY and terminal-screen crates are `portable-pty` and `vt100`; resolve compatible versions through Cargo and retain the lockfile. Separate terminal transport, classification and event storage, and preserve bounded shutdown independently of SQLite waits.

Evaluate a candidate backend against the role bar before it ships:

1. Open a design doc for the candidate with a `## Role evaluation` section: a `Role | Requirement | Source | Delivery | Eligible | Evidence` table with Director `goal`, `listen` and `verdict` rows and one Member row, shaped like the [SPEC Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) table.
2. Check each role's Eligible-tier requirements in [SPEC §6.7 Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) against an installed native version, recording the version per requirement group: official documentation and installed `--help` / `--version` for host capabilities, CAFleet PTY probes on each registered multiplexer for runtime behavior. Record the Director `goal` and `listen` sources (`native` or `extension`) and the `listen` at the highest tier established: `subscribe` delivery from a passing [subscribe probe](#run-a-cafleet-subscribe-probe), or `compromise` delivery from a passing [stream probe](#run-a-cafleet-stream-probe) together with the negative evidence Role requirements defines.
3. A verdict is `yes` only when every requirement is established, otherwise `no` with "not established" or the observed failure; the Director `verdict` is `yes` only when `goal` and `listen` are both `yes`, otherwise it names the unmet requirements.
4. For a Member-eligible candidate, claim detection and request-bound allow and deny answers for every approval form the default mode renders, and record the profile's layout limits (`W_min`, wrapped or clipped operations, uncaptured geometries); a default mode rendering no approval form needs no request rules. Claim other bundled-support capabilities with the evidence in the table below; an unclaimed optional capability ships as `unknown` coverage.
5. On shipping, copy the rows into the SPEC Role eligibility table, summarize each role as a `Role | Eligible | Basis` row (sources, delivery, checked versions and runtime caveats) in the backend's Role eligibility subsection of `assets/skills/cafleet/reference/coding-agents.md`, and ship only the roles marked `yes`.

| Capability | Required native evidence |
|---|---|
| Permission detection | Live captures of each permission form the claimed policies reach. |
| Automatic review annotations | Captures of each automatic refusal and unavailable-review notice in the idle, working and overlay states where the native host shows them. |
| Composer plans | For prompt and native exec each, empty-composer, typed-literal and post-submit captures with separate literal and submit-key stages. |
| Request-bound answers | Allow and deny pre-stage captures and a continuation capture for each decision. Pin single-request semantics, focus/key stages, request-token revalidation and human/output race behavior. |

Every claimed detector/composer/answer capability requires CAFleet-owned positive and adversarial replay fixtures, including redraw, wrapping, fragmented ANSI/UTF-8, alternate screens, competing forms and quoted transcript false positives. Every capture is real native PTY evidence from a native version within the profile's interval on each registered multiplexer, including concurrent output/input and the observed-state race boundary. A submission plan is measured, not an assumed Enter mapping. [SPEC §6.9 Native acceptance and coverage](https://github.com/himkt/cafleet/blob/main/SPEC.md#native-acceptance-and-coverage) owns the minimum policies, evidence fields and fault cases. A launch-help probe supplies no detector/composer/answer claim; missing minimum native evidence fails feature acceptance.

### Run a CAFleet subscribe probe

The subscribe probe establishes a tier-1 (native) or tier-2 (extension) subscribe listen as SPEC Role requirements defines it. The candidate Director starts its receiver through the mechanism under test, such as a native tool or a CAFleet plugin tool, and a probe member sends the messages. A probe message ID is *observed in a notice* when a `pending` record the receiver delivers names it.

1. Prepare the isolated database, Director session and probe member as in steps 1–3 of the stream probe below, and start the receiver after the Director consumes the ready. Then give the Director the instruction below the steps.
2. **Idle.** With the Director idle, send one message. A notice naming it must start a turn without user input, and the poll it prompts must return it.
3. **Busy.** Ask the Director to run a foreground command of at least 60 s, such as `sleep 90`, and send two messages while it runs. The command must not be interrupted; a notice must arrive no later than the end of that turn, and the poll it prompts must return both IDs.
4. **Burst.** Ask the Director to run a foreground command of at least 60 s and under 300 s, and send 150 messages back to back while it runs. At most two notice turns may arrive for the burst, every ID must be consumed through a notice-prompted poll, and no ID may appear in two notices. On Codex the forwarder must stay alive with no `codex queue` refusal.
5. **Consumed notice.** Direct one turn: a 30 s foreground command while m1 is sent, one poll, a second 30 s foreground command while m2 is sent, one poll, then end the turn. Every notice turn delivered after that turn must name only consumed IDs and run no command, and no ID may appear in two notices.
6. **Hold ceiling.** Say `hold`, then send `hold-probe 1` to the idle Director and record t1, the time its notice turn starts; the Director ends that turn without polling. After that turn ends and 60 s have passed since t1, send `hold-probe 2`. No notice may arrive before t1 + 285 s, and one notice naming both IDs must arrive between t1 + 285 s and t1 + 315 s; its IDs are above the watermark, so the Director polls once and the poll must return both.
7. **Owner exit.** End the Director session the receiver belongs to, and confirm with `ps` that no `cafleet message listen` survives it.
8. Record the backend, native version, mechanism, message IDs, send timestamps, each notice's delivery turn, timestamp and named IDs, poll timestamps and the receiver's exit in the design doc's evidence record.

The Director instruction for step 1:

> Consume broker messages only through your receiver. On each `[cafleet listen record]` turn, end the turn with no commands when every named ID is at or below your consumed watermark; otherwise run `cafleet message poll <director-id> --json` once and raise the watermark. Run no other `poll` or `show` unless a step below directs it. When I say `hold`, end the next notice turn without polling.

A `poll` or `show` that first reveals a probe ID without a preceding notice turn invalidates the run; the poll a notice prompts is part of consumption. The ±15 s hold-ceiling tolerance covers the receiver latency between the listener's delivery and t1 and the listener's one-second check interval. Probe each mechanism separately; the backend's Director reception recipe owns the concrete tool calls.

### Run a CAFleet stream probe

The stream probe establishes a tier-3 built-in compromise listen as SPEC Role requirements defines it: the Director reads its background listener's output while supervision continues. The candidate starts `cafleet message listen` through its native background-execution facility and keeps working on something unrelated in the same session; the run classification below the steps grades each run. Probe each native mechanism that can read a running background process separately. A pass is recorded only with tier-1 and tier-2 negative evidence, and the backend's Director reception recipe owns the concrete tool calls.

1. Create an isolated database in a fresh short absolute directory, such as one from `mktemp -d`, so the member runtime socket path stays within the platform limit; record its location and run `cafleet setup --database-url sqlite:///<probe dir>/director.db`. Pass the same `--database-url` on every `cafleet` command of the probe.
2. In a tmux pane, start the candidate Director session and register it with `cafleet fleet create --name <name> --coding-agent <backend>`, adding the pane ID and server it asks for when it cannot confirm the candidate's pane ([Director pane verification](concepts/members.md#director-pane-verification)).
3. Have the Director spawn one probe member on any Member-eligible backend, with a spawn prompt that carries the `{database_url}` placeholder, sends `ready` and then ends its turn without further broker traffic; its ID is the sender identity.
4. After the Director consumes the ready, have it start `cafleet message listen <director-id> --timeout 300` through the mechanism under test, then send the task prompt below the steps.
5. From a separate shell, send three probe messages at 20 s, 60 s and 120 s after the task prompt: `cafleet message send --from-member-id <probe-id> --to-member-id <director-id> "stream-probe 1"`, then `2` and `3`. Record each printed message ID and timestamp.
6. Classify the run by the run classification below the steps. Capture the Director pane and retain the session records that show each probe ID's first observation.
7. Record the backend, native version, mechanism (tool call and flags), message IDs, send timestamps, each probe ID's first observation (the tool result or injected event carrying the `pending` line that names it, and its timestamp), poll timestamps, the listener's exit timestamp, the task-turn end timestamp and the classification in the design doc's evidence record.

The task prompt for step 4 sets `<mechanism instruction>` to "between steps, read your background listener with `<tool call>`" for a pull mechanism and "consume each record your background listener delivers" for a push mechanism:

> Summarize the files `<file 1>`, `<file 2>`, … one at a time. `<mechanism instruction>`. Learn about broker messages only from that listener's output; run no other `poll` or `show`. When a `pending` record appears there, run `cafleet message poll <director-id> --json` once and continue. Keep working until a poll has returned `stream-probe 3` or 240 seconds have passed since this prompt, then end your turn.

Classify each run as follows; the 240-second turn bound leaves 60 seconds of the 300-second lease for turn-boundary delivery.

| Classification | Condition |
|---|---|
| Invalid | The task turn ended before 240 s without a poll having returned `stream-probe 3`; the task turn was still running when the listener exited; a probe ID was first observed outside a `pending` record, or the Director ran any other `poll` or `show`; every probe ID was first observed in a notice while the listener ran but the poll consuming one ran at or after the listener exited; or user input was given after the task prompt. Repeat the run, up to three runs per mechanism. |
| Pass | A valid run in which every probe ID was first observed in a `pending` record delivered through the mechanism under test and consumed by a notice-prompted poll before the listener exited. |
| Fail | A valid run in which any probe ID was first observed only after the listener exited, or was never observed in a `pending` record. |

A mechanism with three invalid runs is recorded as "not established" with the invalidating cause.
