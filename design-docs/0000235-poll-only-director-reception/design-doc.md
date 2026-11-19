# Poll-Only Director Reception

**Status**: Approved
**Progress**: 16/30 tasks complete
**Last Updated**: 2026-11-19

## Overview

Every Director wake consumes through `cafleet message poll`. A `message listen` `pending` record carries the exact consuming command in a `next` field instead of message IDs, the consumed-watermark skip procedure leaves every Director instruction, and the long-body member preview pointer names the recipient's `poll` command. The ID-only notice invited `message show`, which leaves messages pending and stalls later notices behind the listener's hold.

## Success Criteria

- [ ] `message listen` emits `pending` records of exactly `{"event":"pending","next":"<command>"}` (key order `event`, `next`), where `<command>` is `cafleet message poll <MEMBER_ID> --json`, followed by ` --database-url <url>` exactly when the listener was started with `--database-url`
- [ ] The `--codex-thread` forwarder queues `[cafleet listen record] <that same compact record>`
- [ ] Notice hold, ceiling re-notice, startup replay, lease, idle cadence and the Codex refused-notice retry behave as before (existing hold tests pass against the internal pending-ID decision)
- [ ] `ready` and `closed` records are unchanged
- [ ] No Director instruction (director.md, every backend reception recipe in coding-agents.md, `cli.md`, the hermes/opencode/pi `cafleet_listen` descriptions, `SPEC.md`, `docs/`) mentions a consumed watermark or tells the Director to skip a notice; every notice triggers exactly one poll, and an empty poll (`[]`) is described as the normal outcome for a stale notice
- [ ] No contract or agent-facing sentence describes a notice as naming, listing or carrying message IDs
- [ ] The long-body preview pointer reads `Body is N characters; run cafleet message poll <recipient-id> --json to read it.` for every database
- [ ] `message show` behavior and output are unchanged
- [ ] `mise //:test`, `mise //:lint` and `mise //docs:build` pass

---

## Background

A Codex Director (member 7) answered ID-only `pending` notices with `message show` instead of `poll`. `show` changes no state, so the messages stayed `input_required`: message 5887 was created at 07:31:21 and consumed at 07:32:57, and 5890 was created at 07:35:15 and consumed at 07:35:34. While a notice's IDs stay unpolled, the listener's hold delays every later notice until the 300 s ceiling re-notice, and already-read messages reappear in that re-notice and in later polls.

The record shape invites this: it lists message IDs, which reads like "go read these messages", and `member_id` is the listened recipient but reads like a sender. The watermark procedure added a second decision per notice (compare IDs against W, then maybe poll). Stale notices queued behind a busy turn are inherent to the notice bound (`SPEC.md` §6.3 `message listen`, Notice bound: a turn with k mid-turn polls queues at most k + 1 notices behind it, each covering only messages that turn already consumed). Answered by a poll, each such notice returns `[]` and costs one cheap command, so the watermark buys nothing.

---

## Specification

### `pending` record

```json
{"event":"pending","next":"cafleet message poll 7 --json"}
{"event":"pending","next":"cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"}
{"event":"pending","next":"cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"}
```

| Field | Type | Contract |
|---|---|---|
| `event` | string | `"pending"` |
| `next` | string | The consuming command for the listened subject, identical for every notice of one listener process |

Key order is `event`, `next`. A `pending` record is still never emitted for an empty pending set. The record carries neither `member_id` nor `message_ids`.

`next` construction:

1. Start with `cafleet message poll <MEMBER_ID> --json`, where `<MEMBER_ID>` is the listener's positional `MEMBER_ID` as an integer.
2. When the listener process was invoked with `--database-url <url>`, append ` --database-url ` and the shell word for `<url>` (below), verbatim as supplied, even when it names the default database file. When `--database-url` was omitted, append nothing.

The listener is invoked with `--database-url` exactly when the fleet uses a non-default database. Every installed recipe and extension passes it under that condition: Codex recipe step 1, the `database_url` tool argument of the hermes/opencode/pi `cafleet_listen`, and Claude's Monitor command under the cafleet core's IDs rule.

