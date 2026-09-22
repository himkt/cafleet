# Reduce test duplication and CI overhead while preserving necessary scenarios

**Status**: Complete
**Progress**: 19/19 tasks complete
**Last Updated**: 2026-09-15

Final review approved commit `ade3ecd6` after two rounds. The E2E shutdown race was fixed and independently rechecked; all other recorded source hashes remain unchanged. PR: https://github.com/himkt/cafleet/pull/385. User-authorized publication completed; cache experimentation was skipped by explicit instruction. Design and execution evidence remain local and uncommitted.

## Overview

Reduce repeated installation, subprocess creation, successful notifications, and brittle documentation assertions while retaining the scenarios that protect CAFleet's behavior. Give each retained contract a clear test owner and measure execution separately from compilation and cache transfer. Keep the existing Cargo cache policy; the user removed the cache experiment from implementation scope on 2026-09-15.

## Success Criteria

- [x] Every removed or consolidated test maps to a retained assertion owner below, or to an explicitly identified documentation-review obligation; all necessary runtime scenarios remain in default PR validation.
- [x] Schema-only CLI fixtures run production migrations directly and install zero skill/preset trees; parser-only cases invoke the real Clap definitions without database setup.
- [x] The named duplicate notifications and fixture operations are eliminated, with before/after operation counts and separate suite timings recorded.
- [x] Documentation checks share one immutable source inventory and validate focused structural contracts; ordinary prose edits no longer fail word-distance or historical-vocabulary assertions.
- [x] Full Rust and frontend suites, frontend type/build checks, and existing lint checks pass. The four real timeout scenarios, twelve cleanup combinations, compensation stages, migration rollback, corruption, isolation, and backend distinctions remain exercised.
- [x] Repeated comparable local measurements show reduced test execution and fixture work, with sample ranges disclosed. Preserve the existing cache/profile policy; remote cache and cold-cache experiments are outside the final user-approved scope.

---

## Background

Three independent Astra/high auditors examined all **592 Rust tests**—352 inline tests and 240 integration tests—and **50 frontend cases**, including production code, fixtures, task dependencies, and CI configuration. Counts describe harness cases, not semantic coverage: one monitor cleanup function runs twelve combinations. Overlap discussions between auditors do not increase the unique total.

| Evidence | Scope |
|---|---|
| [Runtime audit](.audit/runtime-auditor.md) | All 352 inline tests and 96 runtime/CLI integration tests; exhaustive per-function disposition ledger |
| [Docs/setup audit](.audit/docs-setup-auditor.md) | All 126 documentation, setup, doctor, and global CLI tests; shared-fixture and unit-owner analysis |
| [Frontend/CI audit](.audit/frontend-ci-auditor.md) | 50 frontend cases, 18 remaining Rust integration cases, build/CI surfaces, coordinated local measurements |
| [Successful CI baseline](.audit/ci-successful-baseline.md) | Complete successful run 34959515487 at commit `7d35842aa56bc63ff7a1dc55b28fdcbda0ca8cd6` |
| [Earlier failed baseline](.audit/ci-observed-baseline.md) | Incomplete run 34958976900, stopped at documentation validation |
| [Director handoff](.audit/drafting-handoff.md) | Scope defaults and reconciliation requirements |

The clarification round asked about optimization scope, performance targets, and PR coverage. No user preferences were supplied. The Director resolved these as disclosed defaults: include fixture/build/cache improvements; use measured baseline comparisons without a promised duration; preserve necessary scenarios on every PR. These defaults permit drafting and remain subject to document review and user approval.

### Observed cost

