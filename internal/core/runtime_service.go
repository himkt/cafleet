package core

import (
	"context"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"os"
	"sync"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"golang.org/x/sys/unix"
)

func runtimeServiceTerminalSize(terminal *os.File) (terminalSize, error) {
	size, err := unix.IoctlGetWinsize(int(terminal.Fd()), unix.TIOCGWINSZ)
	if err != nil {
		return terminalSize{}, ioError(ioErrorText(err))
	}
	result := terminalSize{rows: size.Row, cols: size.Col}
	if err := result.validate(); err != nil {
		return terminalSize{}, err
	}
	return result, nil
}

// sweepSnapshot is the sweep's read snapshot: the run's member is an active ordinary member whose
// recorded run is this run with a concrete placement, and its oldest unnotified pending message.
func sweepSnapshot(ctx context.Context, conn *dbConn, run runRecord) (opt[messageRecord], error) {
	return inTransaction(conn, transactionDeferred, func() (opt[messageRecord], error) {
		fleetID, active, err := activeMemberFleetOf(ctx, conn, run.memberID)
		if err != nil || !active {
			return none[messageRecord](), err
		}
		member, err := getMember(ctx, conn, run.memberID, fleetID)
		if err != nil {
			return none[messageRecord](), err
		}
		record, found := member.get()
		if !found || record.kind != memberKindMember || !record.placement.muxPaneID.set || !recordsRun(record, run.runID) {
			return none[messageRecord](), nil
		}
		return oldestPendingPreview(ctx, conn, run.memberID)
	})
}

func activeMemberFleetOf(ctx context.Context, conn *dbConn, memberID int64) (int64, bool, error) {
	fleet, err := activeMemberFleet(ctx, conn, memberID)
	if err != nil {
		return 0, false, err
	}
	fleetID, active := fleet.get()
	return fleetID, active, nil
}

func recordsRun(member memberRecord, runID int64) bool {
	snapshot, recorded := member.runtime.get()
	return recorded && snapshot.run.runID == runID
}

// directorSweepThread is the Director sweep on its own goroutine, connection and multiplexer
// adapter.
type directorSweepThread struct {
	stop     *atomic.Bool
	shutdown *shutdownInstant
	done     chan struct{}
}

// shutdownInstant is the shutdown deadline the runner hands the sweep, unset until then.
type shutdownInstant struct {
	mu sync.Mutex
	at opt[time.Time]
}

func (s *shutdownInstant) get() opt[time.Time] {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.at
}

func (s *shutdownInstant) set(at time.Time) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.at = some(at)
}

func directorSweepThreadStart(databaseURL string, run runRecord, catalog *profileCatalog, c clock, diagnostics runnerDiagnostics) *directorSweepThread {
	return directorSweepThreadStartWithRunner(databaseURL, run, catalog, c, diagnostics, func() commandRunner { return systemRunner{} })
}

// directorSweepThreadStartWithRunner starts the sweep with the command runner runner builds on
// the sweep goroutine.
func directorSweepThreadStartWithRunner(databaseURL string, run runRecord, catalog *profileCatalog, c clock, diagnostics runnerDiagnostics, runner func() commandRunner) *directorSweepThread {
	thread := &directorSweepThread{stop: &atomic.Bool{}, shutdown: &shutdownInstant{}, done: make(chan struct{})}
	go func() {
		defer close(thread.done)
		bounded := shutdownBoundRunner{inner: runner(), shutdown: thread.shutdown, clock: c}
		sweep := newDirectorSweep(run, catalog, bounded, c, diagnostics)
		var conn *dbConn
		defer func() {
			if conn != nil {
				if err := conn.close(); err != nil {
					sweep.storageUnavailable(cafleetErrorOf(err))
				}
			}
		}()
		for !thread.stop.Load() {
			shutdown := thread.shutdown.get()
			if conn == nil {
				conn = sweep.connect(databaseURL)
			}
			if conn != nil {
				if err := sweep.tick(context.Background(), conn, thread.stop, shutdown); err != nil {
					sweep.storageUnavailable(cafleetErrorOf(err))
				}
			}
			c.sleep(directorSweepIdle)
		}
	}()
	return thread
}