**Shell word.** A value whose every character is in `[A-Za-z0-9_./:~@%+=,-]` is used as is. Any other value is POSIX single-quoted: `'` + the value with each `'` replaced by `'\''` + `'`. An empty URL never reaches emission, because startup fails opening the database before `ready`.

**Internal state is unchanged.** The hold (`H`, `held_at`, `F`, `HOLD_CEILING`), reconcile/decide/commit order, re-listing, notice bound, startup replay, lease, idle cadence and Codex refused-notice rebuild keep operating on the snapshot's ordered pending-ID set. Only serialization changes: the notice decision still yields the pending-ID set that the hold commits, and the emitted record carries `next` instead of that set.

### SPEC hold prose restated with a notice's pending set

SPEC §6.3 `message listen` defines one term right after the `pending` field table: *A notice's **pending set** is `P` of the snapshot that produced it. The listener keeps it internally for the hold; the record does not carry it. `pending(P)` denotes a `pending` notice whose pending set is `P`.* Every sentence that today says a notice *names* or *carries* IDs is restated with that term:

| Location | Current | New |
|---|---|---|
| Record intro | `A pending record is an ID-only notice; it carries no message body, and the receiver consumes full records through message poll (or reads one through message show):` | `A pending record is a wake notice: it carries no message ID or body, only the consuming command in next, and the receiver consumes full records by running that command:` |
| Notice hold | `A hold exists exactly while some ID named by the last delivered notice is still pending` | `A hold exists exactly while some ID in the last delivered notice's pending set is still pending` |
| Decide table, ceiling row | `Emit pending(P), the ceiling re-notice: one bulk record naming every unpolled ID` | `Emit pending(P), the ceiling re-notice: one bulk notice whose pending set is every unpolled ID` |
| After the decide table | `Every notice carries the full pending set, so every pending ID an earlier notice named is in H` | `Every notice's pending set is the full pending set, so every pending ID in an earlier notice's pending set is in H` |
| Re-listing | `The next ordinary notice names only IDs committed after that poll.` | `The next ordinary notice's pending set holds only IDs committed after that poll.` |
| Re-listing | `an ID appears in two notices only through a ceiling re-notice` … `so every ID it names, held or fresh, is unpolled` | `an ID is in two notices' pending sets only through a ceiling re-notice` … `so every ID in its pending set, held or fresh, is unpolled` |
| Notice bound | `each naming only IDs that turn already consumed` | `each whose pending set holds only IDs that turn already consumed, so the poll it prompts returns [] unless messages arrived since` |
| Situation table | `One ceiling re-notice naming the held and fresh IDs, …` | `One ceiling re-notice whose pending set holds the held and fresh IDs, …` |
| Situation table | `The receiver polls before any notice names an ID` | `The receiver polls before any notice's pending set includes an ID` |
| Situation table | `The queued notice names only IDs that poll consumed` | `The queued notice's pending set holds only IDs that poll consumed; the poll it prompts returns [] unless messages arrived since` |
| Situation table | `one notice naming every pending (unpolled) ID, including IDs a previous listener notified` | `one notice whose pending set is every pending (unpolled) ID, including IDs a previous listener notified` |
| §6.9 Native acceptance latency (`while a notice is held, the committed ID reaches receiver output in the next notice, …`) | — | `while a notice is held, the next notice announces the committed message, after the receiver's poll releases the hold or the ceiling fires` |

In the table, backtick spans inside quoted SPEC sentences are dropped for readability; the implementer keeps SPEC's existing code spans around `pending`, `P`, `H`, `next`, `message poll` and `[]`.

### Codex forwarding

The forwarded text stays `[cafleet listen record] <compact record JSON>`, now `[cafleet listen record] {"event":"pending","next":"cafleet message poll 7 --json"}`. Its length depends on the database URL alone, so the SPEC "Argument size" bullet (growth per pending ID, `E2BIG`) is deleted; the generic `could not start codex` failure row stays.

### Director wake procedure

The Director runs each notice's `next` once. That command is `cafleet message poll <director-member-id> --json` plus the fleet's `--database-url`, which is what the Director already runs under the IDs rule. When a record is truncated or unparseable, it runs `cafleet message poll <director-member-id> --json` (plus `--database-url`) once. An empty result `[]` is the normal outcome for a notice queued behind a turn that already polled, and it completes that notice. After every poll, the Director records each returned message before any other command, as today.

