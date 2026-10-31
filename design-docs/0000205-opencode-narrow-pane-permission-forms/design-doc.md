# OpenCode Narrow-Pane Permission Forms and Profile Identifier Removal

**Status**: Approved
**Progress**: 22/22 tasks complete
**Last Updated**: 2026-11-01

## Overview

Detect, extract and answer the OpenCode 2.1.4 `cafleet-ask` shell permission form in narrow panes, where OpenCode puts the choice row and the control line on separate rows and wraps a long command across several rows. Remove the bundled-profile identifier (`profile_version`, `answer_profile_version` and every profile-id string), because `min_version` already identifies each profile, and start a fresh `cafleet_v9.db` format family whose V1 baseline has no profile-identifier columns.

## Success Criteria

- [ ] At every width where OpenCode 2.1.4 `mini` renders the three choices on one row, the `cafleet-ask` shell permission form produces exactly one `interaction_opened` event, whether the control line shares the choice row or sits on its own row.
- [ ] A command that wraps in the form is reported as its displayed rows joined with `\n`, with the continuation margin and the scrollbar column removed, and `operation_rows` equals the displayed row count; the request fingerprint keeps the row breaks.
- [ ] A command clipped by the form's command box, and a form narrower than the lowest supported width, produce no `interaction_opened`; programmatic input stays held and the form is answered manually, as documented limits.
- [ ] The existing allow (Enter) and deny (`h`, Enter, Enter on the empty rejection-reason dialog) plans answer the narrow form, with every stage gate proven on a narrow native pre-stage fixture, including the rejection-reason dialog whose control line splits from its textarea row.
- [ ] Every existing 36x120 OpenCode fixture keeps identical detection, gate and request-identity results.
- [ ] On both tmux and herdr, a 57-column `cafleet-ask` member with a wrapped command is answered by Director-auto allow and deny through `permission respond`, closing the episode `not_visible` with the native result.
- [ ] The default database is `~/.local/share/cafleet/cafleet_v9.db` with `application_id` `1128351289` and one embedded migration, `V1__baseline.sql`; an existing `cafleet_v8.db` stays byte-identical, and an explicitly selected v8 file fails with the incompatible-format error.
- [ ] Runs, events, interaction evidence, inspection, permission responses, composer plans and fingerprints carry no profile identifier, and `permission respond` binds a response to its inspection through the request token, answer rule, backend and native version.
- [ ] Outside `design-docs/`, `git grep -nE 'profile_version|answer_profile_version|claude-2\.1\.302-(auto|default)-[0-9]|codex-0\.163\.0-(approve-for-me|on-request)-[0-9]|opencode-2\.1\.4-cafleet-ask-[0-9]|profile=ANSWER_PROFILE'` finds nothing.
- [ ] `SPEC.md`, the docs, the skill references and the project rules describe the narrow layout, its limits, the identifier-free contracts and the v9 format; `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

### Narrow-pane forms

Design [0000196](../0000196-opencode-ask-permission-forms/design-doc.md) built the bundled OpenCode profile from 36x120 captures only and listed wrapped or narrow-pane forms as out of scope. Its request rule `opencode-shell-permission` requires the `focus` line matcher

```text
^   Allow once   (?:Always allow   )?Reject +⇆ select  enter confirm  esc reject\s*$
```

to match exactly once in the live region. In a live demo the member pane was 57 columns wide, and OpenCode rendered the choices and the control line on separate rows:

```text
   Allow once   Always allow   Reject
  ⇆ select  enter confirm  esc reject
```

The detector's `extract()` (`cafleet/src/coding_agent/profiles/detector.rs`) requires every matcher to match exactly once, so it returned no match and no `interaction_opened` fired. The Director answered with `cafleet member press`. The long command also wrapped (`$ cat /Users/himkt/.config/opencode/skills/cafleet/`, then `roles/member.md`), while the `operation` matcher is line-scoped and reads only the first row.

The Claude wrapped-form work (fixtures `claude-2.1.302-wrapped-*` at 36x52, commit `37721848`) is the prior art. It uses a region-scoped operation matcher, removes a `row_margin` from each displayed row, reports the displayed row count as `operation_rows` and keeps the displayed row breaks in the request fingerprint. OpenCode `mini` lays out its form itself, so its row breaks are hard rows, not terminal soft wraps. The detector's logical lines therefore see each displayed row separately.

### The profile identifier

Design [0000200](../0000200-min-version-runner-profiles/design-doc.md) made `min_version` the profile selection key: catalog loading rejects two profiles of one backend with equal precedence, and a native version selects the profile with the greatest `min_version` at or below it. The `profile_version` string stayed as a recorded label. It is written to `member_runs.profile_version`, the evidence object, semantic and annotation fingerprints, `permission inspect` output, composer plans and the NOT NULL `member_permission_responses.answer_profile_version`, which the permission-response payload digest covers. `permission respond` requires the inspected capability's label to equal the response metadata's.

The label adds no identity that the backend and native version lack within one binary. Editing a profile in place, as this design does for OpenCode, also keeps the label unchanged while the rules change, so the label never distinguished profile contents.

Stored interaction evidence and event payloads carry the label inside the evidence object, and every read validates the allowed key set and recomputes `evidence_digest` (`cafleet/src/broker/runtime/evidence.rs`). A hand-written SQL migration cannot recompute SHA-256, so the existing rows cannot shed the key in place. The user therefore chose a format-family reset, the path `.claude/rules/database-migrations.md` provides for a deliberate break.

---

## Specification

### Scope

| In scope | Out of scope |
|---|---|
| The OpenCode split and compact layouts, at every width where the three choices fit on one row | Layouts where the choice row itself wraps or truncates; they stay manual |
| A command wrapped across several rows of the command box | A command clipped by the command box; it stays manual |
| The narrow rejection-reason dialog | Typed rejection reasons; non-shell `ask` forms |
| Two operation-extract options, `margin_rows` and `row_trailer` (§ Operation extraction) | Changes to the Claude or Codex rules, plans or fixtures |
| Narrow native captures through the existing capture driver, and narrow Director-auto acceptance on tmux and herdr | User-attended acceptance cases; native resize captures |
| Removing every profile identifier from the catalog, runtime records, outputs, documentation and tests | A replacement identifier for the respond-time capability binding |
| The `cafleet_v9.db` format family with a fresh `V1__baseline.sql` | Carrying `cafleet_v8.db` fleets, messages or runtime history into v9 |

The narrow-pane work changes no answer bytes, gates, runner, event, inspection, response or Director-workflow contract; only the rules the stages are gated on widen. The removal changes those contracts only by deleting the identifier fields and the binding check named in § Respond-time binding.

The payload digest versions keep their current meaning in the new family, including the version-1 preview path that only pre-V7 rows used, and readers keep validating schema-version-1 event payloads. Retiring those paths is a separate change.

### Layouts and supported widths

OpenCode 2.1.4 `mini` renders the shell permission form in one of four layouts, by pane width (Step 3 survey, 36 rows):

| Layout | Widths | Choice row | Control line | Detection |
|---|---|---|---|---|
| Wide | 77 and above | `   Allow once   Always allow   Reject` with the control line on the same row | `⇆ select  enter confirm  esc reject`, sharing the choice row | Supported (shipped) |
| Split | 56–76 | The three choices on one row | `  ⇆ select  enter confirm  esc reject` on its own row below | Supported (this design) |
| Compact | 36–55 | ` Allow once   Always allow   Reject`, one-space margin; the heading, `# Shell command` and `$ ` rows start at column 0 | `pgup/pgdn scroll`, sharing the choice row from 53 to 55 and on its own row from 36 to 52 | Supported (this design) |
| Broken | 35 and below | The choices wrap (`Reject` moves to its own row) | Any | Unsupported: no detection, manual answer |

