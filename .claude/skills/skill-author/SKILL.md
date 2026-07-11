---
name: skill-author
description: >
  Teach an author how to integrate the CAFleet-orchestrated team pattern into a
  new skill they are writing. Auto-load when the author intends to create a new
  CAFleet-orchestrated skill, write a skill that spawns cafleet members, add a
  Director/Member team skill, design a multi-agent broker-coordinated skill,
  build a CAFleet-team-driven skill, or write a skill that uses
  `cafleet member create`. This skill is project-local to the cafleet repo and
  is fully self-contained — no `cafleet` `reference/base-dir.md` cross-reference
  is required to follow the guide.
---

# Skill Author — Integrating the CAFleet-Orchestrated Pattern

You are about to write a new skill that drives a Director and one or more spawned members through the CAFleet message broker. This guide walks you through every sub-system you need to wire up, explains why each step exists, and finishes with a worked example you can read end-to-end. It is a teaching document — not a paste-this template — because the canonical rules drift quickly and skills that copy a prefab spawn prompt rot the fastest.

Read this whole document before you start writing your `SKILL.md`. The rules are short individually but the failure modes when any of them is missed are loud, and most of them have already bitten earlier authors.

---

## 1. What "CAFleet-orchestrated" means

A CAFleet-orchestrated skill is one where the Director (the main Claude session running your slash command) bootstraps a fresh CAFleet fleet, spawns one or more **members** as separate `claude` (or `codex`) processes inside dedicated tmux panes, and coordinates the work through the CAFleet message broker (`cafleet message send` / `cafleet message poll` / `cafleet message ack`). Members are real, isolated coding-agent processes — not in-process subagents — and the broker delivers each message as a 2-line keystroke preview into the recipient's tmux pane.

The shape always looks like this:

```
User
 +-- Director (main Claude — runs cafleet fleet create / member create; coordinates via the broker)
      +-- member-1 (claude pane)
      +-- member-2 (claude pane)
      +-- ...
```

Use this pattern when:

- The work needs **parallelism that the harness cannot provide on its own** — multiple members drafting in parallel, multiple researchers investigating sub-topics, multiple verifiers running on different slide ranges.
- The work needs **role specialization** — Director / Drafter / Reviewer / Programmer / Tester are roles that justify separate processes with separate role files.
- The work needs **persistent inter-member memory** — the broker stores every message in SQLite and the audit-file path under `${BASE}/prompts/` is a permanent record.

Do **not** use this pattern when:

- The work fits inside one `Agent` tool subagent run. A subagent is cheaper and faster than a full CAFleet team.
- The work is a single-shot transformation (file edit, search, summarize). A normal slash command running inside the Director's session is enough.
- You only need the harness `TaskList` for tracking — you do not need cross-pane coordination.

If you are unsure: write the simpler version first. The CAFleet team pattern is overkill for most tasks and the orchestration overhead (`cafleet doctor`, `cafleet fleet create`, `cafleet member create`, `cafleet monitor`, `member delete`, `fleet delete`) costs the user real seconds and real cognitive load.

---

## 2. The five-part integration checklist

Every CAFleet-orchestrated skill must wire up these five sub-systems, in this order, in its `SKILL.md`. The first three are setup; the fourth is the per-member spawn pattern; the fifth is teardown. Skip any one of them and the skill will fail silently in production.

### 2.1 Resolve the task-scoped BASE

Before any other work, the Director resolves the task-scoped output directory by following the `cafleet` skill's `reference/base-dir.md` task-scope resolution procedure with a `TASK_NAME` derived from the skill's per-task convention. The procedure uses only `git rev-parse --show-toplevel` (via Bash) and writes nothing at resolution time — there is no `cafleet` CLI subcommand for it.

`<task-relpath>` is a path under the inferred repo root that describes the per-task folder. The two recognized buckets are:

- `researches/<topic-slug>` — for research-style skills (one folder per research run).
- `design-docs/<NNNNNNN>-<slug>` — for design-doc-style skills (one folder per design document, with a 7-digit zero-padded number prefix per `.claude/rules/design-doc-numbering.md`).

The procedure:

1. Walks up from CWD (`git rev-parse --show-toplevel`) to infer the repo root.
2. Joins `<task-relpath>` against the repo root and resolves it to the absolute task folder.
3. Yields `base = <abs task-folder>`; the folder is created lazily on the first consumer write.

Use the resolved `base` as `${BASE}` for the rest of the run. **Every** scratch / audit / figure / spawn-prompt-render write the skill performs MUST live under `${BASE}` — never `/tmp`, never the repo root.

