# OpenCode Ask Permission Forms

**Status**: Approved
**Progress**: 0/17 tasks complete
**Last Updated**: 2026-10-28

## Overview

Add the explicit OpenCode launch policy `cafleet-ask`, backed by a second embedded agent preset whose unmatched shell commands use `ask`, so an OpenCode member shows a native permission form instead of a silent denial. Add a bundled OpenCode 2.1.4 profile, built from CAFleet's own native `mini` captures and measured answer stages, so the member runtime detects that form, notifies the Director and answers it through `permission respond`, as it already does for Claude `default` and Codex `on-request`. The omitted OpenCode policy stays `cafleet`.

## Success Criteria

- [ ] `cafleet member create --coding-agent opencode --permission-mode cafleet-ask` launches `opencode mini --standalone --agent cafleet-ask --prompt <prompt>` and records `requested_mode=cafleet-ask`, `resolved_mode=cafleet-ask`, `mode_source=explicit` and `permission_argv_json=["--agent","cafleet-ask"]`; omitted selection still launches `--agent cafleet` with `mode_source=default`.
- [ ] `cafleet setup --coding-agent opencode` installs both `agents/cafleet.md` and `agents/cafleet-ask.md` under the resolved preset base and prints one preset line per target, `cafleet doctor` reports `opencode.preset` and `opencode.preset.cafleet-ask`, and `member create` requires only the selected policy's preset file.
- [ ] The `cafleet-ask` preset differs from the `cafleet` preset only in its description, its shell base rule `"*": "ask"` and its body text; every allowlist entry, every Director-only shell deny and every non-shell permission is identical.
- [ ] The native version probe records `2.1.4` for OpenCode 2.1.4, whose `--version` prints `opencode v2.1.4`, and an OpenCode 2.1.4 member under either policy records profile `opencode-2.1.4-cafleet-ask-1` and `coverage=tested`.
- [ ] The bundled profile declares tested detection, allow-once and reject-once answers and the empty-composer observation, each resting on native OpenCode 2.1.4 `mini` captures at 36x120; prompt and native-exec submission stay `unknown`, and no answer plan exists for an "always" choice.
- [ ] On both tmux and herdr, a `cafleet-ask` member's shell permission form produces one `interaction_opened` event with the review instruction, and Director-auto allow and deny through `permission respond` reach the original request, close the episode `not_visible` and show the native result and the member's continuation.
- [ ] On one multiplexer, a user-sourced response, a user-executed complete bound command and a manual original-surface answer each reach the original request, with zero Director response bytes before the user's permission.
- [ ] Every affected `docs/` page, `SPEC.md`, skill reference and preset describes the shipped `cafleet-ask` policy, the OpenCode profile and its limits, and `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build` pass.

---

## Background

Design [0000194](../0000194-member-pty-permission-events/design-doc.md) added the member PTY runner, durable permission events, per-member launch-policy selection and request-bound `permission respond`, with bundled profiles `claude-2.1.302-default-3` and `codex-0.163.0-on-request-1`. Its OpenCode adapter accepts exactly `cafleet`, mapped to `--agent cafleet`. That preset (`presets/opencode/cafleet.md`) resolves every shell check to `allow` or `deny`, so an OpenCode member never shows a permission form. No bundled profile exists for OpenCode, so every OpenCode member records `coverage=unknown`, receives a `coverage_unknown` notification and has programmatic input held.

herdr's OpenCode manifest at the pinned commit `5608de0608918aba2f9d501e7793b984a0ca9cfa` ([`src/detect/manifests/opencode.toml`](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/src/detect/manifests/opencode.toml)) classifies `blocked` with rule `permission_required` when the recent screen contains `△ Permission required`, or `esc dismiss` together with `enter confirm` / `enter submit` / `enter toggle` and `↑↓ select` / `⇆ tab`. It matches anywhere in a `whole_recent` region, and it was written for the full OpenCode TUI. CAFleet launches `opencode mini --standalone`, so the CAFleet profile rests on its own `mini` captures and treats the herdr rule as a reference for candidate anchors only.

The user has installed OpenCode 2.1.4 (the documented validated version) on the acceptance machine; `opencode --version` prints `opencode v2.1.4`. The probe's version-token pattern `^[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$` rejects `v2.1.4`, so the current probe records `native_version` null for that output.

