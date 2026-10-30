# Audit: INP — runtime input subsystem

Scope: `cafleet/src/runtime/runner/{input.rs, input_client.rs, input_clock.rs, input_recovery.rs, input_protocol.rs, input_protocol/wire.rs, input_protocol/wire/replies.rs, input_protocol/wire/strict_json.rs}`, `cafleet/src/input.rs`, and the test-only harness these files depend on (`runtime/runner_tests/control_service/input_service*.rs`, `runner_tests/mod.rs::Fixture`, `runner_tests/fixtures/*`, `runner_tests/fleet_deletion.rs`, `runner_tests/recovery/{deletion,foundations}.rs`).

## Summary

The slice holds about 21.7k lines; about 15k of them are tests, already inline in the implementation files but split into several modules, often placed before the production code. The largest wins are test-scaffolding consolidation:

- the `manual_progress` harness rebuilds `support::Host` (INP-06);
- coordinator call boilerplate and a `#[cfg(test)]` `handle` inside the production impl (INP-08);
- repeated poll loops, budget observers, `Settings` literals and table dumps across the input tests (INP-09, INP-13, INP-14);
- relocating the `runner_tests` input harness into `runner/service.rs` (INP-01).

On the production side:

- collapse the four-file `input_protocol` module into one (INP-02);
- delete the constant `protocol_version` request field and its three never-failing checks (INP-03);
- fold `CurrentRequest` into `ObservedRequest` (INP-04);
- drop client-side reply checks that `replies::decode` already enforces (INP-05);
- single-source the client/runner payload digest and the inherited-deadline clamp (INP-11, INP-12).

`cafleet/src/input.rs` mixes three concerns (SQLite wait budget, pane input lock, member/fleet deletion), has duplicated `DeletionHost::shutdown` impls and repeated error mapping. Estimated total: about 1,300–1,500 lines removable (roughly 350 production, 1,000+ test), excluding pure moves. The shipped contract stays unchanged: every proposal preserves CLI output, error strings that can be reached, the wire bytes and JSON key order.

## Test placement inventory

| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/runtime/runner_tests/control_service/input_service.rs` | 764 | Helpers (`with_service`, `barrier`, `composer`, `records`, `wait_record`, `install_panic_record`, `panic_records`) → `#[cfg(test)] pub(crate)` items in `runner/service.rs`; its five `step5_service_input_*` tests drive the service loop (service.rs:617-773) → `runner/service.rs` tests | Loads `../fixtures/synthetic-profile.json` (line 133); private copy of `RealInputDeadlineClock` (90-112, see INP-27) | Consumers: `delivery.rs` (14 uses), `cli/member.rs` (9), `cli/runtime.rs` (3), `input_client.rs` tests (~45). Re-exported via `runtime/mod.rs:8-16` aliases (INP-29). The re-exec dispatch path (`control_service.rs:24,30-33`) must be rewritten to the new test path. |
| `cafleet/src/runtime/runner_tests/control_service/input_service/native.rs` | 109 | `#[cfg(test)]` helper in `runner/service.rs` (synthetic native child program started via `NATIVE_ENV` re-exec) | Uses `observation_services.rs:68 private_write`, `ProcessIdentity` | — |
| `cafleet/src/runtime/runner_tests/control_service.rs` (`ServiceFixture`, re-exported as `InputServiceFixture`) | 786 | `#[cfg(test)]` helper in `runner/service.rs`, next to the already-cfg(test) `RunnerTestServices` / `run_with_test_services` (mod.rs:46-47) | — | Shared with other slices; only the input-facing portion is listed here. |
| `cafleet/src/runtime/runner_tests/mod.rs` (`Fixture`, `read_fixture_bytes`, `write_fixture_bytes`) | 168 | `#[cfg(test)] pub(crate) mod test_support` in `runner/mod.rs` (owns `SpawnSpec`/`TerminalSize`, which `Fixture::spec` builds) | Uses `crate::broker::tests as common` | Imported by `input.rs:7100,7534`, `input_client.rs:1454,4972`, `input_recovery.rs:396`, `wire.rs:316`, plus ten other runner files. `write_fixture_bytes` duplicates the write-all loop in input_service.rs (INP-27). |
| `cafleet/src/runtime/runner_tests/owned_child.rs` (`OwnedChild`) | 105 | `#[cfg(test)]` helper in `runner/process.rs` (owns `ProcessIdentity`) | — | Used at `input.rs:7100,8350`. |
| `cafleet/src/runtime/runner_tests/control_service/observation_services.rs::synthetic` | 194 (file) | `#[cfg(test)]` helper in `runner/observation.rs` (needs-verification: confirm it is the observation-service owner) | Same JSON fixture (line 96) | Used at `input_client.rs:1797,1943,2308`. |
| `cafleet/src/runtime/runner_tests/fleet_deletion.rs` | 533 | `cafleet/src/input.rs` tests (exercises `delete_fleet_with_host`), or the deletion module proposed in INP-38 | Test `DeletionHost` impl `Host` (61-125); `settings(fixture)` (29) duplicates INP-14 | Tests the deletion orchestration defined in `input.rs:1289-1776`. |
| `cafleet/src/runtime/runner_tests/recovery/deletion.rs` | 642 | `cafleet/src/input.rs` tests (or the INP-38 deletion module): `delete_fleet_with_host` + `DeletionHost::recover` | Third test `DeletionHost` impl (47-168) | Overlaps the runner-tests slice; listed because every test calls `crate::input::delete_fleet_with_host`. |
| `cafleet/src/runtime/runner_tests/recovery/foundations.rs` (`pane_lock_for` tests, ~lines 240-262) | ~25 of 330 | `cafleet/src/input.rs` `pane_lock_wait_tests` | — | The rest of the file belongs to the recovery slice. |
| `cafleet/src/runtime/runner_tests/fixtures/synthetic-profile.json` | 1 (1466 bytes) | `cafleet/src/coding_agent/profiles/` fixture behind one `#[cfg(test)] pub(crate) fn synthetic_profile_definition()` in `profiles/catalog.rs` (which already `include_str!`s it at 21 and 83) | Loaded at `catalog.rs:21,83`, `runner/input.rs:8361`, `writer.rs:851`, `input_service.rs:133`, `observation_services.rs:96`, `native_capture/support.rs:28` | Test-only. Every load site is under `#[cfg(test)]`, and it is not bundled runtime data. Final placement belongs to the coding_agent slice. |
| `cafleet/src/runtime/runner_tests/fixtures/claude-2.1.302-blocked-form-blink-1.raw` | 441 bytes | Inline `const` byte string in the `runner/input.rs` test module | — | Test-only; its single consumer is `runner/input.rs:1045-1046` (`include_bytes!`). |
| `coding_agent/profiles/native/*.json` (loaded at `runner/input.rs:101,103,160-170,1073`) | — | Stay in place | — | Runtime data: production `BUNDLED_FIXTURES` (`catalog.rs:1147-1163`) embeds them. Tests re-including them is fine. |
| Misplaced tests inside slice files: `input_clock.rs:423-457` (3 `step5_b0_clock_prepared_submission_*` tests + `request()`) | 35 | `runner/input_client.rs` tests (owner of `PreparedSubmission`) | `ScriptClock` becomes a `#[cfg(test)] pub(super)` helper in `input_clock.rs` | — |
| Misplaced tests inside slice files: `wire.rs:894-1429` (p1–p10 reply tests) | 540 | Exercise `wire/replies.rs` | — | Moot once INP-02 merges the protocol files. |
| Misplaced tests inside slice files: `input_client.rs:1464-1739` | 275 | `cafleet/src/delivery.rs` tests (see INP-23) | — | — |

