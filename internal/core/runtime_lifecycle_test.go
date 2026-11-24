package core

import (
	"context"
	"encoding/binary"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"github.com/ncruces/go-sqlite3"
	"golang.org/x/sys/unix"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

type shutdownCall struct {
	name string
	at   time.Time
}

// scriptedShutdownHost records every shutdown call with the virtual time it ran at, advances its
// clock to each bounded phase's deadline and fails the calls named in fail.
type scriptedShutdownHost struct {
	testClock *testClock
	calls     []shutdownCall
	deadlines []shutdownCall
	fail      []string
	native    opt[childExit]
	dropped   uint64
	pending   uint64
	gap       uint64
}

func newScriptedShutdownHost(now time.Time) *scriptedShutdownHost {
	return &scriptedShutdownHost{testClock: testClockStartingAt(now), native: some[childExit](childExitCode{code: 0})}
}

func (h *scriptedShutdownHost) reach(deadline time.Time) {
	if remaining := deadline.Sub(h.testClock.now()); remaining > 0 {
		h.testClock.advance(remaining)
	}
}

func (h *scriptedShutdownHost) call(name string) error {
	h.calls = append(h.calls, shutdownCall{name: name, at: h.testClock.now()})
	if slices.Contains(h.fail, name) {
		return appError("fixture " + name + " failure")
	}
	return nil
}

func (h *scriptedShutdownHost) bounded(name string, deadline time.Time) error {
	h.deadlines = append(h.deadlines, shutdownCall{name: name, at: deadline})
	h.reach(deadline)
	return h.call(name)
}

func (h *scriptedShutdownHost) clock() clock { return h.testClock }

func (h *scriptedShutdownHost) gateProgrammaticInput() {
	h.calls = append(h.calls, shutdownCall{name: "gate", at: h.testClock.now()})
}

func (h *scriptedShutdownHost) termVerifiedDescendants() error { return h.call("term") }

func (h *scriptedShutdownHost) drainAndReapUntil(deadline time.Time) (opt[childExit], error) {
	if err := h.bounded("drain", deadline); err != nil {
		return none[childExit](), err
	}
	return h.native, nil
}

func (h *scriptedShutdownHost) killVerifiedSurvivors() error { return h.call("kill") }

func (h *scriptedShutdownHost) closePty() error { return h.call("close") }

func (h *scriptedShutdownHost) finalNativeExit() (opt[childExit], error) {
	return none[childExit](), nil
}

func (h *scriptedShutdownHost) nativeWasSpawned() bool { return true }

func (h *scriptedShutdownHost) flushDisplayUntil(deadline time.Time) (uint64, error) {
	if err := h.bounded("display", deadline); err != nil {
		return 0, err
	}
	return h.dropped, nil
}

func (h *scriptedShutdownHost) finishEventsUntil(deadline time.Time) (uint64, uint64, error) {
	if err := h.bounded("events", deadline); err != nil {
		return 0, 0, err
	}
	return h.pending, h.gap, nil
}

func (h *scriptedShutdownHost) restoreTerminal() error { return h.call("restore") }

var everyShutdownCall = []string{"gate", "term", "drain", "kill", "close", "display", "events", "restore"}

func TestRuntimeLifecycle_RunnerShutdownFollowsOneOriginalScheduleAndKeepsEveryOutcome(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name            string
		hostStartsAfter time.Duration
		drained         opt[childExit]
		fail            []string
		dropped         uint64
		pending         uint64
		gap             uint64
		reported        opt[childExit]
		calls           []string
		nativeExit      opt[childExit]
		cleanupState    cleanupState
		diagnostics     []string
	}{
		{
			name:         "one clock bounds descendants, display, events and restoration",
			drained:      none[childExit](),
			reported:     some[childExit](childExitCode{code: 7}),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 7}),
			cleanupState: cleanupStateComplete,
		},
		{
			name:         "every failure is collected and the terminal is still restored without an extra budget",
			drained:      none[childExit](),
			fail:         []string{"term", "drain", "kill", "close", "display", "events", "restore"},
			reported:     some[childExit](childExitCode{code: 9}),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 9}),
			cleanupState: cleanupStateIncomplete,
			diagnostics: []string{
				"fixture term failure",
				"fixture drain failure",
				"fixture kill failure",
				"fixture close failure",
				"fixture display failure",
				"fixture events failure",
				"fixture restore failure",
			},
		},
		{
			name:         "display truncation, pending frames and unsaved gaps stay separate",
			drained:      some[childExit](childExitCode{code: 0}),
			dropped:      17,
			pending:      2,
			gap:          3,
			reported:     none[childExit](),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 0}),
			cleanupState: cleanupStateIncomplete,
			diagnostics: []string{
				"display truncated: 17 undelivered bytes",
				"pending events=2 unsaved observation gap=3",
			},
		},
		{
			name:            "a clock started before the current phase is never rebased",
			hostStartsAfter: 3 * time.Second,
			drained:         none[childExit](),
			reported:        some[childExit](childExitSignal{signal: 15}),
			calls:           everyShutdownCall,
			nativeExit:      some[childExit](childExitSignal{signal: 15}),
			cleanupState:    cleanupStateComplete,
		},
	}
	for _, c := range cases {
		start := time.Now()
		hostStart := start.Add(c.hostStartsAfter)
		host := newScriptedShutdownHost(hostStart)
		host.native = c.drained
		host.fail = c.fail
		host.dropped = c.dropped
		host.pending = c.pending
		host.gap = c.gap
		outcome := shutdown(host, shutdownDeadlinesStartingAt(start, shutdownUnit), c.reported)
		var calls []string
		for _, call := range host.calls {
			calls = append(calls, call.name)
		}
		if !slices.Equal(calls, c.calls) {
			t.Fatalf("%s: calls %v, want %v", c.name, calls, c.calls)
		}
		wantDeadlines := []shutdownCall{
			{"drain", start.Add(2 * time.Second)},
			{"display", start.Add(3 * time.Second)},
			{"events", start.Add(4 * time.Second)},
		}
		if !slices.EqualFunc(host.deadlines, wantDeadlines, func(a, b shutdownCall) bool { return a.name == b.name && a.at.Equal(b.at) }) {
			t.Fatalf("%s: deadlines %v, want %v", c.name, host.deadlines, wantDeadlines)
		}
		childDeadline := start.Add(2 * time.Second)
		if hostStart.After(childDeadline) {
			childDeadline = hostStart
		}
		for _, call := range host.calls {
			if (call.name == "kill" || call.name == "close") && !call.at.Equal(childDeadline) {
				t.Fatalf("%s: %s ran at %v, want %v", c.name, call.name, call.at, childDeadline)
			}
		}
		if now := host.testClock.now(); !now.Equal(start.Add(4 * time.Second)) {
			t.Fatalf("%s: the clock ends at %v, want %v", c.name, now, start.Add(4*time.Second))
		}
		if outcome.nativeExit != c.nativeExit {
			t.Fatalf("%s: native exit %v, want %v", c.name, outcome.nativeExit, c.nativeExit)
		}
		if outcome.cleanupState != c.cleanupState {
			t.Fatalf("%s: cleanup state %s, want %s", c.name, outcome.cleanupState.asStr(), c.cleanupState.asStr())
		}
		if !slices.Equal(outcome.diagnostics, c.diagnostics) {
			t.Fatalf("%s: diagnostics %q, want %q", c.name, outcome.diagnostics, c.diagnostics)
		}
	}
}

func TestRuntimeLifecycle_RunnerExitStatusKeepsNativeCodeSignalAndCleanupOutcomeDistinct(t *testing.T) {
	t.Parallel()
	cases := []struct {
		native   opt[childExit]
		complete bool
		expected int32
	}{
		{some[childExit](childExitCode{code: 0}), true, 0},
		{some[childExit](childExitCode{code: 7}), true, 7},
		{some[childExit](childExitSignal{signal: 15}), true, 143},
		{some[childExit](childExitCode{code: 0}), false, 1},
		{some[childExit](childExitCode{code: 7}), false, 7},
		{some[childExit](childExitSignal{signal: 15}), false, 143},
		{none[childExit](), false, 1},
	}
	for _, c := range cases {
		status, err := runnerExitStatus(c.native, c.complete)
		if err != nil || status != c.expected {
			t.Fatalf("native %v, cleanup complete %v: status %d, %v; want %d", c.native, c.complete, status, err, c.expected)
		}
	}
	if _, err := runnerExitStatus(none[childExit](), true); err == nil {
		t.Fatal("complete cleanup without a native exit is refused")
	}
}

// lifecycleEndpoint is a run with its artifacts and launch descriptor, whose control socket the
// test serves.
type lifecycleEndpoint struct {
	fixture    *runnerFixture
	artifacts  *runArtifacts
	descriptor launchDescriptor
}

func newLifecycleEndpoint(t *testing.T) *lifecycleEndpoint {
	t.Helper()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	descriptor, err := newLaunchDescriptor(fixture.run, fixture.spec())
	if err != nil {
		t.Fatal(err)
	}
	return &lifecycleEndpoint{fixture: fixture, artifacts: artifacts, descriptor: descriptor}
}

