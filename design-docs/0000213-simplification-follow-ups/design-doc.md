# Simplification Follow-ups

**Status**: Approved
**Progress**: 28/28 tasks complete
**Last Updated**: 2026-11-03

## Overview

Resolve the three follow-ups of design 0000212 (§ Follow-ups F1-F3) and the residue and pointer Observations that its clean-docs runs recorded, in one execute-workflow cycle. It makes `member create` report an unknown fleet like every other fleet command (F1) and restates prescriptive `SPEC.md` passages as descriptive contract (F3). A final native re-capture of Claude Code's `Teach auto mode` dialog documents only the recurrence it establishes (F2).

## Success Criteria

- [x] `cafleet member create --fleet-id <unknown>` prints `Error: fleet '<fleet-id>' not found.` and exits 1, matching `fleet show`, `fleet delete` and `member list`. A unit test in `src/cli/member.rs` asserts it.
- [x] `SPEC.md` § Error Messages and `skills/cafleet/reference/cli.md` § Error messages carry one combined unknown-fleet row for the four commands, and no `Fleet '<fleet-id>' not found.` string remains in the repository outside `design-docs/` and `researches/`.
- [x] Every F1, F3 and X1-X7 row in § Specification is applied with the replacement text given there, and F2-a to F2-d are applied exactly as § F2 capture evidence establishes; the Reviewer confirms the diff contains only those edits.
- [x] `SPEC.md` §6.9 Native acceptance and coverage, §9 Testing strategy, the §6.7 opencode preset limits table, the §6.7 Claude permission posture profile lead and both paragraphs of §6.3 `permissions.allow` coverage state descriptive contract only, with every case, threshold, entry and table row preserved.
- [x] `SPEC.md` has no run of two or more blank lines outside fenced code blocks, and no unnumbered `(§ …)` pointer names a heading of another numbered section.
- [x] Step 7 records its native capture evidence in § F2 capture evidence and applies its documentation edits from that evidence, or records the capture limit and leaves the dialog sentences unchanged.
- [x] The user's real Claude Code configuration is unchanged by F2: the real-state check in § Isolation and credentials passes (auto-mode key snapshot of `~/.claude.json`, SHA-256 of `~/.claude/settings.json` and the names-only `~/.claude/` listing match before and after).
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass in Step 6 and again in Step 8.
- [ ] One branch `fix/0000213-simplification-follow-ups` and one PR. Commits: the Director-reviewed failing F1-b test commit precedes the F1-a fix commit, and the documentation lands in one commit per surface (SPEC, docs, skills, design doc).

---

## Background

0000212 recorded three items it could not resolve inside a simplification run (`design-docs/0000212-docs-spec-skills-simplification/design-doc.md` § Follow-ups). Its three clean-docs runs also reported Observations that their rubric could not apply: change-relative "existing" wording, cross-section pointers without a section number, a stray `manual-generation`, doubled blank lines and a "TDD team" wording drift in the design-doc skill. The user selected those items for this cycle (X1-X7). Line numbers in this document refer to `7dbf1111`; locate every row by its quoted text.

---

## Specification

### Decision labels

| Label | Meaning |
|---|---|
| `DD` | A documentation row: replace the quoted current text with the given replacement, adapted only to sentence or hard-wrap boundaries with every fact, string and pointer intact |
| `CODE` | A Rust change with its unit test |
| `F2` | An edit whose text the F2 capture determines (§ F2 native re-capture) |

### F1 — Unknown fleet in `member create`

Verified: `src/cli/member.rs:371-372` resolves the fleet with `broker::get_fleet(conn, fleet_id)?.ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?`. `CafleetError::Usage` exits 2 (`src/error.rs:52`). `fleet show` and `fleet delete` call `broker::require_fleet` (`src/cli/fleet.rs:87,478`), and `member list` calls `require_live_fleet` (`src/broker/members.rs:43,126,239`), which starts with `require_fleet`; that helper returns `CafleetError::App(format!("fleet '{fleet_id}' not found."))`, exit 1 (`src/broker/fleets.rs:34-37`).

**CODE F1-a.** Replace `src/cli/member.rs:371-372` with:

```rust
    let fleet = broker::require_fleet(conn, fleet_id)?;
```

The following `deleted_at` check (`fleet {fleet_id} is deleted`, exit 1) and the missing-`director_member_id` check stay unchanged in text and position. Behavior is otherwise identical: the lookup still runs first in the command body, after the schema-version guard and before Director resolution.

**CODE F1-b (test).** Add the test to `src/cli/member.rs` `mod tests::launch_policy`, following the pattern of its `invalid_creation` helper:

1. `let dir = crate::runtime::test_support::short_tempdir();` and `let mut conn = migrated_conn(&dir);` — the baseline database holds no fleet.
2. Build the argv from `create_arguments()` with `arguments[4]` set to `"999"` and the positional prompt `"literal assignment"` appended; parse it with `CliArgs::try_parse_from` into `Command::Member(command)`.
3. `let before = table_snapshot(&conn, |_| true);`
4. `Settings::for_database(Path::new(conn.path().unwrap()), Some("unavailable-test-multiplexer"))`, then `member::run(&mut conn, &settings, command, crate::time::Deadline::new())`.

Assertions:

- the error is `CafleetError::App` with `message()` exactly `fleet '999' not found.`;
- `exit_code()` is `1`;
- `table_snapshot(&conn, |_| true)` equals `before`.

The unavailable multiplexer makes the ordering observable: a run that reached multiplexer resolution would fail with a multiplexer error instead of the fleet error.

Edge cases stay as today: a non-integer `--fleet-id` is a parser error (exit 2); a soft-deleted fleet fails with `fleet X is deleted` (exit 1); a fleet without a recorded root Director fails with its existing message (exit 1).

**DD F1-c** `SPEC.md:1150` (§6.3 Error Messages). Replace the row

`| member create | Unknown --fleet-id | Error: Fleet '<fleet-id>' not found. | 2 | A usage error raised by the first fleet check, after the schema-version guard and before Director resolution |`

with:

```markdown
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID | `Error: fleet '<fleet-id>' not found.` | 1 | For `member create`, raised by the first fleet check, after the schema-version guard and before Director resolution |
```

**DD F1-d** `skills/cafleet/reference/cli.md:457` (§ Error messages, four columns). Replace the `member create` / Unknown `--fleet-id` row with:

```markdown
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID | `Error: fleet '<fleet-id>' not found.` | 1 |
```

The command-prose mentions at `SPEC.md:396`, `SPEC.md:400` and `cli.md:86` stay. `docs/` carries no copy of either string.

### F3 — Descriptive SPEC voice

Basis: `.claude/rules/documentation-maintenance.md` keeps `SPEC.md` "descriptive — specification only, no recommendations or implementation advice". Scope is limited to the named sites below; every case, threshold, entry and table row survives.

**DD F3-a** `SPEC.md:3426` (§6.9 Native acceptance and coverage, first paragraph), first sentence:
- Current: "Use synthetic terminal replay and real native PTYs for every declared detector, composer and answer capability on each registered multiplexer."
- Replacement: "Acceptance evidence for every declared detector, composer and answer capability consists of synthetic terminal replay and real native PTYs on each registered multiplexer."

**DD F3-b** `SPEC.md:3428` (release-minimum paragraph). Replacement paragraph:

> The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Acceptance records pin exact installed CLI versions to their corresponding bundled profiles, and their permitted disposable fixtures actually reach the native approval surface. Every minimum case demonstrates blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and a preserved original request. Separate fresh requests cover verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, manual original-surface input and a user-executed complete bound command. The records show zero response bytes before required user permission and record actual native results and observed continuation separately from submitted keys. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

The automatic-mode paragraph (`:3430`), the minimum-version delivery lead and table (`:3432-3445`) and the measurement sentence (`:3447`) are already descriptive and stay.

**DD F3-c** `SPEC.md:3449` (live acceptance record paragraph). Replacement paragraph:

> Live acceptance records carry the source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, Director mode/provenance, attributed decision/evidence references, request identity/token, receipt/stage, multiplexer, terminal size, cwd and redacted evidence. They include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. The exercised cases cover stale tokens/epochs/events, same/different operation keys, conflicting decisions, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.

**DD F3-d** `SPEC.md:3451-3462` (transport/lifecycle/persistence lead and bullets). Replacement:

```markdown
Acceptance additionally covers the transport, lifecycle and persistence cases below. With a live receiver and available storage, the latency target is two seconds from a stable prompt to inbox commit and another two seconds from commit to receiver output.

- Exact child argv for the selected native policy, with native/managed restrictions and sandbox boundaries preserved.
- Equal run IDs across sibling databases, and replacement at an unchanged path.
- A held write transaction and a long statement, showing that restoration and exit are independent of the connection's default timeout.
- A child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.
- Timeout followed by the original runner's recovery, and concurrent recovery attempts.
- A command containing a known secret, which is absent from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.
- Hard kill with complete uncommitted frames, replay of an opening before loss closure, concurrent recovery, interruption during recovery, and deregistration before replay with archived rather than fabricated inbox records.
- A live unlinked lock, verified death, unknown identity without attestation, matching/stale attestations, positive live evidence rejecting an attestation and concurrent database-only recovery.
- Cross-invocation accepted-then-failed-stamp recovery, queued-then-drained delivery after interaction closure, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.
- Every registered backend's omitted-policy argv and each adapter-declared explicit policy; a profile claiming detection, composer or answer support carries both fixtures and real native evidence.
```