The procedure's positional branch also accepts an absolute path. If the path lies strictly under the inferred repo root, it is used verbatim as the task folder — the resolver does NOT walk ancestors or match skill-specific bucket patterns. If the path lies outside the repo root (or equals the repo root), the resolver yields the literal sentinel `<unset>` for `${BASE}`. **Consumer-strips contract**: because the resolver does not fold child paths, each consuming skill MUST canonicalize its argument to the actual task-folder path (relative or absolute) BEFORE resolving. For a **relative** argument: strip trailing filenames like `/design-doc.md`, strip leading bucket prefixes like `design-docs/`, then prepend its own bucket. For an **absolute** argument: apply only the trailing-filename strip — it is used verbatim as the task folder when it lies strictly under the repo root (no bucket prepend), otherwise it yields `<unset>`. When `${BASE}` is `<unset>`, the skill MUST guard every BASE-derived write with an explicit `${BASE} != <unset>` check, omit the `BASE:` line from any spawn prompt entirely, and never fall back to `/tmp`. The standardized loud-error message is `Error: BASE is <unset>; refusing to fall back to /tmp`.

When CWD has no `.git` ancestor (typical when CWD is `$HOME` or under `$HOME/.claude`) AND a `TASK_NAME` is supplied, the resolution fails with `cannot resolve task-scope base-dir: no .git ancestor found from CWD <cwd>. cd to the repo root and retry.` — surface this error to the user and stop.

### 2.2 Bootstrap a CAFleet fleet

The Director creates the fleet inside a tmux pane:

```bash
cafleet fleet create --name "<fleet-name>" --json
```

The CLI atomically (1) creates a `fleets` row and (2) registers a root Director bound to the current tmux pane. Capture both `fleet_id` and `director.member_id` from the JSON response and substitute them as **literal id strings** into every subsequent `cafleet ...` call.

Never store these IDs in shell variables (`export FLEET=...`). The Claude Code harness's `permissions.allow` matches Bash invocations as literal command strings; an exported shell variable you reference yourself breaks the literal match and forces per-invocation permission prompts that interrupt the agent loop.

If the user is not inside a tmux session, `cafleet fleet create` exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing. Surface this and stop — do NOT try to start a tmux session yourself.

### 2.3 Spawn the monitoring member first

CAFleet members do not auto-poll. The broker delivers a 2-line inline preview into the recipient's pane via `tmux.send_inline_preview` keystroke; that preview is the trigger that wakes the recipient. If the keystroke is missed (pane buffered, recipient mid-Bash, etc.), the message just sits in `INPUT_REQUIRED` until the recipient runs `cafleet message poll` themselves.

Every CAFleet-orchestrated skill runs a dedicated **monitoring member** — it is a session-level requirement, not a per-skill choice. The Director NEVER runs `cafleet monitor start` itself. Instead, the **first** `cafleet member create` in the fleet is a dedicated monitoring member spawned with `--role monitor --model sonnet`; it runs `cafleet monitor start` as a background task in its own pane and reports `ready: monitor live`, which **gates the first ordinary `member create`** (first-in). The monitor loop wakes **only** the monitoring member; on each wake the monitoring member captures the Director's pane, judges it active vs idle, and re-engages an idle Director on demand via `cafleet message send` (which persists an ACKable broker task **and** fires the hardened, `Esc`-safeguarded inline preview). The Director is never keystroked by the loop. The `cafleet` skill's `roles/monitor.md` documents the monitoring member's canonical spawn prompt and the first-in / first-out lifecycle (the heartbeat mechanism itself lives in `reference/supervision.md`); the heartbeat is identical on any backend (claude, codex, opencode).

The monitoring member runs one of two idle-nudge routines (both spawn the monitoring member; the difference is only *when* it nudges the idle Director):

- **Conditional idle-nudge (canonical).** The monitoring member re-engages the Director only when it can name what needs attention (un-acked inbox items, stalled members). Use this for skills whose Director makes forward progress from member replies — the conditional nudge surfaces real stalls without busy-waking the Director.
- **Unconditional idle-nudge (extended).** For skills whose Director must wake on a cadence that is NOT driven by member replies — e.g. polling an external service (CI, a code-review bot) that never keystrokes the Director's pane — the monitoring member nudges the idle Director unconditionally, granting it a re-poll turn even when the inbox is empty.

### 2.4 Spawn members with `cafleet member create --text-file <abs path>`

For each member you spawn, follow the **two-step render-to-file pattern**:

1. **Render the spawn prompt locally**. Substitute every `[INSERT …]` marker with the concrete value, and keep the four `{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}` identity placeholders as written — `cafleet member create` runs `str.format` over the prompt at spawn and renders them to literals (§ 3.2). Any literal brace in the prompt body must be doubled (`{{` / `}}`) to survive `.format()`; leave no other stray single braces. By the time you write the file, every `[INSERT …]` marker must be replaced with its concrete literal value.

2. **Write the rendered text** to `${BASE}/prompts/<role>-<UTC-compact>.md` where `<UTC-compact>` is `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")` (Python). Create `${BASE}/prompts/` on first write via `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`. On same-second collisions, append `_2`, `_3`, … to the filename until it is unique — never overwrite. **The pre-spawn file IS the audit artifact.** There is no second post-spawn re-render; the file is both the CLI input and the permanent record of what was spawned, carrying the four `{...}` identity placeholders pre-substitution (which is expected).