`assets/skills/cafleet/roles/director.md` § Director wake becomes:

- Step 2: "A `pending` notice is a wake signal whose `next` field is the consuming command, never a payload." This replaces "naming unpolled message IDs".
- Step 3 (replaces the W step and its two sub-bullets): "On every `pending` notice, run its `next` command once. It is `cafleet message poll <director-member-id> --json` with the fleet's `--database-url` when non-default. A notice queued behind a busy turn whose messages an earlier poll already consumed returns `[]`. That empty poll is the normal outcome and completes the notice. When a record is unparseable or truncated, run `cafleet message poll <director-member-id> --json` once."
- Step 4: delete "and raise W"; the rest is unchanged.

Every backend reception recipe in `assets/skills/cafleet/reference/coding-agents.md` (Claude, Codex, OpenCode, Hermes, pi) replaces its "When every named ID is at or below your consumed watermark, end that turn with no commands; otherwise run … once, record …, raise the watermark …" sentence with the same rule. Each recipe runs the notice's `next` once, records every returned message before any other command and acts within authorization (Director wake steps 3–4). An empty `[]` poll completes a stale notice. The Claude recipe keeps its truncated/unparseable clause. "Each `pending` stdout line is a notice naming unpolled message IDs" becomes "Each `pending` stdout line is a wake notice whose `next` field is the consuming command".

The Claude Monitor note in coding-agents.md keeps its batching, rate, suppression and 500/3,000-character truncation facts. Its clause "a long `pending` line arrives truncated, and a truncated line prompts one poll, so take the IDs from that poll" is deleted, because a `pending` line no longer grows with the pending set. The Director wake truncated-record rule still covers an exceptionally long URL. The matching SPEC §6.7 sentence ("A `pending` line longer than 500 characters (roughly 100 or more three-digit IDs) …") is deleted, and the truncation fact sentence before it stays.

### `cli.md` `message listen` section

| Line (current) | New text |
|---|---|
| Intro: "Notifies an active member's pending message IDs, …" | "Notifies an active member's pending messages, …" (rest unchanged) |
| Record list: "`pending` with `member_id,message_ids`" | "`pending` with `next`" |
| Example | `{"event":"pending","next":"cafleet message poll 1 --json"}` |
| Notice paragraph | "A `pending` notice is a wake signal, never a payload: `next` is the exact consuming command, `cafleet message poll <member-id> --json`, plus `--database-url <url>` when the listener was started with one (single-quoted when the URL needs it). Run it once per notice; it returns and consumes every pending body, and `[]` means an earlier poll already consumed what the notice announced." |
| **Hold** | "After a notice is delivered, the listener holds later notices while any message pending at that notice stays unpolled. Any poll consumes the whole pending set and so releases the hold; the next snapshot with pending messages emits one notice announcing only messages committed after that poll. A hold lasting 300 s with messages committed after the held notice re-sends one bulk notice covering every unpolled message, held and fresh, and restarts the 300 s; held messages alone never trigger a re-notice." |
| **Notice bound** | "A listener emits at most 1 notice, plus one per poll that consumed held messages, plus one per 300 s ceiling firing, so a burst of any size adds no notice beyond the outstanding one. Within one listener, two notices announce the same message only through a ceiling re-notice. A turn with k mid-turn polls can queue at most k + 1 notices behind it; each announces only messages those polls already consumed, so its poll returns `[]` unless messages arrived since. A restarted listener emits one notice covering every unpolled message right after `ready`. Messages you poll before any notice announces them are never notified." The watermark sentence ("Message IDs follow commit order, so a notice whose every ID is at or below the highest ID your polls returned needs no command.") is deleted. |

### Extension `cafleet_listen` descriptions

The Hermes description in the plugin and in the `SPEC.md` §6.7 extension table becomes:

```text
Start CAFleet Director reception for this session: each turn beginning '[cafleet listen record]' carries a pending notice; run the command in its 'next' field once, which consumes and returns every unpolled message, and an empty [] result means an earlier poll already consumed them.
```

