# Geometry-robust preview and composer delivery

**Status**: Approved
**Progress**: 0/55 tasks complete
**Last Updated**: 2026-11-27

## Overview

Previews, `member prompt`/`member exec` input and Director previews fail as `literal_mismatch` when the live pane is too short or narrow for the typed literal, or when a composer scrolls internally, and an uncertain preview is then retyped repeatedly (issue #660). This design picks the literal from the live pane geometry, so pre-submit verification always sees the whole literal. Delivery stays correct at every pane size and through mid-session resizes. When nothing fits, delivery holds with zero bytes and an explicit diagnostic, and after a failure each message gets exactly one more delivery, as one of the short poll rungs (pointer, cue or minimal).

## Success Criteria

- [ ] Every bundled profile (each file under `assets/profiles/`) declares a measured `composer.geometry`. Catalog loading rejects a profile whose executable composer plan lacks it.
- [ ] On every bundled backend, tmux and herdr, a 9-row and a 12-row pane at 80 and 120 columns, idle or working, deliver a preview of 90–430 body characters whenever a rung fits. It arrives as the rung §4 selects for the live geometry, with stage `submitted`, never as `literal_mismatch`.
- [ ] At and above the per-profile full and compact sizes that Step 1 records in §2 § Rung sizes, on tmux and herdr, idle or working, a preview whose body is `B_full(profile)` characters arrives as the `full` rung, to a member and to the Director. At and above the compact size, a preview whose body exceeds `B_full` but is at most 4000 characters arrives as the `compact` rung.
- [ ] Below each profile's `min_rows`/`min_columns`, down to 2 rows × 10 columns, no delivery writes a byte into a composer where the chosen rung does not fit. The preview holds with `composer_too_small`, publishes no `input_delivery_stalled` event, and is delivered automatically once the pane grows enough for a rung.
- [ ] A literal clipped or scrolled at pre-submit verification records `composer_clipped` or `composer_unreadable`, never `literal_mismatch`. Only visible text that contradicts the literal records `literal_mismatch`.
- [ ] A clipped or unreadable preview, whether to a member or to the Director, is submitted late without retyping when its full literal becomes visible within the late-submission window (120 s).
- [ ] A message is typed at most twice: once in any rung and, after an uncertain outcome, once as the pointer, cue or minimal rung. A second uncertain outcome stamps `notified_at` and records a diagnostic. This holds for member and Director delivery.
- [ ] `member prompt`/`member exec` whose literal does not fit the live composer queue with zero bytes under `composer_too_small` and drain automatically when it fits.
- [ ] `member show` reports live `observation.geometry`. Its `below_minimum` clears without operator action when the pane grows back. `doctor` prints a `member.<id>.geometry` row per probed member and a `fleet.<fleet_id>.director.geometry` row per fleet whose Director pane receives Director delivery. A below-minimum Director pane holds under `composer_too_small` immediately, not after 3 minutes as `composer_busy`.
- [ ] `member create` admits a member to a column only when every pane in it keeps at least its backend's slot budget. A resize after creation is reported and never re-plans the layout.
- [ ] Step 1 reproduces both unexplained behaviors in issue #660 with recorded fixtures and records their root cause in this document.
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

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

Working hypothesis: a Director `member press Enter` on a busy Claude member moves the literal to the native `Queued · ctrl+enter to send now` area and empties the composer. The still-unnotified preview is then retyped into the empty composer, and `composer_empty` describes that state. The alternative is that an empty-composer rule (`claude-idle-composer` or `claude-suggestion-composer`) matches a screen that holds a literal. Step 1 decides between them with captures.

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

```json
{
  "anchor": "composer_row",
  "margin_columns": 4,
  "max_text_rows": 7,
  "pane_fraction": null,
  "rows_below_text": 2,
  "reserved_rows": null,
  "transient_rows": 3,
  "wrap_slack_rows": 0,
  "collapsed_paste": null,
  "min_rows": 12,
  "min_columns": 40
}
```

| Field | Type | Meaning |
|---|---|---|
| `anchor` | `"composer_row"` or `"screen_bottom"` | `composer_row`: the composer starts at its live row and grows downward, so rows below it bound the text (Claude). `screen_bottom`: the composer is pinned to the bottom and grows upward (codex, opencode, pi, Hermes TUI) |
| `margin_columns` | integer ≥ 0 | Cells per composer row unavailable to text: prefix, margin and borders. Text columns `T = cols − margin_columns` |
| `max_text_rows` | integer ≥ 1 or `null` | The composer's internal window cap; `null` when it has none |
| `pane_fraction` | number in (0, 1] or `null` | A window of `floor(pane_fraction × rows)` rows, as pi draws; `null` when absent |
| `rows_below_text` | integer ≥ 0; required for `composer_row`, `null` otherwise | Rows the native draws below the last text row (bottom border, status lines), all of which must stay visible for extraction |
| `reserved_rows` | integer ≥ 0; required for `screen_bottom`, `null` otherwise | Non-text rows the native keeps on screen (header, frame, footer) at the smallest captured geometry |
| `transient_rows` | integer ≥ 0 | The largest number of chrome rows (spinner, `Tip:`, update notice, queued-input rows) that can appear while a literal is being typed and displace the composer |
| `wrap_slack_rows` | 0 or 1 | Rows added to the greedy word-wrap estimate when the native wrap algorithm can waste a row compared with greedy wrapping |
| `collapsed_paste` | `{"min_scalars": N, "rows": 1}` or `null` | For a `bracketed_paste` plan: a bracketed literal of at least `N` scalars renders as a one-row placeholder |
| `min_rows`, `min_columns` | integers ≥ 1; profile level only | The smallest captured geometry at which every composer's empty composer verifies and the minimal rung fits on one row. Used for placement, `below_minimum`, `hold:geometry` and the Director `composer_too_small` hold; correctness never depends on them |

All values are measured from native captures (Step 1, Step 12) and validated by provenance (§12). The expected starting values, which Step 1 confirms or corrects, come from SPEC §6.5 Accepted trade-offs:

| Profile | `anchor` | `margin_columns` | `max_text_rows` | `pane_fraction` |
|---|---|---|---|---|
| `claude` | `composer_row` | 4 | 7 | `null` |
| `codex` | `screen_bottom` | measured | measured | `null` |
| `opencode` | `screen_bottom` | 4 | 6 | `null` |
| `pi` | `screen_bottom` | 1 | `null` | 0.3 |
| `hermes` TUI rule | `screen_bottom` | 6 | `null` | `null` |
| `hermes` classic rule | measured | measured | 8 | `null` |

Catalog loading rejects an executable composer plan without `composer.geometry` with `composer plan for <backend> requires composer geometry`. It rejects an invalid field with `composer geometry of <backend> is invalid: <field>`.

#### Rung sizes

Step 1 derives these from the measured fields with §3, and Success Criterion 3 checks them live.

- **Full-size body.** `B_full(profile)` is the longest ASCII body whose `full` literal stays within both the profile's prompt `plain_literal_limit` and its `director_delivery.plain_limit`. The reference envelope is a three-digit message ID and sender ID with a canonical 32-scalar timestamp.
- **Full size.** For each profile and each of 80 and 120 columns, the smallest pane height at or above `min_rows` at which a `B_full` body fits the `full` rung with headroom.
- **Compact size.** The smallest such height at which the `compact` rung fits with an excerpt of at least 80 scalars.

| Profile | `B_full` | Full size at 80 / 120 columns | Compact size at 80 / 120 columns |
|---|---|---|---|
| `claude` | Step 1 | Step 1 | Step 1 |
| `codex` | Step 1 | Step 1 | Step 1 |
| `opencode` | Step 1 | Step 1 | Step 1 |
| `pi` | Step 1 | Step 1 | Step 1 |
| `hermes` | Step 1 | Step 1 | Step 1 |

### 3. Fit computation

Inputs are the live snapshot's `rows` and `cols`, and, for `composer_row`, the first composer text row `text_top`.

`text_top` is read from the verified empty screen: the region top of the one `composer.text` rule that `Detector::composer_text` selects there, plus one when that rule is framed (its `text_framed`, else `composer.text.framed`). The selected rule's `text_geometry` supplies the fit fields. A `screen_bottom` computation uses no `text_top`, but it still takes its fit fields from the selected rule.

When no `composer.text` rule, or more than one, matches a screen that observation verifies empty, the profile does not describe that screen. Delivery then holds with zero bytes under `composer_geometry_unavailable`. That reason is not a geometry hold: it runs the ordinary stall wait and records `preview delivery: message <id> held: composer_geometry_unavailable` (or the `director delivery:` form), because only a profile correction resolves it. Provenance requires every verified-empty fixture to select exactly one `composer.text` rule, so the bundled profiles exhibit this only on uncaptured screens.

The member runner uses its PTY screen model (`transport.snapshot()`). The Director sweep uses the capture. A herdr Director capture reports layout width, so `cols` is reduced by `HERDR_CAPTURE_WIDTH_SLACK = 4`. Step 1 measures the real difference and raises the constant if a capture exceeds it.

- Text columns: `T = cols − margin_columns`. `T < 1` means nothing fits.
- Visible text rows, with headroom `h`:
  - `composer_row`: `V = min(max_text_rows?, rows − text_top − rows_below_text − h)`.
  - `screen_bottom`: `V = min(max_text_rows?, floor(pane_fraction × rows)?, rows − reserved_rows − h)`, where `?` terms apply only when declared.
- Required rows `R(literal)` for the rendered literal, including the operation prefix:
  - A bracketed literal at or above `collapsed_paste.min_scalars` takes `collapsed_paste.rows`.
  - `char` wrap: fill cells greedily, where a scalar occupies its `unicode-width` cell count and a wide scalar that does not fit starts the next row.
  - `word` wrap: break greedily at the last space that fits, hard-break a word longer than `T`, and add `wrap_slack_rows`.
- Fit: `R ≤ V`.

Headroom `h` is `transient_rows` for the full and compact rungs and for prompt/exec, and 0 for the pointer, cue and minimal rungs. Chrome already on screen is accounted for by the live `text_top` and `rows`. Headroom guards only against chrome appearing between the literal and the submit key. On the short rungs, that race falls to late submission (§6) and the single retry (§7), so the shortest rungs stay available in small panes. A prompt or exec has no retry, so it waits for headroom.

`unicode-width` (0.2, already in `Cargo.lock` through `vt100`) becomes a direct dependency. The computation lives in a new `src/coding_agent/geometry.rs` (`ComposerGeometry`, `fit_rows`, `visible_text_rows`), and `Detector::composer_geometry(&ScreenSnapshot) -> Result<Option<LiveComposer>>` returns `{rows, cols, text_top, geometry}` from the selected `composer.text` rule, where `geometry` holds the effective fit fields, or `None` when the selection fails.

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
  - The capture (with `cols` reduced by `HERDR_CAPTURE_WIDTH_SLACK` on herdr) is below the Director profile's `min_rows`/`min_columns`, and no Director composer or interaction rule matches.
  - The composer is verified empty but no rung fits.

  The first condition replaces the `composer_busy` hold the attempt records today for an unmatched screen (`src/runtime/service.rs:1826`), but only for a capture below the minimum and with no Director composer or interaction rule matching. A matched busy or draft composer stays `composer_busy` at any size, and so does an unmatched screen at or above the minimum, mirroring §5 rows 2–4.
- **Retries.** A geometry hold never retypes, never stamps `notified_at` and needs no Director action. The sweep re-evaluates fit at each 1 s interval without backoff, because the PTY screen model already tracks size.
- **Diagnostics.** Member delivery records `preview delivery: message <id> held: composer_too_small` (source `preview_sweep`) once per continuous hold. Director delivery records `director delivery: message <id> held: composer_too_small` once per runner per continuous hold, immediately rather than after 3 minutes. A queued prompt/exec reports `defer_reason=composer_too_small` through `input show` and its queued CLI reply.
- **Interactions.** An interaction rendered below the minimum may go undetected, as a recorded layout limit (SPEC §6.7). Ordinary input still writes nothing, because the composer does not verify empty.

### 6. Pre-submit verification classes and late submission

`ComposerText::matches` becomes `ComposerText::compare(&self, chunks) -> CompareOutcome`, and `matches` is removed:

| `CompareOutcome` | Condition | Submit key | Stop cause at window close |
|---|---|---|---|
| `Complete` | Segments consume the entire literal (current `matches` semantics) | Written | — |
| `Partial` | The region is clipped or scrolled (below), and the segments consume a contiguous run of the literal: starting at its first scalar for a clipped-only region, at any offset for a scrolled region | Not written | `composer_clipped` |
| `Contradicts` | Any other extractable text, including text extending beyond the literal | Not written | `literal_mismatch` |
| (no text) | `Detector::composer_text` returns `None` | Not written | `composer_unreadable` |

A region is **clipped** when the screen's bottom edge closed it (`bottom_closed`). It is **scrolled** when its extracted text rows equal the native window cap `V₀`: §3's `V` with `h = 0`, recomputed from the pre-submit screen's live `rows`, `cols` and, for `composer_row`, that screen's `text_top`. A region that is both clipped and scrolled is classified as scrolled, so the contiguous run may start at any offset.

`Partial` only classifies the failure. It never authorizes a submit key, so tail verification stays unchanged. For pi, the composer matcher accepts a screen-bottom-closed region, so a clipped editor yields `Partial` rather than `None`.

**Late submission.** This applies only to member and Director previews. A pre-submit window that closes with `composer_clipped` or `composer_unreadable` parks the attempt instead of ending it:

1. In one write transaction, still holding the pane input lock, set `messages.preview_parked_until = now + LATE_SUBMISSION_WINDOW` (`LATE_SUBMISSION_WINDOW = 120 s`), but only while `notified_at` is null and the message is `input_required`. Then release the lock. The runner keeps the attempt's written literal, rung, `message_id` and stage `partial` in memory.
2. At every sweep interval (1 s), the parking runner re-reads the message and observes the composer (Director: one capture), then compares again:
   - The message is no longer `input_required`, because a `message poll` consumed it: end the park with no submit key and no uncertain outcome. Clear `preview_parked_until` and record `preview delivery: message <id> park ended: consumed` (or the `director delivery:` form). A literal left in the composer is then an ordinary non-empty composer.
   - `Complete`: acquire the lock nonblockingly, start a fresh 5 s attempt deadline, re-observe, require `Complete` again, write the submit key and run the 2 s post-submit verification. Confirmed submission stamps `notified_at`, clears `preview_parked_until` and ends the park.
   - `Contradicts`: end the park as uncertain `literal_mismatch`.
   - Verified empty composer (someone submitted or cleared it): end the park as uncertain with the park cause.
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
| Director sweep | Inside `attempt`, after the classifying capture verifies the composer empty and the rung is selected, and before `type_literal_until` |

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
| Between the literal and the submit key | The pre-submit comparison runs against the live screen. `Complete` submits; `Partial` or no text parks (§6, previews) or records `uncertain/composer_clipped` or `uncertain/composer_unreadable` (prompt/exec); `Contradicts` records `literal_mismatch` |
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

**`doctor`, Director pane.** When the `database` row is `ok`, one `fleet.<fleet_id>.director.geometry` row follows the member rows for each fleet that meets three conditions: its root Director is active with a concrete placement, the Director backend's bundled profile declares `director_delivery` listing the placement's multiplexer, and that profile declares `composer.geometry`. Rows appear in fleet-ID order. doctor takes one read-only capture of the Director pane (§6.5 Backend operations), applying `HERDR_CAPTURE_WIDTH_SLACK` on herdr, under the same one-second deadline as the member probes:

| Situation | Status | Detail |
|---|---|---|
| Capture failed | `info` | `director=<member_id> geometry=unavailable: <cause>` |
| Below the Director profile's `min_rows`/`min_columns` | `warn` | `director=<member_id> rows=<r> columns=<c> below the <backend> minimum <min_rows>x<min_columns>; Director previews shorten or hold until the pane grows` |
| Otherwise | `ok` | `director=<member_id> rows=<r> columns=<c>` |

The delivering members' `member show` diagnostics carry the Director hold itself (§5).

**Runtime diagnostic.** When `below_minimum` turns true, the runner records `observation: pane <rows>x<columns> below the <backend> minimum <min_rows>x<min_columns>` (source `observation`).

### 10. Layout planner

`MIN_PANE_HEIGHT` is replaced by a per-backend slot budget: `slot_rows(backend) = composer.geometry.min_rows + 1`, where the extra row is the pane separator or border. A backend with no profile geometry uses the documented default `DEFAULT_SLOT_ROWS = 15`. A column of window height `H` holding members of backends `B` admits backend `b` when both hold:

- `count(B) + 1 ≤ MAX_PANES_PER_COLUMN` (8);
- `floor(H / (count(B) + 1)) ≥ max(slot_rows(b), max over B of slot_rows)`.

An overflow column always admits its first member. `choose_slot` receives each column's member backends, from `member_placements.coding_agent`, and the creating backend. Slot order, ordinals and the overflow rule stay unchanged.

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
| `geometry` | `text_columns`, `visible_text_rows`, `text_rows` | Recompute `T` and `V₀` (`h = 0`) from the effective geometry of the screen's selected `composer.text` rule, the screen's `rows`/`cols` and that region's `text_top`, and require equality. On a pre-submit screen, `text_rows` must equal the extracted text rows. It must also equal `R(expected_literal)` when the literal fits, or `V₀` when the screen is scrolled |
| `geometry_hold` | `expected_eligibility` (`hold:geometry` or `composer_empty`) | The screen classifies with that eligibility. For `composer_empty`, the rung selector returns `composer_too_small` for every message. The screen carries no `geometry` or `unsubmitted_literal` record |

A *typeable size* is a captured size at which the empty composer verifies and at least the `minimal` rung fits. Each executable operation plan cites, on both multiplexers, the fixtures the matrix below names. A missing citation fails with `composer geometry for <backend> requires measured fixtures on <multiplexer>`.

| Axis | Values |
|---|---|
| Backend | Every bundled profile |
| Multiplexer | `tmux`, `herdr` |
| Size | 9×80, 9×120, 12×80, 12×120, `min_rows`×`min_columns`, 5×40 and 2×10 |
| State | idle, working (spinner and tip rows visible) |
| Evidence at a typeable size | An empty screen with `geometry`. Pre-submit screens with `geometry` and `unsubmitted_literal` for a one-row literal, a wrapped literal that fits, and each rung's longest form that fits at that size. A clipped or scrolled literal that does not fit, as the `Partial` case |
| Evidence at any other size | One empty-state screen with `geometry_hold` per state and no literal screens |

Captures use the installed native versions with the CLAUDE.md development models. Names follow `<backend>-<version>-composer-geometry-<case>-<mux>-<rows>x<cols>-<n>`. Masks and privacy review follow the existing fixture rules.

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

The existing `director delivery: message <id> uncertain: <cause>` row gains the causes `composer_clipped` and `composer_unreadable`.

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

- [ ] Reproduce "retype while a copy sits in the composer" and "`composer_empty` during the stall" with five Claude members stacked in one herdr column (9 rows each), capturing each state with `member capture` and runner health. <!-- completed: -->
- [ ] Record the root cause of both under Background § Unexplained behaviors. If an empty-composer rule matches a screen holding a literal, correct that rule and add the capture as a negative fixture. <!-- completed: -->
- [ ] Measure each profile's geometry fields (§2) at the installed native versions on tmux and herdr, including transient chrome while working, and record the values in §2. <!-- completed: -->
- [ ] Derive each profile's `B_full`, full size and compact size with §3 from the measured fields and plain limits, and record them in §2 § Rung sizes. <!-- completed: -->
- [ ] Measure herdr layout width against the terminal width for Director captures, and confirm or raise `HERDR_CAPTURE_WIDTH_SLACK`. <!-- completed: -->
- [ ] For each finding that contradicts this specification, record a programmer marker at the contradicted section under the coordination protocol and send `blocked (<pointer>)` before Step 2. <!-- completed: -->

### Step 2: Documentation

- [ ] Update `SPEC.md` sections listed in §14, preserving section numbering. <!-- completed: -->
- [ ] Update every `docs/` page that states preview forms, column capacity or per-backend composer windows. <!-- completed: -->
- [ ] Update `assets/skills/cafleet/reference/cli.md` facts listed in §14. <!-- completed: -->
- [ ] Update member and Director roles and the cafleet `SKILL.md` for every rung form as a wake. <!-- completed: -->
- [ ] Update `coding-agents.md` Role eligibility Basis cells that cite a Director pane width. <!-- completed: -->
- [ ] Run `mise //docs:build` and inspect any changed table cell holding a literal pipe in the rendered HTML. <!-- completed: -->

### Step 3: Profile geometry in the catalog

- [ ] Add `ComposerGeometry` parsing and validation for `composer.geometry` and per-rule `text_geometry` in `src/coding_agent/catalog.rs`, with the two catalog-loading errors. <!-- completed: -->
- [ ] Add the `geometry` and `geometry_hold` `composer_evidence` kinds and their provenance checks, including the one-rule selection on every verified-empty fixture (§3, §12). <!-- completed: -->
- [ ] Add the measured `composer.geometry` to every bundled profile. <!-- completed: -->
- [ ] Table-driven tests: missing and invalid geometry; a per-rule `text_geometry` overriding fit fields, with a minimum in `text_geometry` rejected; provenance accepts the bundled catalog and rejects a mismatched `visible_text_rows`. <!-- completed: -->

### Step 4: Fit computation

- [ ] Add `unicode-width` as a direct dependency. <!-- completed: -->
- [ ] Add `src/coding_agent/geometry.rs` with `visible_text_rows`, `fit_rows` (char and word wrap, wide scalars, `wrap_slack_rows`, collapsed paste) and headroom. <!-- completed: -->
- [ ] Add `Detector::composer_geometry` returning `{rows, cols, text_top, geometry}` from the selected `composer.text` rule, or `None` when no rule or several rules match (§3). <!-- completed: -->
- [ ] Table-driven tests over generated screens at every §12 size, including `T < 1`, wide scalars at the row edge and words longer than `T`. <!-- completed: -->

### Step 5: Preview ladder

- [ ] Add `compact_preview_line`, `cue_preview_line` and `minimal_preview_line` in `src/runtime/preview.rs`, cutting the excerpt in stored text before folding (§4). <!-- completed: -->
- [ ] Add a rung selector that takes the message, plain limit, live geometry, headroom rule and `preview_typings`, and returns a rung, `composer_too_small` or `composer_geometry_unavailable`. <!-- completed: -->
- [ ] Use the selector in `select_delivery` for member previews, and rerun it on zero-byte reselection. <!-- completed: -->
- [ ] Table-driven tests: each rung chosen at its boundary geometry; a second typing skips full and compact; a `\r\n` and a control character on each side of the excerpt cut give the stored-count `N`. <!-- completed: -->

### Step 6: Verification classes and stop causes

- [ ] Replace `ComposerText::matches` with `compare` returning `CompareOutcome`, and update every caller. <!-- completed: -->
- [ ] Let the pi composer matcher accept a screen-bottom-closed region and add a clipped pi fixture. <!-- completed: -->
- [ ] Add `composer_clipped` and `composer_unreadable` to `StopCause`, `SCHEMA_SQL` and the stored-enum mapping. <!-- completed: -->
- [ ] Map window-close outcomes in `arm_pending` and in the Director attempt to the §6 causes. <!-- completed: -->
- [ ] Tests: clipped-only, scrolled, clipped-and-scrolled, contradicting and unextractable screens for each backend produce the §6 causes, using `V₀` from the pre-submit screen, and never write a submit key. <!-- completed: -->

### Step 7: Geometry hold

- [ ] Add `GateReason::Geometry` (`hold:geometry`) to observation eligibility for unmatched below-minimum screens. <!-- completed: -->
- [ ] Hold member previews under `composer_too_small` or `composer_geometry_unavailable` with the §3/§5 diagnostics, and queue prompt/exec under the matching `defer_reason`. <!-- completed: -->
- [ ] Add `DirectorHold::ComposerTooSmall` and `DirectorHold::ComposerGeometryUnavailable` to the Director attempt, replacing `composer_busy` only for below-minimum captures where no Director composer or interaction rule matches (§5). <!-- completed: -->
- [ ] Suspend the preview and queued-head stall waits while the pane is `below_minimum` or the hold is `composer_too_small`, whatever the eligibility. <!-- completed: -->
- [ ] Scenario-harness tests: a hold writes zero bytes and publishes no stalled event; a draft in a below-minimum pane starts its stall wait only after a grow; a later grow delivers without Director action. <!-- completed: -->

### Step 8: Late submission

- [ ] Add `messages.preview_parked_until` to `SCHEMA_SQL`, and make every member and Director sweep treat a head parked into the future as held. <!-- completed: -->
- [ ] Add the parked state to the member sweep attempt: set `preview_parked_until` under the lock, release the lock and re-observe every sweep interval under `TestClock`. <!-- completed: -->
- [ ] Add the same parked state to the Director sweep thread. <!-- completed: -->
- [ ] Scenario-harness tests for each §6 transition: late `Complete`, `Contradicts`, emptied composer, consumption by poll, window expiry, shutdown, and expiry after parker loss handled by another runner. Include a second runner that never types a parked Director head. <!-- completed: -->

### Step 9: Single pointer retry

- [ ] Add `messages.preview_typings` to `SCHEMA_SQL`. Count each typing at the §7 count points, immediately before the first literal byte, refusing at 2, while notified, or while parked. Stamp `notified_at` per §7 in member and Director uncertain handling. <!-- completed: -->
- [ ] Stamp an exhausted head, and record the abandonment diagnostics including `typings_exhausted`. <!-- completed: -->
- [ ] Tests: at most two typings per message across two runners delivering to one Director; zero-byte Director ticks and member reservations that return to `ready` leave the count unchanged; an exhausted head is stamped and the next message delivered; newer messages are delivered by the pointer's poll. <!-- completed: -->

### Step 10: Director sweep fit

- [ ] Use the rung selector in the Director attempt with capture geometry and `HERDR_CAPTURE_WIDTH_SLACK`. <!-- completed: -->
- [ ] Tests: Director captures at every §12 size choose the expected rung or hold. <!-- completed: -->

### Step 11: Reporting and placement

- [ ] Add `geometry` to `ObservationHealth`, its validation, `member show` JSON and text, and the `observation` runtime diagnostic. <!-- completed: -->
- [ ] Add the `doctor` `member.<id>.geometry` and `fleet.<fleet_id>.director.geometry` rows (§9). <!-- completed: -->
- [ ] Replace `MIN_PANE_HEIGHT` with the per-backend slot budget in `src/multiplexer/layout.rs` and pass column backends to `choose_slot`. <!-- completed: -->
- [ ] Tests: health validation, doctor rows for each situation, mixed-backend column admission and the overflow minimum. <!-- completed: -->

### Step 12: Native fixtures

- [ ] Capture the §12 matrix for every bundled profile on tmux and herdr. <!-- completed: -->
- [ ] Add the captures to each profile's `screens` with the evidence §12 assigns to their size (`geometry` and `unsubmitted_literal` at typeable sizes, `geometry_hold` otherwise), and cite them from the plans. <!-- completed: -->
- [ ] Confirm provenance validation passes with the captured fixtures. <!-- completed: -->

### Step 13: Validation

- [ ] Run `mise //:test`. <!-- completed: -->
- [ ] Run `mise //:lint`. <!-- completed: -->
- [ ] Run `mise //docs:build`. <!-- completed: -->
- [ ] Reinstall the CLI and repeat the issue #660 reproduction, confirming every Success Criterion that a live fleet observes. <!-- completed: -->
- [ ] Re-run the Director delivery probe on tmux and herdr for each Director-eligible backend and update `SPEC.md` § Role eligibility dates. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-27 | Initial draft |
| 2026-11-27 | Review round 1: rung-based and rung-size criteria; profile-level minimums; `text_top` from the selected `composer.text` rule; stored-text excerpt cut; stall suspension by pane size; Director `composer_too_small` hold and doctor row; `V₀` for `Partial`; durable park (`preview_parked_until`) and typing count (`preview_typings`); typeable-size fixture evidence |
| 2026-11-27 | Review round 2: `B_full` from the plain limits; Director `composer_too_small` limited to unmatched below-minimum screens; typings counted immediately before the first literal byte; exhausted heads stamped |
