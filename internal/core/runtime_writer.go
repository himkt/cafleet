package core

import (
	"context"
	"fmt"
	"sync"
	"time"
)

type writerBoundary uint8

const (
	writerBoundaryReceive writerBoundary = iota + 1
	writerBoundaryPersist
	writerBoundaryFlush
)

const (
	// writerStepBound equals the connection's busy timeout, so one worker step waits at most as
	// long as one contended statement may.
	writerStepBound       = 5 * time.Second
	writerJoinGrace       = 50 * time.Millisecond
	diagnosticFlushBound  = 500 * time.Millisecond
	diagnosticFlushRetry  = time.Second
	writerChannelCapacity = 64
	writerAcceptedLimit   = 128
)

// writerHooks lets a test act before a worker boundary; production workers have none.
type writerHooks interface {
	before(boundary writerBoundary, bound *stepBound) error
}

// stepBound is one worker step's limits: real time for the step itself, so a virtual-time jump
// never expires it, and the shutdown deadline on the worker's clock.
type stepBound struct {
	until    time.Time
	shutdown *sharedDeadline
	clock    clock
}

func newStepBound(shutdown *sharedDeadline, c clock) *stepBound {
	return stepBoundWithin(writerStepBound, shutdown, c)
}

func stepBoundWithin(limit time.Duration, shutdown *sharedDeadline, c clock) *stepBound {
	return &stepBound{until: time.Now().Add(limit), shutdown: shutdown, clock: c}
}

func (b *stepBound) check() error {
	end, set := b.shutdown.instant().get()
	if !time.Now().Before(b.until) || (set && !b.clock.now().Before(end)) {
		return deadlineExceededError()
	}
	return nil
}

type writerStatus struct {
	pendingCount uint64
	gapCount     uint64
	diagnostic   opt[string]
}

// retainReason is why trySubmit refused a transition, which decides how the controller routes it.
type retainReason uint8

const (
	retainReasonEvidenceInvalid retainReason = iota + 1
	retainReasonChannelFull
	retainReasonStopped
	retainReasonSequenceRegressed
	retainReasonFenced
)

type retainedTransition struct {
	transition observedTransition
	cause      *cafleetError
	reason     retainReason
}

// writerMailbox is the state the writer and its worker share; mu guards every field except
// deadline, which synchronizes itself.
type writerMailbox struct {
	mu           sync.Mutex
	pending      uint64
	gaps         uint64
	diagnostic   opt[string]
	lastSequence int64
	stop         bool
	fenced       bool
	deadline     *sharedDeadline
	diagnostics  runnerDiagnostics
}

func (m *writerMailbox) failure(cause *cafleetError, at string) {
	m.gaps = checkedAddUint64(m.gaps, 1, "writer gap count")
	message := cause.message()
	if m.diagnostic != some(message) {
		m.diagnostics.record(runnerDiagnosticSourceEventWriter, "event writer: "+message, at)
	}
	m.diagnostic = some(message)
}

func (m *writerMailbox) fence(cause *cafleetError, discarded uint64, at string) {
	if discarded > m.pending {
		panic("discarded transitions were accepted")
	}
	m.pending -= discarded
	m.gaps = checkedAddUint64(m.gaps, discarded, "writer gap count")
	diagnostic := "event writer fenced: " + cause.message()
	m.diagnostics.record(runnerDiagnosticSourceEventWriter, diagnostic, at)
	m.diagnostic = some(diagnostic)
	m.fenced = true
}

func (m *writerMailbox) success() {
	m.diagnostic = none[string]()
}

// eventWriter persists accepted transitions in order on one worker goroutine, which alone owns
// the writer's database connection. Its owner calls close, the Rust Drop, which finishes it.
type eventWriter struct {
	sender      chan *observedTransition
	mailbox     *writerMailbox
	done        chan struct{}
	joined      bool
	clock       clock
	diagnostics runnerDiagnostics
}

