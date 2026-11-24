package core

import (
	"context"
	"fmt"
	"os/exec"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
)

type deletionMutation uint8

const (
	deletionMutationNone deletionMutation = iota + 1
	deletionMutationAfterShutdown
	deletionMutationBeforeRecovery
	deletionMutationAfterRecovery
	deletionMutationAfterClose
	deletionMutationOtherPlacement
)

const deletionSettlementTime = 30 * time.Millisecond

// deletionTestHost is the Rust fleet test_support Host. A fleet host ends each run itself and
// keeps production recovery; a recovering host (recovery set) leaves the run for the recovery
// coordinator and replays it through recovery on the deleting connection.
type deletionTestHost struct {
	t             testing.TB
	recovery      *recoveryTestHost
	connection    *dbConn
	settings      settings
	deadline      time.Time
	runID         int64
	mutation      deletionMutation
	otherMember   int64
	calls         []string
	owners        [][2]int64
	panes         []placement
	failingPane   opt[string]
	restored      int
	recovered     *runRecord
	runnerGone    bool
	hermesFailure opt[string]
	settled       opt[time.Duration]
	layoutWaited  bool
}

func newFleetDeletionHost(t testing.TB, s settings, runID int64, d deadline) *deletionTestHost {
	return &deletionTestHost{t: t, settings: s, deadline: d.instant(), runID: runID, mutation: deletionMutationNone,
		failingPane: none[string](), hermesFailure: none[string](), settled: none[time.Duration]()}
}

func newRecoveringDeletionHost(t testing.TB, c *recoveryCase, d deadline) *deletionTestHost {
	host := newFleetDeletionHost(t, c.settings, c.fixture.run.runID, d)
	host.recovery = newRecoveryHostOnRealTime(t, d.instant())
	host.connection = c.fixture.conn
	return host
}

// assertDeadline checks every cleanup step shares the deletion deadline, which the fleet layout
// lock wait before the first step and a settlement each extend by the time they took.
func (h *deletionTestHost) assertDeadline(d time.Time) {
	settlement, settled := h.settled.get()
	h.settled = none[time.Duration]()
	firstStep := !h.layoutWaited
	h.layoutWaited = true
	if settled || firstStep {
		extension := max(d.Sub(h.deadline), 0)
		if extension < settlement || extension >= settlement+time.Second {
			h.t.Fatalf("deadline extension %v after a %v settlement", extension, settlement)
		}
		h.deadline = d
		if h.recovery != nil {
			h.recovery.deadline = d
		}
	}
	if !d.Equal(h.deadline) {
		h.t.Fatalf("step deadline %v, want the shared %v", d, h.deadline)
	}
}

func (h *deletionTestHost) assertPaneOwned(p placement) {
	lock, acquired, err := paneLockTryAcquire(h.settings.databaseURL, p)
	if err != nil {
		h.t.Fatal(err)
	}
	if acquired {
		lock.close()
		h.t.Fatal("deletion holds the step's pane lock")
	}
}

func (h *deletionTestHost) assertConnection(conn *dbConn) {
	if h.recovery != nil && conn != h.connection {
		h.t.Fatal("recovery runs on the deleting connection")
	}
}

func (h *deletionTestHost) memberPlacement(ctx context.Context, conn *dbConn, run runRecord) placement {
	found, err := getMember(ctx, conn, run.memberID, run.fleetID)
	if err != nil {
		h.t.Fatal(err)
	}
	member, present := found.get()
	if !present {
		h.t.Fatal("the run's member is registered")
	}
	return member.placement
}

func deletionBumpEpoch(t testing.TB, conn *dbConn, runID int64) {
	if changed, err := sqlExecute(conn, "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", runID); err != nil || changed != 1 {
		t.Fatalf("bumping the epoch changed %d rows: %v", changed, err)
	}
}

func (h *deletionTestHost) settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, d deadline) error {
	h.assertConnection(conn)
	budget := time.Until(d.instant())
	if budget <= 19*time.Second || budget > 20*time.Second {
		h.t.Fatalf("each Hermes member gets its own settlement budget: %v", budget)
	}
	if !d.instant().After(h.deadline) {
		h.t.Fatal("the settlement budget extends past the deletion deadline")
	}
	p := h.memberPlacement(ctx, conn, run)
	if p.muxPaneID != some(paneID) {
		h.t.Fatalf("settlement pane %q, placement %v", paneID, p.muxPaneID)
	}
	h.assertPaneOwned(p)
	h.calls = append(h.calls, "settle")
	time.Sleep(deletionSettlementTime)
	h.settled = some(deletionSettlementTime)
	if cause, failed := h.hermesFailure.get(); failed {
		return appError(cause)
	}
	return nil
}

func (h *deletionTestHost) shutdown(ctx context.Context, conn *dbConn, run runRecord, d deadline) (shutdownOutcome, error) {
	h.assertConnection(conn)
	h.assertDeadline(d.instant())
	h.assertPaneOwned(h.memberPlacement(ctx, conn, run))
	h.calls = append(h.calls, "shutdown")
	if h.runnerGone {
		return shutdownOutcome{}, appError("member runtime: ECONNREFUSED: Connection refused")
	}
	nativeExit := none[childExit]()
	if h.recovery == nil {
		h.owners = append(h.owners, [2]int64{run.runID, run.ownerEpoch})
		h.runID = run.runID
		mustExecute(h.t, conn, "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T13:00:00Z',last_health_at='2026-10-26T13:00:00Z' WHERE run_id=?1", run.runID)
		nativeExit = some[childExit](childExitCode{code: 0})
	} else {
		mustExecute(h.t, conn, "UPDATE member_runs SET cleanup_state='complete' WHERE run_id=?1", run.runID)
	}
	if h.mutation == deletionMutationAfterShutdown {
		deletionBumpEpoch(h.t, conn, run.runID)
	}
	return shutdownOutcomeComplete(nativeExit), nil
}

func mustExecute(t testing.TB, conn *dbConn, sql string, args ...any) {
	if _, err := sqlExecute(conn, sql, args...); err != nil {
		t.Fatal(err)
	}
}

func (h *deletionTestHost) recover(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, held *paneLock, d deadline) (recoveryCompletion, error) {
	if h.recovery == nil {
		return runtimeDeletionSteps{}.recover(ctx, conn, s, expectedRun, expectedPlacement, held, d)
	}
	h.assertConnection(conn)
	h.assertDeadline(d.instant())
	h.assertPaneOwned(expectedPlacement)
	if err := held.validateBinding(s.databaseURL, expectedPlacement); err != nil {
		h.t.Fatal(err)
	}
	h.calls = append(h.calls, "recover")
	if h.mutation == deletionMutationBeforeRecovery {
		deletionBumpEpoch(h.t, conn, expectedRun.runID)
	}
	completion, err := recoverMemberWithHeldPaneLock(ctx, conn, s, expectedRun, expectedPlacement, d.instant(), held, h.recovery)
	if err != nil {
		return recoveryCompletion{}, err
	}
	stored, err := getRun(ctx, conn, expectedRun.runID)
	if err != nil {
		h.t.Fatal(err)
	}
	if !completion.run.equal(stored) {
		h.t.Fatal("the completion carries the stored run")
	}
	h.recovered = &completion.run
	switch h.mutation {
	case deletionMutationAfterRecovery:
		deletionBumpEpoch(h.t, conn, expectedRun.runID)
	case deletionMutationOtherPlacement:
		mustExecute(h.t, conn, "UPDATE member_placements SET mux_pane_id='%foreign', mux_window_id=COALESCE(mux_window_id, '@1'), mux_window_name=COALESCE(mux_window_name, 'main') WHERE member_id=?1", h.otherMember)
	case deletionMutationNone, deletionMutationAfterShutdown, deletionMutationBeforeRecovery, deletionMutationAfterClose:
	default:
		panic("invalid deletionMutation")
	}
	return completion, nil
}

func (h *deletionTestHost) closePane(p placement, d deadline) error {
	h.assertDeadline(d.instant())
	h.assertPaneOwned(p)
	h.calls = append(h.calls, "close")
	h.panes = append(h.panes, p)
	if failing, set := h.failingPane.get(); set && p.muxPaneID == some(failing) {
		return appError("pane vanished")
	}
	if h.mutation == deletionMutationAfterClose {
		runID := h.runID
		if h.recovery != nil {
			recovered, proven := h.recovered, h.recovered != nil
			if !proven {
				h.t.Fatal("close follows proven recovery")
			}
			runID = recovered.runID
		}
		side, err := connect(h.settings.databaseURL)
		if err != nil {
			h.t.Fatal(err)
		}
		deletionBumpEpoch(h.t, side, runID)
		if err := side.close(); err != nil {
			h.t.Fatal(err)
		}
	}
	return nil
}

func (h *deletionTestHost) restoreLayout() error {
	h.restored++
	return nil
}

func fleetDeleted(t *testing.T, f *runnerFixture) bool {
	t.Helper()
	fleet, err := requireFleet(context.Background(), f.conn, f.run.fleetID)
	if err != nil {
		t.Fatal(err)
	}
	return fleet.deletedAt.set
}

func assertRegistryActive(t *testing.T, f *runnerFixture) {
	t.Helper()
	if fleetDeleted(t, f) {
		t.Fatal("the fleet stays active")
	}
	found, err := getMember(context.Background(), f.conn, f.run.memberID, f.run.fleetID)
	if err != nil {
		t.Fatal(err)
	}
	if _, present := found.get(); !present {
		t.Fatal("the member stays registered")
	}
}

// fixtureDeletionHost serves broker fixtures, which register no member runs.
type fixtureDeletionHost struct{}

func (fixtureDeletionHost) recover(context.Context, *dbConn, settings, runRecord, placement, *paneLock, deadline) (recoveryCompletion, error) {
	panic("broker fixtures register no member runs")
}

func (fixtureDeletionHost) shutdown(context.Context, *dbConn, runRecord, deadline) (shutdownOutcome, error) {
	panic("broker fixtures register no member runs")
}

