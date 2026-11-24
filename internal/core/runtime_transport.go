package core

import (
	"bytes"
	"errors"
	"fmt"
	"math"
	"os"
	"syscall"
	"time"

	"github.com/himkt/cafleet/internal/vt"
	"golang.org/x/sys/unix"
)

type inputDrain uint8

const (
	inputDrainPending inputDrain = iota + 1
	inputDrainQuiescent
)

// inputInstance identifies one transport for drain proofs, as Rust's Arc<()> pointer identity.
type inputInstance struct{ _ byte }

// drainedInput proves the transport was quiescent at one turn. Rust's human_generation() and
// output_seq() accessors are its fields, which the package reads directly.
type drainedInput struct {
	instance          *inputInstance
	turn              uint64
	humanGeneration   uint64
	pointerGeneration uint64
	outputSeq         uint64
	health            observationHealth
}

//sumtype:decl
type programmaticWrite interface{ isProgrammaticWrite() }

type programmaticWriteComplete struct{ bytes int }

type programmaticWritePartial struct{ bytes int }

type programmaticWritePending struct{}

func (programmaticWriteComplete) isProgrammaticWrite() {}
func (programmaticWritePartial) isProgrammaticWrite()  {}
func (programmaticWritePending) isProgrammaticWrite()  {}

// byteInput is a nonblocking reader; its errors are raw errno values, so EAGAIN is Rust's
// WouldBlock and EINTR its Interrupted.
type byteInput interface {
	tryRead(bytes []byte) (int, error)
}

type ptyIo interface {
	byteInput
	tryWrite(bytes []byte) (int, error)
	resize(size terminalSize) error
	close() error
}

type display interface {
	tryWrite(bytes []byte) (int, error)
	tryFlush() error
}

// screenObserver receives the screen after every observation step and reports input health.
type screenObserver interface {
	observe(screen *screenSnapshot) error
	inputHealth() observationHealth
	parserFailure() error
}

type transportProgress struct {
	childEOF       bool
	outerEOF       bool
	humanBytes     int
	forwardedBytes int
}

type transportLimits struct {
	inputBytes   int
	displayBytes int
	bytesPerTick int
}

func wouldBlockOrInterrupted(err error) bool {
	return errors.Is(err, syscall.EAGAIN) || errors.Is(err, syscall.EINTR)
}

func transportIoError(err error) *cafleetError {
	return ioError(ioErrorText(err))
}

// transport owns the PTY's two byte queues, the terminal model and the input drain proof; one
// runner goroutine owns it.
type transport struct {
	terminal          *terminalState
	limits            transportLimits
	input             []byte
	display           []byte
	outerEOF          bool
	childEOF          bool
	humanGeneration   uint64
	pointerGeneration uint64
	replies           replyClassifier
	inputTurn         uint64
	inputInstance     *inputInstance
	inputDrain        *drainedInput
	drainBlocker      opt[string]
	observationFenced bool
	// observationRemainder holds unobserved child bytes as a string; some("") re-observes the
	// current screen.
	observationRemainder opt[string]
	clock                clock
	observationHooks     observationHooks
	observationPhase     observationPhase
	// deferredChildBytes is a test prefetch of child output; nil outside tests.
	deferredChildBytes []byte
}

// availableRead is one nonblocking read; available is its bytes, empty at EOF, and false when
// nothing was read.
//
//sumtype:decl
type availableRead interface {
	isAvailableRead()
	available() ([]byte, bool)
}

type availableReadSkipped struct{}

type availableReadWouldBlock struct{}

type availableReadInterrupted struct{}

type availableReadEOF struct{}

type availableReadData struct{ bytes []byte }

func (availableReadSkipped) isAvailableRead()     {}
func (availableReadWouldBlock) isAvailableRead()  {}
func (availableReadInterrupted) isAvailableRead() {}
func (availableReadEOF) isAvailableRead()         {}
func (availableReadData) isAvailableRead()        {}

func (availableReadSkipped) available() ([]byte, bool)     { return nil, false }
func (availableReadWouldBlock) available() ([]byte, bool)  { return nil, false }
func (availableReadInterrupted) available() ([]byte, bool) { return nil, false }
func (availableReadEOF) available() ([]byte, bool)         { return []byte{}, true }
func (r availableReadData) available() ([]byte, bool)      { return r.bytes, true }

func readClassified(input byteInput, capacity int) (availableRead, error) {
	if capacity == 0 {
		return availableReadSkipped{}, nil
	}
	bytes := make([]byte, min(capacity, 16384))
	count, err := input.tryRead(bytes)
	switch {
	case err == nil:
		if count < 0 || count > len(bytes) {
			panic("reader respects supplied capacity")
		}
		if count == 0 {
			return availableReadEOF{}, nil
		}
		return availableReadData{bytes: bytes[:count]}, nil
	case errors.Is(err, syscall.EAGAIN):
		return availableReadWouldBlock{}, nil
	case errors.Is(err, syscall.EINTR):
		return availableReadInterrupted{}, nil
	default:
		return nil, err
	}
}

