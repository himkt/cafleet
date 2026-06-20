# Findings — scanner-spec (specification + operational reference slice)

Slice: `docs/spec/cli-options.md`, `docs/spec/data-model.md`,
`docs/spec/message-envelope.md`, `docs/spec/webui-api.md`,
`docs/reference/coding-agents/codex.md`, `docs/reference/coding-agents/opencode.md`.

Voice target I am proposing for every reference page in this slice: **state the
current CLI/API surface in tables; use prose only where a table cannot carry the
meaning; never narrate migration history or restate a rationale that already has
a canonical home; state each error exactly once.**

---

## docs/spec/cli-options.md (1032 lines — the dominant wordiness source)

This page is long for two different reasons: it is *exhaustive* (legitimate for a
reference) and it is *repetitive* (not legitimate). The fix is to kill the
repetition and push concept-narration back to the concepts pages, NOT to split
into many files. After the cuts below the page drops a few hundred lines while
staying a complete flag reference.

### CUT
- **The triple-stated `--fleet-id`-is-a-literal-flag rationale.** The same
  "permissions.allow matches literal strings, so use literal ids not shell vars"
  argument is written three times: the blockquote at L55–57, again in the "Fleet
  ID" section L88–92, and again in "`permissions.allow` coverage" L98/L128.
  Keep ONE canonical statement (the blockquote at L55–57) and reduce the other
  two to a one-line pointer. Rationale: a reader meets the identical argument
  three times in one page.
- **Inline per-subcommand error restatements that duplicate the Error Messages
  table (L990–1031).** Every error in that 40-row table is also stated inline in
  the subcommand body: `member exec` errors at L783–785 AND L1007–1009;
  `member send-input` validation at L715–720 AND L1002–1006; `member ping`
  pending-placement at L1011 AND inline L848; `member nudge` at L916 AND
  L1013–1016; `member create --prompt-file` errors at L572 (prose) AND
  L1017–1026; root-Director / Administrator deregister guards at L249 AND
  L999–1000. Pick the consolidated table as the single home and delete the
  inline duplicates (leave a one-line "see Error Messages" pointer in each
  subcommand). Rationale: the same ~15 error strings are written twice.
- **Conceptual narration inside `monitor start` (L931–937) and the `monitor`
  group intro (L921–923).** Three dense paragraphs re-explain the watched set,
  the wake nudge, the "does NOT lead with Esc" reasoning, single-instance claim,
  tick precision — all canonical in `concepts/monitoring.md` (the page even says
  so). A CLI-surface reference needs: what `start` runs, its one flag, its exit
  codes, the no-monitoring-member warning. Cut the concept prose to a pointer.
- **`member ping` description paragraph (L819).** One ~12-line paragraph mixes
  the CLI action (Esc → poll command → Enter) with concept narration (the
  monitor-loop contrast, the Esc-safeguard placement rationale). Keep the action;
  cut the concept narration to a pointer to `monitoring.md`.
- **`member nudge` "Behavior" prose (L888–890) and the long description
  (L873–875)** restate the Esc-safeguard / "functionally equivalent to message
  send" story already told for `message send` and in `tmux-push.md`. Condense to:
  persists an ACKable task + fires the hardened inline preview; see message send.

### REWRITE
- **`permissions.allow` coverage (L98–140).** Currently enumerates all 20 base
  patterns AND 8 `--json` companion patterns as literal blocks (≈40 lines).
  Replace with: the rule (one pattern per allow-listed subcommand, canonical
  `--fleet-id`-first order, `member exec` excluded, `--json` needs a companion
  because it precedes the subcommand) + 3 representative example lines + the note
  that the full set is mechanical. A reader does not need all 28 lines pasted;
  they need the generation rule.
- **`member create` `--coding-agent` (L545) and `--role` (L547) Notes cells.**
  Each is a paragraph-length cell duplicating `coding-agents.md` + `monitoring.md`
  (backend inference, monitor inheritance, enrollment skip, kind marker, the
  "second monitor rejected" rule). Trim each to the flag's own behavior + a
  pointer; the concept lives on the concepts page.
- **The six `message {send,ack,cancel,show,poll,broadcast}` subcommand bodies
  (L469–532).** Each repeats "Text output is `<verb>` followed by the compact
  envelope; `--quiet` prints only the task id." Collapse the shared output
  contract into one sentence above the six flag tables; keep only each
  subcommand's unique flags.

### KEEP
- The **Subcommand summary** table (L13–42) — genuine high-value index. KEEP.
- The **`--full` semantics** cross-subcommand table (L77–86) — the one place the
  overloaded flag is reconciled. KEEP.
