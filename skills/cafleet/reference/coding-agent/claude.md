# Overlay: claude

Apply these deltas on top of the cafleet base. The base states each instruction in backend-neutral terms and points here; this overlay states how Claude Code realizes it.

## 1. Decision surface

When you need a recorded user reaction — **approve**, **choose among options**, **confirm**, or **continue-or-abort** — solicit it through the `AskUserQuestion` tool. This is the single canonical surface for every user reaction across CAFleet skills, roles, and rules. Never request a reaction in free-form prose ("let me know if this looks good", "shall I proceed?", "reply with your choice") — that surface records no answer and routinely stalls.

**Standalone vs. fleet.** A standalone Claude Code agent (running a skill directly, no fleet) calls `AskUserQuestion` itself. A fleet **member** never talks to the user: it sends its question to the Director via `cafleet message send`, and the **Director** relays it through `AskUserQuestion` (see the `cafleet-agent-team-supervision` skill § *User Delegation Protocol*).

**Question-shape taxonomy** — pick the shape that fits; ≤ 4 options each. The tool's built-in "Other" always exposes a free-text field, so do NOT add an explicit "Write my own" / "Custom" option. No preamble sentence above the question — the conversation context plus the question text carry it.

| Reaction shape | AskUserQuestion form |
|---|---|
| Choice among labeled options | Up to 4 options mirroring the labels |
| Approve / yes-no | Two options (e.g. Approve / Revise) |
| Continue-or-abort | Two options (Continue / Abort) |
| Open-ended "what next" / draft selection | 2–4 complete candidate bodies to compare side-by-side |

**Every escalation is a decision point — give it a surface.** "Escalate to the user", "surface to the user", and "defer to the user" name no surface on their own. Whenever an agent escalates — including a pure action handoff (e.g. "the user must start the server manually") — front it with an `AskUserQuestion` continue/abort gate so no escalation is ever left surface-less.

**Exemptions** (no reaction is being solicited, so `AskUserQuestion` is not required):

- A final informational status report that asks for no decision ("Done — here is what changed"). Reporting an outcome is not soliciting a reaction.
- A member's question to its Director: the member uses `cafleet message send`; the Director then relays via `AskUserQuestion`.

### Answering a member's AskUserQuestion prompt (`cafleet member send-input`)

`cafleet member send-input` forwards a restricted keystroke to a member's tmux pane — the AskUserQuestion-only, write-path companion to `cafleet member capture`. Exactly one of `--choice` (integer `1`–`3`, sends the digit) or `--freetext` (sends `4`, then the literal text, then `Enter`; newlines and a leading `!` rejected — use `member exec` for shell) must appear.

When `cafleet member capture` reveals a member paused on an AskUserQuestion-shaped 4-option frame (`1. …`, `2. …`, `3. …`, `4. Type something`), the Director MUST delegate the decision to the user via the three-beat shape:

1. **Capture** with `--lines 120` (recommended default; bump to `--lines 200` only if the AskUserQuestion frame is truncated).
2. **Ask the user via `AskUserQuestion`** with shape-appropriate options (table below). The question text names the member; no preamble sentence above the question.
3. **Invoke the resolved `cafleet member send-input`** via the Director's own Bash tool. Claude Code's per-call Bash permission prompt is the user-consent surface — never print a fenced `bash` block as an instruction.

#### Pane prompt shapes

The pane is ALWAYS on the AskUserQuestion 4-option frame when `send-input` is appropriate.

| Shape | Member pane looks like | Director's AskUserQuestion options | Resolved send-input call |
|---|---|---|---|
| **Choice-routing** | Option labels `1.`/`2.`/`3.` are the decision point. | Mirror UP TO 3 of the member's labels (don't add a 4th — `--choice` is `IntRange(1, 3)` and built-in "Other" handles freetext). | `--choice N` for picked mirror option; `--freetext "<typed>"` for built-in Other. |
| **Open-ended** | Option labels are NOT useful — the member is waiting for free-form instruction. | 2–4 *complete candidate message bodies*. `label` is a short intent tag (≈12 chars); `description` holds the full draft body. | `--freetext "<picked body>"` or `--freetext "<typed>"`. |
| **Other shapes** | Pane is NOT on an AskUserQuestion (mid-command, REPL, crashed, yes/no confirmation, mid tool-call). | Do NOT call `AskUserQuestion`; do NOT call `send-input`. Sending any keystroke would corrupt pane state. | None. Escalate via `cafleet message send`, or wait. |

#### AskUserQuestion constraints

- 1–4 questions per call; 2–4 options per question.
- Built-in "Other" is always exposed by the tool — do NOT add an explicit "Write my own" / "Custom" option.
- ≥ 5 candidate bodies → narrow to 2–4 (drop near-duplicates, span decision axes). Do NOT paginate.
- No preamble text above the question — the capture output already printed plus the question text carry all context.

Validation and key sequences: [`cli-options.md`](../../../../docs/spec/cli-options.md#member-send-input).

## 2. Monitor model

`--model haiku` — the cheapest capable model for the claude backend. The Director substitutes this into the `cafleet member create … --role monitor` spawn command.

## 3. Auto-approval / permission mode

`--permission-mode dontAsk`. The member's Bash tool is enabled and routine permission prompts auto-resolve; the deny-list fallback routes the blocked command through the Director (`cafleet member exec`).

## 4. Background-task + task-list primitives

Run long-lived background work (e.g. the Slidev dev server) via the Bash tool with `run_in_background: true`; the returned task id feeds `TaskStop` at teardown. Coordinate parallel sub-work via the harness task-list primitives — `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet` are the work-coordination substrate.

## 5. Pane discovery / pane title

`cafleet member list` (the `pane_id` column) is ground truth for locating a member's pane. On the claude backend, `claude --name <member-name>` additionally sets `#{pane_title}` to the member name, so claude panes are also identifiable by title.

## 6. Skill-loading recipe

Load the listed skills at startup via the Skill tool; dispatch sub-agents via the Agent tool. Do not read skill files directly when the loader is available.