func readAvailable(input byteInput, capacity int) ([]byte, bool, error) {
	read, err := readClassified(input, capacity)
	if err != nil {
		return nil, false, err
	}
	bytes, ok := read.available()
	return bytes, ok, nil
}

func writePending(queue *[]byte, limit int, write func([]byte) (int, error)) (int, error) {
	if len(*queue) == 0 {
		return 0, nil
	}
	offered := (*queue)[:min(len(*queue), limit)]
	count, err := write(offered)
	switch {
	case err == nil && count == 0:
		return 0, runtimeError("terminal write made no progress")
	case err != nil && wouldBlockOrInterrupted(err):
		count = 0
	case err != nil:
		return 0, transportIoError(err)
	}
	if err := ensure(count <= len(offered), "terminal writer exceeded supplied bytes"); err != nil {
		return 0, err
	}
	*queue = (*queue)[count:]
	return count, nil
}

func newTransport(size terminalSize, limits transportLimits, c clock) (*transport, error) {
	if err := ensure(limits.inputBytes > 0 && limits.displayBytes > 0 && limits.bytesPerTick > 0, "transport limits must be positive"); err != nil {
		return nil, err
	}
	terminal, err := newTerminalState(size)
	if err != nil {
		return nil, err
	}
	return &transport{
		terminal:             terminal,
		limits:               limits,
		inputInstance:        &inputInstance{},
		drainBlocker:         none[string](),
		observationRemainder: none[string](),
		clock:                c,
		observationPhase:     observationPhaseChildOutput,
	}, nil
}

func (t *transport) tick(human byteInput, pty ptyIo, out display, observer screenObserver) (transportProgress, error) {
	t.invalidateInputDrain()
	humanBytes, err := t.drainOuter(human)
	if err != nil {
		return transportProgress{}, err
	}
	progress := transportProgress{humanBytes: humanBytes}
	if progress.forwardedBytes, err = writePending(&t.input, math.MaxInt, pty.tryWrite); err != nil {
		return transportProgress{}, err
	}
	if remainder, set := t.takeObservationRemainder(); set {
		if err := t.observeTickChild([]byte(remainder), observer); err != nil {
			return transportProgress{}, err
		}
	}
	bytes, read, err := t.readChild(pty)
	if err != nil {
		return transportProgress{}, err
	}
	if read {
		t.childEOF = len(bytes) == 0
		t.display = append(t.display, bytes...)
		if len(bytes) > 0 && t.observationHooks != nil {
			if cause := t.observationHooks.before(t.observationPhase, observationOperationParser); cause != nil {
				t.observationFenced = true
				if err := observer.parserFailure(); err != nil {
					return transportProgress{}, err
				}
				return transportProgress{}, cause
			}
		}
		if len(bytes) > 0 {
			if err := t.observeTickChild(bytes, observer); err != nil {
				return transportProgress{}, err
			}
		}
	}
	if _, err := t.flushDisplay(out); err != nil {
		return transportProgress{}, err
	}
	progress.outerEOF = t.outerEOF
	progress.childEOF = t.childEOF
	return progress, nil
}

func (t *transport) takeObservationRemainder() (string, bool) {
	remainder, set := t.observationRemainder.get()
	t.observationRemainder = none[string]()
	return remainder, set
}

func (t *transport) observeTickChild(bytes []byte, observer screenObserver) error {
	switch failure := t.observeChildBytes(bytes, observer).(type) {
	case nil:
		return nil
	case terminalObservationErrorParser:
		if err := observer.parserFailure(); err != nil {
			return err
		}
		return failure.cause
	case terminalObservationErrorObserver:
		return failure.cause
	default:
		panic("invalid terminalObservationError")
	}
}

// observeChildBytes feeds bytes to the observer. An observer failure leaves the unconsumed
// suffix of bytes as the observation remainder, which the next tick or drain feeds before any new
// child read; an empty remainder re-observes the current screen, so observation completes only
// after a successful observer callback. A parser failure has no resumable boundary and fences
// observation for the transport lifetime.
func (t *transport) observeChildBytes(bytes []byte, observer screenObserver) terminalObservationError {
	var observed terminalObservationError
	if len(bytes) == 0 {
		snapshot := t.terminal.snapshot()
		if err := observer.observe(&snapshot); err != nil {
			observed = terminalObservationErrorObserver{cause: err, consumed: 0}
		}
	} else {
		observed = t.terminal.feedObservedClassified(bytes, observer)
	}
	switch failure := observed.(type) {
	case nil:
	case terminalObservationErrorObserver:
		t.retainObservationRemainder(bytes[failure.consumed:])
	case terminalObservationErrorParser:
		t.observationFenced = true
	default:
		panic("invalid terminalObservationError")
	}
	return observed
}

