# Claude Auto-Mode Nudge Dialogs and Claude Detection Gaps

**Status**: Approved
**Progress**: 0/33 tasks complete
**Last Updated**: 2026-11-04

## Overview

The Director answers Claude Code's two auto-mode nudge dialogs — "Make auto mode your default permission mode?" (newly detected) and "Teach auto mode about your environment?" (already detected) — through `cafleet permission respond --choice`, a request-bound answer to a `question` interaction that selects only a tested dismissal choice. The bundled Claude profile also closes every Claude detection gap reproduced on Claude Code 2.1.312: the primary-screen onboarding session, the `default`-mode `Read file`/`Edit file`/`Create file` forms, the Tip-less three-choice Bash form, and forms drawn beside the diff sidebar.

## Success Criteria

- [ ] On Claude Code 2.1.312 under the runner, "Make auto mode your default permission mode?" yields a `question` observation from rule `claude-auto-default-nudge`, an `interaction_opened` event with `director_action.kind=review_question`, and a non-null `runtime.current_interaction_id`; a preview, a `member prompt` and a `member exec` sent while it is open each queue with zero bytes and drain once after it closes.
- [ ] `permission inspect` on that interaction returns `answer_capability=tested`, a token and `supported_choices=[{"choice_id":"decline",…,"scope":"persistent"}]`; `permission respond --choice decline` sends exactly `0e` while `Yes` is focused, then `0d` while `No, keep …` is focused, closes the dialog and changes only `hasSeenAutoDefaultNudge` in the native configuration.
- [ ] "Teach auto mode about your environment?" yields a `question` from `claude-auto-startup` with `supported_choices` `not-now` and `dont-show-again`; `--choice not-now` sends exactly byte `32` and `--choice dont-show-again` exactly byte `33`, each changing only its documented `autoModeEnvSetup` key. The `1. Yes` wizard is a separate `question` from `claude-auto-env-setup-wizard` with no answer capability.
- [ ] No answer plan, `permission respond` or queued input selects `Yes, set auto mode as my default permission mode` or `1. Yes`, and no answer stage contains an Escape byte.
- [ ] The notice `Claude has a question for you — it shows once you send or clear what you're typing.` yields a `question` from `claude-question-pending-notice`, which stops a profile submission before its submit key; the interaction closes when the user clears the draft (Dialog A then opens its own episode) or sends it.
- [ ] In the Claude process that performs first-launch onboarding (primary screen), the idle composer and the auto-tip Bash form are detected by their existing rules, and previews go out through the profile.
- [ ] In `default` mode, a read, edit or file creation outside the working directories yields a `permission_request` from `claude-read-file-permission`, `claude-edit-file-permission` or `claude-create-file-permission`; with `permissions.disableAutoMode` `disable`, a Bash command yields one from `claude-bash-permission-no-tip`. Each is answered by `--decision allow` (byte `31`) and `--decision deny` (byte `33`) when its digit checks passed, and is detection-only otherwise.
- [ ] With the diff sidebar open, the auto-tip Bash form drawn left of the sidebar separator yields the same `permission_request`, evidence and answers as without the sidebar.
- [ ] `mise //:test`, `mise //:lint`, `mise //:typecheck` and `mise //docs:build` pass, and live acceptance passes on both tmux and herdr with the release build and a private Claude configuration per member.

---

## Background

| Symptom (2.1.312; `.probe-findings.md`, `.q6-findings.md`, `.f-findings.md`, `.round2-findings.md`) | Cause |
|---|---|
| "Make auto mode your default permission mode?" produces no event; programmatic input is not held behind it. A profile-delivered preview, `member prompt` or `member exec` (literal, then Enter) loses its text and its Enter accepts `Yes`, switching the operator's user default permission mode to `auto` for every Claude session (reproduced three times); Escape-first fallback delivery declines the dialog instead | No Claude rule matches the dialog |
| "Teach auto mode about your environment?" arrives as a `question` that the Director can only hand to the user | Answer plans may target only `permission_request` rules with `allow`/`deny` decisions (`answer requires permission form`); `permission respond` accepts only `--decision`; the response audit CHECKs `decision IN ('allow','deny')`; focus-gated plans resolve focus only from cell backgrounds |
| The process that ran first-launch onboarding is undetected for its lifetime; every input waits for the `idle_timeout` fallback | That process stays on the primary screen (`alternate_on=0`) with rows identical to the alternate-screen rendering; every Claude rule declares `region.screen` `alternate` |
| In `default` mode, ` Read file`, ` Edit file` and ` Create file` forms for paths outside the working directories produce no event | No rule matches their headings, questions and choices |
| A three-choice Bash form (description row, no Tip row, `3. No`) is reported as `interaction_obscured` / `unmatched-screen` | Claude omits the Tip and `switch to auto mode` choice whenever auto mode cannot be offered (`disableAutoMode`, unavailable plan/model, fast mode, an ask rule or hook, `plan` mode); no rule matches that form |
| With the diff sidebar open, auto-tip Bash forms produce no event | The sidebar draws a dim `│` at column 92 (166-column pane) and panel text on every form row, so the `\s*$`-anchored matchers fail. It auto-opens at ≥ 144 columns (≥ 110 with `.claude.json` `diffSidebarOpen` `true`) in a git repository after the working-tree change set changes; `diffSidebarOpen` `false` disables auto-open |

Dialog A is pending when onboarding is complete, `hasSeenAutoDefaultNudge` is unset, the server-side gate `tengu_maple_pier` is on, the user settings pin a non-`auto` `permissions.defaultMode` and no project, local, flag or policy settings set `defaultMode`. A `--permission-mode` launch flag does not suppress it. At startup it renders below the banner; after a turn it renders at the bottom; in the onboarding process it renders on the primary screen right after the folder-trust step. In each placement it replaces the composer, its borders and the footer badge, and the hidden terminal cursor sits at column 3 of the `No` row:

```
──────────────────────────────────────── (full-width rule)
 Make auto mode your default permission mode?

   Auto mode lets Claude handle permission prompts automatically. Claude checks each tool call for risky actions and
   prompt injection before executing, runs the ones it assesses as lower-risk, and blocks the rest.

   ❯ Yes, set auto mode as my default permission mode
     No, keep manual mode
```

