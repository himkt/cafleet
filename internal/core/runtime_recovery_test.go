package core

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"encoding/json/jsontext"

	"github.com/ncruces/go-sqlite3"
)

const (
	recoveryFixtureNow     = "2026-10-26T20:00:00.000000+00:00"
	descriptorPrivateValue = "descriptor-private-fixture-canary"
)

type recoveryCase struct {
	fixture   *runnerFixture
	artifacts *runArtifacts
	held      *runLock
	settings  settings
	placement placement
}

// newRecoveryCase is a run whose lock the current process holds as its running runner.
func newRecoveryCase(t *testing.T) *recoveryCase {
	t.Helper()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	held := acquireReleasableRunLock(t, fixture.conn, fixture.run.owner())
	c := bindRecoveryCase(t, fixture, artifacts, held)
	if _, err := held.transferToRunner(ctx, fixture.conn, fixture.run.owner(), currentProcess(t)); err != nil {
		t.Fatal(err)
	}
	if _, err := fixture.conn.execute("UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2", recoveryFixtureNow, fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	return c
}

func (c *recoveryCase) refresh(t *testing.T) {
	t.Helper()
	c.fixture.refresh(t)
}

// describedRun is a running case whose launch descriptor carries a private argv value and a
// private environment name.
func describedRun(t *testing.T) *recoveryCase {
	t.Helper()
	c := newRecoveryCase(t)
	spec := c.fixture.spec()
	spec.argv = []string{"/usr/bin/false", descriptorPrivateValue}
	spec.environment = append(spec.environment, [2]string{"CAFLEET_TEST_PRIVATE_VALUE", descriptorPrivateValue})
	descriptor, err := newLaunchDescriptor(c.fixture.run, spec)
	if err != nil {
		t.Fatal(err)
	}
	if err := c.artifacts.writeDescriptor(descriptor); err != nil {
		t.Fatal(err)
	}
	return c
}

type fileIdentity struct {
	device, inode string
	size          int64
	modified      int64
}

func pathIdentity(t *testing.T, path string) fileIdentity {
	t.Helper()
	info, err := os.Lstat(path)
	if err != nil {
		t.Fatal(err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		t.Fatalf("%s has no stat record", path)
	}
	return fileIdentity{device: fmt.Sprint(stat.Dev), inode: fmt.Sprint(stat.Ino), size: info.Size(), modified: info.ModTime().UnixNano()}
}

// ownedBytes reads the descriptor while the current process holds the run's validated lock.
func ownedBytes(t *testing.T, c *recoveryCase) []byte {
	t.Helper()
	ctx := context.Background()
	run, err := getRun(ctx, c.fixture.conn, c.fixture.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	current := currentProcess(t)
	if run.ownerPID != current.pid || run.ownerStartIdentity != current.startIdentity {
		t.Fatalf("the current process owns the run: %d %q", run.ownerPID, run.ownerStartIdentity)
	}
	if err := c.held.validate(ctx, c.fixture.conn, run.owner()); err != nil {
		t.Fatal(err)
	}
	bytes, err := os.ReadFile(c.artifacts.paths.descriptor)
	if err != nil {
		t.Fatal(err)
	}
	return bytes
}

func writePrivate(t *testing.T, path string, bytes []byte) {
	t.Helper()
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.Write(bytes); err != nil {
		t.Fatal(err)
	}
	if err := file.Sync(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}

func secretFree(t *testing.T, text string) {
	t.Helper()
	if strings.Contains(text, descriptorPrivateValue) {
		t.Fatalf("descriptor diagnostics preserve private spawn data: %q", text)
	}
	if strings.Contains(text, "CAFLEET_TEST_PRIVATE_VALUE") {
		t.Fatalf("descriptor diagnostics preserve private environment names: %q", text)
	}
}

func bumpEpoch(t *testing.T, conn *dbConn, runID int64) {
	t.Helper()
	if changed, err := conn.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", runID); err != nil || changed != 1 {
		t.Fatalf("bumpEpoch changed %d rows: %v", changed, err)
	}
}

func currentProcess(t *testing.T) processIdentity {
	t.Helper()
	process, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	return process
}

// recoveryCreatorCase is a run whose lock the current process holds as its pending creator.
func recoveryCreatorCase(t *testing.T) *recoveryCase {
	t.Helper()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	return bindRecoveryCase(t, fixture, artifacts, acquireReleasableRunLock(t, fixture.conn, fixture.run.owner()))
}

// acquireReleasableRunLock acquires the run's lock for a case, whose release or cleanup closes
// it exactly once.
func acquireReleasableRunLock(t *testing.T, conn *dbConn, owner runOwner) *runLock {
	t.Helper()
	acquired, err := runLockAcquire(context.Background(), conn, owner)
	if err != nil {
		t.Fatal(err)
	}
	lock, ok := acquired.get()
	if !ok {
		t.Fatal("the run lock is free")
	}
	return lock
}

// bindRecoveryCase records the case's database settings and its member's placement.
func bindRecoveryCase(t *testing.T, fixture *runnerFixture, artifacts *runArtifacts, held *runLock) *recoveryCase {
	t.Helper()
	c := &recoveryCase{fixture: fixture, artifacts: artifacts, held: held}
	t.Cleanup(func() { c.release(t) })
	c.settings = settingsForDatabase(c.fixture.databaseFile(t))
	found, err := getMember(context.Background(), c.fixture.conn, c.fixture.run.memberID, c.fixture.run.fleetID)
	if err != nil {
		t.Fatal(err)
	}
	member, present := found.get()
	if !present {
		t.Fatal("the case's member is registered")
	}
	c.placement = member.placement
	return c
}

func (c *recoveryCase) seed(t *testing.T, transitions ...observedTransition) {
	t.Helper()
	owner := c.fixture.run.owner()
	for _, transition := range transitions {
		if _, _, err := persistTransition(context.Background(), c.fixture.conn, owner, transition, persistenceModeLive); err != nil {
			t.Fatal(err)
		}
	}
}

// release drops the case's run lock, as the Rust Case releases its held Arc.
func (c *recoveryCase) release(t *testing.T) {
	t.Helper()
	if c.held == nil {
		return
	}
	if err := c.held.close(); err != nil {
		t.Fatal(err)
	}
	c.held = nil
}

// exitedPendingCleanup commits a live runnerExited transition and leaves the run's cleanup
// incomplete, the state an owner leaves when it dies after recording its native exit.
func (c *recoveryCase) exitedPendingCleanup(t *testing.T, code, signal opt[int64]) (observedTransition, runtimeEventRecord) {
	t.Helper()
	transition := recoveryExited(t, c.fixture.run, 1, code, signal)
	event, _, err := persistTransition(context.Background(), c.fixture.conn, c.fixture.run.owner(), transition, persistenceModeLive)
	if err != nil {
		t.Fatal(err)
	}
	if changed, err := sqlExecute(c.fixture.conn, "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1", c.fixture.run.runID); err != nil || changed != 1 {
		t.Fatalf("leaving cleanup incomplete changed %d rows: %v", changed, err)
	}
	c.refresh(t)
	return transition, event
}

// recoverMemberWithHost is the Rust test-only recover_member_with_host.
func recoverMemberWithHost(ctx context.Context, conn *dbConn, s settings, memberID int64, deadline time.Time, host recoveryHost) (recoveryReport, error) {
	completion, err := runtimeRecoveryRecover(ctx, conn, s, memberID, deadline, nil, host)
	return completion.report, err
}

// recoverMemberWithHeldPaneLock is the Rust test-only recover_member_with_held_pane_lock.
func recoverMemberWithHeldPaneLock(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, deadline time.Time, held *paneLock, host recoveryHost) (recoveryCompletion, error) {
	borrowed := &borrowedPane{run: expectedRun, placement: expectedPlacement, guard: held}
	return runtimeRecoveryRecover(ctx, conn, s, expectedRun.memberID, deadline, borrowed, host)
}

func (c *recoveryCase) recover(t *testing.T, host *recoveryTestHost) (recoveryReport, error) {
	t.Helper()
	c.release(t)
	return recoverMemberWithHost(context.Background(), c.fixture.conn, c.settings, c.fixture.run.memberID, host.deadline, host)
}

func (c *recoveryCase) recoverHeld(t *testing.T, expected runRecord, held *paneLock, host *recoveryTestHost) (recoveryCompletion, error) {
	t.Helper()
	return recoverMemberWithHeldPaneLock(context.Background(), c.fixture.conn, c.settings, expected, c.placement, host.deadline, held, host)
}

// withOwnerLock runs read while the current process holds the run's validated original lock,
// reacquiring it when the case released it.
func withOwnerLock[T any](t *testing.T, c *recoveryCase, read func() T) T {
	t.Helper()
	ctx := context.Background()
	run, err := getRun(ctx, c.fixture.conn, c.fixture.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	current := currentProcess(t)
	if run.ownerPID != current.pid || run.ownerStartIdentity != current.startIdentity {
		t.Fatal("the current process owns the run")
	}
	held := c.held
	if held == nil {
		found, err := runLockAcquireReleased(ctx, c.fixture.conn, run.owner())
		if err != nil {
			t.Fatal(err)
		}
		acquired, ok := found.get()
		if !ok {
			t.Fatal("the actual current owner reacquires its original matching lock")
		}
		defer func() {
			if err := acquired.close(); err != nil {
				t.Error(err)
			}
		}()
		held = acquired
	}
	if err := held.validate(ctx, c.fixture.conn, run.owner()); err != nil {
		t.Fatal(err)
	}
	return read()
}

func (c *recoveryCase) paneLock(t *testing.T) *paneLock {
	t.Helper()
	lock, acquired, err := paneLockTryAcquire(c.settings.databaseURL, c.placement)
	if err != nil {
		t.Fatal(err)
	}
	if !acquired {
		t.Fatal("the case's pane lock is free")
	}
	t.Cleanup(lock.close)
	return lock
}

type recoveryCheckpointAction func(boundary string, run runRecord, deadline time.Time) error

// recoveryClockProbe brackets the coordinator's entry clock: afterEntry is taken at the first
// snapshot checkpoint, which then pauses for delay.
type recoveryClockProbe struct {
	beforeEntry time.Time
	afterEntry  opt[time.Time]
	delay       time.Duration
	observed    opt[shutdownDeadlines]
}

type recoveryUnlinkProbe struct {
	path  string
	calls int
	fail  bool
}

// recoveryTestHost is the Rust recovery test_support Host.
type recoveryTestHost struct {
	t              testing.TB
	deadline       time.Time
	process        processEvidence
	matchingHealth bool
	cleanup        cleanupState
	calls          []string
	action         recoveryCheckpointAction
	probe          *recoveryClockProbe
	unlink         *recoveryUnlinkProbe
	clockSource    clock
	virtual        *testClock
}

// newRecoveryHostOnVirtualTime is a host on a virtual clock whose deadline lies as far ahead as
// deadline does now.
func newRecoveryHostOnVirtualTime(t testing.TB, deadline time.Time) *recoveryTestHost {
	clock := virtualClock()
	host := newRecoveryHostOnRealTime(t, deadline)
	host.deadline = clock.now().Add(max(time.Until(deadline), 0))
	host.clockSource = clock
	host.virtual = clock
	return host
}

// newRecoveryHostOnRealTime is a host on real time, for a caller whose deadline is real.
func newRecoveryHostOnRealTime(t testing.TB, deadline time.Time) *recoveryTestHost {
	return &recoveryTestHost{t: t, deadline: deadline, process: processEvidenceTerminated, cleanup: cleanupStateComplete, clockSource: systemClock{}}
}

func newRecoveryHost(t testing.TB) *recoveryTestHost {
	return newRecoveryHostOnVirtualTime(t, time.Now().Add(2*time.Second))
}

func newRecoveryHostWithClock(t testing.TB, deadline time.Time, delay time.Duration) *recoveryTestHost {
	host := newRecoveryHostOnVirtualTime(t, deadline)
	host.probe = &recoveryClockProbe{beforeEntry: host.clockSource.now(), afterEntry: none[time.Time](), delay: delay, observed: none[shutdownDeadlines]()}
	return host
}

func newRecoveryHostWithUnlink(t testing.TB, c *recoveryCase, deadline time.Time, fail bool) *recoveryTestHost {
	host := newRecoveryHostOnVirtualTime(t, deadline)
	host.unlink = &recoveryUnlinkProbe{path: c.artifacts.paths.descriptor, fail: fail}
	return host
}

func (h *recoveryTestHost) virtualClock() *testClock {
	if h.virtual == nil {
		h.t.Fatal("this host runs on the virtual clock")
	}
	return h.virtual
}

func (h *recoveryTestHost) called(name string) bool {
	return slices.Contains(h.calls, name)
}

func (h *recoveryTestHost) assertEntryClock(deadlines shutdownDeadlines) {
	probe := h.probe
	if probe == nil {
		return
	}
	if _, set := probe.observed.get(); set {
		h.t.Fatal("cleanup observes its entry clock once")
	}
	probe.observed = some(deadlines)
	after, set := probe.afterEntry.get()
	if !set {
		h.t.Fatal("actual coordinator snapshot brackets its entry")
	}
	observed := []time.Time{deadlines.childDeadline(), deadlines.displayDeadline(), deadlines.storageDeadline()}
	for index, seconds := range []int{2, 3, 4} {
		offset := time.Duration(seconds) * time.Second
		lower := minTime(probe.beforeEntry.Add(offset), h.deadline)
		upper := minTime(after.Add(offset), h.deadline)
		if observed[index].Before(lower) || observed[index].After(upper) {
			h.t.Fatalf("cleanup carries the one clock captured at common coordinator entry: observed=%v lower=%v upper=%v cap=%v", observed[index], lower, upper, h.deadline)
		}
	}
}

func (h *recoveryTestHost) check(deadline time.Time) error {
	if !deadline.Equal(h.deadline) {
		h.t.Fatalf("all evidence/checkpoints share one original deadline: %v != %v", deadline, h.deadline)
	}
	return runtimeCheckDeadline(h.clockSource.now(), deadline)
}

func (h *recoveryTestHost) clock() clock { return h.clockSource }

func (h *recoveryTestHost) currentProcess() (processIdentity, error) {
	h.calls = append(h.calls, "current_process")
	return processIdentityCurrent()
}

func (h *recoveryTestHost) processEvidence(process processIdentity, deadline time.Time) (processEvidence, error) {
	if err := h.check(deadline); err != nil {
		return 0, err
	}
	h.calls = append(h.calls, fmt.Sprintf("process:%d:%s", process.pid, process.startIdentity))
	return h.process, nil
}

func (h *recoveryTestHost) health(run runRecord, deadline time.Time) (recoveryHealth, error) {
	if err := h.check(deadline); err != nil {
		return nil, err
	}
	h.calls = append(h.calls, "health")
	if !h.matchingHealth {
		return recoveryHealthUnavailable{reason: "bounded injected endpoint unavailable"}, nil
	}
	runnerPID, _ := run.runnerPID.get()
	runnerStart, _ := run.runnerStartIdentity.get()
	snapshot := jsonValue(jsonObject{
		{"identity", controlIdentityFromRun(run)},
		{"runner", jsonObject{{"pid", runnerPID}, {"start_identity", runnerStart}}},
		{"child", nil},
		{"output_seq", 1},
	})
	if _, err := healthSnapshotDecode(snapshot, run); err != nil {
		return nil, err
	}
	return recoveryHealthMatching{}, nil
}

func (h *recoveryTestHost) cleanupOwned(_ runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error) {
	h.assertEntryClock(deadlines)
	for _, bound := range []time.Time{deadlines.childDeadline(), deadlines.displayDeadline(), deadlines.storageDeadline()} {
		if bound.After(h.deadline) {
			h.t.Fatalf("cleanup deadline %v passes the original deadline %v", bound, h.deadline)
		}
	}
	h.calls = append(h.calls, "cleanup")
	outcome := shutdownOutcomeComplete(none[childExit]())
	outcome.cleanupState = h.cleanup
	return outcome, nil
}

func recoveryBoundaryName(boundary recoveryBoundary) string {
	switch boundary {
	case recoveryBoundaryAfterSnapshot:
		return "after_snapshot"
	case recoveryBoundaryBeforeTakeover:
		return "before_takeover"
	case recoveryBoundaryAfterTakeover:
		return "after_takeover"
	case recoveryBoundaryAfterCommit:
		return "after_commit"
	case recoveryBoundaryBeforeFinish:
		return "before_finish"
	default:
		panic("invalid recoveryBoundary")
	}
}

func (h *recoveryTestHost) checkpoint(boundary recoveryBoundary, run runRecord, deadline time.Time) error {
	snapshot := boundary == recoveryBoundaryAfterSnapshot
	if snapshot && h.probe != nil {
		if _, set := h.probe.afterEntry.get(); set {
			h.t.Fatal("the snapshot checkpoint brackets entry once")
		}
		h.probe.afterEntry = some(h.clockSource.now())
	}
	if err := h.check(deadline); err != nil {
		return err
	}
	name := recoveryBoundaryName(boundary)
	h.calls = append(h.calls, name)
	if len(h.calls) >= 1024 {
		h.t.Fatal("a bounded recovery pass makes finite progress")
	}
	if h.action != nil {
		if err := h.action(name, run, deadline); err != nil {
			return err
		}
	}
	if snapshot && h.probe != nil {
		if !h.clockSource.now().Add(h.probe.delay).Before(deadline) {
			h.t.Fatal("the probe delay fits the deadline")
		}
		h.virtualClock().advance(h.probe.delay)
	}
	return nil
}

func (h *recoveryTestHost) unlinkDescriptor(path string, deadline time.Time) error {
	probe := h.unlink
	if probe == nil {
		return unlinkDescriptorFile(path, deadline, h.clockSource)
	}
	probe.calls++
	if path != probe.path {
		h.t.Fatalf("unlink path %q, want %q", path, probe.path)
	}
	if err := h.check(deadline); err != nil {
		return err
	}
	if probe.fail {
		return appError("owned coordinator unlink-dispatch failure")
	}
	if err := os.Remove(path); err != nil {
		return appError("owned coordinator descriptor removal failed")
	}
	return nil
}

func recoveryEvidence(t *testing.T, run runRecord, sequence int64) jsontext.Value {
	t.Helper()
	evidence, err := runEvidence(run, "recovery-fixture", uint64(sequence)) //nolint:gosec // fixture sequences are positive
	if err != nil {
		t.Fatal(err)
	}
	return evidence
}

func recoveryOpening(t *testing.T, run runRecord, sequence, episode int64) observedTransition {
	t.Helper()
	return observedTransition{
		RunID:          run.runID,
		EventSeq:       sequence,
		EpisodeSeq:     some(episode),
		Kind:           runtimeEventKindInteractionOpened,
		Classification: some(interactionClassPermissionRequest),
		ObservedAt:     recoveryFixtureNow,
		EvidenceJSON:   recoveryEvidence(t, run, sequence),
		CloseReason:    none[interactionCloseReason](),
	}
}

func recoveryExited(t *testing.T, run runRecord, sequence int64, code, signal opt[int64]) observedTransition {
	t.Helper()
	return observedTransition{
		RunID:          run.runID,
		EventSeq:       sequence,
		EpisodeSeq:     none[int64](),
		Kind:           runtimeEventKindRunnerExited,
		Classification: none[interactionClass](),
		ObservedAt:     recoveryFixtureNow,
		EvidenceJSON:   recoveryEvidence(t, run, sequence),
		CloseReason:    none[interactionCloseReason](),
		CauseJSON:      jsonValue(jsonObject{{"kind", "native_exit"}, {"exit_code", code}, {"exit_signal", signal}}),
	}
}

func recoveryEvents(t *testing.T, c *recoveryCase) []runtimeEventView {
	t.Helper()
	ctx := context.Background()
	ids, err := queryRows(c.fixture.conn, "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
		[]any{c.fixture.run.runID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		t.Fatal(err)
	}
	views := make([]runtimeEventView, 0, len(ids))
	for _, id := range ids {
		view, err := getEvent(ctx, c.fixture.conn, id)
		if err != nil {
			t.Fatal(err)
		}
		views = append(views, view)
	}
	return views
}

func recoveryEventKinds(t *testing.T, c *recoveryCase) []runtimeEventKind {
	t.Helper()
	kinds := []runtimeEventKind{}
	for _, view := range recoveryEvents(t, c) {
		kinds = append(kinds, view.event.kind)
	}
	return kinds
}

func recoveryRows(t *testing.T, conn *dbConn) []tableSnapshot {
	t.Helper()
	return takeTableSnapshot(t, conn, func(string) bool { return true })
}

func assertRecoveryRows(t *testing.T, conn *dbConn, want []tableSnapshot, context string) {
	t.Helper()
	if got := recoveryRows(t, conn); !tableSnapshotsEqual(got, want) {
		t.Fatalf("%s: rows changed", context)
	}
}

// recoveryInputRow is a prompt receipt on the case's run; deferred stamps deferred_at and
// defer_reason.
type recoveryInputRow struct {
	key            string
	state          string
	disposition    opt[string]
	deliveryReason opt[string]
	deliveryMethod opt[string]
	stopCause      opt[string]
	deferred       bool
}

func (r recoveryInputRow) insert(t *testing.T, c *recoveryCase) int64 {
	t.Helper()
	deferredAt, deferReason := none[string](), none[string]()
	if r.deferred {
		deferredAt, deferReason = some(recoveryFixtureNow), some("interaction_open")
	}
	if _, err := sqlExecute(c.fixture.conn,
		"INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,disposition,delivery_reason,delivery_method,stop_cause,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'prompt',?3,?4,?5,?6,?7,?8,?9,?9,?10,?11)",
		c.fixture.run.runID, r.key, strings.Repeat("d", 64), r.state, r.disposition, r.deliveryReason, r.deliveryMethod, r.stopCause,
		recoveryFixtureNow, deferredAt, deferReason); err != nil {
		t.Fatal(err)
	}
	return c.fixture.conn.raw.LastInsertRowID()
}

func recoveryDeliveryResults(t *testing.T, c *recoveryCase, inputID int64) []jsontext.Value {
	t.Helper()
	payloads, err := queryRows(c.fixture.conn,
		"SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2 ORDER BY event_seq",
		[]any{c.fixture.run.runID, inputID}, func(row sqlRow) (string, error) { return row.text(0) })
	if err != nil {
		t.Fatal(err)
	}
	values := make([]jsontext.Value, 0, len(payloads))
	for _, payload := range payloads {
		values = append(values, jsontext.Value(payload))
	}
	return values
}

func persistRecoveryOpening(t *testing.T, c *recoveryCase) runtimeEventRecord {
	t.Helper()
	event, _, err := persistTransition(context.Background(), c.fixture.conn, c.fixture.run.owner(), recoveryOpening(t, c.fixture.run, 1, 1), persistenceModeLive)
	if err != nil {
		t.Fatal(err)
	}
	return event
}

func missingLock(t *testing.T, c *recoveryCase) {
	t.Helper()
	c.release(t)
	if err := os.Remove(c.artifacts.paths.ownerLock); err != nil {
		t.Fatal(err)
	}
}

func recoveryInput(t *testing.T, conn *dbConn, inputID int64) inputOperation {
	t.Helper()
	return mustInput(t)(getInput(context.Background(), conn, inputID))
}

func jsonTextOf(value jsontext.Value) string {
	if value == nil {
		return "null"
	}
	return string(value)
}

func assertRecoveryLoss(t *testing.T, c *recoveryCase, report recoveryReport, original runRecord) {
	t.Helper()
	if report.outcome != recoveryOutcomeRecovered || report.status != runStatusLost {
		t.Fatalf("report %v/%v, want recovered/lost", report.outcome, report.status)
	}
	current := mustRun(t, c.fixture.conn, original.runID)
	if current.ownerEpoch != original.ownerEpoch+1 || current.ownerKind != ownerKindReconciler {
		t.Fatalf("owner %d/%v after loss", current.ownerEpoch, current.ownerKind)
	}
	events := recoveryEvents(t, c)
	kinds := recoveryEventKinds(t, c)
	if !slices.Equal(kinds, []runtimeEventKind{runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed, runtimeEventKindRunnerLost}) {
		t.Fatalf("event kinds %v", kinds)
	}
	closed, _ := events[1].interaction.get()
	if closed.closeReason != some(interactionCloseReasonMonitorLost) {
		t.Fatalf("close reason %v", closed.closeReason)
	}
	for index, view := range events {
		if view.event.eventSeq != int64(index+1) {
			t.Fatalf("event %d has sequence %d", index, view.event.eventSeq)
		}
	}
	if !strings.Contains(jsonTextOf(current.cleanupDiagnosticsJSON), "runtime_path_missing") {
		t.Fatalf("diagnostics %s lack runtime_path_missing", jsonTextOf(current.cleanupDiagnosticsJSON))
	}
}

func recoverReleased(t *testing.T, c *recoveryCase) (recoveryReport, error) {
	t.Helper()
	return recoverReleasedWithHost(t, c, newRecoveryHost(t))
}

// recoverReleasedWithHost recovers right after the case's owner lock is released. A child forked
// by a concurrent test shares that flock until its exec closes it, which recovery reads as a live
// owner, so it retries that side-effect-free outcome briefly.
func recoverReleasedWithHost(t *testing.T, c *recoveryCase, host *recoveryTestHost) (recoveryReport, error) {
	t.Helper()
	until := minTime(time.Now().Add(releasedLockGrace), time.Now().Add(max(host.deadline.Sub(host.clockSource.now()), 0)))
	for {
		report, err := c.recover(t, host)
		contended := err == nil && report.outcome == recoveryOutcomeLiveOrUnreachable
		if !contended || !time.Now().Before(until) {
			return report, err
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func assertDescriptorIncomplete(t *testing.T, c *recoveryCase, report recoveryReport, err error) {
	t.Helper()
	if err != nil {
		secretFree(t, cafleetErrorOf(err).message())
	} else {
		if report.outcome != recoveryOutcomeRecovering {
			t.Fatalf("descriptor failure remains an error or incomplete outcome, got %v", report.outcome)
		}
		if len(report.cleanupDiagnostics) == 0 {
			t.Fatal("an incomplete descriptor cleanup reports its diagnostics")
		}
		secretFree(t, string(jsonValue(report.cleanupDiagnostics)))
	}
	run := mustRun(t, c.fixture.conn, c.fixture.run.runID)
	if run.cleanupState != cleanupStateIncomplete {
		t.Fatalf("cleanup state %v", run.cleanupState)
	}
	secretFree(t, jsonTextOf(run.cleanupDiagnosticsJSON))
}

type recoveryStage uint8

const (
	recoveryStageOrdinaryTakeover recoveryStage = iota + 1
	recoveryStageMissingPathTakeover
	recoveryStageFinalization
)

func (s recoveryStage) boundary() string {
	if s == recoveryStageFinalization {
		return "before_finish"
	}
	return "before_takeover"
}

func establishedTerminal(t *testing.T) *recoveryCase {
	t.Helper()
	c := newRecoveryCase(t)
	c.exitedPendingCleanup(t, some(int64(7)), none[int64]())
	if c.fixture.run.status != runStatusExited || c.fixture.run.exitCode != some(int64(7)) || c.fixture.run.cleanupState != cleanupStateIncomplete {
		t.Fatal("the terminal fixture records its exit with incomplete cleanup")
	}
	return c
}

func prepareRecoveryStage(t *testing.T, stage recoveryStage) *recoveryCase {
	t.Helper()
	c := establishedTerminal(t)
	if stage == recoveryStageMissingPathTakeover {
		missingLock(t, c)
	}
	return c
}

func invokeRecovery(t *testing.T, c *recoveryCase, host *recoveryTestHost) (recoveryReport, error) {
	t.Helper()
	c.release(t)
	deadline := host.deadline
	return withDatabaseDeadline(context.Background(), c.fixture.conn, deadline, func(ctx context.Context, conn *dbConn) (recoveryReport, error) {
		return recoverMemberWithHost(ctx, conn, c.settings, c.fixture.run.memberID, deadline, host)
	})
}

func openSideConnection(t *testing.T, c *recoveryCase) *dbConn {
	t.Helper()
	conn, err := connect(c.fixture.url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	return conn
}

func recoveryRelationshipRefusal(t *testing.T, stage recoveryStage) {
	t.Helper()
	c := prepareRecoveryStage(t, stage)
	original := c.fixture.run
	target, _ := createFleetFixture(t, c.fixture.conn, "other-relationship-fleet")
	mutation := openSideConnection(t, c)
	var expected []tableSnapshot
	published := false
	host := newRecoveryHost(t)
	host.action = func(boundary string, run runRecord, deadline time.Time) error {
		if boundary != stage.boundary() {
			return nil
		}
		if published {
			t.Fatal("the relationship change happens once")
		}
		wantStatus, wantEpoch := runStatusExited, original.ownerEpoch
		if stage == recoveryStageFinalization {
			wantStatus, wantEpoch = runStatusRecovering, original.ownerEpoch+1
			if run.recoveryOriginStatus != some(runStatusExited) {
				t.Fatalf("finalization origin %v", run.recoveryOriginStatus)
			}
		}
		if run.status != wantStatus || run.cleanupState != cleanupStateIncomplete || run.exitCode != some(int64(7)) || run.ownerEpoch != wantEpoch {
			t.Fatalf("%d: unexpected run at the boundary: %v %v %v %d", stage, run.status, run.cleanupState, run.exitCode, run.ownerEpoch)
		}
		count := queryInt64(t, mutation, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='runner_exited'", run.runID)
		if count != 1 {
			t.Fatalf("terminal truth exists before the injected relationship change: %d", count)
		}
		if changed, err := sqlExecute(mutation, "UPDATE members SET fleet_id=?1 WHERE member_id=?2", target, run.memberID); err != nil || changed != 1 {
			t.Fatalf("moving the member changed %d rows: %v", changed, err)
		}
		expected = recoveryRows(t, mutation)
		published = true
		return nil
	}
	_, err := invokeRecovery(t, c, host)
	if !published {
		t.Fatal("the selected actual coordinator boundary executes")
	}
	assertRecoveryRows(t, c.fixture.conn, expected, fmt.Sprintf("relationship refusal at stage %d", stage))
	if err == nil {
		t.Fatal("a broker relationship mismatch rejects its actual mutation transaction")
	}
	if stage != recoveryStageFinalization && (host.called("after_takeover") || host.called("cleanup")) {
		t.Fatalf("calls %v pass the refused takeover", host.calls)
	}
}

type recoveryLifecycle uint8

const (
	recoveryLifecycleMemberInactive recoveryLifecycle = iota + 1
	recoveryLifecycleDirectorInactive
	recoveryLifecycleFleetDeleted
)

var recoveryLifecycles = []recoveryLifecycle{recoveryLifecycleMemberInactive, recoveryLifecycleDirectorInactive, recoveryLifecycleFleetDeleted}

func (l recoveryLifecycle) apply(t *testing.T, conn *dbConn, run runRecord) {
	t.Helper()
	var changed int64
	var err error
	switch l {
	case recoveryLifecycleMemberInactive:
		changed, err = sqlExecute(conn, "UPDATE members SET status='deregistered' WHERE member_id=?1", run.memberID)
	case recoveryLifecycleDirectorInactive:
		changed, err = sqlExecute(conn, "UPDATE members SET status='deregistered' WHERE member_id=?1", run.directorMemberID)
	case recoveryLifecycleFleetDeleted:
		changed, err = sqlExecute(conn, "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2", recoveryFixtureNow, run.fleetID)
	default:
		panic("invalid recoveryLifecycle")
	}
	if err != nil || changed != 1 {
		t.Fatalf("lifecycle %d changed %d rows: %v", l, changed, err)
	}
}

func (l recoveryLifecycle) archiveReason() archiveReason {
	switch l {
	case recoveryLifecycleMemberInactive:
		return archiveReasonMemberInactive
	case recoveryLifecycleDirectorInactive:
		return archiveReasonDirectorInactive
	case recoveryLifecycleFleetDeleted:
		return archiveReasonFleetDeleted
	default:
		panic("invalid recoveryLifecycle")
	}
}

// endLifecycle ends lifecycle before recovery; the member goes through real deregistration,
// which also deletes its placement.
func endLifecycle(t *testing.T, c *recoveryCase, lifecycle recoveryLifecycle) {
	t.Helper()
	if lifecycle != recoveryLifecycleMemberInactive {
		lifecycle.apply(t, c.fixture.conn, c.fixture.run)
		return
	}
	deregistered, err := deregisterMember(context.Background(), c.fixture.conn, c.fixture.run.memberID)
	if err != nil || !deregistered {
		t.Fatalf("deregistering the member: %v %v", deregistered, err)
	}
}

func retainedHistory(t *testing.T, conn *dbConn) []tableSnapshot {
	t.Helper()
	return takeTableSnapshot(t, conn, func(name string) bool {
		switch name {
		case "messages", "member_runtime_events", "member_interactions", "member_input_operations":
			return true
		default:
			return strings.HasPrefix(name, "member_permission_")
		}
	})
}

func recoveryHistoricalControl(t *testing.T, stage recoveryStage) {
	t.Helper()
	lifecycle := recoveryLifecycleDirectorInactive
	c := prepareRecoveryStage(t, stage)
	original := c.fixture.run
	mutation := openSideConnection(t, c)
	var prior []tableSnapshot
	published := false
	host := newRecoveryHost(t)
	host.action = func(boundary string, run runRecord, _ time.Time) error {
		if boundary == stage.boundary() {
			if published {
				t.Fatal("the lifecycle change happens once")
			}
			lifecycle.apply(t, mutation, run)
			prior = retainedHistory(t, mutation)
			published = true
		}
		return nil
	}
	report, err := invokeRecovery(t, c, host)
	if err != nil {
		t.Fatal(err)
	}
	if !published || report.outcome != recoveryOutcomeRecovered || report.status != runStatusExited {
		t.Fatalf("stage %d: published=%v report %v/%v", stage, published, report.outcome, report.status)
	}
	run := mustRun(t, c.fixture.conn, original.runID)
	if run.ownerEpoch != original.ownerEpoch+1 || run.ownerKind != ownerKindReconciler || run.status != runStatusExited ||
		run.exitCode != some(int64(7)) || run.cleanupState != cleanupStateComplete {
		t.Fatalf("stage %d: finalized run %d/%v/%v/%v/%v", stage, run.ownerEpoch, run.ownerKind, run.status, run.exitCode, run.cleanupState)
	}
	if !tableSnapshotsEqual(retainedHistory(t, c.fixture.conn), prior) {
		t.Fatalf("stage %d: retained history changed", stage)
	}
	events := recoveryEvents(t, c)
	if len(events) != 1 || events[0].event.notificationState != notificationStateInbox {
		t.Fatalf("stage %d: events %d", stage, len(events))
	}
	if stage == recoveryStageMissingPathTakeover && host.called("cleanup") {
		t.Fatal("a missing-path takeover runs no cleanup")
	}
}

type artifactWitness struct {
	path     string
	identity fileIdentity
}

func loseArtifacts(t *testing.T, c *recoveryCase, directory bool) artifactWitness {
	t.Helper()
	launch, err := newLaunchDescriptor(c.fixture.run, c.fixture.spec())
	if err != nil {
		t.Fatal(err)
	}
	if err := c.artifacts.writeDescriptor(launch); err != nil {
		t.Fatal(err)
	}
	descriptor := c.artifacts.paths.descriptor
	witness := artifactWitness{path: descriptor, identity: pathIdentity(t, descriptor)}
	c.release(t)
	if directory {
		retained := filepath.Join(c.fixture.dir, "retained-runtime")
		if err := os.Rename(c.artifacts.paths.runDir, retained); err != nil {
			t.Fatal(err)
		}
		witness.path = filepath.Join(retained, filepath.Base(descriptor))
	} else if err := os.Remove(c.artifacts.paths.ownerLock); err != nil {
		t.Fatal(err)
	}
	return witness
}

func originalTerminal(t *testing.T, status runStatus) *recoveryCase {
	t.Helper()
	c := newRecoveryCase(t)
	c.seed(t, observedTransition{
		RunID:          c.fixture.run.runID,
		EventSeq:       1,
		EpisodeSeq:     none[int64](),
		Kind:           runtimeEventKindCoverageUnknown,
		Classification: none[interactionClass](),
		ObservedAt:     recoveryFixtureNow,
		EvidenceJSON:   recoveryEvidence(t, c.fixture.run, 1),
		CloseReason:    none[interactionCloseReason](),
		CauseJSON:      jsonValue(jsonObject{{"kind", "coverage_unknown"}}),
	})
	exitCode := none[int64]()
	if status == runStatusExited {
		exitCode = some(int64(7))
	}
	if _, err := sqlExecute(c.fixture.conn,
		"UPDATE member_runs SET status=?1,cleanup_state='incomplete',exit_code=?2,ended_at=?3,cleanup_diagnostics_json=?4 WHERE run_id=?5",
		status.asStr(), exitCode, recoveryFixtureNow, `[{"kind":"retained_terminal_history"}]`, c.fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	c.refresh(t)
	return c
}

func recoverMissing(t *testing.T, c *recoveryCase) (recoveryReport, error) {
	t.Helper()
	host := newRecoveryHost(t)
	report, err := recoverMemberWithHost(context.Background(), c.fixture.conn, c.settings, c.fixture.run.memberID, host.deadline, host)
	if host.called("cleanup") {
		t.Fatal("database-only recovery runs no cleanup")
	}
	return report, err
}

func assertTerminalBookkeeping(t *testing.T, c *recoveryCase, report recoveryReport, before runRecord, status runStatus, prior []tableSnapshot, artifact artifactWitness) {
	t.Helper()
	current := mustRun(t, c.fixture.conn, before.runID)
	process := currentProcess(t)
	origin := before.recoveryOriginStatus
	if _, set := origin.get(); !set {
		origin = some(before.status)
	}
	if report.outcome != recoveryOutcomeRecovered || report.status != status || current.status != status ||
		current.ownerEpoch != before.ownerEpoch+1 || current.ownerKind != ownerKindReconciler ||
		current.ownerPID != process.pid || current.ownerStartIdentity != process.startIdentity ||
		current.exitCode != before.exitCode || current.cleanupState != cleanupStateComplete || current.recoveryOriginStatus != origin {
		t.Fatalf("terminal bookkeeping: report %v/%v run %+v", report.outcome, report.status, current.status)
	}
	if !tableSnapshotsEqual(retainedHistory(t, c.fixture.conn), prior) {
		t.Fatal("terminal bookkeeping keeps retained history")
	}
	if pathIdentity(t, artifact.path) != artifact.identity {
		t.Fatal("database-only bookkeeping preserves surviving runtime artifact identity and metadata")
	}
	if !strings.Contains(jsonTextOf(current.cleanupDiagnosticsJSON), "runtime_path_missing") {
		t.Fatalf("diagnostics %s", jsonTextOf(current.cleanupDiagnosticsJSON))
	}
}

func originalHistoryCases(t *testing.T, status runStatus) {
	t.Helper()
	for _, directory := range []bool{false, true} {
		c := originalTerminal(t, status)
		before := c.fixture.run
		prior := retainedHistory(t, c.fixture.conn)
		artifact := loseArtifacts(t, c, directory)
		report, err := recoverMissing(t, c)
		if err != nil {
			t.Fatalf("directory=%v: %v", directory, err)
		}
		assertTerminalBookkeeping(t, c, report, before, status, prior, artifact)
		if !strings.Contains(jsonTextOf(mustRun(t, c.fixture.conn, before.runID).cleanupDiagnosticsJSON), "retained_terminal_history") {
			t.Fatalf("directory=%v: the prior diagnostics survive", directory)
		}
	}
}

func TestRuntimeRecovery_UnpublishedHistory_RunnerRecoveryMachineMissingPathsPreserveOriginalExitedAndFailedHistory(t *testing.T) {
	t.Parallel()
	originalHistoryCases(t, runStatusExited)
	originalHistoryCases(t, runStatusFailed)
}

func TestRuntimeRecovery_UnpublishedHistory_RunnerRecoveryMachineMissingPathsKeepCommittedNativeExitCodeSignalAndMessage(t *testing.T) {
	t.Parallel()
	for _, directory := range []bool{false, true} {
		c := newRecoveryCase(t)
		c.exitedPendingCleanup(t, none[int64](), some(int64(15)))
		before := c.fixture.run
		prior := retainedHistory(t, c.fixture.conn)
		artifact := loseArtifacts(t, c, directory)
		report, err := recoverMissing(t, c)
		if err != nil {
			t.Fatal(err)
		}
		assertTerminalBookkeeping(t, c, report, before, runStatusExited, prior, artifact)
		if len(recoveryEvents(t, c)) != 1 {
			t.Fatalf("directory=%v: one committed event remains", directory)
		}
	}
}

func TestRuntimeRecovery_UnpublishedHistory_RunnerRecoveryTerminalBookkeepingFaultsRollbackAtomicallyThenRetryWithoutEvents(t *testing.T) {
	t.Parallel()
	faults := []struct {
		name string
		plan faultPlan
	}{
		{"owner", faultPlanAbort("fail_history", "BEFORE UPDATE OF owner_epoch ON member_runs", "owned history owner fault")},
		{"diagnostic", faultPlanAbort("fail_history", "BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs", "owned history diagnostic fault")},
		{"finish", faultPlanAbort("fail_history", "BEFORE UPDATE OF cleanup_state ON member_runs WHEN NEW.cleanup_state='complete'", "owned history finish fault")},
		{"commit", faultPlanRefuseCommit()},
	}
	for _, fault := range faults {
		c := originalTerminal(t, runStatusFailed)
		beforeRun := c.fixture.run
		prior := retainedHistory(t, c.fixture.conn)
		artifact := loseArtifacts(t, c, false)
		fault.plan.arm(t, c.fixture.conn)
		before := recoveryRows(t, c.fixture.conn)
		_, err := recoverMissing(t, c)
		fault.plan.clear(t, c.fixture.conn)
		if err == nil {
			t.Fatalf("%s: the fault fails recovery", fault.name)
		}
		assertRecoveryRows(t, c.fixture.conn, before, fault.name)
		if pathIdentity(t, artifact.path) != artifact.identity {
			t.Fatalf("%s: the artifact survives", fault.name)
		}
		if fault.name == "commit" {
			report, err := recoverMissing(t, c)
			if err != nil {
				t.Fatal(err)
			}
			assertTerminalBookkeeping(t, c, report, beforeRun, runStatusFailed, prior, artifact)
		}
	}
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryMissingPathMachineTerminationCommitsOneAtomicDatabaseOnlyLoss(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	original := c.fixture.run
	missingLock(t, c)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	assertRecoveryLoss(t, c, report, original)
	if _, err := os.Lstat(c.artifacts.paths.ownerLock); err == nil {
		t.Fatal("database-only recovery does not recreate the owner lock")
	}
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryOfAReleasedRunnerClosesItsInteractionAndRecordsOneRunnerLost(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	c.release(t)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered || report.status != runStatusLost {
		t.Fatalf("report %v/%v", report.outcome, report.status)
	}
	expected := []runtimeEventKind{runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed, runtimeEventKindRunnerLost}
	if kinds := recoveryEventKinds(t, c); !slices.Equal(kinds, expected) {
		t.Fatalf("kinds %v", kinds)
	}
	closed, _ := recoveryEvents(t, c)[1].interaction.get()
	if closed.closeReason != some(interactionCloseReasonMonitorLost) {
		t.Fatalf("close reason %v", closed.closeReason)
	}
	_, _ = c.recover(t, newRecoveryHost(t))
	if kinds := recoveryEventKinds(t, c); !slices.Equal(kinds, expected) {
		t.Fatalf("a second pass records nothing new: %v", kinds)
	}
}

func deferredReceipt(t *testing.T, c *recoveryCase, key string, settledReason opt[string]) int64 {
	t.Helper()
	row := recoveryInputRow{key: key, state: "ready", deferred: true}
	if reason, settled := settledReason.get(); settled {
		row.state, row.disposition, row.deliveryReason = "settled", some("not_submitted"), some(reason)
	}
	return row.insert(t, c)
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryMissingPathSettlesQueuedReceiptsRunnerLostAndPublishesDurableResultsOnce(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	queued := deferredReceipt(t, c, "prompt:queued", none[string]())
	unpublished := deferredReceipt(t, c, "prompt:unpublished", some("runner_stopped"))
	uncertain := recoveryInputRow{key: "prompt:uncertain", state: "uncertain", deliveryMethod: some("profile"), stopCause: some("runner_stopped"), deferred: true}.insert(t, c)
	missingLock(t, c)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered || report.status != runStatusLost {
		t.Fatalf("report %v/%v", report.outcome, report.status)
	}
	lost := recoveryInput(t, c.fixture.conn, queued)
	if lost.state != inputStateSettled || lost.disposition != some(inputDispositionNotSubmitted) || lost.deliveryReason != some("runner_lost") {
		t.Fatalf("queued receipt %v/%v/%v", lost.state, lost.disposition, lost.deliveryReason)
	}
	if reason := recoveryInput(t, c.fixture.conn, unpublished).deliveryReason; reason != some("runner_stopped") {
		t.Fatalf("unpublished reason %v", reason)
	}
	for _, inputID := range []int64{queued, unpublished, uncertain} {
		if got := len(recoveryDeliveryResults(t, c, inputID)); got != 1 {
			t.Fatalf("input %d has %d results", inputID, got)
		}
	}
	publishedCause := func(inputID int64) (string, bool) {
		return jsonString(jsonIndex(jsonIndex(recoveryDeliveryResults(t, c, inputID)[0], "cause"), "kind"))
	}
	if kind, set := publishedCause(uncertain); !set || kind != "runner_stopped" {
		t.Fatalf("uncertain cause %q", kind)
	}
	for _, inputID := range []int64{queued, unpublished} {
		if _, set := publishedCause(inputID); set {
			t.Fatalf("input %d publishes no cause", inputID)
		}
	}
	if _, err := c.recover(t, newRecoveryHost(t)); err != nil {
		t.Fatal(err)
	}
	for _, inputID := range []int64{queued, unpublished, uncertain} {
		if got := len(recoveryDeliveryResults(t, c, inputID)); got != 1 {
			t.Fatalf("a repeated recovery publishes input %d once, got %d", inputID, got)
		}
	}
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryMissingPathUsesCurrentLifecycleForAtomicArchivedClosureAndLoss(t *testing.T) {
	t.Parallel()
	for _, lifecycle := range recoveryLifecycles {
		c := newRecoveryCase(t)
		opening := persistRecoveryOpening(t, c)
		endLifecycle(t, c, lifecycle)
		missingLock(t, c)
		if _, err := c.recover(t, newRecoveryHost(t)); err != nil {
			t.Fatalf("lifecycle %d: %v", lifecycle, err)
		}
		committed := recoveryEvents(t, c)
		if len(committed) != 3 || committed[0].event.eventID != opening.eventID {
			t.Fatalf("lifecycle %d: committed %d events", lifecycle, len(committed))
		}
		for _, view := range committed[1:] {
			_, messaged := view.event.messageID.get()
			if view.event.notificationState != notificationStateArchived || messaged || view.event.archiveReason != some(lifecycle.archiveReason()) {
				t.Fatalf("lifecycle %d: event %d is not archived for its lifecycle", lifecycle, view.event.eventSeq)
			}
		}
	}
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryMissingPathFaultsRollbackOwnerProvenanceCountersEpisodeAndEventsTogether(t *testing.T) {
	t.Parallel()
	faults := []struct {
		name string
		plan faultPlan
	}{
		{"owner", faultPlanAbort("fault", "BEFORE UPDATE OF owner_epoch ON member_runs", "injected owner failure")},
		{"provenance", faultPlanAbort("fault", "BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs", "injected provenance failure")},
		{"closure", faultPlanAbort("fault", "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='interaction_closed'", "injected closure failure")},
		{"loss", faultPlanAbort("fault", "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='runner_lost'", "injected loss failure")},
		{"terminal", faultPlanAbort("fault", "BEFORE UPDATE OF status ON member_runs WHEN NEW.status='lost'", "injected terminal failure")},
		{"commit", faultPlanRefuseCommit()},
	}
	for _, fault := range faults {
		c := newRecoveryCase(t)
		persistRecoveryOpening(t, c)
		missingLock(t, c)
		fault.plan.arm(t, c.fixture.conn)
		before := recoveryRows(t, c.fixture.conn)
		_, err := c.recover(t, newRecoveryHost(t))
		fault.plan.clear(t, c.fixture.conn)
		if err == nil {
			t.Fatalf("%s: the fault fails recovery", fault.name)
		}
		assertRecoveryRows(t, c.fixture.conn, before, fault.name)
	}
}

func TestRuntimeRecovery_UnpublishedMissingPaths_RunnerRecoveryMissingPathRevalidatesEpochAfterObtainingTheActualWriteLock(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	missingLock(t, c)
	original := c.fixture.run
	start := make(chan struct{}, 1)
	held := make(chan struct{}, 1)
	started := false
	host := newRecoveryHost(t)
	host.action = func(boundary string, _ runRecord, _ time.Time) error {
		if boundary == "before_takeover" && !started {
			started = true
			start <- struct{}{}
			select {
			case <-held:
			case <-time.After(2 * time.Second):
				t.Error("the writer holds its lock in time")
			}
		}
		return nil
	}
	writerDone := make(chan error, 1)
	go func() {
		writerDone <- holdWriteLockBumpingEpoch(c.fixture.url, original.runID, start, held, 80*time.Millisecond)
	}()
	_, err := c.recover(t, host)
	if writerErr := <-writerDone; writerErr != nil {
		t.Fatal(writerErr)
	}
	if err == nil {
		t.Fatal("a stale epoch behind the write lock fails recovery")
	}
	current := mustRun(t, c.fixture.conn, original.runID)
	if current.ownerEpoch != original.ownerEpoch+1 || current.ownerKind != original.ownerKind || current.status != original.status ||
		!jsonOptionalEqual(current.cleanupDiagnosticsJSON, original.cleanupDiagnosticsJSON) {
		t.Fatal("only the foreign epoch change committed")
	}
	if len(recoveryEvents(t, c)) != 1 {
		t.Fatal("no recovery event committed")
	}
}

// holdWriteLockBumpingEpoch opens its own connection once start arrives, bumps the run's epoch
// inside an immediate transaction, signals held and commits after hold.
func holdWriteLockBumpingEpoch(url string, runID int64, start <-chan struct{}, held chan<- struct{}, hold time.Duration) error {
	select {
	case <-start:
	case <-time.After(time.Second):
		return appError("the recovery never reached its boundary")
	}
	writer, err := connect(url)
	if err != nil {
		return err
	}
	defer writer.close() //nolint:errcheck // a side connection of the test
	if err := writer.executeBatch("BEGIN IMMEDIATE"); err != nil {
		return err
	}
	if _, err := writer.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", runID); err != nil {
		return err
	}
	held <- struct{}{}
	time.Sleep(hold)
	return writer.executeBatch("COMMIT")
}

func TestRuntimeRecovery_Descriptor_RunnerRecoveryVerifiedRecoveryRemovesItsPrivateDescriptor(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered || report.status != runStatusLost {
		t.Fatalf("report %v/%v", report.outcome, report.status)
	}
	secretFree(t, string(jsonValue(report.cleanupDiagnostics)))
	if _, err := os.Lstat(c.artifacts.paths.descriptor); err == nil {
		t.Fatal("successful creator cleanup removes its launch descriptor")
	}
}

func TestRuntimeRecovery_Descriptor_RunnerRecoveryHistoricalDescriptorSurvivesReconcilerRetriesUntilVerifiedRemoval(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	bytes := ownedBytes(t, c)
	originalEpoch := c.fixture.run.ownerEpoch
	for increment := int64(1); increment <= 2; increment++ {
		host := newRecoveryHost(t)
		host.action = func(boundary string, _ runRecord, _ time.Time) error {
			if boundary == "before_finish" {
				return appError("owned descriptor completion interruption")
			}
			return nil
		}
		if _, err := c.recover(t, host); err == nil {
			t.Fatal("the interrupted finish fails recovery")
		}
		if !host.called("before_finish") {
			t.Fatal("recovery reaches its finish")
		}
		c.refresh(t)
		if c.fixture.run.ownerEpoch != originalEpoch+increment || c.fixture.run.cleanupState != cleanupStateIncomplete {
			t.Fatalf("retry %d: epoch %d cleanup %v", increment, c.fixture.run.ownerEpoch, c.fixture.run.cleanupState)
		}
		if string(withOwnerLock(t, c, func() []byte { return readFile(t, c.artifacts.paths.descriptor) })) != string(bytes) {
			t.Fatal("interrupted cleanup retains its original descriptor")
		}
		if len(recoveryEvents(t, c)) != 1 {
			t.Fatal("an interrupted cleanup records no event")
		}
	}
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered || len(recoveryEvents(t, c)) != 1 {
		t.Fatal("the final retry recovers without a new event")
	}
	if _, err := os.Lstat(c.artifacts.paths.descriptor); err == nil {
		t.Fatal("a positive historical epoch remains removable after legitimate takeovers")
	}
}

func readFile(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestRuntimeRecovery_Descriptor_RunnerRecoveryForeignEpochBeforeUnlinkPreservesDescriptor(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	original := ownedBytes(t, c)
	side := openSideConnection(t, c)
	host := newRecoveryHost(t)
	host.action = func(boundary string, run runRecord, _ time.Time) error {
		if boundary != "before_finish" {
			return nil
		}
		found, err := runLockAcquire(context.Background(), side, run.owner())
		if err != nil {
			t.Fatal(err)
		}
		if _, held := found.get(); held {
			t.Fatal("recovery holds the run lock at its finish")
		}
		bumpEpoch(t, side, run.runID)
		return nil
	}
	report, err := c.recover(t, host)
	if !host.called("before_finish") {
		t.Fatal("recovery reaches its finish")
	}
	assertDescriptorIncomplete(t, c, report, err)
	if string(withOwnerLock(t, c, func() []byte { return readFile(t, c.artifacts.paths.descriptor) })) != string(original) {
		t.Fatal("the refused descriptor remains unchanged")
	}
}

func TestRuntimeRecovery_Descriptor_RunnerRecoveryInvalidDescriptorIdentityEpochShapeOrPermissionsFailsPrivately(t *testing.T) {
	t.Parallel()
	for _, damage := range []string{"nonce", "symlink"} {
		c := describedRun(t)
		value := jsontext.Value(ownedBytes(t, c))
		if damage == "nonce" {
			value = jsonWith(value, "launch_nonce", strings.Repeat("b", 64))
		}
		if err := os.WriteFile(c.artifacts.paths.descriptor, value, 0o600); err != nil {
			t.Fatal(err)
		}
		if damage == "symlink" {
			retained := filepath.Join(c.artifacts.paths.runDir, "invalid-target.json")
			if err := os.Rename(c.artifacts.paths.descriptor, retained); err != nil {
				t.Fatal(err)
			}
			if err := os.Symlink(retained, c.artifacts.paths.descriptor); err != nil {
				t.Fatal(err)
			}
		}
		before := pathIdentity(t, c.artifacts.paths.descriptor)
		report, err := recoverReleased(t, c)
		assertDescriptorIncomplete(t, c, report, err)
		if pathIdentity(t, c.artifacts.paths.descriptor) != before {
			t.Fatalf("%s: the damaged descriptor is preserved", damage)
		}
	}
}

func TestRuntimeRecovery_Descriptor_RunnerRecoveryMissingPathDatabaseOnlyReconciliationPreservesSuspectDescriptor(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	before := pathIdentity(t, c.artifacts.paths.descriptor)
	missingLock(t, c)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered || pathIdentity(t, c.artifacts.paths.descriptor) != before {
		t.Fatal("database-only reconciliation keeps the suspect descriptor")
	}
	secretFree(t, string(jsonValue(report.cleanupDiagnostics)))
}

func eventRecords(t *testing.T, c *recoveryCase) []runtimeEventRecord {
	t.Helper()
	records := []runtimeEventRecord{}
	for _, view := range recoveryEvents(t, c) {
		records = append(records, view.event)
	}
	return records
}

func TestRuntimeRecovery_DescriptorCleanup_RunnerRecoveryCoordinatorPropagatesActualUnlinkFailureAndRetriesHistoricalEpoch(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	bytes := ownedBytes(t, c)
	failed := newRecoveryHostWithUnlink(t, c, time.Now().Add(2*time.Second), true)
	report, err := c.recover(t, failed)
	if failed.unlink.calls != 1 {
		t.Fatalf("the real coordinator reaches the approved unlink operation: %d", failed.unlink.calls)
	}
	assertDescriptorIncomplete(t, c, report, err)
	if string(withOwnerLock(t, c, func() []byte { return readFile(t, c.artifacts.paths.descriptor) })) != string(bytes) {
		t.Fatal("the failed dispatch preserves its selected descriptor")
	}
	first := eventRecords(t, c)
	if len(first) != 1 {
		t.Fatalf("first pass committed %d events", len(first))
	}
	epoch := mustRun(t, c.fixture.conn, c.fixture.run.runID).ownerEpoch
	retry := newRecoveryHostWithUnlink(t, c, time.Now().Add(2*time.Second), false)
	report, err = c.recover(t, retry)
	if err != nil {
		t.Fatal(err)
	}
	if retry.unlink.calls != 1 || report.outcome != recoveryOutcomeRecovered {
		t.Fatalf("retry unlink calls %d outcome %v", retry.unlink.calls, report.outcome)
	}
	if !slices.Equal(eventRecordIDs(eventRecords(t, c)), eventRecordIDs(first)) {
		t.Fatal("the retry records no new event")
	}
	if mustRun(t, c.fixture.conn, c.fixture.run.runID).ownerEpoch != epoch+1 {
		t.Fatal("the retry takes over one epoch further")
	}
	if _, err := os.Lstat(c.artifacts.paths.descriptor); err == nil {
		t.Fatal("the retry removes the descriptor")
	}
}

func eventRecordIDs(records []runtimeEventRecord) []int64 {
	ids := []int64{}
	for _, record := range records {
		ids = append(ids, record.eventID)
	}
	return ids
}

func TestRuntimeRecovery_DescriptorCleanup_RunnerRecoveryExpiredOriginalBudgetPreventsActualUnlinkDispatch(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	bytes := ownedBytes(t, c)
	host := newRecoveryHostWithUnlink(t, c, time.Now().Add(500*time.Millisecond), false)
	clock := host.virtualClock()
	host.action = func(boundary string, _ runRecord, deadline time.Time) error {
		if boundary == "before_finish" {
			clock.advance(max(deadline.Sub(clock.now()), 0))
		}
		return nil
	}
	report, err := c.recover(t, host)
	if !host.called("before_finish") || host.clockSource.now().Before(host.deadline) {
		t.Fatal("recovery reaches its finish at the expired deadline")
	}
	if host.unlink.calls != 0 {
		t.Fatal("the expired deadline is checked before forwarding the unlink seam")
	}
	assertDescriptorIncomplete(t, c, report, err)
	if string(withOwnerLock(t, c, func() []byte { return readFile(t, c.artifacts.paths.descriptor) })) != string(bytes) {
		t.Fatal("expiry preserves the selected descriptor")
	}
}

func TestRuntimeRecovery_Identity_RunnerRecoveryCreatorFreePrecreatedLockIsLiveOrUnreachable(t *testing.T) {
	t.Parallel()
	c := recoveryCreatorCase(t)
	before := recoveryRows(t, c.fixture.conn)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeLiveOrUnreachable {
		t.Fatalf("outcome %v", report.outcome)
	}
	assertRecoveryRows(t, c.fixture.conn, before, "creator-free lock")
}

func TestRuntimeRecovery_Identity_RunnerRecoveryBorrowedTerminalNoWorkValidatesFixedStartingIdentity(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	if _, err := sqlExecute(c.fixture.conn, "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at=?1 WHERE run_id=?2", recoveryFixtureNow, c.fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	c.refresh(t)
	expected := c.fixture.run
	held := c.paneLock(t)
	c.release(t)
	bumpEpoch(t, c.fixture.conn, expected.runID)
	before := recoveryRows(t, c.fixture.conn)
	if _, err := c.recoverHeld(t, expected, held, newRecoveryHost(t)); err == nil {
		t.Fatal("a stale borrowed identity is refused")
	}
	assertRecoveryRows(t, c.fixture.conn, before, "stale borrowed identity")
	c.refresh(t)
	matching := c.fixture.run
	completion, err := c.recoverHeld(t, matching, held, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if !completion.run.equal(matching) {
		t.Fatal("a terminal run's completion is the run itself")
	}
	assertRecoveryRows(t, c.fixture.conn, before, "terminal no-work recovery")
}

func TestRuntimeRecovery_Identity_RunnerRecoveryBorrowedExpectationsRejectStableRunAndConcretePlacementMismatches(t *testing.T) {
	t.Parallel()
	for _, field := range []string{"nonce", "member", "pane"} {
		c := newRecoveryCase(t)
		expected := c.fixture.run
		p := c.placement
		switch field {
		case "nonce":
			expected.launchNonce = strings.Repeat("b", 64)
		case "member":
			expected.memberID++
		case "pane":
			p.muxPaneID = some("%foreign")
		}
		held := c.paneLock(t)
		c.release(t)
		before := recoveryRows(t, c.fixture.conn)
		host := newRecoveryHost(t)
		if _, err := recoverMemberWithHeldPaneLock(context.Background(), c.fixture.conn, c.settings, expected, p, host.deadline, held, host); err == nil {
			t.Fatalf("%s: the mismatched expectation is refused", field)
		}
		assertRecoveryRows(t, c.fixture.conn, before, field)
	}
}

func TestRuntimeRecovery_Identity_StableRunIdentityChangesWithEachStableFieldAndIgnoresMutableState(t *testing.T) {
	t.Parallel()
	run := newRunnerFixture(t).run
	stable := stableRunIdentity(run)
	changes := []struct {
		field  string
		change func(*runRecord)
	}{
		{"database", func(r *runRecord) {
			r.databaseIdentityJSON = jsonWith(r.databaseIdentityJSON, "instance_id", strings.Repeat("0", 32))
		}},
		{"fleet", func(r *runRecord) { r.fleetID++ }},
		{"member", func(r *runRecord) { r.memberID++ }},
		{"director", func(r *runRecord) { r.directorMemberID++ }},
		{"backend", func(r *runRecord) { r.backend += "-foreign" }},
		{"run", func(r *runRecord) { r.runID++ }},
		{"nonce", func(r *runRecord) { r.launchNonce = strings.Repeat("b", 64) }},
		{"epoch", func(r *runRecord) { r.ownerEpoch++ }},
		{"owner_kind", func(r *runRecord) {
			if r.ownerKind == ownerKindReconciler {
				r.ownerKind = ownerKindRunner
			} else {
				r.ownerKind = ownerKindReconciler
			}
		}},
		{"owner_pid", func(r *runRecord) { r.ownerPID++ }},
		{"owner_start", func(r *runRecord) { r.ownerStartIdentity += "-foreign" }},
		{"runtime_path", func(r *runRecord) { r.runtimePath += "-foreign" }},
		{"directory", func(r *runRecord) { r.runtimeDirIdentityJSON = jsontext.Value(`{"device":0,"inode":0}`) }},
		{"lock", func(r *runRecord) { r.runLockIdentityJSON = jsontext.Value(`{"device":0,"inode":0}`) }},
		{"runner_pid", func(r *runRecord) { r.runnerPID = some(int64(1)) }},
		{"runner_start", func(r *runRecord) { r.runnerStartIdentity = some("foreign-generation") }},
		{"child_pid", func(r *runRecord) { r.childPID = some(int64(1)) }},
		{"child_start", func(r *runRecord) { r.childStartIdentity = some("foreign-generation") }},
	}
	for _, change := range changes {
		changed := run
		change.change(&changed)
		if jsonEqual(stableRunIdentity(changed), stable) {
			t.Fatalf("%s: the stable identity changes", change.field)
		}
	}
	mutable := run
	mutable.lastHealthAt = some("2000-01-01T00:00:00Z")
	mutable.cleanupDiagnosticsJSON = jsontext.Value(`["old bounded diagnostic"]`)
	if !jsonEqual(stableRunIdentity(mutable), stable) {
		t.Fatal("mutable state leaves the stable identity unchanged")
	}
}

func TestRuntimeRecovery_Identity_RunnerRecoveryAfterSnapshotPrecedesRealPaneWaitAndRevalidatesStableOwner(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	pane := c.paneLock(t)
	originalEpoch := c.fixture.run.ownerEpoch
	runID := c.fixture.run.runID
	side := openSideConnection(t, c)
	signal := make(chan struct{}, 1)
	signalled := false
	host := newRecoveryHost(t)
	host.action = func(boundary string, run runRecord, _ time.Time) error {
		if boundary == "after_snapshot" && !signalled {
			signalled = true
			if run.ownerEpoch != originalEpoch {
				t.Errorf("the snapshot sees epoch %d", run.ownerEpoch)
			}
			signal <- struct{}{}
		}
		if boundary == "after_takeover" && run.ownerEpoch < originalEpoch+2 {
			t.Errorf("the takeover follows the foreign epoch: %d", run.ownerEpoch)
		}
		return nil
	}
	mutated := make(chan struct{})
	go func() {
		defer close(mutated)
		select {
		case <-signal:
		case <-time.After(time.Second):
			t.Error("recovery reached its snapshot")
			return
		}
		if _, err := side.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", runID); err != nil {
			t.Error(err)
		}
		pane.close()
	}()
	report, err := c.recover(t, host)
	<-mutated
	if !host.called("after_snapshot") {
		t.Fatal("recovery reaches its snapshot")
	}
	if err == nil {
		if report.status != runStatusLost {
			t.Fatalf("status %v", report.status)
		}
	} else if len(recoveryEvents(t, c)) != 0 {
		t.Fatal("a refused recovery records no event")
	}
}

func recoveryClockCase(t *testing.T, borrowed bool, capDuration, delay time.Duration) {
	t.Helper()
	c := newRecoveryCase(t)
	c.seed(t, recoveryOpening(t, c.fixture.run, 1, 1))
	expected := c.fixture.run
	var held *paneLock
	if borrowed {
		held = c.paneLock(t)
	}
	c.release(t)
	host := newRecoveryHostWithClock(t, time.Now().Add(capDuration), delay)
	until := host.deadline
	var report recoveryReport
	var err error
	if borrowed {
		report, err = withDatabaseDeadline(context.Background(), c.fixture.conn, until, func(ctx context.Context, conn *dbConn) (recoveryReport, error) {
			host.virtualClock().advance(100 * time.Millisecond)
			host.probe.beforeEntry = host.clockSource.now()
			completion, err := recoverMemberWithHeldPaneLock(ctx, conn, c.settings, expected, c.placement, until, held, host)
			return completion.report, err
		})
	} else {
		host.probe.beforeEntry = host.clockSource.now()
		report, err = recoverMemberWithHost(context.Background(), c.fixture.conn, c.settings, expected.memberID, until, host)
	}
	if err != nil {
		t.Fatal(err)
	}
	if _, observed := host.probe.observed.get(); report.outcome != recoveryOutcomeRecovered || !observed {
		t.Fatalf("outcome %v, cleanup observed %v", report.outcome, observed)
	}
}

func TestRuntimeRecovery_Clock_RunnerRecoveryCleanupKeepsItsEntryClockAfterAWait(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name          string
		borrowed      bool
		budget, delay time.Duration
	}{
		{"standalone cleanup uses its original entry clock", false, 7 * time.Second, 150 * time.Millisecond},
		{"borrowed cleanup keeps the entry clock and the original outer cap", true, 3300 * time.Millisecond, 150 * time.Millisecond},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			recoveryClockCase(t, tc.borrowed, tc.budget, tc.delay)
		})
	}
}

func completeRun(t *testing.T, conn *dbConn, runID int64, status runStatus, foreignOwner bool) []tableSnapshot {
	t.Helper()
	exitCode := none[int64]()
	if status == runStatusExited {
		exitCode = some(int64(7))
	}
	foreign := int64(0)
	if foreignOwner {
		foreign = 1
	}
	diagnostics := jsonValue([]jsonObject{{{"kind", "same_owner_completion"}, {"status", status.asStr()}}})
	if changed, err := sqlExecute(conn,
		"UPDATE member_runs SET status=?1,cleanup_state='complete',exit_code=?2,ended_at=?3,cleanup_diagnostics_json=?4,owner_epoch=owner_epoch+?5 WHERE run_id=?6",
		status.asStr(), exitCode, recoveryFixtureNow, string(diagnostics), foreign, runID); err != nil || changed != 1 {
		t.Errorf("completing the run changed %d rows: %v", changed, err)
	}
	return recoveryRows(t, conn)
}

func prepareCompletion(t *testing.T, missingPath bool) *recoveryCase {
	t.Helper()
	c := newRecoveryCase(t)
	c.release(t)
	if missingPath {
		if err := os.Remove(c.artifacts.paths.ownerLock); err != nil {
			t.Fatal(err)
		}
	}
	return c
}

func assertCompleted(t *testing.T, c *recoveryCase, host *recoveryTestHost, report recoveryReport, status runStatus, expected []tableSnapshot) {
	t.Helper()
	if report.outcome != recoveryOutcomeRecovered || report.status != status {
		t.Fatalf("report %v/%v", report.outcome, report.status)
	}
	assertRecoveryRows(t, c.fixture.conn, expected, "same-owner terminal completion remains immutable")
	if host.called("after_takeover") || host.called("cleanup") {
		t.Fatalf("calls %v", host.calls)
	}
	run := mustRun(t, c.fixture.conn, c.fixture.run.runID)
	exitCode := none[int64]()
	if status == runStatusExited {
		exitCode = some(int64(7))
	}
	if !run.owner().equal(c.fixture.run.owner()) || run.exitCode != exitCode {
		t.Fatal("the completed run keeps its owner and exit")
	}
	if string(jsonValue(report.cleanupDiagnostics)) != jsonText(run.cleanupDiagnosticsJSON) {
		t.Fatalf("report diagnostics %s, run %s", jsonValue(report.cleanupDiagnostics), jsonTextOf(run.cleanupDiagnosticsJSON))
	}
	if len(recoveryEvents(t, c)) != 0 {
		t.Fatal("a completed run records no event")
	}
}

func immediateCompletion(t *testing.T, missingPath, foreignOwner bool, status runStatus) {
	t.Helper()
	c := prepareCompletion(t, missingPath)
	var inherited *runLock
	if foreignOwner && !missingPath {
		inherited = acquireReleasableRunLock(t, c.fixture.conn, c.fixture.run.owner())
		t.Cleanup(func() {
			if inherited != nil {
				if err := inherited.close(); err != nil {
					t.Error(err)
				}
			}
		})
	}
	initialSnapshotSeen := false
	var expected []tableSnapshot
	side := openSideConnection(t, c)
	host := newRecoveryHost(t)
	host.action = func(boundary string, run runRecord, _ time.Time) error {
		if boundary == "after_snapshot" && inherited != nil {
			if initialSnapshotSeen {
				if err := inherited.close(); err != nil {
					t.Error(err)
				}
				inherited = nil
			}
			initialSnapshotSeen = true
		}
		if boundary == "before_takeover" {
			if !mustRun(t, side, run.runID).owner().equal(run.owner()) {
				t.Error("the takeover boundary sees the snapshot owner")
			}
			expected = completeRun(t, side, run.runID, status, foreignOwner)
		}
		return nil
	}
	report, err := recoverReleasedWithHost(t, c, host)
	if foreignOwner {
		if err == nil {
			t.Fatal("a terminal shortcut still validates the fixed owner")
		}
		assertRecoveryRows(t, c.fixture.conn, expected, "foreign completion")
		return
	}
	if err != nil {
		t.Fatal(err)
	}
	assertCompleted(t, c, host, report, status, expected)
}

func TestRuntimeRecovery_Completion_RunnerRecoverySameOwnerCompletionBeforeTakeoverHasZeroEffects(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		missingPath bool
		status      runStatus
	}{
		{"before an ordinary takeover", false, runStatusExited},
		{"before a missing-path takeover", true, runStatusFailed},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			immediateCompletion(t, tc.missingPath, false, tc.status)
		})
	}
}

func TestRuntimeRecovery_Completion_RunnerRecoveryCompletedForeignOwnerStillRejectsTheFixedSnapshot(t *testing.T) {
	t.Parallel()
	immediateCompletion(t, false, true, runStatusLost)
	immediateCompletion(t, true, true, runStatusExited)
}

func completionBehindWriteLock(t *testing.T, missingPath bool, status runStatus) {
	t.Helper()
	c := prepareCompletion(t, missingPath)
	var expected []tableSnapshot
	runID := c.fixture.run.runID
	start := make(chan struct{}, 1)
	held := make(chan struct{}, 1)
	begin := make(chan struct{}, 1)
	var armed, observedBegin atomic.Bool
	if err := c.fixture.conn.raw.SetAuthorizer(func(action sqlite3.AuthorizerActionCode, operation, _, _, _ string) sqlite3.AuthorizerReturnCode {
		if armed.Load() && action == sqlite3.AUTH_TRANSACTION && operation == "BEGIN" && observedBegin.CompareAndSwap(false, true) {
			begin <- struct{}{}
		}
		return sqlite3.AUTH_OK
	}); err != nil {
		t.Fatal(err)
	}
	host := newRecoveryHost(t)
	started := false
	host.action = func(boundary string, _ runRecord, _ time.Time) error {
		if boundary == "before_takeover" && !started {
			started = true
			start <- struct{}{}
			select {
			case <-held:
			case <-time.After(2 * time.Second):
				t.Error("the writer holds the write lock in time")
			}
			armed.Store(true)
		}
		return nil
	}
	writerDone := make(chan error, 1)
	go func() {
		writerDone <- func() error {
			select {
			case <-start:
			case <-time.After(2 * time.Second):
				return appError("recovery never reached its takeover")
			}
			writer, err := connect(c.fixture.url)
			if err != nil {
				return err
			}
			defer writer.close() //nolint:errcheck // a side connection of the test
			if err := writer.executeBatch("BEGIN IMMEDIATE"); err != nil {
				return err
			}
			expected = completeRun(t, writer, runID, status, false)
			held <- struct{}{}
			select {
			case <-begin:
			case <-time.After(2 * time.Second):
				return appError("the recovery never attempted its immediate transaction")
			}
			return writer.executeBatch("COMMIT")
		}()
	}()
	report, err := c.recover(t, host)
	if writerErr := <-writerDone; writerErr != nil {
		t.Fatal(writerErr)
	}
	if err := c.fixture.conn.raw.SetAuthorizer(nil); err != nil {
		t.Fatal(err)
	}
	if !observedBegin.Load() {
		t.Fatal("the caller attempted its real immediate transaction while completion held the write lock")
	}
	if err != nil {
		t.Fatal(err)
	}
	assertCompleted(t, c, host, report, status, expected)
}

func TestRuntimeRecovery_Completion_RunnerRecoveryCompletionIsRecheckedAfterActualWriteLockWait(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		missingPath bool
		status      runStatus
	}{
		{"an ordinary completion", false, runStatusExited},
		{"a missing-path completion", true, runStatusLost},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			completionBehindWriteLock(t, tc.missingPath, tc.status)
		})
	}
}

func TestRuntimeRecovery_MissingPaths_RunnerRecoveryMissingPathUnknownIdentityPreservesOpenEpisodeAsLiveOrUnreachable(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	missingLock(t, c)
	before := recoveryRows(t, c.fixture.conn)
	host := newRecoveryHost(t)
	host.process = processEvidenceUnknown
	report, err := c.recover(t, host)
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeLiveOrUnreachable {
		t.Fatalf("outcome %v", report.outcome)
	}
	assertRecoveryRows(t, c.fixture.conn, before, "unknown identity")
	if interaction, _ := recoveryEvents(t, c)[0].interaction.get(); interaction.closedAt.set {
		t.Fatal("the episode stays open")
	}
	if _, err := os.Lstat(c.artifacts.paths.ownerLock); err == nil || host.called("cleanup") {
		t.Fatal("recovery neither recreates the lock nor cleans up")
	}
}

func TestRuntimeRecovery_MissingPaths_RunnerRecoveryLiveUnlinkedOriginalLockIsNotReplacedOrTreatedAsTermination(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	persistRecoveryOpening(t, c)
	originalGuard := c.held
	c.held = nil
	if err := os.Remove(c.artifacts.paths.ownerLock); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(c.artifacts.paths.ownerLock, []byte("foreign replacement inode"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(c.artifacts.paths.ownerLock, 0o600); err != nil {
		t.Fatal(err)
	}
	before := recoveryRows(t, c.fixture.conn)
	host := newRecoveryHost(t)
	host.process = processEvidenceAlive
	report, err := c.recover(t, host)
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeLiveOrUnreachable {
		t.Fatalf("outcome %v", report.outcome)
	}
	assertRecoveryRows(t, c.fixture.conn, before, "live unlinked lock")
	if string(readFile(t, c.artifacts.paths.ownerLock)) != "foreign replacement inode" {
		t.Fatal("the foreign replacement survives")
	}
	if err := originalGuard.validate(context.Background(), c.fixture.conn, c.fixture.run.owner()); err == nil {
		t.Fatal("the unlinked original lock no longer validates")
	}
	if err := originalGuard.close(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeRecovery_MissingPaths_RunnerRecoveryPositivePidLockOrStrictHealthIndependentlyKeepsTheOwnerLive(t *testing.T) {
	t.Parallel()
	for _, witness := range []string{"pid", "lock", "health"} {
		c := newRecoveryCase(t)
		persistRecoveryOpening(t, c)
		if witness != "lock" {
			missingLock(t, c)
		}
		before := recoveryRows(t, c.fixture.conn)
		host := newRecoveryHost(t)
		host.process = processEvidenceUnknown
		if witness == "pid" {
			host.process = processEvidenceAlive
		}
		host.matchingHealth = witness == "health"
		report, err := recoverMemberWithHost(context.Background(), c.fixture.conn, c.settings, c.fixture.run.memberID, host.deadline, host)
		if err == nil && report.outcome != recoveryOutcomeLiveOrUnreachable {
			t.Fatalf("%s: outcome %v", witness, report.outcome)
		}
		assertRecoveryRows(t, c.fixture.conn, before, witness)
		if host.called("after_takeover") || host.called("cleanup") {
			t.Fatalf("%s: calls %v", witness, host.calls)
		}
	}
}

func TestRuntimeRecovery_Relationships_RunnerRecoveryRefusesAMemberFleetChangeAndKeepsValidHistoryAtEveryStage(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		stage   recoveryStage
		refused bool
	}{
		{"ordinary takeover refuses a member/fleet change", recoveryStageOrdinaryTakeover, true},
		{"ordinary takeover keeps valid history and its inbox", recoveryStageOrdinaryTakeover, false},
		{"zero-event missing-path takeover refuses a member/fleet change", recoveryStageMissingPathTakeover, true},
		{"zero-event missing-path bookkeeping keeps valid history", recoveryStageMissingPathTakeover, false},
		{"finalization refuses a member/fleet change", recoveryStageFinalization, true},
		{"a valid historical lifecycle change allows finalization", recoveryStageFinalization, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if tc.refused {
				recoveryRelationshipRefusal(t, tc.stage)
			} else {
				recoveryHistoricalControl(t, tc.stage)
			}
		})
	}
}

func TestRuntimeRecovery_DeferredInputs_RunnerRecoverySettlesADeferredReadyReceiptNotSubmitted(t *testing.T) {
	t.Parallel()
	c := newRecoveryCase(t)
	insert := func(key, state string, deferred bool) int64 {
		row := recoveryInputRow{key: key, state: state, deferred: deferred}
		if state != "ready" {
			row.deliveryMethod = some("profile")
		}
		return row.insert(t, c)
	}
	deferred := insert("prompt:deferred", "ready", true)
	attempted := insert("prompt:attempted", "in_progress", false)
	attemptedBefore := recoveryInput(t, c.fixture.conn, attempted)
	report, err := c.recover(t, newRecoveryHost(t))
	if err != nil {
		t.Fatal(err)
	}
	if report.outcome != recoveryOutcomeRecovered {
		t.Fatalf("outcome %v", report.outcome)
	}
	settled := recoveryInput(t, c.fixture.conn, deferred)
	if settled.state != inputStateSettled || settled.disposition != some(inputDispositionNotSubmitted) ||
		settled.deliveryMethod.set || settled.deliveryReason != some("runner_lost") || settled.deferReason != some("interaction_open") {
		t.Fatal("the deferred receipt settles not submitted as runner_lost")
	}
	if recoveryInput(t, c.fixture.conn, attempted) != attemptedBefore {
		t.Fatal("an attempt that may have written bytes keeps its reconciliation state")
	}
	if got := len(recoveryDeliveryResults(t, c, deferred)); got != 1 {
		t.Fatalf("the recovery settlement publishes its deferred result once, got %d", got)
	}
	if _, err := c.recover(t, newRecoveryHost(t)); err != nil {
		t.Fatal(err)
	}
	if recoveryInput(t, c.fixture.conn, deferred) != settled || len(recoveryDeliveryResults(t, c, deferred)) != 1 {
		t.Fatal("a repeated recovery changes nothing and publishes nothing new")
	}
}

func TestRuntimeRecovery_NativeProcessEvidence_OwnerProcessEvidenceFollowsTheRecordedIdentity(t *testing.T) {
	t.Parallel()
	evidence := func(process processIdentity) processEvidence {
		observed, err := nativeRecoveryHost{}.processEvidence(process, time.Now().Add(5*time.Second))
		if err != nil {
			t.Fatal(err)
		}
		return observed
	}
	if observed := evidence(currentProcess(t)); observed != processEvidenceAlive {
		t.Fatalf("a running owner with its recorded identity is alive: %v", observed)
	}
	reap := func(child *exec.Cmd) {
		if err := child.Wait(); err == nil {
			t.Fatal("the killed child reports its signal")
		}
	}
	for _, reaped := range []bool{false, true} {
		child, identity := zombie(t)
		if reaped {
			reap(child)
		}
		observed := evidence(identity)
		if !reaped {
			reap(child)
		}
		if observed != processEvidenceTerminated {
			t.Fatalf("reaped=%v: %v", reaped, observed)
		}
	}
}
