# Member pane ID naming

**Status**: Approved
**Progress**: 16/16 tasks complete
**Last Updated**: 2026-11-19

## Overview

Name each ordinary member pane `<member_id> - <member_name>` (for example `12 - drafter`) instead of the bare member name, so operators read the member ID that `member prompt`, `exec`, `press` and `delete` take directly from the pane. The same name becomes the Claude and pi `--name` value, because those agents retitle their tmux pane after startup.

## Success Criteria

- [ ] A member spawned by `member create` on tmux gets `tmux select-pane -t <pane> -T "<member_id> - <member_name>"`.
- [ ] A member spawned on herdr gets `herdr pane rename <pane> "<member_id> - <member_name>"`.
- [ ] A Claude or pi member's child argv carries `--name "<member_id> - <member_name>"`.
- [ ] One formatter produces the member pane name; every consumer calls it with the allocated member ID.
- [ ] Window/tab names, the Director pane title/label, the registry and every `member` command output are unchanged.
- [ ] `SPEC.md`, the `docs/` pages and the cafleet skill references state the new pane name; `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

`member create` passes the bare `--name` value to two independent title mechanisms:

| Mechanism | Where it is set today | Effect |
|---|---|---|
| Multiplexer pane name | `spawn_member_pane` in `src/multiplexer/tmux.rs` (`select-pane -T`) and `src/multiplexer/herdr.rs` (`pane rename`), reached through `NativeLaunchHost::split_runner` (`src/runtime/lifecycle.rs`), which `src/cli/member.rs` constructs with the member name | tmux pane title; herdr pane label |
| Backend `--name` | `CodingAgent::build_spawn_argv_with_policy` (`src/coding_agent/mod.rs`) receives `display_name`, which `src/cli/member.rs` passes as the member name | Claude sets `#{pane_title}` to the value; pi titles its terminal `π - <name> - <cwd basename>` |

On tmux, a Claude or pi child retitles its pane after CAFleet's `select-pane -T`, so changing only the multiplexer name would leave those panes showing the bare name. A herdr label is independent of the agent's terminal title.

`register_pending_run` allocates the member ID before the pane spawn and before the `prepare` closure builds the backend argv, so the ID is available at every naming point. `NativeLaunchHost` is constructed earlier, before the ID exists, which is why the name moves onto the `split_runner` call.

---

## Specification

### Member pane name

```rust
// src/multiplexer/mod.rs
pub(crate) fn member_pane_name(member_id: i64, member_name: &str) -> String {
    format!("{member_id} - {member_name}")
}
```

| Property | Value |
|---|---|
| Format | Decimal member ID, the literal three-character separator ` - ` (space, hyphen, space), the member name exactly as given to `member create --name` |
| Example | Member 12 named `drafter` → `12 - drafter` |
| Not included | Fleet ID, padding, truncation, escaping |
| Storage | None; the name is derived at spawn time and never persisted |

The formatter lives in the multiplexer module because the pane name is a multiplexer concept; the backend `--name` value reuses it so the two titles never diverge.

### Consumers

| Consumer | Change |
|---|---|
| `LaunchHost::split_runner` (`src/runtime/lifecycle.rs`) | Signature becomes `fn split_runner(&mut self, pane_name: &str, argv: &[String], deadline: Instant) -> Result<SpawnedPane>` |
| `launch_registered` (`src/runtime/lifecycle.rs`) | Calls `host.split_runner(&member_pane_name(pending.member.member_id, &pending.member.name), &argv, deadline)` |
| `NativeLaunchHost` (`src/runtime/lifecycle.rs`) | Drops its `pane_name` field and constructor parameter; `split_runner` forwards the received `pane_name` to `Multiplexer::spawn_member_pane`. Its doc comment becomes `/// Spawns the runner pane at \`target\`, starting in \`cwd\`, named by each \`split_runner\` call.` |
| `member create` (`src/cli/member.rs`) | Constructs `NativeLaunchHost::new(&*mux, &target.pane, &cwd_text)`; inside the `prepare` closure passes `&member_pane_name(pending.member.member_id, name)` as `launch_argv`'s `display_name` |
| Every other `LaunchHost` implementation and `NativeLaunchHost::new` call (the `LaunchFixtureHost` and `with_native_host` test hosts in `src/runtime/lifecycle.rs`, the `CaptureVerificationLaunchHost` in `src/runtime/observation.rs`) | Adopts the new `split_runner` signature and constructor; a wrapping host forwards `pane_name` unchanged |

`Multiplexer::spawn_member_pane`, `CodingAgent::launch_argv` and `build_spawn_argv_with_policy` keep their signatures: each still receives a ready-made opaque name, so the tmux/herdr naming tests and the per-backend argv tests stay valid without edits.

### Unchanged behavior

- Naming stays best effort: a failed `select-pane -T` or `pane rename` leaves the member created and reports nothing.
- Only panes spawned by `member create` after the change get the new name; running members keep their current titles.
- Overflow window and Director window names (`cafleet-<fleet_id>-<ordinal>`), the Director pane title/label, the registry schema and the `member create` / `member show` / `member list` output are unchanged.
- Backends without a `--name` analog (codex, opencode, hermes) are unaffected beyond the multiplexer name.

