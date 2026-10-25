# opencode v2 mini spawn, capture-cue and draft removal, and at-least-once preview delivery

**Status**: Complete
**Progress**: 54/54 tasks complete
**Last Updated**: 2026-10-25

## Overview

Spawn opencode members through the opencode v2 `opencode mini --standalone` subcommand and drop opencode v1 support entirely. Simplify Director supervision by removing the per-backend pane-state capture cues and every retained-draft procedure. Replace the durable preview claim with at-least-once notification: a pending message whose `notified_at` is NULL is simply retried, with exactly two outcomes (notified / not notified) and no `unconfirmed_previews`.

## Success Criteria

- [x] `cafleet member create --coding-agent opencode` spawns `opencode mini --standalone --agent cafleet [--model M] --prompt P`; an opencode v2.1.4 member reaches `ready`, receives a pushed `[cafleet msg …]` line as a submitted user turn, and polls/ACKs it.
- [x] No source, test, doc, skill or rule mentions opencode 1.x or the bare `opencode` TUI argv; every opencode behavior the docs assert is either observed on v2.1.4 by a Step 10 probe or stated as unverified; the docs state opencode 2 as the minimum supported version.
- [x] The embedded preset declares its command allowlist under the `shell` permission key, and `opencode debug agents` (v2.1.4) lists the allowlist under `shell` and every other preset permission key.
- [x] No backend section or Template in `skills/cafleet/reference/coding-agents.md` contains a `Pane-state capture cues` subsection; `.claude/rules/coding-agent-overlay.md` specifies four subsections.
- [x] `skills/cafleet/reference/supervision.md` carries one capture-reading rule (≈20-line default capture, `--lines` for more, empty composer ≠ completion) and no Idle Semantics evidence table; no retained-draft detection or preservation procedure remains in any skill, doc or SPEC.
- [x] Migration `V3__drop_preview_claims.sql` drops `messages.preview_started_at`; a V2 database upgrades in place with every row preserved, and the chain guard expects `[(1, baseline), (2, preview_attempts), (3, drop_preview_claims)]`.
- [x] `member list --json` and `member show --json` no longer emit `unconfirmed_previews`.
- [x] `message wait --json` notification diagnostics have the shape `{member_id, message_id, error}`. No `delivery` key, `not_sent` or `uncertain` classification appears on the message notification surfaces: `message send` / `message wait` output, preview notification diagnostics, and their docs, SPEC.md passages and tests. `member prompt` / `member exec` dispatch uncertainty and the multiplexer's `submission_started` classifier are a separate surface and stay.
- [x] Every preview notification failure emits the single not-notified diagnostic defined in § Notification diagnostic, `message wait` retains one diagnostic per not-notified message, and a later send or `message wait` re-notifies the message.
- [x] One `message send` or `message wait` invocation attempts each pending message's notification at most once.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

### opencode v2

The user upgraded opencode to v2.1.4. CAFleet 0.29.0 spawns opencode members with `opencode --agent cafleet [--model M] --prompt P`; the v2 top-level TUI rejects that (`Unrecognized flag: --agent in command opencode`, same for `--model`) and keeps only `--standalone`, `--server`, `--auto`, `--continue`, `--session`, `--prompt`. The v2 `opencode mini` subcommand (minimal interactive interface) accepts `--agent`, `--model`/`-m` (`provider/model`), `--prompt`, `--session`, `--continue`, `--fork`, `--replay`, `--replay-limit`, `--standalone` and `--server`. By default `mini` connects to a shared background service; `--standalone` runs a private server for the process.

Observed on opencode v2.1.4 (Director): the installed preset `~/.opencode/agents/cafleet.md` loads (`opencode debug agents` lists the `cafleet` agent, its bash rules shown under the v2 permission action name `shell`); `opencode debug config` lists `~/.config/opencode` and `~/.opencode` as config directories; `opencode models` lists the `opencode/` provider IDs used by the Role defaults.

### Capture cues and drafts

Each backend section of `skills/cafleet/reference/coding-agents.md` carries a `### Pane-state capture cues` table (idle / working / awaiting_user / draft / unknown, keyed on styling). The cues are consulted only by Director visual inspection, never by delivery or dispatch; `member capture` strips ANSI by default, so the styling they describe is not visible; they are near-identical across backends and break on every TUI change. Around them sit draft procedures (check for a retained draft before prompting, preserve retained drafts during recovery, serialize complete input). Member panes are spawned by CAFleet and humans rarely type into them, so losing a half-typed line is acceptable.

### Preview claim

Today delivery stamps `preview_started_at` (a durable claim, migration V2) before typing and `notified_at` after complete submission. An unresolved claim suspends automatic re-notification for that exact message across processes, surfaces in `member list` / `member show` as `unconfirmed_previews`, and the Director is expected to capture the pane and `member prompt` the member to poll. Failures are classified `not_sent` or `uncertain`. Every notification line carries the message ID and `message poll` + ACK are the source of truth, so a duplicate line is harmless while a suspended notification can stall a member.

---

## Specification

### Part 1 — opencode v2 `mini` spawn

#### Spawn argv

`Opencode::build_spawn_argv` (`cafleet/src/coding_agent/opencode.rs`) emits:

