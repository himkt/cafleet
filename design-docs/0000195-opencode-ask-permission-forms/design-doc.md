# OpenCode Ask Permission Forms

**Status**: Approved
**Progress**: 18/28 tasks complete
**Last Updated**: 2026-10-28

## Overview

Add the explicit OpenCode launch policy `cafleet-ask`, backed by a second embedded agent preset whose unmatched shell commands use `ask`, so an OpenCode member shows a native permission form instead of a silent denial. Add a bundled OpenCode 2.1.4 profile, built from CAFleet's own native `mini` captures and measured answer stages, so the member runtime detects that form, notifies the Director and answers it through `permission respond`, as it already does for Claude `default` and Codex `on-request`. The omitted OpenCode policy stays `cafleet`.

## Success Criteria

- [ ] `cafleet member create --coding-agent opencode --permission-mode cafleet-ask` launches `opencode mini --standalone --agent cafleet-ask --prompt <prompt>` and records `requested_mode=cafleet-ask`, `resolved_mode=cafleet-ask`, `mode_source=explicit` and `permission_argv_json=["--agent","cafleet-ask"]`; omitted selection still launches `--agent cafleet` with `mode_source=default`.
- [ ] `cafleet setup --coding-agent opencode` installs both `agents/cafleet.md` and `agents/cafleet-ask.md` under the resolved preset base and prints one preset line per target, `cafleet doctor` reports `opencode.preset` and `opencode.preset.cafleet-ask`, and `member create` requires only the selected policy's preset file.
- [ ] A member reads the configuration CAFleet validated: `member create` forwards the selected backend's `CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `OPENCODE_CONFIG_DIR` into the member environment when the creator sets it, and the OpenCode preset base defaults to `~/.config/opencode`, where OpenCode 2.1.4 loads both presets.
- [ ] The `cafleet-ask` preset differs from the `cafleet` preset only in its description, its shell base rule `"*": "ask"` and its body text; every allowlist entry, every Director-only shell deny and every non-shell permission is identical.
- [ ] The native version probe records `2.1.4` for OpenCode 2.1.4, whose `--version` prints `opencode v2.1.4`, and an OpenCode 2.1.4 member under either policy records profile `opencode-2.1.4-cafleet-ask-1` and `coverage=tested`.
- [ ] The bundled profile declares tested detection, allow-once and reject-once answers and the empty-composer observation, each resting on native OpenCode 2.1.4 `mini` captures at 36x120; every answer stage is gated on the observed focus or continuation screen and none contains Escape, prompt and native-exec submission stay `unknown`, and no answer plan exists for an "always" choice.
- [ ] On both tmux and herdr, a `cafleet-ask` member's shell permission form produces one `interaction_opened` event with the review instruction, and Director-auto allow and deny through `permission respond` reach the original request, close the episode `not_visible` and show the native result. After allow the member continues on its own; after deny OpenCode shows `Step interrupted` and the member continues on the Director's ordinary continuation message.
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
| Forwarding the selected backend's config-location variable into the member environment, and the `~/.config/opencode` OpenCode preset base (§ Member configuration environment and the OpenCode preset base) | Migrating or deleting files at the former `~/.opencode` base |
| Bundled profile `opencode-2.1.4-cafleet-ask-1`: shell permission detection, allow-once and reject-once answers, empty-composer observation | `ask` for non-shell permissions (`edit`, `external_directory`, `webfetch`, `websearch` and the other preset keys) |
| OpenCode 2.1.4 `cafleet` members moving from `coverage=unknown` (with its `coverage_unknown` notification) to `coverage=tested` with the empty-composer boundary | Prompt and native-exec composer submission plans |
| Gated multi-stage answer plans: per-stage focus and continuation-screen gates, the expected transition between stages and the Escape-free stage invariant (§ Focus-gated answer stages) | Answer stages without an observed focus gate in a navigation plan; any stage containing Escape |
| Forwarding outer-terminal input bursts to the child within one tick (§ Outer-terminal input forwarding) | Escape-sequence framing or buffering of manual input |
| Native acceptance on tmux and herdr | Wrapped or narrow-pane permission forms; OpenCode versions other than 2.1.4 |

The runner, event, inspection, response, receipt and Director-workflow contracts of 0000194 apply unchanged, except for the answer-plan extension in § Focus-gated answer stages, which leaves the Claude and Codex plans and their behavior unchanged. The `director_action` instruction, including its multi-row sentence rule, is backend-neutral; a single-line OpenCode operation always has `operation_rows` 1.

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

### Member configuration environment and the OpenCode preset base

`member create` validates the selected backend's assets in the creator's environment, but the native child runs in the multiplexer pane's environment plus the spawn environment. Today the spawn environment holds only `CAFLEET_DATABASE_URL`. A config-location variable set for the creator therefore never reaches the member, and the backend reads a configuration CAFleet never checked. Two changes make the member read the same configuration CAFleet validated and installed into.

**Forwarded config-location variable.** When the creator's environment sets the selected backend's config-location variable, `member create` adds it, with the creator's exact value, to the spawn environment after `CAFLEET_DATABASE_URL`:

| Backend | Variable | Forwarded when |
|---|---|---|
| `claude` | `CLAUDE_CONFIG_DIR` | Set in the creator's environment |
| `codex` | `CODEX_HOME` | Set in the creator's environment |
| `opencode` | `OPENCODE_CONFIG_DIR` | Set in the creator's environment |

An unset variable is not forwarded, so the member uses the native default, which equals CAFleet's default base below. The value is the one config-dir resolution already validated as absolute, so an invalid value still fails `member create` before registration with `<VAR> must be an absolute path (got '<value>')`. Only the selected backend's variable is forwarded. The runner passes the spawn environment to the native child as overrides, and the launch descriptor and launch evidence record it. Config locations are paths, not secrets. The variable is never forwarded with a default value, because Claude Code locates its own state file differently when `CLAUDE_CONFIG_DIR` is set.

