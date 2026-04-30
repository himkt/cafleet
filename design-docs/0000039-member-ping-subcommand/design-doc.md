# `cafleet member ping` — extract poll-trigger dispatch into its own subcommand

**Status**: Approved
**Progress**: 10/23 tasks complete
**Last Updated**: 2026-04-30

## Overview

Add a Director-only `cafleet member ping --agent-id <director-id> --member-id <member-id>` subcommand whose only action is to inject the existing `cafleet --session-id <s> message poll --agent-id <m>` keystroke + Enter into the target member's pane via the existing `tmux.send_poll_trigger` helper. It is the manually-invokable counterpart of the broker's auto-fire (`broker._try_notify_recipient`), carved out of the strict-approval `cafleet member exec` surface so monitoring loops can nudge a stalled member's inbox without inheriting the unrestricted shell-dispatch surface that `member exec` requires.

## Success Criteria

- [ ] `cafleet member ping --agent-id <director-id> --member-id <member-id>` exists as a new Director-only subcommand. The Click signature carries exactly two required options (`--agent-id`, `--member-id`) and **no positional argument** — the action is wholly determined by the subcommand name.
- [ ] The subcommand reuses `tmux.send_poll_trigger(target_pane_id=..., session_id=..., agent_id=<member-id>)` as its dispatch primitive — no new tmux helper, no `!`-shortcut, no new keystroke shape. The keystroke shape is fixed: `cafleet --session-id <session-id> message poll --agent-id <member-agent-id>` + `Enter`, identical to what `broker._try_notify_recipient` already injects today.
- [ ] The subcommand reuses `_load_authorized_member` for cross-Director boundary enforcement and reuses the existing missing-agent / missing-placement / cross-Director / pending-placement error wording verbatim — same authorization contract as `member capture` / `member send-input` / `member exec`.
- [ ] Exit codes: `0` on dispatch success, `1` on tmux failure (`send_poll_trigger` returning `False` is converted to `click.ClickException` with wording `Error: send failed: <details>` so an operator running the command manually — or a monitoring loop wrapping it — sees the failure rather than a silent best-effort skip), `1` on every authorization-boundary rejection (cross-Director, missing placement, pending placement, agent not found, tmux unavailable). There are no exit-2 input-validation failures because the subcommand has no positional / freeform input — Click's built-in "Missing option" error covers omitted `--agent-id` / `--member-id` (exit 2).
- [ ] JSON output (`cafleet --json ... member ping ...`) emits exactly two keys: `member_agent_id`, `pane_id`. No `action` field, no `polled` field — the subcommand name IS the action; failures surface via exit 1, not via a `polled: false` field.
- [ ] Text output is one line: `Pinged member <name> (<pane_id>) — poll keystroke dispatched.`
- [ ] `.claude/settings.json` adds `Bash(cafleet --session-id * member ping *)` to **`permissions.allow`** (NOT `permissions.ask`) so monitoring loops can fire the subcommand without per-call operator confirmation. The exec surface stays under `permissions.ask` (`Bash(cafleet --session-id * member exec *)` per design 0000038) — that distinction is the entire point of the carve-out.
- [ ] `broker._try_notify_recipient` is **not** modified. The new subcommand is strictly additive: an additional manual entry-point that reuses the same `tmux.send_poll_trigger` helper but does not refactor or replace the auto-fire path. The auto-fire stays best-effort (returns `False` silently on tmux failure); the manual `member ping` path converts `False` to exit 1.
- [ ] Documentation is updated FIRST per `.claude/rules/design-doc-numbering.md`. Targets: `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `README.md`, `skills/cafleet/SKILL.md`, `skills/cafleet/roles/director.md`, `skills/cafleet/roles/member.md`, `skills/cafleet-monitoring/SKILL.md`, `.claude/rules/bash-tool.md`, `.claude/settings.json`, and the `cafleet/src/cafleet/tmux.py::send_poll_trigger` docstring.
- [ ] Every place in the documentation surface that currently advises `cafleet member exec "cafleet ... message poll ..."` (or implies using `member exec` to trigger an inbox-poll) is re-pointed to `cafleet member ping`. The `member exec` surface stays scoped to *arbitrary shell dispatch* (its original 0000038 charter); the inbox-poll usage moves to `member ping`. See §6 Doc-surface re-pointing for the explicit list.
- [ ] `cafleet/tests/test_cli_member_ping.py` is created and covers: happy-path dispatch (asserts `tmux.send_poll_trigger` called once with the correct kwargs), text and JSON outputs (two keys), tmux-failure-to-exit-1, missing-agent / missing-placement / cross-Director / pending-placement rejection wording (mirrors `test_cli_member_exec.py`), tmux-unavailable.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` all pass.

