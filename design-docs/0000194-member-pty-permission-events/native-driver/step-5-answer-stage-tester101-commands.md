# Answer-stage cases: reviewed commands (Tester101, H5)

Plan: `step-5-answer-stage-measurement-plan.md`, accepted with the Director's §6 decisions (digit `31` allow, digit `34` deny, director-auto decision source, post-stage screens audit-only). Run from /Users/himkt/work/himkt/cafleet, each command as its own invocation, only after Verifier review and explicit runtime dispatch. Every script call names its case with `--case`. Prepare-start runs as a background task (F1). Stage, capture and stop calls run in the foreground.

| Case | Decision / stage | Probe | Case cwd (`CASE`) | Capture output | Database / socket |
|---|---|---|---|---|---|
| `ans-allow-001` | allow / `31` | `permission-probe allow` | `/private/tmp/cafleet-194/ans-allow-001` | `/private/tmp/cafleet-194/ans-allow-001-capture` | `.p194/na01.db` / `.p194/ta01` |
| `ans-deny-001` | deny / `34` | `permission-probe deny` | `/private/tmp/cafleet-194/ans-deny-001` | `/private/tmp/cafleet-194/ans-deny-001-capture` | `.p194/nd01.db` / `.p194/td01` |

Both cases have the 120 s case ceiling plus the 60 s reviewed answer window, and the cleanup reserve stays at 4 s. The driver keeps the case alive after the pre-stage capture until a bound stop (H1). The case record binds the decision, stage and probe argument, and the driver rejects any other binding (H2). It accepts only the requested marker of an allow case: for deny, both markers must stay absent (H4). Each fresh neutral cwd may show Claude's workspace trust screen first. Trusting it is a separate user-authorized action, as in cells-002 attempt 2, and is not the measured stage.

Commands for `ans-allow-001` follow. For `ans-deny-001`, substitute `--case ans-deny-001`, `CASE`, socket `td01`, stage `34` and the manifest name `ans-deny-001.manifest.json`.

## S0 launch (background)

    python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py prepare-start --case ans-allow-001 --libtest /Users/himkt/work/himkt/cafleet/.execution/194/native-target-001/debug/deps/cafleet-6f79be762df3e803 --source-manifest /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-source-manifest-r8.json --environment-evidence /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-environment-evidence.json --redaction /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-redaction.json --capture-policy /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-capture-policy.json --director-backend claude

## S1 pre-stage inspection, privacy review, arm and capture

    python3 -I -c "import json,sys; print(json.load(open(sys.argv[1]))['pane_id'])" /private/tmp/cafleet-194/ans-allow-001/started.json
    tmux -S /Users/himkt/work/himkt/cafleet/.p194/ta01 capture-pane -p -t <runner pane ID>
    python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py arm --case ans-allow-001 --privacy-reviewed
    ls -l /private/tmp/cafleet-194/ans-allow-001-capture/ans-allow-001.manifest.json

Before the stage, confirm on the captured form that `❯ 1. Yes` is the focus line and that the operation is `/private/tmp/cafleet-194/ans-allow-001/permission-probe allow`; for deny it is `…/ans-deny-001/permission-probe deny`.

## S3 decision record (before any byte)

Record the director-auto decision for this case, with its mode/evidence reference, in the Director observations before S4. Zero child bytes are sent before this record.

## S4 the single stage

    tmux -S /Users/himkt/work/himkt/cafleet/.p194/ta01 send-keys -t <runner pane ID> -H 31

Record the exact command line and the UTC time. Send no further keys: a failed digit, with the form still present, is recorded and the case is stopped (plan §2).

## S5 post-stage screens (+200 ms, +1 s, +5 s; read-only, audit-only)

    tmux -S /Users/himkt/work/himkt/cafleet/.p194/ta01 capture-pane -p -e -N -t <runner pane ID>

Run it three times at the stated offsets, plus `ls -l /private/tmp/cafleet-194/ans-allow-001/allow.executed` for the marker. For deny, `deny.executed` must be absent.

## S6 continuation (final capture once Claude settles, ≤30 s)

    tmux -S /Users/himkt/work/himkt/cafleet/.p194/ta01 capture-pane -p -e -N -t <runner pane ID>

## S7 stop and cleanup

    python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py stop --case ans-allow-001

The background prepare-start prints `decision`, `stage_hex`, `requested_marker_present` and `post_stage` (H3). `post_stage` holds the run's event kinds, close reasons and observation times, the output sequence at arm and at stop, and the requested marker's digest. The runner health reply has no manual-input counter, so the one-byte manual-input count is recorded as unavailable. The stage command line and the post-stage screens are the evidence of what was sent. `driver-result.json` and the redacted `runner-pane-capture.json` stay in `CASE`. Copy `/private/tmp/cafleet-194/ans-*` into `.execution/194/native-fixtures/` afterwards, because `/private/tmp` is cleared on reboot.

## How S4's single-byte delivery is evidenced

The runner's health reply exposes no manual-input counter, so `post_stage.manual_input_count` is `unavailable`. A runner-side count of the bytes forwarded to the child is therefore not part of this evidence. The one-byte delivery rests on these separately recorded facts:

| Evidence | What it establishes |
|---|---|
| The exact `send-keys -t <pane> -H 31` (or `34`) command line, its UTC time and exit status 0 | tmux accepted exactly one literal byte for the runner pane. `-H` sends the listed byte unchanged, with no key-name translation or added Enter. |
| The dedicated socket `.p194/ta01` / `td01`, with no attached client and no other `send-keys` in the Director's command log for this case | No other input source reached the pane between arm and stop. |
| The pre-stage capture (`ans-*.complete.json`, the form with `❯ 1. Yes`) and the S5 screens at +200 ms, +1 s and +5 s | The form was live before the byte and closed (or stayed, for a failed candidate) after it. |
| `post_stage.events` (`interaction_closed` with its close reason after the arm) and `output_seq_at_arm` → `output_seq_at_stop` | The runner observed the form close and new child output, both independently of the pane captures. |
| The marker state (`allow.executed` present with digest for allow; both markers absent for deny) and the S6 transcript | The native result the byte produced. |

That the runner forwarded exactly one byte to the child remains an inference from these facts, not a measured count. Recording it directly needs a runner health field, which is a production change outside this harness.

Failure capture and pre-ownership handling are the same as for cells-002 (`step-6-first-native-driver-tester101-commands-cells-002.md` § F8c), with this case's socket and `CASE`.
