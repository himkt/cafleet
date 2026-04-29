# Bash Tool — Member Behavior

This rule fires every time you reach for the Bash tool as a CAFleet team member. Read it before invoking Bash, before emitting any text that looks like a command result, and before responding to any "run X" request.

## The MUST rule

> **If you are a CAFleet member spawned by `cafleet member create`, your harness runs in `--permission-mode dontAsk`. Your Bash tool is ENABLED and permission prompts auto-resolve silently. Run cafleet (and any other shell command) directly via the Bash tool. No prefix, no Director routing, no operator prompts.**

## How to detect that you are a CAFleet member

Any of the following signals means you are a member subject to this rule:

- Your spawn prompt names a Director / `director_agent_id` / refers to you as a "member" / "teammate" of a CAFleet team.
- The status line at the bottom of your pane shows `⏵⏵ don't ask on`.
- Your spawn prompt instructs you to wait for the Director's instructions via `cafleet ... message poll`.

## What you do when asked to run a command

1. **Run it.** Use the Bash tool directly. Permission prompts auto-resolve. No prefix, no routing.
2. **Process the output** as your next-turn context.
3. **Reply to the Director** via `cafleet --session-id <s> message send --agent-id <my-id> --to <director-id> --text "..."` if a reply is expected.

You take a single action — invoke Bash — instead of the previous "send a CAFleet message and wait" routing.

## What you MUST NOT do

- **Do not emit `<bash-input>...</bash-input>` markup, fenced ```bash result blocks, or any text that *looks* like a command result without actually running the command.** That markup is meaningful only when produced by the harness; emitting it yourself produces no execution and **misleads the operator into thinking the command ran**.
- **Do not fabricate output.** If you have not actually run the command and seen real output, you do not know the result. Say so plainly.
- **Do not "just answer from prior context"** when a fresh command result is needed. Working-tree state changes between turns; only a real execution gives a true answer.
- **Do not refuse to run a command on the assumption that Bash is denied.** Bash is enabled under dontAsk. Try it.
- **Do not route a denied command to the Director without first reconsidering it.** Most denials happen because the command is wrong (typo, wrong flag, wrong path) or unnecessary altogether. Fix the command yourself before asking anyone else to run it.

## When your Bash tool denies a command — reconsider, then auto-route to the Director

`dontAsk` mode auto-resolves most permission prompts, but the Claude Code harness has its own deny list above dontAsk for destructive operations (e.g. `git push`, `rm -rf`). When the harness denies a Bash invocation:

1. **Reconsider the command first.** Re-examine what you tried. Is the flag right? Is the path right? Did you really need to run it at all? In most cases the right answer is to fix the command and try again, or to skip it entirely because you already have the answer.
2. **Only when the command is genuinely correct AND genuinely needed AND the harness still denies it**, take the routing action below.

> **When you do route, you MUST do so automatically. You MUST NOT ask the operator for a routing decision. You MUST NOT offer the operator alternative paths. You MUST NOT propose `!` prefix workarounds for the operator to run themselves.**

The operator has already asked you to run the command. The Director is your fallback when your harness can't run it. Take the single action:

```bash
cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> \
  --text "Please run \`<command>\` for me — my Bash tool denied it (<denial reason if known>)."
```

Then **wait** for the `! <command>` output to land in your pane. The Director will dispatch the command via `cafleet member send-input --bash <command>`, which keystrokes `! <command>` + Enter into your pane via Claude Code's `!` shortcut. The captured stdout/stderr lands in your next-turn context.

If the `cafleet message send` itself is also denied by the harness, surface that to the operator as a single fact ("my Bash and my cafleet are both denied; please dispatch via `cafleet member send-input --bash <command>` from your Director pane") — but that is the **only** time you ask the operator for help. Anything else, route through the Director silently.

### Why no operator-prompts-for-routing

When you offer the operator a list of options ("(1) you run it via `!`, (2) route through Director, (3) skip"), you push a routing decision back to the operator that they already implicitly answered by asking you to run the command. They wanted it run. Routing is implementation. The bash-via-Director protocol exists precisely to handle the harness-denied case without operator interaction.

## If `<session-id>` / `<my-agent-id>` / `<director-agent-id>` are missing

The harness injects these into your spawn prompt. They should already be in your context. If they are genuinely missing, say so explicitly and ask the operator. Do **not** guess UUIDs.

## Why this rule is associated with the Bash tool

The Bash tool is the entry point for every shell-execution request. If you can pause at the moment of "I want to run a command" and check this rule, you cannot fall into the failure modes of fake markup, silent stalling, or unnecessary Director-routing. **Treat this rule as a precondition for every Bash invocation.**

## Director side (for completeness)

If you are the **Director** (not a member), this rule applies in reverse only when a member auto-routes a denied command to you. In that case, dispatch the requested command via:

```bash
cafleet --session-id <session-id> member send-input \
  --agent-id <director-agent-id> --member-id <member-agent-id> \
  --bash "<command>"
```

See `skills/cafleet/SKILL.md` § Routing Bash via the Director for the full protocol, serialization rules, and cross-Director boundary.
