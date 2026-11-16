# Simplify Quickstart Setup and Verify Permission Requirements

**Status**: Complete
**Progress**: 17/17 tasks complete
**Last Updated**: 2026-11-16

## Overview

Make the Quickstart a shared installation flow followed by the recommended configuration for each backend. Establish Codex permission and directory-trust requirements through reproducible checks, and keep the contract, user documentation and agent instructions consistent with those results. Include focused implementation fixes when verification reveals a relevant setup, launch or rule defect.

## Success Criteria

- [x] The Quickstart runs `cafleet setup` once in Install; backend sections proceed from that shared prerequisite, with correct numbering and configuration-directory guidance.
- [x] Claude Code has one Quickstart configuration recommending `auto`; supported alternatives remain in detailed references.
- [x] Codex instructions cover installed rules, restart, Director approval policy, listener escalation, `ready`, notification delivery and ownership of one receiver.
- [x] Native tool-path checks pass on Linux either with both broad sandbox grants omitted or with only a demonstrated necessary grant retained; the published configuration identifies the conditions requiring each retained setting.
- [x] Working-directory trust guidance follows Linux fresh-directory and worktree results for every backend whose live cases are required by the approved acceptance scope.
- [x] Documentation builds, relevant code checks pass, and all Linux evidence required by the approved scope is available before implementation acceptance.

---

## Background

