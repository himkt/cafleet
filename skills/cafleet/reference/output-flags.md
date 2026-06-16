# Output flags — `--full` / `--json` / `--quiet`

`--full`, `--json`, and `--quiet` are cafleet's output-control flags over its compact default output: `--full` and `--json` opt back into untruncated / structured output, while `--quiet` trims output to the bare id.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch over a single flag covering four overloaded surfaces: `message {send,poll,ack,cancel,show}` → untruncated `text` + the full typed-column envelope; `message broadcast` → the single `broadcast_summary` task rendered verbose (never per-recipient rows or a `recipient_ids` list); `agent list` / `agent show` → the four-line per-agent block (never `agent_card_json`); `member capture` → no-op (use `--lines N` / `--ansi` explicitly). Per-surface detail: [`cli-options.md`](../../../docs/spec/cli-options.md#full-semantics).

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON: compact single-line encoding (no whitespace; non-ASCII like the `…` suffix emitted as UTF-8), cheap to pipe into `jq` from a Director loop.

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

Environment variable controlling body truncation in the rendered envelope; default `200` codepoints, suffix the single codepoint `…` (U+2026). Separate hard-coded caps apply to `agent.description` (`60`) and metadata strings (`80`); `--full` bypasses all three. See [`cli-options.md`](../../../docs/spec/cli-options.md#message-body-truncation).