The [successful CI run](https://github.com/himkt/cafleet/actions/runs/34959515487) took **213 seconds** for the test job.

| Layer | Observed duration | Meaning |
|---|---:|---|
| Tool setup | 13 s | mise/environment setup |
| Cargo cache restore | 113 s | Exact-key hit, 3,801,520,083 bytes, approximately 3.80 GB |
| Frontend dependency install | 4 s | Frozen lockfile |
| Frontend test step | 2 s | Runner overhead plus 50 cases |
| Rust compilation/linking | 40.48 s | Cargo's reported profile/build phase despite the cache hit |
| Rust harness execution, summed | 30.83 s | Includes per-test fixtures and subprocess work |
| Residual inside Rust task | About 2.91 s | Frontend build plus command/rounding overhead; derived, not an isolated build measurement |

The Rust task totals 74.22 seconds; its subrows above are components, not additional job costs. Setup/checkout/post-job overhead accounts for the remaining job time. Parallel lint-job time is separate runner consumption.

| Rust harness | Cases | Successful CI execution |
|---|---:|---:|
| Library | 352 | 4.81 s |
| CLI compensation | 14 | 0.39 s |
| CLI fleet | 15 | 6.59 s |
| CLI global | 21 | 0.42 s |
| CLI member | 35 | 1.90 s |
| CLI message | 22 | 4.04 s |
| CLI server | 2 | 0.00 s rounded |
| CLI setup/doctor | 40 | 0.41 s |
| Documentation sync | 65 | 7.66 s |
| End-to-end lifecycle | 1 | 4.24 s |
| Monitor uniqueness | 10 | 0.08 s |
| WebUI routes | 15 | 0.29 s |

The earlier failure involved an illustrative brace token. Its documentation suite took 6.90 seconds, while cache restore took 101 seconds and compilation 43.09 seconds. It stopped before later harnesses; those missing durations are unknown. The notation fix passed the complete subsequent CI run. This incident motivates checking the documentation tests' signal, while the full baseline identifies the larger job costs.

Local macOS observations differ substantially: Vitest took 158 ms, E2E 13.67 seconds, WebUI routes 1.67 seconds, global CLI 11.03 seconds, setup/doctor 9.75 seconds, and documentation 5.15 seconds. They used existing build outputs. They establish local behavior and variability; **no before/after optimization savings have been measured**.

---

## Specification

### 1. Scope and reduction rules

Implement the concrete changes in sections 2–8. Retain other audited scenarios, including cheap pure tests, unless a removal is explicitly mapped here. The exhaustive audit ledgers supply supporting source locations and assertion inventories; their optional suggestions are not additional mandatory changes.

For each consolidation, first make the surviving case assert the donor's meaningful outputs, state, and failure stage. Run it successfully, then remove the donor and its unused helpers/imports. Use names describing the resulting scenario. Keep repository documentation and task references aligned when deleting a harness or renaming a fixture; historical donor names belong in this design and audit records.

Sharing a fixture is appropriate within a coherent lifecycle. Each independent test retains its own mutable database, environment, shim state, and child processes. Preserve distinct APIs even where their fixtures overlap—for example inbox/sent/timeline queries, text/JSON output, member uniqueness/loop ownership, and skip/attempted notification failure.

Production CLI/API behavior, delay values, compatibility handling, and migration contents stay within their current contracts. A test-only environment variable that bypasses delivery delays would change the exercised path; remove unnecessary deliveries through fixture seeding instead. Keep default PR coverage enabled for every necessary scenario. Scheduled-only suites, new browser infrastructure, release workflow changes, and Docs workflow optimization are outside this plan.

### 2. Fixture ownership

`cafleet/tests/common/mod.rs::Cli::migrate` currently launches `setup`, installs assets for three agents, deletes the recorded rows and default installed trees, and leaves a migrated database. `ready` then inserts a current Claude record. Replace this install/remove cycle with explicit fixture levels:

| Fixture | Construction | Consumers |
|---|---|---|
| Isolated command environment | `Cli::new` supplies a temporary HOME, explicit PATH/presence variables and shims; no database | Executable parser/help smokes and environment-derived server help |
| Migrated database | `db::connect` using the fixture URL, then `db::migrate_to_head`; assert returned head | Schema, doctor, guard, corruption, and ordinary command fixtures |
| Guard-ready database | Migrated database plus the minimum current asset-install record at its resolved path | Commands whose subject is independent of installation |
| Seeded roster | Guard-ready database plus production broker fleet/member APIs and deterministic placement callback | Lookup, scan, capture, messaging, deletion fixtures |
| Real CLI lifecycle | Explicit successful `setup`, `fleet create`, or `member create` command | Installation, bootstrap, argument handoff, spawn, compensation, and E2E owners |

Keep `migrate` and `ready` semantics narrowly named and update every consumer that needs real installation to call an explicit installation helper. E2E must explicitly invoke successful real setup rather than losing it when `ready` changes. Setup/doctor's connection-reuse module test keeps its real installation assertions.

Seed fleet/member rows through public broker APIs using current migrations. Preserve file-backed databases for subprocess access, connection ownership, transactions across connections, and historical migrations. An in-memory fixture is appropriate only for a single-connection test and uses `db::connect("sqlite:///:memory:")` so connection PRAGMAs remain active. This phase changes shared fixtures and affected tests; wholesale conversion of all unit fixtures is deferred.

Return actual allocated IDs. Assign distinct pane IDs to the Director, monitor, and workers in seeded rosters; make scan shims return distinct pane content. Assert fixture setup success and required records immediately. Treat a missing shim log as the documented zero-call state; propagate other read errors. Read the relevant log suffix once per action. Exact argument boundaries remain owned by FakeRunner adapter tests because the shell log flattens arguments.

The docs/setup audit identifies 27 migration-fixture invocations in setup/doctor/global alone, implying 27 setup subprocesses and approximately 2,727 skill/preset writes removed. This is a source-derived operation count, not a timing result. Shared-fixture savings elsewhere must be counted once across all consumers.

### 3. Parser, setup, doctor, and global CLI consolidation

Use `CliArgs::try_parse_from` within the existing private CLI module test boundary. Add focused tables using the real command definitions; assert parsed variants/values or native Clap error kind and offending input. Preserve one executable unknown-argument exit-2 smoke, version/help bypass, and environment-derived server help. Keep file/stdin reads, blank-body validation after guards, and application errors at their real boundaries.

Move global option-placement/subject/body-option tables, setup space/repeated/mixed selector syntax, invalid setup values, fleet required options/subject grammar, and member body/role grammar into these parser tests. The current four global parser functions alone execute 29 subprocesses. Preserve valid vectors including order/duplicates; selected installation separately checks deduplication and fixed install order. Remove setup's exact help-description sentence assertion; option/metavar and successful help remain covered.

#### Setup ownership

| Retained owner in `cli_setup_doctor.rs` | Donors / removed duplicate work | Assertions retained before removal |
|---|---|---|
| `plain_setup_installs_and_records_all_three_agents_on_a_fresh_database` | All-agent selector install; stale-directory cleanup; rows-at-old-path reinstall fixture | Actual head, both skills for all agents, one complete installed path/content comparison including nested runtime references and presets, resolved identity/version rows; second setup removes stale entries and replaces content, preserves same-key row cardinality and unrelated/superseded rows |
| `selector_setup_installs_at_the_env_resolved_paths` | `selector_setup_installs_only_the_named_agents`; untargeted-invalid-variable fixture | Select only Codex at custom skill/preset paths; exact selected row and installed bytes; untargeted trees absent; invalid untargeted variable is not evaluated |
| `selector_setup_deduplicates_and_installs_in_the_fixed_order` | Successful mixed/repeated selector process permutations | One mixed selector list with duplicates/reversed order; one install per selected target in fixed order and correct rows |
| `opencode_skills_stay_at_the_fixed_discovery_path_when_the_variable_is_set` | Retain independently | Fixed skills discovery path versus relocated preset and identity record |
| `plain_setup_fails_the_assets_half_on_an_invalid_config_path_variable` | `an_invalid_variable_fails_the_assets_half_with_the_pinned_error` | Invalid Codex path: schema succeeds, earlier Claude installation/row survives, Codex error is preserved, later OpenCode remains untouched |
| `setup_refuses_an_unversioned_database_with_existing_tables` | Retain independently | Failure plus foreign data/table preservation; no migration ledger or install record introduced |

The stale-directory cleanup is current production behavior and stays tested. The new installation owner compares the embedded package once, rather than rebuilding an installation matrix around parser permutations. Derive current head through `head_version()`; retain literal historical versions in fixtures whose physical layout matters.

#### Doctor and guard ownership

| Retained family / exact owner | Consolidation | Required distinctions |
|---|---|---|
| Healthy doctor text | Retain `doctor_reports_a_healthy_environment_with_no_issues` | Three sections, tmux context, head, recorded current assets, zero issues, exit 0 |
| Mixed doctor text/JSON | Combine `doctor_setup_cells_cover_ok_stale_and_not_installed`, `doctor_json_mirrors_the_report`, `doctor_lists_superseded_rows_as_footnotes`, `doctor_frames_the_table_by_display_width` | Unicode custom current Claude path, stale Codex, missing OpenCode, sorted superseded rows, remedies, path/source, display alignment, singular issue footer and both output modes |
| Failure doctor text/JSON | Keep `doctor_json_null_contracts_on_failures`; absorb multiplexer-failure and invalid-resolution text assertions | Missing mux/database plus invalid path, all agent rows and sections, three issues, plural footer, explicit keys then null/type/value assertions |
| Open failure | Keep `step6_doctor_open_and_path_failures_still_render_all_sections_in_text_and_json`; remove `doctor_reports_a_database_connection_failure` | Directory-as-database differs from absence; independent path failure, both formats, complete report, exit 1 |
| Legacy doctor | Share fixture for `doctor_completes_the_report_against_a_pre_v6_database` and its JSON equivalent | Actual incompatible asset table, skipped asset query, not-installed rows and nulls, version/remedy; retain both process output modes |
| Non-head corruption | Keep both rows of `step6_doctor_non_head_states_do_not_query_malformed_asset_records`; remove generic behind/newer doctor fixtures after transferring remedies/version assertions | Behind and ahead must both skip malformed asset records; at-head corruption remains an error |
| Schema classification | Add empty-ledger with/without foreign tables to `diagnosis::schema_states_preserve_versions_and_sql_failures`; replace redundant empty-ledger doctor processes | Missing, unversioned, behind, head, ahead and SQL failure remain distinct; retain representative missing/unversioned CLI reports |
| Informational asset absence | Share fixture for dropped-at-head and empty-existing-table doctor inputs | Both supported inputs retain zero-install informational behavior; corruption remains separate |
| Guard group wiring | Keep `the_schema_guard_blocks_every_guarded_group`; remove `the_schema_guard_reports_an_outdated_database` and `a_fleet_scoped_command_against_a_pre_v6_database_names_setup_not_sqlite` | Fleet/member/message/monitor/scan exit before handler effects with setup remedy; retain separately bounded server-startup rejection and server asset-guard exemption |
| Successful asset guard | Merge `an_at_head_database_passes_the_schema_guard` and superseded-row/no-row success cases into `the_guard_ignores_superseded_rows_at_other_paths` | Current resolved row passes; same-agent superseded row and agent with only another-path row do not interfere; valid empty fleet output |
| Relocated asset guard | One fixture for both `a_config_location_variable_rekeys_the_guard_to_the_resolved_path` and `a_config_location_variable_supersedes_the_default_path_row` | Override without row fails; stale override fails differently; current override succeeds |

Keep foreign/ahead schema-guard remedies, invalid-path precedence, stale-agent ordering, missing/empty database handling, and guard wiring across all command groups. These checks use cheaper fixtures; the plan does not replace them solely with diagnosis tests. Doctor reports recorded installation state, so seeded rows establish that contract without claiming filesystem health. Preserve explicit JSON key order wherever an existing public contract pins it; require key presence before null assertions in every consolidated JSON fixture.

### 4. Runtime duplicate transfers

Paths below are under `cafleet/`. Transfer all named observations before deleting donor functions. Other compensation and uniqueness cases keep their own failure stages.

| Donor | Surviving owner | Preserved scenario / avoided work |
|---|---|---|
| `tests/cli_fleet.rs::fleet_create_reports_the_compact_line_with_director_and_monitor` | `fleet_create_spawns_the_monitor_pane_with_identity_and_model` | Exact compact stdout plus real bootstrap identity/model/argv; one fewer successful create |
| `tests/cli_fleet.rs::fleet_create_substitution_failure_rolls_back_everything` | `tests/cli_compensation.rs::fleet_placeholder_failure_rolls_back_without_creating_or_killing_panes` | Exact unknown-placeholder error, empty bootstrap tables, zero split/kill |
| `tests/cli_member.rs::member_create_unknown_placeholder_exits_2_and_leaves_no_orphan` | `tests/cli_compensation.rs::member_placeholder_failure_keeps_usage_exit_and_deregisters_without_a_pane` | Exact diagnostic, exit 2, one deregistration, no owned pane |
| `tests/cli_member.rs::member_create_rejects_any_role_value_but_monitor` | `src/cli/member.rs::create_rejects_any_other_role_value` plus executable parser smoke | Same real Clap invalid-role branch |
| `tests/cli_member.rs::member_ping_dispatches_the_subject_only_poll_keystroke` and pending-capture fixture | `member_ping_skips_a_pending_placement_and_exits_zero` | Pending capture exit 1 versus ping exit 0/no send; pending JSON contains `member_id`, present null `pane_id` and `skipped: true`; placed JSON contains the same member, allocated pane and `skipped: false`; assert exit 0 for both JSON calls and exact subject-only poll after placement; one fewer successful ping |
| `tests/cli_message.rs::send_prints_the_header_and_the_compact_echo` | `send_truncates_the_echo_but_never_the_persisted_text` | Header, compact echo, Unicode truncation boundary, complete persisted body; one fewer send |
| `tests/cli_message.rs::show_json_pins_the_typed_column_envelope` | Existing show step in `json_is_untruncated_on_every_message_subcommand` | Actual show call, raw envelope/key order, types/null origin and untruncated body; one fewer send fixture |
| `tests/cli_message.rs::ack_guards_are_existence_and_state_only` | `poll_and_ack_walk_the_delivery_lifecycle_with_subject_ids_only` | Missing ACK, first text ACK, second ACK failure, empty poll; separate JSON lifecycle retains first JSON ACK |
| Two notifying sends in `integrity_invalid_stored_message_enum_exits_one_without_success_output_or_panic` | Same test, broker-seeded messages | Both corrupt fields × both output modes, exit 1, empty success stdout, no panic |
| `src/broker/messaging.rs::send_message_truncates_the_preview_but_persists_full_text` | `send_message_persists_the_full_row_and_notifies` | One exact stored row and one correctly routed/truncated preview |
| `src/broker/messaging.rs::send_message_to_self_skips_the_preview` and `send_message_to_a_paneless_recipient_skips_the_preview` | `a_failing_notifier_cannot_fail_self_send_or_no_pane_skips` | Both durable messages/ownership, Skipped outcomes, false sent flag, zero notifier calls even when notifier would fail |
| `src/broker/members.rs::register_member_writes_no_monitor_config_row` | `src/db/mod.rs::baseline_creates_exactly_the_head_tables` | Existing current-schema table-set owner already covers the absence; remove the historical registration-only assertion |
| `src/broker/asset_installs.rs::the_same_agent_records_distinct_rows_at_distinct_paths` and `record_asset_install_upserts_on_the_composite_key` | `record_and_list_orders_rows_by_coding_agent_then_path` | Exact sorted composite-key rows, updating one preserves the other and changes version/timestamp |
| `src/broker/members.rs::get_member_names_batches_and_includes_deregistered` | `names_deduplicate_ids_across_batch_boundaries` | Add Director and deregistered-name assertions; retain 0/1/500/501/1,001 probes, duplicate IDs and misses |

Further bounded fixture consolidation in `cli_member.rs` combines plain/shell prompt dispatch into one seeded worker and text/JSON capture into one fixture. Both successful command invocations remain. Combine scan text ordering, JSON field order and custom line depth into the current ordering owner using distinct pane bodies; preserve default/custom argv and zero-depth rejection. Share pending/placementless scan setup; retain all-failed capture and Director-only scan cases. Transfer unknown/deleted fleet diagnostics into `step8_scan_live_fleet_guard_precedes_invalid_mux_resolution`, retaining actual soft deletion and capture's opposite guard precedence.

Keep monitor-role recovery in `member_create_without_a_monitor_hits_the_monitor_first_guard`: absent monitor blocks worker; monitor creation records kind; duplicate rejects without row/placement mutation or split/kill; deregistration allows a replacement. Move `member_create_role_monitor_registers_the_monitor_kind`, `member_create_role_monitor_twice_hits_the_one_per_fleet_guard`, and `monitor_uniqueness::duplicate_cli_guard_keeps_error_and_performs_no_registration_or_pane_creation` into that lifecycle. Preserve the complete database snapshot from the uniqueness donor. All other uniqueness tests stay.

Preserve real CLI creation for stdin/file ingress, backend availability, pre-side-effect validation, split failures/retry, and compensation. Keep unicast attempted failure, absent/ambiguous mux, self/no-pane skip, and broadcast partial preview failure distinct. Combining text/JSON partial-failure fixtures still executes both modes and checks each newly persisted ID and output channel.

The first ping/send/show/ACK transfers remove four successful tmux deliveries; corruption seeding removes two more. Current production requests 0.1-second settle plus 1-second submit delay per successful delivery: **6.6 seconds of aggregate configured sleep work**, not guaranteed suite wall time. Count each eliminated delivery once.

### 5. Documentation contracts

Replace the 65 term-oriented `docs_sync.rs` functions with focused contract families and shared helpers. The [65-test ledger](.audit/docs-setup-auditor.md#exhaustive-disposition-ledger-docs_syncrs-65-tests) identifies every current donor. The families below are the intended automated owners; the final harness count follows useful diagnostics rather than a quota.

| New focused owner | Current families consolidated | Automated contract |
|---|---|---|
| Required-read links | Specialized roster, every-role row-one, startup, Director required reads, workflow path-normalization links | Applicable roles exist; each role's own Required reading section contains its expected backend/BASE/coordination links, with backend reference in row one; targets resolve |
| Backend binding tables | Full vocabulary, complete lookup sections, role-default catalog membership | Three supported backend sections; seven runtime and two default bindings exactly once/nonempty in their respective tables; defaults name local model tokens/aliases |
| Backend worked examples | Capture cues and three backend launch/stop contracts | Launch form in each backend's `Worked resolution`; stop form in its `Runtime bindings` `bg_stop` row; four state keys in its `Pane-state capture cues`. Preserve Codex managed-session execution, Claude `TaskStop`, and OpenCode background launch/recorded-process stop |
| Workflow routes and receiver examples | Finalization, ready/markers, resume/review-only, re-review, JSON payload retrieval | Exact route literals and complete poll/show JSON plus ACK examples in their authoritative sections; owner links from consumers |
| Spawn/manual lifecycle examples | Director prompt template and public installed-monitor prompt | Four identity labels/placeholders, absolute installed role/reference paths, monitor-file bootstrap and consistent identifiers in the selected example |
| Message and invocation examples | Broadcast, Send/no-resend, shell dispatch, one-shot isolation and permission diagnostic | Real command/flag tokens and owner links in selected examples; ordered command sequences for success/failure branches where explicitly represented as commands |
| Monitor/supervision owner references | Monitor role, shared summaries, public monitoring/cadence/storage references | Valid links to normative monitor/backend/supervision owners; current bootstrap and recovery command examples remain accessible |
| Public configuration/navigation | Quickstart, model navigation, concept navigation and spec anchors | Setup/configuration tokens; referenced navigation pages exist with nonempty unique labels; locally referenced anchors resolve |
| Local source references | Existing inline-code path scan plus required links above | Existing concrete repository path references resolve; required Markdown owner links resolve relative to the logical source path and their anchors exist |
| Operational placeholder syntax | Global brace sweep replaced by selected binding/template checks | Known binding keys and four spawn placeholders in the operational template; production `spawn_prompt` tests continue to own substitution/escaped-brace grammar |

Create one lazily initialized immutable inventory for `skills/**/*.md`, following the installed runtime alias, plus the existing explicit `.claude/skills/clean-docs` role and `.claude/skills/skill-author`/bash-rule consumers. Keep logical paths for relative-link validation; canonical paths may deduplicate content reads. Fail on required file/read/link errors, and report file, line, section/backend and expected target. Cache retained regular expressions once. Scope row lookup to its own section so a later unrelated row cannot satisfy row-one.

Validate these backend checks against the unchanged normative owner file before removing donor assertions. Worked launch examples need only their documented launch/confirmation behavior; the binding rows separately own how to stop each backend's process or session.

Use the existing section/table helpers, tightened for these known structures. Validate the explicit required-link set and the existing concrete inline-code paths; a universal Markdown/link parser is outside scope. Existing Rspress dead-link validation remains responsible for the full published site. For required anchors, support the repository's existing simple headings and explicit IDs; unsupported required syntax fails with a specific diagnostic so the fixture/owner can be updated deliberately. Installed aliases are validated at their logical source location, not rescued by a second unrelated repository-root lookup.

Prose examples such as numbered design-document paths are valid documentation. The operational placeholder owner checks bindings and the selected spawn-template region, rather than interpreting every brace in narrative text as a spawn variable. Keep unknown-token and malformed-brace rejection in the real spawn formatter tests. This deliberately ends automated typo detection for arbitrary narrative brace tokens; document review checks actual usage there. Do not expand a global exception list each time a new illustration appears.

Remove broad term lists, word-distance regexes, chapter/title/order snapshots, and historical vocabulary sweeps, including `fixed_ping_surfaces_carry_no_nudge_vocabulary`, `the_readme_and_webui_api_stay_free_of_internal_monitor_state`, `REMOVED_VOCABULARY`, `OLD_CLI_SURFACE`, and recurring-poll prose blacklists. Retained exact route/flag/payload tokens are interface checks, not spelling preferences.

**Semantic coverage changes explicitly:** structural checks establish reachability and syntax; they cannot prove agent obedience, correct conditional approval, no-resend reasoning, quiet-state decisions, authority boundaries, or safe shutdown. Review changes to their normative owners against those behaviors. The donor families for Director diagnostics/no-Tester routing/local approval, public non-authentication explanation, BASE ordering, supervision uncertainty/closure and async-turn policy become semantic review obligations. Keep the normative instructions and worked examples intact. Their removal from regex testing is justified by weak signal and false positives, not by claiming link validation is equivalent semantic coverage.

Add a compact checker-fixture table for missing target/anchor, misplaced prerequisite row, missing/duplicate binding, unknown operational placeholder, and valid illustrative prose. These fixtures verify the new checkers' relevant failure modes; they do not introduce a general documentation-validation platform.

### 6. Small data fixtures, frontend, and HTTP

| Surface | Exact change | Retained owner and boundary |
|---|---|---|
| `broker/queries.rs::timeline_filters_summaries_before_cap_and_keeps_partial_broadcast_as_rows` | Use one unicast plus two broadcast deliveries, limits 2/3; remove 198 single-message sends | Filter summaries before LIMIT, partial broadcast membership, full three-delivery count, separate summary lookup. This API takes a limit parameter |
| `webui_routes.rs::the_timeline_is_hard_capped_at_200` | Insert 201 valid rows with deterministic timestamps using one prepared statement/transaction | Real public cap 200, newest/last retained row and 201st exclusion. The separate frontend history cap stays |
| Frontend history | Remove only 1205 from `selects up to 200 of %i deliveries and reports actual truncation` | Keep 0/200/201, summary filtering and order/immutability |
| Frontend timeline | Replace ACK-count parameter rows 0/1/2 with mixed row 1 | `groupMessages` returns members rather than computing counts; assert exact mixed-status rows. HTTP broadcast test retains real 0→1→2 ACK transitions |
| Frontend resource | Keep string/null non-Error rejection cases; remove undefined/object repeats | Useful normalized Error and retry; retain all abort, obsolete success/error/finally, queued refresh, independent resource and snapshot/listener cases |
| HTTP header parsing | Keep exhaustive grammar in `the_fleet_header_dependency_resolves_in_the_pinned_order`; remove empty/abc repeats only from PATCH monitor and wake | Each changed handler keeps missing-header plus valid dispatch; retain its own body validation, liveness/lookup order and persisted outcomes |
| HTTP successful broadcast | Move broadcast response assertions from `post_send_handles_unicast_broadcast_and_the_error_surfaces` to `timeline_broadcast_excludes_summary_through_two_delivery_ack_transitions` | Actual POST, summary response and immutable summary, origin/recipient names and three ACK states |
| HTTP missing-name bridge | Remove `integrity_missing_recipient_name_returns_500_without_a_panicked_task_detail` after strengthening direct presenter owner | `webui::integrity_regressions::required_message_names_return_errors_without_unwinding_in_the_presenter` keeps both missing identities with useful errors/no panic; missing-sender HTTP case keeps the 500 bridge |
| Server harness | Move both `cli_server.rs` tests into global help family, delete empty target and references | Default and environment-derived host/port help through separate processes; retains two scenarios and removes one integration link target |

Frontend case count becomes **45** from these five removed parameter cases; all five families remain. Preserve all eighteen client cases, including each independently wired endpoint, concurrent fleet identity, signal propagation, error precedence and cancellation identity. Their warm runner is already cheap, so this is maintenance simplification rather than a significant speed claim.

HTTP corruption fixtures stay isolated, with message kind/status, member status and required-name failures distinguished. Preserve wire key-order checks where currently pinned. The entire HTTP suite took 0.29 seconds in successful CI; its operation savings are bounded accordingly.

POST handlers currently construct `RuntimeNotifier` even when fixture setup uses `NullNotifier`. Preserve existing POST integration behavior and explicit persistence-after-preview-failure assertions. A production app-construction notifier injection is deferred: it adds a seam beyond the demonstrated runtime opportunity. Placementless seeding may be used only for cases whose subject does not include attempted notification, with existing attempted-failure owners retained.

### 7. E2E and scenarios kept intact

Keep `end_to_end_lifecycle_with_one_monitor_tick` as one full binary scenario: real setup/bootstrap/member creation, send/poll/ACK/empty poll, real monitor startup, separate-process second-loop rejection, actual wake to the monitor, and persisted wake time. Explicitly assert process exit codes for poll/ACK steps.

Replace its fixed one-second and two-second sleeps with bounded observation. After spawning, poll the test database for the child's owned live runtime slot; fail if the child exits or a 10-second deadline expires. Then spawn the second monitor under its own child guard, require exit within 10 seconds, and assert its rejection. This bounds a regression that incorrectly accepts the second loop. Await a nonempty `last_wake_at` and the expected monitor-directed wake in the shim log under a fresh 10-second deadline, polling at 20 ms. Keep the production tick/interval unchanged. Scoped child guards terminate and reap both test-owned processes on success or assertion failure; collect stdout/stderr and the final log for diagnostics. Assert startup/wake output and that the Director was not targeted. Distinct fixture pane IDs make the destination meaningful.

Those deadlines are upper bounds, not success-path sleeps or weakened assertions. The fixed sleeps total three seconds; possible E2E savings of roughly 1–3 seconds remain an estimate. The separate binary claim remains valuable alongside direct loop ownership tests.

| Protected family | Required scenarios retained |
|---|---|
| `cli_compensation.rs` | All 14 failure stages: insert/substitution/placement/vanished result/commit, pane and deregistration cleanup failures, known/unknown pane ownership, both herdr creation paths; original errors and database snapshots |
| Setup/migration/uniqueness | v5 multi-migration rollback, populated v7 upgrade, unique predicate boundaries, unrelated SQL cause, post-diagnosis interference/rediagnosis outcomes, stale prechecks across two file-backed connections |
| Monitor scheduler/cleanup | Fake-time interval baselines and changes, zero interval, forced wake/skip/retry, lost claim, dead/absent/pending monitor, wake ledger; all six primary × two cleanup outcomes with signal ownership |
| System runner | All four one-second real OS timeout cases: sleeping child, busy streams, exited child/open descendant pipes, closed pipes/live child; large success/failure streams and cleanup/error provenance |
| Multiplexer/coding-agent adapters | Both tmux/herdr argv, exact token boundaries, ordering/delay events, every preview failure position, pane ownership and layout; all three coding-agent model/effort/path/preset distinctions |
| Queries/presentation | Fleet isolation and owner lookup, summary filtering, ordering/ties, stored enums, required names, Unicode/ANSI/CR transformations, truncation and wire shapes |

Keep the current timeout deadlines and cheap FakeRunner/pure formatter matrices. Shared capture-table extraction, broad broker/monitor lifecycle rewrites, and constant-table regrouping are deferred. Their additional complexity has no established job-time benefit after the selected transfers.

Existing gaps—simultaneous overlapping writers, mid-broadcast SQL failure, rendered React/StrictMode lifecycle, broader HTTP cross-fleet wiring, actual socket-serving smoke and additional installer write failures—remain recorded in the audits. This change adds only assertion transfers needed to make its reductions safe; it does not claim those pre-existing gaps are now covered.

### 8. Cache and build experiment

Keep the real frontend build, type checking and embedded assets. `cafleet/build.rs` requires `webui-dist/index.html`; empty test assets would invalidate the shipped-binary coverage. The task residual of about 2.91 seconds supplies little evidence for an extra frontend artifact job, so retain the current lint/test job arrangement and admin build dependency in this phase.

Measure these cache policies using the same source revision, runner image and toolchain:

| Candidate | Contents / settings |
|---|---|
| Existing baseline | Registry/git plus whole `cafleet/target` archive and default test profile |
| Dependency download cache | Registry/git only, rebuild all compiled output |
| Compact compiled cache | Registry/git and reusable target artifacts with `target/debug/incremental` excluded; CI test `debug = 1`, `incremental = false` |

For the compact candidate, use command-scoped `CARGO_PROFILE_TEST_DEBUG=1` and `CARGO_PROFILE_TEST_INCREMENTAL=false`; preserve debug assertions, overflow checking and useful panic locations. Separate test and lint caches. Cache identity includes OS/architecture, installed Rust version, profile/configuration version and Cargo.lock hash; restore prefixes preserve compatibility dimensions. Save under a revision-specific suffix so a new successful revision can refresh reusable artifacts, and bound retention rather than accumulating every revision indefinitely. An exact match must continue to skip redundant saving.

Freeze revision `R` as the audit commit `7d35842aa56bc63ff7a1dc55b28fdcbda0ca8cd6`. Freeze one concrete source-change patch `P`: the section 4 ping reduction that removes `member_ping_dispatches_the_subject_only_poll_keystroke` and adds the explicit JSON key/value and exit-code assertions to `member_ping_skips_a_pending_placement_and_exits_zero`. Keep the pending-capture transfer and every other reduction outside `P`. Save the exact patch and its SHA-256 before timing; apply those identical bytes to `R` for every candidate. Cache-policy/profile overlays are the only other candidate-specific changes. This isolates a real test-source rebuild from all other suite reductions.

For each candidate, measure all three cells below against the existing policy in **at least three paired samples per cell**. Each pair runs sequentially on matching fresh runner images; alternate baseline/candidate order between pairs. Use isolated cache namespaces per pair, seed identical source states before timing, and reset the namespace before each fresh sample. Preparation builds are recorded separately and excluded from timed jobs.

| Sampling cell | State prepared before timing | Timed revision and cache path |
|---|---|---|
| Exact repeat | Successful candidate-specific cache created from `R` | `R`, exact restore, no cache save |
| Revised source | Only the compatible cache from `R`; no destination key for `R+P` | `R+P`, prefix restore, source rebuild and save of the new revision key |
| Cold control | Empty Cargo download/build cache namespace | `R+P`, cache miss, download/rebuild and initial save under that candidate's policy |

Capture cache bytes, restore/save, dependency compilation, project compilation/linking, build-script execution, frontend build, each harness and complete job duration. Report each candidate/cell's median and range separately. An actual exact hit, prefix hit or miss and the intended save behavior must be confirmed from logs; a different path invalidates the sample. Tool setup and frontend dependency-cache state stay equal within each pair.

Use **revised-source complete-job time** as the sole ranking cell, representing a new PR revision; other cells are adoption gates, not pooled observations. For pair `i`, let `B_i` be baseline seconds, `C_i` candidate seconds and `d_i = B_i - C_i`. Define `D = median(d_i)` and `N = median(abs(d_i - D))` separately in each cell. A candidate qualifies only when:

1. Every measured job passes the complete functional validation.
2. Revised-source savings are positive in every pair and `D >= max(5 seconds, 2 × N)`.
3. Exact-repeat and cold-control medians do not regress (`D >= 0` in each), and no individual paired regression exceeds `max(5 seconds, 5% of B_i)`.

Among qualifying candidates, choose the largest revised-source `D`; break an exact tie by smaller restored archive bytes. These numerical thresholds are conservative experimental decision rules, not promised savings or statistical confidence claims. If no candidate qualifies, retain the existing policy and report the experiment as inconclusive. Cold-cache correctness or performance failure prevents adoption even when warm results improve.

Before adoption, repeat the same three cells with at least three baseline/candidate pairs on final reduced-source benchmark snapshots. Prepare `F0` with the standalone successful-ping donor retained using current fixture APIs and `F1` with only that donor removed; the stronger retained owner and every other reduction are identical in both. Exact-repeat uses `F0`, revised-source restores `F0` for `F1`, and cold uses `F1`. Record both tree/revision identities and the exact patch digest; every policy in this confirmation uses the same pair. Apply the same qualification rules. This confirms that the policy remains beneficial after the full fixture/test reduction rather than extrapolating solely from the audited suite.

The 113-second restore is an opportunity bound. Removing the archive can increase rebuild time; measure the net result. Profile/cache changes are a separately reviewable stage so fixture savings are not credited twice. Build-script freshness changes and reusable admin artifacts remain deferred unless this experiment identifies a material cost and supplies a narrowly scoped follow-up design.

### 9. Validation and acceptance

Record the source revision, runner/toolchain, test thread count, cache state and machine load for every timed sample. Use the unchanged audit revision as the functional and timing baseline; run comparable before/after samples under the same environment, sequentially without competing audit jobs. Report median and range from at least three successful runs; separate compilation and execution and disclose missing measurements.

During implementation, maintain a reduction ledger containing old test/case, surviving file/function, transferred assertions, removed operations, and validation result. The tables above define its starting entries. Keep this evidence with the implementation review artifacts; historical removal narratives stay out of source comments and user-facing docs.

Run focused affected families after each stage and the complete suite after shared-fixture changes and at the end. The final validation uses the existing `mise //admin:test`, `mise //cafleet:test`, `mise //admin:lint`, and `mise //cafleet:lint` paths, including the dependent real frontend build. Count executed cases and disclose the expected frontend 50→45 and server harness relocation. Inspect ignored/filtered/default-run membership so a faster command cannot hide necessary cases.

Use narrow fault checks for changed test infrastructure: malformed documentation fixtures must fail with relevant diagnostics; a seeded mixed roster must distinguish the monitor's pane from workers; a wrong cap/filter/order must fail the retained query and HTTP boundaries. Verify the rewritten E2E wait fails within its deadline when readiness or wake never occurs and reaps its child. These checks target the risk introduced by the reduction, not a new broad mutation-testing platform.

Reject a consolidation when the proposed owner cannot assert the donor's meaningful condition without additional independent setup; retain the donor in that case and record the reason. Reject a fixture change that masks required errors, changes the command path under test, loses isolation, or weakens a failure stage. Revert a cache/profile candidate whose net job time regresses or whose clean build fails. A smaller test counter alone does not satisfy acceptance.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Establish comparable baselines and assertion owners

- [x] Capture at least three consistent full-suite samples with build/execution split, revision/toolchain/cache state, case counts and per-harness range; preserve the supplied CI evidence separately. <!-- completed: 2026-09-15T20:40+09:00 -->
- [x] Create the implementation reduction ledger from sections 3–6, verifying each donor's current assertions and retained owner before edits. <!-- completed: 2026-09-15T20:40+09:00 -->

### Step 2: Remove fixture installation and parser overhead

- [x] Implement direct production migration and guard-ready fixture levels; give actual setup/create consumers explicit helpers, including real setup in E2E; run every common-fixture consumer. <!-- completed: 2026-09-15T20:57+09:00 -->
- [x] Add broker-seeded rosters with returned IDs, distinct pane/capture identities and precise shim-log failures; migrate lookup/scan/message/corruption fixtures while preserving real creation/compensation owners. <!-- completed: 2026-09-15T20:57+09:00 -->
- [x] Move the specified argument grammar matrices into private production-Clap tests; retain executable exit/help/version and file/stdin/application boundaries. <!-- completed: 2026-09-15T20:57+09:00 -->

### Step 3: Consolidate setup, doctor and global families

- [x] Transfer installation assertions into the all-agent reinstall, selected Codex, mixed selector, OpenCode and partial-failure owners; remove mapped duplicate install/parser fixtures. <!-- completed: 2026-09-15T21:09+09:00 -->
- [x] Consolidate doctor text/JSON fixtures, transfer schema classifier/absence cases, require explicit JSON keys, and remove only the mapped report duplicates. <!-- completed: 2026-09-15T21:09+09:00 -->
- [x] Consolidate global guard fixtures and move both server-help cases into the global harness; remove the empty server target and update references. <!-- completed: 2026-09-15T21:09+09:00 -->

### Step 4: Reduce runtime duplicate work

- [x] Apply the section 4 fleet/member/message and broker donor transfers, including corrupt-message seeding; verify six fewer successful notification setup operations. <!-- completed: 2026-09-15T21:26+09:00 -->
- [x] Consolidate monitor recovery, scan, prompt and capture fixtures with all named output/ordering/failure assertions; retain untouched compensation, uniqueness and adapter scenarios. <!-- completed: 2026-09-15T21:26+09:00 -->
- [x] Reduce the parameterized broker timeline fixture to limits 2/3 and preserve the separate real batch-boundary fixture at 1,001 IDs. <!-- completed: 2026-09-15T21:26+09:00 -->

### Step 5: Replace broad documentation matchers

- [x] Build the shared immutable source inventory and focused section/table/link/binding/example checks, preserving logical installed aliases and explicit external skill/rule consumers; verify backend checks pass on unchanged owner sections before donor removal. <!-- completed: 2026-09-15T21:59+09:00 -->
- [x] Map all 65 current documentation functions to the new contract families or explicit semantic-review obligations; remove broad prose/history matchers and obsolete helper data. <!-- completed: 2026-09-15T21:59+09:00 -->
- [x] Add the compact malformed-structure/valid-illustration checker fixtures and verify accurate file/line diagnostics; review the retained normative instructions against the transferred semantic obligations. <!-- completed: 2026-09-15T21:59+09:00 -->

### Step 6: Simplify frontend/HTTP and E2E fixtures

- [x] Remove the five specified frontend parameter cases and transfer the HTTP broadcast/header/name-bridge assertions; seed the public 201-row cap in one transaction. <!-- completed: 2026-09-15T22:21+09:00 -->
- [x] Replace E2E fixed sleeps with bounded readiness/wake observation and scoped child cleanup; preserve real setup, second-process refusal, correct wake destination and ledger assertions. <!-- completed: 2026-09-15T22:21+09:00 -->

### Step 7: Preserve the existing Cargo cache policy

The user explicitly requested skipping the experiment and continuing with final review and a PR on 2026-09-15. Both experiment runs (34977169174 and 34979490282) are cancelled. Cache experimentation, candidate selection and confirmation are removed from the implementation scope; no cache policy change is included. The historical experimental specification in section 8 and local preparation evidence remain the record of the earlier plan. The implementation branch contains only the completed test reductions. Final review and PR publication are authorized.

### Step 8: Validate coverage, performance and review readiness

Accepted local evidence: [.execution/final-verification.md](.execution/final-verification.md). Three default pairs each passed 484 Rust tests and 45 frontend cases, with zero ignored/filtered Rust cases; existing lint and type/build checks passed. All 133 removed or relocated baseline names reconcile to retained owners or approved semantic-review obligations. Protected matrices and changed infrastructure fault checks pass, and all 170 recorded source hashes match after restoration. Local Rust task median is 120.790 seconds versus 155.150 baseline; harness sum is 39.030 versus 72.280. The report separates ranges, build time, operation counts and unattributed overhead. The cache experiment is skipped by user instruction; fresh final Reviewer approval remains open.

- [x] Run full default tests and lint/build validation; confirm protected failure matrices, default-run membership, expected case/harness changes and clean child-process cleanup. <!-- completed: 2026-09-15T22:41+09:00 -->
- [x] Publish before/after operation counts and comparable timing medians/ranges, separating test execution, build/cache and maintenance-only reductions. <!-- completed: 2026-09-15T22:41+09:00 -->
- [x] Review every completed reduction-ledger entry and the semantic documentation tradeoffs; retain any donor lacking a sufficient owner and report the final accepted scope. <!-- completed: 2026-09-15T22:41+09:00 -->
