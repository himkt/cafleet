package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"github.com/ncruces/go-sqlite3"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

const observationOpened = "2026-11-21T12:04:08.000000+00:00"

func healthyObservation() observationHealth {
	return observationHealth{Visibility: visibilityNotVisible, Eligibility: inputEligibilityComposerEmpty{}}
}

func degradedObservation() observationHealth {
	return observationHealth{
		Visibility:       visibilityUnknown,
		Eligibility:      inputEligibilityHold{reason: gateReasonGap},
		PendingCount:     2,
		GapCount:         3,
		Degraded:         true,
		Diagnostic:       some("sink_unavailable"),
		WriterDiagnostic: some("database error: database is locked"),
		GapOpenSince:     some(observationOpened),
	}
}

func registryRuntime() jsonObject {
	return jsonObject{
		{"run_id", 37}, {"mode", "auto"}, {"status", "running"}, {"coverage", "tested"},
		{"child_pid", 4242}, {"child_start_identity", "child-start"},
	}
}

func showDiagnostics() []runnerDiagnosticEntry {
	return []runnerDiagnosticEntry{
		{source: runnerDiagnosticSourceEventWriter, message: "event writer diagnostic: database is locked", firstAt: "2026-11-21T12:04:00.000000+00:00", lastAt: "2026-11-21T12:05:00.000000+00:00", count: 4},
		{source: runnerDiagnosticSourceObservation, message: "observation gap opened: sink_unavailable", firstAt: observationOpened, lastAt: observationOpened, count: 1},
	}
}

type showCase struct {
	name        string
	probe       observationProbe
	probeName   string
	observation jsontext.Value
	err         jsontext.Value
	suffix      string
}

var showCases = []showCase{
	{"skipped", observationProbeSkipped{}, "skipped", jsontext.Value("null"), jsontext.Value("null"), " observation=-"},
	{"failed", observationProbeFailed{message: "cannot connect to the runner"}, "failed", jsontext.Value("null"), jsontext.Value(`"cannot connect to the runner"`), " observation=unavailable"},
	{"answered before the child started", observationProbeAnswered{health: none[observationHealth]()}, "answered", jsontext.Value("null"), jsontext.Value("null"), " observation=pending"},
	{"answered healthy", observationProbeAnswered{health: some(healthyObservation())}, "answered",
		jsontext.Value(`{"visibility":"not_visible","eligibility":"composer_empty","degraded":false,"diagnostic":null,"writer_diagnostic":null,"pending_count":0,"gap_count":0,"gap_open_since":null}`),
		jsontext.Value("null"), " observation=composer_empty"},
	{"answered degraded", observationProbeAnswered{health: some(degradedObservation())}, "answered",
		jsontext.Value(`{"visibility":"unknown","eligibility":"hold:gap","degraded":true,"diagnostic":"sink_unavailable","writer_diagnostic":"database error: database is locked","pending_count":2,"gap_count":3,"gap_open_since":"` + observationOpened + `"}`),
		jsontext.Value("null"), " observation=hold:gap degraded"},
}

func TestCliMember_ObservationSurface_MemberShowRuntimeAppendsTheProbeKeysForEachOutcome(t *testing.T) {
	t.Parallel()
	registry := mustJSON(t, registryRuntime())
	registryKeys := testJSONKeys(t, registry)
	for _, tc := range showCases {
		runtime := mustJSON(t, showRuntime(registryRuntime(), tc.probe, showDiagnostics()))
		if keys := testJSONKeys(t, runtime); !slices.Equal(keys, append(slices.Clone(registryKeys), "observation_probe", "observation", "observation_error", "diagnostics")) {
			t.Fatalf("%s: keys %q", tc.name, keys)
		}
		for _, key := range registryKeys {
			assertJSONEqual(t, jsonAt(t, runtime, key), jsonAt(t, registry, key), tc.name+": "+key)
		}
		assertJSONEqual(t, jsonAt(t, runtime, "observation_probe"), mustJSON(t, tc.probeName), tc.name)
		observation := jsonAt(t, runtime, "observation")
		assertJSONEqual(t, observation, tc.observation, tc.name)
		if string(tc.observation) != "null" {
			if keys := testJSONKeys(t, observation); !slices.Equal(keys, []string{"visibility", "eligibility", "degraded", "diagnostic", "writer_diagnostic", "pending_count", "gap_count", "gap_open_since"}) {
				t.Fatalf("%s: observation keys %q", tc.name, keys)
			}
		}
		assertJSONEqual(t, jsonAt(t, runtime, "observation_error"), tc.err, tc.name)
		diagnostics := jsonAt(t, runtime, "diagnostics")
		assertJSONEqual(t, diagnostics, jsontext.Value(`[
			{"source":"event_writer","message":"event writer diagnostic: database is locked","first_at":"2026-11-21T12:04:00.000000+00:00","last_at":"2026-11-21T12:05:00.000000+00:00","count":4},
			{"source":"observation","message":"observation gap opened: sink_unavailable","first_at":"`+observationOpened+`","last_at":"`+observationOpened+`","count":1}]`), tc.name)
		for _, row := range decodedArray(t, diagnostics) {
			if keys := testJSONKeys(t, row); !slices.Equal(keys, []string{"source", "message", "first_at", "last_at", "count"}) {
				t.Fatalf("%s: diagnostic keys %q", tc.name, keys)
			}
		}
	}
}

func TestCliMember_ObservationSurface_MemberShowTextSuffixNamesTheObservationStateAndDegradation(t *testing.T) {
	t.Parallel()
	for _, tc := range showCases {
		if got := observationSuffix(tc.probe); got != tc.suffix {
			t.Fatalf("%s: suffix %q, want %q", tc.name, got, tc.suffix)
		}
	}
}

func TestCliMember_ObservationSurface_MemberShowRuntimeWithoutDiagnosticsReportsAnEmptyList(t *testing.T) {
	t.Parallel()
	runtime := mustJSON(t, showRuntime(registryRuntime(), observationProbeSkipped{}, nil))
	assertJSONEqual(t, jsonAt(t, runtime, "diagnostics"), jsontext.Value("[]"), "diagnostics")
}