The second label names the user-settings default mode (`No, keep manual mode` for `default`, `No, keep accept edits` for `acceptEdits`). Focus starts on `Yes` and is marked by the `❯` glyph and a foreground colour only. Down/Up and Ctrl+N/Ctrl+P (also `j`/`k`) move and wrap the focus, Enter confirms the focused choice, typed text (digits, Tab, Space, Ctrl+J and other printable keys) is discarded, and Escape resolves as decline. `Yes` writes `permissions.defaultMode: "auto"` into the user settings and switches the session; `No` and Escape set `hasSeenAutoDefaultNudge: true`. When a draft reaches the composer before the dialog mounts, the dialog stays hidden behind a notice row (§ Pending-question notice).

Dialog B appears in `auto` when `numStartups >= 5`, `autoModeEnvSetup.denials >= 5`, no auto-mode environment is configured, `autoModeEnvSetup.dismissed` is not set and `dismissedAt` is older than 7 days. `2. Not now` sets `autoModeEnvSetup.dismissedAt` (7-day snooze), `3. Don't show again` sets `autoModeEnvSetup.dismissed: true`, and `1. Yes` opens a multi-step wizard that scans repositories and shell history and writes settings. A single digit selects its choice without Enter.

---

## Specification

### Scope

The user put every reproduced gap in scope and accepted that the dismissal answers change the operator's Claude configuration.

| In scope | Out of scope |
|---|---|
| Choice answers for `question` interactions: catalog entries, `marker` focus, `permission inspect`/`respond`, storage, `review_question` Director action | Answers to `trust`, `update` and `unknown_interaction` classes |
| Rules `claude-auto-default-nudge` (with a `decline` plan) and `claude-question-pending-notice`; `claude-auto-startup` narrowed to the offer with `not-now`/`dont-show-again` plans; rule `claude-auto-env-setup-wizard` | Plans for `Yes, set auto mode as my default permission mode`, `1. Yes` and the wizard; Escape in any answer stage |
| `region.screen` `either` for every Claude rule | Primary-screen renderings that differ from the alternate screen (none observed) |
| Rules `claude-read-file-permission`, `claude-edit-file-permission`, `claude-create-file-permission`, `claude-bash-permission-no-tip` with single-request allow/deny plans | Their session- and project-scoped choice 2; Bash title variants `Run command`, `Bash command (unsandboxed)`, `Bash command (runs on <host>)` (not rendered) |
| Sidebar clipping (`panel_separator`) | Other right-hand panels |
| The Claude explicit-mode adapter entry for 2.1.312; driver cases with seeded private Claude configurations; 2.1.312 fixtures | A new profile or version boundary; auto-mode entry notices (informational) |

### Profile placement

Every rule, answer plan and fixture joins the Claude profile from `2.1.302`. Each new fixture keeps `native_version` `2.1.312`; `min_version` stays `2.1.302`; existing fixtures keep their versions. The Member eligibility verdict and its `Evidence:` line are unchanged: question dialogs are outside the approval-form requirement, and the new permission rules extend coverage of `manual` modes and recorded layouts without changing the classified default `auto`.

### Explicit-mode adapter for 2.1.312

`CodingAgent::policy_choices` records Claude's explicit choices for `2.1.312` exactly as for `2.1.302` (`default`, `manual`, `dontAsk`, `acceptEdits`, `auto`, `bypassPermissions`, `plan`), after Step 1 verifies them from `claude --help` and the parser on the installed 2.1.312. `SPEC.md` §6.9 Launch policy and capability registry names 2.1.312 beside 2.1.302 in the sentence on canonical `default` and its `manual` alias.

### Question answers in the catalog

`capabilities.answers` holds two entry forms, distinguished by the `decision` key:

| Form | Keys | Request rule | Choice scope | Uniqueness |
|---|---|---|---|---|
| Decision answer | `decision,request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `permission_request` | `single_request` | `request_rule_id` + `decision` |
| Choice answer | `request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `question` | any declared scope (`single_request`, `session`, `persistent`) | `request_rule_id` + `choice_id` |

Stage, gate, fixture and Escape validation are identical for both forms. Catalog errors: `answer requires permission form` (decision entry on another class), `choice answer requires question form` (choice entry on another class), and `answer requires single-request scope` for decision entries only. A choice id matches `^[a-z0-9]+(?:-[a-z0-9]+)*$` and is at most 64 bytes.

Bundled choice answers select only a dismissal choice. The scope is evidence for the Director, not a gate on the plan; the plan's declared `choice_id` is the only choice it can select.

Compiled types: `Answer.decision` becomes `Answer.key: AnswerKey`, an enum of `Decision(PermissionDecision)` and `Choice(String)`; `AnswerCapability` carries the same key; `ProfileCatalog::answer_plan` and the per-profile lookup take `(version, request_rule_id, &AnswerKey)`. `Detector::match_answer` re-resolves the plan by that key and accepts an observation of class `permission_request` for a decision key and `question` for a choice key, with the same complete-evidence and rule-identity checks.

### Marker focus

A focus matcher declares `focus_style` `background` (existing) or `marker`. Under `marker`, the focused choice is the one choice whose label matcher matched the same row as the focus matcher; no such choice, or several, resolve to no focused choice. The catalog accepts a focus-gated plan whose focus matcher declares either style (error `a focus-gated plan needs a focus_style matcher` otherwise). `match_answer` sets `focused_choice` from the declared style, and the focus identity string for `marker` equals the `background` form (capture plus focused choice).

### `permission inspect`

The `request` object gains `supported_choices` after `supported_decisions`:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq,evidence,missing_evidence
```

| Field | Permission request | Question |
|---|---|---|
| `supported_decisions` | tested subset of `allow,deny` | `[]` |
| `supported_choices` | `[]` | array of `{choice_id,label,scope}` (ordered keys), one per tested choice plan, in rule choice order; `label` is the extracted label |
| `answer_rule_id` | common rule of the supported decision plans | common rule of the supported choice plans when unique, else null |
| `missing_evidence` | unchanged | `[]` — the operation-centred facts do not apply to a question |
| `answer_capability` `tested` | visible permission request, non-empty decision set | visible question, non-empty choice set |

A question without a tested plan (the wizard, the pending notice, any other backend's question) returns `answer_capability=unknown`, null token, empty arrays and `capability_reason` `question has no tested answer`. The blocked-request reasons of §6.3 apply unchanged to questions. Text output keeps its layout; `answers=` lists supported decisions for a permission request and supported choice ids for a question.

### `permission respond --choice`

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]
```

