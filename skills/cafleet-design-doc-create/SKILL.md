---
name: cafleet-design-doc-create
description: "Create a new design document using CAFleet-native orchestration (Director/Drafter/Reviewer team). Use when the user asks to create a design doc, design document, specification, or technical spec. Do NOT write the doc directly with Write or use EnterPlanMode — always invoke this skill."
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch
---

# Design Doc Create (CAFleet Edition)

Create high-quality design documents using a three-role team orchestrated via the CAFleet message broker: Director (orchestrator), Drafter (writes the document), and Reviewer (critically reviews drafts). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. The team iterates through an internal quality loop before presenting a polished draft to the user.

| Role | Identity | Does | Does NOT | Role definition |
|:--|:--|:--|:--|:--|
| **Director** | Main Claude | Register with CAFleet fleet, spawn members via `cafleet member create`, relay user answers, enforce clarification gate, orchestrate internal quality loop, present polished draft to user | Write the document, review it in detail | [roles/director.md](roles/director.md) |
| **Drafter** | Member agent (claude) | Ask clarifying questions (via Director relay), read target codebase, write and revise the design document | Communicate with user directly (goes through Director), review own work | [roles/drafter.md](roles/drafter.md) |
| **Reviewer** | Member agent (claude) | Critically review drafts for rule compliance, readability, completeness, correctness | Write the document, communicate with user | [roles/reviewer.md](roles/reviewer.md) |

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
| `approved` | Reviewer → Director, or Director → user-result | "All quality criteria are met at the pointer (typically `doc`)." | Reviewer approval signal in the `cafleet-design-doc-create` skill. |

**Verb choice for `complete` vs `addressed`**: `complete` signals a fresh deliverable (work that did not previously have a marker waiting). `addressed` signals resolution of a pre-existing marker (any `COMMENT(role)`, any Copilot line, any Director arbitration). When in doubt, ask: "did a marker exist before I started this turn?" — if yes, use `addressed`; if no, use `complete`.

**Director stall-nudges** reuse `ready (doc)` or `ready (paragraph-...)` — they are `ready` from the Director's perspective ("the pointer target is ready for you, please act"). The recipient interprets contextually: on receiving `ready (...)`, scan the pointer target for any `COMMENT(role)` markers addressed to your role or relevant to your current phase, and act accordingly. Re-sent `ready (...)` after a member's idle window is a nudge, not a new assignment — same target, same expected action.

### Pointer Forms

Exactly 3 canonical forms. Use the tightest one that locates the target.

| Form | Example | When to use |
|:--|:--|:--|
| `paragraph-<HeadingPath>` | `paragraph-Implementation > Step 2` | The target is a heading (or sub-heading) inside the design document. Use the literal heading text. Nest with the three-character separator ` > ` (space, greater-than, space). Heading text is preserved verbatim — slashes, colons, hyphens, and other punctuation inside a heading remain literal. |
| `<file>:<line>` or `<file>:<line-start>-<line-end>` | `cafleet/src/cafleet/cli/main.py:142` | The target is a specific line (or range) in a source file, test file, or the design doc itself. Used for source-anchored Copilot inline review and for source-file `COMMENT` markers added during code review. |
| `doc` | `doc` | The target is the design document as a whole (e.g., Reviewer signalling overall approval, Drafter signalling the full draft is ready). |

**Pointer-marker pairing rule.** When a verb's spec requires a paired `COMMENT(role)` marker (`blocked` / `escalating`, also Director arbitration replies and `COMMENT(copilot)` placements), the marker MUST live at the SAME pointer as the cafleet body:

