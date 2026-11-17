# Fleet Window Naming

**Status**: Complete
**Progress**: 6/6 tasks complete
**Last Updated**: 2026-11-16

## Overview

Name every newly registered fleet's windows or tabs `cafleet-<fleet_id>-<ordinal>`, with the Director window at ordinal `0`. Apply names at creation time by overwriting the existing Director window/tab name and retaining the existing overflow naming scheme.

## Success Criteria

- [x] Successful `fleet create` names the resolved Director window/tab `cafleet-<fleet_id>-0`, and its returned and stored placement contain that exact name.
- [x] Naming and verification use the resolved backend/server and concrete IDs, including explicit `--pane` and `--mux-server` targets.
- [x] A naming or verification failure returns an error and rolls back uncommitted registration; any applied name remains in place and the Director pane keeps running.
- [x] Overflow names, ordinal allocation, layout, and ordinary pane naming retain their current behavior. Existing registrations remain usable with their recorded names.

---

## Background

`src/cli/fleet.rs::create_with` currently reads the Director window's existing name and passes it to `broker::create_fleet`, which allocates the fleet ID and commits registration. The Director already occupies ordinal `0`; `src/multiplexer/layout.rs::overflow_window_name` formats subsequent windows correctly.

This document replaces the implementation proposal in `design-docs/0000229-director-pane-window-naming/design-doc.md`. That approved, unimplemented document remains unchanged; this proposal covers window/tab naming only.

---

## Specification

### Naming contract

| Surface | Creation-time behavior |
|---|---|
| Existing Director window/tab | Overwrite its name with `cafleet-<fleet_id>-0` |
| New overflow window/tab | Use `cafleet-<fleet_id>-<ordinal>` with the existing increasing ordinal allocation |
| Director placement | Record the canonical name with the resolved server, session, window ID and pane ID; ordinal `0`, column `left` |

Rename `overflow_window_name` to `fleet_window_name(fleet_id, ordinal)` and update every caller and test. The same formatter supplies ordinal zero and positive ordinals; the first overflow remains ordinal `1`, and failed member creation can still consume an ordinal.

Naming happens during creation. Existing fleets retain their recorded names, and subsequent user renames leave placement snapshots unchanged. Fleet deletion leaves the current Director window/tab name in place. The logical Director name, Director pane title/label, ordinary pane naming, and agent launch behavior retain their current contracts. Use the existing schema, CLI arguments and output shapes.

### Multiplexer interface

Add this operation to `Multiplexer`, implemented with `BackendCore::argv` and the injected runner:

```rust
fn rename_window(&self, window_id: &str, name: &str) -> Result<(), MultiplexerError>;
```

| Backend | Command, with each value passed as one argv element |
|---|---|
| tmux | `tmux -S SOCKET rename-window -t WINDOW_ID NAME` |
| herdr | `herdr --session SERVER tab rename TAB_ID NAME` |

Propagate command failures. herdr also validates its normal JSON result envelope with the existing `run_json` helper. The generated names contain only the fixed prefix, decimal IDs and separators, so they can be passed directly. tmux's `rename-window` disables `automatic-rename` for the target window ([upstream implementation](https://raw.githubusercontent.com/tmux/tmux/master/cmd-rename-window.c)); use that command without additional option-management machinery.

Retain `window_name` for verification. Its tmux implementation must remove exactly the final output LF, split at the first `|`, require the echoed window ID, and preserve the remaining name exactly. Its herdr implementation must require `result.tab.tab_id` to equal the requested tab ID and require a string `label`. Missing fields, mismatched IDs and malformed output return errors. This makes an exact comparison with the canonical name meaningful.

### Registration sequence

Keep multiplexer orchestration in `src/cli/fleet.rs`. Extend the broker's transaction-owning registration logic with a placement factory that receives the allocated fleet ID:

```rust
pub(crate) fn create_fleet_with(
    conn: &mut Connection,
    name: &str,
    director: impl FnOnce(i64) -> Result<NewPlacement, CafleetError>,
) -> Result<CreatedFleet, CafleetError>;
```

Keep `create_fleet(conn, name, director)` as a database-only wrapper supplying a cloned placement, so existing fixtures and the native verification driver retain their current interface. Both entry points share one registration implementation and the current Director-placement assertions.

The public CLI follows this sequence:

