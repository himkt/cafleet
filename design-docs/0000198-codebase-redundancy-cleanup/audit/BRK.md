# Audit: broker and persistence (BRK)

## Summary
The slice has no test-only *files*, but `cafleet/src/broker/mod.rs` is a 13-line module declaration followed by a 6,223-line `runtime_contract_tests` module (lines 14-6236) that exercises `broker/runtime/*`, `presentation.rs`, `output/*` and the V4 migration — every one of those tests belongs in the implementation file it exercises. The shared `broker::tests` helper module is imported by 20 files across the crate and should be split onto the modules that own the types it builds. The largest production-code wins are: replacing ~270 `.map_err(db_err)` call sites (~125 in-slice) with one `From<rusqlite::Error>` conversion living in `error.rs`; deleting or test-gating seven `pub` functions that only tests call (`register_member`, `deregister_member`, `update_placement_pane_id`, `create_run`, `reserve_input`, `settle_input`, `release_permission_claim`); merging the two identical enum macros; and removing duplicated validation in `events.rs`/`permissions.rs`. Test-side scaffolding is heavily duplicated (six row-dump helpers, four migrate-to-version fixtures, a second `refinery::embed_migrations!`, repeated SQL-domain tests). Estimated total: ~580 lines removable in-slice (≈ -250 more repo-wide if the `db_err` and SHA-256-hex consolidations are applied crate-wide), with the shipped CLI/SQL/error contract unchanged.