**OpenCode default preset base.** OpenCode 2.1.4 loads agent definitions only from `~/.config/opencode` and from `OPENCODE_CONFIG_DIR`; `opencode debug config` lists no `~/.opencode`. The OpenCode preset base therefore becomes:

| Variable state | Preset base | Preset targets |
|---|---|---|
| `OPENCODE_CONFIG_DIR` set | `$OPENCODE_CONFIG_DIR` | `<base>/agents/cafleet.md`, `<base>/agents/cafleet-ask.md` |
| `OPENCODE_CONFIG_DIR` unset | `~/.config/opencode` | `~/.config/opencode/agents/cafleet.md`, `~/.config/opencode/agents/cafleet-ask.md` |

The skills base stays the fixed `~/.config/opencode/skills`, so with the variable unset skills and presets share one base. The recorded identity path for `opencode` stays the resolved preset base, now `~/.config/opencode` by default. An installation recorded at `~/.opencode` no longer matches:

- Doctor reports `opencode.configuration` `ok` at `~/.config/opencode`, `opencode.installation` `info` with `no installation recorded at <home>/.config/opencode; run 'cafleet setup --coding-agent opencode'`, and both preset checks against the files at `~/.config/opencode/agents`.
- The stale-assets guard ignores the row, as it ignores every row at a non-resolved path.
- An OpenCode `member create` fails its selected-preset precondition with `cannot read opencode agent preset at ~/.config/opencode/agents/<mode>.md: <cause>; run 'cafleet setup --coding-agent opencode' first`.

Rerunning setup installs both presets at the new base and leaves files at the former base in place.

### Outer-terminal input forwarding

The runner forwards outer-terminal input to the native child as manual input. One read of the runner's raw-mode stdin returns at most the line discipline's 4096-byte buffer, so a longer terminal reply arrives in several reads. One example is herdr's answer to OpenCode's 256-entry OSC 4 palette query. The shipped tick reads once, writes once and the loop then sleeps up to 2 ms. The child therefore receives the first 4096 bytes alone, a tick before the rest. When that gap exceeds OpenCode's incomplete-sequence timeout, OpenCode parses the remainder as keys and fails with `Invalid key name: key name cannot be empty`. Its screen then diverges from a normal form, so no `interaction_opened` arrives under herdr. OpenCode run directly under herdr meets the same 4096-byte boundary without the gap.

| Concern | Contract |
|---|---|
| Read | Each tick repeats the non-blocking outer read until it would block, reaches EOF or fills the 65536-byte input queue; each read keeps the existing per-read capacity |
| Write | The tick then writes the whole queue to the PTY master in one write; bytes the master does not accept stay queued, in order, for the next tick |
| EOF | Bytes read before EOF are forwarded; EOF is then reported as today |
| Pacing | When a tick read outer bytes or leaves input queued, the service loop starts the next tick without its idle sleep; otherwise the idle sleep is unchanged |
| Framing | None: bytes pass unchanged and in order, with no escape-sequence parsing, holding or timeout |
| Manual accounting | Unchanged: every read advances the manual-input generation by its byte count |

A reply longer than the 65536-byte queue still spans ticks, but with no idle sleep between them.

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
| Detection | `tested` | Rule `opencode-shell-permission`: the native `mini` shell permission form on the primary screen, from its `△ Permission required` heading to its `⇆ select  enter confirm  esc reject` control line within 24 rows, with the single-line `$ ` operation, the focused choice and the `Allow once` and `Reject` choices; the cursor is hidden and its position is not part of the match |
| Allow answer | `tested` | Answer rule `opencode-shell-allow-once`: one focus-gated stage, Enter (`0d`) while `Allow once` is focused, as `allow-once` |
| Deny answer | `tested` | Answer rule `opencode-shell-deny-once`: three gated stages, `h` (`68`) while `Allow once` is focused, Enter (`0d`) while `Reject` is focused, then Enter (`0d`) while the continuation rule `opencode-reject-reason` shows the rejection-reason dialog with an empty reason, as `deny-once` |
| Empty-composer observation | `tested` | Rule `opencode-idle-composer`: the idle `mini` composer with the terminal cursor at the end of its prompt prefix; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

Fixtures, each a native capture of OpenCode 2.1.4 `mini` under the CAFleet runner at 36 rows by 120 columns, with privacy masking per the existing fixture contract:

| Fixture ID | Screen | Cited by |
|---|---|---|
| `opencode-2.1.4-ask-permission-1` | The permission form before any answer byte | Detection |
| `opencode-2.1.4-allow-pre-stage-1` | The form with `Allow once` focused, immediately before the allow stage | Allow answer, stage 1 |
| `opencode-2.1.4-deny-pre-stage-1` | The form with `Allow once` focused, immediately before the deny `h` stage | Deny answer, stage 1 |
| `opencode-2.1.4-deny-pre-stage-2` | The form with `Reject` focused after the measured `h`, immediately before the deny Enter stage | Deny answer, stage 2 |
| `opencode-2.1.4-deny-pre-stage-3` | The rejection-reason dialog with an empty reason after the measured `h` and Enter, immediately before the final Enter | Deny answer, stage 3; continuation rule `opencode-reject-reason` |
| `opencode-2.1.4-focus-always-1` | The form with `Always allow` focused after a measured `l` | Unmet-gate tests (no plan writes a byte on it) |
| `opencode-2.1.4-idle-composer-1` | The idle composer with no interaction | Empty-composer observation |