| Pointer | Canonical marker placement |
|:--|:--|
| `paragraph-<HeadingPath>` | Inline within that heading's section. |
| `<file>:<line>` | At that exact line in the file (immediately above or on `<line>` per the file's native comment syntax). |
| `doc` | Doc-top — directly under the metadata block (`Status:` / `Progress:` / `Last Updated:`), before the first heading. |

The ` > ` separator avoids the collision that would arise if `/` were used as a nesting separator (heading text in real-world design docs frequently contains `/`, e.g. `Step 2: Update docs/spec/cli-options.md`). ` > ` is unambiguous, ASCII-safe, and shell-safe inside double-quoted `--text` arguments.

### Message Format

Every `cafleet message send --text` body, when used to coordinate within a `cafleet-design-doc-create` skill team, MUST match:

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
- `complete (doc) — 12 issues`
- `addressed (doc)`
- `blocked (paragraph-Specification > Retry Strategy)`
- `escalating (paragraph-Implementation > Step 3)`
- `approved (doc)`

### COMMENT(role) Marker

Inline marker placed in the design document (or, for source-anchored Copilot inline review, in the source file at `file:line`).

```
COMMENT(<role>): <substantive content>
```

Roles:

| Role | Who writes it | When |
|:--|:--|:--|
| `claude` | The Director acting as user-mediator (existing convention from the `cafleet-design-doc-interview` skill) | Carries user-derived clarifications. Existing usage unchanged. |
| `director` | The Director | Spec resolution notes, Director judgments, ambiguity arbitration, design-doc-anchored Copilot review (see *Copilot Routing* below). |
| `drafter` | The Drafter | Spec ambiguities the Drafter cannot resolve and needs Director input on. |
| `reviewer` | The Reviewer | Review findings, tagged with the existing `[COMPLIANCE]` / `[GAP]` / `[UNCLEAR]` / `[INCORRECT]` / `[IMPROVEMENT]` taxonomy inside the marker body. |
| `copilot` | The Director on Copilot's behalf (Copilot does not edit files) | One marker per source-anchored inline review item, written into the **source file** at `<file>:<line>`. Design-doc-anchored Copilot lines route through `COMMENT(director)` instead — see *Copilot Routing*. |

Rules:

- One marker per logical issue. Do not bundle.
- Body must be actionable — state the issue and what should change.
- Reviewer markers carry one of the 5 review tags inside the body: `COMMENT(reviewer): [GAP] Step 4 lacks a Phase D entry.`
- Markers are split into two classes — *issue* and *status*. Only the *issue* class enters the doc.

### Issue Markers vs Status Markers (split)

`COMMENT(role)` markers carry **issue and feedback content only**. Status updates ("ready", "complete") stay as pure cafleet messages with verb + pointer; they do NOT add a marker to the design doc.

| Class | Example | Lifecycle |
|:--|:--|:--|
| Issue | `COMMENT(reviewer): [GAP] Specification omits the retry budget.` | Persists in the doc until resolved. The resolver removes the marker as part of the fix. Per `skills/cafleet-design-doc/guidelines.md` § *Completeness Check*, the doc cannot reach `Status: Approved` while any `COMMENT(` marker remains. |
| Status | (none — never enters the doc) | Lives only in `cafleet message send` text. |

This keeps the design doc clean: at any moment, the markers in the doc reflect *outstanding work*, never historical chatter.

### Copilot Routing

| Anchor | Where the marker lives | cafleet message | Resolver |
|:--|:--|:--|:--|
| Design doc (e.g. `design-docs/foo/design-doc.md:42`) | `COMMENT(director): <body>` inline at the affected paragraph in the design doc | (no cafleet route — Director resolves directly per the existing rule) | Director applies the spec change and removes the marker. **No self-note cafleet message is sent** — the git commit and marker removal are sufficient audit trail. |

### Anchorless Status

A member may need to communicate something that does not point at any heading, file, or doc — e.g. "I crashed and restarted," "still working, no progress yet," a generic ping ack.

- These ride as a pure cafleet message with a freeform short phrase. The set is **NOT a fixed canonical list** — members may use whatever short phrase fits ("restarted", "still working", "ack", "noted", etc.).
- They MUST NOT match the `<verb> (<pointer>)` schema (no parentheses) and the Director MUST treat them as informational, not as work signals.
- They do NOT add markers to the design doc.

If a member finds themselves needing to send anchorless status updates frequently, that is a stall signal — the Director's stall-response ladder (per `skills/cafleet/SKILL.md` and the existing role files) still applies.

### Finalize-Time Cleanup

When the design doc moves to `Status: Approved` (Step 6):

1. Issue markers (`COMMENT(role)` for `reviewer`, `director`, `drafter`, `claude`) MUST already be resolved per `skills/cafleet-design-doc/guidelines.md` § *Completeness Check* — the existing rule "No `COMMENT(` markers remain" stays.
2. Status markers do not exist in the design doc by construction (split), so there is nothing to strip.
3. `COMMENT(copilot)` markers in source files are removed by the routed member as part of each fix commit; finalize-time validation only needs to confirm the design doc is marker-free.

The audit trail of every status hop lives in the cafleet message log (admin WebUI timeline) and in git history. The design document itself reads as the current state, not an archaeological dig.

### Director Per-File Detail Recovery

Members do not ship file lists in cafleet bodies (the verb + pointer schema carries only a poke). The Director recovers per-file detail directly via git when a commit message needs it: `git status` for unstaged/staged file lists, `git diff --stat <base>..HEAD` for cumulative scope, `git log <base>..HEAD --name-only` for file-touch history, `git diff <base>..HEAD -- <pattern>` for content.

### Skill-specific overrides

- **Clarification Exemption**: Director-to-Drafter messages during the **Step 2 clarification phase** are exempt from the verb + pointer schema. At clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying), so the Director's "User answers: ..." relay rides as a free-form multi-line cafleet body. From Step 3 onward (once the initial draft exists) every message falls back under the schema.