## Findings

### INP-01: Dissolve the runner_tests input harness into its owning implementation modules
- Category: test-placement
- Location: `cafleet/src/runtime/runner_tests/control_service/input_service.rs:1-764`, `input_service/native.rs:1-109`, `runner_tests/mod.rs:1-168`, `cafleet/src/runtime/mod.rs:7-18`
- Evidence: Every input test in the slice depends on this harness through `crate::runtime::runner_tests::*` or the `runtime/mod.rs` re-exports. Users are `input_client.rs:1454-1457` and about 45 call sites, `input.rs:7100,7534`, `input_recovery.rs:396`, `wire.rs:316`, `delivery.rs:788-1396`, `cli/member.rs:1412-1603` and `cli/runtime.rs:354-449`.
- Proposal: Move the items as the inventory lists:
  - `ServiceFixture` and the input-service helpers become `#[cfg(test)] pub(crate)` items in `runner/service.rs`.
  - `Fixture` moves to a `#[cfg(test)] mod test_support` in `runner/mod.rs`.
  - `OwnedChild` moves to `runner/process.rs`.
  - The `step5_service_input_*` tests move to `runner/service.rs`.
  - Delete the `runner_tests/control_service/input_service*` files and the `runtime/mod.rs` aliases.
- Spec impact: none. Tests only.
- Risk: medium. The re-exec child test paths (`CHILD_TEST`, `NATIVE_ENV` dispatch) are string-encoded and must be updated together.
- Estimated delta: -20 lines net (module headers, re-exports, aliases); the remaining ~1,000 lines are moved, not deleted.

### INP-02: Collapse the four-file input_protocol module into one file
- Category: over-abstraction
- Location: `runner/input_protocol.rs:1-3`, `input_protocol/wire.rs:6-7,301-311`, `wire/replies.rs:253,290`, `wire/strict_json.rs:87,135`, `runner/mod.rs:1-2`
- Evidence:
  - `input_protocol.rs` declares the types and then `pub(super) use wire::*`; `mod.rs` re-exports everything with `pub(crate) use input_protocol::*`.
  - `decode_input_reply` / `encode_input_reply` (wire.rs:301-311) only forward to `replies::decode` / `replies::encode`.
  - `strict_json.rs` reaches `ensure` through `super::super::super::ensure`, although it imports `error` directly at line 1.
  - `decode_input_request` has no caller outside `wire.rs` (`decode_service_request` and the tests).
  - The reply tests live in `wire.rs` (894-1429) but exercise `replies.rs`.
- Proposal: Merge the request codec, reply codec and the private strict deserializer into `input_protocol.rs`. Give the reply functions their public names and delete the forwarders. Make `decode_input_request` private.
- Spec impact: none. Code motion and visibility only; the encoded bytes are identical.
- Risk: low.
- Estimated delta: -25 lines, 3 modules removed.

### INP-03: Remove the constant-valued `InputControlRequest.protocol_version` field
- Category: dead-code
- Location: `input_protocol.rs:162`; initializers at `input_client.rs:36,517,749,1162,1353`, `control.rs:729`, `input.rs:8755`, `wire.rs:220`; checks at `input.rs:4136,4568,5331`; encoder at `wire.rs:287`
- Evidence: Every construction sets `CONTROL_PROTOCOL_VERSION` or `1`, and the decoder sets the constant itself (wire.rs:220). As a result, the three `ensure(request.protocol_version == CONTROL_PROTOCOL_VERSION, …)` checks can never fail. The field is read only by the encoder and by those checks (grep `protocol_version ==`).
- Proposal: Drop the field. Have the encoder write `CONTROL_PROTOCOL_VERSION`. Delete the three checks, the initializers and the assert at `wire.rs:494`.
- Spec impact: none. The envelope still carries `"protocol_version":1` in the same key position. The removed error strings can never be produced and appear in no docs, SPEC.md or skill.
- Risk: low.
- Estimated delta: -20 lines.

### INP-04: `CurrentRequest` mirrors `ObservedRequest` field by field
- Category: duplication
- Location: `runner/input.rs:3703-3745` (struct and `same_request`), `4193-4210` (construction), `4339-4359` (13-field conjunct in `validate_permission_binding`), `4476-4482`; `runner/observation.rs:375-407`
- Evidence:
  - 11 of the 16 `CurrentRequest` fields are copies of `ObservedRequest` fields.
  - `CurrentRequest::same_request` re-implements `ObservedRequest::same_request` and adds `request_generation` and the identity fields.
  - `validate_permission_binding` lists the same conjunct a third time.
- Proposal:
  - Store `observed: ObservedRequest` (derive `Clone`) plus `identity`, `interaction_id`, `event_id`, `message_id` and `manual_generation`.
  - Delegate to `ObservedRequest::same_request` (made `pub(super)`) and compare `request_generation`.
- Spec impact: none. Same predicates and error texts.
- Risk: medium. These are security-relevant binding checks; verify field-for-field equivalence, including that `output_seq` stays excluded where it is today.
- Estimated delta: -40 lines.

### INP-05: The input client re-checks reply bindings that `replies::decode` already enforces
- Category: duplication (overly defensive duplicate validation)
- Location:
  - `input_client.rs:1173-1182` (response identity)
  - `776-779`, `792-795` (Settled/Retired id; same pattern at `delivery.rs:441-446`)
  - the `submission.input_id == Some(input_id)` conjunct at `761-762`
  - 6 of the 12 inspection identity conjuncts at `1373-1381`, `evidence::validate` at `1388`, token re-validation at `1405-1407`, and the ordered-subset check at `1408-1415`
- Evidence:
  - `NativeInputHost::exchange` always goes through `ControlClient::exchange_input` (`control.rs:118-129`), which runs `decode_input_reply` → `replies::decode` → `validate_binding` (`replies.rs:198-250`).
  - That path already enforces the response event, interaction, member, run, owner_epoch, decision and source, the Retire→Retired and Reconcile→Settled id pairing, Present's `submission.input_id`, and the inspection event, interaction, director, member, fleet, run and owner_epoch.
  - `replies.rs:91-113` also enforces the token format, the allow/deny order and `evidence::validate`.
  - Test `step5_wire_p9_*` (`wire.rs:1257`) rejects each mutated binding field.