---

## Background

### Predecessor designs

| Design | Status | Relationship to this design |
|---|---|---|
| **0000034** `member-bash-via-director` | Complete | Introduced `tmux.send_poll_trigger` (the broker's auto-fire helper that types `cafleet ... message poll ...` + Enter into the recipient's pane after every `cafleet message send`). The helper is reused as-is by this design — no source change. |
| **0000038** `exec-subcommand` | Approved | Carved `cafleet member exec` out of `cafleet member send-input --bash` so the strict `permissions.ask` confirmation could be scoped to *arbitrary shell-dispatch* without polluting the AskUserQuestion-answer path. Established the `cafleet member <verb>` carve-out pattern that this design replicates. The `Bash(cafleet --session-id * member exec *)` `permissions.ask` rule stays in place; this design adds a sibling `Bash(cafleet --session-id * member ping *)` rule that goes in `permissions.allow` instead. |

### Why a second carve-out, not "trust the Director to pass safe commands to exec"

Today the Director (and any monitoring loop) has exactly one CLI entry point for putting a keystroke into a member's pane: `cafleet member exec CMD`. That subcommand is intentionally under `permissions.ask` because CMD is *arbitrary* — `git push`, `rm -rf`, anything the operator types. The strict per-call confirmation is the right surface for that.

But the monitoring loop's go-to nudge for a stalled member — "type `cafleet ... message poll ...` into the member's pane so it picks up the queued message" — is the polar opposite: it is benign, repetitive, fixed-shape, and fired multiple times per minute by a normal `/loop` health-check. Routing it through `member exec` means every single nudge inherits the strict approval prompt, interrupts the monitoring tick, and either trains the operator to reflex-approve (which defeats the safety the prompt provides for genuine `git push` cases) or wedges the loop until the operator is at the keyboard.

A dedicated `cafleet member ping` subcommand resolves the conflict at the CLI surface: the subcommand body is fixed-action (no operator-controlled CMD argument), so it can sit safely in `permissions.allow` without giving away unrestricted shell-dispatch. The exec surface keeps its `permissions.ask` rule. The two carve-outs partition cleanly: **exec = arbitrary shell, ask per call; ping = fixed inbox-poll keystroke, pre-approved.**

### Why this is *not* a refactor of `broker._try_notify_recipient`

The broker auto-fires the same poll-trigger keystroke after every `cafleet message send` (via `_try_notify_recipient` → `send_poll_trigger`). That auto-fire path is best-effort by design (per design 0000034: "the message queue remains the sole source of truth; push notification is an optimization") and stays untouched by this design. The new `cafleet member ping` is an *additional* manual entry-point for cases where the auto-fire was missed — for example, a fresh `/loop` tick observing a stalled member after the original `message send`'s push notification failed silently, or after a long idle window. Both entry-points call the same `tmux.send_poll_trigger` helper; only the failure-handling differs (auto-fire swallows `False`, `member ping` raises exit 1).

---

## Specification

### 1. Subcommand surface

| Subcommand | Before | After |
|---|---|---|
| `cafleet member exec` | accepts a single required positional `CMD`. Director-only. Strict `permissions.ask`. Used for arbitrary shell dispatch AND, by overload, as the operator's only CLI primitive for nudging a member's inbox. | unchanged. Stays scoped to *arbitrary shell dispatch* — its 0000038 charter. |
| `cafleet member ping` | does not exist. | accepts no positional argument. Director-only. Pre-approved via `permissions.allow`. Fixed action: types the inbox-poll keystroke into the member's pane. |

