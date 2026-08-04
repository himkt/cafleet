# CLI Interface Simplification

**Status**: Approved
**Progress**: 23/23 tasks complete
**Last Updated**: 2026-08-04

## Overview

Simplify the `cafleet` CLI by replacing the per-command flag sets with three uniform rules: the subject id is a positional argument (relationship ids stay flags), `--json` is the single output switch, and message/prompt bodies ride as positional `TEXT` with `--file PATH` as the alternative. Breaking changes are explicitly allowed; the rollout is protected by the stale-assets guard, with no transitional aliases.

## Success Criteria

- [ ] `cafleet <group> --help` output matches the target command tree exactly for every group and subcommand.
- [ ] `--full`, `--quiet`, `--no-ansi`, `--text`, and `--text-file` are rejected everywhere with clap's standard unknown-argument error.
- [ ] `--json` output carries complete, untruncated message bodies on every message subcommand; text output stays truncated to `CAFLEET_MAX_TEXT_LEN`.
- [ ] `message ack` / `message show` / `message poll` succeed with only the subject id; `message send` rejects a cross-fleet pair with `members A and B are not in the same fleet.`
- [ ] `monitor` is a single command (`cafleet monitor FLEET_ID`); pane capture lives at `cafleet member capture MEMBER_ID`.
- [ ] SPEC.md, `docs/docs/spec/cli-options.md`, `docs/docs/spec/message-envelope.md`, every skill, and every project rule quote only the new command forms.
- [ ] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` pass.

---

## Background

The command tree (7 groups, 20 subcommands) is sound; the complexity lives in the flags. Nearly every command carries `--fleet-id` plus an inconsistent subset of `--full` / `--quiet` / `--json`, and fleet and recipient identity are restated where they are derivable: member ids are globally unique rowids (the fleet is derivable from the member row), and every message row is per-recipient — broadcast fans out one delivery row per recipient, each with its own id — so recipient and fleet are derivable from the message id alone. The user confirmed backward compatibility is not a constraint.

---

## Specification

### The three uniform rules

1. **Positional subject, flag relationships.** The id a command acts on is a positional argument. Ids that describe a relationship rather than the subject stay as flags: `member create --fleet-id INT` (the subject is the new member), and `message send --from-member-id` / `--to-member-id` (the roles need labels; the long spellings are retained by user decision). `member list` takes a positional `FLEET_ID` — the fleet is the subject of the listing.
2. **One output switch: `--json`.** Text output stays human/pane-friendly and truncated as today; JSON is always the complete, untruncated machine form. `--full` and `--quiet` are deleted. (Today the JSON path truncates unless `--full` is given; under the new contract JSON never truncates.)
3. **One body convention.** Bodies ride as positional `TEXT`, with `--file PATH` (and `--file -` for stdin) as the alternative — the convention `member prompt` already uses for its positional, extended to `message send`, `message broadcast`, and `member create`. Exactly one of the positional or `--file` must be supplied; the existing empty-body, UTF-8, and `-`-stdin rules carry over unchanged.

### Target command tree

```
cafleet setup [--skip AGENT]...
cafleet doctor [--json]
cafleet server [--host H] [--port P]

cafleet fleet create --name NAME --coding-agent AGENT [--json]
cafleet fleet list [--json]
cafleet fleet show FLEET_ID [--json]
cafleet fleet delete FLEET_ID [--json]

cafleet member create --fleet-id INT --name NAME --description TEXT
                      [--coding-agent A] [--model M] [--effort E]
                      (PROMPT | --file PATH) [--json]
cafleet member list FLEET_ID [--json]
cafleet member show MEMBER_ID [--json]
cafleet member delete MEMBER_ID [--json]
cafleet member prompt MEMBER_ID TEXT [--shell] [--json]
cafleet member ping MEMBER_ID [--json]
cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]

cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]
cafleet message broadcast --from-member-id ID (TEXT | --file PATH) [--json]
cafleet message poll MEMBER_ID [--json]
cafleet message ack MESSAGE_ID [--json]
cafleet message show MESSAGE_ID [--json]