OpenCode uses the Hermes text. pi uses the Hermes text with `each turn beginning` replaced by `each message beginning`, as today. The plugins' pass-through behavior and `{"event":"closed"` prefix check are unchanged.

### Member preview pointer

The pointer becomes `Body is N characters; run cafleet message poll <recipient-id> --json to read it.` for every database. It never carries `--database-url`, like today's `message show` pointer. A member already appends its `DATABASE URL:` value to every command under the cafleet core's IDs rule, so the pointer stays one database-independent literal that every rendering process (the sender's `message send`, the recipient's runner) produces identically. Its length differs from today's only by the digit count of the recipient ID versus the message ID, so the plain-input-limit behavior is unchanged.

`N` counts stored text, as today; `<recipient-id>` is the message's `to_member_id`. The pointer appears where it does today: for a folded body above 4000 scalars (`canonical_preview_line`) and for a preview above the plan's plain-input limit without bracketed paste (`pointer_preview_line`). The inline form, header, resume clause and fallback note are unchanged, and both functions keep their signatures.

**Late pointer.** A held or retried pointer preview can land after the recipient's poll consumed the message. The pointer's poll then returns `[]`, and the member reads that body with `message show MESSAGE_ID --json` (member role, below).

### Unchanged surfaces

- `message show`: full record in every state, no `next`. It stays the way to re-read a consumed message and to check consumption state.
- `message poll`, the `ready`/`closed` records, the inline preview form and the `[cafleet msg ID from SENDER TIMESTAMP]` header.
- Preview delivery, the plain-limit tables and the runner pending-preview sweep.
- The schema: no change to `assets/migrations/V1__baseline.sql`.

### Probe and evidence wording (SPEC §6.7, `docs/docs/contributing.md`)

The subscribe and stream probes can no longer observe IDs in a notice. They observe them through the poll the notice prompts:

| Item | New wording |
|---|---|
| Observation definition | A probe message ID is *observed through a notice* when the one `poll` that a delivered `pending` record prompts returns it |
| Subscribe pass condition | Each probe message ID is first returned by a notice-prompted poll; invalidation rule unchanged |
| Stream pass condition | Each probe message ID is first returned by a poll that a notice in the running listener's output prompted, while the same listener process still runs; invalidation rule unchanged |
| Stream outcome table | Pass: `Every probe ID first returned by a notice-prompted poll before the listener exited`; Fail: `A probe ID first returned only after the listener exited, or never returned by a notice-prompted poll` |
| Subscribe Director instruction | `Consume broker messages only through your receiver. On each [cafleet listen record] turn, run the command in the record's next field once. Run no other poll or show unless a step below directs it. When I say hold, end only the next notice turn without running its command, then apply the rule above to every later notice turn, including the ceiling re-notice.` (literals in code spans as in the current text) |
| Stream task prompt | `Learn about broker messages only from that listener's output; run no other poll or show. When a pending record appears there, run the command in its next field once and continue.` (matches the subscribe instruction) |
| Idle | A notice starts a turn without input; the poll it prompts returns the message |
| Busy | Unchanged apart from "the poll it prompts returns both IDs" |
| Burst | At most two notice turns arrive for the burst, and every ID is returned by a notice-prompted poll; Codex clause unchanged. "No ID appears in two notices" is deleted (no longer observable) |
| Consumed notice | Every notice turn delivered after the directed turn runs one poll, which returns `[]` |
| Hold ceiling | No notice before t1 + 285 s; one notice arrives between t1 + 285 s and t1 + 315 s, and the poll it prompts returns both IDs |
| contributing.md step 8 record | "each notice's delivery turn and timestamp, each poll's timestamp and returned IDs" in place of "each notice's delivery turn, timestamp and named IDs, poll timestamps" |

The §6.7 Role eligibility rows keep each verdict, checked version and date, and no live re-check is part of this design. Only the clause describing what the old watermark instruction made the Director do on a consumed notice is reworded, to what was observed about delivery:

| Backend | Old clause | New clause |
|---|---|---|
| `codex` | `queued consumed notices drained as zero-command turns` | `queued consumed notices drained one per turn` |
| `opencode`, `hermes`, `pi` | `consumed notices ending turns with no command` | `consumed notices delivered as separate turns` |

