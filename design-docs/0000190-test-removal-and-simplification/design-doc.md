# CAFleet Test Removal and Simplification

**Status**: Approved
**Progress**: 0/18 tasks complete
**Last Updated**: 2026-10-25

## Overview

Remove standalone test infrastructure and simplify CAFleet's runtime, documentation and agent workflows while retaining basic fleet operation and existing data. Retain necessary unit tests inline and the production mechanisms that protect transactions, pane input, deadlines and durable delivery. This document authorizes a future implementation; this creation turn changes design and audit artifacts only.

## Success Criteria

- [ ] Every standalone test-only repository file, directory, helper and dedicated testing role is removed, with its registrations and current references.
- [ ] The coverage ledger demonstrates that necessary unit coverage remains inline; removed cases are redundant, obsolete or implementation mirrors, and retained helpers/dependencies serve concrete behavior. Test count and LOC are measured outcomes.
- [ ] Existing database contents, supported upgrades, transaction boundaries, physical-pane exclusion, absolute deadlines, completion grace and durable delivery remain intact.
- [ ] Basic lifecycle, durable send/poll/ACK, native Director reception, ordinary-member retry recovery, herdr equalization, roster activity/idle and existing backend support pass the acceptance matrix.
- [ ] The selected runtime cuts are implemented; final evidence reports production Rust savings, measured test consolidation and documentation savings separately.
- [ ] The compact site and skills satisfy the stated content budgets, resolve their local links and install from the single compiled binary.
- [ ] Required checks and manual acceptance pass; every implementation task is complete before implementation review is accepted.

---

## Background

The user requires standalone test-only files/folders to disappear while retaining the necessary minimum of meaningful unit coverage inline. Select that minimum by protected behavior, rather than a test-count or line budget. Preserve herdr automatic sizing/equalization and roster activity/idle. Substantial other command/output simplification is acceptable. Existing coding-agent/multiplexer support, data and transaction/input-exclusion/deadline/delivery guarantees remain requirements. The CLI is the supported product boundary; undocumented Rust helper APIs may change.

The baseline is commit `35d3b44d6ad0d428d229e071a6643d6118ad959b`, version `0.29.0`. Source-backed findings and complete inventories are in [Tests](audit/tests.md), [Broker/CLI](audit/broker_cli.md), [Delivery/multiplexers](audit/delivery_mux.md), [Setup/agents](audit/setup_agents.md), [Documentation](audit/docs.md) and [Skills](audit/skills.md). The checkout's native Director listener is current behavior; the installed orchestration skills are older and remain untouched while this team runs.

### Non-overlapping size ledger

Physical LOC includes blank lines/comments. Count regular source files once, exclude historical `design-docs/` and `researches/`, generated output and dependencies, and count runtime-document symlinks as zero additional source lines. Lockfile changes are reported separately. Reproducible baseline inventories are in `.analysis/loc-baseline.json` and `.analysis/production-loc-groups.json`.

| Surface | Measured baseline | Planned remainder / saving | Evidence class |
|---|---:|---|---|
| Standalone test-only Rust | 14,843 LOC / 22 files | Delete original files after migrating necessary coverage inline | Exact gross file removal; relocated coverage is counted in the final suite |
| Inline Rust test items | 8,685 LOC / 350 test functions | Keep useful existing cases and consolidate migrated coverage by invariant | Measured baseline; no count/LOC reduction target |
| Production Rust under `cafleet/src/` | 8,335 LOC | Approximately 1,100–1,600 net LOC removed | Refactor estimate, not an implemented diff |
| Root SPEC plus site Markdown | 4,160 LOC | At most 1,100 site LOC; at least 3,060 net removed | Rewrite budget; includes 1,930-line SPEC deletion |
| Public/local skills and presets | 4,675 LOC | At most 724 LOC; at least 3,951 net removed | Rewrite budget; includes 3,137 gross whole-file lines |
| Repository rule Markdown | 243 LOC | At most 80 LOC; at least 163 net removed | Rewrite budget |