- Proposal: Delete the duplicated conjuncts. Keep the checks the wire layer does not perform: message_id, episode_seq, backend, versions, `missing_evidence` equality, the capability rules, the 256-byte profile id limit, and the query's `actual == input_id`. Delete the `InspectionHost` test host (1898-1938) and its `wrong_event` subcase (2045-2073), which mutates a reply after decode only to reach the removed check.
- Spec impact: none. Whenever these checks would fail, the wire error is raised first, so the client's messages are unreachable in production.
- Risk: medium. This removes defense in depth behind the `InputCommandHost` seam. Keep `reply.director_member_id == run.director_member_id` unless `inspection_association` is confirmed to bind it (needs-verification).
- Estimated delta: about -35 production lines, about -70 test lines.

### INP-06: The `manual_progress` test harness duplicates `support::Host`
- Category: redundant-test (oversized scaffolding)
- Location: `runner/input.rs:7116-7377` against `8487-8507` (`Reader`), `8559-8567` (`Child`), `8609-8740` (`build`), `8856-8867` (`finish`), `8912-8924` (`with_owned_host`)
- Evidence:
  - `Harness::new` repeats the `Host::build` sequence: `Fixture::new`, `RunLock::acquire`, `transfer_to_runner`, `reserve_event_sequences`, `EventWriter::start` + `set_deadline`, spawning `/bin/cat` through `OwnedChild`, the same `UPDATE member_runs SET status='running'` SQL, and `controller.started`.
  - `NativeChild` duplicates `Child`, `Harness::finish` duplicates `Host::finish`, and `with_harness` duplicates `with_owned_host`.
  - The differences are parameters only: bundled catalog, 64-byte `TransportLimits`, no coordinator at construction, and a panicking `Output`.
- Proposal: Extract `support::RunnerFixture::new(catalog, profile, limits, until)` with a `finish()` method. `Host` wraps it, and `manual_progress` uses it directly.
- Spec impact: none.
- Risk: medium. Verify the tests that depend on the 64-byte limits.
- Estimated delta: -120 lines.

### INP-07: runner/input.rs test modules are split around the production code and import forward
- Category: test-placement
- Location: `runner/input.rs:31-732`, `733-3640`, `6254-7090`, `7092-8927`
- Evidence: The first two modules import from the last one (`input.rs:33-35,735` use `super::contract_tests::ordinary_prompt::support::{Host, …}`). `support` and its parent are declared `pub(in crate::runtime::runner::input)` (7528, 8348) only to allow those forward imports. `manual_progress` (7095-7106) and `ordinary_prompt` (7529-7539) repeat near-identical `use` blocks.
- Proposal: Put the production code first, then one `#[cfg(test)] mod tests` at the end of the file. Inside it:
  - `mod support` comes first;
  - then `permission_inspection`, `permission_response` (with `gated`), `retirement`, `manual_progress` and `ordinary_prompt` as nested modules;
  - visibility drops to `pub(super)`, and the shared `use` lines appear once.
- Spec impact: none.
- Risk: low. This is a mechanical move.
- Estimated delta: -25 lines.

### INP-08: A `#[cfg(test)] handle` wrapper sits in the production impl, and the coordinator call boilerplate is copied
- Category: test-placement / duplication
- Location: `runner/input.rs:13-14` (cfg(test) import), `5280-5310` (`handle`); `.handle(` at 14 test sites (6285 … 8823); `.handle_received(` at 8 sites (785 … 3105); per-module wrappers `handle` (779-792), `handle_until` (3098-3112), `receipt` (1965-1987, 6396-6411) and `advance` (1954-1963)
- Evidence: Only tests use `handle`. It mirrors the expiration step that `service.rs` performs at 631 and 690.
- Proposal: Move `handle` into `support` as `Host::handle` and `Host::handle_until`. Add `Host::try_advance`, `Host::receipt` and `Host::respond`, then delete the per-module copies and the cfg(test) import.
- Spec impact: none.
- Risk: low.
- Estimated delta: -60 lines.

### INP-09: Repeated in-test fragments in input_client.rs tests
- Category: duplication (test scaffolding)
- Location and proposal, in `input_client.rs` unless noted:

  | Repeated fragment | Locations | Proposal | Delta |
  |---|---|---|---|
  | Interaction-opened poll loop | 1805-1817, 1970-1982, 2331-2343 | `opened_interaction(fixture, until)` helper | -25 |
  | `notified_at` query | 1507-1516, 1708-1716, 1884-1892, 2157-2165 | Reuse `delivery.rs:1199 preview_stamp` | -20 |
  | Authorizer budget observer | 2208-2226, 3657-3676, 3876-3896, 4273-4292; also `delivery.rs:847`, `input_recovery.rs:1376` | `#[cfg(test)] with_observed_input_reads(conn, f)` in the `cafleet/src/input.rs` tests | -60 |
  | Permission-table counts | 1824-1836, 2145-2156 | One helper | -10 |
  | `PermissionRespondInput` built from Case metadata | 2181-2194, 5451-5467 | `Case::command()` | -13 |
  | Reconciler-owner expectation | 2993-2999, 3233-3239 | Share `input_recovery.rs:697-705 assert_owner_only` | -12 |
  | SIGWINCH resize | 1471-1486, 2495-2512 | One helper | -12 |
  | Observation catalog overwrite | 1945-1961, 2315-2322 | One helper | -10 |
  | Case placement re-fetch | 2868-2872, 2940-2945; `input_recovery.rs:726-730,890-895` | Expose `Case.placement` as `pub(in crate::runtime)` | -15 |
  | Ready-state Case builder | `staged::ready()` 6463-6479, `input_recovery.rs:711-719` | Add a `Retained::Ready` variant | -12 |

- Spec impact: none.
- Risk: low.
- Estimated delta: about -190 lines.

### INP-10: `submit_wrapped_input` duplicates its `#[cfg(test)]` host variant, and the under-lock wrapper only forwards
- Category: over-abstraction / dead-code
- Location: `input_client.rs:319-351`, `353-388` (`#[cfg(test)] submit_wrapped_input_with_host`), `430-449` (`submit_wrapped_input_under_pane_lock`)
- Evidence: The cfg(test) function is identical to the production one except for a generic host. `submit_wrapped_input_under_pane_lock` has exactly one caller (line 341); `delivery.rs:17,404` calls the `_with_host` variant directly. The production entry is called from `cli/member.rs:724,1585`.
- Proposal: Make `submit_wrapped_input_with_host` a production function. Reduce `submit_wrapped_input` to one call with `&mut NativeInputHost`, and delete `submit_wrapped_input_under_pane_lock`.
- Spec impact: none. `NativeInputHost` is zero-sized, and both paths pass a fresh instance to the same functions.
- Risk: low.
- Estimated delta: -43 lines.

### INP-11: The ordinary-input payload digest is computed in two production places and three test oracles
- Category: duplication
- Location: production at `input_client.rs:194-201` (`submission_digest`) and `runner/input.rs:5394`; test oracles at `input_client.rs:3309-3318`, `delivery.rs:1208-1218` and `runner/input.rs:7656-7667`
- Evidence: Both production sites compute SHA-256 hex of `canonical_json({run_id, kind, message_id, literal})`. They must agree byte for byte, or "conflicting input operation key reuse" fires. The permission digest already has a single home in `broker/runtime/permissions.rs:142 permission_response_digest`.
- Proposal: Add `pub(crate) fn input_payload_digest(run_id, &SubmitInput)` beside `permission_response_digest` (in `broker/runtime/inputs.rs`) and call it from both sides. Keep one independent test oracle there and delete the other two.
- Spec impact: none. The bytes are unchanged.
- Risk: low.
- Estimated delta: about -30 lines.

