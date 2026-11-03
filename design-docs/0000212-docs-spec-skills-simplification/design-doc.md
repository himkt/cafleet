# Docs, SPEC and Skills Simplification

**Status**: Complete
**Progress**: 30/30 tasks complete
**Last Updated**: 2026-11-03

## Overview

Resolve the contradictions, redundancy and ambiguity that a read-only audit found in `SPEC.md`, the `docs/` site, `README.md` and `skills/`. Every finding has been re-verified against the files and, for behavioral facts, against the Rust code. This document records one decision per finding and runs the edits as three sequential clean-docs simplification runs, with the design doc authorizing each correction row.

## Success Criteria

- [ ] Every audit finding (SA1-20 with its lower-ranked observations, SB1-20 with its minor bullets, D1-20, K1-27) and every verification-discovered finding (SN, DN, KN) carries a decision label (§ Decision labels) in § Decisions. Every entry that asserts or corrects a fact cites its verification basis.
- [ ] Every edit lands through a clean-docs simplification run. Three runs execute in rule order (`SPEC.md` → `docs/` + `README.md` → `skills/` + the decided rule edit), and each run's applied diff is verified before the next run starts.
- [ ] Every accepted correction (`DD` row) matches its cited code or binary evidence. The post-apply review confirms the diff contains only approved rows.
- [ ] `git diff main -- src migrations Cargo.toml Cargo.lock build.rs` is empty, and so is `git diff main -- .claude/skills`.
- [ ] `mise //:test` and `mise //:lint` pass after every run; `mise //docs:build` passes after Run 2 and Run 3.
- [ ] Every heading anchor removed or renamed by a run has zero remaining inbound links in the repository (excluding `design-docs/` and `researches/`). Every `SPEC.md#<slug>` link in `docs/` and `README.md` resolves to a `SPEC.md` heading.
- [ ] `git grep -n -E "inbox stream|native receiver|native inbox tools" -- docs/docs README.md` returns nothing, and the Core terms table defines `listener` and `finite receive`.
- [ ] Every contradiction a scanner discovers during a run is recorded in § Run-discovered corrections with evidence before application. Every code-side defect is recorded in § Follow-ups, with no Rust change.
- [ ] One feature branch and one PR, with one commit per surface.

---

## Background