- Exactly one of `--decision` and `--choice` is required (clap argument group; both or neither is a usage error, exit 2). A `--choice` value outside the choice-id shape is a usage error, exit 2.
- Provenance, retained-key, token, epoch, blocked-state and stage semantics are those of `--decision`. `director-auto` still requires `--director-mode auto`.
- Zero-byte `rejected` replies (exit 1, structured response):

| Condition | `reason` |
|---|---|
| `--choice` on a non-question interaction | `choice answers apply only to question interactions` |
| `--decision` on a question | `decision answers apply only to permission requests` |
| `--choice` naming no tested plan of the current question | `choice CHOICE_ID has no tested answer for this question` |

The response object gains `choice` after `decision`; exactly one of them is non-null:

```text
status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,decision_source,stage,output_seq,reason
Permission response: interaction=INTERACTION_ID run=RUN_ID decision=DECISION choice=CHOICE source=SOURCE status=STATUS input=INPUT_ID stage=STAGE output_seq=SEQUENCE
```

The runner's capability binding for a choice: backend/native version equal the run's, answer rule equals the audit's `answer_rule_id`, request rule equals the event evidence `rule_id`, and the plan key equals the requested choice. The token binds the full decision/choice-to-plan set.

### Storage

`migrations/V1__baseline.sql` `member_permission_responses` changes in place:

```sql
    decision TEXT CHECK(decision IS NULL OR decision IN ('allow','deny')),
    choice_id TEXT CHECK(choice_id IS NULL OR (length(choice_id) BETWEEN 1 AND 64
        AND choice_id NOT GLOB '*[^a-z0-9-]*')),
    ...
    CHECK((decision IS NULL) != (choice_id IS NULL))
```

`choice_id` follows `decision` in column order. The canonical payload digest covers `…,request_token,decision,choice_id,director_mode,…` with JSON null for the absent one. The immutable-audit insert requires a `permission_request` opening for a decision and a `question` opening for a choice. `input show` / `input reconcile` `permission_response` gains `choice_id` after `decision`; its text line becomes `Permission decision: interaction=… event=… decision=DECISION choice=CHOICE source=… mode=… director=… epoch=…` with `-` for null.

### Director action `review_question`

Every `interaction_opened` with classification `question` carries:

```json
{"kind":"review_question","instruction":"<QUESTION_INSTRUCTION>","inspect_argv":["cafleet","permission","inspect","<INTERACTION_ID>","--event-id","<EVENT_ID>","--from-member-id","<DIRECTOR_ID>","--json"]}
```

`QUESTION_INSTRUCTION` is: "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward."

`director_action.kind` gains `review_question`; payload schema version stays 2 (each cafleet version uses a fresh database).

### Interaction frame guards

An interaction rule may declare `frame` (today annotation-only). For an interaction rule, `require_patterns` may be empty and `reject_patterns` must be non-empty; annotation frames keep their non-empty `require_patterns` requirement. Catalog error for an interaction frame with no reject pattern: `interaction frame requires a reject pattern`.

### Dialog A rule `claude-auto-default-nudge`

| Field | Value |
|---|---|
| `priority` / `observation` | `100` / `question` |
| `region.screen` | `either` |
| `region.start_pattern` | `^ Make auto mode your default permission mode\?\s*$` |
| `region.end_pattern` | `^   (?:❯ |  )No, keep \S.*$` |
| `region.max_rows` | `10`, raised to the 36x120 heading-to-last-choice span when larger |
| `region.cursor` | `inside`: the hidden cursor sits at column 3 of the `No` row both at startup and after a turn (measured with an argv-prompt launch, as CAFleet spawns members) |
| `region.panel_separator` | `│` |
| `frame` | `require_patterns` `[]`, `reject_patterns` `["^❯\\u{a0}"]` — the dialog replaces the live composer, so a quoted dialog above a live composer never matches |

| Matcher id | Pattern |
|---|---|
| `heading` | `^ Make auto mode your default permission mode\?\s*$` |
| `focus` | `^   ❯ \S`, `focus_style` `marker` |
| `accept` | `^   (?:❯ |  )(?P<label>Yes, set auto mode as my default permission mode)\s*$` |
| `decline` | `^   (?:❯ |  )(?P<label>No, keep \S.*?)\s*$` |

Choices: `accept` (`persistent`, no plan; declared so a focus gate can name it) and `decline` (`persistent`). Operation null. The `decline` label carries the user default's name, so the evidence shows which mode is kept; the label is part of the fingerprint. The explanatory paragraph has no matcher.

**Answer plan.** The arrow keys carry Escape bytes, which no answer stage may contain; Ctrl+N (`0e`) moves the focus from `Yes` to `No` and Enter confirms the focused choice:

| Stage | `bytes_hex` | Gate (marker focus before the stage) | Native effect |
|---|---|---|---|
| 1 | `0e` (Ctrl+N) | `accept` | Focus moves to `No, keep …` |
| 2 | `0d` (Enter) | `decline` | Dialog closes as decline; `hasSeenAutoDefaultNudge: true`; user settings unchanged |

Answer rule id `claude-auto-default-decline`, `choice_id` `decline`, `focus_matcher_id` `focus`; fixtures: the pre-stage capture (focus `accept`) and the post-stage-1 capture (focus `decline`). Ctrl+N wraps, so a focus already on `decline` (moved by a human) fails the stage-1 gate before any byte and the Director reinspects. The gate failure rules of § Request-bound permission responses apply between the stages.

### Pending-question notice `claude-question-pending-notice`

| Field | Value |
|---|---|
| `priority` / `observation` | `100` / `question` |
| `region` | `either`; start and end the notice row `^ *Claude has a question for you — it shows once you send or clear what you're typing\.\s*$`; `max_rows` `1`; cursor `any`; `panel_separator` `│` |
| `frame` | `require_patterns` `["^❯\\u{a0}"]`, `reject_patterns` `["^  Showing detailed transcript · ctrl\\+o to toggle"]` |
| matchers, choices, operation | the notice line; no choices; operation null; no answer plan |