// stopAndJoin clips every later attempt to shutdown, then stops the sweep before its next stage;
// a stopped attempt starts no new writing stage.
func (t *directorSweepThread) stopAndJoin(shutdown time.Time) {
	t.shutdown.set(shutdown)
	t.stop.Store(true)
	<-t.done
}

// shutdownBoundRunner clips every subprocess of the sweep to the shutdown deadline once the
// runner hands it over, including a process of an attempt already in flight.
type shutdownBoundRunner struct {
	inner    commandRunner
	shutdown *shutdownInstant
	clock    clock
}

func (r shutdownBoundRunner) binaryExists(name string) bool {
	return r.inner.binaryExists(name)
}

func (r shutdownBoundRunner) run(argv []string) (string, *runError) {
	remaining, bounded := r.remaining().get()
	if !bounded {
		return r.inner.run(argv)
	}
	return r.runFor(argv, remaining)
}

func (r shutdownBoundRunner) runFor(argv []string, timeout time.Duration) (string, *runError) {
	if remaining, bounded := r.remaining().get(); bounded {
		timeout = min(timeout, remaining)
	}
	if timeout <= 0 {
		return "", runErrorDeadlineExpired()
	}
	return r.inner.runFor(argv, timeout)
}

func (r shutdownBoundRunner) remaining() opt[time.Duration] {
	shutdown, set := r.shutdown.get().get()
	if !set {
		return none[time.Duration]()
	}
	return some(max(shutdown.Sub(r.clock.now()), 0))
}

const directorSweepIdle = 100 * time.Millisecond

// directorTarget is the root Director's oldest pending message and the placement the Director
// sweep types into.
type directorTarget struct {
	message   messageRecord
	placement placement
}

// directorSweepSnapshot is one read transaction: the run's member is current, the fleet's root
// Director is active with a concrete pane whose backend declares Director delivery on its
// multiplexer, and its oldest unnotified pending message.
func directorSweepSnapshot(ctx context.Context, conn *dbConn, run runRecord, catalog *profileCatalog) (opt[directorTarget], error) {
	return inTransaction(conn, transactionDeferred, func() (opt[directorTarget], error) {
		return runtimeServiceDirectorTarget(ctx, conn, run, catalog)
	})
}

func runtimeServiceDirectorTarget(ctx context.Context, conn *dbConn, run runRecord, catalog *profileCatalog) (opt[directorTarget], error) {
	fleetID, active, err := activeMemberFleetOf(ctx, conn, run.memberID)
	if err != nil || !active {
		return none[directorTarget](), err
	}
	member, err := getMember(ctx, conn, run.memberID, fleetID)
	if err != nil {
		return none[directorTarget](), err
	}
	record, found := member.get()
	current := found && record.kind == memberKindMember && recordsRun(record, run.runID)
	if !current {
		return none[directorTarget](), nil
	}
	directorFleet, err := activeMemberFleet(ctx, conn, run.directorMemberID)
	if err != nil || directorFleet != some(fleetID) {
		return none[directorTarget](), err
	}
	director, err := getMember(ctx, conn, run.directorMemberID, fleetID)
	if err != nil {
		return none[directorTarget](), err
	}
	directorRecord, found := director.get()
	if !found || directorRecord.kind != memberKindDirector {
		return none[directorTarget](), nil
	}
	target := directorRecord.placement
	if !target.muxPaneID.set {
		return none[directorTarget](), nil
	}
	backend, known := codingAgentCodingAgent(target.codingAgent)
	if !known {
		return none[directorTarget](), runtimeError("the Director placement names an unknown backend")
	}
	multiplexer, muxErr := muxBackendParse(target.backend)
	if muxErr != nil {
		return none[directorTarget](), runtimeError(muxErr.Error())
	}
	delivery, declared := catalog.directorDelivery(backend)
	if !declared || !delivery.covers(multiplexer) {
		return none[directorTarget](), nil
	}
	message, err := oldestPendingPreview(ctx, conn, run.directorMemberID)
	if err != nil {
		return none[directorTarget](), err
	}
	pending, found := message.get()
	if !found {
		return none[directorTarget](), nil
	}
	return some(directorTarget{message: pending, placement: target}), nil
}

const (
	directorSweepInterval    = time.Second
	directorAttempt          = 5 * time.Second
	directorSettle           = 500 * time.Millisecond
	directorVerification     = 2 * time.Second
	directorVerificationPoll = 100 * time.Millisecond
	directorWriteMargin      = 250 * time.Millisecond
	directorHeldReport       = 180 * time.Second
	sweepBackoffFloor        = time.Second
	sweepBackoffCap          = 30 * time.Second
)