func (fixtureDeletionHost) settleHermes(context.Context, *dbConn, runRecord, string, deadline) error {
	panic("broker fixtures register no member runs")
}

func (fixtureDeletionHost) closePane(placement, deadline) error { return nil }

func (fixtureDeletionHost) restoreLayout() error { return nil }

func guardedDeleteFleet(t *testing.T, conn *dbConn, url string, fleet int64) (deletedFleet, error) {
	t.Helper()
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	return deleteFleetWithHost(context.Background(), conn, settingsForDatabase(path), fleet, newDeadline(), fixtureDeletionHost{})
}

func fixtureSettings(t *testing.T, f *runnerFixture) settings {
	t.Helper()
	return settingsForDatabase(f.databaseFile(t))
}

func errorMessage(err error) string {
	return cafleetErrorOf(err).message()
}

func TestCliFleet_DeletionRows_FleetDeletionIsAtomicSoftIdempotentAndPreservesMessages(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "alpha")
	worker := register(t, conn, fleet, "worker", none[string]())
	message, err := sendMessage(ctx, conn, director, worker, "retained history")
	if err != nil {
		t.Fatal(err)
	}
	before, err := getFleet(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	members := []memberRecord{requireMember(t, conn, director, fleet), requireMember(t, conn, worker, fleet)}
	mustExecBatch(t, conn, "CREATE TRIGGER fail_deletion BEFORE DELETE ON member_placements BEGIN SELECT RAISE(ABORT, 'deletion failure'); END;")
	if _, err := guardedDeleteFleet(t, conn, url, fleet); err == nil || !strings.Contains(errorMessage(err), "deletion failure") {
		t.Fatalf("the injected failure fails deletion: %v", err)
	}
	if after, _ := getFleet(ctx, conn, fleet); after != before {
		t.Fatal("the failed deletion leaves the fleet unchanged")
	}
	for _, member := range members {
		if current := requireMember(t, conn, member.memberID, fleet); !memberRecordsEqual(current, member) {
			t.Fatalf("member %d changed", member.memberID)
		}
		if status := queryText(t, conn, "SELECT status FROM members WHERE member_id=?1", member.memberID); status != "active" {
			t.Fatalf("member %d is %s", member.memberID, status)
		}
	}
	mustExecBatch(t, conn, "DROP TRIGGER fail_deletion")
	deleted, err := guardedDeleteFleet(t, conn, url, fleet)
	if err != nil || deleted.deregisteredCount != 2 {
		t.Fatalf("deletion deregisters both members: %+v %v", deleted, err)
	}
	if count := queryInt64(t, conn, "SELECT COUNT(*) FROM member_placements"); count != 0 {
		t.Fatalf("%d placements remain", count)
	}
	afterDeletion, _ := getFleet(ctx, conn, fleet)
	again, err := guardedDeleteFleet(t, conn, url, fleet)
	if err != nil || again.deregisteredCount != 0 {
		t.Fatalf("a repeated deletion deregisters nothing: %+v %v", again, err)
	}
	if current, _ := getFleet(ctx, conn, fleet); current != afterDeletion {
		t.Fatal("a repeated deletion leaves the deleted fleet unchanged")
	}
	if retained, err := getMessage(ctx, conn, message.messageID); err != nil || retained != message {
		t.Fatalf("the message is retained: %v", err)
	}
	if _, err := guardedDeleteFleet(t, conn, url, 999); err == nil {
		t.Fatal("an unknown fleet fails deletion")
	}
}

func memberRecordsEqual(left, right memberRecord) bool {
	return left.memberID == right.memberID && left.fleetID == right.fleetID && left.kind == right.kind &&
		left.placement == right.placement && sameRunIdentity(optionalRunIdentity(left.runtime), optionalRunIdentity(right.runtime))
}

func TestCliFleet_FleetCreateAcceptsEveryRegisteredBackendIncludingHermes(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		if _, exit := parseCli("fleet", "create", "--name", "fixture", "--coding-agent", agent.name()); exit != nil {
			t.Fatalf("%s: %v", agent.name(), exit)
		}
	}
}

func lockOrderedPlacements(t *testing.T, conn *dbConn, s settings, fleet int64, exclude opt[int64]) []memberRecord {
	t.Helper()
	activities, err := listMembers(context.Background(), conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	members := []memberRecord{}
	for _, activity := range activities {
		if excluded, set := exclude.get(); set && activity.member.memberID == excluded {
			continue
		}
		if activity.member.placement.muxPaneID.set {
			members = append(members, activity.member)
		}
	}
	path := func(member memberRecord) string {
		lock, err := lockPath(s.databaseURL, member.placement)
		if err != nil {
			t.Fatal(err)
		}
		return lock
	}
	slices.SortFunc(members, func(a, b memberRecord) int { return strings.Compare(path(a), path(b)) })
	return members
}

func tryPaneLock(t *testing.T, s settings, p placement) (*paneLock, bool) {
	t.Helper()
	lock, acquired, err := paneLockTryAcquire(s.databaseURL, p)
	if err != nil {
		t.Fatal(err)
	}
	return lock, acquired
}

type deletionResult struct {
	deleted deletedFleet
	err     error
	host    *deletionTestHost
}

// whileLocksAreAcquired starts a fleet deletion on its own connection while the test holds the
// last pane lock in lock order, runs change once the deletion owns its first lock, then releases
// the last lock.
func whileLocksAreAcquired(t *testing.T, f *runnerFixture, change func(s settings) *paneLock) deletionResult {
	t.Helper()
	s := fixtureSettings(t, f)
	placements := lockOrderedPlacements(t, f.conn, s, f.run.fleetID, none[int64]())
	if len(placements) < 2 {
		t.Fatalf("the fleet has %d placed members", len(placements))
	}
	first := placements[0].placement
	last, acquired := tryPaneLock(t, s, placements[len(placements)-1].placement)
	if !acquired {
		t.Fatal("the last pane lock is free")
	}
	started := time.Now()
	d := deadlineUntil(started.Add(time.Second))
	results := make(chan deletionResult, 1)
	go func() {
		conn, err := connect(f.url)
		if err != nil {
			results <- deletionResult{err: err}
			return
		}
		defer conn.close() //nolint:errcheck // the deleting goroutine's own connection
		host := newFleetDeletionHost(t, s, f.run.runID, d)
		deleted, err := deleteFleetWithHost(context.Background(), conn, s, f.run.fleetID, d, host)
		results <- deletionResult{deleted: deleted, err: err, host: host}
	}()
	for {
		probe, free := tryPaneLock(t, s, first)
		if !free {
			break
		}
		probe.close()
		if time.Since(started) >= 700*time.Millisecond {
			t.Fatal("deletion did not acquire its first physical lock")
		}
		time.Sleep(2 * time.Millisecond)
	}
	changed := change(s)
	last.close()
	result := <-results
	if changed != nil {
		changed.close()
	}
	if time.Since(started) >= 1750*time.Millisecond {
		t.Fatalf("deletion took %v", time.Since(started))
	}
	return result
}

func TestCliFleet_Fleet_RunnerReviewFleetChangedPlacementIsRelockedBeforeAnyDestructiveEffect(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	result := whileLocksAreAcquired(t, f, func(s settings) *paneLock {
		mustExecute(t, f.conn, "UPDATE member_placements SET mux_pane_id='%changed' WHERE member_id=?1", f.run.memberID)
		lock, acquired := tryPaneLock(t, s, requireMember(t, f.conn, f.run.memberID, f.run.fleetID).placement)
		if !acquired {
			t.Fatal("the changed placement's lock is free")
		}
		return lock
	})
	if result.err == nil {
		t.Fatal("a changed placement fails deletion")
	}
	if len(result.host.calls) != 0 {
		t.Fatalf("stale deletion effects: %v", result.host.calls)
	}
	assertRegistryActive(t, f)
}

func TestCliFleet_Fleet_RunnerReviewFleetChangedRunOnTheSamePanePreservesTheNewRunIdentity(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	oldRun := f.run
	newRunID := none[int64]()
	result := whileLocksAreAcquired(t, f, func(settings) *paneLock {
		mustExecute(t, f.conn, "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", oldRun.runID)
		process := currentProcess(t)
		spec := brokerRuntimeFixtureSpec(t, f.conn, f.url, oldRun.fleetID, oldRun.directorMemberID, oldRun.memberID)
		spec.launchNonce = strings.Repeat("b", 64)
		spec.runtimePath = f.dir + "/replacement-run"
		spec.ownerPID = process.pid
		spec.ownerStartIdentity = process.startIdentity
		created, err := createRun(context.Background(), f.conn, spec)
		if err != nil {
			t.Error(err)
			return nil
		}
		newRunID = some(created.runID)
		return nil
	})
	if result.err != nil {
		t.Fatal(result.err)
	}
	replacement, created := newRunID.get()
	if !created || !slices.Equal(result.host.owners, [][2]int64{{replacement, 1}}) || replacement == oldRun.runID {
		t.Fatalf("owners %v, new run %v", result.host.owners, newRunID)
	}
	if !slices.Equal(result.host.calls, []string{"shutdown", "close"}) {
		t.Fatalf("calls %v", result.host.calls)
	}
}

func deleteFixtureFleet(t *testing.T, f *runnerFixture, configure func(*deletionTestHost)) (deletedFleet, *deletionTestHost, error) {
	t.Helper()
	s := fixtureSettings(t, f)
	d := newDeadline()
	host := newFleetDeletionHost(t, s, f.run.runID, d)
	if configure != nil {
		configure(host)
	}
	deleted, err := deleteFleetWithHost(context.Background(), f.conn, s, f.run.fleetID, d, host)
	return deleted, host, err
}

func TestCliFleet_Fleet_RunnerReviewFleetOwnerChangeDuringShutdownStopsBeforePaneClose(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	_, host, err := deleteFixtureFleet(t, f, func(h *deletionTestHost) { h.mutation = deletionMutationAfterShutdown })
	if err == nil || !slices.Equal(host.calls, []string{"shutdown"}) {
		t.Fatalf("calls %v err %v", host.calls, err)
	}
	assertRegistryActive(t, f)
}

func TestCliFleet_Fleet_HermesMemberDeletionSettlesBeforeShutdownAndAnUnverifiedStopKeepsTheMember(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		failure opt[string]
	}{
		{"a verified stop deletes the member", none[string]()},
		{"an unverified stop keeps the member", some("hermes work for member 2 could not be verified as stopped: Hermes did not report /stop; member was not deleted.")},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			f := newRunnerFixture(t)
			mustExecute(t, f.conn, "UPDATE member_runs SET backend='hermes' WHERE run_id=?1", f.run.runID)
			_, host, err := deleteFixtureFleet(t, f, func(h *deletionTestHost) { h.hermesFailure = tc.failure })
			cause, failing := tc.failure.get()
			if !failing {
				if err != nil {
					t.Fatal(err)
				}
				if !slices.Equal(host.calls, []string{"settle", "shutdown", "close"}) {
					t.Fatalf("calls %v", host.calls)
				}
				return
			}
			if err == nil {
				t.Fatal(cause)
			}
			if cafleetErrorOf(err).exitCode() != 1 || !strings.Contains(errorMessage(err), cause) {
				t.Fatalf("error %v", err)
			}
			if !slices.Equal(host.calls, []string{"settle"}) {
				t.Fatalf("no shutdown after an unverified stop: %v", host.calls)
			}
			assertRegistryActive(t, f)
			if mustRun(t, f.conn, f.run.runID).status.terminal() {
				t.Fatal("the runner keeps running for a retry")
			}
		})
	}
}

