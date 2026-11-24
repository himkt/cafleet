package core

import (
	"context"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"math"
	"os"
	"os/signal"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
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

// pendingAttempt is the in-flight immediate input attempt that later turns drive to its reply.
//
//sumtype:decl
type pendingAttempt interface{ isPendingAttempt() }

// pendingAttemptClient is a control client's attempt, answered on that client.
type pendingAttemptClient struct {
	id       uint64
	deadline time.Time
}

// pendingAttemptSweep is the runner's own sweep preview, which has no client to answer.
type pendingAttemptSweep struct {
	deadline time.Time
}

func (pendingAttemptClient) isPendingAttempt() {}
func (pendingAttemptSweep) isPendingAttempt()  {}

// inputTurnOutcome is what a turn's input work asks the runner loop to do next.
//
//sumtype:decl
type inputTurnOutcome interface{ isInputTurnOutcome() }

type inputTurnContinue struct{}

type inputTurnShutdown struct {
	deadlines shutdownDeadlines
}

// inputTurnReply is a control client's completed immediate attempt and its reply, or the cause
// it failed with.
type inputTurnReply struct {
	client uint64
	reply  inputControlReply
	cause  *cafleetError
}

func (inputTurnContinue) isInputTurnOutcome() {}
func (inputTurnShutdown) isInputTurnOutcome() {}
func (inputTurnReply) isInputTurnOutcome()    {}

// inputTurnIo is the runner I/O that input work drives between transport turns.
type inputTurnIo interface {
	clock() clock
	controller() *observationController
	quiesce(coordinator *inputCoordinator, deadline time.Time) (inputDrain, *inputFailure)
	advance(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, deadline time.Time) (opt[inputControlReply], error)
	// terminateRequested reports a termination request, which preempts any further write.
	terminateRequested() bool
	// resizeRequested reports a pending resize, which defers a write to a later turn.
	resizeRequested() bool
	// shutdownDeadlines is the deadlines of the shutdown a termination request starts.
	shutdownDeadlines() shutdownDeadlines
	// observeExit records an observed native exit and returns the shutdown it starts.
	observeExit(ctx context.Context, coordinator *inputCoordinator, conn *dbConn) (opt[shutdownDeadlines], error)
}

// inputTurns is the runner's input work after each transport turn: queue housekeeping, the
// in-flight immediate attempt, deferred retries and the preview sweep.
type inputTurns struct {
	pending             opt[pendingAttempt]
	previewSweep        previewSweep
	deferredRetry       deferredRetry
	queueHousekeepingAt opt[time.Time]
}

func newInputTurns() *inputTurns {
	return &inputTurns{}
}

// admitClient holds a client's accepted immediate attempt until a later turn completes it.
func (t *inputTurns) admitClient(id uint64, deadline time.Time) error {
	if err := ensure(!t.pending.set, "an accepted immediate input requires no other attempt in flight"); err != nil {
		return err
	}
	t.pending = some[pendingAttempt](pendingAttemptClient{id: id, deadline: deadline})
	return nil
}

// turn runs one turn's input work in the runner loop's order.
func (t *inputTurns) turn(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, run runRecord, io inputTurnIo) (inputTurnOutcome, error) {
	if err := t.housekeepQueue(ctx, coordinator, conn, io); err != nil {
		return nil, err
	}
	if t.pending.set {
		return t.completePendingInput(ctx, coordinator, conn, io)
	}
	if err := t.retryDeferred(ctx, coordinator, conn, io); err != nil {
		return nil, err
	}
	if err := t.sweepPreviews(ctx, coordinator, conn, run, io); err != nil {
		return nil, err
	}
	return inputTurnContinue{}, nil
}

// completePendingInput drives the in-flight immediate input to its reply: a client-owned
// attempt answers its client, and the runner's own sweep attempt settles without a reply.
func (t *inputTurns) completePendingInput(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, io inputTurnIo) (inputTurnOutcome, error) {
	attempt, inFlight := t.pending.get()
	if !inFlight {
		panic("an attempt is in flight")
	}
	var deadline time.Time
	switch attempt := attempt.(type) {
	case pendingAttemptClient:
		deadline = attempt.deadline
	case pendingAttemptSweep:
		deadline = attempt.deadline
	default:
		panic("invalid pendingAttempt")
	}
	var reply opt[inputControlReply]
	var cause error
	drain, failure := io.quiesce(coordinator, deadline)
	switch {
	case failure != nil:
		failedReply, failed := coordinator.pendingAttemptFailed(ctx, conn, io.controller(), failure)
		if failed != nil {
			cause = failed.asError()
		} else {
			reply = some(failedReply)
		}
	case drain == inputDrainPending:
	case drain == inputDrainQuiescent:
		if io.terminateRequested() {
			return inputTurnShutdown{deadlines: io.shutdownDeadlines()}, nil
		}
		deadlines, err := io.observeExit(ctx, coordinator, conn)
		if err != nil {
			return nil, err
		}
		if exited, set := deadlines.get(); set {
			return inputTurnShutdown{deadlines: exited}, nil
		}
		if !io.resizeRequested() {
			reply, cause = io.advance(ctx, coordinator, conn, deadline)
		}
	default:
		panic("invalid inputDrain")
	}
	if cause == nil && !reply.set {
		return inputTurnContinue{}, nil
	}
	coordinator.finishPendingAttempt()
	t.deferredRetry.nextAt = none[time.Time]()
	t.pending = none[pendingAttempt]()
	completed, _ := reply.get()
	var failedCause *cafleetError
	if cause != nil {
		failedCause = cafleetErrorOf(cause)
	}
	switch attempt := attempt.(type) {
	case pendingAttemptSweep:
		t.finishSweep(coordinator, completed, failedCause, io.clock().now())
		return inputTurnContinue{}, nil
	case pendingAttemptClient:
		return inputTurnReply{client: attempt.id, reply: completed, cause: failedCause}, nil
	default:
		panic("invalid pendingAttempt")
	}
}

func (t *inputTurns) retryDeferred(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, io inputTurnIo) error {
	if !coordinator.hasDeferredWork() ||
		!t.deferredRetry.due(coordinator.deferredAttemptWriting(), io.controller().status(), io.clock().now()) {
		return nil
	}
	deadline := io.clock().now().Add(5 * time.Second)
	drain, failure := io.quiesce(coordinator, deadline)
	if failure != nil || drain != inputDrainQuiescent || io.terminateRequested() || io.resizeRequested() {
		return nil
	}
	if _, err := io.advance(ctx, coordinator, conn, deadline); err != nil {
		if coordinator.deferredFailed {
			return err
		}
		coordinator.recordDiagnostic(runnerDiagnosticSourceDeferredInput, "deferred input: "+cafleetErrorOf(err).message())
	}
	return nil
}

func (t *inputTurns) housekeepQueue(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, io inputTurnIo) error {
	now := io.clock().now()
	if at, set := t.queueHousekeepingAt.get(); set && now.Before(at) {
		return nil
	}
	t.queueHousekeepingAt = some(now.Add(queueHousekeepingInterval))
	if err := coordinator.queueHousekeeping(ctx, conn, io.controller()); err != nil {
		if coordinator.deferredFailed {
			return err
		}
		coordinator.recordDiagnostic(runnerDiagnosticSourceQueueHousekeeping, "queue housekeeping: "+cafleetErrorOf(err).message())
	}
	return nil
}

// sweepPreviews delivers the member's oldest unnotified pending message, strictly oldest-first,
// when no ordinary attempt is active and the FIFO is empty.
func (t *inputTurns) sweepPreviews(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, run runRecord, io inputTurnIo) error {
	now := io.clock().now()
	t.previewSweep.observe(io.controller().status().eligibility)
	if !coordinator.sweepMayStart() || !t.previewSweep.due(now) {
		return nil
	}
	deadline := now.Add(sweepAttemptBudget)
	snapshot, err := sweepStorage(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (opt[messageRecord], error) {
		return sweepSnapshot(ctx, conn, run)
	})
	if err != nil {
		recordSweepFailure(coordinator, cafleetErrorOf(err))
		t.previewSweep.backOff(io.clock().now())
		return nil
	}
	message, pending := snapshot.get()
	if !pending {
		return nil
	}
	reply, failure := coordinator.handlePreview(ctx, conn, message, io.controller(), deadline)
	switch {
	case failure != nil:
		t.finishSweep(coordinator, nil, failure.asError(), io.clock().now())
	case reply.set:
		completed, _ := reply.get()
		t.finishSweep(coordinator, completed, nil, io.clock().now())
	default:
		t.pending = some[pendingAttempt](pendingAttemptSweep{deadline: deadline})
	}
	return nil
}

// finishSweep feeds a completed sweep attempt's outcome to the run-wide backoff: a submitted
// preview resets it, and a transient obstacle arms it.
func (t *inputTurns) finishSweep(coordinator *inputCoordinator, reply inputControlReply, cause *cafleetError, now time.Time) {
	if cause != nil {
		recordSweepFailure(coordinator, cause)
		t.previewSweep.backOff(now)
		return
	}
	submission, isSubmission := reply.(submissionResult)
	if !isSubmission {
		panic("a preview submission replies with a submission")
	}
	switch submission.Status {
	case submissionStatusAccepted:
		t.previewSweep.reset()
	case submissionStatusHeld:
		reason, reasoned := submission.Reason.get()
		if reasoned && slices.Contains([]string{"pane_input_busy", "storage_unavailable", "observer_unavailable", "delivery_budget_exhausted"}, reason) {
			t.previewSweep.backOff(now)
		}
	case submissionStatusQueued, submissionStatusRejected, submissionStatusUncertain:
	default:
		panic("invalid submissionStatus")
	}
}

func recordSweepFailure(coordinator *inputCoordinator, cause *cafleetError) {
	coordinator.recordDiagnostic(runnerDiagnosticSourcePreviewSweep, "preview sweep: "+cause.message())
}

// publishStoppedResults fences ordinary input after the native exit and queues every durable
// deferred result that has no event yet.
func publishStoppedResults(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, runID int64, controller *observationController, c clock) error {
	deadline := c.now().Add(time.Second)
	if _, err := coordinator.fence(ctx, deadline); err != nil {
		return err
	}
	unpublished, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) ([]unpublishedDeliveryResult, error) {
		return unpublishedDeliveryResults(ctx, conn, runID)
	})
	if err != nil {
		return err
	}
	for _, result := range unpublished {
		controller.publishDeliveryResult(inputDeliveryResultCause(result.inputID, result.stage, result.stopCause))
	}
	return nil
}

