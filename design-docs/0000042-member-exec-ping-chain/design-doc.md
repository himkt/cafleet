# Member Exec then Ping Chain Protocol

**Status**: Approved
**Progress**: 10/19 tasks complete
**Last Updated**: 2026-05-01

## Overview

After every successful `cafleet member exec`, the Director MUST follow up with `cafleet member ping` against the same member so the member's next turn fires immediately. Without this chain, both Director and member sit idle until the 1-minute `cafleet-monitoring` tick re-pokes the member, introducing avoidable latency. The fix is documentation-only — the two primitives already exist and are correctly scoped.

## Success Criteria

- [ ] `skills/cafleet/SKILL.md` Member Exec subsection documents the chain rule with one canonical exec-then-ping bash snippet, placed after the JSON output example and before the Member Ping subsection.
- [ ] `skills/cafleet/roles/director.md` "What you MUST do" list contains a step explicitly requiring `cafleet member ping` after every successful `cafleet member exec`, with subsequent steps renumbered.
- [ ] `skills/cafleet-monitoring/SKILL.md` escalation-table ping row enumerates both the original "stalled despite recent message send" use case AND the new post-exec chain use case in a single row.
- [ ] An end-to-end live verification has been performed where a Director (or member acting as Director) issues `cafleet member exec` followed by `cafleet member ping` and observes the member begin its next turn without waiting for the 1-minute monitoring tick.
- [ ] No surface mentions `cafleet message poll` as the chain primitive — the miscopy guard.

---

## Background

`cafleet member exec` keystrokes `! <command>` + `Enter` into a member's pane via `tmux.send_bash_command`. Claude Code's `!` shortcut runs the command and stages its captured stdout/stderr as context for the member's next turn — but **staging context is not the same as advancing a turn**. Without an additional user-message keystroke, the member sits at the input prompt waiting.

`cafleet member ping` is the primitive that solves this: it keystrokes a fresh `cafleet --session-id <s> message poll --agent-id <m>` line into the member's pane via `tmux.send_poll_trigger` (`cafleet/src/cafleet/tmux.py:83`). The fresh keystroke lands as the next user message after the bang output is staged, forcing the member's next turn.

The 1-minute `cafleet-monitoring` `/loop` tick eventually fires the same `tmux.send_poll_trigger` keystroke and wakes the member, so this is a latency issue, not a correctness one. But re-using the monitor as the wake-up path is wasteful when the Director already knows it just dispatched a command.

Today the chain is missing from the canonical Director protocol docs. Operators discover the latency by observation and re-derive the workaround. Documenting the chain as a Director protocol rule eliminates the rediscovery.

---

## Specification

### The chain rule

> **After every successful `cafleet member exec` invocation, the Director MUST immediately invoke `cafleet member ping` against the same member.**
>
> The chain is unconditional on exec success. It is skipped only when `cafleet member exec` exits non-zero — in that case the dispatch never reached the pane, and the 1-minute `cafleet-monitoring` tick is the safety net.

### Why ping, not poll

The doc must make this distinction explicit so the rule is not miscopied as `cafleet message poll`.

| Primitive | Effect | Wakes the member? |
|---|---|---|
| `cafleet message poll --agent-id <director-agent-id>` | Polls the **Director's** inbox over SQLite. Returns messages addressed to the Director. | No. |
| `cafleet member ping --agent-id <director-agent-id> --member-id <member-agent-id>` | Keystrokes `cafleet --session-id <s> message poll --agent-id <m>` + `Enter` into the **member's** pane via `tmux.send_poll_trigger` (`cafleet/src/cafleet/tmux.py:83`). | Yes — the keystroke lands as the member's next user message. |

Documents that mention the chain MUST name `cafleet member ping` explicitly. They MUST NOT abbreviate to "poll", "message poll", or "the poll trigger".

### Canonical exec-then-ping pairing

```bash
# 1. Dispatch the shell command into the member's pane.
cafleet --session-id <session-id> member exec \
  --agent-id <director-agent-id> --member-id <member-agent-id> \
  "<command>"

# 2. Immediately fire the poll-trigger keystroke so the member begins its next turn.
cafleet --session-id <session-id> member ping \
  --agent-id <director-agent-id> --member-id <member-agent-id>
```

Each call is a separate Bash invocation. Do not chain with `&&` — the project's `.claude/rules/bash-command.md` prohibits shell operators in CAFleet command invocations because they break `permissions.allow` literal matching. Issuing them as two consecutive Bash calls is correct.

### Series of exec calls

For a series of exec calls on the same member, the ping MUST follow each exec, not only the last one. Every bang command stages its own output as context, and the member needs a turn to consume each before the Director's next dispatch is meaningful. Skipping intermediate pings leaves the member chewing through queued bang output without an opportunity to react in between.

### Behavior on ping failure

`cafleet member ping` exits 1 when `tmux.send_poll_trigger` returns False (tmux missing on PATH, pane dead, send-keys subprocess error). The Director treats a ping failure as a **non-fatal warning**: the failure has already been printed to stderr by the CLI, so surface it but do not abort the workflow. The 1-minute `cafleet-monitoring` tick is the recovery path — it will fire the same `send_poll_trigger` keystroke on the next loop iteration.

### Pane-shape precondition

The chain inherits the existing pane-shape guidance from `skills/cafleet/SKILL.md`. `cafleet member exec` is appropriate only when the member's pane is at the Claude Code input prompt; sending `! <command>` + `Enter` into an `AskUserQuestion`-shaped pane corrupts pane state. Wherever exec is appropriate, ping is appropriate. The new chain rule does not re-state the precondition — it inherits transitively.

