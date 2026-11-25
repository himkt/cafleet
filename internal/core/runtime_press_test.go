package core

import (
	"context"
	"fmt"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
)

const pressServer = "/owned/tmux-fixture.sock"

func pressAcknowledgmentFixture(t *testing.T) keyAcknowledgment {
	t.Helper()
	keys, err := pressKeysParse([]string{"cTrL+A", ";"})
	if err != nil {
		t.Fatal(err)
	}
	return keyAcknowledgment{memberID: 42, paneID: "%5", keys: keys.canonical()}
}

type pressMode uint8

const (
	pressModeSuccess pressMode = iota + 1
	pressModeChangePlacement
	pressModeExpireMetadata
	pressModeNotStarted
	pressModeStartedFailure
)

// pressProbe is the tmux runner of one press attempt. It asserts that every process runs inside
// the invocation deadline, under the held pane lock and a termination-deferral span.
type pressProbe struct {
	t           *testing.T
	conn        *dbConn
	databaseURL string
	placement   placement
	memberID    int64
	started     time.Time
	until       time.Time
	mode        pressMode
	mu          sync.Mutex
	calls       [][]string
}

func (p *pressProbe) recorded() [][]string {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Clone(p.calls)
}

func (p *pressProbe) writes() int {
	count := 0
	for _, call := range p.recorded() {
		if call[1] == "send-keys" {
			count++
		}
	}
	return count
}

func (p *pressProbe) binaryExists(name string) bool {
	if name != "tmux" {
		p.t.Errorf("binaryExists(%q), want tmux", name)
	}
	return true
}

func (p *pressProbe) run([]string) (string, *runError) {
	p.t.Error("press bounds every process by the invocation deadline")
	return "", &runError{kind: runErrorNotStarted, detail: "unbounded process"}
}

func (p *pressProbe) assertProcessContext(timeout time.Duration) {
	if timeout <= 0 || p.started.Add(timeout).After(p.until) {
		p.t.Errorf("each process is bounded by the remaining invocation deadline: %v", timeout)
	}
	if !time.Now().Before(p.until) {
		p.t.Error("no new process is offered after expiry")
	}
	held, acquired, err := paneLockTryAcquire(p.databaseURL, p.placement)
	if err != nil {
		p.t.Error(err)
	}
	if acquired {
		held.close()
		p.t.Error("the original physical pane lock remains held through every process call")
	}
	terminationDeferral.mu.Lock()
	deferred := terminationDeferral.depth > 0
	terminationDeferral.mu.Unlock()
	if !deferred {
		p.t.Error("termination is deferred while press owns input")
	}
}

func (p *pressProbe) runFor(argv []string, timeout time.Duration) (string, *runError) {
	p.assertProcessContext(timeout)
	if len(argv) < 4 || !slices.Equal(argv[:3], []string{"tmux", "-S", pressServer}) {
		p.t.Errorf("press process %q runs on the owned server", argv)
		return "", &runError{kind: runErrorNotStarted, detail: "foreign server"}
	}
	args := append([]string{argv[0]}, argv[3:]...)
	p.mu.Lock()
	p.calls = append(p.calls, args)
	p.mu.Unlock()
	switch args[1] {
	case "display-message":
		return p.metadata(args)
	case "send-keys":
		return p.sendKeys(args)
	default:
		p.t.Errorf("unexpected press process: %q", args)
		return "", &runError{kind: runErrorNotStarted, detail: "unexpected process"}
	}
}

func (p *pressProbe) metadata(args []string) (string, *runError) {
	if query := args[len(args)-1]; query != "#{pane_id}|#{window_id}|#{session_name}" {
		p.t.Errorf("unexpected metadata query: %s", query)
	}
	if !slices.Contains(args, "%2") {
		p.t.Errorf("metadata query %q targets the recorded pane", args)
	}
	switch p.mode {
	case pressModeChangePlacement:
		changed, err := p.conn.execute("UPDATE member_placements SET coding_agent='claude' WHERE member_id=?1", p.memberID)
		if err != nil || changed != 1 {
			p.t.Errorf("placement change: %d %v", changed, err)
		}
	case pressModeExpireMetadata:
		time.Sleep(time.Until(p.until) + time.Millisecond)
	case pressModeSuccess, pressModeNotStarted, pressModeStartedFailure:
	}
	return "%2|@1|main\n", nil
}

