---
name: cafleet
description: >-
  Interact with the CAFleet message broker and supervise CAFleet member teams.
  Use when an agent needs to register as a member, send/receive messages, poll
  inbox, acknowledge messages, or discover other members; or when a Director is
  about to spawn, monitor, health-check, or recover a stalled team of CAFleet
  members (any `cafleet member create`), which requires the dedicated monitoring
  member, the heartbeat, and the supervision governance.
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as a member, send and receive messages, and discover other members on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

## Required reading

Before your first action other than these Reads, Read every file in the **Load-bearing** table below, in order (row #2 applies only if you write files) — each carries a protocol you cannot reconstruct from this page. Identify your coding agent first: your spawn prompt's `CODING AGENT:` line names it; a standalone agent uses its own identity. After reading your overlay, **resolve** it before acting — see *Resolve your overlay* below.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`reference/coding-agent/<name>-overlay.md`](reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay*) | you skip resolution — you emit a literal `{monitor_model}` / `{permission_flags}`, **or** guess a wrong/default value (spawn the monitor on the wrong model), **or** ignore a backend note (codex has no harness task list) |
| 2 | [`reference/base-dir.md`](reference/base-dir.md) — if you write any scratch / audit / figure file | the no-bypass write protocol and the `<unset>` contract — you mis-root every write or fall back to `/tmp` |

**Load-bearing on trigger — Read at the named moment, before that action:**

| Read | Read before you… | What you lose if you skip it |
|------|------------------|------------------------------|
| [`reference/exec-routing.md`](reference/exec-routing.md) | route a Bash-denied command to the Director | the dispatch shape — you stall or fabricate command output |
| [`reference/recovery.md`](reference/recovery.md) | tear down or recover a member / fleet (also the Shutdown Protocol) | the first-out teardown order — you orphan panes / leak the fleet |
| [`reference/cli.md`](reference/cli.md) § *Broadcast* | broadcast to the fleet or thread via `origin_message_id` | the broadcast send/ack semantics — your fan-out misfires or double-acks |

**On-demand — Read only when you need that capability:**

| Read | When |
|------|------|
| [`reference/cli.md`](reference/cli.md) | you need a CLI subcommand beyond send/poll/ack — global options, output flags (`--full` / `--json` opt-back-in semantics, `CAFLEET_MAX_TEXT_LEN`), coding-agent backends, `message cancel` / `show` / `broadcast`, `member show` / `member list`, `doctor`, `fleet delete`, the typical workflow |

Director-only governance — [`reference/supervision.md`](reference/supervision.md) (governance + the `cafleet monitor` heartbeat) and [`reference/director.md`](reference/director.md) (`member create` / `member delete` / `member list` / `member capture` / `member exec` / `member ping`) — is load-bearing for a Director; its gated Required-reading block lives in [`roles/director.md`](roles/director.md), not on this dispatch surface.

Exhaustive per-subcommand flags, exit codes, and error strings live in [`docs/spec/cli-options.md`](../../docs/spec/cli-options.md).

## Resolve your overlay

You have read your overlay (Required-reading row #1). Before your first action, resolve it:

1. **Materialize values.** For every `{placeholder}` token you will use this session, take the concrete value from your overlay's table and use that literal value — never the brace token. Resolution order for each token: (i) your overlay's value; (ii) the documented default below, only if your overlay omits the token or you cannot identify your backend. Never a literal `{token}`, never an ad-hoc guess.
2. **Apply notes.** When you reach a base instruction named in your overlay's *Note → applies at* table, follow that note's caveat there (e.g. on codex, coordinate via cafleet messages, not a harness task list; on opencode, treat a permission popup as a regression to escalate, not a decision point).
3. **Self-check at emission.** A literal `{token}` in any command you run, any message you send, or anything you show the user is a defect — stop and resolve it before emitting.

### Documented defaults

Used only when your overlay omits a token or your backend is unknown. Each default is the correct neutral-floor behavior — the form that functions on every backend — not a guess.

| Token | Documented default (overlay silent / backend unknown) |
|-------|-------------------------------------------------------|
| `{decision_surface}` | a Director-relayed operator message (a member always routes to the Director) |
| `{monitor_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly cost-suboptimal |
| `{reviewer_model}` | the spawning Director's own model (inherit the parent) — a safe floor, possibly intelligence-suboptimal |
| `{permission_flags}` | describe the mode neutrally as "workspace-scoped auto-approval" — for prose uses only; spawn-flag construction never falls here |
| `{bg_run}` | a backgrounded `!` shell command |
| `{bg_stop}` | killing the recorded background process |
| `{task_coord}` | cafleet messages |
| `{pane_title}` | no `--name` analog |
| `{skill_loader}` | reading the skill's `SKILL.md` + your overlay by absolute path |

## Required Flags

Every `cafleet` invocation that touches members or messages is **fleet-scoped** — it carries `--fleet-id` — and most additionally carry a member-identity flag with exactly one meaning per spelling:

- `--fleet-id <int>` — per-subcommand (placed **after** the subcommand name), required on every `member *` / `message *` / `monitor *` subcommand plus `fleet show` / `fleet delete`. There is **no environment default**; a missing value exits 1 with the shared callback error naming `cafleet fleet create`. Rejected with `No such option` on `setup` / `fleet create` / `fleet list` / `server` / `doctor`.
- `--member-id <int>` — **the member in question**: the requester on `message poll` / `ack` / `cancel` / `show`, the target on `member delete` / `show` / `capture` / `exec` / `ping`, and the member whose schedule is shown/edited on `monitor config`.
- `--from-member-id <int>` / `--to-member-id <int>` — the two parties of a two-party command: the **sender** and the **recipient** on `message send`; `message broadcast` takes the sender only. (`member create` takes **no** identity flag — the Director is auto-resolved from the fleet row; `member list`, `monitor start` / `monitor status`, and the `fleet *` commands take none either.)

In the Director's own commands, substitute the literal ids printed by `cafleet fleet create` / `cafleet member create` — never your own exported shell variables. `permissions.allow` matches Bash invocations as fixed strings, so an ad-hoc `export FLEET_ID=…; --fleet-id $FLEET_ID` breaks the match and forces prompts. See [`cli-options.md`](../../docs/spec/cli-options.md#fleet-id) for the rationale and [`permissions.allow` coverage](../../docs/spec/cli-options.md#permissionsallow-coverage) for the pattern set.

### Spawned-member identity via `str.format` substitution

`cafleet member create` runs `str.format` over the resolved spawn prompt (supplied via exactly one of `--text` / `--text-file`), rendering exactly four placeholders to literals at spawn time:

- `{fleet_id}` — the member's fleet id.
- `{member_id}` — the member's **own** newly-allocated id (the CLI allocates it during the spawn and substitutes it itself — the Director never needs to know it).
- `{director_member_id}` — the member's Director id.
- `{coding_agent}` — the resolved backend name (`claude` / `codex` / `opencode`).

An author writes the spawn prompt with those brace placeholders; after spawn the member reads its identity as literal labeled lines (e.g. `FLEET ID: 24`, `YOUR MEMBER ID: 88`). **Any literal brace in prompt text must be doubled** (`{{` / `}}`) to survive `.format()`; an unknown placeholder or a malformed brace expression is a `UsageError` (exit 2). No identity environment variable is injected into the pane — the member takes the literal ids from its prompt and passes them explicitly: a poll is `cafleet message poll --fleet-id 24 --member-id 88`; a self-attributed send is `cafleet message send --fleet-id 24 --from-member-id 88 --to-member-id <director-member-id> --text "..."`.

CLI environment variables (the `CAFLEET_`-prefixed `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`, `CAFLEET_MAX_TEXT_LEN`) are catalogued in [`reference/cli.md`](reference/cli.md) § Environment variables.

## Team supervision

When a Director spawns a team, the **FIRST** member created is the dedicated monitoring member (`cafleet member create --role monitor --model {monitor_model} --text-file <rendered monitor prompt>`). It owns the heartbeat and gates every ordinary `member create` behind its `ready: monitor live` handshake. The Director never runs `cafleet monitor start` itself.

For the full governance + heartbeat mechanism (Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, Stall Response, Cleanup, the 5-step facilitation loop, Monitor Lifecycle), Read [`reference/supervision.md`](reference/supervision.md).

For the monitoring member's own role definition (startup, on-wake routine, teardown), Read [`roles/monitor.md`](roles/monitor.md).

## Placeholder convention

In every example, substitute the literal integer ids printed by `cafleet fleet create` / `cafleet member create`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<fleet-id>` — the fleet id printed by `cafleet fleet create`
- `<my-member-id>` — your own id, read from the literal `YOUR MEMBER ID:` line in your spawn prompt
- `<director-member-id>` — the Director's id (in your spawn prompt if you are a member)
- `<target-member-id>` — the recipient of a unicast message
- `<message-id>` — the message id printed by `message poll` / `message send`

Every id input (`--fleet-id`, `--member-id`, `--from-member-id`, `--to-member-id`, `--message-id`) is a DB-assigned integer (typically 1–4 digits), passed in full — no prefix resolution. A non-integer fails with Click's standard not-a-valid-integer error (exit 2).

## Soliciting user reactions

When you need a recorded user reaction — **approve**, **choose among options**, **confirm**, or **continue-or-abort** — solicit it through {decision_surface}, never in free-form prose ("let me know if this looks good", "shall I proceed?", "reply with your choice") which records no answer and routinely stalls. A fleet **member** never talks to the user: it sends its question to the Director via `cafleet message send`, and the Director relays it. See your overlay for the question shapes and any surface constraints.

## Send (Unicast)

```bash
cafleet message send --fleet-id <fleet-id> --from-member-id <my-member-id> \
  --to-member-id <target-member-id> --text "Did the API schema change?"
```

`--to-member-id` (recipient id) is required, plus exactly one of `--text` (inline body) or `--text-file <path>` (a UTF-8 file, or `-` for stdin — use it for long or multi-line bodies that would exceed the shell's `ARG_MAX`); the delivered body is truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` in the inline preview by default. `--full` per [`reference/cli.md`](reference/cli.md) § *Output flags*. After persisting, the broker keystrokes a 2-line inline preview into the recipient's pane — an `Esc`-safeguarded auto-fire the recipient consumes as a fresh user-turn (the same path serves `message broadcast`), caught on the next manual `message poll` or a Director `cafleet member ping` if missed; full mechanics in [`multiplexer-backends.md`](../../docs/spec/multiplexer-backends.md#push-notifications).

## Poll (Check Inbox)

Returns only un-acked (`input_required`) deliveries addressed to this member, newest first; ACKing one drops it from `poll` output. `--full` emits the untruncated typed-column envelope. Poll is an on-demand inbox check — run it on wake or when you have a reason to check now, never on a self-scheduled `sleep`-timer loop; the broker re-opens your turn when work arrives.

```bash
cafleet message poll --fleet-id <fleet-id> --member-id <my-member-id> [--full]
```

## Acknowledge (ACK)

Moves a message from `input_required` to `completed`. `--message-id` required.

```bash
cafleet message ack --fleet-id <fleet-id> --member-id <my-member-id> --message-id <message-id>
```