// nonblockingFile is a nonblocking duplicate of a standard stream that restores the stream's
// original status flags.
type nonblockingFile struct {
	fd       int
	flags    int
	restored bool
}

func newNonblockingFile(source *os.File) (*nonblockingFile, error) {
	fd, err := unix.FcntlInt(source.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, ioError(errnoText(err))
	}
	flags, err := unix.FcntlInt(uintptr(fd), unix.F_GETFL, 0)
	if err != nil {
		return nil, mergeCleanup(ioError(errnoText(err)), closeFd(fd))
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		return nil, mergeCleanup(ioError(errnoText(err)), closeFd(fd))
	}
	return &nonblockingFile{fd: fd, flags: flags}, nil
}

func closeFd(fd int) error {
	if err := unix.Close(fd); err != nil {
		return ioError(errnoText(err))
	}
	return nil
}

func (f *nonblockingFile) restore() error {
	if f.restored {
		return nil
	}
	if _, err := unix.FcntlInt(uintptr(f.fd), unix.F_SETFL, f.flags); err != nil {
		return ioError(errnoText(err))
	}
	f.restored = true
	return nil
}

// close restores the flags, reporting a failure on stderr as the Rust Drop does, and closes the
// duplicate.
func (f *nonblockingFile) close() {
	if err := f.restore(); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
	if err := closeFd(f.fd); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
}

func (f *nonblockingFile) tryRead(bytes []byte) (int, error) {
	count, err := unix.Read(f.fd, bytes)
	return max(count, 0), err
}

func (f *nonblockingFile) tryWrite(bytes []byte) (int, error) {
	count, err := unix.Write(f.fd, bytes)
	return max(count, 0), err
}

func (f *nonblockingFile) tryFlush() error {
	return nil
}

// signalGuards sets terminate on SIGTERM, SIGINT or SIGHUP and resize on SIGWINCH until close.
type signalGuards struct {
	signals chan os.Signal
	done    chan struct{}
}

func newSignalGuards(terminate, resize *atomic.Bool) *signalGuards {
	guards := &signalGuards{signals: make(chan os.Signal, 8), done: make(chan struct{})}
	signal.Notify(guards.signals, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP, syscall.SIGWINCH)
	go func() {
		defer close(guards.done)
		for sig := range guards.signals {
			if sig == syscall.SIGWINCH {
				resize.Store(true)
			} else {
				terminate.Store(true)
			}
		}
	}()
	return guards
}

func (g *signalGuards) close() {
	signal.Stop(g.signals)
	close(g.signals)
	<-g.done
}

type systemRequestTokens struct{}

func (systemRequestTokens) fill(bytes *[32]byte) error {
	if err := randomBytes(bytes); err != nil {
		return ioError(cafleetErrorOf(err).message())
	}
	return nil
}