Step 1 pins the notice row's exact margin and excludes the assistant transcript margin, so a model quote (`● …` or two-space continuation) does not match. As a candidate interaction, the notice stops a profile submission after its literal stage and before its submit key (§ Measured submission plans and settle intervals), and queues later input.

The notice appears only when a draft reaches the composer before Dialog A mounts (a narrow race), and it leaves the screen in either of two native outcomes, which close the notice interaction as not visible:

| User action on the native surface | Native outcome (2.1.312) |
|---|---|
| Clear the draft | The notice row disappears and Dialog A is drawn in its place at once; it opens its own `question` episode and is answered as § Dialog A specifies |
| Send the draft | The first Enter does not submit; the second Enter submits the draft and the notice row disappears. Dialog A does not reappear in that process, and `hasSeenAutoDefaultNudge` stays unset, so it returns at the next launch |

The Director receives `review_question` with no listed choice and asks the user to clear the draft on the native surface, so Dialog A appears and can be dismissed through `--choice decline`, or to send it with two Enters when the draft must run first; in the second case Dialog A returns at the member's next launch.

### Dialog B rules

`claude-auto-startup` narrows to the offer; the wizard gets its own rule, so the offer and the wizard are separate `question` episodes (the wizard opens only after a manual `1. Yes`).

| Field | `claude-auto-startup` (offer) | `claude-auto-env-setup-wizard` |
|---|---|---|
| `priority` / `observation` | `100` / `question` | `100` / `question` |
| `region.screen` / `panel_separator` | `either` / `│` | `either` / `│` |
| start | `^  Teach auto mode about your environment\?\s*$` | `^   Teach auto mode about your environment\?\s*$` |
| end | `^  Enter to confirm · Esc to cancel\s*$` | `^   ←/→ to change · Enter to continue · Esc to cancel\s*$` |
| `max_rows` / `cursor` | `14` / `inside` | `14` / `inside` |
| matchers | `heading`; `focus` `^  ❯ [1-9]\. `; `yes` `^  [❯ ] 1\. Yes\s*$`; `not_now` `^  [❯ ] 2\. (?P<label>Not now)\s*$`; `dont_show` `^  [❯ ] 3\. (?P<label>Don't show again)\s*$`; `footer` | `heading`; `focus` `^   ❯ \S`; `footer` |
| choices | `not-now` (`persistent`), `dont-show-again` (`persistent`) | none |

The two-space and three-space margins come from the 2.1.302 fixtures (`claude-2.1.302-auto-env-setup-offer-1` cursor (27,2), wizard cursor (30,3)); Step 1 confirms them on 2.1.312, and both 2.1.302 fixtures are reassigned to their rule.

| Answer rule | `choice_id` | Stage | Gate |
|---|---|---|---|
| `claude-auto-env-setup-not-now-digit` | `not-now` | `32` | ungated |
| `claude-auto-env-setup-dont-show-again-digit` | `dont-show-again` | `33` | ungated |

### Primary-screen rendering

The Claude process that performs first-launch onboarding stays on the primary screen for its lifetime, with the composer and forms in rows identical to the alternate-screen rendering (idle composer `❯` + U+00A0 with the cursor right after it; auto-tip form with the cursor on the focused choice row). That process also shows Dialog A right after the folder-trust step when the user settings pin a non-`auto` default. Every Claude rule changes `region.screen` from `alternate` to `either`. On the primary screen, earlier transcript rows stay visible above the live region; the cursor rules, the annotation frame guards and the transcript prompt prefix `❯` + U+0020 (not U+00A0) keep them from matching. Step 1 captures the primary idle composer, the auto-tip form, the screen after the form closes and the screen after a resize, and a detector test asserts that no remnant row yields a request. A primary-screen form whose rows differ from its alternate-screen literals is reported to the Director before any rule change.

### Diff sidebar clipping

A region may declare `panel_separator`, a single non-whitespace glyph. When any rule of the profile declares it, the detector derives a clipped line set once per screen:

1. A row is a panel row when, at some column `C` with `40 <= C < cols - 1`, the cell holds the separator glyph, its background differs from the cell at `C - 1` and equals the cell at `C + 1`.
2. Every panel row of the screen must share one `C`; otherwise no row is clipped.
3. Each panel row keeps cells `0..C-1`; other rows are unchanged. A clipped row ends a logical line (never soft-joined to the next row).

A rule that declares `panel_separator` matches against the clipped set; region rows, matcher rows, label cells and cursor tests use the same row indexes. A boxed Bash operation's `│ ` gutter shares its row's background, so it is never a separator.

Measured on 2.1.312 (tmux, exact widths): the left-area cells use the default background, and the separator (a dim U+2502) and the first panel cell use indexed background 235. The separator sits at one column on every row from row 0 down to the row above the composer border; the composer border, composer row and footer stay full width with no separator. Its column depends on the pane width and is neither fixed nor proportional (110→70, 120→70, 144→80, 166→92, 200→110, 240→150), so the detector finds it per frame. The sidebar opens mid-turn after an allowed Edit of an untracked file in the repository at ≥ 144 columns and, once open, stays down to 110 columns.

Every Claude rule declares `│` except `claude-idle-composer` and `claude-auto-mode-indicator`, whose rows are the full-width composer and footer. The annotation rules whose regions end at the composer border keep that unclipped border row as their end row.

### Manual-mode file permission forms

In `default` mode, Claude Code 2.1.312 asks before a file tool touches a path outside the working directories; Edit first shows the Read form for its target. Each form has a bold heading, dashed `╌` rules, the question, three numbered choices and the footer ` Esc to cancel · Tab to amend`:

```
 Read file
╌╌╌…
 Read(/tmp/cafleet-f-out/probe.txt)
╌╌╌…
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, allow reading from /tmp/cafleet-f-out during this session
   3. No

 Esc to cancel · Tab to amend
```

```
 Edit file
 ../../../../tmp/cafleet-f-out/edit-target.txt
╌╌╌…
 1 -before
 1 +after
╌╌╌…
 Do you want to make this edit to edit-target.txt?
 ❯ 1. Yes
   2. Yes, and switch to accept edits (auto-approve file edits and common file commands) for this session; Yes, and always allow access to /tmp/cafleet-f-out for this
      session (shift+tab)
   3. No

 Esc to cancel · Tab to amend
```