cafleet monitor FLEET_ID [--tick N] [--interval N]
```

`member prompt` and `member ping` keep their names and their text-carrying vs text-free split — the codex permission preset allows the bare `cafleet` prefix and ask-gates exactly `["cafleet", "member", "prompt"]`; the permission story is untouched. `member prompt` keeps its single-line positional `TEXT` with no `--file` alternative (its body is a one-line keystroke by contract).

### Identity derivation and authorization semantics

| Command | Old identity inputs | New derivation | Guard that remains |
|---|---|---|---|
| `message poll MEMBER_ID` | `--fleet-id` + `--member-id`, fleet-gated | fleet derived from the member row | member existence — unknown id fails with `Member N not found` |
| `message ack MESSAGE_ID` | `--fleet-id` + `--member-id` + `--message-id`, recipient check | recipient and fleet derived from the message row | message existence + `input_required` state; the "Only the recipient can ACK" check is deleted |
| `message show MESSAGE_ID` | `--fleet-id` + `--member-id` + `--message-id`, fleet-gated | fleet derived from the message row | message existence only |
| `message send` | `--fleet-id` + sender fleet-gate | fleet derived from the sender row | sender and recipient must exist; a cross-fleet pair fails with `members A and B are not in the same fleet.` |
| `message broadcast` | `--fleet-id` | fleet derived from the sender row | sender existence |
| `member show` / `delete` / `prompt` / `ping` / `capture` | `--fleet-id` + `--member-id` | fleet derived from the member row | existing guards unchanged (root-Director delete guard, pane requirement, placement checks) |
| `member create` | `--fleet-id` flag | unchanged — the fleet row is still validated and the Director auto-resolved from it | unchanged |

The recipient/ack check deletion is deliberate: the old check only compared a caller-supplied id against the row — there is no caller authentication — so it validated honesty, not identity. `poll` / `ack` / `show` lookup errors keep the existing string shapes (`Member N not found`, `Message N not found`).

`send` and `broadcast` error strings change where they referenced the caller-supplied fleet. The exact post-change set (error strings are SPEC contract):

| Failure | Post-change error string |
|---|---|
| Sender missing or not active (`send` and `broadcast`) | `Sender member not found or not active: {from}` — drops the old `in fleet` suffix; no caller-supplied fleet exists |
| Recipient missing or not active (`send`) | `Destination member not found: {to}` — unchanged |
| Sender and recipient in different fleets (`send`) | `members {from} and {to} are not in the same fleet.` — replaces both the broker's `Destination member not in fleet: {to}` and the deleted CLI gate's `member {id} is not in fleet {fleet_id}.` |

### Output contract

| Mode | Contract |
|---|---|
| Text (default) | Human/pane-friendly, message bodies truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…`. No untruncated text form exists. |
| `--json` | The complete machine form — untruncated bodies, full envelopes — on every subcommand that accepts it. |

`member capture` splits the same way: text mode prints the raw capture content only; `--json` carries the full envelope (`member_id`, `pane_id`, `lines`, `content`, `captured_at`, `content_sha256`). ANSI stripping stays the default; `--ansi` preserves escapes and `--no-ansi` is deleted. `fleet delete` gains `--json`; `doctor` keeps it.

`--full` has a second semantic beyond truncation: on `member show` / `member create` / `fleet create` it switches the text rendering to a labeled verbose block. After the deletion, the compact default text form is the only text form on those commands — the verbose text block is deleted (not made the default), and the detailed view is `--json`-only.

### Monitor flattening

`monitor capture` moves to `member capture` — it is a pane read and already borrows the member-group loader (`load_member_with_pane`). With `capture` gone, `monitor` flattens to a single command with a positional fleet id: `cafleet monitor FLEET_ID [--tick N] [--interval N]`. The live-fleet guard, tick validation, and `CAFLEET_MONITOR_WAKE_INTERVAL` default are unchanged on the flattened `monitor`.

The separate `require_live_fleet` check that `monitor capture` runs today does not move with the command: `fleet delete` deregisters every member of the fleet, so an active member row implies a live fleet, and the member-row lookup (`Member N not found`) subsumes the deleted-fleet rejection. `member capture` carries the same guards as the rest of the member group.

### Dependent surfaces