func eventWriterStart(databaseURL string, run runRecord, lock *runLock, c clock, diagnostics runnerDiagnostics) (*eventWriter, error) {
	return eventWriterStartWorker(databaseURL, run, lock, c, diagnostics, nil)
}

// eventWriterStartWorker starts the writer; hooks is nil outside tests.
func eventWriterStartWorker(databaseURL string, run runRecord, lock *runLock, c clock, diagnostics runnerDiagnostics, hooks writerHooks) (*eventWriter, error) {
	ctx := context.Background()
	conn, err := connect(databaseURL)
	if err != nil {
		return nil, err
	}
	if _, err := withDatabaseDeadline(ctx, conn, c.now().Add(time.Second), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, lock.validate(ctx, conn, run.owner())
	}); err != nil {
		return nil, mergeCleanup(err, conn.close())
	}
	channel := make(chan *observedTransition, writerChannelCapacity)
	mailbox := &writerMailbox{deadline: &sharedDeadline{}, diagnostics: diagnostics}
	worker := &writerWorker{
		conn:        conn,
		run:         run,
		lock:        lock,
		mailbox:     mailbox,
		receiver:    channel,
		shutdown:    mailbox.deadline,
		flushNext:   c.now(),
		diagnostics: diagnostics,
		clock:       c,
		hooks:       hooks,
	}
	done := make(chan struct{})
	go func() {
		defer close(done)
		worker.serve()
	}()
	return &eventWriter{sender: channel, mailbox: mailbox, done: done, clock: c, diagnostics: diagnostics}, nil
}

// runnerDiagnostics is the runner diagnostics buffer this writer flushes.
func (w *eventWriter) runnerDiagnostics() runnerDiagnostics {
	return w.diagnostics
}

func (w *eventWriter) send(transition *observedTransition) error {
	select {
	case w.sender <- transition:
		return nil
	default:
		return runtimeError("runtime observation channel is full or unavailable")
	}
}

// trySubmit accepts transition for persistence, or returns it retained with why it was refused.
func (w *eventWriter) trySubmit(transition observedTransition) *retainedTransition {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	state := w.mailbox
	var reason retainReason
	var cause *cafleetError
	switch {
	case state.fenced:
		reason, cause = retainReasonFenced, runtimeError("event writer is fenced")
	case state.stop:
		reason, cause = retainReasonStopped, runtimeError("event writer is stopped")
	case transition.EventSeq <= state.lastSequence:
		reason, cause = retainReasonSequenceRegressed, runtimeError("accepted event sequence regressed")
	default:
		if err := validateEvidence(transition.EvidenceJSON); err != nil {
			reason, cause = retainReasonEvidenceInvalid, cafleetErrorOf(err)
		} else if err := w.send(&transition); err != nil {
			reason, cause = retainReasonChannelFull, cafleetErrorOf(err)
		}
	}
	if cause == nil {
		state.pending = checkedAddUint64(state.pending, 1, "writer pending")
		if state.pending > writerAcceptedLimit {
			panic("bounded accepted writer capacity")
		}
		state.lastSequence = transition.EventSeq
		return nil
	}
	if reason != retainReasonEvidenceInvalid && reason != retainReasonFenced {
		state.failure(cause, wallStamp(w.clock))
	}
	return &retainedTransition{transition: transition, cause: cause, reason: reason}
}

func (w *eventWriter) status() writerStatus {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	return writerStatus{pendingCount: w.mailbox.pending, gapCount: w.mailbox.gaps, diagnostic: w.mailbox.diagnostic}
}

func (w *eventWriter) diagnostic() opt[string] {
	return w.status().diagnostic
}

func (w *eventWriter) setDeadline(until time.Time) {
	w.mailbox.deadline.lower(until)
}

func (w *eventWriter) beginShutdown(until time.Time) {
	w.setDeadline(until)
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	w.mailbox.stop = true
}

func (w *eventWriter) workerFinished() bool {
	select {
	case <-w.done:
		return true
	default:
		return false
	}
}

