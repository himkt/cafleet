---
name: cafleet-design-doc-execute
description: "Implement features based on a design document using CAFleet-native orchestration with TDD cycle. Use when the user asks to implement or execute a design document. Takes document path as argument. Do NOT implement a design document by reading it and coding manually — always invoke this skill instead."
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---

# Design Doc Execute (CAFleet Edition)

Implement features based on a design document using up to four roles orchestrated via the CAFleet message broker: Director (orchestrator), Programmer (implements), Tester (writes tests), and Verifier (E2E/integration testing). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The Director judges which members to spawn based on the nature of the implementation tasks. For each step, the Tester writes unit tests first, the Director reviews and approves them, then the Programmer implements code to pass the tests. The Director also reviews the Programmer's implementation for code quality and design doc compliance before committing. After all TDD steps, the Verifier performs E2E/integration verification (Phase D) if spawned. After user approval, the Director runs the full publication flow: Step 6 pushes the feature branch and opens a PR with `@copilot` requested, Step 7 runs a cron-driven Copilot review loop that routes inline comments to the still-live Programmer / Tester and exits when Copilot approves or has been quiescent for 5 ticks, and Step 8 finalizes, commits the completion marker, pushes it (when the branch is tracked on origin), and tears the team down.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet, spawn members via `cafleet member create`, validate doc, assign steps, review tests against design doc, review implementation code for quality and compliance, commit after each phase, escalation arbitration, orchestrate TDD cycle | Write code, write tests | [roles/director.md](roles/director.md) |
| **Programmer** | Member agent | Implement code to pass tests, run tests, report results via `cafleet message send`, escalate test defects to Director, update design doc checkboxes and Progress counter | Write or modify tests, commit code, communicate with user directly | [roles/programmer.md](roles/programmer.md) |
| **Tester** | Member agent | Read design doc, write unit tests per step, fix tests based on Director feedback, report to Director via `cafleet message send` | Write implementation code, commit code, communicate with user directly | [roles/tester.md](roles/tester.md) |
| **Verifier** | Member agent (optional) | E2E/integration testing, tool discovery, evidence collection (screenshots, logs, output), failure reporting with suggested fixes | Write code, write tests, commit, communicate with user directly | [roles/verifier.md](roles/verifier.md) |

## Additional resources

- For the document template, see: [../cafleet-design-doc/template.md](../cafleet-design-doc/template.md)
- For section guidelines and quality standards, see: [../cafleet-design-doc/guidelines.md](../cafleet-design-doc/guidelines.md)

## Coordination Protocol

Mechanics for inter-agent coordination in this skill. The design document is the substantive communication medium; `cafleet message send --text` carries only a single-line **verb + pointer** poke. Substantive content (feedback, reports, escalation reasons, review items) lives in inline `COMMENT(role)` markers in the design doc — except for source-anchored Copilot inline review, which is annotated in the source file at `<file>:<line>` because that is where the comment lives.

### Core Principle

`cafleet message send` is a poke, not a payload. Status hops travel as compact `<verb> (<pointer>)` lines on the broker; reasoning, findings, and item-by-item routing live as `COMMENT(role)` markers inside the design document (or, for source-anchored Copilot, the source file). Anchorless meta-events (member crashed/restarted, generic ping ack, "still working") do NOT use a verb from the canonical list and do NOT touch the design doc.

### Verb Vocabulary

The canonical set is exactly 6. Members and the Director MUST pick from this list and MUST NOT invent new verbs.

| Verb | Sender direction | Meaning | Used for |
|:--|:--|:--|:--|
| `ready` | Director → member, or member → Director | "The pointer target is ready for you to read / act on." | Fresh assignments, member-to-Director "I have something for you to look at," and Director stall-nudges (the recipient interprets contextually — see below). |
| `complete` | Member → Director | "I finished a fresh deliverable at the pointer target." | Initial drafts, freshly written tests, freshly written implementation, FIXME-resolution sweeps. |
| `addressed` | Member → Director, or Director → Director (self-note) | "I resolved a pre-existing marker (a `COMMENT(role)` marker, a Copilot inline comment, or a Director-arbitration note) at the pointer." | Round-2+ work on items already flagged in the doc or in source. |
| `blocked` | Member → Director | "I cannot proceed at the pointer; the blocker rationale is in a `COMMENT(role)` marker at the same pointer." | Spec ambiguity, missing deps, environmental issues. |
| `escalating` | Member → Director | "I am escalating an issue (e.g. a suspected test defect) at the pointer; the rationale is in a `COMMENT(role)` marker at the same pointer." | Test-defect arbitration, multi-round disagreements. |
| `approved` | Reviewer → Director, or Director → user-result | "All quality criteria are met at the pointer (typically `doc`)." | Reviewer approval signal. |

**Verb choice for `complete` vs `addressed`**: `complete` signals a fresh deliverable (work that did not previously have a marker waiting). `addressed` signals resolution of a pre-existing marker (any `COMMENT(role)`, any Copilot line, any Director arbitration). When in doubt, ask: "did a marker exist before I started this turn?" — if yes, use `addressed`; if no, use `complete`.

**Director stall-nudges** reuse `ready (doc)` or `ready (paragraph-...)` — they are `ready` from the Director's perspective ("the pointer target is ready for you, please act"). The recipient interprets contextually: on receiving `ready (...)`, scan the pointer target for any `COMMENT(role)` markers addressed to your role or relevant to your current phase, and act accordingly. Re-sent `ready (...)` after a member's idle window is a nudge, not a new assignment — same target, same expected action.

### Pointer Forms

Exactly 3 canonical forms. Use the tightest one that locates the target.

| Form | Example | When to use |
|:--|:--|:--|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. Use the literal heading text. Nest with the three-character separator ` > ` (space, greater-than, space). Heading text is preserved verbatim — slashes, colons, hyphens, and other punctuation inside a heading remain literal. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `cafleet/src/cafleet/cli/main.py:142` | The target is a specific line (or range) in a source file, test file, or the design doc itself. Used for source-anchored Copilot inline review and for source-file `COMMENT` markers added during code review. |
| `doc` | `doc` | The target is the design document as a whole (e.g., Verifier signalling overall E2E success). |

**Pointer-marker pairing rule.** When a verb's spec requires a paired `COMMENT(role)` marker (`blocked` / `escalating`, also Director arbitration replies and `COMMENT(copilot)` placements), the marker MUST live at the SAME pointer as the cafleet body:

| Pointer | Canonical marker placement |
|:--|:--|
| `paragraph-<HeadingPath>` | Inline within that heading's section. |
| `<file>:<line>` | At that exact line in the file (immediately above or on `<line>` per the file's native comment syntax). |
| `doc` | Doc-top — directly under the metadata block (`Status:` / `Progress:` / `Last Updated:`), before the first heading. |

The ` > ` separator avoids the collision that would arise if `/` were used as a nesting separator (heading text in real-world design docs frequently contains `/`, e.g. `Step 2: Update docs/spec/cli-options.md`). ` > ` is unambiguous, ASCII-safe, and shell-safe inside double-quoted `--text` arguments.

### Message Format

Every `cafleet message send --text` body, when used to coordinate within a `cafleet-design-doc-execute` skill team, MUST match:

```
<verb> (<pointer>)
```

Optional one-line summary may follow, separated by ` — ` (space, em-dash, space):

```
<verb> (<pointer>) — <one-line summary>
```

Constraints:

| Constraint | Rule |
|:--|:--|
| Single line | The body MUST be a single line. No literal newlines. |
| Summary cap | The optional summary SHOULD fit on one terminal line; aim for ≤ 80 codepoints. |
| Enumeration cap | The summary MUST NOT enumerate more than 3 items. Longer enumerations belong in a `COMMENT(role)` marker at the pointer. |
| No payloads | The summary is for human readability in the admin WebUI timeline; substantive content (reasoning, file lists beyond 3, multi-paragraph reports) MUST go in a `COMMENT(role)` marker. |

Examples:

- `ready (paragraph-Implementation > Step 1)`
- `complete (paragraph-Implementation > Step 1) — 12 tests pass`
- `addressed (cafleet/src/cafleet/cli/main.py:142)`
- `blocked (paragraph-Specification > Retry Strategy)`
- `escalating (paragraph-Implementation > Step 3)`

### COMMENT(role) Marker

Inline marker placed in the design document (or, for source-anchored Copilot inline review, in the source file at `file:line`).

```
COMMENT(<role>): <substantive content>
```

Roles:

| Role | Who writes it | When |
|:--|:--|:--|
| `claude` | The Director acting as user-mediator | Carries user-derived clarifications (e.g. test-framework arbitration). |
| `director` | The Director | Spec resolution notes, Director judgments, ambiguity arbitration, design-doc-anchored Copilot review (see *Copilot Routing* below), Phase C code-review feedback. |
| `programmer` | The Programmer | Implementation-side notes, escalation rationales, observations of spec gaps that block implementation. |
| `tester` | The Tester | Test-spec gaps (Phase 2 and Phase 1 framework-selection), escalation rationales, evaluation of Programmer-routed test-defect reports. |
| `verifier` | The Verifier | E2E findings, evidence pointers (`see file:line`), suggested-fix categorisation (impl bug / test gap / spec issue). |
| `copilot` | The Director on Copilot's behalf (Copilot does not edit files) | One marker per source-anchored inline review item, written into the **source file** at `<file>:<line>`. Design-doc-anchored Copilot lines route through `COMMENT(director)` instead — see *Copilot Routing*. |

Rules:

- One marker per logical issue. Do not bundle.
- Body must be actionable — state the issue and what should change.
- Markers are split into two classes — *issue* and *status*. Only the *issue* class enters the doc.

### Issue Markers vs Status Markers (split)

`COMMENT(role)` markers carry **issue and feedback content only**. Status updates ("ready", "complete") stay as pure cafleet messages with verb + pointer; they do NOT add a marker to the design doc.

| Class | Example | Lifecycle |
|:--|:--|:--|
| Issue | `COMMENT(programmer): test <test-name> expects X but design doc says Y; please arbitrate` | Persists in the doc until resolved. The resolver removes the marker as part of the fix. Per `skills/cafleet-design-doc/guidelines.md` § *Completeness Check*, the doc cannot reach `Status: Complete` while any `COMMENT(` marker remains. |
| Status | (none — never enters the doc) | Lives only in `cafleet message send` text. |

This keeps the design doc clean: at any moment, the markers in the doc reflect *outstanding work*, never historical chatter.

### Copilot Routing

Copilot reviews split into two line-anchored classes (source file, design doc) plus a PR-level catch-all:

| Anchor | Where the marker lives | cafleet message | Resolver |
|:--|:--|:--|:--|
| Source file (e.g. `cafleet/.../foo.py:42`) | `COMMENT(copilot): <body>` in the source file at `<file>:<line>` | `ready (<file>:<line>)` to Programmer or Tester per the existing path-pattern routing | Routed member fixes source, removes marker, replies `addressed (<file>:<line>)` |
| Design doc (e.g. `design-docs/foo/design-doc.md:42`) | `COMMENT(director): <body>` inline at the affected paragraph in the design doc | (no cafleet route — Director resolves directly per the existing rule) | Director applies the spec change and removes the marker. **No self-note cafleet message is sent** — the git commit and marker removal are sufficient audit trail. |
| PR-level (non-line-anchored) | Director judgment per existing classification (spec → `COMMENT(director)` in design doc; impl → `COMMENT(copilot)` at a representative file:line; test → `COMMENT(copilot)` at a representative test file:line) | `ready (...)` per anchor | per anchor |

The Director's commit message follows the existing convention (`fix: address Copilot review - <short summary>`); the message text contains the summary, not the `COMMENT(copilot)` body (which would be gone from source by then).

### Anchorless Status

A member may need to communicate something that does not point at any heading, file, or doc — e.g. "I crashed and restarted," "still working, no progress yet," a generic ping ack.

- These ride as a pure cafleet message with a freeform short phrase. The set is **NOT a fixed canonical list** — members may use whatever short phrase fits ("restarted", "still working", "ack", "noted", etc.).
- They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses) and the Director MUST treat them as informational, not as work signals.
- They do NOT add markers to the design doc.

If a member finds themselves needing to send anchorless status updates frequently, that is a stall signal — the Director's stall-response ladder (per `skills/cafleet/SKILL.md` and the existing role files) still applies.

### Finalize-Time Cleanup

When the design doc moves to `Status: Complete` (Step 8):

1. Issue markers (`COMMENT(role)` for `director`, `programmer`, `tester`, `verifier`, `claude`) MUST already be resolved per `skills/cafleet-design-doc/guidelines.md` § *Completeness Check* — the existing rule "No `COMMENT(` markers remain" stays.
2. Status markers do not exist in the design doc by construction (split), so there is nothing to strip.
3. `COMMENT(copilot)` markers in source files are removed by the routed member as part of each fix commit; finalize-time validation only needs to confirm the design doc is marker-free.

The audit trail of every status hop lives in the cafleet message log (admin WebUI timeline) and in git history. The design document itself reads as the current state, not an archaeological dig.

### Director Per-File Detail Recovery

Members do not ship file lists in cafleet bodies (the verb + pointer schema carries only a poke). The Director recovers per-file detail directly via git when a commit message needs it: `git status` for unstaged/staged file lists, `git diff --stat <base>..HEAD` for cumulative scope, `git log <base>..HEAD --name-only` for file-touch history, `git diff <base>..HEAD -- <pattern>` for content. This applies in Phase A (test commits), Phase B/C (impl commits), Phase 7d (Copilot fix commits), and Step 8 (finalize commit).

### Skill-specific overrides