func (e *lifecycleEndpoint) path() string { return e.artifacts.paths.controlSocket }

func (e *lifecycleEndpoint) prepared(t *testing.T) prepared {
	identity := controlIdentityFromRun(e.fixture.run)
	identity.OwnerEpoch++
	return prepared{Identity: identity, Runner: currentProcess(t)}
}

func withNativeLaunchHost[T any](t *testing.T, operation func(*nativeLaunchHost) T) T {
	t.Helper()
	runner := fakeRunnerWithoutBinaries(t)
	target := columnTarget{windowID: "@1", side: columnSideRight, otherPanes: []string{"%1"}}
	host := newNativeLaunchHost(tmuxMultiplexerForServer(runner, "/tmp/tmux-1000/default"), target, "/readiness-fixture", systemClock{})
	result := operation(host)
	if err := host.close(); err != nil {
		t.Error(err)
	}
	if calls := runner.runArgvs(); len(calls) != 0 {
		t.Errorf("readiness runs no process: %q", calls)
	}
	return result
}

// cloexecAccept accepts one connection and marks it close-on-exec under the fork lock, so a
// child started concurrently never inherits it.
func cloexecAccept(listener int) (int, error) {
	syscall.ForkLock.RLock()
	defer syscall.ForkLock.RUnlock()
	fd, _, err := unix.Accept(listener)
	if err != nil {
		return -1, err
	}
	unix.CloseOnExec(fd)
	return fd, nil
}

// acceptBefore accepts one connection on the nonblocking listener until finished or deadline.
func acceptBefore(t *testing.T, listener int, finished *atomic.Bool, deadline time.Time) (int, bool) {
	for {
		fd, err := cloexecAccept(listener)
		if err == nil {
			if err := unix.SetNonblock(fd, true); err != nil {
				t.Error(err)
			}
			return fd, true
		}
		if !errors.Is(err, unix.EAGAIN) {
			t.Errorf("fixture accept failed: %v", err)
			return -1, false
		}
		if finished.Load() || !time.Now().Before(deadline) {
			return -1, false
		}
		time.Sleep(2 * time.Millisecond)
	}
}

func readPrepare(fd int, identity controlIdentity, deadline time.Time) (jsontext.Value, error) {
	header := make([]byte, 4)
	if err := readFixtureBytes(fd, header, deadline); err != nil {
		return nil, err
	}
	length := binary.BigEndian.Uint32(header)
	if length == 0 || length > 65536 {
		return nil, fmt.Errorf("invalid request length %d", length)
	}
	body := make([]byte, length)
	if err := readFixtureBytes(fd, body, deadline); err != nil {
		return nil, err
	}
	request := jsontext.Value(body)
	version, _ := jsonInt64(jsonIndex(request, "protocol_version"))
	kind, _ := jsonString(jsonIndex(request, "kind"))
	remaining, _ := jsonUint64(jsonIndex(request, "remaining_ms"))
	if version != 1 || kind != "prepare" || remaining == 0 || !jsonEqual(jsonIndex(request, "identity"), jsonValue(identity)) {
		return nil, fmt.Errorf("unexpected prepare request %s", request)
	}
	return request, nil
}

type startupInterval uint8

const (
	startupIntervalBound startupInterval = iota + 1
	startupIntervalBeforePrivateMode
)

func readinessListener(t *testing.T, endpoint *lifecycleEndpoint, interval startupInterval) int {
	t.Helper()
	var listener int
	switch interval {
	case startupIntervalBound:
		listener = boundSocket(t, endpoint.path(), 0o600, false)
	case startupIntervalBeforePrivateMode:
		listener = boundSocket(t, endpoint.path(), 0o644, true)
	default:
		panic("invalid startupInterval")
	}
	return listener
}

type readinessServed struct {
	request   jsontext.Value
	premature int
	err       error
}

func readinessSuccess(t *testing.T, interval startupInterval) {
	t.Helper()
	endpoint := newLifecycleEndpoint(t)
	listener := readinessListener(t, endpoint, interval)
	if interval == startupIntervalBound {
		probe := cloexecSocket(t)
		err := unix.Connect(probe, &unix.SockaddrUnix{Name: endpoint.path()})
		if !errors.Is(err, unix.ECONNREFUSED) {
			t.Fatalf("a bound socket refuses connections before it listens: %v", err)
		}
	}
	var finished atomic.Bool
	started := time.Now()
	deadline := started.Add(time.Second)
	readyAt := started.Add(100 * time.Millisecond)
	identity := controlIdentityFromRun(endpoint.fixture.run)
	reply, err := encodeJSON(jsonObject{{"protocol_version", 1}, {"result", endpoint.prepared(t)}})
	if err != nil {
		t.Fatal(err)
	}
	served := make(chan readinessServed, 1)
	go func() {
		premature := 0
		for time.Now().Before(readyAt) {
			if interval == startupIntervalBeforePrivateMode {
				if fd, err := cloexecAccept(listener); err == nil {
					premature++
					closeFD(t, fd)
				}
			}
			if finished.Load() {
				served <- readinessServed{premature: premature}
				return
			}
			time.Sleep(2 * time.Millisecond)
		}
		var ready error
		if interval == startupIntervalBound {
			ready = unix.Listen(listener, 8)
		} else {
			ready = os.Chmod(endpoint.path(), 0o600)
		}
		if ready != nil {
			served <- readinessServed{premature: premature, err: ready}
			return
		}
		fd, accepted := acceptBefore(t, listener, &finished, deadline)
		if !accepted {
			served <- readinessServed{premature: premature}
			return
		}
		defer closeFD(t, fd)
		request, err := readPrepare(fd, identity, deadline)
		if err == nil {
			err = writeFixtureBytes(fd, independentFrame(reply), deadline)
		}
		served <- readinessServed{request: request, premature: premature, err: err}
	}()
	type waited struct {
		ready prepared
		err   error
	}
	result := withNativeLaunchHost(t, func(host *nativeLaunchHost) waited {
		ready, err := host.waitPrepared(endpoint.descriptor, deadline)
		return waited{ready, err}
	})
	finished.Store(true)
	server := <-served
	if server.err != nil || server.premature != 0 {
		t.Fatalf("server %v, %d premature connections", server.err, server.premature)
	}
	if result.err != nil {
		t.Fatalf("startup interval must become ready within its original deadline: %v", result.err)
	}
	if !result.ready.Identity.equal(endpoint.prepared(t).Identity) {
		t.Fatal("the prepared reply carries the advanced identity")
	}
	if time.Since(started) < 100*time.Millisecond || !time.Now().Before(deadline) {
		t.Fatalf("readiness after %v", time.Since(started))
	}
	if remaining, _ := jsonUint64(jsonIndex(server.request, "remaining_ms")); remaining > 900 {
		t.Fatalf("remaining_ms %d", remaining)
	}
}

func TestRuntimeLifecycle_LaunchReadinessTests_RunnerReadinessBoundSocketBecomesListeningWithinTheOriginalDeadline(t *testing.T) {
	t.Parallel()
	readinessSuccess(t, startupIntervalBound)
}

func TestRuntimeLifecycle_LaunchReadinessTests_RunnerReadinessPrePrivateSocketWaitsWithoutSendingPrepare(t *testing.T) {
	t.Parallel()
	readinessSuccess(t, startupIntervalBeforePrivateMode)
}

func TestRuntimeLifecycle_LaunchReadinessTests_RunnerReadinessBoundUnreadySocketExpiresAtTheSuppliedDeadline(t *testing.T) {
	t.Parallel()
	endpoint := newLifecycleEndpoint(t)
	readinessListener(t, endpoint, startupIntervalBound)
	started := time.Now()
	deadline := started.Add(120 * time.Millisecond)
	err := withNativeLaunchHost(t, func(host *nativeLaunchHost) error {
		_, err := host.waitPrepared(endpoint.descriptor, deadline)
		return err
	})
	if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded {
		t.Fatalf("%v, want deadline exceeded", err)
	}
	if time.Now().Before(deadline) || time.Since(started) >= time.Second {
		t.Fatalf("expired after %v", time.Since(started))
	}
}

func TestRuntimeLifecycle_LaunchReadinessTests_RunnerReadinessSymlinkToAPrivateSocketIsAnIdentityFailure(t *testing.T) {
	t.Parallel()
	endpoint := newLifecycleEndpoint(t)
	target := filepath.Join(endpoint.fixture.dir, "linked.sock")
	listener := boundSocket(t, target, 0o600, true)
	if err := os.Symlink(target, endpoint.path()); err != nil {
		t.Fatal(err)
	}
	started := time.Now()
	err := withNativeLaunchHost(t, func(host *nativeLaunchHost) error {
		_, err := host.waitPrepared(endpoint.descriptor, started.Add(time.Second))
		return err
	})
	if err == nil || !strings.Contains(err.Error(), "private control socket identity required") {
		t.Fatalf("invalid endpoint identity must fail explicitly: %v", err)
	}
	if time.Since(started) >= 500*time.Millisecond {
		t.Fatalf("failed after %v", time.Since(started))
	}
	if _, err := cloexecAccept(listener); !errors.Is(err, unix.EAGAIN) {
		t.Fatalf("readiness never connects through the symlink: %v", err)
	}
}