The ` Create file` form matches the Edit form with the new file's content between the dashed rules and the question ` Do you want to create new.txt?`. The rules share: priority `100`, observation `permission_request`, `region.screen` `either`, `panel_separator` `│`, end `^ Esc to cancel · Tab to amend\s*$`, `max_rows` `24`, cursor `inside`, `focus` `^ ❯ [1-9]\. `, `allow` `^ [❯ ] 1\. (?P<label>Yes)\s*$`, `deny` `^ [❯ ] 3\. (?P<label>No)\s*$`, choices `allow-once` and `deny-once` (`single_request`). Choice 2 is session-scoped and has a binding matcher but no choice entry.

| Rule | Heading / start | Operation (extracted) | Question matcher | Choice-2 matcher |
|---|---|---|---|---|
| `claude-read-file-permission` | `^ Read file\s*$` | The shared operation matcher with `row_margin` `" "`: the `Read(<absolute path>)` rows | `^ Do you want to proceed\?\s*$` | `^ [❯ ] 2\. Yes, allow reading from \S` |
| `claude-edit-file-permission` | `^ Edit file\s*$` | Region matcher `(?m)^ Edit file\s*\n (?P<operation>[^\n]*\S)\s*\n╌{3,}$`: the path row | `^ Do you want to make this edit to \S.*\?\s*$` | `^ [❯ ] 2\. Yes, and switch to accept edits \S` |
| `claude-create-file-permission` | `^ Create file\s*$` | Region matcher `(?m)^ Create file\s*\n (?P<operation>[^\n]*\S)\s*\n╌{3,}$`: the path row | `^ Do you want to create \S.*\?\s*$` | `^ [❯ ] 2\. Yes, and switch to accept edits \S` |

The Edit and Create operation is the target path relative to the working directory; the diff rows are not extracted, and the Director reviews the content from the capture excerpt. The choice-2 matchers anchor only on the label's start, because the label wraps onto a continuation row. Step 1 pins every literal and the cursor at 36x120 and confirms that the heading, question and choice 1 and 3 rows occupy one row each.

### Three-choice Bash rule `claude-bash-permission-no-tip`

Captured at 120x36 with `permissions.disableAutoMode` `disable` in `default` mode (alternate screen, cursor on the focused choice row):

```
 Bash command
 Create empty file p4.txt in /tmp/cafleet-f-out
╌╌╌…
 touch /tmp/cafleet-f-out/p4.txt
╌╌╌…
 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and always allow access to /tmp/cafleet-f-out from this project
   3. No

 Esc to cancel · Tab to amend
```

| Field | Value |
|---|---|
| `priority` / `observation` | `100` / `permission_request` |
| `region` | `either`, start `^ Bash command\s*$`, end `^ Esc to cancel · Tab to amend\s*$`, `max_rows` `24`, cursor `inside`, `panel_separator` `│` |
| matchers | `heading`; shared operation matcher with `row_margin` `" "`; `question` `^ Do you want to proceed\?\s*$`; `focus` `^ ❯ [1-9]\. `; `allow` `^ [❯ ] 1\. (?P<label>Yes)\s*$`; `access` `^ [❯ ] 2\. Yes, and always allow access to \S.* from this project\s*$`; `deny` `^ [❯ ] 3\. (?P<label>No)\s*$` |
| choices | `allow-once`, `deny-once`; `access` has no entry |
| answers | allow `31` (`claude-bash-no-tip-allow-digit`), deny `33` (`claude-bash-no-tip-deny-digit`), one ungated stage each |

`claude-bash-permission` requires the approval line and `4. No`, `claude-bash-permission-auto-tip` requires `3. Yes, and switch to auto mode …` and `4. No`, and `claude-auto-ask-permission` requires the `│ Ask rule` row and `2. No`, so the four Bash rules match disjoint forms. The description row is model-authored text without a matcher. Measured natively: `1` ran the command; `3` refused it with `Interrupted · What should Claude do instead?`.

### File and Bash answer plans

| Rule | Allow | Deny | Measured on 2.1.312 |
|---|---|---|---|
| `claude-read-file-permission` | `31`, `claude-read-file-allow-digit` | `33`, `claude-read-file-deny-digit` | `1` read; `3` refused with `Interrupted · What should Claude do instead?` |
| `claude-edit-file-permission` | `31`, `claude-edit-file-allow-digit` | `33`, `claude-edit-file-deny-digit` | `1` applied the edit; `3` not measured |
| `claude-create-file-permission` | `31`, `claude-create-file-allow-digit` | `33`, `claude-create-file-deny-digit` | `3` refused with `User rejected write to <path>`; `1` not measured |
| `claude-bash-permission-no-tip` | `31` | `33` | `1` ran; `3` refused |

Each plan is one ungated stage and ships only when its Step 1 digit check passes: the digit alone closes a fresh form and the operation runs (allow) or does not run (deny), with no Enter and no persistent change. A failed check leaves that decision unsupported.

### Native capture procedure

Capture precedes the documentation step because `SPEC.md`, the docs and the skills must carry the literals, margins and digit outcomes that only the 2.1.312 captures establish; the driver is test-only code, and the adapter entry is the one production line the capture needs.

Captures use the test-only native-capture driver (inline `native_capture` module of `src/runtime/observation.rs`, ignored test `native_capture_prepare_start_driver`, registry `APPROVED_CASES`) with Claude Code 2.1.312 on `PATH`, size `DRIVER_FULL_SIZE` (36x120) unless a row says otherwise, and the driver's private database and tmux socket per case.

**Seed configuration.** Before any case, build `<DRIVER_PRIVATE_ROOT>/claude-seed` (`0700`):

1. Copy only the operator's `.credentials.json` into it (`0600`).
2. Start Claude Code 2.1.312 interactively with `CLAUDE_CONFIG_DIR` set to the seed, in a disposable cwd under `DRIVER_NEUTRAL_ROOT`; complete onboarding on the native surface, send one tool-free prompt and exit.
3. Confirm a second launch shows neither onboarding nor a nudge dialog.