// nativeIo is the runner's native child I/O: the outer terminal streams, the PTY, the transport
// and the observation controller.
type nativeIo struct {
	input      *nonblockingFile
	pty        *ptySession
	output     *nonblockingFile
	transport  *transport
	controller *observationController
	writer     *eventWriter
	clock      clock
}

func (n *nativeIo) shutdownDeadlines() shutdownDeadlines {
	return shutdownDeadlinesStartingNow(n.clock)
}

func (n *nativeIo) quiesce(coordinator *inputCoordinator, deadline time.Time) (inputDrain, *inputFailure) {
	return progressWithManual(coordinator, n.transport, func(t *transport) (inputDrain, error) {
		return t.quiesceForInput(n.input, n.pty, n.output, n.controller.observer(n.writer, n.clock), deadline)
	})
}

// observeExit records a native exit after beforeExit has queued every deferred result, so the
// results precede runner_exited.
func (n *nativeIo) observeExit(nativeExit *opt[childExit], beforeExit func(*observationController) error) (opt[shutdownDeadlines], error) {
	exit, exited, err := n.pty.tryWait()
	if err != nil || !exited {
		return none[shutdownDeadlines](), err
	}
	*nativeExit = some(exit)
	if err := beforeExit(n.controller); err != nil {
		return none[shutdownDeadlines](), err
	}
	screen := n.transport.snapshot()
	if err := n.controller.childExited(exit, &screen, n.writer, n.clock); err != nil {
		return none[shutdownDeadlines](), err
	}
	return some(n.shutdownDeadlines()), nil
}

func inputDeadline(request inputControlRequest, admittedAt time.Time, c clock) (time.Time, error) {
	return receiveInputExpiration(request.ExpiresAtMonoNS, admittedAt, c)
}

func (r *runnerLoop) inspect(request inputControlRequest, admittedAt time.Time) (inputControlReply, *cafleetError) {
	operation, isInspection := request.Operation.(permissionInspectOperation)
	if !isInspection {
		panic("inspection receives an inspection operation")
	}
	scope := inputContext{runID: some(r.run.runID), interactionID: some(operation.input.InteractionID)}
	reply, err := r.inspectWithinDeadline(request, operation.input, admittedAt)
	if err != nil {
		return nil, scope.failureOf(err).asError()
	}
	return reply, nil
}

func (r *runnerLoop) inspectWithinDeadline(request inputControlRequest, input permissionInspectInput, admittedAt time.Time) (inputControlReply, error) {
	deadline, err := inputDeadline(request, admittedAt, r.native.clock)
	if err != nil {
		return nil, err
	}
	if err := request.Identity.validate(r.run); err != nil {
		return nil, err
	}
	if r.coordinator == nil {
		return nil, runtimeError("native input has not started")
	}
	if _, failure := r.native.quiesce(r.coordinator, deadline); failure != nil {
		return nil, failure.asError()
	}
	reply, failure := r.coordinator.inspectReceived(context.Background(), r.conn, input, r.native.controller, r.native.transport, deadline)
	if failure != nil {
		return nil, failure.asError()
	}
	return reply, nil
}

func (r *runnerLoop) submit(request inputControlRequest, admittedAt time.Time, clientID uint64) (opt[inputControlReply], *cafleetError) {
	respond, responds := request.Operation.(permissionRespondOperation)
	reply, err := r.submitWithinDeadline(request, responds, admittedAt, clientID)
	if err == nil {
		return reply, nil
	}
	if responds {
		scope := inputContext{runID: some(r.run.runID), interactionID: some(respond.input.InteractionID)}
		return none[inputControlReply](), scope.failureOf(err).asError()
	}
	return none[inputControlReply](), cafleetErrorOf(err)
}

func (r *runnerLoop) submitWithinDeadline(request inputControlRequest, responds bool, admittedAt time.Time, clientID uint64) (opt[inputControlReply], error) {
	deadline, err := inputDeadline(request, admittedAt, r.native.clock)
	if err != nil {
		return none[inputControlReply](), err
	}
	coordinator := r.coordinator
	if coordinator == nil {
		return none[inputControlReply](), runtimeError("native input has not started")
	}
	if responds {
		refusal, failure := coordinator.answeredEpisodeRefusal(request)
		if failure != nil {
			return none[inputControlReply](), failure.asError()
		}
		if refusal.set {
			return refusal, nil
		}
		admission, failure := r.native.quiesce(coordinator, deadline)
		if failure != nil {
			return none[inputControlReply](), failure.asError()
		}
		if admission == inputDrainPending {
			blocker, named := r.native.transport.drainBlocker.get()
			if !named {
				panic("a Pending drain names its reason")
			}
			return none[inputControlReply](), runtimeError("permission request is blocked: " + blocker)
		}
	}
	reply, failure := coordinator.handleReceived(context.Background(), r.conn, request, r.native.controller, deadline)
	if failure != nil {
		return none[inputControlReply](), failure.asError()
	}
	if !reply.set {
		if err := r.turns.admitClient(clientID, deadline); err != nil {
			return none[inputControlReply](), err
		}
	}
	return reply, nil
}

// liveShutdown is the live runner's shutdownHost.
type liveShutdown struct {
	pty         *ptySession
	transport   *transport
	input       *nonblockingFile
	output      *nonblockingFile
	raw         *rawTerminalGuard
	writer      *eventWriter
	deadlines   shutdownDeadlines
	controller  *observationController
	shutdownClk clock
	coordinator *inputCoordinator
}

func (l *liveShutdown) pollShutdown() error {
	snapshot := l.transport.snapshot()
	_, err := l.controller.poll(&snapshot, l.writer, l.shutdownClk)
	return err
}

func (l *liveShutdown) sendExit() error {
	exit, exited, err := l.pty.tryWait()
	if err != nil || !exited {
		return err
	}
	snapshot := l.transport.snapshot()
	return l.controller.childExited(exit, &snapshot, l.writer, l.shutdownClk)
}

func (l *liveShutdown) clock() clock {
	return l.shutdownClk
}

func (l *liveShutdown) gateProgrammaticInput() {
	l.writer.setDeadline(l.deadlines.storageDeadline())
}

func (l *liveShutdown) termVerifiedDescendants() error {
	return l.pty.signalVerified(syscall.SIGTERM)
}

func (l *liveShutdown) drainAndReapUntil(deadline time.Time) (opt[childExit], error) {
	for l.shutdownClk.now().Before(deadline) {
		tick := func(t *transport) (transportProgress, error) {
			return t.tick(l.input, l.pty, l.output, l.controller.observer(l.writer, l.shutdownClk))
		}
		var progress transportProgress
		if l.coordinator != nil {
			var failure *inputFailure
			if progress, failure = progressWithManual(l.coordinator, l.transport, tick); failure != nil {
				return none[childExit](), failure.asError()
			}
		} else {
			var err error
			if progress, err = tick(l.transport); err != nil {
				return none[childExit](), err
			}
		}
		if err := l.pollShutdown(); err != nil {
			return none[childExit](), err
		}
		if err := l.sendExit(); err != nil {
			return none[childExit](), err
		}
		survivors, err := l.pty.survivors()
		if err != nil {
			return none[childExit](), err
		}
		if !survivors && progress.childEOF {
			break
		}
		napUntil(l.shutdownClk, deadline)
	}
	return l.finalNativeExit()
}