func TestRuntimeLifecycle_LaunchReadinessTests_RunnerReadinessLostReplyAfterWrittenPrepareIsNotReconnected(t *testing.T) {
	t.Parallel()
	bodies := []struct {
		name string
		body opt[string]
	}{
		{"no reply", none[string]()},
		{"unsupported version", some(`{"protocol_version":2,"result":null}`)},
		{"malformed result", some(`{"protocol_version":1,"result":{"identity":null}}`)},
	}
	for _, tc := range bodies {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			endpoint := newLifecycleEndpoint(t)
			listener := readinessListener(t, endpoint, startupIntervalBeforePrivateMode)
			if err := os.Chmod(endpoint.path(), 0o600); err != nil {
				t.Fatal(err)
			}
			var finished atomic.Bool
			started := time.Now()
			deadline := started.Add(time.Second)
			identity := controlIdentityFromRun(endpoint.fixture.run)
			connections := make(chan int, 1)
			go func() {
				fd, accepted := acceptBefore(t, listener, &finished, deadline)
				if !accepted {
					t.Error("actual startup client must connect")
					connections <- 0
					return
				}
				if _, err := readPrepare(fd, identity, deadline); err != nil {
					t.Error(err)
				}
				if body, reply := tc.body.get(); reply {
					if err := writeFixtureBytes(fd, independentFrame([]byte(body)), deadline); err != nil {
						t.Error(err)
					}
				}
				closeFD(t, fd)
				count := 1
				for {
					again, accepted := acceptBefore(t, listener, &finished, deadline)
					if !accepted {
						break
					}
					count++
					closeFD(t, again)
				}
				connections <- count
			}()
			err := withNativeLaunchHost(t, func(host *nativeLaunchHost) error {
				_, err := host.waitPrepared(endpoint.descriptor, deadline)
				return err
			})
			finished.Store(true)
			count := <-connections
			if err == nil {
				t.Fatal("a lost reply fails readiness")
			}
			if kind, _ := cafleetKind(err); kind == errorDeadlineExceeded {
				t.Fatalf("a lost reply fails before the deadline: %v", err)
			}
			if time.Since(started) >= 500*time.Millisecond {
				t.Fatalf("failed after %v", time.Since(started))
			}
			if count != 1 {
				t.Fatalf("a written prepare must not be reconnected: %d connections", count)
			}
		})
	}
}

type publicationFailure uint8

const (
	publicationFailureNone publicationFailure = iota + 1
	publicationFailureMissing
	publicationFailureRejected
)

const lifecycleFixtureWindow = "@4"

func lifecycleFixturePane() spawnedPane {
	return spawnedPane{paneID: "%9", windowID: lifecycleFixtureWindow}
}

type phaseDeadline struct {
	phase    string
	deadline time.Time
}

// launchFixtureHost is the Rust LaunchFixtureHost.
type launchFixtureHost struct {
	t                  *testing.T
	databaseURL        string
	clockSource        *testClock
	delay              time.Duration
	calls              []string
	deadlines          []phaseDeadline
	paneName           opt[string]
	argv               []string
	childArgv          []string
	spawnCount         int
	fail               string
	failureIsUsage     bool
	publicationFailure publicationFailure
	panicAt            string
	corrupt            string
	fastExit           bool
	cleanupFailure     bool
	hermesFailure      opt[string]
	lock               *runLock
	closed             []spawnedPane
}

func newLaunchFixtureHost(t *testing.T, f *runnerFixture) *launchFixtureHost {
	host := &launchFixtureHost{t: t, databaseURL: f.url, clockSource: testClockStartingAt(time.Now()), paneName: none[string](),
		publicationFailure: publicationFailureNone, hermesFailure: none[string]()}
	t.Cleanup(func() {
		if host.lock != nil {
			if err := host.lock.close(); err != nil {
				t.Error(err)
			}
		}
	})
	return host
}

func (h *launchFixtureHost) connect() *dbConn {
	return fileConn(h.t, h.databaseURL)
}

func (h *launchFixtureHost) phase(name string, deadline time.Time) error {
	h.calls = append(h.calls, name)
	h.deadlines = append(h.deadlines, phaseDeadline{name, deadline})
	h.clockSource.advance(h.delay)
	if h.fail != name {
		return nil
	}
	message := fmt.Sprintf("fixture %s failure", name)
	if h.failureIsUsage {
		return usageError(message)
	}
	return appError(message)
}

func (h *launchFixtureHost) clock() clock { return h.clockSource }

func (h *launchFixtureHost) currentExecutable() (string, error) { return "/absolute/cafleet", nil }

func (h *launchFixtureHost) splitRunner(paneName string, argv []string, deadline time.Time) (spawnedPane, error) {
	if err := h.phase("split", deadline); err != nil {
		return spawnedPane{}, err
	}
	h.paneName = some(paneName)
	h.argv = slices.Clone(argv)
	path, err := databasePath(h.databaseURL)
	if err != nil {
		h.t.Fatal(err)
	}
	canonical, err := filepath.EvalSymlinks(path)
	if err != nil {
		h.t.Fatal(err)
	}
	if len(argv) != 6 || argv[0] != "/absolute/cafleet" || argv[1] != "--database-url" || argv[2] != "sqlite:///"+canonical ||
		argv[3] != "__member-run" || argv[4] != "--descriptor" || !filepath.IsAbs(argv[5]) {
		h.t.Fatalf("runner argv %q", argv)
	}
	return lifecycleFixturePane(), nil
}

func (h *launchFixtureHost) waitPrepared(descriptor launchDescriptor, deadline time.Time) (prepared, error) {
	if err := h.phase("prepared", deadline); err != nil {
		return prepared{}, err
	}
	if h.panicAt == "prepared" {
		panic("fixture prepared unwind")
	}
	h.childArgv = slices.Clone(descriptor.Spawn.argv)
	ctx := context.Background()
	conn := h.connect()
	before := mustRun(h.t, conn, descriptor.Identity.RunID)
	if before.childPID.set || h.spawnCount != 0 {
		h.t.Fatal("preparation precedes any child")
	}
	h.lock = acquireReleasableRunLock(h.t, conn, before.owner())
	process := currentProcess(h.t)
	owner, err := h.lock.transferToRunner(ctx, conn, before.owner(), process)
	if err != nil {
		h.t.Fatal(err)
	}
	mustExecute(h.t, conn, "UPDATE member_runs SET runner_pid=?1,runner_start_identity=?2 WHERE run_id=?3", process.pid, process.startIdentity, owner.runID)
	run := mustRun(h.t, conn, owner.runID)
	switch h.publicationFailure {
	case publicationFailureMissing:
		mustExecute(h.t, conn, "DELETE FROM member_placements WHERE member_id=?1", run.memberID)
	case publicationFailureRejected:
		mustExecBatch(h.t, conn, "CREATE TRIGGER reject_pending_publication BEFORE UPDATE OF mux_pane_id ON member_placements BEGIN SELECT RAISE(ABORT,'fixture publication blocked'); END")
	case publicationFailureNone:
	default:
		panic("invalid publicationFailure")
	}
	identity := controlIdentityFromRun(run)
	switch h.corrupt {
	case "prepared-nonce":
		identity.LaunchNonce = strings.Repeat("b", 64)
	case "prepared-epoch":
		identity.OwnerEpoch++
	case "prepared-run":
		identity.RunID++
	case "prepared-director":
		identity.DirectorMemberID++
	}
	return prepared{Identity: identity, Runner: process}, nil
}

func (h *launchFixtureHost) start(ready prepared, p placement, deadline time.Time) (started, error) {
	if err := h.phase("start", deadline); err != nil {
		return started{}, err
	}
	ctx := context.Background()
	conn := h.connect()
	run := mustRun(h.t, conn, ready.Identity.RunID)
	if run.status != runStatusPrepared {
		h.t.Fatalf("start follows preparation: %v", run.status)
	}
	if member := requireMember(h.t, conn, run.memberID, run.fleetID); member.placement.muxPaneID != some("%9") || p.muxPaneID != some("%9") {
		h.t.Fatal("the prepared placement is published before the start")
	}
	h.spawnCount++
	child := processIdentity{pid: run.ownerPID + 1000000, startIdentity: "fixture-child-start"}
	mustExecute(h.t, conn, "UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",
		child.pid, child.startIdentity, "2026-10-26T11:00:00.000000+00:00", run.runID)
	if h.panicAt == "started" {
		panic("fixture started unwind")
	}
	if h.fastExit {
		h.persistFastExit(ctx, conn, run.runID)
	}
	if h.fail == "started-reply" {
		return started{}, appError("fixture started reply lost after spawn")
	}
	identity := controlIdentityFromRun(mustRun(h.t, conn, run.runID))
	replyChild := child
	if h.corrupt == "started-epoch" {
		identity.OwnerEpoch++
	}
	if h.corrupt == "started-child" {
		replyChild.pid++
	}
	return started{Identity: identity, Runner: ready.Runner, Child: replyChild}, nil
}

