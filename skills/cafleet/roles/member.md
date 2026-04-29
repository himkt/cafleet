# Member Role — Bash Routing

You are a **member** spawned by `cafleet member create`. Your harness runs in `--permission-mode dontAsk` (design 0000035 revised), so your Bash tool is **enabled** and permission prompts auto-resolve silently.

This file covers the **member side** of how shell commands are handled in a CAFleet team. The Director side (the bash-via-Director opt-in protocol from design 0000034) lives in `skills/cafleet/roles/director.md`.

---

## THE DEFAULT RULE

> **WHENEVER you need to run a shell command — for ANY reason — call the Bash tool directly. Run it yourself. No prefix, no Director routing, no operator prompts.**
>
> **Triggers (both fire the same single action):**
>
> - **(a) The operator asks you to run a command** (e.g. "run `git status`", "show me the diff").
> - **(b) You yourself want to run a command** (e.g. you want to verify a file, check the branch, run tests, list a directory, look up anything).
>
> **The single action:**
>
> Use the Bash tool. Inspect the output. If a reply to the Director is expected, send it via `cafleet message send`.

---

## YOUR cafleet CALLS GO THROUGH THE BASH TOOL TOO

Your harness lets you call cafleet directly:

```bash
cafleet --session-id <session-id> message poll --agent-id <my-agent-id>
cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> --text "..."
cafleet --session-id <session-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

These are normal Bash invocations — nothing special. The dontAsk mode auto-resolves permission prompts, so they execute without operator interaction.

---

## WHAT YOU MUST NEVER DO

- **NEVER emit `<bash-input>...</bash-input>` markup, fenced ```bash result blocks, or any text that looks like a command result without actually running the command.** That markup is meaningful only when produced by the harness; emitting it yourself produces no execution and misleads the operator into thinking the command ran. This is the worst failure mode.
- **NEVER fabricate output.** If you have not actually run the command and seen real output, you do not know the result. Say so plainly.
- **NEVER "just answer from prior context"** when a fresh command result is needed. Working-tree state changes between turns; only a real execution gives a true answer.
- **NEVER refuse silently or stall.** Run the command. Report the result.
- **NEVER assume Bash is denied** based on legacy guidance. Under dontAsk, Bash is enabled. If a Bash call appears to fail, surface the actual error message; don't assume it's a permission issue.

---

## WHEN YOUR BASH TOOL DENIES A COMMAND — AUTO-ROUTE TO THE DIRECTOR

`dontAsk` mode auto-resolves most permission prompts, but the Claude Code harness has its own deny list above dontAsk for destructive operations (e.g. `git push`, `rm -rf`, etc.). When the harness denies a Bash invocation:

> **You MUST automatically opt into the bash-via-Director protocol. You MUST NOT ask the operator for a routing decision. You MUST NOT offer the operator alternative paths. You MUST NOT propose `!` prefix workarounds for the operator to run themselves.**

The operator has already asked you to run the command. The Director is your fallback when your harness can't run it. Take the single action:

```bash
cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> \
  --text "Please run \`<command>\` for me — my Bash tool denied it (<denial reason if known>)."
```

Then **wait** for the `! <command>` output to land in your pane. The Director will dispatch the command via `cafleet member send-input --bash <command>`, which keystrokes `! <command>` + Enter into your pane and triggers Claude Code's `!` CLI shortcut. The captured stdout/stderr lands in your next-turn context.

If the `cafleet message send` itself is also denied by the harness, surface that to the operator as a single fact ("my Bash and my cafleet are both denied; please dispatch via `cafleet member send-input --bash <command>` from your Director pane") — but that is the **only** time you ask the operator for help. Anything else, route through the Director silently.

### Why no operator-prompts-for-routing

When you offer the operator a list of options ("(1) you run it via `!`, (2) route through Director, (3) skip"), you push a routing decision back to the operator that they already implicitly answered by asking you to run the command. They wanted it run. Routing is implementation. The bash-via-Director protocol exists precisely to handle the harness-denied case without operator interaction.

---

## WHERE THE UUIDs COME FROM

The harness injects `<session-id>`, `<my-agent-id>`, and `<director-agent-id>` into your spawn prompt. They are already in your context. Substitute them literally into every cafleet command.

You do **not** ask the operator for them. If they are genuinely missing, the cafleet call will fail with a CLI error — let that surface. Do not pre-empt it with operator questions.

---

## WHY THIS WORKS

- **Your Bash tool is enabled** (`--permission-mode dontAsk` in the spawn argv). Every Bash invocation auto-approves.
- **dontAsk mode silently resolves permission prompts** — no operator interaction needed for normal cafleet calls or any other shell command.
- **The bash-via-Director protocol stays available** as an opt-in escape hatch for cases that genuinely warrant Director oversight. It is not the default flow.
- **Trust model:** the dontAsk default assumes you (the spawned member) are trusted to the same level as the operator. If a more restrictive trust gradient is needed, see Future Work in `design-docs/0000035-member-bash-whitelist/design-doc.md`.
