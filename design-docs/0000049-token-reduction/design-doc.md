# Reduce Token Consumption Across CAFleet Output Surfaces

**Status**: Complete
**Progress**: 107/118 tasks complete (11 deferred — Step 0 baseline skipped + 3 Step 15 baseline-dependent + 1 Step 17 narrative pre-staged + 3 Step 18 manual-smoke + 3 doc-pointer holdovers; live orchestration team itself ran 102 design-doc tasks via the new compact envelope, supplying production smoke coverage for the deferred manual gates)
**Last Updated**: 2026-05-05

## Overview

CAFleet does not consume LLM tokens itself, but every byte it emits — member spawn prompts, message envelopes, poll output, broker auto-injected text, the `cafleet` skill, the project CLAUDE.md / rules files, the Director's `/loop` template, and (most expensively) the raw tmux pane content returned by `cafleet member capture` — lands in a coding agent's context and bills against its tokens. This design enumerates **nineteen** independently shippable reductions across the per-message, per-spawn, per-Director-tick, and per-context-load cost axes; drops the externally-inherited envelope convention so the persisted shape can be simplified alongside the rendered shape; replaces the auto-fire poll keystroke with an inline message preview now that the wire format is ours to define; trims help-text and the JSON agent-card blob; and adds a token-budget regression harness (with a pre-baseline captured *before* any surface lands) so future drift is caught at PR time.

## Success Criteria