The driver accepts the seed only when it holds `.credentials.json`, `.claude.json` and a theme-only `settings.json`, no `settings.local.json`, and its `.claude.json` holds neither `hasSeenAutoDefaultNudge` nor `autoModeEnvSetup`; it refuses every case otherwise. The seed is deleted after the last case.

**Driver changes.**

| Change | Specification |
|---|---|
| `ClaudeConfigCase` | `ApprovedCase` gains `claude_config: Option<ClaudeConfigCase>` with variants `Seeded { settings, global }` and `Fresh { settings }`. `Seeded` copies the seed and applies the literal `settings` and `global` (`.claude.json`) overlays' top-level keys, replacing equal keys; a settings overlay may hold only `tui` and `permissions`. `Fresh` copies only `.credentials.json` plus its `settings` overlay as `settings.json`, so Claude runs first-launch onboarding. Every case with `claude_config`, tool-free or file-tool, gets `CLAUDE_CONFIG_DIR=<DRIVER_PRIVATE_ROOT>/<case id>-claude`; file-tool cases default to `Seeded` with empty overlays |
| Overlay placeholders | Inside an overlay string, `{outside}` expands to the case's absolute outside directory and `{cwd}` to its absolute cwd before the overlay is applied; a case whose overlay holds `{outside}` without a file probe is refused |
| Launch policy | `requested_mode()` returns none when `permission_mode` equals the backend default (`auto`) and the mode otherwise; on 2.1.312 the explicit modes resolve through the adapter entry |
| Stage guard | The guard refusing `31`, `32`, `0d`, `0a`, `09` applies to file-tool cases in `auto`, whose dialog is the read-outside form with persistent choices 1–2. A `default` file-tool case may send only `31` or `33` |
| Fresh onboarding | A `Fresh` case is `no_answer`; the implementer completes onboarding on the case's tmux pane (`tmux -L <socket> send-keys`) as manual native input, after which the spawn prompt runs |
| Persistent-setting check | Runs for every case with `claude_config`. The driver compares the answer case's changed key paths with its baseline case's (table column below) plus exactly the answer's documented change, including `settings.json`, requires that no other settings file appeared, fails the case on any difference and publishes both path lists |
| Sidebar cases | Size `DRIVER_PANEL_SIZE` (36x166) or `DRIVER_PANEL_MIN_SIZE` (36x144). Case setup makes the cwd a fresh git repository with one committed file and an untracked `panel-probe.txt` (content `before`), and seeds the allow rule `Edit(/{cwd}/panel-probe.txt)`. A sidebar prompt branch asks the member to replace `before` with `after` in that file with the Edit tool, then run the Bash probe helper; the allowed Edit opens the sidebar mid-turn, so the Bash form renders beside it. The driver confirms the separator is visible before the form's capture |

| Answer | Documented change |
|---|---|
| Dialog A `decline` | `.claude.json` `/hasSeenAutoDefaultNudge` → `true` |
| Dialog B `not-now` | `.claude.json` `/autoModeEnvSetup/dismissedAt` set |
| Dialog B `dont-show-again` | `.claude.json` `/autoModeEnvSetup/dismissed` → `true` |
| Every permission answer | none |

| Case | Mode | `claude_config` | Driver mode | Stage bytes | Baseline | Supplies |
|---|---|---|---|---|---|---|
| `claude-auto-default-cells-001` | `auto` | `Seeded`, settings `{"permissions":{"defaultMode":"default"}}` | `no_answer` | — | — | Detection fixture, literals, cursor |
| `claude-auto-default-decline-001` | `auto` | same | `answer_stage` | `0e`, then `0d` (gated) | `claude-auto-default-cells-001` | Pre-stage and post-stage-1 fixtures, continuation |
| `claude-auto-default-accept-edits-cells-001` | `auto` | `Seeded`, settings `{"permissions":{"defaultMode":"acceptEdits"}}` | `no_answer` | — | — | Second-label variant |
| `claude-question-pending-cells-001` | `auto` | as `claude-auto-default-cells-001` | `no_answer` | — | — | Notice fixture, then the after-clear fixture (Dialog A drawn, notice gone); the implementer types a draft on the pane before the turn ends and clears it with Backspace |
| `claude-question-pending-cells-002` | `auto` | same | `no_answer` | — | — | Notice fixture, then the after-send fixtures: the screen after the first Enter (notice still shown) and after the second (notice gone, composer idle); the implementer sends the draft with two Enters |
| `claude-auto-env-setup-cells-001` | `auto` | `Seeded`, global `{"numStartups":5,"autoModeEnvSetup":{"denials":5}}` | `no_answer` | — | — | Offer fixture |
| `claude-auto-env-setup-not-now-001` | `auto` | same | `answer_stage` | `32` | `claude-auto-env-setup-cells-001` | Pre-stage fixture, continuation |
| `claude-auto-env-setup-dont-show-again-001` | `auto` | same | `answer_stage` | `33` | `claude-auto-env-setup-cells-001` | Pre-stage fixture, continuation |
| `claude-primary-auto-tip-cells-001` | `default` | `Fresh {}` | `no_answer` | — | — | Primary idle composer, auto-tip form, closed-form and resize screens (Bash probe helper) |
| `claude-primary-auto-default-cells-001` | `auto` | `Fresh`, settings `{"permissions":{"defaultMode":"default"}}` | `no_answer` | — | — | Primary-screen Dialog A right after folder trust, with its cursor |
| `claude-read-file-cells-001`, `-allow-001`, `-deny-001` | `default` | `Seeded` | per row | — / `31` / `33` | the cells case | Fixtures and digit checks |
| `claude-edit-file-cells-001`, `-allow-001`, `-deny-001` | `default` | `Seeded`, settings `{"permissions":{"allow":["Read(/{outside}/**)"]}}` | per row | — / `31` / `33` | the cells case | Fixtures and digit checks; only the Edit form appears |
| `claude-create-file-cells-001`, `-allow-001`, `-deny-001` | `default` | `Seeded` | per row | — / `31` / `33` | the cells case | Fixtures and digit checks |
| `claude-bash-no-tip-cells-001`, `-allow-001`, `-deny-001` | `default` | `Seeded`, settings `{"permissions":{"defaultMode":"default","disableAutoMode":"disable"}}` | per row | — / `31` / `33` | the cells case | Fixtures and digit checks (Bash probe helper) |
| `claude-panel-auto-tip-cells-001` | `default` | `Seeded`, settings `{"permissions":{"allow":["Edit(/{cwd}/panel-probe.txt)"]}}` | `no_answer` | — | — | Auto-tip form beside the sidebar at 36x166 (separator column 92) |
| `claude-panel-auto-tip-cells-002` | `default` | same | `no_answer` | — | — | The same at 36x144 (separator column 80) |
| `claude-panel-auto-tip-allow-001` | `default` | same | `answer_stage` | `31` | `claude-panel-auto-tip-cells-001` | Allow pre-stage fixture beside the sidebar; the command runs |