- **Spawn-prompt placeholders**: all four (`{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}`) stay unchanged. Members still read their identity from the literal spawn-prompt lines; only the command templates written in skills and role docs change shape.
- **Embedded command strings**: the ping poll-trigger keystroke (`send_poll_trigger`) currently injects `cafleet message poll --fleet-id N --member-id M …`; it becomes `cafleet message poll M …`. Every other internal string quoting a CLI form (error hints, ping resume text) is updated in the same change.
- **Shipped permission presets**: no content change. Exactly two presets ship — `presets/codex/cafleet.rules` and `presets/opencode/cafleet.md`; there is no claude preset (claude members are spawned with `--permission-mode dontAsk`). Both are prefix-based (the codex `["cafleet"]` allow and `["cafleet", "member", "prompt"]` ask-gate; the opencode `"cafleet *"`), so the flag-to-positional change touches neither — the binding constraint is that `member prompt` keeps its name. Skills ship with the binary via `cafleet setup`, and the stale-assets guard makes the migration safe — old skills cannot run against the new binary unnoticed.
- **WebUI / HTTP API**: the HTTP API contract is unchanged — the fleet-scoped routes, request/response shapes, and the WebUI's untruncated rendering stay as documented in SPEC §6.8 / §7.3. `broker::verify_member_fleet` remains for the WebUI's route-scoping (only the CLI message-group `fleet_gate` call site is deleted); the WebUI call sites (`send_message`, `broadcast_message`, `get_member`) adapt to the new broker signatures.
- **No transitional aliases.** One clean break; no deprecation shims, no dual parsing.

### Code deletions that fall out

- `FleetIdArg` and `require_fleet_id` (`cli/mod.rs`, `cli/helpers.rs`).
- The hand-rolled `--text` / `--text-file` exclusivity arms in `resolve_text_body` (replaced by positional-vs-`--file` resolution with the same empty/UTF-8/stdin rules).
- `--no-ansi` and its `overrides_with` pairing (`cli/monitor.rs`, moving to `cli/member.rs`).
- The `quiet && !json` special cases (`message send` / `ack`, `member ping`).
- The message-group `fleet_gate` and the `full` parameter threaded through `truncate_message_text`, `render_messages_in_result`, and the message/member formatters.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first, then skills, then code, per the project's documentation-first order. One task per surface.

### Step 1: SPEC.md

- [x] Rewrite SPEC.md §6.3 (command tree, shared option surface, text-body input, per-group handler sequences) to the target tree: positional subject ids, `--json`-only output, positional `TEXT` + `--file`, `member capture`, flattened `monitor` <!-- completed: 2026-08-04T08:32 -->
- [x] Rewrite SPEC.md §6.4 (output/truncation contract and error strings): JSON always untruncated, no `--full`/`--quiet`, the revised send/broadcast error-string set, ack/show/poll derivation semantics <!-- completed: 2026-08-04T08:40 -->
- [x] Sweep the remaining SPEC sections for the old surface: §2 (the `--text`/`--text-file` reader walkthrough), §6.2 (the ack recipient check and fleet-gate broker contracts), §6.5 (the `send_poll_trigger` payload strings), §7.2 (the missing-`--fleet-id` exit-code case), §7.3 (the `--full` truncation policy), §10 (the CLI command checklist), §11 <!-- completed: 2026-08-04T08:41 -->

### Step 2: Docs pages

- [x] Rewrite `docs/docs/spec/cli-options.md` to the new per-subcommand flag/positional surface, exit codes, and error strings <!-- completed: 2026-08-04T08:52 -->
- [x] Update `docs/docs/spec/message-envelope.md` (truncation/`--json` contract) and `docs/docs/spec/multiplexer-backends.md` (the injected poll-trigger command string) <!-- completed: 2026-08-04T08:54 -->
- [x] Sweep the remaining `docs/` pages (concepts, permission-preset docs) for quoted command forms and update them <!-- completed: 2026-08-04T08:58 -->
- [x] Run the `/update-readme` skill to sync `README.md` and confirm the SPEC surfaces are consistent <!-- completed: 2026-08-04T09:03 -->

### Step 3: cafleet skill