## Test placement inventory
| Test-only path | Lines | Target implementation file(s) | Shared helpers / fixtures | Notes |
|---|---|---|---|---|
| `cafleet/src/broker/mod.rs` `mod runtime_contract_tests` § `interaction_refresh` | 26-502 | `cafleet/src/broker/runtime/events.rs` (`update_interaction_evidence`) | `Fixture`, `raw_evidence`, `rows` (see helper row) | Misplaced test module inside a non-owning file. |
| `broker/mod.rs` § `permission_responses` | 503-2199 | `cafleet/src/broker/runtime/permissions.rs` | `ResponseFixture` (keep local to permissions tests), `Fixture` | `ResponseFixture` builds on `Fixture`; move together with the permission tests. |
| `broker/mod.rs` § `permission_responses::runtime_permission_presentation_flattens_the_audit_in_shipping_order` | 2200-2259 | `cafleet/src/presentation.rs` (asserts `presentation::input_operation` + `output::format_input_operation`) | `ResponseFixture` → needs a `pub(crate)` `#[cfg(test)]` constructor in permissions.rs | Presentation contract, not storage. |
| `broker/mod.rs` § `versioned_events` | 2260-3075 | `cafleet/src/broker/runtime/events.rs` (v1/v2 payload reader/writer) | `legacy_opening` needs a migrate-to-V4 helper owned by `db/mod.rs` (BRK-03); drops the duplicate `mod v4_migrations { refinery::embed_migrations!("migrations"); }` at 2265-2267 | |
| `broker/mod.rs` § `automatic_review_events` (excluding nested `effective_policy_writes`) | 3076-3489 | `cafleet/src/broker/runtime/events.rs` (`director_action`, automatic review events) | `Fixture` | |
| `broker/mod.rs` § `automatic_review_events::effective_policy_writes` | 3490-3945 | `cafleet/src/broker/runtime/effective_policy.rs` | `Fixture` | |
| `broker/mod.rs` shared runtime fixture: `NOW`, `SECRET`, `Fixture`, `raw_evidence`, `metadata`, `opening`, `closure`, `rows`, `durable_state`, `assert_no_secret`, `assert_key_order` | 3947-4185 | `#[cfg(test)] pub(crate) mod fixtures` inside `cafleet/src/broker/runtime/mod.rs` (owner of `NewRun`/`RunRecord`/`create_run`) | `rows` should be the single crate test row-dump helper (BRK-03) | `assert_key_order` is JSON-output-contract scaffolding; it can live with the presentation tests that use it. |
| `broker/mod.rs` run tests (`new_run_records_…` … `failed_sequence_reservation_rolls_back_…`, `concurrent_sequence_reservations_commit_disjoint_ranges`, `missing_database_instance_identity_…`) | 4186-4365, 5761-5786, 5856-5864 | `cafleet/src/broker/runtime/mod.rs` | `Fixture` | |
| `broker/mod.rs` transition tests (`opening_commits_…` … `event_read_rejects_a_tampered_…`, `transition_outside_reserved_sequences_…`, terminal/event-shape regressions) | 4366-4794, 5787-5815, 5865-6236 | `cafleet/src/broker/runtime/events.rs` | `native_exit`, `replace_public_payload` move with them | |
| `broker/mod.rs` input/receipt tests (`receipt_key_digest_…` … `settled_receipt_history_…`, `failed_input_reservation_and_corrupt_receipt_reads_…`) | 4795-5085, 5816-5855 | `cafleet/src/broker/runtime/inputs.rs` | `Fixture::input` | |
| `broker/mod.rs` evidence tests (`metadata_removes_all_screen_fields…` … `evidence_utf8_and_complete_payload_bounds…`) | 5086-5404 | `cafleet/src/broker/runtime/evidence.rs` | `EVIDENCE_KEYS`, `redacting`, `independent_evidence_digest` | `metadata_and_redacted_transitions_remain_secret_free_through_storage_and_updates` spans storage; place in events.rs if evidence.rs tests must stay storage-free. |
| `broker/mod.rs` presentation tests (`runtime_presentation_*`, `effective_policy_record_is_rendered_last_in_every_runtime_snapshot_surface`, `event_and_input_read_wrappers_follow_the_approved_order_and_compact_text`) | 5405-5520, 5544-5605, 5689-5760 | `cafleet/src/presentation.rs` (JSON) / `cafleet/src/output/runtime.rs` (text) | `Fixture` via `runtime::fixtures` | |
| `broker/mod.rs` effective-policy reader tests (`effective_policy`, `store_effective_policy`, `effective_policy_reader_*`) | 5521-5543, 5606-5688 | `cafleet/src/broker/runtime/effective_policy.rs` | | |
| `broker/mod.rs` `pub(crate) mod tests` (shared helpers `migrated_conn`, `placement`, `create_fleet`, `register`, `FixtureDeletionHost`, `guarded_delete_fleet`) | 6238-6326 | Split by owner (BRK-02): `migrated_conn` → `db/mod.rs`; `create_fleet` → `broker/fleets.rs`; `placement`, `register` → `broker/members.rs`; `FixtureDeletionHost` + `guarded_delete_fleet` → `cafleet/src/input.rs` (owner of `DeletionHost`/`delete_fleet_with_host`) | Imported by 20 files: `assets.rs`, `input.rs`, `cli/{mod,member,doctor}.rs`, `db/mod.rs`, `runtime/runner/input_client.rs`, `runtime/runner_tests/mod.rs` and every broker file | Test-support module living in a non-owning file. |
| `cafleet/src/db/mod.rs` `mod runtime_tests` | 239-2175 | Already in the owning file (`db/mod.rs` owns the embedded migration chain and schema guard) | `Fixture` (db variant), `v4_fixture`/`fixture_at`, `query_rows` ×2 | Merge with the second `mod tests` (2177-2434) into one `#[cfg(test)] mod tests` with `permission_migration` / `automatic_review_migration` / `runtime_schema` submodules. |

No test fixtures (JSON or otherwise) are loaded by tests in this slice; `cafleet/migrations/*.sql` are runtime data embedded via `refinery::embed_migrations!`.

## Findings

