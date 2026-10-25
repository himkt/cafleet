# CLI reference

Use `cafleet --help` and each subcommand's `--help` as the command/option registry. This page explains operational behavior. Run each CAFleet command as one shell-tool invocation.

## IDs and bodies

Pass a literal integer subject immediately after its subcommand: `member show MEMBER_ID`, `member list FLEET_ID` or `message ack MESSAGE_ID`. Send names its sender/recipient with `--from-member-id` and `--to-member-id`; both must be active in the same live fleet.

Send and member creation accept exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Bodies preserve exact nonblank text. Use a file for long/multiline input. Compact spawn prompts remain necessary because rendered prompts enter backend argv.

## Configuration

`CAFLEET_DATABASE_URL` selects a SQLite file using `sqlite:///<path>`, defaulting to `~/.local/share/cafleet/cafleet_v8.db`. Resolve relative files against the invoking directory; an explicit tilde stays literal. `CAFLEET_MULTIPLEXER` selects a supported backend instead of automatic context discovery. Use CLI help for supported values.

`CAFLEET_MAX_TEXT_LEN` defaults to 200 Unicode codepoints and accepts nonnegative integers, including zero. Text rendering truncates bodies with `…`; JSON preserves full bodies. Invalid supplied values fail explicitly.

### Config-dir resolution

| Backend asset | Override | Default root | Installed location |
|---|---|---|---|
| Claude skills | `CLAUDE_CONFIG_DIR` | `~/.claude` | `skills/` |
| Codex skills/rules | `CODEX_HOME` | `~/.codex` | `skills/`, `rules/cafleet.rules` |
| OpenCode skills | Fixed discovery path | `~/.config/opencode` | `skills/` |
| OpenCode preset | `OPENCODE_CONFIG_DIR` | `~/.opencode` | `agents/cafleet.md` |

Supplied overrides must be absolute, nonempty paths. Validate only the selected backend before member registration/pane effects; unrelated malformed paths leave that launch unaffected. OpenCode's preset may relocate while its skills remain fixed. Same-name OpenCode definitions can merge with home configuration.

## Setup and diagnosis

### Setup

Setup validates database compatibility first, applies supported migrations, then installs selected embedded skills/presets in order. It stops at the first failure and records each backend only after that backend's complete install succeeds. Unrelated skills remain intact. Replacing a symlink unlinks that entry, preserving its referent.

Compatible upgrades preserve application rows and existing migration history. Ahead, foreign or malformed databases are refused before either storage or assets change. Ordinary fleet/member/message commands require a current compatible schema.

Run setup after upgrading the CLI. Installed versions remain diagnostic facts; durable poll/ACK can continue independently of unrelated installation metadata.

### Doctor

Doctor reports independent multiplexer context, database compatibility and backend executable/configuration/current-path install facts. It collects every independently obtainable check before returning.

```json
{"checks":[{"name":"example","status":"ok","detail":"Observed usable prerequisite"}]}
```

Text prints one line per check. Exit 1 means at least one `error`; exit 0 means none. Missing optional executables and absent/stale install records use `info`. Malformed supplied configuration, required missing presets for available backends and actual check/query failures use `error`. Unavailable dependent checks keep their name with an explanatory `info` detail. Doctor is advisory; actual host permissions govern execution.

## Fleet and member lifecycle

Fleet creation registers the current Director/pane atomically and spawns no session. Declare the backend of that calling host. Member creation uses detached panes, preserves focus and publishes concrete placement after successful launch. It checks the selected executable/configuration and required preset before effects.

Prompt substitution accepts exactly `{fleet_id}`, `{member_id}`, `{director_member_id}` and `{coding_agent}`; double all other literal braces. Members receive literal identity lines and the selected database URL.

Failed creation attempts cleanup only for a known owned pane and registration. Unknown-pane or cleanup failures retain their causes. Inspect actual state before recovery. Herdr creation/deletion automatically sizes/equalizes the affected layout.

Deletion acquires shared concrete-pane ownership, revalidates placement, closes the pane outside the write transaction, then finalizes storage. Contention expiry leaves pane/registry unchanged. Mandatory finalization has independent completion grace; optional layout failure remains visible after successful deregistration.

