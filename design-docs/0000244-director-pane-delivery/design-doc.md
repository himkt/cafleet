# Director Pane Delivery

**Status**: Complete
**Progress**: 55/55 tasks complete
**Last Updated**: 2026-11-23

## Overview

Replace Director reception (`cafleet message listen`, the Claude Monitor recipe, the Codex `--codex-thread` forwarder and the `cafleet_listen` extension tools) with `[cafleet msg …]` previews typed into the Director's own multiplexer pane by the fleet's member runners. Each runner captures the Director pane, classifies it with the bundled screen profiles and delivers only into a verified empty composer with no open interaction. One shared delivery path also gains pre-submit literal verification, so human keystrokes during the settle window never merge into a submission.

## Success Criteria

- [ ] `cafleet message listen` and every receiver built on it are gone: the subcommand and its flags, the Claude Monitor recipe, Codex forwarding, the `cafleet_listen` tools of the opencode and pi extensions, and the whole Hermes plugin with its setup and doctor surfaces.
- [ ] The residue check over tracked files returns no line: `git grep -n -i -E 'message listen|cafleet_listen|codex-thread|codex queue|\[cafleet listen|allow_gateway_injection|plugin_enabled|subscribe probe|stream probe|receiver recovery|no automatic pane input|stdout receiver|listen requirement|Monitor task|CODEX_THREAD_ID' -- SPEC.md docs README.md assets src .claude ':!design-docs'`.
- [ ] Every live member runner sweeps the root Director's pending messages oldest first across senders, serialized by the Director pane's input lock. Each message is typed at most once per confirmed submission and stamped `notified_at` only after post-submit verification confirms it.
- [ ] Director delivery types only into a composer the Director-screen profile verifies empty, with no open interaction. It never uses Escape-first delivery fallback or bracketed paste, and a Director backend or multiplexer without Director-screen coverage receives no automatic input.
- [ ] Captures without cursor or alternate-screen data (herdr) are classified cursor-free; tmux captures keep cursor and screen conditions.
- [ ] Every staged ordinary write (member previews, `member prompt`, `member exec` and Director previews) compares the composer with the written literal before the submit key. On a mismatch it stops with stop cause `literal_mismatch`, without writing the submit key or erasing anything. Every bundled pre-submit fixture extracts to its `expected_literal`.
- [ ] The bundled opencode profile accepts any agent name in its composer status line, and the bundled Hermes profile gains TUI composer and approval-form rules. Each is backed by native screen fixtures captured live from Director screens.
- [ ] SPEC §6.7 defines the Director as `goal` + `pane delivery`. Every backend's `pane delivery` row records a live Director delivery probe run on the shipped build on tmux and herdr, including the herdr interaction-hold step for every backend with a native approval dialog.
- [ ] `SPEC.md`, `docs/`, `README.md`, `assets/skills/` and `.claude/` describe only pane delivery, with no residue of the removed reception surfaces.
- [ ] `mise //:test` and `mise //:lint` pass, and `mise //docs:build` succeeds.

---

## Background

Director reception today is a persistent `cafleet message listen` process. Each backend hands its JSONL wake notices to the session through its own receiver: a Claude Monitor task, Codex `codex queue` forwarding, an OpenCode plugin, a Hermes plugin and a pi extension. That is five receivers with five failure and recovery tables. Codex forwarding also buffers notices that later drain as empty `poll` turns, so the Director sees "listen events arrive too often".

Ordinary members already receive previews from their runner. The runner types a canonical preview into a composer its bundled profile verifies empty. The probe in `researches/director-pane-delivery-probe.md` (2026-11-23) applied the same mechanism to the Director's pane from outside: a multiplexer capture replayed into the runner's terminal model and classified with the bundled profiles. Across all five Director backends on tmux 3.8 and herdr 0.9.5 it delivered idle and busy previews, held for drafts and approval dialogs, and accepted bursts and ~3,000-character previews. Every delivery was confirmed by post-submit verification. The probe also surfaced four required changes:

| Finding | Change in this design |
|---|---|
| Director screens differ for opencode (own agent in the status line) and Hermes (TUI client) | Director-screen rules (§ Profile changes) |
| herdr exposes neither cursor position nor the alternate-screen flag | Cursor-free classification (§ Cursor-free classification) |
| A key typed during the 500 ms settle merged into the submission and still verified `accepted` | Pre-submit literal verification (§ Pre-submit literal verification) |
| Codex inserts input into the running turn; the others queue it behind the turn | Accepted: both never interrupt and arrive no later than the end of the turn |

---

## Specification

### Decisions

| Topic | Decision |
|---|---|
| Removal set | `message listen` (subcommand, `--timeout`, `--codex-thread`, notice hold, ceiling re-notice, `[cafleet listen record]` / `[cafleet listen closed]` protocol), the Claude Monitor recipe, Codex forwarding, the opencode and pi `cafleet_listen` tools and the entire Hermes plugin (it is listen-only) |
| What stays in extensions | The opencode goal extension; the pi goal extension and pi member gate; Director verification for the remaining Director-only tools |
| Director eligibility | `goal` + `pane delivery`. The listen tiers and the subscribe and stream probes are removed |
| Delivering runner | Every live member runner sweeps the root Director's pending messages, oldest first across senders, under the Director pane's input lock, and re-checks the message under that lock |
| No live runner | Director messages stay pending until a runner is live again; the Director polls manually at the points in § No live runner. An exiting runner makes no final Director sweep |
| Preview form | The canonical member preview, one per message; the Director runs `cafleet message poll <director-id> --json` on each preview wake |
| Delivery method | Profile delivery with a plain literal only. No Escape-first fallback and no bracketed paste into the Director pane. A Director backend or multiplexer without Director-screen coverage gets no automatic input and reads by manual `poll` |
| Pre-submit verification | Part of the one shared staged-write path: member previews, prompt, exec and Director previews |
| Held or stalled Director preview | No runtime event; a stored runner diagnostic under the new source `director_delivery`, documented as an accepted trade-off |
| Director pane unreachable | Hold with the Director sweep's backoff (1 s doubling to 30 s) and a `director_delivery` diagnostic. No registry change |
| Cursor-free classification | Only for captures that carry no cursor or alternate-screen data, as declared by the capture source (herdr). No environment variable |
| Director-screen rules | Ordinary rules of the per-backend profile files, with native screen fixtures captured live from Director screens |
| Schema | Two CHECK lists change in `SCHEMA_SQL`, edited in place. `member_input_operations.stop_cause` gains `literal_mismatch`, because C1 puts pre-submit verification on `member prompt` / `member exec`, whose receipts store their stop cause. `member_runner_diagnostics.source` gains `director_delivery` for the Director sweep's diagnostics. The user approved both CHECK additions |

### Director delivery sweep

A Director delivery sweep runs in every member runner, beside that member's own pending-preview sweep. Its multiplexer subprocesses block for up to their deadlines, so it runs on a dedicated runner thread with its own database connection and its own multiplexer adapter built from the Director placement (`mux_backend`, `mux_server`). It never touches the member's owned PTY, FIFO, observation controller or receipts, so a held Director preview never delays the member's own input, and the reverse.