func (t *transport) retainObservationRemainder(unobserved []byte) {
	if _, set := t.observationRemainder.get(); set {
		panic("an observation remainder is fed before new child output")
	}
	t.observationRemainder = some(string(unobserved))
}

func (t *transport) observationComplete() bool {
	_, remainder := t.observationRemainder.get()
	return !t.observationFenced && !remainder
}

// drainOuter reads the outer terminal until it would block. One outer-terminal read returns at
// most the line discipline's buffer, so a longer terminal reply must reach the child within the
// same tick: a child that sees only its first part may parse the rest as keys.
func (t *transport) drainOuter(human byteInput) (int, error) {
	t.expireReplyPrefix()
	forwarded := 0
	for !t.outerEOF {
		capacity := min(t.limits.inputBytes-len(t.input), t.limits.bytesPerTick)
		bytes, read, err := readAvailable(human, capacity)
		if err != nil {
			return 0, transportIoError(err)
		}
		if !read {
			break
		}
		if len(bytes) == 0 {
			t.closeOuter()
		} else {
			forwarded += len(bytes)
			t.retainHuman(bytes)
		}
	}
	return forwarded, nil
}

func (t *transport) readChild(pty ptyIo) ([]byte, bool, error) {
	if t.deferredChildBytes != nil {
		bytes := t.deferredChildBytes
		t.deferredChildBytes = nil
		return bytes, true, nil
	}
	if t.childEOF {
		return nil, false, nil
	}
	bytes, read, err := readAvailable(pty, min(t.limits.displayBytes-len(t.display), t.limits.bytesPerTick))
	if err != nil {
		return nil, false, transportIoError(err)
	}
	return bytes, read, nil
}

func (t *transport) flushDisplay(out display) (int, error) {
	t.invalidateInputDrain()
	count, err := writePending(&t.display, t.limits.bytesPerTick, out.tryWrite)
	if err != nil {
		return 0, err
	}
	if err := flushNonblocking(out); err != nil {
		return 0, err
	}
	return count, nil
}

func (t *transport) resize(pty ptyIo, size terminalSize) error {
	t.invalidateInputDrain()
	if err := size.validate(); err != nil {
		return err
	}
	if err := pty.resize(size); err != nil {
		return err
	}
	return t.terminal.resize(size)
}

func (t *transport) invalidateInputDrain() {
	t.inputDrain = nil
	if t.inputTurn == math.MaxUint64 {
		panic("input drain turn fits u64")
	}
	t.inputTurn++
}

func (t *transport) retainHuman(bytes []byte) {
	t.countInput(t.replies.feed(bytes, t.clock.now()))
	t.input = append(t.input, bytes...)
}

func (t *transport) expireReplyPrefix() {
	t.countInput(t.replies.expire(t.clock.now()))
}

func (t *transport) closeOuter() {
	t.countInput(t.replies.finish())
	t.outerEOF = true
}

func (t *transport) countInput(c classified) {
	t.humanGeneration = checkedAddUint64(t.humanGeneration, uint64(c.manual), "human input generation")        //nolint:gosec // a byte count is never negative
	t.pointerGeneration = checkedAddUint64(t.pointerGeneration, uint64(c.pointer), "pointer input generation") //nolint:gosec // a byte count is never negative
}

func (t *transport) takeInputDrain() *drainedInput {
	drain := t.inputDrain
	t.inputDrain = nil
	return drain
}

func (t *transport) validateInputDrain(proof *drainedInput, deadline time.Time) error {
	if err := t.validateOrdinary(proof, deadline); err != nil {
		return err
	}
	eligible := proof.health.Eligibility == inputEligibility(inputEligibilityComposerEmpty{}) &&
		proof.health.Visibility == visibilityNotVisible
	return ensure(eligible, "input drain is not eligible for composer input")
}

func (t *transport) validateOrdinary(proof *drainedInput, deadline time.Time) error {
	return t.validateObservationDrain(proof, deadline)
}

func (t *transport) validatePermission(proof *drainedInput, deadline time.Time) error {
	if err := t.validateOrdinary(proof, deadline); err != nil {
		return err
	}
	return ensure(t.pointerGeneration == proof.pointerGeneration, "input drain pointer binding changed")
}

