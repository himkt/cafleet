# Database URL on every Director action argv

**Status**: Approved
**Progress**: 0/19 tasks complete
**Last Updated**: 2026-11-27

## Overview

For a non-default database, every runtime event's `director_action.inspect_argv` ends with `--database-url sqlite:///<canonical path>`, so a Director that runs any action argv as given reaches the fleet's database (GitHub issue himkt/cafleet#663). The URL is derived from the run's recorded database identity rather than from a connection path, so the payload stays a pure function of stored rows and the exact stored-payload comparison on read keeps passing through a copied or moved non-default database file.

## Success Criteria

- [ ] For a non-default database, the `inspect_argv` of `review_permission_request`, `review_question`, `review_stalled_delivery` and `reconcile_uncertain_input` each ends with `"--json","--database-url","sqlite:///<canonical path>"`, where `<canonical path>` is the run's recorded `database_identity_json.canonical_path`.
- [ ] For the default database, no `inspect_argv` carries `--database-url`.
- [ ] The URL an action carries equals the `DATABASE URL:` value `member create` renders for the same database and default path, because both delegate to one shared default decision.
- [ ] For a non-default database, `event show` (`get_event`) re-validates every action kind without error when the database is read through a copied file and through a moved (renamed) file.
- [ ] An event persisted through a connection opened on a non-canonical path to the database (for example a symlink, as a recovery-publishing Director command may use) carries the canonical URL.
- [ ] `permission inspect` against a copied file and against a moved file still fails with `invalid stored value for member_runs.database_identity_json: selected database identity mismatch`.
- [ ] `SPEC.md` and `assets/skills/cafleet/reference/cli.md` describe the URL on every action argv and the `HOME` read it adds, with no remaining text saying permission/question argv omit it.
- [ ] `mise //:test` and `mise //:lint` pass.

Criteria 2 and 3 are verified by the `src/config.rs` decision test plus the two caller tests in § Tests: both callers delegate to `non_default_database_url`, and each caller's test asserts its rendering of that function's result for the same default and non-default inputs.

---

## Background

`src/broker/runtime/events.rs` builds `director_action` in two places:

| Action kind | Builder | URL today |
|---|---|---|
| `review_permission_request`, `review_question` | `director_action(run, kind, classification, event_id, interaction_id)` | Never appended |
| `review_stalled_delivery` | `review_stalled_delivery_action(conn, run)` → `member_capture_argv(conn, member_id)` | `action_database_args(conn)` |
| `reconcile_uncertain_input` | `reconcile_uncertain_input_action(conn, blocker)` | `action_database_args(conn)` |

`action_database_args(conn)` forms `sqlite:///{conn.path()}` and appends `["--database-url", url]` when that string differs from `config::default_database_url()`. The URL therefore depends on the path of whichever connection builds the payload.

`get_event` re-derives the complete payload with `payload(...)` and requires JSON equality with the stored `payload_json` (`invalid stored value for payload_json: stored payload does not match its event`). An inbox event also requires the message text to equal the stored payload bytes, and the Director normally reads the event through `message poll`, i.e. those stored bytes. Two consequences follow:

- Appending the URL in `director_action` from the connection path would make every permission/question event unreadable through a copied or moved file, and the two stalled-delivery actions already have that weakness.
- A recovery publisher runs inside a Director command whose `--database-url` may be relative or pass through a symlink, so its stalled events can carry a different URL string from live-published ones.

Every run already records `member_runs.database_identity_json` = `{"canonical_path","instance_id","device","inode"}` at `member create` (SPEC.md §6.9 Runner boundary and creation transaction), validated on load. `member create` already derives `{database_url}` from that canonical path in `spawn_database_url` (`src/cli/member.rs`): `default` when the canonicalized default path equals it, otherwise `sqlite:///<canonical path>`.

---

## Specification

### Decision: derive the URL from the run's recorded canonical path

Every action argv's URL comes from `run.database_identity_json["canonical_path"]`, decided by the same default rule as `{database_url}`. The run row is immutable, so building the payload at write time and re-deriving it at read time produce the same argv regardless of the connection path, and the exact stored-payload comparison stays as is. The URL also matches the members' `DATABASE URL:` line and is canonical for every publisher, live or recovery.

Two alternatives were rejected. Ignoring a trailing `--database-url` pair during the comparison would stop validating that value. Rendering the URL from the reading database at read time cannot fix the issue, because `message poll` returns the stored bytes, which would still lack the URL.

### Shared default decision

Move the decision out of `src/cli/member.rs` into `src/config.rs`, beside `default_database_url`, as a pure function of its two inputs:

```rust
/// `None` when `canonical_path` is the default database, otherwise its `sqlite:///` URL.
pub(crate) fn non_default_database_url(
    canonical_path: &str,
    default_url: &str,
) -> Result<Option<String>, CafleetError>
```

| Condition | Result |
|---|---|
| `std::fs::canonicalize(db::database_path(default_url)?)` succeeds and equals `Path::new(canonical_path)` | `Ok(None)` |
| The default path canonicalizes to another path | `Ok(Some(format!("sqlite:///{canonical_path}")))` |
| `std::fs::canonicalize` of the default path fails with any error (not found, permission denied, symlink loop, a non-directory component) | `Ok(Some(format!("sqlite:///{canonical_path}")))` |
| `default_url` has no valid file path | The `db::database_path` error |

An unresolvable default path is a definite non-default answer, not a masked failure. The question is only whether a command that omits `--database-url` would open the run's recorded file. A default path that cannot be resolved opens no file at all, so the explicit URL is the only argv that reaches the run's database. This keeps `spawn_database_url`'s current `is_ok_and` behavior unchanged.

`spawn_database_url` in `src/cli/member.rs` becomes the pure `spawn_database_url(canonical_path, default_url)`, returning `non_default_database_url(canonical_path, default_url)?` mapped with `.unwrap_or_else(|| DEFAULT_DATABASE.to_owned())`. Its caller passes `&config::default_database_url()?`. The `default` value is the documented `{database_url}` rendering, not a fallback, and member create behaves exactly as before. An unset or non-UTF-8 `HOME` fails every caller of `default_database_url` with the existing `HOME is not set; cannot derive the default database path` / `HOME is not valid UTF-8; cannot derive the default database path` errors.

### Action argv construction (`src/broker/runtime/events.rs`)

A pure renderer carries the decision, and `action_database_args` feeds it the run's recorded path and the current default URL:

```rust
fn database_url_args(canonical_path: &str, default_url: &str) -> Result<Vec<String>, CafleetError> {
    Ok(match crate::config::non_default_database_url(canonical_path, default_url)? {
        Some(url) => vec!["--database-url".into(), url],
        None => Vec::new(),
    })
}

fn action_database_args(run: &RunRecord) -> Result<Vec<String>, CafleetError> {
    let canonical_path = run.database_identity_json["canonical_path"]
        .as_str()
        .expect("a loaded run's validated database identity carries its canonical path");
    database_url_args(canonical_path, &crate::config::default_database_url()?)
}
```

| Builder | Change |
|---|---|
| `member_capture_argv` | Signature `(run: &RunRecord)`; uses `run.member_id`; extends with `action_database_args(run)?` |
| `review_stalled_delivery_action` | Drops its `conn` argument; calls `member_capture_argv(run)` |
| `reconcile_uncertain_input_action` | Signature `(run: &RunRecord, blocker: i64)`; extends with `action_database_args(run)?` |
| `stalled_delivery` | Passes `run` to both builders (it keeps `conn` for its input/message lookups) |
| `director_action` | Builds `inspect_argv` as a `Vec<String>` ending in `"--json"`, then extends it with `action_database_args(run)?` |

The resulting shapes for a non-default database:

```json
["cafleet","permission","inspect","108","--event-id","481","--from-member-id","91","--json","--database-url","sqlite:////srv/fleet.db"]
["cafleet","member","capture","92","--json","--database-url","sqlite:////srv/fleet.db"]
["cafleet","input","show","5120","--json","--database-url","sqlite:////srv/fleet.db"]
```

`review_question` uses the same argv as `review_permission_request`. Key order, instructions and every other payload field are unchanged. No schema change is made: the decision needs nothing beyond the existing `member_runs.database_identity_json`.

### Read-time behavior and stated limits

`get_event` keeps its exact comparison. Reading a non-default database through a copied or moved file re-derives the URL of the original canonical path, which is what the stored payload holds. `permission inspect` keeps its earlier run-identity check, so a copied file (another inode and path) and a moved file (same inode, another path) are both still refused with `invalid stored value for member_runs.database_identity_json: selected database identity mismatch`.

The default decision reads `HOME` and canonicalizes the default path when a payload is built and again when it is re-derived. These cases are the stated limit of the contract; each fails `get_event` with `invalid stored value for payload_json: stored payload does not match its event`, exit 1:

- The reader's `HOME` differs from the writer's, so one side sees the run's database as the default and the other does not.
- A copy or move of the default database is read while nothing exists at the default path, so the re-derived argv carries a URL the stored one lacks.

The second limit is new for permission and question events. Today their argv carries no URL, so they re-validate through a moved default database. Stalled-delivery events already fail there, because their URL comes from the reading connection's path. SPEC.md §6.9 records this narrowed guarantee.

Building or re-validating any action now reads `HOME`. Permission and question events join the stalled-delivery events here, so a command with an explicit `--database-url` and no `HOME` fails with the existing `HOME` error when it builds or re-validates an event that carries an action. A runner without `HOME` takes the existing rejection path (`runner_degraded` with cause `event_rejected`) for such a transition. No new error string is introduced.

### Documentation changes

The owner of the rule is SPEC.md §6.9 *Persistence, notification and read interfaces*; every other mention becomes a pointer.

| Location | Change |
|---|---|
| SPEC.md §6.3 *Database selection* (the `HOME` paragraph) | Add: building or re-validating a runtime event's `director_action` reads `HOME` for the same default decision as `{database_url}`, even under an explicit `--database-url` (§6.9 Persistence, notification and read interfaces). |
| SPEC.md §6.3 *Error Messages* (the `HOME` row, condition `(any command without --database-url)`) | Widen the condition to `(any command without --database-url, and any command that builds or re-validates a runtime event's director_action)`, keeping the two error strings verbatim. |
| SPEC.md §6.3 *Runtime event and input commands* (re-derived fields table, `director_action` row) | State that the argv's database URL pair comes from the run's recorded database identity, never the reading connection. |
| SPEC.md §6.9 *Persistence, notification and read interfaces* (the paragraph after the payload example) | Add the owning rule. Every non-null action's `inspect_argv` ends with `"--database-url","sqlite:///<canonical path>"` after `"--json"` when the run's recorded `canonical_path` (§ Runner boundary and creation transaction) is not the default database under the `{database_url}` rule (§6.3 Spawn-prompt substitution), and carries none for the default database. Also state the non-default copy/move re-validation guarantee, the two stated limits with the payload-mismatch error, and label the existing JSON example as a default-database payload. |
| SPEC.md §6.9 *Input delivery stalled event* | The two `director_action` shape sentences append "followed by the database URL pair for a non-default database (§ Persistence, notification and read interfaces)"; replace the sentence saying only the stalled-delivery arrays carry the URL with that same pointer. Label the JSON example as a default-database payload. |
| SPEC.md §6.9 *Request-bound permission responses* | "Build inspection argv from validated stored identities" also covers the run's database URL pair (§ Persistence, notification and read interfaces). |
| `assets/skills/cafleet/reference/cli.md` database-selection paragraph | Add one clause: a command that builds or reads a runtime event with a Director action reads `HOME` even with an explicit `--database-url`, and fails with the same `HOME` errors. |
| `assets/skills/cafleet/reference/cli.md` runtime events (the `director_action` paragraph, the `input_delivery_stalled` paragraph, the archive paragraph) | State once that for a non-default database every `inspect_argv` ends with `--database-url URL`, so the Director runs it as given; delete "a permission or question `inspect_argv` does not, so add `--database-url URL` yourself when running it". |

`assets/skills/cafleet/roles/director.md` has no step telling the Director to add the URL by hand, so it stays unchanged; `docs/` and `README.md` have no mention and stay out of scope.

### Tests

All tests are in-file unit tests. Related cases go in one table-driven test over a named case array, and every assertion message names its case. "Moved" means `std::fs::rename` of the fixture's database file itself (same inode, new path); "copied" means `std::fs::copy` to a new path.

| File | Test | Asserts |
|---|---|---|
| `src/config.rs` | `non_default_database_url_is_none_only_for_the_canonical_default_path` | Over temp-dir default URLs: an existing default file returns `None` for its own canonical path. It returns `Some("sqlite:///<path>")` when the default file is another existing file, when it does not exist, and when its path runs through a non-directory component. |
| `src/cli/member.rs` | `spawn_database_url_renders_the_shared_default_decision` | For a default URL equal to the database's canonical path, it renders `default`; for another existing default file and a missing one, it renders `sqlite:///<canonical path>`. |
| `src/broker/runtime/events.rs` | `database_url_args_render_the_shared_default_decision` | For the same three inputs, `database_url_args` returns `[]` for the default case and `["--database-url","sqlite:///<canonical path>"]` for both non-default cases. |
| `src/broker/runtime/events.rs` | `every_director_action_argv_ends_with_the_run_database_url` | Cases: permission opening, question opening, stalled head, stalled preview, stalled blocker. Each `inspect_argv` ends with `"--json","--database-url",format!("sqlite:///{canonical_path}")`, where `canonical_path` is the fixture run's recorded `database_identity_json.canonical_path` (a fixture temp database is non-default). |
| `src/broker/runtime/events.rs` | `events_revalidate_through_a_copied_or_moved_database_file` | Cases: copied and moved. Over a fixture holding one event of each action kind, `get_event` on a connection to the new path succeeds for every event, and each argv still names the original canonical path. |
| `src/broker/runtime/events.rs` | `an_event_persisted_through_a_non_canonical_path_carries_the_canonical_url` | A stalled event persisted through a connection opened on a symlink to the fixture database carries `sqlite:///<canonical path>`, not the symlink path. |
| `src/runtime/input_client.rs` | `permission_inspect_names_the_run_database_identity_field_on_a_mismatch` | Converted to a table over copied and moved. `inspect_permission` on a connection to the new path fails with field `member_runs.database_identity_json`, value `selected database identity mismatch`, its rendered message and exit 1. |

Existing tests to update:

- `src/broker/runtime/events.rs` `versioned_events` helpers `action` and `question_action` append the fixture run's URL pair to the expected `inspect_argv`.
- `src/broker/runtime/events.rs` blocker-record test (the expected argv built around `fixture.conn.path()`) builds the URL from the run's recorded canonical path.
- Any other expected action argv that `mise //:test` reports. `src/runtime/input.rs` checks only the argv prefix and needs no change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Specification and skill documentation

- [ ] Update SPEC.md §6.9 *Persistence, notification and read interfaces* with the owning action-argv database URL rule, the non-default copy/move guarantee, the stated limits and the default-database example label <!-- completed: -->
- [ ] Update SPEC.md §6.9 *Input delivery stalled event* (pointers and example label) and *Request-bound permission responses*, removing the "permission or question `inspect_argv` never carries it" sentence <!-- completed: -->
- [ ] Update SPEC.md §6.3 *Database selection* (`HOME` read for action building and re-validation), the §6.3 *Error Messages* `HOME` row condition and §6.3 *Runtime event and input commands* (`director_action` re-derivation source) <!-- completed: -->
- [ ] Update the `assets/skills/cafleet/reference/cli.md` database-selection paragraph with the `HOME` clause <!-- completed: -->
- [ ] Update `assets/skills/cafleet/reference/cli.md` runtime event paragraphs so every `inspect_argv` carries the URL for a non-default database and the Director runs it as given <!-- completed: -->

### Step 2: Shared default decision

- [ ] Add `config::non_default_database_url(canonical_path, default_url)` to `src/config.rs` with the table-driven test `non_default_database_url_is_none_only_for_the_canonical_default_path` <!-- completed: -->
- [ ] Make `spawn_database_url(canonical_path, default_url)` in `src/cli/member.rs` delegate to `non_default_database_url`, pass `&default_database_url()?` at its call site and add `spawn_database_url_renders_the_shared_default_decision` <!-- completed: -->

### Step 3: Action argv from the run identity

- [ ] Add `database_url_args(canonical_path, default_url)` and change `action_database_args` to take `&RunRecord`, feeding it `database_identity_json.canonical_path` and `default_database_url()?` <!-- completed: -->
- [ ] Update `member_capture_argv`, `review_stalled_delivery_action`, `reconcile_uncertain_input_action` and `stalled_delivery` to pass the run <!-- completed: -->
- [ ] Append `action_database_args(run)?` to the permission/question `inspect_argv` in `director_action` <!-- completed: -->

### Step 4: Tests

- [ ] Update the `versioned_events` `action` and `question_action` helpers and the blocker-record expected argv in `src/broker/runtime/events.rs` <!-- completed: -->
- [ ] Add `database_url_args_render_the_shared_default_decision` <!-- completed: -->
- [ ] Add `every_director_action_argv_ends_with_the_run_database_url` <!-- completed: -->
- [ ] Add `events_revalidate_through_a_copied_or_moved_database_file` <!-- completed: -->
- [ ] Add `an_event_persisted_through_a_non_canonical_path_carries_the_canonical_url` <!-- completed: -->
- [ ] Convert `permission_inspect_names_the_run_database_identity_field_on_a_mismatch` in `src/runtime/input_client.rs` to a table over copied and moved files <!-- completed: -->
- [ ] Fix any other expected action argv `mise //:test` reports <!-- completed: -->

### Step 5: Verification

- [ ] Run `mise //:test` and `mise //:lint` and confirm both pass <!-- completed: -->
- [ ] Confirm no text in `SPEC.md` or `assets/skills/` still says a permission or question `inspect_argv` omits `--database-url` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-27 | Initial draft |