| Property | Contract |
|---|---|
| Thread lifetime | Starts after the run's `started` transition. It stops when the runner begins orderly shutdown or detects runner loss, and the runner joins it within the shutdown deadline (§ Stop and deadlines) |
| Cadence | At most once per Director sweep interval (1 s), and not before the Director sweep backoff expires |
| Eligibility snapshot | One short read transaction, `director_sweep_snapshot`, shaped like the member `sweep_snapshot`. The run's member is active with this run recorded; the fleet's root Director is active with a concrete placement; the Director placement's `coding_agent` has a bundled profile whose `director_delivery` lists the placement's multiplexer. Otherwise the tick does nothing |
| Selection | In the same transaction, `oldest_pending_preview(director_id)`: the oldest `to_member_id=<director> AND status_state='input_required' AND notified_at IS NULL` row by `(created_at, message_id)`, reusing the member sweep's `PENDING_PREVIEW` predicate |
| Lock | Acquire the Director pane's input lock (§6.5 Pane input lock key over the Director placement) nonblockingly. A busy lock ends the tick with no backoff, because another runner is delivering. The sweep thread takes only this lock, so no lock-order cycle exists |
| Re-check under the lock | Re-read the selected message and the Director placement. A stamped, consumed or missing message, or a changed placement, releases the lock and ends the tick |
| Attempt | Capture → classify → write the plain literal → settle 500 ms → pre-submit verification → submit key → post-submit verification (2 s), all under the lock, within a 5 s attempt deadline that starts when the lock is acquired |
| Stamp | `notified_at` is set in one write transaction after confirmed post-submit verification, while the lock is still held, and the transaction succeeds only while that column is still `NULL` |
| Order | Strictly oldest first across every sender. A held head blocks newer Director messages |

Because every runner selects the same head and only the lock holder types, each message is typed at most once per confirmed submission. Captures still happen while delivery is held: up to one per live runner per second while a draft or dialog holds the composer.

The literal is the canonical preview of §6.5 Push notifications, written with the profile's `prompt` plan prefix and submit bytes. The plan's `bracketed_paste` framing and member-measured `plain_literal_limit` are ignored. The Director path never writes a bracketed paste, because a capture cannot observe whether the Director's application enabled mode 2004 (§6.9 Measured submission plans: the bracketed row requires that state).

Member limits were measured on member screens, for example Hermes's 1024-scalar cap on the classic CLI with its 50 ms Enter guard, while the Hermes Director runs the TUI. So the Director limit is `director_delivery.plain_limit` (§ Profile changes), measured on Director screens. A preview whose rendered line exceeds it is rendered in the pointer form. The Step 6 Director maximum-length fixtures establish the limit on each listed multiplexer, and the Step 9 Long preview step confirms it on the shipped build.

#### Stop and deadlines

- The attempt deadline is the earlier of the 5 s attempt deadline and the run's shutdown deadline, matching the event worker's clipping in §6.9. The runner hands the thread its shutdown deadline before it sets the stop signal, and the snapshot, the re-check under the lock and the `notified_at` stamp each run under the database deadline clipped to that attempt deadline.
- The thread checks its stop signal before each stage: capture, literal chunk, settle end, pre-submit capture, submit key and each verification capture. A stopped thread starts no new writing stage and releases the lock.
- A stop after the first literal byte and before confirmed submission is an uncertain Director preview (§ Outcomes and diagnostics). The text stays in the Director composer, and the message stays unnotified.
- Every subprocess runs through the bounded subprocess runner under the remaining clipped deadline, so the join completes within the shutdown deadline.

#### No live runner

Delivery needs at least one live member runner in the fleet. Messages committed while none is live stay pending and unnotified, and the first runner to start or recover delivers them oldest first. Typical cases: the runtime event (`runner_exited`, `runner_lost`) or final report of the last remaining member, and anything sent after the last `member delete`.

An exiting runner makes no final Director sweep. Its shutdown deadline is reserved for its own cleanup, and a sweep could hold the Director lock for its full attempt deadline. Every orderly exit of the last runner follows a Director command (`member delete`, `fleet delete`), so the Director polls right after that command. The Director runs a manual `cafleet message poll <director-id> --json` at these points:

| Point | Reason |
|---|---|
| After a `member delete` that leaves the fleet with no active member | The deleted member's last report and exit event have no runner to deliver them |
| Before `fleet delete` (Shutdown step 1) | Final inbox consumption |
| At each wake's overdue-assignment check (Director wake step 5) | Catches an unexpected exit of the last runner at the next Director turn |
| After Director session restart or context loss | Previews typed before the restart may not be in the new context |

This is an accepted trade-off in §6.5. The removed listener woke the Director with no member alive; pane delivery does not. An unexpected exit of the last runner is noticed only at the next Director turn, which the user or the Director's own ongoing work starts, so an idle Director stays idle with no signal.

#### Classification and hold

The sweep classifies the capture with the Director placement's bundled profile, over the rules its `director_delivery` capability names plus every interaction rule of the profile. A hold leaves the message unnotified for a later tick:

| Classification | Action | Backoff |
|---|---|---|
| Composer verified empty, no open interaction | Deliver | Reset on confirmed submission |
| Composer not verified empty (draft, leftover literal, unrecognized screen, pane narrower than the profile's `W_min`) | Hold | None (retry next interval) |
| Open interaction (permission request, question, unknown interaction) | Hold | None |
| Capture failure, missing pane, mux server error, unstable geometry after retries | Hold | 1 s doubling, capped at 30 s |
| Storage busy or unavailable | Hold | 1 s doubling, capped at 30 s |

The Director's native version is never probed, because CAFleet does not launch the Director. The sweep builds its detector with `Detector::new(catalog, backend, None)`, so each observation's `raw_evidence.native_version` is `null`. Profile and rule selection depend only on the backend (`catalog.profile(backend)`); no rule or fixture is selected by version. § Role eligibility records the versions the delivery probe checked.

#### Outcomes and diagnostics

Director previews create no durable input receipt and publish no runtime event: an event addressed to the Director would itself wait behind the held preview. Each non-confirmed outcome is a runner diagnostic under the new source `director_delivery`. It is stored in `member_runner_diagnostics` like every other source, printed to the runner's stderr as `member runtime: <message>`, and visible through `member show <member-id> --json` of the delivering member. Messages are stable texts without timestamps or durations, so the existing `(source, message)` buffer deduplicates repeats and counts them:

| Outcome | Message state | Stored `message` | Emitted |
|---|---|---|---|
| Confirmed submission | `notified_at` stamped | — | — |
| Held 3 minutes | Unnotified | `director delivery: message <id> held: <reason>` | Once per runner when its continuous hold of that message reaches 3 minutes |
| `literal_mismatch`, `submit_unconfirmed`, `write_failed`, `runner_stopped` after the first byte | Unnotified; typed again once the composer verifies empty | `director delivery: message <id> uncertain: <cause>` | Each occurrence (deduplicated by the buffer) |
| Capture or multiplexer failure | Unnotified | `director delivery: pane <pane-id> unavailable: <cause>` | Each occurrence (deduplicated by the buffer) |
| Storage failure after a confirmed submission | Unnotified; typed again once the composer verifies empty | `director delivery: message <id> uncertain: storage_unavailable` | Each occurrence (deduplicated by the buffer) |
| Storage failure before a message is selected, including the sweep connection | Unnotified | `director delivery: storage unavailable: <cause>` | Each failure, retried under the storage backoff (deduplicated by the buffer) |

`<reason>` is `composer_busy`, `interaction_open`, `pane_unavailable` or `storage_unavailable`; one capture classifies the Director screen, so it has no candidate interaction state. `<cause>` is the multiplexer or storage error text truncated at a scalar boundary so the stored message stays within the 512-byte CHECK bound.

#### Director pane capture

`Multiplexer` gains three methods:

```rust
fn capture_screen(&self, pane: &str) -> Result<ScreenCapture, MultiplexerError>;
fn type_literal(&self, pane: &str, literal: &str) -> Result<(), MultiplexerError>;
fn press_submit(&self, pane: &str, submit: &[u8]) -> Result<(), MultiplexerError>;

pub(crate) struct ScreenCapture {
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) cursor: Option<(u16, u16)>,
    pub(crate) alternate_screen: Option<bool>,
    pub(crate) body: String,
}
```

| Backend | `capture_screen` argv (after the recorded server selector) | Cursor / alternate |
|---|---|---|
| tmux | `display-message -p -t PANE '#{pane_height} #{pane_width} #{cursor_y} #{cursor_x} #{alternate_on}'`, `capture-pane -p -e -N -t PANE`, then the same `display-message` again; a changed metadata line retries every 50 ms within the attempt deadline | `Some` |
| herdr | `api snapshot` for the pane's `rect.width` (in `snapshot.layouts[].panes[]`) and `scroll.viewport_rows` (in `snapshot.panes[]`, joined by `pane_id`; the row count); `pane read PANE --source visible --format ansi` for the body, which keeps leading and interior blank rows and drops only trailing blank rows, so its lines fill the screen from the top. A body with more lines than the row count is a capture failure | `None` |

The capture replays into a fresh `TerminalState` of the capture's size. Replay enters the alternate screen when `alternate_screen == Some(true)`, writes each row at its row position, then places the cursor when known. Classification uses `Detector::observe`. A malformed metadata line or a pane absent from the snapshot is a capture failure.

#### Director pane writes

| Method | tmux | herdr |
|---|---|---|
| `type_literal` | `send-keys -t PANE -l -- <chunk>`, in chunks of at most 256 characters, in order | `pane send-text PANE <literal>` |
| `press_submit` | `send-keys -t PANE -H <hex bytes>` | `pane send-keys PANE <key>` through the explicit-key mapping (`0d` → `enter`) |

Both methods use `acknowledged_write`, so a process that started and then failed or timed out maps through `MultiplexerError::after_write`. Failure semantics:

| Failure | Bytes written | Outcome |
|---|---|---|
| Spawn failure, missing binary or expired deadline before the first `type_literal` process | Zero | Hold, capture-failure backoff |
| Any `type_literal` failure after its first process started, or an unreadable acknowledgment | Possibly partial | Uncertain `write_failed`, stage `partial` |
| `press_submit` failure after its process started | Possibly submitted | Uncertain `write_failed`, stage `submitted` |

herdr `pane send-text` takes literal text with no paste option. Whether herdr frames it as a bracketed paste for an application with mode 2004 enabled is recorded from the Step 6 Director fixtures. The composer-text model covers both renderings (§ Expected rendering). A plan whose submit bytes have no herdr key mapping cannot be declared for herdr in `director_delivery` (§ Profile changes).

### Pre-submit literal verification

Every staged ordinary write gains one stage between the literal settle and the submit key. The member path observes its own PTY screen; the Director path re-captures the pane.

1. Observe the composer through the profile's `composer.text` extraction (§ Composer-text extraction).
2. Match the extracted segments against the written literal (§ Expected rendering).
3. A complete match continues to the submit key.
4. Without a match, re-observe until the composer matches or a 1 s pre-submit window after the settle ends, whichever comes first. The window ends earlier at the attempt deadline and at the stop signal, which keeps its own stop cause.
5. When the window closes without a match, or the composer text cannot be extracted, write no submit key and erase nothing. Record `uncertain` with `stop_cause=literal_mismatch`, stage `partial`. A text that extends beyond the literal, such as an appended human key, still mismatches when the window closes.

A literal that renders late, as a codex herdr 1024-scalar literal did after the first observation, matches within the window. The remaining race is a key typed between the pre-submit observation and the submit key, a window of one observation-to-write gap. It replaces the broader "human typing during the write can mix with the typed literal" trade-off in §6.5 Accepted trade-offs. A run without composer verification has no `composer.text` and keeps its current stages.

#### Composer-text extraction

Each profile with composer verification declares `composer.text`:

```json
{
  "rule_ids": ["codex-idle-composer", "codex-composer-text"],
  "first_prefix": "^(?:›|!) ",
  "continuation_prefix": "^  ",
  "mode_prefix": "!",
  "wrap": "char",
  "placeholders": [
    {"pattern": "\\[Pasted Content (?<count>\\d+) chars\\]", "unit": "scalars"}
  ]
}
```

| Field | Meaning |
|---|---|
| `rule_ids` | Composer region rules whose region bounds the composer rows; a `composer_text` observation is one of them matching exactly one region |
| `first_prefix` | Pattern anchored at the first composer row; the matched prefix is stripped |
| `continuation_prefix` | Pattern stripped from each later composer row (the continuation margin, e.g. codex `  `); a row that does not match it ends the composer text |
| `mode_prefix` | Optional. A literal that begins with it matches with or without it, because the native CLI may consume it as a mode switch: claude and codex draw `!` as the mode prompt, opencode draws nothing, and a codex single burst sometimes keeps the `!` as text |
| `framed` | Optional, default `false`. The region's start row is a frame row (pi's top editor border) and composer text begins on the next row |
| `wrap` | `char`: rows join with nothing. `word`: a row break matches either nothing or exactly one literal space |
| `placeholders` | Collapsed-paste renderings with a named `count` group and a `unit`: `scalars` (codex) or `lines` (Hermes `\[Pasted text #\d+: (?<count>\d+) lines → [^\]]*\]`) |

A `composer_text` rule may carry `text_prefixes: {first_prefix, continuation_prefix}`, `text_wrap` and `text_framed`, which replace the `composer.text` prefixes, `wrap` and `framed` for the regions that rule bounds, so one profile reads composers that differ in margin, wrapping or framing; only `composer_text` rules declare them.

Cells drawn in the profile's suggestion style are excluded before extraction, and trailing blanks are trimmed from each row. A `composer_text` rule's region runs from its start row to the first later row that matches `end_pattern`, so each start row yields one candidate, and that end row is never composer text. A start row with no later `end_pattern` row within `max_rows` is closed by the screen's bottom edge, as a composer drawn on the screen's last rows is, and every row of that region is composer text. Native transcripts echo submitted prompts in the composer's own shape, and the live composer is always below the transcript, so a `composer_text` rule takes the bottom-most candidate that satisfies its matchers instead of requiring a unique one. A wrong pick can only fail the literal comparison, which withholds the submit. `composer_text` rules rank below the empty-composer rules, so a composer that both match classifies empty.