func (t *transport) validateObservationDrain(proof *drainedInput, deadline time.Time) error {
	if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
		return err
	}
	bound := t.inputInstance == proof.instance && t.inputTurn == proof.turn &&
		t.humanGeneration == proof.humanGeneration && t.terminal.snapshot().screenSeq == proof.outputSeq
	if err := ensure(bound, "input drain binding changed"); err != nil {
		return err
	}
	quiescent := t.observationComplete() && !t.replies.pending() && !t.outerEOF && !t.childEOF &&
		len(t.input) == 0 && len(t.display) < t.limits.displayBytes
	if err := ensure(quiescent, "input transport is no longer quiescent"); err != nil {
		return err
	}
	if err := proof.health.validate(); err != nil {
		return err
	}
	_, diagnosed := proof.health.Diagnostic.get()
	return ensure(!proof.health.Degraded && proof.health.PendingCount == 0 && !diagnosed, "observation drain is not healthy")
}

func (t *transport) snapshot() screenSnapshot {
	return t.terminal.snapshot()
}

func (t *transport) bracketedPaste() bool {
	return t.terminal.parser.Screen().BracketedPaste()
}

func (t *transport) pendingInputBytes() int {
	return len(t.input)
}

func (t *transport) pendingDisplayBytes() int {
	return len(t.display)
}

func (t *transport) discardDisplayTail() int {
	t.invalidateInputDrain()
	if t.deferredChildBytes != nil {
		t.observationFenced = true
		t.display = append(t.display, t.deferredChildBytes...)
		t.deferredChildBytes = nil
	}
	count := len(t.display)
	t.display = nil
	return count
}

const roundLimit = 8

type drainBudget struct {
	human   int
	child   int
	manual  int
	display int
}

func unsettledHealth(health observationHealth) opt[string] {
	switch {
	case health.Degraded:
		if code, set := health.Diagnostic.get(); set {
			return some("observation degraded: " + code)
		}
		return some("observation degraded")
	case health.PendingCount != 0:
		return some("observation transitions pending")
	default:
		return none[string]()
	}
}

// deadlineObserver bounds every observer callback by deadline.
type deadlineObserver struct {
	inner    screenObserver
	deadline time.Time
	clock    clock
}

func (o *deadlineObserver) observe(screen *screenSnapshot) error {
	if err := runtimeCheckDeadline(o.clock.now(), o.deadline); err != nil {
		return err
	}
	if err := o.inner.observe(screen); err != nil {
		return err
	}
	return runtimeCheckDeadline(o.clock.now(), o.deadline)
}

func (o *deadlineObserver) inputHealth() observationHealth {
	return o.inner.inputHealth()
}

func (o *deadlineObserver) parserFailure() error {
	if err := runtimeCheckDeadline(o.clock.now(), o.deadline); err != nil {
		return err
	}
	if err := o.inner.parserFailure(); err != nil {
		return err
	}
	return runtimeCheckDeadline(o.clock.now(), o.deadline)
}