### INP-12: The "min with the inherited database deadline" clamp is copied at 12 sites, and two re-clamps are no-ops
- Category: duplication
- Location: `runner/input.rs:4132,4620,4648,4810,4910,5016,5162,5607,5706`; `input_client.rs:262-269`; `input_recovery.rs:141-142`; `delivery.rs:183-185`
- Evidence:
  - Each site computes `inherited_database_deadline(conn).map_or(d, |b| d.min(b.instant()))`. The `map_or` is a legitimate default, because an absent scope is a valid state.
  - At `input.rs:5607-5608` and `5706-5707` the value already comes from `effective_deadline()` (4804-4815), which applies the same clamp.
  - `with_database_budget` pushes and pops its frame symmetrically (`cafleet/src/input.rs:178-185`, `216-225`), so the second clamp is a no-op.
- Proposal: Add `pub(crate) fn bounded_by_inherited(conn: &Connection, until: Instant) -> Instant` to `cafleet/src/input.rs`, use it at every site, and delete the two re-clamps.
- Spec impact: none. The deadline values are identical.
- Risk: low.
- Estimated delta: -26 lines.

### INP-13: Five copies of the "snapshot every table" test helper
- Category: duplication (test scaffolding)
- Location: `input_client.rs:5194-5248` (`Snapshot`/`Table`), `input_client.rs:3602-3618` (`health_records`, a 3-table subset), `input_recovery.rs:664-695` (`rows`), `runner/health.rs:465-499` (`registry_snapshot`), `runner_tests/recovery.rs:389-422` (`rows`)
- Evidence: Each one lists tables with `SELECT name FROM sqlite_schema WHERE type='table'`, runs `SELECT * … ORDER BY rowid`, and collects `rusqlite::types::Value` rows.
- Proposal: Add one `#[cfg(test)] pub(crate) fn table_rows(conn, include: impl Fn(&str) -> bool)` in `cafleet/src/db/mod.rs`. `health_records` becomes a filtered call.
- Spec impact: none.
- Risk: low.
- Estimated delta: about -90 lines repo-wide (about -75 in-slice).

### INP-14: The test `Settings` literal for a database URL is repeated
- Category: duplication (test scaffolding)
- Location:
  - `input_client.rs:1604-1608, 1819-1823, 3379-3383, 3736-3739, 4108-4112, 4523-4526, 5166-5170, 5268-5272, 5986-5990`
  - `cafleet/src/input.rs:840-843`, `broker/mod.rs:6313-6317`, `runner_tests/fleet_deletion.rs:29`
  - `delivery.rs:571-574` (`settings_for`)
  - `cli/mod.rs` (2), `cli/runtime.rs` (1)
- Evidence: Each site builds `Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(...))` or the equivalent struct literal.
- Proposal: Add one `#[cfg(test)] pub(crate) fn Settings::for_database_url(url)` in `config.rs` (owner of `Settings`) and use it everywhere.
- Spec impact: none.
- Risk: low.
- Estimated delta: about -40 lines in-slice.

### INP-15: wire.rs tests copy the operation list five times
- Category: redundant-test (oversized scaffolding)
- Location: `wire.rs:483-491, 591-600, 618-627, 643-652, 827-836`
- Evidence: The same seven-element `[(kind, action)]` array appears five times.
- Proposal: Use one `const OPERATIONS: [(&str, Option<&str>); 7]`. Optionally replace the single-field `Case { fixture }` wrapper with functions that take `&Fixture`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -35 lines.

### INP-16: A custom key-order deserializer duplicates serde_json `preserve_order`
- Category: redundant-test (oversized scaffolding)
- Location: `wire.rs:317, 319, 1323-1357, 1392-1398`
- Evidence: `cafleet/Cargo.toml:22` enables `serde_json` `preserve_order`, so a parsed `Value` keeps key order. `OrderedKeys`, its visitor, `OrderedEnvelope` and `OrderedResult` only list keys.
- Proposal: Read `serde_json::from_slice::<Value>(body)?["result"]["request"].as_object().unwrap().keys()` and delete the scaffolding.
- Spec impact: none. The key-order assertion is kept.
- Risk: low.
- Estimated delta: -32 lines.

### INP-17: Redundant wire tests
- Category: redundant-test
- Location: `wire.rs:577-588` (q4), `1648-1654` (r1 missing-nonce block), `913-981` (p2)
- Evidence:
  - q4's `duplicate(&good,"kind")` and `prefix(&good, r#""kind":"submit_prompt","#)` produce identical bytes. No escaped name is tested, despite the test's name. q5 (590-606) already rejects a duplicate of every field.
  - The r1 missing `launch_nonce` case is already covered by q7 (634-638), which removes every identity field for every operation.
  - p2 asserts 23 fields one by one, while p10 round-trips `inspection(false)`.
- Proposal: Delete q4 and assert `decode_service_request(&raw).is_err()` inside q5's loop. Delete the r1 block. Optionally add `inspection(true)` to p10 and shrink p2 to its typed checks.
- Spec impact: none.
- Risk: low for q4 and r1. Medium for p2, because a round-trip cannot catch decode and encode being wrong in the same way.
- Estimated delta: -18 lines (-63 with the optional p2 change).

### INP-18: strict_json frame-size checks can never trigger
- Category: other (overly defensive duplicate validation)
- Location: `strict_json.rs:87-90, 135-138`
- Evidence: The only production inputs are `AdmittedControlFrame::body()` (`service.rs:617`, `control.rs:126`). `AdmittedControlFrame` is built only in `ControlDecoder::feed_admitted` (`codec.rs:563`), after `length <= MAX_CONTROL_FRAME` has been checked (`codec.rs:550`).
- Proposal: Delete both checks; the framer stays the single owner of the size limit. Optionally type the decode argument as `&AdmittedControlFrame`. Drop the direct oversized-body asserts at `wire.rs:851, 1415`.
- Spec impact: none. "control frame exceeds size limit" is still raised by the framer.
- Risk: low.
- Estimated delta: -10 lines.

### INP-19: `wire::version` duplicates `codec::validate_version`
- Category: duplication
- Location: `wire.rs:37-42` against `codec.rs:450-457`
- Evidence: Both raise the identical string "control frame requires integer protocol_version 1" with the same `as_u64() == Some(1)` test.
- Proposal: Make `codec::validate_version` `pub(super)`, call it at `wire.rs:121` and `replies.rs:257,262`, and delete `version`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -7 lines.