### BRK-01: Redistribute `runtime_contract_tests` out of `broker/mod.rs`
- Category: test-placement
- Location: cafleet/src/broker/mod.rs:14-6236
- Evidence: `broker/mod.rs` has 13 lines of module declarations/re-exports and a 6,223-line `#[cfg(test)] mod runtime_contract_tests` that imports `super::runtime::{self as storage, *}` and calls `storage::*`, `crate::presentation::*` (17 call sites) and `crate::output::format_*` (8 call sites). None of the tested items are defined in `broker/mod.rs`.
- Proposal: Move each group into the implementation file listed in the inventory. Put the shared `Fixture`/evidence builders in a `#[cfg(test)] pub(crate) mod fixtures` inside `broker/runtime/mod.rs` so every runtime file's `mod tests` and `presentation.rs`/`output/runtime.rs` tests import one fixture. Drop the per-group preambles that re-import the same crates.
- Spec impact: none — test-only code moves; no production item changes.
- Risk: low — mechanical move; `cargo test` names change (module paths), which nothing consumes.
- Estimated delta: -40 lines (duplicated `use` preambles and module wrappers; the bulk moves rather than disappears)

### BRK-02: Split the crate-wide `broker::tests` helper module onto its owning modules
- Category: test-placement
- Location: cafleet/src/broker/mod.rs:6238-6326
- Evidence: `pub(crate) mod tests` is imported as `crate::broker::tests` / `common` in 20 files (grep `broker::tests`). `migrated_conn` wraps `db::connect`+`db::migrate_to_head`; `guarded_delete_fleet` wraps `input::delete_fleet_with_host` with a `DeletionHost` impl — neither is a broker type. `delivery.rs:547-565` re-implements `create_fleet`/register helpers with identical literals (`"main", "@1", "%0", "claude", "tmux"`).
- Proposal: `#[cfg(test)] pub(crate) fn migrated_conn` in `db/mod.rs`; `#[cfg(test)] pub(crate) fn create_fleet_fixture` in `fleets.rs`; `placement`/`register` in `members.rs`; `FixtureDeletionHost`/`guarded_delete_fleet` in `input.rs`. Point `delivery.rs` tests at the shared helpers instead of its private copies (cross-slice).
- Spec impact: none — test helpers only.
- Risk: low — import-path edits across 20 test modules.
- Estimated delta: -10 lines in-slice (≈ -20 more in `delivery.rs`)

### BRK-03: Collapse duplicated test scaffolding (row dumps, migrate-to-version fixtures, second migration embed, fixture structs, instruction constants)
- Category: redundant-test
- Location: cafleet/src/broker/mod.rs:83-99, 2265-2267, 2269-2271, 2285-2327, 3079-3081, 4109-4131; cafleet/src/db/mod.rs:253-308, 955-1018, 978-986, 2185-2203
- Evidence: Six near-identical "SELECT … collect rows as `Vec<Vec<Value>>`" helpers: `broker/mod.rs` `rows` (4109), `state` (83), `legacy_rows` (2285); `db/mod.rs` `query_rows` (274), `query_rows` (978, byte-identical to 274), `rows` (2195). Four migrate-to-version fixtures: `db/mod.rs` `v4_fixture` (253, equals `fixture_at(4)` at 955 except the fleet name), `fixture_at` (955), `at_version` (2185), and `broker/mod.rs` `legacy_opening` (2316) which needs its own `mod v4_migrations { refinery::embed_migrations!("migrations"); }` (2265) because `db::embedded` is private. Two `Fixture` structs with the same `_dir/conn/fleet/director/member` shape (`db/mod.rs:1657`, `broker/mod.rs:3950`). `MULTI_ROW_SENTENCE` is declared twice in tests (2271, 3081); `REVIEW_INSTRUCTION`/`REFUSAL_INSTRUCTION`/`UNAVAILABLE_INSTRUCTION` test oracles sit in two modules.
- Proposal: Add `#[cfg(test)] pub(crate) fn migrate_to_version(conn, version)` and `#[cfg(test)] pub(crate) fn table_rows(conn, sql)` to `db/mod.rs`; replace all four fixtures and six row helpers with them; delete the second `embed_migrations!`. Keep the instruction literals as independent oracles, declared once in the events.rs test module.
- Spec impact: none — tests keep asserting the same literals and shapes.
- Risk: low — helper signatures differ slightly (`ORDER BY 1` vs `ORDER BY 1, 2`); keep the caller-supplied SQL so ordering is unchanged.
- Estimated delta: -120 lines