- The **Error Messages** catalogue (L990–1031) — make it the single home (see
  CUT above). KEEP as the canonical error reference.
- Per-flag **Required/Notes tables** — these are the reference's reason to exist;
  they overlap `--help` but a reference page legitimately documents flags. KEEP.

### Split decision
**Do not split into many pages.** The length is repetition + concept-leak, not
genuine surface. Optionally the Error Messages catalogue could become its own
short `cli-errors.md`, but that is cosmetic; condensing in place is sufficient.

---

## docs/spec/data-model.md

### CUT
- **Duplicate opening (L7 and L9).** "The `Task` payload is fully relational…"
  and "The model is **predominantly relational**…" state the same fact twice in
  consecutive paragraphs. Cut one (keep L7, drop L9 or fold its index/blob detail
  into L7).
- **Enrollment-inversion data migration section (L151–162),** including the raw
  `DELETE FROM monitor_config …` SQL. This narrates one historical Alembic
  revision (`0005`) — migration archaeology. The *current* enrollment rule is
  already stated at L130. Per the repo removal/affirmative-writing philosophy,
  migration history belongs in the design doc, not the data-model spec. Cut to at
  most one sentence ("enrollment is backfilled by revision 0005 for upgraded DBs")
  or remove entirely.
- **"Deregistered Agents" section (L215–217)** restates the soft-delete behavior
  already given in the `agents` table notes (L52). Redundant tail section — fold
  the one new fact (WebUI still surfaces them) into the `agents` section and drop
  the standalone section.

### REWRITE
- **`interval_seconds` Note (L126)** narrates "the column `DEFAULT 60` is the
  frozen schema default written by migration `0002`; it is never relied upon…"
  — history again. State the current behavior: every enrollment writes an
  explicit interval (Director 180 s, member 720 s); the schema default is inert.
  Drop the migration-number archaeology.
- **The watched-set definition is stated four times** (L121, L126, L130, plus the
  kind-marker section L134–149): "root Director 180 s + every ordinary member
  720 s; monitoring member not enrolled." State it once (the `monitor_config`
  intro) and reference it; the schema-table Notes cells should not re-derive it.

### KEEP
- All five **schema tables** (`fleets`, `agents`, `tasks`, `agent_placements`,
  `monitor_config`, `monitor_runtime`) — these are the canonical schema and earn
  every row. KEEP.
- The **AUTOINCREMENT / id-never-reused** explanation (L15) — non-obvious, stated
  once, justifies the `0` sentinel. KEEP.
- **Task Visibility Rules** and **Broadcast Grouping** tables — unique, precise.
  KEEP.

---

## docs/spec/message-envelope.md

### CUT
- **L9 redundant restatement.** "This document covers the canonical envelope
  shape: the broker constructs it, the CLI renders it, and the WebUI consumes it."
  repeats the one-line intro at L5–7. Cut.

### MERGE
- **The "Persisted shape (typed columns)" table (L15–26) duplicates the `tasks`
  table in data-model.md** almost column-for-column. This page's unique value is
  the *rendered projection* (default vs `--full`), not the persisted columns.
  Proposal: replace the full persisted-columns table with a one-line pointer to
  `data-model.md#tasks`, and keep only the rendered-shape content here. (Debate
  point: scanner covering data-model should weigh in on which page owns the
  column list.)

### KEEP
- **Compact rendered envelope** field-decision table (L40–51) and **Text mode**
  rules (L96–106) — this is the page's reason to exist; nowhere else documents the
  default→rendered projection. KEEP.
- The default/`--full` JSON examples (L64–93) — concrete, useful. KEEP.

---

## docs/spec/webui-api.md