Three breakpoints separate the layouts: `W_split` = 76, the widest split width; `W_compact` = 55, the widest compact width; and `W_min` = 36, the narrowest width that still renders the three choices on one row. The supported range is every width at or above `W_min`. Step 3 records the survey evidence in this document, and Step 4 records the breakpoints in `SPEC.md` and the skill overlay.

The two-option layout (`Allow once` / `Reject`) follows the same rule because the patterns already accept it; its breakpoints are not measured separately.

### Request rule `opencode-shell-permission`

`opencode-shell-permission` matches the wide and split layouts. Its answers, choices, `native_request_id` and sensitivity are unchanged. The compact layout uses its own margins, so it has its own request rule (§ Compact rules).

| Element | Shipped | Wide and split |
|---|---|---|
| Region start | `^  △ Permission required\s*$` | Unchanged |
| Region end | `  ⇆ select  enter confirm  esc reject\s*$`, not anchored at the row start | Unchanged: it matches the control line in either layout |
| `max_rows` | 24 | The largest form height among the supported captures, including the long-command capture when its box grows (§ Clipped commands), and at least 24 |
| `focus` (line, `focus_style: "background"`) | Choices and control line on one row | `^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject)?\s*$`, the choice row in either layout |
| `allow` (line) | `^   (?P<label>Allow once)   ` | Unchanged |
| `deny` (line) | <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject) </code> | <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject)(?: &#124;$)</code>, so a choice row ending at `Reject` matches |
| `controls` (region, new) | — | Either the wide choice-and-control row ends the region, or the split control row ends it with the choice row as the last non-blank row above it; the exact pattern is pinned from the Step 3 captures |
| `operation` | Line-scoped, single row | Region-scoped, multi-row (§ Operation extraction) |
| `unclipped` (new, conditional) | — | Holds only when the command box shows the whole command (§ Clipped commands) |

The `focus` matcher still resolves the focused choice from cell backgrounds. Its labels come from the line-scoped `allow` and `deny` matchers, as catalog validation requires. The split control row carries no label, so it takes no part in focus resolution.

The row background is the background that covers more than half of the choice line's cells; when no background does, no choice resolves and no gate holds. A label is focused when every cell of it differs from the row background. This replaces the shipped left-edge rule, which reads column 0: the compact layout has no left margin, its highlight covers the focused label plus one cell on each side, and at 36 columns a focused `Reject` reaches the last column, so neither edge carries the row background. The rule still compares backgrounds rather than theme colours, and only `focus_style: "background"` matchers, all OpenCode, use it. `SPEC.md` § Focused-choice resolution states it in Step 5.

The focus matcher's full capture is part of the request identity. The layouts capture different text, so a resize across `W_split` or `W_compact` starts a new request generation, as a focus change does today. An earlier request token stops validating, and a response in flight fails before its next byte (`uncertain` once a byte was written). This is the shipped behavior; a synthetic test covers it.

### Compact rules

The compact layout (36–55 columns) drops the form margins, so its patterns cannot share the wide/split rule's margins. Two compact rules match it, pinned from the Step 3 compact captures:

| Rule | Kind | Differences from its wide/split counterpart |
|---|---|---|
| `opencode-shell-permission-compact` | Permission request | The heading, `# Shell command` and `$ ` rows start at column 0, the choice row has a one-space margin, and the region ends at the `pgup/pgdn scroll` control text, which shares the choice row (53–55 columns) or sits on its own row (36–52). Its `operation` extract declares `row_trailer: "█"` and no `row_margin`, because compact continuation rows start at column 0; the both-ends anchoring of § Operation extraction applies, with "starts with a non-space character" in place of "starts with the continuation margin and then a non-space character". |
| `opencode-reject-reason-compact` | Permission continuation | The rejection-reason dialog in its compact margins and control text; `continues_rule_id` `opencode-shell-permission-compact`, `continues_choice_id` `deny-once`, and the same `empty_reason` meaning |

The compact request rule has its own `allow` and `deny` answer entries, with the bytes and gates of § Answer stages and the compact pre-stage fixtures. Its choices, `native_request_id`, focus resolution and sensitivity are those of `opencode-shell-permission`. The detection capability lists both request rules. At any width exactly one of the two request rules matches, because their heading margins differ.

### Operation extraction

The operation is read from every displayed row of the command, not only the first. Two optional keys extend `extract.operation`, and the OpenCode request rule declares both:

| Key | Values | Behavior |
|---|---|---|
| `margin_rows` | `all` (default) or `continuation` | Which captured rows carry `row_margin`. `all` keeps the shipped behavior: every row must start with the margin, which is removed. `continuation` exempts the first row, whose prefix the matcher already consumed. Declaring it without `row_margin` fails catalog loading. |
| `row_trailer` | A non-empty single-row literal without leading or trailing whitespace | A captured row that ends with the trailer, ignoring trailing whitespace, loses the trailer and the whitespace before it. Every other row is left as it is at this step. |

Extraction applies, per row: remove the trailer, then remove the margin from the rows `margin_rows` selects, then remove trailing whitespace. A selected row without the margin fails the match, as today. This keeps the shipped margin-then-trim order, so a whitespace-only margined row, such as a blank line inside a multi-line Claude command, still matches. The Claude and Codex rules declare neither key, so their extraction is unchanged.

The OpenCode `operation` matcher becomes region-scoped. It captures from after `  $ ` on the first command row through each following row that starts with the continuation margin and then a non-space character. Its extract declares `row_margin` equal to the continuation margin the Step 3 captures show, `margin_rows: "continuation"` and `row_trailer: "█"`, the glyph the 120-column capture shows in a column beside the command box. A one-row command extracts the same text as today, with `operation_rows` 1.

The capture is anchored at both ends, so it cannot drop command rows. After the last captured row, only rows that are blank or hold only the trailer may follow until the choice row. Any other row in between fails the match, so the request is not observed. Examples are a continuation row whose text begins with a space after the margin, such as an indented second line of a multi-line command, or a row without the margin. Such a form is answered manually (§ Limits).

The `operation` evidence reports the displayed rows joined with `\n`, and `operation_rows` equals the displayed row count, so the Director's existing multi-row instruction applies. The request's semantic fingerprint keeps those row breaks, as for Claude and Codex. A resize that moves a row break therefore closes the episode and opens a new one with a fresh `interaction_opened`.

### Clipped commands

A command taller than the form's command box may scroll inside the box and hide rows. The Director must not approve a command it cannot read in full, so a clipped command matches no request. The Step 3 long-command capture decides how the rule observes this:

| Long-command capture shows | Rule requirement |
|---|---|
| The box grows to fit the command, with no hidden rows | No `unclipped` matcher. The long-command capture is then a supported capture, so `max_rows` covers its form height. A taller form, beyond `max_rows` or the pane, matches no request (§ Limits). |
| A scrollbar thumb that differs textually from its track | Region matcher `unclipped`: the thumb glyph `█` spans every row of the box's track |
| Hidden rows distinguishable only by cell background | Step 3 blocker: the text matchers cannot observe it |

A clipped form keeps input held, as any unmatched interaction does, and is answered manually on the native surface.

### Continuation rule `opencode-reject-reason`

In the split layout, the rejection-reason dialog's control line `enter confirm  esc cancel` moves to its own row. The empty textarea row then reads `  Tell OpenCode what to do differently`, the same text as the prompt row above it. The line-scoped `prompt` matcher would match twice, so the rule would fail.

| Element | Shipped | Both layouts |
|---|---|---|
| Region | `△ Reject permission` to `enter confirm  esc cancel\s*$` within 24 rows | Unchanged; the compact dialog has its own rule (§ Compact rules) |
| `prompt` and `empty` (line) | `prompt` matches the prompt row; `empty` matches the placeholder sharing the control-line row | Replaced by one region matcher `empty_reason`. It requires the prompt row, then the placeholder either sharing the control-line row (wide) or on the last non-blank row above a control-only row (split). The exact pattern is pinned from the Step 3 captures. |

The empty-reason meaning is unchanged. The textarea must show exactly the placeholder; typed text replaces it and fails the stage-3 gate, and a typed copy of the placeholder remains a documented limit.

### Answer stages

The 0000196 bytes, gates and native effects apply in both layouts:

| Answer | Stage | Bytes | Gate |
|---|---|---|---|
| Allow | 1 | `0d` | Focus `allow-once` |
| Deny | 1 | `68` | Focus `allow-once` |
| Deny | 2 | `0d` | Focus `deny-once` |
| Deny | 3 | `0d` | Screen `opencode-reject-reason` with an empty reason |

Each answer entry's `fixture_ids` gains the narrow pre-stage fixtures (§ Fixtures). Step 3 stops at a blocker when a narrow measurement contradicts the table. The blocker is a Programmer blocker marker at `paragraph-Implementation > Step 3`, paired with a `blocked (paragraph-Implementation > Step 3)` message to the Director. Each of these measurements is a blocker:

- the focused choice cannot be resolved from cell backgrounds in the split or compact layout;
- `h` from `Allow once` focuses an option other than `Reject`;
- Enter on `Reject` shows anything other than the empty rejection-reason dialog;
- Enter in the empty dialog does not reject the request without text.

### Native capture procedure

Captures use the existing driver and its approved-case registry in `cafleet/src/runtime/runner/observation.rs`, with the 0000196 OpenCode procedure:

- a private `OPENCODE_CONFIG_DIR` holding `cafleet-ask.md`;
- the OpenCode prompt branch;
- `prefix_hex` stages sent before arming;
- a fresh pane capture confirming each stage's gate before its byte.

Every case uses backend `opencode`, permission mode `cafleet-ask`, 36 rows, database `<case id>.db` and socket `<case id>` in the driver's private root. The case cwd is `/tmp/cafleet-194/<case id>`, so `<cwd>/permission-probe <argument>` exceeds 57 columns and wraps.

The survey cases bracket the breakpoints. Each is `no_answer` with probe argument `allow`, and survey captures are evidence only, with no fixtures:

| Case ID | Columns |
|---|---|
| `opencode-survey-096` | 96 |
| `opencode-survey-080` | 80 |
| `opencode-survey-064` | 64 |
| `opencode-survey-048` | 48 |
| `opencode-survey-040` | 40 |

Each breakpoint needs an observed layout on both sides of it. The Programmer adds cases as reviewed registry edits, each with the same shape and an ID of the form `opencode-survey-<columns, three digits>`:

1. **Upper edge of `W_split`.** The shipped 120-column capture (`opencode-cells-001`) is the wide upper bound. If 96 is already split, add cases between 96 and 120 until the widest split width and the narrowest wide width are adjacent column counts.
2. **Lower edge of `W_min`.** If 40 still renders the choices on one row, add narrower cases (32, then 4 columns narrower each time) until a broken layout is observed.
3. **Exact breakpoints.** Bisect between each bracketing pair until `W_split` (the widest split width) and `W_min` (the narrowest one-row-choice width) are exact column counts.

The fixture cases run at 57 columns (split, the demo width), at `W_compact` = 55 (compact with a shared control row) and at `W_min` = 36 (compact with a control-only row):

| Case ID | Columns | Driver mode | Decision | Prefix bytes | Stage bytes | Probe argument | Supplies |
|---|---|---|---|---|---|---|---|
| `opencode-narrow-cells-057` | 57 | `no_answer` | — | — | — | `allow` | The split form with a wrapped command; the allow and deny stage-1 pre-stage |
| `opencode-narrow-allow-057` | 57 | `answer_stage` | `allow` | — | `0d` | `allow` | Narrow allow decision evidence |
| `opencode-narrow-deny-002-057` | 57 | `answer_stage` | `deny` | `68` | `0d`, then `0d` | `deny` | The deny stage-2 pre-stage (`Reject` focused) and narrow deny decision evidence |
| `opencode-narrow-deny-003-057` | 57 | `answer_stage` | `deny` | `68`, then `0d` | `0d` | `deny` | The deny stage-3 pre-stage: the split empty rejection-reason dialog |
| `opencode-narrow-long-057` | 57 | `no_answer` | — | — | — | `allow`, one space and 400 `x` characters | The long-command screen for § Clipped commands |
| `opencode-narrow-idle-057` | 57 | `no_answer` | — | — | — | — | The idle composer at 57 columns, the screen after an answer |
| `opencode-compact-cells-055` | 55 | `no_answer` | — | — | — | `allow` | The compact form with its control text sharing the choice row; the allow and deny stage-1 pre-stage |
| `opencode-compact-deny-002-055` | 55 | `answer_stage` | `deny` | `68` | `0d`, then `0d` | `deny` | The deny stage-2 pre-stage at 55 |
| `opencode-compact-deny-003-055` | 55 | `answer_stage` | `deny` | `68`, then `0d` | `0d` | `deny` | The deny stage-3 pre-stage: the compact rejection-reason dialog at 55 |
| `opencode-narrow-cells-min` | `W_min` | `no_answer` | — | — | — | `allow` | The form at the lowest supported width |
| `opencode-narrow-deny-002-min` | `W_min` | `answer_stage` | `deny` | `68` | `0d`, then `0d` | `deny` | The deny stage-2 pre-stage at `W_min` |
| `opencode-narrow-deny-003-min` | `W_min` | `answer_stage` | `deny` | `68`, then `0d` | `0d` | `deny` | The deny stage-3 pre-stage at `W_min` |

The long-command case never answers, so the helper never runs and its argument needs no meaning. Step 3 reviews masking as in 0000196 and confirms every rule region lies in unmasked rows. Each fixture records `cell_backgrounds` so focus resolves on replay. Step 3 records the CAFleet commit, OpenCode version, model, geometry, breakpoints and masking review.

### Fixtures

| Fixture ID | From case | Cited by |
|---|---|---|
| `opencode-2.1.4-narrow-permission-057-1` | `opencode-narrow-cells-057` | Detection; allow stage 1; deny stage 1 |
| `opencode-2.1.4-narrow-deny-pre-stage-2-057-1` | `opencode-narrow-deny-002-057` | Deny stage 2 |
| `opencode-2.1.4-narrow-deny-pre-stage-3-057-1` | `opencode-narrow-deny-003-057` | Deny stage 3; `opencode-reject-reason` |
| `opencode-2.1.4-narrow-long-command-057-1` | `opencode-narrow-long-057` | Clipped-command tests: no match when clipped, or full extraction when the box grows |
| `opencode-2.1.4-narrow-idle-composer-057-1` | `opencode-narrow-idle-057` | Empty-composer observation at 57 columns |
| `opencode-2.1.4-compact-permission-055-1` | `opencode-compact-cells-055` | `opencode-shell-permission-compact` detection; its allow stage 1 and deny stage 1 |
| `opencode-2.1.4-compact-deny-pre-stage-2-055-1` | `opencode-compact-deny-002-055` | `opencode-shell-permission-compact` deny stage 2 |
| `opencode-2.1.4-compact-deny-pre-stage-3-055-1` | `opencode-compact-deny-003-055` | `opencode-shell-permission-compact` deny stage 3; `opencode-reject-reason-compact` |
| `opencode-2.1.4-narrow-permission-min-1` | `opencode-narrow-cells-min` | `opencode-shell-permission-compact` detection; its allow stage 1 and deny stage 1 |
| `opencode-2.1.4-narrow-deny-pre-stage-2-min-1` | `opencode-narrow-deny-002-min` | `opencode-shell-permission-compact` deny stage 2 |
| `opencode-2.1.4-narrow-deny-pre-stage-3-min-1` | `opencode-narrow-deny-003-min` | `opencode-shell-permission-compact` deny stage 3; `opencode-reject-reason-compact` |

The 0000196 fixtures stay, with their citations. Fixture IDs name captures, not profiles, so they keep their native-version prefix.

### Captured evidence

Step 3 ran on 2026-11-01 on macOS with OpenCode 2.1.4 `mini` (`opencode v2.1.4`), the `cafleet-ask` preset from a private `OPENCODE_CONFIG_DIR`, a dedicated tmux 3.6a server per case, 36 rows and the primary screen. The capture libtest was built from commit `4e6cae41` exported into an isolated tree, plus the working-tree driver changes: the approved-case registry entries, a macOS private root `/private/tmp/cafleet-194-p` (the runner launch passes the canonical database path, which the `/tmp` spelling fails), and a requested marker taken from the probe argument's first word. The survey cases use short database and socket names (`osNNN`); every fixture case uses its case ID.

The private configuration selected the model. Every survey case and eight fixture cases ran on `opencode/big-pickle`. After that free model reached its usage limit, `opencode-narrow-cells-min`, `opencode-compact-deny-003-055` and `opencode-narrow-deny-003-min` ran on the user-selected `opencode-go/glm-5.3`.

| Measurement | Observed |
|---|---|
| Layouts | Wide at 77 columns and above, split 56–76, compact 36–55 (`pgup/pgdn scroll` shares the choice row at 53–55), broken at 35 and below (`Reject` wraps) |
| Focus | The focused label sits on rgb(250,178,131) and the other labels on rgb(30,30,30) in the split and compact layouts |
| Deny stages | `h` from `Allow once` focused `Reject`, Enter on `Reject` opened the empty rejection-reason dialog, and Enter there printed `Step interrupted` with no marker, at 57, 55 and 36 columns. No blocker in § Answer stages was observed. |
| Allow stage | Enter on `Allow once` ran the command once at 57 columns (`allow.executed` present) |
| Long command | At 57 columns the command box keeps its height and clips the command; the track shows `▀` on its first row and is blank below. § Clipped commands therefore uses the `unclipped` matcher. |
| Compact rejection dialog | `△ Reject permission` at column 0, no prompt row, the placeholder `Tell OpenCode what to do differently` at column 0 and the control text `enter reject esc back` at column 0 on the next row |
| Continuation margin | Two spaces in the wide and split layouts; none in the compact layout |
| Region height | At most 14 rows (compact), so `max_rows` stays 24 for every rule |

Masking: every capture masked one reviewed row. The 0000196 row 35 is masked in every fixture except `opencode-2.1.4-compact-deny-pre-stage-3-055-1`, `opencode-2.1.4-narrow-permission-min-1` and `opencode-2.1.4-narrow-deny-pre-stage-3-min-1`, whose compact control text sits on row 35; those mask row 0, a transcript row. No fixture shows an account, home path or plan text. The idle fixture shows the model name and a context-usage counter, as the 0000196 idle fixture does, and the footer that names the workspace appears only after an answer, in no fixture.

The patterns below were checked with an emulation of the detector's region and matcher semantics against every OpenCode fixture and every survey capture. Every 120-column fixture keeps its operation, every width from 36 to 96 matches exactly one request rule, widths of 35 and below match none, and the long-command fixture fails only `unclipped`.

| Rule | Element | Pattern |
|---|---|---|
| `opencode-shell-permission` | `operation` (region) | `(?m)^  \$ (?P<operation>\S[^\n]*(?:\n  \S[^\n]*)*)(?:\n *(?:█ *)?)*\n   Allow once   ` with `row_margin` two spaces, `margin_rows: "continuation"`, `row_trailer: "█"` |
| `opencode-shell-permission` | `unclipped` (region) | `(?m)^   # Shell command +█ *\n(?: *█ *\n)*  \$ [^\n]*█ *(?:\n  \S[^\n]*█ *)*(?:\n *(?:█ *)?)*\n   Allow once   ` |
| `opencode-shell-permission` | `controls` (region) | `(?m)^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject *&#124; *(?:\n *)*\n  ⇆ select  enter confirm  esc reject *)\z` |
| `opencode-shell-permission-compact` | Region | Start `^△ Permission required\s*$`, end `pgup/pgdn scroll\s*$`, 24 rows |
| `opencode-shell-permission-compact` | `heading`, `shell` (line) | `^△ Permission required\s*$`, `^# Shell command(?:\s&#124;$)` |
| `opencode-shell-permission-compact` | `operation` (region) | `(?m)^\$ (?P<operation>\S[^\n]*(?:\n\S[^\n]*)*)(?:\n *(?:█ *)?)*\n Allow once   ` with `row_trailer: "█"` and no `row_margin` |
| `opencode-shell-permission-compact` | `unclipped` (region) | `(?m)^# Shell command +█ *\n(?: *█ *\n)*\$ [^\n]*█ *(?:\n\S[^\n]*█ *)*(?:\n *(?:█ *)?)*\n Allow once   ` |
| `opencode-shell-permission-compact` | `controls` (region) | `(?m)^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll *&#124; *(?:\n *)*\npgup/pgdn scroll *)\z` |
| `opencode-shell-permission-compact` | `focus`, `allow`, `deny` (line) | `^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll)?\s*$` with `focus_style: "background"`, `^ (?P<label>Allow once)   `, `(?:Allow once&#124;Always allow)   (?P<label>Reject)(?: &#124;$)` |
| `opencode-reject-reason` | `empty_reason` (region) | `(?m)^  △ Reject permission *(?:\n *)*\n  Tell OpenCode what to do differently *(?:\n *)*\n  Tell OpenCode what to do differently(?: {2,}enter confirm  esc cancel *&#124; *(?:\n *)*\n  enter confirm  esc cancel *)\z` |
| `opencode-reject-reason-compact` | Region | Start `^△ Reject permission\s*$`, end `^enter reject esc back\s*$`, 24 rows |
| `opencode-reject-reason-compact` | `heading`, `empty_reason` | Line `^△ Reject permission\s*$`; region `(?m)^△ Reject permission *(?:\n *)*\nTell OpenCode what to do differently *(?:\n *)*\nenter reject esc back *\z` |

In the table, `&#124;` stands for the regex alternation bar. The wide/split `focus` and `deny` patterns are those of § Request rule `opencode-shell-permission`.

| Fixture ID | SHA-256 |
|---|---|
| `opencode-2.1.4-narrow-permission-057-1` | `a7085d984d8fca560c0996996de144b654c3766a06bbb84e387b3effe6a34df9` |
| `opencode-2.1.4-narrow-deny-pre-stage-2-057-1` | `fb731b1eebaaab736018be64b5bc1099f1803db87876697473933cf16b0487a2` |
| `opencode-2.1.4-narrow-deny-pre-stage-3-057-1` | `bd3c00ec1cdd930d960b28bfd85d3ab53826bf861a8a831ecbe051f7618f08f6` |
| `opencode-2.1.4-narrow-long-command-057-1` | `1ba505df087beffb1c6351a86401d7ef467c8fe963a232478d0782448c973cab` |
| `opencode-2.1.4-narrow-idle-composer-057-1` | `20735e881d0befc46ffffa66429f2ac7e5dd6263845c1117b6aed9dde2978e1f` |
| `opencode-2.1.4-compact-permission-055-1` | `9150de0d5fee4b94824ba6cb45dd14b7fdb3bf48fdb07306781d2a98b2662fc6` |
| `opencode-2.1.4-compact-deny-pre-stage-2-055-1` | `323483fa5f965c096d50f03fcfef0115bfd3b292b99637d06353a33378dd1a01` |
| `opencode-2.1.4-compact-deny-pre-stage-3-055-1` | `27a41af0b1b6eee76cc987e65175388164e5e81f97fbbaa53031ef29b845ca8d` |
| `opencode-2.1.4-narrow-permission-min-1` | `41c5d247d69ffd2d37779563941bf1b3f0b405cd193a331921fada45dd21d551` |
| `opencode-2.1.4-narrow-deny-pre-stage-2-min-1` | `cc366ac7736137e623e5e629ec9a8acdac0b680da0c69eb7e230cbefd6fb82ce` |
| `opencode-2.1.4-narrow-deny-pre-stage-3-min-1` | `9717287c8a06acfdb0455c6ee3899e679731af28fd5928798271e4c25463b85b` |

### Profile identity after removal

A bundled profile entry has required `backend`, `min_version`, `rules` and `capabilities`, and nothing else. The strict loader rejects any other key, `profile_version` included, with its existing unknown-key error. Documentation and messages refer to a profile by backend and boundary, written as "the OpenCode profile from `2.1.4`". The OpenCode profile is edited in place; its boundary `2.1.4` is unchanged.

| Surface | Change |
|---|---|
| Catalog and selection (`catalog.rs`, `native_version.rs`, `profiles.rs`) | The profile struct, the selection result and the detection, composer and answer capabilities lose the identifier; selection reports the selected `min_version` where tests need to name the selected profile |
| Detector (`detector.rs`) | Raw evidence loses `profile_version`. Semantic parts and `AnnotationFingerprint` lose the profile element; within one detector it was constant. |
| Composer plans (`composer.rs`) | A plan retains the run's native version and its cited fixture identities |
| Run creation and replacement (`cli/member.rs`, `runtime/runner/creation.rs`, `deletion.rs`) | No identifier is recorded or copied; `coverage` and `native_version` stay |
| Runner wire and client (`input_protocol.rs`, `input_client.rs`, `input.rs`, `observation.rs`, `service.rs`, `writer.rs`, `recovery/replay.rs`, `runner/mod.rs`) | Inspection replies and response requests lose both fields and their cross-checks |
| Broker records and validation (`broker/runtime/*.rs`) | Run rows, event validation, evidence keys, inspection association and the response audit lose the fields |
| Presentation (`presentation.rs`, `output/runtime.rs`) | JSON objects lose the keys, and the inspection text line loses `profile=` |

Three runtime invariants compare the identifier today. Each keeps an identifier-free invariant:

| Invariant | Today | After removal |
|---|---|---|
| Observer initialization (`observation.rs`, `observer profile/coverage mismatch`) | The catalog's selection for the run's backend and native version has the run's `profile_version` and its detection coverage equals the run's `coverage` | The selection's detection coverage equals the run's `coverage`. A `tested` coverage therefore still requires a selected profile, and a probe failure (null native version) still requires `unknown`. |
| Inspection reply (`input_client.rs`) | The reply's and its evidence's `profile_version` equal the run's; a `tested` reply's `answer_profile_version` equals the run's `profile_version` | The reply's and its evidence's backend and native version equal the run's. A `tested` reply requires a non-null run `native_version`, a token, a non-empty decision set, a visible permission request and no capability reason. Its `answer_rule_id` stays nullable, as shipped: the runner reports it only when every supported decision shares one rule, and a present value is non-blank and at most 256 bytes. |
| Capture-run identity (`same_capture_run`, `observation.rs`) | Run fields including `profile_version` are equal | The same fields without `profile_version`; backend and native version already determine the selection |

### Respond-time binding

`permission respond` no longer compares a profile identifier. It still requires, before the first byte:

- the capability's backend and native version to equal the run's;
- the capability's answer rule to equal the response metadata's `answer_rule_id`;
- the capability's request rule to equal the event evidence `rule_id`;
- the capability's decision to equal the requested decision, with a non-empty fixture list;
- the request token, expected owner epoch, event/message association and the remaining shipped checks.

The runner is one process from one binary and selects its profile deterministically from the backend and native version. The request token already binds a response to the inspection that minted it, so no replacement identifier is recorded.

### Output and record contracts

| Contract | New shape |
|---|---|
| Evidence object (events and stored interaction evidence) | Ordered keys `backend,native_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`; `evidence_digest` covers this shape |
| `permission inspect` JSON `{request}` | Ordered keys `event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence` |
| `permission inspect` text | `Request: token=TOKEN output_seq=SEQUENCE rule=ANSWER_RULE` |
| `input show` / `input reconcile` `permission_response` | Ordered keys `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id,decided_at` |
| Permission-response payload digest (version 1) | Covers `run_id,owner_epoch,interaction_id,event_id,director_member_id,request_token,decision,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id` |
| Metadata evidence policy | Persists identities, classification, rule and native version, sequence, timestamps and health/closure causes |
| Permission-request semantic fingerprint | Backend, interaction class, visible operation with its row breaks and stable choice labels |
| Annotation fingerprint | Backend, rule id and the normalized displayed operation or notice text |

Event and evidence schema version numbers stay as they are. The v9 family starts empty, so no record of the earlier shape coexists with the new one.

### Database format `cafleet_v9.db`

| Element | Value |
|---|---|
| Default URL | `sqlite:///` + `~/.local/share/cafleet/cafleet_v9.db` |
| `PRAGMA application_id` | `1128351289` (`0x43414639`, ASCII `CAF9`) |
| Embedded migrations | `cafleet/migrations/V1__baseline.sql` only; the in-tree V1 through V8 files of the v8 family are deleted (git keeps them) |
| Baseline content | The current head schema (V1 through V8 applied) with `member_runs.profile_version` and `member_permission_responses.answer_profile_version` omitted, the new application identity, and every other table, column, CHECK, index, foreign key and the singleton instance-identity initialization unchanged |
| Predecessor file | `~/.local/share/cafleet/cafleet_v8.db` and its sidecars are never opened, migrated, renamed or deleted |

The shipped format classification applies unchanged:

| Selected file | Behavior |
|---|---|
| Empty SQLite file, or the absent default path | `cafleet setup` applies the baseline transactionally, then installs assets |
| v9 identity and the V1 shape | Normal commands and idempotent setup proceed |
| v9 identity with a ledger ahead of V1 | `database schema M is newer than this cafleet (head 1); upgrade cafleet` |
| Any other identified or populated file, including a v8 file selected through `CAFLEET_DATABASE_URL` | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.`, with file bytes preserved |

After upgrading, a user runs `cafleet setup` once. It creates the v9 database and records the asset installations there, and fleets are created anew. `cafleet doctor` reports the v9 file as absent until setup runs.

The chain guard in `cafleet/src/db/mod.rs` derives the registry from the embedded migrations, so it now asserts a single baseline at head 1. A baseline test asserts the fresh head shape, including the absence of both identifier columns. The behind-state classification and its `database schema is outdated` error stay for future successors; at head 1 no earlier version exists to exercise them, and the first V2 adds that coverage through its successor test.

Every test that depends on an intermediate version or the v8 identity changes:

| Test | Change |
|---|---|
| The V2 through V8 successor tests in `db/mod.rs`, including the `head_version() >= 5/6/7/8` modules and their `migrate_to_version(…, 6)` and `schema_matches_version(…, 6/7/8)` assertions | Deleted with their migrations |
| `runtime_migration_extends_the_existing_family_after_v3` (`db/mod.rs`) | Deleted; the baseline-shape test asserts the runtime tables and `APPLICATION_ID` |
| `supported_upgrades_and_repeated_migration_preserve_all_application_rows` (`db/mod.rs`) | Rewritten against the baseline: repeated migration at head 1 preserves every application row; the `version == 2` branch and the behind-version assertions go |
| `legacy_opening` and its callers (`broker/runtime/events.rs`) | Rewritten to insert the schema-version-1 event payload directly into the v9 baseline, without `migrate_to_version(…, 4)` or the upgrade-preservation assertion |
| The `Unreachable` classification test (`diagnosis.rs`) | Uses `crate::db::APPLICATION_ID` instead of the literal `1128351288` |

The schema-version-1 event read path stays, as the version-1 preview digest path does (§ Scope): readers keep validating v1 and v2 event payloads, and `legacy_opening` keeps covering v1.

### Tests

Every test is an in-file unit test of the file that owns the asserted behavior (`.claude/rules/code-quality.md` § Test placement):

| Behavior | Owning file |
|---|---|
| `margin_rows` and `row_trailer` parsing and validation: an unknown value, `margin_rows` without `row_margin`, an empty or whitespace-bearing trailer | `cafleet/src/coding_agent/profiles/catalog.rs` |
| Extraction order, the first-row exemption, a continuation row without its margin failing the match, a row without the trailer kept unchanged, and unchanged Claude and Codex extraction | `cafleet/src/coding_agent/profiles/detector.rs` |
| Detection on every narrow fixture (operation rows, `operation_rows` at least 2, both choices, resolved focus) and every gate holding on its narrow pre-stage fixture | `cafleet/src/coding_agent/profiles/catalog.rs` |
| Every existing 36x120 OpenCode fixture keeping its operation, choices, focus, fingerprint and gate results | `cafleet/src/coding_agent/profiles/catalog.rs` |
| Synthetic screens yielding no request or no gate: a broken layout, a clipped command, a continuation row beginning with a space after the margin or lacking the margin before the box end, form text outside a live region, a control-only row without a choice row above it, a prompt-only rejection dialog, a typed reason in the split dialog | `cafleet/src/coding_agent/profiles/detector.rs` |
| A blank margined row inside a multi-line Claude command still matching under the shipped extraction order | `cafleet/src/coding_agent/profiles/detector.rs` |
| The focus capture and the semantic fingerprint differing between the wide and split layouts of one request, and across a rewrap that moves a row break | `cafleet/src/coding_agent/profiles/detector.rs` |
| A resize across `W_split` starting a new request generation, and a rewrap that moves a row break opening a new episode | `cafleet/src/runtime/runner/observation.rs` |
| A resize across `W_split` between deny stages failing the response before its next byte (`uncertain`) | `cafleet/src/runtime/runner/input.rs` |
| The identifier-free observer-initialization and capture-run invariants | `cafleet/src/runtime/runner/observation.rs` |
| The identifier-free inspection-reply invariant | `cafleet/src/runtime/runner/input_client.rs` |
| The survey and fixture cases in the approved-case registry | `cafleet/src/runtime/runner/observation.rs` |
| The v9 default URL | `cafleet/src/config.rs` |
| The baseline at head 1, its application identity and shape, and v8-file rejection with preserved bytes | `cafleet/src/db/mod.rs` |
| The new evidence, inspection, `permission_response` and digest shapes | The files that own each shape (`evidence.rs`, `presentation.rs`, `output/runtime.rs`, `permissions.rs`, `input_protocol.rs`) |

Existing tests that set or assert a profile identifier are rewritten to the identifier-free shape; no test asserts that the removed key is rejected.

### Acceptance

The executing fleet's installed CLI and its `cafleet_v8.db` stay untouched during implementation. Acceptance runs the shipping release build by its build path, with an explicit private `CAFLEET_DATABASE_URL`, OpenCode 2.1.4 and `--permission-mode cafleet-ask`. The member pane is 57 columns wide, and the case folder is long enough that the probe command wraps.

| Case | Multiplexers | Required evidence |
|---|---|---|
| Director-auto allow | tmux and herdr | `interaction_opened` with `operation_rows` at least 2, no profile identifier in its evidence, and the review instruction; `accepted`/`submitted`; the allow marker exists; `interaction_closed` `not_visible`; the member continues |
| Director-auto deny | tmux and herdr | The same opening; the three-stage deny `accepted`/`submitted`; no marker; `Step interrupted` and the idle composer; `interaction_closed` `not_visible` |

Each case sizes the member pane before the probe runs:

1. Create the member with a spawn prompt that tells it to wait for the Director's next message before running `<case folder>/permission-probe <allow|deny>`. On herdr, size the member column first (step 2) so the member starts at its final width.
2. Size the member pane to a 57-column PTY. On tmux, run `tmux resize-pane -t <member pane> -x 57` after creation. On herdr, before creation, run `herdr pane resize --pane <director pane> --direction right --amount <delta>` with the signed ratio delta computed from the tab width so that the new member column's `rect.width` is 59; herdr's `rect.width` includes the two border cells.
3. Confirm the width: tmux `tmux display-message -p -t <member pane> '#{pane_width}'` prints `57`; on herdr, the member's `rect.width` in a fresh `herdr pane layout` read is 59. Repeat step 2 until it holds.
4. Send the Director message that starts the probe, then follow the case.

Record the confirmed width output, the CAFleet commit, versions, latency and every unobserved case; a missing case fails acceptance.

#### Acceptance evidence (2026-11-01, macOS)

Release build of `08710230` (`mise //cafleet:build`, run as `cafleet/target/release/cafleet`) against the private database `/private/tmp/c205/a.db` (v9 baseline, head 1); OpenCode 2.1.4 (`opencode v2.1.4`, mini) with model `opencode-go/glm-5.3`, `--permission-mode cafleet-ask`, presets from a private `OPENCODE_CONFIG_DIR`; tmux 3.6a on a dedicated server (`-L c205`, 230x50) and herdr 0.9.5 in a dedicated tab. Members ran with `--event-evidence redacted` so that `operation` and `operation_rows` are reported; the first tmux allow run used the default `metadata` policy, which reports both as null. The case folders sit under the task folder, so the probe command wraps across three rows of the 57-column command box.

| Case | Multiplexer | Width confirmation | Observed outcome |
|---|---|---|---|
| Director-auto allow | tmux | `#{pane_width}` printed `57` | `interaction_opened` rule `opencode-shell-permission` (split layout), `operation_rows` 3, operation `…/design-docs/\n0000205-opencode-narrow-pane-permission-forms/.\nverifier/cases/tmux-allow-2/permission-probe allow`, choices `Allow once`/`Reject`, the multi-row review instruction, evidence keys without a profile identifier; `permission respond` `accepted`/`submitted`; `allow.executed` created; `interaction_closed` `not_visible` (rule `opencode-idle-composer`); the member reported the result |
| Director-auto deny | tmux | `#{pane_width}` printed `57` | The same opening with `operation_rows` 3; three-stage deny `accepted`/`submitted`; no marker; `Step interrupted` and the idle composer; `interaction_closed` `not_visible` |
| Director-auto allow | herdr | Member `rect.width` 59 in a fresh `herdr pane layout` (57-column PTY) | The same split-layout opening with `operation_rows` 3; `accepted`/`submitted`; `allow.executed` created; `interaction_closed` `not_visible`; the member reported the result |
| Director-auto deny | herdr | Member `rect.width` 59 (57-column PTY) | The same opening; three-stage deny `accepted`/`submitted`; no marker; `Step interrupted` and the idle composer; `interaction_closed` `not_visible` |

Detection-to-inbox latency (event `observed_at` to inbox `created_at`) was 14–26 ms. `permission inspect` reported `answer_capability` `tested`, `supported_decisions` `allow,deny` and `answer_rule_id` null (allow and deny use different answer rules); each response audit recorded its actual rule (`opencode-shell-allow-once` for allow). Deleted members produced `runner_exited` (native exit 130).

Observed outside the required cases:

- herdr, member pane resized after spawn (the step-2 procedure): before the resize the composer was clean; immediately after `herdr pane resize`, OpenCode printed `[Keymap] Error in event match resolver … Invalid key name: key name cannot be empty` with a colour-reply fragment (`4;76;rgb:5f5f/d7d7/0000…`) in its composer, the form region filled with the error text and no `interaction_opened` fired. Reproduced twice; tmux resizes were unaffected. The herdr cases above therefore sized the column before spawn, so the member started at its final width.
- herdr `rect.width` includes the pane border: `rect.width` 57 gave a 55-column PTY, which renders the compact layout.
- herdr, 57-column PTY with a command needing four rows (`…/cases/herdr-allow-2/permission-probe allow`): the box showed three rows and a `▀` thumb with `allow` hidden; no `interaction_opened` fired within 180 s and input stayed held, matching § Clipped commands.
- `opencode-go/glm-5.3` sometimes started the probe in its first turn, before the Director's start message; the form was still detected and answered through `permission respond`.
- A setup with a private `OPENCODE_CONFIG_DIR` installs presets there but writes the skills to the fixed `~/.config/opencode/skills`.

### Limits

| Limit | Behavior |
|---|---|
| A pane narrower than `W_min` | The choices break, no request matches, input stays held and the form is answered manually |
| A command clipped by the command box | No request matches; the form is answered manually |
| A form taller than `max_rows` or the pane | No request matches; the form is answered manually |
| A command row that begins with whitespace after the continuation margin, such as an indented line of a multi-line command | The anchored capture fails, no request matches, and the form is answered manually |
| A resize while the form is open | A new request generation, and a new episode when the command's row breaks move; an earlier token stops validating |
| Resizing a running OpenCode member pane on herdr | OpenCode can receive a fragment of a terminal colour reply as keys (`Invalid key name`), after which no request matches; size the herdr pane before creating the member. The runner's handling of that reply is a separate change. |
| Upgrading from `cafleet_v8.db` | Fleets, messages and runtime history stay in the v8 file and are not shown by the new CLI |

### Documentation targets

| Target | Change |
|---|---|
| `SPEC.md` §6.9 OpenCode profile | Both layouts, `W_split` and `W_min`, the widened `focus`, `deny`, `controls`, `operation` and `unclipped` matchers, the widened `opencode-reject-reason`, the narrow fixtures, and limits replacing "A command that wraps in the form yields no detection" |
| `SPEC.md` operation-extract contract | `margin_rows` and `row_trailer`, their validation and the extraction order |
| `SPEC.md` §6.7 OpenCode prose | The narrow-width and clipped-command limits replace "A command that wraps in the form … answered manually" |
| `SPEC.md` profile invariants | The selection table's `profile_version` and `coverage` column becomes `coverage` only: "Its identifier; `tested` …" becomes "`tested` when the selected profile declares detection, otherwise `unknown`", and the `null; unknown` cells become `unknown`. "Fresh creation and observer initialization verify that deterministic selection agrees with the recorded profile and coverage" becomes "… that deterministic selection from the recorded backend and native version yields the recorded coverage". "Recovery reports an unavailable or mismatched historical profile" becomes "recovery reports a recorded `tested` coverage for which no profile is selected now". The §8 `member_runs` responsibility "selected profile" is dropped. The inspection-reply invariant in § Profile identity after removal is stated with the inspection contract. |
| `SPEC.md` profile and runtime contracts | The bundled-entry key list, "earlier runs keep their recorded `profile_version` text", the composer and capability retention sentence, every profile-id string (rewritten to "the BACKEND profile from VERSION", including the Role-eligibility evidence rows), the evidence, inspection, `permission_response` and digest key lists, the fingerprint and metadata-policy sentences, the answer-profile inspection sentence, the payload-digest "separate from any native profile version" clause, and the example evidence JSON |
| `SPEC.md` storage | The default URL rows, the HOME-expansion rule, the `member_runs` row, the persistence paragraph and the §8 baseline DDL become the v9 baseline; § V6, V7 and V8 storage and §8's V2/V3 through V8 migration sections are deleted |
| `docs/docs/concepts/storage.md` | The `cafleet_v9.db` default and the upgrade step: run `cafleet setup` and recreate fleets, with the v8 file left in place |
| `docs/` other pages | No page states the OpenCode form-layout limits or a profile identifier; Step 2 confirms this by repository search |
| `README.md` | No change: install commands and the pitch are unchanged |
| `skills/cafleet/reference/cli.md` | The v9 default URL, the inspection key list and text line, and the `permission_response` key list |
| `skills/cafleet/reference/coding-agents.md` | The opencode deny note's "A wrapped command … is answered manually" becomes the `W_min` and clipped-command limits; each Role-eligibility "Bundled support: profile `<id>`" becomes "Bundled support: the BACKEND profile from `<min_version>`" with an unchanged boundary and verdict |
| `.claude/rules/database-migrations.md` | "The `cafleet_v8.db` family has its own V1 baseline" names `cafleet_v9.db` |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Document the narrow layout and extract options

- [x] Document `margin_rows` and `row_trailer` in the `SPEC.md` operation-extract contract, and the split layout, the supported-width rule and the limits in `SPEC.md` §6.7 and §6.9; `W_split`, `W_min` and the pinned patterns follow in Step 4. <!-- completed: 2026-11-01T00:47 -->
- [x] Update the opencode deny note in `skills/cafleet/reference/coding-agents.md`. <!-- completed: 2026-11-01T00:47 -->

### Step 2: Document the identifier removal and the v9 format

- [x] Apply the `SPEC.md` profile, runtime and storage changes in § Documentation targets, including the v9 baseline DDL in §8. <!-- completed: 2026-11-01T00:53 -->
- [x] Update `docs/docs/concepts/storage.md`, and confirm by repository search that no other `docs/` page names a profile identifier or the OpenCode form-layout limits. <!-- completed: 2026-11-01T00:53 -->
- [x] Update `skills/cafleet/reference/cli.md`, the Role-eligibility rows in `skills/cafleet/reference/coding-agents.md` and `.claude/rules/database-migrations.md`. <!-- completed: 2026-11-01T00:53 -->

### Step 3: Capture narrow native evidence

- [x] Add the survey cases to the approved-case registry and run them, extend and bisect the survey per § Native capture procedure until `W_split` and `W_min` are exact, and record both in this document. <!-- completed: 2026-11-01T02:18 -->
- [x] Add and run the 57-column and `W_min` fixture cases, confirming each stage's gate in a fresh pane capture before its byte and applying the blocker list in § Answer stages. <!-- completed: 2026-11-01T02:18 -->
- [x] Pin the exact `controls`, `operation`, `unclipped` and `empty_reason` patterns, the continuation margin and `max_rows` from the captures, applying § Clipped commands. <!-- completed: 2026-11-01T02:18 -->
- [x] Review masking, convert the captures into fixtures with `cell_backgrounds`, add them to the native fixture directory with their SHA-256 digests, and record the evidence. <!-- completed: 2026-11-01T02:18 -->

### Step 4: Document the measured profile

- [x] Record `W_split`, `W_min`, the pinned patterns and the narrow fixtures in `SPEC.md` §6.9 and the opencode section of `skills/cafleet/reference/coding-agents.md`. <!-- completed: 2026-11-01T02:24 -->

### Step 5: Implement narrow detection

- [x] Write the extract-option, narrow-fixture, 36x120 regression, synthetic-screen and resize tests in § Tests. <!-- completed: 2026-11-01T03:25 -->
- [x] Implement `margin_rows` and `row_trailer` in the catalog loader and the detector's extraction. <!-- completed: 2026-11-01T03:25 -->
- [x] Widen `opencode-shell-permission` and `opencode-reject-reason` in `bundled.json`, register the narrow fixtures and add them to the detection and answer `fixture_ids`. <!-- completed: 2026-11-01T03:25 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: 2026-11-01T03:25 -->

### Step 6: Remove the profile identifier

- [x] Rewrite the tests that set or assert a profile identifier to the identifier-free shapes in § Output and record contracts, and add the shape, respond-binding and identifier-free invariant tests in § Tests. <!-- completed: 2026-11-01T01:13 -->
- [x] Remove `profile_version` from `bundled.json` and every surface in § Profile identity after removal, replace the three runtime invariants with their identifier-free forms, and remove the identifier comparison in § Respond-time binding. <!-- completed: 2026-11-01T02:25 -->

- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck` once Step 7's v9 baseline has landed, because the v8 head keeps `answer_profile_version` NOT NULL. <!-- completed: 2026-11-01T02:25 -->

### Step 7: Start the v9 format family

- [x] Write the config default, baseline-shape and v8-rejection tests in § Tests, and apply the version-dependent test changes in § Database format `cafleet_v9.db` (successor and post-V3 tests deleted, the repeated-migration and `legacy_opening` tests rewritten against the baseline, the diagnosis test on `APPLICATION_ID`). <!-- completed: 2026-11-01T01:17 -->
- [x] Replace `cafleet/migrations/V1__baseline.sql` through `V8__uncertain_cause.sql` with the v9 `V1__baseline.sql`, set `APPLICATION_ID` to `1128351289` and the default URL to `cafleet_v9.db`. <!-- completed: 2026-11-01T02:25 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: 2026-11-01T02:25 -->

### Step 8: Accept natively

- [x] Run Director-auto allow and deny at 57 columns on tmux and on herdr with the release build and a private database, sizing and confirming the member pane per § Acceptance and recording the evidence there. <!-- completed: 2026-11-01T03:47 -->

### Step 9: Validate the shipping surfaces

- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`, run the Success Criteria `git grep`, verify the bundled catalog and baseline match `SPEC.md`, and record the execution summary in this document. <!-- completed: 2026-11-01T03:52 -->

Execution summary (2026-11-01, macOS, Rust 1.99.1, working tree on top of `HEAD` with only the `SPEC.md` herdr-resize limit row uncommitted):

| Check | Result |
|---|---|
| `mise //cafleet:typecheck` | Pass |
| `mise //cafleet:lint` | Pass: clippy with `-D warnings` and `cargo fmt --check` |
| `mise //cafleet:test` | 1499 passed, 3 failed, 7 ignored at the default thread count. The three failures (`opencode_deny_moves_the_focus_selects_reject_and_submits_the_empty_reason`, `opencode_focus_on_always_allow_after_the_first_deny_stage_fails_uncertain`, `service_quiet_timer_persists_one_opening_without_changing_output_sequence`) each pass in isolation. |
| `mise //cafleet:test -- -- --test-threads=4` | 1502 passed, 0 failed, 7 ignored |
| `mise //docs:build` | Pass |
| Success Criteria `git grep` outside `design-docs/` | No match; the same pattern matches inside `design-docs/`, so the pattern itself works |
| Baseline against `SPEC.md` §8 | `cafleet/migrations/V1__baseline.sql` equals the §8 DDL line for line, and it is the only embedded migration |
| v9 identity | `APPLICATION_ID` `1128351289` and the default URL `~/.local/share/cafleet/cafleet_v9.db` in `config.rs` |
| Bundled catalog against `SPEC.md` §6.9 | Every profile entry has exactly `backend`, `min_version`, `rules` and `capabilities`. Every OpenCode region and matcher pattern appears verbatim in `SPEC.md`, except the unchanged idle composer prefix `^┃ `, which §6.9 describes in prose. Detection lists both request rules with the 120-, 57-, 55- and 36-column form fixtures, each request rule has its own allow and deny entries, and every narrow or compact fixture ID that `SPEC.md` names is registered. |

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-01 | Initial draft |
| 2026-11-01 | Review revision: shipped margin-then-trim extraction order, a capture anchored at the command box end, `max_rows` covering a growing long-command form, two-edge breakpoint bracketing, identifier-free runtime invariants, the version-dependent test changes, runner-owned resize tests and the acceptance pane-sizing procedure |
| 2026-11-01 | Approved by the user |
