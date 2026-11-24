package core

import (
	"context"
	"encoding/json/jsontext"
	"runtime"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
)

const writerPauseSlice = 10 * time.Millisecond

func writerBoundarySlot(boundary writerBoundary) int {
	switch boundary {
	case writerBoundaryReceive:
		return 0
	case writerBoundaryPersist:
		return 1
	case writerBoundaryFlush:
		return 2
	default:
		panic("invalid writerBoundary")
	}
}

// writerTestHooks pauses, fails or counts the worker at each boundary.
type writerTestHooks struct {
	mu       sync.Mutex
	paused   [3]bool
	failures [3]uint64
	entered  [3]uint64
	busyCall [3]opt[uint64]
	busy     [3]bool
}

func (h *writerTestHooks) pause(boundary writerBoundary) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.paused[writerBoundarySlot(boundary)] = true
}

func (h *writerTestHooks) releaseForCleanup() {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.paused = [3]bool{}
	h.busy = [3]bool{}
}

func (h *writerTestHooks) failOnce(boundary writerBoundary) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.failures[writerBoundarySlot(boundary)]++
}

// busyOnCall fails only the call-th entry to boundary, counting from 1, as a transient busy cause.
func (h *writerTestHooks) busyOnCall(boundary writerBoundary, call uint64) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.busyCall[writerBoundarySlot(boundary)] = some(call)
}

func (h *writerTestHooks) calls(boundary writerBoundary) uint64 {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.entered[writerBoundarySlot(boundary)]
}

func (h *writerTestHooks) wait(t *testing.T, boundary writerBoundary, count uint64) {
	t.Helper()
	until := time.Now().Add(2 * time.Second)
	for h.calls(boundary) < count {
		if !time.Now().Before(until) {
			t.Fatalf("worker boundary %d was not reached %d times; calls=%d", boundary, count, h.calls(boundary))
		}
		time.Sleep(time.Millisecond)
	}
}

func (h *writerTestHooks) before(boundary writerBoundary, bound *stepBound) error {
	slot := writerBoundarySlot(boundary)
	h.mu.Lock()
	defer h.mu.Unlock()
	h.entered[slot]++
	for h.paused[slot] {
		if bound.check() != nil {
			return deadlineExceededError()
		}
		h.mu.Unlock()
		time.Sleep(writerPauseSlice)
		h.mu.Lock()
	}
	if h.failures[slot] > 0 {
		h.failures[slot]--
		return databaseBusyError("fixture writer boundary busy")
	}
	if h.busy[slot] || h.busyCall[slot] == some(h.entered[slot]) {
		return databaseBusyError("fixture writer boundary busy")
	}
	return nil
}

// yieldingClock is a testClock as a background worker sees it: the same virtual time, but its
// sleeps yield instead of advancing, so only the test moves time.
type yieldingClock struct {
	*testClock
}

func (yieldingClock) sleep(time.Duration) {
	runtime.Gosched()
}

type writerFixture struct {
	*runnerFixture
	artifacts *runArtifacts
	lock      *runLock
}

func newWriterFixture(t *testing.T) *writerFixture {
	t.Helper()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	lock := acquireRunLock(t, fixture.conn, fixture.run.owner())
	if _, err := lock.transferToRunner(context.Background(), fixture.conn, fixture.run.owner(), currentProcess(t)); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	return &writerFixture{runnerFixture: fixture, artifacts: artifacts, lock: lock}
}

func (f *writerFixture) start() (*eventWriter, error) {
	return eventWriterStart(f.url, f.run, f.lock, systemClock{}, newRunnerDiagnostics())
}

// startWorker starts a writer with hooks; cleanup releases the hooks before closing the writer.
func startWorker(t *testing.T, fixture *writerFixture, hooks *writerTestHooks, c clock, diagnostics runnerDiagnostics) *eventWriter {
	t.Helper()
	writer, err := eventWriterStartWorker(fixture.url, fixture.run, fixture.lock, c, diagnostics, hooks)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		hooks.releaseForCleanup()
		writer.close()
	})
	return writer
}

// startOn starts a writer whose worker reads clock's virtual time.
func startOn(t *testing.T, fixture *writerFixture, hooks *writerTestHooks, clock *testClock) *eventWriter {
	t.Helper()
	return startWorker(t, fixture, hooks, yieldingClock{clock}, newRunnerDiagnostics())
}