func TestCliFleet_Fleet_RunnerReviewFleetFinalIdentityChangePreservesRegistryWithoutRepeatingEffects(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	_, host, err := deleteFixtureFleet(t, f, func(h *deletionTestHost) { h.mutation = deletionMutationAfterClose })
	if err == nil || !slices.Equal(host.calls, []string{"shutdown", "close"}) {
		t.Fatalf("calls %v err %v", host.calls, err)
	}
	assertRegistryActive(t, f)
}

func TestCliFleet_Fleet_RunnerReviewFleetMutableShutdownEvidenceKeepsOneDeadlineAndRunLessTeardown(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	mustExecute(t, f.conn, "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", f.run.runID)
	deleted, host, err := deleteFixtureFleet(t, f, nil)
	if err != nil {
		t.Fatal(err)
	}
	if deleted.deregisteredCount != 2 || !slices.Equal(host.calls, []string{"close"}) || len(host.panes) != 1 || host.panes[0].muxPaneID != some("%2") {
		t.Fatalf("deleted %+v calls %v panes %v", deleted, host.calls, host.panes)
	}
	if !fleetDeleted(t, f) {
		t.Fatal("the fleet is deleted")
	}
}

type memberOutcome struct {
	memberID   int64
	paneStatus string
}

func deletionOutcomes(deleted deletedFleet) []memberOutcome {
	outcomes := []memberOutcome{}
	for _, member := range deleted.members {
		outcomes = append(outcomes, memberOutcome(member))
	}
	return outcomes
}

func sortedOutcomes(outcomes []memberOutcome) []memberOutcome {
	slices.SortFunc(outcomes, func(a, b memberOutcome) int {
		if a.memberID != b.memberID {
			return int(a.memberID - b.memberID)
		}
		return strings.Compare(a.paneStatus, b.paneStatus)
	})
	return outcomes
}

func directorOf(t *testing.T, conn *dbConn, fleet int64) int64 {
	t.Helper()
	record, err := requireFleet(context.Background(), conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	return record.directorMemberID
}

func TestCliFleet_Fleet_FleetDeletionEndsEveryOrdinaryMemberAndKeepsTheDirectorPane(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	fleet := f.run.fleetID
	pending := register(t, f.conn, fleet, "pending", none[string]())
	deleted, host, err := deleteFixtureFleet(t, f, nil)
	if err != nil {
		t.Fatal(err)
	}
	if deleted.fleetID != fleet || deleted.deregisteredCount != 3 {
		t.Fatalf("deleted %+v", deleted)
	}
	expected := sortedOutcomes([]memberOutcome{{f.run.memberID, "%2 (killed)"}, {pending, pendingPaneStatus}})
	if got := sortedOutcomes(deletionOutcomes(deleted)); !slices.Equal(got, expected) {
		t.Fatalf("outcomes %v, want %v", got, expected)
	}
	if len(host.panes) != 1 || host.panes[0].muxPaneID != some("%2") || host.restored != 1 {
		t.Fatalf("panes %v restored %d", host.panes, host.restored)
	}
	if !fleetDeleted(t, f) {
		t.Fatal("the fleet is deleted")
	}
	if active := queryInt64(t, f.conn, "SELECT COUNT(*) FROM members WHERE fleet_id=?1 AND status='active'", fleet); active != 0 {
		t.Fatalf("%d members stay active", active)
	}
}

func TestCliFleet_Fleet_FleetDeletionStopsAtTheFirstFailedMemberAndARerunResumes(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	fleet := f.run.fleetID
	for _, member := range []struct{ name, pane string }{{"first", "%5"}, {"second", "%6"}, {"third", "%7"}} {
		register(t, f.conn, fleet, member.name, some(member.pane))
	}
	s := fixtureSettings(t, f)
	order := lockOrderedPlacements(t, f.conn, s, fleet, some(directorOf(t, f.conn, fleet)))
	if len(order) != 4 {
		t.Fatalf("%d ordinary placements", len(order))
	}
	failing := 1
	if order[1].memberID == f.run.memberID {
		failing = 2
	}
	failed := order[failing]
	failedPane, _ := failed.placement.muxPaneID.get()
	_, host, err := deleteFixtureFleet(t, f, func(h *deletionTestHost) { h.failingPane = some(failedPane) })
	if err == nil {
		t.Fatal("the failed pane close fails deletion")
	}
	ended := []string{}
	for _, member := range order[:failing] {
		ended = append(ended, fmt.Sprint(member.memberID))
	}
	want := fmt.Sprintf("fleet %d was not deleted: member %d: kill_pane failed for pane %s: pane vanished. Verify with 'cafleet doctor'. Members already deleted: %s.",
		fleet, failed.memberID, failedPane, strings.Join(ended, ", "))
	if errorMessage(err) != want {
		t.Fatalf("message %q\nwant %q", errorMessage(err), want)
	}
	if len(host.panes) != failing+1 || host.restored != failing || fleetDeleted(t, f) {
		t.Fatalf("panes %d restored %d", len(host.panes), host.restored)
	}
	for _, member := range order[:failing] {
		if found, _ := getMember(context.Background(), f.conn, member.memberID, fleet); found.set {
			t.Fatalf("member %d is deleted", member.memberID)
		}
	}
	for _, member := range order[failing:] {
		if current := requireMember(t, f.conn, member.memberID, fleet); current.placement != member.placement {
			t.Fatalf("member %d keeps its placement", member.memberID)
		}
		lock, free := tryPaneLock(t, s, member.placement)
		if !free {
			t.Fatalf("member %d's pane lock is released", member.memberID)
		}
		lock.close()
	}
	deleted, rerun, err := deleteFixtureFleet(t, f, nil)
	if err != nil {
		t.Fatal(err)
	}
	expected := []memberOutcome{}
	for _, member := range order[failing:] {
		pane, _ := member.placement.muxPaneID.get()
		expected = append(expected, memberOutcome{member.memberID, pane + " (killed)"})
	}
	if got := sortedOutcomes(deletionOutcomes(deleted)); !slices.Equal(got, sortedOutcomes(expected)) {
		t.Fatalf("rerun outcomes %v", got)
	}
	if deleted.deregisteredCount != len(order)-failing+1 || len(rerun.panes) != len(order)-failing || !fleetDeleted(t, f) {
		t.Fatalf("rerun deleted %+v panes %d", deleted, len(rerun.panes))
	}
}

func storedMemberStatus(t *testing.T, conn *dbConn, memberID, fleetID int64) string {
	t.Helper()
	return queryText(t, conn, "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2", memberID, fleetID)
}

func deleteRecoveryFleet(t *testing.T, c *recoveryCase, d deadline, configure func(*deletionTestHost)) (*deletionTestHost, error) {
	t.Helper()
	host := newRecoveringDeletionHost(t, c, d)
	if configure != nil {
		configure(host)
	}
	_, err := deleteFleetWithHost(context.Background(), c.fixture.conn, c.settings, c.fixture.run.fleetID, d, host)
	return host, err
}

func twoSecondDeadline() deadline {
	return deadlineUntil(time.Now().Add(2 * time.Second))
}

func TestCliFleet_Recovery_RunnerRecoveryDeletionOfAnExitedRunnerWithIncompleteCleanupRecoversWithoutContactingIt(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	mustExecute(t, c.fixture.conn, "UPDATE member_runs SET status='exited',cleanup_state='incomplete',exit_code=143,ended_at=?1 WHERE run_id=?2", recoveryFixtureNow, c.fixture.run.runID)
	c.refresh(t)
	original := c.fixture.run
	c.release(t)
	host, err := deleteRecoveryFleet(t, c, twoSecondDeadline(), func(h *deletionTestHost) { h.runnerGone = true })
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(host.calls, []string{"recover", "close"}) {
		t.Fatalf("an exited runner is recovered, never asked to shut down: %v", host.calls)
	}
	recovered := host.recovered
	if recovered.status != runStatusExited || recovered.exitCode != some(int64(143)) || recovered.cleanupState != cleanupStateComplete {
		t.Fatalf("recovered %v/%v/%v", recovered.status, recovered.exitCode, recovered.cleanupState)
	}
	if len(recoveryEvents(t, c)) != 0 {
		t.Fatal("recovery of an exited runner records no event")
	}
	if status := storedMemberStatus(t, c.fixture.conn, original.memberID, original.fleetID); status != "deregistered" {
		t.Fatalf("member status %s", status)
	}
}

func TestCliFleet_Recovery_RunnerRecoveryDeletionOfANonTerminalRunRequestsShutdownOnlyFromALiveOwner(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		owner func(t *testing.T) processIdentity
		lost  bool
	}{
		{"gone_runner", func(t *testing.T) processIdentity {
			child, identity := zombie(t)
			if err := child.Wait(); err == nil {
				t.Fatal("the killed child reports its signal")
			}
			return identity
		}, true},
		{"live_owner_refuses", currentProcess, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			c := newRecoveryCase(t)
			owner := tc.owner(t)
			mustExecute(t, c.fixture.conn, "UPDATE member_runs SET owner_pid=?1, owner_start_identity=?2 WHERE run_id=?3", owner.pid, owner.startIdentity, c.fixture.run.runID)
			c.refresh(t)
			original := c.fixture.run
			c.release(t)
			host, err := deleteRecoveryFleet(t, c, twoSecondDeadline(), func(h *deletionTestHost) { h.runnerGone = true })
			if tc.lost {
				if err != nil {
					t.Fatalf("deletion of a dead owner's run failed after %v: %v", host.calls, err)
				}
				if !slices.Equal(host.calls, []string{"recover", "close"}) {
					t.Fatalf("a dead owner is recovered, never asked to shut down: %v", host.calls)
				}
				recovered, ran := host.recovered, host.recovered != nil
				if !ran || recovered.status != runStatusLost {
					t.Fatal("recovery ran and lost the run")
				}
				lost := 0
				for _, view := range recoveryEvents(t, c) {
					if view.event.kind == runtimeEventKindRunnerLost {
						lost++
					}
				}
				if lost != 1 {
					t.Fatalf("exactly one runner_lost event, got %d", lost)
				}
				if status := storedMemberStatus(t, c.fixture.conn, original.memberID, original.fleetID); status != "deregistered" {
					t.Fatalf("member status %s", status)
				}
				return
			}
			if err == nil || !strings.Contains(errorMessage(err), "ECONNREFUSED") {
				t.Fatalf("a live owner's refused shutdown keeps the member: %v", err)
			}
			if !slices.Equal(host.calls, []string{"shutdown"}) {
				t.Fatalf("a live owner is asked to shut down first: %v", host.calls)
			}
			assertRegistryActive(t, c.fixture)
			if current := mustRun(t, c.fixture.conn, original.runID); current.status != runStatusRunning {
				t.Fatalf("status %v", current.status)
			}
		})
	}
}