```
["opencode", "mini", "--standalone", "--agent", "cafleet"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

| Token | Reason |
|---|---|
| `mini` | The v2 top-level TUI rejects `--agent`/`--model`; `mini` is the v2 interactive interface that accepts them. |
| `--standalone` | Each member pane owns a private opencode server instead of sharing the background service (user decision). |
| `--agent cafleet` | Binds the member to the installed `cafleet` preset (unchanged spawn precondition). |
| `--model M` | Unchanged `<provider-id>/<model-id>` validation. |
| `--prompt P` | `mini` takes no positional message; the prompt stays a two-token flag pair. |

`validate_model`, `validate_effort`, `ensure_available` (PATH check then preset existence), the preset error string and the `{permission_flags}` runtime binding (`--agent cafleet`) are unchanged. No version detection or compatibility path exists: the argv above is the only opencode argv.

#### Preset (`presets/opencode/cafleet.md`)

- Rename the `permission` key `"bash"` to `"shell"` (v2 action name). The ruleset content and order are unchanged.
- Change the frontmatter `description` from "deny-by-default bash allowlist" to "deny-by-default shell allowlist".
- Change the body sentence "Bash defaults to deny" to "Shell defaults to deny".
- SPEC.md § *Exact preset file contents (verbatim)* mirrors the file and its prose ("`bash`/`read`/`edit` are glob→decision maps") changes to `shell`/`read`/`edit`. `docs/docs/spec/coding-agent-backends.md` § *The `cafleet` agent preset* says "The preset's `shell` ruleset".
- `docs/docs/concepts/coding-agents.md` comparison table, Sandbox isolation row, `opencode` cell: "not supported — a deny-by-default shell allowlist".

#### Minimum supported version

The docs state: "Validated against opencode 2.1.4; opencode 2 is the minimum supported version." This replaces "Validated against `opencode 1.15.5` (the minimum supported version)." in `docs/docs/spec/coding-agent-backends.md`. The paragraph "The pane runs the bare `opencode` TUI (not `opencode run`) … bare `opencode`'s positional is a project path, not a message." becomes: the pane runs `opencode mini --standalone`, the long-lived minimal interactive interface with a private server per pane, and the prompt travels through `--prompt` because `mini` takes no positional message.

#### Verification and open items

These are verified during implementation (Step 10) and recorded in both homes — agent-facing `skills/cafleet/reference/coding-agents.md` § opencode notes and human-facing `docs/docs/spec/coding-agent-backends.md` § Opencode — stating only observed results:

| # | Item | Probe | Docs that own the fact |
|---|---|---|---|
| P1 | `OPENCODE_CONFIG_DIR` honored | `OPENCODE_CONFIG_DIR=<scratch> opencode debug config` lists `<scratch>` as a config directory | Config-dir resolution (`cli-options.md` + SPEC.md §6.3), `quickstart.md`, `coding-agent-backends.md` § preset |
| P2 | Every preset permission key is a v2 permission name | Install the renamed preset into `<scratch>` (`cafleet setup --coding-agent opencode` with `OPENCODE_CONFIG_DIR=<scratch>`), run `opencode debug agents`, and confirm `shell`, `read`, `edit`, `external_directory`, `webfetch`, `websearch`, `repo_clone`, `question`, `plan_enter` and `plan_exit` each appear with the preset's decisions | Preset verbatim copy (SPEC.md), `coding-agent-backends.md` § preset and § Safety-floor caveats |
| P3 | Last matching rule wins | From the scratch member, attempt `cafleet member prompt <id> x`; observe a denial without a permission popup, while `cafleet message poll <id>` runs | `coding-agent-backends.md` § preset ("selects the **last** matching rule") and its SPEC.md copy |
| P4 | `mini --standalone` member spawn | Spawn one opencode member with the repo-built binary in a scratch fleet; observe its `ready` | `coding-agent-backends.md` spawn table and § Opencode; SPEC.md argv block |
| P5 | Push notification and `member prompt` into `mini` | Send a message from the scratch Director to the member; capture and the retained mini session establish the exact `[cafleet msg …]` user turn, poll and ACK. Then `member prompt` a short instruction and observe its exact user turn. When capture contains only the status bar, retain that limitation and use the session records for submission evidence. | `multiplexer-backends.md` § Prompt dispatch and § Accepted trade-offs and their SPEC.md copies |
| P6 | Leading-`!` native shell input in `mini` (used by `member exec`) | `cafleet member exec <id> "pwd"` into that member, then `member capture` | Listed below |

The owning living documentation states supported behavior and usage-relevant limits directly in the section that owns each fact. Probe IDs, procedures, dates, fixture/model details, parser-disambiguation steps and evidence provenance stay in this design document and retained verification reports. Configuration-source merging and capture visibility remain in the relevant specifications because they affect how users configure and supervise members.

Outcomes:

- **Observed as expected** — the owning docs state the observed v2.1.4 behavior.
- **Inconclusive** (P6 only) — the docs state leading-`!` input as unverified for `mini`; implementation completes.
- **Contradicted** (any probe) — implementation stops at that probe. The executing team reports the observed behavior to the Director, who relays it to the user. The design doc is revised before dependent docs or code proceed: a renamed permission key (P2) or a changed rule-precedence (P3) means a preset/ruleset change; an ignored `OPENCODE_CONFIG_DIR` (P1) or a failed spawn/delivery (P4, P5) means an argv or delivery change. A conclusive P6 negative leaves `member exec` without a native path for opencode members (the preset denies every unlisted command), so the user chooses the replacement routing path before completion.

P6 rewrites, from the observed result, every text that asserts leading-`!` input for opencode today:

| Home | Text |
|---|---|
| `skills/cafleet/reference/coding-agents.md` § opencode notes | The row "The TUI documents leading `!` native shell input …" and its v1 provenance link (`https://docs.opencode.ai/docs/tui/#shell-commands`) |
| `docs/docs/spec/coding-agent-backends.md` § Spawn argv | The shared-contract bullet "All three honor the leading-`!` shell shortcut …" |
| `docs/docs/spec/coding-agent-backends.md` § Opencode | "The TUI documents leading `!` input and conversation-visible output; an actual OpenCode native-input permission probe was outside the investigation." |
| `docs/docs/concepts/coding-agents.md` | "All three honor a leading-`!` shell shortcut on the coding agent's input line …" |
| SPEC.md § backend notes | The **opencode** paragraph's "The TUI documents leading `!` input …" sentence |