## Architecture

The Director is the root agent of a CAFleet fleet — bootstrapped automatically by `cafleet fleet create` (no separate `cafleet agent register` call) — and spawns both the Drafter and Reviewer via `cafleet member create`. All coordination goes through the persistent message queue — every message is auditable via the admin WebUI.

```
User
 +-- Director (main Claude -- cafleet fleet create, cafleet member create, orchestrates cycle)
      +-- Drafter (member agent -- spawned in tmux pane; writes the design document)
      +-- Reviewer (member agent -- spawned in tmux pane; critically reviews the draft)
```

- **Director ↔ User**: `AskUserQuestion` (clarification relay, draft presentation, feedback collection)
- **Director ↔ Drafter**: `cafleet message send` (questions relay, user answers, reviewer feedback, drafting instructions)
- **Director ↔ Reviewer**: `cafleet message send` (draft review requests, review feedback)
- Members receive messages via a push notification: the broker keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into the member's pane via `tmux.send_inline_preview` whenever a `cafleet message send` is persisted. The recipient processes the preview as a fresh user-turn input — no `cafleet message poll` invocation is in the auto-fire path; to fetch the full body, the recipient calls `cafleet message poll` themselves. `--fleet-id` is global (before the subcommand); `--agent-id` is per-subcommand (after the subcommand name).

## Prerequisites

The Director MUST be running inside a tmux session (required by `cafleet member create`). Verify by running `cafleet doctor` before spawning anyone — it reports the tmux session/window/pane identifiers and exits non-zero with a clear message when the environment is not ready. If `cafleet doctor` reports a problem, abort and surface its message to the user. Do NOT invoke `tmux display-message`, `printenv TMUX`, or any other raw tmux/env probe — `cafleet doctor` is the only supported environment check (see `skills/cafleet/SKILL.md` § *use cafleet primitives only*).

## Primitive Mapping