func (p *pressProbe) sendKeys(args []string) (string, *runError) {
	if want := []string{"tmux", "send-keys", "-t", "%2", "Tab"}; !slices.Equal(args, want) {
		p.t.Errorf("send-keys argv %q, want %q", args, want)
	}
	if p.writes() != 1 {
		p.t.Error("a started failure is never retried")
	}
	switch p.mode {
	case pressModeSuccess:
		return "", nil
	case pressModeNotStarted:
		return "", &runError{kind: runErrorNotStarted, detail: "owned prelaunch refusal"}
	case pressModeStartedFailure:
		return "", &runError{kind: runErrorFailed, detail: "can't find pane: owned started failure"}
	case pressModeChangePlacement, pressModeExpireMetadata:
		p.t.Error("post-metadata refusal precedes writing")
	}
	return "", &runError{kind: runErrorNotStarted, detail: "refused write"}
}

func pressSetup(t *testing.T, fixture *runnerFixture, mode pressMode, until time.Time) (settings, *pressProbe, multiplexer) {
	t.Helper()
	s := settingsForDatabase(filepath.Join(fixture.dir, "broker.db"))
	member := requireMember(t, fixture.conn, fixture.run.memberID, fixture.run.fleetID)
	if _, present := member.runtime.get(); !present {
		t.Fatal("the fixture member has a runtime")
	}
	probe := &pressProbe{
		t:           t,
		conn:        fixture.conn,
		databaseURL: s.databaseURL,
		placement:   member.placement,
		memberID:    member.memberID,
		started:     time.Now(),
		until:       until,
		mode:        mode,
	}
	mux, err := multiplexerForServer("tmux", pressServer, probe)
	if err != nil {
		t.Fatal(err.message)
	}
	return s, probe, mux
}

func pressTabKeys(t *testing.T) pressKeys {
	t.Helper()
	keys, err := pressKeysParse([]string{"tAb"})
	if err != nil {
		t.Fatal(err)
	}
	return keys
}

// pressLoaded loads memberID as pressMember does and presses keys through mux.
func pressLoaded(t *testing.T, fixture *runnerFixture, s settings, memberID int64, keys pressKeys, until time.Time, mux multiplexer) (keyAcknowledgment, error) {
	t.Helper()
	member, err := activeMember(context.Background(), fixture.conn, memberID)
	if err != nil {
		t.Fatal(err)
	}
	return pressMemberWithMux(context.Background(), fixture.conn, s, member, keys, deadlineUntil(until), mux, systemClock{})
}

func pressFailure(t *testing.T, name string, err error) *cafleetError {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: this press attempt is refused", name)
	}
	return cafleetErrorOf(err)
}

func pressPrimary(cause *cafleetError) *cafleetError {
	for cause.kind == errorWithDiagnostics {
		cause = cause.cause
	}
	return cause
}

func assertPressZeroEffect(t *testing.T, name string, cause *cafleetError, member int64) {
	t.Helper()
	if cause.exitCode() != 1 || !strings.HasSuffix(cause.message(), fmt.Sprintf("keys were not dispatched to member %d.", member)) {
		t.Fatalf("%s: exit %d, message %q", name, cause.exitCode(), cause.message())
	}
}

func assertPressLockReleased(t *testing.T, name string, probe *pressProbe) {
	t.Helper()
	held, acquired, err := paneLockAcquireWithin(probe.databaseURL, probe.placement, deadlineWithin(releasedLockGrace))
	if err != nil || !acquired {
		t.Fatalf("%s: post-refusal pane lock is released: %v %v (mode=%d placement=%+v)", name, acquired, err, probe.mode, probe.placement)
	}
	held.close()
}