### INP-20: `validate_order` copies enum domains and has a branch that can never fire
- Category: duplication / dead-code
- Location: `replies.rs:94-112, 118-132`
- Evidence: `&["allow","deny"]` copies `PermissionDecision::VALUES` (`records.rs:275`), and the seven-item list copies `MissingEvidence::as_str` (`input_protocol.rs:101-112`). `serde_json::from_value` (`replies.rs:89`) already rejects values outside those domains, so the "invalid control inspection subset" branch (`replies.rs:124`) can never fire.
- Proposal: Use `PermissionDecision::VALUES` and a `MissingEvidence::VALUES` const, or derive `PartialOrd` and require a strictly increasing `windows(2)`. Keep "control inspection subset order mismatch".
- Spec impact: none. The removed string is unreachable and absent from SPEC.md, docs and skills.
- Risk: low.
- Estimated delta: -12 lines.

### INP-21: The reply-against-durable-state check is duplicated in input_client.rs
- Category: duplication
- Location: `input_client.rs:537-572` (submission) and `1200-1240` (permission response)
- Evidence: The `(reply.input_id, durable)` match and the per-status predicates are identical; only four error strings differ. None of those strings is asserted in tests or SPEC.md.
- Proposal: Extract `validate_reply_against_durable(reply, durable, messages: [&str; 4])` and keep each string verbatim.
- Spec impact: none.
- Risk: low.
- Estimated delta: -30 lines.

### INP-22: Hand-written clones of protocol input structs
- Category: duplication
- Location:
  - `input_client.rs:1069-1084` (`copy_response_input`), `505-510`, `1356-1360`
  - the `PermissionInspectInput` rebuild at `1054-1058` and `1131-1135`
  - tests at `input_client.rs:4433-4438` and `runner/input.rs:776-778` (serde round-trip `copy`)
- Evidence: `PermissionRespondInput` and `PermissionInspectInput` derive only `Serialize, Deserialize`, and `SubmitInput` has no derives at all (`input_protocol.rs:46-77, 141-146`). Every field is `Clone`.
- Proposal: Add `#[derive(Clone)]` to all three structs, replace the copies with `.clone()`, and build the inspect input once.
- Spec impact: none. The wire encoding is unchanged.
- Risk: low.
- Estimated delta: -30 lines.

### INP-23: A 275-line delivery test lives in input_client.rs
- Category: test-placement
- Location: `input_client.rs:1464-1739` (`external_finite_wait_retries_held_preview_after_actual_composer_recovery`)
- Evidence: The test drives `crate::receive::wait`, `BrokerReceiver` and the wrapped-preview delivery path. The only client symbol it uses is `NativeInputHost.health` (line 1628). `delivery.rs` tests already host the same kind of preview tests and have the `wrapped_line` (576) and `preview_stamp` (1199) helpers.
- Proposal: Move the test to the `delivery.rs` tests and reuse those helpers.
- Spec impact: none.
- Risk: low.
- Estimated delta: -15 lines after reusing the helpers.

### INP-24: input_client.rs carries two top-level test modules, and the `InputFailure` tests are misplaced
- Category: test-placement
- Location: `input_client.rs:1451-4962` (`mod tests`), `4964-6546` (`pub(super) mod permission_history`), `6021-6106` (`step5_a1_input_failure_*`)
- Evidence: `permission_history` is separate only so that `Case`/`Retained` can be `pub(in crate::runtime)` for `input_recovery.rs:709,881`. It repeats the whole import block (4966-4976). The `InputFailure` conversion tests exercise `From<InputFailure> for CafleetError` (52-79) and are unrelated to permission history.
- Proposal: Fold `Case`, `Retained` and `Snapshot` into `mod tests`, keep `inspect_errors`, `entry`, `review` and `staged` as nested modules, and move the `InputFailure` tests beside the other unit tests.
- Spec impact: none.
- Risk: low.
- Estimated delta: -12 lines.

### INP-25: Smaller duplications inside input_client.rs production code
- Category: duplication
- Location:
  - database-path identity check at `277-285` and `597-607`
  - association-unchanged predicate at `1137-1143` and `1334-1340`
  - member-placement lookup at `1111-1115` and `1312-1314`
  - `PermissionReceiptIdentity { database_identity_json, run_id }` at `399-402`, `470-473`, `1007-1010` and `delivery.rs:344-347`
  - `wire.rs:187-191` repeats the five calls of `validate_respond` (`input_client.rs:853-871`)
- Proposal:
  - one path-check helper that takes the two messages;
  - `same_association(view, original)`;
  - `permission_placement(conn, run)`;
  - a `PermissionReceiptIdentity::for_run(&RunRecord)` constructor in `broker/runtime/records.rs`;
  - `validate_respond(&input)?` at `wire.rs:187`.
  Keep every message verbatim.
- Spec impact: none. `validate_respond`'s extra positive-id check is already guaranteed by `object.positive(...)` at `wire.rs:168-172`, so the first failing check is the same.
- Risk: low.
- Estimated delta: -37 lines.

### INP-26: `SocketHost::failure` duplicates `host_failure`, and diagnostic `eprintln!` calls are left in tests
- Category: redundant-test
- Location: `input_client.rs:3408-3416` against `154-162`; `eprintln!` at `2255-2262`, `4331-4342`, `6398-6404`, `6438-6441`
- Evidence: The two failure helpers are identical, and other test hosts already call `host_failure` (3094, 3116). The prints repeat state that the following assertions already check.
- Proposal: Use `host_failure`. Delete the prints, or fold any value that is genuinely needed into an assertion message.
- Spec impact: none.
- Risk: low.
- Estimated delta: -37 lines.

### INP-27: Test clocks and I/O loops re-implement production helpers
- Category: duplication (tests)
- Location:
  - `input_service.rs:90-112` (copy of `RealInputDeadlineClock`, input_clock.rs:22-39), used 3 times in `native.rs`
  - `service.rs:1421-1442` (`Clock::at` = `ScriptClock::one(...)`)
  - write-all loops at `input_service.rs:191-206`, `263-278` and `runner_tests/mod.rs:47-75`
  - `input_service.rs:118-123` `pause` against `control.rs:16-22`
  - the submission-reply decode at `input_service.rs:246-257` and `732-736`
- Proposal:
  - Use `RealInputDeadlineClock` directly.
  - Expose `ScriptClock` (`input_clock.rs:123-149`) as `#[cfg(test)] pub(super)` and use it in `service.rs`.
  - Write one `write_all_until(&mut impl Write, bytes, until)`.
  - Call `control::pause(until)`.
  - Reuse `submission()` with the asserts passed in.
- Spec impact: none.
- Risk: low.
- Estimated delta: -65 lines.

### INP-28: Clock tests assert guarantees the type system already gives
- Category: redundant-test
- Location: `input_clock.rs:178-179` (the same assert twice); `324, 339, 350, 364, 374, 386, 403, 409, 412`; `received()` at 208
- Evidence: `receive(&self)` on a plain `u64` newtype with no interior mutability cannot change `monotonic_ns()`.
- Proposal: Keep the single check in `step5_b0_clock_receive_reuses_immutable_expiration_without_phase_extension` and drop the rest.
- Spec impact: none.
- Risk: low.
- Estimated delta: -12 lines.