The test rows total 23,528 current LOC and 678 test functions. Gross deletion of a standalone file is not a net test-saving claim: necessary cases move inline, and useful existing inline coverage remains. Report final test LOC/count only after the behavior-based consolidation. They include the four test-only source files and their separate inline module declarations exactly once. Dedicated testing-role prose belongs only to the skills row. The production baseline excludes all current test regions, including those in mixed files later deleted. Build-script LOC, navigation, CI/manifests and other small configuration changes are outside the savings subtotal.

Production estimates use exclusive file groups rather than adding overlapping investigator totals:

| Exclusive group | Current production LOC | Estimated net reduction |
|---|---:|---:|
| `broker/`, `db/mod.rs`, `input.rs` | 1,990 | 190–310 |
| `cli/` except setup/doctor, `output/`, `presentation.rs`, `capture.rs` | 2,061 | 210–330 |
| Setup/doctor, diagnosis, permission emulator, assets/config/path resolution, coding agents and spawn parser | 1,521 | 580–680 |
| Delivery/receive/listen/signals, runtime and multiplexers | 2,615 | 120–280 |
| Other source modules | 148 | 0 |

The selected result is approximately 13–19% less production Rust, independently of test consolidation. The forecast excludes deletion of equalization/activity behavior and retains the small injected boundaries specified below; group budgets are estimates rounded conservatively, not additive investigator totals. Final implementation evidence replaces estimates with actual disjoint measurements; retained guarantees take precedence over an estimate.

---

## Specification

### 1. Consolidate necessary unit coverage inline

Delete `cafleet/tests/` and its common helpers, plus `cafleet/src/listen_tests.rs`, `cafleet/src/broker/test_support.rs`, `cafleet/src/coding_agent/test_support.rs` and `cafleet/src/multiplexer/test_support.rs`. First classify their cases by protected behavior and move necessary coverage into the owning production source files. Existing useful `cfg(test)` modules and short local helpers remain. Remove the deleted files' module registrations in the same change.

Use the following per-area ledger to select coverage. Group duplicates under their common invariant; for each removed family record its reason and the retained owner that still exercises any necessary behavior. Retain distinct boundary cases when they protect different failures. Required coverage is the acceptance criterion; there is no numerical test/LOC cap or promised net test deletion.

| Area / inline owner | Keep | Combine | Remove |
|---|---|---|---|
| Configuration, paths and spawn parser | Required versus optional values, valid zero limits, malformed supplied paths, brace/identity parsing and loud invalid input | Table-driven variations of the same parser boundary | Constant/getter checks, copied choice registries and exact incidental error prose |
| Coding-agent enum and preflight | Literal argv/flag boundaries, model/effort rules, selected-backend path validation, missing executable/preset | Common backend identity/availability setup using the retained small probe | Singleton-object shape and repeated no-op validation assertions |
| Schema, setup and stored-value decoding | Supported upgrades/data preservation, idempotence, identity/ledger/required-shape refusal, setup ordering, corrupt required values | Shared migration scenarios through real SQLite; local setup where useful | Full copied DDL registries, repeated column/key snapshots and connection-open counters |
| Fleet/member/message broker | Registration atomicity, live/same-fleet checks, full bodies, poll/strict ACK, preview eligibility, soft-delete history, pending placement, roster activity/idle | Equivalent broker/CLI projections and repeated state-transition setup | Cosmetic JSON/text order and deleted fleet-capture cases |
| Input and delivery | Actual lock identity/exclusion, under-lock revalidation, aggregate/nested deadlines, restoration/unwind, completion grace, FIFO recovery, uncertain/post-commit outcomes, safe notification rendering | Repeated transport permutations at the owning boundary | Shell-shim protocols, artificial cost accounting and duplicate race matrices |
| Wait and listener | Subject-inbox priority, retry rotation/exclusion, timeout, read-only replay/dedup/closure, SQL cancellation/cleanup, partial output/backpressure errors | Focused scenarios with the retained clock/IO boundaries and short local state | The large scripted event scheduler and its self-tests; final-diagnostic-map cases after that output removal |
| Multiplexer and native process runtime | Escape/literal/Enter order, reserved budgets, uncertain submission, herdr geometry/equalization on create/delete, native dual-pipe collection, timeout and direct-child reaping, signal restoration | Common transport behavior once, backend-specific command/geometry cases separately | Getter/constant mirrors and repeated per-layer sequencing matrices |
| Typed presentation and capture | Unicode/zero-limit/full-body behavior, optional/pending placement, activity/idle display, ANSI/CR and useful capture windowing | Semantically identical formatter and CLI snapshots | JSON-to-JSON implementation shapes, arbitrary heterogeneous walker fixtures and cosmetic spacing/key-order assertions |