func TestCliMember_ObservationSurface_ProbeObservationSkipsUnprobedStatusesAndFailsWithoutARunner(t *testing.T) {
	t.Parallel()
	cases := []struct {
		status string
		probed bool
	}{
		{"starting", false}, {"prepared", false}, {"recovering", false}, {"exited", false}, {"failed", false},
		{"running", true}, {"degraded", true},
	}
	for _, tc := range cases {
		fixture := newBrokerRuntimeFixture(t)
		if _, err := fixture.conn.execute("UPDATE member_runs SET status=?1 WHERE run_id=?2", tc.status, fixture.run.runID); err != nil {
			t.Fatal(err)
		}
		run := mustRun(t, fixture.conn, fixture.run.runID)
		if want, err := runStatusTryFrom(tc.status); err != nil || run.status != want {
			t.Fatalf("%s: status %v %v", tc.status, run.status, err)
		}
		started := time.Now()
		probe := probeObservation(run, systemClock{})
		if elapsed := time.Since(started); elapsed >= 2*time.Second {
			t.Fatalf("%s: the probe is bounded by its 1 s deadline: %v", tc.status, elapsed)
		}
		switch probe := probe.(type) {
		case observationProbeSkipped:
			if tc.probed {
				t.Fatalf("%s: skipped", tc.status)
			}
		case observationProbeFailed:
			if !tc.probed || strings.TrimSpace(probe.message) == "" {
				t.Fatalf("%s: failed %q", tc.status, probe.message)
			}
		case observationProbeAnswered:
			t.Fatalf("%s: no runner answers in this fixture", tc.status)
		default:
			panic("invalid observationProbe")
		}
	}
}

const memberCloseSocket = "/tmp/tmux-1000/default"

// closeRunner counts and checks the member's pane close; the window restoration that follows it
// lists a single-column window, which needs no equalization.
type closeRunner struct {
	t       *testing.T
	check   func()
	calls   int
	failure opt[string]
}

func (r *closeRunner) binaryExists(name string) bool {
	if name != "tmux" {
		r.t.Fatalf("binary lookup %q", name)
	}
	return true
}

func (r *closeRunner) runFor(argv []string, _ time.Duration) (string, *runError) {
	return r.run(argv)
}

func (r *closeRunner) run(argv []string) (string, *runError) {
	restoration := []string{"tmux", "-S", memberCloseSocket, "list-panes", "-t", "@1", "-F", "#{pane_id}|#{pane_left}"}
	if slices.Equal(argv, restoration) {
		return "%0|0\n", nil
	}
	if want := []string{"tmux", "-S", memberCloseSocket, "kill-pane", "-t", "%9"}; !slices.Equal(argv, want) {
		r.t.Fatalf("process call %q, want %q", argv, want)
	}
	r.calls++
	r.check()
	if cause, failing := r.failure.get(); failing {
		return "", failedRun(cause)
	}
	return "", nil
}

func newCloseRunner(t *testing.T, check func()) *closeRunner {
	return &closeRunner{t: t, check: check, failure: none[string]()}
}

func closeMux(runner *closeRunner, d deadline) multiplexer {
	return tmuxMultiplexerForServer(runner, memberCloseSocket).bounded(d)
}

type memberFixture struct {
	conn                    *dbConn
	url                     string
	settings                settings
	fleet, director, worker int64
}

func newMemberFixture(t *testing.T) memberFixture {
	t.Helper()
	conn, url := initializedFileConn(t)
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	fleet, director := createFleetFixture(t, conn, "deletion")
	worker := register(t, conn, fleet, "worker", none[string]())
	return memberFixture{conn: conn, url: url, settings: settingsForDatabase(path), fleet: fleet, director: director, worker: worker}
}

// shortFileConn is an initialized database under a short directory, for runs whose runtime
// sockets live beside it.
func shortFileConn(t *testing.T) (*dbConn, string, settings) {
	t.Helper()
	dir := shortTempdir(t)
	conn, _ := initializedConnAt(t, filepath.Join(dir, "broker.db"))
	return conn, dir, settingsForDatabase(filepath.Join(dir, "broker.db"))
}

func memberRows(t *testing.T, conn *dbConn) int64 {
	t.Helper()
	return queryInt64(t, conn, "SELECT COUNT(*) FROM members")
}

func ownedRunSpec(t *testing.T, conn *dbConn, url string, fleet, director, worker int64, backend, mode string, argv jsontext.Value) newRun {
	t.Helper()
	identity, err := databaseIdentity(context.Background(), conn)
	if err != nil {
		t.Fatal(err)
	}
	paths, err := runtimePathsForRun(identity, 1)
	if err != nil {
		t.Fatal(err)
	}
	process := currentProcess(t)
	spec := brokerRuntimeFixtureSpec(t, conn, url, fleet, director, worker)
	spec.backend = backend
	spec.runtimePath = paths.runDir
	spec.resolvedMode = mode
	spec.permissionArgvJSON = argv
	spec.nativeVersion = none[string]()
	spec.coverage = coverageUnknown
	spec.ownerPID = process.pid
	spec.ownerStartIdentity = process.startIdentity
	return spec
}

func TestCliMember_RecoveryDeletion_HermesMemberDeletionWaitsForBusyPaneInputOnlyWithinTheCommandDeadline(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _, s := shortFileConn(t)
	fleet, director := createFleetFixture(t, conn, "hermes-busy")
	worker := register(t, conn, fleet, "worker", some("%9"))
	if _, err := createRun(ctx, conn, ownedRunSpec(t, conn, s.databaseURL, fleet, director, worker, "hermes", "smart", jsontext.Value(`[]`))); err != nil {
		t.Fatal(err)
	}
	member, err := activeMember(ctx, conn, worker)
	if err != nil {
		t.Fatal(err)
	}
	busy, acquired := tryPaneLock(t, s, member.placement)
	if !acquired {
		t.Fatal("the test holds the pane input")
	}
	defer busy.close()
	started := time.Now()
	err = cliMemberDelete(ctx, conn, s, worker, false, deadlineUntil(started.Add(300*time.Millisecond)))
	if elapsed := time.Since(started); elapsed >= 2*time.Second {
		t.Fatalf("deletion waited %v", elapsed)
	}
	if want := fmt.Sprintf("member %d's pane input remained busy for 5 seconds; member was not deleted.", worker); err == nil || errorMessage(err) != want {
		t.Fatalf("error %v, want %q", err, want)
	}
	if fleetOf, err := activeMemberFleet(ctx, conn, worker); err != nil || !fleetOf.set {
		t.Fatalf("the member stays active: %v", err)
	}
}

const memberRecoveryCaseEnv = "CAFLEET_TEST_MEMBER_RECOVERY_CASE"