### INP-29: An alias re-export block in runtime/mod.rs serves only the test harness
- Category: over-abstraction
- Location: `cafleet/src/runtime/mod.rs:8-16`
- Evidence: Five `x as input_service_x` aliases exist only because generic helper names (`barrier`, `records`, …) are flattened into `crate::runtime`.
- Proposal: After INP-01, define the helpers with their final names in `runner/service.rs` (e.g. `with_input_service`) and let callers import from `crate::runtime::runner`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -8 lines.

### INP-30: `InputHooks` has one production implementor and two `InputBoundary` variants that are never constructed
- Category: dead-code
- Location: `runner/input.rs:3646-3681` (trait, `#[expect(dead_code)]` enum, `InertInputHooks`), `8461-8485` (test impl)
- Evidence: `InputBoundary::Settled` and `InputBoundary::Retired` appear only in the declaration (3655-3656) and the test match (8474-8475). Production emits only `Reserved`, `BeforeWrite`, `AfterWrite` and `TerminalState` (5869, 5891, 5923, 6032). The trait's only production impl is `InertInputHooks` (service.rs:927); the checkpoint-ordering tests depend on the seam.
- Proposal: Delete the two variants, the `#[expect]` attribute and the two test arms. Keep the trait as the test seam: `checkpoint()` still performs the production deadline check.
- Spec impact: none. The removed variants are never emitted.
- Risk: low.
- Estimated delta: -9 lines.

### INP-31: Three identical panicking `RequestTokenSource` test implementations
- Category: duplication (tests)
- Location: `runner/input.rs:2496-2501` (`NoTokens`), `7108-7114` (`UnusedTokens`), `8422-8427` (`support::Tokens`)
- Evidence: All three implement `fill` as a panic. The trait has one production impl (`service.rs:118-125 SystemRequestTokens`) and is a legitimate seam for deterministic tokens and the entropy-failure test (`input.rs:570-599`).
- Proposal: Keep the trait. Replace the three test impls with one `support::PanicTokens(&'static str)`.
- Spec impact: none.
- Risk: low.
- Estimated delta: -12 lines.

### INP-32: The drain proof always returns `Quiescent`, leaving four `unreachable!` destructures; the drain validation is duplicated
- Category: over-abstraction / duplication
- Location:
  - `runner/input.rs:4950-4959, 5540-5550, 5677-5687, 5771-5778`; `transport.rs:26-33`
  - `fresh_drain` / `acquire_write_guard` at `input.rs:5525-5539` and `5640-5687`
- Evidence:
  - `DrainedInput::evidence()` builds only `InputDrain::Quiescent`. Its four callers each `let … else { unreachable!() }`, and three of them repeat the manual-generation `ensure`.
  - In `fresh_drain`, `validate_permission_binding` runs twice around `transport.take_input_drain()`, which mutates only `transport`, so both calls see the same state.
- Proposal:
  - Replace `evidence()` with `human_generation()` / `output_seq()` accessors.
  - Add one `current_drain_seq(proof, message)` helper.
  - Drop the second binding validation and keep the first, so the drain is not consumed on failure.
  - Extract one `validate_taken_drain` for both functions.
- Spec impact: none. Same checks; error texts are passed as parameters.
- Risk: low.
- Estimated delta: -40 lines.

### INP-33: `retire_received` repeats validations that can never fail
- Category: other (overly defensive duplicate validation)
- Location: `runner/input.rs:5182-5205` (and, optionally, 5168-5179)
- Evidence: `validate_receipt_cache(&input)?` (5182) already ensures `input.run_id == self.run.run_id` and cached-field equality (4972-4985). The conjunct at 5184 and the `ensure` at 5196-5204 therefore always pass.
- Proposal:
  - Delete both re-checks.
  - Optional, needs an owner decision: replace 5168-5179 with `validate_receipt_owner`. This changes two internal invariant texts that no test or document asserts.
- Spec impact: none for the mandatory part.
- Risk: low.
- Estimated delta: -10 lines (-20 with the optional part).

### INP-34: Smaller repeated literals and branches in runner/input.rs production code
- Category: duplication
- Location and proposal:

  | Item | Location | Proposal |
  |---|---|---|
  | `InputFailure { source, run_id: Some(self.run.run_id), … }` literals | 8 sites (4276, 4618, 4646, 4760, 5144, 5215, 5298, 5477) | An `InputContext`-style builder, already present at `input_client.rs:80-91` |
  | Duplicated permission-response prelude | `retained_permission_response` and `permission_received` | One private function |
  | `Held` result literals | 4719, 5414, 5428, 5443 | `SubmissionResult::held(reason)`; fold the two byte-identical plan-unavailable exits while keeping the `coding_agent(...)` lookup order |
  | Literal `4096` | 4733, 5423 | `const RECEIPT_CACHE_CAPACITY` |
  | State→`SubmissionResult` mapping, duplicated against `input_client.rs:239-252` | 5238-5256 | Share it; the runner maps `Ready` to `Held` |
  | `WithDiagnostics` built by hand | 6025, 6105, 6247 | One helper |
  | Unreachable match arms from passing the whole request | 4615, 4643, 5013, 5140 | Pass `&PermissionRespondInput` instead |
  | `InputCheckpoint.input_id`, always `Some` (sole constructor at 5491) | 5271-5273 | Make it `i64` and drop the redundant `output_seq` assignment at 4993-4996 |
  | `observe_manual(0)` guard (6206-6212) | 6206-6212 | Collapse to `.and_then(|c| self.observe_manual(c))` and make the method private |

- Spec impact: none. Same strings and the same ordering.
- Risk: low.
- Estimated delta: -95 lines.

### INP-35: The settlement branch is duplicated between runner/input.rs and input_recovery.rs
- Category: duplication
- Location: `runner/input.rs:5088-5126` against `input_recovery.rs:303-339`
- Evidence: Both choose between `settle_permission_response_in_transaction(...)` for `PermissionResponse` and `settle_input_in_transaction(...)` otherwise. They are the only non-storage callers of each function.
- Proposal: Add one storage-level `settle_any_input_in_transaction(tx, owner, input, disposition, evidence_ref, proof: Option<&ZeroByteRetirement>, deadline)`. Each caller keeps its own proof derivation.
- Spec impact: none.
- Risk: low.
- Estimated delta: -15 lines.

### INP-36: Small helpers duplicated across runner/input.rs tests
- Category: duplication (tests)
- Location:
  - `pending()` at 338-352 and 6413-6427
  - `HostProfile::synthetic(EvidencePolicy::metadata(), "metadata")` at 96, 8590, 8880, 8897
  - `Host::new` (8586-8596) repeating `with_host_until` (8876-8887)
  - `form` (232-238) against `gated::show` (3088-3091)
  - `native_screen_bytes` (105-135) re-implementing `coding_agent/profiles/detector.rs:116-156 replay_native_fixture`
  - negative trait-impl assertions at 720-730 and 2472-2495; the same trick appears at `detector.rs:1294`, `profiles.rs:490,3346`, `catalog.rs:308` and `observation.rs:213`
