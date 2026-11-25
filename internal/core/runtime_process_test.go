package core

import (
	"errors"
	"fmt"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/creack/pty"
	"golang.org/x/sys/unix"
)

// fixtureProcessTable is a literal process table: absent pids read as absent and unreadable
// pids fail with their message.
type fixtureProcessTable struct {
	own              int64
	entries          map[int64]processEntry
	unreadablePIDs   map[int64]string
	nestedNamespace  bool
	namespaceFailure opt[string]
}

var _ processTable = (*fixtureProcessTable)(nil)

func newFixtureProcessTable(own int64) *fixtureProcessTable {
	return &fixtureProcessTable{own: own, entries: map[int64]processEntry{}, unreadablePIDs: map[int64]string{}}
}

func (f *fixtureProcessTable) process(pid, parent int64, argv ...string) *fixtureProcessTable {
	if _, declared := f.entries[pid]; declared {
		panic(fmt.Sprintf("fixture pid %d is declared twice", pid))
	}
	f.entries[pid] = processEntry{parent: parent, argv: argv}
	return f
}

func (f *fixtureProcessTable) unreadable(pid int64, message string) *fixtureProcessTable {
	if _, declared := f.unreadablePIDs[pid]; declared {
		panic(fmt.Sprintf("fixture pid %d is declared unreadable twice", pid))
	}
	f.unreadablePIDs[pid] = message
	return f
}

func (f *fixtureProcessTable) nested() *fixtureProcessTable {
	f.nestedNamespace = true
	return f
}

func (f *fixtureProcessTable) namespaceError(message string) *fixtureProcessTable {
	f.namespaceFailure = some(message)
	return f
}

func (f *fixtureProcessTable) ownPID() int64 {
	return f.own
}

func (f *fixtureProcessTable) entry(pid int64) (processEntry, bool, error) {
	if message, unreadable := f.unreadablePIDs[pid]; unreadable {
		return processEntry{}, false, appError(message)
	}
	entry, present := f.entries[pid]
	return entry, present, nil
}

func (f *fixtureProcessTable) nestedPIDNamespace() (bool, error) {
	if message, failed := f.namespaceFailure.get(); failed {
		return false, appError(message)
	}
	return f.nestedNamespace, nil
}

// zombieInGroup starts `sleep 30` in process group group (0 makes it lead its own group), kills
// it and waits until it is an unreaped zombie that still matches its recorded identity.
func zombieInGroup(t *testing.T, group int) (*exec.Cmd, processIdentity) {
	t.Helper()
	child := exec.Command("sleep", "30")
	child.SysProcAttr = &syscall.SysProcAttr{Setpgid: true, Pgid: group}
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	identity := requireIdentity(t, int64(child.Process.Pid))
	if err := child.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	waitUntilStopped(t, identity.pid)
	matches, err := identity.matches()
	if err != nil {
		t.Fatal(err)
	}
	if !matches {
		t.Fatalf("an unreaped zombie keeps its recorded identity %#v", identity)
	}
	return child, identity
}

func zombie(t *testing.T) (*exec.Cmd, processIdentity) {
	t.Helper()
	return zombieInGroup(t, 0)
}

func requireIdentity(t *testing.T, pid int64) processIdentity {
	t.Helper()
	identity, present, err := processIdentityRead(pid)
	if err != nil {
		t.Fatal(err)
	}
	if !present {
		t.Fatalf("a just-spawned process %d is visible", pid)
	}
	return identity
}

