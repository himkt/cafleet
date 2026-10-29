# Member Automatic Approval Modes

**Status**: Approved
**Progress**: 13/31 tasks complete
**Last Updated**: 2026-10-29

## Overview

Make each backend's native automatic approval mode the omitted member launch policy for Claude (`--permission-mode auto`) and Codex (`--approve-for-me`); another mode applies only when the user explicitly asks for it, through an explicit `--permission-mode` choice. Extend the bundled Claude Code 2.1.302 and Codex 0.163.0 profiles from native captures so the member PTY runner reports an automatic refusal and unavailable automatic review to the Director as their own durable events, and records the observed availability of automatic review on the run. Native permission forms that still appear in automatic mode continue through the existing `interaction_opened` and `permission respond` flow. A native version without its own bundled profile runs under the backend's newest bundled profile, recorded as `profile_match=fallback`, so an unchanged native UI keeps working without a new capture.

## Success Criteria

- [ ] An omitted `--permission-mode` launches Claude with `--permission-mode auto` and Codex with `--approve-for-me`, recording `resolved_mode` `auto` / `approve-for-me` and `mode_source=default`; OpenCode still launches `--agent cafleet`.
- [ ] Every previous choice remains explicit: Claude `dontAsk` and the other native values, Codex `on-request` and `never` with their current argv, OpenCode `cafleet` and `cafleet-ask`; catalog-derived help and the usage error list `approve-for-me` and the new defaults.
- [ ] The Director guidance spawns every member with the omitted default and passes `--permission-mode` only when the user explicitly requests that mode for the member: a non-automatic choice for Claude or Codex, `cafleet-ask` for OpenCode.
- [ ] The next contiguous current-family migration (V6) adds the event kinds `automatic_refusal_observed` and `automatic_review_unavailable`, preserves every existing row, sequence and foreign key, and the derived chain and schema-shape guards pass at head V6.
- [ ] A Claude 2.1.302 member and a Codex 0.163.0 member record bundled profiles `claude-2.1.302-auto-1` and `codex-0.163.0-approve-for-me-1` with `coverage=tested` and `profile_match=exact`, each profile meets its § Default-switch gate, and every rule it claims rests on native captures at that exact version.
- [ ] A captured automatic refusal produces exactly one `automatic_refusal_observed` event and Director inbox record with `director_action.kind=review_automatic_refusal`, creates no interaction row, leaves the frame's input eligibility unchanged and offers no `permission respond`.
- [ ] A captured unavailable-review screen produces one `automatic_review_unavailable` event with `director_action.kind=review_automatic_review_unavailable`, and `effective_policy_json` records `automatic_review=unavailable` with its `rule_id` and `event_id`; `observed_mode` holds the displayed mode only under `redacted` evidence and is null under `metadata`. The member keeps running, and `member show` exposes the recorded policy.
- [ ] A member whose probed native version has no bundled profile (for example Claude Code 2.1.306) records the backend's newest bundled profile with `profile_match=fallback`, the profile's capabilities and the fallback warning; on an unchanged native UI it detects, delivers prompts and answers forms as under its exact version, and a run with no profile records `profile_match=null` and `coverage=unknown` as today.
- [ ] A native permission form shown in automatic mode opens the existing `interaction_opened` episode and is answered through `permission respond` exactly as under the interactive policies.
- [ ] On both tmux and herdr, live acceptance on each backend covers the omitted-default launch, an auto-approved fixture, an auto-refused fixture and, where the native host shows one, an escalated form answered allow and deny; each uncaptured screen is a documented limit.
- [ ] Member and Director guidance, `docs/`, `SPEC.md` and the skill references describe the new defaults, both events and their Director handling, and `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

Design [0000192](../0000192-member-auto-approval/design-doc.md) selected automatic member policies and routed returned refusals to the Director through member-authored messages, and was aborted in favor of [0000194](../0000194-member-pty-permission-events/design-doc.md). 0000194 added the member PTY runner, durable runtime events, per-member `--permission-mode` selection and request-bound `permission respond`, with omitted defaults Claude `dontAsk`, Codex `never` + `workspace-write` and OpenCode `cafleet`. [0000196](../0000196-opencode-ask-permission-forms/design-doc.md) added OpenCode `cafleet-ask` and the focus-gated answer stages. Issue [#502](https://github.com/himkt/cafleet/issues/502) asks for automatic approval modes on that runtime.

The shipped launch catalog (`CodingAgent::policy_choices`) already accepts Claude `auto` for 2.1.302, with no automatic-mode evidence behind it. Codex 0.163.0 accepts `on-request` and `never`; its `codex --help`, checked by the Director, lists:

```text
--approve-for-me  Route approval requests through automatic review using the workspace-write sandbox
-c, --config <key=value>  Override a configuration value that would otherwise be loaded from ~/.codex/config.toml
-a, --ask-for-approval <on-request|never>
-s, --sandbox <read-only|workspace-write|danger-full-access>
```

Bundled profiles are keyed by backend and native version, not by mode, so one profile per version serves every policy of that backend. Selection requires an exact version match, so every native update (the installed Claude Code is already 2.1.306) drops a member to `coverage=unknown` until its UI is recaptured, even when the UI is unchanged; rules match screen patterns, and a changed UI already fails to match. The detector (`Detector::observe`) produces one primary observation per frame: it matches the profile's `detection` and `composer` rules and continuation rules, keeps the highest priority, and turns a conflict into `unknown_interaction` or `unknown`. The existing `permission_denied` observation is such a primary observation; the episode tracker emits `permission_denied_observed` once per stable result and resets that identity after a 250 ms profile-verified noninteraction boundary (an idle-composer rule). No bundled profile declares a `permission_denied` rule today.

The runtime event kind is a closed SQL CHECK domain created in V4; `member_permission_responses.event_id` (V5) references `member_runtime_events`. `effective_policy_json` exists on `member_runs` and stays null until an actual native observation supplies it. In the 0000194 acceptance environment the user's Codex `approvals_reviewer = "auto_review"` configuration answered escalations without a native form and was disabled for those captures; this design's acceptance needs automatic review active.

---

## Specification

### Scope

| In scope | Out of scope |
|---|---|
| New omitted defaults for Claude (`auto`) and Codex (`approve-for-me`); the explicit `approve-for-me` choice; Director spawn guidance for the defaults | OpenCode policies, presets and its default |
| Bundled profiles `claude-2.1.302-auto-1` and `codex-0.163.0-approve-for-me-1` built from native captures, and the profile `annotations` capability | Native versions other than Claude Code 2.1.302 and Codex 0.163.0 |
| Event kinds `automatic_refusal_observed` and `automatic_review_unavailable`, their `director_action` and the V6 migration | A request-bound answer to an automatic refusal (no native request is waiting) |
| The observed availability of automatic review in `effective_policy_json` and in the shared runtime snapshot | Forcing the Codex reviewer on or off for the explicit `on-request` / `never` choices |
| Profile version fallback for every backend: the newest bundled profile serves an unprofiled native version, recorded in `member_runs.profile_match` | Version ranges or per-rule version declarations |
| Member and Director guidance, `docs/`, `SPEC.md` and skill references for the defaults and events | Changes to the Director's own session settings or the Director profiles |

The runner, episode, inspection, response, receipt, recovery and Director-continuation contracts of 0000194 and 0000196 apply unchanged except where this document states otherwise.

### Launch policy

The default resolution (`CodingAgent::default_policy`) changes for Claude and Codex. The Codex catalog gains `approve-for-me`, appended after the existing values so existing help order is preserved.

| Backend | Omitted selection | Explicit choices (native version) | Permission argv |
|---|---|---|---|
| Claude | `auto` | `default`, `manual`, `dontAsk`, `acceptEdits`, `auto`, `bypassPermissions`, `plan` (2.1.302 or unprobed) | `--permission-mode <mode>` (unchanged mapping) |
| Codex | `approve-for-me` | `on-request`, `never`, `approve-for-me` (0.163.0 or unprobed) | `on-request` / `never`: `--ask-for-approval <mode> --sandbox workspace-write` (unchanged); `approve-for-me`: `--approve-for-me` |
| OpenCode | `cafleet` | `cafleet`, `cafleet-ask` (every version) | `--agent <mode>` (unchanged) |

- The Codex token `approve-for-me` names its native flag, as the other Codex tokens name their native `--ask-for-approval` values. Codex 0.163.0 rejects `--approve-for-me` together with `--sandbox` (`error: the argument '--approve-for-me' cannot be used with '--sandbox <SANDBOX_MODE>'`, exit 2) because the flag itself selects the workspace-write sandbox, so the `approve-for-me` argv is the single flag; `on-request` and `never` keep their explicit `--sandbox workspace-write`.
- The explicit `on-request` and `never` choices keep their current launch and inherit the user's Codex configuration, including any `approvals_reviewer`. Documentation states that such a configuration can answer escalations without a native form, as `docs/docs/spec/member-runtime.md` already records.
- An omitted selection is not validated against the version catalog, as today: a Claude or Codex member whose probed version has no catalog entry still launches with the default. A native version that rejects `auto` or `--approve-for-me` exits immediately and produces the existing `runner_exited` event with its native status; the runner never retries another mode.
- Run records keep the 0000194 launch-intent fields. Omitted Claude: `requested_mode=null`, `resolved_mode=auto`, `mode_source=default`, `permission_argv_json=["--permission-mode","auto"]`. Omitted Codex: `requested_mode=null`, `resolved_mode=approve-for-me`, `mode_source=default`, `permission_argv_json=["--approve-for-me"]`.
- Claude keeps its other spawn arguments (`--prompt-suggestions false`, `--allowedTools Bash(cafleet message *)`, `--name`, model, effort).

Spawn argv with omitted selection:

| Backend | Spawn argv |
|---|---|
| `claude` | `claude --permission-mode auto --prompt-suggestions false --allowedTools <rule>... --name <member-name> <prompt>` |
| `codex` | `codex --approve-for-me <prompt>` |
| `opencode` | `opencode mini --standalone --agent cafleet --prompt <prompt>` |

Catalog-derived help reads `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet)`. An unknown Codex value fails before registration with `--permission-mode for codex must be one of on-request, never, approve-for-me (got 'VALUE')`, exit 2.

### Native screens in automatic mode

Each screen below is captured natively at the pinned version under the CAFleet runner (36 rows by 120 columns, privacy masking per the existing fixture contract) in Step 1. A profile claims a rule only for a screen its captures show; a screen the native host cannot be made to show is a documented limit and has no rule, subject to § Default-switch gate.

| Screen | Observation | Handling |
|---|---|---|
| Auto-approved operation (the reviewer allows and the tool runs) | Existing primary working/composer observations | None. |
| Idle composer in automatic mode | `composer_empty` (existing rule, or an automatic-mode variant when the mode badge changes the composer region) | Existing empty-composer boundary and prompt delivery. |
| Automatic refusal returned to the model (Claude classifier block, Codex automatic-review denial) | Annotation `automatic_refusal` | § Automatic refusal event. |
| Escalation shown as a native permission form | Primary `permission_request` | Existing episode, inspection and `permission respond` flow. If the form matches the existing `claude-2.1.302-default` / `codex-0.163.0-on-request` request and answer rules, those rules are reused unchanged; a different form gets its own rule and answer plans with pre-stage fixtures. |
| Automatic review unavailable or degraded (for example auto mode unavailable for the account or model, a fallback to prompting, or an unavailable reviewer) | Annotation `automatic_review_unavailable` | § Unavailable automatic review event. |
| Positive automatic-mode indicator (for example a footer mode badge) | Annotation `automatic_review_available` | Records `automatic_review=available` in `effective_policy_json`. |
| Startup notice or opt-in dialog for automatic mode | Primary `question` when captured, otherwise `unknown_interaction` | § Startup dialog. |

### Startup dialog

Step 1 establishes, per backend, whether launching the pinned version in its automatic mode under the runner shows a startup notice or opt-in dialog: on a fresh native configuration, then again on a second launch with the same configuration after the dialog was accepted once.

| Step 1 outcome | Consequence |
|---|---|
| No dialog under the runner | No action. |
| Dialog once per native configuration, persisted after acceptance | The default switch proceeds. The profile adds a `question` detection rule for the dialog from its capture, with no answer plan, so a held member produces `interaction_opened` with classification `question`. Operator documentation adds a one-time setup step: accept the dialog once in an interactive session of the pinned version with the same configuration directory. The Director handles a held startup dialog by asking the user to answer it on the member's native surface; it sends no answer itself. |
| Dialog on every launch | The § Default-switch gate fails for that backend. |

### Default-switch gate

Each backend switches its omitted default and claims `coverage=tested` for its new profile only when Step 1 establishes all four conditions:

1. The pinned version parses and starts with the new default argv under the runner.
2. No startup dialog appears on every launch (§ Startup dialog).
3. An idle-composer rule matches the automatic-mode idle screen as a verified noninteraction, so prompt delivery and episode closure work.
4. An `automatic_refusal` annotation rule is captured.

When a condition fails for a backend, Step 1 stops and reports the failure to the Director before Step 2; the user then decides between keeping that backend's previous default and revising this design. The other screens are optional per backend, and each missing one is a documented limit. If neither backend produces an unavailable-review screen, the report says so and the user decides whether `automatic_review_unavailable` remains in scope.

### Bundled profiles

| Profile | Applies to | Contents |
|---|---|---|
| `claude-2.1.302-auto-1` | Every Claude Code 2.1.302 member, under any policy | Every rule and answer plan of `claude-2.1.302-default-3`, unchanged, plus the captured automatic-mode rules |
| `codex-0.163.0-approve-for-me-1` | Every Codex 0.163.0 member, under any policy | Every rule and answer plan of `codex-0.163.0-on-request-1`, unchanged, plus the captured automatic-mode rules |

Each profile replaces its predecessor as the version's bundled profile; earlier runs keep their recorded `profile_version` text. Profile names follow the existing pattern `<backend>-<version>-<mode>-<revision>`, where the mode names the policy whose captures the revision added.

New rules, each declared with the existing rule fields (`id`, `priority`, `observation`, `region`, `matchers`, `extract`, `verified_noninteraction`, `sensitive`):

| Rule id | `observation` | Capability group | `extract.operation` |
|---|---|---|---|
| `claude-auto-refusal` | `automatic_refusal` | `annotations` | Required: the refused tool call of a verbose-view block (`● Tool(<operation>)` followed by `⎿  Denied by auto mode classifier ∙ [<category>] ∙ …`) |
| `claude-auto-refusal-notice` | `automatic_refusal` | `annotations` | Required: the default-view status-row notice text `<tool> denied by auto mode · [<category>]`; the refused command itself is not displayed |
| `claude-auto-unavailable` | `automatic_review_unavailable` | `annotations` | `null` |
| `claude-auto-mode-indicator` | `automatic_review_available` | `annotations` | `null` |
| `claude-auto-ask-permission` | `permission_request` | `detection` | Required: the Bash operation of the ask-rule form; answer plans allow `31` (`1. Yes`) and deny `32` (`2. No`), both ungated |
| `codex-auto-review-refusal` | `automatic_refusal` | `annotations` | Required: the refused operation |
| `codex-auto-review-indicator` | `automatic_review_available` | `annotations` | `null` |

Claude Code 2.1.302 renders an automatic refusal in one of two native forms depending on the user's Verbose output setting, and the profile claims both. The default view shows only a transient status-row notice above the composer border for about 8–10 seconds; it names the tool and category but not the command, so refusals of the same tool and category produce one fingerprint and one event. The verbose view keeps one transcript block per refused call with its command. The Claude escalated form appears in automatic mode only through an explicit `permissions.ask` rule; it differs from the `claude-bash-permission` form (an `Ask rule … overrides auto mode for this command.` row, a `/permissions to let auto mode decide` row, `2. No` as the deny choice), so it has its own request rule and answer plans. Claude needs no startup rule: the first launch shows only a non-blocking transcript notice.

Codex 0.163.0 in `approve-for-me` renders an automatic refusal as a persistent column-0 transcript line `✗ Request denied for codex to run <cmd>` (model quotes are never at column 0) and shows its positive indicator only in the `/status` block (`Permissions: Workspace (Approve for me)`). It showed no startup dialog, no native permission form (every escalation went to the reviewer) and no unavailable-review screen, so the Codex profile claims no startup, unavailable or new request rule; the existing `codex-0.163.0-on-request-1` request and answer rules stay unchanged for the interactive policies. Codex truncates commands longer than about 70 characters, so refusals sharing the displayed prefix share a fingerprint.

An automatic-mode composer variant, when Step 1 needs one, is an ordinary `composer_empty` rule in the `composer` group.

### Profile version fallback

Profile selection after the native version probe changes for every backend:

| Probe result | Selected profile | `profile_version` | `profile_match` | `coverage` |
|---|---|---|---|---|
| Version with a bundled profile | That profile | Its name | `exact` | `tested` when it declares detection, otherwise `unknown` (unchanged) |
| Version without a bundled profile, backend has bundled profiles | The backend's profile with the highest declared native version | Its name | `fallback` | Computed from the selected profile exactly as for `exact` |
| Version without a bundled profile, backend has none | None | `null` | `null` | `unknown` (unchanged) |
| Probe failure | None | `null` | `null` | `unknown` (unchanged) |

- Versions compare by their dotted numeric components, left to right; a missing component counts as `0`. The selection does not depend on whether the probed version is newer or older than the declared one.
- Every capability lookup keyed by backend and native version (detection, composer, submission and answer plans, `NativeCapabilities`) resolves through the selected profile, so a fallback run behaves exactly as an exact run of the profile's declared version, and `native_version` still records the probed version.
- The rules match screen patterns, so a changed UI fails safe: an unmatched form or composer yields the existing `unknown` / `unknown_interaction` observation, holds programmatic input and offers no answer plan. An answer plan writes only after its request rule, choices and gates matched the fresh observation, as today.
- `member create` writes one stderr line for a fallback, `Warning: no bundled profile for BACKEND VERSION; using PROFILE (profile_match=fallback).`, and leaves stdout text and JSON otherwise unchanged. A fallback emits no `coverage_unknown` event.
- Profile provenance stays exact-version: a profile declares only the versions its captures were taken on, and catalog validation is unchanged. A fallback adds no provenance; acceptance on an unprofiled version is the recorded evidence that its UI is unchanged.

### Profile schema extension

| Element | Change |
|---|---|
| `ObservationKind` | Gains the annotation kinds `automatic_refusal`, `automatic_review_unavailable` and `automatic_review_available`, parsed from and named by those exact strings. |
| `capabilities` | Gains the required key `annotations`, either `null` or `{ "rule_ids": [...], "fixture_ids": [...] }`, validated with the existing `capability()` provenance rules (nonempty rule ids, known fixtures, every native version covered). Every bundled profile declares the key; `opencode-2.1.4-cafleet-ask-1` declares `null`. |
| Group membership | `annotations` accepts only annotation kinds; `detection`, `composer` and continuation rules reject them. An annotation rule outside the `annotations` group fails catalog compilation. |
| Annotation rule fields | `extract.choices` is empty, `extract.native_request_id` is `null` and `verified_noninteraction` is `false`; `automatic_refusal` requires an `operation` capture and the other two require `null`. Annotation rules additionally require the field `frame`, `{ "require_patterns": [...], "reject_patterns": [...] }`, with at least one compilable require pattern and zero or more compilable reject patterns; non-annotation rules reject the field. A violation fails catalog compilation. |
| Annotation region matching | An annotation rule yields one block for every qualifying region of its `region`, in row order and non-overlapping, where the existing contract for other rules still requires exactly one qualifying region. |
| Fixture classification | Each automatic-mode fixture is asserted to produce exactly its expected primary observation and exactly its expected annotation set, and no annotation fixture produces a `permission_denied` primary observation. |

### Annotation detection

An annotation is a transcript result or notice, not a live form. Annotations are evaluated on every frame alongside the primary observation and are excluded from its priority and conflict resolution: `Detector::observe` returns the primary observation exactly as today plus the list of annotation blocks, each with its rule id, matched region, post-policy evidence and in-memory fingerprint. Annotations create no `member_interactions` row and leave input eligibility, episode visibility and the primary observation unchanged. They are evaluated only while semantic classification runs, so a degraded runner's observation gap covers them too.

Annotation rules use the region contract of the existing rules, with screen and patterns pinned from the Step 1 captures:

- **Screen.** Each rule declares the screen its captures show. Both pinned TUIs render their live transcript, forms and composer on the alternate screen, as every existing Claude 2.1.302 and Codex 0.163.0 rule declares, so the expected value is `alternate`. Scrollback is excluded, as for every rule.
- **Blocks.** The start and end patterns delimit one refusal or notice block, and matchers require the native result decoration (for example the tool-result prefix), so model text quoting the same words does not match. Every qualifying region yields a block (§ Profile schema extension), so two refusals visible together are two blocks.
- **Live-transcript frame guard.** The rule's `frame.require_patterns` must each match a live-screen line of the same frame, and no `frame.reject_patterns` may match one. The require patterns name lines present only in the live TUI (for example its composer or footer line), and the reject patterns name the transcript viewer's header, both pinned from captures. A frame failing the guard yields no blocks for that rule.

The fingerprint is backend, profile, rule id and the displayed operation (or notice text) normalized by first stripping the extract's `row_margin` from each continuation row, then removing every row break and collapsing each whitespace run to one space, so a resize that rewraps a block, even inside a token, keeps its fingerprint. The event's `operation` evidence still reports the displayed rows, as 0000194 specifies.

The annotation tracker keeps, per annotation rule, a set of fingerprint states:

1. A fingerprint present in a frame's blocks is emitted after two compatible observations at least 100 ms apart, as for every runtime observation; several fingerprints are tracked and emitted independently.
2. An emitted fingerprint is not emitted again until its state resets.
3. A state resets only after the fingerprint has been absent for 250 ms of continuous frames whose primary observation is a verified noninteraction and whose frame guard holds. Frames showing a permission form, another interaction, an overlay, an unknown screen, a guard failure, or a redraw do not advance the reset, so a block hidden by any of them keeps its state and emits nothing when it reappears. A working screen advances the reset only when the profile classifies it as a verified noninteraction; Claude Code 2.1.302 keeps its idle composer visible while working, so its working frames do.
4. `automatic_review_unavailable` is emitted at most once per run per rule id unless an `automatic_review_available` annotation was observed after it.
5. `automatic_review_available` emits no event; it only updates the effective policy record.

The existing `permission_denied` reset uses the verified noninteraction boundary alone because a primary refusal frame and an idle-composer frame exclude each other. An annotation stays visible above the idle composer, so that boundary alone would re-emit a still-visible refusal after every turn; the reset therefore requires the annotation's absence on such frames. An automatic refusal is a separate kind from `permission_denied_observed` because the deciding party is the automatic reviewer and the Director procedure differs (`director_action` `review_automatic_refusal`, where `permission_denied_observed` carries none); the two never match the same block, because one is a primary rule and the other an annotation and the fixture classification above asserts the separation.

Limits stated in the documentation: the event is an alert, not a one-to-one count of native refusals. A second refusal of the same normalized operation emits nothing while the first is visible, or before an idle frame shows the first gone. A refusal rendered and removed within 100 ms is not observed. For Claude in the default view, the notice names no command, refusals of the same tool and category collapse into one event, and a notice that expires while hidden by a form is never observed. The native-quote guard for Claude rests on the U+00A0 the native result prefix carries. The first launch on a brand-new Claude configuration renders on the primary screen, so no rule matches during that session. The member's routed refusal evidence remains the record in each case.

### Automatic refusal event

The event follows the 0000194 runtime-event contract (atomic event plus `input_required` Director inbox message, outbox, idempotent replay, archived recovery and evidence policy) with these values:

| Field | Value |
|---|---|
| `kind` | `automatic_refusal_observed` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` |
| `evidence` | The existing ordered evidence object; `rule_id` names the refusal rule, and `operation` / `operation_rows` / `excerpt` follow the run's evidence policy (null under `metadata`) |
| `director_action.kind` | `review_automatic_refusal` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]`, built from the stored event ID |

The `director_action.instruction` is: "Verify this event. The member's automatic reviewer refused the operation and no native request is waiting. Dispatch evidence comes only from the member's routed request with its exact command, cwd, environment and refusal; this event and screen captures corroborate it. With no routed request, dispatch nothing and ask the member whether the refused operation is still needed when it next reports or goes idle. For a routed request, decide in the Director session's effective auto mode whether to authorize a separate execution through the routed-command path or to refuse; otherwise obtain the user's permission first. Inspect the actual result and the member's continuation afterward." The multi-row sentence of 0000194 is appended under the same `operation_rows` rule.

The event has no linked interaction, so `permission inspect` and `permission respond` naming it fail their existing event/interaction association check before any byte.

### Unavailable automatic review event

| Field | Value |
|---|---|
| `kind` | `automatic_review_unavailable` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` |
| `evidence` | The existing ordered evidence object; `rule_id` names the notice rule; `operation` and `operation_rows` are null; `excerpt` follows the evidence policy |
| `director_action.kind` | `review_automatic_review_unavailable` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]` |

The `director_action.instruction` is: "Verify this event. Automatic review is unavailable for this member, so native permission forms may appear and each needs a decision, and refusals may stop its work. Supervise the member as interactive, and tell the user when the selected automatic mode cannot apply." The member keeps running under the mode the native host actually applies; CAFleet never restarts it in another mode.

### Effective policy record

The runner writes `member_runs.effective_policy_json` only from a tested annotation observation. Shape:

```json
{
  "schema_version": 1,
  "automatic_review": "unavailable",
  "observed_mode": null,
  "rule_id": "claude-auto-unavailable",
  "event_id": 512,
  "observed_at": "2026-10-29T02:30:00Z"
}
```

| Key | Type | Contract |
|---|---|---|
| `schema_version` | integer | `1` |
| `automatic_review` | text | `available` or `unavailable` |
| `observed_mode` | text or null | The native mode name the screen displays, after the evidence policy: null under `metadata`; under `redacted`, the displayed name with the configured patterns applied, or null when the screen names none |
| `rule_id` | text | The observing rule |
| `event_id` | integer or null | The emitted event; null for an `automatic_review_available` update |
| `observed_at` | text | UTC RFC 3339 observation time |

Only a change of `automatic_review` or `observed_mode` writes the record; repeated identical observations write nothing. Readers validate the shape and fail on a malformed record. Absence (`null`) means no tested observation has been made; it is never filled from argv or the selected mode.

| Write | Transaction | Failure behavior |
|---|---|---|
| With `automatic_review_unavailable` | Part of the event's outbox frame and its immediate event transaction, so live commit and recovery replay apply it idempotently with the event | The existing event-transaction behavior |
| `automatic_review_available` update | Submitted to the event worker in observation order and committed through the existing `owned_transaction` (owner epoch, database identity and historical ownership checks), additionally requiring `member_runs.status` `running` or `degraded` | A stale owner fails with the existing `stale ownership token` error and any other status fails as a `member_runs.status` invariant violation; the worker records a local diagnostic, drops that write without retry and continues, so a superseded runner never overwrites the record |

The available update is not spooled: a runner lost before it commits loses only that observation, and the next indicator observation records it again.

### Runtime snapshot presentation

The shared runtime snapshot (`presentation::runtime_snapshot` for JSON, `output::format_runtime_snapshot` for text) inserts `profile_match` (`exact`, `fallback` or `null`) immediately after `coverage` and appends `effective_policy` (the stored object or `null`) as its last key, after `child_start_identity`. The text runtime line becomes `Runtime: run=ID mode=MODE status=STATUS coverage=COVERAGE profile=MATCH review=STATE`, where `MATCH` is the recorded `profile_match` or `-` and `STATE` is the recorded `automatic_review` value or `-`. Every command that renders the snapshot changes accordingly:

| Command | JSON surface | Text surface |
|---|---|---|
| `member create` | `runtime` object | `Runtime:` line |
| `member show` | `runtime` object | `Runtime:` line |
| `event show` | `run` object | Runtime summary line |

`event show` renders the current run, so an existing event read after the upgrade shows the new key and field as well; the event payload itself and its message bytes are unchanged. Steps 4 and 6 enumerate every consumer of the two helpers and tests each surface's exact key order and text line.

### Payload schema version

New events keep `schema_version: 2`. Both kinds fit the existing v2 shape: the reviewer is identified by `evidence.backend` together with the kind, and the effective policy lives on the run, not in the payload. The v2 reader's value domains extend: event `kind` gains the two kinds, and `director_action.kind` gains `review_automatic_refusal` and `review_automatic_review_unavailable`. Existing v1 and v2 records and their message bytes replay unchanged. An older binary never reads the new values, because it refuses the V6 database as ahead of its embedded chain.

### V6 migration

Add `cafleet/migrations/V6__automatic_review_events.sql`. The migration widens the `member_runtime_events.kind` CHECK to:

```sql
CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','runner_degraded','runner_lost','runner_exited'))
```

SQLite cannot alter a CHECK in place, and the grouped migration transaction runs with `foreign_keys=ON`, which cannot be changed inside it. `member_permission_responses.event_id` references `member_runtime_events`, and `member_permission_response_claims` references `member_permission_responses` with `ON DELETE RESTRICT`, so dropping the events table first would fail. Rebuild the three tables in dependency order:

1. Create `member_runtime_events_v6` with the widened CHECK and every other column, constraint and foreign key unchanged; copy all rows. Then carry its `sqlite_sequence` row from `member_runtime_events` exactly as V5 does for `member_input_operations`, before any drop, so event IDs are never reused.
2. Create `member_permission_responses_v6` with `event_id REFERENCES member_runtime_events_v6(event_id)` and otherwise the V5 definition; copy all rows.
3. Create `member_permission_response_claims_v6` with its composite foreign key referencing `member_permission_responses_v6(interaction_id, input_id)` and otherwise the V5 definition; copy all rows.
4. Drop `member_permission_response_claims`, then `member_permission_responses`, then `member_runtime_events`; each drop has no remaining referencing rows, and the last removes the old sequence row.
5. Rename `member_runtime_events_v6`, `member_permission_responses_v6` and `member_permission_response_claims_v6` to their final names in that order. SQLite's non-legacy rename rewrites the referencing foreign keys to the final names and renames the `_v6` sequence row with its table.
6. Add the profile-match column in place and backfill existing runs, which all selected their profile by exact version. SQLite tests an added column's CHECK against existing rows, and a NULL `profile_match` makes `IN` NULL rather than false, so the CHECK names `IS NOT NULL` explicitly and the profiled runs are stashed and their `profile_version` cleared while the column is added:

```sql
CREATE TEMP TABLE v6_profiled_runs AS
  SELECT run_id, profile_version FROM member_runs WHERE profile_version IS NOT NULL;