| Agent Teams primitive | CAFleet equivalent |
|---|---|
| `TeamCreate(name="create-{slug}")` | CAFleet fleet created via `cafleet fleet create` — it bootstraps the fleet + root Director + placement + Administrator in one transaction (no separate `cafleet agent register` call needed for the Director) |
| `Agent(team_name=..., subagent_type=...)` | `cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> --name "..." --description "..." -- "<prompt>"` |
| `SendMessage(to="Drafter")` | `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <drafter-agent-id> --text "..."` |
| `SendMessage(to="Director")` (from member) | `cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> --to <director-agent-id> --text "..."` |
| `cafleet-agent-team-supervision` `/loop` | Load the `cafleet-agent-team-monitoring` skill (mechanism + `/loop`) and the `cafleet-agent-team-supervision` skill (governance), then run `/loop` from the `cafleet-agent-team-monitoring` skill |
| `TeamDelete` | `cafleet --fleet-id <fleet-id> member delete --agent-id <director-agent-id> --member-id <member-agent-id>` for each member, then `cafleet fleet delete <fleet-id>` (soft-deletes the fleet, deregisters the root Director + Administrator + any surviving members in one transaction). The root Director cannot be deregistered via `cafleet agent deregister` — `fleet delete` is the only supported teardown. |
| Auto message delivery | Push notification keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into member's tmux pane via `tmux.send_inline_preview` |

## Process

### Step 0: Path Resolution & Resume Detection (Director)

**Path resolution** (before resume detection):

Load the `cafleet-base-dir` skill for the no-bypass write protocol and `<unset>` sentinel contract. Then canonicalize `$ARGUMENTS` and resolve the task-scoped BASE:

- **Relative input** — accept any of: `0000060-foo`, `0000060-foo/design-doc.md`, `design-docs/0000060-foo`, `design-docs/0000060-foo/design-doc.md`. Canonicalize to `design-docs/<slug>` by: (1) stripping the trailing `/design-doc.md` if present; (2) stripping the leading `design-docs/` if present; (3) prepending `design-docs/`. The skill's Step 0 does NOT perform this stripping (per the `cafleet-base-dir` skill § *Consumer contract*) — canonicalize first, then run the skill's **Step 0 (task-scope resolution)** with the relpath `design-docs/<slug>`.

- **Absolute path** (e.g. `/abs/path/to/design-docs/0000060-skill-task-scoped-base-dir/design-doc.md`): Step 0 accepts only the task-folder path, not a child file. Strip the trailing `/design-doc.md` if present so the absolute path identifies the task folder, then run Step 0 with that absolute task-folder path. Step 0 accepts the absolute path if it lies strictly under the inferred repo root; otherwise it yields the `<unset>` sentinel.

Branch on Step 0's outcome: when it **resolves**, set `${BASE}` to the resolved task folder and `${DOC_PATH} = ${BASE}/design-doc.md` (the task folder IS the design-doc directory; no further `${BASE}/design-docs/...` concatenation). When it yields **`<unset>`** (absolute `$ARGUMENTS` outside the repo root, or equal to the repo root), set `${DOC_PATH}` to the **canonicalized** absolute task-folder path with `/design-doc.md` appended (unless `$ARGUMENTS` already names `design-doc.md`, in which case use it verbatim) so the Drafter receives a writable doc file path rather than a directory, and set `${BASE}` to the `<unset>` sentinel so audit-file writes guard-skip per the `cafleet-base-dir` skill § *The `<unset>` sentinel*.

Pass `${DOC_PATH}` to the Drafter as OUTPUT PATH in the spawn prompt. The audit-file path `${BASE}/prompts/<role>-<UTC-compact>.md` is naturally task-scoped — it lives under `<task-folder>/prompts/`, not under the repo root.

**Resume detection** (using resolved `${DOC_PATH}`):

