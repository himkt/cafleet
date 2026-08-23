# Keep the Codex Monitor Alive with a Managed Execution Session

**Status**: Approved
**Progress**: 13/13 tasks complete
**Last Updated**: 2026-08-23

## Overview

Correct the Codex coding-agent overlay so the monitor member runs `cafleet monitor <fleet-id>` without shell `&` and keeps the resulting managed execution session alive. Align shared lifecycle wording, contract documentation, and regression tests while leaving the CLI/runtime implementation and the Claude/OpenCode launch behavior unchanged.

## Success Criteria

- [x] The Codex overlay launches `cafleet monitor <fleet-id>` without a shell ampersand, retains the managed execution-session ID, inspects the initial output, and performs at most one immediate poll to observe `monitor loop started (...)`.
- [x] Codex withholds `monitor live` and reports startup failure when no managed session is returned, the session exits early, or the startup line remains absent after the bounded confirmation sequence; a still-active unconfirmed session is terminated first.
- [x] On every later Codex monitor-member turn reopened by a broker message, the retained session is polled once before other work; an exited loop follows the existing relaunch and `monitor restarted` flow.
- [x] The Codex `{bg_stop}` value describes interrupting or terminating the retained managed execution session.
- [x] The monitor member alone owns the Codex execution-session ID and all launch/liveness polling; the Director runs no timer, sleep loop, or session poll loop and reacts only to broker signals.
- [x] Claude continues to use its background-task facility and OpenCode continues to use its existing shell-backgrounding behavior; neither backend's overlay contract changes.
- [x] Shared skill, specification, and public-documentation prose no longer claims that every backend hosts the loop as the same kind of backgrounded shell command.
- [x] Section-scoped documentation tests enforce the new Codex contract and preserve the Claude/OpenCode contracts.
- [x] The targeted Rust documentation-contract tests and public documentation build pass without changes to Rust CLI/runtime source.

---

## Background