A v2 provenance link replaces the v1 one only when an official v2 page documents the behavior. Probes run in a scratch fleet and a scratch `OPENCODE_CONFIG_DIR` with the repo-built binary, so the active executing fleet's installed runtime and assets stay intact.

#### Herdr literal input correction from live verification

The initial P5/P6 run on herdr 0.9.5 retained a literal `-- ` prefix on every delivered input. `herdr pane send-text --help` specifies `<PANE_ID> <TEXT>`; CAFleet supplied the extra positional argument `--`. The malformed `-- ! pwd` became a conversational turn. A Director-owned diagnostic with exactly `herdr pane send-text PANE "! pwd"` followed by Enter produced a native shell record with command ` pwd`, exit 0 and the repository path on opencode 2.1.4. The observed defect belongs to herdr argument construction; this result requires no replacement native-command routing choice.

Herdr literal writes pass the validated payload as one TEXT argument: `["herdr", "pane", "send-text", PANE, TEXT]`. Tmux retains its own `--` option terminator. The shared input guard, Escape lead, settling, Enter, operation deadline and dispatch-result semantics stay intact. The transport table and its SPEC.md copy state each backend's exact argv. Tests assert exact payload preservation for leading hyphens, metacharacters and `! ` native input. Rebuild and freeze the corrected repo runtime, retain its hash, and rerun P5/P6 through that runtime before recording the final observed result. Preserve the original malformed-input evidence separately.

### Part 2 — Remove per-backend pane-state capture cues

| Location | Change |
|---|---|
| `skills/cafleet/reference/coding-agents.md` | Delete `### Pane-state capture cues` from `## claude`, `## codex`, `## opencode` and `## Template`. Delete the "Interpret a captured member pane" row of the top reader table (two rows remain), the sentence "A Claude Director inspecting Codex applies the target backend’s capture cues.", "pane cues" from the runtime-maintenance ownership sentence, and the Template's "Bind the pane-cue table to Director capture inspection …" sentence. |
| `.claude/rules/coding-agent-overlay.md` | Specify **four** subsections (drop item 5). Delete the "Interpret a captured pane" reader-contract row and the example sentence "A Claude Director inspecting Codex applies the target backend’s capture cues while keeping its own tools." "Add a backend by copying `## Template` and supplying all four subsections." |
| `skills/cafleet/SKILL.md` § Resolve your overlay | Delete the step-1 sentence "For captured panes, use the observed member's recorded backend and its Pane-state capture cues." and the example sentence about inspecting Codex. |
| `skills/cafleet-design-doc/{create,interview,execute}/*.md` | Delete "Interpret captured panes using the observed member's backend cues." and "; observed backend owns pane cues". |
| `.claude/skills/clean-docs/SKILL.md` | Delete the capture-cue resolution clauses (shared spine and per-workflow resolution sentences). |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | Delete "pane-state capture cues" / "pane cues" from the preserved-runtime-documentation lists. |
| `cafleet/tests/docs_sync.rs` | Delete the `pane-state-capture-cues` required-subsection entry and the cue-table assertions. |

The human-facing docs carry no cue table; their equivalents are handled in Part 3.

### Part 3 — Capture-reading rule and draft removal

Replace `skills/cafleet/reference/supervision.md` § Idle Semantics (prose plus evidence table) with:

```markdown
## Reading captures

Capture about 20 lines with `cafleet member capture <member-id>` (its default) and read the content directly; capture more with `--lines` when it is insufficient. Captures guide your next action only; delivery and dispatch never consult them. An empty composer does not establish task completion — judge completion from messages and artifacts.

A timeout, idle duration or unread count alone proves no failure. Expected ready/reply deadlines belong to the assignment and are checked by the Director.
```

Every `#idle-semantics` link points to `#reading-captures`. Draft-handling removals:

| Location | Removed text |
|---|---|
| `skills/cafleet/roles/director.md` § Member Prompt | "Check a capture for a retained draft before prompting, and relay a user draft before replacing it." |
| `skills/cafleet/reference/supervision.md` § Recovery | "Preserve retained drafts and relay them to the user." and "Persistently unknown input stays untouched while inspected or relayed." |
| `skills/cafleet/reference/coding-agents.md` § claude notes | The row "Composer controls act at the beginning of submitted input; a local probe merged control text into a concurrent preview. Serialize complete input and inspect its capture." |
| `docs/docs/spec/coding-agent-backends.md` and SPEC.md (§ claude backend notes) | The two sentences "Native composer controls act at the beginning of submitted input. A local probe merged control text into a concurrent preview, so serialize complete input and inspect capture evidence." The native-shell-probe sentences stay. |
| `docs/docs/concepts/member-lifecycle.md` | "Preserve retained user drafts and relay them to the user." replaced by the human-facing capture rule: capture about 20 lines, read the content, use `--lines` for more; an empty composer does not mean the task is complete. |
| `cafleet/tests/docs_sync.rs` | Draft-phrase assertions ("retained draft check before prompting", "retained draft relay", "a retained draft can merge with a delivery"). |

The multiplexer known-limitation bullet ("Escape does not reliably clear typed composer text on every host. A draft in an ordinary member's pane can merge with the delivered line and be submitted with it.") in `docs/docs/spec/multiplexer-backends.md` and SPEC.md is a statement of transport behavior, not a procedure; it stays, reworded to "Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line."

### Part 4 — At-least-once preview delivery

#### Delivery model

