# Findings — API-reference landing surface (scanner-api)

Slice: `docs/api/broker.md`, `docs/api/coding-agent.md`, `docs/api/config.md`,
`docs/api/multiplexer.md`, `docs/index.md`.

Headline: the four `api/*` pages are thin wrappers around a mkdocstrings
`::: cafleet.<module>` directive. Their only hand-written prose is a 2–4
sentence intro, and **every one of them ends with the identical 4-line
audience-preamble boilerplate**. That boilerplate is the single largest, most
obvious cut in this slice.

---

## docs/api/broker.md

This is the only api page with real hand-written reference content (package
layout table + re-export contract). Keep the substance; cut the preamble.

- **CUT** — Lines 9–12, the "Like every API page, it is for contributors
  changing cafleet and embedders driving it from Python; CLI users find the
  command surface in [CLI options]" sentence. Verbatim on all four api pages;
  pure boilerplate. (See cross-cutting policy below for the replacement.)
- **REWRITE** — Intro (lines 7–12). The first sentence ("The data-access layer
  every CLI command and the WebUI share: all agent, fleet, and message
  operations against SQLite live here.") is good and earns its place. The
  second ("Read this page to embed the broker from Python or to change any
  persisted behavior.") is a soft restatement of audience — fold into one
  tightened lead sentence. Target: one sentence of what + one of who.
- **KEEP** — Package layout table (lines 14–25): genuinely useful, maps
  submodule → responsibility, not derivable from the generated docs. Keep.
- **KEEP** — Re-export contract (lines 27–34): states the import contract and
  the test patch seam — real, non-obvious info a reader needs. Keep, but see
  REWRITE below.
- **REWRITE** — Re-export contract prose (lines 29–34) is one dense 4-line
  sentence with two embedded clauses (patch seam + DB seam). Split into two
  short sentences: (1) import the package, use attribute access, never a
  submodule; (2) the patch seam is the package attribute, the single DB seam is
  `get_sync_sessionmaker`.
- **KEEP** — `::: cafleet.broker` (line 36). The generated API surface is the
  point of the page.

## docs/api/coding-agent.md

Thin page: intro + boilerplate + directive (15 lines total).

- **CUT** — Lines 9–12, the shared audience preamble (same as broker).
- **KEEP/REWRITE** — Intro (lines 7–9): "The backend abstraction behind
  `--coding-agent`: the interface each spawned binary (claude, codex, opencode)
  implements — spawn argv, model validation, availability checks. Read this
  page to add or change a backend." First sentence is excellent and specific —
  keep. "Read this page to add or change a backend" is a fine one-line audience
  cue — keep it OR fold into the shared one-liner policy (decide in debate).
- **KEEP** — `::: cafleet.coding_agent.base` (line 14).

## docs/api/config.md

Thin page: intro + boilerplate + directive (13 lines total).

- **CUT** — Lines 9–11, the shared audience preamble.
- **KEEP** — Intro (lines 7–9): "every `CAFLEET_*` environment variable
  resolves to a field on the `Settings` model defined here." Specific and
  useful. "Read this page to see defaults and aliases when embedding or
  contributing" — trim to a short audience cue or fold into shared policy.
- **KEEP** — `::: cafleet.config` (line 13).

## docs/api/multiplexer.md

Thin page: intro + boilerplate + directive (13 lines total).

- **CUT** — Lines 9–11, the shared audience preamble.
- **KEEP** — Intro (lines 7–9): "The tmux abstraction: pane discovery, window
  splitting, keystroke delivery, and capture used by the spawn and
  push-notification paths." Specific, keep. "Read this page to change how
  cafleet drives tmux" — short audience cue, keep or fold.
- **KEEP** — `::: cafleet.multiplexer.base` (line 13).

## docs/index.md

The site landing page. Mostly earns its place, but the two body paragraphs
overlap and can tighten.

- **REWRITE/MERGE** — Paragraphs at lines 8–14 and 16–21 both re-state "message
  broker and agent registry for coding agents," the SQLite-direct-no-server
  point, and the multi-backend point. Two paragraphs say the same three things
  twice. Merge into one tight paragraph: what it is (broker + registry, unified
  CLI + WebUI on single-file SQLite), the fleet-isolation + direct-SQLite fact,
  and the three backends coexisting. The "built for developers running
  multi-agent coding teams in tmux… and for operators who want a single-file
  SQLite broker" audience framing (lines 18–21) is borderline marketing —
  compress to a half-sentence.
- **KEEP** — Tagline (lines 8–9) "Agent Teams reinvented…with full code
  transparency." It is the product positioning line; keep.
- **KEEP** — Get-started button (line 23).
- **KEEP/REWRITE** — "Browse the docs" list (lines 26–31). The structure (one
  bullet per top section) is good and should stay. But each bullet's description
  is a long comma-list that duplicates the section's own landing page. The
  Concepts bullet (line 29) enumerates nine concepts; the Spec bullet and API
  bullet likewise list every sub-page. Trim each to a short phrase — the link
  target carries the detail. Especially line 31 (API Reference) which re-lists
  the four module names already visible one click away.

---

## Cross-cutting observations (seeds the debate)

1. **The audience preamble is the marquee redundancy.** The exact text "Like
   every API page, it is for contributors changing cafleet and embedders
   driving it from Python; CLI users find the command surface in CLI options"
   appears verbatim on all four `api/*` pages (broker 9–12, coding-agent 9–12,
   config 9–11, multiplexer 9–11). **Proposed repo-wide rule: cut it from every
   page entirely.** A reference reader who opened `api/broker.md` does not need
   to be told on every page who the API section is for. If we want the
   audience/cross-ref preserved at all, it belongs **once** on the `api/`
   section landing/index (an `api/index.md` or the mkdocs section overview), not
   repeated per-page. This is the key policy question for debate: cut entirely
   vs. single shared one-liner, and where it lives.

2. **The per-page "Read this page to X" cue is a softer version of the same
   redundancy.** Each api page has a one-line "Read this page to {add a backend
   / see defaults / change how cafleet drives tmux}." These are *per-page
   specific* (unlike the boilerplate), so they are cheap and arguably useful as
   a one-line "what you'd change here" cue. Recommend KEEP these (they are not
   duplicated), but enforce a one-sentence cap. Debate: do we keep them or fold
   into the lead sentence?

3. **Consistent voice/length target for an api page:** one lead sentence (what
   the module is) + at most one audience/what-you'd-change cue + any genuinely
   non-generated reference content (only broker has this) + the `:::` directive.
   No page-level audience boilerplate. coding-agent/config/multiplexer should
   land at ~6–8 lines incl. frontmatter.

4. **index.md double-statement.** The landing page says "message broker and
   agent registry for coding agents" and the no-server/direct-SQLite and
   multi-backend facts twice across two paragraphs. One paragraph suffices.

5. **"Browse the docs" descriptions duplicate downstream landing pages.** Every
   bullet restates the contents of the page it links to. Trim to a short phrase;
   let the link carry the detail. (Relevant to other scanners covering
   get-started/how-to/concepts/spec — we should agree one rule: section-index
   bullets are a phrase, not a contents dump.)

6. **Diagrams:** none in this slice. No retention concern here, but I support
   the team-wide "keep diagrams" rule.

---

## CONSENSUS

Agreed by all three scanners (scanner-api 409, scanner-spec 410,
scanner-concepts 411) across Round 2. Zero open questions.

### Q1 — the audience/boilerplate preamble: DELETE, zero copies
- **Rule:** Delete the 4-line audience preamble ("Like every API page, it is
  for contributors changing cafleet and embedders driving it from Python; CLI
  users find the command surface in CLI options") from **all four** `api/*`
  pages with **zero surviving copies**. No new `api/index.md` is added.
- **Why no surviving copy:** `index.md` "Browse the docs" already routes
  audiences — the Specification bullet sends CLI users to CLI options, the API
  Reference bullet sends Python users to the modules. The only load-bearing bit
  of the preamble (the CLI-options cross-ref) is therefore already carried by
  the index nav. A per-page one-liner, even shared, would be N physical copies
  and violates the one-home rule.
- scanner-spec originally leaned "single shared one-liner owned by the relevant
  page"; conceded it still meant 4 copies. scanner-concepts pushed zero-copies
  / no-new-page; I (scanner-api) dropped my own index.md-bullet idea once the
  index nav was recognized as already carrying the routing. Unanimous.

### Q2 — single-source ownership map (api-relevant entries)
- The proposed cross-slice map (monitoring → concepts/monitoring.md; Esc
  mechanics → concepts/tmux-push.md; spawn flags → concepts/coding-agents.md;
  error strings → spec/cli-options.md Error Messages; fleet-id rationale →
  spec/cli-options.md "Fleet ID"; persisted columns → spec/data-model.md#tasks;
  --activity list → concepts/token-reduction.md; backend non-claude-pane / `!`
  shortcut / pane-title / overview framing → concepts/coding-agents.md) is
  **confirmed**.
- **Added (api slice):** `api/config.md` is the generated canonical home for the
  full `CAFLEET_*` field defaults and aliases (rendered from the `Settings`
  model). Two agreed refinements: (a) `concepts/storage.md` keeps only
  `CAFLEET_DATABASE_URL` (its own subject) and points to config.md for the rest;
  (b) `spec/cli-options.md` keeps the server-section precedence rule (flag beats
  env beats default), which inherently names the `127.0.0.1`/`8000` default
  inline — that one default stays where the precedence is stated — but holds no
  parallel exhaustive env-var catalog.

### Q3 — backend verification recipes: KEEP verbatim
- The codex.md / opencode.md copy-paste smoke tests are **kept verbatim**, not
  reduced to a pointer. A runnable smoke test is genuine reference content;
  half-trimming it into non-runnable fragments is the worst outcome. (Not my
  slice; I support scanner-spec's position. Unanimous.)

### Q4 — repo-wide reference voice/length policy (signed by all three)
- A reference page states the current surface in tables; prose only where a
  table cannot carry the meaning. Each fact, error, and rationale has exactly
  one home; every other mention is a one-line pointer. No migration/history
  narration, no promotional editorializing, no `##` heading whose body is only a
  cross-link. Diagrams are kept.
- **Api-specific corollary (signed):** every reference/concept page opens with
  **ONE lead sentence** stating what the page is (audience implicit). **No
  standalone audience line and no "Read this page to X" line anywhere** — the
  per-page cue folds into the lead sentence. This kills the api-vs-concepts
  inconsistency (concepts pages already carry no separate audience line).
- **Resulting target for the four api pages:** one lead sentence (what the
  module is, with the change-surface folded in) + any genuinely non-generated
  reference content (only `broker.md` has this — its package-layout table and
  re-export contract, both KEEP) + the `::: cafleet.<module>` directive. The
  three thin pages (coding-agent, config, multiplexer) land at ~6–8 lines incl.
  frontmatter.
- **index.md:** merge the two body paragraphs (lines 8–21) into one — they state
  "broker + registry / no-server direct-SQLite / multi-backend coexistence"
  twice; compress the "built for developers… and for operators…" audience
  framing to a half-sentence. Trim each "Browse the docs" bullet (lines 26–31)
  to a short phrase; the link target carries the detail (the API Reference
  bullet must not re-list the four module names). Keep the tagline and the
  Get-started button.

## OPEN QUESTIONS FOR USER

none
