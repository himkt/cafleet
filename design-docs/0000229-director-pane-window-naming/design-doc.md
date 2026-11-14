# Director Window and Pane Naming

**Status**: Approved
**Progress**: 0/8 tasks complete
**Last Updated**: 2026-11-14

## Overview

Have `cafleet fleet create` name the existing Director window or tab `cafleet-<fleet_id>-0` and its pane `director`. This extends the fleet's existing window naming scheme to ordinal zero and makes the Director easy to locate in both tmux and herdr.

## Success Criteria

- [ ] Successful creation names the resolved Director window/tab `cafleet-<fleet_id>-0`, names its pane `director`, and returns a Director placement containing the same window name.
- [ ] Every naming command targets the resolved server and concrete window/pane IDs, including when `--pane` and `--mux-server` select a different environment.
- [ ] Handled failures preserve the running Director pane, roll back uncommitted registration, attempt bounded restoration, and identify incomplete or uncertain recovery.
- [ ] Existing fleets remain usable, ordinary-member naming retains its current behavior, and the first overflow window remains `cafleet-<fleet_id>-1`.

---

## Background

`src/cli/fleet.rs::create_with` resolves and verifies the Director pane, reads its window name, and passes the placement to `broker::create_fleet`. The broker atomically inserts the fleet, its logical `Director` member, and its placement. That placement already has `window_ordinal = 0` and `column_side = left`; `fleets.last_window_ordinal` starts at zero.

`src/multiplexer/layout.rs::overflow_window_name` formats later windows as `cafleet-<fleet_id>-<ordinal>`. Ordinary-member creation already names panes through `tmux select-pane -T` or `herdr pane rename`, treating those member-title failures as best effort. Fleet creation currently performs neither rename.

---

## Specification

### Names and compatibility

| Surface | Value after successful creation |
|---|---|
| Existing Director window/tab | `cafleet-<fleet_id>-0` |
| Existing Director pane title/label | `director` |
| Logical member name | `Director` |
| Director placement | Existing backend, server, session, window ID and pane ID; canonical window name; ordinal `0`; column `left` |
| Overflow allocation | Existing increasing ordinals starting at `1` |

Rename `overflow_window_name` to `fleet_window_name(fleet_id, ordinal)` and update its callers and tests. The formatter supports ordinal zero and preserves the current output for positive ordinals. Derive the name from SQLite's allocated fleet ID within the registration transaction, so concurrent creation and rolled-back allocations follow SQLite's existing allocation rules.

Use the existing placement schema and CLI arguments/output shapes. `mux_window_name` remains a recorded snapshot, and concrete IDs remain the targeting identity. Existing fleets keep their stored names; ordinary-member creation continues to work with those placements. Shared windows remain supported: the latest successful creation supplies the visible name, while earlier fleet/member rows retain their snapshots. Creation renames the existing targets in place and preserves their layout and the Director's process. Fleet deletion keeps its existing Director-pane preservation and leaves the current names in place.

The scope is creation-time naming. Subsequent user renames and coding-agent pane-title updates retain their existing behavior. This task produces the design document; the Implementation section specifies subsequent source work.

### Multiplexer operations

Extend `Multiplexer` with a small Director-specific read/apply/restore contract. Keep command construction and response parsing in each backend, with all commands routed through `BackendCore::argv` and the injected runner.

| Operation | Contract |
|---|---|
| Read naming state | Capture the verified context, window name, pane title/label, and backend-specific restoration state. Validate returned IDs against the requested targets. |
| Apply Director names | Rename the existing window and pane in order; propagate every error. The caller treats any attempted mutation as potentially applied, including a command that reports failure. |
| Restore naming state | While the original write transaction remains active, restore affected fields, verify their values, continue independent restoration attempts after an error, and return all failures. |

Represent naming state with typed backend variants. For tmux, capture the original pane title and window name plus the window-local `automatic-rename` setting, distinguishing an explicit boolean from inheritance. For herdr, capture the original tab label and optional explicit pane label; an unlabeled pane is a valid state restored with `pane rename --clear`. Empty names/titles remain valid values. Missing required response objects, invalid IDs, and malformed option values fail before mutation.