func waitUntilStopped(t *testing.T, pid int64) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for {
		gone, err := stopped(pid)
		if err != nil {
			t.Fatal(err)
		}
		if gone {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("process %d did not exit", pid)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func assertReaped(t *testing.T, name string, pid int64) {
	t.Helper()
	var status syscall.WaitStatus
	_, err := syscall.Wait4(int(pid), &status, syscall.WNOHANG, nil)
	if !errors.Is(err, syscall.ECHILD) {
		t.Fatalf("%s: wait4(%d) = %v, want ECHILD: the direct child is already reaped", name, pid, err)
	}
}

func TestRuntimeProcess_ZombieLiveness_AProcessWithItsRecordedIdentityIsAliveOnlyAsThatGeneration(t *testing.T) {
	t.Parallel()
	current, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	alive, err := current.isAlive()
	if err != nil || !alive {
		t.Fatalf("the current process is alive as its own generation: %v %v", alive, err)
	}
	other := processIdentity{pid: current.pid, startIdentity: current.startIdentity + "-other"}
	alive, err = other.isAlive()
	if err != nil || alive {
		t.Fatalf("another generation of the current PID is not alive: %v %v", alive, err)
	}
}

func TestRuntimeProcess_ZombieLiveness_AnUnreapedZombieKeepsItsIdentityButIsNotAlive(t *testing.T) {
	t.Parallel()
	child, identity := zombie(t)
	checks := func(stage string, wantMatches bool) {
		matches, err := identity.matches()
		if err != nil || matches != wantMatches {
			t.Fatalf("%s: matches = %v %v, want %v", stage, matches, err, wantMatches)
		}
		alive, err := identity.isAlive()
		if err != nil || alive {
			t.Fatalf("%s: isAlive = %v %v, want false", stage, alive, err)
		}
	}
	checks("unreaped zombie", true)
	if err := child.Wait(); err == nil {
		t.Fatal("the killed child reports its signal")
	}
	checks("reaped", false)
}

func runWithChild(t *testing.T, child processIdentity) runRecord {
	t.Helper()
	run := newRunnerFixture(t).run
	run.childPID = some(child.pid)
	run.childStartIdentity = some(child.startIdentity)
	return run
}

func recoveryDeadlines() shutdownDeadlines {
	return shutdownDeadlinesStartingAt(time.Now(), 100*time.Millisecond)
}

// groupLeader starts `sleep 30` as the leader of its own process group.
func groupLeader(t *testing.T) (*exec.Cmd, processIdentity) {
	t.Helper()
	child := exec.Command("sleep", "30")
	child.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	return child, requireIdentity(t, int64(child.Process.Pid))
}

func killAndReap(t *testing.T, child *exec.Cmd) {
	t.Helper()
	if err := child.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	if err := child.Wait(); err == nil {
		t.Fatal("the killed child reports its signal")
	}
}

func assertRecovery(t *testing.T, name string, outcome shutdownOutcome, err error, diagnostics []string, state cleanupState) {
	t.Helper()
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	if !slices.Equal(outcome.diagnostics, diagnostics) || outcome.cleanupState != state {
		t.Fatalf("%s: outcome %#v, want diagnostics %q and cleanup state %v", name, outcome, diagnostics, state)
	}
}

func TestRuntimeProcess_ZombieLiveness_RecoveryCompletesForAZombieChildAloneInItsGroup(t *testing.T) {
	t.Parallel()
	child, identity := zombie(t)
	outcome, err := recoverDescendants(runWithChild(t, identity), recoveryDeadlines(), systemClock{})
	if waitErr := child.Wait(); waitErr == nil {
		t.Fatal("the killed child reports its signal")
	}
	assertRecovery(t, "zombie child", outcome, err, nil, cleanupStateComplete)
}

func TestRuntimeProcess_ZombieLiveness_RecoveryCountsOnlyNonZombieMembersOfAReapedChildGroup(t *testing.T) {
	t.Parallel()
	leader, identity := groupLeader(t)
	descendant, _ := zombieInGroup(t, int(identity.pid))
	killAndReap(t, leader)
	outcome, err := recoverDescendants(runWithChild(t, identity), recoveryDeadlines(), systemClock{})
	if waitErr := descendant.Wait(); waitErr == nil {
		t.Fatal("the killed descendant reports its signal")
	}
	assertRecovery(t, "reaped child group", outcome, err, nil, cleanupStateComplete)
}

func TestRuntimeProcess_ZombieLiveness_RecoveryStillRefusesALiveGroupMemberItCannotAnchor(t *testing.T) {
	t.Parallel()
	leader, identity := groupLeader(t)
	stale := processIdentity{pid: identity.pid, startIdentity: identity.startIdentity + "-stale"}
	outcome, err := recoverDescendants(runWithChild(t, stale), recoveryDeadlines(), systemClock{})
	killAndReap(t, leader)
	assertRecovery(t, "unanchored live member", outcome, err,
		[]string{"descendant ownership cannot be anchored to the recorded child generation"}, cleanupStateIncomplete)
}

func TestRuntimeProcess_AReapedPidReadsAsAbsentRatherThanAnError(t *testing.T) {
	t.Parallel()
	child := exec.Command("true")
	if err := child.Run(); err != nil {
		t.Fatal(err)
	}
	identity, present, err := processIdentityRead(int64(child.Process.Pid))
	if err != nil {
		t.Fatalf("a reaped PID read fails: %v", err)
	}
	if present {
		t.Fatalf("a reaped PID reads as present: %#v", identity)
	}
	gone, err := stopped(int64(child.Process.Pid))
	if err != nil || !gone {
		t.Fatalf("a reaped PID is stopped: %v %v", gone, err)
	}
}

const (
	refreshGroup int32 = 41001
	refreshA     int64 = 41001
	refreshB     int64 = 41002
	refreshC     int64 = 41003
)

type processObservationKind uint8

const (
	observeMembers processObservationKind = iota + 1
	observeIdentity
	observeStopped
)

type observationStep struct {
	kind     processObservationKind
	group    int32
	pid      int64
	members  []int64
	identity processIdentity
	present  bool
	stopped  bool
	err      error
}

// observationScript answers every process observation from its steps in order and records the
// first observation that departs from them.
type observationScript struct {
	steps     []observationStep
	violation string
}

var _ processObservations = (*observationScript)(nil)

func newObservationScript(steps ...[]observationStep) *observationScript {
	return &observationScript{steps: slices.Concat(steps...)}
}

func (s *observationScript) next(kind processObservationKind, subject string) (observationStep, bool) {
	if s.violation != "" {
		return observationStep{}, false
	}
	if len(s.steps) == 0 {
		s.violation = "unscripted observation: " + subject
		return observationStep{}, false
	}
	step := s.steps[0]
	s.steps = s.steps[1:]
	if step.kind != kind {
		s.violation = fmt.Sprintf("observation %s does not match the scripted step %#v", subject, step)
		return observationStep{}, false
	}
	return step, true
}

func (s *observationScript) members(group int32) ([]int64, error) {
	step, ok := s.next(observeMembers, fmt.Sprintf("members of group %d", group))
	if !ok {
		return nil, appError("script violated")
	}
	if step.group != group {
		s.violation = fmt.Sprintf("enumerated group %d, scripted %d", group, step.group)
	}
	return step.members, step.err
}

func (s *observationScript) identity(pid int64) (processIdentity, bool, error) {
	step, ok := s.next(observeIdentity, fmt.Sprintf("identity of %d", pid))
	if !ok {
		return processIdentity{}, false, appError("script violated")
	}
	if step.pid != pid {
		s.violation = fmt.Sprintf("identified PID %d, scripted %d", pid, step.pid)
	}
	return step.identity, step.present, step.err
}

func (s *observationScript) stopped(pid int64) (bool, error) {
	step, ok := s.next(observeStopped, fmt.Sprintf("stopped state of %d", pid))
	if !ok {
		return false, appError("script violated")
	}
	if step.pid != pid {
		s.violation = fmt.Sprintf("inspected PID %d, scripted %d", pid, step.pid)
	}
	return step.stopped, step.err
}

func (s *observationScript) finished(t *testing.T, name string) {
	t.Helper()
	if s.violation != "" {
		t.Fatalf("%s: %s", name, s.violation)
	}
	if len(s.steps) != 0 {
		t.Fatalf("%s: unused observations: %#v", name, s.steps)
	}
}

func syntheticIdentity(pid int64) processIdentity {
	return processIdentity{pid: pid, startIdentity: fmt.Sprintf("synthetic-generation-%d", pid)}
}

func replacementIdentity(pid int64) processIdentity {
	return processIdentity{pid: pid, startIdentity: fmt.Sprintf("synthetic-replacement-%d", pid)}
}

func membersStep(pids ...int64) []observationStep {
	return []observationStep{{kind: observeMembers, group: refreshGroup, members: pids}}
}

func membersFailure(group int32, err error) []observationStep {
	return []observationStep{{kind: observeMembers, group: group, err: err}}
}

func observedStep(process processIdentity) []observationStep {
	return []observationStep{{kind: observeIdentity, pid: process.pid, identity: process, present: true}}
}

func absentStep(pid int64) []observationStep {
	return []observationStep{{kind: observeIdentity, pid: pid}}
}

func identityFailure(pid int64, err error) []observationStep {
	return []observationStep{{kind: observeIdentity, pid: pid, err: err}}
}

func stoppedStep(pid int64, gone bool) []observationStep {
	return []observationStep{{kind: observeStopped, pid: pid, stopped: gone}}
}

func stoppedFailure(pid int64, err error) []observationStep {
	return []observationStep{{kind: observeStopped, pid: pid, err: err}}
}

func activeSteps(process processIdentity) []observationStep {
	return slices.Concat(observedStep(process), stoppedStep(process.pid, false))
}

func pinnedMap(processes ...processIdentity) map[int64]processIdentity {
	pinned := make(map[int64]processIdentity, len(processes))
	for _, process := range processes {
		pinned[process.pid] = process
	}
	return pinned
}

// inertSession is a session whose native A has exited and been reaped, holding only its pinned
// generations and its original clipped shutdown clock.
func inertSession(pinned ...processIdentity) *ptySession {
	origin := time.Now()
	return &ptySession{
		clock:             systemClock{},
		native:            some(syntheticIdentity(refreshA)),
		exit:              some[childExit](childExitCode{code: 0}),
		owned:             pinnedMap(pinned...),
		shutdownDeadlines: some(shutdownDeadlinesStartingAt(origin, shutdownUnit).clipped(origin.Add(900 * time.Millisecond))),
	}
}

func boundaries(t *testing.T, session *ptySession) [3]time.Time {
	t.Helper()
	deadlines, ok := session.shutdownDeadlines.get()
	if !ok {
		t.Fatal("the fixture retains its original clock")
	}
	return [3]time.Time{deadlines.childDeadline(), deadlines.displayDeadline(), deadlines.storageDeadline()}
}

func assertSessionState(t *testing.T, name string, session *ptySession, pinned map[int64]processIdentity, closed bool, clock [3]time.Time) {
	t.Helper()
	if !maps.Equal(session.owned, pinned) {
		t.Fatalf("%s: owned %#v, want %#v", name, session.owned, pinned)
	}
	if session.groupClosed != closed {
		t.Fatalf("%s: groupClosed %v, want %v", name, session.groupClosed, closed)
	}
	if got := boundaries(t, session); got != clock {
		t.Fatalf("%s: shutdown boundaries %v, want %v", name, got, clock)
	}
	if _, ok := session.child.get(); ok {
		t.Fatalf("%s: an inert session gained a child", name)
	}
	if session.native != some(syntheticIdentity(refreshA)) {
		t.Fatalf("%s: native %#v changed", name, session.native)
	}
	if session.exit != some[childExit](childExitCode{code: 0}) {
		t.Fatalf("%s: exit %#v changed", name, session.exit)
	}
}

func assertRefreshError(t *testing.T, name string, err error, message string) {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: the refresh succeeded, want %q", name, message)
	}
	if got := cafleetErrorOf(err).message(); got != message || cafleetErrorOf(err).exitCode() != 1 {
		t.Fatalf("%s: error %q, want the application error %q", name, got, message)
	}
}

func falseFoldScript(recheck []observationStep) *observationScript {
	return newObservationScript(
		membersStep(refreshA),
		activeSteps(syntheticIdentity(refreshA)),
		absentStep(refreshA),
		recheck,
	)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshMatchingAnchorAdoptsOnlyOriginalActiveGenerations(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA, refreshB),
		activeSteps(syntheticIdentity(refreshA)),
		activeSteps(syntheticIdentity(refreshB)),
		observedStep(syntheticIdentity(refreshA)),
	)
	if err := session.refreshOwnedWith(script); err != nil {
		t.Fatal(err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinnedMap(syntheticIdentity(refreshA), syntheticIdentity(refreshB)), false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_SurvivorsReportALiveGroupMemberTheClosedRefreshNeverPinned(t *testing.T) {
	t.Parallel()
	session := inertSession()
	session.groupClosed = true
	script := newObservationScript(membersStep(refreshB), activeSteps(syntheticIdentity(refreshB)))
	survivors, err := session.survivorsWith(script)
	if err != nil || !survivors {
		t.Fatalf("survivors = %v %v, want a live unpinned member", survivors, err)
	}
	script.finished(t, "survivors")
}

func TestRuntimeProcess_OwnershipRefreshTests_SurvivorsIgnoreAZombieGroupMemberOnceOwnedProcessesAreGone(t *testing.T) {
	t.Parallel()
	session := inertSession()
	session.groupClosed = true
	script := newObservationScript(membersStep(refreshB), observedStep(syntheticIdentity(refreshB)), stoppedStep(refreshB, true))
	survivors, err := session.survivorsWith(script)
	if err != nil || survivors {
		t.Fatalf("survivors = %v %v, want none", survivors, err)
	}
	script.finished(t, "survivors")
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshInitialEmptyClosesWithOneQueryAndRetainsMapAndClock(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(membersStep())
	if err := session.refreshOwnedWith(script); err != nil {
		t.Fatal(err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, true, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshStaleActiveThenFreshEmptyClosesWithoutAdoptionOrMoreReads(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA, refreshB),
		activeSteps(syntheticIdentity(refreshA)),
		activeSteps(syntheticIdentity(refreshB)),
		absentStep(refreshA),
		membersStep(),
	)
	if err := session.refreshOwnedWith(script); err != nil {
		t.Fatal(err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, true, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshFreshRemainingMembersPreserveExactRefusalWithoutAdoption(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := falseFoldScript(membersStep(refreshB, refreshC))
	assertRefreshError(t, "refresh", session.refreshOwnedWith(script), "owned process group lost its pinned generation")
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshSamePidReplacementCannotSupplyAnAnchorOrBeAdopted(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA),
		activeSteps(syntheticIdentity(refreshA)),
		observedStep(replacementIdentity(refreshA)),
		membersStep(refreshA),
	)
	assertRefreshError(t, "refresh", session.refreshOwnedWith(script), "owned process group lost its pinned generation")
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshOriginalObservationErrorsStopAtTheActualFirstFailure(t *testing.T) {
	t.Parallel()
	failure := databaseBusyError("synthetic-original-observation")
	cases := []struct {
		name   string
		prefix [][]observationStep
	}{
		{name: "enumeration", prefix: [][]observationStep{membersFailure(refreshGroup, failure)}},
		{name: "first identity", prefix: [][]observationStep{membersStep(refreshA, refreshB), identityFailure(refreshA, failure)}},
		{name: "first stopped check", prefix: [][]observationStep{
			membersStep(refreshA, refreshB), observedStep(syntheticIdentity(refreshA)), stoppedFailure(refreshA, failure),
		}},
		{name: "anchor read", prefix: [][]observationStep{
			membersStep(refreshA), observedStep(syntheticIdentity(refreshA)), stoppedStep(refreshA, false), identityFailure(refreshA, failure),
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			session := inertSession(syntheticIdentity(refreshA))
			pinned := maps.Clone(session.owned)
			clock := boundaries(t, session)
			script := newObservationScript(tc.prefix...)
			if err := session.refreshOwnedWith(script); err != failure {
				t.Fatalf("%s: refresh returned %v, want the scripted failure itself", tc.name, err)
			}
			script.finished(t, tc.name)
			assertSessionState(t, tc.name, session, pinned, false, clock)
		})
	}
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshAnchorFoldStillReadsLaterPinsAfterAMatchAndPropagatesError(t *testing.T) {
	t.Parallel()
	failure := appError("synthetic-later-anchor-read")
	session := inertSession(syntheticIdentity(refreshA), syntheticIdentity(refreshC))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA),
		activeSteps(syntheticIdentity(refreshA)),
		observedStep(syntheticIdentity(refreshA)),
		identityFailure(refreshC, failure),
	)
	if err := session.refreshOwnedWith(script); err != failure {
		t.Fatalf("refresh returned %v, want the scripted failure itself", err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshFreshQueryErrorPropagatesWithoutClosureOrFollowupReads(t *testing.T) {
	t.Parallel()
	failure := databaseBusyError("synthetic-fresh-enumeration")
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := falseFoldScript(membersFailure(refreshGroup, failure))
	if err := session.refreshOwnedWith(script); err != failure {
		t.Fatalf("refresh returned %v, want the scripted failure itself", err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshAnchoredPidMismatchRetainsFenceWithoutFreshQueryFallback(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA), syntheticIdentity(refreshC))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA, refreshC),
		activeSteps(replacementIdentity(refreshA)),
		activeSteps(syntheticIdentity(refreshC)),
		observedStep(replacementIdentity(refreshA)),
		observedStep(syntheticIdentity(refreshC)),
	)
	assertRefreshError(t, "refresh", session.refreshOwnedWith(script), "owned PID was replaced by another generation")
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, false, clock)
}

func TestRuntimeProcess_OwnershipRefreshTests_RefreshAbsentAndStoppedMembersCloseBeforeAnyAnchorFold(t *testing.T) {
	t.Parallel()
	session := inertSession(syntheticIdentity(refreshA))
	pinned := maps.Clone(session.owned)
	clock := boundaries(t, session)
	script := newObservationScript(
		membersStep(refreshA, refreshB),
		absentStep(refreshA),
		observedStep(syntheticIdentity(refreshB)),
		stoppedStep(refreshB, true),
	)
	if err := session.refreshOwnedWith(script); err != nil {
		t.Fatal(err)
	}
	script.finished(t, "refresh")
	assertSessionState(t, "refresh", session, pinned, true, clock)
}

// scanBeforeReap delegates to its script and records whether every group scan saw the exited
// leader still unreaped.
type scanBeforeReap struct {
	script    *observationScript
	leader    int64
	scans     int
	reapedErr string
}

var _ processObservations = (*scanBeforeReap)(nil)

func (s *scanBeforeReap) members(group int32) ([]int64, error) {
	if _, present, err := processIdentityRead(s.leader); err != nil || !present {
		s.reapedErr = fmt.Sprintf("the group scan ran after the leader was reaped (present=%v err=%v)", present, err)
	}
	s.scans++
	return s.script.members(group)
}

func (s *scanBeforeReap) identity(pid int64) (processIdentity, bool, error) {
	return s.script.identity(pid)
}

func (s *scanBeforeReap) stopped(pid int64) (bool, error) {
	return s.script.stopped(pid)
}

// liveLeader starts script as the leader of its own process group and returns a session that
// owns it as its native child; cleanup kills and reaps a leader the test left unreaped.
func liveLeader(t *testing.T, script string) (*ptySession, processIdentity) {
	t.Helper()
	child := exec.Command("/bin/sh", "-c", script)
	child.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	native := requireIdentity(t, int64(child.Process.Pid))
	t.Cleanup(func() {
		var status syscall.WaitStatus
		if reaped, err := syscall.Wait4(int(native.pid), &status, syscall.WNOHANG, nil); err == nil && reaped == 0 {
			_ = syscall.Kill(int(native.pid), syscall.SIGKILL)
			_, _ = syscall.Wait4(int(native.pid), &status, 0, nil)
		}
	})
	return &ptySession{
		clock:  systemClock{},
		child:  some(int(native.pid)),
		native: some(native),
		owned:  pinnedMap(native),
	}, native
}

// exitedLeader is liveLeader once the leader has exited unreaped.
func exitedLeader(t *testing.T, script string) (*ptySession, processIdentity) {
	t.Helper()
	session, native := liveLeader(t, script)
	waitUntilStopped(t, native.pid)
	return session, native
}

func TestRuntimeProcess_OwnershipRefreshTests_LeaderWaitTests_TryWaitWithALiveLeaderChecksOnlyTheLeaderAndScansNoGroup(t *testing.T) {
	t.Parallel()
	session, native := liveLeader(t, "exec /bin/sleep 30")
	for round := range 3 {
		script := newObservationScript(stoppedStep(native.pid, false))
		if exit, present, err := session.tryWaitWith(script); err != nil || present {
			t.Fatalf("round %d: tryWaitWith = %#v %v %v, want a running leader", round, exit, present, err)
		}
		script.finished(t, fmt.Sprintf("round %d", round))
	}
	if want := pinnedMap(native); !maps.Equal(session.owned, want) {
		t.Fatalf("owned %#v, want %#v", session.owned, want)
	}
	if _, ok := session.exit.get(); ok {
		t.Fatalf("a live leader recorded exit %#v", session.exit)
	}
	if _, ok := session.shutdownDeadlines.get(); ok {
		t.Fatal("a live leader begins no shutdown clock")
	}
	if session.groupClosed {
		t.Fatal("a live leader keeps the group open")
	}
	if _, present, err := processIdentityRead(native.pid); err != nil || !present {
		t.Fatalf("a live leader is not reaped: present=%v err=%v", present, err)
	}
	outcome := session.shutdown(shutdownDeadlinesStartingAt(time.Now(), shutdownUnit))
	if outcome.cleanupState != cleanupStateComplete {
		t.Fatalf("shutdown outcome %#v, want complete", outcome)
	}
	assertReaped(t, "live leader", native.pid)
}

func requireGroup(t *testing.T, pid int64) int32 {
	t.Helper()
	group, err := pidGroup(pid)
	if err != nil {
		t.Fatal(err)
	}
	return group
}

func assertExit(t *testing.T, name string, got childExit, present bool, err error, want childExit) {
	t.Helper()
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	if !present || got != want {
		t.Fatalf("%s: exit %#v (%v), want %#v", name, got, present, want)
	}
}

func TestRuntimeProcess_OwnershipRefreshTests_LeaderWaitTests_TryWaitWithAFailedLeaderCheckPropagatesWithoutAScanOrReap(t *testing.T) {
	t.Parallel()
	session, native := exitedLeader(t, "exit 0")
	failure := appError("synthetic-leader-check")
	script := newObservationScript(stoppedFailure(native.pid, failure))
	if _, _, err := session.tryWaitWith(script); err != failure {
		t.Fatalf("tryWaitWith returned %v, want the leader check failure itself", err)
	}
	script.finished(t, "failed leader check")
	if _, ok := session.exit.get(); ok {
		t.Fatalf("a failed leader check recorded exit %#v", session.exit)
	}
	if _, present, err := processIdentityRead(native.pid); err != nil || !present {
		t.Fatalf("a failed leader check does not reap: present=%v err=%v", present, err)
	}
	exit, present, err := session.tryWait()
	assertExit(t, "native try wait", exit, present, err, childExitCode{code: 0})
}

func TestRuntimeProcess_OwnershipRefreshTests_LeaderWaitTests_TryWaitWithAnExitedLeaderScansOncePinsLiveMembersThenReaps(t *testing.T) {
	t.Parallel()
	session, native := exitedLeader(t, "exit 3")
	group := requireGroup(t, native.pid)
	observations := &scanBeforeReap{
		script: newObservationScript(
			stoppedStep(native.pid, true),
			[]observationStep{{kind: observeMembers, group: group, members: []int64{native.pid, refreshB}}},
			observedStep(native),
			stoppedStep(native.pid, true),
			observedStep(syntheticIdentity(refreshB)),
			stoppedStep(refreshB, false),
			observedStep(native),
		),
		leader: native.pid,
	}
	exit, present, err := session.tryWaitWith(observations)
	assertExit(t, "exited leader", exit, present, err, childExitCode{code: 3})
	if observations.scans != 1 || observations.reapedErr != "" {
		t.Fatalf("scans = %d %s, want one scan before the reap", observations.scans, observations.reapedErr)
	}
	observations.script.finished(t, "exited leader")
	if want := pinnedMap(native, syntheticIdentity(refreshB)); !maps.Equal(session.owned, want) {
		t.Fatalf("owned %#v, want %#v", session.owned, want)
	}
	if session.groupClosed {
		t.Fatal("a live member keeps the group open")
	}
	if _, ok := session.shutdownDeadlines.get(); !ok {
		t.Fatal("the recorded exit begins the shutdown clock")
	}
	assertReaped(t, "exited leader", native.pid)
	settled := newObservationScript()
	exit, present, err = session.tryWaitWith(settled)
	assertExit(t, "recorded exit", exit, present, err, childExitCode{code: 3})
	settled.finished(t, "a recorded exit is returned without observing again")
}

func TestRuntimeProcess_OwnershipRefreshTests_LeaderWaitTests_TryWaitWithAnExitedLeaderAndNoLiveMemberClosesTheGroupThenReaps(t *testing.T) {
	t.Parallel()
	session, native := exitedLeader(t, "exit 0")
	group := requireGroup(t, native.pid)
	observations := &scanBeforeReap{
		script: newObservationScript(
			stoppedStep(native.pid, true),
			[]observationStep{{kind: observeMembers, group: group, members: []int64{native.pid}}},
			observedStep(native),
			stoppedStep(native.pid, true),
		),
		leader: native.pid,
	}
	exit, present, err := session.tryWaitWith(observations)
	assertExit(t, "exited leader", exit, present, err, childExitCode{code: 0})
	if observations.scans != 1 || observations.reapedErr != "" {
		t.Fatalf("scans = %d %s, want one scan before the reap", observations.scans, observations.reapedErr)
	}
	observations.script.finished(t, "exited leader")
	if !session.groupClosed {
		t.Fatal("a group without a live member closes")
	}
	if want := pinnedMap(native); !maps.Equal(session.owned, want) {
		t.Fatalf("owned %#v, want %#v", session.owned, want)
	}
	assertReaped(t, "exited leader", native.pid)
}

func TestRuntimeProcess_OwnershipRefreshTests_LeaderWaitTests_TryWaitWithAnUnidentifiedNativeReapsTheExitedChildWithoutObserving(t *testing.T) {
	t.Parallel()
	session, native := exitedLeader(t, "exit 5")
	session.native = none[processIdentity]()
	clear(session.owned)
	script := newObservationScript()
	exit, present, err := session.tryWaitWith(script)
	assertExit(t, "unidentified native", exit, present, err, childExitCode{code: 5})
	script.finished(t, "unidentified native")
	if len(session.owned) != 0 || session.groupClosed {
		t.Fatalf("an unidentified native pins nothing: owned=%#v closed=%v", session.owned, session.groupClosed)
	}
	if _, ok := session.shutdownDeadlines.get(); !ok {
		t.Fatal("the recorded exit begins the shutdown clock")
	}
	assertReaped(t, "unidentified native", native.pid)
}

func TestRuntimeProcess_ProcessTableTests_StatParser_ReadsTheSecondFieldAfterTheLastClosingParenthesis(t *testing.T) {
	t.Parallel()
	parent, err := parseStatParent(412, []byte("412 (we) ird (x)) S 77 412 412 0 -1 4194304\n"))
	if err != nil || parent != 77 {
		t.Fatalf("parent = %d %v, want 77", parent, err)
	}
}

func assertMalformedStat(t *testing.T, stat string) {
	t.Helper()
	_, err := parseStatParent(412, []byte(stat))
	if want := "malformed /proc/412/stat"; err == nil || err.Error() != want {
		t.Fatalf("%q: error %v, want %q", stat, err, want)
	}
}

func TestRuntimeProcess_ProcessTableTests_StatParser_MissingParentFieldIsMalformed(t *testing.T) {
	t.Parallel()
	assertMalformedStat(t, "412 (bash) S")
}

func TestRuntimeProcess_ProcessTableTests_StatParser_NonIntegerParentFieldIsMalformed(t *testing.T) {
	t.Parallel()
	assertMalformedStat(t, "412 (bash) S parent 412 412")
}

func TestRuntimeProcess_ProcessTableTests_StatParser_StatWithoutAClosingParenthesisIsMalformed(t *testing.T) {
	t.Parallel()
	assertMalformedStat(t, "412 bash S 77")
}

func assertCmdline(t *testing.T, cmdline string, want []string) {
	t.Helper()
	if got := parseCmdline([]byte(cmdline)); !slices.Equal(got, want) {
		t.Fatalf("%q: argv %q, want %q", cmdline, got, want)
	}
}

func TestRuntimeProcess_ProcessTableTests_CmdlineParser_SplitsOnNulAndDropsTheTrailingEmptySegment(t *testing.T) {
	t.Parallel()
	assertCmdline(t, "codex\x00app-server\x00--list\x00", []string{"codex", "app-server", "--list"})
}

func TestRuntimeProcess_ProcessTableTests_CmdlineParser_KeepsTheLastArgumentWithoutATrailingNul(t *testing.T) {
	t.Parallel()
	assertCmdline(t, "zsh\x00-l", []string{"zsh", "-l"})
}

func TestRuntimeProcess_ProcessTableTests_CmdlineParser_KeepsAnEmptyArgumentBeforeTheEnd(t *testing.T) {
	t.Parallel()
	assertCmdline(t, "sh\x00\x00-c\x00", []string{"sh", "", "-c"})
}

func TestRuntimeProcess_ProcessTableTests_CmdlineParser_EmptyCmdlineIsAnEmptyArgv(t *testing.T) {
	t.Parallel()
	assertCmdline(t, "", []string{})
}

func assertNested(t *testing.T, status string, want bool) {
	t.Helper()
	nested, err := parseNspidNested(status)
	if err != nil || nested != want {
		t.Fatalf("%q: nested = %v %v, want %v", status, nested, err, want)
	}
}

func TestRuntimeProcess_ProcessTableTests_NspidParser_ASinglePidIsNotNested(t *testing.T) {
	t.Parallel()
	assertNested(t, "Name:\tcafleet\nPid:\t4242\nNSpid:\t4242\nPPid:\t1\n", false)
}

func TestRuntimeProcess_ProcessTableTests_NspidParser_SeveralPidsAreNested(t *testing.T) {
	t.Parallel()
	assertNested(t, "Name:\tcafleet\nNSpid:\t4242\t17\nPPid:\t1\n", true)
}

func TestRuntimeProcess_ProcessTableTests_NspidParser_AnAbsentNspidLineIsNotNested(t *testing.T) {
	t.Parallel()
	assertNested(t, "Name:\tcafleet\nPid:\t4242\nPPid:\t1\n", false)
}

func TestRuntimeProcess_ProcessTableTests_NspidParser_ANonIntegerFieldIsMalformedWithTheLine(t *testing.T) {
	t.Parallel()
	_, err := parseNspidNested("Name:\tcafleet\nNSpid:\t4242\tx\nPPid:\t1\n")
	if want := "malformed /proc/self/status NSpid line: NSpid:\t4242\tx"; err == nil || err.Error() != want {
		t.Fatalf("error %v, want %q", err, want)
	}
}

func assertPsEntry(t *testing.T, output string, want processEntry) {
	t.Helper()
	entry, present, err := parsePsOutput(42, output)
	if err != nil || !present || entry.parent != want.parent || !slices.Equal(entry.argv, want.argv) {
		t.Fatalf("%q: entry %#v (%v) %v, want %#v", output, entry, present, err, want)
	}
}

func TestRuntimeProcess_ProcessTableTests_PsParser_ReadsTheParentAndTheWhitespaceSplitArgv(t *testing.T) {
	t.Parallel()
	assertPsEntry(t, "  1234 /bin/zsh -l  --login\n", processEntry{parent: 1234, argv: []string{"/bin/zsh", "-l", "--login"}})
}

func TestRuntimeProcess_ProcessTableTests_PsParser_ReadsTheFirstNonBlankLine(t *testing.T) {
	t.Parallel()
	assertPsEntry(t, "\n   \n 77 sh -c true\n", processEntry{parent: 77, argv: []string{"sh", "-c", "true"}})
}

func TestRuntimeProcess_ProcessTableTests_PsParser_BlankOutputIsNone(t *testing.T) {
	t.Parallel()
	for _, output := range []string{"", "  \n\t\n"} {
		entry, present, err := parsePsOutput(42, output)
		if err != nil || present {
			t.Fatalf("%q: entry %#v (%v) %v, want none", output, entry, present, err)
		}
	}
}

func assertPsError(t *testing.T, output, want string) {
	t.Helper()
	_, _, err := parsePsOutput(42, output)
	if err == nil || err.Error() != want {
		t.Fatalf("%q: error %v, want %q", output, err, want)
	}
}

func TestRuntimeProcess_ProcessTableTests_PsParser_ANonIntegerParentIsNoParent(t *testing.T) {
	t.Parallel()
	assertPsError(t, "abc /bin/sh\n", "ps printed no parent for 42: abc /bin/sh")
}

func TestRuntimeProcess_ProcessTableTests_PsParser_AParentWithoutACommandIsNoCommand(t *testing.T) {
	t.Parallel()
	assertPsError(t, "1234\n", "ps printed no command for 42: 1234")
}

func TestRuntimeProcess_ProcessTableTests_SystemTable_OwnPidIsThisProcess(t *testing.T) {
	t.Parallel()
	if got := (systemProcessTable{}).ownPID(); got != int64(os.Getpid()) {
		t.Fatalf("ownPID = %d, want %d", got, os.Getpid())
	}
}

func TestRuntimeProcess_ProcessTableTests_SystemTable_EntryForItsOwnPidReadsTheParentAndANonEmptyArgv(t *testing.T) {
	t.Parallel()
	table := systemProcessTable{}
	entry, present, err := table.entry(table.ownPID())
	if err != nil || !present {
		t.Fatalf("the running test process has an entry: %v %v", present, err)
	}
	if entry.parent != int64(os.Getppid()) || len(entry.argv) == 0 {
		t.Fatalf("entry %#v, want parent %d and a non-empty argv", entry, os.Getppid())
	}
}

func TestRuntimeProcess_ProcessTableTests_SystemTable_EntryForAReapedChildIsNone(t *testing.T) {
	t.Parallel()
	child := exec.Command("true")
	if err := child.Run(); err != nil {
		t.Fatal(err)
	}
	entry, present, err := (systemProcessTable{}).entry(int64(child.Process.Pid))
	if err != nil || present {
		t.Fatalf("a reaped child has entry %#v (%v) %v, want none", entry, present, err)
	}
}

func TestRuntimeProcess_ProcessTableTests_SystemTable_NestedPidNamespaceReadsWithoutError(t *testing.T) {
	t.Parallel()
	if _, err := (systemProcessTable{}).nestedPIDNamespace(); err != nil {
		t.Fatal(err)
	}
}

// ownedPty is a real PTY session that the test's cleanup shuts down unless the test already
// completed its cleanup.
type ownedPty struct {
	session   *ptySession
	deadlines opt[shutdownDeadlines]
	cleaned   bool
}

func prepareOwnedPty(t *testing.T, size terminalSize) *ownedPty {
	t.Helper()
	session, err := ptySessionPrepare(size, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	owned := &ownedPty{session: session}
	t.Cleanup(func() {
		if !owned.cleaned {
			owned.stop()
		}
	})
	return owned
}

func (o *ownedPty) stop() shutdownOutcome {
	deadlines, set := o.deadlines.get()
	if !set {
		deadlines = shutdownDeadlinesStartingAt(time.Now(), shutdownUnit)
		o.deadlines = some(deadlines)
	}
	outcome := o.session.shutdown(deadlines)
	o.cleaned = outcome.cleanupState == cleanupStateComplete
	return outcome
}

// ptyReadUntil reads the session's PTY until its output holds every needle.
func ptyReadUntil(t *testing.T, session *ptySession, needles ...string) string {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	var output []byte
	for {
		buffer := make([]byte, 1024)
		count, err := session.tryRead(buffer)
		if err != nil && !errors.Is(err, syscall.EAGAIN) {
			t.Fatalf("fixture PTY read failed: %v", err)
		}
		output = append(output, buffer[:count]...)
		text := string(output)
		if !slices.ContainsFunc(needles, func(needle string) bool { return !strings.Contains(text, needle) }) {
			return text
		}
		if !time.Now().Before(deadline) {
			t.Fatalf("fixture output lacks %q: %q", needles, text)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// ptyCollect reads the session's PTY until the child has exited and its output is drained.
func ptyCollect(t *testing.T, session *ptySession) ([]byte, childExit) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	var output []byte
	var status childExit
	read := func(stage string) int {
		buffer := make([]byte, 4096)
		count, err := session.tryRead(buffer)
		if errors.Is(err, syscall.EAGAIN) {
			return -1
		}
		if err != nil {
			t.Fatalf("fixture PTY %s failed: %v", stage, err)
		}
		output = append(output, buffer[:count]...)
		return count
	}
	for {
		if read("read") == 0 && status != nil {
			return output, status
		}
		if status == nil {
			exit, exited, err := session.tryWait()
			if err != nil {
				t.Fatal(err)
			}
			if exited {
				status = exit
			}
		}
		if status != nil && read("tail") == 0 {
			return output, status
		}
		if !time.Now().Before(deadline) {
			t.Fatal("fixture child did not exit/drain within its deadline")
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// announcedPID is the PID the fixture printed on its `<prefix><pid>` line.
func announcedPID(t *testing.T, output, prefix string) int64 {
	t.Helper()
	for line := range strings.Lines(output) {
		if value, found := strings.CutPrefix(strings.TrimSpace(line), prefix); found {
			pid, err := strconv.ParseInt(strings.TrimSpace(value), 10, 64)
			if err != nil {
				t.Fatal(err)
			}
			return pid
		}
	}
	t.Fatalf("fixture output %q announces no %s PID", output, prefix)
	return 0
}

func assertCleanupComplete(t *testing.T, name string, outcome shutdownOutcome) {
	t.Helper()
	if outcome.cleanupState != cleanupStateComplete {
		t.Fatalf("%s: shutdown outcome %#v, want complete", name, outcome)
	}
}

func TestRuntimeProcess_MechanismsTests_RunnerRealPtyPreparesWithoutAChildAndSpawnsLiteralArgvCwdAndEnvironmentOnce(t *testing.T) {
	t.Parallel()
	const literal = "assignment '$x' `backticks` 世界"
	path := os.Getenv("PATH")
	if path == "" {
		t.Fatal("the fixture inherits the host execution path")
	}
	spec := newRunnerFixture(t).spec()
	spec.argv = []string{
		"/bin/sh", "-c",
		`printf '%s\n' "$1"; printf '%s\n' "$CAFLEET_FIXTURE_MARKER"; if [ "$PATH" = "$2" ]; then printf 'inherited-path\n'; else exit 11; fi; pwd; exit 7`,
		"cafleet-fixture", literal, path,
	}
	spec.environment = append(spec.environment, [2]string{"CAFLEET_FIXTURE_MARKER", "literal-marker-$x"})
	owned := prepareOwnedPty(t, spec.size)
	if exit, present, err := owned.session.tryWait(); err != nil || present {
		t.Fatalf("a prepared PTY without a child waits %#v %v %v", exit, present, err)
	}
	process, err := owned.session.spawn(spec)
	if err != nil {
		t.Fatal(err)
	}
	if process.pid <= 0 || process.startIdentity == "" {
		t.Fatalf("spawned process identity %#v", process)
	}
	if _, err := owned.session.spawn(spec); err == nil {
		t.Fatal("a second spawn on the same session succeeds")
	}
	output, status := ptyCollect(t, owned.session)
	assertCleanupComplete(t, "spawn once", owned.stop())
	cwd, err := filepath.EvalSymlinks(spec.cwd)
	if err != nil {
		t.Fatal(err)
	}
	text := string(output)
	for _, want := range []string{literal, "literal-marker-$x", "inherited-path", cwd} {
		if !strings.Contains(text, want) {
			t.Fatalf("child output %q lacks %q", text, want)
		}
	}
	if status != childExit(childExitCode{code: 7}) {
		t.Fatalf("native exit %#v, want code 7", status)
	}
}

func TestRuntimeProcess_MechanismsTests_RunnerRealPtyChildReceivesTheForwardedConfigLocation(t *testing.T) {
	t.Parallel()
	spec := newRunnerFixture(t).spec()
	forwarded, err := codingAgentOpencode.forwardedConfig(func(name string) (opt[string], error) {
		if name == "OPENCODE_CONFIG_DIR" {
			return some("/cfg/open code"), nil
		}
		return none[string](), nil
	})
	if err != nil {
		t.Fatal(err)
	}
	entry, set := forwarded.get()
	if !set {
		t.Fatal("the set configuration variable is forwarded")
	}
	spec.environment = [][2]string{entry}
	spec.argv = []string{"/bin/sh", "-c", `printf 'config=%s\n' "$OPENCODE_CONFIG_DIR"`}
	owned := prepareOwnedPty(t, spec.size)
	if _, err := owned.session.spawn(spec); err != nil {
		t.Fatal(err)
	}
	output, status := ptyCollect(t, owned.session)
	assertCleanupComplete(t, "forwarded config", owned.stop())
	if text := string(output); !strings.Contains(text, "config=/cfg/open code") {
		t.Fatalf("child output %q lacks the forwarded config location", text)
	}
	if status != childExit(childExitCode{code: 0}) {
		t.Fatalf("native exit %#v, want code 0", status)
	}
}

func TestRuntimeProcess_MechanismsTests_RunnerRealPtyRetainsTheNumericNativeSignalExit(t *testing.T) {
	t.Parallel()
	spec := newRunnerFixture(t).spec()
	spec.argv = []string{"/bin/sh", "-c", "kill -TERM $$"}
	owned := prepareOwnedPty(t, spec.size)
	if _, err := owned.session.spawn(spec); err != nil {
		t.Fatal(err)
	}
	_, status := ptyCollect(t, owned.session)
	assertCleanupComplete(t, "signal exit", owned.stop())
	if status != childExit(childExitSignal{signal: 15}) {
		t.Fatalf("native exit %#v, want signal 15", status)
	}
}

func assertTerminalRestored(t *testing.T, name string, terminal *os.File, expected unix.Termios) {
	t.Helper()
	actual, err := getTermios(int(terminal.Fd()))
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS == "darwin" {
		// macOS sets PENDIN when TCSANOW restores canonical input.
		actual.Lflag &^= unix.PENDIN
		expected.Lflag &^= unix.PENDIN
	}
	if *actual != expected {
		t.Fatalf("%s: terminal attributes %#v, want %#v", name, *actual, expected)
	}
}

func TestRuntimeProcess_MechanismsTests_RunnerRealRawTerminalGuardRestoresOnlyItsOwnedFixtureTerminal(t *testing.T) {
	t.Parallel()
	master, terminal, err := pty.Open()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		closeFile(terminal)
		closeFile(master)
	})
	before, err := getTermios(int(terminal.Fd()))
	if err != nil {
		t.Fatal(err)
	}
	guard, err := rawTerminalGuardEnter(terminal)
	if err != nil {
		t.Fatal(err)
	}
	defer guard.close()
	raw, err := getTermios(int(terminal.Fd()))
	if err != nil {
		t.Fatal(err)
	}
	if raw.Lflag&unix.ICANON != 0 || raw.Lflag&unix.ECHO != 0 {
		t.Fatalf("raw mode keeps canonical input or echo: %#v", raw.Lflag)
	}
	for _, stage := range []string{"restore", "repeated restore"} {
		if err := guard.restore(); err != nil {
			t.Fatalf("%s: %v", stage, err)
		}
		assertTerminalRestored(t, stage, terminal, *before)
	}
	enterAndClose := func() {
		scoped, err := rawTerminalGuardEnter(terminal)
		if err != nil {
			t.Fatal(err)
		}
		defer scoped.close()
	}
	enterAndClose()
	assertTerminalRestored(t, "scoped guard", terminal, *before)
	unwound := func() (recovered bool) {
		defer func() { recovered = recover() != nil }()
		unwinding, err := rawTerminalGuardEnter(terminal)
		if err != nil {
			t.Fatal(err)
		}
		defer unwinding.close()
		panic("fixture unwind")
	}()
	if !unwound {
		t.Fatal("the fixture panic unwinds")
	}
	assertTerminalRestored(t, "unwound guard", terminal, *before)
}

// lateShutdownDeadlines start 1.7 units before now, leaving 0.3 units until the child deadline.
func lateShutdownDeadlines(start time.Time) shutdownDeadlines {
	return shutdownDeadlinesStartingAt(start.Add(-1700*time.Millisecond), shutdownUnit)
}

func TestRuntimeProcess_MechanismsTests_RunnerRealShutdownKillsAndReapsATermIgnoringOwnedChildWithinTheSharedDeadline(t *testing.T) {
	t.Parallel()
	spec := newRunnerFixture(t).spec()
	spec.argv = []string{"/bin/sh", "-c", "trap '' TERM; printf 'ready\\n'; while :; do sleep 1; done"}
	owned := prepareOwnedPty(t, spec.size)
	process, err := owned.session.spawn(spec)
	if err != nil {
		t.Fatal(err)
	}
	ptyReadUntil(t, owned.session, "ready")
	start := time.Now()
	owned.deadlines = some(lateShutdownDeadlines(start))
	outcome := owned.stop()
	if elapsed := time.Since(start); elapsed >= 4750*time.Millisecond {
		t.Fatalf("cleanup took %v: %#v", elapsed, outcome)
	}
	assertCleanupComplete(t, "TERM-ignoring child", outcome)
	if outcome.nativeExit != some[childExit](childExitSignal{signal: 9}) {
		t.Fatalf("native exit %#v, want signal 9", outcome.nativeExit)
	}
	assertReaped(t, "TERM-ignoring child", process.pid)
}

func TestRuntimeProcess_MechanismsTests_RunnerRealShutdownClosesASlaveRetainingDescendantAndPreservesTheNativeZeroExit(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	ready := filepath.Join(fixture.dir, "descendant-ready")
	spec := fixture.spec()
	// The descendant must not fork: a new group member appearing during KILL enumeration is unpinned.
	spec.argv = []string{
		"/bin/sh", "-c",
		`/bin/sh -c 'trap "" TERM HUP; printf ready > "$1"; exec /bin/sleep 600' descendant "$1" & descendant=$!; while [ ! -f "$1" ]; do sleep 0.01; done; printf 'descendant=%s\n' "$descendant"; exit 0`,
		"parent", ready,
	}
	owned := prepareOwnedPty(t, spec.size)
	native, err := owned.session.spawn(spec)
	if err != nil {
		t.Fatal(err)
	}
	pid := announcedPID(t, ptyReadUntil(t, owned.session, "\n"), "descendant=")
	waitDeadline := time.Now().Add(5 * time.Second)
	for {
		exit, exited, err := owned.session.tryWait()
		if err != nil {
			t.Fatalf("native %#v with descendant %d wait failed: %v", native, pid, err)
		}
		if exited {
			if exit != childExit(childExitCode{code: 0}) {
				t.Fatalf("native %#v exit %#v, want code 0", native, exit)
			}
			break
		}
		if !time.Now().Before(waitDeadline) {
			t.Fatalf("native %#v with descendant %d did not exit", native, pid)
		}
		time.Sleep(5 * time.Millisecond)
	}
	start := time.Now()
	owned.deadlines = some(lateShutdownDeadlines(start))
	outcome := owned.stop()
	if elapsed := time.Since(start); elapsed >= 4750*time.Millisecond {
		t.Fatalf("cleanup took %v: %#v", elapsed, outcome)
	}
	if outcome.nativeExit != some[childExit](childExitCode{code: 0}) {
		t.Fatalf("native exit %#v, want code 0", outcome.nativeExit)
	}
	assertCleanupComplete(t, "slave-retaining descendant", outcome)
	observation, err := exec.Command("/bin/ps", "-o", "stat=", "-p", strconv.FormatInt(pid, 10)).Output()
	var exitErr *exec.ExitError
	if err != nil && (!errors.As(err, &exitErr) || exitErr.ExitCode() != 1) {
		t.Fatal(err)
	}
	if state := strings.TrimSpace(string(observation)); state != "" && !strings.HasPrefix(state, "Z") {
		t.Fatalf("owned descendant remains running: %s", state)
	}
}