// directorHold is why a Director preview waits with zero bytes written.
type directorHold uint8

const (
	directorHoldComposerBusy directorHold = iota + 1
	directorHoldInteractionOpen
	directorHoldPaneUnavailable
	directorHoldStorageUnavailable
)

func (h directorHold) reason() string {
	switch h {
	case directorHoldComposerBusy:
		return "composer_busy"
	case directorHoldInteractionOpen:
		return "interaction_open"
	case directorHoldPaneUnavailable:
		return "pane_unavailable"
	case directorHoldStorageUnavailable:
		return "storage_unavailable"
	default:
		panic("invalid directorHold")
	}
}

// directorOutcome is how one locked Director attempt ended.
//
//sumtype:decl
type directorOutcome interface{ isDirectorOutcome() }

type directorOutcomeDelivered struct{}

type directorOutcomeHeld struct{ hold directorHold }

// directorOutcomeUnavailable is a capture or multiplexer failure before any byte, with its cause.
type directorOutcomeUnavailable struct{ cause string }

// directorOutcomeUncertain means bytes may be in the Director composer and the message stays
// unnotified.
type directorOutcomeUncertain struct{ cause string }

// directorOutcomeNotStarted means the stop signal or the attempt budget ended the attempt before
// its first byte.
type directorOutcomeNotStarted struct{}

func (directorOutcomeDelivered) isDirectorOutcome()   {}
func (directorOutcomeHeld) isDirectorOutcome()        {}
func (directorOutcomeUnavailable) isDirectorOutcome() {}
func (directorOutcomeUncertain) isDirectorOutcome()   {}
func (directorOutcomeNotStarted) isDirectorOutcome()  {}

// heldHead is a held head's continuous hold, reported once when it reaches directorHeldReport.
type heldHead struct {
	messageID int64
	since     time.Time
	reported  bool
}

// directorSweep types the root Director's pending previews into its pane from one member runner;
// it owns no member PTY, receipt or runtime event.
type directorSweep struct {
	run          runRecord
	catalog      *profileCatalog
	runner       commandRunner
	clock        clock
	diagnostics  runnerDiagnostics
	nextPass     opt[time.Time]
	backoff      time.Duration
	backoffUntil opt[time.Time]
	held         opt[heldHead]
}

func newDirectorSweep(run runRecord, catalog *profileCatalog, runner commandRunner, c clock, diagnostics runnerDiagnostics) *directorSweep {
	return &directorSweep{run: run, catalog: catalog, runner: runner, clock: c, diagnostics: diagnostics, backoff: sweepBackoffFloor}
}

// connect is the sweep's connection, retried under the storage backoff with a diagnostic per
// failure; nil while it backs off or fails.
func (s *directorSweep) connect(databaseURL string) *dbConn {
	if until, set := s.backoffUntil.get(); set && s.clock.now().Before(until) {
		return nil
	}
	conn, err := connect(databaseURL)
	if err != nil {
		s.storageUnavailable(cafleetErrorOf(err))
		return nil
	}
	return conn
}

// storageUnavailable records a storage failure that no selected message owns and backs off.
func (s *directorSweep) storageUnavailable(cause *cafleetError) {
	s.record("director delivery: storage unavailable: " + cause.message())
	s.backOff(s.clock.now())
}

// storageHold holds messageID for storage and backs off.
func (s *directorSweep) storageHold(messageID int64) {
	s.noteHold(messageID, directorHoldStorageUnavailable)
	s.backOff(s.clock.now())
}

// storageUntil is the real instant as far from now as at is from the sweep clock's now.
func (s *directorSweep) storageUntil(at time.Time) time.Time {
	return time.Now().Add(max(at.Sub(s.clock.now()), 0))
}

