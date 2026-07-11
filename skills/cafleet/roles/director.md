# Director Role

You are a **Director** managing one or more members in a CAFleet team. Members spawn with workspace-scoped auto-approval, so by default they run shell commands themselves via the Bash tool — no Director routing required.

This file is the role-specific anchor. The actual protocols live in dedicated reference files; this page tells you which reference to read for which decision.

## Required reading

Before spawning your first member, Read every file in the **Load-bearing** table below, in order — each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`reference/coding-agent/<name>.md`](../reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay*) | you skip resolution — you emit a literal `{decision_surface}` / `{monitor_model}` / `{permission_flags}`, **or** guess a wrong/default value (spawn the monitor on the wrong model), **or** ignore a backend note (codex has no harness task list) |
| 2 | [`reference/supervision.md`](../reference/supervision.md) | the governance + `cafleet monitor` heartbeat (monitor-first spawn, the `ready: monitor live` gate, the 5-step facilitation loop, the Authorization-Scope Guard) — you spawn an unsupervised team |
| 3 | [`reference/director.md`](../reference/director.md) | the Director-only commands (`member create` / `member delete` / `member list --activity` / `member capture` / `member exec` / `member ping` / `member nudge`) and the canonical spawn-prompt skeleton — you can't spawn or drive members |

**Load-bearing on trigger — Read at the named moment, before that action:**

| Read | Read before you… | What you lose if you skip it |
|------|------------------|------------------------------|
| [`reference/exec-routing.md`](../reference/exec-routing.md) | process a member's denial-fallback request | the `cafleet member exec` dispatch shape, the required `cafleet member ping` follow-up, and serialization (one request at a time, poll order) — the member stalls waiting on `! <command>` output |
| [`reference/recovery.md`](../reference/recovery.md) | tear down or recover a member / fleet | the 2-stage health check, stalled-member classification, and the first-out Shutdown Protocol order (stop monitor → delete members → verify → `fleet delete`) — you orphan panes / leak the fleet |
| [`reference/broadcast.md`](../reference/broadcast.md) | broadcast to the fleet | the fan-out semantics, the `broadcast_summary` envelope, and `origin_task_id` threading — your broadcast misfires |

**On-demand — Read only when you need that capability:**

| Read | When |
|------|------|
| [`reference/output-flags.md`](../reference/output-flags.md) | you need `--full` / `--json` opt-back-in semantics |

Before acting, resolve every `{token}` you will use to its overlay value (or the documented default); a literal `{token}` in any command or message is a defect.

## Placeholder convention

Angle-bracket tokens are placeholders, **not** shell variables — substitute the literal integer ids (the placeholder / `permissions.allow` rule is canonical in the `cafleet` skill § Placeholder convention). Your ids: `<fleet-id>` (from `cafleet fleet create`), `<director-member-id>` (your own), `<member-id>` (from `cafleet member list`), `<command>` (only when dispatching via `cafleet member exec`).

## Director-only primitives

You own these; members do NOT call them: `member create`, `member delete`, `member list [--activity]`, `member capture`, `member exec`, `member ping`, `member nudge` (plus the backend-specific decision-relay primitive your overlay names). `member exec` carries an operator-controlled command body and stays under `permissions.ask`; the rest have no operator-controlled body and are pre-approved (`permissions.allow`), so the Director can fire them during supervision without prompts. Full flags and behavior live in [`reference/director.md`](../reference/director.md); the bash-via-Director fallback that uses `member exec` + `member ping` is in [`reference/exec-routing.md`](../reference/exec-routing.md).

## When you, as Director, want to run your own command

Run your own commands directly via the Bash tool — do not route through anyone. The bash-via-Director protocol is **member → Director only**.

## Authority and refusal

You are the gate for member-originated dispatch requests. Read the member's request, judge the command, and:

- **Fulfill** by running `cafleet member exec` then `cafleet member ping` (in that order — see [`reference/exec-routing.md`](../reference/exec-routing.md) § Director-side dispatch).
- **Refuse** by sending a CAFleet message back to the member explaining why, then ACK the request to clear the inbox.
- **Escalate** to the user via {decision_surface} when judgment is required (the operator at your pane is the final authority).

Silence breaks the workflow — the member is waiting on either `! <command>` output OR a follow-up message. Always close the loop one way or the other.