The Claude row ("consumed notices delivered between tool calls of the directed turn") and the coding-agents.md Role eligibility tables carry no watermark-dependent clause and stay unchanged. "in message-ID terms" in each row stays: the probes still check message IDs, now through the prompted poll.

### Documentation surfaces

| Surface | Change |
|---|---|
| `SPEC.md` §1 core paragraph, §6.3 subcommand table, `message listen` intro | "notifies pending message IDs" → "notifies pending messages" |
| `SPEC.md` §6.3 `message listen` | Example line, `pending` field table, key order, pending-set term, `next` construction and shell word; hold prose per § SPEC hold prose restated |
| `SPEC.md` §6.3 Codex forwarding | Forwarded-text example; delete "Argument size" bullet |
| `SPEC.md` §6.5 Push notifications | Pointer text |
| `SPEC.md` §6.7 | Probes, Role eligibility clauses, extension description table, Monitor truncation sentence |
| `SPEC.md` §6.9 Native acceptance and coverage | Latency sentence per § SPEC hold prose restated |
| `docs/docs/mixed-backend-team.md` § Receive results | Replace "A notice names unpolled message IDs only. The Director remembers … needs no command. Otherwise it polls once:" with "Each notice carries the command that consumes it, and the Director runs it once:"; after the example add "A notice queued behind a turn that already polled returns `[]`, which is normal."; "the listener's next notice names only messages committed after it" → "the listener's next notice announces only messages committed after it" |
| `docs/docs/contributing.md` § Run a CAFleet subscribe probe | Probe wording above |
| `docs/docs/concepts/overview.md` | listener term: "notifies unpolled message IDs as JSONL" → "notifies pending messages as JSONL wake notices that name the consuming poll"; CLI row "notify pending message IDs" → "notify pending messages"; inline preview term "or a show pointer" → "or a poll pointer" |
| `docs/docs/concepts/coding-agents.md` | "reception of notices naming its pending messages" → "reception of notices announcing its pending messages"; "a short pointer to the full message" → "a short pointer to the poll that reads it" |
| `docs/docs/concepts/members.md` | "a short pointer to the full broker message" → "a short pointer to the poll that reads it" |
| `README.md` | No change: it mentions none of these surfaces |
| `assets/skills/cafleet/SKILL.md` § Send | "or a `message show ID --json` pointer" → "or a pointer to the recipient's `message poll` command" |
| `assets/skills/cafleet/roles/member.md` § Poll paragraph | "carries the body inline or a `message show` pointer" → "carries the body inline or a pointer to your `poll --json`"; add "When that poll returns `[]`, an earlier poll consumed the message; read it with `message show MESSAGE_ID --json`." |
| `assets/skills/cafleet/reference/cli.md` | Subcommand row "Notify pending message IDs through stdout" → "Notify pending messages through stdout"; `message listen` section per § `cli.md` `message listen` section; Push-notification pointer text; "the short `message show` pointer" → "the short `poll` pointer" |
| `assets/skills/cafleet/roles/director.md` | Director wake steps 2–4 as above |
| `assets/skills/cafleet/reference/coding-agents.md` | Five reception recipes and the Claude Monitor note as above |
| `.claude/rules/` | No change: no rule covers this convention |

### Code changes

| File | Change |
|---|---|
| `src/config.rs` | `Settings` gains `database_url_explicit: bool`, set by `Settings::new` when `--database-url` was supplied; `for_database` goes through `new`, so it is `true` |
| `src/runtime/input_client.rs` tests | The two struct literals `Settings { database_url: … }` (the `foreign_settings` constructions) add `database_url_explicit: true` |
| `src/cli/message.rs` | New `pub(crate) fn poll_command(member_id: i64, database_url: Option<&str>) -> String` and `fn shell_word(&str) -> Cow<str>`; `run_listener`/`run_forwarder` build `next` once from `MEMBER_ID` and `settings` (`Some(&settings.database_url)` only when `database_url_explicit`) and pass it to `listen(…, next: &str)`, which emits `listen_pending(next)` while `hold.commit(ids, …)` still receives the decided IDs. `ListenIo` and `ListenOutput` keep their production signatures |
| `src/output/json.rs` | `listen_pending(next: &str) -> Value` returns `{"event":"pending","next":next}` |
| `src/delivery.rs` | `canonical_preview_line` and `pointer_preview_line` render the pointer as `poll_command(message.to_member_id, None)` inside the existing sentence frame |
| `assets/extensions/hermes/cafleet/__init__.py`, `assets/extensions/opencode/cafleet.ts`, `assets/extensions/pi/cafleet.ts` | `cafleet_listen` description text above |

