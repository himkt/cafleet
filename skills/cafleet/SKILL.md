---
name: cafleet
description: Coordinate independent coding-agent sessions through CAFleet durable messages and owned multiplexer panes.
---

# CAFleet

Use the CLI for delegated research, implementation and review. Continue an existing assigned team when spawned as a member; start a fleet only when coordinating a new team.

## Load the procedure

Read your executing backend in [coding agents](reference/coding-agents.md), then your [member](roles/member.md) or [Director](roles/director.md) procedure. Claude Code uses its Skill tool; Codex and OpenCode read this core, their backend section and assigned references by absolute path. Use the host instructions and literal identity supplied in the spawn prompt.

The Director supplies explicit absolute artifact/scratch paths and scoped assignments, owns Git and user decisions, receives ready, reviews evidence and closes owned resources. Members perform their assignment and suitable checks. Substantial work receives independent review.

## Durable communication

Run each CAFleet process as the only command in its shell-tool invocation. Pass literal returned integer IDs; the subject follows the subcommand, while sender/recipient relationships use flags. Leading environment assignments may configure that one process.

```bash
cafleet message send --from-member-id 3 --to-member-id 2 "Completed the parser change; focused checks passed."
```

```bash
cafleet message poll 3 --json
```

```bash
cafleet message ack 42
```

Send assignments, findings, blockers and evidence directly. Name artifact paths for long reports; send long/multiline bodies with `--file /absolute/path`. Read full JSON bodies, record deferred actions in host-owned task state and ACK consumption. ACK means the message was read; assignment completion remains separately tracked.

Poll after ready, on every wake and before reporting completion. Reconcile known/replayed IDs before acting. Consume queued assignments before becoming idle. Read a long notification pointer with `cafleet message show ID --json`.

Sending commits the durable row before attempting preview transport. If stderr says `Message N was persisted.`, retain that ID and send exactly one follow-up to the same recipient: `message N was not delivered to your pane; poll your inbox`. If that follow-up has the same diagnostic, end the turn and preserve both durable records.

## Permissions and evidence

Execute host-permitted commands directly. Reconsider a denied command's path/flags or a suitable permitted alternative, then send the exact necessary command and observed denial to the Director. A denied file edit needs a matching host rule or a Director edit.

The Director authorizes native pane input under its host rules. `member exec` submits visible `! COMMAND`; inspect actual output and process completion before continuing. Failed or uncertain submission requires inspection before a newly authorized retry. Tool availability supplies no additional assignment or Git authority.

Carry existing user authorization forward. Relay consequential missing information through the Director, preserve prior answers and honor stop requests. Broker events leave pending user decisions and host goal controls intact.

## Runtime reference

The installed [operator pages](reference/runtime/quickstart.md) explain setup and recovery. Use `cafleet --help` and subcommand help as the command/option registry; see [CLI behavior](reference/runtime/spec/cli-options.md), [fleet operation](reference/runtime/how-to/mixed-backend-team.md) and [message records](reference/runtime/spec/message-envelope.md).