func TestCliFleet_Recovery_RunnerRecoveryDeletionRejectsForeignPreEntryPostRecoveryAndPostCloseOwnerChanges(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		mutation deletionMutation
		calls    []string
		events   int
		epochs   int64
	}{
		{"before recovery", deletionMutationBeforeRecovery, []string{"shutdown", "recover"}, 1, 1},
		{"after recovery", deletionMutationAfterRecovery, []string{"shutdown", "recover"}, 3, 2},
		{"after close", deletionMutationAfterClose, []string{"shutdown", "recover", "close"}, 3, 2},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			c := newRecoveryCase(t)
			c.seed(t, recoveryOpening(t, c.fixture.run, 1, 1))
			epoch := c.fixture.run.ownerEpoch
			c.release(t)
			host, err := deleteRecoveryFleet(t, c, twoSecondDeadline(), func(h *deletionTestHost) { h.mutation = tc.mutation })
			if err == nil {
				t.Fatal("a foreign owner change fails deletion")
			}
			assertRegistryActive(t, c.fixture)
			current := mustRun(t, c.fixture.conn, c.fixture.run.runID)
			if !slices.Equal(host.calls, tc.calls) || len(recoveryEvents(t, c)) != tc.events || current.ownerEpoch != epoch+tc.epochs {
				t.Fatalf("calls %v events %d epoch %d", host.calls, len(recoveryEvents(t, c)), current.ownerEpoch-epoch)
			}
		})
	}
}

func TestCliFleet_Recovery_RunnerRecoveryDeletionUpdatesOnlyItsProvenTargetAndKeepsOtherPlacementsPinned(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	c.seed(t, recoveryOpening(t, c.fixture.run, 1, 1))
	other := register(t, c.fixture.conn, c.fixture.run.fleetID, "other-deletion-target", none[string]())
	c.release(t)
	host, err := deleteRecoveryFleet(t, c, twoSecondDeadline(), func(h *deletionTestHost) {
		h.mutation = deletionMutationOtherPlacement
		h.otherMember = other
	})
	if err == nil {
		t.Fatal("a changed other placement fails deletion")
	}
	assertRegistryActive(t, c.fixture)
	if host.recovered == nil || !slices.Equal(host.calls, []string{"shutdown", "recover"}) {
		t.Fatalf("calls %v", host.calls)
	}
	if pane := requireMember(t, c.fixture.conn, other, c.fixture.run.fleetID).placement.muxPaneID; pane != some("%foreign") {
		t.Fatalf("other pane %v", pane)
	}
}

func TestCliFleet_Recovery_RunnerRecoveryDeletionExhaustedDeadlineRetainsPendingNamespaceAndActiveRegistry(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	c.seed(t, recoveryOpening(t, c.fixture.run, 1, 1))
	before := recoveryRows(t, c.fixture.conn)
	c.release(t)
	host, err := deleteRecoveryFleet(t, c, deadlineUntil(time.Now().Add(-time.Millisecond)), nil)
	if err == nil {
		t.Fatal("an exhausted deadline fails deletion")
	}
	assertRecoveryRows(t, c.fixture.conn, before, "exhausted deadline")
	assertRegistryActive(t, c.fixture)
	if len(host.calls) != 0 {
		t.Fatalf("calls %v", host.calls)
	}
}

const recoveryDeletionSlowSum = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter"

func TestCliFleet_Recovery_RunnerRecoveryDeletionUnwindRestoresTheStillLiveOuterScope(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	c.seed(t, recoveryOpening(t, c.fixture.run, 1, 1))
	c.release(t)
	start := time.Now()
	inner := start.Add(1500 * time.Millisecond)
	outer := start.Add(2500 * time.Millisecond)
	emergency, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	d := deadlineUntil(inner)
	host := newRecoveringDeletionHost(t, c, d)
	host.recovery.action = func(boundary string, _ runRecord, _ time.Time) error {
		if boundary == "after_commit" {
			panic("owned injected recovery unwind")
		}
		return nil
	}
	mustExecBatch(t, c.fixture.conn, "PRAGMA busy_timeout=137")
	fleet := c.fixture.run.fleetID
	_, err := withDatabaseDeadline(emergency, c.fixture.conn, outer, func(ctx context.Context, conn *dbConn) (int64, error) {
		panicked := func() (panicked bool) {
			defer func() { panicked = recover() != nil }()
			_, _ = deleteFleetWithHost(ctx, conn, c.settings, fleet, d, host)
			return false
		}()
		if !panicked || !host.recovery.called("after_commit") {
			t.Fatalf("the injected recovery unwind panics out of deletion after its commit: panicked=%v calls=%v deletion=%v", panicked, host.recovery.calls, host.calls)
		}
		time.Sleep(time.Until(inner.Add(20 * time.Millisecond)))
		if now := time.Now(); !now.After(inner) || !now.Before(outer) {
			t.Fatal("the probe runs between the inner and outer deadlines")
		}
		if value := queryInt64(t, conn, "SELECT 42"); value != 42 {
			t.Fatal("the outer scope is live")
		}
		return queryRow(conn, recoveryDeletionSlowSum, nil, func(row sqlRow) (int64, error) { return row.int64(0) })
	})
	if emergency.Err() != nil {
		t.Fatal("the original outer deadline cancels SQL")
	}
	if err == nil || time.Now().Before(outer) {
		t.Fatal("the retired inner deadline cannot cancel outer work")
	}
	if elapsed := time.Since(start); elapsed >= 3500*time.Millisecond {
		t.Fatalf("the outer deadline ends the scope: %v", elapsed)
	}
	if timeout := queryInt64(t, c.fixture.conn, "PRAGMA busy_timeout"); timeout != 137 {
		t.Fatalf("busy timeout %d", timeout)
	}
	if value := queryInt64(t, c.fixture.conn, "SELECT 42"); value != 42 {
		t.Fatal("the connection is reusable")
	}
}

func lockFixtureConn(t *testing.T, f lockFixture) (*dbConn, settings) {
	t.Helper()
	conn := fileConn(t, f.url)
	path, err := databasePath(f.url)
	if err != nil {
		t.Fatal(err)
	}
	return conn, settingsForDatabase(path)
}

