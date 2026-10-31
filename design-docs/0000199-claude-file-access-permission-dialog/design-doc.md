# Claude File-Access Permission Dialog and Multi-Line herdr Capture

**Status**: Approved
**Progress**: 0/23 tasks complete
**Last Updated**: 2026-10-31

## Overview

The bundled Claude profile detects Claude Code's "Read outside the working directories" dialog as a native permission request, so the runner publishes `interaction_opened`, queues programmatic input behind it, and answers it only with its single-request choices. `cafleet member capture` returns every requested line on herdr, where a CRLF line ending currently blanks every line but the last (GitHub issue #527).

## Success Criteria

- [ ] On Claude Code 2.1.309 under the runner, the "Read outside the working directories" dialog yields a `permission_request` observation from profile `claude-2.1.302-auto-1`, an `interaction_opened` event with `kind=review_permission_request`, and a non-null `runtime.current_interaction_id` in `member show --json`.
- [ ] Previews, `member prompt` and `member exec` sent while the dialog is open are queued with zero bytes and drain after it closes.
- [ ] `permission respond` with allow sends exactly byte `34` (digit `4`, `Yes, but ask again next time`) and with deny exactly byte `33` (digit `3`, `No, and ask again next time`). No answer plan, `permission respond` or queued preview, `member prompt` or `member exec` input sends `1`, `2`, Enter or Tab to the dialog; explicit `member press` keys remain the Director's manual choice. If the native capture shows that a digit does not select its choice directly, the rule ships detection-only and both answers stay manual.
- [ ] Each of Edit, Write, Glob and Grep is probed against a path outside the working directory; a dialog with its own "… outside the working directories" literals is detected (with answer plans when its digit check passes), and every tool that produced no such dialog is a documented limit.
- [ ] `cafleet member capture <id> --lines N` on herdr returns every drawn line of the open dialog, not only the last one, for every herdr pane. Its plain output equals, line by line after trimming trailing whitespace, `herdr pane read <pane> --format text --source recent-unwrapped --lines <N+1000>` after the same visually-blank-tail trim and last-N window. `--ansi` output keeps the raw bytes, including CRLF.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint` and `mise //docs:build` pass, and live acceptance passes on both tmux and herdr.

---

## Background

| Symptom | Cause |
|---|---|
| No `interaction_opened`, `current_interaction_id: null`, previews not queued while the dialog is open | Every `permission_request` rule of `claude-2.1.302-auto-1` (`claude-bash-permission`, `claude-auto-ask-permission`, `claude-bash-permission-auto-tip`) anchors on `^ Bash command\s*$` and a `Do you want to proceed?` question. The file-access dialog has its own heading, question and four choices, so the frame is `unknown` and the Director receives no event. |
| `member capture` prints only ` Esc to cancel · Tab to amend` under herdr | herdr 0.9.5 `pane read --format ansi` ends every line with `\r\n` (`--format text` uses `\n`). The plain-text cleanup splits on `\n` and keeps the segment after the last `\r` of each line, which is empty for every CRLF-terminated line. Only the final line, which has no terminator, survives. tmux `capture-pane -p -e` emits no `\r` before `\n` and is unaffected. Reproduced on wrapped member panes and on the Director pane. |

The dialog captured in the issue (Claude Code 2.1.309, `auto`):

```
 Read outside the working directories
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Read(/development-pvc/work/himkt/cafleet/SPEC.md · lines 2690-2799)
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
 Auto mode and the sandbox read outside the working directories without asking. Yes or No answers for this read; the rest is for later reads. ...

 Allow this read outside the working directories?
 ❯ 1. Yes, and keep allowing any reads outside the working directories
   2. No, and block reads outside the working directories from now on
   3. No, and ask again next time
   4. Yes, but ask again next time

 Esc to cancel · Tab to amend
```

Choices 1 and 2 change a persistent setting; choices 3 and 4 apply to this read only. The dialog opens focused on choice 1, so Enter would select the persistent allow.

