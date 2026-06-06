---
icon: lucide/coins
---

# Token reduction

CAFleet does not consume LLM tokens itself, but every byte it emits — member
spawn prompts, message envelopes, poll output, broker auto-injected text,
the `cafleet` skill, the project `CLAUDE.md` / rules files, the Director's
`/loop` template, and (most expensively) the raw tmux pane content returned
by `cafleet member capture` — lands in a coding agent's context and bills
against its tokens. The architectural-shape choices that keep per-message,
per-spawn, per-Director-tick, and per-context-load cost down are summarized
below.

| Technique | Architectural touch-points |
|---|---|
| Compact rendered envelope (`output.render_task`) | Default JSON output is compact (no whitespace). Default text-mode envelope is 2 lines per task (the `--full` form is 6). The 8-char IDs in compact output are pasteable back into the next command via prefix resolution on `--to` / `--id` / `--member-id` / `--task-id`. |
| Slim member spawn prompt | `_MEMBER_PROMPT_TEMPLATE` is ~60 tokens (single sentence + identity + skill-load directive + poll command). `_resolve_prompt` has no `{director_name}` placeholder. |
| Skill-file split | `skills/cafleet/SKILL.md` core is ≤ 350 lines (identity + poll/send/ack + minimal codex/claude branch); director-only, broadcast, exec-routing, recovery, and output-flag content lives in `skills/cafleet/reference/*.md` files loaded on demand via Read. Codex and Claude Code agents both load the core SKILL the same way; reference files are pulled only when the workflow needs them. |
| `cafleet member list --activity` | Aggregates `tasks.status_timestamp` per member into `last_sent` / `last_recv` / `last_ack` / `idle` columns; the `last_ack` proxy filters `Task.type != 'broadcast_summary'`. Indexes `idx_tasks_context_status_ts` and `idx_tasks_from_agent_status_ts` cover the join. |
| Capture defaults | `cafleet member capture --lines` defaults to 30; `--ansi/--no-ansi` (default `--no-ansi`, ANSI escapes stripped via `re.sub` plus carriage-return de-fragmentation); `--tail` is an alias for `--lines`. |
| Persisted-shape simplification | Every `Task` field is a flat typed column; `Task.text` carries the message body and there is no opaque per-task JSON blob. Six broker callers (`_save_task`, `_read_task`, `_unicast_task_dict`, broadcast-summary builder, `poll_tasks`, `ack_task`/`cancel_task`) read and write typed columns directly. WebUI consumers use the typed-column flat shape. |
| Inline message preview | `broker._try_notify_recipient` keystrokes a 2-line `[cafleet msg <id8> from <sender8> <ts>]` + body preview directly into the recipient's pane via `TmuxMultiplexer.send_inline_preview`. Documented in [tmux push notifications](tmux-push.md); `TmuxMultiplexer.send_poll_trigger` is the `member ping` re-poke primitive. |
| Agent render slim | `output.render_agent` projects each broker agent dict to the minimum-required fields by default (`id`, `name`, `description` truncated, `status`, and `coding_agent` from placement); `--full` returns the agent dict unchanged. The agent surfaces (`broker.list_agents` / `get_agent`) never load `agent_card_json`, so it is not emitted in either mode. |

The token-budget regression suite under `tests/token_budget/` checks
character counts on representative outputs against checked-in baselines so
future drift is caught at PR time.
`tests/token_budget/scenarios/idle_3_member_baseline_stub.py` is the
manually-runnable single-shot measurement stub for the per-tick cost of the
Director monitoring commands (sleeps `SETTLE_SECONDS=30` and captures once).
The char-anchored regression tests under `tests/token_budget/` are the
canonical contract.
