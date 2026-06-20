# Reference Documentation Simplification

**Status**: Approved
**Progress**: 25/39 tasks complete
**Last Updated**: 2026-06-20

## Overview

Simplify CAFleet's reference documentation surface (`docs/api/*`, `docs/spec/*`, `docs/concepts/*`, `docs/reference/coding-agents/*`, plus `docs/index.md`) so each page is user-friendly, simple, and just-enough: no boilerplate, no cross-file redundancy, one canonical home per fact. The same single-home rule extends conservatively to `README.md` and `SKILL.md` echoes. A first-class goal is **coding-agent neutrality** — the docs must read as if `claude`, `codex`, and `opencode` are peers, which includes creating the missing `docs/reference/coding-agents/claude.md`.

## Principles

These four principles govern every per-file action in this document.

1. **Reference voice/length policy.** A reference page states the current surface in tables; prose only where a table cannot carry the meaning. Each fact, error, and rationale has exactly one home; every other mention is a one-line pointer. No migration/history narration, no promotional editorializing, no `##` heading whose body is only a cross-link. Every page opens with one lead sentence stating what it is (audience implicit) — no standalone audience line, no standalone "Read this page to X" line. **Diagrams are kept.**
2. **Single-source ownership.** Every repeated topic has exactly one canonical home (the [ownership map](#single-source-ownership-map)); every other mention is a ≤2-line pointer to it.
3. **Coding-agent neutrality.** No backend is privileged in the prose. `claude`, `codex`, and `opencode` are peers; backend specifics live on per-backend reference pages that each document links to — the base-plus-overlay philosophy the repo already uses for skills (`.claude/rules/coding-agent-overlay.md`), now applied to human-facing docs. The true fact that `--coding-agent` defaults to `claude` stays, framed as a neutral default rather than a privilege.
4. **Conservative SKILL.md/README cleanup.** `docs/` is the canonical home. `README.md` and `SKILL.md` files demote only clearly-duplicated *narrative* facts to one-line pointers; operational contract lines an agent needs to act (literal command syntax, the interval values a monitor must use, the literal-id invocation rule) stay in place. Do not restructure skills.

## Success Criteria

- [ ] The 4-line audience preamble ("Like every API page…") is deleted from all four `docs/api/*` pages with zero surviving copies; no `api/index.md` is added.
- [ ] `docs/reference/coding-agents/claude.md` exists as a structural peer of `codex.md` / `opencode.md` and is registered in the zensical nav under "Coding-agent backends".
- [ ] Every reference/concept page opens with exactly one lead sentence; no page carries a standalone audience or "Read this page to X" line.
- [ ] Each topic in the [ownership map](#single-source-ownership-map) appears in full on exactly one page; every other mention is a ≤2-line pointer carrying no duplicated numbers/rationale.
- [ ] No `docs/spec/*` page narrates a migration revision number or contains raw migration SQL.
- [ ] `docs/concepts/coding-agents.md` represents all three backends symmetrically and links to all three per-backend reference pages.
- [ ] The codex/opencode verification recipes remain verbatim and runnable.
- [ ] All diagrams present before the change are still present after it.
- [ ] `mise //:docs-build` (runs `uv run zensical build --clean`) builds with no broken internal links.

---

## Background

The reference surface grew page-by-page and accumulated three kinds of waste: (1) **per-page boilerplate** — the identical 4-line audience preamble on all four `api/*` pages; (2) **cross-file redundancy** — the monitoring model retold across five concept pages, the `Esc` safeguard explained in full twice, ~15 error strings written twice in `cli-options.md`, the persisted-task columns duplicated between `data-model.md` and `message-envelope.md`; and (3) **scope leak** — spec pages narrating concept "why" and Alembic migration history that belongs in git and design docs.

A separate three-member Scanner team read the entire surface and debated to unanimous consensus (findings in `scan/findings-api.md`, `scan/findings-spec.md`, `scan/findings-concepts.md`). This document turns that consensus, plus the user's four policy answers and the coding-agent-neutrality addendum, into concrete per-file actions.

A structural symptom of the privilege bias: `docs/reference/coding-agents/` has `codex.md` and `opencode.md` but **no** `claude.md`, and the nav's "Coding-agent backends" group lists only two of three backends. `data-model.md` is internally inconsistent — it claims "a single initial migration" (lines 11, 69) while simultaneously narrating revisions `0002` and `0005` (lines 126, 151). The repo actually ships five revisions (`0001`–`0005`), so both halves are wrong: the single-migration claim is false and the revision narration is archaeology. The fix cuts the archaeology AND rewrites the false claim to the accurate current state (schema managed by a chain of Alembic revisions, applied once via `cafleet db init`).

---

## Specification

### Single-source ownership map

Confirmed by all three scanners and the user. Each topic appears in full on exactly one page; every other mention is a ≤2-line pointer with **no** duplicated interval numbers or rationale.

| Topic | Canonical home | Demoted to a pointer on |
|---|---|---|
| Monitoring model (when/what split, 180 s/720 s watched set) | `concepts/monitoring.md` | `overview.md` §Monitoring; `token-reduction.md` intro; `coding-agents.md` `--role monitor` (which today carries backend-inheritance text only — there is no monitoring-model prose there to cut; the `--role monitor` text must hold NO interval numbers / when-what mechanics, which stay in `monitoring.md`); `cli-options.md` monitor group + `monitor start` + `member ping` + `member create --role`; `webui-api.md` monitor field; `data-model.md` watched-set restatements; `README.md` §monitoring paragraph |
| `Esc` keystroke mechanics | `concepts/tmux-push.md` | `concepts/monitoring.md` (keeps only the wake-nudge exception, one line) |
| Per-backend spawn-argv + auto-approval flags + `--role monitor` backend inheritance | `concepts/coding-agents.md` (Backend-resolution table) | `bash-routing.md` para 1; `cli-options.md` spawn-command table; `codex.md`/`opencode.md`/`claude.md` Spawn-flags sections; `monitoring.md` `--role monitor` |
| Error strings | `spec/cli-options.md` Error Messages table | every inline per-subcommand error restatement in `cli-options.md` |
| `--fleet-id` literal-flag rationale | `spec/cli-options.md` "Fleet ID" | `cli-options.md` blockquote + `permissions.allow` section; `skills/cafleet/SKILL.md` (shorten rationale, keep the literal-id contract); `README.md` |
| Persisted task columns | `spec/data-model.md` #tasks | `message-envelope.md` (keeps only the rendered projection) |
| contextId routing rationale (the WHY) | `concepts/storage.md` | `data-model.md` (states the `context_id` column existence only) |
| `CAFLEET_*` field → default → alias catalog | `api/config.md` (generated from `Settings`) | `storage.md` (keeps `CAFLEET_DATABASE_URL` only); `cli-options.md` (keeps the `server` precedence rule with its inline `127.0.0.1`/`8000` default) |
| `--activity` column list | `concepts/token-reduction.md` table | `member-lifecycle.md` §Commands |
| Backend "usage from a non-claude pane / `!` shortcut / pane-title / overview framing" | `concepts/coding-agents.md` | `codex.md`/`opencode.md`/`claude.md` (keep genuine deltas only) |

### Coding-agent neutrality

The docs apply the same base-plus-overlay split the repo uses for skills: backend-neutral base prose, with each backend's specifics on its own reference page that the base links to. Concretely:

- A new `docs/reference/coding-agents/claude.md` is created as a peer of `codex.md`/`opencode.md`, carrying claude's genuine operator-facing deltas only (see [Step 3](#step-3-coding-agent-reference-pages--neutrality)).
- `concepts/coding-agents.md` represents all three backends symmetrically (its Backend-resolution table already does — extend the "Operational details" pointer paragraph to link claude.md too).
- `concepts/*` and `spec/*` prose that states a claude-specific command/flag/framing inline as if it were the universal default is neutralized: keep the true `--coding-agent` default of `claude` framed as a neutral default, and point backend specifics to the relevant per-backend page.
- The zensical nav lists all three reference pages under "Coding-agent backends".

### Per-file actions

The authoritative, per-file CUT/MERGE/REWRITE/KEEP list is encoded as the [Implementation](#implementation) steps below. Action verbs:

- **CUT** — delete the content outright (it is boilerplate, duplicate, or history).
- **MERGE** — fold the one durable fact into an adjacent section, then delete the standalone block/section.
- **REWRITE** — keep the content's job but compress to the policy voice (table where possible, one rationale clause max).
- **KEEP** — genuine, single-homed reference content; do not touch.
- **POINTER** — replace with a ≤2-line link to the canonical home.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> This design doc edits documentation only; no source code changes. The documentation-first ordering required by `.claude/rules/design-doc-numbering.md` is satisfied inherently — every step is a doc edit.

### Step 1: API slice — `docs/api/*` + `docs/index.md`

- [x] `api/broker.md` — CUT the audience-preamble sentence (lines 9–12, "Like every API page…"). REWRITE the intro to one lead sentence: keep "The data-access layer every CLI command and the WebUI share…"; fold the change-surface cue in, drop "Read this page to embed…". KEEP the Package layout table and the `::: cafleet.broker` directive. REWRITE the re-export contract into two short sentences (import the package + use attribute access, never a submodule; the patch seam is the package attribute, the single DB seam is `get_sync_sessionmaker`). <!-- completed: 2026-06-20T11:21 -->
- [x] `api/coding-agent.md` — CUT the audience preamble (lines 9–12). Collapse to one lead sentence (keep "The backend abstraction behind `--coding-agent`…"; fold "add or change a backend" cue in). KEEP the `:::` directive. Target ~6–8 lines incl. frontmatter. <!-- completed: 2026-06-20T11:21 -->
- [x] `api/config.md` — CUT the audience preamble (lines 9–11). One lead sentence (keep "every `CAFLEET_*` environment variable resolves to a field on the `Settings` model defined here"). KEEP the `:::` directive. This page is the canonical home for the full `CAFLEET_*` defaults/aliases catalog. <!-- completed: 2026-06-20T11:21 -->
- [x] `api/multiplexer.md` — CUT the audience preamble (lines 9–11). One lead sentence (keep "The tmux abstraction: pane discovery, window splitting, keystroke delivery, and capture…"). KEEP the `:::` directive. <!-- completed: 2026-06-20T11:21 -->
- [x] `index.md` — MERGE the two body paragraphs (lines 10–21) into one: what it is (broker + registry, unified CLI + WebUI on single-file SQLite), fleet isolation + direct-SQLite, three backends coexisting; compress the "built for developers… and operators…" framing to a half-sentence. KEEP the tagline (7–8) and the Get-started button. REWRITE each "Browse the docs" bullet (27–31) to a short phrase — the API Reference bullet (31) must not re-list the four module names. <!-- completed: 2026-06-20T11:21 -->

### Step 2: Spec slice — `docs/spec/*`

**`cli-options.md`** (condense in place; do NOT split):

- [x] CUT the duplicated `--fleet-id`-literal rationale: keep the canonical statement in the "Fleet ID" section; reduce the blockquote (L55–57) and the `permissions.allow` coverage restatements (L98/L128) to one-line pointers to "Fleet ID". <!-- completed: 2026-06-20T11:36 -->
- [x] CUT the ~15 inline per-subcommand error restatements (`member exec`, `member send-input`, `member ping`, `member nudge`, `member create --prompt-file`, root-Director/Administrator deregister guards); the Error Messages table (L990–1031) is the single home, each subcommand keeps a one-line "see Error Messages" pointer. <!-- completed: 2026-06-20T11:36 -->
- [x] REWRITE the `permissions.allow` coverage section (L98–140): replace the ~40 literal pattern lines (20 base + 8 `--json` companions) with the generation rule (one pattern per allow-listed subcommand, canonical `--fleet-id`-first order, `member exec` excluded, `--json` needs a companion because it precedes the subcommand) + 3 representative example lines + a note that the full set is mechanical. <!-- completed: 2026-06-20T11:36 -->
- [x] CUT concept narration to a `monitoring.md` pointer in: the `monitor` group intro (L921–923), `monitor start` (L931–937), `member ping` description (L819), `member nudge` Behavior/description (L873–875, L888–890). Keep only the CLI surface (action, flags, exit codes, the no-monitoring-member warning). <!-- completed: 2026-06-20T11:36 -->
- [x] REWRITE the `member create` `--coding-agent` (L545) and `--role` (L547) Notes cells to the flag's own behavior + a pointer to `coding-agents.md`/`monitoring.md`. Demote the "Spawn command per backend" table (L553–557) to a pointer to `coding-agents.md`. <!-- completed: 2026-06-20T11:36 -->
- [x] REWRITE the six `message {send,ack,cancel,show,poll,broadcast}` bodies (L469–532): collapse the shared output contract ("Text output is `<verb>` + compact envelope; `--quiet` prints only the task id") into one sentence above the six tables; keep each subcommand's unique flags. <!-- completed: 2026-06-20T11:36 -->
- [x] Env-var defaults point to `api/config.md`, EXCEPT the `server` section, which keeps `127.0.0.1`/`8000` inline where it states the flag-beats-env-beats-default precedence. KEEP: Subcommand summary table (L13–42), `--full` semantics table (L77–86), Error Messages catalogue, per-flag Required/Notes tables. <!-- completed: 2026-06-20T11:36 -->

**`data-model.md`**:

- [x] CUT the duplicate opening: "The model is predominantly relational…" (L9) duplicates L7 — keep L7, fold any index/blob detail in. <!-- completed: 2026-06-20T11:36 -->
- [x] CUT ENTIRELY the migration archaeology: the "Enrollment-inversion data migration" section (L151–162) including the raw `DELETE FROM monitor_config …` SQL, and the revision `0002`/`0005` narration in the `interval_seconds` Note (L126). State only current behavior: every enrollment writes an explicit interval; the schema default is inert. <!-- completed: 2026-06-20T11:36 -->
- [x] FIX the false single-migration claim while removing the archaeology: the repo ships five Alembic revisions (`0001`–`0005`, verified in `cafleet/src/cafleet/db/alembic/versions/`), so L11 ("the schema is created by a single initial migration") and L69 ("the single initial migration is schema-only") are wrong. Rewrite both to the accurate current state — the schema is managed by a chain of Alembic revisions, applied once via `cafleet db init` — without naming individual revision numbers. <!-- completed: 2026-06-20T11:36 -->
- [x] REWRITE the watched-set restatements (L121, L126, L130, kind-marker section L134–149): state the watched set once at the `monitor_config` intro as a pointer to `monitoring.md` with NO interval numbers; Notes cells reference it. <!-- completed: 2026-06-20T11:36 -->
- [x] MERGE the "Deregistered Agents" section (L215–217) into the `agents` table notes (fold the one new fact: WebUI still surfaces them); drop the standalone section. KEEP all schema tables, the AUTOINCREMENT/id-never-reused explanation, Task Visibility Rules, Broadcast Grouping. Reduce the `context_id` mention to the column's existence (rationale lives in `storage.md`). <!-- completed: 2026-06-20T11:36 -->

**`message-envelope.md`**:

- [x] CUT the L9 redundant restatement ("This document covers the canonical envelope shape…") — it repeats the L5–7 intro. <!-- completed: 2026-06-20T11:36 -->
- [x] MERGE/POINTER: replace the "Persisted shape (typed columns)" table (L15–26) with a one-line pointer to `data-model.md#tasks`; keep only the rendered projection here. KEEP the Compact rendered envelope table (L40–51), Text-mode rules (L96–106), and the default/`--full` JSON examples (L64–93). <!-- completed: 2026-06-20T11:36 -->

**`webui-api.md`**:

- [x] CUT the `409 (reserved for future deregister endpoint)` paragraph (L314) — speculative future-state narration. REWRITE the `monitor`-field watched-set explanation (L89–97) to field shape + a pointer to `monitoring.md` (no interval numbers). KEEP every endpoint spec and the shared-row-formatter note (L212). <!-- completed: 2026-06-20T11:36 -->

### Step 3: Coding-agent reference pages + neutrality

- [x] CREATE `docs/reference/coding-agents/claude.md` as a structural peer of `codex.md`/`opencode.md` (match their section order exactly: frontmatter `icon: lucide/bot`, lead sentence, Overview, Spawn flags, Required CLI version note if applicable, `cafleet` usage pointer, `!` shortcut pointer, Verification recipe). Carry claude's genuine deltas only: claude is the **default** backend when `--coding-agent` is omitted (framed as a neutral default); claude is the only backend that sets the pane title (positive capability, via `--name`); the spawn permission posture is `--permission-mode dontAsk`; and common claude model-name examples (`fable`, `opus`, `sonnet`, `haiku`, `best`, `default`, `opusplan`, `sonnet[1m]`, `opus[1m]`) — framed as examples, NOT an enforced whitelist: like every backend, cafleet passes any `--model` string through verbatim and the claude binary itself rejects unknown models (the same contract codex.md states). Close with a runnable verification recipe mirroring the codex/opencode smoke tests. For the shared "usage from a pane / `!` shortcut / pane-title concept / skills-loading contrast / overview framing", link to `concepts/coding-agents.md` rather than restating (that page is the canonical home for all of it, including the claude-loads-skills-directly vs codex/opencode-read-by-absolute-path contrast). <!-- completed: 2026-06-20T11:46 -->
- [x] `codex.md` — MERGE the shared boilerplate to pointers (per the ownership map): the "cafleet usage from inside a codex pane" block (L54–67), the `!` shortcut section (L69–71), the pane-title asymmetry (L73–75), and the Overview's default-claude/single-Director framing (L11–20) reduce to one-line pointers to `coding-agents.md`. KEEP verbatim: the `writable_roots`/SQLite-DB IMPORTANT callout (L34–43), required-version + binary-not-on-PATH (L46–52), and the Verification recipe (L77–107, runnable — do NOT trim). <!-- completed: 2026-06-20T11:46 -->
- [x] `opencode.md` — MERGE the same four shared sections (usage L68–81, `!` shortcut L83–85, pane-title L87–89, Overview framing L11–22) to pointers. KEEP verbatim: the `cafleet` agent-preset mechanics + refresh procedure, the `--model <provider>/<model>` format rule, permission-popup recovery posture, safety-floor caveats, "writes one file under `$HOME`", and the Verification recipe (L108–148, runnable — do NOT trim). <!-- completed: 2026-06-20T11:46 -->
- [x] `concepts/coding-agents.md` — ADD the canonical "cafleet usage from a member pane" statement that the demoted codex/opencode/claude pointers resolve to (a new short section, or a clearly-headed paragraph). It owns, in one place: the cafleet CLI works unchanged from any backend pane using the literal ids from the spawn prompt (no env-var fallback); claude panes load Claude Code skills directly while codex/opencode panes read the cafleet skill files by absolute path; and all three honor the leading-`!` shell shortcut. This is the home the ownership-map "usage from a non-claude pane / `!` shortcut / pane-title / overview framing" row points at, and the single home for the skills-loading contrast — claude.md and the backend pages link here, none restate it; without it the Step-3 pointers would orphan the "non-claude panes read skills by absolute path" fact. Then: extend the "Operational details" pointer paragraph (L75–80) to link `claude.md` alongside codex.md/opencode.md, so all three backends are referenced symmetrically. REWRITE the post-table paragraph (L20–39): the leading-`!` shortcut (re-explained on bash-routing.md/tmux-push.md) and the thrice-stated `--coding-agent` recording collapse to single statements; the `--role monitor` backend-inheritance paragraph (L35–39) is the canonical home (monitoring.md points here). REWRITE Model selection (L42–60) to a compact paragraph; defer exhaustive per-flag detail to `cli-options.md`. KEEP the Backend-resolution table and Known-asymmetries list. <!-- completed: 2026-06-20T11:46 -->
- [x] Register the new page in `zensical.toml` nav: add `{ "Claude members" = "reference/coding-agents/claude.md" }` to the "Coding-agent backends" group (currently lists only Codex and Opencode at L40–41), ordered with claude first as the default backend. <!-- completed: 2026-06-20T11:46 -->

### Step 4: Concepts slice — remaining `docs/concepts/*`

- [ ] `overview.md` — CUT the broker-restatement (L50–51) and the "Design document orchestration skills" promotional section (L87–95; at most a one-line Core-terms/get-started pointer). REWRITE §Monitoring (L74–85) to 2–3 sentences + link (drop interval numbers and when/what mechanics — they live in monitoring.md). REWRITE §WebUI (L60–69) to ~3 lines; move the per-agent route detail to webui-api.md. KEEP front-matter, intro, Core terms table, architecture diagram. <!-- completed: -->
- [ ] `fleet-isolation.md` — MERGE the four pointer-stub sections (Fleet bootstrap, Fleet soft-delete, Root Director protection, Built-in Administrator) into one "Lifecycle" 4-row table (behavior + CLI-options link each). CUT the duplicated "fleet_id is non-secret" statement (keep the "partitions for tidiness, not security" framing, folded into the intro). CUT the Registration section's repeated "created via `cafleet fleet create`" (fold the one new clause — non-soft-deleted fleet_id required — into Isolation rules). KEEP Isolation rules ("not found indistinguishable from non-existence"). <!-- completed: -->
- [ ] `storage.md` — REWRITE §Concurrency (L20–24) to one sentence (`busy_timeout=5000` retry + low contention). REWRITE the contextId convention (L48–55) to the rule + one rationale clause (this page owns the contextId WHY). REWRITE §Relational model (L26–30) to two lines (the durable fact: only JSON blob is `agents.agent_card_json`; point to data-model.md). KEEP Backend, Schema management, No physical cleanup; keep `CAFLEET_DATABASE_URL` concretely but point to `api/config.md` for the full var set. <!-- completed: -->
- [ ] `member-lifecycle.md` — MERGE §Terminology (L11–15) into the intro (fold "the Director is NOT a member; it uses plain `agent register`"). REWRITE §Commands (L74–86) to the subgroup list + a pointer to cli-options.md for flags/member-resolution; drop the `--activity` column list (owned by token-reduction.md). CUT §Pane display name (L70–72) — coding-agents.md owns the asymmetry. KEEP intro, Single-Director invariant, lifecycle state diagram, Atomic create flow, Delete ordering. <!-- completed: -->
- [ ] `bash-routing.md` — REWRITE the first paragraph (L7–15): replace the per-backend auto-approval flag enumeration with "Members spawn with workspace-scoped auto-approval (see Coding agents for the per-backend flags)." KEEP the two-paragraph routing protocol and the closing pointer to `skills/cafleet/reference/exec-routing.md`. <!-- completed: -->
- [ ] `tmux-push.md` — KEEP the full `Esc`-safeguard explanation here (this page owns keystroke mechanics), plus intro, sequence diagram, literal preview block, Design principles, response-annotations note. REWRITE L38–48 (placement-row resolution) to tighten; durable new fact: root Director's placement carries `director_agent_id=NULL`. MERGE §Manual entry-point (L95–99) with the fallback-chain paragraph (L67–73) into one `member ping` description. <!-- completed: -->
- [ ] `monitoring.md` — This is the canonical monitoring home; it KEEPS the two diagrams, The watched set core facts, Single-instance and liveness, Cadence/tick-precision table, Enrollment and schema. CUT the `Esc`-safeguard paragraphs (L20–33) to one sentence stating only the wake-nudge exception + pointer to tmux-push.md. CUT the repeated when/what "alarm clock not the worker" prose (keep the Heartbeat vs facilitation table; cut the surrounding restatements at L66–67, L86–90). REWRITE The monitoring member (L107–158): make the backend-inheritance sentence (L110–115) a pointer to coding-agents.md; keep the numbered routine, cut the trailing restatements (L147–158). <!-- completed: -->
- [ ] `token-reduction.md` — REWRITE the intro (L7–22, a 16-line monitoring re-narration) to ~4 lines: cafleet spends no tokens itself but every byte lands in an agent's context; moving supervision scheduling into the monitor process (see Monitoring) removes the per-tick scheduling prompt; the table catalogs the cost reductions. Drop interval/wake-nudge mechanics. KEEP the techniques table (this page owns the `--activity` column list). CUT the Inline message preview row's re-explanation (L31) to a one-line technique + link to tmux-push.md. <!-- completed: -->

### Step 5: SKILL.md / README echo demotion (conservative)

- [ ] `README.md` — REWRITE the §monitoring mega-paragraph (L85): compress the full 180 s/720 s + wake-nudge + Esc-safeguard retelling to a 2–3 line summary that ends with the existing `concepts/monitoring/` link. This is overview prose with no agent-behavioral contract, so it is safe to compress. Demote any `--fleet-id` literal-flag rationale prose to a pointer; keep statements of what the product is. <!-- completed: -->
- [ ] `skills/cafleet/SKILL.md` — SHORTEN the §Required Flags rationale (L32) so the *narrative* "why" points to `cli-options.md#fleet-id`, while KEEPING the load-bearing operational contract line ("Use literal ids, never shell variables"). Do not touch behavioral contract lines elsewhere. <!-- completed: -->
- [ ] Audit the remaining `SKILL.md` echoes of ownership-map topics (the supervision/monitoring skills, `reference/director.md`) and demote ONLY pure narrative duplications of the 180 s/720 s model to one-line pointers. Explicitly leave the `cafleet-agent-team-monitoring` skill as the agent-facing operational home for the monitoring loop (it is not a docs duplicate — agents act from it), and preserve every operational interval value an agent must use. No skill restructuring. <!-- completed: -->

### Step 6: Verification

- [ ] Grep the full `docs/` tree to confirm zero surviving copies of the audience preamble ("Like every API page") and that no `docs/spec/*` page contains a migration revision number or raw migration SQL. <!-- completed: -->
- [ ] Confirm each ownership-map topic appears in full on exactly its canonical page and as a ≤2-line pointer elsewhere (no duplicated interval numbers outside `monitoring.md`). <!-- completed: -->
- [ ] Build the docs with `mise //:docs-build` (runs `uv run zensical build --clean`) and confirm no broken internal links, that `claude.md` renders under "Coding-agent backends", and that all pre-existing diagrams still render. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-20 | Initial draft |