func (t *transport) quiesceForInput(human byteInput, pty ptyIo, out display, observer screenObserver, deadline time.Time) (inputDrain, error) {
	t.invalidateInputDrain()
	t.drainBlocker = none[string]()
	if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
		return 0, err
	}
	remaining := drainBudget{
		human:   t.limits.bytesPerTick,
		child:   t.limits.bytesPerTick,
		manual:  t.limits.bytesPerTick,
		display: t.limits.bytesPerTick,
	}
	for range roundLimit {
		if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
			return 0, err
		}
		t.expireReplyPrefix()
		var humanRead availableRead = availableReadSkipped{}
		if !t.outerEOF {
			read, err := readClassified(human, min(remaining.human, t.limits.inputBytes-len(t.input)))
			if err != nil {
				return 0, transportIoError(err)
			}
			humanRead = read
		}
		_, humanQuiet := humanRead.(availableReadWouldBlock)
		_, interrupted := humanRead.(availableReadInterrupted)
		switch read := humanRead.(type) {
		case availableReadData:
			remaining.human -= len(read.bytes)
			t.retainHuman(read.bytes)
		case availableReadEOF:
			t.closeOuter()
		case availableReadSkipped, availableReadWouldBlock, availableReadInterrupted:
		default:
			panic("invalid availableRead")
		}
		if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
			return 0, err
		}
		if remaining.manual > 0 && len(t.input) > 0 {
			written, err := writePending(&t.input, remaining.manual, pty.tryWrite)
			if err != nil {
				return 0, err
			}
			remaining.manual -= written
			if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
				return 0, err
			}
		}
		if remainder, set := t.takeObservationRemainder(); set {
			if len(remainder) > remaining.child {
				panic("an observation remainder fits a fresh drain's child quota")
			}
			remaining.child -= len(remainder)
			if err := t.observeDrainChild([]byte(remainder), observer, deadline); err != nil {
				return 0, err
			}
		}
		childRead, err := t.readDrainChild(pty, min(remaining.child, t.limits.displayBytes-len(t.display)))
		if err != nil {
			return 0, err
		}
		_, childQuiet := childRead.(availableReadWouldBlock)
		_, childInterrupted := childRead.(availableReadInterrupted)
		interrupted = interrupted || childInterrupted
		switch read := childRead.(type) {
		case availableReadData:
			remaining.child -= len(read.bytes)
			t.display = append(t.display, read.bytes...)
			if err := t.observeDrainChild(read.bytes, observer, deadline); err != nil {
				return 0, err
			}
		case availableReadEOF:
			t.childEOF = true
		case availableReadSkipped, availableReadWouldBlock, availableReadInterrupted:
		default:
			panic("invalid availableRead")
		}
		if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
			return 0, err
		}
		if remaining.display > 0 && len(t.display) > 0 {
			written, err := writePending(&t.display, remaining.display, out.tryWrite)
			if err != nil {
				return 0, err
			}
			remaining.display -= written
			if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
				return 0, err
			}
		}
		if err := flushNonblocking(out); err != nil {
			return 0, err
		}
		if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
			return 0, err
		}
		switch {
		case interrupted:
			return t.pending("terminal input interrupted"), nil
		case t.outerEOF:
			return t.pending("outer terminal closed"), nil
		case t.childEOF:
			return t.pending("native output closed"), nil
		}
		if humanQuiet && childQuiet && len(t.input) == 0 && len(t.display) < t.limits.displayBytes {
			if t.replies.pending() {
				return t.pending("terminal reply pending"), nil
			}
			if !t.observationComplete() {
				return t.pending("observation incomplete"), nil
			}
			health := observer.inputHealth()
			if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
				return 0, err
			}
			if reason, unsettled := unsettledHealth(health).get(); unsettled {
				return t.pending(reason), nil
			}
			outputSeq := t.terminal.snapshot().screenSeq
			if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
				return 0, err
			}
			t.inputDrain = &drainedInput{
				instance:          t.inputInstance,
				turn:              t.inputTurn,
				humanGeneration:   t.humanGeneration,
				pointerGeneration: t.pointerGeneration,
				outputSeq:         outputSeq,
				health:            health,
			}
			return inputDrainQuiescent, nil
		}
	}
	return t.pending("terminal output still arriving"), nil
}

func (t *transport) pending(reason string) inputDrain {
	t.drainBlocker = some(reason)
	return inputDrainPending
}

func (t *transport) readDrainChild(pty ptyIo, capacity int) (availableRead, error) {
	if capacity == 0 || t.childEOF {
		return availableReadSkipped{}, nil
	}
	if t.deferredChildBytes != nil {
		bytes := t.deferredChildBytes
		if len(bytes) == 0 {
			panic("prefetch retains positive child bytes")
		}
		split := min(len(bytes), capacity)
		t.deferredChildBytes = nil
		if split < len(bytes) {
			t.deferredChildBytes = bytes[split:]
		}
		return availableReadData{bytes: bytes[:split]}, nil
	}
	read, err := readClassified(pty, capacity)
	if err != nil {
		return nil, transportIoError(err)
	}
	return read, nil
}

func (t *transport) observeDrainChild(bytes []byte, observer screenObserver, deadline time.Time) error {
	if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
		t.retainObservationRemainder(bytes)
		return err
	}
	if t.observationHooks != nil {
		if err := t.observationHooks.before(t.observationPhase, observationOperationParser); err != nil {
			t.observationFenced = true
			return err
		}
		if err := runtimeCheckDeadline(t.clock.now(), deadline); err != nil {
			t.retainObservationRemainder(bytes)
			return err
		}
	}
	bounded := &deadlineObserver{inner: observer, deadline: deadline, clock: t.clock}
	switch failure := t.observeChildBytes(bytes, bounded).(type) {
	case nil:
		return runtimeCheckDeadline(t.clock.now(), deadline)
	case terminalObservationErrorObserver:
		return failure.cause
	case terminalObservationErrorParser:
		if notification := bounded.parserFailure(); notification != nil {
			return cafleetErrorOf(failure.cause).withDiagnostic(cafleetErrorOf(notification).message())
		}
		return failure.cause
	default:
		panic("invalid terminalObservationError")
	}
}

const (
	replyPrefixBound  = 100 * time.Millisecond
	csiParameterLimit = 64
	stringBodyLimit   = 4096
	asciiEscape       = 0x1b
	asciiBell         = 0x07
	replyPasteEnd     = "\x1b[201~"
)

type stringKind uint8