func startOnWith(t *testing.T, fixture *writerFixture, hooks *writerTestHooks, clock *testClock, diagnostics runnerDiagnostics) *eventWriter {
	t.Helper()
	return startWorker(t, fixture, hooks, yieldingClock{clock}, diagnostics)
}

func startReal(t *testing.T, fixture *writerFixture, hooks *writerTestHooks) *eventWriter {
	t.Helper()
	return startWorker(t, fixture, hooks, systemClock{}, newRunnerDiagnostics())
}

func startRealWith(t *testing.T, fixture *writerFixture, hooks *writerTestHooks, diagnostics runnerDiagnostics) *eventWriter {
	t.Helper()
	return startWorker(t, fixture, hooks, systemClock{}, diagnostics)
}

// yieldUntil yields until the worker makes condition hold, within ten seconds of real time.
func yieldUntil(t *testing.T, condition func() bool) {
	t.Helper()
	until := time.Now().Add(10 * time.Second)
	for !condition() {
		if !time.Now().Before(until) {
			t.Fatal("bounded worker condition did not complete")
		}
		runtime.Gosched()
	}
}

func waitUntilWithin(t *testing.T, budget time.Duration, condition func() bool) {
	t.Helper()
	until := time.Now().Add(budget)
	for !condition() {
		if !time.Now().Before(until) {
			t.Fatal("bounded worker condition did not complete")
		}
		time.Sleep(2 * time.Millisecond)
	}
}

func writerUntil(clock *testClock) time.Time {
	return clock.now().Add(2 * time.Second)
}

func realDeadline() time.Time {
	return time.Now().Add(2 * time.Second)
}

func committed(t *testing.T, fixture *writerFixture) int64 {
	t.Helper()
	return queryInt64(t, fixture.conn, "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1", fixture.run.runID)
}

// runTransition is the coverage-unknown transition sequence of run.
func runTransition(t *testing.T, run runRecord, sequence int64) observedTransition {
	t.Helper()
	evidence, err := runEvidence(run, "runner-lifecycle", uint64(max(sequence, 0)))
	if err != nil {
		t.Fatal(err)
	}
	return observedTransition{
		RunID:          run.runID,
		EventSeq:       sequence,
		EpisodeSeq:     none[int64](),
		Kind:           runtimeEventKindCoverageUnknown,
		Classification: none[interactionClass](),
		ObservedAt:     "2026-10-26T16:40:00.000000+00:00",
		EvidenceJSON:   evidence,
		CloseReason:    none[interactionCloseReason](),
		CauseJSON:      jsontext.Value(`{"kind":"coverage_unknown"}`),
	}
}

func writerTransition(t *testing.T, fixture *writerFixture, sequence int64) observedTransition {
	t.Helper()
	return runTransition(t, fixture.run, sequence)
}

func mustAccept(t *testing.T, writer *eventWriter, transition observedTransition) {
	t.Helper()
	if retained := writer.trySubmit(transition); retained != nil {
		t.Fatalf("transition %d is refused: %v (%d)", transition.EventSeq, retained.cause, retained.reason)
	}
}

func mustRetain(t *testing.T, writer *eventWriter, transition observedTransition, reason retainReason) *retainedTransition {
	t.Helper()
	retained := writer.trySubmit(transition)
	if retained == nil || retained.reason != reason {
		t.Fatalf("transition %d: retained %+v, want reason %d", transition.EventSeq, retained, reason)
	}
	return retained
}

func assertFinish(t *testing.T, writer *eventWriter, until time.Time, pending, unsaved uint64) {
	t.Helper()
	if gotPending, gotUnsaved := writer.finish(until); gotPending != pending || gotUnsaved != unsaved {
		t.Fatalf("finish = (%d, %d), want (%d, %d)", gotPending, gotUnsaved, pending, unsaved)
	}
}

func assertFinishPending(t *testing.T, writer *eventWriter, until time.Time, pending uint64) {
	t.Helper()
	if got, _ := writer.finish(until); got != pending {
		t.Fatalf("finish pending = %d, want %d", got, pending)
	}
}

func fencedDiagnostic(writer *eventWriter) bool {
	diagnostic, set := writer.status().diagnostic.get()
	return set && strings.HasPrefix(diagnostic, "event writer fenced: ")
}

