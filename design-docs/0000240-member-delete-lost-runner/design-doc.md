# Member deletion of a lost runner

**Status**: Approved
**Progress**: 8/17 tasks complete
**Last Updated**: 2026-11-22

## Overview

`cafleet member delete` and `cafleet fleet delete` cannot end a member whose runner exited while its run is still recorded as non-terminal: every attempt fails with `Error: member runtime: ECONNREFUSED: Connection refused` (issue #636). Deletion now requests runner shutdown only when the run's recorded owner process is alive, and otherwise goes straight to recovery, which records the run's loss so deletion completes.

## Success Criteria

- [ ] Deleting a member whose run is non-terminal and whose recorded owner process is not alive sends no shutdown request, records `runner_lost` through recovery, closes the pane and deregisters the member.
- [ ] Deleting a member whose recorded owner process is alive still requests shutdown first; a refused control socket from a live owner keeps today's `Error: member runtime: ECONNREFUSED: Connection refused` and leaves the member registered.
- [ ] `fleet delete` follows the same rule for every member it ends.
- [ ] One table-driven unit test in `src/cli/fleet.rs` covers both cases through `delete_fleet_with_host`; `mise //:test` and `mise //:lint` pass.
- [ ] `SPEC.md`, `docs/docs/concepts/members.md` and `assets/skills/cafleet/reference/cli.md` describe the conditional shutdown; `mise //docs:build` passes.

---

## Background

`end_member` (`src/cli/fleet.rs`) is the per-member cleanup shared by `member delete` and `fleet delete`. Under the pane lock and fleet layout lock it calls `host.shutdown` for every run whose status is not terminal, then `host.recover`, then closes the pane and deregisters:

```rust
if let Some(runtime) = crate::broker::runtime::member_runtime(conn, member_id)?
    && !runtime.run.status.terminal()
{
    let outcome = host.shutdown(conn, &runtime.run, deadline)?;
    ...
}
```

`request_shutdown` (`src/runtime/control.rs`) connects to the runner's control socket; `ControlClient::connect` retries `ECONNREFUSED`/`ENOENT` only in the `Starting` endpoint phase and otherwise returns the error, so the `?` aborts deletion before recovery runs. A runner that exited without committing a terminal status (for example, after its cleanup write failed with `cleanup evidence pending`) leaves a `running` run, a socket that refuses every connection, and a member that no command can remove.

Recovery (`src/runtime/recovery.rs` `recover`) already settles exactly this state: when the owner `RunLock` is acquirable, the owner's process evidence is `Terminated`, the owner is not the creator and health is unavailable, it takes over the run, settles its deferred inputs (`settle_lost_deferred`), and `reconcile_retained_runtime` commits `runner_lost` (built by `loss_transitions` and persisted in `perform` when no terminal event exists) and finishes as `lost`. Otherwise it returns `live_or_unreachable`, which `end_member` refuses with `member recovery remains incomplete; inspect with cafleet doctor`. The existing deletion test `runner_recovery_deletion_of_an_exited_runner_with_incomplete_cleanup_recovers_without_contacting_it` reaches recovery only because it marks the run `exited` first.

---

## Specification

### Shutdown gate

`end_member` requests runner shutdown only when the run is non-terminal **and** its recorded owner process is alive:

```rust
if let Some(runtime) = crate::broker::runtime::member_runtime(conn, member_id)?
    && !runtime.run.status.terminal()
    && runtime.run.owner_process().is_alive()?
{
    let outcome = host.shutdown(conn, &runtime.run, deadline)?;
    ...
}
```

| Decision | Rule | Rationale |
|---|---|---|
| Liveness source | `RunRecord::owner_process()` (recorded `owner_pid` + `owner_start_identity`) checked with `ProcessIdentity::is_alive()` | The same rule deletion already applies in `await_runner_exit`; an unreaped zombie counts as not alive (SPEC §6.9). |
| Who decides loss | Recovery | The gate only skips a request that cannot succeed; recovery's lock, process-evidence and health checks make the final liveness decision and refuse with `live_or_unreachable` when any of them shows life. |
| Gate placement | Inside the existing `with_database_budget` block, after revalidation, evaluated only for a non-terminal run (the `let` chain short-circuits) | Keeps the check under the held pane/layout locks against the revalidated run. |
| Liveness read error | Propagated with `?` as the deletion error | Fail fast: an unreadable process table is not evidence of death. |
| `DeletionHost` trait | Unchanged; `end_member` calls `is_alive` directly | Tests control liveness through the run's recorded owner identity, as `await_runner_exit` tests already do. |
| Hermes runs | Unchanged order: settlement, `await_runner_exit`, then the gated shutdown | `await_runner_exit` returns at once for a non-terminal run (it waits only for a terminal, unsettled run with a live owner), so the gate applies to a non-terminal Hermes run exactly as to any other backend. |

### Outcomes by run state

Rows for a non-terminal run whose recorded owner is not alive are keyed on `owner_kind`, because recovery distinguishes owners, not statuses.

| Recorded run | Owner process | Deletion behavior |
|---|---|---|
| Terminal (`exited`, `failed`, `lost`) | Any | No shutdown request; recovery as today. |
| Non-terminal, any `owner_kind` | Alive | Shutdown request, then recovery, as today. A live owner whose socket refuses keeps `Error: member runtime: ECONNREFUSED: Connection refused`, member kept. |
| Non-terminal, `owner_kind=runner` (`prepared`, `running` or `degraded`) | Not alive | No shutdown request; recovery takes over, commits `runner_lost`, finishes `lost`; pane closed and member deregistered. This is issue #636. |
| Non-terminal, `owner_kind=reconciler` (`recovering`, a prior recovery pass whose reconciler died) | Not alive | No shutdown request; recovery takes a fresh epoch and finishes the interrupted pass; deletion completes. |
| Non-terminal, `owner_kind=creator` (a crashed `member create`) | Not alive | No shutdown request; recovery returns `live_or_unreachable` for a creator-owned run, so deletion fails with `member recovery remains incomplete; inspect with cafleet doctor` instead of the socket error. Settling this state is out of scope. |

**Check-then-exit race.** An owner alive at the gate that exits before the shutdown connect fails deletion with the shutdown's connection error (`member runtime: ECONNREFUSED: …` or `ENOENT`), leaving the member registered; a rerun sees the dead owner and completes through recovery. No post-error fallback is added.

### Contract changes

No error string, exit code, JSON field or output layout changes. The only contract change is that the shutdown request becomes conditional on the recorded owner's liveness.

| Target | Edit |
|---|---|
| `SPEC.md` §6.3 `member delete` (first paragraph) | Request owned runner shutdown only while the run is non-terminal and its recorded owner process is alive (§6.9 liveness rule); a non-terminal run whose recorded owner is no longer alive goes directly to recovery, which records its loss. State the check-then-exit race and that a rerun completes. |
| `SPEC.md` §6.3 `fleet delete` step 2 | "runner shutdown and recovery" becomes "runner shutdown when its recorded owner is alive, and recovery", keeping the `member delete` pointer. |
| `SPEC.md` §6.9 deletion-recovery paragraph ("Member and fleet deletion run recovery …") | Recovery runs after successful runner shutdown, or without one when the recorded owner process is no longer alive, including a `recovering` run whose reconciler died. |
| `SPEC.md` §6.9 liveness paragraph ("Recovery and deletion count a recorded process as alive …") | Add deletion's shutdown gate to the list of checks that treat an unreaped zombie as terminated. |
| `SPEC.md` §6.9 Terminal transport and lifecycle table, "Member/fleet deletion" row | "request runner shutdown" becomes "request runner shutdown from a live recorded owner". |
| `docs/docs/concepts/members.md` § Teardown (the paragraph opening "Runner-managed deletion coordinates shutdown and event recovery…") | One sentence after its first sentence: a runner that already exited without recording it is not contacted; deletion settles its runtime as lost and completes, and a runner that exits mid-deletion is completed by a rerun. |
| `assets/skills/cafleet/reference/cli.md` § `member delete` | First sentence: shuts down the member's live runner (a runner already gone is settled as lost without a shutdown request); add that `member runtime: ECONNREFUSED` from a runner that exited during deletion is resolved by rerunning the delete. No new outcome-table row. |
| `README.md`, `.claude/rules/` | No change. |

### Test

One table-driven test in `src/cli/fleet.rs` `mod tests` > `mod recovery`, beside `runner_recovery_deletion_of_an_exited_runner_with_incomplete_cleanup_recovers_without_contacting_it`, named `runner_recovery_deletion_of_a_non_terminal_run_requests_shutdown_only_from_a_live_owner`. Each case names itself in every assertion message.

Shared setup per case:

1. `Case::new()` (a `running` run owned by the current test process as runner).
2. Apply the case's owner identity with `UPDATE member_runs SET owner_pid=?1, owner_start_identity=?2 WHERE run_id=?3`, then `case.refresh()` and `case.release()`.
3. `DeletionTestHost::recovering(&case, deadline)` with a 2-second deadline and `host.runner_gone = true` (the test host's `shutdown` returns `member runtime: ECONNREFUSED: Connection refused`); the recovery host keeps its defaults (`Evidence::Terminated`, unavailable health).
4. Run `delete_fleet_with_host` for the case's fleet.

| Case | Owner identity | Expected |
|---|---|---|
| `gone_runner` | A reaped child: spawn `sleep 10`, read `ProcessIdentity::read(pid)`, kill and `wait` it | `Ok`; `host.calls == ["recover", "close"]`; `host.recovered` status `RunStatus::Lost`; exactly one `runner_lost` event among `events(&case)`; the member row's status is `deregistered` |
| `live_owner_refuses` | Unchanged (current test process) | `Err` whose message contains `ECONNREFUSED`; `host.calls == ["shutdown"]`; `assert_registry_active(&case.fixture)`; the run's status is still `running` |

### Existing test adjustment

Deletion tests built on `Case::new` or `runtime::test_support::Fixture::new` record the current process as owner, so the gate leaves their `shutdown` expectations unchanged. `runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity` is the exception: its replacement run comes from `crate::broker::runtime::test_support::Fixture::spec`, whose `owner_start_identity` is `"fixture-start-identity"`, so `is_alive()` is false and the gate would skip the shutdown the test asserts (`["shutdown", "close"]`, `host.owners`). Its `NewRun` spec sets `owner_pid` and `owner_start_identity` from `ProcessIdentity::current()`, as the `src/cli/member.rs` deletion tests do, so the test keeps asserting that shutdown.

### Out of scope

- `cafleet doctor` not reporting a member stuck in this state.
- The poison event that triggers it (`event sequence is not the run's next value`), covered by design 0000239.
- Settling a creator-owned `starting`/`prepared` run whose creator died.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update SPEC.md

- [x] Edit §6.3 `member delete` first paragraph per § Contract changes, including the check-then-exit race and rerun <!-- completed: 2026-11-22T00:36 -->
- [x] Edit §6.3 `fleet delete` step 2 wording <!-- completed: 2026-11-22T00:36 -->
- [x] Edit the §6.9 deletion-recovery paragraph to cover recovery without shutdown for a dead owner, including a `recovering` run <!-- completed: 2026-11-22T00:36 -->
- [x] Add deletion's shutdown gate to the §6.9 liveness paragraph <!-- completed: 2026-11-22T00:36 -->
- [x] Edit the "Member/fleet deletion" row of the §6.9 Terminal transport and lifecycle table <!-- completed: 2026-11-22T00:36 -->

### Step 2: Update user docs and skills

- [x] Add the lost-runner deletion sentence to the "Runner-managed deletion coordinates shutdown…" paragraph of `docs/docs/concepts/members.md` § Teardown <!-- completed: 2026-11-22T00:37 -->
- [x] Update `assets/skills/cafleet/reference/cli.md` § `member delete` per § Contract changes <!-- completed: 2026-11-22T00:37 -->
- [x] Run `mise //docs:build` and confirm it passes <!-- completed: 2026-11-22T00:37 -->

### Step 3: Add the failing test

- [ ] Add `runner_recovery_deletion_of_a_non_terminal_run_requests_shutdown_only_from_a_live_owner` with the `gone_runner` and `live_owner_refuses` cases to `src/cli/fleet.rs` `mod tests` > `mod recovery` <!-- completed: -->
- [ ] Run `mise //:test runner_recovery_deletion_of_a_non_terminal_run` and confirm `gone_runner` fails with calls `["shutdown"]` and the ECONNREFUSED error <!-- completed: -->

### Step 4: Gate the shutdown request

- [ ] Add `&& runtime.run.owner_process().is_alive()?` to the shutdown condition in `end_member` <!-- completed: -->
- [ ] Set the replacement run's `owner_pid`/`owner_start_identity` from `ProcessIdentity::current()` in `runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity` per § Existing test adjustment <!-- completed: -->
- [ ] Run the new test and `runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity`, and confirm they pass <!-- completed: -->

### Step 5: Validate

- [ ] Run `mise //:test` and confirm every test passes <!-- completed: -->
- [ ] Run `mise //:lint` and confirm it passes <!-- completed: -->
- [ ] Reinstall the CLI through its `mise tasks --all` install task <!-- completed: -->
- [ ] Commit the design doc with the implementation per the project git-workflow rule <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-22 | Initial draft |