### BRK-04: Move `db_err` to `error.rs` as `impl From<rusqlite::Error> for CafleetError`
- Category: duplication
- Location: cafleet/src/broker/members.rs:16-29 (definition); ~270 `.map_err(db_err)` / `.map_err(crate::broker::members::db_err)` sites crate-wide (in-slice: messaging 23, members 15, fleets 15, runtime/mod 15, permissions 16, events 16, db/mod 13, inputs 7, panes 3, asset_installs 1, effective_policy 1)
- Evidence: `db_err` is a generic rusqlite→`CafleetError` mapping (busy/locked → `DatabaseBusy`, embedded `CafleetError` passthrough, else `App("database error: …")`) but lives in `broker::members`; `db/mod.rs`, `input.rs`, `diagnosis.rs`, `delivery.rs`, `receive.rs`, `cli/*` and `runtime/runner/*` reach into `crate::broker::members::db_err`. No `impl From<rusqlite::Error>` exists (`grep "impl From" error.rs` is empty).
- Proposal: Implement `From<rusqlite::Error> for CafleetError` in `error.rs` with the exact `db_err` body; replace `.map_err(db_err)?` with `?` and delete every `use …::db_err` import. Keep the explicit custom mappings (e.g. `connect_with_flags`'s "failed to open database at …" / "failed to apply connection PRAGMAs: …", `reference_schema`'s "reference migration failed: …") as explicit `map_err` calls.
- Spec impact: none — the conversion produces the same variants and messages as `db_err`.
- Risk: medium — an implicit `?` conversion can silently replace an intentionally different mapping in future edits; reviewers must confirm each converted site currently uses `db_err` and not a custom message.
- Estimated delta: -120 lines in-slice (≈ -150 more repo-wide; rustfmt places most `.map_err(db_err)` on their own line)

### BRK-05: Test-gate or delete `pub` functions that only tests call
- Category: dead-code
- Location: cafleet/src/broker/members.rs:31-44 (`register_member`), 131-136 (`deregister_member`), 174-187 (`update_placement_pane_id`); cafleet/src/broker/runtime/mod.rs:322-333 (`create_run`); cafleet/src/broker/runtime/inputs.rs:101-109 (`reserve_input`), 227-244 (`settle_input`); cafleet/src/broker/runtime/permissions.rs:393-405 (`release_permission_claim`)
- Evidence: Every caller sits inside a `#[cfg(test)]` module: `register_member` (listen.rs:894 >295, delivery.rs:561 >521, cli/mod.rs:837 in `runtime_parser_tests` 311-1311, broker tests); `deregister_member` (broker/messaging/fleets/members tests, input_client.rs 2948/3159 in `mod tests` 1452-4962, input_recovery.rs 1079/1329 >389, runner_tests); `update_placement_pane_id` (members.rs tests, cli/member.rs 991/1733 >912); `create_run` (broker tests, cli/member.rs:994, input_client.rs:6169, runner_tests); `reserve_input` / `settle_input` (delivery.rs tests, cli/runtime.rs:361 >344, input_client.rs tests, runner/input.rs 2539/6988-7062/7674 inside `permission_response_contracts` 734-3640 and later test modules, input_recovery.rs:576); `release_permission_claim` (runner/input.rs:2038 in `permission_response_contracts`, broker tests). Production paths use `register_member_rows`, `deregister_member_rows`, `insert_run`, `reserve_input_with_provenance`, `settle_input_in_transaction`, `release_permission_claim_in_transaction`. `settle_input` additionally repeats the `nonblank(evidence_ref)` and `expected != Settled` checks that `settle_input_in_transaction` performs again (inputs.rs:235-240 vs 259-264), and its inner call re-runs `validate_owner` that `owned_transaction` already ran.
- Proposal: Mark these seven functions `#[cfg(test)]` (keeping them as test-only conveniences next to the production variant they wrap) and drop `settle_input`'s duplicated prechecks; remove their `pub use` entries from `broker/runtime/mod.rs:18-24` for non-test builds. `update_placement_pane_id` has no production need at all and becomes a test helper in members.rs.
- Spec impact: none — no CLI command or runtime path calls them; SPEC.md/docs/skills do not mention them (grep).
- Risk: low — compile errors pinpoint any missed production caller.
- Estimated delta: -60 lines of production code (≈ -30 net if all seven are kept as `#[cfg(test)]` helpers)

