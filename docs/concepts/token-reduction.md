---
icon: lucide/coins
---

# Token reduction

CAFleet does not consume LLM tokens itself, but every byte it emits — member
spawn prompts, message envelopes, poll output, broker auto-injected text,
the `cafleet` skill, the project `CLAUDE.md` / rules files, the Director's
`/loop` template, and (most expensively) the raw tmux pane content returned
by `cafleet member capture` — lands in a coding agent's context and bills
against its tokens. The architectural-shape changes that reduce per-message,
per-spawn, per-Director-tick, and per-context-load cost are summarized
below.

| Surface | Change | Architectural touch-points |
|---|---|---|
| 1 | Compact rendered envelope (`output.render_task`) | Default JSON output is compact (no whitespace). Default text-mode envelope drops from 6 lines per task to 2. The 8-char IDs in compact output are pasteable back into the next command via prefix resolution on `--to` / `--id` / `--member-id` / `--task-id`. |
| 6 | Slim member spawn prompt | `_MEMBER_PROMPT_TEMPLATE` shrinks from ~150 to ~60 tokens (single sentence + identity + skill-load directive + poll command). The `{director_name}` placeholder is removed from `_resolve_prompt`. |
| 7 | Skill-file split | `skills/cafleet/SKILL.md` core trimmed to ≤ 350 lines (identity + poll/send/ack + minimal codex/claude branch); director-only, broadcast, exec-routing, recovery, and legacy-flag content moves to `skills/cafleet/reference/*.md` files loaded on demand via Read. Codex and Claude Code agents both load the core SKILL the same way; reference files are pulled only when the workflow needs them. |
| 8 | `cafleet member list --activity` | Aggregates `tasks.status_timestamp` per member into `last_sent` / `last_recv` / `last_ack` / `idle` columns; the `last_ack` proxy filters `Task.type != 'broadcast_summary'`. Existing indexes `idx_tasks_context_status_ts` and `idx_tasks_from_agent_status_ts` cover the join. |
| 9 | Capture defaults | `cafleet member capture --lines` default drops 80 → 30; new `--ansi/--no-ansi` (default `--no-ansi`, ANSI escapes stripped via `re.sub` plus carriage-return de-fragmentation); `--tail` alias for `--lines`. |
| 14 | Persisted-shape simplification | `Task.task_json` blob dropped, `Task.text` typed column added. Single Alembic revision `0009_drop_task_json_add_text` with pre-flight non-null check + backfill; operator backup procedure documented in [data model](../spec/data-model.md). Six broker callers rewritten in lockstep (`_save_task`, `_read_task`, `_unicast_task_dict`, broadcast-summary builder, `poll_tasks`, `ack_task`/`cancel_task`). WebUI consumers update their type definitions. |
| 15 | Inline message preview | `broker._try_notify_recipient` switches from the legacy poll-keystroke to `TmuxMultiplexer.send_inline_preview`, which keystrokes a 2-line `[cafleet msg <id8> from <sender8> <ts>]` + body preview directly into the recipient's pane. Documented in [tmux push notifications](tmux-push.md); `TmuxMultiplexer.send_poll_trigger` survives only as the `member ping` re-poke primitive. |
| 18 | Agent-card render slim | New `output.render_agent` projects `agent_card_json` to the minimum-required fields by default (`agent_id`, `name`, `description` truncated, `status`, `coding_agent`); full card returned on `--full`. Storage-side `agent_card_json` blob is unchanged in this release — render-side projection captures most of the win. |

The token-budget regression suite under `tests/token_budget/` (Surface 13)
checks character counts on representative outputs against checked-in
baselines so future drift is caught at PR time.
`tests/token_budget/scenarios/idle_3_member_baseline_stub.py` is the
manually-runnable single-shot measurement stub for the per-tick cost of the
Director monitoring commands (sleeps `SETTLE_SECONDS=30` and captures once;
the originally-scoped 10-minute window was deferred when Surface 13 shipped
char-anchored regression tests as the canonical contract).