func (l *liveShutdown) killVerifiedSurvivors() error {
	return l.pty.signalVerified(syscall.SIGKILL)
}

func (l *liveShutdown) closePty() error {
	if err := l.pty.close(); err != nil {
		return err
	}
	for l.shutdownClk.now().Before(l.deadlines.storageDeadline()) {
		_, exited, err := l.pty.tryWait()
		if err != nil {
			return err
		}
		survivors, err := l.pty.survivors()
		if err != nil {
			return err
		}
		if (exited || !l.pty.hasChild()) && !survivors {
			break
		}
		napUntil(l.shutdownClk, l.deadlines.storageDeadline())
	}
	if err := l.sendExit(); err != nil {
		return err
	}
	if err := ensure(l.transport.pendingInputBytes() == 0, "human input tail remains undelivered after native termination"); err != nil {
		return err
	}
	survivors, err := l.pty.survivors()
	if err != nil {
		return err
	}
	return ensure(!survivors, "owned descendants remain unconfirmed")
}

func (l *liveShutdown) flushDisplayUntil(deadline time.Time) (uint64, error) {
	for l.transport.pendingDisplayBytes() > 0 && l.shutdownClk.now().Before(deadline) {
		if _, err := l.transport.flushDisplay(l.output); err != nil {
			return 0, err
		}
		napUntil(l.shutdownClk, deadline)
	}
	return uint64(l.transport.discardDisplayTail()), nil //nolint:gosec // a byte count is never negative
}

func (l *liveShutdown) finalNativeExit() (opt[childExit], error) {
	exit, exited, err := l.pty.tryWait()
	if err != nil || !exited {
		return none[childExit](), err
	}
	return some(exit), nil
}

func (l *liveShutdown) nativeWasSpawned() bool {
	return l.pty.hasChild()
}

func (l *liveShutdown) finishEventsUntil(deadline time.Time) (uint64, uint64, error) {
	if err := l.sendExit(); err != nil {
		return 0, 0, err
	}
	if l.deadlines.remainingStorageAt(l.shutdownClk.now()) <= 0 {
		return 0, 0, deadlineExceededError()
	}
	for l.controller.hasPending() && l.shutdownClk.now().Before(deadline) {
		if err := l.pollShutdown(); err != nil {
			return 0, 0, err
		}
		napUntil(l.shutdownClk, deadline)
	}
	pending, gap := l.writer.finish(deadline)
	pending, err := checkedAddUint64Err(pending, l.controller.pendingTransitionCount(), "pending event count overflow")
	if err != nil {
		return 0, 0, err
	}
	gap, err = checkedAddUint64Err(gap, l.controller.unsavedGapCount(), "unsaved observation count overflow")
	if err != nil {
		return 0, 0, err
	}
	return pending, gap, nil
}

func checkedAddUint64Err(a, b uint64, overflow string) (uint64, error) {
	if b > math.MaxUint64-a {
		return 0, runtimeError(overflow)
	}
	return a + b, nil
}

func (l *liveShutdown) restoreTerminal() error {
	var failures []string
	note(&failures, struct{}{}, l.output.restore())
	note(&failures, struct{}{}, l.input.restore())
	note(&failures, struct{}{}, l.raw.restore())
	if len(failures) == 0 {
		return nil
	}
	return runtimeError(strings.Join(failures, "; "))
}

type runnerDependencies struct {
	catalog          *profileCatalog
	clock            clock
	diagnostics      runnerDiagnostics
	writerHooks      writerHooks
	observationHooks observationHooks
	previewSweep     bool
}

func runnerRun(s settings, descriptorPath string) (int32, error) {
	return runService(s, descriptorPath, runnerDependencies{
		catalog:      profileCatalogBundled(),
		clock:        systemClock{},
		diagnostics:  newRunnerDiagnostics(),
		previewSweep: true,
	})
}

// runtimeServiceStarted is the input coordinator of a started runner.
func runtimeServiceStarted(coordinator *inputCoordinator) *inputCoordinator {
	if coordinator == nil {
		panic("a started runner has its input coordinator")
	}
	return coordinator
}

// runnerLoop is the runner's control loop: it accepts control clients, serves their requests
// and, once the native child runs, ticks its transport and input.
type runnerLoop struct {
	native            *nativeIo
	conn              *dbConn
	original          runRecord
	run               runRecord
	lock              *runLock
	artifacts         *runArtifacts
	descriptor        launchDescriptor
	process           processIdentity
	creator           processIdentity
	listener          int
	terminal          *os.File
	terminate         *atomic.Bool
	resize            *atomic.Bool
	catalog           *profileCatalog
	clock             clock
	handshakeDeadline time.Time
	clients           []*client
	nextClient        uint64
	creatorConnection opt[uint64]
	started           bool
	coordinator       *inputCoordinator
	turns             *inputTurns
	shutdownClient    opt[uint64]
	nativeExit        opt[childExit]
	directorSweep     *directorSweepThread
}

func (r *runnerLoop) shutdownDeadlines() shutdownDeadlines {
	return r.native.shutdownDeadlines()
}

func (r *runnerLoop) serve() (shutdownDeadlines, error) {
	for {
		if r.terminate.Load() {
			return r.shutdownDeadlines(), nil
		}
		if !r.started {
			if err := runtimeCheckDeadline(r.native.clock.now(), r.handshakeDeadline); err != nil {
				return shutdownDeadlines{}, err
			}
			matches, err := r.creator.matches()
			if err != nil {
				return shutdownDeadlines{}, err
			}
			if err := ensure(matches, "creator disconnected before native start"); err != nil {
				return shutdownDeadlines{}, err
			}
		}
		if err := r.accept(); err != nil {
			return shutdownDeadlines{}, err
		}
		for index := range r.clients {
			deadlines, err := r.serveClient(index)
			if err != nil {
				return shutdownDeadlines{}, err
			}
			if ended, set := deadlines.get(); set {
				return ended, nil
			}
		}
		r.clients = slices.DeleteFunc(r.clients, func(c *client) bool {
			if c.closed {
				closeControlSocket(c.fd)
			}
			return c.closed
		})
		var progress transportProgress
		if r.started {
			deadlines, ticked, err := r.tickRunning()
			if err != nil {
				return shutdownDeadlines{}, err
			}
			if ended, set := deadlines.get(); set {
				return ended, nil
			}
			progress = ticked
		}
		r.native.clock.sleep(tickSleep(progress, r.native.controller.status().nextDeadline, r.native.clock.now()))
	}
}