func TestCliMember_RecoveryDeletion_RunnerRecoveryMemberDeletionFinishesTerminalWorkAndPreservesAForeignEpoch(t *testing.T) {
	t.Parallel()
	for _, scenario := range []string{"terminal", "epoch"} {
		t.Run(scenario, func(t *testing.T) {
			t.Parallel()
			command := childTestCommand(TestCliMember_RecoveryDeletion_RunnerRecoveryMemberDeletionChild)
			command.Env = append(command.Env, memberRecoveryCaseEnv+"="+scenario)
			var output bytes.Buffer
			command.Stdout, command.Stderr = &output, &output
			if err := command.Start(); err != nil {
				t.Fatal(err)
			}
			finished := make(chan error, 1)
			go func() { finished <- command.Wait() }()
			select {
			case err := <-finished:
				if err != nil {
					t.Fatalf("%s: %v\n%s", scenario, err, output.String())
				}
			case <-time.After(5 * time.Second):
				if err := command.Process.Kill(); err != nil {
					t.Error(err)
				}
				if waitErr := <-finished; waitErr == nil {
					t.Error("the killed child reports its signal")
				}
				t.Fatalf("%s: the child did not finish\n%s", scenario, output.String())
			}
		})
	}
}

func TestCliMember_RecoveryDeletion_RunnerRecoveryMemberDeletionChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	scenario := os.Getenv(memberRecoveryCaseEnv)
	if scenario != "terminal" && scenario != "epoch" {
		t.Fatalf("scenario %q", scenario)
	}
	ctx := context.Background()
	conn, _, s := shortFileConn(t)
	fleet, director := createFleetFixture(t, conn, "member-recovery")
	worker := register(t, conn, fleet, "worker", some("%9"))
	run, err := createRun(ctx, conn, ownedRunSpec(t, conn, s.databaseURL, fleet, director, worker, "claude", "default", jsontext.Value(`["--permission-mode","default"]`)))
	if err != nil {
		t.Fatal(err)
	}
	artifacts, err := createRunArtifacts(ctx, conn, run.runID)
	if err != nil {
		t.Fatal(err)
	}
	defer artifacts.close()
	run = mustRun(t, conn, run.runID)
	runtimeLock := acquireReleasableRunLock(t, conn, run.owner())
	owner, err := runtimeLock.transferToRunner(ctx, conn, run.owner(), currentProcess(t))
	if err != nil {
		t.Fatal(err)
	}
	mustExecute(t, conn, "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T20:00:00Z' WHERE run_id=?1", run.runID)
	if err := runtimeLock.close(); err != nil {
		t.Fatal(err)
	}
	d := deadlineUntil(time.Now().Add(2 * time.Second))
	locked, err := lockMemberDeletion(ctx, conn, s, worker, d)
	if err != nil {
		t.Fatal(err)
	}
	defer locked.close()
	p := locked.snapshot.placement
	runner := newCloseRunner(t, func() {
		if lock, free := tryPaneLock(t, s, p); free {
			lock.close()
			t.Fatal("the deletion holds the pane lock through the close")
		}
		check := fileConn(t, s.databaseURL)
		if current := mustRun(t, check, run.runID); current.ownerEpoch != owner.ownerEpoch {
			t.Fatal("a terminal no-work observation advances no owner")
		}
		if events := queryInt64(t, check, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1", run.runID); events != 0 {
			t.Fatalf("%d events", events)
		}
		if scenario == "epoch" {
			deletionBumpEpoch(t, check, run.runID)
		}
	})
	paneStatus, err := closeMemberWithin(ctx, conn, s, worker, locked, closeMux(runner, d), d, layoutLockWait)
	if runner.calls != 1 {
		t.Fatalf("%d pane closes", runner.calls)
	}
	if scenario == "terminal" {
		if err != nil || paneStatus != "%9 (killed)" {
			t.Fatalf("pane status %q: %v", paneStatus, err)
		}
		if found, _ := getMember(ctx, conn, worker, fleet); found.set {
			t.Fatal("the member is deleted")
		}
	} else {
		if err == nil {
			t.Fatal("a foreign epoch change preserves registration after the fake pane close")
		}
		if !strings.Contains(errorMessage(err), "closed") {
			t.Fatalf("error %v", err)
		}
		if found, _ := getMember(ctx, conn, worker, fleet); !found.set {
			t.Fatal("the member stays registered")
		}
	}
	if found, _ := getMember(ctx, conn, director, fleet); !found.set {
		t.Fatal("the Director stays registered")
	}
}

func hermesMemberCreateArgs(fleet int64, description string) []string {
	return []string{"member", "create", "--fleet-id", fmt.Sprint(fleet), "--name", "worker", "--description", description,
		"--coding-agent", "hermes", "literal assignment on {database_url}"}
}

func TestCliMember_HermesMemberCreateFailsBeforeRegistrationWhenTheHermesHomeCannotBeResolved(t *testing.T) {
	t.Parallel()
	cases := []struct{ configPath, expected string }{
		{"printf 'config.yaml\\n'", "Error: cannot read hermes configuration through 'hermes config path': printed no absolute file path: config.yaml\n"},
		{"echo 'no profile' >&2\n    exit 4", "Error: cannot read hermes configuration through 'hermes config path': no profile\n"},
	}
	for _, tc := range cases {
		conn, dir, s := shortFileConn(t)
		fleet, _ := createFleetFixture(t, conn, "hermes-home")
		before := memberRows(t, conn)
		hermes := "case \"$*\" in\n  --version) printf 'Hermes Agent vunknown (2026.9.24)\\n' ;;\n  'config path')\n    " + tc.configPath + " ;;\n  *) exit 3 ;;\nesac"
		tmux := "printf '%s\\n' \"$*\" >> \"$CAFLEET_TEST_PANE_LOG\"\ncase \" $* \" in *' #{window_height} '*) printf '120\\n' ;; *' display-message '*) printf 'main|@1|%%1\\n' ;; esac"
		output, paneCalls := isolatedCliWith(t, s, hermesMemberCreateArgs(fleet, "hermes home fixture"),
			[][2]string{{"tmux", tmux}, {"hermes", hermes}}, [][2]string{{"TMUX_PANE", "%1"}, {"HERMES_HOME", dir}})
		if output.stderr != tc.expected || output.code != 1 {
			t.Fatalf("%s: exit %d stderr %q", tc.configPath, output.code, output.stderr)
		}
		if strings.Contains(paneCalls, "split-window") || memberRows(t, conn) != before {
			t.Fatalf("%s: creation spawned or registered: %s", tc.configPath, paneCalls)
		}
	}
}

