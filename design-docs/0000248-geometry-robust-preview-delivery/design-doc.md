# Geometry-robust preview and composer delivery

**Status**: Complete
**Progress**: 61/61 tasks complete
**Last Updated**: 2026-11-28

## Overview

Previews, `member prompt`/`member exec` input and Director previews fail as `literal_mismatch` when the live pane is too short or narrow for the typed literal, or when a composer scrolls internally, and an uncertain preview is then retyped repeatedly (issue #660). This design picks the literal from the live pane geometry, so pre-submit verification always sees the whole literal. Delivery stays correct at every pane size and through mid-session resizes. When nothing fits, delivery holds with zero bytes and an explicit diagnostic, and after a failure each message gets exactly one more delivery, as one of the short poll rungs (pointer, cue or minimal).

## Success Criteria

- [x] Every bundled profile (each file under `assets/profiles/`) declares a measured `composer.geometry`. Catalog loading rejects a profile whose executable composer plan lacks it.
- [x] On every bundled backend, tmux and herdr, a 9-row and a 12-row pane at 80 and 120 columns, idle or working, deliver a preview of 90–430 body characters whenever a rung fits. It arrives as the rung §4 selects for the live geometry, with stage `submitted`, never as `literal_mismatch`.
- [x] At and above the per-profile full and compact sizes that Step 1 records in §2 § Rung sizes, on tmux and herdr, idle or working, a preview whose body is `B_full(profile)` characters arrives as the `full` rung, to a member and to the Director. At and above the compact size, a preview whose body exceeds `B_full` but is at most 4000 characters arrives as the `compact` rung.
- [x] Below each profile's `min_rows`/`min_columns`, down to 2 rows × 10 columns, no delivery writes a byte into a composer where the chosen rung does not fit. The preview holds with `composer_too_small`, publishes no `input_delivery_stalled` event, and is delivered automatically once the pane grows enough for a rung.
- [x] A literal clipped or scrolled at pre-submit verification records `composer_clipped` or `composer_unreadable`, never `literal_mismatch`. Only visible text that contradicts the literal records `literal_mismatch`.
- [x] A clipped or unreadable preview, whether to a member or to the Director, is submitted late without retyping when its full literal becomes visible within the late-submission window (120 s).
- [x] A message is typed at most twice: once in any rung and, after an uncertain outcome, once as the pointer, cue or minimal rung. A second uncertain outcome stamps `notified_at` and records a diagnostic. This holds for member and Director delivery.
- [x] `member prompt`/`member exec` whose literal does not fit the live composer queue with zero bytes under `composer_too_small` and drain automatically when it fits.
- [x] `member show` reports live `observation.geometry`. Its `below_minimum` clears without operator action when the pane grows back. `doctor` prints a `member.<id>.geometry` row per probed member and a `fleet.<fleet_id>.director.geometry` row per fleet whose Director pane receives Director delivery. A below-minimum Director pane holds under `composer_too_small` immediately, not after 3 minutes as `composer_busy`.
- [x] `member create` admits a member to a column only when every pane in it keeps at least its backend's slot budget. A resize after creation is reported and never re-plans the layout.
- [x] Step 1 reproduces both unexplained behaviors in issue #660 with recorded fixtures and records their root cause in this document.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

### Verified root cause

| Component | Current behavior | Effect on a short pane |
|---|---|---|
| `bottom_closed` (`src/coding_agent/detector.rs:461`) | Closes a `composer_text` region at the screen bottom when no `end_pattern` row follows | Claude yields only the visible composer rows. The issue's "`composer_text` returns `None`" diagnosis is inaccurate for Claude |
| `ComposerText::matches` (`src/coding_agent/catalog.rs:814`) | Succeeds only when the segments consume the entire literal | A clipped or internally scrolled composer shows part of the literal, so the match fails |
| `arm_pending` (`src/runtime/input.rs:4066`) | A failed match after the pre-submit window records `LiteralMismatch` | Unreadable, clipped and contradicting composers share one stop cause |
| Plain-vs-pointer selection (`select_delivery`, `src/runtime/input.rs:2326`; Director `attempt`, `src/runtime/service.rs:1777`) | A fixed `plain_literal_limit` / `director_delivery.plain_limit` per profile, measured on full-height panes of 80–120 columns | The literal's form ignores the live rows and columns |
| `record_uncertain` (`src/runtime/input.rs:2489`) | On a verified-composer run an uncertain sweep preview stays unnotified | The sweep retypes it once the composer verifies empty again |
| `oldest_pending_preview` (`src/broker/messaging.rs:92`) | Strict oldest first; `PENDING_PREVIEW` already excludes consumed rows | One stuck preview blocks every newer message |

The other profiles fail the same way, through different mechanisms:

- The pi composer matcher requires the bottom `─+` row, so a clipped pi editor cannot be extracted at all.
- codex, opencode, pi and the Hermes TUI cap their composers at a fixed number of rows and scroll internally (SPEC §6.5 Accepted trade-offs).
- A scrolled window can lose the `first_prefix` row or show only the literal's tail.

### Unexplained behaviors (gating Step 1)

- A preview was typed again while an earlier copy still sat in the composer.
- `member show` reported `observation.eligibility=composer_empty` during the stall.

Root cause, reproduced with Claude 2.1.327 on tmux 3.8 (one member at 9x88 while working) and on herdr 0.9.5 (five members in one column, four of them at 9 rows). No empty-composer rule matches a screen that holds a literal, so no rule correction or negative fixture is needed.

1. **First typing.** A preview's first typing into a 9-row pane records `literal_mismatch`/`partial`. On herdr, four 3-row previews to idle members all failed with the bottom rule and status rows clipped, exactly as in the issue. On tmux, a 205-character body scrolled the composer internally: `─── ↑ 1 more ───` replaced the top border, the `❯ ` row left the screen and the bottom rule was clipped.
2. **`composer_empty` during the stall.** A Director `member press Enter` on a busy member moves the literal into Claude's native queue: a transcript row followed by `⎿  Queued · <hint> to send now`, where the hint reads `ctrl+x ctrl+s` on tmux and `ctrl+enter` on herdr. The composer then shows `❯ ` with a faint `Press up to edit queued messages`, which `claude-suggestion-composer` correctly classifies as `composer_empty`.
3. **Retype while a copy sits in the composer.** `record_uncertain` does not stamp `notified_at` on a verified-composer run, so the runner retypes the unnotified preview into the emptied composer. One copy then sits in the native queue and a second in the composer; an idle Enter that submits one literal is followed by the same retype. The second uncertain outcome publishes no new `input_delivery_result`, because results are deduplicated per run and message.
4. **Third typing.** After the queued copy ran, the member replied from the inline body without `message poll`, so the message stayed `input_required`. When the composer next verified empty, the runner typed it a third time and submitted it, and Claude answered that it was a redelivery. A member left untouched stalled after 3 minutes.

Evidence lives in BASE/.verify/: repro-tmux.md, repro-tmux-part2.md, repro-herdr.md and tmux/claude/repro-*.txt.

### Geometry changes are normal

A terminal resize, such as moving between an external monitor and the built-in display, shrinks or grows every pane at any time after creation. The issue's five 9-row panes in a 53-row tab came from such a resize, not from a planner defect. Placement never migrates panes (SPEC § Placement policy). Delivery must therefore be correct at every geometry the multiplexer allows. The per-profile minimum guides placement and diagnostics only.

---

## Specification

### 1. Scope

| Path | Ladder | Late submission | Single pointer retry | Fit failure |
|---|---|---|---|---|
| Member preview sweep (verified-composer run) | yes | yes | yes | Hold, zero bytes, `composer_too_small` |
| Director delivery sweep | yes | yes | yes | Hold, zero bytes, `composer_too_small` |
| `member prompt` / `member exec` (verified-composer run) | no (literal is fixed) | no | no | Queue with zero bytes, `defer_reason=composer_too_small` |
| Any run without composer verification or with fallback delivery | unchanged | no | no | unchanged |

Prompt and exec do not park for late submission, for two reasons. Their receipts promise the caller an outcome within the 5 s attempt deadline, and `input reconcile` already settles an unsubmitted literal. Fit-before-type limits this case to a resize inside the roughly 1.5 s between the literal and the submit key.

### 2. Profile geometry (`capabilities.composer.geometry`)

`composer.geometry` is required whenever the composer declares an executable submission plan. A `composer_text` rule may carry `text_geometry`, holding any of the fit fields (every field below except `min_rows` and `min_columns`). Those fields replace the profile-level values for the regions that rule bounds (Hermes has a classic and a TUI composer). `min_rows` and `min_columns` are backend-wide and always come from the profile-level object. They must cover every composer the profile reads: the smallest geometry at which each `composer.text` rule's empty composer verifies and the minimal rung fits on one row.

Every composer is fitted as bottom-bounded. Once the transcript fills the screen, each native, Claude and opencode included, keeps a measured set of rows between its composer and the screen bottom and grows the composer upward.

```json
{
  "margin_columns": 4,
  "first_row_margin_columns": 0,
  "text_window": {"fraction": 0.5, "offset": 5, "min": 3},
  "reserved_rows": {"steps": [[9, 8], [12, 4]]},
  "transient_rows": 4,
  "wrap_slack_rows": 0,
  "carried_space": false,
  "fit_wrap": "word",
  "collapsed_paste": null,
  "min_rows": 9,
  "min_columns": 80
}
```

The object is Claude's measured geometry (§ Measured values).

| Field | Type | Meaning |
|---|---|---|
| `margin_columns` | integer ≥ 0 | Cells per composer row unavailable to text: prefix, margin and borders. Text columns `T = cols − margin_columns` |
| `first_row_margin_columns` | integer ≥ 0 | Further cells the first text row loses when the native's first-row prefix is wider than its continuation prefix (Hermes classic `☤ ❯ ` while working: 4). The first row holds `T − first_row_margin_columns` cells |
| `text_window` | `null`, a fraction object or a step object | The composer's internal window `W(rows)`, past which the native scrolls its text (forms below) |
| `reserved_rows` | integer ≥ 0 or a step object | Non-text rows extraction needs on screen at a size: the frame, the prompt row and the rows the native keeps between the last text row and the screen bottom |
| `transient_rows` | integer ≥ 0 | The largest number of chrome rows (spinner, `Tip:`, update notice, queued-input rows) that can appear while a literal is being typed and displace the composer |
| `wrap_slack_rows` | 0 or 1 | Rows added to the greedy word-wrap estimate when the native wrap algorithm can waste a row compared with greedy wrapping |
| `carried_space` | boolean | For `word` wrap: when a word ends exactly at a row's last text column, the separating space starts the next row and takes one of its cells. `false` when the native drops that space |
| `fit_wrap` | `char` or `word` | The wrap `R` is computed with. Equal to the rule's text wrap unless the native wraps differently from how extraction reads it |
| `collapsed_paste` | `{"min_scalars": N, "rows": 1}` or `null` | For a `bracketed_paste` plan: a bracketed literal of at least `N` scalars renders as a one-row placeholder |
| `min_rows`, `min_columns` | integers ≥ 1; profile level only | The smallest captured geometry at which every composer's empty composer verifies and the minimal rung fits on one row. Used for placement, `below_minimum`, `hold:geometry` and the Director `composer_too_small` hold; correctness never depends on them |

| `text_window` form | `W(rows)` |
|---|---|
| `null` | Unbounded: the native has no internal window |
| `{"fraction": f, "offset": o, "min": m}` with `0 ≤ f ≤ 1`, `o ≥ 0`, `m ≥ 1` | `max(m, floor(f × rows) − o)`; `f = 0` gives a fixed window of `m` rows |
| `{"steps": [[min_rows, W], …], "uncapped_from": N}` | The `W` of the last step whose `min_rows ≤ rows`, and unbounded at `rows ≥ N`. `N` is `null` when the last step applies at every larger size |

`reserved_rows` takes the step form `{"steps": [[min_rows, r], …]}` when the measured value depends on the size; it gives the `r` of the last step whose `min_rows ≤ rows`. Below the first step of either step list nothing fits. A step list is non-empty, its `min_rows` strictly increase from at least 1, every window `W` is at least 1, and `uncapped_from` exceeds the last step's `min_rows`.


A `composer_text` rule may declare `scroll_marker`, a regex matching the indicator the native draws while its window hides earlier text rows. The rule's `start_pattern` and region matchers accept the marker row, so a scrolled composer still opens a region. Extraction strips the marker and §6 classifies a marked region as scrolled; a rule without `scroll_marker` is classified by the `V₀` equality (§6).

Catalog loading rejects an executable composer plan without `composer.geometry` with `composer plan for <backend> requires composer geometry`. It rejects an invalid field with `composer geometry of <backend> is invalid: <field>`.

#### Measured values

Measured on 2026-11-27 at the installed natives on tmux 3.8 and herdr 0.9.5. At matching PTY sizes, every backend's herdr capture shows the same visible literal rows, first visible row, scroll marker and closing row as its tmux capture, so the values are multiplexer-independent. A step's smaller sizes come from the tmux empty-screen captures, which Step 12's `min_rows`×`min_columns` and 5×40 fixtures confirm.

| Rule | Native | `margin_columns` | `text_window` | `reserved_rows` | `transient_rows` | Smallest verified-empty size | `scroll_marker` |
|---|---|---|---|---|---|---|---|
| `claude-composer-text` | 2.1.327 | 4 | `{"fraction": 0.5, "offset": 5, "min": 3}` | `{"steps": [[9, 8], [12, 4]]}` | 4 | 6x80 | Top border `─── ↑ N more ───` |
| `codex-composer-text` | 0.170.0 | 3 | `{"steps": [[5, 3], [6, 4], [7, 5], [8, 6], [14, 7], [15, 8], [18, 10]], "uncapped_from": 21}` | 2 | 2 | 3x80, 5x40 | `↑` at the end of the first visible row |
| `opencode-composer-text` | 2.3.2 | 4 | `{"fraction": 0, "offset": 0, "min": 6}` | 2 | 0 | 4x80, 5x40 | — |
| `pi-composer-text` | 1.5.2 | 1 | `{"fraction": 0.3, "offset": 0, "min": 5}` | `{"steps": [[5, 3], [6, 4], [7, 6]]}` | 0 | 4x80, 5x40 | Top border `─── ↑ N more ───`, after the working label while working |
| `hermes-composer-text` (classic) | 2026.9.24 | 0, with `first_row_margin_columns` 4 | `{"fraction": 0, "offset": 0, "min": 8}` | `{"steps": [[5, 2]]}` | 0 | 2x10 | — |
| `hermes-tui-composer-text` | 2026.9.24 | 6 | `null` | 3 | 4, unmeasured | Step 12 | — |

The Step 12 working captures show no chrome taking composer rows on opencode (the status row changes in place), pi (the label sits in the top border) or Hermes classic (the spinner sits in the title border), so their `transient_rows` is 0. No working Hermes TUI screen could be produced, so that rule keeps the largest measured chrome, Claude's 4 rows, as a known gap. pi sets `wrap_slack_rows` to 1: it wraps a word that would end exactly at column `T`, so a literal can take one row more than greedy wrapping. `hermes-tui-composer-text` sets `carried_space` to true: a continuation row after a row filled to its last text column starts with the separating space. Every other rule drops that space, so `carried_space` is false there.

| Profile | `min_rows` | `min_columns` | Smallest typeable size |
|---|---|---|---|
| `claude` | 9 | 80 | 9x80: one text row above the bottom rule; no narrower typeable size was captured |
| `codex` | 5 | 40 | 5x40: a 3-row window |
| `opencode` | 4 | 40 | 4x80 and 5x40: a 2-row window |
| `pi` | 5 | 40 | 5x40: a 2-row window |
| `hermes` | 5 | 64 | 12x64: the classic working row renders its `❯ ` only from 64 columns; the idle composers verify down to 5x40, with 4 classic and 2 TUI text rows |

- **Claude.** `W` is 3 at 9–15 rows and 4, 5, 7, 8 and 10 at 18, 21, 24, 27 and 30 rows. Live fit reserves one more column beyond the measured 4-cell margin. At 12×120, a compact literal with an ellipsis at column 116 of its second predicted row rendered on four rows, hiding its first row. At ≥ 12 rows the bottom rule and normally two status rows stay below the text. At 9 rows the frame overflows the screen bottom: only the bottom rule stays below the `❯ ` row, and a literal of two or more rows loses it. The working chrome is the spinner, a `Tip:` row that wraps to 2 rows at 86 columns, and a blank row: up to 4 rows above the composer.
- **codex.** The two footer rows hide once the literal needs their space. A scrolled region keeps `› ` on its first visible row. The working chrome is the `• Working (…)` row plus a blank row.
- **opencode.** At ≤ 16 rows the blank rows above and below the `┃` rows disappear; the bundled rule requires the blank row above, so a fully visible 3-row literal records `literal_mismatch` on tmux and herdr, which Step 6 corrects. The window is 6 rows at every height ≥ 9, and scrolling shows no indicator.
- **pi.** The bottom border and three status rows stay below the text at ≥ 9 rows, two status rows at 6 rows and one at 5 rows. The bottom border stays drawn for at most 3, 4 and 5 text rows at 9, 10 and 11 rows, so `reserved_rows` is 6 from 9 rows; 7 and 8 rows were not measured and take the larger neighbouring value, 6. At 6x80 and 5x40 a literal of three or more rows shows no border.
- **Hermes classic.** The composer character-wraps at every width and in every state; `wNNN` words only look word-wrapped at widths divisible by 5. The first row opens with `❯ ` (2 cells) idle and `☤ ❯ ` (4 cells) working, and continuation rows use the full width; `first_row_margin_columns` 4 is exact while working and under-fills the idle first row by 2 cells, which only shortens the fit. The rule keeps `word` extraction, because a row break may match nothing, and sets `fit_wrap` to `char`; every other rule's `fit_wrap` equals its text wrap. The rule and status row stay below the text at every typeable size, 5x64 included; at 5x40, below the Hermes minimum, only the status row stays. The window is 8 rows at every height ≥ 12, and the `❯ ` row scrolls away without an indicator.
- **Hermes TUI.** The text grows downward from the ` ❯ ` row while the status row moves up. A literal that does not fit loses its tail at the screen bottom without an indicator.

#### Rung sizes

Derived from the measured values with §3, and checked live by Success Criterion 3.

- **Full-size body.** `B_full(profile)` is the longest ASCII body whose `full` literal stays within both the profile's prompt `plain_literal_limit` and its `director_delivery.plain_limit`. The reference envelope is a three-digit message ID and sender ID with a canonical 32-scalar timestamp: a 60-scalar header `[cafleet msg 123 from 456 <TS>] ` and the 59-scalar resume suffix.
- **Full size.** For each profile and each of 80 and 120 columns, the smallest pane height at or above `min_rows` at which a `B_full` body fits the `full` rung with headroom.
- **Compact size.** The smallest such height at which the `compact` rung fits with an excerpt of at least 80 scalars, with a four-digit `N` and a three-digit recipient ID.

The bodies are `w001 w002 …` words, as in the live checks. A size marked provisional depends on the unmeasured Hermes TUI `transient_rows`. Hermes members deliver through the classic composer and the Hermes Director through the TUI composer.

| Profile | Prompt / Director plain limit | `B_full` | Full size at 80 / 120 columns | Compact size at 80 / 120 columns |
|---|---|---|---|---|
| `claude` | 456 / 456 scalars | 337 | 24 / 18 | 18 / 12 |
| `codex` | 1024 bytes / 1024 bytes | 905 | 24 / 18 | 8 / 7 |
| `opencode` | 380 / 380 scalars | 261 | 8 / 6 | 6 / 5 |
| `pi` | 474 / 474 scalars | 355 | 24 / 11 | 11 / 10 |
| `hermes` | 558 / 1480 scalars | 439 | Classic 10 / 7, TUI 15 / 12, provisional | Classic 6 / 5, TUI 11 / 10, provisional |

### 3. Fit computation

Inputs are the live snapshot's `rows` and `cols` and the effective geometry of the one `composer.text` rule that `Detector::composer_text` selects on the verified empty screen: that rule's `text_geometry` fields over the profile-level values.

When no `composer.text` rule, or more than one, matches a screen that observation verifies empty, the profile does not describe that screen. Delivery then holds with zero bytes under `composer_geometry_unavailable`. That reason is not a geometry hold: it runs the ordinary stall wait and records `preview delivery: message <id> held: composer_geometry_unavailable` (or the `director delivery:` form), because only a profile correction resolves it. Provenance requires every verified-empty fixture to select exactly one `composer.text` rule, so the bundled profiles exhibit this only on uncaptured screens.

The member runner uses its PTY screen model (`transport.snapshot()`). The Director sweep uses the capture's geometry:

| Multiplexer | `rows` | `cols` |
|---|---|---|
| tmux | `#{pane_height}` | `#{pane_width}` |
| herdr | `scroll.viewport_rows` | `min(rect.width − HERDR_PANE_CHROME_COLUMNS, widest capture row in display cells)`, with `HERDR_PANE_CHROME_COLUMNS = 2` |

herdr resizes a pane's PTY only while a client renders its tab, so a background tab keeps the size it was last rendered at while its `rect` follows the current splits. `scroll.viewport_rows` tracks the PTY. The rect term bounds a tab last rendered wider than its rect, and the widest-row term bounds one last rendered narrower, because no captured row is wider than the PTY. Both terms only under-count, so the fit stays conservative without a rendered-tab check. With herdr's default `pane_scrollbars`, a pane whose native draws on the primary screen with scrollback (opencode, Hermes classic) gives one column to the scrollbar, so its PTY is one column narrower than the pane area; the capture omits that column, so the widest-row term covers it.

Replay uses the greater of the herdr layout width and the longest visible row's display width to preserve the original rows. A zoomed Director pane can retain its split rectangle while its PTY widens, just as a background pane can retain an earlier width. Fit still uses the conservative width above.

- Text columns: `T = cols − margin_columns`, and the first text row holds `T − first_row_margin_columns` cells. Nothing fits when either is below 1.
- Visible text rows, with headroom `h`: `V = min(W(rows), rows − reserved_rows(rows) − h)`, evaluating `text_window` and `reserved_rows` at the live `rows` (§2). Nothing fits when `V < 1`.
- Required rows `R(literal)` for the rendered literal, including the operation prefix, wrapped by the effective `fit_wrap`:
  - A bracketed literal at or above `collapsed_paste.min_scalars` takes `collapsed_paste.rows`.
  - `char` wrap: fill cells greedily, where a scalar occupies its `unicode-width` cell count and a wide scalar that does not fit starts the next row.
  - `word` wrap: break greedily at the last space that fits, hard-break a word longer than its row, and add `wrap_slack_rows`. With `carried_space`, a row that follows a word ending exactly at the last text column starts with the separating space, so it holds one cell less.
- Fit: `R ≤ V`.

Headroom `h` is `transient_rows` for the full and compact rungs and for prompt/exec, and 0 for the pointer, cue and minimal rungs. Headroom guards only against chrome appearing between the literal and the submit key. On the short rungs, that race falls to late submission (§6) and the single retry (§7), so the shortest rungs stay available in small panes. A prompt or exec has no retry, so it waits for headroom.

For Claude, when the verified composer closing border is visible, measure the extent through the last nonblank footer row and include both borders. Subtract the excess over the profile's reserved rows from the visible fit budget. This accounts for custom status rows that shrink the native window even while the pane size stays fixed. A clipped border uses the profile budget, and every literal still needs a complete pre-submit comparison.

For Codex, bound the live fit by `min(rows, max(8, floor(2 × rows / 3))) − 2`, with subtraction saturating at zero. The owned transcript caps the composer and the textarea insets consume two rows. Keep the measured window for fixture provenance and apply the native bound during planning. The full preview requires 14 rows at 80 columns, first available at a pane height of 24. [Codex composer height limit](https://github.com/openai/codex/blob/rust-v0.170.0/codex-rs/tui/src/app/side_pane.rs#L410-L415).

`unicode-width` (0.2, already in `Cargo.lock` through `vt100`) becomes a direct dependency. The computation lives in a new `src/coding_agent/geometry.rs` (`ComposerGeometry`, `fit_rows`, `visible_text_rows`), and `Detector::composer_geometry(&ScreenSnapshot) -> Result<Option<LiveComposer>>` returns `{rows, cols, geometry}` from the selected `composer.text` rule, where `geometry` holds the effective fit fields, or `None` when the selection fails.

### 4. Preview ladder

Selection walks the rungs in order and takes the first one whose literal fits (§3) and that the run's or Director's existing full-height plain limit allows:

| Rung | Literal (profile delivery) | Eligible when |
|---|---|---|
| `full` | `[cafleet msg ID from SENDER TS] BODY [cafleet] Resume your work if something was still running.` (unchanged canonical form, including the existing 4000-scalar body pointer) | `preview_typings` is 0 |
| `compact` | `[cafleet msg ID from SENDER TS] EXCERPT… (N more characters; run cafleet message poll RECIPIENT_ID --json) [cafleet] Resume your work if something was still running.` | `preview_typings` is 0, the folded body is inline (≤ 4000 scalars), and an `EXCERPT` of at least 1 scalar fits |
| `pointer` | `[cafleet msg ID from SENDER TS] Body is N characters; run cafleet message poll RECIPIENT_ID --json to read it. [cafleet] Resume your work if something was still running.` (unchanged pointer form) | always |
| `cue` | `[cafleet msg ID] run cafleet message poll RECIPIENT_ID --json` | always |
| `minimal` | `[cafleet msg ID]` | always |

- `EXCERPT` is the fold of the longest stored-body prefix, ending on a stored scalar boundary, for which the compact literal fits. Because the prefix is cut in stored text before folding, a fold unit (`\r\n` → `⏎`, a control character → `\xNN`) is either wholly inside the excerpt or wholly outside it. `N` is the stored body's scalar count minus the stored scalars of that prefix, counted like the pointer's `N`.
- A run without composer verification keeps its current selection: plain limit, then pointer.
- The rung is pinned at the first byte, like framing today. Reselection while zero bytes are established reruns the ladder against fresh geometry.
- Recipients treat every rung as a wake. `[cafleet msg ID]` alone means "run `message poll`", and the member and Director skills say so (Step 2).

When no rung fits, the preview holds (§5).

### 5. Geometry hold

| Condition | Observation eligibility | Delivery action |
|---|---|---|
| Composer verified empty but no rung (preview) or the literal (prompt/exec) fits | `composer_empty` | Zero bytes. A preview stays pending and a prompt/exec queues under `defer_reason=composer_too_small` |
| No composer or interaction rule matches and `rows < min_rows` or `cols < min_columns` | `hold:geometry` (new `GateReason::Geometry`) | Zero bytes, as above |
| No rule matches at or above the minimum | `hold:unknown` (unchanged) | Ordinary composer hold, with the 3-minute stall bound |
| Any other hold (a draft, a busy composer, a non-empty composer) while the pane is below the minimum | Unchanged | Ordinary composer hold, with the stall wait suspended (below) |

- **Stall clock.** The stall wait is suspended by pane size, whatever the eligibility. A held preview or queued head on a pane whose live `below_minimum` is true starts no `input_delivery_stalled` wait, and an open wait is cleared. The `composer_too_small` holds of row 1 suspend it at any size. The wait starts fresh when the pane is no longer below the minimum and the hold persists, so a draft in a below-minimum pane stalls only after the pane grows to the minimum.
- **Director pane.** `DirectorHold` gains `ComposerTooSmall` (`composer_too_small`), applied when either condition holds:
  - The capture geometry (§3) is below the Director profile's `min_rows`/`min_columns`, and no Director composer or interaction rule matches.
  - The composer is verified empty but no rung fits.

  The first condition replaces the `composer_busy` hold the attempt records today for an unmatched screen (`src/runtime/service.rs:1826`), but only for a capture below the minimum and with no Director composer or interaction rule matching. A matched busy or draft composer stays `composer_busy` at any size, and so does an unmatched screen at or above the minimum, mirroring §5 rows 2–4.
- **Retries.** A geometry hold never retypes, never stamps `notified_at` and needs no Director action. The sweep re-evaluates fit at each 1 s interval without backoff, because the PTY screen model already tracks size.
- **Diagnostics.** Member delivery records `preview delivery: message <id> held: composer_too_small` (source `preview_sweep`) once per continuous hold. Director delivery records `director delivery: message <id> held: composer_too_small` once per runner per continuous hold, immediately rather than after 3 minutes. A queued prompt/exec reports `defer_reason=composer_too_small` through `input show` and its queued CLI reply.
- **Interactions.** An interaction rendered below the minimum may go undetected, as a recorded layout limit (SPEC §6.7). Ordinary input still writes nothing, because the composer does not verify empty.

### 6. Pre-submit verification classes and late submission

`ComposerText::matches` becomes `ComposerText::compare(&self, chunks) -> CompareOutcome`, and `matches` is removed:

| `CompareOutcome` | Condition | Submit key | Stop cause at window close |
|---|---|---|---|
| `Complete` | Segments consume the entire literal and the region has no explicit native scroll marker | Written | — |
| `Partial` | The region is clipped or scrolled (below), and the segments consume a contiguous run of the literal: starting at its first scalar for a clipped-only region, at any offset for a scrolled region | Not written | `composer_clipped` |
| `Contradicts` | Any other extractable text, including text extending beyond the literal | Not written | `literal_mismatch` |
| (no text) | `Detector::composer_text` returns `None` | Not written | `composer_unreadable` |

A region is **clipped** when the screen's bottom edge closed it (`bottom_closed`). Whether it is **scrolled** depends on its rule:

- A rule with `scroll_marker` (§2): the region is scrolled when it shows the native marker. Codex renders the marker as a faint arrow in the right gutter, so arrows in the editable text remain part of the comparison. Cursorless captures use the widest painted row to locate the gutter independently of the layout rectangle.
- A rule without `scroll_marker`: the region is scrolled when its extracted text rows equal the native window cap `V₀`, which is §3's `V` with `h = 0`, recomputed from the pre-submit screen's live `rows` and `cols`.

A region that is both clipped and scrolled is classified as scrolled, so the contiguous run may start at any offset.

Extraction removes each marker match from its row before the prefixes are read, so the comparison never sees the marker text. A row the removal leaves empty counts as the frame row when it opens a framed rule's region, and is dropped otherwise. The first text row of a scrolled region reads with `first_prefix` when that matches, else with `continuation_prefix`, because the native's first-prefix row may have scrolled away.

`Partial` only classifies the failure. It never authorizes a submit key, so tail verification stays unchanged. A pi editor that overflows its window without the `↑ N more` marker draws no bottom border, and its text rows run straight into the status rows. Its region does not close, so the comparison records `literal_mismatch`; when no pi region remains on screen, the composer text is unreadable and the window closes as `composer_unreadable`, which parks a preview. Fit-before-type keeps a pi literal inside its measured window, so this arises only from a resize between the literal and the submit key, which the single pointer retry (§7) covers.

**Late submission.** This applies only to member and Director previews. A pre-submit window that closes with `composer_clipped` or `composer_unreadable` parks the attempt instead of ending it:

1. In one write transaction, still holding the pane input lock, set `messages.preview_parked_until = now + LATE_SUBMISSION_WINDOW` (`LATE_SUBMISSION_WINDOW = 120 s`), but only while `notified_at` is null and the message is `input_required`. Then release the lock. The runner keeps the attempt's written literal, rung, `message_id` and stage `partial` in memory.
2. At every sweep interval (1 s), the parking runner re-reads the message and observes the composer (Director: one capture), then compares again:
   - The message is no longer `input_required`, because a `message poll` consumed it: end the park with no submit key and no uncertain outcome. Clear `preview_parked_until` and record `preview delivery: message <id> park ended: consumed` (or the `director delivery:` form). A literal left in the composer is then an ordinary non-empty composer.
   - `Complete`: acquire the lock nonblockingly, start a fresh 5 s attempt deadline, re-observe, require `Complete` again, write the submit key and run the 2 s post-submit verification. Confirmed submission stamps `notified_at`, clears `preview_parked_until` and ends the park.
   - `Contradicts`: end the park as uncertain `literal_mismatch`.
   - Verified empty composer: a Director preview parked while waiting for its first draft render stays parked until a draft renders or the window expires. Otherwise, end the park as uncertain with the park cause.
   - `Partial` or no text: keep parked.
3. When `preview_parked_until` passes, end the park as uncertain with the park cause.

An uncertain park end clears `preview_parked_until` in the same transaction that applies §7.

While a message's `preview_parked_until` is in the future:

- **No other typing.** No runner types that message, so the parker alone owns it. Every member sweep and every runner's Director sweep treats such a head as held, keeping strict oldest-first order. The parking member runner's own sweep starts nothing else, as when an attempt is pending.
- **No stall wait.** The preview's stall wait does not run. `member press` stays available to the Director.
- **Parker loss.** Shutdown ends the park as uncertain `runner_stopped`. A parker lost without shutdown leaves the column set. Once it has passed, any runner that selects the head clears it and applies §7 as an uncertain outcome with cause `runner_stopped` before typing.

An uncertain outcome from a park follows §7.

### 7. Single pointer retry

`messages.preview_typings INTEGER NOT NULL DEFAULT 0` counts the typings of a message across every runner that delivers it. A typing is counted in a write transaction immediately before its first literal byte, while the delivering runner holds the pane input lock. The count comes after the screen has verified empty and a rung has been selected, so a zero-byte outcome never consumes a typing:

| Path | Count point |
|---|---|
| Member sweep | After the reservation passes its fresh drain and interaction check, and after the rung is pinned. A reservation that returns to `ready` with zero bytes has not counted |
| Director sweep | Inside `attempt`, after the classifying capture verifies the composer empty and the rung is selected, and before `type_literal_until`. The atomic reservation requires `input_required` and is released under the pane input lock when the outcome proves zero bytes were written |

The transaction refuses the typing when `preview_typings = 2`, `notified_at` is set or `preview_parked_until` is in the future, and the attempt then ends with zero bytes. A crash between the count and the first byte overcounts by one typing.

**Exhausted state.** A selected head with `preview_typings = 2`, `notified_at IS NULL` and no future `preview_parked_until` is exhausted. It can arise from an overcount, or from an uncertain second typing whose stamp failed. The selecting runner stamps `notified_at` and records the abandonment diagnostic with cause `typings_exhausted`, so the head advances to the next message.

| `preview_typings` before the typing | Rungs allowed | An uncertain outcome of this typing (any stop cause, verified-composer member run or Director sweep) |
|---|---|---|
| 0 | Any rung | `notified_at` stays null; the next typing waits for a verified empty composer |
| 1 | `pointer`, `cue` or `minimal` | Stamp `notified_at` |

- **Second-outcome diagnostics.** The second outcome records `preview delivery: message <id> abandoned after pointer retry: <cause>` (member, source `preview_sweep`) or `director delivery: message <id> abandoned after pointer retry: <cause>` (Director). The message stays `input_required` and reaches the recipient through any later `message poll`.
- **Event deduplication.** The member's uncertain-preview `input_delivery_result` stays deduplicated by run plus `message_id`, so the second outcome adds only the diagnostic.
- **Retyping.** A pointer retry is idempotent for the recipient: `message poll` returns `[]` once the message is consumed. Because a poll consumes every pending message, the retry also delivers the newer messages queued behind the stuck one, so no skip-ahead is needed.
- **Runs without composer verification.** These keep stamping `notified_at` before publication. They still count typings, but the count never restricts their selection.

### 8. Resize timeline

| Resize moment | Behavior |
|---|---|
| Before the first byte | Selection reads the live geometry; a zero-byte reselection reruns the ladder |
| Between the literal and the submit key | The pre-submit comparison runs against the live screen, classifying a shrink that clips or scrolls the composer by §6. `Complete` submits; `Partial` or no text parks (§6, previews) or records `uncertain/composer_clipped` or `uncertain/composer_unreadable` (prompt/exec); `Contradicts` records `literal_mismatch` |
| During post-submit verification | Unchanged: confirmation or `submit_unconfirmed` |
| While parked | A grow that reveals the whole literal submits it late |
| While held | A grow that makes a rung fit delivers it at the next sweep interval; `below_minimum` clears in the same health update |

### 9. Geometry reporting

**Observation health.** `geometry` is appended after `gap_open_since`, with ordered keys `rows,columns,min_rows,min_columns,below_minimum`. It is null when the run's profile declares no geometry or the child has not started. `below_minimum` is `rows < min_rows || columns < min_columns` for the live PTY size, so it clears itself when the pane grows. `ObservationHealth::validate` checks `below_minimum` against the other four values. Diagnostics rows stay as history.

**`member show`.** The text line appends ` geometry=ROWSxCOLUMNS below_minimum` after the observation state when `below_minimum` is true.

**`doctor`.** One `member.<member_id>.geometry` row follows each `member.<member_id>.runtime` row and uses the same health probe:

| Situation | Status | Detail |
|---|---|---|
| Runtime row not probed, probe failed, or `geometry` null | `info` | `run=<run_id> geometry=unavailable` |
| `below_minimum` | `warn` | `run=<run_id> rows=<r> columns=<c> below the <backend> minimum <min_rows>x<min_columns>; previews shorten or hold until the pane grows` |
| Otherwise | `ok` | `run=<run_id> rows=<r> columns=<c>` |

**`doctor`, Director pane.** When the `database` row is `ok`, one `fleet.<fleet_id>.director.geometry` row follows the member rows for each fleet that meets three conditions: its root Director is active with a concrete placement, the Director backend's bundled profile declares `director_delivery` listing the placement's multiplexer, and that profile declares `composer.geometry`. Rows appear in fleet-ID order. doctor takes one read-only capture of the Director pane (§6.5 Backend operations) and derives its geometry as the Director sweep does (§3), under the same one-second deadline as the member probes:

| Situation | Status | Detail |
|---|---|---|
| Capture failed | `info` | `director=<member_id> geometry=unavailable: <cause>` |
| Below the Director profile's `min_rows`/`min_columns` | `warn` | `director=<member_id> rows=<r> columns=<c> below the <backend> minimum <min_rows>x<min_columns>; Director previews shorten or hold until the pane grows` |
| Otherwise | `ok` | `director=<member_id> rows=<r> columns=<c>` |

The delivering members' `member show` diagnostics carry the Director hold itself (§5).

**Runtime diagnostic.** When `below_minimum` turns true, the runner records `observation: pane <rows>x<columns> below the <backend> minimum <min_rows>x<min_columns>` (source `observation`).

### 10. Layout planner

`MIN_PANE_HEIGHT` is replaced by a per-backend slot budget, `slot_rows(backend, multiplexer) = max(DEFAULT_MIN_ROWS, min_rows(backend)) + pane_chrome_rows(multiplexer)`, with `DEFAULT_MIN_ROWS = 14`. `min_rows` is the profile's `composer.geometry.min_rows`; a backend with no profile geometry takes `DEFAULT_MIN_ROWS`. `pane_chrome_rows` is the rows a pane's border or separator takes: 1 on tmux and 2 on herdr. `min_rows` exists so delivery degrades safely, not as a placement target: placement keeps today's 15-row tmux slots, and only a profile whose own minimum exceeds the floor raises its slot. A column of window height `H` holding members of backends `B` admits backend `b` when both hold:

- `count(B) + 1 ≤ MAX_PANES_PER_COLUMN` (8);
- `floor(H / (count(B) + 1)) ≥ max(slot_rows(b), max over B of slot_rows)`.

An overflow column always admits its first member. `choose_slot` receives each column's member backends, from `member_placements.coding_agent`, the creating backend and the multiplexer. Slot order, ordinals and the overflow rule stay unchanged.

`H` is the multiplexer's layout height (`layout.area.height` on herdr). It is the size every pane takes once its tab renders, even while a background herdr tab keeps the PTY size it was last rendered at. Delivery reads the live PTY size (§3), so a stale background PTY never affects admission correctness.

The planner only reports a resize and never re-plans. Re-planning after a shrink could only move live members into new windows, which breaks the no-migration guarantee and hides members until a later grow moves them back. Delivery stays correct at any size (§3–§8), so reporting through `observation.geometry`, `doctor` and the runtime diagnostic is enough. Later creations read the live height.

### 11. Schema

`SCHEMA_SQL` in `src/db.rs` changes in place:

```sql
CREATE TABLE messages (
    ...
    notified_at TEXT,
    preview_typings INTEGER NOT NULL DEFAULT 0 CHECK(preview_typings IN (0, 1, 2)),
    preview_parked_until TEXT
);

CREATE TABLE member_input_operations (
    ...
    stop_cause TEXT CHECK(stop_cause IS NULL
        OR (stop_cause IN ('interaction_open','interaction_candidate','literal_mismatch',
                          'composer_clipped','composer_unreadable',
                          'submit_unconfirmed','deadline_expired','short_write','write_failed',
                          'runner_stopped')
            AND state IN ('uncertain','settled'))),
    ...
);
```

`StopCause` gains `ComposerClipped => "composer_clipped"` and `ComposerUnreadable => "composer_unreadable"` (`src/broker/runtime/mod.rs:563`). `GateReason` gains `Geometry => "geometry"`. `DirectorHold` gains `ComposerTooSmall` and `ComposerGeometryUnavailable`. The defer and queue reason vocabulary gains `composer_too_small` and `composer_geometry_unavailable`.

### 12. Provenance and fixtures

Two new `composer_evidence` kinds:

| Kind | Additional required fields | Provenance check |
|---|---|---|
| `geometry` | `text_columns`, `visible_text_rows`, `text_rows` | Recompute `T` and `V₀` (`h = 0`) from the effective geometry of the screen's selected `composer.text` rule and the screen's `rows`/`cols`, and require equality. On a pre-submit screen, `text_rows` must equal the extracted text rows. When the literal fits it must lie in `R(expected_literal) − wrap_slack_rows ..= R(expected_literal)`, because `wrap_slack_rows` bounds a row the native may or may not waste; when the region is scrolled and not clipped it must equal `V₀`. A scrolled screen of a rule with `scroll_marker` must show the marker |
| `geometry_hold` | `expected_eligibility` (`hold:geometry` or `composer_empty`) | The screen classifies with that eligibility. For `composer_empty`, the rung selector returns `composer_too_small` for every message. The screen carries no `geometry` or `unsubmitted_literal` record |

An *unreadable partial* screen shows a typed literal whose region the selected rule cannot extract, such as a pi region without its bottom border or a Hermes classic region whose `❯ ` row scrolled away. Its `geometry` record also carries `rule_id`, the `composer.text` rule whose geometry it is fitted against, and `typed_literal`, the literal that was typed, and the screen carries no `unsubmitted_literal` record. Provenance requires `R(typed_literal) > V₀`, `text_rows` equal to `V₀`, and a comparison of `typed_literal` against the screen that extracts no text.

A *typeable* cell is a captured size and state at which the empty composer verifies and at least the `minimal` rung fits, below `min_rows`×`min_columns` included, because the runner delivers there. `min_rows` and `min_columns` remain the smallest size at which every state verifies. Geometry values are multiplexer-independent: at matching PTY sizes, every backend's herdr capture shows the same visible literal rows, first visible row, scroll marker and closing row as its tmux capture. The prompt plan cites, on each multiplexer, the fixtures that multiplexer's row of the size axis names. A missing citation fails with `composer geometry for <backend> requires measured fixtures on <multiplexer>`. A herdr fixture records its PTY size read with `stty`, so a herdr cell of a primary-screen native (opencode, Hermes classic) is one column narrower than its axis size, such as 9x79 for 9×80, and that one-column narrowing satisfies the axis. The matrix exempts exactly one composer and state: the Hermes TUI composer working. The initial capture run ended with a gateway error. The later Nous run verified live delivery during work, and the required geometry fixture series remains unmeasured. Its `transient_rows` stays at 4.

| Axis | Values |
|---|---|
| Backend | Every bundled profile |
| Multiplexer | `tmux`, `herdr` |
| Size on tmux | 9×80, 9×120, 12×80, 12×120, `min_rows`×`min_columns`, 5×40 and 2×10 |
| Size on herdr | 9×80, 9×120, 12×80, 12×120 and the smallest size a rendered client split reaches |
| State | idle, working (spinner and tip rows visible) |
| Evidence at a typeable size, per state | An `empty` screen with `geometry`. Pre-submit screens with `geometry` and `unsubmitted_literal`: `one`, a one-row literal; `v0`, a wrapped literal of exactly `V₀` rows (waived when `V₀` is 1); and `partial`, a literal of `V₀ + 1` or more rows, clipped or scrolled. A `partial` screen whose region compares as unreadable carries `geometry` without `unsubmitted_literal` |
| Evidence at any other size, per state | One empty `hold` screen with `geometry_hold` and no literal screens |

The matrix carries no rung literals: rung selection is covered by the Step 5 and Step 10 tests, and a rung literal renders like any literal of its row count. The matrix applies to the prompt plan, because exec types into the same composer and its `! ` prefix is already part of `R`; the exec plan carries no matrix.

Captures use the installed native versions with the CLAUDE.md development models. Names follow `<backend>-<version>-composer-geometry-[<composer>-]<state>-<kind>-<mux>-<rows>x<cols>-<n>`, where `<composer>` is empty for the member composer and `tui` for the Hermes TUI, `<state>` is `idle` or `working` and `<kind>` is the evidence row's screen name. Masks and privacy review follow the existing fixture rules.

### 13. Error and diagnostic strings

| String | Where |
|---|---|
| `composer plan for <backend> requires composer geometry` | Catalog loading |
| `composer geometry of <backend> is invalid: <field>` | Catalog loading |
| `composer geometry for <backend> requires measured fixtures on <multiplexer>` | Provenance validation |
| `preview delivery: message <id> held: composer_too_small` | Member runner diagnostic, `preview_sweep` |
| `preview delivery: message <id> abandoned after pointer retry: <cause>` | Member runner diagnostic, `preview_sweep` |
| `director delivery: message <id> held: composer_too_small` | Director diagnostic |
| `director delivery: message <id> abandoned after pointer retry: <cause>` | Director diagnostic |
| `preview delivery: message <id> held: composer_geometry_unavailable` | Member runner diagnostic, `preview_sweep` |
| `director delivery: message <id> held: composer_geometry_unavailable` | Director diagnostic |
| `preview delivery: message <id> park ended: consumed` | Member runner diagnostic, `preview_sweep` |
| `director delivery: message <id> park ended: consumed` | Director diagnostic |
| `observation: pane <rows>x<columns> below the <backend> minimum <min_rows>x<min_columns>` | Runner diagnostic, `observation` |

`<field>` names the offending field's path inside the geometry object, such as `text_window.steps` or `reserved_rows`. The existing `director delivery: message <id> uncertain: <cause>` row gains the causes `composer_clipped` and `composer_unreadable`.

### 14. Documentation targets

| Target | Change |
|---|---|
| `SPEC.md` §6.5 Push notifications, Held delivery, Accepted trade-offs | Rung ladder and forms; geometry hold; replace the per-backend composer-window bullets with the geometry contract and the remaining races |
| `SPEC.md` § Fleet window layout, Placement policy | Slot budget formula replacing the 15-row constant and capacity table; resize is reported, never re-planned |
| `SPEC.md` §6.3 `doctor`, Runtime presentation, `member prompt`/`member exec`, Error Messages | `member.<id>.geometry` and `fleet.<fleet_id>.director.geometry` rows; `observation.geometry`; `hold:geometry`; `composer_too_small` and `composer_geometry_unavailable`; new error strings |
| `SPEC.md` §6.9 Bundled profiles, Composer-text extraction, Director delivery capability and sweep, Measured submission plans, Delivery method, Pending-preview sweep, Input delivery result/stalled events, Stop cause | Geometry field and the `geometry`/`geometry_hold` evidence kinds; `CompareOutcome` classes; fit-based selection; durable late submission; typing count; stall suspension; Director `composer_too_small` hold; new stop causes |
| `SPEC.md` §8 | DDL from §11 |
| `docs/` | Every page that states preview forms, column capacity or per-backend composer windows: rung behavior, resize behavior and the doctor row in user terms |
| `assets/skills/cafleet/reference/cli.md` | Push notifications, Director previews, Wrapped delivery methods, Runner queue, doctor and member show facts listed above |
| `assets/skills/cafleet/roles/member.md`, `roles/director.md`, `SKILL.md` | Every `[cafleet msg ID…]` form, including bare `[cafleet msg ID]`, is a wake that calls for one `message poll` |
| `assets/skills/cafleet/reference/coding-agents.md` | Role eligibility Basis cells that cite a Director pane width for the plain limit |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Reproduce and measure (gating)

- [x] Reproduce "retype while a copy sits in the composer" and "`composer_empty` during the stall" with five Claude members stacked in one herdr column (9 rows each), capturing each state with `member capture` and runner health. <!-- completed: 2026-11-27T13:39 -->
- [x] Record the root cause of both under Background § Unexplained behaviors. If an empty-composer rule matches a screen holding a literal, correct that rule and add the capture as a negative fixture. <!-- completed: 2026-11-27T13:39 -->
- [x] Measure each profile's geometry fields (§2) at the installed native versions on tmux and herdr, including transient chrome while working, and record the values in §2. <!-- completed: 2026-11-27T13:39 -->
- [x] Derive each profile's `B_full`, full size and compact size with §3 from the measured fields and plain limits, and record them in §2 § Rung sizes. <!-- completed: 2026-11-27T13:39 -->
- [x] Measure herdr layout width against the PTY width for Director captures, and settle the herdr capture geometry (§3). <!-- completed: 2026-11-27T13:39 -->
- [x] For each finding that contradicts this specification, record a programmer marker at the contradicted section under the coordination protocol and send `blocked (<pointer>)` before Step 2. <!-- completed: 2026-11-27T13:39 -->

### Step 2: Documentation

- [x] Update `SPEC.md` sections listed in §14, preserving section numbering. <!-- completed: 2026-11-27T13:54 -->
- [x] Update every `docs/` page that states preview forms, column capacity or per-backend composer windows. <!-- completed: 2026-11-27T13:54 -->
- [x] Update `assets/skills/cafleet/reference/cli.md` facts listed in §14. <!-- completed: 2026-11-27T13:54 -->
- [x] Update member and Director roles and the cafleet `SKILL.md` for every rung form as a wake. <!-- completed: 2026-11-27T13:54 -->
- [x] Update `coding-agents.md` Role eligibility Basis cells that cite a Director pane width. <!-- completed: 2026-11-27T13:54 -->
- [x] Run `mise //docs:build` and inspect any changed table cell holding a literal pipe in the rendered HTML. <!-- completed: 2026-11-27T13:54 -->

### Step 3: Profile geometry in the catalog

- [x] Add `ComposerGeometry` parsing and validation for `composer.geometry`, per-rule `text_geometry` (every `text_window` and `reserved_rows` form) and per-rule `scroll_marker` in `src/coding_agent/catalog.rs`, with the two catalog-loading errors. <!-- completed: 2026-11-27T14:14 -->
- [x] Add the `geometry` and `geometry_hold` `composer_evidence` kinds with their parsing and structural provenance checks (§12), the production accessor `ProfileCatalog::composer_geometry(backend, rule_id)` and the selected-rule id on composer text. <!-- completed: 2026-11-27T14:14 -->
- [x] Add the measured `composer.geometry` to every bundled profile. <!-- completed: 2026-11-27T14:14 -->
- [x] Table-driven tests: missing and invalid geometry, including unordered or empty step lists and an `uncapped_from` at or below the last step; a per-rule `text_geometry` overriding fit fields, with a minimum in `text_geometry` rejected; provenance accepts the bundled catalog and rejects a mismatched `visible_text_rows`. <!-- completed: 2026-11-27T14:14 -->

### Step 4: Fit computation

- [x] Add `unicode-width` as a direct dependency. <!-- completed: 2026-11-27T14:14 -->
- [x] Add `src/coding_agent/geometry.rs` with `visible_text_rows` (each `text_window` form and `reserved_rows` at the live size), `fit_rows` (char and word wrap, `first_row_margin_columns`, wide scalars, `wrap_slack_rows`, collapsed paste) and headroom. <!-- completed: 2026-11-27T14:14 -->
- [x] Add `Detector::composer_geometry` returning `{rows, cols, geometry}` from the selected `composer.text` rule, or `None` when no rule or several rules match (§3). <!-- completed: 2026-11-27T14:14 -->
- [x] Add the `geometry` evidence's `V₀` provenance check and the one-rule selection check on every verified-empty fixture (§3, §12). <!-- completed: 2026-11-27T14:14 -->
- [x] Table-driven tests over generated screens at every §12 size, including `T < 1`, a size below the first step of a step list, wide scalars at the row edge and words longer than their row. <!-- completed: 2026-11-27T14:14 -->

### Step 5: Preview ladder

- [x] Add `compact_preview_line`, `cue_preview_line` and `minimal_preview_line` in `src/runtime/preview.rs`, cutting the excerpt in stored text before folding (§4). <!-- completed: 2026-11-27T14:28 -->
- [x] Add a rung selector that takes the message, plain limit, live geometry, headroom rule and `preview_typings`, and returns a rung, `composer_too_small` or `composer_geometry_unavailable`. <!-- completed: 2026-11-27T14:28 -->
- [x] Use the selector in `select_delivery` for member previews, and rerun it on zero-byte reselection. <!-- completed: 2026-11-27T14:28 -->
- [x] Add the `geometry_hold` evidence's rung-selector provenance check (§12). <!-- completed: 2026-11-27T14:28 -->
- [x] Table-driven tests: each rung chosen at its boundary geometry; a second typing skips full and compact; a `\r\n` and a control character on each side of the excerpt cut give the stored-count `N`. <!-- completed: 2026-11-27T14:28 -->

### Step 6: Verification classes and stop causes

- [x] Replace `ComposerText::matches` with `compare` returning `CompareOutcome`, and update every caller. <!-- completed: 2026-11-27T14:52 -->
- [x] Strip `scroll_marker` matches during extraction and classify a marked region as scrolled (§6). Declare the measured markers on the claude, codex and pi composer-text rules, widen their `start_pattern` and region matchers to open at a marker row. <!-- completed: 2026-11-27T14:52 -->
- [x] Correct `opencode-composer-text` so that it does not require the blank row above `┃`. <!-- completed: 2026-11-27T14:52 -->
- [x] Add `composer_clipped` and `composer_unreadable` to `StopCause`, `SCHEMA_SQL` and the stored-enum mapping. <!-- completed: 2026-11-27T14:52 -->
- [x] Map window-close outcomes in `arm_pending` and in the Director attempt to the §6 causes. <!-- completed: 2026-11-27T14:52 -->
- [x] Tests: clipped-only, scrolled, clipped-and-scrolled, contradicting and unextractable screens for each backend produce the §6 causes, using the marker or `V₀` from the pre-submit screen, and never write a submit key. <!-- completed: 2026-11-27T14:52 -->

### Step 7: Geometry hold

- [x] Add `GateReason::Geometry` (`hold:geometry`) to observation eligibility for unmatched below-minimum screens. <!-- completed: 2026-11-27T15:04 -->
- [x] Hold member previews under `composer_too_small` or `composer_geometry_unavailable` with the §3/§5 diagnostics, and queue prompt/exec under the matching `defer_reason`. <!-- completed: 2026-11-27T15:04 -->
- [x] Add `DirectorHold::ComposerTooSmall` and `DirectorHold::ComposerGeometryUnavailable` to the Director attempt, replacing `composer_busy` only for below-minimum captures where no Director composer or interaction rule matches (§5). <!-- completed: 2026-11-27T15:04 -->
- [x] Suspend the preview and queued-head stall waits while the pane is `below_minimum` or the hold is `composer_too_small`, whatever the eligibility. <!-- completed: 2026-11-27T15:04 -->
- [x] Scenario-harness tests: a hold writes zero bytes and publishes no stalled event; a draft in a below-minimum pane starts its stall wait only after a grow; a later grow delivers without Director action. <!-- completed: 2026-11-27T15:04 -->

### Step 8: Late submission

- [x] Add `messages.preview_parked_until` to `SCHEMA_SQL`, and make every member and Director sweep treat a head parked into the future as held. <!-- completed: 2026-11-27T15:21 -->
- [x] Add the parked state to the member sweep attempt: set `preview_parked_until` under the lock, release the lock and re-observe every sweep interval under `TestClock`. <!-- completed: 2026-11-27T15:21 -->
- [x] Add the same parked state to the Director sweep thread. <!-- completed: 2026-11-27T15:21 -->
- [x] Scenario-harness tests for each §6 transition: late `Complete`, `Contradicts`, emptied composer, consumption by poll, window expiry, shutdown, and expiry after parker loss handled by another runner. Include a second runner that never types a parked Director head. <!-- completed: 2026-11-27T15:21 -->

### Step 9: Single pointer retry

- [x] Add `messages.preview_typings` to `SCHEMA_SQL`. Count each typing at the §7 count points, immediately before the first literal byte, refusing at 2, while notified, or while parked. Stamp `notified_at` per §7 in member and Director uncertain handling. <!-- completed: 2026-11-27T15:30 -->
- [x] Stamp an exhausted head, and record the abandonment diagnostics including `typings_exhausted`. <!-- completed: 2026-11-27T15:30 -->
- [x] Tests: at most two typings per message across two runners delivering to one Director; zero-byte Director ticks and member reservations that return to `ready` leave the count unchanged; an exhausted head is stamped and the next message delivered; newer messages are delivered by the pointer's poll. <!-- completed: 2026-11-27T15:30 -->

### Step 10: Director sweep fit

- [x] Use the rung selector in the Director attempt with the capture geometry (§3), deriving herdr `cols` from the rect width less `HERDR_PANE_CHROME_COLUMNS` and the widest capture row. <!-- completed: 2026-11-27T15:36 -->
- [x] Tests: Director captures at every §12 size choose the expected rung or hold, including herdr captures whose PTY is wider and narrower than the rect. <!-- completed: 2026-11-27T15:36 -->

### Step 11: Reporting and placement

- [x] Add `geometry` to `ObservationHealth`, its validation, `member show` JSON and text, and the `observation` runtime diagnostic. <!-- completed: 2026-11-27T23:32 -->
- [x] Add the `doctor` `member.<id>.geometry` and `fleet.<fleet_id>.director.geometry` rows (§9), the Director row deriving its geometry as the Director sweep does. <!-- completed: 2026-11-27T23:32 -->
- [x] Replace `MIN_PANE_HEIGHT` with the per-backend, per-multiplexer slot budget in `src/multiplexer/layout.rs` and pass column backends and the multiplexer to `choose_slot`. <!-- completed: 2026-11-27T23:32 -->
- [x] Tests: health validation, doctor rows for each situation, mixed-backend column admission on tmux and herdr and the overflow minimum. <!-- completed: 2026-11-27T23:32 -->

### Step 12: Native fixtures

- [x] Capture the §12 matrix for every bundled profile on tmux and herdr: `empty`, `one`, `v0` and `partial` at each typeable cell and one `hold` screen at every other cell, in idle and working state. <!-- completed: 2026-11-28T04:44 -->
- [x] Capture and cite the native fixtures for the Step 6 rule changes: a scrolled screen per marked rule (claude, codex and pi) and opencode short-pane screens at 9x80 and 12x80 covering the probe-event-5 screen. <!-- completed: 2026-11-28T04:44 -->
- [x] Capture working-state fixtures for opencode, pi and hermes (both rules) with a probe whose preset allows the working command (answering pi's member gate) and, for Hermes, working provider credentials. Replace the provisional `transient_rows` with the measured values, re-derive every provisional rung size from them with §3, and record a Hermes working state that still cannot be produced as a known gap in the final report. <!-- completed: 2026-11-28T04:44 -->
- [x] Add the captures to each profile's `screens` with the evidence §12 assigns to their size (`geometry` and `unsubmitted_literal` at typeable sizes, `geometry_hold` otherwise), and cite them from the plans. <!-- completed: 2026-11-28T04:44 -->
- [x] Add the per-multiplexer fixture-matrix check over the prompt plan's citations, `composer geometry for <backend> requires measured fixtures on <multiplexer>` (§12). <!-- completed: 2026-11-28T04:44 -->
- [x] Confirm provenance validation passes with the captured fixtures. <!-- completed: 2026-11-28T04:44 -->

### Step 13: Validation

- [x] Run `mise //:test`. <!-- completed: 2026-11-28T04:51 -->
- [x] Run `mise //:lint`. <!-- completed: 2026-11-28T04:51 -->
- [x] Run `mise //docs:build`. <!-- completed: 2026-11-28T04:51 -->
- [x] Build the CLI and repeat the issue #660 reproduction with it, confirming every Success Criterion that a live fleet observes. <!-- completed: 2026-11-28T08:03 -->
- [x] Re-run the Director delivery probe on tmux and herdr for each Director-eligible backend and update `SPEC.md` § Role eligibility dates. <!-- completed: 2026-11-28T07:38 -->

The live acceptance used workspace release builds and fresh SQLite databases in isolated tmux servers and herdr sessions. The installed CLI remained on its original schema.

#### Resumption evidence

The resumed run used tmux 3.8, herdr 0.9.5 and isolated databases under `/tmp/cf248r` with a workspace release build. `results.jsonl` in that directory records message IDs, measured geometry, delivery stamps, typing counts and diagnostics. Screen captures and SQLite receipts provide the matching evidence.

`mise //:test` finished with 1,583 passed and 11 ignored. `mise //:lint`, `mise //docs:build`, `mise //:build` and `git diff --check` passed on the resumed change.

| Area | Observation |
|---|---|
| Wrapped literal | The inherited databases contained `literal_mismatch` for Codex at 9×80 and for Claude and pi at 12×80. These bodies ended with a space that the pre-submit renderer hid at a wrap along with the separator. The comparison now accepts spaces hidden at a word-wrap boundary and preserves visible spaces within a row. |
| Custom status rows | Claude Code 2.1.327 parked a 337-character preview at 24×80 with either 3 or 8 configured status rows. The adjusted fit sent one compact preview for 3 rows and one cue for 8 rows, both with a submission stamp and without parking. At 12×80 the native rendered two of the configured rows, and at 9×80 it hid the status rows. The 90-character probes submitted at both sizes. |
| Hold and grow | A Claude member at 2×10 on tmux and 4×40 on herdr held a preview under `composer_too_small` with zero typings, then submitted it after a grow. `doctor` showed member and Director geometry, and `member show` cleared `below_minimum`. A Claude prompt and exec each queued at 9×80 and appeared as submitted in their receipts after growing to 28×120. |
| Verified late submission | The tmux runner parked a preview after the pane shrank between its first typing and the submit key. Growing the pane produced a submission stamp at a typing count of one. |
| Retry cap | Two deliberate tmux settle-window edits yielded two typings, an abandonment diagnostic and a message still available to poll. A separate storage test covers the Director's atomic second-outcome stamp and park clear. |
| Issue #660 replay | In `herdr-claude-660-queue-fixed.db`, five Claude members stayed in one column at 9–10×89 after a resize. Messages 1–5 were sent while idle. Messages 6–10 were accepted by the native queue while the members were running a foreground exec. All ten were submitted once and consumed, with no input delivery events. Before the queue confirmation fix, each working preview reported `submit_unconfirmed` and was typed twice after its native command ended. |
| Director busy burst | The tmux Claude Director kept a 65-second foreground command running. It received a preview while the command ran and polled all 12 messages after completion. The first two had one typing each and the rest were consumed before their previews were typed. |
| Zoomed Director capture | A 120-column PTY reported a 60-column herdr layout rectangle. Earlier replay held Claude and recorded `literal_mismatch` on Codex. Replaying at the observed row width allowed both to submit with one typing. Claude also polled its idle preview, held an operator draft for ten seconds and polled the next preview once the draft was cleared. |
| Idle member samples | Native herdr PTYs submitted 29 of 30 sampled short and long previews across the bundled member backends. One OpenCode sample had no submission stamp. The actual captured dimensions varied with pane borders and toolbars, so these samples cover selected points of the matrix. |
| Hermes TUI retry | Launching the native TUI with the configured Nous provider recovered from the OpenRouter gateway error without changing the default configuration. Idle, draft hold, full, compact, settle race and a 12-message busy burst passed on tmux and herdr. The probe records are in `/tmp/cf248-hermes/results.jsonl`. |
| Wrapped Claude queue | A working Director at 120 columns displayed a two-row queue echo immediately before the native queue notice. The runner now verifies the complete echo and still requires a fresh drain and absence of that echo before submit. The tmux replay consumed all 12 messages; the first was submitted once with zero diagnostics. |
| Codex input limit | The herdr probe rendered exactly 1024 UTF-8 bytes and clipped the final two ASCII characters when an earlier literal included a three-byte ellipsis. Director planning now applies the 1024-byte limit on both multiplexers. |
| Member matrix | All 160 cases passed across five backends and both multiplexers, including all 32 Claude cases on the margin-adjusted build. At 12×120, Claude submitted its 430- and 4000-character bodies as compact previews with one typing, idle and during a foreground command. The evidence and replay notes are in `/tmp/cf248-mx-evidence.md`. |
| Director probe rechecks | Claude, Codex, OpenCode and pi passed the standard probe on both multiplexers. The Codex herdr replay at 37×92 passed the 4000-character compact probe with one typing. At 21×80, a full tmux preview sometimes scrolled and parked; growing the pane submitted it at the original typing count. The Claude herdr 24×80 and 18×120 full-threshold probes submitted successfully but used shorter forms in zoomed panes with narrower layout rectangles. Raw results are under `/tmp/cf248-dir` and `/tmp/cf248-rest`. |
| Hermes approval | Hermes TUI renders the standard composer prefix for a profile named `custom`. A temporary profile with that name and manual approvals held message 1 at zero typings while the dialog was open, then submitted and polled it with one typing after denial, on tmux and herdr. Both disposable markers remained present. |
| Exact Director thresholds | Claude passed all four sizes on tmux and on unzoomed herdr. Codex passed the 18×120 full and both compact sizes on unzoomed herdr. Build `9f30f4a0` selected compact at 21×80 and full at 24×80, with one typing and native poll completion on both multiplexers. A clean tmux replay also passed full at 18×120. Hermes TUI passed the two full and two compact sizes on both multiplexers, messages 2–5 in each manual interaction fixture. |
| tmux literal chunks | tmux 3.8 consumed the semicolon at scalar 256 of a compact preview on OpenCode at 5×120. Passing a backslash immediately before the terminal semicolon preserves it, including when the input already contains consecutive semicolons or a backslash. The tmux transport applies this escaping to each literal chunk. |
| Director render timing | The first sampled frame containing the exact busy Codex draft on tmux arrived 32 ms after the old verification window closed. The Director now observes for up to 2 s after settle, bounded by the same attempt deadline and the reserved post-submit budget. |
| Director first render | On build `87dcaf43`, Codex on tmux at 38×120 retained its empty placeholder across two captures after typing. The 4,000-character body yielded an 848-character excerpt and a 3,152-character remainder. The preview was typed once, stamped, and consumed by the native poll with zero diagnostics. The Director late-submission regression additionally verifies the same transition after the preview has already parked. |
| Final working thresholds | Claude and Codex passed all 16 Director threshold replays across tmux and herdr, including the two 18×120 Codex full replays on `cc884c18`. OpenCode and pi passed all 32 idle and working threshold cases, including the OpenCode semicolon replays at 5×120 on `783c5beb`. Hermes TUI stamped the exact thresholds while its foreground command ran on both multiplexers. An existing native queue can consume additional pane rows, so the focused retries started with the native queue empty. The receipts are indexed in `/tmp/cf248-dir-evidence.md`, `/tmp/cf248-rest-thresholds.md`, and `/tmp/cf248-hermes/results.jsonl`. |
| Standalone exec and automated checks | A standalone Claude exec at 9×80 on each multiplexer queued under `composer_too_small` and settled after growing to 28×120, operation 9 in each v4 matrix database. The final full suite passed 1,593 tests with 11 ignored. All nine Director late-submission tests passed, including single-write delayed rendering and deliberate draft clearance. Clippy and the release and documentation builds passed. |

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-27 | Initial draft |
| 2026-11-27 | Review round 1: rung-based and rung-size criteria; profile-level minimums; `text_top` from the selected `composer.text` rule; stored-text excerpt cut; stall suspension by pane size; Director `composer_too_small` hold and doctor row; `V₀` for `Partial`; durable park (`preview_parked_until`) and typing count (`preview_typings`); typeable-size fixture evidence |
| 2026-11-27 | Review round 2: `B_full` from the plain limits; Director `composer_too_small` limited to unmatched below-minimum screens; typings counted immediately before the first literal byte; exhausted heads stamped |
| 2026-11-27 | Step 1 tmux measurements: every composer fitted as bottom-bounded (`anchor`, `rows_below_text` and `text_top` removed); `text_window` replaces `max_text_rows` and `pane_fraction`; per-size `reserved_rows`; `first_row_margin_columns`; per-rule `scroll_marker`; opencode composer-text rule correction; measured values and `B_full` recorded |
| 2026-11-27 | Step 1 close: confirmed root cause; herdr capture geometry from `viewport_rows` and `HERDR_PANE_CHROME_COLUMNS` replacing the width slack; per-multiplexer slot budget; per-multiplexer fixture sizes; measured `transient_rows`, minimums and rung sizes, with provisional values settled by a new Step 12 working-state task |
| 2026-11-28 | Step 12 scope: the fixture matrix keeps every multiplexer, size and state and drops the per-rung screens; each typeable cell requires `empty`, `one`, `v0` and `partial`, every other cell one `hold` screen; the matrix applies to the prompt plan only; capture names carry the composer, state and kind |
| 2026-11-28 | Step 12 measurements: pi `wrap_slack_rows` 1 with the `R − wrap_slack_rows ..= R` provenance range and `reserved_rows` 6 from 7 rows; Hermes `min_columns` 64, classic `reserved_rows` `[[5, 2]]`, `fit_wrap` `char` and a 4-cell first row; new fit fields `carried_space` and `fit_wrap`; measured `transient_rows` 0 for opencode, pi and Hermes classic; per-cell typeability; unreadable partial evidence; herdr scrollbar column; the Hermes TUI working gap |