1. **File does not exist** → Fresh creation (proceed to Step 1 as normal).
2. **File exists** → Check for `COMMENT(claude)` markers:
   - Use Grep to search for `COMMENT(claude)` in the file. The grep is tightened to the `claude` role because user-derived clarifications are the only marker class that warrants resume-mode. Stale `COMMENT(reviewer)` / `COMMENT(director)` / `COMMENT(programmer)` markers from other workflows MUST NOT be misclassified as interview-resume. Note: the `cafleet-design-doc-execute` skill also writes transient `COMMENT(claude): <choice>` markers for user arbitration (e.g., test-framework selection in `Phase 1`), but those are short-lived and removed by the routed member as part of the fix; under normal flow no `COMMENT(claude)` survives an in-progress execute run. If the user invokes the `cafleet-design-doc-create` skill against a half-finished execute doc that happens to carry a transient `COMMENT(claude)`, treating it as resume-mode is acceptable — the Drafter will read and resolve the marker the same way it handles interview clarifications.

   - **`COMMENT(claude)` markers found** → This is **resume mode**. Proceed to Step 1 with the resume-specific Drafter spawn prompt. Set an internal flag `SKIP_CLARIFICATION=true` so Step 2 (clarification) is skipped.
   - **No `COMMENT(claude)` markers found** → Inform the user: "No `COMMENT(claude)` markers found in the existing document." Use `AskUserQuestion` with two options:
     - **"Run quality review"**: Set internal flags `SKIP_CLARIFICATION=true` and `QUALITY_REVIEW_ONLY=true`. Skip Step 2 entirely and enter Step 3 by immediately routing the existing `${DOC_PATH}` to the Reviewer via `cafleet message send` (no new draft is produced; the Drafter is only involved later if the Reviewer requests revisions).
     - **"Start fresh"**: Treat as new creation, ignoring the existing file. Ensure `SKIP_CLARIFICATION` and `QUALITY_REVIEW_ONLY` are unset, then proceed to Step 1 as normal.

### Step 1: Register & Spawn Members (Director)

Load the `cafleet` skill, the `cafleet-agent-team-monitoring` skill, and the `cafleet-agent-team-supervision` skill (in that order — monitoring is the foundation layer, supervision the governance layer that depends on it).

#### 1a. Establish a CAFleet fleet and capture the root Director's `agent_id`

`cafleet fleet create` (which must be run inside a tmux session) atomically creates the fleet and registers a root Director bound to the current tmux pane — there is no separate `cafleet agent register` step for the Director. Use `--json` so both IDs are machine-parseable:

```bash
cafleet fleet create --label "design-doc-create-{slug}" --json
# → {
#     "fleet_id": "550e8400-e29b-41d4-a716-446655440000",
#     "label": "design-doc-create-{slug}",
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

Capture `fleet_id` and `director.agent_id` from the JSON response. Substitute them for `<fleet-id>` and `<director-agent-id>` in every subsequent command. **Do not store them in shell variables** — `permissions.allow` matches command strings literally, so every command must carry the literal UUIDs.

If you already have a running fleet (e.g. an outer orchestration), reuse its `fleet_id` and its root Director's `agent_id` instead of creating a new fleet. Do **not** attempt to register a second Director with `cafleet agent register --name Director` — the root Director from `fleet create` is the team lead; a second registration would just create an unrelated agent with no placement row.

#### 1b. Start the monitoring `/loop`

BEFORE spawning any member, use the `cafleet-agent-team-monitoring` skill's `/loop` Prompt Template and start a `/loop` monitor at the 1-minute interval using the literal `<fleet-id>` and `<director-agent-id>` UUIDs. The loop must stay active from the first `member create` until Step 6's shutdown cleanup. Supervision obligations (Authorization-Scope Guard, idle semantics, etc.) come from the `cafleet-agent-team-supervision` skill, which loads the `cafleet-agent-team-monitoring` skill as a hard prerequisite.

#### 1c. Locate role definitions (path-by-reference)

The Director references each role definition by **absolute path** in the spawn prompt — the spawned member opens its role doc with `Read` at startup. Do NOT inline the role content. Resolve the absolute paths for:

- `<abs path to this skill>/roles/drafter.md`
- `<abs path to this skill>/roles/reviewer.md`

Substitute these absolute paths into the spawn prompts below.

> **Why path-by-reference (and not inline-verbatim)**: cafleet `member create` passes the prompt to `tmux split-window` as a single positional argument. The cumulative caller-shell + cafleet-argv + tmux-argv budget exhausts well below `ARG_MAX` and surfaces as `command too long` once the shell-quoted prompt grows past a few KB. The role file is typically large enough that inlining it exceeds the limit. The member loads the role file via `Read` on its first turn. See the `cafleet` skill's `reference/director.md` reference file § *Spawn prompt size limit* for the canonical write-up.
>
> **Spawn-prompt audit file**: every spawn in this skill writes the rendered prompt to `${BASE}/prompts/<role>-<UTC-compact>.md` BEFORE invoking `cafleet member create --prompt-file <abs path>` (see Step 1d / 1e for the per-role flow). The pre-spawn file IS both the CLI input AND the permanent audit artifact — there is no second post-spawn re-render write. See the `cafleet-base-dir` skill § *No-bypass write protocol* and the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the contract, including the `${BASE} == <unset>` guarded-skip + inline-fallback branch.

#### 1d. Spawn the Drafter

**Drafter spawn prompt (normal mode):**

The spawn prompt's identity block uses `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders — cafleet `member create` runs `str.format()` over the entire prompt and substitutes these from the new member's allocated `agent_id`, the fleet ID, and the spawning Director's `agent_id`. The `[INSERT …]` markers in the prompt (e.g. `[INSERT DOC PATH]`, `[INSERT USER'S ORIGINAL REQUEST]`, `[INSERT abs path to roles/drafter.md]`) are NOT format placeholders — the Director substitutes them in shell before calling `member create`. See the Template safety note under `Member Create` in `skills/cafleet/reference/director.md` — any literal `{` / `}` you embed in a custom prompt must be doubled (`{{` / `}}`), and prompts MUST stay under ~2 KB (see Step 1c above on path-by-reference).