func TestCliMember_HermesMemberCreateBoundsConfigPathByFiveSecondsAndTheVersionProbeByTwo(t *testing.T) {
	t.Parallel()
	const versionLine = "printf 'Hermes Agent vunknown (2026.9.24)\\n'"
	const homeConfig = "printf '%s/config.yaml\\n' \"$HERMES_HOME\""
	cases := []struct {
		name, version, configPath string
		refused                   opt[string]
		warning                   opt[string]
	}{
		{"both probes answer at once", versionLine, homeConfig, none[string](), none[string]()},
		{"config path answers after three seconds", versionLine, "/bin/sleep 3\n    printf '%s/config.yaml\\n' \"$HERMES_HOME\"", none[string](), none[string]()},
		{"config path runs past five seconds", versionLine, "exec /bin/sleep 8",
			some("Error: cannot read hermes configuration through 'hermes config path': did not finish within its deadline\n"), none[string]()},
		{"version probe runs past two seconds", "exec /bin/sleep 8", homeConfig, none[string](),
			some("Warning: native version unavailable for hermes: hermes --version did not finish within its deadline.\n")},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			conn, dir, s := shortFileConn(t)
			fleet, _ := createFleetFixture(t, conn, "hermes-probe-bounds")
			before := memberRows(t, conn)
			hermes := "case \"$*\" in\n  --version)\n    " + tc.version + " ;;\n  'config path')\n    " + tc.configPath + " ;;\n  *) exit 3 ;;\nesac"
			tmux := "printf '%s\\n' \"$*\" >> \"$CAFLEET_TEST_PANE_LOG\"\ncase \" $* \" in *' #{window_height} '*) printf '120\\n' ;; *' display-message '*) printf 'main|@1|%%1\\n' ;; *' split-window '*) exit 1 ;; esac"
			output, paneCalls := isolatedCliWith(t, s, hermesMemberCreateArgs(fleet, "hermes probe bound fixture"),
				[][2]string{{"tmux", tmux}, {"hermes", hermes}}, [][2]string{{"TMUX_PANE", "%1"}, {"HERMES_HOME", dir}})
			if output.code != 1 {
				t.Fatalf("exit %d: %s", output.code, output.stderr)
			}
			if expected, refused := tc.refused.get(); refused {
				if output.stderr != expected || strings.Contains(paneCalls, "split-window") || memberRows(t, conn) != before {
					t.Fatalf("stderr %q calls %s", output.stderr, paneCalls)
				}
				return
			}
			if strings.Contains(output.stderr, "hermes configuration") {
				t.Fatalf("the config path probe finished within its bound: %s", output.stderr)
			}
			if !strings.Contains(paneCalls, "split-window") || !strings.Contains(output.stderr, "runner pane spawn failed") {
				t.Fatalf("creation reached the pane spawn: %s / %s", paneCalls, output.stderr)
			}
			if warning, warned := tc.warning.get(); warned != strings.Contains(output.stderr, "native version unavailable") ||
				(warned && !strings.Contains(output.stderr, warning)) {
				t.Fatalf("version warning: %s", output.stderr)
			}
		})
	}
}

func TestCliMember_MemberCreateRendersIdentityAndDatabaseIntoThePromptAndTheRunnerArgv(t *testing.T) {
	t.Parallel()
	conn, dir, s := shortFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "member-environment")
	config := filepath.Join(dir, "claude config")
	if err := os.Mkdir(config, 0o700); err != nil {
		t.Fatal(err)
	}
	descriptorCopy := filepath.Join(dir, "descriptor-copy.json")
	tmux := "printf '%s\\n' \"$*\" >> \"$CAFLEET_TEST_PANE_LOG\"\ncase \" $* \" in\n  *' #{window_height} '*) printf '120\\n' ;;\n  *' display-message '*) printf 'main|@1|%%1\\n' ;;\n  *' split-window '*)\n    while [ $# -gt 0 ]; do\n      if [ \"$1\" = --descriptor ]; then\n        IFS= read -r line < \"$2\" || true\n        printf '%s' \"$line\" > \"$CAFLEET_TEST_DESCRIPTOR_COPY\"\n      fi\n      shift\n    done\n    exit 1 ;;\nesac"
	output, paneCalls := isolatedCliWith(t, s,
		[]string{"member", "create", "--fleet-id", fmt.Sprint(fleet), "--name", "worker", "--description", "member environment fixture",
			"--coding-agent", "claude", "allocated member {member_id} on {database_url}"},
		[][2]string{{"tmux", tmux}, {"claude", "printf '2.1.302 (Claude Code)\\n'"}},
		[][2]string{{"TMUX_PANE", "%1"}, {"CLAUDE_CONFIG_DIR", config}, {"CAFLEET_TEST_DESCRIPTOR_COPY", descriptorCopy}})
	if output.code != 1 || !strings.Contains(output.stderr, "runner pane spawn failed") {
		t.Fatalf("exit %d: %s", output.code, output.stderr)
	}
	split := ""
	for line := range strings.Lines(paneCalls) {
		if strings.Contains(line, " split-window ") {
			split = line
			break
		}
	}
	if split == "" {
		t.Fatalf("the runner pane was requested: %s", paneCalls)
	}
	canonical, err := filepath.EvalSymlinks(filepath.Join(dir, "broker.db"))
	if err != nil {
		t.Fatal(err)
	}
	databaseURL := "sqlite:///" + canonical
	if !strings.Contains(split, " --database-url "+databaseURL+" __member-run --descriptor ") {
		t.Fatalf("split %q", split)
	}
	data, err := os.ReadFile(descriptorCopy)
	if err != nil {
		t.Fatal(err)
	}
	descriptor, err := parseLaunchDescriptor(data)
	if err != nil {
		t.Fatal(err)
	}
	memberID := fmt.Sprint(descriptor.Identity.MemberID)
	argv := descriptor.Spawn.argv
	if !slices.Contains(argv, "allocated member "+memberID+" on "+databaseURL) {
		t.Fatalf("argv %q", argv)
	}
	named := false
	for index := range len(argv) - 1 {
		named = named || (argv[index] == "--name" && argv[index+1] == memberID+" - worker")
	}
	if !named {
		t.Fatalf("argv %q names no pane", argv)
	}
	if want := [][2]string{{"CLAUDE_CONFIG_DIR", config}}; !slices.Equal(descriptor.Spawn.environment, want) {
		t.Fatalf("environment %q", descriptor.Spawn.environment)
	}
}

func uncertainSubmission(cause opt[string], method opt[deliveryMethod]) submissionResult {
	return submissionResult{
		Status:    submissionStatusUncertain,
		InputID:   some(int64(9)),
		Stage:     submissionStagePartial,
		OutputSeq: some(uint64(4)),
		Reason:    cause,
		Delivery:  some(deliveryProjection{Status: deliveryStatusUncertain, Method: method, Reason: none[string]()}),
	}
}

