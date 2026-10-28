# idle-001 no-answer idle-composer capture: reviewed commands (Tester101)

Plan: `step-5-claude-noninteraction-rule-plan.md` §2, accepted by the Director. Run from /Users/himkt/work/himkt/cafleet, each command as its own invocation, after explicit runtime dispatch. r9 manifest `750ce0739642dc5fbba04a25ee0368729e9038329a9d9c45a487b8a3c711ef48` must be 0600 first.

| Item | Value |
|---|---|
| Case cwd (`CASE`) | `/private/tmp/cafleet-194/idle-001` |
| Capture output | `/private/tmp/cafleet-194/idle-001-capture` |
| Database / socket / session | `.p194/ni01.db` / `.p194/ti01` / `cafleet194ni01` |
| Prompt | `Reply with the single word ready and then wait.` (tool-free; no probe is requested, so both probe markers must stay absent) |
| Mode | no_answer: 120 s ceiling, 4 s cleanup reserve, one arm |

Launch, as a background task:

    python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py prepare-start --case idle-001 --libtest /Users/himkt/work/himkt/cafleet/.execution/194/native-target-001/debug/deps/cafleet-6f79be762df3e803 --source-manifest /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-source-manifest-r9.json --environment-evidence /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-environment-evidence.json --redaction /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-redaction.json --capture-policy /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver-capture-policy.json --director-backend claude

Inspection, read-only:

    python3 -I -c "import json,sys; print(json.load(open(sys.argv[1]))['pane_id'])" /private/tmp/cafleet-194/idle-001/started.json
    tmux -S /Users/himkt/work/himkt/cafleet/.p194/ti01 capture-pane -p -t <runner pane ID>

Sequence:

1. If the workspace trust screen appears, answer it as in ans-allow-001, under the user's authorization for disposable case folders. That is the only key input in this case.
2. Wait until Claude has replied `ready` and settled: no spinner and no `esc to interrupt`, with an empty composer. Confirm the privacy review against the [0,12) mask. Also check that the visible rows carry no home path and no session UUID.
3. Arm once:

       python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py arm --case idle-001 --privacy-reviewed

4. When the manifest appears, stop promptly:

       ls -l /private/tmp/cafleet-194/idle-001-capture/idle-001.manifest.json
       python3 -I /Users/himkt/work/himkt/cafleet/.execution/194/step-6-first-native-driver.py stop --case idle-001

The capture records the real cursor row/column with the cells, and the at-end cursor decision (plan §3, Option A) depends on that value. Record it in the observations together with the composer row text. Copy `/private/tmp/cafleet-194/idle-001*` into `.execution/194/native-fixtures/` afterwards.

Failure capture is as for cells-002 § F8c, with socket `ti01` and this `CASE`.