Each stage cites one pre-stage fixture that proves its `required_rule_id` (`opencode-shell-permission`), its `required_choice_id` (the answer's `allow-once` or `deny-once`) and its gate: the `required_focus_choice_id` on the focus the native form actually showed, or the `required_screen_rule_id` on the continuation screen it showed. Both answer entries use `request_rule_id` `opencode-shell-permission` and `focus_matcher_id` `focus`, the id of the rule's focused-choice matcher.

Rule requirements, pinned to exact patterns from the captures in Step 3:

- **Live region.** Start and end patterns anchor on the form's own heading line `△ Permission required` and closing control line `⇆ select  enter confirm  esc reject`, as the `opencode-cells-001` capture shows, not on absolute rows. The choices share the control line's row. `cursor` is `any`: `mini` hides the cursor while the form shows, and its recorded position varies. The fixtures show it at the pending-wrap position on the row below the form ([22,120]) and inside the choice row ([21,38], [21,29]). Text outside that region, including transcript lines and scrollback, cannot open a request.
- **Required matchers.** Ids `heading`, `operation`, `focus`, `allow` and `deny`, as in the Claude and Codex rules: the heading, the operation line, a focused-choice line and the allow-once and reject-once choice lines, each bound to the same live region. The operation matcher captures the displayed single-line command without its prefix decoration. Model-authored descriptive text the form shows is required where present but excluded from the request identity.
- **Focus.** The `focus` matcher declares `focus_style: "background"` and resolves the focused choice to exactly one choice id (§ Focus-gated answer stages), because the form marks focus by cell background only. A focus on `Always allow`, or a focus the matcher cannot resolve, resolves to no choice, so no gate holds.
- **Choices.** `allow-once` and `deny-once` with scope `single_request`, labels captured from their choice lines; the rule matches both the three-option (`Allow once` / `Always allow` / `Reject`) and the two-option layout. An "always" or session-wide choice has no answer plan and no choice entry. `native_request_id` is null.
- **Rejection-reason continuation.** Rule `opencode-reject-reason`, kind `permission_continuation`, with `continues_rule_id` `opencode-shell-permission` and `continues_choice_id` `deny-once`. It matches from the `△ Reject permission` heading to the `enter confirm  esc cancel` control line, requires the `Tell OpenCode what to do differently` prompt, and identifies an empty reason textarea by its placeholder. The textarea span on the control-line row must equal the placeholder `Tell OpenCode what to do differently` exactly; typed text replaces the placeholder. `cursor` is `any`, as for the form. It declares no operation, choices or answers; its exact patterns come from the `opencode-2.1.4-deny-pre-stage-3` capture.
- **Composer.** The idle composer rule anchors on the prompt prefix with cursor `at_end` and `verified_noninteraction: true`; a draft composer or a working screen does not match it.
- **Sensitivity.** `sensitive: false`; metadata and redacted evidence policies apply unchanged.

`mini` renders the form on the primary screen, as the `opencode-cells-001` capture shows. An answered form or dialog leaves no transcript text: it disappears, and the transcript shows the member's report after allow or `Step interrupted` after deny, followed by the idle composer. The false-positive tests therefore use the idle-composer fixture, which is the screen after an answer, plus synthetic screens that carry form text outside a live region. No native post-answer grid is required.

### Answer stages

The OpenCode 2.1.4 permission dialog binds `left`/`h` and `right`/`l` (cycling focus through the options), `return` (select the focused option) and `escape` (reject). Focus starts on the first option, `Allow once`. Selecting `Reject` in `mini` opens a rejection-reason dialog (`△ Reject permission`, `Tell OpenCode what to do differently`, an empty textarea, `enter confirm  esc cancel`) while the request stays pending. In that dialog, Enter replies reject with the textarea text (no reason when it is empty), and Escape returns to the form. The form therefore offers no focus-independent key, and `Reject` is reachable without Escape only by navigation, so both answers are gated plans (§ Focus-gated answer stages):

| Answer | Stage | Bytes | Gate | Native effect |
|---|---|---|---|---|
| Allow | 1 | `0d` (Enter) | Focus `allow-once` | Selects `Allow once`; OpenCode runs the command once |
| Deny | 1 | `68` (`h`) | Focus `allow-once` | Cycles focus from the first option to the last, `Reject`, in both the two- and three-option layouts |
| Deny | 2 | `0d` (Enter) | Focus `deny-once` | Selects `Reject`; OpenCode opens the rejection-reason dialog |
| Deny | 3 | `0d` (Enter) | Screen `opencode-reject-reason` with an empty reason | Submits the empty reason; OpenCode rejects the request without running it |

The deny answer always submits an empty reason, so CAFleet types no text into the dialog. No plan contains Escape.

The native decision evidence is:

| Answer | Evidence on the native surface | Continuation |
|---|---|---|
| Allow | The hint area briefly shows `Waiting for permission event…`, the form disappears without transcript text, the command runs, and the member prints its own report, a `<agent> · <model> · <duration>` line and the idle composer | The member continues on its own |
| Deny | After the final Enter the hint area briefly shows `Waiting for permission event…`, the dialog disappears, and the transcript shows `Step interrupted`, a `<agent> · <model> · <duration>` line and the idle composer; nothing runs | OpenCode does not continue after a rejection; the Director continues the member with an ordinary message, as after Claude's `Interrupted · What should Claude do instead?` |

Step 3 stops at a blocker when a measurement contradicts this table. Each of these outcomes requires a Programmer blocker marker at `paragraph-Implementation > Step 3`, paired with a `blocked (paragraph-Implementation > Step 3)` message to the Director, because the user-required acceptance includes both Director-auto allow and deny:

| Measured outcome | Why the plan is not declarable |
|---|---|
| The focused option cannot be resolved from the captured screen by the focus mechanism in § Focus-gated answer stages | No stage gate can be observed |
| `h` from `Allow once` focuses an option other than `Reject` | The deny stage-2 gate never holds |
| Enter on `Reject` shows anything other than the rejection-reason dialog with an empty reason | The deny stage-3 gate never holds |
| Enter in the empty rejection-reason dialog does not reject the request, or rejects it with text | The plan does not end in a single-request rejection |
| Enter on `Allow once` does not run the command exactly once | The allow stage does not select a single-request choice |

The Step 3 capture records the native decision evidence after allow and after deny (the lines OpenCode prints, whether the command ran, and the member's continuation). The documentation records these lines as decision evidence for the Director's continuation check, not as bundled rules.

### Focus-gated answer stages

This extends the 0000194 answer-plan engine. Today every stage carries `bytes_hex`, `required_rule_id` and `required_choice_id`; the catalog requires both ids to equal the answer's request rule and choice, and the runner re-checks that equality statically before each write. The focused choice on the screen is never compared with a stage. The focus matcher's full capture is part of the request identity (`ObservedRequest`, `AnswerMatch` and the write-time binding all compare it), so a focus change starts a new request generation and the bound request token stops validating. Between stages the runner already requires a fresh quiescent observation drain and revalidates the binding, the manual-input generation and the durable reservation before each write. A failure after any written byte records the receipt `uncertain` (`ready` when no byte was written), blocks the pending input and returns the cause; nothing is retried.

**Stage schema.** A stage gains at most one optional gate, checked on the fresh observation immediately before the runner writes that stage:

| Gate | Holds when |
|---|---|
| `required_focus_choice_id` | The plan's focus matcher resolves this choice id as focused on the request form |
| `required_screen_rule_id` | This `permission_continuation` rule matches as the continuation of the bound request (below) |

A stage without a gate is focus-independent; the Claude and Codex stages stay ungated and keep their behavior. `required_choice_id` keeps its meaning, the answer's own choice.

**Catalog validation.** Profile loading fails unless:

- every focus gate names a choice of the request rule;
- every screen gate names a `permission_continuation` rule whose `continues_rule_id` is the answer's request rule, and the stage immediately before it is gated on that rule's `continues_choice_id`;
- a plan with any gated stage gates every stage, and its final stage is gated either on the answer's own `choice_id` or on a continuation rule whose `continues_choice_id` is the answer's own `choice_id`;
- no stage's bytes contain `1b` (Escape), for every backend;
- a plan with a focus gate has a focus matcher that declares `focus_style`, and every choice label it compares is captured by a line-scoped matcher;
- a `permission_continuation` rule declares no operation, choices or answers, and cites a native fixture.

**Focused-choice resolution.** The OpenCode form marks the focused option by cell styling only; the `opencode-cells-001` capture shows no textual marker. The OpenCode focus matcher therefore declares `focus_style: "background"`, which the detector resolves without depending on the theme:

1. Locate each choice label's cells through its line's cell boundaries.
2. Take the choice row background from the choice line's cell at the live region's left edge.
3. Resolve the focused choice as the one choice whose label cells all carry a single background that differs from the row background, provided no other choice label carries a differing background.

A focus on an option without a choice entry (`Always allow`), on several options or on none resolves to no choice. A gate that does not hold is never written. The colours the capture shows (the focused `Allow once` on rgb(250,178,131), the other options on the row background rgb(30,30,30)) are capture evidence, not rule constants.

**Focus in the request identity.** The focus matcher's text does not change when a styling-only focus moves. For a `focus_style` matcher, the focus value compared in the observed request, the answer match and the write-time binding is therefore the resolved focused choice id (or none) together with the matcher's capture. A styling-only focus change thus starts a new request generation, as a textual focus change does today.

**Snapshot, capture and fixture support.** Per-cell background reaches the detector at runtime but not from the bundled fixtures:

| Surface | Per-cell background today | Change |
|---|---|---|
| Runner screen snapshot | Retained: the snapshot holds the full `vt100::Screen`, whose cells expose their background colour | None; the detector reads it |
| Native capture artifact | Retained: each unmasked projected cell records `fg` and `bg` (`default`, `indexed` or `rgb`) | None |
| Bundled native fixture | Not retained: schema 1 carries only `lines`, the loader rejects unknown keys, and test replay paints text only | Add the optional `cell_backgrounds` key |

`cell_backgrounds` lists runs `{row, start_col, end_col, bg}`, one per maximal run of unmasked cells whose background is not `default`, with `bg` in the capture artifact's colour shape. The capture-to-fixture conversion derives the runs from the capture grid. The loader requires every run to lie inside the geometry, outside masked rows, without overlap, and with a valid colour. Replay sets each run's background before writing its text. An absent key means every cell has the default background, so the existing Claude and Codex fixtures and their digests are unchanged.

**Request identity across stages.** A request token is minted for the observed request, including its focus, and the first stage requires that same observation. After a stage's complete write, the runner accepts exactly one change before the next stage: the transition the next stage's gate names. For a screen gate this is the continuation binding below. For a focus gate it is the expected focus transition: a fresh quiescent observation qualifies when its episode, rule, semantic fingerprint, choices, native correlation and manual-input generation are unchanged, and its resolved focus equals the next stage's gate. The runner then rebinds the in-flight selection to that observation, with its new request generation and focus, and writes the next stage. It mints no token, because the reservation already consumed the request token.

**Continuation screens.** A `permission_continuation` rule describes a screen that the native agent shows for a still-pending request after one of its choices is selected, such as the rejection-reason dialog. It shows no operation, so it cannot identify the request by itself. It is bound to the request by succession and episode continuity instead:

| Concern | Contract |
|---|---|
| Episode | While a permission episode of its `continues_rule_id` is active, a continuation observation keeps that episode open and `Visible` with input held for the interaction. It opens no episode, emits no event, starts no obscured timer and leaves the episode's stored request evidence unchanged. Without such an active episode it is an unknown screen. |
| Request binding | Inside a response, the continuation qualifies for the next stage only as the first fresh quiescent observation after the complete write of the stage gated on its `continues_choice_id`. The episode, manual-input generation and runner ownership must be unchanged, with no observation gap or degradation in between. The runner then rebinds the in-flight selection to the continuation's rule id and match fingerprint within the same episode, and writes the gated stage. |
| Inspection | `permission inspect` on a continuation offers no request token or answer capability, so a continuation reached by human input stays manual. |
| Empty reason | The OpenCode rule requires the reason textarea to be empty. A dialog with text, typed by anyone, does not match, so the gate does not hold. |

**Waiting and failure.** The next stage waits for its gate within the responding invocation's existing deadline. An observation that still shows the previous stage's screen and focus keeps waiting. These end the response with the existing failure reporting (`uncertain` once a byte was written), and the native surface stays for manual input:

- a focus on any other option;
- a screen other than the expected continuation after a stage that opens one;
- the episode closing before the final stage;
- any other identity change, human input or a degraded observation;
- the deadline expiring.

After a failed deny stage 2, the form remains open with `Reject` or another option focused. After a failed deny stage 3, the rejection-reason dialog or whatever OpenCode shows stays for the user. In both cases CAFleet sends no further byte.

### Native capture procedure

Captures use the existing native-capture driver that produced the Claude and Codex fixtures. The driver accepts only reviewed cases from its approved-case registry, each pinning backend, permission mode, driver mode, decision, stage bytes, probe argument, database, tmux socket and size, and it runs the existing `permission-probe` helper in the case folder, whose `allow` and `deny` arguments create the `allow.executed` and `deny.executed` markers.

The capture harness records one armed capture per case, so a pre-stage fixture for a later stage needs its own case whose earlier stage is sent before arming. The registry binding therefore splits the stage bytes into `prefix_hex` (the measured stages the Director sends before arming) and `stage_hex` (the stages sent after arming, in order). The Director sends each stage as one `tmux send-keys -H` command only after a fresh pane capture shows that stage's gate.

Add these OpenCode cases, all with backend `opencode`, permission mode `cafleet-ask` and the 36x120 full size:

| Case ID | Driver mode | Decision | Prefix bytes | Stage bytes | Probe argument | Database | Socket | Supplies |
|---|---|---|---|---|---|---|---|---|
| `opencode-cells-001` | `no_answer` | — | — | — | `allow` | `oc01.db` | `oc01` | The form fixture, the key hints and the focus indicator |
| `opencode-ans-allow-001` | `answer_stage` | `allow` | — | `0d` | `allow` | `oa01.db` | `oa01` | The allow pre-stage fixture, post-stage screens and allow decision evidence |
| `opencode-ans-deny-001` | `answer_stage` | `deny` | — | `68`, then `0d` | `deny` | `od01.db` | `od01` | The deny stage-1 pre-stage fixture, the screen after `h`, and the rejection-reason dialog after Enter (captured; stopped before any third byte) |
| `opencode-ans-deny-002` | `answer_stage` | `deny` | `68` | `0d`, then `0d` | `deny` | `od02.db` | `od02` | The deny stage-2 pre-stage fixture, the dialog after the second stage, and deny decision evidence |
| `opencode-ans-deny-003` | `answer_stage` | `deny` | `68`, then `0d` | `0d` | `deny` | `od03.db` | `od03` | The deny stage-3 pre-stage fixture (the empty rejection-reason dialog) and deny decision evidence |
| `opencode-focus-always-001` | `answer_stage` | `deny` | `6c` | — | `deny` | `of01.db` | `of01` | Stopped before its capture published; no fixture |
| `opencode-focus-always-002` | `answer_stage` | `deny` | `6c` | — | `deny` | `of02.db` | `of02` | The `Always allow`-focused fixture; no confirming byte is sent |
| `opencode-idle-001` | `no_answer` | — | — | — | — | `oi01.db` | `oi01` | The idle-composer fixture |

`opencode-focus-always-002` sends only the measured `l` and never Enter, so both markers stay absent and the form is closed by member cleanup.

A capture's two samples come from the runner's quiet-timer polling, so a static screen needs no output after arming. The runner does cancel an unpublished capture when its case stops, and a stage byte sent before the second sample enters the pre-stage fixture. The driver's `arm` therefore returns only after the capture manifest is published, and `stop` refuses an armed case whose manifest is not yet published.

The driver's member prompt gains an OpenCode branch, because the generic branch names Claude's Bash tool: `Use the shell tool to run exactly <helper> <argument> as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.` The tool-free prompt serves the idle case unchanged.

1. Build the capture binary with the `cafleet-ask` policy and the OpenCode cases. Write `presets/opencode/cafleet-ask.md` to `<private dir>/agents/cafleet-ask.md` under a private `OPENCODE_CONFIG_DIR` set for the driver invocation, so the operator's installed presets and skills stay untouched, and confirm the child loads the `cafleet-ask` agent.
2. Run `opencode-cells-001` and read the form, its choice keys, its screen and how it marks the focused option (text or cell styling, from the `capture-pane -e` output), which selects the focus mechanism in § Focus-gated answer stages.
3. Apply the blocker table in § Answer stages, then record the stage bytes above in the answer-stage cases as a reviewed registry edit.
4. Run `opencode-ans-allow-001`, `opencode-ans-deny-001`, `opencode-ans-deny-002`, `opencode-ans-deny-003`, `opencode-focus-always-002` and `opencode-idle-001`. Before every byte, capture the pane and confirm that stage's gate: the focused option, or the rejection-reason dialog with an empty reason. The post-stage screens supply the decision evidence.
5. Review masking (home paths, account, model or plan text) and confirm every rule region lies in unmasked rows. The reviewed mask is row 35. The idle-composer capture shows the OpenCode default public model name (`Big Pickle`) and a context-usage counter in rows 9 and 13; neither is an account, plan or path, so the masking review accepts them unmasked. Add the fixtures to the native fixture directory and record their SHA-256 digests in the catalog; the Director commits them.

Record the CAFleet commit, OpenCode version, model, terminal geometry and masking review with the execution evidence.

### Director workflow

Unchanged from 0000194. The Director consumes the `interaction_opened` event, verifies it with `event show`, inspects with `permission inspect`, decides under its own session's mode (Director-auto in verified auto mode; the user's request-specific permission otherwise), submits through `permission respond`, then inspects the native result and the member's continuation and runs a finite `message wait` to retry held previews. Unknown answer capability, a revoked token or a non-matching form preserves manual original-surface operation.

### Acceptance

Run every case with the shipping release build, a private database and a dedicated tmux server or herdr tab, OpenCode 2.1.4 as the child executable, the case folder as cwd and `--permission-mode cafleet-ask`.

| Case | Multiplexers | Required evidence |
|---|---|---|
| Director-auto allow | tmux and herdr | `interaction_opened` with the review instruction; `accepted`/`submitted`; the marker exists; OpenCode's allow evidence; `interaction_closed` `not_visible`; the member continues |
| Director-auto deny | tmux and herdr | The same event; `accepted`/`submitted`; no marker; OpenCode's `Step interrupted` and idle composer; `interaction_closed` `not_visible`; the member continues on the Director's ordinary continuation message |
| User-sourced response (`--decision-source user`) | One of tmux or herdr | Zero response bytes before the user's permission; `accepted`/`submitted`; native result and closure |
| User-executed complete bound command | Same multiplexer | The user runs the presented `permission respond` command; zero Director response bytes; native result and closure |
| Manual original-surface answer | Same multiplexer | The user answers in the native pane; closure follows; a later respond with the earlier token is refused before any byte |
| Default policy coverage | One multiplexer | A `cafleet` OpenCode 2.1.4 member records `coverage=tested` and profile `opencode-2.1.4-cafleet-ask-1`, emits no `coverage_unknown` event, and an unmatched command is still denied without a form |

Record detection-to-inbox latency, the CAFleet commit, versions, geometry and every unobserved case. A missing required case fails acceptance.

### Limits

| Limit | Behavior |
|---|---|
| A command that wraps in the form | Detection covers a single-line command at 120 columns; a wrapped form yields no `interaction_opened`, programmatic input stays held, and the request is answered manually on the native surface |
| Focus moved by input other than the plan | A focus change before the response invalidates its request token; one between stages fails the response without further bytes (`uncertain`), and the form is answered manually on the native surface |
| A rejection with a reason | A CAFleet deny always submits an empty reason; a reason is typed and submitted manually on the native surface |
| A rejection-reason dialog reached by human input | The dialog keeps the episode open but offers no request token, so it is completed manually; a dialog with text never satisfies the deny stage-3 gate |
| A typed reason identical to the placeholder | The fixture-backed matcher reads the placeholder text and cannot tell a typed copy of it from an empty textarea; such typing is human input, which fails the stage-3 gate inside a response |
| Non-shell `ask` forms | The preset never asks for non-shell permissions; any other form is outside the profile |
| OpenCode versions other than 2.1.4 | `coverage=unknown`; `cafleet-ask` still launches, its forms are answered manually |
| Compound shell commands and MCP tools | Their permission matching remains unverified on OpenCode 2.1.4; the existing caveats apply to both presets |
| Prompt and native-exec submission | `unknown`; previews and explicit prompt/exec stay held for wrapped OpenCode members, as for the other bundled profiles |

### Documentation targets

| Target | Change |
|---|---|
| `docs/docs/spec/member-runtime.md` | Launch-policy table row for OpenCode; the version-probe pattern and `opencode v2.1.4` example; the focus-gated answer-stage contract (focus and screen gates, catalog validation, `focus_style` resolution, the focus value in the request identity, expected focus transition, continuation screens, waiting and failure) in the answer-plan, episode and request-bound response sections; the native fixture's optional `cell_backgrounds`; the OpenCode profile section (capability table, anchors, focus indicator, stages, decision evidence, limits) |
| `docs/docs/spec/coding-agent-backends.md` | `cafleet-ask` argv and preset; the OpenCode section's preset text, replacing "every check resolves to `allow` or `deny`" with the per-preset behavior and the permission-form workflow for `cafleet-ask`; launch-policy evidence for OpenCode 2.1.4 |
| `docs/docs/spec/cli-options.md` | `--permission-mode` choices, the missing-preset error string and its selected-preset path, setup's installed presets and its per-preset success lines, and doctor's `opencode.preset` / `opencode.preset.cafleet-ask` checks in the coding-agents section |
| `docs/docs/concepts/member-permission-events.md`, `docs/docs/concepts/coding-agents.md`, `docs/docs/quickstart.md` | OpenCode joins the supported interactive policies; quickstart lists both installed presets |
| `SPEC.md` | Config-dir/preset install rows, setup's per-preset success lines, doctor's per-preset checks, the OpenCode preset contract for both presets, the launch-policy table, the probe pattern, the missing-preset error string, the focus-gated answer-stage contract and the OpenCode bundled profile |
| `skills/cafleet/reference/coding-agents.md` (opencode) | `{permission_flags}` and notes: `--agent cafleet` by default, `--agent cafleet-ask` when selected; a permission form under `cafleet-ask` is a runtime decision handled through the member runtime, while under `cafleet` it remains an issue to capture and relay |
| `skills/cafleet/reference/member-runtime.md`, `skills/cafleet/roles/director.md` | OpenCode `cafleet-ask` as an interactive member policy; the shipped missing-preset error |
| `docs/docs/spec/cli-options.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/quickstart.md`, `SPEC.md`, `skills/cafleet/reference/coding-agents.md` (opencode) | The forwarded config-location variable in `member create`, its launch descriptor and the native child environment; the `~/.config/opencode` OpenCode preset base in the config-dir, assets, recorded-identity and doctor examples, the preset tables and quickstart, replacing every `~/.opencode` default |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Document the policy, preset and probe contracts

- [x] Update `docs/docs/spec/member-runtime.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/spec/cli-options.md`, the concepts pages and quickstart for the `cafleet-ask` policy, both installed presets, setup's per-preset success lines, doctor's per-preset checks, the selected-preset precondition with the shipped error string and the `v`-prefixed version token. <!-- completed: 2026-10-28T11:38 -->
- [x] Update `SPEC.md` and `skills/cafleet/reference/coding-agents.md`, `skills/cafleet/reference/member-runtime.md` and `skills/cafleet/roles/director.md` for the same contracts. <!-- completed: 2026-10-28T11:41 -->

### Step 2: Implement the policy, preset, setup and probe

- [x] Write tests: the OpenCode catalog accepts `cafleet` and `cafleet-ask` for any version, explicit/omitted argv and recorded launch intent, help and usage-error choices, setup installs both presets (including replace-on-install and failure recording) and prints the skills line then one preset line per target in install order for OpenCode and Codex, doctor emits `opencode.preset` and `opencode.preset.cafleet-ask` with `ok`, `error` and `info` outcomes, the precondition reads only the selected preset with the shipped error, the preset differs from `cafleet` only in description, shell base and body, and the probe records `2.1.4` from `opencode v2.1.4` while rejecting several tokens. <!-- completed: 2026-10-28T11:40 -->
- [x] Add `presets/opencode/cafleet-ask.md` and embed it; extend the OpenCode policy catalog and `--agent` mapping, setup's preset installation and success lines, the mode-aware `required_preset` used by the selected-preset precondition, and doctor's per-choice preset checks. <!-- completed: 2026-10-28T11:47 -->
- [x] Extend the version-token pattern with the optional leading `v` and remove it from the recorded version. <!-- completed: 2026-10-28T11:47 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: 2026-10-28T11:47 -->

### Step 3: Capture native OpenCode 2.1.4 evidence

- [x] Add the `opencode-cells-001` and `opencode-idle-001` driver cases and the OpenCode prompt branch, run `opencode-cells-001`, and confirm the screen `mini` uses, the live-region anchors, the choice keys and the focus indicator. <!-- completed: 2026-10-28T11:56 -->
- [x] Apply the blocker table in § Answer stages, then add `prefix_hex` to the case binding and the answer-stage cases of § Native capture procedure with their stage bytes, and run them with `opencode-idle-001`, recording the pre-stage and post-stage screens and OpenCode's decision evidence. <!-- completed: 2026-10-28T12:33 -->
- [x] Review masking for every capture, convert each capture into a fixture with its `cell_backgrounds` runs, add the fixtures to the native fixture directory with their SHA-256 digests, and record the masking review. <!-- completed: 2026-10-28T12:47 -->

### Step 4: Document the OpenCode profile

- [x] Document the focus-gated answer-stage contract of § Focus-gated answer stages in `docs/docs/spec/member-runtime.md` and `SPEC.md`: the optional focus and screen gates, catalog validation including the Escape-free invariant, `focus_style` focused-choice resolution, the focus value in the request identity, the expected focus transition, `permission_continuation` rules (episode continuity, succession binding, no inspection token), waiting and failure reporting, and the native fixture's optional `cell_backgrounds`. <!-- completed: 2026-10-28T12:37 -->
- [x] Add the OpenCode 2.1.4 profile section to `docs/docs/spec/member-runtime.md` and `SPEC.md` with the captured anchors, answer keys, decision evidence and limits, and update the OpenCode limits in `docs/docs/spec/coding-agent-backends.md` and the skill references. <!-- completed: 2026-10-28T12:40 -->

### Step 5: Implement the profile

- [x] Write synthetic engine tests: catalog validation of gates (unknown choice, partial gating, final gate on another choice, a screen gate on an unknown or unrelated continuation rule or without its `continues_choice_id` stage before it, a continuation rule with operation or choices, a `1b` byte in any backend's stage), continuation screens (keeping the active episode open and `Visible` with no event or obscured timer, an unknown screen without an active episode, no inspection token, the succession binding and its rejection after human input, a gap or a different episode, a non-empty reason never satisfying the gate, the episode closing before the final stage failing as `uncertain`), `focus_style` resolution (one, none, several, an option without a choice entry and a different theme's colours), a styling-only focus change starting a new request generation, `cell_backgrounds` loading (geometry, masked rows, overlap, colour shape, absent key) and replay, a gated stage never written while its gate is unmet, the expected focus transition rebinding the selection without a new token, every other change between stages failing as `uncertain` with no further byte, waiting while the previous focus persists until the deadline, and unchanged ungated Claude and Codex plans. <!-- completed: 2026-10-28T12:58 -->
- [x] Implement the optional `required_focus_choice_id` and `required_screen_rule_id`, their catalog validation, `focus_style` resolution and the resolved focus in the request identity, the `permission_continuation` rule kind in the detector and episode tracker, the fixture's optional `cell_backgrounds` in the loader and replay, and the runner's per-stage gate, expected focus transition and continuation binding. <!-- completed: 2026-10-28T13:19 -->
- [x] Write fixture-backed tests: detection on the form fixture, each answer stage's gate holding on its pre-stage fixture (the stage-3 continuation on the empty rejection-reason dialog), no gate holding on the `Always allow`-focused fixture, the idle composer as a verified non-interaction boundary, no detection on the idle-composer fixture and on synthetic screens that carry form or dialog text outside a live region, no answer plan for an "always" choice, and catalog capability results for OpenCode 2.1.4 and for another OpenCode version. <!-- completed: 2026-10-28T12:58 -->
- [x] Add the native fixtures and profile `opencode-2.1.4-cafleet-ask-1` to the bundled catalog and its embedded fixture table. <!-- completed: 2026-10-28T13:19 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: 2026-10-28T13:30 -->

### Step 6: Forward the config location and relocate the OpenCode preset base

- [x] Document § Member configuration environment and the OpenCode preset base in `docs/docs/spec/cli-options.md`, `docs/docs/spec/coding-agent-backends.md`, `docs/docs/quickstart.md`, `SPEC.md` and `skills/cafleet/reference/coding-agents.md`, replacing every `~/.opencode` default with `~/.config/opencode`, and the doctor check lines for an OpenCode installation recorded at another base. <!-- completed: 2026-10-28T14:15 -->
- [ ] Write tests: for each backend `member create` forwards its config-location variable with the creator's exact value after `CAFLEET_DATABASE_URL` and adds nothing when it is unset, forwards no other backend's variable, still fails an invalid value before registration and records the variable in the launch descriptor; the runner's native child receives it; the OpenCode preset base defaults to `~/.config/opencode` and setup installs both presets and records that identity there; with an installation row at `~/.opencode`, doctor reports `opencode.configuration` `ok` at `~/.config/opencode`, `opencode.installation` `info` with the run-setup hint, and both preset checks reading only the new base; and the selected-preset precondition reads the new default. <!-- completed: -->
- [ ] Implement the forwarded variable in `member create`'s spawn environment and the `~/.config/opencode` default in config-dir resolution. <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: -->

### Step 7: Forward outer-terminal input bursts intact

COMMENT(director): Decision on finding (C): accept the proposed fix steps 1-3 (drain outer input until WouldBlock, EOF or the 65536-byte input limit within one tick and write the queue to the PTY master; start the next tick immediately while outer input moves; no escape-sequence framing of manual input). Programmer29: add it as a new Implementation step after Step 6 ("Forward outer-terminal input bursts intact") with docs, test and implementation tasks, document the contract where the runner's outer-input forwarding is specified (member-runtime.md and SPEC.md), remove the resolved finding-(C) programmer and director markers except this decision, and send complete; Tester20 writes its tests after Step 6's. The pre-fix read log is not required: the post-fix herdr Director-auto rerun is the acceptance evidence, with the bounded screen metadata as the fallback if detection still fails.

- [x] Document § Outer-terminal input forwarding in `docs/docs/spec/member-runtime.md` and `SPEC.md`, where the runner's outer-input relay is specified. <!-- completed: 2026-10-28T14:22 -->
- [ ] Write tests: one tick drains a fake outer input that returns 4096 bytes, then 3600 bytes, then would-block, queues all 7696 bytes in order and writes them to the PTY master within that tick; the manual-input generation advances by 7696; reads stop when the 65536-byte input queue is full, leave the rest unread and resume on the next tick; data followed by EOF forwards the data and reports outer EOF; a partial PTY write keeps the remainder queued in order; the pacing decision starts the next tick without the idle sleep while a tick read outer bytes or input stays queued, and keeps the idle sleep otherwise; and the existing transport tests still pass. <!-- completed: -->
- [ ] Implement the draining outer read and whole-queue PTY write in the transport tick, and the input-aware pacing in the service loop. <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint` and `mise //cafleet:typecheck`. <!-- completed: -->

### Step 8: Accept natively

COMMENT(tester): Finding (C) diagnosis: no rule condition fails on the saved screen. A scratch test (removed, nothing committed) dropped the two echo rows and wrote each remaining row with its SGR at its own row (CUP) into an 82x169 primary TerminalState with the cursor hidden; 170 columns gave the same result. The bundled detector returned `Interaction(PermissionRequest)`, rule `opencode-shell-permission`, `Complete` (no conflict), with the cursor at (21,69) accepted by cursor `any`. Every row stayed unwrapped, the region spanned heading to control line within 24 rows, every matcher matched once, and `match_answer` matched both the allow and deny plans with `focus_style` resolving `allow-once` from the 250,178,131 label background. The stack-trace rows below the form matched no rule. The missing `interaction_opened` therefore comes from the runner's live screen or episode path, not from the rule: either the runner's own vt100 grid differs from the herdr rendering (for example the suspected OSC 10/11 colour reply corrupting OpenCode's input, or OpenCode redrawing continuously after its error so no two compatible observations stay 100 ms apart), or the observer was degraded or gapped. Next evidence: capture the runner's own screen snapshot and observation (classification, output_seq and gap/degraded status from health or doctor) while the form shows, and compare it with this replay.

- [ ] Run Director-auto allow and deny on tmux and on herdr, recording the evidence in § Acceptance. <!-- completed: -->
- [ ] Run the three user-attended cases on one multiplexer with the user. <!-- completed: -->
- [ ] Verify the default-policy coverage case, and record latency, versions, geometry and unobserved cases. <!-- completed: -->

### Step 9: Validate the shipping surfaces

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` and `mise //docs:build`; verify help, embedded presets and the bundled catalog match the documentation, and record the execution summary in this document. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-10-28 | Initial draft |
| 2026-10-28 | Focus-gated multi-stage answers: OpenCode 2.1.4 has no focus-independent answer key, so allow is Enter on `Allow once` and deny is `h` then Enter on `Reject`, each stage gated on the observed focus; adds the engine extension, the revised blocker table, the per-stage capture cases and fixtures, and the engine tasks |
| 2026-10-28 | Empty-reason rejection: Enter on `Reject` opens a rejection-reason dialog in `mini`, so deny gains a third stage, Enter gated on the `permission_continuation` rule `opencode-reject-reason` with an empty reason; adds screen gates, continuation-screen episode and binding rules, the `opencode-ans-deny-003` case and the stage-3 fixture |
| 2026-10-28 | Live herdr acceptance findings: `member create` forwards the selected backend's config-location variable into the member environment, and the OpenCode preset base defaults to `~/.config/opencode`, the only directory OpenCode 2.1.4 loads agents from; adds Step 6 and renumbers acceptance and validation to Steps 7 and 8 |
| 2026-10-28 | Herdr reply split: an outer-terminal reply longer than 4096 bytes reached OpenCode a tick apart and was parsed as keys, so the runner drains outer input within one tick, writes the whole queue and skips the idle sleep while input moves; adds § Outer-terminal input forwarding and Step 7, and renumbers acceptance and validation to Steps 8 and 9 |