func TestCliMember_WrappedUncertainPromptAndExecErrorsNameTheRecordedCauseAndMethod(t *testing.T) {
	t.Parallel()
	for _, label := range []string{"prompt", "native command"} {
		got := wrappedDeliveryRefusal(label, 110, uncertainSubmission(some("manual_input"), some(deliveryMethodProfile)))
		if want := label + " delivery to member 110 is uncertain: manual_input (method=profile)"; got != want {
			t.Fatalf("%q, want %q", got, want)
		}
	}
	if got := wrappedDeliveryRefusal("native command", 110, uncertainSubmission(some("write_failed"), some(deliveryMethodFallback))); got != "native command delivery to member 110 is uncertain: write_failed (method=fallback)" {
		t.Fatal(got)
	}
	if got := wrappedDeliveryRefusal("prompt", 110, uncertainSubmission(none[string](), none[deliveryMethod]())); got != "prompt delivery to member 110 is uncertain" {
		t.Fatalf("a cause-less receipt without a recorded method adds neither: %q", got)
	}
}

func TestCliMember_PossibleIoRefusalsKeepTheirCategoryInsteadOfClaimingNoDispatch(t *testing.T) {
	t.Parallel()
	const cause = "the input is 6 bytes, above the codex plain-input limit of 3 bytes, and the codex profile has no bracketed-paste delivery"
	statuses := []struct {
		status      submissionStatus
		description string
	}{{submissionStatusUncertain, "uncertain"}, {submissionStatusRejected, "rejected"}}
	for _, label := range []string{"prompt", "native command"} {
		for _, status := range statuses {
			for _, stage := range []submissionStage{submissionStageUnknown, submissionStagePartial, submissionStageSubmitted} {
				result := uncertainSubmission(some(cause), some(deliveryMethodProfile))
				result.Status, result.Stage = status.status, stage
				want := fmt.Sprintf("%s delivery to member 110 is %s: %s (method=profile)", label, status.description, cause)
				if got := wrappedDeliveryRefusal(label, 110, result); got != want {
					t.Fatalf("%q, want %q", got, want)
				}
			}
		}
	}
}

func TestCliMember_DeletionReloadsAPublishedPendingPaneAndRetainsOwnershipThroughCompletion(t *testing.T) {
	t.Parallel()
	for _, expireAfterClose := range []bool{false, true} {
		t.Run(fmt.Sprintf("expire after close %v", expireAfterClose), func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			f := newMemberFixture(t)
			published := false
			setAuthorizer(t, f.conn, func(action sqlite3.AuthorizerActionCode, name3rd, _ string) sqlite3.AuthorizerReturnCode {
				if !published && action == sqlite3.AUTH_TRANSACTION && name3rd == "BEGIN" {
					published = true
					side := fileConn(t, f.url)
					mustExecute(t, side, "UPDATE member_placements SET mux_pane_id='%9', mux_window_id=COALESCE(mux_window_id, '@1'), mux_window_name=COALESCE(mux_window_name, 'main') WHERE member_id=?1", f.worker)
				}
				return sqlite3.AUTH_OK
			})
			d := deadlineUntil(time.Now().Add(500 * time.Millisecond))
			locked, err := lockMemberDeletion(ctx, f.conn, f.settings, f.worker, d)
			if err != nil {
				t.Fatal(err)
			}
			p := locked.snapshot.placement
			if p.muxPaneID != some("%9") {
				t.Fatalf("the deletion reloads the published pane: %v", p.muxPaneID)
			}
			runner := newCloseRunner(t, func() {
				if lock, free := tryPaneLock(t, f.settings, p); free {
					lock.close()
					t.Fatal("the deletion holds the pane lock through the close")
				}
				mustExecBatch(t, fileConn(t, f.url), "BEGIN IMMEDIATE; ROLLBACK;")
				if expireAfterClose {
					time.Sleep(510 * time.Millisecond)
				}
			})
			paneStatus, err := closeMemberWithin(ctx, f.conn, f.settings, f.worker, locked, closeMux(runner, d), d, layoutLockWait)
			if err != nil || paneStatus != "%9 (killed)" {
				t.Fatalf("pane status %q: %v", paneStatus, err)
			}
			if runner.calls != 1 {
				t.Fatalf("%d closes", runner.calls)
			}
			if found, _ := getMember(ctx, f.conn, f.worker, f.fleet); found.set {
				t.Fatal("the worker is deleted")
			}
			if found, _ := getMember(ctx, f.conn, f.director, f.fleet); !found.set {
				t.Fatal("the Director stays registered")
			}
			if lock, free := tryPaneLock(t, f.settings, p); free {
				lock.close()
				t.Fatal("the deletion retains its pane lock until it is closed")
			}
			locked.close()
			paneLockAcquireReleased(t, f.settings.databaseURL, p).close()
		})
	}
}

func TestCliMember_IrreversibleClosePreservesTheRegistryErrorAndReleasesOwnershipOnReturn(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := newMemberFixture(t)
	mustExecute(t, f.conn, "UPDATE member_placements SET mux_pane_id='%9', mux_window_id=COALESCE(mux_window_id, '@1'), mux_window_name=COALESCE(mux_window_name, 'main') WHERE member_id=?1", f.worker)
	mustExecBatch(t, f.conn, "CREATE TRIGGER reject_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'deregister blocked'); END;")
	d := newDeadline()
	locked, err := lockMemberDeletion(ctx, f.conn, f.settings, f.worker, d)
	if err != nil {
		t.Fatal(err)
	}
	defer locked.close()
	runner := newCloseRunner(t, func() {})
	_, err = closeMemberWithin(ctx, f.conn, f.settings, f.worker, locked, closeMux(runner, d), d, layoutLockWait)
	if err == nil || !strings.Contains(errorMessage(err), "was closed") || !strings.Contains(errorMessage(err), "deregister blocked") {
		t.Fatalf("error %v", err)
	}
	if runner.calls != 1 {
		t.Fatalf("%d closes", runner.calls)
	}
	if found, _ := getMember(ctx, f.conn, f.worker, f.fleet); !found.set {
		t.Fatal("the worker stays registered")
	}
}

func TestCliMember_RootTargetsAndInvalidNativeInputFailBeforeMultiplexerEffects(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := newMemberFixture(t)
	if err := cliMemberPrompt(ctx, f.conn, f.settings, f.director, "body", false, newDeadline()); err == nil || !strings.Contains(errorMessage(err), "Director's own pane") {
		t.Fatalf("prompt to the Director: %v", err)
	}
	if err := cliMemberExec(ctx, f.conn, f.settings, f.director, "echo literal", false, newDeadline()); err == nil || !strings.Contains(errorMessage(err), "Director's own pane") {
		t.Fatalf("exec to the Director: %v", err)
	}
	if _, err := lockMemberDeletion(ctx, f.conn, f.settings, f.director, newDeadline()); err == nil {
		t.Fatal("the Director cannot be deleted as a member")
	}
	if err := cliMemberPrompt(ctx, f.conn, f.settings, f.worker, "two\nlines", false, newDeadline()); err == nil {
		t.Fatal("a multi-line prompt is refused")
	}
	pending, err := activeMember(ctx, f.conn, f.worker)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := requirePane(pending, "capture"); err == nil {
		t.Fatal("a pending member has no pane to capture")
	}
}

