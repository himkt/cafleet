package core

import (
	"errors"
	"fmt"
	"maps"
	"os"
	"os/exec"
	"slices"
	"syscall"
	"testing"
	"time"
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

type observationKind uint8

const (
	observeMembers observationKind = iota + 1
	observeIdentity
	observeStopped
)

type observationStep struct {
	kind     observationKind
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

func (s *observationScript) next(kind observationKind, subject string) (observationStep, bool) {
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

// exitedLeader starts script as the leader of its own process group, waits until it has
// exited unreaped and returns a session that owns it as its native child.
func exitedLeader(t *testing.T, script string) (*ptySession, processIdentity) {
	t.Helper()
	child := exec.Command("/bin/sh", "-c", script)
	child.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	native := requireIdentity(t, int64(child.Process.Pid))
	t.Cleanup(func() {
		var status syscall.WaitStatus
		_, _ = syscall.Wait4(int(native.pid), &status, syscall.WNOHANG, nil)
	})
	waitUntilStopped(t, native.pid)
	return &ptySession{
		clock:  systemClock{},
		child:  some(int(native.pid)),
		native: some(native),
		owned:  pinnedMap(native),
	}, native
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