### 2. `cafleet member ping` Click signature and handler contract

```python
@member.command("ping")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.pass_context
def member_ping(ctx, agent_id, member_id):
    """Inject an inbox-poll keystroke into a member's pane (Director-only).

    Fixed-action wrapper around tmux.send_poll_trigger. Same authorization
    boundary as member capture / send-input / exec. Pre-approved in
    permissions.allow because the keystroke shape is fixed.
    """
```

#### Input-validation contract

| Input | Behavior |
|---|---|
| Missing `--agent-id` | Click built-in `Error: Missing option '--agent-id'.` (exit 2). |
| Missing `--member-id` | Click built-in `Error: Missing option '--member-id'.` (exit 2). |
| Any other input | N/A — the subcommand has no positional argument and no other flags. There is no operator-controlled keystroke body to validate. |

The subcommand intentionally has no `--text`, no `--cmd`, no positional CMD, and no `--since` / `--page-size` knobs. The action is wholly determined by the subcommand name. This is the property that justifies the `permissions.allow` pre-approval (operator-controlled inputs would re-introduce the exec surface concern).

#### Authorization contract (post-validation, pre-dispatch)

Mirrors `member exec` step-for-step:

1. `_require_session_id(ctx)`.
2. `tmux.ensure_tmux_available()` — raises `tmux.TmuxError`, wrapped as `ClickException` exit 1. Existing wording by cause: outside a tmux session keeps `cafleet member commands must be run inside a tmux session`; missing `tmux` binary keeps `tmux binary not found on PATH`.
3. `_load_authorized_member(session_id, agent_id, member_id, placement_missing_msg=...)` — emits the existing missing-agent / missing-placement / cross-Director errors verbatim (exit 1).
4. If `placement["tmux_pane_id"] is None` (pending placement), exit 1 with the existing wording: `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`

The `placement_missing_msg` parameter passed to `_load_authorized_member` reuses the `cafleet member send-input` / `cafleet member exec` wording verbatim: `agent {member_id} has no placement row; it was not spawned via 'cafleet member create'.`

#### Dispatch

```python
try:
    ok = tmux.send_poll_trigger(
        target_pane_id=pane_id,
        session_id=session_id,
        agent_id=member_id,
    )
except tmux.TmuxError as exc:
    raise click.ClickException(f"send failed: {exc}") from exc
if not ok:
    raise click.ClickException(
        f"send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane {pane_id}."
    )
```

`tmux.send_poll_trigger` returns `False` (rather than raising) on every tmux failure, by its 0000034 contract. The `member ping` handler converts that `False` to a `click.ClickException` (exit 1) so the operator (or a monitoring loop wrapping it) sees the failure. The auto-fire path inside `broker._try_notify_recipient` keeps swallowing `False` silently — only the manual entry-point converts.

#### Output

Text:

```
Pinged member Claude-B (%7) — poll keystroke dispatched.
```