Use direct real SQLite/native operations for storage/process invariants, and small explicit recording fakes where a unit boundary makes deterministic coverage simpler. Expected calls fail when an unexpected operation occurs. Keep local helpers in existing mixed production files; remove oversized shared shell/scheduler machinery rather than relocating it wholesale. Keep `tempfile` for necessary native file-lock/database/process cases. Other test dependencies require a concrete retained use; dependency removal follows actual reachability rather than a blanket rule. Manual real-product checks supplement this unit coverage.

Keep the mixed-purpose task registry and CI workflow. Set the existing test task to `cargo test --lib`; fold its invocation into the lint job and remove duplicate job setup. Keep lint, formatting, typecheck, release build and docs build. Remove duplicate dev declarations for `rusqlite` and `sha2`, `unicode-width` with the doctor table, and `serde_json`'s `preserve_order` feature. Regenerate Cargo.lock through Cargo. Retain production `sha2`, `regex`, `nix` signal/fs/poll and `rusqlite` hooks. Remove dedicated testing-role scratch entries from `.gitignore` while preserving historical artifact rules.

### 2. Keep safety owners explicit; simplify storage and deadlines

Validate schema once at the ordinary invocation boundary and after setup migration. Retain application identity, ledger validity, required columns/keys/indexes and ahead/incompatible refusal. Build a reference schema only for the selected supported recorded version; use the head reference for ledger validation of an ahead version. Delete the process-local validated-shape cache, mutex/eviction/snapshot key and all-version reference replay. Preserve structural comparison and acceptance of unrelated auxiliary objects; migrations remain the source of truth. Preserve the existing migration files and checksums.

Use one typed broker boundary. Remove test-only asset-list/presentation APIs, `InvocationHooks`, `run_with_hooks` and the classifier callback used only for synthetic schema scaffolding. Ordinary handlers receive the real `&mut Connection`. Fold `broker/queries.rs` into messaging, pass integer IDs without string round-trips, and use SQL `RETURNING` for inserted/updated records. Remove duplicate preflight schema/business checks when the owning boundary already performs them. Keep active same-fleet/root checks inside write transactions and placement revalidation after acquiring pane ownership.

Replace the 32-function database callback registry with one thread-local owning-connection identity and a nested frame stack. A frame contains its absolute deadline and whether it starts mandatory completion. Entry asserts that a nested scope uses the same connection; sequential connections and separate threads remain valid. Normal frames use the earliest applicable deadline; a completion frame starts a new effective segment at `max(original_deadline, now + 1 second)`. Popping it restores the original normal limit. Restore the outer CAFleet callback or pre-scope busy timeout on every exit, including unwind, and report explicit restoration failures. Cross-connection same-thread nesting becomes an internal invariant violation. The alternative flattened `DbWindow` design is excluded.

Preserve short SQL transactions around registration, message insertion, snapshots and finalization. External pane I/O occurs outside SQLite write transactions. Keep canonical database/backend/session/pane lock identity, owner-only stable lockfiles, sorted multi-pane lock acquisition and revalidation after ownership. Preserve signal deferral and independent completion grace after irreversible input or pane close. Reuse `PaneOwnership` for the CLI's known returned pane and retain the distinct registration guard; preserve backend ownership before split returns and CLI ownership until placement publication.

