# Step 6 shipping-CLI end-to-end acceptance plan (Verifier102)

Assignment: Director99 message 3055. Read-only planning at HEAD `b25d94ed`, which includes:
- the bundled profile `claude-2.1.302-default-2`, with detection, the idle-composer rule `claude-idle-composer` (`verified_noninteraction=true`) and measured answers allow `31` / deny `34`;
- the member-create native version probe `d121603f`.

No command below has been run. Each command runs as its own invocation from the Director's shell, only after explicit runtime dispatch. `<…>` marks a runtime value read from an earlier command's output. Everything else is literal.

## 1. What this run must show

For each multiplexer (a dedicated tmux server, and the existing herdr session), a real wrapped Claude 2.1.302 member created by the shipping `member create --permission-mode default` must show the following, on two separate fresh requests (allow, then deny on another member):

| # | Observation | Evidence command |
|---|---|---|
| O1 | The run records native version `2.1.302`, profile `claude-2.1.302-default-2`, `coverage=tested`, mode `default`/`explicit` | `member show --json` |
| O2 | A blocked native permission form produces one `interaction_opened` event and one Director inbox record while the model is blocked | `message wait`, `event show` |
| O3 | The event payload carries `director_action.kind=review_permission_request` and `inspect_argv` bound to stored IDs | `event show --json` |
| O4 | Inspection returns `answer_capability=tested`, `supported_decisions=["allow","deny"]`, a token and the redacted operation | `permission inspect --json` |
| O5 | A director-auto response is `accepted`/`submitted`, with a durable input row and permission audit | `permission respond --json`, `input show --json` |
| O6 | Native result: `allow.executed` present after allow; both markers absent after deny | `ls` of the case cwd, `member capture` |
| O7 | The episode closes as `interaction_closed` with `close_reason=not_visible` once the idle composer is stable | `message wait`, `event show` |
| O8 | Continuation: Claude reports the probe result (allow) or the refusal (deny) and returns to its idle composer | `member capture` |
| O9 | `member press` delivers only the requested keys (used for the workspace trust form) | `member press --json`, `member capture` |
| O10 | Deletion shuts down the runner with complete cleanup; the protected installed and release binaries are unchanged | `member delete --json`, `member show --json`, `shasum` |

User-sourced responses, a user-executed bound command and manual original-surface answering are separate Step 6 cases (§8). They are not part of this director-auto run.

## 2. Fixed resources

| Item | Value |
|---|---|
| Separate build target | `/Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001` |
| Binary (`BIN`) | `/Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001/release/cafleet` |
| Private config home for the assets guard | `CODEX_HOME=/private/tmp/cafleet-194/e2e-home/codex` (see §3 R1) |
| Redaction file | `/Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-redaction.json` (reviewed `68caf6fb`) |
| Helper | `/Users/himkt/work/himkt/cafleet/.execution/194/native-fixtures/claude-default-tmux/permission-probe` (407 B, `e1a2edc9`) |
| tmux DB / socket / session | `.p194/e2e-tmux.db`, `.p194/te01`, `cafleet194e2e` |
| herdr DB | `.p194/e2e-herdr.db` (the Director's live herdr pane is the private fleet's Director placement) |
| Case cwds (fresh, 0700) | `/private/tmp/cafleet-194/e2e-tmux-allow`, `e2e-tmux-deny`, `e2e-herdr-allow`, `e2e-herdr-deny` |
| Member names | `e2e-tmux-allow`, `e2e-tmux-deny`, `e2e-herdr-allow`, `e2e-herdr-deny` |

`.p194` paths are under `/Users/himkt/work/himkt/cafleet/`. The probe assignment for case `C` is exactly:

```text
Use Bash to attempt exactly /private/tmp/cafleet-194/C/permission-probe ARG as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.
```

`ARG` is `allow` for `*-allow` and `deny` for `*-deny`. This is the wording proven in ans-allow-001/ans-deny-001.

