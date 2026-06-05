# Legacy flags — `--full` / `--json` opt-back-ins

`--full` / `--json` are the opt-back-ins for cafleet's compact default output. Each flag is documented below.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch. A single flag covers four overloaded surfaces. Per-subcommand granular variants (`--full-envelope` / `--full-recipients` / `--full-card` / `--full-body`) were considered and rejected.

| Subcommand | Default | `--full` |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix; compact rendered envelope (`id`, `from`, `ts`, `text` + optional `kind`/`origin`). | Untruncated `text` + full typed-column envelope (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`). |
| `message broadcast` | One-line summary `broadcast id=<id8> recipients=<count>`. The broker only ever returns the single `broadcast_summary` task plus the top-level `notifications_sent_count` wrapper field — there are no per-recipient envelopes or `recipient_ids` list in the response. | No effect. `--full` is preserved for surface consistency with the five `message {send,poll,ack,cancel,show}` subcommands but is a no-op on broadcast. |
| `agent list` / `agent show` | One row per agent (`<id8> <name> <status>`); `description` truncated to 60 codepoints; `agent_card_json` projected to minimum-required fields. | Four-line per-agent block including untruncated `description` and the full `agent_card_json` blob. |
| `member capture` | Default `--lines 30`; ANSI escapes stripped (`--no-ansi` is the default). | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `member capture` for surface consistency but is a no-op there. |

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON. JSON encoding is compact (`json.dumps(data, separators=(",",":"))` — no whitespace), so `--json` is cheap to pipe into `jq` from a Director loop without paying for indentation.

```bash
cafleet --json message poll --agent-id <m>
cafleet --json message poll --agent-id <m> --full
```

## `--quiet` (per-subcommand, message-id-only)

On `message send`, `message ack`, and `member ping`: emit only the new task id (8-char prefix) on stdout, nothing else. Mutually exclusive with `--full`. Useful for scripted loops where the rest of the echo is noise.

```bash
cafleet --session-id <s> message send --agent-id <m> --to <r> --text "..." --quiet
# → abc12345
```

## `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope. Default `200` codepoints. Wired via `Field(validation_alias="CAFLEET_MAX_TEXT_LEN")` on `Settings`, matching the `CAFLEET_`-prefixed convention used by `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST`, and `CAFLEET_BROKER_PORT`. The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS).

`CAFLEET_MAX_TEXT_LEN` also caps `agent.description` (limit `60`, hard-coded) and metadata-string truncation (limit `80`, hard-coded). Bypass all three via `--full`.