| Concept | Rule |
|---|---|
| Pending notification | Message is `input_required`, non-self, addressed to an active ordinary member with a concrete pane, and `notified_at IS NULL` (existing `PENDING_PREVIEW`). |
| Outcome | Exactly two: **notified** (`notified_at` stamped after complete submission while the message is pending, or the message was ACKed meanwhile) or **not notified** (`notified_at` stays NULL). |
| Retry trigger | The next `message send` to that recipient (its drain) or a `message wait` rotation. |
| Per-invocation bound | One `message send` or `message wait` invocation attempts each pending message's notification at most once; a failed attempt is retried by a later invocation. |
| Duplicates | Acceptable: a line whose submission partially succeeded may be typed again. Every line carries the message ID; `message poll` + ACK are authoritative. |
| Director | Unchanged: root-Director messages receive no pane input. |

#### Drain procedure

The drain keeps the existing seven-step structure in `docs/docs/spec/multiplexer-backends.md` § Push notifications and its SPEC.md copy; both copies carry the text below, the SPEC.md copy in its existing link-free form. Steps 1–3 keep their current text; steps 4–7 and the two paragraphs after them replace the current steps 4–7, the lock-hold sentence and the claim paragraph:

```markdown
1. Take the eligibility snapshot. An inactive/ineligible recipient, a fleet mismatch or no pending message ends the attempt with nothing to deliver. A placement without a concrete pane holds delivery.
2. Wait for the [pane input lock](#pane-input-lock).
3. Under the lock, re-read the snapshot; a changed placement holds delivery. Resolve the multiplexer and check its availability.
4. Re-read the recipient and select its oldest pending message that this invocation has not already attempted. A departed recipient or one now serving as root Director ends the drain, and a placement change holds it. The drain ends when no such message remains.
5. When the remaining budget no longer covers the lead's reservation plus a 0.5-second minimum literal budget, stop. For the first message this fails with the not-notified diagnostic; later messages stay pending with no diagnostic.
6. Submit its line, with the Escape lead for the first line and the direct lead afterwards. A submission failure ends the drain with the not-notified diagnostic for that message.
7. Stamp `notified_at` while the message remains pending, then continue at step 4. A message the recipient ACKed before the stamp counts as notified. A stamp failure ends the drain with the not-notified diagnostic for that message.

The lock is held from placement revalidation through the last Enter and the last `notified_at` stamp of the drain.

Delivery is at-least-once. Nothing is written before input: a message stays pending, with `notified_at` NULL, until its complete submission is stamped. Every failure leaves the message pending and ends the drain with one not-notified diagnostic. The next send to that recipient or a later `message wait` invocation retries it, so a partially or fully submitted line can appear twice; every line carries its message ID, and `message poll` plus ACK remain authoritative. Mandatory finalization uses the later of the original deadline and one second after finalization begins. The Director consumes its own pending records through broker reads.
```

The per-message revalidation of the deleted claim step lands in step 4 (one read snapshot under the lock). "This invocation has not already attempted" is the `attempted` exclusion; "ends the drain" maps to `AttemptEnd::Failed`.

Code mapping in `cafleet/src/delivery.rs`:

- `deliver_pending(conn, settings, member_id, expected_fleet, deadline, attempted, resolve)` gains `attempted: &BTreeSet<i64>` — the message IDs this invocation has already attempted and failed. `message send` passes an empty set.
- Step 4 is one `read_snapshot` returning the member and `broker::oldest_pending_preview(conn, member_id, attempted)`: ineligible → `NoPending`, placement changed → `Held`, no candidate → `Drained`.
- Step 7 uses `stamp_notified_validated` (`UPDATE … SET notified_at WHERE message_id=? AND PENDING_PREVIEW`); a zero-row update counts as notified.
- Errors split into two classes:

| Class | Members | Result |
|---|---|---|
| Per-attempt failure | Lock busy or lock acquisition error, multiplexer resolution/availability error, insufficient budget, transport error, and `CafleetError::DatabaseBusy` / `CafleetError::DeadlineExceeded` from the under-lock snapshot read or the stamp | `AttemptEnd::Failed` naming the current candidate — before step 4 has selected one, the message from the step-3 snapshot — with `sent` preserved |
| Fatal error | Every other `CafleetError`: corruption, schema/format mismatch, unreadable rows, and a termination-span start failure | `deliver_pending` returns `Err` with its cause |

- Fatal errors keep today's command contracts. `message send` prints the existing `Message NEW_ID was persisted. CAUSE` and keeps its committed stdout record. `message wait` returns the error through `attach_diagnostics` (every retained not-notified diagnostic attached) and exits 1, as cli-options/SPEC.md state ("Corruption and other fatal storage errors retain their causes"; "corrupt/unreadable state exits 1 with its cause").

Deleted: `claim_current_preview`, `PreviewClaim`, `ClaimSkip`, the "earlier candidate appeared" hold, `messaging::claim_preview`, `messaging::release_preview`, claim-release retry and the "remains suspended" text. `broker::oldest_pending_preview` drops `preview_started_at IS NULL` and accepts the `attempted` exclusion. The `submission_started` flag on `MultiplexerError` stays (used by `member prompt` / `member exec`).

#### `message wait` rotation

- `broker::panes::pending_previews` returns `(member_id, message_id)` pairs; `PendingPreview.started` is deleted.
- `receive::wait` keeps `attempted: BTreeSet<i64>`; each `AttemptEnd::Failed` inserts its message ID. Rotation eligibility excludes pending entries in `attempted`; a member is eligible while it has a non-attempted pending entry. `attempt()` forwards `attempted` to `deliver_pending`.
- An `io.attempt` error of kind `DatabaseBusy` / `DeadlineExceeded` (`receive.rs`, today a raw `error.to_string()`) becomes a failure for the selected `preview.message_id`, built with the same `delivery.rs` helper, and that ID enters `attempted`.
- Retained diagnostics are keyed by `message_id` (`BTreeMap<i64 message_id, NotificationError>`): one diagnostic per not-notified message, emitted in ascending message-ID order. Because each message is attempted at most once per invocation, no diagnostic replaces another.
- Reconciliation is per message: an entry is removed when a successful snapshot shows that message ACKed, its recipient gone, or the message notified by any invocation, and when this invocation's drain notifies it.
- Deleted: the synthetic "an earlier input attempt has no confirmed notification stamp" error, the `started` filter and the uncertainty-based eligibility filter.
- `preserve_uncertainty` becomes `attach_diagnostics`: a fatal receive error carries every retained notification diagnostic as `CafleetError::WithDiagnostics`.
- Kept: one member per one-second interval and the round-robin cursor.

