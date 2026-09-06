# Routing Bash via the Director

The bash-via-Director protocol is the **fallback** for a harness-denied command. Members run shell commands directly via the Bash tool by default (workspace-scoped auto-approval — see [`roles/member.md`](../roles/member.md)); how often the fallback fires is per-backend. For claude and codex members, denial is the rare case — the harness deny-list rejects a few destructive operations (e.g. `git push`, `rm -rf`). For opencode members, whose preset is a deny-by-default bash allowlist, denial is the common case for any un-allowlisted command, and routing workflow commands (`mise`, `mkdir`, …) through the Director is the routine path, not a rare destructive-command event. Either way the member auto-routes a plain CAFleet message to its Director, which dispatches the command into the member's pane via `cafleet member prompt --shell` (keystrokes literal `! <cmd>` + `Enter`, honored by `claude` / `codex` / `opencode`).

## The two primitives

| Primitive | Purpose | Permission gate |
|---|---|---|
| [`cafleet member prompt`](director.md#member-prompt) | Keystroke dispatch with an operator-controlled `TEXT` body. `--shell` keystrokes `! <cmd>` + `Enter`; the plain form keystrokes `TEXT` + `Enter` as a submitted user turn. | `permissions.ask` |
| [`cafleet member ping`](director.md#member-ping-manual-inbox-poll) | Fixed-action inbox-poll; no operator-controlled body. | `permissions.allow` |

## The two forms: shell vs plain

`cafleet member prompt` has two forms with distinct follow-up semantics:

- **`--shell`** dispatches Esc-safeguarded `! <cmd>`. The bang output only **stages** in the pane — it does not advance the member's turn, so every successful shell dispatch requires a `cafleet member ping` follow-up.
- **Plain** (no `--shell`) dispatches Esc-safeguarded `TEXT` as a **real user turn** — the trailing `Enter` submits it, opening the member's turn directly. **No ping follows the plain form.** It exists for text that only takes effect when it arrives as a direct user turn in the member's pane — slash commands, skill invocations, and other magic commands a broker message body cannot trigger (a `message send` inline preview arrives as content, not as a typed command). Broker messaging remains the canonical coordination channel; the plain form is not a substitute for `message send` and is not a stall-recovery or urgent-redirect primitive.

## Member-side: reconsider, then route

Reconsider first (per-backend denial semantics above): most denials are a wrong flag, wrong path, or an unnecessary command; on opencode check whether an allowlisted command covers the need. Fix or drop what you can yourself. Only a genuinely-correct, genuinely-needed, still-denied command gets routed:

1. Send a plain CAFleet message to the Director:
   ```bash
   cafleet message send --from-member-id <my-member-id> \
     --to-member-id <director-member-id> \
     "Need to run: <command>. My harness denied it (<reason if known>)."
   ```
2. **Wait** for the Director's `member prompt --shell` dispatch to land in your pane; the bang-shortcut output appears in your next-turn context.
3. Process the output; reply to the Director if a follow-up is expected.

The forbidden behaviors (never fake `<bash-input>` markup or fabricate output, never answer from stale context, never assume Bash is denied without trying) are canonical in [`roles/member.md`](../roles/member.md#what-you-must-never-do). One routing-specific addition: never offer the operator a list of routing options — the operator already asked for the command to run; routing is implementation. If your `cafleet message send` is *also* harness-denied, tell the operator both are denied (the only time you ask the operator for help); otherwise route silently.

## Director-side dispatch

On a member's denial-fallback request: read it (`cafleet message poll` / `message show`), verify it is reasonable and from a real team member, then dispatch and follow up — **in this order**:

```bash
cafleet member prompt <member-id> --shell "<command>"
cafleet member ping <member-id>
cafleet message ack <message-id>
```

The `member ping` is required — `member prompt --shell` only stages the bang output; the ping advances the member's turn so it consumes the output.

**Serialize.** Process requests in poll order, one at a time: `prompt --shell → ping → ack → next`. Two `member prompt` dispatches firing concurrently against the same pane race the keystroke sequence and corrupt output. Within one member, `prompt --shell → ping → prompt --shell → ping`; never two concurrent prompt dispatches.

**Targeting boundary.** `member prompt` reaches any active member by its `MEMBER_ID` (no caller-auth check); an unknown or inactive id returns "not found" (see [`cli-options.md`](runtime/spec/cli-options.md#member-prompt)). A request naming a member you cannot serve is answered with a plain `cafleet message send` explaining the mismatch.