func (r *runnerLoop) accept() error {
	fd, err := acceptControlClient(r.listener)
	if err != nil || fd < 0 {
		return err
	}
	if len(r.clients) >= 32 {
		closeControlSocket(fd)
		return nil
	}
	r.clients = append(r.clients, &client{id: r.nextClient, fd: fd, decoder: newControlDecoder()})
	if r.nextClient == math.MaxUint64 {
		return runtimeError("control connection sequence exhausted")
	}
	r.nextClient++
	return nil
}

func (r *runnerLoop) creatorLost(clientID uint64) bool {
	return r.creatorConnection == some(clientID) && !r.started
}

func (r *runnerLoop) serveClient(index int) (opt[shutdownDeadlines], error) {
	current := r.clients[index]
	clientID := current.id
	requests, err := current.poll(r.native.clock)
	if err != nil {
		current.closed = true
		requests = nil
	}
	if current.closed && r.creatorLost(clientID) {
		return none[shutdownDeadlines](), runtimeError("creator control disconnected before start")
	}
	for _, frame := range requests {
		decoded, err := decodeServiceRequest(frame.body)
		if err != nil {
			if !current.answer(nil, cafleetErrorOf(err)) {
				break
			}
			continue
		}
		var lifecycle lifecycleRequest
		switch decoded := decoded.(type) {
		case inputServiceRequest:
			if !r.serveInput(index, decoded.request, frame.admittedAt) {
				return none[shutdownDeadlines](), nil
			}
			continue
		case lifecycleServiceRequest:
			lifecycle = decoded.request
		default:
			panic("invalid serviceControlRequest")
		}
		acceptedStartup := false
		result, err := r.serveLifecycle(clientID, lifecycle, &acceptedStartup)
		if r.shutdownClient == some(clientID) {
			return some(r.shutdownDeadlines()), nil
		}
		var cause *cafleetError
		if err != nil {
			cause = cafleetErrorOf(err)
		}
		current.answer(result, cause)
		if cause != nil && acceptedStartup {
			return none[shutdownDeadlines](), runtimeError("native startup failed; owned compensation required: " + cause.message())
		}
		if current.closed {
			if r.creatorLost(clientID) {
				return none[shutdownDeadlines](), runtimeError("creator control disconnected before start")
			}
			break
		}
	}
	return none[shutdownDeadlines](), nil
}

// serveInput serves one input request; false when its reply could not be queued.
func (r *runnerLoop) serveInput(index int, request inputControlRequest, admittedAt time.Time) bool {
	current := r.clients[index]
	if _, inspects := request.Operation.(permissionInspectOperation); inspects {
		reply, cause := r.inspect(request, admittedAt)
		return current.answerInput(reply, cause)
	}
	reply, cause := r.submit(request, admittedAt, current.id)
	if cause != nil {
		return current.answerInput(nil, cause)
	}
	if completed, set := reply.get(); set {
		return current.answerInput(completed, nil)
	}
	return true
}

func (r *runnerLoop) serveLifecycle(clientID uint64, request lifecycleRequest, acceptedStartup *bool) (jsontext.Value, error) {
	switch request := request.(type) {
	case lifecyclePrepare:
		return r.prepare(clientID, request.Identity, request.RemainingMS, acceptedStartup)
	case lifecycleStart:
		if err := request.Identity.validate(r.run); err != nil {
			return nil, err
		}
		return r.start(clientID, request.PaneID, acceptedStartup)
	case lifecycleHealth:
		if err := request.Identity.validate(r.run); err != nil {
			return nil, err
		}
		return r.health()
	case lifecycleShutdown:
		if err := request.Identity.validate(r.run); err != nil {
			return nil, err
		}
		r.shutdownClient = some(clientID)
		return jsonNull, nil
	default:
		panic("invalid lifecycleRequest")
	}
}

func (r *runnerLoop) prepare(clientID uint64, identity controlIdentity, remaining uint64, acceptedStartup *bool) (jsontext.Value, error) {
	if err := ensure(!r.started && !r.creatorConnection.set, "preparation is single-use"); err != nil {
		return nil, err
	}
	if err := identity.validate(r.original); err != nil {
		return nil, err
	}
	if err := ensure(remaining > 0 && remaining <= 10000, "invalid handshake budget"); err != nil {
		return nil, err
	}
	if limit := r.native.clock.now().Add(time.Duration(remaining) * time.Millisecond); limit.Before(r.handshakeDeadline) { //nolint:gosec // remaining is at most 10000
		r.handshakeDeadline = limit
	}
	r.creatorConnection = some(clientID)
	*acceptedStartup = true
	if err := r.artifacts.removeDescriptor(); err != nil {
		return nil, err
	}
	return jsonValue(prepared{Identity: controlIdentityFromRun(r.run), Runner: r.process}), nil
}

func (r *runnerLoop) start(clientID uint64, pane string, acceptedStartup *bool) (jsontext.Value, error) {
	ctx := context.Background()
	if err := ensure(!r.started && r.creatorConnection == some(clientID), "start requires prepared creator connection"); err != nil {
		return nil, err
	}
	*acceptedStartup = true
	if _, err := withDatabaseDeadline(ctx, r.conn, r.handshakeDeadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := r.lock.validate(ctx, conn, r.run.owner()); err != nil {
			return struct{}{}, err
		}
		current, err := getRun(ctx, conn, r.run.runID)
		if err != nil {
			return struct{}{}, err
		}
		if err := ensure(current.status == runStatusPrepared, "start requires published prepared state"); err != nil {
			return struct{}{}, err
		}
		member, err := getMember(ctx, conn, r.run.memberID, r.run.fleetID)
		if err != nil {
			return struct{}{}, err
		}
		record, found := member.get()
		if !found {
			return struct{}{}, runtimeError("prepared member became inactive")
		}
		return struct{}{}, ensure(record.placement.muxPaneID == some(pane), "start placement changed")
	}); err != nil {
		return nil, err
	}
	child, err := r.native.pty.spawn(r.descriptor.Spawn)
	if err != nil {
		return nil, err
	}
	if _, err := withDatabaseDeadline(ctx, r.conn, r.handshakeDeadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := r.lock.validate(ctx, conn, r.run.owner()); err != nil {
			return struct{}{}, err
		}
		changed, err := conn.execute(
			"UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3,last_health_at=?3 WHERE run_id=?4 AND owner_epoch=?5 AND status='prepared'",
			child.pid, child.startIdentity, wallStamp(r.native.clock), r.run.runID, r.run.ownerEpoch,
		)
		if err != nil {
			return struct{}{}, err
		}
		return struct{}{}, ensure(changed == 1, "native spawn storage lost ownership")
	}); err != nil {
		return nil, err
	}
	run, err := getRun(ctx, r.conn, r.run.runID)
	if err != nil {
		return nil, err
	}
	r.run = run
	r.started = true
	screen := r.native.transport.snapshot()
	if err := r.native.controller.start(r.run, &screen, r.native.writer, r.native.clock); err != nil {
		return nil, err
	}
	coordinator, err := newInputCoordinator(ctx, r.conn, r.run, r.lock, newInputServices(r.catalog, systemRequestTokens{}, r.clock), r.handshakeDeadline)
	if err != nil {
		return nil, err
	}
	coordinator.shareDiagnostics(r.native.writer.runnerDiagnostics())
	r.coordinator = coordinator
	path := r.conn.path()
	if path == "" {
		return nil, runtimeError("runner database has no file path")
	}
	r.directorSweep = directorSweepThreadStart("sqlite:///"+path, r.run, r.catalog, r.clock, r.native.writer.runnerDiagnostics())
	return jsonValue(started{Identity: controlIdentityFromRun(r.run), Runner: r.process, Child: child}), nil
}