### 3. Reduce transport and diagnostic convenience layers

Use inherent methods on the closed multiplexer type; remove the duplicate `Multiplexer` trait and forwarding macro. Retain the small `CommandRunner`/`DeadlineRunner` boundary: it represents real process execution, enforces remaining deadlines and supports focused deterministic transport tests. Retain `SpawnProbe`/`SystemProbe` for selected-backend preflight. Preserve minimum remaining budgets, settle/Enter reservation, simultaneous stdout/stderr collection, per-stream drain limits, nonblocking poll, uncertain submission and direct-child kill/reap. Keep existing creation/compensation timing: context discovery, split/run and creation rollback have no new aggregate timeout, and a stalled creation can defer termination while protected. Existing bounded input/deletion/listener operations and completion grace keep their deadlines. Close only a known owned pane and preserve unknown-pane diagnostics and primary/cleanup causes.

Preserve herdr geometry reconstruction, automatic spawn sizing/equalization and post-close layout restoration. Keep detached creation, right/down choice, literal shell quoting and concrete ownership. Creating or deleting a pane continues to equalize the affected layout; optional layout-restoration failures retain their current visible diagnostics after successful storage finalization.

Remove `fleet capture` and its scan types/presenters, keeping `member list` and targeted `member capture` with ANSI/carriage-return cleanup, useful tail windowing, timestamps and hashes. Preserve `MemberActivity`, `last_sent`/`last_recv`/`last_ack`, existing timestamp selection/null handling, nonnegative idle clamping and the human idle display. Remove only the capture-specific roster projection and its inactive-activity query branch; the surviving roster still computes activity. Directors can use idle alongside expected replies and targeted pane evidence.

Keep the separate receive behaviors: `wait` prioritizes the subject inbox and fairly rotates ordinary-member preview retries, while `listen` is a read-only JSONL stream with ready/message/closed events. Retain the small `Clock`, `ReceiveIo`, `ListenIo`, `ListenOutput` and listener clock-injection boundaries for necessary scheduling/lifecycle/error tests; replace their oversized test implementations with focused local helpers. Preserve per-invocation failed-attempt exclusion, FIFO drain, one Escape per drain, emitted-ID tracking, restart replay, explicit ACK and zero automatic Director/self pane input.

Keep the standard Write-based JSONL helper for partial-write tests and the real deadline-aware descriptor adapter; remove duplicate retry/flush plumbing where the existing adapter already owns it. Retain SQL progress interruption, remaining-time busy waits, no output while a snapshot transaction is open, backpressure/broken-pipe failure and stdout-flag restoration. Simplify wait diagnostics to one immediate stderr event per failed attempt with member/message IDs and cause; remove the accumulated diagnostic map and `notification_errors` JSON field. The durable rows/stamps remain the current-state authority.

Text formatters consume typed records, including `MemberActivity`, and required strings. Remove JSON-to-JSON field renaming, arbitrary JSON truncation traversal, envelope guessing and duplicate emit forks. Preserve character-based preview limits, including zero, and exact full stored/JSON bodies. Named message fields and listener event framing stay usable; JSON key order and cosmetic text layout are outside the contract.

### 4. Simplify setup, backends and doctor

Consolidate coding-agent singleton objects and trait dispatch into one `CodingAgent` enum in its existing module; delete the per-backend source modules after moving their small argv branches. Derive CLI choices, stored-token parsing, iteration and configuration metadata from the enum. Keep the current backend tokens, permissions, literal argv and model/effort semantics, including Claude's variadic-flag boundary, Codex's single config argument and OpenCode's preset/model/effort rules. Use real executable/preset preflight and preserve OpenCode's fixed skills root versus relocatable preset root. Keep the small shared configuration/path parsers and their specified defaults/errors.