func TestRuntimePress_PressEntryOutcomesFollowTheMultiplexerAndReleaseThePaneLock(t *testing.T) {
	t.Parallel()
	type outcome uint8
	const (
		acknowledged outcome = iota + 1
		placementChanged
		expired
		notDispatched
		uncertain
	)
	cases := []struct {
		name    string
		mode    pressMode
		budget  time.Duration
		calls   int
		writes  int
		outcome outcome
	}{
		{"success acknowledges the canonical keys without touching the run", pressModeSuccess, 2 * time.Second, 2, 1, acknowledged},
		{"a placement change after metadata refuses before writing", pressModeChangePlacement, 2 * time.Second, 1, 0, placementChanged},
		{"expiry during metadata refuses before writing", pressModeExpireMetadata, 300 * time.Millisecond, 1, 0, expired},
		{"a prelaunch refusal reports zero effect", pressModeNotStarted, 2 * time.Second, 2, 1, notDispatched},
		{"a started failure stays uncertain even for pane-gone stderr", pressModeStartedFailure, 2 * time.Second, 2, 1, uncertain},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			fixture := newRunnerFixture(t)
			until := time.Now().Add(tc.budget)
			s, probe, mux := pressSetup(t, fixture, tc.mode, until)
			member := fixture.run.memberID
			before, err := getRun(context.Background(), fixture.conn, fixture.run.runID)
			if err != nil {
				t.Fatal(err)
			}
			value, err := pressLoaded(t, fixture, s, member, pressTabKeys(t), until, mux)
			if got := len(probe.recorded()); got != tc.calls {
				t.Fatalf("%s: %d processes, want %d", tc.name, got, tc.calls)
			}
			if got := probe.writes(); got != tc.writes {
				t.Fatalf("%s: %d writes, want %d", tc.name, got, tc.writes)
			}
			switch tc.outcome {
			case acknowledged:
				if err != nil {
					t.Fatalf("%s: %v", tc.name, err)
				}
				if value.memberID != member || value.paneID != "%2" || !slices.Equal(value.keys, []string{"Tab"}) {
					t.Fatalf("%s: acknowledgment %+v", tc.name, value)
				}
				after, err := getRun(context.Background(), fixture.conn, fixture.run.runID)
				if err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(after, before) {
					t.Fatalf("%s: run changed\n%+v\nwant\n%+v", tc.name, after, before)
				}
				if inputs := queryInt64(t, fixture.conn, "SELECT COUNT(*) FROM member_input_operations"); inputs != 0 {
					t.Fatalf("%s: %d input operations", tc.name, inputs)
				}
			case placementChanged:
				cause := pressFailure(t, tc.name, err)
				assertPressZeroEffect(t, tc.name, cause, member)
				if got, want := pressPrimary(cause).message(), fmt.Sprintf("member %d placement changed before key dispatch", member); got != want {
					t.Fatalf("%s: primary %q, want %q", tc.name, got, want)
				}
			case expired:
				cause := pressFailure(t, tc.name, err)
				assertPressZeroEffect(t, tc.name, cause, member)
				if pressPrimary(cause).kind != errorDeadlineExceeded {
					t.Fatalf("%s: primary %q, want the deadline", tc.name, cause.message())
				}
			case notDispatched:
				cause := pressFailure(t, tc.name, err)
				if !strings.Contains(pressPrimary(cause).message(), "owned prelaunch refusal") {
					t.Fatalf("%s: primary %q", tc.name, cause.message())
				}
				assertPressZeroEffect(t, tc.name, cause, member)
			case uncertain:
				cause := pressFailure(t, tc.name, err)
				text := cause.message()
				effect := fmt.Sprintf("key delivery to member %d is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.", member)
				primary := strings.Index(text, "owned started failure")
				if cause.exitCode() != 1 || primary < 0 || primary > strings.Index(text, effect) ||
					!strings.HasSuffix(text, effect) || strings.Count(text, effect) != 1 {
					t.Fatalf("%s: exit %d, message %q", tc.name, cause.exitCode(), text)
				}
			}
			assertPressLockReleased(t, tc.name, probe)
		})
	}
}

func TestRuntimePress_PressEntryRefusesDirectorAndABusySharedLockBeforeProbes(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	until := time.Now().Add(2 * time.Second)
	s, probe, mux := pressSetup(t, fixture, pressModeSuccess, until)
	_, err := pressLoaded(t, fixture, s, fixture.run.directorMemberID, pressTabKeys(t), until, mux)
	director := pressFailure(t, "director", err)
	if got := pressPrimary(director).message(); got != "cannot press keys in the Director's own pane" {
		t.Fatalf("director primary %q", got)
	}
	assertPressZeroEffect(t, "director", director, fixture.run.directorMemberID)
	if calls := probe.recorded(); len(calls) != 0 {
		t.Fatalf("director refusal ran %q", calls)
	}
	held, acquired, err := paneLockTryAcquire(s.databaseURL, probe.placement)
	if err != nil || !acquired {
		t.Fatalf("the shared pane lock is taken: %v %v", acquired, err)
	}
	_, err = pressLoaded(t, fixture, s, fixture.run.memberID, pressTabKeys(t), time.Now().Add(1100*time.Millisecond), mux)
	busy := pressFailure(t, "busy", err)
	if got, want := pressPrimary(busy).message(), fmt.Sprintf("member %d's pane input remained busy", fixture.run.memberID); got != want {
		t.Fatalf("busy primary %q, want %q", got, want)
	}
	assertPressZeroEffect(t, "busy", busy, fixture.run.memberID)
	if calls := probe.recorded(); len(calls) != 0 {
		t.Fatalf("busy refusal ran %q", calls)
	}
	held.close()
	assertPressLockReleased(t, "busy", probe)
}
