# Coding-agent backends

Use the backend declared by the calling Director host and exact member launch requests. Omitted member backend inherits the Director's; omitted model/effort uses that backend's configured defaults. CLI help owns accepted values.

## Launch behavior

| Backend | Member launch | Permission owner |
|---|---|---|
| Claude Code | Interactive `claude`, named session, `dontAsk`, broker allow rule | Host settings and protected paths |
| Codex | `codex` with `--ask-for-approval never --sandbox workspace-write` | Host rules and sandbox |
| OpenCode | `opencode mini --standalone --agent cafleet --prompt ...` | Installed preset and effective configuration |

Model tokens pass literally. OpenCode requires a `provider/model` shape and accepts no effort. The chosen backend validates optional model/effort before effects. The selected executable/configuration and required preset are checked before registration or pane creation.

[Quickstart](../quickstart.md#configure) supplies permission/trust setup; [configuration paths](cli-options.md#config-dir-resolution) describe relocation.

## Permissions

### Claude Code

Spawn grants the broker commands members need. Matching host deny/ask rules remain authoritative. Add project task permissions and skill/file access deliberately. Native pane prompt/exec stay under Director/operator authorization; protected file edits may require the Director.

### Codex

Setup owns `rules/cafleet.rules`: it allows the CAFleet prefix and keeps pane prompt/exec under prompting. Strictest matching decisions win. Put custom rules in another file so setup can refresh its own asset. Grant the database directory and multiplexer socket access within the host's sandbox configuration.

### OpenCode

Use OpenCode 2 or newer. Setup owns the `cafleet` preset; its shell rules default to deny, then explicit allows apply and specific pane-action denies win by last match. Broad tool permission remains subject to assigned scope.

Keep loaded configurations free of MCP servers while their permission enforcement is unverified. Submit one command per shell invocation and authorize code/hooks invoked by allowed tooling. A required denied operation routes to the Director; a permission popup needs diagnosis rather than an improvised answer.

The relocated preset can merge with same-name definitions from default home configuration. Inspect effective permissions when customizing it; skills retain their fixed discovery path.

## Native reception

The Director owns one finite `message listen` source and requires ready before relying on replies. The installed CAFleet skill owns concrete host-tool recipes, partial-line handling, replay reconciliation and renewal.

| Host | Native receive behavior |
|---|---|
| Claude Code | An authorized Monitor can deliver each stdout record as a host event before process exit. Verify Monitor separately from Bash permissions. |
| Codex | Retain one execution session and read it explicitly with bounded native reads while delegated work remains. Fully idle reception resumes at the next user/native activation. |
| OpenCode | State the limitation and use manual poll/finite wait where a native stream adapter is unavailable. |

Read full durable messages, record deferred work and ACK consumption. Keep one handle/source process, observe its exit before replacement and preserve pending messages after failure. A healthy empty read retains the same receiver. Clean lease expiry may renew; unexpected failure requires a successful health/inbox check first.

Use verified cancellation with observed child cleanup or stop renewal and drain the finite lease. Receiving leaves host goal activation, pause, completion, budgets and pending user decisions authoritative. A broker event supplies no answer to an open question.

## Native shell input

The Director authorizes the actual routed command under its host rules. `member exec` submits visible leading-`!` shell input and returns dispatch metadata. Inspect actual output/process completion before explicit continuation. Partial/uncertain input requires inspection before a newly authorized retry; native visibility establishes no second member-tool permission check.

OpenCode captures may show only the status bar. Use retained host/session evidence and broker poll/ACK when a capture omits relevant work.