Allow evidence is the existing file-tool evidence (the member reports `cafleet-199-read-probe`, `edit-target.txt` reads `after`, `write-target.txt` exists) or the probe helper's `allow.executed` marker; deny evidence is its absence. The driver never sends Escape, `1` to Dialog B, or any byte selecting a "Yes" choice of a nudge dialog. Before each stage it captures the pane and confirms the expected form is current. A case whose dialog does not appear (for example `tengu_maple_pier` off) sends no bytes and stops unpublished, and the implementer reports `blocked` to the Director. Teardown removes each private configuration after the check, including on failure. Fixtures follow the existing contract: reviewed masking of banner, account, model, plan and home paths, every rule region in unmasked rows; primary-screen fixtures record `alternate_screen` `false`.

### Director guidance

| Interaction | Director auto mode | Other modes |
|---|---|---|
| Dialog A | `--choice decline` | Relay; the user picks `decline` or answers natively |
| Dialog B offer | `--choice not-now`; ask the user before `dont-show-again` | Relay with both choices |
| Pending notice | Ask the user to clear the draft natively (Dialog A then follows), or to send it with two Enters | Same |
| Wizard, question without choices | Ask the user to answer natively | Same |
| File and Bash permission forms | The existing permission-request procedure; the path or command is the risk evidence | Same |

### Limits

| Limit | Behavior |
|---|---|
| `Yes` choices of both dialogs and the wizard | Answered by the user on the native surface |
| User defaults other than `default` / `acceptEdits` | The `decline` matcher accepts any `No, keep …` label; uncaptured labels are claimed through that pattern only |
| Server-side gate | A host where `tengu_maple_pier` is off never shows Dialog A |
| Bash title variants `Run command`, `Bash command (unsandboxed)`, `Bash command (runs on <host>)` | Not rendered in the captures; an `unknown` frame |
| File forms in `acceptEdits` / `plan`, wrapped paths or questions, panes narrower than 120 columns | Uncaptured; a differing rendering is an `unknown` frame |
| Sidebar widths other than 144 and 166 columns | The separator is found per frame, so other widths follow the same clipping; fixtures exist only for the two captured widths |

### Documentation targets

| Target | Change |
|---|---|
| `SPEC.md` §6.3 Permission commands, Permission-response audit output, Error Messages | `supported_choices`, `--choice`, response `choice`, rejection reasons, audit `choice_id`; the sentence "sends only a tested single-request native choice" becomes "sends only a tested single-request choice for a decision, or a tested declared dismissal choice for a question" |
| `SPEC.md` §6.9 Launch policy and capability registry | 2.1.312 in the Claude explicit-mode sentence |
| `SPEC.md` §6.9 Bundled profiles | Claude capability table: a question-answer row, `either` screens and every new rule; the sentence "Every bundled answer plan selects a single-request choice: a broadened, persistent or session-wide choice has no answer plan" becomes "Every decision answer plan selects a single-request choice, and every choice answer plan selects a declared dismissal choice of a question; other broadened, persistent or session-wide choices have no answer plan"; Dialog A/B, pending-notice, primary-screen, sidebar, file-form and no-tip paragraphs; limits |
| `SPEC.md` §6.9 Operation extraction / rule format | `focus_style` `marker`, `panel_separator`, interaction `frame` |
| `SPEC.md` §6.9 Persistence (`director_action` kinds), Request-bound permission responses, Permission response storage, §8 | `review_question` and its instruction; choice answers; DDL |
| `docs/docs/concepts/members.md` | Consume a runtime event and Preserve a native request cover question choices; the `manual`-mode sentence names the file and no-tip forms |
| `docs/docs/quickstart.md` | The Teach-auto-mode paragraph becomes the Director-answered flow; Dialog A named |
| `skills/cafleet/reference/cli.md` | Subcommand summary, `permission inspect`/`respond` |
| `skills/cafleet/reference/coding-agents.md` § claude Note → applies at | Dialog A/B and pending-notice notes with the answer route and § Director guidance |
| `skills/cafleet/roles/director.md` | Launch-policy line and § Consume observed interactions: the question route through `--choice` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Capture native evidence

- [ ] Verify Claude 2.1.312's `--permission-mode` choices from `claude --help` and the parser, and add the 2.1.312 adapter entry in `src/coding_agent/mod.rs` with a `policy_choices` test. <!-- completed: -->
- [ ] Extend the driver per § Native capture procedure: `ClaudeConfigCase`, the `{outside}` token, launch policy, stage guard, persistent-setting comparison, sidebar size and repository setup, the new `APPROVED_CASES` entries, with driver unit tests for the guard, the token and the comparison. <!-- completed: -->
- [ ] Build the seed per § Native capture procedure and confirm its checks. <!-- completed: -->
- [ ] Run the Dialog A cells cases and both pending-notice cases; pin literals, `max_rows`, cursor and the notice margin; confirm the notice row is gone after clear and after the second Enter; report `blocked` if the dialog does not appear. <!-- completed: -->
- [ ] Run the Dialog A decline case and the Dialog B cells, `not-now` and `dont-show-again` cases. <!-- completed: -->
- [ ] Run both primary-screen cases; confirm identical rows (including Dialog A and its cursor) and no matching remnant after closure and resize. <!-- completed: -->
- [ ] Run the Read, Edit, Create and no-tip Bash cases; pin literals and cursor and record each digit check. <!-- completed: -->
- [ ] Run the sidebar cases at 166 and 144 columns; confirm the separator column and backgrounds of § Diff sidebar clipping and the allow digit beside the sidebar. <!-- completed: -->
- [ ] Review masking, convert captures into fixtures and report every outcome (failed digit check, unproduced dialog, differing primary row) to the Director before Step 2. <!-- completed: -->