func memberActivities(t *testing.T, conn *dbConn, fleet int64) []memberActivity {
	t.Helper()
	activities, err := listMembers(context.Background(), conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	return activities
}

func activityIdentities(activities []memberActivity) []string {
	identities := []string{}
	for _, activity := range activities {
		identities = append(identities, fmt.Sprintf("%d %+v", activity.member.memberID, activity.member.placement))
	}
	return identities
}

func TestCliFleet_PaneLocks_FleetDeletionContentionPreservesRowsAndReleasesPrecedingGuards(t *testing.T) {
	t.Parallel()
	f := newLockFixture(t)
	conn, s := lockFixtureConn(t, f)
	fleet := f.member.fleetID
	placements, err := deletionPlacements(context.Background(), conn, s, fleet)
	if err != nil {
		t.Fatal(err)
	}
	guard, acquired := tryPaneLock(t, s, placements[len(placements)-1].placement)
	if !acquired {
		t.Fatal("the last placement's lock is free")
	}
	before := activityIdentities(memberActivities(t, conn, fleet))
	started := time.Now()
	if _, err := deleteFleet(context.Background(), conn, s, fleet, deadlineUntil(started.Add(160*time.Millisecond))); err == nil {
		t.Fatal("a held pane lock fails deletion")
	}
	if elapsed := time.Since(started); elapsed >= 700*time.Millisecond {
		t.Fatalf("deletion waited %v", elapsed)
	}
	if after := activityIdentities(memberActivities(t, conn, fleet)); !slices.Equal(after, before) {
		t.Fatal("contention preserves every member row")
	}
	for _, p := range placements[:len(placements)-1] {
		paneLockAcquireReleased(t, s.databaseURL, p.placement).close()
	}
	guard.close()
	deleted, err := guardedDeleteFleet(t, conn, f.url, fleet)
	if err != nil || deleted.deregisteredCount != len(before) {
		t.Fatalf("deleted %+v %v", deleted, err)
	}
}

func TestCliFleet_PaneLocks_FleetDeletionReloadsNewPlacementsWithinTheOriginalDeadline(t *testing.T) {
	t.Parallel()
	f := newLockFixture(t)
	conn, s := lockFixtureConn(t, f)
	fleet := f.member.fleetID
	placements, err := deletionPlacements(context.Background(), conn, s, fleet)
	if err != nil {
		t.Fatal(err)
	}
	first := placements[0].placement
	last, acquired := tryPaneLock(t, s, placements[1].placement)
	if !acquired {
		t.Fatal("the last placement's lock is free")
	}
	started := time.Now()
	d := deadlineUntil(started.Add(600 * time.Millisecond))
	result := make(chan error, 1)
	go func() {
		deleting, err := connect(f.url)
		if err != nil {
			result <- err
			return
		}
		defer deleting.close() //nolint:errcheck // the deleting goroutine's own connection
		_, err = deleteFleet(context.Background(), deleting, s, fleet, d)
		result <- err
	}()
	for {
		probe, free := tryPaneLock(t, s, first)
		if !free {
			break
		}
		probe.close()
		if time.Since(started) >= 400*time.Millisecond {
			t.Fatal("deletion owns the first key before waiting on the last")
		}
		time.Sleep(5 * time.Millisecond)
	}
	added := register(t, conn, fleet, "added-during-wait", some("%99"))
	addedGuard, acquired := tryPaneLock(t, s, requireMember(t, conn, added, fleet).placement)
	if !acquired {
		t.Fatal("the added placement's lock is free")
	}
	time.Sleep(100 * time.Millisecond)
	last.close()
	err = <-result
	if err == nil || !strings.Contains(errorMessage(err), fmt.Sprintf("at member %d;", added)) {
		t.Fatalf("deletion stops at the added member: %v", err)
	}
	if elapsed := time.Since(started); elapsed >= 1100*time.Millisecond {
		t.Fatalf("deletion took %v", elapsed)
	}
	if count := len(memberActivities(t, conn, fleet)); count != 3 {
		t.Fatalf("%d members remain", count)
	}
	if record, _ := requireFleet(context.Background(), conn, fleet); record.deletedAt.set {
		t.Fatal("the fleet stays active")
	}
	addedGuard.close()
	for _, p := range placements {
		paneLockAcquireReleased(t, s.databaseURL, p.placement).close()
	}
}

func TestCliFleet_MultiplexerDeletionHostBoundsEachPaneCloseByItsOwnDeadline(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.succeed(1)
	host := newMultiplexerDeletionHost(tmuxMultiplexerForServer(runner, "/tmp/tmux-1000/default"))
	p := placement{
		backend:       "tmux",
		muxSession:    "s",
		muxWindowID:   some("@5"),
		muxWindowName: some("cafleet-1-2"),
		windowOrdinal: 2,
		columnSide:    columnSideLeft,
		muxPaneID:     some("%9"),
		codingAgent:   "hermes",
		createdAt:     "2026-11-07T00:00:00Z",
		muxServer:     "default",
	}
	if err := host.closePane(p, deadlineUntil(time.Now())); err == nil {
		t.Fatal("a spent deadline fails the close")
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatal("a spent deadline runs no command")
	}
	if err := host.closePane(p, deadlineUntil(time.Now().Add(5*time.Second))); err != nil {
		t.Fatal(err)
	}
	runner.respond("%0|0\n")
	if err := host.restoreLayout(); err != nil {
		t.Fatal(err)
	}
	tmux := func(parts ...string) []string {
		return append([]string{"tmux", "-S", "/tmp/tmux-1000/default"}, parts...)
	}
	assertCalls(t, "the close and its restoration target the recorded window", runner.runArgvs(), [][]string{
		tmux("kill-pane", "-t", "%9"),
		tmux("list-panes", "-t", "@5", "-F", "#{pane_id}|#{pane_left}"),
	})
}

// untouchedDeletionHost fails the test on any step of a refused fleet deletion.
type untouchedDeletionHost struct{}

func (untouchedDeletionHost) recover(context.Context, *dbConn, settings, runRecord, placement, *paneLock, deadline) (recoveryCompletion, error) {
	panic("a refused fleet deletion recovers no runner")
}

func (untouchedDeletionHost) shutdown(context.Context, *dbConn, runRecord, deadline) (shutdownOutcome, error) {
	panic("a refused fleet deletion shuts no runner down")
}

func (untouchedDeletionHost) settleHermes(context.Context, *dbConn, runRecord, string, deadline) error {
	panic("a refused fleet deletion settles nothing")
}

func (untouchedDeletionHost) closePane(placement, deadline) error {
	panic("a refused fleet deletion closes no pane")
}

func (untouchedDeletionHost) restoreLayout() error {
	panic("a refused fleet deletion restores no layout")
}

func TestCliFleet_LayoutLock_ABusyLayoutLockFailsFleetDeleteAndLeavesEveryMemberUnchanged(t *testing.T) {
	t.Parallel()
	conn, url := initializedFileConn(t)
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	s := settingsForDatabase(path)
	fleet, director := createFleetFixture(t, conn, "layout-fleet-busy")
	worker := register(t, conn, fleet, "worker", some("%5"))
	pending := register(t, conn, fleet, "pending", none[string]())
	before := memberActivities(t, conn, fleet)
	held, acquired, err := layoutLockTryAcquire(s.databaseURL, fleet)
	if err != nil || !acquired {
		t.Fatalf("the test holds the fleet layout: %v %v", acquired, err)
	}
	defer held.close()
	started := time.Now()
	_, err = deleteFleetWithLayoutWait(context.Background(), conn, s, fleet, deadlineUntil(started.Add(5*time.Second)), 300*time.Millisecond, untouchedDeletionHost{})
	waited := time.Since(started)
	if err == nil {
		t.Fatal("a busy layout fails deletion")
	}
	if waited < 300*time.Millisecond || waited >= 3*time.Second {
		t.Fatalf("waited %v", waited)
	}
	if want := fmt.Sprintf("fleet %d pane layout remained busy for 60 seconds; fleet was not deleted.", fleet); errorMessage(err) != want || cafleetErrorOf(err).exitCode() != 1 {
		t.Fatalf("error %q", errorMessage(err))
	}
	if record, _ := requireFleet(context.Background(), conn, fleet); record.deletedAt.set {
		t.Fatal("the fleet stays active")
	}
	after := memberActivities(t, conn, fleet)
	if !slices.Equal(activityIdentities(after), activityIdentities(before)) {
		t.Fatal("every member keeps its placement")
	}
	ids := []int64{}
	for _, activity := range after {
		ids = append(ids, activity.member.memberID)
	}
	if !slices.Equal(ids, []int64{director, worker, pending}) {
		t.Fatalf("members %v", ids)
	}
	paneLockAcquireReleased(t, s.databaseURL, after[1].member.placement).close()
}

func setRunOwner(t *testing.T, f *runnerFixture, owner processIdentity) {
	t.Helper()
	mustExecute(t, f.conn, "UPDATE member_runs SET status='exited', cleanup_state='incomplete', owner_pid=?1, owner_start_identity=?2 WHERE run_id=?3", owner.pid, owner.startIdentity, f.run.runID)
}

func TestCliFleet_DeletionWaitsForTheRunnerASettlementLeftRunningOnlyWithinItsDeadline(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	runner := exec.Command("sleep", "0.3")
	if err := runner.Start(); err != nil {
		t.Fatal(err)
	}
	setRunOwner(t, f, requireIdentity(t, int64(runner.Process.Pid)))
	var reaped sync.WaitGroup
	reaped.Go(func() {
		if err := runner.Wait(); err != nil {
			t.Error(err)
		}
	})
	started := time.Now()
	if err := awaitRunnerExit(context.Background(), f.conn, f.run.runID, newDeadline()); err != nil {
		t.Fatal(err)
	}
	if elapsed := time.Since(started); elapsed < 250*time.Millisecond {
		t.Fatalf("deletion returned after %v", elapsed)
	}
	reaped.Wait()
	setRunOwner(t, f, currentProcess(t))
	started = time.Now()
	err := awaitRunnerExit(context.Background(), f.conn, f.run.runID, deadlineUntil(started.Add(100*time.Millisecond)))
	if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded {
		t.Fatalf("%v, want deadline exceeded", err)
	}
	if elapsed := time.Since(started); elapsed >= time.Second {
		t.Fatalf("the wait ended after %v", elapsed)
	}
}

func TestCliFleet_DeletionTreatsAnUnreapedZombieRunnerAsExited(t *testing.T) {
	t.Parallel()
	f := newRunnerFixture(t)
	runner, identity := zombie(t)
	setRunOwner(t, f, identity)
	started := time.Now()
	waited := awaitRunnerExit(context.Background(), f.conn, f.run.runID, deadlineUntil(started.Add(2*time.Second)))
	elapsed := time.Since(started)
	if err := runner.Wait(); err == nil {
		t.Fatal("the killed child reports its signal")
	}
	if waited != nil {
		t.Fatal(waited)
	}
	if elapsed >= time.Second {
		t.Fatalf("the wait ended after %v", elapsed)
	}
}

const (
	createTmuxSocket       = "/tmp/tmux-1000/default"
	createRerunSocket      = "/tmp/tmux-0/default"
	createContextFormat    = "#{session_name}|#{window_id}|#{pane_id}"
	createProcessFormat    = "#{pane_id}|#{pane_pid}"
	createWindowNameFormat = "#{window_id}|#{window_name}"
)

var createTmuxEnv = map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0", "TMUX_PANE": "%3"}

func tabLabel(t *testing.T, tab, label string) string {
	return herdrEnvelope(t, map[string]any{"tab": map[string]any{"tab_id": tab, "label": label}})
}

func herdrContext(t *testing.T, workspace, tab, pane string) string {
	return herdrEnvelope(t, map[string]any{"pane": map[string]any{"workspace_id": workspace, "tab_id": tab, "pane_id": pane}})
}

func tmuxCreation(runner *fakeRunner, context, window, name string) {
	runner.respond(context)
	runner.succeed(1)
	runner.respond(window + "|" + name + "\n")
	runner.respond(context)
}

func herdrCreation(t *testing.T, runner *fakeRunner, workspace, tab, pane, name string) {
	runner.respond(herdrContext(t, workspace, tab, pane))
	runner.respond(herdrEnvelope(t, map[string]any{}))
	runner.respond(tabLabel(t, tab, name))
	runner.respond(herdrContext(t, workspace, tab, pane))
}

func tmuxAt(server string, parts ...string) []string {
	return append([]string{"tmux", "-S", server}, parts...)
}

func herdrAt(server string, parts ...string) []string {
	return append([]string{"herdr", "--session", server}, parts...)
}

func assertStoredPlacement(t *testing.T, conn *dbConn, fleet createdFleet) {
	t.Helper()
	stored, err := directorPlacement(context.Background(), conn, fleet.fleetID)
	if err != nil {
		t.Fatal(err)
	}
	p := fleet.director.placement
	if stored != p || fleet.director.name != "Director" || p.windowOrdinal != 0 || p.columnSide != columnSideLeft || !conn.raw.GetAutocommit() {
		t.Fatalf("stored placement %+v, created %+v", stored, p)
	}
}

type namingTarget struct {
	backend muxBackend
	server  string
	pane    string
	window  string
}

func newNamingTarget(backend muxBackend) namingTarget {
	switch backend {
	case muxBackendTmux:
		return namingTarget{backend: backend, server: createRerunSocket, pane: "%5", window: "@1"}
	case muxBackendHerdr:
		return namingTarget{backend: backend, server: "default", pane: "w4:p3N", window: "w4:t1"}
	default:
		panic("invalid muxBackend")
	}
}

func (n namingTarget) context(t *testing.T) string {
	if n.backend == muxBackendTmux {
		return "main|@1|%5\n"
	}
	return herdrContext(t, "w4", n.window, n.pane)
}

func (n namingTarget) renamed(t *testing.T) string {
	if n.backend == muxBackendTmux {
		return ""
	}
	return herdrEnvelope(t, map[string]any{})
}

func (n namingTarget) nameReply(t *testing.T, name string) string {
	if n.backend == muxBackendTmux {
		return n.window + "|" + name + "\n"
	}
	return tabLabel(t, n.window, name)
}

func (n namingTarget) calls(name string) [][]string {
	if n.backend == muxBackendTmux {
		return [][]string{
			tmuxAt(n.server, "display-message", "-p", "-t", n.pane, createContextFormat),
			tmuxAt(n.server, "rename-window", "-t", n.window, name),
			tmuxAt(n.server, "display-message", "-p", "-t", n.window, createWindowNameFormat),
			tmuxAt(n.server, "display-message", "-p", "-t", n.pane, createContextFormat),
		}
	}
	return [][]string{
		herdrAt(n.server, "pane", "current", "--pane", n.pane),
		herdrAt(n.server, "tab", "rename", n.window, name),
		herdrAt(n.server, "tab", "get", n.window),
		herdrAt(n.server, "pane", "current", "--pane", n.pane),
	}
}

func (n namingTarget) assertDiagnostic(t *testing.T, err error, cause opt[string]) {
	t.Helper()
	diagnostic := errorMessage(err)
	for _, expected := range []string{n.backend.name(), n.server, n.window, n.pane, "cafleet-1-0", "fleet list", "changed"} {
		if !strings.Contains(diagnostic, expected) {
			t.Fatalf("missing %q: %s", expected, diagnostic)
		}
	}
	if text, set := cause.get(); set && !strings.Contains(diagnostic, text) {
		t.Fatalf("missing cause %q: %s", text, diagnostic)
	}
}

// unreadTable fails the test on any read, for paths that must not walk.
type unreadTable struct{}

func (unreadTable) ownPID() int64 { panic("this path reads no process table") }

func (unreadTable) entry(int64) (processEntry, bool, error) {
	panic("this path reads no process table")
}

func (unreadTable) nestedPIDNamespace() (bool, error) { panic("this path reads no process table") }

func createRequest(pane, muxServer opt[string]) fleetCreateRequest {
	return fleetCreateRequest{name: "alpha", agent: codingAgentCodex, pane: pane, muxServer: muxServer}
}

func explicitRequest(pane, server string) fleetCreateRequest {
	return createRequest(some(pane), some(server))
}

func fleetRows(t *testing.T, conn *dbConn) [2]int64 {
	t.Helper()
	return [2]int64{queryInt64(t, conn, "SELECT COUNT(*) FROM fleets"), queryInt64(t, conn, "SELECT COUNT(*) FROM members")}
}

// herdrDaemonChain is herdr pane A's shared Codex daemon chain.
func herdrDaemonChain() *fixtureProcessTable {
	return newFixtureProcessTable(408076).
		process(408076, 407596, "cafleet", "fleet", "create").
		process(407596, 407442, "/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex", "app-server", "--list").
		process(407442, 407363, "codex", "--enable", "daemon_auto_start", "--dangerously-bypass-approvals-and-sandbox", "<prompt>").
		process(407363, 3340413, "/bin/sh").
		process(3340413, 1, "/root/.local/share/mise/installs/aqua-ogulcancelik-herdr/v0.9.5/herdr", "server")
}

func createdWith(t *testing.T, conn *dbConn, request fleetCreateRequest, env map[string]string, runner *fakeRunner, table processTable) createdFleet {
	t.Helper()
	fleet, err := createWith(context.Background(), conn, request, env, runner, table)
	if err != nil {
		t.Fatal(err)
	}
	return fleet
}

func refusedCreate(t *testing.T, conn *dbConn, request fleetCreateRequest, env map[string]string, runner *fakeRunner, table processTable) error {
	t.Helper()
	fleet, err := createWith(context.Background(), conn, request, env, runner, table)
	if err == nil {
		t.Fatalf("fleet create registered %+v", fleet)
	}
	if cafleetErrorOf(err).exitCode() != 1 || fleetRows(t, conn) != [2]int64{0, 0} ||
		queryInt64(t, conn, "SELECT COUNT(*) FROM member_placements") != 0 || !conn.raw.GetAutocommit() {
		t.Fatalf("a refused create registers nothing: %v", err)
	}
	return err
}

func createDatabase(t *testing.T) *dbConn {
	t.Helper()
	conn, _ := initializedFileConn(t)
	return conn
}

func TestCliFleet_Create_AnExplicitTmuxPaneReadsNoProcessTableAndIgnoresAHerdrEnvironment(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "tmux")
	tmuxCreation(runner, "main|@1|%5\n", "@1", "cafleet-1-0")
	fleet := createdWith(t, conn, explicitRequest("%5", createRerunSocket),
		map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "default", "HERDR_PANE_ID": "w5:p1"}, runner, unreadTable{})
	p := fleet.director.placement
	if p.backend != "tmux" || p.muxServer != createRerunSocket || p.muxSession != "main" || p.muxWindowID != some("@1") ||
		p.muxWindowName != some("cafleet-1-0") || p.windowOrdinal != 0 || p.columnSide != columnSideLeft || p.muxPaneID != some("%5") || p.codingAgent != "codex" {
		t.Fatalf("placement %+v", p)
	}
	assertCalls(t, "explicit tmux pane", runner.runArgvs(), [][]string{
		tmuxAt(createRerunSocket, "display-message", "-p", "-t", "%5", createContextFormat),
		tmuxAt(createRerunSocket, "rename-window", "-t", "@1", "cafleet-1-0"),
		tmuxAt(createRerunSocket, "display-message", "-p", "-t", "@1", createWindowNameFormat),
		tmuxAt(createRerunSocket, "display-message", "-p", "-t", "%5", createContextFormat),
	})
	if fleetRows(t, conn) != [2]int64{1, 1} {
		t.Fatal("one fleet and its Director are registered")
	}
	assertStoredPlacement(t, conn, fleet)
}

