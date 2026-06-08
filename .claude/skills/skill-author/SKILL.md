---
name: skill-author
description: >
  Teach an author how to integrate the CAFleet-orchestrated team pattern into a
  new skill they are writing. Auto-load when the author intends to create a new
  CAFleet-orchestrated skill, write a skill that spawns cafleet members, add a
  Director/Member team skill, design a multi-agent broker-coordinated skill,
  build a CAFleet-team-driven skill, or write a skill that uses
  `cafleet member create`. This skill is project-local to the cafleet repo and
  is fully self-contained — no `cafleet-base-dir` skill cross-reference is
  required to follow the guide.
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
 +-- Director (main Claude — runs cafleet fleet create / member create / drives the loop)
      +-- member-1 (claude pane)
      +-- member-2 (claude pane)
      +-- ...
```

Use this pattern when:

- The work needs **parallelism that the harness cannot provide on its own** — multiple members drafting in parallel, multiple researchers investigating sub-topics, multiple verifiers running on different slide ranges.
- The work needs **role specialization** — Director / Drafter / Reviewer / Programmer / Tester are roles that justify separate processes with separate role files.
- The work needs **persistent inter-agent memory** — the broker stores every message in SQLite and the audit-file path under `${BASE}/prompts/` is a permanent record.

Do **not** use this pattern when:

- The work fits inside one `Agent` tool subagent run. A subagent is cheaper and faster than a full CAFleet team.
- The work is a single-shot transformation (file edit, search, summarize). A normal slash command running inside the Director's session is enough.
- You only need the harness `TaskList` for tracking — you do not need cross-pane coordination.

If you are unsure: write the simpler version first. The CAFleet team pattern is overkill for most tasks and the orchestration overhead (`cafleet doctor`, `cafleet fleet create`, `cafleet member create`, `/loop` monitor, `member delete`, `fleet delete`) costs the user real seconds and real cognitive load.

---

## 2. The five-part integration checklist

Every CAFleet-orchestrated skill must wire up these five sub-systems, in this order, in its `SKILL.md`. The first three are setup; the fourth is the per-member spawn pattern; the fifth is teardown. Skip any one of them and the skill will fail silently in production.

### 2.1 Resolve the task-scoped BASE

Before any other work, the Director resolves the task-scoped output directory by following the `cafleet-base-dir` skill's task-scope resolution procedure with a `TASK_NAME` derived from the skill's per-task convention. The procedure uses only `git rev-parse --show-toplevel` (via Bash) and writes nothing at resolution time — there is no `cafleet` CLI subcommand for it.

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
cafleet --json fleet create --label "<short-label>"
```

The CLI atomically (1) creates a `fleets` row, (2) registers a root Director agent bound to the current tmux pane, (3) seeds a built-in Administrator agent. Capture both `fleet_id` and `director.agent_id` from the JSON response and substitute them as **literal UUID strings** into every subsequent `cafleet ...` call.

Never store these IDs in shell variables (`export FLEET=...`). The Claude Code harness's `permissions.allow` matches Bash invocations as literal command strings; shell variables break the literal match and force per-invocation permission prompts that interrupt the agent loop.

If the user is not inside a tmux session, `cafleet fleet create` exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing. Surface this and stop — do NOT try to start a tmux session yourself.

### 2.3 Start the agent-team-monitoring `/loop`

CAFleet members do not auto-poll. The broker delivers a 2-line inline preview into the recipient's pane via `tmux.send_inline_preview` keystroke; that preview is the trigger that wakes the recipient. If the keystroke is missed (pane buffered, recipient mid-Bash, etc.), the message just sits in `INPUT_REQUIRED` until the recipient runs `cafleet message poll` themselves.

To prevent dead loops, the Director MUST run a monitoring `/loop` that polls its own inbox at a regular interval (1 minute is typical), inspects member health via `cafleet member capture`, and nudges stalled members with `cafleet member ping`. The skill `agent-team-monitoring` documents the exact `/loop` template and the cron-like primitive (Claude Code uses `CronCreate` + `/loop`; codex has no in-session scheduling and uses fallback options).

Start the loop **before** the first `cafleet member create` call so the first tick fires while the first member is spawning.

### 2.4 Spawn members with `cafleet member create --prompt-file <abs path>`

For each member you spawn, follow the **two-step render-to-file pattern**:

1. **Render the spawn prompt locally**. Substitute every `[INSERT …]` marker with the concrete value. Leave the three str.format placeholders `{fleet_id}` / `{agent_id}` / `{director_agent_id}` single-braced — `cafleet member create` runs `str.format()` over the prompt at member-create time using the freshly-allocated `agent_id`. Double any other literal `{` or `}` in the prompt body (e.g. a JSON example, a `${{VAR}}` reference) to `{{` / `}}`.

2. **Write the rendered text** to `${BASE}/prompts/<role>-<UTC-compact>.md` where `<UTC-compact>` is `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")` (Python). Create `${BASE}/prompts/` on first write via `(Path(BASE) / "prompts").mkdir(parents=True, exist_ok=True)`. On same-second collisions, append `_2`, `_3`, … to the filename until it is unique — never overwrite. **The pre-spawn file IS the audit artifact.** There is no second post-spawn re-render; the file is both the CLI input and the permanent record of what was spawned.

3. **Spawn with `--prompt-file`** pointing at the absolute path of the rendered file:

   ```bash
   cafleet --fleet-id <fleet-id> --json member create --agent-id <director-agent-id> \
     --name "<member-name>" \
     --description "<one-sentence purpose>" \
     --prompt-file ${BASE}/prompts/<role>-<UTC-compact>.md
   ```

   Capture the printed `agent_id` from the JSON response and substitute it for the member's UUID in every subsequent `cafleet ...` call that targets it.

#### Why the two-step pattern instead of inline `prompt_argv`?

`cafleet member create` accepts the spawn prompt either as an inline positional `prompt_argv` argument or via `--prompt-file <abs path>`. Inline passes the prompt to `tmux split-window` as a single positional argument; tmux fails with `command too long` once the shell-quoted prompt grows past a few KB, and cafleet rolls back the agent registration. Real role files are typically 5–15 KB and the spawn prompt that includes them often pushes well past 20 KB. `--prompt-file` reads the file inside the cafleet process and writes the text to the new pane through a separate path that is not size-limited.

Use `--prompt-file` always. The inline `prompt_argv` form exists only as the documented fallback when `${BASE}` is `<unset>` and the audit-file write is impossible (see § 4).

#### Path-by-reference for role files