## 3. Risks resolved by construction (verify before relying on them)

| # | Risk | Plan |
|---|---|---|
| R1 | Every `fleet`/`member`/`message` command runs the stale-assets guard, and a fresh private DB has no `asset_installs` row. Running plain `setup` would rewrite the protected `~/.claude` skills. | Run `setup --coding-agent codex` with a private `CODEX_HOME`, so the only recorded row is at that private path. Pass the same `CODEX_HOME` to every later `BIN` call. Claude ignores `CODEX_HOME`, and only `CAFLEET_DATABASE_URL` is forwarded into member panes. `~/.claude` gets no row in this DB, so it contributes nothing to the guard. |
| R2 | The Director shell carries `HERDR_ENV` and `TMUX`-less herdr context, so auto-detection would pick herdr for the tmux variant. | Prefix every tmux-variant call with `CAFLEET_MULTIPLEXER=tmux TMUX=<socket>,<server pid>,0 TMUX_PANE=%0`. The tmux adapter uses `TMUX`/`TMUX_PANE` (tmux.rs:96, 143), and the tmux client selects the server from the socket in `TMUX`. Prefix herdr-variant calls with `CAFLEET_MULTIPLEXER=herdr`. |
| R3 | A dedicated tmux server seeded from the Director shell would pass `CLAUDECODE`, `CLAUDE_CODE_MESSAGING_TOKEN` and similar variables to the member (the cells-001 F7 lesson). | Start the server through `env -u` for every matching name, then verify that `show-environment -g` contains none (§4 T1-T3). For herdr, the member pane is spawned by the user's herdr server: verify names only with `ps -E` after creation (§6 H2). |
| R4 | `member create` uses the invoking process cwd as the member cwd (cli/member.rs:433). | `cd` into the fresh case cwd as its own Bash call before each `member create`, then `cd` back to the repository. |
| R5 | A fresh cwd shows Claude's workspace trust form before the assignment. | Answer it with `member press` (O9): `Down`, check the screen with `member capture`, then `Enter`. This is a user-authorized action (§7 U1). |
| R6 | The bundled detection fixtures are 120x36. Production panes take the split size. | Record each member pane's geometry. Size the tmux window 240x80 so the member pane is wider than 120 columns. A detection miss at another geometry is an acceptance finding: record it, answer nothing, stop that member. |
| R7 | The member's `cafleet` on PATH is the protected installed binary, which fails the guard against the private DB. | The probe assignment needs no `cafleet message`, and the member's `--allowedTools Bash(cafleet message *)` allowance stays unused. |