func TestCliFleet_Create_AnExplicitHerdrPaneReadsNoProcessTableAndIgnoresATmuxEnvironment(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "herdr")
	herdrCreation(t, runner, "w4", "w4:t1", "w4:p3N", "cafleet-1-0")
	fleet := createdWith(t, conn, explicitRequest("w4:p3N", "default"), createTmuxEnv, runner, unreadTable{})
	p := fleet.director.placement
	if p.backend != "herdr" || p.muxServer != "default" || p.muxSession != "w4" || p.muxWindowID != some("w4:t1") ||
		p.muxWindowName != some("cafleet-1-0") || p.muxPaneID != some("w4:p3N") {
		t.Fatalf("placement %+v", p)
	}
	assertCalls(t, "explicit herdr pane", runner.runArgvs(), [][]string{
		herdrAt("default", "pane", "current", "--pane", "w4:p3N"),
		herdrAt("default", "tab", "rename", "w4:t1", "cafleet-1-0"),
		herdrAt("default", "tab", "get", "w4:t1"),
		herdrAt("default", "pane", "current", "--pane", "w4:p3N"),
	})
	assertStoredPlacement(t, conn, fleet)
}

func TestCliFleet_Create_AnExplicitPaneMissingFromItsServerReportsPaneNotFound(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.respond("||\n")
	err := refusedCreate(t, conn, explicitRequest("%5", createRerunSocket), map[string]string{}, runner, unreadTable{})
	if want := "pane %5 was not found on the tmux server /tmp/tmux-0/default; pass --pane and --mux-server for the Director's pane"; err.Error() != want {
		t.Fatalf("%q", err.Error())
	}
}