Do NOT inline a role definition (5–15 KB markdown file describing a member's accountability, communication protocol, role-specific workflow, escalation rules) into the spawn prompt. Instead, reference the role file by absolute path:

```
ROLE DEFINITION: Open <abs path to roles/<role>.md> with the Read tool BEFORE any other action.
```

The spawned member opens its role file with `Read` on its first turn. The role file lives in your skill's `roles/` directory and is stable, so this is safe. This pattern keeps the spawn prompt small (under the tmux limit), makes role updates take effect without a respawn, and concentrates role-specific accountability in a single canonical file rather than smearing it across the spawn prompt.

### 2.5 Tear down per the Shutdown Protocol

When the work is done, the Director MUST tear down in this exact order:

1. **`cafleet member delete --member-id <id>`** for every member. This sends `/exit` to the member's pane and waits up to 15 s for the pane to disappear. Surviving member coding-agent processes are NOT auto-closed by `cafleet fleet delete` — call `member delete` first.
2. **Stop the `/loop` monitor.** Use `CronDelete` with the cron job ID returned by `/loop` at startup. A loop that fires after the fleet is deleted will keystroke against a dead pane and clutter the broker log with errors.
3. **`cafleet fleet delete <fleet-id>`**. Soft-deletes the fleet (sets `deleted_at`), deregisters every active agent in the fleet (root Director + Administrator + remaining members), and physically deletes every associated `agent_placements` row. Tasks are preserved.

Order matters. If you call `fleet delete` before `member delete`, the member panes orphan (the `claude` process keeps running but has no broker to talk to). If you stop the `/loop` before `member delete`, the monitor cannot catch a member that fails its `/exit` shutdown.

---

## 3. Spawn-prompt anatomy

Every member spawn prompt follows the same skeleton. Read this section as the canonical anatomy — it explains every section, why it is there, and the substitution rules.

```
You are <role> in a <skill> team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/<role>.md] with the Read tool BEFORE any other action. That file is your authoritative role definition. Re-read it whenever you are unsure of protocol.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives, literal-UUID flag convention, and bash-via-Director routing
- <other skills as needed>

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: [INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]

<role-specific assignment text — e.g. CURRENT DATE, USER REQUEST, OUTPUT PATH, YOUR TASK ID>

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID from each entry as `[task-id]` and ack it via cafleet --fleet-id {fleet_id} message ack --agent-id {agent_id} --task-id [task-id], then act on the instructions.

<role-specific instructions>
```

### 3.1 The identity block

```
FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: <abs task-folder path>
```

These four lines are the member's grounding identity. The first three are the literal UUIDs the member will substitute into every `cafleet ...` call. The fourth is the resolved task-scoped BASE the Director computed in § 2.1 — the member uses this verbatim and MUST NOT re-resolve BASE on its own. Members that re-resolve risk drift from the Director's resolved BASE.

### 3.2 The `str.format()` placeholder rules

`cafleet member create` runs `str.format()` over the prompt at member-create time with three kwargs: `fleet_id`, `agent_id` (the freshly allocated member's UUID), and `director_agent_id`. The placeholders `{fleet_id}` / `{agent_id}` / `{director_agent_id}` are replaced with the literal UUIDs.

**Three rules:**

1. Leave those three placeholders single-braced exactly as shown. Do not URL-encode them, do not quote them, do not translate them. The cafleet code matches the literal `{<key>}` substring.
2. Double any other literal `{` or `}` in the prompt body to `{{` / `}}`. A JSON example like `{"foo": "bar"}` becomes `{{"foo": "bar"}}`. A shell-style reference to a Python f-string variable `${VAR}` becomes `${{VAR}}`.
3. With role content NOT inlined (per § 2.4), the prompt body rarely needs `{` or `}` at all. If you find yourself doubling many braces, that is a code smell — you are probably inlining content that belongs in the role file.

### 3.3 The `[INSERT ...]` shell-substitution rules

`[INSERT …]` markers are NOT runtime-substituted by cafleet. They are placeholders for **you** (the skill author writing the SKILL.md) to direct the Director on what to substitute when rendering. The Director performs every `[INSERT …]` substitution in step 1 of § 2.4 (local rendering) before writing the file. By the time the file lands at `${BASE}/prompts/<role>-<UTC-compact>.md`, no `[INSERT …]` marker should remain.

A common mistake is to leave a literal `[INSERT abs path to roles/<role>.md]` in the rendered file because the Director forgot to compute the absolute path. The member then opens a file that does not exist on its first turn. Spot-check the rendered files in `${BASE}/prompts/` before continuing.

### 3.4 The path-by-reference rule (one more time)

Spawn prompt body references the role file by absolute path. The role file lives in `<your skill dir>/roles/<role>.md`. Compute the absolute path once at Director startup and substitute it into every spawn prompt that needs it. Do NOT inline the role file content.

### 3.5 The `command too long` cliff and `--prompt-file`

`tmux split-window` accepts the spawn prompt as a single positional argument. Linux's `ARG_MAX` (the `execve()` argument-list size limit) and tmux's own command parser combine to fail with `command too long` once the shell-quoted prompt grows past a few KB. Even with role content not inlined, a prompt with multiple `[INSERT …]` substitutions + the identity block + the communication protocol + role-specific assignment text often exceeds the limit.

`--prompt-file <abs path>` sidesteps this. cafleet reads the file inside its own process and writes the text to the new pane through a separate path that is not size-limited. **Use `--prompt-file` for every spawn.** The inline `prompt_argv` form is the documented fallback only for the `${BASE} == <unset>` case where the file write is impossible.

### 3.6 The `${BASE} == <unset>` skip semantics

When the base-dir resolution yields the `unset` outcome (absolute-path argument outside the repo root, or equal to the repo root itself), `${BASE}` is the literal sentinel string `<unset>`. The skill MUST:

- **Skip the audit-file write.** Do not try to write `<unset>/prompts/<role>-<UTC-compact>.md` — that is a literal path with a `<` in it, which most filesystems reject. The guard is `if BASE != "<unset>"`.
- **Omit the `BASE:` line from the spawn prompt.** The spawn prompt does NOT include the literal string `BASE: <unset>` — the line is dropped entirely. The member's existence-check naturally treats audit-file features as disabled.
- **Fall back to inline `prompt_argv`** for the `cafleet member create` call (the file-write path is gone, so the only way to get the prompt to cafleet is inline). Be aware that this risks the `command too long` failure mode for prompts above the tmux limit; surface that as a hard error to the user, not as a silent retry.
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

4. **The pre-spawn file IS the audit artifact**. There is no second post-spawn re-render. After `cafleet member create` succeeds, the file at `${BASE}/prompts/<role>-<UTC-compact>.md` is the permanent record of what was spawned and is never touched again.

5. **Every write under `${BASE}`**. Audit files, scratch notes, figure artifacts, intermediate working files — every output the skill produces lands under `${BASE}` or a consumer-supplied absolute path. **Never `/tmp`** unless `${BASE}` itself is `/tmp/claude-code` (which is a legitimate base-dir choice when the user picked it via `AskUserQuestion`).

6. **`${BASE} == <unset>` is a hard stop**, not a fallback. See § 3.6.

This protocol is the single most-violated rule in past CAFleet-orchestrated skills. Authors routinely forget the same-second collision rule and overwrite the previous member's audit file when spawning two members in a tick. Authors routinely forget the `<unset>` sentinel and crash with a Path-with-`<`-in-it error. Read the rules above before writing any file write code.

---

## 5. Coordination protocol summary

Inter-agent communication uses the **verb + pointer** schema. This is the longest section in this guide because it is the most easily miswired surface.

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
| `<file>:<line>` | A specific file location — e.g. `cafleet/src/cafleet/broker.py:142`, `docs/concepts/overview.md:178`. |
| `doc` | The whole document; used for top-level milestones (`complete (doc)`, `approved (doc)`) and document-wide blocks (`blocked (doc) — test framework ambiguous`). |

### 5.4 The pointer-marker pairing rule

When a sender includes substantive content in a `COMMENT(role)` marker, the marker MUST live at the same pointer the cafleet body references. Examples:

- `blocked (paragraph-Implementation > Step 5)` + `COMMENT(programmer): test X expects Y but design doc says Z` at `paragraph-Implementation > Step 5` in the design doc.
- `ready (cafleet/src/cafleet/broker.py:142)` + `COMMENT(director): use pathlib.Path.mkdir(parents=True, exist_ok=True), not subprocess.run(["mkdir", ...])` at line 142 of the file.

The recipient reads the cafleet body, navigates to the pointer, reads the standing marker, applies the fix or arbitration, removes the marker, and replies with the next-step verb (`addressed (...)` for member-side fixes, `ready (...)` for director-side arbitration handoffs).

### 5.5 The role taxonomy

The marker role identifier in `COMMENT(<role>)` is one of: `claude` (user-derived clarifications baked into the doc), `director`, `programmer`, `tester`, `reviewer`, `verifier`, `analyzer`, `drafter`. New roles for new skills SHOULD be added to this list; do not abbreviate. The `claude` role is reserved for the `cafleet-design-doc-interview` skill's user-derived clarifications and the `cafleet-design-doc-execute` skill's test-framework arbitration; do not use it for arbitrary "Claude said this" content.

### 5.6 Anchorless status

A small handful of statuses do not pair with a pointer because the condition is global. These are emitted as parens-free anchorless strings:

- `audit-disabled no BASE in spawn prompt` — emitted by a member whose spawn prompt lacks the `BASE:` line entirely (the `${BASE} == <unset>` branch).
- (Other anchorless statuses are documented per-skill; do not invent new ones casually.)

The phrasing deliberately omits parentheses so a parser does not misinterpret it as a malformed `<verb> (<pointer>)` hop.

### 5.7 Acking messages

After acting on a polled message, the recipient MUST `cafleet message ack` it. Un-acked messages stay in `INPUT_REQUIRED` and re-surface on every subsequent `message poll` cycle, polluting the recipient's context with stale work.

```bash
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

The `<task-id>` is the full UUID returned by `cafleet --json message poll --agent-id <my-agent-id> --full`. The default text-mode poll output truncates to an 8-character prefix; pass `--full` when you need the full UUID for ack.

---

## 6. Worked example — `summarize-pr`

This is a tiny end-to-end CAFleet-orchestrated skill called `summarize-pr` (single Director + single member named Summarizer). The example uses fake `<slug>`, `<fleet-id>`, etc. and is read-only — it is illustrative, not a template you copy. Read it to understand how all five sub-systems fit together; then write your own skill from scratch.

### Skill purpose

The user invokes `/summarize-pr <pr-number>`. The Director:

1. Fetches the PR diff via `gh pr diff <pr-number>`.
2. Spawns a single Summarizer member to digest the diff, identify the top 3 risk areas, and write a 200-word summary to a file.
3. Reviews the summary, asks the user for approval, then tears down.

### Resolved task-relpath

The skill's task convention is `researches/pr-<pr-number>` (PR summaries are research-shaped — one folder per PR with the diff + summary inside).

```text
# Resolve task-scope BASE for researches/pr-1234 (cafleet-base-dir skill procedure, built-in tools):
#   git rev-parse --show-toplevel → /repo
#   task folder → /repo/researches/pr-1234  (auto-created)
```

`${BASE} = /repo/researches/pr-1234`. The Director writes the diff to `${BASE}/diff.patch` (a non-audit working file) and the summary will land at `${BASE}/summary.md` (also a working file, not under `prompts/`).

### Fleet bootstrap

```bash
cafleet --json fleet create --label "summarize-pr-1234"
# → {"fleet_id": "abc...", "director": {"agent_id": "def..."}, "administrator_agent_id": "ghi..."}
```

Substitute `abc...` and `def...` literally into every subsequent call.

### Loop start

```
/loop 1m <agent-team-monitoring template with fleet-id=abc... and director-agent-id=def...>
```

Loop fires every minute starting before the first `member create`.

### Render the Summarizer spawn prompt

The Director creates this spawn prompt body (with the four `[INSERT …]` markers substituted before writing):

```
You are the Summarizer in a summarize-pr team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/summarizer.md] with the Read tool BEFORE any other action. That file is your authoritative role definition.

Load these skills at startup:
- the `cafleet` skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR AGENT ID: {director_agent_id}
YOUR AGENT ID: {agent_id}
BASE: /repo/researches/pr-1234

INPUT FILE: /repo/researches/pr-1234/diff.patch
OUTPUT FILE: /repo/researches/pr-1234/summary.md

COMMUNICATION PROTOCOL:
- Report to Director: cafleet --fleet-id {fleet_id} message send --agent-id {agent_id} --to {director_agent_id} --text "..."
- When you see cafleet message poll output with a message from the Director, capture the `id:` UUID and ack it via cafleet --fleet-id {fleet_id} message ack --agent-id {agent_id} --task-id [task-id], then act on the instructions.

Read INPUT FILE, write a 200-word summary highlighting the top 3 risk areas to OUTPUT FILE, then send `complete (doc)` to the Director.
```

The Director writes this rendered text to `/repo/researches/pr-1234/prompts/summarizer-20260516T003344Z.md` (UTC-compact timestamp) — the audit artifact.

### Spawn the Summarizer

```bash
cafleet --fleet-id abc... --json member create --agent-id def... \
  --name "summarizer" \
  --description "Digests a PR diff into a 200-word risk summary" \
  --prompt-file /repo/researches/pr-1234/prompts/summarizer-20260516T003344Z.md
# → {"agent_id": "jkl...", ...}
```

Capture `jkl...` as the Summarizer's UUID for the rest of the run.

### Coordination

The Summarizer reads the diff, writes the summary, and sends:

```bash
cafleet --fleet-id abc... message send --agent-id jkl... --to def... \
  --text "complete (doc) — summary 198 words, 3 risk areas"
```

The Director polls, acks, reads `${BASE}/summary.md`, presents it to the user via `AskUserQuestion`. If the user approves, the Director tears down. If the user requests revisions, the Director sends:

```bash
cafleet --fleet-id abc... message send --agent-id def... --to jkl... \
  --text "ready (doc)"
```

(with a `COMMENT(director): <revision request>` marker at the top of `summary.md`) and waits for `addressed (doc)`.

### Teardown

```bash
cafleet --fleet-id abc... member delete --member-id jkl...
CronDelete <loop-job-id>
cafleet fleet delete abc...
```

Order matters. Member first, loop second, fleet last.

### What this example demonstrates

- All five integration sub-systems fire (resolve BASE → bootstrap fleet → start loop → spawn member → tear down).
- The audit file at `${BASE}/prompts/summarizer-<ts>.md` lives under the task folder, not the repo root.
- The cafleet body uses the verb + pointer schema (`complete (doc)`, `ready (doc)`, `addressed (doc)`).
- The substantive revision request rides as a `COMMENT(director)` marker in the document, not in the cafleet body.
- Teardown is in the correct order.

---

## 7. Common failure modes

These are the failures that have bitten earlier authors. Read them before writing your skill, not after debugging.

### 7.1 Forgetting to ack messages

Symptom: every `cafleet message poll` returns the same message over and over, the Director's context fills with stale "ready" hops, and the loop monitor flags the recipient as not-progressing.

Fix: every message you act on, ack it. Acking moves the task from `INPUT_REQUIRED` to `COMPLETED` and removes it from subsequent poll output.

### 7.2 Inlining role-file content into the spawn prompt

Symptom: `cafleet member create` exits non-zero with `Error: tmux split-window failed: command too long`, the agent registration is rolled back, no member pane appears.

Fix: use `--prompt-file` (always) and reference the role file by absolute path inside the spawn prompt, not by inlining the content.

### 7.3 Shell-variable-substituting the literal UUIDs

Symptom: every `cafleet ...` call triggers a permission prompt that interrupts the agent loop. The user complains that the skill is "asking me about every single command."

Fix: substitute the literal UUIDs printed by `cafleet fleet create` / `cafleet member create` directly into every command. The Claude Code harness's `permissions.allow` matches Bash invocations as literal command strings; shell variables (`$FLEET_ID`, `${FLEET_ID}`) break the literal match. Never `export` IDs and reference them via `$VAR`.

### 7.4 Writing audit files under the repo root

Symptom: `git status` shows untracked `prompts/` directory at the repo root after running the skill. Operators add `/prompts/` to `.gitignore`. Per-task evidence is scattered across the repo root instead of co-located with the task folder.

Fix: resolve BASE via the `cafleet-base-dir` skill's task-scope procedure with a `<task-relpath>` (per § 2.1). The resolved `base` IS the task folder; `${BASE}/prompts/` lives inside the task folder. Do NOT resolve the shared-root BASE (no task-relpath) and then write `${BASE}/researches/<slug>/prompts/...` — that pattern produces the stale repo-root artifacts.

### 7.5 Forgetting to omit the `BASE:` line under `${BASE} == <unset>`

Symptom: a member's spawn prompt contains the literal string `BASE: <unset>`, the member tries to compute `Path(BASE) / "prompts" / "..."`, and the file write fails with `OSError: [Errno 22] Invalid argument` (most filesystems reject `<` in paths) or the path appears literally in `git status` as `<unset>/prompts/...`.

Fix: when `${BASE} == <unset>`, drop the `BASE:` line from the spawn prompt body entirely. The member's existence-check (`grep '^BASE:'` on its own prompt) naturally treats audit-file features as disabled. The member emits the parens-free anchorless status `audit-disabled no BASE in spawn prompt` once.

### 7.6 Falling back to `/tmp` when BASE resolution fails

Symptom: scratch and audit files appear under `/tmp/<random>/prompts/...` instead of under `${BASE}`. The user cannot find their per-task evidence after the run.

Fix: never fall back to `/tmp` silently. The `<unset>` sentinel is a hard stop, not a fallback. If `${BASE}` is `<unset>`, abort with the standardized error `Error: BASE is <unset>; refusing to fall back to /tmp` (or, for spawned members, follow the skip + inline-fallback branch in § 3.6).

### 7.7 Calling `cafleet fleet delete` before `cafleet member delete`

Symptom: orphan `claude` processes lingering in tmux panes after the skill completes. The user closes the panes manually. On the next `cafleet fleet create`, the panes are rebound and the orphan members re-emerge.

Fix: tear down in this exact order — `cafleet member delete` for every member, then stop the `/loop`, then `cafleet fleet delete`. See § 2.5.

### 7.8 Not starting the `/loop` before the first `cafleet member create`

Symptom: the first member spawns and sits idle waiting for a Director-side ack. The `/loop` only starts after the spawn returns, so the first tick is several seconds late, and a fast member can complete its first task before the Director polls its inbox.

Fix: start the `/loop` BEFORE the first `cafleet member create`. The first tick fires while the first member is spawning and is the canonical no-op tick that establishes the loop is running.

---

You now have everything you need to write a CAFleet-orchestrated skill. Re-read § 2 (the integration checklist) and § 5 (the coordination protocol) once more before you start writing the SKILL.md, and keep the worked example in § 6 open as a reference shape — but write the skill yourself, do not paste the example.