### Out of scope

- No code change to `cafleet member exec` (no auto-ping behavior, no `--ping-after` flag).
- No change to `cafleet member ping` semantics.
- No change to the 1-minute `cafleet-monitoring` `/loop` interval.
- No change to `cafleet/src/cafleet/tmux.py` or `cafleet/src/cafleet/cli.py`.
- No change to `README.md` or `ARCHITECTURE.md` — the chain is a Director protocol detail at the skill-doc layer, not a top-level architectural concept.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Document the chain in skills/cafleet/SKILL.md

Target subsection: "Member Exec" (under `## Command Reference`).

- [x] Add a paragraph titled "Required follow-up: cafleet member ping" placed AFTER the existing JSON output example for `member exec` and BEFORE the next subsection (`### Member Ping`). <!-- completed: 2026-05-01T00:10 -->
- [x] Inside that paragraph, embed the canonical exec-then-ping bash snippet from this design doc's Specification section verbatim. <!-- completed: 2026-05-01T00:10 -->
- [x] State explicitly that `cafleet member ping` (not `cafleet message poll`) is the correct follow-up primitive, with a one-line distinction summarizing the table from the Specification. <!-- completed: 2026-05-01T00:10 -->
- [x] State that the ping is unconditional on exec success and is skipped on exec exit-non-zero. <!-- completed: 2026-05-01T00:10 -->
- [x] State that for a series of exec calls on the same member, the ping follows each exec, not only the last. <!-- completed: 2026-05-01T00:10 -->

### Step 2: Document the chain in skills/cafleet/roles/director.md

Target list: the "What you MUST do" numbered list inside the bash-routing protocol (currently 5 items, items 1-5).

- [x] Insert a new step 3 between the existing step 2 ("If fulfilling, dispatch via `cafleet member exec`") and the existing step 3 ("`member exec` mechanics"). Title it "After dispatch, ping the member." <!-- completed: 2026-05-01T00:20 -->
- [x] In the new step 3, name the `cafleet member ping` invocation with literal `--agent-id` / `--member-id` flags and reference `tmux.send_poll_trigger` (`cafleet/src/cafleet/tmux.py:83`) once. <!-- completed: 2026-05-01T00:20 -->
- [x] In the new step 3, make explicit that ping (not `message poll`) is the right primitive, that it is unconditional on exec success, and that exec-failure cases skip the ping (the 1-minute monitor is the safety net). <!-- completed: 2026-05-01T00:20 -->
- [x] Renumber the existing items: step 3 (`member exec` mechanics) → step 4, step 4 (Acknowledge the request) → step 5, step 5 (Refusing a request) → step 6. <!-- completed: 2026-05-01T00:20 -->
- [x] Verify no in-file cross-references point at the old step numbers — if any do, update them to the renumbered targets. <!-- completed: 2026-05-01T00:20 -->

### Step 3: Document the chain in skills/cafleet-monitoring/SKILL.md

Target table: the escalation table inside `## Stall Response`. The table currently has a `cafleet ... member ping ...` row documenting the "stalled despite a recent message send" use case.

- [ ] Locate the existing `cafleet ... member ping ...` row in the escalation table. <!-- completed: -->
- [ ] Extend the "When to use" cell of that row to enumerate two use cases: (a) the existing post-message-send recovery (member appears stalled despite a recent `message send`, or after a long idle window), and (b) the new post-exec chain (Director MUST follow every successful `cafleet member exec` with this ping). For (b), cross-reference `Skill(cafleet)` § Member Exec for the chain definition rather than duplicating wording. <!-- completed: -->
- [ ] Do NOT add a second ping row. Do NOT add a separate cell-level note in the `member exec` row beyond a short pointer phrase such as "see ping row for the required follow-up". <!-- completed: -->

### Step 4: Live verification

End-to-end live verification is required so the chain rule is observed working in a real session, not just asserted in docs. A Director (or a member acting as Director within its own team) issues exec then ping against a live member and confirms the member begins its next turn without waiting for the 1-minute monitoring tick.

- [ ] In an existing CAFleet session, run `cafleet --session-id <session-id> member exec --agent-id <director-agent-id> --member-id <test-member-id> "echo hello"` against a live member that is at the Claude Code input prompt. <!-- completed: -->
- [ ] Immediately follow with `cafleet --session-id <session-id> member ping --agent-id <director-agent-id> --member-id <test-member-id>` against the same member. <!-- completed: -->
- [ ] Capture the member's pane via `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <test-member-id> --lines 60` and confirm the member began its next turn within seconds — NOT waiting for the 1-minute monitoring tick. <!-- completed: -->
- [ ] Record the verification outcome (timestamp, member id, captured first-turn snippet) in the Verification log subsection below. <!-- completed: -->

#### Verification log

| Date (UTC) | Member id | First-turn snippet | Notes |
|---|---|---|---|
| _pending_ | _pending_ | _pending_ | _pending_ |

### Step 5: Cross-document consistency check

- [ ] Re-read `skills/cafleet/SKILL.md`, `skills/cafleet/roles/director.md`, and `skills/cafleet-monitoring/SKILL.md` end-to-end and confirm the chain rule is consistent across them — same primitive name (`cafleet member ping`), same conditional (after every successful `member exec`), same skip rule (skip on exec exit-non-zero), same ping-failure handling (non-fatal warning). <!-- completed: -->
- [ ] Confirm no surface mentions `cafleet message poll` (or any abbreviation thereof) as the chain primitive — the miscopy guard. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-30 | Initial draft |