[Issue 610](https://github.com/himkt/cafleet/issues/610) identifies repeated backend installation steps, duplicated Claude permission examples and unconditional Codex sandbox/trust prerequisites. The detailed Codex reference also says Codex has no classifier, while the launch contract already supports automatic review.

The current rule asset allows the `cafleet` prefix. The implementation composes Codex members as `codex --no-daemon --approve-for-me`; a Director uses the shared daemon for listener forwarding. These roles have different launch requirements, and command authorization alone does not establish directory-consent behavior.

The user authorized design creation now. Implementation includes necessary fixes discovered by verification; runtime checks below are implementation work, and their absence does not prevent approval of this design.

---

## Specification

### Evidence and acceptance

The user's review of PR #611 requires an actual reduction in reading and setup decisions. Keep Quickstart shorter than its original main-branch version, with shared setup, one Claude recommendation, a minimal Codex recipe and backend launch prerequisites. Keep user-facing backend references actionable and concise. Store native verification matrices, exact runtime provenance, fixture diagnostics and investigation history in this design and .verification/; published documentation carries only the resulting configuration and behavior. Preserve verified permission, escalation and trust semantics. The user has authorized publication of this branch and PR; this correction remains within that scope.

CAFleet registration for the editorial revision failed because the prior herdr server w1 is no longer running. Continue with scoped fallback delegation and independent review, preserving file ownership, checks and Git boundaries.

Use installed native backend versions and record their exact versions. Codex 0.167.0 is a required regression target; compatibility claims follow recorded evidence rather than an assumed historical-version matrix.

| Verification area | Execution scope |
|---|---|
| Linux setup, permissions and reception | Required: isolated shared setup and asset checks for the default backend registry, native command-path and approval/reviewer checks, listener delivery and cleanup |
| Linux member readiness and trust | Required: fresh-repository/worktree cases for every backend whose live cases are in scope under the exclusions below |
| macOS | Excluded by the user on 2026-11-16: native minimum-configuration/listener checks and the backend trust matrix. Existing incident evidence remains input to troubleshooting guidance |
| pi/Hermes live provider readiness and trust | Excluded by the user on 2026-11-16: provider-dependent model/readiness and fresh-repository/worktree trust cases. Shared setup/asset checks and observed prerequisite diagnostics remain part of the evidence |

Keep excluded live cases classified by their observed prerequisite diagnostics; their exclusion establishes no successful model readiness or directory-trust result. Record implementation completion in Progress and retain Status as In Progress until final independent review and user acceptance.

Acceptance depends on completing the required Linux scenarios through the native tool's documented rule-authorized or approved-escalation path. Record host limitations for the affected execution path and evaluate each required scenario against its actual result; a failed direct sandbox probe leaves successful rule-authorized or approved-escalation results valid. Complete any required scenario that remains unavailable on a functioning Linux host before acceptance.

| Existing observation | What it establishes | Remaining limit |
|---|---|---|
| Issue report: macOS, CAFleet 0.35.2, Codex 0.167.0, herdr, `workspace-write`; normal listener returned `cannot run ps: Operation not permitted (os error 1)` | Process inspection can fail inside the macOS sandbox | The report does not establish a fresh minimum configuration |
| Issue report: the same command succeeded with `sandbox_permissions: "require_escalated"`, emitted `ready`, and delivered notifications under automatic approval review | That escalation and reviewer combination worked | `on-request` is not established as the only working user-facing mode |
| Director-reported Codex 0.167.0 rule check matched the direct listener argv; raw `bash -lc` argv had no matching rule, including a plain poll script | The supplied argv shapes have different rule-check results | Raw shell-wrapper checks do not establish normalization by the native execution tool |
| Director-reported current listener emitted `ready` through escalation | The current session can start that receiver | Existing configuration is not a clean-setup test |
| Director's Linux Codex 0.167.0 `codex sandbox -- true` failed with bwrap namespace creation, including through the escalated host path | This host cannot start that direct sandbox path | Required native command-tool scenarios are evaluated separately through installed rules and approved escalation |
| Linux standalone Codex 0.167.0 members omit broad socket/database grants, disable automatic temporary-directory grants and use an external broker database | Standard trusted repository/worktree members send `ready` and complete broker send/poll/ACK; a separate exact-0.167.0 `--no-daemon exec` probe captures a real herdr pane without explicit escalation | Standalone binary provenance is independent of the daemon-backed Director checks |
| Daemon-backed Director checks use CLI 0.167.0; the initial server version is unrecorded and a managed replacement is observed at 0.169.0 | The variable-bearing listener reaches `ready` through approved escalation and delivers queued notices; later pane capture and final receiver recovery/closure succeed under the restrictive configuration | Client `session_meta.cli_version` supplies client provenance; actual server versions come from separate binary/updater records |
| Native Codex single-request user-review decisions allow installed-skill reads and deny the exact harmless `/usr/bin/true` request; a `never` session's matching broker send succeeds while an ordinary nonmatching command fails at bwrap startup | User-review allow/deny, automatic review and rule-authorized execution with unavailable approval under `never` are established by native decisions, receipts and policy contexts | Native `never` preserves its restriction on requesting escalation; the recorded namespace failure is an execution result rather than an approval denial |
| The initial successful listener's native execution handle later becomes unavailable; process absence is confirmed, pending messages are manually recovered, and one replacement emits `ready`, receives queued `fleet_deleted` closure and exits 0 | The documented recovery and one-receiver cleanup complete, and both disposable fleets are removed | Updater records show the server's shutdown/replacement during the test, consistent with the lost handle; the causal attribution remains an inference and the initial server binary is unrecorded |
| Linux Claude Code 2.1.319 standard `auto` members request native folder trust in the fresh repository and detached worktree; both send `ready` after persisted fixture trust | Directory consent is a prerequisite on this tested launch, independently of auto-mode availability | Trust scope follows the native prompt; the fixture establishes readiness with configured trust |
| Linux OpenCode 2.2.6 standard `mini --standalone --agent cafleet-ask` members send `ready` in both fresh directories without directory consent | The tested default launch needs no extra directory-trust prerequisite | Existing native provider authentication remains available in this test |
| Linux pi 1.5.0 starts with an unknown model and no configured default provider/model or credentials | Native provider/model configuration is a prerequisite for model readiness | Directory trust and broker readiness remain unverified under the live-case exclusion above |
| Linux Hermes 2026.9.24 reports missing Nous Portal login after configuration preflight | Native provider authentication is a prerequisite for model readiness | Directory trust and broker readiness remain unverified under the live-case exclusion above |
| Drafter shell returned `codex-cli 0.166.0` | The Drafter's local command resolves a different version | Its checks cannot stand in for the Director's 0.167.0 evidence |

For this run, `.verification/native-version-provenance.json` separates the CLI/standalone binaries from the managed daemon, and `.verification/daemon-updater-events.json` records the observed shutdown and replacement. `.verification/socket-codex-0167.json` records the exact-version standalone socket probe; `.verification/native-receiver-recovery.json` retains confirmed receiver absence, manual recovery, one replacement, queued `fleet_deleted` closure and exit code 0. The updater timing supports a possible explanation for the earlier handle loss; the initial server binary and a definitive causal attribution remain unrecorded.

Create implementation evidence under this task's hidden `.verification/` directory. Each record includes date, operator, OS, CAFleet source revision and installed version, native CLI version, multiplexer/version, actual child argv and cwd, effective approval policy/reviewer, relevant configuration layers and rules, and redacted command/tool results. Record daemon-backed client and server versions separately, with an uncaptured server binary explicitly unrecorded; link updater events and label inferred causes as inferences. Record whether each case passed, failed or was unavailable, with a concrete reason.

Use isolated configuration roots, database and temporary workspaces so verification preserves the executing team's installed assets and sessions. Record authentication/onboarding prerequisites separately from directory trust; redact credentials and thread identifiers consistently. Keep the same root selections through setup and launch.

Select the disposable database explicitly with `--database-url sqlite:///<absolute-path>`; the URL is the `sqlite:///` prefix followed by the absolute filesystem path. Backend configuration-root overrides select assets independently of the database. Before running setup, resolve the database location and the active team's selected database location, including any explicit override. For the check that omits `writable_roots`, establish every effective sandbox-writable location, including test workspaces, configured roots and automatic temporary-directory grants, and assert that the disposable database is outside all of them and differs from the active team's database. Resolve existing parent directories, symlinks and platform aliases when the new file does not yet exist. Stop before setup if an assertion fails or the required paths or effective writable locations cannot be established, and record the successful preflight in the evidence.

Automatic temporary-directory access is part of this preflight: record the host's resolved temporary locations and effective `exclude_tmpdir_env_var` and `exclude_slash_tmp` settings, described in the [official advanced configuration reference](https://learn.chatgpt.com/docs/config-file/config-advanced#approval-policies-and-sandbox-modes). If the test disables those automatic grants instead, record the effective disabling settings and verify that they apply before treating a temporary location as outside the writable boundary.

Pass that same absolute URL to setup, fleet registration and every verification command that accesses CAFleet state, including broker reads/writes, lifecycle operations, inspection, cleanup and the listener. Test member prompts include the usual identity lines plus `DATABASE URL: {database_url}`; CAFleet renders the placeholder, and the prompt instructs the member to pass its rendered URL on every command, starting with initial ready/send/poll/ACK. Confirm the rendered prompt and recorded runner arguments select the disposable database before accepting member results. The examples below use the illustrative non-temporary path `sqlite:////absolute/verification-state/cafleet.db`; replace it consistently with a fresh, host-specific location that passes the preflight.

Reproducible operator logs from a functioning Linux host are acceptable when they contain the same required evidence as direct execution. Static source inspection and the Issue's existing report retain their diagnostic scope; required native Linux results establish acceptance.

### Shared installation and Claude Quickstart

The shared Install section owns installation of CAFleet followed by one `cafleet setup`. Tell readers to install the native backends they intend to use and choose any relocated configuration directories before setup. Preserve the upgrade instruction to rerun setup after upgrading.

Every backend-specific Quickstart section continues with configuration and launch. Remove repeated `cafleet setup --coding-agent ...` steps and repair dependent numbering and references. Preserve configuration-path behavior, including pi's relocated agent directory. Hermes must be available during shared setup for its plugin to be installed and enabled; installing it later leads to the detailed targeted-repair instructions.

Keep setup selection options in the CLI reference and targeted troubleshooting. Detailed backend configuration may explain what setup installs or repairs without making a second installation a normal prerequisite.

Claude Code recommends `auto` and retains the existing auto-mode JSON example with `permissions.defaultMode` set to `auto`. Preserve its skill/read/edit permissions, optional pane-input confirmation entries and account/model availability requirement. Update the Configure table accordingly and remove the Quickstart's separate Other permission modes subsection and duplicate example. The detailed reference continues to explain supported alternatives and when their profiles apply.

### Codex rules and Director policy

Describe the intended sequence: install the rules through shared setup, restart Codex to load them, select an approval policy that permits needed escalation, then run isolated CAFleet commands through the native command tool. Setup owns `~/.codex/rules/cafleet.rules` by default; custom rules belong in a separate file, with relocated homes covered by the existing configuration-directory reference.

The proposed Quickstart configuration for a Director using the workspace sandbox is:

```toml
approval_policy = "on-request"
sandbox_mode = "workspace-write"
```

Place these top-level keys before TOML table headers. Publish this as the minimum verified example only after the checks below pass; add any further setting only for a demonstrated remaining requirement. The detailed equivalent launch is `codex --ask-for-approval on-request --sandbox workspace-write`.

Approval policy controls whether an escalation request can be made. The reviewer controls who decides that request. Explain their relationship in the detailed reference:

| Effective configuration | Required behavior |
|---|---|
| `on-request` with the user reviewer | Necessary escalation can request operator approval |
| `on-request` with `approvals_reviewer = "auto_review"`, or the native `--approve-for-me` mode when available | Necessary escalation goes through automatic review; success depends on the decision and managed policy |
| `never` | A matching authorized command can still run; an operation that requires a new approval cannot obtain it |

Retain existing automatic review when it supplies the required escalation. Recommend `on-request` when the effective policy otherwise prevents that request or the operator wants to approve it. Changing the policy alone does not select the reviewer. Other eligible granular policies may be linked in the detailed reference without expanding the Quickstart into a policy catalog. These distinctions follow the [official Auto-review documentation](https://learn.chatgpt.com/docs/sandboxing/auto-review).

Keep Director startup separate from `cafleet member create --permission-mode`, which selects a spawned member's launch policy. Preserve the standard member's `--no-daemon` launch; a Codex Director needs the shared daemon used by `codex queue`.

The [official rules documentation](https://learn.chatgpt.com/docs/agent-configuration/rules) describes matching `allow` rules as authorizing execution outside the sandbox. More restrictive matches and managed requirements still govern the operation; shell syntax can also affect the command presented for matching. This supports testing removal of broad sandbox grants, rather than assuming the rule has already proved it.

### Native command-path and minimum-configuration checks

Use the installed rule asset and a restarted native session. Begin with both `network_access` and the CAFleet database directory's extra `writable_roots` omitted, including inherited grants from active configuration layers. Keep the database outside every effective sandbox-writable location, including automatic temporary-directory grants, as established by the preflight, and use a real multiplexer socket so the test exercises both relevant boundaries.

Check direct argv with the native rule-check command, using literal disposable IDs and a sample thread value:

```bash
codex execpolicy check --pretty --rules assets/presets/codex/cafleet.rules -- cafleet message listen 3 --codex-thread example-thread --database-url sqlite:////absolute/verification-state/cafleet.db
```

Then verify the actual execution tool, including its shell wrapper and variable handling, using:

```bash
cafleet message listen 3 --codex-thread "$CODEX_THREAD_ID" --database-url sqlite:////absolute/verification-state/cafleet.db
```

The ID is illustrative; the implementer uses the registered test Director's literal ID. Record the original tool command separately from any observed expanded argv. Raw `execpolicy check` results for a shell wrapper remain diagnostic evidence, not proof of native tool behavior.

Run one CAFleet command per tool invocation. Test ordinary Director commands, a default Codex member's send/poll/ACK exchange, and the variable-bearing listener command, all selecting the same disposable database explicitly. A minimum-configuration pass requires successful broker database writes, multiplexer access, listener startup and notification delivery with both broad grants omitted or only a demonstrated necessary grant retained under its documented conditions. Test native interactive approval and automatic review separately, recording their effective policy and reviewer.

Run this protocol on Linux, with Codex 0.167.0 included. If a check fails, distinguish an unloaded rule, a command-shape mismatch, a stricter policy, and a demonstrated sandbox access need. Fix a CAFleet setup/launch/rule defect when appropriate and repeat the affected checks. If an additional sandbox setting remains necessary, publish only the setting and configuration-specific reason the evidence supports; the minimum-configuration gate passes only after that documented recipe works. Remove obsolete blanket requirements and their explanations everywhere they occur.

### Listener startup and failure handling

The detailed Codex reference owns the complete troubleshooting procedure; the Quickstart includes the prerequisite, key symptom and a link to that procedure. The Codex skill overlay carries the corresponding executable instructions for agents.

1. Start one receiver with the registered Director ID, its own `CODEX_THREAD_ID` and the verification run's explicit `--database-url`, retaining the native execution handle.
2. If startup fails before `ready` because the sandbox blocks process inspection or hides the Codex ancestor/daemon, confirm the failed process exited. Retry the same command through the execution tool with `sandbox_permissions: "require_escalated"` under the host's approval flow.
3. Verify the `ready` JSON record identifies the expected member and fleet, and retain the successful receiver handle. A successful shell launch without `ready` is insufficient.
4. Send a test member message and require the queued listener notice to reach the Director, followed by its notice-prompted poll and ACK. Keep one receiver running; replace it only after confirming the previous receiver exited.
5. Verify receiver shutdown when its owner exits or the disposable fleet is deleted, following the existing lifecycle contract.

Include the macOS `cannot run ps: Operation not permitted (os error 1)` symptom, Linux namespace failures and the existing `--codex-thread requires a codex ancestor process` error. Ancestor errors also require checking that the command actually runs under the intended Codex session.

When escalation is unavailable or refused, report the actual policy/error and resolve it through the operator or the established Director routing procedure. Preserve the host's restrictions and use the existing manual poll/finite-wait recovery where authorized; record that this does not establish working listener reception. Automatic review is a decision path, not a guaranteed approval.

### Working-directory trust

Remove the shared Trust the working directory subsection and replace its blanket advice with evidence-specific guidance beside the affected backend. The detailed backend reference owns general trust troubleshooting. A backend retains a required trust step only when its tested standard launch blocks on directory consent.

For each backend whose live cases are required by the acceptance scope, test the actual generated default member launch on Linux in both a fresh Git repository and a newly created worktree. Establish that the relevant trust store contains no prior approval for the directory or inherited approval that invalidates the fresh case. Register and launch against the disposable database using the propagation procedure above, capture startup until it reaches an actionable state, and require its initial broker `ready` message in that database before declaring unattended startup successful.

| Backend | Launch behavior the trust check must preserve |
|---|---|
| Claude Code | Default `auto`, the generated broker allow rule and the member's normal prompt; distinguish directory consent from auto-mode availability/dialogs |
| Codex | Exact `--no-daemon --approve-for-me` member launch; verify 0.167.0 even though the installed command rule allows CAFleet |
| OpenCode | `mini --standalone` and the selected installed default agent preset |

A blocking directory-consent screen is a positive result that trust setup is required, not a backend failure. Record its exact backend/version behavior, approve the directory through the native supported flow and rerun to demonstrate successful readiness. Explain worktree trust scope only to the extent those checks establish it; retain a Codex project trust entry example only if the verified standard launch needs it.

If the fresh launch reaches readiness without prior trust, omit that prerequisite and example for the verified path. An authentication failure, unavailable model, early process exit, unexplained timeout or source-only inference cannot establish either outcome. Record those cases as incomplete and finish the required Linux matrix before accepting implementation; reproducible Linux operator logs satisfy the same evidence requirements.

### Documentation ownership and focused fixes

Update documentation before source changes, in repository order: `SPEC.md`, user documentation, affected README content, then embedded skills. Preserve SPEC section numbering and keep recommendations in user documentation; SPEC records the observed rule, sandbox and launch contracts.

| Surface | Required reconciliation |
|---|---|
| `SPEC.md` §6.7 backend capabilities, Codex sandbox/configuration and rules | Replace unconditional sandbox/trust assertions with verified behavior; distinguish policy, reviewer and rule execution boundaries |
| `docs/docs/quickstart.md` | Shared installation, Claude recommended profile, minimum verified Codex recipe and backend-specific trust steps |
| `docs/docs/concepts/coding-agents.md` | Own the full configuration, supported alternatives, listener/policy troubleshooting and trust behavior |
| Affected lifecycle pages and CLI references | Align links and procedures while retaining setup selection and targeted repair use cases |
| `assets/skills/cafleet/reference/coding-agents.md` and affected agent references | Include macOS process-inspection failure, escalation/readiness handling and verified runtime prerequisites as self-contained agent instructions |
| `README.md` and repository rules | Update only where changed entry-point claims or instructions otherwise drift |

Keep user pages focused on user-visible configuration and behavior. Skills carry operational facts directly and retain their existing reference boundaries. Preserve supported permission-mode choices and setup selectors. Remove every obsolete prerequisite mention affected by this change, leaving its history in this document and Git.

Source changes are conditional on a reproduced relevant defect, primarily in setup asset installation, backend launch composition or listener startup. Record the triggering case, update its contract/docs, make the smallest correction and add an in-file behavioral unit test. Preserve managed restrictions, standard sandbox boundaries and Director/member role separation. A verification pass with no defect completes this branch with a recorded no-fix-needed result.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Establish an isolated verification baseline

- [x] Record source/installed versions, effective native versions, configuration provenance and test locations in `.verification/`; include Codex 0.167.0, distinguish the Drafter's 0.166.0 observation and assert that the disposable database differs from the active team's database and, when omitting `writable_roots`, lies outside every effective writable location, accounting for automatic temporary-directory grants and recording any verified disabling settings. <!-- completed: 2026-11-16T06:47 -->
- [x] Run shared setup once with the intended backends and configuration roots prepared, pass the same explicit absolute database URL through setup, registration, all state-accessing commands and test member prompts, verify installed assets and the Hermes prerequisite, and preserve the active team's installed runtime. <!-- completed: 2026-11-16T06:45 -->
- [x] Record direct and raw shell-wrapper rule checks, then establish the actual native execution-tool behavior for ordinary CAFleet commands and the listener's thread-variable invocation. <!-- completed: 2026-11-16T06:45 -->

### Step 2: Verify Codex access and receiver startup

- [x] Verify the minimum configuration on Linux, including external database writes, multiplexer access, default member broker exchange, listener `ready`, queued notification delivery and cleanup. <!-- completed: 2026-11-16T07:16 -->
- [x] Exercise user approval, automatic review and the unavailable/refused-escalation outcomes; record effective policy/reviewer and confirm stricter rules remain authoritative. <!-- completed: 2026-11-16T07:16 -->

### Step 3: Establish backend-specific trust requirements

- [x] Run standard member launches in a fresh repository and new worktree on Linux for every backend whose live cases are required by the acceptance scope, recording trust state, startup screen and readiness or the exact blocking consent flow. <!-- completed: 2026-11-16T07:44 -->

### Step 4: Reconcile documentation and necessary implementation

- [x] Update SPEC's affected backend contracts and prerequisites from the verified outcomes while preserving section numbering and contract ownership. <!-- completed: 2026-11-16T05:49 -->
- [x] Rewrite Quickstart shared setup, Claude auto configuration, Codex minimum recipe and backend-specific trust guidance, removing duplicate setup steps and repairing numbering. <!-- completed: 2026-11-16T07:44 -->
- [x] Update the detailed backend configuration and affected lifecycle/CLI references with policy/reviewer distinctions, supported alternatives and listener/trust troubleshooting. <!-- completed: 2026-11-16T05:49 -->
- [x] Align embedded runtime instructions, check README/rules for applicable drift and remove obsolete blanket requirements across living documentation. <!-- completed: 2026-11-16T05:49 -->
- [x] Resolve any reproduced relevant setup/launch/rules defect after documenting its intended behavior, add focused in-file behavioral tests and repeat affected native checks; record when no source fix is needed. Required checks reproduced no CAFleet setup, launch or rules defect, so this documentation implementation required no Rust correction. <!-- completed: 2026-11-16T07:44 -->

### Step 5: Validate and accept

Independent review approved the original full `main...HEAD` diff and scoped verification evidence on 2026-11-16 (Reviewer member 20, message 479). The Reviewer independently passed the documentation build, rendered configuration, anchor and whitespace checks with no substantive findings. The user approved local completion and then authorized publication as [PR #611](https://github.com/himkt/cafleet/pull/611). The original implementation commits are `af9e79b3` and `4b91f6ff`.

- [x] Run `mise //docs:build`, inspect rendered configuration examples and links, and audit consistency of rule execution, sandbox grants, trust and setup instructions. <!-- completed: 2026-11-16T05:49 -->
- [x] If Rust changes are needed, run `mise //:test` and `mise //:lint`; otherwise record that this conditional check does not apply. No Rust source changed in this documentation revision, so these checks do not apply. <!-- completed: 2026-11-16T05:49 -->
- [x] Review the evidence against every success criterion, confirm all required Linux cases are complete and reproduce the final published recipe before accepting implementation. <!-- completed: 2026-11-16T07:46 -->

### Step 6: Reduce the published documentation

- [x] Reduce Quickstart and detailed backend guidance to necessary configuration and user decisions; retain investigation evidence in design artifacts. <!-- completed: 2026-11-16T08:23 -->
- [x] Remove redundant observation history from SPEC and runtime references while preserving operational contracts; verify the documentation build, examples, links and whitespace, and record size changes. Quickstart is 115 lines/545 words versus main's 168/944; the backend reference is 215/4134 versus 197/4145. The build, JSON/TOML parsing and whitespace checks pass. <!-- completed: 2026-11-16T08:23 -->
- [x] Complete independent review of the revised full diff and update the already-authorized PR. The fresh Reviewer approved all seven changed files and independently verified configuration examples, generated internal links and whitespace. Commit `27bf4bf2` and the revised PR description are published. <!-- completed: 2026-11-16T08:28 -->

The user explicitly requested committing and pushing this design document on 2026-11-16. Publish `design-doc.md` as the design record; verification artifacts and agent scratch files remain outside version control.
