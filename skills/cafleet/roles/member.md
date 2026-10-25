# Member

Read the [core](../SKILL.md), your [backend](../reference/coding-agents.md) and supplied host instructions. Use the literal member/Director/fleet IDs and absolute artifact paths from your prompt.

Send `ready` as your first operational broker command; prerequisite reads may precede it when shell is the only reader:

```bash
cafleet message send --from-member-id 3 --to-member-id 2 "ready"
```

Poll your inbox with `--json`, read full assignments and ACK consumed deliveries. Perform the assigned scope and suitable checks; report changed behavior, evidence, limitations and artifact paths directly to the Director. Escalate consequential ambiguity or blockers immediately.

Poll after ready, on every wake and before reporting completion. Handle queued work before becoming idle; end your turn when no assignment remains. A delivered notification resumes interrupted work. Reissue a needed tool call if its prompt was dismissed by notification Escape.

Run each permitted command directly as one shell invocation. For a required denied command, reconsider its spelling or a permitted alternative, then send the exact command and real denial to the Director and wait. Report denied edits for host-rule adjustment or Director editing. Use actual native output after the Director explicitly continues you.

The Director owns Git, user decisions, member lifecycle, pane dispatch and teardown. Keep edits within assigned ownership and preserve teammates' work. The installed permission map remains subject to this assignment authority. Ask the Director before expanding scope or starting other agents.
