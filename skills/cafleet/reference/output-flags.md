# Output flags — `--full` / `--json` / `--quiet`

`--full`, `--json`, and `--quiet` are cafleet's output-control flags over its compact default output: `--full` and `--json` opt back into untruncated / structured output, while `--quiet` trims output to the bare id.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch. A single flag covers four overloaded surfaces. Per-subcommand granular variants (`--full-envelope` / `--full-recipients` / `--full-card` / `--full-body`) were considered and rejected.

| Subcommand | Default | `--full` |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix; compact rendered envelope (`id`, `from`, `ts`, `text` + optional `kind`/`origin`). | Untruncated `text` + full typed-column envelope (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`). |
| `message broadcast` | One-line summary `broadcast id=<id> recipients=<count>`. The broker only ever returns the single `broadcast_summary` task plus the top-level `notifications_sent_count` wrapper field — there are no per-recipient envelopes or `recipient_ids` list in the response. | Renders the single `broadcast_summary` task as the full verbose envelope (typed-column dict in `--json`) instead of the one-line summary. Never adds per-recipient envelopes or a `recipient_ids` list — the response is always that one summary task plus `notifications_sent_count`. |
| `agent list` / `agent show` | One row per agent (`<id> <name> <status>`); `description` truncated to 60 codepoints. JSON projects each agent to `id` / `name` / `description` / `status` (plus `coding_agent` when a placement is present). | Four-line per-agent block: full `agent_id`, `name`, `description` (still truncated to 60 codepoints), `status`. JSON returns the broker agent dict unchanged. No `agent_card_json` — the agent surfaces never load it. |
| `member capture` | Default `--lines 30`; ANSI escapes stripped (`--no-ansi` is the default). | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `member capture` for surface consistency but is a no-op there. |

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON. JSON encoding is compact (`json.dumps(data, separators=(",",":"), ensure_ascii=False)` — no whitespace, non-ASCII like the `…` suffix emitted as UTF-8), so `--json` is cheap to pipe into `jq` from a Director loop.

```bash
cafleet --json message poll --agent-id <m>
cafleet --json message poll --agent-id <m> --full
```

## `--quiet` (per-subcommand, message-id-only)

On `message send`, `message ack`, and `member ping`: emit only the new task id on stdout, nothing else. Mutually exclusive with `--full`. Useful for scripted loops where the rest of the echo is noise.

```bash
cafleet message send --fleet-id <s> --agent-id <m> --to <r> --text "..." --quiet
# → 42
```

## `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope. Default `200` codepoints. Wired via `Field(validation_alias="CAFLEET_MAX_TEXT_LEN")` on `Settings`, matching the `CAFLEET_`-prefixed convention used by `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST`, and `CAFLEET_BROKER_PORT`. The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS).

`CAFLEET_MAX_TEXT_LEN` also caps `agent.description` (limit `60`, hard-coded) and metadata-string truncation (limit `80`, hard-coded). Bypass all three via `--full`.