// tick runs one sweep pass at most once per interval and after the backoff; every storage step
// and the attempt are clipped to shutdown.
func (s *directorSweep) tick(ctx context.Context, conn *dbConn, stop *atomic.Bool, shutdown opt[time.Time]) error {
	now := s.clock.now()
	if next, set := s.nextPass.get(); set && now.Before(next) {
		return nil
	}
	if until, set := s.backoffUntil.get(); set && now.Before(until) {
		return nil
	}
	s.nextPass = some(now.Add(directorSweepInterval))
	clipped := func(at time.Time) time.Time {
		if limit, set := shutdown.get(); set && limit.Before(at) {
			return limit
		}
		return at
	}
	snapshot, err := sweepStorage(ctx, conn, s.storageUntil(clipped(now.Add(directorAttempt))), func(ctx context.Context, conn *dbConn) (opt[directorTarget], error) {
		return directorSweepSnapshot(ctx, conn, s.run, s.catalog)
	})
	if err != nil {
		if held, set := s.held.get(); set {
			s.storageHold(held.messageID)
		} else {
			s.storageUnavailable(cafleetErrorOf(err))
		}
		return nil
	}
	target, found := snapshot.get()
	if !found {
		s.held = none[heldHead]()
		return nil
	}
	messageID := target.message.messageID
	path := conn.path()
	if path == "" {
		return runtimeError("the Director sweep database has no file path")
	}
	lock, acquired, err := paneLockTryAcquire("sqlite:///"+path, target.placement)
	if err != nil {
		s.storageHold(messageID)
		return nil
	}
	if !acquired {
		return nil
	}
	defer lock.close()
	attemptDeadline := clipped(s.clock.now().Add(directorAttempt))
	rechecked, err := sweepStorage(ctx, conn, s.storageUntil(attemptDeadline), func(ctx context.Context, conn *dbConn) (opt[directorTarget], error) {
		return runtimeServiceDirectorTarget(ctx, conn, s.run, s.catalog)
	})
	if err != nil {
		s.storageHold(messageID)
		return nil
	}
	current, found := rechecked.get()
	if !found || current.message.messageID != messageID || current.placement != target.placement {
		return nil
	}
	switch outcome := s.attempt(target, attemptDeadline, stop).(type) {
	case directorOutcomeDelivered:
		if _, err := sweepStorage(ctx, conn, s.storageUntil(attemptDeadline), func(ctx context.Context, conn *dbConn) (bool, error) {
			return stampDirectorPreview(ctx, conn, s.run.owner(), messageID)
		}); err == nil {
			s.backoff = sweepBackoffFloor
			s.backoffUntil = none[time.Time]()
			s.held = none[heldHead]()
		} else {
			s.record(fmt.Sprintf("director delivery: message %d uncertain: storage_unavailable", messageID))
			s.backOff(s.clock.now())
		}
	case directorOutcomeHeld:
		s.noteHold(messageID, outcome.hold)
	case directorOutcomeUnavailable:
		pane, placed := target.placement.muxPaneID.get()
		if !placed {
			panic("a Director target has a concrete pane")
		}
		s.record(fmt.Sprintf("director delivery: pane %s unavailable: %s", pane, outcome.cause))
		s.noteHold(messageID, directorHoldPaneUnavailable)
		s.backOff(now)
	case directorOutcomeUncertain:
		s.record(fmt.Sprintf("director delivery: message %d uncertain: %s", messageID, outcome.cause))
	case directorOutcomeNotStarted:
	default:
		panic("invalid directorOutcome")
	}
	return nil
}

func (s *directorSweep) backOff(now time.Time) {
	s.backoffUntil = some(now.Add(s.backoff))
	s.backoff = min(s.backoff*2, sweepBackoffCap)
}

func (s *directorSweep) record(message string) {
	s.diagnostics.record(runnerDiagnosticSourceDirectorDelivery, message, wallStamp(s.clock))
}

func (s *directorSweep) noteHold(messageID int64, hold directorHold) {
	now := s.clock.now()
	held, set := s.held.get()
	if !set || held.messageID != messageID {
		held = heldHead{messageID: messageID, since: now}
	}
	reached := now.Sub(held.since) >= directorHeldReport
	if reached && !held.reported {
		s.record(fmt.Sprintf("director delivery: message %d held: %s", messageID, hold.reason()))
	}
	held.reported = held.reported || reached
	s.held = some(held)
}