Setup becomes: validate URL/path, open, classify, migrate compatible state, verify head, install selected assets, record completed installations. Return the first actionable failure. Remove independent-half retry/aggregation, optional connection slots and duplicate setup guards. An incompatible/ahead database is refused before asset writes; an asset failure can leave a completed migration and earlier successful per-agent records, which remain truthful. Keep symlink-aware target replacement and offline embedding. Derive setup-owned skill roots from embedded top-level `SKILL.md` entries. Remove the unrelated installed `cafleet-research` cleanup action.

Remove the global asset-version gate from ordinary commands. They require a compatible database and their own actual prerequisites, so unrelated stale metadata or configuration cannot block durable poll/ACK. Keep the asset ledger and successful install recording; installation versions are diagnostic facts. Before registration or pane effects, member creation resolves and validates only the selected backend’s configuration paths using the shared resolver, then checks its executable and required preset. This preserves supplied-path errors for the selected backend without consulting unrelated backends or adding a replacement install-version gate. Operators run setup after installing a new CLI; active workflows can continue with their already installed assets.

Delete `member_permissions.rs` and its module registration. Doctor becomes a compact independent diagnostic pass over multiplexer context, database compatibility and per-backend executable/configuration/current-path install facts. It reports all independently obtainable results; missing optional backends and stale/missing install metadata are informational, malformed supplied configuration and actual check failures are errors. Preserve `doctor --json` with the replacement shape `{"checks":[{"name":"...","status":"ok|info|error","detail":"..."}]}`. Text prints one line per check; exit 1 means an error check, exit 0 means none. Doctor is advisory rather than a universal startup gate. Remove old nested output, tables, historical-path reports and the Claude permission emulator. Actual host enforcement, spawn allow rules, presets, ready evidence and denied-command routing remain authoritative.

| Diagnostic state | Status and dependent checks |
|---|---|
| Usable multiplexer context | `ok`; include its concrete context. |
| Missing multiplexer binary/context, ambiguity or discovery failure | `error`; retain the actual cause. Continue database and backend checks. |
| Head-compatible database | `ok`; current-path install metadata can be queried. |
| Missing or behind database | `error`; recommend setup. Install metadata is `info` with an explicit unavailable reason. |
| Ahead, foreign/malformed or unreachable database | `error`; preserve the cause and data. Install metadata is `info` with an explicit unavailable reason. |
| Valid backend configuration path | `ok`; continue its independent executable/preset checks. |
| Malformed supplied configuration | `error`; dependent preset/install checks are `info` with an unavailable reason, while executable discovery still runs. |
| Backend executable absent | `info`; that backend is optional. Its preset availability is `info` with an unavailable reason; valid configuration and install facts can still be reported. |
| Required preset absent/unreadable for an available backend | `error`; recommend setup for that backend. |
| Current-path install record matches the CLI | `ok`; report the recorded version. |
| Install record absent or stale | `info`; recommend setup, without claiming installed files were inspected. |
| Install query fails despite a usable database/path | `error`; include the actual query/decode cause. |

Unavailable checks keep the same `name`, use `status: "info"`, and explain the failed prerequisite in `detail`; they are distinct from an observed missing record. Collect and emit the complete check list, including all independent failures, before selecting exit 0/1. Argument-parse failures retain the CLI parser’s normal behavior.

### 5. Consolidate documentation and agent operation

Delete `SPEC.md` and all live citations. CLI help owns the option registry; migrations own storage shape; source owns rendering; the site explains operator behavior; skills own agent procedure. Keep the current theme/demo and publication toolchain. Reduce site Markdown to at most 1,100 lines using the surviving URLs: home, Quickstart, Overview, fleet guide, design workflow guide, CLI/backend/envelope references and Contributing. Delete `concepts/storage.md`, `concepts/member-lifecycle.md`, `concepts/coding-agents.md`, `spec/data-model.md` and `spec/multiplexer-backends.md` after moving useful behavior to those owners. Update navigation and anchors; use Reference as the navigation label. Keep installation, trust/permission setup, diagnosis, recovery, teardown and publication instructions.