- **Verifier Phase 1 exemption**: The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination, and rides as a free-form multi-line cafleet body (same precedent as the Analyzer's question list in the `cafleet-design-doc-interview` skill). Phase 2 verification reports follow the schema.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` (no separate `cafleet agent register` call) — and spawns each needed member via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet fleet create, cafleet member create, orchestrates TDD cycle)
      +-- Programmer (member agent -- implements code to pass tests)
      +-- Tester (member agent -- writes unit tests per step)
      +-- Verifier (member agent, optional -- E2E/integration testing)
```

- **Director ↔ Programmer**: `cafleet message send` (step assignments, test results, code review feedback, escalation)
- **Director ↔ Tester**: `cafleet message send` (step assignments, test review feedback, test defect reports)
- **Director ↔ Verifier**: `cafleet message send` (verification assignments, results, failure routing)
- **Director**: git operations (commit after each phase — tests and implementation separately)
- Members receive messages via push notification: the broker keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into the member's pane via `tmux.send_inline_preview`. The recipient processes the preview as a fresh user-turn input — no `cafleet message poll` invocation is in the auto-fire path; to fetch the full body, the recipient calls `cafleet message poll` themselves. `--fleet-id` is a global flag (placed **before** the subcommand); `--agent-id` is a per-subcommand option (placed **after** the subcommand name).

## Prerequisites

- The Director MUST be running inside a tmux session (required by `cafleet member create`). Verify by running `cafleet doctor` before spawning anyone — it reports the tmux session/window/pane identifiers and exits non-zero with a clear message when the environment is not ready. If `cafleet doctor` reports a problem, abort and surface its message to the user. Do NOT invoke `tmux display-message`, `printenv TMUX`, or any other raw tmux/env probe — `cafleet doctor` is the only supported environment check (see `skills/cafleet/SKILL.md` § *use cafleet primitives only*).
- `gh` must be authenticated for Steps 6 + 7. Lack of auth is NOT fatal — the Director checks `gh auth status` at Step 6a and falls back to Step 8 local-finalize, skipping the PR and Copilot review loop entirely. All other prerequisites (tmux, approved design doc, feature branch) remain unchanged.

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="execute-{slug}")` | CAFleet fleet created via `cafleet fleet create` — it bootstraps the fleet + root Director + placement + Administrator in one transaction (no separate `cafleet agent register` call needed for the Director) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> --name "..." --description "..." -- "<prompt>"` |
| `SendMessage(to="Programmer")` | `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <programmer-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from member) | `cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `cafleet-agent-team-supervision` `/loop` | Load the `cafleet-agent-team-monitoring` skill (mechanism + `/loop`) and the `cafleet-agent-team-supervision` skill (governance), then run `/loop` from the `cafleet-agent-team-monitoring` skill |
| `TeamDelete` | `cafleet --fleet-id <fleet-id> member delete --agent-id <director-agent-id> --member-id <member-agent-id>` for each member, then `cafleet fleet delete <fleet-id>` (soft-deletes the fleet and sweeps the root Director + Administrator + any surviving members in one transaction). The root Director cannot be deregistered via `cafleet agent deregister` — `fleet delete` is the only supported teardown. |
| Auto message delivery | Push notification keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into member's tmux pane via `tmux.send_inline_preview` |

## Process

### Step 1: Resolve Design Document Path (Director)

Before validation, resolve `$ARGUMENTS` into a concrete `design-doc.md` path.

#### Phase 1: Base Directory Resolution

Load the `cafleet-base-dir` skill for the no-bypass write protocol and `<unset>` sentinel contract. Then resolve BASE based on whether `$ARGUMENTS` was supplied:

- **`$ARGUMENTS` present** (the typical execute-a-specific-doc flow): canonicalize `$ARGUMENTS` and call the task-scope resolver positionally. `$ARGUMENTS` is normally a slug name (`0000060-skill-task-scoped-base-dir`) or a path containing such a slug.

  - **Relative input** — accept any of: `0000060-foo`, `0000060-foo/design-doc.md`, `design-docs/0000060-foo`, `design-docs/0000060-foo/design-doc.md`. Canonicalize to `design-docs/<slug>` by: (1) stripping the trailing `/design-doc.md` if present; (2) stripping the leading `design-docs/` if present; (3) prepending `design-docs/`. The skill's Step 0 does NOT perform this stripping (per the `cafleet-base-dir` skill § *Consumer contract*) — canonicalize first, then run the skill's **Step 0 (task-scope resolution)** with the relpath `design-docs/<slug>`.

  - **Absolute path** (e.g. `/abs/path/to/design-docs/0000060-foo/design-doc.md`): Step 0 accepts only the task-folder path, not a child file. Strip the trailing `/design-doc.md` if present so the absolute path identifies the task folder, then run Step 0 with that absolute task-folder path. Step 0 accepts the absolute path if it lies strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

  Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder (the slug folder) and `${RESOLVED_ARGS} = ${BASE}/design-doc.md` (short-circuits at Tier 1 below). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${RESOLVED_ARGS}` to the literal `$ARGUMENTS` path so Tier 1 / Tier 2 still run against the user-supplied path, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet-base-dir` skill § *The `<unset>` sentinel*.

- **`$ARGUMENTS` absent** (the discover-all-approved-docs flow): the no-argument form scans `<repo-root>/design-docs/`, so the Director MUST invoke from the repo root. Verify with `git rev-parse --show-toplevel` and abort with a clear "invoke from the repo root" error if `cwd` differs. Then run the skill's **Step 1 (shared-root resolution)**:

  Step 1 resolves `${BASE}` to the CWD (the verified repo root). In the rare edge case where the repo root is itself `$HOME` or under `~/.claude`, Step 1 reaches **Step 2** `AskUserQuestion`; there, explicitly choose the `${CWD}` candidate so `${BASE}` stays the verified repo root — do NOT pick `/tmp/claude-code`, which would make `${RESOLVED_ARGS} = /tmp/claude-code/design-docs/` and point the discovery scan at the wrong directory. With `${BASE}` resolved to the repo root, set `${RESOLVED_ARGS} = ${BASE}/design-docs/` — this matches Tier 3 below and engages the discovery flow that scans every approved slug under `<repo>/design-docs/`.

#### Phase 2: Three-Tier Detection

Using `${RESOLVED_ARGS}`, apply a three-tier detection strategy, evaluated in order:

| Tier | Condition | Action |
|:--|:--|:--|
| 1 — Direct file path | `${RESOLVED_ARGS}` ends with `design-doc.md` | Use as-is |
| 2 — Slug directory | `${RESOLVED_ARGS}` is a directory that contains `design-doc.md` directly | Append `/design-doc.md` |
| 3 — Base directory | `${RESOLVED_ARGS}` is a directory containing `**/design-doc.md` (one level deep) | Enter discovery flow |

Tier evaluation is sequential and short-circuits.

> **Tier 3 with task-scope BASE**: When Phase 1's present-argument branch fires, `${BASE}` is one slug folder and `${RESOLVED_ARGS}` is set to `${BASE}/design-doc.md` — Tier 1 short-circuits before Tier 3 is reached, so the task-scoped BASE never exercises the discovery flow. Tier 3 is preserved for the no-argument branch, where `${BASE}` is the repo root and the discovery flow scans every approved slug under `<repo>/design-docs/`.

#### Discovery Flow (Tier 3)

When the base directory tier matches:

1. **Discover**: Use Glob to find all `**/design-doc.md` files under the base directory, then filter results to keep only those exactly one level deep (i.e., `<base>/<slug>/design-doc.md`). Discard any deeper matches.
2. **Read Status**: For each discovered file, read the `**Status**:` field from the document header.
3. **Filter**: Keep only documents with `Status: Approved`. Documents with any other status (`Draft`, `In Progress`, `Complete`) are excluded.
4. **Branch by count**:

| Count | Behavior |
|:--|:--|
| 0 | Error and abort (see Error: Zero Approved below) |
| 1 | Auto-select: proceed with this document directly |
| 2–4 | Present options via `AskUserQuestion` (see Selection UI below) |
| 5+ | Present options via paginated `AskUserQuestion` (see Pagination below) |

#### Selection UI (2–4 Approved Docs)

Use `AskUserQuestion` with one question. Each option label is the slug name (directory name) of the design doc. The built-in "Other" option is always available for the user to type a direct path or cancel.

Example with 3 approved docs:

```
Question: "Which design document would you like to implement?"
Options:
  1: "feature-auth"
  2: "refactor-db-layer"
  3: "add-cli-export"
  (Other is added automatically)