### Tests

All tests are in-file `#[cfg(test)] mod tests` units (`.claude/rules/code-quality.md` § Test placement).

| File | Test |
|---|---|
| `src/output/json.rs` | Replace `listen_pending_names_the_member_then_the_message_ids_in_the_given_order` with a key-order/shape test: exactly `event`, `next` |
| `src/cli/message.rs` | `poll_command`: no URL, safe URL verbatim, URL with a space and URL with `'` single-quoted (`'\''`) |
| `src/cli/message.rs` | Listener hold tests: the harness shares one `Rc<RefCell<Vec<i64>>>` between `ScriptedInbox` and `RecordingOutput`. `ScriptedInbox::snapshot` writes the ordered message IDs of each `Open` snapshot it returns, and `RecordingOutput::emit` copies that vector beside each delivered or refused `pending` record. `notice_ids`, `notices` and `refusals` read the copied IDs instead of `record["message_ids"]`, and each emitted `pending` record is asserted equal to `{"event":"pending","next":<expected>}`. Existing hold, burst, ceiling, replay and refused-notice expectations stay unchanged |
| `src/cli/message.rs` | Forwarder test: the harness (`Forwarder::start(&broker.url, …)`) always passes an explicit `--database-url`, so assert the forwarded text is `[cafleet listen record] {"event":"pending","next":"cafleet message poll <id> --json --database-url <broker.url shell word>"}`; the no-URL form stays with the `poll_command` unit test |
| `src/config.rs` | `database_url_explicit` is `false` for an omitted URL and `true` for a supplied one |
| `src/delivery.rs` | Update the existing `Body is … message show 57 --json` assertion to `Body is … message poll <to_member_id> --json` |
| `src/runtime/input.rs` | Update the existing 3000×`あ` over-plain-limit preview test (the assertion on `cafleet message show {} --json`) to the `cafleet message poll <recipient-id> --json` pointer |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first, in the order `.claude/rules/documentation-maintenance.md` prescribes.

### Step 1: SPEC.md

- [x] §1 core paragraph, §6.3 subcommand table and `message listen` intro: "pending message IDs" → "pending messages" <!-- completed: 2026-11-19T11:02 -->
- [x] §6.3 `message listen`: example, `pending` field table, key order, pending-set term, `next` construction and shell word per § `pending` record <!-- completed: 2026-11-19T11:02 -->
- [x] §6.3 `message listen` hold prose and §6.9 latency sentence per § SPEC hold prose restated <!-- completed: 2026-11-19T11:02 -->
- [x] §6.3 Codex forwarding: forwarded-text example with `next`; delete the "Argument size" bullet <!-- completed: 2026-11-19T11:02 -->
- [x] §6.5 Push notifications: pointer text <!-- completed: 2026-11-19T11:02 -->
- [x] §6.7: probe observation definition, pass conditions, stream outcome table, subscribe instruction, stream task prompt and step table per § Probe and evidence wording; Role eligibility clause rewording; extension description table; delete the 500-character `pending` line sentence <!-- completed: 2026-11-19T11:02 -->
- [x] Grep `SPEC.md` for `message_ids`, `watermark`, `ID-only`, `named ID`, `named by`, `naming`, `names only`, `names an ID`, `observed in a notice`, `message show MESSAGE_ID --json to read`; every remaining hit is unrelated to notices or the pointer <!-- completed: 2026-11-19T11:02 -->

### Step 2: docs/ and README.md