Keep the offline symlink/embedding mechanism. Remove aliases to deleted pages and mirror every surviving site Markdown page except the homepage under `skills/cafleet/reference/runtime/` with the same relative path. Keep links within that tree relative and closed. The existing build watches canonical docs and setup writes embedded bytes as ordinary installed files. Count canonical pages once; verify both site links and the installed tree.

Replace the public skill framework with one general member role, one Director procedure, concrete backend notes and concise design recipes. Delete `skills/cafleet/reference/base-dir.md`, `prompt-routing.md` and `supervision.md` after merging essential ownership/authorization/receiver behavior. Delete the design skill's `create/`, `interview/` and `execute/` trees and `reference/coordination.md`. This includes every dedicated testing role and all workflow-specific roles. Preserve the public skill entry names and a compact template in `reference/guidelines.md`.

A Director supplies explicit absolute output/scratch paths and scoped assignments, owns Git/user decisions, receives ready, reads full durable messages and ACKs consumption. Members implement assigned work and suitable checks. Messages carry assignments/findings/blockers/evidence directly, with artifact paths for long reports. Replace mandatory marker grammar, per-step TDD/test commits, fixed question categories, interview round machinery, path sentinels, model catalogs/prices/refresh rules and ranked replacement with this procedure. Explicit backend/model/effort requests remain authoritative; omitted model/effort uses the selected backend's configured defaults. Preserve one owned native receiver, bounded receive ownership, replay reconciliation, observed process exit before replacement, uncertain-input inspection and owned-member teardown. Review substantial work independently; routine edits use proportionate validation. Resolve only consequential missing information, preserve prior answers/annotations and honor stop requests. Existing authorization governs publication.

Delete `.claude/skills/cafleet-model-list-refresh/`, `skill-author/`, `update-readme/` and every nested workflow/role/reference subtree of `clean-docs/`; retain its single short core. Simplify repository rules to one ownership/commands/migration/writing policy and reconcile Git handling with active host rules. Remove deleted path/role/feature references completely. Existing historical records stay intact.

| Surviving skill/preset source | Maximum LOC |
|---|---:|
| `skills/cafleet/SKILL.md` | 100 |
| `skills/cafleet/roles/member.md` | 25 |
| `skills/cafleet/roles/director.md` | 120 |
| `skills/cafleet/reference/coding-agents.md` | 100 |
| `skills/cafleet-design-doc/SKILL.md` | 160 |
| `skills/cafleet-design-doc/reference/guidelines.md` | 65 |
| `.claude/skills/clean-docs/SKILL.md` | 80 |
| Existing executable presets combined | 74 |

Preserve executable preset authority rather than broadening allow patterns for brevity. Future templates use task checkboxes as progress truth; existing documents/answers remain readable without conversion. This document uses the current creation template; implementation progress remains separate from design approval.

### 6. Accepted compatibility changes and acceptance evidence

Accepted changes are bounded to the removed fleet-capture command, compact doctor/JSON, advisory asset versions, fail-fast sequential setup, immediate wait diagnostics, cosmetic output/key order, internal Rust helpers and removed workflow/model/role paths. There are no compatibility aliases or historical notices in maintained product surfaces. Existing database paths/identity/migrations/data, backend support, message fields/listener framing and the safety owners described above remain.

The implementing member records evidence in this task's existing artifacts; create no replacement verification role, permanent script, fixture tree or test-only source module. Use the built binary, an isolated database/configuration and disposable real multiplexer sessions, preserving this team's installed binary/assets.