const (
	stringKindOsc stringKind = iota + 1
	stringKindDcs
	stringKindApc
)

//sumtype:decl
type replyState interface{ isReplyState() }

type replyStateGround struct{}

type replyStateEscape struct{}

type replyStateCsi struct {
	parameters   []byte
	intermediate bool
}

type replyStateX10 struct{ remaining uint8 }

type replyStateStringBody struct {
	kind   stringKind
	length int
}

type replyStateStringEscape struct{}

func (replyStateGround) isReplyState()       {}
func (replyStateEscape) isReplyState()       {}
func (replyStateCsi) isReplyState()          {}
func (replyStateX10) isReplyState()          {}
func (replyStateStringBody) isReplyState()   {}
func (replyStateStringEscape) isReplyState() {}

func flushNonblocking(out display) error {
	if err := out.tryFlush(); err != nil && !errors.Is(err, syscall.EAGAIN) {
		return transportIoError(err)
	}
	return nil
}

// replyClassifier splits outer-terminal input into terminal replies, pointer reports and manual
// keystrokes. Its zero value is the Rust Default, a Ground state.
type replyClassifier struct {
	state           replyState
	pendingBytes    int
	lastByte        opt[time.Time]
	pasteEndMatched opt[int]
}

type classified struct {
	manual  int
	pointer int
}

func classifiedManual(count int) classified {
	return classified{manual: count}
}

func classifiedPointer(count int) classified {
	return classified{pointer: count}
}

func (r *replyClassifier) feed(bytes []byte, now time.Time) classified {
	var total classified
	for _, b := range bytes {
		c := r.classify(b, now)
		total.manual += c.manual
		total.pointer += c.pointer
	}
	return total
}

func (r *replyClassifier) expire(now time.Time) classified {
	manual := 0
	if last, set := r.lastByte.get(); set && max(now.Sub(last), 0) >= replyPrefixBound {
		manual = r.resolveManual()
	}
	return classifiedManual(manual)
}

func (r *replyClassifier) finish() classified {
	return classifiedManual(r.resolveManual())
}

func (r *replyClassifier) pending() bool {
	return r.pendingBytes > 0
}

func (r *replyClassifier) classify(b byte, now time.Time) classified {
	var c classified
	if matched, set := r.pasteEndMatched.get(); set {
		r.advancePaste(matched, b)
		c = classifiedManual(1)
	} else {
		c = r.step(b)
	}
	r.lastByte = none[time.Time]()
	if r.pending() {
		r.lastByte = some(now)
	}
	return c
}

func (r *replyClassifier) advancePaste(matched int, b byte) {
	switch b {
	case replyPasteEnd[matched]:
		matched++
	case asciiEscape:
		matched = 1
	default:
		matched = 0
	}
	r.pasteEndMatched = none[int]()
	if matched < len(replyPasteEnd) {
		r.pasteEndMatched = some(matched)
	}
}

func (r *replyClassifier) takeState() replyState {
	state := r.state
	r.state = replyStateGround{}
	if state == nil {
		return replyStateGround{}
	}
	return state
}

func (r *replyClassifier) step(b byte) classified {
	switch state := r.takeState().(type) {
	case replyStateGround:
		return r.ground(b)
	case replyStateEscape:
		switch b {
		case '[':
			return r.extend(replyStateCsi{})
		case ']':
			return r.openString(stringKindOsc)
		case 'P':
			return r.openString(stringKindDcs)
		case '_':
			return r.openString(stringKindApc)
		default:
			return r.invalid(b)
		}
	case replyStateCsi:
		switch {
		case b >= 0x20 && b <= 0x3f && len(state.parameters) == csiParameterLimit:
			return r.invalid(b)
		case b >= 0x30 && b <= 0x3f && !state.intermediate:
			return r.extend(replyStateCsi{parameters: append(state.parameters, b), intermediate: state.intermediate})
		case b >= 0x20 && b <= 0x2f:
			return r.extend(replyStateCsi{parameters: append(state.parameters, b), intermediate: true})
		case b >= 0x40 && b <= 0x7e:
			return r.completeCsi(state.parameters, b)
		default:
			return r.invalid(b)
		}
	case replyStateX10:
		switch {
		case b >= 0x20 && state.remaining == 1:
			length := r.pendingBytes + 1
			r.pendingBytes = 0
			return classifiedPointer(length)
		case b >= 0x20:
			if state.remaining == 0 {
				panic("X10 payload remains")
			}
			return r.extend(replyStateX10{remaining: state.remaining - 1})
		default:
			return r.invalid(b)
		}
	case replyStateStringBody:
		printable := (b >= 0x20 && b <= 0x7e) || b >= 0x80
		switch {
		case printable && state.length == stringBodyLimit:
			return r.invalid(b)
		case printable:
			return r.extend(replyStateStringBody{kind: state.kind, length: state.length + 1})
		case b == asciiBell && state.kind == stringKindOsc:
			return r.completeReply()
		case b == asciiEscape:
			return r.extend(replyStateStringEscape{})
		default:
			return r.invalid(b)
		}
	case replyStateStringEscape:
		if b == '\\' {
			return r.completeReply()
		}
		return r.invalid(b)
	default:
		panic("invalid replyState")
	}
}

