# Findings — Concepts slice (scanner-concepts)

Slice: `docs/concepts/{overview, fleet-isolation, storage, member-lifecycle,
coding-agents, bash-routing, tmux-push, monitoring, token-reduction}.md`

Headline: the concepts surface is the worst offender for **cross-file
redundancy**, not in-page filler. The monitoring story (the *when/what* split,
the 180 s/720 s watched set, the `Esc` safeguard) is retold nearly verbatim
across **five** pages: overview, monitoring, tmux-push, token-reduction, and
partly coding-agents. `monitoring.md` and `tmux-push.md` each independently
explain the `Esc`-before-Enter keystroke in full. Diagrams are all worth
keeping. The biggest single win is to make monitoring.md the *one* canonical
home and demote every other mention to a one-line pointer.

---

## overview.md

- **KEEP** — front-matter, intro paragraph (lines 7–12), Core terms table,
  architecture diagram. These are the genuine job of an overview page.
- **CUT** — lines 50–51 ("The broker package is the single data access
  layer… No async stores, no HTTP client, no protocol layer."). Restates the
  intro ("access SQLite directly through a shared broker package — no HTTP
  server is needed") and the diagram caption. Redundant.
- **REWRITE** — the **Monitoring** section (lines 74–85) is a 12-line
  paragraph that duplicates monitoring.md almost sentence-for-sentence
  (180 s/720 s, the *when/what* split, "plain loop not agent reasoning",
  "works the same on any backend"). On an overview page this should be 2–3
  sentences + the link. Cut the interval numbers and the when/what mechanics
  here; they belong on monitoring.md.
- **REWRITE** — the **WebUI** section (lines 60–69): the Discord-style
  timeline description is one long 9-line sentence. Tighten to ~3 lines; the
  per-agent route detail (`#/fleets/<fleetId>/agents/<agentId>`) is reference
  trivia that belongs on webui-api.md, not the overview.
- **CUT** — the **Design document orchestration skills** section (lines
  87–95). This is promotional prose about skills, not an architectural
  concept, and it editorializes ("an auditable trail, in contrast to the
  ephemeral in-memory coordination of Agent Teams"). At most a one-line
  pointer in Core terms or get-started; does not earn a top-level section on
  the concepts overview.

## fleet-isolation.md

- **CUT/MERGE** — the page is six thin sections, four of which are 1–2
  sentence pointer stubs to CLI options: **Fleet bootstrap**, **Fleet
  soft-delete**, **Root Director protection**, **Built-in Administrator
  agent**. Collapse these four into a single short "Lifecycle" paragraph (or
  a 4-row table: bootstrap / soft-delete / Director protection / Administrator
  → one-line behavior + CLI-options link each). Four `##` headings for four
  sentences is structural over-explanation.
- **CUT** — redundant restatement of "fleet_id is non-secret": stated at
  lines 7–10 (intro) and again at lines 12–14 ("The fleet_id is a non-secret
  fleet identifier. Fleets are partitions for tidiness, not security
  boundaries."). Keep one. The "partitions for tidiness, not security" framing
  is the keeper; fold it into the intro.
- **CUT** — **Registration** section (lines 16–19) repeats "Fleets are
  created via `cafleet fleet create`" a third time (already in intro line 8
  and again line 9). The only new fact is "non-soft-deleted fleet_id required"
  — fold that single clause into the Isolation rules or Lifecycle paragraph.
- **KEEP** — **Isolation rules** (lines 21–24): the "not found
  indistinguishable from non-existence" rule is the real content of this page.

## storage.md

- **KEEP** — **Backend**, **Schema management**, **No physical cleanup**.
  These carry real operational facts (XDG path, env override, `db init`
  idempotency/refusal behavior, dead-row filtering) found nowhere else.
- **REWRITE** — **Concurrency** (lines 20–24): trim. "Expected contention is
  low — CLI operations are short transactions (single INSERT or UPDATE), and
  multiple agents polling concurrently is read-only" is justification prose.
  One sentence suffices: "`PRAGMA busy_timeout=5000` lets SQLite retry up to
  5 s before `SQLITE_BUSY`; contention is low (short single-statement
  transactions)."
- **REWRITE** — **contextId convention** (lines 48–55): the middle two
  sentences explain the trade-off twice ("This trades per-conversation
  grouping… for simple inbox discovery, which suits the fire-and-forget
  messaging pattern"). Tighten to the rule + one rationale clause.
- **REWRITE** — **Relational model** (lines 26–30) is mostly a pointer to
  data-model.md; the one durable fact is "only JSON blob is
  `agents.agent_card_json`". Compress to two lines.

## member-lifecycle.md

- **KEEP** — intro, **Single-Director invariant**, the lifecycle state
  diagram, **Atomic create flow**, **Delete ordering**. Real, non-redundant
  mechanics.
- **MERGE** — **Terminology** (lines 11–15) overlaps the intro (both define
  "member" via `member create` + placement row). Fold the one new fact ("the
  Director is NOT a member; it uses plain `agent register`") into the intro
  and drop the separate Terminology block.
- **REWRITE** — **Commands** (lines 74–86): this enumerates every subcommand
  and re-explains `--agent-id` vs `--member-id` resolution, which is exactly
  what cli-options.md owns. Cut to: "The subgroup is `member
  create/delete/list/capture/send-input/exec/ping`; see CLI options for flags
  and member-resolution rules. `member exec` is the bash-routing primitive
  (see Bash routing)." The `--activity` column list is duplicated on
  token-reduction.md — drop it here.
- **CUT** — **Pane display name** (lines 70–72): one-sentence pointer to
  coding-agents.md for an asymmetry that coding-agents.md already documents in
  full. Redundant; delete and let coding-agents.md own it.

## coding-agents.md

- **KEEP** — **Backend resolution** table, **Known asymmetries** list. These
  are the canonical home for per-backend spawn commands and the
  pane-title/sandbox/bash-disable asymmetries.
- **REWRITE** — the post-table paragraph (lines 20–39) is dense and partly
  redundant: the leading-`!` shortcut is re-explained on bash-routing.md and
  tmux-push.md; the `--coding-agent` recording is restated three times
  (table Notes + lines 23–24 + lines 31–39). The **`--role monitor` backend
  inheritance** paragraph (lines 35–39) is duplicated almost verbatim in
  monitoring.md (lines 110–115). Keep the canonical statement in ONE place
  (recommend coding-agents.md owns spawn/flag mechanics; monitoring.md
  pointer-links it) and cut the other.
- **REWRITE** — **Model selection** (lines 42–60): three paragraphs for one
  flag. The per-backend example list and the "spawn-time only, not recorded,
  no migration, not in member list" enumeration read like a spec entry; this
  is a candidate to compress to a short paragraph + defer exhaustive detail to
  cli-options.md `member create`.

## bash-routing.md

- **KEEP** — the two-paragraph protocol (members auto-approve → deny-list
  fallback → Director `member exec` dispatch → reconsider-first). This is the
  conceptual core and is reasonably tight already.
- **CUT/MERGE** — the first paragraph (lines 7–15) re-enumerates the
  per-backend auto-approval flags (`--permission-mode dontAsk`,
  `--ask-for-approval never --sandbox workspace-write`, `--agent cafleet`)
  that coding-agents.md's table already owns. Replace with "Members spawn with
  workspace-scoped auto-approval (see Coding agents for the per-backend
  flags)." Keeps this page about *routing*, not spawn flags.
- **KEEP** — the closing pointer to `skills/cafleet/reference/exec-routing.md`
  for the full convention. Good single-source deferral.

## tmux-push.md

- **KEEP** — intro (pull-based + inline-preview optimization), the sequence
  diagram, the literal preview block (lines 49–52), **Design principles**
  (best-effort / self-send skip / silent failure / no-TMUX-var), the
  response-annotations note. This is the canonical home for the push
  mechanism.
- **CUT (major redundancy)** — the **`Esc` safeguard** is explained in full
  here (lines 60–65) AND again in full on monitoring.md (lines 20–33). Pick
  ONE canonical home. Recommend: tmux-push.md owns the keystroke-mechanics
  `Esc` explanation (it owns keystrokes); monitoring.md keeps only the
  *exception* (wake nudge omits `Esc`) as a one-liner pointing here. Today
  both pages explain the full rule AND the exception — cut one copy.
- **REWRITE** — lines 38–48 ("After the broker saves a delivery task…")
  re-explain placement-row resolution that tmux-push.md's own diagram and the
  member-lifecycle/data-model pages already cover. Tighten; the durable new
  fact is "root Director's placement carries `director_agent_id=NULL`".
- **REWRITE** — **Manual entry-point** (lines 95–99) overlaps the
  fallback-chain paragraph (lines 67–73): both describe `cafleet member ping`
  injecting the poll command. Merge into one description of `member ping`.

## monitoring.md

This is the single most over-explained page in the slice. The *when/what*
split, the `Esc` rule + its wake-nudge exception, and the watched-set cadence
are each stated **three or more times** across the page's sections.

- **KEEP** — the two diagrams (lifecycle flowchart + the Heartbeat/Facilitation
  table), **The watched set** core facts, **Single-instance and liveness**,
  **Cadence and tick precision** table, **Enrollment and schema**. The schema
  and liveness/ownership-checking content is canonical and lives nowhere else.
- **CUT (repetition)** — the **`Esc` safeguard** paragraphs (lines 20–33)
  duplicate tmux-push.md's full explanation. Cut to one sentence: "The wake
  nudge is the one keystroke that does NOT lead with `Esc` (the monitoring
  member's own pane is never on a permission prompt); every other delivery
  keystroke does — see tmux push." Delete the rest.
- **CUT (repetition)** — the *when vs what* / "alarm clock not the worker"
  framing is restated in the intro, in **Heartbeat vs facilitation** (table +
  prose), and again at lines 66–67 and 86–90. Keep the table; cut the
  surrounding prose restatements.
- **REWRITE** — **The monitoring member** (lines 107–158): the backend-
  inheritance sentence (110–115) duplicates coding-agents.md — make it a
  pointer. The 4-step routine is useful but the trailing two paragraphs
  (147–158) re-explain "observation spans all, actuation is Director-only" and
  "wake lands in own pane, no `Esc`" which are already stated above. Cut the
  restatements; keep the numbered steps.
- **REWRITE** — **The watched set** (lines 70–94): the "loop never keystrokes
  a watched pane" rule appears at lines 86–90 and is implied again in
  Lifecycle. State once.

## token-reduction.md

- **KEEP** — the **techniques table** (lines 24–33). It is genuinely the
  page's value: a compact catalog of cost-reduction touch-points found nowhere
  else in one place.
- **REWRITE (major)** — the intro paragraph (lines 7–22) is one 16-line
  sentence-pile that re-narrates the entire monitoring model (180 s/720 s,
  wake-nudge, on-demand re-engagement, "no agent carries a scheduling prompt").
  This is the fourth full retelling of monitoring in the slice. Cut to ~4
  lines: "CAFleet spends no tokens itself, but every byte it emits lands in a
  coding agent's context. Moving supervision scheduling into the monitor
  process (see Monitoring) removes the per-tick scheduling prompt from every
  agent. The table below catalogs the per-message / per-spawn / per-context
  cost reductions." Drop the duplicated interval/wake-nudge mechanics.
- **CUT** — within the table, the **Inline message preview** row (line 31)
  re-describes the mechanism already owned by tmux-push.md; compress its cell
  to the one-line technique + link, drop the re-explanation of the poll-trigger
  distinction.

---

## Cross-cutting observations (seeds for debate)

1. **Monitoring is told 4–5 times.** overview.md §Monitoring, monitoring.md
   (whole page), token-reduction.md intro, and partly coding-agents.md
   (`--role monitor` inheritance) all retell the 180 s/720 s watched set + the
   when/what split. **Policy proposal:** monitoring.md is the ONE canonical
   home. Every other page gets a ≤2-line summary + link, with NO interval
   numbers or when/what mechanics repeated.

2. **The `Esc` safeguard is explained in full twice** (tmux-push.md lines
   60–65 and monitoring.md lines 20–33), and its wake-nudge exception twice
   more. **Policy proposal:** tmux-push.md owns the keystroke `Esc` mechanics;
   monitoring.md states only the exception in one sentence + link.

3. **Per-backend spawn flags are restated on three pages** (coding-agents.md
   table, bash-routing.md para 1, tmux-push/monitoring auto-approval mentions).
   **Policy proposal:** coding-agents.md's Backend-resolution table is the
   single source; other pages link to it instead of re-listing
   `--permission-mode dontAsk` / `--ask-for-approval never` / `--agent cafleet`.

4. **Thin pointer-stub sections.** fleet-isolation.md (4 sections), member-
   lifecycle.md §Pane display name, storage.md §Relational model are
   one/two-sentence sections whose only content is "see CLI options /
   data-model." **Policy proposal (repo-wide):** a concept page should not
   create a `##` heading whose body is solely a cross-link. Fold such stubs
   into a sentence in an adjacent section or a small table.

5. **`--activity` column list duplicated** (member-lifecycle.md line 76–77 and
   token-reduction.md line 29). Pick one owner (token-reduction.md's table is
   fine); the other links.

6. **Voice/length target proposal for a concept page:** lead with one
   intro paragraph stating the concept, then the mechanics that live ONLY on
   this page, then diagrams. No re-justification prose ("this suits the
   fire-and-forget pattern…"), no promotional editorializing ("an auditable
   trail, in contrast to…"), no re-narrating a concept that owns its own page.
   Cross-references are one-liners, never their own section.

7. **Diagrams: keep all.** overview architecture flowchart, member-lifecycle
   state diagram, tmux-push sequence diagram, monitoring lifecycle flowchart +
   Heartbeat/Facilitation table all earn their place. No diagram cuts proposed.

---

## CONSENSUS

All three scanners (concepts/411, api/409, spec/410) converged. Agreements:

1. **Audience/boilerplate preamble — cut entirely, zero surviving copies, no
   new page.** The 4-line "Like every API page…" preamble is deleted from all
   four `api/*` pages. We do NOT add an `api/index.md` to host it — the
   existing `index.md` "Browse the docs" list already routes audiences
   implicitly (the Specification bullet points CLI users at CLI options, the
   API bullet points Python users at the modules). Nothing to preserve.
   **Corollary (repo-wide):** every reference/concept page opens with ONE lead
   sentence stating what the page is (audience implicit); there is **no**
   standalone audience line and no standalone "Read this page to X" line on any
   page — fold that cue into the lead sentence. This makes api pages consistent
   with concepts pages, which already carry no separate audience line.

2. **Single-source ownership map (confirmed, with additions).** Each repeated
   topic has exactly one canonical home; every other mention is a ≤2-line
   pointer:
   - monitoring model (when/what split, 180 s/720 s watched set) →
     `concepts/monitoring.md`. The intervals and the when/what mechanics appear
     **only** there with NO numbers repeated elsewhere. Demoted to pointers:
     `overview.md §Monitoring`, `token-reduction.md` intro, `coding-agents.md`
     `--role monitor`, plus the spec-side leaks scanner-spec tallied
     (`cli-options` monitor group + `monitor start`, `member ping` desc,
     `member create --role` Notes; `webui-api` monitor field; `data-model`'s 4×
     watched-set restatement).
   - `Esc` keystroke mechanics → `concepts/tmux-push.md`; `monitoring.md` keeps
     only the wake-nudge exception as one line + pointer.
   - per-backend spawn flags / auto-approval **and the full spawn-argv table**
     → `concepts/coding-agents.md` (its Backend-resolution table is canonical).
     `bash-routing.md` para 1, `cli-options` spawn-command table, and
     `codex.md`/`opencode.md` Spawn-flags sections demote to pointers + genuine
     deltas only.
   - `--role monitor` backend inheritance → `concepts/coding-agents.md`
     (it is a spawn mechanic); `monitoring.md` and `cli-options` point to it.
   - error strings → `spec/cli-options.md` Error Messages table.
   - fleet-id literal-flag rationale → `spec/cli-options.md` "Fleet ID".
   - persisted task columns → `spec/data-model.md#tasks`;
     `message-envelope.md` keeps only the rendered projection.
   - **contextId routing rationale (the WHY) → `concepts/storage.md`;**
     `data-model.md` states the `context_id` column only, no rationale.
   - **CAFLEET_* field defaults + aliases → `api/config.md`;** `storage.md`
     keeps `CAFLEET_DATABASE_URL` concretely (its own subject) but points to
     `config.md` for the full var set rather than enumerating others.
   - `--activity` column list → `concepts/token-reduction.md` table;
     `member-lifecycle.md` points to it.

3. **Backend verification recipes (codex.md / opencode.md): KEEP verbatim.**
   They are runnable copy-paste smoke tests; a pointer is worse and
   half-trimming into non-runnable fragments is the worst outcome.

4. **Repo-wide reference voice/length policy (all three sign):** A reference
   page states the current surface in tables; prose only where a table cannot
   carry the meaning. Each fact, error, and rationale has exactly one home;
   every other mention is a one-line pointer. Every page opens with one lead
   sentence stating what it is (audience implicit) — no standalone audience or
   "read this to" line. No migration/history narration, no promotional
   editorializing, no `##` heading whose body is only a cross-link. Diagrams
   are kept.

## OPEN QUESTIONS FOR USER

none
