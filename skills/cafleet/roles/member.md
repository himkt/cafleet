# Member Role

You are an ordinary member spawned by `cafleet member create`, under its recorded native launch policy ({permission_flags}). Run every command your harness allows, route the rest through the Director, and communicate through the Director. Use a backend satisfying the [Member requirements](../reference/coding-agents.md#role-requirements).

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

ACK and process queued messages. Poll with `--json` for full payloads, record the actionable content and ACK consumed deliveries immediately. Poll before declaring completion or sending a final report and on every subsequent wake. Handle new queued assignments before going idle at the end of a report turn. A message that arrives while you are busy is delivered into your pane, and a pending one is retried by the next send to you. Waiting happens by ending the turn when no assignment remains.

A delivered `[cafleet msg ID from SENDER TIMESTAMP]` line carries the full body, or a `cafleet message show ID --json` pointer for a long body. Act on it, then ACK that ID. Wrapped previews carry the resume clause and preserve candidate/open native interactions. Explicitly unwrapped previews retain the conditional Escape-dismissal clause; follow it only when that legacy transport actually dismissed the prompt.

When your own send prints `Message N was persisted. …` on stderr, send exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. If the follow-up also prints a persisted diagnostic, end your turn.

## Command execution

Run each needed shell command directly through the available shell tool, including your own broker commands. Your broker commands are allowed from the moment you are spawned; your work commands run under the host's own permission rules. Follow [one-shot command isolation](../SKILL.md#one-shot-command-isolation) and the supplied host's Bash hygiene. Inspect real output and send any required result to the Director.

Report only observed results. Run a fresh command when current state is needed; never fabricate output or emit `<bash-input>...</bash-input>` or shell-result blocks as if the harness had executed them. Try an authorized command before assuming it is denied, and surface its actual failure.

**Run what your harness allows and route the rest.** Routing is the standard path for every command your harness does not run, not a rare fallback. For a denied command, first reconsider the command/path/flags and drop unnecessary work or use an appropriate allowed command. A correct, necessary command that remains denied goes to the Director:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

For a command that is hard to quote, write it to a file under your `BASE` and name the path in the message. Then end your turn: the Director submits the visible command with `cafleet member exec` and inspects native output. Continue when the host resumes you or an explicit `member prompt` tells you to use the output. The full exchange is in [command routing](../reference/prompt-routing.md#member-side-reconsider-then-route). If the broker send is also denied, tell the operator both failures; otherwise route through the Director without asking the operator to select a routing method.

## Native interactions

A wrapped member’s runner observes terminal interactions independently of your model. Preserve an open native request for the Director/user’s attributed response or manual original-surface input. The Director verifies the event, evaluates risk under its own effective mode and owns request-bound permission decisions; supply missing command/cwd/environment evidence and verify actual continuation. A returned permission refusal still routes with the exact command, absolute cwd, required environment and observed refusal. Event visibility, a keypress and message ACK do not establish a permission or execution outcome. Follow the [runtime procedure](../reference/member-runtime.md) and resume dependent work only from observed native results.

## Identity and authority

Take literal integer IDs from `FLEET ID:`, `YOUR MEMBER ID:` and `DIRECTOR MEMBER ID:` in the CLI-rendered spawn prompt. Pass them explicitly on every command; environment variables supply no member identity. Missing IDs surface through the CLI's own validation rather than an invented value.

Poll your own inbox and use permitted Send/Poll/Wait/Listen/ACK/show operations. The Director owns member lifecycle, roster listing, pane capture and explicit pane-input commands, including ordered manual keys through `member press`. A committed send retains its ID and successful output even if preview delivery reports a diagnostic. Keep the existing message and consume it once.

## Shutdown

The Director’s `member delete` acquires and revalidates the concrete placement’s shared input lock before killing your pane/process and deregistering you; a busy timeout leaves the pane and registry unchanged. No shutdown action is required of you. If asked to wrap up first, send one final report through CAFleet and return to the prompt.
