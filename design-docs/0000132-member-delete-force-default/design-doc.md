# Make immediate kill the default for `cafleet member delete`

**Status**: Approved
**Progress**: 23/23 tasks complete
**Last Updated**: 2026-07-12

## Overview

`cafleet member delete` today sends the backend exit keystroke and blocks up to 15 s for the pane to close (exit 2 + pane tail on timeout), with `--force`/`-f` as the immediate kill-pane escape hatch. This change makes the immediate kill-pane the sole behavior — an ordinary `cafleet member delete` kills the pane at once and exits 0 on the pane and registry paths (the root-Director guard and a `kill_pane` failure still exit 1) — removing the graceful wait, the exit-2 timeout path, and the `--force` flag entirely (a hard break per `removal.md`). The motivation is real teardown friction: the graceful wait/timeout stalls fleet shutdown.

## Success Criteria

- [x] `cafleet member delete --fleet-id <s> --member-id <m>` on a live pane kills the pane immediately (no exit keystroke, no wait) and exits 0, even if the pane was already gone.
- [x] The 15 s graceful wait, the exit-2 timeout path, and the pane-tail-on-stderr diagnostic no longer exist on any `member delete` path.
- [x] `--force` / `-f` is removed; `cafleet member delete --force` (or `-f`) fails with Click's "no such option" (exit 2), pinned by a regression test.
- [x] `wait_for_pane_gone` (base/tmux/herdr) and `poll_until_pane_gone` (base) are deleted along with their tests; no dead graceful-teardown code remains. `pane_exists` is intentionally retained (see Non-goals).
- [x] Success header collapses to `Member deleted.` with pane status `<pane_id> (killed)`; JSON `{member_id, pane_status}` is unchanged; the root-Director guard (exit 1), placementless/pending registry soft-delete (exit 0), and `kill_pane` multiplexer-error (exit 1) contracts are unchanged.
- [x] Every affected surface is updated in the same cycle with no historical "was `--force`" / "15 s wait" residue: `SPEC.md`, `docs/spec/cli-options.md`, `docs/concepts/member-lifecycle.md`, `skills/cafleet/reference/{director,recovery,supervision}.md`, and every role/workflow teardown mention.
- [x] `mise //cafleet:lint` / `:typecheck` / `:test` pass.

---

## Background

### Current behavior (`cli/member.py` `member_delete`)

The command has three outcomes today, selected by placement and the `--force` flag:

| Path | Sequence | Exit | Output |
|---|---|---|---|
| Default, live pane | `send_exit` → `wait_for_pane_gone(timeout=15.0, interval=0.5)` → deregister | 0 (gone) / **2 (timeout)** | `Member deleted.` `(closed)` / on timeout: pane tail on stderr + recovery hint, **no deregister**, `(timeout)` |
| `--force`, live pane | `kill_pane(ignore_missing=True)` → deregister | 0 | `Member deleted (--force).` `(killed)` |
| Placementless / pending placement | registry soft-delete only | 0 | `Member deleted.` `(no placement)` / `(pending — no pane)` |

Guards (unchanged by this design): the root-Director guard rejects deleting a fleet's Director before any pane mutation (exit 1); a `MultiplexerError` from the pane op surfaces as exit 1 with a "server may be unreachable" hint.

### The tension with design 0000123