- [x] Update `skills/cafleet/SKILL.md` (send/poll/ack examples, required-flags section — fleet-scoping prose becomes subject-id prose) <!-- completed: 2026-08-04T09:12 -->
- [x] Update `skills/cafleet/reference/*` (cli.md, director.md, supervision.md, prompt-routing.md, recovery.md, base-dir.md, coding-agent overlays) and `skills/cafleet/roles/*` for every quoted command <!-- completed: 2026-08-04T09:20 -->

### Step 4: Other skills and project rules

- [x] Update `skills/cafleet-design-doc/**` and `skills/cafleet-research/**` (role files, workflow bodies, coordination examples) for every quoted command <!-- completed: 2026-08-04T09:35 -->
- [x] Update the remaining skills (`skills/clean-docs/**`, `skills/skill-author/**`, others quoting `cafleet` commands) <!-- completed: 2026-08-04T09:16 -->
- [x] Update `.claude/rules/*` (bash-tool.md, commands.md, database-migrations.md) for every quoted command <!-- completed: 2026-08-04T09:16 -->

### Step 5: CLI argument surface

- [x] Delete `FleetIdArg` / `require_fleet_id`; convert `fleet show` / `fleet delete` to positional `FLEET_ID`; add `--json` to `fleet delete`; drop `--full` from `fleet create` <!-- completed: 2026-08-04T09:57 -->
- [x] Convert the member group: positional `MEMBER_ID` on show/delete/prompt/ping, positional `FLEET_ID` on list, `(PROMPT | --file PATH)` on create; drop `--full` / `--quiet` <!-- completed: 2026-08-04T09:58 -->
- [x] Move `monitor capture` to `member capture MEMBER_ID [--lines N] [--ansi] [--json]`; delete `--no-ansi`; flatten `monitor` to `cafleet monitor FLEET_ID [--tick N] [--interval N]` <!-- completed: 2026-08-04T09:58 -->
- [x] Convert the message group: positional `MESSAGE_ID` / `MEMBER_ID` subjects, `(TEXT | --file PATH)` bodies, keep `--from-member-id` / `--to-member-id`; drop `--full` / `--quiet` <!-- completed: 2026-08-04T09:59 -->
- [x] Replace `resolve_text_body` with the positional-vs-`--file` resolver, preserving the empty-body, UTF-8, and stdin (`-`) rules and error strings <!-- completed: 2026-08-04T09:59 -->

### Step 6: Broker and output semantics

- [x] Derive fleet/recipient in the broker: `ack_message(message_id)` (existence + state only), `get_message(message_id)`, `poll_messages` with a member-existence check, `send_message` / `broadcast_message` deriving the fleet from the sender row with the new cross-fleet error string <!-- completed: 2026-08-04T10:22 -->
- [x] Remove the `full` threading from `truncate_message_text` / `render_messages_in_result` / formatters: text always truncated, JSON always complete <!-- completed: 2026-08-04T10:24 -->
- [x] Update embedded command strings: `send_poll_trigger` injects `cafleet message poll <member-id>`; sweep error hints and ping resume text for old flag shapes <!-- completed: 2026-08-04T10:25 -->
- [x] Adapt the WebUI call sites (`send_message`, `broadcast_message`, `get_member`) to the new broker signatures, preserving the HTTP API contract and the route-level `verify_member_fleet` scoping <!-- completed: 2026-08-04T10:26 -->

### Step 7: Tests and verification

- [x] Update the CLI and broker test suites to the new argument shapes and semantics (ack unconditional, poll existence error, cross-fleet send error, JSON-untruncated contract, capture split) <!-- completed: 2026-08-04T10:23 -->

- [x] Run `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` and fix fallout <!-- completed: 2026-08-04T10:38 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-04 | Initial draft |
| 2026-08-04 | Review round 1: enumerated the send/broadcast error-string set, specified the `--full` verbose-block fate, dropped the live-fleet guard on the moved `member capture`, corrected the preset surface to no-content-change, scoped the WebUI/HTTP API surface, widened the SPEC sweep |
| 2026-08-04 | User feedback: clarified that `--skip` retains its per-agent assets-skip purpose; `--schema-only` skips the assets half entirely rather than superseding `--skip` |
| 2026-08-04 | User feedback: dropped `--schema-only` — `setup` is untouched by this design; the documented schema-only invocation remains the triple-skip form |