JSON (`cafleet --json ... member ping ...`):

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7"
}
```

Two keys: `member_agent_id`, `pane_id`. No `action` field (the subcommand name IS the action; matches the `member exec` two-key convention sans `command`). No `polled` field (failures surface via exit 1).

#### Exit code summary

| Outcome | Exit | Source |
|---|---|---|
| Dispatch success | `0` | normal return |
| Missing `--agent-id` or `--member-id` | `2` | Click built-in `Missing option` |
| `tmux` unavailable / `TMUX` env var missing | `1` | `tmux.ensure_tmux_available()` → wrapped `ClickException` |
| Agent not found | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Cross-Director (placement.director_agent_id mismatch) | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) — covers both the `TmuxError` branch and the `send_poll_trigger` returning `False` branch |

### 3. `.claude/settings.json` updates

Add to `permissions.allow`:

```json
"Bash(cafleet --session-id * member ping *)"
```

Do **not** add the equivalent entry to `permissions.ask`. The `permissions.allow` entry is the entire point of the carve-out: the subcommand is fixed-action, so operator confirmation per call is unnecessary and would defeat the monitoring-loop use case. The existing `permissions.ask` entry for `Bash(cafleet --session-id * member exec *)` (added by design 0000038) stays untouched — exec retains its strict per-call confirmation surface.

The wildcard pattern `Bash(cafleet --session-id * member ping *)` matches every `cafleet --session-id <uuid> member ping --agent-id <uuid> --member-id <uuid>` invocation regardless of UUID values, identical in shape to the exec / send-input / capture entries elsewhere in the project.

### 4. Doc-surface re-pointing (exec → ping for inbox-poll usage)

This is the semantic carve-out the user asked for: every place currently saying or implying "use `cafleet member exec` to trigger a poll" switches to `cafleet member ping`. `member exec` stays scoped to *arbitrary shell dispatch* (its 0000038 charter). The list:

| Surface | Re-pointing action |
|---|---|
| `skills/cafleet-monitoring/SKILL.md` | Add `cafleet member ping` to the Stall Response escalation table as a dedicated row between the `message send` (auto-fire-via-broker) row and the `member send-input` row. Update the `/loop` Prompt Template to mention `cafleet member ping` as the dedicated nudge primitive when the broker auto-fire was insufficient or the member needs a re-poke after long idle. The existing `member exec` row in the escalation table stays — its description switches to "shell-dispatch only" (no inbox-poll mention). The "Bash request blocking case" callout stays unchanged (it correctly points at `member exec` for *shell-dispatch* requests, which is its 0000038 charter). |
| `.claude/rules/bash-tool.md` | The §"Director side (for completeness)" code block stays pointed at `cafleet member exec` (it correctly describes the bash-routing fallback, which is shell-dispatch). Add a new short paragraph immediately above it noting that for the *inbox-poll-only* nudge case, the Director's primitive is `cafleet member ping <director-id> --member-id <member-id>` — which is pre-approved in `permissions.allow` and does NOT take an arbitrary command. |
| `skills/cafleet/SKILL.md` | Add a new `### Member Ping` section between `### Member Exec` and `### Server`, mirroring the Member Exec section's structure (flags table, key-sequence row, validation table, authorization-boundary subsection, output format, exit-code summary). Update the `When to Use` list at the top of the file to mention "Triggering a member's inbox poll without dispatching a shell command (Director only)". Update the Stall-Response escalation table (if present) and the §"Routing Bash via the Director" intro to clarify that `member exec` is the *shell-dispatch* primitive and `member ping` is the *poll-only* nudge primitive. |
| `skills/cafleet/roles/director.md` | Add a short subsection — "Nudging an idle member" — distinct from the bash-routing fallback subsection. It documents `cafleet member ping --agent-id <director-id> --member-id <member-id>` as the operator's pre-approved nudge primitive, and explicitly contrasts it with `member exec` (which is for *fulfilling a shell-dispatch request from a member*, not for nudging). The existing bash-routing protocol (`member exec` for shell dispatch, serialization, cross-Director boundary) stays unchanged. |
| `skills/cafleet/roles/member.md` | No source change required — the member side of the bash-routing fallback (sends a `cafleet message send` request, waits for `! <command>` keystroke) is unaffected. Optional: add one sentence to "WHERE THE UUIDs COME FROM" noting that members do **not** invoke `cafleet member ping` (it is Director-only). |
| `ARCHITECTURE.md` | Update line 162 "Commands: `member create`, `member delete`, `member list`, `member capture`, `member send-input`, `member exec`." to add `member ping`. Add a `member ping` row to the §"Operation Mapping" table (heading at line 70, with the `member exec` row at line 87) using the existing row format: `broker.get_agent()` → authorization check + `tmux.send_poll_trigger`. Director-only manual inbox-poll nudge (counterpart to `broker._try_notify_recipient` auto-fire). Update the §"tmux Push Notifications" section to note that the broker auto-fire and the manual `cafleet member ping` subcommand share the same `send_poll_trigger` helper; the auto-fire is best-effort while the manual subcommand converts failure to exit 1. |
| `README.md` | Update the Member-Lifecycle bullet (line 22 area) and the CLI command table (lines 232-237 area) to add `cafleet member ping`. Add a one-line bullet describing it as the Director-only manual inbox-poll nudge primitive (counterpart to the broker auto-fire). |
| `docs/spec/cli-options.md` | Update line 30 "Subcommands that require `--session-id`" to add `member ping`. Update line 67 "Commands that require `--agent-id`" to add `member ping`. Add a new `### member ping` section after `### member exec` (line 435 area), mirroring its structure (flags table, key-sequence row, validation table, authorization-boundary subsection, output format, exit-code summary). Update the §"Error Messages" table (line 514 area) to add `member ping` rows for the same authorization-boundary errors as `member exec`. |
| `cafleet/src/cafleet/tmux.py::send_poll_trigger` docstring | Add one sentence noting the helper has two callers: `broker._try_notify_recipient` (auto-fire after `message send`, swallows `False` silently) and `cafleet member ping` (manual Director-only entry-point, converts `False` to exit 1). The implementation stays unchanged. |