```

#### Pagination (5+ Approved Docs)

When there are more than 4 approved docs, `AskUserQuestion`'s option limit (max 4) is exceeded. Use pagination with all options sorted alphabetically by slug:

- **Non-last page**: Show 3 options + a 4th option labeled `"More..."`.
- **Last page rule**: If remaining items after the current page would be ≤ 4, show all remaining items directly (no `"More..."` needed). This avoids a last page with only 1 option, which would violate `AskUserQuestion`'s minimum of 2 options per question.
- Continue until the user selects a document or uses "Other".

Example with 7 approved docs: page 1 shows 3 + "More..." (4 remain), page 2 shows all 4. Example with 5: page 1 shows 3 + "More..." (2 remain), page 2 shows both 2.

#### Error: Zero Approved Docs

When design docs exist but none have `Status: Approved`, display a message listing all found docs with their current statuses so the user understands why none qualified. Format:

```
No approved design documents found in <base-directory>.

Found documents:
  - <slug-1>/design-doc.md — Status: Draft
  - <slug-2>/design-doc.md — Status: In Progress
  - <slug-3>/design-doc.md — Status: Complete

Only documents with Status: Approved can be executed. Update the status or specify a direct path.
```

Then abort (do not proceed to team creation or execution).

#### Error: Invalid Path

When `${RESOLVED_ARGS}` does not match any of the three tiers (not a file path ending in `design-doc.md`, not a directory containing `design-doc.md`, and no `**/design-doc.md` found underneath), display:

```
Invalid argument: `${RESOLVED_ARGS}`
Expected one of:
  - Path to a design-doc.md file (e.g., my-feature/design-doc.md)
  - Slug directory containing design-doc.md (e.g., my-feature/)
  - No argument (discovers all design docs in ${BASE}/design-docs/)