---

## Specification

### Scope

| In scope | Out of scope |
|---|---|
| A `permission_request` rule and allow/deny answer plans for the Read dialog in the existing profile `claude-2.1.302-auto-1` | A new profile or version boundary |
| A rule per sibling "… outside the working directories" dialog of Edit, Write, Glob or Grep that the capture step actually produces | Other tools, and uncaptured sibling dialogs, which remain `unknown` frames and documented limits |
| Driver cases, a file-tool prompt branch and a private per-case Claude configuration in the test-only native-capture driver | Changes to the shipping runner beyond the profile data |
| Native 36x120 fixtures recorded on Claude Code 2.1.309, including a long-path operation over more than one row | Narrower renderings, which are a documented limit |
| Treating `\r\n` as a line ending in the plain-text capture cleanup | Changing `--ansi` output, herdr argv, or the tmux capture path |

### Profile placement

The rule, its answer plans and its fixtures join `claude-2.1.302-auto-1`. That profile already applies to every Claude version at or above `2.1.302` and already cites 2.1.308 fixtures, so each new fixture keeps its own `native_version` `2.1.309` and `min_version` stays `2.1.302`. On a 2.1.302–2.1.308 host that never renders the dialog, the rule never matches. The existing rules, answer plans and fixtures are unchanged.

### Detection rule `claude-read-outside-permission`

| Field | Value |
|---|---|
| `id` | `claude-read-outside-permission` |
| `priority` | `100` |
| `observation` | `permission_request` |
| `region.screen` | `alternate` |
| `region.start_pattern` | `^ Read outside the working directories\s*$` |
| `region.end_pattern` | `^ Esc to cancel · Tab to amend\s*$` |
| `region.max_rows` | `24` |
| `region.cursor` | `inside` |
| `verified_noninteraction` | `false` |
| `sensitive` | `false` |

Line matchers end in `\s*$` so trailing repaint padding keeps matching, as in the existing Claude rules:

| Matcher id | Scope | Pattern |
|---|---|---|
| `heading` | line | `^ Read outside the working directories\s*$` |
| `operation` | region | `(?m)^╌{3,}\n(?P<operation> [^\n]+(?:\n [^\n]+)*)\n╌{3,}$` (the shared Claude operation matcher) |
| `question` | line | `^ Allow this read outside the working directories\?\s*$` |
| `focus` | line | `^ ❯ [1-9]\. ` |
| `keep_allowing` | line | `^ [❯ ] 1\. Yes, and keep allowing any reads outside the working directories\s*$` |
| `block_from_now` | line | `^ [❯ ] 2\. No, and block reads outside the working directories from now on\s*$` |
| `deny` | line | `^ [❯ ] 3\. (?P<label>No, and ask again next time)\s*$` |
| `allow` | line | `^ [❯ ] 4\. (?P<label>Yes, but ask again next time)\s*$` |

The values above are the expected starting point. Step 1 pins from the cells and long-path captures:

| Item | Pinned from |
|---|---|
| Every matcher literal | The captured row text; when it differs from the issue's transcript, the capture governs and the difference is recorded in the Changelog |
| `region.screen` | The fixture's `alternate_screen` |
| `region.cursor` | Where the cursor sits while the dialog is shown, across both captures (`inside`, or `any` when it lies outside the region) |
| `region.max_rows` | `24`, raised to the long-path capture's heading-to-`Esc to cancel · Tab to amend` row span when that span exceeds it |

Step 1 also confirms that the heading, the question and each choice row occupy one logical line at 120 columns. The detector joins soft-wrapped rows into one logical line before matching, while a hard-broken label would split its `\s*$`-anchored matcher and make the frame `unknown`. The explanatory paragraph has no matcher, so its rows wrap freely.

Extraction:

```json
"extract": {
  "operation": {"matcher_id": "operation", "capture": "operation", "row_margin": " "},
  "choices": [
    {"id": "allow-once", "label": {"matcher_id": "allow", "capture": "label"}, "scope": "single_request"},
    {"id": "deny-once", "label": {"matcher_id": "deny", "capture": "label"}, "scope": "single_request"}
  ],
  "native_request_id": null
}
```

The operation is the displayed `Read(<path> · lines <a>-<b>)` rows with the one-space margin removed and rows joined by line breaks, exactly as the Bash rules report theirs; a long path over several rows sets `operation_rows` above 1 and extends the Director's review instruction through the existing sentence. The evidence `choices` are `["Yes, but ask again next time", "No, and ask again next time"]`. `keep_allowing` and `block_from_now` are matched only so the rule binds the whole live form; they declare no choice and receive no answer plan.

### Sibling file-access dialogs

The probe set is exactly Edit, Write, Glob and Grep. Each probe runs one harmless operation inside the case's outside directory (§ Native capture procedure):

| Tool | Probe operation | Allow evidence |
|---|---|---|
| Edit | Replace `before` with `after` in `edit-target.txt` (content `before`) | The file reads `after` |
| Write | Write `write-target.txt` with content `cafleet-199-write-probe` | The file exists with that content |
| Glob | Pattern `*.txt` in the outside directory | The member reports the matched `probe.txt` |
| Grep | Pattern `cafleet-199-read-probe` in the outside directory | The member reports the match in `probe.txt` |

The cells case of each tool decides its profile change:

| Capture outcome | Profile change |
|---|---|
| The tool shows its own "… outside the working directories" heading, question or choice set | Add rule `claude-<tool>-outside-permission` (lowercase tool name), structured exactly as the Read rule with that dialog's literals, its single-request choices as `allow-once`/`deny-once` and its persistent choices matched without plans. Its allow/deny `answer_stage` cases run its digit check and record its pre-stage fixtures; it receives answer plans only when that check passes, and otherwise ships detection-only |
| The tool shows the Read dialog's literals | The Read rule already matches it; add its capture as a further detection fixture |
| The tool shows no dialog, an automatic refusal, or a dialog without an "outside the working directories" heading | No rule; record the outcome as a limit |

### Answer plans

Digit selection is focus-independent, so the plans never move focus and never press Enter. Both are one ungated stage:

| Decision | `answer_rule_id` | `choice_id` | Stage `bytes_hex` | `required_rule_id` / `required_choice_id` | Fixtures |
|---|---|---|---|---|---|
| allow | `claude-read-outside-allow-digit` | `allow-once` | `34` | `claude-read-outside-permission` / `allow-once` | allow pre-stage capture |
| deny | `claude-read-outside-deny-digit` | `deny-once` | `33` | `claude-read-outside-permission` / `deny-once` | deny pre-stage capture |

Each entry uses `request_rule_id` `claude-read-outside-permission` and `focus_matcher_id` `focus`.

**Digit check.** Step 1 establishes, on fresh dialogs, that typing `4` alone closes the dialog and performs the read, and that typing `3` alone closes it without the read, with no Enter. If either digit only moves focus or does nothing, add no answer entry for this rule: detection stays `tested`, `permission respond` reports no answer capability, and the Director asks the user to answer on the native surface.

### Detection capability and fixtures

`claude-read-outside-permission` (and each added sibling rule) joins `capabilities.detection.rule_ids`. Each new native fixture joins the bundled `fixtures` list with its SHA-256 digest and the embedded fixture table in the catalog source, and cites its capability group:

| Fixture id | Case | Group |
|---|---|---|
| `claude-2.1.309-read-outside-permission-1` | Dialog at 36x120 with a single-row operation | detection |
| `claude-2.1.309-read-outside-long-path-1` | Dialog at 36x120 whose operation spans two or more rows | detection |
| `claude-2.1.309-read-outside-allow-pre-stage-1` | Dialog captured immediately before byte `34` | allow answer |
| `claude-2.1.309-read-outside-deny-pre-stage-1` | Dialog captured immediately before byte `33` | deny answer |
| `claude-2.1.309-read-outside-allow-continuation-1` | Screen after allow | classification test only |
| `claude-2.1.309-read-outside-deny-continuation-1` | Screen after deny | classification test only |
| `claude-2.1.309-<tool>-outside-permission-1` | Each captured sibling dialog | detection |
| `claude-2.1.309-<tool>-outside-allow-pre-stage-1`, `claude-2.1.309-<tool>-outside-deny-pre-stage-1` | A sibling dialog with its own rule, captured immediately before its allow or deny digit | that rule's allow / deny answer |

Answer pre-stage fixtures are bundled only for a dialog whose digit check passes. Fixtures follow the existing native-fixture contract: 36 rows by 120 columns, alternate screen, reviewed privacy masking of home paths and account, model or plan text, with every rule region in unmasked rows.

### Native capture procedure

Captures use the test-only native-capture driver that produced the existing fixtures: the inline `native_capture` module of `cafleet/src/runtime/runner/observation.rs`, run through its ignored test `native_capture_prepare_start_driver` with its approved-case registry `APPROVED_CASES`. Every case uses backend `claude`, permission mode `auto` (the omitted default), Claude Code 2.1.309 on `PATH`, the driver's private database and dedicated tmux socket, and size `DRIVER_FULL_SIZE` (36x120).

**Driver changes.** The registry and its case validation today bind every tool-using case to the Bash `permission-probe` helper and its `allow.executed` / `deny.executed` markers. Extend the driver as follows:

| Change | Specification |
|---|---|
| File-tool binding | `ApprovedCase` gains an optional file-tool operation (tool name and probe operation from § Sibling file-access dialogs, or a Read of `probe.txt`). A file-tool case has no `probe_argument`, no helper and no markers; its evidence is the outside directory's state and the member's report |
| Outside directory | `<DRIVER_NEUTRAL_ROOT>/<case id>-outside`, a sibling of the case cwd `<DRIVER_NEUTRAL_ROOT>/<case id>`, owned by the operator with mode `0700`. The case setup writes `probe.txt` (content `cafleet-199-read-probe`) and `edit-target.txt` (content `before`); the long-path case places `probe.txt` under nested directories deep enough that the displayed `Read(…)` operation exceeds one 120-column row |
| Case validation | `run_native_capture_prepare_start` requires, for a file-tool case, the outside directory above instead of `<cwd>/permission-probe`, and refuses a case whose outside directory lies inside its cwd |
| Prompt branch | `Use the <Tool> tool to <probe operation> on exactly <absolute path>. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.` |
| Private Claude configuration | Each file-tool case passes `CLAUDE_CONFIG_DIR=<DRIVER_PRIVATE_ROOT>/<case id>-claude` through `child_environment`. The case setup creates it fresh (directory `0700`, files `0600`) as a copy of the seed configuration below, plus a `projects` entry for the case cwd with `hasTrustDialogAccepted: true` added to its `.claude.json` |

**Seed configuration.** The persistent outside-read setting's store is not established, so no operator file beyond the credential is copied. Step 1 builds the seed once, before any case:

1. Create `<DRIVER_PRIVATE_ROOT>/claude-seed` (`0700`) and copy only the operator's `.credentials.json` into it (`0600`).
2. Start Claude Code 2.1.309 interactively with `CLAUDE_CONFIG_DIR` set to the seed, `--permission-mode auto`, in a disposable cwd under `DRIVER_NEUTRAL_ROOT`. Complete first-launch onboarding on the native surface and send one tool-free prompt.
3. When `Teach auto mode about your environment?` appears, answer `3. Don't show again`; confirm a second launch shows neither onboarding nor that dialog.
4. Perform no file operation outside the cwd and exit. The seed then holds `.credentials.json`, the `.claude.json` Claude wrote (onboarding, auto-mode and account flags) and no `settings.json`.