func (h *launchFixtureHost) persistFastExit(ctx context.Context, conn *dbConn, runID int64) {
	run := mustRun(h.t, conn, runID)
	evidence, err := metadataEvidence(jsontext.Value(`{"backend":"codex","native_version":"0.163.0","rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false}`))
	if err != nil {
		h.t.Fatal(err)
	}
	transition := observedTransition{
		RunID:          run.runID,
		EventSeq:       1,
		EpisodeSeq:     none[int64](),
		Kind:           runtimeEventKindRunnerExited,
		Classification: none[interactionClass](),
		ObservedAt:     "2026-10-26T11:00:00.000000+00:00",
		EvidenceJSON:   evidence,
		CloseReason:    none[interactionCloseReason](),
		CauseJSON:      jsontext.Value(`{"kind":"native_exit","exit_code":7,"exit_signal":null}`),
	}
	if _, _, err := persistTransition(ctx, conn, run.owner(), transition, persistenceModeLive); err != nil {
		h.t.Fatal(err)
	}
}

func (h *launchFixtureHost) shutdownOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error) {
	h.calls = append(h.calls, "shutdown")
	if want := h.clockSource.now().Add(4 * time.Second); !deadlines.storageDeadline().Equal(want) {
		h.t.Fatalf("storage deadline %v, want %v", deadlines.storageDeadline(), want)
	}
	if current := mustRun(h.t, h.connect(), run.runID); current.runID != run.runID {
		h.t.Fatal("shutdown reads its own run")
	}
	if h.cleanupFailure {
		return shutdownOutcome{}, appError("fixture owned-child cleanup failed")
	}
	if h.lock != nil {
		if err := h.lock.close(); err != nil {
			h.t.Fatal(err)
		}
		h.lock = nil
	}
	return shutdownOutcomeComplete(none[childExit]()), nil
}

func (h *launchFixtureHost) settleHermes(_ context.Context, _ *dbConn, run runRecord, paneID string, _ time.Time) error {
	h.calls = append(h.calls, "settle")
	if run.backend != "hermes" || paneID != "%9" {
		h.t.Fatalf("settlement of %s pane %s", run.backend, paneID)
	}
	if cause, failing := h.hermesFailure.get(); failing {
		return appError(cause)
	}
	return nil
}

func (h *launchFixtureHost) closePane(pane spawnedPane, deadline time.Time) error {
	h.calls = append(h.calls, "close")
	h.deadlines = append(h.deadlines, phaseDeadline{"close", deadline})
	h.closed = append(h.closed, pane)
	if pane != lifecycleFixturePane() {
		h.t.Fatalf("closed pane %+v", pane)
	}
	if h.cleanupFailure {
		return appError("fixture pane cleanup failed")
	}
	return nil
}

// launchCase is a fresh fixture, its launch host and the creation deadline ten seconds after the
// host's clock.
func launchCase(t *testing.T) (*runnerFixture, *launchFixtureHost, time.Time) {
	t.Helper()
	fixture := newRunnerFixture(t)
	host := newLaunchFixtureHost(t, fixture)
	return fixture, host, host.clockSource.now().Add(10 * time.Second)
}

func launchInputsFor(t *testing.T, f *runnerFixture, backend string) (pendingMemberSpec, runSettings) {
	t.Helper()
	p := testPlacement(none[string]())
	p.codingAgent = backend
	return pendingMemberSpec{
		fleetID:                   f.run.fleetID,
		name:                      "pending-worker",
		description:               "pending assignment",
		placement:                 p,
		windowName:                "main",
		advancesWindowOrdinalFrom: none[int64](),
	}, runSettings{
		databaseIdentityJSON: f.run.databaseIdentityJSON,
		creator:              currentProcess(t),
		launchNonce:          strings.Repeat("c", 64),
		requestedMode:        none[string](),
		resolvedMode:         "approve-for-me",
		modeSource:           modeSourceDefault,
		permissionArgvJSON:   f.run.permissionArgvJSON,
		nativeVersion:        none[string](),
		coverage:             coverageUnknown,
	}
}

func launchInputs(t *testing.T, f *runnerFixture) (pendingMemberSpec, runSettings) {
	t.Helper()
	return launchInputsFor(t, f, "codex")
}

func launchMember(f *runnerFixture, host *launchFixtureHost, deadline time.Time, member pendingMemberSpec, s runSettings) (launchOutcome, error) {
	spec := f.spec()
	return createRegistered(context.Background(), f.conn, member, s, func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
		artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, deadline)
		return artifacts, spec, err
	}, host, deadline)
}

func launchBackend(t *testing.T, f *runnerFixture, host *launchFixtureHost, deadline time.Time, backend string) (launchOutcome, error) {
	t.Helper()
	member, s := launchInputsFor(t, f, backend)
	return launchMember(f, host, deadline, member, s)
}

func pendingLaunchRun(t *testing.T, f *runnerFixture) runRecord {
	t.Helper()
	id := queryInt64(t, f.conn, "SELECT run_id FROM member_runs WHERE member_id IN (SELECT member_id FROM members WHERE name='pending-worker')")
	return mustRun(t, f.conn, id)
}

// assertUnlaunched checks the member and run rows end in these states and the launch
// descriptor is gone.
func assertUnlaunched(t *testing.T, f *runnerFixture, run runRecord, member memberStatus, status runStatus, name string) {
	t.Helper()
	if recorded := queryText(t, f.conn, "SELECT status FROM members WHERE member_id=?1", run.memberID); recorded != member.asStr() {
		t.Fatalf("%s: member %s, want %s", name, recorded, member.asStr())
	}
	if current := mustRun(t, f.conn, run.runID); current.status != status {
		t.Fatalf("%s: run %v, want %v", name, current.status, status)
	}
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(paths.descriptor); err == nil {
		t.Fatalf("%s: the descriptor is removed", name)
	}
}

func assertCompensated(t *testing.T, f *runnerFixture, run runRecord, name string) {
	t.Helper()
	assertUnlaunched(t, f, run, memberStatusDeregistered, runStatusFailed, name)
}

func countRegistrationCleanup(t *testing.T, conn *dbConn) *atomic.Int64 {
	t.Helper()
	attempts := &atomic.Int64{}
	setAuthorizer(t, conn, func(action sqlite3.AuthorizerActionCode, name3rd, name4th string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_UPDATE && name3rd == "members" && name4th == "status" {
			attempts.Add(1)
		}
		return sqlite3.AUTH_OK
	})
	return attempts
}

func rejectRegistrationCleanup(t *testing.T, conn *dbConn) {
	t.Helper()
	mustExecBatch(t, conn, "CREATE TRIGGER reject_registration_cleanup BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT,'fixture database cleanup blocked'); END")
}

func panicked(operation func()) (unwound bool) {
	defer func() { unwound = recover() != nil }()
	operation()
	return false
}

func TestRuntimeLifecycle_RunnerCreationAllocatesMemberAndRunInOneTransactionOrRollsBothBack(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	member, s := launchInputs(t, f)
	before := queryInt64(t, f.conn, "SELECT COUNT(*) FROM members")
	mustExecBatch(t, f.conn, "CREATE TRIGGER fail_pending_run BEFORE INSERT ON member_runs BEGIN SELECT RAISE(ABORT,'pending run insert failure'); END")
	if _, err := registerPendingRun(context.Background(), f.conn, member, s, time.Now().Add(10*time.Second)); err == nil {
		t.Fatal("a failed run insert fails registration")
	}
	if queryInt64(t, f.conn, "SELECT COUNT(*) FROM members") != before || queryInt64(t, f.conn, "SELECT COUNT(*) FROM member_runs") != 1 {
		t.Fatal("the member and run allocation rolls back together")
	}
}

func TestRuntimeLifecycle_RunnerCreationRecordsTheProbedVersionAndCoverageOnThePendingRun(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		native   opt[string]
		coverage coverage
	}{
		{"an unprobed native version records unknown coverage", none[string](), coverageUnknown},
		{"a probed tested native version records tested coverage", some("0.164.0"), coverageTested},
	}
	for _, tc := range cases {
		f := newRunnerFixture(t)
		member, s := launchInputs(t, f)
		s.nativeVersion, s.coverage = tc.native, tc.coverage
		pending, err := registerPendingRun(context.Background(), f.conn, member, s, time.Now().Add(10*time.Second))
		if err != nil {
			t.Fatal(err)
		}
		for _, run := range []runRecord{pending.run, mustRun(t, f.conn, pending.run.runID)} {
			if run.nativeVersion != tc.native || run.coverage != tc.coverage {
				t.Fatalf("%s: %v %v", tc.name, run.nativeVersion, run.coverage)
			}
		}
	}
}

var preparationStages = []string{"before-artifacts", "after-artifacts", "after-descriptor"}