`docs/docs/spec/cli-options.md` § message wait and its SPEC.md copy change accordingly: "Keep only the latest error per member in memory." becomes "Keep one diagnostic per not-notified message, ordered by message ID."; "A drain that delivers a member's recorded message removes that member's entry." becomes "A drain that notifies a recorded message removes that message's entry."; "a later error replaces the previous one" is deleted; "Pending placement and a placement change at claim time are ordinary holds" becomes "Pending placement and a placement change under the lock are ordinary holds".

#### Notification diagnostic

`receive::Delivery` and `NotificationError.delivery` are deleted. Every preview notification failure produces one diagnostic, built by one helper in `cafleet/src/delivery.rs`:

```
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| Pane input lock still busy after the lock wait | `pane input remained busy` |
| Lock acquisition error | `pane input: <lock error>` |
| Multiplexer resolution/availability error | the multiplexer error text |
| Budget cannot cover literal input, settle and Enter before the first line | `insufficient pane input budget for literal input, settle and Enter` |
| Transport error (before or during typing) | the transport error text |
| `DatabaseBusy` / `DeadlineExceeded` from the under-lock snapshot read or the `notified_at` stamp | the error text; names the current candidate per § Drain procedure |
| `message wait` attempt error of kind `DatabaseBusy` / `DeadlineExceeded` before `deliver_pending` selects a candidate | the error text; names the selected rotation entry, which enters `attempted` |

Only per-attempt failures (§ Drain procedure) produce this diagnostic; fatal errors keep their command contracts unchanged. `receive.rs` builds its row through the same `delivery.rs` helper. The diagnostic reports this invocation's outcome; the message stays eligible for retry.

The old lock-busy string `member ID's pane input remained busy; preview MESSAGE_ID was not sent.` and its `not_sent` label are replaced by the not-notified diagnostic with `CAUSE` = `pane input remained busy` at:

| Location | Current text |
|---|---|
| `docs/docs/spec/multiplexer-backends.md` § Pane input lock, per-writer table, `message send` / `message wait` row, and its SPEC.md copy | `` `not_sent`: `member ID's pane input remained busy; preview MESSAGE_ID was not sent.` `` |
| The sentence after that table, and its SPEC.md copy | "one short read selects the oldest pending unclaimed message" → "one short read selects the oldest pending message this invocation has not attempted" |
| `docs/docs/spec/cli-options.md` § message wait (budget paragraph), and its SPEC.md copy | "a lock still busy then adds a `not_sent` entry with `member ID's pane input remained busy; preview MESSAGE_ID was not sent.` to `notification_errors`" → "a lock still busy then records the not-notified diagnostic in `notification_errors`" |
| `docs/docs/spec/cli-options.md` § message send delivery-outcome table, and its SPEC.md copy | Rows become the `CAUSE` table above; "classified `not_sent` or `uncertain`" is deleted |
| `cafleet/tests/docs_sync.rs` (assertion requiring the old string verbatim) | Requires the new diagnostic template instead |

| Surface | Output |
|---|---|
| `message send` stderr | `Message NEW_ID was persisted. ` + diagnostic (prefix unchanged) |
| `message wait` text | diagnostic lines on stderr |
| `message wait --json` | `"notification_errors": [{"member_id":11,"message_id":102,"error":"<diagnostic>"}]` — key order `member_id`, `message_id`, `error` |

`member prompt` / `member exec` diagnostics (including their "is uncertain … Inspect the pane before retrying." strings) are out of scope and unchanged.

#### Schema migration

`cafleet/migrations/V3__drop_preview_claims.sql`:

```sql
ALTER TABLE messages DROP COLUMN preview_started_at;
```

- Same `cafleet_v8.db` family and application identity; `V1__baseline.sql` and `V2__preview_attempts.sql` stay in the chain.
- Bundled SQLite (`rusqlite` `bundled`) supports `DROP COLUMN`; the column has no index, constraint or reference.
- Rows with a former claim and `notified_at IS NULL` become ordinary pending notifications and are retried after upgrade.
- `cafleet/src/db/mod.rs`: the chain test expects `[(1, "baseline"), (2, "preview_attempts"), (3, "drop_preview_claims")]`; the head `messages` shape drops `("preview_started_at", "TEXT", 0, 0)`; tests reading `preview_started_at` are rewritten.
- An older binary against a V3 database refuses it as an ahead version (existing compatibility gate).
- SPEC.md §8 DDL already shows `messages` without `preview_started_at`; it stays as is.

#### Output removal

- `MemberActivity.unconfirmed_previews` (`broker/records.rs`), `broker::members::unconfirmed_previews`, and its population in `list_members` are deleted.
- `presentation::member_activity` and `presentation::member_detail` drop the key; `member_detail` loses its second parameter; `cli/member.rs::show` stops querying it. The text outputs are unchanged (they never showed the field).

#### Sender follow-up (decision)

Kept, reduced to its retry trigger. With no claim, the follow-up's own drain re-notifies `N` first, so one follow-up guarantees a retry when the sender would otherwise never send again (a Director awaiting a reply would deadlock). The durable-claim branch and the "inspect the target pane and `member prompt`" escalation go.

New `supervision.md` § Sender follow-up:

```markdown
When an ordinary-recipient `cafleet message send` prints `Message N was persisted. …` on stderr, send exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. Its drain retries `N` first. When the follow-up also prints a persisted diagnostic, send no further follow-up: a later send or `message wait` retries both, and the overdue-reply check in Director wake covers a recipient that stays silent.
```

`member.md` (send one follow-up; end the turn when it also persists) stays unchanged. SPEC.md and `cli-options.md` § message send keep "When the follow-up also fails, the sender stops." and replace "For an unclaimed message its drain delivers `N` first; for a durable claim its line wakes the recipient, whose poll lists `N`." with "Its drain retries `N` first."

#### Documentation changes for Part 4

| Location | Change |
|---|---|
| `skills/cafleet/reference/supervision.md` § Director wake | Step 4: run `member list --json` and check assignments past their expected reply point, with captures and artifacts as evidence. Step 5 (per-ID capture and `member prompt`) is deleted; its last two sentences (consume own pending IDs through broker reads; finite `wait` for retry rotation; the listener performs no preview retries) join step 4. |
| `supervision.md` § Team-facilitation step 4, § Escalation | Drop `unconfirmed_previews`; Escalation's "Pending previews and `unconfirmed_previews` entries do not count as delivered." becomes "Pending notifications do not count as delivered." |
| `skills/cafleet/roles/director.md` | § Member List: "use the `idle` column and overdue assignments instead of capturing every member". § Director wake summary: drop "ordinary-member `unconfirmed_previews`,". |
| `skills/cafleet-design-doc/{create,interview,execute}/*.md` | Drop "ordinary-member `unconfirmed_previews`," from the reception paragraph. |
| `docs/docs/how-to/mixed-backend-team.md` | Drop the `unconfirmed_previews` clause. |
| `docs/docs/spec/cli-options.md` + SPEC.md | Member list: remove the field, its definition paragraph and JSON example line; member show: remove the trailing key. Message send: delivery-outcome table uses the not-notified diagnostic; follow-up claim clause removed. Message wait: rotation paragraph without claims, at-most-once rule, diagnostic shape `{member_id, message_id, error}`, "retains earlier notification diagnostics". Message listen: "leaves consumption and `notified_at` unchanged". SPEC.md also: § storage validation "notification stamp operations"; roster derivation sentence; § pane-input budget sentence on persisting claims removed; §9 testing strategy keeps its wording. |
| `docs/docs/spec/data-model.md` | Messages columns: only `notified_at` is nullable notification state. Preview paragraph rewritten to the delivery model above. Roster paragraph drops `unconfirmed_previews`. Inbox-stream sentence: "independently of `notified_at`". |
| `docs/docs/spec/message-envelope.md` | "internal `notified_at` state". |
| `docs/docs/spec/multiplexer-backends.md` + SPEC.md | Lock table and lock-wait sentence per § Notification diagnostic. Termination-span row: "From lock acquisition through the last `notified_at` stamp". SIGKILL table row "between claim and stamp" → "`message send` / `message wait` between first keystroke and stamp \| `notified_at` stays NULL; the line may or may not have reached the pane \| The next send or `message wait` retries it; a duplicate line is possible". Push-notification steps and paragraphs per § Drain procedure. § Held delivery per the block below. § Accepted trade-offs gains the duplicate-line bullet below. |

§ Held delivery, replacing its first paragraph (both copies):

```markdown
Pending placement and a placement change under the lock are holds. Lock contention waits for the lock within the attempt budget. A message remains pending while the lock stays busy, the pane is not yet placed, the budget runs out or an attempt fails. The next send to that ordinary recipient, or a message wait rotation, retries it. Rotation selects concrete ordinary-member placements, excluding the root Director and the wait's own subject, and attempts at most one member per interval. One invocation attempts each pending message at most once; a later invocation retries a failed one. Successful snapshots can establish ACK, departure or notification by another invocation.
```

§ Accepted trade-offs, new bullet (both copies): "A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID."

### Residue sweep

Steps 4 and 10 run this from the repository root:

```bash
git grep -n -i -E 'preview_started_at|unconfirmed_previews|capture cue|pane cue|backend cue|pane-state|retained draft|user draft|idle.semantics|serialize complete input|not_sent|uncertain|Delivery::|preserve_uncertainty|notification claim|unclaimed|durable claim|claim_preview|release_preview|suspended|1\.15\.5|opencode --agent|"opencode", "--agent"|bare .opencode|opencode run|bash allowlist|.bash. ruleset|.bash./.read|"bash":' -- . ':!design-docs'
```

| Allowed match | Why it stays |
|---|---|
| `member prompt` / `member exec` dispatch uncertainty (`cli-options.md` and SPEC.md § member prompt / member exec, § Prompt dispatch "is uncertain", `prompt-routing.md`, `cli/member.rs`, the docs_sync native-exec contract test) | Separate, non-retried surface; out of scope |
| The multiplexer `submission_started` classifier and its tests under `cafleet/src/multiplexer/` | Feeds `member prompt` / `member exec` |
| `cafleet/migrations/V2__preview_attempts.sql`, `V3__drop_preview_claims.sql`, the chain-test slugs in `cafleet/src/db/mod.rs`, and the storage_format V2-upgrade fixture | The migration chain itself |
| A match whose meaning is unrelated to notification delivery, opencode or pane cues | Judged individually by the Reviewer |

Every other match is residue to remove.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first (docs/, README, SPEC, skills, rules), then code, per `.claude/rules/documentation-maintenance.md`. Agent-facing reference and human-facing docs are edited independently, with no cross-links.

### Step 1: Human-facing docs (`docs/`)

