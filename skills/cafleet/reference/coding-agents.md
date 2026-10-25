# Coding agents

Read your executing backend before using host tools. Explicit backend/model/effort requests govern spawning; otherwise use the selected backend's configured defaults. Use CLI help for supported backend/effort choices and forward exact model tokens. [Director procedure](../roles/director.md) owns receiver recovery and authorization.

## claude

Claude Code loads skills through the Skill tool and reads referenced roles normally. Members launch with `--permission-mode dontAsk`, a broker allow rule and `--name`. Host deny/ask rules and protected paths remain authoritative; route required denied operations through the Director.

The Director uses the host's recorded user-decision tool for consequential questions. Discover and authorize Monitor independently of Bash rules, then start one owned task with literal IDs:

```json
{"command":"cafleet message listen 2 --timeout 210","description":"CAFleet Director inbox","timeout_ms":240000}
```

Require the stdout ready record. Retain task ID, fleet/member IDs and deferred actions. Monitor events may start model turns before process exit; consume complete records, reconcile durable IDs, ACK consumption and keep the task active while work remains. An event supplies no answer to an open user decision.

On closure/completion establish source-child exit, then renew only when authorized work remains. Deduplicate closure/completion by owned task ID. A host timeout requires observed child cleanup before replacement. Use verified cancellation or drain the finite lease. Preserve supplied goal state and budget; receive/ACK/renewal use no goal lifecycle calls.

Where Monitor is unavailable or denied, state that reception limit and use manual poll/finite wait. Ordinary background Bash completion notification alone establishes no per-line reception. Validate actual expiry/cancellation and child cleanup in the current host.

## codex

Codex loads this core, its backend section and assigned role references by absolute path. Members launch with `--ask-for-approval never --sandbox workspace-write`; host rules/sandbox remain authoritative. Relay consequential user questions through the Director.

After registration start `cafleet message listen DIRECTOR_ID --timeout 300` once with native `exec_command`, a literal ID and a short initial yield (for example 1,000 ms). Require ready and retain the returned live `session_id`.

While delegated work remains, read that same session with `write_stdin`, empty `chars` and a bounded yield (for example 10,000 ms). Accumulate partial JSONL; act on complete records, poll full broker state, reconcile replay and ACK consumption. Continue independent work and updates between reads. A tool read timeout keeps the receiver running and differs from CLI lease expiry.

Retain the session, pinned IDs, partial line and unfinished actions across compaction. Establish native process exit after closure. A clean lease may renew; unexpected failure needs a successful health/inbox check first. Use verified cancellation with observed termination or stop renewal and drain the finite lease.

Keep this tool-bearing receive loop active independently of host goals. A background process alone supplies no unsolicited per-line wake after the model turn ends; fully idle Codex reconciles on the next user/native activation. Preserve activation, pause, completion, budget and pending user decisions; receiving uses no goal lifecycle tools.

## opencode

OpenCode loads skill cores/backend/role references by absolute path. Each member runs `opencode mini --standalone --agent cafleet` with `--prompt`, a private server and optional exact `provider/model`. Effort is unsupported. Use OpenCode 2 or newer.

The preset's declared permissions are authoritative: shell defaults to deny, explicit allows follow, and specific pane-dispatch denies win by last matching rule. Assignment authority still limits permitted tools. Route required denied commands; report a permission popup to the Director.

Keep loaded configurations free of MCP servers while their preset permission enforcement is unverified, and submit one shell command per invocation. `OPENCODE_CONFIG_DIR` relocates the preset while skills remain at their fixed discovery path; same-name definitions may merge with home configuration.

A Director without a verified native stream adapter states that limitation and uses manual poll/finite wait. Mini notifications/direct prompts arrive as user turns. Captures may show only a status bar; use retained session records plus broker poll/ACK when evidence is missing.

## Native commands

For each backend, `member exec` submits visible leading-`!` shell input. The Director authorizes the actual command and inspects its output before continuation. The effective native permission boundary is host-specific; successful dispatch or harmless output establishes no second member-tool permission check.