// finish joins the worker once it finishes or the shutdown deadline plus a short grace passes; an
// in-flight step aborts at the shutdown deadline, so the join returns. It returns the pending
// count and the gaps left unsaved.
func (w *eventWriter) finish(until time.Time) (uint64, uint64) {
	w.beginShutdown(until)
	deadline, set := w.mailbox.deadline.instant().get()
	if !set {
		panic("shutdown deadline")
	}
	if !w.joined {
		joinBy := deadline.Add(writerJoinGrace)
		for !w.workerFinished() && w.clock.now().Before(joinBy) {
			napUntil(w.clock, joinBy)
		}
		<-w.done
		w.joined = true
	}
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	unsaved := uint64(0)
	if _, failing := w.mailbox.diagnostic.get(); failing {
		unsaved = w.mailbox.gaps
	}
	return w.mailbox.pending, unsaved
}

func (w *eventWriter) close() {
	until, set := w.mailbox.deadline.instant().get()
	if !set {
		until = w.clock.now().Add(4 * time.Second)
	}
	w.finish(until)
}

type writerWorker struct {
	conn        *dbConn
	run         runRecord
	lock        *runLock
	mailbox     *writerMailbox
	receiver    <-chan *observedTransition
	queue       []*observedTransition
	shutdown    *sharedDeadline
	flushNext   time.Time
	diagnostics runnerDiagnostics
	clock       clock
	hooks       writerHooks
}

func (w *writerWorker) stopping() bool {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	if end, set := w.mailbox.deadline.instant().get(); set && !w.clock.now().Before(end) {
		return true
	}
	return w.mailbox.stop && w.mailbox.pending == 0
}

func (w *writerWorker) pending() uint64 {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	return w.mailbox.pending
}

func (w *writerWorker) serve() {
	defer w.conn.close() //nolint:errcheck // as the Rust worker's Connection drop, a close failure has no reader
	retry := 100 * time.Millisecond
	next := w.clock.now()
	for !w.stopping() {
		w.flushDiagnostics()
		if w.clock.now().Before(next) {
			napUntil(w.clock, next)
			continue
		}
		if err := w.step(); err != nil {
			w.mailbox.mu.Lock()
			w.mailbox.failure(cafleetErrorOf(err), wallStamp(w.clock))
			w.mailbox.mu.Unlock()
			next = w.clock.now().Add(retry)
			retry = min(retry*2, 2*time.Second)
		} else {
			retry = 100 * time.Millisecond
			next = w.clock.now()
		}
		if w.pending() == 0 {
			w.clock.sleep(2 * time.Millisecond)
		}
	}
}

func (w *writerWorker) fenced() bool {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	return w.mailbox.fenced
}

func (w *writerWorker) step() error {
	if w.fenced() {
		return nil
	}
	bound := newStepBound(w.shutdown, w.clock)
	proceed, err := w.receive(bound)
	if err != nil || !proceed {
		return err
	}
	return w.persistFront(bound)
}

func (w *writerWorker) before(boundary writerBoundary, bound *stepBound) error {
	if w.hooks == nil {
		return nil
	}
	return w.hooks.before(boundary, bound)
}

// receive moves channel transitions into the queue; false means a test hook ended this pass
// before any persistence.
func (w *writerWorker) receive(bound *stepBound) (bool, error) {
	for len(w.queue) < writerChannelCapacity {
		if err := w.before(writerBoundaryReceive, bound); err != nil {
			if cafleetErrorOf(err).kind == errorDeadlineExceeded {
				return false, nil
			}
			return false, err
		}
		if err := bound.check(); err != nil {
			return false, err
		}
		select {
		case transition := <-w.receiver:
			w.queue = append(w.queue, transition)
		default:
			return true, nil
		}
	}
	return true, nil
}