### Step 2: Update documentation

- [ ] Update `SPEC.md` per § Documentation targets with the Step 1 literals. <!-- completed: -->
- [ ] Update `docs/docs/concepts/members.md` and `docs/docs/quickstart.md`. <!-- completed: -->
- [ ] Update `skills/cafleet/reference/cli.md`, `skills/cafleet/reference/coding-agents.md` and `skills/cafleet/roles/director.md`. <!-- completed: -->

### Step 3: Choice answers (tests first)

- [ ] Catalog tests in `src/coding_agent/catalog.rs`: a choice entry on a question rule loads with any scope; it is refused on a permission rule, with a `decision` key on a question rule, with an unknown choice and with a duplicate `request_rule_id`+`choice_id`; `focus_style` `marker` loads and gates; an interaction frame needs a reject pattern. <!-- completed: -->
- [ ] Implement `AnswerKey`, the two entry forms, the keyed `answer_plan` lookups, `marker` focus and interaction frames in `catalog.rs`. <!-- completed: -->
- [ ] Detector tests in `src/coding_agent/detector.rs`: `match_answer` matches a question for a choice key and refuses a decision key on a question and the reverse; `marker` focus resolves one focused choice and none for zero or several marker rows. <!-- completed: -->
- [ ] Implement the `match_answer` and `marker` changes in `detector.rs`. <!-- completed: -->
- [ ] Control tests in `src/runtime/control.rs`: inspection key order with `supported_choices`; question capability, token and reasons; respond rejection reasons; choice capability binding. <!-- completed: -->
- [ ] Implement `--choice` in `src/cli/runtime.rs` (argument group, shape check) and the runner path in `src/runtime/control.rs` and `src/runtime/input.rs`. <!-- completed: -->
- [ ] Storage tests and DDL in `migrations/V1__baseline.sql` and `src/broker/runtime/inputs.rs`: exactly-one CHECK, digest with `choice_id`, question-opening requirement, audit output. <!-- completed: -->
- [ ] `review_question` Director action and instruction in `src/broker/runtime/events.rs`, with tests. <!-- completed: -->

### Step 4: Sidebar clipping (tests first)

- [ ] Detector tests in `detector.rs` with generated screens: one shared separator column clips panel rows; differing columns or a same-background `│` (boxed gutter) clip nothing; a clipped row is never soft-joined; rules without `panel_separator` see unclipped rows. <!-- completed: -->
- [ ] Implement `panel_separator` loading in `catalog.rs` and the clipped line set in `detector.rs`. <!-- completed: -->

### Step 5: Profile rules

- [ ] Add the Step 1 fixtures to `src/coding_agent/native/claude.json`, `bundled.json` and the embedded fixture table in `catalog.rs`. <!-- completed: -->
- [ ] Catalog tests: each fixture classifies with its expected rule and evidence (operation, choice labels, `operation_rows`); offer and wizard are separate episodes; primary and sidebar fixtures yield the same rule and evidence as their alternate, full-width counterparts; existing classifications unchanged. <!-- completed: -->
- [ ] Answer-plan tests: Dialog A `decline` resolves to the two gated stages and Dialog B `not-now`/`dont-show-again` to theirs; no plan targets `accept` or `1. Yes`; each file and no-tip decision resolves to `31`/`33` when its digit check passed and is absent otherwise. <!-- completed: -->
- [ ] Detector tests: a quoted Dialog A above an idle composer, a model-quoted pending notice and primary-screen remnants produce no interaction; trailing padding keeps each new rule's identity. Tracker tests: replaying the notice fixture then the after-clear fixture closes the notice episode and opens a Dialog A episode; replaying the notice, after-first-Enter and after-second-Enter fixtures keeps the notice episode open, then closes it with the composer idle. <!-- completed: -->
- [ ] Add the rules, `either` screens, `panel_separator` declarations and answers to `bundled.json`. <!-- completed: -->
- [ ] Runner input tests in `src/runtime/input.rs`: replaying the Dialog A fixture queues a preview, a `member prompt` and a `member exec` with zero bytes and the idle composer drains each once; the pending-notice fixture during a submission's settle stops it before the submit key. <!-- completed: -->

### Step 6: Validate and accept

- [ ] Run `mise //:test`, `mise //:lint`, `mise //:typecheck` and `mise //docs:build`. <!-- completed: -->
- [ ] On tmux and herdr with the release build and a private onboarded Claude configuration per member: Dialog A event; a preview, `member prompt` and `member exec` queued and delivered once; Director-auto `--choice decline`; closure and `hasSeenAutoDefaultNudge`. <!-- completed: -->
- [ ] On both multiplexers: Dialog B `--choice not-now` and, on a fresh configuration, `--choice dont-show-again`; `--decision` on a question and `--choice` on a permission request are rejected with zero bytes. <!-- completed: -->
- [ ] On both multiplexers: an onboarding-session member's idle composer and Bash form are detected and answered; each file form and the no-tip Bash form are detected and answered as the digit checks recorded. <!-- completed: -->
- [ ] On herdr (≥ 144 columns) and tmux at the same width: after an allowed Edit of an untracked repository file opens the sidebar, a Bash form is detected and answered. Record versions, geometry, outcomes and every unobserved case in an Execution record section and as limits in `SPEC.md`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-04 | Initial draft |
| 2026-11-04 | Review round 1 and user decision: findings 1, 3 and 5 fixed instead of limits (primary screen `either`, `claude-bash-permission-no-tip`, `panel_separator` clipping); `marker` focus; keyed answer types and `match_answer`; interaction frame guard for Dialog A; pending-question notice rule; 2.1.312 explicit-mode adapter; driver launch policy, stage guard, `{outside}` token, baselines and inline seed; SPEC contradiction sentences named; capture-before-docs rationale; prompt/exec coverage |
| 2026-11-04 | Review round 2: Dialog A after-turn cursor measured inside the region; pending-notice outcomes (clear, two-Enter send) with closure fixtures and tracker tests; typed-input harm in Background; primary-screen Dialog A case |
| 2026-11-04 | Approved by the Reviewer and the user |
