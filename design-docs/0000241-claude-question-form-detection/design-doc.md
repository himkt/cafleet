# Single bundled profile per backend and Claude AskUserQuestion detection

**Status**: Complete
**Progress**: 40/40 tasks complete
**Last Updated**: 2026-11-22

## Overview

Every backend gets exactly one bundled runner profile with no native-version boundary, so profile selection, the below-minimum creation refusal and capture-interval provenance disappear, and the native version probe becomes diagnostic metadata. The single Claude profile gains rules that open a `question` interaction with no listed choice for a generic AskUserQuestion form and close it when the form disappears (himkt/cafleet#639). With detection working, the skipped Permanent and Transient phases of design 0000239's live verification run again.

## Success Criteria

- [x] `assets/profiles/bundled.json` holds exactly one profile per backend, with keys `backend`, `rules` and `capabilities`, and the catalog loader rejects a second profile for one backend.
- [x] `member create` launches every parsed native version with its backend's profile; `Error: unsupported native version …` no longer exists in code, SPEC, docs or skills.
- [x] A failed native version probe records `native_version` null, prints `Warning: native version unavailable for BACKEND: CAUSE.`, and records `coverage=tested` when the backend's profile declares detection.
- [x] Inspection and answers bind to the run's backend only: a run with a null `native_version` gets `answer_capability=tested` for a visible request whose tested plan exists.
- [x] Every AskUserQuestion capture (single question, multi-question tab bar, multi-select, "Review your answers" screen) on tmux and on herdr replays through the bundled Claude profile to its expected observation in the § Testing replay tests; this establishes the tmux half.
- [x] The § Live verification Question and Tabs phases on Herdr each produce one `question` `interaction_opened` with `review_question`, `permission inspect` returns `supported_choices` `[]`, `answer_capability` `unknown` and `capability_reason` `question has no tested answer`, and answering on the native surface produces `interaction_closed` with `not_visible`; with the herdr captures, this establishes the herdr half.
- [x] A quoted form above a live composer, the transcript viewer and every existing Claude form keep their current observation, except Claude Code's own `Model switch` question (`claude-2.1.302-auto-model-switch-question-1`), which renders with the AskUserQuestion component and becomes a `question` through `claude-ask-user-question`.
- [x] Outside `design-docs/`, no file mentions `min_version`, profile selection by version, a profile "from `<version>`", a profile boundary or a capture interval.
- [x] Design 0000239's Permanent and Transient phases each pass, or fail only for a recorded cause unrelated to detection with a follow-up issue opened. Their observed values replace 0000239's Step 9 "skipped" note; 0000239's live-verification Success Criterion is checked only when both pass, and otherwise stays open with a pointer to the follow-up issue.
- [x] `mise //:test`, `mise //:lint` and `mise //docs:build` pass.

---

## Background

Each bundled profile declares a SemVer `min_version`, and `member create` selects the profile with the greatest `min_version` at or below the probed native version (SPEC §6.9 Profile selection by minimum version). Three backends carry two profiles: Claude from `2.1.302` and `2.1.322`, Codex from `0.163.0` and `0.167.0`, OpenCode from `2.1.4` and `2.2.7`. The catalog guard requires every cited fixture's capture version to lie inside its profile's interval. Adding a rule to every Claude profile would therefore need a capture on a Claude Code below 2.1.322. Maintaining parallel profiles and old native installs costs more than the coverage it buys, so the user chose one unversioned profile per backend: an older native that renders a form differently fails to match and stays `unknown`.

Every Claude `question` rule matches one specific native dialog (`claude-auto-startup`, `claude-auto-env-setup-wizard`, `claude-auto-default-nudge`, `claude-question-pending-notice`). An AskUserQuestion form, as observed on Claude Code 2.1.323 on herdr in issue #639, is an unmatched screen:

```text
 ☐ Color

Which color do you prefer?

❯ 1. Red
  2. Blue
  3. Type something.
──────────────
  4. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel
```

`member show` reports `observation.eligibility` `hold:unknown` and the Director receives no event. Design 0000239's Permanent and Transient live phases open an interaction with exactly this form, so both were skipped (0000239 Step 9).

---

## Specification

### Single bundled profile per backend

#### Catalog shape

A profile entry has exactly the keys `backend`, `rules` and `capabilities`; `min_version` is gone from the schema, the decoder and every document. The catalog holds at most one profile per backend. `from_definitions` replaces the equal-threshold check with a uniqueness check that fails with `invalid native profile: more than one profile for one backend`. A backend with no profile entry keeps today's behavior: `coverage=unknown` and delivery fallback.

| Function in `src/coding_agent/catalog.rs` | New signature and behavior |
|---|---|
| `matching(backend, version)` | Replaced by `profile(backend) -> Option<&Profile>`, the backend's single profile |
| `unsupported_minimum` | Deleted |
| `detection_coverage(backend, version)` | `detection_coverage(backend)`: `tested` when `profile(backend)` declares detection |
| `answer_plans`, `answer_plan` (test), `verified_composer`, `composer_plan` | Drop the `version` / `native_version` parameter and resolve through `profile(backend)` |
| `Profile::min_version`, `Profile::min_version_text`, the raw `min_version` fields | Deleted |
| `validate_provenance` | Drops `Interval`; a citation requires a known fixture of the profile's backend, plus the existing non-empty, native-continuation and composer-evidence rules |
| The pattern-syntax report line | `<backend> <location>: `<construct>`` (no version), e.g. `` claude rule claude-bash-command: `\s` `` |

`Profile::answer_plans` and its plan structs drop the native version they carry, because a plan no longer binds to one.

With every caller gone (`matching`, `unsupported_minimum`, the threshold check, `Interval` and the per-version helper in `detector.rs` tests), `NativeVersion::precedence` in `src/coding_agent/mod.rs` is deleted, together with its `Ordering` import when nothing else in the file uses it. `NativeVersion::parse` stays: it validates the probe output and fixture capture versions.

#### Merge of existing profiles

Each backend's single profile is its newest profile today. The newest profiles already carry every predecessor rule, capability and answer plan unchanged; they differ only in the strict idle composer rule and the suggestion composer rule (SPEC §6.7 Native suggestions), and continuation rules differ only in their own `fixture_ids`.

| Backend | Single profile built from | Rules dropped | Citation handling |
|---|---|---|---|
| `claude` | Profile from `2.1.322`, plus the two AskUserQuestion rules | The `2.1.302` profile's looser `claude-idle-composer` | Detection and every answer plan cite the union of both profiles' citations for that rule or plan, plus the new captures; composer and submission keep the `2.1.322` citations |
| `codex` | Profile from `0.167.0` | The `0.163.0` profile's `codex-idle-composer` | Detection and answers cite the union; composer and submission keep the `0.167.0` citations |
| `opencode` | Profile from `2.2.7` | The `2.1.4` profile's `opencode-idle-composer` | Detection, answers and each continuation rule's `fixture_ids` cite the union; composer and submission keep the `2.2.7` citations |
| `hermes`, `pi` | Their only profile | — | Unchanged; only `min_version` is removed |

The composer and submission citations stay with the newest captures because only those screens are evidence for the strict idle rule. A fixture entry that no capability cites after the merge stays in the registry (provenance validation allows uncited entries) unless no remaining test reads it after Step 5's test cleanup; such a fixture is deleted together with its screen in `assets/profiles/native/<backend>.json`. Every fixture entry keeps its own `native_version`, the version the screen was captured on; it records a fact about the capture and bounds nothing.

#### Native version probe

`member create` keeps running `<resolved executable> --version` (SPEC §6.9 Native version probe), including the Hermes first-line rule and the concurrent Hermes `config path` batch, and records the result as the run's `native_version`. The value is diagnostic metadata: it appears in the run record, the event evidence object and `permission inspect`, and it selects nothing.

| Concern | Current | New |
|---|---|---|
| Profile used by a run | Selected from backend and native version | `profile(backend)` |
| Known version below the first boundary | Exit 1 with `Error: unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION` | Launches with the backend's profile |
| Failed probe | `native_version` null, `coverage=unknown`, `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` | `native_version` null, `coverage` from the profile, `Warning: native version unavailable for BACKEND: CAUSE.` |
| `coverage` consistency at creation and observer initialization | Selection from recorded backend and version yields recorded coverage; null version requires `unknown` | `profile(backend)` yields recorded coverage, whatever the version |
| Recovery of a recorded `tested` run | Reports explicitly when no profile is selected now | Reports explicitly when the backend's profile declares no detection now |
| `answer_capability=tested` | Also requires a non-null run `native_version` | Version plays no part |
| Answer and input binding | Backend and native version equal the run's | Backend equals the run's; the request token binds no native version |
| Verified-composer predicate | Recorded backend and native version select empty-composer observation and a plan | The recorded backend's profile supplies both |

The §8 `native_version` column, the evidence key order `backend,native_version,rule_id,screen_seq,evidence_digest`, the `permission inspect` field list and every JSON key stay unchanged, so no schema change and no JSON change ships.

#### What stays

- Fixture entries keep `native_version` as their capture version, and new captures verify that version from the capturing process's own `--version` output, as today.
- Role eligibility evidence keeps each requirement's checked native version and date: it records a live maintainer check, not a profile. The Member rows' "Bundled support: the X profile from `V`" becomes "Bundled support: the bundled X profile", and the skill Member Basis cells change the same way (`the Claude profile from 2.1.302` becomes `the bundled Claude profile`).
- The eligibility re-check trigger becomes "a change to the backend's bundled profile or recorded evidence" in SPEC §6.7 Role requirements and in `.claude/rules/documentation-maintenance.md`.
- Director-extension minimums (OpenCode 2.2.0, pi 1.5.0 in SPEC § Role eligibility) are extension facts and stay.

### Claude AskUserQuestion detection

#### Answer model

The form opens a `question` with no listed choice. The Director receives `interaction_opened` with `review_question`; `permission inspect` returns `supported_choices` `[]`, `answer_capability` `unknown` and `capability_reason` `question has no tested answer`; the Director relays the question to the user, who answers on the member's native surface (the existing "question with no listed choice" path of `director.md` § Consume observed interactions). Listing the options was rejected for three reasons. The model writes the option labels and count (two to four, plus `Type something.` and `Chat about this`) for each call, while a profile binds fixed choice ids to fixed bytes. The answer is the user's own decision about the member's question, not a dismissal. And SPEC §6.9 limits every choice answer plan to a declared dismissal choice, so listed options would need a new answer contract.

While the episode is open, the runner queues prompts, native commands and previews with zero bytes and `defer_reason` `interaction_open`, as for every blocking interaction. An Escape-led delivery therefore no longer dismisses a detected form.

#### Rules

Both rules are `question` rules with priority 100, `screen` `either`, `panel_separator` `│`, `cursor` `any`, `max_rows` 60, no operation, no choices, `verified_noninteraction` false and no answer plan. Their frame guard has no require pattern and rejects `^❯\x{a0}` (the live composer prefix) and `^  Showing detailed transcript · ctrl\+o to toggle` (the transcript viewer footer). The guard and three anchored rows carry liveness, so `cursor` is `any`: the cursor position while the user edits the `Type something.` row is not established. `max_rows` 60 admits four options with description rows and a preview box; the heading and footer anchors bound the region regardless.

In the patterns below, `WS` stands for the catalog's whitespace class `[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]` and `NWS` for its negation, written out in full in `bundled.json` and SPEC. The margins follow the issue's layout; Step 1's captures fix the exact margins, and Step 2 writes the captured literals.

| Rule | Region start | Region end | Required matchers |
|---|---|---|---|
| `claude-ask-user-question` | `heading` `^(?: \|←WS+)[☐☒✔]WSNWS` | `footer` `^Enter to select · (?:[^·]+ · )+Esc to cancelWS*$` | `heading`, `chat` `^(?:❯ \|  )[1-9]\. Chat about thisWS*$`, `footer` |
| `claude-ask-user-question-review` | The same `heading` | `cancel` `^(?:❯ \|  )[1-9]\. CancelWS*$` | `heading`, `review` `^Review your answersWS*$`, `submit` `^(?:❯ \|  )[1-9]\. Submit answersWS*$`, `cancel` |

The `heading` pattern matches the single-select one-question heading (` ☐ Color`, one leading space) and the tab bar that every multi-question and every `multiSelect` form starts at column 0 (`←  ☐ Color  ☐ Size  ✔ Submit  →`); an answered or toggled question renders `☒`. The repeated `[^·]+ · ` footer segment admits `↑/↓ to navigate`, the multi-question `Tab/Arrow keys to navigate` and the extra `ctrl+g to edit in VS Code` segment shown while `Type something.` is focused. Multi-select options render inside the same anchors, so they need no rule of their own. The `Review your answers` screen shows neither the footer nor a `Chat about this` row; it ends with `Ready to submit your answers?`, `❯ 1. Submit answers` and `  2. Cancel`, so its rule ends at the `Cancel` row.

A captured variant that lacks one of these anchored rows, or that shows the live composer prefix while the form is open, is reported to the Director before any rule is written; the Director decides between an added rule and a recorded layout limit.

#### Episode identity

The semantic fingerprint is `(kind, operation, choice labels)` without the rule id (`src/coding_agent/detector.rs` `observe`). Both rules produce `("question", None, [])`, so:

| Native transition | Episode outcome |
|---|---|
| Moving focus, switching tabs, toggling a multi-select option, typing into `Type something.`, reaching the review screen | One episode; no new event |
| A draft typed into the composer when the form mounts | On Claude Code 2.1.322 the form mounts over the draft with no `Claude has a question for you — …` notice, so the form opens the episode directly; SPEC records this outcome |
| Form answered or cancelled, then the idle composer stable for 250 ms | `interaction_closed` with `not_visible` |
| Two forms with no verified noninteraction boundary between them | One episode, the existing identity limitation of SPEC §6.9 Classification and episode identity |
| Both rules matching one frame | No conflict: their parts deduplicate |

#### Recorded limits

| Rendering | Behavior |
|---|---|
| A pane too narrow for the footer, heading or `Chat about this` row to stay on one row | Unmatched; answered on the native surface |
| A form taller than the pane, clipping the heading or footer | Unmatched |
| An older or newer Claude Code that renders different anchors | Unmatched (`unknown`), the accepted fail-safe of the single profile |

Step 1 records a `W_min` per rule: the narrowest captured width at which that rule still matches, from the `narrow` case for `claude-ask-user-question` and the `review-narrow` case for `claude-ask-user-question-review`. The captures give 60 columns for the form rule (both footers fit one row at 60 and wrap at 40) and 40 columns for the review rule. SPEC states both values.

### Native capture procedure

Captures record the installed Claude Code with `script -q -f -O <case>.raw -T <case>.timing -m advanced -c '<claude argv>'` at the case geometry, after verifying the version from `claude --version` in the same pane, as every Claude 2.1.322 fixture was captured (design 0000236 § Fixture capture method). The advanced timing log records the start geometry and every resize. A scratch converter outside the repository replays each recorded byte prefix up to a target state through `vt100` and emits `lines`, cursor, `alternate_screen`, `cell_backgrounds`, `cell_styles` and the canonical digest; the bundled-catalog guard and the replay tests then validate every fixture. tmux cases run on a private tmux server. herdr cases run in a dedicated herdr pane outside the fleet's panes, keeping the pane geometry the herdr server gives it, recorded in the fixture's `rows` and `cols`, like the existing `claude-2.1.322-composer-*-herdr-092-1` fixtures, whose `092` is the pane's column count (those screens are 36x92).

Fixture ids follow `claude-<captured version>-ask-<case>-<mux>-<cols>-<n>`: `<mux>` is `tmux` or `herdr`, `<cols>` the captured column count zero-padded to three digits, and `<n>` numbers the captures of one case from 1. For example, `claude-2.1.322-ask-tabs-herdr-092-2` is the second capture of `tabs` in a 92-column herdr pane.

Each case prompts Claude Code to call AskUserQuestion with the case's arguments, waits for the footer row, marks the recorded byte offset of each target state, and sends the case's navigation bytes. The capture script sends only arrow keys, Tab, Space, digits, typed draft text, the Ctrl+U (`0x15`) that `pending` uses to clear its draft, and the Enter that `single-closed` names. It never sends Escape to an open form.

| Case | AskUserQuestion arguments and capture actions | Mux and size | Expected observation |
|---|---|---|---|
| `single` | One question, two options (issue layout) | tmux 36x120, herdr | `claude-ask-user-question` |
| `single-closed` | `single`, then `1` and Enter; capture after the idle composer returns | tmux 36x120, herdr | Idle composer; closes the episode |
| `tabs` | Two questions; capture on each tab after Tab | tmux 36x120, herdr | `claude-ask-user-question`, one fingerprint |
| `multi` | One `multiSelect` question, three options; capture before and after Space toggles option 2 | tmux 36x120, herdr | `claude-ask-user-question` |
| `review` | `tabs`, then `1` on each question to reach the review screen | tmux 36x120, herdr | `claude-ask-user-question-review` |
| `review-narrow` | `review` at 36x60 and 36x40 | tmux | Positive where anchors hold; otherwise the review rule's `W_min` limit |
| `typing` | `single`, focus `Type something.`, type `abc` | tmux 36x120 | `claude-ask-user-question` |
| `narrow` | `single` at 36x60 and 36x40 | tmux | Positive where anchors hold; otherwise the form rule's `W_min` limit |
| `quoted` | The member prints the issue's form text as a code block in its reply | tmux 36x120 | Not a question |
| `pending` | A draft typed into the composer before the form mounts | tmux 36x120 | The form, mounted over the draft with no notice (§ Episode identity) |

Every case other than `single-closed` leaves its form open: after the last target state the script sends no closing byte, and the case's private tmux server, or its dedicated herdr pane, is discarded with the native process. Masking follows the existing review (banner, account, model, plan and home paths masked; every rule region unmasked).

### Documentation targets

Edits follow the project order: SPEC, `docs/`, README (no change: it states no profile versioning), skills, rules.

| Target | Edit |
|---|---|
| `SPEC.md` §6.3 `member create` | Drop the below-boundary refusal sentence and "with unknown coverage" from the probe clause |
| `SPEC.md` §6.3 Permission commands | Drop "a non-null run `native_version`" from the `tested` requirements |
| `SPEC.md` §6.7 Role requirements | Member Eligible/Bundled support rows: "the backend's bundled profile" instead of "the bundled profile for the evaluated native version" and "selecting a bundled profile"; verdict paragraph: drop "against the profile selected for the recorded native version", re-check after "a change to the backend's bundled profile or recorded evidence", and "a version probe that fails or selects no profile" becomes "a backend with no bundled profile" |
| `SPEC.md` §6.7 Role eligibility | Each Member row's "Bundled support" names "the bundled X profile" |
| `SPEC.md` §6.7 opencode and pi spawn argv | Drop the first-boundary refusal sentences and the "separately probes the native version for profile selection" clause |
| `SPEC.md` §6.7 Native suggestions | The sentence before the table: "A backend whose checked native version below renders inline suggestions has a fixture-backed suggestion rule". The paragraph after the table becomes: the bundled claude, codex and opencode profiles pair their suggestion rule with the strict idle rule (unchanged description), resting on the cell-styled captures of the table's checked versions; the bundled hermes and pi profiles keep an idle rule that accepts any text after the marker while the cursor sits on it (with the existing hermes evidence reason and its consequence for a typed draft), and only hermes among them also declares a suggestion rule. The "These boundaries …" sentence and every mention of earlier profiles are deleted |
| `SPEC.md` §6.5 Push notifications | "its selected profile's measured plain-input limit" becomes "its run's profile's measured plain-input limit" |
| `SPEC.md` §6.9 Launch policy and capability registry | In "An omitted selection is not validated against the version catalog, so a member whose probed version is newer than every profile boundary still launches with the default; a native version that rejects …", delete the clause up to the semicolon and keep "A native version that rejects `auto` or `--approve-for-me` exits immediately with `runner_exited` (below)". "(§ Profile selection by minimum version)" becomes "(§ Bundled profiles and native screen fixtures)"; "A failed probe or a backend without bundled profiles yields `coverage=unknown`" becomes "A backend without a bundled profile yields `coverage=unknown`"; "selected profile" becomes "the backend's bundled profile" |
| `SPEC.md` §6.9 Native version probe | The probe is diagnostic metadata; the new warning; delete "The probed version then selects the run's bundled profile" |
| `SPEC.md` §6.9 Profile selection by minimum version | Delete the section, including its SemVer precedence paragraph; move its fail-safe paragraph ("The rules match screen patterns, so a changed native UI fails safe …") and the runner-pins-its-profile paragraph, rewritten for one profile per backend, into Bundled profiles and native screen fixtures |
| `SPEC.md` §6.9 Bundled profiles and native screen fixtures | One profile per backend with keys `backend`, `rules`, `capabilities`; profiles named "the bundled Claude profile"; provenance rules without intervals; the per-profile plan table keyed by backend with its evidence versions; delete the predecessor-profile paragraph; every "profile from `V`" in the per-backend subsections becomes "the bundled X profile"; add the AskUserQuestion rules, their captures, both `W_min` values, the draft-mount outcome and the limits to the Claude subsection and the Detection row |
| `SPEC.md` §6.9 Executable native plans, Measured submission plans, Persistence (coverage), Coverage and latency | Replace selection by version and every "selected profile" / "profile selected" with the backend's bundled profile; "on a native version within its interval" becomes "on an installed native version"; "A run without a selected profile" becomes "A run whose backend has no bundled profile"; "the version-selected profile" becomes "the backend's bundled profile" |
| `SPEC.md` §6.9 Classification and episode identity | "known backend version" becomes "backend"; "Versioned profiles are bundled locally in CAFleet" becomes "Profiles are bundled locally in CAFleet"; "A run with no selected profile" becomes "A run whose backend has no bundled profile" |
| `SPEC.md` §6.9 Request-bound permission responses | The token's binding list drops "native version" ("bound in memory to the complete database/run/nonce/epoch, interaction/episode, live operation/choice/focus identity, …"); the answer binding becomes "its backend equals the run's"; "the run's selected profile" becomes "the run's bundled profile"; the "(§ Profile selection by minimum version)" pointer becomes "(§ Bundled profiles and native screen fixtures)" |
| `docs/docs/concepts/members.md` § Choose a native policy | Replace the minimum-version sentences with: each backend has one bundled profile; a native version that renders a form differently is not recognized and is answered on the member's surface; a failed version probe only leaves the recorded version empty |
| `docs/docs/concepts/coding-agents.md` | Add after the auto-mode dialog paragraph: a member's AskUserQuestion form reaches the Director as a question with no listed choice, and the user answers it in the member pane |
| `docs/docs/contributing.md` | Rewrite the adapter paragraph (one profile per backend, extended in place); "from a native version within the profile's interval" becomes "from the installed native version"; the later-version acceptance case drops `min_version` and "the selected profile"; delete the `Threshold selection` coverage row; the pattern report example loses its version; the bundled-catalog guard paragraph replaces "fixture validation shares the SemVer comparison of profile selection" with "fixture validation parses capture versions with the version probe's SemVer rule" |
| `assets/skills/cafleet/reference/cli.md` § `member create` | Probe `--version` and record it; no selection or refusal; the new warning |
| `assets/skills/cafleet/reference/cli.md` § member-runtime | "from its recorded backend/native version" becomes "from its recorded backend's profile"; "Absent plan/profile/version probe uses fallback" becomes "An absent plan or profile uses fallback" |
| `assets/skills/cafleet/reference/coding-agents.md` | The Role requirements intro: "a failed probe or unselected profile still launches with `coverage=unknown`" becomes "a backend without a bundled profile still launches with `coverage=unknown`"; each Member Basis names "the bundled X profile"; add a claude Note row (below) |
| `assets/skills/cafleet/roles/director.md` § Runtime events | "`member create` launches with its backend's bundled profile, which supplies every capability" instead of selection and refusal; "failed probes/unknown coverage preserve manual work and generated-input fallback" becomes "unknown coverage preserves manual work and generated-input fallback"; § Reconcile generated input: "the selected profile's independently tested answer plan" becomes "the bundled profile's independently tested answer plan" |
| `.claude/rules/documentation-maintenance.md` | Re-check eligibility "whenever a bundled profile or `Evidence:` line changes" |

Step 6's search catches any remaining sentence of this kind; each hit is rewritten in the same style before validation completes.

The new claude Note → applies at row:

| Note | Applies at |
|---|---|
| An AskUserQuestion form (a `☐`/`✔` heading or tab row, numbered options, `Chat about this` and the `Enter to select · … · Esc to cancel` footer, or its `Review your answers` screen) arrives as a `question` `interaction_opened` with `review_question`; inspection lists no choice. Relay the question to the user, who answers in the member pane. | [Observed interactions](../roles/director.md#consume-observed-interactions) |

### Testing

Every test is an in-file unit test of the file that owns the asserted behavior.

| File | Tests |
|---|---|
| `src/coding_agent/catalog.rs` | The bundled catalog has one profile per backend and passes provenance validation; a catalog with two profiles for one backend fails with `more than one profile for one backend`; a citation of another backend's fixture fails; each AskUserQuestion fixture replays to its expected observation through the bundled Claude profile; the quoted and transcript-viewer frames yield no question; every existing Claude fixture keeps its current observation with the two new rules present |
| `src/coding_agent/detector.rs` | The `tabs`, `multi` and `typing` captures and the `single` capture yield equal fingerprints; the `review` capture equals them; the existing `claude-question-pending-notice` fixture and the form fingerprints are equal. Frame-guard mutation cases in the style of `a_dialog_a_quoted_above_a_live_composer_is_no_interaction`: the captured live `single` form with a `❯\u{a0}` composer row written into it yields no interaction, and the same form with the `  Showing detailed transcript · ctrl+o to toggle` row written into it yields none. The `quoted` capture alone does not exercise the guard, because its transcript margin already fails the `heading` anchor |
| `src/coding_agent/mod.rs` | A probed version below every former boundary (e.g. `0.0.1`) launches with the backend's profile and `coverage=tested`; a failed probe records null `native_version`, `coverage=tested` for a backend with detection, and the new warning text |
| `src/runtime/observation.rs` | The creation/observer coverage consistency table: `(None, tested)` is consistent for a backend whose profile declares detection, `(Some, unknown)` is inconsistent; a scenario that opens an AskUserQuestion episode, queues a prompt with `interaction_open`, and closes `not_visible` after the idle composer, then delivers the queued prompt |
| `src/runtime/input.rs`, `src/runtime/input_client.rs` | `permission inspect` on an AskUserQuestion episode returns `[]`, `unknown` and `question has no tested answer`; a run with null `native_version` gets `tested` capability for a permission request with a tested plan |

Tests deleted because the behavior no longer exists: threshold and selection tests (equal/between/above/below boundaries, permutation, prerelease and build-metadata ordering, malformed `min_version`), the interval provenance tests (`fixture_interval_bounds_every_cited_capture_by_its_profile_thresholds`, `composer_operation_provenance_captured_inside_the_interval_serves_every_later_run`), the below-minimum refusal tests, the per-version profile helpers in `detector.rs` tests, and every test whose assertion depends on a predecessor's looser idle rule (the strict idle rule's tests cover the merged behavior).

### Live verification

All phases use the branch binary on Herdr with a dedicated database `D` (`--database-url sqlite:///D` on every command) and the installed Claude Code.

| Phase | Procedure | Expected |
|---|---|---|
| Question | 1. Create member `M` and let it go idle. 2. `member prompt M` asking it to call AskUserQuestion with one question and two options. 3. Run `permission inspect` for the event. 4. Answer option 1 on `M`'s native surface | The Director inbox receives `interaction_opened`, `classification` `question`, `director_action.kind` `review_question`. Inspect: `supported_choices` `[]`, `answer_capability` `unknown`, `capability_reason` `question has no tested answer`. After the answer: `interaction_closed` with `not_visible`, and `member show` reports `observation.eligibility` `composer_empty` |
| Tabs | `member prompt M` asking for two questions in one call; switch tabs, answer both, submit on the review screen | Exactly one `interaction_opened` and one `interaction_closed` for the whole form |
| 0000239 Permanent | Design 0000239 § Live verification, Permanent row, as written | That row's Expected column |
| 0000239 Transient | Design 0000239 § Live verification, Transient row, as written | That row's Expected column |

A 0000239 phase that fails for a cause unrelated to detection is recorded with its observed values in both design docs, and a follow-up GitHub issue is opened for it.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Step 1 (captures and fixtures) deliberately precedes the documentation step required first by `.claude/rules/documentation-maintenance.md` § Implementation Order, because SPEC must carry the margins, `W_min` values and draft-mount outcome that only the captures establish.

### Step 1: Capture AskUserQuestion evidence

- [x] Write the § Native capture procedure capture script and scratch converter outside the repository, sending only the listed byte set <!-- completed: 2026-11-22T01:53 -->
- [x] Run the cases on tmux with the installed Claude Code and capture the herdr cases on a herdr server, verifying the version from the pane's `--version` output <!-- completed: 2026-11-22T01:53 -->
- [x] Review masking and convert the captures into fixture entries and screens <!-- completed: 2026-11-22T01:53 -->
- [x] Report the captured margins, `W_min` and any variant that lacks an anchor or shows the live composer to the Director before Step 2 <!-- completed: 2026-11-22T01:53 -->

### Step 2: Documentation

- [x] Update `SPEC.md` per § Documentation targets, including the AskUserQuestion rules with their captured literals <!-- completed: 2026-11-22T01:50 -->
- [x] Update `docs/docs/concepts/members.md`, `docs/docs/concepts/coding-agents.md` and `docs/docs/contributing.md` <!-- completed: 2026-11-22T01:28 -->
- [x] Update `assets/skills/cafleet/reference/cli.md`, `assets/skills/cafleet/reference/coding-agents.md` (Member Basis cells and the new claude Note row) and `assets/skills/cafleet/roles/director.md` <!-- completed: 2026-11-22T01:28 -->
- [x] Update `.claude/rules/documentation-maintenance.md` <!-- completed: 2026-11-22T01:28 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-22T01:28 -->

### Step 3: Merge the bundled profiles

- [x] Rebuild `assets/profiles/bundled.json` with one profile per backend per § Merge of existing profiles, removing every `min_version` <!-- completed: 2026-11-22T01:32 -->
- [x] Union the detection, answer and continuation citations as the merge table states <!-- completed: 2026-11-22T01:32 -->
- [x] Add `claude-ask-user-question` and `claude-ask-user-question-review` to the Claude rules and `detection.rule_ids`, citing the Step 1 fixtures <!-- completed: 2026-11-22T01:57 -->

### Step 4: Catalog and runtime code

- [x] Remove `min_version` from the raw and compiled profile types and add the one-profile-per-backend check <!-- completed: 2026-11-22T01:37 -->
- [x] Replace `matching` with `profile(backend)`, delete `unsupported_minimum`, and drop the version parameter from `detection_coverage`, `answer_plans`, `answer_plan`, `verified_composer` and `composer_plan` and their callers <!-- completed: 2026-11-22T02:12 -->
- [x] Remove the interval from `validate_provenance` and the version from the pattern-syntax report line <!-- completed: 2026-11-22T02:12 -->
- [x] Delete the below-minimum refusal in `src/coding_agent/mod.rs` and change the probe warning and failed-probe coverage <!-- completed: 2026-11-22T01:37 -->
- [x] Update `src/coding_agent/detector.rs` `Detector::new` to take the backend's profile <!-- completed: 2026-11-22T01:37 -->
- [x] Change the creation/observer coverage consistency check and recovery's tested-coverage report in `src/runtime/` to the backend's profile <!-- completed: 2026-11-22T01:37 -->
- [x] Remove the native version from answer plans, the request-token binding, the `tested` capability requirement and input validation in `src/runtime/input.rs`, `src/runtime/input_client.rs` and `src/runtime/control.rs` <!-- completed: 2026-11-22T01:37 -->
- [x] In `src/runtime/observation.rs`, delete `ObservedRequest.native_version` and its comparison in the request equality, and the `let Some(version) = &self.run.native_version else { return Ok(None) }` guard before `answer_plans`, so a null-version run builds its `ObservedRequest` and sets `unplanned_question_episode` <!-- completed: 2026-11-22T01:37 -->
- [x] Delete `NativeVersion::precedence` and its unused `Ordering` import in `src/coding_agent/mod.rs` <!-- completed: 2026-11-22T01:37 -->

### Step 5: Tests

- [x] Delete the tests listed under § Testing as removed behavior <!-- completed: 2026-11-22T02:12 -->
- [x] Add the `catalog.rs` tests from § Testing <!-- completed: 2026-11-22T02:12 -->
- [x] Add the `detector.rs` fingerprint tests <!-- completed: 2026-11-22T02:12 -->
- [x] Add the `coding_agent/mod.rs` probe and launch tests <!-- completed: 2026-11-22T02:12 -->
- [x] Add the `observation.rs` consistency table cases and the AskUserQuestion scenario <!-- completed: 2026-11-22T02:12 -->
- [x] Add the `input.rs` / `input_client.rs` inspection and null-version capability tests <!-- completed: 2026-11-22T02:12 -->
- [x] Delete fixtures (entry and screen) that no capability cites and no remaining test reads <!-- completed: 2026-11-22T02:29 -->

### Step 6: Validation

- [x] Run `mise //:test` <!-- completed: 2026-11-22T02:31 -->
- [x] Run `mise //:lint` <!-- completed: 2026-11-22T02:31 -->
- [x] Run `mise //docs:build` <!-- completed: 2026-11-22T02:31 -->
- [x] Run `git grep` over tracked files outside `design-docs/` for `min_version`, `unsupported_minimum`, `minimum supported version`, `Profile selection by minimum version`, `profile-selection-by-minimum-version`, `profile from`, `profiles from`, `profile boundar`, `These boundaries`, `profile interval`, `within its interval`, `first boundary`, `first profile`, `greatest minimum`, `selected profile`, `profile selected`, `version-selected`, `Versioned profiles`, `SemVer precedence` and `coverage unknown.`, and confirm nothing is found <!-- completed: 2026-11-22T02:31 -->
- [x] Recheck every § Documentation targets row against the shipped behavior <!-- completed: 2026-11-22T02:31 -->
- [x] Install the branch CLI through its mise task <!-- completed: 2026-11-22T03:09 -->
  - Installed with `mise //:all-install` after the execution fleet was deleted, so the running fleet kept its installed binary; live verification used the built branch binary `target/release/cafleet`.

### Step 7: Live verification

- [x] Run the Question phase and record each observed value <!-- completed: 2026-11-22T02:57 -->
  - Passed on the branch binary with Claude Code 2.1.323 on Herdr (dedicated database). The Director inbox received `interaction_opened` (interaction 1), `classification` `question`, `director_action.kind` `review_question`, rule `claude-ask-user-question`. `permission inspect`: `supported_choices` `[]`, `answer_capability` `unknown`, `capability_reason` `question has no tested answer`. After `1` on the native surface: `interaction_closed` with `not_visible` (rule `claude-idle-composer`), and `member show` reported `observation.eligibility` `composer_empty`.
- [x] Run the Tabs phase and record each observed value <!-- completed: 2026-11-22T02:57 -->
  - Passed: the two-question form (`←  ☐ Color  ☐ Size  ✔ Submit  →`) through Tab, Left, both answers (`☒` headings) and `Submit answers` on the `Review your answers` screen produced exactly one `interaction_opened` (interaction 2) and one `interaction_closed` (`not_visible`).
- [x] Run 0000239's Permanent phase and record each observed value <!-- completed: 2026-11-22T02:57 -->
  - Passed; values recorded in design 0000239 Step 9. A first attempt was discarded because the trigger could not be created before the second prompt queued; it still showed the queued prompt delivering after the form closed.
- [x] Run 0000239's Transient phase and record each observed value <!-- completed: 2026-11-22T02:57 -->
  - Passed; values recorded in design 0000239 Step 9. M's Claude Code refused to run any `sleep`, so the hold started first and the request was typed into M's pane through the multiplexer (no database write); two earlier attempts were discarded because the form appeared before the hold.
- [x] Replace 0000239's Step 9 "skipped" note with the observed Permanent and Transient values; check 0000239's live-verification Success Criterion when both pass, and otherwise leave it open with a pointer to the follow-up issue <!-- completed: 2026-11-22T02:57 -->
- [x] Open a follow-up issue for any 0000239 phase failure unrelated to detection and record it in both design docs <!-- completed: 2026-11-22T02:57 -->
  - No phase failed, so no follow-up issue was needed.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-11-22 | Initial draft |
| 2026-11-22 | Review round 1: per-half evidence for the tmux/herdr criterion and the 0000239 pass-or-recorded-failure criterion; `NativeVersion::precedence` deletion; `pending-sent` and `review-narrow` captures with a `W_min` per rule; herdr geometry, fixture id form and case endings; frame-guard mutation tests; the capture-first ordering exception; the `observation.rs` version guard; documentation rows for Native suggestions, Push notifications, Classification, the request token, the skill intro, cli.md fallback, director.md and the contributing guard paragraph; a wider removal search |
| 2026-11-22 | Execution: captures use `script(1)` and a scratch converter instead of a native-capture driver mode, because one driver arm yields a single two-sample pair and needs an external orchestrator (user decision) |
| 2026-11-22 | Execution (Director decisions on Step 1 captures): heading admits the column-0 tab bar and `☒`; footer admits several middle segments; the review rule ends at its `Cancel` row and requires `Submit answers`, since that screen has no footer or `Chat about this` row; `W_min` 60 (form) and 40 (review); `pending-sent` dropped because 2.1.322 mounts the form over a draft with no notice |
| 2026-11-22 | Execution: Claude Code's `Model switch` question, drawn with the AskUserQuestion component, now opens a no-choice `question` (Success Criterion 7 amended) |