func (s *directorSweep) attempt(target directorTarget, attemptDeadline time.Time, stop *atomic.Bool) directorOutcome {
	p := target.placement
	pane, placed := p.muxPaneID.get()
	if !placed {
		panic("a Director target has a concrete pane")
	}
	backend, known := codingAgentCodingAgent(p.codingAgent)
	if !known {
		panic("the snapshot resolved the Director backend")
	}
	delivery, declared := s.catalog.directorDelivery(backend)
	if !declared {
		panic("the snapshot found Director delivery")
	}
	plan, planned := s.catalog.composerPlan(backend, composerOperationPrompt)
	if !planned {
		panic("Director delivery requires a prompt plan")
	}
	detector := newDetector(s.catalog, backend, none[string]())
	stopped := stop.Load
	realDeadline := deadlineUntil(time.Now().Add(max(attemptDeadline.Sub(s.clock.now()), 0)))
	unbounded, muxErr := multiplexerForServer(p.backend, p.muxServer, s.runner)
	if muxErr != nil {
		return directorOutcomeUnavailable{cause: muxErr.Error()}
	}
	mux := unbounded.bounded(realDeadline)
	if stopped() {
		return directorOutcomeNotStarted{}
	}
	screen, cause := runtimeServiceCapture(mux, pane)
	if cause != nil {
		return directorOutcomeUnavailable{cause: *cause}
	}
	observed, err := detector.observeDirector(screen, delivery.ruleIDs)
	if err != nil {
		return directorOutcomeUnavailable{cause: cafleetErrorOf(err).message()}
	}
	if _, interaction := observed.kind.interaction(); interaction || observed.kind == observationKindPermissionContinuation {
		return directorOutcomeHeld{hold: directorHoldInteractionOpen}
	}
	if observed.kind != observationKindComposerEmpty || observed.matchEvidence != matchEvidenceComplete {
		return directorOutcomeHeld{hold: directorHoldComposerBusy}
	}
	line := canonicalPreviewLine(target.message)
	if !delivery.plainLiteralAllowed(line) {
		line = pointerPreviewLine(target.message)
	}
	stages, err := plan.stages(line, literalFramingPlain)
	if err != nil {
		panic("a measured Director prompt plan encodes a plain preview line: " + err.Error())
	}
	if !utf8.Valid(stages[0]) {
		panic("an encoded preview is UTF-8")
	}
	literal, submit := string(stages[0]), stages[1]
	required := directorSettle + directorVerification + directorWriteMargin
	if stopped() || attemptDeadline.Sub(s.clock.now()) < required {
		return directorOutcomeNotStarted{}
	}
	typed, typeErr := mux.typeLiteralUntil(pane, literal, stopped)
	switch {
	case typeErr != nil && typeErr.submissionStarted:
		return directorOutcomeUncertain{cause: "write_failed"}
	case typeErr != nil:
		return directorOutcomeUnavailable{cause: typeErr.Error()}
	case !typed:
		return directorOutcomeUncertain{cause: "runner_stopped"}
	}
	if stopped() {
		return directorOutcomeUncertain{cause: "runner_stopped"}
	}
	s.clock.sleep(directorSettle)
	if stopped() {
		return directorOutcomeUncertain{cause: "runner_stopped"}
	}
	windowUntil := s.clock.now().Add(preSubmitWindow)
	if limit := attemptDeadline.Add(-(directorVerification + directorWriteMargin)); limit.Before(windowUntil) {
		windowUntil = limit
	}
	for !s.composerShows(mux, detector, pane, literal) {
		if s.clock.now().Add(directorVerificationPoll).After(windowUntil) {
			return directorOutcomeUncertain{cause: "literal_mismatch"}
		}
		s.clock.sleep(directorVerificationPoll)
		if stopped() {
			return directorOutcomeUncertain{cause: "runner_stopped"}
		}
	}
	if stopped() {
		return directorOutcomeUncertain{cause: "runner_stopped"}
	}
	if mux.pressSubmit(pane, submit) != nil {
		return directorOutcomeUncertain{cause: "write_failed"}
	}
	verifyUntil := s.clock.now().Add(directorVerification)
	if attemptDeadline.Before(verifyUntil) {
		verifyUntil = attemptDeadline
	}
	for {
		if stopped() {
			return directorOutcomeUncertain{cause: "runner_stopped"}
		}
		if s.composerEmpty(mux, detector, delivery, pane) {
			return directorOutcomeDelivered{}
		}
		if s.clock.now().Add(directorVerificationPoll).After(verifyUntil) {
			return directorOutcomeUncertain{cause: "submit_unconfirmed"}
		}
		s.clock.sleep(directorVerificationPoll)
	}
}

