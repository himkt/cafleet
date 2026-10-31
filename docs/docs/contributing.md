# Contributing

Develop changes through reviewed designs and the bundled CAFleet workflows. Run repository tasks from the root using mise full-path notation. [SPEC.md](https://github.com/himkt/cafleet/blob/main/SPEC.md) is the specification for the CLI, runtime and storage contracts.

## Project structure

| Entry | Responsibility |
|---|---|
| `cafleet/` | Rust CLI, typed synchronous SQLite broker, multiplexer/process adapters and embedded assets |
| `skills/` | Installed agent-facing skills, roles and references |
| `presets/` | Backend permission/configuration assets |
| `docs/` | Independent operator documentation and its Node/pnpm site tooling |
| `design-docs/` | Numbered design documents and local evidence |

## Rust boundaries

Broker queries return typed records; CLI presenters construct wire output. Shared diagnosis validates database identity and schema. Keep durable storage separate from synchronous notification, finite receiving, input ownership and multiplexer I/O. Required state fails at its read boundary. Preserve process pipe draining, owned-child cleanup, pane ownership compensation and layout cleanup.

Every Rust test is a unit test in a `#[cfg(test)] mod tests` block of the implementation file it exercises. Shared test helpers live in a `#[cfg(test)] pub(crate) mod test_support` beside the type they build, and test inputs stay inline in the test module.

## Development

```bash
git clone https://github.com/himkt/cafleet.git
```

Enter the checkout, then run repository-root tasks:

```bash
mise //cafleet:install
```

```bash
cafleet setup
```

Use `mise tasks` and package task definitions as the task registry. Run Rust validation with `mise //cafleet:test` and `mise //cafleet:lint`. The release build needs Rust and embedded skills/presets. Reinstall the binary after Rust changes to use the new implementation.

### Installing the skills from your checkout

Setup installs assets embedded at binary build time. Contributors can install working-tree skills with `mise //:skill-install`; run it after setup when you want those local files. Preserve an active executing team's installed assets until its workflow finishes.

## Building docs locally

The documentation site retains its Node/pnpm toolchain and lockfile. Run `mise //docs:install`, then `mise //docs:build`; the build checks links. Use the docs task registry for preview and other available operations. Update dependencies through the manifest and regenerate its pnpm lockfile deliberately.

## Contributing changes

Follow the [design document workflow](how-to/design-doc-development.md#prompts). Preserve role boundaries, clarification, review and user approval. The active session's host rules determine Git and artifact handling.

## Documentation style

Write human/operator behavior in the site and agent instructions in skills. State intended behavior directly. Link each shared enumeration to its owner; describe growing groups through their selecting property or registry. Use tables when parallel entries carry multiple shared attributes, with at least two data rows and at most two sentences per cell. Keep literal commands, flags, paths and errors in code spans.

## Extend a coding-agent backend

Implement a typed permission adapter as the single source of truth for launch-policy choices/default, literal argv mapping, native-version eligibility and versioned detector/composer/answer profiles. Each profile declares a SemVer `min_version`; a backend's profiles form an ordered stack, and a new profile is added only when an observed native change requires different rules or plans, with `min_version` set to the first native version requiring it. Preserve omitted defaults, native/managed restrictions and sandbox boundaries. Launchability, permission detection, composer input and request-bound answers are separate capabilities; unknown coverage supports manual terminal work, and generated input without a measured submission plan uses delivery fallback.

The following herdr references are pinned to commit `5608de0608918aba2f9d501e7793b984a0ca9cfa`. They provide implementation patterns; each CAFleet profile still requires its own native contract and acceptance evidence.

| Pinned reference | Purpose |
|---|---|
| [Backend manifests](https://github.com/herdrdev/herdr/tree/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifests) | Backend-specific live-screen/title rules and controls that distinguish current forms from old transcript text. |
| [Manifest evaluator](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifest.rs) | Region extraction, priority and combined matching conditions for adapting rules with explicit conflicts. |
| [Pane processing](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane.rs) | Ordered PTY-byte processing into terminal state and detector observations. |
| [Terminal snapshots](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/pane/terminal.rs) | Live screen and title evidence without treating scrollback as a current request. |

Map herdr’s broad `blocked` state into CAFleet’s typed interaction classes, returned-result observations and input-eligibility states. Questions, trust and update screens retain their own classes; a permission request needs the live operation and choice/input region together. Conflicting strongest rules produce an unknown interaction and preserve the native surface. CAFleet owns its runner, profiles, classification, persistence and notification; herdr APIs remain diagnostic rather than runtime detection dependencies.

Use safe PTY/terminal APIs behind deterministic transport traits; the crate sets `unsafe_code = "deny"`. The single exception is the macOS process identity read, one `sysctl(CTL_KERN, KERN_PROC, KERN_PROC_PID)` call into a fixed, size-checked `kinfo_proc` buffer. Every safe start-time interface applies a same-effective-user check that fails for setuid descendants in the owned process group. That call carries an item-scoped `#[allow(unsafe_code)]` with its safety invariants stated inline; add no other unsafe code. The selected PTY and terminal-screen crates are `portable-pty` and `vt100`; resolve compatible versions through Cargo and retain the lockfile. Separate terminal transport, classification and event storage, and preserve bounded shutdown independently of SQLite waits.

Evaluate a candidate backend against the role bar before it ships:

1. Open a design doc for the candidate. It carries a `## Role evaluation` section with a `Role | Eligible | Evidence` table: one Director row and one Member row.
2. For each role, check every Eligible-tier requirement in [SPEC §6.7 Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) against an installed native version, recording the version per requirement group: official documentation and installed `--help` / `--version` for host capabilities, and CAFleet PTY probes on each registered multiplexer for runtime behavior.
3. Derive each verdict: `yes` only when every requirement is established; otherwise `no` with "not established" or the observed failure.
4. For a Member-eligible candidate, choose the bundled-support capabilities to claim and collect the evidence in the table below; an unclaimed capability ships as `unknown` coverage.
5. When the backend ships, copy the verdicts into its Role eligibility subsection in `skills/cafleet/reference/coding-agents.md` and the [SPEC Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) table, and ship only the roles marked `yes`.

| Capability | Required native evidence |
|---|---|
| Permission detection | Live captures of each permission form the claimed policies reach. |
| Automatic review annotations | Captures of each automatic refusal and unavailable-review notice in the idle, working and overlay states where the native host shows them. |
| Composer plans | For prompt and native exec each, empty-composer, typed-literal and post-submit captures with separate literal and submit-key stages. |
| Request-bound answers | Allow and deny pre-stage captures and a continuation capture for each decision. Pin single-request semantics, focus/key stages, request-token revalidation and human/output race behavior. |

Every claimed detector/composer/answer capability requires CAFleet-owned positive and adversarial replay fixtures, including redraw, wrapping, fragmented ANSI/UTF-8, alternate screens, competing forms and quoted transcript false positives. Every capture is real native PTY evidence on a native version within the profile's interval on each registered multiplexer, including concurrent output/input and the observed-state race boundary. A submission plan is measured, never an assumed Enter mapping. [SPEC §6.9 Native acceptance and coverage](https://github.com/himkt/cafleet/blob/main/SPEC.md#native-acceptance-and-coverage) owns the minimum policies, evidence fields and fault cases. A launch-help probe supplies no detector/composer/answer claim; missing minimum native evidence fails feature acceptance.