### 5. What this design deliberately does NOT do

- **No refactor of `broker._try_notify_recipient`.** It keeps calling `send_poll_trigger` directly and keeps swallowing `False` silently. The two callers (auto-fire and manual `member ping`) intentionally have different failure-handling behaviors — the auto-fire is a fire-and-forget optimization on top of the broker queue, while the manual subcommand is an operator-driven nudge whose failure is informational.
- **No new tmux helper.** `tmux.send_poll_trigger` already exists and is correct as written; this design only adds a CLI-level wrapper around it.
- **No `!`-shortcut.** The injected keystroke is a normal `cafleet ... message poll ...` invocation — the member's `--permission-mode dontAsk` Bash tool auto-resolves it. There is no need to bypass the Bash tool layer here (which is what `!` does for `member exec`'s arbitrary commands).
- **No generalization to "ping any agent in the session".** The subcommand stays Director-to-member only, mirroring `capture` / `send-input` / `exec`. Nudging a peer agent or another Director is out of scope.
- **No change to the auto-fire response annotations** (`notification_sent` boolean on unicast responses, `notificationsSentCount` on broadcast summaries). Those continue to reflect only the auto-fire path.
- **No alias from `cafleet member exec "cafleet ... message poll ..."` to `cafleet member ping`.** The exec form continues to work (the CLI cannot tell which command it was asked to dispatch) but documentation is the only mechanism guiding users to the new subcommand. There is no soft-deprecation warning, because exec is not deprecated for any other usage.

### 6. Test surface

#### `cafleet/tests/test_cli_member_ping.py` — new

Mirror the structure of `cafleet/tests/test_cli_member_exec.py`:

- Reuse the `_placement` / `_agent` / `_UNSET` / `session_id` / `runner` / `_stub_tmux_available` / `happy_path_agent` fixture patterns verbatim.
- Add a `poll_recorder` fixture (records calls to `tmux.send_poll_trigger`, returns the recorded list, defaults `send_poll_trigger` to return `True` so happy-path tests succeed). Use `raising=False` on `monkeypatch.setattr` for the same reason as `bash_recorder` in `test_cli_member_exec.py` (clean FAIL beats setup ERROR before the Programmer adds the subcommand).
- Add the `_invoke` helper for `cafleet --session-id <s> member ping --agent-id <d> --member-id <m>`.
- Test classes:

| Class | Tests |
|---|---|
| `TestPingDispatch` | `test_poll_trigger_called_with_correct_kwargs` (asserts `tmux.send_poll_trigger` called once with `target_pane_id=PANE_ID`, `session_id=<sid>`, `agent_id=MEMBER_ID`); `test_text_output` (asserts the one-line `Pinged member <name> (<pane>) — poll keystroke dispatched.`); `test_json_output_two_keys` (asserts JSON output is exactly `{"member_agent_id": MEMBER_ID, "pane_id": PANE_ID}` — two keys, no extras). |
| `TestSendFailure` | `test_send_poll_trigger_returns_false_exits_one` (monkeypatches `tmux.send_poll_trigger` to return `False`; asserts exit 1 and stderr contains `send failed: tmux send-keys did not deliver the poll-trigger keystroke`); `test_send_poll_trigger_raises_tmux_error_exits_one` (monkeypatches `tmux.send_poll_trigger` to raise `TmuxError("simulated")`; asserts exit 1 and stderr contains `send failed: simulated`). |
| `TestAuthorizationBoundary` | `test_missing_agent_exits_one`; `test_placement_none_exits_one_with_exact_message`; `test_cross_director_exits_one_with_exact_message`; `test_pending_pane_exits_one_with_exact_message`. Wording reuses the existing `_load_authorized_member` strings verbatim (mirrors `test_cli_member_exec.py::TestAuthorizationBoundary` line-for-line). |
| `TestTmuxUnavailable` | `test_tmux_not_available_exits_one` (monkeypatches `tmux.ensure_tmux_available` to raise `TmuxError("cafleet member commands must be run inside a tmux session")`; asserts exit 1 and the existing wording). |
| `TestInputValidation` | `test_missing_agent_id_exits_two` (Click built-in `Missing option '--agent-id'`); `test_missing_member_id_exits_two` (Click built-in `Missing option '--member-id'`). No positional / freetext validation tests because the subcommand has no positional argument or freetext input. |

`broker._try_notify_recipient` test surface stays untouched — the auto-fire path is not modified by this design, and its existing tests in `test_session_bootstrap.py::test_send_message_invokes_send_poll_trigger_with_director_pane` continue to assert the auto-fire behavior.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation FIRST

Per `.claude/rules/design-doc-numbering.md`, every documentation surface is updated BEFORE any code or test changes.