func TestCliFleet_Create_AMalformedPaneFailsBeforeAnyCommandOrProcessRead(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "tmux")
	err := refusedCreate(t, conn, explicitRequest("12", createRerunSocket), createTmuxEnv, runner, unreadTable{})
	if want := "--pane 12 is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)"; err.Error() != want {
		t.Fatalf("%q", err.Error())
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatal("a malformed pane runs no command")
	}
}

var herdrPaneA = map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "default", "HERDR_PANE_ID": "w5:p1"}

func TestCliFleet_Create_ACrossMultiplexerCallerIsRefusedWithoutCommandsThenTheRerunRegistersTmux(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithoutBinaries(t)
	err := refusedCreate(t, conn, createRequest(none[string](), none[string]()), herdrPaneA, runner, herdrDaemonChain())
	if want := "cafleet fleet create " + sharedServerMessage(sharedServer{label: "codex app-server", pid: 407596}); err.Error() != want {
		t.Fatalf("%q, want %q", err.Error(), want)
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatal("the refusal runs no command")
	}
	rerun := fakeRunnerWithBinary(t, "tmux")
	tmuxCreation(rerun, "main|@2|%5\n", "@2", "cafleet-1-0")
	fleet := createdWith(t, conn, explicitRequest("%5", createRerunSocket), herdrPaneA, rerun, herdrDaemonChain())
	if p := fleet.director.placement; p.backend != "tmux" || p.muxServer != createRerunSocket || p.muxPaneID != some("%5") {
		t.Fatalf("placement %+v", p)
	}
	calls := rerun.runArgvs()
	if len(calls) == 0 {
		t.Fatal("the rerun runs tmux")
	}
	for _, call := range calls {
		if !slices.Equal(call[:3], []string{"tmux", "-S", createRerunSocket}) {
			t.Fatalf("call %q", call)
		}
	}
}

func TestCliFleet_Create_ASharedServerRefusesBeforeMultiplexerDetection(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithoutBinaries(t)
	err := refusedCreate(t, conn, createRequest(none[string](), none[string]()), map[string]string{}, runner, herdrDaemonChain())
	if !strings.HasPrefix(err.Error(), "cafleet fleet create cannot verify this session's pane: a shared codex app-server server (pid 407596)") || len(runner.runArgvs()) != 0 {
		t.Fatalf("%q", err.Error())
	}
}

func TestCliFleet_Create_ACallerOutsideAnyMultiplexerKeepsTheDetectionMessage(t *testing.T) {
	t.Parallel()
	for _, env := range []map[string]string{{}, {"HERDR_ENV": "1", "TMUX": "/tmp/tmux-1000/default,123,0"}} {
		conn := createDatabase(t)
		err := refusedCreate(t, conn, createRequest(none[string](), none[string]()), env, fakeRunnerWithoutBinaries(t), plainChain())
		if err.Error() != "cafleet fleet create must be run inside a tmux or herdr session" {
			t.Fatalf("%v: %q", env, err.Error())
		}
	}
}

func TestCliFleet_Create_AnUnsetEnvPaneNamesTheUserRunCommandForItsBackend(t *testing.T) {
	t.Parallel()
	cases := []struct {
		backend muxBackend
		env     map[string]string
	}{
		{muxBackendTmux, map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0", "TMUX_PANE": ""}},
		{muxBackendHerdr, map[string]string{"HERDR_ENV": "1"}},
	}
	for _, tc := range cases {
		conn := createDatabase(t)
		runner := fakeRunnerWithBinary(t, tc.backend.name())
		err := refusedCreate(t, conn, createRequest(none[string](), none[string]()), tc.env, runner, plainChain())
		if want := "cafleet fleet create " + unsetMessage(tc.backend); err.Error() != want || len(runner.runArgvs()) != 0 {
			t.Fatalf("%s: %q", tc.backend.name(), err.Error())
		}
	}
}

func TestCliFleet_Create_EachUnverifiedEnvPaneRefusesWithItsCause(t *testing.T) {
	t.Parallel()
	const unreadable = "cannot read /proc/412/stat: Permission denied (os error 13)"
	cases := []struct {
		name    string
		table   *fixtureProcessTable
		panePID opt[int64]
		refusal refusal
	}{
		{"pane not found", plainChain(), none[int64](), refusalPaneNotFound{}},
		{"nested namespace", plainChain().nested(), some(int64(20)), refusalNestedPidNamespace{}},
		{"process unreadable", newFixtureProcessTable(30).process(30, 412, "cafleet").unreadable(412, unreadable), some(int64(99)), refusalProcessUnreadable{cause: unreadable}},
		{"walk stopped", newFixtureProcessTable(30).process(30, 20, "cafleet"), some(int64(99)), refusalWalkStopped{pid: 20}},
		{"pane not an ancestor", plainChain(), some(int64(99)), refusalPaneNotAncestor{panePID: 99}},
	}
	for _, tc := range cases {
		conn := createDatabase(t)
		runner := fakeRunnerWithBinary(t, "tmux")
		if pid, set := tc.panePID.get(); set {
			runner.respond(fmt.Sprintf("%%3|%d\n", pid))
		} else {
			runner.respond("|\n")
		}
		err := refusedCreate(t, conn, createRequest(none[string](), none[string]()), createTmuxEnv, runner, tc.table)
		caller := callerPane{variable: "TMUX_PANE", pane: "%3"}
		mux := callerMux{backend: muxBackendTmux, server: createTmuxSocket, mux: muxBackendTmux.forServer(createTmuxSocket, fakeRunnerWithoutBinaries(t))}
		if want := "cafleet fleet create " + unverifiedMessage(caller, tc.refusal, mux); err.Error() != want {
			t.Fatalf("%s: %q\nwant %q", tc.name, err.Error(), want)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxAt(createTmuxSocket, "display-message", "-p", "-t", "%3", createProcessFormat)})
	}
}

func TestCliFleet_Create_AVerifiedTmuxEnvPaneRegistersItsContext(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.respond("%3|20\n")
	tmuxCreation(runner, "main|@1|%3\n", "@1", "cafleet-1-0")
	fleet := createdWith(t, conn, createRequest(none[string](), none[string]()), createTmuxEnv, runner, plainChain())
	p := fleet.director.placement
	if p.backend != "tmux" || p.muxServer != createTmuxSocket || p.muxSession != "main" || p.muxWindowID != some("@1") ||
		p.muxWindowName != some("cafleet-1-0") || p.muxPaneID != some("%3") || p.codingAgent != "codex" {
		t.Fatalf("placement %+v", p)
	}
	assertCalls(t, "verified tmux pane", runner.runArgvs(), [][]string{
		tmuxAt(createTmuxSocket, "display-message", "-p", "-t", "%3", createProcessFormat),
		tmuxAt(createTmuxSocket, "display-message", "-p", "-t", "%3", createContextFormat),
		tmuxAt(createTmuxSocket, "rename-window", "-t", "@1", "cafleet-1-0"),
		tmuxAt(createTmuxSocket, "display-message", "-p", "-t", "@1", createWindowNameFormat),
		tmuxAt(createTmuxSocket, "display-message", "-p", "-t", "%3", createContextFormat),
	})
	if fleetRows(t, conn) != [2]int64{1, 1} {
		t.Fatal("one fleet and its Director are registered")
	}
	assertStoredPlacement(t, conn, fleet)
}

func TestCliFleet_Create_AVerifiedHerdrEnvPaneRegistersItsContext(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{"shell_pid": 20}}))
	herdrCreation(t, runner, "w4", "w4:t1", "w4:p3N", "cafleet-1-0")
	fleet := createdWith(t, conn, createRequest(none[string](), none[string]()),
		map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "work", "HERDR_PANE_ID": "w4:p3N"}, runner, plainChain())
	p := fleet.director.placement
	if p.backend != "herdr" || p.muxServer != "work" || p.muxSession != "w4" || p.muxWindowID != some("w4:t1") ||
		p.muxWindowName != some("cafleet-1-0") || p.muxPaneID != some("w4:p3N") {
		t.Fatalf("placement %+v", p)
	}
	calls := runner.runArgvs()
	selector := []string{"herdr", "--session", "work", "pane"}
	if len(calls) != 5 || !slices.Equal(calls[0][:4], selector) || calls[0][4] != "process-info" || !slices.Equal(calls[1][:4], selector) || calls[1][4] != "current" {
		t.Fatalf("calls %q", calls)
	}
	assertCalls(t, "verified herdr naming", calls[2:], [][]string{
		herdrAt("work", "tab", "rename", "w4:t1", "cafleet-1-0"),
		herdrAt("work", "tab", "get", "w4:t1"),
		herdrAt("work", "pane", "current", "--pane", "w4:p3N"),
	})
	assertStoredPlacement(t, conn, fleet)
}

func TestCliFleet_Create_AFailedTmuxWindowNameReadRegistersNothing(t *testing.T) {
	t.Parallel()
	for _, reply := range []string{"\n", "@1\n", "@2|main\n"} {
		conn := createDatabase(t)
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.respond("main|@1|%5\n")
		runner.succeed(1)
		runner.respond(reply)
		err := refusedCreate(t, conn, explicitRequest("%5", createRerunSocket), map[string]string{}, runner, unreadTable{})
		if !strings.Contains(err.Error(), "unexpected tmux display-message output: ") || len(runner.runArgvs()) != 3 {
			t.Fatalf("%q: %v", reply, err)
		}
	}
}