func TestCliMember_BackendInheritanceAndExplicitSelectionPreserveRequiredStoredValues(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := newMemberFixture(t)
	if agent, err := resolveCodingAgent(ctx, f.conn, f.fleet, f.director, none[string]()); err != nil || agent != codingAgentClaude {
		t.Fatalf("inherited %v %v", agent, err)
	}
	if agent, err := resolveCodingAgent(ctx, f.conn, f.fleet, f.director, some("codex")); err != nil || agent != codingAgentCodex {
		t.Fatalf("explicit %v %v", agent, err)
	}
	mustExecute(t, f.conn, "UPDATE member_placements SET coding_agent='broken' WHERE member_id=?1", f.director)
	if _, err := resolveCodingAgent(ctx, f.conn, f.fleet, f.director, none[string]()); err == nil {
		t.Fatal("a broken stored coding agent fails inheritance")
	}
}

func policyCreateArguments(fleet string) []string {
	return []string{"member", "create", "--fleet-id", fleet, "--name", "worker", "--description", "policy fixture"}
}

func parsedMemberCommand(t *testing.T, args []string) memberCommand {
	t.Helper()
	parsed, exit := parseCli(args...)
	if exit != nil {
		t.Fatalf("%q: %v", args, exit)
	}
	command, member := parsed.command.(memberCommand)
	if !member {
		t.Fatalf("%q is a member command", args)
	}
	return command
}

func runMemberCommand(ctx context.Context, conn *dbConn, s settings, command memberCommand) error {
	prepared, err := prepareInput(command)
	if err != nil {
		return err
	}
	return memberRun(ctx, conn, s, prepared, newDeadline())
}

func invalidCreationWithBody(t *testing.T, backend codingAgent, extra ...string) error {
	t.Helper()
	ctx := context.Background()
	conn, _, s := shortFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "invalid-launch-fixture")
	args := append(policyCreateArguments(fmt.Sprint(fleet)), "--coding-agent", backend.name())
	command := parsedMemberCommand(t, append(args, extra...))
	before := takeTableSnapshot(t, conn, func(string) bool { return true })
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		t.Fatal(err)
	}
	paths, err := runtimePathsForRun(identity, 1)
	if err != nil {
		t.Fatal(err)
	}
	failure := runMemberCommand(ctx, conn, s, command)
	if kind, _ := cafleetKind(failure); kind != errorUsage {
		t.Fatalf("invalid launch must fail as usage: %v", failure)
	}
	if !tableSnapshotsEqual(takeTableSnapshot(t, conn, func(string) bool { return true }), before) {
		t.Fatal("an invalid launch changes no row")
	}
	if _, err := os.Lstat(paths.runDir); err == nil {
		t.Fatal("an invalid launch creates no runtime directory")
	}
	return failure
}

func invalidCreation(t *testing.T, backend codingAgent, extra ...string) error {
	t.Helper()
	return invalidCreationWithBody(t, backend, append(extra, "literal assignment")...)
}

func TestCliMember_LaunchPolicy_RuntimePolicyParserKeepsOmittedSelectionAndModelEffortBoundaries(t *testing.T) {
	t.Parallel()
	args := append(policyCreateArguments("1"), "--coding-agent", "codex", "--model", "fixture-model", "--effort", "high", "literal '$x' `text` assignment")
	create, ok := parsedMemberCommand(t, args).(memberCommandCreate)
	if !ok {
		t.Fatal("member create expected")
	}
	if create.codingAgent != some("codex") || create.model != some("fixture-model") || create.effort != some("high") {
		t.Fatalf("parsed %+v", create.createArgs)
	}
}

func TestCliMember_LaunchPolicy_RuntimePolicyInvalidLaunchSelectionHasUsageDiagnosticsBeforeCreationEffects(t *testing.T) {
	t.Parallel()
	failure := errorMessage(invalidCreation(t, codingAgentCodex, "--permission-mode", "unsupported-fixture-mode"))
	if !strings.Contains(failure, codingAgentCodex.name()) {
		t.Fatalf("%q names no backend", failure)
	}
	for _, choice := range codingAgentCodex.policyChoices() {
		if !strings.Contains(failure, choice) {
			t.Fatalf("%q lacks %q", failure, choice)
		}
	}
}