Four auditors read the documentation surfaces read-only and produced ranked reports in `.audit/` at the repository root (gitignored and local): `.audit/auditor-spec-a.md` (`SPEC.md` 1-1913), `.audit/auditor-spec-b.md` (`SPEC.md` 1900-end), `.audit/auditor-docs.md` (`README.md`, `docs/docs/`) and `.audit/auditor-skills.md` (`skills/`). Their quotes had not been re-verified. For this document every quote was checked at `c7cdc830`, and every behavioral claim was checked against `src/`, `migrations/V1__baseline.sql`, the embedded `src/coding_agent/bundled.json` and `cafleet --help`. Several audit proposals turned out to be wrong or incomplete (for example SPEC-B #11 is not a contradiction, and docs #2's proposal would drop facts). Verification also found new defects.

The clean-docs simplification workflow (`.claude/skills/clean-docs/SKILL.md`) owns P3 (redundant prose) and P5 (verbose phrasing) rows. These rows keep voice and behavior unchanged, and the workflow reports content drift as Observations. Resolving a contradiction changes a stated fact or an agent instruction, so this document is the authority for those rows (§ Execution model). The clean-docs skill itself stays unchanged.

The verification reports with seed wording live locally beside this document in `.verify/` (`spec-a.md`, `spec-b.md`, `spec-redundancy.md`, `docs.md`, `skills.md`). They are gitignored and non-authoritative: where they differ from this document, this document governs.

---

## Specification

### Execution model

Each run is one complete clean-docs simplification workflow run (its Per-run process steps 1-8) with these run-level inputs. The Director renders the three lines verbatim, with `<surface heading of this run>` substituted, into the CONTEXT LINES slot of every scanner and reviewer prompt, so the skill text stays unchanged:

```text
DESIGN DOC: /development-pvc/work/himkt/cafleet/design-docs/0000212-docs-spec-skills-simplification/design-doc.md
DECISIONS: § Decisions — <surface heading of this run>
AUTHORIZED ROWS: Beside the simplification rubric's P3/P5 rows, this run carries the row classes `DD <ID>` and `DD-new`, governed by the DESIGN DOC's § Execution model > Row classes in the findings artifact table. Those rows are design-authorized exceptions to the simplification zero-behavior-change invariant, the KEEP guardrails (including contract detail and error strings) and the voice checks. A `DD <ID>` row is limited to the replacement text its DECISIONS entry gives; a `DD-new` row is limited to the fix of one verified contradiction, recorded with quoted sides, code or binary evidence, verdict and exact replacement. Scanners emit a `DD-new` row instead of an Observation for every contradiction they verify; every other row stays within the simplification rules.
```

#### Orchestration

The executing Director runs the three clean-docs simplification runs sequentially and spawns no execute-workflow Programmer or Tester. Each run creates its own fleet (clean-docs Per-run step 2) and deletes it at step 8. The Director keeps one native listener: it arms the listener for each run's Director ID after `fleet create` and retires it when that fleet is deleted, so no fleet stays registered across runs. After Run 3, the execute workflow's final gates still apply: a fresh, independent Reviewer reviews the full branch diff in its own short-lived fleet, and user approval precedes the PR (Implementation Step 5 and Step 6).

| Run | Slices (disjoint whole-file ownership) | Gate before the next run |
|---|---|---|
| 1 | scanner-1: `SPEC.md` | Post-apply review approved; `mise //:test`, `mise //:lint`; anchor check |
| 2 | scanner-1: `docs/docs/concepts/**`; scanner-2: `docs/docs/index.md`, `quickstart.md`, `mixed-backend-team.md`, `contributing.md`, `README.md` | As Run 1, plus `mise //docs:build` and the rendered-pipe check |
| 3 | scanner-1: `skills/cafleet/**`, `.claude/rules/documentation-maintenance.md`; scanner-2: `skills/cafleet-design-doc/**` | As Run 2 |

#### Row classes in the findings artifact

| Class | Source | Content rule | Reviewer check |
|---|---|---|---|
| `P3` / `P5` | clean-docs rubric, seeded by a § Decisions consolidation row or discovered by the scanner | Owner keeps the fact; each other copy becomes a pointer or is deleted. Voice and behavior are unchanged. | The clean-docs simplification checks |
| `DD <ID>` | A § Decisions correction row | The replacement uses the text given here. It may be adapted only to fit the current sentence boundaries, with every fact, string and pointer intact. | Replacement matches this document; the quoted anchor still exists |
| `DD-new` | A contradiction the scanner finds during the run | Quotes both sides with `file:line`, cites evidence (code `file:line` or binary output), states the verdict and gives the exact replacement | Evidence re-checked against code or binary; docs follow the binary |

Coverage rules:

- Each accepted decision maps to at least one findings row, or to a row marked `already satisfied` with its evidence. The Director checks this coverage before sending `ready (findings)` to the reviewer.
- A rejected decision produces no row.
- A row whose quoted anchor no longer matches the file is `blocked` for Director arbitration.
- Candidates for the residue or affirmative workflows that § Decisions does not list stay Observations and go into the run report.
- Line numbers in § Decisions refer to `c7cdc830`. Scanners locate each row by its quoted text.

Run-discovered findings (user-decided scope):

1. A scanner may add further P3/P5 rows in its slice.
2. A scanner may fix a newly found contradiction as a `DD-new` row. It needs the same verification evidence as an audited correction.
3. After reviewer approval and before application, the Director appends each approved `DD-new` row to § Run-discovered corrections in this document.
4. A finding whose correct resolution is a code change goes to § Follow-ups. The documentation then describes the shipped binary.

#### Run-wide constraints

- Rust sources, migrations, Cargo files and `.claude/skills/**` stay unchanged.
- `SPEC.md` keeps its section structure: Run 1 renames and removes no heading. Where a row makes a whole section a pointer, the heading stays and only its body becomes the pointer.
- An edit that removes or renames a heading anchor updates every inbound link in the same run. Anchors linked from `.claude/**` stay, because `.claude/skills/**` is out of scope.
- Skills stay backend-neutral: backend-specific facts live in `skills/cafleet/reference/coding-agents.md` Note → applies at rows (`.claude/rules/documentation-maintenance.md` § Coding-agent overlay). IMPORTANT lines, hard role constraints, start cues and spawn-skeleton lossless items survive verbatim in meaning.
- The installed CLI and skill replicas stay as they are during the runs (no install task), so the executing fleet keeps its runtime.

### Terminology for the docs site

These terms govern `docs/` and `README.md` through the Core terms table (Term tie-break). `skills/` keeps its own vocabulary; for example, `director.md#receiver-recovery` is linked from `.claude/**`.

| Concept | Term | Replaces |
|---|---|---|
| Persistent `cafleet message listen` observer | `listener` | `inbox stream`, "native receiver", "receiver", "native inbox tools" |
| Bounded `cafleet message wait` | `finite receive` | the bare `receive` Core term |

### Decision labels

Every § Decisions entry carries one label. An entry that asserts or corrects a fact cites its verification basis (`Verified:` or `Basis:`). A pure consolidation's basis is the copies its Action names, each verified at `c7cdc830` to state the same fact; copies that differ in a fact become a correction entry instead.

| Label | Meaning |
|---|---|
| `accept` | The audit's proposal is adopted as written |
| `accept-revised` | The audit's finding is adopted with verified changes to its wording, scope or owner |
| `reject` | No edit; the Rejected table gives the reason |
| `new` | Found during verification and accepted |

### Decisions — SPEC.md (Run 1)

#### Corrections (`DD` rows)

- **SA1** (accept-revised) `SPEC.md:97`. Verified: `fleet delete` ends members one by one, each member's deregistration in its own transaction, then runs one final transaction (`src/cli/fleet.rs:427-436,520-566`). Replace the third sentence: "Delete locks and revalidates the active concrete placements, ends each active ordinary member with a concrete pane through member deletion, then soft-deletes the fleet, deregisters the remaining members and removes their placements in one final transaction, preserving messages; a member that cannot be ended stops the command with the members ended before it already deregistered (§6.3 `fleet delete`)."
- **SA2** (accept-revised) `SPEC.md:1041`. Verified: creation records member and run in one transaction (`src/runtime/lifecycle.rs:353-400`), and runs are never deleted (`V1__baseline.sql:66`). Run-less rows are still handled (`src/broker/members.rs:15-24`, `src/cli/fleet.rs:381-392`). Replace the second sentence: "Member creation records the member's run in its registration transaction, so every ordinary member CAFleet registers has a recorded run. `runtime` is `null` for the root Director and for an ordinary row without a run, a registry state CAFleet never writes, which send, prompt, exec, recover and delete handle as specified at each command."
- **SA3** (accept-revised) Verified: only setup initializes (`src/cli/setup.rs:86-91`); every other command except doctor refuses before any database access (`src/cli/mod.rs:101-123`).
  - `SPEC.md:85` third sentence: "Setup initializes a missing file, or an empty SQLite file with identity zero and no user tables, with the baseline atomically; every other guarded command refuses such a file with `no cafleet database; run 'cafleet setup'`."
  - `SPEC.md:1163` command cell: "(any command except `setup` and `doctor`)".
  - `SPEC.md:388` first sentence: "Every fleet, member, message, event, input and permission command validates the current format (§8 Database schema) before any other database read or write."
- **SA5** (accept-revised) **+ SN1** (new) `SPEC.md:1858-1861`. Verified: the only OpenCode boundary is `2.1.4` (`bundled.json:2543`), and earlier versions are refused before registration (`src/coding_agent/catalog.rs:1406-1417`, `mod.rs:504-510`). `member create` probes the version even though `ensure_available` does not (`src/cli/member.rs:416-420`). Replace "availability checks validate PATH and the selected preset without detecting a version." with "availability checks validate PATH and the selected preset, and `member create` separately probes the native version for profile selection." Replace the next sentence with: "Spawn, preset loading, broker input and native shell input are validated against opencode 2.1.4; the OpenCode profile from `2.1.4` is the first boundary, so `member create` refuses an earlier probed version (§6.9 Profile selection by minimum version)."
- **SA6** (accept-revised) `SPEC.md:1420`, `:1422`. Verified per writer: 50 ms for deletion, wrapped clients and explicit keys (`src/pane_lock.rs:162,271,306-310`), and a 1 s press reservation (`src/runtime/press.rs:83-88`). The runner retries an active attempt on each loop pass and a queued request on a 100 ms timer (`src/runtime/input.rs:2392,2896-2910`, `service.rs:1324-1345`). Rows:
  - Acquisition: "Exclusive nonblocking OS file lock; preserve unexpected errors with path/member context"
  - Contention: "Deletion, wrapped clients and explicit keys retry a busy lock every 50 milliseconds; explicit keys stop waiting one second before their deadline. The runner retries an active attempt's lock on each event-loop pass and a queued request's lock every 100 milliseconds; a lock still busy at its wait bound fails that writer"
- **SA7** (accept-revised) `SPEC.md:854-855`. Verified: `CAFLEET_MEMBER_ID` is always set (`src/coding_agent/mod.rs:119-131`) and read only as a presence check (`src/cli/mod.rs:69,88-98`). First sentence: "The spawned member learns its identity from the literals rendered into its prompt; commands take every ID as a literal argument, and `CAFLEET_MEMBER_ID` only marks the process for the Director-only command guard, which reads its presence and never parses its value." Align the closing clause of `SPEC.md:2196` to the same fact.
- **SA8** (accept-revised) `SPEC.md:1670`, `:1763-1767`. Verified: the registry is a fixed `ValueEnum` in declaration order with exact lookup (`src/coding_agent/mod.rs:48-55,71-84,224-229`). An unknown stored token fails with exit 1 (`src/cli/member.rs:248-251`). Delete the second sentence of 1670. Replace 1763-1767 with: "The backend registry is the fixed set of supported backends in registry order, each keyed by its `name`. Resolution is an exact match of the persisted `placement.coding_agent` token, with no fuzzy matching or default; a token with no entry fails with `invalid stored value for member_placements.coding_agent: VALUE`, exit 1."
- **SA13** (accept-revised; help string) `SPEC.md:166`. Verified: the binary's help begins `Launch-only native permission policy for the resolved backend — ` (`src/coding_agent/mod.rs:249-262`). Quote the complete help text including that prefix.
- **SA20a** (accept-revised) `SPEC.md:1171` Notes cell. Verified: the check is the first step of the `member create` body, after the schema guard and before Director resolution (`src/cli/member.rs:371-372,382`). Notes: "A usage error raised by the first fleet check, after the schema-version guard and before Director resolution". The exit-code inconsistency is follow-up F1.
- **SA20b** (accept) Verified: the argv and `mini` are defined only at `SPEC.md:1850-1855`. `SPEC.md:1498`: "An OpenCode member (`opencode mini`, §6.7 Per-backend `build_spawn_argv`) receives pushed notifications …". `SPEC.md:1561`: "OpenCode `mini` captures can expose only the status bar …".
- **SA20c** (accept-revised) Verified: the lock is acquired within the original 5 s, settlement has its own 20 s budget, and the deadline is extended by the elapsed time (`src/cli/fleet.rs:366-376`, `src/coding_agent/hermes_settlement.rs:110-114`, `src/time.rs:86-88`).
  - `SPEC.md:908` first sentence: "For a `hermes` run, after the pane lock is held and before runner shutdown, run the Hermes work settlement (§6.7 Hermes work settlement) under its own 20-second budget; the deletion deadline then moves later by the time the settlement took, so lock acquisition stays bounded by the original five seconds and the remaining steps keep the time they had when settlement began."
  - `SPEC.md:415` last sentence: "One operation-wide five-second deadline bounds the whole command, extended by the time each Hermes work settlement takes (§6.3 `member delete`)."
- **SN2** (new) §6.3 Error Messages intro (`SPEC.md:1158`). Verified: the executable prefixes the first stderr line with `Error: ` (`src/main.rs:45`); command prose such as `SPEC.md:411` quotes messages without it. Append: "The executable prefixes the first stderr line of every application error with `Error: `; command sections quote messages without that prefix."
- **SN3** (new) `SPEC.md:1043` example. Verified: label `native command` (`src/cli/member.rs:689,854`; test at `:1836-1854`). Use `native command delivery to member 110 is uncertain: manual_input (method=profile)`.
- **SN4** (new) `SPEC.md:2247`. Verified: a profile entry has exactly `backend`, `min_version`, `rules` and `capabilities` (`src/coding_agent/catalog.rs:85-93,1216`). Remove "rule-set version" from the list of profile declarations.
- **SN5** (new) `SPEC.md:1520` pointer. It points to §6.9 Delivery reporting, which points back. Use "(§6.9 Delivery method and interaction priority)".
- **SB17** (accept) **+ SN6** (new) Change-relative wording becomes current-state wording. Verified: each sentence describes a change rather than the current state, and the cited code gives the current state:
  - `SPEC.md:2318`: delete "Each automatic-mode profile replaces its predecessor as its version's bundled profile and keeps every predecessor rule and answer plan unchanged." Each backend has one profile in `bundled.json`.
  - `SPEC.md:2863`: "In this release, machine decision outcome remains `unknown`" → "Machine decision outcome is `unknown`".
  - `SPEC.md:3160`: "Lifecycle requests carry `protocol_version`, `kind` and `identity` and no `expires_at_mono_ns`: `prepare` adds `remaining_ms`, `start` adds `pane_id`, and `health` and `shutdown` add nothing." Source: `src/runtime/control.rs:1261-1267`, `src/runtime/lifecycle.rs:830,846`.
  - `SPEC.md:3317` paragraph: "Preview forms, body folding, inline limits and long-body pointers follow §6.5 Push notifications."
  - `SPEC.md:3302` "with the existing delivery context" → "with its delivery context". `SPEC.md:3303` "Existing error categories, input identity and stage diagnostics" → "Exit 1 with input identity and stage diagnostics" (`src/error.rs:49-57`).
  - `SPEC.md:1043` "retains its existing exit category" → "exits 1". Rewrite that paragraph's "preserves its existing text row" and "The existing submitted-success lines", plus `SPEC.md:1154` "Preserve the existing input text line", `SPEC.md:2929` "under existing database ownership/retention behavior" and `SPEC.md:3452` "preserve the existing evidence route". Each becomes a present-tense statement of the same current behavior, naming the text row, line or route it means; the reviewer checks each against the code path the sentence describes.
- **SB1** (accept-revised) `SPEC.md:2029-2032`. Verified: under the omitted `approve-for-me` Codex's automatic reviewer decides escalations (`src/coding_agent/mod.rs:244,285-291`; `bundled.json:2372,2410`). Replace with: "That minimum set suffices for members, not for a Director session: the rules settle only the commands they match, so every other `cafleet` command a Director runs follows that session's own approval policy and sandbox. It runs inside the workspace-write sandbox, and an escalation goes to Codex's automatic reviewer under the omitted `approve-for-me`, to a native approval form under explicit `on-request`, and returns as a failure under explicit `never`."
- **SB2** (accept-revised) `SPEC.md:2314`. Verified: the Claude `min_version` is `2.1.302`, with fixtures at 2.1.302, 2.1.308 and 2.1.309 (`bundled.json:1508,775-828,1065,1075-1138`; `catalog.rs:805-823`). Sentence: "Each existing profile's `min_version` is the capture version of its earliest native fixtures; fixtures captured on later versions inside the interval add declarations to the same entry, and adding operation plans to an existing entry changes its contents without introducing a new boundary."
- **SB3** (accept-revised) `SPEC.md:3468` second sentence. Verified: selection ignores the permission mode (`src/coding_agent/mod.rs:499-529`, `catalog.rs:1399-1405`). Text: "A run without a selected profile, and a capability its selected profile does not declare, retain explicit `unknown` coverage and manual operation; fixed-policy launch modes use the version-selected profile like every other mode."
- **SB4** (accept-revised) Verified: run `coverage` is `tested` exactly when the selected profile declares detection, and no release gate is checked (`catalog.rs:1439-1443`, `mod.rs:512-516`).
  - `SPEC.md:2943` first clause: "`coverage` is `tested` when the profile selected for the run's backend and native version declares detection, and `unknown` otherwise (§6.9 Profile selection by minimum version); it describes the selected profile's declared detection, not universal prompt detection."
  - `SPEC.md:2518`: "A capability's status is `tested` only when …".
- **SB9** (accept-revised) § Contract error strings (`SPEC.md:2161-2176`). Verified strings: `src/coding_agent/hermes_settlement.rs:77-80,347-349,356-359`. Add two items:
  - "hermes effective home unreadable: `cannot read hermes configuration through 'hermes config path': {cause}`"
  - "hermes effective home not a path: `cannot read hermes configuration through 'hermes config path': printed no absolute file path: {stdout}`"

  Replace 2175-2176 with: "The hermes deletion error is `hermes work for member {id} could not be verified as stopped: {cause}; member was not deleted.` (§ Hermes work settlement)." Each section keeps its own placeholder notation.
- **SB11** (accept-revised: not a contradiction; the tables only lack field names) Verified: the two tables project two fields of one result (`src/runtime/control.rs:158-206`, `src/broker/runtime/inputs.rs:42-115`). Head the `SPEC.md:3200` table "| Durable receipt | `submission.status` | Stage without a cache entry |" and the `SPEC.md:3333` table "| Ordinary receipt state/disposition | `delivery.status` | Method and reason |". After 3206 add: "`submission.status` is the runner's outcome vocabulary for the attempt; the same row's `delivery` object carries the separate `delivery.status` projection in § Delivery result projection."
- **SB14** (accept-revised) The `Teach auto mode` dialog. This is native behavior; the repository holds evidence only for `3. Don't show again` suppressing it.
  - `SPEC.md:2364`: "The dialog consumes typed input; after `3. Don't show again` it does not reappear for that configuration."
  - `SPEC.md:1947-1949` and `:2318`: replace "once per configuration directory" / "at most once per native configuration" with a pointer to that sentence.
  - The broader claim is follow-up F2.
- **SB15** (accept-revised) Verified: the two rules' heading margins differ (`bundled.json:2551,2560` vs `:2673,2682`), and below `W_min` or for a clipped form no rule matches (`SPEC.md:2440,2450`). `SPEC.md:2442`: "At every width at or above `W_min`, a form whose command is unclipped and that fits within `max_rows` matches exactly one of the two request rules, and at any width at most one matches, because their heading margins differ."
- **SB16** (accept-revised) Verified: the pending-prefix bound applies to deferred writes (`DEFERRED_ATTEMPT_BUDGET`, `src/runtime/input.rs:69`) and to command-path writes (`Deadline::new`, `src/time.rs:63-68`). `SPEC.md:2808`: "The bound runs inside the deadline of the write attempt it delays (the runner's five-second ordinary-input attempt budget for a deferred write, or the invoking command's five-second deadline for a prompt, exec or permission answer) and adds no deadline of its own." Naming every five-second budget is rejected: the other uses each sit in the section that owns their budget.
- **SB18** (accept-revised) Verified against `V1__baseline.sql:53-57`.
  - `SPEC.md:2918` inline signature sentence: "The singleton `runtime_database_identity` row (§8 DDL) holds a random 128-bit instance ID, stored as 32 lowercase hex characters and initialized once during baseline initialization."
  - `SPEC.md:3770` second sentence: "`V1__baseline.sql` is the complete schema of this version, and each cafleet version starts from a fresh database file (§8)."
- **SB-min thumb** (accept-revised: not a contradiction) Verified: `█` ends every row of an unclipped form and `▀` appears only in the clipped capture (`bundled.json:2573,2605`; `src/coding_agent/native/opencode.json`). `SPEC.md:2450`: "… clips the command and shows a partial thumb `▀` on the first track row with the rest of the track blank. Each request rule therefore requires its region matcher `unclipped`, which holds only when the full-track thumb `█` ends every row from `# Shell command` through the last command row."
- **SB-min generation** (accept) `SPEC.md:2715`: "advance the manual-input generation (`human_generation`, § Terminal reply classification)". Use `human_generation` at 2852, 3230, 3353, 3388 and 3392 (`src/runtime/transport.rs:18-32`, `input.rs:2822`).
- **SB-min Hermes scope** (accept-revised) `SPEC.md:2518`. Verified: Hermes emits no permission argv, and selection ignores the mode (`src/coding_agent/mod.rs:293`). Lead: "The Hermes profile from `2026.9.24` applies to `hermes chat --cli` under either Hermes launch choice; its request rules and answers come from captures under the operator configuration's `approvals.mode: smart`. An approval prompt shown under `approvals.mode: manual` is detected only when its screen matches a request rule; otherwise it is an `unknown` frame answered manually." The SB4 sentence follows it.
- **SB-min 1928** (accept) Verified: no heading is named "Hermes profile". "(§6.9 Hermes profile)" → "(§6.9 Bundled profiles and native screen fixtures)".
- **SB-min 1963-1969** (accept-revised) Verified: the column values "Other modes" and "Both" are defined nowhere in `SPEC.md`. Name the two profiles in the 1961 lead as the "auto mode profile" and the "other-modes profile" (the names `docs/docs/quickstart.md` uses), and use those names in the profile column.
- **SB-min 2092** (accept-revised) Basis: `.claude/rules/documentation-maintenance.md` keeps `SPEC.md` descriptive only. Keep "unverified" once and delete the "Operators MUST …" sentence, which is advice in a descriptive spec.

#### Consolidations (`P3` / `P5` rows)

Each row keeps every fact in the named owner and turns the other copies into pointers. A unique fact found only in a non-owner copy moves into the owner.

| ID | Decision | Owner | Action |
|---|---|---|---|
| SA4 | accept | §6.3 Schema-version guard | Owns classifier inputs, error strings, preservation and the doctor exemption; move the facts of §6.3 Shared diagnosis and connection reuse into it, keeping that heading with a pointer body; reduce `SPEC.md:85-89` to connection facts plus a pointer |
| SA9 | accept-revised | §6.3 `message wait` | Owns rotation, attempted set and diagnostic reconciliation; 1546 keeps "The next send or a `message wait` rotation (§6.3) retries it"; §6.6's first paragraph keeps only in-memory state, ascending diagnostic order and busy-before-selection |
| SA10 | accept | §6.3 `member capture` | §6.4 Capture snapshot keeps its heading; its body becomes a pointer |
| SA11 | accept-revised | §6.3 Director-only command guard; §6.3 Hermes environment passthrough | 148, 207, 227 and 836-840 become pointers; 815-818 and 1164 stay |
| SA12 | accept-revised | §6.3 `member delete` / `fleet delete` | 1435-1437 becomes "Deletion locking follows §6.3 `member delete` and `fleet delete`"; its revalidation-in-final-transaction and release-on-timeout facts move into `fleet delete`; 418 points to `member delete` for the run-less case |
| SA13 | accept-revised | §6.9 Launch policy and capability registry | Error template stated once with one example; 786 becomes "omission selects the backend's default `auto` mode (§6.9 Launch policy and capability registry)" and drops the per-backend list and "Pass it only …" |
| SA14 | accept | §6.3 Error Messages | 203 and 1124 keep one pointer clause each |
| SA15 | accept-revised | §6.3 `message send`; §6.3 Runtime presentation; §6.9 Delivery method and interaction priority | §6.5 Delivery outcomes keeps its heading, and its body becomes a pointer; 673 points to Runtime presentation; hold/queue rules live in §6.9, and 1494/1544 point there; "submitted" is the one verb |
| SA16 | accept-revised | §6.4 Text-mode projection; §6.5 Push notifications | §6.3 Message Body Truncation keeps its heading, and its body becomes a pointer; 237 keeps its pane-capture clause; the independence sentence stays only at 1520 |
| SA17 | accept-revised | §6.3 `cafleet doctor` | Keep 427-428; drop the 441-442 order sentence; 448 keeps introducing the table |
| SA18 | accept-revised | §6.3 Config-dir resolution; §6.3 Asset installation | Add a "Recorded path" column to the 257 table; 293 table becomes one sentence; 803-825 point to config-dir resolution; 308 keeps only the source → target mapping |
| SA19 | accept-revised | §6.7 Interface; §6.9 Native executable resolution | "`validate_model` and `validate_effort` failures exit 2 (usage) before any side effect" stated once; argv[0] rule once; delete the test-probe sentence (1726-1728); per-backend paragraphs point to the Model selection, Reasoning effort and Backend capabilities tables |
| SA-low | accept-revised | §5.4; §6.5 Push notifications; §6.4 Persisted shape; §6.3 Permission commands | Member kind: 103, 199 point to §5.4. Root-Director reception: 403, 661 point to §6.5 (109 stays). ACK meaning: 107, 759 point to §6.4. Unknown `INTERACTION_ID` ordering: 1198 points to 1065 |
| SN7 | new | §6.3 Permission commands | 3404's first four sentences become "Retained operation keys are validated as §6.3 Permission commands specifies"; 3406 keeps one clause plus the pointer; 3386's first two sentences become "Arguments are validated as §6.3 Permission commands specifies" (the code rejects blank keys, `src/cli/runtime.rs:737`) |
| SB5 | accept-revised | §6.9 Launch policy and capability registry (2229) | §6.7 1919-1924 becomes one clause plus a pointer; the 1914/1915 cells and Claude posture section stay |
| SB6 | accept-revised | §6.9 Launch policy (2207 table, 2231) | 1987-1993 becomes the sandbox fact plus a pointer; 2411 keeps only the detection consequence |
| SB7 | accept | §6.9 Native acceptance and coverage | Delete the duplicate test sentence at 3235; collapse the double blank lines at 3233-3234 and 3415-3416 |
| SB8 | accept-revised | §6.9 Permission response storage (3425) | 3327 becomes a pointer, and its `owner_epoch` = `expected_owner_epoch` fact moves to 3425; drop the 3429 restatement |
| SB10 | accept-revised | §6.9 Profile selection by minimum version | Capability independence and interval provenance stated once (2304/2310); 2247, 2599, 2601, 2630, 2875 and 3433 point there; 2247 keeps its launch-specific facts (with SN4) |
| SB12 | accept | §6.9 Classification and episode identity (2831) | Define "*stable*: two compatible observations at least 100 ms apart"; use it at 2833, 2840, 2842 and 2893 |
| SB13 | accept-revised | §6.9 Bundled profiles intro (2314-2318) | State once that every profile answers only single-request choices, that broadened/persistent/session choices have no choice entry or answer plan, and that captured post-answer lines are decision evidence, not rules; each profile still names its non-single-request choices |
| SB19 | accept-revised | §6.9 exact-fields table (2933); §6.9 Permission response storage | Merge the 2920 summary table into the exact-fields table; 3414 points to Permission response storage; drop the 3427/3429 claim-release restatement; retention reads "for the lifetime of the database file" at 2925 and 3212 |
| SB20 | accept-revised | §6.9 Request-bound permission responses (3375); §6.9 Director consumption and continuation (3373) | 2979 cites the 3375 instruction; 3452 points to 3373; the 3459 row cites 3021's routed-evidence fields. The per-backend runner-guard clauses (233, 1934, 1977, 2020, 2061) are rejected for change: each is already a one-clause pointer carrying its backend's own justification |
| SB-min | accept-revised | as named | 2063-2065/2108-2109: keep one statement (§6.7 Preset file format). 2096-2097 keeps "free of MCP servers" once. 2043-2046 points to the §6.3 Asset installation table, keeping its launch-policy and spawn-argument columns. 2011-2012 points to 2021-2024. 2190/2192 point to § Role eligibility. The frame limit and `protocol_version: 1` are stated once in the 3144 paragraph (§6.9 Input arbitration and the original native answer) |

#### Rejected

| ID | Reason |
|---|---|
| SA20a code alignment | The fix belongs in code (follow-up F1); the SPEC already reports both binary behaviors correctly |
| SB16 budget naming | The only genuinely ambiguous use is 2808, corrected above |

### Decisions — docs/ and README.md (Run 2)

#### Corrections (`DD` rows)

- **D1** (accept-revised) **+ DN2** (new: `mixed-backend-team.md:3,15`) Terminology (§ Terminology for the docs site). Verified: SPEC calls the `message listen` process a listener (`SPEC.md:719,753,755`).
  - Core terms rows (`overview.md:16-17`):

    ```markdown
    | finite receive | A bounded inbox wait (`message wait`) for scripts and manual use that also retries ordinary-member previews | [Message wait](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-wait) |
    | listener | The Director's persistent read-only `message listen` process that streams pending messages as JSONL, with explicit ACK consumption | [Message listen](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen) |
    ```

  - `mixed-backend-team.md:3`: "Create independent coding-agent sessions in tmux or herdr, assign work and receive durable replies through the Director's listener."
  - `mixed-backend-team.md:15`: "Your agent loads the Director instructions, starts its listener, creates members and assigns each after its ready. The Director consumes broker events and checks evidence while preserving the host's supplied continuation and user gates."
  - `members.md:49` step 1: "Read the full inbox record from the listener or `message poll DIRECTOR_ID --json`."
  - `mixed-backend-team.md:54`: "Suppose the returned ID is `3`. Read its ready message, ACK its actual message ID, then dispatch its assignment:"
  - Every other occurrence of the replaced terms takes the decided term, through the D2, D10, D15 and D18 rewrites where they overlap.
- **D8** (accept-revised) Verified: the overlay binds codex `{skill_loader}` to absolute-path reads (`skills/cafleet/reference/coding-agents.md:94`), and the overview list omits Hermes. `overview.md:76-80` § Invocation syntax body: "Each prompt is a plain request; your coding agent loads the `cafleet-design-doc` skill through its own skill mechanism, which [Coding agents](coding-agents.md) lists for each backend."
- **D12** (accept-revised) **+ D20b** (accept-revised) **+ DN4** (new: the table has no anchor) **+ DN7** (new) Verified: the classes match `SPEC.md:2211-2222` and `src/coding_agent/mod.rs:241-280`; `members.md:39` lists 3 of the 6 `manual` modes; an explicit Codex `on-request` can be answered by `approvals_reviewer` without a form (`SPEC.md:2226`). Add a `## Launch modes` heading directly above the launch-mode table in `coding-agents.md` (new anchor `#launch-modes`). Replace `members.md:39` with: "Spawn members with the omitted `--permission-mode`, which selects the backend's default `auto` mode. Pass `--permission-mode MODE` only when the user asks for another mode for that member; [Launch modes](coding-agents.md#launch-modes) classifies each backend's modes `auto` or `manual`, and the [policy contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#launch-policy-and-capability-registry) owns choices, defaults and coverage. Member launch mode is independent of the Director's decision policy. A `manual` mode shows a native permission form for each operation its rules do not settle; OpenCode's `cafleet-ask` preset does so for every shell command outside its allowlist instead of denying it. An `auto` mode can still show a native form when its reviewer escalates an operation, as Hermes's guardian does, or, on Claude, before a file tool reads outside the working directories; that form follows the same request flow. Once Claude's `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`, later outside reads in `auto` proceed without asking."
- **D13** (accept-revised) **+ DN1** (new) `coding-agents.md:27`. SPEC §6.7 Role requirements requires goal lifecycle controls as well as streamed output. Replace with: "Run the root Director on a backend whose Director verdict is `yes`, and spawn members on backends whose Member verdict is `yes`; a backend whose Director verdict is `no` runs only as a member. [Role eligibility](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) records each backend's verdicts with their evidence. A Director backend needs native goal lifecycle controls and must deliver the output of the Director's background listener to the session while the listener runs, at the latest at the session's next turn boundary; [Role requirements](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-requirements) defines both requirements and the probe."
- **D14** (accept-revised) Verified: OpenCode rejects `--effort` (`src/coding_agent/mod.rs:162`). "Family word" is undefined on the site.
  - `coding-agents.md:80-91`: "A bare Claude or GPT model reference selects Claude Code or Codex on every fleet, including an OpenCode fleet, and a provider-prefixed reference such as `opencode/glm-5.3` selects OpenCode. Naming OpenCode for the member selects OpenCode for that model, and a bare name offered only by OpenCode selects OpenCode when it uniquely matches a catalog ID. For an OpenCode model named without a provider, the Director uses the OpenCode Go form when your configured providers offer it, otherwise the Console form. The Director asks you to resolve an unknown, ambiguous or mismatched reference, or an OpenCode model your providers do not offer. The installed Director policy owns the exact resolution rules." Then the paragraph "Use the selected backend's role defaults and catalog as the source of truth."
  - `coding-agents.md:115-118`: "Evidence of insufficient capability can trigger a replacement with a strictly stronger model on the same backend, a bounded number of times per task. The installed Director policy owns evidence, handoff, limits and selection; a user-pinned model requires an operator decision before replacement."
- **D18** (accept-revised) Verified: "host", "continuation" and "native inbox tools" are not Core terms. `README.md:5`: "CAFleet multiplexes independent coding-agent sessions in tmux or herdr and coordinates them through durable messages: collaborative coding across multiple coding-agent backends, with full code transparency. Use its CLI and skills to delegate research, implementation and review, then collect durable replies while your session continues the task." `README.md:31`: "- [Members](https://himkt.github.io/cafleet/concepts/members) — member lifecycle, command routing and permission handling."
- **D20a** (accept-revised) Verified: OpenCode panes run `opencode mini --standalone` (`src/coding_agent/mod.rs:349`). `coding-agents.md:52-53`: "Each backend honors a leading-`!` shell shortcut on the coding agent's input line, including the minimal `mini` interface that OpenCode member panes run; [backend capabilities](https://github.com/himkt/cafleet/blob/main/SPEC.md#backend-capabilities) compares their sandbox and shell-command posture."
- **D20c** (accept-revised) Basis: SA2. `members.md:86`: "`member show` reads recorded runtime state without a socket probe; every ordinary member has a runtime record, and the root Director has none ([runtime presentation](https://github.com/himkt/cafleet/blob/main/SPEC.md#runtime-presentation)). `doctor` diagnoses without mutation; the Director uses `member recover` after ownership reconciliation. Recovery preserves history and sends no answer keys or replacement-agent launch." This text merges into § Inspection and recovery under DN5.
- **DN3** (new) `quickstart.md:36`: not-applicable cells use `—` (Rendering conventions). D7 carries the table.
- **DN6** (new) `storage.md:19-23` first setup-table row: "Missing file, or an empty SQLite file with identity zero and no user tables". Setup creates and initializes a missing file (`src/cli/setup.rs:66-93`).
- **DN8** (new) `members.md:25`: "`member capture` returns the last 20 lines by default; use `--lines` for more when that is insufficient." The default is exactly 20 (`src/cli/member.rs:190`).
- **DN9** (new) `quickstart.md:107-110`. Basis: SB14. "Claude Code 2.1.302 in auto mode can show a `Teach auto mode about your environment?` dialog after a member's turn. Answer it with `Don't show again` in an interactive session of that version that uses the members' configuration directory; after that answer it does not return."

#### Consolidations (`P3` / `P5` rows)

| ID | Decision | Owner | Action |
|---|---|---|---|
| D2 | accept-revised | `members.md` § Deliver generated input | Move the held-preview fact from `mixed-backend-team.md:60` into the owner ("A preview is held rather than queued while an earlier receipt is uncertain or the runner's observer, ownership or storage is unavailable; a later `message send` or finite receive retries a held preview."); shorten `members.md:13` and `mixed-backend-team.md:60` to a link plus one clause |
| D3 | accept-revised | `members.md` § Consume a runtime event, step 4 (and the § Member permission events lead) | `mixed-backend-team.md:88` becomes a link plus its example-specific facts; `coding-agents.md:40` gains only the link (D11) |
| D4 | accept-revised | SPEC `#director-only-command-guard` | `quickstart.md:239-256`: "CAFleet needs no `approvals.deny` entries; keep every `cafleet …` glob out of that list." followed by the Hermes-session consequence and the inspect/rewrite commands; the six-glob list goes |
| D5 | accept-revised | `members.md` § Command routing | Keep `quickstart.md:69-71` and `:185`; delete `:164-167`; `:226-229` keeps only why the passthrough matters |
| D6 | accept-revised | `quickstart.md` Claude Code entry table | Drop the behavior table (59-64), carry its one unique fact (which profile matches which Director mode) in prose, and state "`ask` entries outrank allow entries" once |
| D7 | accept-revised | `quickstart.md` § Configure | Remove the Reference column and link each backend name to its sub-section; the OpenCode cell names the `~/.config/opencode/agents/` directory |
| D9 | accept-revised | `members.md` § Teardown | `overview.md:26` ends with "The Director owns [teardown](members.md#teardown)."; `mixed-backend-team.md:114` keeps the command plus a link; `:126` keeps the rerun instruction |
| D10 | accept-revised | SPEC `#message-listen`, `#director-consumption-and-continuation`; skill recipes | `mixed-backend-team.md:31,64,76-80,124` become operator-level sentences plus SPEC links; agent procedure leaves the page; keep the command examples |
| D11 | accept-revised | `members.md` § Choose a native policy | Cut the restated profile/coverage sentences from `coding-agents.md:40`; `contributing.md:58` keeps only the contributor instruction plus a SPEC link |
| D15 | accept-revised | `mixed-backend-team.md` § Receive results | Delete `coding-agents.md` § Complete member prompts and § Director wake (the only inbound link is `members.md:57`, which retargets to `#receive-results`); merge overview's two reception mentions into one link |
| D16 | accept-revised | SPEC creation, persistence and runner sections | `members.md:7,61` and `storage.md:9,49` keep user-observable effects plus a SPEC link; delete `storage.md:25` |
| D17 | accept-revised | `members.md` § Command routing | Keep routing, exec, capture, prompt, the retry sentence (`mixed-backend-team.md:110` relies on it) and the worked-example link; permission sentences move under § Member permission events, where they already exist |
| D19 | accept-revised | `.claude/rules/code-quality.md` § Test placement; `.claude/rules/documentation-maintenance.md` | `contributing.md:20` and `:54` become one clause plus a link; keep "state intended behavior directly", which no repository rule owns |
| D20d | accept | `coding-agents.md` intro | Delete "The flag means slightly different things per command:" |
| DN5 | new | `members.md` § Inspection and recovery | Merge § Inspect and recover into it; nothing links `#inspect-and-recover` |

### Decisions — skills/ and rules (Run 3)

#### Corrections (`DD` rows)

- **K1** (accept-revised) Verified: the facts match `SPEC.md:2318,2364,2911`; the overlay rule places backend facts in `coding-agents.md`. Backend facts move out of `director.md` into Note → applies at rows.
  - New `coding-agents.md` § claude row: "In `auto`, Claude Code 2.1.302 can show `Teach auto mode about your environment?`; after `3. Don't show again` it does not reappear for that configuration. It arrives as an `interaction_opened` event with classification `question` and has no answer plan: ask the user to answer it on the member's native surface, and send no answer yourself." Applies at: `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [observed interactions](../roles/director.md#consume-observed-interactions).
  - New § codex row: "`approve-for-me` on Codex 0.163.0 shows no startup dialog. After three consecutive automatic denials Codex interrupts the turn, so the member stops without routing a request; the Director receives one `automatic_refusal_observed` per denial." Applies at: `{permission_flags}`; [Member launch policy](../roles/director.md#member-launch-policy); [automatic review events](../roles/director.md#consume-automatic-review-events).
  - `director.md:87`: "A backend's `{permission_flags}` note names any first-launch dialog its default mode shows. A dialog with no answer plan is a user question: ask the user to answer it on the member's native surface, and send no answer yourself. Any other first-launch dialog, such as a folder-trust prompt, follows the existing held-interaction procedure."
  - `director.md:408` Procedure cell: "A backend can end the turn after repeated automatic denials (its `{permission_flags}` note names the threshold), so the member stops without routing a request. Treat the events as one pending item and ask the member through the broker whether the refused operations are still needed; the broker message resumes it."
  - `director.md:395` step 7 last sentence: "A deny can end the member's turn (its backend note names the native signal); once nothing ran, continue that member with a message, which its runner delivers into the idle composer."
  - `director.md:385`: delete "; OpenCode `cafleet-ask` opens a native permission form for unmatched shell commands" (`coding-agents.md:316` carries it).
  - `director.md:72`: "Spawn preflight failures — a missing backend binary, a missing or unreadable preset, a failed backend configuration preflight — exit 1 before registration; [Error messages](../reference/cli.md#error-messages) owns their exact text."
- **K2** (accept-revised) The CLI-guarded set (`src/cli/mod.rs:74-85`) and Director policy ownership are distinct concepts.
  - `member.md:74-76`: "Take literal IDs from `FLEET ID:`, `YOUR MEMBER ID:` and `DIRECTOR MEMBER ID:`; pass them explicitly. CLI validation surfaces missing identity. Use Send, Poll, ACK and `message show` on your own inbox; every other `cafleet` command is Director-owned. The [guarded subset](../reference/cli.md#director-only-command-guard) — explicit pane input, permission answers, input reconciliation and run recovery — refuses inside a member with a `Director-only command` error; the rest run without that refusal and remain the Director's. Send a need for any Director-owned command to the Director with the routing message in [Reconsider, then route](#reconsider-then-route), naming the command and its purpose."
  - `director.md:383`: "The Director owns `permission inspect`/`respond`, `input reconcile` and `member recover`; the [CLI guard](../reference/cli.md#director-only-command-guard) refuses all of them except `permission inspect` inside a member. Use `event show` and `input show` to verify durable identities, preserving ownership, snapshot-bound evidence and uncertainty."
  - `SKILL.md:53`: "Pass full database-assigned integers literally. Angle-bracket examples denote values to substitute, and environment variables supply no member identity. Literal IDs preserve host permission-pattern matching."
- **K3** (accept) `director.md:419`. The code has 8 causes (`src/broker/runtime/mod.rs:583-588`). Replace the parenthetical sentence with "Read its recorded [`uncertain_cause`](../reference/cli.md#input-show-and-input-reconcile) with pane/native evidence before reconciliation."
- **K4** (accept-revised) Verified: only task-scope resolution returns `<unset>` (`guidelines.md:88`), and the Director omits the BASE line for it (`director.md:253`). Members never receive `<unset>`, and the `audit-disabled` status is a member-to-Director message.
  - The `member.md` § BASE table keeps exactly the rows "Absolute `BASE:` supplied" and "`BASE:` omitted"; delete the `<unset>` and sentinel rows and the guard sentence.
  - `director.md:253`: "When BASE is `<unset>`, omit the BASE line, skip audit files and use a compact path-by-reference positional `PROMPT` under about 2 KB; each such member reports `audit-disabled no BASE in spawn prompt` once. Explicit output targets remain available. Unguarded sentinel construction fails with `Error: BASE is <unset>; refusing to fall back to /tmp`." `:249` keeps the single guard sentence.
  - `director.md:261`: "Only task-scope absolute-path containment can return the case-sensitive `<unset>` sentinel; follow [Scratch and audit files](#member-create--scratch-and-audit-files) for it."
  - `guidelines.md:78` second sentence: "Task-scope resolution owns containment; the Director's [scratch and audit procedure](../../cafleet/roles/director.md#member-create--scratch-and-audit-files) owns guarded writes and disabled-audit behavior. The invoking workflow owns document discovery and its explicit output target."
  - `guidelines.md:91`: "The resolver applies containment to the normalized folder without writing files."
- **K9** (accept) Verified: every backend section already classifies its modes (`coding-agents.md:62,127,155,315-316,333,357`). Delete the `director.md:80-85` table. `director.md:78`: "Spawn every member with the omitted `--permission-mode`, which selects the backend's default `auto` mode. Pass `--permission-mode` only when the user explicitly requests that mode for that member; [member create](../reference/cli.md#member-create) lists each backend's choices and each backend's `{permission_flags}` note classifies them. An `auto` mode is native automatic review or a fixed allow/deny policy; a `manual` mode asks a human on the native surface for operations its rules do not settle." The `director.md:68` Notes cell: "Per [Member launch policy](#member-launch-policy)." `member.md:3` first sentence: "You are an ordinary `cafleet member create` member under your recorded launch policy ({permission_flags})."
- **K12** (accept) Verified: "step 1/3" sits under a table whose rows are numbered 1-5 (`director.md:94-103`). `director.md:103`: "(backend-resolution step 1)" and "(backend-resolution step 3)". Delete from `coding-agents.md:349` the two sentences selecting `hermes` and relaying cost-efficiency choices; Model selection owns them.
- **K13** (accept-revised) Verified: `drafter.md:47` recognizes only the literal, and `create.md:60` never gives it. `create.md:60`: "After user approval, send the Drafter exactly `ready (doc) — user approved; finalize`: incorporate accepted decisions, resolve coordination markers, set Status to Approved and keep implementation progress accurate. Verify the final artifact and apply host Git/publication rules. Honor abort without marking the design approved."
- **K14** (accept-revised) Verified: `drafter.md:23` (at least 3 categories) and `:38` (do not re-ask) conflict for a detailed request. The ask-before-drafting gate stays; the three `create.md:36` IMPORTANT lines and the start cue stay verbatim.
  - `drafter.md:11`: "- **Ask clarifying questions before drafting (non-negotiable).** You MUST send at least one clarification round to the Director via `cafleet message send`, and receive the answers, BEFORE creating any design document file. Build the round with the Structured Question Framework below."
  - `drafter.md:23`: "Cover at least 3 categories from the framework below, or every relevant category when fewer than 3 apply. In each covered category, ask every consequential question the request leaves unanswered; where the request already answers it, state your understanding for confirmation instead (e.g., "I want to confirm my understanding: [summary]. Is this correct? Any adjustments?"). Skip a category only when the request makes it entirely irrelevant (e.g., skip UI/UX for a backend-only feature)."
  - `drafter.md:38`: "Never re-ask a point the request or an earlier answer already settles. After receiving answers, send at most one focused follow-up round, only when critical ambiguities remain."
  - `create.md:41`: "For fresh work, have the Drafter inspect relevant source and send one clarification round before drafting: questions for consequential unanswered requirements and confirmation of the requirements it understood. Relay it through the executing backend's user-decision surface, preserve answers and route them back faithfully. Resume and review-only work start from existing content and annotations."
- **K15** (accept) Verified: the Drafter's own step 2 is Draft (`drafter.md:43`). `drafter.md:17`: "**Coordination Protocol**: Clarification questions and answers are free-form ([Payload exemptions](../../reference/coordination.md#payload-exemptions)); send long payloads through `--file` and retrieve answers with `--json`. Every message from your first draft onward follows the **verb + pointer + `COMMENT(role)`** schema in [coordination](../../reference/coordination.md) — a single-line `<verb> (<pointer>)` body with substantive content in inline `COMMENT(role)` markers." In `drafter.md:42`, the parenthetical becomes "(free-form body under Payload exemptions)".
- **K16** (accept-revised) Verified: `execute.md:31-32` allows a Programmer-only team, and the checkbox edit is assigned twice (`execute.md:60`, `programmer.md:12,63`). `programmer.md:11`: "- **Implement code that passes all tests.** In a TDD team the Tester has already written unit tests for each step; write implementation code that makes ALL tests pass while faithfully following the design document specification." `execute.md:60` third sentence: "Verify the Programmer's checkbox, timestamp and Progress update for each completed task."
- **K19** (accept-revised) Verified: no Phase C exists, Phase D stands alone, and `programmer.md` has no Phase 1. `execute.md:62` heading: "### Verification (Director) — conditional". `coordination.md:98`: "Step 4 code-review feedback in the execute workflow". `coordination.md:109`: "`COMMENT(reviewer): [GAP] Step 4 lacks a verification entry.`" `coordination.md:12` Boundary: "First substantive payload after ready; later verification reports use ordinary paired markers." `programmer.md:28,39,47` headings: "### FIXME Resolution", "### Resumption (when document is partially complete)", "### Implementation (TDD)". Update any inbound link to the renamed headings in the same run.
- **K20** (accept-revised) **+ KN3** (new) **+ KN9** (new) Verified: `coordination.md:132` calls anchorless statuses informational, while `director.md:432` and `create.md:89` dispatch on the handshake `ready`. The § Anchorless Status second bullet becomes: "They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses), and the Director treats them as informational, except for the spawn handshake below." Add this bullet: "The spawn handshake `ready` (optionally `ready: <recap>`) is an anchorless status, distinct from the `ready (<pointer>)` verb. It is the member's startup gate under the Director's Spawn Protocol, not a deliverable signal, and it never carries or replaces a role's first substantive message — the Drafter still sends its clarification round as a separate message and drafts nothing before the answers arrive." In the verb table, `addressed` direction becomes "Member → Director" and `approved` direction becomes "Reviewer → Director". `member.md:25` names the handshake inline as "a short status phrase with no `(<pointer>)`, not the `ready (<pointer>)` verb" (KN9 keeps the core role free of design-doc links).
- **K21** (accept) Verified: the schema lives in § Verb Vocabulary, § Pointer Forms and § Message Format (`coordination.md:18-83`). `programmer.md:18`, `tester.md:18`: "**Coordination Protocol**: Follow [coordination](../../reference/coordination.md) for the verb + pointer schema, the `COMMENT(role)` role taxonomy and marker rules." `create/roles/reviewer.md:23`: "See [coordination](../../reference/coordination.md) § *COMMENT(role) Marker* for the marker format and § *Pointer Forms* for placement."
- **K22** (accept) Verified: `coordination.md:51` defines doc-top placement. `drafter.md:44`: "If you encounter a spec ambiguity you cannot resolve unaided, write a `COMMENT(drafter): <issue>` marker AND send `blocked (<pointer>)`, placing the marker at that pointer under the pairing rule in coordination.md (`paragraph-<HeadingPath>` for a local ambiguity, `doc` for a doc-wide one)." In `tester.md:32`, "near the top of the doc body" becomes "at the `doc` pointer".
- **K23** (accept-revised) **+ KN4** (new) Verified: fallback prompt/exec also send Escape without the clause (`cli.md:174,190`), so the audit's "otherwise treat it as real" is rejected.
  - `member.md:54`: "**Delivery-dismissed prompts.** A delivery that falls back to Escape can dismiss an approval or question the runner did not detect. When a rejection is followed by a preview ending with the Escape-dismissal clause, that rejection came from the delivery, not the user: reissue the tool call or re-ask the question if it is still needed. A preview with only the resume clause leaves the rejection standing. After a Director prompt or native command, follow that input on whether to retry; with no further input, wait. Explicit operator/Director stop instructions prevail."
  - `member.md:33`: "A `[cafleet msg ID from SENDER TIMESTAMP]` line carries the body or a `message show ID --json` pointer. Act and ACK that ID."
  - `member.md:66`: delete "A runner `automatic_refusal_observed` event only corroborates your request and supplies no missing dispatch evidence." (`director.md:406` owns it). Keep "A separately dispatched native command is a new action, distinct from answering a pending permission request.", which no other skill file states.
  - New `coding-agents.md` § claude row: "A delivery-dismissed prompt can show `The user doesn't want to proceed with this tool use.`; only an Escape-dismissal preview establishes that the delivery caused it." Applies at: [Command execution](../roles/member.md#command-execution).
- **K24** (accept-revised) Verified: the heading at `director.md:478` says halt while `:480` says continue, and "Stage 2" has no Stage 1 (`:486`). Keep the `#stall-response` anchor (`coordination.md:135` links it).
  - `director.md:478-480`: heading "#### Stop signals", body "Explicit cancellation, a required user decision, a concrete unrecoverable failure or a workflow gate halts only the work it affects. Track that state and continue independent authorized work when available."
  - § Stall Response body (no stage numbering): "Start with your inbox and assignment/artifact state, then take a read-only `cafleet member capture <member-id> --lines <depth>` for complete backend UI evidence; capture is safe while the member works. A quiet or ambiguous pane calls for inspection rather than an inferred failure: timeouts, idle duration and unread counts alone establish no failure, so check the assignment's expected ready/reply point."
  - Delete the duplicates at `director.md:304` (last sentence) and `:315`.
- **K26** (accept-revised) Verified: the renderer is Rust and validates before registration (`src/cli/member.rs:424,897-955`). `director.md:205`: "the identity lines carry the CLI's four identity placeholders, rendered to literals by `cafleet member create` at spawn time". Delete `director.md:193`. `director.md:234` appends "[member create](../reference/cli.md#member-create) owns substitution syntax and errors." `SKILL.md:65`: "Supply exactly one positional `PROMPT` or `--file PATH`. [CLI environment variables](reference/cli.md#environment-variables) owns supported environment settings."
- **K27** (accept-revised) Verified: the design-doc skills apply in every repository using CAFleet (`guidelines.md:80`). `programmer.md:56`: "Read the whole commit range rather than filtering by a test pathspec: inline test modules (for example Rust `#[cfg(test)]` modules) live inside the source files they cover, so a pathspec would hide them. Read the test content to understand the expected behavior and interfaces." The last sentence of `programmer.md:20`, `tester.md:20` and `verifier.md:22`: "You run no subagents and launch no other coding-agent session (no backend binary and no `cafleet member create`); `cafleet` broker commands remain part of your protocol."
- **KN1** (new) `cli.md:360`: "`uncertain_cause` records the transition to uncertainty and survives reconciliation; it is null for input that never became uncertain and for `in_progress` input left by runner loss." There is no pre-V8 input: the schema is one V1 baseline, and other-version files are refused (`src/db.rs:839-869`).
- **KN2** (new) Verified: previews have their own limit (`cli.md:164`), and `max_text_len` is read only by text formatters (`src/cli/message.rs:173-225`). `SKILL.md:83`: "Supply one positional body or `--file PATH`; use file input for long/multiline text. Text output truncates bodies at `CAFLEET_MAX_TEXT_LEN`; `--json` returns complete payloads." The `cli.md:51` Meaning cell: "CLI text-output body codepoint limit". Previews use their own 4000-scalar limit.
- **KN5** (new) Basis: `execute.md:31-33` allows a team without a Tester. `programmer.md:3`: "… a Tester in a TDD team (who writes unit tests for each step), and optionally a Verifier …". `verifier.md:3`: "… a Programmer (who implements code) and, in a TDD team, a Tester (who writes unit tests)."
- **KN6** (new) Verified: `drafter.md:47` replies `addressed (doc)` on a marker-free finalize, against `coordination.md:26,31`. `coordination.md:26` Meaning: "I resolved a pre-existing marker (a `COMMENT(role)` marker or a Director-arbitration note) at the pointer, or completed the Director's finalize assignment." `coordination.md:31`: "… if yes, or if you are completing a finalize assignment, use `addressed`; otherwise use `complete`."
- **KN7** (new) Verified: everywhere else in the file, "catalog" means the Model catalog. `coding-agents.md:7`: "explicit `member create --permission-mode MODE` selects one of that backend's [permission choices](cli.md#member-create) while preserving argv/sandbox boundaries."
- **KN8** (new) Basis: the overlay rule keeps base roles backend-neutral. `tester.md:29`: "Check the project-instructions file your harness loads for testing conventions or preferences."
- **R1** (new; required by D1) `.claude/rules/documentation-maintenance.md:44-45` examples: ("the broker", "the listener", "the finite receive"). `:124` echo example: "a finite receive bounds its native task lifetime". This keeps the rule's examples consistent with the Core terms decided in D1.

#### Consolidations (`P3` / `P5` rows)

| ID | Decision | Owner | Action |
|---|---|---|---|
| K5 | accept-revised | `cli.md` § `message listen` | `director.md:449-453` becomes the code block plus "Run one persistent receiver through the backend recipe; [message listen](../reference/cli.md#message-listen) owns the lease, record shapes, ordering, replay, failures and closure. Native tools own receive handles; broker state owns delivery and consumption." |
| K7 | accept-revised | `director.md` § Receiver recovery | Its last row reads "Native stream tool unavailable or denied at runtime → state the reception limit and use manual `poll`/finite `wait` within host permissions"; `director.md:430` and `coding-agents.md:11,76,372` link it; "both" becomes "the Director requirements" |
| K8 | accept-revised | `director.md` § Director wake | Goal-authority preservation stated once at `:437`; drop the restatements at `:468`, `:480` and `coding-agents.md:60,76,124,139,374`; backend recipes keep their own nouns (Codex token budget, Hermes `/goal` `gate`/`wait`) |
| K10 | accept-revised | `cli.md` § `member create` | Keep both examples; `director.md:61-70` becomes a Director-decision table (flag → pass when) plus a link; delete the cleanup restatements at `:49`, `:197` and `:255`; keep the "command too long" size signal and `-d` focus |
| K11 | accept | `cli.md` § `member delete` | `director.md:290,296` become the code block plus "Delete a member before creating its replacement. A busy pane or an unverified Hermes stop exits 1 and keeps the member: inspect the cause and retry. Deleting the root Director is refused; use `cafleet fleet delete`. [member delete](../reference/cli.md#member-delete) owns teardown order, locking, settlement, exit codes and output." |
| K17 | accept-revised | spawn frame + `cafleet/SKILL.md` § Required reading (reader rule); `{skill_loader}` (loader); `director.md:17` (recovery timing) | Shorten `member.md:7`, `cafleet-design-doc/SKILL.md:19,53,63`, `create.md:7,9` and `execute.md:7,9`; keep the two `cafleet-design-doc/SKILL.md` required-reading blocks, which serve different readers |
| K18 | accept-revised | role files (cadence); `cli.md` send/wait (retry) | `SKILL.md:95` describes what poll returns and defers cadence to role files; `SKILL.md:87`: "a later send or `message wait` retries pending ordinary-member notifications" |
| K25 | accept-revised | each backend section; `cli.md` § `member delete` | Merge the claude `:59`/`:60` and codex `:125`/`:126` row pairs; the Hermes PATH fact stays only in the `:361` row; the Hermes delete row keeps only operator recovery plus a link |

#### Rejected

| ID | Reason |
|---|---|
| K6 | Under the lossless rule (`director.md:245`), each workflow's per-role delta table is the authoritative inventory of its roles' hard lines, so the repeated poll-handling and Bash-protocol lines in `create.md:36` and `execute.md:53` are intended |
| Audit table-cell-cap argument (K6, K25) | The two-sentence cell cap binds `docs/` pages only; K25's merges stand on redundancy alone |

### Follow-ups (out of scope)

| ID | Finding | Evidence | Suggested follow-up |
|---|---|---|---|
| F1 | `member create --fleet-id` with an unknown fleet prints `Fleet 'X' not found.` and exits 2; `fleet show`/`fleet delete`/`member list` print `fleet 'X' not found.` and exit 1 | `src/cli/member.rs:371-372`; `src/broker/fleets.rs:34-37` | Separate design doc: use `require_fleet` in `member create`, then update `SPEC.md:1171` and `skills/cafleet/reference/cli.md:457` |
| F2 | Whether `2. Not now` or cancel suppresses the `Teach auto mode` dialog is unestablished | `bundled.json:435-458`; no reappearance capture | Re-capture on Claude Code before broadening SB14's sentence |
| F3 | §6.9 Native acceptance and coverage is a list of test instructions, and §6.7 Claude permission posture describes "recommended" profiles; both conflict with the descriptive-only SPEC rule | `SPEC.md:3466-3504`, §6.7 | An affirmative or rewrite cycle; a voice change is outside simplification |

### Run-discovered corrections

The Director appends one row per approved `DD-new` row before applying that run's edits.

| ID | Run | Location | Evidence | Decision and replacement |
|---|---|---|---|---|
| RD1 | 1 | `SPEC.md:1158` (SN2 second clause) | `SPEC.md:222,794,908,1124,1126,1166-1176` quote messages with `Error: `, while `:411` and `:943` quote without it; `src/main.rs:45` prints the prefix | SN2's "command sections quote messages without that prefix" is false. Append instead: "The executable prefixes the first stderr line of every application error with `Error: `; a message this document quotes without that prefix prints with it." |
| RD2 | 1 | `SPEC.md:3206` (SB11 appended sentence) | `SPEC.md` has no "Delivery result projection" heading; the `delivery.status` table is under `##### Delivery reporting` (`SPEC.md:3294`) | SB11's pointer becomes "§ Delivery reporting"; the rest of the sentence is unchanged |
| RD3 | 2 | `docs/docs/concepts/coding-agents.md:49-51` | The page lists the member environment without `CAFLEET_MEMBER_ID`; `members.md:21` and `SPEC.md` § Member environment state it is always set; `src/coding_agent/mod.rs:120-131` always emits it | "The member environment carries `CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and the selected backend's config-location variable when the creator sets it; `CAFLEET_MEMBER_ID` only marks the process for the [Director-only command guard](members.md#command-routing)." |
| RD4 | 2 | `docs/docs/quickstart.md:180-181` | "the default XDG location" implies `XDG_DATA_HOME` resolution; `SPEC.md:141` and `storage.md:7` state the fixed `~/.local/share/cafleet`; `src/config.rs:19-32` builds the path from HOME and reads no `XDG_DATA_HOME` | "write access to the directory of the selected database file, the default `~/.local/share/cafleet` unless `CAFLEET_DATABASE_URL` selects another." |
| RD5 | 3 | `skills/cafleet/reference/cli.md:319` | The page quotes `exec delivery to member 110 is uncertain: …` and "preserve exit category"; `cli.md:268` and `SPEC.md` § `member exec` give `native command delivery …` and exit 1; `src/cli/member.rs:703,854` | "Uncertain errors exit 1 with known receipt/context, including `native command delivery to member 110 is uncertain: manual_input (method=profile)` and `input_id=INPUT_ID`." |
| RD6 | 3 | `skills/cafleet/reference/cli.md:315` | "only the root Director has `runtime:null`" vs the run-less ordinary handling at `cli.md:118,250,261` and `SPEC.md` (SA2); `src/broker/members.rs:15-24` | "Member creation records every ordinary member's run; `runtime` is null for the root Director and for an ordinary row without a run, which CAFleet never writes but send, prompt, exec, recover and delete handle." |
| RD7 | 3 | `skills/cafleet/reference/cli.md:450-451` | Command cells "(any fleet/member/message command)" vs `SPEC.md` (SA3) "(any command except `setup` and `doctor`)"; `src/cli/mod.rs:119-170` opens every other command through the guarded connection | Both cells become "(any command except `setup` and `doctor`)" |
| RD8 | 3 | `skills/cafleet/roles/director.md:315` | "the default approximately 20 lines" vs `cli.md:301` "(default 20)"; `src/cli/member.rs:190` `default_value_t = 20` | "Read the default 20 lines directly and capture more when insufficient." |
| RD9 | Fix 1 (final review round 1) | `SPEC.md` § Asset installation intro | The SB-min consolidation removed §6.7's "installed … by `setup`'s asset installation (§6.3), overwriting any existing target" (`c7cdc830` `SPEC.md:2039`) without moving the outcome into the owner's intro; `src/assets.rs:111-114` removes the existing target, then writes the embedded bytes | "Setup installs every embedded preset of the agent, in this order, replacing any existing target:" |

### Verification

| Check | When | Pass condition |
|---|---|---|
| clean-docs pre-apply review | Each run | `approved (findings)`; every accepted decision covered; every `DD`/`DD-new` row checked against this document and its evidence |
| clean-docs post-apply review | Each run | The diff equals the approved rows; contract surfaces preserved |
| `mise //:test`, `mise //:lint` | Each run | Both pass (embedded-asset tests cover skill edits) |
| `mise //docs:build` | Runs 2 and 3 | Passes |
| Rendered-pipe check | Run 2, every changed table cell holding a literal `\|` | Rendered HTML keeps the cell intact |
| Anchor check | Each run | For every heading removed or renamed in the diff, `git grep -n "#<old-slug>" -- ':(exclude)design-docs/**' ':(exclude)researches/**'` returns nothing; every `SPEC.md#<slug>` from `git grep -o -h "SPEC.md#[a-z0-9_-]*" -- docs/docs README.md` matches a `SPEC.md` heading slug |
| Scope check | Final | `git diff main --stat` lists only `SPEC.md`, `docs/docs/**`, `README.md`, `skills/**`, `.claude/rules/documentation-maintenance.md` and this design doc |
| Terminology check | After Run 2 | The Success Criteria grep returns nothing |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

The executing Director carries out every documentation edit through the clean-docs skill's **simplification** workflow (`.claude/skills/clean-docs/SKILL.md` § Simplification and its Per-run process), applying § Execution model and § Orchestration. It spawns no execute-workflow Programmer or Tester. The execute workflow's final gates map to Implementation Step 5 (execute Step 5 Reviewer Review Loop) and Step 6 (execute Step 6 User Approval, Step 8 Finalize for the Status update, Step 7 Push & Create PR).

Every run follows the same per-run procedure:

1. Resolve the run's BASE as `researches/clean-docs-simplification-<UTC-compact>/` (clean-docs § BASE convention), run `cafleet doctor`, then `cafleet fleet create --name <name> --coding-agent <backend> --json`. Arm the single native listener for the returned Director ID.
2. Spawn the run's scanners and one reviewer with the clean-docs spawn deltas, adding the three § Execution model context lines (`DESIGN DOC`, `DECISIONS`, `AUTHORIZED ROWS`) verbatim.
3. After clean-docs teardown (Per-run step 8), verify the root-only roster, delete the fleet and retire the listener with observed source-process exit before the next run creates its own fleet.

### Step 1: Prepare

- [x] Create branch `docs/0000212-docs-spec-skills-simplification` from `main`. <!-- completed: 2026-11-03T12:15 -->
- [x] Run `mise //:test`, `mise //:lint` and `mise //docs:build` on the unchanged branch and record the baseline. <!-- completed: 2026-11-03T12:15 -->

### Step 2: Run 1 — SPEC.md

- [x] Per-run procedure items 1-2: create the Run 1 fleet, arm the listener, and spawn one scanner owning `SPEC.md` plus a reviewer, with `DECISIONS: § Decisions — SPEC.md (Run 1)`. <!-- completed: 2026-11-03T12:16 -->
- [x] The scanner writes findings covering every SA/SB/SN entry labeled other than `reject`, plus its own P3/P5 and `DD-new` rows; the Director checks coverage, then sends `ready (findings)`. <!-- completed: 2026-11-03T12:29 -->
- [x] After `approved (findings)`, record each approved `DD-new` row in § Run-discovered corrections and any code-side defect in § Follow-ups. <!-- completed: 2026-11-03T12:38 -->
- [x] Relay approval; the scanner applies; the reviewer completes the post-apply review. <!-- completed: 2026-11-03T12:46 -->
- [x] Pass `mise //:test`, `mise //:lint` and the anchor check. <!-- completed: 2026-11-03T12:46 -->
- [x] Per-run procedure item 3: tear down the Run 1 team, delete its fleet and retire the listener. <!-- completed: 2026-11-03T12:46 -->
- [x] Commit `docs: simplify SPEC.md and resolve verified contradictions`. <!-- completed: 2026-11-03T12:46 -->

### Step 3: Run 2 — docs/ and README.md

- [x] Per-run procedure items 1-2: create the Run 2 fleet, arm the listener, and spawn the two Run 2 scanners plus a reviewer, with `DECISIONS: § Decisions — docs/ and README.md (Run 2)`. <!-- completed: 2026-11-03T12:47 -->
- [x] Scanners write findings covering every D/DN entry labeled other than `reject`, against the post-Run-1 `SPEC.md` anchors, plus their own P3/P5 and `DD-new` rows; the Director checks coverage, then sends `ready (findings)`. <!-- completed: 2026-11-03T12:54 -->
- [x] After `approved (findings)`, record approved `DD-new` rows and follow-ups in this document. <!-- completed: 2026-11-03T12:58 -->
- [x] Relay approval; scanners apply; the reviewer completes the post-apply review. <!-- completed: 2026-11-03T13:03 -->
- [x] Pass `mise //:test`, `mise //:lint`, `mise //docs:build`, the rendered-pipe check, the anchor check and the terminology check. Confirm that `README.md:5` matches the `docs/docs/index.md` tagline and `overview.md:3`; route any drift back to the scanner as a Run 2 row before teardown. <!-- completed: 2026-11-03T13:03 -->
- [x] Per-run procedure item 3: tear down the Run 2 team, delete its fleet and retire the listener. <!-- completed: 2026-11-03T13:03 -->
- [x] Commit `docs: simplify docs site and README`. <!-- completed: 2026-11-03T13:03 -->

### Step 4: Run 3 — skills/ and the rule edit

- [x] Per-run procedure items 1-2: create the Run 3 fleet, arm the listener, and spawn the two Run 3 scanners plus a reviewer, with `DECISIONS: § Decisions — skills/ and rules (Run 3)`. <!-- completed: 2026-11-03T13:04 -->
- [x] Scanners write findings covering every K/KN/R entry labeled other than `reject`, plus their own P3/P5 and `DD-new` rows, preserving backend neutrality and every anchor linked from `.claude/**`; the Director checks coverage, then sends `ready (findings)`. <!-- completed: 2026-11-03T13:10 -->
- [x] After `approved (findings)`, record approved `DD-new` rows and follow-ups in this document. <!-- completed: 2026-11-03T13:16 -->
- [x] Relay approval; scanners apply; the reviewer completes the post-apply review. <!-- completed: 2026-11-03T13:22 -->
- [x] Pass `mise //:test`, `mise //:lint`, `mise //docs:build` and the anchor check. <!-- completed: 2026-11-03T13:22 -->
- [x] Per-run procedure item 3: tear down the Run 3 team, delete its fleet and retire the listener. <!-- completed: 2026-11-03T13:22 -->
- [x] Commit `docs: simplify skills and resolve verified contradictions` (`skills/**`) and `docs: align documentation rule examples with Core terms` (`.claude/rules/documentation-maintenance.md`). <!-- completed: 2026-11-03T13:22 -->

### Step 5: Final review (execute Step 5)

- [x] Verify every Success Criterion except the PR, including the scope check and an empty `git diff main -- src migrations Cargo.toml Cargo.lock build.rs .claude/skills`. <!-- completed: 2026-11-03T13:23 -->
- [x] Run sequential review rounds until a round approves. Each round: create a short-lived review fleet, arm the listener for its Director ID, and spawn one fresh, independent Reviewer under `skills/cafleet-design-doc/execute/roles/reviewer.md`; give it this design doc, `git diff main...HEAD` and the recorded check evidence; collect its verdict; then delete that fleet and retire its listener. <!-- completed: 2026-11-03T13:46 -->
- [x] After each round with findings that require edits, and only once that round's fleet is deleted, carry them out in one clean-docs simplification fix run. The fix run follows the per-run procedure limited to the affected files and records each finding as a `DD-new` row in § Run-discovered corrections. Rerun the affected checks, then start the next review round with a new review fleet. <!-- completed: 2026-11-03T13:46 -->
- [x] If fix runs added commits, re-cut the branch with `git reset --soft main` and recommit one commit per surface in rule order, keeping the Step 2-4 commit messages. <!-- completed: 2026-11-03T13:46 -->

### Step 6: User approval, finalize and PR (execute Steps 6-8)

- [x] Present the reviewed result (diff command, applied decisions, checks, follow-ups) for user approval through the executing backend's decision surface, and record the publication scope. Route any requested revision through a fix run and Step 5 before presenting again (execute Step 6). <!-- completed: 2026-11-03T13:45 -->
- [x] After user approval, set Status to Complete, refresh Progress and Last Updated, and commit `docs: add design doc 0000212-docs-spec-skills-simplification` (design-doc.md only) (execute Step 8). <!-- completed: 2026-11-03T13:45 -->
- [x] With remote approval, push the branch without force and open one PR with `gh pr create --title "..." --body-file <path>` (execute Step 7). <!-- completed: 2026-11-03T13:48 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-03 | Initial draft |
| 2026-11-03 | Review round 1: decision labels with verification basis, `AUTHORIZED ROWS` context line, orchestration and final execute gates, SA3 wording, heading-stays rule, K6 rejected, K23 keeps the native-command sentence |
| 2026-11-03 | Review round 2: sequential final-review rounds; user approval precedes the Status update |
| 2026-11-03 | Implemented: Runs 1-3 applied; run-discovered corrections RD1-RD9 recorded; final review round 1 found one gap (fix run 1, RD9), round 2 approved; user approved with push and PR; Status → Complete |