func prepareUntil(t *testing.T, stage string, deadline time.Time, spec spawnSpec, ctx context.Context, conn *dbConn, pending pendingRegistration) error {
	t.Helper()
	artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, deadline)
	if err != nil {
		return err
	}
	if stage == "after-descriptor" {
		run, err := getRun(ctx, conn, pending.run.runID)
		if err != nil {
			return err
		}
		descriptor, err := newLaunchDescriptor(run, spec)
		if err != nil {
			return err
		}
		if err := artifacts.writeDescriptor(descriptor); err != nil {
			return err
		}
	}
	artifacts.close()
	return nil
}

func TestRuntimeLifecycle_RunnerCreationGuardCoversPreparationFailureBeforeArtifactsAfterAllocationAndAfterDescriptor(t *testing.T) {
	t.Parallel()
	for _, stage := range preparationStages {
		f, host, deadline := launchCase(t)
		member, s := launchInputs(t, f)
		spec := f.spec()
		_, err := createRegistered(context.Background(), f.conn, member, s, func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
			if stage != "before-artifacts" {
				if err := prepareUntil(t, stage, deadline, spec, ctx, conn, pending); err != nil {
					return nil, spawnSpec{}, err
				}
			}
			return nil, spawnSpec{}, appError("fixture preparation failure")
		}, host, deadline)
		if err == nil || !strings.Contains(errorMessage(err), "fixture preparation failure") {
			t.Fatalf("%s must compensate: %v", stage, err)
		}
		assertCompensated(t, f, pendingLaunchRun(t, f), stage)
		if len(host.calls) != 0 || host.spawnCount != 0 {
			t.Fatalf("%s: calls %v", stage, host.calls)
		}
	}
}

func TestRuntimeLifecycle_RunnerCreationPublishesPreparedPlacementBeforeOneStartAndUnlinksTheDescriptor(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f, host, deadline := launchCase(t)
	cleanupAttempts := countRegistrationCleanup(t, f.conn)
	outcome, err := launchBackend(t, f, host, deadline, "codex")
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(host.calls, []string{"split", "prepared", "start"}) || host.spawnCount != 1 {
		t.Fatalf("calls %v spawns %d", host.calls, host.spawnCount)
	}
	for _, recorded := range host.deadlines {
		if !recorded.deadline.Equal(deadline) {
			t.Fatalf("%s ran under %v, want %v", recorded.phase, recorded.deadline, deadline)
		}
	}
	allocated := pendingLaunchRun(t, f).memberID
	if outcome.member.memberID != allocated || outcome.member.name != "pending-worker" || host.paneName != some(fmt.Sprintf("%d - pending-worker", allocated)) {
		t.Fatalf("member %+v pane name %v", outcome.member, host.paneName)
	}
	if outcome.run.status != runStatusRunning || outcome.run.ownerKind != ownerKindRunner || outcome.run.ownerEpoch != 2 {
		t.Fatalf("run %v/%v/%d", outcome.run.status, outcome.run.ownerKind, outcome.run.ownerEpoch)
	}
	if lock, err := runLockAcquire(ctx, f.conn, outcome.run.owner()); err != nil || lock.set {
		t.Fatalf("the runner holds its lock: %v", err)
	}
	if outcome.placement.muxPaneID != some("%9") || outcome.run.childPID != some(outcome.started.Child.pid) {
		t.Fatal("the outcome carries the published pane and the started child")
	}
	paths, err := runtimePathsForRun(outcome.run.databaseIdentityJSON, outcome.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Lstat(paths.descriptor); err == nil {
		t.Fatal("a successful launch unlinks its descriptor")
	}
	if found, _ := getMember(ctx, f.conn, outcome.member.memberID, outcome.run.fleetID); !found.set {
		t.Fatal("the launched member stays registered")
	}
	if cleanupAttempts.Load() != 0 || !slices.Equal(host.calls, []string{"split", "prepared", "start"}) {
		t.Fatal("a successful launch compensates nothing")
	}
}

type compensationCase struct {
	name                     string
	backend                  string
	fail                     string
	corrupt                  string
	publication              publicationFailure
	delay                    time.Duration
	cleanupFailure           bool
	registrationCleanupFails bool
	hermesFailure            opt[string]
	calls                    []string
	spawns                   int
	primary                  string
	diagnostics              []string
	member                   memberStatus
	runStatus                runStatus
	cleanupState             cleanupState
}

func compensation(name string, change func(*compensationCase)) compensationCase {
	c := compensationCase{name: name, backend: "codex", publication: publicationFailureNone, hermesFailure: none[string](),
		member: memberStatusDeregistered, runStatus: runStatusFailed, cleanupState: cleanupStateComplete}
	change(&c)
	return c
}

var (
	spawnedCalls   = []string{"split", "prepared", "start", "shutdown", "close"}
	unspawnedCalls = []string{"split", "prepared", "shutdown", "close"}
	hermesCalls    = []string{"split", "prepared", "start", "settle", "shutdown", "close"}
)

var compensationCases = []compensationCase{
	compensation("a split failure compensates the registration without a pane", func(c *compensationCase) {
		c.fail, c.calls, c.primary = "split", []string{"split"}, "fixture split failure"
	}),
	compensation("a prepare failure closes the owned pane", func(c *compensationCase) {
		c.fail, c.calls, c.primary = "prepared", []string{"split", "prepared", "close"}, "fixture prepared failure"
	}),
	compensation("a spawn failure stops the runner and closes the pane", func(c *compensationCase) {
		c.fail, c.calls, c.primary = "start", spawnedCalls, "fixture start failure"
	}),
	compensation("a stale prepared identity is rejected before starting a child", func(c *compensationCase) {
		c.corrupt, c.calls, c.primary = "prepared-nonce", unspawnedCalls, "identity"
	}),
	compensation("a lost started reply compensates one uncertain spawn", func(c *compensationCase) {
		c.fail, c.calls, c.spawns, c.primary = "started-reply", spawnedCalls, 1, "fixture started reply lost after spawn"
	}),
	compensation("a started reply with a stale epoch compensates one uncertain spawn", func(c *compensationCase) {
		c.corrupt, c.calls, c.spawns, c.primary = "started-epoch", spawnedCalls, 1, "identity"
	}),
	compensation("a started reply naming another child compensates one uncertain spawn", func(c *compensationCase) {
		c.corrupt, c.calls, c.spawns, c.primary = "started-child", spawnedCalls, 1, "started reply lacks a matching completed native spawn"
	}),
	compensation("one handshake deadline expires after start and compensates the spawn", func(c *compensationCase) {
		c.delay, c.calls, c.spawns, c.primary = 4*time.Second, spawnedCalls, 1, "deadline"
	}),
	compensation("a missing placement at publication compensates the owned pane once", func(c *compensationCase) {
		c.publication, c.calls, c.primary = publicationFailureMissing, unspawnedCalls, "placement"
	}),
	compensation("a rejected publication compensates the owned pane once", func(c *compensationCase) {
		c.publication, c.calls, c.primary = publicationFailureRejected, unspawnedCalls, "fixture publication blocked"
		c.diagnostics = []string{"fixture publication blocked"}
	}),
	compensation("a rejected publication keeps every combined cleanup error once", func(c *compensationCase) {
		c.publication, c.cleanupFailure, c.registrationCleanupFails = publicationFailureRejected, true, true
		c.calls, c.primary = unspawnedCalls, "fixture publication blocked"
		c.diagnostics = []string{"fixture publication blocked", "fixture owned-child cleanup failed", "fixture pane cleanup failed", "fixture database cleanup blocked"}
		c.member, c.runStatus, c.cleanupState = memberStatusActive, runStatusStarting, cleanupStateIncomplete
	}),
	compensation("a lost started reply keeps the primary failure and every owned cleanup diagnostic", func(c *compensationCase) {
		c.fail, c.cleanupFailure, c.calls, c.spawns = "started-reply", true, spawnedCalls, 1
		c.primary = "fixture started reply lost after spawn"
		c.diagnostics = []string{"fixture owned-child cleanup failed", "fixture pane cleanup failed"}
		c.cleanupState = cleanupStateIncomplete
	}),
	compensation("a hermes run whose child started settles its client before runner shutdown", func(c *compensationCase) {
		c.backend, c.fail, c.calls, c.spawns, c.primary = "hermes", "started-reply", hermesCalls, 1, "fixture started reply lost after spawn"
	}),
	compensation("a failed hermes settlement keeps its diagnostic and still fails the run", func(c *compensationCase) {
		c.backend, c.fail, c.hermesFailure, c.calls, c.spawns = "hermes", "started-reply", some("fixture hermes settlement failed"), hermesCalls, 1
		c.primary, c.diagnostics, c.cleanupState = "fixture started reply lost after spawn", []string{"fixture hermes settlement failed"}, cleanupStateIncomplete
	}),
	compensation("a hermes run whose spawn failed has no client to settle", func(c *compensationCase) {
		c.backend, c.fail, c.calls, c.primary = "hermes", "start", spawnedCalls, "fixture start failure"
	}),
}

func TestRuntimeLifecycle_RunnerCreationFailureCompensatesEveryOwnedEffectOnce(t *testing.T) {
	t.Parallel()
	for _, tc := range compensationCases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			f, host, deadline := launchCase(t)
			cleanupAttempts := countRegistrationCleanup(t, f.conn)
			if tc.registrationCleanupFails {
				rejectRegistrationCleanup(t, f.conn)
			}
			host.fail, host.corrupt, host.publicationFailure, host.delay = tc.fail, tc.corrupt, tc.publication, tc.delay
			host.cleanupFailure, host.hermesFailure = tc.cleanupFailure, tc.hermesFailure
			_, err := launchBackend(t, f, host, deadline, tc.backend)
			if err == nil {
				t.Fatal("fixture creation must fail")
			}
			text := errorMessage(err)
			first, _, _ := strings.Cut(text, "\n")
			if cafleetErrorOf(err).exitCode() != 1 || !strings.Contains(first, tc.primary) {
				t.Fatalf("error %q", text)
			}
			for _, diagnostic := range tc.diagnostics {
				if strings.Count(text, diagnostic) != 1 {
					t.Fatalf("%q appears %d times in %q", diagnostic, strings.Count(text, diagnostic), text)
				}
			}
			if !slices.Equal(host.calls, tc.calls) || host.spawnCount != tc.spawns {
				t.Fatalf("calls %v spawns %d", host.calls, host.spawnCount)
			}
			for _, recorded := range host.deadlines {
				if recorded.phase != "close" && !recorded.deadline.Equal(deadline) {
					t.Fatalf("%s ran under %v", recorded.phase, recorded.deadline)
				}
			}
			if cleanupAttempts.Load() != 1 {
				t.Fatalf("%d registration cleanups", cleanupAttempts.Load())
			}
			run := pendingLaunchRun(t, f)
			assertUnlaunched(t, f, run, tc.member, tc.runStatus, tc.name)
			if run.cleanupState != tc.cleanupState {
				t.Fatalf("cleanup state %v", run.cleanupState)
			}
		})
	}
}