UPDATE member_runs SET profile_version = NULL WHERE profile_version IS NOT NULL;
ALTER TABLE member_runs ADD COLUMN profile_match TEXT
  CHECK((profile_version IS NULL AND profile_match IS NULL)
     OR (profile_version IS NOT NULL AND profile_match IS NOT NULL
         AND profile_match IN ('exact','fallback')));
UPDATE member_runs SET
  profile_version = (SELECT p.profile_version FROM v6_profiled_runs p WHERE p.run_id = member_runs.run_id),
  profile_match = 'exact'
  WHERE run_id IN (SELECT run_id FROM v6_profiled_runs);
DROP TABLE v6_profiled_runs;
```

The derived chain guard and schema-shape guard must pass at head V6; both are computed from the embedded registry, and the shape guard compares columns, foreign keys and indexes. The migration tests establish the CHECK widening and data preservation: populate V5 rows in all three tables (including a response and a claim), migrate, and assert identical rows, the preserved `sqlite_sequence` value, an empty `PRAGMA foreign_key_check`, insertion of both new kinds and rejection of an unknown kind, `profile_match='exact'` on every V5 run with a profile and `null` on the others, and rejection of a `profile_match` inconsistent with `profile_version`.

### Director guidance

The Director spawns every member with the omitted `--permission-mode`, which selects the backend's automatic mode for Claude and Codex and `cafleet` for OpenCode. It passes `--permission-mode` only when the user explicitly requests that mode for that member:

| Backend | Omitted default | Explicit choice on the user's request |
|---|---|---|
| Claude | `auto` | A non-automatic native value, such as `default` or `dontAsk` |
| Codex | `approve-for-me` | `on-request` or `never` |
| OpenCode | `cafleet` | `cafleet-ask` |

The Director handles both events through its existing wake, `event show` verification, ACK and host-owned pending-action record.

| Event | Director procedure |
|---|---|
| `interaction_opened` in automatic mode | Unchanged 0000194 procedure, including `permission respond`. A `question` startup dialog follows § Startup dialog. |
| `automatic_refusal_observed` with a routed request | Dispatch evidence comes only from the member's routed request: exact command, absolute cwd, required environment and refusal. The event and captures corroborate it and supply no missing field. Apply the Director-mode rule: in verified auto mode decide to authorize or refuse; otherwise obtain the user's permission first. Authorize through the existing routed-command dispatch (`member exec`) under its eligibility rules, or refuse with a broker reply to the member. Inspect the actual result and continuation. |
| `automatic_refusal_observed` without a routed request | Dispatch nothing; the member may have abandoned the operation or chosen an alternative. ACK and keep the event as an informational pending item. When the member next reports, routes a request or reaches an idle composer without addressing it, ask the member through the broker whether the refused operation is still needed. The item closes when a routed request arrives (then the row above applies) or the member answers that it is not needed. |
| `automatic_review_unavailable` | Record the member as interactive in host-owned supervision state, expect native permission forms from it, and tell the user when the selected automatic mode cannot apply. No answer is sent. |

A routed request that arrives with no event follows the existing refusal route unchanged.

### Member guidance

The member role and the coding-agent overlay describe the default posture as the backend's automatic approval mode: the member attempts role-authorized commands, an automatic refusal is a permission refusal for routing purposes, and the member routes a necessary refused command to the Director with the existing command, cwd, environment and refusal evidence, then stops dependent work. The Claude `{permission_flags}` binding becomes `--permission-mode auto` and the Codex binding becomes `--approve-for-me`; the Claude `dontAsk` note is replaced by the corresponding note for `auto` (native rules, hooks and managed policy remain authoritative; refused work routes to the Director).

### Documentation targets

| Target | Change |
|---|---|
| `docs/docs/spec/member-runtime.md` | Launch-policy defaults and the Codex choice; automatic-mode screens, startup dialog and default-switch gate; annotation detection and the profile schema extension; both events and their `director_action`; effective policy record; profile version fallback and `profile_match` in the native version probe section; profiles and limits |
| `docs/docs/spec/coding-agent-backends.md` | Spawn argv table, shared contract and per-backend posture for the new defaults; the explicit choices and their config inheritance; the one-time startup-dialog setup step when it applies |
| `docs/docs/concepts/member-permission-events.md` | Automatic refusals and unavailable review as distinct observations |
| `docs/docs/spec/cli-options.md` and every other `docs/` page that states a member default policy | Help text and usage error; the fallback warning; the `profile_match` and `effective_policy` keys and the ` profile=MATCH` / ` review=STATE` fields in the `member create`, `member show` and `event show` JSON and text layouts |
| `SPEC.md` | Defaults, catalog, argv, V6 CHECK domain and `profile_match` column, profile selection and fallback warning, event kinds, payload value domains, effective policy shape, and the runtime snapshot key order and text line for `member create`, `member show` and `event show` |
| `skills/cafleet/roles/director.md` | Spawn every member with the omitted default and pass `--permission-mode` only on the user's explicit request for that member, per the § Director guidance table; Director handling of both events and the startup dialog |
| `skills/cafleet/reference/coding-agents.md`, `skills/cafleet/roles/member.md`, `skills/cafleet/reference/member-runtime.md`, `skills/cafleet/reference/prompt-routing.md` | Runtime bindings, notes, member posture and the event procedures |

Find the remaining pages with a repository search for the old default argv strings (`--permission-mode dontAsk`, `--ask-for-approval never`) and for `dontAsk` as a member default; every hit that describes the member default is updated.

### Acceptance

Acceptance uses the shipping release build, a private database under `/tmp`, Claude Code 2.1.302 and Codex 0.163.0 first on `PATH` (Claude 2.1.302 is installed beside the user's 2.1.306 with `mise install aqua:anthropics/claude-code@2.1.302` and selected by prepending its install directory to `PATH`), automatic mode available for the account and model, and a dedicated tmux server or herdr tab per case. Fixture commands are disposable and harmless even if executed:

| Fixture | Purpose | Example |
|---|---|---|
| Auto-approved | The reviewer allows; the command runs | Write a marker file inside the case folder |
| Auto-refused | The reviewer refuses; the command does not run | Write outside the workspace to a disposable `/tmp/cafleet-acceptance-<case>/outside` path, or pipe a fetch from a reserved `.invalid` host into a shell |
| Escalated | The host shows a native permission form | Pinned from the Step 1 captures for each backend |
| Unavailable | The host reports automatic review unavailable | Pinned from the Step 1 captures; omitted when the host cannot be made to show it |

A fallback case repeats the omitted-default launch, auto-approved, auto-refused and escalated cases with the installed Claude Code 2.1.306, which has no bundled profile: it records `profile_match=fallback` and either the same observations as 2.1.302 or each UI difference that failed to match, which becomes a documented limit for that version.

Each case records versions, mode, profile, profile match, multiplexer, terminal size, cwd, events, inbox records, effective policy and the observed native result. A fixture that does not produce its intended screen is a recorded limit for that backend, not a pass. User-attended cases are limited to flows where a user decision is part of the procedure: a non-auto-mode Director decision on an automatic refusal and on an escalated form, and the startup dialog when it applies.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Capture native automatic-mode evidence


- [x] Verify that Codex 0.163.0 parses and starts with `--approve-for-me` and that Claude Code 2.1.302 starts with `--permission-mode auto` under the runner, and establish the § Startup dialog outcome for each on a fresh and a second launch. <!-- completed: 2026-10-29T10:45 -->
- [x] Capture each screen in § Native screens in automatic mode for Claude 2.1.302 `auto` under the runner at 36x120 with privacy masking, including two different refusals visible together, a refusal hidden by a permission form and rewrapped by a resize, the transcript viewer showing a refusal, and false-positive quotes of the refusal text. <!-- completed: 2026-10-29T10:45 -->
- [x] Capture the same screens for Codex 0.163.0 `approve-for-me`, including any escalated form and its allow/deny pre-stage forms. <!-- completed: 2026-10-29T10:45 -->
- [x] Evaluate § Default-switch gate per backend and report its result and every screen that could not be produced to the Director; stop before Step 2 when a gate condition fails. <!-- completed: 2026-10-29T10:45 -->

### Step 2: Update documentation

COMMENT(director): Programmer — start now with the parts that do not depend on Step 1 captures (profile version fallback, V6 migration and `profile_match`, launch-policy defaults and argv, both event kinds and `director_action`, effective policy record, snapshot layouts, Director/member guidance). Leave rule patterns, captured screens, startup-dialog outcome and limits for after the Step 1 gate report; the Director will re-send ready for this step then. Report `complete` only when the capture-independent part is done, and keep this marker until the full step is done.

- [ ] Update every `docs/` target in § Documentation targets with the defaults, Codex choice, screens, startup dialog, gate, annotations, events, effective policy, profile version fallback, snapshot layouts, profiles and limits; pin exact rule patterns from the Step 1 captures. <!-- completed: -->
- [x] Update `SPEC.md` contract surfaces: catalog, defaults, argv, V6 CHECK domain and `profile_match` column, profile selection and fallback warning, event kinds, `director_action` kinds, effective policy shape and the runtime snapshot keys and text line for every listed command. <!-- completed: 2026-10-29T07:57 -->
- [ ] Update the Director role with the omitted-default spawn rule and the event and startup-dialog procedures. <!-- completed: -->
- [x] Update the remaining skill references and the member role for runtime bindings, notes and member posture. <!-- completed: 2026-10-29T07:57 -->
- [x] Search for remaining member-default mentions of `dontAsk`, `--permission-mode dontAsk` and `--ask-for-approval never` and update each one that describes the default. <!-- completed: 2026-10-29T07:57 -->

### Step 3: Add the V6 migration

- [x] Write migration tests: V5 rows in events, responses and claims survive with identical values, `sqlite_sequence` is preserved, `PRAGMA foreign_key_check` is empty, both new kinds insert and an unknown kind is rejected, and `profile_match` is backfilled and constrained. <!-- completed: 2026-10-29T08:05 -->
- [x] Add `V6__automatic_review_events.sql` with the dependency-ordered rebuild and the `profile_match` column, extend `RuntimeEventKind`, and confirm the derived chain and schema-shape guards pass at head V6. <!-- completed: 2026-10-29T08:09 -->

### Step 4: Add profile version fallback

- [x] Write tests: exact, fallback (newer and older probed versions, multi-component comparison), no-profile and probe-failure selection with their `profile_version` / `profile_match` / `coverage`; capabilities resolved through the fallback profile; the fallback warning and absent `coverage_unknown` event; a fallback run detecting and answering a predecessor fixture; and the `profile_match` snapshot key and ` profile=MATCH` text field on `member create`, `member show` and `event show`. <!-- completed: 2026-10-29T08:48 -->
- [x] Implement fallback selection in the catalog and native launch identity, route every version-keyed capability lookup through the selected profile, persist `profile_match` on the run and render it in the shared runtime snapshot. <!-- completed: 2026-10-29T08:48 -->

### Step 5: Change the launch policy

- [x] Write tests: omitted Claude and Codex argv and run-record fields, explicit `approve-for-me`, unchanged explicit `dontAsk` / `on-request` / `never` / OpenCode argv, help text and the Codex usage error. <!-- completed: 2026-10-29T10:26 -->
- [x] Implement the new defaults and the `approve-for-me` catalog entry and mapping in `cafleet/src/coding_agent/policy.rs`, updating the 0000194/0000196 tests whose assertions encoded the old defaults. <!-- completed: 2026-10-29T10:26 -->

### Step 6: Add profiles, annotations and events

COMMENT(director): Approved — implement the wiring (1)–(5) as proposed; the row_margin deviation is accepted (record it in docs/docs/spec/member-runtime.md where the Claude rules are pinned). Tester adds runner-level tests for it: an unavailable frame carrying effective_policy in the outbox and replayed by recovery, an available update committed through the worker in order and dropped with a diagnostic on stale owner, other frames byte-unchanged, and the bounded-update count ignoring annotation drafts. Each removes their half; the last deletes this marker. Proposal: Runner wiring proposal (the spec fixes behavior, not this interface, and it changes the outbox frame that recovery replays). Implemented so far, all building: catalog/detector annotations, both event kinds and `director_action`, `storage::persist_transition_with_effective_policy` / `storage::record_effective_policy` and record validation, snapshot key and ` review=` text, the 28 Claude fixtures registered, `claude-2.1.302-auto-1` built from `proposed-rules-claude.json`, and the accepted tracker interface (plus `ScreenObservation.guarded_annotation_rules`, the annotation rule ids whose frame guard held, which the 250 ms reset needs). Deviation: `claude-auto-refusal` drops the proposed `row_margin: "  "`, because our `row_margin` strips the margin from every row and the refusal's first row (after `● Bash(`) has none, so every single-row refusal would fail to match; `validate_rules.py` ignores `row_margin`, so its replay never exercised it. Operation evidence therefore keeps the displayed continuation rows with their two-space margin, matching the replay output. Proposed wiring: (1) `ObservationController` pairs `update.effective_policy` carrying `Unavailable` with the update's `AutomaticReviewUnavailable` draft and submits it through `EventWriter::try_submit_with_policy(transition, Some(observation))`. (2) That outbox frame body is the `ObservedTransition` JSON plus one top-level `effective_policy` key holding the observation; every other frame keeps exactly its current bytes. (3) The writer worker and `recovery::replay` persist a frame that has the key through `persist_transition_with_effective_policy` and every other frame unchanged. (4) An `Available` observation goes to `EventWriter::try_record_policy(observation)`, queued on the worker in observation order behind already accepted transitions, committed with `record_effective_policy` and not spooled. A failure records a local diagnostic and drops that write without retry or degradation. (5) The bounded-update check in `retain` counts only primary transitions, so annotation drafts can add to one update.

- [ ] Write catalog tests for the profile schema extension: annotation kinds, the required `annotations` group, group-membership and annotation-field violations including `frame`, multi-block region matching, and provenance. <!-- completed: -->
- [ ] Implement the schema extension in the catalog and detector, returning annotations beside the unchanged primary observation; add `"annotations": null` to the OpenCode profile. <!-- completed: -->
- [ ] Add the Step 1 captures as embedded native fixtures with their digests, and the false-positive fixtures. <!-- completed: -->
- [ ] Write fixture-backed tracker tests for each claimed rule: stable-twice emission, two different refusals visible together emitting two events, no re-emission while present, reset only after 250 ms absence on verified-noninteraction frames, no re-emission after a permission form or overlay hides the block or after a resize rewraps it, the unavailable-once rule, quote/viewer/scrollback false positives and frame-guard failures, unchanged primary observations and eligibility on annotation frames, and unchanged request/answer behavior on the predecessor fixtures. <!-- completed: -->
- [ ] Implement `claude-2.1.302-auto-1` and `codex-0.163.0-approve-for-me-1` with the claimed rules and the annotation tracker. <!-- completed: -->
- [ ] Write tests for both event payloads, `director_action` instructions and argv, inbox delivery, archived recovery, idempotent replay, metadata/redacted evidence, and `permission inspect` / `respond` rejection of a refusal event. <!-- completed: -->
- [ ] Implement both event kinds through the existing outbox and event transaction, with no interaction row and no input change. <!-- completed: -->
- [ ] Write tests for the effective policy record: shape validation, change-only writes, `observed_mode` under each evidence policy, replayed event-carried writes, rejected available writes for a stale owner or non-live status, and the snapshot key and text line on `member create`, `member show` and `event show`. <!-- completed: -->
- [ ] Implement the effective policy writes and the shared runtime snapshot presentation. <!-- completed: -->

### Step 7: Validate and accept

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`; verify help, JSON and embedded assets match the documentation. <!-- completed: -->
- [ ] Run live acceptance for Claude on tmux and herdr: omitted-default launch, auto-approved, auto-refused with its event and Director handling, escalated form allow and deny, and unavailable where captured. <!-- completed: -->
- [ ] Run live acceptance for Codex on tmux and herdr with the same cases. <!-- completed: -->
- [ ] Run the § Acceptance fallback case with the installed Claude Code 2.1.306 on one multiplexer, recording `profile_match=fallback` and every UI difference that failed to match. <!-- completed: -->
- [ ] Run the user-attended cases: a non-auto-mode Director decision on an automatic refusal and on an escalated form, and the startup dialog when it applies. <!-- completed: -->
- [ ] Record versions, outcomes, latencies and every uncaptured or unobserved case as a documented limit in the execution record and the shipping documentation. <!-- completed: -->
- [ ] Confirm the explicit interactive policies still pass their existing 0000194/0000196 acceptance shape on one multiplexer with the new profiles. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-29 | Initial draft |
| 2026-10-29 | Added profile version fallback (`profile_match`), its V6 column, snapshot field, Step 4 and the Claude 2.1.306 fallback acceptance case; Claude captures stay on 2.1.302 installed beside 2.1.306 |
| 2026-10-29 | Step 1 gate results: Codex `approve-for-me` argv is the single `--approve-for-me` flag (0.163.0 rejects it with `--sandbox`); Claude claims both refusal renderings (`claude-auto-refusal`, `claude-auto-refusal-notice`) and an ask-rule request rule, needs no startup rule, and its working frames advance the annotation reset; added the resulting limits |