**DD F3-e** `SPEC.md:3732` (§9 Testing strategy). Replace the paragraph's first word "Exercise" with "The test suite covers", and replace its last two sentences "…; links, embedding and release builds. Preserve general timed pipe draining and owned-child cleanup coverage." with "…; links, embedding and release builds; and general timed pipe draining and owned-child cleanup." Every listed item between them stays verbatim.

**DD F3-f** `SPEC.md:2050-2062` (§6.7 opencode preset limits). Replacements:
- "Standalone un-enumerated commands fall to the `"*"` base. Apply these limits when choosing tools and authorizing commands:" → "Standalone un-enumerated commands fall to the `"*"` base. These limits apply to tool use and command authorization:"
- Header `| Surface | Example | Usage limit |` → `| Surface | Example | Limit |`.
- MCP cell: "Their participation in preset permission evaluation is unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers." → "Their participation in preset permission evaluation is unverified on opencode 2.1.4; evaluation is established only for loaded configurations free of MCP servers."
- Compound cell: "Permission matching for compound lines is unverified on opencode 2.1.4. Submit one command per invocation and authorize each actual command." → "Permission matching for compound lines is unverified on opencode 2.1.4; matching is established for one command per invocation."
- Interpreter cell: "Authorize the code and hooks that an allowed command can execute." → "An allowed command also runs the code and hooks it executes, under that command's permission."
- "cafleet ships no MCP stanzas. Use the `codex` backend for kernel-enforced isolation." → "cafleet ships no MCP stanzas. The `codex` backend provides kernel-enforced isolation (§6.7 Codex sandbox and configuration)."