func (r *replyClassifier) ground(b byte) classified {
	if b == asciiEscape {
		r.state = replyStateEscape{}
		r.pendingBytes = 1
		return classified{}
	}
	return classifiedManual(1)
}

func (r *replyClassifier) openString(kind stringKind) classified {
	return r.extend(replyStateStringBody{kind: kind})
}

func (r *replyClassifier) extend(state replyState) classified {
	r.state = state
	r.pendingBytes++
	return classified{}
}

func (r *replyClassifier) invalid(b byte) classified {
	manual := r.resolveManual()
	c := r.ground(b)
	c.manual += manual
	return c
}

func (r *replyClassifier) resolveManual() int {
	r.state = replyStateGround{}
	r.lastByte = none[time.Time]()
	pending := r.pendingBytes
	r.pendingBytes = 0
	return pending
}

func (r *replyClassifier) completeReply() classified {
	r.pendingBytes = 0
	return classified{}
}

func (r *replyClassifier) completeCsi(parameters []byte, finalByte byte) classified {
	if len(parameters) == 0 && finalByte == 'M' {
		return r.extend(replyStateX10{remaining: 3})
	}
	length := r.pendingBytes + 1
	r.pendingBytes = 0
	if finalByte == 'M' || finalByte == 'm' {
		if list, ok := bytes.CutPrefix(parameters, []byte("<")); ok {
			if count, valid := parameterCount(list).get(); valid && count == 3 {
				return classifiedPointer(length)
			}
		}
	}
	if isCsiReply(parameters, finalByte) {
		return classified{}
	}
	if string(parameters) == "200" && finalByte == '~' {
		r.pasteEndMatched = some(0)
	}
	return classifiedManual(length)
}

func isCsiReply(parameters []byte, finalByte byte) bool {
	marker := byte(0)
	rest := parameters
	if len(parameters) > 0 && (parameters[0] == '?' || parameters[0] == '>') {
		marker, rest = parameters[0], parameters[1:]
	}
	split := len(rest)
	for index, b := range rest {
		if b >= 0x20 && b <= 0x2f {
			split = index
			break
		}
	}
	list, intermediates := rest[:split], string(rest[split:])
	arity := func(low, high int) bool {
		count, valid := parameterCount(list).get()
		return valid && count >= low && count <= high
	}
	switch {
	case marker == 0 && intermediates == "" && finalByte == 'R':
		return arity(2, 2)
	case marker == '?' && intermediates == "" && finalByte == 'R':
		return arity(2, 3)
	case marker == 0 && intermediates == "" && finalByte == 'n':
		return string(list) == "0" || string(list) == "3"
	case marker == '?' && intermediates == "" && finalByte == 'n':
		return arity(1, 4)
	case marker == '?' && intermediates == "" && finalByte == 'c':
		return arity(1, 32)
	case marker == '>' && intermediates == "" && finalByte == 'c':
		return arity(1, 4)
	case (marker == '?' || marker == 0) && intermediates == "$" && finalByte == 'y':
		return arity(2, 2)
	case marker == 0 && intermediates == "" && finalByte == 't':
		return arity(1, 3)
	case marker == '?' && intermediates == "" && finalByte == 'u':
		return arity(1, 1)
	case marker == 0 && intermediates == "" && (finalByte == 'I' || finalByte == 'O'):
		return len(list) == 0
	default:
		return false
	}
}

// parameterCount counts ;-joined digit parameters, or none when any parameter is empty or holds a
// non-digit byte.
func parameterCount(list []byte) opt[int] {
	count := 0
	for parameter := range bytes.SplitSeq(list, []byte(";")) {
		if len(parameter) == 0 || bytes.ContainsFunc(parameter, func(r rune) bool { return r < '0' || r > '9' }) {
			return none[int]()
		}
		count++
	}
	return some(count)
}

type screenSnapshot struct {
	rows uint16
	cols uint16
	// cursor is (row, col); absent for a capture whose source exposes no cursor.
	cursor opt[[2]uint16]
	// alternateScreen is absent for a capture whose source exposes no screen mode.
	alternateScreen opt[bool]
	screenSeq       uint64
	screen          *vt.Screen
}

type terminalState struct {
	parser   *vt.Parser
	sequence uint64
}