- [x] `docs/docs/spec/coding-agent-backends.md`: opencode argv row, `mini --standalone` paragraph, preset `shell` ruleset, minimum-version sentence, claude composer-serialization sentences removed; leading-`!` texts (P6 table) state the behavior as unverified for `mini` until Step 10 <!-- completed: 2026-10-25T05:12 -->
- [x] `docs/docs/concepts/coding-agents.md`: Sandbox isolation row says "a deny-by-default shell allowlist"; the leading-`!` sentence states the behavior as unverified for `mini` until Step 10 <!-- completed: 2026-10-25T05:12 -->
- [x] `docs/docs/concepts/member-lifecycle.md`: replace the retained-draft sentence with the capture-reading rule <!-- completed: 2026-10-25T05:12 -->
- [x] `docs/docs/spec/cli-options.md`: member list/show, message send, wait and listen per Part 4 <!-- completed: 2026-10-25T05:12 -->
- [x] `docs/docs/spec/data-model.md` and `docs/docs/spec/message-envelope.md`: drop `preview_started_at`, claim and `unconfirmed_previews` <!-- completed: 2026-10-25T05:13 -->
- [x] `docs/docs/spec/multiplexer-backends.md`: drain, lock, failure table and retry rules per Part 4; reword the composer-text limitation bullet <!-- completed: 2026-10-25T05:13 -->
- [x] `docs/docs/how-to/mixed-backend-team.md`: drop the `unconfirmed_previews` clause <!-- completed: 2026-10-25T05:13 -->

### Step 2: README.md and SPEC.md

- [x] Run the `/update-readme` reconciliation; confirm README.md needs no change (it carries no affected surface) <!-- completed: 2026-10-25T05:14 -->
- [x] SPEC.md: opencode argv block and prose, verbatim preset copy (`shell`), claude composer sentences removed, composer limitation bullet reworded <!-- completed: 2026-10-25T05:14 -->
- [x] SPEC.md: storage validation, roster, member list/show, message send/wait/listen, drain, lock, failure table, retry and pane-input budget passages per Part 4 <!-- completed: 2026-10-25T05:15 -->

### Step 3: Agent-facing skills

- [x] `skills/cafleet/reference/coding-agents.md`: delete every cue subsection and the Template one, the reader-table row, example and ownership mentions; delete the claude composer note row; the opencode leading-`!` note row states the behavior as unverified for `mini` until Step 10 <!-- completed: 2026-10-25T05:17 -->
- [x] `skills/cafleet/SKILL.md` § Resolve your overlay: delete captured-pane step text and example <!-- completed: 2026-10-25T05:17 -->
- [x] `skills/cafleet/reference/supervision.md`: Reading captures section, Director wake steps 4–5, Sender follow-up, Team-facilitation, Escalation, Recovery; retarget `#idle-semantics` links <!-- completed: 2026-10-25T05:17 -->
- [x] `skills/cafleet/roles/director.md`: Member List, Member Prompt draft sentence, Director wake summary <!-- completed: 2026-10-25T05:18 -->
- [x] `skills/cafleet-design-doc/{create,interview,execute}` bodies and roles: drop pane-cue and `unconfirmed_previews` mentions <!-- completed: 2026-10-25T05:18 -->
- [x] `.claude/skills/clean-docs/SKILL.md` and `.claude/skills/cafleet-model-list-refresh/SKILL.md`: drop capture-cue mentions <!-- completed: 2026-10-25T05:18 -->

### Step 4: Project rules

- [x] `.claude/rules/coding-agent-overlay.md`: four subsections, two reader-contract rows, example sentence trimmed <!-- completed: 2026-10-25T05:19 -->
- [x] Documentation sweep: § Residue sweep with the pathspec narrowed to `docs SPEC.md README.md skills .claude`; every match is an allowed match <!-- completed: 2026-10-25T05:19 -->

### Step 5: opencode spawn and preset

- [x] Rewrite `spawn_argv_uses_the_prompt_flag_pair_and_ignores_display_name` to pin `opencode mini --standalone --agent cafleet [--model M] --prompt P` (test first) <!-- completed: 2026-10-25T05:23 -->
- [x] Update `build_spawn_argv` and the module doc comment in `cafleet/src/coding_agent/opencode.rs` <!-- completed: 2026-10-25T05:24 -->
- [x] `presets/opencode/cafleet.md`: `bash` → `shell` key, description and body wording <!-- completed: 2026-10-25T05:25 -->
- [x] Update `cafleet/tests/docs_sync.rs` preset/SPEC verbatim checks for the renamed key <!-- completed: 2026-10-25T05:25 -->

### Step 6: Migration V3

- [x] Test first: chain test expects V3; head-shape test drops `preview_started_at`; storage_format test upgrades a V2 database containing a claimed-unstamped pending row and asserts every row survives and the column is gone <!-- completed: 2026-10-25T05:29 -->
- [x] Add `cafleet/migrations/V3__drop_preview_claims.sql` <!-- completed: 2026-10-25T05:29 -->
- [x] Update the expected chain registry and head shape in `cafleet/src/db/mod.rs`; rewrite `db/mod.rs` tests that read `preview_started_at` <!-- completed: 2026-10-25T05:29 -->

### Step 7: At-least-once delivery