```
You are the Drafter in a design document creation team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/drafter.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
OUTPUT PATH: [INSERT DOC PATH]

The user's request: [INSERT USER'S ORIGINAL REQUEST]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

IMPORTANT: You MUST ask clarifying questions BEFORE writing any design document file.
Send your questions to the Director who will relay them to the user.
Start by reading the target codebase for context, then send your clarifying questions.
Do NOT create any design document file until you have received answers.
```

**Drafter spawn prompt (resume mode):**

Use this instead when Step 0 detected resume mode:

```
You are the Drafter in a design document creation team (CAFleet-native, RESUME MODE).

ROLE DEFINITION: Open [INSERT abs path to roles/drafter.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Follow the Resume Mode section in particular. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

This is a RESUME run. The document contains COMMENT markers from a previous
interview. Follow the Resume Mode instructions in your role definition.
Do NOT ask clarifying questions — the COMMENTs contain the needed information.
Start by reading the design document.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact — the CLI's `str.format()` pass resolves them at member-create time using the newly-allocated `agent_id`.
2. **Write the rendered text** to `${BASE}/prompts/drafter-<UTC-compact>.md` (both normal and resume modes — the resume-mode rendered prompt body indicates the COMMENT-resolution flow), where `${BASE}` is resolved by the `cafleet-base-dir` skill in Step 0 and `<UTC-compact>` is `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`. The UTC timestamp disambiguates audit artifacts between modes; the filename `<role>` segment stays `drafter` per the canonical convention in `skills/cafleet/reference/director.md` § *Member Create — Scratch and audit files*. Create the `${BASE}/prompts/` subdirectory on first write (Python: `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*: skip the file write, fall back to the inline positional `prompt_argv` form, and emit the `audit-disabled no BASE in spawn prompt` anchorless status once.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Drafter" \
     --description "Writes and revises the design document" \
     --prompt-file ${BASE}/prompts/drafter-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<drafter-agent-id>` in every subsequent command. The pre-spawn file at `${BASE}/prompts/drafter-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

#### 1e. Spawn the Reviewer

**Reviewer spawn prompt:**

```
You are the Reviewer in a design document creation team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/reviewer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for communication with the Director
- the `cafleet-design-doc` skill — for template and guidelines

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]
DESIGN DOCUMENT: [INSERT DOC PATH]

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "your report"
- When you see cafleet message poll output with a message from the Director, act on those instructions.

Wait for the Director to assign a document for review (cafleet body: `ready (doc)`). When you receive that message, the `doc` pointer refers to the DESIGN DOCUMENT path above — read that file and provide specific, actionable feedback per the role definition.
```