// composerShows reports whether the Director composer renders literal; a failed capture or read
// shows nothing.
func (s *directorSweep) composerShows(mux multiplexer, d *detector, pane, literal string) bool {
	screen, cause := runtimeServiceCapture(mux, pane)
	if cause != nil {
		return false
	}
	text, read, err := d.composerText(screen)
	return err == nil && read && text.matches([]string{literal})
}

// composerEmpty reports whether the Director composer is verifiably empty; a failed capture or
// observation is not.
func (s *directorSweep) composerEmpty(mux multiplexer, d *detector, delivery *directorDelivery, pane string) bool {
	screen, cause := runtimeServiceCapture(mux, pane)
	if cause != nil {
		return false
	}
	observed, err := d.observeDirector(screen, delivery.ruleIDs)
	return err == nil && observed.kind == observationKindComposerEmpty && observed.matchEvidence == matchEvidenceComplete
}

// runtimeServiceCapture is the Director pane's screen as a replayed snapshot, or the capture failure's text.
func runtimeServiceCapture(mux multiplexer, pane string) (*screenSnapshot, *string) {
	screen, err := mux.captureScreen(pane)
	if err != nil {
		cause := err.Error()
		return nil, &cause
	}
	snapshot, replayErr := terminalStateReplay(screen)
	if replayErr != nil {
		cause := cafleetErrorOf(replayErr).message()
		return nil, &cause
	}
	return &snapshot, nil
}

// client is one accepted control connection: its nonblocking socket, frame decoder and queued
// reply bytes.
type client struct {
	id      uint64
	fd      int
	decoder *controlDecoder
	output  []byte
	closed  bool
}

// poll reads what the connection has and returns the frames it completes; at most 16 per tick.
func (c *client) poll(cl clock) ([]admittedControlFrame, error) {
	requests := []admittedControlFrame{}
	buffer := make([]byte, 4096)
	count, err := unix.Read(c.fd, buffer)
	switch {
	case errors.Is(err, unix.EAGAIN), errors.Is(err, unix.EINTR):
	case err != nil:
		return nil, ioError(ioErrorText(err))
	case count == 0:
		if err := c.decoder.finish(); err != nil {
			return nil, err
		}
		c.closed = true
	default:
		if requests, err = c.decoder.feedAdmitted(buffer[:count], cl.now()); err != nil {
			return nil, err
		}
	}
	if err := ensure(len(requests) <= 16, "too many control requests per tick"); err != nil {
		return nil, err
	}
	if err := c.flush(); err != nil {
		return nil, err
	}
	return requests, nil
}

// answer queues a lifecycle reply, or the error reply when cause is set; false, with the client
// closed, when it cannot be queued.
func (c *client) answer(result jsontext.Value, cause *cafleetError) bool {
	var encoded []byte
	var err error
	if cause != nil {
		encoded, err = encodeErrorReply(cause)
	} else {
		encoded, err = encodeResultReply(result)
	}
	if err == nil {
		err = c.queueReply(encoded)
	}
	c.closed = c.closed || err != nil
	return err == nil
}

// answerInput queues an input reply, or the error reply when cause is set; false, with the client
// closed, when it cannot be queued.
func (c *client) answerInput(reply inputControlReply, cause *cafleetError) bool {
	var encoded []byte
	var err error
	if cause != nil {
		encoded, err = encodeErrorReply(cause)
	} else {
		encoded, err = encodeInputReply(reply)
	}
	if err == nil {
		err = c.queueReply(encoded)
	}
	c.closed = c.closed || err != nil
	return err == nil
}

func (c *client) queueReply(data []byte) error {
	if err := ensure(len(c.output)+len(data) <= maxControlFrame+4, "control reply buffer is full"); err != nil {
		return err
	}
	c.output = append(c.output, data...)
	return c.flush()
}

func (c *client) flush() error {
	if len(c.output) == 0 {
		return nil
	}
	count, err := unix.Write(c.fd, c.output)
	switch {
	case errors.Is(err, unix.EAGAIN), errors.Is(err, unix.EINTR):
	case err != nil:
		return ioError(ioErrorText(err))
	case count == 0:
		c.closed = true
	default:
		c.output = c.output[count:]
	}
	return nil
}