| Acceptance scenario | Required observation |
|---|---|
| Existing and fresh storage | Fresh/repeated setup works. Copied V1/V2/current databases preserve complete application rows through setup. Foreign/ahead/malformed copies and their sidecars stay unchanged on refusal; assets are untouched. Invalid stored required values fail explicitly. |
| Real basic operation | Exercise lifecycle on each supported multiplexer and launch each available supported coding-agent backend. Confirm ready, literal identity/prompt boundaries, list, targeted capture, detached creation, automatic herdr equalization after both add/delete, activity timestamps/idle changes and owned teardown. Record any unavailable backend as incomplete verification. |
| Durable messaging | Send Unicode/multiline/long bodies, compare exact full JSON/poll text, inspect safe preview/pointer, ACK and repoll. Director/self sends perform no pane input; send success keeps the same committed ID after transport failure. |
| Retry and exclusion | Use real concurrent send/input and a held physical-pane lock, including duplicate registrations/database aliases. Confirm serialized complete submissions, bounded failure, unchanged registry on failed deletion and later FIFO recovery through send/wait. Revalidate a placement changed during lock wait. |
| Finalization and cancellation | Hold a real SQLite reader/writer across a disposable input/close operation. Confirm aggregate deadlines, independent completion grace, lock retention and visible storage errors. Send termination during an observed protected span and verify finalization before exit. Run the retained native process cases for dual-pipe, timeout, inherited-pipe and same-parent reaping behavior. |
| Native reception | Confirm ready before replay, per-process deduplication, restart replay, explicit ACK, finite closure and zero retry/pane effects. A slow/broken output consumer produces bounded visible failure without consuming rows or leaking stdout state. |
| Setup and permissions | Doctor reports independent checks and actual configuration errors. Unrelated stale/malformed backend metadata does not block a ready fleet's send/poll/ACK. A malformed selected-backend path fails member creation before registration/pane effects; an unrelated malformed path does not affect that launch. Actual host deny/ask rules still apply; OpenCode preset relocation preserves its fixed skills location. |
| Packaging and reference closure | Build and inspect isolated embedded-asset installation: deleted standalone test/role files are absent, retained unit modules and required presets remain, every local Markdown link/anchor resolves, and canonical docs edits reach embedded bytes. Preserve active installed assets until teardown. |

#### Automated and supplementary boundary evidence

The retained inline suite is the primary evidence for parsing, transformations, state transitions, timing decisions and native invariants in the coverage ledger. Preserve real native tests in their owning production modules: for example, process cases can spawn a real Child, record its PID, invoke the actual private collector and assert same-parent `waitpid(..., WNOHANG)` returns ECHILD. Exercise dual stdout/stderr, failure output, timeout and inherited pipes there. Keep actual SQLite/file-lock cases with local temporary resources, and deterministic receiver/transport cases through the retained small interfaces. No separate driver, diagnostic entrypoint or permanent verification script is required.

Use the following supplementary procedures where a changed boundary needs live-host evidence beyond those unit tests. Pre-arm transient controller actions before starting; debugger pauses consume real monotonic time. Record the final source breakpoint, actual IDs, timing, observation and cleanup. Each required invariant must have adequate automated evidence and any necessary live-host observation; missing evidence or an unavailable required backend stays explicit incomplete acceptance.

Build symbols through the existing task when needed: `mise //cafleet:build --config profile.release.opt-level=0 --config profile.release.debug=2`. LLDB and Python's standard SQLite/process/descriptor APIs can drive disposable real operations. Map the semantic breakpoints below to final source; baseline pointers identify the boundary without adding a product hook.