func TestRuntimeLifecycle_RunnerCreationPreservesUsageExitThroughPreparationAndLaunchCleanup(t *testing.T) {
	t.Parallel()
	for _, phase := range []string{"preparation", "start"} {
		for _, cleanupFails := range []bool{false, true} {
			name := fmt.Sprintf("usage failure at %s, cleanup fails: %v", phase, cleanupFails)
			t.Run(name, func(t *testing.T) {
				t.Parallel()
				f, host, deadline := launchCase(t)
				cleanupAttempts := countRegistrationCleanup(t, f.conn)
				if cleanupFails {
					rejectRegistrationCleanup(t, f.conn)
				}
				var err error
				if phase == "preparation" {
					member, s := launchInputs(t, f)
					_, err = createRegistered(context.Background(), f.conn, member, s, func(context.Context, *dbConn, pendingRegistration) (*runArtifacts, spawnSpec, error) {
						return nil, spawnSpec{}, usageError("fixture preparation usage failure")
					}, host, deadline)
				} else {
					host.fail, host.failureIsUsage = "start", true
					_, err = launchBackend(t, f, host, deadline, "codex")
				}
				if err == nil || cafleetErrorOf(err).exitCode() != 2 || !strings.HasPrefix(errorMessage(err), "fixture "+phase) {
					t.Fatalf("error %v", err)
				}
				wantBlocked := 0
				if cleanupFails {
					wantBlocked = 1
				}
				if strings.Count(errorMessage(err), "fixture database cleanup blocked") != wantBlocked || cleanupAttempts.Load() != 1 || host.spawnCount != 0 {
					t.Fatalf("error %q cleanups %d spawns %d", errorMessage(err), cleanupAttempts.Load(), host.spawnCount)
				}
				run := pendingLaunchRun(t, f)
				if cleanupFails {
					if found, _ := getMember(context.Background(), f.conn, run.memberID, run.fleetID); !found.set {
						t.Fatal("a blocked cleanup keeps the member")
					}
				} else {
					assertCompensated(t, f, run, name)
				}
				expected := []string{}
				if phase == "start" {
					expected = []string{"shutdown", "close"}
				}
				cleanup := []string{}
				for _, call := range host.calls {
					if call == "shutdown" || call == "close" {
						cleanup = append(cleanup, call)
					}
				}
				if !slices.Equal(cleanup, expected) {
					t.Fatalf("cleanup calls %v", cleanup)
				}
			})
		}
	}
}

func TestRuntimeLifecycle_RunnerCreationUnwindDuringPreparationCompensatesRegistrationAndDescriptorOnce(t *testing.T) {
	t.Parallel()
	for _, stage := range preparationStages {
		f, host, deadline := launchCase(t)
		cleanupAttempts := countRegistrationCleanup(t, f.conn)
		member, s := launchInputs(t, f)
		spec := f.spec()
		unwound := panicked(func() {
			_, _ = createRegistered(context.Background(), f.conn, member, s, func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
				if stage != "before-artifacts" {
					if err := prepareUntil(t, stage, deadline, spec, ctx, conn, pending); err != nil {
						return nil, spawnSpec{}, err
					}
				}
				panic("fixture preparation unwind")
			}, host, deadline)
		})
		if !unwound || cleanupAttempts.Load() != 1 {
			t.Fatalf("%s: unwound %v cleanups %d", stage, unwound, cleanupAttempts.Load())
		}
		assertCompensated(t, f, pendingLaunchRun(t, f), stage)
		if len(host.calls) != 0 || host.spawnCount != 0 {
			t.Fatalf("%s: calls %v", stage, host.calls)
		}
	}
}

func assertLaunchUnwindCompensates(t *testing.T, phase string) {
	t.Helper()
	f, host, deadline := launchCase(t)
	cleanupAttempts := countRegistrationCleanup(t, f.conn)
	host.panicAt = phase
	if !panicked(func() { _, _ = launchBackend(t, f, host, deadline, "codex") }) {
		t.Fatal("the fixture unwinds the launch")
	}
	started := 0
	if phase == "started" {
		started = 1
	}
	count := func(name string) int {
		n := 0
		for _, call := range host.calls {
			if call == name {
				n++
			}
		}
		return n
	}
	if host.spawnCount != started || count("shutdown") != started || count("close") != 1 || cleanupAttempts.Load() != 1 {
		t.Fatalf("calls %v spawns %d cleanups %d", host.calls, host.spawnCount, cleanupAttempts.Load())
	}
	assertCompensated(t, f, pendingLaunchRun(t, f), phase)
}

func TestRuntimeLifecycle_RunnerCreationUnwindAfterSplitClosesTheOwnedPaneOnce(t *testing.T) {
	t.Parallel()
	assertLaunchUnwindCompensates(t, "prepared")
}

func TestRuntimeLifecycle_RunnerCreationUnwindAfterSpawnStopsTheOwnedChildAndClosesThePaneOnce(t *testing.T) {
	t.Parallel()
	assertLaunchUnwindCompensates(t, "started")
}

func TestRuntimeLifecycle_RunnerCreationImmediateNonzeroNativeExitIsASuccessfulSpawnWithOneInboxEvent(t *testing.T) {
	t.Parallel()
	f, host, deadline := launchCase(t)
	host.fastExit = true
	outcome, err := launchBackend(t, f, host, deadline, "codex")
	if err != nil {
		t.Fatal(err)
	}
	if host.spawnCount != 1 || slices.Contains(host.calls, "shutdown") {
		t.Fatalf("calls %v", host.calls)
	}
	if outcome.run.status != runStatusExited || outcome.run.exitCode != some(int64(7)) || outcome.run.resolvedMode != "approve-for-me" {
		t.Fatalf("run %v %v %s", outcome.run.status, outcome.run.exitCode, outcome.run.resolvedMode)
	}
	messages, err := pollMessages(context.Background(), f.conn, outcome.run.directorMemberID)
	if err != nil || len(messages) != 1 {
		t.Fatalf("%d messages: %v", len(messages), err)
	}
	payload := jsontext.Value(messages[0].text)
	kind, _ := jsonString(jsonIndex(payload, "kind"))
	code, _ := jsonInt64(jsonIndex(jsonIndex(payload, "cause"), "exit_code"))
	if kind != "runner_exited" || code != 7 {
		t.Fatalf("payload %s", payload)
	}
}

func TestRuntimeLifecycle_RunnerCreationInvalidSpawnConstructionUnderTheGuardKeepsAssignmentOutOfFailedRows(t *testing.T) {
	t.Parallel()
	f, host, deadline := launchCase(t)
	member, s := launchInputs(t, f)
	spec := f.spec()
	const private = "fixture-private-assignment-194"
	spec.argv = append(spec.argv, private)
	spec.cwd = "relative"
	_, err := createRegistered(context.Background(), f.conn, member, s, func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
		artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, deadline)
		if err != nil {
			return nil, spawnSpec{}, err
		}
		run, err := getRun(ctx, conn, pending.run.runID)
		if err != nil {
			artifacts.close()
			return nil, spawnSpec{}, err
		}
		if _, err := newLaunchDescriptor(run, spec); err != nil {
			artifacts.close()
			return nil, spawnSpec{}, err
		}
		return artifacts, spec, nil
	}, host, deadline)
	if err == nil {
		t.Fatal("an invalid spawn fails creation")
	}
	run := pendingLaunchRun(t, f)
	assertCompensated(t, f, run, "invalid spawn construction")
	serialized := queryText(t, f.conn, "SELECT permission_argv_json || COALESCE(cleanup_diagnostics_json,'') FROM member_runs WHERE run_id=?1", run.runID)
	if strings.Contains(serialized, private) || len(host.calls) != 0 {
		t.Fatalf("stored %s calls %v", serialized, host.calls)
	}
}