#### Expected rendering

The extracted composer text is split left to right into segments: each placeholder match is a placeholder segment, and the text between matches is a text segment. The match walks the written literal (prefix plus rendered preview, prompt or command) in order:

| Segment | Consumes from the literal |
|---|---|
| Text | Exactly its own characters, with the `wrap` rule at each row join |
| Placeholder, unit `scalars` | Exactly `count` Unicode scalar values |
| Placeholder, unit `lines` | Exactly one whole written literal chunk (one bracketed-paste write), whose line count must equal `count` |

The match succeeds only when every segment matches and both the segments and the literal are fully consumed. Examples from the bundled native screens:

| Native screen | Segments | Literal |
|---|---|---|
| codex `› [Pasted Content 1024 chars]a` | placeholder(1024 scalars), text `a` | 1025 scalars |
| codex `!  true aaaa…` then `  aaaa…` continuation rows (`mode_prefix` `!`) | text ` true aaaa…aaaa…` (mode prompt `! ` and margin stripped, word wrap) | `! ` + command, matched without `!` |
| Hermes classic CLI `❯ [Pasted text #5: 1 lines → /root/.hermes/pastes/…]` (`first_prefix` `^(?:☤ )?❯ `, the `☤ ` drawn only during a working turn) | placeholder(1 line) | one one-line paste write |

#### Fixtures and regression guard

The comparison runs on every member delivery that works today, so each profile with composer verification proves it on native screens before the change ships:

A *pre-submit fixture* is a screen with a `composer_evidence` entry of kind `unsubmitted_literal`: the literal is typed and the submit key is not. Kinds `framing`, `plain_limit` and `post_submit` are excluded, because the bundled `framing` records sit on post-submit screens. Every `unsubmitted_literal` entry gains a required field `expected_literal`. It is the exact literal written before the screen was captured, including the operation prefix such as `! `, as a JSON string; fixture literals are synthetic measurement text. A `{"repeat": "<unit>", "count": N, "prefix": "<text>"}` object may stand for a long literal: `prefix` followed by `unit` repeated `count` times.