- [x] Update `ARCHITECTURE.md`: add `member ping` to the commands list (line 162 area); add a `member ping` row to the §"Operation Mapping" table (heading at line 70, exec row at line 87) with the body `broker.get_agent()` → authorization check + `tmux.send_poll_trigger`. Director-only manual inbox-poll nudge (counterpart to `broker._try_notify_recipient` auto-fire); add a paragraph to §"tmux Push Notifications" noting the helper is shared between `broker._try_notify_recipient` (best-effort) and `cafleet member ping` (manual, exit-1-on-failure). <!-- completed: 2026-04-30T16:00 -->
- [x] Update `docs/spec/cli-options.md`: add `member ping` to the "Subcommands that require `--session-id`" enumeration (line 30) and the "Commands that require `--agent-id`" list (line 67); add a new `### member ping` section after `### member exec` (line 435 area) mirroring the Member Exec structure (flags table, key-sequence row, validation table, authorization-boundary subsection, output format, exit-code summary); add `member ping`-specific rows to the Error Messages table (line 514 area) for the same authorization-boundary errors as `member exec`. <!-- completed: 2026-04-30T16:00 -->
- [x] Update `README.md`: extend the Member-Lifecycle bullet (line 22 area) to mention `member ping` as the Director-only manual inbox-poll nudge primitive; add a `cafleet --session-id <id> member ping --agent-id <id> --member-id <id>` row to the CLI command table (line 232-237 area); update the bullet at line 27 to add `ping` to the `member create/delete/list/capture/send-input/exec/ping` enumeration. <!-- completed: 2026-04-30T16:00 -->
- [x] Update `skills/cafleet/SKILL.md`: the Required Flags table's `--session-id` row (line 27) already enumerates `member *` as a wildcard, so `member ping` is implicitly covered — **no edit to that cell needed**. Add a new `### Member Ping` section between `### Member Exec` and `### Server` mirroring the Member Exec subsection structure (flags table, key-sequence row, validation table, authorization-boundary subsection, output format, exit-code summary); update §"Routing Bash via the Director" intro to clarify exec = shell-dispatch primitive, ping = inbox-poll-only nudge primitive; add a one-line entry to the When-to-Use bullet list at the top of the file. <!-- completed: 2026-04-30T16:00 -->
- [ ] Update `skills/cafleet/roles/director.md`: add a new "## Nudging an idle member" subsection (distinct from the existing bash-routing fallback subsection) documenting `cafleet member ping` as the operator's pre-approved nudge primitive, contrasting it with `member exec` (shell dispatch only). Keep the existing bash-routing protocol (`member exec` for shell dispatch, serialization, cross-Director boundary) unchanged. **Also** rewrite line 39 to drop the regression-style residue from design 0000038's hard-rename: change `**\`member exec\` mechanics:** the command is a single required positional argument — there is no \`--bash\` flag, no \`--choice\` / \`--freetext\` interplay, and no AskUserQuestion \`4\` digit prepended.` to `**\`member exec\` mechanics:** the command is a single required positional argument. The subcommand works on any pane that is at the Claude Code input prompt. Empty / whitespace-only commands and commands containing newlines are rejected by the CLI handler with exit 2.` Per `.claude/rules/removal.md`, source code and skills should describe only the current state — the `--bash` / `--choice` / `--freetext` enumeration belongs in the design 0000038 historical record, not in a live skill. <!-- completed: 2026-04-30T16:00 -->
- [x] Update `skills/cafleet/roles/member.md`: add one sentence to "WHERE THE UUIDs COME FROM" noting members do **not** invoke `cafleet member ping` (Director-only). No other change required — the member side of the bash-routing fallback is unaffected. <!-- completed: 2026-04-30T16:00 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md`: add a `cafleet member ping` row to the Stall Response escalation table (line 80 area) between the `message send` (which auto-fires the broker poll-trigger) row and the `member send-input` row, describing it as the Director's pre-approved manual inbox-poll nudge for cases where the broker auto-fire was insufficient; update the `/loop` Prompt Template (line 96 area) to mention `cafleet member ping` as the dedicated re-poke primitive when a member appears stalled despite a recent `message send`. The existing `member exec` row stays — its description tightens to "shell-dispatch only". The "Bash request blocking case" callout (line 52) stays unchanged — it correctly points at `member exec` for shell-dispatch requests. <!-- completed: 2026-04-30T16:00 -->
- [x] Update `.claude/rules/bash-tool.md`: in §"Director side (for completeness)", add a short paragraph immediately above the existing `cafleet member exec` code block noting that for the *inbox-poll-only* nudge case, the Director's primitive is `cafleet member ping --agent-id <director-id> --member-id <member-id>` (no positional argument, pre-approved in `permissions.allow`, fixed action). The existing `member exec` code block — describing the shell-dispatch fallback — stays unchanged. <!-- completed: 2026-04-30T16:00 (director-applied — Programmer's Edit tool was harness-denied for this file) -->
- [x] Update `.claude/settings.json`: add `Bash(cafleet --session-id * member ping *)` to `permissions.allow` (NOT `permissions.ask`). The existing `permissions.ask` entry for `member exec` stays untouched. <!-- completed: 2026-04-30T16:00 -->
- [x] Update the `cafleet/src/cafleet/tmux.py::send_poll_trigger` docstring: add one sentence at the end noting the helper has two callers — `broker._try_notify_recipient` (auto-fire after `message send`, swallows `False` silently) and `cafleet member ping` (manual Director-only entry-point, converts `False` to exit 1). The implementation stays unchanged. <!-- completed: 2026-04-30T16:00 -->

### Step 2: Implementation — `cafleet/src/cafleet/cli.py`

- [ ] Add the new `member_ping` subcommand. Place it directly after `member_exec` to keep the Director-only dispatch primitives clustered. Implementation outline:
  1. `_require_session_id(ctx)`.
  2. `tmux.ensure_tmux_available()` (wrap `TmuxError` as `ClickException`, exit 1).
  3. `_load_authorized_member(session_id, agent_id, member_id, placement_missing_msg=...)` — reuse the same `placement_missing_msg` wording as `member send-input` / `member exec` verbatim.
  4. `if pane_id is None: raise click.ClickException("member <id> has no pane yet (pending placement) — nothing to send.")` (existing wording).
  5. Wrap `tmux.send_poll_trigger(target_pane_id=pane_id, session_id=session_id, agent_id=member_id)` in `try` / `except tmux.TmuxError as exc: raise click.ClickException(f"send failed: {exc}") from exc`. (Defense-in-depth: `send_poll_trigger` is documented as catching `TmuxError` internally and returning `False`, but if a future refactor surfaces a raise, the handler still does the right thing.)
  6. If the helper returns `False`, raise `click.ClickException(f"send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane {pane_id}.")`.
  7. JSON output: `{"member_agent_id": member_id, "pane_id": pane_id}` (two keys, exactly).
  8. Text output: `f"Pinged member {target['name']} ({pane_id}) — poll keystroke dispatched."`. <!-- completed: -->
- [ ] Run `mise //cafleet:lint` and `mise //cafleet:format` to confirm the new code is clean. <!-- completed: -->