- Proposal:
  - one `support::pending_snapshot`;
  - `HostProfile::metadata()`;
  - `Host::new` calls `Host::until`;
  - `show` used for `form`;
  - a shared `#[cfg(test)] native_fixture_bytes` in the detector tests (coordinate with the coding_agent slice);
  - one crate-level `#[cfg(test)] assert_not_impl!` macro.
- Spec impact: none.
- Risk: low.
- Estimated delta: -80 lines in-slice.

### INP-37: input_recovery.rs: a duplicate pane-pair check, a self-validating real clock, and repeated test helpers
- Category: other (overly defensive duplicate validation) / duplication (tests)
- Location:
  - `input_recovery.rs:152-155` against `44-47` (the same predicate and string again, as the first statement of `validate_snapshot` called at 158)
  - `input_clock.rs:40-46` (the real clock calls `sample.validate()`, which `capture`/`receive` call again at 54 and 75)
  - test helpers repeated inline at `input_recovery.rs:736-743` / `654-661` (spool); `827-830, 872-876, 971-975` / `646-652` (released-lock check); `994-1002` / `1025-1033` (`notified_at`); `726-730, 891-895, 603-611` (placement)
  - optional: the forwarder at `185-192` to `ZeroByteRetirement::from_exclusive_guard`
- Proposal: Delete the first pane-pair check and the real clock's self-validation. Extract free test functions `spool`, `assert_lock_released`, `notified_at` and `placement`.
- Spec impact: none. The same errors are raised before any side effect, and the `host.calls.is_empty()` test at 1145 still holds.
- Risk: low.
- Estimated delta: -40 lines.

### INP-38: cafleet/src/input.rs mixes three concerns; member/fleet deletion orchestration does not belong in the input module
- Category: other (cohesion)
- Location:
  - `cafleet/src/input.rs:1-293` (SQLite wait budget: `Deadline`, `with_database_budget`)
  - `519-816` (pane input lock: `PaneLock`, `lock_member`, `lock_deletion`)
  - `1289-1776` (deletion: `DeletionPlacement`, `DeletionHost`, `end_member`, `delete_fleet`, `unplaced_member_outcomes`; ~490 lines)
- Evidence: The deletion code is consumed by `cli/member.rs:210,554-660` and `cli/fleet` through `delete_fleet`, and its tests live in three different places: `input.rs:1081-1286`, `runner_tests/fleet_deletion.rs` and `runner_tests/recovery/deletion.rs`.
- Proposal: Move the deletion orchestration and its tests into a dedicated module (e.g. `cafleet/src/deletion.rs`, next to the CLI deletion callers). `input.rs` keeps the database budget and the pane lock, and the deletion module consumes `lock_deletion`.
- Spec impact: none. Code motion only.
- Risk: low.
- Estimated delta: 0 lines moved. It removes one conceptual overlap and gives the three deletion test files one home.

### INP-39: Identical `DeletionHost::shutdown` impls and a forwarding `deletion_run_identity`
- Category: duplication / over-abstraction
- Location: `cafleet/src/input.rs:1380-1387` and `1437-1444` (`shutdown`), `1463-1465` (`deletion_run_identity`)
- Evidence:
  - Both production impls (`MultiplexerDeletionHost`, `NativeFleetDeletionHost`) implement `shutdown` as `request_shutdown(conn, run, deadline.instant())`.
  - The trait already uses a default method for `recover` (1343-1362).
  - The three test impls override `shutdown` (`broker/mod.rs:6277`, `runner_tests/fleet_deletion.rs:62`, `runner_tests/recovery/deletion.rs:48`), so a default changes none of them.
  - `deletion_run_identity` only calls `crate::runtime::runner::stable_run_identity` (4 call sites). `delivery.rs:469` calls `stable_run_identity` directly.
- Proposal: Make `shutdown` a default trait method, delete both overrides, and call `stable_run_identity` directly.
- Spec impact: none.
- Risk: low.
- Estimated delta: -15 lines.

### INP-40: Repeated pane-lock error mapping and lock-path recomputation in cafleet/src/input.rs
- Category: duplication
- Location: `cafleet/src/input.rs:714-718`, `729-735`, `792-798`, `1315-1320` (the same `CafleetError::App(format!("member {member_id} pane input: {}", error.message()))` map); `lock_deletion` recomputes `lock_path` (714) for targets whose path `deletion_placements` already computed (1315)
- Proposal: Add one `fn member_pane_input_error(member_id) -> impl FnOnce(CafleetError) -> CafleetError` helper. Optionally let `lock_deletion`'s loader return `(path, member_id, placement)` triples so the path is computed once.
- Spec impact: none. The strings are identical.
- Risk: low.
- Estimated delta: -15 lines.

### INP-41: `lock_deletion` re-implements `PaneLock::acquire_within`; `Deadline::retry` has a single caller (needs-verification)
- Category: duplication
- Location: `cafleet/src/input.rs:727-741` against `613-627`; `Deadline::retry` at `65-73` (sole caller: line 738)
- Evidence:
  - Both loops retry `try_acquire` with a sleep of at most 50 ms until the deadline expires.
  - They differ at the boundary. `lock_deletion` checks `deadline.remaining()?` before each try, yielding `DeadlineExceeded`, and maps exhaustion to `busy(member_id)`. `acquire_within` tries once even when the deadline is already expired and returns `None`.
- Proposal: Replace the inner loop with `deadline.remaining()?; PaneLock::acquire_within(...)?.ok_or_else(|| busy(member_id))?`, then delete `Deadline::retry`.
- What would confirm: that no test or SPEC.md error expectation depends on `DeadlineExceeded` versus the busy message when the deadline expires mid-wait (`fleet_deletion_contention_preserves_rows_and_releases_preceding_guards`, `input.rs:1082`, only asserts `is_err`).
- Spec impact: none if confirmed. Otherwise keep the loop and only inline `retry`.
- Risk: medium.
- Estimated delta: -20 lines.

### INP-42: `Deadline::inherited_or_new` duplicates the frame-minimum and relies on a `Default` impl
- Category: duplication
- Location: `cafleet/src/input.rs:25-36`, `76-80`, `114-118`
- Evidence:
  - The `frames.iter().map(|f| f.deadline).min_by_key(...)` fold appears both in `inherited_or_new` and in `inherited_database_deadline`.
  - `impl Default for Deadline` has no user other than the `.unwrap_or_default()` at line 35 (grep `Deadline::default` and `Default::default()` found none).
  - Absence of a scope is a valid state, so a 5-second default is legitimate. Writing it as `unwrap_or_else(Deadline::new)` states it explicitly.
- Proposal: Extract `fn innermost_deadline(waits) -> Option<Deadline>`. Write `inherited_or_new` as `…unwrap_or_else(Deadline::new)` and delete the `Default` impl.
- Spec impact: none.
- Risk: low.
- Estimated delta: -10 lines.