Spawn with the two-step (render to file, then `--prompt-file`) pattern:

1. **Render the prompt locally** with all `[INSERT …]` markers substituted. Leave `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders intact.
2. **Write the rendered text** to `${BASE}/prompts/reviewer-<UTC-compact>.md` (`${BASE}` resolved by the `cafleet-base-dir` skill in Step 0; `<UTC-compact>` = `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`). Same-second collision: append `_2`, `_3`, … until the name is unique — never overwrite. If `${BASE}` is the sentinel `<unset>`, follow the `<unset>` fallback in the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*.
3. **Spawn with `--prompt-file`** pointing at the rendered file (use the absolute path):

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "Reviewer" \
     --description "Critically reviews drafts for rule compliance and quality" \
     --prompt-file ${BASE}/prompts/reviewer-<UTC-compact>.md
   ```

   Parse `agent_id` from the JSON response and substitute it for `<reviewer-agent-id>` in every subsequent command. The pre-spawn file at `${BASE}/prompts/reviewer-<UTC-compact>.md` IS the audit artifact — no second post-spawn re-render is performed.

#### 1f. Verify members are live

```bash
cafleet --fleet-id <fleet-id> member list --agent-id <director-agent-id>
```

Both members must show `status: active` with a non-null `pane_id`. If either is missing or pending, retry the spawn before proceeding.

### Step 2: Clarification Phase (Director)

**Skip this step entirely when `SKIP_CLARIFICATION=true`** (set by Step 0 in resume mode or quality-review-only mode). Resume mode: the COMMENT markers serve as the clarification and the Drafter already has all the information needed. Quality-review-only mode: the Drafter is not producing a new draft at all — proceed directly to Step 3 by routing the existing `${DOC_PATH}` to the Reviewer.