If any verification step contradicts these assumptions (for example, the runner's internal command refuses on the assets guard without `CODEX_HOME`, or a removed variable reaches the member), stop and route it as a finding. Do not work around it.

## 4. Phase P: build, identity and private setup

```text
P1  env CARGO_TARGET_DIR=/Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001 mise //cafleet:build
P2  shasum -a 256 /Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001/release/cafleet
P3  command -v cafleet
P4  shasum -a 256 <P3 path>                                      (expect 5d6c1c703fb69c74af502c6a2ca652a3e18828a24d14c8206e488ad1eb4f9743)
P5  shasum -a 256 /Users/himkt/work/himkt/cafleet/cafleet/target/release/cafleet   (expect 49daf67797978ef7970b7883d451b3230a7220aace8642c27e41e059185ca859)
P6  /Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001/release/cafleet --version
P7  /Users/himkt/.local/share/mise/installs/aqua-anthropics-claude-code/v2.1.302/claude --version          (expect 2.1.302 (Claude Code))
P8  mkdir -p -m 0700 /private/tmp/cafleet-194/e2e-home/codex
P9  CAFLEET_DATABASE_URL=sqlite:////Users/himkt/work/himkt/cafleet/.p194/e2e-tmux.db CODEX_HOME=/private/tmp/cafleet-194/e2e-home/codex /Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001/release/cafleet setup --coding-agent codex
P10 CAFLEET_DATABASE_URL=sqlite:////Users/himkt/work/himkt/cafleet/.p194/e2e-herdr.db CODEX_HOME=/private/tmp/cafleet-194/e2e-home/codex /Users/himkt/work/himkt/cafleet/.execution/194/e2e-target-001/release/cafleet setup --coding-agent codex
P11 chmod 600 /Users/himkt/work/himkt/cafleet/.p194/e2e-tmux.db /Users/himkt/work/himkt/cafleet/.p194/e2e-herdr.db
```

P9/P10 must print `Created … applied migrations to head` and a single `codex: installed … -> /private/tmp/cafleet-194/e2e-home/codex/skills` line, plus its preset line. Any write under `~/.claude` or `~/.codex` is a stop condition. Record P2 as the shipping-binary identity for every later observation. The task registry's `build` task is its only binary-producing task; the invocation-local `CARGO_TARGET_DIR` keeps the protected `cafleet/target/release` binary untouched.

Per case, create the cwd and helper (repeat with each case name):

```text
C1  mkdir -m 0700 /private/tmp/cafleet-194/e2e-tmux-allow
C2  install -m 0700 /Users/himkt/work/himkt/cafleet/.execution/194/native-fixtures/claude-default-tmux/permission-probe /private/tmp/cafleet-194/e2e-tmux-allow/permission-probe
C3  shasum -a 256 /private/tmp/cafleet-194/e2e-tmux-allow/permission-probe     (expect e1a2edc980a137060f888e36ee7780be076ab24bae487e368e8b35b085660dc4)
```

## 5. Phase T: tmux (dedicated socket)

Server and stand-in Director pane (the Director's decisions still come from this session through the CLI):

```text
T0  env | cut -d= -f1 | grep -E '^(CLAUDECODE|CLAUDE_EFFORT|CLAUDE_PID|CLAUDE_CODE_|HERDR_)'
T1  env -u <each T0 name> tmux -S /Users/himkt/work/himkt/cafleet/.p194/te01 -f /dev/null new-session -d -s cafleet194e2e -x 240 -y 80 -c /private/tmp/cafleet-194 /bin/sleep 3600
T2  tmux -S /Users/himkt/work/himkt/cafleet/.p194/te01 show-environment -g | cut -d= -f1 | grep -E '^(CLAUDECODE|CLAUDE_EFFORT|CLAUDE_PID|CLAUDE_CODE_|HERDR_)'    (expect no output)
T3  tmux -S /Users/himkt/work/himkt/cafleet/.p194/te01 display-message -p -t cafleet194e2e '#{pid}|#{session_name}|#{window_id}|#{pane_id}'
```

Below, `ENV_T` stands for this literal prefix, with `<pid>` taken from T3:

```text
CAFLEET_MULTIPLEXER=tmux TMUX=/Users/himkt/work/himkt/cafleet/.p194/te01,<pid>,0 TMUX_PANE=%0 CAFLEET_DATABASE_URL=sqlite:////Users/himkt/work/himkt/cafleet/.p194/e2e-tmux.db CODEX_HOME=/private/tmp/cafleet-194/e2e-home/codex
```

Write it out in full on every call, because host allow rules match literal strings.

```text
T4  ENV_T BIN doctor --json                                    (multiplexer tmux, database at head, codex assets current)
T5  ENV_T BIN fleet create --name e2e-tmux --coding-agent claude --json        → <FT>, Director <DT>
T6  cd /private/tmp/cafleet-194/e2e-tmux-allow
T7  ENV_T BIN member create --fleet-id <FT> --name e2e-tmux-allow --description "E2E allow probe" --coding-agent claude --permission-mode default --event-evidence redacted --event-redaction-file /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-redaction.json --json "Use Bash to attempt exactly /private/tmp/cafleet-194/e2e-tmux-allow/permission-probe allow as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation."
T8  cd /Users/himkt/work/himkt/cafleet
```

T7 must return `runtime` with `resolved_mode=default`, `coverage=tested` and a run ID. A stderr line `Warning: native version unavailable …` fails O1: stop and record it.

The per-member sequence is S1-S10 below, with `M` the new member ID, `D`=`<DT>` and prefix `ENV_T`.

## 6. Phase H: herdr (existing session)

`ENV_H` stands for this literal prefix:

```text
CAFLEET_MULTIPLEXER=herdr CAFLEET_DATABASE_URL=sqlite:////Users/himkt/work/himkt/cafleet/.p194/e2e-herdr.db CODEX_HOME=/private/tmp/cafleet-194/e2e-home/codex
```

```text
H0  ENV_H BIN doctor --json
H1  ENV_H BIN fleet create --name e2e-herdr --coding-agent claude --json       → <FH>, Director <DH>
    (makes the Director's own herdr pane the private fleet's Director placement; fleet 25 in the default database is unaffected)
H2a cd /private/tmp/cafleet-194/e2e-herdr-allow
H2b ENV_H BIN member create --fleet-id <FH> --name e2e-herdr-allow … (otherwise identical to T7, with the e2e-herdr-allow path in the assignment)
H2c cd /Users/himkt/work/himkt/cafleet
H3  ps -E -o command= -p <runtime.child_pid> | tr ' ' '\n' | grep -c -E '^(CLAUDECODE|CLAUDE_CODE_MESSAGING_TOKEN|CLAUDE_CODE_SESSION_ID)='
```

H3 is a names-only count; it never prints values. A nonzero count means a Director-session credential or marker reached the member: record it as a security finding before continuing. Run the same check after T7 for the tmux member.

The new herdr panes appear in the user's own workspace, so the user should avoid typing into them during the run (§7 U2).

## 7. Per-member sequence S1-S10 (both multiplexers, allow member then deny member)

| Step | Command | Required observation |
|---|---|---|
| S1 | `PREFIX BIN member show M --json` | O1: `native_version` 2.1.302 (in run evidence), `coverage=tested`, `resolved_mode=default`, run/owner/child IDs present. Record the pane ID. |
| S2 | `PREFIX BIN member capture M --lines 40` | The workspace trust form for the case path. Record the geometry with `tmux -S … display-message -p -t <pane> '#{pane_width}x#{pane_height}'`, or the herdr equivalent. |
| S3 | `PREFIX BIN member press M Down --json`, then `member capture`, then `PREFIX BIN member press M Enter --json` | O9: each returns `status=acknowledged` with exactly the one canonical key. The capture between them shows focus on `Yes, I trust this folder`. Requires U1. |
| S4 | `PREFIX BIN message wait D --timeout 120 --json` | O2: one record from `M` whose text is the v2 event JSON with `kind=interaction_opened` and `classification=permission_request`. Record `event_id`, `interaction_id`, `message_id`, `observed_at`, `committed_at`. `member capture` shows the form with `❯ 1. Yes`. |
| S5 | `PREFIX BIN event show <E> --json`, then `PREFIX BIN message ack <MSG>` | O3: `notification_state=inbox`, `message_id` equals S4's, `director_action.kind=review_permission_request`, and `inspect_argv` equals the IDs. ACK records consumption. |
| S6 | `PREFIX BIN permission inspect <I> --event-id <E> --from-member-id D --json` | O4: `answer_capability=tested`, `supported_decisions` `["allow","deny"]`, `answer_profile_version` `claude-2.1.302-default-2`, a 64-hex `request_token`, the redacted operation equal to the case probe command, and `missing_evidence` listed. |
| S7 | Write the decision record `/private/tmp/cafleet-194/C/decision.md`: decision, director-auto source, mode evidence (host auto mode, same reference as ans-allow-001), risk assessment of the disposable probe, and a statement that zero bytes have been sent before it | Exists before S8. |
| S8 | `PREFIX BIN permission respond <I> --event-id <E> --from-member-id D --expected-run-id <R> --expected-owner-epoch <EP> --request-token <TOK> --decision allow --decision-source director-auto --director-mode auto --mode-evidence-ref /private/tmp/cafleet-194/C/decision.md#mode --decision-evidence-ref /private/tmp/cafleet-194/C/decision.md#decision --operation-key e2e-<mux>-allow-001 --json` (the deny member uses `--decision deny` and `…-deny-001`) | O5: `status=accepted`, `stage=submitted`, `input_id` non-null, exit 0. Take `R`/`EP`/`TOK` from S6 output only. |
| S9 | `PREFIX BIN input show <INPUT> --json`; `ls -l /private/tmp/cafleet-194/C/`; `PREFIX BIN member capture M --lines 40` | O5 audit: `kind=permission_response`, `permission_response.decision_source=director-auto`, `director_mode=auto`, `answer_profile_version` and the rule recorded. O6: allow → `allow.executed` present and `deny.executed` absent; deny → both absent. O8: Claude's report and the idle composer. |
| S10 | `PREFIX BIN message wait D --timeout 60 --json`, then `event show` and `message ack` for each record | O7: an `interaction_closed` event for `<I>` with `close_reason=not_visible`, and no new `interaction_opened` for the same operation. `interaction_obscured` or `superseded` instead of `not_visible` fails O7: record it. |

After S10 for both members of a multiplexer:

```text
X1  PREFIX BIN member delete M --json          (each member)
X2  PREFIX BIN member show M --json            (deregistered; runtime status exited, cleanup_state complete)
X3  PREFIX BIN fleet delete <F> --json
X4  tmux -S /Users/himkt/work/himkt/cafleet/.p194/te01 kill-server    (tmux variant only, after X1-X3)
```

`member delete` shuts down the native session, which may print a `claude --resume` hint. Native termination is session termination, not a deny. O10 then reruns P2, P4 and P5.

Evidence copy: `cp -Rp /private/tmp/cafleet-194/e2e-* /Users/himkt/work/himkt/cafleet/.execution/194/native-fixtures/` before any reboot. Record all JSON outputs from S1-S10 and X1-X3 in `.execution/194/step-6-e2e-<mux>-observations.md`.

## 8. User actions

| # | Step | Action | Why it is the user's |
|---|---|---|---|
| U1 | S3 for each of the four fresh case folders | Authorize trusting `/private/tmp/cafleet-194/e2e-*`. The existing 2026-10-28 "trust the disposable case folders" authorization covers this only if the Director judges that it extends to these new names; otherwise ask once for all four. | Trusting a folder is a persistent change to the user's native configuration. |
| U2 | Phase H | Avoid typing into the new herdr panes until X1. | Human keystrokes are native input and would change the measured state. |
| — | S6-S8 | None: director-auto, under the Director's recorded auto mode and the user's direction to complete the remaining work. | Already recorded for ans-allow-001/ans-deny-001. |

Separate Step 6 cases that require the user, not run here:

| Case | User action |
|---|---|
| User-sourced response | The user decides allow or deny for a fresh request; the Director runs S8 with `--decision-source user --director-mode <actual>` and records the user's words as `--decision-evidence-ref`. |
| User-executed bound command | The Director presents the complete S8 command with the S6 snapshot values; the user runs it themselves. Record zero response bytes before the user acts. |
| Manual original-surface answer | The user presses the native choice in the member pane. Record the result as attributed manual input; `permission inspect` must afterwards show the token revoked. |

## 9. Stop conditions

Stop the affected member and record without retrying if any of these occur:
- the native version probe warning;
- `coverage=unknown`;
- no event within S4's timeout;
- inspect `answer_capability=unknown`;
- respond `held`, `rejected` or `uncertain`;
- a marker contrary to the decision;
- a nonzero H3 count;
- any write to protected paths.

An uncertain response goes to `input show` and Director reconciliation (`input reconcile`) before any new attempt with a new operation key.