- [x] `docs/docs/mixed-backend-team.md` § Receive results rewritten per § Documentation surfaces <!-- completed: 2026-11-19T11:08 -->
- [x] `docs/docs/contributing.md` subscribe probe steps 2–8 and Director instruction per § Probe and evidence wording <!-- completed: 2026-11-19T11:08 -->
- [x] `docs/docs/concepts/overview.md`, `coding-agents.md` and `members.md` term and pointer wording <!-- completed: 2026-11-19T11:08 -->
- [x] Confirm `README.md` needs no change (no listen/notice/pointer mention) <!-- completed: 2026-11-19T11:08 -->

### Step 3: Skills

- [x] `assets/skills/cafleet/roles/director.md` § Director wake steps 2–4 per § Director wake procedure <!-- completed: 2026-11-19T11:15 -->
- [x] `assets/skills/cafleet/reference/coding-agents.md`: Claude, Codex, OpenCode, Hermes and pi reception recipes; Claude Monitor note clause deletion <!-- completed: 2026-11-19T11:15 -->
- [x] `assets/skills/cafleet/reference/cli.md`: subcommand row, `message listen` section per § `cli.md` `message listen` section, pointer text, "short `poll` pointer" <!-- completed: 2026-11-19T11:15 -->
- [x] `assets/skills/cafleet/SKILL.md` § Send pointer wording; `assets/skills/cafleet/roles/member.md` pointer wording and empty-poll `show` sentence <!-- completed: 2026-11-19T11:15 -->
- [x] Grep `assets/skills/` and `docs/` for `watermark`, `message_ids`, `highest ID`, `named ID`, `named by`, `names only`, `naming unpolled`, `notices naming`, `notice naming`, `show` pointer; every remaining hit is unrelated to notices or the pointer <!-- completed: 2026-11-19T11:15 -->

### Step 4: Listener `next`

- [ ] `src/config.rs`: `Settings.database_url_explicit` and its test; add the field to the two `Settings` struct literals in `src/runtime/input_client.rs` tests <!-- completed: -->
- [ ] `src/cli/message.rs`: `shell_word`, `poll_command` and their tests <!-- completed: -->
- [ ] `src/output/json.rs`: `listen_pending(next)` and its shape test <!-- completed: -->
- [ ] `src/cli/message.rs`: `run_listener`/`run_forwarder` build `next`; `listen` takes `next` and keeps committing decided IDs <!-- completed: -->
- [ ] `src/cli/message.rs` tests: shared snapshot-ID cell between `ScriptedInbox` and `RecordingOutput`; rework `notice_ids`/`notices`/`refusals`; assert record equality with `next`; forwarder text test with the URL form <!-- completed: -->

### Step 5: Preview pointer

- [ ] `src/delivery.rs`: pointer through `poll_command(message.to_member_id, None)` in both preview-line functions; update the existing pointer assertion <!-- completed: -->
- [ ] `src/runtime/input.rs`: update the 3000×`あ` over-plain-limit preview test's pointer assertion <!-- completed: -->

### Step 6: Extensions

- [ ] Update the `cafleet_listen` description in `assets/extensions/hermes/cafleet/__init__.py`, `assets/extensions/opencode/cafleet.ts` and `assets/extensions/pi/cafleet.ts`, matching `SPEC.md` verbatim <!-- completed: -->
- [ ] Grep `assets/extensions/` for `watermark` and `naming unpolled`; no hit remains <!-- completed: -->

### Step 7: Validation

- [ ] `mise //:test` passes <!-- completed: -->
- [ ] `mise //:lint` passes <!-- completed: -->
- [ ] `mise //docs:build` passes <!-- completed: -->
- [ ] Repository-wide grep (excluding `design-docs/`) for `"message_ids"`, `message_ids:`, `watermark`, `naming unpolled`, `message show MESSAGE_ID --json to read it` and `message show {} --json`; no hit remains. The `message_ids` identifiers in `src/broker/messaging.rs` (test `message_ids_from_interleaved_inserts_increase_in_commit_order`) and the local variable in `src/runtime/recovery.rs` are unrelated and stay <!-- completed: -->
- [ ] Reinstall the CLI through the task registry (`mise tasks --all`) only after the executing fleet ends, preserving its installed runtime/assets meanwhile <!-- completed: -->