### Step 3: Tests — `cafleet/tests/test_cli_member_ping.py` (new)

- [ ] Create `cafleet/tests/test_cli_member_ping.py`. Mirror the fixture pattern from `test_cli_member_exec.py`: `_placement`, `_agent`, `_UNSET`, `session_id`, `runner`, `_stub_tmux_available` (autouse), `happy_path_agent`, and a new `poll_recorder` fixture (records calls to `tmux.send_poll_trigger`; defaults to returning `True`; uses `raising=False` on the monkeypatch). <!-- completed: -->
- [ ] Add the `_invoke` helper for `cafleet --session-id <s> member ping --agent-id <d> --member-id <m>` (no positional argument). <!-- completed: -->
- [ ] Add `TestPingDispatch` class with: `test_poll_trigger_called_with_correct_kwargs`, `test_text_output`, `test_json_output_two_keys`. <!-- completed: -->
- [ ] Add `TestSendFailure` class with: `test_send_poll_trigger_returns_false_exits_one`, `test_send_poll_trigger_raises_tmux_error_exits_one`. <!-- completed: -->
- [ ] Add `TestAuthorizationBoundary` class with: `test_missing_agent_exits_one`, `test_placement_none_exits_one_with_exact_message`, `test_cross_director_exits_one_with_exact_message`, `test_pending_pane_exits_one_with_exact_message`. Wording mirrors `test_cli_member_exec.py::TestAuthorizationBoundary` verbatim. <!-- completed: -->
- [ ] Add `TestTmuxUnavailable` class with `test_tmux_not_available_exits_one`. <!-- completed: -->
- [ ] Add `TestInputValidation` class with `test_missing_agent_id_exits_two` and `test_missing_member_id_exits_two` (Click built-in `Missing option` errors). No positional / freetext validation tests because the subcommand has no operator-controlled keystroke body. <!-- completed: -->

### Step 4: Quality gates

- [ ] `mise //cafleet:test` passes (existing tests still pass; new `test_cli_member_ping.py` passes; `broker._try_notify_recipient` tests still pass unchanged). <!-- completed: -->
- [ ] `mise //cafleet:lint` passes. <!-- completed: -->
- [ ] `mise //cafleet:format` passes. <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-30 | Initial draft. |
| 2026-04-30 | Reviewer round 1. Applied 3 fixes. (1) Relabeled "Member-Lifecycle table" mentions to the actual ARCHITECTURE.md §"Operation Mapping" table heading (line 70, exec row at line 87) in both §6 doc-surface table and Step 1 ARCHITECTURE.md task. (2) Dropped the redundant Required Flags `--session-id` cell edit from the Step 1 SKILL.md task (the `member *` wildcard at SKILL.md line 27 already covers `member ping`). (3) Folded the `.claude/rules/removal.md` residue cleanup at `skills/cafleet/roles/director.md` line 39 into the existing Step 1 director.md task (drop the regression-style "no `--bash` flag, no `--choice` / `--freetext` interplay" sentence left behind by design 0000038's hard-rename). |
| 2026-04-30 | Approved by user. Status flipped to Approved. |