### Documentation changes

| Target | Change |
|---|---|
| `SPEC.md` §6.5 Window and pane naming | Member pane row: Name becomes `<member_id> - <member_name>`, e.g. `12 - drafter` |
| `SPEC.md` §6.5 Member pane creation | The spawn paragraph ("names it after the member") names the pane with the member pane name (§ Window and pane naming); the tmux `select-pane -t <new pane> -T <member name>` sentence and the herdr step `2. herdr pane rename <new pane> <member name>` print `<member pane name>` |
| `SPEC.md` §6.3 `member create` | `--name` row adds that the member pane name derives from it (§6.5 Window and pane naming) |
| `SPEC.md` §6.7 Backend capabilities | Claude and pi rows read `--name <member pane name>`; pi's terminal title becomes `π - <member pane name> - <cwd basename>`; the note beneath says CAFleet names every member pane with the member pane name |
| `SPEC.md` §6.7 Interface | The `build_spawn_argv(prompt, display_name, cwd, model, effort)` bullet defines `display_name` as the member pane name (§6.5 Window and pane naming); §6.7 Per-backend `build_spawn_argv` keeps its per-backend "`display_name` becomes `--name`" wording |
| `docs/docs/concepts/members.md` | The pane-naming paragraph states that each pane carries `<member ID> - <member name>`, e.g. `12 - drafter`, the ID the `member` commands take |
| `docs/docs/concepts/coding-agents.md` | The backend-differences paragraph says herdr labels and tmux titles every member pane with its member ID and name |
| `assets/skills/cafleet/reference/cli.md` | `member create` paragraph: CAFleet names every member pane `<member_id> - <name>`; pi argv shows `--name <member_id> - <name>` |
| `assets/skills/cafleet/reference/coding-agents.md` | claude and pi `{pane_title}` bindings use `<member_id> - <member-name>`; the pi Note → applies at row's argv reads `--name <member_id> - <name>` |

`README.md` and `.claude/rules/` need no change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update SPEC.md

- [x] Update the §6.5 Window and pane naming member pane row, and in §6.5 Member pane creation the spawn paragraph, the tmux `select-pane -T` sentence and the herdr `pane rename` step, to the member pane name <!-- completed: 2026-11-19T10:22 -->
- [x] Update the §6.3 `member create` `--name` row <!-- completed: 2026-11-19T10:22 -->
- [x] Update §6.7 Backend capabilities (Claude and pi rows, note) and define `display_name` as the member pane name in the §6.7 Interface `build_spawn_argv` bullet <!-- completed: 2026-11-19T10:22 -->

### Step 2: Update user documentation

- [x] Update the pane-naming paragraph in `docs/docs/concepts/members.md` <!-- completed: 2026-11-19T10:23 -->
- [x] Update the backend-differences paragraph in `docs/docs/concepts/coding-agents.md` <!-- completed: 2026-11-19T10:23 -->

### Step 3: Update skill references

- [x] Update the `member create` paragraph in `assets/skills/cafleet/reference/cli.md` <!-- completed: 2026-11-19T10:24 -->
- [x] Update the claude and pi `{pane_title}` bindings and the pi Note → applies at argv row in `assets/skills/cafleet/reference/coding-agents.md` <!-- completed: 2026-11-19T10:24 -->

### Step 4: Add the formatter

- [x] Add `member_pane_name` to `src/multiplexer/mod.rs` with a unit test asserting `member_pane_name(12, "drafter") == "12 - drafter"` and that a name containing spaces or ` - ` is kept verbatim <!-- completed: 2026-11-19T10:48 -->

### Step 5: Pass the name through the launch path

- [x] Change `LaunchHost::split_runner` to take `pane_name`, drop `NativeLaunchHost`'s `pane_name` field and constructor parameter, rewrite its doc comment for the new shape, and forward the received name to `spawn_member_pane` <!-- completed: 2026-11-19T10:48 -->
- [x] Make `launch_registered` call `split_runner` with `member_pane_name(pending.member.member_id, &pending.member.name)` <!-- completed: 2026-11-19T10:48 -->
- [x] Adopt the new signature in `LaunchFixtureHost`, `with_native_host` and `CaptureVerificationLaunchHost` <!-- completed: 2026-11-19T10:48 -->
- [x] Record the received `pane_name` in `LaunchFixtureHost` and add a `src/runtime/lifecycle.rs` unit test asserting it equals `<allocated member_id> - <member name>` for a created member <!-- completed: 2026-11-19T10:48 -->

### Step 6: Use the name for the backend `--name`

- [x] In `src/cli/member.rs`, construct `NativeLaunchHost` without the name and pass `member_pane_name(pending.member.member_id, name)` as `launch_argv`'s `display_name` <!-- completed: 2026-11-19T10:48 -->
- [x] Extend `member_create_renders_identity_and_database_into_the_prompt_and_the_runner_argv` to assert the descriptor argv contains `--name` followed by `<member_id> - worker` <!-- completed: 2026-11-19T10:48 -->

### Step 7: Verify

- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-19T10:48 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-19T10:48 -->