func TestRuntimeLifecycle_RunnerCreationSubstitutesOnceAndKeepsTheAssignmentInOneLiteralNativeArgument(t *testing.T) {
	t.Parallel()
	f, host, deadline := launchCase(t)
	member, s := launchInputs(t, f)
	spec := f.spec()
	const template = "member={member_id}; database={database_url}; literal={{member_id}}; '$x' `text` 世界"
	outcome, err := createRegistered(context.Background(), f.conn, member, s, func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
		assignment, err := substituteSpawnPlaceholders(template, pending.run.fleetID, pending.member.memberID, pending.run.directorMemberID, "codex", "sqlite:///fixture.db")
		if err != nil {
			return nil, spawnSpec{}, err
		}
		spec.argv = []string{"/fixture/codex", assignment}
		artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, deadline)
		return artifacts, spec, err
	}, host, deadline)
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"/fixture/codex", fmt.Sprintf("member=%d; database=sqlite:///fixture.db; literal={member_id}; '$x' `text` 世界", outcome.member.memberID)}
	if !slices.Equal(host.childArgv, want) {
		t.Fatalf("child argv %q", host.childArgv)
	}
	for _, arg := range host.argv {
		if strings.Contains(arg, "literal=") {
			t.Fatalf("runner argv %q carries the assignment", host.argv)
		}
	}
}

func TestRuntimeLifecycle_RunnerRecoveryClockClipsEachOriginalBoundaryAndNeverRenewsTime(t *testing.T) {
	t.Parallel()
	start := time.Now()
	for _, offset := range []time.Duration{0, 1, 2, 3, 4, 9} {
		until := start.Add(offset * time.Second)
		if got, want := shutdownDeadlinesStartingAt(start, shutdownUnit).remainingStorageAt(until), max(4*time.Second-offset*time.Second, 0); got != want {
			t.Fatalf("clipped %d s after start: remaining %v, want %v", offset, got, want)
		}
		clipped := shutdownDeadlinesStartingAt(start, shutdownUnit).clipped(until)
		for _, boundary := range []struct {
			got   time.Time
			units time.Duration
		}{{clipped.childDeadline(), 2}, {clipped.displayDeadline(), 3}, {clipped.storageDeadline(), 4}} {
			if want := minTime(start.Add(boundary.units*time.Second), until); !boundary.got.Equal(want) {
				t.Fatalf("clipped %d s after start: boundary %v, want %v", offset, boundary.got, want)
			}
		}
		if clipped.remainingStorageAt(until) != 0 {
			t.Fatalf("clipped %d s after start: storage time remains", offset)
		}
	}
	past := start.Add(-time.Second)
	clipped := shutdownDeadlinesStartingAt(start, shutdownUnit).clipped(past)
	if !clipped.childDeadline().Equal(past) || !clipped.displayDeadline().Equal(past) || !clipped.storageDeadline().Equal(past) || clipped.remainingStorageAt(start) != 0 {
		t.Fatal("a past clip ends every boundary at the clip")
	}
}

func TestRuntimeLifecycle_RunnerRecoveryClockRepeatedClippingOnlyTightensTheSameOriginalSchedule(t *testing.T) {
	t.Parallel()
	start := time.Now()
	first := start.Add(2500 * time.Millisecond)
	second := start.Add(1500 * time.Millisecond)
	clipped := shutdownDeadlinesStartingAt(start, shutdownUnit).clipped(first).clipped(start.Add(20 * time.Second))
	if !clipped.childDeadline().Equal(start.Add(2*time.Second)) || !clipped.displayDeadline().Equal(first) || !clipped.storageDeadline().Equal(first) {
		t.Fatal("a later clip never renews time")
	}
	clipped = clipped.clipped(second).clipped(first)
	if !clipped.childDeadline().Equal(second) || !clipped.displayDeadline().Equal(second) || !clipped.storageDeadline().Equal(second) {
		t.Fatal("an earlier clip tightens every boundary")
	}
}

const (
	lifecycleSlowSum = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter"
	lifecycleFastSum = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter"
)