### BRK-06: Merge the `get_fleet` forwarder and centralize the "fleet not found" lookup
- Category: over-abstraction
- Location: cafleet/src/broker/fleets.rs:13-25, 104-105, 163-165, 168-169; callers cafleet/src/input.rs:1626-1627, cafleet/src/cli/fleet.rs:84-85
- Evidence: `pub fn get_fleet` is a one-line forward to `pub(crate) fn fetch_fleet`; both are used (fetch_fleet ×6, get_fleet ×1 production + tests). `CafleetError::App(format!("fleet '{fleet_id}' not found."))` after `fetch_fleet(...)?.ok_or_else` is written four times (fleets.rs:105, 169, input.rs:1627, cli/fleet.rs:85).
- Proposal: Keep one `get_fleet` (rename `fetch_fleet` call sites) and add `pub(crate) fn require_fleet(conn, id) -> Result<FleetRecord>` emitting the exact message; use it in `require_live_fleet`, `delete_fleet_rows`, `input.rs` and `cli/fleet.rs`. `cli/member.rs:381` keeps its distinct `Usage("Fleet '…' not found.")` contract.
- Spec impact: none — identical message and variant at every site.
- Risk: low
- Estimated delta: -15 lines

### BRK-07: Unify `stored_enum!` and `runtime_enum!`
- Category: duplication
- Location: cafleet/src/broker/records.rs:5-32; cafleet/src/broker/runtime/records.rs:7-35
- Evidence: The two macros are line-for-line identical (`as_str`, `TryFrom<&str>` → `InvalidStoredValue { field, value }`, `FromSql`) except `runtime_enum!` also derives `Serialize/Deserialize` with `#[serde(rename = $wire)]` and emits `VALUES`.
- Proposal: Keep `runtime_enum!` (rename to `stored_enum!`), define it once in `broker/records.rs` (`macro_rules!` + `pub(crate) use`), and declare `MemberStatus`, `MemberKind`, `MessageStatus` with it. The extra derives/const are unused for those three types and cost nothing.
- Spec impact: none — wire strings and error fields unchanged.
- Risk: low
- Estimated delta: -27 lines

### BRK-08: One member fleet/status lookup; collapse duplicate "Destination member not found" branches
- Category: duplication
- Location: cafleet/src/broker/messaging.rs:80-97, 287-294; cafleet/src/broker/runtime/mod.rs:250-263
- Evidence: `SELECT fleet_id, status FROM members WHERE member_id=?1` mapped to `(i64, MemberStatus)` is written four times (grep). `insert_message_validated` returns the identical `Value("Destination member not found: {to}")` from two branches (missing row, inactive row).
- Proposal: Add `pub(crate) fn member_fleet_status(conn, id) -> Result<Option<(i64, MemberStatus)>>` in members.rs; use it in messaging (×2) and `historical_ownership` (×2, with `.ok_or_else(invalid(...))` where presence is required — currently a bare `query_row` error). Replace the two destination branches with one `let Some((recipient_fleet, MemberStatus::Active)) = recipient else { return Err(...) };`.
- Spec impact: none — same queries and messages. `historical_ownership` must keep surfacing the rusqlite "no rows" error text, or the change must be confirmed acceptable (a missing member row there is corrupt state).
- Risk: low
- Estimated delta: -25 lines