3. **Spawn with `--text-file`** pointing at the absolute path of the rendered file (the acting Director is auto-resolved from the fleet row — no identity flag):

   ```bash
   cafleet member create --fleet-id <fleet-id> \
     --name "<member-name>" \
     --description "<one-sentence purpose>" \
     --text-file ${BASE}/prompts/<role>-<UTC-compact>.md \
     --json
   ```

   Capture the printed `member_id` from the JSON response and substitute it for the member's id in every subsequent `cafleet ...` call **the Director** makes that targets it. (The member itself learns its own id from the literal `YOUR MEMBER ID:` line the CLI rendered into its prompt — § 3.2.)

#### Why the two-step pattern instead of inline `--text`?

`cafleet member create` accepts the spawn prompt either inline via `--text` or via `--text-file <abs path>`. The inline `--text` form passes the prompt to `tmux split-window` as a single positional argument; tmux fails with `command too long` once the shell-quoted prompt grows past a few KB, and cafleet rolls back the member registration. Real role files are typically 5–15 KB and the spawn prompt that includes them often pushes well past 20 KB. `--text-file` reads the file inside the cafleet process and writes the text to the new pane through a separate path that is not size-limited.

Use `--text-file` always. The inline `--text` form exists only as the documented fallback when `${BASE}` is `<unset>` and the audit-file write is impossible (see § 4).

#### Path-by-reference for role files