> **Clarification Exemption**: Director-to-Drafter messages during this step are exempt from the verb + pointer schema documented in [the Coordination Protocol section above](#coordination-protocol). At clarification time the design doc does not yet exist (the Drafter is forbidden from creating any file before clarifying), so there is no in-doc target for `COMMENT(claude)` markers — the user-answer relay rides as a free-form multi-line cafleet body. From Step 3 onward (once the initial draft exists) every message falls back under the schema.

1. Wait for the Drafter's clarifying questions. The monitoring `/loop` and periodic `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>` will surface the Drafter's message once it arrives.
2. `cafleet --fleet-id <fleet-id> message ack --agent-id <director-agent-id> --task-id <task-id>` each received message after reading it.
3. Relay the questions to the user via `AskUserQuestion`. If the number of questions exceeds the per-call limit of `AskUserQuestion`, split them into multiple sequential calls to relay all questions without omission.
4. Relay the user's answers back to the Drafter (free-form, per the Clarification Exemption above):
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "User answers: ..."
   ```
5. **Gate check**: If the Drafter produces a draft without prior questions, reject it and instruct them to ask first (also free-form, per the Clarification Exemption):
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "Stop — you must send clarifying questions before drafting. Discard the draft and send questions first."
   ```
   A focused confirmation round counts as valid clarification.

### Step 3: Internal Quality Loop (Director)

Enter this step after the Drafter reports `complete (doc)`, **or immediately** when `QUALITY_REVIEW_ONLY=true` (the existing `${DOC_PATH}` is treated as the "completed draft" — no waiting for a Drafter report):

1. **Route to Reviewer**. The Reviewer reads `${DOC_PATH}` directly; no path needs to be embedded in the cafleet body.
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <reviewer-agent-id> --text "ready (doc)"
   ```
2. **Wait** for the Reviewer's response via `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>`. Round-1 fresh review arrives as `complete (doc) — N issues`; approval arrives as `approved (doc)`. Each finding is recorded as a `COMMENT(reviewer): [TAG] <body>` marker inline in the design doc — the Director does NOT relay the finding text in cafleet.
3. **On feedback**: Route the Drafter to address the markers in-doc:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "ready (doc)"
   ```
4. Wait for the Drafter's `addressed (doc)` reply (revisions resolve the `COMMENT(reviewer)` markers), then loop back to step 1 (re-route to Reviewer with `ready (doc)`).
5. Repeat until the Reviewer explicitly signals `approved (doc)`.
6. **Iteration limit**: Aim for 2–3 rounds. If not converging, escalate to the user: summarize the remaining issues at a high level (read directly from the surviving `COMMENT(reviewer)` markers in the doc) and use `AskUserQuestion` to ask whether to continue iterating or abort. Do not proceed to Step 4 until the Reviewer has approved.

### Step 4: Present to User (Director)

Only after the Reviewer explicitly approves, present a summary (including file path) and use `AskUserQuestion`:

| Option | Label | Description | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with the current result | Proceed to finalization (Step 6) |
| 2 | **Scan for COMMENT markers** | Immediately scan the document for `COMMENT(name): feedback` markers and process them | Scan immediately and process markers (see Step 5) |
| 3 | *(Other — built-in)* | *(Free text input)* | Interpret user intent (see Step 5) |

See [roles/director.md](roles/director.md) for user interaction rules (COMMENT handling, intent judgment, abort detection).

### Step 5: User Feedback Loop (Director)

Process the user's selection:

- **"Scan for COMMENT markers"**:
  1. **Immediately** scan the document with Grep for `COMMENT(` markers — do NOT wait for the user to confirm they are done editing. The selection itself is the signal to scan now.
  2. **If markers are found**: Route the Drafter to read and resolve the markers in-doc with `ready (doc)` — no marker content is quoted in the cafleet body:
     ```bash
     cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
       --to <drafter-agent-id> --text "ready (doc)"
     ```
     After the Drafter replies `addressed (doc)` and removes the markers, verify with Grep that no `COMMENT(` markers remain. Then re-enter the quality loop (Step 3) and re-present (Step 4).
  3. **If no markers are found**: Explain the COMMENT marker convention to the user — markers follow the pattern `# COMMENT(username): feedback` placed directly in the design document file. Show the file path so the user can edit it. Then re-prompt with the same three-option pattern (Approve / Scan for COMMENT markers / Other).

- **"Other" (free text)**: Use LLM reasoning — not keyword matching — to distinguish between:
  - **Abort intent** (user wants to stop or cancel the process): Trigger the Abort Flow — follow the Shutdown Protocol (Step 6) without Drafter finalization.
  - **Non-abort intent** (user providing verbal feedback or asking a question): Explain that feedback should be provided via COMMENT markers in the design document, then re-prompt with the same three-option pattern.

No round limit — loop continues until approved or aborted.

### Step 6: Finalize & Clean Up (Director)

1. Instruct the Drafter to finalize. The Drafter's role definition spells out the finalize checklist (set Status to Approved, refresh Last Updated, bump the Progress header field if present, verify implementation steps are actionable); the cafleet body is just the verb + pointer poke:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "ready (doc)"
   ```
   Wait for the Drafter's `addressed (doc)` confirmation.

2. Run the canonical teardown per the `cafleet` skill § *Shutdown Protocol*:
   1. `CronDelete` the `/loop` monitor (cron ID recorded at Step 1b).
   2. `cafleet member delete` for each member (Drafter, then Reviewer). Each call blocks until the pane is gone (15 s timeout); on exit 2 follow the `member capture` + `send-input` recovery in the canonical protocol, or rerun with `--force`.
   3. `cafleet member list` — the team's roster MUST be empty before continuing.
   4. `cafleet fleet delete <fleet-id>` (positional, no `--fleet-id` flag).
   5. `cafleet fleet list` — the fleet MUST not appear (soft-deleted fleets are hidden).

The fleet row is soft-deleted and `tasks` are preserved so the message trail remains inspectable in the admin WebUI.