Delete owned ordinary members before fleet deletion. Fleet deletion soft-deletes registry state and retains messages/history; it closes no panes. See [Run a fleet](../how-to/mixed-backend-team.md).

### Roster and capture

Roster JSON includes activity fields `last_sent`, `last_recv`, `last_ack` and nonnegative `idle`. Missing activity/placement is nullable; a pending pane remains distinguishable. Human idle output uses seconds/minutes/hours.

Targeted `member capture` reads a useful tail window, cleaning ANSI escapes/carriage-return redraws by default; `--ansi` preserves escapes. JSON includes content, member/pane identity, requested lines, capture timestamp and the SHA256 of emitted UTF-8 content. Capture writes no broker state.

### Authorized pane input

`member prompt` submits one nonblank control-free line. `member exec` accepts one command line positionally or through `--file`, strips at most one terminal LF/CRLF for file input and submits visible `! COMMAND`.

Both reject the Director's own pane, acquire shared input ownership and revalidate placement. Submission leads with Escape, settle, literal text, settle and Enter. Escape can interrupt work or dismiss a prompt.

Success establishes submission only. Inspect actual output/completion before continuation. Failed writes can partially type and failure after Enter may mean execution began; inspect uncertainty before a newly authorized retry. The Director authorizes the actual command under its host rules.

## Messaging

### Send, poll and ACK

Send persists first, then attempts ordinary-member previews. Sends addressed to the root Director and self-sends use durable inboxes without automatic pane input. A committed ID survives transport failure; consume that record once. Poll returns pending incoming deliveries, and ACK strictly transitions `input_required` to `completed`.

Notifications are safe one-line submissions, carrying folded text or a show pointer for long content. Eligible pending messages drain FIFO under shared pane ownership. Busy/pending placement or transport failure can defer preview; later send/wait recovers it.

### Message wait

Finite wait prioritizes the subject inbox and retries eligible ordinary-member previews using a round-robin cursor. It excludes the subject, root Director, self-sends and already-consumed/notified records. A failed message is excluded for the rest of that invocation; later sends or wait invocations can retry it, while later messages for the same member remain eligible.

Receive expiry is monotonic. In-flight input retains its bounded transport budget and mandatory completion grace. Retry failures print immediately to stderr; a fatal storage failure retains its cause. Availability/timeout outcomes return success, with full messages in JSON. Poll and ACK consumption explicitly.

### Message listen

`cafleet message listen MEMBER_ID --timeout 300` emits JSONL without `--json`. A lease accepts 1 through 1800 seconds; omitting it runs until closure/cancellation/failure. Activity leaves the absolute lease unchanged.

```json
{"event":"ready","member_id":2,"fleet_id":1}
{"event":"message","message":{"message_id":10,"from_member_id":3,"to_member_id":2,"created_at":"2026-10-25T00:00:00+00:00","status_state":"input_required","status_timestamp":"2026-10-25T00:00:00+00:00","text":"Review complete"}}
{"event":"closed","member_id":2,"reason":"timeout"}
```

Ready is flushed before replay. Each short snapshot selects pending records in creation/ID order; one process emits each pending ID once and restart replays unACKed records. It performs no ACK, preview retry or pane operation. Closure also reports observed member/fleet disappearance.

Snapshot SQL and output have bounded deadlines shortened by the lease. Partial writes resume from their byte offset; broken/blocked output fails visibly and restores descriptor flags. Complete records remain valid after a later error; recover an incomplete final line through durable replay.

The native owner retains one handle, reconciles replay and observes old-process exit before replacement. [Backend reception](coding-agent-backends.md#native-reception) and the installed skill provide host operation.

## Output and errors

Use trailing per-subcommand `--json` where help advertises it. JSON preserves full Unicode body text; key order is incidental. Message wrappers are described in the [envelope reference](message-envelope.md).

Application errors go to stderr and exit 1, including with JSON requested. Argument-parse errors retain the parser's normal exit 2. Preserve actual storage/process/cleanup causes. A send diagnostic identifying an already persisted message supplies no reason to create a duplicate.