1. **Placement/exclusion.** Pause a real input writer after it owns the concrete pane lock. Start a second writer and observe its initial placement read before ownership (baseline `delivery.rs:181`). A pre-armed controller commits a different real pane ID through a separate SQLite connection, then resumes both within the original wait budget. Require under-lock detection and no input into the stale pane. Repeat with the holder retained and unchanged placement: the contender times out within the existing five-second budget without registry mutation. Restore the disposable placement and close all owned panes.
2. **Finalization and signals.** Break after Enter/before stamp (`delivery.rs:272`), or after close/before deregistration (`cli/member.rs:482`); establish the actual input/closed pane and committed ID first. Only then acquire `BEGIN IMMEDIATE` separately. Expire the original deadline while stopped, resume and release the blocker after 100 ms: finalization must succeed within independent completion grace. Holding beyond grace must return a visible storage error within two seconds of uninterrupted resume, retain the committed record and avoid false stamp/deregistration success. Input ownership lasts until completion returns. To verify deferral, pass SIGTERM to that exact paused protected process before the release run and inspect final state before accepting its exit. Roll back blockers, inspect rows and explicitly recover/tear down the disposable member. Native unit tests cover the deterministic deadline/restoration/signal rules; this procedure establishes the actual pane-side-effect boundary.
3. **Listener output.** Seed a pending Director/self body larger than pipe capacity. A transient console creates a real pipe, retains a duplicate write end and records its `F_GETFL` flags, then launches `message listen <id> --timeout 2` on that same open file description. Read ready and stop draining: require visible bounded output failure/exit within three seconds of ready, unchanged pending consumption and restored flags on the retained descriptor. Repeat with all read ends closed and with a normally drained stream. Close descriptors, reap the listener and inspect replay before ACK/teardown. The inline partial-write/lifecycle/SQL-cleanup tests remain necessary alongside this OS observation.

Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:build` and `mise //docs:build` through the existing task registry. Inspect changed skill/root Markdown and embedded links directly; the docs build alone checks only the site. Review the final diff and current-tree reference search for removed interfaces, SPEC citations, support modules, test environment protocols and dedicated testing roles. Exclude history from removal searches and savings. Report actual test/production/docs-skills totals and remaining manual evidence gaps separately.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Establish compact documentation and skill owners

- [ ] Consolidate the site, delete SPEC and selected pages, update navigation and close the offline runtime mirror. <!-- completed: -->
- [ ] Replace public workflow/role/coordination/model machinery and local maintenance subtrees with the specified compact skill sources. <!-- completed: -->
- [ ] Simplify repository rules, update all affected examples/links and preserve active installed runtime/assets. <!-- completed: -->

### Step 2: Consolidate unit coverage and remove standalone test infrastructure

- [ ] Classify coverage by the per-area ledger, migrate necessary standalone cases inline, then delete standalone test files/registrations and redundant test protocols. <!-- completed: -->
- [ ] Keep/consolidate the necessary inline unit cases and local helpers, recording the protected behavior and each removal reason without count/LOC targets. <!-- completed: -->
- [ ] Narrow the test task, merge shared CI checks, remove obsolete scratch entries/dependencies and regenerate the lockfile. <!-- completed: -->

### Step 3: Simplify storage, validation and budget ownership

- [ ] Replace schema caches/all-version replay with selected-version structural validation and retain existing migration/data compatibility. <!-- completed: -->
- [ ] Consolidate typed broker queries, integer IDs, RETURNING results and invocation-owned connection/validation boundaries. <!-- completed: -->
- [ ] Implement the single-owner nested database deadline stack, explicit restoration and shared pane ownership compensation. <!-- completed: -->

### Step 4: Simplify transport and remove convenience state

- [ ] Remove duplicate multiplexer dispatch while retaining the small runner/probe/clock/IO boundaries needed by focused unit tests and preserving operation deadlines, creation timing and process collection. <!-- completed: -->
- [ ] Remove fleet capture and its unused projections while preserving targeted capture, herdr automatic equalization and roster activity/idle. <!-- completed: -->
- [ ] Simplify wait/listener diagnostics and bounded output, and render human text directly from typed records. <!-- completed: -->

### Step 5: Simplify setup and diagnostics

- [ ] Consolidate coding-agent registry/argv/preflight and derive setup-owned skill roots from embedded manifests. <!-- completed: -->
- [ ] Make setup sequential, preserve per-agent success recording and remove obsolete installed-entry cleanup. <!-- completed: -->
- [ ] Replace rich doctor/permission emulation and the global asset gate; complete output/dependency/reference cleanup. <!-- completed: -->

### Step 6: Verify the resulting product and reduction

- [ ] Run the retained suite, lint/typecheck, release build and site build, then verify isolated embedded-asset/link closure. <!-- completed: -->
- [ ] Execute and record every real-product acceptance scenario, resolving failures and incomplete boundary observations before completion. <!-- completed: -->
- [ ] Recount disjoint LOC and inline tests, inspect complete removal/safety boundaries and obtain independent implementation review. <!-- completed: -->