### BRK-09: Share member/placement row inserts and kind mapping; drop a dead clippy allow
- Category: duplication
- Location: cafleet/src/broker/fleets.rs:27-98; cafleet/src/broker/members.rs:55-86, 106-112, 204-216
- Evidence: `create_fleet` repeats the `INSERT INTO members … RETURNING` and the 7-column `INSERT INTO member_placements` that `register_member_rows` performs (it cannot call `register_member_rows` because that requires a live fleet first). `kind: if row.get(n)? { MemberKind::Director } else { MemberKind::Member }` plus `backend.map(|_| Placement::from_row(row, n)).transpose()` is duplicated in `get_member` and `roster_rows`. `#[allow(clippy::too_many_arguments)]` decorates a 7-parameter function; clippy's default threshold fires only above 7 and the repo has no clippy.toml.
- Proposal: Extract `insert_member_row(conn, fleet, name, description, now)` and `insert_placement_row(conn, member_id, &NewPlacement, now)` used by both paths (`create_fleet` can build a `NewPlacement` from its arguments). Add `MemberKind::from_director_flag(bool)` or a small row mapper shared by both selects. Remove the `allow` (verify with `mise //cafleet:lint`).
- Spec impact: none — same SQL, same column values, same timestamps.
- Risk: low
- Estimated delta: -25 lines

### BRK-10: Remove the redundant RunnerExited exit-status re-validation
- Category: dead-code
- Location: cafleet/src/broker/runtime/events.rs:638-667
- Evidence: `persist_transition_in_transaction` first calls `validate_transition` (539), which for `RunnerExited` requires a `NativeExit` cause (`validate_cause` compatibility, 360-381), requires `exit_code`/`exit_signal` to be null-or-integer (398-406) and at least one integer present (416-423). The later block re-parses the same fields with errors "native exit evidence required", "integer required", "actual native status required" — all unreachable. None of these strings appear in SPEC.md, docs or tests (grep).
- Proposal: Replace the block with direct extraction (`cause.get("exit_code").and_then(Value::as_i64)` etc., with `expect` naming the `validate_cause` invariant) and keep the UPDATE.
- Spec impact: none — only unreachable error branches disappear.
- Risk: low
- Estimated delta: -22 lines

### BRK-11: One permission-event association predicate; stop fetching the event twice per reservation
- Category: duplication
- Location: cafleet/src/broker/runtime/permissions.rs:254-267, 342-370, 447-463
- Evidence: The predicate "event in this run, same interaction, `InteractionOpened`, `Inbox`, interaction classified `PermissionRequest` [and open]" is spelled out three times. `reserve_permission_response` checks owner kind/status/archive, epoch/Director and the open event (342-369), then `validate_current_capability` re-runs `get_event` and re-checks all of it (447-463). The precheck descriptions ("new response requires active running runner ownership", "current epoch and pinned Director required", "current open inbox permission event required") are asserted nowhere (grep).
- Proposal: Add `fn permission_event_matches(event, run_id, metadata, require_open) -> bool` and use it at the three sites with each site's existing description; pass the already-fetched `RuntimeEventView` into `validate_current_capability` instead of refetching.
- Spec impact: none — each site keeps its current error description, and the order of checks is preserved.
- Risk: low
- Estimated delta: -25 lines