If onboarding or the auto-mode startup dialog still appears in a case, the case sends no bytes and stops unpublished; rebuild the seed and rerun the case. After the last Step 1 case, delete the seed directory.

**Persistent-setting check.** Before each case starts, the driver snapshots every JSON file in the case's private configuration. After the case stops, it compares them and records each changed key path. The cells case of the same dialog sends no answer, so its changed keys are the session-bookkeeping baseline. An allow or deny case passes only when its changed keys equal that baseline and no `settings.json` or `settings.local.json` appeared; any other change fails the digit check for that dialog and is reported to the Director.

**Teardown.** Each case's stop removes its private configuration directory, which holds a credential copy, after the persistent-setting check, including when the case fails.

The driver never sends `1`, `2`, Enter or Tab; registry validation already refuses Escape.

| Case | Driver mode | Decision | Stage bytes | Supplies |
|---|---|---|---|---|
| `claude-read-outside-cells-001` | `no_answer` | — | — | The single-row detection fixture and the native literals |
| `claude-read-outside-long-001` | `no_answer` | — | — | The multi-row detection fixture |
| `claude-read-outside-allow-001` | `answer_stage` | `allow` | `34` | The allow pre-stage fixture, the digit check for `4`, the continuation screen and allow evidence (`cafleet-199-read-probe` in the member's report) |
| `claude-read-outside-deny-001` | `answer_stage` | `deny` | `33` | The deny pre-stage fixture, the digit check for `3`, the continuation screen and deny evidence (no probe content reported) |
| `claude-<tool>-outside-cells-001` | `no_answer` | — | — | One case per tool in the probe set |
| `claude-<tool>-outside-allow-001`, `claude-<tool>-outside-deny-001` | `answer_stage` | `allow` / `deny` | The digit of that dialog's single-request allow / deny choice | Only for a tool whose cells case showed its own dialog; its digit check, pre-stage fixtures and evidence |

Each case gets its own database and socket name following the registry's existing naming. A sibling answer case's stage bytes are a reviewed registry edit made after its cells capture shows the choice numbering.

Before sending any stage byte, capture the pane and confirm the dialog is current. The cells case also confirms that Claude treats the outside directory as outside the working directories. If `claude-read-outside-cells-001` shows no Read dialog (for example, the read proceeds or is refused without asking), stop and report `blocked` to the Director: every later step depends on that dialog.

Record the CAFleet commit, Claude Code version, model, geometry, masking review and every dialog that could not be produced in the Execution record.

### Capture line endings

`strip_ansi` (the plain-text cleanup behind `member capture` without `--ansi`) treats a `\r` immediately before `\n` as part of the line ending:

```rust
pub(crate) fn strip_ansi(text: &str) -> String {
    let cleaned = ANSI_ESCAPE_RE.replace_all(text, "");
    cleaned
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            line.rsplit('\r').next().expect("rsplit yields a segment")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
```

| Raw line | Plain output |
|---|---|
| `text\r\n` | `text` |
| `old\rnew\r\n` | `new` |
| `old\rnew` (final line) | `new` |
| `\r\n` | empty line |

`capture_window` is unchanged: it splits on `\n`, and a whitespace-only line ending in `\r` is already visually blank, so the blank-tail trim still applies. `--ansi` output, the `content_sha256` over emitted content and the tmux path are unchanged. The fix applies to every herdr pane, wrapped or not.

### Director workflow

Unchanged. The Director consumes `interaction_opened`, verifies it with `event show`, inspects with `permission inspect`, decides under its own effective mode (Director-auto allow/deny in verified auto mode; the user's request-specific permission otherwise), submits with `permission respond`, then inspects the native result and the member's continuation. A read outside the working directories is reviewed like any other operation: the path in the operation is the risk evidence.

### Limits

| Limit | Behavior |
|---|---|
| Panes narrower than 120 columns | Only 120-column renderings are claimed; a narrower rendering whose rows differ from the pinned literals is `unknown`, input stays on the existing fallback rules, and the dialog is answered on the native surface |
| Persistent choices 1 and 2 | Never selected by CAFleet; a user who wants them answers on the native surface |
| `Tab to amend` | Not offered by CAFleet |
| Probe-set tools that produced no dialog, and tools outside the probe set | No rule; an outside-working-directory dialog from them is an `unknown` frame with no event, answered on the native surface |
| Digit check failure for a dialog | That rule is detection-only; both of its answers are manual |

### Documentation targets

| Target | Change |
|---|---|
| `SPEC.md` § Bundled profiles and native screen fixtures | Profile `claude-2.1.302-auto-1` capability table: the new detection rule(s), allow digit `4` and deny digit `3` for the Read rule; a paragraph describing the dialog, its matchers, extraction, the persistent choices without plans, the 2.1.309 captures and the limits above |
| `SPEC.md` § `member capture` | `--ansi` bullet: the default strips ANSI escapes, treats a carriage return directly before a line feed as part of the line ending, and cleans the remaining carriage-return redraws |
| `skills/cafleet/reference/cli.md` § `member capture` | The same cleanup sentence |
| `docs/docs/concepts/member-permission-events.md` | The automatic-mode sentence also names Claude's dialog before reading outside the working directories as a native form that follows the same request flow |
| `skills/cafleet/reference/coding-agents.md` § claude Note → applies at | The omitted-`auto` note states that Claude's file-access dialog for reads outside the working directories is a native permission request handled through the runtime-event procedure with single-request answers only |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Capture native evidence

- [ ] Extend the native-capture driver in `cafleet/src/runtime/runner/observation.rs` per § Native capture procedure: the file-tool binding, outside-directory setup and validation, the file-tool prompt branch, the private `CLAUDE_CONFIG_DIR` copied from the seed, the persistent-setting check, the private-configuration teardown, and the Read and sibling `cells` cases in `APPROVED_CASES`. <!-- completed: -->
- [ ] Build the seed configuration per § Native capture procedure and confirm a second launch shows neither onboarding nor the auto-mode startup dialog. <!-- completed: -->
- [ ] Run `claude-read-outside-cells-001` and `claude-read-outside-long-001` on Claude Code 2.1.309; pin every matcher literal and `region.screen`, `region.cursor` and `region.max_rows` per § Detection rule `claude-read-outside-permission`, and confirm one logical line per matched row. Report `blocked` if no Read dialog appears. <!-- completed: -->
- [ ] Run `claude-read-outside-allow-001` and `claude-read-outside-deny-001`, recording the digit check outcome, pre-stage fixtures, continuation screens and native results. <!-- completed: -->
- [ ] Run `claude-<tool>-outside-cells-001` for Edit, Write, Glob and Grep and apply the § Sibling file-access dialogs table; for each tool with its own dialog, register its allow/deny cases with the reviewed digits and run them. <!-- completed: -->
- [ ] Review masking, convert each capture into a native fixture, and report the capture outcomes (including any digit-check failure or unproduced dialog) to the Director before Step 2. <!-- completed: -->

### Step 2: Update documentation

- [ ] Update `SPEC.md` § Bundled profiles and native screen fixtures and § `member capture` per § Documentation targets, with the literals pinned in Step 1. <!-- completed: -->
- [ ] Update `docs/docs/concepts/member-permission-events.md`. <!-- completed: -->
- [ ] Update `skills/cafleet/reference/cli.md` and `skills/cafleet/reference/coding-agents.md` (claude notes). <!-- completed: -->

### Step 3: Fix capture line endings

- [ ] Extend the `strip_ansi` test in `cafleet/src/output/render.rs` with the § Capture line endings table rows, and add a `CaptureSnapshot::from_raw` test in `cafleet/src/capture.rs` with CRLF input asserting every line survives in plain mode and the raw bytes survive with `ansi`. <!-- completed: -->
- [ ] Add a herdr `capture_pane` test in `cafleet/src/multiplexer/herdr.rs` whose fake reply uses CRLF lines and a blank CRLF tail, asserting the kept window. <!-- completed: -->
- [ ] Implement the `strip_ansi` change. <!-- completed: -->

### Step 4: Add the profile rule

- [ ] Add the Step 1 fixtures to `cafleet/src/coding_agent/profiles/native/`, their entries and digests to `bundled.json`, and their ids to the embedded fixture table in `catalog.rs`. <!-- completed: -->
- [ ] Write catalog tests in `catalog.rs`: each new fixture classifies with its expected rule (dialogs as `permission_request` from `claude-read-outside-permission`, continuations as the idle composer or their observed class); complete match evidence, the extracted operation (single-row and multi-row with `operation_rows`) and the two single-request choice labels; and the existing classification rows unchanged. <!-- completed: -->
- [ ] Write answer-plan tests: allow resolves to one ungated stage `4` and deny to one ungated stage `3` for `claude-read-outside-permission` under profile `claude-2.1.302-auto-1`, each sibling rule resolves to its reviewed digits, and no plan selects a persistent choice; a rule whose digit check failed has no answer plan. <!-- completed: -->
- [ ] Write detector tests in `detector.rs`: trailing repaint padding keeps the request identity, a quoted dialog in the transcript outside a live region and the idle composer produce no request. <!-- completed: -->
- [ ] Add `claude-read-outside-permission` and each captured sibling rule, with the answer plans whose digit checks passed, to `claude-2.1.302-auto-1` in `bundled.json`. <!-- completed: -->
- [ ] Add a test in `cafleet/src/runtime/runner/input.rs` `mod tests`: replaying `claude-2.1.309-read-outside-permission-1` queues a profile preview with zero bytes, and replaying `claude-2.1.302-auto-idle-composer-1` afterwards drains it once. <!-- completed: -->

### Step 5: Validate and accept

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`. <!-- completed: -->
- [ ] On tmux and on herdr, with the release build and a disposable Claude Code 2.1.309 `auto` member reading an outside file: confirm `interaction_opened` with `review_permission_request`, a non-null `current_interaction_id`, a preview sent during the dialog queued and delivered once after closure, Director-auto allow (read performed, `interaction_closed`, continuation) and Director-auto deny (no read, closure, continuation) on fresh dialogs. <!-- completed: -->
- [ ] On one multiplexer, answer a fresh dialog manually on the native surface and confirm a later `permission respond` with the earlier token is refused before any byte. <!-- completed: -->
- [ ] On herdr, confirm `cafleet member capture <id> --lines 40` shows every line of the open dialog, matches the § Success Criteria comparison against `herdr pane read <pane> --format text --source recent-unwrapped --lines 1040`, and that `--ansi` still emits the raw bytes. <!-- completed: -->
- [ ] Record versions, geometry, latencies, outcomes and every unobserved case in an Execution record section of this document and as limits in `SPEC.md`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-31 | Initial draft |
| 2026-10-31 | Review round 1: scoped the no-persistent-key guarantee to CAFleet answer and queued-input paths; specified the herdr capture comparison; pinned region screen, cursor and row span from captures and required one logical line per matched row; fixed the sibling probe set to Edit, Write, Glob and Grep with per-sibling answer cases; located the native-capture driver and specified its file-tool cases, outside directory and private Claude configuration; `blocked` when no Read dialog appears; named the runner input test file and fixtures |
| 2026-10-31 | Review round 2: the private Claude configuration is copied from a natively built seed holding only the credential and Claude's own `.claude.json`; a startup dialog in a case stops it with no bytes; a per-case persistent-setting check against the cells baseline; teardown removes each private configuration and the seed |
| 2026-10-31 | Approved by the Reviewer and the user |