[Issue #339](https://github.com/himkt/cafleet/issues/339) documents that a shell child started through Codex with `&` dies when the command session returns. A blocking command run without `&` instead yields a Codex-managed execution session that remains alive and can be polled, which matches the long-lived monitor loop's lifecycle.

The current Codex section of `skills/cafleet/reference/coding-agent-overlays.md` resolves `{bg_run}` to a backgrounded `!` shell command and gives `cafleet monitor <fleet-id> &` as its worked example. Shared role and documentation pages also describe the monitor as a universally backgrounded task or command, obscuring the fact that the launch primitive differs by coding-agent backend.

---

## Specification

### Scope and invariants

| Area | Required decision |
|---|---|
| CLI/runtime | No change. `cafleet monitor FLEET_ID [--tick N] [--interval N]`, its blocking behavior, startup line, runtime-row claim, signals, and exit codes remain intact. |
| Data model | No change. The managed execution-session ID is transient Codex tool state retained by the monitor member; it is not stored in CAFleet's database or added to member placement metadata. |
| Codex overlay | Replace shell backgrounding with a retained managed execution session, including startup polling, later-turn liveness polling, and managed-session stop semantics. |
| Claude overlay | Preserve `run_in_background: true`, `TaskStop`, and the existing worked-resolution behavior. |
| OpenCode overlay | Preserve the backgrounded `!` shell command, recorded-process stop behavior, and `cafleet monitor <fleet-id> &` worked example. |
| Shared prose | Describe a backend-resolved long-lived execution rather than asserting a single background-task mechanism across all backends. |
| Polling policy | Startup uses a bounded event sequence: inspect the initial managed-execution yield, then perform at most one immediate session poll if the startup line is absent. Later liveness polling occurs once at the start of a turn reopened by a broker message. Do not add a sleep loop or wall-clock timeout. |
| Ownership | The monitor member exclusively launches, retains, polls, restarts, and stops its Codex managed execution. The Director never owns or receives the session ID and never runs a timer, sleep loop, or managed-session poll loop. |
| Generated artifacts | None are added. `cafleet/build.rs` embeds the tracked `skills/` inputs at build time, so updating those inputs and validating the build is sufficient. |

### Codex overlay contract

Keep the existing nine-placeholder vocabulary. In the Codex section only:

- Resolve `{bg_run}` to a short noun phrase describing a retained Codex-managed execution session created by running the command without shell `&`.
- Resolve `{bg_stop}` to interrupting or terminating that retained managed execution session.
- Add a Codex-specific note, attached to `{bg_run}` / `{bg_stop}`, that carries the lifecycle detail without making the placeholder values too long for inline substitution.
- Rewrite the Codex worked resolution to show `cafleet monitor <fleet-id>` with no trailing `&`, retention of the returned session ID, the bounded initial-output-plus-one-poll confirmation sequence, termination of an active but unconfirmed session, and sending `monitor live` only after confirmation.
- Do not edit the Claude or OpenCode placeholder values, notes, or worked commands except for a strictly mechanical shared-file adjustment required by surrounding prose.

The command and shell-backgrounding constraints are exact:

```text
Codex:    cafleet monitor <fleet-id>
OpenCode: cafleet monitor <fleet-id> &
```

### Codex managed-session lifecycle

Every action in this lifecycle belongs to the monitor member. The Director has no access to the managed execution-session ID and does not launch or poll the command; it reacts only to CAFleet broker signals from the monitor member, including `monitor live`, startup-failure reports, and `monitor restarted`.

| Phase | Required behavior | Failure behavior |
|---|---|---|
| Launch | Invoke `cafleet monitor <fleet-id>` without `&` and retain the active managed execution-session ID returned by Codex. | No active session ID is a failed start. Do not send `monitor live`; report the failure to the Director. |
| Confirm startup | Inspect the initial managed-execution output for `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`. If it is absent and the session is active, perform exactly one immediate poll of the same session. This is an event-count bound, not a sleep loop or wall-clock timeout. | If the session exits before the line appears, fail immediately. If the line remains absent after the one immediate poll, interrupt or terminate the still-active session, withhold `monitor live`, and report startup failure to the Director. |
| Enter normal operation | After startup confirmation, send `monitor live`, retain the session ID, and allow the monitor member's turn to end while the managed execution continues. | Never infer liveness merely from having received a session ID. |
| Later turns | Only when a broker message reopens the monitor member's turn, poll the retained session once before any other work. If it is still active, continue with the triggering work; do not create an independent timer or sleep-and-poll loop. | If it exited, relaunch without `&`, retain the new session, repeat the same bounded startup confirmation, and then report `monitor restarted`. A failed relaunch is reported to the Director instead of claiming a restart. |
| Stop/teardown | Use the managed-session interruption or termination operation represented by Codex `{bg_stop}` when an explicit stop is needed. Existing pane-first fleet teardown remains governed by `reference/recovery.md`. | Do not fall back to launching or managing an untracked shell-background child. |

The session ID lives only in the monitor member's conversation/tool context, matching the role's existing in-memory quiet-state notes. It is never included in a broker message or exposed to the Director. This change does not add persistence, cross-member access, or a new CAFleet command.

### Backend-neutral monitor role wording

`skills/cafleet/roles/monitor.md` remains the normative startup and standing-liveness protocol. Revise it so that:

1. Startup calls the loop a backend-resolved long-lived execution using `{bg_run}`, not universally a background task.
2. Startup confirmation remains a hard gate: the startup line must be observed before `monitor live` is sent.
3. Startup confirmation for Codex is bounded to the initial yield plus one immediate poll. A missing handle, early exit, or absent startup line after that poll is a failed start; an active unconfirmed execution is terminated before the failure is reported.
4. The standing obligation distinguishes push-style exit notifications from overlay-required polling. The Codex note requires one retained-session poll before other work only when a broker message reopens a later turn; Claude/OpenCode retain their existing observation behavior.
5. Ownership stays with the monitor member: it alone retains and polls the session ID. The Director receives only broker status signals and never starts a polling loop of its own.
6. Relaunch reports `monitor restarted` only after the replacement execution has emitted the startup line.

No new overlay placeholder is introduced. The existing `{bg_run}` and `{bg_stop}` values plus the Codex note carry the backend-specific mechanics.

### Dependent contract and documentation surfaces

Audit all tracked occurrences that describe how the monitor loop is hosted. At minimum, update these surfaces where their present wording assumes a universal background task or command:

| Surface | Required adjustment |
|---|---|
| `skills/cafleet/reference/supervision.md` | Replace the claim that the loop is “just a backgrounded command” on every backend with a backend-resolved long-lived-execution description; retain identical heartbeat semantics and state that the monitor member, never the Director, owns the execution handle and liveness checks. |
| `skills/cafleet/reference/cli.md` | Describe the loop as a long-lived execution owned by the monitor member, without prescribing one backend's launch primitive. |
| `skills/cafleet/roles/director.md` | Make the recovery/shutdown summary backend-neutral and consistent with the authoritative pane-first shutdown ordering. |
| `.claude/skills/skill-author/SKILL.md` | Remove the authoring guidance's universal background-task claim while preserving the cross-backend heartbeat requirement. |
| `SPEC.md` | Update the loop-form and monitor-heartbeat descriptions to separate blocking runtime behavior from backend-specific hosting. |
| `docs/docs/concepts/monitoring.md` | Update the overview, spawn, recovery, standing-liveness, and ownership language; document the Codex managed-session distinction and the Director's broker-only reaction role without turning the concepts page into a tool manual. |
| `docs/docs/spec/cli-options.md` | Keep the command contract unchanged while removing background-task wording from the summary and loop-form description. |
| `docs/docs/spec/webui-api.md` | Describe CLI-only launch as monitor-member-owned long-lived execution rather than a universal background task. |

The generic documented fallback values in `skills/cafleet/SKILL.md` remain defaults for an unknown or silent backend section; they do not override Codex's explicit overlay and need not be changed solely to mirror Codex.

### Regression coverage

Extend `cafleet/tests/docs_sync.rs` using its existing `overlay_section` and `assert_terms_in` helpers:

- Add a Codex-section test requiring managed-session, retained-session-ID, no-shell-ampersand, initial-output inspection, one immediate poll, startup-line gating, termination of an unconfirmed active session, broker-reopened later-turn liveness, and managed-session stop concepts.
- Reject the exact obsolete Codex worked command `cafleet monitor <fleet-id> &` within the Codex section. Do not ban `&` globally because the Codex prose must explain its absence and OpenCode intentionally retains the command.
- Add or extend section-scoped assertions that Claude still contains `run_in_background: true` / `TaskStop` and OpenCode still contains its backgrounded-shell / recorded-process behavior and ampersand worked command.
- Add required/absent terms for shared contract pages so the false universal “backgrounded command works identically on any backend” claim cannot return and the monitor-member-only ownership boundary remains explicit.

No process-lifecycle integration test or new Rust unit test is required because this change does not alter the executable. Run validation in this order:

1. `cargo test --manifest-path cafleet/Cargo.toml --test docs_sync` for the targeted documentation-contract tests.
2. `mise //cafleet:test` for the full existing Rust test suite, including the embedded-skill build path.
3. `mise //docs:install` to install the pinned public-doc dependencies, including in a clean or CI-like checkout.
4. `mise //docs:build` for the production public-documentation build.

Finish with the tracked-source audit described in Step 4.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Correct the Codex overlay contract

- [x] Update Codex `{bg_run}`, `{bg_stop}`, and the attached note in `skills/cafleet/reference/coding-agent-overlays.md` to define the retained managed-session lifecycle without adding a placeholder. <!-- completed: 2026-08-23T17:58 -->
- [x] Rewrite only the Codex worked resolution to remove shell `&`, retain the session, apply the initial-yield-plus-one-poll startup bound, terminate an active unconfirmed session, and gate `monitor live` on the startup line. <!-- completed: 2026-08-23T17:58 -->
- [x] Verify the Claude and OpenCode overlay sections retain their current launch, stop, and worked-example semantics. <!-- completed: 2026-08-23T17:58 -->

### Step 2: Align the normative skill lifecycle

- [x] Update `skills/cafleet/roles/monitor.md` startup, failure, later-turn liveness, restart, stop, and monitor-member-only ownership wording to support both managed sessions and existing backend primitives. <!-- completed: 2026-08-23T18:05 -->
- [x] Update `skills/cafleet/reference/supervision.md`, `skills/cafleet/reference/cli.md`, and the recovery summary in `skills/cafleet/roles/director.md` to use backend-neutral lifecycle language and keep the Director broker-reactive rather than execution-owning. <!-- completed: 2026-08-23T18:05 -->
- [x] Audit other tracked skill guidance, including `.claude/skills/skill-author/SKILL.md`, and replace only lifecycle wording that incorrectly universalizes shell/background-task behavior. <!-- completed: 2026-08-23T18:05 -->

### Step 3: Synchronize specification and public documentation

- [x] Update both monitor lifecycle descriptions in `SPEC.md` without changing the CLI or runtime contract. <!-- completed: 2026-08-23T18:10 -->
- [x] Update `docs/docs/concepts/monitoring.md` to explain backend-resolved hosting, monitor-member-only session ownership, the Director's broker-only reaction role, Codex later-turn polling, failure gating, and confirmed restart behavior. <!-- completed: 2026-08-23T18:10 -->
- [x] Update `docs/docs/spec/cli-options.md` and `docs/docs/spec/webui-api.md` to remove backend-specific launch assumptions while preserving their command/API semantics. <!-- completed: 2026-08-23T18:10 -->

### Step 4: Add regression checks and verify

- [x] Add Codex-section positive and obsolete-command-negative assertions to `cafleet/tests/docs_sync.rs`. <!-- completed: 2026-08-23T18:19 -->
- [x] Add Claude/OpenCode preservation assertions and shared-contract wording checks to `cafleet/tests/docs_sync.rs`. <!-- completed: 2026-08-23T18:19 -->
- [x] Run `cargo test --manifest-path cafleet/Cargo.toml --test docs_sync`, `mise //cafleet:test`, `mise //docs:install`, and `mise //docs:build`, in that order. <!-- completed: 2026-08-23T18:19 -->
- [x] Audit the final diff and tracked text hits to confirm no Rust CLI/runtime source changed, no extra generated artifact is required, and only OpenCode retains the shell-ampersand worked command. <!-- completed: 2026-08-23T18:19 -->