func TestCliMember_LaunchPolicy_PiPromptBeginningWithAnAtSignFailsBeforeAnyCreationEffect(t *testing.T) {
	t.Parallel()
	file := filepath.Join(t.TempDir(), "prompt.txt")
	if err := os.WriteFile(file, []byte("@notes.md review this"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, extra := range [][]string{{"@notes.md review this"}, {"--file", file}} {
		failure := invalidCreationWithBody(t, codingAgentPi, extra...)
		if cafleetErrorOf(failure).exitCode() != 2 || errorMessage(failure) != "a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment)." {
			t.Fatalf("%q: %v", extra, failure)
		}
	}
}

func TestCliMember_LaunchPolicy_PiModelWithAThinkingSuffixFailsBeforeAnyCreationEffect(t *testing.T) {
	t.Parallel()
	failure := invalidCreation(t, codingAgentPi, "--model", "openrouter/moonshotai/kimi-k2.6:high")
	if want := "--model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got 'openrouter/moonshotai/kimi-k2.6:high')."; errorMessage(failure) != want {
		t.Fatalf("%q", errorMessage(failure))
	}
}

func TestCliMember_LaunchPolicy_UnknownFleetFailsLikeOtherFleetCommandsBeforeMultiplexerResolution(t *testing.T) {
	t.Parallel()
	conn, _, s := shortFileConn(t)
	command := parsedMemberCommand(t, append(policyCreateArguments("999"), "literal assignment"))
	before := takeTableSnapshot(t, conn, func(string) bool { return true })
	failure := runMemberCommand(context.Background(), conn, s, command)
	if kind, _ := cafleetKind(failure); kind != errorApp || errorMessage(failure) != "fleet '999' not found." || cafleetErrorOf(failure).exitCode() != 1 {
		t.Fatalf("unknown fleet: %v", failure)
	}
	if !tableSnapshotsEqual(takeTableSnapshot(t, conn, func(string) bool { return true }), before) {
		t.Fatal("an unknown fleet changes no row")
	}
}

func TestCliMember_Press_PressCliPreservesKeysAfterTheOrdinaryOptionSeparator(t *testing.T) {
	t.Parallel()
	if _, exit := parseCli("member", "show", "42"); exit != nil {
		t.Fatal(exit)
	}
	press, ok := parsedMemberCommand(t, []string{"member", "press", "--json", "--from-member-id", "7", "42", "--", "-", "+", ";", "A", "Ctrl+A", "Shift+Tab"}).(memberCommandPress)
	if !ok {
		t.Fatal("press has its dedicated command arm")
	}
	if press.memberID != 42 || press.directorMemberID != 7 || !press.json || !slices.Equal(press.keys, []string{"-", "+", ";", "A", "Ctrl+A", "Shift+Tab"}) {
		t.Fatalf("press %+v", press)
	}
	for _, args := range [][]string{
		{"member", "press", "--from-member-id", "7", "0", "Enter"},
		{"member", "press", "--from-member-id", "7", "42"},
		{"member", "press", "42", "Enter"},
		{"member", "press", "--from-member-id", "0", "42", "Enter"},
	} {
		_, exit := parseCli(args...)
		if exit == nil || exit.exitCode() != 2 {
			t.Fatalf("%q: press requires a positive member, a positive caller and keys: %v", args, exit)
		}
	}
}

func TestCliMember_SpawnPlaceholdersSubstituteAndUnescape(t *testing.T) {
	t.Parallel()
	cases := []struct{ body, expected string }{
		{"FLEET ID: {fleet_id}\nYOUR MEMBER ID: {member_id}\nDIRECTOR MEMBER ID: {director_member_id}\nCODING AGENT: {coding_agent}\nDATABASE URL: {database_url}",
			"FLEET ID: 3\nYOUR MEMBER ID: 14\nDIRECTOR MEMBER ID: 11\nCODING AGENT: claude\nDATABASE URL: default"},
		{"plain prompt, no braces", "plain prompt, no braces"},
		{"keep {{fleet_id}} literal", "keep {fleet_id} literal"},
		{"{{x}} and {fleet_id}", "{x} and 3"},
	}
	for _, tc := range cases {
		if got, err := substituteSpawnPlaceholders(tc.body, 3, 14, 11, "claude", "default"); err != nil || got != tc.expected {
			t.Fatalf("%q: %q %v", tc.body, got, err)
		}
	}
}

func TestCliMember_SpawnPlaceholderErrorsAreUsageErrors(t *testing.T) {
	t.Parallel()
	const hint = "Double literal braces ({{, }}) to keep them as text."
	cases := []struct{ body, expected string }{
		{"hello {foo}", "Unknown placeholder 'foo' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. " + hint},
		{"{a{b}", "Malformed custom prompt: unexpected '{' in placeholder name. " + hint},
		{"oops }", "Malformed custom prompt: single '}' encountered outside a placeholder. " + hint},
		{"start {fleet_id and never close", "Malformed custom prompt: expected '}' before end of string. " + hint},
	}
	for _, tc := range cases {
		_, err := substituteSpawnPlaceholders(tc.body, 1, 2, 3, "claude", "default")
		if kind, _ := cafleetKind(err); kind != errorUsage || errorMessage(err) != tc.expected || cafleetErrorOf(err).exitCode() != 2 {
			t.Fatalf("%q: %v", tc.body, err)
		}
	}
}

const (
	captureWhen      = "2026-09-06T01:02:03.000007+00:00"
	capturePlainHash = "526a35d455e79594452b17d2378d09d811ce0fb0ece884ac1198c3fc619c10f4"
)

func captureNow(t *testing.T) time.Time {
	t.Helper()
	now, err := time.Parse(time.RFC3339Nano, captureWhen)
	if err != nil {
		t.Fatal(err)
	}
	return now.UTC()
}

func TestCliMember_SnapshotStripsCsiAndCrRewritesBeforeHashingUtf8(t *testing.T) {
	t.Parallel()
	snapshot := captureSnapshotFromRaw("old\r\x1b[31m雪\x1b[0m\nold\r緑", false, captureNow(t))
	if want := (captureSnapshot{content: "雪\n緑", capturedAt: captureWhen, contentSHA256: capturePlainHash}); snapshot != want {
		t.Fatalf("snapshot %+v", snapshot)
	}
}

func TestCliMember_SnapshotAnsiPreservesEveryRawByteAndHashesThatContent(t *testing.T) {
	t.Parallel()
	const raw = "old\r\x1b[31m雪\x1b[0m\nold\r緑"
	snapshot := captureSnapshotFromRaw(raw, true, captureNow(t))
	if snapshot.content != raw || snapshot.capturedAt != captureWhen || snapshot.contentSHA256 != "b9bd2b04f3d96774f4916e31055b70707a34e21a07fb96125c9214df2b1ad191" {
		t.Fatalf("snapshot %+v", snapshot)
	}
}

type failingWriter struct{}

func (failingWriter) Write([]byte) (int, error) { return 0, errors.New("output unavailable") }

func TestCliMember_OutputFailuresPropagateFromMemberCapture(t *testing.T) {
	t.Parallel()
	capture := memberCapture{memberID: 7, paneID: "%9", lines: 12, snapshot: captureSnapshotFromRaw("雪\n緑", false, captureNow(t))}
	for _, json := range []bool{false, true} {
		if err := writeMemberCapture(failingWriter{}, capture, json); err == nil || !strings.Contains(err.Error(), "output unavailable") {
			t.Fatalf("json=%v: %v", json, err)
		}
	}
}

func layoutCreateArguments(fleet int64) []string {
	return []string{"member", "create", "--fleet-id", fmt.Sprint(fleet), "--name", "worker", "--description", "window layout fixture",
		"--coding-agent", "claude", "literal assignment on {database_url}"}
}

func holdLayout(t *testing.T, s settings, fleet int64) *layoutLock {
	t.Helper()
	held, acquired, err := layoutLockTryAcquire(s.databaseURL, fleet)
	if err != nil || !acquired {
		t.Fatalf("the test holds the fleet layout: %v %v", acquired, err)
	}
	t.Cleanup(held.close)
	return held
}

func TestCliMember_WindowLayout_ABusyLayoutLockFailsMemberCreateAndRegistersNothing(t *testing.T) {
	t.Parallel()
	conn, _, s := shortFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout-busy")
	holdLayout(t, s, fleet)
	create, ok := parsedMemberCommand(t, layoutCreateArguments(fleet)).(memberCommandCreate)
	if !ok {
		t.Fatal("member create expected")
	}
	before := memberRows(t, conn)
	started := time.Now()
	err := createWithLayoutWait(context.Background(), conn, create.createArgs, 300*time.Millisecond)
	waited := time.Since(started)
	if waited < 300*time.Millisecond || waited >= 3*time.Second {
		t.Fatalf("waited %v", waited)
	}
	if want := fmt.Sprintf("fleet %d pane layout remained busy for 60 seconds; member was not created.", fleet); err == nil || errorMessage(err) != want || cafleetErrorOf(err).exitCode() != 1 {
		t.Fatalf("error %v", err)
	}
	if memberRows(t, conn) != before {
		t.Fatal("a busy layout registers nothing")
	}
}

func TestCliMember_WindowLayout_TheUnreadableWindowErrorRendersAMultiLineCauseOnOneLine(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _, _ := shortFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout-one-line")
	window := fmt.Sprintf("cafleet-%d-1", fleet)
	first := registerInSlot(t, conn, fleet, "first", 1, columnSideLeft, some(placementPane{muxWindowID: "w1:t3", muxWindowName: window, muxPaneID: "w1:p5"}))
	occupied := occupiedWindow{windowOrdinal: 1, muxWindowID: "w1:t3", muxWindowName: window, left: []string{"w1:p5"}, right: []string{}, panes: []string{"w1:p5"}}
	const stderr = `{"error":{"code":"pane_not_found","message":"boom"},"id":1}`
	cause := newMultiplexerError("herdr command failed: herdr pane layout --pane w1:p5\nstderr: " + stderr)
	want := fmt.Sprintf("window %s (ordinal 1) of fleet %d cannot be read: herdr command failed: herdr pane layout --pane w1:p5; stderr: %s. Its active members %d may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.", window, fleet, stderr, first)
	if got := errorMessage(unreadableWindow(ctx, conn, fleet, occupied, cause)); got != want {
		t.Fatalf("%q\nwant %q", got, want)
	}
	single := newMultiplexerError("unexpected tmux display-message output: ''")
	want = fmt.Sprintf("window %s (ordinal 1) of fleet %d cannot be read: unexpected tmux display-message output: ''. Its active members %d may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.", window, fleet, first)
	if got := errorMessage(unreadableWindow(ctx, conn, fleet, occupied, single)); got != want {
		t.Fatalf("%q\nwant %q", got, want)
	}
}

func TestCliMember_WindowLayout_AnUnreadableOverflowWindowFailsMemberCreateNamingItsMembers(t *testing.T) {
	t.Parallel()
	conn, dir, s := shortFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout-unreadable")
	mustExecute(t, conn, "UPDATE fleets SET last_window_ordinal=1 WHERE fleet_id=?1", fleet)
	window := fmt.Sprintf("cafleet-%d-1", fleet)
	registerInSlot(t, conn, fleet, "director-window", 0, columnSideRight, some(placementPane{muxWindowID: "@1", muxWindowName: "main", muxPaneID: "%2"}))
	first := registerInSlot(t, conn, fleet, "first", 1, columnSideLeft, some(placementPane{muxWindowID: "@7", muxWindowName: window, muxPaneID: "%10"}))
	second := registerInSlot(t, conn, fleet, "second", 1, columnSideRight, some(placementPane{muxWindowID: "@7", muxWindowName: window, muxPaneID: "%11"}))
	before := memberRows(t, conn)
	config := filepath.Join(dir, "claude config")
	if err := os.Mkdir(config, 0o700); err != nil {
		t.Fatal(err)
	}
	tmux := "printf '%s\\n' \"$*\" >> \"$CAFLEET_TEST_PANE_LOG\"\ncase \" $* \" in\n  *' -t @1 #{window_height} '*) printf '25\\n' ;;\n  *' #{window_height} '*) printf '\\n' ;;\n  *) exit 1 ;;\nesac"
	output, paneCalls := isolatedCliWith(t, s, layoutCreateArguments(fleet),
		[][2]string{{"tmux", tmux}, {"claude", "printf '2.1.302 (Claude Code)\\n'"}},
		[][2]string{{"TMUX_PANE", "%0"}, {"CLAUDE_CONFIG_DIR", config}})
	if output.code != 1 {
		t.Fatalf("exit %d: %s", output.code, output.stderr)
	}
	prefix := fmt.Sprintf("Error: window %s (ordinal 1) of fleet %d cannot be read: unexpected tmux display-message output: ", window, fleet)
	suffix := fmt.Sprintf(". Its active members %d, %d may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.\n", first, second)
	if !strings.HasPrefix(output.stderr, prefix) || !strings.HasSuffix(output.stderr, suffix) {
		t.Fatalf("stderr %q", output.stderr)
	}
	if !strings.Contains(paneCalls, " -t @7 #{window_height}") || strings.Contains(paneCalls, "split-window") || strings.Contains(paneCalls, "new-window") {
		t.Fatalf("pane calls %s", paneCalls)
	}
	if memberRows(t, conn) != before {
		t.Fatal("an unreadable window registers nothing")
	}
}

func TestCliMember_WindowLayout_ABusyLayoutLockFailsMemberDeleteAndLeavesRunPaneAndRowUnchanged(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _, s := shortFileConn(t)
	fleet, director := createFleetFixture(t, conn, "layout-delete-busy")
	worker := register(t, conn, fleet, "worker", some("%9"))
	run, err := createRun(ctx, conn, brokerRuntimeFixtureSpec(t, conn, s.databaseURL, fleet, director, worker))
	if err != nil {
		t.Fatal(err)
	}
	before, err := activeMember(ctx, conn, worker)
	if err != nil {
		t.Fatal(err)
	}
	holdLayout(t, s, fleet)
	started := time.Now()
	d := deadlineUntil(started.Add(5 * time.Second))
	locked, err := lockMemberDeletion(ctx, conn, s, worker, d)
	if err != nil {
		t.Fatal(err)
	}
	runner := newCloseRunner(t, func() {})
	_, err = closeMemberWithin(ctx, conn, s, worker, locked, closeMux(runner, d), d, 300*time.Millisecond)
	waited := time.Since(started)
	locked.close()
	if runner.calls != 0 {
		t.Fatal("a busy layout closes no pane")
	}
	if waited < 300*time.Millisecond || waited >= 3*time.Second {
		t.Fatalf("waited %v", waited)
	}
	if want := fmt.Sprintf("fleet %d pane layout remained busy for 60 seconds; member was not deleted.", fleet); err == nil || errorMessage(err) != want || cafleetErrorOf(err).exitCode() != 1 {
		t.Fatalf("error %v", err)
	}
	after, err := activeMember(ctx, conn, worker)
	if err != nil || after.placement != before.placement {
		t.Fatalf("the member keeps its placement: %v", err)
	}
	current := mustRun(t, conn, run.runID)
	if current.status != run.status || current.ownerEpoch != run.ownerEpoch || current.cleanupState != run.cleanupState {
		t.Fatal("the run is unchanged")
	}
	paneLockAcquireReleased(t, s.databaseURL, after.placement).close()
}