func lifecycleSum(conn *dbConn, sql string) (int64, error) {
	return queryRow(conn, sql, nil, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func assertFastSum(t *testing.T, conn *dbConn, name string) {
	t.Helper()
	if total, err := lifecycleSum(conn, lifecycleFastSum); err != nil || total != 50005000 {
		t.Fatalf("%s: the connection is reusable: %d %v", name, total, err)
	}
}

type deadlinePanic struct{ code int }

func recoverDeadlinePanic(t *testing.T, want int, operation func()) {
	t.Helper()
	payload := func() (payload any) {
		defer func() { payload = recover() }()
		operation()
		return nil
	}()
	if payload != (deadlinePanic{want}) {
		t.Fatalf("the original operation panic propagates: %v", payload)
	}
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineBoundsALockedWriteAndRestoresTheBusyTimeout(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	mustExecBatch(t, f.conn, "CREATE TABLE deadline_fixture(value INTEGER)")
	mustExecBatch(t, f.conn, "PRAGMA busy_timeout=137")
	other := fileConn(t, f.url)
	mustExecBatch(t, other, "BEGIN IMMEDIATE")
	start := time.Now()
	_, err := withDatabaseDeadline(context.Background(), f.conn, start.Add(80*time.Millisecond), func(_ context.Context, conn *dbConn) (int64, error) {
		changed, err := sqlExecute(conn, "INSERT INTO deadline_fixture VALUES(1)")
		return changed, classifiedFailure(err)
	})
	if err == nil || time.Since(start) >= 750*time.Millisecond {
		t.Fatalf("the locked write ends at its deadline: %v after %v", err, time.Since(start))
	}
	if timeout := queryInt64(t, f.conn, "PRAGMA busy_timeout"); timeout != 137 {
		t.Fatalf("busy timeout %d", timeout)
	}
	mustExecBatch(t, other, "ROLLBACK")
	mustExecute(t, f.conn, "INSERT INTO deadline_fixture VALUES(2)")
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineCancelsALongStatementAndRemovesItsProgressHandler(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	start := time.Now()
	_, err := withDatabaseDeadline(context.Background(), f.conn, start.Add(80*time.Millisecond), func(_ context.Context, conn *dbConn) (int64, error) {
		total, err := lifecycleSum(conn, lifecycleSlowSum)
		return total, classifiedFailure(err)
	})
	if err == nil || !cafleetErrorOf(err).transientStorage() {
		t.Fatalf("a statement cut off by its deadline is transient: %v", err)
	}
	if time.Since(start) >= 750*time.Millisecond {
		t.Fatalf("cancelled after %v", time.Since(start))
	}
	assertFastSum(t, f.conn, "after cancellation")
	if timeout := queryInt64(t, f.conn, "PRAGMA busy_timeout"); timeout != 5000 {
		t.Fatalf("busy timeout %d", timeout)
	}
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlinePreservesAnAlreadyExpiredSharedBudget(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	invoked := false
	_, err := withDatabaseDeadline(context.Background(), f.conn, time.Now().Add(-time.Second), func(context.Context, *dbConn) (struct{}, error) {
		invoked = true
		return struct{}{}, nil
	})
	if err == nil || invoked {
		t.Fatalf("an expired budget runs nothing: %v invoked=%v", err, invoked)
	}
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineReviewUnwindPreservesThePanicAndRestoresConnectionReuse(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	mustExecBatch(t, f.conn, "PRAGMA busy_timeout=137")
	until := time.Now().Add(40 * time.Millisecond)
	recoverDeadlinePanic(t, 194, func() {
		_, _ = withDatabaseDeadline(context.Background(), f.conn, until, func(context.Context, *dbConn) (struct{}, error) {
			panic(deadlinePanic{194})
		})
	})
	time.Sleep(time.Until(until) + 20*time.Millisecond)
	assertFastSum(t, f.conn, "after the unwind")
	if timeout := queryInt64(t, f.conn, "PRAGMA busy_timeout"); timeout != 137 {
		t.Fatalf("busy timeout %d", timeout)
	}
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineReviewOperationErrorStopsItsWatchdogAndRestoresReuse(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	mustExecBatch(t, f.conn, "PRAGMA busy_timeout=173")
	until := time.Now().Add(100 * time.Millisecond)
	started := time.Now()
	_, err := withDatabaseDeadline(context.Background(), f.conn, until, func(context.Context, *dbConn) (struct{}, error) {
		return struct{}{}, usageError("fixture deadline operation error")
	})
	if err == nil || cafleetErrorOf(err).exitCode() != 2 || errorMessage(err) != "fixture deadline operation error" || time.Since(started) >= 750*time.Millisecond {
		t.Fatalf("the operation error returns unchanged: %v", err)
	}
	time.Sleep(time.Until(until) + 20*time.Millisecond)
	assertFastSum(t, f.conn, "after the operation error")
	if timeout := queryInt64(t, f.conn, "PRAGMA busy_timeout"); timeout != 173 {
		t.Fatalf("busy timeout %d", timeout)
	}
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineReviewNestedLaterDeadlineKeepsTheEarliestOuterBudget(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	started := time.Now()
	_, err := withDatabaseDeadline(context.Background(), f.conn, started.Add(100*time.Millisecond), func(ctx context.Context, conn *dbConn) (int64, error) {
		return withDatabaseDeadline(ctx, conn, started.Add(time.Second), func(_ context.Context, conn *dbConn) (int64, error) {
			return lifecycleSum(conn, lifecycleSlowSum)
		})
	})
	if elapsed := time.Since(started); err == nil || elapsed < 70*time.Millisecond || elapsed >= 750*time.Millisecond {
		t.Fatalf("the outer budget ends the nested statement: %v after %v", err, elapsed)
	}
	assertFastSum(t, f.conn, "after the nested scope")
}

func TestRuntimeLifecycle_RunnerDatabaseDeadlineReviewARetiredInnerScopeRestoresTheLiveOuterScope(t *testing.T) {
	t.Parallel()
	for _, unwinds := range []bool{false, true} {
		name := "an inner scope that succeeds"
		if unwinds {
			name = "an inner scope that unwinds"
		}
		t.Run(name, func(t *testing.T) {
			t.Parallel()
			f := newRunnerFixture(t)
			started := time.Now()
			_, err := withDatabaseDeadline(context.Background(), f.conn, started.Add(1500*time.Millisecond), func(ctx context.Context, conn *dbConn) (int64, error) {
				innerUntil := time.Now().Add(40 * time.Millisecond)
				if unwinds {
					recoverDeadlinePanic(t, 195, func() {
						_, _ = withDatabaseDeadline(ctx, conn, innerUntil, func(context.Context, *dbConn) (struct{}, error) {
							panic(deadlinePanic{195})
						})
					})
				} else if inner, err := withDatabaseDeadline(ctx, conn, innerUntil, func(context.Context, *dbConn) (int64, error) { return 7, nil }); err != nil || inner != 7 {
					t.Fatalf("the inner scope returns its value: %d %v", inner, err)
				}
				time.Sleep(time.Until(innerUntil) + 20*time.Millisecond)
				assertFastSum(t, conn, name)
				return lifecycleSum(conn, lifecycleSlowSum)
			})
			if elapsed := time.Since(started); err == nil || elapsed < 1450*time.Millisecond || elapsed >= 2350*time.Millisecond {
				t.Fatalf("the outer scope still cancels its slow query: %v after %v", err, elapsed)
			}
			assertFastSum(t, f.conn, name)
		})
	}
}

func lastWindowOrdinalOf(t *testing.T, f *runnerFixture) int64 {
	t.Helper()
	return queryInt64(t, f.conn, "SELECT last_window_ordinal FROM fleets WHERE fleet_id=?1", f.run.fleetID)
}

// overflowMember is a pending member that opens overflow window 1 from a fleet whose last
// ordinal is 0.
func overflowMember(t *testing.T, f *runnerFixture) (pendingMemberSpec, runSettings) {
	t.Helper()
	member, s := launchInputs(t, f)
	member.placement.windowOrdinal = 1
	member.placement.columnSide = columnSideLeft
	member.windowName = fmt.Sprintf("cafleet-%d-1", f.run.fleetID)
	member.advancesWindowOrdinalFrom = some(int64(0))
	return member, s
}

type recordedPlacement struct {
	pane, window, windowName opt[string]
	ordinal                  int64
	side                     columnSide
}

func recordedOf(p placement) recordedPlacement {
	return recordedPlacement{p.muxPaneID, p.muxWindowID, p.muxWindowName, p.windowOrdinal, p.columnSide}
}

func TestRuntimeLifecycle_WindowLayout_PublicationRecordsTheSpawnedPaneAndWindowWithTheChosenWindowName(t *testing.T) {
	t.Parallel()
	f, host, deadline := launchCase(t)
	member, s := overflowMember(t, f)
	outcome, err := launchMember(f, host, deadline, member, s)
	if err != nil {
		t.Fatal(err)
	}
	want := recordedPlacement{some("%9"), some(lifecycleFixtureWindow), some(fmt.Sprintf("cafleet-%d-1", f.run.fleetID)), 1, columnSideLeft}
	if got := recordedOf(outcome.placement); got != want {
		t.Fatalf("outcome %+v", got)
	}
	if got := recordedOf(requireMember(t, f.conn, outcome.member.memberID, f.run.fleetID).placement); got != want {
		t.Fatalf("stored %+v", got)
	}
}

func TestRuntimeLifecycle_WindowLayout_ADirectorWindowMemberPublishesTheDirectorWindowNameItWasGiven(t *testing.T) {
	t.Parallel()
	f, host, deadline := launchCase(t)
	member, s := launchInputs(t, f)
	member.windowName = "agents"
	outcome, err := launchMember(f, host, deadline, member, s)
	if err != nil {
		t.Fatal(err)
	}
	if outcome.placement.muxWindowName != some("agents") || outcome.placement.windowOrdinal != 0 || outcome.placement.columnSide != columnSideRight {
		t.Fatalf("placement %+v", recordedOf(outcome.placement))
	}
}

func TestRuntimeLifecycle_WindowLayout_AnOverflowLaunchFailureClosesOnlyItsSpawnedPaneAndKeepsTheNewOrdinalConsumed(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name, fail string
		closed     []spawnedPane
	}{
		{"a prepare failure closes the pane in the window it was spawned in", "prepared", []spawnedPane{lifecycleFixturePane()}},
		{"a split failure spawned no pane to close", "split", nil},
	}
	for _, tc := range cases {
		f, host, deadline := launchCase(t)
		host.fail = tc.fail
		member, s := overflowMember(t, f)
		if lastWindowOrdinalOf(t, f) != 0 {
			t.Fatalf("%s: the fleet starts at ordinal 0", tc.name)
		}
		_, err := launchMember(f, host, deadline, member, s)
		if err == nil || !strings.Contains(errorMessage(err), "fixture "+tc.fail+" failure") {
			t.Fatalf("%s: %v", tc.name, err)
		}
		if !slices.Equal(host.closed, tc.closed) {
			t.Fatalf("%s: closed %+v", tc.name, host.closed)
		}
		assertCompensated(t, f, pendingLaunchRun(t, f), tc.name)
		if lastWindowOrdinalOf(t, f) != 1 {
			t.Fatalf("%s: the new ordinal stays consumed", tc.name)
		}
	}
}

func TestRuntimeLifecycle_WindowLayout_RegistrationAdvancesTheWindowOrdinalOnlyForANewWindow(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := newRunnerFixture(t)
	member, s := launchInputs(t, f)
	if _, err := registerPendingRun(ctx, f.conn, member, s, time.Now().Add(10*time.Second)); err != nil {
		t.Fatal(err)
	}
	if lastWindowOrdinalOf(t, f) != 0 {
		t.Fatal("a Director-window member advances no ordinal")
	}
	f = newRunnerFixture(t)
	member, s = overflowMember(t, f)
	pending, err := registerPendingRun(ctx, f.conn, member, s, time.Now().Add(10*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	if lastWindowOrdinalOf(t, f) != 1 {
		t.Fatal("a new-window member advances the ordinal")
	}
	want := recordedPlacement{none[string](), none[string](), none[string](), 1, columnSideLeft}
	if got := recordedOf(requireMember(t, f.conn, pending.member.memberID, f.run.fleetID).placement); got != want {
		t.Fatalf("placement %+v", got)
	}
}

func TestRuntimeLifecycle_WindowLayout_AWindowOrdinalChangedUnderTheLockRollsTheRegistrationBack(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	fleet := f.run.fleetID
	mustExecute(t, f.conn, "UPDATE fleets SET last_window_ordinal=2 WHERE fleet_id=?1", fleet)
	members, runs := queryInt64(t, f.conn, "SELECT COUNT(*) FROM members"), queryInt64(t, f.conn, "SELECT COUNT(*) FROM member_runs")
	member, s := overflowMember(t, f)
	_, err := registerPendingRun(context.Background(), f.conn, member, s, time.Now().Add(10*time.Second))
	if want := fmt.Sprintf("fleet %d window ordinal changed under the layout lock", fleet); err == nil || errorMessage(err) != want || cafleetErrorOf(err).exitCode() != 1 {
		t.Fatalf("a changed ordinal fails registration: %v", err)
	}
	if queryInt64(t, f.conn, "SELECT COUNT(*) FROM members") != members || queryInt64(t, f.conn, "SELECT COUNT(*) FROM member_runs") != runs || lastWindowOrdinalOf(t, f) != 2 {
		t.Fatal("the registration rolls back")
	}
}