- [x] Test first (`cafleet/tests/notification_retry.rs`, `cafleet/tests/delivery.rs`): a failed preview (busy lock, transport failure before typing, failure after typing began, stamp failure) leaves `notified_at` NULL and the next send re-notifies it; the formerly-upgraded claimed row is re-notified <!-- completed: 2026-10-25T05:46 -->
- [x] Test first (`receive.rs` unit tests, `cafleet/tests/message_wait.rs`): one wait attempts each pending message at most once; a later message for the same member stays eligible; a later wait retries the failed one; reconciliation removes entries on ACK, departure or external notification <!-- completed: 2026-10-25T05:46 -->
- [x] Test first: every per-attempt failure path emits the exact not-notified diagnostic; `message wait --json` errors are `{member_id, message_id, error}`; a fatal storage error in the under-lock snapshot or stamp makes `message wait` exit 1 with its cause and attached diagnostics, and makes `message send` print `Message NEW_ID was persisted. CAUSE` <!-- completed: 2026-10-25T05:46 -->
- [x] `cafleet/src/broker/messaging.rs`: delete `claim_preview`, `release_preview`; `oldest_pending_preview` drops the claim filter and accepts the `attempted` exclusion <!-- completed: 2026-10-25T05:46 -->
- [x] `cafleet/src/broker/panes.rs`: `pending_previews` returns `(member_id, message_id)` <!-- completed: 2026-10-25T05:46 -->
- [x] `cafleet/src/delivery.rs`: drain procedure per § Drain procedure, `attempted` parameter, diagnostic helper; delete claim types and claim tests <!-- completed: 2026-10-25T05:47 -->
- [x] `cafleet/src/receive.rs`: delete `Delivery`, `NotificationError.delivery`, `PendingPreview.started`, synthetic no-stamp error; add `attempted` set; `attach_diagnostics` <!-- completed: 2026-10-25T05:47 -->
- [x] `cafleet/src/runtime` and `cafleet/src/cli/message.rs`: forward `attempted`; JSON diagnostics without `delivery` <!-- completed: 2026-10-25T05:47 -->
- [x] Rewrite `cafleet/src/listen_tests.rs`, `cafleet/tests/message_listen.rs`, `cafleet/tests/pane_input.rs`, `cafleet/tests/cli_message.rs` and `cafleet/tests/common/receive.rs` fixtures that set `preview_started_at` or `Delivery`; delete tests that exist only for claim suspension <!-- completed: 2026-10-25T05:51 -->

### Step 8: Member list and show output

- [x] Test first: `member list --json` and `member show --json` key sets equal the current contract without `unconfirmed_previews` (`cafleet/tests/cli_member.rs`, `broker/members.rs` tests, and the `storage_format::member_output_uses_registration_fields_and_the_director_back_reference` assertion); rerun the two deferred Step 6 member-show tests <!-- completed: 2026-10-25T05:58 -->
- [x] Delete `MemberActivity.unconfirmed_previews`, `broker::members::unconfirmed_previews`, its roster population, and the key in `presentation::member_activity` / `member_detail`; simplify `cli/member.rs::show` <!-- completed: 2026-10-25T05:57 -->

### Step 9: Docs-sync assertions

- [x] `cafleet/tests/docs_sync.rs`: delete cue-subsection and draft-phrase assertions; assert the four-subsection backend structure and the Reading captures rule <!-- completed: 2026-10-25T06:03 -->
- [x] `cafleet/tests/docs_sync.rs`: update claim/`unconfirmed_previews`/`uncertain` phrase assertions to the at-least-once wording <!-- completed: 2026-10-25T06:03 -->
- [x] `cafleet/tests/docs_sync.rs`: replace the verbatim old lock-busy preview string with the not-notified diagnostic template <!-- completed: 2026-10-25T06:03 -->

### Step 9a: Herdr literal input correction from live verification

- [x] `docs/docs/spec/multiplexer-backends.md` and SPEC.md: specify herdr `pane send-text PANE TEXT` and scope the `--` terminator explanation to tmux <!-- completed: 2026-10-25T06:27 -->
- [x] Herdr unit and pane-input integration tests: assert one exact TEXT argument, including leading hyphens, metacharacters and native `! ` input <!-- completed: 2026-10-25T06:34 -->
- [x] `cafleet/src/multiplexer/herdr.rs`: pass the payload directly after the pane ID; preserve shared guard, timing and dispatch semantics <!-- completed: 2026-10-25T06:34 -->

### Step 10: Verification and live probes

- [x] `mise //cafleet:test` <!-- completed: 2026-10-25T06:44 -->
- [x] `mise //cafleet:lint` <!-- completed: 2026-10-25T06:09 -->
- [x] `mise //cafleet:typecheck` <!-- completed: 2026-10-25T06:09 -->
- [x] `mise //docs:build`; inspect rendered tables touched in Parts 2–4 <!-- completed: 2026-10-25T07:05 -->
- [x] Probe P1: `OPENCODE_CONFIG_DIR` honored by `opencode debug config` <!-- completed: 2026-10-25T05:16 -->
- [x] Probe P2: install the renamed preset into the scratch config dir; `opencode debug agents` lists `shell` and every other preset permission key with its decisions <!-- completed: 2026-10-25T05:56 -->
- [x] Probe P4: in a scratch fleet using the repo-built binary, spawn an opencode member with `mini --standalone`; confirm `ready` <!-- completed: 2026-10-25T06:26 -->
- [x] Probe P3: the scratch member's `cafleet member prompt` attempt is denied without a popup while `cafleet message poll` runs <!-- completed: 2026-10-25T06:26 -->
- [x] Probe P5: a pushed message reaches the member as a submitted user turn and is polled/ACKed; a `member prompt` is submitted <!-- completed: 2026-10-25T06:41 -->
- [x] Probe P6: `cafleet member exec` with leading-`!` input and `member capture` <!-- completed: 2026-10-25T06:41 -->
- [x] Record every probe result in the owning docs per § Verification and open items; on a contradicted probe, stop and report it to the Director <!-- completed: 2026-10-25T06:50 -->
- [x] Rerun § Residue sweep repository-wide; every match is an allowed match <!-- completed: 2026-10-25T06:52 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-25 | Initial draft |
| 2026-10-25 | Live probes isolated the herdr literal-input prefix defect; add its transport correction and exact-input rerun, and retain session evidence when mini capture exposes only the status bar. |
| 2026-10-25 | Completed all 54 implementation tasks and 11 success criteria. Independent review approved after one documentation revision; all required checks and live probes passed. Published [PR #493](https://github.com/himkt/cafleet/pull/493). The user authorized committing this completed design document; audit artifacts remain outside version control. |