func (r *runnerLoop) health() (jsontext.Value, error) {
	child, err := r.run.childProcess()
	if err != nil {
		return nil, err
	}
	snapshot := healthSnapshot{
		Identity:  controlIdentityFromRun(r.run),
		Runner:    r.process,
		Child:     child,
		OutputSeq: r.native.transport.snapshot().screenSeq,
	}
	if r.started {
		snapshot.Observation = some(r.native.controller.health(r.native.writer))
	}
	return jsonValue(snapshot), nil
}

// tickRunning runs one transport and input turn; set deadlines end the loop.
func (r *runnerLoop) tickRunning() (opt[shutdownDeadlines], transportProgress, error) {
	ctx := context.Background()
	if err := r.checkSelectedDatabase(); err != nil {
		return none[shutdownDeadlines](), transportProgress{}, err
	}
	native := r.native
	if r.resize.Swap(false) {
		size, err := runtimeServiceTerminalSize(r.terminal)
		if err != nil {
			return none[shutdownDeadlines](), transportProgress{}, err
		}
		if err := native.transport.resize(native.pty, size); err != nil {
			return none[shutdownDeadlines](), transportProgress{}, err
		}
		screen := native.transport.snapshot()
		observer := native.controller.observer(native.writer, native.clock)
		observer.phase = observationPhaseResize
		if err := observer.observe(&screen); err != nil {
			return none[shutdownDeadlines](), transportProgress{}, err
		}
	}
	coordinator := runtimeServiceStarted(r.coordinator)
	progress, failure := progressWithManual(coordinator, native.transport, func(t *transport) (transportProgress, error) {
		return t.tick(native.input, native.pty, native.output, native.controller.observer(native.writer, native.clock))
	})
	if failure != nil {
		return none[shutdownDeadlines](), transportProgress{}, failure.asError()
	}
	screen := native.transport.snapshot()
	if _, err := native.controller.poll(&screen, native.writer, native.clock); err != nil {
		return none[shutdownDeadlines](), transportProgress{}, err
	}
	if progress.outerEOF {
		return some(r.shutdownDeadlines()), progress, nil
	}
	exited, err := r.observeExit()
	if err != nil || exited.set {
		return exited, progress, err
	}
	io := &loopIo{native: native, terminate: r.terminate, resize: r.resize, nativeExit: &r.nativeExit, runID: r.run.runID}
	outcome, err := r.turns.turn(ctx, coordinator, r.conn, r.run, io)
	if err != nil {
		return none[shutdownDeadlines](), transportProgress{}, err
	}
	switch outcome := outcome.(type) {
	case inputTurnContinue:
	case inputTurnShutdown:
		return some(outcome.deadlines), progress, nil
	case inputTurnReply:
		for _, candidate := range r.clients {
			if candidate.id == outcome.client && !candidate.closed {
				candidate.answerInput(outcome.reply, outcome.cause)
				break
			}
		}
	default:
		panic("invalid inputTurnOutcome")
	}
	return none[shutdownDeadlines](), progress, nil
}

// checkSelectedDatabase fences input once the selected database file is replaced.
func (r *runnerLoop) checkSelectedDatabase() error {
	selected, isString := jsonString(jsonIndex(r.run.databaseIdentityJSON, "canonical_path"))
	if !isString {
		panic("validated database path")
	}
	var meta unix.Stat_t
	if err := unix.Stat(selected, &meta); err != nil {
		return ioError(ioErrorText(err))
	}
	device, hasDevice := jsonUint64(jsonIndex(r.run.databaseIdentityJSON, "device"))
	inode, hasInode := jsonUint64(jsonIndex(r.run.databaseIdentityJSON, "inode"))
	return ensure(
		hasDevice && uint64(meta.Dev) == device && hasInode && meta.Ino == inode, //nolint:gosec,unconvert // st_dev is signed on darwin
		"selected database was replaced; input fenced",
	)
}

func (r *runnerLoop) observeExit() (opt[shutdownDeadlines], error) {
	coordinator := runtimeServiceStarted(r.coordinator)
	conn, runID, c := r.conn, r.run.runID, r.native.clock
	return r.native.observeExit(&r.nativeExit, func(controller *observationController) error {
		return publishStoppedResults(context.Background(), coordinator, conn, runID, controller, c)
	})
}

// loopIo is the live runner's I/O for its input turns.
type loopIo struct {
	native     *nativeIo
	terminate  *atomic.Bool
	resize     *atomic.Bool
	nativeExit *opt[childExit]
	runID      int64
}

func (l *loopIo) clock() clock {
	return l.native.clock
}

func (l *loopIo) controller() *observationController {
	return l.native.controller
}

func (l *loopIo) quiesce(coordinator *inputCoordinator, deadline time.Time) (inputDrain, *inputFailure) {
	return l.native.quiesce(coordinator, deadline)
}

func (l *loopIo) advance(ctx context.Context, coordinator *inputCoordinator, conn *dbConn, deadline time.Time) (opt[inputControlReply], error) {
	reply, failure := coordinator.advance(ctx, conn, l.native.transport, l.native.pty, l.native.controller, deadline)
	if failure != nil {
		return none[inputControlReply](), failure.asError()
	}
	return reply, nil
}

func (l *loopIo) terminateRequested() bool {
	return l.terminate.Load()
}

func (l *loopIo) resizeRequested() bool {
	return l.resize.Load()
}

func (l *loopIo) shutdownDeadlines() shutdownDeadlines {
	return l.native.shutdownDeadlines()
}