// persistFront persists the queue front. A permanent failure is replaced within this step by
// its rejection event under a fresh bound, so it opens no gap; a rejection that fails permanently
// fences the writer. A transient failure returns for the ordinary retry backoff.
func (w *writerWorker) persistFront(bound *stepBound) error {
	if len(w.queue) == 0 {
		return nil
	}
	err := w.persist(bound)
	if err == nil {
		return nil
	}
	cause := cafleetErrorOf(err)
	if cause.transientStorage() {
		return err
	}
	front := w.queue[0]
	if isEventRejection(*front) {
		w.fence(cause)
		return nil
	}
	screenSeq, ok := jsonUint64(jsonIndex(front.EvidenceJSON, "screen_seq"))
	if !ok {
		panic("trySubmit validated the front's evidence")
	}
	at := wallStamp(w.clock)
	w.diagnostics.record(runnerDiagnosticSourceEventWriter,
		fmt.Sprintf("event writer rejected %s event %d: %s", front.Kind.asStr(), front.EventSeq, cause.message()), at)
	replacement, err := eventRejectedTransition(w.run, *front, screenSeq, cause, at)
	if err != nil {
		w.fence(cafleetErrorOf(err))
		return nil
	}
	w.queue[0] = &replacement
	if err := w.persist(newStepBound(w.shutdown, w.clock)); err != nil {
		if cafleetErrorOf(err).transientStorage() {
			return err
		}
		w.fence(cafleetErrorOf(err))
	}
	return nil
}

func (w *writerWorker) persist(bound *stepBound) error {
	if len(w.queue) == 0 {
		panic("persist requires a front")
	}
	transition := w.queue[0]
	if err := w.before(writerBoundaryPersist, bound); err != nil {
		return err
	}
	if _, err := withSharedDatabaseDeadline(context.Background(), w.conn, bound.until, w.shutdown, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := w.lock.validate(ctx, conn, w.run.owner()); err != nil {
			return struct{}{}, err
		}
		_, _, err := persistTransition(ctx, conn, w.run.owner(), *transition, persistenceModeLive)
		return struct{}{}, err
	}); err != nil {
		return err
	}
	w.queue = w.queue[1:]
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	if w.mailbox.pending == 0 {
		panic("accepted writer count")
	}
	w.mailbox.pending--
	w.mailbox.success()
	return nil
}

// fence discards every accepted transition, the channel's included, and accepts no more.
func (w *writerWorker) fence(cause *cafleetError) {
	w.mailbox.mu.Lock()
	defer w.mailbox.mu.Unlock()
	discarded := uint64(len(w.queue))
	w.queue = nil
	for drained := true; drained; {
		select {
		case <-w.receiver:
			discarded++
		default:
			drained = false
		}
	}
	w.mailbox.fence(cause, discarded, wallStamp(w.clock))
}

// flushDiagnostics writes buffered runner diagnostics once the retry time arrives. A flush never
// records a writer failure, so it cannot open an observation gap.
func (w *writerWorker) flushDiagnostics() {
	if w.clock.now().Before(w.flushNext) || !w.diagnostics.pending() {
		return
	}
	entries, dropped := w.diagnostics.drain()
	bound := stepBoundWithin(diagnosticFlushBound, w.shutdown, w.clock)
	at := wallStamp(w.clock)
	if err := w.writeDiagnostics(bound, entries, dropped, at); err != nil {
		if cafleetErrorOf(err).transientStorage() {
			w.diagnostics.restore(entries, dropped)
		} else {
			w.diagnostics.discard(entries, dropped)
		}
		w.flushNext = w.clock.now().Add(diagnosticFlushRetry)
		return
	}
	w.flushNext = w.clock.now()
}

func (w *writerWorker) writeDiagnostics(bound *stepBound, entries []runnerDiagnosticEntry, dropped uint64, at string) error {
	if err := w.before(writerBoundaryFlush, bound); err != nil {
		return err
	}
	_, err := withSharedDatabaseDeadline(context.Background(), w.conn, bound.until, w.shutdown, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := w.lock.validate(ctx, conn, w.run.owner()); err != nil {
			return struct{}{}, err
		}
		return struct{}{}, persistRunnerDiagnostics(ctx, conn, w.run.owner(), entries, dropped, at)
	})
	return err
}