| Rule | Contract |
|---|---|
| Provenance validation | Rejects a pre-submit entry without `expected_literal`, an `expected_literal` on any other kind, and a malformed repeat object |
| Masked rows | A pre-submit fixture's composer region (the rows its `composer.text` rule bounds) overlaps no masked row; validation rejects an overlap. Masked transcript rows outside the region are allowed |
| Cursor-free fixtures | A herdr fixture records `null` `cursor_row`, `cursor_col` and `alternate_screen` (herdr reports neither), takes the layout width as `cols`, replays cursor-free and carries the screen field `"multiplexer": "herdr"`. Validation requires that field exactly on screens with the nulls |
| Existing fixtures | Each gets its `expected_literal` from the measurement that produced it (fixture ID and the design doc's evidence record). A fixture whose literal cannot be recovered, or whose composer region is masked, is recaptured |

Regression and coverage rules:

- A unit test in `src/coding_agent/catalog.rs` iterates every bundled pre-submit fixture and requires its extracted composer text to match `expected_literal` under § Expected rendering.
- Each profile adds, per operation plan (`prompt`, `exec`), a one-row literal, a wrapped literal and a maximum-length literal pre-submit fixture: the plan's plain limit, or the 4000-scalar inline limit when the limit is null. A `bracketed_paste` plan adds one paste fixture.
- Each backend with `director_delivery` adds the same three prompt pre-submit fixtures captured from its Director pane on each multiplexer it lists; the maximum-length one is at `director_delivery.plain_limit`.

#### Mismatch recovery

A mismatch leaves the written literal, plus whatever else was typed, in the composer. The composer then holds every later preview and queued input until it is cleared.

| Pane | What happens | Recovery |
|---|---|---|
| Member, preview | The uncertain preview publishes its receipt-free `input_delivery_result` with `cause: literal_mismatch`; it stays unnotified, and after 3 minutes the held head publishes `input_delivery_stalled` | The Director runs `member capture`, then either asks the user to clear or submit the composer, or, when authorized, sends `member press <member-id> --from-member-id <director-id> <keys>` with the keys the capture shows are needed (the existing stalled-delivery recovery) |
| Member, prompt/exec | The receipt is `uncertain/literal_mismatch`; the FIFO stops | As above, then `input reconcile` with the disposition the capture establishes |
| Director | `director_delivery` diagnostic; the message stays unnotified | The user clears or submits the composer in the Director pane |

A mismatch on a member pane happens only when something else typed into it, so a human or a `member press` is involved. When the leftover literal is submitted instead of cleared, the recipient reads it, and because the message stays unnotified the runner later types the preview again. The recipient deduplicates by message ID, as for every uncertain preview (§6.5 Accepted trade-offs).

### Profile changes

| Profile element | Change |
|---|---|
| `capabilities.director_delivery` | New: `{ "coverage": "tested", "multiplexers": ["tmux", "herdr"], "rule_ids": [...], "plain_limit": {"unit": "scalars", "max": 1024}, "fixture_ids": [...] }`. `rule_ids` names the composer rules (empty-composer and `composer.text` rules) that classify the backend's Director screen. `fixture_ids` cites the Director-pane fixtures, including the pre-submit fixtures at `plain_limit`. `multiplexers` lists each multiplexer whose Director delivery probe passed. `plain_limit` is required: the largest plain literal the Director screen renders intact on every listed multiplexer, or `null` when the 4000-scalar inline limit rendered intact. Absent means no Director delivery for that backend |
| `composer.text` | New, per § Composer-text extraction |
| opencode | `opencode-idle-composer` and `opencode-suggestion-composer` end patterns accept any agent name in place of `cafleet(?:-ask)? · `, excluding the `Shell` and `esc normal` mode labels drawn in the same position, so a composer in shell or normal mode never classifies empty |
| hermes | New `hermes-tui-idle-composer`, with the alternate screen, cursor `any` and a one-row region whose start, end and matcher are the whole composer line `^ ❯(?: (?:Ask me anything…\|Try "[^"]*"\|Ctrl\+C to interrupt…))? *(?:┃ *)?$`. Also a TUI composer-text rule `hermes-tui-composer-text`, whose region starts at the ` ─ ` status row (skipped by `text_framed`, because transcript user lines also start with ` ❯ `), is closed by the screen's bottom edge and carries `text_prefixes` `^ ❯ ` and `^   ` for the TUI's one-column margin and `text_wrap` `word`, and a TUI approval-form detection rule (`hermes-tui-approval`, detection only, no answer plans). Members keep the classic CLI rules |
| Fixtures | Each new or changed rule carries native screen fixtures captured live from a Director pane, with provenance naming the capture source (`tmux capture-pane` or `herdr pane read`), backend version and date, validated by the existing provenance check |

The Hermes TUI approval rule is required because the classic-CLI request rules match the primary-screen `╭` box only. Without it, a TUI approval form could leave the composer line verified empty and the preview would be typed into the form.

Loader validation rejects a profile file that:

| Condition | Rejection |
|---|---|
| Has an executable composer plan and no `composer.text`, or a `composer.text` rule ID that names a missing or non-composer rule | `composer plan for <backend> requires composer text extraction` |
| Has a `director_delivery.rule_ids` entry naming a missing rule or a rule that is not a composer rule | `director delivery rule <id> of <backend> is not a composer rule` |
| Lists in `director_delivery.multiplexers` a multiplexer whose prompt plan's submit bytes have no key mapping there | `director delivery for <backend> on <multiplexer> has no submit key mapping` |
| Has a `director_delivery` without `plain_limit` | `director delivery for <backend> requires a measured plain_limit` |
| Has a placeholder pattern without a `count` group or with a unit other than `scalars` or `lines` | `composer placeholder of <backend> must capture count in scalars or lines` |

Provenance validation rejects, with the same `director delivery for <backend> requires a measured plain_limit` text, a `director_delivery` with no Director pre-submit fixture at `plain_limit` on a listed multiplexer.

### Cursor-free classification

`ScreenSnapshot` replaces its `cursor_row`, `cursor_col` and `alternate_screen` fields with `cursor: Option<(u16, u16)>` and `alternate_screen: Option<bool>`. Runner PTY snapshots always carry `Some`. When either is `None`, the detector:

| Rule element | Cursor-free behavior |
|---|---|
| Region `cursor` condition (`at_end`, `after_prompt`, `inside`) | Treated as `any` |
| Region `screen` selector | Ignored |
| Candidate uniqueness | Checked after the region matchers run, so exactly one candidate must match its matchers, instead of exactly one candidate before matching |

A `composer_text` rule keeps its bottom-most-candidate selection (§ Composer-text extraction) in place of the uniqueness check.

On a cursor-free screen a `composer_empty` rule likewise takes its bottom-most candidate that satisfies its matchers, and only the composer matches, of either kind, whose region ends lowest on the screen compete; the live composer is the lowest composer region, and a composer-shaped box above it, such as pi's `Update Available` box, belongs to the transcript. Interaction rules are unaffected.

The probe needed the matcher-before-uniqueness order for pi, where several separator pairs qualify without the cursor. tmux Director captures and every member observation keep the cursor and screen conditions unchanged.

### Removed surfaces

| Surface | Removal |
|---|---|
| CLI | `message listen` with `--timeout`, `--codex-thread`, the JSONL records, notice hold, Codex forwarding, session pinning and liveness; its parse tests and `listen_*` JSON presenters |
| Broker | `listen_snapshot`, `listen_snapshot_with_clock`, `read_listen_snapshot`, `select_listen_snapshot` and their tests. The Director sweep uses `oldest_pending_preview` inside `director_sweep_snapshot` under the existing `with_database_deadline` |
| Claude | The Monitor recipe, Monitor notes, Monitor host-evidence table |
| Codex | Forwarding, the `CODEX_THREAD_ID` recipe, the Codex listener-startup troubleshooting |
| opencode extension | `cafleet_listen`, its receiver child, `[cafleet listen …]` prefixes. The goal-restoration wording that depended on the re-arming `cafleet_listen` turn now refers to the session's next event |
| pi extension | `cafleet_listen`, its receiver child, the runtime-replaced closed notice. Restart continuation refers to the next turn instead of the re-arming `cafleet_listen` turn |
| Hermes plugin | `assets/extensions/hermes/` entirely; setup's Hermes extension install, executable record and plugin enablement (`plugins.enabled`, `allow_gateway_injection`); doctor's `hermes.extension` and `hermes.plugin_enabled` checks; Hermes setup output becomes the skills line only |
| Requirements | Listen tiers, subscribe probe, stream probe, tier negative-evidence rules, `Delivery` column values `subscribe`/`compromise` |
| Director role | Spawn Protocol receiver arming, receiver handle retention, Receiver recovery table, Shutdown listener-closure collection |

The Hermes Director keeps the TUI client (`hermes chat --tui`), where native `/goal` was verified and the Director-screen rules apply. Its `fleet create` still needs `--pane` because its terminal tools run in the shared gateway.

### Role requirements (SPEC §6.7)

| Role | Requirement | Established by |
|---|---|---|
| Director | `goal` | Unchanged: native goal lifecycle controls, or a CAFleet extension meeting the extension-goal minimum bar |
| Director | `pane delivery` | The bundled profile declares `director_delivery` with coverage `tested` and both `tmux` and `herdr`, and the Director delivery probe passes on both for the installed native version |
| Member | — | Unchanged |

The `Role eligibility` table keeps `Requirement` values `goal`, `pane delivery` and `verdict`. `Source` stays `native`/`extension` for `goal` and is `—` for `pane delivery`. The `Delivery` column is removed. The verdict row reads `goal and pane delivery rows`, or names each unmet requirement.

**Director delivery probe**, per backend and multiplexer, on the shipped build, with a Director registered through `fleet create` and one probe member:

| Step | Procedure | Pass condition |
|---|---|---|
| Idle | Send one message to the idle Director | The preview starts a turn without other input, and the Director's poll returns the message |
| Busy | During a foreground command of at least 60 s, send two messages | The command is not interrupted; both previews are read no later than the end of that turn |
| Draft hold | Type a draft into the Director composer, send one message, wait 10 s, then clear the draft | Nothing is typed while the draft exists and the draft stays intact; after clearing, the preview is delivered |
| Interaction hold | Open a native approval dialog with the setup below, send one message, then answer the dialog | Nothing is typed into the dialog; the preview is delivered after it closes |
| Burst | During a foreground command, send 10 messages | Every preview is accepted, and polls return every ID |
| Long preview | Send a body whose preview reaches `director_delivery.plain_limit` (the 4000-scalar inline limit when null) and one above it | Inline preview intact; above the limit, the pointer form arrives |
| Settle race | Type a key during the settle window of a delivery | The attempt records `literal_mismatch`, no submit key is written; after the composer is cleared, the preview is delivered |

Interaction-hold setup:

| Backend | Setup that opens a native approval dialog |
|---|---|
| claude | Launch the Director with `--permission-mode default`; ask it to run a Bash command outside its allow rules |
| codex | Launch with `-s read-only -a on-request -c approvals_reviewer="user"`; ask it to run a command that writes a file |
| opencode | A project `opencode.json` with `"permission": {"bash": "ask"}` (`OPENCODE_PERMISSION` has no effect); ask it to run a shell command |
| hermes | A probe Hermes home with `approvals.mode: manual`; ask it to run a command matching Hermes's dangerous-command patterns, such as recursively deleting a temporary directory |
| pi | — : pi has no native approval dialog (the CAFleet pi gate is member-only) |

`—` is recorded only for a backend with no native approval dialog. A dialog that cannot be reproduced fails the step.

Each run records backend, native version, multiplexer and version, pane geometry, message IDs and the check date in this design doc's evidence record and in SPEC § Role eligibility.

**Failure path.** A failed step on the shipped build removes that multiplexer from the backend's `director_delivery.multiplexers` (and the whole capability when none remains). The `pane delivery` row is then `no`, naming the failed step, multiplexer and version, and the verdict row names `pane delivery`. Director delivery still runs on a listed multiplexer, but the backend is not Director-eligible. The existing Director rule governs it: `director.md` tells an agent on a `no` backend to report the verdict and its unmet requirements, name eligible backends and leave fleet creation to an eligible host. `fleet create` keeps registering any backend, as today. Manual `poll` is the reception path for an eligible Director whose delivery is held or unavailable at runtime, not a substitute for eligibility.

### Documentation changes

Order follows `.claude/rules/documentation-maintenance.md` § Implementation Order. Every listed surface describes only the current behavior.

| Target | Change |
|---|---|
| `SPEC.md` §6.3 | Remove `message listen` and `###### Codex forwarding`, their Subcommand summary row and Error Messages rows. Reword `message send`, `message poll` and `fleet create` to name Director pane delivery. Remove the Hermes extension and plugin rows from Asset installation and setup output, `###### Hermes plugin enablement` and the Hermes extension checks |
| `SPEC.md` §6.5 | Push notifications: the root Director receives previews from member runners (§6.9 Director delivery sweep). Held delivery adds the Director hold reasons. Pane input lock and Fleet layout lock name the Director sweep as a writer taking only the Director pane lock. Backend operations add `capture_screen`, `type_literal` and `press_submit`. Prompt dispatch keeps "no member input path captures the pane" and scopes captures to Director delivery |
| `SPEC.md` §6.5 Accepted trade-offs | Replace the human-typing mix bullet with the pre-submit residual race. Add the Director trade-offs: no stall event, a held head blocks newer Director messages, one capture per runner per second while held, no Director version gate, no delivery while no member runner is live, and an unexpected exit of the last runner noticed only at the next Director turn |
| `SPEC.md` §6.6 | Director reception is the Director delivery sweep plus the consuming poll each preview prompts |
| `SPEC.md` §6.7 | Role requirements, Role eligibility and the probe per § Role requirements. Host evidence: remove Monitor, Codex forwarding and plugin reception prose and the Monitor event table. Rewrite the Claude paragraph's "the root Director's pane, which receives no automatic pane input" (now: Director delivery holds while an approval or `AskUserQuestion` is open). Director extensions: remove receivers; keep goal extensions, pi member gate and Director verification |
| `SPEC.md` §6.9 | Bundled profiles (`director_delivery`, `composer.text`, placeholders, cursor-free classification, Hermes TUI rules, loader rejections) and Measured submission plans (pre-submit stage). Stop cause (`literal_mismatch`) and Runner diagnostics (`director_delivery` source row). New `##### Director delivery sweep` after Runner pending-preview sweep. Rewrite Director consumption and continuation (remove "stdout receiver … no composer injection"), the Runner boundary diagram's `Director stdout receiver` node and Coverage and latency (two seconds from commit to the start of a delivery attempt while the composer is verified empty) |
| `SPEC.md` §8 | `stop_cause` CHECK includes `literal_mismatch`; `source` CHECK includes `director_delivery` |
| `docs/docs/concepts/coding-agents.md` | Director requirements table: reception column becomes pane delivery for every backend; remove Codex listener startup and the pi `cafleet_listen` troubleshooting rows; Hermes notes drop gateway injection |
| `docs/docs/mixed-backend-team.md` | Rewrite Receive results, setup and shutdown. Previews arrive in the Director pane and the Director polls on each. A draft or open dialog holds them. Manual `poll` covers the no-live-runner points |
| `docs/docs/concepts/overview.md` | Core terms: replace the `listener` row with a `Director delivery` row |
| `docs/docs/concepts/members.md`, `docs/docs/quickstart.md` | Replace listener mentions with Director delivery; remove the Codex listener troubleshooting link |
| `docs/docs/contributing.md` | Role evaluation uses `goal` + `pane delivery`; replace the subscribe and stream probe sections with the Director delivery probe, its interaction-hold setup and failure path; remove the listen snapshot implementation note |
| `README.md` | Reconcile through `/update-readme`; no listener mention exists today |
| `assets/skills/cafleet/SKILL.md` | Send (Unicast): the root Director receives previews from member runners like any member |
| `assets/skills/cafleet/reference/cli.md` | Remove the `message listen` section, its Subcommand summary row and the Monitor authorization sentence. Poll no longer mentions listener notices. Add the Director preview facts an agent acts on: preview form, hold conditions and the manual poll points |
| `assets/skills/cafleet/reference/coding-agents.md` | Role requirements table (`goal` + `pane delivery`). Remove the Claude and Codex native reception recipes and every Monitor, forwarding and `cafleet_listen` Note row. Role eligibility Basis cells follow the probe run |
| `assets/skills/cafleet/roles/director.md` | Spawn Protocol drops step 4. Director wake: each `[cafleet msg …]` preview is a wake; run `poll --json` once, record, dispatch. A draft or open dialog in your pane holds previews, so keep the composer clear between turns. Add the manual poll points (§ No live runner). Remove Receiver recovery; Shutdown runs a final poll |
| `assets/skills/cafleet-design-doc/SKILL.md` | Dispatch wording: keep Director wake until fleet deletion, without "reception" |
| `.claude/skills/skill-author/SKILL.md`, `.claude/skills/clean-docs/SKILL.md` | Replace "arm native reception" and receiver-closure steps with Director wake |
| `.claude/rules/documentation-maintenance.md` | Director verdict changes and the Template's Role eligibility use `goal` + `pane delivery` and the Director delivery probe; replace the "the listener" concept example |

### Testing

Every test is an in-file unit test under `mise //:test`, with captures written as literals in the test module.

| Area | File owning the tests | Cases |
|---|---|---|
| Capture parsing | `src/multiplexer/tmux.rs`, `src/multiplexer/herdr.rs` | Metadata parsing; geometry change retry; herdr row count and missing pane; `None` cursor for herdr |
| Director writes | `src/multiplexer/tmux.rs`, `src/multiplexer/herdr.rs` | Chunked `send-keys -l`; submit hex; herdr key mapping; zero-byte failure before the first process versus `after_write` failure after it |
| Cursor-free detector | `src/coding_agent/detector.rs` | `None` cursor treats every cursor rule as `any` and ignores the screen selector; matcher-before-uniqueness picks the one matching candidate; drafted composers stay not-empty; `native_version` is `null` without a version |
| Profile rules | `src/coding_agent/catalog.rs` | `director_delivery` parsing; every loader rejection in § Profile changes; prefix stripping, char and word wrap; scalars and lines placeholders; every bundled pre-submit fixture extracts to its `expected_literal`; provenance rejects a missing or misplaced `expected_literal` and a masked composer row; opencode any-agent status line and Hermes TUI composer and approval rules against their bundled fixtures |
| Pre-submit verification | `src/runtime/input.rs` | Match continues to submit. An appended key gives `literal_mismatch` with no submit byte. Placeholder match; unextractable composer gives a mismatch. FIFO stop for prompt/exec; preview retyped after the composer verifies empty; recovery: a `member press` that clears the composer lets the held preview deliver |
| Director sweep | `src/runtime/service.rs` scenario harness with `TestClock` and a fake multiplexer | Oldest-first selection across senders. Busy lock skips; the re-check under the lock skips a stamped or consumed message. Holds for draft, interaction and narrow pane; backoff on capture failure; stamp only after verification; no runtime event. One `director_delivery` diagnostic per held message at 3 minutes. No Director coverage, or an unlisted multiplexer, means no capture. Above the plain limit, the pointer form is written and no paste. A stop signal between stages starts no new write within the clipped deadline. No live runner means the message stays pending; a runner started later delivers it |
| Schema | `src/db.rs` | `stop_cause='literal_mismatch'` and `source='director_delivery'` accepted |

Live verification is the Director delivery probe (§ Role requirements) on all five backends × tmux and herdr with the shipped build. The herdr interaction-hold step runs for claude, codex, opencode and hermes; pi records `—`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SPEC.md

- [x] §6.3: remove `message listen` and Codex forwarding, their Subcommand summary row and Error Messages rows; reword `message send`, `message poll` and `fleet create` <!-- completed: 2026-11-23T10:01 -->
- [x] §6.3: remove the Hermes extension and plugin rows from Asset installation and setup output, `###### Hermes plugin enablement`, and the Hermes rows of the doctor check table and Extension checks <!-- completed: 2026-11-23T10:01 -->
- [x] §6.5: rewrite Push notifications, Held delivery and Accepted trade-offs; name the Director sweep in Pane input lock and Fleet layout lock; add the three multiplexer methods to Backend operations; scope captures in Prompt dispatch <!-- completed: 2026-11-23T10:01 -->
- [x] §6.6: state Director reception as the Director delivery sweep plus the consuming poll <!-- completed: 2026-11-23T10:01 -->
- [x] §6.7: replace the listen requirement and probes with `pane delivery`, the Director delivery probe, its interaction-hold setup and failure path; restructure Role eligibility (drop `Delivery`, `pane delivery` rows pending the Step 9 evidence) <!-- completed: 2026-11-23T10:01 -->
- [x] §6.7: remove reception from Host evidence (claude, codex, opencode, hermes), rewrite the Claude "no automatic pane input" sentence, and remove receivers from Director extensions, keeping goal extensions, pi member gate and Director verification <!-- completed: 2026-11-23T10:01 -->
- [x] §6.9: add `director_delivery`, `composer.text`, placeholders, Hermes TUI rules, loader rejections and cursor-free classification to Bundled profiles; add the pre-submit stage to Measured submission plans; add `literal_mismatch` to Stop cause and `director_delivery` to Runner diagnostics <!-- completed: 2026-11-23T10:01 -->
- [x] §6.9: add `##### Director delivery sweep`; rewrite Director consumption and continuation, the Runner boundary diagram node and Coverage and latency <!-- completed: 2026-11-23T10:01 -->
- [x] §8: add `literal_mismatch` to the `stop_cause` CHECK and `director_delivery` to the `source` CHECK; confirm §7 keeps only `§` pointers <!-- completed: 2026-11-23T10:01 -->
- [x] Scan `SPEC.md` for Director-reception wording outside the residue patterns ("notice", "receiver", "reception", "listener", "no automatic pane input") and rewrite every hit that describes the removed reception <!-- completed: 2026-11-23T10:01 -->

### Step 2: docs/ and README.md

- [x] `docs/docs/concepts/coding-agents.md`: Director requirements table, Hermes notes and removal of the listener troubleshooting <!-- completed: 2026-11-23T10:20 -->
- [x] `docs/docs/mixed-backend-team.md`: setup, Receive results, manual poll points and shutdown in pane-delivery terms <!-- completed: 2026-11-23T10:20 -->
- [x] `docs/docs/concepts/overview.md`, `members.md` and `quickstart.md`: replace listener mentions <!-- completed: 2026-11-23T10:20 -->
- [x] `docs/docs/contributing.md`: role evaluation, the Director delivery probe procedure with setup and failure path, and removal of the listen implementation note <!-- completed: 2026-11-23T10:20 -->
- [x] Reconcile `README.md` with `/update-readme` and run `mise //docs:build` <!-- completed: 2026-11-23T10:22 -->

### Step 3: Skills and rules

- [x] `assets/skills/cafleet/SKILL.md` and `reference/cli.md`: remove listen and add Director preview facts <!-- completed: 2026-11-23T10:30 -->
- [x] `assets/skills/cafleet/reference/coding-agents.md`: Role requirements, remove reception recipes and Note rows, Role eligibility Basis placeholders pending Step 9 <!-- completed: 2026-11-23T10:35 -->
- [x] `assets/skills/cafleet/roles/director.md`: Spawn Protocol, Director wake with the manual poll points, remove Receiver recovery, Shutdown <!-- completed: 2026-11-23T10:40 -->
- [x] `assets/skills/cafleet-design-doc/SKILL.md`, `.claude/skills/skill-author/SKILL.md` and `.claude/skills/clean-docs/SKILL.md`: Director wake wording <!-- completed: 2026-11-23T10:45 -->
- [x] `.claude/rules/documentation-maintenance.md`: Director verdict changes, Template Role eligibility and the concept example <!-- completed: 2026-11-23T10:48 -->

### Step 4: Remove listen and receivers

- [x] Remove `message listen` from `src/cli/message.rs` and `src/cli/mod.rs`, including Codex forwarding, session pinning and their tests <!-- completed: 2026-11-23T10:40 -->
- [x] Remove the `listen_*` presenters from `src/output/json.rs` and the `listen_snapshot` family from `src/broker/messaging.rs`, with their tests <!-- completed: 2026-11-23T10:40 -->
- [x] Remove `cafleet_listen` and its receiver code from `assets/extensions/opencode/cafleet.ts` and `assets/extensions/pi/cafleet.ts`, keeping goal tools, Director verification and the pi member gate <!-- completed: 2026-11-23T10:40 -->
- [x] Delete `assets/extensions/hermes/` and remove Hermes extension install, executable record, plugin enablement and doctor checks from `src/assets.rs`, `src/cli/setup.rs` and `src/coding_agent/hermes.rs` <!-- completed: 2026-11-23T10:40 -->

### Step 5: Capture, writes and cursor-free classification

- [x] Change `ScreenSnapshot` to `cursor: Option<(u16, u16)>` and `alternate_screen: Option<bool>`, updating every producer and consumer <!-- completed: 2026-11-23T10:40 -->
- [x] Implement cursor-free classification in `src/coding_agent/detector.rs` with its tests <!-- completed: 2026-11-23T10:40 -->
- [x] Add `capture_screen` to the `Multiplexer` trait with tmux and herdr implementations and tests <!-- completed: 2026-11-23T10:40 -->
- [x] Add `type_literal` and `press_submit` for tmux and herdr with their failure mapping and tests <!-- completed: 2026-11-23T10:40 -->
- [x] Add capture replay into `TerminalState` with tests <!-- completed: 2026-11-23T10:40 -->

### Step 6: Profiles

- [x] Add `capabilities.director_delivery` (with `plain_limit`) and `composer.text` (prefixes, wrap, placeholders) to the catalog schema and loader, with every loader rejection and its tests <!-- completed: 2026-11-23T10:55 -->
- [x] Add `expected_literal` to pre-submit `composer_evidence` with its provenance and masked-row validation, and backfill it for every existing pre-submit fixture, recapturing unrecoverable or masked ones <!-- completed: 2026-11-23T11:52 -->
- [x] Implement composer-text extraction and segment matching in `src/coding_agent/catalog.rs` and the regression test over every bundled pre-submit fixture <!-- completed: 2026-11-23T12:00 -->
- [x] Capture live member-screen one-row, wrapped and maximum-length fixtures per operation plan for every profile with composer verification, plus a paste fixture for each `bracketed_paste` plan, recording provenance <!-- completed: 2026-11-23T11:52 -->
- [x] Add `composer.text` and `composer_text` rules to every profile file with composer verification <!-- completed: 2026-11-23T11:52 -->
- [x] Apply the opencode any-agent status-line change with Director fixtures <!-- completed: 2026-11-23T11:20 -->
- [x] Add the Hermes TUI composer, composer-text and approval-form rules with Director fixtures captured under `approvals.mode: manual` <!-- completed: 2026-11-23T11:52 -->
- [x] Measure each backend's Director-screen plain limit on tmux and herdr, and capture Director-pane one-row, wrapped and maximum-length prompt fixtures at it, recording whether herdr `pane send-text` is framed as a paste <!-- completed: 2026-11-23T11:52 -->
- [x] Declare `director_delivery` with its rule IDs and multiplexers in every profile whose Director fixtures cover them <!-- completed: 2026-11-23T11:52 -->

### Step 7: Pre-submit literal verification

- [x] Add the pre-submit stage to the shared staged write in `src/runtime/input.rs` <!-- completed: 2026-11-23T11:10 -->
- [x] Add `literal_mismatch` to the stop-cause type, its projections, result events and `SCHEMA_SQL` <!-- completed: 2026-11-23T11:10 -->
- [x] Tests for match, appended key, placeholder, unextractable composer, prompt/exec FIFO stop, preview retype and `member press` recovery <!-- completed: 2026-11-23T11:52 -->

### Step 8: Director delivery sweep

- [x] Add `director_sweep_snapshot` reusing `oldest_pending_preview`, and the guarded `notified_at` stamp that succeeds only while the column is `NULL` <!-- completed: 2026-11-23T11:10 -->
- [x] Implement the Director sweep thread with its own connection and multiplexer adapter: start, stop signal, join, cadence and backoff <!-- completed: 2026-11-23T11:10 -->
- [x] Implement the locked attempt: lock, re-check, capture, classify, plain literal or pointer form, settle, pre-submit check, submit, verify, stamp, with the deadline clipped to the shutdown deadline <!-- completed: 2026-11-23T11:10 -->
- [x] Add the `director_delivery` diagnostics source to the runner diagnostics type and `SCHEMA_SQL`, with the 3-minute held diagnostic per runner and message <!-- completed: 2026-11-23T11:10 -->
- [x] Scenario-harness tests for every Director sweep case in § Testing <!-- completed: 2026-11-23T11:52 -->
- [x] Run `mise //:test` and `mise //:lint` <!-- completed: 2026-11-23T12:00 -->

### Step 9: Live verification and evidence

- [x] Install the build and run the Director delivery probe for claude on tmux and herdr, including the herdr interaction hold <!-- completed: 2026-11-23T13:20 -->
- [x] Run the probe for codex on tmux and herdr, including the herdr interaction hold <!-- completed: 2026-11-23T13:20 -->
- [x] Run the probe for opencode on tmux and herdr, including the herdr interaction hold <!-- completed: 2026-11-23T13:20 -->
- [x] Run the probe for hermes on tmux and herdr, including the herdr interaction hold <!-- completed: 2026-11-23T13:20 -->
- [x] Run the probe for pi on tmux and herdr, recording `—` for the interaction hold <!-- completed: 2026-11-23T13:20 -->
- [x] Apply the failure path to any failed step: update `director_delivery.multiplexers` and the `pane delivery` and verdict rows <!-- completed: 2026-11-23T13:20 -->
- [x] Record the evidence in this design doc, `SPEC.md` § Role eligibility and the `coding-agents.md` Role eligibility Basis cells <!-- completed: 2026-11-23T13:20 -->
- [x] Run the Success Criteria residue check and `mise //docs:build`; fix every hit <!-- completed: 2026-11-23T13:20 -->

### Evidence

The Director delivery probe ran on build 9b90d851 on 2026-11-23, every run started together, with tmux 3.8 (Director pane 120x39, window 241x39) and herdr 0.9.5 (Director pane 120x41 in a client-attached session). Each run used messages idle 1, busy 2–3, draft hold 4, burst 5–14, long preview 15 (inline at the plain limit) and 16 (pointer form one scalar above it), settle race 17 and interaction hold 18 (hermes: 19; pi: none).

| Backend | Native version | Plain limit | tmux | herdr | Interaction hold |
|---|---|---|---|---|---|
| claude | Claude Code 2.1.325 | 456 | pass | pass | Bash approval, answered |
| codex | codex-cli 0.169.0 | 1024 | pass | pass | relaunched `-s read-only -a on-request`, Esc |
| opencode | opencode 2.3.0 | 380 | pass | pass | Permission required, rejected |
| hermes | Hermes Agent 0.22.4 (2026.9.24) | 1480 | pass | pass | TUI approval form for a recursive delete, answered 4 Deny |
| pi | pi 1.5.2 | 474 | pass | pass | — |

Every settle race recorded exactly one `director delivery: message 17 uncertain: literal_mismatch` diagnostic, left message 17 unnotified until the composer was cleared, then delivered it as one submitted line. No failure path applied, so every `director_delivery` keeps `tmux` and `herdr`.

A regression smoke on build 17e6c425, which bounds the sweep's storage steps and shutdown and retries its connection, passed on tmux and herdr on 2026-11-23: claude and hermes all seven steps, and codex, opencode and pi the idle, long-preview and settle-race steps. Deleting a member, then the fleet, while a pi Director preview was held behind a draft completed in 1.0–2.3 s with exit 0, left the preview held and the draft intact. The only Director delivery diagnostic in every run was the settle race's `literal_mismatch`.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-23 | Initial draft |
| 2026-11-23 | Reviewer round 1: residue check over tracked files, no-live-runner handling, stop and deadline clipping, `director_delivery` diagnostics source, multiplexer write methods, plain-only Director literal, composer-text segment model with fixtures and regression guard, mismatch recovery, loader rejections, probe setup and failure path |
| 2026-11-23 | Reviewer round 2: Director `plain_limit` measured on Director screens, no-live-runner wake wording and trade-off, `expected_literal` pre-submit fixture field with masked-row rule, Hermes classic `☤ ❯ ` prefix |
| 2026-11-23 | Implementation: measured composer limits on live screens, composer-text refinements (`mode_prefix`, `framed`, per-rule prefixes and wrap, bottom-most candidates), herdr `viewport_rows`, one-second pre-submit window, Director storage and shutdown bounds; live probe passed on all backends × tmux/herdr; user-approved. Status → Complete |