func (l *loopIo) observeExit(ctx context.Context, coordinator *inputCoordinator, conn *dbConn) (opt[shutdownDeadlines], error) {
	c, runID := l.native.clock, l.runID
	return l.native.observeExit(l.nativeExit, func(controller *observationController) error {
		return publishStoppedResults(ctx, coordinator, conn, runID, controller, c)
	})
}

// bindControlListener binds the run's private control socket, listening and nonblocking.
func bindControlListener(path string) (int, error) {
	fd, err := controlSocket()
	if err != nil {
		return -1, runtimeError("cannot bind private runtime control socket: " + ioErrorText(err))
	}
	if err := unix.Bind(fd, &unix.SockaddrUnix{Name: path}); err != nil {
		closeControlSocket(fd)
		return -1, runtimeError("cannot bind private runtime control socket: " + ioErrorText(err))
	}
	if err := unix.Listen(fd, 128); err != nil {
		closeControlSocket(fd)
		return -1, runtimeError("cannot bind private runtime control socket: " + ioErrorText(err))
	}
	if err := os.Chmod(path, 0o600); err != nil {
		closeControlSocket(fd)
		return -1, ioError(ioErrorText(err))
	}
	return fd, nil
}

// acceptControlClient accepts one pending connection as a nonblocking close-on-exec socket; -1
// when none is pending.
func acceptControlClient(listener int) (int, error) {
	fd, err := acceptRetryingInterrupts(listener)
	switch {
	case errors.Is(err, unix.EAGAIN):
		return -1, nil
	case err != nil:
		return -1, ioError(ioErrorText(err))
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		closeControlSocket(fd)
		return -1, ioError(ioErrorText(err))
	}
	return fd, nil
}

func acceptRetryingInterrupts(listener int) (int, error) {
	for {
		syscall.ForkLock.RLock()
		fd, _, err := unix.Accept(listener)
		if err == nil {
			unix.CloseOnExec(fd)
		}
		syscall.ForkLock.RUnlock()
		if !errors.Is(err, unix.EINTR) {
			return fd, err
		}
	}
}

func runService(s settings, descriptorPath string, services runnerDependencies) (code int32, err error) {
	ctx := context.Background()
	c := services.clock
	catalog := services.catalog
	conn, err := connect(s.databaseURL)
	if err != nil {
		return 0, err
	}
	defer func() { err = mergeCleanup(err, conn.close()) }()
	if err := classifySelectedSchema(conn, s.databaseURL).requireCurrent(); err != nil {
		return 0, err
	}
	descriptor, err := loadDescriptor(ctx, conn, descriptorPath)
	if err != nil {
		return 0, err
	}
	original, err := getRun(ctx, conn, descriptor.Identity.RunID)
	if err != nil {
		return 0, err
	}
	creator := original.ownerProcess()
	creatorMatches, err := creator.matches()
	if err != nil {
		return 0, err
	}
	if err := ensure(creatorMatches, "launch creator process identity changed"); err != nil {
		return 0, err
	}
	acquired, err := runLockAcquire(ctx, conn, original.owner())
	if err != nil {
		return 0, err
	}
	lock, held := acquired.get()
	if !held {
		return 0, runtimeError("run ownership lock is held")
	}
	defer reportClose(lock.close)
	process, err := processIdentityCurrent()
	if err != nil {
		return 0, err
	}
	handshakeDeadline := c.now().Add(10 * time.Second)
	owner, err := lock.transferToRunnerWithDeadline(ctx, conn, original.owner(), process, handshakeDeadline, c)
	if err != nil {
		return 0, err
	}
	run, err := getRun(ctx, conn, owner.runID)
	if err != nil {
		return 0, err
	}
	if _, err := withDatabaseDeadline(ctx, conn, handshakeDeadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, validateFreshStartup(ctx, conn, lock, owner, run)
	}); err != nil {
		return 0, err
	}
	artifacts, err := runArtifactsFromRun(ctx, conn, run)
	if err != nil {
		return 0, err
	}
	defer artifacts.close()
	terminal, err := duplicateStdin()
	if err != nil {
		return 0, err
	}
	defer reportClose(terminal.Close)
	initialSize, err := runtimeServiceTerminalSize(terminal)
	if err != nil {
		return 0, err
	}
	pty, err := ptySessionPrepare(initialSize, c)
	if err != nil {
		return 0, err
	}
	defer pty.finish()
	raw, err := rawTerminalGuardEnter(terminal)
	if err != nil {
		return 0, err
	}
	defer raw.close()
	input, err := newNonblockingFile(os.Stdin)
	if err != nil {
		return 0, err
	}
	defer input.close()
	output, err := newNonblockingFile(os.Stdout)
	if err != nil {
		return 0, err
	}
	defer output.close()
	transport, err := newTransport(initialSize, transportLimits{inputBytes: 65536, displayBytes: 262144, bytesPerTick: 16384}, c)
	if err != nil {
		return 0, err
	}
	listener, err := bindControlListener(artifacts.paths.controlSocket)
	if err != nil {
		return 0, err
	}
	defer closeControlSocket(listener)
	controller, err := newObservationController(run, catalog)
	if err != nil {
		return 0, err
	}
	controller.shareDiagnostics(services.diagnostics)
	writer, err := eventWriterStartWorker(s.databaseURL, run, lock, c, services.diagnostics, services.writerHooks)
	if err != nil {
		return 0, err
	}
	defer writer.close()
	terminate, resize := &atomic.Bool{}, &atomic.Bool{}
	signals := newSignalGuards(terminate, resize)
	defer signals.close()
	turns := newInputTurns()
	turns.previewSweep.disabled = !services.previewSweep
	runner := &runnerLoop{
		native: &nativeIo{
			input:      input,
			pty:        pty,
			output:     output,
			transport:  transport,
			controller: controller,
			writer:     writer,
			clock:      c,
		},
		conn:              conn,
		original:          original,
		run:               run,
		lock:              lock,
		artifacts:         artifacts,
		descriptor:        descriptor,
		process:           process,
		creator:           creator,
		listener:          listener,
		terminal:          terminal,
		terminate:         terminate,
		resize:            resize,
		catalog:           catalog,
		clock:             c,
		handshakeDeadline: handshakeDeadline,
		nextClient:        1,
		turns:             turns,
	}
	defer func() {
		for _, open := range runner.clients {
			closeControlSocket(open.fd)
		}
	}()
	deadlines, unwind, loopErr := serveRecovering(runner)
	var failure opt[string]
	if loopErr != nil {
		deadlines = shutdownDeadlinesStartingNow(c)
		message := cafleetErrorOf(loopErr).message()
		if runner.started {
			if _, degraded := withDatabaseDeadline(ctx, conn, deadlines.storageDeadline(), func(ctx context.Context, conn *dbConn) (struct{}, error) {
				if err := lock.validate(ctx, conn, owner); err != nil {
					return struct{}{}, err
				}
				_, err := conn.execute(
					"UPDATE member_runs SET status='degraded',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='running' AND child_pid IS NOT NULL",
					wallStamp(c), runner.run.runID, owner.ownerEpoch,
				)
				return struct{}{}, err
			}); degraded != nil {
				message += "; degraded status: " + cafleetErrorOf(degraded).message()
			}
		}
		failure = some(message)
	}
	if runner.directorSweep != nil {
		runner.directorSweep.stopAndJoin(deadlines.childDeadline())
	}
	if coordinator := runner.coordinator; coordinator != nil {
		_, fenced := coordinator.fence(ctx, deadlines.storageDeadline())
		coordinator.finishPendingAttempt()
		unpublished, published := withDatabaseDeadline(ctx, conn, deadlines.storageDeadline(), func(ctx context.Context, conn *dbConn) ([]unpublishedDeliveryResult, error) {
			return unpublishedDeliveryResults(ctx, conn, runner.run.runID)
		})
		if published == nil {
			for _, result := range unpublished {
				controller.publishDeliveryResult(inputDeliveryResultCause(result.inputID, result.stage, result.stopCause))
			}
		}
		for _, step := range []struct {
			name string
			err  error
		}{{"input fence", fenced}, {"result publication", published}} {
			if step.err == nil {
				continue
			}
			message := step.name + ": " + cafleetErrorOf(step.err).message()
			if prior, set := failure.get(); set {
				failure = some(prior + "; " + message)
			} else {
				failure = some(message)
			}
		}
	}
	deadlines = pty.beginShutdown(deadlines)
	live := &liveShutdown{
		pty:         pty,
		transport:   transport,
		input:       input,
		output:      output,
		raw:         raw,
		writer:      writer,
		deadlines:   deadlines,
		controller:  controller,
		shutdownClk: c,
		coordinator: runner.coordinator,
	}
	outcome := shutdown(live, deadlines, runner.nativeExit)
	if cause, set := failure.get(); set {
		outcome.fail(cause)
	}
	if cause := controller.pendingError(); cause != nil {
		outcome.fail(cause.message())
	}
	if cause, set := writer.diagnostic().get(); set {
		outcome.fail(cause)
	}
	diagnosticsAt := wallStamp(c)
	entries, dropped := shutdownDiagnostics(services.diagnostics, outcome, diagnosticsAt)
	if _, persisted := withDatabaseDeadline(ctx, conn, deadlines.storageDeadline(), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, persistCleanup(ctx, conn, lock, owner, runner.run.runID, outcome, entries, dropped, diagnosticsAt, c)
	}); persisted != nil {
		outcome.fail("cleanup evidence pending: " + cafleetErrorOf(persisted).message())
	}
	if id, set := runner.shutdownClient.get(); set {
		answerShutdown(runner.clients, id, outcome, deadlines, c)
	}
	for _, diagnostic := range outcome.diagnostics {
		fmt.Fprintln(os.Stderr, "member runtime: "+diagnostic)
	}
	if unwind != nil {
		panic(unwind)
	}
	if !runner.started {
		return 1, nil
	}
	return runnerExitStatus(outcome.nativeExit, outcome.cleanupState == cleanupStateComplete)
}