Design [0000123-herdr-member-delete-graceful-teardown](../0000123-herdr-member-delete-graceful-teardown/design-doc.md) built the graceful default so it would succeed on the herdr backend (issue #172): `HerdrMultiplexer.wait_for_pane_gone` waits for the coding agent to exit to the bare shell, then reaps the pane. This design **reverses that default** — the graceful path is removed, so 0000123's herdr override and the whole `wait_for_pane_gone` family go with it. Reversing 0000123 is intended and acceptable (user decision). The teardown-friction motivation is exactly the graceful wait 0000123 made reliable: even when it works, the per-member wait stalls fleet shutdown, and the user wants teardown to be immediate.

### `--force`'s only distinct behavior becomes the default

Once immediate kill is the default, `--force` selects nothing — `cafleet member delete` already does what `--force` did. Per `removal.md`, the flag is removed as a hard break rather than kept as a no-op, and every teardown doc/role that referenced `--force` as a fallback drops it.

---

## Specification

### 1. CLI: `cli/member.py` `member_delete`

Reduce the command to a single pane path — immediate kill — plus the unchanged registry-only paths and guards.

**Signature**: remove the `--force` / `-f` option and the `force` parameter. The docstring changes from "close its tmux pane" to "kill its tmux pane".

**Body** (after the unchanged root-Director guard, `_load_authorized_member`, and `pane_id` resolution):

```python
if pane_id is None:
    # Pure registry soft-delete — no multiplexer requirement.
    _deregister_or_die(member_id)
    pane_status = "(no placement)" if placement is None else "(pending — no pane)"
    _emit_member_delete_output(
        json_output, member_id, pane_status, header="Member deleted."
    )
    return

mux = ensure_multiplexer_or_die()
try:
    mux.kill_pane(target_pane_id=pane_id, ignore_missing=True)
except MultiplexerError as exc:
    raise click.ClickException(
        f"kill_pane failed for pane {pane_id}: {exc}. "
        f"The {mux.name} server may be unreachable. Verify with "
        f"'cafleet doctor', then re-run the command."
    ) from exc
_deregister_or_die(member_id)
pane_status = f"{pane_id} (killed)"
_emit_member_delete_output(
    json_output, member_id, pane_status, header="Member deleted."
)
```

**Deleted from the command**: the entire graceful branch — the `send_exit` call and its `MultiplexerError` handler (the delete path's copy; `send_exit` itself survives, see §3), the `wait_for_pane_gone` call and its `MultiplexerError` handler, the `capture_pane` tail, the stderr error/recovery block, `pane_status = f"{pane_id} (timeout)"`, and `ctx.exit(2)`.

**Header change**: the `--force` success header `Member deleted (--force).` is gone; the single pane path emits the plain `Member deleted.` header with `(killed)` status. `_emit_member_delete_output` and `_deregister_or_die` are unchanged.

### 2. Observable contract after the change

| Case | Sequence | Exit | Output |
|---|---|---|---|
| Live pane | `kill_pane(ignore_missing=True)` → deregister | 0 | `Member deleted.`, pane status `<pane_id> (killed)`; stdout only |
| Pane already gone | `kill_pane(ignore_missing=True)` swallows the miss → deregister | 0 | same as above (`(killed)`) |
| Placementless | registry soft-delete | 0 | `Member deleted.`, `(no placement)` |
| Pending placement (no pane) | registry soft-delete | 0 | `Member deleted.`, `(pending — no pane)` |
| Root Director target | reject before any pane mutation | 1 | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `kill_pane` `MultiplexerError` | — | 1 | `kill_pane failed for pane <p>: <e>. The <backend> server may be unreachable. Verify with 'cafleet doctor', then re-run the command.` |
| `--force` / `-f` supplied | Click rejects the unknown option | 2 | Click's `no such option: --force` usage error |

JSON mode is `{member_id, pane_status}` for every success (`pane_status` `(killed)` / `(no placement)` / `(pending — no pane)`), unchanged in shape. There is **no** exit-2 application path anymore; exit 2 arises only from Click usage errors (e.g. the removed flag).

### 3. Multiplexer dead-code removal

Removing the `wait_for_pane_gone` caller in §1 leaves the whole family unused in production. Delete it (full removal, `removal.md`):

| File | Remove |
|---|---|
| `cafleet/src/cafleet/multiplexer/base.py` | the abstract `wait_for_pane_gone` method (§docstring + `...`) **and** the `poll_until_pane_gone` module-level helper |
| `cafleet/src/cafleet/multiplexer/tmux.py` | the `wait_for_pane_gone` method **and** the `poll_until_pane_gone` import (line 10) |
| `cafleet/src/cafleet/multiplexer/herdr.py` | the `wait_for_pane_gone` method (the 0000123 graceful override) |

Clean up imports orphaned by the deletions, verified by grep after the edit: `poll_until_pane_gone` in `tmux.py`; `Callable` / `time` in `base.py` (drop each only if it has no other use); `time` in `herdr.py` (drop only if no other use). **`send_exit`, `kill_pane`, `capture_pane`, and `list_pane_ids` all survive** — they have other callers (`send_exit` in the `member create` placement-rollback path; `capture_pane` in `member capture`; `kill_pane` is the delete op itself). `AgentStateAware` / `agent_status` on herdr also survive (used by the monitor loop and now only internally).

### 4. Non-goals

- **`pane_exists` is retained.** After `wait_for_pane_gone` is removed, `pane_exists` (concrete on `TmuxMultiplexer` and `HerdrMultiplexer`, not an abstract base method) loses its only production caller, but it is a generic "does this pane exist?" primitive with its own dedicated test coverage in both backends, and it was **not** in the user's enumerated removal set. Keep it and its tests untouched.
- **No bulk / all-members delete.** `member delete` still targets exactly one `--member-id`; only the default teardown mechanic changes (user decision — out of scope).
- **No new opt-in graceful flag.** Graceful teardown is removed outright; there is no `--graceful` / `--wait` replacement (user decision). Members that need a clean shutdown use the existing "Director messages you to wrap up first" path in their role Shutdown section before the kill.

### 5. `permissions.allow` — no change needed

The coverage set is one wildcard pattern per subcommand — `Bash(cafleet member delete --fleet-id *)` — whose trailing `*` already matched `--force` and now simply matches the flagless delete. There was never a dedicated `--force` allow pattern, so `docs/spec/cli-options.md` § `permissions.allow` coverage needs no edit. (Recorded here to close the Q5 "drop the `--force` pattern" item: there is nothing to drop.)

### 6. Documentation surfaces (docs-first, per `documentation-maintenance.md`)

Two classes of edit. Class A purges explicit `--force` teardown fallbacks; Class B corrects the now-inaccurate "sends the backend exit keystroke, waits up to 15 s / exit-2 timeout" behavior description. Both must land in this cycle; leave no `removal.md` residue.

**Behavior & contract docs**

| File | Edit |
|---|---|
| `SPEC.md` §6.3 `member delete` (the numbered steps) | Collapse steps 5 (`--force`) and 6 (default path) into one "has pane → ensure multiplexer, `kill_pane` tolerating a missing pane (error → the `kill_pane failed …` application error), deregister, header `Member deleted.`, pane status `<pane_id> (killed)`, exit 0". Drop the `--force`/`-f` option from the options line and the entire timeout/exit-2 sub-bullet. |
| `SPEC.md` §6.5 (multiplexer methods) | Delete **all four** `wait_for_pane_gone` mentions, since the method is fully removed from base/tmux/herdr: (1) the tmux primary method bullet `**wait_for_pane_gone(*, target_pane_id, timeout=15.0, interval=0.5) -> bool**` (~L1781–1784); (2) the "Fail-fast (surface failures)" split-list entry `wait_for_pane_gone` (~L1791); (3) the herdr graceful-teardown bullet (~L1909–1918); (4) the trailing "tmux's `wait_for_pane_gone` is unchanged (still `poll_until_pane_gone` over `list-panes`)" clause inside that herdr bullet (~L1919–1920). No `wait_for_pane_gone` / `poll_until_pane_gone` mention survives in §6.5. |
| `SPEC.md` §7.2 (error/exit model) | Remove the "one deliberate exception to the two-tier mapping is the `member delete` pane-teardown timeout (exit 2)" clause in **both** places it appears (the root-Director-guard paragraph and the decisions list) — the two-tier usage→2 / application→1 model now has no exception. |
| `SPEC.md` CLI checklist (`member delete` line) | Drop `--force`/`-f`; note the pane path always exits 0. |
| `docs/spec/cli-options.md` §`member delete` | Remove the `--force` / `-f` table row; rewrite the prose so the pane path "kills the pane immediately (tolerating an already-gone pane), then soft-deletes; exit 0", drop the "poll every 500 ms, 15.0 s timeout … exits 2 … re-run, or escalate with `--force`" sentence; `pane_status` `<pane_id> (killed)`. |
| `docs/spec/cli-options.md` § Error Messages table | Remove the `member delete` default-path 15.0 s timeout (exit 2) row; keep the root-Director exit-1 row. |
| `docs/concepts/member-lifecycle.md` | Mermaid: drop the `Active --> Exiting --> Gone` graceful states; the single pane path is `Active --> Killed: cafleet member delete` / `Killed --> [*]: kill pane + deregister`. Delete-ordering prose: rewrite to immediate kill and remove the herdr "pane outlives its agent / waits for the agent to exit" paragraph. Commands section: `member delete (with --force for an atomic kill+deregister)` → `member delete`. |

**`cafleet` skill reference**

| File | Edit |
|---|---|
| `skills/cafleet/reference/director.md` § Member Delete | Rewrite the prose to "kills the pane immediately, then deregisters and rebalances the layout (exit 0 even if the pane was already gone)"; drop the `--force`/`-f` sentence and the `--force` example command line. |
| `skills/cafleet/reference/recovery.md` | "Pane crashed" and "Truly wedged" table rows and the doctor-fallback item: `cafleet member delete --force` → `cafleet member delete`. Delete the "default `member delete` … waits … on timeout exits 2" paragraph and its 4-item wedged-exit decision tree (moot once delete never waits). Shutdown Protocol: step 1 monitor force-delete `--member-id <monitor> --force` → `--member-id <monitor>`; step 2 "blocks until the target pane is actually gone (15 s default timeout). On timeout follow the wedged-exit decision tree above." → "kills the pane immediately." |
| `skills/cafleet/reference/supervision.md` | Unblock ladder: `→ cafleet member delete --force (last resort …) →` → `→ cafleet member delete (last resort, kills the pane immediately, never raw tmux kill-pane) →`. |

**Role / workflow teardown mentions** (Class A `--force` + Class B "15 s" boilerplate). Apply the canonical rewrites below to each file:

- Class A workflow teardown bodies — drop the "(each call blocks 15 s; on the 15 s timeout (exit 2) use `member capture` … or `--force`)" parenthetical, leaving "`cafleet member delete` the monitoring member first, then <roles> (each kills the pane immediately)":
  `skills/cafleet-design-doc/create/create.md`, `skills/cafleet-design-doc/interview/interview.md`, `skills/cafleet-design-doc/execute/execute.md`, `skills/cafleet-research/report/report.md`, `skills/cafleet-research/report/roles/director.md`.
- Class A escape-hatch note — `skills/cafleet-research/presentation/presentation.md`: rewrite the per-batch note so delete "closes the pane immediately"; remove the "blocks ≈15 s per batch … `--force` is an escape hatch, not the default" sentence.
- Class B member Shutdown boilerplate — "(sends the backend exit keystroke, waits up to 15 s). When the exit keystroke arrives your `claude` process exits immediately — nothing is required of you." → "which kills your pane immediately. Your `claude` process is terminated — nothing is required of you." (preserve the trailing "If the Director instead messages you to wrap up first, …" clause where present — it is now the only clean-shutdown path):
  `skills/cafleet-design-doc/create/roles/drafter.md`, `.../create/roles/reviewer.md`, `.../interview/roles/analyzer.md`, `.../execute/roles/programmer.md`, `.../execute/roles/tester.md`, `.../execute/roles/verifier.md`, `.../execute/roles/reviewer.md`.
- Class B research/presentation variants — "which sends the backend exit keystroke … waits up to 15 s. …" → "which kills your pane immediately. …":
  `skills/cafleet-research/report/roles/researcher.md`, `.../report/roles/scout.md`, `.../report/roles/manager.md`, `.../presentation/roles/presentation.md`, `.../presentation/roles/transcript.md`, and `.../presentation/roles/visual-reviewer.md` (rewrite its "sends the backend exit keystroke and waits up to 15 s … No additional commands run after the exit keystroke arrives" to "kills your pane immediately … No graceful exit runs", keeping the handshake-first point).

> The design-doc drafter/reviewer role files under `skills/cafleet-design-doc/create/roles/` are edited as part of this sweep; the identical global copies under `~/.claude/skills/` are outside the repo and out of scope.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation — behavior & contract (docs-first)

- [x] `SPEC.md` §6.3 `member delete`: collapse the `--force` + default paths into the single immediate-kill path; drop the `--force`/`-f` option and the timeout/exit-2 sub-bullet (Specification §6). <!-- completed: 2026-07-12T13:55 -->
- [x] `SPEC.md` §6.5: delete all four `wait_for_pane_gone` mentions — the tmux primary method bullet, the Fail-fast split-list entry, the herdr graceful-teardown bullet, and the "tmux's `wait_for_pane_gone` is unchanged" trailing clause — leaving no `wait_for_pane_gone` / `poll_until_pane_gone` residue in §6.5. <!-- completed: 2026-07-12T13:55 -->
- [x] `SPEC.md` §7.2: remove the "one deliberate exception … `member delete` teardown timeout (exit 2)" clause in both occurrences; update the CLI checklist `member delete` line (drop `--force`/`-f`). <!-- completed: 2026-07-12T13:55 -->
- [x] `docs/spec/cli-options.md` §`member delete`: drop the `--force`/`-f` row, rewrite the prose to immediate kill + exit 0 + `(killed)`, and remove the timeout (exit 2) row from the Error Messages table. <!-- completed: 2026-07-12T13:55 -->
- [x] `docs/concepts/member-lifecycle.md`: update the mermaid states (single `Active --> Killed` pane path), rewrite the Delete-ordering prose to immediate kill (remove the herdr pane-outlives-agent paragraph), and simplify the Commands bullet. <!-- completed: 2026-07-12T13:55 -->

### Step 2: Documentation — `cafleet` skill reference & teardown sweep

- [x] `skills/cafleet/reference/director.md` § Member Delete: rewrite to immediate kill; drop the `--force` sentence and example. <!-- completed: 2026-07-12T14:08 -->
- [x] `skills/cafleet/reference/recovery.md`: purge `--force` from the crashed/wedged rows and doctor-fallback item; delete the graceful-wait paragraph + wedged-exit decision tree; rewrite Shutdown Protocol steps 1–2 to immediate kill. <!-- completed: 2026-07-12T14:08 -->
- [x] `skills/cafleet/reference/supervision.md`: rewrite the unblock-ladder `member delete --force` entry to the flagless immediate-kill form. <!-- completed: 2026-07-12T14:08 -->
- [x] Class A workflow teardown bodies (create.md, interview.md, execute.md, report.md, report/roles/director.md): drop the 15 s / exit-2 / `--force` recovery parenthetical. <!-- completed: 2026-07-12T14:08 -->
- [x] Class A presentation.md per-batch note: rewrite to immediate close; remove the "blocks ≈15 s … `--force` is an escape hatch" sentence. <!-- completed: 2026-07-12T14:08 -->
- [x] Class B member Shutdown boilerplate across the 7 cafleet-design-doc role files: apply the "kills your pane immediately" rewrite, preserving the wrap-up-first clause. <!-- completed: 2026-07-12T14:08 -->
- [x] Class B research/presentation role variants (researcher, scout, manager, presentation, transcript, visual-reviewer): apply the "kills your pane immediately" rewrite. <!-- completed: 2026-07-12T14:08 -->

### Step 3: Code — CLI

- [x] `cli/member.py` `member_delete`: remove the `--force`/`-f` option + `force` param; delete the graceful branch (send_exit/wait/capture/exit-2); implement the single immediate-kill pane path with the `Member deleted.` / `(killed)` output (Specification §1). Update the docstring to "kill its tmux pane". <!-- completed: 2026-07-12T14:20 -->

### Step 4: Code — multiplexer removal

- [x] `multiplexer/base.py`: delete the abstract `wait_for_pane_gone` method and the `poll_until_pane_gone` helper; drop `Callable` / `time` imports if orphaned. <!-- completed: 2026-07-12T14:20 -->
- [x] `multiplexer/tmux.py`: delete `wait_for_pane_gone` and the `poll_until_pane_gone` import. <!-- completed: 2026-07-12T14:20 -->
- [x] `multiplexer/herdr.py`: delete the `wait_for_pane_gone` override; drop the `time` import if orphaned. Keep `pane_exists` / `agent_status`. <!-- completed: 2026-07-12T14:20 -->

### Step 5: Tests

- [x] `tests/cli/test_member_delete.py`: rewrite for the immediate-kill default. **Delete**: the three timeout tests (`test_timeout__timeout_exits_two_with_tail_and_recovery_hint`, `test_timeout__timeout_json_output_pane_status`, `test_timeout__capture_failure_still_exits_two`); the `send_exit` / `wait_for_pane_gone` error tests (`test_tmux_error_on_send_exit__…`, `test_tmux_error_on_wait_for_pane_gone__exits_one_with_backend_name`); and the three graceful default-path success tests, which today assert the removed `send_exit → wait_for_pane_gone → deregister` flow with `(closed)` (`test_happy_path__call_ordering_send_exit_then_wait_then_deregister`, `test_happy_path__json_output_returns_member_id_and_pane_status`, `test_pane_already_gone__pane_already_gone_first_poll_yields_happy_path`) — these are superseded by the repurposed `--force` tests, so delete them rather than leave two happy-path families. **Repurpose** the `--force` tests (`test_force__force_kills_pane_then_deregisters`, `test_force__force_json_output_pane_status_killed`; drop `test_force__force_short_flag_works`) into the new default-path tests: no flag, `kill_pane → deregister → exit 0`, header `Member deleted.` (no `(--force)` suffix), status `<pane> (killed)`, JSON `(killed)`; plus a pane-already-gone case (`kill_pane(ignore_missing=True)` → exit 0, `(killed)`). **Both** pending-placement tests: `test_pending_placement_force__force_with_pending_placement_skips_all_tmux` (drop the `--force` invocation; keep the skips-all-tmux assertion) and `test_pending_placement__pending_pane_id_skips_send_exit` (rename off `send_exit` — the delete path no longer calls it — to assert it skips `kill_pane`/all tmux). Update the parametrized root-Director test to drop the `--force` case (only the default path remains). Add a `kill_pane` `MultiplexerError` → exit-1 test ("server may be unreachable" wording). **Remove the now-orphaned fixtures**: the autouse `_stub_tmux_entrypoints` `wait_for_pane_gone` patch and the `wait_for_pane_gone_recorder` fixture (both use `raising=False`, so a green run will not flag them). **Keep**: the auth-boundary, placementless, tmux-relaxation, and root-guard tests. <!-- completed: 2026-07-12T13:59 -->
- [x] `tests/cli/test_member_delete.py`: add a regression test pinning `--force` **and** `-f` removal — `member delete --force` / `-f` exits 2 with Click's "no such option" (asserted via Click's default unknown-option error, per `removal.md` "testing the absence"). <!-- completed: 2026-07-12T13:59 -->
- [x] `tests/multiplexer/test_tmux.py`: remove the `wait_for_pane_gone` / `poll_until_pane_gone` tests; keep the `pane_exists` test. <!-- completed: 2026-07-12T13:59 -->
- [x] `tests/multiplexer/test_herdr.py`: remove the `wait_for_pane_gone` graceful-teardown tests (the 0000123 coverage); keep the `pane_exists` tests. <!-- completed: 2026-07-12T13:59 -->

### Step 6: Verification

- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` pass. <!-- completed: 2026-07-12T14:22 -->
- [ ] Residue grep is clean. Run the `wait_for_pane_gone` / `poll_until_pane_gone` method-name grep across **`cafleet/src`, `cafleet/tests`, `SPEC.md`, `docs/`, and `skills/`** (the SPEC/docs/skills scope catches §6.5 residue such as the tmux bullet's `timeout=15.0` — note "15.0" ≠ "15.0s" — or the "tmux's `wait_for_pane_gone` is unchanged" clause). Separately, no `--force` / "15 s" / "15.0s" `member delete` teardown residue under `docs/`, `skills/`, `SPEC.md`. This design doc and `design-docs/0000123-*` are the only permitted historical mentions of either. <!-- completed: 2026-07-12T14:22 -->
- [ ] Live smoke: `cafleet member create` a member, then `cafleet member delete` (no flag) → pane killed immediately, `Member deleted.` / `(killed)`, exit 0; and `cafleet member delete --force` → exit 2 "no such option". <!-- completed: 2026-07-12T14:22 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-12 | Initial draft |
| 2026-07-12 | Round-1 revision: scoped the Overview exit-0 claim; enumerated all four SPEC §6.5 `wait_for_pane_gone` mentions; named the three deleted graceful success tests, both pending-placement tests, and the orphaned test fixtures; extended the method-name residue grep to SPEC/docs/skills. |
| 2026-07-12 | Finalized: Status → Approved after Reviewer approval and user sign-off. |
