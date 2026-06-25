# Output flags — `--full` / `--json`

`--full` and `--json` are cafleet's output-control flags over its compact default output: both opt back into untruncated / structured output. There is no `--quiet` flag.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch over a single flag covering four overloaded surfaces: `message {send,poll,ack,cancel,show}` → untruncated `text` + the full typed-column envelope; `message broadcast` → the single `broadcast_summary` task rendered verbose (never per-recipient rows or a `recipient_ids` list); `agent list` / `agent show` → the four-line per-agent block (never `agent_card_json`); `pane capture` → no-op (use `--lines N` / `--ansi` explicitly). It is a **documented** flag on every subcommand that accepts it. Per-surface detail: [`cli-options.md`](../../../docs/spec/cli-options.md#full-semantics).

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON: compact single-line encoding (no whitespace; non-ASCII like the `…` suffix emitted as UTF-8), cheap to pipe into `jq` from a Director loop.

```bash
cafleet --json message poll --agent-id <m>
cafleet --json message poll --agent-id <m> --full
```

## `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope; default `200` codepoints, suffix the single codepoint `…` (U+2026). Separate hard-coded caps apply to `agent.description` (`60`) and metadata strings (`80`); `--full` bypasses all three. See [`cli-options.md`](../../../docs/spec/cli-options.md#message-body-truncation).