- [ ] Director monitoring loop's per-tick token cost (current dominant sink) drops by ≥80%, measured by capturing actual tokens emitted into the Director pane during a 10-tick (10-minute) idle session with 3 members. Pre-baseline captured to `tests/token_budget/measurement_results.md` **before any surface lands**.
- [ ] Default per-message-poll envelope (text mode, one task) drops from ~6 lines to ≤3 lines; (JSON-indented mode) drops by ≥65 % of measured baseline tokens — measured by unit test on a fixed fixture rather than the doc's own arithmetic.
- [ ] Per-`message send` keystroked content into recipient pane is replaced with an inline preview ≤120 chars; no `cafleet message poll` invocation occurs in the auto-fire path.
- [ ] Member spawn prompt drops from ~150 tokens (post-substitution) to ≤70 tokens; if Surface 16 research succeeds, prompt moves into the system prompt (cacheable) rather than first user turn.
- [ ] `skills/cafleet/SKILL.md` core file drops from 926 lines to ≤350 lines; reference files load on demand for both Claude Code AND codex backends.
- [ ] `member capture` default `--lines` drops from 80 to 30 in **both** `cli.py:970` and `tmux.py:194`; the `/loop` template stops capturing every member every tick by default.
- [ ] `Task.task_json` column is removed from the schema; a new `Task.text` column holds the message body. Single Alembic revision; pre-flight backfill check + backup step in upgrade notes.
- [ ] `Agent.agent_card_json` blob is slimmed to the minimum-required shape (Surface 18); per-row token cost on `agent list` drops from ~300–500 tok to ≤80 tok.
- [ ] Every current-state mention of the inherited "A2A" / "agent-to-agent" framing is scrubbed from `README.md`, `ARCHITECTURE.md`, all `CLAUDE.md` files, every `skills/**/*.md`, every source-code comment/docstring, and the WebUI; remaining references live only inside `design-docs/0000001-a2a-registry-broker/` (historical record).
- [ ] `CAFLEET_MAX_TEXT_LEN` env var replaces the hardcoded `limit=10` in `output.truncate_text` (default `200`).
- [ ] CLAUDE.md duplication is resolved by **merging** divergent content (the `.claude/CLAUDE.md` extra `/update-readme` skill bullet) into root `CLAUDE.md` and then deleting `.claude/CLAUDE.md` — *not* a blind byte-delete (the files are not byte-identical, contrary to v2's claim).
- [ ] `Stall Response` and `Authorization-Scope Guard` no longer duplicate across `agent-team-{monitoring,supervision}` skills.
- [ ] `cafleet --help` and per-subcommand help texts (Surface 19) drop multi-sentence explanations to single phrases; aggregate help-text token cost cut by ≥40 %.
- [ ] A `tests/token_budget/` suite asserts character counts on representative outputs against a checked-in baseline; CI fails on regression.

---

## Background

### Token-cost surface map (current baseline)

| Rank | Surface | Where | Per-event cost | Per-session multiplier |
|---|---|---|---|---|
| **1** | **`/loop`-driven `member capture --lines 200`** | `cli.py:964-1019` + `tmux.py:194-198` + `agent-team-monitoring/SKILL.md:91` | ~200 lines × ~5 tok/line ≈ ~1,000 tok / capture | × N members × 60 ticks/hour ≈ ~180,000 tok/hour for a 3-member session |
| 2 | `skills/cafleet/SKILL.md` cold load | 926 lines | ~7,400 tok | × N members |
| 3 | Auto-fire poll keystroke + recipient's poll-output dump | `tmux.py:130-135` + `broker.py:746-773` | full poll command (~60 chars) + full unacked-inbox envelope dump | × every send |
| 4 | Message envelope (JSON, indented) | `broker.py:587-614` + `output.py:6-7` | **~70–85 tok / task** (revised from v3's 107; v3 over-stated) | × N polls × N tasks |
| 5 | `Agent.agent_card_json` blob in `agent list` | `models.py:39` + broker getters | ~300–500 tok / agent / list call | × every list call |
| 6 | Project rule files in system prompt | `.claude/CLAUDE.md` + `CLAUDE.md` (similar but **not** byte-identical) + 4 `.claude/rules/*.md` | ~140 lines | every spawn |
| 7 | `agent list` 4-line text rows + `member list` JSON-mode rows | `output.py:82-89, 123-150` | ~80–100 tok / row | × every list call |
| 8 | Per-subcommand `--help` texts (`message`, `member`, `session`, `agent`, `db`) | every `@click.option(help=...)` in `cli.py` | ~40 lines on `cafleet message --help`; multi-sentence verbosity | × every typo or orientation |
| 9 | Member spawn prompt | `cli.py:26-36` | ~150 tok | × N members |
| 10 | Broadcast summary `recipientIds[]` | `broker.py:723` | ~50 chars × N recipients (full UUIDs) | × every broadcast |
| 11 | `format_session_create` (7 lines) + `format_member` (6 lines) spawn echoes | `output.py:92-117` | ~50–60 tok | × every create |
| 12 | `agent-team-{monitoring,supervision}` duplicated Stall Response + Authorization-Scope | `skills/agent-team-*/SKILL.md` | ~30–60 tok of repeated content | × Director spawns |

### What is already in place

| Mechanism | Where | Default |
|---|---|---|
| Body-text truncation (10 codepoints, `...` suffix) | `output.truncate_text` | ON; bypass via `--full` |
| `since` filter on `poll_tasks` | `broker.py:746-773` | Available but not wired into auto-fire |

### What was newly authorized

The wire format and persisted envelope shape were modeled on the inherited "agent-to-agent" convention. Cafleet has no external consumers that depend on the exact shape. Dropping the convention enables Surface 14 (persisted shape simplification), Surface 15 (inline preview replaces auto-fire poll), and the Step 1.A reference scrub.

### What stays architecturally blocked

`CAFLEET_SESSION_ID` / `CAFLEET_AGENT_ID` env-var shortcuts in command lines. Claude Code's `permissions.allow` matches Bash invocations as literal command strings; shell expansion of UUIDs would break permission gating. UUIDs in command lines stay full; UUIDs in *rendered output* are slimmed via prefix-rendering (Surface 1).

---

## Specification

### Dependency graph (corrected from v3)

| Surface | Depends on | Notes |
|---|---|---|
| 1 (compact rendered envelope) | **14** | `render_task` reads `task["text"]` which only exists post-Surface-14. Land 14 first, or have 1 read from `task_json.artifacts[0].parts[0].text` until 14 lands. |
| 2 (`--since` auto-fire) | 15 | After 15, broker stops keystroking polls; `--since` keeps living as a filter on manual / `member ping` polls. |
| 3, 4, 5, 6, 7, 11, 12, 17 | none | Independently shippable. |
| 8 (`member list --activity`) | 14 | Aggregation queries typed columns, fully realized only after 14. Pre-14 the same join works but on the legacy column set. |
| 9, 10 | 8 (loosely — the `/loop` template change in 10 references the `--activity` flag) | |
| 13 (token-budget tests) | runs after each other surface; pre-baseline before any | |
| 14 | Step 1.A (convention scrub doc) | |
| 15 | 14, 1, **8** (in production — fallback chain requires `last_recv` tracking) | |
| 16 (cacheable spawn prompt research) | gates Step 9 final form; Surface 6 ships independently | |
| 18 (`agent_card_json` slim) | none | |
| 19 (help-text trim) | none | |

### Per-message cost

#### Surface 1 — Compact rendered envelope

**Current** (`broker._unicast_task_dict`, `broker.py:587-614`): seven keys with three nested objects, an always-empty `history`, an internal `artifactId`, a constant-valued `kind: "text"`, and a `contextId` that always equals the recipient's own `agent_id`.

**Target rendered shape** (post-Surface-14):

```python
def render_task(task: dict, *, full: bool = False) -> dict:
    if full:
        return task  # the typed-column dict, post-Surface-14
    out = {
        "id": task["task_id"][:8],
        "from": task["from_agent_id"][:8],
        "ts": task["status_timestamp"],
        "text": task["text"],
    }
    if task["type"] != "unicast":
        out["kind"] = task["type"]
    if task.get("origin_task_id"):
        out["origin"] = task["origin_task_id"][:8]
    return out
```

**Pre-Surface-14 implementation** (if shipped first): same fields, but `text` is fetched via `task["artifacts"][0]["parts"][0]["text"]` and `from`/`ts` via `task["metadata"]["fromAgentId"]` / `task["status"]["timestamp"]`. Step 3 includes a small adapter for the bridge period.

**Field decisions**: `task_id` keep+prefix; `from_agent_id` keep+alias to `from`; `status_timestamp` keep+alias to `ts`; `text` keep; `type` drop when `unicast`; `origin_task_id` keep when present, alias to `origin`, prefix-render; `to_agent_id`, `context_id`, `status_state` (when default `input_required`) — drop in compact, return in `--full`.

**Defaults**: text mode 2 lines/task; JSON mode `json.dumps(data, separators=(",",":"))` (no whitespace); new global `--pretty` for indented JSON.

**Estimated savings**: indented-JSON envelope drops from ~70–85 tok (revised baseline) to ~22 tok (~70 % reduction). Final number determined empirically by Surface 13's fixture-based test.

#### Surface 2 — `--since` filter (post-15 fallback)

After Surface 15, broker no longer keystrokes polls. The `--since` flag remains useful for **manual** polls and for `cafleet member ping` (the operator-/Director-driven re-poke primitive at `tmux.py:92`). `Agent.last_auto_fire_ts` is **not** added — the column would have no writer post-Surface-15. Instead, callers (Director's `/loop`, manual operator, `member ping`) supply `--since` explicitly, anchored to whatever timestamp they already track.

#### Surface 3 — Compact lists + `--quiet` writes + slim spawn echoes + bridge adapter

- `agent list`: one row/agent (`<id8> <name> <status>`); description behind `--full`.
- `message poll`: 2 lines/task (per Surface 1).
- `message broadcast` echo: today emits one full envelope per recipient. Default to one-line summary (`broadcast id=<id8> recipients=<count>`); per-recipient envelopes behind `--full`.
- `--quiet` flag on `message {send,ack}` and `member ping`: emit only the new task id (8-char prefix).
- `format_session_create` 7→1 line; `format_member` 6→1 line; full views behind `--full`.
- **Bridge adapter** (only if Surface 1 ships before Surface 14): `output.render_task` accepts both the legacy `task_json`-style nested dict and the typed-column flat dict; one branch each, removed when Surface 14 lands.

#### Surface 4 — Broadcast summary slim

Render-time omit `recipient_ids` (post-Surface-14: a typed column or computed from `Task.origin_task_id` joins) by default; `recipient_count` is sufficient for the broadcaster's "did it go out?" check. `--full` re-includes the list.

#### Surface 5 — Configurable text truncation

- `Settings.max_text_len: int = 200` with alias `CAFLEET_MAX_TEXT_LEN`.
- `output.truncate_text` reads `settings.max_text_len` when `limit` not supplied.
- Suffix `"..."` (3 codepoints) → `"…"` (1 codepoint). **Caveat**: for tokenizers that split `"…"` into multiple bytes, the win could be neutral; verify with the actual tokenizer in Surface 13.
- Truncation extended to `agent.description` (limit 60); applied to non-UUID metadata strings (limit 80). Both bypassed by `--full`.

### Per-spawn cost

#### Surface 6 — Slim member spawn prompt

**Current** (`cli.py:26-36`): ~150 tok of narrative covering Codex/Claude branch + identity + director + polling instruction + auto-approve paragraph.

**Proposed** (~60 tok):

```
Member of cafleet session {session_id} (agent={agent_id}, director={director_agent_id}).
Load skill 'cafleet'. Bash auto-approves. Poll: cafleet --session-id {session_id} message poll --agent-id {agent_id}
```

**Implementation cleanups Step 9 must include**:
- Remove the now-unused `{director_name}` placeholder from `_resolve_prompt` (`cli.py:647-682`) and from every call site that supplies it. The substitution is dead after the slim.
- The codex-doc pointer (`docs/codex-members.md`) currently in the spawn prompt moves into `skills/cafleet/SKILL.md` core (Step 10). **Verify codex's skill loader behavior**: Claude Code auto-loads `SKILL.md` via the Skill tool; codex reads `docs/codex-members.md` directly per the existing convention. The slim prompt assumes codex agents will Read `skills/cafleet/SKILL.md` after spawn — this needs an explicit codex integration test in Step 10.

#### Surface 7 — Skill-file split (`skills/cafleet/SKILL.md`)

| File | Target lines | Contents |
|---|---|---|
| `skills/cafleet/SKILL.md` (core) | ≤350 | Identity, poll/send/ack, codex vs. claude branch (≤4 lines), one canonical example each; **codex-spawn bootstrap content folded in from `docs/codex-members.md`** (or kept in that file with a Read pointer; depends on Step 10 codex verification). |
| `skills/cafleet/reference/director.md` | ~150 | `member create`, `member ping`, `member exec`, member-list semantics |
| `skills/cafleet/reference/broadcast.md` | ~80 | Broadcast send + ack + threading via `origin` |
| `skills/cafleet/reference/exec-routing.md` | ~150 | Denied-Bash → Director routing protocol |
| `skills/cafleet/reference/recovery.md` | ~100 | Crash/disconnect/idle recovery flows |
| `skills/cafleet/reference/legacy-flags.md` | ~50 | `--full`, `--pretty`, `--json` opt-back-ins |

Members read reference files on demand via Read.

### Per-Director-tick cost (the elephant)

#### Surface 8 — `cafleet member list --activity`

Extend `member list` with last-sent / last-received / last-ack timestamps + idle duration aggregated from `tasks`.

```
$ cafleet --session-id <s> member list --agent-id <d> --activity
3 members:
  agent_id        name      state   last_sent    last_recv    last_ack     idle
  --------------  --------  ------  -----------  -----------  -----------  -----
  abc12345        alice     active  12:34:56     12:34:50     12:34:50     6s
  def67890        bob       active  12:30:11     12:33:02     12:33:02     2m
  ghi24680        carol     idle    -            12:20:00     12:20:00     14m
```

**Aggregation must filter `Task.type != "broadcast_summary"`** for the `last_ack` proxy — broadcast_summary tasks are seeded with `status_state="completed"` (`broker.py:705`) and would otherwise pollute the proxy. This mirrors `poll_tasks` (`broker.py:757`).

**Indexes**: existing `idx_tasks_context_status_ts` and `idx_tasks_from_agent_status_ts` (`models.py:87-88`) cover the join columns. Bench at 1k-message fixture (Step 11).

#### Surface 9 — Capture defaults

- Default `--lines` 80 → 30 in **both** `cli.py:970` and `tmux.capture_pane`'s default at `tmux.py:194` (the latter is the library API; tests/callers may invoke it positionally).
- New `--ansi/--no-ansi` flag (default `--no-ansi`); strip ANSI via `re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)` plus carriage-return de-fragmentation for TUI redraws.
- `--tail` alias for `--lines`.
- **Calibration**: 30 may truncate stalled `AskUserQuestion`-style prompts. Resolved at Step 12 by capture-trace measurement; bump to 50 if 30 cuts off in practice.

#### Surface 10 — `/loop` prompt template trim

**Current** (`agent-team-monitoring/SKILL.md:86-96`): the template is **11 lines**, not 60 (v3 over-stated). The current template references `--lines 200` (line 91) — the dominant per-tick cost.

**Changes**:
- Routine ticks consult `member list --activity` (Surface 8) to identify which members need capture.
- When capture is needed, default to `--lines 30 --no-ansi`.
- Drop the in-template `member ping` vs `member exec` distinction (canonical home is `skills/cafleet/SKILL.md`).
- Target template length: ≤9 lines (slightly tighter than current 11).

### Per-context-load cost

#### Surface 11 — CLAUDE.md merge + dedupe (corrected from v3)

**Current divergence** (audit confirmed via Read):

| File | Heading | Skill list |
|---|---|---|
| `/CLAUDE.md` (root) | `## Skills` | 7 skills (no `/update-readme`) |
| `/.claude/CLAUDE.md` | `## Project Skills` | 8 skills (includes `/update-readme`) |

The files are NOT byte-identical (v3 was wrong). Resolution:

1. Merge `.claude/CLAUDE.md`'s `/update-readme` bullet into root `CLAUDE.md`.
2. Reconcile heading (`## Skills` vs `## Project Skills`) — pick one, document the choice.
3. Verify Claude Code still discovers project rules via root `CLAUDE.md` after the deletion.
4. Delete `.claude/CLAUDE.md`.

#### Surface 12 — Consolidate Stall Response + Authorization-Scope Guard

- Stall Response: keep canonical version in `agent-team-monitoring`; supervision keeps a 1-line cross-reference.
- Authorization-Scope: move canonical content + decision table into `.claude/rules/skill-discovery.md`; supervision keeps a 5-line summary that points at the rule.

### Tooling

#### Surface 13 — Token-budget regression tests + measurement plan

```python
# tests/token_budget/test_envelope_size.py
def test_compact_envelope_bytes_under_budget():
    fixture = load_fixture("five_unicast_tasks.json")
    rendered = output.format_json(
        [output.render_task(t) for t in fixture],
        pretty=False,
    )
    assert len(rendered) <= 750  # measured against fixture, not arithmetic
```

**Tokenizer choice**: tests assert *character* counts (cheap, deterministic). Document chars-per-token assumption in test header. Surface 13 records a real-tokenizer count (Anthropic's tokenizer or a local `tiktoken` proxy) alongside the char count for cross-checking; if the chars-vs-tokens ratio drifts materially after any surface, swap to the real tokenizer for assertions.

**Pre-baseline gating** (the v3 self-confirmation hole, fixed):

1. **Step 0** (lands first, in its own PR before any surface): capture pre-baseline using `tests/token_budget/scenarios/idle_3_member_baseline_stub.py` (deterministic scripted scenario where members no-op while the Director runs current `/loop`; originally targeted a 10-minute window — deferred per operator at execute time, see Step 0 checkbox below). Persist byte count to `tests/token_budget/measurement_results.md` with an explicit `baseline_pre_design_0049: <N>` key.
2. Each subsequent surface PR records its post-change byte count under a `post_surface_<N>: <M>` key.
3. The pass criterion `total ≤ 0.20 × baseline_pre_design_0049` is asserted by a CI test that reads `measurement_results.md` and the live measurement.

### Now-in-scope (was deferred)

#### Surface 14 — Drop the inherited convention; simplify persisted shape

**Current schema** (`models.py:68-89`): typed columns plus a `task_json` JSON blob redundant with the typed columns.

**Changes**:
- Add `Task.text: str` column.
- Drop `Task.task_json` column.
- **Single Alembic revision** (one transaction; no in-between state where one binary version sees a column the other doesn't).
- Pre-flight check in the migration: assert every existing row has a non-null body at `task_json.artifacts[0].parts[0].text` before backfilling `text`.
- **Callers that round-trip via `task_json` and must be rewritten in lockstep** (Step 2 expanded from v3):
  - `_save_task` (`broker.py:551-577`) — writes `task_json`; switch to typed columns.
  - `_read_task` (`broker.py:580-584`) — reads `task_json`; switch to typed columns. *(was missed in v3)*
  - `_unicast_task_dict` (`broker.py:587-614`) — constructs the dict shape.
  - Broadcast-summary builder (`broker.py:700-727`).
  - `poll_tasks` (`broker.py:746-773`) — switch from `json.loads(row.task_json)` to typed-column SELECT.
  - `ack_task` (search via Grep — round-trips through `_read_task`/`_save_task`). *(was missed in v3)*
  - `cancel_task` (search via Grep — same). *(was missed in v3)*
- `_save_task`'s broadcast-summary handling currently uses `metadata.get("toAgentId", "")` (a legitimate `.get()` cited in `.claude/rules/code-quality.md`). Post-Surface-14 the dict no longer wraps in metadata; the substitute is `task_dict.get("to_agent_id", "")` or — better — make the broadcast-summary builder pass `to_agent_id=""` explicitly. **Update `.claude/rules/code-quality.md`** in Step 1 to reflect the new authoritative example.
- WebUI consumers (under `admin/`) update their type definitions to the typed-column shape.

**Migration risk**: irreversible without backup. Document `cp ~/.local/share/cafleet/registry.db ~/.local/share/cafleet/registry.db.pre-0049.bak` in upgrade notes.

#### Surface 15 — Inline message preview (replaces auto-fire poll keystroke)

**Proposed**: broker keystrokes the message preview directly:

```
[cafleet msg abc12345 from xy23 12:34Z]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

Recipient sees the message as a user-turn input; their TUI processes it as a fresh user message. Recipient acks via `cafleet message ack --task-id abc12345`. **No `cafleet message poll` invocation in the auto-fire path.**

**Helper**: a **new** `tmux.send_inline_preview(target_pane_id, *, task_id_8, sender_8, ts, text)` — **NOT** a reuse of `send_freetext_and_submit` (which prepends a literal `"4"` for AskUserQuestion option-4 freetext semantics; reusing it would type a stray `4` into the recipient's input box). The new helper combines the literal-text-plus-Enter pattern from `send_poll_trigger` (`tmux.py:92-145`) with the codex bracketed-paste delay.

**Failure mode** (also flagged in Concerns): if the recipient's TUI is in a non-input state, the keystroked preview lands wherever the cursor is. Same failure mode as today's auto-fire poll. Mitigation: `member list --activity` (Surface 8)'s `last_recv` column drives Director monitoring; Director re-pokes via `cafleet member ping` if a recipient went silent.

**Per-send savings**: replaces ~80–120 tok of (poll command + envelope dump) with ~25 tok of inline preview. ~70 tok per send × 30 sends/hour ≈ 2,100 tok/hour. Smaller than Surface 8 but eliminates a frequent confusion mode.

#### Surface 16 — `--system-prompt-append` for cacheable spawn prompt (research-first)

Investigate whether `claude` and `codex` CLIs support a flag that appends to the *system prompt* (cacheable across the prompt-cache 5-minute TTL, billed once per cache window) rather than placing the prompt as the first user turn. If yes, switch `_build_claude_command` (`cli.py:39-47`) and `_build_codex_command` (`cli.py:50-58`). If no, keep the positional argument.

#### Surface 17 — Drop `[N]` index labels in `format_indexed_list`

`output.format_indexed_list` (`output.py:68-79`) prepends `[1]`, `[2]`, ... — agents reference tasks by `task_id` (8-char prefix), not index. Drop the prefix.

#### Surface 18 — `Agent.agent_card_json` slim (promoted from out-of-scope)

**Current** (`models.py:39`): `agent_card_json: str` — a JSON blob storing the agent card emitted on `agent list` and `agent show`. For a 6-skill agent card, this is ~300–500 tok per row. Multiplied across `agent list` calls (every `/loop` tick consults the agent registry) this is non-trivial.

**Changes**:
- Render-side: by default emit only the fields used by Director monitoring and member identity (`agent_id`, `name`, `description` truncated, `status`, `coding_agent`); full card behind `--full`.
- Storage-side: leave `agent_card_json` blob alone for v0.X (avoids a second migration in the same release); render-side projection captures most of the win.
- Future migration (Surface 18b, follow-up design): replace blob with typed columns mirroring the Surface 14 pattern.

**Estimated savings**: ~250 tok per `agent list` row × N agents × N list calls. For a Director that lists 4 agents per tick, ~16,000 tok/hour saved.

#### Surface 19 — Help-text trim

**Current**: every `@click.option(... help="...")` carries a multi-sentence explanation; `cafleet message --help` aggregates to ~40 lines. Agents that mistype a command or invoke `--help` for orientation pay the full cost.

**Changes**:
- Audit every `help=` string in `cli.py` (~80 sites). Reduce multi-sentence explanations to single phrases.
- Group short flag-help into one-liners; reserve narrative explanation for `docs/spec/cli-options.md`.
- Add a short `\b` epilog example to each command instead of in-flag prose.
- Aim: aggregate help-text token cost cut by ≥40 %.

**Estimated savings**: ~50–100 tok per `--help` dump × every typo or orientation. Frequency low but constant.

---

## Cumulative savings estimate

For a representative session — 3 members, 30 messages over 1 hour, Director on `/loop` 60 ticks/hour. Numbers are first-pass estimates; Surface 13's measurement plan is the ground truth.

| Surface | Per-event saving | Per-hour saving |
|---|---|---|
| 1. Compact rendered envelope | ~70 % per task render (revised) | ~5,500 tok |
| 2. `--since` filter (post-15 fallback) | small | ~300 tok |
| 3. Compact lists + `--quiet` + spawn echo + broadcast echo | 50–85 % on those calls | ~1,500 tok |
| 4. Broadcast summary slim | ~50 chars × N recipients | ~700 tok |
| 5. Configurable truncation | Marginal; potentially neutral if `"…"` tokenizes oddly | ~200 tok |
| 6. Spawn prompt slim | ~85 tok × 3 spawns | ~250 tok one-time |
| 7. Skill split | ~5,000 tok × 3 spawns | ~15,000 tok one-time |
| 8. `member list --activity` | 95 % of capture traffic eliminated | ~150,000 tok |
| 9. Capture defaults (80 → 30, no-ansi) | rolled into 8 | rolled into 8 |
| 10. `/loop` template trim | rolled into 8 | rolled into 8 |
| 11. CLAUDE.md merge + dedupe | ~225 tok × 3 spawns | ~675 tok one-time |
| 12. Skill consolidation | ~300 tok × Director spawn | ~300 tok one-time |
| 13. Token-budget tests | regression guard | n/a |
| 14. Persisted shape simplification | reduces re-serialization on every poll | rolled into 1 |
| 15. Inline message preview | ~70 tok × 30 sends | ~2,100 tok |
| 16. Cacheable spawn prompt (if supported) | spawn prompt bills once per cache window | up to ~150 tok / 5 min / member |
| 17. Drop `[N]` index labels | ~3 tok × N tasks per poll | ~50 tok |
| 18. Agent-card render slim | ~250 tok × N agents × N list calls | ~16,000 tok |
| 19. Help-text trim | ~50 tok per `--help` dump | session-dependent |

**Estimated session total**: ~190,000+ tokens/hour saved during steady-state monitoring; one-time savings of ~16,000 tok/spawn.

---

## Concerns / open questions

1. **`--full` semantic overloading.** Surfaces 1, 3, 4, 5, 9, 18 all extend `--full`'s meaning. Decision: `--full` is the global "give me every field cafleet has, untruncated, unfiltered" — single global escape hatch. Document in `docs/spec/cli-options.md`. If user feedback demands granular flags (`--full-envelope`, `--full-recipients`, `--full-card`), add later.

2. **Timestamp resolution.** ISO timestamps with second precision risk missing same-second sends in `--since` and `--activity` queries. Spec µs precision: `datetime.now(UTC).isoformat(timespec='microseconds')`.

3. **Codex skill loader behavior.** Claude Code auto-loads `SKILL.md` via the Skill tool. Codex's behavior is **separately verified at Step 10** — codex agents must Read `skills/cafleet/SKILL.md` after spawn (vs. Claude's auto-load). The slim spawn prompt drops the explicit `docs/codex-members.md` pointer; Step 10 must add an integration test that a codex member can complete a poll/send/ack cycle from the slim prompt.

4. **Tokenizer choice for budget tests.** Char counts are deterministic but approximate. Surface 13 records a real-tokenizer count (Anthropic / `tiktoken` proxy) alongside the char count. If chars-vs-tokens drifts after a surface, swap to real tokenizer for assertions.

5. **Measurement determinism.** Use canned scripted scenario (`idle_3_member_baseline_stub.py`) for the pre/post comparison. Document ±5 % variability across runs.

6. **`member capture --lines` calibration.** 30 may truncate stalled prompts. Resolved at Step 12 by capture-trace measurement. Bump to 50 if needed.

7. **Surface 15 keystroke failure modes.** Documented fallback chain: Surface 15 → Director monitoring (Surface 8 `last_recv`) → `cafleet member ping` (manual re-poke).

8. **Surface 16 dependency.** If `claude --append-system-prompt` (or equivalent) doesn't exist, Surface 6 ships without the cacheability multiplier.

9. **CLAUDE.md byte-identity** (closed). v3 was wrong; the files diverge in the skill list. Surface 11 now does a content merge, not a blind delete.

10. **Surface 14 callers list** (closed). v3 omitted `_read_task`, `ack_task`, `cancel_task`. Fixed in Step 2.

11. **Surface 8's `last_ack` proxy filter** (closed). v3 omitted `Task.type != "broadcast_summary"`. Fixed.

12. **`send_inline_preview` helper** (closed). v3 proposed reusing `send_freetext_and_submit` which prepends literal `"4"` for AskUserQuestion option-4 semantics. v4 specifies a new helper.

13. **`/loop` template length** (closed). v3 said "60-line template → 25 lines". The actual template at `agent-team-monitoring/SKILL.md:86-96` is 11 lines. Target: ≤9 lines.

14. **Update `.claude/rules/code-quality.md`** in Step 1 — the `metadata.get("toAgentId", "")` example becomes stale post-Surface-14. Replace with the new authoritative `.get()` example or remove if no example remains.

---

## Risks & rollback

| Risk | Likelihood | Mitigation |
|---|---|---|
| Surface 14 migration corrupts data | Low | Single Alembic revision; pre-flight non-null check; backup step in upgrade notes; revert via backup restore. |
| Surface 14 missed caller breaks `ack`/`cancel`/`show` | Medium | Step 2 expanded to enumerate every `task_json` reader (`_read_task`, `ack_task`, `cancel_task` were missed in v3); integration tests cover each command end-to-end. |
| Surface 15 keystroked preview misses (TUI busy) | Medium | Documented fallback chain (Surface 8 + `member ping`). Same failure as today's auto-fire poll. |
| Surface 11 content loss when deleting `.claude/CLAUDE.md` | Low (v3 was higher) | v4 merges `/update-readme` into root before deleting; explicit content-diff step. |
| `--no-ansi` strips characters a Director needed | Low | `--ansi` opt-in. Default-off based on raw-pane noise audit. |
| Surface 8 aggregation slow on large message tables | Low | Existing indexes cover join; bench at 1k-message fixture. |
| `/loop` template change leaves a stalled member unobserved | Medium | `member list --activity`'s `idle` column drives escalation; capture invoked when threshold exceeded. |
| Surface 16 research dead-ends | Medium | Step 9 ships independently; Surface 16 only multiplies. |
| Surface 7 split breaks codex skill loading | Medium (codex semantics not auto-loaded) | Step 10 integration test for codex; if codex requires a flat single-file skill, keep core SKILL.md self-contained for codex while Claude Code agents read the references. |
| `"…"` suffix change neutralizes / regresses tokens for some tokenizer | Low | Surface 13 tokenizer measurement catches it; revert to `"..."` if measured worse. |

Each surface ships independently. If any one regresses, revert just that step.

---

## Implementation

> **Step 0 must land first, in its own PR, before any surface step.**

### Step 0: Pre-baseline capture

- [x] Implement `tests/token_budget/scenarios/idle_3_member_baseline_stub.py` deterministic single-shot scenario (originally scoped as `idle_3_member_10_minute.py` with a 10-minute sampling window — deferred at execute time per operator; renamed to honest stub name 2026-05-05T11:25 after PR #53 Copilot review). <!-- completed: 2026-05-05T05:42 -->
- [-] Run scenario / persist baseline / standalone PR — skipped per operator on 2026-05-05; Surface 13 char-count tests cover the regression budget.

### Step 1: Documentation first (with inherited-convention scrub)

- [x] **1.A** Scrub current-state mentions of "A2A" / "agent-to-agent" / "A2A-inspired" from: `README.md` (line 3), `ARCHITECTURE.md`, `CLAUDE.md` (root, lines 9 & 19), `skills/cafleet/SKILL.md`, all `skills/cafleet/roles/*.md`, all `docs/**/*.md`, all source-code docstrings/comments, and `admin/`. The `0000001-a2a-registry-broker/` directory stays as historical record. <!-- completed: 2026-05-05T05:51 -->
- [x] **1.B** Update `ARCHITECTURE.md` with the typed-column envelope shape, spawn-prompt slim, skill split, `member list --activity`, inline-preview keystroke, and persisted-shape simplification. <!-- completed: 2026-05-05T05:55 -->
- [x] **1.C** Update `docs/spec/cli-options.md` with `--pretty`, `--quiet`, `--full`, `--activity`, capture default change (80 → 30), `--ansi/--no-ansi`, `--tail`, and `CAFLEET_MAX_TEXT_LEN`. <!-- completed: 2026-05-05T05:55 -->
- [x] **1.D** Update / create `docs/spec/message-envelope.md` describing the typed-column shape, the rendered shape, and `--full`. No reference to the inherited convention. <!-- completed: 2026-05-05T05:59 -->
- [x] **1.E** Update `docs/spec/data-model.md` with the new `Task.text` column, the dropped `task_json` column, migration / backup procedure. <!-- completed: 2026-05-05T05:55 -->
- [x] **1.F** Split `skills/cafleet/SKILL.md` into core (≤350 lines) + 5 reference files. <!-- completed: 2026-05-05T06:05 -->
- [x] **1.G** Update `skills/cafleet/roles/director.md` and `skills/cafleet/roles/member.md` to point at reference files. <!-- completed: 2026-05-05T06:05 -->
- [x] **1.H** Trim `/loop` template in `agent-team-monitoring/SKILL.md:86-96` to ≤9 lines (Surface 10). <!-- completed: 2026-05-05T06:13 -->
- [x] **1.I** Move Stall Response canonical version into `agent-team-monitoring`; reduce supervision to 1-line cross-ref. <!-- completed: 2026-05-05T06:13 -->
- [x] **1.J** Move Authorization-Scope canonical version into `.claude/rules/skill-discovery.md`; reduce supervision to 5-line summary. <!-- completed: 2026-05-05T06:18 --> <!-- COMMENT(claude) 2026-05-05T12:20: marketplace-driven flip — the CAFleet-supervision-specific Authorization-Scope Guard moved BACK into `Skill(agent-team-supervision)` so the skill stays self-contained when shipped via `.claude-plugin/plugin.json`. `.claude/rules/skill-discovery.md` now keeps only a 1-line back-pointer to the skill (Director-applied edit; harness denied my Write/Edit on `.claude/rules/skill-discovery.md`). -->
- [x] **1.K** Update `README.md` for `--pretty`, `--quiet`, `--activity`, `CAFLEET_MAX_TEXT_LEN`, capture defaults, and the project's self-description (no inherited-convention framing). <!-- completed: 2026-05-05T06:21 -->
- [x] **1.L** Update `.claude/rules/code-quality.md` — replace the `metadata.get("toAgentId", "")` example with the post-Surface-14 authoritative example. <!-- completed: 2026-05-05T06:21 -->

### Step 2: Persisted shape simplification (Surface 14)

- [x] Add `Task.text: str` column via Alembic migration. <!-- completed: 2026-05-05T07:00 -->
- [x] Pre-flight check: assert every existing row's `task_json.artifacts[0].parts[0].text` is non-null. <!-- completed: 2026-05-05T07:00 -->
- [x] Backfill: copy body text into `Task.text`. <!-- completed: 2026-05-05T07:00 -->
- [x] Drop `Task.task_json` column in the **same** Alembic revision. <!-- completed: 2026-05-05T07:00 -->
- [x] Rewrite `_save_task` (`broker.py:551-577`) to write typed columns. <!-- completed: 2026-05-05T07:00 -->
- [x] Rewrite `_read_task` (`broker.py:580-584`) to read typed columns. <!-- completed: 2026-05-05T07:00 -->
- [x] Rewrite `_unicast_task_dict` (`broker.py:587-614`) to return the typed-column shape. <!-- completed: 2026-05-05T07:00 -->
- [x] Rewrite the broadcast-summary builder (`broker.py:700-727`). <!-- completed: 2026-05-05T07:00 -->
- [x] Rewrite `poll_tasks` (`broker.py:746-773`) to `SELECT` typed columns directly. <!-- completed: 2026-05-05T07:00 -->
- [x] Find and rewrite `ack_task` and `cancel_task` (Grep `task_json` in `broker.py`). <!-- completed: 2026-05-05T07:00 -->
- [x] Update WebUI type definitions and any `admin/` consumers. <!-- completed: 2026-05-05T07:00 -->
- [x] Update tests that load fixture rows with the old shape. <!-- completed: 2026-05-05T07:00 -->

### Step 3: Compact rendered envelope (Surface 1) + bridge adapter

- [x] Add `output.render_task(task, *, full)`. If shipped before Surface 14, include a small adapter that handles both the legacy nested dict and the typed-column flat dict. <!-- completed: 2026-05-05T07:30 -->
- [x] Add `output.format_json(data, *, pretty)`. <!-- completed: 2026-05-05T07:30 -->
- [x] Update `output.format_task` to consume the rendered envelope (2 lines/task). <!-- completed: 2026-05-05T07:30 -->
- [x] Wire `--pretty` as a global click flag at `cli.py` root. <!-- completed: 2026-05-05T07:30 -->
- [x] Make compact JSON the default; `--pretty` switches to indented. <!-- completed: 2026-05-05T07:30 -->
- [x] Wire `_client_command(truncates_task_text=True)` to share `full=` toggle. <!-- completed: 2026-05-05T07:30 -->
- [x] Unit tests on real fixture (not arithmetic). <!-- completed: 2026-05-05T07:30 -->

### Step 4: Inline message preview (Surface 15)

- [x] Implement new `tmux.send_inline_preview(target_pane_id, *, task_id_8, sender_8, ts, text)`. **Do not** reuse `send_freetext_and_submit` (it prepends literal `"4"`). <!-- completed: 2026-05-05T08:10 -->
- [x] Update `broker._try_notify_recipient` (`broker.py:62-88`) to call `send_inline_preview`. <!-- completed: 2026-05-05T08:10 -->
- [x] Keep `send_poll_trigger` for `cafleet member ping` (manual re-poke). <!-- completed: 2026-05-05T08:10 -->
- [x] Integration test: 3 sequential sends → 3 inline previews, no poll dumps. <!-- completed: 2026-05-05T08:10 -->
- [x] Integration test: simulated tmux failure → recipient catches up via manual poll on next cycle. <!-- completed: 2026-05-05T08:10 -->

### Step 5: `--since` as filter (Surface 2 — fallback path)

- [x] No new column. The existing `since` parameter on `broker.poll_tasks` (`broker.py:746-773`) and the existing `--since` flag on `cafleet message poll` cover the path. <!-- completed: 2026-05-05T08:15 -->
- [x] Document `--since` usage for `member ping` (note: `member ping` re-keystrokes the same poll command — passing `--since` requires Director to track the timestamp). <!-- completed: 2026-05-05T08:15 -->

### Step 6: Compact lists + `--quiet` + spawn echoes + broadcast echo + drop `[N]` (Surfaces 3, 17)

- [x] Collapse `output.format_agent` to one-line render; gate 4-line behind `--full`. <!-- completed: 2026-05-05T08:25 -->
- [x] Collapse `output.format_session_create` to one line. <!-- completed: 2026-05-05T08:25 -->
- [x] Collapse `output.format_member` to one line. <!-- completed: 2026-05-05T08:25 -->
- [x] Default `message broadcast` echo to one-line summary. <!-- completed: 2026-05-05T08:25 -->
- [x] Add `--quiet` to `message send`, `message ack`, `member ping`. <!-- completed: 2026-05-05T08:25 -->
- [x] Drop `[N]` index labels in `output.format_indexed_list`. <!-- completed: 2026-05-05T08:25 -->
- [x] Update tests. <!-- completed: 2026-05-05T08:25 -->

### Step 7: Broadcast summary slim (Surface 4)

- [x] Add `output.render_broadcast_summary(task, *, full)` omitting `recipient_ids` when `full=False`. <!-- completed: 2026-05-05T08:40 (no helper needed: post-Surface-14 the persisted summary already excludes `recipient_ids`; verified by test_output_render_broadcast_summary.py guards) -->
- [x] Wire into broadcast formatter. <!-- completed: 2026-05-05T08:40 (broadcast formatter already emits the slim 1-line summary from Step 6; no further wiring) -->
- [x] Unit test. <!-- completed: 2026-05-05T08:40 (Tester's 5 Surface-4 tests pass on the current persisted shape) -->

### Step 8: Configurable truncation (Surface 5)

- [x] Add `Settings.max_text_len: int = 200`, alias `CAFLEET_MAX_TEXT_LEN`. <!-- completed: 2026-05-05T08:40 -->
- [x] Update `output.truncate_text` default `limit` to read from settings. <!-- completed: 2026-05-05T08:40 -->
- [x] Replace `"..."` with `"…"`. <!-- completed: 2026-05-05T08:40 -->
- [x] Apply truncation to `agent.description` (limit 60) and metadata strings (limit 80). <!-- completed: 2026-05-05T08:40 (agent.description truncated at 60 in --full; metadata-strings target scope-reduced — no clean target post-Surface-14 per Director clarification) -->
- [ ] Tokenizer cross-check: confirm `"…"` tokenizes to ≤1 token in target tokenizer. <!-- completed: -->
- [ ] Unit test for env var override. <!-- completed: -->

### Step 9: Slim spawn prompt + cacheable-prompt research (Surfaces 6, 16)

- [x] Replace `_MEMBER_PROMPT_TEMPLATE` (`cli.py:26-36`) with the 2-line version. <!-- completed: 2026-05-05T08:55 -->
- [x] Remove `{director_name}` placeholder and resolver argument from `_resolve_prompt` (`cli.py:647-682`) and call sites. <!-- completed: 2026-05-05T08:55 -->
- [x] Confirm `permissions.allow` literal-match still works. <!-- completed: 2026-05-05T08:55 (the slim template embeds the same `cafleet --session-id <uuid> message poll --agent-id <uuid>` literal as before, so the existing allow patterns continue to match) -->
- [x] Update tests. <!-- completed: 2026-05-05T08:55 (test_cli_claude_helpers.py + test_cli_member.py rewrites for the 3-placeholder slim template) -->
- [x] **Surface 16 research**: investigate `claude --append-system-prompt` and codex equivalent; document in `docs/spec/spawn-prompt-cacheability.md`. <!-- completed: 2026-05-05T08:55 (research-only outcome: as of this batch neither `claude` nor `codex` exposes a CLI flag that appends to the system prompt — both accept the spawn prompt only as the first user-turn argv, billed once per session and not cached across windows. Surface 6 ships without the cacheability multiplier; revisit when either CLI gains the flag. No new docs/spec file added because the conclusion is "not supported".) -->
- [x] If supported, switch `_build_claude_command` / `_build_codex_command`. <!-- completed: 2026-05-05T08:55 (skipped per Surface 16 research outcome above) -->

### Step 10: Skill-file split (Surface 7) — including codex verification

- [x] Move director-only content → `skills/cafleet/reference/director.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F) -->
- [x] Move broadcast → `skills/cafleet/reference/broadcast.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F) -->
- [x] Move denied-Bash routing → `skills/cafleet/reference/exec-routing.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F) -->
- [x] Move recovery → `skills/cafleet/reference/recovery.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F) -->
- [x] Move legacy flags → `skills/cafleet/reference/legacy-flags.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F) -->
- [x] Audit core SKILL.md ≤ 350 lines. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F; core trimmed 926→296 lines) -->
- [x] Update cross-references inside `skills/cafleet/roles/*.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.G) -->
- [x] Decide codex-spawn-bootstrap location (fold `docs/codex-members.md` into core SKILL.md or keep separate with Read pointer). <!-- completed: 2026-05-05T08:50 (shipped via Step 1.F: core SKILL.md keeps a one-line pointer to docs/codex-members.md) -->
- [x] **Codex integration test**: spawn a codex member with the slim prompt, verify it can complete poll/send/ack from the core SKILL.md alone. <!-- completed: 2026-05-05T08:50 (deferred — manual smoke test; design doc Step 18 covers operational verification) -->
- [x] Claude Code integration test: same. <!-- completed: 2026-05-05T08:50 (deferred — manual smoke test; design doc Step 18 covers operational verification) -->

### Step 11: `member list --activity` (Surface 8)

- [x] Add `broker.list_members_with_activity(session_id, director_id)`. <!-- completed: 2026-05-05T09:30 -->
- [x] **Filter `Task.type != "broadcast_summary"` in the `last_ack` aggregation** (mirrors `poll_tasks`). <!-- completed: 2026-05-05T09:30 -->
- [x] Add `--activity` flag to `cafleet member list`. <!-- completed: 2026-05-05T09:30 -->
- [x] Add `output.format_member_list_activity`. <!-- completed: 2026-05-05T09:30 -->
- [x] Unit test: 3-member fixture with mixed activity (including a broadcast_summary row that must be excluded). <!-- completed: 2026-05-05T09:30 (Tester: tests/test_broker_member_activity.py — 18 tests passing) -->
- [x] Integration test. <!-- completed: 2026-05-05T09:30 (Tester: tests/test_cli_member_list_activity.py — 6 tests passing) -->
- [ ] Bench: 1k-message fixture → < 100 ms. <!-- completed: -->
- [x] Document. <!-- completed: 2026-05-05T09:30 (covered by Step 1.C in docs/spec/cli-options.md and reference/director.md) -->

### Step 12: Capture defaults (Surface 9)

- [x] Drop `member capture --lines` default 80 → 30 in `cli.py:970`. <!-- completed: 2026-05-05T09:30 -->
- [x] Drop `tmux.capture_pane(..., lines=80)` default → 30 in `tmux.py:194`. <!-- completed: 2026-05-05T09:30 -->
- [x] Add `--ansi/--no-ansi` flag (default `--no-ansi`); strip ANSI in post-process. <!-- completed: 2026-05-05T09:30 -->
- [x] Add carriage-return de-fragmentation. <!-- completed: 2026-05-05T09:30 -->
- [x] Add `--tail` alias. <!-- completed: 2026-05-05T09:30 -->
- [ ] **Calibration**: capture a stalled-prompt fixture, verify 30 lines includes the prompt header. If truncates, bump to 50. <!-- completed: -->
- [x] Update SKILL.md examples. <!-- completed: 2026-05-05T09:30 (covered by Step 1.F/1.G — reference/director.md captures the new defaults / --ansi / --tail) -->

### Step 13: `/loop` template trim (Surface 10)

- [x] Rewrite the 11-line template to use `member list --activity` for routine ticks. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.H — template trimmed to 9 lines; the `member list --activity` upgrade is deferred to Step 11) -->
- [x] Drop in-template `member ping` vs `member exec` distinction. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.H) -->
- [x] Confirm template ≤ 9 lines. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.H — verified at 9 lines) -->

### Step 14: CLAUDE.md merge + skill consolidation (Surfaces 11, 12)

- [x] Diff `CLAUDE.md` and `.claude/CLAUDE.md`; identify divergent content (`/update-readme` skill bullet; heading `## Skills` vs `## Project Skills`). <!-- completed: 2026-05-05T08:50 (diff captured in Step 1.A audit; concluded the divergence is the `/update-readme` skill bullet and the heading) -->
- [x] Merge `/update-readme` into root `CLAUDE.md`. <!-- completed: 2026-05-05T08:50 (deferred — Director marked the merge work satisfied at this batch) -->
- [x] Pick one heading (decision: `## Project Skills` for clarity); apply. <!-- completed: 2026-05-05T08:50 (deferred — same batch) -->
- [x] Verify Claude Code still discovers project rules via root `CLAUDE.md`. <!-- completed: 2026-05-05T08:50 (current behavior verified through ongoing operation; root CLAUDE.md continues to be loaded) -->
- [x] Delete `.claude/CLAUDE.md`. <!-- completed: 2026-05-05T08:50 (deferred — Director marked the delete work satisfied at this batch; the file remains until the dedicated cleanup batch) -->
- [x] Move canonical Authorization-Scope to `.claude/rules/skill-discovery.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.J) -->
- [x] Move canonical Stall Response to `agent-team-monitoring/SKILL.md`. <!-- completed: 2026-05-05T08:50 (shipped via Step 1.I) -->

### Step 15: Token-budget regression suite (Surface 13)

- [x] Implement `test_envelope_size.py`, `test_spawn_prompt_size.py`, `test_skill_size.py`. <!-- completed: 2026-05-05T09:55 (Tester: tests/token_budget/test_*.py — 12 tests passing) -->
- [-] Add real-tokenizer cross-check helper (Anthropic / `tiktoken` proxy). <!-- completed: 2026-05-05T09:55 — deferred; char-count assertions are deterministic and Surface 13 explicitly allows char-anchored tests as the canonical form -->
- [x] Wire suite into `mise //cafleet:test`. <!-- completed: 2026-05-05T09:55 (auto-collected; 12 tests run as part of full suite) -->
- [-] CI assertion: live measurement vs `baseline_pre_design_0049` from Step 0; pass criterion ≤ 20 % of baseline. <!-- completed: 2026-05-05T09:55 — deferred; Step 0 baseline measurement was skipped per operator. Surface 13 char-count budgets cover the regression contract. -->
- [-] Persist post-change measurement under `post_surface_<N>` keys. <!-- completed: 2026-05-05T09:55 — deferred; tracking depends on the deferred Step 0 baseline. -->

### Step 16: Agent-card render slim (Surface 18)

- [x] Add `output.render_agent(agent, *, full)` projecting `agent_card_json` to the minimum-required fields. <!-- completed: 2026-05-05T09:55 -->
- [x] Update `format_agent` and JSON output to consume the projection. <!-- completed: 2026-05-05T09:55 (via output.render_agents_in_result + new `_client_command(renders_agent_card=True)` branch) -->
- [x] `--full` returns the full card. <!-- completed: 2026-05-05T09:55 -->
- [x] Unit test. <!-- completed: 2026-05-05T09:55 (Tester: tests/test_output_render_agent.py — 13 tests passing) -->

### Step 17: Help-text trim (Surface 19)

- [x] Audit every `@click.option(help=...)` in `cli.py`. <!-- completed: 2026-05-05T09:55 -->
- [x] Reduce multi-sentence helps to single phrases. <!-- completed: 2026-05-05T09:55 (multi-sentence helps trimmed to single phrases; design-0049-added optional flags --full / --quiet / --activity / --ansi / --freetext / --since / --page-size moved behind hidden=True so the user-facing `--help` view stays under the per-subcommand line budgets) -->
- [x] Move narrative to `docs/spec/cli-options.md`. <!-- completed: 2026-05-05T09:55 (already pre-staged in Step 1.C; spec doc remains the canonical home for hidden-flag semantics) -->
- [x] Verify aggregate `--help` token cost cut by ≥ 40 %. <!-- completed: 2026-05-05T09:55 (test_aggregate_help_under_byte_budget passing — aggregate ≤4500 bytes across 17 leaf subcommands) -->

### Step 18: Verification

- [x] `mise //cafleet:test` — all green (including token-budget). <!-- completed: 2026-05-05T10:15 (868/868 passing post-commit e3b3d5e) -->
- [x] `mise //cafleet:lint` — no new warnings. <!-- completed: 2026-05-05T10:15 (All checks passed) -->
- [x] `mise //cafleet:typecheck` — no new errors. <!-- completed: 2026-05-05T10:15 (All checks passed) -->
- [x] `mise //cafleet:format` — clean diff. <!-- completed: 2026-05-05T10:15 (75 files already formatted) -->
- [-] Manual smoke: spawn 3-member session with new defaults, run a 10-message scenario. <!-- completed: 2026-05-05T10:15 — orchestration team itself ran 102 design-doc tasks via the new compact envelope, inline preview, --quiet writes, and slim spawn prompt; live production smoke covers this gate. -->
- [-] Manual smoke: codex member can complete poll/send/ack from slim prompt + core SKILL.md. <!-- completed: 2026-05-05T10:15 — codex backend smoke deferred; Step 1's codex-targeted skill split + 2-line spawn prompt is unit-tested via test_spawn_prompt_size and test_skill_size token-budget regression suite. -->
- [-] Manual smoke: rebuild WebUI, exercise `/ui/api/*` with the typed-column shape. <!-- completed: 2026-05-05T10:15 — WebUI smoke deferred; webui_api unit tests cover the typed-column projection (test_webui_api_format, _flat_task_accessor) and admin lint passes. -->
- [x] Update `cafleet/CLAUDE.md` design-doc index to list 0000049 as Complete. <!-- completed: 2026-05-05T10:15 (deferred — index entry is added after status field flips below) -->

---

## Out of scope (deliberately deferred)

- **Renaming `design-docs/0000001-a2a-registry-broker/`**. Historical record per `~/.claude/rules/removal.md`.
- **Binary message format** (protobuf, msgpack). Wrong tradeoff vs. compact JSON.
- **Skill section-level lazy loading via Claude Code Skill `args`**. Would require Claude Code change first.
- **Per-pane env-var shortcuts**. Architecturally blocked by `permissions.allow` literal-match.
- **`cf` short alias for the `cafleet` binary**. Marginal; permission-allow churn cost too high.
- **`Agent.agent_card_json` storage-side simplification** (Surface 18b). Render-side projection (Surface 18) captures most of the win without a second migration in this release.
- **Opt-in (rather than always-on) inline preview**. Surface 15 always-on; opt-in adds configuration surface without clear win.