func shutdownDiagnostics(diagnostics runnerDiagnostics, outcome shutdownOutcome, at string) ([]runnerDiagnosticEntry, uint64) {
	entries, dropped := diagnostics.drain()
	for _, diagnostic := range outcome.diagnostics {
		entries = append(entries, runnerDiagnosticEntry{
			source:  runnerDiagnosticSourceShutdown,
			message: diagnosticText(diagnostic),
			firstAt: at,
			lastAt:  at,
			count:   1,
		})
	}
	return entries, dropped
}

// sweepStorage is withDatabaseDeadline until `until`, rolling back a transaction that a failed
// or interrupted statement left open, so the next pass starts from autocommit as a dropped Rust
// transaction does.
func sweepStorage[T any](ctx context.Context, conn *dbConn, until time.Time, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	value, err := withDatabaseDeadline(ctx, conn, until, operation)
	if err != nil {
		var zero T
		return zero, mergeCleanup(err, conn.rollbackIfOpen())
	}
	return value, nil
}

const (
	idleTickSleep             = 2 * time.Millisecond
	deferredRetryInterval     = 100 * time.Millisecond
	previewSweepInterval      = time.Second
	queueHousekeepingInterval = time.Second
	sweepAttemptBudget        = 5 * time.Second
)

type sweepBackoff struct {
	current time.Duration
}

func (b *sweepBackoff) delay() time.Duration {
	return b.current
}

func (b *sweepBackoff) transient() {
	if b.current == 0 {
		b.current = sweepBackoffFloor
		return
	}
	b.current = min(b.current*2, sweepBackoffCap)
}

func (b *sweepBackoff) reset() {
	b.current = 0
}

// previewSweep paces the pending-preview sweep: one pass per interval, none before the run-wide
// backoff expires.
type previewSweep struct {
	nextAt       opt[time.Time]
	backoff      sweepBackoff
	backoffUntil opt[time.Time]
	eligibility  opt[inputEligibility]
	// disabled is a test hook: a disabled sweep never comes due.
	disabled bool
}

func (s *previewSweep) due(now time.Time) bool {
	if s.disabled {
		return false
	}
	next, scheduled := s.nextAt.get()
	until, backingOff := s.backoffUntil.get()
	due := (!scheduled || !now.Before(next)) && (!backingOff || !now.Before(until))
	if due {
		s.nextAt = some(now.Add(previewSweepInterval))
	}
	return due
}

// observe resets the backoff when an interaction closes or a candidate clears.
func (s *previewSweep) observe(eligibility inputEligibility) {
	previous, seen := s.eligibility.get()
	if seen && holdsForInteraction(previous) && !holdsForInteraction(eligibility) {
		s.reset()
	}
	s.eligibility = some(eligibility)
}

func holdsForInteraction(eligibility inputEligibility) bool {
	hold, held := eligibility.(inputEligibilityHold)
	return held && (hold.reason == gateReasonInteraction || hold.reason == gateReasonCandidate)
}

func (s *previewSweep) backOff(now time.Time) {
	s.backoff.transient()
	s.backoffUntil = some(now.Add(s.backoff.delay()))
}

func (s *previewSweep) reset() {
	s.backoff.reset()
	s.backoffUntil = none[time.Time]()
}

type retryGate struct {
	visibility  visibility
	eligibility inputEligibility
}

// deferredRetry paces queued-input retries on the 100 ms timer, waking early when the observed
// gate changes, such as an interaction closing or a candidate clearing, or when an active input
// completes.
type deferredRetry struct {
	nextAt opt[time.Time]
	gate   opt[retryGate]
}

func (r *deferredRetry) due(writing bool, status observationStatus, now time.Time) bool {
	gate := retryGate{visibility: status.visibility, eligibility: status.eligibility}
	next, scheduled := r.nextAt.get()
	due := writing || r.gate != some(gate) || !scheduled || !now.Before(next)
	if due {
		r.gate = some(gate)
		r.nextAt = some(now.Add(deferredRetryInterval))
	}
	return due
}

func tickSleep(progress transportProgress, nextDeadline opt[time.Time], now time.Time) time.Duration {
	if progress.humanBytes > 0 || progress.forwardedBytes > 0 {
		return 0
	}
	at, scheduled := nextDeadline.get()
	if !scheduled {
		return idleTickSleep
	}
	return min(idleTickSleep, max(at.Sub(now), 0))
}