//sumtype:decl
type terminalObservationError interface{ isTerminalObservationError() }

type terminalObservationErrorParser struct{ cause error }

type terminalObservationErrorObserver struct {
	cause error
	// consumed is the failing step's end: its bytes reached the parser.
	consumed int
}

func (terminalObservationErrorParser) isTerminalObservationError()   {}
func (terminalObservationErrorObserver) isTerminalObservationError() {}

// minScreenCols keeps the screen model at two columns or more: vt100 panics when a wide or
// combining scalar lands on a one-column grid. The PTY keeps the pane's own size.
const minScreenCols uint16 = 2

func screenCols(size terminalSize) uint16 {
	if size.cols == 0 {
		panic("validated cols")
	}
	return max(size.cols, minScreenCols)
}

func newTerminalState(size terminalSize) (*terminalState, error) {
	if err := size.validate(); err != nil {
		return nil, err
	}
	return &terminalState{parser: vt.NewParser(size.rows, screenCols(size), nil)}, nil
}

func (t *terminalState) advanceSequence(by uint64) error {
	if t.sequence > math.MaxUint64-by {
		return runtimeError("terminal sequence exhausted")
	}
	t.sequence += by
	return nil
}

func (t *terminalState) feed(bytes []byte) error {
	if err := t.advanceSequence(uint64(len(bytes))); err != nil {
		return err
	}
	t.parser.Process(bytes)
	return nil
}

// feedObservedClassified feeds bytes one observation step at a time (every ASCII control byte
// alone, and every run of other bytes between them) and shows the observer the screen after
// each step. It returns nil when every step succeeds.
func (t *terminalState) feedObservedClassified(bytes []byte, observer screenObserver) terminalObservationError {
	start := 0
	for index, b := range bytes {
		if b < 0x20 || b == 0x7F {
			if index > start {
				if failure := t.step(bytes, start, index, observer); failure != nil {
					return failure
				}
			}
			if failure := t.step(bytes, index, index+1, observer); failure != nil {
				return failure
			}
			start = index + 1
		}
	}
	if start < len(bytes) {
		return t.step(bytes, start, len(bytes), observer)
	}
	return nil
}

func (t *terminalState) step(bytes []byte, start, end int, observer screenObserver) terminalObservationError {
	if err := t.feed(bytes[start:end]); err != nil {
		return terminalObservationErrorParser{cause: err}
	}
	view := t.view()
	if err := observer.observe(&view); err != nil {
		return terminalObservationErrorObserver{cause: err, consumed: end}
	}
	return nil
}

func (t *terminalState) resize(size terminalSize) error {
	if err := size.validate(); err != nil {
		return err
	}
	if err := t.advanceSequence(1); err != nil {
		return err
	}
	t.parser.SetSize(size.rows, screenCols(size))
	return nil
}

// view is the current screen without a copy; it is valid until the next feed or resize.
func (t *terminalState) view() screenSnapshot {
	screen := t.parser.Screen()
	rows, cols := screen.Size()
	row, col := screen.CursorPosition()
	return screenSnapshot{
		rows:            rows,
		cols:            cols,
		cursor:          some([2]uint16{row, col}),
		alternateScreen: some(screen.AlternateScreen()),
		screenSeq:       t.sequence,
		screen:          screen,
	}
}

// snapshot is the current screen as an owned copy that later feeds and resizes leave unchanged.
func (t *terminalState) snapshot() screenSnapshot {
	s := t.view()
	s.screen = s.screen.Clone()
	return s
}

// rawTerminalGuard holds a terminal in raw mode (F16) until restore or close returns it to its
// original attributes.
type rawTerminalGuard struct {
	terminal *os.File
	original unix.Termios
	restored bool
}

func rawTerminalGuardEnter(terminal *os.File) (*rawTerminalGuard, error) {
	duplicate, err := unix.FcntlInt(terminal.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	guard := &rawTerminalGuard{terminal: os.NewFile(uintptr(duplicate), terminal.Name())}
	original, err := getTermios(duplicate)
	if err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	guard.original = *original
	raw := *original
	cfmakeraw(&raw)
	if err := setTermios(duplicate, &raw); err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	return guard, nil
}

func (g *rawTerminalGuard) restore() error {
	if g.restored {
		return nil
	}
	if err := setTermios(int(g.terminal.Fd()), &g.original); err != nil {
		return runtimeError("terminal restoration requires host recovery: " + errnoText(err))
	}
	g.restored = true
	return nil
}

// close restores the terminal, reporting a failed restoration on stderr, and releases the
// guard's descriptor.
func (g *rawTerminalGuard) close() {
	if err := g.restore(); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
	closeFile(g.terminal)
}
