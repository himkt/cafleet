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
| Compact rendered envelope | Default JSON output is compact (no whitespace). Default text-mode envelope is 2 lines per task (the `--full` form is a variable-length labeled block). Ids are full integers — short by construction (typically 1–4 digits), so they paste straight into `--to` / `--id` / `--member-id` / `--task-id` with no prefix resolution. |
| Slim member spawn prompt | The default spawn-prompt template is ~60 tokens (single sentence + identity + skill-load directive + poll command). |
| Skill-file split | The core cafleet skill stays compact (identity + poll/send/ack); director-only, broadcast, exec-routing, recovery, and output-flag content loads from on-demand reference files. |
| `cafleet member list --activity` | Aggregates per-member message timestamps into `last_sent` / `last_recv` / `last_ack` / `idle` columns; broadcast summary rows are excluded from `last_ack`. |
| Persisted-shape simplification | Every `Task` field is a flat typed column; the message body lives in `Task.text` and there is no opaque per-task JSON blob. WebUI consumers use the same typed-column flat shape. |
| Inline message preview | The broker keystrokes a 2-line `[cafleet msg <task_id> from <sender_id> <ts>]` + body preview directly into the recipient's pane. Documented in [tmux push notifications](tmux-push.md); a separate poll-trigger keystroke backs the `cafleet member ping` re-poke. |
| Agent render slim | Each agent renders to the minimum-required fields by default (`id`, `name`, `description` truncated, `status`, and `coding_agent` from placement); `--full` returns the agent dict unchanged. The agent surfaces never emit `agent_card_json` in either mode. |