func diagnosticContains(status writerStatus, text string) bool {
	diagnostic, set := status.diagnostic.get()
	return set && strings.Contains(diagnostic, text)
}

func drained(writer *eventWriter) bool {
	return writer.status().pendingCount == 0
}

func TestRuntimeWriter_Worker_WriterWorkerReceiveStallPins64ChannelSlotsAndReturnsTheFailedOriginal(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	hooks.pause(writerBoundaryReceive)
	writer := startOn(t, fixture, hooks, clock)
	hooks.wait(t, writerBoundaryReceive, 1)
	for sequence := int64(1); sequence <= 64; sequence++ {
		mustAccept(t, writer, writerTransition(t, fixture, sequence))
	}
	original := writerTransition(t, fixture, 65)
	rejected := mustRetain(t, writer, original, retainReasonChannelFull)
	if !rejected.transition.equal(original) {
		t.Fatal("channel pressure returns the original")
	}
	status := writer.status()
	if status.pendingCount != 64 || status.diagnostic == none[string]() || status.gapCount == 0 {
		t.Fatalf("status = %+v", status)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("committed = %d", got)
	}
	hooks.releaseForCleanup()
	yieldUntil(t, func() bool { return drained(writer) })
	if got := committed(t, fixture); got != 64 {
		t.Fatalf("committed = %d", got)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func TestRuntimeWriter_Worker_WriterWorkerDatabaseFailureRetainsTheSameTransitionForRetry(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	hooks.failOnce(writerBoundaryPersist)
	writer := startOn(t, fixture, hooks, clock)
	original := writerTransition(t, fixture, 1)
	mustAccept(t, writer, original)
	yieldUntil(t, func() bool { return writer.status().diagnostic != none[string]() })
	if got := committed(t, fixture); got != 0 || writer.status().pendingCount != 1 {
		t.Fatalf("the retry waits for its backoff: committed %d, %+v", got, writer.status())
	}
	clock.advance(100 * time.Millisecond)
	yieldUntil(t, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	persisted := readEvent(t, fixture.conn, 1).event
	if persisted.kind != original.Kind {
		t.Fatalf("a transient failure retries the original: %v", persisted.kind)
	}
	assertJSONEqual(t, jsonAt(t, persisted.payloadJSON, "evidence"), original.EvidenceJSON, "evidence")
	if status := writer.status(); status.diagnostic != none[string]() {
		t.Fatalf("status = %+v", status)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func TestRuntimeWriter_Worker_WriterCommitsEachSubmittedTransitionDirectlyWithItsEventSequence(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	writer := startOn(t, fixture, &writerTestHooks{}, clock)
	for sequence := int64(1); sequence <= 3; sequence++ {
		mustAccept(t, writer, writerTransition(t, fixture, sequence))
	}
	yieldUntil(t, func() bool { return committed(t, fixture) == 3 && drained(writer) })
	var sequences []int64
	for _, row := range tableRows(t, fixture.conn, "SELECT event_seq FROM member_runtime_events WHERE run_id=?1 ORDER BY event_id", fixture.run.runID) {
		sequences = append(sequences, row[0].(int64))
	}
	if !slices.Equal(sequences, []int64{1, 2, 3}) {
		t.Fatalf("sequences = %v", sequences)
	}
	if status := writer.status(); status.diagnostic != none[string]() {
		t.Fatalf("status = %+v", status)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func TestRuntimeWriter_Worker_WriterFencesWhenTheRejectionEventAlsoFailsPermanently(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	faultPlanAbort("injected_event_failure", "BEFORE INSERT ON member_runtime_events", "injected event failure").arm(t, fixture.conn)
	hooks := &writerTestHooks{}
	hooks.pause(writerBoundaryReceive)
	writer := startOn(t, fixture, hooks, clock)
	hooks.wait(t, writerBoundaryReceive, 1)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	mustAccept(t, writer, writerTransition(t, fixture, 2))
	hooks.releaseForCleanup()
	yieldUntil(t, func() bool { return fencedDiagnostic(writer) })
	fenced := writer.status()
	if fenced.pendingCount != 0 || !diagnosticContains(fenced, "injected event failure") {
		t.Fatalf("the queue is discarded: %+v", fenced)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("committed = %d", got)
	}
	refused := mustRetain(t, writer, writerTransition(t, fixture, 3), retainReasonFenced)
	if refused.cause.message() != "event writer is fenced" {
		t.Fatalf("cause = %q", refused.cause.message())
	}
	if after := writer.status(); after.diagnostic != fenced.diagnostic || after.gapCount != fenced.gapCount {
		t.Fatalf("a refusal after the fence keeps the fence diagnostic: %+v, want %+v", after, fenced)
	}
	assertFinish(t, writer, writerUntil(clock), 0, 2)
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("committed = %d", got)
	}
}

func TestRuntimeWriter_Worker_WriterWorkerPendingCapacityCountsChannelAndWorkerQueueWithoutGrowth(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	hooks.pause(writerBoundaryPersist)
	writer := startOn(t, fixture, hooks, clock)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	hooks.wait(t, writerBoundaryPersist, 1)
	accepted := uint64(1)
	for sequence := int64(2); sequence <= 256; sequence++ {
		if retained := writer.trySubmit(writerTransition(t, fixture, sequence)); retained != nil {
			if retained.transition.EventSeq != sequence {
				t.Fatalf("retained %d, want %d", retained.transition.EventSeq, sequence)
			}
			break
		}
		accepted++
	}
	if accepted < 64 || accepted > 128 {
		t.Fatalf("accepted = %d", accepted)
	}
	if status := writer.status(); status.pendingCount != accepted || status.diagnostic == none[string]() {
		t.Fatalf("status = %+v, accepted %d", status, accepted)
	}
	hooks.releaseForCleanup()
	yieldUntil(t, func() bool { return drained(writer) })
	if got := committed(t, fixture); got != int64(accepted) {
		t.Fatalf("committed = %d, want %d", got, accepted)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func lockHolder(t *testing.T, fixture *writerFixture) *dbConn {
	t.Helper()
	holder := connectedTo(t, fixture.databaseFile(t))
	mustExecBatch(t, holder, "BEGIN IMMEDIATE")
	return holder
}

func TestRuntimeWriter_StepBound_WriterVirtualClockJumpWhileTheWorkerIdlesNeverExpiresAStep(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	hooks.pause(writerBoundaryReceive)
	writer := startOn(t, fixture, hooks, clock)
	hooks.wait(t, writerBoundaryReceive, 1)
	clock.advance(10 * time.Second)
	hooks.releaseForCleanup()
	hooks.wait(t, writerBoundaryReceive, hooks.calls(writerBoundaryReceive)+2)
	if idle := writer.status(); idle.diagnostic != none[string]() || idle.gapCount != 0 {
		t.Fatalf("idle = %+v", idle)
	}
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	yieldUntil(t, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	if status := writer.status(); status.diagnostic != none[string]() {
		t.Fatalf("status = %+v", status)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func TestRuntimeWriter_StepBound_WriterPersistWaitsBeyond100MsUnderAHeldLockWithoutFailing(t *testing.T) {
	t.Parallel()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	writer := startReal(t, fixture, hooks)
	holder := lockHolder(t, fixture)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	hooks.wait(t, writerBoundaryPersist, 1)
	time.Sleep(300 * time.Millisecond)
	if held := writer.status(); held.diagnostic != none[string]() || held.gapCount != 0 || held.pendingCount != 1 {
		t.Fatalf("held = %+v", held)
	}
	mustExecBatch(t, holder, "ROLLBACK")
	waitUntilWithin(t, 5*time.Second, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	if persisted := writer.status(); persisted.diagnostic != none[string]() || persisted.gapCount != 0 {
		t.Fatalf("persisted = %+v", persisted)
	}
	assertFinish(t, writer, realDeadline(), 0, 0)
}

func TestRuntimeWriter_StepBound_WriterFinishJoinsAnInFlightStepAtTheShutdownDeadline(t *testing.T) {
	t.Parallel()
	const shutdownAfter = 200 * time.Millisecond
	for _, name := range []string{"paused persist hook", "held database lock"} {
		t.Run(name, func(t *testing.T) {
			t.Parallel()
			fixture := newWriterFixture(t)
			hooks := &writerTestHooks{}
			var holder *dbConn
			if name == "paused persist hook" {
				hooks.pause(writerBoundaryPersist)
			} else {
				holder = lockHolder(t, fixture)
			}
			writer := startReal(t, fixture, hooks)
			mustAccept(t, writer, writerTransition(t, fixture, 1))
			hooks.wait(t, writerBoundaryPersist, 1)
			time.Sleep(50 * time.Millisecond)
			started := time.Now()
			pending, _ := writer.finish(started.Add(shutdownAfter))
			elapsed := time.Since(started)
			if !writer.joined {
				t.Fatalf("%s: finish joins the worker once its step aborts", name)
			}
			if elapsed >= time.Second {
				t.Fatalf("%s: the in-flight step ends at the shutdown deadline, not the 5 s step bound: %v", name, elapsed)
			}
			if pending != 1 {
				t.Fatalf("%s: the unsaved transition stays counted: %d", name, pending)
			}
			hooks.releaseForCleanup()
			if holder != nil {
				mustExecBatch(t, holder, "ROLLBACK")
			}
			time.Sleep(50 * time.Millisecond)
			if got := committed(t, fixture); got != 0 {
				t.Fatalf("%s: nothing commits after finish returns: %d", name, got)
			}
		})
	}
}

func TestRuntimeWriter_StepBound_WriterKeepsAStatementCutOffByTheShutdownDeadlineAtTheFrontUnrejected(t *testing.T) {
	t.Parallel()
	fixture := newWriterFixture(t)
	mustExecBatch(t, fixture.conn, `CREATE TABLE slow_rows(value INTEGER);
		INSERT INTO slow_rows WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<2000) SELECT x FROM counter;
		CREATE TRIGGER slow_event_insert BEFORE INSERT ON member_runtime_events BEGIN SELECT count(*) FROM slow_rows a, slow_rows b, slow_rows c; END`)
	diagnostics := newRunnerDiagnostics()
	hooks := &writerTestHooks{}
	writer := startRealWith(t, fixture, hooks, diagnostics)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	hooks.wait(t, writerBoundaryPersist, 1)
	time.Sleep(100 * time.Millisecond)
	started := time.Now()
	pending, _ := writer.finish(started.Add(200 * time.Millisecond))
	if elapsed := time.Since(started); elapsed >= time.Second {
		t.Fatalf("the shutdown deadline interrupts the statement: %v", elapsed)
	}
	if pending != 1 {
		t.Fatalf("the original stays accepted and unsaved: %d", pending)
	}
	if calls := hooks.calls(writerBoundaryPersist); calls != 1 {
		t.Fatalf("no replacement persist follows the interrupted original: %d calls", calls)
	}
	entries, _ := diagnostics.snapshot()
	if slices.ContainsFunc(entries, func(entry runnerDiagnosticEntry) bool {
		return strings.HasPrefix(entry.message, "event writer rejected")
	}) {
		t.Fatalf("an interrupted statement records no rejection: %+v", entries)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("no rejection event is persisted: %d", got)
	}
}

func rejectCoverageUnknown() faultPlan {
	return faultPlanAbort("reject_coverage_unknown", "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='coverage_unknown'", "injected kind rejection")
}

func eventAt(t *testing.T, fixture *writerFixture, sequence int64) runtimeEventRecord {
	t.Helper()
	id := queryInt64(t, fixture.conn, "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2", fixture.run.runID, sequence)
	return readEvent(t, fixture.conn, id).event
}

func assertRejection(t *testing.T, event runtimeEventRecord, rejected observedTransition) {
	t.Helper()
	if event.kind != runtimeEventKindRunnerDegraded || event.eventSeq != rejected.EventSeq {
		t.Fatalf("event = %+v", event)
	}
	cause := jsonAt(t, event.payloadJSON, "cause")
	if keys := testJSONKeys(t, cause); !slices.Equal(keys, []string{"kind", "rejected_kind", "rejected_observed_at", "error"}) {
		t.Fatalf("cause keys = %q", keys)
	}
	assertJSONEqual(t, jsonAt(t, cause, "kind"), jsontext.Value(`"event_rejected"`), "kind")
	assertJSONEqual(t, jsonAt(t, cause, "rejected_kind"), mustJSON(t, rejected.Kind.asStr()), "rejected_kind")
	assertJSONEqual(t, jsonAt(t, cause, "rejected_observed_at"), mustJSON(t, rejected.ObservedAt), "rejected_observed_at")
	if !strings.Contains(string(jsonAt(t, cause, "error")), "injected kind rejection") {
		t.Fatalf("cause = %s", cause)
	}
	assertJSONEqual(t, jsonAt(t, event.payloadJSON, "director_action"), jsontext.Value("null"), "director_action")
	assertJSONEqual(t, jsonAt(t, event.payloadJSON, "evidence", "screen_seq"), jsonAt(t, rejected.EvidenceJSON, "screen_seq"), "screen_seq")
}

func assertWriterHealthy(t *testing.T, writer *eventWriter, context string) {
	t.Helper()
	if status := writer.status(); status.diagnostic != none[string]() || status.gapCount != 0 {
		t.Fatalf("%s: %+v", context, status)
	}
}

func TestRuntimeWriter_Rejection_WriterReplacesAPermanentFailureWithItsRejectionAndContinuesWithoutAGap(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	rejectCoverageUnknown().arm(t, fixture.conn)
	writer := startOn(t, fixture, &writerTestHooks{}, clock)
	rejected := writerTransition(t, fixture, 1)
	mustAccept(t, writer, rejected)
	yieldUntil(t, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	assertWriterHealthy(t, writer, "replaced")
	assertRejection(t, eventAt(t, fixture, 1), rejected)
	rejectCoverageUnknown().clear(t, fixture.conn)
	next := writerTransition(t, fixture, 2)
	mustAccept(t, writer, next)
	yieldUntil(t, func() bool { return committed(t, fixture) == 2 && drained(writer) })
	if kind := eventAt(t, fixture, 2).kind; kind != next.Kind {
		t.Fatalf("kind = %v", kind)
	}
	assertWriterHealthy(t, writer, "continued")
	assertFinish(t, writer, writerUntil(clock), 0, 0)
}

func TestRuntimeWriter_Rejection_WriterKeepsATransientlyFailingReplacementAtTheFrontUntilTheBackoffRetry(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	rejectCoverageUnknown().arm(t, fixture.conn)
	hooks := &writerTestHooks{}
	hooks.busyOnCall(writerBoundaryPersist, 2)
	writer := startOn(t, fixture, hooks, clock)
	rejected := writerTransition(t, fixture, 1)
	mustAccept(t, writer, rejected)
	yieldUntil(t, func() bool { return writer.status().diagnostic != none[string]() })
	if held := writer.status(); held.pendingCount != 1 || !diagnosticContains(held, "fixture writer boundary busy") {
		t.Fatalf("the transient cause takes the ordinary path: %+v", held)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("the retry waits for its backoff: %d", got)
	}
	clock.advance(100 * time.Millisecond)
	yieldUntil(t, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	assertRejection(t, eventAt(t, fixture, 1), rejected)
	if status := writer.status(); status.diagnostic != none[string]() {
		t.Fatalf("status = %+v", status)
	}
	assertFinishPending(t, writer, writerUntil(clock), 0)
}

func TestRuntimeWriter_Rejection_WriterFencesOnASequenceSlotFailure(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	writer := startOn(t, fixture, &writerTestHooks{}, clock)
	mustAccept(t, writer, writerTransition(t, fixture, 2))
	yieldUntil(t, func() bool { return fencedDiagnostic(writer) })
	if fenced := writer.status(); !diagnosticContains(fenced, "event sequence") || fenced.pendingCount != 0 {
		t.Fatalf("fenced = %+v", fenced)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("committed = %d", got)
	}
	assertFinish(t, writer, writerUntil(clock), 0, 1)
}

func TestRuntimeWriter_Rejection_WriterFencesAfterOneReplacementAttemptOnceRunOwnershipIsLost(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	hooks := &writerTestHooks{}
	writer := startOn(t, fixture, hooks, clock)
	bumpEpoch(t, fixture.conn, fixture.run.runID)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	yieldUntil(t, func() bool { return fencedDiagnostic(writer) })
	if calls := hooks.calls(writerBoundaryPersist); calls != 2 {
		t.Fatalf("the original and one replacement are attempted before the fence: %d", calls)
	}
	if fenced := writer.status(); fenced.pendingCount != 0 {
		t.Fatalf("fenced = %+v", fenced)
	}
	if got := committed(t, fixture); got != 0 {
		t.Fatalf("committed = %d", got)
	}
	if writer.trySubmit(writerTransition(t, fixture, 2)) == nil {
		t.Fatal("a fenced writer accepts no transition")
	}
	assertFinish(t, writer, writerUntil(clock), 0, 1)
}

func TestRuntimeWriter_Rejection_WriterRefusesInvalidEvidenceWithoutDegrading(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	writer := startOn(t, fixture, &writerTestHooks{}, clock)
	forged := writerTransition(t, fixture, 1)
	forged.EvidenceJSON = jsontext.Value(`{"forged":true}`)
	refused := mustRetain(t, writer, forged, retainReasonEvidenceInvalid)
	if !refused.transition.equal(forged) {
		t.Fatal("the refusal returns the forged original")
	}
	if status := writer.status(); status.diagnostic != none[string]() || status.gapCount != 0 || status.pendingCount != 0 {
		t.Fatalf("status = %+v", status)
	}
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	yieldUntil(t, func() bool { return committed(t, fixture) == 1 && drained(writer) })
	assertFinish(t, writer, writerUntil(clock), 0, 0)
}

func TestRuntimeWriter_Rejection_WriterRefusesSubmissionsAfterStopAndSequenceRegressionAsPressure(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	writer := startOn(t, fixture, &writerTestHooks{}, clock)
	mustAccept(t, writer, writerTransition(t, fixture, 1))
	mustRetain(t, writer, writerTransition(t, fixture, 1), retainReasonSequenceRegressed)
	writer.beginShutdown(writerUntil(clock))
	mustRetain(t, writer, writerTransition(t, fixture, 3), retainReasonStopped)
	writer.finish(writerUntil(clock))
}

func rejectDiagnostics() faultPlan {
	return faultPlanAbort("reject_runner_diagnostics", "BEFORE INSERT ON member_runner_diagnostics", "injected diagnostics rejection")
}

const writerDiagnosticStamp = "2026-11-21T12:00:00.000000+00:00"

type storedDiagnostic struct {
	source, message string
	count           int64
}

func storedDiagnostics(t *testing.T, fixture *writerFixture) []storedDiagnostic {
	t.Helper()
	var stored []storedDiagnostic
	for _, row := range tableRows(t, fixture.conn, "SELECT source,message,count FROM member_runner_diagnostics WHERE run_id=?1 ORDER BY diagnostic_id", fixture.run.runID) {
		stored = append(stored, storedDiagnostic{source: row[0].(string), message: row[1].(string), count: row[2].(int64)})
	}
	return stored
}

func assertStored(t *testing.T, fixture *writerFixture, want []storedDiagnostic) {
	t.Helper()
	if got := storedDiagnostics(t, fixture); !slices.Equal(got, want) {
		t.Fatalf("stored = %+v, want %+v", got, want)
	}
}

func bufferEmpty(diagnostics runnerDiagnostics) bool {
	entries, _ := diagnostics.snapshot()
	return len(entries) == 0
}

func TestRuntimeWriter_DiagnosticsFlush_WriterFlushesBufferedDiagnosticsAndMergesARepeatIntoItsRow(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	diagnostics := newRunnerDiagnostics()
	writer := startOnWith(t, fixture, &writerTestHooks{}, clock, diagnostics)
	const message = "observation gap opened: sink_unavailable"
	diagnostics.record(runnerDiagnosticSourceObservation, message, writerDiagnosticStamp)
	yieldUntil(t, func() bool { return len(storedDiagnostics(t, fixture)) == 1 && bufferEmpty(diagnostics) })
	diagnostics.record(runnerDiagnosticSourceObservation, message, writerDiagnosticStamp)
	yieldUntil(t, func() bool {
		stored := storedDiagnostics(t, fixture)
		return len(stored) > 0 && stored[0].count == 2
	})
	assertStored(t, fixture, []storedDiagnostic{{"observation", message, 2}})
	assertWriterHealthy(t, writer, "a flush never sets the writer diagnostic")
	assertFinish(t, writer, writerUntil(clock), 0, 0)
}

func TestRuntimeWriter_DiagnosticsFlush_WriterKeepsTheBufferThroughATransientFlushFailure(t *testing.T) {
	t.Parallel()
	fixture := newWriterFixture(t)
	diagnostics := newRunnerDiagnostics()
	writer := startRealWith(t, fixture, &writerTestHooks{}, diagnostics)
	holder := lockHolder(t, fixture)
	diagnostics.record(runnerDiagnosticSourceDeferredInput, "label: held", writerDiagnosticStamp)
	time.Sleep(800 * time.Millisecond)
	if held, dropped := diagnostics.snapshot(); len(held) != 1 || dropped != 0 {
		t.Fatalf("a busy flush keeps its entries: %+v, %d dropped", held, dropped)
	}
	assertStored(t, fixture, nil)
	assertWriterHealthy(t, writer, "held")
	mustExecBatch(t, holder, "ROLLBACK")
	waitUntilWithin(t, 5*time.Second, func() bool { return len(storedDiagnostics(t, fixture)) == 1 && bufferEmpty(diagnostics) })
	assertStored(t, fixture, []storedDiagnostic{{"deferred_input", "label: held", 1}})
	assertWriterHealthy(t, writer, "flushed")
	assertFinish(t, writer, realDeadline(), 0, 0)
}

func TestRuntimeWriter_DiagnosticsFlush_WriterDropsAPermanentlyFailingFlushAndRetriesOneSecondLater(t *testing.T) {
	t.Parallel()
	clock := virtualClock()
	fixture := newWriterFixture(t)
	rejectDiagnostics().arm(t, fixture.conn)
	diagnostics := newRunnerDiagnostics()
	writer := startOnWith(t, fixture, &writerTestHooks{}, clock, diagnostics)
	diagnostics.record(runnerDiagnosticSourcePreviewSweep, "label: lost", writerDiagnosticStamp)
	snapshotIs := func(dropped uint64) bool {
		entries, got := diagnostics.snapshot()
		return len(entries) == 0 && got == dropped
	}
	yieldUntil(t, func() bool { return snapshotIs(1) })
	rejectDiagnostics().clear(t, fixture.conn)
	clock.advance(999 * time.Millisecond)
	time.Sleep(200 * time.Millisecond)
	assertStored(t, fixture, nil)
	if !snapshotIs(1) {
		t.Fatal("the next flush waits one second after the failure")
	}
	clock.advance(time.Millisecond)
	yieldUntil(t, func() bool { return len(storedDiagnostics(t, fixture)) > 0 })
	assertStored(t, fixture, []storedDiagnostic{{"event_writer", "diagnostic buffer overflow: 1 entries dropped", 1}})
	if !snapshotIs(0) {
		t.Fatal("the flushed overflow leaves the buffer empty")
	}
	assertWriterHealthy(t, writer, "after the retry")
	assertFinish(t, writer, writerUntil(clock), 0, 0)
}

func TestRuntimeWriter_DiagnosticsFlush_WriterFailuresAreRecordedAndFlushedUnderEventWriter(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name, failure, names string
	}{
		{"a new transient writer diagnostic", "transient", "fixture writer boundary busy"},
		{"a rejection", "rejection", "injected kind rejection"},
		{"a fence", "fence", "event writer fenced"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			clock := virtualClock()
			fixture := newWriterFixture(t)
			hooks := &writerTestHooks{}
			switch tc.failure {
			case "transient":
				hooks.failOnce(writerBoundaryPersist)
			case "rejection":
				rejectCoverageUnknown().arm(t, fixture.conn)
			case "fence":
				faultPlanAbort("reject_every_event", "BEFORE INSERT ON member_runtime_events", "injected event failure").arm(t, fixture.conn)
			}
			writer := startOnWith(t, fixture, hooks, clock, newRunnerDiagnostics())
			mustAccept(t, writer, writerTransition(t, fixture, 1))
			if tc.failure == "transient" {
				yieldUntil(t, func() bool { return writer.status().diagnostic != none[string]() })
				clock.advance(100 * time.Millisecond)
			}
			yieldUntil(t, func() bool {
				return slices.ContainsFunc(storedDiagnostics(t, fixture), func(row storedDiagnostic) bool {
					return row.source == "event_writer" && strings.Contains(row.message, tc.names)
				})
			})
			for _, row := range storedDiagnostics(t, fixture) {
				if row.source != "event_writer" || strings.TrimSpace(row.message) == "" || row.count <= 0 {
					t.Fatalf("%s: %+v", tc.name, storedDiagnostics(t, fixture))
				}
			}
			writer.finish(writerUntil(clock))
		})
	}
}

func TestRuntimeWriter_WriterStaleOwnerIsRejectedBeforeWorkerStart(t *testing.T) {
	t.Parallel()
	fixture := newWriterFixture(t)
	fixture.run.ownerEpoch++
	writer, err := fixture.start()
	if err == nil {
		writer.finish(time.Now().Add(100 * time.Millisecond))
		t.Fatal("writer startup validates the run owner before accepting work")
	}
}