Member automatic approval ([#502](https://github.com/himkt/cafleet/issues/502)) follows this design and is out of scope.

---

## Specification

### Scope

| In scope | Out of scope |
|---|---|
| The `cafleet-ask` launch policy, its embedded preset, setup installation, the selected-preset precondition and doctor's per-preset checks | Changes to the `cafleet` launch argv, the catalog default or the `cafleet` preset bytes |
| The native version probe accepting a leading `v` | Member automatic approval (#502) |
| Bundled profile `opencode-2.1.4-cafleet-ask-1`: shell permission detection, allow-once and reject-once answers, empty-composer observation | `ask` for non-shell permissions (`edit`, `external_directory`, `webfetch`, `websearch` and the other preset keys) |
| OpenCode 2.1.4 `cafleet` members moving from `coverage=unknown` (with its `coverage_unknown` notification) to `coverage=tested` with the empty-composer boundary | Prompt and native-exec composer submission plans; multi-stage or focus-dependent answer plans |
| Native acceptance on tmux and herdr | Wrapped or narrow-pane permission forms; OpenCode versions other than 2.1.4 |

The runner, event, inspection, response, receipt and Director-workflow contracts of 0000194 apply unchanged. The `director_action` instruction, including its multi-row sentence rule, is backend-neutral; a single-line OpenCode operation always has `operation_rows` 1.

### Launch policy

The OpenCode adapter's catalog gains `cafleet-ask`. Both choices select an agent preset rather than a native permission flag, so they are accepted for every native version; detection coverage still depends on the probed version.

| Choice | Selection | Permission argv | Preset file |
|---|---|---|---|
| `cafleet` | Omitted default, or explicit | `--agent cafleet` | `<preset base>/agents/cafleet.md` |
| `cafleet-ask` | Explicit only | `--agent cafleet-ask` | `<preset base>/agents/cafleet-ask.md` |

The `--agent` value equals the resolved mode. The spawn argv keeps its shape: `opencode mini --standalone --agent <mode> [--model <m>] --prompt <prompt>`. Catalog-derived help reads `opencode: cafleet, cafleet-ask (default cafleet)`, and an unknown OpenCode value fails with the existing usage error `--permission-mode for opencode must be one of cafleet, cafleet-ask (got 'VALUE')`, exit 2, before registration. Run records store the launch intent exactly as for the other backends.

### The `cafleet-ask` preset

Add `presets/opencode/cafleet-ask.md`, embedded beside `presets/opencode/cafleet.md`. Its frontmatter equals the `cafleet` preset's except:

- `description`: `CAFleet-spawned member whose unmatched shell commands open a native permission form for the Director to answer.`
- `permission.shell["*"]` is `"ask"`.

The shell rules keep their order: the `"*"` base first, then the allowlist, then the Director-only denies (`cafleet member prompt *`, `cafleet member exec *`, `cafleet member press *`, `cafleet member recover *`, `cafleet permission respond *`, `cafleet input reconcile *`). OpenCode selects the last matching rule, so an allowlisted command runs, a Director-only command is denied without a form, and every other shell command asks. `read`, `edit`, `external_directory`, `webfetch`, `websearch`, `repo_clone`, `question`, `plan_enter` and `plan_exit` keep the `cafleet` preset's values, so only shell commands produce a form and the `question` tool raises no competing question form.

The body keeps the `cafleet` preset's member protocol and replaces its permission paragraph with this text:

> You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks, denies the Director-owned CAFleet commands, and asks for every other command. An asked command opens a native permission form; CAFleet notifies the Director, who answers it, and you continue from the result OpenCode returns. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials.

The remaining two body paragraphs (role and assignment authority; poll/ACK and Director-owned pane input) are identical to the `cafleet` preset.

### Setup and spawn precondition

`cafleet setup --coding-agent opencode` installs every embedded OpenCode preset — `opencode/cafleet.md` to `<preset base>/agents/cafleet.md` and `opencode/cafleet-ask.md` to `<preset base>/agents/cafleet-ask.md` — with the existing replace-on-install behavior and one `asset_installs` record. A failure on either preset fails setup with the existing `failed to install preset into <target>: <cause>` error and records no installation.

Setup prints the documented success lines, each after its operation completes: the skills line, then one preset line per installed preset target in install order. The shipped `install_agent` prints only the skills line; this change implements the documented preset line for every backend that has a preset, so Codex gains its one preset line too. `cafleet setup --coding-agent opencode` prints:

```text
opencode: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
opencode: installed preset (v<version>) -> <preset base>/agents/cafleet.md
opencode: installed preset (v<version>) -> <preset base>/agents/cafleet-ask.md
```

The preset requirement becomes a function of the launch-policy choice: `required_preset(config, mode)` returns `<config>/agents/<mode>.md` for OpenCode and nothing for the other backends. `cafleet doctor` has no selected mode, so it emits one check per OpenCode catalog choice, in catalog order, each with the existing `opencode.preset` statuses and messages applied to that choice's file:

| Check name | File | Statuses |
|---|---|---|
| `opencode.preset` | `<preset base>/agents/cafleet.md` | `ok` with the path; `error` with `<path>: <cause>; run 'cafleet setup --coding-agent opencode'`; `info` when configuration failed or the executable is absent |
| `opencode.preset.cafleet-ask` | `<preset base>/agents/cafleet-ask.md` | The same statuses and messages |

The default choice keeps the existing check name, so existing doctor consumers see no rename; every other choice appends `.<mode>`. A missing `cafleet-ask` preset is an `error` because setup installs every preset, so its absence means the installation predates this change or was altered.

`member create` resolves the launch policy before the availability check, and the check reads `<preset base>/agents/<resolved mode>.md`. A missing or unreadable selected preset fails with exit 1 and the shipped error `cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first`, before registration or multiplexer effects. The unselected preset is not consulted. The documentation currently records `opencode agent preset not found at <preset>; …`; this change aligns `SPEC.md`, the CLI reference and the Director role with the shipped string, which carries the read cause.

### Native version probe

The version-token pattern becomes `^v?[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$`, and a leading `v` is removed from the recorded `native_version`. The exactly-one-token rule and every other probe rule are unchanged.

| First stdout line | Recorded `native_version` |
|---|---|
| `2.1.302 (Claude Code)` | `2.1.302` |
| `codex-cli 0.163.0` | `0.163.0` |
| `opencode v2.1.4` | `2.1.4` |
| `v2.1.4 2.1.5` | probe failure: several version tokens |

The bundled profile is keyed by the recorded value `2.1.4`.

### Bundled profile `opencode-2.1.4-cafleet-ask-1`

The profile is keyed by backend and native version, like the Claude and Codex profiles, so it applies to an OpenCode 2.1.4 member under either policy. A `cafleet` member records `coverage=tested` and gains the empty-composer boundary; it shows no permission form because its preset never asks.

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `opencode-shell-permission`: the native `mini` shell permission form, from its heading line to its closing control line within 24 rows, on the screen the captures record, with the single-line operation, the focused choice, the allow-once and reject-once choices, and the cursor inside the form |
| Allow answer | `tested` | Answer rule `opencode-shell-allow-once`: one measured focus-independent stage selecting allow-once, as `allow-once` |
| Deny answer | `tested` | Answer rule `opencode-shell-deny-once`: one measured focus-independent stage selecting reject-once, as `deny-once` |
| Empty-composer observation | `tested` | Rule `opencode-idle-composer`: the idle `mini` composer with the terminal cursor at the end of its prompt prefix; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

Fixtures, each a native capture of OpenCode 2.1.4 `mini` under the CAFleet runner at 36 rows by 120 columns, with privacy masking per the existing fixture contract:

| Fixture ID | Screen | Cited by |
|---|---|---|
| `opencode-2.1.4-ask-permission-1` | The permission form before any answer byte | Detection |
| `opencode-2.1.4-allow-pre-stage-1` | The form immediately before the single allow stage | Allow answer |
| `opencode-2.1.4-deny-pre-stage-1` | The form immediately before the single deny stage | Deny answer |
| `opencode-2.1.4-idle-composer-1` | The idle composer with no interaction | Empty-composer observation |

Each answer is exactly one stage, so one pre-stage fixture per answer proves that stage's `required_rule_id` (`opencode-shell-permission`) and `required_choice_id` (`allow-once` or `deny-once`). Both answer entries use `request_rule_id` `opencode-shell-permission` and `focus_matcher_id` `focus`, the id of the rule's focused-choice matcher.

Rule requirements, pinned to exact patterns from the captures in Step 3:

- **Live region.** Start and end patterns anchor on the form's own heading line and closing control line (herdr's `△ Permission required` and `esc dismiss` / `enter confirm` / `↑↓ select` / `⇆ tab` are candidates to confirm against the `mini` capture), not on absolute rows. `cursor` is `inside`. Text outside that region, including transcript lines and scrollback, cannot open a request.
- **Required matchers.** Ids `heading`, `operation`, `focus`, `allow` and `deny`, as in the Claude and Codex rules: the heading, the operation line, a focused-choice line and the allow-once and reject-once choice lines, each bound to the same live region. The operation matcher captures the displayed single-line command without its prefix decoration. Model-authored descriptive text the form shows is required where present but excluded from the request identity.
- **Choices.** `allow-once` and `deny-once` with scope `single_request`, labels captured from their choice lines. An "always" or session-wide choice has no answer plan and no choice entry. `native_request_id` is null.
- **Composer.** The idle composer rule anchors on the prompt prefix with cursor `at_end` and `verified_noninteraction: true`; a draft composer or a working screen does not match it.
- **Sensitivity.** `sensitive: false`; metadata and redacted evidence policies apply unchanged.

The captures fix the screen (`alternate` or `primary`) that `mini` uses. When `mini` renders inline on the primary screen, an answered form may remain visible as transcript text; the live-region, cursor and closing-line requirements must reject it, and a test replays a native post-answer screen to prove it.

### Answer stages

Each answer is exactly one stage measured on the native form: its bytes, the rule and choice it requires before it is written, and the screens after it. The stage's bytes are a key the form offers that selects its choice regardless of which choice is focused, such as a choice digit or a shortcut letter, as the Claude and Codex answers do. CAFleet never sends a lone Escape to a held form, so a reject choice that shows only `esc` uses another measured key, as the Codex deny answer does.

The deny answer ends at the stage after which OpenCode reports the request rejected. A feedback field or other input OpenCode opens after that point is manual native input; CAFleet sends no bytes to it.

Step 3 stops at a blocker when the measurement leaves either answer without a declarable stage. Each of these outcomes requires a Programmer blocker marker at `paragraph-Implementation > Step 3`, paired with a `blocked (paragraph-Implementation > Step 3)` message to the Director, because the user-required acceptance includes both Director-auto allow and deny:

| Measured outcome | Why no stage is declarable |
|---|---|
| OpenCode rejects the request only after a feedback field is submitted | The single-request rejection needs input beyond one stage |
| The reject choice is reachable only by Escape | A lone Escape is never sent to a held form |
| The form offers no focus-independent key for allow or reject | Selecting by navigation needs multi-stage, focus-dependent plans, which are out of scope |

The Director and the user then decide whether to extend the scope (for example to navigation stages gated per stage by `focus_matcher_id`) in a revision of this design.

The Step 3 capture records the native decision evidence after allow and after deny (the lines OpenCode prints, whether the command ran, and the member's continuation). The documentation records these lines as decision evidence for the Director's continuation check, not as bundled rules.

### Native capture procedure

Captures use the existing native-capture driver that produced the Claude and Codex fixtures. The driver accepts only reviewed cases from its approved-case registry, each pinning backend, permission mode, driver mode, decision, stage bytes, probe argument, database, tmux socket and size, and it runs the existing `permission-probe` helper in the case folder, whose `allow` and `deny` arguments create the `allow.executed` and `deny.executed` markers.

Add these OpenCode cases, all with backend `opencode`, permission mode `cafleet-ask` and the 36x120 full size:

| Case ID | Driver mode | Decision | Stage bytes | Probe argument | Database | Socket | Supplies |
|---|---|---|---|---|---|---|---|
| `opencode-cells-001` | `no_answer` | — | — | `allow` | `oc01.db` | `oc01` | The form fixture and the key hints for both answers |
| `opencode-ans-allow-001` | `answer_stage` | `allow` | The measured allow byte(s) | `allow` | `oa01.db` | `oa01` | The allow pre-stage fixture, post-stage screens and allow decision evidence |
| `opencode-ans-deny-001` | `answer_stage` | `deny` | The measured deny byte(s) | `deny` | `od01.db` | `od01` | The deny pre-stage fixture, post-stage screens and deny decision evidence |
| `opencode-idle-001` | `no_answer` | — | — | — | `oi01.db` | `oi01` | The idle-composer fixture |

The driver's member prompt gains an OpenCode branch, because the generic branch names Claude's Bash tool: `Use the shell tool to run exactly <helper> <argument> as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.` The tool-free prompt serves the idle case unchanged.

1. Build the capture binary with the `cafleet-ask` policy and the OpenCode cases. Write `presets/opencode/cafleet-ask.md` to `<private dir>/agents/cafleet-ask.md` under a private `OPENCODE_CONFIG_DIR` set for the driver invocation, so the operator's installed presets and skills stay untouched, and confirm the child loads the `cafleet-ask` agent.
2. Run `opencode-cells-001` and read the form, its choice keys and its screen (`alternate` or `primary`).
3. Record the measured allow and deny bytes in the two answer-stage cases as a reviewed registry edit, applying the blocker table in § Answer stages first. Each answer is one stage, so each case carries one stage-bytes value.
4. Run `opencode-ans-allow-001`, `opencode-ans-deny-001` and `opencode-idle-001`. The answer-stage post-stage screens supply the post-answer screen for the transcript false-positive test.
5. Review masking (home paths, account, model or plan text) and confirm every rule region lies in unmasked rows. Add the fixtures to the native fixture directory and record their SHA-256 digests in the catalog; the Director commits them.

Record the CAFleet commit, OpenCode version, model, terminal geometry and masking review with the execution evidence.

### Director workflow

Unchanged from 0000194. The Director consumes the `interaction_opened` event, verifies it with `event show`, inspects with `permission inspect`, decides under its own session's mode (Director-auto in verified auto mode; the user's request-specific permission otherwise), submits through `permission respond`, then inspects the native result and the member's continuation and runs a finite `message wait` to retry held previews. Unknown answer capability, a revoked token or a non-matching form preserves manual original-surface operation.

### Acceptance

Run every case with the shipping release build, a private database and a dedicated tmux server or herdr tab, OpenCode 2.1.4 as the child executable, the case folder as cwd and `--permission-mode cafleet-ask`.

| Case | Multiplexers | Required evidence |
|---|---|---|
| Director-auto allow | tmux and herdr | `interaction_opened` with the review instruction; `accepted`/`submitted`; the marker exists; OpenCode's allow evidence; `interaction_closed` `not_visible`; the member continues |
| Director-auto deny | tmux and herdr | The same event; `accepted`/`submitted`; no marker; OpenCode's reject evidence; `interaction_closed` `not_visible`; the member continues |
| User-sourced response (`--decision-source user`) | One of tmux or herdr | Zero response bytes before the user's permission; `accepted`/`submitted`; native result and closure |
| User-executed complete bound command | Same multiplexer | The user runs the presented `permission respond` command; zero Director response bytes; native result and closure |
| Manual original-surface answer | Same multiplexer | The user answers in the native pane; closure follows; a later respond with the earlier token is refused before any byte |
| Default policy coverage | One multiplexer | A `cafleet` OpenCode 2.1.4 member records `coverage=tested` and profile `opencode-2.1.4-cafleet-ask-1`, emits no `coverage_unknown` event, and an unmatched command is still denied without a form |

Record detection-to-inbox latency, the CAFleet commit, versions, geometry and every unobserved case. A missing required case fails acceptance.

### Limits

| Limit | Behavior |
|---|---|
| A command that wraps in the form | Detection covers a single-line command at 120 columns; a wrapped form yields no `interaction_opened`, programmatic input stays held, and the request is answered manually on the native surface |
| Non-shell `ask` forms | The preset never asks for non-shell permissions; any other form is outside the profile |
| OpenCode versions other than 2.1.4 | `coverage=unknown`; `cafleet-ask` still launches, its forms are answered manually |
| Compound shell commands and MCP tools | Their permission matching remains unverified on OpenCode 2.1.4; the existing caveats apply to both presets |
| Prompt and native-exec submission | `unknown`; previews and explicit prompt/exec stay held for wrapped OpenCode members, as for the other bundled profiles |

### Documentation targets

| Target | Change |
|---|---|
| `docs/docs/spec/member-runtime.md` | Launch-policy table row for OpenCode; the version-probe pattern and `opencode v2.1.4` example; the OpenCode profile section (capability table, anchors, answer keys, decision evidence, limits) |
| `docs/docs/spec/coding-agent-backends.md` | `cafleet-ask` argv and preset; the OpenCode section's preset text, replacing "every check resolves to `allow` or `deny`" with the per-preset behavior and the permission-form workflow for `cafleet-ask`; launch-policy evidence for OpenCode 2.1.4 |
| `docs/docs/spec/cli-options.md` | `--permission-mode` choices, the missing-preset error string and its selected-preset path, setup's installed presets and its per-preset success lines, and doctor's `opencode.preset` / `opencode.preset.cafleet-ask` checks in the coding-agents section |
| `docs/docs/concepts/member-permission-events.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/quickstart.md` | OpenCode joins the supported interactive policies; quickstart lists both installed presets |
| `SPEC.md` | Config-dir/preset install rows, setup's per-preset success lines, doctor's per-preset checks, the OpenCode preset contract for both presets, the launch-policy table, the probe pattern, the missing-preset error string and the OpenCode bundled profile |
| `skills/cafleet/reference/coding-agents.md` (opencode) | `{permission_flags}` and notes: `--agent cafleet` by default, `--agent cafleet-ask` when selected; a permission form under `cafleet-ask` is a runtime decision handled through the member runtime, while under `cafleet` it remains an issue to capture and relay |
| `skills/cafleet/reference/member-runtime.md`, `skills/cafleet/roles/director.md` | OpenCode `cafleet-ask` as an interactive member policy; the shipped missing-preset error |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Document the policy, preset and probe contracts

- [ ] Update `docs/docs/spec/member-runtime.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/spec/cli-options.md`, the concepts pages and quickstart for the `cafleet-ask` policy, both installed presets, setup's per-preset success lines, doctor's per-preset checks, the selected-preset precondition with the shipped error string and the `v`-prefixed version token. <!-- completed: -->
- [ ] Update `SPEC.md` and `skills/cafleet/reference/coding-agents.md`, `skills/cafleet/reference/member-runtime.md` and `skills/cafleet/roles/director.md` for the same contracts. <!-- completed: -->

### Step 2: Implement the policy, preset, setup and probe

- [ ] Write tests: the OpenCode catalog accepts `cafleet` and `cafleet-ask` for any version, explicit/omitted argv and recorded launch intent, help and usage-error choices, setup installs both presets (including replace-on-install and failure recording) and prints the skills line then one preset line per target in install order for OpenCode and Codex, doctor emits `opencode.preset` and `opencode.preset.cafleet-ask` with `ok`, `error` and `info` outcomes, the precondition reads only the selected preset with the shipped error, the preset differs from `cafleet` only in description, shell base and body, and the probe records `2.1.4` from `opencode v2.1.4` while rejecting several tokens. <!-- completed: -->
- [ ] Add `presets/opencode/cafleet-ask.md` and embed it; extend the OpenCode policy catalog and `--agent` mapping, setup's preset installation and success lines, the mode-aware `required_preset` used by the selected-preset precondition, and doctor's per-choice preset checks. <!-- completed: -->
- [ ] Extend the version-token pattern with the optional leading `v` and remove it from the recorded version. <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: -->

### Step 3: Capture native OpenCode 2.1.4 evidence

- [ ] Add the `opencode-cells-001` and `opencode-idle-001` driver cases and the OpenCode prompt branch, run `opencode-cells-001`, and confirm the screen `mini` uses, the live-region anchors and the choice keys. <!-- completed: -->
- [ ] Apply the blocker table in § Answer stages, then add the two answer-stage cases with the measured stage bytes and run them with `opencode-idle-001`, recording the pre-stage and post-stage screens and OpenCode's decision evidence. <!-- completed: -->
- [ ] Review masking for every capture, add the fixtures to the native fixture directory with their SHA-256 digests, and keep a post-answer screen for the transcript false-positive test. <!-- completed: -->

### Step 4: Document the OpenCode profile

- [ ] Add the OpenCode 2.1.4 profile section to `docs/docs/spec/member-runtime.md` and `SPEC.md` with the captured anchors, answer keys, decision evidence and limits, and update the OpenCode limits in `docs/docs/spec/coding-agent-backends.md` and the skill references. <!-- completed: -->

### Step 5: Implement the profile

- [ ] Write fixture-backed tests: detection on the form fixture, both answer plans on their pre-stage fixtures, the idle composer as a verified non-interaction boundary, no detection on the idle composer and post-answer screens, no answer plan for an "always" choice, and catalog capability results for OpenCode 2.1.4 and for another OpenCode version. <!-- completed: -->
- [ ] Add the native fixtures and profile `opencode-2.1.4-cafleet-ask-1` to the bundled catalog and its embedded fixture table. <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: -->

### Step 6: Accept natively

- [ ] Run Director-auto allow and deny on tmux and on herdr, recording the evidence in § Acceptance. <!-- completed: -->
- [ ] Run the three user-attended cases on one multiplexer with the user. <!-- completed: -->
- [ ] Verify the default-policy coverage case, and record latency, versions, geometry and unobserved cases. <!-- completed: -->

### Step 7: Validate the shipping surfaces

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`; verify help, embedded presets and the bundled catalog match the documentation, and record the execution summary in this document. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-28 | Initial draft |
