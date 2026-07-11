# Output flags — `--full` / `--json`

`--full` and `--json` are cafleet's two **independent, composable** output-control flags over its compact default output. `--full` opts back into untruncated, every-field output — it bypasses the truncation caps. `--json` switches the encoding to structured JSON but is **still truncated** unless combined with `--full` (`--json --full` gives untruncated JSON). A separate `--quiet` flag — on `message send`, `message ack`, and `member ping` — prints only the bare `task_id` (the target member id for `ping`) for shell capture.

## `--full` (cross-subcommand escape hatch)

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch over a single flag covering four overloaded surfaces: `message {send,poll,ack,cancel,show}` → untruncated `text` + the full typed-column envelope; `message broadcast` → the single `broadcast_summary` task rendered verbose (never per-recipient rows or a `recipient_ids` list); `member show` → the labeled block (`kind`, `skills`, placement sub-block) in **text mode only** (JSON is the unprojected broker dict regardless of `--full`); `member create` → the 6-line `Member registered and spawned.` block. Per-surface detail: [`cli-options.md`](../../../docs/spec/cli-options.md#full-semantics).

## `--json` (global, machine-parseable)

`--json` switches CLI output from text to JSON: compact single-line encoding (no whitespace; non-ASCII like the `…` suffix emitted as UTF-8), cheap to pipe into `jq` from a Director loop.

```bash
cafleet --json message poll --fleet-id <fleet-id> --member-id <m>
cafleet --json message poll --fleet-id <fleet-id> --member-id <m> --full
```

## `CAFLEET_MAX_TEXT_LEN`

Environment variable controlling body truncation in the rendered envelope; default `200` codepoints, suffix the single codepoint `…` (U+2026). Separate hard-coded caps apply to `agent.description` (`60`) and metadata strings (`80`); `--full` bypasses all three. See [`cli-options.md`](../../../docs/spec/cli-options.md#message-body-truncation).