### BRK-12: Share SHA-256 hex digest and canonical JSON helpers
- Category: duplication
- Location: cafleet/src/broker/runtime/evidence.rs:237-263; cafleet/src/broker/runtime/permissions.rs:135-145
- Evidence: `Sha256::digest(..).iter().map(|byte| format!("{byte:02x}")).collect()` appears in both files and ~20 more places crate-wide (grep `byte:02x`: input.rs, capture.rs, cli/member.rs, coding_agent/profiles*.rs, runtime/runner/*). `permission_response_digest` sorts keys via a one-level `BTreeMap` while `evidence::canonical` sorts recursively; with `serde_json` `preserve_order` enabled (Cargo.toml:22) both are needed only because no shared canonicalizer is exposed. `runtime/runner/input.rs:5394` has yet another `canonical_json`.
- Proposal: Add one `pub(crate) fn sha256_hex(bytes: &[u8]) -> String` and one `pub(crate) fn canonical_json(value: &Value) -> String` (crate-level util); use them in evidence.rs and permissions.rs (the permission digest object is flat, so recursive canonicalization yields identical bytes — confirmed by the existing `runtime_permission_digest_matches_independent_sorted_metadata_and_changes_per_field` test).
- Spec impact: none — digest bytes unchanged.
- Risk: low — the existing independent-digest tests guard equivalence.
- Estimated delta: -15 lines in-slice (≈ -60 repo-wide)

### BRK-13: Name repeated SQL fragments and bounds
- Category: duplication
- Location: cafleet/src/broker/runtime/events.rs:204, 458-464, 582-588, 622, 764-770, 776; cafleet/src/broker/runtime/inputs.rs:358; cafleet/src/broker/runtime/mod.rs:418
- Evidence: Payload bound `16 * 1024` twice in events.rs; `SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1` twice in events.rs (plus runner/recovery/exceptional.rs:43); interaction-by-episode lookup twice; unresolved-state list `state IN ('in_progress','accepted','uncertain')` in inputs.rs and runtime/mod.rs (and runner/input.rs:6353).
- Proposal: `const PAYLOAD_LIMIT`, `const UNRESOLVED_INPUT_STATES`, and small `fn latest_event_seq(conn, run_id)` / `fn interaction_id_for_episode(conn, run_id, episode)` helpers in events.rs (pub(crate) for the runner caller).
- Spec impact: none
- Risk: low
- Estimated delta: -10 lines

### BRK-14: Collapse thin counted/uncounted transition wrappers
- Category: over-abstraction
- Location: cafleet/src/broker/runtime/events.rs:513-520; cafleet/src/broker/runtime/effective_policy.rs:133-142
- Evidence: `persist_transition` and `persist_transition_with_effective_policy` only drop the `bool` from their `_counted` siblings. Production callers: `writer.rs:290/298` use the uncounted forms; `recovery/replay.rs:64/71` use the counted forms; tests use the uncounted forms.
- Proposal: Keep only the counted forms (renamed without the suffix) and destructure `(event, _)` at the two writer call sites; tests do the same or use a `#[cfg(test)]` wrapper in the fixtures module.
- Spec impact: none
- Risk: low
- Estimated delta: -12 lines

### BRK-15: Remove repeated event-kind SQL-domain tests
- Category: redundant-test
- Location: cafleet/src/db/mod.rs:940-953, 1320-1342, 1537-1567, 2082-2116
- Evidence: The 10-value `member_runtime_events.kind` domain is spelled out three times (`V6_EVENT_KINDS`, `WIDENED_KIND_CHECK`, the inline list in `event_kinds_and_archive_reasons_accept_the_specified_domains`) and accepted-by-SQL is asserted three times: `runtime_v6_event_kind_check_is_widened_to_the_specified_domain` inserts every `V6_EVENT_KINDS` value, `runtime_v6_runtime_event_kind_values_match_the_sql_domain` inserts every `RuntimeEventKind::VALUES` value after asserting `VALUES == V6_EVENT_KINDS`, and `event_kinds_and_archive_reasons_…` updates an event through every kind again.
- Proposal: Keep the widened-check test (accept + reject) and the `VALUES == V6_EVENT_KINDS` equality; delete the second insertion loop in the VALUES test and the kind loop in `event_kinds_and_archive_reasons_…` (keep its archive-reason half). Build `WIDENED_KIND_CHECK` from `V6_EVENT_KINDS` is optional — keeping the literal DDL as an oracle is fine.
- Spec impact: none — the same domain is still asserted once by SQL insertion and once against the Rust enum.
- Risk: low
- Estimated delta: -30 lines

### BRK-16: Trim per-row deadline checks in the listen snapshot (needs-verification)
- Category: other
- Location: cafleet/src/broker/messaging.rs:279-377
- Evidence: `select_listen_snapshot` calls `deadline.remaining_at(now())?` 13 times, including back-to-back checks with only in-memory work between them (336/338, 350/355, 363/365) and three per row (368, 370, 373). `listen.rs:1129-1148` (`row_collection_discards_an_expired_partial_snapshot_using_clock_boundary_checks`) requires ≥40 clock reads across 100 rows, so one check per row still satisfies it. Separately, `listen_snapshot_with_clock` installs its own progress-handler cancellation (215-243) parallel to `runtime::runner::lifecycle::CancellationScope`/`with_database_deadline` (lifecycle.rs:90-214).
- Proposal: Keep one check before each SQL step and one per row; drop the rest. Evaluate whether the listener can use `with_database_deadline` with an injectable clock instead of its own progress handler.
- Spec impact: none — expiry is still detected at every database boundary and per row.
- Risk: medium — needs-verification: run `mise //cafleet:test listen` and confirm the clock-boundary tests' intent (a design doc may require checks around every row read and decode).
- Estimated delta: -10 lines (≈ -25 more if the progress handler is unified)

### BRK-17: Replace the single-field `SchemaShape` newtype with a function
- Category: over-abstraction
- Location: cafleet/src/db/mod.rs:135-146, 223-232
- Evidence: `struct SchemaShape(Vec<SchemaTable>)` has one method (`matches`, an `all` loop) and one field access (`reference.0.iter().find(..)` at 225).
- Proposal: Return `Vec<SchemaTable>` from `read_schema`/`reference_schema` and inline `tables.iter().try_all(...)` (or a free `fn all_match`).
- Spec impact: none
- Risk: low
- Estimated delta: -8 lines

### BRK-18: Use the ordinary temp directory in the records.rs fixture (needs-verification)
- Category: other
- Location: cafleet/src/broker/records.rs:145-153
- Evidence: Only this fixture creates its database with `tempfile::Builder::new().prefix(".typed-records-").tempdir_in(env!("CARGO_MANIFEST_DIR"))`, writing scratch directories inside the package tree; every other broker test uses `tempfile::tempdir()` + `common::migrated_conn`. The three tests corrupt stored values and assert typed errors — nothing path-dependent.
- Proposal: Use `tempfile::tempdir()` like the rest.
- Spec impact: none
- Risk: low — needs-verification: confirm no filesystem-specific reason (e.g. a tmpfs quirk) with a test run.
- Estimated delta: -4 lines

### BRK-19: Stale "expected registry" wording in the migration rule
- Category: stale-doc
- Location: .claude/rules/database-migrations.md (paragraph "The embedded chain guard lives in `cafleet/src/db/mod.rs`. Update its expected registry when the bundled head changes…")
- Evidence: The guard `migration_chain_is_contiguous_from_one_with_a_single_baseline` (db/mod.rs:2417-2433) derives everything from `migration_chain()`; there is no hand-maintained expected registry to update. The only version-specific assertions are the per-migration `runtime_v5_*`/`runtime_v6_*` successor tests.
- Proposal: Rewrite to the current behavior: "The embedded chain guard in `cafleet/src/db/mod.rs` derives the registry from the embedded migrations and asserts contiguous numbering from one baseline; add a successor test for each new `V<N>__<slug>.sql`."
- Spec impact: none — project rule text only.
- Risk: low
- Estimated delta: -0 lines (rewrite)