### INP-43: The primary/cleanup result merge is hand-written repeatedly
- Category: duplication (cross-slice)
- Location: `cafleet/src/input.rs:288-292`, `runner/lifecycle.rs:175-179` and `205-212`, `runner/native_capture/files.rs:503`, `runner/native_capture/verification.rs:77, 665`
- Evidence: Each site matches `(result, cleanup)` and combines the two errors with `primary.with_cleanup(cleanup.message())`.
- Proposal: Add `pub(crate) fn CafleetError::merge_cleanup<T>(result: Result<T>, cleanup: Result<()>) -> Result<T>` in `error.rs` and call it at each site. `runner/input.rs:6243 combine_manual_progress` intentionally builds `WithDiagnostics` instead, so keep it separate.
- Spec impact: none. The same error variant and message are produced.
- Risk: low.
- Estimated delta: -20 lines.

### INP-44: `CancellationScope` repeats the connection-identity tracking that `DatabaseBudget` already enforces (needs-verification)
- Category: duplication
- Location: `runner/lifecycle.rs:90-181` (`CANCELLATION_SCOPES` identity and deadline stack) against `cafleet/src/input.rs:82-120, 151-236` (`DATABASE_WAITS`)
- Evidence: `CancellationScope::new` has one caller, inside `with_database_deadline` (`lifecycle.rs:202`), which always runs inside `with_database_budget`. That function already asserts a single connection identity per thread (`input.rs:164-172`). The `CANCELLATION_SCOPES.identity` assertion (`lifecycle.rs:105-110`) therefore re-checks the same invariant, and its deadline stack mirrors `DATABASE_WAITS.frames`.
- Proposal: Drop the identity field from `CANCELLATION_SCOPES`, and keep only what the progress handler needs (the active deadline stack). Alternatively, let the cancellation scope read `DATABASE_WAITS`.
- What would confirm: that `database_cancelled` reads only the deadline list. This item belongs to the lifecycle owner.
- Spec impact: none.
- Risk: medium.
- Estimated delta: -15 lines.

### INP-45: cafleet/src/input.rs mixes `pub` and `pub(crate)` for crate-internal items, and names live fields with underscores
- Category: other
- Location:
  - `cafleet/src/input.rs:15, 37, 45, 65, 238, 519, 527, 575, 613, 644, 762, 1600` (`pub`)
  - `lib.rs:23` (`pub mod input`)
  - `_file` (520, read at 560), `_guard` (646, read at 656-657), `_guards` (665, read at 688, 696)
- Evidence: `main.rs` uses only `cafleet::cli::run`, and no `benches/`, `tests/` or `examples/` exist, so `pub` is effectively crate-internal. The underscore prefix conventionally marks an unused binding, but these fields are read.
- Proposal: Use `pub(crate)` uniformly, which also lets the compiler report dead items. Rename the fields to `file`, `guard` and `guards`. Coordinate `lib.rs` visibility with its slice owner.
- Spec impact: none. The library API is not a shipped contract; needs-verification that the crate is not published as a library.
- Risk: low.
- Estimated delta: 0 lines; enables dead-code detection.

### INP-46: cafleet/src/input.rs test modules sit mid-file, and deletion tests are mixed into the lock tests
- Category: test-placement
- Location: `cafleet/src/input.rs:295-517` (`database_scope_tests`), `818-1287` (`pane_lock_wait_tests`, including the fleet-deletion tests at 1081-1286); external tests in `runner_tests/fleet_deletion.rs`, `runner_tests/recovery/deletion.rs` and `runner_tests/recovery/foundations.rs` (the `pane_lock_for` tests)
- Proposal: Use one `#[cfg(test)] mod tests` at the end of `input.rs`, with nested `database_scope` and `pane_lock` modules. Move the `pane_lock_for` tests from `foundations.rs` here. Move the deletion tests, including the two runner_tests files, into the deletion module of INP-38. Their three test `DeletionHost` impls stay where they are, because each one fakes different behavior.
- Spec impact: none.
- Risk: low.
- Estimated delta: -10 lines (headers and imports); the rest is moved.

### INP-47: Possibly redundant tests (needs-verification)
- Category: redundant-test
- Location:
  - `input_client.rs:3978-4033` (ordinary post-commit expiry) against `2753-2857` (permission variant)
  - `runner/input.rs:589-594, 620-623` (weak `is_err() || token none` tails) against test 895
  - `runner/input.rs:6264-6388` (4097-cycle capacity test with full per-cycle assertions)
  - one-line wrapper tests at `input_client.rs:5574-5597` and `5672-5691`
- Evidence: The ordinary test's unique assertion is literal privacy (4004), and both tests go through `reconcile_receipt_with_hosts`. Test 895 asserts the same blockers more strongly, except for `calls == 2`.
- Proposal:
  - Fold the literal-privacy assertion into the permission test and delete the ordinary one.
  - Move `calls == 2` into test 895 and delete the tails.
  - Assert the capacity test's per-cycle details on the first and last cycles only.
  - Make the wrapper groups table-driven.
- What would confirm: that no ordinary-kind branch exists in the reconcile path (context built at 658-680, where only `interaction_id` differs), plus an owner decision on the per-cycle coverage.
- Spec impact: none.
- Risk: low.
- Estimated delta: about -130 lines.

### INP-48: Test names carry design-step provenance
- Category: stale-doc (historical residue)
- Location: `step5_*` and `step5_a1_*` / `step5_b0_*` / `step5_wire_*` test names in `runner/input.rs` (13), `input_client.rs` (5575-6296), `input_clock.rs`, `wire.rs` and `input_service.rs`
- Proposal: Drop the step and finding prefixes. The enclosing modules already scope the behavior.
- Spec impact: none.
- Risk: low.
- Estimated delta: 0 lines.

### INP-49: Unused or mismatched serde derives on protocol types (needs-verification)
- Category: dead-code
- Location: `input_protocol.rs:55-56, 62-63, 78-79`
- Evidence:
  - `PermissionResponseResult` derives `Serialize, Deserialize` with a nested `submission` field, but the real wire shape is flat. It is built by hand in `presentation::permission_response_result` and decoded by hand in `replies::response`, and no `from_value`/`to_value` of this type exists.
  - `PermissionInspectInput` and `PermissionRespondInput` are decoded field by field (`wire.rs:155-180`); only their `Serialize` is used (`wire.rs:262, 266`).
- Proposal: Remove the unused derives and their `deny_unknown_fields`.
- What would confirm: a passing `mise //cafleet:typecheck`.
- Spec impact: none if it compiles.
- Risk: low.
- Estimated delta: -4 lines.

### Verified and kept
- **`InputCommandHost` trait:** one production implementor, but seven test hosts depend on it. It is a real seam.
- **`InputDeadlineClock` trait:** a real test seam with one production implementor.
- **`strict_json`:** the only strict duplicate-key JSON parser in the repository.
- **Two-pass routing (`routes_input`, then the strict parse):** required so that legacy frames keep their current behavior.
- **Fallbacks:** the `map_or` clamps (INP-12) and the `unwrap_or_else` default reason at `runner/input.rs:4181` (asserted as `UNAVAILABLE` at 873) are legitimate defaults for valid absent states. No banned fallback was found in the slice.
- **Early blocker checks in `permission_received` (`runner/input.rs:4656-4669`):** they decide which error text comes first, so removing them would change observable errors.
- **Encoder self-validation (`wire.rs:297`, `replies.rs:296`):** a deliberate "reject what we would send" invariant.