```

Then abort.

After resolution, the resolved path is used as the design document path for all subsequent steps.

### Step 2: Validate Design Document & Create Branch (Director)

Before registering with CAFleet:

1. Read the design document completely.
2. Check for `COMMENT(` markers using Grep. If found, resolve them directly: apply the requested changes and remove the markers. Verify with Grep that no `COMMENT(` markers remain before proceeding.
3. Check for `FIXME(claude)` markers in the codebase using Grep. If found, note them for the Programmer to resolve first.
4. Determine the step order and total number of steps.
5. **Create a feature branch if on the default branch.** Get the default branch with `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` and the current branch with `git branch --show-current`. If they match, use `AskUserQuestion` to propose the branch name `feat/<design-doc-slug>` and ask the user to approve before creating it. The user will create the branch themselves or approve the proposed name. If already on a non-default branch, skip this step.

### Step 3: Register & Spawn Members (Director)

Load the `cafleet` skill, the `cafleet-agent-team-monitoring` skill, and the `cafleet-agent-team-supervision` skill (in that order — monitoring is the foundation layer, supervision the governance layer that depends on it).

#### 3a. Establish a CAFleet fleet and capture the root Director's `agent_id`

`cafleet fleet create` (which must be run inside a tmux session) atomically creates the fleet and registers a root Director bound to the current tmux pane — there is no separate `cafleet agent register` step for the Director. Use `--json` so both IDs are machine-parseable:

```bash
cafleet fleet create --label "design-doc-execute-{slug}" --json
# → {
#     "fleet_id": "550e8400-e29b-41d4-a716-446655440000",
#     "label": "design-doc-execute-{slug}",
#     "created_at": "…",
#     "administrator_agent_id": "…",
#     "director": {
#       "agent_id": "7ba91234-…",
#       "name": "Director",
#       "description": "Root Director for this fleet",
#       "registered_at": "…",
#       "placement": { "director_agent_id": null, "tmux_session": "…", "tmux_window_id": "…", "tmux_pane_id": "…", "coding_agent": "unknown", "created_at": "…" }
#     }
#   }
```

Capture `fleet_id` and `director.agent_id` from the JSON response. Substitute them for `<fleet-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal UUIDs. Remember: `--fleet-id` is a global flag that goes **before** the subcommand; `--agent-id` is a per-subcommand option that goes **after** the subcommand name.

If you already have a running fleet (e.g. an outer orchestration), reuse its `fleet_id` and its root Director's `agent_id` instead of creating a new fleet. Do **not** attempt to register a second Director with `cafleet agent register --name Director` — the root Director from `fleet create` is the team lead; a second registration would just create an unrelated agent with no placement row.

#### 3b. Start the monitoring `/loop`

BEFORE spawning any member, use the `cafleet-agent-team-monitoring` skill's `/loop` Prompt Template and start a `/loop` monitor at the 1-minute interval using the literal `<fleet-id>` and `<director-agent-id>` UUIDs. This is the **team-health loop** — it stays active through Steps 3–5 and, when Step 6 runs, is swapped (create-before-delete order in Step 7a) for the augmented team-health + PR-review loop. Whichever loop is active gets `CronDelete`d in Step 8's cleanup. Supervision obligations (Authorization-Scope Guard, idle semantics, etc.) come from the `cafleet-agent-team-supervision` skill, which loads the `cafleet-agent-team-monitoring` skill as a hard prerequisite.

#### 3c. Analyze implementation tasks to decide team composition

Based on the design document steps (see [roles/director.md](roles/director.md) for the full decision matrix):

| Task nature | Team composition |
|---|---|
| Code implementation | Programmer + Tester |
| Config/documentation only | Programmer only |
| E2E verification needed (user-visible changes, CLI/UI/API) | + Verifier |

#### 3d. Read role files

Resolve the absolute path of each role file you will reference by path-by-reference in spawn prompts (the member opens the file via `Read` on its first turn — do NOT inline the content):

- `skills/cafleet-design-doc-execute/roles/programmer.md`
- `skills/cafleet-design-doc-execute/roles/tester.md` (if Tester needed)
- `skills/cafleet-design-doc-execute/roles/verifier.md` (if Verifier needed)

#### 3e. Spawn each member via `cafleet member create`

The spawn prompts below use `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders — cafleet `member create` runs `str.format()` over the entire prompt and substitutes these from the new member's allocated `agent_id`, the fleet ID, and the spawning Director's `agent_id`. The `[INSERT …]` markers (e.g. `[INSERT DESIGN DOC PATH]`, `[INSERT abs path to roles/programmer.md]`) are NOT format placeholders — the Director substitutes them in shell before calling `member create`. See the Template safety note under `Member Create` in `skills/cafleet/reference/director.md`.

> **Path-by-reference for role docs**: Each spawn prompt below references its role file by **absolute path**. The spawned member opens its role doc with `Read` on its first turn. Do NOT inline the role content — cafleet `member create` hits a `tmux command failed: command too long` error once the shell-quoted prompt grows past a few KB, and rolls back the registration. See `skills/cafleet/reference/director.md` § *Spawn prompt size limit* for the canonical write-up. Resolve the absolute path for each of `roles/programmer.md`, `roles/tester.md`, and `roles/verifier.md` (from this skill's `roles/` directory) and substitute into the `[INSERT abs path to …]` markers below.
>
> **Spawn-prompt audit file**: every spawn in this skill writes the rendered prompt to `${BASE}/prompts/<role>-<UTC-compact>.md` BEFORE invoking `cafleet member create --prompt-file <abs path>` (see the per-role flow below). The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet-base-dir` skill § *No-bypass write protocol* and the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

**Programmer spawn prompt:**

```
You are the Programmer in a design document execution team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/programmer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact — the CLI's `str.format()` pass resolves them at member-create time using the newly-allocated `agent_id`.
2. **Write the rendered text** to `${BASE}/prompts/programmer-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 1; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Create `${BASE}/prompts/` on first write (Python: `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Programmer" \
     --description "Implements code to pass tests per step" \
     --prompt-file ${BASE}/prompts/programmer-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<programmer-agent-id>` in every subsequent command. The pre-spawn file at `${BASE}/prompts/programmer-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

**Tester spawn prompt (if needed):**

```
You are the Tester in a design document execution team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/tester.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code yourself. The Director handles all git operations.
IMPORTANT: Do NOT write implementation code — only test code.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.

Start by reading the design document. Then wait for the Director to assign your first step.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/tester-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 1; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Tester" \
     --description "Writes unit tests per step" \
     --prompt-file ${BASE}/prompts/tester-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<tester-agent-id>` in every subsequent command. The pre-spawn file at `${BASE}/prompts/tester-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

**Verifier spawn prompt (if needed):**

> **Phase 1 exemption**: The Verifier's first message — a tool-and-MCP inventory — is a one-time discovery payload, not iterative coordination, and rides as a free-form multi-line cafleet body (same precedent as the Analyzer's question list in the `cafleet-design-doc-interview` skill). Phase 2 verification reports follow the verb + pointer + `COMMENT(verifier)` schema documented in [the Coordination Protocol section above](#coordination-protocol).

```
You are the Verifier in a design document execution team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/verifier.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

IMPORTANT: Do NOT commit code or modify implementation/test files.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.

Start by reading the design document and discovering available tools.
Then wait for the Director to assign your first verification task.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/verifier-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 1; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Verifier" \
     --description "E2E/integration testing and evidence collection" \
     --prompt-file ${BASE}/prompts/verifier-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<verifier-agent-id>` in every subsequent command. The pre-spawn file at `${BASE}/prompts/verifier-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

#### 3f. Verify members are live

```bash
cafleet --fleet-id <fleet-id> member list --agent-id <director-agent-id>
```

All spawned members must show `status: active` with a non-null `pane_id`. If any is missing or pending, retry the spawn before proceeding.

See [roles/director.md](roles/director.md) for commit message conventions.

### Step 4: Execute Steps with Per-Step TDD Cycle (Director)

For each step in the design document:

#### Phase A: Test Writing

**Skip this phase entirely when the Tester was not spawned** (Programmer-only team composition for config/documentation-only steps). Proceed directly to Phase B and assign the step to the Programmer without a separate test-writing commit.

1. **Assign**: Send the Tester a verb + pointer poke. The Tester reads the step description and specification directly from the design document at the pointer.
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <tester-agent-id> --text "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Tester's `complete (paragraph-Implementation > Step N) — <count> tests` (or `blocked (paragraph-Implementation > Step N)` if the spec is unclear)** via `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>`. On `blocked`, read the Tester's `COMMENT(tester)` marker at the same pointer (per the pointer-marker pairing rule in the Coordination Protocol section above); if the test framework is ambiguous (per the Tester's `Phase 1` selection step, which uses `blocked (doc)` with the marker at doc-top), ask the user via `AskUserQuestion`, write the answer back as `COMMENT(claude): <choice>` at the same doc-top location, and reply with `ready (doc)` so the Tester resumes.
3. **Review tests** against the design doc. If issues are found, write `COMMENT(director): <issue>` markers at `paragraph-Implementation > Step N` (matching the cafleet pointer per the pointer-marker pairing rule in the Coordination Protocol section above) and reply `ready (paragraph-Implementation > Step N)`; the Tester resolves the markers and replies `addressed (paragraph-Implementation > Step N)`. Repeat until satisfied.
4. **Commit tests** (separate commands, do NOT chain with `&&`). Recover the per-test file list directly via git (`git status` / `git diff --stat` / `git log --name-only`) — the Tester does not embed file lists in cafleet bodies under the verb + pointer schema.
   - `git add <test-files>`
   - `git commit -m "test: add tests for [feature description]"`

#### Phase B: Implementation

1. **Assign**: Send the Programmer a verb + pointer poke. The Programmer reads the step spec at the pointer and locates the Tester's freshly-committed test files via git (`git log <base>..HEAD --name-only -- '**/test_*' '**/tests/**'`); the prior Tester `complete (...) — N tests` summary went Tester → Director, not Tester → Programmer.
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <programmer-agent-id> --text "ready (paragraph-Implementation > Step N)"
   ```
2. **Wait for the Programmer's `complete (paragraph-Implementation > Step N)`** via `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>`. On `escalating (paragraph-Implementation > Step N)` (suspected test defect), see [roles/director.md](roles/director.md) for the escalation protocol; the rationale lives in a `COMMENT(programmer)` marker at the pointer, not in the cafleet body.
3. **Programmer updates design doc**: Checkboxes, timestamps, and Progress counter.

#### Phase C: Code Review (Director)

1. **Review**: Verify code matches design doc, quality is acceptable, no unnecessary changes.
2. **Feedback loop**: If issues are found, write a `COMMENT(director): <issue>` marker — for design-doc-anchored issues, place it at `paragraph-Implementation > Step N` and send `ready (paragraph-Implementation > Step N)`; for source-anchored issues, place it at `<file>:<line>` and send `ready (<file>:<line>)`. The marker location MUST match the cafleet pointer (per the pointer-marker pairing rule in the Coordination Protocol section above). The Programmer resolves the markers, re-runs tests, and replies `addressed (paragraph-Implementation > Step N)` (or `addressed (<file>:<line>)`). Repeat until satisfied.
3. **Commit implementation** (separate commands, do NOT chain with `&&`). Recover the per-file list via git (`git status` / `git diff --stat <base>..HEAD`):
   - `git add <files> <design-doc>`
   - `git commit -m "feat: [description of what was implemented]"`

Repeat from Phase A for the next step. Always include the design document in the implementation commit.

**Escalation Protocol (Test Defect):** When the Programmer sends `escalating (paragraph-Implementation > Step N)`, the Director reads the design doc paragraph, the Programmer's `COMMENT(programmer)` rationale at that pointer (the marker MUST live at `paragraph-Implementation > Step N` per the pointer-marker pairing rule in the Coordination Protocol section above), and the failing test. The Director then writes a `COMMENT(director): <decision> — <rationale, ≤2 sentences>` marker at the same `paragraph-Implementation > Step N` stating the arbitration outcome, and sends `ready (paragraph-Implementation > Step N)` to whichever member needs to act (Tester to fix the test, or Programmer to adjust the implementation). The recipient acts on the standing markers and replies `addressed (paragraph-Implementation > Step N)`. 3-round limit before escalating to the user.

**On-Demand Verification**: Any member can request verification mid-task via `cafleet message send` to the Director. The Director decides whether to route immediately or defer:

| Route immediately | Defer to Phase D |
|:--|:--|
| User-visible behavior change (UI, CLI output, API response) | Internal refactoring or data model change |
| Integration with external system | Adequately covered by unit tests |
| Behavior difficult to catch with unit tests alone | Verification requires setup from a later step |

### Phase D: Verification (Director) — conditional

**Skip this phase entirely if the Verifier was not spawned.** Proceed directly to Step 5 (User Approval).

If the Verifier was spawned, assign verification:

1. Send the Verifier a verb + pointer poke — the Verifier reads the design document and the completed Implementation paragraphs directly at the pointer:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <verifier-agent-id> --text "ready (doc)"
   ```
2. The Verifier discovers tools, executes E2E verification, captures evidence, and writes each fail / suggested-fix as a `COMMENT(verifier): <category> <body>` marker (category = impl bug / test gap / spec issue). Marker location MUST match the cafleet pointer used to report the failure — for per-step `escalating (paragraph-Implementation > Step N)` reports, the paired `COMMENT(verifier)` marker lives at the SAME `paragraph-Implementation > Step N` (per the pointer-marker pairing rule in the Coordination Protocol section above). On overall success the Verifier sends a single `complete (doc)`; on failures the Verifier sends one `escalating (paragraph-Implementation > Step N)` per affected step.
3. **Route failures** by reading the standing `COMMENT(verifier)` markers and dispatching with `ready (paragraph-Implementation > Step N)`: impl-bug markers → Programmer, test-gap markers → Tester, spec-issue markers → Director resolves directly via `COMMENT(director)` arbitration (or escalates to the user via `AskUserQuestion` if a product decision is needed).
4. Re-verify after fixes. Proceed to User Approval when all verifiable criteria pass.

### Step 5: User Approval (Director)

After all TDD steps complete but before finalization, present the implementation to the user for approval.

#### Success Criteria Verification

**Before presenting to the user**, verify the design document's Success Criteria section:

1. Read the `## Success Criteria` section from the design document.
2. For each criterion, verify it is satisfied by inspecting the implementation (grep, read files, run tests as needed).
3. Check off all satisfied criteria in the design document (`- [ ]` → `- [x]`).
4. If any criterion is NOT satisfied, resolve it before proceeding to user approval — route to Programmer or Tester as needed via `cafleet message send`.

This step is **mandatory** and must not be skipped.

#### Change Presentation

1. **Git diff command** for the user to inspect (e.g., `git diff main...HEAD`).
2. **Step-by-step change summary** — concise prose of what changed per step (files modified, key behaviors).

#### Approval Interaction

Use `AskUserQuestion`:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with push, PR creation, Copilot review loop, then finalize | Steps 6 → 7 → 8 |
| 2 | **Scan for COMMENT markers** | Add `COMMENT(name): feedback` markers to the changed source files, then select this option to process them | Scan and process markers (see Revision Loop below) |
| 3 | *(Other — built-in)* | *(Free text input, e.g. "approve but skip PR")* | Interpret user intent (see Revision Loop below). Intent judgment recognises an **approve-local** variant that skips Steps 6 + 7 and jumps straight to Step 8 (local finalize only, no push/PR). Abort intent triggers the Abort Flow. |

See [roles/director.md](roles/director.md) for user interaction rules (COMMENT handling, classification, intent judgment, abort detection).

#### Revision Loop (COMMENT Marker-Based Feedback)

When the user selects "Scan for COMMENT markers": scan changed files for `COMMENT(` markers. Classify by file location (see [roles/director.md](roles/director.md)) and route via the verb + pointer schema:
- Design-doc `COMMENT(...)` markers → Director resolves directly (apply spec change, remove marker; no cafleet route).
- Source-file `COMMENT(...)` markers → `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <programmer-agent-id> --text "ready (<file>:<line>)"`. The Programmer reads the marker at the source pointer, fixes the source, removes the marker, and replies `addressed (<file>:<line>)`.
- Test-file `COMMENT(...)` markers → `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <tester-agent-id> --text "ready (<file>:<line>)"`. The Tester reads, fixes, removes the marker, and replies `addressed (<file>:<line>)`.

After all `COMMENT(...)` markers are resolved and verified, re-present to user.

When the user selects "Other": interpret intent per [roles/director.md](roles/director.md) rules.

No round limit — the loop continues until the user approves or aborts.

#### Abort Flow

1. Update design document Status to "Aborted", add Changelog entry. Place a `COMMENT(director): aborting — finalize and stand by` marker near the top of the doc body (above the Overview section — `Status:` is bold metadata, not a heading, so it is not a valid `paragraph-` target). Notify any still-live members with a single `cafleet --fleet-id <fleet-id> message send ... --text "ready (doc)"` per member so they read the marker and stand by.
2. Commit (separate commands): `git add <design-doc>` then `git commit -m "docs: mark design doc as aborted"`
3. Follow Shutdown Protocol (Step 8: cancel whichever `/loop` is active — team-health if Step 6 was skipped, augmented if Step 7 started — then delete members and run `cafleet fleet delete <fleet-id>` to tear down the fleet and sweep the root Director + Administrator).

### Step 6: Push & Create PR (Director)

After Step 5 Approve, the Director pushes the feature branch, opens a PR, and requests a Copilot review BEFORE marking the design doc complete. Every command is run as a separate Bash call — do NOT chain with `&&`.

#### 6a. Preconditions (checked in order; first failure aborts to Step 8 local-finalize)

| Check | Command | Failure action |
|:--|:--|:--|
| `gh` authenticated | `gh auth status` | Report `gh not authenticated; skipping PR creation` → Step 8 local-finalize |
| Not on default branch | `git branch --show-current` vs `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` | Report `on default branch; cannot open PR` → Step 8 local-finalize |
| Branch has commits beyond base | `git log <base>..HEAD --oneline` | Report `no commits to push` → Step 8 local-finalize |

#### 6b. Procedure

1. **Resolve owner/repo**: `gh repo view --json nameWithOwner --jq '.nameWithOwner'`. Capture the literal `<owner>/<repo>` string (e.g. `himkt/cafleet`) and substitute it into every `gh api repos/<owner>/<repo>/...` call below. Like the PR number, this is a literal string — NO shell variables.
2. **Initial push**: `git push -u origin <branch-name>`. If this fails (non-fast-forward, branch protection, etc.), report the exact stderr to the user and proceed to Step 8 local-finalize. NEVER force-push.
3. **Check for an existing PR on this branch**: `gh pr list --head <branch-name> --json number --jq '.[0].number // empty'`. If the result is non-empty, reuse that PR number. Otherwise, run `gh pr create --fill` and parse the printed URL's trailing number.
4. **Record PR number literally**: store the PR number (e.g. `42`) and substitute it into `<pr-number>` in every subsequent command. DO NOT use a shell variable — `permissions.allow` matches literal command strings.
5. **Request Copilot review**: `gh pr edit <pr-number> --add-reviewer @copilot`.
6. **Verify the review request**: `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers` should list Copilot. If Copilot is absent from the response AND no Copilot review already exists (`gh pr view <pr-number> --json reviews`), report `Copilot reviewer unavailable for this PR` and proceed to Step 8 local-finalize.
7. **Capture `last_push_ts`**: record the ISO 8601 timestamp of the push completion (the Director's wall-clock time captured immediately after step 2 returned, or `date -u +%Y-%m-%dT%H:%M:%SZ`). This initialises the in-context loop state described in the "PR Review Loop State" subsection below.

### Step 7: Copilot Review Loop (Director)

Once the PR exists and Copilot has been invited, the Director runs a cron-driven review loop. The `cafleet-agent-team-monitoring` team-health `/loop` is replaced by an **augmented** loop that keeps the team-health checks AND adds PR review polling.

#### PR Review Loop State

The Director holds three **PR-review-specific** in-context variables across loop firings (separate from the team-health `--since` timestamp tracked by the `cafleet-agent-team-monitoring` skill for `cafleet message poll`). They are NOT persisted to disk — the Director carries them in its own working memory.

| Variable | Meaning | Update rule |
|:--|:--|:--|
| `last_push_ts` | ISO 8601 timestamp of the most recent push to the PR branch | Reset on every `git push` from 6b-step 2 or 7d-step 3 |
| `silence_ticks` | Consecutive loop ticks with 0 new Copilot items since the last activity | Increment each tick with 0 new items; reset to 0 when new Copilot items arrive OR after a fix-push from 7d |

#### 7a. Replace the monitoring `/loop` (create-before-delete)

On entry to Step 7:

1. **Start the augmented `/loop` first** with the template in "Augmented Loop Prompt" below. Record the new cron ID.
2. **Then `CronDelete` the existing team-health loop** (cron ID recorded in Step 3b).
3. The new loop keeps every team-health check AND adds PR review polling.

Order matters: create-before-delete eliminates any window where no monitor is running. A one-tick overlap (both loops firing for one minute) is harmless — the Director receives two nudge prompts and reconciles them trivially.

On exit from Step 7 (any exit condition), keep the augmented loop running — Step 8's shutdown is responsible for the final `CronDelete`.

#### 7b. Per-tick procedure

On each 1-minute wake-up, the Director runs — in order:

1. **Team health** (unchanged from the `cafleet-agent-team-monitoring` skill): `member list` → `poll` → `member capture` fallback → nudge stalled members.
2. **Fetch new PR reviews**: `gh pr view <pr-number> --json reviews` (GraphQL-shaped; fields are `author.login`, `state`, `submittedAt`, `body`) AND `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments` (REST-shaped; fields are `user.login`, `body`, `path`, `line`, `created_at`).
3. **Filter Copilot-authored entries**: keep items where the login field (`author.login` for `gh pr view` reviews, `user.login` for `gh api` inline comments) matches the regex `^copilot` (case-insensitive). Copilot reviews currently post under a login that begins with `copilot` — the exact slug varies by account plan, so a prefix match is the safe filter.
4. **New-since-push filter**: keep items whose timestamp (`submittedAt` for reviews, `created_at` for inline comments) is strictly later than `last_push_ts`.
5. **Branch on the filter result**:

| Result | Action |
|:--|:--|
| The most recent **post-push** Copilot-authored entry in `reviews` (i.e., one with `submittedAt > last_push_ts`) has `state == "APPROVED"` | Exit loop (success) → Step 8 |
| 0 new Copilot items AND `silence_ticks < 30` | Increment `silence_ticks`, continue waiting |
| 0 new Copilot items AND `silence_ticks >= 30` | Silence-escalation: see 7e |
| ≥ 1 new Copilot items | Reset `silence_ticks = 0`, go to 7c |

The APPROVED check MUST be qualified by the post-push filter (`submittedAt > last_push_ts`). An older approval — say, from a Copilot pass before the most recent fix-push — must NOT be treated as approval of the current HEAD; otherwise a single early approve followed by additional commits would silently finalize the PR.

**Why no auto-exit on silence**: a silent Copilot is NOT proof Copilot is done. Copilot may take longer than expected to re-review after a fix-push, may not have been re-triggered yet, or may be back-pressured. **Auto-exiting** on silence risks finalizing a PR while Copilot is still composing comments. The loop never auto-exits on silence; it instead **escalates to the user** via 7e after 30 consecutive silent ticks (~30 minutes), so the user — not the loop — chooses whether to keep waiting, re-request the review, or finalize. Outside that user gate, the loop only exits on an explicit `state == "APPROVED"` signal, on "Stop means stop", or on the cron's natural 7-day expiry.

**Why not `reviewDecision`**: the PR-level `reviewDecision` only reflects required reviewers (typically CODEOWNERS). Copilot is usually not a CODEOWNER, so an approve from Copilot alone leaves `reviewDecision` null/REVIEW_REQUIRED. Reading the Copilot-specific entry in the `reviews` array is the reliable signal.

#### 7c. Classify and route

For each new inline comment, pick the owner by file-path pattern. **Source-anchored** Copilot lines route via the verb + pointer schema; the Director writes a `COMMENT(copilot): <body>` marker at the source pointer (because that is where the comment lives) and pokes the routed member with `ready (<file>:<line>)`. **Design-doc-anchored** Copilot lines do NOT route to a member — the Director writes a `COMMENT(director): <body>` marker at the affected paragraph, applies the spec change, and removes the marker as part of the fix; no cafleet message is sent (the git commit + marker removal is sufficient audit trail).

| Path pattern | Owner | Marker location | Route |
|:--|:--|:--|:--|
| Design doc (`design-docs/**/design-doc.md`) | Director | `COMMENT(director): <body>` at the affected paragraph in the design doc | (no cafleet route — Director resolves silently) |
| Test file (e.g. `**/test_*.py`, `**/*_test.py`, `**/tests/**`) | Tester | `COMMENT(copilot): <body>` in the test file at `<file>:<line>` | `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <tester-agent-id> --text "ready (<file>:<line>)"` |
| Any other source file | Programmer | `COMMENT(copilot): <body>` in the source file at `<file>:<line>` | `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <programmer-agent-id> --text "ready (<file>:<line>)"` |

The routed member fixes the source, removes the `COMMENT(copilot)` marker as part of the fix, and replies `addressed (<file>:<line>)`.

For review-level comments (body text not attached to a specific line), route by Director judgment: spec-level → `COMMENT(director)` in design doc, Director resolves directly; implementation-level → `COMMENT(copilot)` at a representative source `<file>:<line>` + `ready (<file>:<line>)` to the Programmer; test-level → `COMMENT(copilot)` at a representative test `<file>:<line>` + `ready (<file>:<line>)` to the Tester.

#### 7d. Fix, commit, push, re-request

1. Wait for each routed member to report completion via `cafleet message poll`. Members do NOT commit — the Director commits after each report.
2. Commit fixes per scope (each `git add` / `git commit` is its own Bash call, no `&&`):
   - Programmer fixes: `git commit -m "fix: address Copilot review - <short summary>"`
   - Tester fixes: `git commit -m "fix: address Copilot test review - <short summary>"`
   - Director doc fixes: `git commit -m "docs: address Copilot review - <short summary>"`
3. `git push` (no flags — the branch already tracks origin from Step 6).
4. Update `last_push_ts` to the post-push wall-clock timestamp and reset `silence_ticks = 0` (the new push restarts the review window).
5. Re-request Copilot review: `gh pr edit <pr-number> --add-reviewer @copilot`. Re-adding the same reviewer triggers a fresh Copilot pass.
6. Continue the loop.

#### 7e. Silence escalation

When `silence_ticks >= 30` (≈ 30 minutes since the last Copilot activity AND no new items this tick), escalate to the user via `AskUserQuestion`:

| Option | Behavior |
|:--|:--|
| 1. Keep waiting | Reset `silence_ticks = 0`, continue Step 7 |
| 2. Re-request review | Run `gh pr edit <pr-number> --add-reviewer @copilot` to re-trigger Copilot, reset `silence_ticks = 0`, continue Step 7 |
| 3. Finalize now | Exit loop → Step 8 (accept the current state of Copilot review as-is) |
| 4. *(Other)* | Intent judgment; abort-intent → Abort Flow |

The 30-tick threshold is conservative: Copilot's first review after a `--add-reviewer` typically lands within 3–5 minutes. 30 minutes is enough that Copilot is highly unlikely to still be composing, while leaving the *decision* to the user instead of the loop. The user retains the option to keep waiting indefinitely — the loop never finalizes on its own based on silence.

#### Augmented Loop Prompt

Use this as the `/loop` prompt for Step 7. Substitute the literal UUIDs and the literal PR number before passing the prompt to `/loop` — no shell variables.

```
Monitor team health AND PR review state (interval: 1 minute).

TEAM HEALTH:
1. Run `cafleet --fleet-id <fleet-id> --json member list --agent-id <director-agent-id>`.
2. Run `cafleet --fleet-id <fleet-id> --json message poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check>"`. ACK progress reports.
3. For each member that has not sent a message since last check, run `cafleet --fleet-id <fleet-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 200`.
4. Nudge stalled members via `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "ready (<original-pointer>)"` — re-send the same `ready (paragraph-Implementation > Step N)` or `ready (<file>:<line>)` body that was used for the original assignment. The recipient interprets a re-sent `ready (...)` contextually as a stall-nudge per [the Coordination Protocol section above](#coordination-protocol) (same target, same expected action).

PR REVIEW:
5. Run `gh pr view <pr-number> --json reviews` (GraphQL shape: `author.login`, `state`, `submittedAt`, `body`).
6. Run `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments` (REST shape: `user.login`, `body`, `path`, `line`, `created_at`).
7. Filter to entries where the appropriate login field (`author.login` for GraphQL reviews, `user.login` for REST inline comments) starts with `copilot` (case-insensitive) and the appropriate timestamp (`submittedAt` / `created_at`) > `last_push_ts` (the in-context state variable defined under "PR Review Loop State").
8. If the most recent Copilot-authored entry **in the filtered (post-push) set from step 7** has `state == "APPROVED"`: signal Step 7 exit (success). An older approval (i.e., one with `submittedAt <= last_push_ts`) must NOT trigger this exit.
9. If filter returned 0 entries AND `silence_ticks < 30`: increment `silence_ticks`, continue waiting (do nothing this tick). The loop never auto-exits on Copilot silence.
10. If filter returned 0 entries AND `silence_ticks >= 30`: silence-escalation per 7e — AskUserQuestion (Keep waiting / Re-request review / Finalize now / Other). Reset `silence_ticks = 0` if the user picks Keep waiting or Re-request review; otherwise honor the user's choice.
11. If filter returned ≥ 1 entries: reset `silence_ticks = 0`, classify by file path per Step 7c, write `COMMENT(copilot): <body>` at the source `<file>:<line>` for source/test routes (or `COMMENT(director): <body>` at the affected paragraph for design-doc-anchored items), and dispatch via `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "ready (<file>:<line>)"`. Design-doc-anchored Copilot items are NOT routed — the Director resolves them directly and skips the `cafleet message send`.

ESCALATION:
12. If any member has been nudged 2 times with no progress, escalate to the user.
```

#### Error Handling (Steps 6–7)

| Case | Detection | Behavior |
|:--|:--|:--|
| `gh auth status` fails | Step 6a precondition check | Skip Steps 6 + 7, go directly to Step 8 local-finalize |
| On default branch | Step 6a precondition check | Skip Steps 6 + 7, go directly to Step 8 local-finalize |
| No commits beyond base | Step 6a precondition check | Skip Steps 6 + 7, go directly to Step 8 local-finalize |
| `git push` rejected | stderr of `git push` | Report exact stderr to user, skip Step 7, go to Step 8 local-finalize. NEVER force-push. |
| `gh pr create` fails | stderr of `gh pr create` | Report, skip Step 7, go to Step 8 local-finalize |
| `@copilot` reviewer unavailable | `gh api .../requested_reviewers` shows no Copilot AND no prior Copilot review | Report `Copilot reviewer unavailable for this PR`; skip Step 7; go to Step 8 |
| Fix-push fails mid-loop (any subsequent push after the initial one) | stderr of `git push` | Escalate to user (AskUserQuestion: retry / finalize now / abort) |
| User selects "Other" in Step 5 with abort-intent text | Existing LLM intent judgment | Abort Flow (unchanged — no push) |
| User selects "Other" in Step 5 with approve-local intent | Existing LLM intent judgment, extended | Skip Steps 6 + 7; go to Step 8 local-finalize |

#### User Interjection During Step 7

`/loop` firings keep arriving while the user is speaking to the Director. **Stop means stop**: when the user signals halt (explicit "stop", "wait", "pause", profanity / frustration, or repeated rejection of tool calls), the Director MUST halt dispatch immediately and wait for explicit re-authorization — `/loop` ticks and idle notifications during the halted state are NOT instructions and must be skipped silently. Concretely, the Director:

1. Stops dispatching new `cafleet message send` / `git commit` / `git push` / `gh` actions immediately.
2. Acknowledges the user briefly and waits for explicit instructions.
3. Treats subsequent `/loop` firings as notification-only — runs the PR review poll for situational awareness but does NOT route comments, commit, or push until the user re-engages with a specific instruction.
4. Does NOT silently tear the team down — the state stays paused so the user can resume or explicitly abort.

If the user explicitly aborts, follow the Abort Flow (update doc Status → "Aborted", commit, run Shutdown Protocol). Step 7's cleanup is identical to Step 8's cleanup — `CronDelete` the augmented loop, delete members, run `cafleet fleet delete`.

### Step 8: Finalize & Clean Up (Director)

Runs after Step 7 exits, or directly after Step 5 when Step 6 was skipped (gh not authenticated / default branch / no commits / approve-local intent).

1. Update design document Status to "Complete" and add a final Changelog entry.
2. `git add <design-doc>` (separate Bash call).
3. `git commit -m "docs: mark design doc as complete"` (separate Bash call).
4. **Push decision** (separate Bash call): run `git rev-parse --abbrev-ref <branch-name>@{upstream}`.
   - Exit code 0 (branch is tracked on origin): `git push`. Covers both the "Step 6 fully succeeded" path and the "Step 6 partial-fail (push OK, PR create failed)" path.
   - Non-zero exit: skip the push. The docs commit stays local.
   - The Director does NOT re-request Copilot review on this final docs commit.
5. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol*:
   1. `CronDelete` the currently active `/loop` monitor — team-health (cron ID from Step 3b) if Step 6 was skipped, augmented (cron ID from Step 7a) otherwise.
   2. `cafleet member delete` for each spawned member (Programmer, Tester if spawned, Verifier if spawned). Each call blocks until the pane is gone; on exit 2 follow the `member capture` + `send-input` recovery, or rerun with `--force`.
   3. `cafleet member list` — the team's roster MUST be empty before continuing.
   4. `cafleet fleet delete <fleet-id>`.
   5. `cafleet fleet list` — the fleet MUST not appear.
6. **Report to the user**: include the PR URL (if Step 6 created one), the Copilot loop exit reason (approved / silence-escalated / skipped / aborted), and any skipped-step reasons.