// serveRecovering runs the loop, turning a panic into the loop failure and its recovered
// payload, which the caller re-panics after shutdown.
func serveRecovering(runner *runnerLoop) (deadlines shutdownDeadlines, unwind any, err error) {
	defer func() {
		if payload := recover(); payload != nil {
			unwind, err = payload, runtimeError("runner loop unwound")
		}
	}()
	deadlines, err = runner.serve()
	return deadlines, nil, err
}

func validateFreshStartup(ctx context.Context, conn *dbConn, lock *runLock, owner runOwner, run runRecord) error {
	if err := lock.validate(ctx, conn, owner); err != nil {
		return err
	}
	if err := ensure(
		run.status == runStatusStarting && !run.childPID.set && !run.childStartIdentity.set &&
			!run.startedAt.set && !run.endedAt.set && !run.recoveryOriginStatus.set,
		"live startup requires fresh run",
	); err != nil {
		return err
	}
	history, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM member_interactions WHERE run_id=?1) OR EXISTS(SELECT 1 FROM member_runtime_events WHERE run_id=?1)",
		[]any{run.runID}, func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil {
		return err
	}
	return ensure(!history, "live startup refuses durable history")
}

func persistCleanup(ctx context.Context, conn *dbConn, lock *runLock, owner runOwner, runID int64, outcome shutdownOutcome, entries []runnerDiagnosticEntry, dropped uint64, at string, c clock) error {
	if err := lock.validate(ctx, conn, owner); err != nil {
		return err
	}
	_, err := inTransaction(conn, transactionImmediate, func() (struct{}, error) {
		diagnostics := outcome.diagnostics
		if diagnostics == nil {
			diagnostics = []string{}
		}
		if _, err := conn.execute(
			"UPDATE member_runs SET cleanup_state=?1,cleanup_diagnostics_json=?2,last_health_at=?3,status=CASE WHEN child_pid IS NULL THEN 'failed' ELSE status END,ended_at=CASE WHEN child_pid IS NULL THEN ?3 ELSE ended_at END WHERE run_id=?4 AND owner_epoch=?5",
			outcome.cleanupState.asStr(), string(jsonValue(diagnostics)), wallStamp(c), runID, owner.ownerEpoch,
		); err != nil {
			return struct{}{}, err
		}
		return struct{}{}, writeRunnerDiagnostics(conn, runID, entries, dropped, at)
	})
	return err
}

// answerShutdown answers the shutdown requester with the outcome and flushes it within the
// storage deadline.
func answerShutdown(clients []*client, id uint64, outcome shutdownOutcome, deadlines shutdownDeadlines, c clock) {
	index := slices.IndexFunc(clients, func(candidate *client) bool { return candidate.id == id })
	if index < 0 {
		return
	}
	requester := clients[index]
	requester.answer(jsonValue(outcome), nil)
	for len(requester.output) > 0 && c.now().Before(deadlines.storageDeadline()) {
		if requester.closed || requester.flush() != nil {
			requester.closed = true
			break
		}
		c.sleep(time.Millisecond)
	}
}

func duplicateStdin() (*os.File, error) {
	fd, err := unix.FcntlInt(os.Stdin.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, ioError(errnoText(err))
	}
	return os.NewFile(uintptr(fd), "/dev/stdin"), nil
}

// reportClose runs a deferred release and reports its failure on stderr, as a Rust Drop does.
func reportClose(release func() error) {
	if err := release(); err != nil {
		fmt.Fprintln(os.Stderr, "member runtime: "+err.Error())
	}
}