Use these application commands, with each metavariable passed as one argv element:

| Backend | Window/tab command | Pane command |
|---|---|---|
| tmux | `tmux -S SOCKET rename-window -t WINDOW_ID NAME` | `tmux -S SOCKET select-pane -t PANE_ID -T director` |
| herdr | `herdr --session SERVER tab rename TAB_ID NAME` | `herdr --session SERVER pane rename PANE_ID director` |

Snapshot and verification reads use the same resolved server. Reuse `window_name` and `pane_context`, correcting the tmux name reader to preserve whitespace. For a name/title read, use the fixed format `#{window_id}|#{window_name}` or `#{pane_id}|#{pane_title}`, remove exactly the single final LF emitted by `display-message`, and split at the first `|` to validate the echoed ID. Retain the entire remaining string, including separators, leading/trailing spaces, tabs and original CR/LF characters; require the output terminator instead of trimming the response. Read herdr labels with `pane get`. Read tmux's window-local option without inherited values so restoration can distinguish setting its previous boolean from unsetting the override.

Encode every restored tmux name/title for both interpretation stages: replace each original `#` with `#{a:35}`, which emits a literal hash without recursively expanding the emitted content; then, if the resulting argv element ends in `;`, insert one additional backslash immediately before that final semicolon. Preserve existing backslashes and other characters. For example, the original `#{pane_id};` becomes `#{a:35}{pane_id}\;` as an argv element containing one backslash. Supply `--` before the restored positional window name so leading hyphens remain data. `select-pane` receives its encoded title as the `-T` argument. tmux removes the semicolon escape during argv parsing and evaluates the character expressions during format expansion, preserving format-looking and style-looking text. Apply the same encoding to canonical names for a single naming path. These rules follow the [rename-window implementation](https://raw.githubusercontent.com/tmux/tmux/master/cmd-rename-window.c), [select-pane implementation](https://raw.githubusercontent.com/tmux/tmux/master/cmd-select-pane.c) and [argument parser](https://raw.githubusercontent.com/tmux/tmux/master/cmd-parse.y).

Snapshot storage and parsing preserve the original string. Exact restoration additionally depends on the backend accepting that value: tmux 3.6a's [window-name setter](https://raw.githubusercontent.com/tmux/tmux/3.6a/window.c) VIS-escapes backslashes and control bytes, while its pane-title setter can reject control characters. Verify the result against the raw snapshot and report normalization or rejection as incomplete restoration, retaining the original value in recovery diagnostics. This checked compensation uses the existing naming APIs and a bounded attempt, rather than promising recovery that those APIs cannot provide for every original value.

`rename-window` disables tmux's `automatic-rename` for that window, providing the required automatic-renaming protection without a global setting change. Verify that it is effectively off before commit. Preserve `allow-rename`, `allow-set-title`, pane-border display settings and terminal-title configuration; application/user changes after creation retain normal tmux semantics. During compensation, restore the pane title and window name and read them back for exact equality, then restore or unset the original `automatic-rename` override and verify its local/inherited state. Restoring names precedes restoring the option because `rename-window` disables automatic renaming, while re-enabling it may legitimately rename the window again. Verify restored herdr labels as well. These behaviors follow the [tmux manual](https://man.openbsd.org/tmux#automatic-rename).

Ordinary-member spawning retains its current naming and best-effort policy. Director naming has a checked contract because a successful fleet creation promises the requested names. Naming operations retain focus and leave pane geometry and agent launch state unchanged.

### Registration and failure handling

Keep cross-system orchestration in `src/cli/fleet.rs`. Split the broker's current registration logic into a transaction-neutral insertion helper and its existing transaction-owning wrapper for database-only callers. The insertion helper receives a placement factory taking the allocated fleet ID, allowing the CLI to supply the canonical name before inserting the placement. Preserve the broker's existing Director-placement assertions and atomic registration tests.

The CLI performs this sequence:

1. Resolve the backend/server and verify the pane using the existing explicit-pane or ancestry-verification path.
2. Begin an immediate SQLite transaction, recheck the concrete pane context, and read the complete original naming state. Retain its write ownership through naming, verification and any compensation before rollback, serializing creators using the same database.
3. Insert the fleet, logical Director and canonical placement through the insertion helper. Construct the result from rows read inside the transaction.
4. Apply the window/tab name and then the pane name. Re-read the names and context, require the original concrete IDs/session, and require both requested names and tmux's effective `automatic-rename = off`.
5. Attempt `COMMIT` while retaining the transaction guard, for example through `tx.execute_batch("COMMIT")`, so an error can be inspected before the guard's drop behavior rolls back. A successful commit ends write ownership; emit the existing success result. The returned placement and committed placement contain the same canonical window name.

Pass the existing invocation deadline through creation, bounding backend calls with the existing runner/deadline facilities and database work with `with_database_budget`. A handled failure gets one separate five-second cleanup deadline for restoration, verification and transaction resolution together; every cleanup command shares that deadline. Track attempted mutations before dispatch and perform explicit compensation so cleanup failures reach the error result.

After an error, inspect `Connection::is_autocommit()` before dropping the transaction guard and before each restoration command. While it remains false, this connection still owns its original immediate transaction: restore and verify affected fields first, then explicitly roll back and confirm that the transaction has ended. Keep all compensation before that rollback, including when commit failed while leaving the transaction active. An error can cause SQLite to end a transaction automatically; when autocommit is already true, stop compensation and preserve the current terminal state because a successor may already own it. Report the lost ownership and any unresolved naming or registration outcome. This uses the existing transaction as the ownership boundary and needs no additional lock or schema field; see [SQLite autocommit state](https://sqlite.org/c3ref/get_autocommit.html).

Determine this attempt's outcome from its observed transaction lifecycle: successful commit confirms registration, and successful rollback of its still-active transaction confirms this attempt was discarded. A commit error followed by loss of transaction ownership is reported as unconfirmed. A later row lookup is diagnostic only: rolled-back fleet and member IDs can be reused, so even matching IDs/names do not prove that this attempt committed. Label IDs as attempted IDs in uncertain diagnostics and direct the operator to inspect actual registry/server state; see [SQLite allocation rules](https://sqlite.org/autoinc.html).

| Failure point | Required outcome |
|---|---|
| Resolution, context/snapshot read, transaction acquisition, or row insertion before naming | Return the underlying error and roll back any transaction; preserve terminal state. |
| Rename failure, malformed verification, target movement/disappearance, or work deadline expiry | With the original transaction active, restore and verify affected naming state, then roll back. Recheck target identity before restoring; report a moved/missing target rather than operating on its replacement. Lost transaction ownership stops restoration. |
| Commit error | Inspect transaction state before guard destruction. If still active, compensate under the write lock and then roll back; otherwise preserve terminal state and report the attempt's registration outcome as unconfirmed. |
| Restoration or rollback error, including cleanup timeout | Return the primary failure followed by every cleanup failure or unconfirmed operation; identify the backend, server, concrete targets and attempted names. Attempt rollback after compensation ends even if restoration is incomplete; report a failed rollback and allow invocation teardown to close the connection. |
| Output failure after confirmed commit | Keep the committed registration and names; report the output error through the existing path. |

On a confirmed rollback, this attempt's fleet/member/placement inserts have been discarded; a successor can subsequently reuse their IDs. Restoration uses the captured original values, including an empty title or unlabeled herdr pane. While write ownership is held, a field with an unexpected third-party value is preserved and reported as a restoration conflict. Recovery instructions identify the explicit-server commands or `cafleet fleet show FLEET_ID` needed to inspect the unresolved state. Errors exit `1` through the existing application-error path. Pre-commit failures emit no success payload, including under `--json`; an output failure can leave a partially written response after commit.

SQLite and the multiplexer have separate commit boundaries. A process kill or server loss between naming and commit can leave changed names with no registration; a crash after commit can leave a committed fleet without a delivered success result. SQLite protects registry atomicity, while compensation covers observed errors. Recovery uses recorded IDs and actual server state; this change introduces no persistent recovery journal or automatic retry of `fleet create`. Simultaneous external renames or creation through different databases remain outside the database transaction's serialization guarantee.

### Focused validation

Place Rust tests in the existing `#[cfg(test)] mod tests` of the implementation file owning the asserted behavior. Reuse `FakeRunner`, the existing fleet-create fixtures and SQLite failure injection.

| Owner | Required coverage |
|---|---|
| `src/multiplexer/layout.rs` | Ordinal-zero formatting and unchanged positive-ordinal output. |
| Backend implementation files | Exact explicit-server read/rename/restore argv; lossless snapshot parsing, including retained CR/LF; exact round trips for empty values, `#{pane_id}`, `#(...)`, `#[fg=red]`, multiple hashes before `[`, styles containing format-looking text, standalone/trailing semicolons, leading hyphens, separators and spaces; pane-title backslashes and reported window-name normalization; inherited/local option state; herdr label clearing; command/malformed-response errors; restoration read-back and tmux automatic-renaming verification. |
| `src/cli/fleet.rs` | Successful creation on both backends; explicit targets conflicting with environment; failures before mutation and after each attempted mutation; naming/context mismatch; insertion/commit errors with active versus automatically ended transactions; cleanup failures and deadline exhaustion; preservation of the running Director pane. |
| Broker and existing member/layout tests | Atomic registry insertion, returned/stored canonical-name agreement, unchanged logical Director identity and ordinal counter, shared-window snapshots, legacy Director placement reuse, and first overflow ordinal/name. |

Assert registry contents and side effects together. Use deterministic two-connection interleavings: while failed creator A compensates under its active transaction, creator B remains blocked; after A rolls back, B reuses its fleet ID and succeeds, and A issues no later rename. Also force A's transaction to end automatically before cleanup, allow B to reuse that ID and commit, then verify that A skips restoration and reports uncertainty without misidentifying B's row. Failure tests check the original server/IDs and preservation of pane processes, focus and geometry. Verify that an output failure after commit preserves the registered fleet. A focused disposable tmux smoke check should verify the special-character round trips against the real parser during implementation; keep it isolated from user panes and the default database.

Design-time validation against an isolated tmux 3.6a server passed exact window/title round trips for empty values, literal formats/jobs, styles and mixed style/format text, semicolons, leading hyphens, separators and surrounding spaces. The probe also confirmed the normalization limitation above and removed its disposable server/socket afterward.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Define checked Director naming operations

- [ ] Generalize the shared window-name formatter to `fleet_window_name`, update every reference, and cover ordinal zero alongside the existing overflow behavior. <!-- completed: -->
- [ ] Add typed naming snapshots, lossless tmux parsing/encoding and checked read/apply/restore operations to the multiplexer interface and both backends, preserving ordinary-member naming behavior. <!-- completed: -->

### Step 2: Coordinate naming with registration

- [ ] Extract transaction-neutral broker registration with a fleet-ID placement factory while retaining the existing transaction-owning wrapper and invariants. <!-- completed: -->
- [ ] Implement naming verification and compensation while the original transaction remains active, inspect commit errors before guard destruction, and report ownership loss with bounded cleanup and invocation deadlines. <!-- completed: -->

### Step 3: Verify failure and compatibility behavior

- [ ] Extend backend and fleet-create unit tests with the specified lossless round trips, failure/cleanup cases and deterministic two-connection ID-reuse interleavings. <!-- completed: -->
- [ ] Verify shared-window snapshots, existing Director placements, returned/stored placement agreement, ordinary-member behavior and first-overflow allocation in the owning test modules. <!-- completed: -->

### Step 4: Align documentation and run required checks

- [ ] Update the affected naming, fleet-create and placement descriptions/examples in `SPEC.md`, `assets/skills/cafleet/reference/cli.md` and `docs/docs/concepts/members.md`; reconcile any other current descriptions found by searching for the old Director-name behavior. <!-- completed: -->
- [ ] Run focused unit tests through `mise //:test FILTER`, then `mise //:test`, `mise //:lint` and `mise //docs:build`; record any observed limitations. <!-- completed: -->