### CUT
- **The `409 (reserved for future deregister endpoint)` paragraph (L314).**
  Documents the required error mapping for an endpoint that does not exist
  ("This 409 is not currently reachable…this entry documents the required mapping
  for the future endpoint"). Speculative future-state narration — belongs in a
  design doc, not the current API reference. Cut.
- **The `monitor`-field watched-set explanation (L89–97)** re-derives the
  enrollment rule (Director 180 s / member 720 s / monitoring member unenrolled)
  that is canonical in `monitoring.md` and `data-model.md`. Trim to the field
  shape + a pointer.

### KEEP
- Every **endpoint spec** (request headers, JSON request/response bodies, error
  codes) — this is a genuine API reference, all earns its place. KEEP.
- The shared-row-formatter note (L212) that inbox/sent/timeline share a field set
  — good de-duplication already in place. KEEP.

---

## docs/reference/coding-agents/codex.md & opencode.md (treat together — heavy mutual duplication)

These two pages share four near-verbatim sections. The shared material should be
factored to ONE home (the `coding-agents.md` concepts page or a shared snippet),
with each backend page keeping only its genuine deltas.

### MERGE (across the two files)
- **"cafleet usage from inside a codex/opencode pane"** — codex L54–67 and
  opencode L68–81 are nearly identical: "members can't load Claude Code skills,
  read skill files by absolute path; the same CLI works unchanged; substitute
  literal ids; see CLI options." Plus an identical 3-line poll/send/ack example
  block. Factor into one shared statement; each page keeps only what differs
  (nothing differs today). Rationale: same paragraph written twice.
- **"The `!` shell-shortcut convention"** — codex L69–71 and opencode L83–85 both
  say "this backend honors leading-`!`; cafleet's bash-via-Director fallback uses
  it; see Bash routing." One shared line.
- **"Pane-title asymmetry"** — codex L73–75 and opencode L87–89 are the same line
  ("only `claude` sets the pane title; locate panes via `member list`"). Already
  canonical in `coding-agents.md`; reduce both to a pointer or drop.
- **Overview + default-`--coding-agent`-claude + "single Director may spawn all
  three"** — codex L11–20 and opencode L11–22 repeat the same framing. Keep one
  canonical framing on the concepts page; each backend page opens with just its
  own definition line.

### REWRITE
- **Verification recipes** (codex L77–107, opencode L108–148) are long
  copy-paste smoke tests. They are genuinely useful as runnable scripts, so I
  lean KEEP — but they are a debate point: a reference reader may only need a
  pointer to "the manual smoke test in the design doc." If we keep them, keep
  them verbatim (they are runnable); do not half-trim them into non-runnable
  fragments. Flagging for consensus.

### KEEP (genuine per-backend deltas — do NOT cut)
- codex: the `--sandbox workspace-write` **writable_roots / SQLite DB** IMPORTANT
  callout (L34–43) — backend-specific gotcha, high value. KEEP.
- codex: required-version + binary-not-on-PATH (L46–52). KEEP (trim shared boiler
  if any).
- opencode: the **`cafleet` agent preset** mechanics (L38–56, skip-if-exists,
  refresh procedure), **`--model` `<provider>/<model>` format rule** (L34),
  **permission-popup recovery posture** (L91–93), **safety-floor caveats** (MCP,
  shell wrappers) (L95–102), **"writes one file under $HOME"** (L104–106). These
  are real, backend-unique operational facts. KEEP.

---

## Cross-cutting observations (seeds the debate)

1. **Single-home the `--fleet-id` literal-flag rationale.** It is stated 3× in
   cli-options alone and is very likely echoed in `concepts/` pages and SKILL.md.
   Repo-wide rule: ONE canonical statement (cli-options "Fleet ID"), one-line
   pointers everywhere else. Peers covering concepts/SKILL should confirm where
   the echoes live.

2. **Errors stated once, in a table.** cli-options duplicates ~15 error strings
   inline AND in the Error Messages table. Repo-wide rule: each error string has
   exactly one home (the consolidated table); subcommand bodies point to it.

3. **Spec pages document the CLI/API surface; concepts pages own the "why."**
   `monitor start`, `member ping`, `member create --role`, and webui-api's
   `monitor` field all leak monitoring-concept narration into spec pages that
   explicitly claim to be "CLI surface only." Repo-wide rule: spec pages state
   behavior + flags + exit codes; concept narration lives in `concepts/` with a
   pointer. This is the single biggest line-count win across the slice.

4. **No migration archaeology in spec pages.** data-model narrates revisions
   `0002` and `0005` with raw SQL. Migration history is design-doc/git territory;
   the spec states current behavior only. (Aligns with the repo's removal rule.)

5. **De-duplicate the persisted-columns table** between message-envelope and
   data-model — pick one owner (proposal: data-model owns columns, envelope owns
   the rendered projection).

6. **Backend reference pages share ~4 boilerplate sections** (codex/opencode).
   Factor shared "usage from a non-claude pane / `!` shortcut / pane-title /
   overview framing" to the concepts page; backend pages keep only deltas.

7. **Repo-wide policy positions I am bringing to the debate:**
   - Audience/boilerplate preamble: **cut to a single shared one-liner**, owned by
     the relevant concept page, pointers elsewhere.
   - cli-options: **condense in place, do not split** (error catalogue split is
     optional/cosmetic); the win is cutting repetition + concept-leak.
   - Diagrams: **keep** (per user).
   - Reference voice: **tables for surface, prose only when a table can't carry
     it, zero historical narration, each fact/error stated once.**

---

## CONSENSUS

Reached with scanner-api (409) and scanner-concepts (411) over two peer exchanges
each. All three findings files carry the same agreed answers; zero open questions.

### Q1 — Audience/boilerplate preamble
**DELETE the 4-line audience preamble from all four `docs/api/*` pages with ZERO
surviving copies.** No new `api/index.md`, no per-page one-liner. I conceded my
"single shared one-liner owned by the page" position: a one-liner repeated on four
pages is still four homes, which violates our own one-home rule (scanner-api caught
the contradiction). The preamble's only load-bearing content is the "CLI users find
the command surface in CLI options" cross-ref, and `index.md`'s Browse-the-docs nav
already routes that audience. The home for that cross-ref is the index nav — zero
per-page prose.

### Q2 — Single-source ownership map (confirmed, with my slice's amendments)
Every repeated topic has exactly ONE canonical home; every other mention is a
≤2-line pointer.

| Topic | Canonical home |
|---|---|
| Monitoring model (when/what split, 180 s/720 s watched set) | `concepts/monitoring.md` — interval numbers + when/what mechanics appear NOWHERE else |
| `Esc` keystroke mechanics | `concepts/tmux-push.md`; `monitoring.md` states only the wake-nudge exception (one line) |
| Per-backend spawn-argv table + auto-approval flags + `--role monitor` backend inheritance | `concepts/coding-agents.md` (Backend-resolution table) |
| Error strings | `spec/cli-options.md` Error Messages table |
| `--fleet-id` literal-flag rationale | `spec/cli-options.md` "Fleet ID" section |
| Persisted task columns | `spec/data-model.md` #tasks (message-envelope keeps only the rendered projection) |
| contextId routing **rationale** (the WHY) | `concepts/storage.md`; data-model states the `context_id` column existence only |
| `CAFLEET_*` field → default → alias catalog | `api/config.md`; cli-options keeps operational behavior + precedence and points there |
| `--activity` column list | `concepts/token-reduction.md` table |
| Backend "usage from a non-claude pane / `!` shortcut / pane-title / overview framing" | `concepts/coding-agents.md`; `codex.md`/`opencode.md` keep only genuine deltas |

**My-slice demotions that follow from the map:**
- `cli-options.md` "Spawn command per backend" table (L553–557) → pointer to
  `coding-agents.md`.
- `codex.md` / `opencode.md` "Spawn flags" sections → pointers + genuine deltas
  only (codex `writable_roots` SQLite-DB gotcha; opencode `--model`
  `<provider>/<model>` format rule + the `cafleet` agent preset).
- `cli-options.md` monitor group + `monitor start` (L919–937), `member ping`
  (L819), `member create --role` Notes (L547) → CLI surface (flags/exit codes)
  only; concept narration → pointer to `monitoring.md`.
- `webui-api.md` `monitor` field (L89–97) and `data-model.md` watched-set
  restatements (L121/126/130 + kind section) → pointer to `monitoring.md`, no
  interval numbers.
- `cli-options.md` env-var defaults → point to `api/config.md`, EXCEPT the one
  default a section is actually about: the `server` section keeps `127.0.0.1`/`8000`
  inline where it states the flag-beats-env-beats-default precedence rule (a page
  keeps the single default it is about — same shape as `storage.md` keeping
  `CAFLEET_DATABASE_URL`). No parallel exhaustive env-var catalog in cli-options.
- Inline per-subcommand error restatements → deleted; the Error Messages table is
  the single home.
- The `--fleet-id` rationale's three copies in cli-options collapse to one in the
  "Fleet ID" section; the blockquote and permissions.allow section point to it.

### Q3 — Backend verification recipes
**KEEP the codex.md / opencode.md copy-paste smoke tests verbatim.** A runnable
smoke test is genuine content; trimming it to a pointer destroys its value and
half-trimming into non-runnable fragments is the worst outcome. Unanimous.

### Q4 — Repo-wide reference voice/length policy (all three sign)
"A reference page states the current surface in tables; prose only where a table
cannot carry the meaning. Each fact, error, and rationale has exactly one home;
every other mention is a one-line pointer. No migration/history narration, no
promotional editorializing, no `##` heading whose body is only a cross-link.
Diagrams are kept."

Spec-slice corollary I add: spec pages document the CLI/API **surface** (flags,
output shapes, exit codes, errors); the "why" lives in `concepts/` with a pointer.

## OPEN QUESTIONS FOR USER

none.