The literal-pipe cell (<code>git log --stat; curl … &#124; sh</code>) stays byte-identical.

**DD F3-g** `SPEC.md:1928-1930` (§6.7 Claude permission posture, profile lead):
- Current: "The recommended Claude Code permission profiles, the auto mode profile for a Director session in `auto` and the other-modes profile for any other mode, carry these entries:"
- Replacement: "Two Claude Code permission profiles specify each role's required entries: the auto mode profile for a Director session in `auto` and the other-modes profile for any other mode. They carry these entries:"

The table and the paragraphs after it stay; `docs/docs/quickstart.md` keeps the user-facing recommendation.

**DD F3-h** `SPEC.md:229` (§6.3 `permissions.allow` coverage, second paragraph). Replacement paragraph:

> Keeping the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes under `permissions.ask` makes the operator confirm each explicit native pane-input dispatch of the Director, because ask rules outrank allow rules. Inside a member, the Director-only commands refuse through the runner guard (§ Director-only command guard). `cafleet permission respond` runs under the Director host's effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses. The Claude permission profiles (§6.7 Claude permission posture) specify each role's entries.

**DD F3-i** `SPEC.md:227` (§6.3 `permissions.allow` coverage, first paragraph). Replacement paragraph:

> Prefix rules such as `Bash(cafleet message *)` cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Native Monitor authorization is separate: a Bash-rule check establishes no Monitor permission. Literal IDs and trailing JSON flags preserve prefix matching.

### X1 — SPEC change-relative "existing"

Rule: "existing" stays where it qualifies an object that already exists at that point in the flow (for example "an unread existing row", "replacing any existing target", "every existing entry in its existing order", "an existing interaction", "returns the existing event/message IDs", "preserves existing history"). It goes where it marks a mechanism as pre-dating a change. Basis for each row: the named mechanism is defined in the cited section or code, and the replacement names it without the change-relative qualifier.

| ID | Location | Current | Replacement |
|---|---|---|---|
| X1-01 | `SPEC.md:703` | Native Director reception retains the existing listener process between events. | Native Director reception keeps one listener process running across events (§6.3 `message listen`). |
| X1-02 | `SPEC.md:877-879` | Existing output keys retain their order and nullability; runtime fields append under the runtime presentation contract (§6.3 Runtime presentation). | The output keys keep their documented order and nullability, and runtime fields follow the runtime presentation contract (§6.3 Runtime presentation). |
| X1-03 | `SPEC.md:994` | Missing member/placement and native process errors retain their existing primary diagnostics. | A missing member or placement and a native process error report their own primary diagnostic. |
| X1-04 | `SPEC.md:1044` | … interaction ownership under the existing pane lock. | … interaction ownership under the member's pane-input lock (§6.5 Pane input lock). |
| X1-05 | `SPEC.md:1428` | Every span is bounded by its existing deadline, … | Every span is bounded by its own deadline, … |
| X1-06 | `SPEC.md:1465` | Existing relative/untimed execution retains its separate behavior. | Relative and untimed execution outside the press path follows § Subprocess output and deadlines. |
| X1-07 | `SPEC.md:1511` | This warning uses the existing attempt error shape; | This warning uses the attempt error shape; |
| X1-08 | `SPEC.md:2224` | … belong to the command's existing ten-second creation budget, … | … belong to the command's ten-second creation budget, … |
| X1-09 | `SPEC.md:2565` | Existing observation definitions without a submission plan retain their observation coverage and yield no executable composer capability. | An observation definition without a submission plan keeps its observation coverage and yields no executable composer capability. |
| X1-10 | `SPEC.md:2645` | Preserve the existing inherited native configuration; | Keep the native configuration the child inherits; |
| X1-11 | `SPEC.md:2663` | … and preserve existing rollback/cleanup diagnostics if any phase fails. | … and preserve the rollback/cleanup diagnostics of any phase that fails. |
| X1-12 | `SPEC.md:2693` | Acquire existing pane locks in their established order, revalidate placement/run, request runner shutdown and finish pane/registry cleanup under existing semantics. | Acquire the pane-input locks in the order §6.3 `member delete` and `fleet delete` specify, revalidate placement/run, request runner shutdown and finish pane/registry cleanup as those commands specify. |
| X1-13 | `SPEC.md:2955` | The transaction shares the broker’s existing fleet/active-member checks. | The transaction applies the broker's fleet and active-member checks (§6.2). |
| X1-14 | `SPEC.md:3060` | … its existing two/three/four-second boundaries share that origin … | … its two/three/four-second boundaries share that origin … |
| X1-15 | `SPEC.md:3076` | … acquire the existing pane lock when a concrete placement remains, … | … acquire the member's pane-input lock when a concrete placement remains, … |
| X1-16 | `SPEC.md:3078` | Replay valid originals in their existing ascending order, … | Replay valid originals in their recorded ascending order, … |
| X1-17 | `SPEC.md:3128` | … through the existing manual queue, terminal parser, ordered observer and outer display. | … through the runner's manual queue, terminal parser, ordered observer and outer display. |
| X1-18 | `SPEC.md:3144` | … a zero-length write retains the existing no-progress error. | … a zero-length write fails with `terminal write made no progress`. |
| X1-19 | `SPEC.md:3192` | … use the existing serialized receipt operations and original run lock. | … use the serialized receipt operations and the original run lock. |
| X1-20 | `SPEC.md:3196` | … only confirmed submission or the existing ACK settlement stamps them. | … only confirmed submission or ACK settlement (§6.3 `message ack`) stamps them. |
| X1-21 | `SPEC.md:3247` | … existing receipt reconciliation is required before advancing | … receipt reconciliation (`input reconcile`) is required before advancing |
| X1-22 | `SPEC.md:3249` | Ownership loss transfers finalization to the existing fenced recovery path, … | Ownership loss transfers finalization to the fenced recovery path, … |
| X1-23 | `SPEC.md:3251` | Existing `in_progress`/`uncertain` reconciliation applies … | `in_progress`/`uncertain` reconciliation applies … |
| X1-24 | `SPEC.md:3274` | … until actual submission or the existing ACK settlement. | … until actual submission or ACK settlement (§6.3 `message ack`). |
| X1-25 | `SPEC.md:3328` | … independently observed outcomes under the existing Director/user policy. | … independently observed outcomes under the Director/user policy. |
| X1-26 | `SPEC.md:3361` | … inspection preserves the existing evidence schema and persists no token or pre-policy digest. | … inspection exposes evidence in the permission inspection projection (§6.3 Permission commands) and persists no token or pre-policy digest. |
| X1-27 | `SPEC.md:3367` | … and the existing receipt query/retire boundary. | … and the receipt query/retire boundary (§ Durable input receipts and settlement). |
| X1-28 | `SPEC.md:3395` | Use the existing `message listen`, `poll --json` and finite `wait` interfaces. Runtime events reach … they introduce no composer injection, Escape-first wake, goal activation or new host interrupt guarantee … | The Director consumes runtime events through `message listen`, `poll --json` and finite `wait`. Runtime events reach … they provide no composer injection, Escape-first wake, goal activation or host interrupt guarantee … |
| X1-29 | `SPEC.md:3422` | A routed request that arrives with no event follows the refusal route above unchanged. | A routed request that arrives with no event follows the refusal route above. |

`SPEC.md:3453` "existing sandbox boundaries" is resolved by F3-d. The table cells above quote with `…` only text that stays verbatim; every Replacement cell is the exact `SPEC.md` text.

Row-specific bases: X1-18 names the error `src/runtime/transport.rs:156` returns for `Ok(0)`. X1-26 keeps the fact that inspection exposes evidence in its documented shape; the same paragraph already points to the permission inspection projection (§6.3 Permission commands) for missing facts.

### X2 — docs change-relative "existing"

| ID | Location | Current | Replacement |
|---|---|---|---|
| X2-1 | `docs/docs/concepts/members.md:25` | Replace an unusable member only under the existing authorization, carrying … | Replace an unusable member only when its replacement is authorized, carrying … |
| X2-2 | `docs/docs/concepts/members.md:35` | … coordinates shutdown and event recovery under the existing pane locks, … | … coordinates shutdown and event recovery under each member's pane-input lock, … |

Basis: X2-1 matches `skills/cafleet/roles/director.md:483` ("an authorized unusable-session replacement"); X2-2 names the lock of `SPEC.md` §6.5 Pane input lock. `README.md` needs no edit.

### X3 — skills change-relative wording

| ID | Location | Current | Replacement |
|---|---|---|---|
| X3-1 | `skills/cafleet-design-doc/reference/coordination.md:115-116` | User-written markers remain accepted. This changes no member role obligations, verb/pointer pairing, marker removal, or approval preconditions. | User-written markers are accepted as well. Relayed and user-written feedback follows the same member role obligations, verb/pointer pairing, marker removal and approval preconditions as every other marker. |
| X3-2 | `skills/cafleet-design-doc/reference/coordination.md:100` | … both tagged with the existing `[COMPLIANCE]` / … taxonomy inside the marker body. | … both tagged with the `[COMPLIANCE]` / … taxonomy inside the marker body. |
| X3-3 | `skills/cafleet-design-doc/execute/execute.md:36` | Existing roles retain their responsibilities; a Programmer-only team … | Each role keeps the responsibilities its role file defines; a Programmer-only team … |
| X3-4 | `skills/cafleet/roles/director.md:80` | … such as a folder-trust prompt, follows the existing held-interaction procedure. | … such as a folder-trust prompt, follows [Consume observed interactions](#consume-observed-interactions). |
| X3-5 | `skills/cafleet/roles/director.md:378` | Read complete inbox JSON through the existing receiver, `poll --json` or finite `wait`; … | Read complete inbox JSON through the running receiver, `poll --json` or finite `wait`; … |
| X3-6 | `skills/cafleet/roles/director.md:448` | Surface the cause and follow the existing repair/authorization procedure before restarting | Surface the cause, and restart only after the repair it names is authorized and complete |

Uses of "existing" that qualify an artifact present at that point stay (for example "an existing document", "existing annotations", "Preserve an existing feature branch", "keeping existing entries and their order"). The `#consume-observed-interactions` anchor exists (`director.md:376`).

### X4 — Cross-section pointers

An unnumbered `(§ Heading)` pointer names a heading in its own numbered section; a pointer to another section carries the number. A scan of every `(§ <name>)` pointer at `7dbf1111` found exactly these three crossing a section:

| ID | Location | Current | Replacement |
|---|---|---|---|
| X4-1 | `SPEC.md:522` (§6.3) | `(§ Launch policy and capability registry)` | `(§6.9 Launch policy and capability registry)` |
| X4-2 | `SPEC.md:637` (§6.3) | `(§ Storage pressure and durability boundary)` | `(§6.9 Storage pressure and durability boundary)` |
| X4-3 | `SPEC.md:2163` (§6.8) | `(§ *Member environment*)` | `(§6.3 *Member environment*)` |

### X5, X6 — SPEC terminology and blank lines

- **X5** `SPEC.md:3369`: "current token/request/choice/focus/plan, manual-generation, owner and immediate-transaction validation" → "current token/request/choice/focus/plan, `human_generation`, owner and immediate-transaction validation" (0000212 SB-min generation; `src/runtime/transport.rs:18-32`).
- **X6** Collapse every run of two or more consecutive blank lines outside fenced code blocks in `SPEC.md` to one blank line. At `7dbf1111` the runs start at lines 284, 365, 880 (three blank lines), 900, 916, 1016, 1093, 1181, 1237, 1251, 1301, 1309, 1333, 1353, 1367, 1388 and 1569.

### X7 — Execute-team wording in the design-doc skill

Basis: `skills/cafleet-design-doc/execute/execute.md` § Team composition composes a Programmer-only team for documentation/configuration work.

- `skills/cafleet-design-doc/SKILL.md:6-7` (frontmatter `description`): "(execute workflow → TDD team)" → "(execute workflow → TDD or Programmer-only team)", re-wrapped within the folded scalar.
- `skills/cafleet-design-doc/SKILL.md:49` (Dispatch row): "— TDD team" → "— TDD or Programmer-only team".

### F2 native re-capture

Goal: establish, on the installed Claude Code 2.1.309, whether `Teach auto mode about your environment?` reappears after each answer. Verified at `7dbf1111`: the only capture is the dialog's first appearance (`src/coding_agent/native/claude.json:2659-2720`); `SPEC.md:2331` states only that it does not reappear after `3. Don't show again`. The user's `~/.claude.json` holds `autoModeEnvSetup: {"denials": 4}`, which suggests a classifier-denial trigger; the capture treats that as a hypothesis.

#### Isolation and credentials

| Concern | Rule |
|---|---|
| Configuration directory | A fresh `CLAUDE_CONFIG_DIR` created with `mktemp -d /tmp/cafleet-f2-XXXXXX` (mode `0700`), outside the repository and `researches/`. The user's real `~/.claude` and `~/.claude.json` are never read or written by the capture session. |
| Credentials | The user performs one interactive Claude Code login inside that isolated directory, so its credentials live only in `<iso>/.credentials.json`. Nothing copies, prints or records the credential file or its values. A second login is needed only when a case below requires a fresh directory. |
| Working directory | A disposable git repository at `<iso>/work` with one commit and no remote. |
| Session | A private tmux server: `tmux -L cafleet-f2 new-session -d -s f2 -x 120 -y 40 -c <iso>/work -e CLAUDE_CONFIG_DIR=<iso> 'claude --permission-mode auto'`. The Director relays the user's one-time login, onboarding and workspace trust through the decision surface; the user performs them with `tmux -L cafleet-f2 attach -t f2` and detaches. The Verifier owns the server, drives turns with `tmux -L cafleet-f2 send-keys` and reads screens with `tmux -L cafleet-f2 capture-pane -p`. |
| Evidence files | The Verifier writes screen captures to `<BASE>/.f2-capture/` (hidden, local; only `design-doc.md` is committed). Before writing, each capture is checked to contain no credential or token text. |
| Real-state check | Before the first launch and after teardown, record (a) the auto-mode key snapshot of `~/.claude.json`, which is the JSON of every top-level key matching `/autoMode\|teach/i` with its value, (b) the SHA-256 of `~/.claude/settings.json` and (c) the names-only listing `ls -1A ~/.claude/`. The check passes when all three match. Whole-file hashes of `~/.claude.json`, sizes and mtimes under `~/.claude/`, and `~/.claude/.credentials.json` are outside the check, because the user's and the fleet's own sessions rewrite them. Isolation of the capture's credentials is established positively: `<iso>/.credentials.json` exists after the user's login and `<iso>/.claude.json` holds the capture session's state. A failed check stops Step 7 and the Director reports the differing item to the user. |
| Teardown | `tmux -L cafleet-f2 kill-server`, then `rm -rf` of every isolated directory, which discards the isolated sessions' credentials. |

#### Procedure

An **observation window** is a run of trigger turns that ends when the dialog appears or after 12 turns or 30 minutes, whichever comes first; each window starts with a fresh bound. The whole capture is capped at 3 hours from the first launch; a case not finished at the cap is recorded as "not observed". A **trigger turn** asks for one benign operation the auto-mode classifier is likely to refuse and that is harmless if approved: `git push --force origin main` (no remote exists), `git reset --hard HEAD~1` on the disposable commit, or `rm -rf <iso>/work/decoy`. After each turn the Verifier captures the screen and reads `autoModeEnvSetup` from `<iso>/.claude.json`.

1. **Trigger.** Run one window. When it ends without the dialog, the capture result is "not reproduced" and the procedure continues at step 5.
2. **Esc case.** At the appearance, capture the offer, press Esc, then capture the result and the `<iso>/.claude.json` auto-mode keys. Run one window in the same session, then `/exit`, relaunch in the same configuration and run one more window. Record whether and in which window the dialog reappeared.
3. **`2. Not now` case.** Start from the reappearance seen in step 2. When step 2 saw none, create a fresh isolated directory, relay a new user login, and run one trigger window there; when that window ends without the dialog, record this case as "not observed" and continue at step 4. At the appearance, select `2. Not now` and observe with the two windows of step 2.
4. **`3. Don't show again` case.** Start from the reappearance seen in step 3, or, when step 3 saw none, from a fresh isolated directory with the same rule as step 3. Select `3. Don't show again` and observe with the two windows of step 2. This re-establishes the 2.1.302 fact on 2.1.309.
5. **Record.** Report one `COMMENT(verifier)` marker per case at § F2 capture evidence (version, date, steps, observed recurrence as "window N of the case" or "not within its windows", and the auto-mode key change), with the capture file names. Tear down and run the real-state check.

#### Documentation edits from the evidence

Each edit states only the observed outcomes, scoped "on Claude Code 2.1.309"; the 2.1.302 statement from the original fixtures stays. An unobserved case adds nothing.

| ID | Site | Edit when the case is established |
|---|---|---|
| F2-a | `SPEC.md:2331` sentence "The dialog consumes typed input; after `3. Don't show again` it does not reappear for that configuration." | Append one sentence per established case, e.g. "On Claude Code 2.1.309, after `2. Not now` it reappears <observed condition>, and after Esc it <observed outcome>." |
| F2-b | `SPEC.md:1915` "§6.9 *Bundled profiles and native screen fixtures* states when it stops reappearing." | "… states its recurrence after each answer." when F2-a adds a sentence |
| F2-c | `docs/docs/quickstart.md:102-105` | Add the established outcomes in one sentence after "after that answer it does not return." |
| F2-d | `skills/cafleet/reference/coding-agents.md:62` claude note row | Add the established outcomes after "after `3. Don't show again` it does not reappear for that configuration", within the row's two-sentence cell budget |

When the capture is "not reproduced", or no case beyond `3. Don't show again` is established, F2-a to F2-d make no edit and § F2 capture evidence records the limit.

#### F2 capture evidence

The Director fills this table in Step 7 from the Verifier's markers and removes each marker as its row is filled.

| Case | Version | Date | Steps | Observed recurrence | Auto-mode key change |
|---|---|---|---|---|---|
| Trigger (step 1) | 2.1.309 (session banner) | 2026-11-03 | One window of 12 trigger turns in one isolated `claude --permission-mode auto` session; turns 6 and 9-12 were classifier denials and in turns 7-8 the model declined without attempting the commands; three denials reached the native "3 consecutive actions were blocked" confirmation | Not reproduced: `Teach auto mode about your environment?` did not appear within the window | `autoModeEnvSetup` absent → `{"denials": 5}`, then unchanged by later denials |
| Esc, `2. Not now`, `3. Don't show again` (steps 2-4) | — | — | Not run: step 1 established no appearance, so the procedure continued at step 5 | Not observed | — |

Capture limit: the dialog did not appear on Claude Code 2.1.309 within one observation window, so no recurrence beyond the original fixtures is established and F2-a to F2-d make no edit. The real-state check passed (`.checks/real-state-before.md` matches the post-teardown snapshot).

### Verification

| Check | When | Pass condition |
|---|---|---|
| `mise //:test`, `mise //:lint` | Step 6, Step 8 | Both pass |
| `mise //docs:build` | Step 6, Step 8 | Passes |
| Rendered-pipe check | Step 6, Step 8 | The F3-f compound-command cell is byte-identical to `7dbf1111` |
| String check | Step 6, Step 8 | `git grep -n "Fleet '" -- ':(exclude)design-docs/**' ':(exclude)researches/**'` returns nothing |
| Blank-line and pointer check | Step 6, Step 8 | No double blank line outside fences in `SPEC.md`; the cross-section pointer scan in X4 finds nothing |
| Anchor check | Step 6, Step 8 | No heading is added, removed or renamed by this change |
| Real-state check | Step 7 | § Isolation and credentials passes |
| Scope check | Step 8, Step 9 | `git diff main --stat` lists only `SPEC.md`, `docs/docs/concepts/members.md`, `skills/**`, `src/cli/member.rs` and this design doc, plus `docs/docs/quickstart.md` when F2-c applies |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Team, per the execute workflow's § Team composition:

| Member | Work | Owned paths |
|---|---|---|
| Programmer | Every `DD` row (Steps 2-4), F1-a (Step 5) and F2-a to F2-d (Step 7) | `SPEC.md`, `docs/docs/**`, `skills/**`, the non-test code of `src/cli/member.rs` |
| Tester | F1-b (Step 5) | The `#[cfg(test)] mod tests` region of `src/cli/member.rs` |
| Verifier | The F2 live native capture (Step 7, § Procedure steps 1-5) on the isolated, owned resources of § Isolation and credentials | `<BASE>/.f2-capture/`, the isolated directories and the `cafleet-f2` tmux server |
| Reviewer | One fresh, independent review of the full branch diff after every implementation and verification gate (Step 9) | — |

Order: documentation rows in rule order (`SPEC.md` → `docs/` → `skills/`), the F1 test then its fix, checks, the F2 capture and its edits as the last implementation step, checks again, the fresh review loop, then commits. The Director relays the user's login for the capture through the decision surface. User approval, the design-doc commit and the PR follow under the execute workflow's Steps 6-8. The installed CLI is not reinstalled during the cycle.

### Step 1: Prepare

- [x] Create branch `fix/0000213-simplification-follow-ups` from `main`. <!-- completed: 2026-11-03T15:19 -->
- [x] Run `mise //:test`, `mise //:lint` and `mise //docs:build` on the unchanged branch and record the baseline. <!-- completed: 2026-11-03T15:19 -->

### Step 2: SPEC.md

- [x] Apply F1-c. <!-- completed: 2026-11-03T15:24 -->
- [x] Apply F3-a to F3-i. <!-- completed: 2026-11-03T15:24 -->
- [x] Apply X1-01 to X1-29. <!-- completed: 2026-11-03T15:24 -->
- [x] Apply X4-1 to X4-3, X5 and X6. <!-- completed: 2026-11-03T15:24 -->

### Step 3: docs/

- [x] Apply X2-1 and X2-2. <!-- completed: 2026-11-03T15:24 -->

### Step 4: skills/

- [x] Apply F1-d. <!-- completed: 2026-11-03T15:24 -->
- [x] Apply X3-1 to X3-6. <!-- completed: 2026-11-03T15:24 -->
- [x] Apply X7. <!-- completed: 2026-11-03T15:24 -->

### Step 5: F1 test and code

- [x] Tester: write F1-b in `src/cli/member.rs` `mod tests::launch_policy` and confirm it fails against the current code (`CafleetError::Usage`, `Fleet '999' not found.`). <!-- completed: 2026-11-03T15:22 -->
- [x] Director: review F1-b against § F1 and commit it alone with `fix: add failing test for unknown fleet in member create` (`git add src/cli/member.rs` while the file holds only the test change; the documentation edits stay unstaged). <!-- completed: 2026-11-03T15:24 -->
- [x] Programmer: apply F1-a so F1-b and every existing test pass. <!-- completed: 2026-11-03T15:27 -->

### Step 6: Checks

- [x] Run `mise //:test`, `mise //:lint` and `mise //docs:build`. <!-- completed: 2026-11-03T15:29 -->
- [x] Run the rendered-pipe, string, blank-line/pointer and anchor checks in § Verification. <!-- completed: 2026-11-03T15:29 -->

### Step 7: F2 native re-capture (final implementation step)

- [x] Director: spawn the Verifier, record the real-state snapshots and relay the user's login, onboarding and workspace-trust request through the decision surface once the Verifier has started the isolated session. <!-- completed: 2026-11-03T21:13 -->
- [x] Verifier: run § Procedure steps 1-5, writing captures under `<BASE>/.f2-capture/` and one `COMMENT(verifier)` marker per case at § F2 capture evidence, then tear down. <!-- completed: 2026-11-03T21:13 -->
- [x] Director: confirm the real-state check, fill § F2 capture evidence from the markers and remove them. <!-- completed: 2026-11-03T21:13 -->
- [x] Programmer: apply F2-a to F2-d from the evidence, or no edit when the evidence establishes nothing beyond `3. Don't show again` (no edit: not reproduced). <!-- completed: 2026-11-03T21:13 -->

### Step 8: Checks after F2

- [x] Run `mise //:test`, `mise //:lint` and `mise //docs:build`. <!-- completed: 2026-11-03T21:20 -->
- [x] Run the rendered-pipe, string, blank-line/pointer, anchor and scope checks in § Verification. <!-- completed: 2026-11-03T21:20 -->

### Step 9: Fresh review loop (execute Step 5)

- [x] Director: verify every implementation task and Success Criterion except the PR, then spawn a fresh Reviewer with this design doc, `git diff main` (committed test plus working tree) and the recorded check and capture evidence. <!-- completed: 2026-11-03T21:20 -->
- [x] Reviewer: compare the diff with § Specification (F1, F3, X1-X7 rows and the F2-a to F2-d edits against § F2 capture evidence), check each basis against code or the cited section, and run the scope check. <!-- completed: 2026-11-03T21:27 -->
- [x] Route every finding to its owner, rerun the affected checks and repeat fresh review until it approves. <!-- completed: 2026-11-03T21:27 -->

### Step 10: Commits

- [x] Commit `docs: describe SPEC acceptance and profiles and drop change-relative wording` (`SPEC.md`). <!-- completed: 2026-11-03T15:33 -->
- [x] Commit `docs: drop change-relative wording from docs` (`docs/`). <!-- completed: 2026-11-03T15:33 -->
- [x] Commit `docs: align skills with fleet errors and current-state wording` (`skills/`). <!-- completed: 2026-11-03T15:33 -->
- [x] Commit `fix: report unknown fleet in member create like other fleet commands` (`src/cli/member.rs`), after the Step 5 test commit. <!-- completed: 2026-11-03T15:33 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-03 | Initial draft |
| 2026-11-03 | Review round 1: Overview cap; F1-b pattern from `launch_policy`; F1-c keeps the check-order note; F3-b keeps "permitted"; F3-i for the first `permissions.allow` paragraph; X1-18/X1-26 replacements; observation windows and 3-hour cap; satisfiable real-state check; team per § Team composition with a Verifier for F2; F2 before one fresh review; separate test and fix commits |
| 2026-11-03 | Step 7: F2 capture not reproduced on 2.1.309; F2-a to F2-d make no edit; real-state check passed; fresh review approved in round 2 |
