# Coding-Agent Overlays

One top-level section per backend; every section is self-contained. Identify your backend from your spawn prompt's `CODING AGENT:` line (a standalone agent uses its own identity), then resolve **only your backend's section** per the cafleet `SKILL.md` § *Resolve your overlay*. Values in other sections are never applicable to you — taking a value from another backend's section is a resolution defect, the same class as emitting a literal `{token}`. The cross-section readers are the Director and the monitor member: each applies the **target member's** section for pane-state cues.

## claude

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{reviewer_model}` | `fable` |
| `{monitor_model}` | `haiku` |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |

### Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| *Pane-state capture cues* (below) — the concrete claude-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | claude capture cue |
|---|---|
| `awaiting_user` | A bordered prompt box awaiting a keypress: an `AskUserQuestion` selection list (numbered options with a `❯`-marked choice and an "Other" free-text entry) or a tool/permission approval prompt (`Do you want to proceed?` with `❯ 1. Yes` / `2. No`). A choice is pending; the composer is not at rest. |
| `finished` | The bare composer at rest: an empty `>` input line with its placeholder hint and the idle status line beneath (model + context, the `⏵⏵` mode indicator), with **no** question/approval box above it and **no** running spinner or `esc to interrupt` indicator. |
| `working` | Affirmative active-work evidence: a running spinner, `esc to interrupt`, streaming response, tool execution, or generation indicator. A truncated or ambiguous capture that might hide one of these cues is also `working`. |
| `stall_candidate` | Quiet, non-finished transcript content with no question/approval box, no empty at-rest composer, and no spinner, tool, streaming, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

The canonical monitor-member-side loop launch, fully resolved for this backend:

Launch `cafleet monitor <fleet-id>` via the Bash tool with `run_in_background: true`, confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in the task output, then send `monitor live` to the Director.

## codex

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{reviewer_model}` | `gpt-6-astra` |
| `{monitor_model}` | `gpt-5.6-luna` |
| `{permission_flags}` | `--ask-for-approval never --sandbox workspace-write` |
| `{bg_run}` | a retained Codex-managed execution session created without shell `&` |
| `{bg_stop}` | interrupting or terminating the retained managed execution session |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides |
| `{effort_levels}` | `minimal`, `low`, `medium`, `high`, `xhigh` (spawn flag `--effort <level>`, forwarded as `--config=model_reasoning_effort=<level>`) |

### Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. Ask a concrete, answerable question, not free-form prose. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| Run the monitor command without shell `&` and retain the active managed execution session's returned session ID. Inspect the initial output for `monitor loop started`; if the line is absent and the session remains active, perform one immediate poll. A missing session ID or an early exit is a failed start; if the session is active but unconfirmed after that poll, terminate it. Withhold `monitor live` and report startup failure unless the line was observed. Whenever a broker message reopens a later monitor-member turn, poll the retained session ID once before any other work. If it exited, relaunch with the same bounded confirmation and send `monitor restarted` only after the replacement is confirmed. The monitor member alone owns the session ID and polling; it never sends the ID to the Director or creates a timer or sleep loop. | `{bg_run}` / `{bg_stop}` — `cafleet/roles/monitor.md` § On spawn / § Standing obligation; `cafleet/reference/supervision.md` § Monitor-first Bootstrap / § Idle Semantics |
| *Pane-state capture cues* (below) — the concrete codex-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | codex capture cue |
|---|---|
| `awaiting_user` | A codex confirmation/approval prompt awaiting a keypress — a `[y/n]`-style command-approval or an out-of-sandbox escalation request. Under `--ask-for-approval never --sandbox workspace-write` codex auto-approves in-workspace work, so a visible approval prompt means codex hit something the policy could not clear and is genuinely waiting. |
| `finished` | The empty codex composer at rest — the input prompt with no streaming output above it and no active-turn / "working" indicator. |
| `working` | Affirmative active-work evidence: the active-turn or `working` indicator, streaming model output, a running tool, or generation in progress. A truncated or ambiguous capture that may still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no confirmation prompt, no empty at-rest composer, and no active-turn, tool, streaming, generation, or other work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

The canonical monitor-member-side loop launch, fully resolved for this backend:

Launch `cafleet monitor <fleet-id>` without shell `&` as a Codex-managed execution and retain the returned session ID. Inspect the initial output for `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`; when the line is absent and the session is active, perform one immediate poll of that retained session. Treat a missing session ID or early exit as startup failure. If the execution remains active but unconfirmed after the poll, terminate it. Send `monitor live` to the Director only after observing the startup line; otherwise withhold it and report the failure.

## opencode

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | a Director-relayed operator message |
| `{reviewer_model}` | `opencode/glm-5.2` |
| `{monitor_model}` | `opencode/big-pickle` |
| `{permission_flags}` | `--agent cafleet` |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the cafleet `SKILL.md` core + this overlay by the absolute paths the spawn prompt provides |
| `{effort_levels}` | unsupported — omit `--effort` |

### Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| No in-pane prompt — a fleet member sends its question to the Director, which answers as a plain operator message. The `--agent cafleet` safety floor shows no popup; if a popup ever appears it is a regression to escalate, not a decision point. | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions |
| *Pane-state capture cues* (below) — the concrete opencode-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

A cross-section reader — the monitor member on each wake, the Director at its pre-ping gate — classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | opencode capture cue |
|---|---|
| `awaiting_user` | An opencode permission/selection popup awaiting a choice. The `--agent cafleet` floor suppresses permission popups, so a visible popup is both the `awaiting_user` signal **and** a regression to escalate (see the decision-surface note above). |
| `finished` | The empty opencode prompt at rest — no streaming response above it and no active generation indicator. |
| `working` | Affirmative active-work evidence: streaming response text, an active generation indicator, a running tool, or any other visible in-progress state. A truncated or ambiguous capture that might still be active is also `working`. |
| `stall_candidate` | Quiet, non-finished content with no popup, no empty at-rest prompt, and no streaming, tool, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

### Worked resolution

The canonical monitor-member-side loop launch, fully resolved for this backend:

Launch `cafleet monitor <fleet-id> &` as a backgrounded `!` shell command, confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in its output, then send `monitor live` to the Director.

## Template

Adding a backend means copying this section into a new `## <name>` section of this same file and filling in every angle-bracket placeholder.

Substitute these into the base `{…}` placeholders. Each value must be a short noun phrase that reads correctly when substituted inline into a base sentence; push any constraint or caveat into the *Note → applies at* table below (a required section), where each note names the base token/instruction it qualifies.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | <this backend's recorded-user-reaction surface as a short noun phrase: its interactive prompt tool, or "a Director-relayed operator message" — a fleet member always routes its question to the Director> |
| `{reviewer_model}` | <this backend's reviewer default from the model list's *Monitor and reviewer defaults* table> |
| `{monitor_model}` | <this backend's monitor default from the model list's *Monitor and reviewer defaults* table> |
| `{permission_flags}` | <the exact spawn flags for workspace-scoped auto-approval> |
| `{bg_run}` | <this backend's primitive for running long-lived background work, as a noun phrase> |
| `{bg_stop}` | <the matching stop primitive, as a noun phrase> |
| `{pane_title}` | <any `--name`-style pane-title analog, or "no `--name` analog"> |
| `{skill_loader}` | <the skill-loader, or the read-by-absolute-path fallback, as a noun phrase> |
| `{effort_levels}` | <this backend's accepted reasoning-effort levels and spawn flag for `cafleet member create --effort`, or "unsupported — omit `--effort`"> |

### Note → applies at

Required section. Convert every note (a constraint/caveat the inline value shouldn't carry — e.g. the decision surface's question-shape taxonomy) into a row of this table. **Every note names the base token/instruction it qualifies**: the *Applies at* cell leads with the `{token}` the note binds to, followed by the base section(s) where it takes effect (`<skill>/<file>` § <heading>). A floating note with no bound anchor is not allowed.

| Note | Applies at |
|------|-----------|
| <the caveat, one row each> | `{token}` — `<skill>/<file>` § <base heading> |
| *Pane-state capture cues* (below) — this backend's `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate` discriminators. | the monitor member's on-wake classification (its role file's § *On each wake*) and the Director's pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in [Monitoring](runtime/concepts/monitoring.md) (each reader applies the cues of the **target member's** backend overlay). |

### Pane-state capture cues

Required section. Give concrete capture-content discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`, from pane text alone and never native `agent_status`. `working` includes visible streaming, generation, tool execution, or any ambiguous/truncated state that might still be active. `stall_candidate` is quiet, non-finished content with no prompt and no active-work cue. After the table, add the pointer sentence "Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*." instead of restating those rules. Register the table in *Note → applies at* and bind it to the monitor member's on-wake classification and the Director's pre-ping gate.

| State | <backend> capture cue |
|---|---|
| `awaiting_user` | <the concrete frame this backend shows when a pane is waiting on a user answer or approval — the box/prompt shape, awaiting a keypress> |
| `finished` | <the concrete frame this backend shows at a completed turn — the empty composer/prompt at rest, no pending box, no active-turn indicator> |
| `working` | <affirmative streaming/generation/tool/active-turn indicators; include ambiguous or truncated content that may still be active> |
| `stall_candidate` | <quiet non-finished content with no prompt, no empty finished composer, and no affirmative active-work cue> |

State both ambiguity tie-breaks: `awaiting_user` wins over `finished`, and `working` wins over `stall_candidate`.

### Worked resolution

Required section. Give the canonical monitor-member-side loop launch fully resolved for this backend — every `{placeholder}` replaced by its concrete value — so the reader has a concrete string to match rather than a transformation to invent:

Launch `cafleet monitor <fleet-id>` via <this backend's background-run primitive>, confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in its output, then send `monitor live` to the Director.
