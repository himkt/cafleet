---
icon: lucide/coins
---

# Token reduction

CAFleet does not consume LLM tokens itself, but every byte it emits — member
spawn prompts, message envelopes, poll output, broker auto-injected text, the
`cafleet` skill, and (most expensively) the raw tmux pane content returned by
`cafleet pane capture` — lands in a coding agent's context and bills against
its tokens. Moving the supervision scheduler out of the coding agents into the
monitoring member's `cafleet monitor` process ([Monitoring](monitoring.md))
removes the per-tick scheduling prompt — no agent carries a scheduling template
in context. The techniques below catalog the architectural choices that keep
per-message, per-spawn, per-tick, and per-context-load cost down.

| Technique | Architectural touch-points |
|---|---|
| Compact rendered envelope | Default JSON output is compact (no whitespace). Default text-mode envelope is 2 lines per task (the `--full` form is a variable-length labeled block). Ids are full integers — short by construction (typically 1–4 digits), so they paste straight into `--to` / `--id` / `--agent-id` / `--task-id` with no prefix resolution. |
| Slim member spawn prompt | The default spawn-prompt template is ~60 tokens (single sentence + identity + skill-load directive + poll command), delivered verbatim; identity reaches the pane as env vars (`CAFLEET_FLEET_ID` / `CAFLEET_AGENT_ID` / `CAFLEET_DIRECTOR_AGENT_ID`) rather than via prompt substitution. |
| Skill-file split | The core cafleet skill stays compact (identity + poll/send/ack); director-only, broadcast, exec-routing, recovery, and output-flag content loads from reference files. The split's load-bearing links are protected by the per-reader-role **Required-reading** convention (below), which keeps the saved-token architecture from leaking load-bearing protocol — genuinely optional reads stay on-demand. |
| `cafleet agent list --activity` | Aggregates per-agent message timestamps into `last_sent` / `last_recv` / `last_ack` / `idle` columns; broadcast summary rows are excluded from `last_ack`. |
| Persisted-shape simplification | Every `Task` field is a flat typed column; the message body lives in `Task.text` and there is no opaque per-task JSON blob. WebUI consumers use the same typed-column flat shape. |
| Inline message preview | The broker keystrokes a 2-line preview into the recipient's pane instead of requiring a poll round-trip — see [tmux push notifications](tmux-push.md). |
| Agent render slim | Each agent renders to the minimum-required fields by default (`id`, `name`, `description` truncated, `status`, and `coding_agent` from placement); `--full` returns the agent dict unchanged. The agent surfaces never emit `agent_card_json` in either mode. |

## Required-reading convention

The Skill-file split keeps load-bearing protocol out of the eagerly-loaded core and behind reference links. Because a saved link is a link an agent can glide past, every reader entry point — each `SKILL.md` dispatch surface, each workflow body, each `roles/*.md` — opens with a **Required reading** block that makes the load-bearing links unskippable while leaving the genuinely optional ones lazy. The block is the safety counterpart of the split: the split saves the tokens, the block protects what the split deferred.

Each block classifies every link the reader needs into three tables:

- **Load-bearing — Read in order before acting.** These are the reads the agent cannot reconstruct from the page. Each row carries a concrete "what you lose if you skip it" consequence (a wrong write path, an unresolved `{placeholder}`, a dropped protocol). The agent's overlay (`reference/coding-agent/<name>.md`) is row #1 wherever the page uses `{placeholder}` tokens, and that row is **read-and-resolve**, not read-only: after reading the overlay the agent materializes every `{placeholder}` it will use to the overlay's concrete value, applies each bound note, and self-checks that no literal `{token}` escapes. The *Resolve your overlay* checkpoint in `skills/cafleet/SKILL.md` is the application counterpart of this read gate.
- **Load-bearing on trigger — Read at the named moment.** Deferred but mandatory then (teardown, broadcast, a Bash denial), each with its own consequence cell. This preserves the split's lazy-load savings: the read stays deferred, the gate states when it becomes due.
- **On-demand — Read only when you need that capability.** Genuinely optional; no consequence column.

The classification is **per-reader-role**: each block lists only the load-bearing subset for the reader of that file. A member entry point never force-reads a Director-only governance page, so the token split established for members stays intact — the block documents the boundary instead of erasing it.