Do NOT inline a role definition (5–15 KB markdown file describing a member's accountability, communication protocol, role-specific workflow, escalation rules) into the spawn prompt. Instead, reference the role file by absolute path:

```
ROLE DEFINITION: Open <abs path to roles/<role>.md> with the Read tool BEFORE any other action.
```

The spawned member opens its role file with `Read` on its first turn. The role file lives in your skill's `roles/` directory and is stable, so this is safe. This pattern keeps the spawn prompt small (under the tmux limit), makes role updates take effect without a respawn, and concentrates role-specific accountability in a single canonical file rather than smearing it across the spawn prompt.

### 2.5 Tear down per the Shutdown Protocol

When the work is done, the Director MUST tear down in this exact order:

1. **Stop the monitoring member first.** There is no `monitor stop` command — message the monitoring member to stop its `cafleet monitor start` background task (the task-stop delivers SIGTERM/SIGINT, so the loop runs its `finally` and clears its runtime row), wait for its confirmation, then `cafleet member delete` the monitoring member **first**, before any ordinary member. A tick that keystrokes into a tearing-down pane races the delete path.
2. **`cafleet member delete --member-id <id>`** for every remaining (ordinary) member. This sends the backend exit keystroke to the member's pane and waits up to 15 s for the pane to disappear. Surviving member coding-agent processes are NOT auto-closed by `cafleet fleet delete` — call `member delete` per member.
3. **`cafleet fleet delete --fleet-id <fleet-id>`**. Soft-deletes the fleet (sets `deleted_at`), deregisters every active member in the fleet (root Director + remaining members), and physically deletes every associated `member_placements` row. Tasks are preserved. `fleet delete` makes any still-running loop self-terminate on its next tick, so step 1 is belt-and-suspenders.

Order matters. Stop the monitoring member's monitor and delete it before the ordinary members so a tick cannot keystroke into a tearing-down pane or race the delete path. If you call `fleet delete` before `member delete`, the member panes orphan (the `claude` process keeps running but has no broker to talk to).

---

## 3. Spawn-prompt anatomy

Every member spawn prompt follows the same skeleton. Read this section as the canonical anatomy — it explains every section, why it is there, and the substitution rules.

```
You are <role> in a <skill> team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/<role>.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the cafleet skill — for the broker primitives and bash-via-Director routing
- <other skills as needed>

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet` skill's `reference/base-dir.md`]
CODING AGENT: {coding_agent}

<role-specific assignment text — e.g. CURRENT DATE, USER REQUEST, OUTPUT PATH, YOUR TASK ID>

<role-specific instructions — every IMPORTANT: line, poll-handling line, and start cue>
```

There is no `COMMUNICATION PROTOCOL` command-example block: the member learns the poll/send/ack command shapes from the `cafleet` skill and its role file, and takes its ids from the literal identity lines above.

### 3.1 The identity block

```
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: <abs task-folder path>
CODING AGENT: {coding_agent}
```

These lines are the member's grounding identity. The brace tokens are the CLI's `str.format` placeholders — `cafleet member create` renders each to a literal at spawn (§ 3.2), so the member reads e.g. `FLEET ID: 7` / `YOUR MEMBER ID: 11` and substitutes those integers into its `cafleet ...` commands. The `BASE:` line is the resolved task-scoped BASE the Director computed in § 2.1 — the member uses this verbatim and MUST NOT re-resolve BASE on its own (members that re-resolve risk drift from the Director's resolved BASE). The `CODING AGENT:` line names the member's backend so it can read its overlay (§ 8).

### 3.2 Identity via `str.format` substitution

`cafleet member create` runs `str.format` over the resolved spawn prompt, substituting exactly four placeholders to literals at spawn time:

- **`{fleet_id}`** — the spawned member's fleet id.
- **`{member_id}`** — the spawned member's **own** id, allocated by cafleet during the spawn (so the Director cannot know it at render time — this is exactly why the spawn prompt carries the placeholder rather than a Director-rendered literal; only the CLI can fill it).
- **`{director_member_id}`** — the spawned member's Director id.
- **`{coding_agent}`** — the resolved backend name (`claude` / `codex` / `opencode`).

An unknown placeholder raises a `UsageError` listing the four supported names; a malformed brace expression raises the "double literal braces" `UsageError` (both exit 2, with the just-registered member rolled back). **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`.

This substitution is the **sole** identity-delivery mechanism — no identity environment variable is injected into the pane (the only forwarded env var is `CAFLEET_DATABASE_URL`). The member takes the literal integers from its prompt's identity lines and passes them explicitly:

- a poll is `cafleet message poll --fleet-id <fleet-id> --member-id <my-member-id>`;
- a self-attributed send is `cafleet message send --fleet-id <fleet-id> --from-member-id <my-member-id> --to-member-id <director-member-id> --text "..."`.

A Director may *also* embed the literal `fleet_id` and `director_member_id` (which it knows at render time) directly instead of the placeholders; the member's own id always comes from the CLI-rendered `{member_id}`. The `cafleet` skill documents the convention for spawned members.

### 3.3 The `[INSERT ...]` render-time substitution rules

`[INSERT …]` markers are placeholders for **you** (the skill author writing the SKILL.md) to direct the Director on what to substitute when rendering. The Director performs every `[INSERT …]` substitution in step 1 of § 2.4 (local rendering) before writing the file. By the time the file lands at `${BASE}/prompts/<role>-<UTC-compact>.md`, no `[INSERT …]` marker should remain. The four `{...}` identity placeholders are NOT `[INSERT …]` markers — they are the CLI's `str.format` tokens, rendered at spawn time (§ 3.2), so leave them as written and leave no other stray single braces around them.

A common mistake is to leave a literal `[INSERT abs path to roles/<role>.md]` in the rendered file because the Director forgot to compute the absolute path. The member then opens a file that does not exist on its first turn. Spot-check the rendered files in `${BASE}/prompts/` before continuing.

### 3.4 The path-by-reference rule (one more time)

Spawn prompt body references the role file by absolute path. The role file lives in `<your skill dir>/roles/<role>.md`. Compute the absolute path once at Director startup and substitute it into every spawn prompt that needs it. Do NOT inline the role file content.

### 3.5 The `command too long` cliff and `--text-file`

`tmux split-window` accepts the spawn prompt as a single positional argument. Linux's `ARG_MAX` (the `execve()` argument-list size limit) and tmux's own command parser combine to fail with `command too long` once the shell-quoted prompt grows past a few KB. Even with role content not inlined, a prompt with multiple `[INSERT …]` substitutions + the identity block + role-specific assignment text often exceeds the limit.

`--text-file <abs path>` sidesteps this. cafleet reads the file inside its own process and writes the text to the new pane through a separate path that is not size-limited. **Use `--text-file` for every spawn.** The inline `--text` form is the documented fallback only for the `${BASE} == <unset>` case where the file write is impossible.

### 3.6 The `${BASE} == <unset>` skip semantics

When the base-dir resolution yields the `unset` outcome (absolute-path argument outside the repo root, or equal to the repo root itself), `${BASE}` is the literal sentinel string `<unset>`. The skill MUST:

- **Skip the audit-file write.** Do not try to write `<unset>/prompts/<role>-<UTC-compact>.md` — that is a literal path with a `<` in it, which most filesystems reject. The guard is `if BASE != "<unset>"`.
- **Omit the `BASE:` line from the spawn prompt.** The spawn prompt does NOT include the literal string `BASE: <unset>` — the line is dropped entirely. The member's existence-check naturally treats audit-file features as disabled.
- **Fall back to inline `--text`** for the `cafleet member create` call (the file-write path is gone, so the only way to get the prompt to cafleet is inline). Be aware that this risks the `command too long` failure mode for prompts above the tmux limit; surface that as a hard error to the user, not as a silent retry.
- **Loud-error on unguarded BASE-derivation.** If a code path under `${BASE} == <unset>` reaches an unguarded `Path(BASE) / …` computation, abort with the standardized error: `Error: BASE is <unset>; refusing to fall back to /tmp`.

The member, after spawn, emits a single CAFleet message back to the Director as a parens-free anchorless status:

```
audit-disabled no BASE in spawn prompt
```

The phrasing deliberately omits parentheses so the Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop.

---

## 4. Audit-file write protocol

Every spawn-prompt render is also the spawn-prompt audit artifact. The protocol:

1. **Path**: `${BASE}/prompts/<role>-<UTC-compact>.md` where `<UTC-compact>` is `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`. `<role>` matches the lowercased value of the `--name` flag passed to `cafleet member create` (e.g., `manager`, `scout-1`, `researcher-04`, `programmer`).

2. **Mkdir on first write**: `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`. Use `pathlib`, not `subprocess.run(["mkdir", ...])`. This is enforced by the no-bypass write protocol.

3. **Same-second collisions**: When two spawns happen in the same UTC second (rare but possible — a Director spawning multiple Researchers in a single tick), append `_2`, `_3`, … to the filename until it is unique. **Never overwrite.** The audit record must be permanent.

4. **The pre-spawn file IS the audit artifact**. There is no second post-spawn re-render. After `cafleet member create` succeeds, the file at `${BASE}/prompts/<role>-<UTC-compact>.md` is the permanent record of what was spawned and is never touched again. It carries the four `{...}` identity placeholders pre-substitution — that is expected; the CLI renders them at spawn.

5. **Every write under `${BASE}`**. Audit files, scratch notes, figure artifacts, intermediate working files — every output the skill produces lands under `${BASE}` or a consumer-supplied absolute path. **Never `/tmp`** unless `${BASE}` itself is `/tmp/claude-code` (which is a legitimate base-dir choice when the user picked it via `AskUserQuestion`).

6. **`${BASE} == <unset>` is a hard stop**, not a fallback. See § 3.6.

This protocol is the single most-violated rule in past CAFleet-orchestrated skills. Authors routinely forget the same-second collision rule and overwrite the previous member's audit file when spawning two members in a tick. Authors routinely forget the `<unset>` sentinel and crash with a Path-with-`<`-in-it error. Read the rules above before writing any file write code.

---

## 5. Coordination protocol summary

Inter-member communication uses the **verb + pointer** schema. This is the longest section in this guide because it is the most easily miswired surface.

### 5.1 The cafleet message body shape

Every cafleet message body looks like:

```
<verb> (<pointer>) [— <optional summary up to 80 codepoints, ≤ 3-item enumeration>]
```

The body MUST be short. Substantive content (rationale, evidence, file lists, test names, error stacks) lives as **inline `COMMENT(role)` markers** in the document being edited at the same pointer the cafleet body references. The body is the routing hop; the marker is the substance.

### 5.2 The canonical 6-verb list

There are exactly six verbs:

| Verb | Meaning | Sender | Recipient |
|:--|:--|:--|:--|
| `ready` | The pointer is ready for the recipient to act on (next step assigned, or feedback awaiting addressing). | Director | Member |
| `complete` | The pointer is done from the sender's side; the recipient should review or proceed. | Member | Director |
| `addressed` | The recipient has applied the requested fix at the pointer; the sender should re-verify. | Member | Director |
| `blocked` | The sender cannot proceed at the pointer because something is missing / ambiguous; an inline `COMMENT(role)` marker carries the explanation. | Member | Director |
| `escalating` | The sender suspects a defect outside their authority (test defect, spec ambiguity) and is handing off to the Director for arbitration. | Member | Director |
| `approved` | The Director gives final authorization for a milestone (typically `approved (doc)` for a finished design doc, or per-step approval). | Director | Member |

Do NOT invent new verbs. Do NOT use English synonyms ("done", "ack", "ok", "fixed"). The 6-verb list is the entire protocol; broker log parsers look for these literal words.

### 5.3 The pointer forms

Three pointer shapes:

| Pointer | When |
|:--|:--|
| `paragraph-<HeadingPath>` | A specific section of the document — e.g. `paragraph-Implementation > Step 5`, `paragraph-Specification > 3. Anchor schema`. Use the literal heading text from the document with `>` separators. |
| `<file>:<line>` | A specific file location — e.g. `cafleet/src/cafleet/broker/messaging.py:142`, `docs/concepts/overview.md:178`. |
| `doc` | The whole document; used for top-level milestones (`complete (doc)`, `approved (doc)`) and document-wide blocks (`blocked (doc) — test framework ambiguous`). |

### 5.4 The pointer-marker pairing rule

When a sender includes substantive content in a `COMMENT(role)` marker, the marker MUST live at the same pointer the cafleet body references. Examples:

- `blocked (paragraph-Implementation > Step 5)` + `COMMENT(programmer): test X expects Y but design doc says Z` at `paragraph-Implementation > Step 5` in the design doc.
- `ready (cafleet/src/cafleet/broker/messaging.py:142)` + `COMMENT(director): use pathlib.Path.mkdir(parents=True, exist_ok=True), not subprocess.run(["mkdir", ...])` at line 142 of the file.

The recipient reads the cafleet body, navigates to the pointer, reads the standing marker, applies the fix or arbitration, removes the marker, and replies with the next-step verb (`addressed (...)` for member-side fixes, `ready (...)` for director-side arbitration handoffs).

### 5.5 The role taxonomy

The marker role identifier in `COMMENT(<role>)` is one of: `claude` (user-derived clarifications baked into the doc), `director`, `programmer`, `tester`, `reviewer`, `verifier`, `analyzer`, `drafter`. New roles for new skills SHOULD be added to this list; do not abbreviate. The `claude` role is reserved for the `cafleet-design-doc` skill's interview-workflow user-derived clarifications and its execute-workflow test-framework arbitration; do not use it for arbitrary "Claude said this" content.

### 5.6 Anchorless status

A small handful of statuses do not pair with a pointer because the condition is global. These are emitted as parens-free anchorless strings:

- `audit-disabled no BASE in spawn prompt` — emitted by a member whose spawn prompt lacks the `BASE:` line entirely (the `${BASE} == <unset>` branch).
- (Other anchorless statuses are documented per-skill; do not invent new ones casually.)

The phrasing deliberately omits parentheses so a parser does not misinterpret it as a malformed `<verb> (<pointer>)` hop.

### 5.7 Acking messages

After acting on a polled message, the recipient MUST `cafleet message ack` it. Un-acked messages stay in `INPUT_REQUIRED` and re-surface on every subsequent `message poll` cycle, polluting the recipient's context with stale work.

```bash
cafleet message ack --fleet-id <fleet-id> --member-id <my-member-id> --message-id <message-id>
```

The `<message-id>` is the full id returned by `cafleet message poll --fleet-id <fleet-id> --member-id <my-member-id> --full --json`. The default text-mode poll output truncates the body; pass `--full` when you need the untruncated envelope.

---

## 6. Worked example — `summarize-pr`

This is a tiny end-to-end CAFleet-orchestrated skill called `summarize-pr` (single Director + a mandatory monitoring member + one ordinary member named Summarizer). The example uses fake `<slug>`, `<fleet-id>`, etc. and is read-only — it is illustrative, not a template you copy. Read it to understand how all five sub-systems fit together; then write your own skill from scratch.

### Skill purpose

The user invokes `/summarize-pr <pr-number>`. The Director:

1. Fetches the PR diff via `gh pr diff <pr-number>`.
2. Spawns the monitoring member first, then a Summarizer member to digest the diff, identify the top 3 risk areas, and write a 200-word summary to a file.
3. Reviews the summary, asks the user for approval, then tears down.

### Resolved task-relpath

The skill's task convention is `researches/pr-<pr-number>` (PR summaries are research-shaped — one folder per PR with the diff + summary inside).

```text
# Resolve task-scope BASE for researches/pr-1234 (cafleet reference/base-dir.md procedure, built-in tools):
#   git rev-parse --show-toplevel → /repo
#   task folder → /repo/researches/pr-1234  (auto-created)
```

`${BASE} = /repo/researches/pr-1234`. The Director writes the diff to `${BASE}/diff.patch` (a non-audit working file) and the summary will land at `${BASE}/summary.md` (also a working file, not under `prompts/`).

### Fleet bootstrap

```bash
cafleet fleet create --name "summarize-pr-1234" --json
# → {"fleet_id": 7, "director": {"member_id": 8}}
```

Substitute `7` and `8` literally into every subsequent call the Director makes.

### Supervision model

Like every CAFleet-orchestrated skill, `summarize-pr` spawns a dedicated monitoring member **first**, before the Summarizer (§ 2.3). It uses the canonical conditional-idle-nudge routine — the Director makes forward progress from the Summarizer's replies, and the monitoring member is the heartbeat backstop that surfaces a stall. The Director never runs `cafleet monitor start` itself.

```bash
# First member create: the monitoring member (canonical conditional-nudge routine).
cafleet member create --fleet-id 7 \
  --name "monitor" --description "Monitoring member: owns the heartbeat" \
  --role monitor --model sonnet \
  --text-file /repo/researches/pr-1234/prompts/monitor-20260516T003300Z.md \
  --json
# → {"member_id": 10, ...}
```

Wait for the monitoring member's `ready: monitor live` handshake before spawning the Summarizer (first-in gate).

### Render the Summarizer spawn prompt

The Director creates this spawn prompt body (with the `[INSERT …]` marker substituted before writing); the four `{...}` identity placeholders are left as written for the CLI to render at spawn:

```
You are the Summarizer in a summarize-pr team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/summarizer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition.

Load these skills at startup:
- the cafleet skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /repo/researches/pr-1234
CODING AGENT: {coding_agent}

INPUT FILE: /repo/researches/pr-1234/diff.patch
OUTPUT FILE: /repo/researches/pr-1234/summary.md

When you see cafleet message poll output with a message from the Director, capture the id: from each entry as the task id and ack it via cafleet message ack, then act on the instructions.

Read INPUT FILE, write a 200-word summary highlighting the top 3 risk areas to OUTPUT FILE, then send complete (doc) to the Director.
```

The Director writes this rendered text to `/repo/researches/pr-1234/prompts/summarizer-20260516T003344Z.md` (UTC-compact timestamp) — the audit artifact, carrying the identity placeholders pre-substitution.

### Spawn the Summarizer

```bash
cafleet member create --fleet-id 7 \
  --name "summarizer" \
  --description "Digests a PR diff into a 200-word risk summary" \
  --text-file /repo/researches/pr-1234/prompts/summarizer-20260516T003344Z.md \
  --json
# → {"member_id": 11, ...}
```

Capture `11` as the Summarizer's id for the rest of the run (the Summarizer itself reads its own id from the `YOUR MEMBER ID: 11` line the CLI rendered into its prompt).

### Coordination

The Summarizer reads the diff, writes the summary, and sends (from inside its pane, using the literal ids from its prompt):

```bash
cafleet message send --fleet-id 7 --from-member-id 11 --to-member-id 8 \
  --text "complete (doc) — summary 198 words, 3 risk areas"
```

The Director polls, acks, reads `${BASE}/summary.md`, presents it to the user via `AskUserQuestion`. If the user approves, the Director tears down. If the user requests revisions, the Director sends:

```bash
cafleet message send --fleet-id 7 --from-member-id 8 --to-member-id 11 \
  --text "ready (doc)"
```

(with a `COMMENT(director): <revision request>` marker at the top of `summary.md`) and waits for `addressed (doc)`.

### Teardown

```bash
# Stop the monitor's background task and delete the monitoring member FIRST
# (message the monitoring member to stop its `cafleet monitor start` task; wait for confirmation).
cafleet member delete --fleet-id 7 --member-id 10
cafleet member delete --fleet-id 7 --member-id 11
cafleet fleet delete --fleet-id 7
```

Order matters: stop the monitoring member's `monitor start` task and delete the monitoring member first (first-out), then delete the ordinary Summarizer, then delete the fleet (see § 2.5).

### What this example demonstrates

- All five integration sub-systems fire (resolve BASE → bootstrap fleet → spawn the monitoring member first → spawn the ordinary member → tear down monitor-first).
- The audit file at `${BASE}/prompts/summarizer-<ts>.md` lives under the task folder, not the repo root.
- The cafleet body uses the verb + pointer schema (`complete (doc)`, `ready (doc)`, `addressed (doc)`).
- The substantive revision request rides as a `COMMENT(director)` marker in the document, not in the cafleet body.
- Teardown is in the correct order.

---

## 7. Common failure modes

These are the failures that have bitten earlier authors. Read them before writing your skill, not after debugging.

### 7.1 Forgetting to ack messages

Symptom: every `cafleet message poll` returns the same message over and over, the Director's context fills with stale "ready" hops, and the monitor's health check flags the recipient as not-progressing.

Fix: every message you act on, ack it. Acking moves the task from `INPUT_REQUIRED` to `COMPLETED` and removes it from subsequent poll output.

### 7.2 Inlining role-file content into the spawn prompt

Symptom: `cafleet member create` exits non-zero with `Error: tmux split-window failed: command too long`, the member registration is rolled back, no member pane appears.

Fix: use `--text-file` (always) and reference the role file by absolute path inside the spawn prompt, not by inlining the content.

### 7.3 Shell-variable-substituting the Director's own literal ids

Symptom: every `cafleet ...` call the Director makes triggers a permission prompt that interrupts the agent loop. The user complains that the skill is "asking me about every single command."

Fix: in the **Director's own** commands, substitute the literal ids printed by `cafleet fleet create` / `cafleet member create` directly. The Claude Code harness's `permissions.allow` matches Bash invocations as literal command strings; an exported shell variable you reference yourself (`export FLEET_ID=…; --fleet-id $FLEET_ID`) breaks the literal match. Never `export` IDs and reference them via `$VAR`.

### 7.4 Writing audit files under the repo root

Symptom: `git status` shows untracked `prompts/` directory at the repo root after running the skill. Operators add `/prompts/` to `.gitignore`. Per-task evidence is scattered across the repo root instead of co-located with the task folder.

Fix: resolve BASE via the `cafleet` skill's `reference/base-dir.md` task-scope procedure with a `<task-relpath>` (per § 2.1). The resolved `base` IS the task folder; `${BASE}/prompts/` lives inside the task folder. Do NOT resolve the shared-root BASE (no task-relpath) and then write `${BASE}/researches/<slug>/prompts/...` — that pattern produces the stale repo-root artifacts.

### 7.5 Forgetting to omit the `BASE:` line under `${BASE} == <unset>`

Symptom: a member's spawn prompt contains the literal string `BASE: <unset>`, the member tries to compute `Path(BASE) / "prompts" / "..."`, and the file write fails with `OSError: [Errno 22] Invalid argument` (most filesystems reject `<` in paths) or the path appears literally in `git status` as `<unset>/prompts/...`.

Fix: when `${BASE} == <unset>`, drop the `BASE:` line from the spawn prompt body entirely. The member's existence-check (`grep '^BASE:'` on its own prompt) naturally treats audit-file features as disabled. The member emits the parens-free anchorless status `audit-disabled no BASE in spawn prompt` once.

### 7.6 Falling back to `/tmp` when BASE resolution fails

Symptom: scratch and audit files appear under `/tmp/<random>/prompts/...` instead of under `${BASE}`. The user cannot find their per-task evidence after the run.

Fix: never fall back to `/tmp` silently. The `<unset>` sentinel is a hard stop, not a fallback. If `${BASE}` is `<unset>`, abort with the standardized error `Error: BASE is <unset>; refusing to fall back to /tmp` (or, for spawned members, follow the skip + inline-fallback branch in § 3.6).

### 7.7 Calling `cafleet fleet delete` before `cafleet member delete`

Symptom: orphan `claude` processes lingering in tmux panes after the skill completes. The user closes the panes manually. On the next `cafleet fleet create`, the panes are rebound and the orphan members re-emerge.

Fix: tear down in this exact order — stop the monitoring member's `cafleet monitor start` task and `cafleet member delete` the monitoring member first, then `cafleet member delete` for every ordinary member, then `cafleet fleet delete`. See § 2.5.

### 7.8 Spawning ordinary members before the monitoring member is live

Symptom: ordinary members are spawned before the monitoring member's `ready: monitor live` handshake, so the heartbeat backstop is not yet running when they begin work.

Fix: the **first** `cafleet member create` is the `--role monitor` monitoring member; wait for its `ready: monitor live` message before spawning any ordinary member. The Director never runs `cafleet monitor start` itself.

### 7.9 Leaving stray single braces in the spawn prompt

Symptom: `cafleet member create` exits 2 with `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` or `Error: Malformed custom prompt: ...`, and the just-registered member is rolled back.

Fix: the only single-brace tokens allowed in a spawn prompt are the four identity placeholders. Double every literal brace (`{{` / `}}`) — including braces inside code snippets or JSON examples the prompt quotes.

---

## 8. Keep the base neutral; put backend deltas in the overlay

CAFleet runs members on three coding-agent backends — `claude`, `codex`, `opencode`. When a skill hardcodes one backend's idioms — a specific monitor `--model`, the permission-mode flags, the `AskUserQuestion` decision surface, the harness `Task*` tools, the "load via the Skill tool" recipe — it drifts the moment a member runs on another backend, and forces every non-`claude` reader to mentally subtract the `claude`-only parts. Keep your skill backend-neutral and push the backend specifics into the overlay.

The split:

- **Base — your `SKILL.md` and `roles/*.md`.** Write these so they read the same on any backend. State *what* to do in backend-agnostic terms; wherever behavior varies by backend, state the neutral behavior and point the agent at its overlay.
- **Overlay — `skills/cafleet/reference/coding-agent/<name>.md`.** This is the single canonical home for every backend delta. Six deltas vary by backend: the decision surface (the `AskUserQuestion` analog or the plain-message fallback), the monitor model, the auto-approval / permission flags, the background-task + task-list primitives, pane discovery / pane title, and the skill-loading recipe. Put each backend's concrete realization in its overlay; a new overlay starts from `reference/coding-agent/_template.md`.

Wire it up:

1. **Point at the overlay.** `skills/cafleet/SKILL.md` carries the canonical "apply your coding-agent overlay" instruction; every sibling family `SKILL.md` carries a one-line pointer to `../cafleet/reference/coding-agent/<name>.md`. A new family skill adds the same pointer near its top.
2. **Stamp the backend into the spawn prompt.** Add a `CODING AGENT: {coding_agent}` line to the spawn prompt's identity block (next to `FLEET ID` / `BASE`). The CLI renders it to the resolved backend name at spawn — no Director-side substitution and no CLI change — so a spawned member knows which overlay to read. A standalone agent uses its own identity instead.
3. **Keep the homes independent.** The agent-facing overlay home and the human-facing `docs/reference/coding-agents/` operator docs never cross-link in either direction. Restating an operational fact in both is fine; linking between them is not.

See `.claude/rules/coding-agent-overlay.md` for the convention in brief.

---

You now have everything you need to write a CAFleet-orchestrated skill. Re-read § 2 (the integration checklist) and § 5 (the coordination protocol) once more before you start writing the SKILL.md, and keep the worked example in § 6 open as a reference shape — but write the skill yourself, do not paste the example.
