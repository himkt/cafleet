# Legacy flags — `--full` / `--pretty` / `--json` opt-back-ins

Design 0000049 made every cafleet output as compact as possible by default. The flags below are the opt-back-ins for the pre-design-0049 verbosity.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch. A single flag covers four overloaded surfaces. Per-subcommand granular variants (`--full-envelope` / `--full-recipients` / `--full-card` / `--full-body`) were considered and rejected — see design 0000049 Concerns §1.

| Subcommand | Default | `--full` |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix; compact rendered envelope (`id`, `from`, `ts`, `text` + optional `kind`/`origin`). | Untruncated `text` + full typed-column envelope (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`). |
| `message broadcast` | One-line summary `broadcast id=<id8> recipients=<count>`; `recipient_ids` and per-recipient envelopes elided. | Full broadcast summary including `recipient_ids[]` and per-recipient envelopes. |
| `agent list` / `agent show` | One row per agent (`<id8> <name> <status>`); `description` truncated to 60 codepoints; `agent_card_json` projected to minimum-required fields. | Four-line per-agent block including untruncated `description` and the full `agent_card_json` blob. |
| `member capture` | Default `--lines 30`; ANSI escapes stripped (`--no-ansi` is the default). | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `member capture` for surface consistency but is a no-op there. |

## `--pretty` (global, indented JSON)

```bash
cafleet --json message poll --agent-id <m>             # default: compact JSON, no whitespace
cafleet --json --pretty message poll --agent-id <m>    # indented JSON (json.dumps(..., indent=2))
```

Default JSON encoding is `json.dumps(data, separators=(",",":"))` — no whitespace. `--pretty` switches to indented (`json.dumps(data, indent=2)`). No effect on text-mode output; no effect when `--json` is not passed.

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON. Combined with the new compact-by-default JSON encoding, `--json` is now cheap to pipe into `jq` from a Director loop without paying for indentation. Compose with `--pretty` when a human is reading the output.

```bash
cafleet --json message poll --agent-id <m>
cafleet --json --pretty message poll --agent-id <m>
cafleet --json message poll --agent-id <m> --full
cafleet --json --pretty message poll --agent-id <m> --full
```

## `--quiet` (per-subcommand, message-id-only)

On `message send`, `message ack`, and `member ping`: emit only the new task id (8-char prefix) on stdout, nothing else. Mutually exclusive with `--full`. Useful for scripted loops where the rest of the echo is noise.

```bash
cafleet --session-id <s> message send --agent-id <m> --to <r> --text "..." --quiet
# → abc12345
```

## `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope. Default `200` codepoints. Wired via `Field(validation_alias="CAFLEET_MAX_TEXT_LEN")` on `Settings`, matching the `CAFLEET_`-prefixed convention used by `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST`, and `CAFLEET_BROKER_PORT`. The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS) — replaced the three-ASCII-character `...` suffix in design 0000049 Surface 5.

`CAFLEET_MAX_TEXT_LEN` also caps `agent.description` (limit `60`, hard-coded) and metadata-string truncation (limit `80`, hard-coded). Bypass all three via `--full`.
