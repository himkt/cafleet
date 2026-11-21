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

Every Rust test is an in-file unit test. Related cases form one table-driven test over a named case array; runner tests take time from the injected clock and use real time only when they drive real processes, PTYs, sockets or locks; and runner behavior is driven through the runtime scenario harness, which asserts events, rows and child bytes. [Test placement](https://github.com/himkt/cafleet/blob/main/.claude/rules/code-quality.md#test-placement) owns the convention.

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

Use `mise tasks --all` as the task registry. The root `mise.toml` pins tools, owns the Rust tasks and selects the documentation site's task file through `monorepo.config_roots`. Run Rust validation with `mise //:test` and `mise //:lint`; `mise //:coverage` reports line coverage of the inline unit tests. The release build needs Rust and embedded skills/presets. Reinstall the binary after Rust changes to use them.

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
| Composer plans | For prompt and native exec each, empty-composer, typed-literal and post-submit captures with separate literal and submit-key stages. |
| Request-bound answers | Allow and deny pre-stage captures and a continuation capture for each decision. Pin single-request semantics, focus/key stages, request-token revalidation and human/output race behavior. |

Every claimed detector/composer/answer capability requires CAFleet-owned positive and adversarial replay fixtures, including redraw, wrapping, fragmented ANSI/UTF-8, alternate screens, competing forms and quoted transcript false positives. Every capture is real native PTY evidence from a native version within the profile's interval on each registered multiplexer, including concurrent output/input and the observed-state race boundary. A submission plan is measured, not an assumed Enter mapping. [Release acceptance](#release-acceptance) owns the minimum policies, evidence fields and fault cases. A launch-help probe supplies no detector/composer/answer claim; missing minimum native evidence fails feature acceptance.

### Run a CAFleet subscribe probe

The subscribe probe establishes a tier-1 (native) or tier-2 (extension) subscribe listen as SPEC Role requirements defines it. The candidate Director starts its receiver through the mechanism under test, such as a native tool or a CAFleet plugin tool, and a probe member sends the messages. A probe message ID is *observed through a notice* when the one `poll` that a `pending` record the receiver delivers prompts returns it.

1. Prepare the isolated database, Director session and probe member as in steps 1–3 of the stream probe below, and start the receiver after the Director consumes the ready. Then give the Director the instruction below the steps.
2. **Idle.** With the Director idle, send one message. A notice must start a turn without user input, and the poll it prompts must return the message.
3. **Busy.** Ask the Director to run a foreground command of at least 60 s, such as `sleep 90`, and send two messages while it runs. The command must not be interrupted; a notice must arrive no later than the end of that turn, and the poll it prompts must return both IDs.
4. **Burst.** Ask the Director to run a foreground command of at least 60 s and under 300 s, and send 150 messages back to back while it runs. At most two notice turns may arrive for the burst, and every ID must be returned by a notice-prompted poll. On Codex the forwarder must stay alive with no `codex queue` refusal.
5. **Consumed notice.** Direct one turn: a 30 s foreground command while m1 is sent, one poll, a second 30 s foreground command while m2 is sent, one poll, then end the turn. Every notice turn delivered after the directed turn must run one poll, which returns `[]`.
6. **Hold ceiling.** Say `hold`, then send `hold-probe 1` to the idle Director and record t1, the time its notice turn starts; the Director ends that turn without polling. After that turn ends and 60 s have passed since t1, send `hold-probe 2`. No notice may arrive before t1 + 285 s; one notice must arrive between t1 + 285 s and t1 + 315 s, and the poll it prompts must return both IDs.
7. **Owner exit.** End the Director session the receiver belongs to, and confirm with `ps` that no `cafleet message listen` survives it.
8. Record the backend, native version, mechanism, message IDs, send timestamps, each notice's delivery turn and timestamp, each poll's timestamp and returned IDs and the receiver's exit in the design doc's evidence record.

The Director instruction for step 1:

> Consume broker messages only through your receiver. On each `[cafleet listen record]` turn, run the command in the record's `next` field once. Run no other `poll` or `show` unless a step below directs it. When I say `hold`, end only the next notice turn without running its command, then apply the rule above to every later notice turn, including the ceiling re-notice.

A `poll` or `show` that first reveals a probe ID without a preceding notice turn invalidates the run; the poll a notice prompts is part of consumption. The ±15 s hold-ceiling tolerance covers the receiver latency between the listener's delivery and t1 and the listener's one-second check interval. Probe each mechanism separately; the backend's Director reception recipe owns the concrete tool calls.

### Run a CAFleet stream probe

The stream probe establishes a tier-3 built-in compromise listen as SPEC Role requirements defines it: the Director reads its background listener's output while supervision continues. The candidate starts `cafleet message listen` through its native background-execution facility and keeps working on something unrelated in the same session; the run classification below the steps grades each run. Probe each native mechanism that can read a running background process separately. A pass is recorded only with tier-1 and tier-2 negative evidence, and the backend's Director reception recipe owns the concrete tool calls.

1. Create an isolated database in a fresh short absolute directory, such as one from `mktemp -d`, so the member runtime socket path stays within the platform limit; record its location and run `cafleet setup --database-url sqlite:///<probe dir>/director.db`. Pass the same `--database-url` on every `cafleet` command of the probe.
2. In a tmux pane, start the candidate Director session and register it with `cafleet fleet create --name <name> --coding-agent <backend>`, adding the pane ID and server it asks for when it cannot confirm the candidate's pane ([Director pane verification](concepts/members.md#director-pane-verification)).
3. Have the Director spawn one probe member on any Member-eligible backend, with a spawn prompt that carries the `{database_url}` placeholder, sends `ready` and then ends its turn without further broker traffic; its ID is the sender identity.
4. After the Director consumes the ready, have it start `cafleet message listen <director-id> --timeout 300` through the mechanism under test, then send the task prompt below the steps.
5. From a separate shell, send three probe messages at 20 s, 60 s and 120 s after the task prompt: `cafleet message send --from-member-id <probe-id> --to-member-id <director-id> "stream-probe 1"`, then `2` and `3`. Record each printed message ID and timestamp.
6. Classify the run by the run classification below the steps. Capture the Director pane and retain the session records that show each probe ID's first observation.
7. Record the backend, native version, mechanism (tool call and flags), message IDs, send timestamps, each probe ID's first observation (the notice-prompted poll that first returned it, the tool result or injected event carrying the `pending` line that prompted that poll, and their timestamps), poll timestamps, the listener's exit timestamp, the task-turn end timestamp and the classification in the design doc's evidence record.

The task prompt for step 4 sets `<mechanism instruction>` to "between steps, read your background listener with `<tool call>`" for a pull mechanism and "consume each record your background listener delivers" for a push mechanism:

> Summarize the files `<file 1>`, `<file 2>`, … one at a time. `<mechanism instruction>`. Learn about broker messages only from that listener's output; run no other `poll` or `show`. When a `pending` record appears there, run the command in its `next` field once and continue. Keep working until a poll has returned `stream-probe 3` or 240 seconds have passed since this prompt, then end your turn.

Classify each run as follows; the 240-second turn bound leaves 60 seconds of the 300-second lease for turn-boundary delivery.

| Classification | Condition |
|---|---|
| Invalid | The task turn ended before 240 s without a poll having returned `stream-probe 3`; the task turn was still running when the listener exited; a probe ID was first revealed outside a notice-prompted poll, or the Director ran any other `poll` or `show`; every `pending` record reached the session while the listener ran but a poll one of them prompted ran at or after the listener exited; or user input was given after the task prompt. Repeat the run, up to three runs per mechanism. |
| Pass | A valid run in which every probe ID was first returned, before the listener exited, by a poll that a `pending` record delivered through the mechanism under test prompted. |
| Fail | A valid run in which any probe ID was first returned only after the listener exited, or was never returned by a notice-prompted poll. |

A mechanism with three invalid runs is recorded as "not established" with the invalidating cause.

## Testing and release acceptance

[SPEC §9 Testing strategy](https://github.com/himkt/cafleet/blob/main/SPEC.md#9-testing-strategy) states the conformance levels; this section covers how the CAFleet test suite and release process check them.

### Bundled catalog provenance checks

The bundled-catalog guard tests in `src/coding_agent/catalog.rs` check every provenance rule of [SPEC Bundled profiles and native screen fixtures](https://github.com/himkt/cafleet/blob/main/SPEC.md#bundled-profiles-and-native-screen-fixtures) against the embedded catalog, so a build whose bundled backend files fail them does not pass `mise //:test`. The guard validates every catalog fixture entry, including screens that no profile capability cites. The production catalog loader skips the contents of the `fixtures` registry and of every `fixture_ids` value, so only the guard reads them, and fixture validation shares the SemVer comparison of profile selection. The guard reports these errors:

| Condition | Error |
|---|---|
| The source file is absent from the test suite's fixture bytes | `bundled fixture bytes are not embedded` |
| File `schema_version` differs from `1`, `provenance` differs from `native`, or `backend` differs from the entry | `bundled fixture file identity differs from its catalog entry` |
| The entry's `id` is absent from `screens` | `bundled fixture screen is missing` |
| A `screens` key has no catalog fixture entry | `bundled fixture file holds an uncited screen` |
| The canonical screen digest differs from the entry's `sha256` | `bundled fixture digest mismatch for <ID>: catalog <SHA>, computed <SHA>` |
| Screen `rows` or `cols` differ from the entry | `bundled fixture identity differs from its catalog entry` |
| A pattern-bearing field (a rule matcher, frame guard, region matcher, `prompt_pattern` or any other pattern the catalog decoder compiles) contains a Perl class or boundary (`\s \S \d \D \w \W \b \B`) or an escape outside the [pattern syntax](https://github.com/himkt/cafleet/blob/main/SPEC.md#pattern-syntax) | The profile, rule ID and construct, for example `` claude 2.1.302 rule claude-bash-command: `\s` `` |

Synthetic fixtures exercise rule and capability validation in the test suite. The guard rejects them for the bundled catalog, and explicitly selected test catalogs that use them pass the same provenance rules.

### Native screen fixture capture

The capture-to-fixture conversion inserts the converted screen under its catalog fixture `id` in `assets/profiles/native/<backend>.json`. It derives `cell_backgrounds` from the capture grid's per-cell `bg` and `cell_styles` from its per-cell faint and foreground attributes, and fixture replay applies each run's background and style before writing its row text. The terminal-screen parser gives a continuation cell default attributes whenever its leading cell receives a wide character, both on the runtime screen and in replay, so replay writes a run's attributes onto the leading cell and leaves the continuation cell to the parser. [SPEC Bundled profiles and native screen fixtures](https://github.com/himkt/cafleet/blob/main/SPEC.md#bundled-profiles-and-native-screen-fixtures) owns the fixture file format.

### Storage deadline mechanisms

The SQLite mechanisms below enforce the storage deadlines that SPEC states as outcomes:

- A `message listen` snapshot uses scoped busy waiting and an execution progress handler every 1,000 VM instructions, checking the deadline before and after prepare and execution, before and after decoding each row, and before commit. Every path restores busy handling and removes the listener-owned progress handler, preserving cleanup failures alongside the original cause.
- A runner SQLite attempt replaces the connection's default five-second busy wait with its bounded budget, cancels long-running statements within that deadline and restores the prior handler and connection hooks after the attempt.

### Release acceptance

Acceptance evidence for every declared detector, composer and answer capability consists of synthetic terminal replay and real native PTYs on each registered multiplexer.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Acceptance records pin exact installed CLI versions to their corresponding bundled profiles, and their permitted disposable fixtures actually reach the native approval surface. Every minimum case demonstrates blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and a preserved original request. Separate fresh requests cover verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, manual original-surface input and a user-executed complete bound command. The records show zero response bytes before required user permission. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Automatic-mode acceptance covers Claude `auto` and Codex `approve-for-me` on both tmux and herdr, with automatic review available for the account and model: the omitted-default launch, an auto-approved fixture, an auto-refused fixture whose refusal the member routes to the Director, and, where the native host shows one, an escalated form answered allow and deny. A later-version case repeats the launch, auto-approved, auto-refused and escalated cases on a native version above its profile's `min_version`, recording the selected profile and each UI difference that failed to match. A fixture that does not produce its intended screen is a recorded limit for that backend, not a pass.

Minimum-version delivery acceptance covers every bundled backend on each registered multiplexer, using isolated native sessions and databases and benign marker commands:

| Case | Required evidence |
|---|---|
| Threshold selection | Equal/between/above/below boundaries, catalog permutation, multiple backends, numeric component ordering, prerelease ordering, build-metadata ties, malformed thresholds and failed probes |
| Complete profile reuse | Detection, both composer operations and each available semantic answer resolve the same profile and retain actual run/capture identities |
| Native operation plans | Prompt preview and direct prompt create a native user turn; native exec runs a harmless marker command and exposes its result, with literal-to-submit timing recorded |
| Fallback operation | Missing plan and failed-probe cases show the rule-5 fallback sequence of [SPEC Delivery method and interaction priority](https://github.com/himkt/cafleet/blob/main/SPEC.md#delivery-method-and-interaction-priority) and explicit fallback results |
| Busy composer | A busy composer with an executable plan follows rule 4 (`composer_busy` with zero bytes and profile drain once empty, previews pending) without any Escape; a working member with an empty composer row receives profile delivery |
| Interaction deferral | Permission and question remain open with no Escape/literal bytes; answering on the original native surface yields closure, the queued prompt first and then the pending preview, without another send |
| Adversarial transition | A candidate or superseding interaction appearing during chunks/settles stops the write at its complete prefix, keyboard/pointer input there leaves the write running, and observer/owner loss and partial stages follow the complete-prefix stop outcomes |
| Durable retry | A queued prompt or exec drains by fallback and retains one ID/digest after composer changes and repeated queries; duplicate keys, conflicting digests, lost replies and storage failures avoid duplicate dispatch |
| Deferred failure and liveness | After closure with a busy pane lock and no further output/commands, the runner timer either delivers when the lock frees or publishes not submitted at the deadline; temporary storage/observer failure and terminal-commit recovery preserve FIFO/result visibility |
| Schema and reporting | Fresh-baseline initialization, canonical digest validation, referential integrity, sequence preservation, self-send/no-pane output and durable reason/state projection for ordinary and history receipts |

Composer-submit acceptance uses the installed evidence versions on both tmux and herdr for every bundled backend. Each record verifies versions from the capturing server's pane and records commands, screen fixtures and timings:

| Case | Required result |
|---|---|
| Pointer at 0.3 s | Hover, wheel and click during the settle or between literal chunks leave the write running and it submits; permission tokens/stages are revoked |
| Keyboard `x` at 0.3 s | The write continues and the mixed literal is submitted; post-submit verification records the outcome |
| Plain threshold | Codex previews of 1007 and 1037 bytes and Hermes previews of 2027 and 2057 characters submit using measured framing or pointer rendering |
| Positive short writes | Japanese previews of 7987 and 12085 bytes continue chunks and submit within 5 s |
| Ineffective submit key | Post-submit verification ends as `uncertain/submit_unconfirmed`, stage `submitted` |
| FIFO stop | After an uncertain prompt or exec, neither a queued exec nor a sweep preview writes a byte until `input reconcile` settles it, on runs with and without composer verification |
| Hermes deletion | Drafts of 2057 characters and 8192 scalars clear and work settlement completes; the larger draft clears within 10 s |

Unit coverage replays each unsubmitted-literal fixture through post-submit verification and obtains `submit_unconfirmed`, and exercises interaction stops, close-stage failures, schema constraints and projection/recovery actions. Measurements establish submitted native input separately from model consumption, permission decisions and command completion.

Live acceptance records carry the source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, request identity/token, receipt/stage, multiplexer, terminal size, cwd and metadata evidence. They include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. The exercised cases cover stale tokens/epochs/events, same/different operation keys, a second answer for an already answered episode, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, event-channel pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.

Acceptance also covers these transport, lifecycle and persistence cases:

- Exact child argv for the selected native policy, with native/managed restrictions and sandbox boundaries preserved.
- Equal run IDs across sibling databases, and replacement at an unchanged path.
- A held write transaction and a long statement, showing that restoration and exit are independent of the connection's default timeout.
- A child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.
- Timeout followed by the original runner's recovery, and concurrent recovery attempts.
- A command containing a known secret, which is absent from every database/message field.
- Hard kill with uncommitted transitions, loss closure after a committed opening, concurrent recovery, interruption during recovery, and deregistration before recovery with archived rather than fabricated inbox records.
- A live unlinked lock resolved by `member delete`, verified death, unknown identity reporting `live_or_unreachable` and concurrent database-only recovery.
- Cross-invocation accepted-then-failed-stamp recovery, queued-then-drained delivery after interaction closure, disconnected partial writes, receipt retirement followed by a delayed request, and a lost explicit-command stdout result.
- Every registered backend's omitted-policy argv and each adapter-declared explicit policy.