1. Resolve and verify the Director pane and server using the existing explicit-pane or verified-environment path. Read its concrete session, window and pane IDs.
2. Begin the broker registration transaction and insert the fleet row to obtain its actual ID. Invoke the placement factory once within that transaction.
3. In the factory, format ordinal zero, rename the resolved window/tab, and read its name back. Require exact equality with the requested name. Re-read the Director pane context and require the original session, window ID and pane ID.
4. Return a `NewPlacement::director` containing the canonical name. Insert the logical Director and placement, backfill the fleet's Director reference, and construct `CreatedFleet` from rows read inside the transaction.
5. Commit registration, then emit the existing success output. Every fallible registration read occurs before commit.

Replace the initial window-name read with the rename followed by verification. The previous name is irrelevant to this operation. The existing overflow creation paths continue passing the shared formatter's result to tmux `new-window -n` and herdr tab creation.

### Failure behavior

Treat the requested Director window name as a requirement for successful registration. Use the existing application-error path and exit code `1`; a failure before confirmed commit emits no success payload, including under `--json`.

| Failure | Required outcome |
|---|---|
| Target resolution, pane-context read, transaction acquisition, or fleet-row insertion | Return the cause; roll back active uncommitted registration. |
| Rename command, name read-back, or context verification | Stop immediately and roll back active uncommitted registration. Leave the window/tab at its current name. |
| Director/placement insertion or result construction | Roll back active uncommitted registration; leave any applied name in place. |
| Commit failure | Return the cause. Roll back if the transaction remains active; report the registry outcome as unconfirmed if SQLite already ended the transaction. |
| Output failure after confirmed commit | Preserve registration and names; return the existing output error. |

Keep the transaction guard until the commit outcome is inspected, so an active failed transaction can be explicitly rolled back. Surface a rollback failure alongside the primary error. Once a rename has been attempted, errors identify the attempted fleet ID, backend/server, concrete window and pane IDs, requested name and cause; describe the terminal name as potentially changed and direct inspection through the existing fleet-list and explicit-server multiplexer commands. Attempted IDs are diagnostic, since rolled-back IDs can be reused.

Preserve the running Director pane throughout failure handling. Applied names remain after an error, crash, or fleet deletion; the design stores no previous naming state and performs no naming compensation or automatic creation retry. SQLite protects registration atomicity, while the terminal rename is a separate operation. A crash can therefore leave a changed name without registration, or committed registration without delivered output.

### Validation

Place Rust tests in each owning implementation file's existing `#[cfg(test)] mod tests` and reuse `FakeRunner` and SQLite failure injection.

| Owner | Required checks |
|---|---|
| `src/multiplexer/layout.rs` | Ordinal-zero formatting, positive ordinals, and unchanged slot/ordinal selection |
| Backend implementation files | Exact explicit-server rename argv; command failure; herdr envelope failure; exact name parsing and target-ID validation |
| `src/cli/fleet.rs` | Both backends; explicit and verified-environment targets; canonical returned/stored placement; rename/read failure, wrong name, moved/missing pane, and unchanged Director pane/ordinary pane naming |
| `src/broker/fleets.rs` | Factory receives the allocated ID once; factory/insertion failures roll back all registration rows; commit/rollback failures report their outcome; result construction precedes commit |
| Existing member/layout tests | Existing recorded names remain usable; members reuse the recorded Director name; first overflow remains `cafleet-<fleet_id>-1` |

Assert database state and issued commands together, including the absence of pane-title changes, pane closure and naming restoration during failed Director registration.

---

## Implementation

### Step 1: Share window naming and add the backend operation

- [x] Rename the formatter to `fleet_window_name`, update every reference, and cover ordinal zero with the existing layout tests. <!-- completed: 2026-11-16T09:45 -->
- [x] Add `rename_window` to the multiplexer trait and both backends; tighten the existing name readers and add the focused command/parser tests. <!-- completed: 2026-11-16T09:45 -->

### Step 2: Require naming before fleet registration commits

- [x] Introduce the shared broker registration factory and database-only wrapper; construct the result before commit and surface transaction-resolution errors. <!-- completed: 2026-11-16T09:56 -->
- [x] Wire the CLI factory to rename, read back, validate context and return the canonical placement; update fleet-create and broker tests for success and failure. <!-- completed: 2026-11-16T09:56 -->

### Step 3: Align the documented contract and validate

- [x] Update current naming and creation behavior in `SPEC.md`, `assets/skills/cafleet/reference/cli.md`, and the documentation pages describing windows and pane layout. Replace every obsolete Director-name-read description and distinguish required window naming from ordinary pane naming. <!-- completed: 2026-11-16T09:49 -->
- [x] Run `mise //:test`, `mise //:lint`, and `mise //docs:build`; confirm the original design remains unchanged and the focused change preserves legacy placement and overflow behavior. <!-- completed: 2026-11-16T09:59 -->
