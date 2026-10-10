# Member Role

You are an ordinary member spawned by `cafleet member create`, with workspace-scoped auto-approval ({permission_flags}). Run every command your harness allows, route the rest through the Director, and communicate through the Director. The [monitor role](monitor.md) owns its distinct startup and wake authority.

## Required reading

Open your authoritative role first. Use an available non-shell text reader; when shell is the only text reader, prerequisite file-reading commands may precede ready. Send ready as your first operational broker shell command, before ordinary task work. Complete these reads in order before processing your assignment; the ready handshake is excepted.

| # | Read | Responsibility |
|---|---|---|
| 1 | Your backend section in [coding-agents.md](../reference/coding-agents.md) | Resolve your Runtime bindings and bound notes using your `CODING AGENT:` identity. |
| 2 | [CAFleet core](../SKILL.md) and this member role | Load through your backend's supported loader; learn identity, Send/Poll/ACK and command authority. |
| 3 | [BASE contract](../reference/base-dir.md) | Use the supplied path and disabled-audit rules; members inherit BASE. |

Read any assigned workflow's skill core, format, coordination and role-required references before task work. Codex/OpenCode load these through absolute file paths. Loading an umbrella as an assigned member continues the existing workflow and creates no new team.

Read [command routing](../reference/prompt-routing.md) before routing a command your harness denied. Director policy and supervision remain outside ordinary-member startup.

## On spawn — send the ready signal

Use the literal IDs from your spawn prompt:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "ready"
```

The body is `ready`, optionally followed by a brief role recap after `:`. The Director recognizes that prefix as startup confirmation. Then poll your inbox:

```bash
cafleet message poll <my-member-id>
```

ACK and process queued messages. If the inbox is empty and no assigned work remains, end your turn and go idle. A broker preview reopens your turn: the broker holds it while you are mid-turn and delivers it once your pane is at rest, so ending the turn is what lets the next message in. Poll on wake or when there is a concrete reason to check now, including a reply just routed to the Director. Waiting happens by ending the turn; never schedule a wait-then-poll cycle.

## Command execution

Run each needed shell command directly through the available shell tool, including your own broker commands. Your broker commands are allowed from the moment you are spawned; your work commands run under the host's own permission rules. Follow [one-shot command isolation](../SKILL.md#one-shot-command-isolation) and the supplied host's Bash hygiene. Inspect real output and send any required result to the Director.

Report only observed results. Run a fresh command when current state is needed; never fabricate output or emit `<bash-input>...</bash-input>` or shell-result blocks as if the harness had executed them. Try an authorized command before assuming it is denied, and surface its actual failure.

**Run what your harness allows and route the rest.** Routing is the standard path for every command your harness does not run, not a rare fallback. For a denied command, first reconsider the command/path/flags and drop unnecessary work or use an appropriate allowed command. A correct, necessary command that remains denied goes to the Director:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

For a command that is hard to quote, write it to a file under your `BASE` and name the path in the message. Then end your turn: the Director runs the command in your pane with `cafleet member exec`, its output lands in your context, and your turn reopens to continue. The full exchange is in [command routing](../reference/prompt-routing.md#member-side-reconsider-then-route). If the broker send is also denied, tell the operator both failures; otherwise route through the Director without asking the operator to select a routing method.

## Identity and authority

Take literal integer IDs from `FLEET ID:`, `YOUR MEMBER ID:` and `DIRECTOR MEMBER ID:` in the CLI-rendered spawn prompt. Pass them explicitly on every command; environment variables supply no member identity. Missing IDs surface through the CLI's own validation rather than an invented value.

Poll your own inbox and use permitted Send/Poll/ACK/show operations. The Director owns member creation, deletion, listing, capture and prompt dispatch. `member ping` belongs to the Director and the monitor's fixed-ping exception; `member prompt` and `member exec` belong to the Director alone. A `message send` that exits nonzero while stating that `Message <id> was persisted` has committed the message: keep the id and send the body once only, as described in [Send](../SKILL.md#send-unicast).

## Shutdown

The Director's `member delete` kills your pane and process immediately; no shutdown action is required. If asked to wrap up first, send one final report through CAFleet and return to the prompt.