func TestCliFleet_Create_AFailedHerdrWindowNameReadRegistersNothing(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		reply    func(*testing.T, *fakeRunner)
		expected string
	}{
		{"tab not found", func(t *testing.T, r *fakeRunner) { r.fail(failedRun(herdrErrorStderr(t, "tab_not_found"))) }, "tab_not_found"},
		{"label missing", func(t *testing.T, r *fakeRunner) {
			r.respond(herdrEnvelope(t, map[string]any{"tab": map[string]any{"tab_id": "w4:t1"}}))
		}, "herdr tab get missing 'label' field"},
	}
	for _, tc := range cases {
		conn := createDatabase(t)
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(herdrContext(t, "w4", "w4:t1", "w4:p3N"))
		runner.respond(herdrEnvelope(t, map[string]any{}))
		tc.reply(t, runner)
		err := refusedCreate(t, conn, explicitRequest("w4:p3N", "default"), map[string]string{}, runner, unreadTable{})
		if !strings.Contains(err.Error(), tc.expected) {
			t.Fatalf("%s: %v", tc.name, err)
		}
	}
}

var namingBackends = []muxBackend{muxBackendTmux, muxBackendHerdr}

func TestCliFleet_Create_NamingUsesTheAllocatedFleetIdAndLeavesExistingPlacementsUnchanged(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	for _, backend := range namingBackends {
		conn := createDatabase(t)
		legacy, _ := createFleetFixture(t, conn, "legacy")
		previous, err := directorPlacement(ctx, conn, legacy)
		if err != nil {
			t.Fatal(err)
		}
		target := newNamingTarget(backend)
		runner := fakeRunnerWithBinary(t, backend.name())
		if backend == muxBackendTmux {
			tmuxCreation(runner, target.context(t), target.window, "cafleet-2-0")
		} else {
			herdrCreation(t, runner, "w4", target.window, target.pane, "cafleet-2-0")
		}
		fleet := createdWith(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
		if fleet.fleetID != 2 || fleet.director.placement.muxWindowName != some("cafleet-2-0") {
			t.Fatalf("%s: fleet %d window %v", backend.name(), fleet.fleetID, fleet.director.placement.muxWindowName)
		}
		assertStoredPlacement(t, conn, fleet)
		if current, err := directorPlacement(ctx, conn, legacy); err != nil || current != previous {
			t.Fatalf("%s: the legacy placement is unchanged: %v", backend.name(), err)
		}
		assertCalls(t, backend.name(), runner.runArgvs(), target.calls("cafleet-2-0"))
	}
}

func TestCliFleet_Create_RenameAndReadbackCommandFailuresRollBackAndStopImmediately(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		for _, failure := range []struct {
			call  int
			cause string
		}{{2, "rename denied"}, {3, "readback denied"}} {
			conn := createDatabase(t)
			target := newNamingTarget(backend)
			runner := fakeRunnerWithBinary(t, backend.name())
			runner.respond(target.context(t))
			if failure.call == 3 {
				runner.respond(target.renamed(t))
			}
			runner.fail(failedRun(failure.cause))
			err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
			target.assertDiagnostic(t, err, some(failure.cause))
			assertCalls(t, backend.name()+" "+failure.cause, runner.runArgvs(), target.calls("cafleet-1-0")[:failure.call])
		}
	}
}

func TestCliFleet_Create_SuccessfulReadbackRequiresTheExactCanonicalName(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		for _, name := range []string{"", "main", "cafleet-2-0", "cafleet-1-0 ", "cafleet-1-0\r", "cafleet-1-0\n"} {
			conn := createDatabase(t)
			target := newNamingTarget(backend)
			runner := fakeRunnerWithBinary(t, backend.name())
			runner.respond(target.context(t))
			runner.respond(target.renamed(t))
			runner.respond(target.nameReply(t, name))
			err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
			target.assertDiagnostic(t, err, none[string]())
			assertCalls(t, fmt.Sprintf("%s %q", backend.name(), name), runner.runArgvs(), target.calls("cafleet-1-0")[:3])
		}
	}
}

func TestCliFleet_Create_DirectorContextMustStillMatchAfterRenaming(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		var changed []func(*testing.T, *fakeRunner)
		respond := func(out string) func(*testing.T, *fakeRunner) {
			return func(_ *testing.T, r *fakeRunner) { r.respond(out) }
		}
		denied := func(_ *testing.T, r *fakeRunner) { r.fail(failedRun("context denied")) }
		if backend == muxBackendTmux {
			changed = []func(*testing.T, *fakeRunner){respond("other|@1|%5\n"), respond("main|@2|%5\n"), respond("main|@1|%6\n"), respond("||\n"), denied}
		} else {
			changed = []func(*testing.T, *fakeRunner){
				respond(herdrContext(t, "w9", "w4:t1", "w4:p3N")),
				respond(herdrContext(t, "w4", "w4:t2", "w4:p3N")),
				respond(herdrContext(t, "w4", "w4:t1", "w4:p4N")),
				func(t *testing.T, r *fakeRunner) { r.fail(failedRun(herdrErrorStderr(t, "pane_not_found"))) },
				denied,
			}
		}
		for index, reply := range changed {
			conn := createDatabase(t)
			target := newNamingTarget(backend)
			runner := fakeRunnerWithBinary(t, backend.name())
			runner.respond(target.context(t))
			runner.respond(target.renamed(t))
			runner.respond(target.nameReply(t, "cafleet-1-0"))
			reply(t, runner)
			err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
			target.assertDiagnostic(t, err, none[string]())
			assertCalls(t, fmt.Sprintf("%s change %d", backend.name(), index), runner.runArgvs(), target.calls("cafleet-1-0"))
		}
	}
}

func TestCliFleet_Create_RegistrationInsertFailuresLeaveTheAppliedNameAndDirectorPaneInPlace(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		for _, trigger := range []string{"BEFORE INSERT ON members", "BEFORE INSERT ON member_placements", "BEFORE UPDATE OF director_member_id ON fleets"} {
			conn := createDatabase(t)
			mustExecBatch(t, conn, "CREATE TRIGGER fail_registration "+trigger+" BEGIN SELECT RAISE(ABORT, 'registration failure'); END;")
			target := newNamingTarget(backend)
			runner := fakeRunnerWithBinary(t, backend.name())
			runner.respond(target.context(t))
			runner.respond(target.renamed(t))
			runner.respond(target.nameReply(t, "cafleet-1-0"))
			runner.respond(target.context(t))
			err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
			target.assertDiagnostic(t, err, some("registration failure"))
			assertCalls(t, backend.name()+" "+trigger, runner.runArgvs(), target.calls("cafleet-1-0"))
		}
	}
}

func TestCliFleet_Create_FleetRowInsertionFailureStopsBeforeRenaming(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		conn := createDatabase(t)
		mustExecBatch(t, conn, "CREATE TRIGGER fail_registration BEFORE INSERT ON fleets BEGIN SELECT RAISE(ABORT, 'fleet insertion failure'); END;")
		target := newNamingTarget(backend)
		runner := fakeRunnerWithBinary(t, backend.name())
		runner.respond(target.context(t))
		err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
		if !strings.Contains(errorMessage(err), "fleet insertion failure") {
			t.Fatalf("%s: %v", backend.name(), err)
		}
		assertCalls(t, backend.name(), runner.runArgvs(), target.calls("cafleet-1-0")[:1])
	}
}

func TestCliFleet_Create_CommitFailureKeepsNamingDiagnosticsAndPerformsNoTerminalCompensation(t *testing.T) {
	t.Parallel()
	for _, backend := range namingBackends {
		conn := createDatabase(t)
		refuse := faultPlanRefuseCommit()
		refuse.arm(t, conn)
		target := newNamingTarget(backend)
		runner := fakeRunnerWithBinary(t, backend.name())
		runner.respond(target.context(t))
		runner.respond(target.renamed(t))
		runner.respond(target.nameReply(t, "cafleet-1-0"))
		runner.respond(target.context(t))
		err := refusedCreate(t, conn, explicitRequest(target.pane, target.server), map[string]string{}, runner, unreadTable{})
		refuse.clear(t, conn)
		target.assertDiagnostic(t, err, some("unconfirmed"))
		assertCalls(t, backend.name(), runner.runArgvs(), target.calls("cafleet-1-0"))
	}
}

func TestCliFleet_Create_PaneAndMuxServerHelpStateTheFormRuleAndVerifiedDefault(t *testing.T) {
	t.Parallel()
	create := cliRoot().subcommand("fleet").subcommand("create")
	help := func(long string) string {
		option, declared := create.option(long)
		if !declared {
			t.Fatalf("fleet create declares --%s", long)
		}
		return option.help
	}
	if got := help("pane"); got != "The Director's pane; its tmux %N or herdr <workspace>:<pane> form selects the multiplexer [default: the caller's TMUX_PANE or HERDR_PANE_ID, verified against the caller's process ancestry]" {
		t.Fatalf("--pane help %q", got)
	}
	if got := help("mux-server"); got != "The Director's tmux socket path or herdr session [default: the caller's server on the pane's multiplexer]" {
		t.Fatalf("--mux-server help %q", got)
	}
}

func TestCliFleet_Create_RegistrationRecordsTheRequestedNameAndCodingAgent(t *testing.T) {
	t.Parallel()
	conn := createDatabase(t)
	runner := fakeRunnerWithBinary(t, "tmux")
	tmuxCreation(runner, "main|@1|%5\n", "@1", "cafleet-1-0")
	request := explicitRequest("%5", createRerunSocket)
	request.agent = codingAgentHermes
	fleet := createdWith(t, conn, request, map[string]string{}, runner, unreadTable{})
	if fleet.name != "alpha" || fleet.director.placement.codingAgent != "hermes" {
		t.Fatalf("fleet %s agent %s", fleet.name, fleet.director.placement.codingAgent)
	}
}
